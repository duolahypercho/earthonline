//! the sea, on the GPU-driven path (see native/README.md "Water").
//!
//! Per frame, when attached and on (`Renderer::water_active`):
//!
//! 1. **Patch cull** (`water_cull.wgsl`): one invocation per patch of the
//!    camera-following grid; frustum,
//!    terrain extent, "wholly over land" from the shore field's max pyramid,
//!    and with occlusion on the city's hierarchical-Z test inside the city's
//!    own pyramid pass (the water is never drawn into that depth: a receiver,
//!    not an occluder). Survivors are listed for ONE instanced indirect draw
//!    (its instance count written by the cull), so the sea needs no
//!    multi-draw-indirect-count and draws on every adapter.
//! 2. **FFT** (`water_fft.wgsl`, two dispatches), only while water within
//!    `GATE_NEAR_M` was drawn in the last `SEEN_HOLD_FRAMES` frames (the cull's
//!    counters, read back without waiting).
//! 3. **Copy** of the HDR frame (the refracted bottom and the mirror read it),
//!    same condition.
//! 4. **Water pass** after the late city pass: depth tested against (and
//!    reading) the opaque depth, never writing it; `fs_copy`, or `fs_blend`
//!    when no copy was taken this frame; nothing when every water point lies
//!    past the range fog's far end (it would draw the fog colour the fogged
//!    seabed already shows).
//! 5. The city's glass and the agents' glass are drawn after the water, in
//!    the water pass (they test the read-only depth and never write it).
//!
//! Nothing here loops over objects on the CPU: the CPU writes two uniforms
//! and, every frame, maps one 32-byte counter from two frames back.

use crate::camera::{Camera, frustum_planes};
use bytemuck::{Pod, Zeroable};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};

/// Cascade 0 (the 733 m tile: the open-ocean swell) in the bay. The JS draws
/// its Ocean Beach spectrum (Hs 2 m) over all water; the bay along the city
/// front is fetch-limited, and the JS's own bay figure is 0.35 m at its
/// reference wind (water-surface.js `buildSwellField`). Cascades 1-3 (the
/// wind sea) carry Hs 0.215 m here, so the swell keeps 0.14 of its amplitude:
/// sqrt((0.14 x 1.988)^2 + 0.215^2) = 0.35 m. West of the Golden Gate the full
/// spectrum (the same GOLDEN_GATE_X blend as the body water).
pub const BAY_SWELL: f32 = 0.14;
/// The land mask (ocean-open-sea.js `FAR_SHORE_REACH_M`) and its beach form.
pub const MASK_M: f32 = -4.0;
pub const BEACH_MASK_M: f32 = -60.0;
/// Patch box half height: the displaced surface's reach (a storm's
/// Pacific, Hs 4-5 m, crests to about 7 m).
pub const BOX_M: f32 = 8.0;
/// Frames the FFT and copy keep running after the last frame water was seen.
pub const SEEN_HOLD_FRAMES: u64 = 30;
/// Water counts as seen (the FFT and copy run) when a drawn patch is this
/// near: past it the cascades are mostly faded by the footprint and range, a
/// spectrum a few frames stale does not show, and the mirror falls back to the
/// sky and probe (a sliver of the bay down a street 3 km off costs nothing).
pub const GATE_NEAR_M: f32 = 1500.0;
/// The water's shading features (each with its kill switch; all on by default,
/// `LookOptions::water_features`):
/// - `FEAT_BRDF` (`--no-water-brdf`): the slope variance the pixel does not
///   resolve taken from the sea state's own spectrum and the Cox-Munk total
///   (what the cascades lose to the footprint goes to the BRDF, so the far
///   bay keeps its roughness), the Fresnel averaged over the visible facets
///   (Bruneton et al. 2010) and a Beckmann sun lobe of that variance.
/// - `FEAT_BODY` (`--no-water-body`): the water-leaving radiance from the
///   bay's and the coast's inherent optical properties (eo-water `optics`
///   `deep_rrs`) under the irradiance the city is lit with.
/// - `FEAT_FOAM` (`--no-water-foam-cover`): whitecaps cover what the
///   Monahan and O'Muircheartaigh fit gives (eo-water `FOAM_OVER_TOTAL`), and
///   a pixel that spans several foam texels shows their mean, not one texel.
/// - `FEAT_INSTANCED` (`--no-water-instanced`): the pass draws on an adapter
///   without multi-draw-indirect-count (its patches are one instanced
///   indirect draw); off, it needs the count as before, and without it the
///   flat sea quad draws.
/// - `FEAT_GLINT` (`--no-water-glint`): the sun's glints counted per pixel
///   (a Poisson count of the facets that mirror the sun's disc, from the
///   slope distribution), so the glitter path tapers and feathers by itself
///   and merges into a sheen far off; each pixel's glint held under a
///   screen-referred ceiling (no fireflies) and the water's output kept finite.
/// - `FEAT_TURBID` (`--no-water-turbid`): the body's reflectance and
///   attenuation integrated over the visible spectrum for the bay's sediment
///   (eo-water `BAY_TURBID_IOP`) and the coast.
/// - `FEAT_SHORE` (`--no-water-shore`): the shoreline contact: the waterline
///   fades over the last decimetres of column, foam lines where the water
///   meets a wall, a pier or a pile, the water darkened against them.
/// - `FEAT_FOAM_LIT` (`--no-water-foam-lit`): whitecaps as lit, lacy foam
///   (a coverage-threshold dissolve of bubble cells, wind-aligned residual
///   streaks, each crest its own size) at measured foam albedo under the
///   light the city is lit with.
/// - `FEAT_PPR` (`--no-water-ppr`): the frame's own geometry projected across
///   the sea plane (pixel-projected reflection), so the bridge decks, cables
///   and the skyline stand in the water; the march stays for what it misses.
/// - `FEAT_WIND` (`--no-water-wind-texture`): cat's-paw gust patches, the
///   tidal current's rough water through the Golden Gate and a rotated
///   re-read of the chop cascade against the wind's ribbons.
/// - `FEAT_GRAZE` (`--no-water-grazing`): the sky reflection integrated over
///   the unresolved facets the eye sees (each tilt's own Fresnel and sky), so
///   a grazing view's far water mirrors the bright low sky as it should.
pub const FEAT_BRDF: u32 = 1;
pub const FEAT_BODY: u32 = 2;
pub const FEAT_FOAM: u32 = 4;
pub const FEAT_INSTANCED: u32 = 8;
pub const FEAT_GLINT: u32 = 16;
pub const FEAT_TURBID: u32 = 32;
pub const FEAT_SHORE: u32 = 64;
pub const FEAT_FOAM_LIT: u32 = 128;
pub const FEAT_PPR: u32 = 256;
pub const FEAT_WIND: u32 = 512;
pub const FEAT_GRAZE: u32 = 1024;
pub const FEAT_ALL: u32 = FEAT_BRDF | FEAT_BODY | FEAT_FOAM | FEAT_INSTANCED | FEAT_GLINT | FEAT_TURBID | FEAT_SHORE | FEAT_FOAM_LIT | FEAT_PPR | FEAT_WIND | FEAT_GRAZE;

/// The cull's counters: drawn, in the frustum, land, occluded, nearest drawn (m), pad.
const COUNT_BYTES: u64 = 32;
/// Under the cinematic look (stage L5) the sea's reach (water_common.wgsl
/// `wg_reach`) runs this much further south too, as it does west, north and
/// east: the South Bay and the Pacific off the peninsula are the sea's, the
/// peninsula's land hides it by depth (`set_south_reach`).
pub const CINEMATIC_SOUTH_REACH_M: f32 = 60_000.0;
/// Under the cinematic look (stage L5) the sea pass stands down for distance
/// past this view depth (all its water at least this far: a camera 3.5 km or
/// more up), and the far layer draws the water in its place, inside the city
/// grid too (farshore.wgsl `fs_vs`, the sheet). The fog in use (2e7 m with
/// the volume's aerial perspective) alone would never skip it: from the
/// overview's 8 km the sea pass then cost 1.1 ms of GPU at high (15.6 ->
/// 16.7 ms, over the tier's 16.6), for waves no pixel resolves up there.
pub const CINEMATIC_SEA_LOD_M: f32 = 3_000.0;

/// All water is past the fog's far end (`fog_far`, view depth): a point in the
/// frustum lies within the half-diagonal angle of the view axis, so its view
/// depth (what the range fog reads) is at least its distance times that
/// angle's cosine, and no water is nearer than the camera's height over the
/// displaced surface. When even that bound is past the fog's far end every
/// water pixel would be the fog colour, which the fogged seabed behind it
/// already is: the sea pass draws nothing (`update`).
pub fn sea_fogged(cam: &Camera, fog_far: f32) -> bool {
    let sea = eo_water::SEA_LEVEL_Y as f32;
    let half_diag = ((cam.fov_y_deg.to_radians() * 0.5).tan() * (1.0 + cam.aspect * cam.aspect).sqrt()).atan();
    (cam.eye.y - sea - BOX_M).max(0.0) * half_diag.cos() >= fog_far
}

/// After them in the same buffer, the sea's one DrawIndexedIndirect (its
/// instance count the patches drawn, counted by the cull), then pad.
const DRAW_ARGS_OFFSET: u64 = 32;
const COUNT_BUF_BYTES: u64 = 64;

