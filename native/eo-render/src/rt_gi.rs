//! Ray-traced one-bounce diffuse light (`--rt-gi`, stage R1; opt-in, no
//! tier turns it on): hardware ray queries (wgpu `EXPERIMENTAL_RAY_QUERY`)
//! against the static city, half resolution, one ray a texel, accumulated
//! over frames and filtered, read by the next frame's city pass in place of
//! the rig's sky terms (rt_gi.wgsl, rt_gi_use.wgsl, the hook in
//! look_light.wgsl).
//!
//! What is traced: acceleration structures (BLAS) built from the renderer's
//! own vertex and index arena (no copy of the geometry):
//!
//! * static cells (`RtKey::Cell`, 256 m): the terrain, the street's flat far
//!   strips and the plazas, the grade-separated roads' static level, the
//!   landmarks' near and always-drawn meshes, from the renderer's initial
//!   object table; built when the camera comes within `scene_radius_m`;
//! * kit packs (`RtKey::Kit`): each resident rung of a 128 m kit tile, as
//!   kit.rs reports it resident or evicted, one geometry a building; the near
//!   rung stands in for the mid one where both are built;
//! * hand-built models (`RtKey::Model`), as they are uploaded.
//!
//! Agents, props, trees, water and the street's near packs (kerbs, paint)
//! are not in it (stage R1). One top-level structure (TLAS) over what lies
//! within `scene_radius_m`, rebuilt when that set changes; BLAS builds are
//! spread over frames (`build_triangles` a frame, nearest first).
//!
//! The switch: `ShaderFeatures::rt_gi` composes the read and the hook into
//! the city shaders and adds group 1 bindings 20-22 (`look.rs`); without it
//! the shader text, the layout and every pass are as before (`--no-rt-gi` is
//! today's frame byte for byte). The pass itself (`RtGi`) exists only on a
//! device with ray queries (`GpuContext::ray_query`: Metal only, the one
//! backend tested); eo-app turns the option off with one line elsewhere.
//!
//! Frame-cost diagnostics: `EO_RT_GI_NO_HOOK=1` composes the bindings and the
//! read but not the hook (the pass runs, the city shader ignores it);
//! eo-app's `EO_RT_GI_DEVICE_ONLY=1` asks for the ray-query device with the
//! option off. Measured (stage R1): neither costs the city pass anything;
//! the hook does (see native/README.md `--rt-gi`).

use crate::kit::RangeAlloc;
use crate::lighting;
use crate::scene::{GpuObject, LOD_ALWAYS, LOD_LANDMARK_NEAR, LOD_STREET_FAR, Vertex};
use glam::{Mat4, Vec3};
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use wgpu::util::DeviceExt;

static WANTED: AtomicBool = AtomicBool::new(false);

/// Ask for the ray-query device feature (eo-app, before any `GpuContext`):
/// `GpuContext::new` requests it where the adapter has it on Metal.
pub fn set_wanted(on: bool) {
    WANTED.store(on, Ordering::Relaxed);
}

pub fn wanted() -> bool {
    WANTED.load(Ordering::Relaxed)
}

/// Group 1 (`look-bgl`) bindings of the result, with the option compiled in.
pub const B_RT_GI: u32 = 20;
pub const B_RT_GI_Z: u32 = 21;
pub const B_RT_GI_VIEW: u32 = 22;
/// The result's format (rgb irradiance, a the environment's open share) and its depth's.
pub const GI_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const GI_Z_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
const G_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
/// The history's moments: frame count, the filter guide's mean and mean square.
const MOMENTS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
/// Static cells' size (m).
pub const CELL_M: f32 = 256.0;
/// The TLAS's capacity (instances).
pub const MAX_INSTANCES: u32 = 4096;
/// Geometry table entries (16 bytes each); a TLAS instance's custom data is
/// its first entry, 24 bits.
pub const TABLE_ENTRIES: u32 = 1 << 20;
const STRIDE: u64 = std::mem::size_of::<Vertex>() as u64;

/// The comment in look_light.wgsl the hook replaces.
pub const HOOK_MARKER: &str = "    // RT_GI_HOOK";

/// The hook look_light.wgsl gets with the option: the traced irradiance in
/// place of the rig's (`irr`: the hemisphere light, the environment's
/// diffuse, the ambient volume and its ground bounce) where the previous
/// frame's result covers the surface, and the screen-space bounce and (by
/// `params.x`) occlusion it replaces.
const HOOK: &str = r#"    // (`--rt-gi`, rt_gi.rs) the traced diffuse light where the previous frame's
    // result covers this surface: the rig's ambient, its environment by the
    // traced open share, the traced sky and bounce; the screen-space bounce and
    // (by lx_rt_view.params.x) occlusion give way to it. Never in the probe pass.
    if (frame.copy_step.z < 0.5) {
        let rg = rt_gi_fetch(world);
        let rk = rg.w * lx_rt_view.params.y;
        irr = mix(irr, rg.e + look.ambient.xyz + env_e * env_i * rg.open, rk);
        ao4 = vec4<f32>(ao4.rgb * (1.0 - rk), mix(ao4.a, 1.0, rk * (1.0 - lx_rt_view.params.x)));
    }"#;

/// The hook with `EO_RT_GI_DEBUG` set at start (compiled in only then):
/// 1 the traced diffuse light alone, 2 the open share (grey), 3 the traced
/// irradiance without the environment.
const HOOK_DEBUG: &str = r#"    if (frame.copy_step.z < 0.5) {
        let rg = rt_gi_fetch(world);
        if (rg.w > 0.0) {
            let rt_irr = rg.e + look.ambient.xyz + env_e * env_i * rg.open;
            let dbg = select(select(rg.e, vec3<f32>(rg.open), lx_rt_view.misc.x < 2.5), rt_irr * diffuse / LK_PI, lx_rt_view.misc.x < 1.5);
            return vec4<f32>(dbg, 1.0);
        }
    }"#;

/// look_light.wgsl with the hook spliced in (the option on) or as it is.
pub fn splice_look_light(src: &str, on: bool) -> String {
    if !on {
        return src.to_string();
    }
    assert!(src.contains(HOOK_MARKER), "look_light.wgsl lost its RT_GI_HOOK marker");
    // diagnostic (frame cost): the bindings and the read composed, the hook not spliced.
    if std::env::var_os("EO_RT_GI_NO_HOOK").is_some_and(|v| v == "1") {
        return src.to_string();
    }
    let debug = std::env::var("EO_RT_GI_DEBUG").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(0) > 0;
    src.replace(HOOK_MARKER, if debug { HOOK_DEBUG } else { HOOK })
}

