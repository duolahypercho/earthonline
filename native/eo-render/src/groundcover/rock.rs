//! The scanned rock on the GPU: the seven survey scans (eo-landmarks
//! `rocks::scan`) as one vertex and index buffer of their four levels each
//! (a vertex's fourth normal byte its distance to the scan's cut rim),
//! their diffuse, normal and ARM maps as three texture arrays (a layer a
//! scan, 2k with mips; RGBA8 sRGB diffuse, the normal's x y and the ARM's
//! occlusion and roughness as RG8), and the pieces the landmark build
//! placed as one instance buffer (80 bytes a piece: origin, turn, scale,
//! tint, wetness, bounding sphere), uploaded once. `RockGpu::new` runs on a
//! worker thread (groundcover.rs): nothing of it touches the render thread.
//!
//! Per frame (rock_cull.wgsl), one invocation per piece:
//! * before the city pass (`encode_pre`): the three sun cascades (their
//!   lists drawn by the next frame's shadow passes: the casters lag the
//!   cascades by a frame, with a margin on each sphere), and the camera when
//!   occlusion is off this frame;
//! * inside the occlusion pass (`cull_camera`): the camera against this
//!   frame's depth pyramid (the city's own `hzb_occluded`);
//! each a slot in a (view, scan, level) bucket that is one indexed indirect
//! draw. The level is the piece's projected radius in pixels against its
//! scan's edges (`scans::lod_edges`: each level drawn only where its
//! measured error is under a pixel); past `MAX_DISTANCE_M` or under
//! `MIN_PX` nothing is drawn (the distance cull that replaces round 2's
//! always-drawn shore rocks). The cascades get the pieces within
//! `CAST_DISTANCE_M`, a level coarser.

use crate::camera::{Camera, frustum_planes};
use crate::look::LookGpu;
use crate::{DEPTH_FORMAT, IDS_FORMAT};
use bytemuck::{Pod, Zeroable};
use eo_landmarks::rocks::scan::{self as scans, Library, Placed, Role};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use wgpu::util::DeviceExt;

const VIEWS: u64 = 4;
const KINDS: u64 = 7;
const LODS: u64 = 4;
const ARGS_WORDS: u64 = 5;

/// Under this projected radius (pixels) a piece is not drawn (INFERRED:
/// under a pixel across it is a speck the temporal resolve averages away).
pub const MIN_PX: f32 = 0.8;
/// An open scan's cut rim (rock_types.wgsl `gc_frayed`, `gc_rim_sink`):
/// frayed over `RIM_FRAY_SHARE` of the scan's middle extent (between
/// `RIM_FRAY_MIN_M` and the stored range, scan metres), sunk under its rim
/// plane by `RIM_SINK_SHARE` of that band, the fray's noise at
/// `RIM_NOISE_PER_M` cycles a scan metre (all INFERRED: enough that the
/// straight line where the scan stopped breaks into a ragged edge and
/// tucks under the ground, not so much that the rock's face goes).
pub const RIM_FRAY_SHARE: f32 = 0.2;
pub const RIM_FRAY_MIN_M: f32 = 0.25;
pub const RIM_SINK_SHARE: f32 = 0.5;
pub const RIM_NOISE_PER_M: f32 = 1.6;
/// The farthest a piece is drawn (m, the drawn world): the far terrain's
/// band starts beyond it (INFERRED).
pub const MAX_DISTANCE_M: f32 = 2600.0;
/// The farthest a piece casts into the cascades (m), and the margin on its
/// sphere for the cascades' one-frame lag (m).
pub const CAST_DISTANCE_M: f32 = 450.0;
pub const CASCADE_MARGIN_M: f32 = 2.0;
/// The nominal target height the pixel edges are read at (the shots' and the bench's 1080 rows).
pub const NOMINAL_HEIGHT_PX: f32 = 1080.0;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct GcVertex {
    pub pos: [f32; 3],
    pub normal: [i8; 4],
    pub uv: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GcRock {
    pub pos: [f32; 3],
    pub kind: u32,
    pub rot: [f32; 4],
    pub scale: [f32; 3],
    pub wet: f32,
    pub tint: [f32; 3],
    pub radius: f32,
    pub center: [f32; 3],
    pub flags: u32,
}
const _: () = assert!(std::mem::size_of::<GcRock>() == 80);

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GcParams {
    planes: [[f32; 4]; 6],
    cam: [f32; 4],
    view_proj: [[f32; 4]; 4],
    counts: [u32; 4],
    dist: [f32; 4],
    lod: [[f32; 4]; 8],
}

/// draw.wgsl's `Frame`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GcFrame {
    view_proj: [[f32; 4]; 4],
    cam_pos: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    ambient_sky: [f32; 4],
    probe_info: [f32; 4],
    copy_step: [f32; 4],
}

