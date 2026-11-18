//! The look on the GPU: the JS game's light rig, sky, sun shadows and image
//! pipeline (the numbers and curves come from `eo-look`).
//!
//! Per frame, in order (all recorded by `Renderer::encode_with_ids`):
//!
//! 1. **Shadow cull** (compute, after the camera's LOD + cull pass, which
//!    decides every LOD group's level): one invocation per object against the
//!    three cascade boxes, appending to one indirect list per cascade.
//! 2. **Shadow depth**, one depth-only pass per cascade layer of a
//!    `Depth32Float` 2D array, each ONE `multi_draw_indexed_indirect_count`
//!    over its list: the same GPU-driven path as the city draw, no per-object
//!    CPU work.
//! 3. The **city pass** renders into a `Rgba16Float` HDR target with
//!    `look_light.wgsl` (bind group 1), then the **sky** (a full-screen
//!    triangle at the far plane, only where nothing was drawn), then the
//!    transparent list.
//! 4. **Post**: one full-screen pass from the HDR target to the output
//!    (shoulder, ACES, sRGB, grade, look finish, dither).
//!
//! The CPU writes three small uniforms a frame (the look, the cull boxes, the
//! cascade matrices); the sky model and light rig are rebuilt only when the
//! time of day changes.

use crate::camera::Camera;
use eo_look::shadow::{self, CASCADES, CascadeCamera};
use eo_look::{LookFrame, tone};
use std::sync::Mutex;

pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// 16-bit is enough along the light (900 m range: 1.4 cm steps against
/// 5-10 cm texels and their bias) and halves the depth bandwidth.
pub const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth16Unorm;
/// Timestamp slots this module writes after the renderer's own four.
pub const Q_SHADOW_CULL: u32 = 4;
pub const Q_SHADOW_DRAW: u32 = 6;
pub const Q_POST: u32 = 8;
/// the occlusion + bounce compute pass.
pub const Q_AO: u32 = 10;
pub const AO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const AO_Z_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
const VIEW_STRIDE: u64 = 256;
/// Counter words: commands per cascade [0..3), triangles per cascade [3..6), pad.
pub const SHADOW_COUNTER_WORDS: usize = 8;

/// Default hour when nobody asks: noon.
pub const DEFAULT_HOUR: f64 = 12.0;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LookUniform {
    key_dir: [f32; 4],
    key: [f32; 4],
    rim_dir: [f32; 4],
    rim: [f32; 4],
    hemi_sky: [f32; 4],
    hemi_ground: [f32; 4],
    ambient: [f32; 4],
    env_class: [f32; 4],
    sh: [[f32; 4]; 9],
    fog: [f32; 4],
    fog_color: [f32; 4],
    cam_pos: [f32; 4],
    cam_fwd: [f32; 4],
    inv_view_proj: [[f32; 4]; 4],
    cascade_vp: [[[f32; 4]; 4]; 3],
    cascade_split: [f32; 4],
    cascade_bias: [[f32; 4]; 3],
    sun: [f32; 4],
    beta_r: [f32; 4],
    beta_m: [f32; 4],
    beta_sum: [f32; 4],
    mean: [f32; 4],
    tw: [f32; 4],
    tw2: [f32; 4],
    horizon: [f32; 4],
    ground: [f32; 4],
    dg0: [f32; 4],
    dg1: [f32; 4],
    dg2: [f32; 4],
    haze_tint: [f32; 4],
    veil_tint: [f32; 4],
    disc: [f32; 4],
    post0: [f32; 4],
    post1: [f32; 4],
    post2: [f32; 4],
    shadow_tint: [f32; 4],
    highlight_tint: [f32; 4],
    // see look_common.wgsl.
    ao_vp: [[f32; 4]; 4],
    ao_cam: [f32; 4],
    ao_fwd: [f32; 4],
    ao_params: [f32; 4],
    field: [f32; 4],
    // the sky model's zenith radiance (computeSkyModel's zenithRadiance),
    // with `horizon` the two colours of the JS `skyDomeRadiance`.
    dome_zenith: [f32; 4],
    // the room drive (rooms.rs `drive`): (dusk, occupancy, shop occupancy, cool share), (office share, pixel angle).
    rooms_env: [f32; 4],
    rooms_work: [f32; 4],
    // the weather's dome (`upperFromRow`): overcast, isotropy, desaturation; tint.
    wx: [f32; 4],
    wx_tint: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ShadowCullParams {
    planes: [[f32; 4]; 18],
    object_count: u32,
    compact: u32,
    mask: u32,
    pad: u32,
}

/// The JS refresh rotation (src/render/cascade-refresh.js, 'staggered'): the
/// near cascade every other frame, the middle and far ones every fourth, so
/// steady state is ONE shadow pass a frame. A camera jump (or a new clock)
/// refits all three at once (the module's pose convergence).
pub const REFRESH_SCHEDULE: [usize; 4] = [0, 1, 0, 2];
/// A camera move larger than this between two updates refreshes every cascade.
pub const CONVERGE_JUMP_M: f32 = 4.0;

fn v4(a: [f64; 3], w: f64) -> [f32; 4] {
    [a[0] as f32, a[1] as f32, a[2] as f32, w as f32]
}

/// Quality settings (`--shadow-map`, `--no-shadows`).
#[derive(Clone, Copy, Debug)]
pub struct LookOptions {
    pub shadow_map: u32,
    pub shadows: bool,
    /// scene-scale occlusion + one bounce (`--no-ao` turns it off).
    pub ao: bool,
    /// the facade pass's switches (`facade::RUN_*`; `--no-facade-age`, `--no-facade-detail`, `--no-atlas2`, `--no-probe`).
    pub facade: u32,
    /// clouds, weather, night lights and the camera finish (sky.rs).
    pub sky: crate::sky::SkyOptions,
    /// the water pass (`--no-water`: the flat sea quad, as before).
    pub water: bool,
    /// the CC0 scanned sets' switches (`cc0::RUN_*`; `--no-cc0` and the per-feature `--no-cc0-*`).
    pub cc0: u32,
}

impl Default for LookOptions {
    fn default() -> Self {
        Self { shadow_map: shadow::MAP_SIZE, shadows: true, ao: true, facade: crate::facade::RUN_AGE | crate::facade::RUN_DETAIL | crate::facade::RUN_ATLAS2 | crate::facade::RUN_PROBE | crate::facade::RUN_ROOMS | crate::facade::RUN_AGENT_HZB | crate::facade::RUN_PROBE_DEFER, water: true, sky: Default::default(), cc0: crate::cc0::RUN_DEFAULT } // + rooms, agent pyramid test, probe deferral // water // sky
    }
}

/// the half-resolution occlusion/bounce result and its depth.
struct AoTarget {
    size: (u32, u32),
    ao_view: wgpu::TextureView,
    z_view: wgpu::TextureView,
    /// The compute bind group for one (depth texture, HDR texture) pair.
    bind: Option<(wgpu::Texture, wgpu::Texture, wgpu::BindGroup)>,
}

struct State {
    hour: f64,
    /// the scene probe's measured transfer (None: the JS fallback yield).
    transfer: Option<f64>,
    frame: Option<LookFrame>,
    frame_index: u32,
    /// The cascades as last rendered (a cascade keeps its box until it is refreshed).
    last_cascades: [shadow::Cascade; CASCADES],
    /// Cascades refreshed by the coming encode (bit c).
    mask: u32,
    /// Position in `REFRESH_SCHEDULE`, and the eye of the last update.
    rotation: usize,
    last_eye: Option<glam::Vec3>,
    /// Refit every cascade on the next update.
    force_all: bool,
    /// What the last update fitted (cascades as they will be after the coming encode, eye).
    pending: Option<([shadow::Cascade; CASCADES], glam::Vec3)>,
    /// the camera of the last update (view-projection, eye, forward, vertical fov radians).
    cur_cam: Option<(glam::Mat4, glam::Vec3, glam::Vec3, f32)>,
    /// the camera the occlusion result now in the texture was made with.
    ao_made: Option<(glam::Mat4, glam::Vec3, glam::Vec3)>,
    /// the weather and night terms of `frame`, and the weather they were built for.
    terms: Option<eo_sky::SkyTerms>,
    weather_key: (i64, i64, i64),
    /// the weather frame before its fill (hour, sky profile, frame): `eo_sky::frame::base_frame`.
    sky_base: Option<(f64, String, LookFrame)>,
}

struct PostTarget {
    size: (u32, u32),
    view: wgpu::TextureView,
    /// reading the HDR scene, the resolved history 0, history 1.
    bind: [wgpu::BindGroup; 3],
}

pub struct LookGpu {
    pub options: LookOptions,
    output_srgb: bool,
    object_count: u32,
    compact: bool,
    look_buf: wgpu::Buffer,
    cull_buf: wgpu::Buffer,
    view_buf: wgpu::Buffer,
    shadow_draws: wgpu::Buffer,
    shadow_counters: wgpu::Buffer,
    shadow_layers: Vec<wgpu::TextureView>,
    main_bgl: wgpu::BindGroupLayout,
    /// one per history parity (the previous resolved frame differs).
    main_bg: Mutex<[wgpu::BindGroup; 2]>,
    pub(crate) shadow_array: wgpu::TextureView, // pub(crate)
    // occlusion + bounce.
    ao_pipeline: wgpu::ComputePipeline,
    ao_bgl: wgpu::BindGroupLayout,
    ao_sampler: wgpu::Sampler,
    ao_dummy: (wgpu::TextureView, wgpu::TextureView),
    ao: Mutex<Option<AoTarget>>,
    cull_pipeline: wgpu::ComputePipeline,
    cull_bg: wgpu::BindGroup,
    depth_pipeline: wgpu::RenderPipeline,
    depth_bg: wgpu::BindGroup,
    sky_pipeline: wgpu::RenderPipeline,
    sky_bg: wgpu::BindGroup,
    post_pipeline: wgpu::RenderPipeline,
    post_bgl: wgpu::BindGroupLayout,
    post: Mutex<Option<PostTarget>>,
    state: Mutex<State>,
    pub(crate) cmp_sampler: wgpu::Sampler, // pub(crate)
    pub(crate) device: wgpu::Device, // pub(crate) (the probe rebinds group 0)
    /// the sky, weather, night lights and camera finish passes.
    pub sky: crate::sky::SkyGpu,
    sky_depth_ok: std::sync::atomic::AtomicBool, // sky
    /// what post reads: 0 the HDR scene, 1 + p the resolved history p.
    post_source: std::sync::atomic::AtomicU32,
    /// Bytes this module allocated on the GPU (shadow map, lists, uniforms; the
    /// HDR target is counted when it exists).
    pub bytes: u64,
}

/// WGSL of the city shader: draw.wgsl + the look's shared code and lighting.
pub fn city_shader_source() -> String {
    format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}",
        include_str!("draw.wgsl"),
        street_constants_wgsl(),
        look_common_source(),
        include_str!("look_light.wgsl"),
        include_str!("surfaces_ao_use.wgsl"),
        include_str!("facade_age.wgsl"),             // facade
        crate::facade::facade_constants_wgsl()        // facade
    ) + "\n" + &crate::rooms::wgsl_constants() + include_str!("rooms.wgsl")
        + "\n" + include_str!("sky_light_core.wgsl") + "\n" + include_str!("sky_light.wgsl") // sky
        + &crate::landmarks::city_wgsl() // landmarks
        + "\n" + include_str!("cc0.wgsl") + &crate::cc0::wgsl_constants(crate::cc0::scoring_pitch_m()) // cc0
}

