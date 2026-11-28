//! The client's network side, on its own thread: the handshake, sending the
//! inputs the render thread hands it (every unacknowledged one, each
//! packet), receiving and reassembling snapshots, decoding them against
//! their baselines, the server clock estimate and the interpolation delay,
//! timeouts and reconnecting with the player's resume key.
//!
//! The render thread never waits on the network: every call below takes one
//! short lock and copies or clones `Arc`s; decoding and socket calls happen
//! on the network thread.

use crate::conn::Connection;
use crate::link::{Conditions, Link};
use crate::proto::{self, Accept, Challenge, DenyReason, Hello, Kind, Resume, SpawnHint};
use crate::snapshot::{Correction, Decoded, Own, Store};
use crate::wire::{Check, InputPacket, MAX_INPUTS, NetInput};
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct ClientConfig {
    pub vehicle: eo_player::vehicle::VehicleConfig,
    pub server: SocketAddr,
    pub bind: String,
    pub token: Vec<u8>,
    pub spawn: Option<SpawnHint>,
    pub conditions: Conditions,
    pub seed: u64,
    /// No packet from the server for this long: the connection is lost.
    pub timeout_s: f64,
    /// Reconnect (resuming the same player) after a timeout.
    pub reconnect: bool,
    /// Input packets a second at most (the render thread may produce more steps).
    pub send_hz: f64,
    /// This client's grade-separated roads (`--structures`, `--tunnels`), as
    /// its collision world was built; the server refuses other ones.
    pub structures: Option<eo_structures::StructureOptions>,
}

