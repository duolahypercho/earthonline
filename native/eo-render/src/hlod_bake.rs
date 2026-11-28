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
pub const BAKE_VERSION: u32 = 7; // 2: mips+BC3; 3: glass_body+rhythm+tower; 4: 3D LiDAR/archetype massing + material class; 5: frame.edges parity + mansards + flat-roof detail; 6: exact tower floorHeight+base_y rhythm, welded VF_ROOF verts, trimmed far roof clutter; 7: 3D projecting cornices, textured bay stacks, SF water tanks, VF_RESIDENTIAL night metadata
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
/// Architectural facade material/glazing class packed in alpha bits 4..6 (`(class & 7) << 4`).
pub const VF_MAT_SHIFT: u32 = 4;
/// Residential building-use flag packed in alpha bit 7 (`0x80`, mirrored in `hlod_draw.wgsl` as
/// `HL_RESIDENTIAL = 128u`) so night shaders give commercial/tech towers crisp multi-temp cool/neutral
/// office lighting while residential hillsides & condo/hotel towers glow warm amber + TV/study rooms.
pub const VF_RESIDENTIAL: u32 = 128;
pub const MAT_DEFAULT: u8 = 0;
pub const MAT_CURTAIN_BLUE: u8 = 1;
pub const MAT_CURTAIN_EMERALD: u8 = 2;
pub const MAT_CURTAIN_BRONZE: u8 = 3;
pub const MAT_GRANITE: u8 = 4;
pub const MAT_PRECAST: u8 = 5;
pub const MAT_BRICK: u8 = 6;
pub const MAT_WOOD: u8 = 7;

/// Encoded 3D silhouette massing form carried in `BuildingSummary::massing_form`.
pub const MASSING_FLAT: f32 = 0.0;
pub const MASSING_GABLE: f32 = 1.0;
pub const MASSING_HIP: f32 = 2.0;
pub const MASSING_SHED: f32 = 3.0;
pub const MASSING_STEPPED: f32 = 4.0;
pub const MASSING_CROWN: f32 = 5.0;
pub const MASSING_MANSARD: f32 = 6.0;

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
    h.update(eo_data::lidar_buildings::cache_tag().as_bytes());
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
    /// 3D roof/silhouette form (`MASSING_*`: 0 flat, 1 gable, 2 hip, 3 shed, 4 stepped, 5 crown).
    pub massing_form: f32,
    /// Height above `top` of the roof ridge (gable/hip/shed) or upper setback/crown tier, metres.
    pub ridge_rise: f32,
    /// Principal roof ridge or slope orientation angle, radians (x-east, z-south; NaN/0 = auto OBB).
    pub roof_axis: f32,
    /// Primary rooftop penthouse / setback footprint fraction (`0.0` = none, `0.2..0.82`).
    pub pent_scale: f32,
    /// Primary rooftop penthouse / bulkhead height above `top` (or above crown tier), metres.
    pub pent_h: f32,
    /// Normalized offset of primary penthouse / gable ridge along OBB `(u, v)` in `[-0.4, +0.4]`.
    pub pent_off: [f32; 2],
    /// Architectural facade material/glazing class (`MAT_*` as f32, `0..=7`).
    pub mat_class: f32,
    /// 1.0 for residential / apartment / hotel buildings (`VF_RESIDENTIAL` bit 7), 0.0 for office/commercial/civic.
    pub residential: f32,
}

impl BuildingSummary {
    /// its floats as stored (after the group and building words).
    pub const FLOATS: usize = 27;
    pub fn floats(&self) -> [f32; Self::FLOATS] {
        let (f, r, w, y) = (self.facade, self.roof, self.wall, self.rhythm);
        [
            f[0],
            f[1],
            f[2],
            self.glass,
            r[0],
            r[1],
            r[2],
            self.base,
            self.top,
            w[0],
            w[1],
            w[2],
            y[0],
            y[1],
            y[2],
            y[3],
            y[4],
            self.tower,
            self.massing_form,
            self.ridge_rise,
            self.roof_axis,
            self.pent_scale,
            self.pent_h,
            self.pent_off[0],
            self.pent_off[1],
            self.mat_class,
            self.residential,
        ]
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
            massing_form: v[18],
            ridge_rise: v[19],
            roof_axis: v[20],
            pent_scale: v[21],
            pent_h: v[22],
            pent_off: [v[23], v[24]],
            mat_class: v[25],
            residential: v[26],
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

/// Compact 3D silhouette massing descriptor shared by both cell bakes and districts.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MassingDesc {
    pub form: f32,
    pub ridge_rise: f32,
    pub roof_axis: f32,
    pub pent_scale: f32,
    pub pent_h: f32,
    pub pent_off: [f32; 2],
}

/// Minimum-area oriented 2D bounding box of a footprint ring (x east, z south).
#[derive(Clone, Copy, Debug)]
pub struct Obb2D {
    pub center: [f64; 2],
    /// Unit axis along the longer (or requested) direction.
    pub eu: [f64; 2],
    /// Perpendicular unit axis `(-eu[1], eu[0])`.
    pub ev: [f64; 2],
    /// Half-extents along `eu` and `ev`.
    pub hu: f64,
    pub hv: f64,
    /// Footprint area divided by OBB area (`0..=1`; rectangles are `~1.0`, L-shapes `< 0.75`).
    pub fill: f64,
}

impl Obb2D {
    pub fn point(&self, u: f64, v: f64) -> [f64; 2] {
        [self.center[0] + self.eu[0] * u + self.ev[0] * v, self.center[1] + self.eu[1] * u + self.ev[1] * v]
    }
}

/// Ray-casting point-in-polygon test on an open ring.
pub fn point_in_ring(ring: &[[f64; 2]], p: [f64; 2]) -> bool {
    let n = ring.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (ring[i], ring[j]);
        if ((a[1] > p[1]) != (b[1] > p[1])) && (p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]) {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Fit an oriented bounding box to `ring`. When `forced_axis` is `Some(rad)`,
/// aligns `eu` to `(cos(rad), sin(rad))`; otherwise finds the minimum-area OBB
/// aligned to one of the ring's edges with `hu >= hv`.
pub fn fit_obb(ring: &[[f64; 2]], forced_axis: Option<f64>) -> Option<Obb2D> {
    let n = ring.len();
    if n < 3 {
        return None;
    }
    let poly_area = signed_area(ring).abs();
    if poly_area < 2.0 {
        return None;
    }
    let eval = |ang: f64| {
        let (s, c) = ang.sin_cos();
        let (eu, ev) = ([c, s], [-s, c]);
        let (mut u0, mut u1, mut v0, mut v1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for p in ring {
            let u = p[0] * eu[0] + p[1] * eu[1];
            let v = p[0] * ev[0] + p[1] * ev[1];
            u0 = u0.min(u);
            u1 = u1.max(u);
            v0 = v0.min(v);
            v1 = v1.max(v);
        }
        let hu = 0.5 * (u1 - u0);
        let hv = 0.5 * (v1 - v0);
        let uc = 0.5 * (u0 + u1);
        let vc = 0.5 * (v0 + v1);
        let center = [eu[0] * uc + ev[0] * vc, eu[1] * uc + ev[1] * vc];
        let area = (4.0 * hu * hv).max(1e-6);
        Obb2D { center, eu, ev, hu, hv, fill: (poly_area / area).clamp(0.0, 1.0) }
    };
    if let Some(ang) = forced_axis {
        return Some(eval(ang));
    }
    let mut best: Option<(f64, Obb2D)> = None;
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
        if dx * dx + dz * dz < 1e-4 {
            continue;
        }
        let mut obb = eval(dz.atan2(dx));
        if obb.hv > obb.hu {
            let old_eu = obb.eu;
            obb.eu = obb.ev;
            obb.ev = [-old_eu[0], -old_eu[1]];
            std::mem::swap(&mut obb.hu, &mut obb.hv);
        }
        let a2 = 4.0 * obb.hu * obb.hv;
        if best.is_none_or(|(ba, _)| a2 < ba - 1e-9) {
            best = Some((a2, obb));
        }
    }
    best.map(|(_, o)| o)
}

/// Deterministic per-building hash in `[0, 1)`.
pub fn massing_hash(id: u32, salt: u32) -> f64 {
    let mut h = id.wrapping_mul(0x9E37_79B9) ^ salt.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846C_A68B);
    h ^= h >> 16;
    (h >> 8) as f64 / 16_777_216.0
}

/// Build an oriented rectangular 2D quad `[c0, c1, c2, c3]` inside `ring` for a
/// rooftop penthouse / mechanical room / setback tier. Returns `None` if no
/// clean interior placement fits (e.g. a hollow ring courtyard).
pub fn fit_interior_box(ring: &[[f64; 2]], obb: &Obb2D, scale: f64, off_u: f64, off_v: f64, min_half_m: f64) -> Option<[[f64; 2]; 4]> {
    let mut s = scale.clamp(0.18, 0.85);
    let mut ou = off_u.clamp(-0.35, 0.35);
    let mut ov = off_v.clamp(-0.35, 0.35);
    for attempt in 0..3 {
        let hu = (obb.hu * s).max(min_half_m).min(obb.hu * 0.92);
        let hv = (obb.hv * s).max(min_half_m).min(obb.hv * 0.92);
        if hu < min_half_m * 0.8 || hv < min_half_m * 0.8 {
            return None;
        }
        let max_du = (obb.hu - hu - 0.35).max(0.0);
        let max_dv = (obb.hv - hv - 0.35).max(0.0);
        let du = (ou * obb.hu).clamp(-max_du, max_du);
        let dv = (ov * obb.hv).clamp(-max_dv, max_dv);
        let quad = [
            obb.point(du - hu, dv - hv),
            obb.point(du + hu, dv - hv),
            obb.point(du + hu, dv + hv),
            obb.point(du - hu, dv + hv),
        ];
        // Ensure all 4 corners + center lie inside the building footprint (zero floating over L-notches).
        if obb.fill >= 0.88 || (point_in_ring(ring, obb.point(du, dv)) && quad.iter().all(|&p| point_in_ring(ring, p))) {
            return Some(quad);
        }
        // Recenter and shrink if an offset box clipped an L-shaped notch.
        if attempt == 0 {
            ou = 0.0;
            ov = 0.0;
        } else {
            s *= 0.72;
        }
    }
    None
}

/// Build the oriented 2D base quad `[c0, c1, c2, c3]` (positive signed area, `c0->c1` along
/// the longer axis) for a pitched roof (gable, hip, or shed). Full OBB footprint when
/// `obb.fill >= 0.78`; largest strictly-inscribed main-wing rectangle when `0.60 <= obb.fill < 0.78`
/// so L-shaped & lightwell-notched SF row houses get crisp pitched silhouettes with zero
/// floating overhang over lightwells or rear courts.
pub fn pitched_roof_quad(ring: &[[f64; 2]], obb: &Obb2D) -> Option<[[f64; 2]; 4]> {
    if obb.hu < 1.8 || obb.hv < 1.5 {
        return None;
    }
    let orient = |mut q: [[f64; 2]; 4], align_longest: bool| -> [[f64; 2]; 4] {
        if align_longest {
            let l01 = (q[1][0] - q[0][0]).hypot(q[1][1] - q[0][1]);
            let l12 = (q[2][0] - q[1][0]).hypot(q[2][1] - q[1][1]);
            if l12 > l01 {
                q = [q[1], q[2], q[3], q[0]];
            }
        }
        if signed_area(&q) < 0.0 {
            q.swap(0, 3);
            q.swap(1, 2);
        }
        q
    };
    if obb.fill >= 0.78 {
        let inset = if obb.fill < 0.92 { 0.96 } else { 0.995 };
        let (hu, hv) = (obb.hu * inset, obb.hv * inset);
        return Some(orient([obb.point(-hu, -hv), obb.point(hu, -hv), obb.point(hu, hv), obb.point(-hu, hv)], false));
    }
    if obb.fill < 0.60 {
        return None;
    }
    // Candidate interior rectangles in OBB (u0, u1, v0, v1) fractions in [-1, 1]:
    // centered core, 4 half-wings for L-shaped houses, and 4 quadrant-shifted cores for lightwells.
    const CANDIDATES: [[f64; 4]; 9] = [
        [-0.86, 0.86, -0.86, 0.86],
        [-0.94, 0.94, -0.94, -0.04],
        [-0.94, 0.94, 0.04, 0.94],
        [-0.94, -0.04, -0.94, 0.94],
        [0.04, 0.94, -0.94, 0.94],
        [-0.90, 0.50, -0.90, 0.50],
        [-0.50, 0.90, -0.90, 0.50],
        [-0.90, 0.50, -0.50, 0.90],
        [-0.50, 0.90, -0.50, 0.90],
    ];
    let mut best: Option<(f64, [[f64; 2]; 4])> = None;
    for [fu0, fu1, fv0, fv1] in CANDIDATES {
        let (u0, u1) = (obb.hu * fu0, obb.hu * fu1);
        let (v0, v1) = (obb.hv * fv0, obb.hv * fv1);
        let w = u1 - u0;
        let d = v1 - v0;
        if w < 3.2 || d < 3.2 {
            continue;
        }
        let area = w * d;
        if best.is_some_and(|(ba, _)| area <= ba) {
            continue;
        }
        let quad = [obb.point(u0, v0), obb.point(u1, v0), obb.point(u1, v1), obb.point(u0, v1)];
        let mid = obb.point(0.5 * (u0 + u1), 0.5 * (v0 + v1));
        if point_in_ring(ring, mid) && quad.iter().all(|&p| point_in_ring(ring, p)) {
            best = Some((area, quad));
        }
    }
    best.map(|(_, q)| orient(q, true))
}

/// Deterministic pitched-roof material finish (composition charcoal-slate, weathered terracotta tile,
/// cedar shingle, or slate) blended gently with the building's NAIP measured roof colour so real
/// SF homeowners recognize their actual roof tone while sloped roofs separate crisply across cells & districts.
pub fn pitched_roof_color(roof_rgb: [f32; 3], group: u32, form: i32) -> [f32; 3] {
    pitched_roof_color_measured(roof_rgb, group, form, true)
}

pub fn pitched_roof_color_measured(roof_rgb: [f32; 3], group: u32, form: i32, has_measured: bool) -> [f32; 3] {
    let h = massing_hash(group, 109);
    let finish = if form == 6 {
        // Classic SF Edwardian/Victorian mansard finishes: dark slate, weathered terracotta, charcoal shingle, sage-slate.
        if h < 0.36 {
            [0.068, 0.078, 0.095] // charcoal-blue mansard slate
        } else if h < 0.62 {
            [0.31, 0.140, 0.092] // weathered terracotta tile/shingle
        } else if h < 0.82 {
            [0.145, 0.102, 0.078] // deep weathered cedar/umber shingle
        } else {
            [0.115, 0.132, 0.128] // cool grey-green slate
        }
    } else {
        // Slightly higher Mediterranean terracotta share on hipped roofs; crisp slate/shingle on gables.
        let terra_cut = if form == 2 { 0.36 } else { 0.25 };
        if h < terra_cut {
            [0.34, 0.145, 0.095] // weathered clay barrel tile / terracotta
        } else if h < 0.66 {
            [0.072, 0.078, 0.088] // dark charcoal composition shingle
        } else if h < 0.86 {
            [0.165, 0.112, 0.082] // warm weathered cedar / brown shingle
        } else {
            [0.125, 0.142, 0.138] // cool grey-green slate
        }
    };
    let lum = 0.2126 * roof_rgb[0] + 0.7152 * roof_rgb[1] + 0.0722 * roof_rgb[2];
    let w = if has_measured {
        if form == 6 { 0.44 } else if lum > 0.24 { 0.38 } else { 0.30 }
    } else if form == 6 {
        0.58
    } else if lum > 0.22 {
        0.52
    } else {
        0.42
    };
    [0, 1, 2].map(|k| roof_rgb[k] * (1.0 - w) + finish[k] * w)
}

/// Enrich flat-roof membrane colour diversity and add deterministic weathered two-tone
/// re-roofing patches (charcoal tar, dusty gravel aggregate, silver elastomeric cap-sheet)
/// when NAIP did not supply a high-confidence second roof patch, eliminating sterile uniform
/// flat grey roofs across hillside neighborhoods.
fn enrich_flat_roof(roof_rgb: &mut [f32; 3], roofs: &mut RoofCap, ring: &[[f64; 2]], group: u32) {
    if roofs.pitched {
        return;
    }
    let area = signed_area(ring).abs();
    if area < 35.0 {
        return;
    }
    // Break up monotonous neutral mid-grey roofs with authentic SF flat-roof membrane tones.
    let cmax = roof_rgb[0].max(roof_rgb[1]).max(roof_rgb[2]);
    let cmin = roof_rgb[0].min(roof_rgb[1]).min(roof_rgb[2]);
    let chroma = cmax - cmin;
    let h_mat = massing_hash(group, 127);
    if chroma < 0.045 {
        let (target, w) = if h_mat < 0.28 {
            ([0.078, 0.080, 0.084], 0.36) // weathered dark built-up asphalt tar
        } else if h_mat < 0.54 {
            ([0.245, 0.215, 0.175], 0.32) // warm dusty tan pea-gravel ballast
        } else if h_mat < 0.76 {
            ([0.205, 0.170, 0.142], 0.28) // warm weathered buff/terracotta aggregate
        } else if h_mat < 0.90 {
            ([0.275, 0.285, 0.300], 0.26) // cool silver-grey elastomeric cap-sheet
        } else {
            ([0.135, 0.140, 0.142], 0.30) // weathered dark slate membrane
        };
        for k in 0..3 {
            roof_rgb[k] = (roof_rgb[k] * (1.0 - w) + target[k] * w).clamp(0.04, 0.52);
        }
    }
    // Synthesize a clean edge-aligned re-roofed patch when NAIP has none.
    if roofs.patch.is_none() && area >= 55.0 && eo_data::roof_colours::feature_on("patches") {
        let h_pat = massing_hash(group, 131);
        if h_pat < 0.68 {
            let h_dir = massing_hash(group, 137);
            let h_sh = massing_hash(group, 139);
            let h_col = massing_hash(group, 149);
            let ang = h_dir * std::f64::consts::TAU;
            let dir = [ang.cos(), ang.sin()];
            let share = (0.22 + 0.26 * h_sh) as f32;
            let patch_rgb = if h_col < 0.38 {
                [roof_rgb[0] * 0.66, roof_rgb[1] * 0.67, roof_rgb[2] * 0.69] // fresh dark tar repair
            } else if h_col < 0.68 {
                [(roof_rgb[0] * 1.18).min(0.46), (roof_rgb[1] * 1.13).min(0.44), (roof_rgb[2] * 1.03).min(0.40)] // warm gravel section
            } else {
                [(roof_rgb[0] * 1.18).min(0.48), (roof_rgb[1] * 1.20).min(0.49), (roof_rgb[2] * 1.23).min(0.51)] // pale elastomeric coat
            };
            roofs.patch = Some((patch_rgb, dir, share));
        }
    }
}

/// Emit compact lived-in San Francisco flat-roof assemblies (256 m cells only, zero district cost):
/// raised glass skylights / lightwell monitors and occasional warm hillside redwood rooftop decks
/// strictly inscribed inside `ring`.
#[allow(clippy::too_many_arguments)]
pub fn emit_flat_roof_detail(
    ring: &[[f64; 2]],
    group: u32,
    base_y: f64,
    top: f64,
    massing: &MassingDesc,
    out: &mut Vec<([[f32; 3]; 3], [f32; 3], [f32; 3])>,
) {
    let form = massing.form.round() as i32;
    if form != 0 {
        return;
    }
    let Some(obb) = fit_obb(ring, None) else { return };
    let gs = eo_data::groundscale::scale().max(1e-3);
    let true_area = signed_area(ring).abs() / (gs * gs);
    let height = (top - base_y).max(2.0);
    if true_area < 60.0 || height > 44.0 {
        return;
    }
    let push_tri = |a: [f32; 3], b: [f32; 3], c: [f32; 3], rgb: [f32; 3], out: &mut Vec<([[f32; 3]; 3], [f32; 3], [f32; 3])>| {
        let p = [a, b, c];
        let n = face_normal(&p);
        if n[0] * n[0] + n[1] * n[1] + n[2] * n[2] > 0.25 {
            out.push((p, n, rgb));
        }
    };
    let push_quad = |a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3], rgb: [f32; 3], out: &mut Vec<([[f32; 3]; 3], [f32; 3], [f32; 3])>| {
        push_tri(a, b, c, rgb, out);
        push_tri(a, c, d, rgb, out);
    };
    let push_box = |quad: [[f64; 2]; 4], y0: f32, y1: f32, side_rgb: [f32; 3], cap_rgb: [f32; 3], out: &mut Vec<([[f32; 3]; 3], [f32; 3], [f32; 3])>| {
        let mut q = quad;
        if signed_area(&q) < 0.0 {
            q.reverse();
        }
        let b = q.map(|p| [p[0] as f32, y0, p[1] as f32]);
        let t = q.map(|p| [p[0] as f32, y1, p[1] as f32]);
        for i in 0..4 {
            let j = (i + 1) % 4;
            push_quad(b[i], b[j], t[j], t[i], side_rgb, out);
        }
        push_quad(t[0], t[1], t[2], t[3], cap_rgb, out);
    };

