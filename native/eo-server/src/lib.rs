//! Earth Online's authoritative server (headless, no GPU).
//!
//! One fixed 60 Hz tick:
//!   1. datagrams in (handshakes, inputs, acks) from the I/O thread;
//!   2. every player's controller runs the inputs that arrived, in sequence
//!      (each player has its own Rapier world fed from one shared tile set,
//!      so players step in parallel on the worker pool);
//!   3. the region cells open, close, take their viewers and hand agents
//!      across borders (`regions.rs`), then every region steps (eo-sim's pool);
//!   4. every `TICK_HZ / snapshot_hz` ticks, each client gets a snapshot of
//!      what is near it (interest grid, delta against its acknowledged
//!      baseline, its bandwidth share), built and sent in parallel.
//!
//! The player's step is the client's step (`eo_net::game::step_player`):
//! the server runs exactly the inputs the client predicted with, in order,
//! so the two agree; the client's position check after each packet's
//! newest input is compared and a whole-state correction is sent only when
//! they differ (a lost input the server had to stand in for, or a world the
//! client does not have).

pub mod bots;
pub mod predict;
pub mod regions;
pub mod sys;
pub mod tiles;

use eo_net::auth::{Account, Authenticator, DevTokens};
use eo_net::bits::{ByteReader, PutLe};
use eo_net::conn::Connection;
use eo_net::game::{GameWorld, check_of, spawn_player, step_player};
use eo_net::interest::{Grid, Radii};
use eo_net::link::{Conditions, Link};
use eo_net::pool::Pool;
use eo_net::proto::{self, Accept, Challenge, DenyReason, Hello, Kind, MTU, Resume};
use eo_net::snapshot::{self, CLASS_CAR, CLASS_PLAYER, CLASS_PLAYER_CAR, CLASS_WALKER, Correction, Desired, Own, QState, Replicator, SnapHeader, net_id, q_angle, q_pos, q_speed};
use eo_net::wire::{Check, InputPacket, NetInput, write_player_state};
use eo_player::world::CollisionWorld;
use eo_player::{Mode, Player};
use glam::{DQuat, DVec3};
use regions::{Regions, Viewer};
use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub bind: String,
    pub snapshot_hz: u32,
    /// Each client's snapshot bytes per second at most (records past it wait).
    pub budget_bytes_per_s: usize,
    pub radii: Radii,
    pub conditions: Conditions,
    pub streets: bool,
    /// The grade-separated roads the world is loaded with (`--structures`,
    /// `--tunnels`; `load`). The handshake compares a client's against the
    /// loaded world's own (`GameWorld::structures`).
    pub structures: Option<eo_structures::StructureOptions>,
    pub max_players: usize,
    pub seed: u32,
    /// A dropped player keeps its body this long for a reconnect.
    pub resume_grace_s: f64,
    /// No packet for this long: the client is offline.
    pub timeout_s: f64,
    /// An input missing this long while later ones wait is stood in for.
    pub input_gap_s: f64,
    pub player_threads: usize,
    pub sim_threads: usize,
    pub tile_threads: usize,
    /// Where a player spawns when it gives no hint (the bench spawn on Folsom).
    pub default_spawn: [f64; 3],
}