/// The city shader's read of the result (bindings 20-22).
pub const USE_WGSL: &str = include_str!("rt_gi_use.wgsl");

/// The pass's tunables. Defaults are the stage R1 prototype's; every one
/// can be overridden at start for experiments (`from_env`, `EO_RT_GI_*`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RtGiConfig {
    /// A ray's reach (m).
    pub reach_m: f32,
    /// The ray's origin off the surface along its normal: a constant (m) and per metre of view depth.
    pub origin_m: f32,
    pub origin_per_m: f32,
    /// The share of the rig's open-sky light a hit surface takes (its own sky
    /// visibility is not traced: one bounce).
    pub hit_sky: f32,
    /// The most frames the history averages.
    pub max_frames: f32,
    /// The traced light fades out from this view depth to that (m); past it the rig's.
    pub fade_start_m: f32,
    pub fade_end_m: f32,
    /// The filter's normal power and plane distance scale (m).
    pub normal_power: f32,
    pub plane_m: f32,
    /// A-trous passes (0: none; steps 1, 2, 4, 8).
    pub atrous: u32,
    /// How much of the screen-space occlusion stays on the indirect diffuse (0: replaced).
    pub ssao_keep: f32,
    /// The traced light's weight against the rig's (1: all of it).
    pub weight: f32,
    /// Instances within this of the camera (m, to their bounds); BLASes dropped past `drop_radius_m`.
    pub scene_radius_m: f32,
    pub drop_radius_m: f32,
    /// Triangles built into BLASes a frame at most (the nearest first; one larger is built alone).
    pub build_triangles: u64,
    /// The kit's near rung in place of its mid rung where both are resident.
    pub kit_near: bool,
    /// 1: draw the traced diffuse light alone (irradiance x the surface's diffuse
    /// albedo), 2 the open share, 3 the irradiance without the environment.
    pub debug: u32,
}

impl Default for RtGiConfig {
    fn default() -> Self {
        Self {
            reach_m: 200.0,
            origin_m: 0.05,
            origin_per_m: 0.002,
            hit_sky: 0.6,
            max_frames: 24.0,
            fade_start_m: 260.0,
            fade_end_m: 340.0,
            normal_power: 32.0,
            plane_m: 0.25,
            atrous: 3,
            ssao_keep: 0.5,
            weight: 1.0,
            scene_radius_m: 420.0,
            drop_radius_m: 640.0,
            build_triangles: 400_000,
            kit_near: true,
            debug: 0,
        }
    }
}

impl RtGiConfig {
    /// The defaults with any `EO_RT_GI_<NAME>` given (reach, origin, origin_per_m,
    /// hit_sky, frames, fade_start, fade_end, normal_power, plane, atrous, ssao,
    /// weight, radius, drop, build, kit_near, debug).
    pub fn from_env() -> Self {
        let mut c = Self::default();
        let f = |k: &str, v: &mut f32| {
            if let Some(x) = std::env::var(format!("EO_RT_GI_{k}")).ok().and_then(|s| s.parse::<f32>().ok()) {
                *v = x;
            }
        };
        f("REACH", &mut c.reach_m);
        f("ORIGIN", &mut c.origin_m);
        f("ORIGIN_PER_M", &mut c.origin_per_m);
        f("HIT_SKY", &mut c.hit_sky);
        f("FRAMES", &mut c.max_frames);
        f("FADE_START", &mut c.fade_start_m);
        f("FADE_END", &mut c.fade_end_m);
        f("NORMAL_POWER", &mut c.normal_power);
        f("PLANE", &mut c.plane_m);
        f("SSAO", &mut c.ssao_keep);
        f("WEIGHT", &mut c.weight);
        f("RADIUS", &mut c.scene_radius_m);
        f("DROP", &mut c.drop_radius_m);
        let mut x = c.atrous as f32;
        f("ATROUS", &mut x);
        c.atrous = (x as u32).min(4);
        let mut x = c.build_triangles as f32;
        f("BUILD", &mut x);
        c.build_triangles = x.max(1.0) as u64;
        let mut x = c.kit_near as u32 as f32;
        f("KIT_NEAR", &mut x);
        c.kit_near = x > 0.5;
        let mut x = c.debug as f32;
        f("DEBUG", &mut x);
        c.debug = x as u32;
        c.max_frames = c.max_frames.max(1.0);
        c.drop_radius_m = c.drop_radius_m.max(c.scene_radius_m);
        c
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "reach_m": self.reach_m, "origin_m": self.origin_m, "origin_per_m": self.origin_per_m, "hit_sky": self.hit_sky,
            "max_frames": self.max_frames, "fade_start_m": self.fade_start_m, "fade_end_m": self.fade_end_m,
            "normal_power": self.normal_power, "plane_m": self.plane_m, "atrous": self.atrous, "ssao_keep": self.ssao_keep,
            "weight": self.weight, "scene_radius_m": self.scene_radius_m, "drop_radius_m": self.drop_radius_m,
            "build_triangles": self.build_triangles, "kit_near": self.kit_near, "debug": self.debug,
        })
    }
}

/// The view uniform the city shader reads (rt_gi_use.wgsl `RtGiView`).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RtGiView {
    pub vp: [[f32; 4]; 4],
    pub cam: [f32; 4],
    pub fwd: [f32; 4],
    pub params: [f32; 4],
    pub size: [f32; 4],
    pub misc: [f32; 4],
}

/// The trace uniform (rt_gi.wgsl `RtTrace`).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct RtTrace {
    cam: [f32; 4],
    fwd: [f32; 4],
    prev_vp: [[f32; 4]; 4],
    prev_cam: [f32; 4],
    prev_fwd: [f32; 4],
    params: [f32; 4],
    fade: [f32; 4],
    size: [u32; 4],
}

/// One geometry of a BLAS: an indexed range of the shared arena (absolute
/// vertex and index positions) and what its hit reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RtGeom {
    pub base_vertex: u32,
    pub vertex_count: u32,
    pub first_index: u32,
    pub index_count: u32,
    /// The object's material (the kit palette's first entry for a kit building).
    pub material: u32,
    /// `ObjectKind` (bits 0-7 of the object's kind).
    pub kind: u32,
}

/// What a BLAS is built from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RtKey {
    /// A static cell (`CELL_M` grid).
    Cell(i32, i32),
    /// A kit tile's rung (kit.rs `RUNG_NEAR`, `RUNG_MID`).
    Kit(u32, u8),
    /// A hand-built model (its first object).
    Model(u32),
}

