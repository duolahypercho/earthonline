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
//!    centre of the frame), a plain one beside it (every block alike) and a
//!    bilateral grid of it (cells of `CAM_CELL_PX` pixels, `CAM_GRID_Z`
//!    luminance slices; Chen, Paris and Durand 2007); the meter's log-average
//!    over a percentile window (`cam1`), its correction of the hour's exposure
//!    (clamped, `cam0.z` up, `cam1.w` down, snapped on a cut, else converging at
//!    `CAM_ADAPT_RATE`); the visible sun's energy over its disc; the lens's
//!    focus (the mean dioptre of the frame's middle, from the distances the
//!    resolve leaves in its alpha, pulled at `CAM_FOCUS_RATE`); the grid blurred
//!    and written to a 3D texture;
//! 2. the lens: the depth of field (a gathered blur circle from the lens's
//!    focal length, f-number and focus, `cam_coc`), lateral colour, the bloom
//!    threshold-free and energy kept (the pixel gives up `cam_bloom_part`, the
//!    `CAM_BLOOM_LEVELS` levels give it back with a power-law tail), the
//!    optical fall-off (`CAM_VIGNETTE` of cos^4);
//! 3. the exposure (the hour's x the meter's) and the local exposure: the
//!    grid's base luminance compressed under middle grey and over the
//!    highlights' knee only as far as the frame's own range needs (the meter
//!    measures where its darkest and brightest few percent sit and compresses
//!    each side just enough to bring them inside the display's room, `cam3.xy`,
//!    never past `cam0.xy`), and the detail against the grid's edge-aware base
//!    raised by `cam3.z` (a print's local contrast); at night, when the frame's
//!    brightest sources fall short, a pixel's own highlights pushed toward
//!    white (`cam_push`); the sun's veiling glare, soft starburst and ghosts
//!    scaled by its measured energy (no lamp starbursts: sky.rs skips the
//!    sprite glare);
//! 4. the camera's response: a long toe under a mid-tone contrast
//!    (`CAM_TOE`, `tone_log`) into a hue-preserving filmic curve (an AgX-style
//!    log sigmoid in an inset gamut, so a bright colour runs to white without
//!    per-channel clipping), sRGB, the hour's grade (`CAM_GRADE_KEYS` through
//!    `cam_grade`: its S-curve over the mid-tones only, its warm highlights kept
//!    off a blue sky), a fine grain in the shadows and mid-tones (more at
//!    night; none in a bright sky) and the dither.
//!
//! The sharpen is a robust contrast-adaptive one (after AMD's RCAS: its
//! lobe limited so nothing clips or rings, halved on noise, gated off in
//! flat gradients so a clear sky is not roughened, and off where the focus
//! blurs), and the lens's lateral colour is the model's own (`CAM_LATERAL_CA`).
//!
//! ## The model's later features (camera track, round 2)
//!
//! Each behind its own switch and bit (`CAM_BIT_*`, `SF_CAM_*`), compiled in
//! only with it and `--cine-camera`: `--camera-tone` (the meter on the ground
//! with the sun's pull, a standing local compression, the hue-preserving
//! display transform `display` / `cam_display`, a grain on the sensor's gain),
//! `--white-balance` (a grey-world balance on the frame's surfaces, the hour's
//! tints halved over it: `show_key`), `--lens-halation` (the weather's water
//! widens the bloom's halo: `cam_halation`).

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
    /// (`--cine-camera`) x the local exposure's strongest compression in the
    /// shadows, y in the highlights (1: none), z the meter's largest lift (EV),
    /// w the model on (1/0).
    pub cam0: [f32; 4],
    /// x, y the meter's window (shares of the metered weight), z the exposure
    /// compensation (EV against middle grey), w the meter's largest cut (EV).
    pub cam1: [f32; 4],
    /// x the grain's amount (display, at its peak), y the sun glare's gain, z the
    /// debug bars (1/0), w the lateral colour (px, red to blue, at the corner).
    pub cam2: [f32; 4],
    /// x, y the display's room below and above middle grey (EV) before the local
    /// exposure compresses, z the detail's gain (1: none), w the highlights' knee (EV
    /// over middle grey: the local exposure moves only what lies above it).
    pub cam3: [f32; 4],
    /// x the local exposure's largest expansion of the highlights (1: none), y where the
    /// frame's brightest sources are placed (EV over middle grey: `CAM_TOP_SHARE` of the
    /// frame), z the glare's veil (exposed, at its core), w the lens's f-number (0: no
    /// depth of field).
    pub cam4: [f32; 4],
    /// (`--camera-tone`) x, y the local exposure's standing compression of the base under
    /// middle grey and over the highlights' knee (1: none; the range's fit may take it
    /// further), z the grain's colour share, w the sharpen's strength (of RCAS's limit).
    /// Zero without the feature.
    pub cam5: [f32; 4],
    /// (`--white-balance`) the degrees of adaptation along the colour temperature's
    /// axis (x a warm light, y a cool one) and the green-magenta axis (z), w the
    /// temperature's largest correction (log2 a channel). Zero without the feature.
    pub cam6: [f32; 4],
    /// (`--white-balance`) xyz the hour's white-balance preset (`wb_preset`), w the
    /// hour's saturation in the scene (`wb_preset_sat`); zero without the feature.
    pub cam7: [f32; 4],
}

impl CineUniform {
    pub fn new() -> Self {
        let n = grade::LUT_SIZE as f32;
        Self {
            bloom: [grade::CINE_BLOOM_SHARE as f32, 0.0, 0.0, 0.0],
            grade: [grade::CINE_VIGNETTE as f32, (n - 1.0) / n, 0.5 / n, 0.0],
            cam0: [0.0; 4],
            cam1: [0.0; 4],
            cam2: [0.0; 4],
            cam3: [0.0; 4],
            cam4: [0.0; 4],
            cam5: [0.0; 4],
            cam6: [0.0; 4],
            cam7: [0.0; 4],
        }
    }

    /// (`--cine-camera`) the uniform with the camera model's terms for the sun's altitude
    /// (and its later features' where their code is compiled in: `ShaderFeatures::camera_bits`).
    pub fn with_camera(sun_alt_deg: f64) -> Self {
        Self::with_camera_bits(sun_alt_deg, crate::look::shader_features().camera_bits)
    }

    /// (`--cine-camera`) as `with_camera` for the later features `bits` (`CAM_BIT_*`).
    pub fn with_camera_bits(sun_alt_deg: f64, bits: u32) -> Self {
        let mut u = Self::new();
        let tone = bits & CAM_BIT_TONE != 0;
        let p = if tone { tone_params(sun_alt_deg) } else { camera_params(sun_alt_deg) };
        u.cam0 = [p.shadow_contrast as f32, p.highlight_contrast as f32, p.max_lift_ev as f32, 1.0];
        u.cam1 = [p.window[0] as f32, p.window[1] as f32, p.compensation_ev as f32, p.max_cut_ev as f32];
        // (`EO_CAM_DEBUG=1`: the meter's state drawn as bars along the frame's top, for tuning.)
        // (`EO_CAM_DEBUG=2`: the frame's exposed log luminance written in place of the picture.)
        static DEBUG: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
        let debug = *DEBUG.get_or_init(|| match std::env::var("EO_CAM_DEBUG").as_deref() {
            Ok("1") => 1.0,
            Ok("2") => 2.0,
            _ => 0.0,
        });
        u.cam2 = [p.grain as f32, CAM_FLARE_GAIN as f32, debug, CAM_LATERAL_CA as f32];
        u.cam3 = [p.shadow_room_ev as f32, p.highlight_room_ev as f32, p.detail as f32, p.knee_ev as f32];
        u.cam4 = [p.highlight_expand as f32, p.top_ev as f32, CAM_GLARE_VEIL as f32, p.f_number as f32];
        if tone {
            let day = day_share(sun_alt_deg);
            u.cam5 = [p.base_shadow as f32, p.base_highlight as f32, (CAM_TONE_GRAIN_CHROMA[0] + (CAM_TONE_GRAIN_CHROMA[1] - CAM_TONE_GRAIN_CHROMA[0]) * day) as f32, 2f64.powf(-CAM_TONE_SHARPEN_STOPS) as f32];
        }
        if bits & CAM_BIT_WB != 0 {
            let d = day_share(sun_alt_deg);
            let [n, dd] = CAM_WB_DEGREE;
            u.cam6 = [(n[0] + (dd[0] - n[0]) * d) as f32, (n[1] + (dd[1] - n[1]) * d) as f32, CAM_WB_TINT_DEGREE as f32, CAM_WB_LIMIT[0] as f32];
        }
        u
    }
}

// ---------------------------------------------------------------------------
// (camera track, round 2) the camera model's later features. Each has its own
// switch and bit (`SkyOptions::cam_bits` -> `ShaderFeatures::camera_bits`,
// `SF_CAM_*`), compiled in only with it and with `--cine-camera`; without its
// bit the model draws exactly as before.