const PATCH_COUNT: u32 = eo_water::grid::PATCHES * eo_water::grid::PATCHES;
const N: u32 = eo_water::spectrum::N as u32;
const CASCADES: u32 = eo_water::spectrum::CASCADES as u32;
/// FFT steps one frame may run (the capture warm-up of the foam), and
/// their parameter slots (a dynamic offset each).
const MAX_STEPS: usize = 48;
const STEP_STRIDE: u64 = 256;
/// the warm-up a capture runs when its clock or sea changes: the foam
/// built from nothing over this many seconds in `MAX_STEPS` steps, so a still
/// frame is a function of its clock and sea alone.
const WARM_S: f64 = 12.0;
/// the spray's slots (quads) and the wind (m/s) it starts at.
const SPRAY_SLOTS: u32 = 1536;
const SPRAY_WIND_MS: f32 = 9.0;
/// the derivative layers carry one level (a mip chain was tried and
/// dropped: 8 dispatches over 36 tracked subresources a frame, a suspect in the
/// encoder-finish outliers; the far field reads the plan's cover and the vertex
/// reads cascade 0 at level 0).
const DERIV_MIPS: u32 = 1;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct WaterUniform {
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    cam: [f32; 4],
    fwd: [f32; 4],
    grid: [f32; 4],
    field: [f32; 4],
    extent: [f32; 4],
    screen: [f32; 4],
    flags: [f32; 4],
    sizes: [f32; 4],
    probe: [f32; 4],
    bay: [f32; 4],
    planes: [[f32; 4]; 6],
    // per cascade, the bay's and the open Pacific's amplitude (water_common.wgsl `wg_cascade_amp`)
    wv_bay: [f32; 4],
    wv_pac: [f32; 4],
    // x U10 bay, y U10 Pacific (m/s), z foam cover bay, w foam cover Pacific (fractions)
    wv_sea: [f32; 4],
    // x storm 0..1, y the sine groundswell's gain (0: the spectrum carries the swell), z/w 0
    wv_misc: [f32; 4],
    // per cascade, the GPU spectrum's mean square slope (eo-water Plan `mss_gpu`, before each place's amplitude)
    wv_mss: [f32; 4],
    // (iteration 2) the exposure field's frame: origin x, z, 1 / extent x, z
    wv_expo: [f32; 4],
    // (iteration 2) x the tidal current (m/s, + flood), y the parts on (eo-water switches), z 1 with an exposure field
    wv_gg2: [f32; 4],
    // (iteration 2) per cascade, the wind sea's amplitude on the tidal jet (`doppler_gain`)
    wv_dop: [f32; 4],
    // (iteration 2, exposure) the fetch factors and wind ratios the bay's wind sea is tabulated at
    // (eo-water waves.rs `EXPO_FETCH_NODES`, `EXPO_WIND_NODES`), and per wind node g, fetch node b
    // (at 8 g + b) the bay's amplitude per cascade there (`Plan::expo_amp`)
    wv_xn: [[f32; 4]; 2],
    wv_gn: [[f32; 4]; 2],
    wv_xa: [[f32; 4]; 64],
    // (iteration 3) the extra field's frame (eo-water extra.rs): origin x, z, 1 / extent x, z;
    // the identity (0, 0, 1, 1) without a field
    wv_extra: [f32; 4],
    // x the sea's reach's extra run south (m; `CINEMATIC_SOUTH_REACH_M` under the
    // cinematic look, stage L5, else 0: water_common.wgsl `wg_reach`); y the occupied canopy depth bands
    reach: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FftParams {
    time: f32,
    dt: f32,
    choppiness: f32,
    depth: f32,
    sizes: [f32; 4],
    // water_fft.wgsl `FftParams`
    amp_pac: [f32; 4],
    amp_bay: [f32; 4],
    thr_pac: [f32; 4],
    thr_bay: [f32; 4],
    soft_pac: [f32; 4],
    soft_bay: [f32; 4],
    foam: [f32; 4],
    wind: [f32; 4],
    drift: [f32; 4],
}

/// water_spectrum.wgsl `SpecParams` (eo-water `SpecParamsRaw`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SpecParams {
    sizes: [f32; 4],
    k_lo: [f32; 4],
    k_hi: [f32; 4],
    sa: [[f32; 4]; 3],
    sb: [[f32; 4]; 3],
    lead: [f32; 4],
    misc: [f32; 4],
}

impl From<eo_water::waves::SpecParamsRaw> for SpecParams {
    fn from(r: eo_water::waves::SpecParamsRaw) -> Self {
        Self { sizes: r.sizes, k_lo: r.k_lo, k_hi: r.k_hi, sa: r.sa, sb: r.sb, lead: r.lead, misc: r.misc }
    }
}

/// the sea's inputs (the hour, the weather's wind and severity) or a fixed state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SeaInput {
    Weather { hour: f64, wind: [f64; 2], severity: f64 },
    Fixed(eo_water::waves::SeaState),
}

/// What the app hands the water renderer.
pub struct WaterInit<'a> {
    pub data: &'a eo_water::WaterData,
    /// Where water may draw: min x, min z, max x, max z (the terrain grid, as the flat sea).
    pub extent: [f32; 4],
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WaterStats {
    /// Frames the FFT ran / the copy was taken / the water pass drew (of `frames`).
    pub fft_frames: u64,
    pub copy_frames: u64,
    pub frames: u64,
    /// Frames all water lay past the fog's far end (culled, not drawn).
    pub fogged_frames: u64,
    /// The last counters read back: patches drawn, in the frustum, land, occluded.
    pub last_counts: [u32; 4],
    /// The nearest drawn patch in that frame (m; u32::MAX for none).
    pub last_nearest_m: u32,
    pub readbacks: u64,
}

struct Readback {
    buf: wgpu::Buffer,
    /// 0 free, 1 encoded (copy recorded), 2 mapping, 3 mapped
    stage: std::sync::Arc<AtomicU8>,
    frame: u64,
}

struct State {
    frame: u64,
    time: Option<f64>,
    last_time: Option<f64>,
    radius: f64,
    offset: [f64; 2],
    /// The last frame the cull reported a patch drawn (from the readback).
    seen: Option<u64>,
    /// This frame's decisions.
    fft: bool,
    copy: bool,
    fft_ever: bool,
    near_waterline: bool,
    /// All water is past the fog's far end this frame: nothing drawn.
    fogged: bool,
    /// The reach's extra run south (m; 0, or `CINEMATIC_SOUTH_REACH_M` under the cinematic look).
    south_m: f32,
    active: bool,
    always: bool,
    stats: WaterStats,
    /// The readback slot this frame copies into.
    slot: Option<usize>,
    // the sea in force, its plan, and this frame's work
    sea_in: SeaInput,
    sea_key: Option<[i64; 4]>,
    /// (iteration 2) the day of the year the plan was made for (the station's wind by month).
    sea_day: f64,
    plan: Option<eo_water::waves::Plan>,
    /// The spectrum must be rebuilt on the GPU this frame.
    init: bool,
    /// The foam buffers are cleared before this frame's steps (a warm-up).
    reset_foam: bool,
    /// FFT steps this frame (1, or the warm-up's), and which foam buffer is read first.
    steps: usize,
    foam_flip: usize,
    /// The plan changed since the last warm-up (captures warm up again).
    plan_moved: bool,
    /// A warm-up is due and not yet encoded (a settle runs updates without encoding).
    warm_pending: bool,
    /// what the plan hands the GPU, made once per plan (no per-frame
    /// work but the clock): the FFT step template, the uniform's four vec4, the
    /// spectrum parameters.
    plan_gpu: Option<PlanGpu>,
    /// the spray draws this frame (the wind over the water is over 9 m/s).
    spray: bool,
    /// a plan being built off the render thread (the hour or the
    /// weather moved; captures build it in place).
    plan_job: Option<std::thread::JoinHandle<([i64; 4], eo_water::waves::Plan)>>,
    /// the visible foam over the whitecap fit the plans are made with (`FEAT_FOAM`).
    foam_over: f64,
}

/// a plan's GPU-side numbers.
#[derive(Clone, Copy)]
struct PlanGpu {
    step: FftParams,
    wv: [[f32; 4]; 5],
    spec: SpecParams,
    // (exposure) `Plan::expo_amp`, at 8 g + b
    xa: [[f32; 4]; 64],
}

impl PlanGpu {
    fn of(plan: &eo_water::waves::Plan) -> Self {
        let st = &plan.state;
        let f4 = |v: [f64; 4]| v.map(|x| x as f32);
        Self {
            step: WaterGpu::step_params(plan, 0.0, 0.0),
            wv: [f4(plan.amp_bay), f4(plan.amp_pac), [st.bay.u10 as f32, st.pacific.u10 as f32, plan.cover[1] as f32, plan.cover[0] as f32], [st.storm as f32, 0.0, 0.0, 0.0], f4(plan.mss_gpu)],
            spec: SpecParams::from(plan.spec_params()),
            xa: std::array::from_fn(|i| f4(plan.expo_amp[i / 8][i % 8])),
        }
    }
}

pub struct WaterGpu {
    device: wgpu::Device,
    pub bytes: u64,
    pub(crate) canopy_map: Option<wgpu::Texture>,
    canopy_view: Option<wgpu::TextureView>,
    uniform_buf: wgpu::Buffer,
    fft_buf: wgpu::Buffer,
    fft_rows: wgpu::ComputePipeline,
    fft_cols: wgpu::ComputePipeline,
    /// one per foam ping-pong side (reads A writes B, reads B writes A).
    fft_bg: [wgpu::BindGroup; 2],
    foam_bufs: [wgpu::Buffer; 2],
    spec_buf: wgpu::Buffer,
    spec_init: wgpu::ComputePipeline,
    spec_bg: wgpu::BindGroup,
    h0_buf: wgpu::Buffer,
    disp_view: wgpu::TextureView,
    deriv_view: wgpu::TextureView,
    disp_tex: wgpu::Texture,
    field_view: wgpu::TextureView,
    detail_view: wgpu::TextureView,
    index_buf: wgpu::Buffer,
    /// indices of one patch (16 x 16 quads).
    index_count: u32,
    draw_buf: wgpu::Buffer,
    count_buf: wgpu::Buffer,
    cull_plain: wgpu::ComputePipeline,
    cull_hzb: wgpu::ComputePipeline,
    cull_bg: wgpu::BindGroup,
    draw_copy: wgpu::RenderPipeline,
    draw_blend: wgpu::RenderPipeline,
    submerged: Option<(wgpu::RenderPipeline, eo_water::field::ShoreField)>,
    fallback_dive: std::sync::atomic::AtomicBool,
    /// the spray pass (water_spray.wgsl).
    spray: wgpu::RenderPipeline,
    draw_bgl: wgpu::BindGroupLayout,
    repeat: wgpu::Sampler,
    clamp: wgpu::Sampler,
    probe_sampler: wgpu::Sampler,
    scene: Mutex<Option<(u32, u32, wgpu::Texture, wgpu::TextureView)>>,
    /// (ppr) the projected reflection's target buffer (one u32 per pixel of the
    /// frame; 16 bytes while `FEAT_PPR` is off) and its pass.
    ppr: Mutex<Option<(u32, u32, wgpu::Buffer)>>,
    ppr_bgl: wgpu::BindGroupLayout,
    ppr_pipe: wgpu::ComputePipeline,
    /// the projection's bind group, made when the depth or the buffer is new.
    ppr_bg: Mutex<Option<(wgpu::Texture, wgpu::Buffer, wgpu::BindGroup)>>,
    /// per probe front cube, both made when the depth or the frame copy is new.
    draw_bg: Mutex<Option<(wgpu::Texture, wgpu::Texture, [wgpu::BindGroup; 2])>>,
    readbacks: Mutex<Vec<Readback>>,
    state: Mutex<State>,
    extent: [f32; 4],
    field_frame: [f32; 4],
    on: std::sync::atomic::AtomicBool,
    /// the shading features in force (`FEAT_*`, from `LookOptions::water_features`).
    features: u32,
    /// (iteration 2) the exposure field (a 1 x 1 stand-in without one) and its frame.
    expo_view: wgpu::TextureView,
    expo_frame: [f32; 4],
    expo_on: bool,
    /// (iteration 3) the extra field (eo-water `extra`; a 1 x 1 zero texture without one) and its frame.
    extra_view: wgpu::TextureView,
    extra_frame: [f32; 4],
    /// (iteration 2) the beaches' measured slopes are in the exposure texture's A.
    slopes_on: bool,
    /// (iteration 2) the parts on (eo-water `switches`).
    gg2: u32,
}

