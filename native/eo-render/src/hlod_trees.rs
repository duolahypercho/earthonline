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
    /// One-based model in cove_canopy::SPECIES. Bit 4 selects a baked Black Point tree.
    pub species: u32,
}

fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | ((g as u32) << 8) | ((b as u32) << 16) | (255 << 24)
}

fn hash(a: u32) -> u32 {
    let mut x = a.wrapping_mul(0x9e37_79b9) ^ 0x85eb_ca6b;
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^ (x >> 15)
}

/// (`--far-palms`) The species of the survey's palms: drawn as palms
/// (hlod_trees.wgsl `ft_palm`, `FT_PALM`) from where the street props' palms
/// fade out. Without the switch they keep species 0, a round crown.
pub const FAR_PALM: u32 = 7;

/// The street trees of the survey on the ground `ground` gives.
pub fn street_trees(js: &eo_props::JsProps, ground: &dyn Fn(f64, f64) -> f64) -> Vec<FarTree> {
    street_trees_with(js, ground, false)
}

/// `street_trees`, with the palms as `FAR_PALM` when `palms` (`--far-palms`).
pub fn street_trees_with(js: &eo_props::JsProps, ground: &dyn Fn(f64, f64) -> f64, palms: bool) -> Vec<FarTree> {
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
        out.push(FarTree { pos: [x as f32, ground(x, z) as f32, z as f32], height, crown, tint, seed: h, species: match form { "pollard" => 4, "mature" => 5, "round" => 6, "palm" if palms => FAR_PALM, _ => 0 } });
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
    FarTree { pos: [t.x as f32, t.y as f32, t.z as f32], height: h * k, crown: cr * k, tint, seed, species: match t.species { "park-oak" => 1, "park-pine" => 3, _ => 2 } }
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
                    trees.push(park_tree(t));
                }
            }
            x += 64.0;
        }
        z += 64.0;
    }
}

/// The park trees of the ground cover over [x0, x1) x [z0, z1), in 512 m blocks.
pub fn park_trees(cover: &eo_landmarks::ground_cover::GroundCover, x0: f64, z0: f64, x1: f64, z1: f64, y_at: &(dyn Fn(f64, f64) -> f64 + Sync), blocked: &(dyn Fn(f64, f64) -> bool + Sync)) -> Vec<FarTree> {
    const B: f64 = 512.0;
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
                            out.push(park_tree(t));
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
                    if t.species != 0 && t.species != FAR_PALM && (x0..x1).contains(&(t.pos[0] as f64)) && (z0..z1).contains(&(t.pos[2] as f64)) {
                        t.species |= 16;
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
}

pub const WGSL: &str = include_str!("hlod_trees.wgsl");
pub const CULL_WGSL: &str = include_str!("hlod_trees_cull.wgsl");

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
    /// Trees and blocks drawn (set once the whole list is on the GPU).
    count: std::sync::atomic::AtomicU32,
    blocks: std::sync::atomic::AtomicU32,
    /// The list (trees, then blocks) still to write, and how far it is written.
    pending: std::sync::Mutex<Option<(Vec<u8>, usize, u32, u32)>>,
    pub bytes: u64,
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
}