/// `--camera-tone`: the camera's tone as a modern cine camera's HDR path: the
/// meter reads the ground and the subject, not the sky (sky blocks count
/// `CAM_TONE_SKY_METER`), lifts a shaded street further (`tone_params`); the
/// local exposure keeps a standing compression of the base (the sky and a
/// sunlit wall come down toward the shade, the shade comes up a little) on
/// top of the range's fit, with less of the wide blur in the base (halos);
/// a hue-preserving display transform (`cam_display`: the curve on a norm
/// between luminance and the brightest channel, the ratios kept, a soft
/// gamut edge that desaturates toward the colour's own luminance instead of
/// clipping a channel, the path to white far over it, the toe's chroma rolled
/// off); a grain that follows the sensor's gain (the meter's lift and the
/// night's exposure) with a colour share at night; a stronger sharpen.
pub const CAM_BIT_TONE: u32 = 1;
/// `--white-balance`: the camera's automatic white balance on the scene's own
/// light: a grey-world estimate over the frame's surfaces (no sky, no lamps
/// or deep shadow, no strongly coloured surface), corrected along the colour
/// temperature's axis by `CAM_WB_DEGREE` (by the hour, a warm light less than a
/// cool one) and the green-magenta axis by `CAM_WB_TINT_DEGREE`, in the linear
/// scene before the exposure; and the hour's grade with its tints halved over
/// that balance (`show_key`): one camera's look whose colour follows the light.
pub const CAM_BIT_WB: u32 = 2;
/// `--lens-halation`: the weather's water in the air and on the front glass
/// spreads the bright sources' light into a wider, stronger halo
/// (`cam_halation`: the weather's lamp haze and the rain's rate; energy kept as
/// the bloom is).
pub const CAM_BIT_HALATION: u32 = 4;
pub const CAM_BITS_ALL: u32 = CAM_BIT_TONE | CAM_BIT_WB | CAM_BIT_HALATION;

/// (`--camera-tone`) the norm the curve maps: this share of the brightest
/// channel over the luminance (0: luminance, every colour's ratios kept and a
/// saturated light pushed out of the gamut early; 1: the brightest channel, a
/// saturated light kept dark). Hue is kept either way.
pub const CAM_TONE_NORM_MAX: f64 = 0.4;
/// (`--camera-tone`) the mid-tones' saturation (a print's: about the per-channel
/// sigmoid's at middle grey), the share of it left in the deep toe (display
/// luminance `CAM_TONE_TOE_DESAT` low .. high: a black's colour rolls off toward
/// neutral, as a sensor's does under its noise floor) and in the shoulder (display
/// luminance `CAM_TONE_SHOULDER_DESAT` low .. high: a highlight pales on its way to white).
pub const CAM_TONE_SAT: [f64; 3] = [1.18, 0.85, 0.75];
pub const CAM_TONE_TOE_DESAT: [f64; 2] = [0.002, 0.03];
pub const CAM_TONE_SHOULDER_DESAT: [f64; 2] = [0.25, 0.9];
/// (`--camera-tone`) the gamut's soft edge: from this display value a channel's
/// excess is taken by desaturating toward the colour's luminance, the channel
/// brought toward 1 along a rational curve (k + (1 - k) e / ((1 - k) + e) for
/// an excess e: it reaches 1 only far over it), so a golden sky runs through
/// cream to white without a flat R = 255 plateau; the path to white
/// (`CAM_TONE_WHITEN_EV`) takes the rest.
pub const CAM_TONE_GAMUT_KNEE: f64 = 0.75;
/// (`--camera-tone`) the path to white: the norm this far over middle grey (EV,
/// after the local exposure) goes to white over this range (the sun's disc, a
/// lamp's core: a sensor's channels all full).
pub const CAM_TONE_WHITEN_EV: [f64; 2] = [3.5, 7.5];
/// (`--camera-tone`) the meter's weight of a block of sky (its distance in the
/// resolve's alpha): the meter reads the ground, the walls and the subject.
pub const CAM_TONE_SKY_METER: f64 = 0.15;
/// (`--camera-tone`) the sun in the frame pulls the meter down by up to this many
/// EV, over its disc's mean exposed luminance (log2, at the hour's exposure)
/// `CAM_TONE_SUN_EV`: a backlit street keeps its depth under the glare.
pub const CAM_TONE_SUN_PULL: f64 = 1.0;
pub const CAM_TONE_SUN_EV: [f64; 2] = [3.0, 6.0];
/// (`--camera-tone`) the grid's own (edge-aware) base against the column blur:
/// more of the edge-aware base, as the standing compression would make the
/// blur's halo show at a roofline.
pub const CAM_TONE_BILATERAL_SHARE: f64 = 0.85;
/// (`--camera-tone`) the grain's standard deviation at its peak (display), night
/// and day, before the sensor's gain (x sqrt of the meter's lift, `cam_state.gain`,
/// clamped to 0.5 .. 3), and its colour share (night, day).
pub const CAM_TONE_GRAIN: [f64; 2] = [0.0075, 0.0022];
pub const CAM_TONE_GRAIN_CHROMA: [f64; 2] = [0.40, 0.10];
/// (`--camera-tone`) the sharpen's strength in stops under RCAS's limit.
pub const CAM_TONE_SHARPEN_STOPS: f64 = 0.3;

/// (`--white-balance`) the degree of adaptation along the colour temperature's
/// axis (log2 of blue over red) when the frame's light is warm and when it is
/// cool, at night and by day (`day_share` blends them): a camera's automatic
/// balance that keeps most of a golden hour's warmth (as a camera's "keep warm
/// atmosphere" does) and takes half the sky's blue out of a shaded street by day,
/// and at twilight and night leaves most of the light's colour (the blue hour
/// stays blue) while taking a quarter of a sodium street's orange out.
pub const CAM_WB_DEGREE: [[f64; 2]; 2] = [[0.25, 0.20], [0.15, 0.50]];
/// (`--white-balance`) the degree along the green-magenta axis (green over red and blue).
pub const CAM_WB_TINT_DEGREE: f64 = 0.35;
/// (`--white-balance`) the largest correction a channel takes (log2), on each axis.
pub const CAM_WB_LIMIT: [f64; 2] = [0.45, 0.10];
/// (`--white-balance`) a block counts toward the estimate within these EV of
/// middle grey (after the meter: not the noise floor, not a lamp or the sun's
/// glint) and under the chroma (max - min over max) range's top (a red car or a
/// lawn is not the light's colour).
pub const CAM_WB_EV: [f64; 2] = [-4.5, 2.5];
pub const CAM_WB_CHROMA: [f64; 2] = [0.30, 0.65];
/// (`--white-balance`) the camera's grade (display space, after the curve): the
/// hour's keys' curves (contrast, pivot) with these balances and saturation in
/// place of the hour's tints: a faint cool shadow and warm highlight (a print's;
/// the highlights' balance never raises a channel, so the grade brings back no
/// plateau at 1), no extra saturation (the display transform's).
pub const CAM_SHOW_BALANCE: [[f64; 3]; 2] = [[0.985, 1.0, 1.02], [1.0, 0.985, 0.96]];
pub const CAM_SHOW_SAT: [f64; 2] = [0.97, 1.03];
/// (`--white-balance`) the hour's colour as the camera's white-balance preset,
/// in the scene before the curve (so a warm golden highlight rolls through the
/// display transform's soft gamut edge to white instead of being pushed past it
/// by a display-space tint): the hour's overall cast raised to this power (three
/// quarters of the hour's tint, in log), at unit luminance (`wb_preset`); and the
/// hour's saturation (the mean of its shadows' and highlights') as the camera's
/// colour rendering, about each colour's luminance in the scene (`wb_preset_sat`).
pub const CAM_SHOW_KEEP: f64 = 0.75;

/// (`--lens-halation`) the halo's growth with the weather: the weather's lamp
/// haze over clear air's (1: clear) per `CAM_HAL_HAZE` and the rain's rate (the
/// front glass's water: 1 - exp(-rate / `CAM_HAL_RAIN_MMH`)) add up to `h`;
/// the veil's share grows by `CAM_HAL_GAIN[0]` x h and the halation's by `[1]` x h.
pub const CAM_HAL_HAZE: f64 = 0.5;
pub const CAM_HAL_RAIN_MMH: f64 = 3.0;
pub const CAM_HAL_GAIN: [f64; 2] = [0.8, 1.6];
/// (`--lens-halation`) the bloom's cap on a source's energy (`CAM_BLOOM_CAP`)
/// rises by this x h: a lamp's whole light, not a sun-safe share of it, is
/// scattered by the water in the air.
pub const CAM_HAL_CAP: f64 = 3.0;

