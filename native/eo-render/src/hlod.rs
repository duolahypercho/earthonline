//! the far city at run time (stage 1: neighbourhood cells; stage 2:
//! districts). The bakes are `hlod_bake.rs` and `hlod_district.rs`.
//!
//! Levels, chosen per building on the GPU (cull.wgsl `lod`; nothing per
//! building on the CPU), nearest first:
//!
//!   near      the building's own levels (kit rung 0 / rung 2, a Blender model)
//!   cell      its 256 m neighbourhood cell's merged mesh and facade atlas
//!   district  its 2,048 m district's merged mesh (flat facade colours)
//!   box       the footprint box, only while none of the others is resident
//!
//! Every switch is a band in which both sides draw with complementary
//! screen-door dither, the farther side's share growing with the distance:
//! a kit mid rung to its cell between the mid band's enter and exit radii
//! (470 / 507.6 m to the building's bounding sphere), a Blender model to its
//! cell between the JS HLOD radii (320 / 380 m), a cell to its district
//! between `far_band` (1,350 / 1,450 m). The share is continuous in distance,
//! so nothing pops whichever way the camera crosses a band, and no extra
//! hysteresis is needed. A building past the mid band is drawn by its cell
//! or its district whenever either is resident; with neither, its mid rung
//! stays drawn as long as it is resident (to the mid pack's 600 m drop), and
//! only then the box: no band of plain boxes between the mid rung and the far
//! layer once the far layer is in.
//!
//! Streaming: cells are loaded all round the camera (ground distance to the
//! cell's footprint rectangle within `load_m`, dropped past `drop_m`), nearest
//! first, on worker threads (the cache, or a bake); districts are made from
//! their cells' summaries once and stay resident. Uploads have their own
//! budget (`upload_bytes` a frame), split by buffer ranges and texture rows.

use crate::authored::AuthoredSet;
use crate::hlod_bake::{self, BakeContext, BakeOptions, BakedCell, BuildingSummary, CellSource, HlodSet, HlodVertex, PAGE};
use crate::hlod_district::{self, District, DistrictMesh};
use crate::kit::{KitSet, RangeAlloc};
use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicUsize, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Entries of the GPU cell table (cells, then districts) and of the per-cell residency words.
pub const MAX_CELLS: u32 = 8192;
/// Mip levels of an atlas page (512 down to 16 texels).
pub const PAGE_MIPS: u32 = 6;
/// Residency word bits (`Renderer::hlod_flags_buf`, one word per cell).
pub const FLAG_CELL: u32 = 1;
pub const FLAG_DISTRICT: u32 = 2;
/// Per group state bits (cull.wgsl `lod` writes them every frame).
pub const STATE_CELL: u32 = 1;
pub const STATE_DISTRICT: u32 = 2;
pub const STATE_NEAR_FADE: u32 = 4;
/// GPU cell-table entry flags.
pub const ENTRY_RESIDENT: u32 = 1;
pub const ENTRY_DISTRICT: u32 = 2;
/// A kit group's level while the far layer draws it (no rung, no box).
pub const HLOD_LEVEL: u32 = 4;
/// Object ids (the debug id target) of far-layer pixels: this | the
/// building's LOD group, so two far buildings meeting at a pixel are told
/// apart (hlod_draw.wgsl `hlod_fs_ids`). Not an object-table index.
pub const FAR_ID_BASE: u32 = 0xC000_0000;

/// How the far layer is drawn (set when it is attached; every shot's
/// manifest names it through `HlodOptions::path`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FarPath {
    /// Not attached to the renderer (yet, or the adapter lacks multi-draw-indirect-count and `--no-far-fallback`).
    NotAttached,
    /// The compacted lists (`multi_draw_indexed_indirect_count`).
    Compact,
    /// Fixed-size lists without the count (`multi_draw_indexed_indirect`; one draw for the trees).
    Fallback,
}

/// Metres the fallback's CPU frustum count grows each box by (hlod_cull.wgsl
/// tests the same boxes with the same planes in f32; the margin covers rounding).
pub const FALLBACK_MARGIN_M: f32 = 1.0;

/// The cull's frustum test (hlod_cull.wgsl), the box grown by `margin`.
pub fn in_frustum(planes: &[[f32; 4]; 6], mn: [f32; 3], mx: [f32; 3], margin: f32) -> bool {
    planes.iter().all(|pl| {
        let v = [0, 1, 2].map(|k| if pl[k] >= 0.0 { mx[k] + margin } else { mn[k] - margin });
        pl[0] * v[0] + pl[1] * v[1] + pl[2] * v[2] + pl[3] >= 0.0
    })
}

/// `FarPath` as attached (0 not attached, 1 compact, 2 fallback).
pub static DRAW_PATH: AtomicU8 = AtomicU8::new(0);