struct Built {
    blas: wgpu::Blas,
    table_at: u32,
    table_len: u32,
}

struct Entry {
    geoms: Vec<RtGeom>,
    min: Vec3,
    max: Vec3,
    triangles: u64,
    built: Option<Built>,
}

impl Entry {
    fn distance(&self, p: Vec3) -> f32 {
        (self.min - p).max(p - self.max).max(Vec3::ZERO).length()
    }
}

/// What the pass did, for the bench JSON (`RtGi::stats`).
#[derive(Clone, Copy, Debug, Default)]
pub struct RtGiStats {
    pub frames: u64,
    pub traced_frames: u64,
    pub entries: usize,
    pub blas_built: usize,
    pub blas_triangles: u64,
    pub instances: usize,
    pub instance_triangles: u64,
    pub tlas_builds: u64,
    pub blas_builds: u64,
    pub built_triangles: u64,
    pub most_triangles_a_frame: u64,
    pub blas_dropped: u64,
    pub encode_ms_sum: f64,
    pub encode_ms_max: f64,
}

impl RtGiStats {
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "frames": self.frames, "traced_frames": self.traced_frames, "entries": self.entries,
            "blas_built": self.blas_built, "blas_triangles": self.blas_triangles,
            "instances": self.instances, "instance_triangles": self.instance_triangles,
            "tlas_builds": self.tlas_builds, "blas_builds": self.blas_builds, "built_triangles": self.built_triangles,
            "most_triangles_built_a_frame": self.most_triangles_a_frame, "blas_dropped": self.blas_dropped,
            "encode_cpu_ms_mean": if self.frames > 0 { self.encode_ms_sum / self.frames as f64 } else { 0.0 },
            "encode_cpu_ms_max": self.encode_ms_max,
        })
    }
}

#[derive(Clone, Copy)]
struct Cam {
    vp: Mat4,
    eye: Vec3,
    fwd: Vec3,
}

/// The half-resolution targets and their bind groups (per history parity).
struct Targets {
    size: (u32, u32),
    gi_view: wgpu::TextureView,
    z_view: wgpu::TextureView,
    trace: [wgpu::BindGroup; 2],
    temporal: [wgpu::BindGroup; 2],
    atrous: [Vec<wgpu::BindGroup>; 2],
    bytes: u64,
}

struct State {
    entries: HashMap<RtKey, Entry>,
    table: RangeAlloc,
    /// The TLAS's instances as last built (key order).
    instances: Vec<RtKey>,
    tlas_live: bool,
    frame: u32,
    /// The camera the result in the targets was made with.
    made: Option<Cam>,
    targets: Option<Targets>,
    /// Group 0 over one depth texture.
    g0: Option<(wgpu::Texture, wgpu::BindGroup)>,
    stats: RtGiStats,
}

/// The pass (made only with the option and a device with ray queries).
pub struct RtGi {
    pub config: RtGiConfig,
    device: wgpu::Device,
    queue: wgpu::Queue,
    vertex_buf: wgpu::Buffer,
    index_buf: wgpu::Buffer,
    material_buf: wgpu::Buffer,
    look_buf: wgpu::Buffer,
    trace_buf: wgpu::Buffer,
    view_buf: wgpu::Buffer,
    geo_buf: wgpu::Buffer,
    layer_mean_buf: wgpu::Buffer,
    step_bufs: Vec<wgpu::Buffer>,
    tlas: Mutex<wgpu::Tlas>,
    bgl0: wgpu::BindGroupLayout,
    bgl_trace: wgpu::BindGroupLayout,
    bgl_temporal: wgpu::BindGroupLayout,
    bgl_atrous: wgpu::BindGroupLayout,
    trace: wgpu::ComputePipeline,
    temporal: wgpu::ComputePipeline,
    atrous: wgpu::ComputePipeline,
    state: Mutex<State>,
    /// Bytes allocated on the GPU (buffers and targets; the BLASes are counted by triangle).
    pub bytes: u64,
}

/// Whether a static object of the initial table is traced (`RtKey::Cell`):
/// the terrain, the street's far strips and plazas, the structures' static
/// level, the landmarks' near and always-drawn meshes, the plain extrusions.
pub fn static_object_traced(o: &GpuObject) -> bool {
    let role = o.lod_role;
    match o.kind & 255 {
        1 => true,
        6 | 7 | 11 | 12 => role == LOD_STREET_FAR || role == LOD_ALWAYS,
        9 | 10 => role == LOD_LANDMARK_NEAR || role == LOD_ALWAYS,
        0 => role == LOD_ALWAYS,
        _ => false,
    }
}

fn cell_of(min: [f32; 3], max: [f32; 3]) -> (i32, i32) {
    let c = (Vec3::from(min) + Vec3::from(max)) * 0.5;
    ((c.x / CELL_M).floor() as i32, (c.z / CELL_M).floor() as i32)
}

/// The static cells of an initial object table: each traced object
/// (`static_object_traced`) with its vertex count from its indices (where
/// they are in `indices`; an object written later, the structures' static
/// level, gets the rest of the arena from its base vertex, which the build
/// validates and Metal does not read for indexed geometry).
pub fn static_cells(objects: &[GpuObject], indices: &[u32], vertex_capacity: u32) -> Vec<(RtKey, Vec<RtGeom>, Vec3, Vec3)> {
    let mut cells: HashMap<(i32, i32), (Vec<RtGeom>, Vec3, Vec3)> = HashMap::new();
    for o in objects {
        if o.index_count < 3 || o.copy != 0 || !static_object_traced(o) || o.base_vertex < 0 {
            continue;
        }
        let base = o.base_vertex as u32;
        if base >= vertex_capacity {
            continue;
        }
        let (first, count) = (o.first_index as usize, (o.index_count / 3 * 3) as usize);
        let vertex_count = match indices.get(first..first + count) {
            Some(ix) => ix.iter().copied().max().map(|m| m + 1).unwrap_or(0),
            None => vertex_capacity - base,
        };
        if vertex_count == 0 || base + vertex_count > vertex_capacity {
            continue;
        }
        let g = RtGeom { base_vertex: base, vertex_count, first_index: o.first_index, index_count: count as u32, material: o.material, kind: o.kind & 255 };
        let e = cells.entry(cell_of(o.min, o.max)).or_insert_with(|| (vec![], Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)));
        e.0.push(g);
        e.1 = e.1.min(Vec3::from(o.min));
        e.2 = e.2.max(Vec3::from(o.max));
    }
    let mut out: Vec<_> = cells.into_iter().map(|(k, (g, mn, mx))| (RtKey::Cell(k.0, k.1), g, mn, mx)).collect();
    out.sort_by_key(|e| e.0);
    out
}