/// The day's share over the sun's altitude (0 night, 1 day; -9 .. +2 deg).
pub fn day_share(sun_alt_deg: f64) -> f64 {
    let a = if sun_alt_deg.is_nan() { 90.0 } else { sun_alt_deg };
    let t = ((a + 9.0) / 11.0).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// (`--camera-tone`) the meter and the local exposure by the hour: the model's
/// (`camera_params`) with the meter's compensation raised by day (the shade's
/// whites read white) and its lift allowed further, the blue hour kept darker
/// (the lamps' pools against a twilight street), the highlights' knee lower and a
/// standing compression of the base by day (the shadows a little, the
/// highlights more), a floor a little lower, the grain by `CAM_TONE_GRAIN`.
pub fn tone_params(sun_alt_deg: f64) -> CameraParams {
    let day = day_share(sun_alt_deg);
    let mix = |n: f64, d: f64| n + (d - n) * day;
    let base = camera_params(sun_alt_deg);
    // the blue hour's dip: strongest a little after sunset (day 0.1 .. 0.5).
    let blue = 4.0 * day * (1.0 - day);
    CameraParams {
        highlight_contrast: mix(0.75, 0.45),
        compensation_ev: -1.4 + 1.15 * day * day - 0.45 * blue,
        max_lift_ev: mix(0.75, 2.2),
        knee_ev: mix(0.0, 0.8),
        detail: mix(1.18, 1.30),
        grain: mix(CAM_TONE_GRAIN[0], CAM_TONE_GRAIN[1]),
        base_shadow: mix(1.0, 0.86),
        base_highlight: mix(1.0, 0.62),
        ..base
    }
}

/// (`--white-balance`) a camera key with the camera's grade (`CAM_SHOW_*`) in
/// place of the hour's tints (they move into the white balance: `wb_preset`).
pub fn show_key(k: &grade::GradeKey) -> grade::GradeKey {
    grade::GradeKey { shadow_balance: CAM_SHOW_BALANCE[0], highlight_balance: CAM_SHOW_BALANCE[1], sat_shadow: CAM_SHOW_SAT[0], sat_highlight: CAM_SHOW_SAT[1], gain: [1.0; 3], ..*k }
}

/// (`--white-balance`) the camera's white-balance preset for the hour (the keys'
/// weights `w`, `grade::key_weights`): each key's overall cast (the geometric
/// mean of its shadow and highlight balances, x its gain), the weighted
/// geometric mean raised to `CAM_SHOW_KEEP`, at unit luminance (a warm golden
/// hour, a cool blue hour, near neutral by day and at night).
/// (`--white-balance`) the hour's saturation as the camera's colour rendering
/// (the keys' mean of their shadows' and highlights' saturation, weighted by `w`).
pub fn wb_preset_sat(w: &[f64; grade::KEY_COUNT]) -> f64 {
    let (mut s, mut ws) = (0.0, 0.0);
    for (k, wi) in CAM_GRADE_KEYS.iter().zip(w.iter()) {
        s += wi * 0.5 * (k.sat_shadow + k.sat_highlight);
        ws += wi;
    }
    if ws > 0.0 { s / ws } else { 1.0 }
}

pub fn wb_preset(w: &[f64; grade::KEY_COUNT]) -> [f64; 3] {
    let mut l = [0.0; 3];
    let mut ws = 0.0;
    for (k, wi) in CAM_GRADE_KEYS.iter().zip(w.iter()) {
        if *wi > 0.0 {
            for c in 0..3 {
                l[c] += wi * ((k.shadow_balance[c] * k.highlight_balance[c]).max(1e-6).sqrt() * k.gain[c]).max(1e-6).ln();
            }
            ws += wi;
        }
    }
    if ws <= 0.0 {
        return [1.0; 3];
    }
    let g = l.map(|v| (v / ws * CAM_SHOW_KEEP).exp());
    let y = 0.2126 * g[0] + 0.7152 * g[1] + 0.0722 * g[2];
    g.map(|v| v / y)
}

/// (`--camera-tone`) the camera's display transform (CPU reference of
/// look_cine_camera.wgsl `cam_display`): a scene colour (exposed: middle grey
/// 0.18) to display linear.
pub fn display(x: [f64; 3]) -> [f64; 3] {
    let x = x.map(|v| v.max(0.0));
    let lw = [0.2126, 0.7152, 0.0722];
    let dot = |a: [f64; 3]| a[0] * lw[0] + a[1] * lw[1] + a[2] * lw[2];
    let y = dot(x);
    let m = x[0].max(x[1]).max(x[2]);
    let n = y + (m - y) * CAM_TONE_NORM_MAX;
    if n <= 1e-10 {
        return [0.0; 3];
    }
    let t = sigmoid_display(n);
    let mut c = x.map(|v| v * t / n);
    let l = dot(c);
    let s = CAM_TONE_SAT[0]
        * (CAM_TONE_SAT[1] + (1.0 - CAM_TONE_SAT[1]) * eo_look::math::smoothstep(CAM_TONE_TOE_DESAT[0], CAM_TONE_TOE_DESAT[1], l))
        * (1.0 - (1.0 - CAM_TONE_SAT[2]) * eo_look::math::smoothstep(CAM_TONE_SHOULDER_DESAT[0], CAM_TONE_SHOULDER_DESAT[1], l));
    c = c.map(|v| l + (v - l) * s);
    let w = eo_look::math::smoothstep(CAM_TONE_WHITEN_EV[0], CAM_TONE_WHITEN_EV[1], (n / 0.18).log2());
    let top = c[0].max(c[1]).max(c[2]).max(t);
    c = c.map(|v| v + (top - v) * w);
    let l2 = dot(c);
    let mx = c[0].max(c[1]).max(c[2]);
    let mn = c[0].min(c[1]).min(c[2]);
    let mut k: f64 = 1.0;
    let tk = CAM_TONE_GAMUT_KNEE;
    if mx > tk && mx > l2 + 1e-9 {
        let soft = tk + (1.0 - tk) * (mx - tk) / ((1.0 - tk) + (mx - tk));
        k = (soft.max(l2) - l2) / (mx - l2);
    }
    if mn < 0.0 {
        k = k.min(l2 / (l2 - mn).max(1e-9));
    }
    let k = k.clamp(0.0, 1.0);
    c.map(|v| (l2 + (v - l2) * k).clamp(0.0, 1.0))
}

/// The camera's curve on one value (`cam_tone_log` then the AgX sigmoid, decoded
/// with its 2.2): scene (middle grey 0.18) -> display linear.
pub fn sigmoid_display(n: f64) -> f64 {
    let g = 0.18f64.log2();
    let v = g + tone_log(n.max(1e-10).log2() - g);
    let v = ((v - CAM_AGX_EV[0]) / (CAM_AGX_EV[1] - CAM_AGX_EV[0])).clamp(0.0, 1.0);
    let v2 = v * v;
    let v4 = v2 * v2;
    let e = 15.5 * v4 * v2 - 40.14 * v4 * v + 31.96 * v4 - 6.868 * v2 * v + 0.4298 * v2 + 0.1191 * v - 0.00232;
    e.clamp(0.0, 1.0).powf(2.2)
}

// ---------------------------------------------------------------------------
// (`--cine-camera`) the camera model's constants.

/// The meter's state buffer (cine_camera.wgsl `CamState`).
pub const CAM_STATE_BYTES: u64 = 64;
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
/// field down, 16:9: the corners about 0.7 EV under the centre, a cine prime's
/// natural fall-off near wide open; the full cos^4 law would be 2 EV).
pub const CAM_VIGNETTE: f64 = 0.5;
/// Film grain: the standard deviation in display values at its peak (the
/// shadows and low mid-tones, display 0.12 .. 0.35), a little over a pixel
/// across (the photo finish's `ph_grain`: neighbours correlate as a demosaiced
/// sensor's do), none over display 0.65 (a clear sky stays a clean gradient); at
/// night and by day (a camera pushed to a higher ISO shows more), `camera_params`
/// blends them.
pub const CAM_GRAIN: [f64; 2] = [0.0065, 0.0042];
/// The lens's lateral colour: red against blue at the frame's corner (px of a
/// 1080-line frame), growing as the square of the radius (faint, at the edges only).
pub const CAM_LATERAL_CA: f64 = 2.0;
/// The shares of the frame at each end of its range the local exposure fits
/// into the display's room: the dark end of the meter's weight (the street, not
/// the sky's corners), the bright end of the whole frame unweighted (a sky gap
/// counts where it is, even at the top of the frame); the darkest and brightest
/// few percent may sit outside it (a doorway's black, a glint).
pub const CAM_RANGE_SHARE: [f64; 2] = [0.03, 0.99];
/// The share of the whole frame under its brightest sources: at night the
/// local exposure may expand the highlights (up to `highlight_expand`) until
/// this point sits at `top_ev` (a lamp's core burns to white instead of a
/// dim cream; the street under it keeps its level).
pub const CAM_TOP_SHARE: f64 = 0.999;
/// The camera's response under the sigmoid, in log2 about middle grey: the
/// contrast `CAM_CONTRAST` from the mid-tones down to about 3 EV under grey,
/// easing to `CAM_TOE[0]` in the deep shadows (centre `CAM_TOE[1]` EV, width
/// `CAM_TOE[2]` EV): a cine camera's long toe (a scene 4 EV under grey shows at
/// sRGB 17, 5 EV under at 10, 6 EV under at 6) instead of a crush at 4.5 EV
/// under, while the mid-shadows keep their depth (2 and 3 EV under at 54 and 30,
/// as before): night streets and canyon shade keep their texture, not a lift.
pub const CAM_TOE: [f64; 3] = [0.45, -3.1, 0.5];
/// How hard the detail's gain rolls off (EV): detail this far from its base
/// gets half the gain's lift, so an edge's step is not doubled.
pub const CAM_DETAIL_SPAN: f64 = 1.5;
/// The depth of field: a lens on a full-frame sensor (`CAM_SENSOR_MM` high; the
/// frame's vertical field gives the focal length: 24.6 mm at the cards' 52 deg)
/// at `CAM_F_NUMBER` (night, day), focused by the meter on the frame's middle
/// (the mean dioptre of its centre: the focus that blurs the subject least),
/// pulled at `CAM_FOCUS_RATE`; the blur circle (Gaussian lens: (f^2 / N) |1/S -
/// 1/D| on the sensor) gathered over `CAM_DOF_TAPS` taps where it is over 0.6 px,
/// at most `CAM_DOF_MAX_PX` across (px of a 1080-line frame). At f/4 a street
/// 3 m off with the focus far away blurs by about 2.5 px; past 10 m nothing does.
pub const CAM_SENSOR_MM: f64 = 24.0;
pub const CAM_F_NUMBER: [f64; 2] = [2.8, 4.0];
pub const CAM_FOCUS_RATE: f64 = 4.0;
pub const CAM_DOF_TAPS: u32 = 16;
pub const CAM_DOF_MAX_PX: f64 = 8.0;

/// The sun's glare (starburst and ghosts) at full strength, exposed units.
pub const CAM_FLARE_GAIN: f64 = 1.0;
/// The sun's veiling glare (the lens's scatter round the source: a core and a
/// long 1/r^2 skirt over the frame), exposed units at its core with the sun
/// fully visible.
pub const CAM_GLARE_VEIL: f64 = 2.5;
/// The sun's measured energy (mean exposed luminance over its disc) where its
/// glare starts and where it is full: log2.
pub const CAM_FLARE_EV: [f64; 2] = [2.5, 6.0];

/// The bloom: the share of every pixel the lens spreads (veiling glare: a
/// coated prime's 1-2 %; at 3 % the shadows took a milky lift), the
/// added share of a bright one (the highlights' halation through a light
/// diffusion), from `CAM_BLOOM_MIST_EV[0]` to `[1]` (log2, exposed: +1.5 and
/// +4 EV over middle grey); above `CAM_BLOOM_CAP` (exposed) a source's energy
/// grows only logarithmically (the sun's disc, a glint).
pub const CAM_BLOOM_VEIL: f64 = 0.012;
pub const CAM_BLOOM_MIST: f64 = 0.10;
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

/// The bloom's WGSL constants and its prefilter's switch (sky.rs composes it before sky_bloom.wgsl);
/// (`--lens-halation`, `SF_CAM_HALATION`) the weather's halo, compiled in with its bit.
pub fn bloom_wgsl(camera: bool) -> String {
    let f = |v: f64| format!("{:?}", v as f32);
    let halation = camera && crate::look::shader_features().camera_bits & CAM_BIT_HALATION != 0;
    format!(
        "\n// generated by cine_post.rs bloom_wgsl (`--cine-camera`)\nconst SF_CAMERA: bool = {camera};\nconst CAM_BLOOM_VEIL: f32 = {};\nconst CAM_BLOOM_MIST: f32 = {};\nconst CAM_BLOOM_MIST_EV: vec2<f32> = vec2<f32>({}, {});\nconst CAM_BLOOM_CAP: f32 = {};\nconst SF_CAM_HALATION: bool = {halation};\nconst CAM_HAL_HAZE: f32 = {};\nconst CAM_HAL_RAIN_MMH: f32 = {};\nconst CAM_HAL_GAIN: vec2<f32> = vec2<f32>({}, {});\nconst CAM_HAL_CAP: f32 = {};\n{}",
        f(CAM_BLOOM_VEIL),
        f(CAM_BLOOM_MIST),
        f(CAM_BLOOM_MIST_EV[0]),
        f(CAM_BLOOM_MIST_EV[1]),
        f(CAM_BLOOM_CAP),
        f(CAM_HAL_HAZE),
        f(CAM_HAL_RAIN_MMH),
        f(CAM_HAL_GAIN[0]),
        f(CAM_HAL_GAIN[1]),
        f(CAM_HAL_CAP),
        CAM_BLOOM_PART_WGSL,
    )
}

/// The share of a pixel (scene values `c`, the hour's exposure `e`) the lens
/// spreads, in scene values: the bloom's prefilter takes it from every tap and
/// the post pass takes it off the pixel, so the bloom moves light, it adds none.
/// (`--lens-halation`) the weather's water grows the veil's and the halation's
/// shares (`cam_halation`): the weather's lamp haze over clear air's (droplets in
/// the air scatter a source's light forward into a halo round it) and the rain's
/// rate (water on the front glass); both shaders read the same `skyu`.
const CAM_BLOOM_PART_WGSL: &str = "
fn cam_halation() -> f32 {
    if (!SF_CAM_HALATION) {
        return 0.0;
    }
    let haze = clamp((skyu.bloom_w2.w - 1.0) / CAM_HAL_HAZE, 0.0, 2.0);
    let rain = 1.0 - exp(-max(skyu.wet.z, 0.0) / CAM_HAL_RAIN_MMH);
    return select(haze, 0.0, haze != haze) + select(rain, 0.0, rain != rain);
}

fn cam_bloom_part(c: vec3<f32>, e: f32) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0)) * e;
    let l = dot(x, vec3<f32>(0.2126, 0.7152, 0.0722));
    if (!(l > 1e-12) || !(e > 0.0)) {
        return vec3<f32>(0.0);
    }
    var cap = CAM_BLOOM_CAP;
    var h = 0.0;
    if (SF_CAM_HALATION) {
        h = cam_halation();
        cap = CAM_BLOOM_CAP * (1.0 + CAM_HAL_CAP * h);
    }
    let lc = select(l, cap * (1.0 + log(l / cap)), l > cap);
    let mist = smoothstep(CAM_BLOOM_MIST_EV.x, CAM_BLOOM_MIST_EV.y, log2(lc));
    var share = CAM_BLOOM_VEIL + CAM_BLOOM_MIST * mist;
    if (SF_CAM_HALATION) {
        share = CAM_BLOOM_VEIL * (1.0 + CAM_HAL_GAIN.x * h) + CAM_BLOOM_MIST * (1.0 + CAM_HAL_GAIN.y * h) * mist;
    }
    return x * (lc / l) * share / e;
}
";