/// look_common.wgsl with (surfaces) the street field's constants (GF_*), from
/// eo-look's `GroundField` (data/ground-field.json, the JS DEFAULT_GROUND_FIELD).
pub fn look_common_source() -> String {
    let g = eo_look::ground_field::GroundField::get();
    let f = |v: f64| format!("{v:.8}");
    let v3 = |v: [f64; 3]| format!("vec3<f32>({}, {}, {})", f(v[0]), f(v[1]), f(v[2]));
    let consts = [
        ("GF_S_KERB", "f32", f(g.s_kerb)),
        ("GF_S_FAR_KERB", "f32", f(g.s_far_kerb)),
        ("GF_S_FAR_WALK", "f32", f(g.s_far_walk)),
        ("GF_AXIS", "vec2<f32>", format!("vec2<f32>({}, {})", f(g.axis[0]), f(g.axis[1]))),
        ("GF_CROSS_SHARE", "f32", f(g.cross_share)),
        ("GF_ROAD_RATIO", "f32", f(g.road_ratio)),
        ("GF_WALL_RATIO", "f32", f(g.wall_ratio)),
        ("GF_ROAD_TINT", "vec3<f32>", v3(g.road_tint)),
        ("GF_WALL_TINT", "vec3<f32>", v3(g.wall_tint)),
        ("GF_SPEC_MAX", "f32", f(g.spec_max)),
        ("GF_WALLED_SHARE", "f32", f(g.walled_share)),
        ("GF_CLUTTER", "f32", f(g.clutter)),
        ("GF_CLUTTER_SIN", "f32", f(g.clutter_sin)),
        ("GF_SHARPNESS", "f32", f(g.sharpness)),
        ("GF_POOL_COLOR", "vec3<f32>", v3(g.pool_color)),
        ("GF_SPILL_COLOR", "vec3<f32>", v3(g.spill_color)),
    ];
    let mut s = String::from(include_str!("look_common.wgsl"));
    s.push_str("\n// generated by look.rs look_common_source (eo-look GroundField)\n");
    for (n, t, v) in consts {
        s.push_str(&format!("const {n}: {t} = {v};\n"));
    }
    s.push_str(include_str!("sky_common.wgsl")); // sky
    s.push_str(include_str!("sky_atmo.wgsl")); // the atmosphere tables' sampling helpers
    s.push_str(&crate::sky::sea_wgsl()); // sky
    s
}

/// the mean linear albedo (luminance) the walk concrete is drawn at:
/// weathered Portland-cement concrete, the measured 0.20-0.30 band of the
/// Lawrence Berkeley National Laboratory Heat Island Group's pavement albedo
/// survey (new concrete 0.35-0.45, falling to 0.20-0.30 as it weathers and
/// soils), at the band's top for a swept city footway. The JS draws its
/// footway at the sidewalk tint x the concrete tile's 0.456, a mean of 0.389:
/// new concrete, and the sunlit walk delivered near white (sRGB 219, 202, 185
/// at 11 h) with its scored joints in the tone curve's shoulder. A documented
/// departure from the JS (owner direction 2026-11-14: physical reference
/// values where the JS is the weaker reference).
pub const CONCRETE_AGED_ALBEDO: f64 = 0.30;

/// the fold that takes the mean JS footway (sidewalk tint x 0.456) to
/// `CONCRETE_AGED_ALBEDO`; applied to every walk-concrete class (footway,
/// kerb top, ramp, kerb face) in the street shader.
pub fn concrete_aged_fold() -> f64 {
    let m = eo_street::material::StreetMaterial::get();
    let t = m.tint("sidewalk").map(|v| eo_look::math::srgb_to_linear(v as f64 / 255.0) * m.concrete_base_mean_linear);
    let luma = 0.2126 * t[0] + 0.7152 * t[1] + 0.0722 * t[2];
    (CONCRETE_AGED_ALBEDO / luma).min(1.0)
}