pub fn draw_path() -> FarPath {
    match DRAW_PATH.load(Ordering::Relaxed) {
        1 => FarPath::Compact,
        2 => FarPath::Fallback,
        _ => FarPath::NotAttached,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HlodOptions {
    pub on: bool,
    /// Cells are loaded within this ground distance of the camera to their rectangle, dropped past `drop_m`.
    pub load_m: f32,
    pub drop_m: f32,
    /// The cell -> district band (camera to the building's bounding sphere).
    pub far_band: [f32; 2],
    pub districts: bool,
    /// Stage 3 (the far street lamps) and stage 4 (the distant trees).
    pub lamps: bool,
    pub trees: bool,
    pub threads: usize,
    /// GPU capacities: atlas pages (layers), vertices, indices.
    pub pages: u32,
    pub vertices: u32,
    pub indices: u32,
    pub upload_bytes: u64,
    pub bake: BakeOptions,
    /// a district wall's window rhythm by day (`--no-far-windows`) and the
    /// towers' coated glass (`--no-tower-glass`); the glass seen through by day is
    /// `bake.glass_body` (`--no-far-glass`, which turns all three off).
    pub windows: bool,
    pub tower_glass: bool,
    /// lit windows by night with each building's own activity, offices by floor and tenant (`--no-far-night`),
    /// and a tower's panes tilted panel by panel with the city reflected below the horizon (`--no-tower-panels`).
    pub night: bool,
    pub panels: bool,
    /// a lit office floor seen as an office (`--no-tower-night`, hlod_draw.wgsl
    /// `hl_tower_night`): the ceiling's glow and its fixture rows over the
    /// desks' silhouettes, tenants at their own colour and level, floors
    /// half lit for the cleaners, blinds part down; not one flat tile per window.
    pub tower_night: bool,
    /// a tall tower's crown lit at night (`--no-tower-crowns`, hlod_draw.wgsl
    /// `hl_tower_crown`): floodlit top storeys, some with an LED line at the parapet.
    pub tower_crowns: bool,
    /// How the far layer is drawn (as attached when this was read; not an option).
    pub path: FarPath,
}

impl Default for HlodOptions {
    fn default() -> Self {
        Self {
            on: true,
            load_m: 1500.0,
            drop_m: 1800.0,
            far_band: [1050.0, 1150.0],
            districts: true,
            lamps: true,
            trees: true,
            threads: 6,
            pages: 480,
            // Measured peaks (all districts + the cells within the drop radius): 3.0 M vertices, 9.1 M indices.
            vertices: 4_000_000,
            indices: 12_000_000,
            upload_bytes: 3 << 19,
            bake: BakeOptions::default(),
            windows: true,
            tower_glass: true,
            night: true,
            panels: true,
            tower_night: true,
            tower_crowns: true,
            path: FarPath::NotAttached,
        }
    }
}

impl HlodOptions {
    /// The flags eo-app's parser lets through: (flag, takes a value).
    pub const FLAGS: [(&'static str, bool); 17] = [
        ("--no-hlod", false),
        ("--no-hlod-districts", false),
        ("--no-hlod-lamps", false),
        ("--no-hlod-trees", false),
        ("--hlod-radius", true),
        ("--hlod-texel", true),
        ("--hlod-pages", true),
        ("--hlod-threads", true),
        ("--hlod-upload-mb", true),
        ("--hlod-max-pages", true),
        ("--no-far-glass", false),
        ("--no-far-windows", false),
        ("--no-tower-glass", false),
        ("--no-far-night", false),
        ("--no-tower-panels", false),
        ("--no-tower-night", false),
        ("--no-tower-crowns", false),
    ];

    /// From the command line (see `FLAGS`) and `EO_HLOD=0`.
    pub fn from_env() -> Self {
        let mut o = Self::default();
        o.path = draw_path();
        if std::env::var("EO_HLOD").is_ok_and(|v| v == "0") {
            o.on = false;
        }
        let args: Vec<String> = std::env::args().collect();
        let val = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).and_then(|s| s.parse::<f64>().ok());
        if args.iter().any(|a| a == "--no-hlod") {
            o.on = false;
        }
        if args.iter().any(|a| a == "--no-hlod-districts") {
            o.districts = false;
        }
        if args.iter().any(|a| a == "--no-hlod-lamps") {
            o.lamps = false;
        }
        if args.iter().any(|a| a == "--no-hlod-trees") {
            o.trees = false;
        }
        if args.iter().any(|a| a == "--no-far-glass") {
            o.bake.glass_body = false;
        }
        if args.iter().any(|a| a == "--no-far-windows") {
            o.windows = false;
        }
        if args.iter().any(|a| a == "--no-tower-glass") {
            o.tower_glass = false;
        }
        if args.iter().any(|a| a == "--no-far-night") {
            o.night = false;
        }
        if args.iter().any(|a| a == "--no-tower-panels") {
            o.panels = false;
        }
        if args.iter().any(|a| a == "--no-tower-night") {
            o.tower_night = false;
        }
        if args.iter().any(|a| a == "--no-tower-crowns") {
            o.tower_crowns = false;
        }
        if let Some(r) = val("--hlod-radius") {
            o.load_m = r as f32;
            o.drop_m = r as f32 + 300.0;
            o.far_band = [(r as f32 - 450.0).max(700.0), (r as f32 - 350.0).max(800.0)];
        }
        if let Some(t) = val("--hlod-texel") {
            o.bake.texel_m = t as f32;
        }
        if let Some(p) = val("--hlod-pages") {
            o.pages = p as u32;
        }
        if let Some(p) = val("--hlod-max-pages") {
            o.bake.max_pages = (p as usize).max(1);
        }
        if let Some(t) = val("--hlod-threads") {
            o.threads = (t as usize).max(1);
        }
        if let Some(m) = val("--hlod-upload-mb") {
            o.upload_bytes = (m * 1048576.0) as u64;
        }
        o
    }
}

/// WGSL constants appended to cull.wgsl (lib.rs): whether the far layer is
/// on (off: the LOD pass is exactly what it was) and the cell -> district band.
pub fn cull_constants_wgsl() -> String {
    let o = HlodOptions::from_env();
    format!(
        "\n// hlod\nconst HLOD_ON: bool = {};\nconst HLOD_FAR_A: f32 = {:?};\nconst HLOD_FAR_B: f32 = {:?};\n",
        o.on,
        o.far_band[0],
        o.far_band[1].max(o.far_band[0] + 1.0)
    )
}

/// WGSL constants put before the far shader: the glass seen through by
/// day (the bake's `glass_body`), the district windows, the towers' coated glass.
pub fn far_constants_wgsl(o: &HlodOptions) -> String {
    let glass = o.bake.glass_body;
    format!("
const FL_GLASS: bool = {glass};
const FL_WINDOWS: bool = {};
const FL_TOWER: bool = {};
const FL_NIGHT: bool = {};
const FL_PANELS: bool = {};
const FL_TOWER_NIGHT: bool = {};
const FL_TOWER_CROWNS: bool = {};
", glass && o.windows, glass && o.tower_glass, o.night, glass && o.tower_glass && o.panels, o.night && o.tower_night, o.night && o.tower_crowns)
}

/// One entry of the GPU cell table (48 bytes): a cell or a district.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
pub struct GpuCell {
    pub min: [f32; 3],
    pub layer_base: u32,
    pub max: [f32; 3],
    pub flags: u32,
    pub first_index: u32,
    pub index_count: u32,
    pub base_vertex: i32,
    /// A district chunk: its neighbourhood cell + 1 (0: none), for the cull's skip (hlod_cull.wgsl).
    pub cell: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CullParams {
    planes: [[f32; 4]; 6],
    view_proj: [[f32; 4]; 4],
    counts: [u32; 4],
    /// The camera, and in w the cell -> district band's start (a district chunk wholly nearer, its cell resident, draws nothing).
    cam: [f32; 4],
    /// the bounded list's guards (bounds.rs): the widened frustum, and x the ring's reach, y 1 to count them, z the near sphere.
    wide: [[f32; 4]; 6],
    guard: [f32; 4],
}

// ---------------------------------------------------------------------------
// GPU side (owned by the renderer: `Renderer::attach_hlod`).
// ---------------------------------------------------------------------------

pub struct HlodInit<'a> {
    pub device: &'a wgpu::Device,
    pub draw_bgl: &'a wgpu::BindGroupLayout,
    pub main_bgl: &'a wgpu::BindGroupLayout,
    pub late_bgl: &'a wgpu::BindGroupLayout,
    /// The per-group state the LOD pass writes (`Renderer::hlod_state_buf`).
    pub state_buf: &'a wgpu::Buffer,
    /// The per-cell residency words (`Renderer::hlod_flags_buf`).
    pub flags_buf: &'a wgpu::Buffer,
    pub sample_count: u32,
    pub debug_ids: bool,
    pub opts: HlodOptions,
    /// No multi-draw-indirect-count: a fixed-size list (`draw`).
    pub fallback: bool,
}

pub struct HlodGpu {
    /// Per LOD group its building's lowest and highest drawn point (world y),
    /// for the crowns (`--tower-crowns`); written as its far meshes arrive.
    pub top_buf: wgpu::Buffer,
    pub vertex_buf: wgpu::Buffer,
    pub index_buf: wgpu::Buffer,
    pub cell_buf: wgpu::Buffer,
    pub pages: wgpu::Texture,
    params_buf: wgpu::Buffer,
    cmd_buf: wgpu::Buffer,
    count_buf: wgpu::Buffer,
    /// The list in the count emulation (no MULTI_DRAW_INDIRECT_COUNT), registered at attach.
    counted: std::sync::OnceLock<crate::counted::CountedLists>,
    cull_plain: wgpu::ComputePipeline,
    cull_hzb: wgpu::ComputePipeline,
    cull_bg: wgpu::BindGroup,
    draw_pipeline: wgpu::RenderPipeline,
    draw_bg: wgpu::BindGroup,
    pub vertex_capacity: u32,
    pub index_capacity: u32,
    pub page_capacity: u32,
    /// VRAM allocated: vertex, index, page array, the rest (tables, lists).
    pub bytes: [u64; 4],
    /// The cell -> district band's start (`HlodOptions::far_band`).
    far_a: f32,
    /// The cull drops the district chunks the resident cells draw instead (off: tests).
    chunk_skip: AtomicBool,
    /// Without multi-draw-indirect-count: the cull still appends its commands,
    /// and the pass draws the first `live` slots (zeroed each frame) without a count.
    fallback: bool,
    /// Table entries the cull can keep (resident, with indices).
    live: AtomicU32,
    /// Each entry's box while the cull can keep it (the CPU mirror `live` counts).
    live_set: Mutex<Vec<Option<([f32; 3], [f32; 3])>>>,
    /// Of those, the ones inside this frame's frustum (a box test as the
    /// cull's, its boxes grown by `FALLBACK_MARGIN_M`): no frame has more commands.
    frame_live: AtomicU32,
    device: wgpu::Device,
}

pub const CULL_WGSL: &str = include_str!("hlod_cull.wgsl");
pub const DRAW_WGSL: &str = include_str!("hlod_draw.wgsl");

/// The far layer's draw as the pipelines compose it: `DRAW_WGSL`, and with
/// the far facades compiled in (`--far-facades`, look.rs `ShaderFeatures`)
/// the flat varying that carries an untextured wall's first corner and the
/// cell's lowest point (hlod_draw.wgsl `fb_facade`); with the far relief
/// (`--far-relief`) the varying that carries a district roof's distances to
/// its footprint's box (hlod_district.rs `roof_frame_words`, hlod_draw.wgsl
/// `fr_roof`); off, the source as before.
pub fn draw_wgsl() -> String {
    let f = crate::look::shader_features();
    let mut s = DRAW_WGSL.to_string();
    if f.far_facades {
        s = s
            .replace("    // @ff-corner-field\n", "    @location(12) @interpolate(flat) ff_corner: vec4<f32>,\n")
            .replace("    // @ff-corner-out\n", "    out.ff_corner = vec4<f32>(pos, hl_cells_d[entry].min.y);\n")
            .replace("    return vec4<f32>(in.world, in.world.y - 10.0); // @ff-corner-read\n", "    return in.ff_corner;\n");
    }
    if f.far_relief {
        // (`--far-relief`) a district roof ring's distances to its footprint's box (hlod_district.rs
        // `roof_frame_words`), in metres at the ring's step; a cell's or a wall's are never read.
        s = s
            .replace("    // @fr-roof-field\n", "    @location(13) fr_d: vec4<f32>,\n")
            .replace(
                "    // @fr-roof-out\n",
                "    out.fr_d = select(vec4<f32>(-1.0), vec4<f32>(f32(nrm & 255u), f32((nrm >> 8u) & 255u), f32((nrm >> 16u) & 255u), f32(nrm >> 24u)) * (0.25 * f32(1u << ((uvw >> 27u) & 3u))), district);\n",
            )
            .replace("    return vec4<f32>(-1.0); // @fr-roof-read\n", "    return in.fr_d;\n");
    }
    s
}

/// The pages are BC3 (set by `HlodGpu::new` from the device's features; the
/// workers compress for it).
pub static PAGES_BC: AtomicBool = AtomicBool::new(false);

/// Bytes of one atlas page with its mips (BC3: a quarter of RGBA8).
pub fn page_bytes() -> u64 {
    let rgba: u64 = (0..PAGE_MIPS).map(|m| ((PAGE as u64) >> m).pow(2) * 4).sum();
    if PAGES_BC.load(Ordering::Relaxed) { rgba / 4 } else { rgba }
}

/// Bytes of one row of a page mip as written: a pixel row, or (BC3) a row of 4 x 4 blocks.
fn page_row_bytes(mip: u32) -> u64 {
    let r = ((PAGE as u64) >> mip).max(1);
    if PAGES_BC.load(Ordering::Relaxed) { (r / 4).max(1) * 16 } else { r * 4 }
}

impl HlodGpu {
    pub fn new(i: &HlodInit) -> Self {
        let d = i.device;
        let o = &i.opts;
        let buf = |label: &str, size: u64, usage: wgpu::BufferUsages| d.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size.max(16).next_multiple_of(4), usage, mapped_at_creation: false });
        use wgpu::BufferUsages as U;
        let vstride = std::mem::size_of::<HlodVertex>() as u64;
        let vertex_buf = buf("hlod-vertices", o.vertices as u64 * vstride, U::VERTEX | U::COPY_DST);
        let index_buf = buf("hlod-indices", o.indices as u64 * 4, U::INDEX | U::COPY_DST);
        let cell_buf = buf("hlod-cells", MAX_CELLS as u64 * std::mem::size_of::<GpuCell>() as u64, U::STORAGE | U::COPY_DST);
        let params_buf = buf("hlod-cull-params", std::mem::size_of::<CullParams>() as u64, U::UNIFORM | U::COPY_DST);
        // COPY_DST: the count emulation, the bounded draws and the fallback (`clear`) zero its slots
        let cmd_buf = buf("hlod-draws", MAX_CELLS as u64 * 20, U::STORAGE | U::INDIRECT | U::COPY_DST);
        // [0] commands, [1] triangles, [2] in the frustum, [3] occluded, [4] [5] the bounded list's guards
        let count_buf = buf("hlod-counts", 32, U::STORAGE | U::INDIRECT | U::COPY_SRC | U::COPY_DST);
        let layers = o.pages.max(1);
        let bc = d.features().contains(wgpu::Features::TEXTURE_COMPRESSION_BC);
        PAGES_BC.store(bc, Ordering::Relaxed);
        let pages = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("hlod-pages"),
            size: wgpu::Extent3d { width: PAGE as u32, height: PAGE as u32, depth_or_array_layers: layers },
            mip_level_count: PAGE_MIPS,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: if bc { wgpu::TextureFormat::Bc3RgbaUnormSrgb } else { wgpu::TextureFormat::Rgba8UnormSrgb },
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let sampler = d.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("hlod-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        // Cull: group 0 its own, group 1 the pyramid (the hzb variant).
        let cs = wgpu::ShaderStages::COMPUTE;
        let entry = |binding, visibility, ty| wgpu::BindGroupLayoutEntry { binding, visibility, ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None }, count: None };
        let st = |read_only| wgpu::BufferBindingType::Storage { read_only };
        let cull_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hlod-cull-bgl"),
            entries: &[entry(0, cs, wgpu::BufferBindingType::Uniform), entry(1, cs, st(true)), entry(2, cs, st(false)), entry(3, cs, st(false)), entry(4, cs, st(true))],
        });
        let cull_bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hlod-cull-bg"),
            layout: &cull_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: params_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: cell_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: cmd_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: count_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: i.flags_buf.as_entire_binding() },
            ],
        });
        let plain_src = format!("{CULL_WGSL}\nfn hl_occluded(mn: vec3<f32>, mx: vec3<f32>) -> bool {{ return false; }}\n");
        let hzb_src = format!(
            "{CULL_WGSL}\nstruct HzbInfo {{ screen: vec4<f32> }};\n@group(1) @binding(0) var hzb: texture_2d<f32>;\n@group(1) @binding(1) var<uniform> hzb_info: HzbInfo;\n{}\nfn hl_occluded(mn: vec3<f32>, mx: vec3<f32>) -> bool {{ return hzb_occluded(mn, mx, hc.view_proj, hzb_info.screen.xy); }}\n",
            crate::occlusion::OCCLUSION_WGSL
        );
        let module = |label: &str, src: String| d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(src.into()) });
        let (pm, hm) = (module("hlod_cull.wgsl", plain_src), module("hlod_cull.wgsl (hzb)", hzb_src));
        let lay_p = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("hlod-cull-layout"), bind_group_layouts: &[Some(&cull_bgl)], immediate_size: 0 });
        let lay_h = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("hlod-cull-hzb-layout"), bind_group_layouts: &[Some(&cull_bgl), Some(i.late_bgl)], immediate_size: 0 });
        let compute = |m: &wgpu::ShaderModule, l: &wgpu::PipelineLayout, label: &str| {
            d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(label), layout: Some(l), module: m, entry_point: Some("hlod_cull"), compilation_options: Default::default(), cache: None })
        };
        let cull_plain = compute(&pm, &lay_p, "hlod_cull");
        DRAW_PATH.store(if i.fallback { 2 } else { 1 }, Ordering::Relaxed);
        let cull_hzb = compute(&hm, &lay_h, "hlod_cull (hzb)");
        // Draw: groups 0 and 1 the city's, group 2 the far layer's.
        let vs = wgpu::ShaderStages::VERTEX;
        let fs = wgpu::ShaderStages::FRAGMENT;
        let top_buf = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hlod-group-top"),
            size: (i.state_buf.size() * 2).max(8),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let draw_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hlod-draw-bgl"),
            entries: &[
                entry(0, vs | fs, st(true)),
                entry(1, vs, st(true)),
                entry(4, fs, st(true)),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: fs,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 3, visibility: fs, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let page_view = pages.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        let draw_bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hlod-draw-bg"),
            layout: &draw_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: cell_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: i.state_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&page_view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&sampler) },
                wgpu::BindGroupEntry { binding: 4, resource: top_buf.as_entire_binding() },
            ],
        });
        let dm = module("hlod_draw.wgsl", format!("{}\n{}{}", crate::look::city_shader_source_far(), far_constants_wgsl(&i.opts), draw_wgsl())); // + the switches // the far layer's city shader (lighting: the ambient plane only)
        let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("hlod-draw-layout"), bind_group_layouts: &[Some(i.draw_bgl), Some(i.main_bgl), Some(&draw_bgl)], immediate_size: 0 });
        let attrs = [
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Uint32, offset: 12, shader_location: 1 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Uint32, offset: 16, shader_location: 2 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Uint32, offset: 20, shader_location: 3 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Uint32, offset: 24, shader_location: 4 },
        ];
        let color = wgpu::ColorTargetState { format: crate::look::HDR_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL };
        let ids = wgpu::ColorTargetState { format: crate::IDS_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL };
        let targets_ids = [Some(color.clone()), Some(ids)];
        let targets = [Some(color)];
        let draw_pipeline = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("hlod-draw"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &dm,
                entry_point: Some("hlod_vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout { array_stride: vstride, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attrs })],
            },
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
                module: &dm,
                entry_point: Some(if i.debug_ids { "hlod_fs_ids" } else { "hlod_fs" }),
                compilation_options: Default::default(),
                targets: if i.debug_ids { &targets_ids } else { &targets },
            }),
            multiview_mask: None,
            cache: None,
        });
        let bytes = [vertex_buf.size(), index_buf.size(), page_bytes() * layers as u64, cell_buf.size() + cmd_buf.size() + count_buf.size() + params_buf.size()];
        Self {
            vertex_buf,
            index_buf,
            top_buf,
            cell_buf,
            pages,
            params_buf,
            cmd_buf,
            count_buf,
            counted: std::sync::OnceLock::new(),
            cull_plain,
            cull_hzb,
            cull_bg,
            draw_pipeline,
            draw_bg,
            vertex_capacity: o.vertices,
            index_capacity: o.indices,
            page_capacity: layers,
            bytes,
            far_a: o.far_band[0],
            chunk_skip: AtomicBool::new(true),
            fallback: i.fallback,
            live: AtomicU32::new(0),
            live_set: Mutex::new(vec![None; MAX_CELLS as usize]),
            frame_live: AtomicU32::new(0),
            device: d.clone(),
        }
    }

    /// The most entries the cull can draw this frame: every entry written
    /// resident (with indices) so far, plus a margin for entries written
    /// after the encode and before its submit, in whole 64s.
    pub fn resident_bound(&self) -> u32 {
        (self.live_entries() + 64).next_multiple_of(64).min(MAX_CELLS)
    }

    pub fn total_bytes(&self) -> u64 {
        self.bytes.iter().sum()
    }

    /// The frame's camera (the city pass's jittered clip matrix and its planes).
    pub fn update(&self, queue: &wgpu::Queue, view_proj: glam::Mat4, planes: [[f32; 4]; 6], eye: Vec3) {
        let p = CullParams { planes, view_proj: view_proj.to_cols_array_2d(), counts: [MAX_CELLS, 0, 0, 0], cam: [eye.x, eye.y, eye.z, if self.chunk_skip.load(Ordering::Relaxed) { self.far_a } else { -1.0 }], wide: [[0.0; 4]; 6], guard: [0.0; 4] };
        queue.write_buffer(&self.params_buf, 0, bytemuck::bytes_of(&p));
        if self.fallback {
            // the commands the cull can append this frame: the kept entries in
            // the frustum (boxes grown, so the count is never below the GPU's).
            let set = self.live_set.lock().unwrap();
            let n = set.iter().flatten().filter(|(mn, mx)| in_frustum(&planes, *mn, *mx, FALLBACK_MARGIN_M)).count();
            self.frame_live.store(n as u32, Ordering::Relaxed);
        }
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
        queue.write_buffer(&self.params_buf, at, bytemuck::cast_slice(&words));
    }

    /// The cull's counters: [0] commands, [1] triangles, [2] in the frustum, [3] occluded, [4] [5] the bounded list's guards.
    pub fn counter_buffer(&self) -> &wgpu::Buffer {
        &self.count_buf
    }

    /// `bound`: the bounded list's commands, emptied before the cull appends (bounds.rs).
    pub fn clear(&self, enc: &mut wgpu::CommandEncoder, bound: Option<u32>) {
        enc.clear_buffer(&self.count_buf, 0, None);
        let live = self.frame_bound() as u64;
        if self.fallback && live > 0 {
            // The slots the cull leaves empty draw nothing (index count 0).
            enc.clear_buffer(&self.cmd_buf, 0, Some(live * 20));
        }
        if let Some(b) = bound.filter(|_| !self.fallback) {
            crate::clear_list(enc, &self.cmd_buf, 0, b, crate::DRAW_ARGS_BYTES);
        }
    }

    /// The fallback's slots this frame: the count, at most the list's length.
    fn frame_bound(&self) -> u32 {
        self.frame_live.load(Ordering::Relaxed).min(MAX_CELLS)
    }

    /// Commands the fallback draws a frame (the entries the cull can keep).
    pub fn live_entries(&self) -> u32 {
        self.live.load(Ordering::Relaxed)
    }

    /// Put the cell list (count word 0) in the count emulation.
    pub fn register_counts(&self, counts: &std::sync::Arc<crate::counted::Counts>) {
        let list = crate::counted::List { word: 0, draws: self.cmd_buf.clone(), offset: 0, stride: crate::DRAW_ARGS_BYTES, max: MAX_CELLS, floor: None };
        let _ = self.counted.set(counts.register("far-cells", &self.count_buf, vec![list]));
    }

    /// Whether the cull drops the district chunks the resident cells draw
    /// instead (on; off only to prove it changes no pixel: the occlusion check).
    pub fn set_chunk_skip(&self, on: bool) {
        self.chunk_skip.store(on, Ordering::Relaxed);
    }

    /// Frustum (and, after the pyramid, occlusion) test of every resident entry.
    pub fn encode_cull(&self, pass: &mut wgpu::ComputePass<'_>, hzb: Option<&wgpu::BindGroup>) {
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
        pass.dispatch_workgroups(MAX_CELLS.div_ceil(64), 1, 1);
    }

    /// In the city pass (its groups 0 and 1 bound): every entry the cull kept
    /// (`bound`: the bounded draws' commands, bounds.rs; None: the count
    /// emulation's bound or the indirect count).
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, bound: Option<u32>) {
        pass.set_pipeline(&self.draw_pipeline);
        pass.set_bind_group(2, &self.draw_bg, &[]);
        pass.set_vertex_buffer(0, self.vertex_buf.slice(..));
        pass.set_index_buffer(self.index_buf.slice(..), wgpu::IndexFormat::Uint32);
        if self.fallback {
            // No count on the GPU: the cull's commands are compacted to the
            // front of the list, and every slot up to this frame's bound is drawn (the rest zeroed).
            let live = self.frame_bound();
            if live > 0 {
                pass.multi_draw_indexed_indirect(&self.cmd_buf, 0, live);
            }
            return;
        }
        // (the bounded draws' `bound`, else the count emulation's when `register_counts` put the list in it)
        crate::draw_compacted(pass, bound, self.counted.get(), 0, &self.cmd_buf, 0, &self.count_buf, 0, MAX_CELLS);
    }

    /// The last frame's counters (blocking; tests and bench reports): entries
    /// drawn, triangles submitted, entries in the frustum, entries occluded.
    pub fn readback(&self, queue: &wgpu::Queue) -> [u32; 4] {
        let rb = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("hlod-count-rb"), size: 16, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(&self.count_buf, 0, &rb, 0, 16);
        queue.submit([enc.finish()]);
        let slice = rb.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        let Ok(view) = slice.get_mapped_range() else { return [0; 4] };
        let v: Vec<u32> = bytemuck::pod_collect_to_vec(&view);
        [v[0], v[1], v[2], v[3]]
    }

    pub fn write_entry(&self, queue: &wgpu::Queue, entry: u32, c: &GpuCell) {
        queue.write_buffer(&self.cell_buf, entry as u64 * std::mem::size_of::<GpuCell>() as u64, bytemuck::bytes_of(c));
        // the cull keeps a resident entry with indices (hlod_cull.wgsl)
        let keep = (c.flags & ENTRY_RESIDENT != 0 && c.index_count > 0).then_some((c.min, c.max));
        let mut set = self.live_set.lock().unwrap();
        if let Some(was) = set.get_mut(entry as usize) {
            match (was.is_some(), keep.is_some()) {
                (false, true) => {
                    self.live.fetch_add(1, Ordering::Relaxed);
                    // written after this frame's count: counted until the next `update`
                    self.frame_live.fetch_add(1, Ordering::Relaxed);
                }
                (true, false) => {
                    self.live.fetch_sub(1, Ordering::Relaxed);
                }
                (true, true) if *was != keep => {
                    // a kept entry with a new box: this frame's count used the
                    // old one, so count it again (an extra slot is zeroed)
                    self.frame_live.fetch_add(1, Ordering::Relaxed);
                }
                _ => {}
            }
            *was = keep;
        }
    }
}