/// The meter and the local exposure by the hour (`camera_params`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraParams {
    /// The local exposure's strongest compression about middle grey: below it,
    /// above it (1: none). It compresses only as far as the frame's range needs.
    pub shadow_contrast: f64,
    pub highlight_contrast: f64,
    /// The display's room below and above middle grey (EV, after the meter):
    /// the frame's darkest and brightest few percent (`CAM_RANGE_SHARE`) are
    /// brought inside it by the local exposure, which does nothing to a frame
    /// that fits (the curve holds about -4 .. +4 EV: 7 and 244 of 255).
    pub shadow_room_ev: f64,
    pub highlight_room_ev: f64,
    /// The detail's gain against the grid's edge-aware base (1: none).
    pub detail: f64,
    /// The meter's window: the shares of the metered weight it averages between.
    pub window: [f64; 2],
    /// Where the window's log-average lands, EV against middle grey.
    pub compensation_ev: f64,
    /// The meter's largest correction of the hour's exposure, up and down (EV).
    pub max_lift_ev: f64,
    pub max_cut_ev: f64,
    /// The highlights' knee (EV over middle grey): the local exposure compresses
    /// or expands only what lies above it (by day a sunlit wall keeps its level
    /// while a sky gap comes down).
    pub knee_ev: f64,
    /// The local exposure's largest expansion of the highlights (1: none) and
    /// where it places the frame's brightest sources (`CAM_TOP_SHARE`), EV over grey.
    pub highlight_expand: f64,
    pub top_ev: f64,
    /// The grain's standard deviation at its peak (display values).
    pub grain: f64,
    /// The lens's f-number (the depth of field: wider open at night).
    pub f_number: f64,
    /// (`--camera-tone`) the local exposure's standing compression of the base under
    /// middle grey and over the highlights' knee (1: none; `camera_params` leaves them 1).
    pub base_shadow: f64,
    pub base_highlight: f64,
}

