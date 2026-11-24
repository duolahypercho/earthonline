//! The cinematic look's post pass (`--look cinematic`, stage L1;
//! `LookOptions::look`): eo-look `cinematic::grade`'s display transform in
//! place of the JS chain's. Its own pipeline (`look_cine.wgsl`), made the
//! first time the look is on; with it off the post pass (and the photo pass)
//! is the old pipeline, its shader text and bind groups untouched, so the
//! frame is the old one bit for bit.
//!
//! In order, on the resolved frame:
//!
//! 1. under the photo finish (`--photo-finish 1|2`), its lens: lateral colour,
//!    the residual fall-off and the sensor's noise (photo.rs); the photo
//!    finish's own camera response and grade are not used: the look's curve
//!    and grade stand in for them, so the two stack cleanly;
//! 2. the bloom, energy kept: the pixel gives up `CINE_BLOOM_SHARE` of its
//!    bright part (as the pyramid's prefilter takes it) and the pyramid gives
//!    it back spread out (sky.rs builds the pyramid with the look's threshold,
//!    ceiling and weights, whether or not the photo finish is on: one bloom);
//! 3. the tone (`grade::tone`): a real white point, a hue-preserving
//!    shoulder, a deeper toe;
//! 4. sRGB, the hour's grade from a 32^3 table (`grade::key_weights`,
//!    baked from the keys' tables when the weights move by `grade::LUT_STEP`,
//!    not every frame), the display vignette, the dither.
//!
//! No mesopic fade: the night key's grade replaces it. The submerged camera
//! (`--water-optics`) keeps the old display transform. Cost: one full-screen
//! pass in place of the old one, one 3D texture read and a few dozen ALU
//! operations more a pixel; a 256 KiB upload when the grade moves.
//!
//! ## The camera model (`--cine-camera`, camera track)
//!
//! Under the look, `--cine-camera` (`SF_CAMERA`, `SkyOptions::cine_camera`)
//! replaces steps 1-4 with a camera that meters the frame it is shown:
//!
//! 1. (compute, before the pass; cine_camera.wgsl) a histogram of the resolved
//!    frame's log luminance (4 x 4 pixel blocks, weighted toward the lower
//!    centre of the frame) and a bilateral grid of it (cells of `CAM_CELL_PX`
//!    pixels, `CAM_GRID_Z` luminance slices; Chen, Paris and Durand 2007);
//!    the meter's log-average over a percentile window (`cam1`), its
//!    correction of the hour's exposure (clamped, `cam0.z` up, `cam1.w` down,
//!    snapped on a cut, else converging at `CAM_ADAPT_RATE`); the visible
//!    sun's energy over its disc; the grid blurred and written to a 3D texture;
//! 2. the lens: lateral colour (the photo finish's), the bloom threshold-free and
//!    energy kept (the pixel gives up `cam_bloom_part`, the `CAM_BLOOM_LEVELS`
//!    levels give it back with a power-law tail), the optical fall-off
//!    (`CAM_VIGNETTE` of cos^4), the sun's starburst and ghosts scaled by
//!    its measured energy (no lamp starbursts: sky.rs skips the sprite glare);
//! 3. the exposure (the hour's x the meter's) and the local exposure: the
//!    grid's base luminance compressed about middle grey by `cam0.x`
//!    (shadows) and `cam0.y` (highlights), detail kept;
//! 4. a hue-preserving filmic curve (an AgX-style log sigmoid in an inset
//!    gamut, so a bright colour runs to white without per-channel clipping),
//!    sRGB, the hour's grade (`CAM_GRADE_KEYS`), film grain strongest in the
//!    mid-tones and the dither.

use bytemuck::{Pod, Zeroable};
use eo_look::cinematic::grade;
use std::sync::Mutex;

/// The pass's uniform (look_cine.wgsl `Cine`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, Default, PartialEq)]
pub struct CineUniform {
    /// x the bloom's share of the bright part; yzw unused.
    pub bloom: [f32; 4],
    /// x the display vignette, y the table's coordinate scale, z its offset; w unused.
    pub grade: [f32; 4],
    /// (`--cine-camera`) x the local exposure's contrast in the shadows, y in the
    /// highlights (1: none), z the meter's largest lift (EV), w the model on (1/0).
    pub cam0: [f32; 4],
    /// x, y the meter's window (shares of the metered weight), z the exposure
    /// compensation (EV against middle grey), w the meter's largest cut (EV).
    pub cam1: [f32; 4],
    /// x the grain's amount (display, at mid grey), y the sun glare's gain, zw unused.
    pub cam2: [f32; 4],
}

impl CineUniform {
    pub fn new() -> Self {
        let n = grade::LUT_SIZE as f32;
        Self { bloom: [grade::CINE_BLOOM_SHARE as f32, 0.0, 0.0, 0.0], grade: [grade::CINE_VIGNETTE as f32, (n - 1.0) / n, 0.5 / n, 0.0], cam0: [0.0; 4], cam1: [0.0; 4], cam2: [0.0; 4] }
    }