impl FarTrees {
    /// The pipelines and empty buffers of `CAPACITY` trees and `MAX_BLOCKS`
    /// blocks; the list comes later (`supply`, `pump`).
    pub fn new(i: &FarTreesInit) -> Self {
        let d = i.device;
        use wgpu::BufferUsages as U;
        let buffer = |label: &str, size: u64, usage| d.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false });
        let capacity = if i.cove_only { COVE_TREE_CAPACITY } else { CAPACITY };
        let size = capacity as u64 * std::mem::size_of::<FarTree>() as u64;
        let buf = buffer("hlod-far-trees", size, U::STORAGE | U::COPY_DST);
        let params = buffer("hlod-far-tree-params", 16, U::UNIFORM | U::COPY_DST);
        let block_bytes = MAX_BLOCKS as u64 * std::mem::size_of::<TreeBlock>() as u64;
        let block_buf = buffer("hlod-far-tree-blocks", block_bytes, U::STORAGE | U::COPY_DST);
        let cull_params = buffer("hlod-far-tree-cull-params", std::mem::size_of::<CullParams>() as u64, U::UNIFORM | U::COPY_DST);
        let cmd_buf = buffer("hlod-far-tree-draws", MAX_BLOCKS as u64 * 16, U::STORAGE | U::INDIRECT | U::COPY_DST); // COPY_DST: the count emulation zeroes it
        let count_buf = buffer("hlod-far-tree-counts", 16, U::STORAGE | U::INDIRECT | U::COPY_DST | U::COPY_SRC);
        let side = if i.canopy { cove_canopy::SIDE } else { 1 };
        let canopy = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("Black Point canopy"), size: wgpu::Extent3d { width: side, height: side, depth_or_array_layers: if i.canopy { cove_canopy::LAYERS * 2 } else { 1 } },
            mip_level_count: if i.canopy { cove_canopy::LEVELS } else { 1 }, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm, usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[],
        });
        let canopy_view = canopy.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hlod-far-trees-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: vf, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false,
                }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: vf, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
            ],
        });
        let bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hlod-far-trees-bg"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: params.as_entire_binding() }, wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&canopy_view) }],
        });
        // Cull: group 0 its own, group 1 the pyramid (the hzb variant), as the far cells' (hlod.rs).
        let cs = wgpu::ShaderStages::COMPUTE;
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry { binding, visibility: cs, ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None }, count: None };
        let st = |read_only| wgpu::BufferBindingType::Storage { read_only };
        let cull_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hlod-far-tree-cull-bgl"),
            entries: &[entry(0, wgpu::BufferBindingType::Uniform), entry(1, st(true)), entry(2, st(false)), entry(3, st(false))],
        });
        let cull_bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hlod-far-tree-cull-bg"),
            layout: &cull_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: cull_params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: block_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: cmd_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: count_buf.as_entire_binding() },
            ],
        });
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
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("hlod_trees.wgsl"), source: wgpu::ShaderSource::Wgsl(crate::look::sf_select(&format!("{}\nconst FT_VIEWS: u32 = {}u;\nconst FT_NORMALS: i32 = {};\n{WGSL}", crate::look::city_shader_source_far(), cove_canopy::VIEWS, cove_canopy::LAYERS)).into()) }); // the far layer's city shader (lighting)
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
        let bytes = size + block_bytes + MAX_BLOCKS as u64 * 16 + 16 + 16 + std::mem::size_of::<CullParams>() as u64;
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
            cove_only: i.cove_only,
            canopy,
            pending_canopy: Default::default(),
            count: std::sync::atomic::AtomicU32::new(0),
            blocks: std::sync::atomic::AtomicU32::new(0),
            pending: std::sync::Mutex::new(None),
            bytes: bytes + if i.canopy { cove_canopy::BYTES } else { 4 },
        }
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
        let p = Params { range: [NEAR_M, FULL_M, if on { 1.0 } else { 0.0 }, 0.0] };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&p));
        let c = CullParams { planes, view_proj: view_proj.to_cols_array_2d(), cam: [eye.x, eye.y, eye.z, NEAR_M], counts: [self.block_count(), 0, 0, 0] };
        queue.write_buffer(&self.cull_params, 0, bytemuck::bytes_of(&c));
    }

    pub fn clear(&self, enc: &mut wgpu::CommandEncoder) {
        enc.clear_buffer(&self.count_buf, 0, None);
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
        match hzb {
            Some(h) => {
                pass.set_pipeline(&self.cull_hzb);
                pass.set_bind_group(0, &self.cull_bg, &[]);
                pass.set_bind_group(1, h, &[]);
            }
            None => {
                pass.set_pipeline(&self.cull_plain);
                pass.set_bind_group(0, &self.cull_bg, &[]);
            }
        }
        pass.dispatch_workgroups(n.div_ceil(64), 1, 1);
    }

    /// In the city pass with the opaque city (groups 0 and 1 bound): the blocks the cull kept.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
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
        crate::counted::draw(pass, self.counted.get(), 0, &self.cmd_buf, 0, &self.count_buf, 0, n);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
