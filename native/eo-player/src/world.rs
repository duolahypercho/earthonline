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
//!   street cross-section colliders are; with the grade-separated roads
//!   (`--structures`, `street_ground.rs`) also their deck tops (STREET) and
//!   concrete (WORLD), and the ground has several levels
//!   (`GroundProvider::multi_level`): `surface_below` and `floor_under` then
//!   find the surface nearest below a body, where `surface_y` and `floor_y`
//!   answer for the ground.
//!
//! Tiles are keyed by integer cell (`TileKey`), 128 m square, cooked on worker
//! threads (sampling, earcut, Rapier's trimesh BVH build: everything except the
//! insertion) and handed to the owner thread, which inserts a bounded number
//! per frame. Tiles past the drop radius are removed. With several levels
//! (`--structures`) every world inserts and drops them on one canonical
//! schedule, set by the streaming centre's path alone (`pump_canonical`), so
//! a server's fed world and a client's own hand out the same collider handles
//! and solve a body's contacts in the same order. A lockstep world (a
//! networked player's, `CollisionWorld::lockstep`) inserts and drops them
//! before each authoritative step instead (`ensure_step_tiles`; with several
//! levels on the same canonical schedule), so the server's and the client's
//! follow the body's step path, not the frames or the cookers. Nothing in a
//! tile depends on any other tile, so the keying scales to any number of
//! tiles; for a world bigger than f32 metres allow (the whole Earth) the key
//! gains a region and each region its own origin (not needed for one city:
//! SF spans 20 km, which f32 holds to about a millimetre).

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
/// With several levels (`CollisionWorld::pump_canonical`) tiles are asked
/// for this far beyond the load radius, so a tile is cooked before it is
/// due and the canonical schedule never waits on a worker in play: three
/// seconds at 32 m/s. INFERRED (a streaming margin).
pub const PREFETCH_M: f64 = 96.0;

/// The canonical order of tiles about a centre: nearest first, then by key.
fn canonical(a: &TileKey, b: &TileKey, c: (f64, f64)) -> std::cmp::Ordering {
    a.distance_to(c.0, c.1).total_cmp(&b.distance_to(c.0, c.1)).then(a.cmp(b))
}

/// The reach of one authoritative fixed step: every tile whose rectangle
/// lies within this of the body is resident before the step runs
/// (`CollisionWorld::ensure_step_tiles`). A step reads the building prisms of
/// the tiles within its sweep plus 64 m (`prisms_near`, a footprint is keyed
/// by its centroid), the car moves at most 1 m a step at its 60 m/s top speed
/// and the exit probe looks a car's width beside it: 96 m holds all of that
/// with a margin and stays inside the `LOAD_RADIUS_M` the stream prefetches.
pub const STEP_RADIUS_M: f64 = 96.0;

/// Collision layers (`PHYSICS_LAYERS` in src/simulation/physics/core/layers.js, the subset used here).
pub mod layer {
    use rapier3d::prelude::Group;
    pub const WORLD: Group = Group::GROUP_1;
    pub const STREET: Group = Group::GROUP_2;
    pub const VEHICLE: Group = Group::GROUP_3;
    pub const CHARACTER: Group = Group::GROUP_4;
    /// Moored boats (harbour.rs): met by vehicles, characters and each other.
    pub const BOAT: Group = Group::GROUP_5;
    pub const PEDESTRIAN: Group = Group::GROUP_6;
    /// Static vertical faces still block the chassis. A suspension ray
    /// grazing one reaches the load-bearing surface behind it.
    pub const VERTICAL_FACE: Group = Group::GROUP_7;
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
    /// Paved street cross-sections and junctions shared with traffic on this ground.
    fn traffic_plan(&self) -> eo_data::sim_source::TrafficPlan {
        eo_data::sim_source::TrafficPlan::default()
    }
    /// Mapped carriageway nearest this elevation for an arrival with no height hint.
    fn arrival_road_y(&self, _x: f64, _z: f64, _near_y: f64) -> Option<f64> { None }

    /// Sea floor and still-water surface for a playable water column, in world metres.
    fn water_column(&self, _x: f64, _z: f64) -> Option<(f64, f64)> { None }
    /// Street/sidewalk/kerb triangles inside a tile, if this provider has them.
    fn surface_tile(&self, _key: TileKey) -> Option<SurfaceMesh> {
        None
    }
    /// True when walkable surfaces stand over other walkable surfaces (the
    /// decks of the grade-separated roads, `--structures`): the collision
    /// world's level queries (`CollisionWorld::surface_below`,
    /// `floor_under`) then look for the surface nearest below a body instead
    /// of the one over the ground. False (one level, today's queries) unless
    /// a provider says otherwise.
    fn multi_level(&self) -> bool {
        false
    }
    /// The streamed surface already contains the physical landmark bridge panels.
    fn has_landmark_roads(&self) -> bool {
        false
    }
    /// Structural cut polygons intersecting a tile and its terrain sampling border.
    fn terrain_cuts_in(&self, _min: [f64; 2], _max: [f64; 2]) -> Vec<&eo_structures::TerrainCut> { Vec::new() }

    /// Reference elevation of the tunnel cut at (x, z). Providers with
    /// cut polygons distinguish a lowered open approach from a portal opening.
    /// A scalar-only provider retains the original removed-cell contract.
    fn terrain_cut(&self, _x: f64, _z: f64) -> Option<f64> {
        None
    }
}

/// How far over a body's feet (or a car's road datum) a surface beside it
/// still counts as its own level when the ground has several
/// (`GroundProvider::multi_level`): the reach `CollisionWorld::surface_y`
/// has always searched over the floor (a kerb, a ramp, a car put a few
/// metres up a 30 % hill). Every deck clears the street under it by far
/// more (`structure-vertical-clearance-local` plus the deck's depth).
pub const LEVEL_REACH_M: f64 = 3.0;

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
    /// landmark access: the walkable decks, their lifts and the prisms they open.
    pub decks: Arc<InstalledDecks>,
}

/// One landmark deck's collision: walkable floors (STREET layer) and its
/// railings, glazing and cores (WORLD layer), world metres. Every world
/// holds them from its creation (`CollisionWorld::new`, `fed`), not by tile,
/// so a lift's far stop is solid the step it is reached on a server and on
/// every client alike, whatever their tiles' streaming has done.
#[derive(Clone, Debug, Default)]
pub struct DeckPart {
    pub name: String,
    /// Building floors also participate in WORLD-only architectural probes.
    pub building_floor: bool,
    pub walk: Vec<[[f64; 3]; 3]>,
    pub barrier: Vec<[[f64; 3]; 3]>,
}

