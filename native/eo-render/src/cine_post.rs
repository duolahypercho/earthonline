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
//! only with it and `--cine-camera`: `--camera-tone` (the model's meter and
//! curve, the hour's grade open at the top and brought into the display's
//! gamut by a soft edge instead of clipped a channel at a time, golden keys
//! that split warm light from cooler shade, a veiling-glare floor, a grain of
//! luminance on the sensor's gain), `--white-balance` (a grey-world balance on
//! the frame's surfaces, the hour's tints halved over it: `show_key`),
//! `--lens-halation` (the weather's water widens the bloom's halo:
//! `cam_halation`), `--camera-sharpen` (a stronger robust sharpen and a 3 x 3
//! unsharp band held in its neighbourhood's range), `--lens-flare` (the meter
//! lists the frame's point sources; the post pass draws their diffraction
//! spikes, look_cine_flare.wgsl; the sun's ghosts stronger).

use bytemuck::{Pod, Zeroable};
use eo_look::cinematic::grade;
use std::sync::Mutex;

/// The pass's uniform (look_cine.wgsl `Cine`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, Default, PartialEq)]
pub struct CineUniform {
    /// x the bloom's share of the bright part; y 1 where photo mode added the
    /// bloom before this pass (its bind groups, `bind_group_over`; the test is
    /// compiled out with `--no-photo-mode`: `SF_PHOTO_MODE`); zw unused.
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
    /// x (`--camera-tone`) the lens's veiling glare floor (a share of the metered mean),
    /// z the shade's chroma kept (`CAM_TONE_SHADE_SAT`); w (`--camera-sharpen`) the
    /// sharpen's strength (of RCAS's limit), and its unsharp mask's amount in y. Zero
    /// without the features.
    pub cam5: [f32; 4],
    /// (`--white-balance`) the degrees of adaptation along the colour temperature's
    /// axis (x a warm light, y a cool one) and the green-magenta axis (z), w the
    /// temperature's largest correction (log2 a channel). Zero without the feature.
    pub cam6: [f32; 4],
    /// (`--white-balance`) xyz the hour's white-balance preset (`wb_preset`), w the
    /// hour's saturation in the scene (`wb_preset_sat`); zero without the feature.
    pub cam7: [f32; 4],
    /// (`--camera-print`) x the toe's slope in the deep shadows (`print_toe`), y its
    /// curve's constant (`toe_offset_at`); zw unused. Zero without the feature.
    pub cam8: [f32; 4],
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
            cam8: [0.0; 4],
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
            u.cam5[0] = (CAM_TONE_FLARE[0] + (CAM_TONE_FLARE[1] - CAM_TONE_FLARE[0]) * day) as f32;
            u.cam5[2] = (CAM_TONE_SHADE_SAT[0] + (CAM_TONE_SHADE_SAT[1] - CAM_TONE_SHADE_SAT[0]) * day) as f32;
        }
        if bits & CAM_BIT_PRINT != 0 {
            let day = day_share(sun_alt_deg);
            let lo = print_toe(sun_alt_deg);
            u.cam8 = [lo as f32, toe_offset_at(lo) as f32, 0.0, 0.0];
            u.cam3[2] = (p.detail + (CAM_PRINT_DETAIL_DAY - camera_params(90.0).detail) * day) as f32;
            // the veiling glare's floor fades out by day with the print's toe (a coated prime's
            // veil is under the print's black by day; at night it keeps an unlit facade's trace).
            u.cam5[0] *= (1.0 - day) as f32;
        }
        if bits & CAM_BIT_SHARPEN != 0 {
            u.cam5[1] = CAM_SHARPEN_DETAIL as f32;
            u.cam5[3] = 2f64.powf(-CAM_SHARPEN_STRONG_STOPS) as f32;
        }
        if bits & CAM_BIT_WB != 0 {
            let d = day_share(sun_alt_deg);
            let [n, dd] = CAM_WB_DEGREE;
            let warm = wb_warm_degree(sun_alt_deg, eo_look::features::daylight().wb_high_sun);
            u.cam6 = [(n[0] + (warm - n[0]) * d) as f32, (n[1] + (dd[1] - n[1]) * d) as f32, CAM_WB_TINT_DEGREE as f32, CAM_WB_LIMIT[0] as f32];
        }
        u
    }
}

// ---------------------------------------------------------------------------
// (camera track, round 2) the camera model's later features. Each has its own
// switch and bit (`SkyOptions::cam_bits` -> `ShaderFeatures::camera_bits`,
// `SF_CAM_*`), compiled in only with it and with `--cine-camera`; without its
// bit the model draws exactly as before.