    let y0 = top as f32;
    let h0 = massing_hash(group, 151);
    let h1 = massing_hash(group, 157);
    let h2 = massing_hash(group, 163);
    // Offset skylight away from any existing penthouse along OBB axes.
    let sky_u = if massing.pent_h > 0.8 { -massing.pent_off[0] as f64 * 0.85 } else { (h1 - 0.5) * 0.45 };
    let sky_v = if massing.pent_h > 0.8 { -massing.pent_off[1] as f64 * 0.85 } else { (h2 - 0.5) * 0.45 };

    // 1. Raised glass skylight / lightwell monitor (classic SF flat-roof detail).
    if h0 < 0.62 {
        let sky_scale = (0.18 + 0.08 * h1).clamp(0.16, 0.28);
        let curb_rgb = [0.54, 0.52, 0.49];
        let glass_rgb = [0.042, 0.068, 0.088];
        if let Some(q) = fit_interior_box(ring, &obb, sky_scale, sky_u, sky_v, 1.0) {
            push_box(q, y0 + 0.02, y0 + 0.48, curb_rgb, glass_rgb, out);
        }
    }

    // 2. Warm hillside redwood/cedar rooftop deck on ~26% of bare residential hillside flat roofs
    // (single 2-triangle flat quad at +0.08 m — 4 welded verts instead of a 10-tri box).
    if base_y > 12.0 && height <= 28.0 && true_area >= 65.0 && h1 < 0.26 {
        let deck_u = -sky_u * 0.8 + (h2 - 0.5) * 0.2;
        let deck_v = -sky_v * 0.8;
        let deck_rgb = [0.245, 0.145, 0.088];
        if let Some(mut q) = fit_interior_box(ring, &obb, 0.34, deck_u, deck_v, 1.4) {
            if signed_area(&q) < 0.0 {
                q.reverse();
            }
            let t = q.map(|p| [p[0] as f32, y0 + 0.08, p[1] as f32]);
            push_quad(t[0], t[1], t[2], t[3], deck_rgb, out);
        }
    }
}

/// Whether a building qualifies for an iconic SF hexagonal redwood rooftop water tank
/// (shared deterministically between 256 m cells and 2 km districts).
pub fn wants_water_tank(group: u32, height: f64, true_area: f64, mat_class: u8, form: i32) -> bool {
    if !matches!(form, 0 | 4) || !(14.0..=58.0).contains(&height) || true_area < 190.0 {
        return false;
    }
    if matches!(mat_class, MAT_CURTAIN_BLUE | MAT_CURTAIN_EMERALD | MAT_CURTAIN_BRONZE) {
        return false;
    }
    let chance = if mat_class == MAT_BRICK { 0.34 } else { 0.20 };
    massing_hash(group, 173) < chance
}

