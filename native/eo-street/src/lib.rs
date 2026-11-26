//! Earth Online native: the browser game's street surface, ported.
//!
//! * `options`  the street surface options the stream tiles compile with,
//!              read from the JS export; caMetric values re-derived from the
//!              cited California table and tested equal (no literals here).
//! * `city`     the loader's street contract (`city.segments` /
//!              `city.intersections` of sf-data.js loadSfData), exported for
//!              the whole city by the earlier browser version's exporter.
//! * `plan`     buildStreetscapePlan's geometric passes (runs, nodes, trims,
//!              corner fillets and chords, crossing ramps, carriageway yield).
//! * `paint`    the lane marking plan and the crossing family.
//! * `mesh`     the street surface geometry per 128 m tile (near) and a cheap
//!              far level, plus the drawn-surface query buildings stand on.
//! * `grounding` the street-front floor and the foundation skirt of buildings
//!              (street-front-grade.js / yard-floor-rule.js /
//!              building-foundations.js), and the grounding measurement.
//! * `joins`    where the grade-separated roads meet the paved network
//!              (plain data a host hands in; off by default).

pub mod city;
pub mod evidence;
pub mod ground;
pub mod grounding;
pub mod joins;
pub mod material;
pub mod mesh;
pub mod options;
pub mod bikeway; // (`--bikeways`) kerbside bike lanes as the city's bike network builds them
pub mod paint;
pub mod plan;
pub mod road_openings;
mod arterial_corner;
mod pavement_overlap;
mod paving;
mod driveway;
mod roadbed;
mod continuous;
pub mod seams;

pub use city::StreetCity;
pub use mesh::TileMesh;
pub use options::StreetOptions;
pub use plan::Plan;

use eo_data::ElevationGrid;
use mesh::EntryGeom;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub const STREET_PORT_VERSION: &str = "eo-street-port-v1";
/// Digest of this crate's sources and data (build.rs).
pub const STREET_SOURCE_HASH: &str = env!("STREET_SOURCE_HASH");

/// how far past a street's paved corridor the ground carpet's
/// vertices are still held at the street's carpet level (a rendering margin,
/// not a street dimension), times the ground-plan scale. hills: one whole
/// 8 m elevation cell (was half a cell): a carpet triangle that reaches over
/// the street has its far vertex up to a cell past the corridor's edge, and
/// on the real (steeper) grades that uphill vertex lifted the carpet through
/// the street (ground over the street, eo-street `surface`: 390 -> 39 probes
/// at plan 1.25, heights 1.25).
pub const TERRAIN_CAP_MARGIN_M: f64 = 8.0;

/// Street tiles share the kit's 128 m grid.
pub const STREET_TILE_M: f64 = 128.0;

pub fn tile_key(x: f64, z: f64) -> (i32, i32) {
    ((x / STREET_TILE_M).floor() as i32, (z / STREET_TILE_M).floor() as i32)
}

/// What a triangle of the street surface is (STREET_SURFACE_V2_LAYERS).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Layer {
    Carriageway = 0,
    CurbFace = 1,
    CurbTop = 2,
    Sidewalk = 3,
    Ramp = 4,
    Verge = 5,
    Marking = 6,
    /// The ground the streets stand in (`ground::ground_overlay`, `--street-ground`).
    Ground = 7,
}

impl Layer {
    /// Something a body or a floor probe stands on (STREET_COLLISION_ROLES less the faces).
    pub fn walkable(self) -> bool {
        matches!(self, Layer::Carriageway | Layer::CurbTop | Layer::Sidewalk | Layer::Ramp | Layer::Verge | Layer::Ground)
    }
}

/// 32-bit FNV-1a (`hash32` in street-surface-v2.js).
pub fn hash32(s: &str) -> u32 {
    let mut h: u32 = 0x811c9dc5;
    for b in s.encode_utf16() {
        h ^= b as u32;
        h = h.wrapping_mul(0x01000193);
    }
    h
}

/// The terrain the renderer draws: the elevation grid's vertices at cell
/// centres, triangulated as eo-render/scene.rs does, lifted to the JS ground
/// carpet (`openGroundLiftM`).
#[derive(Clone)]
pub struct Terrain {
    pub grid: Arc<ElevationGrid>,
    pub carpet_lift: f64,
}

impl Terrain {
    pub fn ground(&self, x: f64, z: f64) -> f64 {
        self.grid.ground(x, z)
    }
    /// The drawn terrain mesh's height at (x, z) (the full 8 m level).
    pub fn mesh_y(&self, x: f64, z: f64) -> f64 {
        let m = &self.grid.meta;
        let cell = m.cell_size as f32 as f64;
        let (ox, oz) = (m.origin_x as f32 as f64, m.origin_z as f32 as f64);
        let fx = (x - ox) / cell - 0.5;
        let fz = (z - oz) / cell - 0.5;
        let (cx, cz) = (fx.floor(), fz.floor());
        let (u, v) = (fx - cx, fz - cz);
        let vy = |c: f64, r: f64| {
            let wx = (ox as f32 + (c as f32 + 0.5) * cell as f32) as f64;
            let wz = (oz as f32 + (r as f32 + 0.5) * cell as f32) as f64;
            self.grid.ground(wx, wz) as f32 as f64
        };
        let (y0, y1, y2, y3) = (vy(cx, cz), vy(cx + 1.0, cz), vy(cx, cz + 1.0), vy(cx + 1.0, cz + 1.0));
        if u + v <= 1.0 { y0 + (y1 - y0) * u + (y2 - y0) * v } else { y3 + (y2 - y3) * (1.0 - u) + (y1 - y3) * (1.0 - v) }
    }
}

/// Footprints, for the frontage march and the measurement.
pub struct BuildingIndex {
    pub rings: Vec<Vec<[f64; 2]>>,
    cell: f64,
    buckets: HashMap<(i32, i32), Vec<u32>>,
}

pub fn point_in_ring(x: f64, z: f64, ring: &[[f64; 2]]) -> bool {
    let n = ring.len();
    let mut inside = false;
    let mut j = n.wrapping_sub(1);
    for i in 0..n {
        let (a, b) = (ring[i], ring[j]);
        if (a[1] > z) != (b[1] > z) && x < (b[0] - a[0]) * (z - a[1]) / (b[1] - a[1]) + a[0] {
            inside = !inside;
        }
        j = i;
    }
    inside
}

impl BuildingIndex {
    pub fn new(rings: Vec<Vec<[f64; 2]>>) -> Self {
        let cell = 32.0;
        let mut buckets: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, r) in rings.iter().enumerate() {
            if r.len() < 3 {
                continue;
            }
            let (mut mn, mut mx) = ([f64::MAX; 2], [f64::MIN; 2]);
            for p in r {
                mn = [mn[0].min(p[0]), mn[1].min(p[1])];
                mx = [mx[0].max(p[0]), mx[1].max(p[1])];
            }
            for gz in (mn[1] / cell).floor() as i32..=(mx[1] / cell).floor() as i32 {
                for gx in (mn[0] / cell).floor() as i32..=(mx[0] / cell).floor() as i32 {
                    buckets.entry((gx, gz)).or_default().push(i as u32);
                }
            }
        }
        Self { rings, cell, buckets }
    }
    pub fn at(&self, x: f64, z: f64) -> Option<u32> {
        let k = ((x / self.cell).floor() as i32, (z / self.cell).floor() as i32);
        self.buckets.get(&k)?.iter().copied().find(|&i| point_in_ring(x, z, &self.rings[i as usize]))
    }
    pub fn contains(&self, x: f64, z: f64) -> bool {
        self.at(x, z).is_some()
    }

    /// Unique building indices whose bucket bounds reach within `margin` of `(x, z)`.
    fn nearby(&self, x: f64, z: f64, margin: f64, out: &mut Vec<u32>) {
        out.clear();
        let m = margin.max(0.0);
        let gx0 = ((x - m) / self.cell).floor() as i32;
        let gx1 = ((x + m) / self.cell).floor() as i32;
        let gz0 = ((z - m) / self.cell).floor() as i32;
        let gz1 = ((z + m) / self.cell).floor() as i32;
        for gz in gz0..=gz1 {
            for gx in gx0..=gx1 {
                let Some(list) = self.buckets.get(&(gx, gz)) else { continue };
                for &i in list {
                    if !out.contains(&i) {
                        out.push(i);
                    }
                }
            }
        }
    }

    /// If `(x, z)` lies inside a building footprint or closer than `margin` to
    /// any of its walls, return a nearby point pushed outside to distance
    /// `margin` from the wall (None when already clear).
    pub fn push_outside(&self, x: f64, z: f64, margin: f64) -> Option<[f64; 2]> {
        let margin = margin.max(0.0);
        let mut cands = Vec::new();
        self.nearby(x, z, margin, &mut cands);
        if cands.is_empty() {
            return None;
        }
        let ring_sign = |ring: &[[f64; 2]]| -> f64 {
            let mut a = 0.0;
            for i in 0..ring.len() {
                let (p, q) = (ring[i], ring[(i + 1) % ring.len()]);
                a += p[0] * q[1] - q[0] * p[1];
            }
            if a >= 0.0 { 1.0 } else { -1.0 }
        };
        let (mut px, mut pz) = (x, z);
        let mut moved = false;
        for _ in 0..3 {
            if moved {
                self.nearby(px, pz, margin, &mut cands);
                if cands.is_empty() {
                    break;
                }
            }
            // First: if strictly inside a building, exit across its closest edge
            // whose cleared point does not land inside a neighbouring building
            // (a shared party wall).
            if let Some(&inside_id) = cands.iter().find(|&&i| point_in_ring(px, pz, &self.rings[i as usize])) {
                let ring = &self.rings[inside_id as usize];
                let sign = ring_sign(ring);
                let mut best_clear: Option<(f64, [f64; 2])> = None;
                let mut best_any: Option<(f64, [f64; 2])> = None;
                for k in 0..ring.len() {
                    let (a, b) = (ring[k], ring[(k + 1) % ring.len()]);
                    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
                    let l2 = dx * dx + dz * dz;
                    if l2 < 1e-12 {
                        continue;
                    }
                    let l = l2.sqrt();
                    let t = (((px - a[0]) * dx + (pz - a[1]) * dz) / l2).clamp(0.0, 1.0);
                    let q = [a[0] + dx * t, a[1] + dz * t];
                    let d = (q[0] - px).hypot(q[1] - pz);
                    let edge_out = if sign > 0.0 { [dz / l, -dx / l] } else { [-dz / l, dx / l] };
                    let mut out = if d > 1e-6 { [(q[0] - px) / d, (q[1] - pz) / d] } else { edge_out };
                    let mut cand = [q[0] + out[0] * margin, q[1] + out[1] * margin];
                    if point_in_ring(cand[0], cand[1], ring) {
                        out = edge_out;
                        cand = [q[0] + out[0] * margin, q[1] + out[1] * margin];
                        if point_in_ring(cand[0], cand[1], ring) {
                            cand = [q[0] - out[0] * margin, q[1] - out[1] * margin];
                        }
                    }
                    if best_any.is_none_or(|b| d < b.0) {
                        best_any = Some((d, cand));
                    }
                    if !self.contains(cand[0], cand[1]) && best_clear.is_none_or(|b| d < b.0) {
                        best_clear = Some((d, cand));
                    }
                }
                if let Some((_, cand)) = best_clear.or(best_any) {
                    px = cand[0];
                    pz = cand[1];
                    moved = true;
                    continue;
                }
                break;
            }
            // Outside all buildings: keep at least `margin` clear of the closest wall.
            if !(margin > 0.0) {
                break;
            }
            let mut closest: Option<(f64, [f64; 2], [f64; 2])> = None;
            for &bi in &cands {
                let ring = &self.rings[bi as usize];
                let sign = ring_sign(ring);
                for k in 0..ring.len() {
                    let (a, b) = (ring[k], ring[(k + 1) % ring.len()]);
                    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
                    let l2 = dx * dx + dz * dz;
                    if l2 < 1e-12 {
                        continue;
                    }
                    let t = (((px - a[0]) * dx + (pz - a[1]) * dz) / l2).clamp(0.0, 1.0);
                    let q = [a[0] + dx * t, a[1] + dz * t];
                    let d = (px - q[0]).hypot(pz - q[1]);
                    if d < margin && closest.is_none_or(|c| d < c.0) {
                        let l = l2.sqrt();
                        let out = if d > 1e-6 {
                            [(px - q[0]) / d, (pz - q[1]) / d]
                        } else if sign > 0.0 {
                            [dz / l, -dx / l]
                        } else {
                            [-dz / l, dx / l]
                        };
                        closest = Some((d, q, out));
                    }
                }
            }
            let Some((_, q, out)) = closest else { break };
            let cand = [q[0] + out[0] * margin, q[1] + out[1] * margin];
            if self.contains(cand[0], cand[1]) {
                break;
            }
            px = cand[0];
            pz = cand[1];
            moved = true;
        }
        moved.then_some([px, pz])
    }
}

