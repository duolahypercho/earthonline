//! The collision world, streamed per tile.
//!
//! Engine: Rapier (the `rapier3d` crate), because the JS game's player,
//! vehicle and world colliders are Rapier (`@dimforge/rapier3d-deterministic-compat`
//! 0.20, src/simulation/physics/core/physics-runtime.js): the same kinematic
//! character controller (`computeColliderMovement`), the same raycast vehicle
//! (`DynamicRayCastVehicleController`), the same shapes (heightfield, trimesh
//! prisms).
//!
//! What a tile holds (as src/simulation/physics/world/world-physics.js builds
//! them for the boot window, here cut per tile):
//! * ground: a Rapier heightfield sampled every `HEIGHTFIELD_SPACING_M` (4 m) from
//!   the ground provider (`terrainHeightfieldPhysicsDescriptor`), WORLD layer;
//! * buildings: every footprint whose centroid lies in the tile, as the JS
//!   `buildingFootprintPrismMesh` prism: the exact ring, from the lowest ground
//!   sample under it minus the 0.25 m skirt to the highest plus the ground lift
//!   plus the source height, capped by the earcut triangulation. All prisms of a
//!   tile are one trimesh collider (an engine detail: fewer colliders to insert);
//!   WORLD layer;
//! * street surface, kerb tops and kerb faces when a [`GroundProvider`] supplies
//!   them (`surface_tile`): tops on the STREET layer, faces on WORLD, as the JS
//!   street cross-section colliders are.
//!
//! Tiles are keyed by integer cell (`TileKey`), 128 m square, cooked on worker
//! threads (sampling, earcut, Rapier's trimesh BVH build: everything except the
//! insertion) and handed to the owner thread, which inserts a bounded number
//! per frame. Tiles past the drop radius are removed. Nothing in a tile depends
//! on any other tile, so the keying scales to any number of tiles; for a world
//! bigger than f32 metres allow (the whole Earth) the key gains a region and
//! each region its own origin (not needed for one city: SF spans 20 km, which
//! f32 holds to about a millimetre).

use glam::DVec3;
use rapier3d::parry::query::ShapeCastOptions;
use rapier3d::parry::utils::Array2;
use rapier3d::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use crate::tuning;

pub const TILE_SIZE_M: f64 = 128.0;
/// Tiles whose rectangle is within this distance of the body are wanted.
pub const LOAD_RADIUS_M: f64 = 160.0;
/// And dropped past this.
pub const DROP_RADIUS_M: f64 = 260.0;

/// Collision layers (`PHYSICS_LAYERS` in src/simulation/physics/core/layers.js, the subset used here).
pub mod layer {
    use rapier3d::prelude::Group;
    pub const WORLD: Group = Group::GROUP_1;
    pub const STREET: Group = Group::GROUP_2;
    pub const VEHICLE: Group = Group::GROUP_3;
    pub const CHARACTER: Group = Group::GROUP_4;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey {
    pub x: i32,
    pub z: i32,
}

impl TileKey {
    pub fn of(x: f64, z: f64) -> Self {
        Self { x: (x / TILE_SIZE_M).floor() as i32, z: (z / TILE_SIZE_M).floor() as i32 }
    }
    pub fn origin(&self) -> (f64, f64) {
        (self.x as f64 * TILE_SIZE_M, self.z as f64 * TILE_SIZE_M)
    }
    /// Horizontal distance from (x, z) to this tile's rectangle.
    pub fn distance_to(&self, x: f64, z: f64) -> f64 {
        let (x0, z0) = self.origin();
        let dx = (x0 - x).max(0.0).max(x - (x0 + TILE_SIZE_M));
        let dz = (z0 - z).max(0.0).max(z - (z0 + TILE_SIZE_M));
        dx.hypot(dz)
    }
}

/// Extra walkable/barrier geometry for one tile, world coordinates.
#[derive(Clone, Debug, Default)]
pub struct SurfaceMesh {
    /// Walkable tops (street, sidewalk, kerb top): STREET layer.
    pub street: Vec<[[f64; 3]; 3]>,
    /// Barriers (kerb faces, retaining walls): WORLD layer.
    pub barrier: Vec<[[f64; 3]; 3]>,
}

/// Height (and, when the street mesh lands, kerb) provider. Another worker's
/// street-surface mesh plugs in here; the elevation grid is the default.
pub trait GroundProvider: Send + Sync {
    /// Bare ground height at (x, z), game metres (what the heightfield samples).
    fn ground_y(&self, x: f64, z: f64) -> f64;
    /// Street/sidewalk/kerb triangles inside a tile, if this provider has them.
    fn surface_tile(&self, _key: TileKey) -> Option<SurfaceMesh> {
        None
    }
}

/// The elevation grid as a ground provider (`sf-elevation.bin.gz`, bilinear).
pub struct ElevationGround(pub Arc<eo_data::ElevationGrid>);

impl GroundProvider for ElevationGround {
    fn ground_y(&self, x: f64, z: f64) -> f64 {
        self.0.ground(x, z)
    }
}

/// One building as collision sees it.
#[derive(Clone, Debug)]
pub struct Footprint {
    pub ring: Vec<[f64; 2]>,
    pub height: f64,
}

/// Footprints bucketed by the tile holding their centroid.
pub struct BuildingSet {
    pub buildings: Vec<Footprint>,
    pub by_tile: HashMap<TileKey, Vec<u32>>,
    /// Metres the drawn shells stand above bare terrain (moves the prism top).
    pub ground_lift_m: f64,
}

impl BuildingSet {
    pub fn new(buildings: Vec<Footprint>, ground_lift_m: f64) -> Self {
        let mut by_tile: HashMap<TileKey, Vec<u32>> = HashMap::new();
        for (i, b) in buildings.iter().enumerate() {
            if b.ring.len() < 3 {
                continue;
            }
            let n = b.ring.len() as f64;
            let cx = b.ring.iter().map(|p| p[0]).sum::<f64>() / n;
            let cz = b.ring.iter().map(|p| p[1]).sum::<f64>() / n;
            by_tile.entry(TileKey::of(cx, cz)).or_default().push(i as u32);
        }
        Self { buildings, by_tile, ground_lift_m }
    }

