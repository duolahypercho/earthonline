//! the static street props (signals, signs, trees, tactile pads, and
//! (props2) the street-furniture kinds, the ez-tree trees and the street
//! overhead) as an INSTANCED class of the GPU-driven path.
//!
//! The library (`props_lib.rs`, `props_lib2.rs`: meshes with up to three
//! levels each, their materials and texture arrays) is written once.
//! Instances (48 bytes: position, yaw, scale, model, variant, lamp channel,
//! slope) live in one arena; the app streams them per 128 m street tile
//! (planned on worker threads by eo-props, written through the budgeted
//! upload queue, `upload::Target::PropInstances`). Nothing is done per
//! instance on the CPU.
//!
//! Per frame (`props_cull.wgsl`):
//! 1. before the city pass, after the look's and the agents' casters: one
//!    compute pass classifies every slot for the sun cascades the look
//!    refreshed (and for the camera when occlusion is off), with the model's
//!    distance level and caster cascades; prefix sums give one compacted
//!    command list per (view, part) with its count; each refreshed cascade
//!    draws its casters with two count-driven multi-draws (opaque,
//!    alpha-tested);
//! 2. inside the occlusion pass, after the city's pyramid: the camera view,
//!    each instance's box tested with the city's own `hzb_occluded`, and
//!    (props2) the wire segments;
//! 3. in the late city pass (the single pass with occlusion off), before the
//!    the camera lists (opaque, alpha-tested) and (props2) the
//!    cross-fade pair (views 4 and 5, dithered), lit by `look_light` with the
//!    cascaded sun shadow; the AO pass reads their depth like any other;
//! 4. (props2) after the late city pass, a pass of its own (so it can be
//!    timed): the wires, one indirect draw, alpha-blended, depth-tested.
//! Signal and pedestrian lamps read a small per-frame lamp-channel array
//! (`write_lamps`) the app fills from the simulation's signal controllers.
//!
//! props2 lamp lights: the district street lights' luminaires
//! (`eo_props::furniture::LampLight`) of the resident tiles are kept in a
//! storage buffer here (`lights_buffer`, `GpuLampLight`, 64 bytes each, the
//! count in `light_count_buffer`), written per tile through the upload queue
//! (`Target::PropLights`). Nothing reads them yet: they are for the night lighting.

use crate::camera::{Camera, frustum_planes};
use crate::look::LookGpu;
use crate::props_lib::PropLibrary;
use crate::{DEPTH_FORMAT, DRAW_ARGS_BYTES, GpuContext, IDS_FORMAT};
use anyhow::{Result, anyhow};
use bytemuck::{Pod, Zeroable};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use wgpu::util::DeviceExt;

/// Instance slots in the arena (sized once at start; props2 measured at most
/// ~40 k resident on the bench paths, see the README).
pub const INSTANCE_CAPACITY: u32 = 131_072;
/// Lamp channels (signal and pedestrian heads resident at once).
pub const LAMP_CAPACITY: u32 = 16_384;
/// 0 camera, 1..3 cascades, 4..5 the camera's cross-fade pair.
pub const VIEWS: u64 = 6;
/// Timestamps this module writes per frame (props2): cascade cull begin/end,
/// casters begin/end, wires begin/end.
pub const QUERIES_PER_FRAME: u32 = 6;
/// wire segment slots.
pub const WIRE_CAPACITY: u32 = 196_608;
/// lamp light slots.
pub const LIGHT_CAPACITY: u32 = 16_384;
/// props2 feature bits (PrParams.mode.z).
pub const F_FADE: u32 = 1;
pub const F_WIND: u32 = 2;
pub const F_LEAF: u32 = 4;
pub const F_DOMES: u32 = 8;
pub const F_SIGNS: u32 = 16;
pub const F_ALL: u32 = 31;
/// A slot no instance uses (its model is out of range).
pub const EMPTY: GpuPropInstance = GpuPropInstance { pos: [0.0; 3], yaw: 0.0, scale: [0.0; 3], model: u32::MAX, variant: 0, lamp: 0, slope: 0.0, pad: 0 };

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable, PartialEq)]
pub struct GpuPropInstance {
    pub pos: [f32; 3],
    pub yaw: f32,
    pub scale: [f32; 3],
    pub model: u32,
    pub variant: u32,
    pub lamp: u32,
    pub slope: f32,
    pub pad: u32,
}
const _: () = assert!(std::mem::size_of::<GpuPropInstance>() == 48);

/// one wire segment (32 bytes): end a, true radius, end b, kind; radius 0 = empty slot.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable, PartialEq)]
pub struct GpuWireSeg {
    pub a: [f32; 3],
    pub radius: f32,
    pub b: [f32; 3],
    pub kind: u32,
}
const _: () = assert!(std::mem::size_of::<GpuWireSeg>() == 32);

/// one luminaire (64 bytes), `eo_props::furniture::LampLight` as the GPU will read it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable, PartialEq)]
pub struct GpuLampLight {
    /// The luminaire (`streetLampHead`); the spot source sits `source_lift` above it.
    pub pos: [f32; 3],
    /// three.js `SpotLight.distance` (m).
    pub range: f32,
    /// `LAMP_PRACTICALS.color`, linear RGB.
    pub colour: [f32; 3],
    /// Candela at full `lampsOn` (0: empty slot).
    pub intensity: f32,
    /// Cone half-angle (rad), penumbra fraction, decay, source lift (m).
    pub angle: f32,
    pub penumbra: f32,
    pub decay: f32,
    pub source_lift: f32,
    /// `eo_props::Model` of the lamp (LampHistoric / LampBanner / LampCobra).
    pub kind: u32,
    pub pad: [u32; 3],
}
const _: () = assert!(std::mem::size_of::<GpuLampLight>() == 64);

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PrParams {
    planes: [[f32; 4]; 6],
    cam: [f32; 4],
    counts: [u32; 4],
    mode: [u32; 4],
    view_proj: [[f32; 4]; 4],
    lens: [f32; 4],
    dome: [f32; 4],
    wind: [f32; 4],
    wire: [f32; 4],
    extra: [u32; 4],
}

