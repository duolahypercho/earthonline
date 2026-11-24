//! Moored boats in the physics world: each a Rapier body floating on the sea
//! the water draws (eo-water `sea`), held to its berth by its mooring lines.
//!
//! The sea is the drawn one: the sea state's plan and the world clock give
//! the same surface on every machine, so the server's world for each player
//! and each client's own float the same boats (a boat is a damped body
//! driven by the waves: whenever it was spawned, it settles to the same
//! motion within a few seconds). Where the drawn sea has no waves (the
//! shore holds them back, a calm), a boat lies still.
//!
//! A hull floats on six points of its waterplane (three along the
//! waterline, two across), each pushed up by the water over it:
//! rho g A (d0 + h - y), A its share of the waterplane, d0 the depth at
//! which the six carry the boat's displacement in still water, h the drawn
//! sea's height there, y the point's own. The six give the hull its
//! stiffness in heave, roll and pitch (the waterplane's area and inertia);
//! its mass sits a little below the waterline, so it is stable; water
//! damps its motion. Two mooring lines, at the bow and the stern, pull it
//! back to its berth across the water and leave it free to rise and roll.
//!
//! Boats within `SIM_RADIUS_M` of the streaming centre are bodies (players
//! and cars meet them: the `BOAT` layer); the rest float quasi-statically
//! on the same sea for the renderer ([`float_pose`]).

use crate::world::layer;
use eo_water::sea::{Sample, SeaField, SeaGrid, Shore};
use glam::{DQuat, DVec3};
use rapier3d::prelude::*;
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};

/// Boats within this of the streaming centre are bodies, past the second they are dropped.
pub const SIM_RADIUS_M: f64 = 250.0;
pub const DROP_RADIUS_M: f64 = 300.0;
/// The sea's grids lie this far apart in time; a step reads the two about it.
pub const SEA_STEP_S: f64 = 0.2;
/// Sea water (kg/m3).
pub const WATER_DENSITY: f64 = 1025.0;
const G: f64 = 9.81;
/// The waterplane's fill of its length x beam (a yacht's is 0.70 to 0.75). INFERRED.
pub const WATERPLANE_COEFFICIENT: f64 = 0.72;
/// The waterline's length against the length overall. INFERRED (the bodies' provenance: 0.86 to 0.92).
pub const LWL_OVER_LOA: f64 = 0.88;
/// The six points: along the waterline at these shares of its length, across at this
/// share of the beam (a rectangle's waterplane inertia: B / sqrt 12).
const ALONG: [f64; 3] = [-0.33, 0.0, 0.33];
const ACROSS: f64 = 0.289;
/// The centre of mass below the waterline, a share of the depth of the canoe body. INFERRED.
const MASS_BELOW: f64 = 0.4;
/// Water's damping of the hull's motion (1/s): Rapier's linear and angular damping. INFERRED.
const LINEAR_DAMPING: f32 = 0.9;
const ANGULAR_DAMPING: f32 = 1.6;
/// Mooring lines: the period of the hull's surge on them (s) and their damping ratio. INFERRED.
const MOORING_PERIOD_S: f64 = 9.0;
const MOORING_DAMPING: f64 = 0.6;

/// What a hull floats by (eo-props `boats::Body`, from the bodies' provenance).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hull {
    /// Length overall, beam of the hull (m).
    pub loa: f64,
    pub beam: f64,
    /// Displacement (kg).
    pub mass_kg: f64,
}

impl Hull {
    fn lwl(&self) -> f64 {
        self.loa * LWL_OVER_LOA
    }
    fn waterplane(&self) -> f64 {
        WATERPLANE_COEFFICIENT * self.lwl() * self.beam
    }
    /// The canoe body's depth in still water: the six points carry the displacement.
    pub fn rest_depth(&self) -> f64 {
        self.mass_kg / (WATER_DENSITY * self.waterplane())
    }
    /// The hull's height over the waterline (for its collider). INFERRED: 10 % of its length and 0.2 m.
    pub fn freeboard(&self) -> f64 {
        0.1 * self.loa + 0.2
    }
    /// The six points (local: +x port, +z bow, on the waterline).
    fn points(&self) -> [DVec3; 6] {
        let (l, b) = (self.lwl(), self.beam);
        let mut p = [DVec3::ZERO; 6];
        for (i, a) in ALONG.iter().enumerate() {
            p[i * 2] = DVec3::new(ACROSS * b, 0.0, a * l);
            p[i * 2 + 1] = DVec3::new(-ACROSS * b, 0.0, a * l);
        }
        p
    }
}

