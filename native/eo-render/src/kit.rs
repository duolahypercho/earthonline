//! Kit (archetype) buildings in the GPU-driven renderer (slice 3).
//!
//! Every non-authored building is one LOD group. Its footprint box (slice 1's
//! extrusion) is the far level; its kit geometry comes in two rungs, cooked
//! per 128 m tile on worker threads (eo-kit: the JS planner ported), cached
//! under `cache/kit-tiles/`, and written into arenas of the shared vertex,
//! index and object buffers through the budgeted upload queue:
//!
//!   near pack  rung 0 (the full plan) of every kit building of a tile
//!   mid pack   rung 2 (joinery and per-opening trim dropped, wall bands merged)
//!
//! The LOD pass (cull.wgsl `lod`) picks one level per building each frame
//! from its distance (near <= 65 m, mid <= 470 m, with hysteresis) and what is
//! resident; the cull pass draws the rung objects or the box of that level
//! only, so a building is never drawn twice and never missing. The render
//! thread does per-TILE bookkeeping (which packs to request or evict), never
//! per-building work per frame.
//!
//! Object granularity: one object per building per rung (material per vertex
//! from the kit palette), or with `split_materials` one per building, rung and
//! material (same buffers, more objects) - kept for the measurement.

use crate::authored::{GpuLodGroup, MaterialTable};
use crate::scene::{GpuObject, LOD_KIT_MID, LOD_KIT_NEAR, NO_GROUP, ObjectKind, Vertex};
use crate::upload::{Data, Job, Target, UploadQueue, Write};
use eo_kit::city::KitCity;
use glam::Vec3;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Near enter, near exit, mid enter, mid exit (metres from the camera to the
/// building's bounds): the JS ladder's 65 m and 470 m with 8 % hysteresis.
pub const DEFAULT_KIT_RADII: [f32; 4] = [65.0, 70.2, 470.0, 507.6];
pub const GROUP_KIND_KIT: u32 = 1;
/// a kit building a landmark claims (landmark-registry.js
/// `claims`): its group never picks a level (`level` stays `LEVEL_NONE`), so
/// neither rung nor box draws; its packs still stream as the cache holds them.
pub const GROUP_KIND_CLAIMED: u32 = 4;
pub const LEVEL_NONE: u32 = 3;
pub const KIT_TILE_M: f64 = 128.0;
/// facade-test (off by default; `--facade-test box|shader`): 0 off; 1 the near
/// pack cooks the envelope box (rung 3) in place of rung 0; 2 the same box with
/// wall coordinates in two normal bytes for the shader-only facade (draw.wgsl
/// `ft_facade`): the w byte counts bays along the wall, the y byte storeys up it.
pub static FACADE_TEST: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
/// `PAINTED_FENESTRATION_BAY_M`: the bay the shader facade divides a wall by.
const FACADE_TEST_BAY_M: f64 = 2.8;

/// mark the envelope's wall quads of one sink (already pushed to
/// `verts`, in order) with their bay and storey counts. Returns the walls' top.
fn facade_test_mark(sink: &eo_kit::mesh::Sink, verts: &mut [Vertex], base_y: f64, roof_top: f64, storeys: usize) -> Option<f32> {
    let mut top: Option<f32> = None;
    let storeys = storeys.clamp(1, 100) as u32;
    for q in sink.indices.chunks_exact(6) {
        let b = q[0] as usize;
        if q != [q[0], q[0] + 1, q[0] + 2, q[0], q[0] + 2, q[0] + 3] || b + 3 >= sink.positions.len() {
            continue;
        }
        let n = sink.normals[b];
        if n[1].abs() > 0.3 {
            continue;
        }
        let t = [-(n[2] as f64), n[0] as f64];
        let p = &sink.positions[b..b + 4];
        let s: Vec<f64> = p.iter().map(|p| p[0] * t[0] + p[2] * t[1]).collect();
        let (s0, s1) = (s.iter().cloned().fold(f64::MAX, f64::min), s.iter().cloned().fold(f64::MIN, f64::max));
        let (y0, y1) = (p.iter().map(|p| p[1]).fold(f64::MAX, f64::min), p.iter().map(|p| p[1]).fold(f64::MIN, f64::max));
        // Only the envelope's own walls: floor to roof.
        if (y0 - base_y).abs() > 0.2 || (y1 - y0) < roof_top - 0.2 {
            continue;
        }
        let len = s1 - s0;
        let bays = if len < 1.9 { 0 } else { ((len / FACADE_TEST_BAY_M).round() as u32).clamp(1, 120) };
        for k in 0..4 {
            let u = if s[k] - s0 > len * 0.5 { bays } else { 0 };
            let v = if p[k][1] - y0 > (y1 - y0) * 0.5 { storeys } else { 0 };
            let nrm = &mut verts[b + k].normal;
            *nrm = (*nrm & 0x00ff_00ff) | (v << 8) | (u << 24);
        }
        top = Some(top.map_or(y1 as f32, |t| t.max(y1 as f32)));
    }
    top
}

pub const RUNG_NEAR: u8 = 0;
pub const RUNG_MID: u8 = 2;
pub const CACHE_MAGIC: &[u8; 4] = b"EOKT";
pub const CACHE_VERSION: u32 = 3; // + the packed appearance per building; + the glass window frames
/// Bytes of a frame's upload budget kept for the kit's own small writes
/// (LOD-group readiness, emptied object entries on eviction).
pub const KIT_FRAME_RESERVE_BYTES: u64 = 256 << 10;
/// Packs evicted per frame at most (bounds the eviction writes).
pub const EVICTIONS_PER_FRAME: usize = 4;

/// Streaming radii (camera to the tile's footprint rectangle, metres).
#[derive(Clone, Copy, Debug)]
pub struct KitRadii {
    pub near_load: f32,
    pub near_drop: f32,
    pub mid_load: f32,
    pub mid_drop: f32,
}

impl Default for KitRadii {
    fn default() -> Self {
        // A tile is loaded before any of its buildings can enter the band
        // (buildings stand inside their tile's rectangle) and dropped once
        // the camera is a clear margin past it.
        Self { near_load: 110.0, near_drop: 170.0, mid_load: 520.0, mid_drop: 600.0 }
    }
}

pub struct KitTile {
    pub key: (i32, i32),
    /// City building indices, in group order.
    pub buildings: Vec<u32>,
    /// First group of this tile (relative to the kit groups).
    pub group_first: u32,
    pub min: [f32; 2],
    pub max: [f32; 2],
}

/// Everything about the kit buildings known before any is planned.
pub struct KitSet {
    pub tiles: Vec<KitTile>,
    pub tile_of_key: HashMap<(i32, i32), u32>,
    /// Per city building: its LOD group (absolute), or NO_GROUP.
    pub building_group: Vec<u32>,
    /// Absolute index of the first kit group (after the Blender groups).
    pub group_base: u32,
    pub groups: Vec<GpuLodGroup>,
    /// Per city building: the footprint box's base (lowest ground under its ring).
    pub base_y: Vec<f32>,
    pub palette_base: u32,
    pub palette: Vec<String>,
    pub palette_index: HashMap<String, u8>,
    pub buildings: usize,
    pub excluded: usize,
    /// Slice 4: the street world every floor stands on (None: slice 3's lowest-ground base, no skirt).
    pub streets: Option<Arc<eo_street::StreetWorld>>,
}

fn tile_key(x: f64, z: f64) -> (i32, i32) {
    ((x / KIT_TILE_M).floor() as i32, (z / KIT_TILE_M).floor() as i32)
}

impl KitSet {
    /// `excluded`: building ids drawn by something else (Blender models, the
    /// authored registry). `group_base`: the number of groups before these.
    pub fn prepare(
        city: &eo_data::City,
        elevation: &eo_data::ElevationGrid,
        excluded: &HashSet<String>,
        group_base: u32,
        materials: &mut MaterialTable,
        kit: &eo_assets::kit::KitLibrary,
    ) -> Self {
        let (palette_base, palette) = materials.register_kit_palette(kit);
        let palette_index = palette.iter().enumerate().map(|(i, n)| (n.clone(), i as u8)).collect();
        let mut base_y = vec![0f32; city.buildings.len()];
        let mut by_tile: BTreeMap<(i32, i32), Vec<u32>> = BTreeMap::new();
        let mut n_excluded = 0;
        let mut boxes: Vec<([f32; 3], [f32; 3])> = vec![([0.0; 3], [0.0; 3]); city.buildings.len()];
        for (i, b) in city.buildings.iter().enumerate() {
            let base = b.ring.iter().map(|p| elevation.ground(p[0] as f64, p[1] as f64)).fold(f64::MAX, f64::min) as f32;
            base_y[i] = base;
            if excluded.contains(&b.id) {
                n_excluded += 1;
                continue;
            }
            let (mut mn, mut mx) = ([f32::MAX, base, f32::MAX], [f32::MIN, base + b.height, f32::MIN]);
            for p in &b.ring {
                mn[0] = mn[0].min(p[0]);
                mn[2] = mn[2].min(p[1]);
                mx[0] = mx[0].max(p[0]);
                mx[2] = mx[2].max(p[1]);
            }
            boxes[i] = (mn, mx);
            let (cx, cz) = b.ring.iter().fold((0.0f64, 0.0f64), |a, p| (a.0 + p[0] as f64, a.1 + p[1] as f64));
            let n = b.ring.len() as f64;
            by_tile.entry(tile_key(cx / n, cz / n)).or_default().push(i as u32);
        }
        let mut building_group = vec![NO_GROUP; city.buildings.len()];
        let mut groups = vec![];
        let mut tiles = vec![];
        let mut tile_of_key = HashMap::new();
        for (key, list) in by_tile {
            let group_first = groups.len() as u32;
            let (mut tmn, mut tmx) = ([f32::MAX; 2], [f32::MIN; 2]);
            for &b in &list {
                let (mn, mx) = boxes[b as usize];
                tmn = [tmn[0].min(mn[0]), tmn[1].min(mn[2])];
                tmx = [tmx[0].max(mx[0]), tmx[1].max(mx[2])];
                let c = [(mn[0] + mx[0]) * 0.5, (mn[1] + mx[1]) * 0.5, (mn[2] + mx[2]) * 0.5];
                let r = 0.5 * Vec3::from(mx).distance(Vec3::from(mn));
                building_group[b as usize] = group_base + groups.len() as u32;
                // Start in the far zone with the box drawn.
                groups.push(GpuLodGroup { center: c, radius: r, ready: 0, detailed: 2, kind: GROUP_KIND_KIT, level: 2 });
            }
            tile_of_key.insert(key, tiles.len() as u32);
            tiles.push(KitTile { key, buildings: list, group_first, min: tmn, max: tmx });
        }
        let buildings = groups.len();
        Self { tiles, tile_of_key, building_group, group_base, groups, base_y, palette_base, palette, palette_index, buildings, excluded: n_excluded, streets: None }
    }

    /// Camera to the tile's footprint rectangle, in the ground plane.
    pub fn tile_distance(&self, t: usize, cam: Vec3) -> f32 {
        let tile = &self.tiles[t];
        let dx = (tile.min[0] - cam.x).max(0.0).max(cam.x - tile.max[0]);
        let dz = (tile.min[1] - cam.z).max(0.0).max(cam.z - tile.max[1]);
        (dx * dx + dz * dz).sqrt()
    }

    /// every tile's height range (its buildings' bounding spheres).
    pub fn tile_heights(&self) -> Vec<[f32; 2]> {
        self.tiles
            .iter()
            .map(|t| {
                let gs = &self.groups[t.group_first as usize..t.group_first as usize + t.buildings.len()];
                let lo = gs.iter().map(|g| g.center[1] - g.radius).fold(f32::MAX, f32::min);
                let hi = gs.iter().map(|g| g.center[1] + g.radius).fold(f32::MIN, f32::max);
                if lo > hi { [0.0, 0.0] } else { [lo, hi] }
            })
            .collect()
    }

    /// the tile's box (footprint rectangle x `heights`).
    pub fn tile_box(&self, t: usize, heights: &[[f32; 2]]) -> (Vec3, Vec3) {
        let (tile, y) = (&self.tiles[t], heights.get(t).copied().unwrap_or([0.0, 0.0]));
        (Vec3::new(tile.min[0], y[0], tile.min[1]), Vec3::new(tile.max[0], y[1], tile.max[1]))
    }
}

// ---------------------------------------------------------------------------
// Packs: one tile, one rung, cooked.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PackBuilding {
    /// City building index.
    pub building: u32,
    pub vertex_start: u32,
    pub vertex_count: u32,
    pub index_start: u32,
    pub index_count: u32,
    pub min: [f32; 3],
    pub max: [f32; 3],
    /// Index sub-ranges (relative to the pack) per palette entry.
    pub splits: Vec<(u32, u32, u8)>,
    /// the packed appearance (eo-kit appearance.rs, the JS `archetypeAge`).
    pub age: [f32; 4],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pack {
    pub tile: u32,
    pub rung: u8,
    pub vertices: Vec<Vertex>,
    /// Indices local to each building's first vertex.
    pub indices: Vec<u32>,
    pub buildings: Vec<PackBuilding>,
}

impl Pack {
    pub fn bytes(&self) -> usize {
        self.vertices.len() * std::mem::size_of::<Vertex>() + self.indices.len() * 4
    }
    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(self.bytes() + self.buildings.len() * 64 + 64);
        let u32le = |b: &mut Vec<u8>, v: u32| b.extend_from_slice(&v.to_le_bytes());
        b.extend_from_slice(CACHE_MAGIC);
        u32le(&mut b, CACHE_VERSION);
        u32le(&mut b, self.tile);
        u32le(&mut b, self.rung as u32);
        u32le(&mut b, self.vertices.len() as u32);
        u32le(&mut b, self.indices.len() as u32);
        u32le(&mut b, self.buildings.len() as u32);
        b.extend_from_slice(bytemuck::cast_slice(&self.vertices));
        b.extend_from_slice(bytemuck::cast_slice(&self.indices));
        for pb in &self.buildings {
            for v in [pb.building, pb.vertex_start, pb.vertex_count, pb.index_start, pb.index_count] {
                u32le(&mut b, v);
            }
            for v in pb.min.iter().chain(&pb.max).chain(&pb.age) {
                b.extend_from_slice(&v.to_le_bytes());
            }
            u32le(&mut b, pb.splits.len() as u32);
            for (s, c, m) in &pb.splits {
                u32le(&mut b, *s);
                u32le(&mut b, *c);
                u32le(&mut b, *m as u32);
            }
        }
        let digest = Sha256::digest(&b);
        b.extend_from_slice(&digest);
        b
    }

    pub fn decode(b: &[u8]) -> Result<Self, String> {
        if b.len() < 60 || &b[0..4] != CACHE_MAGIC {
            return Err("not a kit tile".into());
        }
        let (body, digest) = b.split_at(b.len() - 32);
        if Sha256::digest(body).as_slice() != digest {
            return Err("payload digest mismatch".into());
        }
        let mut at = 4;
        let mut u = || {
            let v = u32::from_le_bytes(body[at..at + 4].try_into().unwrap());
            at += 4;
            v
        };
        if u() != CACHE_VERSION {
            return Err("cache version".into());
        }
        let tile = u();
        let rung = u() as u8;
        let nv = u() as usize;
        let ni = u() as usize;
        let nb = u() as usize;
        let mut at = 28;
        let vb = nv * std::mem::size_of::<Vertex>();
        if body.len() < at + vb + ni * 4 {
            return Err("truncated".into());
        }
        let vertices: Vec<Vertex> = bytemuck::pod_collect_to_vec(&body[at..at + vb]);
        at += vb;
        let indices: Vec<u32> = bytemuck::pod_collect_to_vec(&body[at..at + ni * 4]);
        at += ni * 4;
        let rd = |at: &mut usize| -> Result<u32, String> {
            let s = body.get(*at..*at + 4).ok_or("truncated")?;
            *at += 4;
            Ok(u32::from_le_bytes(s.try_into().unwrap()))
        };
        let mut buildings = Vec::with_capacity(nb);
        for _ in 0..nb {
            let mut pb = PackBuilding {
                building: rd(&mut at)?,
                vertex_start: rd(&mut at)?,
                vertex_count: rd(&mut at)?,
                index_start: rd(&mut at)?,
                index_count: rd(&mut at)?,
                ..Default::default()
            };
            for k in 0..10 {
                let v = f32::from_bits(rd(&mut at)?);
                if k < 3 {
                    pb.min[k] = v;
                } else if k < 6 {
                    pb.max[k - 3] = v;
                } else {
                    pb.age[k - 6] = v; // facade
                }
            }
            let ns = rd(&mut at)?;
            for _ in 0..ns {
                pb.splits.push((rd(&mut at)?, rd(&mut at)?, rd(&mut at)? as u8));
            }
            buildings.push(pb);
        }
        if at != body.len() {
            return Err("trailing bytes".into());
        }
        Ok(Pack { tile, rung, vertices, indices, buildings })
    }
}