/// draw.wgsl's `Frame`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PrFrame {
    view_proj: [[f32; 4]; 4],
    cam_pos: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    ambient_sky: [f32; 4],
    ambient_ground: [f32; 4],
    copy_step: [f32; 4],
}

/// props_types.wgsl with the vegetation's range and thinning ahead of it
/// (`props_lib2::vegetation_models::thin_wgsl`).
fn pr_types_src() -> String {
    format!("{}\n{}", crate::props_lib2::vegetation_models::thin_wgsl(), include_str!("props_types.wgsl"))
}

/// What building a `PropsGpu` needs from the renderer (clones: wgpu handles are shared).
#[derive(Clone)]
pub struct PropsInit {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub main_bgl: wgpu::BindGroupLayout,
    pub shadow_views: wgpu::Buffer,
    pub late_bgl: wgpu::BindGroupLayout,
    pub sample_count: u32,
    pub debug_ids: bool,
}

/// the wire constants of `STREET_OVERHEAD` (street-overhead.js).
#[derive(Clone, Copy, Debug)]
pub struct WireLook {
    pub pixel_half_width_rad: f32,
    pub fade_near_m: f32,
    pub fade_far_m: f32,
    pub min_coverage: f32,
}

impl Default for WireLook {
    fn default() -> Self {
        // street-overhead.js STREET_OVERHEAD: pixelHalfWidthRad, fadeNearM, fadeFarM, minCoverage.
        Self { pixel_half_width_rad: 0.0014, fade_near_m: 140.0, fade_far_m: 205.0, min_coverage: 0.7 }
    }
}

pub struct PropsGpu {
    enabled: AtomicBool,
    active: AtomicBool,
    camera_late: AtomicBool,
    high_water: AtomicU32,
    wire_high_water: AtomicU32,
    wires_active: AtomicBool,
    features: AtomicU32,
    time_bits: AtomicU32,
    pub mesh_count: u32,
    pub model_count: u32,
    indirect_count: bool,
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    instances: wgpu::Buffer,
    lamps: wgpu::Buffer,
    params_s: wgpu::Buffer,
    params_c: wgpu::Buffer,
    frame: wgpu::Buffer,
    counts: wgpu::Buffer,
    cmds: wgpu::Buffer,
    list_counts: wgpu::Buffer,
    stats: wgpu::Buffer,
    cull_bg_s: wgpu::BindGroup,
    cull_bg_c: wgpu::BindGroup,
    cull_s: [wgpu::ComputePipeline; 3],
    cull_c: [wgpu::ComputePipeline; 3],
    draw_bg: wgpu::BindGroup,
    data_bg: wgpu::BindGroup,
    view_bg: wgpu::BindGroup,
    opaque: wgpu::RenderPipeline,
    cutout: wgpu::RenderPipeline,
    opaque_fade: wgpu::RenderPipeline,
    cutout_fade: wgpu::RenderPipeline,
    shadow: wgpu::RenderPipeline,
    shadow_cut: wgpu::RenderPipeline,
    // wires.
    wires: wgpu::Buffer,
    wire_args: wgpu::Buffer,
    wire_cull_s: wgpu::ComputePipeline,
    wire_cull_c: wgpu::ComputePipeline,
    wire_cull_bg_s: wgpu::BindGroup,
    wire_cull_bg_c: wgpu::BindGroup,
    wire_draw_bg: wgpu::BindGroup,
    wire_pipe: wgpu::RenderPipeline,
    wire_look: WireLook,
    // lamp lights for the night lighting (kept, not read yet).
    lights: wgpu::Buffer,
    light_count: wgpu::Buffer,
    timing: Mutex<Option<(wgpu::QuerySet, u32)>>,
    alpha_test: f32,
    lens_scale: Mutex<f32>,
    face_count: u32,
    dome: [f32; 4],
    /// GPU bytes allocated here.
    pub bytes: u64,
}

fn texture_array(d: &wgpu::Device, q: &wgpu::Queue, label: &str, format: wgpu::TextureFormat, res: u32, layers: &[&[u8]], keep_alpha_coverage: Option<(usize, f32)>) -> (wgpu::Texture, u64) {
    let mips = res.ilog2() + 1;
    let n = layers.len().max(1) as u32;
    let t = d.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: res, height: res, depth_or_array_layers: n },
        mip_level_count: mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let srgb = format == wgpu::TextureFormat::Rgba8UnormSrgb;
    for (layer, data) in layers.iter().enumerate() {
        let mut chain = crate::authored::mip_chain(data, res as usize, srgb);
        // alpha-tested foliage keeps its mip 0 coverage at every mip
        // (else the crowns thin out with distance as the alpha averages down).
        if let Some((l, test)) = keep_alpha_coverage {
            if l == layer {
                preserve_coverage(&mut chain, res as usize, test);
            }
        }
        for (mip, level) in chain.iter().enumerate().take(mips as usize) {
            let r = (res >> mip).max(1);
            q.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &t, mip_level: mip as u32, origin: wgpu::Origin3d { x: 0, y: 0, z: layer as u32 }, aspect: wgpu::TextureAspect::All },
                level,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(r * 4), rows_per_image: Some(r) },
                wgpu::Extent3d { width: r, height: r, depth_or_array_layers: 1 },
            );
        }
    }
    let bytes = (0..mips).map(|m| ((res >> m).max(1) as u64).pow(2) * 4).sum::<u64>() * n as u64;
    (t, bytes)
}

