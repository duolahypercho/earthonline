//! Scripted bot clients: real protocol clients (handshake, inputs over UDP,
//! snapshots decoded and acknowledged against their baselines) that walk or
//! drive along the city's streets, many to a thread and a socket. They do
//! not predict (no physics on the bot side): each steers from the position
//! the server reports for it. Used by the server bench and the tests.

use eo_net::conn::Connection;
use eo_net::link::{Conditions, Dice, Link};
use eo_net::proto::{self, Accept, Challenge, Hello, Kind, SpawnHint};
use eo_net::snapshot::{CLASS_PLAYER_CAR, Store, net_id};
use eo_net::wire::{InputPacket, MAX_INPUTS, NetInput};
use eo_player::StepInput;
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Streets bots use: the ordinary classes (no freeways, bridges or tunnels).
pub fn bot_roads(city: &eo_data::City) -> Vec<Vec<[f64; 2]>> {
    city.roads
        .iter()
        .filter(|r| !r.bridge && !r.tunnel && matches!(r.highway.as_str(), "primary" | "secondary" | "tertiary" | "residential" | "unclassified") && r.points64.len() >= 2)
        .map(|r| r.points64.clone())
        .collect()
}

/// Road ends within 12 m of each other, gridded.
pub struct RoadGraph {
    pub roads: Vec<Vec<[f64; 2]>>,
    ends: HashMap<(i32, i32), Vec<(u32, bool)>>,
}

impl RoadGraph {
    pub fn new(roads: Vec<Vec<[f64; 2]>>) -> Self {
        let mut ends: HashMap<(i32, i32), Vec<(u32, bool)>> = HashMap::new();
        for (i, r) in roads.iter().enumerate() {
            for (p, at_end) in [(r[0], false), (*r.last().unwrap(), true)] {
                ends.entry(((p[0] / 12.0).floor() as i32, (p[1] / 12.0).floor() as i32)).or_default().push((i as u32, at_end));
            }
        }
        Self { roads, ends }
    }