    /// (`--cine-camera`) the uniform with the camera model's terms for the sun's altitude.
    pub fn with_camera(sun_alt_deg: f64) -> Self {
        let mut u = Self::new();
        let p = camera_params(sun_alt_deg);
        u.cam0 = [p.shadow_contrast as f32, p.highlight_contrast as f32, p.max_lift_ev as f32, 1.0];
        u.cam1 = [p.window[0] as f32, p.window[1] as f32, p.compensation_ev as f32, p.max_cut_ev as f32];
        // (`EO_CAM_DEBUG=1`: the meter's state drawn as bars along the frame's top, for tuning.)
        static DEBUG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let debug = *DEBUG.get_or_init(|| std::env::var("EO_CAM_DEBUG").is_ok_and(|v| v == "1"));
        u.cam2 = [CAM_GRAIN as f32, CAM_FLARE_GAIN as f32, if debug { 1.0 } else { 0.0 }, 0.0];
        u
    }
}

// ---------------------------------------------------------------------------
// (`--cine-camera`) the camera model's constants.

/// The histogram's bins over `CAM_HIST_EV` (log2 of the luminance exposed by
/// the hour's exposure: middle grey is log2 0.18 = -2.47).
pub const CAM_HIST_BINS: u32 = 128;
pub const CAM_HIST_EV: [f64; 2] = [-16.0, 10.0];
/// The bilateral grid: a cell is `CAM_CELL_PX` pixels square (8 x 8 threads of
/// 4 x 4 pixel blocks: one workgroup a column), `CAM_GRID_Z` slices over `CAM_GRID_EV`
/// (0.81 EV a slice).
pub const CAM_CELL_PX: u32 = 32;
pub const CAM_GRID_Z: u32 = 32;
pub const CAM_GRID_EV: [f64; 2] = [-16.0, 10.0];
/// The grid's own (edge-aware) base against the 5 x 5 cell blur of the whole
/// column: the blend keeps large shapes smooth without halos at strong edges.
pub const CAM_BILATERAL_SHARE: f64 = 0.65;
/// How fast the meter's correction converges (1/s, exponential), after a cut snaps it.
pub const CAM_ADAPT_RATE: f64 = 3.0;
/// The optical fall-off: this share of cos^4 of the field angle (a 26 deg half
/// field down, 16:9: the corners about 0.45 EV under the centre).
pub const CAM_VIGNETTE: f64 = 0.33;
/// Film grain: the standard deviation in display values at mid grey (falling
/// to 0 at black and white).
pub const CAM_GRAIN: f64 = 0.006;
/// The sun's glare (starburst and ghosts) at full strength, exposed units.
pub const CAM_FLARE_GAIN: f64 = 1.0;
/// The sun's measured energy (mean exposed luminance over its disc) where its
/// glare starts and where it is full: log2.
pub const CAM_FLARE_EV: [f64; 2] = [2.5, 6.0];

/// The bloom: the share of every pixel the lens spreads (veiling glare), the
/// added share of a bright one (the highlights' halation through a light
/// diffusion), from `CAM_BLOOM_MIST_EV[0]` to `[1]` (log2, exposed: +1.5 and
/// +4 EV over middle grey); above `CAM_BLOOM_CAP` (exposed) a source's energy
/// grows only logarithmically (the sun's disc, a glint).
pub const CAM_BLOOM_VEIL: f64 = 0.03;
pub const CAM_BLOOM_MIST: f64 = 0.12;
pub const CAM_BLOOM_MIST_EV: [f64; 2] = [-1.0, 1.5];
pub const CAM_BLOOM_CAP: f64 = 64.0;
/// The levels' weights (half resolution to 1/128), summing to 1: about equal
/// energy an octave of radius, a 1/r^2 tail.
pub const CAM_BLOOM_WEIGHTS: [f64; crate::sky::CAM_BLOOM_LEVELS] = [0.20, 0.17, 0.15, 0.14, 0.13, 0.11, 0.10];

/// The level weights for sky.rs (`--cine-camera`).
pub fn bloom_level_weights() -> [f32; crate::sky::CAM_BLOOM_LEVELS] {
    let s: f64 = CAM_BLOOM_WEIGHTS.iter().sum();
    CAM_BLOOM_WEIGHTS.map(|v| (v / s) as f32)
}

/// The bloom's WGSL constants and its prefilter's switch (sky.rs composes it before sky_bloom.wgsl).
pub fn bloom_wgsl(camera: bool) -> String {
    let f = |v: f64| format!("{:?}", v as f32);
    format!(
        "\n// generated by cine_post.rs bloom_wgsl (`--cine-camera`)\nconst SF_CAMERA: bool = {camera};\nconst CAM_BLOOM_VEIL: f32 = {};\nconst CAM_BLOOM_MIST: f32 = {};\nconst CAM_BLOOM_MIST_EV: vec2<f32> = vec2<f32>({}, {});\nconst CAM_BLOOM_CAP: f32 = {};\n{}",
        f(CAM_BLOOM_VEIL),
        f(CAM_BLOOM_MIST),
        f(CAM_BLOOM_MIST_EV[0]),
        f(CAM_BLOOM_MIST_EV[1]),
        f(CAM_BLOOM_CAP),
        CAM_BLOOM_PART_WGSL,
    )
}

