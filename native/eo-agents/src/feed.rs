//! From the simulation's instance buffers to the renderer's: one thread that
//! grounds every agent on the drawn street surface (agents carry y = 0),
//! picks each car's body and paint, each walker's family and clip, tilts cars
//! to the grade under their axles, derives the front-wheel steer from the
//! heading rate, and hands the render thread a ready `GpuAgent` array.
//!
//! With the grade-separated roads (`--structures`) a car whose lane is on a
//! structure level (`eo_sim::output::level_of_flags`, 0 without them) stands
//! on that level's deck profile (`StructureWorld::deck_at` at each axle),
//! not on the street under it (`car_ground`). With the tunnels (`--tunnels`)
//! a car on a lane under grade inside a bore carries `AGENT_IN_BORE` and its
//! depth in from the portal (`in_bore`, `bore_flags`): the renderer lights it
//! by the bore, not by the sun and sky.

use crate::catalog::AgentsJs;
use crate::library::Library;
use crate::{AGENT_BLINK_ON, CLIP_IDLE, CLIP_PHONE, CLIP_TALK, GpuAgent, hex_packed};
use eo_sim::output::{FLAG_CROSSING, FLAG_LEVEL_MASK, FLAG_PARTY, FLAG_PARTY_SLOT_MASK, FLAG_PARTY_SLOT_SHIFT, FLAG_TURN_LEFT, FLAG_TURN_RIGHT, KIND_WALKER_BASE, level_of_flags};
use eo_sim::world::Frame;
use glam::Quat;
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

/// (`--crowd`) The paved plazas' outlines, gridded (32 m cells): a walker
/// on one, off the street's own paving, stands on the plaza's paving, which
/// the landmarks lay at the footway datum (the ground plus the street's
/// footway lift, eo-app main.rs `add_plazas_with`), not on the open ground's
/// carpet under it.
pub struct Plazas {
    rings: Vec<Vec<[f64; 2]>>,
    cells: HashMap<(i32, i32), Vec<u32>>,
}

impl Plazas {
    pub fn new(rings: Vec<Vec<[f64; 2]>>) -> Self {
        let mut cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, r) in rings.iter().enumerate() {
            let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
            for p in r {
                for a in 0..2 {
                    lo[a] = lo[a].min(p[a]);
                    hi[a] = hi[a].max(p[a]);
                }
            }
            if r.len() < 3 {
                continue;
            }
            for gx in (lo[0] / 32.0).floor() as i32..=(hi[0] / 32.0).floor() as i32 {
                for gz in (lo[1] / 32.0).floor() as i32..=(hi[1] / 32.0).floor() as i32 {
                    cells.entry((gx, gz)).or_default().push(i as u32);
                }
            }
        }
        Self { rings, cells }
    }

    /// Whether (x, z) lies inside a plaza's outline (even-odd).
    pub fn contains(&self, x: f64, z: f64) -> bool {
        let Some(list) = self.cells.get(&((x / 32.0).floor() as i32, (z / 32.0).floor() as i32)) else { return false };
        list.iter().any(|&i| {
            let r = &self.rings[i as usize];
            let mut inside = false;
            let mut j = r.len() - 1;
            for k in 0..r.len() {
                let (a, b) = (r[k], r[j]);
                if (a[1] > z) != (b[1] > z) && x < (b[0] - a[0]) * (z - a[1]) / (b[1] - a[1]) + a[0] {
                    inside = !inside;
                }
                j = k;
            }
            inside
        })
    }
}

/// (`--crowd`) The walkable paving the landmarks lay above the ground (the
/// quay's apron along the seawall at the wall's crest, the pier decks, the
/// plazas' paving: eo-landmarks `Structure::footway`), as triangles facing up,
/// gridded (16 m cells). A walker over one stands on its top, not on the
/// carpet the paving was laid over (the Embarcadero's apron stands up to a
/// metre over the pressed ground behind the wall).
pub struct Decks {
    tris: Vec<[[f32; 3]; 3]>,
    cells: HashMap<(i32, i32), Vec<u32>>,
}

/// The least upward share of a deck triangle's normal (steeper faces are not stood on).
pub const DECK_UP_MIN: f32 = 0.7;
const DECK_CELL_M: f64 = 16.0;