    /// Every merged building of the city (`City::buildings`).
    pub fn from_city(city: &eo_data::City, ground_lift_m: f64) -> Self {
        let b = city
            .buildings
            .iter()
            .map(|b| Footprint { ring: b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect(), height: b.height as f64 })
            .collect();
        Self::new(b, ground_lift_m)
    }

    pub fn empty() -> Self {
        Self::new(vec![], 0.0)
    }
}

/// An analytic copy of one prism (the JS controller's `exteriorBuildingBlocked`
/// guard reads these, not the collider).
#[derive(Clone, Debug)]
pub struct Prism {
    pub ring: Vec<[f64; 2]>,
    pub base_y: f64,
    pub top_y: f64,
    pub min: [f64; 2],
    pub max: [f64; 2],
}

/// The tile's ground samples, so the floor under any point is the collider's own surface.
#[derive(Clone, Debug)]
pub struct TileHeights {
    pub origin: [f64; 2],
    pub spacing: f64,
    pub n: usize,
    /// Row-major by z then x: `h[iz * n + ix]`.
    pub h: Vec<f32>,
}

impl TileHeights {
    /// Height of the heightfield triangle under (x, z) (parry's default split:
    /// the diagonal runs from (x0, z1) to (x1, z0)).
    pub fn height_at(&self, x: f64, z: f64) -> Option<f64> {
        let fx = (x - self.origin[0]) / self.spacing;
        let fz = (z - self.origin[1]) / self.spacing;
        let cells = (self.n - 1) as f64;
        if !(fx >= 0.0 && fz >= 0.0 && fx <= cells && fz <= cells) {
            return None;
        }
        let j = (fx.floor() as usize).min(self.n - 2);
        let i = (fz.floor() as usize).min(self.n - 2);
        let u = fx - j as f64;
        let v = fz - i as f64;
        let y = |iz: usize, ix: usize| self.h[iz * self.n + ix] as f64;
        let (y00, y10, y01, y11) = (y(i, j), y(i + 1, j), y(i, j + 1), y(i + 1, j + 1));
        Some(if u + v <= 1.0 { y00 + (y01 - y00) * u + (y10 - y00) * v } else { y11 + (y10 - y11) * (1.0 - u) + (y01 - y11) * (1.0 - v) })
    }
}

/// A tile cooked off-thread, ready to insert. (`Clone` shares the
/// collider shapes, so one cooked tile can be inserted in many worlds.)
#[derive(Clone)]
pub struct BuiltTile {
    pub key: TileKey,
    pub colliders: Vec<Collider>,
    pub heights: Arc<TileHeights>,
    pub prisms: Arc<Vec<Prism>>,
    pub cook_ms: f64,
    pub triangles: usize,
}

fn v3(p: [f64; 3]) -> Vector {
    Vector::new(p[0] as f32, p[1] as f32, p[2] as f32)
}

/// the heightfield's sample spacing. At scale 1 the JS's 4 m: a
/// 128 m tile's lattice holds every node of the 8 m elevation grid, so the
/// collider follows the bilinear terrain's creases exactly. On a scaled plan
/// the grid's cells are 8 x s m; the spacing is the largest of 4, 2, 1 m that
/// divides both the tile and the scaled cell (2 m at 1.25: 10 m cells), so the
/// lattice holds the grid's nodes again (the feet would otherwise sink up to
/// 0.2 m under a crease the lattice cuts across).
pub fn heightfield_spacing_m() -> f64 {
    let base = tuning::world::HEIGHTFIELD_SPACING_M;
    let cell = 8.0 * eo_data::groundscale::scale();
    [base, base / 2.0, base / 4.0]
        .into_iter()
        .find(|sp| (TILE_SIZE_M / sp).fract().abs() < 1e-9 && ((cell / sp) - (cell / sp).round()).abs() < 1e-9)
        .unwrap_or(base / 4.0)
}

/// the elevation grid's surveyed origin (sf-elevation.bin.gz header,
/// originX / originZ; a whole number of 8 m cells).
pub const ELEVATION_ORIGIN_SURVEYED_M: [f64; 2] = [-9872.0, -7512.0];

/// where the lattice starts past a multiple of its spacing (x, z), so it
/// holds the grid's nodes. The nodes are cell centres, origin + (i + 0.5) x
/// cell: at scale 1 on the 4 m lattice (offset 0, as before); at 1.25 they fall
/// 5 m past a multiple of 10 m, which a lattice from the tile corner missed
/// (Mount Vernon Avenue's 0.200 m spot): the 2 m lattice starts 1 m in.
pub fn heightfield_offset_m() -> [f64; 2] {
    let (sp, s) = (heightfield_spacing_m(), eo_data::groundscale::scale());
    ELEVATION_ORIGIN_SURVEYED_M.map(|o| {
        let r = (o * s + 4.0 * s).rem_euclid(sp);
        if r.abs() < 1e-9 || (sp - r).abs() < 1e-9 { 0.0 } else { r }
    })
}

/// Cook one tile: heightfield, building prisms, provider surface. Pure; runs on any thread.
pub fn cook_tile(key: TileKey, ground: &dyn GroundProvider, buildings: &BuildingSet) -> BuiltTile {
    let t = Instant::now();
    let spacing = heightfield_spacing_m();
    let (tx0, tz0) = key.origin();
    // an offset lattice (`heightfield_offset_m`) starts one spacing
    // before the tile and has one cell more, so it still covers the tile
    // (neighbours overlap by a strip of the same surface).
    let off = heightfield_offset_m();
    let shifted = off != [0.0, 0.0];
    let cells = (TILE_SIZE_M / spacing).round() as usize + usize::from(shifted);
    let n = cells + 1;
    let (x0, z0) = if shifted { (tx0 + off[0] - spacing, tz0 + off[1] - spacing) } else { (tx0, tz0) };
    let span = cells as f64 * spacing;
    let mut h = vec![0f32; n * n];
    for iz in 0..n {
        for ix in 0..n {
            h[iz * n + ix] = ground.ground_y(x0 + ix as f64 * spacing, z0 + iz as f64 * spacing) as f32;
        }
    }
    // Parry: rows advance along z, columns along x, column-major data.
    let mut data = vec![0f32; n * n];
    for j in 0..n {
        for i in 0..n {
            data[i + j * n] = h[i * n + j];
        }
    }
    let mut colliders = vec![];
    let mut triangles = cells * cells * 2;
    let hf = ColliderBuilder::heightfield(Array2::new(n, n, data), Vector::new(span as f32, 1.0, span as f32)) // span
        .translation(Vector::new((x0 + span / 2.0) as f32, 0.0, (z0 + span / 2.0) as f32))
        .friction(tuning::world::TERRAIN_FRICTION)
        .collision_groups(InteractionGroups::new(layer::WORLD, Group::ALL, InteractionTestMode::And))
        .build();
    colliders.push(hf);
    let heights = Arc::new(TileHeights { origin: [x0, z0], spacing, n, h });

    // Building prisms.
    let mut prisms = vec![];
    let mut verts: Vec<Vector> = vec![];
    let mut tris: Vec<[u32; 3]> = vec![];
    let (cx, cz) = (tx0 + TILE_SIZE_M / 2.0, tz0 + TILE_SIZE_M / 2.0); // the tile, not the lattice
    if let Some(ids) = buildings.by_tile.get(&key) {
        let mut ear = earcut::Earcut::<f64>::new();
        let mut tri_out: Vec<u32> = vec![];
        for &id in ids {
            let b = &buildings.buildings[id as usize];
            let ring = &b.ring;
            let n = ring.len();
            if n < 3 || !(b.height > 0.0) {
                continue;
            }
            let ax = ring.iter().map(|p| p[0]).sum::<f64>() / n as f64;
            let az = ring.iter().map(|p| p[1]).sum::<f64>() / n as f64;
            let mut lo = ground.ground_y(ax, az);
            let mut hi = lo;
            for p in ring {
                let g = ground.ground_y(p[0], p[1]);
                lo = lo.min(g);
                hi = hi.max(g);
            }
            let base_y = lo - tuning::world::BUILDING_SKIRT_M;
            let top_y = hi + buildings.ground_lift_m + b.height;
            tri_out.clear();
            ear.earcut(ring.iter().map(|p| [p[0], p[1]]), &[] as &[u32], &mut tri_out);
            if tri_out.is_empty() {
                continue;
            }
            let base = verts.len() as u32;
            for y in [base_y, top_y] {
                for p in ring {
                    verts.push(Vector::new((p[0] - cx) as f32, y as f32, (p[1] - cz) as f32));
                }
            }
            let nn = n as u32;
            for t in tri_out.chunks_exact(3) {
                // Floor and roof; Rapier's trimesh is two-sided for queries.
                tris.push([base + t[0], base + t[1], base + t[2]]);
                tris.push([base + nn + t[2], base + nn + t[1], base + nn + t[0]]);
            }
            for e in 0..nn {
                let a = e;
                let b2 = (e + 1) % nn;
                tris.push([base + a, base + b2, base + nn + b2]);
                tris.push([base + a, base + nn + b2, base + nn + a]);
            }
            let mut min = [f64::MAX; 2];
            let mut max = [f64::MIN; 2];
            for p in ring {
                min[0] = min[0].min(p[0]);
                min[1] = min[1].min(p[1]);
                max[0] = max[0].max(p[0]);
                max[1] = max[1].max(p[1]);
            }
            prisms.push(Prism { ring: ring.clone(), base_y, top_y, min, max });
        }
    }
    if !tris.is_empty() {
        triangles += tris.len();
        if let Ok(b) = ColliderBuilder::trimesh(verts, tris) {
            colliders.push(
                b.translation(Vector::new(cx as f32, 0.0, cz as f32))
                    .friction(tuning::world::BUILDING_FRICTION)
                    .collision_groups(InteractionGroups::new(layer::WORLD, Group::ALL, InteractionTestMode::And))
                    .build(),
            );
        }
    }
    if let Some(surface) = ground.surface_tile(key) {
        for (list, group) in [(&surface.street, layer::STREET), (&surface.barrier, layer::WORLD)] {
            if list.is_empty() {
                continue;
            }
            let mut v = vec![];
            let mut ix = vec![];
            for t in list.iter() {
                let b = v.len() as u32;
                for p in t {
                    v.push(v3([p[0] - cx, p[1], p[2] - cz]));
                }
                ix.push([b, b + 1, b + 2]);
            }
            triangles += ix.len();
            if let Ok(b) = ColliderBuilder::trimesh(v, ix) {
                colliders.push(
                    b.translation(Vector::new(cx as f32, 0.0, cz as f32))
                        .friction(tuning::world::TERRAIN_FRICTION)
                        .collision_groups(InteractionGroups::new(group, Group::ALL, InteractionTestMode::And))
                        .build(),
                );
            }
        }
    }
    BuiltTile { key, colliders, heights, prisms: Arc::new(prisms), cook_ms: t.elapsed().as_secs_f64() * 1e3, triangles }
}

struct Queue {
    wanted: Vec<TileKey>,
    center: (f64, f64),
    stop: bool,
}

/// Worker threads that cook requested tiles, nearest first.
pub struct TileCooker {
    shared: Arc<(Mutex<Queue>, Condvar)>,
    rx: Receiver<BuiltTile>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl TileCooker {
    pub fn start(ground: Arc<dyn GroundProvider>, buildings: Arc<BuildingSet>, threads: usize) -> Self {
        let shared = Arc::new((Mutex::new(Queue { wanted: vec![], center: (0.0, 0.0), stop: false }), Condvar::new()));
        let (tx, rx): (Sender<BuiltTile>, Receiver<BuiltTile>) = channel();
        let mut hs = vec![];
        for i in 0..threads.max(1) {
            let shared = shared.clone();
            let tx = tx.clone();
            let ground = ground.clone();
            let buildings = buildings.clone();
            hs.push(
                std::thread::Builder::new()
                    .name(format!("eo-player-tiles-{i}"))
                    .spawn(move || {
                        loop {
                            let key = {
                                let (m, cv) = &*shared;
                                let mut q = m.lock().unwrap();
                                loop {
                                    if q.stop {
                                        return;
                                    }
                                    if !q.wanted.is_empty() {
                                        let (cx, cz) = q.center;
                                        let mut best = 0;
                                        for (k, t) in q.wanted.iter().enumerate() {
                                            if t.distance_to(cx, cz) < q.wanted[best].distance_to(cx, cz) {
                                                best = k;
                                            }
                                        }
                                        break q.wanted.swap_remove(best);
                                    }
                                    q = cv.wait(q).unwrap();
                                }
                            };
                            let built = cook_tile(key, &*ground, &buildings);
                            if tx.send(built).is_err() {
                                return;
                            }
                        }
                    })
                    .expect("spawn tile cooker"),
            );
        }
        Self { shared, rx, threads: hs }
    }

    fn request(&self, keys: &[TileKey], center: (f64, f64)) {
        let (m, cv) = &*self.shared;
        let mut q = m.lock().unwrap();
        q.center = center;
        for k in keys {
            if !q.wanted.contains(k) {
                q.wanted.push(*k);
            }
        }
        cv.notify_all();
    }

    fn cancel_except(&self, keep: &HashSet<TileKey>) {
        let (m, _) = &*self.shared;
        m.lock().unwrap().wanted.retain(|k| keep.contains(k));
    }
}

impl Drop for TileCooker {
    fn drop(&mut self) {
        {
            let (m, cv) = &*self.shared;
            m.lock().unwrap().stop = true;
            cv.notify_all();
        }
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

struct Resident {
    colliders: Vec<ColliderHandle>,
    heights: Arc<TileHeights>,
    prisms: Arc<Vec<Prism>>,
}

#[derive(Clone, Debug, Default)]
pub struct StreamStats {
    pub tiles_inserted: usize,
    pub tiles_removed: usize,
    pub tiles_resident: usize,
    pub triangles_resident: usize,
    pub cook_ms_total: f64,
    pub cook_ms_max: f64,
    /// Longest owner-thread time spent inserting or removing tiles in one pump.
    pub pump_ms_max: f64,
    /// Longest single tile insertion / removal on the owner thread.
    pub insert_ms_max: f64,
    pub remove_ms_max: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct RayHit {
    pub distance: f64,
    pub point: DVec3,
    pub normal: DVec3,
    pub collider: ColliderHandle,
    pub vehicle: bool,
}

/// where a world's tiles come from when it does not cook its own: a
/// server cooks each tile once and every player's world inserts a copy
/// (`BuiltTile::clone` shares the shapes).
pub trait TileFeed: Send + Sync {
    /// The cooked tile if it is ready; else None, and the tile is queued
    /// (nearest `center` first).
    fn fetch(&self, key: TileKey, center: (f64, f64)) -> Option<BuiltTile>;
}

/// The Rapier world plus the tile stream.
pub struct CollisionWorld {
    pub physics: PhysicsWorld,
    pub ground: Arc<dyn GroundProvider>,
    pub buildings: Arc<BuildingSet>,
    cooker: Option<TileCooker>, // None when fed
    feed: Option<Arc<dyn TileFeed>>, // net
    resident: HashMap<TileKey, Resident>,
    requested: HashSet<TileKey>,
    pending: Vec<BuiltTile>,
    pub stats: StreamStats,
    /// At most this many tiles are inserted per pump (the rest wait a frame).
    pub insert_per_pump: usize,
    /// Removed tiles go here to be freed on another thread (None when fed: shared shapes are freed in place).
    graveyard: Option<Sender<(Vec<Collider>, Arc<TileHeights>, Arc<Vec<Prism>>)>>,
}

fn new_physics() -> PhysicsWorld {
    let mut physics = PhysicsWorld::new();
    physics.gravity = Vector::new(0.0, -tuning::GRAVITY_MPS2 as f32, 0.0);
    physics.integration_parameters.dt = tuning::FIXED_STEP_S as f32;
    physics.integration_parameters.num_solver_iterations = tuning::SOLVER_ITERATIONS;
    physics
}

impl CollisionWorld {
    pub fn new(ground: Arc<dyn GroundProvider>, buildings: Arc<BuildingSet>, threads: usize) -> Self {
        let physics = new_physics();
        let cooker = TileCooker::start(ground.clone(), buildings.clone(), threads);
        let (graveyard, rx) = channel::<(Vec<Collider>, Arc<TileHeights>, Arc<Vec<Prism>>)>();
        std::thread::Builder::new()
            .name("eo-player-tile-free".into())
            .spawn(move || {
                while let Ok(dead) = rx.recv() {
                    drop(dead);
                }
            })
            .expect("spawn tile graveyard");
        Self { physics, ground, buildings, cooker: Some(cooker), feed: None, resident: HashMap::new(), requested: HashSet::new(), pending: vec![], stats: StreamStats::default(), insert_per_pump: 1, graveyard: Some(graveyard) }
    }

    /// a world whose tiles come from a shared feed (no threads of its own).
    pub fn fed(ground: Arc<dyn GroundProvider>, buildings: Arc<BuildingSet>, feed: Arc<dyn TileFeed>) -> Self {
        Self { physics: new_physics(), ground, buildings, cooker: None, feed: Some(feed), resident: HashMap::new(), requested: HashSet::new(), pending: vec![], stats: StreamStats::default(), insert_per_pump: 1, graveyard: None }
    }

    fn wanted(x: f64, z: f64, radius: f64) -> Vec<TileKey> {
        let r = (radius / TILE_SIZE_M).ceil() as i32 + 1;
        let c = TileKey::of(x, z);
        let mut out = vec![];
        for dz in -r..=r {
            for dx in -r..=r {
                let k = TileKey { x: c.x + dx, z: c.z + dz };
                if k.distance_to(x, z) <= radius {
                    out.push(k);
                }
            }
        }
        out
    }

    /// Owner-thread streaming: request what is near `center`, insert what has
    /// been cooked (bounded), drop what is far. Returns the ms spent here.
    pub fn pump(&mut self, center: DVec3) -> f64 {
        let t = Instant::now();
        let want = Self::wanted(center.x, center.z, LOAD_RADIUS_M);
        if let Some(feed) = &self.feed {
            // a fed world takes what the shared feed has cooked.
            for k in want.iter() {
                if !self.resident.contains_key(k) && !self.pending.iter().any(|b| b.key == *k) {
                    if let Some(b) = feed.fetch(*k, (center.x, center.z)) {
                        self.pending.push(b);
                    }
                }
            }
        } else if let Some(cooker) = &self.cooker {
            let missing: Vec<TileKey> = want.iter().copied().filter(|k| !self.resident.contains_key(k) && !self.requested.contains(k)).collect();
            if !missing.is_empty() {
                self.requested.extend(missing.iter().copied());
                cooker.request(&missing, (center.x, center.z));
            }
            while let Ok(b) = cooker.rx.try_recv() {
                self.pending.push(b);
            }
        }
        // Nearest first.
        self.pending.sort_by(|a, b| b.key.distance_to(center.x, center.z).partial_cmp(&a.key.distance_to(center.x, center.z)).unwrap());
        let mut inserted = 0;
        while inserted < self.insert_per_pump {
            let Some(b) = self.pending.pop() else { break };
            self.requested.remove(&b.key);
            if b.key.distance_to(center.x, center.z) > DROP_RADIUS_M || self.resident.contains_key(&b.key) {
                continue;
            }
            self.insert(b);
            inserted += 1;
        }
        // Drop far tiles, one per pump (and forget far requests). The removed
        // colliders (heightfield and trimesh buffers) are freed off-thread.
        let far = self.resident.keys().copied().filter(|k| k.distance_to(center.x, center.z) > DROP_RADIUS_M).max_by(|a, b| a.distance_to(center.x, center.z).partial_cmp(&b.distance_to(center.x, center.z)).unwrap());
        if let Some(k) = far {
            let tr = Instant::now();
            if let Some(r) = self.resident.remove(&k) {
                let mut gone = Vec::with_capacity(r.colliders.len());
                for h in r.colliders {
                    if let Some(c) = self.physics.remove_collider(h) {
                        gone.push(c);
                    }
                }
                if let Some(g) = &self.graveyard {
                    let _ = g.send((gone, r.heights, r.prisms));
                }
                self.stats.tiles_removed += 1;
            }
            self.stats.remove_ms_max = self.stats.remove_ms_max.max(tr.elapsed().as_secs_f64() * 1e3);
        }
        let keep: HashSet<TileKey> = Self::wanted(center.x, center.z, DROP_RADIUS_M).into_iter().collect();
        self.requested.retain(|k| keep.contains(k));
        if let Some(cooker) = &self.cooker {
            cooker.cancel_except(&keep);
        }
        self.pending.retain(|b| keep.contains(&b.key));
        self.stats.tiles_resident = self.resident.len();
        let ms = t.elapsed().as_secs_f64() * 1e3;
        self.stats.pump_ms_max = self.stats.pump_ms_max.max(ms);
        ms
    }

    fn insert(&mut self, b: BuiltTile) {
        let t = Instant::now();
        let mut handles = vec![];
        for c in b.colliders {
            handles.push(self.physics.insert_collider(c, None));
        }
        self.stats.tiles_inserted += 1;
        self.stats.triangles_resident += b.triangles;
        self.stats.cook_ms_total += b.cook_ms;
        self.stats.cook_ms_max = self.stats.cook_ms_max.max(b.cook_ms);
        self.resident.insert(b.key, Resident { colliders: handles, heights: b.heights, prisms: b.prisms });
        self.stats.insert_ms_max = self.stats.insert_ms_max.max(t.elapsed().as_secs_f64() * 1e3);
    }

    /// Block until every tile within `radius` of `center` is resident (setup and tests).
    pub fn load_blocking(&mut self, center: DVec3, radius: f64) {
        let want = Self::wanted(center.x, center.z, radius);
        if let Some(feed) = self.feed.clone() {
            // wait on the shared feed.
            loop {
                for k in &want {
                    if !self.resident.contains_key(k) {
                        if let Some(b) = feed.fetch(*k, (center.x, center.z)) {
                            self.insert(b);
                        }
                    }
                }
                if want.iter().all(|k| self.resident.contains_key(k)) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            self.stats.tiles_resident = self.resident.len();
            self.physics.step();
            return;
        }
        if self.cooker.is_none() {
            return;
        }
        let missing: Vec<TileKey> = want.iter().copied().filter(|k| !self.resident.contains_key(k) && !self.requested.contains(k)).collect();
        self.requested.extend(missing.iter().copied());
        self.cooker.as_ref().unwrap().request(&missing, (center.x, center.z));
        loop {
            for i in (0..self.pending.len()).rev() {
                if want.contains(&self.pending[i].key) {
                    let b = self.pending.swap_remove(i);
                    self.requested.remove(&b.key);
                    if !self.resident.contains_key(&b.key) {
                        self.insert(b);
                    }
                }
            }
            if want.iter().all(|k| self.resident.contains_key(k)) {
                break;
            }
            match self.cooker.as_ref().unwrap().rx.recv() {
                Ok(b) => self.pending.push(b),
                Err(_) => break,
            }
        }
        self.stats.tiles_resident = self.resident.len();
        // Let the broad phase see the new colliders.
        self.physics.step();
    }

    pub fn is_resident(&self, x: f64, z: f64) -> bool {
        self.resident.contains_key(&TileKey::of(x, z))
    }

    /// The floor under (x, z): the resident heightfield's own triangle, or the
    /// provider's ground where the tile is not resident yet.
    pub fn floor_y(&self, x: f64, z: f64) -> f64 {
        if let Some(r) = self.resident.get(&TileKey::of(x, z)) {
            if let Some(y) = r.heights.height_at(x, z) {
                return y;
            }
        }
        self.ground.ground_y(x, z)
    }

    /// The top walkable surface at (x, z) within 3 m over the floor: a street
    /// or sidewalk where the provider draws one (resident tiles), else the floor.
    pub fn surface_y(&self, x: f64, z: f64) -> f64 {
        let floor = self.floor_y(x, z);
        match self.raycast(DVec3::new(x, floor + 3.0, z), DVec3::NEG_Y, 3.5, layer::STREET, None) {
            Some(h) => h.point.y.max(floor),
            None => floor,
        }
    }

    /// Prisms of the resident tiles around (x, z) within `reach`.
    pub fn prisms_near(&self, x: f64, z: f64, reach: f64) -> Vec<Arc<Vec<Prism>>> {
        let mut out = vec![];
        for k in Self::wanted(x, z, reach + 64.0) {
            if let Some(r) = self.resident.get(&k) {
                if !r.prisms.is_empty() {
                    out.push(r.prisms.clone());
                }
            }
        }
        out
    }

    /// A ray against the given layers. `exclude` skips a body (the driven car).
    pub fn raycast(&self, origin: DVec3, dir: DVec3, max: f64, filter: Group, exclude: Option<RigidBodyHandle>) -> Option<RayHit> {
        let ray = Ray::new(Vector::new(origin.x as f32, origin.y as f32, origin.z as f32), Vector::new(dir.x as f32, dir.y as f32, dir.z as f32));
        let mut f = QueryFilter::default().groups(InteractionGroups::new(Group::ALL, filter, InteractionTestMode::And));
        if let Some(b) = exclude {
            f = f.exclude_rigid_body(b);
        }
        let (h, hit) = self.physics.cast_ray_and_get_normal(&ray, max as f32, true, f)?;
        let vehicle = self.physics.colliders.get(h).map(|c| c.collision_groups().memberships.contains(layer::VEHICLE)).unwrap_or(false);
        let d = hit.time_of_impact as f64;
        Some(RayHit {
            distance: d,
            point: origin + dir * d,
            normal: DVec3::new(hit.normal.x as f64, hit.normal.y as f64, hit.normal.z as f64),
            collider: h,
            vehicle,
        })
    }

    /// A sphere swept along a ray (the camera's ball probe).
    pub fn sphere_cast(&self, origin: DVec3, dir: DVec3, max: f64, radius: f64, filter: Group, exclude: Option<RigidBodyHandle>) -> Option<(f64, bool)> {
        let ball = Ball::new(radius as f32);
        let pos = Pose::from_translation(Vector::new(origin.x as f32, origin.y as f32, origin.z as f32));
        let mut f = QueryFilter::default().groups(InteractionGroups::new(Group::ALL, filter, InteractionTestMode::And));
        if let Some(b) = exclude {
            f = f.exclude_rigid_body(b);
        }
        let opts = ShapeCastOptions { max_time_of_impact: max as f32, target_distance: 0.0, stop_at_penetration: true, compute_impact_geometry_on_penetration: false };
        let (h, hit) = self.physics.cast_shape(&pos, Vector::new(dir.x as f32, dir.y as f32, dir.z as f32), &ball, opts, f)?;
        let vehicle = self.physics.colliders.get(h).map(|c| c.collision_groups().memberships.contains(layer::VEHICLE)).unwrap_or(false);
        Some((hit.time_of_impact as f64, vehicle))
    }
}

/// `pointInFootprint` (world-physics.js): even-odd test.
pub fn point_in_ring(x: f64, z: f64, ring: &[[f64; 2]]) -> bool {
    let mut inside = false;
    let n = ring.len();
    let mut j = n - 1;
    for i in 0..n {
        let (xi, zi) = (ring[i][0], ring[i][1]);
        let (xj, zj) = (ring[j][0], ring[j][1]);
        if (zi > z) != (zj > z) && x < (xj - xi) * (z - zi) / (zj - zi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// `distanceToFootprintBoundary`.
pub fn distance_to_ring(x: f64, z: f64, ring: &[[f64; 2]]) -> f64 {
    let mut best = f64::INFINITY;
    let n = ring.len();
    for i in 0..n {
        let a = ring[i];
        let b = ring[(i + 1) % n];
        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
        let l2 = dx * dx + dz * dz;
        let t = if l2 > 0.0 { (((x - a[0]) * dx + (z - a[1]) * dz) / l2).clamp(0.0, 1.0) } else { 0.0 };
        let (px, pz) = (a[0] + dx * t, a[1] + dz * t);
        best = best.min((x - px).hypot(z - pz));
    }
    best
}