/// The share of a pixel (scene values `c`, the hour's exposure `e`) the lens
/// spreads, in scene values: the bloom's prefilter takes it from every tap and
/// the post pass takes it off the pixel, so the bloom moves light, it adds none.
const CAM_BLOOM_PART_WGSL: &str = "
fn cam_bloom_part(c: vec3<f32>, e: f32) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0)) * e;
    let l = dot(x, vec3<f32>(0.2126, 0.7152, 0.0722));
    if (!(l > 1e-12) || !(e > 0.0)) {
        return vec3<f32>(0.0);
    }
    let lc = select(l, CAM_BLOOM_CAP * (1.0 + log(l / CAM_BLOOM_CAP)), l > CAM_BLOOM_CAP);
    let share = CAM_BLOOM_VEIL + CAM_BLOOM_MIST * smoothstep(CAM_BLOOM_MIST_EV.x, CAM_BLOOM_MIST_EV.y, log2(lc));
    return x * (lc / l) * share / e;
}
";

/// The meter and the local exposure by the hour (`camera_params`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraParams {
    /// The local exposure's contrast about middle grey: below it, above it (1: none).
    pub shadow_contrast: f64,
    pub highlight_contrast: f64,
    /// The meter's window: the shares of the metered weight it averages between.
    pub window: [f64; 2],
    /// Where the window's log-average lands, EV against middle grey.
    pub compensation_ev: f64,
    /// The meter's largest correction of the hour's exposure, up and down (EV).
    pub max_lift_ev: f64,
    pub max_cut_ev: f64,
}

/// By day (and through the golden hour) the meter averages the middle of the
/// frame's tones and may lift a shaded street by 1.5 EV over the hour's
/// exposure, and the local exposure opens the shadows a little; at night it
/// meters the lit part (the upper tones), keeps a darker key and the shadows'
/// depth (a lamp's pool against the unlit gap). Blended over the sun at -9 ..
/// +2 deg (the blue hour, the sun 4-8 deg down, takes about half of each).
pub fn camera_params(sun_alt_deg: f64) -> CameraParams {
    let a = if sun_alt_deg.is_nan() { 90.0 } else { sun_alt_deg };
    let t = ((a + 9.0) / 11.0).clamp(0.0, 1.0);
    let day = t * t * (3.0 - 2.0 * t);
    let mix = |n: f64, d: f64| n + (d - n) * day;
    CameraParams {
        shadow_contrast: mix(1.0, 0.88),
        highlight_contrast: mix(0.92, 0.78),
        window: [mix(0.75, 0.35), mix(0.99, 0.90)],
        compensation_ev: mix(-1.5, -0.5),
        max_lift_ev: mix(0.75, 1.5),
        max_cut_ev: 2.0,
    }
}

/// The grade's keyframes under the camera model (`GRADE_KEYS` order, so
/// `grade::key_weights` blends them): the curve's flatter mid-tones take more
/// contrast, the golden hour a warm key against cool shadows, the blue hour a
/// cool ambient with warm lights, the night coloured blacks.
pub const CAM_GRADE_KEYS: [grade::GradeKey; grade::KEY_COUNT] = [
    grade::GradeKey { name: "cam-night", contrast: 1.22, pivot: 0.30, sat_shadow: 0.92, sat_highlight: 1.22, shadow_balance: [0.90, 0.99, 1.14], highlight_balance: [1.05, 1.0, 0.93], split: 0.30, gain: [1.0, 1.0, 1.0] },
    grade::GradeKey { name: "cam-blue-hour", contrast: 1.16, pivot: 0.38, sat_shadow: 1.10, sat_highlight: 1.15, shadow_balance: [0.90, 0.98, 1.14], highlight_balance: [1.06, 1.0, 0.92], split: 0.36, gain: [0.97, 1.0, 1.05] },
    grade::GradeKey { name: "cam-golden-am", contrast: 1.16, pivot: 0.42, sat_shadow: 1.0, sat_highlight: 1.12, shadow_balance: [0.95, 1.0, 1.06], highlight_balance: [1.06, 1.0, 0.92], split: 0.38, gain: [1.01, 1.0, 0.98] },
    grade::GradeKey { name: "cam-golden-pm", contrast: 1.18, pivot: 0.42, sat_shadow: 1.0, sat_highlight: 1.16, shadow_balance: [0.93, 1.0, 1.08], highlight_balance: [1.10, 1.0, 0.86], split: 0.36, gain: [1.03, 1.0, 0.95] },
    grade::GradeKey { name: "cam-day", contrast: 1.15, pivot: 0.45, sat_shadow: 1.06, sat_highlight: 1.08, shadow_balance: [0.98, 1.0, 1.03], highlight_balance: [1.02, 1.0, 0.97], split: 0.40, gain: [1.0, 1.0, 1.0] },
    grade::GradeKey { name: "cam-overcast", contrast: 1.10, pivot: 0.45, sat_shadow: 0.98, sat_highlight: 1.02, shadow_balance: [0.97, 1.0, 1.04], highlight_balance: [1.01, 1.0, 0.99], split: 0.40, gain: [0.99, 1.0, 1.01] },
];