impl Decks {
    pub fn new(tris: Vec<[[f32; 3]; 3]>) -> Self {
        let mut kept = vec![];
        let mut cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for t in tris {
            let (a, b, c) = (glam::Vec3::from(t[0]), glam::Vec3::from(t[1]), glam::Vec3::from(t[2]));
            let n = (b - a).cross(c - a);
            let len = n.length();
            if !(len > 1e-6) || n.y.abs() / len < DECK_UP_MIN {
                continue;
            }
            let id = kept.len() as u32;
            let (lo_x, hi_x) = (a.x.min(b.x).min(c.x) as f64, a.x.max(b.x).max(c.x) as f64);
            let (lo_z, hi_z) = (a.z.min(b.z).min(c.z) as f64, a.z.max(b.z).max(c.z) as f64);
            for gx in (lo_x / DECK_CELL_M).floor() as i32..=(hi_x / DECK_CELL_M).floor() as i32 {
                for gz in (lo_z / DECK_CELL_M).floor() as i32..=(hi_z / DECK_CELL_M).floor() as i32 {
                    cells.entry((gx, gz)).or_default().push(id);
                }
            }
            kept.push(t);
        }
        Self { tris: kept, cells }
    }

    pub fn len(&self) -> usize {
        self.tris.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tris.is_empty()
    }

    /// The highest deck top over (x, z), if a deck triangle covers it.
    pub fn top(&self, x: f64, z: f64) -> Option<f64> {
        let list = self.cells.get(&((x / DECK_CELL_M).floor() as i32, (z / DECK_CELL_M).floor() as i32))?;
        let mut best: Option<f64> = None;
        for &i in list {
            let [a, b, c] = self.tris[i as usize];
            let (ax, az, bx, bz, cx, cz) = (a[0] as f64, a[2] as f64, b[0] as f64, b[2] as f64, c[0] as f64, c[2] as f64);
            let d = (bz - cz) * (ax - cx) + (cx - bx) * (az - cz);
            if d.abs() < 1e-9 {
                continue;
            }
            let u = ((bz - cz) * (x - cx) + (cx - bx) * (z - cz)) / d;
            let v = ((cz - az) * (x - cx) + (ax - cx) * (z - cz)) / d;
            let w = 1.0 - u - v;
            if u < -1e-6 || v < -1e-6 || w < -1e-6 {
                continue;
            }
            let y = u * a[1] as f64 + v * b[1] as f64 + w * c[1] as f64;
            best = Some(best.map_or(y, |b: f64| b.max(y)));
        }
        best
    }
}

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

/// Where a car stands: its height (the mean of its two axles') and pitch
/// (from their difference), the car centred at (x, z) heading along the unit
/// plan direction `f`, its axles `half_base` either side. On a structure
/// level (`level != 0`, with `structures`) each axle stands on the deck of
/// that level under the car's centre (`StructureWorld::deck_at`), as drawn
/// (`deck_top`: the profile, and the street's crown blended in toward a
/// paved end), an axle past that deck on the level's deck under it, else
/// on the ground as before; at grade both on `ground`, so a car under a deck
/// is never lifted onto it.
#[allow(clippy::too_many_arguments)]
pub fn car_ground(ground: &Ground, structures: Option<&eo_structures::StructureWorld>, level: i8, x: f64, z: f64, f: [f64; 2], half_base: f64) -> (f64, f32) {
    let (xf, zf, xb, zb) = (x + f[0] * half_base, z + f[1] * half_base, x - f[0] * half_base, z - f[1] * half_base);
    // the deck the car is on (at its centre): both axles on it while it is
    // under them (a neighbouring deck of the level nearer an axle is not the
    // car's road), on the deck as drawn (`StructureWorld::deck_top`).
    let deck = structures.filter(|_| level != 0).and_then(|s| s.deck_at(x, z, level).map(|d| (s, d.chain)));
    let at = |px: f64, pz: f64| -> f64 {
        let Some((s, own)) = deck else { return ground.y(px, pz) };
        if let Some(y) = s.deck_top(own, px, pz) {
            return y;
        }
        match s.deck_at(px, pz, level) {
            Some(d) => s.deck_top(d.chain, px, pz).unwrap_or(d.y),
            None => ground.y(px, pz),
        }
    };
    let (yf, yb) = (at(xf, zf), at(xb, zb));
    (0.5 * (yf + yb), ((yf - yb) / (2.0 * half_base)).atan() as f32)
}

