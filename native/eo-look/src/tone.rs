//! The image half: three's `acesFilmicToneMapping` (r180, the version the
//! game pins) with `toneMappingExposure`, the post chain's highlight shoulder
//! before it, and after the sRGB encode the display-referred grade and the
//! look's finish (src/render/post-chain.js `gradeDisplayReferred`,
//! `applyLookFinish`). The WGSL in eo-render/src/look_post.wgsl mirrors these
//! CPU references; the tests compare these with the JS.

use crate::look::LOOK;
use crate::math::*;

/// the ACES 2.0 output transform (a port of the Academy's reference CTL).
pub mod aces2;
/// the camera look (`--no-camera-look`): white balance, ISO 12232 placement, ACES 2.0, baked to a table.
pub mod camera;

/// `RRTAndODTFit`.
pub fn rrt_odt_fit(x: f64) -> f64 {
    let a = x * (x + 0.0245786) - 0.000090537;
    let b = x * ((x + 0.4329510) * 0.983729) + 0.238081;
    a / b
}

const ACES_IN: [[f64; 3]; 3] = [[0.59719, 0.35458, 0.04823], [0.07600, 0.90834, 0.01566], [0.02840, 0.13383, 0.83777]];
const ACES_OUT: [[f64; 3]; 3] = [[1.60475, -0.53108, -0.07367], [-0.10208, 1.10813, -0.00605], [-0.00327, -0.07276, 1.07602]];