impl ClientConfig {
    pub fn new(server: SocketAddr, token: &str) -> Self {
        Self { vehicle: Default::default(), server, bind: "127.0.0.1:0".into(), token: token.as_bytes().to_vec(), spawn: None, conditions: Conditions::NONE, seed: 1, timeout_s: 3.0, reconnect: true, send_hz: 60.0, structures: None }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Connecting,
    Connected,
    Reconnecting,
    Denied(u8),
    Closed,
}

impl Phase {
    pub fn label(&self) -> &'static str {
        match self {
            Phase::Connecting => "Connecting",
            Phase::Connected => "Online",
            Phase::Reconnecting => "Reconnecting",
            Phase::Denied(_) => "Refused",
            Phase::Closed => "Offline",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Status {
    pub phase: Phase,
    pub player_id: u32,
    pub rtt_ms: f64,
    pub rtt_var_ms: f64,
    pub interp_delay_ms: f64,
    pub snapshots: u64,
    pub snapshots_lost: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub reconnects: u32,
    pub packets_lost: u64,
}

pub enum Event {
    /// Joined (or rejoined) the server: the player's state and cars.
    Accepted(Accept),
    Correction(Correction),
    Reliable(Vec<u8>),
}

/// The world to draw this frame: two snapshots and where between them.
#[derive(Clone)]
pub struct View {
    /// The snapshot before `a` and the one after `b`, when held (the other players' smooth path).
    pub pa: Option<Arc<Decoded>>,
    pub pb: Option<Arc<Decoded>>,
    pub a: Arc<Decoded>,
    pub b: Arc<Decoded>,
    /// Render tick (server ticks, fractional).
    pub tick: f64,
    /// The server's simulation clock at that tick, seconds.
    pub sim_time: f64,
}

struct Frag {
    parts: Vec<Option<Vec<u8>>>,
    got: usize,
    first: Instant,
}

struct Inner {
    phase: Phase,
    conn: Connection,
    connection: u64,
    client_salt: u64,
    challenge: Option<Challenge>,
    last_hello: Option<Instant>,
    accept: Option<Accept>,
    resume: Option<Resume>,
    store: Store,
    frames: VecDeque<Arc<Decoded>>,
    frags: HashMap<u32, Frag>,
    outbox: VecDeque<(u32, NetInput)>,
    check: Option<Check>,
    applied: u16,
    ack_input: u32,
    own: Option<Own>,
    events: VecDeque<Event>,
    dirty: bool,
    last_send: Instant,
    offset: Option<f64>,
    jitter_ticks: f64,
    snapshot_hz: f64,
    snapshots: u64,
    snapshots_lost: u64,
    newest_seq: u32,
    reconnects: u32,
    /// Smoothed ping from the echoed input timestamps (None before the first).
    ping_ms: Option<f64>,
}

fn ms16(t0: Instant, now: Instant) -> u16 {
    ((now - t0).as_millis() as u64 & 0xFFFF) as u16
}

pub struct NetClient {
    inner: Arc<Mutex<Inner>>,
    pub link: Arc<Link>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    t0: Instant,
    cfg: ClientConfig,
}

fn salt(seed: u64) -> u64 {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
    let mut x = t ^ seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (std::process::id() as u64) << 32;
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    x | 1
}

impl NetClient {
    pub fn connect(cfg: ClientConfig) -> std::io::Result<Self> {
        let link = Arc::new(Link::bind(&cfg.bind, cfg.conditions, cfg.seed)?);
        let now = Instant::now();
        let inner = Arc::new(Mutex::new(Inner {
            phase: Phase::Connecting,
            conn: Connection::new(),
            connection: 0,
            client_salt: salt(cfg.seed),
            challenge: None,
            last_hello: None,
            accept: None,
            resume: None,
            store: Store::default(),
            frames: VecDeque::new(),
            frags: HashMap::new(),
            outbox: VecDeque::new(),
            check: None,
            applied: 0,
            ack_input: 0,
            own: None,
            events: VecDeque::new(),
            dirty: false,
            last_send: now,
            offset: None,
            jitter_ticks: 1.0,
            snapshot_hz: 20.0,
            snapshots: 0,
            snapshots_lost: 0,
            newest_seq: 0,
            reconnects: 0,
            ping_ms: None,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let t0 = now;
        let thread = {
            let (inner, link, stop, cfg) = (inner.clone(), link.clone(), stop.clone(), cfg.clone());
            std::thread::Builder::new().name("eo-net-client".into()).spawn(move || run(inner, link, stop, cfg, t0)).expect("spawn net client")
        };
        Ok(Self { inner, link, stop, thread: Some(thread), t0, cfg })
    }

    pub fn config(&self) -> &ClientConfig {
        &self.cfg
    }

    pub fn status(&self) -> Status {
        let g = self.inner.lock().unwrap();
        let ls = self.link.stats();
        Status {
            phase: g.phase,
            player_id: g.accept.as_ref().map_or(0, |a| a.player_id),
            rtt_ms: g.ping_ms.unwrap_or(g.conn.stats.rtt_ms),
            rtt_var_ms: g.conn.stats.rtt_var_ms,
            interp_delay_ms: delay_ticks(&g) / proto::TICK_HZ as f64 * 1e3,
            snapshots: g.snapshots,
            snapshots_lost: g.snapshots_lost,
            bytes_in: ls.recv_bytes,
            bytes_out: ls.sent_bytes,
            reconnects: g.reconnects,
            packets_lost: g.conn.stats.packets_lost,
        }
    }

    /// Render thread: the inputs just stepped, the check of the newest, and
    /// the newest correction applied. Sent by the network thread at once.
    pub fn push_inputs(&self, inputs: &[(u32, NetInput)], check: Option<Check>, applied: u16) {
        if inputs.is_empty() && check.is_none() {
            return;
        }
        let mut g = self.inner.lock().unwrap();
        for &(seq, i) in inputs {
            if seq > g.ack_input {
                g.outbox.push_back((seq, i));
            }
        }
        while g.outbox.len() > 4 * MAX_INPUTS {
            g.outbox.pop_front();
        }
        if check.is_some() {
            g.check = check;
        }
        g.applied = applied;
        g.dirty = true;
    }

    /// Send an application message on the reliable channel (an emote, a
    /// fast-travel request: `proto::msg`); it rides the next input packets.
    pub fn command(&self, m: Vec<u8>) {
        let mut g = self.inner.lock().unwrap();
        if g.phase == Phase::Connected {
            g.conn.send_reliable(m);
            g.dirty = true;
        }
    }

    pub fn take_events(&self) -> Vec<Event> {
        self.inner.lock().unwrap().events.drain(..).collect()
    }

    /// The newest report on this client's own player.
    pub fn own(&self) -> Option<Own> {
        self.inner.lock().unwrap().own
    }

    /// The server tick estimated for now (None before the first snapshot).
    pub fn server_tick(&self, now: Instant) -> Option<f64> {
        let g = self.inner.lock().unwrap();
        g.offset.map(|o| (now - self.t0).as_secs_f64() * proto::TICK_HZ as f64 + o)
    }

    /// What to draw now: the snapshots around the render tick.
    pub fn view(&self, now: Instant) -> Option<View> {
        let tick = {
            let g = self.inner.lock().unwrap();
            (now - self.t0).as_secs_f64() * proto::TICK_HZ as f64 + g.offset? - delay_ticks(&g)
        };
        self.view_at(tick)
    }

    /// The render tick `view` would use now.
    pub fn render_tick(&self, now: Instant) -> Option<f64> {
        let g = self.inner.lock().unwrap();
        Some((now - self.t0).as_secs_f64() * proto::TICK_HZ as f64 + g.offset? - delay_ticks(&g))
    }

    /// The world at a given server tick (captures freeze two clients on the same tick).
    pub fn view_at(&self, tick: f64) -> Option<View> {
        let g = self.inner.lock().unwrap();
        let n = g.frames.len();
        if n == 0 {
            return None;
        }
        // a.tick <= tick < b.tick; before the oldest: the oldest; past the newest: extrapolate from it.
        let (ia, ib) = match g.frames.iter().rposition(|f| f.header.tick as f64 <= tick) {
            None => (0, 0),
            Some(i) if i + 1 < n => (i, i + 1),
            Some(i) => (i.saturating_sub(1), i),
        };
        let (a, b) = (g.frames[ia].clone(), g.frames[ib].clone());
        let pa = ia.checked_sub(1).map(|i| g.frames[i].clone());
        let pb = g.frames.get(ib + 1).cloned();
        let (ta, tb) = (a.header.tick as f64, b.header.tick as f64);
        let u = if tb > ta { ((tick - ta) / (tb - ta)).clamp(0.0, 1.0) } else { 0.0 };
        let sim_time = if tick > tb { b.header.sim_time + (tick - tb) / proto::TICK_HZ as f64 } else { a.header.sim_time + (b.header.sim_time - a.header.sim_time) * u };
        Some(View { pa, pb, a, b, tick, sim_time })
    }

    pub fn player_id(&self) -> u32 {
        self.inner.lock().unwrap().accept.as_ref().map_or(0, |a| a.player_id)
    }

    /// Test hook: drop every datagram both ways while `on` (a pulled cable).
    pub fn set_blackout(&self, on: bool) {
        self.link.blackout.store(on, Ordering::Relaxed);
    }

    /// Leave politely (the server frees the player at once).
    pub fn disconnect(&mut self) {
        {
            let mut g = self.inner.lock().unwrap();
            if g.phase == Phase::Connected {
                let mut out = Vec::with_capacity(64);
                proto::begin_connected(Kind::Disconnect, g.connection, &mut out);
                g.conn.begin_packet(&mut out);
                for _ in 0..3 {
                    self.link.send(&out, self.cfg.server);
                }
            }
            g.phase = Phase::Closed;
        }
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for NetClient {
    fn drop(&mut self) {
        self.disconnect();
    }
}

fn delay_ticks(g: &Inner) -> f64 {
    // Two snapshot intervals (one lost snapshot is bridged) plus the arrival jitter.
    let interval = proto::TICK_HZ as f64 / g.snapshot_hz.max(1.0);
    (2.0 * interval + 2.5 * g.jitter_ticks + 1.0).clamp(3.0, 30.0)
}

fn run(inner: Arc<Mutex<Inner>>, link: Arc<Link>, stop: Arc<AtomicBool>, cfg: ClientConfig, t0: Instant) {
    let mut buf = vec![0u8; 2048];
    let send_interval = Duration::from_secs_f64(1.0 / cfg.send_hz.max(1.0));
    while !stop.load(Ordering::Relaxed) {
        if let Some((n, from)) = link.recv(&mut buf) {
            if from == cfg.server {
                handle(&inner, &link, &cfg, &buf[..n], t0);
            }
        }
        let now = Instant::now();
        let mut g = inner.lock().unwrap();
        match g.phase {
            Phase::Connecting | Phase::Reconnecting => {
                if g.last_hello.is_none_or(|t| now - t > Duration::from_millis(100)) {
                    g.last_hello = Some(now);
                    let hello = Hello { vehicle: cfg.vehicle, client_salt: g.client_salt, version: proto::PROTOCOL_VERSION, ground: proto::ground_scale_ppm(), structures: proto::structures_code(cfg.structures), token: cfg.token.clone(), spawn: cfg.spawn, resume: g.resume, server_salt: 0, cookie: 0 };
                    let mut out = Vec::with_capacity(proto::REQUEST_PAD);
                    match &g.challenge {
                        Some(c) => {
                            let h = Hello { server_salt: c.server_salt, cookie: c.cookie, ..hello };
                            h.write(Kind::Response, &mut out);
                        }
                        None => hello.write(Kind::Request, &mut out),
                    }
                    drop(g);
                    link.send(&out, cfg.server);
                    continue;
                }
            }
            Phase::Connected => {
                let lost = (now - g.conn.last_recv).as_secs_f64() > cfg.timeout_s;
                if lost {
                    if cfg.reconnect {
                        g.phase = Phase::Reconnecting;
                        g.reconnects += 1;
                        g.client_salt = salt(cfg.seed.wrapping_add(g.reconnects as u64));
                        g.challenge = None;
                        g.last_hello = None;
                        g.conn = Connection::new();
                        g.frags.clear();
                        g.store = Store::default();
                        g.frames.clear();
                        g.offset = None;
                    } else {
                        g.phase = Phase::Closed;
                    }
                    continue;
                }
                let due = (g.dirty && now - g.last_send >= send_interval) || now - g.last_send >= Duration::from_millis(50);
                if due {
                    let first = g.outbox.front().map(|e| e.0).unwrap_or(g.ack_input + 1);
                    let inputs: Vec<NetInput> = g.outbox.iter().take(MAX_INPUTS).map(|e| e.1).collect();
                    let p = InputPacket { ack_snapshot: g.newest_seq, correction_applied: g.applied, check: g.check, first_seq: first, inputs, ping_ms: ms16(t0, now) };
                    let mut out = Vec::with_capacity(200);
                    proto::begin_connected(Kind::Input, g.connection, &mut out);
                    g.conn.begin_packet(&mut out);
                    p.write(&mut out);
                    g.dirty = false;
                    g.last_send = now;
                    drop(g);
                    link.send(&out, cfg.server);
                    continue;
                }
            }
            Phase::Denied(_) | Phase::Closed => {
                drop(g);
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
        }
        // Forget unfinished fragment sets.
        g.frags.retain(|_, f| now - f.first < Duration::from_millis(500));
    }
}

fn handle(inner: &Arc<Mutex<Inner>>, _link: &Link, cfg: &ClientConfig, data: &[u8], t0: Instant) {
    let Some((kind, mut r)) = proto::peek(data) else { return };
    let mut g = inner.lock().unwrap();
    match kind {
        Kind::Challenge => {
            if let Ok(c) = Challenge::read(&mut r) {
                if c.client_salt == g.client_salt && matches!(g.phase, Phase::Connecting | Phase::Reconnecting) && g.challenge.is_none() {
                    g.challenge = Some(c);
                    g.last_hello = None;
                }
            }
        }
        Kind::Deny => {
            if let (Ok(s), Ok(reason)) = (r.u64(), r.u8()) {
                if s == g.client_salt {
                    g.phase = Phase::Denied(reason);
                    if reason == DenyReason::World as u8 {
                        eprintln!("net: the server refused this client: its ground plan (street size) is not x{} (start both with the same --ground-scale)", eo_data::groundscale::scale());
                    }
                    if reason == DenyReason::Structures as u8 {
                        let theirs = r.u8().map(proto::structures_switches).unwrap_or("other structures");
                        eprintln!(
                            "net: the server refused this client: its world runs with {theirs}, this client with {} (start both with the same --structures and --tunnels)",
                            proto::structures_switches(proto::structures_code(cfg.structures))
                        );
                    }
                }
            }
        }
        Kind::Accept => {
            let Ok(a) = Accept::read(&mut r) else { return };
            if !matches!(g.phase, Phase::Connecting | Phase::Reconnecting) || g.challenge.as_ref().is_none_or(|c| c.client_salt ^ c.server_salt != a.connection) {
                return;
            }
            g.connection = a.connection;
            g.phase = Phase::Connected;
            g.conn = Connection::new();
            g.snapshot_hz = a.snapshot_hz as f64;
            g.resume = Some(Resume { player_id: a.player_id, key: a.resume_key });
            g.ack_input = a.input_seq;
            g.outbox.clear();
            g.applied = 0;
            g.check = None;
            g.accept = Some(a.clone());
            g.events.push_back(Event::Accepted(a));
        }
        Kind::Snapshot | Kind::KeepAlive => {
            if g.phase != Phase::Connected || r.u64().ok() != Some(g.connection) {
                return;
            }
            let Ok(Some((_h, msgs))) = g.conn.receive(&mut r) else { return };
            for m in msgs {
                g.events.push_back(Event::Reliable(m));
            }
            if kind == Kind::KeepAlive {
                return;
            }
            let (Ok(seq), Ok(idx), Ok(count)) = (r.u32(), r.u8(), r.u8()) else { return };
            let part = r.rest().to_vec();
            if count == 0 || idx >= count || seq <= g.newest_seq {
                return;
            }
            let whole = if count == 1 {
                Some(part)
            } else {
                let f = g.frags.entry(seq).or_insert_with(|| Frag { parts: vec![None; count as usize], got: 0, first: Instant::now() });
                if f.parts.len() != count as usize || f.parts[idx as usize].is_some() {
                    return;
                }
                f.parts[idx as usize] = Some(part);
                f.got += 1;
                if f.got == count as usize {
                    let f = g.frags.remove(&seq).unwrap();
                    Some(f.parts.into_iter().flatten().flatten().collect::<Vec<u8>>())
                } else {
                    None
                }
            };
            let Some(body) = whole else { return };
            match g.store.decode(&body) {
                Ok(d) => {
                    let now = Instant::now();
                    if g.newest_seq > 0 && d.header.seq > g.newest_seq + 1 {
                        g.snapshots_lost += (d.header.seq - g.newest_seq - 1) as u64;
                    }
                    g.newest_seq = d.header.seq;
                    g.snapshots += 1;
                    // Server clock: the snapshot's tick plus half a round trip, against local time.
                    let local = (now - t0).as_secs_f64() * proto::TICK_HZ as f64;
                    // Ping: our echoed timestamp, less the time the server held it.
                    let p = ms16(t0, now).wrapping_sub(d.header.echo_ms).wrapping_sub(d.header.hold_ms) as f64;
                    if d.header.echo_ms != 0 && p < 5000.0 {
                        g.ping_ms = Some(match g.ping_ms {
                            None => p,
                            Some(o) => 0.875 * o + 0.125 * p,
                        });
                    }
                    let rtt = g.ping_ms.unwrap_or(g.conn.stats.rtt_ms);
                    let sample = d.header.tick as f64 + rtt / 2.0 / 1e3 * proto::TICK_HZ as f64 - local;
                    match g.offset {
                        None => g.offset = Some(sample),
                        Some(o) => {
                            let e = sample - o;
                            if e.abs() > 30.0 {
                                g.offset = Some(sample);
                            } else {
                                // Late arrivals pull slowly, early ones faster (the clock should track the earliest path).
                                g.offset = Some(o + e * if e > 0.0 { 0.1 } else { 0.02 });
                                g.jitter_ticks = 0.95 * g.jitter_ticks + 0.05 * e.abs();
                            }
                        }
                    }
                    // Inputs the server ran are not resent.
                    let ack = d.header.own.ack_input;
                    if ack > g.ack_input {
                        g.ack_input = ack;
                    }
                    let a = g.ack_input;
                    while g.outbox.front().is_some_and(|e| e.0 <= a) {
                        g.outbox.pop_front();
                    }
                    g.own = Some(d.header.own);
                    if let Some(c) = &d.header.correction {
                        if c.id != g.applied {
                            g.events.push_back(Event::Correction(c.clone()));
                        }
                    }
                    // Keep frames in tick order (a late snapshot older than the newest is dropped by the seq check above).
                    g.frames.push_back(d);
                    while g.frames.len() > 40 {
                        g.frames.pop_front();
                    }
                    g.dirty = true;
                }
                Err(_) => {
                    // A baseline we no longer hold: our next ack names a newer one.
                }
            }
        }
        _ => {}
    }
}

/// kB/s from a byte count over a time.
pub fn kbps(bytes: u64, secs: f64) -> f64 {
    bytes as f64 / 1000.0 / secs.max(1e-9)
}