impl From<&eo_landmarks::promenade::Surfaces> for DeckPart {
    fn from(s: &eo_landmarks::promenade::Surfaces) -> Self {
        let triangles = |faces: &[[[f32; 3]; 3]]| faces.iter().map(|t| t.map(|p| p.map(f64::from))).collect();
        Self { name: s.name.into(), walk: triangles(&s.walk), barrier: triangles(&s.barrier), ..Default::default() }
    }
}

/// Building prisms whose footprint centroid lies within `radius_m` of
/// `centre` are open between `y0` and `y1` (a deck stands inside them there):
/// both the prism collider and the analytic building guard are split.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrismCut {
    pub centre: [f64; 2],
    pub radius_m: f64,
    pub y0: f64,
    pub y1: f64,
}

/// A lift between a ground entrance and a deck: feet and the camera yaw
/// (the player's convention, forward = (sin yaw, 0, cos yaw)) at each stop.
#[derive(Clone, Debug, PartialEq)]
pub struct LiftDef {
    pub name: String,
    pub bottom: DVec3,
    pub bottom_yaw: f64,
    pub top: DVec3,
    pub top_yaw: f64,
}

/// Everything landmark access adds to the collision world (empty: none).
#[derive(Clone, Debug, Default)]
pub struct Decks {
    pub parts: Vec<DeckPart>,
    pub cuts: Vec<PrismCut>,
    pub lifts: Vec<LiftDef>,
}

/// Immutable map geometry. Each simulation world needs its own collider
/// handles, but all can reference the same cooked triangle meshes and BVHs.
#[derive(Default)]
pub struct InstalledDecks {
    layout: Decks,
    colliders: Vec<Collider>,
}

impl std::ops::Deref for InstalledDecks {
    type Target = Decks;
    fn deref(&self) -> &Decks { &self.layout }
}