/// What a cook needs: the planner's city, the set, and where to cache.
pub struct CookContext {
    pub city: Arc<KitCity>,
    pub set: Arc<KitSet>,
    pub cache_dir: Option<PathBuf>,
    /// Digest of the inputs (planner build, data files, palette): names the cache folder.
    pub inputs_key: String,
}

/// Where a kit building's floor stands and how far its skirt goes down
/// (eo-street grounding: the street-front floor and the archetype foundation).
pub fn kit_floor(kc: &KitCity, streets: &eo_street::StreetWorld, b: u32) -> (eo_street::grounding::Floor, eo_street::grounding::Foundation, Option<eo_street::grounding::FrontEdge>) {
    let rec = &kc.records[b as usize];
    let ring: Vec<[f64; 2]> = rec.polygon.iter().map(|p| [p.x, p.z]).collect();
    let front = eo_kit::pipeline::classify_loaded(rec)
        .and_then(|c| eo_kit::pipeline::resolve_frame(rec, &c, &kc.world, b))
        .and_then(|(frame, front, _)| {
            if front.source != "street" && front.source != "service" {
                return None;
            }
            frame.edges.iter().find(|e| e.index == front.index).map(|e| eo_street::grounding::FrontEdge {
                start: [e.start.x, e.start.z],
                u: [e.u.x, e.u.z],
                n: [e.n.x, e.n.z],
                length: e.length,
            })
        });
    let floor = streets.street_front_floor(&ring, front.as_ref());
    let foundation = streets.foundation(&ring, floor.y, streets.o.archetype_foundation_skirt_m);
    (floor, foundation, front)
}