/// A model's geometries (`Renderer::upload_model`): each object's range, its
/// vertex count from its indices (`indices` start at `index_at`).
pub fn model_geoms(objects: &[GpuObject], index_at: u32, indices: &[u32]) -> (Vec<RtGeom>, Vec3, Vec3) {
    let (mut mn, mut mx) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    let mut out = vec![];
    for o in objects {
        if o.index_count < 3 || o.base_vertex < 0 || o.first_index < index_at {
            continue;
        }
        let first = (o.first_index - index_at) as usize;
        let count = (o.index_count / 3 * 3) as usize;
        let Some(ix) = indices.get(first..first + count) else { continue };
        let vc = ix.iter().copied().max().map(|m| m + 1).unwrap_or(0);
        out.push(RtGeom { base_vertex: o.base_vertex as u32, vertex_count: vc, first_index: o.first_index, index_count: count as u32, material: o.material, kind: o.kind & 255 });
        mn = mn.min(Vec3::from(o.min));
        mx = mx.max(Vec3::from(o.max));
    }
    (out, mn, mx)
}

/// The trace shader's source (compiled with ray queries).
pub fn shader_source() -> String {
    format!("enable wgpu_ray_query;\n{}\n{}\n{}", crate::look::look_common_source(), crate::look::street_constants_wgsl(), include_str!("rt_gi.wgsl"))
}

fn target(device: &wgpu::Device, label: &str, size: (u32, u32), format: wgpu::TextureFormat) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

/// What the renderer hands the pass at its creation.
pub struct RtGiInit<'a> {
    pub vertex_buf: &'a wgpu::Buffer,
    pub index_buf: &'a wgpu::Buffer,
    pub material_buf: &'a wgpu::Buffer,
    pub look_buf: &'a wgpu::Buffer,
    /// Kit map layers (the mean albedo table's length).
    pub kit_layers: u32,
}

