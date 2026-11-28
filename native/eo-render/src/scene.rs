//! CPU side of the static world: turn the loaded city into a few large
//! vertex/index arrays plus one object table (bounds, draw range, material,
//! LOD role per object). Runs once at load; nothing here runs per frame.

use bytemuck::{Pod, Zeroable};
use eo_data::standards::{self, CaStandards};
use eo_data::{City, ElevationGrid, LandCover, SEA_LEVEL_Y};
use std::collections::HashMap;
use std::time::Instant;

/// The terrain surface pass's relief patches and materials (round 3).
#[path = "terrain_detail.rs"]
pub mod terrain_detail;

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
/// Use the authored UVs of a local photographic surface.
pub const LOCAL_UV_SCAN_BIT: u32 = 1 << 14;
/// The original whole-carpet consumer of the local UV scan.
pub const RUG_SCAN_BIT: u32 = LOCAL_UV_SCAN_BIT;
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
/// An individually mapped oak plank in the dining tabletop.
pub const TABLE_BOARD_BIT: u32 = 1 << 20;
/// Door lever and lock reflecting the actual porch opening.
pub const ENTRY_BRASS_BIT: u32 = 1 << 21;
/// Cloth shading follows the garment's flat cutting pattern.
pub const CUT_COTTON_BIT: u32 = 1 << 22;
/// Use the measured Cycles irradiance for the enclosed middle floor.
pub const BEDROOM_LIGHT_BIT: u32 = 1 << 23;
/// Ground-storey boards receive the Blender dining pendant and window light.
pub const DINING_FLOOR_LIGHT_BIT: u32 = 1 << 24;
/// Interior faces below the bedroom landing use the furnished-room irradiance grid.
pub const DOWNSTAIRS_LIGHT_BIT: u32 = 1 << 25;
/// The tabletop and place settings receive the installed pendant above the wood.
pub const TABLE_LIGHT_BIT: u32 = 1 << 26;
/// Photographic wood and its two reflection lobes on the four dining boards.
pub const OAK_FINISH_BIT: u32 = 1 << 27;
/// Authored board strips and the surveyed incident light above the furnished ground floor.
pub const FLOOR_REFLECTION_BIT: u32 = 1 << 28;
/// The fitted Blender room carries two half-float UVs in the vertex color.
pub const FITTED_ROOM_BIT: u32 = 1 << 29;
/// Use the fitted photograph on the ground-floor boards of the shared shell.
pub const FITTED_FLOOR_BIT: u32 = 1 << 30;
/// A shell plaster material whose exterior faces can receive the local wall survey.
pub const WALL_DAYLIGHT_BIT: u32 = 1 << 31;

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
    /// Close underwater sediment, shells and flexible eelgrass blades.
    Seabed = 13,
    /// a grade-separated road's deck surface (the street material) and its
    /// paint, joints and pipes (vertex colour), `--structures` (structures.rs):
    /// exact sRGB vertex colour as the street's; receives only.
    StructureDeck = 11,
    /// a grade-separated road's concrete (barriers, fascia, girder, soffit,
    /// caps, columns, walls): the tint in the material, the vertex colour and
    /// the normal's w carry the formwork coordinates and the wear
    /// (structures.rs `encode_concrete`); casts sun shadows at the level shown.
    Structure = 12,
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
    /// Boxes drawn with their facade (far_box.rs; 0 with `--no-far-facade`).
    pub far_facades: usize,
    pub terrain_tiles: usize,
    pub terrain_triangles: u64,
    pub terrain_coarse_triangles: u64,
    /// Terrain tiles with a tunnel portal's canopy left out (their own index
    /// lists; `build_scene_with_cuts`, `--tunnels`); 0 otherwise.
    pub terrain_cut_tiles: usize,
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

/// The built-up ground's colour (WorldCover 50): the city's open paved ground
/// where nothing else is drawn (eo-app's fitted beach sand gives it to a
/// quay the sand used to cover).
pub fn built_ground_color() -> u32 {
    land_color(50, false)
}