/// A boat in its berth: its hull, the middle of its waterline on the plan
/// (game metres) and its bow's heading (radians from +x toward +z).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Moored {
    pub hull: Hull,
    pub pos: [f64; 2],
    pub heading: f64,
}

impl Moored {
    /// The body's yaw (local +z, the bow, toward (sin yaw, cos yaw)).
    pub fn yaw(&self) -> f64 {
        std::f64::consts::FRAC_PI_2 - self.heading
    }
}

/// Every moored boat of the map and the still water's level they float on.
#[derive(Clone, Debug, Default)]
pub struct Fleet {
    pub boats: Vec<Moored>,
    pub sea_y: f64,
}

/// Where the shore field is read (the water's citywide field, or a test's).
pub trait ShoreLookup: Send + Sync {
    fn shore(&self, x: f64, z: f64) -> Shore;
}

impl ShoreLookup for eo_water::field::ShoreField {
    fn shore(&self, x: f64, z: f64) -> Shore {
        Shore::of(self, x, z)
    }
}

impl ShoreLookup for eo_water::WaterData {
    fn shore(&self, x: f64, z: f64) -> Shore {
        Shore::of(&self.field, x, z)
    }
}

/// The sea the boats float on: one plan's field (eo-water `SeaField`), the
/// shore field, and its grids on a lattice of times (`SEA_STEP_S`), built
/// once each and ahead of need on threads of their own.
pub struct Sea {
    field: Arc<SeaField>,
    shore: Arc<dyn ShoreLookup>,
    grids: Mutex<HashMap<i64, Option<Arc<SeaGrid>>>>,
    ready: Condvar,
}

impl Sea {
    pub fn new(field: Arc<SeaField>, shore: Arc<dyn ShoreLookup>) -> Arc<Self> {
        Arc::new(Self { field, shore, grids: Mutex::new(HashMap::new()), ready: Condvar::new() })
    }

    pub fn shore(&self, x: f64, z: f64) -> Shore {
        self.shore.shore(x, z)
    }

    /// The drawn sea at a point and time if its grids are built (else starts them; the
    /// renderer's read, which never waits).
    pub fn try_sample(self: &Arc<Self>, x: f64, z: f64, t: f64) -> Option<Sample> {
        let k = (t / SEA_STEP_S).floor();
        let f = t / SEA_STEP_S - k;
        let k = k as i64;
        for ahead in 0..=3 {
            self.prefetch(k + ahead);
        }
        let g = self.grids.lock().unwrap();
        let (a, b) = (g.get(&k)?.clone()?, g.get(&(k + 1))?.clone()?);
        drop(g);
        let shore = self.shore(x, z);
        Some(Sample::lerp(a.sample(x, z, shore), b.sample(x, z, shore), f))
    }

    /// Start the grid at lattice time `k` on a thread unless it is there or coming.
    fn prefetch(self: &Arc<Self>, k: i64) {
        let mut g = self.grids.lock().unwrap();
        if g.contains_key(&k) {
            return;
        }
        g.insert(k, None);
        drop(g);
        let me = self.clone();
        std::thread::Builder::new()
            .name("eo-player-sea".into())
            .spawn(move || {
                let grid = Arc::new(me.field.grid(k as f64 * SEA_STEP_S));
                me.grids.lock().unwrap().insert(k, Some(grid));
                me.ready.notify_all();
            })
            .expect("spawn sea grid");
    }

    /// The grid at lattice time `k` (waits for it if it is still being built).
    fn grid(self: &Arc<Self>, k: i64) -> Arc<SeaGrid> {
        self.prefetch(k);
        for ahead in 1..=3 {
            self.prefetch(k + ahead);
        }
        let mut g = self.grids.lock().unwrap();
        loop {
            if let Some(Some(grid)) = g.get(&k) {
                let grid = grid.clone();
                // keep the grids about now (a clock jump forgets the rest)
                g.retain(|&q, _| q >= k - 2 && q <= k + 6);
                return grid;
            }
            g = self.ready.wait(g).unwrap();
        }
    }

    /// The drawn sea at a point and world time (between its grids' times).
    pub fn sample(self: &Arc<Self>, x: f64, z: f64, t: f64) -> Sample {
        let k = (t / SEA_STEP_S).floor();
        let f = t / SEA_STEP_S - k;
        let (a, b) = (self.grid(k as i64), self.grid(k as i64 + 1));
        let shore = self.shore(x, z);
        Sample::lerp(a.sample(x, z, shore), b.sample(x, z, shore), f)
    }
}