/// the JS street material numbers (eo-street/data/street-material.json)
/// as WGSL constants for `street_surface` in draw.wgsl.
pub fn street_constants_wgsl() -> String {
    let m = eo_street::material::StreetMaterial::get();
    let (a, c) = (&m.pbr_sets.carriageway, &m.pbr_sets.concrete);
    let g = a.aggregate.as_ref().expect("carriageway aggregate tap");
    let sh = &m.shading;
    let f = |v: f64| format!("{:.6}", v);
    let v2 = |v: [f64; 2]| format!("vec2<f32>({}, {})", f(v[0]), f(v[1]));
    let v3 = |v: [f64; 3]| format!("vec3<f32>({}, {}, {})", f(v[0]), f(v[1]), f(v[2]));
    let rough = |k: &str| m.roughness.get(k).copied().unwrap_or(0.9);
    let base = a.base_mean_linear.unwrap_or([1.0; 3]);
    let mut s = String::from("// generated by look.rs street_constants_wgsl from eo-street/data/street-material.json\n");
    let mut c_ = |name: &str, ty: &str, val: String| s.push_str(&format!("const {name}: {ty} = {val};\n"));
    for (p, set) in [("ASPHALT", a), ("CONCRETE", c)] {
        let tile = if p == "CONCRETE" { sh.concrete_scan_period_m } else { set.tile_metres };
        c_(&format!("ST_{p}_TILE"), "f32", f(tile));
        c_(&format!("ST_{p}_MEAN"), "vec3<f32>", v3(set.mean_linear));
        c_(&format!("ST_{p}_CONTRAST"), "f32", f(set.albedo_contrast));
        c_(&format!("ST_{p}_FAR_CONTRAST"), "f32", f(set.far_contrast));
        c_(&format!("ST_{p}_NEAR"), "f32", f(set.detail_near_m));
        c_(&format!("ST_{p}_FAR"), "f32", f(set.detail_far_m));
        c_(&format!("ST_{p}_ALBEDO_CLAMP"), "vec2<f32>", v2(set.albedo_clamp));
        c_(&format!("ST_{p}_ROUGH_GAIN"), "f32", f(set.roughness_gain));
        c_(&format!("ST_{p}_ROUGH_CLAMP"), "vec2<f32>", v2(set.roughness_clamp));
        c_(&format!("ST_{p}_MEAN_ROUGH"), "f32", f(set.mean_roughness));
        c_(&format!("ST_{p}_MEAN_AO"), "f32", f(set.mean_ao));
        c_(&format!("ST_{p}_AO_STRENGTH"), "f32", f(set.ao_strength));
        c_(&format!("ST_{p}_NORMAL"), "f32", f(set.normal_strength));
    }
    c_("ST_ASPHALT_ROUGHNESS", "f32", f(rough("carriageway")));
    c_("ST_CONCRETE_ROUGHNESS", "f32", f(rough("concrete")));
    c_("ST_PAINT_ROUGHNESS", "f32", f(rough("markings")));
    // The asphalt's base fold is a neutral triple in the JS ([0.44, 0.44, 0.44]).
    c_("ST_ASPHALT_BASE", "vec3<f32>", v3(base));
    // x the aged-concrete fold (see `concrete_aged_fold`).
    c_("ST_CONCRETE_BASE", "f32", f(m.concrete_base_mean_linear * concrete_aged_fold()));
    c_("ST_AGG_TILE", "f32", f(g.tile_metres));
    c_("ST_AGG_CONTRAST", "f32", f(g.albedo_contrast));
    c_("ST_AGG_FAR_CONTRAST", "f32", f(g.far_contrast));
    c_("ST_AGG_NEAR", "f32", f(g.near_m));
    c_("ST_AGG_FAR", "f32", f(g.far_m));
    c_("ST_AGG_ALBEDO_CLAMP", "vec2<f32>", v2(g.albedo_clamp));
    c_("ST_AGG_ROUGH_GAIN", "f32", f(g.roughness_gain));
    c_("ST_AGG_ROUGH_CLAMP", "vec2<f32>", v2(g.roughness_clamp));
    c_("ST_AGG_NORMAL", "f32", f(g.normal_strength));
    c_("ST_AGG_RELIEF_NEAR", "f32", f(g.relief_near_m));
    c_("ST_AGG_RELIEF_FAR", "f32", f(g.relief_far_m));
    let pt = a.patch.as_ref().expect("carriageway patch");
    c_("ST_PATCH_FREQ", "f32", f(pt.frequency));
    c_("ST_PATCH_PHASE", "f32", f(pt.phase));
    c_("ST_PATCH_START", "f32", f(pt.start));
    c_("ST_PATCH_FULL", "f32", f(pt.full));
    c_("ST_PATCH_SCALE", "f32", f(pt.scale));
    c_("ST_PATCH_OFFSET", "vec2<f32>", v2(pt.offset));
    c_("ST_PATCH_TONE", "f32", f(pt.tone));
    c_("ST_PATCH_ROUGH", "f32", f(pt.roughness));
    c_("ST_MACRO_COARSE", "f32", f(sh.macro_coarse));
    c_("ST_MACRO_FINE", "f32", f(sh.macro_fine));
    c_("ST_MACRO_AMPLITUDE", "f32", f(sh.macro_amplitude));
    c_("ST_CROSS_FALL_DARKEN", "f32", f(sh.cross_fall_darken));
    c_("ST_CROSS_FALL_REACH", "f32", f(sh.cross_fall_reach_m));
    c_("ST_SILT_REACH", "f32", f(sh.gutter_silt_reach_m));
    c_("ST_SILT_DARKEN", "f32", f(sh.gutter_silt_darken));
    c_("ST_GUTTER_POUR_SILT", "f32", f(sh.gutter_pan_pour_silt));
    c_("ST_GUTTER_WIDTH", "f32", f(m.gutter_width));
    // A rendering cut-off, not a JS number: 400 m is past every range fade
    // (`detailFarM` 110, aggregate `farM` 85), where a 3 m scan tile is
    // smaller than a pixel and its mip chain has averaged it to the mean.
    c_("ST_SCAN_CUTOFF_M", "f32", f(400.0));
    c_("ST_ENV_PAINTED_METAL", "f32", f(m.env_base.get("painted-metal").copied().unwrap_or(0.9)));
    s
}

pub struct LookInit<'a> {
    pub object_buf: &'a wgpu::Buffer,
    pub group_buf: &'a wgpu::Buffer,
    pub object_count: u32,
    pub compact: bool,
    pub output_format: wgpu::TextureFormat,
    pub sample_count: u32,
    pub debug_ids: bool,
    /// Bytes per vertex of the shared vertex buffer (position f32x3 first).
    pub vertex_stride: u64,
}

/// a half-resolution occlusion target (storage-written, sampled).
fn ao_texture(device: &wgpu::Device, format: wgpu::TextureFormat, size: (u32, u32)) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("ao"),
            size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

fn shadow_texture(device: &wgpu::Device, size: u32) -> (Vec<wgpu::TextureView>, wgpu::TextureView, u64) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sun-shadow-cascades"),
        size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: CASCADES as u32 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: SHADOW_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let layers = (0..CASCADES as u32)
        .map(|l| {
            tex.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: l,
                array_layer_count: Some(1),
                ..Default::default()
            })
        })
        .collect();
    let array = tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
    (layers, array, size as u64 * size as u64 * 2 * CASCADES as u64)
}

