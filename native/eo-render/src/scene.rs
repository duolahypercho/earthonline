//! CPU side of the static world: turn the loaded city into a few large
//! vertex/index arrays plus one object table (bounds, draw range, material,
//! LOD role per object). Runs once at load; nothing here runs per frame.

use bytemuck::{Pod, Zeroable};
use eo_data::standards::{self, CaStandards};
use eo_data::{City, ElevationGrid, LandCover, SEA_LEVEL_Y};
use std::collections::HashMap;
use std::time::Instant;

/// One vertex (20 bytes): world position, a packed normal (snorm8 x 3; all
/// zero = "no normal", the fragment shader then takes the flat face normal
/// from screen derivatives) and an RGBA8 colour (sRGB bytes; white on models).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: u32,
    pub color: u32,
}

/// One cullable object, as the compute shader reads it (64 bytes, std430).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
pub struct GpuObject {
    pub min: [f32; 3],
    pub index_count: u32,
    pub max: [f32; 3],
    pub first_index: u32,
    pub base_vertex: i32,
    /// `ObjectKind` in bits 0-7; `BLEND_BIT` when it draws in the transparent pass.
    pub kind: u32,
    /// Copy index for the stress multiplier (0 for the real city).
    pub copy: u32,
    /// Index into the material table.
    pub material: u32,
    /// LOD group (a Blender building) for roles NEAR and FAR; `NO_GROUP` otherwise.
    pub lod_group: u32,
    pub lod_role: u32,
    /// Coarse index range for `LOD_TERRAIN` objects.
    pub far_first_index: u32,
    pub far_index_count: u32,
}

pub const NO_GROUP: u32 = u32::MAX;
/// Always drawn when in the frustum.
pub const LOD_ALWAYS: u32 = 0;
/// Drawn while its group is detailed (JS HLOD radius) and its model is resident.
pub const LOD_NEAR: u32 = 1;
/// Drawn otherwise: the footprint stand-in of a Blender building.
pub const LOD_FAR: u32 = 2;
/// Terrain tile: full grid near, the coarse index range past the terrain LOD distance.
pub const LOD_TERRAIN: u32 = 3;
/// Kit building rung 0 (the full plan): drawn while its group's level is 0.
pub const LOD_KIT_NEAR: u32 = 4;
/// Kit building rung 2 (reduced): drawn while its group's level is 1.
pub const LOD_KIT_MID: u32 = 5;
/// Kit building's footprint box: drawn while its group's level is 2.
pub const LOD_KIT_BOX: u32 = 6;
/// Street tile near pack: drawn while its group's level is 0.
pub const LOD_STREET_NEAR: u32 = 7;
/// Street tile far strips: level 1.
pub const LOD_STREET_FAR: u32 = 8;
/// Street tile coarse strips: level 2.
pub const LOD_STREET_COARSE: u32 = 9;
/// a landmark's main draws and skirt, drawn while its group is near (landmarks.rs).
pub const LOD_LANDMARK_NEAR: u32 = 10;
/// a landmark's far item, drawn while its group is not near.
pub const LOD_LANDMARK_FAR: u32 = 11;
/// The full material part within 120 m of its bounds, then the coarse mesh.
pub const LOD_AUTHORED_DETAIL: u32 = 12;
pub const BLEND_BIT: u32 = 1 << 8;
/// Model with per-board coordinates for the photographic floor scan.
pub const FLOOR_SCAN_BIT: u32 = 1 << 9;
/// Loose mineral aggregate on a private lot.
pub const GRAVEL_BIT: u32 = 1 << 10;
/// Leaf-local coordinates for a swinging painted timber door.
pub const DOOR_SCAN_BIT: u32 = 1 << 11;
/// Camera-facing reflection in the actual front sash of the Beaver Street house.
pub const FRONT_SASH_BIT: u32 = 1 << 12;
/// Mineral plaster with locally baked exposure to splash and runoff.
pub const LIMEWASH_BIT: u32 = 1 << 13;
/// An entire carpet sampled once in its authored orientation.
pub const RUG_SCAN_BIT: u32 = 1 << 14;
/// Board-mounted room coordinates and the interior reflection of the sash.
pub const WAXED_BOARD_BIT: u32 = 1 << 15;
/// Private paving and aggregate receive the same outdoor bounce as the planted bed.
pub const COURTYARD_BOUNCE_BIT: u32 = 1 << 16;
/// Negative normal W stores near-field visibility on the enclosed side of the wall.
pub const ENCLOSURE_TRANSFER_BIT: u32 = 1 << 17;
/// Thin woven panels and leaves transmit the exterior daylight behind them.
pub const THIN_FIBRE_BIT: u32 = 1 << 18;
/// Opaque bed linen has a rough fibrous surface over a softer filling.
pub const QUILT_FIBRE_BIT: u32 = 1 << 19;