impl RtGi {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, init: RtGiInit) -> Self {
        let config = RtGiConfig::from_env();
        let uni = wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST;
        let trace_buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("rt-gi-trace"), size: std::mem::size_of::<RtTrace>() as u64, usage: uni, mapped_at_creation: false });
        let view_buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("rt-gi-view"), size: std::mem::size_of::<RtGiView>() as u64, usage: uni, mapped_at_creation: false });
        let geo_buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("rt-gi-geometries"), size: TABLE_ENTRIES as u64 * 16, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let layers = init.kit_layers.max(1);
        let grey: Vec<[f32; 4]> = vec![[0.5, 0.5, 0.5, 1.0]; layers as usize];
        let layer_mean_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("rt-gi-layer-means"), contents: bytemuck::cast_slice(&grey), usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST });
        // the a-trous steps (x the step, y 1 on the last pass, z the pass).
        let passes = config.atrous.max(1);
        let step_bufs = (0..passes)
            .map(|k| {
                let step = if config.atrous == 0 { 0 } else { 1u32 << k };
                let last = (k + 1 == passes) as u32;
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("rt-gi-step"), contents: bytemuck::cast_slice(&[step, last, k, 0u32]), usage: wgpu::BufferUsages::UNIFORM })
            })
            .collect();
        let tlas = device.create_tlas(&wgpu::CreateTlasDescriptor { label: Some("rt-gi-tlas"), max_instances: MAX_INSTANCES, flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE, update_mode: wgpu::AccelerationStructureUpdateMode::Build });

        let cs = wgpu::ShaderStages::COMPUTE;
        let e = |binding, ty| wgpu::BindGroupLayoutEntry { binding, visibility: cs, ty, count: None };
        let ubuf = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None };
        let sbuf = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None };
        let tex = wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false };
        let store = |format| wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format, view_dimension: wgpu::TextureViewDimension::D2 };
        let bgl0 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rt-gi-bgl0"),
            entries: &[
                e(0, ubuf),
                e(1, ubuf),
                e(2, wgpu::BindingType::AccelerationStructure { vertex_return: false }),
                e(3, sbuf),
                e(4, sbuf),
                e(5, sbuf),
                e(6, sbuf),
                e(7, sbuf),
                e(8, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
            ],
        });
        let bgl_trace = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("rt-gi-bgl-trace"), entries: &[e(4, store(GI_FORMAT)), e(5, store(G_FORMAT))] });
        let bgl_temporal = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rt-gi-bgl-temporal"),
            entries: &[e(0, tex), e(1, tex), e(2, tex), e(3, tex), e(8, tex), e(4, store(GI_FORMAT)), e(5, store(MOMENTS_FORMAT))],
        });
        let bgl_atrous = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rt-gi-bgl-atrous"),
            entries: &[e(0, tex), e(1, tex), e(2, tex), e(4, store(GI_FORMAT)), e(6, store(GI_Z_FORMAT)), e(7, ubuf)],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("rt_gi.wgsl"), source: wgpu::ShaderSource::Wgsl(shader_source().into()) });
        let pipe = |label: &str, bgl: &wgpu::BindGroupLayout| {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(label), bind_group_layouts: &[Some(&bgl0), Some(bgl)], immediate_size: 0 });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(label), layout: Some(&layout), module: &module, entry_point: Some(label), compilation_options: Default::default(), cache: None })
        };
        let trace = pipe("rt_trace", &bgl_trace);
        let temporal = pipe("rt_temporal", &bgl_temporal);
        let atrous = pipe("rt_atrous", &bgl_atrous);
        let bytes = TABLE_ENTRIES as u64 * 16 + layers as u64 * 16 + 512;
        Self {
            config,
            device: device.clone(),
            queue: queue.clone(),
            vertex_buf: init.vertex_buf.clone(),
            index_buf: init.index_buf.clone(),
            material_buf: init.material_buf.clone(),
            look_buf: init.look_buf.clone(),
            trace_buf,
            view_buf,
            geo_buf,
            layer_mean_buf,
            step_bufs,
            tlas: Mutex::new(tlas),
            bgl0,
            bgl_trace,
            bgl_temporal,
            bgl_atrous,
            trace,
            temporal,
            atrous,
            state: Mutex::new(State { entries: HashMap::new(), table: RangeAlloc::new(TABLE_ENTRIES), instances: vec![], tlas_live: false, frame: 0, made: None, targets: None, g0: None, stats: RtGiStats::default() }),
            bytes,
        }
    }

    /// The uniform the city shader reads (group 1 binding 22).
    pub fn view_buffer(&self) -> &wgpu::Buffer {
        &self.view_buf
    }

    /// Register a BLAS source (replacing one of the same key).
    pub fn add(&self, key: RtKey, geoms: Vec<RtGeom>, min: Vec3, max: Vec3) {
        let geoms: Vec<RtGeom> = geoms.into_iter().filter(|g| g.index_count >= 3 && g.vertex_count > 0).collect();
        if geoms.is_empty() {
            self.remove(key);
            return;
        }
        let triangles = geoms.iter().map(|g| g.index_count as u64 / 3).sum();
        let mut st = self.state.lock().unwrap();
        if let Some(old) = st.entries.insert(key, Entry { geoms, min, max, triangles, built: None }) {
            if let Some(b) = old.built {
                st.table.free(b.table_at, b.table_len);
            }
        }
    }

    /// Forget a source (its BLAS leaves the TLAS at the next encode).
    pub fn remove(&self, key: RtKey) {
        let mut st = self.state.lock().unwrap();
        if let Some(old) = st.entries.remove(&key) {
            if let Some(b) = old.built {
                st.table.free(b.table_at, b.table_len);
            }
        }
    }

    /// The kit maps' layers landed: their mean albedo (the last mip, linear).
    pub fn set_layer_means(&self, first: usize, means: &[[f32; 4]]) {
        if !means.is_empty() {
            self.queue.write_buffer(&self.layer_mean_buf, first as u64 * 16, bytemuck::cast_slice(means));
        }
    }

    pub fn stats(&self) -> RtGiStats {
        let st = self.state.lock().unwrap();
        let mut s = st.stats;
        s.entries = st.entries.len();
        s.blas_built = st.entries.values().filter(|e| e.built.is_some()).count();
        s.blas_triangles = st.entries.values().filter(|e| e.built.is_some()).map(|e| e.triangles).sum();
        s
    }

    /// Bytes of the targets (once made).
    pub fn target_bytes(&self) -> u64 {
        self.state.lock().unwrap().targets.as_ref().map(|t| t.bytes).unwrap_or(0)
    }

    fn make_targets(&self, size: (u32, u32), look: &crate::look::LookGpu) -> Targets {
        let d = &self.device;
        let cur = target(d, "rt-gi-sample", size, GI_FORMAT);
        let g = [target(d, "rt-gi-g0", size, G_FORMAT), target(d, "rt-gi-g1", size, G_FORMAT)];
        let hist = [target(d, "rt-gi-hist0", size, GI_FORMAT), target(d, "rt-gi-hist1", size, GI_FORMAT)];
        let cnt = [target(d, "rt-gi-moments0", size, MOMENTS_FORMAT), target(d, "rt-gi-moments1", size, MOMENTS_FORMAT)];
        let ping = [target(d, "rt-gi-a", size, GI_FORMAT), target(d, "rt-gi-b", size, GI_FORMAT)];
        let out = target(d, "rt-gi", size, GI_FORMAT);
        let z = target(d, "rt-gi-z", size, GI_Z_FORMAT);
        use wgpu::BindingResource::TextureView as T;
        let bg = |layout: &wgpu::BindGroupLayout, entries: &[wgpu::BindGroupEntry]| d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("rt-gi-bg"), layout, entries });
        let be = |binding: u32, resource| wgpu::BindGroupEntry { binding, resource };
        let trace = [0, 1].map(|p| bg(&self.bgl_trace, &[be(4, T(&cur)), be(5, T(&g[p]))]));
        let temporal = [0, 1].map(|p| {
            bg(&self.bgl_temporal, &[be(0, T(&cur)), be(1, T(&g[p])), be(2, T(&g[1 - p])), be(3, T(&hist[1 - p])), be(8, T(&cnt[1 - p])), be(4, T(&hist[p])), be(5, T(&cnt[p]))])
        });
        let passes = self.step_bufs.len();
        let atrous = [0, 1].map(|p| {
            let mut src = &hist[p];
            let mut v = vec![];
            for k in 0..passes {
                let dst = if k + 1 == passes { &out } else { &ping[k % 2] };
                v.push(bg(&self.bgl_atrous, &[be(0, T(src)), be(1, T(&g[p])), be(2, T(&cnt[p])), be(4, T(dst)), be(6, T(&z)), be(7, self.step_bufs[k].as_entire_binding())]));
                src = dst;
            }
            v
        });
        let px = size.0 as u64 * size.1 as u64;
        let bytes = px * (8 + 2 * 16 + 2 * 8 + 2 * 16 + 2 * 8 + 8 + 4);
        look.set_rt_gi(&out, &z, &self.view_buf);
        Targets { size, gi_view: out, z_view: z, trace, temporal, atrous, bytes }
    }

    /// Builds and drops BLASes round `eye` (at most `budget` triangles, one
    /// larger alone) and rebuilds the TLAS when the traced set changed. Returns
    /// how many wanted sources are still unbuilt.
    fn update_scene(&self, st: &mut State, enc: &mut wgpu::CommandEncoder, eye: Vec3, budget: u64) -> usize {
        let cfg = self.config;
        // 1. what is traced: every source within the radius; for a kit tile,
        //    the near rung where it is built (and wanted), else the mid one.
        let mut want: Vec<(f32, RtKey)> = st.entries.iter().map(|(k, e)| (e.distance(eye), *k)).filter(|(d, _)| *d <= cfg.scene_radius_m).collect();
        want.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        // 2. builds: the nearest unbuilt first, within the budget (one larger alone).
        let mut budget = budget;
        let mut new_blas: Vec<(RtKey, wgpu::Blas, Vec<wgpu::BlasTriangleGeometrySizeDescriptor>)> = vec![];
        let mut built_now = 0u64;
        for (_, k) in &want {
            let e = &st.entries[k];
            if e.built.is_some() {
                continue;
            }
            if let RtKey::Kit(t, r) = *k {
                // a near rung not wanted, or a mid rung whose near rung stands in for it.
                if r == crate::kit::RUNG_NEAR && !cfg.kit_near {
                    continue;
                }
                if r == crate::kit::RUNG_MID && cfg.kit_near && st.entries.get(&RtKey::Kit(t, crate::kit::RUNG_NEAR)).is_some_and(|n| n.built.is_some()) {
                    continue;
                }
            }
            if e.triangles > budget && built_now > 0 {
                break;
            }
            budget = budget.saturating_sub(e.triangles);
            built_now += e.triangles;
            let sizes: Vec<wgpu::BlasTriangleGeometrySizeDescriptor> = e
                .geoms
                .iter()
                .map(|g| wgpu::BlasTriangleGeometrySizeDescriptor { vertex_format: wgpu::VertexFormat::Float32x3, vertex_count: g.vertex_count, index_format: Some(wgpu::IndexFormat::Uint32), index_count: Some(g.index_count), flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE })
                .collect();
            let blas = self.device.create_blas(
                &wgpu::CreateBlasDescriptor { label: Some("rt-gi-blas"), flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE, update_mode: wgpu::AccelerationStructureUpdateMode::Build },
                wgpu::BlasGeometrySizeDescriptors::Triangles { descriptors: sizes.clone() },
            );
            new_blas.push((*k, blas, sizes));
            if budget == 0 {
                break;
            }
        }
        // the new BLASes' geometry table entries.
        let mut tables = vec![];
        for (k, _, _) in &new_blas {
            let geos: Vec<[u32; 4]> = st.entries[k].geoms.iter().map(|g| [g.base_vertex, g.first_index, g.material, g.kind]).collect();
            let n = geos.len() as u32;
            let Some(at) = st.table.alloc(n) else {
                tables.push(None);
                continue;
            };
            self.queue.write_buffer(&self.geo_buf, at as u64 * 16, bytemuck::cast_slice(&geos));
            tables.push(Some((at, n)));
        }
        // 3. drops: built BLASes now far away.
        let far: Vec<RtKey> = st.entries.iter().filter(|(_, e)| e.built.is_some() && e.distance(eye) > cfg.drop_radius_m).map(|(k, _)| *k).collect();
        for k in far {
            if let Some(b) = st.entries.get_mut(&k).and_then(|e| e.built.take()) {
                st.table.free(b.table_at, b.table_len);
                st.stats.blas_dropped += 1;
            }
        }
        // the builds this frame.
        let entries: Vec<wgpu::BlasBuildEntry> = new_blas
            .iter()
            .zip(&tables)
            .filter(|(_, t)| t.is_some())
            .map(|((k, blas, sizes), _)| wgpu::BlasBuildEntry {
                blas,
                geometry: wgpu::BlasGeometries::TriangleGeometries(
                    st.entries[k]
                        .geoms
                        .iter()
                        .zip(sizes)
                        .map(|(g, s)| wgpu::BlasTriangleGeometry { size: s, vertex_buffer: &self.vertex_buf, first_vertex: g.base_vertex, vertex_stride: STRIDE, index_buffer: Some(&self.index_buf), first_index: Some(g.first_index), transform_buffer: None, transform_buffer_offset: None })
                        .collect(),
                ),
            })
            .collect();
        let n_builds = entries.len();
        // 4. the instances: what is wanted and built (or built this frame).
        let built_keys: std::collections::HashSet<RtKey> = new_blas.iter().zip(&tables).filter(|(_, t)| t.is_some()).map(|((k, _, _), _)| *k).collect();
        let is_built = |st: &State, k: &RtKey| st.entries.get(k).is_some_and(|e| e.built.is_some()) || built_keys.contains(k);
        let mut inst: Vec<RtKey> = want
            .iter()
            .map(|(_, k)| *k)
            .filter(|k| is_built(&st, k))
            .filter(|k| match *k {
                RtKey::Kit(t, r) if r == crate::kit::RUNG_MID => !(cfg.kit_near && is_built(&st, &RtKey::Kit(t, crate::kit::RUNG_NEAR))),
                RtKey::Kit(_, r) if r == crate::kit::RUNG_NEAR => cfg.kit_near,
                _ => true,
            })
            .take(MAX_INSTANCES as usize)
            .collect();
        inst.sort();
        let rebuild = inst != st.instances || n_builds > 0;
        let mut tlas = self.tlas.lock().unwrap();
        if rebuild && !inst.is_empty() {
            // the table entries of this frame's builds, looked up before they are stored.
            let table_of: HashMap<RtKey, u32> = new_blas.iter().zip(&tables).filter_map(|((k, _, _), t)| t.map(|t| (*k, t.0))).collect();
            let blas_of = |k: &RtKey| -> (&wgpu::Blas, u32) {
                match new_blas.iter().find(|(nk, _, _)| nk == k) {
                    Some((_, b, _)) if table_of.contains_key(k) => (b, table_of[k]),
                    _ => {
                        let b = st.entries[k].built.as_ref().unwrap();
                        (&b.blas, b.table_at)
                    }
                }
            };
            let old = st.instances.len();
            for (i, k) in inst.iter().enumerate() {
                let (b, at) = blas_of(k);
                tlas[i] = Some(wgpu::TlasInstance::new(b, [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0], at, 0xff));
            }
            for i in inst.len()..old.max(inst.len()) {
                tlas[i] = None;
            }
            enc.build_acceleration_structures(entries.iter(), std::iter::once(&*tlas));
            st.stats.tlas_builds += 1;
        } else if n_builds > 0 {
            enc.build_acceleration_structures(entries.iter(), std::iter::empty());
        }
        drop(entries);
        // the built BLASes join their entries.
        for ((k, blas, _), t) in new_blas.into_iter().zip(tables) {
            if let Some((at, n)) = t {
                if let Some(e) = st.entries.get_mut(&k) {
                    st.stats.blas_builds += 1;
                    st.stats.built_triangles += e.triangles;
                    e.built = Some(Built { blas, table_at: at, table_len: n });
                }
            }
        }
        st.stats.most_triangles_a_frame = st.stats.most_triangles_a_frame.max(built_now);
        if rebuild && !inst.is_empty() {
            st.tlas_live = true;
        }
        st.stats.instances = inst.len();
        st.stats.instance_triangles = inst.iter().map(|k| st.entries[k].triangles).sum();
        st.instances = inst;
        drop(tlas);
        // still to build: wanted, needed (a mid rung whose near rung is built is not) and unbuilt.
        want.iter()
            .filter(|(_, k)| st.entries.get(k).is_some_and(|e| e.built.is_none()))
            .filter(|(_, k)| match *k {
                RtKey::Kit(_, r) if r == crate::kit::RUNG_NEAR => cfg.kit_near,
                RtKey::Kit(t, r) if r == crate::kit::RUNG_MID => !(cfg.kit_near && st.entries.get(&RtKey::Kit(t, crate::kit::RUNG_NEAR)).is_some_and(|n| n.built.is_some())),
                _ => true,
            })
            .count()
    }

    /// (bench settling, no frame drawn) Build what the camera of the last
    /// `update` wants, several frames' budget at a time; true when nothing
    /// wanted is left unbuilt, so a capture starts on the whole traced scene.
    pub fn prepare(&self, look: &crate::look::LookGpu) -> bool {
        let Some((_, eye, _, _)) = look.current_camera() else { return true };
        let mut guard = self.state.lock().unwrap();
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("rt-gi-prepare") });
        let left = self.update_scene(&mut guard, &mut enc, eye, self.config.build_triangles * 4);
        drop(guard);
        self.queue.submit([enc.finish()]);
        left == 0
    }

    /// The frame's pass, after the city pass (its depth stored): BLAS builds
    /// and the TLAS when the traced set changed, then trace, temporal and
    /// a-trous at half resolution. Writes its timestamps in `lighting::Q_RT_GI`.
    pub fn encode(&self, enc: &mut wgpu::CommandEncoder, look: &crate::look::LookGpu, depth: &wgpu::TextureView, ts: lighting::Ts<'_>) {
        let t0 = std::time::Instant::now();
        let mut guard = self.state.lock().unwrap();
        let st: &mut State = &mut guard;
        st.stats.frames += 1;
        let dt = depth.texture();
        let usable = dt.sample_count() == 1 && dt.usage().contains(wgpu::TextureUsages::TEXTURE_BINDING) && dt.format() == crate::DEPTH_FORMAT;
        let Some((vp, eye, fwd, _)) = look.current_camera().filter(|_| usable) else {
            st.made = None;
            self.write_view(None, (1, 1));
            lighting::empty_span(enc, ts, lighting::Q_RT_GI, "rt-gi (idle)");
            return;
        };
        let cur = Cam { vp, eye, fwd };
        let cfg = self.config;

        self.update_scene(st, enc, eye, cfg.build_triangles);
        if !st.tlas_live || st.instances.is_empty() {
            st.made = None;
            self.write_view(None, (1, 1));
            lighting::empty_span(enc, ts, lighting::Q_RT_GI, "rt-gi (nothing traced)");
            return;
        }

        // 5. the targets and the passes.
        let size = ((dt.width() / 2).max(1), (dt.height() / 2).max(1));
        if st.targets.as_ref().map(|t| t.size) != Some(size) {
            st.targets = Some(self.make_targets(size, look));
            st.made = None;
        }
        if st.g0.as_ref().is_none_or(|(t, _)| t != dt) {
            crate::hitch::mark(crate::hitch::Mark::BindGroupCreated, 1);
            let tlas = self.tlas.lock().unwrap();
            let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("rt-gi-bg0"),
                layout: &self.bgl0,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: self.look_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: self.trace_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: tlas.as_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: self.vertex_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: self.index_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 5, resource: self.geo_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 6, resource: self.material_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 7, resource: self.layer_mean_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(depth) },
                ],
            });
            st.g0 = Some((dt.clone(), bg));
        }
        // the city pass of this frame reads the result made with the previous camera.
        self.write_view(st.made, size);
        st.frame = st.frame.wrapping_add(1);
        let prev = st.made;
        let tr = RtTrace {
            cam: [eye.x, eye.y, eye.z, (st.frame % (1 << 23)) as f32],
            fwd: [fwd.x, fwd.y, fwd.z, cfg.reach_m],
            prev_vp: prev.map(|p| p.vp.to_cols_array_2d()).unwrap_or([[0.0; 4]; 4]),
            prev_cam: prev.map(|p| p.eye.extend(1.0).to_array()).unwrap_or([0.0; 4]),
            prev_fwd: prev.map(|p| p.fwd.extend(0.0).to_array()).unwrap_or([0.0; 4]),
            params: [cfg.origin_m, cfg.origin_per_m, cfg.hit_sky, cfg.max_frames],
            fade: [cfg.fade_start_m, cfg.fade_end_m, cfg.normal_power, cfg.plane_m],
            size: [size.0, size.1, dt.width(), dt.height()],
        };
        self.queue.write_buffer(&self.trace_buf, 0, bytemuck::bytes_of(&tr));
        let p = (st.frame % 2) as usize;
        {
            let t = st.targets.as_ref().unwrap();
            let g0 = &st.g0.as_ref().unwrap().1;
            let groups = (size.0.div_ceil(8), size.1.div_ceil(8));
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("rt-gi"), timestamp_writes: lighting::compute_writes(ts, lighting::Q_RT_GI) });
            pass.set_bind_group(0, g0, &[]);
            pass.set_pipeline(&self.trace);
            pass.set_bind_group(1, &t.trace[p], &[]);
            pass.dispatch_workgroups(groups.0, groups.1, 1);
            pass.set_pipeline(&self.temporal);
            pass.set_bind_group(1, &t.temporal[p], &[]);
            pass.dispatch_workgroups(groups.0, groups.1, 1);
            pass.set_pipeline(&self.atrous);
            for b in &t.atrous[p] {
                pass.set_bind_group(1, b, &[]);
                pass.dispatch_workgroups(groups.0, groups.1, 1);
            }
        }
        st.made = Some(cur);
        st.stats.traced_frames += 1;
        let ms = t0.elapsed().as_secs_f64() * 1e3;
        st.stats.encode_ms_sum += ms;
        st.stats.encode_ms_max = st.stats.encode_ms_max.max(ms);
    }

    /// The city pass's view of the result: the camera it was made with (none: no result).
    fn write_view(&self, made: Option<Cam>, size: (u32, u32)) {
        let c = self.config;
        let v = RtGiView {
            vp: made.map(|m| m.vp.to_cols_array_2d()).unwrap_or([[0.0; 4]; 4]),
            cam: made.map(|m| m.eye.extend(1.0).to_array()).unwrap_or([0.0; 4]),
            fwd: made.map(|m| m.fwd.extend(0.0).to_array()).unwrap_or([0.0; 4]),
            params: [c.ssao_keep, c.weight, c.fade_start_m, c.fade_end_m],
            size: [size.0 as f32, size.1 as f32, 0.0, 0.0],
            misc: [c.debug as f32, 0.0, 0.0, 0.0],
        };
        self.queue.write_buffer(&self.view_buf, 0, bytemuck::bytes_of(&v));
    }

    /// The result's views (tests).
    pub fn result_views(&self) -> Option<(wgpu::TextureView, wgpu::TextureView)> {
        self.state.lock().unwrap().targets.as_ref().map(|t| (t.gi_view.clone(), t.z_view.clone()))
    }
}