/// Emit a 3D hexagonal weathered SF redwood rooftop water tank perched on a dark steel support base
/// with a conical roof cap (~20 welded verts / 26 tris in `Far::roof`).
#[allow(clippy::too_many_arguments)]
pub fn emit_water_tank(
    ring: &[[f64; 2]],
    group: u32,
    top: f64,
    height: f64,
    mat_class: u8,
    massing: &MassingDesc,
    out: &mut Vec<([[f32; 3]; 3], [f32; 3], [f32; 3])>,
) {
    let form = massing.form.round() as i32;
    let gs = eo_data::groundscale::scale().max(1e-3);
    let true_area = signed_area(ring).abs() / (gs * gs);
    if !wants_water_tank(group, height, true_area, mat_class, form) {
        return;
    }
    let Some(obb) = fit_obb(ring, None) else { return };
    let h1 = massing_hash(group, 179);
    let h2 = massing_hash(group, 181);
    // Offset opposite any penthouse/setback box so the water tank stands cleanly on the main roof deck.
    let off_u = if massing.pent_h > 0.8 || form == 4 { -massing.pent_off[0] as f64 - 0.18 } else { (h1 - 0.5) * 0.42 };
    let off_v = if massing.pent_h > 0.8 || form == 4 { -massing.pent_off[1] as f64 + 0.14 } else { (h2 - 0.5) * 0.42 };
    let Some(q) = fit_interior_box(ring, &obb, 0.22, off_u, off_v, 1.8) else { return };
    let cx = 0.25 * (q[0][0] + q[1][0] + q[2][0] + q[3][0]);
    let cz = 0.25 * (q[0][1] + q[1][1] + q[2][1] + q[3][1]);
    let s0 = (q[1][0] - q[0][0]).hypot(q[1][1] - q[0][1]);
    let s1 = (q[2][0] - q[1][0]).hypot(q[2][1] - q[1][1]);
    let r = (1.50 + 0.65 * h1).min(0.45 * s0.min(s1)).max(1.05);
    let base_y = top as f32;
    let leg_y1 = base_y + 0.95;
    let drum_y1 = leg_y1 + (2.55 + 0.90 * h2) as f32;
    let apex_y = drum_y1 + 0.85;
    let steel_rgb = [0.068, 0.072, 0.078];
    let redwood_rgb = [0.235, 0.122, 0.072];
    let cap_rgb = [0.145, 0.148, 0.152];
    let push_tri = |a: [f32; 3], b: [f32; 3], c: [f32; 3], rgb: [f32; 3], out: &mut Vec<([[f32; 3]; 3], [f32; 3], [f32; 3])>| {
        let p = [a, b, c];
        let n = face_normal(&p);
        if n[0] * n[0] + n[1] * n[1] + n[2] * n[2] > 0.25 {
            out.push((p, n, rgb));
        }
    };
    const SIDES: usize = 6;
    let mut ring_leg0 = [[0f32; 3]; SIDES];
    let mut ring_leg1 = [[0f32; 3]; SIDES];
    let mut ring_b0 = [[0f32; 3]; SIDES];
    let mut ring_b1 = [[0f32; 3]; SIDES];
    let rl = r * 0.62;
    for k in 0..SIDES {
        let ang = k as f64 * std::f64::consts::TAU / SIDES as f64;
        let (sa, ca) = ang.sin_cos();
        ring_leg0[k] = [(cx + ca * rl) as f32, base_y - 0.05, (cz + sa * rl) as f32];
        ring_leg1[k] = [(cx + ca * rl) as f32, leg_y1, (cz + sa * rl) as f32];
        ring_b0[k] = [(cx + ca * r) as f32, leg_y1, (cz + sa * r) as f32];
        ring_b1[k] = [(cx + ca * r) as f32, drum_y1, (cz + sa * r) as f32];
    }
    let apex = [cx as f32, apex_y, cz as f32];
    for k in 0..SIDES {
        let j = (k + 1) % SIDES;
        // Dark steel support trestle core (6 quads).
        push_tri(ring_leg0[k], ring_leg0[j], ring_leg1[j], steel_rgb, out);
        push_tri(ring_leg0[k], ring_leg1[j], ring_leg1[k], steel_rgb, out);
        // Weathered redwood barrel staves (6 quads).
        push_tri(ring_b0[k], ring_b0[j], ring_b1[j], redwood_rgb, out);
        push_tri(ring_b0[k], ring_b1[j], ring_b1[k], redwood_rgb, out);
        // Conical galvanized/shingle tank cap (6 tris).
        push_tri(ring_b1[k], ring_b1[j], apex, cap_rgb, out);
    }
}

/// Emit real 3D projecting cornice overhang rims along up to 2 prominent street-facing non-party
/// edges of flat-roofed buildings in 256 m cells (top sunlit coping, vertical fascia, darkened eaves
/// soffit, and watertight end caps; welded to ~8 verts / 8-10 tris per edge in `Far::roof`).
#[allow(clippy::too_many_arguments)]
pub fn emit_cornice_rim(
    ring: &[[f64; 2]],
    party: &[bool],
    front_mask: &[bool],
    top: f64,
    height: f64,
    mat_class: u8,
    wall_rgb: [f32; 3],
    trim_rgb: [f32; 3],
    pitched: bool,
    out: &mut Vec<([[f32; 3]; 3], [f32; 3], [f32; 3])>,
) {
    if pitched || height < 9.0 || matches!(mat_class, MAT_CURTAIN_BLUE | MAT_CURTAIN_EMERALD | MAT_CURTAIN_BRONZE) {
        return;
    }
    let gs = eo_data::groundscale::scale().max(1e-3);
    let true_area = signed_area(ring).abs() / (gs * gs);
    if true_area < 65.0 {
        return;
    }
    let frames = edge_frames(ring);
    let has_front = frames.iter().enumerate().any(|(e, &(_, _, len))| !party.get(e).copied().unwrap_or(false) && front_mask.get(e).copied().unwrap_or(false) && len >= 4.5);
    let mut cands: Vec<(bool, f64, usize)> = frames
        .iter()
        .enumerate()
        .filter(|&(e, &(_, _, len))| {
            !party.get(e).copied().unwrap_or(false)
                && len >= 4.5
                && (!has_front || front_mask.get(e).copied().unwrap_or(false))
        })
        .map(|(e, &(_, _, len))| (front_mask.get(e).copied().unwrap_or(false), len, e))
        .collect();
    if cands.is_empty() {
        return;
    }
    cands.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.total_cmp(&a.1)));

    let out_m = if height >= 16.0 { 0.58 } else if height >= 12.0 { 0.48 } else { 0.42 };
    let ch = if height >= 16.0 { 0.62 } else if height >= 12.0 { 0.52 } else { 0.44 };
    let y_top = (top + 0.14) as f32;
    let y_bot = (top + 0.14 - ch) as f32;
    // Blend architectural trim with wall tone so cornices stay crisp and cohesive.
    let face_rgb = [0, 1, 2].map(|k| (trim_rgb[k] * 0.65 + wall_rgb[k] * 0.35).clamp(0.06, 0.78));
    let top_rgb = [0, 1, 2].map(|k| (face_rgb[k] * 1.12).min(0.84));
    let soffit_rgb = mul3(face_rgb, [0.42; 3]);
    let push_tri = |a: [f32; 3], b: [f32; 3], c: [f32; 3], rgb: [f32; 3], out: &mut Vec<([[f32; 3]; 3], [f32; 3], [f32; 3])>| {
        let p = [a, b, c];
        let n = face_normal(&p);
        if n[0] * n[0] + n[1] * n[1] + n[2] * n[2] > 0.25 {
            out.push((p, n, rgb));
        }
    };
    let push_quad = |a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3], rgb: [f32; 3], out: &mut Vec<([[f32; 3]; 3], [f32; 3], [f32; 3])>| {
        push_tri(a, b, c, rgb, out);
        push_tri(a, c, d, rgb, out);
    };
    let n_ring = ring.len();
    let max_edges = if height >= 14.0 { 2 } else { 1 };
    for &(_, len, e) in cands.iter().take(max_edges) {
        let (u, n, _) = frames[e];
        let a = ring[e];
        let prev_open = !party.get((e + n_ring - 1) % n_ring).copied().unwrap_or(false);
        let next_open = !party.get((e + 1) % n_ring).copied().unwrap_or(false);
        let s0 = if prev_open { -0.14 } else { 0.04 };
        let s1 = if next_open { len + 0.14 } else { len - 0.04 };
        if s1 <= s0 + 1.0 {
            continue;
        }
        let pt = |s: f64, d: f64, y: f32| [(a[0] + u[0] * s + n[0] * d) as f32, y, (a[1] + u[1] * s + n[1] * d) as f32];
        let ib0 = pt(s0, 0.0, y_bot);
        let ib1 = pt(s1, 0.0, y_bot);
        let it0 = pt(s0, 0.0, y_top);
        let it1 = pt(s1, 0.0, y_top);
        let ob0 = pt(s0, out_m, y_bot);
        let ob1 = pt(s1, out_m, y_bot);
        let ot0 = pt(s0, out_m, y_top);
        let ot1 = pt(s1, out_m, y_top);
        // Sunlit top coping, projecting front fascia, and darkened underside eaves soffit (6 tris / 8 welded verts).
        push_quad(it0, it1, ot1, ot0, top_rgb, out);
        push_quad(ob0, ob1, ot1, ot0, face_rgb, out);
        push_quad(ib1, ib0, ob0, ob1, soffit_rgb, out);
    }
}