/// What the worker thread needs of the renderer to build `RockGpu`: the
/// look's main bind group layout and its cascade-view buffer (both cheap
/// handles), whether the city pass also writes the debug ids target, and
/// the sample count.
#[derive(Clone)]
pub struct RockSetup {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub main_bgl: wgpu::BindGroupLayout,
    pub shadow_view: wgpu::Buffer,
    pub debug_ids: bool,
    pub sample_count: u32,
}

impl RockSetup {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, look: &LookGpu, debug_ids: bool) -> RockSetup {
        RockSetup { device: device.clone(), queue: queue.clone(), main_bgl: look.main_bind_group_layout().clone(), shadow_view: look.shadow_view_buffer().clone(), debug_ids, sample_count: msaa_from_args() }
    }
}

pub struct RockGpu {
    pub count: u32,
    pub bytes: u64,
    lod: [[f32; 4]; 8],
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    args: wgpu::Buffer,
    args_template: wgpu::Buffer,
    stats: wgpu::Buffer,
    params_s: wgpu::Buffer,
    params_c: wgpu::Buffer,
    frame: wgpu::Buffer,
    cull_bg_s: wgpu::BindGroup,
    cull_bg_c: wgpu::BindGroup,
    cull_s: wgpu::ComputePipeline,
    cull_c: wgpu::ComputePipeline,
    draw_bg: wgpu::BindGroup,
    data_bg: wgpu::BindGroup,
    view_bg: wgpu::BindGroup,
    opaque: wgpu::RenderPipeline,
    clipped: wgpu::RenderPipeline,
    shadow: wgpu::RenderPipeline,
    shadow_clip: wgpu::RenderPipeline,
    camera_late: AtomicBool,
    pub stats_on: bool,
    frames: AtomicU32,
    /// Whether each scan draws through the clipped pipelines (an open scan: its rim frays).
    clip_kind: [bool; KINDS as usize],
}

/// The pieces as the GPU reads them (the bounding sphere in the world).
pub fn gpu_rocks(lib: &Library, placed: &Placed) -> Vec<GcRock> {
    placed
        .pieces
        .iter()
        .map(|p| {
            let k = &lib.kinds[p.kind as usize];
            let (c, r) = k.sphere();
            let cw = scans::quat_rotate(p.rot, [c[0] as f64 * p.scale[0], c[1] as f64 * p.scale[1], c[2] as f64 * p.scale[2]]);
            let smax = p.scale[0].max(p.scale[1]).max(p.scale[2]);
            GcRock {
                pos: p.pos.map(|v| v as f32),
                kind: p.kind as u32,
                rot: p.rot.map(|v| v as f32),
                scale: p.scale.map(|v| v as f32),
                wet: p.wet,
                tint: p.tint,
                radius: (r as f64 * smax) as f32,
                center: [0, 1, 2].map(|i| (p.pos[i] + cw[i]) as f32),
                flags: 0,
            }
        })
        .collect()
}