/// How far into a tunnel's bore a car stands (metres in from the nearer
/// portal; infinite with no portal), when it does: only a car on a lane under
/// grade (`level < 0`, the levels the bores' chains carry) is looked up, at a
/// point a metre over its datum `y` (inside the bore's section, not on its
/// floor's edge: `StructureWorld::bore_at`). None without structures or bores,
/// and for every car at grade or on a deck.
pub fn in_bore(structures: Option<&eo_structures::StructureWorld>, level: i8, x: f64, y: f64, z: f64) -> Option<f64> {
    if level >= 0 {
        return None;
    }
    let s = structures?;
    if s.bores().is_empty() {
        return None;
    }
    s.bore_at(x, y + 1.0, z).map(|h| h.depth)
}

/// The player's body or car (whose lane level nothing knows): marked
/// `AGENT_IN_BORE` with its depth when a point a metre over its datum lies
/// inside a tunnel's bore. Nothing without bores.
pub fn mark_in_bore(a: &mut GpuAgent, structures: Option<&eo_structures::StructureWorld>) {
    let Some(s) = structures.filter(|s| !s.bores().is_empty()) else { return };
    if let Some(h) = s.bore_at(a.pos[0] as f64, a.pos[1] as f64 + 1.0, a.pos[2] as f64) {
        a.flags |= crate::bore_flags(h.depth);
    }
}

/// (`--crowd`, `Feeder::separate`) the people's bodies kept apart: a body is
/// a disc of this radius on the ground (Fruin's body ellipse, 0.61 x 0.46 m
/// with clothing, by its mean half axis), relaxed this many times a frame.
pub const BODY_RADIUS_M: f64 = 0.27;
const SEPARATE_ITERATIONS: usize = 4;
/// (`--crowd`) no two people in the same body within this of each other
/// when a walker first appears (its first `TWIN_DRAWS` keyed draws tried).
pub const TWIN_RADIUS_M: f64 = 30.0;
const TWIN_DRAWS: u32 = 8;
/// A vehicle's footprint for the people standing clear of it, from its model
/// (its wheels and bounding sphere: the body's half length the larger of the
/// sphere's radius by this share, the sphere standing for the body's length
/// and height together, and the wheelbase's half plus a wheel; its half width
/// the wheels' track plus this share of a wheel's radius, the tyre and the
/// body over it).
const BODY_LENGTH_OF_RADIUS: f64 = 0.9;
const BODY_WIDTH_PAST_WHEEL: f64 = 0.6;

/// (`--people-space`, with `--crowd`) the room each person keeps from the
/// others: Fruin's no-touch zone (Pedestrian Planning and Design, 1971: a
/// waiting pedestrian keeps a circle of 0.3-0.45 m radius free of others
/// where the space allows, the touch zone being under 0.3 m), its outer
/// radius, so strangers stand and pass 0.9 m apart centre to centre, about
/// half a metre between shoulders, and a dense spot settles between that and
/// the bodies' own breadth.
pub const PERSONAL_RADIUS_M: f64 = 0.45;
/// `keep_personal_space`'s relaxation passes a frame (Gauss-Seidel: each
/// pair's correction applied at once, so a clump spreads in a few passes).
const PERSONAL_ITERATIONS: usize = 8;

static PERSONAL_SPACE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `--people-space`: the feed keeps each person's personal room
/// (`keep_personal_space`) before it keeps the bodies apart (`separate`,
/// with `--crowd`). Off, the bodies are kept apart as before.
pub fn set_personal_space(on: bool) {
    PERSONAL_SPACE.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn personal_space() -> bool {
    PERSONAL_SPACE.load(std::sync::atomic::Ordering::Relaxed)
}

/// (`--people-space`) Keep the people's personal room: each walker a disc of
/// `PERSONAL_RADIUS_M`, every overlapping pair pushed apart along the line
/// between them by half the overlap each (position-based dynamics,
/// `PERSONAL_ITERATIONS` Gauss-Seidel passes over a grid of the discs'
/// diameter). Positions in `pos` in place; deterministic in the input order.
/// `separate` follows it (the bodies and the vehicles).
pub fn keep_personal_space(pos: &mut [[f64; 2]]) {
    keep_personal_space_with(pos, &[]);
}

/// `keep_personal_space`, with each walker's social group (`party`, one a
/// walker; 0 for none, else the same nonzero value for a group's members,
/// `--people-groups`): members of one group keep their own formation's
/// spacing (eo-sim `GROUP_ABREAST_M`, `GROUP_TALK_GAP_M`), inside a stranger's
/// no-touch zone, so only strangers are kept to it. An empty `party` is
/// `keep_personal_space`.
pub fn keep_personal_space_with(pos: &mut [[f64; 2]], party: &[u64]) {
    let n = pos.len();
    let same = |i: usize, j: usize| party.len() == n && party[i] != 0 && party[i] == party[j];
    let r2 = 2.0 * PERSONAL_RADIUS_M;
    let cell = |p: [f64; 2]| ((p[0] / r2).floor() as i32, (p[1] / r2).floor() as i32);
    for _ in 0..PERSONAL_ITERATIONS {
        let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, p) in pos.iter().enumerate() {
            grid.entry(cell(*p)).or_default().push(i as u32);
        }
        let mut any = false;
        for i in 0..n {
            let (cx, cz) = cell(pos[i]);
            for gx in cx - 1..=cx + 1 {
                for gz in cz - 1..=cz + 1 {
                    let Some(list) = grid.get(&(gx, gz)) else { continue };
                    for &j in list {
                        let j = j as usize;
                        if j <= i || same(i, j) {
                            continue;
                        }
                        let (dx, dz) = (pos[j][0] - pos[i][0], pos[j][1] - pos[i][1]);
                        let d = dx.hypot(dz);
                        if d >= r2 {
                            continue;
                        }
                        let (ux, uz) = if d > 1e-6 { (dx / d, dz / d) } else { let a = (i * 7 + j * 13) as f64; (a.cos(), a.sin()) };
                        let h = 0.5 * (r2 - d);
                        pos[i][0] -= ux * h;
                        pos[i][1] -= uz * h;
                        pos[j][0] += ux * h;
                        pos[j][1] += uz * h;
                        any = true;
                    }
                }
            }
        }
        if !any {
            break;
        }
    }
}

