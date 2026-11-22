//! the far city's neighbourhood cells, baked ahead of time on the CPU.
//!
//! The city is cut into `CELL_M` (256 m) cells on the kit tile grid (2 x 2
//! kit tiles). Every building drawn as a building (kit, Blender-built and
//! code-built authored; not the records a landmark claims) belongs to exactly
//! one cell, by its footprint centroid. A cell is baked into one merged mesh
//! and a facade atlas:
//!
//!  * geometry, per building: its footprint prism from the foundation to the
//!    wall top (the rung 3 envelope of lod.js `envelopeParts`, cut at the
//!    eave instead of the ridge), a flat roof cap at the wall top, and the
//!    plan's roof forms (pitched slopes, gables, pediments, bulkheads,
//!    penthouses, monitors, towers, crowns) as they are in rung 2, so the
//!    roof line is kept. Every vertex carries its building's LOD group: the
//!    GPU draws or drops each building of a merged cell on its own
//!    (cull.wgsl `lod` decides, hlod_draw.wgsl reads it).
//!  * the facade atlas: each envelope wall is captured orthographically
//!    from the building's own drawn mesh (a kit building's rung 2, a Blender
//!    building's exterior GLB), 2 x 2 supersampled, nearest surface to an
//!    outside viewer wins, in a slab from 1.5 m behind the wall to 3 m in
//!    front of it. A texel holds the drawn material's mean colour (the kit
//!    map layer's own mean x the material's tint x the vertex paint: what the
//!    city shader averages to at range), darkened where the surface lies
//!    recessed behind the wall plane (window reveals, door recesses, shop
//!    entries), and in its alpha the glass coverage: the lit-window mask the
//!    far shader lights at night. Storey lines, cornices, string courses,
//!    the shop band (storefront glass, bulkheads, sign bands) and window
//!    rhythm come from the geometry itself. Not lit: the far shader lights
//!    it with the same light rig, shadows and fog as the near city.
//!
//! The capture runs on the CPU, not as a GPU orthographic render: it reads
//! the same rung meshes the GPU draws (so the far look follows the near
//! look), needs no GPU time, runs on the streaming worker
//! threads beside the game and is deterministic (the cache key is stable).
//!
//! Cached exactly like the kit tiles: `cache/hlod/<inputs key>/<cx>_<cz>.eohc`,
//! the key a SHA-256 of this file's own source text (embedded at build time),
//! the kit planner's inputs key (planner build, data files, palette, street
//! build), the material colours, the Blender buildings' GLB digests and
//! placements, and the bake options; each file ends with the SHA-256 of its
//! payload, and a damaged or truncated file is refused and re-baked.

use crate::authored::{ArenaBase, AuthoredSet, F_GLASS, F_TEXTURED, GpuLodGroup, GpuMaterial};
use crate::kit::KitSet;
use crate::scene::{LOD_ALWAYS, LOD_FAR, NO_GROUP, ObjectKind, Scene};
use bytemuck::{Pod, Zeroable};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

/// Side of a neighbourhood cell, metres (two kit tiles).
pub const CELL_M: f64 = 256.0;
pub const BAKE_VERSION: u32 = 3; // 2: pages stored with their mips, BC3 where the GPU has it; 3: glass seen through, the summary's wall, window rhythm and tower flag
/// Side of one atlas page (a layer of the GPU's page array), texels.
pub const PAGE: usize = 512;
/// Border texels round every chart (edge replicated): bilinear and the first
/// mips stay inside the chart.
pub const PAD: usize = 2;
pub const CACHE_MAGIC: &[u8; 4] = b"EOHC";
/// The bake's own source, embedded at build time: its digest is part of the
/// cache key, so a change to the baking code bakes into a new folder.
pub const BAKE_SOURCE: &str = include_str!("hlod_bake.rs");
pub const NO_CELL: u32 = u32::MAX;
/// Capture slab round each wall plane, metres: behind (recesses) and in front (bays, cornices, awnings).
pub const DEPTH_IN_M: f64 = 1.5;
pub const DEPTH_OUT_M: f64 = 3.0;
/// Supersampling per texel side.
const SS: usize = 2;

/// Colour-byte flags (the vertex colour's alpha byte).
pub const VF_TEXTURED: u32 = 1;
pub const VF_ROOF: u32 = 2;
/// a tower's wall (a downtown tower, or a civic building over `TOWER_MIN_M`): the far shader's curtain-wall look.
pub const VF_TOWER: u32 = 8;
/// a civic building this tall or taller is a tower to the far shader.
pub const TOWER_MIN_M: f64 = 40.0;

/// One far vertex (28 bytes): position, the face normal (snorm8 x 3) with
/// the chart's atlas page in its fourth byte, the atlas uv (unorm16 x 2, of
/// the page), the sRGB colour (flat faces; a textured face's fill) with the
/// flags in its alpha byte, and the building's LOD group.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
pub struct HlodVertex {
    pub pos: [f32; 3],
    pub normal: u32,
    pub uv: u32,
    pub color: u32,
    pub group: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BakeOptions {
    /// Nominal texel size of a facade, metres (the screen pixel at the switch
    /// distance: 52 deg vertical field, 1080 rows, 470 m: 0.42 m).
    pub texel_m: f32,
    /// A cell that needs more pages at that texel gets coarser texels.
    pub max_pages: usize,
    /// Party walls (a neighbour stands against them) take this many times the texel size.
    pub party_scale: f32,
    /// Pages stored as BC3 (the GPU's format where it has BC; set by the run time).
    pub bc3: bool,
    /// glass texels hold the glass as seen from outside by day (`glass_body`:
    /// the dark room through the pane), not the pane's own tint (off: `--no-far-glass`).
    pub glass_body: bool,
}

impl Default for BakeOptions {
    fn default() -> Self {
        Self { texel_m: 0.5, max_pages: 6, party_scale: 3.0, bc3: false, glass_body: true }
    }
}

/// Mip levels of a page as stored and uploaded (512 down to 16 texels).
pub const PAGE_MIPS: usize = 6;

/// Bytes of mip `m` of a page: RGBA8 rows, or BC3 blocks.
pub fn page_mip_bytes(m: usize, bc3: bool) -> usize {
    let r = (PAGE >> m).max(1);
    if bc3 { (r / 4).max(1).pow(2) * 16 } else { r * r * 4 }
}

/// A page's stored form: its `PAGE_MIPS` levels back to back (BC3 blocks or RGBA8).
pub fn page_levels(rgba: &[u8], bc3: bool) -> Vec<u8> {
    let mut out = vec![];
    for (m, level) in crate::authored::mip_chain(rgba, PAGE, true).into_iter().take(PAGE_MIPS).enumerate() {
        if bc3 {
            out.extend(bc3_encode(&level, PAGE >> m, PAGE >> m));
        } else {
            out.extend(level);
        }
    }
    out
}

/// Split a stored page into its levels.
pub fn split_levels(page: &[u8], bc3: bool) -> Vec<&[u8]> {
    let mut at = 0;
    (0..PAGE_MIPS)
        .map(|m| {
            let n = page_mip_bytes(m, bc3).min(page.len() - at);
            let s = &page[at..at + n];
            at += n;
            s
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The set: which building is in which cell.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// A kit (archetype) building: rung 2 of its plan.
    Kit,
    /// A Blender-built building: its exterior GLB (index into `AuthoredSet::buildings`).
    Blender(u32),
    /// A code-built authored building (no mesh here): its far box, coloured.
    Plain,
}

#[derive(Clone, Debug)]
pub struct CellBuilding {
    /// City building index.
    pub building: u32,
    /// Its LOD group (absolute).
    pub group: u32,
    pub source: Source,
}

#[derive(Clone, Debug)]
pub struct Cell {
    pub key: (i32, i32),
    pub buildings: Vec<CellBuilding>,
    /// Footprint rectangle of its buildings (x, z).
    pub min: [f32; 2],
    pub max: [f32; 2],
}

pub struct HlodSet {
    pub cells: Vec<Cell>,
    pub cell_of_key: HashMap<(i32, i32), u32>,
    /// Per LOD group (absolute, every group of the renderer): its cell or `NO_CELL`.
    pub group_cell: Vec<u32>,
    /// Groups of the code-built authored buildings (appended after every
    /// other group): the far layer switches their box off where it draws them.
    pub extra_groups: Vec<GpuLodGroup>,
    pub extra_group_base: u32,
    pub covered: usize,
    pub by_source: [usize; 3],
    pub skipped_claimed: usize,
    pub skipped_degenerate: usize,
    pub unmatched_plain: usize,
}

pub fn cell_key(x: f64, z: f64) -> (i32, i32) {
    ((x / CELL_M).floor() as i32, (z / CELL_M).floor() as i32)
}

fn ring_bounds(ring: &[[f32; 2]]) -> ([f32; 2], [f32; 2]) {
    let (mut mn, mut mx) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in ring {
        mn = [mn[0].min(p[0]), mn[1].min(p[1])];
        mx = [mx[0].max(p[0]), mx[1].max(p[1])];
    }
    (mn, mx)
}

impl HlodSet {
    /// Every building the city draws as a building, into its cell. `claimed`:
    /// record ids (without `sf-building-`) a landmark draws. `groups_before`:
    /// the renderer's group count without the far layer's own. A code-built
    /// authored building's always-drawn box in `scene` is found by its
    /// footprint bounds and given a group of its own (role FAR), so the far
    /// layer can stand in for it.
    pub fn prepare(city: &eo_data::City, kit: &KitSet, authored: &AuthoredSet, claimed: &HashSet<String>, scene: &mut Scene, groups_before: u32) -> Self {
        let blender_of_group: HashMap<u32, u32> = authored.buildings.iter().enumerate().map(|(k, b)| (b.group, k as u32)).collect();
        let key4 = |mn: [f32; 2], mx: [f32; 2]| (mn[0].to_bits(), mn[1].to_bits(), mx[0].to_bits(), mx[1].to_bits());
        let mut plain_objects: HashMap<(u32, u32, u32, u32), usize> = HashMap::new();
        for (o, ob) in scene.objects.iter().enumerate() {
            if ob.object_kind() == ObjectKind::Building as u32 && ob.lod_role == LOD_ALWAYS && ob.lod_group == NO_GROUP {
                plain_objects.insert(key4([ob.min[0], ob.min[2]], [ob.max[0], ob.max[2]]), o);
            }
        }
        let mut by_key: BTreeMap<(i32, i32), Vec<CellBuilding>> = BTreeMap::new();
        let mut extra_groups = vec![];
        let (mut skipped_claimed, mut skipped_degenerate, mut unmatched_plain) = (0, 0, 0);
        let mut by_source = [0usize; 3];
        for (i, b) in city.buildings.iter().enumerate() {
            if b.ring.len() < 3 {
                skipped_degenerate += 1;
                continue;
            }
            if claimed.contains(b.id.trim_start_matches("sf-building-")) {
                skipped_claimed += 1;
                continue;
            }
            let kg = kit.building_group.get(i).copied().unwrap_or(NO_GROUP);
            let (group, source) = if kg != NO_GROUP {
                if kit.groups[(kg - kit.group_base) as usize].kind == crate::kit::GROUP_KIND_CLAIMED {
                    skipped_claimed += 1;
                    continue;
                }
                (kg, Source::Kit)
            } else if let Some(&g) = authored.group_of_record.get(&b.id) {
                match blender_of_group.get(&g) {
                    Some(&k) => (g, Source::Blender(k)),
                    None => continue,
                }
            } else {
                let (mn, mx) = ring_bounds(&b.ring);
                let Some(&o) = plain_objects.get(&key4(mn, mx)) else {
                    unmatched_plain += 1;
                    continue;
                };
                let group = groups_before + extra_groups.len() as u32;
                let ob = &mut scene.objects[o];
                ob.lod_group = group;
                ob.lod_role = LOD_FAR;
                let c = [(ob.min[0] + ob.max[0]) * 0.5, (ob.min[1] + ob.max[1]) * 0.5, (ob.min[2] + ob.max[2]) * 0.5];
                let r = 0.5 * glam::Vec3::from(ob.max).distance(glam::Vec3::from(ob.min));
                // A Blender-like group (kind 0) that is never resident: its box draws unless the far layer does.
                extra_groups.push(GpuLodGroup { center: c, radius: r, ready: 0, detailed: 0, kind: 0, level: 0 });
                (group, Source::Plain)
            };
            by_source[match source {
                Source::Kit => 0,
                Source::Blender(_) => 1,
                Source::Plain => 2,
            }] += 1;
            let n = b.ring.len() as f64;
            let (sx, sz) = b.ring.iter().fold((0.0f64, 0.0f64), |a, p| (a.0 + p[0] as f64, a.1 + p[1] as f64));
            by_key.entry(cell_key(sx / n, sz / n)).or_default().push(CellBuilding { building: i as u32, group, source });
        }
        let total_groups = groups_before as usize + extra_groups.len();
        let mut group_cell = vec![NO_CELL; total_groups];
        let mut cells = vec![];
        let mut cell_of_key = HashMap::new();
        let mut covered = 0;
        for (key, buildings) in by_key {
            let (mut mn, mut mx) = ([f32::MAX; 2], [f32::MIN; 2]);
            for cb in &buildings {
                let (a, b) = ring_bounds(&city.buildings[cb.building as usize].ring);
                mn = [mn[0].min(a[0]), mn[1].min(a[1])];
                mx = [mx[0].max(b[0]), mx[1].max(b[1])];
                if let Some(slot) = group_cell.get_mut(cb.group as usize) {
                    *slot = cells.len() as u32;
                }
                covered += 1;
            }
            cell_of_key.insert(key, cells.len() as u32);
            cells.push(Cell { key, buildings, min: mn, max: mx });
        }
        Self { cells, cell_of_key, group_cell, extra_groups, extra_group_base: groups_before, covered, by_source, skipped_claimed, skipped_degenerate, unmatched_plain }
    }

    /// Camera to the cell's footprint rectangle, in the ground plane.
    pub fn cell_distance(&self, c: usize, cam: glam::Vec3) -> f32 {
        let cell = &self.cells[c];
        let dx = (cell.min[0] - cam.x).max(0.0).max(cam.x - cell.max[0]);
        let dz = (cell.min[1] - cam.z).max(0.0).max(cam.z - cell.max[1]);
        (dx * dx + dz * dz).sqrt()
    }
}

// ---------------------------------------------------------------------------
// Material colours: what a material averages to at range.
// ---------------------------------------------------------------------------

pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

pub fn linear_to_srgb_byte(c: f32) -> u8 {
    let v = c.clamp(0.0, 1.0);
    let s = if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0).round() as u8
}

fn hex_linear(rgb: u32) -> [f32; 3] {
    let c = |s: u32| srgb_to_linear(((rgb >> s) & 255) as f32 / 255.0);
    [c(16), c(8), c(0)]
}

fn mul3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2]]
}

