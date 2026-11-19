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
/// the ACES 1.3 RRT and sRGB ODT (a port of the Academy's reference CTL; `--camera-aces13`).
pub mod aces13;
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

// ---- the filmic display transform (`--no-filmic` keeps the chain above) ----
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