/// A terrain vertex's colour at (wx, wz) where the ground tone has none
/// (`colour_at` in the scene build: the land-cover class's tint, the cell's
/// lidar flag), for the street ground drawn as terrain (street.rs).
pub fn terrain_land_colour(land: Option<&LandCover>, g: &ElevationGrid, wx: f32, wz: f32) -> u32 {
    let (ox, oz, cell) = (g.meta.origin_x as f32, g.meta.origin_z as f32, g.meta.cell_size as f32);
    let cx = ((wx - ox) / cell).floor();
    let cz = ((wz - oz) / cell).floor();
    let lidar = cx >= 0.0 && cz >= 0.0 && (cx as usize) < g.meta.width && (cz as usize) < g.meta.height && g.is_lidar_ground_cell(cx as usize, cz as usize);
    let class = land.map(|l| l.class_at(wx as f64, wz as f64)).unwrap_or(50);
    land_color(class, !lidar)
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

    /// Emit the same excavated surface at driving and flight distances.
    fn push_cut_ground(&mut self, mut verts: Vec<Vertex>, cuts: &[&eo_structures::TerrainCut], keep: &dyn Fn(usize, usize) -> bool, lod: bool, stats: &mut SceneStats) -> bool {
        let sections: Vec<_> = cuts.iter().flat_map(|c| c.sections()).collect();
        let side = TILE_CELLS + 1;
        let fine = fit_tunnel_ground(&mut verts, side, 1, &sections, keep);
        let coarse = fit_tunnel_ground(&mut verts, side, TERRAIN_COARSE_STRIDE, &sections, keep);
        if let Some(o) = self.push(ObjectKind::Terrain, &verts, &fine) {
            if lod {
                let first = self.indices.len() as u32;
                self.indices.extend_from_slice(&coarse);
                self.objects[o].lod_role = LOD_TERRAIN;
                self.objects[o].far_first_index = first;
                self.objects[o].far_index_count = coarse.len() as u32;
            }
            stats.terrain_triangles += (fine.len() / 3) as u64;
            stats.terrain_coarse_triangles += (coarse.len() / 3) as u64;
            true
        } else {
            false
        }
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
    build_scene_with_cuts(inp, None)
}

pub use eo_structures::cut_ground::{CUT_REACH_CELLS, terrain_cut_floor};

use eo_structures::cut_ground::GroundSection;

/// Apply the excavation footprints to the high-resolution relief patches.
fn tunnel_relief_cuts() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| !std::env::args().any(|a| a == "--no-tunnel-relief-cuts"))
}


