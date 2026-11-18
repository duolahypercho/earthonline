//! The small helpers environment-ibl.js defines, with the same edge cases.

pub const DEG: f64 = std::f64::consts::PI / 180.0;
pub const TAU: f64 = std::f64::consts::PI * 2.0;

pub type Rgb = [f64; 3];

pub fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

pub fn mix(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

pub fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    if e1 == e0 {
        return if x < e0 { 0.0 } else { 1.0 };
    }
    let t = clamp((x - e0) / (e1 - e0), 0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Rec. 709 relative luminance (`luminance` in environment-ibl.js).
pub fn luminance(c: Rgb) -> f64 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

pub fn mix3(a: Rgb, b: Rgb, t: f64) -> Rgb {
    [mix(a[0], b[0], t), mix(a[1], b[1], t), mix(a[2], b[2], t)]
}

pub fn scale3(a: Rgb, s: f64) -> Rgb {
    [a[0] * s, a[1] * s, a[2] * s]
}

/// `geoMix`: interpolation in log space when both rails are positive.
pub fn geo_mix(a: f64, b: f64, t: f64) -> f64 {
    if a > 0.0 && b > 0.0 { mix(a.ln(), b.ln(), t).exp() } else { mix(a, b, t) }
}

/// sRGB transfer (three's `LinearToSRGB`).
pub fn linear_to_srgb(c: f64) -> f64 {
    if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

pub fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}