/// Material 0: the vertex colour (footprints, terrain, roads, sea).
pub const MATERIAL_VERTEX_COLOUR: u32 = 0;

impl GpuObject {
    pub fn new(kind: ObjectKind, min: [f32; 3], max: [f32; 3], first_index: u32, index_count: u32, base_vertex: i32) -> Self {
        Self {
            min,
            index_count,
            max,
            first_index,
            base_vertex,
            kind: kind as u32,
            copy: 0,
            material: MATERIAL_VERTEX_COLOUR,
            lod_group: NO_GROUP,
            lod_role: LOD_ALWAYS,
            far_first_index: 0,
            far_index_count: 0,
        }
    }
    pub fn object_kind(&self) -> u32 {
        self.kind & 0xff
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum ObjectKind {
    Building = 0,
    Terrain = 1,
    Road = 2,
    Sea = 3,
    /// A part (one material) of a Blender-built building's model.
    Model = 4,
    /// A kit building's rung (material per vertex from the kit palette).
    Kit = 5,
    /// Street surface (slice 4): material per vertex from the kit palette.
    Street = 6,
    /// Street paint and banks (slice 4): vertex colour.
    StreetPaint = 7,
    /// A building's foundation skirt (slice 4): vertex colour.
    Skirt = 8,
    /// a landmark's near draw (casts sun shadows): exact sRGB vertex colour.
    Landmark = 9,
    /// a landmark's far item or skirt, and the open-space ground (receives only).
    LandmarkNoShadow = 10,
}

/// A foundation skirt: the footprint ring walled from `bottom` up to `base`.
#[derive(Clone, Debug)]
pub struct Skirt {
    pub ring: Vec<[f32; 2]>,
    pub base: f32,
    pub bottom: f32,
}

/// Append a skirt wall round `ring` (closed or not) to vertex/index lists.
pub fn skirt_quads(ring: &[[f32; 2]], base: f32, bottom: f32, color: u32, verts: &mut Vec<Vertex>, idx: &mut Vec<u32>) {
    let mut r: Vec<[f32; 2]> = ring.to_vec();
    if r.len() > 1 && r[0] == r[r.len() - 1] {
        r.pop();
    }
    if r.len() < 3 || !(base > bottom) {
        return;
    }
    for i in 0..r.len() {
        let (a, b) = (r[i], r[(i + 1) % r.len()]);
        if (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6 {
            continue;
        }
        let s = verts.len() as u32;
        for (p, y) in [(a, bottom), (b, bottom), (b, base), (a, base)] {
            verts.push(Vertex { pos: [p[0], y, p[1]], normal: 0, color });
        }
        idx.extend_from_slice(&[s, s + 1, s + 2, s, s + 2, s + 3]);
    }
}

#[derive(Clone, Debug, Default)]
pub struct SceneStats {
    pub buildings: usize,
    pub building_triangles: u64,
    /// Footprints that stand in for a Blender model (role FAR).
    pub authored_footprints: usize,
    /// Footprints that are a kit building's far box (role KIT_BOX).
    pub kit_boxes: usize,
    pub terrain_tiles: usize,
    pub terrain_triangles: u64,
    pub terrain_coarse_triangles: u64,
    pub road_objects: usize,
    pub road_ways_drawn: usize,
    pub road_ways_skipped_bridge_or_tunnel: usize,
    pub road_triangles: u64,
    pub skirt_triangles: u64,
    pub sea_triangles: u64,
    pub objects: usize,
    pub triangles: u64,
    pub vertices: usize,
    pub indices: usize,
    pub tile_size_m: f32,
    pub city_tiles_with_content: usize,
    pub build_ms: f64,
}

pub struct Scene {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub objects: Vec<GpuObject>,
    pub stats: SceneStats,
    /// World bounds of everything drawn.
    pub world_min: [f32; 3],
    pub world_max: [f32; 3],
}

/// Side of a city tile, metres. Terrain tiles are 32 x 32 elevation cells
/// (8 m) and roads are grouped by the same tiles.
pub const TILE_CELLS: usize = 32;
/// Vertex stride of the coarse terrain level (every 4th grid vertex: 32 m).
pub const TERRAIN_COARSE_STRIDE: usize = 4;

pub const fn rgba(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16) | (255u32 << 24)
}

#[allow(dead_code)]
fn shade(c: u32, k: f32) -> u32 {
    let ch = |s: u32| (((c >> s) & 255) as f32 * k).clamp(0.0, 255.0) as u32;
    ch(0) | (ch(8) << 8) | (ch(16) << 16) | (255 << 24)
}

#[allow(dead_code)]
fn hash(s: &str) -> u32 {
    let mut h: u32 = 2166136261;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    h
}

/// Flat debug palette for terrain, keyed by WorldCover class. These are
/// placeholder tones for a performance slice, not a look.
fn land_color(class: u8, underwater: bool) -> u32 {
    if underwater {
        return rgba(70, 82, 80);
    }
    match class {
        10 => rgba(62, 92, 52),
        20 => rgba(108, 118, 70),
        30 | 40 | 100 => rgba(120, 140, 82),
        60 => rgba(150, 132, 104),
        80 | 90 | 95 => rgba(96, 110, 100),
        _ => rgba(150, 146, 136),
    }
}

fn v(pos: [f32; 3], color: u32) -> Vertex {
    Vertex { pos, normal: 0, color }
}

fn bounds(verts: &[Vertex]) -> ([f32; 3], [f32; 3]) {
    let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
    for v in verts {
        for k in 0..3 {
            mn[k] = mn[k].min(v.pos[k]);
            mx[k] = mx[k].max(v.pos[k]);
        }
    }
    (mn, mx)
}

struct Builder {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    objects: Vec<GpuObject>,
}

impl Builder {
    /// Append one object whose vertices are `verts` and whose indices are
    /// local to them; returns its object index, or None for an empty object.
    fn push(&mut self, kind: ObjectKind, verts: &[Vertex], local_indices: &[u32]) -> Option<usize> {
        if verts.is_empty() || local_indices.is_empty() {
            return None;
        }
        let base_vertex = self.vertices.len() as i32;
        let first_index = self.indices.len() as u32;
        let (mn, mx) = bounds(verts);
        self.vertices.extend_from_slice(verts);
        self.indices.extend_from_slice(local_indices);
        self.objects.push(GpuObject::new(kind, mn, mx, first_index, local_indices.len() as u32, base_vertex));
        Some(self.objects.len() - 1)
    }

    /// Append an object that reuses an index range already in the buffer
    /// (terrain tiles share one grid topology and differ by base vertex).
    fn push_shared(&mut self, kind: ObjectKind, verts: &[Vertex], first_index: u32, index_count: u32) -> usize {
        let base_vertex = self.vertices.len() as i32;
        let (mn, mx) = bounds(verts);
        self.vertices.extend_from_slice(verts);
        self.objects.push(GpuObject::new(kind, mn, mx, first_index, index_count, base_vertex));
        self.objects.len() - 1
    }
}

pub struct SceneInputs<'a> {
    pub city: &'a City,
    pub elevation: &'a ElevationGrid,
    pub land: Option<&'a LandCover>,
    pub standards: &'a CaStandards,
    /// Building id -> LOD group of its Blender model. Those footprints become
    /// the model's far stand-in (role FAR) instead of always drawing.
    pub authored: &'a HashMap<String, u32>,
    /// Give terrain tiles the coarse level (role TERRAIN). Off = slice 1's full grid everywhere.
    pub terrain_lod: bool,
    /// Per city building index: its kit LOD group (`NO_GROUP`: none). Those
    /// footprints become the kit building's far box (role KIT_BOX). Empty: no kit.
    pub kit_groups: &'a [u32],
    /// Slice 4: the terrain vertex height at a grid vertex (the street world's
    /// ground carpet: lifted, held under streets). None: the raw ground.
    pub carpet: Option<&'a (dyn Fn(f32, f32) -> f32 + Sync)>,
    /// Slice 4: the streets' static far and coarse levels, replacing the flat road ribbons.
    pub street: Option<&'a crate::street::StreetStatic>,
    /// Slice 4: foundation skirts drawn always (the Blender-built buildings').
    pub skirts: &'a [Skirt],
    /// per city building index, the floor a kit building's far box
    /// stands its height on (near the street-front floor its rungs are planned
    /// at; `kit::far_floor_estimate`). None: the lowest ground (slice 1).
    pub box_floor: Option<&'a (dyn Fn(usize) -> Option<f32> + Sync)>,
    /// city building indices drawn by something else (the records a
    /// landmark building claims): no footprint box at all.
    pub skip_building: Option<&'a (dyn Fn(usize) -> bool + Sync)>,
    /// a terrain vertex's colour where the open-space ground
    /// decides it (a green's tone, beach sand with the shore distance in its
    /// alpha byte); None: the land-cover tint.
    pub ground_tone: Option<&'a (dyn Fn(f32, f32) -> Option<u32> + Sync)>,
    /// give the terrain smooth vertex normals (central differences
    /// of the drawn heights, as three's `computeVertexNormals` gives the JS
    /// ground carpet) instead of each triangle's face normal.
    pub smooth_terrain: bool,
}

pub fn build_scene(inp: &SceneInputs) -> anyhow::Result<Scene> {
    let t0 = Instant::now();
    let g = inp.elevation;
    let cell = g.meta.cell_size as f32;
    let tile_m = cell * TILE_CELLS as f32;
    let (ox, oz) = (g.meta.origin_x as f32, g.meta.origin_z as f32);
    let tiles_x = g.meta.width.div_ceil(TILE_CELLS);
    let tiles_z = g.meta.height.div_ceil(TILE_CELLS);
    let tile_of = |x: f32, z: f32| -> usize {
        let tx = (((x - ox) / tile_m).floor() as i64).clamp(0, tiles_x as i64 - 1) as usize;
        let tz = (((z - oz) / tile_m).floor() as i64).clamp(0, tiles_z as i64 - 1) as usize;
        tz * tiles_x + tx
    };

    let mut b = Builder { vertices: Vec::new(), indices: Vec::new(), objects: Vec::new() };
    let mut stats = SceneStats { tile_size_m: tile_m, ..Default::default() };
    let mut tile_used = vec![false; tiles_x * tiles_z];

    // --- Buildings: one object each, sorted by tile for locality. -----------
    let mut order: Vec<usize> = (0..inp.city.buildings.len()).collect();
    let centroid = |i: usize| {
        let r = &inp.city.buildings[i].ring;
        let (sx, sz) = r.iter().fold((0.0f32, 0.0f32), |a, p| (a.0 + p[0], a.1 + p[1]));
        (sx / r.len() as f32, sz / r.len() as f32)
    };
    let keys: Vec<usize> = (0..order.len()).map(|i| { let c = centroid(i); tile_of(c.0, c.1) }).collect();
    order.sort_by_key(|&i| keys[i]);
    // the far ring's wall tones, worked out on every core (the
    // material inference costs ~0.6 s for the city on one).
    let tones: Vec<([u8; 3], [u8; 3])> = {
        let bs = &inp.city.buildings;
        let threads = std::thread::available_parallelism().map(|p| p.get()).unwrap_or(4).clamp(1, 16);
        let chunk = bs.len().div_ceil(threads).max(1);
        std::thread::scope(|s| {
            let hs: Vec<_> = bs.chunks(chunk).map(|c| s.spawn(move || c.iter().map(crate::surfaces::far_wall_tones).collect::<Vec<_>>())).collect();
            hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
        })
    };
    let mut ear = earcut::Earcut::<f64>::new();
    let mut tri: Vec<u32> = Vec::new();
    let mut verts: Vec<Vertex> = Vec::new();
    let mut idx: Vec<u32> = Vec::new();
    let mut flat: Vec<[f64; 2]> = Vec::new();
    for &i in &order {
        if inp.skip_building.is_some_and(|f| f(i)) {
            continue; // a landmark draws it
        }
        let bld = &inp.city.buildings[i];
        let ring = &bld.ring;
        let n = ring.len();
        let lowest = ring.iter().map(|p| g.ground(p[0] as f64, p[1] as f64)).fold(f64::MAX, f64::min) as f32;
        // a kit box stands its height on (an estimate of) the floor
        // its rungs are planned at, its foot never above the lowest ground.
        let is_kit = inp.kit_groups.get(i).is_some_and(|g| *g != NO_GROUP) && !inp.authored.contains_key(&bld.id);
        let floor = if is_kit { inp.box_floor.and_then(|f| f(i)).unwrap_or(lowest) } else { lowest };
        let base = lowest.min(floor);
        let top = floor + bld.height;
        // the JS far ring's wall colour (foot and head of the gradient).
        let (foot, head) = tones[i];
        let wall_top = rgba(head[0], head[1], head[2]);
        let wall_bottom = rgba(foot[0], foot[1], foot[2]);
        verts.clear();
        idx.clear();
        for p in ring {
            verts.push(v([p[0], base, p[1]], wall_bottom));
        }
        for p in ring {
            verts.push(v([p[0], top, p[1]], wall_top));
        }
        for a in 0..n {
            let c = (a + 1) % n;
            let (ba, bc, ta, tc) = (a as u32, c as u32, (a + n) as u32, (c + n) as u32);
            idx.extend_from_slice(&[ba, bc, tc, ba, tc, ta]);
        }
        flat.clear();
        flat.extend(ring.iter().map(|p| [p[0] as f64, p[1] as f64]));
        ear.earcut(flat.iter().copied(), &[] as &[u32], &mut tri);
        for t in &tri {
            idx.push(*t + n as u32);
        }
        if let Some(o) = b.push(ObjectKind::Building, &verts, &idx) {
            stats.buildings += 1;
            stats.building_triangles += (idx.len() / 3) as u64;
            tile_used[keys[i]] = true;
            if let Some(&group) = inp.authored.get(&bld.id) {
                b.objects[o].lod_group = group;
                b.objects[o].lod_role = LOD_FAR;
                stats.authored_footprints += 1;
            } else if let Some(&group) = inp.kit_groups.get(i).filter(|g| **g != NO_GROUP) {
                b.objects[o].lod_group = group;
                b.objects[o].lod_role = LOD_KIT_BOX;
                stats.kit_boxes += 1;
            }
        }
    }

    // --- Terrain: 32 x 32-cell tiles over the whole elevation grid. ---------
    // All tiles share one (33 x 33)-vertex grid topology in the index buffer,
    // and one coarse topology over every 4th vertex of the same grid.
    let side = TILE_CELLS + 1;
    let shared_first = b.indices.len() as u32;
    for z in 0..TILE_CELLS {
        for x in 0..TILE_CELLS {
            let i0 = (z * side + x) as u32;
            let i1 = i0 + 1;
            let i2 = i0 + side as u32;
            let i3 = i2 + 1;
            b.indices.extend_from_slice(&[i0, i2, i1, i1, i2, i3]);
        }
    }
    let shared_count = (TILE_CELLS * TILE_CELLS * 6) as u32;
    let coarse_first = b.indices.len() as u32;
    let s = TERRAIN_COARSE_STRIDE;
    for z in (0..TILE_CELLS).step_by(s) {
        for x in (0..TILE_CELLS).step_by(s) {
            let i0 = (z * side + x) as u32;
            let i1 = i0 + s as u32;
            let i2 = i0 + (s * side) as u32;
            let i3 = i2 + s as u32;
            b.indices.extend_from_slice(&[i0, i2, i1, i1, i2, i3]);
        }
    }
    let coarse_count = b.indices.len() as u32 - coarse_first;
    // Vertex rows of every terrain tile, computed in parallel (the street
    // world's carpet answers per vertex and is not free).
    let tile_verts = |tx: usize, tz: usize| -> (Vec<Vertex>, bool) {
        let mut verts = Vec::with_capacity(side * side);
        let mut any_land = false;
        let height = |cx: i64, cz: i64| -> f32 {
            // Vertices sit on cell centres; past the grid's last
            // column/row the sampler clamps.
            let wx = ox + (cx as f32 + 0.5) * cell;
            let wz = oz + (cz as f32 + 0.5) * cell;
            match inp.carpet {
                Some(f) => f(wx, wz),
                None => g.ground(wx as f64, wz as f64) as f32,
            }
        };
        // one ring of cells round the tile, for the central differences.
        let n = side + 2;
        let ring: Vec<f32> = if inp.smooth_terrain {
            (0..n * n).map(|i| height((tx * TILE_CELLS) as i64 + (i % n) as i64 - 1, (tz * TILE_CELLS) as i64 + (i / n) as i64 - 1)).collect()
        } else {
            vec![]
        };
        for z in 0..side {
            for x in 0..side {
                let cx = tx * TILE_CELLS + x;
                let cz = tz * TILE_CELLS + z;
                let wx = ox + (cx as f32 + 0.5) * cell;
                let wz = oz + (cz as f32 + 0.5) * cell;
                let y = if inp.smooth_terrain { ring[(z + 1) * n + x + 1] } else { height(cx as i64, cz as i64) };
                let in_grid = cx < g.meta.width && cz < g.meta.height;
                let lidar = in_grid && g.is_lidar_ground_cell(cx, cz);
                any_land |= lidar;
                let class = inp.land.map(|l| l.class_at(wx as f64, wz as f64)).unwrap_or(50);
                let colour = inp.ground_tone.and_then(|f| f(wx, wz)).unwrap_or_else(|| land_color(class, !lidar));
                let mut vert = v([wx, y, wz], colour);
                if inp.smooth_terrain {
                    let dx = (ring[(z + 1) * n + x + 2] - ring[(z + 1) * n + x]) / (2.0 * cell);
                    let dz = (ring[(z + 2) * n + x + 1] - ring[z * n + x + 1]) / (2.0 * cell);
                    vert.normal = eo_assets::glb::pack_normal(glam::Vec3::new(-dx, 1.0, -dz));
                }
                verts.push(vert);
            }
        }
        (verts, any_land)
    };
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(16);
    let all: Vec<(usize, usize)> = (0..tiles_z).flat_map(|tz| (0..tiles_x).map(move |tx| (tx, tz))).collect();
    let chunk = all.len().div_ceil(threads).max(1);
    let built: Vec<(Vec<Vertex>, bool)> = std::thread::scope(|s| {
        let hs: Vec<_> = all.chunks(chunk).map(|c| s.spawn(move || c.iter().map(|&(tx, tz)| tile_verts(tx, tz)).collect::<Vec<_>>())).collect();
        hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    for (&(tx, tz), (tv, any_land)) in all.iter().zip(built) {
        {
            let verts = tv;
            let o = b.push_shared(ObjectKind::Terrain, &verts, shared_first, shared_count);
            if inp.terrain_lod {
                b.objects[o].lod_role = LOD_TERRAIN;
                b.objects[o].far_first_index = coarse_first;
                b.objects[o].far_index_count = coarse_count;
            }
            stats.terrain_tiles += 1;
            stats.terrain_triangles += (shared_count / 3) as u64;
            stats.terrain_coarse_triangles += (coarse_count / 3) as u64;
            if any_land {
                tile_used[tz * tiles_x + tx] = true;
            }
        }
    }

    // --- Roads: carriageway ribbons on the terrain, grouped per tile. -------
    let lift = inp
        .city
        .meta
        .street_design
        .as_ref()
        .and_then(|s| s.road_surface_lift)
        .ok_or_else(|| anyhow::anyhow!("sf-city.json meta.streetDesign.roadSurfaceLift missing"))? as f32;
    let road_color = rgba(58, 60, 64);
    let mut per_tile: Vec<Vec<usize>> = vec![Vec::new(); tiles_x * tiles_z];
    for (ri, r) in inp.city.roads.iter().enumerate() {
        if inp.street.is_some() {
            break; // slice 4: the street world draws the streets
        }
        if !standards::is_vehicle_way(&r.highway) || r.points.len() < 2 {
            continue;
        }
        if r.bridge || r.tunnel {
            stats.road_ways_skipped_bridge_or_tunnel += 1;
            continue;
        }
        let m = r.points[r.points.len() / 2];
        per_tile[tile_of(m[0], m[1])].push(ri);
    }
    for (t, ways) in per_tile.iter().enumerate() {
        if ways.is_empty() {
            continue;
        }
        verts.clear();
        idx.clear();
        for &ri in ways {
            let r = &inp.city.roads[ri];
            let half = (r.lanes as f64 * inp.standards.class_lane_width(r)? * 0.5) as f32;
            // Resample to the elevation cell so the ribbon follows the ground.
            let mut pts: Vec<[f32; 2]> = Vec::new();
            for w in r.points.windows(2) {
                let (a, c) = (w[0], w[1]);
                let len = ((c[0] - a[0]).powi(2) + (c[1] - a[1]).powi(2)).sqrt();
                let steps = ((len / cell).ceil() as usize).max(1);
                for s in 0..steps {
                    let k = s as f32 / steps as f32;
                    pts.push([a[0] + (c[0] - a[0]) * k, a[1] + (c[1] - a[1]) * k]);
                }
            }
            pts.push(*r.points.last().unwrap());
            pts.dedup();
            if pts.len() < 2 {
                continue;
            }
            let start = verts.len() as u32;
            for i in 0..pts.len() {
                let p = pts[i];
                let prev = pts[i.saturating_sub(1)];
                let next = pts[(i + 1).min(pts.len() - 1)];
                let (dx, dz) = (next[0] - prev[0], next[1] - prev[1]);
                let l = (dx * dx + dz * dz).sqrt().max(1e-6);
                let (nx, nz) = (-dz / l, dx / l);
                for side_sign in [-1.0f32, 1.0] {
                    let x = p[0] + nx * half * side_sign;
                    let z = p[1] + nz * half * side_sign;
                    let y = g.ground(x as f64, z as f64) as f32 + lift;
                    verts.push(v([x, y, z], road_color));
                }
            }
            for i in 0..(pts.len() as u32 - 1) {
                let a = start + i * 2;
                idx.extend_from_slice(&[a, a + 1, a + 3, a, a + 3, a + 2]);
            }
            stats.road_ways_drawn += 1;
        }
        if b.push(ObjectKind::Road, &verts, &idx).is_some() {
            stats.road_objects += 1;
            stats.road_triangles += (idx.len() / 3) as u64;
            tile_used[t] = true;
        }
    }

    // --- Slice 4: the streets' far and coarse levels (static). ---------------
    if let Some(st) = inp.street {
        let base_vertex = b.vertices.len() as i32;
        let base_index = b.indices.len() as u32;
        b.vertices.extend_from_slice(&st.vertices);
        b.indices.extend_from_slice(&st.indices);
        for o in &st.objects {
            let mut o = *o;
            o.base_vertex += base_vertex;
            o.first_index += base_index;
            b.objects.push(o);
            stats.road_objects += 1;
            stats.road_triangles += (o.index_count / 3) as u64;
        }
    }

    // --- Slice 4: foundation skirts of the Blender-built buildings. ----------
    if !inp.skirts.is_empty() {
        verts.clear();
        idx.clear();
        let concrete = rgba(150, 146, 138);
        for s in inp.skirts {
            skirt_quads(&s.ring, s.base, s.bottom, concrete, &mut verts, &mut idx);
        }
        if b.push(ObjectKind::Skirt, &verts, &idx).is_some() {
            stats.skirt_triangles = (idx.len() / 3) as u64;
        }
    }

    // --- Sea: one flat quad at mean sea level over the grid's extent. -------
    let (x0, z0, x1, z1) = (ox, oz, g.max_x() as f32, g.max_z() as f32);
    let sy = SEA_LEVEL_Y as f32;
    let sea = rgba(46, 74, 96);
    let sv = [v([x0, sy, z0], sea), v([x1, sy, z0], sea), v([x0, sy, z1], sea), v([x1, sy, z1], sea)];
    b.push(ObjectKind::Sea, &sv, &[0, 2, 1, 1, 2, 3]);
    stats.sea_triangles = 2;

    stats.objects = b.objects.len();
    stats.triangles = b.objects.iter().map(|o| (o.index_count / 3) as u64).sum();
    stats.vertices = b.vertices.len();
    stats.indices = b.indices.len();
    stats.city_tiles_with_content = tile_used.iter().filter(|u| **u).count();
    let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
    for o in &b.objects {
        for k in 0..3 {
            mn[k] = mn[k].min(o.min[k]);
            mx[k] = mx[k].max(o.max[k]);
        }
    }
    stats.build_ms = t0.elapsed().as_secs_f64() * 1e3;
    Ok(Scene { vertices: b.vertices, indices: b.indices, objects: b.objects, stats, world_min: mn, world_max: mx })
}

/// Metres between stress copies. Small on purpose: every copy stays in the
/// same view as the real city, so drawn objects scale with the multiplier.
pub const COPY_STEP_M: f32 = 3.0;

/// Offset of stress copy `k` (mirrors `copy_offset` in draw.wgsl).
pub fn copy_offset(k: u32) -> [f32; 3] {
    [(k % 4) as f32 * COPY_STEP_M, 0.0, (k / 4) as f32 * COPY_STEP_M]
}

/// Stress variant: `n` copies of every object, sharing the same geometry
/// (copy k reuses the draw range and is shifted in the vertex shader).
/// Only the object table grows; vertex/index buffers do not.
pub fn multiply_objects(objects: &[GpuObject], n: u32) -> Vec<GpuObject> {
    let mut out = Vec::with_capacity(objects.len() * n as usize);
    for k in 0..n {
        let d = copy_offset(k);
        out.extend(objects.iter().map(|o| GpuObject {
            min: [o.min[0] + d[0], o.min[1] + d[1], o.min[2] + d[2]],
            max: [o.max[0] + d[0], o.max[1] + d[1], o.max[2] + d[2]],
            copy: k,
            ..*o
        }));
    }
    out
}

/// Copy little-endian bytes read back from the GPU into u32 words.
pub fn cast_u32(b: &[u8]) -> Vec<u32> {
    bytemuck::pod_collect_to_vec(b)
}