/// One terrain tile's triangles (the shared topology at `stride` over
/// `verts`) less the portals' canopies (`rings`, convex: eo-structures
/// `TerrainCut::canopy`), exactly: a triangle a canopy reaches into is cut
/// along its edges and the pieces outside it kept (new vertices appended to
/// `verts`, interpolated in the triangle's plane, the nearest corner's colour
/// and normal), so the hole is the canopy's footprint, which the portal
/// headwall's top and ends cover.
/// Only the cells `keep(x, z)` keeps are drawn (the cell's first vertex column
/// and row; at `stride` a coarse cell goes with its first fine cell): the
/// terrain detail's relief patches leave their cells out this way.
fn fit_tunnel_ground(verts: &mut Vec<Vertex>, side: usize, stride: usize, sections: &[GroundSection], keep: &dyn Fn(usize, usize) -> bool) -> Vec<u32> {
    const MIN_PIECE_AREA_M2: f64 = 1e-4;
    // Adjacent sections share survey edges. Re-clipping a rounded GPU vertex
    // against the next section can leave thin, disconnected pieces at a step.
    let first_clipped = verts.len();
    let mut clipped = Vec::<[f64; 3]>::new();
    let mut tris: Vec<[u32; 3]> = vec![];
    for z in (0..side - 1).step_by(stride) {
        for x in (0..side - 1).step_by(stride) {
            if !keep(x, z) {
                continue;
            }
            let i0 = (z * side + x) as u32;
            let i1 = i0 + stride as u32;
            let i2 = i0 + (stride * side) as u32;
            let i3 = i2 + stride as u32;
            tris.push([i0, i2, i1]);
            tris.push([i1, i2, i3]);
        }
    }
    for section in sections {
        let (mn, mx) = (section.min, section.max);
        let mut next = Vec::with_capacity(tris.len());
        for t in tris {
            let p = t.map(|i| {
                let i = i as usize;
                if i < first_clipped { verts[i].pos.map(f64::from) } else { clipped[i - first_clipped] }
            });
            let (tx0, tx1) = (p[0][0].min(p[1][0]).min(p[2][0]), p[0][0].max(p[1][0]).max(p[2][0]));
            let (tz0, tz1) = (p[0][2].min(p[1][2]).min(p[2][2]), p[0][2].max(p[1][2]).max(p[2][2]));
            if tx1 < mn[0] || tx0 > mx[0] || tz1 < mn[1] || tz0 > mx[1] {
                next.push(t);
                continue;
            }
            let Some(pieces) = section.shape(p) else {
                next.push(t);
                continue;
            };
            for poly in pieces {
                // (a sliver along an edge of no area is dropped.)
                let area: f64 = (0..poly.len()).map(|i| poly[i][0] * poly[(i + 1) % poly.len()][2] - poly[(i + 1) % poly.len()][0] * poly[i][2]).sum::<f64>().abs() / 2.0;
                if area < MIN_PIECE_AREA_M2 {
                    continue;
                }
                let ids: Vec<u32> = poly
                    .iter()
                    .map(|q| {
                        if let Some(k) = (0..3).find(|&k| p[k] == *q) {
                            return t[k];
                        }
                        // the nearest corner's colour and normal.
                        let near = (0..3).min_by(|&a, &b| ((p[a][0] - q[0]).hypot(p[a][2] - q[2])).total_cmp(&(p[b][0] - q[0]).hypot(p[b][2] - q[2]))).unwrap();
                        let src = verts[t[near] as usize];
                        clipped.push(*q);
                        verts.push(Vertex { pos: [q[0] as f32, q[1] as f32, q[2] as f32], ..src });
                        (verts.len() - 1) as u32
                    })
                    .collect();
                for k in 1..ids.len() - 1 {
                    next.push([ids[0], ids[k], ids[k + 1]]);
                }
            }
        }
        tris = next;
    }
    if clipped.is_empty() {
        return tris.into_iter().flatten().collect();
    }
    // A clipped fan can contain collinear points. Their packed coordinates
    // can coincide or reverse the winding. Keep only top-facing terrain that
    // survives at the same area precision as its clipped polygon.
    tris.into_iter().filter(|t| {
        if t.iter().all(|&i| (i as usize) < first_clipped) { return true; }
        let [a, b, c] = t.map(|i| verts[i as usize].pos.map(f64::from));
        let twice_area = (b[2] - a[2]) * (c[0] - a[0]) - (b[0] - a[0]) * (c[2] - a[2]);
        twice_area >= 2.0 * MIN_PIECE_AREA_M2
    }).flatten().collect()
}

/// The triangles of one terrain tile's grid (`side` x `side` vertices,
/// the shared topology at `stride`) left once every triangle with a vertex in
/// `hole` is left out.
#[cfg(test)]
fn grid_without(side: usize, stride: usize, hole: &[bool]) -> Vec<u32> {
    let mut out = vec![];
    for z in (0..side - 1).step_by(stride) {
        for x in (0..side - 1).step_by(stride) {
            let i0 = (z * side + x) as u32;
            let i1 = i0 + stride as u32;
            let i2 = i0 + (stride * side) as u32;
            let i3 = i2 + stride as u32;
            for t in [[i0, i2, i1], [i1, i2, i3]] {
                if !t.iter().any(|&i| hole[i as usize]) {
                    out.extend_from_slice(&t);
                }
            }
        }
    }
    out
}