/// By day (and through the golden hour) the meter averages the middle of the
/// frame's tones and may lift a shaded street by 1.3 EV over the hour's
/// exposure (the camera's long toe holds the rest) but never darkens a sunlit
/// frame under it; the local exposure fits a deep shade into the display's room
/// and brings a bright sky gap down above a knee 1.6 EV over grey (a sunlit wall
/// keeps its level) and raises the detail by
/// a fifth; at night it meters the lit part (the upper tones), keeps a darker
/// key and most of the shadows' depth (a lamp's pool against the unlit gap; the
/// camera's long toe holds the gap's texture), and expands the highlights by
/// up to half again until the frame's brightest sources sit 4 EV over grey (a
/// pushed night stock: the lamps burn, the street keeps its level), with more
/// grain. Blended over the sun at -9 .. +2 deg (the blue hour, the sun 4-8 deg
/// down, takes about a quarter of the day's; its exposure stays nearer the night's).
pub fn camera_params(sun_alt_deg: f64) -> CameraParams {
    let day = day_share(sun_alt_deg);
    let mix = |n: f64, d: f64| n + (d - n) * day;
    CameraParams {
        shadow_contrast: mix(0.88, 0.70),
        highlight_contrast: mix(0.75, 0.55),
        shadow_room_ev: mix(5.0, 3.5),
        highlight_room_ev: mix(3.5, 2.8),
        detail: mix(1.12, 1.22),
        window: [mix(0.75, 0.35), mix(0.99, 0.90)],
        // (the blue hour keeps nearer the night's darker key: the sky's glow over a dim street)
        compensation_ev: -1.4 + 0.9 * day * day,
        max_lift_ev: mix(0.75, 1.3),
        // by day the hour's exposure is a sunlit scene's (the meter rescues a shaded
        // street); a sunlit aerial or plaza is not pulled down (a golden aerial keeps its glow).
        max_cut_ev: mix(2.0, 0.0),
        knee_ev: mix(0.0, 1.6),
        highlight_expand: mix(1.5, 1.0),
        top_ev: 4.0,
        grain: mix(CAM_GRAIN[0], CAM_GRAIN[1]),
        f_number: mix(CAM_F_NUMBER[0], CAM_F_NUMBER[1]),
        base_shadow: 1.0,
        base_highlight: 1.0,
    }
}

/// The grade's keyframes under the camera model (`GRADE_KEYS` order, so
/// `grade::key_weights` blends them; `cam_grade` applies them): the curve's
/// flatter mid-tones take more contrast over the pivot; by day a sky-cool shade
/// against a slightly warm sun (daylight balanced for the sun and the sky
/// together, as a camera's daylight preset: the shade the sky lights reads cool
/// but near neutral, not navy), the golden hour a warm key against cool
/// shadows, the blue hour a cool ambient with warm lights, the night coloured
/// blacks. The balances stay within about 12 % a channel; the shadows' within 7
/// % by day (the long toe shows them).
pub const CAM_GRADE_KEYS: [grade::GradeKey; grade::KEY_COUNT] = [
    grade::GradeKey { name: "cam-night", contrast: 1.20, pivot: 0.30, sat_shadow: 0.90, sat_highlight: 1.25, shadow_balance: [0.93, 0.99, 1.10], highlight_balance: [1.06, 1.0, 0.92], split: 0.30, gain: [1.0, 1.0, 1.0] },
    grade::GradeKey { name: "cam-blue-hour", contrast: 1.16, pivot: 0.38, sat_shadow: 1.04, sat_highlight: 1.18, shadow_balance: [0.92, 0.98, 1.11], highlight_balance: [1.07, 1.0, 0.91], split: 0.36, gain: [0.97, 1.0, 1.05] },
    grade::GradeKey { name: "cam-golden-am", contrast: 1.14, pivot: 0.42, sat_shadow: 1.0, sat_highlight: 1.15, shadow_balance: [0.95, 0.985, 1.06], highlight_balance: [1.08, 1.0, 0.90], split: 0.38, gain: [1.01, 1.0, 0.97] },
    grade::GradeKey { name: "cam-golden-pm", contrast: 1.16, pivot: 0.42, sat_shadow: 1.0, sat_highlight: 1.20, shadow_balance: [0.95, 0.98, 1.065], highlight_balance: [1.12, 1.0, 0.84], split: 0.36, gain: [1.03, 1.0, 0.94] },
    grade::GradeKey { name: "cam-day", contrast: 1.12, pivot: 0.45, sat_shadow: 1.0, sat_highlight: 1.12, shadow_balance: [0.97, 0.995, 1.045], highlight_balance: [1.05, 1.0, 0.93], split: 0.40, gain: [1.015, 1.0, 0.975] },
    grade::GradeKey { name: "cam-overcast", contrast: 1.10, pivot: 0.45, sat_shadow: 0.97, sat_highlight: 1.04, shadow_balance: [0.97, 1.0, 1.035], highlight_balance: [1.02, 1.0, 0.98], split: 0.40, gain: [0.995, 1.0, 1.005] },
];

/// The grade's S-curve on a luma (0..1), eo-look's: power curves either side of the pivot.
fn s_curve(y: f64, pivot: f64, k: f64) -> f64 {
    let y = y.clamp(0.0, 1.0);
    if y < pivot { pivot * (y / pivot).powf(k) } else { 1.0 - (1.0 - pivot) * ((1.0 - y) / (1.0 - pivot)).powf(k) }
}

/// How much of a display colour is a blue sky's (its blue over its red and
/// green, in the brighter half): the share `cam_grade` keeps the highlights'
/// warm balance off.
pub fn sky_share(s: [f64; 3]) -> f64 {
    use eo_look::math::smoothstep;
    let y = 0.2126 * s[0] + 0.7152 * s[1] + 0.0722 * s[2];
    smoothstep(0.01, 0.07, s[2] - 0.5 * (s[0] + s[1])) * smoothstep(0.2, 0.45, y)
}

/// The camera's grade on a display colour (sRGB-encoded, 0..1): a key's S-curve
/// only from just under its pivot up (eased in over the 0.3 below it: the camera's
/// own toe stays; the key's power curve under the pivot crushed a night street's
/// shadows to black), then eo-look's grade for the key's colour (saturation,
/// the balances, the gain), its highlights' warm balance kept off a blue sky (a
/// colourist's secondary: the warm key on the lit surfaces, the sky's own blue
/// protected instead of bleached to cream).
pub fn cam_grade(s: [f64; 3], k: &grade::GradeKey) -> [f64; 3] {
    use eo_look::math::{mix, smoothstep};
    let s = s.map(|v| v.clamp(0.0, 1.0));
    let y = 0.2126 * s[0] + 0.7152 * s[1] + 0.0722 * s[2];
    let yc = mix(y, s_curve(y, k.pivot, k.contrast), smoothstep(k.pivot - 0.2, k.pivot + 0.1, y));
    // as eo-look's grade: darker by a gain (black stays black), lighter by a gain on the way to white.
    let s = if yc <= y {
        let g = if y > 1e-9 { yc / y } else { 1.0 };
        s.map(|v| v * g)
    } else {
        let g = if y < 1.0 - 1e-9 { (1.0 - yc) / (1.0 - y) } else { 1.0 };
        s.map(|v| 1.0 - (1.0 - v) * g)
    };
    let flat = grade::GradeKey { contrast: 1.0, ..*k };
    let g = grade::grade(s, &flat);
    let w = sky_share(s);
    if w <= 0.0 {
        return g;
    }
    let sky = grade::grade(s, &grade::GradeKey { highlight_balance: [1.0, 1.0, 1.0], ..flat });
    [0, 1, 2].map(|i| mix(g[i], sky[i], w))
}

/// A camera key's table (`cam_grade`), as `grade::bake_lut` lays one out.
pub fn cam_bake_lut(k: &grade::GradeKey) -> Vec<[f32; 3]> {
    let n = grade::LUT_SIZE;
    let step = 1.0 / (n - 1) as f64;
    let mut out = Vec::with_capacity(n * n * n);
    for b in 0..n {
        for g in 0..n {
            for r in 0..n {
                out.push(cam_grade([r as f64 * step, g as f64 * step, b as f64 * step], k).map(|v| v as f32));
            }
        }
    }
    out
}

/// The filmic curve's constants: an AgX-style log sigmoid (Sobotka's formulation
/// and its inset and outset matrices, as published with Blender's AgX view; the
/// sixth-order fit of its sigmoid) over -10 .. +6.5 EV about middle grey.
pub const CAM_AGX_EV: [f64; 2] = [-12.47393, 4.026069];
/// The sharpen's strength in stops under RCAS's limit (0: the strongest, a
/// lobe of 3/16; 0.5: 0.13), on the range-compressed luminance after the
/// temporal resolve.
pub const CAM_SHARPEN_STOPS: f64 = 0.5;
/// The sharpen is off where the neighbourhood's range is under the first share
/// of its brightest and full over the second: a flat gradient (a clear sky, a
/// painted wall) is left smooth instead of having its noise raised.
pub const CAM_SHARPEN_GATE: [f64; 2] = [0.012, 0.05];
/// The scene's contrast about middle grey before the sigmoid (log over log):
/// the sigmoid alone is a flat film (about 0.8-1.1 at grey); 1.25 puts -3 EV at
/// sRGB 31, grey at 127, +2 EV at 201 and white near +5 EV.
pub const CAM_CONTRAST: f64 = 1.25;