impl Decks {
    /// The vertical spans of a prism from `base_y` to `top_y` left by the cuts
    /// that reach its footprint centroid (`ax`, `az`).
    pub fn spans(&self, ax: f64, az: f64, base_y: f64, top_y: f64) -> Vec<(f64, f64)> {
        let mut spans = vec![(base_y, top_y)];
        for c in &self.cuts {
            if (ax - c.centre[0]).hypot(az - c.centre[1]) > c.radius_m {
                continue;
            }
            let mut next = Vec::with_capacity(spans.len() + 1);
            for (a, b) in spans {
                if c.y1 <= a || c.y0 >= b {
                    next.push((a, b));
                    continue;
                }
                if c.y0 > a {
                    next.push((a, c.y0));
                }
                if c.y1 < b {
                    next.push((c.y1, b));
                }
            }
            spans = next;
        }
        spans.retain(|(a, b)| b - a > 0.01);
        spans
    }
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
        Self { buildings, by_tile, ground_lift_m, decks: Arc::default() }
    }

    /// The same buildings with landmark decks (their lifts, the prisms they open).
    pub fn with_decks(mut self, decks: Decks) -> Self {
        self.decks = Arc::new(InstalledDecks { colliders: deck_colliders(&decks), layout: decks });
        self
    }

    /// Remove the footprint records the renderer omits at tunnel mouths.
    /// Retain city record indices so landmark replacements still address the same buildings.
    pub fn open_tunnel_approaches(&mut self, structures: Option<&eo_structures::StructureWorld>) {
        let Some(w) = structures.filter(|w| !w.terrain_cuts().is_empty()) else { return };
        let rings = self.buildings.iter().map(|b| b.ring.as_slice()).collect::<Vec<_>>();
        let omitted = w.buildings_over_cuts(&rings);
        for bucket in self.by_tile.values_mut() {
            bucket.retain(|&i| omitted.binary_search(&(i as usize)).is_err());
        }
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
    /// The cut reference floor by cell (row-major z then x). Open cuts
    /// use the sampled terrain heights. Inside a portal opening this is
    /// the query fallback. Outside cuts the value is NaN.
    pub cut: Vec<f32>,
    /// Cells replaced by terrain triangles clipped at the cut or canopy.
    removed: Vec<bool>,
    canopy_rings: Vec<Vec<[f64; 2]>>,
    open_rings: Vec<(Vec<[f64; 2]>, f64)>,
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
        let cell = i * (self.n - 1) + j;
        if self.removed.get(cell) == Some(&true) && ((self.canopy_rings.is_empty() && self.open_rings.is_empty())
            || self.canopy_rings.iter().any(|ring| point_in_ring(x, z, ring))) {
            return Some(self.cut[cell] as f64);
        }
        let u = fx - j as f64;
        let v = fz - i as f64;
        let y = |iz: usize, ix: usize| self.h[iz * self.n + ix] as f64;
        let (y00, y10, y01, y11) = (y(i, j), y(i + 1, j), y(i, j + 1), y(i + 1, j + 1));
        let terrain = if u + v <= 1.0 { y00 + (y01 - y00) * u + (y10 - y00) * v } else { y11 + (y10 - y11) * (1.0 - u) + (y01 - y11) * (1.0 - v) };
        Some(if self.removed.get(cell) == Some(&true) {
            self.open_rings.iter().filter(|(ring, _)| point_in_ring(x, z, ring)).map(|(_, floor)| *floor).fold(terrain, f64::min)
        } else { terrain })
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

/// Sample modified terrain on the elevation grid the city draws, then subdivide
/// its two triangles onto the physics lattice. Unmodified terrain retains its
/// normal height sampling. Adjacent physics tiles use the same survey origin.
fn shape_open_cut_ground(heights: &mut [f32], n: usize, origin: [f64; 2], spacing: f64,
    cell: f64, reach: f64, cuts: &[&eo_structures::TerrainCut], ground: &dyn GroundProvider) {
    let scale = eo_data::groundscale::scale();
    let first = ELEVATION_ORIGIN_SURVEYED_M.map(|v| v * scale + cell * 0.5);
    let base = [0, 1].map(|i| first[i] + ((origin[i] - first[i]) / cell).floor() * cell);
    let span = (n - 1) as f64 * spacing;
    let dims = [0, 1].map(|i| ((origin[i] + span - base[i]) / cell).floor() as usize + 2);
    let mut vertices = Vec::with_capacity(dims[0] * dims[1]);
    for iz in 0..dims[1] {
        for ix in 0..dims[0] {
            let (x, z) = (base[0] + ix as f64 * cell, base[1] + iz as f64 * cell);
            let y = ground.ground_y(x, z) as f32;
            let lowered = eo_structures::cut_ground::terrain_cut_floor(cuts, x, z, reach)
                .map_or(y, |floor| y.min(floor as f32));
            vertices.push((lowered, lowered < y));
        }
    }
    for (k, height) in heights.iter_mut().enumerate() {
        let x = (origin[0] - base[0] + (k % n) as f64 * spacing) / cell;
        let z = (origin[1] - base[1] + (k / n) as f64 * spacing) / cell;
        let j = z.floor() as usize * dims[0] + x.floor() as usize;
        let (u, v) = (x.fract() as f32, z.fract() as f32);
        let (ids, weights) = if u + v <= 1.0 {
            ([j, j + 1, j + dims[0]], [1.0 - u - v, u, v])
        } else {
            ([j + dims[0] + 1, j + dims[0], j + 1], [u + v - 1.0, 1.0 - u, 1.0 - v])
        };
        if ids.iter().any(|&i| vertices[i].1) {
            // Difference form keeps a flat cut floor exactly at its surveyed height.
            let [a, b, c] = ids.map(|i| vertices[i].0);
            *height = a + (b - a) * weights[1] + (c - a) * weights[2];
        }
    }
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
    let mut colliders = vec![];
    let mut triangles = cells * cells * 2;
    // Locate cut cells and preserve the existing heightfield away from tunnels.
    let corner_cut: Vec<Option<f64>> = (0..n * n).map(|k| ground.terrain_cut(x0 + (k % n) as f64 * spacing, z0 + (k / n) as f64 * spacing)).collect();
    let grid_cell = 8.0 * eo_data::groundscale::scale();
    let reach = eo_structures::cut_ground::CUT_REACH_CELLS * grid_cell;
    let border = reach + grid_cell;
    let cuts = ground.terrain_cuts_in([x0 - border, z0 - border], [x0 + span + border, z0 + span + border]);
    if !cuts.is_empty() {
        shape_open_cut_ground(&mut h, n, [x0, z0], spacing, grid_cell, reach, &cuts, ground);
    }
    let sections: Vec<_> = cuts.iter().flat_map(|c| c.sections()).collect();
    let mut cut = vec![];
    let mut removed = vec![];
    let mut edges = vec![];
    if !sections.is_empty() || corner_cut.iter().any(Option::is_some) {
        cut = vec![f32::NAN; cells * cells];
        removed = vec![false; cells * cells];
        for iz in 0..cells {
            for ix in 0..cells {
                let cell = iz * cells + ix;
                let k = iz * n + ix;
                let corners = [k, k + 1, k + n, k + n + 1];
                let floor = corners.iter().filter_map(|&i| corner_cut[i]).fold(f64::INFINITY, f64::min);
                if floor.is_finite() {
                    cut[cell] = floor as f32;
                    removed[cell] = cuts.is_empty(); // A provider with only the historical cut mask.
                }
                let (x, z) = (x0 + ix as f64 * spacing, z0 + iz as f64 * spacing);
                let mut boundary: Option<Vec<[[f64; 3]; 3]>> = None;
                let original = || {
                    let p = corners.map(|i| [x0 + (i % n) as f64 * spacing, f64::from(h[i]), z0 + (i / n) as f64 * spacing]);
                    [[p[0], p[2], p[1]], [p[1], p[2], p[3]]]
                };
                for section in &sections {
                    if !section.intersects([x, z], [x + spacing, z + spacing]) { continue; }
                    let mut next = Vec::new();
                    let mut changed = false;
                    for triangle in boundary.take().unwrap_or_else(|| original().to_vec()) {
                        let Some(polys) = section.shape(triangle) else { next.push(triangle); continue };
                        changed = true;
                        for poly in polys {
                            for j in 1..poly.len()-1 {
                                let tri = [poly[0], poly[j], poly[j+1]];
                                let ab = DVec3::from_array(tri[1]) - DVec3::from_array(tri[0]);
                                let ac = DVec3::from_array(tri[2]) - DVec3::from_array(tri[0]);
                                if ab.cross(ac).y > 1e-8 { next.push(tri); }
                            }
                        }
                    }
                    if changed {
                        removed[cell] = true;
                        // Preserve a floor-query fallback inside canopy holes
                        // whose tips may fall between all four lattice corners.
                        cut[cell] = if cut[cell].is_finite() { cut[cell].min(section.floor as f32) } else { section.floor as f32 };
                    }
                    if removed[cell] { boundary = Some(next); }
                }
                if let Some(tris) = boundary { edges.extend(tris); }
            }
        }
    }
    // Parry: rows advance along z, columns along x, column-major data.
    let mut data = vec![0f32; n * n];
    for j in 0..n {
        for i in 0..n {
            data[i + j * n] = h[i * n + j];
        }
    }
    let shape = Vector::new(span as f32, 1.0, span as f32);
    let hf = if cut.is_empty() {
        ColliderBuilder::heightfield(Array2::new(n, n, data), shape) // span
    } else {
        // parry's cells: row i along z, column j along x.
        let mut field = HeightField::new(Array2::new(n, n, data), shape);
        for iz in 0..cells {
            for ix in 0..cells {
                if removed[iz * cells + ix] {
                    field.set_cell_status(iz, ix, HeightFieldCellStatus::CELL_REMOVED);
                    triangles -= 2;
                }
            }
        }
        ColliderBuilder::new(SharedShape::new(field))
    };
    let hf = hf
        .translation(Vector::new((x0 + span / 2.0) as f32, 0.0, (z0 + span / 2.0) as f32))
        .friction(tuning::world::TERRAIN_FRICTION)
        .collision_groups(InteractionGroups::new(layer::WORLD, Group::ALL, InteractionTestMode::And))
        .build();
    colliders.push(hf);
    triangles += edges.len();
    if let Some(collider) = surface_collider(&edges, [x0 + span * 0.5, z0 + span * 0.5], layer::WORLD | layer::STREET) {
        colliders.push(collider);
    }
    let canopy_rings = cuts.iter().filter(|c| c.canopy).map(|c| c.ring.clone()).collect();
    // The floor is part of the same single-precision shape as the heightfield.
    let open_rings = cuts.iter().filter(|c| !c.canopy).map(|c| (c.ring.clone(), f64::from(c.floor as f32))).collect();
    let heights = Arc::new(TileHeights { origin: [x0, z0], spacing, n, h, cut, removed, canopy_rings, open_rings });

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
            let mut min = [f64::MAX; 2];
            let mut max = [f64::MIN; 2];
            for p in ring {
                min[0] = min[0].min(p[0]);
                min[1] = min[1].min(p[1]);
                max[0] = max[0].max(p[0]);
                max[1] = max[1].max(p[1]);
            }
            // landmark access: a deck inside the footprint opens the prism there.
            let spans = if buildings.decks.cuts.is_empty() { vec![(base_y, top_y)] } else { buildings.decks.spans(ax, az, base_y, top_y) };
            for (base_y, top_y) in spans {
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
                prisms.push(Prism { ring: ring.clone(), base_y, top_y, min, max });
            }
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
            triangles += list.len();
            add_surface_colliders(&mut colliders, list, [cx, cz], group);
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
    pub walking_bodies: crate::people_contact::WalkingBodies,
    pub contact_traffic: crate::traffic_contact::ContactTraffic,
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
    /// LOCKSTEP: what the world holds at each fixed step is a function of
    /// where the body has been, never of how fast the cookers ran. `pump`
    /// then only requests and collects cooked tiles; `ensure_step_tiles`
    /// (before every authoritative step) inserts the tiles within
    /// `STEP_RADIUS_M`, waiting for a cook that is late, and drops the tiles
    /// past `DROP_RADIUS_M`, both in key order (with several levels, on the
    /// canonical schedule: `pump_canonical`). A client and the server
    /// stepping the same inputs, or a client replaying them after a
    /// correction, see the same colliders however loaded the machine is.
    pub lockstep: bool,
    /// Longest wait for a late cook in `ensure_step_tiles`, ms, and how many
    /// steps waited at all.
    pub step_wait_ms_max: f64,
    pub step_waits: u64,
    /// Removed tiles go here to be freed on another thread (None when fed: shared shapes are freed in place).
    graveyard: Option<Sender<(Vec<Collider>, Arc<TileHeights>, Arc<Vec<Prism>>)>>,
    /// The ground has walkable surfaces over others (`GroundProvider::multi_level`).
    multi_level: bool,
    /// The moored boats this world floats (harbour.rs; None: no boats).
    pub harbour: Option<crate::harbour::Harbour>,
}

/// A freestanding box whose pose can change without recooking its city tile.
/// Its dimensions and transform are expressed in world metres and radians.
pub struct MovableBox(ColliderHandle);

impl MovableBox {
    pub fn new(world: &mut CollisionWorld, half: DVec3, center: DVec3, yaw: f64) -> Self {
        let shape = ColliderBuilder::cuboid(half.x as f32, half.y as f32, half.z as f32)
            .position(Self::pose(center, yaw))
            .collision_groups(InteractionGroups::new(
                layer::WORLD,
                Group::ALL,
                InteractionTestMode::And,
            ))
            .build();
        Self(world.physics.insert_collider(shape, None))
    }

    fn pose(center: DVec3, yaw: f64) -> Pose {
        Pose::from_parts(
            Vector::new(center.x as f32, center.y as f32, center.z as f32),
            Rotation::from_rotation_y(yaw as f32),
        )
    }

    pub fn move_to(&self, world: &mut CollisionWorld, center: DVec3, yaw: f64) {
        world.physics.colliders[self.0].set_position(Self::pose(center, yaw));
    }
}

fn new_physics(decks: &InstalledDecks) -> PhysicsWorld {
    let mut physics = PhysicsWorld::new();
    physics.gravity = Vector::new(0.0, -tuning::GRAVITY_MPS2 as f32, 0.0);
    physics.integration_parameters.dt = tuning::FIXED_STEP_S as f32;
    physics.integration_parameters.num_solver_iterations = tuning::SOLVER_ITERATIONS;
    // landmark access: every deck, from the start, in the same order on every world.
    for collider in &decks.colliders {
        physics.insert_collider(collider.clone(), None);
    }
    physics
}

/// Cook the faces around a nearby origin to preserve precision. Adjacent
/// triangles on a road or floor share an edge; a chassis contacting that
/// edge should receive the surface normal, from either side of a deck.
fn surface_collider<'a>(faces: impl IntoIterator<Item = &'a [[f64; 3]; 3]>, centre: [f64; 2], group: Group) -> Option<Collider> {
    // Vertices and queries must subtract the same origin that Rapier stores.
    // Rounding only the pose shifts the whole floor away from its drawn edge.
    let centre = centre.map(|coordinate| f64::from(coordinate as f32));
    let faces = faces.into_iter();
    let count = faces.size_hint().0;
    let mut vertices = Vec::with_capacity(count * 3);
    let mut indices = Vec::with_capacity(count);
    for face in faces {
        let i = vertices.len() as u32;
        vertices.extend(face.map(|p| v3([p[0] - centre[0], p[1], p[2] - centre[1]])));
        indices.push([i, i + 1, i + 2]);
    }
    if indices.is_empty() {
        return None;
    }
    let flags = if group.intersects(layer::STREET) {
        rapier3d::parry::shape::TriMeshFlags::FIX_INTERNAL_EDGES_TWO_SIDED
    } else {
        Default::default()
    };
    let mesh = ColliderBuilder::trimesh_with_flags(vertices, indices, flags).ok()?;
    Some(
        mesh.translation(Vector::new(centre[0] as f32, 0.0, centre[1] as f32))
            .friction(tuning::world::TERRAIN_FRICTION)
            .collision_groups(InteractionGroups::new(
                group,
                Group::ALL,
                InteractionTestMode::And,
            ))
            .build(),
    )
}

