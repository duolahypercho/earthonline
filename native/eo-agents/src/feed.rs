//! From the simulation's instance buffers to the renderer's: one thread that
//! grounds every agent on the drawn street surface (agents carry y = 0),
//! picks each car's body and paint, each walker's family and clip, tilts cars
//! to the grade under their axles, derives the front-wheel steer from the
//! heading rate, and hands the render thread a ready `GpuAgent` array.

use crate::catalog::AgentsJs;
use crate::library::Library;
use crate::{AGENT_BLINK_ON, CLIP_IDLE, CLIP_PHONE, CLIP_TALK, GpuAgent, hex_packed};
use eo_sim::output::{FLAG_CROSSING, FLAG_TURN_LEFT, FLAG_TURN_RIGHT, KIND_WALKER_BASE};
use eo_sim::world::Frame;
use glam::Quat;
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

/// What the agents stand on.
#[derive(Clone)]
pub enum Ground {
    /// The street surface the renderer draws (`drawnSurfaceAt`), else the ground carpet.
    Street(Arc<eo_street::StreetWorld>),
    Elevation(Arc<eo_data::ElevationGrid>),
    Flat,
}

impl Ground {
    pub fn y(&self, x: f64, z: f64) -> f64 {
        match self {
            Ground::Street(w) => w.surface_at(x, z, None),
            Ground::Elevation(g) => g.ground(x, z),
            Ground::Flat => 0.0,
        }
    }
}

/// The drawn carriageways, gridded (32 m cells): (a, b, half width, footway widths left/right).
pub struct Carriageways {
    cells: HashMap<(i32, i32), Vec<u32>>,
    pieces: Vec<([f64; 2], [f64; 2], f64, [f64; 2])>,
}

impl Carriageways {
    pub fn build(w: &eo_street::StreetWorld) -> Self {
        let mut cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        let mut pieces = vec![];
        for e in &w.plan.entries {
            for s in e.points.windows(2) {
                let (a, b) = (s[0], s[1]);
                let id = pieces.len() as u32;
                pieces.push((a, b, e.half, e.walks));
                let r = e.half;
                for gz in ((a[1].min(b[1]) - r) / 32.0).floor() as i32..=((a[1].max(b[1]) + r) / 32.0).floor() as i32 {
                    for gx in ((a[0].min(b[0]) - r) / 32.0).floor() as i32..=((a[0].max(b[0]) + r) / 32.0).floor() as i32 {
                        cells.entry((gx, gz)).or_default().push(id);
                    }
                }
            }
        }
        Self { cells, pieces }
    }

    /// A point on a drawn carriageway band moved across it onto the middle of
    /// the drawn footway on its nearer side (None when not on a carriageway or
    /// that side has no footway).
    pub fn footway_beside(&self, x: f64, z: f64) -> Option<[f64; 2]> {
        let list = self.cells.get(&((x / 32.0).floor() as i32, (z / 32.0).floor() as i32))?;
        for &id in list {
            let (a, b, half, walks) = self.pieces[id as usize];
            let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
            let len2 = dx * dx + dz * dz;
            if len2 < 1e-12 {
                continue;
            }
            let t = ((x - a[0]) * dx + (z - a[1]) * dz) / len2;
            if !(0.0..=1.0).contains(&t) {
                continue;
            }
            let (px, pz) = (a[0] + dx * t, a[1] + dz * t);
            let len = len2.sqrt();
            // Left normal, as `walk_level_beside` sides the footways.
            let (nx, nz) = (-dz / len, dx / len);
            let lat = (x - px) * nx + (z - pz) * nz;
            if lat.abs() >= half {
                continue;
            }
            let (side, walk) = if lat > 0.0 { (1.0, walks[0]) } else { (-1.0, walks[1]) };
            if walk < 0.5 {
                return None;
            }
            // Keep the walker's place across the street as a place across the
            // footway (a quarter to three quarters of it), so a file of walkers
            // does not collapse onto one line.
            let to = side * (half + walk * (0.25 + 0.5 * (lat.abs() / half)));
            return Some([px + nx * to, pz + nz * to]);
        }
        None
    }
}

/// Turn signals blink at 1.5 Hz (CA Vehicle Code 24952: 60-120 flashes a minute).
pub const BLINK_HZ: f64 = 1.5;
/// Steer is limited to the catalogue's arcade lock.
pub const MAX_STEER_RAD: f32 = 0.6;

#[derive(Clone, Copy)]
struct Car {
    kind: u16,
    model: u32,
    color: u32,
    wheel_radius: f64,
    half_base: f64,
    heading: f32,
    time: f64,
    steer: f32,
}

#[derive(Clone, Copy)]
struct Walker {
    model: u32,
    scale: f32,
    offset: f32,
    /// its place in the people pool, its walk.
    pool: usize,
    motion: crate::motion::WalkerMotion,
}