/// a cheap estimate of a kit building's street-front floor for its
/// far box (no plan, no street mesh: the analytic street corridors only).
/// From the middle of each footprint edge, step outward until a street's paved
/// corridor holds the point; the longest such edge is taken as the street face
/// and its corridor's walk level (terrain at the run's centreline +
/// footwayLift, `walk_level_beside`) as the floor. Without a street face: the
/// lowest ground under the ring (the box's old base). Measured against the
/// real floor by `examples/far_floor_probe.rs`.
pub fn far_floor_estimate(sw: &eo_street::StreetWorld, ring: &[[f64; 2]]) -> f64 {
    let n = ring.len();
    let lowest = ring.iter().map(|p| sw.ground(p[0], p[1])).fold(f64::MAX, f64::min);
    if n < 3 {
        return lowest;
    }
    let c = ring.iter().fold([0.0, 0.0], |a, p| [a[0] + p[0], a[1] + p[1]]);
    let c = [c[0] / n as f64, c[1] / n as f64];
    let mut best: Option<(f64, f64)> = None;
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * dx + dz * dz).sqrt();
        if len < 2.0 {
            continue;
        }
        let m = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        let mut nrm = [dz / len, -dx / len];
        if (m[0] - c[0]) * nrm[0] + (m[1] - c[1]) * nrm[1] < 0.0 {
            nrm = [-nrm[0], -nrm[1]];
        }
        let mut d = 0.3;
        while d <= 6.0 {
            if let Some(y) = sw.walk_level_beside(m[0] + nrm[0] * d, m[1] + nrm[1] * d, None) {
                if best.is_none_or(|bb| len > bb.0) {
                    best = Some((len, y));
                }
                break;
            }
            d += 0.5;
        }
    }
    best.map(|b| b.1).unwrap_or(lowest)
}

#[derive(Clone, Debug, Default)]
pub struct CookStats {
    pub buildings: usize,
    pub unplanned: usize,
    pub unknown_materials: BTreeMap<String, usize>,
}