/// Per GPU material: its mean linear albedo, and whether it is glass.
#[derive(Clone, Debug, Default)]
pub struct MaterialColours {
    pub rgb: Vec<[f32; 3]>,
    pub glass: Vec<bool>,
}

impl MaterialColours {
    /// draw.wgsl `shade`: a textured opaque material is its map x `base`; glass
    /// and untextured materials are `base`. `layer_means`: each kit map
    /// layer's mean linear albedo (`layer_means`).
    pub fn new(materials: &[GpuMaterial], layer_means: &[[f32; 3]]) -> Self {
        let mut rgb = Vec::with_capacity(materials.len());
        let mut glass = Vec::with_capacity(materials.len());
        for m in materials {
            let g = m.flags & F_GLASS != 0;
            let mut c = [m.base[0], m.base[1], m.base[2]];
            if !g && m.flags & F_TEXTURED != 0 && m.layer >= 0 {
                if let Some(l) = layer_means.get(m.layer as usize) {
                    c = mul3(c, *l);
                }
            }
            rgb.push(c);
            glass.push(g);
        }
        Self { rgb, glass }
    }

    pub fn of(&self, material: u32) -> ([f32; 3], bool) {
        (self.rgb.get(material as usize).copied().unwrap_or([0.5; 3]), self.glass.get(material as usize).copied().unwrap_or(false))
    }

    pub fn bytes(&self) -> Vec<u8> {
        let mut b = vec![];
        for (c, g) in self.rgb.iter().zip(&self.glass) {
            for v in c {
                b.extend_from_slice(&v.to_le_bytes());
            }
            b.push(*g as u8);
        }
        b
    }
}