/// `build_scene`, and with `cuts` (eo-structures' world, `--structures
/// --tunnels`) the terrain cut where the tunnels' open stretches go through
/// the ground (`StructureWorld::terrain_cuts_in` per terrain tile): every
/// terrain vertex inside an open cut's ring (`canopy == false`), or within
/// `CUT_REACH_CELLS` of it (not behind a portal's face beside its canopy),
/// is lowered to the cut's floor (its smooth normal from the lowered
/// heights); the ground under a portal's canopy ring (`canopy == true`) is
/// cut out exactly, at both levels (`fit_tunnel_ground`), so only the tiles with
/// a canopy get their own vertices and index lists (the portal headwall's
/// top and ends cover the hole from above). Tiles without a cut,
/// and every tile with `cuts` None or a world without cuts (no `--tunnels`),
/// are built exactly as `build_scene` builds them.
pub fn build_scene_with_cuts(inp: &SceneInputs, cuts: Option<&eo_structures::StructureWorld>) -> anyhow::Result<Scene> {
    let cuts = cuts.filter(|w| !w.terrain_cuts().is_empty());
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
    // the far box's facade per building (far_box.rs), on every core; none with `--no-far-facade`.
    let far_facades: Vec<Option<crate::far_box::FarBox>> = if eo_kit::far_facade::enabled() { crate::far_box::facades(inp.city) } else { vec![] };
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
        let far = far_facades.get(i).and_then(|f| f.as_ref()).filter(|_| n >= 3);
        if let Some(fb) = far {
            crate::far_box::mesh(ring, base, top, fb, &mut verts, &mut idx, &mut ear, &mut tri);
        } else {
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
        }
        if let Some(o) = b.push(ObjectKind::Building, &verts, &idx) {
            if let Some(fb) = far {
                // the facade's words in the two a box never used (far_box.rs `pack`).
                let (w0, w1) = crate::far_box::pack(&fb.facade, floor - base);
                b.objects[o].far_first_index = w0;
                b.objects[o].far_index_count = w1;
                stats.far_facades += 1;
            }
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

    // --- The terrain surface pass (terrain_detail.rs): the survey's materials
    // at the vertices, and the 2 m relief patches round the photo eyes. ------
    let frame = eo_data::geo::GeoFrame::of_city(&inp.city.meta);
    let materials = eo_landmarks::features::gg3_ground("terrain-materials").then(|| terrain_detail::Materials { frame, land: inp.land, sea_y: SEA_LEVEL_Y as f32 });
    let materials = materials.filter(|_| terrain_detail::naip().is_some());
    let vertex_height = |cx: i64, cz: i64| -> f32 {
        let wx = ox + (cx as f32 + 0.5) * cell;
        let wz = oz + (cz as f32 + 0.5) * cell;
        match inp.carpet {
            Some(f) => f(wx, wz),
            None => g.ground(wx as f64, wz as f64) as f32,
        }
    };
    // a vertex's round-2 colour at any plan point (the tiles' rule, the cell's lidar flag)
    let colour_at = |wx: f32, wz: f32| -> u32 {
        let cx = ((wx - ox) / cell).floor();
        let cz = ((wz - oz) / cell).floor();
        let lidar = cx >= 0.0 && cz >= 0.0 && (cx as usize) < g.meta.width && (cz as usize) < g.meta.height && g.is_lidar_ground_cell(cx as usize, cz as usize);
        let class = inp.land.map(|l| l.class_at(wx as f64, wz as f64)).unwrap_or(50);
        inp.ground_tone.and_then(|f| f(wx, wz)).unwrap_or_else(|| land_color(class, !lidar))
    };
    let landing = eo_landmarks::golden_gate::approach_finish().then(terrain_detail::marin_ground::Ground::load);
    let lattice = terrain_detail::Lattice { ox: ox as f64, oz: oz as f64, cell: cell as f64, width: g.meta.width as i64, height: g.meta.height as i64 };
    let detail = if eo_landmarks::features::gg3_ground("terrain-detail") && inp.smooth_terrain {
        let street_tris: Vec<[[f32; 2]; 3]> = inp
            .street
            .map(|st| {
                st.objects
                    .iter()
                    .filter(|o| o.lod_role == LOD_STREET_FAR)
                    .flat_map(|o| {
                        st.indices[o.first_index as usize..(o.first_index + o.index_count) as usize].chunks_exact(3).map(move |t| {
                            [t[0], t[1], t[2]].map(|i| {
                                let v = st.vertices[(o.base_vertex + i as i32) as usize].pos;
                                [v[0], v[2]]
                            })
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let raw = |x: f64, z: f64| g.ground(x, z) as f32;
        let di = terrain_detail::Inputs {
            lattice,
            vertex_height: &vertex_height,
            raw_ground: &raw,
            colour: &colour_at,
            street_tris: &street_tris,
            buildings: inp.city.buildings.iter().map(|b| b.ring.as_slice()).collect(),
            frame,
            height_scale: eo_data::groundscale::height_scale(),
            sea_y: SEA_LEVEL_Y as f32,
        };
        terrain_detail::build(&di, materials.as_ref())
    } else {
        None
    };
    let detail = match &landing {
        Some(ground) => Some(ground.patches(detail, lattice, &vertex_height, &colour_at)),
        None => detail,
    };
    if let Some(d) = &detail {
        eprintln!(
            "terrain detail: {} patches of 64 m at 2 m ({} vertices; {} flat ones left to the tiles, {} rebuilt beside them), {} with relief (largest {:.2} m), {:.0} ms",
            d.stats.patches, d.stats.vertices, d.stats.dropped, d.stats.rebuilt, d.stats.relieved, d.stats.max_relief_m, d.stats.ms
        );
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
    // the relief patches: the tiles' topology (33 x 33 vertices at 2 m), the coarse level every
    // 4th (8 m). Pushed before the tiles: a reader that copies the terrain objects' vertices into
    // the 8 m grid as tiles (eo-app props/vegetation.rs `TerrainGrid::from_scene`) then has every
    // cell written last by the tile that holds it (the tiles keep all their vertices, the cells
    // under a patch included), not by a patch's 2 m vertices.
    if let Some(d) = &detail {
        let cut_world = cuts.filter(|_| tunnel_relief_cuts());
        for p in &d.patches {
            if let Some(world) = cut_world {
                let lo = p.verts[0].pos;
                let hi = p.verts[side * side - 1].pos;
                let near = world.terrain_cuts_in([lo[0] as f64, lo[2] as f64], [hi[0] as f64, hi[2] as f64]);
                if !near.is_empty() {
                    b.push_cut_ground(p.verts.clone(), &near, &|_, _| true, inp.terrain_lod, &mut stats);
                    continue;
                }
            }
            let o = b.push_shared(ObjectKind::Terrain, &p.verts, shared_first, shared_count);
            if inp.terrain_lod {
                b.objects[o].lod_role = LOD_TERRAIN;
                b.objects[o].far_first_index = coarse_first;
                b.objects[o].far_index_count = coarse_count;
            }
            stats.terrain_triangles += (shared_count / 3) as u64;
            stats.terrain_coarse_triangles += (coarse_count / 3) as u64;
        }
    }
    // Vertex rows of every terrain tile, computed in parallel (the street
    // world's carpet answers per vertex and is not free).
    let reach = CUT_REACH_CELLS * cell as f64;
    let tile_verts = |tx: usize, tz: usize| -> (Vec<Vertex>, bool, Vec<&eo_structures::TerrainCut>) {
        let mut verts = Vec::with_capacity(side * side);
        let mut any_land = false;
        // the tunnels' cuts near the tile (its vertices and the ring of cells round it).
        let near: Vec<&eo_structures::TerrainCut> = match cuts {
            Some(w) => {
                let lo = |c: usize, o: f32| (o + (c as f32 - 1.5) * cell) as f64 - reach;
                let hi = |c: usize, o: f32| (o + (c as f32 + TILE_CELLS as f32 + 1.5) * cell) as f64 + reach;
                w.terrain_cuts_in([lo(tx * TILE_CELLS, ox), lo(tz * TILE_CELLS, oz)], [hi(tx * TILE_CELLS, ox), hi(tz * TILE_CELLS, oz)])
            }
            None => vec![],
        };
        let height = |cx: i64, cz: i64| -> f32 {
            // Vertices sit on cell centres; past the grid's last
            // column/row the sampler clamps.
            let wx = ox + (cx as f32 + 0.5) * cell;
            let wz = oz + (cz as f32 + 0.5) * cell;
            let y = match inp.carpet {
                Some(f) => f(wx, wz),
                None => g.ground(wx as f64, wz as f64) as f32,
            };
            // an open cut lowers the ground in and beside it to its floor.
            match terrain_cut_floor(&near, wx as f64, wz as f64, reach) {
                Some(f) => y.min(f as f32),
                None => y,
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
                let mut ny = 1.0f32;
                if inp.smooth_terrain {
                    let dx = (ring[(z + 1) * n + x + 2] - ring[(z + 1) * n + x]) / (2.0 * cell);
                    let dz = (ring[(z + 2) * n + x + 1] - ring[z * n + x + 1]) / (2.0 * cell);
                    let nn = glam::Vec3::new(-dx, 1.0, -dz).normalize();
                    ny = nn.y;
                    vert.normal = eo_assets::glb::pack_normal(nn);
                }
                if let Some(m) = &materials {
                    let (c, w) = m.material(wx as f64, wz as f64, colour, ny as f64, y, cell as f64);
                    vert.color = c;
                    vert.normal |= (w as u8 as u32) << 24;
                }
                if let Some(w) = terrain_detail::sand_slope_byte(wx as f64, wz as f64, vert.color) {
                    vert.normal |= (w as u8 as u32) << 24;
                }
                if let Some(ground) = &landing { ground.apply(&mut vert); }
                verts.push(vert);
            }
        }
        // Fit the tile's terrain to each open approach and remove its covered portals.
        let (x0, x1) = (verts[0].pos[0] as f64, verts[side * side - 1].pos[0] as f64);
        let (z0, z1) = (verts[0].pos[2] as f64, verts[side * side - 1].pos[2] as f64);
        let cuts: Vec<&eo_structures::TerrainCut> = near.iter().copied().filter(|c| c.max[0] >= x0 && c.min[0] <= x1 && c.max[1] >= z0 && c.min[1] <= z1).collect();
        (verts, any_land, cuts)
    };
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(16);
    let all: Vec<(usize, usize)> = (0..tiles_z).flat_map(|tz| (0..tiles_x).map(move |tx| (tx, tz))).collect();
    let chunk = all.len().div_ceil(threads).max(1);
    let built: Vec<(Vec<Vertex>, bool, Vec<&eo_structures::TerrainCut>)> = std::thread::scope(|s| {
        let hs: Vec<_> = all.chunks(chunk).map(|c| s.spawn(move || c.iter().map(|&(tx, tz)| tile_verts(tx, tz)).collect::<Vec<_>>())).collect();
        hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    for (&(tx, tz), (tv, any_land, cuts)) in all.iter().zip(built) {
        let verts = tv;
        // (the terrain detail) the relief patches over this tile: their cells are left out of it at both levels.
        let holed = detail.as_ref().filter(|d| (0..TILE_CELLS as i64).any(|k| (0..TILE_CELLS as i64).step_by(terrain_detail::PATCH_CELLS as usize).any(|j| d.covers_quad((tx * TILE_CELLS) as i64 + j, (tz * TILE_CELLS) as i64 + k))));
        if !cuts.is_empty() || holed.is_some() {
            // Both detail levels follow the road cuts and leave out the portal canopies and relief patches.
            let (cx0, cz0) = ((tx * TILE_CELLS) as i64, (tz * TILE_CELLS) as i64);
            let keep = |x: usize, z: usize| holed.is_none_or(|d| !d.covers_quad(cx0 + x as i64, cz0 + z as i64));
            if b.push_cut_ground(verts, &cuts, &keep, inp.terrain_lod, &mut stats) {
                stats.terrain_tiles += 1;
                if !cuts.is_empty() {
                    stats.terrain_cut_tiles += 1;
                }
            }
            if any_land {
                tile_used[tz * tiles_x + tx] = true;
            }
            continue;
        }
        {
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

    if let Some(d) = detail {
        // the props and rocks seat on the drawn relief (eo_data::detail_ground) where the relief
        // moved the ground; elsewhere (None) on their own ground, which is the drawn 8 m surface there
        let d = std::sync::Arc::new(d);
        eo_data::detail_ground::set_provider(Box::new(move |x, z| d.relief_height(x, z)));
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

#[cfg(test)]
mod tests {
    use super::*;
    use eo_structures::TerrainCut;

    fn minus_canopies(verts: &mut Vec<Vertex>, side: usize, stride: usize, rings: &[&[[f64; 2]]], keep: &dyn Fn(usize, usize) -> bool) -> Vec<u32> {
        let sections: Vec<_> = rings.iter().flat_map(|ring| {
            cut(ring.to_vec(), 0.0, true).sections().collect::<Vec<_>>()
        }).collect();
        fit_tunnel_ground(verts, side, stride, &sections, keep)
    }

    fn cut(ring: Vec<[f64; 2]>, floor: f64, canopy: bool) -> TerrainCut {
        let (mut mn, mut mx) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for q in &ring {
            mn = [mn[0].min(q[0]), mn[1].min(q[1])];
            mx = [mx[0].max(q[0]), mx[1].max(q[1])];
        }
        TerrainCut { chain: 0, s0: 0.0, s1: 0.0, ring, floor, min: mn, max: mx, canopy }
    }

    /// Stage A8: an open cut (x 0..40, z -6..6, its portal face at x 40)
    /// lowers the vertices inside it and within reach of it in front of the
    /// face and under the portal's canopy (x 40 to 52, cut out of the grid),
    /// never those behind the face beside the canopy; a tile's grid without the
    /// triangles touching a hole keeps every other triangle of the shared
    /// topology, at both levels.
    #[test]
    fn an_open_cut_lowers_the_ground_in_front_of_its_portal_only() {
        let open = cut(vec![[0.0, 6.0], [40.0, 6.0], [40.0, -6.0], [0.0, -6.0]], 10.0, false);
        let canopy = cut(vec![[40.0, 8.0], [46.0, 8.0], [52.0, 8.0], [52.0, -8.0], [46.0, -8.0], [40.0, -8.0]], 10.0, true);
        let cuts = [&open, &canopy];
        let reach = 10.0;
        assert_eq!(terrain_cut_floor(&cuts, 20.0, 0.0, reach), Some(10.0), "inside");
        assert_eq!(terrain_cut_floor(&cuts, 20.0, 14.0, reach), Some(10.0), "beside, within reach");
        assert_eq!(terrain_cut_floor(&cuts, 20.0, 17.0, reach), None, "beyond reach");
        assert_eq!(terrain_cut_floor(&cuts, -8.0, 0.0, reach), Some(10.0), "past the street end, within reach");
        assert_eq!(terrain_cut_floor(&cuts, 45.0, 0.0, reach), Some(10.0), "under the canopy (cut out of the grid), within reach");
        assert_eq!(terrain_cut_floor(&cuts, 44.0, 12.0, reach), None, "behind the face, beside the canopy");
        assert_eq!(terrain_cut_floor(&cuts, 55.0, 0.0, reach), None, "behind the canopy, beyond reach");
        assert_eq!(terrain_cut_floor(&[&open], 45.0, 0.0, reach), Some(10.0), "(no portal: within reach)");
        // the grid without a hole's triangles.
        let side = TILE_CELLS + 1;
        let none = vec![false; side * side];
        assert_eq!(grid_without(side, 1, &none).len(), TILE_CELLS * TILE_CELLS * 6);
        assert_eq!(grid_without(side, TERRAIN_COARSE_STRIDE, &none).len(), (TILE_CELLS / TERRAIN_COARSE_STRIDE).pow(2) * 6);
        let mut hole = none.clone();
        hole[5 * side + 7] = true; // an inner vertex: its six triangles go
        let g = grid_without(side, 1, &hole);
        assert_eq!(g.len(), (TILE_CELLS * TILE_CELLS - 3) * 6);
        assert!(!g.contains(&((5 * side + 7) as u32)));
        assert_eq!(grid_without(side, TERRAIN_COARSE_STRIDE, &hole).len(), (TILE_CELLS / TERRAIN_COARSE_STRIDE).pow(2) * 6, "not on the coarse grid");
    }

    /// Stage A8: a portal's canopy is cut out of a tile's grid exactly: what
    /// is left covers the tile less the canopy's footprint (no more, no
    /// less), every piece stays in its triangle's plane, at both levels, and
    /// a canopy off the tile leaves the grid as it was.
    #[test]
    fn a_canopy_is_cut_out_of_the_terrain_exactly() {
        let side = TILE_CELLS + 1;
        let cell = 10.0f32;
        // a tilted plane y = 0.1 x + 0.05 z.
        let base: Vec<Vertex> = (0..side * side).map(|i| {
            let (x, z) = ((i % side) as f32 * cell, (i / side) as f32 * cell);
            Vertex { pos: [x, 0.1 * x + 0.05 * z, z], normal: 0, color: 0xff808080 }
        }).collect();
        let area = |verts: &[Vertex], idx: &[u32]| -> f64 {
            idx.chunks(3).map(|t| {
                let p: Vec<[f64; 3]> = t.iter().map(|&i| verts[i as usize].pos.map(|v| v as f64)).collect();
                ((p[1][0] - p[0][0]) * (p[2][2] - p[0][2]) - (p[2][0] - p[0][0]) * (p[1][2] - p[0][2])).abs() / 2.0
            }).sum()
        };
        let full = (TILE_CELLS as f64 * cell as f64).powi(2);
        // a canopy 12 m x 15.2 m, turned 20 degrees, its corners off the grid's vertices.
        let (c, s) = (20f64.to_radians().cos(), 20f64.to_radians().sin());
        let ring: Vec<[f64; 2]> = [[0.0, -7.6], [6.0, -7.6], [12.0, -7.6], [12.0, 7.6], [6.0, 7.6], [0.0, 7.6]].iter().map(|q| [133.3 + q[0] * c - q[1] * s, 147.9 + q[0] * s + q[1] * c]).collect();
        for stride in [1, TERRAIN_COARSE_STRIDE] {
            let mut verts = base.clone();
            let idx = minus_canopies(&mut verts, side, stride, &[ring.as_slice()], &|_, _| true);
            let left = area(&verts, &idx);
            assert!((left - (full - 12.0 * 15.2)).abs() < 1e-3, "stride {stride}: {left} of {full}");
            for v in &verts[side * side..] {
                assert!((v.pos[1] - (0.1 * v.pos[0] + 0.05 * v.pos[2])).abs() < 1e-3, "a cut vertex off its plane: {:?}", v.pos);
                // on one of the canopy's edge lines (the pieces are cut along them).
                let (x, z) = (v.pos[0] as f64, v.pos[2] as f64);
                let on_line = |a: [f64; 2], b: [f64; 2]| ((b[0] - a[0]) * (z - a[1]) - (b[1] - a[1]) * (x - a[0])).abs() / (b[0] - a[0]).hypot(b[1] - a[1]) < 1e-3;
                assert!((0..ring.len()).any(|i| on_line(ring[i], ring[(i + 1) % ring.len()])), "a new vertex off the canopy's edges: {:?}", v.pos);
            }
            // nothing left inside it (but slivers along its edges, of no area).
            for t in idx.chunks(3) {
                let m = t.iter().fold([0.0f64; 2], |a, &i| [a[0] + verts[i as usize].pos[0] as f64 / 3.0, a[1] + verts[i as usize].pos[2] as f64 / 3.0]);
                let edge = (0..ring.len()).map(|i| eo_structures::cut_ground::distance_to_segment(ring[i], ring[(i + 1) % ring.len()], m[0], m[1])).fold(f64::INFINITY, f64::min);
                assert!(edge < 1e-3 || !eo_street::point_in_ring(m[0], m[1], &ring), "a piece left under the canopy at {m:?}");
            }
        }
        // a canopy off the tile: the grid as it was.
        let mut verts = base.clone();
        let far: Vec<[f64; 2]> = ring.iter().map(|q| [q[0] + 1000.0, q[1]]).collect();
        let idx = minus_canopies(&mut verts, side, 1, &[far.as_slice()], &|_, _| true);
        assert_eq!(verts.len(), side * side);
        assert_eq!(idx, grid_without(side, 1, &vec![false; side * side]));
    }
}
