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
    pub pad: u32,
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
        out.push(FarTree { pos: [x as f32, ground(x, z) as f32, z as f32], height, crown, tint, seed: h, pad: 0 });
    }
    out
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
                            let (h, cr, tint) = match t.species {
                                "park-pine" => (17.0, 3.6, rgb(44, 66, 44)),
                                "park-oak" => (11.0, 5.0, rgb(62, 82, 46)),
                                _ => (13.0, 4.2, rgb(70, 92, 50)),
                            };
                            let k = t.scale.clamp(0.4, 2.0) as f32;
                            let seed = hash((t.x * 10.0) as i64 as u32 ^ ((t.z * 10.0) as i64 as u32).rotate_left(16));
                            out.push(FarTree { pos: [t.x as f32, t.y as f32, t.z as f32], height: h * k, crown: cr * k, tint, seed, pad: 0 });
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
    FarTreeList { trees, blocks }
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
/// The cull without multi-draw-indirect-count (appended to `CULL_WGSL`).
pub const LIST_WGSL: &str = include_str!("hlod_trees_list.wgsl");

/// The far trees' draw shader: the city shader, hlod_trees.wgsl, and the
/// path's part. A tree's index is the instance's own, or (the fallback) the
/// cull's list. The card's discards sit before the lighting on the compact
/// path (as they always did), and after it on the fallback, which is the path
/// of the adapters (Metal) where a discard ends the invocation and leaves the
/// light's screen derivatives undefined (ft_shade).
pub fn draw_shader_source(fallback: bool) -> String {
    let path = if fallback {
        "@group(2) @binding(2) var<storage, read> ft_list: array<u32>;\nfn ft_index(ii: u32) -> u32 { return ft_list[ii]; }\nconst FT_LATE_DISCARD: bool = true;\n"
    } else {
        "fn ft_index(ii: u32) -> u32 { return ii; }\nconst FT_LATE_DISCARD: bool = false;\n"
    };
    format!("{}\n{WGSL}\n{path}", crate::look::city_shader_source())
}

/// Instances the buffer holds (the city has about 155,000).
pub const CAPACITY: u32 = 262_144;
/// Bytes of the tree list written a frame while it streams in.
pub const WRITE_BYTES_PER_FRAME: usize = 512 << 10;

pub struct FarTrees {
    pipeline: wgpu::RenderPipeline,
    bg: wgpu::BindGroup,
    params: wgpu::Buffer,
    buf: wgpu::Buffer,
    block_buf: wgpu::Buffer,
    cull_params: wgpu::Buffer,
    cmd_buf: wgpu::Buffer,
    count_buf: wgpu::Buffer,
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
}

pub struct FarTreesInit<'a> {
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
        let size = CAPACITY as u64 * std::mem::size_of::<FarTree>() as u64;
        let buf = buffer("hlod-far-trees", size, U::STORAGE | U::COPY_DST);
        let params = buffer("hlod-far-tree-params", 16, U::UNIFORM | U::COPY_DST);
        let block_bytes = MAX_BLOCKS as u64 * std::mem::size_of::<TreeBlock>() as u64;
        let block_buf = buffer("hlod-far-tree-blocks", block_bytes, U::STORAGE | U::COPY_DST);
        let cull_params = buffer("hlod-far-tree-cull-params", std::mem::size_of::<CullParams>() as u64, U::UNIFORM | U::COPY_DST);
        let cmd_buf = buffer("hlod-far-tree-draws", MAX_BLOCKS as u64 * 16, U::STORAGE | U::INDIRECT);
        let count_buf = buffer("hlod-far-tree-counts", 16, U::STORAGE | U::INDIRECT | U::COPY_DST);
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
        ];
        if i.fallback {
            draw_entries.push(wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::VERTEX, ty: ro, count: None });
        }
        let bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("hlod-far-trees-bgl"), entries: &draw_entries });
        let mut bg_entries = vec![wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: params.as_entire_binding() }];
        if let Some(l) = &list_buf {
            bg_entries.push(wgpu::BindGroupEntry { binding: 2, resource: l.as_entire_binding() });
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
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("hlod_trees.wgsl"), source: wgpu::ShaderSource::Wgsl(draw_shader_source(i.fallback).into()) });
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
        let bytes = size + block_bytes + MAX_BLOCKS as u64 * 16 + 16 + 16 + std::mem::size_of::<CullParams>() as u64 + if i.fallback { CAPACITY as u64 * 4 + 16 } else { 0 };
        Self {
            pipeline,
            bg,
            params,
            buf,
            block_buf,
            cull_params,
            cmd_buf,
            count_buf,
            cull_plain,
            cull_hzb,
            cull_bg,
            list,
            count: std::sync::atomic::AtomicU32::new(0),
            blocks: std::sync::atomic::AtomicU32::new(0),
            pending: std::sync::Mutex::new(None),
            bytes,
        }
    }

    /// The tree list (gathered and ordered off the render thread, `into_blocks`); written by `pump`.
    pub fn supply(&self, list: FarTreeList) {
        let n = list.trees.len().min(CAPACITY as usize);
        let nb = list.blocks.iter().take_while(|b| (b.first + b.count) as usize <= n).count().min(MAX_BLOCKS as usize);
        let mut data = bytemuck::cast_slice(&list.trees[..n]).to_vec();
        data.extend_from_slice(bytemuck::cast_slice(&list.blocks[..nb]));
        *self.pending.lock().unwrap() = Some((data, 0, n as u32, nb as u32));
    }

    /// One frame of the list's upload (at most `WRITE_BYTES_PER_FRAME`): the
    /// trees, then the blocks; the trees draw once all of it is written.
    /// Returns the bytes written.
    pub fn pump(&self, queue: &wgpu::Queue) -> u64 {
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
        if let Some((_, dr)) = &self.list {
            enc.clear_buffer(dr, 4, Some(4)); // the kept trees; the vertex count stays 6
        }
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

    /// In the city pass with the opaque city (groups 0 and 1 bound): the blocks the cull kept.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
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
        pass.multi_draw_indirect_count(&self.cmd_buf, 0, &self.count_buf, 0, n);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both paths' draw shaders parse and validate (naga, on the CPU).
    #[test]
    fn the_draw_shader_validates_on_both_paths() {
        use wgpu::naga;
        for fallback in [false, true] {
            let src = draw_shader_source(fallback);
            let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("fallback {fallback}: {}", e.emit_to_string(&src)));
            naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
                .validate(&module)
                .unwrap_or_else(|e| panic!("fallback {fallback}: {}", e.emit_to_string(&src)));
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
            trees.push(FarTree { pos: [x, (i % 7) as f32, z], height: 6.0 + (i % 5) as f32, crown: 2.0, tint: 0, seed: i, pad: 0 });
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