/// What a tile draws: the spans and nodes anchored in it.
#[derive(Clone, Debug, Default)]
pub struct TileRefs {
    pub spans: Vec<(u32, u32)>,
    pub nodes: Vec<u32>,
}

/// The walkable triangles near one tile, gridded for point queries.
pub struct SurfaceTile {
    tris: Vec<[f32; 9]>,
    /// each triangle's layer (the surface underfoot).
    layers: Vec<Layer>,
    origin: [f64; 2],
    cell: f64,
    n: usize,
    grid: Vec<Vec<u32>>,
}

impl SurfaceTile {
    fn build(meshes: &[&TileMesh], key: (i32, i32)) -> Self {
        let cell = 4.0;
        let margin = 24.0;
        let origin = [key.0 as f64 * STREET_TILE_M - margin, key.1 as f64 * STREET_TILE_M - margin];
        let n = ((STREET_TILE_M + 2.0 * margin) / cell) as usize;
        let mut grid = vec![vec![]; n * n];
        let mut tris = vec![];
        let mut layers = vec![]; // audio
        for m in meshes {
            for (t, layer) in m.layers.iter().enumerate() {
                if !layer.walkable() {
                    continue;
                }
                let [a, b, c] = [m.indices[3 * t], m.indices[3 * t + 1], m.indices[3 * t + 2]].map(|i| m.positions[i as usize]);
                let tri = [a[0], a[1], a[2], b[0], b[1], b[2], c[0], c[1], c[2]];
                let (x0, x1) = (a[0].min(b[0]).min(c[0]) as f64, a[0].max(b[0]).max(c[0]) as f64);
                let (z0, z1) = (a[2].min(b[2]).min(c[2]) as f64, a[2].max(b[2]).max(c[2]) as f64);
                let gi0 = ((x0 - origin[0]) / cell).floor().max(0.0) as usize;
                let gi1 = (((x1 - origin[0]) / cell).floor() as i64).min(n as i64 - 1);
                let gj0 = ((z0 - origin[1]) / cell).floor().max(0.0) as usize;
                let gj1 = (((z1 - origin[1]) / cell).floor() as i64).min(n as i64 - 1);
                if gi1 < 0 || gj1 < 0 || gi0 >= n || gj0 >= n {
                    continue;
                }
                let id = tris.len() as u32;
                tris.push(tri);
                layers.push(*layer); // audio
                for j in gj0..=gj1 as usize {
                    for i in gi0..=gi1 as usize {
                        grid[j * n + i].push(id);
                    }
                }
            }
        }
        Self { tris, layers, origin, cell, n, grid }
    }

    /// Walkable triangles at a point, in mesh order. An upper road and a
    /// pedestrian surface below it can both cover the same plan coordinates.
    fn hits_at(&self, x: f64, z: f64) -> impl Iterator<Item = (Layer, f64)> + '_ {
        let i = ((x - self.origin[0]) / self.cell).floor() as isize;
        let j = ((z - self.origin[1]) / self.cell).floor() as isize;
        let cells = 0..self.n as isize;
        let ids: &[u32] = if cells.contains(&i) && cells.contains(&j) {
            &self.grid[j as usize * self.n + i as usize]
        } else {
            &[]
        };
        ids.iter().filter_map(move |&id| {
            let p = &self.tris[id as usize];
            let (ax, az, bx, bz, cx, cz) = (p[0] as f64, p[2] as f64, p[3] as f64, p[5] as f64, p[6] as f64, p[8] as f64);
            let det = (bz - cz) * (ax - cx) + (cx - bx) * (az - cz);
            if det.abs() < 1e-12 { return None; }
            let wa = ((bz - cz) * (x - cx) + (cx - bx) * (z - cz)) / det;
            let wb = ((cz - az) * (x - cx) + (ax - cx) * (z - cz)) / det;
            let wc = 1.0 - wa - wb;
            (wa >= -1e-6 && wb >= -1e-6 && wc >= -1e-6)
                .then(|| (self.layers[id as usize], wa * p[1] as f64 + wb * p[4] as f64 + wc * p[7] as f64))
        })
    }

    /// The layer of the highest walkable triangle, with the first face winning a shared edge.
    pub fn top_layer(&self, x: f64, z: f64) -> Option<Layer> {
        self.hits_at(x, z).reduce(|a, b| if b.1 > a.1 { b } else { a }).map(|(layer, _)| layer)
    }

    /// The highest walkable triangle over (x, z), if any.
    pub fn height(&self, x: f64, z: f64) -> Option<f64> {
        self.hits_at(x, z).map(|(_, y)| y).reduce(f64::max)
    }

    /// Every walkable triangle's height over (x, z).
    pub fn heights(&self, x: f64, z: f64) -> Vec<f64> {
        self.heights_on(x, z, None)
    }

    fn heights_on(&self, x: f64, z: f64, layer: Option<Layer>) -> Vec<f64> {
        self.hits_at(x, z).filter_map(|(kind, y)| layer.is_none_or(|l| l == kind).then_some(y)).collect()
    }

    /// Every walkable triangle over (x, z): its height and layer.
    pub fn layered_heights(&self, x: f64, z: f64) -> Vec<(f64, Layer)> {
        let mut out = vec![];
        let i = ((x - self.origin[0]) / self.cell).floor();
        let j = ((z - self.origin[1]) / self.cell).floor();
        if i < 0.0 || j < 0.0 || i >= self.n as f64 || j >= self.n as f64 {
            return out;
        }
        for &t in &self.grid[j as usize * self.n + i as usize] {
            let p = &self.tris[t as usize];
            let (ax, az, bx, bz, cx, cz) = (p[0] as f64, p[2] as f64, p[3] as f64, p[5] as f64, p[6] as f64, p[8] as f64);
            let det = (bz - cz) * (ax - cx) + (cx - bx) * (az - cz);
            if det.abs() < 1e-12 {
                continue;
            }
            let wa = ((bz - cz) * (x - cx) + (cx - bx) * (z - cz)) / det;
            let wb = ((cz - az) * (x - cx) + (ax - cx) * (z - cz)) / det;
            let wc = 1.0 - wa - wb;
            let eps = -1e-6;
            if wa < eps || wb < eps || wc < eps {
                continue;
            }
            out.push((wa * p[1] as f64 + wb * p[4] as f64 + wc * p[7] as f64, self.layers[t as usize]));
        }
        out
    }
}

pub struct StreetWorld {
    pub o: StreetOptions,
    pub city: StreetCity,
    pub plan: Plan,
    pub geoms: Vec<EntryGeom>,
    pub lines: Vec<Vec<paint::Line>>,
    pub tiles: HashMap<(i32, i32), TileRefs>,
    pub terrain: Terrain,
    pub buildings: BuildingIndex,
    /// Centreline pieces of every carriageway: (entry, a, b, half), gridded.
    road_cells: HashMap<(i32, i32), Vec<u32>>,
    road_pieces: Vec<(u32, [f64; 2], [f64; 2], f64)>,
    node_cells: HashMap<(i32, i32), Vec<u32>>,
    corner_bounds: Vec<[[f64; 2]; 2]>,
    /// `ramp-running-slope-max` (drawnSurfaceAt's walking slope).
    pub ramp_running_slope_max: f64,
    pub bank_step_m: f64,
    pub bank_reach_m: f64,
    surface_cache: Mutex<HashMap<(i32, i32), Arc<SurfaceTile>>>,
    mesh_cache: Mutex<HashMap<(i32, i32), Arc<TileMesh>>>,
    road_openings: Arc<road_openings::RoadOpenings>,
    driveway_openings: HashMap<usize, road_openings::RoadOpenings>,
    divided_carriageways: HashMap<String, road_openings::RoadOpenings>,
    pavement_overlap: pavement_overlap::PavementOverlap,
    apron: mesh::Apron,
    surveyed_junction: Option<mesh::surveyed_junction::Surface>,
    roadbed_caps: HashMap<(i32,i32),f32>,
    paving_cache: Mutex<HashMap<(i32,i32),Arc<paving::PavingTile>>>,
    paving_owners: HashMap<(i32,i32),Vec<(i32,i32)>>,
    /// `drawn_mesh`: the paving and its street ground.
    drawn_cache: Mutex<HashMap<(i32, i32), Arc<TileMesh>>>,
    /// (`seams`) each run's [start, end] joint: the run it meets end to end
    /// there with no junction, and whether at that run's start.
    pub(crate) joints: Vec<[Option<(usize, bool)>; 2]>,
    /// `ground::overlay_grid`: each street tile's street ground on its lattice.
    overlay_cache: Mutex<HashMap<(i32, i32), Option<Arc<ground::OverlayGrid>>>>,
    pub build_ms: f64,
    /// Only with `StreetOptions::structures`: whether each structure end's
    /// rise and cut (`joins::StructureEnd::rise`, `cut`) are drawn: the ramp
    /// (cone) back at the street datum before any junction (`check_rises`).
    rises_ok: Vec<[bool; 2]>,
    /// the shore's ground rule over the carpet (eo-water
    /// `ShoreField::press_beach`: a beach laid on its foreshore), set once the
    /// shore field is loaded, so the drawn ground, the collision ground and
    /// everything that stands on the carpet read one height.
    shore_press: std::sync::OnceLock<ShorePress>,
    /// the grade-separated roads' terrain cuts (a tunnel's approach cut and
    /// portals: eo-structures `terrain_cut_at`, the cut's floor), set once
    /// they are built: the street ground (`ground`) leaves them open, as the
    /// terrain and the collision heightfield do.
    ground_cut: std::sync::OnceLock<GroundCut>,
    /// where the drawn terrain is the water's (the shoreline rings' press, a
    /// beach's), set once the water is loaded: no street ground there.
    ground_water: std::sync::OnceLock<GroundWater>,
    /// the junction pads (`plan::PadIndex`), for the corner returns of a
    /// junction in a cluster (`plan::walk_yield_strict`).
    pub(crate) pads: plan::PadIndex,
    /// `ground`: each junction's pad outline (the convex hull of its approach
    /// mouths' corners; empty under three approaches), built on first use.
    pub(crate) node_hulls: std::sync::OnceLock<Vec<Vec<[f64; 2]>>>,
    /// (`--street-tapers`, `StreetOptions::tapers`) each run's taper at its
    /// start and end: (how much narrower it is drawn at the joint, m; over
    /// how long, m), (0, 0) where it keeps its width (`half_at`).
    pub tapers: Vec<[(f64, f64); 2]>,
    /// (`--bikeways`, `StreetOptions::bikeways`) each run's kerbside bike
    /// lane as the city's network builds it (None: drawn as before), and the
    /// markings' dimensions from the standards table.
    pub bikeways: Vec<Option<bikeway::BikeStyle>>,
    pub bike_dims: Option<bikeway::BikeDims>,
    /// (`--midblock-crossings`, `StreetOptions::midblock`) each run's crossings
    /// between junctions, by arc length.
    pub midblock: Vec<Vec<MidCrossing>>,
}