/// The toe's constant: its curve passes through middle grey (`cam_tone_log`).
pub fn toe_offset() -> f64 {
    let sp = |x: f64| x.max(0.0) + (-x.abs()).exp().ln_1p();
    let [lo, u0, w] = CAM_TOE;
    (CAM_CONTRAST - lo) * w * sp(-u0 / w)
}

/// The camera's response in log2 about middle grey (`CAM_TOE`; CPU reference of
/// look_cine_camera.wgsl `cam_tone_log`).
pub fn tone_log(u: f64) -> f64 {
    let sp = |x: f64| x.max(0.0) + (-x.abs()).exp().ln_1p();
    let [lo, u0, w] = CAM_TOE;
    lo * u + (CAM_CONTRAST - lo) * w * sp((u - u0) / w) - toe_offset()
}
/// The look's saturation on the curve's encoded values (the sigmoid's inset
/// desaturates the mid-tones; a print stock's dyes give them back: sunlit
/// paint, foliage and brick read as colours, not tints).
pub const CAM_SATURATION: f64 = 1.18;

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
    // x grain, y the sun glare's gain, z debug bars, w lateral colour (px at the corner)
    cam2: vec4<f32>,
    // x, y the display's room below and above middle grey (EV), z the detail's gain, w the highlights' knee (EV)
    cam3: vec4<f32>,
    // x the highlights' largest expansion, y the brightest sources' place (EV), z the sun's veil, w the f-number
    cam4: vec4<f32>,
    // (`--camera-tone`) x, y the base's standing compression under grey and over the knee, z the grain's
    // colour share, w the sharpen's strength
    cam5: vec4<f32>,
    // (`--white-balance`) the adaptation's degree: x a warm light's temperature, y a cool one's, z the tint; w the
    // temperature's largest correction (log2)
    cam6: vec4<f32>,
    // (`--white-balance`) xyz the hour's white-balance preset, w the hour's saturation in the scene (0: none)
    cam7: vec4<f32>,
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
    // the local exposure's compression in the shadows and the highlights (1: none), smoothed
    k_shadow: f32,
    k_high: f32,
    // the frame's range (EV about middle grey after the meter): its dark end (the bright end is
    // not kept; `EO_CAM_DEBUG` draws this one)
    lo: f32,
    // the lens's focus (dioptres: 1 / metres, 0 at infinity), pulled toward the centre's
    // subject (the mean dioptre of the frame's middle: the focus that blurs it least)
    focus: f32,
    // 1 once the frame has carried its distances (the resolve's alpha) and the focus is set
    focus_ok: f32,
    pad0: f32,
    pad1: f32,
    // (`--white-balance`) the balance's gains on the scene's red, green and blue (smoothed),
    // and 1 once an estimate has set them
    wb_r: f32,
    wb_g: f32,
    wb_b: f32,
    wb_ok: f32,
};
";

/// (`--cine-camera`) the camera model's WGSL constants.
pub fn camera_wgsl() -> String {
    let f = |v: f64| format!("{:?}", v as f32);
    format!(
        "\n// generated by cine_post.rs camera_wgsl (`--cine-camera`)\nconst CAM_HIST_BINS: u32 = {}u;\nconst CAM_HIST_LO: f32 = {};\nconst CAM_HIST_HI: f32 = {};\nconst CAM_CELL_PX: u32 = {}u;\nconst CAM_GRID_Z: u32 = {}u;\nconst CAM_GRID_LO: f32 = {};\nconst CAM_GRID_HI: f32 = {};\nconst CAM_BILATERAL_SHARE: f32 = {};\nconst CAM_ADAPT_RATE: f32 = {};\nconst CAM_VIGNETTE: f32 = {};\nconst CAM_FLARE_EV: vec2<f32> = vec2<f32>({}, {});\nconst CAM_AGX_LO: f32 = {};\nconst CAM_AGX_HI: f32 = {};\nconst CAM_CONTRAST: f32 = {};\nconst CAM_SATURATION: f32 = {};\nconst CAM_SHARPEN: f32 = {};\nconst CAM_SHARPEN_GATE: vec2<f32> = vec2<f32>({}, {});\nconst CAM_RANGE_SHARE: vec2<f32> = vec2<f32>({}, {});\nconst CAM_DETAIL_SPAN: f32 = {};\nconst CAM_TOP_SHARE: f32 = {};\nconst CAM_TOE: vec3<f32> = vec3<f32>({}, {}, {});\nconst CAM_TOE_K: f32 = {};\nconst CAM_SENSOR_MM: f32 = {};\nconst CAM_FOCUS_RATE: f32 = {};\nconst CAM_DOF_TAPS: i32 = {};\nconst CAM_DOF_MAX_PX: f32 = {};\n{}",
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
        f(2f64.powf(-CAM_SHARPEN_STOPS)),
        f(CAM_SHARPEN_GATE[0]),
        f(CAM_SHARPEN_GATE[1]),
        f(CAM_RANGE_SHARE[0]),
        f(CAM_RANGE_SHARE[1]),
        f(CAM_DETAIL_SPAN),
        f(CAM_TOP_SHARE),
        f(CAM_TOE[0]),
        f(CAM_TOE[1]),
        f(CAM_TOE[2]),
        f(toe_offset()),
        f(CAM_SENSOR_MM),
        f(CAM_FOCUS_RATE),
        CAM_DOF_TAPS,
        f(CAM_DOF_MAX_PX),
        later_wgsl(),
    )
}