/// `EO_WATER_DEBUG`: the water pass draws its terms instead of the water (diagnostics).
pub(crate) fn debug_mode() -> f32 {
    static MODE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| std::env::var("EO_WATER_DEBUG").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0))
}

/// `EO_WATER_SPRAY=1` draws the spray (off by default: no normal card
/// shows open water in a strong wind, so it costs nothing until it is wanted).
fn spray_on() -> bool {
    static S: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *S.get_or_init(|| std::env::var("EO_WATER_SPRAY").is_ok_and(|v| v == "1"))
}

/// `EO_WATER_SEA` = storm | gale | calm: the sea state alone held at
/// the storm preset's, a 14 m/s afternoon westerly, or a calm morning's, whatever
/// the sky shows (diagnostics: the sea under a clear sky to see it).
fn sea_override() -> Option<eo_water::waves::SeaState> {
    static S: std::sync::OnceLock<Option<eo_water::waves::SeaState>> = std::sync::OnceLock::new();
    *S.get_or_init(|| match std::env::var("EO_WATER_SEA").ok().as_deref() {
        Some("storm") => Some(eo_water::waves::sea_state(15.0, [11.5, 4.2], 4.0)),
        Some("gale") => Some(eo_water::waves::sea_state(16.0, [10.0, 1.8], 0.0)),
        Some("calm") => Some(eo_water::waves::sea_state(7.5, [0.0, 0.0], 0.0)),
        _ => None,
    })
}

/// What the water's shaders compile in as constants (`common_source`), all
/// fixed for the life of the pipelines: the shading features (`FEAT_*`), the
/// iteration-2 and -3 parts (eo-water `switches`), the debug view
/// (`EO_WATER_DEBUG`, 0 off) and the submerged camera's wave optics
/// (`--water-optics`). The uniform still carries the same values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ShaderBits {
    pub features: u32,
    pub parts: u32,
    pub debug: f32,
    pub optics: bool,
}

/// The water shaders' shared WGSL (constants generated from eo-water, the
/// grid and the uniform), with `bits` compiled in (`W_FEAT`, `W_GG2`,
/// `W_DEBUG`, `W_OPTICS`: water_common.wgsl `wv_feat`, `wg_gg2`, water.wgsl):
/// each gate is a constant, so the branch a run does not take (a feature's
/// code when it is off, the earlier path its switch restores when it is on,
/// the debug views, the wave optics' terms) is compiled out. Gated on the
/// uniform instead, every pixel of the sea carried all of that code in its
/// registers (measured, golden_gate: the water pass 6.2 ms against 1.6 ms
/// with the features and parts as constants, the output unchanged).
pub(crate) fn common_source(cove_detail: bool, bits: ShaderBits) -> String {
    format!(
        "{}\n{}\n{}\n{}\nconst W_COVE_DETAIL: bool = {cove_detail};\nconst W_COVE_SCALE: f32 = {:.8};\nconst W_FEAT: u32 = {}u;\nconst W_GG2: u32 = {}u;\nconst W_DEBUG: f32 = {:.3};\nconst W_OPTICS: bool = {};\n",
        eo_water::optics::wgsl_constants(),
        eo_water::surface::wgsl_constants(),
        include_str!("water_common.wgsl"),
        include_str!("cove_daylight.wgsl"),
        eo_data::groundscale::scale(),
        bits.features,
        bits.parts,
        bits.debug,
        bits.optics
    )
}

/// (iteration 2, current) the wind sea's amplitude gain per cascade on the
/// Golden Gate's tidal jet for a current (m/s, + flood): wave action conserved
/// on a current along the waves (Longuet-Higgins and Stewart 1961, J. Fluid
/// Mech. 10(4), 529: a/a0 = c0 / sqrt(c (c + 2U)), c = c0 (1 + sqrt(1 + 4U/c0)) / 2,
/// deep water), for each cascade's middle wavelength and the bay's wind sea
/// (`waves::BAY_DIR`) against the flood's axis (`climate::current_axis_deg`).
/// Against the ebb the short waves steepen toward blocking and break: the gain
/// is held to `DOPPLER_MAX` (INFERRED: past it the crests spill). Cascade 0
/// (the swell) and 3 (the ripples, which the wind remakes in seconds) keep 1.
pub fn doppler_gain(current: f64) -> [f64; 4] {
    use eo_water::waves::{BAY_DIR, CUT_M};
    let a = eo_water::climate::current_axis_deg().to_radians();
    let flood = (a.sin(), -a.cos()); // compass to (x east, z south)
    let u = current * (flood.0 * BAY_DIR.cos() + flood.1 * BAY_DIR.sin());
    let mut g = [1.0; 4];
    for (c, gc) in g.iter_mut().enumerate().take(3).skip(1) {
        let lam = (CUT_M[c] * CUT_M[c + 1]).sqrt();
        let c0 = (9.81 * lam / std::f64::consts::TAU).sqrt();
        let q = 1.0 + 4.0 * u / c0;
        *gc = if q <= 0.0 {
            DOPPLER_MAX
        } else {
            let cc = 0.5 * c0 * (1.0 + q.sqrt());
            (c0 / (cc * (cc + 2.0 * u)).max(1e-6).sqrt()).clamp(0.5, DOPPLER_MAX)
        };
    }
    g
}

/// (current) the most the ebb steepens the wind sea (INFERRED: see `doppler_gain`).
pub const DOPPLER_MAX: f64 = 2.0;

