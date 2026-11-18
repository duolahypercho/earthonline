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

pub mod city;
pub mod evidence;
pub mod grounding;
pub mod material;
pub mod mesh;
pub mod options;
pub mod paint;
pub mod plan;

pub use city::StreetCity;
pub use mesh::TileMesh;
pub use options::StreetOptions;
pub use plan::Plan;

use eo_data::ElevationGrid;
use mesh::EntryGeom;
use std::collections::{HashMap, HashSet};
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
}

impl Layer {
    /// Something a body or a floor probe stands on (STREET_COLLISION_ROLES less the faces).
    pub fn walkable(self) -> bool {
        matches!(self, Layer::Carriageway | Layer::CurbTop | Layer::Sidewalk | Layer::Ramp | Layer::Verge)
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

    /// the layer of the highest walkable triangle over (x, z), if any.
    pub fn top_layer(&self, x: f64, z: f64) -> Option<Layer> {
        let i = ((x - self.origin[0]) / self.cell).floor();
        let j = ((z - self.origin[1]) / self.cell).floor();
        if i < 0.0 || j < 0.0 || i >= self.n as f64 || j >= self.n as f64 {
            return None;
        }
        let mut best: Option<(f64, Layer)> = None;
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
            if wa < -1e-6 || wb < -1e-6 || wc < -1e-6 {
                continue;
            }
            let y = wa * p[1] as f64 + wb * p[4] as f64 + wc * p[7] as f64;
            if best.is_none_or(|b| y > b.0) {
                best = Some((y, self.layers[t as usize]));
            }
        }
        best.map(|b| b.1)
    }

    /// The highest walkable triangle over (x, z), if any.
    pub fn height(&self, x: f64, z: f64) -> Option<f64> {
        self.heights(x, z).into_iter().reduce(f64::max)
    }

    /// Every walkable triangle's height over (x, z).
    pub fn heights(&self, x: f64, z: f64) -> Vec<f64> {
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
            out.push(wa * p[1] as f64 + wb * p[4] as f64 + wc * p[7] as f64);
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
    /// `ramp-running-slope-max` (drawnSurfaceAt's walking slope).
    pub ramp_running_slope_max: f64,
    pub bank_step_m: f64,
    pub bank_reach_m: f64,
    surface_cache: Mutex<HashMap<(i32, i32), Arc<SurfaceTile>>>,
    mesh_cache: Mutex<HashMap<(i32, i32), Arc<TileMesh>>>,
    pub build_ms: f64,
    /// the shore's ground rule over the carpet (eo-water
    /// `ShoreField::press_beach`: a beach laid on its foreshore), set once the
    /// shore field is loaded, so the drawn ground, the collision ground and
    /// everything that stands on the carpet read one height.
    shore_press: std::sync::OnceLock<ShorePress>,
}

/// (x, z, carpet height) -> the carpet height at the shore.
pub type ShorePress = Arc<dyn Fn(f64, f64, f64) -> f64 + Send + Sync>;

impl StreetWorld {
    pub fn build(o: StreetOptions, city: StreetCity, terrain: Terrain, buildings: BuildingIndex) -> Self {
        let plan = plan::build_plan(&city, &o);
        Self::from_plan(o, city, plan, terrain, buildings)
    }

    /// The street world over a plan already built (the plan needs no terrain or buildings).
    pub fn from_plan(o: StreetOptions, city: StreetCity, plan: Plan, terrain: Terrain, buildings: BuildingIndex) -> Self {
        let t0 = std::time::Instant::now();
        if std::env::var("EO_STREET_TRACE").is_ok() {
            eprintln!("  street plan done: {:.0} ms", t0.elapsed().as_secs_f64() * 1e3);
        }
        let multi: HashSet<u64> = o.multi_lane_two_way.iter().copied().collect();
        let lines: Vec<Vec<paint::Line>> = plan
            .entries
            .iter()
            .map(|e| paint::segment_markings(&city.segments[e.road], &e.highway, e.half * 2.0, e.lanes, &o, &multi))
            .collect();
        let mut road_cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        let mut road_pieces = vec![];
        for (ei, e) in plan.entries.iter().enumerate() {
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
            ramp_running_slope_max,
            bank_step_m,
            bank_reach_m,
            surface_cache: Mutex::new(HashMap::new()),
            mesh_cache: Mutex::new(HashMap::new()),
            build_ms: 0.0,
            shore_press: std::sync::OnceLock::new(), // sea
        };
        w.geoms = (0..w.plan.entries.len()).map(|ei| w.entry_geom(ei)).collect();
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
        w.build_ms = t0.elapsed().as_secs_f64() * 1e3;
        w
    }