/// (`--midblock-crossings`) A signalised, beaconed or flashing crossing
/// between junctions on a run: its centre's arc length, the crosswalk's depth
/// along the run, the device (1 signal, 2 beacon, 3 flasher).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MidCrossing {
    pub s: f64,
    pub depth: f64,
    pub kind: u8,
}

/// (x, z, carpet height) -> the carpet height at the shore.
pub type ShorePress = Arc<dyn Fn(f64, f64, f64) -> f64 + Send + Sync>;

/// (x, z) -> the floor of the terrain cut holding it, if any (eo-structures).
pub type GroundCut = Arc<dyn Fn(f64, f64) -> Option<f64> + Send + Sync>;

/// (x, z) -> whether the water's press moves the drawn terrain there (eo-water).
pub type GroundWater = Arc<dyn Fn(f64, f64) -> bool + Send + Sync>;

/// (`--midblock-crossings`) A mid-block device is on a run when it stands
/// within the run's half width and this far past it (m on the surveyed plan:
/// the footway where a signal pole or beacon stands). INFERRED.
pub const MIDBLOCK_REACH_M: f64 = 6.0;

/// The index of the centreline piece of run `e` starting at point `a`.
fn seg_index(e: &plan::Entry, a: [f64; 2]) -> usize {
    e.points.iter().position(|p| p[0] == a[0] && p[1] == a[1]).unwrap_or(0).min(e.cum.len().saturating_sub(2))
}

impl StreetWorld {
    pub fn traffic_plan(&self) -> eo_data::sim_source::TrafficPlan {
        use eo_data::sim_source::{TrafficJunction, TrafficPlan};
        let sections = plan::traffic_sections_with_lines(&self.city, &self.plan, &self.o, &self.lines, |ei, start, end| {
            self.traffic_surface(ei, start, end)
        });
        let junctions = self.plan.nodes.iter().filter_map(|node| {
            let mut ways: Vec<_> = node.approaches.iter().filter_map(|approach| {
                let entry = &self.plan.entries[approach.entry];
                let piece = if approach.at_start { entry.pieces.first()? } else { entry.pieces.last()? };
                self.city.segments[*piece].street_id.as_deref()?.strip_prefix("sf-street-").map(str::to_owned)
            }).collect();
            ways.sort_unstable();
            ways.dedup();
            if ways.is_empty() {
                return None;
            }
            let (center, rim, triangles) = self.traffic_junction_pad(node);
            (rim.len() >= 3).then_some(TrafficJunction { ways, center, rim, triangles })
        }).collect();
        let surveyed_routes = self.o.surveyed_junction.as_ref().and_then(|s| s.traffic_routes.clone()).and_then(|mut routes| {
            for line in routes.approaches.iter_mut().map(|a| &mut a.line).chain(routes.circulation.iter_mut()) {
                let sampled: Option<Vec<_>> = line.points.iter().map(|&[x, z]| {
                    self.carriageway_near(x, z, self.ground(x, z) + self.o.road_lift)
                }).collect();
                let Some(heights) = sampled else {
                    eprintln!("surveyed traffic routes: a lane leaves the drawn carriageway");
                    return None;
                };
                line.heights = heights;
            }
            Some(routes)
        });
        TrafficPlan { sections, junctions, surveyed_routes }
    }

    fn traffic_surface(&self, ei: usize, start: f64, end: f64) -> (Vec<[f64; 3]>, Vec<[usize; 2]>) {
        let entry = &self.plan.entries[ei];
        let geom = &self.geoms[ei];
        let stations = &geom.stations;
        if stations.len() < 2 || end <= start {
            return Default::default();
        }
        let first = stations.partition_point(|st| st.s <= start);
        let last = stations.partition_point(|st| st.s < end);
        let samples: Vec<_> = std::iter::once(start)
            .chain(stations[first..last].iter().map(|st| st.s))
            .chain(std::iter::once(end))
            .filter_map(|s| {
                let f = entry.frame(s, false);
                // At a trimmed mouth use the datum of this road's own mouth.
                let y = self.datum_at_station(geom, s.clamp(stations[0].s, stations[stations.len() - 1].s))?;
                Some((s, [f.x, self.o.cross_section_y(y, 0.0, entry.half), f.z]))
            })
            .collect();
        let mut spans: Vec<[usize; 2]> = Vec::new();
        for (i, pair) in samples.windows(2).enumerate() {
            let middle = pair[0].0.midpoint(pair[1].0);
            let next = stations.partition_point(|st| st.s <= middle);
            if next == 0 || !geom.keep.get(next - 1).copied().unwrap_or(false) {
                continue;
            }
            if let Some(last) = spans.last_mut().filter(|span| span[1] == i) {
                last[1] = i + 1;
            } else {
                spans.push([i, i + 1]);
            }
        }
        (samples.into_iter().map(|(_, point)| point).collect(), spans)
    }

    pub fn build(o: StreetOptions, city: StreetCity, terrain: Terrain, buildings: BuildingIndex) -> Self {
        let plan = plan::build_plan(&city, &o);
        Self::from_plan(o, city, plan, terrain, buildings)
    }

    /// The street world over a plan already built (the plan needs no terrain or buildings).
    pub fn from_plan(o: StreetOptions, city: StreetCity, mut plan: Plan, terrain: Terrain, buildings: BuildingIndex) -> Self {
        plan::connect_landings(&mut plan.entries, &o, &terrain);
        if o.pavement_repairs {
            mesh::Apron::fit_curb(&mut plan, &city);
        }
        let t0 = std::time::Instant::now();
        if std::env::var("EO_STREET_TRACE").is_ok() {
            eprintln!("  street plan done: {:.0} ms", t0.elapsed().as_secs_f64() * 1e3);
        }
        // (a structure leg, `plan::Entry::structure_leg`, is not drawn: no
        // paint, no carriageway for the corridor queries, no stations.)
        let lines = paint::plan_markings(&city, &plan, &o);
        let mut road_cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        let mut road_pieces = vec![];
        for (ei, e) in plan.entries.iter().enumerate() {
            if e.structure_leg.is_some() {
                continue;
            }
            for w in e.points.windows(2) {
                let id = road_pieces.len() as u32;
                road_pieces.push((ei as u32, w[0], w[1], e.half));
                // out to the terrain cap's margin too (`terrain_cap` reads a
                // vertex's own cell, and a vertex up to the margin past the corridor
                // must find the run: it was missed in the next 32 m cell).
                let r = e.half + e.walks[0].max(e.walks[1]) + TERRAIN_CAP_MARGIN_M * eo_data::groundscale::scale();
                let (x0, x1) = (w[0][0].min(w[1][0]) - r, w[0][0].max(w[1][0]) + r);
                let (z0, z1) = (w[0][1].min(w[1][1]) - r, w[0][1].max(w[1][1]) + r);
                for gz in (z0 / 32.0).floor() as i32..=(z1 / 32.0).floor() as i32 {
                    for gx in (x0 / 32.0).floor() as i32..=(x1 / 32.0).floor() as i32 {
                        road_cells.entry((gx, gz)).or_default().push(id);
                    }
                }
            }
        }
        let mut node_cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (ni, n) in plan.nodes.iter().enumerate() {
            let r = n.approaches.iter().map(|a| a.trim + a.half).fold(0.0, f64::max);
            let p = n.position;
            for gz in ((p[1] - r) / 32.0).floor() as i32..=((p[1] + r) / 32.0).floor() as i32 {
                for gx in ((p[0] - r) / 32.0).floor() as i32..=((p[0] + r) / 32.0).floor() as i32 {
                    node_cells.entry((gx, gz)).or_default().push(ni as u32);
                }
            }
        }
        let corner_bounds = plan.nodes.iter().map(|n| {
            let (mut lo, mut hi) = (n.position, n.position);
            for st in n.paths.iter().flat_map(|path| &path.stations) {
                let side = st.walk.max(o.curb_top_width) * st.scale;
                for p in [[st.x, st.z], [st.x + st.out[0]*side, st.z + st.out[1]*side]] {
                    for k in 0..2 { lo[k]=lo[k].min(p[k]); hi[k]=hi[k].max(p[k]); }
                }
            }
            [lo,hi]
        }).collect();
        let (ramp_running_slope_max, bank_step_m, bank_reach_m) = (o.ramp_running_slope_max, o.bank_step_m, o.bank_reach_m);
        let mut w = StreetWorld {
            o,
            city,
            plan,
            geoms: vec![],
            lines,
            tiles: HashMap::new(),
            terrain,
            buildings,
            road_cells,
            road_pieces,
            node_cells,
            corner_bounds,
            ramp_running_slope_max,
            bank_step_m,
            bank_reach_m,
            surface_cache: Mutex::new(HashMap::new()),
            mesh_cache: Mutex::new(HashMap::new()),
            road_openings: Arc::default(),
            driveway_openings: HashMap::new(),
            divided_carriageways: HashMap::new(),
            pavement_overlap: Default::default(),
            apron: Default::default(),
            surveyed_junction: None,
            roadbed_caps: Default::default(),
            paving_cache: Default::default(),
            paving_owners: Default::default(),
            drawn_cache: Mutex::new(HashMap::new()),
            overlay_cache: Mutex::new(HashMap::new()),
            build_ms: 0.0,
            rises_ok: vec![],
            shore_press: std::sync::OnceLock::new(), // sea
            ground_cut: std::sync::OnceLock::new(),
            ground_water: std::sync::OnceLock::new(),
            pads: plan::PadIndex::default(),
            node_hulls: std::sync::OnceLock::new(),
            joints: vec![],
            tapers: vec![],
            bikeways: vec![],
            bike_dims: None,
            midblock: vec![],
        };
        if plan::walk_yield_strict() {
            w.pads = plan::PadIndex::new(&w.plan.nodes, &w.plan.entries);
        }
        if w.o.tapers {
            w.tapers = w.plan_tapers();
        }
        if w.o.bikeways {
            w.plan_bikeways();
        }
        if w.o.midblock {
            w.plan_midblock();
        }
        w.rises_ok = w.check_rises();
        w.geoms = (0..w.plan.entries.len()).map(|ei| w.entry_geom(ei)).collect();
        if w.o.pavement_repairs { continuous::join(&w.plan, &w.city, &mut w.geoms); }
        if seams::road_seams() {
            w.mitre_joints();
            w.apply_seams();
            w.seam_walks();
        }
        let mut tiles: HashMap<(i32, i32), TileRefs> = HashMap::new();
        for (ei, g) in w.geoms.iter().enumerate() {
            for i in 0..g.stations.len().saturating_sub(1) {
                if !g.keep[i] {
                    continue;
                }
                let (a, b) = (g.stations[i], g.stations[i + 1]);
                tiles.entry(tile_key((a.x + b.x) / 2.0, (a.z + b.z) / 2.0)).or_default().spans.push((ei as u32, i as u32));
            }
        }
        for (ni, n) in w.plan.nodes.iter().enumerate() {
            tiles.entry(tile_key(n.position[0], n.position[1])).or_default().nodes.push(ni as u32);
        }
        w.tiles = tiles;
        if w.o.pavement_repairs {
            w.divided_carriageways = w.index_divided_carriageways();
            w.fit_driveways();
            w.pavement_overlap = pavement_overlap::PavementOverlap::build(&w);
            w.apron = mesh::Apron::fit(&w);
            w.paving_owners = w.index_paving_owners();
        }
        if let Some(survey) = &w.o.surveyed_junction {
            let surface = mesh::surveyed_junction::Surface::build(&w, survey);
            w.tiles.entry(surface.owner).or_default();
            w.surveyed_junction = Some(surface);
            w.mesh_cache.get_mut().unwrap().clear();
            w.surface_cache.get_mut().unwrap().clear();
            w.paving_cache.get_mut().unwrap().clear();
        }
        if w.o.pavement_repairs || w.surveyed_junction.is_some() {
            w.roadbed_caps = roadbed::fit(&w);
        }
        w.build_ms = t0.elapsed().as_secs_f64() * 1e3;
        w
    }