/// `--camera-tone`: the camera's tone kept as the model meters and curves it
/// (its exposure, local exposure and per-channel sigmoid are the model's), with
/// the hour's grade made safe at the top and split by the light:
/// - the grade's table is baked open (`cam_grade_open`: its colour is not
///   clamped at 1) and its result is mapped into the display's gamut after the
///   table (`gamut`, `cam_gamut`): a channel past `CAM_TONE_GAMUT[0]` (display
///   linear) is brought toward 1 along a rational knee that reaches it at
///   `CAM_TONE_GAMUT[1]`, part of the excess taken by darkening the colour
///   (`CAM_TONE_GAMUT[2]` of it in log, as much as the colour is saturated: a
///   neutral white is never darkened) and the rest by desaturating it toward its
///   own luminance; so a golden sky or a sunlit wall keeps its hue and a
///   gradient where the display-space warm balance pushed its red to a flat 255
///   plateau, and a grey runs to white unchanged;
/// - the highlights' warm balance held to just under white (`CAM_TONE_WARM_TOP`,
///   eo-look's grade fades it from 0.92): a golden highlight stays golden up to
///   the gamut's edge instead of a cream plateau;
/// - the shade's colour toward neutral in the scene, before the curve
///   (`CAM_TONE_SHADE_SAT` of its chroma from `CAM_TONE_SHADE_EV[0]` EV under grey,
///   full again at `[1]`; by the hour: by day and through the golden hour, not at
///   night): the sky's blue in an open shade and a deck's orange bounce both read
///   near grey, so a golden frame splits warm light from a neutral shade instead
///   of one orange cast (`CAM_TONE_GRADE_KEYS`: by day the shadows' cool balance
///   is dropped too: a shaded street is not navy);
/// - the lens's veiling glare as a floor under the frame (`CAM_TONE_FLARE`: a
///   share of the metered mean, neutral), so a night's unlit facade keeps a
///   trace of detail instead of a void;
/// - by day the highlights' room 2.5 EV over grey (was 2.8): a frame whose sky
///   gap overflows brings it down a little further;
/// - a grain of luminance only that follows the sensor's gain (the meter's lift).
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
/// `--camera-sharpen`: the resolved frame sharpened as a camera's processing
/// does after its demosaic: the robust contrast-adaptive sharpen at
/// `CAM_SHARPEN_STRONG_STOPS` under its limit (the model's: 0.5) and, on top,
/// an unsharp mask of the 3 x 3 neighbourhood's luminance (`CAM_SHARPEN_DETAIL`
/// of the pixel's difference from its binomial blur), held inside the
/// neighbourhood's range widened by `CAM_SHARPEN_OVERSHOOT` of it (no halo,
/// no ringing), off in flat gradients as the sharpen is and where the focus blurs.
pub const CAM_BIT_SHARPEN: u32 = 8;
/// `--lens-flare`: the lens's diffraction and its inter-reflections from every
/// bright source, not only the sun: the meter finds the frame's point sources
/// (in each grid cell its brightest 4 x 4 block over `CAM_FLARE_DETECT` exposed,
/// refined to its brightest pixel, kept only where that pixel is the brightest
/// within 5 px and stands `CAM_FLARE_POINT` over a ring `CAM_FLARE_RING_PX` round
/// it, or `CAM_FLARE_ORB` x that over a ring twice as wide (a near lamp's globe in
/// its glow): a lamp, a headlamp, a glint, not a lit window, not the sun's disc)
/// into a list of at most `CAM_FLARE_SOURCES`, each with its flux over
/// `CAM_FLARE_FLUX_FLOOR` (exposed; growing only logarithmically past
/// `CAM_FLARE_FLUX_CAP`) and its colour; the post pass draws each one's diffraction
/// spikes: six (a hexagonal aperture's three edge pairs, turned
/// `CAM_FLARE_ANGLE_DEG`), a pixel wide at the core and fanning out
/// (`CAM_FLARE_WIDTH`), their intensity `CAM_FLARE_SPIKE[0]` x the flux at the core
/// falling as a straight edge's diffraction, (r0 / (r0 + r))^2
/// (`CAM_FLARE_SPIKE[1..3]`), each colour's spike as long as its wavelength
/// (`CAM_FLARE_DISPERSION`: a spike's tip goes red), drawn out to where it falls
/// under `CAM_FLARE_SPIKE[3]` (a length that grows as the square root of the flux,
/// at most `CAM_FLARE_SPIKE_MAX_PX`; a source whose spikes would be shorter than
/// `CAM_FLARE_SPIKE_MIN_PX` draws none) and fading over its last 60 %; the sun's
/// ghost chain is scaled by `CAM_FLARE_GHOST_GAIN` and its own soft starburst gives
/// way to the spikes. (Changed in round 2 iteration 3: the spikes were drawn to
/// 360 px at a 1.7 power, on far lights more than on a near lamp's globe, and the
/// ghosts 2.2 x the model's.)
pub const CAM_BIT_FLARE: u32 = 16;
/// `--camera-resolve`: the temporal resolve (sky_resolve.wgsl, `SF_CAM_RESOLVE`)
/// accumulates the frames in the unjittered image: the history is fetched where
/// the pixel's point was without this frame's sub-pixel jitter (the camera's
/// motion alone; the model's resolve fetched it at the point's unjittered place
/// but wrote it back at the jittered pixel, so every frame resampled the history
/// half a pixel off at random and the fine detail went soft); the first frames
/// after a cut averaged evenly (1 / n); a still pixel's clipping box widened
/// (`CAM_RESOLVE_GAMMA`) so the jittered coverage of an edge or a wire thinner
/// than a pixel accumulates instead of being clipped back to whichever side
/// this frame's sample fell on. Its code is in the resolve's shader only
/// (`SF_CAM_RESOLVE`, ShaderFeatures' constants); no uniform term.
pub const CAM_BIT_RESOLVE: u32 = 32;
/// `--camera-print`: by day the camera's response takes a print's black: the
/// toe's slope in the deep shadows (`CAM_TOE[0]`, a cine camera's long toe, kept
/// at night) runs to `CAM_PRINT_TOE_DAY` by day (blended over the sun's altitude as
/// the model's terms are), so a doorway, the gap under a car or a deep reveal goes
/// to black over its last stops instead of resting on a milky floor (5 and 6 EV
/// under grey at sRGB 4 and 1 instead of 10 and 6, 4 EV under at 12 instead of 17;
/// 2 and 3 EV under within 2 levels); the lens's veiling
/// floor (`--camera-tone`) fades out by day with it; and the local exposure's detail
/// against the grid's edge-aware base is raised to `CAM_PRINT_DETAIL_DAY` by day (a
/// print's local contrast: mortar, reveals and the dark of a ledge's underside read);
/// the golden hours' grade keeps no cool push in the shadows (`print_key`: the camera
/// balanced for daylight; the shade's colour is the scene's light).
pub const CAM_BIT_PRINT: u32 = 64;
pub const CAM_BITS_ALL: u32 = CAM_BIT_TONE | CAM_BIT_WB | CAM_BIT_HALATION | CAM_BIT_SHARPEN | CAM_BIT_FLARE | CAM_BIT_RESOLVE | CAM_BIT_PRINT;

/// (`--camera-tone`) the display gamut's soft edge (display linear): the knee a
/// channel is held under, where the rational curve reaches 1, and the share of
/// the excess (in log, x the colour's saturation) taken by darkening rather than
/// desaturating.
pub const CAM_TONE_GAMUT: [f64; 3] = [0.85, 1.6, 0.8];
/// (`--camera-tone`) the highlights' warm balance fades out over this display
/// luma (eo-look's grade: 0.92 .. 1).
pub const CAM_TONE_WARM_TOP: [f64; 2] = [0.97, 1.0];
/// (`--camera-tone`) the shade's chroma kept (at night, by day) at
/// `CAM_TONE_SHADE_EV[0]` EV about middle grey and under it, all of it from `[1]`.
pub const CAM_TONE_SHADE_SAT: [f64; 2] = [1.0, 0.7];
pub const CAM_TONE_SHADE_EV: [f64; 2] = [-4.0, -1.2];
/// (`--camera-tone`) the lens's veiling glare as a floor: this share of the
/// metered mean (exposed) under every pixel, at night and by day (a coated
/// prime's 0.5-1.5 % veiling glare index).
pub const CAM_TONE_FLARE: [f64; 2] = [0.015, 0.004];
/// (`--camera-tone`) the grain's standard deviation at its peak (display), night
/// and day, before the sensor's gain (x sqrt of the meter's lift, clamped to
/// 0.5 .. 3); luminance only.
pub const CAM_TONE_GRAIN: [f64; 2] = [0.0055, 0.0035];
/// (`--camera-tone`) the highlights' room by day (EV over grey; the model's 2.8).
pub const CAM_TONE_HIGHLIGHT_ROOM_EV: f64 = 2.5;

/// (`--camera-sharpen`) the robust sharpen's strength in stops under its limit
/// (the model's `CAM_SHARPEN_STOPS` is 0.5).
pub const CAM_SHARPEN_STRONG_STOPS: f64 = 0.15;
/// (`--camera-sharpen`) the unsharp mask's amount on the 3 x 3 binomial band of
/// the range-compressed luminance, and how far past the neighbourhood's range it
/// may take the pixel (a share of the range).
pub const CAM_SHARPEN_DETAIL: f64 = 0.65;
pub const CAM_SHARPEN_OVERSHOOT: f64 = 0.08;