    pub fn ground(&self, x: f64, z: f64) -> f64 {
        self.terrain.ground(x, z)
    }
    /// The ground carpet's vertex at a terrain grid vertex (world position as
    /// eo-render/scene.rs computes it, in f32): the terrain lifted by
    /// `openGroundLiftM`, held under any street there (`terrain_cap`).
    pub fn carpet_vertex(&self, wx: f32, wz: f32) -> f32 {
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
            self.surface_cache.lock().unwrap().clear();
            self.mesh_cache.lock().unwrap().clear();
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
                let t = ((x - a[0]) * dx + (z - a[1]) * dz) / len2;
                if !(0.0..=1.0).contains(&t) {
                    continue;
                }
                let (px, pz) = (a[0] + dx * t, a[1] + dz * t);
                // half a terrain cell past the paved corridor too, so
                // an 8 m cell straddling the corridor edge on a cross slope
                // cannot interpolate the carpet up through the street.
                // the cell is 8 x s m on a scaled plan; so is its half.
                // a whole cell now (`TERRAIN_CAP_MARGIN_M`).
                if ((x - px).powi(2) + (z - pz).powi(2)).sqrt() > reach + TERRAIN_CAP_MARGIN_M * eo_data::groundscale::scale() {
                    continue;
                }
                let y = self.ground(px, pz) + self.o.road_lift + lift;
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
                    let y = self.ground(f.x, f.z) + self.o.road_lift + lift;
                    cap = Some(cap.map_or(y, |c: f64| c.min(y)));
                }
            }
        }
        cap
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
            let y = self.ground(px, pz) + self.o.footway_lift;
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
                    let g = self.ground(px, pz);
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

    /// Stations, datums, kept spans and kerb depressions of one run (`emitSegment`'s set-up).
    fn entry_geom(&self, ei: usize) -> EntryGeom {
        let o = &self.o;
        let e = &self.plan.entries[ei];
        let (s0, s1) = (e.s0(), e.s1());
        if s1 - s0 < 0.02 {
            return EntryGeom::default();
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
            merged.extend(extra.iter().map(|&v| e.frame(v, false)));
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
        let datums: Vec<f64> = stations.iter().map(|st| self.ground(st.x, st.z) + o.road_lift).collect();
        let keep: Vec<bool> = (0..stations.len().saturating_sub(1)).map(|i| !e.in_yield_span((stations[i].s + stations[i + 1].s) / 2.0)).collect();
        let drawn = plan::segment_kerb_ramps(e, o);
        let dep = [0, 1].map(|k| stations.iter().map(|st| mesh::dep_at(&drawn[k], st.s)).collect::<Vec<f64>>());
        EntryGeom { stations, datums, keep, dep }
    }

    /// Every tile that draws streets.
    pub fn tile_keys(&self) -> Vec<(i32, i32)> {
        let mut k: Vec<(i32, i32)> = self.tiles.keys().copied().collect();
        k.sort();
        k
    }

    /// The near (full) street surface of one tile.
    pub fn tile_mesh(&self, key: (i32, i32)) -> TileMesh {
        let mut m = TileMesh::default();
        if let Some(refs) = self.tiles.get(&key) {
            for &(ei, i) in &refs.spans {
                self.emit_span(&mut m, ei as usize, i as usize);
            }
            for &ni in &refs.nodes {
                self.emit_node(&mut m, ni as usize);
            }
        }
        m
    }

    /// The far level of one tile.
    pub fn far_tile_mesh(&self, key: (i32, i32)) -> TileMesh {
        let mut m = TileMesh::default();
        if let Some(refs) = self.tiles.get(&key) {
            for (ei, i0, i1) in Self::span_runs(&refs.spans, 2) {
                self.emit_far_span(&mut m, ei, i0, i1);
            }
            for &ni in &refs.nodes {
                self.emit_far_node(&mut m, ni as usize);
            }
        }
        m
    }

    /// The coarse level of one tile: every 4th station of each run anchored here.
    pub fn coarse_tile_mesh(&self, key: (i32, i32)) -> TileMesh {
        let mut m = TileMesh::default();
        let Some(refs) = self.tiles.get(&key) else { return m };
        for (ei, i0, i1) in Self::span_runs(&refs.spans, 4) {
            self.emit_coarse_span(&mut m, ei, i0, i1);
        }
        m
    }

    /// Consecutive kept spans of one run joined up to n at a time: (entry, first station, last station).
    fn span_runs(spans: &[(u32, u32)], n: usize) -> Vec<(usize, usize, usize)> {
        let mut by_entry: Vec<(u32, u32)> = spans.to_vec();
        by_entry.sort();
        let mut out = vec![];
        let mut k = 0;
        while k < by_entry.len() {
            let (ei, i0) = by_entry[k];
            let mut j = k;
            while j + 1 < by_entry.len() && by_entry[j + 1].0 == ei && by_entry[j + 1].1 == by_entry[j].1 + 1 && j + 1 - k < n {
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

    /// Every street surface drawn over (x, z) (walkable layers).
    pub fn paved_heights(&self, x: f64, z: f64) -> Vec<f64> {
        self.surface_tile(tile_key(x, z)).heights(x, z)
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