    /// A road starting (or ending) near `p`, other than `not`; (road, reversed).
    fn next(&self, p: [f64; 2], not: u32, dice: &mut Dice) -> Option<(u32, bool)> {
        let (cx, cz) = ((p[0] / 12.0).floor() as i32, (p[1] / 12.0).floor() as i32);
        let mut c = vec![];
        for dx in -1..=1 {
            for dz in -1..=1 {
                if let Some(l) = self.ends.get(&(cx + dx, cz + dz)) {
                    for &(r, at_end) in l {
                        let q = if at_end { *self.roads[r as usize].last().unwrap() } else { self.roads[r as usize][0] };
                        if r != not && (q[0] - p[0]).hypot(q[1] - p[1]) < 12.0 {
                            c.push((r, at_end));
                        }
                    }
                }
            }
        }
        if c.is_empty() {
            return None;
        }
        Some(c[((dice.next_f64() * c.len() as f64) as usize).min(c.len() - 1)])
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Spawn {
    /// Spread over the whole city, at least 150 m apart.
    Spread,
    /// Within `radius` of a point.
    Around { x: f64, z: f64, radius: f64 },
}

#[derive(Clone, Debug)]
pub struct BotConfig {
    pub server: SocketAddr,
    pub count: usize,
    pub threads: usize,
    pub spawn: Spawn,
    /// Share of bots that drive (the rest walk).
    pub drive_share: f64,
    /// Target driving speed, m/s.
    pub drive_mps: f64,
    pub seed: u64,
    pub conditions: Conditions,
    /// Input packets a second per bot (each carries every unacknowledged input).
    pub send_hz: f64,
}

#[derive(Clone, Debug, Default)]
pub struct BotStats {
    pub connected: usize,
    pub snapshots: u64,
    pub decode_misses: u64,
    /// Bytes received per bot per whole second (client side, UDP/IP headers included).
    pub bytes_per_s: Vec<u64>,
    pub positions: Vec<[f64; 3]>,
    pub driving: Vec<bool>,
    pub speeds: Vec<f64>,
}

#[derive(PartialEq)]
enum Phase {
    Hello,
    Connected,
}

struct Bot {
    salt: u64,
    phase: Phase,
    challenge: Option<Challenge>,
    last_hello: Option<Instant>,
    connection: u64,
    conn: Connection,
    store: Store,
    newest: u32,
    seq: u32,
    ack_input: u32,
    outbox: VecDeque<(u32, NetInput)>,
    spawn: SpawnHint,
    player_id: u32,
    pos: [f64; 3],
    driving: bool,
    car_yaw: f64,
    speed: f64,
    road: u32,
    reversed: bool,
    vertex: usize,
    dice: Dice,
    frags: HashMap<u32, (Vec<Option<Vec<u8>>>, usize)>,
    sec_bytes: u64,
    sec_start: Instant,
    bytes_per_s: Vec<u64>,
    stuck: f64,
    reverse_for: f64,
    sprint: bool,
    toggle_at: f64,
}

fn bot_salt(seed: u64, i: usize) -> u64 {
    let mut x = seed ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    x ^= x >> 31;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^ (x >> 29) | 1
}

/// Spawn points and starting roads for `n` bots.
pub fn plan_spawns(graph: &RoadGraph, n: usize, spawn: Spawn, seed: u64) -> Vec<(u32, usize, [f64; 2])> {
    let mut dice = Dice::new(seed.wrapping_add(17));
    let mut order: Vec<u32> = (0..graph.roads.len() as u32).collect();
    for i in (1..order.len()).rev() {
        let j = (dice.next_f64() * (i + 1) as f64) as usize;
        order.swap(i, j.min(i));
    }
    let mut out: Vec<(u32, usize, [f64; 2])> = vec![];
    for min_gap in [150.0, 60.0, 20.0, 0.0] {
        for &r in &order {
            if out.len() >= n {
                break;
            }
            let pts = &graph.roads[r as usize];
            let v = pts.len() / 2;
            let p = pts[v];
            if let Spawn::Around { x, z, radius } = spawn {
                if (p[0] - x).hypot(p[1] - z) > radius {
                    continue;
                }
            }
            if out.iter().any(|o| o.0 == r || (o.2[0] - p[0]).hypot(o.2[1] - p[1]) < min_gap) {
                continue;
            }
            out.push((r, v, p));
        }
    }
    out
}

pub struct BotRun {
    stop: Arc<AtomicBool>,
    threads: Vec<std::thread::JoinHandle<()>>,
    pub stats: Arc<Mutex<BotStats>>,
}

impl BotRun {
    pub fn start(cfg: BotConfig, graph: Arc<RoadGraph>) -> std::io::Result<Self> {
        let spawns = plan_spawns(&graph, cfg.count, cfg.spawn, cfg.seed);
        let stop = Arc::new(AtomicBool::new(false));
        let stats = Arc::new(Mutex::new(BotStats { positions: vec![[0.0; 3]; spawns.len()], driving: vec![false; spawns.len()], speeds: vec![0.0; spawns.len()], ..Default::default() }));
        let mut threads = vec![];
        let per = spawns.len().div_ceil(cfg.threads.max(1)).max(1);
        for (t, chunk) in spawns.chunks(per).enumerate() {
            let link = Link::bind("127.0.0.1:0", cfg.conditions, cfg.seed ^ t as u64)?;
            link.set_read_timeout(Duration::from_millis(1));
            let base = t * per;
            let mut dice = Dice::new(cfg.seed.wrapping_mul(31).wrapping_add(t as u64));
            let bots: Vec<Bot> = chunk
                .iter()
                .enumerate()
                .map(|(k, &(road, vertex, p))| {
                    let i = base + k;
                    let pts = &graph.roads[road as usize];
                    let q = pts[(vertex + 1).min(pts.len() - 1)];
                    let yaw = (q[0] - p[0]).atan2(q[1] - p[1]);
                    let driving = dice.next_f64() < cfg.drive_share;
                    Bot {
                        salt: bot_salt(cfg.seed, i),
                        phase: Phase::Hello,
                        challenge: None,
                        last_hello: None,
                        connection: 0,
                        conn: Connection::new(),
                        store: Store::default(),
                        newest: 0,
                        seq: 0,
                        ack_input: 0,
                        outbox: VecDeque::new(),
                        spawn: SpawnHint { x: p[0] as f32, z: p[1] as f32, yaw: yaw as f32, driving },
                        player_id: 0,
                        pos: [p[0], 0.0, p[1]],
                        driving,
                        car_yaw: yaw,
                        speed: 0.0,
                        road,
                        reversed: false,
                        vertex: (vertex + 1).min(pts.len() - 1),
                        dice: Dice::new(cfg.seed ^ (i as u64 + 1) * 7919),
                        frags: HashMap::new(),
                        sec_bytes: 0,
                        sec_start: Instant::now(),
                        bytes_per_s: vec![],
                        stuck: 0.0,
                        reverse_for: 0.0,
                        sprint: false,
                        toggle_at: 0.0,
                    }
                })
                .collect();
            let (stop, stats, graph, cfg) = (stop.clone(), stats.clone(), graph.clone(), cfg.clone());
            threads.push(std::thread::Builder::new().name(format!("eo-bots-{t}")).spawn(move || drive_bots(bots, base, link, cfg, graph, stop, stats))?);
        }
        Ok(Self { stop, threads, stats })
    }

    pub fn stop(mut self) -> BotStats {
        self.stop.store(true, Ordering::Relaxed);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
        self.stats.lock().unwrap().clone()
    }
}

fn drive_bots(mut bots: Vec<Bot>, base: usize, link: Link, cfg: BotConfig, graph: Arc<RoadGraph>, stop: Arc<AtomicBool>, stats: Arc<Mutex<BotStats>>) {
    let mut by_salt: HashMap<u64, usize> = bots.iter().enumerate().map(|(i, b)| (b.salt, i)).collect();
    let mut by_conn: HashMap<u64, usize> = HashMap::new();
    let mut buf = vec![0u8; 2048];
    let step = Duration::from_secs_f64(1.0 / 60.0);
    let send_every = ((60.0 / cfg.send_hz.max(1.0)).round() as u32).max(1);
    let mut next_step = Instant::now();
    let mut steps: u32 = 0;
    let mut last_report = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        // Receive everything waiting.
        while let Some((n, from)) = link.recv(&mut buf) {
            if from != cfg.server {
                continue;
            }
            let data = &buf[..n];
            let Some((kind, mut r)) = proto::peek(data) else { continue };
            match kind {
                Kind::Challenge => {
                    if let Ok(c) = Challenge::read(&mut r) {
                        if let Some(&i) = by_salt.get(&c.client_salt) {
                            let b = &mut bots[i];
                            if b.challenge.is_none() {
                                b.challenge = Some(c);
                                b.last_hello = None;
                            }
                        }
                    }
                }
                Kind::Accept => {
                    if let Ok(a) = Accept::read(&mut r) {
                        if let Some(i) = bots.iter().position(|b| b.phase == Phase::Hello && b.challenge.as_ref().is_some_and(|c| c.client_salt ^ c.server_salt == a.connection)) {
                            let b = &mut bots[i];
                            b.phase = Phase::Connected;
                            b.connection = a.connection;
                            b.player_id = a.player_id;
                            b.seq = a.input_seq;
                            b.ack_input = a.input_seq;
                            by_conn.insert(a.connection, i);
                            by_salt.remove(&b.salt);
                        }
                    }
                }
                Kind::Snapshot => {
                    let Ok(c) = r.u64() else { continue };
                    let Some(&i) = by_conn.get(&c) else { continue };
                    let b = &mut bots[i];
                    b.count(n);
                    if !matches!(b.conn.receive(&mut r), Ok(Some(_))) {
                        continue;
                    }
                    let (Ok(seq), Ok(idx), Ok(count)) = (r.u32(), r.u8(), r.u8()) else { continue };
                    let part = r.rest().to_vec();
                    if seq <= b.newest || count == 0 || idx >= count {
                        continue;
                    }
                    let body = if count == 1 {
                        Some(part)
                    } else {
                        let f = b.frags.entry(seq).or_insert_with(|| (vec![None; count as usize], 0));
                        if f.0.len() == count as usize && f.0[idx as usize].is_none() {
                            f.0[idx as usize] = Some(part);
                            f.1 += 1;
                        }
                        if f.1 == count as usize { b.frags.remove(&seq).map(|f| f.0.into_iter().flatten().flatten().collect()) } else { None }
                    };
                    let Some(body) = body else { continue };
                    match b.store.decode(&body) {
                        Ok(d) => {
                            b.newest = d.header.seq;
                            b.frags.retain(|s, _| *s > b.newest);
                            let own = d.header.own;
                            b.ack_input = b.ack_input.max(own.ack_input);
                            let a = b.ack_input;
                            while b.outbox.front().is_some_and(|e| e.0 <= a) {
                                b.outbox.pop_front();
                            }
                            b.pos = [own.check.pos_mm[0] as f64 / 1e3, own.check.pos_mm[1] as f64 / 1e3, own.check.pos_mm[2] as f64 / 1e3];
                            b.driving = own.check.driving;
                            let car = net_id(CLASS_PLAYER_CAR, b.player_id * 16);
                            if let Ok(k) = d.entities.binary_search_by_key(&car, |e| e.0) {
                                let s = d.entities[k].1;
                                b.car_yaw = s.yaw_rad();
                                b.speed = s.speed_mps();
                            }
                            let mut st = stats.lock().unwrap();
                            st.snapshots += 1;
                            st.positions[base + i] = b.pos;
                            st.driving[base + i] = b.driving;
                            st.speeds[base + i] = b.speed;
                        }
                        Err(_) => stats.lock().unwrap().decode_misses += 1,
                    }
                }
                _ => {}
            }
        }
        let now = Instant::now();
        // Handshakes.
        for b in bots.iter_mut().filter(|b| b.phase == Phase::Hello) {
            if b.last_hello.is_none_or(|t| now - t > Duration::from_millis(250)) {
                b.last_hello = Some(now);
                let h = Hello { client_salt: b.salt, version: proto::PROTOCOL_VERSION, ground: proto::ground_scale_ppm(), token: format!("dev:bot{}", base + by_salt.get(&b.salt).copied().unwrap_or(0)).into_bytes(), spawn: Some(b.spawn), resume: None, server_salt: 0, cookie: 0 };
                let mut out = Vec::with_capacity(proto::REQUEST_PAD);
                match &b.challenge {
                    Some(c) => Hello { server_salt: c.server_salt, cookie: c.cookie, ..h }.write(Kind::Response, &mut out),
                    None => h.write(Kind::Request, &mut out),
                }
                link.send(&out, cfg.server);
            }
        }
        // Fixed 60 Hz inputs.
        if now >= next_step {
            next_step += step;
            if now > next_step + Duration::from_millis(100) {
                next_step = now + step;
            }
            steps = steps.wrapping_add(1);
            let send = steps % send_every == 0;
            for b in bots.iter_mut().filter(|b| b.phase == Phase::Connected) {
                let input = b.steer(&graph, &cfg);
                b.seq += 1;
                b.outbox.push_back((b.seq, input));
                while b.outbox.len() > MAX_INPUTS {
                    b.outbox.pop_front();
                }
                if send {
                    let p = InputPacket { ack_snapshot: b.newest, correction_applied: 0, check: None, first_seq: b.outbox.front().map(|e| e.0).unwrap_or(b.seq + 1), inputs: b.outbox.iter().map(|e| e.1).collect(), ping_ms: 0 };
                    let mut out = Vec::with_capacity(160);
                    proto::begin_connected(Kind::Input, b.connection, &mut out);
                    b.conn.begin_packet(&mut out);
                    p.write(&mut out);
                    link.send(&out, cfg.server);
                }
            }
        } else {
            std::thread::sleep(Duration::from_micros(500));
        }
        if now - last_report > Duration::from_secs(1) {
            last_report = now;
            let mut st = stats.lock().unwrap();
            let mut bytes = std::mem::take(&mut st.bytes_per_s);
            for b in bots.iter_mut() {
                b.roll(now, &mut bytes);
            }
            st.bytes_per_s = bytes;
            // Each thread reports its own bots; `connected` is the sum over threads.
            let mine = bots.iter().filter(|b| b.phase == Phase::Connected).count();
            CONNECTED.with(|c| {
                st.connected = st.connected + mine - c.get();
                c.set(mine);
            });
        }
    }
}

thread_local! {
    static CONNECTED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl Bot {
    fn count(&mut self, n: usize) {
        self.sec_bytes += n as u64 + eo_net::link::UDP_IP_OVERHEAD;
    }

    fn roll(&mut self, now: Instant, out: &mut Vec<u64>) {
        if (now - self.sec_start).as_secs_f64() >= 1.0 {
            if self.phase == Phase::Connected && self.newest > 0 {
                self.bytes_per_s.push(self.sec_bytes);
                out.push(self.sec_bytes);
            }
            self.sec_bytes = 0;
            self.sec_start = now;
        }
    }

    /// The next waypoint along the road; at its end, the next road.
    fn target(&mut self, graph: &RoadGraph, reach: f64) -> [f64; 2] {
        loop {
            let pts = &graph.roads[self.road as usize];
            let idx = if self.reversed { pts.len() - 1 - self.vertex } else { self.vertex };
            let t = pts[idx];
            if (t[0] - self.pos[0]).hypot(t[1] - self.pos[2]) > reach {
                return t;
            }
            if self.vertex + 1 < pts.len() {
                self.vertex += 1;
                continue;
            }
            let end = t;
            match graph.next(end, self.road, &mut self.dice) {
                Some((r, at_end)) => {
                    self.road = r;
                    self.reversed = at_end;
                    self.vertex = 1;
                }
                None => {
                    self.reversed = !self.reversed;
                    self.vertex = 1;
                }
            }
            return end;
        }
    }

    fn steer(&mut self, graph: &RoadGraph, cfg: &BotConfig) -> NetInput {
        let dt = 1.0 / 60.0;
        if self.driving {
            let t = self.target(graph, 14.0);
            let want = (t[0] - self.pos[0]).atan2(t[1] - self.pos[2]);
            let mut err = want - self.car_yaw;
            while err > std::f64::consts::PI {
                err -= std::f64::consts::TAU;
            }
            while err < -std::f64::consts::PI {
                err += std::f64::consts::TAU;
            }
            let target_speed = if err.abs() > 0.6 { cfg.drive_mps.min(6.0) } else { cfg.drive_mps };
            self.stuck = if self.speed.abs() < 0.5 && self.reverse_for <= 0.0 { self.stuck + dt } else { 0.0 };
            if self.stuck > 3.0 {
                self.reverse_for = 1.5;
                self.stuck = 0.0;
            }
            let mut s = StepInput { steer: (err * 1.5).clamp(-1.0, 1.0), ..Default::default() };
            if self.reverse_for > 0.0 {
                self.reverse_for -= dt;
                s.brake = 1.0;
                s.steer = -s.steer;
            } else if self.speed < target_speed {
                s.throttle = 1.0;
            } else if self.speed > target_speed + 2.0 {
                s.brake = 0.6;
            }
            NetInput::quantize(&s, self.car_yaw)
        } else {
            let t = self.target(graph, 3.0);
            let yaw = (t[0] - self.pos[0]).atan2(t[1] - self.pos[2]);
            self.toggle_at -= dt;
            if self.toggle_at <= 0.0 {
                self.sprint = self.dice.next_f64() < 0.4;
                self.toggle_at = 4.0 + self.dice.next_f64() * 8.0;
            }
            NetInput::quantize(&StepInput { move_y: 1.0, sprint: self.sprint, ..Default::default() }, yaw)
        }
    }
}