// ---------------------------------------------------------------------------
// Run time: requests, bakes, uploads, residency.
// ---------------------------------------------------------------------------

enum Msg {
    /// The cell without its geometry and pages (moved into upload bytes on the worker).
    Cell { entry: u32, cell: Box<BakedCell>, source: CellSource, ms: f64, counts: (u32, u32), vbytes: Vec<u8>, ibytes: Vec<u8>, mips: Vec<Vec<Vec<u8>>> },
    /// A district without its geometry (moved into upload bytes on the worker).
    District { index: usize, mesh: Box<DistrictMesh>, ms: f64, counts: (u32, u32), vbytes: Vec<u8>, ibytes: Vec<u8>, chunks: Vec<hlod_district::DistrictChunk> },
}

#[derive(Clone, Copy, Debug)]
struct Alloc {
    vertex: u32,
    vertices: u32,
    index: u32,
    indices: u32,
    page: u32,
    pages: u32,
}

enum Target {
    Vertex,
    Index,
    Page { layer: u32, mip: u32 },
}

struct HWrite {
    target: Target,
    offset: u64,
    data: Vec<u8>,
    /// Bytes of `data` already written.
    pos: usize,
}

struct Upload {
    entry: u32,
    district: Option<usize>,
    alloc: Alloc,
    /// The table entries written once its data is: the cell's, or the district's chunks.
    records: Vec<(u32, GpuCell)>,
    writes: VecDeque<HWrite>,
}