fn add_surface_colliders(out: &mut Vec<Collider>, faces: &[[[f64; 3]; 3]], centre: [f64; 2], group: Group) {
    if group != layer::WORLD {
        out.extend(surface_collider(faces, centre, group));
        return;
    }
    let (walls, other): (Vec<_>, Vec<_>) = faces.iter().partition(|face| {
        let [a, b, c] = face.map(DVec3::from_array);
        let n = (b - a).cross(c - a);
        // A vertical plane has no projected area in plan. Allow rounding of
        // surveyed vertices to f32 during the geometry bake.
        n.y * n.y <= 1e-10 * n.length_squared()
    });
    out.extend(surface_collider(other, centre, group));
    out.extend(surface_collider(walls, centre, group | layer::VERTICAL_FACE));
}

pub(crate) fn wheel_supports_contact(_: ColliderHandle, collider: &Collider) -> bool {
    !collider.collision_groups().memberships.contains(layer::VERTICAL_FACE)
}

/// The decks' colliders: per part a walkable trimesh (STREET) and a barrier
/// trimesh (WORLD), each about its own centre (f32 holds the city to a millimetre).
pub fn deck_colliders(decks: &Decks) -> Vec<Collider> {
    let mut out = vec![];
    for part in &decks.parts {
        let floors = if part.building_floor { layer::WORLD | layer::STREET } else { layer::STREET };
        for (list, group) in [(&part.walk, floors), (&part.barrier, layer::WORLD)] {
            if list.is_empty() {
                continue;
            }
            let n = (list.len() * 3) as f64;
            let cx = list.iter().flatten().map(|p| p[0]).sum::<f64>() / n;
            let cz = list.iter().flatten().map(|p| p[2]).sum::<f64>() / n;
            add_surface_colliders(&mut out, list, [cx, cz], group);
        }
    }
    out
}