/// Each kit map layer's mean linear albedo: the 1 x 1 mip of its albedo chain.
pub fn layer_means(upload: &crate::KitUpload) -> Vec<[f32; 3]> {
    upload
        .layers
        .iter()
        .map(|l| match l[0].last() {
            Some(px) if px.len() >= 3 => [srgb_to_linear(px[0] as f32 / 255.0), srgb_to_linear(px[1] as f32 / 255.0), srgb_to_linear(px[2] as f32 / 255.0)],
            _ => [0.5; 3],
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The cache key.
// ---------------------------------------------------------------------------

/// The cache folder name: this file's source, the kit planner's inputs key,
/// the material colours, the Blender buildings and the options.
pub fn inputs_key(kit_key: &str, colours: &MaterialColours, authored: &AuthoredSet, opts: &BakeOptions) -> String {
    let mut h = Sha256::new();
    h.update(b"eo-hlod");
    h.update(BAKE_VERSION.to_le_bytes());
    h.update(Sha256::digest(BAKE_SOURCE.as_bytes()));
    h.update(kit_key.as_bytes());
    h.update(colours.bytes());
    for b in &authored.buildings {
        h.update(b.id.as_bytes());
        h.update(b.asset.sha256.as_bytes());
        for v in b.placement.world_from_model.to_cols_array() {
            h.update(v.to_le_bytes());
        }
    }
    h.update(CELL_M.to_le_bytes());
    h.update((PAGE as u32).to_le_bytes());
    h.update((PAD as u32).to_le_bytes());
    h.update(opts.texel_m.to_le_bytes());
    h.update((opts.max_pages as u32).to_le_bytes());
    h.update(opts.party_scale.to_le_bytes());
    h.update([opts.bc3 as u8]);
    h.update([opts.glass_body as u8]);
    let d = h.finalize();
    d.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

pub fn cache_path(dir: &Path, key: &str, cell: (i32, i32)) -> PathBuf {
    dir.join("hlod").join(key).join(format!("{}_{}.eohc", cell.0, cell.1))
}

// ---------------------------------------------------------------------------
// A baked cell.
// ---------------------------------------------------------------------------

/// What the far layer knows of one building after the bake (the district
/// level and the night lights are made from these).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BuildingSummary {
    pub group: u32,
    pub building: u32,
    /// Mean linear colour of its captured facades (the fill where nothing was captured).
    pub facade: [f32; 3],
    /// Share of its facade texels that are glass.
    pub glass: f32,
    pub roof: [f32; 3],
    pub base: f32,
    pub top: f32,
    /// mean linear colour of the opaque (not glass) facade texels.
    pub wall: [f32; 3],
    /// the window rhythm (`Rhythm::floats`; zeros: none known).
    pub rhythm: [f32; 5],
    /// 1 for a tower (a downtown tower, or a civic building over `TOWER_MIN_M`).
    pub tower: f32,
}

impl BuildingSummary {
    /// its floats as stored (after the group and building words).
    pub const FLOATS: usize = 18;
    pub fn floats(&self) -> [f32; Self::FLOATS] {
        let (f, r, w, y) = (self.facade, self.roof, self.wall, self.rhythm);
        [f[0], f[1], f[2], self.glass, r[0], r[1], r[2], self.base, self.top, w[0], w[1], w[2], y[0], y[1], y[2], y[3], y[4], self.tower]
    }
    pub fn from_floats(group: u32, building: u32, v: &[f32]) -> Self {
        Self {
            group,
            building,
            facade: [v[0], v[1], v[2]],
            glass: v[3],
            roof: [v[4], v[5], v[6]],
            base: v[7],
            top: v[8],
            wall: [v[9], v[10], v[11]],
            rhythm: [v[12], v[13], v[14], v[15], v[16]],
            tower: v[17],
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CellStats {
    pub kit: u32,
    pub blender: u32,
    pub plain: u32,
    /// Kit or Blender buildings that fell back to a plain box (no plan, or no model).
    pub fallback: u32,
    pub charts: u32,
    pub texels: u64,
    pub source_triangles: u64,
    pub bake_ms: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BakedCell {
    pub key: (i32, i32),
    pub vertices: Vec<HlodVertex>,
    /// Local to the cell's first vertex.
    pub indices: Vec<u32>,
    /// `PAGE` x `PAGE` pages (sRGB colour, linear glass coverage), each its
    /// `PAGE_MIPS` levels back to back (`page_levels`): BC3 blocks when `bc3`, else RGBA8.
    pub pages: Vec<Vec<u8>>,
    pub bc3: bool,
    pub texel_m: f32,
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub summaries: Vec<BuildingSummary>,
    pub stats: CellStats,
}

impl BakedCell {
    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(self.vertices.len() * 28 + self.indices.len() * 4 + self.pages.len() * PAGE * PAGE * 4 + 256);
        let u = |b: &mut Vec<u8>, v: u32| b.extend_from_slice(&v.to_le_bytes());
        let f = |b: &mut Vec<u8>, v: f32| b.extend_from_slice(&v.to_le_bytes());
        b.extend_from_slice(CACHE_MAGIC);
        u(&mut b, BAKE_VERSION);
        u(&mut b, self.key.0 as u32);
        u(&mut b, self.key.1 as u32);
        u(&mut b, self.vertices.len() as u32);
        u(&mut b, self.indices.len() as u32);
        u(&mut b, self.pages.len() as u32);
        u(&mut b, PAGE as u32 | if self.bc3 { 1 << 31 } else { 0 });
        u(&mut b, self.pages.first().map(|p| p.len()).unwrap_or(0) as u32);
        u(&mut b, self.summaries.len() as u32);
        f(&mut b, self.texel_m);
        for v in self.min.iter().chain(&self.max) {
            f(&mut b, *v);
        }
        let s = &self.stats;
        for v in [s.kit, s.blender, s.plain, s.fallback, s.charts] {
            u(&mut b, v);
        }
        b.extend_from_slice(&s.texels.to_le_bytes());
        b.extend_from_slice(&s.source_triangles.to_le_bytes());
        f(&mut b, s.bake_ms);
        b.extend_from_slice(bytemuck::cast_slice(&self.vertices));
        b.extend_from_slice(bytemuck::cast_slice(&self.indices));
        for p in &self.pages {
            b.extend_from_slice(p);
        }
        for m in &self.summaries {
            u(&mut b, m.group);
            u(&mut b, m.building);
            for v in m.floats() {
                f(&mut b, v);
            }
        }
        let digest = Sha256::digest(&b);
        b.extend_from_slice(&digest);
        b
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < 32 + 4 || &bytes[0..4] != CACHE_MAGIC {
            return Err("not a far cell".into());
        }
        let (body, digest) = bytes.split_at(bytes.len() - 32);
        if Sha256::digest(body).as_slice() != digest {
            return Err("payload digest mismatch".into());
        }
        let mut at = 4usize;
        let take = |at: &mut usize, n: usize| -> Result<&[u8], String> {
            let s = body.get(*at..*at + n).ok_or("truncated")?;
            *at += n;
            Ok(s)
        };
        let u = |at: &mut usize| -> Result<u32, String> { Ok(u32::from_le_bytes(take(at, 4)?.try_into().unwrap())) };
        let f = |at: &mut usize| -> Result<f32, String> { Ok(f32::from_le_bytes(take(at, 4)?.try_into().unwrap())) };
        if u(&mut at)? != BAKE_VERSION {
            return Err("bake version".into());
        }
        let key = (u(&mut at)? as i32, u(&mut at)? as i32);
        let (nv, ni, np, page) = (u(&mut at)? as usize, u(&mut at)? as usize, u(&mut at)? as usize, u(&mut at)?);
        let page_len = u(&mut at)? as usize;
        let ns = u(&mut at)? as usize;
        let bc3 = page & (1 << 31) != 0;
        if (page & !(1 << 31)) as usize != PAGE || (np > 0 && page_len != (0..PAGE_MIPS).map(|m| page_mip_bytes(m, bc3)).sum::<usize>()) {
            return Err("page size".into());
        }
        let texel_m = f(&mut at)?;
        let mut min = [0f32; 3];
        let mut max = [0f32; 3];
        for v in min.iter_mut().chain(max.iter_mut()) {
            *v = f(&mut at)?;
        }
        let mut stats = CellStats { kit: u(&mut at)?, blender: u(&mut at)?, plain: u(&mut at)?, fallback: u(&mut at)?, charts: u(&mut at)?, ..Default::default() };
        stats.texels = u64::from_le_bytes(take(&mut at, 8)?.try_into().unwrap());
        stats.source_triangles = u64::from_le_bytes(take(&mut at, 8)?.try_into().unwrap());
        stats.bake_ms = f(&mut at)?;
        let vertices: Vec<HlodVertex> = bytemuck::pod_collect_to_vec(take(&mut at, nv * std::mem::size_of::<HlodVertex>())?);
        let indices: Vec<u32> = bytemuck::pod_collect_to_vec(take(&mut at, ni * 4)?);
        let mut pages = Vec::with_capacity(np);
        for _ in 0..np {
            pages.push(take(&mut at, page_len)?.to_vec());
        }
        let mut summaries = Vec::with_capacity(ns);
        for _ in 0..ns {
            let (group, building) = (u(&mut at)?, u(&mut at)?);
            let mut v = [0f32; BuildingSummary::FLOATS];
            for x in v.iter_mut() {
                *x = f(&mut at)?;
            }
            summaries.push(BuildingSummary::from_floats(group, building, &v));
        }
        if at != body.len() {
            return Err("trailing bytes".into());
        }
        if indices.iter().any(|&i| i as usize >= nv) {
            return Err("index out of range".into());
        }
        Ok(Self { key, vertices, indices, pages, bc3, texel_m, min, max, summaries, stats })
    }
}

// ---------------------------------------------------------------------------
// The bake.
// ---------------------------------------------------------------------------

/// Everything a bake reads.
pub struct BakeContext {
    pub set: Arc<HlodSet>,
    pub kit: Arc<KitSet>,
    pub kit_city: Arc<eo_kit::city::KitCity>,
    pub authored: Arc<AuthoredSet>,
    pub city: Arc<eo_data::City>,
    pub colours: Arc<MaterialColours>,
    pub repo_root: PathBuf,
    /// The model cache (Blender GLBs) and the far cells' cache.
    pub cache_dir: Option<PathBuf>,
    pub key: String,
    pub opts: BakeOptions,
}

/// A source triangle the facades are captured from.
#[derive(Clone, Copy, Debug)]
struct SrcTri {
    p: [[f32; 3]; 3],
    n: [f32; 3],
    rgb: [f32; 3],
    glass: bool,
    /// The normal's sign is not known (a face normal of unknown winding).
    two_sided: bool,
}

/// One building's far form before packing.
struct Far {
    group: u32,
    building: u32,
    /// Footprint, open ring (no closing duplicate), x/z.
    ring: Vec<[f64; 2]>,
    /// Per edge: a neighbour stands against it.
    party: Vec<bool>,
    bottom: f64,
    top: f64,
    src: Vec<SrcTri>,
    /// Roof forms kept as geometry: triangle, face normal, linear colour.
    roof: Vec<([[f32; 3]; 3], [f32; 3], [f32; 3])>,
    wall_rgb: [f32; 3],
    roof_rgb: [f32; 3],
    textured: bool,
    /// the window rhythm (`Rhythm::floats`; zeros: none known) and whether it is a tower.
    rhythm: [f32; 5],
    tower: bool,
    /// the measured second colour patch and the parapet band.
    roofs: RoofCap,
}

/// a building's window rhythm as the far shader draws it where the
/// atlas has no texels (the district level): the bay period along the wall,
/// the storey period up it, the window's share of each, and where in a storey
/// the window starts (world y over the storey period, fractional part).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rhythm {
    pub bay_m: f32,
    pub storey_m: f32,
    pub window_w: f32,
    pub window_h: f32,
    pub phase: f32,
}

impl Rhythm {
    pub fn floats(&self) -> [f32; 5] {
        [self.bay_m, self.storey_m, self.window_w, self.window_h, self.phase]
    }

    /// From a kit plan's records: the median bay width of its outside (not
    /// party) walls, the median upper storey height, the median window width
    /// and height of the upper storeys' openings over them (doors left out),
    /// and the median window sill over the storey period. None without bays,
    /// storeys or upper windows.
    pub fn of_plan(plan: &eo_kit::elevation::Plan) -> Option<Self> {
        fn median(mut v: Vec<f64>) -> Option<f64> {
            if v.is_empty() {
                return None;
            }
            v.sort_by(|a, b| a.total_cmp(b));
            Some(v[v.len() / 2])
        }
        let upper: Vec<f64> = plan.storeys.iter().filter(|s| s.index >= 1).map(|s| s.height).collect();
        let storey = median(if upper.is_empty() { plan.storeys.iter().map(|s| s.height).collect() } else { upper })?;
        let outside = || plan.edges.iter().filter(|e| !e.party);
        let bay = median(outside().flat_map(|e| e.bays.iter().map(|b| b.width)).filter(|w| *w > 0.3).collect())?;
        let multi = plan.storeys.len() > 1;
        let windows: Vec<&eo_kit::elevation::OpeningRecord> = outside()
            .flat_map(|e| e.openings.iter())
            .filter(|o| (!multi || o.storey >= 1) && !o.type_.contains("door") && !o.type_.contains("garage") && !o.type_.contains("entry"))
            .collect();
        let w = median(windows.iter().map(|o| o.s1 - o.s0).collect())?;
        let h = median(windows.iter().map(|o| o.y1 - o.y0).collect())?;
        let sill = median(windows.iter().map(|o| o.y0).collect())?;
        Self::from_medians(storey, bay, w, h, sill)
    }

    /// The rhythm from the plan's medians (storey and bay periods, window width
    /// and height, sill height, metres).
    pub fn from_medians(storey: f64, bay: f64, w: f64, h: f64, sill: f64) -> Option<Self> {
        if !(storey > 1.5 && bay > 0.3) {
            return None;
        }
        let (mut ww, wh, mut bay, mut storey) = ((w / bay).clamp(0.05, 0.95), (h / storey).clamp(0.05, 0.95), bay, storey);
        // A tower's plan storey can be a band of several floors, and a curtain
        // wall's bay a run of several panes: draw floors of about 3.8 m and
        // panes of about 1.5 m, at the same glass shares.
        let floors = (storey / FLOOR_M).round().max(1.0);
        if storey > 6.0 && floors > 1.0 {
            storey /= floors;
        }
        let panes = (bay / PANE_M).round().max(1.0);
        if ww > 0.85 && bay > 3.0 && panes > 1.0 {
            bay /= panes;
            ww = 0.88;
        }
        Some(Self { bay_m: bay as f32, storey_m: storey as f32, window_w: ww as f32, window_h: wh as f32, phase: (sill / storey).rem_euclid(1.0) as f32 })
    }
}

/// the floor height a tower's multi-floor plan storey is drawn in, and
/// the pane width a curtain wall's wide bay is drawn in (metres).
pub const FLOOR_M: f64 = 3.8;
pub const PANE_M: f64 = 1.5;

/// a glass texel's colour as the far city sees it by day: the pane
/// seen through to a dark room, tinted by the glass (a dark pane keeps its
/// own colour). `bright` is the pane's own base colour (its transmission tint).
pub fn glass_body(bright: [f32; 3]) -> [f32; 3] {
    bright.map(|c| c.min(0.1 * c + GLASS_ROOM))
}

/// the mean linear albedo of a room seen through a pane by day.
pub const GLASS_ROOM: f32 = 0.025;

/// what the far roof cap draws besides its one colour.
#[derive(Clone, Copy, Debug, Default)]
struct RoofCap {
    /// The second measured colour (linear), the direction it lies in from the
    /// footprint centroid, and its share of the roof.
    patch: Option<([f32; 3], [f64; 2], f32)>,
    /// A pitched roof covers the cap: no parapet band.
    pitched: bool,
}

/// the measured roof of building `b` and the share of it to take (none: no table, too unsure).
fn roof_measure(ctx: &BakeContext, b: u32) -> Option<(eo_data::roof_colours::RoofMeasure, f32)> {
    let t = eo_data::roof_colours::table()?;
    let m = t.get(&ctx.city.buildings.get(b as usize)?.id)?;
    let w = eo_kit::roof_measured::weight(m.confidence);
    (w > 0.0).then_some((m, w))
}

/// the roof colour toward the measured one (moved off the walls' lightness when close).
fn measured_roof(old: [f32; 3], wall: [f32; 3], m: &eo_data::roof_colours::RoofMeasure, w: f32) -> [f32; 3] {
    let target = eo_kit::roof_measured::target(m, Some(wall)); // the imagery's cast off (EO_ROOF_NEUTRAL)
    [0, 1, 2].map(|k| old[k] + (target[k] - old[k]) * w)
}

/// the patch of the second measured colour, when it is a real share in one place.
fn roof_patch(m: &eo_data::roof_colours::RoofMeasure, w: f32) -> Option<([f32; 3], [f64; 2], f32)> {
    let d = [m.second_at[0] as f64, m.second_at[1] as f64];
    let len = d[0].hypot(d[1]);
    if !eo_data::roof_colours::feature_on("patches") || w < 0.5 || m.second_share < 0.15 || len < 0.15 {
        return None;
    }
    Some((eo_kit::roof_measured::neutral(eo_data::roof_colours::rgb8_linear(m.second)), [d[0] / len, d[1] / len], m.second_share.min(0.6)))
}

/// rung 2 roof plant kinds of which the largest two per building are kept in the far mesh.
const FAR_PLANT_KINDS: [&str; 3] = ["roof-plant", "roof-hatch", "roof-duct"];
const FAR_PLANT_KEEP: usize = 2;

/// Roof forms kept as geometry in the far mesh (rung 2 parts of these kinds).
pub fn is_roof_form(kind: &str) -> bool {
    matches!(
        kind,
        "roof-slope" | "roof-gable" | "gable" | "gable-infill" | "pediment" | "hip" | "pitch" | "tower" | "crown" | "crown-fin" | "roof-penthouse" | "roof-bulkhead" | "roof-monitor" | "roof-monitor-cap" | "roof-monitor-glass"
    )
}

/// Parts whose tops make the wall top (the eave or the parapet line).
pub fn is_wall_kind(kind: &str) -> bool {
    matches!(kind, "wall" | "parapet" | "parapet-band" | "parapet-fascia" | "spandrel" | "pier" | "pilaster")
}

fn open_ring(mut r: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    while r.len() > 3 {
        let (a, b) = (r[0], r[r.len() - 1]);
        if (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6 {
            r.pop();
        } else {
            break;
        }
    }
    r
}

fn face_normal(p: &[[f32; 3]; 3]) -> [f32; 3] {
    let a = glam::Vec3::from(p[0]);
    let n = (glam::Vec3::from(p[1]) - a).cross(glam::Vec3::from(p[2]) - a);
    n.normalize_or_zero().to_array()
}

fn plain_far(ctx: &BakeContext, b: u32, group: u32) -> Far {
    let bld = &ctx.city.buildings[b as usize];
    let base = ctx.kit.base_y.get(b as usize).copied().unwrap_or(0.0) as f64;
    let (_, head) = crate::surfaces::far_wall_tones(bld);
    let wall = [srgb_to_linear(head[0] as f32 / 255.0), srgb_to_linear(head[1] as f32 / 255.0), srgb_to_linear(head[2] as f32 / 255.0)];
    let ring = open_ring(bld.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect());
    let n = ring.len();
    let mut roof_rgb = mul3(wall, [0.8; 3]);
    let mut roofs = RoofCap::default();
    if let Some((m, w)) = roof_measure(ctx, b) {
        roof_rgb = measured_roof(roof_rgb, wall, &m, w);
        roofs.patch = roof_patch(&m, w);
    }
    Far { group, building: b, ring, party: vec![false; n], bottom: base, top: base + bld.height as f64, src: vec![], roof: vec![], wall_rgb: wall, roof_rgb, textured: false, rhythm: [0.0; 5], tower: false, roofs }
}

fn kit_far(ctx: &BakeContext, b: u32, group: u32) -> Option<Far> {
    use eo_kit::pipeline as kp;
    let kc = &ctx.kit_city;
    let set = &ctx.kit;
    let i = b as usize;
    let rec = kc.records.get(i)?;
    let (base_y, skirt_bottom) = match &set.streets {
        Some(sw) => {
            let (floor, f, _) = crate::kit::kit_floor(kc, sw, b);
            (floor.y, Some(f.bottom_y))
        }
        None => (set.base_y[i] as f64, None),
    };
    let planned = kp::plan_building(rec, &kc.world, b, base_y)?;
    let drawn = kp::drawn_materials(&planned, 2);
    let parts = kp::rung_parts(&planned, 2);
    let mut clamp = eo_kit::mesh::Clamp::new(&planned.plan, &planned.frame);
    let wall = drawn.materials.get("wall").cloned().unwrap_or_else(|| "brick-running-bond".into());
    let fallback = *set.palette_index.get("painted-stucco").unwrap_or(&0);
    let colour_of = |name: &str| -> ([f32; 3], bool) {
        let pal = set.palette_index.get(kp::base_material_name(name)).copied().unwrap_or(fallback);
        let (mut c, glass) = ctx.colours.of(set.palette_base + pal as u32);
        if !glass {
            if let Some(t) = drawn.tints.get(name) {
                c = mul3(c, hex_linear(*t));
            }
        }
        (c, glass)
    };
    let mut src = vec![];
    let mut roof = vec![];
    let mut wall_top = f64::MIN;
    let mut sink = eo_kit::mesh::Sink::default();
    // the measured roof colour exactly on the roof slots (the near tint
    // reaches it only through the finish's nominal colour), the largest plant kept.
    let measured = roof_measure(ctx, b);
    let measured_rgb = measured.map(|(m, w)| (m, w, measured_roof(colour_of(&wall).0, colour_of(&wall).0, &m, 1.0)));
    let plant_on = eo_data::roof_colours::feature_on("plant");
    let mut plant: Vec<(f32, Vec<([[f32; 3]; 3], [f32; 3], [f32; 3])>)> = vec![];
    let mut pitched = false;
    for part in &parts {
        let name = drawn.materials.get(&part.slot).cloned().filter(|s| !s.is_empty()).unwrap_or_else(|| wall.clone());
        let (mut rgb, glass) = colour_of(&name);
        sink.positions.clear();
        sink.normals.clear();
        sink.indices.clear();
        eo_kit::mesh::emit_part(&mut sink, part, &mut clamp);
        if is_wall_kind(&part.kind) {
            wall_top = wall_top.max(part.y1);
        }
        let form = is_roof_form(&part.kind);
        // a roof-slot form takes the measured colour (blended by confidence).
        if let Some((_, w, target)) = measured_rgb {
            if form && !glass && eo_kit::roof_measured::ROOF_SLOTS.contains(&part.slot.as_str()) {
                rgb = [0, 1, 2].map(|k| rgb[k] + (target[k] - rgb[k]) * w);
            }
        }
        pitched |= matches!(part.kind.as_str(), "roof-slope" | "roof-gable" | "gable" | "hip" | "pitch");
        let kept_plant = plant_on && FAR_PLANT_KINDS.contains(&part.kind.as_str());
        let mut plant_tris = vec![];
        let mut plant_area = 0f32;
        for t in sink.indices.chunks_exact(3) {
            let p = [0, 1, 2].map(|k| {
                let q = sink.positions[t[k] as usize];
                [q[0] as f32, q[1] as f32, q[2] as f32]
            });
            let n = sink.normals[t[0] as usize];
            src.push(SrcTri { p, n, rgb, glass, two_sided: false });
            if form {
                roof.push((p, n, rgb));
            } else if kept_plant {
                if n[1] > 0.7 {
                    let a = glam::Vec3::from(p[0]);
                    plant_area += 0.5 * (glam::Vec3::from(p[1]) - a).cross(glam::Vec3::from(p[2]) - a).length();
                }
                plant_tris.push((p, n, rgb));
            }
        }
        if !plant_tris.is_empty() {
            plant.push((plant_area, plant_tris));
        }
    }
    // the largest roof plant (by plan area) stays as boxes in the far mesh.
    plant.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    for (_, tris) in plant.into_iter().take(FAR_PLANT_KEEP) {
        roof.extend(tris);
    }
    let lowest = set.base_y[i] as f64;
    let bottom = skirt_bottom.unwrap_or(base_y).min(base_y).min(lowest);
    // The foundation skirt (cook_tile draws it with every rung).
    if let Some(sb) = skirt_bottom {
        let ring: Vec<[f32; 2]> = rec.polygon.iter().map(|p| [p.x as f32, p.z as f32]).collect();
        let skirt = set.palette_index.get("concrete-precast").copied().unwrap_or(fallback);
        let (c, _) = ctx.colours.of(set.palette_base + skirt as u32);
        let c = mul3(c, hex_linear(0xd8d8d8));
        let mut sv = vec![];
        let mut si = vec![];
        crate::scene::skirt_quads(&ring, base_y as f32, sb as f32, 0, &mut sv, &mut si);
        for t in si.chunks_exact(3) {
            let p = [sv[t[0] as usize].pos, sv[t[1] as usize].pos, sv[t[2] as usize].pos];
            src.push(SrcTri { p, n: face_normal(&p), rgb: c, glass: false, two_sided: true });
        }
    }
    let roof_top = base_y + planned.plan.envelope.roof_top;
    let top = if wall_top > base_y + 1.0 { wall_top.min(roof_top + 0.3) } else { roof_top };
    let top = top.max(base_y + 2.0);
    let frame = &planned.frame;
    let (ring, party): (Vec<[f64; 2]>, Vec<bool>) = match &frame.source_polygon {
        Some(p) if p.len() >= 3 => {
            let r = open_ring(p.iter().map(|q| [q.x, q.z]).collect());
            let n = r.len();
            (r, vec![false; n])
        }
        _ => (frame.edges.iter().map(|e| [e.start.x, e.start.z]).collect(), frame.edges.iter().map(|e| e.party).collect()),
    };
    if ring.len() < 3 {
        return None;
    }
    let (wall_rgb, _) = colour_of(&wall);
    let roof_name = drawn.materials.get("roof").cloned().filter(|s| !s.is_empty()).unwrap_or_else(|| wall.clone());
    let (mut roof_rgb, _) = colour_of(&roof_name);
    let mut roofs = RoofCap { pitched, ..Default::default() };
    if let Some((m, w, _)) = measured_rgb {
        roof_rgb = measured_roof(roof_rgb, wall_rgb, &m, w);
        roofs.patch = roof_patch(&m, w);
    }
    // the window rhythm and the tower flag.
    let rhythm = Rhythm::of_plan(&planned.plan).map(|r| r.floats()).unwrap_or([0.0; 5]);
    let arch = planned.classified.archetype;
    let tower = arch == "downtown-tower" || (arch == "civic-concrete" && top - base_y >= TOWER_MIN_M);
    Some(Far { group, building: b, ring, party, bottom, top, src, roof, wall_rgb, roof_rgb, textured: true, rhythm, tower, roofs })
}

fn blender_far(ctx: &BakeContext, b: u32, group: u32, k: u32) -> Option<Far> {
    let ab = ctx.authored.buildings.get(k as usize)?;
    let (model, _) = eo_assets::load_model(&ctx.repo_root, ctx.cache_dir.as_deref(), &ab.asset).ok()?;
    let up = crate::authored::model_upload(&ctx.authored, k as usize, &model, ArenaBase { vertex: 0, index: 0, object: 0 }, &ctx.authored.materials.gpu).ok()?;
    let mut src = vec![];
    let (mut wall_acc, mut wall_w, mut roof_acc, mut roof_w) = ([0f32; 3], 0f32, [0f32; 3], 0f32);
    for o in &up.objects {
        let first = o.first_index.saturating_sub(ab.slot.index_offset) as usize;
        let n = o.index_count as usize;
        let Some(ix) = up.indices.get(first..first + n) else { continue };
        let (rgb, glass) = ctx.colours.of(o.material);
        for t in ix.chunks_exact(3) {
            let v = [t[0], t[1], t[2]].map(|i| up.vertices[i as usize]);
            let p = [v[0].pos, v[1].pos, v[2].pos];
            let fnrm = face_normal(&p);
            let (n, two_sided) = if v[0].normal == 0 { (fnrm, true) } else { (eo_assets::glb::unpack_normal(v[0].normal).normalize_or_zero().to_array(), false) };
            let a = glam::Vec3::from(p[0]);
            let area = 0.5 * (glam::Vec3::from(p[1]) - a).cross(glam::Vec3::from(p[2]) - a).length();
            if !glass {
                if n[1].abs() > 0.7 {
                    roof_acc = [roof_acc[0] + rgb[0] * area, roof_acc[1] + rgb[1] * area, roof_acc[2] + rgb[2] * area];
                    roof_w += area;
                } else if n[1].abs() < 0.3 {
                    wall_acc = [wall_acc[0] + rgb[0] * area, wall_acc[1] + rgb[1] * area, wall_acc[2] + rgb[2] * area];
                    wall_w += area;
                }
            }
            src.push(SrcTri { p, n, rgb, glass, two_sided });
        }
    }
    let bld = &ctx.city.buildings[b as usize];
    let base = ctx.kit.base_y.get(b as usize).copied().unwrap_or(0.0) as f64;
    let ring = open_ring(bld.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect());
    let n = ring.len();
    let avg = |a: [f32; 3], w: f32| if w > 0.0 { [a[0] / w, a[1] / w, a[2] / w] } else { [0.45; 3] };
    let (wall_rgb, mut roof_rgb) = (avg(wall_acc, wall_w), avg(roof_acc, roof_w));
    let mut roofs = RoofCap { pitched: true, ..Default::default() }; // the model draws its own roof edge
    if let Some((m, w)) = roof_measure(ctx, b) {
        roof_rgb = measured_roof(roof_rgb, wall_rgb, &m, w); // the cap of a hand-built building
        roofs.patch = roof_patch(&m, w);
    }
    Some(Far { group, building: b, ring, party: vec![false; n], bottom: base, top: base + bld.height as f64, src, roof: vec![], wall_rgb, roof_rgb, textured: true, rhythm: [0.0; 5], tower: bld.height as f64 >= TOWER_MIN_M * 2.0, roofs })
}

/// One wall chart: a stretch of a footprint edge between two heights.
#[derive(Clone, Copy, Debug)]
struct Chart {
    far: usize,
    edge: usize,
    u0: f64,
    u1: f64,
    v0: f64,
    v1: f64,
    w: usize,
    h: usize,
    page: usize,
    x: usize,
    y: usize,
}

/// Outward unit normal of every edge of an open ring (x/z).
fn edge_frames(ring: &[[f64; 2]]) -> Vec<([f64; 2], [f64; 2], f64)> {
    let n = ring.len();
    let mut area = 0.0;
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        area += a[0] * b[1] - b[0] * a[1];
    }
    (0..n)
        .map(|i| {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
            let len = (dx * dx + dz * dz).sqrt();
            if len < 1e-9 {
                return ([1.0, 0.0], [0.0, 1.0], 0.0);
            }
            let u = [dx / len, dz / len];
            let nrm = if area > 0.0 { [u[1], -u[0]] } else { [-u[1], u[0]] };
            (u, nrm, len)
        })
        .collect()
}

fn make_charts(fars: &[Far], texel: f64, party_scale: f64) -> Vec<Chart> {
    let inner = PAGE - 2 * PAD;
    let mut out = vec![];
    for (fi, f) in fars.iter().enumerate() {
        if !f.textured {
            continue;
        }
        let frames = edge_frames(&f.ring);
        let height = (f.top - f.bottom).max(0.1);
        for (e, &(_, _, len)) in frames.iter().enumerate() {
            if len < 0.05 {
                continue;
            }
            let t = if f.party.get(e).copied().unwrap_or(false) { texel * party_scale } else { texel };
            let w = ((len / t).ceil() as usize).max(1);
            let h = ((height / t).ceil() as usize).max(1);
            let (nu, nv) = (w.div_ceil(inner), h.div_ceil(inner));
            for iu in 0..nu {
                for iv in 0..nv {
                    out.push(Chart {
                        far: fi,
                        edge: e,
                        u0: len * iu as f64 / nu as f64,
                        u1: len * (iu + 1) as f64 / nu as f64,
                        v0: f.top - height * (iv + 1) as f64 / nv as f64,
                        v1: f.top - height * iv as f64 / nv as f64,
                        w: w.div_ceil(nu),
                        h: h.div_ceil(nv),
                        page: 0,
                        x: 0,
                        y: 0,
                    });
                }
            }
        }
    }
    out
}

/// Shelf packing: tallest first. Returns the page count.
fn pack(charts: &mut [Chart]) -> usize {
    let mut order: Vec<usize> = (0..charts.len()).collect();
    order.sort_by(|&a, &b| (charts[b].h, charts[b].w).cmp(&(charts[a].h, charts[a].w)));
    let (mut page, mut x, mut y, mut shelf) = (0usize, 0usize, 0usize, 0usize);
    let mut used = false;
    for k in order {
        let (w, h) = (charts[k].w + 2 * PAD, charts[k].h + 2 * PAD);
        if x + w > PAGE {
            y += shelf;
            x = 0;
            shelf = 0;
        }
        if y + h > PAGE {
            page += 1;
            x = 0;
            y = 0;
            shelf = 0;
        }
        charts[k].page = page;
        charts[k].x = x;
        charts[k].y = y;
        used = true;
        x += w;
        shelf = shelf.max(h);
    }
    if used { page + 1 } else { 0 }
}

fn recess_shade(d: f64) -> f32 {
    if d < -0.04 { 1.0 - 0.5 * ((-d - 0.04) / 0.5).clamp(0.0, 1.0) as f32 } else { 1.0 }
}

/// Capture one chart of `far` into `page` (its interior at `(px, py)`).
/// Returns the sum of its linear colours and of its glass coverage.
/// also the sum of its opaque (not glass) colour and that coverage; with
/// `glass_body` a glass texel holds `glass_body` of the pane's colour.
fn capture_chart(page: &mut [u8], c: &Chart, far: &Far, frame: ([f64; 2], [f64; 2]), tris: &[usize], glass_body_on: bool) -> ([f64; 3], f64, [f64; 3], f64) {
    let (u, n) = frame;
    let a = far.ring[c.edge];
    let (sw, sh) = (c.w * SS, c.h * SS);
    let mut zb = vec![f64::NEG_INFINITY; sw * sh];
    let mut col = vec![[0f32; 4]; sw * sh];
    let du = (c.u1 - c.u0) / sw as f64;
    let dv = (c.v1 - c.v0) / sh as f64;
    for &ti in tris {
        let t = &far.src[ti];
        let s = t.p.map(|p| {
            let (rx, rz) = (p[0] as f64 - a[0], p[2] as f64 - a[1]);
            let uu = rx * u[0] + rz * u[1];
            let dd = rx * n[0] + rz * n[1];
            ((uu - c.u0) / du, (c.v1 - p[1] as f64) / dv, dd)
        });
        let (minx, maxx) = (s[0].0.min(s[1].0).min(s[2].0), s[0].0.max(s[1].0).max(s[2].0));
        let (miny, maxy) = (s[0].1.min(s[1].1).min(s[2].1), s[0].1.max(s[1].1).max(s[2].1));
        if maxx < 0.0 || maxy < 0.0 || minx > sw as f64 || miny > sh as f64 {
            continue;
        }
        let area = (s[1].0 - s[0].0) * (s[2].1 - s[0].1) - (s[2].0 - s[0].0) * (s[1].1 - s[0].1);
        if area.abs() < 1e-12 {
            continue;
        }
        let x0 = (minx - 0.5).floor().max(0.0) as usize;
        let x1 = ((maxx - 0.5).ceil().max(0.0) as usize).min(sw - 1);
        let y0 = (miny - 0.5).floor().max(0.0) as usize;
        let y1 = ((maxy - 0.5).ceil().max(0.0) as usize).min(sh - 1);
        let edge = |p: (f64, f64, f64), q: (f64, f64, f64), x: f64, y: f64| (q.0 - p.0) * (y - p.1) - (x - p.0) * (q.1 - p.1);
        for yy in y0..=y1 {
            let py = yy as f64 + 0.5;
            for xx in x0..=x1 {
                let px = xx as f64 + 0.5;
                let w0 = edge(s[1], s[2], px, py) / area;
                let w1 = edge(s[2], s[0], px, py) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < -1e-7 || w1 < -1e-7 || w2 < -1e-7 {
                    continue;
                }
                let d = w0 * s[0].2 + w1 * s[1].2 + w2 * s[2].2;
                if !(-DEPTH_IN_M..=DEPTH_OUT_M).contains(&d) {
                    continue;
                }
                let k = yy * sw + xx;
                if d > zb[k] {
                    zb[k] = d;
                    let shade = recess_shade(d);
                    let rgb = if t.glass && glass_body_on { glass_body(t.rgb) } else { t.rgb };
                    col[k] = [rgb[0] * shade, rgb[1] * shade, rgb[2] * shade, if t.glass { 1.0 } else { 0.0 }];
                }
            }
        }
    }
    // Resolve the subsamples into texels, then the border.
    let (px0, py0) = (c.x + PAD, c.y + PAD);
    let mut sum = [0f64; 3];
    let (mut opaque, mut opaque_w) = ([0f64; 3], 0f64);
    let mut glass = 0f64;
    let inv = 1.0 / (SS * SS) as f32;
    for ty in 0..c.h {
        for tx in 0..c.w {
            let mut acc = [0f32; 4];
            for sy in 0..SS {
                for sx in 0..SS {
                    let k = (ty * SS + sy) * sw + tx * SS + sx;
                    let v = if zb[k] == f64::NEG_INFINITY { [far.wall_rgb[0], far.wall_rgb[1], far.wall_rgb[2], 0.0] } else { col[k] };
                    if v[3] < 0.5 {
                        for q in 0..3 {
                            opaque[q] += (v[q] * inv) as f64;
                        }
                        opaque_w += inv as f64;
                    }
                    for q in 0..4 {
                        acc[q] += v[q] * inv;
                    }
                }
            }
            for q in 0..3 {
                sum[q] += acc[q] as f64;
            }
            glass += acc[3] as f64;
            let o = ((py0 + ty) * PAGE + px0 + tx) * 4;
            page[o] = linear_to_srgb_byte(acc[0]);
            page[o + 1] = linear_to_srgb_byte(acc[1]);
            page[o + 2] = linear_to_srgb_byte(acc[2]);
            page[o + 3] = (acc[3].clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
    // Edge-replicated border.
    for yy in c.y..c.y + c.h + 2 * PAD {
        for xx in c.x..c.x + c.w + 2 * PAD {
            let (ix, iy) = (xx.clamp(px0, px0 + c.w - 1), yy.clamp(py0, py0 + c.h - 1));
            if ix == xx && iy == yy {
                continue;
            }
            let (s, d) = ((iy * PAGE + ix) * 4, (yy * PAGE + xx) * 4);
            let px: [u8; 4] = page[s..s + 4].try_into().unwrap();
            page[d..d + 4].copy_from_slice(&px);
        }
    }
    (sum, glass, opaque, opaque_w)
}

/// the parapet band's width (metres) and its shade (a multiply on the roof colour).
const PARAPET_BAND: (f64, f64) = (0.45, 0.9);
const PARAPET_SHADE: f32 = 0.6;
/// the band lies this far above the cap (no depth fight at a kilometre).
const PARAPET_LIFT_M: f64 = 0.06;

fn signed_area(r: &[[f64; 2]]) -> f64 {
    let mut a = 0.0;
    for i in 0..r.len() {
        let (p, q) = (r[i], r[(i + 1) % r.len()]);
        a += p[0] * q[1] - q[0] * p[1];
    }
    a * 0.5
}

/// the part of a ring on one side of the line `dot(p, dir) = t` (Sutherland-Hodgman).
fn clip_ring(r: &[[f64; 2]], dir: [f64; 2], t: f64, keep_above: bool) -> Vec<[f64; 2]> {
    let side = |p: [f64; 2]| {
        let s = p[0] * dir[0] + p[1] * dir[1] - t;
        if keep_above { s } else { -s }
    };
    let mut out = vec![];
    for i in 0..r.len() {
        let (p, q) = (r[i], r[(i + 1) % r.len()]);
        let (sp, sq) = (side(p), side(q));
        if sp >= 0.0 {
            out.push(p);
        }
        if (sp >= 0.0) != (sq >= 0.0) {
            let k = sp / (sp - sq);
            out.push([p[0] + (q[0] - p[0]) * k, p[1] + (q[1] - p[1]) * k]);
        }
    }
    out
}

/// `dir` turned to the nearest normal of one of the ring's edges (2 m or
/// longer), so the split runs along the building, as a roof's sections do.
fn square_to_ring(r: &[[f64; 2]], dir: [f64; 2]) -> [f64; 2] {
    let mut best = (f64::MIN, dir);
    for i in 0..r.len() {
        let (p, q) = (r[i], r[(i + 1) % r.len()]);
        let (ex, ez) = (q[0] - p[0], q[1] - p[1]);
        let len = ex.hypot(ez);
        if len < 2.0 {
            continue;
        }
        for n in [[-ez / len, ex / len], [ez / len, -ex / len]] {
            let d = n[0] * dir[0] + n[1] * dir[1];
            if d > best.0 {
                best = (d, n);
            }
        }
    }
    best.1
}

/// the cap ring split so the part toward `dir` holds `share` of its area.
fn split_ring(r: &[[f64; 2]], dir: [f64; 2], share: f64) -> Option<(Vec<[f64; 2]>, Vec<[f64; 2]>)> {
    let total = signed_area(r).abs();
    if total < 1.0 {
        return None;
    }
    let s: Vec<f64> = r.iter().map(|p| p[0] * dir[0] + p[1] * dir[1]).collect();
    let (mut lo, mut hi) = (s.iter().cloned().fold(f64::MAX, f64::min), s.iter().cloned().fold(f64::MIN, f64::max));
    for _ in 0..24 {
        let mid = 0.5 * (lo + hi);
        if signed_area(&clip_ring(r, dir, mid, true)).abs() > share * total {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let t = 0.5 * (lo + hi);
    let (main, patch) = (clip_ring(r, dir, t, false), clip_ring(r, dir, t, true));
    (main.len() >= 3 && patch.len() >= 3).then_some((main, patch))
}

/// The roof cap at the wall top, one colour; with the measured second
/// colour's patch on the side it lies (`patches`) and a darker band inside
/// each edge for the parapet and its shade line (`parapet`, flat roofs only).
/// Returns the cap's mean colour.
fn roof_cap(f: &Far, vertices: &mut Vec<HlodVertex>, indices: &mut Vec<u32>, ear: &mut earcut::Earcut<f64>, tri: &mut Vec<u32>) -> [f32; 3] {
    let up = pack_normal([0.0, 1.0, 0.0], 0);
    let mut fill = |poly: &[[f64; 2]], rgb: [f32; 3], vertices: &mut Vec<HlodVertex>, indices: &mut Vec<u32>| {
        tri.clear();
        ear.earcut(poly.iter().copied(), &[] as &[u32], tri);
        let base = vertices.len() as u32;
        let word = srgb_word(rgb, VF_ROOF);
        for p in poly {
            vertices.push(HlodVertex { pos: [p[0] as f32, f.top as f32, p[1] as f32], normal: up, uv: 0, color: word, group: f.group });
        }
        indices.extend(tri.iter().map(|t| base + t));
    };
    let mut mean = f.roof_rgb;
    let split = f.roofs.patch.and_then(|(rgb, dir, share)| split_ring(&f.ring, square_to_ring(&f.ring, dir), share as f64).map(|(a, b)| (a, b, rgb, share)));
    match split {
        Some((main, patch, rgb, share)) => {
            fill(&main, f.roof_rgb, vertices, indices);
            fill(&patch, rgb, vertices, indices);
            mean = [0, 1, 2].map(|k| f.roof_rgb[k] * (1.0 - share) + rgb[k] * share);
        }
        None => fill(&f.ring, f.roof_rgb, vertices, indices),
    }
    let area = signed_area(&f.ring);
    if eo_data::roof_colours::feature_on("parapet") && !f.roofs.pitched && area.abs() >= 30.0 {
        let w = (0.035 * area.abs().sqrt()).clamp(PARAPET_BAND.0, PARAPET_BAND.1);
        let sign = area.signum();
        let word = srgb_word(mul3(f.roof_rgb, [PARAPET_SHADE; 3]), VF_ROOF);
        let y = (f.top + PARAPET_LIFT_M) as f32;
        let n = f.ring.len();
        for e in 0..n {
            let (a, b) = (f.ring[e], f.ring[(e + 1) % n]);
            let (ex, ez) = (b[0] - a[0], b[1] - a[1]);
            let len = ex.hypot(ez);
            if len < 1.0 {
                continue;
            }
            // A party wall's band is narrower (the low dividing parapet between row houses).
            let we = if f.party.get(e).copied().unwrap_or(false) { w * 0.6 } else { w };
            let inward = [-ez / len * sign * we, ex / len * sign * we];
            let s = vertices.len() as u32;
            for p in [a, b, [b[0] + inward[0], b[1] + inward[1]], [a[0] + inward[0], a[1] + inward[1]]] {
                vertices.push(HlodVertex { pos: [p[0] as f32, y, p[1] as f32], normal: up, uv: 0, color: word, group: f.group });
            }
            indices.extend_from_slice(&[s, s + 1, s + 2, s, s + 2, s + 3]);
        }
    }
    mean
}

fn pack_normal(n: [f32; 3], page: u32) -> u32 {
    eo_assets::glb::pack_normal(glam::Vec3::from(n)) | ((page & 255) << 24)
}

fn srgb_word(c: [f32; 3], flags: u32) -> u32 {
    linear_to_srgb_byte(c[0]) as u32 | ((linear_to_srgb_byte(c[1]) as u32) << 8) | ((linear_to_srgb_byte(c[2]) as u32) << 16) | ((flags & 255) << 24)
}

fn uv_word(u: f64, v: f64) -> u32 {
    let q = |x: f64| ((x / PAGE as f64).clamp(0.0, 1.0) * 65535.0).round() as u32;
    q(u) | (q(v) << 16)
}

/// Bake one cell (no cache).
pub fn bake_cell(ctx: &BakeContext, cell: u32) -> BakedCell {
    let t0 = Instant::now();
    let c = &ctx.set.cells[cell as usize];
    let mut stats = CellStats::default();
    let mut fars = Vec::with_capacity(c.buildings.len());
    for cb in &c.buildings {
        let far = match cb.source {
            Source::Kit => {
                stats.kit += 1;
                kit_far(ctx, cb.building, cb.group)
            }
            Source::Blender(k) => {
                stats.blender += 1;
                blender_far(ctx, cb.building, cb.group, k)
            }
            Source::Plain => {
                stats.plain += 1;
                Some(plain_far(ctx, cb.building, cb.group))
            }
        };
        let far = far.unwrap_or_else(|| {
            stats.fallback += 1;
            plain_far(ctx, cb.building, cb.group)
        });
        if far.ring.len() >= 3 {
            stats.source_triangles += far.src.len() as u64;
            fars.push(far);
        }
    }
    // Texel size: the nominal one, coarser until the cell fits its pages.
    let mut texel = ctx.opts.texel_m as f64;
    let (mut charts, mut pages);
    loop {
        charts = make_charts(&fars, texel, ctx.opts.party_scale as f64);
        pages = pack(&mut charts);
        if pages <= ctx.opts.max_pages.max(1) || texel > 16.0 {
            break;
        }
        texel *= 1.15;
    }
    let mut page_data: Vec<Vec<u8>> = (0..pages).map(|_| vec![0u8; PAGE * PAGE * 4]).collect();
    // Per building and edge: the source triangles facing it inside the slab.
    let mut acc: Vec<([f64; 3], f64, u64)> = vec![([0.0; 3], 0.0, 0); fars.len()];
    let mut opaque_acc: Vec<([f64; 3], f64)> = vec![([0.0; 3], 0.0); fars.len()];
    let mut cache: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    let frames: Vec<Vec<([f64; 2], [f64; 2], f64)>> = fars.iter().map(|f| edge_frames(&f.ring)).collect();
    for ch in &charts {
        let f = &fars[ch.far];
        let (u, n, len) = frames[ch.far][ch.edge];
        let tris = cache.entry((ch.far, ch.edge)).or_insert_with(|| {
            let a = f.ring[ch.edge];
            let n3 = [n[0] as f32, 0.0, n[1] as f32];
            f.src
                .iter()
                .enumerate()
                .filter(|(_, t)| {
                    let dot = t.n[0] * n3[0] + t.n[2] * n3[2];
                    let facing = if t.two_sided { dot.abs() >= 0.3 } else { dot >= 0.3 };
                    if !facing {
                        return false;
                    }
                    let mut dmin = f64::MAX;
                    let mut dmax = f64::MIN;
                    let mut umin = f64::MAX;
                    let mut umax = f64::MIN;
                    for p in &t.p {
                        let (rx, rz) = (p[0] as f64 - a[0], p[2] as f64 - a[1]);
                        let d = rx * n[0] + rz * n[1];
                        let uu = rx * u[0] + rz * u[1];
                        dmin = dmin.min(d);
                        dmax = dmax.max(d);
                        umin = umin.min(uu);
                        umax = umax.max(uu);
                    }
                    dmax >= -DEPTH_IN_M && dmin <= DEPTH_OUT_M && umax >= 0.0 && umin <= len
                })
                .map(|(k, _)| k)
                .collect()
        });
        let (sum, glass, opaque, opaque_w) = capture_chart(&mut page_data[ch.page], ch, f, (u, n), tris, ctx.opts.glass_body);
        for q in 0..3 {
            opaque_acc[ch.far].0[q] += opaque[q];
        }
        opaque_acc[ch.far].1 += opaque_w;
        let a = &mut acc[ch.far];
        for q in 0..3 {
            a.0[q] += sum[q];
        }
        a.1 += glass;
        a.2 += (ch.w * ch.h) as u64;
        stats.texels += (ch.w * ch.h) as u64;
    }
    stats.charts = charts.len() as u32;
    // Geometry.
    let mut vertices: Vec<HlodVertex> = vec![];
    let mut indices: Vec<u32> = vec![];
    let quad = |vs: &mut Vec<HlodVertex>, is: &mut Vec<u32>, q: [HlodVertex; 4]| {
        let s = vs.len() as u32;
        vs.extend_from_slice(&q);
        is.extend_from_slice(&[s, s + 1, s + 2, s, s + 2, s + 3]);
    };
    let mut by_far: Vec<Vec<usize>> = vec![vec![]; fars.len()];
    for (k, ch) in charts.iter().enumerate() {
        by_far[ch.far].push(k);
    }
    let mut summaries = Vec::with_capacity(fars.len());
    let mut ear = earcut::Earcut::<f64>::new();
    let mut tri: Vec<u32> = vec![];
    for (fi, f) in fars.iter().enumerate() {
        let frames = &frames[fi];
        let tower_flag = if f.tower { VF_TOWER } else { 0 };
        let wall_word = srgb_word(f.wall_rgb, if f.textured { VF_TEXTURED } else { 0 } | tower_flag);
        let at = |e: usize, s: f64, y: f64| -> [f32; 3] {
            let a = f.ring[e];
            let u = frames[e].0;
            [(a[0] + u[0] * s) as f32, y as f32, (a[1] + u[1] * s) as f32]
        };
        if f.textured {
            for &k in &by_far[fi] {
                let ch = &charts[k];
                let (_, n, _) = frames[ch.edge];
                let nrm = pack_normal([n[0] as f32, 0.0, n[1] as f32], ch.page as u32);
                let (x0, y0) = ((ch.x + PAD) as f64, (ch.y + PAD) as f64);
                let (x1, y1) = (x0 + ch.w as f64, y0 + ch.h as f64);
                let v = |p: [f32; 3], uv: u32| HlodVertex { pos: p, normal: nrm, uv, color: wall_word, group: f.group };
                quad(
                    &mut vertices,
                    &mut indices,
                    [
                        v(at(ch.edge, ch.u0, ch.v1), uv_word(x0, y0)),
                        v(at(ch.edge, ch.u1, ch.v1), uv_word(x1, y0)),
                        v(at(ch.edge, ch.u1, ch.v0), uv_word(x1, y1)),
                        v(at(ch.edge, ch.u0, ch.v0), uv_word(x0, y1)),
                    ],
                );
            }
        } else {
            for (e, &(_, n, len)) in frames.iter().enumerate() {
                if len < 0.05 {
                    continue;
                }
                let nrm = pack_normal([n[0] as f32, 0.0, n[1] as f32], 0);
                let v = |p: [f32; 3]| HlodVertex { pos: p, normal: nrm, uv: 0, color: wall_word, group: f.group };
                quad(&mut vertices, &mut indices, [v(at(e, 0.0, f.top)), v(at(e, len, f.top)), v(at(e, len, f.bottom)), v(at(e, 0.0, f.bottom))]);
            }
        }
        // The roof cap at the wall top (split by the measured second colour, edged by the parapet band).
        let cap_rgb = roof_cap(f, &mut vertices, &mut indices, &mut ear, &mut tri);
        // Roof forms.
        for (p, n, rgb) in &f.roof {
            let s = vertices.len() as u32;
            let w = srgb_word(*rgb, VF_ROOF);
            let nrm = pack_normal(*n, 0);
            for q in p {
                vertices.push(HlodVertex { pos: *q, normal: nrm, uv: 0, color: w, group: f.group });
            }
            indices.extend_from_slice(&[s, s + 1, s + 2]);
        }
        let a = &acc[fi];
        let texels = a.2.max(1) as f64;
        summaries.push(BuildingSummary {
            group: f.group,
            building: f.building,
            facade: if a.2 > 0 { [(a.0[0] / texels) as f32, (a.0[1] / texels) as f32, (a.0[2] / texels) as f32] } else { f.wall_rgb },
            glass: (a.1 / texels) as f32,
            roof: cap_rgb, // the cap's mean (the second colour's share in it)
            base: f.bottom as f32,
            top: f.top as f32,
            wall: if opaque_acc[fi].1 > 0.0 { opaque_acc[fi].0.map(|v| (v / opaque_acc[fi].1) as f32) } else { f.wall_rgb },
            rhythm: f.rhythm,
            tower: if f.tower { 1.0 } else { 0.0 },
        });
    }
    let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
    for v in &vertices {
        for k in 0..3 {
            mn[k] = mn[k].min(v.pos[k]);
            mx[k] = mx[k].max(v.pos[k]);
        }
    }
    if vertices.is_empty() {
        mn = [0.0; 3];
        mx = [0.0; 3];
    }
    stats.bake_ms = (t0.elapsed().as_secs_f64() * 1e3) as f32;
    // The pages as stored and uploaded: their mips, compressed where the GPU reads BC3.
    let pages = page_data.iter().map(|p| page_levels(p, ctx.opts.bc3)).collect();
    stats.bake_ms = (t0.elapsed().as_secs_f64() * 1e3) as f32;
    BakedCell { key: c.key, vertices, indices, pages, bc3: ctx.opts.bc3, texel_m: texel as f32, min: mn, max: mx, summaries, stats }
}

/// BC3 (DXT5) blocks of an RGBA8 image (`w`, `h` multiples of 4), row of
/// blocks after row: per 4 x 4 block the alpha as 8 levels between its min
/// and max (3-bit indices), the colour as 4 levels on the bounding box
/// diagonal of its texels (565 endpoints, 2-bit indices). The colour is
/// encoded in the bytes' own (sRGB) space, as the sRGB format decodes it.
pub fn bc3_encode(rgba: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(w * h);
    let q565 = |c: [f32; 3]| -> u16 {
        let r = (c[0] * 31.0 / 255.0).round().clamp(0.0, 31.0) as u16;
        let g = (c[1] * 63.0 / 255.0).round().clamp(0.0, 63.0) as u16;
        let b = (c[2] * 31.0 / 255.0).round().clamp(0.0, 31.0) as u16;
        (r << 11) | (g << 5) | b
    };
    let un565 = |v: u16| -> [f32; 3] { [((v >> 11) & 31) as f32 * 255.0 / 31.0, ((v >> 5) & 63) as f32 * 255.0 / 63.0, (v & 31) as f32 * 255.0 / 31.0] };
    for by in (0..h).step_by(4) {
        for bx in (0..w).step_by(4) {
            let mut px = [[0f32; 4]; 16];
            for k in 0..16 {
                let (x, y) = ((bx + k % 4).min(w - 1), (by + k / 4).min(h - 1));
                let o = (y * w + x) * 4;
                px[k] = [rgba[o] as f32, rgba[o + 1] as f32, rgba[o + 2] as f32, rgba[o + 3] as f32];
            }
            // Alpha: a0 = max > a1 = min, eight levels.
            let amax = px.iter().map(|p| p[3]).fold(0f32, f32::max) as u8;
            let amin = px.iter().map(|p| p[3]).fold(255f32, f32::min) as u8;
            let mut abits: u64 = 0;
            if amax > amin {
                let levels: [f32; 8] = {
                    let (a0, a1) = (amax as f32, amin as f32);
                    [a0, a1, (6.0 * a0 + a1) / 7.0, (5.0 * a0 + 2.0 * a1) / 7.0, (4.0 * a0 + 3.0 * a1) / 7.0, (3.0 * a0 + 4.0 * a1) / 7.0, (2.0 * a0 + 5.0 * a1) / 7.0, (a0 + 6.0 * a1) / 7.0]
                };
                for (k, p) in px.iter().enumerate() {
                    let best = (0..8u64).min_by(|&i, &j| (levels[i as usize] - p[3]).abs().total_cmp(&(levels[j as usize] - p[3]).abs())).unwrap();
                    abits |= best << (3 * k);
                }
            }
            out.push(amax);
            out.push(amin);
            out.extend_from_slice(&abits.to_le_bytes()[..6]);
            // Colour: the bounding box diagonal.
            let mut lo = [255f32; 3];
            let mut hi = [0f32; 3];
            for p in &px {
                for c in 0..3 {
                    lo[c] = lo[c].min(p[c]);
                    hi[c] = hi[c].max(p[c]);
                }
            }
            let (c0, c1) = (q565(hi), q565(lo));
            let (e0, e1) = (un565(c0), un565(c1));
            let mut cbits: u32 = 0;
            let axis = [e0[0] - e1[0], e0[1] - e1[1], e0[2] - e1[2]];
            let len2 = axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2];
            if c0 != c1 && len2 > 0.0 {
                for (k, p) in px.iter().enumerate() {
                    let t = ((p[0] - e1[0]) * axis[0] + (p[1] - e1[1]) * axis[1] + (p[2] - e1[2]) * axis[2]) / len2;
                    // Levels: 0 = c0, 2 = 2/3 c0 + 1/3 c1, 3 = 1/3 c0 + 2/3 c1, 1 = c1.
                    let idx = if t >= 5.0 / 6.0 { 0 } else if t >= 0.5 { 2 } else if t >= 1.0 / 6.0 { 3 } else { 1 };
                    cbits |= idx << (2 * k);
                }
            }
            out.extend_from_slice(&c0.to_le_bytes());
            out.extend_from_slice(&c1.to_le_bytes());
            out.extend_from_slice(&cbits.to_le_bytes());
        }
    }
    out
}

/// Decode one BC3 texel (tests).
pub fn bc3_texel(blocks: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
    let b = &blocks[((y / 4) * (w / 4) + x / 4) * 16..][..16];
    let k = (y % 4) * 4 + x % 4;
    let (a0, a1) = (b[0] as f32, b[1] as f32);
    let mut ab = [0u8; 8];
    ab[..6].copy_from_slice(&b[2..8]);
    let ai = (u64::from_le_bytes(ab) >> (3 * k)) & 7;
    let a = if a0 > a1 {
        match ai {
            0 => a0,
            1 => a1,
            i => ((8 - i) as f32 * a0 + (i - 1) as f32 * a1) / 7.0,
        }
    } else {
        match ai {
            0 => a0,
            1 => a1,
            6 => 0.0,
            7 => 255.0,
            i => ((6 - i) as f32 * a0 + (i - 1) as f32 * a1) / 5.0,
        }
    };
    let c0 = u16::from_le_bytes([b[8], b[9]]);
    let c1 = u16::from_le_bytes([b[10], b[11]]);
    let un = |v: u16| [((v >> 11) & 31) as f32 * 255.0 / 31.0, ((v >> 5) & 63) as f32 * 255.0 / 63.0, (v & 31) as f32 * 255.0 / 31.0];
    let (e0, e1) = (un(c0), un(c1));
    let ci = (u32::from_le_bytes([b[12], b[13], b[14], b[15]]) >> (2 * k)) & 3;
    let w0 = match ci {
        0 => 1.0,
        1 => 0.0,
        2 => 2.0 / 3.0,
        _ => 1.0 / 3.0,
    };
    let c = |i: usize| (e0[i] * w0 + e1[i] * (1.0 - w0)).round() as u8;
    [c(0), c(1), c(2), a.round() as u8]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellSource {
    Cache,
    Baked,
}

/// The cell's summaries beside it (the district level reads only these).
pub fn summary_path(dir: &Path, key: &str, cell: (i32, i32)) -> PathBuf {
    dir.join("hlod").join(key).join(format!("{}_{}.eohs", cell.0, cell.1))
}

const SUMMARY_MAGIC: &[u8; 4] = b"EOHS";

pub fn encode_summaries(cell: (i32, i32), s: &[BuildingSummary]) -> Vec<u8> {
    let mut b = vec![];
    b.extend_from_slice(SUMMARY_MAGIC);
    for v in [BAKE_VERSION, cell.0 as u32, cell.1 as u32, s.len() as u32] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    for m in s {
        b.extend_from_slice(&m.group.to_le_bytes());
        b.extend_from_slice(&m.building.to_le_bytes());
        for v in m.floats() {
            b.extend_from_slice(&v.to_le_bytes());
        }
    }
    let d = Sha256::digest(&b);
    b.extend_from_slice(&d);
    b
}

pub fn decode_summaries(bytes: &[u8], cell: (i32, i32)) -> Result<Vec<BuildingSummary>, String> {
    if bytes.len() < 20 + 32 || &bytes[0..4] != SUMMARY_MAGIC {
        return Err("not a summary file".into());
    }
    let (body, digest) = bytes.split_at(bytes.len() - 32);
    if Sha256::digest(body).as_slice() != digest {
        return Err("payload digest mismatch".into());
    }
    let w = |at: usize| u32::from_le_bytes(body[at..at + 4].try_into().unwrap());
    if w(4) != BAKE_VERSION || (w(8) as i32, w(12) as i32) != cell {
        return Err("another version or cell".into());
    }
    let n = w(16) as usize;
    let rec = 8 + 4 * BuildingSummary::FLOATS;
    if body.len() != 20 + n * rec {
        return Err("truncated".into());
    }
    Ok((0..n)
        .map(|k| {
            let at = 20 + k * rec;
            let v: Vec<f32> = (0..BuildingSummary::FLOATS).map(|j| f32::from_bits(w(at + 8 + 4 * j))).collect();
            BuildingSummary::from_floats(w(at), w(at + 4), &v)
        })
        .collect())
}

fn write_atomic(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    if std::fs::write(&tmp, bytes).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// The summaries of a baked cell, from its summary file (None: not baked yet, or damaged).
pub fn read_summaries(ctx: &BakeContext, cell: u32) -> Option<Vec<BuildingSummary>> {
    let key = ctx.set.cells[cell as usize].key;
    let dir = ctx.cache_dir.as_ref()?;
    decode_summaries(&std::fs::read(summary_path(dir, &ctx.key, key)).ok()?, key).ok()
}

/// one building's colours for the far-colour diagnosis
/// (`--bench --far-shots --far-diag`): its archetype, the palette's wall and
/// roof colour (material mean x tint), the area-weighted mean of what rung 2
/// draws facing out (walls only, glass apart) and its glass share and colour.
#[derive(Clone, Debug, Default)]
pub struct DiagBuilding {
    pub group: u32,
    pub archetype: String,
    pub storeys: f32,
    pub height: f32,
    pub wall_palette: [f32; 3],
    pub roof_palette: [f32; 3],
    /// Rung 2's vertical faces, opaque materials only, area weighted.
    pub rung_wall: [f32; 3],
    /// Rung 2's glass area over all vertical area, and the glass's mean base colour.
    pub rung_glass_share: f32,
    pub rung_glass: [f32; 3],
    pub rhythm: [f32; 5],
    pub tower: bool,
}

/// the diagnosis rows of a cell's kit buildings (no capture, no cache).
pub fn diag_cell(ctx: &BakeContext, cell: u32) -> Vec<DiagBuilding> {
    let c = &ctx.set.cells[cell as usize];
    let mut out = vec![];
    for cb in &c.buildings {
        if cb.source != Source::Kit {
            continue;
        }
        let Some(far) = kit_far(ctx, cb.building, cb.group) else { continue };
        let rec = &ctx.kit_city.records[cb.building as usize];
        let base_y = ctx.kit.base_y[cb.building as usize] as f64;
        let (archetype, storeys) = match eo_kit::pipeline::plan_building(rec, &ctx.kit_city.world, cb.building, base_y) {
            Some(p) => (p.classified.archetype.to_string(), p.plan.storeys.len() as f32),
            None => (String::from("?"), 0.0),
        };
        let (mut w, mut wa, mut g, mut ga) = ([0f64; 3], 0f64, [0f64; 3], 0f64);
        for t in &far.src {
            if t.n[1].abs() > 0.3 || t.two_sided {
                continue;
            }
            let a = glam::Vec3::from(t.p[0]);
            let area = 0.5 * (glam::Vec3::from(t.p[1]) - a).cross(glam::Vec3::from(t.p[2]) - a).length() as f64;
            let (acc, n) = if t.glass { (&mut g, &mut ga) } else { (&mut w, &mut wa) };
            for k in 0..3 {
                acc[k] += t.rgb[k] as f64 * area;
            }
            *n += area;
        }
        let avg = |a: [f64; 3], n: f64| if n > 0.0 { [(a[0] / n) as f32, (a[1] / n) as f32, (a[2] / n) as f32] } else { [0.0; 3] };
        out.push(DiagBuilding {
            group: cb.group,
            archetype,
            storeys,
            height: (far.top - far.bottom) as f32,
            wall_palette: far.wall_rgb,
            roof_palette: far.roof_rgb,
            rung_wall: avg(w, wa),
            rung_glass_share: if wa + ga > 0.0 { (ga / (wa + ga)) as f32 } else { 0.0 },
            rung_glass: avg(g, ga),
            rhythm: far.rhythm,
            tower: far.tower,
        });
    }
    out
}

/// Read a cell from the cache, or bake and cache it. A file that does not
/// decode, or names another cell, is baked again (and replaced).
pub fn load_or_bake(ctx: &BakeContext, cell: u32) -> (BakedCell, CellSource) {
    let key = ctx.set.cells[cell as usize].key;
    if let Some(dir) = &ctx.cache_dir {
        if let Ok(bytes) = std::fs::read(cache_path(dir, &ctx.key, key)) {
            if let Ok(c) = BakedCell::decode(&bytes) {
                if c.key == key {
                    let sp = summary_path(dir, &ctx.key, key);
                    if !sp.exists() {
                        write_atomic(&sp, &encode_summaries(key, &c.summaries));
                    }
                    return (c, CellSource::Cache);
                }
            }
        }
    }
    let baked = bake_cell(ctx, cell);
    if let Some(dir) = &ctx.cache_dir {
        write_atomic(&cache_path(dir, &ctx.key, key), &baked.encode());
        write_atomic(&summary_path(dir, &ctx.key, key), &encode_summaries(key, &baked.summaries));
    }
    (baked, CellSource::Baked)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_cell() -> BakedCell {
        let v = |x: f32, g: u32| HlodVertex { pos: [x, 1.0, 2.0], normal: 7, uv: 9, color: 0x01ff8040, group: g };
        BakedCell {
            key: (-3, 7),
            vertices: vec![v(0.0, 5), v(1.0, 5), v(2.0, 6)],
            indices: vec![0, 1, 2],
            pages: vec![(0..(0..PAGE_MIPS).map(|m| page_mip_bytes(m, true)).sum::<usize>()).map(|i| (i % 251) as u8).collect()],
            bc3: true,
            texel_m: 0.5,
            min: [0.0, 1.0, 2.0],
            max: [2.0, 1.0, 2.0],
            summaries: vec![BuildingSummary { group: 5, building: 11, facade: [0.1, 0.2, 0.3], glass: 0.25, roof: [0.4, 0.4, 0.4], base: 3.0, top: 12.0, wall: [0.2, 0.25, 0.3], rhythm: [3.1, 3.2, 0.4, 0.5, 0.25], tower: 1.0 }],
            stats: CellStats { kit: 1, blender: 0, plain: 1, fallback: 0, charts: 4, texels: 1234, source_triangles: 99, bake_ms: 1.5 },
        }
    }

    #[test]
    fn bc3_keeps_a_facade_pattern_close() {
        // A 16 x 8 image: wall, a dark glass pane (alpha 255), a lighter trim row.
        let (w, h) = (16usize, 8usize);
        let mut img = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let o = (y * w + x) * 4;
                let px: [u8; 4] = if (4..8).contains(&x) && (2..6).contains(&y) { [40, 46, 52, 255] } else if y == 7 { [220, 214, 200, 0] } else { [182, 150, 120, 0] };
                img[o..o + 4].copy_from_slice(&px);
            }
        }
        let b = bc3_encode(&img, w, h);
        assert_eq!(b.len(), (w / 4) * (h / 4) * 16);
        let mut sum = 0i64;
        for y in 0..h {
            for x in 0..w {
                let o = (y * w + x) * 4;
                let d = bc3_texel(&b, w, x, y);
                // Glass coverage is exact (two levels in a block).
                assert_eq!(d[3], img[o + 3], "texel ({x}, {y}) alpha");
                for c in 0..3 {
                    let e = (d[c] as i32 - img[o + c] as i32).abs();
                    sum += e as i64;
                    // A block of three colours not on one line is the format's own limit.
                    assert!(e <= 40, "texel ({x}, {y}) channel {c}: {} vs {}", d[c], img[o + c]);
                }
            }
        }
        assert!(sum as f64 / (w * h * 3) as f64 <= 8.0, "mean error {}", sum as f64 / (w * h * 3) as f64);
    }

    // the cap split by the second measured colour and the parapet band.
    #[test]
    fn a_roof_splits_by_its_share_toward_its_second_colour() {
        let ring = vec![[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.0, 10.0]];
        let (main, patch) = split_ring(&ring, [1.0, 0.0], 0.3).unwrap();
        assert!((signed_area(&patch).abs() - 60.0).abs() < 0.01);
        assert!((signed_area(&main).abs() - 140.0).abs() < 0.01);
        assert!(patch.iter().all(|p| p[0] >= 14.0 - 1e-6), "the patch lies toward +x");
        // A direction off the building's axes is turned onto the nearest edge normal.
        let d = square_to_ring(&ring, [0.8, 0.6]);
        assert!((d[0] - 1.0).abs() < 1e-9 && d[1].abs() < 1e-9, "{d:?}");
        // An L shape, toward -z (north).
        let l = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 4.0], [4.0, 4.0], [4.0, 10.0], [0.0, 10.0]];
        let (_, patch) = split_ring(&l, [0.0, -1.0], 0.5).unwrap();
        assert!((signed_area(&patch).abs() - 0.5 * signed_area(&l).abs()).abs() < 0.01);
    }

    #[test]
    fn a_flat_roof_cap_gets_its_parapet_band_and_a_pitched_one_does_not() {
        let ring = vec![[0.0, 0.0], [12.0, 0.0], [12.0, 8.0], [0.0, 8.0]];
        let far = |pitched: bool, patch: Option<([f32; 3], [f64; 2], f32)>| Far {
            group: 1,
            building: 0,
            ring: ring.clone(),
            party: vec![false, true, false, false],
            bottom: 0.0,
            top: 9.0,
            src: vec![],
            roof: vec![],
            wall_rgb: [0.6; 3],
            roof_rgb: [0.3; 3],
            textured: false,
            rhythm: [0.0; 5],
            tower: false,
            roofs: RoofCap { patch, pitched },
        };
        let mut ear = earcut::Earcut::new();
        let mut tri = vec![];
        let (mut v, mut i) = (vec![], vec![]);
        let mean = roof_cap(&far(false, None), &mut v, &mut i, &mut ear, &mut tri);
        assert_eq!(mean, [0.3; 3]);
        if eo_data::roof_colours::feature_on("parapet") {
            assert_eq!(v.len(), 4 + 4 * 4, "the cap and one band quad per edge");
            // Every band vertex lies on or inside the footprint, above the cap.
            assert!(v[4..].iter().all(|q| q.pos[0] >= -1e-4 && q.pos[0] <= 12.0001 && q.pos[2] >= -1e-4 && q.pos[2] <= 8.0001 && q.pos[1] > 9.0));
        }
        let (mut v, mut i) = (vec![], vec![]);
        roof_cap(&far(true, None), &mut v, &mut i, &mut ear, &mut tri);
        assert_eq!(v.len(), 4, "a pitched roof's cap is one polygon");
        let (mut v, mut i) = (vec![], vec![]);
        let mean = roof_cap(&far(true, Some(([0.9; 3], [0.0, 1.0], 0.25))), &mut v, &mut i, &mut ear, &mut tri);
        assert!((mean[0] - (0.3 * 0.75 + 0.9 * 0.25)).abs() < 1e-5, "the cap's mean holds the patch's share");
        assert!(v.len() > 4);
    }

    #[test]
    fn cell_round_trips() {
        let c = sample_cell();
        assert_eq!(BakedCell::decode(&c.encode()).unwrap(), c);
    }

    #[test]
    fn damaged_or_truncated_cells_are_refused() {
        let bytes = sample_cell().encode();
        let mut flipped = bytes.clone();
        flipped[100] ^= 0x40;
        assert!(BakedCell::decode(&flipped).is_err(), "a flipped byte must fail the digest");
        assert!(BakedCell::decode(&bytes[..bytes.len() - 1]).is_err(), "a truncated file must be refused");
        assert!(BakedCell::decode(&bytes[..10]).is_err());
        let mut magic = bytes.clone();
        magic[0] = b'X';
        assert!(BakedCell::decode(&magic).is_err());
    }

    #[test]
    fn cache_key_changes_with_its_inputs() {
        let a = AuthoredSet::empty();
        let colours = MaterialColours { rgb: vec![[0.5, 0.4, 0.3]], glass: vec![false] };
        let k0 = inputs_key("kit-a", &colours, &a, &BakeOptions::default());
        assert_eq!(k0, inputs_key("kit-a", &colours, &a, &BakeOptions::default()), "the key is deterministic");
        assert_ne!(k0, inputs_key("kit-b", &colours, &a, &BakeOptions::default()), "the planner's inputs are in the key");
        let other = MaterialColours { rgb: vec![[0.5, 0.4, 0.31]], glass: vec![false] };
        assert_ne!(k0, inputs_key("kit-a", &other, &a, &BakeOptions::default()), "the material colours are in the key");
        assert_ne!(k0, inputs_key("kit-a", &colours, &a, &BakeOptions { texel_m: 0.6, ..Default::default() }), "the options are in the key");
        assert_ne!(k0, inputs_key("kit-a", &colours, &a, &BakeOptions { glass_body: false, ..Default::default() }), "the glass seen through is in the key");
        assert_eq!(k0.len(), 16);
    }

    #[test]
    fn packing_keeps_charts_inside_their_page_and_apart() {
        let mut charts: Vec<Chart> = (0..300)
            .map(|k| Chart { far: 0, edge: 0, u0: 0.0, u1: 1.0, v0: 0.0, v1: 1.0, w: 3 + (k * 37) % 120, h: 2 + (k * 53) % 90, page: 0, x: 0, y: 0 })
            .collect();
        let pages = pack(&mut charts);
        assert!(pages >= 1);
        for (i, a) in charts.iter().enumerate() {
            assert!(a.page < pages);
            assert!(a.x + a.w + 2 * PAD <= PAGE && a.y + a.h + 2 * PAD <= PAGE, "chart {i} leaves its page");
            for b in &charts[i + 1..] {
                if a.page != b.page {
                    continue;
                }
                let apart = a.x + a.w + 2 * PAD <= b.x || b.x + b.w + 2 * PAD <= a.x || a.y + a.h + 2 * PAD <= b.y || b.y + b.h + 2 * PAD <= a.y;
                assert!(apart, "two charts overlap on page {}", a.page);
            }
        }
    }

    #[test]
    fn a_wall_capture_sees_its_own_window() {
        // A 10 m x 6 m wall on the x axis (outward -z, the ring is counter-clockwise
        // in x/z), a glass pane 0.3 m behind it between u 4..6, v 2..4.
        let ring = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 8.0], [0.0, 8.0]];
        let frames = edge_frames(&ring);
        assert!((frames[0].1[1] + 1.0).abs() < 1e-9, "edge 0 faces -z: {:?}", frames[0].1);
        let wall = [0.6f32, 0.5, 0.4];
        let quad = |x0: f32, x1: f32, y0: f32, y1: f32, z: f32, rgb: [f32; 3], glass: bool| {
            [
                SrcTri { p: [[x0, y0, z], [x1, y0, z], [x1, y1, z]], n: [0.0, 0.0, -1.0], rgb, glass, two_sided: false },
                SrcTri { p: [[x0, y0, z], [x1, y1, z], [x0, y1, z]], n: [0.0, 0.0, -1.0], rgb, glass, two_sided: false },
            ]
        };
        let mut src = vec![];
        src.extend(quad(0.0, 10.0, 0.0, 6.0, 0.0, wall, false));
        src.extend(quad(4.0, 6.0, 2.0, 4.0, 0.3, [0.1, 0.1, 0.12], true)); // 0.3 m inside (outward is -z)
        // The far side (faces +z): never on this chart.
        src.extend(quad(0.0, 10.0, 0.0, 6.0, 8.0, [1.0, 0.0, 0.0], false).map(|mut t| {
            t.n = [0.0, 0.0, 1.0];
            t
        }));
        let far = Far { group: 1, building: 0, ring, party: vec![false; 4], bottom: 0.0, top: 6.0, src, roof: vec![], wall_rgb: wall, roof_rgb: wall, textured: true, rhythm: [0.0; 5], tower: false, roofs: RoofCap::default() };
        let ch = Chart { far: 0, edge: 0, u0: 0.0, u1: 10.0, v0: 0.0, v1: 6.0, w: 20, h: 12, page: 0, x: 0, y: 0 };
        let mut page = vec![0u8; PAGE * PAGE * 4];
        let tris: Vec<usize> = (0..far.src.len()).filter(|&k| far.src[k].n[2] < 0.0).collect();
        // The wall is seen from -z: it occludes the pane unless the pane is in
        // front, so cut a hole: only the pane's quad and the wall around it.
        let (_, glass, _, _) = capture_chart(&mut page, &ch, &far, (frames[0].0, frames[0].1), &tris, false);
        // The wall is the outermost surface everywhere (depth 0 > -0.3): no glass seen.
        assert_eq!(glass, 0.0);
        // Without the wall in front of it, the pane is captured, recessed (darker).
        let tris: Vec<usize> = vec![2, 3];
        let mut page = vec![0u8; PAGE * PAGE * 4];
        let (_, glass, _, _) = capture_chart(&mut page, &ch, &far, (frames[0].0, frames[0].1), &tris, false);
        assert!((glass - 16.0).abs() < 1e-6, "a 2 m x 2 m pane at 0.5 m texels is 16 texels of glass, got {glass}");
        let texel = |tx: usize, ty: usize| {
            let o = ((ty + PAD) * PAGE + tx + PAD) * 4;
            [page[o], page[o + 1], page[o + 2], page[o + 3]]
        };
        // u 5 m, v 3 m -> texel (10, 6) (rows from the top: (6 - 3) / 0.5).
        assert_eq!(texel(10, 6)[3], 255, "the pane's texel is glass");
        assert_eq!(texel(1, 1)[3], 0, "outside the pane: the fill, no glass");
        assert_eq!(texel(1, 1)[0], linear_to_srgb_byte(wall[0]), "outside the pane: the wall colour");
        // The border replicates the chart's edge.
        let o = 0;
        let first = (PAD * PAGE + PAD) * 4;
        assert_eq!(page[o..o + 4], page[first..first + 4]);
    }

    #[test]
    fn a_rhythm_keeps_its_shares_and_splits_tower_bands_into_floors() {
        // a row house: 3.2 m storeys, 3.6 m bays, 1.2 x 1.6 m windows, sill at 4.1 m.
        let r = Rhythm::from_medians(3.2, 3.6, 1.2, 1.6, 4.1).unwrap();
        assert!((r.storey_m - 3.2).abs() < 1e-6 && (r.bay_m - 3.6).abs() < 1e-6);
        assert!((r.window_w - 1.2 / 3.6).abs() < 1e-6 && (r.window_h - 0.5).abs() < 1e-6);
        assert!((r.phase - (4.1f32 / 3.2).fract()).abs() < 1e-5);
        // A tower's 16 m plan storey of full-width glass: four 4 m floors, 1.5 m-ish panes.
        let t = Rhythm::from_medians(16.0, 5.2, 5.0, 11.5, 30.0).unwrap();
        assert!((t.storey_m - 4.0).abs() < 1e-6, "{t:?}");
        assert!((t.bay_m - 5.2 / 3.0).abs() < 1e-6 && (t.window_w - 0.88).abs() < 1e-6, "{t:?}");
        assert!((t.window_h - 11.5 / 16.0).abs() < 1e-6, "the floor keeps the band's glass share");
        assert!(Rhythm::from_medians(1.0, 3.0, 1.0, 1.0, 0.0).is_none(), "no storey, no rhythm");
    }

    #[test]
    fn glass_texels_hold_the_room_seen_through_the_pane_and_the_wall_mean_leaves_glass_out() {
        // a 10 m x 6 m wall with a 2 m x 2 m light pane in a 2 m x 2 m hole, 0.1 m behind.
        let ring = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 8.0], [0.0, 8.0]];
        let frames = edge_frames(&ring);
        let wall = [0.6f32, 0.5, 0.4];
        let pane = [0.46f32, 0.58, 0.6]; // a clear pane's light tint (glass-clear)
        let quad = |x0: f32, x1: f32, y0: f32, y1: f32, z: f32, rgb: [f32; 3], glass: bool| {
            [
                SrcTri { p: [[x0, y0, z], [x1, y0, z], [x1, y1, z]], n: [0.0, 0.0, -1.0], rgb, glass, two_sided: false },
                SrcTri { p: [[x0, y0, z], [x1, y1, z], [x0, y1, z]], n: [0.0, 0.0, -1.0], rgb, glass, two_sided: false },
            ]
        };
        let mut src = vec![];
        src.extend(quad(0.0, 4.0, 0.0, 6.0, 0.0, wall, false));
        src.extend(quad(6.0, 10.0, 0.0, 6.0, 0.0, wall, false));
        src.extend(quad(4.0, 6.0, 0.0, 2.0, 0.0, wall, false));
        src.extend(quad(4.0, 6.0, 4.0, 6.0, 0.0, wall, false));
        src.extend(quad(4.0, 6.0, 2.0, 4.0, 0.03, pane, true));
        let far = Far { group: 1, building: 0, ring, party: vec![false; 4], bottom: 0.0, top: 6.0, src, roof: vec![], wall_rgb: wall, roof_rgb: wall, textured: true, rhythm: [0.0; 5], tower: false, roofs: RoofCap::default() };
        let ch = Chart { far: 0, edge: 0, u0: 0.0, u1: 10.0, v0: 0.0, v1: 6.0, w: 20, h: 12, page: 0, x: 0, y: 0 };
        let tris: Vec<usize> = (0..far.src.len()).collect();
        let mut page = vec![0u8; PAGE * PAGE * 4];
        let (sum, glass, opaque, opaque_w) = capture_chart(&mut page, &ch, &far, (frames[0].0, frames[0].1), &tris, true);
        assert!((glass - 16.0).abs() < 1e-6, "{glass}");
        assert!((opaque_w - 224.0).abs() < 1e-6, "every other texel is opaque: {opaque_w}");
        for q in 0..3 {
            assert!((opaque[q] / opaque_w - wall[q] as f64).abs() < 1e-5, "the wall mean is the wall alone");
        }
        let o = ((6 + PAD) * PAGE + 10 + PAD) * 4;
        let body = glass_body(pane);
        for q in 0..3 {
            assert_eq!(page[o + q], linear_to_srgb_byte(body[q]), "the pane's texel is the room through it");
            assert!(body[q] < 0.1 && body[q] < wall[q] * 0.25, "glass reads darker than the wall by day");
        }
        // The mean is the share-weighted mix: wall where opaque, the body where glass.
        for q in 0..3 {
            let want = (opaque[q] + 16.0 * body[q] as f64) / 240.0;
            assert!((sum[q] / 240.0 - want).abs() < 1e-5);
        }
        // A dark pane keeps its own colour.
        assert_eq!(glass_body([0.02, 0.022, 0.024]), [0.02, 0.022, 0.024]);
    }
}