pub struct Feeder {
    lib: Arc<Library>,
    js: AgentsJs,
    ground: Ground,
    seed_int: u32,
    cars: HashMap<(usize, u32), Car>,
    walkers: HashMap<(usize, u32), Walker>,
    /// The drawn carriageways (built on the feed thread on first use).
    carriageways: Option<Carriageways>,
    /// Walkers moved off a drawn carriageway onto its footway, last build.
    pub moved_to_footway: usize,
    pub last_ms: f64,
}

fn fract(x: f64) -> f64 {
    x - x.floor()
}

impl Feeder {
    pub fn new(lib: Arc<Library>, ground: Ground, seed_int: u32) -> anyhow::Result<Self> {
        Ok(Self { lib, js: AgentsJs::load()?, ground, seed_int, cars: HashMap::new(), walkers: HashMap::new(), carriageways: None, moved_to_footway: 0, last_ms: 0.0 })
    }

    /// Build the instances of one simulation frame (appended to `out`).
    pub fn build(&mut self, frame: &Frame, out: &mut Vec<GpuAgent>) {
        let t0 = Instant::now();
        let lib = self.lib.clone();
        let blink = fract(frame.time * BLINK_HZ) < 0.5;
        if self.carriageways.is_none() {
            if let Ground::Street(w) = &self.ground {
                self.carriageways = Some(Carriageways::build(w));
            }
        }
        self.moved_to_footway = 0;
        for (ri, r) in frame.regions.iter().enumerate() {
            let [ox, oz] = r.origin;
            for a in &r.vehicles {
                let key = (ri, a.id);
                let car = match self.cars.get(&key) {
                    Some(c) if c.kind == a.kind => *c,
                    _ => {
                        let mut body = self.js.pick_body(self.seed_int, a.id, a.kind).unwrap_or("sedan");
                        if lib.model(body).is_none() {
                            // A body not in this checkout: another of the kind's pool that is.
                            let present: Vec<&str> = self.js.pool(a.kind).iter().map(|s| s.as_str()).filter(|b| lib.model(b).is_some()).collect();
                            if let Some(&b) = present.get(a.id as usize % present.len().max(1)) {
                                body = b;
                            }
                        }
                        let Some(model) = lib.model(body).or_else(|| lib.model("sedan")) else { continue };
                        let paint = self.js.pick_paint(self.seed_int, a.id, body);
                        let m = &lib.models[model as usize];
                        let wb = (m.wheel[0][2] - m.wheel[2][2]).abs() as f64;
                        let c = Car {
                            kind: a.kind,
                            model,
                            color: hex_packed(&paint.hex),
                            wheel_radius: (m.wheel_radius as f64).max(0.2),
                            half_base: if wb > 0.5 { wb / 2.0 } else { 1.4 },
                            heading: a.heading,
                            time: frame.time,
                            steer: 0.0,
                        };
                        self.cars.insert(key, c);
                        c
                    }
                };
                let (x, z) = (ox + a.pos[0] as f64, oz + a.pos[2] as f64);
                let (s, c) = (a.heading as f64).sin_cos();
                let (fx, fz) = (s, c);
                let yf = self.ground.y(x + fx * car.half_base, z + fz * car.half_base);
                let yb = self.ground.y(x - fx * car.half_base, z - fz * car.half_base);
                let y = 0.5 * (yf + yb);
                let pitch = ((yf - yb) / (2.0 * car.half_base)).atan() as f32;
                // Steer from the heading rate: the bicycle model, tan(steer) = wheelbase * yaw rate / speed.
                let dt = frame.time - car.time;
                let mut steer = car.steer;
                if dt > 1e-4 {
                    let mut dh = a.heading - car.heading;
                    while dh > std::f32::consts::PI {
                        dh -= std::f32::consts::TAU;
                    }
                    while dh < -std::f32::consts::PI {
                        dh += std::f32::consts::TAU;
                    }
                    let rate = dh as f64 / dt;
                    let target = if a.speed > 0.3 { ((2.0 * car.half_base * rate) / a.speed as f64).atan() as f32 } else { car.steer };
                    let target = target.clamp(-MAX_STEER_RAD, MAX_STEER_RAD);
                    let k = (dt / 0.15).min(1.0) as f32;
                    steer += (target - steer) * k;
                }
                if let Some(cs) = self.cars.get_mut(&key) {
                    cs.heading = a.heading;
                    cs.time = frame.time;
                    cs.steer = steer;
                }
                let rot = Quat::from_rotation_y(a.heading) * Quat::from_rotation_x(-pitch);
                let spin = (a.anim as f64 / car.wheel_radius).rem_euclid(std::f64::consts::TAU) as f32;
                let mut flags = a.flags as u32;
                if blink && a.flags & (FLAG_TURN_LEFT | FLAG_TURN_RIGHT) != 0 {
                    flags |= AGENT_BLINK_ON;
                }
                out.push(GpuAgent {
                    pos: [x as f32, y as f32, z as f32],
                    model: car.model,
                    rot: rot.to_array(),
                    color: car.color,
                    flags,
                    anim: spin,
                    steer,
                    speed: a.speed,
                    scale: eo_data::groundscale::vehicles() as f32, // the trial's visual vehicle scale, 1.0 unless set
                    clip_phase: 0.0,
                    clip: 0,
                });
            }
            if lib.meta.people_models.is_empty() {
                continue;
            }
            for a in &r.walkers {
                let key = (ri, a.id);
                let w = *self.walkers.entry(key).or_insert_with(|| {
                    let k = format!("ambient-{}", a.id);
                    let fams = &lib.meta.people_models;
                    let f = ((eo_sim::rng::keyed_random(&k, "civilian-family") * fams.len() as f64) as usize).min(fams.len() - 1);
                    // `NPC_HEIGHT_SHAPE_RANGE`: height x (1 +- 0.04 shape).
                    let shape = eo_sim::rng::keyed_random(&k, "civilian-height") * 2.0 - 1.0;
                    Walker { model: fams[f], scale: ((1.0 + 0.04 * shape) * eo_data::groundscale::people()) as f32, offset: eo_sim::rng::keyed_random(&k, "civilian-clip") as f32, pool: f, motion: Default::default() } // * people(), the trial's visual people scale, 1.0 unless set
                });
                let (mut x, mut z) = (ox + a.pos[0] as f64, oz + a.pos[2] as f64);
                // The simulation's footways come from the class street model
                // (eo-sim `native`), narrower than the surveyed street the
                // renderer draws: a walker it puts on a drawn carriageway away
                // from a crossing is drawn on the drawn footway beside it.
                if a.flags & FLAG_CROSSING == 0 {
                    if let Some(p) = self.carriageways.as_ref().and_then(|c| c.footway_beside(x, z)) {
                        x = p[0];
                        z = p[1];
                        self.moved_to_footway += 1;
                    }
                }
                let y = self.ground.y(x, z);
                let activity = a.kind.saturating_sub(KIND_WALKER_BASE);
                let clip = match activity {
                    2 => CLIP_TALK,
                    3 => CLIP_PHONE,
                    _ => CLIP_IDLE,
                };
                let secs = lib.meta.clip_seconds[clip as usize].max(0.1) as f64;
                let mut anim = fract(a.anim as f64 / std::f64::consts::TAU) as f32;
                let mut clip_phase = fract(frame.time / secs + w.offset as f64) as f32;
                let mut clip = clip;
                // the walk styles and the Rocketbox clips.
                let mo = &lib.meta.motion;
                if mo.any() {
                    if let Some(ws) = self.walkers.get_mut(&key) {
                        if mo.styled() {
                            let gait = a.anim as f64 / std::f64::consts::TAU;
                            anim = crate::motion::walk_step(mo, &mut ws.motion, w.pool, a.id, gait, a.speed as f64, w.scale, activity == 3);
                        }
                        if let Some((slot, l)) = crate::motion::standing(mo, &mut ws.motion, activity, a.flags as u32, a.id) {
                            clip = slot as u32;
                            clip_phase = mo.clip_phase(slot, l, frame.time / mo.loop_seconds(slot, l).max(0.1) + w.offset as f64);
                        }
                    }
                }
                out.push(GpuAgent {
                    pos: [x as f32, y as f32, z as f32],
                    model: w.model,
                    rot: Quat::from_rotation_y(a.heading).to_array(),
                    color: 0xffffffff,
                    flags: a.flags as u32,
                    anim,
                    steer: 0.0,
                    speed: a.speed,
                    scale: w.scale,
                    clip_phase,
                    clip,
                });
            }
        }
        // Forget agents not seen for a while (ids are recycled).
        if self.cars.len() > 8 * out.len().max(64) {
            let t = frame.time;
            self.cars.retain(|_, c| t - c.time < 5.0);
            self.walkers.clear();
        }
        self.last_ms = t0.elapsed().as_secs_f64() * 1e3;
    }
}