/// Emit pitched roof triangles `(tri, outward_normal, linear_rgb)` for a cell mesh (`Far::roof`)
/// so 256 m cells and 2 km districts show identical 3D silhouettes.
#[allow(clippy::too_many_arguments)]
pub fn emit_massing_roof_tris(
    ring: &[[f64; 2]],
    group: u32,
    top: f64,
    massing: &MassingDesc,
    wall_rgb: [f32; 3],
    roof_rgb: [f32; 3],
    skip_pitched: bool,
    skip_boxes: bool,
    out: &mut Vec<([[f32; 3]; 3], [f32; 3], [f32; 3])>,
) -> bool {
    let form = massing.form.round() as i32;
    let forced = (massing.roof_axis.is_finite() && massing.roof_axis.abs() > 1e-5).then_some(massing.roof_axis as f64);
    let Some(obb) = fit_obb(ring, if matches!(form, 1 | 3) { forced } else { None }) else { return false };
    let push_tri = |a: [f32; 3], b: [f32; 3], c: [f32; 3], rgb: [f32; 3], out: &mut Vec<([[f32; 3]; 3], [f32; 3], [f32; 3])>| {
        let p = [a, b, c];
        let n = face_normal(&p);
        if n[0] * n[0] + n[1] * n[1] + n[2] * n[2] > 0.25 {
            out.push((p, n, rgb));
        }
    };
    let push_quad = |a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3], rgb: [f32; 3], out: &mut Vec<([[f32; 3]; 3], [f32; 3], [f32; 3])>| {
        push_tri(a, b, c, rgb, out);
        push_tri(a, c, d, rgb, out);
    };
    let push_box = |quad: [[f64; 2]; 4], y0: f32, y1: f32, side_rgb: [f32; 3], cap_rgb: [f32; 3], out: &mut Vec<([[f32; 3]; 3], [f32; 3], [f32; 3])>| {
        let mut q = quad;
        if signed_area(&q) < 0.0 {
            q.reverse();
        }
        let b = q.map(|p| [p[0] as f32, y0, p[1] as f32]);
        let t = q.map(|p| [p[0] as f32, y1, p[1] as f32]);
        for i in 0..4 {
            let j = (i + 1) % 4;
            push_quad(b[i], b[j], t[j], t[i], side_rgb, out);
        }
        push_quad(t[0], t[1], t[2], t[3], cap_rgb, out);
    };

    let y0 = top as f32;
    let rise = massing.ridge_rise.max(0.0);
    let mut emitted_full_pitched = false;
    if !skip_pitched && rise >= 0.8 && matches!(form, 1..=3 | 6) {
        if let Some([c0, c1, c2, c3]) = pitched_roof_quad(ring, &obb) {
            let eave_y = y0 + if obb.fill < 0.90 { 0.05 } else { 0.0 };
            let yr = eave_y + rise;
            let p3 = |p: [f64; 2], y: f32| [p[0] as f32, y, p[1] as f32];
            let slope_rgb = pitched_roof_color(roof_rgb, group, form);
            let m0 = [0.5 * (c3[0] + c0[0]), 0.5 * (c3[1] + c0[1])];
            let m1 = [0.5 * (c1[0] + c2[0]), 0.5 * (c1[1] + c2[1])];
            let u_vec = [m1[0] - m0[0], m1[1] - m0[1]];
            let u_len = u_vec[0].hypot(u_vec[1]).max(1e-4);
            let v_vec = [c3[0] - c0[0], c3[1] - c0[1]];
            let v_len = v_vec[0].hypot(v_vec[1]).max(1e-4);
            match form {
                1 => {
                    // Gable ridge along c0->c1 axis, sloped roof planes + wall-coloured pediment ends.
                    let shift = (massing.pent_off[0] as f64).clamp(-0.35, 0.35) * 0.5;
                    let r0 = [m0[0] + v_vec[0] * shift, m0[1] + v_vec[1] * shift];
                    let r1 = [m1[0] + v_vec[0] * shift, m1[1] + v_vec[1] * shift];
                    push_quad(p3(c0, eave_y), p3(c1, eave_y), p3(r1, yr), p3(r0, yr), slope_rgb, out);
                    push_quad(p3(c2, eave_y), p3(c3, eave_y), p3(r0, yr), p3(r1, yr), slope_rgb, out);
                    push_tri(p3(c3, eave_y), p3(c0, eave_y), p3(r0, yr), wall_rgb, out);
                    push_tri(p3(c1, eave_y), p3(c2, eave_y), p3(r1, yr), wall_rgb, out);
                }
                2 => {
                    // 4-sided Hip roof: ridge along longer axis, pyramid when nearly square.
                    let t_inset = (0.42 * v_len / u_len).clamp(0.08, 0.49);
                    let r0 = [m0[0] + u_vec[0] * t_inset, m0[1] + u_vec[1] * t_inset];
                    let r1 = [m1[0] - u_vec[0] * t_inset, m1[1] - u_vec[1] * t_inset];
                    push_quad(p3(c0, eave_y), p3(c1, eave_y), p3(r1, yr), p3(r0, yr), slope_rgb, out);
                    push_quad(p3(c2, eave_y), p3(c3, eave_y), p3(r0, yr), p3(r1, yr), slope_rgb, out);
                    push_tri(p3(c3, eave_y), p3(c0, eave_y), p3(r0, yr), slope_rgb, out);
                    push_tri(p3(c1, eave_y), p3(c2, eave_y), p3(r1, yr), slope_rgb, out);
                }
                3 => {
                    // Mono-pitch shed roof tilting down toward c1/c2.
                    push_quad(p3(c0, yr), p3(c1, eave_y), p3(c2, eave_y), p3(c3, yr), slope_rgb, out);
                    push_quad(p3(c3, eave_y), p3(c0, eave_y), p3(c0, yr), p3(c3, yr), wall_rgb, out);
                    push_tri(p3(c0, eave_y), p3(c1, eave_y), p3(c0, yr), wall_rgb, out);
                    push_tri(p3(c2, eave_y), p3(c3, eave_y), p3(c3, yr), wall_rgb, out);
                }
                6 => {
                    // Classic SF Mansard roof: steep sloped slate/shingle perimeter skirt + dark flat tar deck.
                    let iu = (0.26 * v_len / u_len).clamp(0.10, 0.28);
                    let iv = 0.26;
                    let lerp2 = |a: [f64; 2], b: [f64; 2], t: f64| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
                    let bilerp = |fu: f64, fv: f64| lerp2(lerp2(c0, c1, fu), lerp2(c3, c2, fu), fv);
                    let t0 = bilerp(iu, iv);
                    let t1 = bilerp(1.0 - iu, iv);
                    let t2 = bilerp(1.0 - iu, 1.0 - iv);
                    let t3 = bilerp(iu, 1.0 - iv);
                    let deck_rgb = [0.088, 0.092, 0.098];
                    push_quad(p3(c0, eave_y), p3(c1, eave_y), p3(t1, yr), p3(t0, yr), slope_rgb, out);
                    push_quad(p3(c1, eave_y), p3(c2, eave_y), p3(t2, yr), p3(t1, yr), slope_rgb, out);
                    push_quad(p3(c2, eave_y), p3(c3, eave_y), p3(t3, yr), p3(t2, yr), slope_rgb, out);
                    push_quad(p3(c3, eave_y), p3(c0, eave_y), p3(t0, yr), p3(t3, yr), slope_rgb, out);
                    push_quad(p3(t0, yr), p3(t1, yr), p3(t2, yr), p3(t3, yr), deck_rgb, out);
                }
                _ => {}
            }
            emitted_full_pitched = obb.fill >= 0.78;
        }
    }

    if skip_boxes {
        return emitted_full_pitched;
    }
    let mech_wall = mul3(wall_rgb, [0.88, 0.86, 0.83]);
    let mech_cap = [0.092, 0.096, 0.102];
    let hvac_wall = [0.50, 0.53, 0.56];
    let hvac_cap = [0.40, 0.43, 0.46];
    let crown_wall = mul3(wall_rgb, [0.86, 0.88, 0.90]);
    match form {
        4 => {
            // Stepped setback upper tier + optional mechanical bulkhead above it.
            let tier_scale = massing.pent_scale.clamp(0.42, 0.80) as f64;
            let tier_top = y0 + rise.max(2.0);
            if let Some(q1) = fit_interior_box(ring, &obb, tier_scale, massing.pent_off[0] as f64, massing.pent_off[1] as f64, 1.5) {
                push_box(q1, y0 - 0.1, tier_top, wall_rgb, roof_rgb, out);
                if massing.pent_h > 0.8 {
                    if let Some(q2) = fit_interior_box(&q1, &fit_obb(&q1, None).unwrap_or(obb), 0.45, 0.0, 0.0, 1.0) {
                        push_box(q2, tier_top - 0.1, tier_top + massing.pent_h, mech_wall, mech_cap, out);
                    }
                }
            }
        }
        5 => {
            // Tall tower stepped crown + rooftop mechanical / elevator core.
            let crown_top = y0 + rise.max(2.4);
            let crown_scale = (0.66 + 0.12 * (massing.pent_off[0].abs() as f64)).clamp(0.62, 0.78);
            if let Some(q1) = fit_interior_box(ring, &obb, crown_scale, 0.0, 0.0, 2.5) {
                push_box(q1, y0 - 0.15, crown_top, crown_wall, mech_cap, out);
                if massing.pent_h > 0.8 && massing.pent_scale > 0.15 {
                    let q1_obb = fit_obb(&q1, None).unwrap_or(obb);
                    if let Some(q2) = fit_interior_box(&q1, &q1_obb, (massing.pent_scale as f64 / crown_scale).clamp(0.28, 0.68), massing.pent_off[0] as f64, massing.pent_off[1] as f64, 1.5) {
                        push_box(q2, crown_top - 0.1, crown_top + massing.pent_h, mech_wall, mech_cap, out);
                    }
                }
            }
        }
        _ => {
            // Flat roof with 3D rooftop penthouse / stair bulkhead (+ galvanized HVAC enclosure on wider roofs).
            if massing.pent_h > 0.8 && massing.pent_scale > 0.15 {
                if let Some(q) = fit_interior_box(ring, &obb, massing.pent_scale as f64, massing.pent_off[0] as f64, massing.pent_off[1] as f64, 1.1) {
                    push_box(q, y0 - 0.1, y0 + massing.pent_h, mech_wall, mech_cap, out);
                }
                if 4.0 * obb.hu * obb.hv >= 260.0 && massing.pent_h >= 1.7 {
                    let s2 = (massing.pent_scale as f64 * 0.64).clamp(0.15, 0.28);
                    let h2 = (massing.pent_h * 0.58).max(1.3);
                    if let Some(q2) = fit_interior_box(ring, &obb, s2, -massing.pent_off[0] as f64 - 0.18, -massing.pent_off[1] as f64, 1.0) {
                        push_box(q2, y0 - 0.1, y0 + h2, hvac_wall, hvac_cap, out);
                    }
                }
            }
        }
    }
    emitted_full_pitched
}

/// Classify a building's architectural material/glazing class (`MAT_*`, 0..7)
/// for `hlod_draw.wgsl` bits 4..6.
fn classify_mat_class(arch: &str, wall_mat: &str, glass_mat: &str, system: Option<&str>, tower: bool, group: u32) -> u8 {
    if tower {
        if let Some(sys) = system {
            if sys.contains("granite") || sys.contains("limestone") || sys.contains("masonry") {
                return MAT_GRANITE;
            }
            if sys.contains("precast") || sys.contains("concrete") {
                return MAT_PRECAST;
            }
        }
        if wall_mat.contains("granite") || wall_mat.contains("limestone") || wall_mat.contains("stone") {
            return MAT_GRANITE;
        }
        if wall_mat.contains("concrete") || wall_mat.contains("precast") {
            return MAT_PRECAST;
        }
        if wall_mat.contains("brick") {
            return MAT_BRICK;
        }
        // Curtain-wall glass towers: varied realistic SF glass families (blue-silver, seafoam/emerald, bronze).
        let r = massing_hash(group, 71);
        return if glass_mat.contains("film") || wall_mat.contains("aluminium") || wall_mat.contains("metal") {
            if r < 0.48 { MAT_CURTAIN_BLUE } else if r < 0.78 { MAT_CURTAIN_EMERALD } else { MAT_CURTAIN_BRONZE }
        } else if r < 0.55 {
            MAT_CURTAIN_BLUE
        } else {
            MAT_PRECAST
        };
    }
    if wall_mat.contains("wood") || wall_mat.contains("clapboard") || wall_mat.contains("shingle") || arch == "victorian-row" || arch == "cottage-garage" {
        return MAT_WOOD;
    }
    if wall_mat.contains("brick") || arch == "soma-warehouse" {
        return MAT_BRICK;
    }
    if wall_mat.contains("limestone") || wall_mat.contains("granite") || wall_mat.contains("ashlar") {
        return MAT_GRANITE;
    }
    if wall_mat.contains("concrete") || wall_mat.contains("precast") || arch == "civic-concrete" {
        return MAT_PRECAST;
    }
    MAT_DEFAULT
}

/// Classify whether a building is residential/hotel (bit 7 `VF_RESIDENTIAL = 128`) vs
/// commercial/office/civic/industrial so night shaders produce authentic multi-temperature SF lighting.
fn classify_residential(arch: &str, tag: &str, amenity: &str, has_retail_ground: bool, base_y: f64, height: f64, tower: bool) -> bool {
    let t = tag.trim().to_ascii_lowercase();
    if matches!(
        t.as_str(),
        "apartments" | "residential" | "house" | "terrace" | "detached" | "semidetached_house" | "dormitory" | "hotel" | "bungalow"
    ) {
        return true;
    }
    if matches!(
        t.as_str(),
        "office" | "commercial" | "industrial" | "warehouse" | "retail" | "civic" | "public" | "hospital" | "school" | "university" | "church" | "cathedral" | "stadium" | "train_station"
    ) {
        return false;
    }
    let am = amenity.trim().to_ascii_lowercase();
    if matches!(
        am.as_str(),
        "bank" | "school" | "university" | "hospital" | "theatre" | "cinema" | "library" | "courthouse" | "police" | "fire_station" | "townhall"
    ) {
        return false;
    }
    match arch {
        "victorian-row" | "cottage-garage" | "stucco-row" | "edwardian-flats" | "bay-window-apartment" | "midcentury-stucco" => true,
        "downtown-tower" | "civic-concrete" | "soma-warehouse" | "industrial-shed" | "retail-strip" => false,
        _ => !tower && !has_retail_ground && height <= 26.0 && base_y > 14.0,
    }
}