/// Cook one tile: plan every kit building, mesh rungs 0 and 2.
pub fn cook_tile(ctx: &CookContext, tile: u32) -> ([Pack; 2], CookStats) {
    let set = &ctx.set;
    let kc = &ctx.city;
    let t = &set.tiles[tile as usize];
    let mut packs = [Pack { tile, rung: RUNG_NEAR, ..Default::default() }, Pack { tile, rung: RUNG_MID, ..Default::default() }];
    let mut st = CookStats::default();
    let fallback = *set.palette_index.get("painted-stucco").unwrap_or(&0);
    let skirt_pal = *set.palette_index.get("concrete-precast").unwrap_or(&fallback);
    for &b in &t.buildings {
        let i = b as usize;
        let (base_y, skirt) = match &set.streets {
            Some(sw) => {
                let (floor, f, _) = kit_floor(kc, sw, b);
                (floor.y, Some(f.bottom_y))
            }
            None => (set.base_y[i] as f64, None),
        };
        let planned = eo_kit::pipeline::plan_building(&kc.records[i], &kc.world, b, base_y);
        let Some(planned) = planned else {
            st.unplanned += 1;
            continue;
        };
        st.buildings += 1;
        // the building's appearance, packed as the JS tile stream packs it.
        let age = eo_kit::appearance::planned_appearance(&kc.records[i], &planned, base_y);
        let ft = FACADE_TEST.load(Ordering::Relaxed);
        for (k, rung) in [if ft != 0 { 3u32 } else { 0 }, 2].into_iter().enumerate() {
            let mesh = eo_kit::pipeline::build_rung(&planned, rung);
            let pack = &mut packs[k];
            let mut ft_top: Option<f32> = None; // the envelope walls' top
            let vertex_start = pack.vertices.len() as u32;
            let index_start = pack.indices.len() as u32;
            let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
            let mut splits = vec![];
            for (name, tint, sink) in &mesh.groups {
                let base = eo_kit::pipeline::base_material_name(name);
                let pal = match set.palette_index.get(base) {
                    Some(p) => *p,
                    None => {
                        *st.unknown_materials.entry(base.to_string()).or_default() += 1;
                        fallback
                    }
                };
                let rgb = tint.unwrap_or(0xffffff);
                let color = ((rgb >> 16) & 255) | (((rgb >> 8) & 255) << 8) | ((rgb & 255) << 16) | ((pal as u32) << 24);
                let first_vertex = pack.vertices.len() as u32 - vertex_start;
                let split_start = pack.indices.len() as u32;
                // glass that draws a room carries its window frame in the
                // bytes it does not use (rooms.rs; bakeNightWindowUv).
                let frames = (crate::rooms::kit_glass_room(base) != 0).then(|| {
                    let r = &kc.records[i];
                    crate::rooms::bake(&sink.positions, &sink.normals, &sink.indices, Some(base_y), crate::rooms::night_use(&r.amenity, r.program, r.usage))
                });
                for (k, (p, n)) in sink.positions.iter().zip(&sink.normals).enumerate() {
                    let pos = [p[0] as f32, p[1] as f32, p[2] as f32];
                    for a in 0..3 {
                        mn[a] = mn[a].min(pos[a]);
                        mx[a] = mx[a].max(pos[a]);
                    }
                    let mut normal = eo_assets::glb::pack_normal(Vec3::from(*n));
                    let mut c = color;
                    if let Some(f) = &frames {
                        let (rgb, nw) = f[k];
                        c = rgb[0] as u32 | ((rgb[1] as u32) << 8) | ((rgb[2] as u32) << 16) | ((pal as u32) << 24);
                        normal |= (nw as u32) << 24;
                    }
                    pack.vertices.push(Vertex { pos, normal, color: c });
                }
                if ft == 2 && k == 0 {
                    // bay and storey counts on the envelope's walls.
                    let at = (vertex_start + first_vertex) as usize;
                    let plan = &planned.plan;
                    if let Some(t) = facade_test_mark(sink, &mut pack.vertices[at..], base_y, plan.envelope.roof_top, plan.storeys.len()) {
                        ft_top = Some(ft_top.map_or(t, |a| a.max(t)));
                    }
                }
                pack.indices.extend(sink.indices.iter().map(|ix| ix + first_vertex));
                let count = pack.indices.len() as u32 - split_start;
                if count > 0 {
                    splits.push((split_start, count, pal));
                }
            }
            // The foundation skirt: the outline walled from the floor down to
            // below the lowest ground round it (planBuildingFoundation).
            if let Some(bottom) = skirt {
                let ring: Vec<[f32; 2]> = kc.records[i].polygon.iter().map(|p| [p.x as f32, p.z as f32]).collect();
                let color = 0xd8d8d8 | ((skirt_pal as u32) << 24);
                let mut sv = vec![];
                let mut si = vec![];
                crate::scene::skirt_quads(&ring, base_y as f32, bottom as f32, color, &mut sv, &mut si);
                if !si.is_empty() {
                    let first_vertex = pack.vertices.len() as u32 - vertex_start;
                    let split_start = pack.indices.len() as u32;
                    for v in &sv {
                        for a in 0..3 {
                            mn[a] = mn[a].min(v.pos[a]);
                            mx[a] = mx[a].max(v.pos[a]);
                        }
                    }
                    pack.vertices.extend(sv);
                    pack.indices.extend(si.iter().map(|ix| ix + first_vertex));
                    splits.push((split_start, pack.indices.len() as u32 - split_start, skirt_pal));
                }
            }
            let index_count = pack.indices.len() as u32 - index_start;
            if index_count == 0 {
                continue;
            }
            if let Some(t) = ft_top {
                // the shader reads the wall top from the object's box (a roof bulkhead above it is left out).
                mx[1] = t;
            }
            pack.buildings.push(PackBuilding {
                building: b,
                vertex_start,
                vertex_count: pack.vertices.len() as u32 - vertex_start,
                index_start,
                index_count,
                min: mn,
                max: mx,
                splits,
                age,
            });
        }
    }
    (packs, st)
}

pub fn cache_path(dir: &Path, inputs_key: &str, tile: &KitTile, rung: u8) -> PathBuf {
    dir.join("kit-tiles").join(inputs_key).join(format!("{}_{}_r{}.eokt", tile.key.0, tile.key.1, rung))
}