/// The filmic curve's constants: an AgX-style log sigmoid (Sobotka's formulation
/// and its inset and outset matrices, as published with Blender's AgX view; the
/// sixth-order fit of its sigmoid) over -10 .. +6.5 EV about middle grey.
pub const CAM_AGX_EV: [f64; 2] = [-12.47393, 4.026069];
/// The sharpen's lobe at full strength (CAS: 1/8 its softest, 1/5 its strongest).
pub const CAM_SHARPEN: f64 = 0.15;
/// The scene's contrast about middle grey before the sigmoid (log over log):
/// the sigmoid alone is a flat film (about 0.8-1.1 at grey); 1.25 puts -3 EV at
/// sRGB 31, grey at 127, +2 EV at 201 and white near +5 EV.
pub const CAM_CONTRAST: f64 = 1.25;
/// The look's saturation on the curve's encoded values (the sigmoid's inset
/// desaturates the mid-tones a little).
pub const CAM_SATURATION: f64 = 1.10;

/// eo-look's curve as WGSL constants (`CN_*`).
pub fn wgsl() -> String {
    let c = grade::curve_params();
    let f = |v: f64| format!("{:?}", v as f32);
    format!(
        "\n// generated by cine_post.rs wgsl (eo-look cinematic::grade)\nconst CN_X0: f32 = {};\nconst CN_Y0: f32 = {};\nconst CN_SLOPE: f32 = {};\nconst CN_TOE_POWER: f32 = {};\nconst CN_X1: f32 = {};\nconst CN_Y1: f32 = {};\nconst CN_RATE: f32 = {};\nconst CN_WHITE_X: f32 = {};\nconst CN_WHITEN_EV0: f32 = {};\nconst CN_WHITEN_EV1: f32 = {};\nconst CN_MAX_X: f32 = {:.1};\n",
        f(c.x0),
        f(c.y0),
        f(c.slope),
        f(c.toe_power),
        f(c.x1),
        f(c.y1),
        f(c.rate),
        f(grade::CINE_WHITE_X),
        f(grade::CINE_WHITEN_EV[0]),
        f(grade::CINE_WHITEN_EV[1]),
        grade::CINE_MAX_X,
    )
}

/// The pass's uniform struct (look_cine.wgsl and cine_camera.wgsl).
pub const CINE_STRUCT_WGSL: &str = "
struct Cine {
    // x the bloom's share of the bright part (eo-look `CINE_BLOOM_SHARE`); yzw unused
    bloom: vec4<f32>,
    // x the display vignette, y the table's scale, z its offset (a texel centre); w unused
    grade: vec4<f32>,
    // (`--cine-camera`) x local contrast in the shadows, y in the highlights, z the meter's largest lift (EV), w on
    cam0: vec4<f32>,
    // x, y the meter's window, z the compensation (EV), w the meter's largest cut (EV)
    cam1: vec4<f32>,
    // x grain, y the sun glare's gain
    cam2: vec4<f32>,
};

// (`--cine-camera`) the meter's state (cine_camera.wgsl `cam_adapt` writes it, the post pass reads it).
struct CamState {
    // log2 of the exposure in use (the hour's x the meter's), smoothed
    ev: f32,
    // the meter's gain over the hour's exposure (the post's multiplier)
    gain: f32,
    // the metered log-average (log2, exposed by the hour's exposure)
    mean: f32,
    // the visible sun's mean exposed luminance over its disc (0: hidden or off the frame)
    sun_e: f32,
    // 1 once a frame has been metered
    init: f32,
    pad0: f32,
    pad1: f32,
    pad2: f32,
};
";

/// (`--cine-camera`) the camera model's WGSL constants.
pub fn camera_wgsl() -> String {
    let f = |v: f64| format!("{:?}", v as f32);
    format!(
        "\n// generated by cine_post.rs camera_wgsl (`--cine-camera`)\nconst CAM_HIST_BINS: u32 = {}u;\nconst CAM_HIST_LO: f32 = {};\nconst CAM_HIST_HI: f32 = {};\nconst CAM_CELL_PX: u32 = {}u;\nconst CAM_GRID_Z: u32 = {}u;\nconst CAM_GRID_LO: f32 = {};\nconst CAM_GRID_HI: f32 = {};\nconst CAM_BILATERAL_SHARE: f32 = {};\nconst CAM_ADAPT_RATE: f32 = {};\nconst CAM_VIGNETTE: f32 = {};\nconst CAM_FLARE_EV: vec2<f32> = vec2<f32>({}, {});\nconst CAM_AGX_LO: f32 = {};\nconst CAM_AGX_HI: f32 = {};\nconst CAM_CONTRAST: f32 = {};\nconst CAM_SATURATION: f32 = {};\nconst CAM_SHARPEN: f32 = {};\n",
        CAM_HIST_BINS,
        f(CAM_HIST_EV[0]),
        f(CAM_HIST_EV[1]),
        CAM_CELL_PX,
        CAM_GRID_Z,
        f(CAM_GRID_EV[0]),
        f(CAM_GRID_EV[1]),
        f(CAM_BILATERAL_SHARE),
        f(CAM_ADAPT_RATE),
        f(CAM_VIGNETTE),
        f(CAM_FLARE_EV[0]),
        f(CAM_FLARE_EV[1]),
        f(CAM_AGX_EV[0]),
        f(CAM_AGX_EV[1]),
        f(CAM_CONTRAST),
        f(CAM_SATURATION),
        f(CAM_SHARPEN),
    )
}

