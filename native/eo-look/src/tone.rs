//! The image half: three's `acesFilmicToneMapping` (r180, the version the
//! game pins) with `toneMappingExposure`, the post chain's highlight shoulder
//! before it, and after the sRGB encode the display-referred grade and the
//! look's finish (src/render/post-chain.js `gradeDisplayReferred`,
//! `applyLookFinish`). The WGSL in eo-render/src/look_post.wgsl mirrors these
//! CPU references; the tests compare these with the JS.

use crate::look::LOOK;
use crate::math::*;

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