/// (`--crowd`) Keep the people's bodies apart and out of the vehicles: each
/// walker a disc of `BODY_RADIUS_M`, every overlapping pair pushed apart
/// along the line between them by half the overlap each (position-based
/// dynamics, `SEPARATE_ITERATIONS` Jacobi passes over a 1 m grid), a walker
/// inside a vehicle's footprint (`cars`: x, z, heading sin and cos, half
/// length and width) pushed out across its nearest side. The simulation does
/// not see the bodies (its walkers are lines' offsets, as the JS ambient pool,
/// whose separation lived in a physics bridge not ported), so this is where
/// they get their room. Positions in `pos` in place; deterministic in the
/// input order.
pub fn separate(pos: &mut [[f64; 2]], cars: &[(f64, f64, f64, f64, f64, f64)]) {
    let n = pos.len();
    if n == 0 {
        return;
    }
    let r2 = 2.0 * BODY_RADIUS_M;
    let cell = |p: [f64; 2]| ((p[0] / r2).floor() as i32, (p[1] / r2).floor() as i32);
    let mut push = vec![[0.0f64; 2]; n];
    for _ in 0..SEPARATE_ITERATIONS {
        let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, p) in pos.iter().enumerate() {
            grid.entry(cell(*p)).or_default().push(i as u32);
        }
        push.iter_mut().for_each(|d| *d = [0.0; 2]);
        let mut any = false;
        for i in 0..n {
            let (cx, cz) = cell(pos[i]);
            for gx in cx - 1..=cx + 1 {
                for gz in cz - 1..=cz + 1 {
                    let Some(list) = grid.get(&(gx, gz)) else { continue };
                    for &j in list {
                        let j = j as usize;
                        if j <= i {
                            continue;
                        }
                        let (dx, dz) = (pos[j][0] - pos[i][0], pos[j][1] - pos[i][1]);
                        let d = dx.hypot(dz);
                        if d >= r2 {
                            continue;
                        }
                        // coincident bodies part along a direction fixed by their order
                        let (ux, uz) = if d > 1e-6 { (dx / d, dz / d) } else { let a = (i * 7 + j * 13) as f64; (a.cos(), a.sin()) };
                        let h = 0.5 * (r2 - d);
                        push[i][0] -= ux * h;
                        push[i][1] -= uz * h;
                        push[j][0] += ux * h;
                        push[j][1] += uz * h;
                        any = true;
                    }
                }
            }
        }
        for i in 0..n {
            pos[i][0] += push[i][0];
            pos[i][1] += push[i][1];
        }
        if !any {
            break;
        }
    }
    if cars.is_empty() {
        return;
    }
    // Out of the vehicles: on a grid of 8 m cells by each footprint's reach.
    const CAR_CELL: f64 = 8.0;
    let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
    for (k, c) in cars.iter().enumerate() {
        let reach = c.4.hypot(c.5) + BODY_RADIUS_M;
        for gx in ((c.0 - reach) / CAR_CELL).floor() as i32..=((c.0 + reach) / CAR_CELL).floor() as i32 {
            for gz in ((c.1 - reach) / CAR_CELL).floor() as i32..=((c.1 + reach) / CAR_CELL).floor() as i32 {
                grid.entry((gx, gz)).or_default().push(k as u32);
            }
        }
    }
    for p in pos.iter_mut() {
        let Some(list) = grid.get(&((p[0] / CAR_CELL).floor() as i32, (p[1] / CAR_CELL).floor() as i32)) else { continue };
        for &k in list {
            let (x, z, s, c, hl, hw) = cars[k as usize];
            // body frame: forward (sin, cos), right (cos, -sin)
            let (dx, dz) = (p[0] - x, p[1] - z);
            let lf = dx * s + dz * c;
            let lr = dx * c - dz * s;
            let (ef, er) = (hl + BODY_RADIUS_M - lf.abs(), hw + BODY_RADIUS_M - lr.abs());
            if ef <= 0.0 || er <= 0.0 {
                continue;
            }
            if er <= ef {
                let m = er * lr.signum();
                p[0] += c * m;
                p[1] -= s * m;
            } else {
                let m = ef * lf.signum();
                p[0] += s * m;
                p[1] += c * m;
            }
        }
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
    /// (`--crowd`) where it was last drawn and when (the no-twin rule, the cleanup).
    at: [f64; 2],
    seen: f64,
}