impl CollisionWorld {
    pub fn new(ground: Arc<dyn GroundProvider>, buildings: Arc<BuildingSet>, threads: usize) -> Self {
        let physics = new_physics(&buildings.decks);
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
        let multi_level = ground.multi_level();
        Self { walking_bodies: Default::default(), contact_traffic: Default::default(), physics, ground, buildings, cooker: Some(cooker), feed: None, resident: HashMap::new(), requested: HashSet::new(), pending: vec![], stats: StreamStats::default(), insert_per_pump: 1, lockstep: false, step_wait_ms_max: 0.0, step_waits: 0, graveyard: Some(graveyard), multi_level, harbour: None }
    }

    /// a world whose tiles come from a shared feed (no threads of its own).
    pub fn fed(ground: Arc<dyn GroundProvider>, buildings: Arc<BuildingSet>, feed: Arc<dyn TileFeed>) -> Self {
        let multi_level = ground.multi_level();
        Self { walking_bodies: Default::default(), contact_traffic: Default::default(), physics: new_physics(&buildings.decks), ground, buildings, cooker: None, feed: Some(feed), resident: HashMap::new(), requested: HashSet::new(), pending: vec![], stats: StreamStats::default(), insert_per_pump: 1, lockstep: false, step_wait_ms_max: 0.0, step_waits: 0, graveyard: None, multi_level, harbour: None }
    }

    /// The ground has walkable surfaces over others (decks, `--structures`):
    /// the level queries pick the surface nearest below a body.
    pub fn multi_level(&self) -> bool {
        // Landmark floors are permanently resident. They affect support
        // queries, while only streamed structures need canonical tile order.
        self.multi_level || !self.buildings.decks.parts.is_empty()
    }

    /// One physics step: the boats' buoyancy and mooring first (harbour.rs), then Rapier's.
    pub fn step_physics(&mut self) {
        if !self.walking_bodies.is_empty() {
            let mut walkers = std::mem::take(&mut self.walking_bodies);
            walkers.step(self);
            self.walking_bodies = walkers;
        }
        if !self.contact_traffic.is_empty() {
            let mut traffic = std::mem::take(&mut self.contact_traffic);
            traffic.step(self);
            self.contact_traffic = traffic;
        }
        if let Some(h) = self.harbour.as_mut() {
            h.apply_forces(&mut self.physics, crate::tuning::FIXED_STEP_S);
        }
        self.physics.step();
    }

    /// Float the boats of `fleet` near the streaming centre on `sea`, the sea's clock at `time` (s).
    pub fn set_harbour(&mut self, fleet: std::sync::Arc<crate::harbour::Fleet>, sea: std::sync::Arc<crate::harbour::Sea>, time: f64) {
        self.harbour = Some(crate::harbour::Harbour::new(fleet, sea, time));
    }