/// Resolve 3D roof/setback/penthouse massing for one building, combining:
/// 1. Measured LiDAR (`eo_data::lidar_buildings::get`) when available;
/// 2. Existing `eo-kit` pitched/penthouse roof geometry;
/// 3. Whole-map architectural synthesis (stepped tower crowns, mid-rise mechanical
///    penthouses, and realistic SF hillside/Victorian pitched roofs).
#[allow(clippy::too_many_arguments)]
fn resolve_massing(
    record_id: &str,
    group: u32,
    ring: &[[f64; 2]],
    base_y: f64,
    top_y: f64,
    arch: &str,
    tower: bool,
    kit_pitched: bool,
    kit_roof_peak: f64,
    kit_has_penthouse: bool,
    roof_patch_share: f32,
) -> (MassingDesc, bool /* emit_extra_in_cell */) {
    let height = (top_y - base_y).max(2.0);
    let area = signed_area(ring).abs();
    let Some(obb) = fit_obb(ring, None) else {
        return (MassingDesc::default(), false);
    };
    // Footprint rings are scaled horizontally by groundscale (default 1.25x); unscale before
    // applying true real-world building area & half-width thresholds (heights are 1:1).
    let gs = eo_data::groundscale::scale().max(1e-3);
    let true_area = area / (gs * gs);
    let true_hv = obb.hv / gs;
    let h0 = massing_hash(group, 13);
    let h1 = massing_hash(group, 29);
    let h2 = massing_hash(group, 47);
    let h3 = massing_hash(group, 61);

    // 1. Direct USGS 3DEP LiDAR measurement (highest fidelity).
    if let Some(lm) = eo_data::lidar_buildings::get(record_id) {
        use eo_data::lidar_buildings::{BOX_SETBACK, RoofForm};
        let mut desc = MassingDesc::default();
        match lm.form {
            RoofForm::Gable => {
                desc.form = MASSING_GABLE;
                desc.ridge_rise = (lm.ridge_h - lm.eave_h).clamp(1.0, 6.0);
                desc.roof_axis = lm.axis_rad;
                desc.pent_off[0] = (lm.ridge_offset_m as f64 / obb.hv.max(2.0)).clamp(-0.35, 0.35) as f32;
                return (desc, !kit_pitched);
            }
            RoofForm::Hip => {
                desc.form = MASSING_HIP;
                desc.ridge_rise = (lm.ridge_h - lm.eave_h).clamp(1.0, 5.5);
                desc.roof_axis = lm.axis_rad;
                return (desc, !kit_pitched);
            }
            RoofForm::Shed => {
                desc.form = MASSING_SHED;
                desc.ridge_rise = (lm.ridge_h - lm.eave_h).clamp(0.9, 4.5);
                desc.roof_axis = lm.axis_rad;
                return (desc, !kit_pitched);
            }
            RoofForm::Stepped => {
                desc.form = MASSING_STEPPED;
                desc.ridge_rise = (lm.ridge_h - lm.eave_h).clamp(2.0, 14.0);
                if let Some(b0) = lm.boxes().iter().find(|b| b.is_valid() && b.kind == BOX_SETBACK).or_else(|| lm.boxes().first().filter(|b| b.is_valid())) {
                    let s = eo_data::groundscale::scale();
                    let frac = (((b0.hx as f64 * s / obb.hu.max(1.0)) + (b0.hz as f64 * s / obb.hv.max(1.0))) * 0.5).clamp(0.35, 0.78);
                    desc.pent_scale = frac as f32;
                    desc.pent_off = [
                        ((b0.dx as f64 * s) / obb.hu.max(2.0)).clamp(-0.32, 0.32) as f32,
                        ((b0.dz as f64 * s) / obb.hv.max(2.0)).clamp(-0.32, 0.32) as f32,
                    ];
                } else {
                    desc.pent_scale = 0.58;
                }
                if let Some(b1) = lm.boxes().get(1).filter(|b| b.is_valid()) {
                    desc.pent_h = (b1.y1 - b1.y0).clamp(1.4, 5.5);
                }
                return (desc, true);
            }
            RoofForm::Flat => {
                desc.form = MASSING_FLAT;
                if let Some(b0) = lm.boxes().first().filter(|b| b.is_valid()) {
                    let s = eo_data::groundscale::scale();
                    let frac = (((b0.hx as f64 * s / obb.hu.max(1.0)) + (b0.hz as f64 * s / obb.hv.max(1.0))) * 0.5).clamp(0.22, 0.55);
                    desc.pent_scale = frac as f32;
                    desc.pent_h = (b0.y1 - b0.y0).clamp(1.4, 6.0);
                    desc.pent_off = [
                        ((b0.dx as f64 * s) / obb.hu.max(2.0)).clamp(-0.3, 0.3) as f32,
                        ((b0.dz as f64 * s) / obb.hv.max(2.0)).clamp(-0.3, 0.3) as f32,
                    ];
                    return (desc, !kit_has_penthouse);
                }
                return (desc, false);
            }
        }
    }

    // 2. Preserve existing eo-kit pitched roof in districts (zero popping across cell -> district!).
    if kit_pitched {
        let rise = (kit_roof_peak - top_y).clamp(1.4, 5.5) as f32;
        let axis = obb.eu[1].atan2(obb.eu[0]) as f32;
        return (MassingDesc { form: MASSING_GABLE, ridge_rise: rise, roof_axis: axis, ..Default::default() }, false);
    }

    // 3. Tall skyscrapers & Nob Hill / FiDi high-rises (>= 35 m or `tower`): stepped architectural crown + mechanical yard.
    if (tower || height >= 35.0) && true_area >= 90.0 {
        let crown_h = if height >= 90.0 {
            (4.2 + 4.4 * h0) as f32
        } else if height >= 55.0 {
            (3.2 + 3.2 * h0) as f32
        } else {
            (2.5 + 2.5 * h0) as f32
        };
        let pent_scale = (0.30 + 0.16 * h1) as f32;
        let pent_h = (2.6 + 2.8 * h2) as f32;
        let pent_off = [((h3 - 0.5) * 0.36) as f32, ((h1 - 0.5) * 0.32) as f32];
        let form = if height >= 46.0 || h0 > 0.28 { MASSING_CROWN } else { MASSING_STEPPED };
        let desc = MassingDesc { form, ridge_rise: crown_h, roof_axis: 0.0, pent_scale, pent_h, pent_off };
        return (desc, !kit_has_penthouse || form == MASSING_CROWN);
    }

    // 4. Mid-rise commercial / civic / Nob Hill apartment blocks (16.5 m .. 35 m): stepped upper penthouse or 3D bulkhead.
    if height >= 16.5 && true_area >= 85.0 {
        if height >= 20.0 && true_area >= 165.0 && h0 < 0.68 {
            let desc = MassingDesc {
                form: MASSING_STEPPED,
                ridge_rise: (3.2 + 2.0 * h1) as f32,
                roof_axis: 0.0,
                pent_scale: (0.54 + 0.18 * h2) as f32,
                pent_h: (2.3 + 1.6 * h3) as f32,
                pent_off: [((h1 - 0.5) * 0.24) as f32, ((h2 - 0.5) * 0.24) as f32],
            };
            return (desc, !kit_has_penthouse);
        }
        let desc = MassingDesc {
            form: MASSING_FLAT,
            ridge_rise: 0.0,
            roof_axis: 0.0,
            pent_scale: (0.26 + 0.16 * h1) as f32,
            pent_h: (2.4 + 1.8 * h2) as f32,
            pent_off: [((h3 - 0.5) * 0.44) as f32, ((h0 - 0.5) * 0.44) as f32],
        };
        return (desc, !kit_has_penthouse);
    }

    // 5. Elongated waterfront pier sheds & low-rise warehouses (Embarcadero piers, SoMa sheds):
    // low-pitch gabled/hipped roofs or raised clerestory monitors instead of bare flat shoeboxes.
    let aspect = obb.hu / obb.hv.max(1.0);
    let warehouse_like = matches!(arch, "soma-warehouse" | "industrial-shed" | "retail-strip" | "");
    if warehouse_like && height >= 4.5 && height <= 15.0 && true_area >= 220.0 && obb.fill >= 0.74 && aspect >= 1.85 {
        let axis = obb.eu[1].atan2(obb.eu[0]) as f32;
        if aspect >= 2.0 && height <= 14.0 && h0 < 0.76 {
            let rise = (true_hv * (0.17 + 0.09 * h1)).clamp(1.6, 3.4) as f32;
            let desc = MassingDesc {
                form: if h2 < 0.45 { MASSING_HIP } else { MASSING_GABLE },
                ridge_rise: rise,
                roof_axis: axis,
                ..Default::default()
            };
            return (desc, true);
        }
        let desc = MassingDesc {
            form: MASSING_FLAT,
            ridge_rise: 0.0,
            roof_axis: axis,
            pent_scale: (0.34 + 0.14 * h1) as f32,
            pent_h: (2.0 + 1.2 * h2) as f32,
            pent_off: [0.0, 0.0],
        };
        return (desc, !kit_has_penthouse);
    }

    // 6. Residential hills & SF neighborhoods (4.5 .. 21.5 m): authentic SF mix of pitched
    // hipped/gabled terracotta/slate/shingle roofs + Parisian/Edwardian mansard roofs on
    // Russian Hill, North Beach, Telegraph Hill, Nob Hill, Pacific Heights, Castro & Twin Peaks.
    let hillside = base_y > 12.0;
    let steep_hill = base_y > 38.0;
    let res_pitched_arch = matches!(
        arch,
        "cottage-garage" | "victorian-row" | "stucco-row" | "edwardian-flats" | "bay-window-apartment" | "midcentury-stucco" | ""
    );
    let max_res_h = if hillside { 21.5 } else { 17.5 };
    let compact_limit = if hillside { 420.0 } else if matches!(arch, "edwardian-flats" | "bay-window-apartment" | "midcentury-stucco") { 320.0 } else { 380.0 };
    let compact_house = true_area >= 30.0 && true_area <= compact_limit && obb.fill >= 0.60 && true_hv >= 2.0 && true_hv <= 12.5;
    if res_pitched_arch && compact_house && height >= 4.5 && height <= max_res_h {
        let mut pitch_chance: f64 = match arch {
            "cottage-garage" => if hillside { 0.80 } else { 0.72 },
            "victorian-row" => if hillside { 0.78 } else { 0.70 },
            "edwardian-flats" | "bay-window-apartment" => if hillside { 0.74 } else { 0.24 },
            "stucco-row" => if hillside { 0.44 } else { 0.11 },
            "midcentury-stucco" => if hillside { 0.38 } else { 0.10 },
            _ => if hillside { 0.56 } else { 0.16 },
        };
        if steep_hill {
            pitch_chance += if arch == "stucco-row" { 0.16 } else { 0.18 };
        }
        if roof_patch_share >= 0.18 {
            pitch_chance += 0.14;
        }
        if h0 < pitch_chance.min(0.92) {
            let axis = obb.eu[1].atan2(obb.eu[0]) as f32;
            let mansard_candidate = hillside && height >= 9.5 && true_area >= 75.0
                && matches!(arch, "edwardian-flats" | "bay-window-apartment" | "victorian-row" | "stucco-row" | "");
            if mansard_candidate && (height > 16.5 || h1 < 0.34) {
                let rise = (1.7 + 1.0 * h2) as f32;
                let desc = MassingDesc { form: MASSING_MANSARD, ridge_rise: rise, roof_axis: axis, ..Default::default() };
                return (desc, true);
            }
            if height <= 17.5 {
                let hip = (h1 < 0.58 && aspect < 2.3) || (obb.fill < 0.78 && h1 < 0.72);
                let slope = if hip { 0.44 + 0.22 * h2 } else { 0.56 + 0.30 * h2 };
                let rise = if hip {
                    (true_hv * slope).clamp(1.6, 3.8) as f32
                } else {
                    (true_hv * slope).clamp(1.9, 4.4) as f32
                };
                let desc = MassingDesc {
                    form: if hip { MASSING_HIP } else { MASSING_GABLE },
                    ridge_rise: rise,
                    roof_axis: axis,
                    ..Default::default()
                };
                return (desc, true);
            }
        }
    }

    // 7. 2-5 storey flat-roof residential/mixed-use blocks (6.8..16.5 m): real 3D rooftop stair/elevator bulkhead.
    if height >= 6.8 && true_area >= 52.0 && h0 < 0.76 {
        let desc = MassingDesc {
            form: MASSING_FLAT,
            ridge_rise: 0.0,
            roof_axis: 0.0,
            pent_scale: (0.21 + 0.14 * h1) as f32,
            pent_h: (1.9 + 1.4 * h2) as f32,
            pent_off: [((h3 - 0.5) * 0.46) as f32, ((h1 - 0.5) * 0.46) as f32],
        };
        return (desc, !kit_has_penthouse);
    }

    (MassingDesc::default(), false)
}