/// The cache folder name: planner build, input data and palette.
pub fn inputs_key(repo_root: &Path, palette: &[String], streets: bool) -> String {
    let mut h = Sha256::new();
    h.update(eo_kit::KIT_PORT_VERSION.as_bytes());
    h.update(eo_kit::KIT_SOURCE_HASH.as_bytes());
    h.update(CACHE_VERSION.to_le_bytes()); // a pack format change cooks into a new folder
    match FACADE_TEST.load(Ordering::Relaxed) {
        // its packs cook into their own folders (off: the key is unchanged).
        1 => h.update(b"facade-test-box-v1"),
        2 => h.update(b"facade-test-shader-v1"),
        _ => {}
    }
    if streets {
        // Slice 4: floors and skirts stand on the street surface.
        h.update(eo_street::STREET_PORT_VERSION.as_bytes());
        h.update(eo_street::STREET_SOURCE_HASH.as_bytes());
    }
    h.update(KIT_TILE_M.to_le_bytes());
    // a scaled ground plan cooks into its own folder (scale and plan
    // version; at scale 1 the key is unchanged).
    let tag = eo_data::groundscale::cache_tag();
    if !tag.is_empty() {
        h.update(tag.as_bytes());
    }
    // measured roof colours cook into their own folder (no table: the key is unchanged).
    let roofs = eo_data::roof_colours::cache_tag();
    if !roofs.is_empty() {
        h.update(roofs.as_bytes());
    }
    // the wall paint cooks into its own folder (off: the key is unchanged).
    let paint = eo_kit::wall_paint::cache_tag();
    if !paint.is_empty() {
        h.update(paint.as_bytes());
    }
    for p in palette {
        h.update(p.as_bytes());
        h.update([0u8]);
    }
    for f in [
        "public/data/sf/sf-city.json.gz",
        "public/data/sf/sf-footprints.json.gz",
        "public/data/sf/sf-footprint-supplement.json",
        "public/data/sf/sf-elevation.bin.gz",
        "native/eo-kit/data/district-era-prior.data.js",
        "public/data/sf/building-metadata/v1/index.json",
        "public/data/sf/building-metadata/v1-pack/index.json",
    ] {
        h.update(f.as_bytes());
        h.update(std::fs::read(repo_root.join(f)).unwrap_or_default());
    }
    let d = h.finalize();
    d.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackSource {
    Cache,
    Cooked,
}

pub struct PackArrival {
    pub pack: Pack,
    pub source: PackSource,
    pub ms: f64,
    pub stats: CookStats,
    /// The pack's vertices and indices as upload bytes, made on the worker
    /// (`into_upload`) so the render thread copies nothing.
    pub vbytes: Vec<u8>,
    pub ibytes: Vec<u8>,
    pub nv: u32,
    pub ni: u32,
}

impl PackArrival {
    /// Move the geometry into upload bytes (worker thread).
    pub fn into_upload(mut self) -> Self {
        self.nv = self.pack.vertices.len() as u32;
        self.ni = self.pack.indices.len() as u32;
        self.vbytes = bytemuck::cast_slice(&self.pack.vertices).to_vec();
        self.ibytes = bytemuck::cast_slice(&self.pack.indices).to_vec();
        self.pack.vertices = vec![];
        self.pack.indices = vec![];
        self
    }
}

/// Load a pack from the cache, or cook its tile (both rungs) and cache both.
pub fn load_pack(ctx: &CookContext, tile: u32, rung: u8) -> PackArrival {
    let t0 = Instant::now();
    let t = &ctx.set.tiles[tile as usize];
    if let Some(dir) = &ctx.cache_dir {
        if let Ok(bytes) = std::fs::read(cache_path(dir, &ctx.inputs_key, t, rung)) {
            if let Ok(pack) = Pack::decode(&bytes) {
                if pack.tile == tile && pack.rung == rung {
                    return PackArrival { pack, source: PackSource::Cache, ms: t0.elapsed().as_secs_f64() * 1e3, stats: CookStats::default(), vbytes: vec![], ibytes: vec![], nv: 0, ni: 0 };
                }
            }
        }
    }
    let (packs, stats) = cook_tile(ctx, tile);
    if let Some(dir) = &ctx.cache_dir {
        for p in &packs {
            let path = cache_path(dir, &ctx.inputs_key, t, p.rung);
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let tmp = path.with_extension("tmp");
            if std::fs::write(&tmp, p.encode()).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }
    let [near, mid] = packs;
    let pack = if rung == RUNG_NEAR { near } else { mid };
    PackArrival { pack, source: PackSource::Cooked, ms: t0.elapsed().as_secs_f64() * 1e3, stats, vbytes: vec![], ibytes: vec![], nv: 0, ni: 0 }
}

// ---------------------------------------------------------------------------
// Arena allocation.
// ---------------------------------------------------------------------------

/// First-fit range allocator with coalescing free list.
#[derive(Clone, Debug)]
pub struct RangeAlloc {
    free: BTreeMap<u32, u32>,
    pub capacity: u32,
    pub used: u32,
    pub peak: u32,
}

impl RangeAlloc {
    pub fn new(capacity: u32) -> Self {
        let mut free = BTreeMap::new();
        if capacity > 0 {
            free.insert(0, capacity);
        }
        Self { free, capacity, used: 0, peak: 0 }
    }
    pub fn alloc(&mut self, n: u32) -> Option<u32> {
        if n == 0 {
            return Some(0);
        }
        let (&at, &len) = self.free.iter().find(|(_, len)| **len >= n)?;
        self.free.remove(&at);
        if len > n {
            self.free.insert(at + n, len - n);
        }
        self.used += n;
        self.peak = self.peak.max(self.used);
        Some(at)
    }
    pub fn free(&mut self, at: u32, n: u32) {
        if n == 0 {
            return;
        }
        self.used -= n;
        let mut start = at;
        let mut len = n;
        if let Some((&p, &pl)) = self.free.range(..at).next_back() {
            if p + pl == at {
                self.free.remove(&p);
                start = p;
                len += pl;
            }
        }
        if let Some(&nl) = self.free.get(&(at + n)) {
            self.free.remove(&(at + n));
            len += nl;
        }
        self.free.insert(start, len);
    }
}

/// Where the kit arenas start in the shared buffers, and their capacities.
#[derive(Clone, Copy, Debug, Default)]
pub struct KitArena {
    pub vertex_base: u32,
    pub vertex_capacity: u32,
    pub index_base: u32,
    pub index_capacity: u32,
    pub object_base: u32,
    pub object_capacity: u32,
}

// ---------------------------------------------------------------------------
// Runtime: requests, arrivals, uploads, eviction.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct Alloc {
    vertex: u32,
    vertices: u32,
    index: u32,
    indices: u32,
    object: u32,
    objects: u32,
    /// allocated from the near packs' own part of the arena.
    near_pool: bool,
}

enum Slot {
    Requested,
    Uploading(Alloc),
    Resident(Alloc),
    /// arrived while its part of the arena was full; asked for
    /// again once that part frees space (instead of every frame).
    Deferred,
}

/// the near packs' part of the kit arena (`split_near`): ranges
/// relative to `base` (vertices, indices, objects from the arena's start).
struct NearPool {
    vtx: RangeAlloc,
    idx: RangeAlloc,
    obj: RangeAlloc,
    base: [u32; 3],
}

pub const TAG_KIT: u64 = 1 << 62;

fn tag_of(tile: u32, rung: u8) -> u64 {
    TAG_KIT | ((tile as u64) << 8) | rung as u64
}

#[derive(Clone, Debug, Default)]
pub struct KitStats {
    pub packs_from_cache: usize,
    pub packs_cooked: usize,
    pub packs_uploaded: usize,
    pub packs_evicted: usize,
    pub packs_dropped_unneeded: usize,
    pub packs_skipped_arena_full: usize,
    pub worker_ms: f64,
    pub cooked_buildings: usize,
    pub unplanned: usize,
    pub unknown_materials: BTreeMap<String, usize>,
    pub resident_near: usize,
    pub resident_mid: usize,
    pub resident_triangles: u64,
    pub resident_vertex_bytes: u64,
    pub resident_index_bytes: u64,
    pub group_bytes_written: u64,
    pub city_ready_at_ms: Option<f64>,
    /// arena use by rung (near, mid): vertices, indices, objects
    /// resident now and the most at once.
    pub rung_use: [[u64; 3]; 2],
    pub rung_peak: [[u64; 3]; 2],
    /// packs that arrived while their part of the arena was full.
    pub packs_deferred: usize,
}

pub struct KitRuntime {
    pub set: Arc<KitSet>,
    pub arena: KitArena,
    pub radii: KitRadii,
    pub split_materials: bool,
    vtx: RangeAlloc,
    idx: RangeAlloc,
    obj: RangeAlloc,
    slots: HashMap<(u32, u8), Slot>,
    mirror: Vec<GpuLodGroup>,
    pending: Arc<Mutex<Vec<(u32, u8)>>>,
    /// requests and cancellations not yet applied to `pending`.
    outbox: crate::hitch::Outbox<(u32, u8)>,
    camera: Arc<Mutex<Vec3>>,
    rx: Receiver<PackArrival>,
    stop: Arc<AtomicBool>,
    in_flight: Arc<AtomicUsize>,
    threads: Vec<std::thread::JoinHandle<()>>,
    started: Instant,
    pub stats: KitStats,
    /// Group bytes written by the last `after_pump` (counted with the frame's uploads).
    pub last_group_bytes: u64,
    pub updates: u64,
    /// the widened view the workers order requests by, every
    /// tile's height range, the near packs' hysteresis, and how many near
    /// packs inside their radius the last `update` left out as out of view.
    view: Arc<Mutex<Option<crate::view_detail::ViewCone>>>,
    tile_y: Arc<Vec<[f32; 2]>>,
    hold: crate::view_detail::ViewHold<u32>,
    pub near_out_of_view: usize,
    /// resident packs the last `update` found no longer wanted (evicted
    /// EVICTIONS_PER_FRAME a frame): what an earlier camera left behind.
    pub stale_resident: usize,
    /// the near packs' own part of the arena (None: one arena for
    /// both rungs, as before), and whether a part freed space since a pack was
    /// deferred (near, mid).
    near_pool: Option<NearPool>,
    freed: [bool; 2],
}

impl KitRuntime {
    /// give the near packs (rung 0) their own part of the arena,
    /// at its end: `vertices`, `indices` and `objects` of the capacities. The
    /// mid packs (the radial layer) keep the rest, so a full near part can
    /// never keep a mid pack, and with it the stand-in, out. Call before
    /// anything is allocated.
    pub fn split_near(&mut self, vertices: u32, indices: u32, objects: u32) {
        assert!(self.vtx.used == 0 && self.idx.used == 0 && self.obj.used == 0, "split_near before the first pack");
        let a = self.arena;
        let (v, i, o) = (vertices.min(a.vertex_capacity / 2), indices.min(a.index_capacity / 2), objects.min(a.object_capacity / 2));
        self.vtx = RangeAlloc::new(a.vertex_capacity - v);
        self.idx = RangeAlloc::new(a.index_capacity - i);
        self.obj = RangeAlloc::new(a.object_capacity - o);
        self.near_pool = Some(NearPool {
            vtx: RangeAlloc::new(v),
            idx: RangeAlloc::new(i),
            obj: RangeAlloc::new(o),
            base: [a.vertex_capacity - v, a.index_capacity - i, a.object_capacity - o],
        });
    }

    /// the near part's capacities, if split (vertices, indices, objects).
    pub fn near_pool_capacity(&self) -> Option<[u32; 3]> {
        self.near_pool.as_ref().map(|p| [p.vtx.capacity, p.idx.capacity, p.obj.capacity])
    }

    /// Start the workers. `city_rx` delivers the planner's city (built off the
    /// render thread at load); nothing is requested until it arrives.
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        set: Arc<KitSet>,
        city_rx: Receiver<Result<(Arc<KitCity>, String), String>>,
        cache_dir: Option<PathBuf>,
        arena: KitArena,
        radii: KitRadii,
        threads: usize,
        camera: Vec3,
        split_materials: bool,
        started: Instant,
    ) -> Self {
        let (tx, rx): (Sender<PackArrival>, Receiver<PackArrival>) = std::sync::mpsc::channel();
        let pending: Arc<Mutex<Vec<(u32, u8)>>> = Arc::new(Mutex::new(vec![]));
        let cam = Arc::new(Mutex::new(camera));
        let stop = Arc::new(AtomicBool::new(false));
        let in_flight = Arc::new(AtomicUsize::new(0));
        let ctx_slot: Arc<Mutex<Option<Arc<CookContext>>>> = Arc::new(Mutex::new(None));
        let view: Arc<Mutex<Option<crate::view_detail::ViewCone>>> = Arc::new(Mutex::new(None));
        let tile_y = Arc::new(set.tile_heights());
        // One thread waits for the city, then hands every worker the context.
        {
            let (ctx_slot, set, cache_dir) = (ctx_slot.clone(), set.clone(), cache_dir.clone());
            std::thread::spawn(move || {
                if let Ok(Ok((city, key))) = city_rx.recv() {
                    *ctx_slot.lock().unwrap() = Some(Arc::new(CookContext { city, set, cache_dir, inputs_key: key }));
                }
            });
        }
        let mut handles = vec![];
        for _ in 0..threads.max(1) {
            let (tx, pending, cam, stop, in_flight, ctx_slot, set) =
                (tx.clone(), pending.clone(), cam.clone(), stop.clone(), in_flight.clone(), ctx_slot.clone(), set.clone());
            let (view, tile_y) = (view.clone(), tile_y.clone());
            handles.push(std::thread::spawn(move || {
                crate::hitch::worker_started(); // below the render thread
                while !stop.load(Ordering::Relaxed) {
                    let ctx = ctx_slot.lock().unwrap().clone();
                    let Some(ctx) = ctx else {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                        continue;
                    };
                    let next = {
                        let c = *cam.lock().unwrap();
                        let v = *view.lock().unwrap();
                        let mut p = pending.lock().unwrap();
                        // in view first by screen importance, then nearest (no view: nearest, as before).
                        let best = p
                            .iter()
                            .enumerate()
                            .map(|(k, &(t, r))| {
                                let (mn, mx) = set.tile_box(t as usize, &tile_y);
                                (k, crate::view_detail::rank(v.as_ref(), mn, mx, set.tile_distance(t as usize, c)) + if r == RUNG_NEAR { 0.0 } else { 40.0 })
                            })
                            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
                        best.map(|(k, _)| {
                            in_flight.fetch_add(1, Ordering::SeqCst);
                            p.swap_remove(k)
                        })
                    };
                    let Some((tile, rung)) = next else {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        continue;
                    };
                    let arrival = load_pack(&ctx, tile, rung).into_upload();
                    in_flight.fetch_sub(1, Ordering::SeqCst);
                    if tx.send(arrival).is_err() {
                        break;
                    }
                }
            }));
        }
        let mirror = set.groups.clone();
        Self {
            vtx: RangeAlloc::new(arena.vertex_capacity),
            idx: RangeAlloc::new(arena.index_capacity),
            obj: RangeAlloc::new(arena.object_capacity),
            set,
            arena,
            radii,
            split_materials,
            slots: HashMap::new(),
            mirror,
            pending,
            outbox: Default::default(),
            camera: cam,
            rx,
            stop,
            in_flight,
            threads: handles,
            started,
            stats: KitStats::default(),
            last_group_bytes: 0,
            updates: 0,
            view,
            tile_y,
            hold: Default::default(),
            near_out_of_view: 0,
            stale_resident: 0,
            near_pool: None,
            freed: [false; 2],
        }
    }

    /// readiness bits of a kit group (relative to the kit groups):
    /// bit 0 the near pack is resident, bit 1 the mid pack (the GPU's copy).
    pub fn group_ready(&self, rel: usize) -> u32 {
        self.mirror.get(rel).map(|g| g.ready).unwrap_or(0)
    }

    /// every tile's height range (for a caller's view tests).
    pub fn tile_heights(&self) -> &[[f32; 2]] {
        &self.tile_y
    }

    /// Requests and evictions for the camera, then arrivals into upload jobs.
    /// Per-tile work only (the tiles around the camera).
    pub fn update(&mut self, renderer: &crate::Renderer, queue: &wgpu::Queue, uploads: &mut UploadQueue, cam: Vec3) {
        self.updates += 1;
        crate::hitch::try_set(&self.camera, cam); // never wait for a worker's lock
        // the mid packs stay radial; a near pack is wanted only while
        // its tile is in the widened view (or was, within the hold).
        let cone = renderer.view_cone();
        crate::hitch::try_set(&self.view, cone);
        self.near_out_of_view = 0;
        // a deferred pack (its part of the arena was full) is asked
        // for again once that part freed space; while one waits, that part is
        // under pressure: near packs out of view are not held, and mid packs
        // past their load radius are not kept.
        let pool_of = |near_pool: bool, rung: u8| if near_pool && rung == RUNG_NEAR { 0 } else { 1 };
        let split = self.near_pool.is_some();
        if self.freed.iter().any(|f| *f) {
            let freed = self.freed;
            self.slots.retain(|k, s| !(matches!(s, Slot::Deferred) && (freed[pool_of(split, k.1)] || !split)));
            self.freed = [false; 2];
        }
        let mut pressure = [false; 2];
        for (k, s) in &self.slots {
            if matches!(s, Slot::Deferred) {
                pressure[pool_of(split, k.1)] = true;
                if !split {
                    pressure = [true; 2];
                }
            }
        }
        let r = self.radii;
        let reach = (r.mid_drop as f64 / KIT_TILE_M).ceil() as i32 + 1;
        let (cx, cz) = tile_key(cam.x as f64, cam.z as f64);
        let mut wanted: HashSet<(u32, u8)> = HashSet::new();
        for tz in cz - reach..=cz + reach {
            for tx in cx - reach..=cx + reach {
                let Some(&t) = self.set.tile_of_key.get(&(tx, tz)) else { continue };
                let d = self.set.tile_distance(t as usize, cam);
                for (rung, load, drop) in [(RUNG_NEAR, r.near_load, r.near_drop), (RUNG_MID, r.mid_load, r.mid_drop)] {
                    let have = self.slots.contains_key(&(t, rung));
                    let under = pressure[pool_of(split, rung)];
                    if d <= load || (have && d <= drop && !(under && rung == RUNG_MID)) {
                        if let (RUNG_NEAR, Some(c)) = (rung, cone.as_ref()) {
                            let (mn, mx) = self.set.tile_box(t as usize, &self.tile_y);
                            let held = self.hold.wanted(c, t, mn, mx);
                            if !held || (under && !c.sees_box(mn, mx)) {
                                self.near_out_of_view += 1;
                                continue;
                            }
                        }
                        wanted.insert((t, rung));
                    }
                }
            }
        }
        if let Some(c) = cone.as_ref() {
            self.hold.prune(c);
        }
        // Evict and cancel what is no longer wanted (resident packs: at most
        // EVICTIONS_PER_FRAME a frame, the rest stay drawn one more frame).
        let mut evictable = 0;
        let gone: Vec<(u32, u8)> = self
            .slots
            .iter()
            .filter(|(k, _)| !wanted.contains(k))
            .filter(|(_, s)| {
                if matches!(s, Slot::Resident(_)) {
                    evictable += 1;
                    evictable <= EVICTIONS_PER_FRAME
                } else {
                    true
                }
            })
            .map(|(k, _)| *k)
            .collect();
        self.stale_resident = evictable;
        if !gone.is_empty() {
            let gone_set: HashSet<(u32, u8)> = gone.iter().copied().collect();
            for k in &gone_set {
                self.outbox.cancel(*k); // applied without waiting for the workers' lock
            }
            uploads.cancel(|tag| {
                if tag & TAG_KIT == 0 {
                    return true;
                }
                !gone_set.contains(&(((tag >> 8) & 0xffff_ffff) as u32, (tag & 0xff) as u8))
            });
            for k in gone {
                match self.slots.remove(&k) {
                    Some(Slot::Resident(a)) => {
                        self.evict(renderer, queue, k, a);
                        self.stats.packs_evicted += 1;
                    }
                    Some(Slot::Uploading(a)) => {
                        self.free(a);
                        self.stats.packs_dropped_unneeded += 1;
                    }
                    _ => {}
                }
            }
        }
        // Request what is wanted and absent.
        for k in &wanted {
            if !self.slots.contains_key(k) {
                self.slots.insert(*k, Slot::Requested);
                self.outbox.request(*k); // hitch
            }
        }
        self.outbox.flush(&self.pending); // try_lock only
        // Arrivals: allocate and queue the upload.
        while let Ok(mut a) = self.rx.try_recv() {
            crate::hitch::count(crate::hitch::Event::Arrival, 1); // hitch
            self.stats.worker_ms += a.ms;
            match a.source {
                PackSource::Cache => self.stats.packs_from_cache += 1,
                PackSource::Cooked => {
                    self.stats.packs_cooked += 1;
                    self.stats.cooked_buildings += a.stats.buildings;
                    self.stats.unplanned += a.stats.unplanned;
                    for (k, v) in &a.stats.unknown_materials {
                        *self.stats.unknown_materials.entry(k.clone()).or_default() += v;
                    }
                }
            }
            if self.stats.city_ready_at_ms.is_none() {
                self.stats.city_ready_at_ms = Some(self.started.elapsed().as_secs_f64() * 1e3);
            }
            let key = (a.pack.tile, a.pack.rung);
            if !matches!(self.slots.get(&key), Some(Slot::Requested)) {
                self.stats.packs_dropped_unneeded += 1;
                crate::upload::bury(Box::new(a)); // a whole pack (MBs) is not freed on the render thread
                continue;
            }
            match self.make_job(&mut a) {
                Some((job, alloc)) => {
                    self.slots.insert(key, Slot::Uploading(alloc));
                    uploads.push(job);
                }
                None => {
                    // Arena full: stays a box (or the other rung); asked for
                    // again once its part of the arena frees space (viewdetail:
                    // deferred, instead of loaded again every frame).
                    self.slots.insert(key, Slot::Deferred);
                    self.stats.packs_skipped_arena_full += 1;
                    self.stats.packs_deferred += 1;
                }
            }
            crate::upload::bury(Box::new(a)); // the pack's buildings (and the bytes of a skipped one) freed off this thread
        }
    }

    fn free(&mut self, a: Alloc) {
        // back to the part it came from; a deferred pack may now fit.
        match (&mut self.near_pool, a.near_pool) {
            (Some(p), true) => {
                p.vtx.free(a.vertex - p.base[0], a.vertices);
                p.idx.free(a.index - p.base[1], a.indices);
                p.obj.free(a.object - p.base[2], a.objects);
                self.freed[0] = true;
            }
            _ => {
                self.vtx.free(a.vertex, a.vertices);
                self.idx.free(a.index, a.indices);
                self.obj.free(a.object, a.objects);
                self.freed[1] = true;
            }
        }
    }

    /// vertex, index and object ranges (from the arena's start)
    /// from the pack's part of the arena, or None when it is full.
    fn alloc_ranges(&mut self, near: bool, nv: u32, ni: u32, no: u32) -> Option<(u32, u32, u32, bool)> {
        let (vtx, idx, obj, base, pooled) = match (&mut self.near_pool, near) {
            (Some(p), true) => (&mut p.vtx, &mut p.idx, &mut p.obj, p.base, true),
            _ => (&mut self.vtx, &mut self.idx, &mut self.obj, [0; 3], false),
        };
        let v = vtx.alloc(nv)?;
        let Some(i) = idx.alloc(ni) else {
            vtx.free(v, nv);
            return None;
        };
        let Some(o) = obj.alloc(no) else {
            vtx.free(v, nv);
            idx.free(i, ni);
            return None;
        };
        Some((v + base[0], i + base[1], o + base[2], pooled))
    }

    fn make_job(&mut self, a: &mut PackArrival) -> Option<(Job, Alloc)> {
        let objects_needed: u32 =
            if self.split_materials { a.pack.buildings.iter().map(|b| b.splits.len() as u32).sum() } else { a.pack.buildings.len() as u32 };
        let (v, i, o, pooled) = self.alloc_ranges(a.pack.rung == RUNG_NEAR, a.nv, a.ni, objects_needed)?;
        let pack = &a.pack;
        let alloc = Alloc { vertex: v, vertices: a.nv, index: i, indices: a.ni, object: o, objects: objects_needed, near_pool: pooled };
        let ar = self.arena;
        let role = if pack.rung == RUNG_NEAR { LOD_KIT_NEAR } else { LOD_KIT_MID };
        let mut objects = Vec::with_capacity(objects_needed as usize);
        for b in &pack.buildings {
            let group = self.set.building_group[b.building as usize];
            let base_vertex = (ar.vertex_base + v + b.vertex_start) as i32;
            let mk = |first: u32, count: u32| {
                let mut ob = GpuObject::new(ObjectKind::Kit, b.min, b.max, ar.index_base + i + first, count, base_vertex);
                ob.material = self.set.palette_base;
                ob.lod_group = group;
                ob.lod_role = role;
                ob
            };
            if self.split_materials {
                for (s, c, _) in &b.splits {
                    objects.push(mk(*s, *c));
                }
            } else {
                objects.push(mk(b.index_start, b.index_count));
            }
        }
        let vsize = std::mem::size_of::<Vertex>() as u64;
        let osize = std::mem::size_of::<GpuObject>() as u64;
        // the buildings' appearance, one contiguous write over the tile's groups.
        let gs: Vec<u32> = pack.buildings.iter().map(|b| self.set.building_group[b.building as usize]).collect();
        let mut finals = vec![Write { target: Target::Object, offset: (ar.object_base + o) as u64 * osize, data: Data::Objects(objects) }];
        if let (Some(&g0), Some(&g1)) = (gs.iter().min(), gs.iter().max()) {
            let mut ages = vec![[0f32, 0.0, 0.0, (512 * 1024 + 512) as f32]; (g1 - g0 + 1) as usize];
            for (b, g) in pack.buildings.iter().zip(&gs) {
                ages[(g - g0) as usize] = b.age;
            }
            finals.push(Write { target: Target::Age, offset: g0 as u64 * 16, data: Data::Bytes(bytemuck::cast_slice(&ages).to_vec()) });
        }
        let job = Job::new(
            tag_of(pack.tile, pack.rung),
            vec![
                Write { target: Target::Vertex, offset: (ar.vertex_base + v) as u64 * vsize, data: Data::Bytes(std::mem::take(&mut a.vbytes)) },
                Write { target: Target::Index, offset: (ar.index_base + i) as u64 * 4, data: Data::Bytes(std::mem::take(&mut a.ibytes)) },
            ],
            finals,
        );
        Some((job, alloc))
    }

    /// Write the tile's groups from the mirror (ready bits changed).
    fn write_groups(&mut self, renderer: &crate::Renderer, queue: &wgpu::Queue, tile: u32) {
        let t = &self.set.tiles[tile as usize];
        let first = t.group_first as usize;
        let n = t.buildings.len();
        let data = bytemuck::cast_slice(&self.mirror[first..first + n]);
        let at = (self.set.group_base as u64 + first as u64) * std::mem::size_of::<GpuLodGroup>() as u64;
        renderer.write(queue, Target::Group, at, data);
        self.last_group_bytes += data.len() as u64;
        self.stats.group_bytes_written += data.len() as u64;
    }

    fn set_ready(&mut self, tile: u32, rung: u8, on: bool) {
        let bit = if rung == RUNG_NEAR { 1 } else { 2 };
        let t = &self.set.tiles[tile as usize];
        for k in 0..t.buildings.len() {
            let g = &mut self.mirror[t.group_first as usize + k];
            if on {
                g.ready |= bit;
            } else {
                g.ready &= !bit;
            }
        }
    }

    fn evict(&mut self, renderer: &crate::Renderer, queue: &wgpu::Queue, key: (u32, u8), a: Alloc) {
        crate::hitch::count(crate::hitch::Event::Eviction, 1); // hitch
        // Not ready first, then the objects emptied; the ranges are reused only
        // by later writes, which the queue orders after the frames in flight.
        self.set_ready(key.0, key.1, false);
        self.write_groups(renderer, queue, key.0);
        let zero = vec![GpuObject::zeroed_kit(); a.objects as usize];
        renderer.write(queue, Target::Object, (self.arena.object_base + a.object) as u64 * std::mem::size_of::<GpuObject>() as u64, bytemuck::cast_slice(&zero));
        self.last_group_bytes += (zero.len() * std::mem::size_of::<GpuObject>()) as u64;
        self.account(a, false, key.1);
        self.free(a);
    }

    fn account(&mut self, a: Alloc, add: bool, rung: u8) {
        let tris = a.indices as u64 / 3;
        let vb = a.vertices as u64 * std::mem::size_of::<Vertex>() as u64;
        let ib = a.indices as u64 * 4;
        // arena use by rung, and the most at once.
        let r = if rung == RUNG_NEAR { 0 } else { 1 };
        for (k, n) in [a.vertices, a.indices, a.objects].into_iter().enumerate() {
            let u = &mut self.stats.rung_use[r][k];
            *u = if add { *u + n as u64 } else { u.saturating_sub(n as u64) };
            self.stats.rung_peak[r][k] = self.stats.rung_peak[r][k].max(*u);
        }
        if add {
            self.stats.resident_triangles += tris;
            self.stats.resident_vertex_bytes += vb;
            self.stats.resident_index_bytes += ib;
            if rung == RUNG_NEAR {
                self.stats.resident_near += 1;
            } else {
                self.stats.resident_mid += 1;
            }
        } else {
            self.stats.resident_triangles -= tris;
            self.stats.resident_vertex_bytes -= vb;
            self.stats.resident_index_bytes -= ib;
            if rung == RUNG_NEAR {
                self.stats.resident_near -= 1;
            } else {
                self.stats.resident_mid -= 1;
            }
        }
    }

    /// Mark the packs whose upload finished this frame resident (their
    /// objects were written by the job; their groups are written here).
    pub fn after_pump(&mut self, renderer: &crate::Renderer, queue: &wgpu::Queue, done: &[u64]) {
        for &tag in done {
            if tag & TAG_KIT == 0 {
                continue;
            }
            let key = (((tag >> 8) & 0xffff_ffff) as u32, (tag & 0xff) as u8);
            let Some(Slot::Uploading(a)) = self.slots.remove(&key) else { continue };
            self.slots.insert(key, Slot::Resident(a));
            crate::hitch::mark(crate::hitch::Mark::KitPackResident, 1);
            self.set_ready(key.0, key.1, true);
            self.write_groups(renderer, queue, key.0);
            self.account(a, true, key.1);
            self.stats.packs_uploaded += 1;
        }
    }

    /// Reset the per-frame group byte count (call before `update`).
    pub fn begin_frame(&mut self) {
        self.last_group_bytes = 0;
    }

    /// Everything wanted around the camera is resident (nothing requested,
    /// cooking or uploading).
    pub fn settled(&self) -> bool {
        self.updates > 0
            && self.outbox.is_empty() // hitch
            && self.pending.lock().unwrap().is_empty()
            && self.in_flight.load(Ordering::SeqCst) == 0
            && self.slots.values().all(|s| matches!(s, Slot::Resident(_) | Slot::Deferred)) // + deferred (waits for space, not for work)
    }

    pub fn requested(&self) -> usize {
        self.slots.values().filter(|s| matches!(s, Slot::Requested)).count()
    }

    /// Used and peak of the arena (both parts summed when split;
    /// the peaks of the two parts may fall in different frames).
    pub fn arena_use(&self) -> (u32, u32, u32, u32, u32, u32) {
        let (mut u, p) = ((self.vtx.used, self.vtx.peak, self.idx.used, self.idx.peak, self.obj.used, self.obj.peak), &self.near_pool);
        if let Some(p) = p {
            u = (u.0 + p.vtx.used, u.1 + p.vtx.peak, u.2 + p.idx.used, u.3 + p.idx.peak, u.4 + p.obj.used, u.5 + p.obj.peak);
        }
        u
    }

    /// the near part's own use and peak (vertices, indices, objects), when split.
    pub fn near_pool_use(&self) -> Option<[(u32, u32); 3]> {
        self.near_pool.as_ref().map(|p| [(p.vtx.used, p.vtx.peak), (p.idx.used, p.idx.peak), (p.obj.used, p.obj.peak)])
    }

    /// Resident packs, for tests: (tile, rung) -> object range.
    pub fn resident(&self) -> Vec<((u32, u8), (u32, u32))> {
        self.slots
            .iter()
            .filter_map(|(k, s)| if let Slot::Resident(a) = s { Some((*k, (self.arena.object_base + a.object, a.objects))) } else { None })
            .collect()
    }
}

impl Drop for KitRuntime {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for h in self.threads.drain(..) {
            let _ = h.join();
        }
    }
}

impl GpuObject {
    /// An empty kit-arena slot (never drawn).
    pub fn zeroed_kit() -> Self {
        let mut o = GpuObject::new(ObjectKind::Kit, [0.0; 3], [0.0; 3], 0, 0, 0);
        o.lod_role = LOD_KIT_NEAR;
        o.lod_group = 0;
        o
    }
}

/// Build the planner's city off the render thread (load time).
pub fn spawn_kit_city(city: Arc<eo_data::City>, repo_root: PathBuf, palette: Vec<String>, streets: bool) -> Receiver<Result<(Arc<KitCity>, String), String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let r = (|| {
            eo_data::roof_colours::install(&repo_root); // before the planner and the cache key read it
            let tenancy = eo_kit::tenancy::Tenancy::load(&repo_root)?;
            let kc = KitCity::build(&city, &tenancy, &eo_kit::city::load_exclusions());
            Ok((Arc::new(kc), inputs_key(&repo_root, &palette, streets)))
        })();
        let _ = tx.send(r);
    });
    rx
}
