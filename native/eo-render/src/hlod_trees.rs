//! stage 4, distant trees. The DataSF street trees (the props slice's
//! survey: position, form, crown and height) and the park trees (the
//! landmarks2 ground cover's own lattice, `GroundCover::park_trees`, over the
//! whole city) as camera-facing cards with an ellipsoid crown
//! (hlod_trees.wgsl), drawn from where the street props' trees stop
//! (`NEAR_M`, faded in by screen-door dither to `FULL_M`) to the far plane.
//! All resident (one instance buffer, ordered by 128 m block): the GPU culls
//! the blocks (frustum, near distance, the occlusion pyramid; hlod_trees_cull.wgsl)
//! and the vertex shader drops the trees nearer than `NEAR_M` or behind the camera.

use bytemuck::{Pod, Zeroable};
use crate::cove_canopy;

pub const NEAR_M: f32 = 270.0;
pub const FULL_M: f32 = 330.0;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
pub struct FarTree {
    /// Trunk base, world metres.
    pub pos: [f32; 3],
    pub height: f32,
    /// Crown half width.
    pub crown: f32,
    /// sRGB crown tint.
    pub tint: u32,
    pub seed: u32,
    /// The card's form and the baked canopy's model, packed: the form in the
    /// `SPECIES_FORM_MASK` bits (0 the round park crown; 1 + `vegetation::Kind::Tree`
    /// species: 1 cypress, 2 pine, 3 blue gum, `--no-far-species`: never),
    /// `SPECIES_BAKED` when the tree draws the baked Black Point canopy
    /// (`FarTreeList::bake_black_point_canopy`), its one-based model in
    /// `cove_canopy::SPECIES` from `SPECIES_MODEL_SHIFT` up (0 none). hlod_trees.wgsl
    /// reads the same layout (`draw_shader_source` passes the constants).
    pub species: u32,
}

/// `FarTree::species`: the card's form's bits, the baked canopy's flag, the
/// first bit of the canopy model (hlod_trees.wgsl `FT_FORM_MASK`, `FT_BAKED`,
/// `FT_MODEL_SHIFT`).
pub const SPECIES_FORM_MASK: u32 = 15;
pub const SPECIES_BAKED: u32 = 16;
pub const SPECIES_MODEL_SHIFT: u32 = 8;

fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | ((g as u32) << 8) | ((b as u32) << 16) | (255 << 24)
}

fn hash(a: u32) -> u32 {
    let mut x = a.wrapping_mul(0x9e37_79b9) ^ 0x85eb_ca6b;
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^ (x >> 15)
}

/// The street trees of the survey on the ground `ground` gives.
pub fn street_trees(js: &eo_props::JsProps, ground: &dyn Fn(f64, f64) -> f64) -> Vec<FarTree> {
    let t = &js.trees;
    let n = if t.stride > 0 { t.data.len() / t.stride } else { 0 };
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let o = i * t.stride;
        let (x, z) = (t.data[o] as f64 / 10.0, t.data[o + 1] as f64 / 10.0);
        let form = t.forms.get(t.data[o + 2] as usize).map(|s| s.as_str()).unwrap_or("tall");
        let crown = if t.stride > 3 && t.data[o + 3] > 0 { t.data[o + 3] as f32 / 10.0 } else { 0.0 };
        let height = if t.stride > 4 && t.data[o + 4] > 0 { t.data[o + 4] as f32 / 10.0 } else { 0.0 };
        let (dh, dc) = match form {
            "palm" => (11.0, 2.2),
            "columnar" => (9.0, 1.4),
            "small" => (5.0, 1.8),
            _ => (8.5, 2.8),
        };
        let height = if height > 1.0 { height } else { dh };
        let crown = if crown > 0.5 { (crown * 0.5).min(height * 0.6) } else { dc };
        let h = hash(i as u32 ^ 0x51ed);
        let g = 70 + (h % 30) as u8;
        let tint = if form == "palm" { rgb(78, 96, 52) } else { rgb(52 + (h >> 8) as u8 % 16, g + 20, 40 + (h >> 16) as u8 % 14) };
        out.push(FarTree { pos: [x as f32, ground(x, z) as f32, z as f32], height, crown, tint, seed: h, species: (match form { "pollard" => 4, "mature" => 5, "round" => 6, _ => 0 }) << SPECIES_MODEL_SHIFT });
    }
    out
}

fn park_tree(t: eo_landmarks::ground_cover::ParkTree) -> FarTree {
    let (h, cr, tint) = match t.species {
        "park-pine" => (17.0, 3.6, rgb(44, 66, 44)),
        "park-oak" => (11.0, 5.0, rgb(62, 82, 46)),
        _ => (13.0, 4.2, rgb(70, 92, 50)),
    };
    let k = t.scale.clamp(0.4, 2.0) as f32;
    let seed = hash((t.x * 10.0) as i64 as u32 ^ ((t.z * 10.0) as i64 as u32).rotate_left(16));
    FarTree { pos: [t.x as f32, t.y as f32, t.z as f32], height: h * k, crown: cr * k, tint, seed, species: park_model(t.species) << SPECIES_MODEL_SHIFT }
}

/// A park tree's model in `cove_canopy::SPECIES` (one-based).
fn park_model(species: &str) -> u32 {
    match species {
        "park-oak" => 1,
        "park-pine" => 3,
        _ => 2,
    }
}

/// The far tier's far-species context (`--no-far-species`): the Presidio's green.
fn presidio_green(cover: &eo_landmarks::ground_cover::GroundCover, g3: &eo_landmarks::vegetation::Gg3) -> Option<usize> {
    g3.far_species
        .then(|| {
            let (lat, lon) = eo_landmarks::vegetation::PRESIDIO_POINT;
            let (px, pz) = eo_landmarks::vegetation::game_xz(lat, lon);
            cover.largest_park_holding(px, pz)
        })
        .flatten()
}