/// The pass's whole shader: photo.rs's (the look's common source, the old
/// display transform's helpers, the photo finish's lens and noise), the
/// constants, and look_cine.wgsl; (`--cine-camera`, `SF_CAMERA`) with the
/// camera model's bindings and constants and the bloom's share.
pub fn shader_source() -> String {
    // (`--cine-camera`) the model's bindings, constants and fragment path only when it is
    // compiled in; else a stub the pass's dead `SF_CAMERA` branch names.
    let cam = if crate::look::shader_features().camera {
        format!(
            "{}{}\n@group(0) @binding(9) var<storage, read> cam_state: CamState;\n@group(0) @binding(10) var cam_grid: texture_3d<f32>;\n{}",
            camera_wgsl(),
            bloom_wgsl(true).replace("const SF_CAMERA: bool = true;\n", ""),
            include_str!("look_cine_camera.wgsl")
        )
    } else {
        "\nfn cine_camera(pos: vec2<f32>, dims: vec2<f32>) -> vec4<f32> {\n    return vec4<f32>(0.0);\n}\n".to_string()
    };
    format!("{}\n{}\n{}\n{}\n{}", crate::photo::shader_source(), wgsl(), CINE_STRUCT_WGSL, include_str!("look_cine.wgsl"), cam)
}

/// (`--cine-camera`) the meter's, the grid's and the sun's compute passes.
pub fn camera_compute_source() -> String {
    format!("{}\n{}{}\n{}", crate::look::look_common_source(), CINE_STRUCT_WGSL, camera_wgsl(), include_str!("cine_camera.wgsl"))
}

/// The table's texels as RGBA f16 (alpha 1).
fn lut_bytes(lut: &[[f32; 3]]) -> Vec<u8> {
    let one = crate::ambient_bake::f16_bits(1.0);
    let mut out = Vec::with_capacity(lut.len() * 8);
    for t in lut {
        for v in [crate::ambient_bake::f16_bits(t[0]), crate::ambient_bake::f16_bits(t[1]), crate::ambient_bake::f16_bits(t[2]), one] {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out
}

/// The pipeline, its layout, its uniforms and the grade's table.
pub struct CineGpu {
    pub(crate) pipeline: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    /// the photo finish's uniform for this pass (zeros with the photo finish off: no lens).
    photo: wgpu::Buffer,
    lut: wgpu::Texture,
    lut_view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    /// each key's table (`grade::GRADE_KEYS` order), baked once.
    keys: Vec<Vec<[f32; 3]>>,
    /// (`--cine-camera`) the camera's keys' tables (`CAM_GRADE_KEYS`), when its code is compiled in.
    cam_keys: Vec<Vec<[f32; 3]>>,
    /// the weights the table in use was baked for.
    last: Mutex<Option<[f64; grade::KEY_COUNT]>>,
    /// how many times the table was baked and uploaded.
    uploads: std::sync::atomic::AtomicU64,
    /// (`--cine-camera`) the camera model's meter, made only when its code is compiled in.
    camera: Option<CameraGpu>,
}

/// (`--cine-camera`) the meter's passes and their buffers; the grid's are sized with the frame.
struct CameraGpu {
    bgl: wgpu::BindGroupLayout,
    build: wgpu::ComputePipeline,
    adapt: wgpu::ComputePipeline,
    blur: wgpu::ComputePipeline,
    hist: wgpu::Buffer,
    state: wgpu::Buffer,
    /// the frame's size the grid was made for, its buffers, texture and the passes' bind groups per post source.
    sized: Mutex<Option<CameraSized>>,
    /// the model on this frame (`CineGpu::write`).
    on: std::sync::atomic::AtomicBool,
}

struct CameraSized {
    size: (u32, u32),
    columns: (u32, u32),
    binds: [wgpu::BindGroup; 3],
    bytes: u64,
}

impl CameraGpu {
    fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("cine_camera.wgsl"), source: wgpu::ShaderSource::Wgsl(camera_compute_source().into()) });
        let cs = wgpu::ShaderStages::COMPUTE;
        let e = |binding, ty| wgpu::BindGroupLayoutEntry { binding, visibility: cs, ty, count: None };
        let ubuf = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None };
        let sbuf = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cine-camera-bgl"),
            // the look, the sky, the frame, a linear sampler, the histogram, the state, the cells,
            // the columns, the grid texture, the pass's uniform.
            entries: &[
                e(0, ubuf),
                e(1, ubuf),
                e(2, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                e(3, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                e(4, sbuf),
                e(5, sbuf),
                e(6, sbuf),
                e(7, sbuf),
                e(8, wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: wgpu::TextureFormat::Rgba16Float, view_dimension: wgpu::TextureViewDimension::D3 }),
                e(9, ubuf),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("cine-camera-layout"), bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
        let pipe = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(entry), layout: Some(&layout), module: &module, entry_point: Some(entry), compilation_options: Default::default(), cache: None })
        };
        let buffer = |label, size| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        Self {
            build: pipe("cam_build"),
            adapt: pipe("cam_adapt"),
            blur: pipe("cam_blur"),
            bgl,
            hist: buffer("cine-camera-hist", CAM_HIST_BINS as u64 * 4),
            state: buffer("cine-camera-state", 32),
            sized: Mutex::new(None),
            on: Default::default(),
        }
    }

    /// The grid for a frame of `size` and the passes' bind groups over the post's sources.
    #[allow(clippy::too_many_arguments)]
    fn ensure(&self, device: &wgpu::Device, size: (u32, u32), look: &wgpu::Buffer, srcs: [&wgpu::TextureView; 3], sky: &wgpu::Buffer, sampler: &wgpu::Sampler, cine: &wgpu::Buffer) -> wgpu::TextureView {
        let mut g = self.sized.lock().unwrap();
        let columns = (size.0.div_ceil(CAM_CELL_PX).max(1), size.1.div_ceil(CAM_CELL_PX).max(1));
        let n = columns.0 as u64 * columns.1 as u64;
        let buffer = |label, size| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false });
        let cells = buffer("cine-camera-cells", n * CAM_GRID_Z as u64 * 8);
        let cols = buffer("cine-camera-columns", n * 8);
        let grid = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cine-camera-grid"),
            size: wgpu::Extent3d { width: columns.0, height: columns.1, depth_or_array_layers: CAM_GRID_Z },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let grid_view = grid.create_view(&Default::default());
        let binds = srcs.map(|src| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("cine-camera-bg"),
                layout: &self.bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: look.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: sky.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(src) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(sampler) },
                    wgpu::BindGroupEntry { binding: 4, resource: self.hist.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 5, resource: self.state.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 6, resource: cells.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 7, resource: cols.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&grid_view) },
                    wgpu::BindGroupEntry { binding: 9, resource: cine.as_entire_binding() },
                ],
            })
        });
        let bytes = n * CAM_GRID_Z as u64 * (8 + 8) + n * 8 + CAM_HIST_BINS as u64 * 4 + 32;
        *g = Some(CameraSized { size, columns, binds, bytes });
        grid_view
    }

    /// The meter's passes over post source `src` (before the post pass reads the meter).
    fn encode(&self, enc: &mut wgpu::CommandEncoder, src: usize) {
        if !self.on.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let g = self.sized.lock().unwrap();
        let Some(t) = g.as_ref() else { return };
        enc.clear_buffer(&self.hist, 0, None);
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("cine-camera"), timestamp_writes: None });
        pass.set_bind_group(0, &t.binds[src.min(2)], &[]);
        pass.set_pipeline(&self.build);
        pass.dispatch_workgroups(t.columns.0, t.columns.1, 1);
        pass.set_pipeline(&self.adapt);
        pass.dispatch_workgroups(1, 1, 1);
        pass.set_pipeline(&self.blur);
        pass.dispatch_workgroups(t.columns.0.div_ceil(4), t.columns.1.div_ceil(4), CAM_GRID_Z.div_ceil(4));
        let _ = t.size;
    }
}