enum Slot {
    Requested,
    Uploading,
    Resident(Alloc),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DState {
    Idle,
    Requested,
    Uploading,
    Resident,
    Failed,
}

#[derive(Clone, Debug, Default)]
pub struct HlodStats {
    pub cells: usize,
    pub districts: usize,
    pub resident_cells: usize,
    pub resident_districts: usize,
    pub from_cache: usize,
    pub baked: usize,
    pub evicted: usize,
    pub skipped_no_space: usize,
    pub bake_ms_sum: f64,
    pub load_ms_sum: f64,
    pub district_ms_sum: f64,
    pub bytes_uploaded: u64,
    pub max_frame_bytes: u64,
    pub resident_vertices: u64,
    pub resident_indices: u64,
    pub resident_pages: u64,
    pub district_vertices: u64,
    pub district_indices: u64,
    pub peak_vertices: u64,
    pub peak_indices: u64,
    pub peak_pages: u64,
    pub texel_m_min: f32,
    pub texel_m_max: f32,
    pub buildings_loaded: u64,
    pub fallback_buildings: u64,
    pub district_buildings: u64,
    pub first_resident_ms: Option<f64>,
    pub inputs_ready_ms: Option<f64>,
    pub districts_ready_ms: Option<f64>,
    pub key: Option<String>,
    /// The render thread's time in `update` (requests, arrivals, uploads).
    pub update_ms_max: f64,
    pub update_ms_sum: f64,
    pub updates: u64,
}

/// The optional shore material is registered after all building materials.
/// Exclude its row from the cache identity, keeping the live palette indexed
/// exactly as the GPU sees it. Landmark colours after that row keep their order.
fn building_cache_key(
    kit_key: &str,
    colours: &hlod_bake::MaterialColours,
    authored: &AuthoredSet,
    bake: &hlod_bake::BakeOptions,
) -> String {
    let building_colours = authored.materials.by_name(crate::seabed::rock_maps::NAME).map(|rock| {
        let mut palette = colours.clone();
        palette.rgb.remove(rock as usize);
        palette.glass.remove(rock as usize);
        palette
    });
    hlod_bake::inputs_key(kit_key, building_colours.as_ref().unwrap_or(colours), authored, bake)
}

#[cfg(test)]
mod building_cache_tests {
    use super::*;
    use crate::authored::GpuMaterial;