/// One 3D projecting bay-window stack along a building's exterior wall edge (256 m cells).
/// Front face carries the wall chart's exact atlas UVs so windows, glazing, and night lights stay crisp.
#[derive(Clone, Copy, Debug)]
struct BayBox {
    edge: usize,
    u0: f64,
    u1: f64,
    y0: f64,
    y1: f64,
    proj: f64,
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
    /// Real 3D projecting bay-window stacks emitted with exact atlas UVs in 256 m cells.
    bays: Vec<BayBox>,
    wall_rgb: [f32; 3],
    roof_rgb: [f32; 3],
    textured: bool,
    /// the window rhythm (`Rhythm::floats`; zeros: none known) and whether it is a tower.
    rhythm: [f32; 5],
    tower: bool,
    /// Residential / hotel night-lighting class (`VF_RESIDENTIAL`, bit 7).
    residential: bool,
    /// the measured second colour patch and the parapet band.
    roofs: RoofCap,
    /// 3D roof/setback/penthouse massing mirrored in districts.
    massing: MassingDesc,
    /// Architectural facade material/glazing class (`MAT_*`, 0..7).
    mat_class: u8,
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
        let base_y = if plan.base_y.is_finite() { plan.base_y } else { 0.0 };
        let upper: Vec<f64> = plan.storeys.iter().filter(|s| s.index >= 1).map(|s| s.height).collect();
        let mut storey = median(if upper.is_empty() { plan.storeys.iter().map(|s| s.height).collect() } else { upper })?;
        let outside = || plan.edges.iter().filter(|e| !e.party);
        let bay = median(outside().flat_map(|e| e.bays.iter().map(|b| b.width)).filter(|w| *w > 0.3).collect())?;
        let multi = plan.storeys.len() > 1;
        let windows: Vec<&eo_kit::elevation::OpeningRecord> = outside()
            .flat_map(|e| e.openings.iter())
            .filter(|o| (!multi || o.storey >= 1) && !o.type_.contains("door") && !o.type_.contains("garage") && !o.type_.contains("entry"))
            .collect();
        let w = median(windows.iter().map(|o| o.s1 - o.s0).collect())?;
        let mut h = median(windows.iter().map(|o| o.y1 - o.y0).collect())?;
        let mut sill = base_y + median(windows.iter().map(|o| o.y0).collect())?;
        // For multi-floor downtown towers (`downtown_tower.rs`), use the exact planned
        // `floorHeight`, `spandrelBand`, and `shaftIndex` so district `hl_window_grid` stays
        // strictly in-phase with the baked cell atlas across all floors (zero cross-fade zebra!).
        if let Some(arch) = plan.extra.get("archetype") {
            if let Some(fh) = arch.get("floorHeight").and_then(|v| v.as_f64()).filter(|&x| x.is_finite() && x > 2.0) {
                storey = fh;
                let band = arch.get("spandrelBand").and_then(|v| v.as_f64()).filter(|&x| x.is_finite() && x > 0.05 && x < fh - 0.25);
                if let Some(b) = band {
                    h = (fh - b).clamp(0.15 * fh, 0.95 * fh);
                }
                if let Some(si) = arch.get("shaftIndex").and_then(|v| v.as_i64()).filter(|&i| i >= 0) {
                    if let Some(shaft) = plan.storeys.get(si as usize) {
                        // `downtown_tower.rs` places floor spandrel bands at `shaft_y0 + fh*i - 0.58*band .. + 0.42*band`.
                        let b = band.unwrap_or(0.0);
                        sill = base_y + shaft.y0 + 0.42 * b;
                    }
                }
            }
        }
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

/// rung 2 roof plant kinds of which the largest two substantial units (>= 1.5 m²) per building
/// are kept in the far mesh (sub-pixel mini-splits & thin rails stay in near kit rungs).
const FAR_PLANT_KINDS: [&str; 3] = ["roof-plant", "roof-hatch", "roof-duct"];
const FAR_PLANT_KEEP: usize = 2;
const FAR_PLANT_MIN_AREA_M2: f32 = 1.5;

/// Roof forms kept as geometry in the far mesh (rung 2 parts of these kinds).
pub fn is_roof_form(kind: &str) -> bool {
    matches!(
        kind,
        "roof-slope" | "roof-gable" | "gable" | "gable-infill" | "gable-rake" | "pediment" | "hip" | "pitch" | "chimney" | "cooling-tower" | "tower" | "crown" | "crown-fin" | "roof-penthouse" | "roof-bulkhead" | "roof-monitor" | "roof-monitor-cap" | "roof-monitor-glass"
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
    let mut patch_share = 0.0f32;
    if let Some((m, w)) = roof_measure(ctx, b) {
        roof_rgb = measured_roof(roof_rgb, wall, &m, w);
        roofs.patch = roof_patch(&m, w);
        patch_share = m.second_share;
    }
    let top = base + bld.height as f64;
    let height = (top - base).max(2.0);
    let tower = bld.height as f64 >= TOWER_MIN_M;
    let residential = classify_residential("", &bld.tags.building, &bld.tags.amenity, false, base, height, tower);
    let (massing, emit_extra) = resolve_massing(&bld.id, group, &ring, base, top, "", tower, false, top, false, patch_share);
    let mat_class = classify_mat_class("", "", "", None, tower, group);
    let mut roof = vec![];
    if emit_extra {
        if emit_massing_roof_tris(&ring, group, top, &massing, wall, roof_rgb, false, false, &mut roof) {
            roofs.pitched = true;
        }
    }
    let party = vec![false; n];
    let front_mask = vec![false; n];
    let trim_rgb = [0, 1, 2].map(|k| (wall[k] * 1.12).min(0.76));
    emit_cornice_rim(&ring, &party, &front_mask, top, height, mat_class, wall, trim_rgb, roofs.pitched, &mut roof);
    emit_water_tank(&ring, group, top, height, mat_class, &massing, &mut roof);
    enrich_flat_roof(&mut roof_rgb, &mut roofs, &ring, group);
    if !roofs.pitched {
        emit_flat_roof_detail(&ring, group, base, top, &massing, &mut roof);
    }
    Far { group, building: b, ring, party, bottom: base, top, src: vec![], roof, bays: vec![], wall_rgb: wall, roof_rgb, textured: false, rhythm: [0.0; 5], tower, residential, roofs, massing, mat_class }
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
    let glass_name = drawn.materials.get("glass").cloned().unwrap_or_default();
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
    let mut kit_roof_peak = f64::MIN;
    let mut kit_has_penthouse = false;
    let mut sink = eo_kit::mesh::Sink::default();
    // the measured roof colour exactly on the roof slots (the near tint
    // reaches it only through the finish's nominal colour), the largest plant kept.
    let measured = roof_measure(ctx, b);
    let measured_rgb = measured.map(|(m, w)| (m, w, measured_roof(colour_of(&wall).0, colour_of(&wall).0, &m, 1.0)));
    let plant_on = eo_data::roof_colours::feature_on("plant");
    let mut plant: Vec<(f32, Vec<([[f32; 3]; 3], [f32; 3], [f32; 3])>)> = vec![];
    let mut raw_bays: Vec<(Option<usize>, Vec<[f32; 3]>)> = vec![];
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
        if part.kind == "bay" && !sink.positions.is_empty() {
            raw_bays.push((part.edge, sink.positions.iter().map(|q| [q[0] as f32, q[1] as f32, q[2] as f32]).collect()));
        }
        let form = is_roof_form(&part.kind);
        // a roof-slot form takes the measured colour (blended by confidence).
        if let Some((_, w, target)) = measured_rgb {
            if form && !glass && eo_kit::roof_measured::ROOF_SLOTS.contains(&part.slot.as_str()) {
                rgb = [0, 1, 2].map(|k| rgb[k] + (target[k] - rgb[k]) * w);
            }
        }
        if matches!(part.kind.as_str(), "roof-slope" | "roof-gable" | "gable" | "hip" | "pitch") {
            pitched = true;
            kit_roof_peak = kit_roof_peak.max(part.y1);
        }
        if matches!(part.kind.as_str(), "roof-penthouse" | "roof-bulkhead" | "cooling-tower" | "tower" | "crown") {
            kit_has_penthouse = true;
        }
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
            // Skip strictly downward-facing flat undersides (n.y < -0.85) of roof boxes/slabs resting on the cap.
            if form {
                if n[1] >= -0.85 {
                    roof.push((p, n, rgb));
                }
            } else if kept_plant {
                if n[1] > 0.7 {
                    let a = glam::Vec3::from(p[0]);
                    plant_area += 0.5 * (glam::Vec3::from(p[1]) - a).cross(glam::Vec3::from(p[2]) - a).length();
                }
                if n[1] >= -0.85 {
                    plant_tris.push((p, n, rgb));
                }
            }
        }
        if !plant_tris.is_empty() && plant_area >= FAR_PLANT_MIN_AREA_M2 {
            plant.push((plant_area, plant_tris));
        }
    }
    // the largest roof plant (by plan area) stays as boxes in the far mesh.
    plant.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let kept_plant_count = plant.len().min(FAR_PLANT_KEEP);
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
    // Prefer `frame.edges` — the exact regularized wall outline that eo-kit renders in Rung 0 & Rung 2 —
    // so HLOD cells align to 0.00 m against Kit Rung 2 during cross-fade (no overlap stipple) and
    // preserve `e.party` flags to save atlas pages & cell vertices.
    let (ring, party, front_mask): (Vec<[f64; 2]>, Vec<bool>, Vec<bool>) = if frame.edges.len() >= 3 {
        (
            frame.edges.iter().map(|e| [e.start.x, e.start.z]).collect(),
            frame.edges.iter().map(|e| e.party).collect(),
            frame.edges.iter().map(|e| matches!(e.role, "front" | "front-secondary")).collect(),
        )
    } else if let Some(p) = &frame.source_polygon {
        let r = open_ring(p.iter().map(|q| [q.x, q.z]).collect());
        let n = r.len();
        (r, vec![false; n], vec![false; n])
    } else {
        return None;
    };
    if ring.len() < 3 {
        return None;
    }
    let (wall_rgb, _) = colour_of(&wall);
    let trim_name = drawn
        .materials
        .get("trim")
        .or_else(|| drawn.materials.get("cornice"))
        .or_else(|| drawn.materials.get("coping"))
        .cloned()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| wall.clone());
    let (trim_rgb, _) = colour_of(&trim_name);
    let roof_name = drawn.materials.get("roof").cloned().filter(|s| !s.is_empty()).unwrap_or_else(|| wall.clone());
    let (mut roof_rgb, _) = colour_of(&roof_name);
    let mut roofs = RoofCap { pitched, ..Default::default() };
    let mut patch_share = 0.0f32;
    if let Some((m, w, _)) = measured_rgb {
        roof_rgb = measured_roof(roof_rgb, wall_rgb, &m, w);
        roofs.patch = roof_patch(&m, w);
        patch_share = m.second_share;
    }
    // the window rhythm, tower flag, material class, and residential night-lighting class.
    let rhythm = Rhythm::of_plan(&planned.plan).map(|r| r.floats()).unwrap_or([0.0; 5]);
    let arch = planned.classified.archetype;
    let height = (top - base_y).max(2.0);
    let tower = arch == "downtown-tower" || (arch == "civic-concrete" && height >= TOWER_MIN_M);
    let residential = classify_residential(
        arch,
        &planned.classified.tag,
        &planned.classified.amenity,
        planned.classified.has_retail_ground,
        base_y,
        height,
        tower,
    );
    let system = planned.plan.extra.get("archetype").and_then(|a| a.get("system")).and_then(|v| v.as_str());
    let mat_class = classify_mat_class(arch, &wall, &glass_name, system, tower, group);
    let peak = if kit_roof_peak > top { kit_roof_peak } else { top + 2.0 };
    let (massing, emit_extra) = resolve_massing(&rec.key, group, &ring, base_y, top, arch, tower, pitched, peak, kit_has_penthouse, patch_share);
    if emit_extra {
        if emit_massing_roof_tris(&ring, group, top, &massing, wall_rgb, roof_rgb, pitched, kit_has_penthouse && massing.form != MASSING_CROWN, &mut roof) {
            roofs.pitched = true;
        }
    }
    // Projecting 3D cornice overhang rims on prominent street-facing flat-roofed elevations & SF redwood water tanks.
    emit_cornice_rim(&ring, &party, &front_mask, top, height, mat_class, wall_rgb, trim_rgb, roofs.pitched, &mut roof);
    emit_water_tank(&ring, group, top, height, mat_class, &massing, &mut roof);
    // Extract real 3D bay-window stacks from eo-kit Rung 2 `"bay"` geometry (0.00 m parity with Kit R2!)
    // + subtle architectural fallback bays on prominent hillside Victorian/Edwardian/apartment frontages.
    let eframes = edge_frames(&ring);
    let mut bays: Vec<BayBox> = vec![];
    for (edge_hint, pts) in &raw_bays {
        if pts.is_empty() {
            continue;
        }
        let mut cx = 0.0f64;
        let mut cz = 0.0f64;
        for q in pts {
            cx += q[0] as f64;
            cz += q[2] as f64;
        }
        cx /= pts.len() as f64;
        cz /= pts.len() as f64;
        let edge_idx = edge_hint.filter(|&e| e < ring.len() && !party[e] && front_mask[e]).or_else(|| {
            let mut best: Option<(f64, usize)> = None;
            for (e, &(u, n, len)) in eframes.iter().enumerate() {
                if party[e] || !front_mask[e] || len < 3.0 {
                    continue;
                }
                let (rx, rz) = (cx - ring[e][0], cz - ring[e][1]);
                let uu = rx * u[0] + rz * u[1];
                let dd = rx * n[0] + rz * n[1];
                if uu >= -0.5 && uu <= len + 0.5 && (-0.25..=1.8).contains(&dd) {
                    let score = dd.abs();
                    if best.is_none_or(|(bs, _)| score < bs) {
                        best = Some((score, e));
                    }
                }
            }
            best.map(|(_, e)| e)
        });
        let Some(e) = edge_idx else { continue };
        let (u, n, len) = eframes[e];
        let a = ring[e];
        let (mut u0, mut u1, mut y0, mut y1, mut dmax) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN, f64::MIN);
        for q in pts {
            let (rx, rz) = (q[0] as f64 - a[0], q[2] as f64 - a[1]);
            let uu = rx * u[0] + rz * u[1];
            let dd = rx * n[0] + rz * n[1];
            u0 = u0.min(uu);
            u1 = u1.max(uu);
            y0 = y0.min(q[1] as f64);
            y1 = y1.max(q[1] as f64);
            dmax = dmax.max(dd);
        }
        let u0 = u0.max(0.15);
        let u1 = u1.min(len - 0.15);
        let y0 = y0.max(bottom + 0.2);
        let y1 = y1.min(top - 0.1);
        if dmax >= 0.42 && u1 - u0 >= 1.6 && y1 - y0 >= 3.4 {
            let cand = BayBox { edge: e, u0, u1, y0, y1, proj: dmax.clamp(0.45, 0.90) };
            // Merge vertically stacked bays along the same horizontal window column.
            if let Some(ex) = bays.iter_mut().find(|b| {
                b.edge == cand.edge && {
                    let ov = b.u1.min(cand.u1) - b.u0.max(cand.u0);
                    let mw = (b.u1 - b.u0).min(cand.u1 - cand.u0);
                    ov >= 0.45 * mw
                }
            }) {
                ex.u0 = ex.u0.min(cand.u0);
                ex.u1 = ex.u1.max(cand.u1);
                ex.y0 = ex.y0.min(cand.y0);
                ex.y1 = ex.y1.max(cand.y1);
                ex.proj = ex.proj.max(cand.proj);
            } else {
                bays.push(cand);
            }
        }
    }
    if bays.is_empty()
        && matches!(arch, "victorian-row" | "edwardian-flats" | "bay-window-apartment")
        && height >= 9.5
        && (base_y > 16.0 || height >= 12.5)
    {
        if let Some((e, &(_, _, len))) = eframes
            .iter()
            .enumerate()
            .filter(|&(e, &(_, _, len))| !party[e] && front_mask[e] && len >= 7.0)
            .max_by(|a, b| a.1.2.total_cmp(&b.1.2))
        {
            let by0 = (base_y + 3.2).min(top - 3.4);
            let by1 = (top - 0.55).max(by0 + 3.0);
            if by1 > by0 + 3.0 {
                if len >= 14.0 {
                    let bw = (len * 0.22).clamp(2.4, 3.2);
                    for frac in [0.28, 0.72] {
                        let c = len * frac;
                        bays.push(BayBox { edge: e, u0: (c - 0.5 * bw).max(0.4), u1: (c + 0.5 * bw).min(len - 0.4), y0: by0, y1: by1, proj: 0.50 });
                    }
                } else {
                    let bw = (len * 0.34).clamp(2.2, 3.0);
                    let c = len * 0.5;
                    bays.push(BayBox { edge: e, u0: (c - 0.5 * bw).max(0.4), u1: (c + 0.5 * bw).min(len - 0.4), y0: by0, y1: by1, proj: 0.50 });
                }
            }
        }
    }
    let max_bays = if height >= 13.0 { 2 } else { 1 };
    if bays.len() > max_bays {
        bays.sort_by(|a, b| ((b.u1 - b.u0) * (b.y1 - b.y0)).total_cmp(&((a.u1 - a.u0) * (a.y1 - a.y0))));
        bays.truncate(max_bays);
    }
    // Emit welded bay-hood top caps (& bottom soffits on tall cantilevers) into `roof`.
    let bay_top_rgb = [0, 1, 2].map(|k| (trim_rgb[k] * 0.70 + wall_rgb[k] * 0.30).clamp(0.06, 0.80));
    let bay_bot_rgb = mul3(bay_top_rgb, [0.44; 3]);
    for bay in &bays {
        let (u, n, _) = eframes[bay.edge];
        let a = ring[bay.edge];
        let pt = |s: f64, d: f64, y: f64| [(a[0] + u[0] * s + n[0] * d) as f32, y as f32, (a[1] + u[1] * s + n[1] * d) as f32];
        let t0 = pt(bay.u0, 0.0, bay.y1);
        let t1 = pt(bay.u1, 0.0, bay.y1);
        let t2 = pt(bay.u1, bay.proj, bay.y1);
        let t3 = pt(bay.u0, bay.proj, bay.y1);
        roof.push(([t0, t1, t2], [0.0, 1.0, 0.0], bay_top_rgb));
        roof.push(([t0, t2, t3], [0.0, 1.0, 0.0], bay_top_rgb));
        if bay.y0 > bottom + 2.4 && bay.proj >= 0.52 {
            let b0 = pt(bay.u0, 0.0, bay.y0);
            let b1 = pt(bay.u1, 0.0, bay.y0);
            let b2 = pt(bay.u1, bay.proj, bay.y0);
            let b3 = pt(bay.u0, bay.proj, bay.y0);
            roof.push(([b1, b0, b3], [0.0, -1.0, 0.0], bay_bot_rgb));
            roof.push(([b1, b3, b2], [0.0, -1.0, 0.0], bay_bot_rgb));
        }
    }
    enrich_flat_roof(&mut roof_rgb, &mut roofs, &ring, group);
    if !roofs.pitched && !kit_has_penthouse && kept_plant_count == 0 {
        emit_flat_roof_detail(&ring, group, base_y, top, &massing, &mut roof);
    }
    Some(Far { group, building: b, ring, party, bottom, top, src, roof, bays, wall_rgb, roof_rgb, textured: true, rhythm, tower, residential, roofs, massing, mat_class })
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
    let mut patch_share = 0.0f32;
    if let Some((m, w)) = roof_measure(ctx, b) {
        roof_rgb = measured_roof(roof_rgb, wall_rgb, &m, w); // the cap of a hand-built building
        roofs.patch = roof_patch(&m, w);
        patch_share = m.second_share;
    }
    let top = base + bld.height as f64;
    let height = (top - base).max(2.0);
    let tower = bld.height as f64 >= TOWER_MIN_M * 2.0;
    let residential = classify_residential("", &bld.tags.building, &bld.tags.amenity, false, base, height, tower);
    let (massing, _) = resolve_massing(&bld.id, group, &ring, base, top, "", tower, false, top, true, patch_share);
    let mat_class = classify_mat_class("", "", "", None, tower, group);
    Some(Far { group, building: b, ring, party: vec![false; n], bottom: base, top, src, roof: vec![], bays: vec![], wall_rgb, roof_rgb, textured: true, rhythm: [0.0; 5], tower, residential, roofs, massing, mat_class })
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
        // Adaptive facade atlas budget: give tall skyline towers finer texels for crisp
        // mullions/spandrels/mechanical bands, and tiny low sheds coarser texels.
        let height_scale = if f.tower || height >= 30.0 {
            0.78
        } else if height < 5.0 {
            1.5
        } else {
            1.0
        };
        for (e, &(_, _, len)) in frames.iter().enumerate() {
            if len < 0.05 {
                continue;
            }
            let t = if f.party.get(e).copied().unwrap_or(false) { texel * party_scale } else { texel * height_scale };
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

/// Best-fit shelf packing (tallest first, reuses earlier shelves with room). Returns the page count.
fn pack(charts: &mut [Chart]) -> usize {
    struct Shelf {
        page: usize,
        y: usize,
        h: usize,
        x: usize,
    }
    let mut order: Vec<usize> = (0..charts.len()).collect();
    order.sort_by(|&a, &b| (charts[b].h, charts[b].w).cmp(&(charts[a].h, charts[a].w)));
    let mut shelves: Vec<Shelf> = vec![];
    let mut page_y: Vec<usize> = vec![0];
    let mut used = false;
    for k in order {
        let (w, h) = (charts[k].w + 2 * PAD, charts[k].h + 2 * PAD);
        // Prefer an existing shelf whose height matches closely and has horizontal room.
        let mut best_shelf: Option<(usize, usize)> = None;
        for (si, sh) in shelves.iter().enumerate() {
            if h <= sh.h && sh.x + w <= PAGE {
                let waste = sh.h - h;
                if best_shelf.is_none_or(|(bw, _)| waste < bw) {
                    best_shelf = Some((waste, si));
                    if waste == 0 {
                        break;
                    }
                }
            }
        }
        if let Some((_, si)) = best_shelf {
            let sh = &mut shelves[si];
            charts[k].page = sh.page;
            charts[k].x = sh.x;
            charts[k].y = sh.y;
            sh.x += w;
            used = true;
            continue;
        }
        // Open a new shelf on the earliest page with enough vertical space.
        let target_page = page_y.iter().position(|&py| py + h <= PAGE).unwrap_or_else(|| {
            page_y.push(0);
            page_y.len() - 1
        });
        let y = page_y[target_page];
        page_y[target_page] += h;
        charts[k].page = target_page;
        charts[k].x = 0;
        charts[k].y = y;
        shelves.push(Shelf { page: target_page, y, h, x: w });
        used = true;
    }
    if used { page_y.len() } else { 0 }
}

fn recess_shade(d: f64) -> f32 {
    if d < -0.04 {
        // Deeper mullion/reveal contrast so recessed windows stay crisp through BC3 + mips.
        1.0 - 0.56 * ((-d - 0.04) / 0.45).clamp(0.0, 1.0) as f32
    } else if d > 0.05 {
        // Subtle relief lift on projecting spandrel bands, cornices, and pilasters.
        1.0 + 0.06 * ((d - 0.05) / 0.25).clamp(0.0, 1.0) as f32
    } else {
        1.0
    }
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
const PARAPET_SHADE: f32 = 0.48;
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

pub(crate) fn pack_normal(n: [f32; 3], page: u32) -> u32 {
    eo_assets::glb::pack_normal(glam::Vec3::from(n)) | ((page & 255) << 24)
}

pub(crate) fn srgb_word(c: [f32; 3], flags: u32) -> u32 {
    linear_to_srgb_byte(c[0]) as u32 | ((linear_to_srgb_byte(c[1]) as u32) << 8) | ((linear_to_srgb_byte(c[2]) as u32) << 16) | ((flags & 255) << 24)
}

fn uv_word(u: f64, v: f64) -> u32 {
    let q = |x: f64| ((x / PAGE as f64).clamp(0.0, 1.0) * 65535.0).round() as u32;
    q(u) | (q(v) << 16)
}

/// Bake one cell (no cache).
pub fn bake_cell(ctx: &BakeContext, cell: u32) -> BakedCell {
    // Ensure LiDAR building massing is lazily installed if external/lidar/ exists.
    eo_data::lidar_buildings::install(&ctx.repo_root, ctx.cache_dir.as_deref());
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
        let mat_bits = ((f.mat_class as u32) & 7) << VF_MAT_SHIFT;
        let res_flag = if f.residential { VF_RESIDENTIAL } else { 0 };
        let wall_word = srgb_word(f.wall_rgb, if f.textured { VF_TEXTURED } else { 0 } | tower_flag | mat_bits | res_flag);
        let at = |e: usize, s: f64, y: f64| -> [f32; 3] {
            let a = f.ring[e];
            let u = frames[e].0;
            [(a[0] + u[0] * s) as f32, y as f32, (a[1] + u[1] * s) as f32]
        };
        if f.textured {
            for &k in &by_far[fi] {
                let ch = &charts[k];
                let (u, n, _) = frames[ch.edge];
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
                // Real 3D projecting bay-window stacks mapped directly to the chart's atlas UVs
                // (crisp windows + grazing side-return sunlight/shadow contrast, 0.00 m Kit R2 parity).
                let du = (ch.u1 - ch.u0).max(1e-6);
                let dv = (ch.v1 - ch.v0).max(1e-6);
                for bay in &f.bays {
                    if bay.edge != ch.edge {
                        continue;
                    }
                    let bu0 = bay.u0.max(ch.u0);
                    let bu1 = bay.u1.min(ch.u1);
                    let bv0 = bay.y0.max(ch.v0);
                    let bv1 = bay.y1.min(ch.v1);
                    if bu1 - bu0 < 0.9 || bv1 - bv0 < 1.8 {
                        continue;
                    }
                    let bx0 = x0 + (bu0 - ch.u0) / du * (x1 - x0);
                    let bx1 = x0 + (bu1 - ch.u0) / du * (x1 - x0);
                    let by0 = y0 + (ch.v1 - bv1) / dv * (y1 - y0);
                    let by1 = y0 + (ch.v1 - bv0) / dv * (y1 - y0);
                    let a = f.ring[ch.edge];
                    let pt = |s: f64, d: f64, y: f64| [(a[0] + u[0] * s + n[0] * d) as f32, y as f32, (a[1] + u[1] * s + n[1] * d) as f32];
                    // Front projecting face with exact baked window atlas UVs.
                    quad(
                        &mut vertices,
                        &mut indices,
                        [
                            v(pt(bu0, bay.proj, bv1), uv_word(bx0, by0)),
                            v(pt(bu1, bay.proj, bv1), uv_word(bx1, by0)),
                            v(pt(bu1, bay.proj, bv0), uv_word(bx1, by1)),
                            v(pt(bu0, bay.proj, bv0), uv_word(bx0, by1)),
                        ],
                    );
                    // Left & right return faces (1-texel vertical edge strip so floor tone matches cleanly without stretching windows).
                    let nl = pack_normal([-u[0] as f32, 0.0, -u[1] as f32], ch.page as u32);
                    let nr = pack_normal([u[0] as f32, 0.0, u[1] as f32], ch.page as u32);
                    let vl = |p: [f32; 3], uv: u32| HlodVertex { pos: p, normal: nl, uv, color: wall_word, group: f.group };
                    let vr = |p: [f32; 3], uv: u32| HlodVertex { pos: p, normal: nr, uv, color: wall_word, group: f.group };
                    let sl_u = (bx0 + 0.5).min(x1 - 0.5);
                    let sr_u = (bx1 - 0.5).max(x0 + 0.5);
                    quad(
                        &mut vertices,
                        &mut indices,
                        [
                            vl(pt(bu0, 0.0, bv1), uv_word(sl_u, by0)),
                            vl(pt(bu0, bay.proj, bv1), uv_word(sl_u, by0)),
                            vl(pt(bu0, bay.proj, bv0), uv_word(sl_u, by1)),
                            vl(pt(bu0, 0.0, bv0), uv_word(sl_u, by1)),
                        ],
                    );
                    quad(
                        &mut vertices,
                        &mut indices,
                        [
                            vr(pt(bu1, bay.proj, bv1), uv_word(sr_u, by0)),
                            vr(pt(bu1, 0.0, bv1), uv_word(sr_u, by0)),
                            vr(pt(bu1, 0.0, bv0), uv_word(sr_u, by1)),
                            vr(pt(bu1, bay.proj, bv0), uv_word(sr_u, by1)),
                        ],
                    );
                }
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
        // Roof forms: weld shared vertices per building with `normal = 0` so `hlod_draw.wgsl`
        // computes exact flat per-triangle face normals via `cross(dpdx, dpdy)`, slashing cell
        // roof vertex counts ~2.5-3x (freeing >50% of the 4M GPU vertex arena for resident cells!).
        if !f.roof.is_empty() {
            let mut weld: HashMap<([u32; 3], u32), u32> = HashMap::with_capacity(f.roof.len());
            for (p, _, rgb) in &f.roof {
                let w = srgb_word(*rgb, VF_ROOF);
                for q in p {
                    let key = ([q[0].to_bits(), q[1].to_bits(), q[2].to_bits()], w);
                    let idx = *weld.entry(key).or_insert_with(|| {
                        let s = vertices.len() as u32;
                        vertices.push(HlodVertex { pos: *q, normal: 0, uv: 0, color: w, group: f.group });
                        s
                    });
                    indices.push(idx);
                }
            }
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
            massing_form: f.massing.form,
            ridge_rise: f.massing.ridge_rise,
            roof_axis: f.massing.roof_axis,
            pent_scale: f.massing.pent_scale,
            pent_h: f.massing.pent_h,
            pent_off: f.massing.pent_off,
            mat_class: f.mat_class as f32,
            residential: if f.residential { 1.0 } else { 0.0 },
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
            summaries: vec![BuildingSummary {
                group: 5,
                building: 11,
                facade: [0.1, 0.2, 0.3],
                glass: 0.25,
                roof: [0.4, 0.4, 0.4],
                base: 3.0,
                top: 12.0,
                wall: [0.2, 0.25, 0.3],
                rhythm: [3.1, 3.2, 0.4, 0.5, 0.25],
                tower: 1.0,
                massing_form: MASSING_CROWN,
                ridge_rise: 4.5,
                roof_axis: 0.25,
                pent_scale: 0.35,
                pent_h: 3.2,
                pent_off: [0.1, -0.05],
                mat_class: MAT_CURTAIN_BLUE as f32,
                residential: 1.0,
            }],
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
            bays: vec![],
            wall_rgb: [0.6; 3],
            roof_rgb: [0.3; 3],
            textured: false,
            rhythm: [0.0; 5],
            tower: false,
            residential: false,
            roofs: RoofCap { patch, pitched },
            massing: MassingDesc::default(),
            mat_class: MAT_DEFAULT,
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
        let far = Far { group: 1, building: 0, ring, party: vec![false; 4], bottom: 0.0, top: 6.0, src, roof: vec![], bays: vec![], wall_rgb: wall, roof_rgb: wall, textured: true, rhythm: [0.0; 5], tower: false, residential: false, roofs: RoofCap::default(), massing: MassingDesc::default(), mat_class: MAT_DEFAULT };
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
        // Tower plan with explicit `floorHeight`, `spandrelBand`, `shaftIndex`, and non-zero `base_y`
        // keeps exact world-Y floor period & phase parity between cell atlas and district `hl_window_grid`.
        let mut plan = eo_kit::elevation::Plan::empty();
        plan.base_y = 2.33;
        plan.storeys = vec![
            eo_kit::elevation::Storey { index: 0, height: 6.0, y0: 0.0, y1: 6.0, use_: "retail".into(), extra: serde_json::Value::Null },
            eo_kit::elevation::Storey { index: 1, height: 56.42, y0: 6.0, y1: 62.42, use_: "upper".into(), extra: serde_json::Value::Null },
        ];
        plan.edges = vec![eo_kit::elevation::EdgeRecord {
            index: 0,
            role: "front",
            party: false,
            length: 30.0,
            bays: vec![eo_kit::elevation::BayRecord { index: 0, s0: 0.0, s1: 4.5, width: 4.5 }],
            openings: vec![eo_kit::elevation::OpeningRecord {
                id: "w1".into(),
                storey: 1,
                bay: 0,
                type_: "curtain".into(),
                s0: 0.1,
                s1: 4.4,
                y0: 6.0,
                y1: 62.42,
            }],
        }];
        plan.extra.insert(
            "archetype".into(),
            serde_json::json!({ "floorHeight": 4.03, "spandrelBand": 1.05, "shaftIndex": 1 }),
        );
        let tp = Rhythm::of_plan(&plan).unwrap();
        assert!((tp.storey_m - 4.03).abs() < 1e-4, "keeps exact floorHeight: {tp:?}");
        assert!((tp.window_h - (4.03 - 1.05) / 4.03).abs() < 1e-3, "matches spandrelBand duty: {tp:?}");
        let want_phase = ((2.33_f64 + 6.0 + 0.42 * 1.05) / 4.03).rem_euclid(1.0) as f32;
        assert!((tp.phase - want_phase).abs() < 1e-4, "phase includes base_y + shaft_y0 + 0.42*band: got {}, want {want_phase}", tp.phase);
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
        let far = Far { group: 1, building: 0, ring, party: vec![false; 4], bottom: 0.0, top: 6.0, src, roof: vec![], bays: vec![], wall_rgb: wall, roof_rgb: wall, textured: true, rhythm: [0.0; 5], tower: false, residential: false, roofs: RoofCap::default(), massing: MassingDesc::default(), mat_class: MAT_DEFAULT };
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

    #[test]
    fn massing_roof_emits_watertight_gables_hips_and_tower_crowns() {
        let rect = vec![[0.0, 0.0], [16.0, 0.0], [16.0, 10.0], [0.0, 10.0]];
        let obb = fit_obb(&rect, None).unwrap();
        assert!((obb.hu - 8.0).abs() < 1e-6 && (obb.hv - 5.0).abs() < 1e-6 && (obb.fill - 1.0).abs() < 1e-6);
        let mut tris = vec![];
        let gable = MassingDesc { form: MASSING_GABLE, ridge_rise: 3.0, roof_axis: 0.0, ..Default::default() };
        emit_massing_roof_tris(&rect, 7, 10.0, &gable, [0.5; 3], [0.3; 3], false, false, &mut tris);
        assert_eq!(tris.len(), 6, "gable has 2 slope quads (4 tris) + 2 pediment ends (2 tris)");
        assert!(tris.iter().any(|(p, _, _)| p.iter().any(|q| (q[1] - 13.0).abs() < 1e-4)), "ridge reaches top + rise");

        let mut hip_tris = vec![];
        let hip = MassingDesc { form: MASSING_HIP, ridge_rise: 2.5, roof_axis: 0.0, ..Default::default() };
        emit_massing_roof_tris(&rect, 11, 10.0, &hip, [0.5; 3], [0.3; 3], false, false, &mut hip_tris);
        assert_eq!(hip_tris.len(), 6, "hip roof has 6 triangles");

        // L-shaped notched SF row house (`fill = 0.75`): still gets an inscribed pitched roof with zero floating corners.
        let l_ring = vec![[0.0, 0.0], [16.0, 0.0], [16.0, 5.0], [8.0, 5.0], [8.0, 10.0], [0.0, 10.0]];
        let l_obb = fit_obb(&l_ring, None).unwrap();
        assert!(l_obb.fill > 0.70 && l_obb.fill < 0.78);
        let mut l_tris = vec![];
        emit_massing_roof_tris(&l_ring, 19, 10.0, &hip, [0.5; 3], [0.3; 3], false, false, &mut l_tris);
        assert_eq!(l_tris.len(), 6, "notched row house fits inscribed hip roof");

        let mut mansard_tris = vec![];
        let mansard = MassingDesc { form: MASSING_MANSARD, ridge_rise: 2.2, roof_axis: 0.0, ..Default::default() };
        emit_massing_roof_tris(&rect, 17, 14.0, &mansard, [0.5; 3], [0.3; 3], false, false, &mut mansard_tris);
        assert_eq!(mansard_tris.len(), 10, "mansard has 4 sloped skirt quads (8 tris) + flat tar deck (2 tris)");
        assert!(mansard_tris.iter().any(|(p, _, _)| p.iter().any(|q| (q[1] - 16.2).abs() < 1e-4)), "mansard crown reaches top + rise");

        let tower_ring = vec![[0.0, 0.0], [32.0, 0.0], [32.0, 24.0], [0.0, 24.0]];
        let mut crown_tris = vec![];
        let crown = MassingDesc { form: MASSING_CROWN, ridge_rise: 5.0, roof_axis: 0.0, pent_scale: 0.35, pent_h: 3.5, pent_off: [0.1, 0.0] };
        emit_massing_roof_tris(&tower_ring, 23, 100.0, &crown, [0.4; 3], [0.2; 3], false, false, &mut crown_tris);
        assert!(crown_tris.len() >= 20, "two-tier tower crown + penthouse emits >=20 tris, got {}", crown_tris.len());
        let peak_y = crown_tris.iter().flat_map(|(p, _, _)| p.iter().map(|q| q[1])).fold(0.0f32, f32::max);
        assert!((peak_y - 108.5).abs() < 0.05, "crown + penthouse top = {peak_y}");

        // Projecting 3D cornice overhang rims emit on prominent street-facing flat roofs.
        let mut cornice_tris = vec![];
        emit_cornice_rim(&rect, &[false, true, false, true], &[true, false, false, false], 12.0, 12.0, MAT_BRICK, [0.5; 3], [0.7; 3], false, &mut cornice_tris);
        assert!(!cornice_tris.is_empty(), "cornice rim emits projecting overhang");
        assert!(cornice_tris.iter().any(|(p, _, _)| p.iter().any(|q| q[2] < -0.35)), "cornice overhangs outward");

        // Water tank emits hexagonal redwood drum + conical cap on qualifying prewar midrises.
        let mut tank_tris = vec![];
        let big_rect = vec![[0.0, 0.0], [28.0, 0.0], [28.0, 22.0], [0.0, 22.0]];
        let tank_grp = (1u32..500).find(|&g| wants_water_tank(g, 24.0, 350.0, MAT_BRICK, 0)).expect("some brick midrise gets a water tank");
        emit_water_tank(&big_rect, tank_grp, 24.0, 24.0, MAT_BRICK, &MassingDesc::default(), &mut tank_tris);
        assert_eq!(tank_tris.len(), 30, "hexagonal water tank: 12 leg tris + 12 barrel tris + 6 cone cap tris");

        // Residential vs commercial night classification.
        assert!(classify_residential("victorian-row", "", "", false, 40.0, 11.0, false));
        assert!(classify_residential("downtown-tower", "apartments", "", false, 10.0, 90.0, true));
        assert!(!classify_residential("downtown-tower", "office", "", false, 5.0, 140.0, true));
    }
}