    /// Bodies for the boats near `center` (called with the tile streaming).
    pub fn stream_harbour(&mut self, center: DVec3) {
        if let Some(h) = self.harbour.as_mut() {
            h.stream(&mut self.physics, center);
        }
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
    /// With several levels (`multi_level`) the tiles are inserted on the
    /// canonical schedule instead (`pump_canonical`). A lockstep world only
    /// requests and collects here, with one level or several: its tiles go
    /// in and out in `ensure_step_tiles`, on the body's step path.
    pub fn pump(&mut self, center: DVec3) -> f64 {
        if self.multi_level {
            return self.pump_canonical(center);
        }
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
        // Lockstep: the cooked tiles wait in `pending` for `ensure_step_tiles`.
        let budget = if self.lockstep { 0 } else { self.insert_per_pump };
        while inserted < budget {
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
        let far = if self.lockstep { None } else { self.resident.keys().copied().filter(|k| k.distance_to(center.x, center.z) > DROP_RADIUS_M).max_by(|a, b| a.distance_to(center.x, center.z).partial_cmp(&b.distance_to(center.x, center.z)).unwrap()) };
        if let Some(k) = far {
            self.remove_tile(k);
        }
        let keep: HashSet<TileKey> = Self::wanted(center.x, center.z, DROP_RADIUS_M).into_iter().collect();
        self.requested.retain(|k| keep.contains(k));
        if let Some(cooker) = &self.cooker {
            cooker.cancel_except(&keep);
        }
        let resident = &self.resident;
        self.pending.retain(|b| keep.contains(&b.key) && !resident.contains_key(&b.key));
        self.stats.tiles_resident = self.resident.len();
        let ms = t.elapsed().as_secs_f64() * 1e3;
        self.stats.pump_ms_max = self.stats.pump_ms_max.max(ms);
        ms
    }

    /// `pump` with the grade-separated roads: a body can touch concrete of
    /// several tiles at once (a deck's barrier crosses a tile seam), and the
    /// physics solves contacts in the order of the colliders' handles, which
    /// the world hands out in insertion order (reusing the slots of removed
    /// ones); so that a server's fed world and a client's own agree contact
    /// for contact, every world inserts and removes tiles on a schedule that
    /// depends only on the path of `center`, never on when a worker finished
    /// a tile: each pump inserts the nearest due tiles (inside the load
    /// radius, not resident; nearest first, then by key), at most
    /// `insert_per_pump`, waiting for one not yet cooked (tiles are asked
    /// for `PREFETCH_M` beyond the load radius, so they are cooked long
    /// before they are due), and drops the farthest tile past the drop
    /// radius (then by key) (`canonical_move`). A lockstep world makes that
    /// move before each authoritative step instead (`ensure_step_tiles`), at
    /// the body's position, which is where the pump after the step before
    /// makes it in a world that is not lockstep: the two keep the same tiles
    /// in the same order, step for step.
    fn pump_canonical(&mut self, center: DVec3) -> f64 {
        let t = Instant::now();
        let c = (center.x, center.z);
        let ask: Vec<TileKey> = Self::wanted(c.0, c.1, LOAD_RADIUS_M + PREFETCH_M)
            .into_iter()
            .filter(|k| !self.resident.contains_key(k) && !self.pending.iter().any(|b| b.key == *k))
            .collect();
        if let Some(feed) = &self.feed {
            for k in ask {
                match feed.fetch(k, c) {
                    Some(b) => {
                        self.requested.remove(&k);
                        self.pending.push(b);
                    }
                    None => {
                        self.requested.insert(k);
                    }
                }
            }
        } else if let Some(cooker) = &self.cooker {
            let missing: Vec<TileKey> = ask.into_iter().filter(|k| !self.requested.contains(k)).collect();
            if !missing.is_empty() {
                self.requested.extend(missing.iter().copied());
                cooker.request(&missing, c);
            }
            while let Ok(b) = cooker.rx.try_recv() {
                self.requested.remove(&b.key);
                self.pending.push(b);
            }
        }
        if !self.lockstep {
            self.canonical_move(c);
        }
        let keep: HashSet<TileKey> = Self::wanted(c.0, c.1, DROP_RADIUS_M.max(LOAD_RADIUS_M + PREFETCH_M)).into_iter().collect();
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

    /// One move of the canonical schedule about `c` (`pump_canonical`): the
    /// nearest due tiles (inside the load radius, not resident; nearest
    /// first, then by key), at most `insert_per_pump`, inserted (waiting for
    /// one not yet cooked), and the farthest tile past the drop radius (then
    /// by key) dropped. Returns the tiles inserted.
    fn canonical_move(&mut self, c: (f64, f64)) -> usize {
        let mut inserted = 0;
        let mut due: Vec<TileKey> = Self::wanted(c.0, c.1, LOAD_RADIUS_M).into_iter().filter(|k| !self.resident.contains_key(k)).collect();
        due.sort_by(|a, b| canonical(a, b, c));
        for k in due.into_iter().take(self.insert_per_pump) {
            if let Some(b) = self.take_cooked(k, c) {
                self.insert(b);
                inserted += 1;
            }
        }
        let far = self.resident.keys().copied().filter(|k| k.distance_to(c.0, c.1) > DROP_RADIUS_M).max_by(|a, b| canonical(a, b, c));
        if let Some(k) = far {
            self.remove_tile(k);
        }
        inserted
    }

    /// Tile `k` cooked: from the pending ones, else waited for (asked for
    /// now if it never was). None only when no cooker or feed is left.
    fn take_cooked(&mut self, k: TileKey, c: (f64, f64)) -> Option<BuiltTile> {
        if let Some(i) = self.pending.iter().position(|b| b.key == k) {
            self.requested.remove(&k);
            return Some(self.pending.swap_remove(i));
        }
        if let Some(feed) = self.feed.clone() {
            loop {
                if let Some(b) = feed.fetch(k, c) {
                    self.requested.remove(&k);
                    return Some(b);
                }
                std::thread::sleep(std::time::Duration::from_micros(200));
            }
        }
        let cooker = self.cooker.as_ref()?;
        if !self.requested.contains(&k) {
            self.requested.insert(k);
            cooker.request(&[k], c);
        }
        loop {
            let b = cooker.rx.recv().ok()?;
            self.requested.remove(&b.key);
            if b.key == k {
                return Some(b);
            }
            self.pending.push(b);
        }
    }

    fn remove_tile(&mut self, k: TileKey) {
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

    /// Before an authoritative fixed step at `center` (the body): every tile
    /// within `STEP_RADIUS_M` resident, waiting for the cook of any that is
    /// not ready yet (normally none: the stream asked for them 64 m earlier),
    /// inserted in key order; in a lockstep world, the tiles past
    /// `DROP_RADIUS_M` removed, in key order. With several levels a lockstep
    /// world first makes the canonical schedule's move at `center`
    /// (`canonical_move`, as `pump_canonical` makes it in a world that is not
    /// lockstep) and drops on that schedule. No physics step is taken (the
    /// tiles that arrive here lie 95 m or more from the body, so the step
    /// cannot touch them before the next one). Returns the tiles inserted.
    ///
    /// Why: without it the colliders at a step depend on how fast the
    /// worker threads cooked relative to the stepping. A replay that runs
    /// faster than real time (a correction, a test) or a loaded machine
    /// steps the body over tiles that are not resident yet in one run and
    /// are in another, and the two runs part (tens of metres, measured with
    /// a 40 ms cook: `eo-server` `determinism`).
    pub fn ensure_step_tiles(&mut self, center: DVec3) -> usize {
        let (x, z) = (center.x, center.z);
        let mut inserted = 0;
        if self.lockstep && self.multi_level {
            inserted += self.canonical_move((x, z));
        }
        let want = Self::wanted(x, z, STEP_RADIUS_M); // row by row: a fixed key order
        let missing: Vec<TileKey> = want.into_iter().filter(|k| !self.resident.contains_key(k)).collect();
        if !missing.is_empty() {
            let t = Instant::now();
            let mut got: Vec<Option<BuiltTile>> = missing
                .iter()
                .map(|k| {
                    let i = self.pending.iter().position(|b| b.key == *k)?;
                    Some(self.pending.swap_remove(i))
                })
                .collect();
            let mut waited = false;
            let mut asked = Instant::now() - std::time::Duration::from_secs(10);
            while got.iter().any(Option::is_none) {
                if let Some(feed) = self.feed.clone() {
                    for (k, g) in missing.iter().zip(got.iter_mut()) {
                        if g.is_none() {
                            *g = feed.fetch(*k, (x, z));
                        }
                    }
                    if got.iter().any(Option::is_none) {
                        waited = true;
                        std::thread::sleep(std::time::Duration::from_micros(200));
                    }
                } else if let Some(cooker) = &self.cooker {
                    // (Re)ask for what is neither cooked nor known to be queued; once a second
                    // ask again for all of it (a request the stream cancelled meanwhile).
                    if asked.elapsed().as_secs_f64() > 1.0 {
                        let ask: Vec<TileKey> = missing.iter().zip(&got).filter(|(_, g)| g.is_none()).map(|(k, _)| *k).collect();
                        self.requested.extend(ask.iter().copied());
                        cooker.request(&ask, (x, z));
                        asked = Instant::now();
                    }
                    waited = true;
                    match cooker.rx.recv_timeout(std::time::Duration::from_millis(250)) {
                        Ok(b) => match missing.iter().position(|k| *k == b.key) {
                            Some(i) if got[i].is_none() => got[i] = Some(b),
                            _ => self.pending.push(b),
                        },
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                        Err(_) => break,
                    }
                } else {
                    break;
                }
            }
            for b in got.into_iter().flatten() {
                self.requested.remove(&b.key);
                self.insert(b);
                inserted += 1;
            }
            if waited {
                self.step_waits += 1;
                self.step_wait_ms_max = self.step_wait_ms_max.max(t.elapsed().as_secs_f64() * 1e3);
            }
        }
        if self.lockstep && !self.multi_level {
            let mut far: Vec<TileKey> = self.resident.keys().copied().filter(|k| k.distance_to(x, z) > DROP_RADIUS_M).collect();
            far.sort_unstable();
            for k in far {
                self.remove_tile(k);
            }
        }
        self.stats.tiles_resident = self.resident.len();
        inserted
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

    /// Block until every tile within `radius` of `center` is resident (setup
    /// and tests). With several levels (`multi_level`) they are inserted in
    /// the canonical order (nearest first, then by key) whatever order they
    /// were cooked in.
    pub fn load_blocking(&mut self, center: DVec3, radius: f64) {
        let want = Self::wanted(center.x, center.z, radius);
        if self.multi_level && (self.feed.is_some() || self.cooker.is_some()) {
            let c = (center.x, center.z);
            let mut due: Vec<TileKey> = want.iter().copied().filter(|k| !self.resident.contains_key(k)).collect();
            due.sort_by(|a, b| canonical(a, b, c));
            // (asked for all at once, so they cook side by side.)
            if let Some(cooker) = &self.cooker {
                let missing: Vec<TileKey> = due.iter().copied().filter(|k| !self.requested.contains(k) && !self.pending.iter().any(|b| b.key == *k)).collect();
                self.requested.extend(missing.iter().copied());
                cooker.request(&missing, c);
            }
            for k in due {
                if let Some(b) = self.take_cooked(k, c) {
                    self.insert(b);
                }
            }
            self.stats.tiles_resident = self.resident.len();
            self.physics.detect_collisions(&(), &());
            return;
        }
        // Every tile cooked first, then inserted in `want`'s fixed order: the
        // order the cooks finish in varies from run to run, and the colliders'
        // handles (and so the solver's contact order) follow the insertion order.
        let mut got: HashMap<TileKey, BuiltTile> = HashMap::new();
        if let Some(feed) = self.feed.clone() {
            // Use already prefetched tiles before requesting more from the feed.
            for i in (0..self.pending.len()).rev() {
                if want.contains(&self.pending[i].key) {
                    let tile = self.pending.swap_remove(i);
                    self.requested.remove(&tile.key);
                    got.insert(tile.key, tile);
                }
            }
            loop {
                for k in &want {
                    if !self.resident.contains_key(k) && !got.contains_key(k) {
                        if let Some(b) = feed.fetch(*k, (center.x, center.z)) {
                            got.insert(*k, b);
                        }
                    }
                }
                if want.iter().all(|k| self.resident.contains_key(k) || got.contains_key(k)) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        } else if self.cooker.is_some() {
            let missing: Vec<TileKey> = want.iter().copied().filter(|k| !self.resident.contains_key(k) && !self.requested.contains(k)).collect();
            self.requested.extend(missing.iter().copied());
            self.cooker.as_ref().unwrap().request(&missing, (center.x, center.z));
            loop {
                for i in (0..self.pending.len()).rev() {
                    if want.contains(&self.pending[i].key) {
                        let b = self.pending.swap_remove(i);
                        self.requested.remove(&b.key);
                        if !self.resident.contains_key(&b.key) {
                            got.insert(b.key, b);
                        }
                    }
                }
                if want.iter().all(|k| self.resident.contains_key(k) || got.contains_key(k)) {
                    break;
                }
                match self.cooker.as_ref().unwrap().rx.recv() {
                    Ok(b) => self.pending.push(b),
                    Err(_) => break,
                }
            }
        } else {
            return;
        }
        for k in &want {
            if let Some(b) = got.remove(k) {
                self.insert(b);
            }
        }
        self.stats.tiles_resident = self.resident.len();
        // Make the ground queryable immediately. Loading a tile can happen
        // between game ticks, including while a body is in free flight.
        self.physics.detect_collisions(&(), &());
    }

    /// A joining peer can prepare the spawn off-thread, then insert all of
    /// its shapes in the same order as a predicting client.
    pub fn tiles_ready(&self, center: DVec3, radius: f64) -> bool {
        Self::wanted(center.x, center.z, radius).iter().all(|k|
            self.resident.contains_key(k) || self.pending.iter().any(|tile| tile.key == *k))
    }

    pub fn is_resident(&self, x: f64, z: f64) -> bool {
        self.resident.contains_key(&TileKey::of(x, z))
    }

    /// Keep the tiles for a sparse group of authoritative contact bodies.
    /// These bodies use the shared feed and step on the server's clock.
    pub fn sync_traffic_tiles(&mut self, footprints: &[(DVec3, f64)]) {
        let Some(feed) = self.feed.clone() else { return };
        let mut wanted = std::collections::BTreeSet::new();
        for &(centre, radius) in footprints {
            let lo = TileKey::of(centre.x - radius, centre.z - radius);
            let hi = TileKey::of(centre.x + radius, centre.z + radius);
            for x in lo.x..=hi.x {
                for z in lo.z..=hi.z {
                    wanted.insert(TileKey { x, z });
                }
            }
        }
        let mut far: Vec<_> = self.resident.keys().filter(|key| !wanted.contains(key)).copied().collect();
        far.sort_unstable();
        let mut changed = !far.is_empty();
        for key in far {
            self.remove_tile(key);
        }
        for key in wanted {
            if !self.resident.contains_key(&key) && let Some(tile) = feed.fetch(key, key.origin()) {
                self.insert(tile);
                changed = true;
            }
        }
        self.stats.tiles_resident = self.resident.len();
        if changed {
            self.physics.detect_collisions(&(), &());
        }
    }

    /// The swept footprint must have collision before the actor can enter it.
    /// Usually the render-frame pump has already prefetched these tiles.
    pub fn prepare_support(&mut self, start: DVec3, end: DVec3, radius: f64) {
        let low = TileKey::of(start.x.min(end.x) - radius, start.z.min(end.z) - radius);
        let high = TileKey::of(start.x.max(end.x) + radius, start.z.max(end.z) + radius);
        for z in low.z..=high.z {
            for x in low.x..=high.x {
                let key = TileKey { x, z };
                if !self.resident.contains_key(&key) {
                    let (ox, oz) = key.origin();
                    let at = DVec3::new(ox + TILE_SIZE_M * 0.5, end.y, oz + TILE_SIZE_M * 0.5);
                    if self.ensure_step_tiles(at) > 0 {
                        self.physics.detect_collisions(&(), &());
                    }
                }
            }
        }
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
        self.surface_at_floor(x, z, floor)
    }

    fn surface_at_floor(&self, x: f64, z: f64, floor: f64) -> f64 {
        match self.raycast(DVec3::new(x, floor + 3.0, z), DVec3::NEG_Y, 3.5, layer::STREET, None) {
            Some(h) => h.point.y.max(floor),
            None => floor,
        }
    }

    /// A map arrival starts on a road where one is drawn. A graded tunnel
    /// approach can be well above the underlying terrain. Existing walkers
    /// and parked cars with a known floor use level-relative probes instead.
    pub fn arrival_y(&self, x: f64, z: f64) -> f64 {
        let local = self.surface_y(x, z);
        if !self.multi_level() || local > self.floor_y(x, z) + tuning::capsule::CONTROLLER_OFFSET_M {
            return local;
        }
        let Some(road) = self.ground.arrival_road_y(x, z, local) else { return local };
        if road <= local { return local; }
        let margin = tuning::kerb::GROUND_CONTACT_TOLERANCE_M;
        self.raycast(DVec3::new(x, road + margin, z), DVec3::NEG_Y, 2.0 * margin, layer::STREET, None)
            .map_or(local, |hit| hit.point.y)
    }

    /// The walkable surface nearest below `from_y` at (x, z) (the body's
    /// height plus a margin, `LEVEL_REACH_M`): a deck where the point is on
    /// one, the street under it where it is under one. Walkable is the
    /// street layer (street, kerb tops, deck tops: the first hit of a ray down
    /// from `from_y`) and the ground heightfield (the floor, when it is under
    /// `from_y`), whichever is higher; never a building roof or the
    /// structures' concrete (a barrier top, or the inside of a girder a
    /// reference height may start in). Where the ground stands over `from_y`
    /// (under the terrain: a future tunnel) the street layer under it, else
    /// the floor. A measured street can cross above the terrain even in
    /// surface-street mode. Probe from the caller's level on that crossing.
    pub fn surface_below(&self, x: f64, z: f64, from_y: f64) -> f64 {
        let floor = self.floor_y(x, z);
        if !self.multi_level() {
            // An elevated surveyed crossing is still walkable with structures
            // disabled. Ordinary streets use the same ray origin as surface_y:
            // probing them from flight height loses precision in the collider.
            let street_limit = floor + 3.0;
            if from_y > street_limit
                && let Some(y) = self.street_below(x, z, from_y, floor)
                && y > street_limit
            {
                return y;
            }
            return self.surface_at_floor(x, z, floor);
        }
        match self.street_below(x, z, from_y, floor) {
            Some(y) if floor <= from_y => y.max(floor),
            Some(y) => y,
            None => floor,
        }
    }

    /// Follow the walker's level to a nearby parking spot. Resolving the
    /// intervening slope keeps an overhead floor out of the next probe.
    pub fn surface_beside(&self, feet: DVec3, x: f64, z: f64) -> f64 {
        if !self.multi_level() {
            let floor = self.floor_y(x, z);
            if feet.y + LEVEL_REACH_M <= floor + 3.0 {
                return self.surface_at_floor(x, z, floor);
            }
        }
        let dx = x - feet.x;
        let dz = z - feet.z;
        let distance = dx.hypot(dz);
        if distance > LOAD_RADIUS_M {
            // A remote spawn has no resident path to trace.
            return self.surface_below(x, z, feet.y + LEVEL_REACH_M);
        }
        let steps = (distance / tuning::capsule::CENTER_FROM_FEET_M).ceil().max(1.0) as usize;
        let rise = tuning::capsule::MAX_STEP_HEIGHT_M + distance / steps as f64
            * tuning::capsule::MAX_SLOPE_DEGREES.to_radians().tan();
        let mut y = feet.y;
        for i in 1..=steps {
            let t = i as f64 / steps as f64;
            y = self.surface_below(feet.x + t * dx, feet.z + t * dz, y + rise);
        }
        y
    }

    /// The height a body moving to (x, z) is never let below (the on-foot
    /// controller's floor clamp, `containmentFloorY`): the floor where it is
    /// under `from_y` (the capsule's centre); where the ground stands over
    /// `from_y` (a body under the terrain, in a future tunnel: the old clamp
    /// would push it up through the ground) the street layer's surface under
    /// it, or no clamp (`NEG_INFINITY`). On a deck the floor is the ground
    /// far below and the deck's own collider holds the body; under a deck
    /// the floor is the street's ground as before. One level: `floor_y`.
    pub fn floor_under(&self, x: f64, z: f64, from_y: f64) -> f64 {
        let floor = self.floor_y(x, z);
        if !self.multi_level || floor <= from_y {
            return floor;
        }
        self.street_below(x, z, from_y, floor).unwrap_or(f64::NEG_INFINITY)
    }

    /// The first street-layer hit (street, kerb tops, deck tops) of a ray
    /// down from `from_y` to just under the floor (or `LEVEL_REACH_M` under
    /// `from_y` where the floor is over it).
    fn street_below(&self, x: f64, z: f64, from_y: f64, floor: f64) -> Option<f64> {
        let to = floor.min(from_y - LEVEL_REACH_M) - 0.5;
        self.raycast(DVec3::new(x, from_y, z), DVec3::NEG_Y, from_y - to, layer::STREET, None).map(|h| h.point.y)
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
        let (h, hit) = if dir == DVec3::NEG_Y && filter.intersects(layer::WORLD | layer::STREET) {
            // Rounding a city coordinate to f32 before subtracting the tile
            // origin can move a ray across a road seam or a cut terrain edge.
            // Find candidates in world space and cast in each shape's frame.
            let bottom = (origin.y - max) as f32;
            let bounds = Aabb::new(
                Vector::new(ray.origin.x.next_down(), bottom.next_down(), ray.origin.z.next_down()),
                Vector::new(ray.origin.x.next_up(), ray.origin.y.next_up(), ray.origin.z.next_up()),
            );
            self.physics.intersect_aabb_conservative(bounds, f).filter_map(|(handle, collider)| {
                let pose = collider.position();
                let offset = origin - DVec3::new(pose.translation.x.into(), pose.translation.y.into(), pose.translation.z.into());
                let inverse = pose.rotation.inverse();
                let local = Ray::new(inverse * Vector::new(offset.x as f32, offset.y as f32, offset.z as f32), inverse * ray.dir);
                let mut hit = collider.shape().cast_local_ray_and_get_normal(&local, max as f32, true)?;
                hit.normal = pose.rotation * hit.normal;
                Some((handle, hit))
            }).min_by(|(ha, a), (hb, b)| a.time_of_impact.total_cmp(&b.time_of_impact)
                .then_with(|| ha.into_raw_parts().cmp(&hb.into_raw_parts())))
        } else {
            self.physics.cast_ray_and_get_normal(&ray, max as f32, true, f)
        }?;
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