    #[test]
    fn shoreline_scan_reuses_the_city_cache_and_retains_building_colour_changes() {
        let key = |rock: bool, red: f32| {
            let mut authored = AuthoredSet::empty();
            let building = GpuMaterial { base: [red, 0.4, 0.3, 1.0], ..GpuMaterial::vertex_colour() };
            authored.materials.named("building", building);
            if rock {
                authored.materials.named(crate::seabed::rock_maps::NAME, GpuMaterial::vertex_colour());
            }
            authored.materials.named("landmark", GpuMaterial::vertex_colour());
            let palette = hlod_bake::MaterialColours::new(&authored.materials.gpu, &[]);
            building_cache_key("city", &palette, &authored, &hlod_bake::BakeOptions::default())
        };
        assert_eq!(key(false, 0.2), key(true, 0.2));
        assert_ne!(key(true, 0.2), key(true, 0.3));
    }
}

/// What the bakes read, apart from the planner's city and the material colours (which arrive later).
pub struct HlodInputs {
    pub set: Arc<HlodSet>,
    pub kit: Arc<KitSet>,
    pub authored: Arc<AuthoredSet>,
    pub city: Arc<eo_data::City>,
    pub repo_root: PathBuf,
    pub cache_dir: Option<PathBuf>,
}

type CityRx = Receiver<Result<(Arc<eo_kit::city::KitCity>, String), String>>;

pub struct HlodRuntime {
    pub set: Arc<HlodSet>,
    pub opts: HlodOptions,
    ctx_slot: Arc<Mutex<Option<Arc<BakeContext>>>>,
    means_tx: Option<std::sync::mpsc::Sender<Vec<[f32; 3]>>>,
    pending: Arc<Mutex<Vec<u32>>>,
    pending_districts: Arc<Mutex<Vec<usize>>>,
    camera: Arc<Mutex<Vec3>>,
    rx: Receiver<Msg>,
    stop: Arc<AtomicBool>,
    in_flight: Arc<AtomicUsize>,
    threads: Vec<std::thread::JoinHandle<()>>,
    slots: HashMap<u32, Slot>,
    vtx: RangeAlloc,
    idx: RangeAlloc,
    pgs: RangeAlloc,
    uploads: VecDeque<Upload>,
    /// CPU mirror of the per-cell residency words.
    flags: Vec<u32>,
    flags_dirty: bool,
    pub districts: Arc<Vec<District>>,
    dstate: Vec<DState>,
    districts_requested: bool,
    started: Instant,
    pub stats: HlodStats,
    no_space: HashSet<u32>,
    /// each LOD group's drawn span so far (`write_group_tops`).
    group_tops: HashMap<u32, [f32; 2]>,
}

/// Each building's lowest and highest drawn point in a far mesh's vertices
/// (`HlodVertex`: 28 bytes, y at 4, the group at 24), written into the
/// per-group table the crowns read (`HlodGpu::top_buf`): the geometry's own
/// top, so a crown sits at the roofline the far layer draws.
/// `seen` keeps each group's widest span over every mesh that drew it (a
/// district's simplified form is lower than its cell's: the union is written).
fn write_group_tops(queue: &wgpu::Queue, top_buf: &wgpu::Buffer, seen: &mut HashMap<u32, [f32; 2]>, vbytes: &[u8]) {
    let mut span: HashMap<u32, [f32; 2]> = HashMap::new();
    for v in vbytes.chunks_exact(std::mem::size_of::<HlodVertex>()) {
        let y = f32::from_le_bytes([v[4], v[5], v[6], v[7]]);
        let g = u32::from_le_bytes([v[24], v[25], v[26], v[27]]);
        let e = span.entry(g).or_insert([f32::MAX, f32::MIN]);
        *e = [e[0].min(y), e[1].max(y)];
    }
    let cap = top_buf.size() / 8;
    for (g, t) in span {
        let e = seen.entry(g).or_insert(t);
        let u = [e[0].min(t[0]), e[1].max(t[1])];
        if (g as u64) < cap && (u != *e || u == t) {
            queue.write_buffer(top_buf, g as u64 * 8, bytemuck::cast_slice(&u));
        }
        *e = u;
    }
}

impl HlodRuntime {
    /// Start the workers. `kit_city_rx` delivers the planner's city and its
    /// inputs key (the kit tile cache's); the kit maps' layer means come
    /// through `set_layer_means`. Nothing is baked before both are in.
    pub fn start(inp: HlodInputs, kit_city_rx: CityRx, opts: HlodOptions, camera: Vec3, started: Instant) -> Self {
        let set = inp.set.clone();
        let districts = Arc::new(if opts.districts { hlod_district::districts_of(&set) } else { vec![] });
        let ctx_slot: Arc<Mutex<Option<Arc<BakeContext>>>> = Arc::new(Mutex::new(None));
        let (means_tx, means_rx) = std::sync::mpsc::channel::<Vec<[f32; 3]>>();
        {
            let (slot, mut bake) = (ctx_slot.clone(), opts.bake);
            // The pages are baked in the GPU's format (the renderer is made first).
            bake.bc3 = PAGES_BC.load(Ordering::Relaxed);
            std::thread::spawn(move || {
                let Ok(Ok((kit_city, kit_key))) = kit_city_rx.recv() else { return };
                let Ok(means) = means_rx.recv() else { return };
                let colours = hlod_bake::MaterialColours::new(&inp.authored.materials.gpu, &means);
                // the far layer bakes the scaled city; the scale and the ground
                // plan's version are in its key (the kit key carries them too; scale 1: unchanged).
                // + the far roof features (all off: the key as before)
                let key = building_cache_key(&format!("{kit_key}{}{}", eo_data::groundscale::cache_tag(), eo_data::roof_colours::far_cache_tag()), &colours, &inp.authored, &bake);
                *slot.lock().unwrap() = Some(Arc::new(BakeContext {
                    set: inp.set,
                    kit: inp.kit,
                    kit_city,
                    authored: inp.authored,
                    city: inp.city,
                    colours: Arc::new(colours),
                    repo_root: inp.repo_root,
                    cache_dir: inp.cache_dir,
                    key,
                    opts: bake,
                }));
            });
        }
        let (tx, rx) = std::sync::mpsc::channel::<Msg>();
        let pending: Arc<Mutex<Vec<u32>>> = Arc::new(Mutex::new(vec![]));
        let pending_districts: Arc<Mutex<Vec<usize>>> = Arc::new(Mutex::new(vec![]));
        let cam = Arc::new(Mutex::new(camera));
        let stop = Arc::new(AtomicBool::new(false));
        let in_flight = Arc::new(AtomicUsize::new(0));
        let mut threads = vec![];
        for _ in 0..opts.threads.max(1) {
            let (tx, pending, pd, cam, stop, in_flight, slot, set, districts) =
                (tx.clone(), pending.clone(), pending_districts.clone(), cam.clone(), stop.clone(), in_flight.clone(), ctx_slot.clone(), set.clone(), districts.clone());
            threads.push(std::thread::spawn(move || {
                crate::hitch::worker_started(); // below the render thread
                while !stop.load(Ordering::Relaxed) {
                    let ctx = slot.lock().unwrap().clone();
                    let Some(ctx) = ctx else {
                        std::thread::sleep(std::time::Duration::from_millis(20));
                        continue;
                    };
                    let c = *cam.lock().unwrap();
                    // Cells nearest the camera first, then districts.
                    let cell = {
                        let mut p = pending.lock().unwrap();
                        let best = p.iter().enumerate().map(|(k, &e)| (k, set.cell_distance(e as usize, c))).min_by(|a, b| a.1.total_cmp(&b.1));
                        best.map(|(k, _)| {
                            in_flight.fetch_add(1, Ordering::SeqCst);
                            p.swap_remove(k)
                        })
                    };
                    if let Some(entry) = cell {
                        let t = Instant::now();
                        let (mut cell, source) = hlod_bake::load_or_bake(&ctx, entry);
                        // The pages come with their mips, in the GPU's format (baked so).
                        let mips = cell.pages.iter().map(|p| hlod_bake::split_levels(p, cell.bc3).into_iter().map(|l| l.to_vec()).collect()).collect();
                        let counts = (cell.vertices.len() as u32, cell.indices.len() as u32);
                        let vbytes = bytemuck::cast_slice(&cell.vertices).to_vec();
                        let ibytes = bytemuck::cast_slice(&cell.indices).to_vec();
                        cell.vertices = vec![];
                        cell.indices = vec![];
                        cell.pages = vec![];
                        let m = Msg::Cell { entry, cell: Box::new(cell), source, ms: t.elapsed().as_secs_f64() * 1e3, counts, vbytes, ibytes, mips };
                        in_flight.fetch_sub(1, Ordering::SeqCst);
                        if tx.send(m).is_err() {
                            break;
                        }
                        continue;
                    }
                    let district = {
                        let mut p = pd.lock().unwrap();
                        let dist = |k: usize| {
                            let d = &districts[k];
                            let dx = (d.min[0] - c.x).max(0.0).max(c.x - d.max[0]);
                            let dz = (d.min[1] - c.z).max(0.0).max(c.z - d.max[1]);
                            dx * dx + dz * dz
                        };
                        let best = p.iter().enumerate().map(|(k, &d)| (k, dist(d))).min_by(|a, b| a.1.total_cmp(&b.1));
                        best.map(|(k, _)| {
                            in_flight.fetch_add(1, Ordering::SeqCst);
                            p.swap_remove(k)
                        })
                    };
                    let Some(index) = district else {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        continue;
                    };
                    let t = Instant::now();
                    let mut mesh = hlod_district::load_or_build(&ctx, &districts[index]);
                    // One chunk per cell, so the cull drops what the resident cells draw.
                    let chunks = hlod_district::chunk_by_cell(&mut mesh, &set.group_cell, &districts[index].cells);
                    let counts = (mesh.vertices.len() as u32, mesh.indices.len() as u32);
                    let vbytes = bytemuck::cast_slice(&mesh.vertices).to_vec();
                    let ibytes = bytemuck::cast_slice(&mesh.indices).to_vec();
                    mesh.vertices = vec![];
                    mesh.indices = vec![];
                    let m = Msg::District { index, mesh: Box::new(mesh), ms: t.elapsed().as_secs_f64() * 1e3, counts, vbytes, ibytes, chunks };
                    in_flight.fetch_sub(1, Ordering::SeqCst);
                    if tx.send(m).is_err() {
                        break;
                    }
                }
            }));
        }
        let stats = HlodStats { cells: set.cells.len(), districts: districts.len(), texel_m_min: f32::MAX, ..Default::default() };
        let nd = districts.len();
        Self {
            set,
            opts,
            ctx_slot,
            means_tx: Some(means_tx),
            pending,
            pending_districts,
            camera: cam,
            rx,
            stop,
            in_flight,
            threads,
            slots: HashMap::new(),
            vtx: RangeAlloc::new(opts.vertices),
            idx: RangeAlloc::new(opts.indices),
            pgs: RangeAlloc::new(opts.pages),
            uploads: VecDeque::new(),
            flags: vec![0; MAX_CELLS as usize],
            flags_dirty: false,
            districts,
            dstate: vec![DState::Idle; nd],
            districts_requested: false,
            started,
            stats,
            no_space: HashSet::new(),
            group_tops: HashMap::new(),
        }
    }