/// One park tree of the ground cover as a far card: None on the beach sand
/// (`--no-cliff-rock-cover`); a coastal tree of the mapped woods or the
/// Presidio in its species' form (`--no-far-species`); else the round park
/// crown (`park_tree`). Each keeps its baked canopy model.
fn far_park_tree(cover: &eo_landmarks::ground_cover::GroundCover, t: eo_landmarks::ground_cover::ParkTree, g3: &eo_landmarks::vegetation::Gg3, presidio: Option<usize>, y_at: &dyn Fn(f64, f64) -> f64) -> Option<FarTree> {
    // the shore distance: the citywide shore field once the app installs it,
    // else the nearest drawn sea within the reach that matters here.
    let shore = || eo_landmarks::vegetation::shore_distance(t.x, t.z).unwrap_or_else(|| shore_estimate(t.x, t.z, y_at));
    // round 3 (`--no-cliff-rock-cover`): not on the beach sand the ground tone
    // draws past a mapped beach's ring (the near tier's own test).
    if g3.cliff_rock_cover && cover.near_mapped_beach(t.x, t.z) && cover.on_beach_sand(t.x, t.z, shore(), t.y) {
        return None;
    }
    if g3.far_species && (cover.tree_class_at(t.x, t.z) == 1 || presidio.is_some_and(|i| cover.in_green(i, t.x, t.z))) {
        let seed = hash((t.x * 10.0) as i64 as u32 ^ ((t.z * 10.0) as i64 as u32).rotate_left(16));
        let shore = shore();
        let draw = eo_landmarks::vegetation::hash((t.x * 8.0) as i64, (t.z * 8.0) as i64, 91);
        let (sp, h, cr, c) = eo_landmarks::vegetation::far_form(t.x, t.z, shore, draw);
        let species = (sp as u32 + 1) | (park_model(t.species) << SPECIES_MODEL_SHIFT);
        return Some(FarTree { pos: [t.x as f32, t.y as f32, t.z as f32], height: h, crown: cr, tint: rgb(c[0], c[1], c[2]), seed, species });
    }
    Some(park_tree(t))
}

const COVE_TREE_CAPACITY: u32 = 4096;

fn black_point_bounds() -> ([f64; 2], [f64; 2]) {
    (eo_data::groundscale::plan([-905.0, -3349.0]), eo_data::groundscale::plan([-607.0, -3100.0]))
}

/// Fill the mapped Black Point woods at the same sampling density as the near
/// trees. A single 512 m gathering area otherwise shares its 150-tree limit
/// between the entire hillside and the neighbouring park. Keep existing stems.
pub fn fill_black_point(
    trees: &mut Vec<FarTree>,
    cover: &eo_landmarks::ground_cover::GroundCover,
    ground: &dyn Fn(f64, f64) -> f64,
    blocked: &dyn Fn(f64, f64) -> bool,
) {
    let ([x0, z0], [x1, z1]) = black_point_bounds();
    let g3 = eo_landmarks::vegetation::gg3();
    let presidio = presidio_green(cover, &g3);
    let key = |x: f32, z: f32| (x.to_bits(), z.to_bits());
    let mut existing = std::collections::HashSet::new();
    for t in trees.iter().filter(|t| (x0..x1).contains(&(t.pos[0] as f64)) && (z0..z1).contains(&(t.pos[2] as f64))) {
        existing.insert(key(t.pos[0], t.pos[2]));
    }
    let mut z = z0;
    while z < z1 {
        let mut x = x0;
        while x < x1 {
            for t in cover.park_trees(x, z, (x + 64.0).min(x1), (z + 64.0).min(z1), ground, blocked) {
                if cover.tree_class_at(t.x, t.z) == 1 && existing.insert(key(t.x as f32, t.z as f32)) {
                    trees.extend(far_park_tree(cover, t, &g3, presidio, ground));
                }
            }
            x += 64.0;
        }
        z += 64.0;
    }
}

/// The ground cover's block before `--no-far-tree-density`: 512 m, where its
/// cap of 150 trees a call (ground_cover.rs `MAX_NEAR`) held every stand to
/// about 5.7 trees a game hectare (about 3.7 a real one).
pub const PARK_BLOCK_OLD_M: f64 = 512.0;
/// The block the distant park trees are sampled in: the street tile
/// (`eo_street::STREET_TILE_M`, 128 m), the block the near tier (eo-app
/// props.rs) samples the same ground cover in, so both tiers sample the same
/// world-anchored lattice in the same blocks. The 14 m lattice puts at most
/// about 84 points in it, below the ground cover's cap (the test
/// `the_street_tile_block_is_below_the_ground_covers_cap` reads the cap from
/// ground_cover.rs's behaviour), so a stand keeps its class rule's density.
///
/// For scale, in real metres (the plan is drawn at 1.25, so a 14 m game
/// lattice is 11.2 m real, one point per 125 real m2): at most about 80
/// trees a real hectare, class 1 woods (chance 0.86) about 69. The Presidio
/// Trust's forestry fact sheet (June 2023) gives the historic forest as
/// about 75,000 trees on 306 acres (about 600 a hectare) and thins
/// reforested stands to about 50 trees an acre (about 124 a hectare) when
/// mature, so the lattice stays below either.
pub const PARK_BLOCK_M: f64 = eo_street::STREET_TILE_M;