/// scale each mip's alpha so the fraction of texels passing `test` equals mip 0's
/// (the standard coverage-preserving alpha mip fix, bisection on the scale).
fn preserve_coverage(chain: &mut [Vec<u8>], res: usize, test: f32) {
    let cov = |m: &[u8], s: f32| m.chunks_exact(4).filter(|p| (p[3] as f32 / 255.0 * s) >= test).count() as f32 / (m.len() / 4).max(1) as f32;
    let target = cov(&chain[0], 1.0);
    for (k, m) in chain.iter_mut().enumerate().skip(1) {
        if (res >> k) < 4 {
            break;
        }
        let (mut lo, mut hi) = (0.25f32, 4.0f32);
        for _ in 0..12 {
            let mid = (lo + hi) / 2.0;
            if cov(m, mid) < target {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let s = (lo + hi) / 2.0;
        for p in m.chunks_exact_mut(4) {
            p[3] = (p[3] as f32 * s).round().clamp(0.0, 255.0) as u8;
        }
    }
}

impl PropsGpu {
    /// Build on any thread (pipelines, buffers, texture mips): `init` from `Renderer::props_init`.
    pub fn new(pi: &PropsInit, lib: &PropLibrary) -> Result<Self> {
        let d = &pi.device;
        let q = &pi.queue;
        let (sample_count, debug_ids, late_bgl) = (pi.sample_count, pi.debug_ids, &pi.late_bgl);
        let mesh_count = lib.meshes.len() as u32;
        if mesh_count == 0 || mesh_count > 256 {
            return Err(anyhow!("props library has {mesh_count} meshes (1..=256 supported)"));
        }
        let indirect_count = d.features().contains(wgpu::Features::MULTI_DRAW_INDIRECT_COUNT);
        use wgpu::BufferUsages as U;
        let init = |label: &str, bytes: &[u8], usage: wgpu::BufferUsages| {
            let contents: Vec<u8> = if bytes.is_empty() { vec![0u8; 16] } else { bytes.to_vec() };
            d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents: &contents, usage })
        };
        let empty = |label: &str, size: u64, usage: wgpu::BufferUsages| d.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size.max(16).next_multiple_of(16), usage, mapped_at_creation: false });
        let vbuf = init("prop-vertices", bytemuck::cast_slice(&lib.vertices), U::VERTEX);
        let ibuf = init("prop-indices", bytemuck::cast_slice(&lib.indices), U::INDEX);
        let meshes = init("prop-meshes", bytemuck::cast_slice(&lib.meshes), U::STORAGE);
        let models = init("prop-models", bytemuck::cast_slice(&lib.models), U::STORAGE);
        let materials = init("prop-materials", bytemuck::cast_slice(&lib.materials), U::STORAGE);
        let faces = init("prop-faces", bytemuck::cast_slice(&lib.faces), U::STORAGE);
        let lens = init("prop-lens", bytemuck::cast_slice(&lib.lens), U::UNIFORM);
        let objects = init("prop-object", &[0u8; 64], U::STORAGE);
        let n = INSTANCE_CAPACITY as u64;
        let empty_slots = vec![EMPTY; INSTANCE_CAPACITY as usize];
        let instances = init("prop-instances", bytemuck::cast_slice(&empty_slots), U::STORAGE | U::COPY_DST);
        let lamps = empty("prop-lamps", LAMP_CAPACITY as u64 * 4, U::STORAGE | U::COPY_DST);
        let params_s = empty("prop-params-pre", std::mem::size_of::<PrParams>() as u64, U::UNIFORM | U::COPY_DST);
        let params_c = empty("prop-params-camera", std::mem::size_of::<PrParams>() as u64, U::UNIFORM | U::COPY_DST);
        let frame = empty("prop-frame", std::mem::size_of::<PrFrame>() as u64, U::UNIFORM | U::COPY_DST);
        let records = empty("prop-records", n * VIEWS * 4, U::STORAGE);
        let visible = empty("prop-visible", n * VIEWS * 4, U::STORAGE);
        let m = mesh_count as u64;
        let counts = empty("prop-counts", m * VIEWS * 2 * 4, U::STORAGE | U::COPY_DST);
        let cmds = empty("prop-draws", m * VIEWS * 2 * DRAW_ARGS_BYTES, U::STORAGE | U::INDIRECT | U::COPY_SRC);
        let list_counts = empty("prop-list-counts", VIEWS * 2 * 4, U::STORAGE | U::INDIRECT | U::COPY_DST);
        let stats = empty("prop-stats", 16 * 4, U::STORAGE | U::COPY_DST | U::COPY_SRC);
        // wires and lamp lights.
        let wires = init("prop-wires", bytemuck::cast_slice(&vec![GpuWireSeg::default(); WIRE_CAPACITY as usize]), U::STORAGE | U::COPY_DST);
        let wire_vis = empty("prop-wire-visible", WIRE_CAPACITY as u64 * 4, U::STORAGE);
        let wire_args = empty("prop-wire-args", 32, U::STORAGE | U::INDIRECT | U::COPY_DST | U::COPY_SRC);
        let lights = init("prop-lamp-lights", bytemuck::cast_slice(&vec![GpuLampLight::default(); LIGHT_CAPACITY as usize]), U::STORAGE | U::COPY_DST | U::COPY_SRC);
        let light_count = empty("prop-lamp-light-count", 16, U::STORAGE | U::UNIFORM | U::COPY_DST | U::COPY_SRC);

        let alb: Vec<&[u8]> = lib.layers.iter().map(|l| l[0].as_slice()).collect();
        let nor: Vec<&[u8]> = lib.layers.iter().map(|l| l[1].as_slice()).collect();
        let orm: Vec<&[u8]> = lib.layers.iter().map(|l| l[2].as_slice()).collect();
        let (t_alb, b0) = texture_array(d, q, "prop-albedo", wgpu::TextureFormat::Rgba8UnormSrgb, crate::props_lib::ALBEDO_RES, &alb, None);
        let (t_nor, b1) = texture_array(d, q, "prop-normal", wgpu::TextureFormat::Rgba8Unorm, crate::props_lib::AUX_RES, &nor, None);
        let (t_orm, b2) = texture_array(d, q, "prop-orm", wgpu::TextureFormat::Rgba8Unorm, crate::props_lib::AUX_RES, &orm, None);
        // the big array (vegetation atlas with coverage-preserving mips, sign faces, printed faces).
        let big: Vec<&[u8]> = lib.big_layers.iter().map(|l| l.as_slice()).collect();
        let fallback = crate::props_lib::flat_layer([255; 4], 4);
        let big_ok = !big.is_empty() && big.iter().all(|l| l.len() == (crate::props_lib::BIG_RES as usize).pow(2) * 4);
        let (t_big, b3) = if big_ok {
            texture_array(d, q, "prop-big", wgpu::TextureFormat::Rgba8UnormSrgb, crate::props_lib::BIG_RES, &big, Some((0, lib.alpha_test)))
        } else {
            texture_array(d, q, "prop-big", wgpu::TextureFormat::Rgba8UnormSrgb, 4, &[&fallback, &fallback, &fallback], None)
        };
        let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        let sampler = d.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("prop-sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });

        let entry = |binding, visibility, ty| wgpu::BindGroupLayoutEntry { binding, visibility, ty, count: None };
        let ubuf = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None };
        let sbuf = |read_only| wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None };
        let cs = wgpu::ShaderStages::COMPUTE;
        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let fs = wgpu::ShaderStages::FRAGMENT;
        fn bge(binding: u32, b: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry { binding, resource: b.as_entire_binding() }
        }
        let tex = |binding, vis| {
            entry(binding, vis, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false })
        };

        // Cull (group 0); the Hi-Z variant also binds the city pyramid (group 1).
        let cull_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("prop-cull-bgl"),
            entries: &[
                entry(0, cs, ubuf),
                entry(1, cs, sbuf(true)),
                entry(2, cs, sbuf(true)),
                entry(3, cs, sbuf(true)),
                entry(4, cs, sbuf(false)),
                entry(5, cs, sbuf(false)),
                entry(6, cs, sbuf(false)),
                entry(7, cs, sbuf(false)),
                entry(8, cs, sbuf(false)),
                entry(9, cs, ubuf),
                entry(10, cs, sbuf(false)),
            ],
        });
        let cascades = wgpu::BindGroupEntry {
            binding: 9,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &pi.shadow_views, offset: 0, size: Some(std::num::NonZeroU64::new(768).unwrap()) }),
        };
        let cull_bg = |params: &wgpu::Buffer, label: &str| {
            d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &cull_bgl,
                entries: &[
                    bge(0, params),
                    bge(1, &instances),
                    bge(2, &models),
                    bge(3, &meshes),
                    bge(4, &records),
                    bge(5, &visible),
                    bge(6, &counts),
                    bge(7, &cmds),
                    bge(8, &stats),
                    cascades.clone(),
                    bge(10, &list_counts),
                ],
            })
        };
        let cull_bg_s = cull_bg(&params_s, "prop-cull-bg-pre");
        let cull_bg_c = cull_bg(&params_c, "prop-cull-bg-camera");
        let plain_src = format!("{}\n{}\nfn pr_occluded(mn: vec3<f32>, mx: vec3<f32>) -> bool {{ return false; }}\n", pr_types_src(), include_str!("props_cull.wgsl"));
        let hzb_tail = format!(
            "struct HzbInfo {{ screen: vec4<f32> }};\n@group(1) @binding(0) var hzb: texture_2d<f32>;\n@group(1) @binding(1) var<uniform> hzb_info: HzbInfo;\n{}\nfn pr_occluded(mn: vec3<f32>, mx: vec3<f32>) -> bool {{ return hzb_occluded(mn, mx, pr.view_proj, hzb_info.screen.xy); }}\n",
            crate::occlusion::OCCLUSION_WGSL
        );
        let hzb_src = format!("{}\n{}\n{hzb_tail}", pr_types_src(), include_str!("props_cull.wgsl"));
        let plain_mod = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("props_cull.wgsl"), source: wgpu::ShaderSource::Wgsl(plain_src.into()) });
        let hzb_mod = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("props_cull.wgsl (hzb)"), source: wgpu::ShaderSource::Wgsl(hzb_src.into()) });
        let lay_s = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("prop-cull-layout"), bind_group_layouts: &[Some(&cull_bgl)], immediate_size: 0 });
        let lay_c = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("prop-cull-hzb-layout"), bind_group_layouts: &[Some(&cull_bgl), Some(late_bgl)], immediate_size: 0 });
        let compute = |module: &wgpu::ShaderModule, layout: &wgpu::PipelineLayout, e: &str| {
            d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(e), layout: Some(layout), module, entry_point: Some(e), compilation_options: Default::default(), cache: None })
        };
        let cull_s = [compute(&plain_mod, &lay_s, "classify"), compute(&plain_mod, &lay_s, "offsets"), compute(&plain_mod, &lay_s, "scatter")];
        let cull_c = [compute(&hzb_mod, &lay_c, "classify"), compute(&hzb_mod, &lay_c, "offsets"), compute(&hzb_mod, &lay_c, "scatter")];

        // Props data (group 2 of the colour and shadow pipelines).
        let data_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("prop-data-bgl"),
            entries: &[
                entry(0, vf, ubuf),
                entry(1, vf, sbuf(true)),
                entry(2, vf, sbuf(true)),
                entry(3, vf, sbuf(true)),
                entry(4, vf, sbuf(true)),
                entry(5, vf, ubuf),
                tex(6, vf),
                entry(7, vf, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                entry(8, vf, sbuf(true)),
                tex(9, vf), // the big array
            ],
        });
        let (av, nv, ov, bv) = (view(&t_alb), view(&t_nor), view(&t_orm), view(&t_big));
        let data_bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("prop-data-bg"),
            layout: &data_bgl,
            entries: &[
                bge(0, &params_s),
                bge(1, &instances),
                bge(2, &visible),
                bge(3, &lamps),
                bge(4, &faces),
                bge(5, &lens),
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&av) },
                wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::Sampler(&sampler) },
                bge(8, &materials),
                wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(&bv) },
            ],
        });
        let draw_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("prop-draw-bgl"),
            entries: &[
                entry(0, vf, ubuf),
                entry(1, vf, sbuf(true)),
                entry(2, fs, sbuf(true)),
                tex(3, fs),
                tex(4, fs),
                tex(5, fs),
                entry(6, fs, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                // A scene-probe slot for a look_light that names one (bindings
                // 14-16 of draw.wgsl's group 0 in trees that have it): a 1 x 1
                // stand-in; the props' frame says no probe.
                tex(14, fs),
                entry(15, fs, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                entry(16, fs, sbuf(true)),
            ],
        });
        let probe_stand_in = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("prop-probe-stand-in"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 6 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let probe_view = view(&probe_stand_in);
        let probe_sh = empty("prop-probe-sh-stand-in", 160, U::STORAGE);
        let draw_bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("prop-draw-bg"),
            layout: &draw_bgl,
            entries: &[
                bge(0, &frame),
                bge(1, &objects),
                bge(2, &materials),
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&av) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&nv) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&ov) },
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::Sampler(&sampler) },
                wgpu::BindGroupEntry { binding: 14, resource: wgpu::BindingResource::TextureView(&probe_view) },
                wgpu::BindGroupEntry { binding: 15, resource: wgpu::BindingResource::Sampler(&sampler) },
                wgpu::BindGroupEntry { binding: 16, resource: probe_sh.as_entire_binding() },
            ],
        });
        let draw_src = format!("{}\n{}\n{}", crate::look::city_shader_source(), pr_types_src(), include_str!("props_draw.wgsl"));
        let draw_mod = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("props_draw.wgsl"), source: wgpu::ShaderSource::Wgsl(draw_src.into()) });
        let draw_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("prop-draw-layout"),
            bind_group_layouts: &[Some(&draw_bgl), Some(&pi.main_bgl), Some(&data_bgl)],
            immediate_size: 0,
        });
        let attrs = [
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Snorm8x4, offset: 12, shader_location: 1 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Unorm16x2, offset: 16, shader_location: 2 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Unorm8x4, offset: 20, shader_location: 3 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Uint32, offset: 24, shader_location: 4 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32, offset: 28, shader_location: 5 }, // wind weight
        ];
        let stride = std::mem::size_of::<crate::props_lib::PropVertex>() as u64;
        let make = |label: &str, fs_entry: &str| {
            let color = wgpu::ColorTargetState { format: crate::look::HDR_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL };
            let ids = wgpu::ColorTargetState { format: IDS_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL };
            let targets_ids = [Some(color.clone()), Some(ids)];
            let targets = [Some(color)];
            let entry_name = if debug_ids { format!("{fs_entry}_ids") } else { fs_entry.to_string() };
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&draw_layout),
                vertex: wgpu::VertexState {
                    module: &draw_mod,
                    entry_point: Some("prop_vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout { array_stride: stride, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attrs })],
                },
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Greater),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState { count: sample_count, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &draw_mod,
                    entry_point: Some(&entry_name),
                    compilation_options: Default::default(),
                    targets: if debug_ids { &targets_ids } else { &targets },
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let opaque = make("prop-opaque", "prop_fs");
        let cutout = make("prop-cutout", "prop_fs_cut");
        let opaque_fade = make("prop-opaque-fade", "prop_fs_fade");
        let cutout_fade = make("prop-cutout-fade", "prop_fs_cut_fade");

        // Sun casters.
        let view_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("prop-shadow-view-bgl"),
            entries: &[entry(0, wgpu::ShaderStages::VERTEX, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: None })],
        });
        let view_bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("prop-shadow-view-bg"),
            layout: &view_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &pi.shadow_views, offset: 0, size: Some(std::num::NonZeroU64::new(80).unwrap()) }),
            }],
        });
        let shadow_src = format!("{}\n{}", pr_types_src(), include_str!("props_shadow.wgsl"));
        let shadow_mod = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("props_shadow.wgsl"), source: wgpu::ShaderSource::Wgsl(shadow_src.into()) });
        let shadow_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("prop-shadow-layout"), bind_group_layouts: &[Some(&view_bgl), None, Some(&data_bgl)], immediate_size: 0 });
        let shadow_attrs = [attrs[0], attrs[2], attrs[4], attrs[5]];
        let shadow_pipe = |label: &str, cut: bool| {
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&shadow_layout),
                vertex: wgpu::VertexState {
                    module: &shadow_mod,
                    entry_point: Some("prop_shadow_vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout { array_stride: stride, step_mode: wgpu::VertexStepMode::Vertex, attributes: &shadow_attrs })],
                },
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: crate::look::SHADOW_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: cut.then(|| wgpu::FragmentState { module: &shadow_mod, entry_point: Some("prop_shadow_cut_fs"), compilation_options: Default::default(), targets: &[] }),
                multiview_mask: None,
                cache: None,
            })
        };
        let shadow = shadow_pipe("prop-shadow", false);
        let shadow_cut = shadow_pipe("prop-shadow-cut", true);

        // wires (cull in the props' compute passes, draw in a pass of its own).
        let wsrc = include_str!("props_wires.wgsl");
        let section = |tag: &str| -> String {
            let (open, close) = (format!("#ifdef_{tag}"), format!("#endif_{tag}"));
            let a = wsrc.find(&open).map(|i| i + open.len()).unwrap_or(0);
            let b = wsrc.find(&close).unwrap_or(wsrc.len());
            wsrc[a..b].to_string()
        };
        let head = wsrc.split("#ifdef_cull").next().unwrap_or("");
        let wire_cull_plain = format!("{}\n{head}\n{}\nfn pr_occluded(mn: vec3<f32>, mx: vec3<f32>) -> bool {{ return false; }}\n", pr_types_src(), section("cull"));
        let wire_cull_hzb = format!("{}\n{head}\n{}\n{hzb_tail}", pr_types_src(), section("cull"));
        let wire_draw_src = format!("{}\n{}\n{head}\n{}", crate::look::look_common_source(), pr_types_src(), section("draw"));
        let wire_cull_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("prop-wire-cull-bgl"),
            entries: &[entry(0, cs, ubuf), entry(1, cs, sbuf(true)), entry(2, cs, sbuf(false)), entry(3, cs, sbuf(false))],
        });
        let wire_cull_bg = |p: &wgpu::Buffer, label: &str| {
            d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some(label), layout: &wire_cull_bgl, entries: &[bge(0, p), bge(1, &wires), bge(2, &wire_vis), bge(3, &wire_args)] })
        };
        let wire_cull_bg_s = wire_cull_bg(&params_s, "prop-wire-cull-bg-pre");
        let wire_cull_bg_c = wire_cull_bg(&params_c, "prop-wire-cull-bg-camera");
        let wmod_p = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("props_wires.wgsl (cull)"), source: wgpu::ShaderSource::Wgsl(wire_cull_plain.into()) });
        let wmod_h = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("props_wires.wgsl (cull, hzb)"), source: wgpu::ShaderSource::Wgsl(wire_cull_hzb.into()) });
        let wlay_s = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("prop-wire-cull-layout"), bind_group_layouts: &[Some(&wire_cull_bgl)], immediate_size: 0 });
        let wlay_c = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("prop-wire-cull-hzb-layout"), bind_group_layouts: &[Some(&wire_cull_bgl), Some(late_bgl)], immediate_size: 0 });
        let wire_cull_s = compute(&wmod_p, &wlay_s, "wire_cull");
        let wire_cull_c = compute(&wmod_h, &wlay_c, "wire_cull");
        let wire_draw_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("prop-wire-draw-bgl"),
            entries: &[entry(0, vf, ubuf), entry(1, vf, sbuf(true)), entry(2, vf, sbuf(true))],
        });
        let wire_draw_bg = d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("prop-wire-draw-bg"), layout: &wire_draw_bgl, entries: &[bge(0, &params_c), bge(1, &wires), bge(2, &wire_vis)] });
        let wmod_d = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("props_wires.wgsl (draw)"), source: wgpu::ShaderSource::Wgsl(wire_draw_src.into()) });
        let wlay_d = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("prop-wire-draw-layout"), bind_group_layouts: &[Some(&wire_draw_bgl), Some(&pi.main_bgl)], immediate_size: 0 });
        let wire_pipe = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("prop-wires"),
            layout: Some(&wlay_d),
            vertex: wgpu::VertexState { module: &wmod_d, entry_point: Some("wire_vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState { count: sample_count, ..Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &wmod_d,
                entry_point: Some("wire_fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: crate::look::HDR_FORMAT, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::COLOR })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let bytes = [
            &vbuf, &ibuf, &meshes, &models, &materials, &faces, &lens, &objects, &instances, &lamps, &params_s, &params_c, &frame, &records, &visible, &counts, &cmds, &list_counts, &stats, &wires, &wire_vis, &wire_args, &lights, &light_count,
        ]
        .iter()
        .map(|b| b.size())
        .sum::<u64>()
            + b0
            + b1
            + b2
            + b3;
        Ok(Self {
            enabled: AtomicBool::new(true),
            active: AtomicBool::new(false),
            camera_late: AtomicBool::new(false),
            high_water: AtomicU32::new(0),
            wire_high_water: AtomicU32::new(0),
            wires_active: AtomicBool::new(false),
            features: AtomicU32::new(F_ALL),
            time_bits: AtomicU32::new(0),
            mesh_count,
            model_count: lib.models.len() as u32,
            indirect_count,
            vbuf,
            ibuf,
            instances,
            lamps,
            params_s,
            params_c,
            frame,
            counts,
            cmds,
            list_counts,
            stats,
            cull_bg_s,
            cull_bg_c,
            cull_s,
            cull_c,
            draw_bg,
            data_bg,
            view_bg,
            opaque,
            cutout,
            opaque_fade,
            cutout_fade,
            shadow,
            shadow_cut,
            wires,
            wire_args,
            wire_cull_s,
            wire_cull_c,
            wire_cull_bg_s,
            wire_cull_bg_c,
            wire_draw_bg,
            wire_pipe,
            wire_look: WireLook::default(),
            lights,
            light_count,
            timing: Mutex::new(None),
            alpha_test: lib.alpha_test,
            lens_scale: Mutex::new(lib.lens[0][3]),
            face_count: lib.face_count,
            dome: lib.dome,
            bytes,
        })
    }

    /// Props on or off (off: no pass runs, nothing is drawn).
    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// the shader features (`F_*` bits; `--no-props2` clears them all).
    pub fn set_features(&self, f: u32) {
        self.features.store(f, Ordering::Relaxed);
    }

    /// the clock the wind runs on (the simulation's, seconds).
    pub fn set_time(&self, seconds: f64) {
        self.time_bits.store((seconds.rem_euclid(3600.0) as f32).to_bits(), Ordering::Relaxed);
    }

    /// the wire constants (`STREET_OVERHEAD`).
    pub fn set_wire_look(&mut self, w: WireLook) {
        self.wire_look = w;
    }

    /// Instance slots in use (the cull runs over `0..n`).
    pub fn set_high_water(&self, n: u32) {
        self.high_water.store(n.min(INSTANCE_CAPACITY), Ordering::Relaxed);
    }

    pub fn high_water(&self) -> u32 {
        self.high_water.load(Ordering::Relaxed)
    }

    /// wire segment slots in use (the wire cull runs over `0..n`).
    pub fn set_wire_high_water(&self, n: u32) {
        self.wire_high_water.store(n.min(WIRE_CAPACITY), Ordering::Relaxed);
    }

    pub fn wire_high_water(&self) -> u32 {
        self.wire_high_water.load(Ordering::Relaxed)
    }

    pub fn instance_buffer(&self) -> &wgpu::Buffer {
        &self.instances
    }

    /// the wire segment arena (`Target::PropWires`).
    pub fn wire_buffer(&self) -> &wgpu::Buffer {
        &self.wires
    }

    /// the lamp lights of the resident tiles (`GpuLampLight` slots; `Target::PropLights`).
    pub fn lights_buffer(&self) -> &wgpu::Buffer {
        &self.lights
    }

    /// `[slots in use (high-water), capacity, 0, 0]` of `lights_buffer`.
    pub fn light_count_buffer(&self) -> &wgpu::Buffer {
        &self.light_count
    }

    /// the lamp-light slots in use (written when it changes).
    pub fn set_light_count(&self, queue: &wgpu::Queue, n: u32) {
        queue.write_buffer(&self.light_count, 0, bytemuck::cast_slice(&[n.min(LIGHT_CAPACITY), LIGHT_CAPACITY, 0, 0]));
    }

    /// This frame's lamp channel states (a few hundred words).
    pub fn write_lamps(&self, queue: &wgpu::Queue, lamps: &[u32]) {
        let n = lamps.len().min(LAMP_CAPACITY as usize);
        if n > 0 {
            queue.write_buffer(&self.lamps, 0, bytemuck::cast_slice(&lamps[..n]));
        }
    }

    pub fn set_timestamps(&self, ts: Option<(wgpu::QuerySet, u32)>) {
        *self.timing.lock().unwrap() = ts;
    }

    pub fn stats_buffer(&self) -> &wgpu::Buffer {
        &self.stats
    }

    /// The camera's uniforms (after the look's update). `occlusion`: whether
    /// this frame's camera view is culled in the occlusion pass (Hi-Z).
    pub fn update(&self, queue: &wgpu::Queue, cam: &Camera, look: &LookGpu, occlusion: bool) {
        let vp = cam.view_proj();
        let planes = frustum_planes(vp).map(|p| p.to_array());
        let exposure = look.frame().exposure.max(1e-6) as f32;
        let aces = *self.lens_scale.lock().unwrap();
        let counts = [self.high_water(), self.mesh_count, INSTANCE_CAPACITY, self.model_count];
        let feat = self.features.load(Ordering::Relaxed);
        let t = f32::from_bits(self.time_bits.load(Ordering::Relaxed));
        let w = self.wire_look;
        // A light westerly (the bay breeze), amplitude 6 cm at the crown of a 1x tree.
        let wind_dir = [0.94f32, 0.34];
        let base = PrParams {
            planes,
            cam: [cam.eye.x, cam.eye.y, cam.eye.z, 0.0],
            counts,
            mode: [0, 0, feat, 0],
            view_proj: vp.to_cols_array_2d(),
            lens: [aces / exposure, self.alpha_test, t, self.face_count as f32],
            dome: self.dome,
            wind: [wind_dir[0], wind_dir[1], 0.06, w.pixel_half_width_rad],
            wire: [w.fade_near_m, w.fade_far_m, w.min_coverage, 0.0],
            extra: [self.wire_high_water(), 0, 0, 0],
        };
        // Run S: the cascades (+ the camera when there is no occlusion pass this frame).
        let cam_bits = if occlusion { 0 } else { 0b110001 };
        let s = PrParams { mode: [0b1110 | cam_bits, 0b1110 | cam_bits, feat, 0], ..base };
        let c = PrParams { mode: [0b110001, 0b110001, feat, 0], ..base };
        queue.write_buffer(&self.params_s, 0, bytemuck::bytes_of(&s));
        queue.write_buffer(&self.params_c, 0, bytemuck::bytes_of(&c));
        // the wire draw: 6 vertices a slot over every slot (culled slots degenerate,
        // fixed blend order); word 4 counts the segments the cull kept.
        queue.write_buffer(&self.wire_args, 0, bytemuck::cast_slice(&[6u32, self.wire_high_water(), 0, 0, 0, 0, 0, 0]));
        self.camera_late.store(occlusion, Ordering::Relaxed);
        let f = PrFrame {
            view_proj: vp.to_cols_array_2d(),
            cam_pos: cam.eye.extend(1.0).to_array(),
            sun_dir: [0.0, 1.0, 0.0, 0.0],
            sun_color: [1.0, 1.0, 1.0, 0.0],
            ambient_sky: [0.0; 4],
            ambient_ground: [0.0; 4],
            copy_step: [0.0; 4],
        };
        queue.write_buffer(&self.frame, 0, bytemuck::bytes_of(&f));
    }

    /// Empty timestamp passes (a query a frame resolves is always written).
    fn empty_ts(enc: &mut wgpu::CommandEncoder, ts: &Option<(wgpu::QuerySet, u32)>, slot: u32) {
        if let Some((q, b)) = ts {
            enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("props idle"),
                timestamp_writes: Some(wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(b + slot), end_of_pass_write_index: Some(b + slot + 1) }),
            });
        }
    }

    /// Before the city pass, after `LookGpu::encode_shadows` (and the agents'
    /// casters): the cascades' cull and their casters.
    pub fn encode_pre(&self, enc: &mut wgpu::CommandEncoder, look: &LookGpu) {
        let n = self.high_water();
        let nw = self.wire_high_water();
        let on = self.enabled() && n > 0;
        let wires_on = self.enabled() && nw > 0;
        self.active.store(on, Ordering::Relaxed);
        self.wires_active.store(wires_on, Ordering::Relaxed);
        let ts = self.timing.lock().unwrap().clone();
        if !on {
            Self::empty_ts(enc, &ts, 0);
            Self::empty_ts(enc, &ts, 2);
            if wires_on && !self.camera_late.load(Ordering::Relaxed) {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("props-wire-cull"), timestamp_writes: None });
                pass.set_pipeline(&self.wire_cull_s);
                pass.set_bind_group(0, &self.wire_cull_bg_s, &[]);
                pass.dispatch_workgroups(nw.div_ceil(64), 1, 1);
            }
            return;
        }
        enc.clear_buffer(&self.counts, 0, None);
        enc.clear_buffer(&self.stats, 0, None);
        let mask = look.encoded_shadow_mask();
        let list: Vec<usize> = (0..3).filter(|c| mask & (1 << c) != 0).collect();
        {
            let tw = ts.as_ref().map(|(q, b)| wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(*b), end_of_pass_write_index: Some(*b + 1) });
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("props-cull"), timestamp_writes: tw });
            pass.set_bind_group(0, &self.cull_bg_s, &[]);
            let groups = n.div_ceil(64);
            for (k, p) in self.cull_s.iter().enumerate() {
                pass.set_pipeline(p);
                pass.dispatch_workgroups(if k == 1 { 1 } else { groups }, 1, 1);
            }
            // the wires, when the camera is culled here (occlusion off).
            if wires_on && !self.camera_late.load(Ordering::Relaxed) {
                pass.set_pipeline(&self.wire_cull_s);
                pass.set_bind_group(0, &self.wire_cull_bg_s, &[]);
                pass.dispatch_workgroups(nw.div_ceil(64), 1, 1);
            }
        }
        if list.is_empty() {
            Self::empty_ts(enc, &ts, 2);
        }
        let layers = look.shadow_layer_views();
        for (k, &c) in list.iter().enumerate() {
            let tw = ts.as_ref().map(|(q, b)| wgpu::RenderPassTimestampWrites {
                query_set: q,
                beginning_of_pass_write_index: (k == 0).then_some(*b + 2),
                end_of_pass_write_index: (k == list.len() - 1).then_some(*b + 3),
            });
            let tw = tw.filter(|t| t.beginning_of_pass_write_index.is_some() || t.end_of_pass_write_index.is_some());
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("props-casters"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &layers[c],
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: tw,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.view_bg, &[(c as u64 * 256) as u32]);
            pass.set_bind_group(2, &self.data_bg, &[]);
            pass.set_vertex_buffer(0, self.vbuf.slice(..));
            pass.set_index_buffer(self.ibuf.slice(..), wgpu::IndexFormat::Uint32);
            let v = 1 + c as u64;
            pass.set_pipeline(&self.shadow);
            self.draw_list(&mut pass, v * 2);
            pass.set_pipeline(&self.shadow_cut);
            self.draw_list(&mut pass, v * 2 + 1);
        }
    }

    /// One compacted command list (props2): count-driven where the device can.
    fn draw_list(&self, pass: &mut wgpu::RenderPass<'_>, list: u64) {
        let m = self.mesh_count as u64;
        if self.indirect_count {
            pass.multi_draw_indexed_indirect_count(&self.cmds, list * m * DRAW_ARGS_BYTES, &self.list_counts, list * 4, self.mesh_count);
        } else {
            pass.multi_draw_indexed_indirect(&self.cmds, list * m * DRAW_ARGS_BYTES, self.mesh_count);
        }
    }

    /// Inside the occlusion compute pass, after the pyramid: the camera view
    /// tested against it (`hzb_bg`: `OcclusionGpu::late_bgl`'s group).
    pub fn cull_camera(&self, pass: &mut wgpu::ComputePass<'_>, hzb_bg: &wgpu::BindGroup) {
        if !self.camera_late.load(Ordering::Relaxed) {
            return;
        }
        if self.active.load(Ordering::Relaxed) {
            let groups = self.high_water().div_ceil(64);
            pass.set_bind_group(0, &self.cull_bg_c, &[]);
            pass.set_bind_group(1, hzb_bg, &[]);
            for (k, p) in self.cull_c.iter().enumerate() {
                pass.set_pipeline(p);
                pass.dispatch_workgroups(if k == 1 { 1 } else { groups }, 1, 1);
            }
        }
        // the wire segments against the same pyramid.
        if self.wires_active.load(Ordering::Relaxed) {
            pass.set_pipeline(&self.wire_cull_c);
            pass.set_bind_group(0, &self.wire_cull_bg_c, &[]);
            pass.set_bind_group(1, hzb_bg, &[]);
            pass.dispatch_workgroups(self.wire_high_water().div_ceil(64), 1, 1);
        }
    }

    /// Inside the (late) city pass, after the opaque city list. Leaves the
    /// props vertex and index buffers bound: the caller binds its own again.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, look: &LookGpu) {
        if !self.active.load(Ordering::Relaxed) {
            return;
        }
        pass.set_bind_group(0, &self.draw_bg, &[]);
        look.bind_main(pass);
        pass.set_bind_group(2, &self.data_bg, &[]);
        pass.set_vertex_buffer(0, self.vbuf.slice(..));
        pass.set_index_buffer(self.ibuf.slice(..), wgpu::IndexFormat::Uint32);
        pass.set_pipeline(&self.opaque);
        self.draw_list(pass, 0);
        pass.set_pipeline(&self.cutout);
        self.draw_list(pass, 1);
        // the cross-fade pair (leaving level 8/9, arriving level 10/11).
        pass.set_pipeline(&self.opaque_fade);
        self.draw_list(pass, 8);
        self.draw_list(pass, 10);
        pass.set_pipeline(&self.cutout_fade);
        self.draw_list(pass, 9);
        self.draw_list(pass, 11);
    }

    /// whether the wire pass will draw this frame (the city pass keeps its depth for it).
    pub fn wires_pending(&self) -> bool {
        self.enabled() && self.wire_high_water() > 0
    }

    /// the wires, a pass of its own after the late city pass (timed).
    /// `color` / `resolve` / `depth` as the city pass had them.
    pub fn draw_wires(&self, enc: &mut wgpu::CommandEncoder, color: &wgpu::TextureView, resolve: Option<&wgpu::TextureView>, depth: &wgpu::TextureView, look: &LookGpu) {
        let ts = self.timing.lock().unwrap().clone();
        if !self.wires_active.load(Ordering::Relaxed) {
            Self::empty_ts(enc, &ts, 4);
            return;
        }
        let tw = ts.as_ref().map(|(q, b)| wgpu::RenderPassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(*b + 4), end_of_pass_write_index: Some(*b + 5) });
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("props-wires"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: color, depth_slice: None, resolve_target: resolve, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: depth, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }), stencil_ops: None }),
            timestamp_writes: tw,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.wire_pipe);
        pass.set_bind_group(0, &self.wire_draw_bg, &[]);
        look.bind_main(&mut pass);
        pass.draw_indirect(&self.wire_args, 0);
    }

    /// The stats words of the last frame (tests; blocks): per view instances and triangles.
    pub fn stats_readback(&self, ctx: &GpuContext) -> Result<Vec<u32>> {
        let size = self.stats.size();
        let staging = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: Some("prop-stats-readback"), size, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(&self.stats, 0, &staging, 0, size);
        ctx.queue.submit([enc.finish()]);
        Ok(bytemuck::pod_collect_to_vec(&crate::read_mapped(ctx, &staging)?))
    }

    /// the wire draw's instance count of the last frame (tests; blocks).
    pub fn wire_drawn_readback(&self, ctx: &GpuContext) -> Result<u32> {
        let staging = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: Some("prop-wire-readback"), size: 32, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(&self.wire_args, 0, &staging, 0, 32);
        ctx.queue.submit([enc.finish()]);
        let w: Vec<u32> = bytemuck::pod_collect_to_vec(&crate::read_mapped(ctx, &staging)?);
        Ok(w[4])
    }
}