/// A boat's pose floating at rest on the sea at `t`, without dynamics: its
/// six points' water levels fitted by a plane (the heave, the roll and the
/// pitch the waterplane's stiffness alone would give). The renderer's pose
/// for a boat no body carries.
pub fn float_pose(sea: &Arc<Sea>, m: &Moored, sea_y: f64, t: f64) -> (DVec3, DQuat) {
    float_pose_with(m, sea_y, |x, z| Some(sea.sample(x, z, t).height)).expect("a waiting read")
}

/// `float_pose` from whatever reads the sea's height (None: not yet built).
pub fn float_pose_with(m: &Moored, sea_y: f64, height: impl Fn(f64, f64) -> Option<f64>) -> Option<(DVec3, DQuat)> {
    let yaw = DQuat::from_rotation_y(m.yaw());
    let centre = DVec3::new(m.pos[0], sea_y, m.pos[1]);
    let pts = m.hull.points();
    let (mut h, mut roll_num, mut roll_den, mut pitch_num, mut pitch_den) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for p in pts {
        let w = centre + yaw * p;
        let s = height(w.x, w.z)?;
        h += s / 6.0;
        roll_num += s * p.x;
        roll_den += p.x * p.x;
        pitch_num += s * p.z;
        pitch_den += p.z * p.z;
    }
    // the waterplane takes the water's slope: a rotation r about +z lifts a point x to port by
    // x sin r, a rotation p about +x drops a point z forward by z sin p.
    let (roll, pitch) = ((roll_num / roll_den).atan(), -(pitch_num / pitch_den).atan());
    let rot = yaw * DQuat::from_rotation_x(pitch) * DQuat::from_rotation_z(roll);
    Some((centre + DVec3::Y * h, rot))
}

struct BoatBody {
    body: RigidBodyHandle,
}

/// The boats a physics world carries now.
pub struct Harbour {
    pub fleet: Arc<Fleet>,
    pub sea: Arc<Sea>,
    bodies: HashMap<usize, BoatBody>,
    /// The sea's clock (s): the world clock the water is drawn at, advanced a fixed step each step.
    pub time: f64,
}

impl Harbour {
    pub fn new(fleet: Arc<Fleet>, sea: Arc<Sea>, time: f64) -> Self {
        Self { fleet, sea, bodies: HashMap::new(), time }
    }

    /// Bodies for the boats near `center`, none for those far from it (canonical order: by index).
    pub fn stream(&mut self, physics: &mut PhysicsWorld, center: DVec3) {
        let near = |m: &Moored, r: f64| (m.pos[0] - center.x).hypot(m.pos[1] - center.z) <= r;
        let gone: Vec<usize> = self.bodies.keys().copied().filter(|&i| !near(&self.fleet.boats[i], DROP_RADIUS_M)).collect();
        for i in gone {
            if let Some(b) = self.bodies.remove(&i) {
                physics.remove_body_with_colliders(b.body, true);
            }
        }
        for i in 0..self.fleet.boats.len() {
            let m = self.fleet.boats[i];
            if self.bodies.contains_key(&i) || !near(&m, SIM_RADIUS_M) {
                continue;
            }
            let (pos, rot) = float_pose(&self.sea, &m, self.fleet.sea_y, self.time);
            let hull = m.hull;
            let depth = 2.0 * hull.rest_depth();
            let (fb, half_b, half_l) = (hull.freeboard(), hull.beam / 2.0, hull.loa / 2.0);
            let pose = Pose::from_parts(Vector::new(pos.x as f32, pos.y as f32, pos.z as f32), Rotation::from_xyzw(rot.x as f32, rot.y as f32, rot.z as f32, rot.w as f32));
            let (b, l) = (hull.beam, hull.loa);
            let body = RigidBodyBuilder::dynamic()
                .pose(pose)
                .linear_damping(LINEAR_DAMPING)
                .angular_damping(ANGULAR_DAMPING)
                .additional_mass_properties(MassProperties::new(
                    Vector::new(0.0, (-MASS_BELOW * depth) as f32, 0.0),
                    hull.mass_kg as f32,
                    // roll about z (k = 0.35 B), pitch about x and yaw (k = 0.25 L)
                    Vector::new((hull.mass_kg * (0.25 * l).powi(2)) as f32, (hull.mass_kg * (0.25 * l).powi(2)) as f32, (hull.mass_kg * (0.35 * b).powi(2)) as f32),
                ))
                .build();
            let body = physics.insert_body(body);
            let collider = ColliderBuilder::cuboid(half_b as f32, ((fb + depth) / 2.0) as f32, half_l as f32)
                .translation(Vector::new(0.0, ((fb - depth) / 2.0) as f32, 0.0))
                .density(0.0)
                .friction(0.6)
                .restitution(0.05)
                .collision_groups(InteractionGroups::new(layer::BOAT, layer::VEHICLE | layer::CHARACTER | layer::BOAT, InteractionTestMode::And))
                .build();
            physics.insert_collider(collider, Some(body));
            let (bodies, colliders) = (&mut physics.bodies, &physics.colliders);
            bodies[body].recompute_mass_properties_from_colliders(colliders);
            self.bodies.insert(i, BoatBody { body });
        }
    }