/// The park trees of the ground cover over [x0, x1) x [z0, z1), in
/// `PARK_BLOCK_M` blocks (`PARK_BLOCK_OLD_M` with `--no-far-tree-density`).
pub fn park_trees(cover: &eo_landmarks::ground_cover::GroundCover, x0: f64, z0: f64, x1: f64, z1: f64, y_at: &(dyn Fn(f64, f64) -> f64 + Sync), blocked: &(dyn Fn(f64, f64) -> bool + Sync)) -> Vec<FarTree> {
    park_trees_in(cover, x0, z0, x1, z1, y_at, blocked, if crate::far_city::FarCity::current().tree_density { PARK_BLOCK_M } else { PARK_BLOCK_OLD_M })
}

/// `park_trees` with block side `b`.
#[allow(clippy::too_many_arguments)]
/// How far the drawn sea is from (x, z) (m): the nearest of 16 rays, in
/// 6 m steps out to `SHORE_REACH_M`, that meets ground under the sea plus
/// 0.3 m (the far tier's `blocked` water test); 1 km when none does. The
/// far tier reads it before the app installs the shore field
/// (`eo_landmarks::vegetation::install_shore_distance`): the bluff cypress
/// mix and the beach-sand test need only the first 240 m.
pub const SHORE_REACH_M: f64 = 252.0;
pub fn shore_estimate(x: f64, z: f64, y_at: &dyn Fn(f64, f64) -> f64) -> f64 {
    let sea = eo_water::SEA_LEVEL_Y + 0.3;
    let mut best = 1e3f64;
    for k in 0..16 {
        let (sn, cs) = (k as f64 * std::f64::consts::TAU / 16.0).sin_cos();
        let mut d = 6.0;
        while d <= SHORE_REACH_M.min(best) {
            if y_at(x + cs * d, z + sn * d) < sea {
                best = best.min(d);
                break;
            }
            d += 6.0;
        }
    }
    best
}