impl Default for ServerConfig {
    fn default() -> Self {
        let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(8);
        Self {
            bind: "127.0.0.1:47800".into(),
            snapshot_hz: 20,
            budget_bytes_per_s: 96_000,
            radii: Radii::default(),
            conditions: Conditions::NONE,
            streets: true,
            structures: None,
            max_players: 512,
            seed: 1,
            resume_grace_s: 30.0,
            timeout_s: 3.0,
            input_gap_s: 0.25,
            player_threads: (n / 2).max(1),
            sim_threads: (n / 2).max(1),
            tile_threads: 4,
            // eo-app's bench spawn on Folsom near 4th (on the scaled plan, as eo-app's `bench::spawn()`).
            default_spawn: {
                let p = eo_data::groundscale::plan([1539.0, -366.0]);
                [p[0], p[1], 0.0]
            },
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SessionStats {
    pub inputs_run: u64,
    pub inputs_stood_in: u64,
    pub inputs_late: u64,
    pub corrections_sent: u64,
    pub mismatches: u64,
    pub snapshots: u64,
    pub deferred: u64,
    /// Bytes on the wire to this client (UDP/IP headers included) in each whole second.
    pub bytes_per_s: Vec<u64>,
    sec_bytes: u64,
    sec_start: Option<Instant>,
    pub step_ns: u64,
    pub entities: usize,
}

/// One player (online, or offline within its grace period).
pub struct Session {
    pub player_id: u32,
    pub account: Account,
    pub resume_key: u64,
    pub addr: SocketAddr,
    pub connection: u64,
    pub conn: Connection,
    pub online: bool,
    pub offline_since: Option<Instant>,
    pub player: Player,
    pub cw: CollisionWorld,
    pub last_seq: u32,
    pending: BTreeMap<u32, NetInput>,
    last_input: NetInput,
    gap_since: Option<Instant>,
    check: Option<Check>,
    client_applied: u16,
    corr_id: u16,
    correction: Option<Correction>,
    tokens: f64,
    pub rep: Replicator,
    accept: Accept,
    pub stats: SessionStats,
    pub pos: DVec3,
    /// Debug/test: teleport the player by this much on the next step (a server-side event the client cannot predict).
    pub nudge: Option<DVec3>,
    /// The newest input packet's timestamp and when it arrived (echoed for ping).
    ping_echo: Option<(u16, Instant)>,
}

const PLAYER_BLOCK_CARS: u32 = 16;

impl Session {
    fn view_yaw(&self) -> f64 {
        self.player.yaw
    }

    /// Run the inputs that arrived, in sequence (tokens limit a client to real time plus a burst).
    fn run(&mut self, now: Instant, gap_s: f64) {
        let t0 = Instant::now();
        self.tokens = (self.tokens + 1.0).min(30.0);
        let mut ran = 0;
        while self.tokens >= 1.0 {
            let next = self.last_seq.wrapping_add(1);
            let input = if let Some(i) = self.pending.remove(&next) {
                self.gap_since = None;
                i
            } else if !self.pending.is_empty() {
                let since = *self.gap_since.get_or_insert(now);
                if (now - since).as_secs_f64() < gap_s {
                    break;
                }
                // Stand in for the lost input: the last one, without its taps.
                self.gap_since = None;
                self.stats.inputs_stood_in += 1;
                let mut i = self.last_input;
                i.buttons &= !(eo_net::wire::button::JUMP | eo_net::wire::button::VEHICLE_TOGGLE);
                i
            } else {
                break;
            };
            if let Some(d) = self.nudge.take() {
                let mut s = self.player.state(&self.cw);
                s.character.position = (DVec3::from_array(s.character.position) + d).to_array();
                for v in s.vehicles.iter_mut() {
                    v.translation = [v.translation[0] + d.x as f32, v.translation[1] + d.y as f32, v.translation[2] + d.z as f32];
                }
                self.player.set_state(&mut self.cw, &s);
            }
            step_player(&mut self.player, &mut self.cw, &input);
            self.last_seq = next;
            self.last_input = input;
            self.tokens -= 1.0;
            self.stats.inputs_run += 1;
            ran += 1;
            // The client's check for this step: a mismatch earns a correction (once per id).
            if let Some(c) = self.check {
                if c.seq == next && self.client_applied == self.corr_id {
                    let mine = check_of(next, &self.player, &self.cw);
                    let off = (0..3).any(|k| (mine.pos_mm[k] - c.pos_mm[k]).abs() > 2) || mine.driving != c.driving;
                    if off {
                        if std::env::var_os("EO_NET_DEBUG").is_some() {
                            eprintln!("server: player {} mismatch at seq {next}: server {:?} client {:?} (applied {} corr {})", self.player_id, mine.pos_mm, c.pos_mm, self.client_applied, self.corr_id);
                        }
                        self.stats.mismatches += 1;
                        self.corr_id = self.corr_id.wrapping_add(1).max(1);
                        let mut state = Vec::with_capacity(512);
                        write_player_state(&self.player.state(&self.cw), &mut state);
                        self.correction = Some(Correction { id: self.corr_id, seq: next, state });
                        self.stats.corrections_sent += 1;
                    }
                }
            }
            if ran >= 12 {
                break;
            }
        }
        if ran > 0 {
            self.pos = self.player.position(&self.cw);
            self.cw.pump(self.pos);
        }
        self.stats.step_ns = t0.elapsed().as_nanos() as u64;
    }

    fn take_packet(&mut self, p: InputPacket, arrived: Instant) {
        self.ping_echo = Some((p.ping_ms, arrived));
        self.rep.ack(p.ack_snapshot);
        self.client_applied = p.correction_applied;
        if self.client_applied == self.corr_id {
            self.correction = None;
        }
        if let Some(c) = p.check {
            if self.check.is_none_or(|old| c.seq > old.seq) {
                self.check = Some(c);
            }
        }
        for (k, i) in p.inputs.iter().enumerate() {
            let seq = p.first_seq.wrapping_add(k as u32);
            if seq <= self.last_seq {
                continue;
            }
            if self.pending.len() >= 240 {
                self.stats.inputs_late += 1;
                break;
            }
            self.pending.entry(seq).or_insert(*i);
        }
    }

    fn count_bytes(&mut self, n: usize, now: Instant) {
        let start = *self.stats.sec_start.get_or_insert(now);
        if (now - start).as_secs_f64() >= 1.0 {
            self.stats.bytes_per_s.push(self.stats.sec_bytes);
            self.stats.sec_bytes = 0;
            self.stats.sec_start = Some(start + Duration::from_secs(1));
        }
        self.stats.sec_bytes += n as u64;
    }
}

struct Joining {
    connection: u64,
    addr: SocketAddr,
    account: Account,
    spawn: [f64; 3],
    driving: bool,
    cw: CollisionWorld,
    since: Instant,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TickTiming {
    pub total_ms: f64,
    pub net_in_ms: f64,
    pub players_ms: f64,
    /// Summed over the pool's threads.
    pub players_cpu_ms: f64,
    pub regions_ms: f64,
    pub sim_ms: f64,
    pub sim_cpu_ms: f64,
    pub snapshot_ms: f64,
    pub snapshot_tick: bool,
}

#[derive(Default)]
pub struct ServerStats {
    pub ticks: Vec<TickTiming>,
    pub joined: u64,
    pub resumed: u64,
    pub left: u64,
    pub denied: u64,
    pub packets_in: u64,
    pub bad_packets: u64,
}

pub struct Server {
    pub cfg: ServerConfig,
    pub link: Arc<Link>,
    /// Datagrams from the I/O thread, with when they arrived (ping excludes the wait for a tick).
    rx: Receiver<(Vec<u8>, SocketAddr, Instant)>,
    io_stop: Arc<AtomicBool>,
    io: Option<std::thread::JoinHandle<()>>,
    pub game: Arc<GameWorld>,
    pub tiles: Arc<tiles::SharedTiles>,
    auth: Box<dyn Authenticator>,
    secret: [u64; 2],
    pub sessions: Vec<Session>,
    by_conn: HashMap<u64, usize>,
    joining: Vec<Joining>,
    pub regions: Regions,
    pool: Pool,
    pub tick: u32,
    next_player: u32,
    grid: Grid,
    pub stats: ServerStats,
    pub started: Instant,
}

fn keyed(secret: [u64; 2], parts: &[u64]) -> u64 {
    // SipHash would be the choice with a crate; this keyed mix is a
    // stateless cookie against blind spoofing, not a cryptographic MAC.
    let mut h = secret[0] ^ 0x243F_6A88_85A3_08D3;
    for &p in parts {
        h ^= p.wrapping_add(secret[1]);
        h = h.rotate_left(29).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        h ^= h >> 31;
    }
    h
}

fn addr_bits(a: &SocketAddr) -> u64 {
    match a {
        SocketAddr::V4(v) => (u32::from(*v.ip()) as u64) << 16 | v.port() as u64,
        SocketAddr::V6(v) => {
            let o = v.ip().octets();
            u64::from_le_bytes(o[..8].try_into().unwrap()) ^ u64::from_le_bytes(o[8..].try_into().unwrap()) ^ v.port() as u64
        }
    }
}

fn random_secret() -> [u64; 2] {
    use std::hash::{BuildHasher, Hasher};
    let s = std::collections::hash_map::RandomState::new();
    let mut a = s.build_hasher();
    a.write_u64(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(7));
    let mut b = s.build_hasher();
    b.write_u64(std::process::id() as u64);
    [a.finish(), b.finish()]
}

impl Server {
    pub fn start(cfg: ServerConfig, game: Arc<GameWorld>, source: Arc<eo_sim::native::NativeSource>) -> anyhow::Result<Self> {
        let link = Arc::new(Link::bind(&cfg.bind, cfg.conditions, cfg.seed as u64 ^ 0x5EED)?);
        let (tx, rx) = channel();
        let io_stop = Arc::new(AtomicBool::new(false));
        let io = {
            let (link, stop) = (link.clone(), io_stop.clone());
            std::thread::Builder::new()
                .name("eo-server-io".into())
                .spawn(move || {
                    let mut buf = vec![0u8; 2048];
                    while !stop.load(Ordering::Relaxed) {
                        if let Some((n, a)) = link.recv(&mut buf) {
                            if tx.send((buf[..n].to_vec(), a, Instant::now())).is_err() {
                                return;
                            }
                        }
                    }
                })?
        };
        let tiles = tiles::SharedTiles::start(game.ground.clone(), game.buildings.clone(), cfg.tile_threads);
        let regions = Regions::new(source, cfg.seed, cfg.sim_threads.saturating_sub(1));
        let pool = Pool::new(cfg.player_threads.saturating_sub(1), "eo-server-work");
        Ok(Self {
            cfg,
            link,
            rx,
            io_stop,
            io: Some(io),
            game,
            tiles,
            auth: Box::new(DevTokens),
            secret: random_secret(),
            sessions: vec![],
            by_conn: HashMap::new(),
            joining: vec![],
            regions,
            pool,
            tick: 0,
            next_player: 1,
            grid: Grid::default(),
            stats: ServerStats::default(),
            started: Instant::now(),
        })
    }

    /// Replace the development token check with an account system's.
    pub fn set_authenticator(&mut self, a: Box<dyn Authenticator>) {
        self.auth = a;
    }

    pub fn addr(&self) -> SocketAddr {
        self.link.local_addr()
    }

    pub fn online(&self) -> usize {
        self.sessions.iter().filter(|s| s.online).count()
    }

    pub fn session(&self, player_id: u32) -> Option<&Session> {
        self.sessions.iter().find(|s| s.player_id == player_id)
    }

    pub fn session_mut(&mut self, player_id: u32) -> Option<&mut Session> {
        self.sessions.iter_mut().find(|s| s.player_id == player_id)
    }

    /// Run ticks at 60 Hz until `stop` (sleeping between them).
    pub fn run(&mut self, stop: &AtomicBool) {
        let dt = Duration::from_secs_f64(1.0 / proto::TICK_HZ as f64);
        let mut next = Instant::now();
        while !stop.load(Ordering::Relaxed) {
            self.tick();
            next += dt;
            let now = Instant::now();
            if next > now {
                let wait = next - now;
                if wait > Duration::from_micros(1500) {
                    std::thread::sleep(wait - Duration::from_micros(1000));
                }
                while Instant::now() < next {
                    std::hint::spin_loop();
                }
            } else if now - next > Duration::from_millis(250) {
                next = now; // fell far behind: do not try to catch up
            }
        }
    }

    pub fn tick(&mut self) -> TickTiming {
        let t0 = Instant::now();
        while let Ok((d, a, at)) = self.rx.try_recv() {
            self.stats.packets_in += 1;
            self.handle(&d, a, at);
        }
        self.pump_joining();
        let t1 = Instant::now();
        let gap = self.cfg.input_gap_s;
        // Players step on the pool; a network tile waiting to be admitted is
        // admitted beside them on its own thread (they share nothing).
        let pending = self.regions.take_build();
        let (pool, sessions, regions) = (&self.pool, &mut self.sessions, &mut self.regions);
        let step_players = |sessions: &mut Vec<Session>| {
            pool.for_each_mut(sessions, |_, s| {
                if s.online {
                    s.run(t1, gap);
                }
            })
        };
        match pending {
            Some(p) => std::thread::scope(|sc| {
                sc.spawn(move || regions.apply_build(p));
                step_players(sessions);
            }),
            None => step_players(sessions),
        }
        let players_cpu: u64 = self.sessions.iter().map(|s| if s.online { s.stats.step_ns } else { 0 }).sum();
        let t2 = Instant::now();
        let viewers: Vec<Viewer> = self.sessions.iter().map(|s| Viewer { player: s.player_id, x: s.pos.x, z: s.pos.z, yaw: s.view_yaw() }).collect();
        self.regions.update(&viewers);
        let t3 = Instant::now();
        self.regions.step();
        let t4 = Instant::now();
        let div = (proto::TICK_HZ / self.cfg.snapshot_hz.max(1)).max(1);
        let snap = self.tick % div == 0;
        if snap {
            self.replicate();
        }
        let t5 = Instant::now();
        self.expire();
        if self.tick % 600 == 0 {
            self.tiles.evict(60.0);
        }
        self.tick = self.tick.wrapping_add(1);
        let ms = |a: Instant, b: Instant| (b - a).as_secs_f64() * 1e3;
        let t = TickTiming {
            total_ms: ms(t0, Instant::now()),
            net_in_ms: ms(t0, t1),
            players_ms: ms(t1, t2),
            players_cpu_ms: players_cpu as f64 / 1e6,
            regions_ms: ms(t2, t3),
            sim_ms: ms(t3, t4),
            sim_cpu_ms: self.regions.stats.step_cpu_ms,
            snapshot_ms: ms(t4, t5),
            snapshot_tick: snap,
        };
        if self.stats.ticks.len() < 1 << 20 {
            self.stats.ticks.push(t);
        }
        t
    }

    fn expire(&mut self) {
        let now = Instant::now();
        for s in self.sessions.iter_mut() {
            if s.online && (now - s.conn.last_recv).as_secs_f64() > self.cfg.timeout_s {
                s.online = false;
                s.offline_since = Some(now);
            }
        }
        let grace = self.cfg.resume_grace_s;
        let before = self.sessions.len();
        self.sessions.retain(|s| s.online || s.offline_since.is_none_or(|t| (now - t).as_secs_f64() < grace));
        if self.sessions.len() != before {
            self.stats.left += (before - self.sessions.len()) as u64;
            self.reindex();
        }
    }

    fn reindex(&mut self) {
        self.by_conn = self.sessions.iter().enumerate().filter(|(_, s)| s.online).map(|(i, s)| (s.connection, i)).collect();
    }

    fn handle(&mut self, data: &[u8], addr: SocketAddr, arrived: Instant) {
        let Some((kind, mut r)) = proto::peek(data) else {
            self.stats.bad_packets += 1;
            return;
        };
        match kind {
            Kind::Request => {
                if data.len() < proto::REQUEST_PAD {
                    return;
                }
                let Ok(h) = Hello::read(Kind::Request, &mut r) else { return };
                let mut out = Vec::with_capacity(40);
                if h.version != proto::PROTOCOL_VERSION {
                    proto::write_deny(h.client_salt, DenyReason::Version, &mut out);
                } else if h.ground != proto::ground_scale_ppm() {
                    // a client on another ground plan would desync.
                    proto::write_deny(h.client_salt, DenyReason::World, &mut out);
                    self.stats.denied += 1;
                } else if h.structures != proto::structures_code(self.game.structures) {
                    // a client with other grade-separated roads collides and drives on another world.
                    proto::write_deny_structures(h.client_salt, proto::structures_code(self.game.structures), &mut out);
                    self.stats.denied += 1;
                } else {
                    let server_salt = keyed(self.secret, &[addr_bits(&addr), h.client_salt, 1]) | 1;
                    let cookie = keyed(self.secret, &[addr_bits(&addr), h.client_salt, server_salt, 2]);
                    Challenge { client_salt: h.client_salt, server_salt, cookie }.write(&mut out);
                }
                self.link.send(&out, addr);
            }
            Kind::Response => {
                if data.len() < proto::REQUEST_PAD {
                    return;
                }
                let Ok(h) = Hello::read(Kind::Response, &mut r) else { return };
                if h.ground != proto::ground_scale_ppm() || h.structures != proto::structures_code(self.game.structures) {
                    return; // refused at its request
                }
                if h.cookie != keyed(self.secret, &[addr_bits(&addr), h.client_salt, h.server_salt, 2]) || h.server_salt != keyed(self.secret, &[addr_bits(&addr), h.client_salt, 1]) | 1 {
                    return;
                }
                let connection = h.client_salt ^ h.server_salt;
                if let Some(&i) = self.by_conn.get(&connection) {
                    let mut out = vec![];
                    self.sessions[i].accept.write(&mut out);
                    self.link.send(&out, addr);
                    return;
                }
                if self.joining.iter().any(|j| j.connection == connection) {
                    return;
                }
                let account = match self.auth.validate(&h.token) {
                    Ok(a) => a,
                    Err(_) => {
                        let mut out = vec![];
                        proto::write_deny(h.client_salt, DenyReason::Token, &mut out);
                        self.link.send(&out, addr);
                        self.stats.denied += 1;
                        return;
                    }
                };
                if let Some(rs) = h.resume {
                    if self.resume(rs, &account, connection, addr) {
                        return;
                    }
                }
                if self.sessions.len() + self.joining.len() >= self.cfg.max_players {
                    let mut out = vec![];
                    proto::write_deny(h.client_salt, DenyReason::Full, &mut out);
                    self.link.send(&out, addr);
                    return;
                }
                let (spawn, driving) = match h.spawn {
                    Some(s) if s.x.is_finite() && s.z.is_finite() && s.x.abs() < 20_000.0 && s.z.abs() < 20_000.0 => ([s.x as f64, s.z as f64, s.yaw as f64], s.driving),
                    _ => (self.cfg.default_spawn, false),
                };
                let mut cw = CollisionWorld::fed(self.game.ground.clone(), self.game.buildings.clone(), self.tiles.clone());
                cw.insert_per_pump = 4;
                self.joining.push(Joining { connection, addr, account, spawn, driving, cw, since: Instant::now() });
            }
            Kind::Input | Kind::KeepAlive | Kind::Disconnect => {
                let Ok(connection) = r.u64() else { return };
                let Some(&i) = self.by_conn.get(&connection) else { return };
                let s = &mut self.sessions[i];
                if s.addr != addr {
                    return;
                }
                let Ok(Some(_)) = s.conn.receive(&mut r) else { return };
                match kind {
                    Kind::Input => {
                        if let Ok(p) = InputPacket::read(r.rest()) {
                            s.take_packet(p, arrived);
                        }
                    }
                    Kind::Disconnect => {
                        s.online = false;
                        s.offline_since = Some(Instant::now() - Duration::from_secs_f64(self.cfg.resume_grace_s));
                        self.reindex();
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn resume(&mut self, rs: Resume, account: &Account, connection: u64, addr: SocketAddr) -> bool {
        let Some(i) = self.sessions.iter().position(|s| s.player_id == rs.player_id && s.resume_key == rs.key && s.account.id == account.id) else { return false };
        let s = &mut self.sessions[i];
        s.addr = addr;
        s.connection = connection;
        s.conn = Connection::new();
        s.online = true;
        s.offline_since = None;
        s.rep.reset();
        s.pending.clear();
        s.check = None;
        s.correction = None;
        s.client_applied = 0;
        s.corr_id = 0;
        s.gap_since = None;
        let mut state = Vec::with_capacity(512);
        write_player_state(&s.player.state(&s.cw), &mut state);
        s.accept = Accept { connection, input_seq: s.last_seq, state, resumed: true, server_tick: self.tick, ..s.accept.clone() };
        let mut out = vec![];
        s.accept.write(&mut out);
        self.link.send(&out, addr);
        self.stats.resumed += 1;
        self.reindex();
        true
    }

    /// Joining players whose collision tiles are in: spawn and accept.
    fn pump_joining(&mut self) {
        let mut ready = vec![];
        for (k, j) in self.joining.iter_mut().enumerate() {
            let c = DVec3::new(j.spawn[0], 0.0, j.spawn[1]);
            j.cw.pump(c);
            // Every tile the spawn's blocking load wants (so spawning never waits on a cook).
            let r = eo_player::world::TILE_SIZE_M;
            let k0 = eo_player::world::TileKey::of(c.x, c.z);
            let all = (-2..=2).all(|dx| {
                (-2..=2).all(|dz| {
                    let k = eo_player::world::TileKey { x: k0.x + dx, z: k0.z + dz };
                    k.distance_to(c.x, c.z) > eo_player::world::LOAD_RADIUS_M || j.cw.is_resident(k.x as f64 * r + r / 2.0, k.z as f64 * r + r / 2.0)
                })
            });
            if (all || j.since.elapsed().as_secs_f64() > 10.0) && ready.len() < 2 {
                ready.push(k); // at most two spawns a tick
            }
        }
        for k in ready.into_iter().rev() {
            let mut j = self.joining.swap_remove(k);
            let (player, cars) = spawn_player(&mut j.cw, j.spawn[0], j.spawn[1], j.spawn[2], j.driving);
            let player_id = self.next_player;
            self.next_player += 1;
            let resume_key = keyed(self.secret, &[player_id as u64, j.connection, 3]);
            let mut state = Vec::with_capacity(512);
            write_player_state(&player.state(&j.cw), &mut state);
            let accept = Accept { connection: j.connection, player_id, resume_key, tick_hz: proto::TICK_HZ as u8, snapshot_hz: self.cfg.snapshot_hz as u8, server_tick: self.tick, input_seq: 0, state, car_spawns: cars, resumed: false };
            let mut out = vec![];
            accept.write(&mut out);
            self.link.send(&out, j.addr);
            let pos = player.position(&j.cw);
            let mut conn = Connection::new();
            conn.send_reliable({
                let mut m = vec![proto::msg::PLAYER_JOINED];
                m.put_u32(player_id);
                m.extend_from_slice(j.account.display_name.as_bytes());
                m
            });
            self.sessions.push(Session {
                player_id,
                account: j.account,
                resume_key,
                addr: j.addr,
                connection: j.connection,
                conn,
                online: true,
                offline_since: None,
                player,
                cw: j.cw,
                last_seq: 0,
                pending: BTreeMap::new(),
                last_input: NetInput::default(),
                gap_since: None,
                check: None,
                client_applied: 0,
                corr_id: 0,
                correction: None,
                tokens: 10.0,
                rep: Replicator::new(),
                accept,
                stats: SessionStats::default(),
                pos,
                nudge: None,
                ping_echo: None,
            });
            self.stats.joined += 1;
            self.reindex();
        }
    }

    /// Everything replicable this tick, and each online client's snapshot.
    fn replicate(&mut self) {
        let mut ents: Vec<(u32, QState)> = Vec::with_capacity(4096);
        for r in &self.regions.world.regions {
            let [ox, oz] = r.out.origin;
            for a in &r.out.vehicles {
                ents.push((
                    net_id(CLASS_CAR, a.id),
                    QState {
                        x: q_pos(ox + a.pos[0] as f64),
                        y: 0,
                        z: q_pos(oz + a.pos[2] as f64),
                        yaw: q_angle(a.heading as f64),
                        speed: q_speed(a.speed as f64),
                        anim: ((a.anim as f64).rem_euclid(16.0) * 16.0) as u8,
                        kind: a.kind as u8,
                        flags: a.flags,
                        ..Default::default()
                    },
                ));
            }
            for a in &r.out.walkers {
                ents.push((
                    net_id(CLASS_WALKER, a.id),
                    QState {
                        x: q_pos(ox + a.pos[0] as f64),
                        y: 0,
                        z: q_pos(oz + a.pos[2] as f64),
                        yaw: q_angle(a.heading as f64),
                        speed: q_speed(a.speed as f64),
                        anim: ((a.anim as f64).rem_euclid(std::f64::consts::TAU) / std::f64::consts::TAU * 256.0) as u8,
                        kind: a.kind as u8,
                        flags: a.flags,
                        ..Default::default()
                    },
                ));
            }
        }
        for s in &self.sessions {
            player_entities(s, &mut ents);
        }
        self.grid.rebuild(self.regions.zone, ents);
        let tick = self.tick;
        let sim_time = self.regions.world.time;
        let budget = self.cfg.budget_bytes_per_s / self.cfg.snapshot_hz.max(1) as usize;
        let (grid, radii, link) = (&self.grid, self.cfg.radii, &self.link);
        let scratch: Mutex<Vec<Vec<Desired>>> = Mutex::new(vec![]);
        self.pool.for_each_mut(&mut self.sessions, |_, s| {
            if !s.online {
                return;
            }
            let mut desired = scratch.lock().unwrap().pop().unwrap_or_default();
            let you = net_id(CLASS_PLAYER, s.player_id);
            grid.query(s.pos.x, s.pos.z, &radii, |id| s.rep.holds(id), &mut desired, Some(you));
            let own = Own { ack_input: s.last_seq, check: check_of(s.last_seq, &s.player, &s.cw) };
            let correction = if s.client_applied != s.corr_id { s.correction.clone() } else { None };
            let (echo_ms, hold_ms) = s.ping_echo.map(|(e, at)| (e, at.elapsed().as_millis().min(65535) as u16)).unwrap_or((0, 0));
            let header = SnapHeader { tick, sim_time, own, correction, you, echo_ms, hold_ms, ..Default::default() };
            let (body, st) = s.rep.build(header, &desired, budget);
            scratch.lock().unwrap().push(desired);
            s.stats.snapshots += 1;
            s.stats.deferred += st.deferred as u64;
            s.stats.entities = st.entities;
            let room = MTU - 40;
            let count = body.len().div_ceil(room).max(1);
            let now = Instant::now();
            for (k, chunk) in body.chunks(room).enumerate() {
                let mut out = Vec::with_capacity(chunk.len() + 48);
                proto::begin_connected(Kind::Snapshot, s.connection, &mut out);
                s.conn.begin_packet(&mut out);
                out.put_u32(st.seq);
                out.put_u8(k as u8);
                out.put_u8(count as u8);
                out.extend_from_slice(chunk);
                s.count_bytes(out.len() + eo_net::link::UDP_IP_OVERHEAD as usize, now);
                link.send(&out, s.addr);
            }
        });
    }
}

/// A player's body and cars as entities.
fn player_entities(s: &Session, ents: &mut Vec<(u32, QState)>) {
    let p = &s.player;
    let driving = matches!(p.mode, Mode::Driving(_));
    let feet = p.character.feet();
    ents.push((
        net_id(CLASS_PLAYER, s.player_id),
        QState {
            x: q_pos(feet.x),
            y: q_pos(feet.y),
            z: q_pos(feet.z),
            yaw: q_angle(p.facing_yaw),
            speed: q_speed(p.ground_speed),
            kind: (s.player_id % 251) as u8,
            flags: driving as u16 | (p.character.crouched as u16) << 1 | (!s.online as u16) << 2,
            ..Default::default()
        },
    ));
    for (k, v) in p.vehicles.iter().enumerate().take(PLAYER_BLOCK_CARS as usize) {
        let (_, rot) = v.pose(&s.cw);
        let d = v.datum(&s.cw);
        let (yaw, pitch, roll) = ypr(rot);
        let steer = v.controller.wheels().first().map(|w| w.steering as f64).unwrap_or(0.0);
        let braking = v.controls.brake > 0.0 || v.controls.handbrake;
        let driven = p.mode == Mode::Driving(k);
        ents.push((
            net_id(CLASS_PLAYER_CAR, s.player_id * PLAYER_BLOCK_CARS + k as u32),
            QState {
                x: q_pos(d.x),
                y: q_pos(d.y),
                z: q_pos(d.z),
                yaw: q_angle(yaw),
                tilt: q_angle(pitch) as u32 | (q_angle(roll) as u32) << 16,
                speed: q_speed(v.forward_speed(&s.cw)),
                kind: (s.player_id % 251) as u8,
                flags: braking as u16 | (driven as u16) << 1,
                steer: (steer / 0.6 * 127.0).round().clamp(-127.0, 127.0) as i8,
                ..Default::default()
            },
        ));
    }
}

/// Yaw (about y), pitch (about x), roll (about z) of a rotation, intrinsic Y-X-Z
/// (`compose_ypr` inverts it).
pub fn ypr(r: DQuat) -> (f64, f64, f64) {
    let f = r * DVec3::Z;
    let yaw = f.x.atan2(f.z);
    let pitch = (-f.y).clamp(-1.0, 1.0).asin();
    let q0 = DQuat::from_rotation_y(yaw) * DQuat::from_rotation_x(pitch);
    let u = q0.inverse() * (r * DVec3::Y);
    let roll = (-u.x).atan2(u.y);
    (yaw, pitch, roll)
}

pub fn compose_ypr(yaw: f64, pitch: f64, roll: f64) -> DQuat {
    DQuat::from_rotation_y(yaw) * DQuat::from_rotation_x(pitch) * DQuat::from_rotation_z(roll)
}

/// Snapshot class helpers for tools.
pub fn class_name(id: u32) -> &'static str {
    match snapshot::class_of(id) {
        CLASS_CAR => "car",
        CLASS_WALKER => "walker",
        CLASS_PLAYER => "player",
        _ => "player car",
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.io_stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.io.take() {
            let _ = t.join();
        }
    }
}

/// Load what a server needs (the city physics world and the simulation source).
pub fn load(streets: bool) -> anyhow::Result<(Arc<GameWorld>, Arc<eo_sim::native::NativeSource>)> {
    load_with(streets, None)
}

/// As `load`, with the grade-separated roads (`ServerConfig::structures`) in
/// the world; the simulation's lanes then take their levels from them
/// (`GameWorld::way_levels`, as eo-app's own sim and `--connect local`
/// server do), so the cars on a deck are on its level. Without them the
/// table is empty and the source is as `load`'s.
pub fn load_with(streets: bool, structures: Option<eo_structures::StructureOptions>) -> anyhow::Result<(Arc<GameWorld>, Arc<eo_sim::native::NativeSource>)> {
    let paths = eo_data::DataPaths::discover()?;
    let (g, s) = std::thread::scope(|sc| {
        let p2 = paths.clone();
        let src = sc.spawn(move || eo_sim::native::load(&p2));
        let g = GameWorld::load_with(&paths, streets, structures);
        (g, src.join().expect("simulation source thread"))
    });
    let g = g?;
    let s = s?.with_levels(g.way_levels());
    Ok((Arc::new(g), Arc::new(s)))
}

/// Make the reader for a packet (tests).
pub fn reader(data: &[u8]) -> ByteReader<'_> {
    ByteReader::new(data)
}