/// (`--lens-flare`) the most point sources the meter keeps a frame.
pub const CAM_FLARE_SOURCES: u32 = 64;
/// (`--lens-flare`) a cell's brightest 4 x 4 block counts as a source over this
/// mean exposed luminance (raised so wet metal hydrants and sign poles do not
/// trigger artificial 6-ray diffraction starbursts, only true point lamps).
pub const CAM_FLARE_DETECT: f64 = 7.5;
/// (`--lens-flare`) a source's brightest pixel stands at least this many times
/// over the mean of a ring `CAM_FLARE_RING_PX` round it (a point or a lamp's globe,
/// not an area of light such as a lit window), or `CAM_FLARE_ORB` times that over a
/// ring twice as wide (a near lamp's globe in its own glow, wider than the first
/// ring: the brightest lamp in the frame was missed and the small far ones drew).
pub const CAM_FLARE_POINT: f64 = 2.5;
pub const CAM_FLARE_RING_PX: f64 = 18.0;
pub const CAM_FLARE_ORB: f64 = 2.0;
/// (`--lens-flare`) the part of each pixel's exposed luminance over this floor
/// counts toward the source's flux; past `CAM_FLARE_FLUX_CAP` it grows only
/// logarithmically (the sun's disc).
pub const CAM_FLARE_FLUX_FLOOR: f64 = 1.0;
pub const CAM_FLARE_FLUX_CAP: f64 = 400.0;
/// (`--lens-flare`) the spikes: x their intensity at the core per unit of flux,
/// y the falloff's power, z its scale r0 (px of a 1080-line frame), w the
/// intensity (exposed) they are drawn down to.
/// The falloff's power is a straight edge's diffraction (intensity ~ 1/r^2 along the
/// spike), so a spike is visible out to a length that grows as the square root of
/// its source's flux: a brighter lamp's spikes are longer, a dim one's short or none.
pub const CAM_FLARE_SPIKE: [f64; 4] = [0.0022, 2.0, 2.4, 0.003];
/// (`--lens-flare`) the drawn length's cap and floor (px of a 1080-line frame): no
/// spike crosses a third of the frame; a source whose spikes would be shorter than
/// the floor draws none (a faint far light keeps its round bloom).
pub const CAM_FLARE_SPIKE_MAX_PX: f64 = 90.0;
pub const CAM_FLARE_SPIKE_MIN_PX: f64 = 12.0;
/// (`--lens-flare`) a spike's width (px of a 1080-line frame) at its core and its
/// growth along it (px per px): it fans out softly and its energy along a unit of
/// length is kept.
pub const CAM_FLARE_WIDTH: [f64; 2] = [1.0, 0.012];
/// (`--lens-flare`) each colour's spike length against green's (its wavelength's).
pub const CAM_FLARE_DISPERSION: [f64; 3] = [1.12, 1.0, 0.86];
/// (`--lens-flare`) the spikes' turn from the frame's horizontal (degrees).
pub const CAM_FLARE_ANGLE_DEG: f64 = 15.0;
/// (`--lens-flare`) the sun's ghosts' gain over the model's (a coated prime's ghosts
/// are faint: at 2.2 a stray hexagon read as a smear on the frame).
pub const CAM_FLARE_GHOST_GAIN: f64 = 0.8;
/// (`--white-balance`) the degree of adaptation along the colour temperature's
/// axis (log2 of blue over red) when the frame's light is warm and when it is
/// cool, at night and by day (`day_share` blends them): a camera's automatic
/// balance that keeps most of a golden hour's warmth (as a camera's "keep warm
/// atmosphere" does) and takes half the sky's blue out of a shaded street by day,
/// and at twilight and night leaves most of the light's colour (the blue hour
/// stays blue) while taking a quarter of a sodium street's orange out.
pub const CAM_WB_DEGREE: [[f64; 2]; 2] = [[0.25, 0.20], [0.15, 0.50]];
/// (`--white-balance`, `--wb-high-sun`) the degree a warm light takes by day under a
/// high sun: a camera's daylight balance renders sunlit grey neutral at midday (the
/// matched CC0 photograph of Mission and 3rd Streets, 1 August 2011, 12:06: sunlit
/// asphalt blue/red 0.93-1.00 in display-linear), where `CAM_WB_DEGREE`'s day 0.15
/// left the sun's 5200 K beam's asphalt at 0.64 on Folsom Street at noon (play mode,
/// with the measured aerosol). The whole cast: 0.6 gave 0.75, 0.8 0.79, 1.0 0.86
/// (0.93 a little further down the street). The warmth a golden hour keeps is a low sun's.
pub const CAM_WB_HIGH_SUN_DEGREE: f64 = 1.0;
/// (`--wb-high-sun`) the sun's altitudes (degrees) between which the warm degree moves
/// from `CAM_WB_DEGREE`'s day value to `CAM_WB_HIGH_SUN_DEGREE` (smoothstep): under 30
/// degrees the afternoon's and the golden hour's warmth is kept as before.
pub const CAM_WB_HIGH_SUN_DEG: [f64; 2] = [30.0, 50.0];
/// (`--white-balance`) the degree along the green-magenta axis (green over red and blue).
pub const CAM_WB_TINT_DEGREE: f64 = 0.35;

/// (`--white-balance`) the day's degree for a warm light at the sun's altitude:
/// `CAM_WB_DEGREE`'s day value, moved to `CAM_WB_HIGH_SUN_DEGREE` over
/// `CAM_WB_HIGH_SUN_DEG` with `high_sun` (`--wb-high-sun`; off: the day value).
pub fn wb_warm_degree(sun_alt_deg: f64, high_sun: bool) -> f64 {
    let day = CAM_WB_DEGREE[1][0];
    if !high_sun || sun_alt_deg.is_nan() {
        return day;
    }
    let t = eo_look::math::smoothstep(CAM_WB_HIGH_SUN_DEG[0], CAM_WB_HIGH_SUN_DEG[1], sun_alt_deg);
    day + (CAM_WB_HIGH_SUN_DEGREE - day) * t
}
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
pub const CAM_HAL_GAIN: [f64; 2] = [0.22, 0.45];
/// (`--lens-halation`) the bloom's cap on a source's energy (`CAM_BLOOM_CAP`)
/// rises by this x h: a lamp's light spreads cleanly in mist/rain without blowing out into a fog ball.
pub const CAM_HAL_CAP: f64 = 0.8;