/// The feeder on its own thread: post the newest simulation frame, take the
/// newest instance array (one frame behind).
pub struct FeedThread {
    shared: Arc<(Mutex<FeedMail>, Condvar)>,
    thread: Option<std::thread::JoinHandle<()>>,
}

#[derive(Default)]
struct FeedMail {
    input: Option<Frame>,
    output: Option<Arc<Vec<GpuAgent>>>,
    build_ms: f64,
    moved: usize,
    shutdown: bool,
}

impl FeedThread {
    pub fn start(mut feeder: Feeder) -> Self {
        let shared: Arc<(Mutex<FeedMail>, Condvar)> = Arc::new((Mutex::new(FeedMail::default()), Condvar::new()));
        let s = shared.clone();
        let thread = std::thread::Builder::new()
            .name("eo-agents-feed".into())
            .spawn(move || {
                let mut buf: Vec<GpuAgent> = vec![];
                loop {
                    let frame = {
                        let mut m = s.0.lock().unwrap();
                        while m.input.is_none() && !m.shutdown {
                            m = s.1.wait(m).unwrap();
                        }
                        if m.shutdown {
                            return;
                        }
                        m.input.take().unwrap()
                    };
                    buf.clear();
                    feeder.build(&frame, &mut buf);
                    let mut m = s.0.lock().unwrap();
                    m.output = Some(Arc::new(buf.clone()));
                    m.build_ms = feeder.last_ms;
                    m.moved = feeder.moved_to_footway;
                }
            })
            .expect("spawn agent feed thread");
        Self { shared, thread: Some(thread) }
    }