/// The sample count the app renders with (`--msaa N`, else 1), read here as the props' is passed in.
fn msaa_from_args() -> u32 {
    let a: Vec<String> = std::env::args().collect();
    a.iter().position(|s| s == "--msaa").and_then(|i| a.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(1)
}

/// The fourth normal byte: the vertex's distance to its scan's cut rim, 127 at `RIM_RANGE_M`.
pub fn rim_byte(d: f32) -> i8 {
    ((d / scans::RIM_RANGE_M).clamp(0.0, 1.0) * 127.0).round() as i8
}

/// A scan's cut-rim treatment (rock_types.wgsl `GcKind::fray`): band, sink, noise, range.
pub fn fray_of(kind: &scans::Kind) -> [f32; 4] {
    if kind.rim.is_none() {
        return [0.0, 0.0, 0.0, scans::RIM_RANGE_M];
    }
    let mut ext = [0, 1, 2].map(|i| kind.max[i] - kind.min[i]);
    ext.sort_by(f32::total_cmp);
    let band = (RIM_FRAY_SHARE * ext[1]).clamp(RIM_FRAY_MIN_M, scans::RIM_RANGE_M);
    [band, RIM_SINK_SHARE * band, RIM_NOISE_PER_M, scans::RIM_RANGE_M]
}

impl RockGpu {
    pub fn new(setup: &RockSetup, lib: &Library, placed: &Placed) -> RockGpu {
        let t0 = std::time::Instant::now();
        let d = &setup.device;
        let queue = &setup.queue;
        use wgpu::BufferUsages as U;
        // geometry: every scan's vertices, its four levels' indices after one another
        let mut verts: Vec<GcVertex> = vec![];
        let mut indices: Vec<u32> = vec![];
        let mut lod_ranges = vec![[0u32; 3]; (KINDS * LODS) as usize]; // index count, first index, base vertex
        let mut tris = vec![0u32; (KINDS * LODS) as usize];
        for (k, kind) in lib.kinds.iter().enumerate() {
            let base = verts.len() as u32;
            for i in 0..kind.mesh.positions.len() {
                let n = kind.mesh.normals[i];
                let q = |v: f32| (v.clamp(-1.0, 1.0) * 127.0).round() as i8;
                let rim = kind.rim_dist.get(i).copied().unwrap_or(scans::RIM_RANGE_M);
                verts.push(GcVertex { pos: kind.mesh.positions[i], normal: [q(n[0]), q(n[1]), q(n[2]), rim_byte(rim)], uv: kind.mesh.uvs[i] });
            }
            for (l, lod) in kind.mesh.lods.iter().enumerate().take(LODS as usize) {
                lod_ranges[k * LODS as usize + l] = [lod.len() as u32, indices.len() as u32, base];
                tris[k * LODS as usize + l] = (lod.len() / 3) as u32;
                indices.extend_from_slice(lod);
            }
        }
        let rocks = gpu_rocks(lib, placed);
        let n = rocks.len().max(1) as u64;
        let mut per_kind = [0u64; KINDS as usize];
        for r in &rocks {
            per_kind[r.kind as usize] += 1;
        }
        let mut kind_off = [0u64; KINDS as usize];
        for k in 1..KINDS as usize {
            kind_off[k] = kind_off[k - 1] + per_kind[k - 1];
        }
        // the indirect draws: (view, scan, level) buckets, each its share of the visible list
        let mut template = vec![0u32; (VIEWS * KINDS * LODS * ARGS_WORDS) as usize];
        for v in 0..VIEWS {
            for k in 0..KINDS {
                for l in 0..LODS {
                    let b = ((v * KINDS + k) * LODS + l) as usize;
                    let [count, first, base] = lod_ranges[k as usize * LODS as usize + l as usize];
                    let first_instance = v * n * LODS + kind_off[k as usize] * LODS + l * per_kind[k as usize];
                    template[b * 5..b * 5 + 5].copy_from_slice(&[count, 0, first, base, first_instance as u32]);
                }
            }
        }
        let init = |label: &str, bytes: &[u8], usage: wgpu::BufferUsages| {
            let contents: Vec<u8> = if bytes.is_empty() { vec![0u8; 16] } else { bytes.to_vec() };
            d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents: &contents, usage })
        };
        let empty = |label: &str, size: u64, usage: wgpu::BufferUsages| d.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size.max(16).next_multiple_of(16), usage, mapped_at_creation: false });
        let vbuf = init("gc-rock-vertices", bytemuck::cast_slice(&verts), U::VERTEX);
        let ibuf = init("gc-rock-indices", bytemuck::cast_slice(&indices), U::INDEX);
        let rocks_buf = init("gc-rocks", bytemuck::cast_slice(&rocks), U::STORAGE);
        let visible = empty("gc-rock-visible", VIEWS * n * LODS * 4, U::STORAGE);
        let args = init("gc-rock-args", bytemuck::cast_slice(&template), U::STORAGE | U::INDIRECT | U::COPY_DST);
        let args_template = init("gc-rock-args-template", bytemuck::cast_slice(&template), U::COPY_SRC);
        let stats = empty("gc-rock-stats", 32, U::STORAGE | U::COPY_DST | U::COPY_SRC);
        let tris_buf = init("gc-rock-triangles", bytemuck::cast_slice(&tris), U::STORAGE);
        // per scan: its rim plane (and a patch's clip height), its rim's fray; its level edges
        let mut kinds = [[0.0f32; 4]; 16];
        let mut lod = [[f32::MAX, f32::MAX, f32::MAX, MIN_PX]; 8];
        let mut clip_kind = [false; KINDS as usize];
        for (k, kind) in lib.kinds.iter().enumerate() {
            kinds[2 * k] = match (kind.role, kind.rim) {
                (Role::Patch, Some(r)) => [r.a as f32, r.b as f32, r.c as f32, scans::clip_m(k as u8)],
                (_, Some(r)) => [r.a as f32, r.b as f32, r.c as f32, -1.0e3],
                _ => [0.0, 0.0, 0.0, -1.0e3],
            };
            kinds[2 * k + 1] = fray_of(kind);
            clip_kind[k] = kind.rim.is_some();
            let e = scans::lod_edges(&kind.lod_errors);
            lod[k] = [e[0], e[1], e[2], MIN_PX];
        }
        let kinds_buf = init("gc-rock-kinds", bytemuck::cast_slice(&kinds), U::UNIFORM);
        let params_s = empty("gc-rock-params-pre", std::mem::size_of::<GcParams>() as u64, U::UNIFORM | U::COPY_DST);
        let params_c = empty("gc-rock-params-camera", std::mem::size_of::<GcParams>() as u64, U::UNIFORM | U::COPY_DST);
        let frame = empty("gc-rock-frame", std::mem::size_of::<GcFrame>() as u64, U::UNIFORM | U::COPY_DST);
        let objects = init("gc-rock-object", &[0u8; 64], U::STORAGE);
        let materials = init("gc-rock-material", &[0u8; 64], U::STORAGE);

        // the maps: three arrays, a layer a scan, with box-filtered mips (made in parallel)
        let res = lib.kinds[0].tex_res;
        let mips = res.ilog2() + 1;
        // the normal's x y and the ARM's occlusion and roughness: two channels each (the normal's z is rebuilt, the metalness is 0)
        let chains: Vec<Vec<Vec<u8>>> = std::thread::scope(|s| {
            let hs: Vec<_> = (0..3)
                .flat_map(|m| lib.kinds.iter().map(move |k| (m, k)))
                .map(|(m, k)| {
                    s.spawn(move || {
                        let c = crate::authored::mip_chain(&k.textures[m], k.tex_res as usize, m == 0);
                        if m == 0 { c } else { c.into_iter().map(|l| l.chunks_exact(4).flat_map(|p| [p[0], p[1]]).collect()).collect() }
                    })
                })
                .collect();
            hs.into_iter().map(|h| h.join().expect("mip thread")).collect()
        });
        let mut tex_bytes = 0u64;
        let make_array = |label: &str, format: wgpu::TextureFormat, map: usize, texel: u32| {
            let t = d.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: res, height: res, depth_or_array_layers: KINDS as u32 },
                mip_level_count: mips,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            for layer in 0..KINDS as usize {
                for (mip, level) in chains[map * KINDS as usize + layer].iter().enumerate().take(mips as usize) {
                    let r = (res >> mip).max(1);
                    queue.write_texture(
                        wgpu::TexelCopyTextureInfo { texture: &t, mip_level: mip as u32, origin: wgpu::Origin3d { x: 0, y: 0, z: layer as u32 }, aspect: wgpu::TextureAspect::All },
                        level,
                        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(r * texel), rows_per_image: Some(r) },
                        wgpu::Extent3d { width: r, height: r, depth_or_array_layers: 1 },
                    );
                }
            }
            t
        };
        let t_alb = make_array("gc-rock-albedo", wgpu::TextureFormat::Rgba8UnormSrgb, 0, 4);
        let t_nor = make_array("gc-rock-normal", wgpu::TextureFormat::Rg8Unorm, 1, 2);
        let t_arm = make_array("gc-rock-arm", wgpu::TextureFormat::Rg8Unorm, 2, 2);
        tex_bytes += (4 + 2 + 2) * KINDS * (0..mips).map(|m| ((res >> m).max(1) as u64).pow(2)).sum::<u64>();
        let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        let sampler = d.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("gc-rock-sampler"),
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
        let tex = |binding, vis| entry(binding, vis, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false });

        // cull (group 0); the Hi-Z variant binds the city's pyramid as group 1 (OcclusionGpu::late_bgl's entries)
        let cull_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gc-rock-cull-bgl"),
            entries: &[entry(0, cs, ubuf), entry(1, cs, sbuf(true)), entry(2, cs, sbuf(false)), entry(3, cs, sbuf(false)), entry(4, cs, ubuf), entry(5, cs, sbuf(false)), entry(6, cs, sbuf(true))],
        });
        let hzb_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gc-rock-hzb-bgl"),
            entries: &[entry(0, cs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }), entry(1, cs, ubuf)],
        });
        let cascades = wgpu::BindGroupEntry {
            binding: 4,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &setup.shadow_view, offset: 0, size: Some(std::num::NonZeroU64::new(768).unwrap()) }),
        };
        let cull_bg = |p: &wgpu::Buffer, label: &str| {
            d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &cull_bgl,
                entries: &[bge(0, p), bge(1, &rocks_buf), bge(2, &args), bge(3, &visible), cascades.clone(), bge(5, &stats), bge(6, &tris_buf)],
            })
        };
        let cull_bg_s = cull_bg(&params_s, "gc-rock-cull-bg-pre");
        let cull_bg_c = cull_bg(&params_c, "gc-rock-cull-bg-camera");
        let types = include_str!("rock_types.wgsl");
        let plain_src = format!("{types}\n{}\nfn gc_occluded(mn: vec3<f32>, mx: vec3<f32>) -> bool {{ return false; }}\n", include_str!("rock_cull.wgsl"));
        let hzb_src = format!(
            "{types}\n{}\nstruct HzbInfo {{ screen: vec4<f32> }};\n@group(1) @binding(0) var hzb: texture_2d<f32>;\n@group(1) @binding(1) var<uniform> hzb_info: HzbInfo;\n{}\nfn gc_occluded(mn: vec3<f32>, mx: vec3<f32>) -> bool {{ return hzb_occluded(mn, mx, gc.view_proj, hzb_info.screen.xy); }}\n",
            include_str!("rock_cull.wgsl"),
            crate::occlusion::OCCLUSION_WGSL
        );
        let plain_mod = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("rock_cull.wgsl"), source: wgpu::ShaderSource::Wgsl(plain_src.into()) });
        let hzb_mod = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("rock_cull.wgsl (hzb)"), source: wgpu::ShaderSource::Wgsl(hzb_src.into()) });
        let lay_s = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("gc-rock-cull"), bind_group_layouts: &[Some(&cull_bgl)], immediate_size: 0 });
        let lay_c = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("gc-rock-cull-hzb"), bind_group_layouts: &[Some(&cull_bgl), Some(&hzb_bgl)], immediate_size: 0 });
        let compute = |module: &wgpu::ShaderModule, layout: &wgpu::PipelineLayout| d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("gc-rock-classify"), layout: Some(layout), module, entry_point: Some("classify"), compilation_options: Default::default(), cache: None });
        let cull_s = compute(&plain_mod, &lay_s);
        let cull_c = compute(&hzb_mod, &lay_c);

        // the pieces (group 2 of the colour and shadow pipelines)
        let data_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("gc-rock-data-bgl"), entries: &[entry(0, vf, sbuf(true)), entry(1, vf, sbuf(true)), entry(2, vf, ubuf)] });
        let data_bg = d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("gc-rock-data-bg"), layout: &data_bgl, entries: &[bge(0, &rocks_buf), bge(1, &visible), bge(2, &kinds_buf)] });
        // draw.wgsl's group 0, the scans' arrays in the kit's slots (as the props bind theirs)
        let draw_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gc-rock-draw-bgl"),
            entries: &[
                entry(0, vf, ubuf),
                entry(1, vf, sbuf(true)),
                entry(2, fs, sbuf(true)),
                tex(3, fs),
                tex(4, fs),
                tex(5, fs),
                entry(6, fs, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                tex(14, fs),
                entry(15, fs, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                entry(16, fs, sbuf(true)),
            ],
        });
        let probe_stand_in = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("gc-rock-probe-stand-in"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 6 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let probe_view = view(&probe_stand_in);
        let probe_sh = empty("gc-rock-probe-sh-stand-in", 160, U::STORAGE);
        let (av, nv, ov) = (view(&t_alb), view(&t_nor), view(&t_arm));
        let draw_bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gc-rock-draw-bg"),
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
        let draw_src = format!("{}\n{types}\n{}", crate::look::city_shader_source(), include_str!("rock_draw.wgsl"));
        let draw_mod = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("rock_draw.wgsl"), source: wgpu::ShaderSource::Wgsl(draw_src.into()) });
        let draw_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("gc-rock-draw"), bind_group_layouts: &[Some(&draw_bgl), Some(&setup.main_bgl), Some(&data_bgl)], immediate_size: 0 });
        let attrs = [
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Snorm8x4, offset: 12, shader_location: 1 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 16, shader_location: 2 },
        ];
        let stride = std::mem::size_of::<GcVertex>() as u64;
        let sample_count = setup.sample_count;
        // the city pass's targets: colour, and the debug ids when the renderer writes them
        let make = |label: &str, fs_entry: &str| {
            let color = wgpu::ColorTargetState { format: crate::look::HDR_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL };
            let ids = wgpu::ColorTargetState { format: IDS_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL };
            let targets_ids = [Some(color.clone()), Some(ids)];
            let targets = [Some(color)];
            let entry_name = if setup.debug_ids { format!("{fs_entry}_ids") } else { fs_entry.to_string() };
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&draw_layout),
                vertex: wgpu::VertexState { module: &draw_mod, entry_point: Some("rock_vs"), compilation_options: Default::default(), buffers: &[Some(wgpu::VertexBufferLayout { array_stride: stride, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attrs })] },
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH_FORMAT, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::Greater), stencil: Default::default(), bias: Default::default() }),
                multisample: wgpu::MultisampleState { count: sample_count, ..Default::default() },
                fragment: Some(wgpu::FragmentState { module: &draw_mod, entry_point: Some(&entry_name), compilation_options: Default::default(), targets: if setup.debug_ids { &targets_ids } else { &targets } }),
                multiview_mask: None,
                cache: None,
            })
        };
        let opaque = make("gc-rock-opaque", "rock_fs");
        let clipped = make("gc-rock-clipped", "rock_fs_clip");

        // the sun casters (group 0: the look's cascade view at a dynamic offset)
        let view_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gc-rock-shadow-view-bgl"),
            entries: &[entry(0, wgpu::ShaderStages::VERTEX, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: None })],
        });
        let view_bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gc-rock-shadow-view-bg"),
            layout: &view_bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &setup.shadow_view, offset: 0, size: Some(std::num::NonZeroU64::new(80).unwrap()) }) }],
        });
        let shadow_src = format!("{types}\n{}", include_str!("rock_shadow.wgsl"));
        let shadow_mod = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("rock_shadow.wgsl"), source: wgpu::ShaderSource::Wgsl(shadow_src.into()) });
        let shadow_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("gc-rock-shadow"), bind_group_layouts: &[Some(&view_bgl), None, Some(&data_bgl)], immediate_size: 0 });
        let shadow_attrs = [attrs[0], attrs[1]];
        let shadow_pipe = |label: &str, clip: bool| {
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&shadow_layout),
                vertex: wgpu::VertexState { module: &shadow_mod, entry_point: Some("rock_shadow_vs"), compilation_options: Default::default(), buffers: &[Some(wgpu::VertexBufferLayout { array_stride: stride, step_mode: wgpu::VertexStepMode::Vertex, attributes: &shadow_attrs })] },
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState { format: crate::look::SHADOW_FORMAT, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::Less), stencil: Default::default(), bias: Default::default() }),
                multisample: Default::default(),
                fragment: clip.then(|| wgpu::FragmentState { module: &shadow_mod, entry_point: Some("rock_shadow_clip_fs"), compilation_options: Default::default(), targets: &[] }),
                multiview_mask: None,
                cache: None,
            })
        };
        let shadow = shadow_pipe("gc-rock-shadow", false);
        let shadow_clip = shadow_pipe("gc-rock-shadow-clip", true);
        let bytes = [&vbuf, &ibuf, &rocks_buf, &visible, &args, &args_template, &stats, &tris_buf, &kinds_buf, &params_s, &params_c, &frame].iter().map(|b| b.size()).sum::<u64>() + tex_bytes;
        let stats_on = std::env::var_os("EO_ROCK_STATS").is_some();
        let build_ms = t0.elapsed().as_secs_f64() * 1e3;
        eprintln!(
            "scanned rock: {} pieces drawn from {} scans, {} vertices, {} indices, {:.0} MB on the GPU, built in {:.0} ms on a worker thread; level edges (px) {:?}",
            rocks.len(),
            lib.kinds.len(),
            verts.len(),
            indices.len(),
            bytes as f64 / 1048576.0,
            build_ms,
            lod.iter().take(lib.kinds.len()).map(|e| [e[0].min(9999.0).round(), e[1].min(9999.0).round(), e[2].min(9999.0).round()]).collect::<Vec<_>>()
        );
        RockGpu {
            count: rocks.len() as u32,
            bytes,
            lod,
            vbuf,
            ibuf,
            args,
            args_template,
            stats,
            params_s,
            params_c,
            frame,
            cull_bg_s,
            cull_bg_c,
            cull_s,
            cull_c,
            draw_bg,
            data_bg,
            view_bg,
            opaque,
            clipped,
            shadow,
            shadow_clip,
            camera_late: AtomicBool::new(false),
            stats_on,
            frames: AtomicU32::new(0),
            clip_kind,
        }
    }

    pub fn update(&self, queue: &wgpu::Queue, cam: &Camera, occlusion: bool) {
        let vp = cam.view_proj();
        let px_scale = 0.5 * NOMINAL_HEIGHT_PX / (0.5 * cam.fov_y_deg.to_radians()).tan();
        let base = GcParams {
            planes: frustum_planes(vp).map(|p| p.to_array()),
            cam: [cam.eye.x, cam.eye.y, cam.eye.z, px_scale],
            view_proj: vp.to_cols_array_2d(),
            counts: [self.count, 0, self.stats_on as u32, 0],
            dist: [MAX_DISTANCE_M, CAST_DISTANCE_M, CASCADE_MARGIN_M, 0.0],
            lod: self.lod,
        };
        let s = GcParams { counts: [self.count, 0b1110 | if occlusion { 0 } else { 1 }, self.stats_on as u32, 0], ..base };
        let c = GcParams { counts: [self.count, 0b0001, self.stats_on as u32, 0], ..base };
        queue.write_buffer(&self.params_s, 0, bytemuck::bytes_of(&s));
        queue.write_buffer(&self.params_c, 0, bytemuck::bytes_of(&c));
        self.camera_late.store(occlusion, Ordering::Relaxed);
        let f = GcFrame {
            view_proj: vp.to_cols_array_2d(),
            cam_pos: cam.eye.extend(1.0).to_array(),
            sun_dir: [0.0, 1.0, 0.0, 0.0],
            sun_color: [1.0, 1.0, 1.0, 0.0],
            ambient_sky: [0.0; 4],
            probe_info: [0.0; 4],
            copy_step: [0.0; 4],
        };
        queue.write_buffer(&self.frame, 0, bytemuck::bytes_of(&f));
        self.frames.fetch_add(1, Ordering::Relaxed);
    }

    /// The last frame's counts (pieces and triangles per view; blocks): `EO_ROCK_STATS`.
    pub fn report_stats(&self, device: &wgpu::Device, queue: &wgpu::Queue) {
        if !self.stats_on || self.frames.load(Ordering::Relaxed) % 120 != 60 {
            return;
        }
        let staging = device.create_buffer(&wgpu::BufferDescriptor { label: Some("gc-rock-stats-read"), size: 32, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut enc = device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(&self.stats, 0, &staging, 0, 32);
        queue.submit([enc.finish()]);
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        if rx.recv().is_ok_and(|r| r.is_ok()) {
            let w: Vec<u32> = bytemuck::pod_collect_to_vec(&slice.get_mapped_range().unwrap());
            eprintln!("scanned rock stats: camera {} pieces {} triangles; cascades {:?}", w[0], w[1], [(w[2], w[3]), (w[4], w[5]), (w[6], w[7])]);
        }
        staging.unmap();
    }

    /// Before the city pass: reset the draws, then the cascades' cull (for the next frame's
    /// shadow passes) and the camera's when occlusion is off.
    pub fn encode_pre(&self, enc: &mut wgpu::CommandEncoder) {
        enc.copy_buffer_to_buffer(&self.args_template, 0, &self.args, 0, self.args.size());
        enc.clear_buffer(&self.stats, 0, None);
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("gc-rock-cull"), timestamp_writes: None });
        pass.set_pipeline(&self.cull_s);
        pass.set_bind_group(0, &self.cull_bg_s, &[]);
        pass.dispatch_workgroups(self.count.div_ceil(64), 1, 1);
    }

    pub fn cull_camera(&self, pass: &mut wgpu::ComputePass<'_>, hzb_bg: &wgpu::BindGroup) {
        if !self.camera_late.load(Ordering::Relaxed) {
            return;
        }
        pass.set_pipeline(&self.cull_c);
        pass.set_bind_group(0, &self.cull_bg_c, &[]);
        pass.set_bind_group(1, hzb_bg, &[]);
        pass.dispatch_workgroups(self.count.div_ceil(64), 1, 1);
    }

    fn draws(&self, pass: &mut wgpu::RenderPass<'_>, view: u64, opaque: &wgpu::RenderPipeline, clipped: &wgpu::RenderPipeline) {
        for clip in [false, true] {
            pass.set_pipeline(if clip { clipped } else { opaque });
            for k in 0..KINDS {
                if self.clip_kind[k as usize] != clip {
                    continue;
                }
                for l in 0..LODS {
                    let b = (view * KINDS + k) * LODS + l;
                    pass.draw_indexed_indirect(&self.args, b * ARGS_WORDS * 4);
                }
            }
        }
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, look: &LookGpu) {
        pass.set_bind_group(0, &self.draw_bg, &[]);
        look.bind_main(pass);
        pass.set_bind_group(2, &self.data_bg, &[]);
        pass.set_vertex_buffer(0, self.vbuf.slice(..));
        pass.set_index_buffer(self.ibuf.slice(..), wgpu::IndexFormat::Uint32);
        self.draws(pass, 0, &self.opaque, &self.clipped);
    }

    pub fn draw_shadow(&self, pass: &mut wgpu::RenderPass<'_>, cascade: usize) {
        pass.set_bind_group(0, &self.view_bg, &[(cascade as u64 * 256) as u32]);
        pass.set_bind_group(2, &self.data_bg, &[]);
        pass.set_vertex_buffer(0, self.vbuf.slice(..));
        pass.set_index_buffer(self.ibuf.slice(..), wgpu::IndexFormat::Uint32);
        self.draws(pass, 1 + cascade as u64, &self.shadow, &self.shadow_clip);
    }
}