    /// Before a physics step: each body's buoyancy at its six points, its
    /// mooring lines; then the sea's clock moves on by the step.
    pub fn apply_forces(&mut self, physics: &mut PhysicsWorld, dt: f64) {
        let t = self.time;
        let sea_y = self.fleet.sea_y;
        let mut keys: Vec<usize> = self.bodies.keys().copied().collect();
        keys.sort_unstable(); // a fixed order on every machine
        for i in keys {
            let m = self.fleet.boats[i];
            let h = self.bodies[&i].body;
            let Some(rb) = physics.bodies.get_mut(h) else { continue };
            rb.reset_forces(false);
            rb.reset_torques(false);
            let (tr, q) = (rb.translation(), rb.rotation());
            let pos = DVec3::new(tr.x as f64, tr.y as f64, tr.z as f64);
            let rot = DQuat::from_xyzw(q.x as f64, q.y as f64, q.z as f64, q.w as f64);
            let hull = m.hull;
            let (d0, area, top) = (hull.rest_depth(), hull.waterplane() / 6.0, hull.rest_depth() + hull.freeboard());
            for pt in hull.points() {
                let w = pos + rot * pt;
                let water = sea_y + self.sea.sample(w.x, w.z, t).height;
                let depth = (d0 + water - w.y).clamp(0.0, top);
                let f = WATER_DENSITY * G * area * depth;
                rb.add_force_at_point(Vector::new(0.0, f as f32, 0.0), Vector::new(w.x as f32, w.y as f32, w.z as f32), true);
            }
            // the mooring lines: the bow and the stern pulled back across the water to their berth.
            let rest_rot = DQuat::from_rotation_y(m.yaw());
            let rest = DVec3::new(m.pos[0], sea_y, m.pos[1]);
            let k = hull.mass_kg / 2.0 * (std::f64::consts::TAU / MOORING_PERIOD_S).powi(2);
            let c = 2.0 * MOORING_DAMPING * (k * hull.mass_kg / 2.0).sqrt();
            let lin = rb.linvel();
            let ang = rb.angvel();
            let (lin, ang) = (DVec3::new(lin.x as f64, lin.y as f64, lin.z as f64), DVec3::new(ang.x as f64, ang.y as f64, ang.z as f64));
            for end in [0.45, -0.45] {
                let local = DVec3::new(0.0, 0.0, end * hull.loa);
                let (now, want) = (pos + rot * local, rest + rest_rot * local);
                let vel = lin + ang.cross(rot * local);
                let mut f = (want - now) * k - vel * c;
                f.y = 0.0;
                rb.add_force_at_point(Vector::new(f.x as f32, 0.0, f.z as f32), Vector::new(now.x as f32, now.y as f32, now.z as f32), true);
            }
        }
        self.time += dt;
    }

    /// The boats the bodies carry now: (fleet index, waterline middle, rotation).
    pub fn poses(&self, physics: &PhysicsWorld) -> Vec<(usize, DVec3, DQuat)> {
        let mut out: Vec<(usize, DVec3, DQuat)> = self
            .bodies
            .iter()
            .filter_map(|(&i, b)| {
                let rb = physics.bodies.get(b.body)?;
                let (t, r) = (rb.translation(), rb.rotation());
                Some((i, DVec3::new(t.x as f64, t.y as f64, t.z as f64), DQuat::from_xyzw(r.x as f64, r.y as f64, r.z as f64, r.w as f64)))
            })
            .collect();
        out.sort_unstable_by_key(|p| p.0);
        out
    }

    pub fn bodies(&self) -> usize {
        self.bodies.len()
    }
}