impl CineGpu {
    pub fn new(device: &wgpu::Device, output_format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("look_cine.wgsl"), source: wgpu::ShaderSource::Wgsl(shader_source().into()) });
        let fs = wgpu::ShaderStages::FRAGMENT;
        let ubuf = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None };
        let tex = |d| wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: d, multisampled: false };
        let samp = wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering);
        let e = |binding, ty| wgpu::BindGroupLayoutEntry { binding, visibility: fs, ty, count: None };
        let d2 = wgpu::TextureViewDimension::D2;
        let camera_in = crate::look::shader_features().camera;
        let mut entries = vec![e(0, ubuf), e(1, tex(d2)), e(2, ubuf), e(3, tex(d2)), e(4, samp), e(5, ubuf), e(6, tex(wgpu::TextureViewDimension::D3)), e(7, samp), e(8, ubuf)];
        if camera_in {
            // (`--cine-camera`) the meter's state and the local exposure's grid.
            entries.push(e(9, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }));
            entries.push(e(10, tex(wgpu::TextureViewDimension::D3)));
        }
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cine-bgl"),
            // as the photo pass's (the look, the frame, the sky, the bloom, a linear
            // sampler, the photo uniform), then the table, its sampler, the pass's uniform.
            entries: &entries,
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("cine-layout"), bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("post-cine"),
            layout: Some(&layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("photo_vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("cine_fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: output_format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let buffer = |label, size| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let uniform = buffer("cine-uniform", std::mem::size_of::<CineUniform>() as u64);
        let photo = buffer("cine-photo-uniform", std::mem::size_of::<crate::photo::PhotoUniform>() as u64);
        let n = grade::LUT_SIZE as u32;
        let lut = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cine-grade-lut"),
            size: wgpu::Extent3d { width: n, height: n, depth_or_array_layers: n },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let lut_view = lut.create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("cine-grade-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let keys = grade::GRADE_KEYS.iter().map(grade::bake_lut).collect();
        // (`--cine-camera`) the camera's keys for its curve.
        let cam_keys = if camera_in { CAM_GRADE_KEYS.iter().map(grade::bake_lut).collect() } else { Vec::new() };
        let camera = camera_in.then(|| CameraGpu::new(device));
        Self { pipeline, bgl, uniform, photo, lut, lut_view, sampler, keys, cam_keys, last: Mutex::new(None), uploads: Default::default(), camera }
    }

    /// One bind group per post source (the HDR scene, the resolved history 0, 1).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn bind_groups(&self, device: &wgpu::Device, look: &wgpu::Buffer, srcs: [&wgpu::TextureView; 3], sky: &wgpu::Buffer, bloom: &wgpu::TextureView, sampler: &wgpu::Sampler) -> [wgpu::BindGroup; 3] {
        // (`--cine-camera`) the meter's grid for this frame size and its passes' bind groups.
        let grid = self.camera.as_ref().map(|c| {
            let t = srcs[0].texture();
            c.ensure(device, (t.width(), t.height()), look, srcs, sky, sampler, &self.uniform)
        });
        srcs.map(|src| {
            let mut entries = vec![
                    wgpu::BindGroupEntry { binding: 0, resource: look.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(src) },
                    wgpu::BindGroupEntry { binding: 2, resource: sky.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(bloom) },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(sampler) },
                    wgpu::BindGroupEntry { binding: 5, resource: self.photo.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&self.lut_view) },
                    wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                    wgpu::BindGroupEntry { binding: 8, resource: self.uniform.as_entire_binding() },
            ];
            if let (Some(c), Some(grid)) = (&self.camera, &grid) {
                entries.push(wgpu::BindGroupEntry { binding: 9, resource: c.state.as_entire_binding() });
                entries.push(wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(grid) });
            }
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("cine-bg"),
                layout: &self.bgl,
                entries: &entries,
            })
        })
    }

    /// (`--cine-camera`) the meter's passes over post source `src`, before the post pass.
    pub(crate) fn encode_camera(&self, enc: &mut wgpu::CommandEncoder, src: usize) {
        if let Some(c) = &self.camera {
            c.encode(enc, src);
        }
    }

    /// The frame's uniforms (the photo finish's when it is on, else none) and
    /// the grade for the sun's altitude, the hour and the weather's overcast:
    /// the table is baked and uploaded only when the keys' weights have moved
    /// by more than `grade::LUT_STEP` since it was last (always the first time).
    /// Returns whether it was.
    /// `camera`: the camera model draws this frame (`--cine-camera`; sky.rs `camera_on`):
    /// its uniform terms, its meter's passes and its grade's keys.
    pub(crate) fn write(&self, queue: &wgpu::Queue, photo: Option<&crate::photo::PhotoUniform>, sun_alt_deg: f64, hour: f64, overcast: f64, camera: bool) -> bool {
        let camera = camera && self.camera.is_some();
        let u = if camera { CineUniform::with_camera(sun_alt_deg) } else { CineUniform::new() };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
        queue.write_buffer(&self.photo, 0, bytemuck::bytes_of(&photo.copied().unwrap_or_default()));
        if let Some(c) = &self.camera {
            // a change of model bakes the table again (its keys are the model's).
            if c.on.swap(camera, std::sync::atomic::Ordering::Relaxed) != camera {
                *self.last.lock().unwrap() = None;
            }
        }
        let w = grade::key_weights(sun_alt_deg, hour, overcast);
        let mut last = self.last.lock().unwrap();
        if !grade::lut_needs_update(last.as_ref(), &w) {
            return false;
        }
        let lut = grade::blend_luts(if camera { &self.cam_keys } else { &self.keys }, &w);
        let n = grade::LUT_SIZE as u32;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &self.lut, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &lut_bytes(&lut),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(n * 8), rows_per_image: Some(n) },
            wgpu::Extent3d { width: n, height: n, depth_or_array_layers: n },
        );
        *last = Some(w);
        self.uploads.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        true
    }

    /// The weights the table in use was baked for (None before the first frame).
    pub fn weights(&self) -> Option<[f64; grade::KEY_COUNT]> {
        *self.last.lock().unwrap()
    }

    /// How many times the table has been baked and uploaded.
    pub fn uploads(&self) -> u64 {
        self.uploads.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Bytes of its table and uniforms (and the camera model's meter and grid).
    pub fn bytes(&self) -> u64 {
        let n = grade::LUT_SIZE as u64;
        let cam = self.camera.as_ref().and_then(|c| c.sized.lock().unwrap().as_ref().map(|t| t.bytes)).unwrap_or(0);
        n * n * n * 8 + self.uniform.size() + self.photo.size() + cam
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::naga;

    #[test]
    fn the_shader_validates_and_its_uniform_matches() {
        let src = shader_source();
        let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("look_cine.wgsl: {}", e.emit_to_string(&src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).expect("look_cine.wgsl validates");
        let (_, ty) = module.types.iter().find(|(_, t)| t.name.as_deref() == Some("Cine")).expect("Cine struct");
        let naga::TypeInner::Struct { span, .. } = &ty.inner else { panic!("Cine is a struct") };
        assert_eq!(*span as usize, std::mem::size_of::<CineUniform>());
        for entry in ["photo_vs", "cine_fs"] {
            assert!(module.entry_points.iter().any(|e| e.name == entry), "{entry}");
        }
    }

    /// The sky's effects (sky_fx.wgsl, with the lamp glare) compose and validate
    /// with the look compiled in and out.
    #[test]
    fn the_glare_shader_validates_with_and_without_the_look() {
        for cinematic in [false, true] {
            let f = crate::look::ShaderFeatures { cinematic, ..crate::look::ShaderFeatures::NONE };
            crate::look::with_shader_features(f, || {
                let src = format!("{}\n{}", crate::look::look_common_source(), include_str!("sky_fx.wgsl"));
                let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("sky_fx.wgsl: {}", e.emit_to_string(&src)));
                naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).expect("sky_fx.wgsl validates");
                assert!(module.entry_points.iter().any(|e| e.name == "glare_vs") && module.entry_points.iter().any(|e| e.name == "glare_fs"));
            });
        }
        // the glare's parameters carry the look's mark (a negative w) and scale with the frame.
        let g = crate::sky::glare_params(1080);
        assert!(g[3] < 0.0 && (g[1] as f64 - grade::CINE_GLARE_LENGTH_PX).abs() < 1e-4);
        assert!((crate::sky::glare_params(540)[1] - g[1] * 0.5).abs() < 1e-4);
    }

    /// (`--cine-camera`) the post pass with the model compiled in, the meter's
    /// passes and the bloom with and without it validate; the uniform and the
    /// state match their WGSL structs.
    #[test]
    fn the_camera_models_shaders_validate() {
        let validate = |name: &str, src: &str| {
            let module = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(src)));
            naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            module
        };
        let on = crate::look::ShaderFeatures { cinematic: true, camera: true, ..crate::look::ShaderFeatures::NONE };
        crate::look::with_shader_features(on, || {
            let m = validate("look_cine.wgsl + camera", &shader_source());
            let (_, ty) = m.types.iter().find(|(_, t)| t.name.as_deref() == Some("Cine")).expect("Cine struct");
            let naga::TypeInner::Struct { span, .. } = &ty.inner else { panic!("Cine is a struct") };
            assert_eq!(*span as usize, std::mem::size_of::<CineUniform>());
            let (_, ty) = m.types.iter().find(|(_, t)| t.name.as_deref() == Some("CamState")).expect("CamState struct");
            let naga::TypeInner::Struct { span, .. } = &ty.inner else { panic!("CamState is a struct") };
            assert_eq!(*span, 32, "the state buffer's size");
            let c = validate("cine_camera.wgsl", &camera_compute_source());
            for entry in ["cam_build", "cam_adapt", "cam_blur"] {
                assert!(c.entry_points.iter().any(|e| e.name == entry), "{entry}");
            }
        });
        for camera in [false, true] {
            let src = format!("{}\n{}\n{}", include_str!("sky_common.wgsl"), bloom_wgsl(camera), include_str!("sky_bloom.wgsl"));
            validate("sky_bloom.wgsl", &src);
        }
        // compiled out: the stub, and none of the model's bindings.
        crate::look::with_shader_features(crate::look::ShaderFeatures { cinematic: true, ..crate::look::ShaderFeatures::NONE }, || {
            let src = shader_source();
            assert!(!src.contains("cam_state") && !src.contains("cam_grid"));
            validate("look_cine.wgsl", &src);
        });
    }

    /// (`--cine-camera`) the bloom's levels sum to 1 and lean to the inner levels; the
    /// parameters move from the night's to the day's over the sun's altitude.
    #[test]
    fn the_camera_models_terms_hold() {
        let w = bloom_level_weights();
        assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        assert!(w.windows(2).all(|p| p[0] >= p[1]));
        let (n, d) = (camera_params(-20.0), camera_params(30.0));
        assert!(n.shadow_contrast > d.shadow_contrast && n.max_lift_ev < d.max_lift_ev && n.window[0] > d.window[0]);
        assert!(n.compensation_ev < d.compensation_ev);
        assert_eq!(camera_params(f64::NAN), d);
        let u = CineUniform::with_camera(30.0);
        assert_eq!(u.cam0[3], 1.0);
        assert_eq!(CineUniform::new().cam0[3], 0.0);
    }

    #[test]
    fn the_constants_are_eo_looks_curve() {
        let src = wgsl();
        let c = grade::curve_params();
        let get = |name: &str| -> f64 {
            let line = src.lines().find(|l| l.starts_with(&format!("const {name}:"))).unwrap_or_else(|| panic!("{name}"));
            line.split('=').nth(1).unwrap().trim().trim_end_matches(';').parse().unwrap()
        };
        for (n, v) in [("CN_X0", c.x0), ("CN_Y0", c.y0), ("CN_SLOPE", c.slope), ("CN_X1", c.x1), ("CN_Y1", c.y1), ("CN_RATE", c.rate), ("CN_WHITE_X", grade::CINE_WHITE_X)] {
            assert!((get(n) - v).abs() <= 1e-6 * v.abs(), "{n}");
        }
        let u = CineUniform::new();
        // the table's texel centres: 0 -> half a texel in, 1 -> half a texel short of the end.
        assert!((u.grade[2] - 0.5 / 32.0).abs() < 1e-9 && (u.grade[1] + u.grade[2] - 31.5 / 32.0).abs() < 1e-7);
        assert_eq!(lut_bytes(&[[0.5, 0.25, 1.0]]).len(), 8);
    }
}