    /// The kit maps' layer means (from the kit upload, before it goes to the GPU): the material colours.
    pub fn set_layer_means(&mut self, means: Vec<[f32; 3]>) {
        if let Some(tx) = self.means_tx.take() {
            let _ = tx.send(means);
        }
    }

    pub fn wants_layer_means(&self) -> bool {
        self.means_tx.is_some()
    }

    fn ctx(&self) -> Option<Arc<BakeContext>> {
        self.ctx_slot.lock().unwrap().clone()
    }

    /// One frame: requests and evictions around the camera, arrivals into
    /// uploads, at most `upload_bytes` written, residency switched on for what
    /// finished. Per-cell work only, never per building.
    pub fn update(&mut self, renderer: &crate::Renderer, queue: &wgpu::Queue, cam: Vec3) {
        let t = Instant::now();
        self.update_inner(renderer, queue, cam);
        let ms = t.elapsed().as_secs_f64() * 1e3;
        self.stats.update_ms_max = self.stats.update_ms_max.max(ms);
        self.stats.update_ms_sum += ms;
        self.stats.updates += 1;
    }

    fn update_inner(&mut self, renderer: &crate::Renderer, queue: &wgpu::Queue, cam: Vec3) {
        let Some(gpu) = renderer.hlod.get() else { return };
        crate::hitch::try_set(&self.camera, cam);
        if self.stats.inputs_ready_ms.is_none() {
            if let Some(c) = self.ctx() {
                self.stats.inputs_ready_ms = Some(self.started.elapsed().as_secs_f64() * 1e3);
                self.stats.key = Some(c.key.clone());
            }
        }
        if !self.districts_requested && self.stats.inputs_ready_ms.is_some() && !self.districts.is_empty() {
            self.districts_requested = true;
            let all: Vec<usize> = (0..self.districts.len()).filter(|&k| (self.districts[k].entry + self.districts[k].cells.len() as u32) < MAX_CELLS).collect();
            for &k in &all {
                self.dstate[k] = DState::Requested;
            }
            self.pending_districts.lock().unwrap().extend(all);
        }
        // Wanted: every cell within the load radius (kept to the drop radius).
        let n = self.set.cells.len().min(MAX_CELLS as usize);
        let mut wanted = HashSet::new();
        for c in 0..n {
            let d = self.set.cell_distance(c, cam);
            let have = self.slots.contains_key(&(c as u32));
            if d <= self.opts.load_m || (have && d <= self.opts.drop_m) {
                wanted.insert(c as u32);
            }
        }
        // Evict (at most 16 a frame: two flag words and a table entry each) and cancel.
        let gone: Vec<u32> = self.slots.keys().filter(|k| !wanted.contains(k)).copied().take(16).collect();
        for e in gone {
            match self.slots.remove(&e) {
                Some(Slot::Resident(a)) => {
                    // Not resident first; the ranges are reused only by later writes.
                    self.flags[e as usize] &= !FLAG_CELL;
                    self.flags_dirty = true;
                    gpu.write_entry(queue, e, &GpuCell::zeroed());
                    self.account(a, false, false);
                    self.free(a);
                    self.stats.resident_cells -= 1;
                    self.stats.evicted += 1;
                    self.no_space.clear();
                }
                Some(Slot::Uploading) => {
                    if let Some(k) = self.uploads.iter().position(|u| u.entry == e && u.district.is_none()) {
                        let u = self.uploads.remove(k).unwrap();
                        self.free(u.alloc);
                    }
                }
                Some(Slot::Requested) => {
                    self.pending.lock().unwrap().retain(|&p| p != e);
                }
                None => {}
            }
        }
        let mut new = vec![];
        for &e in &wanted {
            if !self.slots.contains_key(&e) && !self.no_space.contains(&e) {
                self.slots.insert(e, Slot::Requested);
                new.push(e);
            }
        }
        if !new.is_empty() {
            self.pending.lock().unwrap().extend(new);
        }
        // Arrivals.
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Cell { entry, cell, source, ms, counts, vbytes, ibytes, mips } => {
                    match source {
                        CellSource::Cache => {
                            self.stats.from_cache += 1;
                            self.stats.load_ms_sum += ms;
                        }
                        CellSource::Baked => {
                            self.stats.baked += 1;
                            self.stats.bake_ms_sum += ms;
                        }
                    }
                    self.stats.buildings_loaded += cell.summaries.len() as u64;
                    self.stats.fallback_buildings += cell.stats.fallback as u64;
                    self.stats.texel_m_min = self.stats.texel_m_min.min(cell.texel_m);
                    self.stats.texel_m_max = self.stats.texel_m_max.max(cell.texel_m);
                    if !matches!(self.slots.get(&entry), Some(Slot::Requested)) {
                        crate::upload::bury(Box::new((cell, vbytes, ibytes, mips)));
                        continue;
                    }
                    write_group_tops(queue, &gpu.top_buf, &mut self.group_tops, &vbytes); // (`--tower-crowns`) each building's drawn top
                    if let Some(u) = self.make_cell_upload(entry, counts, vbytes, ibytes, mips, (cell.min, cell.max)) {
                        self.slots.insert(entry, Slot::Uploading);
                        self.uploads.push_back(u);
                    }
                    crate::upload::bury(cell);
                }
                Msg::District { index, mesh, ms, counts, vbytes, ibytes, chunks } => {
                    self.stats.district_ms_sum += ms;
                    self.stats.district_buildings += mesh.buildings as u64;
                    write_group_tops(queue, &gpu.top_buf, &mut self.group_tops, &vbytes); // (`--tower-crowns`) each building's drawn top
                    match self.make_district_upload(index, &mesh, counts, vbytes, ibytes, &chunks) {
                        Some(u) => {
                            self.dstate[index] = DState::Uploading;
                            self.uploads.push_back(u);
                        }
                        None => self.dstate[index] = DState::Failed,
                    }
                    crate::upload::bury(mesh);
                }
            }
        }
        // Uploads within the budget.
        let mut budget = self.opts.upload_bytes;
        let mut bytes = 0u64;
        while budget > 0 {
            let Some(u) = self.uploads.front_mut() else { break };
            while budget > 0 {
                let Some(w) = u.writes.front_mut() else { break };
                let take = split_len(w, budget);
                let data = &w.data[w.pos..w.pos + take];
                let at = w.offset + w.pos as u64;
                match w.target {
                    Target::Vertex => queue.write_buffer(&gpu.vertex_buf, at, data),
                    Target::Index => queue.write_buffer(&gpu.index_buf, at, data),
                    Target::Page { layer, mip } => {
                        let r = (PAGE as u32 >> mip).max(1);
                        let row = page_row_bytes(mip);
                        // A written row is a pixel row, or (BC3) a row of blocks covering four.
                        let texels = if PAGES_BC.load(Ordering::Relaxed) { 4 } else { 1 };
                        let (y0, rows) = ((at / row) as u32 * texels, (data.len() as u64 / row) as u32);
                        queue.write_texture(
                            wgpu::TexelCopyTextureInfo { texture: &gpu.pages, mip_level: mip, origin: wgpu::Origin3d { x: 0, y: y0, z: layer }, aspect: wgpu::TextureAspect::All },
                            data,
                            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row as u32), rows_per_image: Some(rows) },
                            wgpu::Extent3d { width: r, height: rows * texels, depth_or_array_layers: 1 },
                        );
                    }
                }
                w.pos += take;
                budget = budget.saturating_sub(take as u64);
                bytes += take as u64;
                if w.pos >= w.data.len() {
                    let done = u.writes.pop_front().unwrap();
                    crate::upload::bury(Box::new(done.data)); // a payload is freed off this thread
                }
            }
            if !u.writes.is_empty() {
                break;
            }
            let u = self.uploads.pop_front().unwrap();
            for (e, r) in &u.records {
                gpu.write_entry(queue, *e, r);
            }
            match u.district {
                Some(k) => {
                    for &c in &self.districts[k].cells {
                        if (c as usize) < self.flags.len() {
                            self.flags[c as usize] |= FLAG_DISTRICT;
                        }
                    }
                    self.dstate[k] = DState::Resident;
                    self.stats.resident_districts += 1;
                    self.account(u.alloc, true, true);
                    if self.dstate.iter().all(|s| matches!(s, DState::Resident | DState::Failed)) && self.stats.districts_ready_ms.is_none() {
                        self.stats.districts_ready_ms = Some(self.started.elapsed().as_secs_f64() * 1e3);
                    }
                }
                None => {
                    self.flags[u.entry as usize] |= FLAG_CELL;
                    self.slots.insert(u.entry, Slot::Resident(u.alloc));
                    self.stats.resident_cells += 1;
                    self.account(u.alloc, true, false);
                    if self.stats.first_resident_ms.is_none() {
                        self.stats.first_resident_ms = Some(self.started.elapsed().as_secs_f64() * 1e3);
                    }
                }
            }
            self.flags_dirty = true;
        }
        if self.flags_dirty {
            renderer.write_hlod_flags(queue, &self.flags); // (+ the CPU copy the fixed-slot lists' ranges read)
            bytes += self.flags.len() as u64 * 4;
            self.flags_dirty = false;
        }
        renderer.note_streamed(bytes);
        self.stats.bytes_uploaded += bytes;
        self.stats.max_frame_bytes = self.stats.max_frame_bytes.max(bytes);
    }

    fn free(&mut self, a: Alloc) {
        self.vtx.free(a.vertex, a.vertices);
        self.idx.free(a.index, a.indices);
        self.pgs.free(a.page, a.pages);
    }

    fn account(&mut self, a: Alloc, add: bool, district: bool) {
        let s = &mut self.stats;
        let (v, i, p) = (a.vertices as u64, a.indices as u64, a.pages as u64);
        if district {
            s.district_vertices += v;
            s.district_indices += i;
        } else if add {
            s.resident_vertices += v;
            s.resident_indices += i;
            s.resident_pages += p;
        } else {
            s.resident_vertices -= v;
            s.resident_indices -= i;
            s.resident_pages -= p;
        }
        s.peak_vertices = s.peak_vertices.max(s.resident_vertices);
        s.peak_indices = s.peak_indices.max(s.resident_indices);
        s.peak_pages = s.peak_pages.max(s.resident_pages);
    }

    fn alloc(&mut self, nv: u32, ni: u32, np: u32) -> Option<Alloc> {
        let v = self.vtx.alloc(nv)?;
        let Some(i) = self.idx.alloc(ni) else {
            self.vtx.free(v, nv);
            return None;
        };
        let Some(p) = self.pgs.alloc(np) else {
            self.vtx.free(v, nv);
            self.idx.free(i, ni);
            return None;
        };
        Some(Alloc { vertex: v, vertices: nv, index: i, indices: ni, page: p, pages: np })
    }

    fn make_cell_upload(&mut self, entry: u32, counts: (u32, u32), vbytes: Vec<u8>, ibytes: Vec<u8>, mips: Vec<Vec<Vec<u8>>>, bounds: ([f32; 3], [f32; 3])) -> Option<Upload> {
        let (nv, ni, np) = (counts.0, counts.1, mips.len() as u32);
        let Some(alloc) = self.alloc(nv, ni, np) else {
            // Out of space: its buildings stay at their mid rung, district or box until something is evicted.
            self.slots.remove(&entry);
            self.no_space.insert(entry);
            self.stats.skipped_no_space += 1;
            return None;
        };
        let vs = std::mem::size_of::<HlodVertex>() as u64;
        let mut writes = VecDeque::new();
        writes.push_back(HWrite { target: Target::Vertex, offset: alloc.vertex as u64 * vs, data: vbytes, pos: 0 });
        writes.push_back(HWrite { target: Target::Index, offset: alloc.index as u64 * 4, data: ibytes, pos: 0 });
        for (p, levels) in mips.into_iter().enumerate() {
            for (m, data) in levels.into_iter().enumerate() {
                writes.push_back(HWrite { target: Target::Page { layer: alloc.page + p as u32, mip: m as u32 }, offset: 0, data, pos: 0 });
            }
        }
        let record = GpuCell { min: bounds.0, layer_base: alloc.page, max: bounds.1, flags: ENTRY_RESIDENT, first_index: alloc.index, index_count: ni, base_vertex: alloc.vertex as i32, cell: 0 };
        Some(Upload { entry, district: None, alloc, records: vec![(entry, record)], writes })
    }

    fn make_district_upload(&mut self, index: usize, _d: &DistrictMesh, counts: (u32, u32), vbytes: Vec<u8>, ibytes: Vec<u8>, chunks: &[hlod_district::DistrictChunk]) -> Option<Upload> {
        let (nv, ni) = counts;
        if ni == 0 {
            return None;
        }
        let alloc = self.alloc(nv, ni, 0)?;
        let vs = std::mem::size_of::<HlodVertex>() as u64;
        let mut writes = VecDeque::new();
        writes.push_back(HWrite { target: Target::Vertex, offset: alloc.vertex as u64 * vs, data: vbytes, pos: 0 });
        writes.push_back(HWrite { target: Target::Index, offset: alloc.index as u64 * 4, data: ibytes, pos: 0 });
        let base = self.districts[index].entry;
        let records = chunks
            .iter()
            .map(|c| {
                let r = GpuCell { min: c.min, layer_base: 0, max: c.max, flags: ENTRY_RESIDENT | ENTRY_DISTRICT, first_index: alloc.index + c.first_index, index_count: c.index_count, base_vertex: alloc.vertex as i32, cell: c.cell.map_or(0, |k| k + 1) };
                (base + c.slot, r)
            })
            .collect();
        Some(Upload { entry: base, district: Some(index), alloc, records, writes })
    }

    /// Every cell wanted around the camera resident and every district built
    /// and resident (nothing requested, baking or uploading).
    pub fn settled(&self) -> bool {
        if self.ctx().is_none() {
            return false;
        }
        self.pending.lock().unwrap().is_empty()
            && self.pending_districts.lock().unwrap().is_empty()
            && self.in_flight.load(Ordering::SeqCst) == 0
            && self.uploads.is_empty()
            && self.slots.values().all(|s| matches!(s, Slot::Resident(_)))
            && (self.districts.is_empty() || self.districts_requested)
            && self.dstate.iter().all(|s| matches!(s, DState::Resident | DState::Failed | DState::Idle))
    }

    /// Why `settled` is false (for a settle that timed out).
    pub fn unsettled(&self) -> String {
        let count = |s: DState| self.dstate.iter().filter(|&&d| d == s).count();
        format!(
            "far layer: inputs {}, cells pending {}, districts pending {}, in flight {}, uploads {}, cells wanted {} resident {}, districts resident {} of {} (requested {}; idle {}, queued {}, uploading {}, failed {})",
            self.ctx().is_some(),
            self.pending.lock().unwrap().len(),
            self.pending_districts.lock().unwrap().len(),
            self.in_flight.load(Ordering::SeqCst),
            self.uploads.len(),
            self.slots.len(),
            self.stats.resident_cells,
            self.stats.resident_districts,
            self.districts.len(),
            self.districts_requested,
            count(DState::Idle),
            count(DState::Requested),
            count(DState::Uploading),
            count(DState::Failed),
        )
    }

    /// the bake's inputs once they are in (the far-colour diagnosis reads them).
    pub fn bake_context(&self) -> Option<Arc<BakeContext>> {
        self.ctx_slot.lock().ok()?.clone()
    }

    pub fn stats_json(&self) -> serde_json::Value {
        let s = &self.stats;
        let o = &self.opts;
        let vs = std::mem::size_of::<HlodVertex>() as u64;
        // (in parts: one `json!` this size passes the macro's recursion limit)
        let memory = serde_json::json!({
            "resident_cells_memory": { "vertices": s.resident_vertices, "triangles": s.resident_indices / 3, "pages": s.resident_pages,
                "vertex_bytes": s.resident_vertices * vs, "index_bytes": s.resident_indices * 4, "page_bytes": s.resident_pages * page_bytes() },
            "peak_cells_memory": { "vertex_bytes": s.peak_vertices * vs, "index_bytes": s.peak_indices * 4, "page_bytes": s.peak_pages * page_bytes(), "pages": s.peak_pages },
            "districts_memory": { "vertices": s.district_vertices, "triangles": s.district_indices / 3, "vertex_bytes": s.district_vertices * vs, "index_bytes": s.district_indices * 4 },
            "capacity": { "vertices": o.vertices, "indices": o.indices, "pages": o.pages },
        });
        let set = serde_json::json!({ "covered": self.set.covered, "kit": self.set.by_source[0], "blender": self.set.by_source[1], "code_built": self.set.by_source[2],
            "skipped_claimed_by_landmarks": self.set.skipped_claimed, "skipped_degenerate": self.set.skipped_degenerate, "unmatched_plain": self.set.unmatched_plain });
        let far_look = serde_json::json!({ "glass_body": o.bake.glass_body, "windows": o.windows, "tower_glass": o.tower_glass, "night": o.night, "panels": o.panels }); // night, panels
        let bake = serde_json::json!({ "texel_m": o.bake.texel_m, "max_pages_per_cell": o.bake.max_pages, "party_wall_texel_scale": o.bake.party_scale, "page": PAGE, "pad": hlod_bake::PAD, "mips": PAGE_MIPS, "pages_bc3": PAGES_BC.load(Ordering::Relaxed) });
        let mut v = serde_json::json!({
            "on": o.on,
            "cell_m": hlod_bake::CELL_M,
            "district_m": hlod_bake::CELL_M * hlod_district::DISTRICT_CELLS as f64,
            "cells": s.cells, "districts": s.districts,
            "load_drop_m": [o.load_m, o.drop_m],
            "cell_to_district_band_m": o.far_band,
            "resident_cells": s.resident_cells, "resident_districts": s.resident_districts, "wanted_cells": self.slots.len(),
            "cells_from_cache": s.from_cache, "cells_baked": s.baked, "cells_evicted": s.evicted, "cells_skipped_no_space": s.skipped_no_space,
            "cell_bake_ms_sum": s.bake_ms_sum, "cell_cache_load_ms_sum": s.load_ms_sum, "district_ms_sum": s.district_ms_sum,
            "buildings_in_loaded_cells": s.buildings_loaded, "fallback_buildings": s.fallback_buildings, "district_buildings": s.district_buildings,
            "texel_m_min_max": [if s.texel_m_min == f32::MAX { 0.0 } else { s.texel_m_min }, s.texel_m_max],
            "bytes_uploaded": s.bytes_uploaded, "max_frame_bytes": s.max_frame_bytes, "upload_budget_bytes": o.upload_bytes,
            "first_cell_resident_ms": s.first_resident_ms, "inputs_ready_ms": s.inputs_ready_ms, "districts_ready_ms": s.districts_ready_ms,
            "render_thread_update_ms": { "max": s.update_ms_max, "mean": if s.updates > 0 { s.update_ms_sum / s.updates as f64 } else { 0.0 }, "frames": s.updates },
            "cache_key": s.key,
        });
        if let (Some(m), serde_json::Value::Object(mem)) = (v.as_object_mut(), memory) {
            m.extend(mem);
            m.insert("bake_options".into(), bake);
            m.insert("set".into(), set);
            m.insert("far_look".into(), far_look);
        }
        v
    }
}