pub struct Feeder {
    lib: Arc<Library>,
    js: AgentsJs,
    ground: Ground,
    /// The grade-separated roads beside the ground, when the world has them
    /// (`--structures`): cars on a structure level stand on their decks.
    structures: Option<Arc<eo_structures::StructureWorld>>,
    seed_int: u32,
    /// Cars the last build stood on a structure level's deck.
    pub on_decks: usize,
    /// Cars the last build found inside a tunnel's bore (`AGENT_IN_BORE`).
    pub in_bores: usize,
    cars: HashMap<(usize, u32), Car>,
    walkers: HashMap<(usize, u32), Walker>,
    /// The drawn carriageways (built on the feed thread on first use).
    carriageways: Option<Carriageways>,
    /// Walkers moved off a drawn carriageway onto its footway, last build.
    pub moved_to_footway: usize,
    pub last_ms: f64,
    /// (`--crowd`; false as before) keep the people's bodies apart and out of
    /// the vehicles (`separate`).
    pub separate: bool,
    /// (`--crowd`; None as before) the paved plazas a walker stands on (`Plazas`).
    pub plazas: Option<Arc<Plazas>>,
    /// (`--crowd`; None as before) the landmarks' walkable paving (`Decks`).
    pub decks: Option<Arc<Decks>>,
}

fn fract(x: f64) -> f64 {
    x - x.floor()
}

impl Feeder {
    /// `structures`: the grade-separated roads beside `ground` (None without `--structures`).
    pub fn new(lib: Arc<Library>, ground: Ground, structures: Option<Arc<eo_structures::StructureWorld>>, seed_int: u32) -> anyhow::Result<Self> {
        Ok(Self { lib, js: AgentsJs::load()?, ground, structures, seed_int, on_decks: 0, in_bores: 0, cars: HashMap::new(), walkers: HashMap::new(), carriageways: None, moved_to_footway: 0, last_ms: 0.0, separate: false, plazas: None, decks: None })
    }

    /// Where a walker stands at (x, z): the street's drawn surface, or on a
    /// paved plaza (`plazas`, off the street's paving) the plaza's paving.
    fn walker_y(&self, x: f64, z: f64) -> f64 {
        let mut y = self.ground.y(x, z);
        if let (Some(p), Ground::Street(sw)) = (&self.plazas, &self.ground) {
            if p.contains(x, z) && sw.paved_at(x, z).is_none() {
                y = y.max(sw.ground(x, z) + sw.o.footway_lift);
            }
        }
        // (`--crowd`) on the landmarks' paving where one is laid over the ground.
        if let Some(top) = self.decks.as_ref().and_then(|d| d.top(x, z)) {
            y = y.max(top);
        }
        y
    }

    /// The grade-separated roads it was given.
    pub fn structures(&self) -> Option<&Arc<eo_structures::StructureWorld>> {
        self.structures.as_ref()
    }