    pub fn post(&self, frame: Frame) {
        let mut m = self.shared.0.lock().unwrap();
        m.input = Some(frame);
        self.shared.1.notify_one();
    }

    /// The newest instances, if any arrived since the last take; and the feed's last build time.
    pub fn take(&self) -> (Option<Arc<Vec<GpuAgent>>>, f64) {
        let mut m = self.shared.0.lock().unwrap();
        (m.output.take(), m.build_ms)
    }

    /// Walkers the last build moved off a drawn carriageway onto its footway.
    pub fn moved_to_footway(&self) -> usize {
        self.shared.0.lock().unwrap().moved
    }

    /// Build synchronously on the caller (captures, tests).
    pub fn wait_output(&self, timeout_s: f64) -> Option<Arc<Vec<GpuAgent>>> {
        let t = Instant::now();
        loop {
            if let (Some(o), _) = self.take() {
                return Some(o);
            }
            if t.elapsed().as_secs_f64() > timeout_s {
                return None;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}

impl Drop for FeedThread {
    fn drop(&mut self) {
        {
            let mut m = self.shared.0.lock().unwrap();
            m.shutdown = true;
            self.shared.1.notify_one();
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The player's body (the first pool person) on its feet, facing `yaw`.
pub fn player_body(lib: &Library, feet: [f64; 3], yaw: f64, speed: f64, walk_phase: f64, time: f64) -> Option<GpuAgent> {
    let model = lib.meta.player_model;
    if lib.meta.people_models.is_empty() {
        return None;
    }
    let secs = lib.meta.clip_seconds[CLIP_IDLE as usize].max(0.1) as f64;
    // the player walks the first walk style and stands in the first idle loop.
    let mo = &lib.meta.motion;
    let (anim, clip_phase) = if mo.any() {
        let idle_s = if mo.looped(CLIP_IDLE as usize) { mo.player_seconds[CLIP_IDLE as usize].max(0.1) as f64 } else { secs };
        (mo.walk_anim(0, walk_phase), mo.clip_phase(CLIP_IDLE as usize, 0, time / idle_s))
    } else {
        (fract(walk_phase) as f32, fract(time / secs) as f32)
    };
    Some(GpuAgent {
        pos: [feet[0] as f32, feet[1] as f32, feet[2] as f32],
        model,
        rot: Quat::from_rotation_y(yaw as f32).to_array(),
        color: 0xffffffff,
        flags: crate::AGENT_PLAYER,
        anim,
        steer: 0.0,
        speed: speed as f32,
        scale: eo_data::groundscale::people() as f32, // the trial's visual people scale, 1.0 unless set
        clip_phase,
        clip: CLIP_IDLE,
    })
}

/// The player's car: its road datum (bottom of the wheels) and orientation.
#[allow(clippy::too_many_arguments)]
pub fn player_car(lib: &Library, body: &str, datum: [f64; 3], rot: [f64; 4], spin: f64, steer: f64, speed: f64, braking: bool, paint_hex: &str) -> Option<GpuAgent> {
    let model = lib.model(body)?;
    let q = Quat::from_xyzw(rot[0] as f32, rot[1] as f32, rot[2] as f32, rot[3] as f32);
    Some(GpuAgent {
        pos: [datum[0] as f32, datum[1] as f32, datum[2] as f32],
        model,
        rot: q.to_array(),
        color: hex_packed(paint_hex),
        flags: crate::AGENT_PLAYER | if braking { eo_sim::output::FLAG_BRAKING as u32 } else { 0 },
        anim: spin.rem_euclid(std::f64::consts::TAU) as f32,
        steer: steer as f32,
        speed: speed as f32,
        scale: eo_data::groundscale::vehicles() as f32, // the trial's visual vehicle scale, 1.0 unless set
        clip_phase: 0.0,
        clip: 0,
    })
}