impl LookGpu {
    pub fn new(device: &wgpu::Device, init: LookInit) -> Self {
        // a bare renderer (tests) starts with the sky slice's stages off;
        // the app turns them on through `set_options` (LookOptions::default()).
        let options = LookOptions { sky: crate::sky::SkyOptions::off(), ..LookOptions::default() };
        let buffer = |label: &str, size: u64, usage: wgpu::BufferUsages| {
            device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size.max(16).next_multiple_of(16), usage, mapped_at_creation: false })
        };
        let uni = wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST;
        let look_buf = buffer("look", std::mem::size_of::<LookUniform>() as u64, uni);
        let cull_buf = buffer("shadow-cull", std::mem::size_of::<ShadowCullParams>() as u64, uni);
        let view_buf = buffer("shadow-views", VIEW_STRIDE * CASCADES as u64, uni);
        let n = init.object_count.max(1) as u64;
        let draws_bytes = n * CASCADES as u64 * crate::DRAW_ARGS_BYTES;
        let shadow_draws = buffer("shadow-draws", draws_bytes, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT);
        let shadow_counters = buffer(
            "shadow-counters",
            (SHADOW_COUNTER_WORDS * 4) as u64,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
        );
        let (shadow_layers, shadow_array, shadow_bytes) = shadow_texture(device, options.shadow_map);
        let cmp_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow-compare"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });

        let entry = |binding, visibility, ty| wgpu::BindGroupLayoutEntry { binding, visibility, ty, count: None };
        let ubuf = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None };
        let sbuf = |read_only| wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None };
        let fs = wgpu::ShaderStages::FRAGMENT;
        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let cs = wgpu::ShaderStages::COMPUTE;

        // Group 1 of the city pipelines.
        let main_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("look-bgl"),
            entries: &[
                entry(0, vf, ubuf),
                entry(1, fs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false }),
                entry(2, fs, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison)),
                // occlusion + bounce result, its depth, a clamped linear sampler.
                entry(3, fs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                entry(4, fs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                entry(5, fs, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                // the sky uniform (the agents' velocity vertex shader reads it too), the
                // cloud sheet, the frame's lights and clusters, the previous resolved frame, a repeat sampler.
                entry(6, vf, ubuf),
                entry(7, fs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                entry(8, fs, sbuf(true)),
                entry(9, fs, sbuf(true)),
                entry(10, fs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                entry(11, fs, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
            ],
        });
        let ao_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ao-linear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let ao_dummy = (ao_texture(device, AO_FORMAT, (1, 1)), ao_texture(device, AO_Z_FORMAT, (1, 1)));
        // the sky passes and their resources.
        let sky = crate::sky::SkyGpu::new(
            device,
            crate::sky::SkyInit { look_buf: &look_buf, hdr_format: HDR_FORMAT, depth_format: crate::DEPTH_FORMAT, shadow_array: &shadow_array, cmp_sampler: &cmp_sampler },
        );
        let main_bg = [0, 1].map(|_| Self::make_main_bg(device, &main_bgl, &look_buf, &shadow_array, &cmp_sampler, &ao_dummy.0, &ao_dummy.1, &ao_sampler, &sky, &sky.dummy_hist));
        // the occlusion + bounce compute.
        let ao_src = format!("{}\n{}", look_common_source(), include_str!("surfaces_ao.wgsl"));
        let ao_module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("surfaces_ao.wgsl"), source: wgpu::ShaderSource::Wgsl(ao_src.into()) });
        let ao_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ao-bgl"),
            entries: &[
                entry(0, cs, ubuf),
                entry(1, cs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                entry(2, cs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                entry(3, cs, wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: AO_FORMAT, view_dimension: wgpu::TextureViewDimension::D2 }),
                entry(4, cs, wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: AO_Z_FORMAT, view_dimension: wgpu::TextureViewDimension::D2 }),
            ],
        });
        let ao_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("ao-layout"), bind_group_layouts: &[Some(&ao_bgl)], immediate_size: 0 });
        let ao_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ao_main"),
            layout: Some(&ao_layout),
            module: &ao_module,
            entry_point: Some("ao_main"),
            compilation_options: Default::default(),
            cache: None,
        });

        // Shadow cull.
        let cull_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("look_shadow_cull.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("look_shadow_cull.wgsl").into()),
        });
        let cull_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow-cull-bgl"),
            entries: &[entry(0, cs, ubuf), entry(1, cs, sbuf(true)), entry(2, cs, sbuf(true)), entry(3, cs, sbuf(false)), entry(4, cs, sbuf(false))],
        });
        let cull_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow-cull-bg"),
            layout: &cull_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: cull_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: init.object_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: init.group_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: shadow_draws.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: shadow_counters.as_entire_binding() },
            ],
        });
        let cull_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("shadow-cull-layout"), bind_group_layouts: &[Some(&cull_bgl)], immediate_size: 0 });
        let cull_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("shadow_cull"),
            layout: Some(&cull_layout),
            module: &cull_module,
            entry_point: Some("shadow_cull"),
            compilation_options: Default::default(),
            cache: None,
        });

        // Shadow depth.
        let depth_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("look_shadow_draw.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("look_shadow_draw.wgsl").into()),
        });
        let depth_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow-depth-bgl"),
            entries: &[
                entry(0, wgpu::ShaderStages::VERTEX, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: None }),
                entry(1, wgpu::ShaderStages::VERTEX, sbuf(true)),
            ],
        });
        let depth_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow-depth-bg"),
            layout: &depth_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &view_buf, offset: 0, size: Some(std::num::NonZeroU64::new(80).unwrap()) }),
                },
                wgpu::BindGroupEntry { binding: 1, resource: init.object_buf.as_entire_binding() },
            ],
        });
        let depth_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("shadow-depth-layout"), bind_group_layouts: &[Some(&depth_bgl)], immediate_size: 0 });
        let pos_only = [wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 }];
        let depth_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow-depth"),
            layout: Some(&depth_layout),
            vertex: wgpu::VertexState {
                module: &depth_module,
                entry_point: Some("shadow_vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout { array_stride: init.vertex_stride, step_mode: wgpu::VertexStepMode::Vertex, attributes: &pos_only })],
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: SHADOW_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });

        // Sky (inside the city pass: same targets and sample count).
        let sky_src = format!("{}\n{}", look_common_source(), include_str!("look_sky.wgsl"));
        let sky_module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("look_sky.wgsl"), source: wgpu::ShaderSource::Wgsl(sky_src.into()) });
        // + the sky uniform (moon, stars).
        let look_only_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("look-only-bgl"), entries: &[entry(0, vf, ubuf), entry(1, vf, ubuf)] });
        let sky_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky-bg"),
            layout: &look_only_bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: look_buf.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: sky.uniform.as_entire_binding() }],
        });
        let sky_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("sky-layout"), bind_group_layouts: &[Some(&look_only_bgl)], immediate_size: 0 });
        let hdr_target = wgpu::ColorTargetState { format: HDR_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL };
        let ids_target = wgpu::ColorTargetState { format: crate::IDS_FORMAT, blend: None, write_mask: wgpu::ColorWrites::empty() };
        let sky_targets_ids = [Some(hdr_target.clone()), Some(ids_target)];
        let sky_targets = [Some(hdr_target)];
        let sky_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky"),
            layout: Some(&sky_layout),
            vertex: wgpu::VertexState { module: &sky_module, entry_point: Some("sky_vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: crate::DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState { count: init.sample_count, ..Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &sky_module,
                entry_point: Some(if init.debug_ids { "sky_fs_ids" } else { "sky_fs" }),
                compilation_options: Default::default(),
                targets: if init.debug_ids { &sky_targets_ids } else { &sky_targets },
            }),
            multiview_mask: None,
            cache: None,
        });

        // Post.
        let post_src = format!("{}\n{}", look_common_source(), include_str!("look_post.wgsl"));
        let post_module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("look_post.wgsl"), source: wgpu::ShaderSource::Wgsl(post_src.into()) });
        let post_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post-bgl"),
            entries: &[
                entry(0, vf, ubuf),
                entry(1, fs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                // the sky uniform, the bloom, its sampler.
                entry(2, fs, ubuf),
                entry(3, fs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                entry(4, fs, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
            ],
        });
        let post_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("post-layout"), bind_group_layouts: &[Some(&post_bgl)], immediate_size: 0 });
        let post_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("post"),
            layout: Some(&post_layout),
            vertex: wgpu::VertexState { module: &post_module, entry_point: Some("post_vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &post_module,
                entry_point: Some("post_fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: init.output_format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let bytes = shadow_bytes + draws_bytes + (SHADOW_COUNTER_WORDS * 4) as u64 + look_buf.size() + cull_buf.size() + view_buf.size() + sky.bytes; // sky
        Self {
            sky, // sky
            sky_depth_ok: std::sync::atomic::AtomicBool::new(false), // sky
            post_source: std::sync::atomic::AtomicU32::new(0),       // sky
            options,
            output_srgb: init.output_format.is_srgb(),
            object_count: init.object_count,
            compact: init.compact,
            look_buf,
            cull_buf,
            view_buf,
            shadow_draws,
            shadow_counters,
            shadow_layers,
            main_bgl,
            main_bg: Mutex::new(main_bg),
            shadow_array,
            ao_pipeline,
            ao_bgl,
            ao_sampler,
            ao_dummy,
            ao: Mutex::new(None),
            cull_pipeline,
            cull_bg,
            depth_pipeline,
            depth_bg,
            sky_pipeline,
            sky_bg,
            post_pipeline,
            post_bgl,
            post: Mutex::new(None),
            state: Mutex::new(State {
                hour: DEFAULT_HOUR,
                transfer: None,
                frame: None,
                frame_index: 0,
                last_cascades: Default::default(),
                mask: 0,
                rotation: 0,
                last_eye: None,
                force_all: true,
                pending: None,
                cur_cam: None,
                ao_made: None,
                terms: None,                // sky
                weather_key: (i64::MIN, 0, 0), // sky
                sky_base: None,
            }),
            cmp_sampler,
            device: device.clone(),
            bytes,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn make_main_bg(
        device: &wgpu::Device,
        bgl: &wgpu::BindGroupLayout,
        look: &wgpu::Buffer,
        shadow: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        ao: &wgpu::TextureView,
        ao_z: &wgpu::TextureView,
        ao_sampler: &wgpu::Sampler,
        sky: &crate::sky::SkyGpu,     // sky
        prev: &wgpu::TextureView,     // the previous resolved frame
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("look-bg"),
            layout: bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: look.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(shadow) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(ao) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(ao_z) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(ao_sampler) },
                // sky
                wgpu::BindGroupEntry { binding: 6, resource: sky.uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(&sky.clouds.shadow_view) }, // the cloud-shadow map (was the coverage sheet)
                wgpu::BindGroupEntry { binding: 8, resource: sky.frame_lights.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 9, resource: sky.clusters.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(prev) },
                wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::Sampler(&sky.rep) },
            ],
        })
    }

    fn rebuild_main_bg(&self) {
        let ao = self.ao.lock().unwrap();
        let (a, z) = match ao.as_ref() {
            Some(t) => (&t.ao_view, &t.z_view),
            None => (&self.ao_dummy.0, &self.ao_dummy.1),
        };
        // main_bg[p] is bound while the resolve writes history p: it reads history 1 - p.
        let bg = |p: usize| {
            let prev = self.sky.history_view(1 - p).unwrap_or_else(|| self.sky.dummy_hist.clone());
            Self::make_main_bg(&self.device, &self.main_bgl, &self.look_buf, &self.shadow_array, &self.cmp_sampler, a, z, &self.ao_sampler, &self.sky, &prev)
        };
        *self.main_bg.lock().unwrap() = [bg(0), bg(1)];
    }

    /// occlusion + bounce at half resolution of the output, after the
    /// city pass (needs the depth it drew, stored, and the HDR it lit).
    /// Records an empty pass (for the timestamps) when it cannot run.
    pub fn encode_ao(&self, enc: &mut wgpu::CommandEncoder, depth: &wgpu::TextureView, hdr: &wgpu::TextureView, ts: Option<(&wgpu::QuerySet, u32)>) {
        let tw = ts.map(|(q, b)| wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(b + Q_AO), end_of_pass_write_index: Some(b + Q_AO + 1) });
        let dt = depth.texture();
        let usable = self.options.ao && dt.sample_count() == 1 && dt.usage().contains(wgpu::TextureUsages::TEXTURE_BINDING) && dt.format() == crate::DEPTH_FORMAT;
        if !usable {
            self.state.lock().unwrap().ao_made = None;
            enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("ao (off)"), timestamp_writes: tw });
            return;
        }
        let size = ((dt.width() / 2).max(1), (dt.height() / 2).max(1));
        let mut created = false;
        let mut ao = self.ao.lock().unwrap();
        if ao.as_ref().map(|t| t.size) != Some(size) {
            *ao = Some(AoTarget { size, ao_view: ao_texture(&self.device, AO_FORMAT, size), z_view: ao_texture(&self.device, AO_Z_FORMAT, size), bind: None });
            created = true;
        }
        let t = ao.as_mut().unwrap();
        let ht = hdr.texture();
        if t.bind.as_ref().is_none_or(|(d, h, _)| d != dt || h != ht) {
            crate::hitch::mark(crate::hitch::Mark::BindGroupCreated, 1);
            let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ao-bg"),
                layout: &self.ao_bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: self.look_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(depth) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(hdr) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&t.ao_view) },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&t.z_view) },
                ],
            });
            t.bind = Some((dt.clone(), ht.clone(), bg));
        }
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("ao"), timestamp_writes: tw });
            pass.set_pipeline(&self.ao_pipeline);
            pass.set_bind_group(0, &t.bind.as_ref().unwrap().2, &[]);
            pass.dispatch_workgroups(size.0.div_ceil(8), size.1.div_ceil(8), 1);
        }
        drop(ao);
        if created {
            self.rebuild_main_bg();
        }
        let mut st = self.state.lock().unwrap();
        st.ao_made = st.cur_cam.map(|(vp, eye, fwd, _)| (vp, eye, fwd));
    }

    /// Bytes of the occlusion targets, once they exist.
    pub fn ao_bytes(&self) -> u64 {
        self.ao.lock().unwrap().as_ref().map(|t| t.size.0 as u64 * t.size.1 as u64 * (8 + 4)).unwrap_or(0)
    }

    /// Group 1 of the city pipelines.
    pub fn main_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.main_bgl
    }

    /// Re-allocate the shadow map at another size (quality setting).
    pub fn set_options(&mut self, device: &wgpu::Device, options: LookOptions) {
        if options.shadow_map != self.options.shadow_map {
            let (layers, array, bytes) = shadow_texture(device, options.shadow_map.max(256));
            let old = self.options.shadow_map as u64 * self.options.shadow_map as u64 * 2 * CASCADES as u64;
            self.bytes = self.bytes - old + bytes;
            self.shadow_layers = layers;
            self.shadow_array = array;
            let _ = device;
            self.rebuild_main_bg();
        }
        if !options.ao {
            self.state.lock().unwrap().ao_made = None;
        }
        // the stages and the weather; a new weather rebuilds the frame.
        self.set_sky_options(options.sky);
        self.options = options;
    }

    /// the output size the post targets were made for (1920 x 1080 before the first frame).
    pub fn out_size(&self) -> (u32, u32) {
        self.post.lock().unwrap().as_ref().map(|t| t.size).unwrap_or((1920, 1080))
    }

    /// whether the passes after the city pass need the scene's depth.
    pub fn sky_wants_depth(&self) -> bool {
        let o = self.sky.options();
        o.any_post() || o.ssr
    }

    /// change the sky stages or the weather at run time (A/B, captures).
    pub fn set_sky_options(&self, o: crate::sky::SkyOptions) {
        if self.sky.options() != o {
            self.sky.set_options(o);
            let mut s = self.state.lock().unwrap();
            s.frame = None;
            s.weather_key = (i64::MIN, 0, 0);
            s.force_all = true;
        }
    }

    /// record whether this frame's depth is usable by the sky passes
    /// (single-sample, bindable); the next update writes it into the uniform.
    pub fn set_sky_depth_ok(&self, ok: bool) {
        self.sky_depth_ok.store(ok, std::sync::atomic::Ordering::Relaxed);
    }

    /// the lights' gather and clusters (before the city pass).
    pub fn encode_sky_lights(&self, enc: &mut wgpu::CommandEncoder, agents: Option<&crate::agents::AgentsGpu>, props: Option<&crate::props::PropsGpu>, ts: Option<(&wgpu::QuerySet, u32)>) {
        self.sky.bind_props(props.map(|p| (p.lights_buffer(), p.light_count_buffer())));
        self.sky.bind_agents(agents.map(|a| a.sky_buffers()));
        self.sky.encode_lights(enc, ts);
    }

    /// after the occlusion pass: luminaires and rain, the agents'
    /// velocity, the volume, the resolve and the bloom; post then reads the
    /// resolved frame.
    pub fn encode_sky_after(&self, enc: &mut wgpu::CommandEncoder, depth: &wgpu::TextureView, hdr: &wgpu::TextureView, agents: Option<&crate::agents::AgentsGpu>, ts: Option<(&wgpu::QuerySet, u32)>) {
        let dt = depth.texture();
        let depth_now = dt.sample_count() == 1 && dt.usage().contains(wgpu::TextureUsages::TEXTURE_BINDING) && dt.format() == crate::DEPTH_FORMAT;
        self.set_sky_depth_ok(depth_now);
        let r = self.sky.encode_after(enc, &self.look_buf, &self.shadow_array, &self.cmp_sampler, depth, hdr, agents, self, ts, depth_now);
        self.post_source.store(r.map(|p| p as u32 + 1).unwrap_or(0), std::sync::atomic::Ordering::Relaxed);
    }

    /// The clock (hours, 0..24); the sky model and light rig follow on the next update.
    pub fn set_time_of_day(&self, hour: f64) {
        let mut s = self.state.lock().unwrap();
        if s.hour != hour {
            s.hour = hour;
            s.frame = None;
            s.force_all = true;
        }
    }

    /// the look uniform buffer (the probe's sky fill reads it).
    pub fn look_buffer(&self) -> &wgpu::Buffer {
        &self.look_buf
    }

    /// the rig follows the scene probe's measured transfer (None:
    /// the JS fallback yield). The frame is rebuilt only when it moves by more
    /// than 0.005 (the readback lands once per capture).
    pub fn set_transfer(&self, transfer: Option<f64>) {
        let mut s = self.state.lock().unwrap();
        let changed = match (s.transfer, transfer) {
            (Some(a), Some(b)) => (a - b).abs() > 0.005,
            (None, None) => false,
            _ => true,
        };
        if changed {
            s.transfer = transfer;
            // Re-solve the fill only (no sky integration on the render thread).
            // a weather frame carries its own fill terms: rebuilt instead.
            s.frame = if s.terms.is_some() { None } else { s.frame.map(|f| f.retarget(transfer)) };
        }
    }

    pub fn transfer(&self) -> Option<f64> {
        self.state.lock().unwrap().transfer
    }

    pub fn time_of_day(&self) -> f64 {
        self.state.lock().unwrap().hour
    }

    /// The look frame in force (built on demand). sky: with any sky stage on,
    /// the frame of the weather in force (its sky model, the weather's fill,
    /// the moon key, the night exposure; eo-sky `frame::look_frame`), rebuilt
    /// when the weather moves; with every stage off, the JS clear frame as before.
    pub fn frame(&self) -> LookFrame {
        let opts = self.sky.options(); // sky
        let legacy = opts == crate::sky::SkyOptions::off();
        let w = if legacy { None } else { Some(self.sky.weather()) };
        // the exposure the frame was drawn with (metered, adapted), for the frame's readers.
        let drawn = if legacy { None } else { self.sky.drawn_exposure() };
        let mut s = self.state.lock().unwrap();
        let (h, t) = (s.hour, s.transfer);
        if let Some(w) = w {
            let key = crate::sky::weather_key(&w);
            if s.weather_key != key {
                s.weather_key = key;
                s.frame = None;
            }
            if s.frame.is_none() {
                let _span = crate::hitch::span(crate::hitch::Phase::LookFrame);
                crate::hitch::mark(crate::hitch::Mark::LookFrameRebuilt, 1);
                let c = eo_sky::Conditions { hour: h, weather: w, moon_day: opts.moon_day };
                // the sky is integrated once per hour and weather; a new
                // probe transfer (each capture) re-solves only the fill from it.
                let profile = format!("{:?}", eo_sky::weather::sky_profile(&w.profile));
                if s.sky_base.as_ref().is_none_or(|(bh, bp, _)| *bh != h || *bp != profile) {
                    s.sky_base = Some((h, profile, eo_sky::frame::base_frame(&c, t)));
                }
                let (f, terms) = eo_sky::frame::look_frame_from(&s.sky_base.as_ref().unwrap().2, &c, t);
                s.frame = Some(f);
                s.terms = Some(terms);
            }
            let mut fr = s.frame.unwrap();
            if let Some(e) = drawn {
                fr.exposure = e; // sky
            }
            return fr;
        }
        s.terms = None;
        *s.frame.get_or_insert_with(|| {
            let _span = crate::hitch::span(crate::hitch::Phase::LookFrame);
            crate::hitch::mark(crate::hitch::Mark::LookFrameRebuilt, 1);
            LookFrame::with_transfer(h, t)
        })
    }

    /// the weather and night terms of the frame in force (None with every sky stage off).
    pub fn sky_terms(&self) -> Option<eo_sky::SkyTerms> {
        let _ = self.frame();
        self.state.lock().unwrap().terms
    }

    /// The shadow counter buffer (see `SHADOW_COUNTER_WORDS`).
    pub fn shadow_counter_buffer(&self) -> &wgpu::Buffer {
        &self.shadow_counters
    }

    /// Write this frame's uniforms (the look, the cascade boxes and matrices).
    pub fn update(&self, queue: &wgpu::Queue, cam: &Camera, copy_step: f32) {
        let f = self.frame();
        let fwd = cam.forward();
        let right = cam.right();
        let up = right.cross(fwd);
        let to = |v: glam::Vec3| [v.x as f64, v.y as f64, v.z as f64];
        let cc = CascadeCamera {
            eye: to(cam.eye),
            forward: to(fwd),
            right: to(right),
            up: to(up),
            fov_y_deg: cam.fov_y_deg as f64,
            aspect: cam.aspect as f64,
            near: cam.near as f64,
            far: cam.far as f64,
        };
        let fitted = shadow::fit_cascades(&cc, f.key_dir, self.options.shadow_map);
        let shadows_on = self.options.shadows && f.shadow_strength > 0.0;
        // Which cascades the coming encode refreshes; the others keep the box
        // their depth map was rendered with (committed in `encode_shadows`).
        let mut st = self.state.lock().unwrap();
        st.frame_index = st.frame_index.wrapping_add(1);
        let jumped = st.last_eye.is_none_or(|e| e.distance(cam.eye) > CONVERGE_JUMP_M);
        let mask = if st.force_all || jumped {
            (1u32 << CASCADES) - 1
        } else {
            1u32 << REFRESH_SCHEDULE[st.rotation % REFRESH_SCHEDULE.len()]
        };
        let mut cascades = st.last_cascades;
        for (c, fit) in fitted.iter().enumerate() {
            if mask & (1 << c) != 0 {
                cascades[c] = *fit;
            }
        }
        st.mask = mask;
        st.pending = Some((cascades, cam.eye));
        let frame_index = st.frame_index;
        // the occlusion result in the texture was made with `ao_made`;
        // this frame's camera is what the coming AO pass will run with.
        let ao_made = if self.options.ao { st.ao_made } else { None };
        st.cur_cam = Some((cam.view_proj(), cam.eye, fwd, cam.fov_y_deg.to_radians()));
        let terms = st.terms; // sky
        drop(st);
        // the sky passes' uniform (unjittered camera) and the exposure, adapted.
        let out_size = self.post.lock().unwrap().as_ref().map(|t| t.size).unwrap_or((1920, 1080));
        let exposure = match terms {
            Some(t) => {
                let _span = crate::hitch::span(crate::hitch::Phase::SkyUpdate);
                self.sky.update(queue, &Camera { jitter: [0.0; 2], ..*cam }, &f, &t, out_size, self.sky_depth_ok.load(std::sync::atomic::Ordering::Relaxed))
            }
            None => f.exposure,
        };
        let out_h = self.post.lock().unwrap().as_ref().map(|t| t.size.1).unwrap_or(1080) as f32;
        let focal_px = 0.5 * out_h / (0.5 * cam.fov_y_deg.to_radians()).tan();
        let m = &f.model;
        let fld = &m.field;
        let fog = f.fog_color(fwd.x as f64, fwd.z as f64);
        // (a rolled photo camera's up; world up otherwise, as before)
        let rot = glam::camera::rh::view::look_to_mat4(glam::Vec3::ZERO, fwd, if cam.roll == 0.0 { glam::Vec3::Y } else { cam.up() });
        let inv = (cam.proj() * rot).inverse();
        let g = m.grade;
        let look = eo_look::look::LOOK;
        let mut sh = [[0f32; 4]; 9];
        for (k, c) in f.env_sh.iter().enumerate() {
            sh[k] = v4(*c, 0.0);
        }
        let split_far = [fitted[0].split_far, fitted[1].split_far, fitted[2].split_far];
        let fade_start = split_far[2] * 0.75;
        // over the marine layer (fog weather, the eye 50-250 m above its
        // top: Twin Peaks, the towers, the air) the sky is the clear sky the fog
        // lies under: the dome's weather terms fade out and its gain makes the
        // zenith the clear hour's (eo-sky `above_fog_gain`); below, the fog's grey dome.
        let above = match terms {
            Some(t) if t.weather.kind == eo_sky::Kind::Fog => {
                let top = t.decks.low_top_m.max(t.decks.marine_top_m + t.decks.marine_swing_m);
                eo_look::math::smoothstep(top + 50.0, top + 250.0, cam.eye.y as f64)
            }
            _ => 0.0,
        };
        let wx_keep = 1.0 - above;
        let above_gain = 1.0 + (terms.map(|t| t.above_fog_gain).unwrap_or(1.0) - 1.0) * above;
        // clear air's aerial perspective is the volume's; a weather that veils (fog,
        // drizzle, rain, storm: visibility under clear air's) keeps the range fog in every
        // pass (the far land and other late passes take it too).
        // every weather: the veiling weathers' fog is the volume's too (sky.rs `atmo3.x`).
        let atmo_ap = self.sky.options().atmosphere && terms.is_some();
        let u = LookUniform {
            key_dir: v4(f.key_dir, if shadows_on { f.shadow_strength } else { 0.0 }),
            key: v4(f.key, exposure), // adapted (f.exposure with every sky stage off)
            rim_dir: v4(f.rim_dir, 0.0),
            rim: v4(f.rim, 0.0),
            hemi_sky: v4(f.hemi_sky, 0.0),
            hemi_ground: v4(f.hemi_ground, 0.0),
            ambient: v4(f.ambient, f.rig.environment_intensity),
            env_class: [f.rig.env_painted as f32, f.rig.env_glass as f32, f.rig.env_asphalt as f32, f.rig.env_ground as f32],
            sh,
            // with the atmosphere tables the volume pass carries the aerial
            // perspective (sky_volume `vol_aerial`) for the city, the far land and
            // the sea: the range fog every pass applies is pushed out of reach.
            fog: if atmo_ap { [1.0e7, 2.0e7, 0.0, 0.0] } else { [f.aerial.near as f32, f.aerial.far as f32, 0.0, 0.0] },
            fog_color: v4(fog, 0.0),
            cam_pos: [cam.eye.x, cam.eye.y, cam.eye.z, 1.0],
            cam_fwd: [fwd.x, fwd.y, fwd.z, 0.0],
            inv_view_proj: inv.to_cols_array_2d(),
            cascade_vp: [cascades[0].view_proj, cascades[1].view_proj, cascades[2].view_proj],
            cascade_split: [split_far[0] as f32, split_far[1] as f32, split_far[2] as f32, fade_start as f32],
            cascade_bias: cascades.map(|c| [c.normal_bias as f32, (-c.depth_bias_m / c.depth_range) as f32, c.texel_world as f32, 0.0]),
            sun: v4(m.sun.dir(), fld.state.sun_e),
            beta_r: v4(fld.state.beta_r, fld.state.horizon_up),
            beta_m: v4(fld.state.beta_m, fld.state.g),
            beta_sum: v4(fld.state.beta_sum, fld.grade),
            mean: v4(fld.mean, fld.twilight),
            tw: [fld.night as f32, fld.ozone as f32, fld.belt as f32, fld.shadow_top_sin as f32],
            tw2: [fld.deep_twilight as f32, fld.airglow as f32, m.profile.urban_glow as f32, m.profile.overcast as f32],
            horizon: v4(m.horizon_radiance, m.profile.turbidity),
            // the street field's unlit ground x `NIGHT_GROUND_FILL` by nightness, as the
            // surfaces' night fill (eo-sky frame.rs); its lamp pools and shop spill (`field`) keep theirs.
            ground: v4(m.ground_radiance.map(|v| v * (1.0 + (eo_sky::frame::NIGHT_GROUND_FILL - 1.0) * eo_sky::night::nightness(m.sun.altitude_deg))), if g.is_some() { 1.0 } else { 0.0 }),
            dg0: g.map(|g| [g.weight, g.veil_luminance, g.zenith_desaturation, g.horizon_desaturation].map(|x| x as f32)).unwrap_or_default(),
            dg1: g.map(|g| [g.horizon_lift, g.dome_gain * above_gain, g.skyline_share, g.low_sun_share].map(|x| x as f32)).unwrap_or([0.0, 1.0, 0.0, 0.0]),
            dg2: [
                g.map(|g| g.sunward_horizon_desaturation).unwrap_or(0.0) as f32,
                look.dome_gradient_power as f32,
                look.skyline_band_width as f32,
                look.skyline_band_peak as f32,
            ],
            haze_tint: v4(look.haze_tint, 0.0),
            veil_tint: v4(look.veil_tint, 0.0),
            disc: v4(f.disc, f.disc_radius.cos()),
            post0: [tone::aces_neutral_inverse(tone::HIGHLIGHT_START_DISPLAY) as f32, tone::HIGHLIGHT_SCALE as f32, look.grade_saturation as f32, look.grade_contrast as f32],
            post1: [look.grade_brightness as f32, look.finish_balance as f32, look.finish_vibrance as f32, look.finish_vignette as f32],
            post2: [look.finish_density as f32, 1.0 / 255.0, if self.output_srgb { 1.0 } else { 0.0 }, (frame_index % 64) as f32],
            shadow_tint: v4(look.finish_shadow_tint, 0.0),
            highlight_tint: v4(look.finish_highlight_tint, 0.0),
            ao_vp: ao_made.map(|m| m.0.to_cols_array_2d()).unwrap_or([[0.0; 4]; 4]),
            ao_cam: ao_made.map(|m| m.1.extend(1.0).to_array()).unwrap_or([0.0; 4]),
            ao_fwd: ao_made.map(|m| m.2.extend(0.0).to_array()).unwrap_or([0.0; 4]),
            ao_params: [
                if ao_made.is_some() { 1.0 } else { 0.0 },
                1.0 / focal_px,
                std::env::var("EO_AO_DEBUG").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0),
                0.0,
            ],
            field: {
                // makeGroundField's practicals ramps on the sun's altitude (dry, model exposure 1).
                let gf = eo_look::ground_field::GroundField::get();
                let alt = m.sun.altitude_deg;
                let ss = |a: f64, b: f64, x: f64| {
                    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
                    t * t * (3.0 - 2.0 * t)
                };
                [(gf.lamp_pool * (1.0 - ss(-3.0, 8.0, alt))) as f32, (gf.shop_spill * (1.0 - ss(2.0, 16.0, alt))) as f32, 0.0, 0.0]
            },
            dome_zenith: v4(m.zenith_radiance, 0.0),
            rooms_env: crate::rooms::drive(m.sun.altitude_deg, self.time_of_day()).0,
            rooms_work: crate::rooms::drive(m.sun.altitude_deg, self.time_of_day()).1,
            wx: [(m.profile.overcast * wx_keep) as f32, (m.profile.isotropy * wx_keep) as f32, (m.profile.desaturation * wx_keep) as f32, if m.ext.on { 1.0 } else { 0.0 }], // eo-look `SkyExt`
            wx_tint: v4(m.profile.tint.map(|v| 1.0 + (v - 1.0) * wx_keep), m.ext.afterglow), // w the afterglow arch
        };
        queue.write_buffer(&self.look_buf, 0, bytemuck::bytes_of(&u));
        let mut planes = [[0f32; 4]; 18];
        for (c, cas) in cascades.iter().enumerate() {
            planes[c * 6..c * 6 + 6].copy_from_slice(&cas.planes);
        }
        let p = ShadowCullParams { planes, object_count: self.object_count, compact: self.compact as u32, mask, pad: 0 };
        queue.write_buffer(&self.cull_buf, 0, bytemuck::bytes_of(&p));
        for (c, cas) in cascades.iter().enumerate() {
            let mut bytes = [0f32; 20];
            for col in 0..4 {
                bytes[col * 4..col * 4 + 4].copy_from_slice(&cas.view_proj[col]);
            }
            bytes[16] = copy_step;
            queue.write_buffer(&self.view_buf, c as u64 * VIEW_STRIDE, bytemuck::cast_slice(&bytes));
        }
    }

    fn shadows_active(&self) -> bool {
        self.options.shadows && self.frame().shadow_strength > 0.0
    }

    /// Shadow cull + one depth pass per cascade (after the camera's cull pass).
    pub fn encode_shadows(&self, enc: &mut wgpu::CommandEncoder, vertex: &wgpu::Buffer, index: &wgpu::Buffer, ts: Option<(&wgpu::QuerySet, u32)>) {
        let active = self.shadows_active();
        // Commit what the last update fitted: these boxes are what the maps
        // will hold after this encode.
        let mask = {
            let mut st = self.state.lock().unwrap();
            let mask = st.mask;
            if let Some((cascades, eye)) = st.pending.take() {
                if active {
                    st.last_cascades = cascades;
                    st.last_eye = Some(eye);
                    if mask == (1u32 << CASCADES) - 1 {
                        st.force_all = false;
                    } else {
                        st.rotation += 1;
                    }
                } else {
                    st.force_all = true;
                }
            }
            if active { mask } else { 0 }
        };
        crate::hitch::mark(crate::hitch::Mark::CascadesRefreshed, mask.count_ones() as u64);
        enc.clear_buffer(&self.shadow_counters, 0, None);
        {
            let tw = ts.map(|(q, b)| wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(b + Q_SHADOW_CULL), end_of_pass_write_index: Some(b + Q_SHADOW_CULL + 1) });
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("shadow-cull"), timestamp_writes: tw });
            if active {
                pass.set_pipeline(&self.cull_pipeline);
                pass.set_bind_group(0, &self.cull_bg, &[]);
                pass.dispatch_workgroups(self.object_count.div_ceil(64), 1, 1);
            }
        }
        let n = self.object_count;
        // Only the refreshed cascades get a pass; the others keep their maps.
        // With nothing to refresh, one empty pass still carries the timestamps.
        let list: Vec<usize> = (0..CASCADES).filter(|c| mask & (1 << c) != 0).collect();
        let passes: Vec<Option<usize>> = if list.is_empty() { vec![None] } else { list.iter().map(|c| Some(*c)).collect() };
        for (k, pc) in passes.iter().enumerate() {
            let tw = ts.and_then(|(q, b)| {
                let begin = (k == 0).then_some(b + Q_SHADOW_DRAW);
                let end = (k == passes.len() - 1).then_some(b + Q_SHADOW_DRAW + 1);
                (begin.is_some() || end.is_some()).then_some(wgpu::RenderPassTimestampWrites { query_set: q, beginning_of_pass_write_index: begin, end_of_pass_write_index: end })
            });
            let Some(c) = *pc else {
                enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("shadow-depth (none this frame)"),
                    timestamp_writes: tw.map(|t| wgpu::ComputePassTimestampWrites {
                        query_set: t.query_set,
                        beginning_of_pass_write_index: t.beginning_of_pass_write_index,
                        end_of_pass_write_index: t.end_of_pass_write_index,
                    }),
                });
                continue;
            };
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow-depth"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_layers[c],
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: tw,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.depth_pipeline);
            pass.set_bind_group(0, &self.depth_bg, &[(c as u64 * VIEW_STRIDE) as u32]);
            pass.set_vertex_buffer(0, vertex.slice(..));
            pass.set_index_buffer(index.slice(..), wgpu::IndexFormat::Uint32);
            let offset = c as u64 * n as u64 * crate::DRAW_ARGS_BYTES;
            if self.compact {
                pass.multi_draw_indexed_indirect_count(&self.shadow_draws, offset, &self.shadow_counters, c as u64 * 4, n);
            } else {
                pass.multi_draw_indexed_indirect(&self.shadow_draws, offset, n);
            }
        }
    }

    pub fn bind_main(&self, pass: &mut wgpu::RenderPass<'_>) {
        let bg = self.main_bg.lock().unwrap()[self.sky.parity()].clone(); // the previous frame for this parity
        pass.set_bind_group(1, &bg, &[]);
    }

    /// The sky, inside the city pass, after the opaque list.
    pub fn draw_sky(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.sky_pipeline);
        pass.set_bind_group(0, &self.sky_bg, &[]);
        pass.draw(0..3, 0..1);
    }

    /// The HDR scene target (single-sample; the MSAA resolve target when MSAA is on) for an output size.
    pub fn hdr_target(&self, w: u32, h: u32) -> wgpu::TextureView {
        let device = &self.device;
        let mut p = self.post.lock().unwrap();
        if p.as_ref().map(|t| t.size) != Some((w, h)) {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("hdr-scene"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: HDR_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = tex.create_view(&Default::default());
            // the resolved histories and the bloom, of the same size.
            self.sky.ensure_targets((w, h), HDR_FORMAT);
            let bloom = self.sky.bloom_view().unwrap_or_else(|| self.sky.dummy_hist.clone());
            let make = |src: &wgpu::TextureView| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("post-bg"),
                    layout: &self.post_bgl,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: self.look_buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(src) },
                        wgpu::BindGroupEntry { binding: 2, resource: self.sky.uniform.as_entire_binding() }, // sky
                        wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&bloom) },
                        wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(self.sky.linear_sampler()) },
                    ],
                })
            };
            let h0 = self.sky.history_view(0).unwrap_or_else(|| self.sky.dummy_hist.clone());
            let h1 = self.sky.history_view(1).unwrap_or_else(|| self.sky.dummy_hist.clone());
            let bind = [make(&view), make(&h0), make(&h1)];
            *p = Some(PostTarget { size: (w, h), view, bind });
            drop(p);
            self.rebuild_main_bg(); // the city pass reads the new histories
            return self.post.lock().unwrap().as_ref().unwrap().view.clone();
        }
        p.as_ref().unwrap().view.clone()
    }

    /// Bytes of the HDR target, once it exists.
    pub fn hdr_bytes(&self) -> u64 {
        self.post.lock().unwrap().as_ref().map(|t| t.size.0 as u64 * t.size.1 as u64 * 8).unwrap_or(0)
    }

    /// photo mode: what post would read this frame (the HDR scene or the resolved history).
    pub(crate) fn post_source_view(&self, hdr: &wgpu::TextureView) -> wgpu::TextureView {
        match self.post_source.load(std::sync::atomic::Ordering::Relaxed) {
            0 => hdr.clone(),
            n => self.sky.history_view((n - 1) as usize).unwrap_or_else(|| hdr.clone()),
        }
    }

    /// photo mode: the bloom post would add (None with the bloom stage off or no targets yet).
    pub(crate) fn bloom_view(&self) -> Option<wgpu::TextureView> {
        if self.sky.options().bloom { self.sky.bloom_view() } else { None }
    }

    /// photo mode: a post bind group reading `src`, with `bloom` in the bloom's place.
    pub(crate) fn post_bind_group(&self, src: &wgpu::TextureView, bloom: &wgpu::TextureView) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post-bg-photo"),
            layout: &self.post_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.look_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(src) },
                wgpu::BindGroupEntry { binding: 2, resource: self.sky.uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(bloom) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(self.sky.linear_sampler()) },
            ],
        })
    }

    /// HDR target -> output.
    pub fn encode_post(&self, enc: &mut wgpu::CommandEncoder, out: &wgpu::TextureView, ts: Option<(&wgpu::QuerySet, u32)>) {
        self.encode_post_with(enc, out, ts, None);
    }

    /// HDR target -> output, or `photo` (photo mode's bind group) -> output.
    pub fn encode_post_with(&self, enc: &mut wgpu::CommandEncoder, out: &wgpu::TextureView, ts: Option<(&wgpu::QuerySet, u32)>, photo: Option<&wgpu::BindGroup>) {
        let p = self.post.lock().unwrap();
        let t = p.as_ref().expect("hdr_target before encode_post");
        let tw = ts.map(|(q, b)| wgpu::RenderPassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(b + Q_POST), end_of_pass_write_index: Some(b + Q_POST + 1) });
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("post"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: out,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: tw,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.post_pipeline);
        let src = self.post_source.load(std::sync::atomic::Ordering::Relaxed) as usize; // sky
        match photo {
            Some(bg) => pass.set_bind_group(0, bg, &[]),
            None => pass.set_bind_group(0, &t.bind[src.min(2)], &[]),
        }
        pass.draw(0..3, 0..1);
    }

    // what the agent renderer (agents.rs) needs to cast into the cascades.
    /// The cascades the last `encode_shadows` refreshed (bit c), 0 when shadows are off.
    pub fn encoded_shadow_mask(&self) -> u32 {
        if self.shadows_active() { self.state.lock().unwrap().mask } else { 0 }
    }
    /// One depth view per cascade layer.
    pub fn shadow_layer_views(&self) -> &[wgpu::TextureView] {
        &self.shadow_layers
    }
    /// The cascade view-projections, 256 bytes a cascade (mat4 + copy step).
    pub fn shadow_view_buffer(&self) -> &wgpu::Buffer {
        &self.view_buf
    }
    // end agents

    /// Cascade far edges (view depth, metres) and texel sizes of the last update.
    pub fn cascade_summary(&self) -> Vec<(f64, f64)> {
        self.state.lock().unwrap().last_cascades.iter().map(|c| (c.split_far, c.texel_world)).collect()
    }
}