    /// The car drawn for `key`: its body, paint and wheel, kept while its kind holds.
    fn car_entry(&mut self, lib: &Library, key: (usize, u32), id: u32, kind: u16, heading: f32, time: f64) -> Option<Car> {
        if let Some(c) = self.cars.get(&key) {
            if c.kind == kind {
                return Some(*c);
            }
        }
        let mut body = self.js.pick_body(self.seed_int, id, kind).unwrap_or("sedan");
        if lib.model(body).is_none() {
            // A body not in this checkout: another of the kind's pool that is.
            let present: Vec<&str> = self.js.pool(kind).iter().map(|s| s.as_str()).filter(|b| lib.model(b).is_some()).collect();
            if let Some(&b) = present.get(id as usize % present.len().max(1)) {
                body = b;
            }
        }
        let model = lib.model(body).or_else(|| lib.model("sedan"))?;
        let paint = self.js.pick_paint(self.seed_int, id, body);
        let m = &lib.models[model as usize];
        let wb = (m.wheel[0][2] - m.wheel[2][2]).abs() as f64;
        let c = Car {
            kind,
            model,
            color: hex_packed(&paint.hex),
            wheel_radius: (m.wheel_radius as f64).max(0.2),
            half_base: if wb > 0.5 { wb / 2.0 } else { 1.4 },
            heading,
            time,
            steer: 0.0,
        };
        self.cars.insert(key, c);
        Some(c)
    }

    /// The walker drawn for `key`: its family, height and clip offset.
    fn walker_entry(&mut self, lib: &Library, key: (usize, u32), id: u32) -> Walker {
        *self.walkers.entry(key).or_insert_with(|| {
            let k = format!("ambient-{}", id);
            let fams = &lib.meta.people_models;
            let f = ((eo_sim::rng::keyed_random(&k, "civilian-family") * fams.len() as f64) as usize).min(fams.len() - 1);
            // `NPC_HEIGHT_SHAPE_RANGE`: height x (1 +- 0.04 shape).
            let shape = eo_sim::rng::keyed_random(&k, "civilian-height") * 2.0 - 1.0;
            Walker { model: fams[f], scale: ((1.0 + 0.04 * shape) * eo_data::groundscale::people()) as f32, offset: eo_sim::rng::keyed_random(&k, "civilian-clip") as f32, pool: f, motion: Default::default(), at: [0.0; 2], seen: 0.0 } // * people(), the trial's visual people scale, 1.0 unless set
        })
    }