/// The mean of a kit layer's albedo (its last mip: one sRGB texel) in linear rgb.
pub fn layer_mean(albedo_mips: &[Vec<u8>]) -> [f32; 4] {
    let lin = |c: u8| {
        let c = c as f32 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    match albedo_mips.last() {
        Some(t) if t.len() >= 4 => [lin(t[0]), lin(t[1]), lin(t[2]), 1.0],
        _ => [0.5, 0.5, 0.5, 1.0],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::ObjectKind;

    #[test]
    fn the_hook_is_spliced_only_with_the_option() {
        let src = include_str!("look_light.wgsl");
        assert_eq!(splice_look_light(src, false), src, "off: look_light.wgsl as it is");
        let on = splice_look_light(src, true);
        assert!(!on.contains(HOOK_MARKER) && on.contains("rt_gi_fetch(world)"), "on: the hook in the marker's place");
        assert_eq!(src.matches(HOOK_MARKER).count(), 1, "one marker");
    }

    #[test]
    fn static_cells_take_the_traced_kinds_with_their_vertex_counts() {
        let mut objs = vec![];
        let mut t = GpuObject::new(ObjectKind::Terrain, [0.0; 3], [10.0, 1.0, 10.0], 0, 6, 0);
        t.lod_role = crate::scene::LOD_TERRAIN;
        objs.push(t);
        let mut s = GpuObject::new(ObjectKind::Street, [300.0, 0.0, 0.0], [310.0, 1.0, 10.0], 6, 3, 4);
        s.lod_role = crate::scene::LOD_STREET_NEAR; // streamed: not static
        objs.push(s);
        let mut f = GpuObject::new(ObjectKind::Street, [300.0, 0.0, 0.0], [310.0, 1.0, 10.0], 6, 3, 4);
        f.lod_role = LOD_STREET_FAR;
        objs.push(f);
        let k = GpuObject::new(ObjectKind::Kit, [0.0; 3], [1.0; 3], 0, 3, 0);
        objs.push(k);
        let late = GpuObject::new(ObjectKind::Terrain, [0.0; 3], [1.0; 3], 100, 3, 50); // indices not in the table yet
        objs.push(late);
        let indices = [0, 1, 2, 2, 3, 0, 0, 1, 2];
        let cells = static_cells(&objs, &indices, 64);
        assert_eq!(cells.len(), 2, "{cells:?}");
        let a = cells.iter().find(|c| c.0 == RtKey::Cell(0, 0)).unwrap();
        assert_eq!(a.1.len(), 2, "the terrain and the late terrain");
        assert_eq!(a.1[0].vertex_count, 4);
        assert_eq!(a.1[1].vertex_count, 64 - 50, "a range not yet written: the rest of the arena");
        let b = cells.iter().find(|c| c.0 == RtKey::Cell(1, 0)).unwrap();
        assert_eq!(b.1.len(), 1, "the far strip only");
        assert_eq!((b.1[0].base_vertex, b.1[0].vertex_count, b.1[0].kind), (4, 3, ObjectKind::Street as u32));
    }

    #[test]
    fn uniforms_match_their_wgsl_structs() {
        let src = format!("{}\n@group(1) @binding(0) var<uniform> look: Look;\n@group(1) @binding(5) var lk_ao_samp: sampler;\n{}", crate::look::look_common_source(), USE_WGSL);
        let m = wgpu::naga::front::wgsl::parse_str(&src).expect("rt_gi_use.wgsl parses");
        let (_, ty) = m.types.iter().find(|(_, t)| t.name.as_deref() == Some("RtGiView")).expect("RtGiView");
        let wgpu::naga::TypeInner::Struct { span, .. } = ty.inner else { panic!() };
        assert_eq!(span as usize, std::mem::size_of::<RtGiView>());
        let src = shader_source();
        let m = wgpu::naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
        let (_, ty) = m.types.iter().find(|(_, t)| t.name.as_deref() == Some("RtTrace")).expect("RtTrace");
        let wgpu::naga::TypeInner::Struct { span, .. } = ty.inner else { panic!() };
        assert_eq!(span as usize, std::mem::size_of::<RtTrace>());
        wgpu::naga::valid::Validator::new(wgpu::naga::valid::ValidationFlags::all(), wgpu::naga::valid::Capabilities::all())
            .validate(&m)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
    }

    /// Every lit pipeline's shader as composed with the option off is, after
    /// translation to Metal, the shader of the frame before the option existed:
    /// the composition differs from the old one only by the hook's marker
    /// comment, which the front end drops (`--no-rt-gi` is today's frame byte
    /// for byte at the shader level; the layout and the device features are
    /// unchanged without the option, see `look::RtGiBindings` and `GpuContext::ray_query`).
    #[cfg(target_os = "macos")]
    #[test]
    fn the_shaders_without_the_option_translate_as_before() {
        use wgpu::naga;
        let msl = |label: &str, src: &str| -> String {
            let m = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(src)));
            let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).unwrap_or_else(|e| panic!("{label}: {e:?}"));
            let opts = naga::back::msl::Options { lang_version: (2, 4), ..Default::default() };
            naga::back::msl::write_string(&m, &info, &opts, &naga::back::msl::PipelineOptions::default()).unwrap_or_else(|e| panic!("{label}: {e:?}")).0
        };
        for (name, f) in [("none", crate::look::ShaderFeatures::NONE), ("all", crate::look::ShaderFeatures::ALL)] {
            crate::look::with_shader_features(f, || {
                let variants = [
                    ("city", crate::look::city_shader_source()),
                    ("far layer", crate::look::city_shader_source_far()),
                    ("props", crate::look::city_shader_source_variant_probes(crate::ambient::ShaderLevels::Plane, crate::look::ShadowLevels::Near, false)),
                ];
                for (label, src) in variants {
                    assert!(src.contains(HOOK_MARKER) && !src.contains("rt_gi_fetch") && !src.contains("lx_rt_gi"), "{name} {label}: no trace of the option");
                    let old = src.replacen(&format!("{HOOK_MARKER}\n"), "", 1);
                    assert_ne!(old, src);
                    assert_eq!(msl(label, &src), msl(label, &old), "{name} {label}: the Metal shader is the old one");
                }
            });
        }
        // with the option, the read and its bindings are in.
        crate::look::with_shader_features(crate::look::ShaderFeatures { rt_gi: true, ..crate::look::ShaderFeatures::NONE }, || {
            let src = crate::look::city_shader_source();
            assert!(src.contains("rt_gi_fetch(world)") && src.contains("@group(1) @binding(20)"));
            msl("city with the option", &src);
        });
    }

    #[test]
    fn the_config_reads_its_defaults() {
        let c = RtGiConfig::default();
        assert!(c.fade_start_m < c.fade_end_m && c.fade_end_m <= c.scene_radius_m, "the traced light fades out inside the traced set");
        assert!(c.drop_radius_m >= c.scene_radius_m);
        assert!(c.reach_m <= c.scene_radius_m);
    }
}