impl WaterGpu {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, look: &crate::look::LookGpu, late_bgl: &wgpu::BindGroupLayout, init: &WaterInit) -> Self {
        let d = device;
        let fld = &init.data.field;
        let mut bytes = 0u64;
        let buffer = |label: &str, size: u64, usage: wgpu::BufferUsages| d.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size.max(16).next_multiple_of(16), usage, mapped_at_creation: false });
        let uniform_buf = buffer("water-uniform", std::mem::size_of::<WaterUniform>() as u64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
        // one parameter slot per FFT step (dynamic offsets).
        let fft_buf = buffer("water-fft-params", STEP_STRIDE * MAX_STEPS as u64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
        let texels = (N * N * CASCADES) as u64;
        // h0 is built on the GPU from the sea state's plan and these draws.
        let h0_buf = buffer("water-h0", texels * 16, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC);
        let gauss_buf = buffer("water-gauss", texels * 16, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
        queue.write_buffer(&gauss_buf, 0, bytemuck::cast_slice(&eo_water::waves::gaussians()));
        let spec_buf_params = buffer("water-spectrum-params", std::mem::size_of::<SpecParams>() as u64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
        let spec_buf = buffer("water-spectrum", texels * 32, wgpu::BufferUsages::STORAGE);
        let foam_bufs = [0, 1].map(|i| buffer(if i == 0 { "water-foam-a" } else { "water-foam-b" }, texels * 8, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST));
        bytes += texels * (16 + 16 + 32 + 8 + 8);
        let layered = |label: &str, mips: u32| {
            d.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: N, height: N, depth_or_array_layers: CASCADES },
                mip_level_count: mips,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let disp = layered("water-displacement", 1);
        let deriv = layered("water-derivatives", DERIV_MIPS);
        bytes += texels * 8 * 2;
        let arr = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        let (disp_view, deriv_view) = (arr(&disp), arr(&deriv));
        let mip_view = |m: u32| deriv.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), base_mip_level: m, mip_level_count: Some(1), ..Default::default() });
        let deriv_store = mip_view(0);

        // FFT pipelines.
        let fft_module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("water_fft.wgsl"), source: wgpu::ShaderSource::Wgsl(include_str!("water_fft.wgsl").into()) });
        let cs = wgpu::ShaderStages::COMPUTE;
        let entry = |binding, visibility, ty| wgpu::BindGroupLayoutEntry { binding, visibility, ty, count: None };
        let ubuf = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None };
        let sbuf = |read_only| wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None };
        let store = wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: wgpu::TextureFormat::Rgba16Float, view_dimension: wgpu::TextureViewDimension::D2Array };
        // the step's parameters at a dynamic offset; the foam ping-pong (read 3, write 6).
        let step_ubuf = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<FftParams>() as u64) };
        let fft_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("water-fft-bgl"),
            entries: &[entry(0, cs, step_ubuf), entry(1, cs, sbuf(true)), entry(2, cs, sbuf(false)), entry(3, cs, sbuf(true)), entry(4, cs, store), entry(5, cs, store), entry(6, cs, sbuf(false))],
        });
        let fft_bg = [0usize, 1].map(|side| {
            d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("water-fft-bg"),
                layout: &fft_bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &fft_buf, offset: 0, size: wgpu::BufferSize::new(std::mem::size_of::<FftParams>() as u64) }) },
                    wgpu::BindGroupEntry { binding: 1, resource: h0_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: spec_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: foam_bufs[side].as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&disp_view) },
                    wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&deriv_store) },
                    wgpu::BindGroupEntry { binding: 6, resource: foam_bufs[1 - side].as_entire_binding() },
                ],
            })
        });
        let fft_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("water-fft"), bind_group_layouts: &[Some(&fft_bgl)], immediate_size: 0 });
        let compute = |module: &wgpu::ShaderModule, layout: &wgpu::PipelineLayout, entry: &str| {
            d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(entry), layout: Some(layout), module, entry_point: Some(entry), compilation_options: Default::default(), cache: None })
        };
        let fft_rows = compute(&fft_module, &fft_layout, "fft_rows");
        let fft_cols = compute(&fft_module, &fft_layout, "fft_cols");
        // the spectrum's build (when the sea's plan changes).
        let spec_module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("water_spectrum.wgsl"), source: wgpu::ShaderSource::Wgsl(include_str!("water_spectrum.wgsl").into()) });
        let spec_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("water-spectrum-bgl"), entries: &[entry(0, cs, ubuf), entry(1, cs, sbuf(true)), entry(2, cs, sbuf(false))] });
        let spec_bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("water-spectrum-bg"),
            layout: &spec_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: spec_buf_params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: gauss_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: h0_buf.as_entire_binding() },
            ],
        });
        let spec_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("water-spectrum"), bind_group_layouts: &[Some(&spec_bgl)], immediate_size: 0 });
        let spec_init = compute(&spec_module, &spec_layout, "spectrum_init");

        // The shore field (RG16F, filtered) and its max pyramid (R32F mips, loaded).
        let field_tex = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("water-shore-field"),
            size: wgpu::Extent3d { width: fld.width as u32, height: fld.height as u32, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rg16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &field_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytemuck::cast_slice(&fld.half),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(fld.width as u32 * 4), rows_per_image: Some(fld.height as u32) },
            wgpu::Extent3d { width: fld.width as u32, height: fld.height as u32, depth_or_array_layers: 1 },
        );
        bytes += (fld.width * fld.height * 4) as u64;
        let pyr = fld.max_pyramid();
        let land_tex = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("water-land-max"),
            size: wgpu::Extent3d { width: fld.width as u32, height: fld.height as u32, depth_or_array_layers: 1 },
            mip_level_count: pyr.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (m, (w, h, v)) in pyr.iter().enumerate() {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &land_tex, mip_level: m as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                bytemuck::cast_slice(v),
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(*w as u32 * 4), rows_per_image: Some(*h as u32) },
                wgpu::Extent3d { width: *w as u32, height: *h as u32, depth_or_array_layers: 1 },
            );
            bytes += (*w * *h * 4) as u64;
        }
        let field_view = field_tex.create_view(&Default::default());
        let land_view = land_tex.create_view(&Default::default());
        // The sea detail noise (gusts, slicks, windrows), RGBA16F, repeat-sampled.
        let detail_size = 256u32;
        let noise: Vec<u16> = eo_water::surface::sea_detail_noise(detail_size as usize).iter().map(|v| eo_water::field::to_half_three(*v as f64)).collect();
        let detail_tex = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("water-sea-detail"),
            size: wgpu::Extent3d { width: detail_size, height: detail_size, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &detail_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytemuck::cast_slice(&noise),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(detail_size * 8), rows_per_image: Some(detail_size) },
            wgpu::Extent3d { width: detail_size, height: detail_size, depth_or_array_layers: 1 },
        );
        bytes += (detail_size * detail_size * 8) as u64;
        let detail_view = detail_tex.create_view(&Default::default());
        // (iteration 2, exposure) the sea's exposure (eo-water exposure.rs), RGBA8,
        // clamp-sampled; a 1 x 1 stand-in without one (the shader then keeps the ramp)
        // the parts the command line keeps, less those the quality tier leaves out (`LookOptions::water_parts_off`).
        let gg2 = eo_water::switches::on() & !look.options.water_parts_off;
        let expo = eo_water::exposure::installed().filter(|_| gg2 & eo_water::switches::EXPOSURE != 0);
        // (beach profiles) each beach's slope in A, as 55 x the slope / 10 (water.wgsl `w_surf_k`)
        let slopes = eo_water::beaches::installed().filter(|_| gg2 & eo_water::switches::BEACH_PROFILES != 0);
        let (ew, eh, mut etex): (u32, u32, Vec<[u8; 4]>) = match (expo, slopes) {
            (Some(e), _) => (e.width as u32, e.height as u32, e.texels.clone()),
            (None, Some(_)) => {
                let d = eo_water::exposure::DOMAIN;
                let (w, h) = (((d[2] - d[0]) / eo_water::exposure::CELL_M).round() as u32, ((d[3] - d[1]) / eo_water::exposure::CELL_M).round() as u32);
                (w, h, vec![[0, 128, 255, 255]; (w * h) as usize])
            }
            (None, None) => (1, 1, vec![[0, 128, 255, 255]]),
        };
        let (eorigin, ecell) = (expo.map_or([eo_water::exposure::DOMAIN[0], eo_water::exposure::DOMAIN[1]], |e| e.origin), expo.map_or(eo_water::exposure::CELL_M, |e| e.cell_m));
        if let Some(v) = slopes {
            let fallback = eo_water::field::BEACH_FORESHORE_SLOPE;
            for j in 0..eh as usize {
                for i in 0..ew as usize {
                    let (x, z) = (eorigin[0] + (i as f64 + 0.5) * ecell, eorigin[1] + (j as f64 + 0.5) * ecell);
                    let k = eo_water::beaches::slope_at(v, x, z, fallback) / fallback;
                    etex[j * ew as usize + i][3] = ((k / 10.0).clamp(0.0, 1.0) * 255.0).round() as u8;
                }
            }
        }
        let expo_tex = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("water-exposure"),
            size: wgpu::Extent3d { width: ew, height: eh, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &expo_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytemuck::cast_slice(&etex),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(ew * 4), rows_per_image: Some(eh) },
            wgpu::Extent3d { width: ew, height: eh, depth_or_array_layers: 1 },
        );
        bytes += (ew * eh * 4) as u64;
        let expo_view = expo_tex.create_view(&Default::default());
        let expo_on = expo.is_some();
        let slopes_on = slopes.is_some();
        let expo_frame = if expo_on || slopes_on { [eorigin[0] as f32, eorigin[1] as f32, (1.0 / (ew as f64 * ecell)) as f32, (1.0 / (eh as f64 * ecell)) as f32] } else { [0.0; 4] };
        // (iteration 3) the extra field (eo-water extra.rs), RGBA16F: none yet, so a 1 x 1
        // zero texel and the identity frame; bound to every pipeline of group 0, read by none.
        let extra = eo_water::extra::field().filter(|f| f.width > 0 && f.height > 0 && f.texels.len() == f.width * f.height);
        let (xw, xh, xtex): (u32, u32, &[[u16; 4]]) = match extra {
            Some(f) => (f.width as u32, f.height as u32, &f.texels),
            None => (1, 1, &[[0u16; 4]]),
        };
        let extra_tex = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("water-extra"),
            size: wgpu::Extent3d { width: xw, height: xh, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &extra_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytemuck::cast_slice(xtex),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(xw * 8), rows_per_image: Some(xh) },
            wgpu::Extent3d { width: xw, height: xh, depth_or_array_layers: 1 },
        );
        bytes += (xw * xh * 8) as u64;
        let extra_view = extra_tex.create_view(&Default::default());
        let extra_frame = extra.map_or(eo_water::extra::IDENTITY_FRAME, |f| f.frame()).map(|v| v as f32);

        // The grid: one shared index list and a compact stream of visible patch IDs.
        let idx = eo_water::grid::patch_indices();
        let index_buf = buffer("water-patch-indices", (idx.len() * 4) as u64, wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST);
        queue.write_buffer(&index_buf, 0, bytemuck::cast_slice(&idx));
        let index_count = idx.len() as u32;
        // the patches drawn this frame (one u32 per patch, the vertex shader's
        // `w_patches`); the counters and the one indirect draw after them.
        let draw_buf = buffer("water-patch-list", PATCH_COUNT as u64 * 4, wgpu::BufferUsages::STORAGE);
        let count_buf = buffer("water-counts", COUNT_BUF_BYTES, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST);
        bytes += idx.len() as u64 * 4 + PATCH_COUNT as u64 * 4 + COUNT_BUF_BYTES;

        // Patch cull: with the city's pyramid (group 1) and without.
        let tex_unfiltered = wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false };
        let cull_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("water-cull-bgl"),
            entries: &[entry(0, cs, ubuf), entry(1, cs, sbuf(false)), entry(2, cs, sbuf(false)), entry(3, cs, tex_unfiltered)],
        });
        let cull_bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("water-cull-bg"),
            layout: &cull_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: draw_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: count_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&land_view) },
            ],
        });
        let common = common_source(look.options.cove_detail, ShaderBits { features: look.options.water_features, parts: gg2, debug: debug_mode(), optics: look.options.water_optics });
        let plain_src = format!("{common}\n{}\nfn patch_occluded(mn: vec3<f32>, mx: vec3<f32>) -> bool {{ return false; }}\n", include_str!("water_cull.wgsl"));
        let hzb_src = format!(
            "{common}\n{}\nstruct HzbInfo {{ screen: vec4<f32> }};\n@group(1) @binding(0) var hzb: texture_2d<f32>;\n@group(1) @binding(1) var<uniform> hzb_info: HzbInfo;\n{}\nfn patch_occluded(mn: vec3<f32>, mx: vec3<f32>) -> bool {{ return hzb_occluded(mn, mx, wu.view_proj, hzb_info.screen.xy); }}\n",
            include_str!("water_cull.wgsl"),
            crate::occlusion::OCCLUSION_WGSL
        );
        let plain_module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("water_cull.wgsl (plain)"), source: wgpu::ShaderSource::Wgsl(plain_src.into()) });
        let hzb_module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("water_cull.wgsl (hzb)"), source: wgpu::ShaderSource::Wgsl(hzb_src.into()) });
        let plain_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("water-cull"), bind_group_layouts: &[Some(&cull_bgl)], immediate_size: 0 });
        let hzb_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("water-cull-hzb"), bind_group_layouts: &[Some(&cull_bgl), Some(late_bgl)], immediate_size: 0 });
        let cull_plain = compute(&plain_module, &plain_layout, "water_cull");
        let cull_hzb = compute(&hzb_module, &hzb_layout, "water_cull");

        // The water pass.
        let fs = wgpu::ShaderStages::FRAGMENT;
        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let tex = |sample_type, view_dimension| wgpu::BindingType::Texture { sample_type, view_dimension, multisampled: false };
        let filt = wgpu::TextureSampleType::Float { filterable: true };
        let mut draw_entries = vec![
                entry(0, vf, ubuf),
                entry(1, vf, tex(filt, wgpu::TextureViewDimension::D2)),
                entry(2, vf, tex(filt, wgpu::TextureViewDimension::D2Array)),
                entry(3, vf, tex(filt, wgpu::TextureViewDimension::D2Array)), // the vertex reads the far slope
                entry(4, vf, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                entry(5, vf, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                entry(6, fs, tex(wgpu::TextureSampleType::Depth, wgpu::TextureViewDimension::D2)),
                entry(7, fs, tex(filt, wgpu::TextureViewDimension::D2)),
                entry(8, fs, tex(filt, wgpu::TextureViewDimension::D2Array)),
                entry(9, fs, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                entry(10, fs, tex(filt, wgpu::TextureViewDimension::D2)),
                // the sky slice's atmosphere tables (sky view, transmittance)
                entry(11, vf, tex(filt, wgpu::TextureViewDimension::D2)),
                entry(12, fs, tex(filt, wgpu::TextureViewDimension::D2)),
                // the patch list the cull wrote (the vertex shader's `w_patches`)
                entry(13, vf, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }),
                // (ppr) the projected reflection's sources (water_ppr.wgsl)
                entry(14, fs, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }),
                // (iteration 2) the exposure field
                entry(15, vf, tex(filt, wgpu::TextureViewDimension::D2)),
                // (iteration 3) the extra field and its sampler (eo-water extra.rs; read by nothing yet)
                entry(18, vf, tex(filt, wgpu::TextureViewDimension::D2)),
                entry(19, vf, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                // (`--volumetric-clouds`) hemispherical off-screen cloud reflection dome
                entry(21, fs, tex(filt, wgpu::TextureViewDimension::D2)),
        ];
        if look.options.water_optics {
            // (submerged camera) the camera look's table and sampler (look_camera.wgsl), so a
            // submerged frame finishes through the same transform as the post pass.
            draw_entries.push(entry(16, fs, tex(filt, wgpu::TextureViewDimension::D3)));
            draw_entries.push(entry(17, fs, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)));
        }
        let canopy_map = (look.options.reef_canopy_light && look.options.water_optics).then(|| {
            draw_entries.push(entry(20, fs, tex(filt, wgpu::TextureViewDimension::D2)));
            let side = crate::ocean_life::canopy::SIDE;
            let levels = if look.options.dive_daylight { crate::ocean_life::canopy::LEVELS } else { 1 };
            bytes += (0..levels).map(|level| u64::from((side >> level).pow(2) * 4)).sum::<u64>();
            d.create_texture(&wgpu::TextureDescriptor {
                label: Some("kelp projected blade area"), size: wgpu::Extent3d { width: side, height: side, depth_or_array_layers: 1 },
                mip_level_count: levels, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[],
            })
        });
        let canopy_view = canopy_map.as_ref().map(|t| t.create_view(&Default::default()));
        let draw_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("water-draw-bgl"), entries: &draw_entries });
        // (ppr) the projection pass: the uniform, the opaque depth, the target buffer.
        let cs = wgpu::ShaderStages::COMPUTE;
        let ppr_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("water-ppr-bgl"),
            entries: &[
                entry(0, cs, ubuf),
                entry(1, cs, tex(wgpu::TextureSampleType::Depth, wgpu::TextureViewDimension::D2)),
                entry(2, cs, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }),
            ],
        });
        let ppr_module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("water_ppr.wgsl"), source: wgpu::ShaderSource::Wgsl(format!("{common}\n{}", include_str!("water_ppr.wgsl")).into()) });
        let ppr_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("water-ppr"), bind_group_layouts: &[Some(&ppr_bgl)], immediate_size: 0 });
        let ppr_pipe = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("ppr_project"), layout: Some(&ppr_layout), module: &ppr_module, entry_point: Some("ppr_project"), compilation_options: Default::default(), cache: None });
        let draw_src = format!("{common}\n{}\n{}\n{}\n{}\n{}", crate::look::look_common_source(), include_str!("sky_light_core.wgsl"), crate::look::LOOK_EXT_WGSL, crate::look::sf_select(include_str!("water.wgsl")), include_str!("water_surf.wgsl")) + include_str!("water_spray.wgsl") + "\n" + crate::far_shadow::WGSL + "\n" + &crate::probe_grid::wgsl(true); // + the spray, the cloud shadow and the fog depth; + group 1 bindings 12-18; + the far sun shadow map's lookup and the probe grid's (lighting)
        let draw_src = if look.options.water_optics {
            // + the dive exposure's switch, the display transforms (look.rs `DISPLAY_SHADER`, the camera look at group 0 bindings 16, 17)
            draw_src + &format!("\nconst UW_DEPTH_EXPOSURE: bool = {};\nconst UW_DIVE_DETAIL: bool = {};\nconst UW_COASTAL_HABITAT: bool = {};\nconst UW_DIVE_DAYLIGHT: bool = {};\n", look.options.dive_exposure, look.options.dive_detail, look.options.coastal_habitat, look.options.dive_daylight) + crate::look::DISPLAY_SHADER + &crate::look::camera_wgsl_at(0, 16, 17) + if look.options.reef_canopy_light { include_str!("ocean_life/canopy.wgsl") } else { "\nfn reef_transmission(p:vec3f, sun:vec3f) -> vec2f { return vec2f(1.); }\n" } + include_str!("underwater.wgsl")
        } else {
            draw_src
        };
        let draw_module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("water.wgsl"), source: wgpu::ShaderSource::Wgsl(draw_src.into()) });
        let draw_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("water-draw"), bind_group_layouts: &[Some(&draw_bgl), Some(look.main_bind_group_layout())], immediate_size: 0 });
        let make = |entry: &str, blend: Option<wgpu::BlendState>| {
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&draw_layout),
                vertex: wgpu::VertexState { module: &draw_module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: crate::DEPTH_FORMAT,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::Greater),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState { count: 1, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &draw_module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: crate::look::HDR_FORMAT, blend, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let submerged = look.options.water_optics.then(|| {
            let pipeline = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("submerged-camera"),
                layout: Some(&draw_layout),
                vertex: wgpu::VertexState { module: &draw_module, entry_point: Some("uw_vs"), compilation_options: Default::default(), buffers: &[] },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &draw_module, entry_point: Some("uw_fs"), compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: look.output_format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            });
            (pipeline, fld.clone())
        });
        let draw_copy = make("fs_copy", None);
        let premul = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
        };
        let draw_blend = make("fs_blend", Some(premul));
        // the spray off the breaking crests (water_spray.wgsl), after the sea in its pass.
        let spray = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("water-spray"),
            layout: Some(&draw_layout),
            vertex: wgpu::VertexState { module: &draw_module, entry_point: Some("vs_wspray"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: crate::DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState { count: 1, ..Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &draw_module,
                entry_point: Some("fs_wspray"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: crate::look::HDR_FORMAT, blend: Some(premul), write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = |label: &str, mode: wgpu::AddressMode| {
            d.create_sampler(&wgpu::SamplerDescriptor {
                label: Some(label),
                address_mode_u: mode,
                address_mode_v: mode,
                address_mode_w: mode,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                ..Default::default()
            })
        };
        let readbacks = (0..4)
            .map(|_| Readback {
                buf: d.create_buffer(&wgpu::BufferDescriptor { label: Some("water-count-readback"), size: COUNT_BYTES, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
                stage: std::sync::Arc::new(AtomicU8::new(0)),
                frame: 0,
            })
            .collect();
        bytes += uniform_buf.size() + fft_buf.size();
        Self {
            device: d.clone(),
            canopy_map, canopy_view,
            bytes,
            uniform_buf,
            fft_buf,
            fft_rows,
            fft_cols,
            fft_bg,
            foam_bufs,
            spec_buf: spec_buf_params,
            spec_init,
            spec_bg,
            h0_buf,
            disp_view,
            deriv_view,
            disp_tex: disp,
            field_view,
            detail_view,
            index_buf,
            index_count,
            draw_buf,
            count_buf,
            cull_plain,
            cull_hzb,
            cull_bg,
            draw_copy,
            draw_blend,
            submerged,
            fallback_dive: std::sync::atomic::AtomicBool::new(false),
            spray,
            draw_bgl,
            repeat: sampler("water-repeat", wgpu::AddressMode::Repeat),
            clamp: sampler("water-clamp", wgpu::AddressMode::ClampToEdge),
            probe_sampler: sampler("water-probe", wgpu::AddressMode::ClampToEdge),
            scene: Mutex::new(None),
            ppr: Mutex::new(None),
            ppr_bgl,
            ppr_pipe,
            ppr_bg: Mutex::new(None),
            draw_bg: Mutex::new(None),
            readbacks: Mutex::new(readbacks),
            state: Mutex::new(State {
                frame: 0,
                time: None,
                last_time: None,
                radius: eo_water::grid::MIN_RADIUS_M,
                offset: [0.0; 2],
                seen: None,
                fft: false,
                copy: false,
                fft_ever: false,
                near_waterline: false,
                fogged: false,
                south_m: 0.0,
                active: false,
                always: false,
                stats: Default::default(),
                slot: None,
                sea_in: SeaInput::Weather { hour: 16.0, wind: [0.0, 0.0], severity: 0.0 },
                sea_key: None,
                sea_day: f64::NAN,
                plan: None,
                init: false,
                reset_foam: false,
                steps: 0,
                foam_flip: 0,
                plan_moved: false,
                warm_pending: false,
                plan_gpu: None,
                plan_job: None,
                spray: false,
                foam_over: if look.options.water_features & FEAT_FOAM != 0 { eo_water::waves::FOAM_OVER_TOTAL } else { eo_water::waves::FOAM_OVER_ACTIVE },
            }),
            extent: init.extent,
            field_frame: [fld.origin[0] as f32, fld.origin[1] as f32, fld.size[0] as f32, fld.size[1] as f32],
            on: std::sync::atomic::AtomicBool::new(true),
            features: look.options.water_features,
            expo_view,
            expo_frame,
            expo_on,
            extra_view,
            extra_frame,
            slopes_on,
            gg2,
        }
    }

    /// On / off (`--no-water` keeps the flat sea instead).
    pub fn set_on(&self, on: bool) {
        self.on.store(on, Ordering::Relaxed);
    }
    pub fn on(&self) -> bool {
        self.on.load(Ordering::Relaxed)
    }
    /// the pass may draw without multi-draw-indirect-count (`FEAT_INSTANCED`).
    /// What the sea's shaders compile in (`common_source`), for the surf
    /// strip's, which shares them.
    pub(crate) fn shader_bits(&self) -> ShaderBits {
        ShaderBits { features: self.features, parts: self.gg2, debug: debug_mode(), optics: self.submerged.is_some() }
    }

    pub fn instanced(&self) -> bool {
        self.features & FEAT_INSTANCED != 0
    }

    /// The sea's reach past the terrain grid to the south (m; stage L5: 0 by
    /// default, `CINEMATIC_SOUTH_REACH_M` under the cinematic look).
    pub fn set_south_reach(&self, m: f32) {
        self.state.lock().unwrap().south_m = m;
    }

    /// The water clock (seconds), the time the spectrum and the collar run on.
    pub fn set_time(&self, t: f64) {
        self.state.lock().unwrap().time = Some(t);
    }

    /// Run the FFT and take the copy every frame, whatever the gate says
    /// (a still frame must not depend on the frames before it).
    /// with it on, a change of the clock or of the sea rebuilds the
    /// whitecap foam from nothing over `WARM_S` in one frame.
    /// The clock the waves are drawn at, given the world's (a fixed test time wins).
    pub fn clock(&self, world_time: f64) -> f64 {
        self.state.lock().unwrap().time.unwrap_or(world_time)
    }

    pub fn set_always(&self, always: bool) {
        self.state.lock().unwrap().always = always;
    }

    /// the sea's inputs this frame (the renderer passes the hour and the
    /// weather in force each frame; a fixed state, from a test or a tool, holds
    /// until `set_sea(None)`).
    pub fn set_sea(&self, input: Option<SeaInput>) {
        let mut st = self.state.lock().unwrap();
        match input {
            Some(i @ SeaInput::Weather { .. }) => {
                if let Some(f) = sea_override() {
                    st.sea_in = SeaInput::Fixed(f);
                } else if !matches!(st.sea_in, SeaInput::Fixed(_)) {
                    st.sea_in = i;
                }
            }
            Some(i) => st.sea_in = i,
            None => st.sea_in = SeaInput::Weather { hour: 16.0, wind: [0.0, 0.0], severity: 0.0 },
        }
    }

    /// the swell's significant height in force (m), without cloning the plan
    /// (the surf strip reads it every frame).
    pub fn swell_hs(&self) -> Option<f32> {
        self.state.lock().unwrap().plan.as_ref().map(|p| p.state.swell.hs as f32)
    }

    /// the plan in force (its sea state, cascades and whitecap thresholds).
    pub fn plan(&self) -> Option<eo_water::waves::Plan> {
        self.state.lock().unwrap().plan.clone()
    }

    /// the plan for the sea's inputs, rebuilt when they move. (off
    /// the render thread once a plan is in force, outside captures: the old
    /// plan stays until the new one is ready; nothing per frame but a key
    /// compare.)
    fn refresh_plan(st: &mut State) {
        use eo_water::waves;
        if st.plan_job.as_ref().is_some_and(|j| j.is_finished()) {
            if let Ok((key, plan)) = st.plan_job.take().unwrap().join() {
                Self::install_plan(st, key, plan);
            }
        }
        let (state, key) = match st.sea_in {
            SeaInput::Weather { hour, wind, severity } => {
                let key = waves::quantise(hour, wind, severity);
                // (iteration 2) the station's wind by the day of the year
                let day = eo_look::sun::day_of_year().round();
                if st.sea_key == Some(key) && st.plan.is_some() && st.sea_day == day {
                    return;
                }
                st.sea_day = day;
                let (h, w, sev) = waves::unquantise(key);
                let on = eo_water::switches::on();
                (waves::sea_state_on(day, h, w, sev, on & eo_water::switches::EXPOSURE != 0, on & eo_water::switches::STREAK_CAPS != 0), key)
            }
            SeaInput::Fixed(s) => {
                if st.plan.as_ref().is_some_and(|p| p.state == s) {
                    return;
                }
                (s, [i64::MIN; 4])
            }
        };
        if st.plan.is_none() || st.always || matches!(st.sea_in, SeaInput::Fixed(_)) {
            st.plan_job = None;
            Self::install_plan(st, key, waves::Plan::with_foam(&state, st.foam_over));
        } else if st.plan_job.is_none() {
            let over = st.foam_over;
            st.plan_job = Some(std::thread::spawn(move || (key, waves::Plan::with_foam(&state, over))));
        }
    }

    fn install_plan(st: &mut State, key: [i64; 4], plan: eo_water::waves::Plan) {
        if std::env::var("EO_WATER_LOG").is_ok_and(|v| v == "1") {
            let s = &plan.state;
            eprintln!("water: sea plan key {key:?} day {}: bay U10 {:.2} m/s Hs {:.2} m, Pacific U10 {:.2} m/s, swell Hs {:.2} m, amp bay {:?}", st.sea_day, s.bay.u10, s.bay.hs(), s.pacific.u10, s.swell.hs, plan.amp_bay);
        }
        st.sea_key = Some(key);
        st.plan_gpu = Some(PlanGpu::of(&plan));
        st.plan = Some(plan);
        st.init = true;
        st.plan_moved = true;
    }

    /// one FFT step's parameters.
    fn step_params(plan: &eo_water::waves::Plan, time: f64, dt: f64) -> FftParams {
        use eo_water::waves;
        let f4 = |v: [f64; 4]| v.map(|x| x as f32);
        let st = &plan.state;
        let short_dir = plan.systems[2].dir;
        let short_u = st.bay.u10.max(0.7 * st.pacific.u10);
        FftParams {
            time: time as f32,
            dt: dt as f32,
            choppiness: plan.chop as f32,
            depth: waves::DEPTH_M as f32,
            sizes: f4(waves::SIZES),
            amp_pac: f4(plan.amp_pac),
            amp_bay: f4(plan.amp_bay),
            thr_pac: f4(plan.thr[0]),
            thr_bay: f4(plan.thr[1]),
            soft_pac: f4(plan.soft[0]),
            soft_bay: f4(plan.soft[1]),
            foam: [(1.0 / plan.foam_tau[0]) as f32, (1.0 / plan.foam_tau[1]) as f32, waves::FOAM_RATE as f32, waves::FOAM_SPREAD as f32],
            wind: [st.pacific.dir.cos() as f32, st.pacific.dir.sin() as f32, short_dir.cos() as f32, short_dir.sin() as f32],
            drift: [(waves::FOAM_DRIFT * st.pacific.u10) as f32, (waves::FOAM_DRIFT * short_u) as f32, 0.0, 0.0],
        }
    }

    pub fn stats(&self) -> WaterStats {
        self.state.lock().unwrap().stats
    }

    /// The counters of the last frame encoded: patches drawn, in the frustum, land, occluded.
    pub fn counter_buffer(&self) -> &wgpu::Buffer {
        &self.count_buf
    }

    /// The FFT's displacement layers (dx, height, dz, foam per cascade; tests).
    pub fn displacement_texture(&self) -> &wgpu::Texture {
        &self.disp_tex
    }

    /// the FFT's derivative layers (dh/dx, dh/dz, Pacific foam, bay foam per cascade; tests).
    pub fn derivative_texture(&self) -> wgpu::Texture {
        self.deriv_view.texture().clone()
    }

    /// the spectrum the GPU built (h0 per texel; tests).
    pub fn h0_buffer(&self) -> &wgpu::Buffer {
        &self.h0_buf
    }

    /// the water pass's group-0 layout, for the surf strip's pipeline (surf.rs).
    pub fn surf_layout(&self) -> &wgpu::BindGroupLayout {
        &self.draw_bgl
    }

    /// this frame's group-0 bind group (made by `encode_draw`), for the surf strip's pass.
    pub fn surf_bind_group(&self, probe_front: usize) -> Option<wgpu::BindGroup> {
        self.draw_bg.lock().unwrap().as_ref().map(|c| c.2[probe_front].clone())
    }

    // On a device using fixed draw slots, keep the normal land renderer until
    // the dive camera reaches water. Both media at the waterline then use
    // the same displaced surface and the same opaque frame copy.
    pub(crate) fn locate_fallback_dive(&self, cam: &Camera) {
        let wet = self.submerged.as_ref().is_some_and(|(_, field)|
            field.signed_at(cam.eye.x as f64, cam.eye.z as f64) > MASK_M as f64);
        self.fallback_dive.store(wet, Ordering::Relaxed);
    }

    /// The sky and skyline that a sheltered water surface reflects are in air.
    /// Share the cove's above-water cube while swimming. Nearby receivers and
    /// their reflections still come from this frame's exact screen positions.
    pub(crate) fn cove_reflection_eye(&self, eye: glam::Vec3) -> glam::Vec3 {
        let sea = eo_water::SEA_LEVEL_Y as f32;
        let scale = eo_data::groundscale::scale() as f32;
        let x = eye.x / scale;
        let z = eye.z / scale;
        let edge = ((x + 625.0) / 48.0).min((-150.0 - x) / 48.0)
            .min((z + 3620.0) / 64.0).min((-3110.0 - z) / 40.0);
        let w = edge.min((sea + 8.0 - eye.y) / 5.0).clamp(0.0, 1.0);
        if w == 0.0 || !self.submerged.as_ref().is_some_and(|(_, f)| f.signed_at(eye.x as f64, eye.z as f64) > 0.0) {
            return eye;
        }
        let anchor = glam::Vec3::new(-365.0 * scale, sea + 1.7, -3250.0 * scale);
        eye.lerp(anchor, w * w * (3.0 - 2.0 * w))
    }

    pub(crate) fn fallback_dive(&self) -> bool {
        self.fallback_dive.load(Ordering::Relaxed)
    }

    pub(crate) fn draw_submerged(&self, pass: &mut wgpu::RenderPass<'_>, look: &crate::look::LookGpu, front: usize) {
        if let Some((pipeline, _)) = &self.submerged
            && self.state.lock().unwrap().near_waterline
            && let Some(bg) = self.surf_bind_group(front)
        {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bg, &[]);
            look.bind_main(pass);
            pass.draw(0..3, 0..1);
        }
    }

    /// the sea draws nothing this frame (all of it past the fog), nor does the strip.
    pub fn surf_fogged(&self) -> bool {
        self.state.lock().unwrap().fogged
    }

    /// Bytes of the frame copy, once it exists.
    pub fn copy_bytes(&self) -> u64 {
        self.scene.lock().unwrap().as_ref().map(|(w, h, _, _)| *w as u64 * *h as u64 * 8).unwrap_or(0)
    }

    /// Collect finished readbacks and start mapping last frame's.
    fn poll_readbacks(&self, st: &mut State) {
        let mut rbs = self.readbacks.lock().unwrap();
        for rb in rbs.iter_mut() {
            match rb.stage.load(Ordering::Acquire) {
                1 => {
                    // Recorded in a frame already submitted (update runs before this frame's encode).
                    rb.stage.store(2, Ordering::Release);
                    let flag = rb.stage.clone();
                    rb.buf.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                        flag.store(if r.is_ok() { 3 } else { 0 }, Ordering::Release);
                    });
                }
                3 => {
                    let counts: [u32; 8] = match rb.buf.slice(..).get_mapped_range() {
                        Ok(v) => bytemuck::pod_read_unaligned(&v[..COUNT_BYTES as usize]),
                        Err(_) => [0; 8],
                    };
                    rb.buf.unmap();
                    rb.stage.store(0, Ordering::Release);
                    st.stats.readbacks += 1;
                    st.stats.last_counts = [counts[0], counts[1], counts[2], counts[3]];
                    st.stats.last_nearest_m = counts[4];
                    if counts[0] > 0 && (counts[4] as f32) < GATE_NEAR_M {
                        st.seen = Some(st.seen.map_or(rb.frame, |s| s.max(rb.frame)));
                    }
                }
                _ => {}
            }
        }
        st.slot = rbs.iter().position(|rb| rb.stage.load(Ordering::Acquire) == 0);
    }

    /// Per frame, before encoding: the grid, the clock, this frame's decisions
    /// (FFT and copy while water was seen lately; nothing drawn when all water
    /// is past `fog_far`, the look's range fog end), the uniforms.
    pub fn update(&self, queue: &wgpu::Queue, cam: &Camera, probe: Option<glam::Vec3>, active: bool, fog_far: f32) {
        self.update_at_time(queue, cam, probe, active, fog_far, 0.0, None);
    }

    pub(crate) fn update_at_time(&self, queue: &wgpu::Queue, cam: &Camera, probe: Option<glam::Vec3>, active: bool, fog_far: f32, world_time: f64, mapped_ocean: Option<crate::ocean_life::HabitatView>) {
        let mut st = self.state.lock().unwrap();
        st.active = active;
        if !active {
            return;
        }
        let time = st.time.unwrap_or(world_time);
        st.frame += 1;
        self.poll_readbacks(&mut st);
        let sea = eo_water::SEA_LEVEL_Y as f32;
        let height = (cam.eye.y - sea).max(0.0) as f64;
        st.radius = eo_water::grid::radius_for(height, cam.far as f64, st.radius);
        let snap = eo_water::grid::snap_for(st.radius);
        st.offset = eo_water::grid::offset_for(cam.eye.x as f64, cam.eye.z as f64, snap);
        // FFT and copy while water was drawn within the hold, or before any
        // readback has answered (the first frames).
        let frame = st.frame;
        // Looking down can cull every surface patch. A submerged camera still
        // needs the current wave field and the opaque scene behind the water.
        st.near_waterline = self.submerged.as_ref().is_some_and(|(_, field)| {
            cam.eye.y <= sea + BOX_M + cam.near
                && cam.eye.x >= self.extent[0] - 60000.0 && cam.eye.x <= self.extent[2] + 60000.0
                && cam.eye.z >= self.extent[1] - 60000.0 && cam.eye.z <= self.extent[3] + if mapped_ocean.is_some() {st.south_m} else {0.0}
                && mapped_ocean.map(|v| v.wet).unwrap_or_else(||field.signed_at(cam.eye.x as f64, cam.eye.z as f64) > 0.0)
        });
        let recent = st.always || st.near_waterline
            || match st.seen {
                Some(s) => frame.saturating_sub(s) <= SEEN_HOLD_FRAMES,
                None => st.stats.readbacks == 0,
            };
        // all water past the fog's far end: draw nothing (`sea_fogged`).
        st.fogged = sea_fogged(cam, fog_far);
        if st.fft != (recent && !st.fogged) {
            crate::hitch::mark(crate::hitch::Mark::WaterFftFlip, 1);
        }
        st.fft = recent && !st.fogged;
        st.copy = st.fft;
        st.fft_ever |= st.fft;
        let dt = st.last_time.map(|l| (time - l).clamp(0.0, 0.1)).unwrap_or(1.0 / 60.0);
        // a clock jump (or none yet); the plan for this frame's sea.
        let jumped = st.last_time.is_none_or(|l| time < l || time - l > 0.25);
        Self::refresh_plan(&mut st);
        if st.fft {
            st.last_time = Some(time);
        }
        let vp = cam.view_proj();
        let mut planes = frustum_planes(vp).map(|p| p.to_array());
        // no far plane for the sea (reverse Z: plane 4 is the far one): from
        // the air it runs on past the camera's far plane into the haze; the
        // vertex shader holds such points just inside the depth range.
        planes[4] = [0.0, 0.0, 0.0, 1.0];
        let s = &eo_water::waves::SIZES; // the sea state's cascades
        let pg = st.plan_gpu.expect("waves: a plan");
        // (iteration 2, current) the tidal current at the Gate for the sea's hour and the day
        let current = match st.sea_in {
            SeaInput::Weather { hour, .. } if self.gg2 & eo_water::switches::CURRENT != 0 => eo_water::climate::tidal_current(eo_look::sun::day_of_year(), hour),
            _ => 0.0,
        };
        let u = WaterUniform {
            view_proj: vp.to_cols_array_2d(),
            inv_view_proj: vp.inverse().to_cols_array_2d(),
            cam: [cam.eye.x, cam.eye.y, cam.eye.z, time as f32],
            fwd: { let f = cam.forward(); [f.x, f.y, f.z, st.radius as f32] },
            grid: [st.offset[0] as f32, st.offset[1] as f32, sea, BOX_M],
            field: self.field_frame,
            extent: self.extent,
            // x: EO_WATER_DEBUG (1: foam / column / shore distance; 2: beach / surf / collar), 0 off
            screen: [debug_mode(), crate::surf::hands_over(), self.features as f32, self.submerged.is_some() as u32 as f32], // y 1 while the surf strip draws (water_surf.wgsl `surf_cover`); z the shading features (`FEAT_*`); w 1 with the submerged camera's wave optics (`--water-optics`)
            flags: [st.copy as u32 as f32, probe.is_some() as u32 as f32, MASK_M, st.fft_ever as u32 as f32],
            sizes: [s[0] as f32, s[1] as f32, s[2] as f32, s[3] as f32],
            probe: { let p = probe.unwrap_or_default(); [p.x, p.y, p.z, BEACH_MASK_M] },
            bay: [BAY_SWELL, 1.0, 1.0, 1.0],
            planes,
            wv_bay: pg.wv[0],
            wv_pac: pg.wv[1],
            wv_sea: pg.wv[2],
            wv_misc: pg.wv[3],
            wv_mss: pg.wv[4],
            wv_expo: self.expo_frame,
            wv_gg2: [current as f32, self.gg2 as f32, self.expo_on as u32 as f32, self.slopes_on as u32 as f32],
            wv_dop: doppler_gain(current).map(|g| g as f32),
            wv_xn: { let n = eo_water::waves::EXPO_FETCH_NODES.map(|v| v as f32); [[n[0], n[1], n[2], n[3]], [n[4], n[5], n[6], n[7]]] },
            wv_gn: { let n = eo_water::waves::EXPO_WIND_NODES.map(|v| v as f32); [[n[0], n[1], n[2], n[3]], [n[4], n[5], n[6], n[7]]] },
            wv_xa: pg.xa,
            wv_extra: self.extra_frame,
            reach: [st.south_m, mapped_ocean.map_or(0., |v| f32::from(v.canopy_bands)), 0.0, 0.0],
        };
        queue.write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&u));
        queue.write_buffer(&self.count_buf, 0, bytemuck::cast_slice(&[0u32, 0, 0, 0, u32::MAX, 0, 0, 0, self.index_count, 0, 0, 0, 0, 0, 0, 0]));
        st.steps = 0;
        st.reset_foam = false;
        st.spray = spray_on() && st.fft_ever && !st.fogged && pg.wv[2][0].max(pg.wv[2][1]) > SPRAY_WIND_MS;
        if st.fft {
            // the spectrum when the plan moved; one step on the clock, or
            // (captures, after a clock jump or a new sea) the foam's warm-up:
            // MAX_STEPS steps over the WARM_S before the clock, from no foam.
            if st.init {
                queue.write_buffer(&self.spec_buf, 0, bytemuck::bytes_of(&pg.spec));
            }
            let warm = st.always && (jumped || st.plan_moved);
            st.plan_moved = false;
            st.warm_pending |= warm;
            let warm = st.warm_pending;
            let steps: Vec<(f64, f64)> = if warm {
                let h = WARM_S / MAX_STEPS as f64;
                (0..MAX_STEPS).map(|i| (time - WARM_S + (i + 1) as f64 * h, h)).collect()
            } else {
                Vec::new()
            };
            let steps = if steps.is_empty() { vec![(time, dt)] } else { steps };
            if steps.len() == 1 {
                let p = FftParams { time: time as f32, dt: dt as f32, ..pg.step };
                queue.write_buffer(&self.fft_buf, 0, bytemuck::bytes_of(&p));
            } else {
                let mut bytes = vec![0u8; STEP_STRIDE as usize * steps.len()];
                for (i, (t, h)) in steps.iter().enumerate() {
                    let p = FftParams { time: *t as f32, dt: *h as f32, ..pg.step };
                    let at = i * STEP_STRIDE as usize;
                    bytes[at..at + std::mem::size_of::<FftParams>()].copy_from_slice(bytemuck::bytes_of(&p));
                }
                queue.write_buffer(&self.fft_buf, 0, &bytes);
            }
            st.steps = steps.len();
            st.reset_foam = warm;
            st.stats.fft_frames += 1;
        }
        if st.copy {
            st.stats.copy_frames += 1;
        }
        if st.fogged {
            st.stats.fogged_frames += 1;
        }
        st.stats.frames += 1;
    }

    /// The patch cull with the city's pyramid (inside its compute pass).
    pub fn cull_hzb(&self, pass: &mut wgpu::ComputePass<'_>, hzb_bg: &wgpu::BindGroup) {
        pass.set_pipeline(&self.cull_hzb);
        pass.set_bind_group(0, &self.cull_bg, &[]);
        pass.set_bind_group(1, hzb_bg, &[]);
        pass.dispatch_workgroups(PATCH_COUNT.div_ceil(64), 1, 1);
    }

    /// The FFT (when water was seen lately) and, without the pyramid, the
    /// plain patch cull: one compute pass (timestamps `ts`).
    pub fn encode_compute(&self, enc: &mut wgpu::CommandEncoder, culled: bool, ts: Option<(&wgpu::QuerySet, u32)>) {
        // this frame's spectrum build, foam reset and FFT steps.
        let (fft, init, reset, steps, mut side) = {
            let mut st = self.state.lock().unwrap();
            let init = st.fft && st.init;
            if init {
                st.init = false;
            }
            let side = st.foam_flip;
            st.foam_flip = (side + st.steps) % 2;
            st.warm_pending = false;
            (st.fft, init, st.reset_foam, st.steps, side)
        };
        if reset {
            for b in &self.foam_bufs {
                enc.clear_buffer(b, 0, None);
            }
        }
        let tw = ts.map(|(q, b)| wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(b), end_of_pass_write_index: Some(b + 1) });
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("water fft + cull"), timestamp_writes: tw });
        if fft {
            if init {
                pass.set_pipeline(&self.spec_init);
                pass.set_bind_group(0, &self.spec_bg, &[]);
                pass.dispatch_workgroups((N * N * CASCADES).div_ceil(64), 1, 1);
            }
            for i in 0..steps {
                let offset = (i as u64 * STEP_STRIDE) as u32;
                pass.set_bind_group(0, &self.fft_bg[side], &[offset]);
                pass.set_pipeline(&self.fft_rows);
                pass.dispatch_workgroups(N * CASCADES, 1, 1);
                pass.set_pipeline(&self.fft_cols);
                pass.dispatch_workgroups(N * CASCADES, 1, 1);
                side = 1 - side;
            }
        }
        if !culled {
            pass.set_pipeline(&self.cull_plain);
            pass.set_bind_group(0, &self.cull_bg, &[]);
            pass.dispatch_workgroups(PATCH_COUNT.div_ceil(64), 1, 1);
        }
    }

    /// The frame copy (when water was seen lately) and the water pass; `then`
    /// draws more into the same pass after the water (the city's and the
    /// agents' glass: their pipelines test depth without writing it, so they
    /// share the pass's read-only depth).
    #[allow(clippy::too_many_arguments)]
    pub fn encode_draw(
        &self,
        enc: &mut wgpu::CommandEncoder,
        look: &crate::look::LookGpu,
        hdr: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        probe_views: [&wgpu::TextureView; 2], // both cubes (bind groups made once, a capture flips the index)
        probe_front: usize,
        ts: Option<(&wgpu::QuerySet, u32)>,
        ts_ppr: Option<(&wgpu::QuerySet, u32)>,
        then: impl FnOnce(&mut wgpu::RenderPass<'_>),
    ) {
        let ppr_tw = ts_ppr.map(|(q, b)| wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(b), end_of_pass_write_index: Some(b + 1) });
        let (copy, slot, fogged, spray) = {
            let st = self.state.lock().unwrap();
            (st.copy, st.slot, st.fogged, st.spray)
        };
        let ht = hdr.texture();
        let (w, h) = (ht.width(), ht.height());
        let scene_view = {
            let mut sc = self.scene.lock().unwrap();
            if sc.as_ref().is_none_or(|s| s.0 != w || s.1 != h) {
                let t = self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("water-frame-copy"),
                    size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: crate::look::HDR_FORMAT,
                    usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                let v = t.create_view(&Default::default());
                *sc = Some((w, h, t, v));
            }
            let s = sc.as_ref().unwrap();
            if copy {
                enc.copy_texture_to_texture(ht.as_image_copy(), s.2.as_image_copy(), wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 });
            }
            (s.2.clone(), s.3.clone())
        };
        // (ppr) the target buffer at the depth's size (16 bytes while the feature is off)
        let ppr_on = self.features & FEAT_PPR != 0;
        let (dw, dh) = (depth.texture().width(), depth.texture().height());
        let ppr_buf = {
            let mut pb = self.ppr.lock().unwrap();
            if pb.as_ref().is_none_or(|p| p.0 != dw || p.1 != dh) {
                let size = if ppr_on { dw as u64 * dh as u64 * 4 } else { 16 };
                let b = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("water-ppr"), size, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
                *pb = Some((dw, dh, b));
            }
            pb.as_ref().unwrap().2.clone()
        };
        if copy && ppr_on && !fogged {
            let pbg = {
                let mut cache = self.ppr_bg.lock().unwrap();
                let dt = depth.texture();
                if cache.as_ref().is_none_or(|(d, b, _)| d != dt || *b != ppr_buf) {
                    crate::hitch::mark(crate::hitch::Mark::BindGroupCreated, 2);
                    let g = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("water-ppr-bg"),
                        layout: &self.ppr_bgl,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: self.uniform_buf.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(depth) },
                            wgpu::BindGroupEntry { binding: 2, resource: ppr_buf.as_entire_binding() },
                        ],
                    });
                    *cache = Some((dt.clone(), ppr_buf.clone(), g));
                }
                cache.as_ref().unwrap().2.clone()
            };
            enc.clear_buffer(&ppr_buf, 0, None);
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("water-ppr"), timestamp_writes: ppr_tw });
            pass.set_pipeline(&self.ppr_pipe);
            pass.set_bind_group(0, &pbg, &[]);
            pass.dispatch_workgroups(dw.div_ceil(8), dh.div_ceil(8), 1);
        } else if ppr_tw.is_some() {
            enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("water-ppr (none)"), timestamp_writes: ppr_tw });
        }
        let bg = {
            let mut cache = self.draw_bg.lock().unwrap();
            let dt = depth.texture();
            let stale = cache.as_ref().is_none_or(|(d, s, _)| d != dt || *s != scene_view.0);
            if stale {
                crate::hitch::mark(crate::hitch::Mark::BindGroupCreated, 2);
                let g = probe_views.map(|probe_view| {
                    let mut entries = vec![
                        wgpu::BindGroupEntry { binding: 0, resource: self.uniform_buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&self.field_view) },
                        wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&self.disp_view) },
                        wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&self.deriv_view) },
                        wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&self.repeat) },
                        wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(&self.clamp) },
                        wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(depth) },
                        wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(&scene_view.1) },
                        wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(probe_view) },
                        wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::Sampler(&self.probe_sampler) },
                        wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(&self.detail_view) },
                        wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::TextureView(look.sky.atmo_tables().0) },
                        wgpu::BindGroupEntry { binding: 12, resource: wgpu::BindingResource::TextureView(look.sky.atmo_tables().1) },
                        wgpu::BindGroupEntry { binding: 13, resource: self.draw_buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 14, resource: ppr_buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 15, resource: wgpu::BindingResource::TextureView(&self.expo_view) },
                        wgpu::BindGroupEntry { binding: 18, resource: wgpu::BindingResource::TextureView(&self.extra_view) },
                        wgpu::BindGroupEntry { binding: 19, resource: wgpu::BindingResource::Sampler(&self.clamp) },
                        wgpu::BindGroupEntry { binding: 21, resource: wgpu::BindingResource::TextureView(look.sky.cloud_refl_view()) },
                    ];
                    if let Some(view) = &self.canopy_view {
                        entries.push(wgpu::BindGroupEntry { binding: 20, resource: wgpu::BindingResource::TextureView(view) });
                    }
                    if self.submerged.is_some() {
                        let (lut, samp) = look.camera_lut();
                        entries.push(wgpu::BindGroupEntry { binding: 16, resource: wgpu::BindingResource::TextureView(lut) });
                        entries.push(wgpu::BindGroupEntry { binding: 17, resource: wgpu::BindingResource::Sampler(samp) });
                    }
                    self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("water-draw-bg"), layout: &self.draw_bgl, entries: &entries })
                });
                *cache = Some((dt.clone(), scene_view.0.clone(), g)); // one per probe cube
            }
            cache.as_ref().unwrap().2[probe_front].clone()
        };
        {
            let tw = ts.map(|(q, b)| wgpu::RenderPassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(b), end_of_pass_write_index: Some(b + 1) });
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("water"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: hdr,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                // Read-only depth: tested against and sampled in the same pass.
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: depth, depth_ops: None, stencil_ops: None }),
                timestamp_writes: tw,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if !fogged {
                pass.set_pipeline(if copy { &self.draw_copy } else { &self.draw_blend });
                pass.set_bind_group(0, &bg, &[]);
                look.bind_main(&mut pass);
                pass.set_index_buffer(self.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed_indirect(&self.count_buf, DRAW_ARGS_OFFSET); // every patch the cull kept, one instance each
                if spray {
                    // the spray, same bind groups
                    pass.set_pipeline(&self.spray);
                    pass.draw(0..SPRAY_SLOTS * 6, 0..1);
                }
            }
            then(&mut pass);
        }
        // The counters of this frame, for the gate (mapped next frame, read when ready).
        if let Some(i) = slot {
            let frame = self.state.lock().unwrap().frame;
            let mut rbs = self.readbacks.lock().unwrap();
            if rbs[i].stage.load(Ordering::Acquire) == 0 {
                enc.copy_buffer_to_buffer(&self.count_buf, 0, &rbs[i].buf, 0, COUNT_BYTES);
                rbs[i].stage.store(1, Ordering::Release);
                rbs[i].frame = frame;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::{self, ShaderFeatures};

    #[test]
    fn water_draw_shader_validates_with_and_without_aaa_buildings() {
        use wgpu::naga;
        let common = common_source(false, ShaderBits { features: u32::MAX, parts: eo_water::switches::ALL, debug: 0.0, optics: false });
        for (label, features, expect_aaa) in [
            ("default_medium", ShaderFeatures::NONE, false),
            ("aaa_high", ShaderFeatures { aaa_buildings: u32::MAX, ..ShaderFeatures::ALL }, true),
        ] {
            let src = look::with_shader_features(features, || {
                format!("{common}\n{}\n{}\n{}\n{}\n{}", look::look_common_source(), include_str!("sky_light_core.wgsl"), look::LOOK_EXT_WGSL, look::sf_select(include_str!("water.wgsl")), include_str!("water_surf.wgsl"))
                    + include_str!("water_spray.wgsl") + "\n" + crate::far_shadow::WGSL + "\n" + &crate::probe_grid::wgsl(true)
            });
            assert_eq!(src.contains("fn w_rip2"), expect_aaa, "{label}: AAA water block presence mismatch");
            let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(&src)));
            naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
                .validate(&module)
                .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(&src)));
        }
    }
}