fn mul(m: &[[f64; 3]; 3], v: Rgb) -> Rgb {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// three's `acesFilmicToneMapping(color, exposure)`, clamped to 0..1.
pub fn aces_filmic(c: Rgb, exposure: f64) -> Rgb {
    let v = mul(&ACES_IN, scale3(c, exposure / 0.6));
    let v = [rrt_odt_fit(v[0]), rrt_odt_fit(v[1]), rrt_odt_fit(v[2])];
    let o = mul(&ACES_OUT, v);
    [clamp(o[0], 0.0, 1.0), clamp(o[1], 0.0, 1.0), clamp(o[2], 0.0, 1.0)]
}

/// `acesNeutralDisplay(v)`.
pub fn aces_neutral_display(v: f64) -> f64 {
    clamp(rrt_odt_fit(v.max(0.0)), 0.0, 1.0)
}

/// `acesNeutralInverse(display)`.
pub fn aces_neutral_inverse(display: f64) -> f64 {
    let d = clamp(display, 0.001, 0.995);
    let a = 1.0 - 0.983729 * d;
    let b = 0.0245786 - 0.425906 * d;
    let c = -(0.000090537 + 0.238081 * d);
    (-b + (b * b - 4.0 * a * c).sqrt()) / (2.0 * a)
}

/// `POST_CHAIN_DEFAULTS.highlight`: shoulder start (display) and width.
pub const HIGHLIGHT_START_DISPLAY: f64 = 0.85;
pub const HIGHLIGHT_SCALE: f64 = 1.0;

/// CSS `saturate()` luma weights the grade uses.
pub const CSS_SATURATE_LUMA: Rgb = [0.213, 0.715, 0.072];

/// `lookDensityCurve(x, density)`.
pub fn look_density_curve(x: f64, density: f64) -> f64 {
    let t = clamp((x - 0.25) / 0.75, 0.0, 1.0);
    let s = t * t * (3.0 - 2.0 * t);
    x * (1.0 - density * s * (1.0 - x))
}

/// The whole display transform on one pixel (CPU reference of look_post.wgsl,
/// no dither and no AA): shoulder, ACES, sRGB, grade, finish.
pub fn display(c: Rgb, exposure: f64, uv: [f64; 2]) -> Rgb {
    let start = aces_neutral_inverse(HIGHLIGHT_START_DISPLAY);
    let scaled = scale3(c, exposure / 0.6);
    let peak = scaled[0].max(scaled[1]).max(scaled[2]);
    let held = peak.max(start);
    let rolled = start + HIGHLIGHT_SCALE * ((held - start) / HIGHLIGHT_SCALE + 1.0).ln();
    let c = scale3(c, rolled / held);
    let t = aces_filmic(c, exposure);
    let s = [linear_to_srgb(t[0]), linear_to_srgb(t[1]), linear_to_srgb(t[2])];
    // grade
    let luma = s[0] * CSS_SATURATE_LUMA[0] + s[1] * CSS_SATURATE_LUMA[1] + s[2] * CSS_SATURATE_LUMA[2];
    let sat = [0, 1, 2].map(|i| clamp(mix(luma, s[i], LOOK.grade_saturation), 0.0, 1.0));
    let k = LOOK.grade_contrast;
    let con = sat.map(|x| if x < 0.5 { (x * 2.0).powf(k) * 0.5 } else { 1.0 - ((1.0 - x) * 2.0).powf(k) * 0.5 });
    let g = con.map(|x| clamp(x * LOOK.grade_brightness, 0.0, 1.0));
    // finish
    let d = LOOK.finish_density;
    let rgb = g.map(|x| x * (1.0 - d * smoothstep(0.25, 1.0, x) * (1.0 - x)));
    let l = rgb[0] * CSS_SATURATE_LUMA[0] + rgb[1] * CSS_SATURATE_LUMA[1] + rgb[2] * CSS_SATURATE_LUMA[2];
    let sw = smoothstep(0.0, 0.12, l) * (1.0 - smoothstep(0.12, LOOK.finish_balance, l));
    let hw = smoothstep(LOOK.finish_balance, 0.9, l) * (1.0 - smoothstep(0.96, 1.0, l));
    let toned = [0, 1, 2].map(|i| rgb[i] + LOOK.finish_shadow_tint[i] * sw + LOOK.finish_highlight_tint[i] * hw);
    let chroma = toned[0].max(toned[1]).max(toned[2]) - toned[0].min(toned[1]).min(toned[2]);
    let tl = toned[0] * CSS_SATURATE_LUMA[0] + toned[1] * CSS_SATURATE_LUMA[1] + toned[2] * CSS_SATURATE_LUMA[2];
    let amount = (1.0 - clamp(chroma, 0.0, 1.0)) * LOOK.finish_vibrance + 1.0;
    let cu = [uv[0] - 0.5, uv[1] - 0.5];
    let vig = 1.0 - smoothstep(0.12, 0.5, cu[0] * cu[0] + cu[1] * cu[1]) * LOOK.finish_vignette;
    toned.map(|x| clamp(mix(tl, x, amount) * vig, 0.0, 1.0))
}

// ---------------------------------------------------------------------------
// The photographic finish (`--photo-finish 1|2`, stage W4): a camera's
// response in place of the JS chain's per-channel ACES, its sensor's noise and
// its lens. eo-render look_photo.wgsl mirrors these; with the option off the
// post pass is the chain above, untouched.
//
// What a frame of the old chain gets wrong against photographs of the city
// (the reference sheet's section 5.10 notes): a bright coloured source clips
// to a pure hue (a sodium lamp stays orange to its core, a sunlit red wall
// keeps its red into the shoulder) where a camera's highlights go to white; the
// grade adds saturation on top of ACES's own (1.12 and a vibrance of 0.10);
// nothing is noisy at night; the lens is perfect.

/// The largest scaled value the neutral curve is evaluated at (a guard: the
/// shoulder's logarithm of an infinite pixel would be infinite, and the fit
/// infinite over infinite). The fit is within 1e-6 of its limit long before.
pub const NEUTRAL_MAX_SCALED: f64 = 1.0e6;

/// The old chain's display value (linear, 0..1) of a grey of scene value `x`
/// (radiance x exposure; photographic middle grey is 0.18): the highlight
/// shoulder and ACES's fit (the ACES matrices leave a grey alone). The camera
/// response keeps this neutral axis, so a grey lands exactly where it did and
/// the exposure the adaptation picks (stage B1) means what it meant: middle
/// grey 0.18 -> 0.213 linear, sRGB 127 before the grade.
pub fn neutral_display(x: f64) -> f64 {
    let start = aces_neutral_inverse(HIGHLIGHT_START_DISPLAY);
    let s = if x.is_nan() { 0.0 } else { (x.max(0.0) / 0.6).min(NEUTRAL_MAX_SCALED) };
    let s = if s > start { start + HIGHLIGHT_SCALE * ((s - start) / HIGHLIGHT_SCALE + 1.0).ln() } else { s };
    clamp(rrt_odt_fit(s), 0.0, 1.0)
}

/// The camera response (display linear, 0..1) of a colour of scene value `x`
/// (radiance x exposure). The brightest channel takes the neutral curve and
/// the colour keeps its channel ratios (its hue and saturation, as the old
/// shoulder's "one gain off the peak channel" did); then it moves straight
/// toward white, by as much as a per-channel curve lifts its least channel.
/// In the shoulder a per-channel film or sensor curve compresses the bright
/// channel while the dim ones still climb, so a bright colour pales toward
/// white; taken that way it also turns (orange to yellow, red to orange), so
/// here only its amount is taken and the colour moves on its own hue line: the
/// least channel lands where the per-channel curve puts it, the brightest stays
/// on the neutral curve, and (mid - min) / (max - min), the hue, is unchanged.
/// In the toe the per-channel curve would darken the dim channels (more
/// saturation): there nothing moves (the ratios are kept). A grey is the
/// neutral curve exactly. Monotone in brightness for every chromaticity; never
/// NaN (negative, NaN and infinite channels are read as 0, 0 and very bright).
/// The amount is taken with the least channel at least `SENSOR_CROSSTALK` of
/// the brightest, so a pure primary (an emissive signal lens, a primary-coloured
/// texel) pales at its core too. Last, where the per-channel curve lifts the
/// middle channel above this (in the shoulder), it takes `CAMERA_HUE_SKEW` of
/// the way there (held under the brightest): a bright warm colour turns a
/// little toward yellow, as a camera's does, and never to a primary or a
/// secondary; in the toe, where a per-channel curve would turn it toward the
/// primary, the hue is held.
pub fn camera_response(x: Rgb) -> Rgb {
    let x = x.map(|v| if v.is_nan() { 0.0 } else { v.clamp(0.0, 0.6 * NEUTRAL_MAX_SCALED) });
    let n = x[0].max(x[1]).max(x[2]);
    if n <= 1e-10 {
        return [0.0; 3];
    }
    let m = x[0].min(x[1]).min(x[2]);
    let t = neutral_display(n);
    let k = t / n;
    let qmin = m * k;
    let pmin = neutral_display(m.max(SENSOR_CROSSTALK * n));
    let w = clamp((pmin - qmin) / (t - qmin).max(1e-9), 0.0, 1.0);
    let h = x.map(|v| {
        let q = v * k;
        q + (t - q) * w
    });
    [0, 1, 2].map(|i| h[i] + (clamp(neutral_display(x[i]), h[i], t) - h[i]) * CAMERA_HUE_SKEW)
}

/// How far the middle channel goes toward the per-channel curve's (0: the
/// hue held exactly, 1: a per-channel curve's turn). A per-channel curve (a
/// camera's picture profile, film, the JS chain's ACES) turns a bright orange
/// toward yellow as its red channel saturates (the old chain moved a sunlit
/// cream wall at golden hour from hue 24 to 31 degrees); held exactly, the
/// warm-lit walls in stage W4's captures read salmon where photographs of the
/// city's walls in evening sun read golden. A third of the turn is kept: the
/// hue stays within a third of the way from where the light put it, nothing
/// runs to a primary or a secondary. Only the middle channel moves (the
/// brightest and the least stay), so the whitening above is unchanged.
pub const CAMERA_HUE_SKEW: f64 = 0.35;

/// A camera's colour filters overlap: no light reaches its sensor with one
/// channel at zero (a Bayer filter passes some per cent of the other bands,
/// and the colour matrix spreads it). For the amount of whitening only, the
/// least channel counts as at least this share of the brightest; a pure
/// primary at middle grey is whitened by under 1 %, at +10 EV it is near white.
pub const SENSOR_CROSSTALK: f64 = 0.05;

/// The photo finish's grade saturation (CSS `saturate`). The JS look's 1.12
/// sits on top of ACES's per-channel contrast, which already raises the
/// chroma of a mid-tone colour by about a tenth (a 2:1 colour at middle grey
/// comes out 2.23:1); the camera response keeps ratios, and this keeps them
/// too: a camera's standard picture profile renders the scene's own colour.
pub const PHOTO_GRADE_SATURATION: f64 = 1.0;
/// The photo finish's vibrance (the JS look's 0.10 lifts low-chroma colours:
/// the concrete, the asphalt and the haze; the "too clean, too saturated"
/// frames). None.
pub const PHOTO_VIBRANCE: f64 = 0.0;
/// The grade's brightness (`LOOK.grade_brightness`, 1.05) clipped everything
/// over display 0.952 to white: the top twentieth of the range flat. Over this
/// display value the gain rolls off instead (a quadratic that meets white only
/// where the old chain's own white, display 1.0, lands), so a sunlit wall or
/// a bright cloud keeps its gradation. Middle grey (0.52 there) is untouched.
pub const PHOTO_SOFT_WHITE_KNEE: f64 = 0.9;

/// The grade's brightness with the soft white: `y` (the contrast curve's
/// output times `top`, the brightness gain) -> 0..1; equal to `y` up to the
/// knee, 1 at `top` (white times the gain), slope 1 at the knee and still
/// positive at `top` (`top` under 2 - knee). A gain of 1 or less only clamps.
pub fn soft_white(y: f64, top: f64) -> f64 {
    let k = PHOTO_SOFT_WHITE_KNEE;
    if top <= 1.0 || y <= k {
        return clamp(y, 0.0, 1.0);
    }
    if y >= top {
        return 1.0;
    }
    let d = y - k;
    k + d - d * d * (top - 1.0) / ((top - k) * (top - k))
}

/// The share of the lens's natural fall-off (cos^4 of the field angle) a
/// phone's lens-shading correction leaves: 0.2. At the game's 52 deg vertical
/// field on a 16:9 frame the corners sit 45 deg off axis (cos^4 = 0.25), so
/// they come out 0.85 of the centre (-0.23 EV); natural vignetting uncorrected
/// would be -2 EV there, a phone's processed photo shows a fifth to a third of
/// a stop. Applied to the scene's light, before the curve, as a lens does.
pub const LENS_VIGNETTE_RESIDUAL: f64 = 0.2;
/// Lateral chromatic aberration left after the camera's correction, pixels of
/// red-to-blue separation at the frame's corner on a 1080-line frame (scaled
/// with the frame's height), growing as the square of the distance from the
/// centre, so the middle is clean and only the edges fringe: a phone's main
/// camera keeps a pixel or two of lateral colour at 12 MP after correction,
/// about half a pixel to a pixel once downsampled to 1080 lines. Red is
/// spread outward and blue inward by half of it each (a positive lateral
/// colour, the common sign of a short wide-angle lens).
pub const LENS_CA_CORNER_PX_1080: f64 = 1.1;

/// The lens's relative illumination at a point of the frame (`uv` 0..1 across
/// and down) for a field of view of `tan_half` (the tangents of the half field
/// across and down): 1 at the centre, 1 - residual x (1 - cos^4) off axis.
pub fn vignette_gain(uv: [f64; 2], tan_half: [f64; 2]) -> f64 {
    let (a, b) = ((uv[0] * 2.0 - 1.0) * tan_half[0], (uv[1] * 2.0 - 1.0) * tan_half[1]);
    let c2 = 1.0 / (1.0 + a * a + b * b);
    1.0 - LENS_VIGNETTE_RESIDUAL * (1.0 - c2 * c2)
}

/// The camera the frames are taken with, as most of the reference sheet's
/// recent street photos were (section 5: phone EXIF): an f/1.8 lens, a slowest
/// handheld shutter of 1/30 s, ISO 50 at base and 3200 at most. At full
/// aperture and the slowest shutter, ISO 100 meters EV100 log2(1.8^2 x 30) =
/// 6.6; the camera holds base ISO down to EV100 7.6 and raises the ISO a stop
/// for each EV below it: a lit street at night (EV100 3-6, sheet
/// section 3.8) is shot at ISO 150-1200, a clear day at base.
pub const CAMERA_F_NUMBER: f64 = 1.8;
pub const CAMERA_SLOWEST_S: f64 = 1.0 / 30.0;
pub const CAMERA_ISO_BASE: f64 = 50.0;
pub const CAMERA_ISO_MAX: f64 = 3200.0;

/// The ISO the camera picks for a scene of EV100 `ev` (ISO 100 = EV100's own
/// sensitivity): base until the slowest shutter at full aperture, then up.
pub fn camera_iso(ev100: f64) -> f64 {
    let limit = (CAMERA_F_NUMBER * CAMERA_F_NUMBER / CAMERA_SLOWEST_S).log2();
    let ev = if ev100.is_nan() { limit } else { ev100 };
    clamp(100.0 * (limit - ev).exp2(), CAMERA_ISO_BASE, CAMERA_ISO_MAX)
}

/// The noise left in a processed photo at middle grey and base ISO, as a
/// share of the signal (luminance, after the camera's denoising): 0.6 %, under
/// one 8-bit level (invisible by day). It grows as the square root of the gain
/// (photon shot noise: each stop of ISO halves the light collected), so a
/// night street at ISO 500-1000 shows 2-3 %, the soft fine grain of a phone's
/// night photo; the sheet's night photos (sections 5.8-5.9) are ISO 3200
/// frames of a dedicated camera, grainier still.
pub const GRAIN_MID_BASE: f64 = 0.006;
/// The read noise floor at base ISO in scene units (middle grey 0.18; a
/// phone's 2-3 electrons over about 1,000 at middle grey, after the same
/// denoising): it scales with the gain, so it shows only in the deep shadows
/// of a night frame (at 2.5e-4 a night sky came out 9 % noisy at 1:1, coarser
/// than a phone's night photo; 1.8e-4 gives 7 %).
pub const GRAIN_READ_BASE: f64 = 1.8e-4;
/// The share of the noise that differs between the channels (chroma noise;
/// phones suppress most of it): level 2 only.
pub const GRAIN_CHROMA_SHARE: f64 = 0.25;

/// The noise variance at scene value `x` (radiance x exposure) at ISO `iso`:
/// shot noise (variance proportional to the signal and to the gain) and the
/// read floor (its sigma proportional to the gain). Returns (the shot
/// coefficient, the read variance): variance = a x + b.
pub fn noise_terms(iso: f64) -> (f64, f64) {
    let g = (iso / CAMERA_ISO_BASE).max(1.0);
    let shot = GRAIN_MID_BASE * GRAIN_MID_BASE * 0.18;
    (g * shot, (g * GRAIN_READ_BASE) * (g * GRAIN_READ_BASE))
}

/// The noise sigma (scene units) at `x` and ISO `iso`.
pub fn noise_sigma(x: f64, iso: f64) -> f64 {
    let (a, b) = noise_terms(iso);
    (a * x.max(0.0) + b).sqrt()
}

/// The bloom of the photo finish (eo-render sky.rs, sky_bloom.wgsl): a lens's
/// glow round a bright source, not a veil. The JS chain's bloom is a bright
/// pass at the display's knee (scene value 2.1, +3.5 EV over middle grey) with
/// a hard 0.55 knee, each source texel held to 4 x the threshold (so a lamp a
/// few pixels across, hundreds of times the threshold, glows like a dull
/// patch), spread over five levels weighted 0.21-0.10 (most of it in the wide
/// levels: the milky ring round a sunlit cloud edge). A lens's point spread is
/// a tight core with a fading tail (a few per cent of the light past a few
/// pixels): the threshold here starts lower and softer, a small source keeps
/// more of its brightness, and the weight falls with each wider level.
/// Threshold (scene value, radiance x exposure: +3 EV over middle grey) and
/// knee (its width, a share of it).
pub const PHOTO_BLOOM_THRESHOLD_X: f64 = 1.44;
pub const PHOTO_BLOOM_KNEE: f64 = 1.0;
/// Each source texel held to this many times the threshold (the sun's disc
/// still cannot flood the frame: the prefilter also caps its result at 16 x).
pub const PHOTO_BLOOM_CEILING: f64 = 12.0;
/// The five levels' weights, half resolution to 1/32 (the first levels a
/// lamp's or a glint's own glow, the last a faint wide halo).
pub const PHOTO_BLOOM_WEIGHTS: [f64; 5] = [0.20, 0.12, 0.065, 0.03, 0.012];

/// The photo finish's whole display transform on one pixel, without the
/// lateral colour (a spatial effect), the noise and the dither: the lens's
/// fall-off, the camera response, sRGB, the grade (`PHOTO_GRADE_SATURATION`,
/// the look's contrast, its brightness with the soft white) and the look's
/// finish (density, split tone, `PHOTO_VIBRANCE`, the look's vignette). CPU
/// reference of eo-render look_photo.wgsl (`tan_half` the field of view's half
/// tangents across and down).
pub fn photo_display(c: Rgb, exposure: f64, uv: [f64; 2], tan_half: [f64; 2]) -> Rgb {
    let v = vignette_gain(uv, tan_half);
    let x = c.map(|ch| ch.max(0.0) * v * exposure);
    let t = camera_response(x);
    let s = t.map(linear_to_srgb);
    let luma = s[0] * CSS_SATURATE_LUMA[0] + s[1] * CSS_SATURATE_LUMA[1] + s[2] * CSS_SATURATE_LUMA[2];
    let sat = s.map(|x| clamp(mix(luma, x, PHOTO_GRADE_SATURATION), 0.0, 1.0));
    let k = LOOK.grade_contrast;
    let con = sat.map(|x| if x < 0.5 { (x * 2.0).powf(k) * 0.5 } else { 1.0 - ((1.0 - x) * 2.0).powf(k) * 0.5 });
    let g = con.map(|x| soft_white(x * LOOK.grade_brightness, LOOK.grade_brightness));
    finish(g, uv, PHOTO_VIBRANCE)
}

/// The look's finish on a graded display colour (density shoulder, split
/// tone, vibrance, the look's display vignette), as `display` does it.
fn finish(g: Rgb, uv: [f64; 2], vibrance: f64) -> Rgb {
    let d = LOOK.finish_density;
    let rgb = g.map(|x| x * (1.0 - d * smoothstep(0.25, 1.0, x) * (1.0 - x)));
    let l = rgb[0] * CSS_SATURATE_LUMA[0] + rgb[1] * CSS_SATURATE_LUMA[1] + rgb[2] * CSS_SATURATE_LUMA[2];
    let sw = smoothstep(0.0, 0.12, l) * (1.0 - smoothstep(0.12, LOOK.finish_balance, l));
    let hw = smoothstep(LOOK.finish_balance, 0.9, l) * (1.0 - smoothstep(0.96, 1.0, l));
    let toned = [0, 1, 2].map(|i| rgb[i] + LOOK.finish_shadow_tint[i] * sw + LOOK.finish_highlight_tint[i] * hw);
    let chroma = toned[0].max(toned[1]).max(toned[2]) - toned[0].min(toned[1]).min(toned[2]);
    let tl = toned[0] * CSS_SATURATE_LUMA[0] + toned[1] * CSS_SATURATE_LUMA[1] + toned[2] * CSS_SATURATE_LUMA[2];
    let amount = (1.0 - clamp(chroma, 0.0, 1.0)) * vibrance + 1.0;
    let cu = [uv[0] - 0.5, uv[1] - 0.5];
    let vig = 1.0 - smoothstep(0.12, 0.5, cu[0] * cu[0] + cu[1] * cu[1]) * LOOK.finish_vignette;
    toned.map(|x| clamp(mix(tl, x, amount) * vig, 0.0, 1.0))
}

// ---- the filmic display transform (`--no-filmic` keeps `display`'s chain) ----
//
// A film-like characteristic curve with a toe and a shoulder, the
// generalised power curve of Lottes 2016 ("Advanced Techniques and
// Optimization of HDR Color Pipelines", GDC 2016): y = x^a / ((x^a)^d b + c),
// with b and c solved so that scene middle grey (0.18) shows at display
// middle grey (0.18, sRGB 118, L* 50) and `FILMIC_WHITE` (6.3 stops over it)
// at display white. `FILMIC_CONTRAST` is the curve's slope in log-log about
// middle grey (a print film's gamma, ~1.3-1.5 for a camera-negative-to-print
// chain); `FILMIC_SHOULDER` < 1 rolls the highlights off. Applied per channel
// in linear sRGB, so a highlight drifts toward white as film's dye layers
// saturate. Then a chroma gain in Oklab (Ottosson 2020): a camera's rendering
// of an outdoor scene keeps the colour a per-channel shoulder takes out of
// bright saturated tones (the clear sky), as an sRGB camera's picture style
// does (a standard style adds roughly 10-30 % chroma over a colorimetric
// rendering); `FILMIC_CHROMA` is that gain, 1.2, faded out toward black and white.
// The input is the scene radiance x exposure / 0.6 (the chain above's
// scaling, so the frame's exposure keeps its meaning for every consumer).

use std::sync::atomic::{AtomicBool, Ordering};

pub const FILMIC_CONTRAST: f64 = 1.35;
pub const FILMIC_SHOULDER: f64 = 0.96;
pub const FILMIC_WHITE: f64 = 14.0;
pub const FILMIC_MID_IN: f64 = 0.18;
pub const FILMIC_MID_OUT: f64 = 0.18;
pub const FILMIC_CHROMA: f64 = 1.2;

/// The curve's (b, c) for (a, d, white, mid in, mid out).
pub fn filmic_coefficients(a: f64, d: f64, white: f64, mid_in: f64, mid_out: f64) -> (f64, f64) {
    let (wa, ma) = (white.powf(a), mid_in.powf(a));
    let (wad, mad) = (white.powf(a * d), mid_in.powf(a * d));
    let b = (-ma + wa * mid_out) / ((wad - mad) * mid_out);
    let c = (wad * ma - wa * mad * mid_out) / ((wad - mad) * mid_out);
    (b, c)
}

pub fn filmic_curve(x: f64) -> f64 {
    let (b, c) = filmic_coefficients(FILMIC_CONTRAST, FILMIC_SHOULDER, FILMIC_WHITE, FILMIC_MID_IN, FILMIC_MID_OUT);
    if x <= 0.0 {
        return 0.0;
    }
    let xa = x.powf(FILMIC_CONTRAST);
    clamp(xa / (xa.powf(FILMIC_SHOULDER) * b + c), 0.0, 1.0)
}

fn lrgb_to_oklab(c: Rgb) -> Rgb {
    let l = 0.4122214708 * c[0] + 0.5363325363 * c[1] + 0.0514459929 * c[2];
    let m = 0.2119034982 * c[0] + 0.6806995451 * c[1] + 0.1073969566 * c[2];
    let s = 0.0883024619 * c[0] + 0.2817188376 * c[1] + 0.6299787005 * c[2];
    let (l, m, s) = (l.max(0.0).cbrt(), m.max(0.0).cbrt(), s.max(0.0).cbrt());
    [
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    ]
}

fn oklab_to_lrgb(c: Rgb) -> Rgb {
    let l = c[0] + 0.3963377774 * c[1] + 0.2158037573 * c[2];
    let m = c[0] - 0.1055613458 * c[1] - 0.0638541728 * c[2];
    let s = c[0] - 0.0894841775 * c[1] - 1.2914855480 * c[2];
    let (l, m, s) = (l * l * l, m * m * m, s * s * s);
    [
        4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
        -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
        -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s,
    ]
}

/// The chroma gain, faded to none at black and at white (Oklab L 0..0.15, 0.85..1).
pub fn filmic_chroma(y: Rgb, gain: f64) -> Rgb {
    let lab = lrgb_to_oklab(y);
    let k = 1.0 + (gain - 1.0) * smoothstep(0.0, 0.15, lab[0]) * (1.0 - smoothstep(0.85, 1.0, lab[0]));
    let o = oklab_to_lrgb([lab[0], lab[1] * k, lab[2] * k]);
    o.map(|v| clamp(v, 0.0, 1.0))
}

/// The camera's white balance: a camera set to daylight renders a white lit
/// by CIE D55 (5503 K, x 0.3324, y 0.3474, CIE 015:2018 table T.3) as display
/// white (sRGB's D65: x 0.3127, y 0.3290), so sunlit white reads white at
/// noon and the shade under the blue sky reads cool, as in a photograph. A
/// Bradford chromatic adaptation (Lam 1985; CIE 160:2004), applied in linear
/// sRGB before the curve.
pub const FILMIC_WHITE_XY: [f64; 2] = [0.3324, 0.3474];
pub const SRGB_WHITE_XY: [f64; 2] = [0.3127, 0.3290];

type M3 = [[f64; 3]; 3];

fn m3_mul(a: &M3, b: &M3) -> M3 {
    let mut o = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            o[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    o
}

fn m3_inv(m: &M3) -> M3 {
    let [[a, b, c], [d, e, f], [g, h, i]] = *m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    let k = 1.0 / det;
    [
        [(e * i - f * h) * k, (c * h - b * i) * k, (b * f - c * e) * k],
        [(f * g - d * i) * k, (a * i - c * g) * k, (c * d - a * f) * k],
        [(d * h - e * g) * k, (b * g - a * h) * k, (a * e - b * d) * k],
    ]
}

fn m3_apply(m: &M3, v: Rgb) -> Rgb {
    [0, 1, 2].map(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

/// The linear-sRGB matrix that takes a white of chromaticity `from` to one of
/// `to` (Bradford), luminance kept.
pub fn white_balance_matrix(from: [f64; 2], to: [f64; 2]) -> M3 {
    // IEC 61966-2-1 sRGB <-> XYZ, and the Bradford cone matrix.
    let rgb_to_xyz: M3 = [[0.4124, 0.3576, 0.1805], [0.2126, 0.7152, 0.0722], [0.0193, 0.1192, 0.9505]];
    let xyz_to_rgb = m3_inv(&rgb_to_xyz);
    let bfd: M3 = [[0.8951, 0.2664, -0.1614], [-0.7502, 1.7135, 0.0367], [0.0389, -0.0685, 1.0296]];
    let w = |xy: [f64; 2]| [xy[0] / xy[1], 1.0, (1.0 - xy[0] - xy[1]) / xy[1]];
    let (s, d) = (m3_apply(&bfd, w(from)), m3_apply(&bfd, w(to)));
    let diag: M3 = [[d[0] / s[0], 0.0, 0.0], [0.0, d[1] / s[1], 0.0], [0.0, 0.0, d[2] / s[2]]];
    let xyz = m3_mul(&m3_inv(&bfd), &m3_mul(&diag, &bfd));
    m3_mul(&xyz_to_rgb, &m3_mul(&xyz, &rgb_to_xyz))
}

/// The filmic transform's white balance (daylight, D55 to D65).
pub fn filmic_white_balance() -> M3 {
    white_balance_matrix(FILMIC_WHITE_XY, SRGB_WHITE_XY)
}

/// The filmic display transform on one pixel (CPU reference of look_post.wgsl
/// with the filmic stage on, no dither): sRGB-encoded display values.
pub fn display_filmic(c: Rgb, exposure: f64) -> Rgb {
    let x = scale3(m3_apply(&filmic_white_balance(), c), exposure / 0.6);
    let y = filmic_chroma(x.map(filmic_curve), FILMIC_CHROMA);
    y.map(linear_to_srgb)
}

static FILMIC_ON: AtomicBool = AtomicBool::new(false);

/// Which display transform the frame is drawn with (the renderer sets it from
/// `--no-filmic`), so the night exposure solve (eo-sky) meets the display it
/// will be shown through. Off by default: the chain above.
pub fn set_filmic(on: bool) {
    FILMIC_ON.store(on, Ordering::Relaxed);
}

pub fn filmic_on() -> bool {
    FILMIC_ON.load(Ordering::Relaxed)
}

/// The display transform in force (`camera::display_camera`, `display_filmic` or `display`).
pub fn display_active(c: Rgb, exposure: f64, uv: [f64; 2]) -> Rgb {
    if camera::camera_look_on() {
        camera::display_camera(c, exposure)
    } else if filmic_on() {
        display_filmic(c, exposure)
    } else {
        display(c, exposure, uv)
    }
}

#[cfg(test)]
mod filmic_tests {
    use super::*;

    #[test]
    fn middle_grey_and_white_land_where_the_curve_puts_them() {
        assert!((filmic_curve(FILMIC_MID_IN) - FILMIC_MID_OUT).abs() < 1e-9);
        assert!((filmic_curve(FILMIC_WHITE) - 1.0).abs() < 1e-9);
        // a toe: 4 stops under middle grey keeps detail (display > 0.005) but under the line
        let t = filmic_curve(0.18 / 16.0);
        assert!(t > 0.005 && t < 0.18 / 16.0, "{t}");
        // monotone
        let mut last = 0.0;
        for i in 1..400 {
            let v = filmic_curve(i as f64 * 0.05);
            assert!(v >= last);
            last = v;
        }
    }

    #[test]
    fn the_white_balance_whitens_daylight_and_keeps_its_luminance() {
        let m = filmic_white_balance();
        // a D55 white in linear sRGB
        let w = crate::atmosphere::xy_to_srgb(FILMIC_WHITE_XY[0], FILMIC_WHITE_XY[1]);
        let o = m3_apply(&m, w);
        assert!((o[0] - o[2]).abs() < 2e-3 && (o[1] - 1.0).abs() < 2e-3, "{w:?} -> {o:?}");
        assert!((luminance(o) - luminance(w)).abs() < 2e-3);
    }

    #[test]
    fn the_chroma_gain_keeps_greys_grey() {
        let g = filmic_chroma([0.3, 0.3, 0.3], 1.3);
        assert!((g[0] - g[2]).abs() < 1e-6 && (g[0] - 0.3).abs() < 1e-4, "{g:?}");
        let b = filmic_chroma([0.05, 0.12, 0.4], 1.3);
        assert!(b[2] - b[0] > 0.35 - 0.05, "{b:?}");
    }
}