/// How much of a write fits in `budget` (whole rows of a texture mip, 4-byte multiples of a buffer).
fn split_len(w: &HWrite, budget: u64) -> usize {
    let len = w.data.len() - w.pos;
    match w.target {
        Target::Page { mip, .. } => {
            let row = page_row_bytes(mip);
            let rows = (budget / row).max(1);
            ((rows * row) as usize).min(len)
        }
        _ => ((budget.max(4) / 4 * 4) as usize).min(len),
    }
}

impl Drop for HlodRuntime {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for h in self.threads.drain(..) {
            let _ = h.join();
        }
    }
}

/// The far layer's summaries per building (tests).
pub fn summaries_of(cells: &[BakedCell]) -> Vec<BuildingSummary> {
    cells.iter().flat_map(|c| c.summaries.iter().copied()).collect()
}
// the far night windows' hash and lit decision (hlod_draw.wgsl
// `hl_u`, `hl_h3`, `hl_night_windows`), mirrored for the tests.
pub const NIGHT_SEG_LIT: f32 = 0.7;
pub const NIGHT_LIT_IN_SEG: f32 = 0.88;
pub const NIGHT_SCATTER: f32 = 0.03;

pub fn night_u(x: u32) -> u32 {
    let s = x.wrapping_mul(747796405).wrapping_add(2891336453);
    let w = ((s >> ((s >> 28) + 4)) ^ s).wrapping_mul(277803737);
    (w >> 22) ^ w
}