    /// Regrade an existing join plan. Heights do not change its horizontal
    /// stations, yield spans, crossings, paint layout or tile ownership.
    pub fn update_structure_landings(&mut self, landings: &[(plan::P2, joins::Rise)]) {
        if self.o.landings == landings { return; }
        self.o.landings = landings.to_vec();
        let prior: Vec<_> = self.plan.entries.iter_mut().map(|e| std::mem::take(&mut e.landings)).collect();
        plan::connect_landings(&mut self.plan.entries, &self.o, &self.terrain);
        for (i, old) in prior.into_iter().enumerate() {
            if old != self.plan.entries[i].landings {
                self.geoms[i] = self.entry_geom(i);
            }
        }
        self.refresh_graded_pavement();
    }

    /// Refit surfaces that depend on the road datums after the joined
    /// structures and their street approach have been graded together.
    pub fn refresh_graded_pavement(&mut self) {
        if self.o.pavement_repairs {
            continuous::join(&self.plan, &self.city, &mut self.geoms);
            self.divided_carriageways = self.index_divided_carriageways();
            self.fit_driveways();
            self.pavement_overlap = pavement_overlap::PavementOverlap::build(self);
            self.apron = mesh::Apron::fit(self);
            self.paving_cache.get_mut().unwrap().clear();
        }
        if self.o.pavement_repairs || self.surveyed_junction.is_some() {
            self.roadbed_caps = roadbed::fit(self);
        }
        self.surface_cache.get_mut().unwrap().clear();
        self.mesh_cache.get_mut().unwrap().clear();
    }

    pub fn ground(&self, x: f64, z: f64) -> f64 {
        self.terrain.ground(x, z)
    }

    /// (`--street-tapers`) The half-width run `ei` is drawn at, at arc length
    /// `s`: its own, less what it gives up toward a narrower run it meets end
    /// to end (`tapers`), eased over the taper (a smooth step: no kink at
    /// either end of it). Without tapers, `e.half` everywhere.
    pub fn half_at(&self, ei: usize, s: f64) -> f64 {
        let e = &self.plan.entries[ei];
        let Some(t) = self.tapers.get(ei) else { return e.half };
        let mut h = e.half;
        for (end, &(delta, len)) in t.iter().enumerate() {
            if !(delta > 0.0 && len > 0.0) {
                continue;
            }
            let d = if end == 0 { s - e.s0() } else { e.s1() - s };
            let x = (d / len).clamp(0.0, 1.0);
            h -= delta * (1.0 - x * x * (3.0 - 2.0 * x));
        }
        h
    }

    /// (`--bikeways`) Each run with a kerbside bike lane joined to the city's
    /// bike network by its middle and heading (the network's line within 14 m
    /// and 25 degrees, in the surveyed plan), and its markings adjusted: no
    /// green where the network records no green paint, no buffer line where
    /// an island fills the buffer, a second solid line bounding a marked
    /// buffer (Section 9E.07 para 14).
    fn plan_bikeways(&mut self) {
        let ca = eo_data::DataPaths::discover().ok().and_then(|p| eo_data::CaStandards::load(&p).ok());
        let net = eo_data::DataPaths::discover().ok().and_then(|p| eo_data::bikeways::Bikeways::load(&p).ok());
        let (Some(ca), Some(net)) = (ca, net) else {
            eprintln!("bikeways: the standards table or sf-bikeways.json.gz did not load; bike lanes as before");
            return;
        };
        match bikeway::BikeDims::load(&ca, &self.o) {
            Ok(d) => self.bike_dims = Some(d),
            Err(e) => {
                eprintln!("bikeways: {e:#}; bike lanes as before");
                return;
            }
        }
        let s = eo_data::groundscale::scale();
        let mut out = vec![None; self.plan.entries.len()];
        for (ei, e) in self.plan.entries.iter().enumerate() {
            if e.structure_leg.is_some() || !self.lines[ei].iter().any(|l| l.role == "cycle-lane") {
                continue;
            }
            let (s0, s1) = (e.s0(), e.s1());
            if s1 - s0 < 2.0 {
                continue;
            }
            let (a, b) = (e.frame(s0, false), e.frame(s1, false));
            let mid = e.frame((s0 + s1) / 2.0, false);
            let l = ((b.x - a.x).powi(2) + (b.z - a.z).powi(2)).sqrt().max(1e-9);
            let found = net.segment_at([mid.x / s, mid.z / s], [(b.x - a.x) / l, (b.z - a.z) / l], 14.0, 25.0);
            out[ei] = found.and_then(|(rec, _)| bikeway::BikeStyle::from_record(rec));
        }
        for (ei, st) in out.iter().enumerate() {
            let Some(st) = st else { continue };
            let lines = &mut self.lines[ei];
            let mut add = vec![];
            let edge_w = self.o.edge_line_width;
            for l in lines.iter_mut() {
                if l.role == "cycle-lane" && l.paint == paint::Paint::White {
                    // a lane between the traffic and the parking, drawn before as one white band:
                    // its fill green or bare, bounded by solid white bike lane lines (Section 9E.04)
                    l.paint = if st.green { paint::Paint::Green } else { paint::Paint::None };
                    for side in [-1.0, 1.0] {
                        add.push(paint::Line { role: "cycle-lane-edge", u: l.u + side * (l.width / 2.0 + edge_w / 2.0), width: edge_w, dashed: false, paint: paint::Paint::White, direction: 0, side: 0 });
                    }
                }
                if l.role == "cycle-lane" && l.paint == paint::Paint::Green && !st.green {
                    l.paint = paint::Paint::None;
                }
                if l.role == "cycle-buffer-edge" && st.island {
                    l.paint = paint::Paint::None;
                }
            }
            if !st.island {
                // the buffer's inner bounding line (its outer one is `cycle-buffer-edge`)
                for l in lines.iter() {
                    if l.role == "cycle-buffer" && l.width > 0.2 {
                        let edge = lines.iter().filter(|x| x.role == "cycle-buffer-edge").min_by(|x, y| (x.u - l.u).abs().total_cmp(&(y.u - l.u).abs()));
                        if let Some(edge) = edge {
                            let away = (l.u - edge.u).signum();
                            add.push(paint::Line { role: "cycle-buffer-inner", u: l.u + away * (l.width / 2.0 - edge.width / 2.0), width: edge.width, dashed: false, paint: paint::Paint::White, direction: 0, side: 0 });
                        }
                    }
                }
            }
            lines.extend(add);
        }
        let n = out.iter().filter(|x| x.is_some()).count();
        eprintln!("bikeways: {n} runs with a kerbside bike lane joined to the bike network");
        self.bikeways = out;
    }