/// The day's share over the sun's altitude (0 night, 1 day; -9 .. +2 deg).
pub fn day_share(sun_alt_deg: f64) -> f64 {
    let a = if sun_alt_deg.is_nan() { 90.0 } else { sun_alt_deg };
    let t = ((a + 9.0) / 11.0).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// (`--camera-tone`) the meter and the local exposure by the hour: the model's
/// (`camera_params`), the highlights' room a little lower by day
/// (`CAM_TONE_HIGHLIGHT_ROOM_EV`), the grain by `CAM_TONE_GRAIN`.
pub fn tone_params(sun_alt_deg: f64) -> CameraParams {
    let day = day_share(sun_alt_deg);
    let mix = |n: f64, d: f64| n + (d - n) * day;
    let base = camera_params(sun_alt_deg);
    CameraParams { highlight_room_ev: mix(base.highlight_room_ev, CAM_TONE_HIGHLIGHT_ROOM_EV).min(base.highlight_room_ev), grain: mix(CAM_TONE_GRAIN[0], CAM_TONE_GRAIN[1]), ..base }
}

/// (`--camera-tone`) the hour's keys under the camera's tone (`CAM_GRADE_KEYS`):
/// the model's, but by day the shadows' cool balance is neutral (the shade's
/// colour is the scene's, toward neutral by `CAM_TONE_SHADE_SAT`).
pub const CAM_TONE_GRADE_KEYS: [grade::GradeKey; grade::KEY_COUNT] = [
    CAM_GRADE_KEYS[0],
    CAM_GRADE_KEYS[1],
    CAM_GRADE_KEYS[2],
    CAM_GRADE_KEYS[3],
    grade::GradeKey { name: "cam-tone-day", shadow_balance: [1.0, 1.0, 1.0], ..CAM_GRADE_KEYS[4] },
    CAM_GRADE_KEYS[5],
];
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

/// (`--camera-tone`) eo-look's grade's colour (saturation, the balances, the
/// gain) on a display colour, open at the top: nothing is clamped at 1 (the
/// gamut's soft edge, `gamut`, takes it after the table), only at 0.
fn grade_colour_open(s: [f64; 3], k: &grade::GradeKey) -> [f64; 3] {
    use eo_look::math::{mix, smoothstep};
    let luma = |c: [f64; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let norm = |g: [f64; 3]| {
        let l = luma(g).max(1e-6);
        g.map(|v| v / l)
    };
    let yl = luma(s);
    let sat = mix(k.sat_shadow, k.sat_highlight, smoothstep(0.05, 0.6, yl));
    let s = s.map(|v| yl + (v - yl) * sat);
    let ws = 1.0 - smoothstep(0.0, k.split, yl);
    let wh = smoothstep(k.split, 1.0, yl) * (1.0 - smoothstep(CAM_TONE_WARM_TOP[0], CAM_TONE_WARM_TOP[1], yl));
    let (sb, hb, gn) = (norm(k.shadow_balance), norm(k.highlight_balance), norm(k.gain));
    [0, 1, 2].map(|i| (s[i] * mix(1.0, sb[i], ws) * mix(1.0, hb[i], wh) * gn[i]).max(0.0))
}

/// (`--camera-tone`) the camera's grade open at the top (`cam_grade` with
/// `grade_colour_open`): the key's S-curve over the mid-tones, then its colour,
/// its highlights' warm balance kept off a blue sky; a warm highlight may pass 1.
pub fn cam_grade_open(s: [f64; 3], k: &grade::GradeKey) -> [f64; 3] {
    use eo_look::math::{mix, smoothstep};
    let s = s.map(|v| v.clamp(0.0, 1.0));
    let y = 0.2126 * s[0] + 0.7152 * s[1] + 0.0722 * s[2];
    let yc = mix(y, s_curve(y, k.pivot, k.contrast), smoothstep(k.pivot - 0.2, k.pivot + 0.1, y));
    let s = if yc <= y {
        let g = if y > 1e-9 { yc / y } else { 1.0 };
        s.map(|v| v * g)
    } else {
        let g = if y < 1.0 - 1e-9 { (1.0 - yc) / (1.0 - y) } else { 1.0 };
        s.map(|v| 1.0 - (1.0 - v) * g)
    };
    let g = grade_colour_open(s, k);
    let w = sky_share(s);
    if w <= 0.0 {
        return g;
    }
    let sky = grade_colour_open(s, &grade::GradeKey { highlight_balance: [1.0, 1.0, 1.0], ..*k });
    [0, 1, 2].map(|i| mix(g[i], sky[i], w))
}

/// (`--camera-tone`) a key's open table (`cam_grade_open`), as `grade::bake_lut` lays one out.
pub fn cam_bake_lut_open(k: &grade::GradeKey) -> Vec<[f32; 3]> {
    let n = grade::LUT_SIZE;
    let step = 1.0 / (n - 1) as f64;
    let mut out = Vec::with_capacity(n * n * n);
    for b in 0..n {
        for g in 0..n {
            for r in 0..n {
                out.push(cam_grade_open([r as f64 * step, g as f64 * step, b as f64 * step], k).map(|v| v as f32));
            }
        }
    }
    out
}

/// (`--camera-tone`) the gamut's rational knee: `x` over `knee` brought under 1,
/// reaching it at `lim`.
pub fn gamut_knee(x: f64, knee: f64, lim: f64) -> f64 {
    if x <= knee {
        return x;
    }
    let s = (1.0 - knee) * (lim - knee) / ((lim - knee) - (1.0 - knee));
    knee + (x - knee) / (1.0 + (x - knee) / s)
}

/// (`--camera-tone`) the display gamut's soft edge (CPU reference of
/// look_cine_camera.wgsl `cam_gamut`; display linear in and out): a colour whose
/// brightest channel passes the knee (`CAM_TONE_GAMUT`) is brought under it by
/// darkening (its share of the excess x the colour's saturation: a grey is never
/// darkened) and then desaturating toward its luminance; a colour inside is left
/// alone; a luminance over 1 is white.
pub fn gamut(c: [f64; 3]) -> [f64; 3] {
    let [knee, lim, dark] = CAM_TONE_GAMUT;
    let luma = |c: [f64; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let c = c.map(|v| v.max(0.0));
    let mx = c[0].max(c[1]).max(c[2]);
    if mx <= knee {
        return c;
    }
    let mn = c[0].min(c[1]).min(c[2]);
    let t = gamut_knee(mx, knee, lim);
    let c = c.map(|v| v * (t / mx).powf(dark * (mx - mn) / mx));
    let l = luma(c);
    let mx = c[0].max(c[1]).max(c[2]);
    if l >= t {
        return [l.min(1.0); 3];
    }
    let k = if mx > l + 1e-12 { ((t - l) / (mx - l)).min(1.0) } else { 1.0 };
    c.map(|v| (l + (v - l) * k).clamp(0.0, 1.0))
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
pub const CAM_LATERAL_CA: f64 = 0.35;
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
pub const CAM_F_NUMBER: [f64; 2] = [5.6, 8.0];
pub const CAM_FOCUS_RATE: f64 = 4.0;
pub const CAM_DOF_TAPS: u32 = 16;
pub const CAM_DOF_MAX_PX: f64 = 2.5;

/// The sun's glare (starburst and ghosts) at full strength, exposed units.
pub const CAM_FLARE_GAIN: f64 = 0.65;
/// The sun's veiling glare (the lens's scatter round the source: a core and a
/// compact 1/r^2 skirt), exposed units at its core with the sun fully visible.
pub const CAM_GLARE_VEIL: f64 = 0.75;
/// The sun's measured energy (mean exposed luminance over its disc) where its
/// glare starts and where it is full: log2.
pub const CAM_FLARE_EV: [f64; 2] = [2.5, 6.0];

/// The bloom: a tight optical PSF so normal sunlit walls and clouds stay sharp
/// while genuinely bright light sources glow cleanly without milky blur halos.
pub const CAM_BLOOM_VEIL: f64 = 0.0018;
pub const CAM_BLOOM_MIST: f64 = 0.022;
pub const CAM_BLOOM_MIST_EV: [f64; 2] = [0.8, 3.0];
pub const CAM_BLOOM_CAP: f64 = 24.0;
/// The levels' weights (half resolution to 1/128), summing to 1 and weighted toward
/// tight inner octaves so highlights glow crisply rather than smearing across the frame.
pub const CAM_BLOOM_WEIGHTS: [f64; crate::sky::CAM_BLOOM_LEVELS] = [0.42, 0.27, 0.15, 0.08, 0.04, 0.025, 0.015];

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
    tone_log_at(u, CAM_TOE[0])
}

/// (`--camera-print`) the toe's constant for a deep-shadow slope `lo` (`toe_offset`'s).
pub fn toe_offset_at(lo: f64) -> f64 {
    let sp = |x: f64| x.max(0.0) + (-x.abs()).exp().ln_1p();
    let [_, u0, w] = CAM_TOE;
    (CAM_CONTRAST - lo) * w * sp(-u0 / w)
}

/// The camera's response with the deep shadows' slope `lo` (`tone_log` at `CAM_TOE[0]`).
pub fn tone_log_at(u: f64, lo: f64) -> f64 {
    let sp = |x: f64| x.max(0.0) + (-x.abs()).exp().ln_1p();
    let [_, u0, w] = CAM_TOE;
    lo * u + (CAM_CONTRAST - lo) * w * sp((u - u0) / w) - toe_offset_at(lo)
}

/// (`--camera-print`) the toe's slope by day (the print's black; the night keeps the
/// cine camera's long toe, `CAM_TOE[0]`) and the detail's gain by day (the model's 1.22).
pub const CAM_PRINT_TOE_DAY: f64 = 0.85;
pub const CAM_PRINT_DETAIL_DAY: f64 = 1.35;

/// (`--camera-print`) a camera key under the print: the golden hours' shadow balance
/// neutral (the camera balanced for daylight, as the day's key under `--camera-tone`:
/// the shade shows the scene's own light, its sky blue and its warm bounce, instead of a
/// cool push over the whole shaded street); every other term the key's.
pub fn print_key(k: &grade::GradeKey) -> grade::GradeKey {
    if k.name.contains("golden") { grade::GradeKey { shadow_balance: [1.0; 3], ..*k } } else { *k }
}

/// (`--camera-print`) the toe's slope at the sun's altitude.
pub fn print_toe(sun_alt_deg: f64) -> f64 {
    CAM_TOE[0] + (CAM_PRINT_TOE_DAY - CAM_TOE[0]) * day_share(sun_alt_deg)
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
    // x (`--camera-tone`) the veiling glare's floor (a share of the metered mean); y (`--camera-sharpen`)
    // the unsharp mask's amount; z (`--camera-tone`) the shade's chroma kept; w (`--camera-sharpen`) the
    // sharpen's strength
    cam5: vec4<f32>,
    // (`--white-balance`) the adaptation's degree: x a warm light's temperature, y a cool one's, z the tint; w the
    // temperature's largest correction (log2)
    cam6: vec4<f32>,
    // (`--white-balance`) xyz the hour's white-balance preset, w the hour's saturation in the scene (0: none)
    cam7: vec4<f32>,
    // (`--camera-print`) x the toe's slope in the deep shadows, y its curve's constant; zw unused
    cam8: vec4<f32>,
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
    let mut o = format!(
        "// (camera track, round 2) the later features\nconst SF_CAM_TONE: bool = {};\nconst SF_CAM_WB: bool = {};\nconst SF_CAM_SHARPEN: bool = {};\nconst SF_CAM_FLARE: bool = {};\nconst SF_CAM_PRINT: bool = {};\nconst CAM_TONE_GAMUT: vec3<f32> = vec3<f32>({}, {}, {});\nconst CAM_WB_EV: vec2<f32> = vec2<f32>({}, {});\nconst CAM_WB_CHROMA: vec2<f32> = vec2<f32>({}, {});\nconst CAM_WB_TINT_LIMIT: f32 = {};\nconst CAM_SHARPEN_OVERSHOOT: f32 = {};\nconst CAM_TONE_SHADE_EV: vec2<f32> = vec2<f32>({}, {});\n",
        bits & CAM_BIT_TONE != 0,
        bits & CAM_BIT_WB != 0,
        bits & CAM_BIT_SHARPEN != 0,
        bits & CAM_BIT_FLARE != 0,
        bits & CAM_BIT_PRINT != 0,
        f(CAM_TONE_GAMUT[0]),
        f(CAM_TONE_GAMUT[1]),
        f(CAM_TONE_GAMUT[2]),
        f(CAM_WB_EV[0]),
        f(CAM_WB_EV[1]),
        f(CAM_WB_CHROMA[0]),
        f(CAM_WB_CHROMA[1]),
        f(CAM_WB_LIMIT[1]),
        f(CAM_SHARPEN_OVERSHOOT),
        f(CAM_TONE_SHADE_EV[0]),
        f(CAM_TONE_SHADE_EV[1]),
    );
    o += &format!(
        "const CAM_FLARE_SOURCES: u32 = {}u;\nconst CAM_FLARE_DETECT: f32 = {};\nconst CAM_FLARE_POINT: f32 = {};\nconst CAM_FLARE_RING_PX: f32 = {};\nconst CAM_FLARE_ORB: f32 = {};\nconst CAM_FLARE_FLUX_FLOOR: f32 = {};\nconst CAM_FLARE_FLUX_CAP: f32 = {};\nconst CAM_FLARE_SPIKE: vec4<f32> = vec4<f32>({}, {}, {}, {});\nconst CAM_FLARE_SPIKE_MAX_PX: f32 = {};\nconst CAM_FLARE_SPIKE_MIN_PX: f32 = {};\nconst CAM_FLARE_WIDTH: vec2<f32> = vec2<f32>({}, {});\nconst CAM_FLARE_DISPERSION: vec3<f32> = vec3<f32>({}, {}, {});\nconst CAM_FLARE_ANGLE: f32 = {};\nconst CAM_FLARE_GHOST_GAIN: f32 = {};\n{}",
        CAM_FLARE_SOURCES,
        f(CAM_FLARE_DETECT),
        f(CAM_FLARE_POINT),
        f(CAM_FLARE_RING_PX),
        f(CAM_FLARE_ORB),
        f(CAM_FLARE_FLUX_FLOOR),
        f(CAM_FLARE_FLUX_CAP),
        f(CAM_FLARE_SPIKE[0]),
        f(CAM_FLARE_SPIKE[1]),
        f(CAM_FLARE_SPIKE[2]),
        f(CAM_FLARE_SPIKE[3]),
        f(CAM_FLARE_SPIKE_MAX_PX),
        f(CAM_FLARE_SPIKE_MIN_PX),
        f(CAM_FLARE_WIDTH[0]),
        f(CAM_FLARE_WIDTH[1]),
        f(CAM_FLARE_DISPERSION[0]),
        f(CAM_FLARE_DISPERSION[1]),
        f(CAM_FLARE_DISPERSION[2]),
        f(CAM_FLARE_ANGLE_DEG.to_radians()),
        f(CAM_FLARE_GHOST_GAIN),
        CAM_STARS_STRUCT_WGSL,
    );
    o
}

/// (`--lens-flare`) the meter's list of the frame's point sources (cine_camera.wgsl
/// `cam_build` appends, the post pass reads): a count, then two vec4 a source: its
/// centre (px), its spikes' intensity at the core (exposed) and their drawn length
/// (px); its colour at unit luminance.
pub const CAM_STARS_STRUCT_WGSL: &str = "
struct CamStarsR {
    count: u32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
    s: array<vec4<f32>>,
};
";

/// (`--lens-flare`) the list's buffer: 16 bytes of count, two vec4 a source.
pub const CAM_STARS_BYTES: u64 = 16 + CAM_FLARE_SOURCES as u64 * 32;

/// The pass's whole shader: photo.rs's (the look's common source, the old
/// display transform's helpers, the photo finish's lens and noise), the
/// constants, and look_cine.wgsl; (`--cine-camera`, `SF_CAMERA`) with the
/// camera model's bindings and constants and the bloom's share.
pub fn shader_source() -> String {
    // (`--cine-camera`) the model's bindings, constants and fragment path only when it is
    // compiled in; else a stub the pass's dead `SF_CAMERA` branch names.
    let cam = if crate::look::shader_features().camera {
        format!(
            "{}{}\n@group(0) @binding(9) var<storage, read> cam_state: CamState;\n@group(0) @binding(10) var cam_grid: texture_3d<f32>;\n{}{}",
            camera_wgsl(),
            bloom_wgsl(true).replace("const SF_CAMERA: bool = true;\n", ""),
            // (`--lens-flare`) the meter's point sources and their spikes; a stub without it.
            if crate::look::shader_features().camera_bits & CAM_BIT_FLARE != 0 {
                format!("@group(0) @binding(11) var<storage, read> cam_stars: CamStarsR;\n{}", include_str!("look_cine_flare.wgsl"))
            } else {
                "\nfn cam_spikes(pos: vec2<f32>, dims: vec2<f32>) -> vec3<f32> {\n    return vec3<f32>(0.0);\n}\n".to_string()
            },
            include_str!("look_cine_camera.wgsl")
        )
    } else {
        "\nfn cine_camera(pos: vec2<f32>, dims: vec2<f32>) -> vec4<f32> {\n    return vec4<f32>(0.0);\n}\n".to_string()
    };
    format!("{}\n{}\n{}\n{}\n{}", crate::photo::shader_source(), wgsl(), CINE_STRUCT_WGSL, include_str!("look_cine.wgsl"), cam)
}

/// (`--cine-camera`) the meter's, the grid's and the sun's compute passes.
pub fn camera_compute_source() -> String {
    let stars = if crate::look::shader_features().camera_bits & CAM_BIT_FLARE != 0 { CAM_STARS_COMPUTE_WGSL } else { CAM_STARS_STUB_WGSL };
    format!("{}\n{}{}\n{}\n{}", crate::look::look_common_source(), CINE_STRUCT_WGSL, camera_wgsl(), stars, include_str!("cine_camera.wgsl"))
}

/// (`--lens-flare`) the meter's side of the source list: its binding and the append.
const CAM_STARS_COMPUTE_WGSL: &str = "
struct CamStarsW {
    count: atomic<u32>,
    pad0: u32,
    pad1: u32,
    pad2: u32,
    s: array<vec4<f32>>,
};
@group(0) @binding(10) var<storage, read_write> cam_stars: CamStarsW;

fn cam_star_push(a: vec4<f32>, b: vec4<f32>) {
    let i = atomicAdd(&cam_stars.count, 1u);
    if (i < CAM_FLARE_SOURCES) {
        cam_stars.s[2u * i] = a;
        cam_stars.s[2u * i + 1u] = b;
    }
}
";

/// Without `--lens-flare`: no list, the append a stub (`cam_build`'s call sits behind `SF_CAM_FLARE`).
const CAM_STARS_STUB_WGSL: &str = "
fn cam_star_push(a: vec4<f32>, b: vec4<f32>) {}
";

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
    /// the same, the bloom marked as added upstream (photo mode's bind groups).
    uniform_upstream: wgpu::Buffer,
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
    /// (`--lens-flare`) the frame's point sources (`CAM_STARS_BYTES`), made only with its bit.
    stars: Option<wgpu::Buffer>,
    /// the frame's size the grid was made for, its buffers, texture and the passes' bind groups per post source.
    sized: Mutex<Option<CameraSized>>,
    /// the model on this frame (`CineGpu::write`).
    on: std::sync::atomic::AtomicBool,
}

struct CameraSized {
    size: (u32, u32),
    columns: (u32, u32),
    binds: [wgpu::BindGroup; 3],
    grid_view: wgpu::TextureView,
    bytes: u64,
}

impl CameraGpu {
    fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("cine_camera.wgsl"), source: wgpu::ShaderSource::Wgsl(camera_compute_source().into()) });
        let cs = wgpu::ShaderStages::COMPUTE;
        let e = |binding, ty| wgpu::BindGroupLayoutEntry { binding, visibility: cs, ty, count: None };
        let ubuf = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None };
        let sbuf = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None };
        let flare = crate::look::shader_features().camera_bits & CAM_BIT_FLARE != 0;
        // the look, the sky, the frame, a linear sampler, the histogram, the state, the cells,
        // the columns, the grid texture, the pass's uniform; (`--lens-flare`) the source list.
        let mut entries = vec![
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
        ];
        if flare {
            entries.push(e(10, sbuf));
        }
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("cine-camera-bgl"), entries: &entries });
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
            stars: flare.then(|| buffer("cine-camera-stars", CAM_STARS_BYTES)),
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
            let mut entries = vec![
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
            ];
            if let Some(b) = &self.stars {
                entries.push(wgpu::BindGroupEntry { binding: 10, resource: b.as_entire_binding() });
            }
            device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("cine-camera-bg"), layout: &self.bgl, entries: &entries })
        });
        let bytes = n * CAM_GRID_Z as u64 * (8 + 8) + n * 8 + (2 * CAM_HIST_BINS as u64 + 4) * 4 + CAM_STATE_BYTES + self.stars.as_ref().map_or(0, |b| b.size());
        *g = Some(CameraSized { size, columns, binds, grid_view: grid_view.clone(), bytes });
        grid_view
    }

    fn grid_view(&self, device: &wgpu::Device, look: &wgpu::Buffer, src: &wgpu::TextureView, sky: &wgpu::Buffer, sampler: &wgpu::Sampler, cine: &wgpu::Buffer) -> wgpu::TextureView {
        if let Some(t) = self.sized.lock().unwrap().as_ref() {
            return t.grid_view.clone();
        }
        let tex = src.texture();
        self.ensure(device, (tex.width(), tex.height()), look, [src, src, src], sky, sampler, cine)
    }

    /// The meter's passes over post source `src` (before the post pass reads the meter).
    fn encode(&self, enc: &mut wgpu::CommandEncoder, src: usize) {
        if !self.on.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let g = self.sized.lock().unwrap();
        let Some(t) = g.as_ref() else { return };
        enc.clear_buffer(&self.hist, 0, None);
        if let Some(b) = &self.stars {
            enc.clear_buffer(b, 0, Some(16));
        }
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
            if crate::look::shader_features().camera_bits & CAM_BIT_FLARE != 0 {
                // (`--lens-flare`) the meter's point sources.
                entries.push(e(11, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }));
            }
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
        let uniform_upstream = buffer("cine-uniform-photo-mode", std::mem::size_of::<CineUniform>() as u64);
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
        // (`--cine-camera`) the camera's keys for its curve; (`--white-balance`) with the one grade's
        // balances; (`--camera-tone`) its own keys, the table open at the top.
        let bits = crate::look::shader_features().camera_bits;
        let (wb, tone, print) = (bits & CAM_BIT_WB != 0, bits & CAM_BIT_TONE != 0, bits & CAM_BIT_PRINT != 0);
        let cam_keys = if !camera_in {
            Vec::new()
        } else {
            let keys = if tone { &CAM_TONE_GRADE_KEYS } else { &CAM_GRADE_KEYS };
            // (`--camera-print`) the golden hours' shadows balanced as the day's (`print_key`).
            keys.iter().map(|k| if print { print_key(k) } else { *k }).map(|k| if wb { show_key(&k) } else { k }).map(|k| if tone { cam_bake_lut_open(&k) } else { cam_bake_lut(&k) }).collect()
        };
        let camera = camera_in.then(|| CameraGpu::new(device));
        Self { pipeline, bgl, uniform, uniform_upstream, photo, lut, lut_view, sampler, keys, cam_keys, last: Mutex::new(None), uploads: Default::default(), camera }
    }

    /// One bind group per post source (the HDR scene, the resolved history 0, 1).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn bind_groups(&self, device: &wgpu::Device, look: &wgpu::Buffer, srcs: [&wgpu::TextureView; 3], sky: &wgpu::Buffer, bloom: &wgpu::TextureView, sampler: &wgpu::Sampler) -> [wgpu::BindGroup; 3] {
        // (`--cine-camera`) the meter's grid for this frame size and its passes' bind groups.
        let grid = self.camera.as_ref().map(|c| {
            let t = srcs[0].texture();
            c.ensure(device, (t.width(), t.height()), look, srcs, sky, sampler, &self.uniform)
        });
        srcs.map(|src| self.bind_group_with(device, look, src, sky, bloom, sampler, false, grid.as_ref()))
    }

    /// A bind group over one source; `upstream`: the bloom was added before
    /// this pass (photo mode's output), which then adds none and keeps the
    /// pixel's bright part.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn bind_group_over(&self, device: &wgpu::Device, look: &wgpu::Buffer, src: &wgpu::TextureView, sky: &wgpu::Buffer, bloom: &wgpu::TextureView, sampler: &wgpu::Sampler, upstream: bool) -> wgpu::BindGroup {
        let grid = self.camera.as_ref().map(|c| c.grid_view(device, look, src, sky, sampler, &self.uniform));
        self.bind_group_with(device, look, src, sky, bloom, sampler, upstream, grid.as_ref())
    }

    #[allow(clippy::too_many_arguments)]
    fn bind_group_with(
        &self,
        device: &wgpu::Device,
        look: &wgpu::Buffer,
        src: &wgpu::TextureView,
        sky: &wgpu::Buffer,
        bloom: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        upstream: bool,
        grid: Option<&wgpu::TextureView>,
    ) -> wgpu::BindGroup {
        let mut entries = vec![
            wgpu::BindGroupEntry { binding: 0, resource: look.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(src) },
            wgpu::BindGroupEntry { binding: 2, resource: sky.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(bloom) },
            wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(sampler) },
            wgpu::BindGroupEntry { binding: 5, resource: self.photo.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&self.lut_view) },
            wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            wgpu::BindGroupEntry { binding: 8, resource: if upstream { &self.uniform_upstream } else { &self.uniform }.as_entire_binding() },
        ];
        if let (Some(c), Some(grid)) = (&self.camera, grid) {
            entries.push(wgpu::BindGroupEntry { binding: 9, resource: c.state.as_entire_binding() });
            entries.push(wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(grid) });
            if let Some(b) = &c.stars {
                entries.push(wgpu::BindGroupEntry { binding: 11, resource: b.as_entire_binding() });
            }
        }
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(if upstream { "cine-bg-photo-mode" } else { "cine-bg" }),
            layout: &self.bgl,
            entries: &entries,
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
        queue.write_buffer(&self.uniform_upstream, 0, bytemuck::bytes_of(&CineUniform { bloom: [u.bloom[0], 1.0, 0.0, 0.0], ..u }));
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
        n * n * n * 8 + self.uniform.size() + self.uniform_upstream.size() + self.photo.size() + cam
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::naga;

    /// (`--wb-high-sun`) a high sun's warmth is taken out, a low sun's kept as before.
    #[test]
    fn the_white_balance_neutralises_a_high_sun_and_keeps_a_low_ones_warmth() {
        let day = CAM_WB_DEGREE[1][0];
        for alt in [-5.0, 5.0, 15.0, 25.0, 30.0] {
            assert_eq!(wb_warm_degree(alt, true), day, "{alt}");
        }
        for alt in [50.0, 65.0, 80.0] {
            assert!((wb_warm_degree(alt, true) - CAM_WB_HIGH_SUN_DEGREE).abs() < 1e-12, "{alt}");
        }
        let mut last = day;
        for i in 0..=40 {
            let w = wb_warm_degree(30.0 + i as f64 * 0.5, true);
            assert!(w >= last && w <= CAM_WB_HIGH_SUN_DEGREE, "monotone");
            last = w;
        }
        // the switch off: the day's degree at every altitude
        for alt in [10.0, 40.0, 65.0] {
            assert_eq!(wb_warm_degree(alt, false), day);
        }
        // the uniform at a noon sun carries it (with the process's switch on)
        if eo_look::features::daylight().wb_high_sun {
            let u = CineUniform::with_camera_bits(65.0, CAM_BIT_WB);
            assert!((u.cam6[0] as f64 - CAM_WB_HIGH_SUN_DEGREE).abs() < 1e-3, "{:?}", u.cam6);
        }
    }

    #[test]
    fn the_shader_validates_and_its_uniform_matches() {
        // (with photo mode's bloom test compiled in and out: `SF_PHOTO_MODE`)
        for photo_mode in [true, false] {
            let f = crate::look::ShaderFeatures { photo_mode, ..crate::look::ShaderFeatures::ALL };
            crate::look::with_shader_features(f, || {
                let src = shader_source();
                assert!(src.contains(&format!("const SF_PHOTO_MODE: bool = {photo_mode};")));
                let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("look_cine.wgsl: {}", e.emit_to_string(&src)));
                naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).expect("look_cine.wgsl validates");
                let (_, ty) = module.types.iter().find(|(_, t)| t.name.as_deref() == Some("Cine")).expect("Cine struct");
                let naga::TypeInner::Struct { span, .. } = &ty.inner else { panic!("Cine is a struct") };
                assert_eq!(*span as usize, std::mem::size_of::<CineUniform>());
                for entry in ["photo_vs", "cine_fs"] {
                    assert!(module.entry_points.iter().any(|e| e.name == entry), "{entry}");
                }
            });
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
        for bits in [0, CAM_BIT_TONE, CAM_BIT_WB, CAM_BIT_HALATION, CAM_BIT_SHARPEN, CAM_BIT_FLARE, CAM_BIT_RESOLVE, CAM_BIT_PRINT, CAM_BITS_ALL] {
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
            // (`--camera-resolve`) its resolve follows its bit.
            assert!(r.contains(&format!("const SF_CAM_RESOLVE: bool = {};", bits & CAM_BIT_RESOLVE != 0)));
            validate("sky_resolve.wgsl + camera", &r);
            // (round 2) each later feature's switch follows its bit, in the pass, the meter and the bloom.
            let post = shader_source();
            assert!(post.contains(&format!("const SF_CAM_TONE: bool = {};", bits & CAM_BIT_TONE != 0)));
            assert!(post.contains(&format!("const SF_CAM_WB: bool = {};", bits & CAM_BIT_WB != 0)));
            assert!(post.contains(&format!("const SF_CAM_HALATION: bool = {};", bits & CAM_BIT_HALATION != 0)));
            assert!(camera_compute_source().contains(&format!("const SF_CAM_WB: bool = {};", bits & CAM_BIT_WB != 0)));
            assert!(post.contains(&format!("const SF_CAM_SHARPEN: bool = {};", bits & CAM_BIT_SHARPEN != 0)));
            assert!(post.contains(&format!("const SF_CAM_FLARE: bool = {};", bits & CAM_BIT_FLARE != 0)));
            assert!(post.contains(&format!("const SF_CAM_PRINT: bool = {};", bits & CAM_BIT_PRINT != 0)));
            // (`--lens-flare`) the source list is bound in the meter and the pass only with its bit.
            let flare = bits & CAM_BIT_FLARE != 0;
            assert_eq!(post.contains("var<storage, read> cam_stars"), flare);
            assert_eq!(camera_compute_source().contains("var<storage, read_write> cam_stars"), flare);
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

    /// (`--camera-print`) by day the deep shadows run to a print's black (the toe's
    /// slope near the mid-tones' contrast: 6 EV under grey a stop and more deeper than
    /// the long toe), the mid-shadows (2 EV under) within a tenth of a stop of the
    /// model's, the night's toe the model's; the detail is raised and the veiling floor
    /// gone by day only; without the bit the uniform is the model's.
    #[test]
    fn the_camera_print_deepens_the_days_black_only() {
        let (day, night) = (print_toe(30.0), print_toe(-20.0));
        assert!((day - CAM_PRINT_TOE_DAY).abs() < 1e-9 && (night - CAM_TOE[0]).abs() < 1e-9);
        assert!(tone_log_at(0.0, day).abs() < 1e-9 && (tone_log_at(-3.0, CAM_TOE[0]) - tone_log(-3.0)).abs() < 1e-12);
        assert!(tone_log_at(-6.0, day) < tone_log(-6.0) - 0.9, "{} {}", tone_log_at(-6.0, day), tone_log(-6.0));
        assert!((tone_log_at(-2.0, day) - tone_log(-2.0)).abs() < 0.1);
        let mut last = f64::NEG_INFINITY;
        for i in -120..80 {
            let v = tone_log_at(i as f64 * 0.1, day);
            assert!(v > last);
            last = v;
        }
        let base = CineUniform::with_camera_bits(30.0, CAM_BIT_TONE);
        let (pd, pn) = (CineUniform::with_camera_bits(30.0, CAM_BIT_TONE | CAM_BIT_PRINT), CineUniform::with_camera_bits(-20.0, CAM_BIT_TONE | CAM_BIT_PRINT));
        assert_eq!(base.cam8, [0.0; 4]);
        assert_eq!(pd.cam8, [day as f32, toe_offset_at(day) as f32, 0.0, 0.0]);
        assert_eq!((pd.cam3[2], pd.cam5[0]), (CAM_PRINT_DETAIL_DAY as f32, 0.0));
        let n0 = CineUniform::with_camera_bits(-20.0, CAM_BIT_TONE);
        assert_eq!((pn.cam3, pn.cam5), (n0.cam3, n0.cam5));
        assert_eq!(pn.cam8[0], CAM_TOE[0] as f32);
        assert_eq!(CineUniform { cam8: [0.0; 4], cam3: base.cam3, cam5: base.cam5, ..pd }, base);
        // the golden keys' shadows neutral, every other key and term as they were.
        for k in CAM_TONE_GRADE_KEYS.iter().chain(CAM_GRADE_KEYS.iter()) {
            let p = print_key(k);
            if k.name.contains("golden") {
                assert_eq!(p.shadow_balance, [1.0; 3]);
                assert_eq!(grade::GradeKey { shadow_balance: k.shadow_balance, ..p }, *k);
            } else {
                assert_eq!(p, *k);
            }
        }
        assert_eq!(CAM_GRADE_KEYS.iter().filter(|k| k.name.contains("golden")).count(), 2);
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

    /// (`--camera-tone`) the gamut's soft edge: a colour inside the knee and any grey are
    /// left alone; nothing passes 1; a golden highlight ramp through the hour's open grade
    /// keeps its hue's order and a rising red with no flat run at 1, where the model's
    /// clamped grade made one.
    #[test]
    fn the_camera_tone_gamut_keeps_hue_and_greys() {
        let [knee, lim, _] = CAM_TONE_GAMUT;
        assert!((gamut_knee(lim, knee, lim) - 1.0).abs() < 1e-9 && gamut_knee(0.5, knee, lim) == 0.5);
        for c in [[0.1, 0.4, 0.7], [0.79, 0.2, 0.05], [0.0, 0.0, 0.0]] {
            assert_eq!(gamut(c), c);
        }
        for v in [0.81, 0.9, 0.97, 1.0] {
            let g = gamut([v; 3]);
            assert!(g.iter().all(|x| (x - v).abs() < 1e-9), "{v} {g:?}");
        }
        let dec = |v: f64| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
        let pm = &CAM_TONE_GRADE_KEYS[3];
        let (mut reds, mut plateau, mut base_plateau) = (Vec::new(), 0, 0);
        for i in 0..=60 {
            let t = 0.55 + 0.45 * i as f64 / 60.0;
            let d = [1.0, 0.86, 0.66].map(|v| v * t);
            let o = gamut(cam_grade_open(d, pm).map(dec));
            assert!(o.iter().all(|v| (0.0..=1.0).contains(v)), "{o:?}");
            assert!(o[0] >= o[1] - 1e-9 && o[1] >= o[2] - 1e-9, "the hue's order: {o:?}");
            reds.push(o[0]);
            plateau += (o[0] > 0.995 && o[2] < 0.9) as usize;
            let b = cam_grade(d, &CAM_GRADE_KEYS[3]);
            base_plateau += (b[0] > 0.999 && b[2] < 0.9) as usize;
        }
        assert!(reds.windows(2).all(|p| p[1] >= p[0] - 1e-9), "{reds:?}");
        assert_eq!(plateau, 0, "{reds:?}");
        assert!(base_plateau > 5, "the model's clamped grade plateaus: {base_plateau}");
    }

    /// (`--camera-tone`) the keys are the model's but for the day's neutral shadow balance;
    /// the open grade keeps the golden hour's warm balance nearer white than the model's.
    #[test]
    fn the_camera_tone_keys_keep_the_hours_look() {
        for i in [0, 1, 2, 3, 5] {
            assert_eq!(CAM_TONE_GRADE_KEYS[i], CAM_GRADE_KEYS[i]);
        }
        let day = &CAM_TONE_GRADE_KEYS[4];
        assert_eq!(day.shadow_balance, [1.0; 3]);
        assert_eq!(grade::GradeKey { shadow_balance: CAM_GRADE_KEYS[4].shadow_balance, name: CAM_GRADE_KEYS[4].name, ..*day }, CAM_GRADE_KEYS[4]);
        let pm = &CAM_TONE_GRADE_KEYS[3];
        let w = cam_grade_open([0.95, 0.95, 0.95], pm);
        let b = cam_grade([0.95, 0.95, 0.95], pm);
        assert!(w[0] - w[2] > b[0] - b[2], "{w:?} {b:?}");
        assert_eq!(cam_bake_lut_open(day).len(), grade::LUT_SIZE.pow(3));
        assert!(CAM_TONE_SHADE_SAT[0] == 1.0 && CAM_TONE_SHADE_SAT[1] < 1.0 && CAM_TONE_SHADE_SAT[1] > 0.4);
        assert!(CAM_TONE_SHADE_EV[0] < CAM_TONE_SHADE_EV[1] && CAM_TONE_SHADE_EV[1] <= 0.0);
    }

    /// (`--camera-tone`, `--camera-sharpen`, `--lens-flare`) the terms: the tone keeps the
    /// model's meter but for a lower highlights' room by day, a veil floor stronger at night,
    /// a lighter grain; each bit sets only its own uniform terms; the spikes' list and
    /// length hold.
    #[test]
    fn the_camera_later_terms_hold() {
        let (d, n) = (tone_params(30.0), tone_params(-20.0));
        let (d0, n0) = (camera_params(30.0), camera_params(-20.0));
        assert!(d.highlight_room_ev < d0.highlight_room_ev && (d.highlight_room_ev - CAM_TONE_HIGHLIGHT_ROOM_EV).abs() < 1e-9);
        assert_eq!(n.highlight_room_ev, n0.highlight_room_ev);
        assert_eq!((d.compensation_ev, d.max_lift_ev, d.shadow_contrast), (d0.compensation_ev, d0.max_lift_ev, d0.shadow_contrast));
        assert!(n.grain > d.grain && d.grain < d0.grain);
        let u0 = CineUniform::with_camera_bits(30.0, 0);
        assert_eq!((u0.cam5, u0.cam6, u0.cam7), ([0.0; 4], [0.0; 4], [0.0; 4]));
        assert_eq!(u0, CineUniform::with_camera_bits(30.0, CAM_BIT_HALATION | CAM_BIT_FLARE));
        let (td, tn) = (CineUniform::with_camera_bits(30.0, CAM_BIT_TONE), CineUniform::with_camera_bits(-20.0, CAM_BIT_TONE));
        assert!(tn.cam5[0] > td.cam5[0] && td.cam5[0] > 0.0 && tn.cam5[0] < 0.03);
        assert!(tn.cam5[2] == 1.0 && (td.cam5[2] as f64 - CAM_TONE_SHADE_SAT[1]).abs() < 1e-6);
        assert_eq!((td.cam5[1], td.cam5[3]), (0.0, 0.0));
        assert_eq!(td.cam3[1], d.highlight_room_ev as f32);
        let us = CineUniform::with_camera_bits(30.0, CAM_BIT_SHARPEN);
        assert_eq!(us.cam5[0], 0.0);
        assert!(us.cam5[1] > 0.0 && us.cam5[3] > 2f64.powf(-CAM_SHARPEN_STOPS) as f32 && us.cam5[3] <= 1.0);
        let uw = CineUniform::with_camera_bits(30.0, CAM_BIT_WB);
        assert_eq!(uw.cam6, [CAM_WB_DEGREE[1][0] as f32, CAM_WB_DEGREE[1][1] as f32, CAM_WB_TINT_DEGREE as f32, CAM_WB_LIMIT[0] as f32]);
        assert_eq!(CAM_STARS_BYTES, 16 + 32 * CAM_FLARE_SOURCES as u64);
        // a spike's length grows as the square root of its source's flux (a straight edge's
        // 1/r^2 diffraction): the brightest sources reach the drawn cap (a fifth of the frame's
        // height at most, not a third), a lamp at the flux cap a few tens of pixels, and a
        // source whose spikes would be under the floor draws none.
        let [k, p, r0, eps] = CAM_FLARE_SPIKE;
        assert_eq!(p, 2.0);
        let len = |flux: f64| r0 * ((k * flux / eps).powf(1.0 / p) - 1.0) * CAM_FLARE_DISPERSION[0];
        let hot = len(CAM_FLARE_FLUX_CAP * (1.0 + (1e6f64 / CAM_FLARE_FLUX_CAP).ln()));
        assert!(hot > CAM_FLARE_SPIKE_MAX_PX && CAM_FLARE_SPIKE_MAX_PX <= 1080.0 / 5.0, "{hot}");
        let lamp = len(CAM_FLARE_FLUX_CAP);
        assert!(lamp > 30.0 && lamp < 0.6 * CAM_FLARE_SPIKE_MAX_PX, "{lamp}");
        assert!((len(4.0 * CAM_FLARE_FLUX_CAP) / lamp - 2.0).abs() < 0.2, "four times the flux, twice the length");
        assert!(len(2.0 * eps / k) < CAM_FLARE_SPIKE_MIN_PX && CAM_FLARE_ORB > 1.0 && CAM_FLARE_GHOST_GAIN < 1.0);
        assert!(CAM_FLARE_DISPERSION[0] > 1.0 && CAM_FLARE_DISPERSION[2] < 1.0 && CAM_FLARE_DETECT > CAM_FLARE_FLUX_FLOOR);
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