pub fn night_hash3(a: u32, b: i32, c: u32) -> f32 {
    (night_u(a ^ night_u((b as u32) ^ night_u(c))) >> 8) as f32 / 16777216.0
}

/// A tower's chance that a floor is lit: the room drive's office share times the tower's own activity.
pub fn night_floor_chance(group: u32, office: f32) -> f32 {
    let hb = night_hash3(group, 0, 11);
    (office / 0.7 * 2.6 * hb * hb).clamp(0.0, 0.92)
}

/// One tower window by night (face: the heading in eighths; floor; along: metres along the wall; bay in metres).
pub fn night_tower_window(group: u32, face: u32, office: f32, floor: i32, along: f32, bay: f32) -> bool {
    let p_floor = night_floor_chance(group, office);
    let floor_lit = night_hash3(group, floor, 1) < p_floor;
    let seg_len = 6.0 + 12.0 * night_hash3(group, floor, 2);
    let seg = (along / seg_len + night_hash3(group, floor, 3)).floor() as i32;
    let gf = group ^ (face << 24);
    let seg_lit = night_hash3(gf, floor.wrapping_mul(131).wrapping_add(seg), 4) < NIGHT_SEG_LIT;
    let cell = (along / bay).floor() as i32;
    let wh = night_hash3(gf, floor.wrapping_mul(977).wrapping_add(cell), 5);
    if floor_lit { seg_lit && wh < NIGHT_LIT_IN_SEG } else { wh < NIGHT_SCATTER }
}

/// What the far shader fades a tower to once its storeys are under a pixel.
pub fn night_tower_mean(group: u32, office: f32) -> f32 {
    let p = night_floor_chance(group, office);
    p * NIGHT_SEG_LIT * NIGHT_LIT_IN_SEG + (1.0 - p) * NIGHT_SCATTER
}

#[cfg(test)]
mod towers2_tests {
    use super::*;

    fn office_at(hour: f64) -> f32 {
        crate::rooms::drive(-20.0, hour).1[0]
    }

    /// Lit windows of one tower face: 40 floors of 60 bays of 1.5 m.
    fn tower_share(g: u32, office: f32) -> (f32, Vec<f32>) {
        let mut floors = vec![];
        for fl in 0..40 {
            let lit = (0..60).filter(|&c| night_tower_window(g, 2, office, fl, c as f32 * 1.5 + 0.75, 1.5)).count();
            floors.push(lit as f32 / 60.0);
        }
        (floors.iter().sum::<f32>() / 40.0, floors)
    }

    #[test]
    fn a_minority_of_downtown_windows_are_lit_at_nine_and_towers_differ() {
        let office = office_at(21.0);
        assert!(office > 0.15 && office < 0.35, "office share {office}");
        let shares: Vec<f32> = (0..400u32).map(|g| tower_share(g * 7919 + 13, office).0).collect();
        let mean = shares.iter().sum::<f32>() / shares.len() as f32;
        let dark = shares.iter().filter(|&&s| s < 0.1).count() as f32 / shares.len() as f32;
        let busy = shares.iter().cloned().fold(0.0, f32::max);
        assert!(mean > 0.08 && mean < 0.3, "mean lit share {mean}");
        assert!(dark > 0.25, "towers mostly dark {dark}");
        assert!(busy > 0.4, "the busiest tower {busy}");
        // Late at night fewer, at dusk more.
        let late: f32 = (0..400u32).map(|g| tower_share(g * 7919 + 13, office_at(1.0)).0).sum::<f32>() / 400.0;
        let dusk: f32 = (0..400u32).map(|g| tower_share(g * 7919 + 13, office_at(18.5)).0).sum::<f32>() / 400.0;
        assert!(late < mean && mean < dusk, "{late} {mean} {dusk}");
    }

    #[test]
    fn lit_windows_cluster_by_floor() {
        let office = office_at(21.0);
        let (mut in_lit_floors, mut all) = (0.0, 0.0);
        for g in 0..300u32 {
            for f in tower_share(g * 104729 + 1, office).1 {
                all += f;
                if f >= 0.3 {
                    in_lit_floors += f;
                }
            }
        }
        assert!(in_lit_floors / all > 0.75, "share of lit windows on lit floors {}", in_lit_floors / all);
    }

    #[test]
    fn the_far_mean_is_the_mean_of_the_windows() {
        // The shader fades to night_tower_mean under a pixel: it must be what the windows average to.
        let office = office_at(21.0);
        let (mut got, mut want) = (0.0, 0.0);
        for g in 0..600u32 {
            got += tower_share(g * 31 + 5, office).0;
            want += night_tower_mean(g * 31 + 5, office);
        }
        let (got, want) = (got / 600.0, want / 600.0);
        assert!((got - want).abs() < 0.015, "windows {got} against the mean {want}");
    }

    #[test]
    fn the_hash_is_uniform() {
        let n = 100_000;
        let mut bins = [0u32; 10];
        for i in 0..n {
            bins[(night_hash3(i as u32 * 3 + 1, (i % 97) as i32 - 40, 5) * 10.0) as usize] += 1;
        }
        for b in bins {
            assert!((b as f32 / n as f32 - 0.1).abs() < 0.01, "{bins:?}");
        }
    }
}