    /// The corridor layer's vehicles and walkers (eo-sim `corridor`): placed
    /// where the corridor puts them (its centreline carries the height and
    /// grade; nothing is grounded here), marked `AGENT_CORRIDOR`.
    fn build_corridor(&mut self, frame: &Frame, lib: &Library, blink: bool, out: &mut Vec<GpuAgent>) {
        const REGION: usize = usize::MAX;
        for a in &frame.corridor {
            let key = (REGION, a.id);
            let (x, y, z) = (a.pos[0], a.pos[1], a.pos[2]);
            if a.kind < KIND_WALKER_BASE {
                let Some(car) = self.car_entry(lib, key, a.id, a.kind, a.heading, frame.time) else { continue };
                if let Some(cs) = self.cars.get_mut(&key) {
                    cs.heading = a.heading;
                    cs.time = frame.time;
                }
                let rot = Quat::from_rotation_y(a.heading) * Quat::from_rotation_x(-a.pitch);
                let spin = (a.anim as f64 / car.wheel_radius).rem_euclid(std::f64::consts::TAU) as f32;
                let mut flags = a.flags as u32 | crate::AGENT_CORRIDOR;
                if blink && a.flags & (FLAG_TURN_LEFT | FLAG_TURN_RIGHT) != 0 {
                    flags |= AGENT_BLINK_ON;
                }
                out.push(GpuAgent {
                    pos: [x, y, z],
                    model: car.model,
                    rot: rot.to_array(),
                    color: car.color,
                    flags,
                    anim: spin,
                    steer: 0.0,
                    speed: a.speed,
                    scale: eo_data::groundscale::vehicles() as f32,
                    clip_phase: 0.0,
                    clip: 0,
                });
            } else if !lib.meta.people_models.is_empty() {
                let w = self.walker_entry(lib, key, a.id);
                let secs = lib.meta.clip_seconds[CLIP_IDLE as usize].max(0.1) as f64;
                let mut anim = fract(a.anim as f64 / std::f64::consts::TAU) as f32;
                // the corridor layer's own clock (it replays; the regions' may drop time).
                let clip_phase = fract(frame.corridor_time / secs + w.offset as f64) as f32;
                let mo = &lib.meta.motion;
                if mo.any() && mo.styled() {
                    if let Some(ws) = self.walkers.get_mut(&key) {
                        let gait = a.anim as f64 / std::f64::consts::TAU;
                        anim = crate::motion::walk_step(mo, &mut ws.motion, w.pool, a.id, gait, a.speed as f64, w.scale, false);
                    }
                }
                out.push(GpuAgent {
                    pos: [x, y, z],
                    model: w.model,
                    rot: Quat::from_rotation_y(a.heading).to_array(),
                    color: 0xffffffff,
                    flags: a.flags as u32 | crate::AGENT_CORRIDOR,
                    anim,
                    steer: 0.0,
                    speed: a.speed,
                    scale: w.scale,
                    clip_phase,
                    clip: CLIP_IDLE,
                });
            }
        }
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
        self.on_decks = 0;
        self.in_bores = 0;
        // (`--crowd`) the vehicles' footprints and the walkers' places in `out`, kept apart below.
        let mut footprints: Vec<(f64, f64, f64, f64, f64, f64)> = vec![];
        let mut walker_at: Vec<usize> = vec![];
        // (`--people-groups`) each kept walker's social group (0: none).
        let mut walker_party: Vec<u64> = vec![];
        for (ri, r) in frame.regions.iter().enumerate() {
            let [ox, oz] = r.origin;
            for a in &r.vehicles {
                let key = (ri, a.id);
                let Some(car) = self.car_entry(&lib, key, a.id, a.kind, a.heading, frame.time) else { continue };
                let (x, z) = (ox + a.pos[0] as f64, oz + a.pos[2] as f64);
                let (s, c) = (a.heading as f64).sin_cos();
                let level = level_of_flags(a.flags);
                let (y, pitch) = car_ground(&self.ground, self.structures.as_deref(), level, x, z, [s, c], car.half_base);
                if level != 0 && self.structures.is_some() {
                    self.on_decks += 1;
                }
                // a car on a lane under grade inside a tunnel's bore (`--tunnels`):
                // the renderer lights it as the bore is lit (`AGENT_IN_BORE`).
                let bore = in_bore(self.structures.as_deref(), level, x, y, z);
                if bore.is_some() {
                    self.in_bores += 1;
                }
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
                if self.separate {
                    let m = &lib.models[car.model as usize];
                    let track = m.wheel.iter().map(|w| w[0].abs()).fold(0.0f32, f32::max) as f64;
                    let hl = (m.radius as f64 * BODY_LENGTH_OF_RADIUS).max(car.half_base + car.wheel_radius);
                    footprints.push((x, z, s, c, hl, track + BODY_WIDTH_PAST_WHEEL * car.wheel_radius));
                }
                let rot = Quat::from_rotation_y(a.heading) * Quat::from_rotation_x(-pitch);
                let spin = (a.anim as f64 / car.wheel_radius).rem_euclid(std::f64::consts::TAU) as f32;
                // (the level rides in the instance flags; the renderer gets the lamp flags only.)
                let mut flags = (a.flags & !FLAG_LEVEL_MASK) as u32;
                if blink && a.flags & (FLAG_TURN_LEFT | FLAG_TURN_RIGHT) != 0 {
                    flags |= AGENT_BLINK_ON;
                }
                if let Some(depth) = bore {
                    flags |= crate::bore_flags(depth);
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
                let (sx, sz) = (ox + a.pos[0] as f64, oz + a.pos[2] as f64);
                if !self.walkers.contains_key(&key) {
                    let k = format!("ambient-{}", a.id);
                    let fams = &lib.meta.people_models;
                    let pick = |salt: &str| ((eo_sim::rng::keyed_random(&k, salt) * fams.len() as f64) as usize).min(fams.len() - 1);
                    let mut f = pick("civilian-family");
                    if self.separate {
                        // (`--crowd`) no twins in sight: the first of its keyed draws
                        // no one within `TWIN_RADIUS_M` already wears (else the first).
                        let near = |m: u32| self.walkers.iter().any(|(kk, o)| kk.0 == ri && o.model == m && (o.at[0] - sx).hypot(o.at[1] - sz) < TWIN_RADIUS_M);
                        for n in 0..TWIN_DRAWS {
                            let c = if n == 0 { f } else { pick(&format!("civilian-family-{n}")) };
                            if !near(fams[c]) {
                                f = c;
                                break;
                            }
                        }
                    }
                    // `NPC_HEIGHT_SHAPE_RANGE`: height x (1 +- 0.04 shape).
                    let shape = eo_sim::rng::keyed_random(&k, "civilian-height") * 2.0 - 1.0;
                    let w = Walker { model: fams[f], scale: ((1.0 + 0.04 * shape) * eo_data::groundscale::people()) as f32, offset: eo_sim::rng::keyed_random(&k, "civilian-clip") as f32, pool: f, motion: Default::default(), at: [sx, sz], seen: frame.time }; // * people(), the trial's visual people scale, 1.0 unless set
                    self.walkers.insert(key, w);
                }
                let w = {
                    let e = self.walkers.get_mut(&key).expect("inserted above");
                    e.at = [sx, sz];
                    e.seen = frame.time;
                    *e
                };
                let (mut x, mut z) = (sx, sz);
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
                let y = self.walker_y(x, z);
                if self.separate {
                    walker_at.push(out.len());
                    walker_party.push(if a.flags & FLAG_PARTY != 0 {
                        let slot = ((a.flags & FLAG_PARTY_SLOT_MASK) >> FLAG_PARTY_SLOT_SHIFT) as u32;
                        ((ri as u64) << 32 | a.id.wrapping_sub(slot) as u64) + 1
                    } else {
                        0
                    });
                }
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
                    // (the social group's bits are the feed's, not the renderer's.)
                    flags: (a.flags & !(FLAG_PARTY | FLAG_PARTY_SLOT_MASK)) as u32,
                    anim,
                    steer: 0.0,
                    speed: a.speed,
                    scale: w.scale,
                    clip_phase,
                    clip,
                });
            }
        }
        if self.separate && !walker_at.is_empty() {
            let mut pos: Vec<[f64; 2]> = walker_at.iter().map(|&i| [out[i].pos[0] as f64, out[i].pos[2] as f64]).collect();
            if personal_space() {
                keep_personal_space_with(&mut pos, &walker_party); // (`--people-space`) each one's room first
            }
            separate(&mut pos, &footprints);
            for (k, &i) in walker_at.iter().enumerate() {
                let (x, z) = (pos[k][0], pos[k][1]);
                if (x as f32, z as f32) != (out[i].pos[0], out[i].pos[2]) {
                    out[i].pos = [x as f32, self.walker_y(x, z) as f32, z as f32];
                }
            }
        }
        if !frame.corridor.is_empty() {
            self.build_corridor(frame, &lib, blink, out);
        }
        // Forget agents not seen for a while (ids are recycled).
        if self.cars.len() > 8 * out.len().max(64) {
            let t = frame.time;
            self.cars.retain(|_, c| t - c.time < 5.0);
            if self.separate {
                // (`--crowd`) keep the walkers still drawn: a model drawn again could differ (the no-twin rule).
                self.walkers.retain(|_, w| t - w.seen < 5.0);
            } else {
                self.walkers.clear();
            }
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
    /// frames posted, and the post number of the frame the newest output was built from.
    posted: u64,
    built: u64,
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
                    let (frame, seq) = {
                        let mut m = s.0.lock().unwrap();
                        while m.input.is_none() && !m.shutdown {
                            m = s.1.wait(m).unwrap();
                        }
                        if m.shutdown {
                            return;
                        }
                        (m.input.take().unwrap(), m.posted)
                    };
                    buf.clear();
                    feeder.build(&frame, &mut buf);
                    let mut m = s.0.lock().unwrap();
                    m.output = Some(Arc::new(buf.clone()));
                    m.build_ms = feeder.last_ms;
                    m.moved = feeder.moved_to_footway;
                    m.built = seq;
                }
            })
            .expect("spawn agent feed thread");
        Self { shared, thread: Some(thread) }
    }

    /// Hand over a frame; returns its post number (`take_seq`).
    pub fn post(&self, frame: Frame) -> u64 {
        let mut m = self.shared.0.lock().unwrap();
        m.input = Some(frame);
        m.posted += 1;
        self.shared.1.notify_one();
        m.posted
    }

    /// The newest instances, if any arrived since the last take; and the feed's last build time.
    pub fn take(&self) -> (Option<Arc<Vec<GpuAgent>>>, f64) {
        let (o, ms) = self.take_seq();
        (o.map(|(o, _)| o), ms)
    }

    /// `take`, with the post number of the frame the instances were built from.
    pub fn take_seq(&self) -> (Option<(Arc<Vec<GpuAgent>>, u64)>, f64) {
        let mut m = self.shared.0.lock().unwrap();
        let built = m.built;
        (m.output.take().map(|o| (o, built)), m.build_ms)
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