/// (camera track, round 2) the later features' switches (`SF_CAM_*`, from
/// `ShaderFeatures::camera_bits`) and constants, in the post pass and the meter.
fn later_wgsl() -> String {
    let f = |v: f64| format!("{:?}", v as f32);
    let bits = crate::look::shader_features().camera_bits;
    format!(
        "// (camera track, round 2) the later features\nconst SF_CAM_TONE: bool = {};\nconst SF_CAM_WB: bool = {};\nconst CAM_TONE_NORM_MAX: f32 = {};\nconst CAM_TONE_SAT: vec3<f32> = vec3<f32>({}, {}, {});\nconst CAM_TONE_TOE_DESAT: vec2<f32> = vec2<f32>({}, {});\nconst CAM_TONE_SHOULDER_DESAT: vec2<f32> = vec2<f32>({}, {});\nconst CAM_TONE_GAMUT_KNEE: f32 = {};\nconst CAM_TONE_WHITEN_EV: vec2<f32> = vec2<f32>({}, {});\nconst CAM_TONE_SKY_METER: f32 = {};\nconst CAM_TONE_SUN_PULL: f32 = {};\nconst CAM_TONE_SUN_EV: vec2<f32> = vec2<f32>({}, {});\nconst CAM_TONE_BILATERAL_SHARE: f32 = {};\nconst CAM_WB_EV: vec2<f32> = vec2<f32>({}, {});\nconst CAM_WB_CHROMA: vec2<f32> = vec2<f32>({}, {});\nconst CAM_WB_TINT_LIMIT: f32 = {};\n",
        bits & CAM_BIT_TONE != 0,
        bits & CAM_BIT_WB != 0,
        f(CAM_TONE_NORM_MAX),
        f(CAM_TONE_SAT[0]),
        f(CAM_TONE_SAT[1]),
        f(CAM_TONE_SAT[2]),
        f(CAM_TONE_TOE_DESAT[0]),
        f(CAM_TONE_TOE_DESAT[1]),
        f(CAM_TONE_SHOULDER_DESAT[0]),
        f(CAM_TONE_SHOULDER_DESAT[1]),
        f(CAM_TONE_GAMUT_KNEE),
        f(CAM_TONE_WHITEN_EV[0]),
        f(CAM_TONE_WHITEN_EV[1]),
        f(CAM_TONE_SKY_METER),
        f(CAM_TONE_SUN_PULL),
        f(CAM_TONE_SUN_EV[0]),
        f(CAM_TONE_SUN_EV[1]),
        f(CAM_TONE_BILATERAL_SHARE),
        f(CAM_WB_EV[0]),
        f(CAM_WB_EV[1]),
        f(CAM_WB_CHROMA[0]),
        f(CAM_WB_CHROMA[1]),
        f(CAM_WB_LIMIT[1]),
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
            // the meter's histogram and the frame's plain one, then (`--white-balance`) the
            // balance's estimate (red, green, blue, weight).
            hist: buffer("cine-camera-hist", (2 * CAM_HIST_BINS as u64 + 4) * 4),
            state: buffer("cine-camera-state", CAM_STATE_BYTES),
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
        let bytes = n * CAM_GRID_Z as u64 * (8 + 8) + n * 8 + (2 * CAM_HIST_BINS as u64 + 4) * 4 + CAM_STATE_BYTES;
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
        // (`--cine-camera`) the camera's keys for its curve; (`--white-balance`) with the one grade's balances.
        let wb = crate::look::shader_features().camera_bits & CAM_BIT_WB != 0;
        let cam_keys = if !camera_in {
            Vec::new()
        } else if wb {
            CAM_GRADE_KEYS.iter().map(|k| cam_bake_lut(&show_key(k))).collect()
        } else {
            CAM_GRADE_KEYS.iter().map(cam_bake_lut).collect()
        };
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
        let w = grade::key_weights(sun_alt_deg, hour, overcast);
        let mut u = if camera { CineUniform::with_camera(sun_alt_deg) } else { CineUniform::new() };
        if camera && crate::look::shader_features().camera_bits & CAM_BIT_WB != 0 {
            // (`--white-balance`) the hour's preset, in the scene before the curve.
            let p = wb_preset(&w);
            u.cam7 = [p[0] as f32, p[1] as f32, p[2] as f32, wb_preset_sat(&w) as f32];
        }
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
        queue.write_buffer(&self.photo, 0, bytemuck::bytes_of(&photo.copied().unwrap_or_default()));
        if let Some(c) = &self.camera {
            // a change of model bakes the table again (its keys are the model's).
            if c.on.swap(camera, std::sync::atomic::Ordering::Relaxed) != camera {
                *self.last.lock().unwrap() = None;
            }
        }
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
        for bits in [0, CAM_BIT_TONE, CAM_BIT_WB, CAM_BIT_HALATION, CAM_BITS_ALL] {
        let on = crate::look::ShaderFeatures { cinematic: true, camera: true, camera_bits: bits, ..crate::look::ShaderFeatures::NONE };
        crate::look::with_shader_features(on, || {
            let m = validate("look_cine.wgsl + camera", &shader_source());
            let (_, ty) = m.types.iter().find(|(_, t)| t.name.as_deref() == Some("Cine")).expect("Cine struct");
            let naga::TypeInner::Struct { span, .. } = &ty.inner else { panic!("Cine is a struct") };
            assert_eq!(*span as usize, std::mem::size_of::<CineUniform>());
            let (_, ty) = m.types.iter().find(|(_, t)| t.name.as_deref() == Some("CamState")).expect("CamState struct");
            let naga::TypeInner::Struct { span, .. } = &ty.inner else { panic!("CamState is a struct") };
            assert_eq!(*span as u64, CAM_STATE_BYTES, "the state buffer's size");
            let c = validate("cine_camera.wgsl", &camera_compute_source());
            for entry in ["cam_build", "cam_adapt", "cam_blur"] {
                assert!(c.entry_points.iter().any(|e| e.name == entry), "{entry}");
            }
            // the resolve leaves the distances in its alpha for the depth of field.
            let r = format!("{}\n{}", crate::look::look_common_source(), include_str!("sky_resolve.wgsl"));
            assert!(r.contains("const SF_CAMERA: bool = true;"));
            validate("sky_resolve.wgsl + camera", &r);
            // (round 2) each later feature's switch follows its bit, in the pass, the meter and the bloom.
            let post = shader_source();
            assert!(post.contains(&format!("const SF_CAM_TONE: bool = {};", bits & CAM_BIT_TONE != 0)));
            assert!(post.contains(&format!("const SF_CAM_WB: bool = {};", bits & CAM_BIT_WB != 0)));
            assert!(post.contains(&format!("const SF_CAM_HALATION: bool = {};", bits & CAM_BIT_HALATION != 0)));
            assert!(camera_compute_source().contains(&format!("const SF_CAM_WB: bool = {};", bits & CAM_BIT_WB != 0)));
            let src = format!("{}\n{}\n{}", include_str!("sky_common.wgsl"), bloom_wgsl(true), include_str!("sky_bloom.wgsl"));
            assert!(src.contains(&format!("const SF_CAM_HALATION: bool = {};", bits & CAM_BIT_HALATION != 0)));
            validate("sky_bloom.wgsl + camera", &src);
        });
        }
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
        // the night keeps more of its shadows' depth, gives its lamps more room and raises less detail.
        assert!(n.shadow_room_ev > d.shadow_room_ev && n.highlight_room_ev > d.highlight_room_ev && n.detail < d.detail);
        for p in [n, d] {
            assert!(p.shadow_contrast > 0.5 && p.shadow_contrast <= 1.0 && p.highlight_contrast > 0.5 && p.highlight_contrast <= 1.0);
            assert!(p.detail >= 1.0 && p.detail < 1.5);
        }
        assert_eq!(camera_params(f64::NAN), d);
        let u = CineUniform::with_camera(30.0);
        assert_eq!(u.cam0[3], 1.0);
        assert_eq!(u.cam3[2], d.detail as f32);
        assert_eq!(u.cam2[3], CAM_LATERAL_CA as f32);
        assert!(CAM_RANGE_SHARE[0] < 0.1 && CAM_RANGE_SHARE[1] > 0.9 && CAM_TOP_SHARE > CAM_RANGE_SHARE[1]);
        assert_eq!(CineUniform::new().cam0[3], 0.0);
        // the night pushes its highlights and shows more grain; the day only compresses
        // them over a knee, and cuts a sunlit frame little.
        assert!(n.highlight_expand > 1.0 && d.highlight_expand == 1.0);
        assert!(d.knee_ev > n.knee_ev && d.knee_ev < d.highlight_room_ev && n.top_ev > n.highlight_room_ev);
        assert!(n.grain > d.grain && n.max_cut_ev > d.max_cut_ev);
        // the lens opens up at night (a shallower focus), stops down by day.
        assert!(n.f_number < d.f_number && n.f_number >= 1.4 && d.f_number <= 8.0);
        assert_eq!(u.cam4[3], d.f_number as f32);
        assert_eq!(u.cam3[3], d.knee_ev as f32);
        assert_eq!(u.cam4[2], CAM_GLARE_VEIL as f32);
    }

    /// (`--cine-camera`) the camera's response passes through middle grey with the
    /// mid-tones' contrast, rises everywhere, and rolls the shadows off slowly
    /// (a long toe, not a crush): 6 EV under grey lands about where 4.1 EV under
    /// would at the mid-tones' contrast (sRGB 6, not 0).
    #[test]
    fn the_camera_models_toe_is_long() {
        assert!(tone_log(0.0).abs() < 1e-9);
        let slope = |u: f64| (tone_log(u + 1e-4) - tone_log(u - 1e-4)) / 2e-4;
        assert!((slope(0.0) - CAM_CONTRAST).abs() < 0.01, "{}", slope(0.0));
        assert!((slope(-2.0) - CAM_CONTRAST).abs() < 0.1, "{}", slope(-2.0));
        assert!((slope(4.0) - CAM_CONTRAST).abs() < 0.01);
        assert!((slope(-9.0) - CAM_TOE[0]).abs() < 0.02);
        let mut last = f64::NEG_INFINITY;
        for i in -120..80 {
            let v = tone_log(i as f64 * 0.1);
            assert!(v > last);
            last = v;
        }
        assert!(tone_log(-6.0) > -6.0 && tone_log(-6.0) < -4.0, "{}", tone_log(-6.0));
        assert!(tone_log(-1.0) < -1.0);
    }

    /// (`--cine-camera`) the camera's grade leaves the toe to the curve (a dark
    /// grey is not crushed by the key's S-curve), keeps black and white, and keeps
    /// the golden hour's warm highlight balance off a blue sky but not off a sunlit wall.
    #[test]
    fn the_camera_grade_spares_the_toe_and_the_sky() {
        let night = &CAM_GRADE_KEYS[0];
        let dark = [0.05, 0.05, 0.05];
        let g = cam_grade(dark, night);
        let y = 0.2126 * g[0] + 0.7152 * g[1] + 0.0722 * g[2];
        assert!(y > 0.044, "{g:?}");
        let crushed = grade::grade(dark, night);
        assert!(y > 0.2126 * crushed[0] + 0.7152 * crushed[1] + 0.0722 * crushed[2]);
        for k in &CAM_GRADE_KEYS {
            assert!(cam_grade([0.0; 3], k).iter().all(|v| v.abs() < 1e-9));
        }
        let pm = &CAM_GRADE_KEYS[3];
        let sky = [0.62, 0.72, 0.86];
        let wall = [0.80, 0.70, 0.58];
        assert!(sky_share(sky) > 0.9 && sky_share(wall) == 0.0);
        let gs = cam_grade(sky, pm);
        let warm = grade::grade(sky, &grade::GradeKey { contrast: 1.0, ..*pm });
        assert!(gs[2] - gs[0] > warm[2] - warm[0], "{gs:?} {warm:?}");
        assert_eq!(cam_bake_lut(pm).len(), grade::LUT_SIZE.pow(3));
    }

    /// (`--camera-tone`) the display transform: grey where the model's curve put it,
    /// rising with the scene, never past 1 in any channel; a colour's hue kept (its
    /// channels' order and their ratios in the mid-tones); a bright golden sky's red
    /// runs up to 1 without a plateau while it whitens; far over white it is white; a
    /// black's colour rolls off.
    #[test]
    fn the_camera_tone_keeps_hue_and_runs_to_white() {
        let lw = [0.2126, 0.7152, 0.0722];
        let y = |c: [f64; 3]| c[0] * lw[0] + c[1] * lw[1] + c[2] * lw[2];
        let g = display([0.18; 3]);
        assert!((g[0] - g[1]).abs() < 1e-9 && (g[1] - g[2]).abs() < 1e-9);
        assert!((g[0] - sigmoid_display(0.18)).abs() < 1e-9 && g[0] > 0.12 && g[0] < 0.26, "{g:?}");
        let mut last = -1.0;
        for i in -100..90 {
            let v = 0.18 * 2f64.powf(i as f64 * 0.1);
            let d = display([v; 3]);
            assert!(d[1] >= last - 1e-12, "{v}");
            last = d[1];
        }
        // the mid-tones: an orange's ratios kept (x the saturation, about its luminance).
        let o = [0.30, 0.18, 0.08];
        let d = display(o);
        assert!(d[0] > d[1] && d[1] > d[2]);
        let r = |c: [f64; 3]| (c[1] - c[2]) / (c[0] - c[2]);
        assert!((r(d) - r(o)).abs() < 0.02, "{:?} {}", d, r(o));
        // a golden sky 1 .. 6 EV over grey: every channel under 1, red rising until it whitens,
        // no flat run of red at 1 while green and blue rise under it.
        let sky = [1.0, 0.74, 0.44];
        let mut reds = Vec::new();
        let mut plateau = 0;
        for i in 0..=50 {
            let k = 0.18 * 2f64.powf(1.0 + i as f64 * 0.1) / y(sky);
            let d = display(sky.map(|v| v * k));
            assert!(d.iter().all(|v| (0.0..=1.0).contains(v)), "{d:?}");
            assert!(d[0] >= d[1] - 1e-9 && d[1] >= d[2] - 1e-9, "the hue's order is kept: {d:?}");
            reds.push(d[0]);
            if d[0] > 0.995 && d[2] < 0.95 {
                plateau += 1;
            }
        }
        assert!(reds.windows(2).all(|p| p[1] >= p[0] - 1e-9), "{reds:?}");
        assert_eq!(plateau, 0, "{reds:?}");
        let hot = display(sky.map(|v| v * 1e4));
        assert!(hot.iter().all(|v| *v > 0.97), "{hot:?}");
        // a deep shadow's colour rolls off toward neutral (less chroma than its ratios gave).
        let navy = [0.004, 0.006, 0.010];
        let d = display(navy);
        let t = sigmoid_display(navy[0] + (navy[2] - y(navy)) * CAM_TONE_NORM_MAX + y(navy) * (1.0 - CAM_TONE_NORM_MAX));
        let kept = navy.map(|v| v * t / (y(navy) + (navy[2] - y(navy)) * CAM_TONE_NORM_MAX));
        assert!((d[2] - d[0]) < (kept[2] - kept[0]), "{d:?} {kept:?}");
    }

    /// (`--camera-tone`) the meter and the local exposure: by day a higher key and
    /// more lift, a lower knee and a standing compression; the night's push and its
    /// shadows left alone; the blue hour darker than the model's. Without the bit the
    /// uniform is the model's, bit for bit.
    #[test]
    fn the_camera_tone_params_hold() {
        let (d, n, b) = (tone_params(30.0), tone_params(-20.0), tone_params(-5.0));
        let (d0, n0, b0) = (camera_params(30.0), camera_params(-20.0), camera_params(-5.0));
        assert!(d.compensation_ev > d0.compensation_ev && d.max_lift_ev > d0.max_lift_ev && d.knee_ev < d0.knee_ev);
        assert!(d.base_shadow < 1.0 && d.base_highlight < d.base_shadow && d.base_highlight > 0.5);
        assert_eq!((n.base_shadow, n.base_highlight), (1.0, 1.0));
        assert!((n.compensation_ev - n0.compensation_ev).abs() < 1e-9 && n.highlight_expand == n0.highlight_expand);
        assert!(b.compensation_ev < b0.compensation_ev, "{} {}", b.compensation_ev, b0.compensation_ev);
        assert!(n.grain > d.grain && d.grain < d0.grain);
        assert_eq!((d0.base_shadow, d0.base_highlight), (1.0, 1.0));
        let u0 = CineUniform::with_camera_bits(30.0, 0);
        assert_eq!(u0, CineUniform { cam5: [0.0; 4], cam6: [0.0; 4], ..CineUniform::with_camera_bits(30.0, CAM_BIT_HALATION) });
        assert_eq!((u0.cam5, u0.cam6), ([0.0; 4], [0.0; 4]));
        assert_eq!(u0.cam1[2], d0.compensation_ev as f32);
        let ut = CineUniform::with_camera_bits(30.0, CAM_BIT_TONE);
        assert_eq!(ut.cam1[2], d.compensation_ev as f32);
        assert_eq!(ut.cam5[0], d.base_shadow as f32);
        assert!(ut.cam5[3] > u0.cam5[3] && ut.cam5[3] <= 1.0);
        let uw = CineUniform::with_camera_bits(30.0, CAM_BIT_WB);
        assert_eq!(uw.cam6, [CAM_WB_DEGREE[1][0] as f32, CAM_WB_DEGREE[1][1] as f32, CAM_WB_TINT_DEGREE as f32, CAM_WB_LIMIT[0] as f32]);
        let un = CineUniform::with_camera_bits(-20.0, CAM_BIT_WB);
        assert_eq!((un.cam6[0], un.cam6[1]), (CAM_WB_DEGREE[0][0] as f32, CAM_WB_DEGREE[0][1] as f32));
        assert!(CAM_WB_DEGREE.iter().flatten().all(|v| *v > 0.0 && *v < 1.0) && CAM_WB_LIMIT[1] < CAM_WB_LIMIT[0]);
        assert!(CAM_WB_DEGREE[1][0] < CAM_WB_DEGREE[1][1], "by day a warm light keeps more of its colour than a cool one");
        assert!(CAM_WB_DEGREE[0][1] < CAM_WB_DEGREE[1][1], "the twilight's and the night's blue stay");
    }

    /// (`--white-balance`) the camera's grade: every hour's key keeps its curve and
    /// takes the same near-neutral balances; a grey stays near grey; a bright warm
    /// colour is never pushed to a plateau at 1. The hour's colour moves into the
    /// white balance's preset: warm at golden hour and at night, cool at the blue
    /// hour, near neutral by day, at unit luminance and about half the hour's tint.
    #[test]
    fn the_show_grade_is_one_and_near_neutral() {
        for k in &CAM_GRADE_KEYS {
            let s = show_key(k);
            assert_eq!((s.contrast, s.pivot), (k.contrast, k.pivot));
            assert!(s.highlight_balance.iter().all(|v| *v <= 1.0));
            for v in [0.15, 0.4, 0.7] {
                let g = cam_grade([v; 3], &s);
                assert!((g[0] - g[2]).abs() < 0.04, "{} {v}: {g:?}", k.name);
            }
            for c in [[0.97, 0.85, 0.64], [0.92, 0.80, 0.60], [0.85, 0.74, 0.56]] {
                let g = cam_grade(c, &s);
                assert!(g[0] < 0.995, "{} {c:?}: {g:?}", k.name);
            }
        }
        let one = |i: usize| {
            let mut w = [0.0; grade::KEY_COUNT];
            w[i] = 1.0;
            wb_preset(&w)
        };
        let y = |p: [f64; 3]| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
        let (night, blue, pm, day) = (one(0), one(1), one(3), one(4));
        for p in [night, blue, pm, day] {
            assert!((y(p) - 1.0).abs() < 1e-9);
        }
        assert!(pm[0] > pm[2] && blue[2] > blue[0], "{pm:?} {blue:?}");
        let rb = |p: [f64; 3]| (p[0] / p[2]).ln();
        assert!(rb(day).abs() < rb(pm).abs() && rb(night).abs() < rb(pm).abs());
        let mut wd = [0.0; grade::KEY_COUNT];
        wd[1] = 1.0;
        assert!((wb_preset_sat(&wd) - 0.5 * (CAM_GRADE_KEYS[1].sat_shadow + CAM_GRADE_KEYS[1].sat_highlight)).abs() < 1e-12);
        let k = &CAM_GRADE_KEYS[3];
        let cast = |c: usize| (k.shadow_balance[c] * k.highlight_balance[c]).sqrt() * k.gain[c];
        assert!((rb(pm) - CAM_SHOW_KEEP * (cast(0) / cast(2)).ln()).abs() < 1e-9);
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