pub fn park_trees_in(cover: &eo_landmarks::ground_cover::GroundCover, x0: f64, z0: f64, x1: f64, z1: f64, y_at: &(dyn Fn(f64, f64) -> f64 + Sync), blocked: &(dyn Fn(f64, f64) -> bool + Sync), b: f64) -> Vec<FarTree> {
    #[allow(non_snake_case)]
    let B = b;
    let mut blocks = vec![];
    let mut z = (z0 / B).floor() * B;
    while z < z1 {
        let mut x = (x0 / B).floor() * B;
        while x < x1 {
            blocks.push((x, z));
            x += B;
        }
        z += B;
    }
    // round 3 (`--no-far-species`): the coastal trees of the mapped woods and the
    // Presidio as the near tier draws them (its species, sizes and leaf colour; the
    // card's form by species, hlod_trees.wgsl), instead of round park trees.
    let g3 = eo_landmarks::vegetation::gg3();
    let presidio = presidio_green(cover, &g3);
    let threads = std::thread::available_parallelism().map(|p| p.get()).unwrap_or(4).clamp(1, 8);
    let chunk = blocks.len().div_ceil(threads).max(1);
    let parts: Vec<Vec<FarTree>> = std::thread::scope(|s| {
        let hs: Vec<_> = blocks
            .chunks(chunk)
            .map(|c| {
                s.spawn(move || {
                    let mut out = vec![];
                    for &(x, z) in c {
                        for t in cover.park_trees(x, z, x + B, z + B, y_at, blocked) {
                            out.extend(far_park_tree(cover, t, &g3, presidio, y_at));
                        }
                    }
                    out
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    parts.into_iter().flatten().collect()
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    range: [f32; 4],
}

/// Side of a tree block, metres: the unit the GPU culls (frustum, near
/// distance, the occlusion pyramid) before any tree's vertices run.
pub const BLOCK_M: f32 = 128.0;
/// Blocks the table holds (the city has about 3,000 with trees).
pub const MAX_BLOCKS: u32 = 16_384;

/// One block of trees: a contiguous run of the list and its box.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
pub struct TreeBlock {
    pub min: [f32; 3],
    pub first: u32,
    pub max: [f32; 3],
    pub count: u32,
}

/// The tree list ordered by block, and its blocks (`into_blocks`).
#[derive(Clone, Debug, Default)]
pub struct FarTreeList {
    pub trees: Vec<FarTree>,
    pub blocks: Vec<TreeBlock>,
    canopy: std::collections::VecDeque<cove_canopy::Level>,
}

impl FarTreeList {
    pub fn bake_black_point_canopy(&mut self, root: &std::path::Path, js: &eo_props::JsProps) {
        let ([x0, z0], [x1, z1]) = black_point_bounds();
        match cove_canopy::bake(root, &js.consts["vegetationAtlas"]) {
            Ok(levels) => {
                for t in &mut self.trees {
                    if t.species >> SPECIES_MODEL_SHIFT != 0 && (x0..x1).contains(&(t.pos[0] as f64)) && (z0..z1).contains(&(t.pos[2] as f64)) {
                        t.species |= SPECIES_BAKED;
                    }
                }
                self.canopy = levels;
            }
            Err(e) => eprintln!("Black Point canopy unavailable ({e:#}); using the distant tree silhouettes"),
        }
    }
}

/// Orders the trees by `BLOCK_M` block and makes the block table (on the
/// gathering thread, not the render thread). A block's box holds every card
/// of its trees: the crown's half width round the trunk, from its base to its top.
pub fn into_blocks(mut trees: Vec<FarTree>) -> FarTreeList {
    trees.truncate(CAPACITY as usize);
    let key = |t: &FarTree| ((t.pos[2] / BLOCK_M).floor() as i32, (t.pos[0] / BLOCK_M).floor() as i32);
    trees.sort_by_key(key);
    let mut blocks: Vec<TreeBlock> = vec![];
    let mut last = None;
    for (i, t) in trees.iter().enumerate() {
        let k = key(t);
        let (lo, hi) = ([t.pos[0] - t.crown, t.pos[1], t.pos[2] - t.crown], [t.pos[0] + t.crown, t.pos[1] + t.height, t.pos[2] + t.crown]);
        match blocks.last_mut() {
            Some(b) if last == Some(k) => {
                for a in 0..3 {
                    b.min[a] = b.min[a].min(lo[a]);
                    b.max[a] = b.max[a].max(hi[a]);
                }
                b.count += 1;
            }
            _ => {
                blocks.push(TreeBlock { min: lo, first: i as u32, max: hi, count: 1 });
                last = Some(k);
            }
        }
    }
    blocks.truncate(MAX_BLOCKS as usize);
    if let Some(b) = blocks.last() {
        trees.truncate((b.first + b.count) as usize);
    }
    FarTreeList { trees, blocks, canopy: Default::default() }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CullParams {
    planes: [[f32; 4]; 6],
    view_proj: [[f32; 4]; 4],
    /// The camera; w the near distance (`NEAR_M`).
    cam: [f32; 4],
    /// x the blocks.
    counts: [u32; 4],
    /// the bounded list's guards (bounds.rs): the widened frustum, and x the ring's reach, y 1 to count them.
    wide: [[f32; 4]; 6],
    guard: [f32; 4],
}

pub const WGSL: &str = include_str!("hlod_trees.wgsl");
pub const CULL_WGSL: &str = include_str!("hlod_trees_cull.wgsl");
/// The cull without multi-draw-indirect-count (appended to `CULL_WGSL`).
pub const LIST_WGSL: &str = include_str!("hlod_trees_list.wgsl");

/// The far trees' draw shader: the far layer's city shader (lighting:
/// `look::city_shader_source_far`), the baked canopy's and `FarTree::species`'s
/// constants, hlod_trees.wgsl, and the path's part. A tree's index is the
/// instance's own, or (`fallback`) the cull's list. The card's discards sit
/// before the lighting on the compact path (as they always did), and after it
/// (`late_discard`) on the paths without the count (the fallback's list, the
/// Black Point grove alone), the paths of the adapters (Metal) where a discard
/// ends the invocation and leaves the light's screen derivatives undefined (ft_shade).
pub fn draw_shader_source(fallback: bool, late_discard: bool) -> String {
    let index = if fallback {
        "@group(2) @binding(3) var<storage, read> ft_list: array<u32>;\nfn ft_index(ii: u32) -> u32 { return ft_list[ii]; }\n"
    } else {
        "fn ft_index(ii: u32) -> u32 { return ii; }\n"
    };
    format!(
        "{}\nconst FT_VIEWS: u32 = {}u;\nconst FT_NORMALS: i32 = {};\nconst FT_FORM_MASK: u32 = {SPECIES_FORM_MASK}u;\nconst FT_BAKED: u32 = {SPECIES_BAKED}u;\nconst FT_MODEL_SHIFT: u32 = {SPECIES_MODEL_SHIFT}u;\n{WGSL}\n{index}const FT_LATE_DISCARD: bool = {late_discard};\n",
        crate::look::city_shader_source_far(),
        cove_canopy::VIEWS,
        cove_canopy::LAYERS
    )
}

/// Instances the buffer holds (the city has about 155,000).
pub const CAPACITY: u32 = 262_144;
/// Bytes of the tree list written a frame while it streams in.
pub const WRITE_BYTES_PER_FRAME: usize = 512 << 10;

pub struct FarTrees {
    cove_only: bool,
    canopy: wgpu::Texture,
    pending_canopy: std::sync::Mutex<std::collections::VecDeque<cove_canopy::Level>>,
    pipeline: wgpu::RenderPipeline,
    bg: wgpu::BindGroup,
    params: wgpu::Buffer,
    buf: wgpu::Buffer,
    block_buf: wgpu::Buffer,
    cull_params: wgpu::Buffer,
    cmd_buf: wgpu::Buffer,
    count_buf: wgpu::Buffer,
    /// The list in the count emulation (no MULTI_DRAW_INDIRECT_COUNT), registered at attach.
    counted: std::sync::OnceLock<crate::counted::CountedLists>,
    cull_plain: wgpu::ComputePipeline,
    cull_hzb: wgpu::ComputePipeline,
    cull_bg: wgpu::BindGroup,
    /// Without multi-draw-indirect-count: the cull that lists the kept trees,
    /// and the one draw (6 vertices, the kept trees) it fills (`draw`).
    list: Option<(wgpu::ComputePipeline, wgpu::Buffer)>,
    /// Trees and blocks drawn (set once the whole list is on the GPU).
    count: std::sync::atomic::AtomicU32,
    blocks: std::sync::atomic::AtomicU32,
    /// The list (trees, then blocks) still to write, and how far it is written.
    pending: std::sync::Mutex<Option<(Vec<u8>, usize, u32, u32)>>,
    pub bytes: u64,
    /// Frames updated (the fade dither's phase).
    frame: std::sync::atomic::AtomicU32,
    /// The phase held (`set_phase_frozen`): every update draws the same screen door.
    phase_frozen: std::sync::atomic::AtomicBool,
}

pub struct FarTreesInit<'a> {
    pub cove_only: bool,
    pub canopy: bool,
    pub device: &'a wgpu::Device,
    pub draw_bgl: &'a wgpu::BindGroupLayout,
    pub main_bgl: &'a wgpu::BindGroupLayout,
    /// The occlusion pyramid's layout (`occlusion.late_bgl`).
    pub late_bgl: &'a wgpu::BindGroupLayout,
    pub sample_count: u32,
    pub debug_ids: bool,
    /// No multi-draw-indirect-count: one draw over a list the cull writes.
    pub fallback: bool,
}

impl FarTrees {
    /// The pipelines and empty buffers of `CAPACITY` trees and `MAX_BLOCKS`
    /// blocks; the list comes later (`supply`, `pump`).
    pub fn new(i: &FarTreesInit) -> Self {
        let d = i.device;
        use wgpu::BufferUsages as U;
        let buffer = |label: &str, size: u64, usage| d.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false });
        // the Black Point grove alone, as one instanced draw, where the fallback's
        // list (which draws every tree without the count) is not built
        let cove_only = i.cove_only && !i.fallback;
        let capacity = if cove_only { COVE_TREE_CAPACITY } else { CAPACITY };
        let size = capacity as u64 * std::mem::size_of::<FarTree>() as u64;
        let buf = buffer("hlod-far-trees", size, U::STORAGE | U::COPY_DST);
        let params = buffer("hlod-far-tree-params", 16, U::UNIFORM | U::COPY_DST);
        let block_bytes = MAX_BLOCKS as u64 * std::mem::size_of::<TreeBlock>() as u64;
        let block_buf = buffer("hlod-far-tree-blocks", block_bytes, U::STORAGE | U::COPY_DST);
        let cull_params = buffer("hlod-far-tree-cull-params", std::mem::size_of::<CullParams>() as u64, U::UNIFORM | U::COPY_DST);
        let cmd_buf = buffer("hlod-far-tree-draws", MAX_BLOCKS as u64 * 16, U::STORAGE | U::INDIRECT | U::COPY_DST); // COPY_DST: the count emulation and the bounded draws zero it
        // [0] commands, [1] trees, [2] blocks in the frustum, [3] occluded, [4] [5] the bounded list's guards.
        let count_buf = buffer("hlod-far-tree-counts", 32, U::STORAGE | U::INDIRECT | U::COPY_DST | U::COPY_SRC);
        let side = if i.canopy { cove_canopy::SIDE } else { 1 };
        let canopy = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("Black Point canopy"), size: wgpu::Extent3d { width: side, height: side, depth_or_array_layers: if i.canopy { cove_canopy::LAYERS * 2 } else { 1 } },
            mip_level_count: if i.canopy { cove_canopy::LEVELS } else { 1 }, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm, usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[],
        });
        let canopy_view = canopy.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        // the fallback's list of kept trees (one index each) and its one draw
        let list_buf = i.fallback.then(|| buffer("hlod-far-tree-list", CAPACITY as u64 * 4, U::STORAGE));
        let draw_buf = i.fallback.then(|| {
            use wgpu::util::DeviceExt;
            d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("hlod-far-tree-draw"), contents: bytemuck::cast_slice(&[6u32, 0, 0, 0]), usage: U::STORAGE | U::INDIRECT | U::COPY_DST })
        });
        let ro = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None };
        let mut draw_entries = vec![
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: vf, ty: ro, count: None },
            wgpu::BindGroupLayoutEntry { binding: 1, visibility: vf, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
            // the baked Black Point canopy (a 1 x 1 stand-in without `canopy`)
            wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false,
            }, count: None },
        ];
        if i.fallback {
            // the fallback's list of kept trees (`draw_shader_source`'s `ft_list`)
            draw_entries.push(wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::VERTEX, ty: ro, count: None });
        }
        let bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("hlod-far-trees-bgl"), entries: &draw_entries });
        let mut bg_entries = vec![
            wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: params.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&canopy_view) },
        ];
        if let Some(l) = &list_buf {
            bg_entries.push(wgpu::BindGroupEntry { binding: 3, resource: l.as_entire_binding() });
        }
        let bg = d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("hlod-far-trees-bg"), layout: &bgl, entries: &bg_entries });
        // Cull: group 0 its own, group 1 the pyramid (the hzb variant), as the far cells' (hlod.rs).
        let cs = wgpu::ShaderStages::COMPUTE;
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry { binding, visibility: cs, ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None }, count: None };
        let st = |read_only| wgpu::BufferBindingType::Storage { read_only };
        let mut cull_entries = vec![entry(0, wgpu::BufferBindingType::Uniform), entry(1, st(true)), entry(2, st(false)), entry(3, st(false))];
        if i.fallback {
            cull_entries.extend([entry(4, st(false)), entry(5, st(false))]);
        }
        let cull_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("hlod-far-tree-cull-bgl"), entries: &cull_entries });
        let mut cull_bind = vec![
            wgpu::BindGroupEntry { binding: 0, resource: cull_params.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: block_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: cmd_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: count_buf.as_entire_binding() },
        ];
        if let (Some(l), Some(dr)) = (&list_buf, &draw_buf) {
            cull_bind.extend([wgpu::BindGroupEntry { binding: 4, resource: l.as_entire_binding() }, wgpu::BindGroupEntry { binding: 5, resource: dr.as_entire_binding() }]);
        }
        let cull_bg = d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("hlod-far-tree-cull-bg"), layout: &cull_bgl, entries: &cull_bind });
        let plain_src = format!("{CULL_WGSL}\nfn tc_occluded(mn: vec3<f32>, mx: vec3<f32>) -> bool {{ return false; }}\n");
        let hzb_src = format!(
            "{CULL_WGSL}\nstruct HzbInfo {{ screen: vec4<f32> }};\n@group(1) @binding(0) var hzb: texture_2d<f32>;\n@group(1) @binding(1) var<uniform> hzb_info: HzbInfo;\n{}\nfn tc_occluded(mn: vec3<f32>, mx: vec3<f32>) -> bool {{ return hzb_occluded(mn, mx, tc.view_proj, hzb_info.screen.xy); }}\n",
            crate::occlusion::OCCLUSION_WGSL
        );
        let cmod = |label: &str, src: String| d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(src.into()) });
        let (pm, hm) = (cmod("hlod_trees_cull.wgsl", plain_src), cmod("hlod_trees_cull.wgsl (hzb)", hzb_src));
        let lay_p = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("hlod-far-tree-cull-layout"), bind_group_layouts: &[Some(&cull_bgl)], immediate_size: 0 });
        let lay_h = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("hlod-far-tree-cull-hzb-layout"), bind_group_layouts: &[Some(&cull_bgl), Some(i.late_bgl)], immediate_size: 0 });
        let compute = |m: &wgpu::ShaderModule, l: &wgpu::PipelineLayout, label: &str| {
            d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(label), layout: Some(l), module: m, entry_point: Some("tree_cull"), compilation_options: Default::default(), cache: None })
        };
        let cull_plain = compute(&pm, &lay_p, "hlod_tree_cull");
        let cull_hzb = compute(&hm, &lay_h, "hlod_tree_cull (hzb)");
        let list = draw_buf.map(|dr| {
            let lm = cmod("hlod_trees_list.wgsl", format!("{CULL_WGSL}\n{LIST_WGSL}\nfn tc_occluded(mn: vec3<f32>, mx: vec3<f32>) -> bool {{ return false; }}\n"));
            let p = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("hlod_tree_cull (list)"), layout: Some(&lay_p), module: &lm, entry_point: Some("tree_cull_list"), compilation_options: Default::default(), cache: None });
            (p, dr)
        });
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("hlod_trees.wgsl"), source: wgpu::ShaderSource::Wgsl(draw_shader_source(i.fallback, i.fallback || cove_only).into()) });
        let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("hlod-far-trees-layout"), bind_group_layouts: &[Some(i.draw_bgl), Some(i.main_bgl), Some(&bgl)], immediate_size: 0 });
        let color = wgpu::ColorTargetState { format: crate::look::HDR_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL };
        let ids = wgpu::ColorTargetState { format: crate::IDS_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL };
        let targets_ids = [Some(color.clone()), Some(ids)];
        let targets = [Some(color)];
        let pipeline = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("hlod-far-trees"),
            layout: Some(&layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("far_tree_vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: crate::DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState { count: i.sample_count, ..Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some(if i.debug_ids { "far_tree_fs_ids" } else { "far_tree_fs" }),
                compilation_options: Default::default(),
                targets: if i.debug_ids { &targets_ids } else { &targets },
            }),
            multiview_mask: None,
            cache: None,
        });
        let bytes = size + block_bytes + MAX_BLOCKS as u64 * 16 + 16 + 32 + std::mem::size_of::<CullParams>() as u64 + if i.fallback { CAPACITY as u64 * 4 + 16 } else { 0 };
        Self {
            pipeline,
            bg,
            params,
            buf,
            block_buf,
            cull_params,
            cmd_buf,
            count_buf,
            counted: std::sync::OnceLock::new(),
            cull_plain,
            cull_hzb,
            cull_bg,
            cove_only,
            canopy,
            pending_canopy: Default::default(),
            list,
            count: std::sync::atomic::AtomicU32::new(0),
            blocks: std::sync::atomic::AtomicU32::new(0),
            pending: std::sync::Mutex::new(None),
            bytes: bytes + if i.canopy { cove_canopy::BYTES } else { 4 },
            frame: std::sync::atomic::AtomicU32::new(0),
            phase_frozen: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Hold the fade dither's phase (no step a frame) while `on`, so frames
    /// rendered in turn draw the same screen door (`--bench --occlusion-check`
    /// compares such frames pixel by pixel). Off by default: the phase steps.
    pub fn set_phase_frozen(&self, on: bool) {
        self.phase_frozen.store(on, std::sync::atomic::Ordering::Relaxed);
    }

    /// The tree list (gathered and ordered off the render thread, `into_blocks`); written by `pump`.
    pub fn supply(&self, mut list: FarTreeList) {
        *self.pending_canopy.lock().unwrap() = std::mem::take(&mut list.canopy);
        let capacity = if self.cove_only {
            let ([x0, z0], [x1, z1]) = black_point_bounds();
            list.trees.retain(|t| (x0..x1).contains(&(t.pos[0] as f64)) && (z0..z1).contains(&(t.pos[2] as f64)));
            list.blocks.clear();
            COVE_TREE_CAPACITY
        } else { CAPACITY };
        let n = list.trees.len().min(capacity as usize);
        let nb = list.blocks.iter().take_while(|b| (b.first + b.count) as usize <= n).count().min(MAX_BLOCKS as usize);
        let mut data = bytemuck::cast_slice(&list.trees[..n]).to_vec();
        data.extend_from_slice(bytemuck::cast_slice(&list.blocks[..nb]));
        *self.pending.lock().unwrap() = Some((data, 0, n as u32, nb as u32));
    }

    /// One frame of the list's upload (at most `WRITE_BYTES_PER_FRAME`): the
    /// trees, then the blocks; the trees draw once all of it is written.
    /// Returns the bytes written.
    pub fn pump(&self, queue: &wgpu::Queue) -> u64 {
        let mut uploads = self.pending_canopy.lock().unwrap();
        let mut written = 0;
        while uploads.front().is_some_and(|l| written + l.bytes.len() <= WRITE_BYTES_PER_FRAME) {
            let level = uploads.pop_front().unwrap();
            let side = cove_canopy::SIDE >> level.mip;
            queue.write_texture(wgpu::TexelCopyTextureInfo { texture: &self.canopy, mip_level: level.mip,
                origin: wgpu::Origin3d { x: 0, y: 0, z: level.layer }, aspect: wgpu::TextureAspect::All },
                &level.bytes, wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(side * 4), rows_per_image: Some(side) },
                wgpu::Extent3d { width: side, height: side, depth_or_array_layers: cove_canopy::VIEWS });
            written += level.bytes.len();
        }
        drop(uploads);
        if written != 0 { return written as u64; }
        let mut p = self.pending.lock().unwrap();
        let Some((data, at, n, nb)) = p.as_mut() else { return 0 };
        let tree_bytes = *n as usize * std::mem::size_of::<FarTree>();
        // Never across the trees / blocks boundary in one write.
        let end = if *at < tree_bytes { tree_bytes } else { data.len() };
        let take = (end - *at).min(WRITE_BYTES_PER_FRAME);
        if take > 0 {
            if *at < tree_bytes {
                queue.write_buffer(&self.buf, *at as u64, &data[*at..*at + take]);
            } else {
                queue.write_buffer(&self.block_buf, (*at - tree_bytes) as u64, &data[*at..*at + take]);
            }
            *at += take;
        }
        if *at >= data.len() {
            self.count.store(*n, std::sync::atomic::Ordering::Relaxed);
            self.blocks.store(*nb, std::sync::atomic::Ordering::Relaxed);
            *p = None;
        }
        take as u64
    }

    /// No list pending (the trees are drawn, or none were supplied yet).
    pub fn idle(&self) -> bool {
        self.pending.lock().unwrap().is_none()
    }

    pub fn count(&self) -> u32 {
        self.count.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn block_count(&self) -> u32 {
        self.blocks.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The frame's camera (the city pass's jittered clip matrix and its planes).
    pub fn update(&self, queue: &wgpu::Queue, on: bool, view_proj: glam::Mat4, planes: [[f32; 4]; 6], eye: glam::Vec3) {
        // w: 0 the round-2 light and a fixed dither; with the far tree light
        // (`--no-far-tree-light`) 1 + this frame's dither phase (a golden-ratio
        // step a frame, so the temporal resolve averages the fade band's screen door).
        let w = if eo_landmarks::vegetation::gg3().far_tree_light {
            let f = if self.phase_frozen.load(std::sync::atomic::Ordering::Relaxed) {
                self.frame.load(std::sync::atomic::Ordering::Relaxed)
            } else {
                self.frame.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            };
            1.0 + (f as f32 * 0.618_034).fract()
        } else {
            0.0
        };
        let p = Params { range: [NEAR_M, FULL_M, if on { 1.0 } else { 0.0 }, w] };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&p));
        let c = CullParams { planes, view_proj: view_proj.to_cols_array_2d(), cam: [eye.x, eye.y, eye.z, NEAR_M], counts: [self.block_count(), 0, 0, 0], wide: [[0.0; 4]; 6], guard: [0.0; 4] };
        queue.write_buffer(&self.cull_params, 0, bytemuck::bytes_of(&c));
    }

    /// The bounded list's guards for this frame (after `update`): the widened
    /// frustum's planes and the ring's reach (bounds.rs).
    pub fn set_guard(&self, queue: &wgpu::Queue, wide: [[f32; 4]; 6], reach: f32) {
        let at = std::mem::offset_of!(CullParams, wide) as u64;
        let mut words = [0f32; 28];
        for (k, p) in wide.iter().enumerate() {
            words[k * 4..k * 4 + 4].copy_from_slice(p);
        }
        words[24] = reach;
        words[25] = 1.0;
        words[26] = crate::bounds::NEAR_M;
        queue.write_buffer(&self.cull_params, at, bytemuck::cast_slice(&words));
    }

    /// The cull's counters (see `count_buf`).
    pub fn counter_buffer(&self) -> &wgpu::Buffer {
        &self.count_buf
    }

    /// `bound`: the bounded list's commands, emptied before the cull appends (bounds.rs).
    pub fn clear(&self, enc: &mut wgpu::CommandEncoder, bound: Option<u32>) {
        enc.clear_buffer(&self.count_buf, 0, None);
        if let Some((_, dr)) = &self.list {
            enc.clear_buffer(dr, 4, Some(4)); // the kept trees; the vertex count stays 6
        }
        if let Some(b) = bound.filter(|_| self.list.is_none() && !self.cove_only) {
            crate::clear_list(enc, &self.cmd_buf, 0, b, 16);
        }
    }

    /// Put the block list (count word 0, non-indexed) in the count emulation.
    pub fn register_counts(&self, counts: &std::sync::Arc<crate::counted::Counts>) {
        let list = crate::counted::List { word: 0, draws: self.cmd_buf.clone(), offset: 0, stride: 16, max: MAX_BLOCKS, floor: None };
        let _ = self.counted.set(counts.register("far-trees", &self.count_buf, vec![list]));
    }

    /// Frustum, near-distance and (after the pyramid) occlusion test of every block.
    pub fn encode_cull(&self, pass: &mut wgpu::ComputePass<'_>, hzb: Option<&wgpu::BindGroup>) {
        let n = self.block_count();
        if n == 0 {
            return;
        }
        match (hzb, &self.list) {
            // without the count there is no occlusion pass either (lib.rs)
            (_, Some((p, _))) => {
                pass.set_pipeline(p);
                pass.set_bind_group(0, &self.cull_bg, &[]);
            }
            (Some(h), None) => {
                pass.set_pipeline(&self.cull_hzb);
                pass.set_bind_group(0, &self.cull_bg, &[]);
                pass.set_bind_group(1, h, &[]);
            }
            (None, None) => {
                pass.set_pipeline(&self.cull_plain);
                pass.set_bind_group(0, &self.cull_bg, &[]);
            }
        }
        pass.dispatch_workgroups(n.div_ceil(64), 1, 1);
    }

    /// In the city pass with the opaque city (groups 0 and 1 bound): the blocks the cull kept
    /// (`bound`: the bounded draws' commands, bounds.rs; None: the count emulation's or the indirect count).
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, bound: Option<u32>) {
        if self.cove_only {
            let count = self.count();
            if count > 0 {
                // The grove fits in a single ordinary instanced draw on adapters
                // without counted draws. The vertex shader handles the near fade.
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(2, &self.bg, &[]);
                pass.draw(0..6, 0..count);
            }
            return;
        }
        let n = self.block_count();
        if n == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(2, &self.bg, &[]);
        if let Some((_, dr)) = &self.list {
            pass.draw_indirect(dr, 0); // the kept trees as one draw
            return;
        }
        match bound {
            None => crate::counted::draw(pass, self.counted.get(), 0, &self.cmd_buf, 0, &self.count_buf, 0, n),
            Some(b) if b.min(n) > 0 => pass.multi_draw_indirect(&self.cmd_buf, 0, b.min(n)),
            Some(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both paths' draw shaders parse and validate (naga, on the CPU).
    #[test]
    fn the_draw_shader_validates_on_both_paths() {
        use wgpu::naga;
        for (fallback, late) in [(false, false), (true, true), (false, true)] {
            let src = draw_shader_source(fallback, late);
            let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("fallback {fallback}, late {late}: {}", e.emit_to_string(&src)));
            naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
                .validate(&module)
                .unwrap_or_else(|e| panic!("fallback {fallback}, late {late}: {}", e.emit_to_string(&src)));
        }
    }

    /// One all-wood green over the test area (class 1, the densest rule).
    fn all_wood() -> eo_landmarks::ground_cover::GroundCover {
        let wood = eo_landmarks::plaza::Area {
            id: "test-wood".into(),
            kind: "wood".into(),
            ground: "vegetated".into(),
            surface: String::new(),
            polygon: vec![[-2000.0, -2000.0], [2000.0, -2000.0], [2000.0, 2000.0], [-2000.0, 2000.0]],
            holes: vec![],
        };
        eo_landmarks::ground_cover::GroundCover::new(&[wood], &[], &[])
    }

    #[test]
    fn the_street_tile_block_is_below_the_ground_covers_cap() {
        assert_eq!(PARK_BLOCK_M, eo_street::STREET_TILE_M, "the near and far tiers sample in the same blocks");
        let cover = all_wood();
        let (flat, open) = (|_: f64, _: f64| 0.0, |_: f64, _: f64| false);
        let n = |x: f64, z: f64, b: f64| cover.park_trees(x, z, x + b, z + b, &flat, &open).len();
        // a block keeps every tree when it holds as many as its four quarters
        // together, so ground_cover's cap does not bind in it
        for (x, z) in [(0.0, 0.0), (128.0, -256.0), (-640.0, 384.0)] {
            let h = PARK_BLOCK_M / 2.0;
            let quarters = n(x, z, h) + n(x + h, z, h) + n(x, z + h, h) + n(x + h, z + h, h);
            assert_eq!(n(x, z, PARK_BLOCK_M), quarters, "the cap binds in a {PARK_BLOCK_M} m block at ({x}, {z})");
            assert!(quarters > 40, "{quarters}: the wood is not dense");
        }
        // the old block was capped: it holds fewer than its sixteen street tiles
        let tiles: usize = (0..4).flat_map(|i| (0..4).map(move |j| (i, j))).map(|(i, j)| n(i as f64 * 128.0, j as f64 * 128.0, 128.0)).sum();
        let old = n(0.0, 0.0, PARK_BLOCK_OLD_M);
        assert!(old * 4 < tiles, "old block {old}, its street tiles {tiles}");
        assert!(old as f64 / (PARK_BLOCK_OLD_M * PARK_BLOCK_OLD_M / 1e4) < 6.0, "the old block held {old}: more than about 5.7 a game hectare");
    }

    #[test]
    fn blocks_hold_every_tree_in_one_contiguous_run() {
        let mut trees = vec![];
        for i in 0..500u32 {
            let (x, z) = ((i * 37 % 1000) as f32 - 500.0, (i * 91 % 700) as f32 - 350.0);
            trees.push(FarTree { pos: [x, (i % 7) as f32, z], height: 6.0 + (i % 5) as f32, crown: 2.0, tint: 0, seed: i, species: 0 });
        }
        let list = into_blocks(trees.clone());
        assert_eq!(list.trees.len(), trees.len());
        let mut covered = 0;
        let mut keys = std::collections::HashSet::new();
        for b in &list.blocks {
            assert_eq!(b.first, covered, "blocks are contiguous runs in order");
            covered += b.count;
            let run = &list.trees[b.first as usize..(b.first + b.count) as usize];
            let k = ((run[0].pos[2] / BLOCK_M).floor() as i32, (run[0].pos[0] / BLOCK_M).floor() as i32);
            assert!(keys.insert(k), "one run per block");
            for t in run {
                assert_eq!(((t.pos[2] / BLOCK_M).floor() as i32, (t.pos[0] / BLOCK_M).floor() as i32), k);
                assert!(t.pos[0] - t.crown >= b.min[0] && t.pos[0] + t.crown <= b.max[0]);
                assert!(t.pos[2] - t.crown >= b.min[2] && t.pos[2] + t.crown <= b.max[2]);
                assert!(t.pos[1] >= b.min[1] && t.pos[1] + t.height <= b.max[1], "the card's height is inside the box");
            }
        }
        assert_eq!(covered as usize, list.trees.len());
        let mut a: Vec<u32> = trees.iter().map(|t| t.seed).collect();
        let mut b: Vec<u32> = list.trees.iter().map(|t| t.seed).collect();
        a.sort_unstable();
        b.sort_unstable();
        assert_eq!(a, b, "the same trees");
    }
}