    /// (`--midblock-crossings`) Each mid-block device of the city's survey
    /// (eo-data `street_survey`, DataSF Traffic Signals' MIDBLOCK rows) on the
    /// run whose carriageway it stands on or beside (within its half width and
    /// `MIDBLOCK_REACH_M` on the scaled plan), clear of the run's ends by a
    /// crosswalk and a ramp's flare: a crossing there, the crosswalk as deep
    /// as a junction's of the run's class (`crossing_band_depth`), and a kerb
    /// ramp reserved on each kerb with a footway wide enough (as
    /// `planCrossingRamps` reserves a junction's).
    fn plan_midblock(&mut self) {
        let Some(survey) = eo_data::DataPaths::discover().ok().and_then(|p| eo_data::street_survey::StreetSurvey::load(&p).ok()) else {
            eprintln!("midblock crossings: sf-street-survey.json.gz did not load; none drawn");
            return;
        };
        let s = eo_data::groundscale::scale();
        let o = self.o.clone();
        let mut out: Vec<Vec<MidCrossing>> = vec![vec![]; self.plan.entries.len()];
        let mut placed = 0;
        for mb in &survey.midblock {
            let p = [mb.at[0] * s, mb.at[1] * s];
            let key = ((p[0] / 32.0).floor() as i32, (p[1] / 32.0).floor() as i32);
            let mut best: Option<(usize, f64, f64)> = None; // (entry, station, |lateral|)
            for &id in self.road_cells.get(&key).map(|v| v.as_slice()).unwrap_or(&[]) {
                let (ei, a, b, half) = self.road_pieces[id as usize];
                let e = &self.plan.entries[ei as usize];
                let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
                let l2 = dx * dx + dz * dz;
                if l2 < 1e-9 {
                    continue;
                }
                let t = ((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / l2;
                if !(0.0..=1.0).contains(&t) {
                    continue;
                }
                let lat = ((p[0] - a[0]) * -dz + (p[1] - a[1]) * dx).abs() / l2.sqrt();
                if lat > half + MIDBLOCK_REACH_M * s {
                    continue;
                }
                let i = seg_index(e, a);
                let station = e.cum[i] + t * l2.sqrt();
                if best.is_none_or(|bb| lat < bb.2) {
                    best = Some((ei as usize, station, lat));
                }
            }
            let Some((ei, station, _)) = best else { continue };
            let e = &mut self.plan.entries[ei];
            let depth = plan::crossing_band_depth(e.class_rank, &o);
            let margin = depth / 2.0 + o.crosswalk_clearance + o.stop_bar_clearance + o.stop_bar_depth + o.ramp_flare_run;
            if station < e.s0() + margin || station > e.s1() - margin {
                continue;
            }
            let flat = o.ramp_width.clamp(plan::RAMP_MIN_CLEAR_WIDTH, depth.max(plan::RAMP_MIN_CLEAR_WIDTH));
            let (lo, hi) = (station - flat / 2.0, station + flat / 2.0);
            let reach = o.curb_top_width + o.ramp_run;
            for k in 0..2 {
                if e.walks[k] > reach + 0.05 {
                    e.kerb_ramps[k].push(plan::RampWin { lo, hi, flare_lo: lo - o.ramp_flare_run, flare_hi: hi + o.ramp_flare_run, node: usize::MAX, at_start: true });
                }
            }
            out[ei].push(MidCrossing { s: station, depth, kind: mb.kind });
            placed += 1;
        }
        eprintln!("midblock crossings: {placed} of {} surveyed devices on a run", survey.midblock.len());
        self.midblock = out;
    }

    /// (`--midblock-crossings`) The stretches of run `ei` its mid-block
    /// crossings keep the lane lines off: the crosswalk and the limit lines
    /// either side of it.
    pub fn midblock_gaps(&self, ei: usize) -> Vec<[f64; 2]> {
        let o = &self.o;
        self.midblock
            .get(ei)
            .map(|v| v.iter().map(|c| {
                let r = c.depth / 2.0 + o.stop_bar_clearance + o.stop_bar_depth + 0.1;
                [c.s - r, c.s + r]
            }).collect())
            .unwrap_or_default()
    }

    /// `[lo, hi]` less run `ei`'s mid-block gaps.
    pub fn cut_midblock(&self, ei: usize, lo: f64, hi: f64) -> Vec<[f64; 2]> {
        let mut out = vec![[lo, hi]];
        for g in self.midblock_gaps(ei) {
            out = out
                .into_iter()
                .flat_map(|[a, b]| {
                    if g[1] <= a || g[0] >= b {
                        vec![[a, b]]
                    } else {
                        let mut v = vec![];
                        if g[0] > a {
                            v.push([a, g[0]]);
                        }
                        if g[1] < b {
                            v.push([g[1], b]);
                        }
                        v
                    }
                })
                .collect();
        }
        out
    }

    /// (`--bikeways`) A run's kerbside bike lane style, None where drawn as before.
    pub fn bikeway(&self, ei: usize) -> Option<&bikeway::BikeStyle> {
        self.bikeways.get(ei).and_then(|b| b.as_ref())
    }

    /// (`--street-tapers`) How far a run's tapers reach in from its start and
    /// its end (m; 0 without one): the kerb there is no place to park.
    pub fn taper_reach(&self, ei: usize) -> [f64; 2] {
        self.tapers.get(ei).map(|t| [t[0].1, t[1].1]).unwrap_or([0.0, 0.0])
    }

    /// (`--street-tapers`) The plain joints (two runs end to end at a point
    /// no junction claims, neither trimmed there) where the runs' widths
    /// differ: the wider narrows to the other's width over the lane-reduction
    /// taper, L = W v^2 / 11.991 m below 20.1 m/s (W the offset, v the prima
    /// facie speed of a business or residence district), never under the
    /// urban minimum (`caMetric('lane-reduction-taper')`,
    /// `caMetric('speed-prima-facie-business-residence')`), and no longer
    /// than 0.45 of the run, so a short run keeps room for both ends.
    fn plan_tapers(&self) -> Vec<[(f64, f64); 2]> {
        let mut out = vec![[(0.0, 0.0); 2]; self.plan.entries.len()];
        let Some(ca) = eo_data::DataPaths::discover().ok().and_then(|p| eo_data::CaStandards::load(&p).ok()) else {
            eprintln!("street tapers: the street-standards table did not load; no tapers");
            return out;
        };
        let (Ok(v), Ok(div), Ok(hi_from), Ok(hi_k), Ok(min_l)) = (
            ca.metric("speed-prima-facie-business-residence"),
            ca.metric_key("lane-reduction-taper", "lowSpeedDivisor"),
            ca.metric_key("lane-reduction-taper", "highSpeedFrom"),
            ca.metric_key("lane-reduction-taper", "highSpeedFactor"),
            ca.metric_key("lane-reduction-taper", "minUrban"),
        ) else {
            eprintln!("street tapers: a lane-reduction-taper or prima facie speed value is missing from the standards table; no tapers");
            return out;
        };
        let taper_len = |w: f64| if v < hi_from { w * v * v / div } else { w * v * hi_k }.max(min_l);
        let mut ends: HashMap<(i64, i64), Vec<(usize, usize)>> = HashMap::new();
        for (ei, e) in self.plan.entries.iter().enumerate() {
            if e.structure_leg.is_some() || e.points.len() < 2 {
                continue;
            }
            for (end, p) in [(0usize, e.points[0]), (1, e.points[e.points.len() - 1])] {
                ends.entry(((p[0] * 4.0).round() as i64, (p[1] * 4.0).round() as i64)).or_default().push((ei, end));
            }
        }
        let plain = |ei: usize, end: usize| {
            let e = &self.plan.entries[ei];
            let (app, trim) = if end == 0 { (e.approach_start, e.trim_start) } else { (e.approach_end, e.trim_end) };
            app.is_none() && trim < 0.01 && e.structure_ends[end].is_none()
        };
        for v in ends.values() {
            if v.len() != 2 || !plain(v[0].0, v[0].1) || !plain(v[1].0, v[1].1) {
                continue;
            }
            let (a, b) = (&self.plan.entries[v[0].0], &self.plan.entries[v[1].0]);
            let ((wi, wend), narrow) = if a.half > b.half { (v[0], b.half) } else { (v[1], a.half) };
            let w = &self.plan.entries[wi];
            let delta = w.half - narrow;
            if delta < 0.05 {
                continue;
            }
            let room = 0.45 * (w.s1() - w.s0()).max(0.0);
            let len = taper_len(delta).min(room);
            if len > 0.5 {
                out[wi][wend] = (delta, len);
            }
        }
        out
    }
    /// The ground carpet's vertex at a terrain grid vertex (world position as
    /// eo-render/scene.rs computes it, in f32): the terrain lifted by
    /// `openGroundLiftM`, held under any street there (`terrain_cap`).
    pub fn carpet_vertex(&self, wx: f32, wz: f32) -> f32 {
        let ground=self.base_carpet_vertex(wx,wz);
        if self.roadbed_caps.is_empty() { return ground; }
        let m=&self.terrain.grid.meta;
        let index=|v:f32, origin:f64| ((v-origin as f32)/m.cell_size as f32-0.5).round() as i32;
        self.roadbed_caps.get(&(index(wx,m.origin_x),index(wz,m.origin_z))).map_or(ground,|&cap| ground.min(cap))
    }

    fn base_carpet_vertex(&self, wx:f32, wz:f32)->f32 {
        let y = self.terrain.grid.ground(wx as f64, wz as f64) as f32 + self.terrain.carpet_lift as f32;
        let y = match self.terrain_cap(wx as f64, wz as f64) {
            Some(c) => y.min(c as f32),
            None => y,
        };
        // the shore's rule (a beach on its foreshore), when installed.
        match self.shore_press.get() {
            Some(p) => p(wx as f64, wz as f64, y as f64) as f32,
            None => y,
        }
    }

    /// install the shore's ground rule (once; later calls are ignored).
    /// Tiles cooked before it are dropped so every tile reads the same ground.
    pub fn set_shore_press(&self, press: ShorePress) {
        if self.shore_press.set(press).is_ok() {
            self.paving_cache.lock().unwrap().clear();
            self.surface_cache.lock().unwrap().clear();
            self.mesh_cache.lock().unwrap().clear();
            self.drawn_cache.lock().unwrap().clear();
            self.overlay_cache.lock().unwrap().clear();
        }
    }

    /// whether the shore's ground rule is installed.
    pub fn has_shore_press(&self) -> bool {
        self.shore_press.get().is_some()
    }

    /// The ground carpet as the renderer draws it (the 8 m level).
    pub fn carpet_y(&self, x: f64, z: f64) -> f64 {
        let m = &self.terrain.grid.meta;
        let cell = m.cell_size as f32;
        let (ox, oz) = (m.origin_x as f32, m.origin_z as f32);
        let fx = (x - ox as f64) / cell as f64 - 0.5;
        let fz = (z - oz as f64) / cell as f64 - 0.5;
        let (cx, cz) = (fx.floor(), fz.floor());
        let (u, v) = (fx - cx, fz - cz);
        let vy = |c: f64, r: f64| self.carpet_vertex(ox + (c as f32 + 0.5) * cell, oz + (r as f32 + 0.5) * cell) as f64;
        let (y0, y1, y2, y3) = (vy(cx, cz), vy(cx + 1.0, cz), vy(cx, cz + 1.0), vy(cx + 1.0, cz + 1.0));
        if u + v <= 1.0 { y0 + (y1 - y0) * u + (y2 - y0) * v } else { y3 + (y2 - y3) * (1.0 - u) + (y1 - y3) * (1.0 - v) }
    }

    /// Is (x, z) on another run's carriageway band (|lateral| < half)?
    pub fn on_other_carriageway(&self, x: f64, z: f64, own: usize) -> bool {
        let k = ((x / 32.0).floor() as i32, (z / 32.0).floor() as i32);
        let Some(list) = self.road_cells.get(&k) else { return false };
        for &id in list {
            let (ei, a, b, half) = self.road_pieces[id as usize];
            if ei as usize == own {
                continue;
            }
            let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
            let len2 = dx * dx + dz * dz;
            if len2 < 1e-12 {
                continue;
            }
            let t = ((x - a[0]) * dx + (z - a[1]) * dz) / len2;
            if !(0.0..=1.0).contains(&t) {
                continue;
            }
            let d = ((x - a[0] - dx * t).powi(2) + (z - a[1] - dz * t).powi(2)).sqrt();
            if d < half {
                return true;
            }
        }
        false
    }

    /// The highest the ground carpet may be drawn at (x, z) without showing
    /// through the street: inside a run's paved corridor (carriageway plus its
    /// wider footway, between its trims) or a junction pad, the carpet as it
    /// stands at the street's own datum (`datum - roadLift + openGroundLift`,
    /// ground-datum.js: the carpet sits that far under the road it borders).
    /// None away from streets.
    pub fn terrain_cap(&self, x: f64, z: f64) -> Option<f64> {
        let k = ((x / 32.0).floor() as i32, (z / 32.0).floor() as i32);
        let mut cap: Option<f64> = None;
        let lift = self.terrain.carpet_lift - self.o.road_lift;
        if let Some(list) = self.road_cells.get(&k) {
            for &id in list {
                let (ei, a, b, _) = self.road_pieces[id as usize];
                let e = &self.plan.entries[ei as usize];
                let reach = e.half + e.walks[0].max(e.walks[1]);
                let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
                let len2 = dx * dx + dz * dz;
                if len2 < 1e-12 {
                    continue;
                }
                let raw = ((x - a[0]) * dx + (z - a[1]) * dz) / len2;
                let Some(t) = self.piece_t(ei as usize, a, raw) else { continue };
                let (px, pz) = (a[0] + dx * t, a[1] + dz * t);
                // half a terrain cell past the paved corridor too, so
                // an 8 m cell straddling the corridor edge on a cross slope
                // cannot interpolate the carpet up through the street.
                // the cell is 8 x s m on a scaled plan; so is its half.
                // a whole cell now (`TERRAIN_CAP_MARGIN_M`); round the
                // outside of a bend (`piece_t`, `seams`) half a cell: a whole
                // one held a disc of open ground down round every bend of a
                // road in a cut.
                let margin = if (0.0..=1.0).contains(&raw) { 1.0 } else { 0.5 } * TERRAIN_CAP_MARGIN_M * eo_data::groundscale::scale();
                if ((x - px).powi(2) + (z - pz).powi(2)).sqrt() > reach + margin {
                    continue;
                }
                let base = self.run_ground(ei as usize, a, t, len2, px, pz) + self.o.road_lift;
                let mut y = self.surveyed_datum(e, [px, pz], base).unwrap_or(base).min(base) + lift;
                // a stub lowered into a tunnel's approach cut (`joins`): the
                // carpet held under the street as drawn there.
                if e.structure_ends.iter().any(Option::is_some) || !e.structure_ramps.is_empty() {
                    if let Some(d) = self.geoms.get(ei as usize).and_then(|g| self.datum_at_station(g, e.cum[seg_index(e, a)] + t * (len2.sqrt()))) {
                        y = y.min(d + lift);
                    }
                }
                cap = Some(cap.map_or(y, |c: f64| c.min(y)));
            }
        }
        if let Some(list) = self.node_cells.get(&k) {
            for &ni in list {
                let n = &self.plan.nodes[ni as usize];
                let r = n.approaches.iter().map(|a| a.trim + a.half).fold(0.0, f64::max);
                if ((x - n.position[0]).powi(2) + (z - n.position[1]).powi(2)).sqrt() > r {
                    continue;
                }
                for app in &n.approaches {
                    let f = plan::approach_end_frame(&self.plan.entries, app);
                    // (`seams`: a run moved to meet a busier one's surface, as its pad meets it)
                    let d = self.geoms.get(app.entry).filter(|g| g.seamed).and_then(|g| self.datum_at_station(g, f.s)).unwrap_or(self.ground(f.x, f.z) + self.o.road_lift);
                    let y = d + lift;
                    cap = Some(cap.map_or(y, |c: f64| c.min(y)));
                }
            }
        }
        cap
    }

    /// A point's parameter `raw` along run `ei`'s centreline piece from `a`:
    /// itself on the piece, else None (past the piece's end); with `seams`,
    /// clamped to the piece's end round an interior vertex or a joint with
    /// the next run, where the ribbon's mitre covers the outside of the bend.
    pub(crate) fn piece_t(&self, ei: usize, a: [f64; 2], raw: f64) -> Option<f64> {
        if (0.0..=1.0).contains(&raw) {
            return Some(raw);
        }
        if !seams::road_seams() {
            return None;
        }
        let e = &self.plan.entries[ei];
        let piece = seg_index(e, a);
        let n = e.points.len();
        let dir = |p: [f64; 2], q: [f64; 2]| {
            let l = (q[0] - p[0]).hypot(q[1] - p[1]).max(1e-9);
            [(q[0] - p[0]) / l, (q[1] - p[1]) / l]
        };
        // the heading on past the vertex (in this piece's direction of travel)
        let next = if raw < 0.0 {
            if piece > 0 {
                Some(dir(e.points[piece - 1], e.points[piece]))
            } else {
                self.joints.get(ei).and_then(|j| j[0]).map(|(o, o_start)| {
                    let q = &self.plan.entries[o];
                    let m = q.points.len();
                    if o_start { dir(q.points[1], q.points[0]) } else { dir(q.points[m - 2], q.points[m - 1]) }
                })
            }
        } else if piece + 2 < n {
            Some(dir(e.points[piece + 1], e.points[piece + 2]))
        } else {
            self.joints.get(ei).and_then(|j| j[1]).map(|(o, o_start)| {
                let q = &self.plan.entries[o];
                let m = q.points.len();
                if o_start { dir(q.points[0], q.points[1]) } else { dir(q.points[m - 1], q.points[m - 2]) }
            })
        };
        // only round a bend its ribbon mitres (`plan::frame_at`, `mitre_joints`: cos of the half angle over 0.25)
        let d = dir(e.points[piece], e.points[piece + 1]);
        let next = next?;
        let (ax, az) = (d[0] + next[0], d[1] + next[1]);
        let al = ax.hypot(az);
        if al < 1e-6 || (ax * d[0] + az * d[1]) / al <= 0.25 {
            return None;
        }
        Some(raw.clamp(0.0, 1.0))
    }

    /// The ground a run's street stands on at its centreline point (px, pz),
    /// `t` along its piece from `a` (`len2` long, squared): the survey there,
    /// or (`seams`) where the run was moved to meet a busier run's surface,
    /// its drawn datum less the road lift.
    fn run_ground(&self, ei: usize, a: [f64; 2], t: f64, len2: f64, px: f64, pz: f64) -> f64 {
        if let Some(g) = self.geoms.get(ei).filter(|g| g.seamed) {
            let e = &self.plan.entries[ei];
            if let Some(d) = self.datum_at_station(g, e.cum[seg_index(e, a)] + t * len2.sqrt()) {
                return d - self.o.road_lift;
            }
        }
        self.ground(px, pz)
    }

    /// The street datum at (x, z): the survey plus the road lift, or (`seams`)
    /// on a run moved to meet a busier one's surface, that run's drawn datum
    /// there (the nearest centreline whose carriageway holds the point).
    pub fn street_datum(&self, x: f64, z: f64) -> f64 {
        let k = ((x / 32.0).floor() as i32, (z / 32.0).floor() as i32);
        let mut best: Option<(f64, f64)> = None;
        for &id in self.road_cells.get(&k).map(Vec::as_slice).unwrap_or(&[]) {
            let (ei, a, b, half) = self.road_pieces[id as usize];
            if !self.geoms.get(ei as usize).is_some_and(|g| g.seamed) {
                continue;
            }
            let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
            let len2 = dx * dx + dz * dz;
            if len2 < 1e-12 {
                continue;
            }
            let Some(t) = self.piece_t(ei as usize, a, ((x - a[0]) * dx + (z - a[1]) * dz) / len2) else { continue };
            let (px, pz) = (a[0] + dx * t, a[1] + dz * t);
            let d = (x - px).hypot(z - pz);
            if d <= half && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, self.run_ground(ei as usize, a, t, len2, px, pz) + self.o.road_lift));
            }
        }
        best.map_or_else(|| self.ground(x, z) + self.o.road_lift, |(_, y)| y)
    }

    /// The footway level of another run whose paved corridor (carriageway and
    /// footway) holds (x, z): terrain at its centreline + footwayLiftM.
    pub fn walk_level_beside(&self, x: f64, z: f64, own: Option<usize>) -> Option<f64> {
        let k = ((x / 32.0).floor() as i32, (z / 32.0).floor() as i32);
        let list = self.road_cells.get(&k)?;
        let mut best: Option<f64> = None;
        for &id in list {
            let (ei, a, b, half) = self.road_pieces[id as usize];
            if Some(ei as usize) == own {
                continue;
            }
            let e = &self.plan.entries[ei as usize];
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
            let left = (x - px) * -dz + (z - pz) * dx > 0.0;
            let walk = if left { e.walks[0] } else { e.walks[1] };
            if ((x - px).powi(2) + (z - pz).powi(2)).sqrt() > half + walk {
                continue;
            }
            let datum = self.neighbour_datum(ei as usize, a, t * len2.sqrt(), [px, pz]);
            let y = datum + (self.o.footway_lift - self.o.road_lift);
            best = Some(best.map_or(y, |c: f64| c.max(y)));
        }
        best
    }

    /// the lowest paving another run lays at (x, z): its gutter
    /// invert over its carriageway, its walk level over its footway, and its
    /// frontage level (walk + frontageLift) out to its surveyed property line
    /// plus `frontagePastLine` (where its frontage paving can reach). A paved
    /// corridor (carriageway or footway) outranks a frontage zone.
    pub fn paving_level_beside(&self, x: f64, z: f64, own: Option<usize>) -> Option<f64> {
        let k = ((x / 32.0).floor() as i32, (z / 32.0).floor() as i32);
        let (mut paved, mut front): (Option<f64>, Option<f64>) = (None, None);
        for gz in k.1 - 1..=k.1 + 1 {
            for gx in k.0 - 1..=k.0 + 1 {
                let Some(list) = self.road_cells.get(&(gx, gz)) else { continue };
                for &id in list {
                    let (ei, a, b, half) = self.road_pieces[id as usize];
                    if Some(ei as usize) == own {
                        continue;
                    }
                    let e = &self.plan.entries[ei as usize];
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
                    let left = (x - px) * -dz + (z - pz) * dx > 0.0;
                    let walk = if left { e.walks[0] } else { e.walks[1] };
                    let walk = if walk >= self.o.min_sidewalk_width { walk } else { 0.0 };
                    let d = ((x - px).powi(2) + (z - pz).powi(2)).sqrt();
                    let g = self.neighbour_datum(ei as usize, a, t * len2.sqrt(), [px, pz]) - self.o.road_lift;
                    let low = |a: Option<f64>, y: f64| Some(a.map_or(y, |c: f64| c.min(y)));
                    if d <= half {
                        paved = low(paved, g + self.o.road_lift - self.o.gutter_depth);
                    } else if d <= half + walk {
                        paved = low(paved, g + self.o.footway_lift);
                    } else if walk > 0.0 {
                        let row = self.city.segments[e.road].right_of_way.filter(|r| *r > 0.0);
                        let reach_front = row.map(|r| (r / 2.0).max(half + walk) + self.o.frontage_past_line).unwrap_or(half + walk + self.o.frontage_reach);
                        if d <= reach_front {
                            front = low(front, g + self.o.footway_lift + self.o.frontage_lift);
                        }
                    }
                }
            }
        }
        paved.or(front)
    }

    /// Datum of the neighbouring paving an earth bank meets. A grade-separated
    /// road may raise a stub or cut a street below the bare-earth grid.
    fn neighbour_datum(&self, ei: usize, piece_start: [f64; 2], along: f64, at: [f64; 2]) -> f64 {
        let terrain = self.ground(at[0], at[1]) + self.o.road_lift;
        let e = &self.plan.entries[ei];
        let has_struct = e.structure_ends.iter().any(Option::is_some) || !e.structure_ramps.is_empty();
        if let Some(g) = self.geoms.get(ei).filter(|g| has_struct || g.seamed) {
            let station = e.cum[seg_index(e, piece_start)] + along;
            if let Some(d) = self.datum_at_station(g, station) {
                return d;
            }
        }
        self.surveyed_datum(e, at, terrain).unwrap_or(terrain)
    }

    /// Datum of a measured bridge approach or a connected landing that rises
    /// above the existing street. Junction arms carry it at their trimmed mouths.
    pub fn surveyed_datum(&self, e: &plan::Entry, at: [f64; 2], terrain: f64) -> Option<f64> {
        let way = self.city.segments[e.road].street_id.as_deref();
        if self.o.pavement_repairs {
            // The surveyed entrance and street are below the freeway. A rise
            // requested by an overhead deck cannot set this junction's datum.
            if let Some(y) = way.and_then(|id| eo_data::bridge_survey::mariposa_grade(id, at, terrain)) {
                return Some(y);
            }
        }
        let driveway = way.filter(|_| self.o.pavement_repairs)
            .and_then(|way| eo_data::bridge_survey::driveway_grade(way, at, terrain));
        if self.o.structures.is_none() { return driveway; }
        let measured = driveway.or_else(|| way.and_then(|way| eo_data::bridge_survey::approach(way, at, terrain)));
        if e.landings.is_empty() { return measured; }
        let (s, _) = plan::nearest_station(e, at);
        let s = s.clamp(e.s0(), e.s1());
        let base = measured.unwrap_or(terrain);
        let y = e.landings.iter().fold(base, |y, (from, rise)| joins::ramp_datum(y, rise, (s - from).abs()));
        if y > base { Some(y) } else { measured }
    }

    /// Stations, datums, kept spans and kerb depressions of one run.
    fn entry_geom(&self, ei: usize) -> EntryGeom {
        let o = &self.o;
        let e = &self.plan.entries[ei];
        let (mut s0, mut s1) = (e.s0(), e.s1());
        if s1 - s0 < 0.02 || e.structure_leg.is_some() {
            return EntryGeom::default();
        }
        // (`seams`) a slip road alongside its through road drawn from the
        // through leg's mouth (`plan::Approach::merge_trim`).
        if seams::road_seams() {
            let merge = |end: Option<(usize, usize)>| end.and_then(|(n, a)| self.plan.nodes[n].approaches[a].merge_trim);
            if let Some(t) = merge(e.approach_start) {
                s0 = s0.min(t.max(0.0));
            }
            if let Some(t) = merge(e.approach_end) {
                s1 = s1.max(e.length - t.max(0.0));
            }
        }
        let mut stations = plan::build_stations(&e.points, &e.cum, s0, s1, o.max_step);
        let ramps = plan::resolve_kerb_ramps(e);
        let mut extra = vec![];
        for sp in e.yield_spans.iter().chain(e.walk_yield_spans[0].iter()).chain(e.walk_yield_spans[1].iter()) {
            for v in [sp[0], sp[1]] {
                if v > s0 + 1e-3 && v < s1 - 1e-3 {
                    extra.push(v);
                }
            }
        }
        for list in &ramps {
            for w in list {
                for v in [w.flare_lo, w.lo, w.hi, w.flare_hi] {
                    if v > s0 + 1e-3 && v < s1 - 1e-3 {
                        extra.push(v);
                    }
                }
            }
        }
        if !extra.is_empty() {
            let mut merged: Vec<plan::Frame> = stations.clone();
            merged.extend(extra.iter().map(|&s| {
                if o.pavement_repairs {
                    // A ramp or yield boundary can sit millimetres from a
                    // surveyed bend. Keep its kerbs on the mitered edges so
                    // the new cross-section cannot fold back over the road.
                    let i = stations.partition_point(|f| f.s < s);
                    if i > 0 && i < stations.len() {
                        return stations[i - 1].split(stations[i], s);
                    }
                }
                e.frame(s, false)
            }));
            merged.sort_by(|a, b| a.s.partial_cmp(&b.s).unwrap());
            stations.clear();
            for st in merged {
                if let Some(l) = stations.last() {
                    if (l.s - st.s).abs() < 1e-4 {
                        continue;
                    }
                }
                stations.push(st);
            }
        }
        let mut datums: Vec<f64> = stations.iter().map(|st| {
            let terrain = self.ground(st.x, st.z) + o.road_lift;
            self.surveyed_datum(e, [st.x, st.z], terrain).unwrap_or(terrain)
        }).collect();
        // an approach embankment (`joins::StructureEnd::rise`, only with
        // `StreetOptions::structures`): a stub's datum climbs to the height
        // its structure asks at its end, where the ramp is back at the datum
        // before the run's other drawn end.
        if e.structure_ends.iter().any(Option::is_some) || !e.structure_ramps.is_empty() {
            self.raise_stub(e, &stations, &mut datums);
        }
        driveway::insert_curb_stations(&e.driveway_cuts, &mut stations, &mut datums);
        let keep: Vec<bool> = (0..stations.len().saturating_sub(1)).map(|i| !e.in_yield_span((stations[i].s + stations[i + 1].s) / 2.0)).collect();
        let drawn = plan::segment_kerb_ramps(e, o);
        let dep = [0, 1].map(|k| stations.iter().map(|st| mesh::dep_at(&drawn[k], st.s)).collect::<Vec<f64>>());
        EntryGeom { stations, datums, keep, dep, ..Default::default() }
    }

    /// Which structure ends' rises are drawn: a ramp from the end at its
    /// grade down along the stub and the runs it continues into
    /// (`plan::Entry::structure_ramps`) that is back at the street datum
    /// before it meets a junction or the last of those runs.
    fn check_rises(&self) -> Vec<[bool; 2]> {
        let Some(joins) = self.o.structures.as_ref() else { return vec![] };
        let o = &self.o;
        let mut ok = vec![[false; 2]; joins.ends.len()];
        for (j, end) in joins.ends.iter().enumerate() {
          for (which, r) in [(0usize, end.rise), (1, end.cut)] {
            let Some(r) = r else { continue };
            if !(r.grade > 0.0) || !r.to.is_finite() {
                continue;
            }
            // the runs it climbs over, nearest first.
            let mut runs: Vec<(usize, bool, f64)> = vec![];
            for (ei, e) in self.plan.entries.iter().enumerate() {
                for k in 0..2 {
                    if e.structure_ends[k] == Some(j) {
                        runs.push((ei, k == 0, 0.0));
                    }
                }
                runs.extend(e.structure_ramps.iter().filter(|x| x.0 == j).map(|x| (ei, x.1, x.2)));
            }
            runs.sort_by(|a, b| a.2.total_cmp(&b.2).then(a.0.cmp(&b.0)));
            for (q, &(ei, start, off)) in runs.iter().enumerate() {
                let e = &self.plan.entries[ei];
                if if start { e.approach_start.is_some() } else { e.approach_end.is_some() } {
                    break;
                }
                let (s0, s1) = (e.s0(), e.s1());
                let n = (((s1 - s0) / o.max_step).ceil() as usize).max(1);
                let back = (0..=n).any(|i| {
                    let s = s0 + (s1 - s0) * i as f64 / n as f64;
                    let f = e.frame(s, false);
                    let dist = off + if start { s } else { e.length - s };
                    let datum = self.ground(f.x, f.z) + o.road_lift;
                    if which == 0 { joins::ramp_datum(datum, &r, dist) <= datum } else { joins::cut_datum(datum, &r, dist) >= datum }
                });
                if back {
                    ok[j][which] = true;
                    break;
                }
                let far_node = if start { e.approach_end.is_some() } else { e.approach_start.is_some() };
                if far_node || q + 1 == runs.len() {
                    break;
                }
            }
          }
        }
        ok
    }

    /// A run's datums raised on the ramps of the structure ends it climbs to
    /// (`joins::StructureEnd::rise`, its own end or one ahead of it), those
    /// `check_rises` lets stand.
    fn raise_stub(&self, e: &plan::Entry, stations: &[plan::Frame], datums: &mut [f64]) {
        let Some(joins) = self.o.structures.as_ref() else { return };
        // (rise or cut, its end toward the structure, the distance to it).
        let mut ramps: Vec<(usize, joins::Rise, bool, f64)> = vec![];
        let mut add = |j: usize, start: bool, off: f64| {
            for (which, r) in [(0usize, joins.ends.get(j).and_then(|x| x.rise)), (1, joins.ends.get(j).and_then(|x| x.cut))] {
                if let (Some(r), Some(true)) = (r, self.rises_ok.get(j).map(|ok| ok[which])) {
                    ramps.push((which, r, start, off));
                }
            }
        };
        for k in 0..2 {
            let at_node = if k == 0 { e.approach_start.is_some() } else { e.approach_end.is_some() };
            if let (Some(j), false) = (e.structure_ends[k], at_node) {
                add(j, k == 0, 0.0);
            }
        }
        for &(j, start, off) in &e.structure_ramps {
            add(j, start, off);
        }
        for (which, r, start, off) in ramps {
            for (d, st) in datums.iter_mut().zip(stations) {
                let dist = off + if start { st.s } else { e.length - st.s };
                *d = if which == 0 { joins::ramp_datum(*d, &r, dist) } else { joins::cut_datum(*d, &r, dist) };
            }
        }
    }

    /// The street datum a run draws at station `i` of its geometry less the
    /// ground's own datum there (above 0 only on an approach embankment).
    pub fn datum_raise(&self, ei: usize, i: usize) -> f64 {
        let g = &self.geoms[ei];
        let st = &g.stations[i];
        g.datums[i] - (self.ground(st.x, st.z) + self.o.road_lift)
    }

    /// Every tile that draws streets.
    pub fn tile_keys(&self) -> Vec<(i32, i32)> {
        let mut k: Vec<(i32, i32)> = self.tiles.keys().copied().collect();
        k.sort();
        k
    }

    /// Install the final tunnel approach geometry after the joined road profiles settle.
    pub fn set_road_openings(&mut self, openings: Arc<road_openings::RoadOpenings>) {
        self.road_openings = openings;
        self.paving_cache.get_mut().unwrap().clear();
        self.surface_cache.get_mut().unwrap().clear();
        self.mesh_cache.get_mut().unwrap().clear();
    }

    /// The near (full) street surface of one tile.
    pub fn tile_mesh(&self, key: (i32, i32)) -> TileMesh {
        let mut m = self.draw_tile(key, true);
        if self.o.pavement_repairs {
            self.finish_paving(key, &mut m);
        }
        m
    }

    fn draw_tile(&self, key: (i32, i32), with_edges: bool) -> TileMesh {
        let mut m = self.street_tile(key, with_edges);
        self.apron.draw(key, &mut m, with_edges);
        self.apply_surface_overlays(key, &mut m);
        m
    }

    fn apply_surface_overlays(&self, key: (i32, i32), mesh: &mut TileMesh) {
        if let Some(surface) = &self.surveyed_junction {
            surface.draw(key, mesh, self.o.junction_paint_lift);
        }
        mesh.open_roadways(&self.road_openings);
    }

    fn street_tile(&self, key: (i32, i32), with_edges: bool) -> TileMesh {
        let mut m = TileMesh::default();
        if let Some(refs) = self.tiles.get(&key) {
            for &(ei, i) in &refs.spans {
                let first = m.layers.len();
                self.emit_span(&mut m, ei as usize, i as usize, with_edges);
                self.pavement_overlap.finish_span(&mut m, ei as usize, i as usize, first);
            }
            for &ni in &refs.nodes {
                self.emit_node(&mut m, ni as usize, with_edges);
            }
        }
        m
    }

    /// (diagnostic) every walkable surface drawn over (x, z) by the 3 x 3
    /// tiles round it, named by what drew it: a run's span or a node's pad
    /// and corners, with its layer and height.
    pub fn surface_sources(&self, x: f64, z: f64) -> Vec<(String, Layer, f64)> {
        let key = tile_key(x, z);
        let mut out = vec![];
        let mut seen_spans = HashSet::new();
        let mut seen_nodes = HashSet::new();
        for dj in -1..=1 {
            for di in -1..=1 {
                let Some(refs) = self.tiles.get(&(key.0 + di, key.1 + dj)) else { continue };
                let mut probe = |m: &TileMesh, name: String| {
                    for (t, l) in m.indices.chunks_exact(3).zip(&m.layers) {
                        if !l.walkable() {
                            continue;
                        }
                        let p = [t[0], t[1], t[2]].map(|i| m.positions[i as usize]);
                        let (ax, az, bx, bz, cx, cz) = (p[0][0] as f64, p[0][2] as f64, p[1][0] as f64, p[1][2] as f64, p[2][0] as f64, p[2][2] as f64);
                        let d = (bz - cz) * (ax - cx) + (cx - bx) * (az - cz);
                        if d.abs() < 1e-12 {
                            continue;
                        }
                        let w0 = ((bz - cz) * (x - cx) + (cx - bx) * (z - cz)) / d;
                        let w1 = ((cz - az) * (x - cx) + (ax - cx) * (z - cz)) / d;
                        let w2 = 1.0 - w0 - w1;
                        if w0 >= -1e-6 && w1 >= -1e-6 && w2 >= -1e-6 {
                            out.push((name.clone(), *l, w0 * p[0][1] as f64 + w1 * p[1][1] as f64 + w2 * p[2][1] as f64));
                        }
                    }
                };
                for &(ei, i) in &refs.spans {
                    if !seen_spans.insert((ei, i)) {
                        continue;
                    }
                    let mut m = TileMesh::default();
                    self.emit_span(&mut m, ei as usize, i as usize);
                    probe(&m, format!("run {ei} span {i}"));
                }
                for &ni in &refs.nodes {
                    if !seen_nodes.insert(ni) {
                        continue;
                    }
                    let mut m = TileMesh::default();
                    self.emit_node(&mut m, ni as usize);
                    probe(&m, format!("node {ni}"));
                }
            }
        }
        out
    }

    /// The far level of one tile.
    pub fn far_tile_mesh(&self, key: (i32, i32)) -> TileMesh {
        let mut m = TileMesh::default();
        if let Some(refs) = self.tiles.get(&key) {
            for (ei, i0, i1) in self.span_runs(&refs.spans, 2) {
                let first = m.layers.len();
                self.emit_far_span(&mut m, ei, i0, i1);
                self.pavement_overlap.finish_span(&mut m, ei, i0, first);
            }
            for &ni in &refs.nodes {
                self.emit_far_node(&mut m, ni as usize);
            }
        }
        self.apron.draw(key, &mut m, false);
        self.apply_surface_overlays(key, &mut m);
        m
    }

    /// The coarse level of one tile: every 4th station of each run anchored here.
    pub fn coarse_tile_mesh(&self, key: (i32, i32)) -> TileMesh {
        let mut m = TileMesh::default();
        let Some(refs) = self.tiles.get(&key) else { return m };
        for (ei, i0, i1) in self.span_runs(&refs.spans, 4) {
            let first = m.layers.len();
            self.emit_coarse_span(&mut m, ei, i0, i1);
            self.pavement_overlap.finish_span(&mut m, ei, i0, first);
        }
        // Runs end at their trimmed mouths at every distance. Keep the same
        // junction pad across the two static levels so the asphalt meets there.
        if self.o.pavement_repairs {
            for &ni in &refs.nodes { self.emit_far_node(&mut m, ni as usize); }
        }
        self.apron.draw(key, &mut m, false);
        self.apply_surface_overlays(key, &mut m);
        m
    }

    /// Consecutive kept spans of one run joined up to n at a time: (entry, first station, last station).
    fn span_runs(&self, spans: &[(u32, u32)], n: usize) -> Vec<(usize, usize, usize)> {
        let mut by_entry: Vec<(u32, u32)> = spans.to_vec();
        by_entry.sort();
        let mut out = vec![];
        let mut k = 0;
        while k < by_entry.len() {
            let (ei, i0) = by_entry[k];
            let mut j = k;
            while j + 1 < by_entry.len() && by_entry[j + 1].0 == ei && by_entry[j + 1].1 == by_entry[j].1 + 1 && j + 1 - k < n
                && !self.geoms[ei as usize].crosses_apron(i0 as usize, by_entry[j+1].1 as usize + 1)
                && !self.pavement_overlap.needs_detail(ei as usize, by_entry[j].1 as usize)
                && !self.pavement_overlap.needs_detail(ei as usize, by_entry[j+1].1 as usize) {
                j += 1;
            }
            out.push((ei as usize, i0 as usize, by_entry[j].1 as usize + 1));
            k = j + 1;
        }
        out
    }

    /// The near mesh of a tile, shared (built once, kept while the cache is small).
    pub fn cached_mesh(&self, key: (i32, i32)) -> Arc<TileMesh> {
        if let Some(m) = self.mesh_cache.lock().unwrap().get(&key) {
            return m.clone();
        }
        let m = Arc::new(self.tile_mesh(key));
        let mut c = self.mesh_cache.lock().unwrap();
        if c.len() > 2048 {
            c.clear();
        }
        c.insert(key, m.clone());
        m
    }

    fn surface_tile(&self, key: (i32, i32)) -> Arc<SurfaceTile> {
        if let Some(s) = self.surface_cache.lock().unwrap().get(&key) {
            return s.clone();
        }
        let meshes: Vec<Arc<TileMesh>> = (-1..=1).flat_map(|j| (-1..=1).map(move |i| (key.0 + i, key.1 + j))).map(|k| self.cached_mesh(k)).collect();
        let refs: Vec<&TileMesh> = meshes.iter().map(|m| m.as_ref()).collect();
        let s = Arc::new(SurfaceTile::build(&refs, key));
        let mut c = self.surface_cache.lock().unwrap();
        if c.len() > 1024 {
            c.clear();
        }
        c.insert(key, s.clone());
        s
    }

    /// The street paving drawn at (x, z): the highest walkable street triangle, or None.
    pub fn paved_at(&self, x: f64, z: f64) -> Option<f64> {
        self.surface_tile(tile_key(x, z)).height(x, z)
    }

    /// what the paving at (x, z) is (carriageway, kerb top, sidewalk, ramp, verge), or None off the street.
    pub fn paved_layer_at(&self, x: f64, z: f64) -> Option<Layer> {
        self.surface_tile(tile_key(x, z)).top_layer(x, z)
    }

    /// True only where every drawn floor at this point belongs to the road.
    /// A lower sidewalk underneath a road deck still has its own ramp fittings.
    pub fn only_carriageway_at(&self, x: f64, z: f64) -> bool {
        let tile = self.surface_tile(tile_key(x, z));
        let mut hits = tile.hits_at(x, z);
        hits.next().is_some_and(|(kind, _)| kind == Layer::Carriageway)
            && hits.all(|(kind, _)| kind == Layer::Carriageway)
    }

    /// The carriageway at this level when a junction also carries an upper street.
    pub fn carriageway_near(&self, x: f64, z: f64, y: f64) -> Option<f64> {
        self.surface_tile(tile_key(x, z)).heights_on(x, z, Some(Layer::Carriageway))
            .into_iter().min_by(|a,b| (a-y).abs().total_cmp(&(b-y).abs()))
    }

    /// Every street surface drawn over (x, z) (walkable layers).
    pub fn paved_heights(&self, x: f64, z: f64) -> Vec<f64> {
        self.surface_tile(tile_key(x, z)).heights(x, z)
    }

    /// Every street surface drawn over (x, z) (walkable layers), with its layer.
    pub fn paved_layers(&self, x: f64, z: f64) -> Vec<(f64, Layer)> {
        self.surface_tile(tile_key(x, z)).layered_heights(x, z)
    }

    /// `drawnSurfaceAt` (street-front-grade.js): the paving at (x, z), or the
    /// ground carpet; with a wall's outward normal, past a bank to the first
    /// walking slope within `bank_reach_m`.
    pub fn surface_at(&self, x: f64, z: f64, n: Option<[f64; 2]>) -> f64 {
        if let Some(walk) = self.paved_at(x, z) {
            let Some(n) = n else { return walk };
            let mut y = walk;
            let step = self.bank_step_m;
            let mut d = 0.0;
            while d <= self.bank_reach_m + 1e-9 {
                let Some(next) = self.paved_at(x + n[0] * (d + step), z + n[1] * (d + step)) else { break };
                if (next - y).abs() / step <= self.ramp_running_slope_max {
                    return y;
                }
                y = next;
                d += step;
            }
            return walk;
        }
        self.carpet_y(x, z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn building_index_push_outside_handles_interior_margin_and_party_wall() {
        // Two buildings sharing a party wall at x = 10.
        let b1 = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let b2 = vec![[10.0, 0.0], [20.0, 0.0], [20.0, 10.0], [10.0, 10.0]];
        let idx = BuildingIndex::new(vec![b1, b2]);

        // Clear point is untouched.
        assert_eq!(idx.push_outside(-2.0, 5.0, 0.4), None);

        // Point inside b1 near the west wall exits west to x = -0.4.
        let p = idx.push_outside(0.3, 5.0, 0.4).expect("inside b1");
        assert!(!idx.contains(p[0], p[1]));
        assert!((p[0] + 0.4).abs() < 1e-6 && (p[1] - 5.0).abs() < 1e-6, "got {p:?}");

        // Point inside b1 closest to the shared party wall (x=9.8, z=2.0) exits
        // through the open south wall (z=-0.4), not into b2.
        let q = idx.push_outside(9.8, 2.0, 0.4).expect("near party wall");
        assert!(!idx.contains(q[0], q[1]), "exited into neighbour: {q:?}");
        assert!(q[1] <= -0.39, "expected south exit, got {q:?}");

        // Exterior point grazing a wall within margin is pushed to margin.
        let g = idx.push_outside(-0.1, 5.0, 0.4).expect("grazing wall");
        assert!(!idx.contains(g[0], g[1]));
        assert!((g[0] + 0.4).abs() < 1e-6 && (g[1] - 5.0).abs() < 1e-6, "got {g:?}");
    }
}
