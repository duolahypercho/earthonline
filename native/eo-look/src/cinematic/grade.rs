//! The cinematic look's image half (`--look cinematic`, stage L1): a filmic
//! tone curve with a real white point and a hue-preserving shoulder, a
//! per-hour grade baked into a 32^3 lookup table from named keyframes, the
//! energy-conserving bloom's and the lamp glare's constants. eo-render's
//! cinematic post pass (look_cine.wgsl, cine_post.rs) mirrors these CPU
//! references; today's display transform (`tone::display`) is untouched.
//!
//! In order, on a pixel of scene value `x` (radiance x exposure, middle grey
//! 0.18):
//!
//! 1. `tone`: the brightest channel takes `curve` (toe, straight section,
//!    shoulder to white at `CINE_WHITE_X`), the colour keeps its channel ratios
//!    (its hue and saturation) and moves toward white only once it is near or
//!    past the white point (`CINE_WHITEN_EV`): a sodium lamp's pool stays orange,
//!    its lens and the sky round the sun go white smoothly, nothing clips per
//!    channel.
//! 2. sRGB encode.
//! 3. `graded`: the grade of the hour and weather, a blend of the keyframes'
//!    grades (`GRADE_KEYS`, weights from `key_weights`), sampled from the
//!    lookup table on the GPU.
//! 4. the display vignette (`CINE_VIGNETTE`), the dither.

use crate::math::*;

/// Photographic middle grey (scene value).
pub const GREY: f64 = 0.18;

/// The scene value that shows as white: 12 x middle grey (+3.6 EV). Today's
/// chain (three's ACES fit after the /0.6 and its shoulder) reaches display
/// 0.99 only at about 33 x grey, so its top stop and a half is a slow crawl
/// toward a white it never quite reaches: highlights read grey-milky. A film
/// stock or a camera's picture profile puts white about 3-4 stops over
/// middle grey; 12 x is the middle of the plan's 8-12 x.
pub const CINE_WHITE_X: f64 = 2.16;
/// How far under white (display linear) the curve is at `CINE_WHITE_X`: a
/// sixteenth of an 8-bit step, so white is white on the display.
pub const CINE_WHITE_RESIDUAL: f64 = 1.0 / 1024.0;
/// Middle grey's display value (linear): 0.20, sRGB 124 (today's chain puts
/// it at 0.213, sRGB 127, before its grade). A touch lower so the extra
/// contrast below has room above it.
pub const CINE_GREY_DISPLAY: f64 = 0.20;
/// The curve's contrast at middle grey: the slope of log(display) over
/// log(scene). 1.35 (today's ACES fit: 1.25, and its grade's 1.04 S-curve).
pub const CINE_GREY_CONTRAST: f64 = 1.35;
/// The toe's power (display = y0 (x / x0)^p under the toe's end): 1.6, a
/// deeper toe than a straight line (1) and than ACES's fit near black (about
/// 1.2-1.4): the shadows' density.
pub const CINE_TOE_POWER: f64 = 1.6;
/// Where a colour starts toward white, and where it is white, in EV against
/// `CINE_WHITE_X` of its brightest channel: from a stop under white to two
/// stops over it. A saturated light keeps its hue until it is very bright.
pub const CINE_WHITEN_EV: [f64; 2] = [-1.0, 2.0];
/// The display vignette (as the default look's `finish_vignette`, which is
/// 0): the corners 8 % darker, a frame's gentle fall-off toward its edges.
pub const CINE_VIGNETTE: f64 = 0.08;

/// The curve's pieces, solved from the constants above (`curve_params`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Curve {
    /// The toe's end (scene) and its display value.
    pub x0: f64,
    pub y0: f64,
    /// The straight section's slope (display per scene unit).
    pub slope: f64,
    pub toe_power: f64,
    /// The shoulder's start (scene) and its display value.
    pub x1: f64,
    pub y1: f64,
    /// The shoulder's rate: display = 1 - (1 - y1) exp(-rate (x - x1)).
    pub rate: f64,
}

/// The curve's pieces: a power toe, a straight section through middle grey
/// at `CINE_GREY_DISPLAY` with the contrast `CINE_GREY_CONTRAST`, an
/// exponential shoulder joined with the same slope and starting where it
/// lands `CINE_WHITE_RESIDUAL` under white at `CINE_WHITE_X`. Continuous with
/// its slope everywhere, strictly increasing, 0 at 0, toward 1.
pub fn curve_params() -> Curve {
    let slope = CINE_GREY_DISPLAY * CINE_GREY_CONTRAST / GREY;
    let tp = CINE_TOE_POWER;
    // y0 + slope (G - x0) = grey's display, and the toe's slope at x0 is y0 tp / x0 = slope.
    let x0 = (GREY - CINE_GREY_DISPLAY / slope) / (1.0 - 1.0 / tp);
    let y0 = slope * x0 / tp;
    let line = |x: f64| y0 + slope * (x - x0);
    let residual = |x1: f64| {
        let y1 = line(x1);
        (1.0 - y1) * (-slope * (CINE_WHITE_X - x1) / (1.0 - y1)).exp()
    };
    // the residual falls as the shoulder starts later; it must start under white.
    let (mut lo, mut hi) = (GREY, (x0 + (1.0 - y0) / slope).min(CINE_WHITE_X) - 1e-9);
    for _ in 0..80 {
        let mid = 0.5 * (lo + hi);
        if residual(mid) > CINE_WHITE_RESIDUAL { lo = mid } else { hi = mid }
    }
    let x1 = 0.5 * (lo + hi);
    let y1 = line(x1);
    Curve { x0, y0, slope, toe_power: tp, x1, y1, rate: slope / (1.0 - y1) }
}

fn params() -> &'static Curve {
    static C: std::sync::OnceLock<Curve> = std::sync::OnceLock::new();
    C.get_or_init(curve_params)
}

/// The tone curve: scene value -> display linear (0..1).
pub fn curve(x: f64) -> f64 {
    let c = params();
    if x.is_nan() || x <= 0.0 {
        return 0.0;
    }
    if x < c.x0 {
        c.y0 * (x / c.x0).powf(c.toe_power)
    } else if x < c.x1 {
        c.y0 + c.slope * (x - c.x0)
    } else {
        1.0 - (1.0 - c.y1) * (-c.rate * (x - c.x1)).exp()
    }
}

/// The largest scene value a channel is read at (a guard against infinities).
pub const CINE_MAX_X: f64 = 1.0e6;

/// The tone on one colour of scene values (radiance x exposure) -> display
/// linear: the brightest channel on `curve`, the ratios kept, then toward
/// white by `CINE_WHITEN_EV`. A grey is `curve` exactly; negative and NaN
/// channels read as 0.
pub fn tone(x: Rgb) -> Rgb {
    let x = x.map(|v| if v.is_nan() { 0.0 } else { v.clamp(0.0, CINE_MAX_X) });
    let n = x[0].max(x[1]).max(x[2]);
    if n <= 1e-10 {
        return [0.0; 3];
    }
    let t = curve(n);
    let a = smoothstep(CINE_WHITEN_EV[0], CINE_WHITEN_EV[1], (n / CINE_WHITE_X).log2());
    x.map(|v| {
        let q = v / n * t;
        q + (t - q) * a
    })
}

// ---------------------------------------------------------------------------
// The grade.

/// One keyframe of the grade, in display (sRGB-encoded) values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradeKey {
    pub name: &'static str,
    /// The S-curve on the luma: strength (1 none) and pivot.
    pub contrast: f64,
    pub pivot: f64,
    /// Saturation in the shadows and in the highlights (1 none), blended over
    /// the luma 0.05 .. 0.6.
    pub sat_shadow: f64,
    pub sat_highlight: f64,
    /// Colour balance of the shadows (full at black, gone at `split`) and of
    /// the highlights (from `split`, gone again at white so white stays
    /// white). Gains per channel, normalised to keep the luma.
    pub shadow_balance: Rgb,
    pub highlight_balance: Rgb,
    pub split: f64,
    /// Overall gain per channel, normalised to keep the luma (a white balance).
    pub gain: Rgb,
}

/// Day: near neutral, gentle contrast; a grey stays grey.
pub const KEY_DAY: GradeKey = GradeKey {
    name: "day",
    contrast: 1.06,
    pivot: 0.45,
    sat_shadow: 1.0,
    sat_highlight: 1.03,
    shadow_balance: [1.0, 1.0, 1.0],
    highlight_balance: [1.0, 1.0, 1.0],
    split: 0.4,
    gain: [1.0, 1.0, 1.0],
};

/// Golden hour, evening: warm highlights, teal-cool shadows, more contrast.
/// The balances stay within about 6 % a channel (no garish teal-orange).
pub const KEY_GOLDEN_PM: GradeKey = GradeKey {
    name: "golden-pm",
    contrast: 1.08,
    pivot: 0.42,
    sat_shadow: 0.96,
    sat_highlight: 1.10,
    shadow_balance: [0.95, 1.0, 1.06],
    highlight_balance: [1.05, 1.0, 0.92],
    split: 0.38,
    gain: [1.01, 1.0, 0.98],
};

/// Golden hour, morning: as the evening's, cooler and softer (morning air is
/// clearer and the light paler).
pub const KEY_GOLDEN_AM: GradeKey = GradeKey {
    name: "golden-am",
    contrast: 1.08,
    pivot: 0.42,
    sat_shadow: 0.97,
    sat_highlight: 1.07,
    shadow_balance: [0.96, 1.0, 1.05],
    highlight_balance: [1.035, 1.0, 0.95],
    split: 0.38,
    gain: [1.0, 1.0, 0.99],
};

/// Blue hour: cool and saturated; the first lights keep their warmth.
pub const KEY_BLUE: GradeKey = GradeKey {
    name: "blue-hour",
    contrast: 1.10,
    pivot: 0.38,
    sat_shadow: 1.08,
    sat_highlight: 1.12,
    shadow_balance: [0.93, 0.99, 1.10],
    highlight_balance: [1.03, 1.0, 0.97],
    split: 0.35,
    gain: [0.98, 1.0, 1.03],
};

/// Night: deep, cool blacks; the lights keep their colour (this replaces
/// today's mesopic fade, which drained colour from everything dim).
pub const KEY_NIGHT: GradeKey = GradeKey {
    name: "night",
    contrast: 1.35,
    pivot: 0.25,
    sat_shadow: 0.90,
    sat_highlight: 1.3,
    shadow_balance: [0.88, 0.98, 1.18],
    highlight_balance: [1.02, 1.0, 0.98],
    split: 0.30,
    gain: [1.0, 1.0, 1.0],
};

/// Overcast (and rain, fog): flat light; a little less contrast and colour,
/// slightly cool. Blended in by the weather's overcast, by day.
pub const KEY_OVERCAST: GradeKey = GradeKey {
    name: "overcast",
    contrast: 1.03,
    pivot: 0.45,
    sat_shadow: 0.94,
    sat_highlight: 0.98,
    shadow_balance: [0.97, 1.0, 1.04],
    highlight_balance: [1.0, 1.0, 1.0],
    split: 0.4,
    gain: [0.99, 1.0, 1.01],
};

/// Every keyframe, in the order of `key_weights`.
pub const GRADE_KEYS: [GradeKey; 6] = [KEY_NIGHT, KEY_BLUE, KEY_GOLDEN_AM, KEY_GOLDEN_PM, KEY_DAY, KEY_OVERCAST];
pub const KEY_COUNT: usize = 6;

/// The sun altitudes (deg) the keys hand over at: full night at or under
/// the first; blue hour at the second; golden hour from the third to the
/// fourth; day at and over the last. Smoothstep between neighbours, so the
/// weights and their slope are continuous. Blue hour proper is the sun 4-8
/// deg under the horizon; golden hour about 6 deg either side of it.
pub const KEY_ALTITUDES_DEG: [f64; 5] = [-12.0, -5.0, 1.5, 9.0, 22.0];
/// The overcast key's share at full overcast by day (fading out by night,
/// where the night key's cool blacks stay).
pub const OVERCAST_SHARE: f64 = 0.7;

/// The keys' weights (`GRADE_KEYS` order, summing to 1) for the sun's
/// altitude, the hour (morning or evening golden hour) and the weather's
/// overcast (0..1). Continuous in all three.
pub fn key_weights(sun_alt_deg: f64, hour: f64, overcast: f64) -> [f64; KEY_COUNT] {
    let a = if sun_alt_deg.is_nan() { 90.0 } else { sun_alt_deg };
    let [n, b, g0, g1, d] = KEY_ALTITUDES_DEG;
    let mut w = [0.0; KEY_COUNT];
    let golden = if hour.rem_euclid(24.0) < 12.0 { 2 } else { 3 };
    if a <= n {
        w[0] = 1.0;
    } else if a <= b {
        let t = smoothstep(n, b, a);
        w[0] = 1.0 - t;
        w[1] = t;
    } else if a <= g0 {
        let t = smoothstep(b, g0, a);
        w[1] = 1.0 - t;
        w[golden] = t;
    } else if a <= g1 {
        w[golden] = 1.0;
    } else {
        let t = smoothstep(g1, d, a);
        w[golden] = 1.0 - t;
        w[4] = t;
    }
    let o = OVERCAST_SHARE * clamp(if overcast.is_nan() { 0.0 } else { overcast }, 0.0, 1.0) * smoothstep(-6.0, 6.0, a);
    for v in w.iter_mut() {
        *v *= 1.0 - o;
    }
    w[5] += o;
    w
}

/// Rec. 709 luma of display values (as the default grade's CSS saturate).
fn luma(c: Rgb) -> f64 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn normalised(g: Rgb) -> Rgb {
    let l = luma(g).max(1e-6);
    g.map(|v| v / l)
}

/// The S-curve on a luma (0..1): power curves either side of the pivot.
fn s_curve(y: f64, pivot: f64, k: f64) -> f64 {
    let y = clamp(y, 0.0, 1.0);
    if y < pivot { pivot * (y / pivot).powf(k) } else { 1.0 - (1.0 - pivot) * ((1.0 - y) / (1.0 - pivot)).powf(k) }
}

/// One key's grade on a display colour (sRGB-encoded, 0..1) -> display.
pub fn grade(s: Rgb, k: &GradeKey) -> Rgb {
    let s = s.map(|v| clamp(v, 0.0, 1.0));
    let y = luma(s);
    let yc = s_curve(y, k.pivot, k.contrast);
    // the contrast on the luma: darker by a gain (black stays black), lighter
    // by a gain on the way to white (white stays white), so no channel clips.
    let s = if yc <= y {
        let g = if y > 1e-9 { yc / y } else { 1.0 };
        s.map(|v| v * g)
    } else {
        let g = if y < 1.0 - 1e-9 { (1.0 - yc) / (1.0 - y) } else { 1.0 };
        s.map(|v| 1.0 - (1.0 - v) * g)
    };
    let yl = luma(s);
    let sat = mix(k.sat_shadow, k.sat_highlight, smoothstep(0.05, 0.6, yl));
    let s = s.map(|v| yl + (v - yl) * sat);
    let ws = 1.0 - smoothstep(0.0, k.split, yl);
    let wh = smoothstep(k.split, 1.0, yl) * (1.0 - smoothstep(0.92, 1.0, yl));
    let (sb, hb, gn) = (normalised(k.shadow_balance), normalised(k.highlight_balance), normalised(k.gain));
    [0, 1, 2].map(|i| clamp(s[i] * mix(1.0, sb[i], ws) * mix(1.0, hb[i], wh) * gn[i], 0.0, 1.0))
}

/// The blended grade (the lookup table's value): the keys' grades weighted.
pub fn graded(s: Rgb, weights: &[f64; KEY_COUNT]) -> Rgb {
    let mut o = [0.0; 3];
    for (k, w) in GRADE_KEYS.iter().zip(weights) {
        if *w > 0.0 {
            let g = grade(s, k);
            for i in 0..3 {
                o[i] += w * g[i];
            }
        }
    }
    o
}

/// The lookup table's size per side.
pub const LUT_SIZE: usize = 32;

/// A key's table: `LUT_SIZE`^3 display colours, red fastest, then green,
/// then blue (texel (r, g, b) holds the grade of (r, g, b) / (LUT_SIZE - 1)).
pub fn bake_lut(k: &GradeKey) -> Vec<[f32; 3]> {
    let n = LUT_SIZE;
    let step = 1.0 / (n - 1) as f64;
    let mut out = Vec::with_capacity(n * n * n);
    for b in 0..n {
        for g in 0..n {
            for r in 0..n {
                let o = grade([r as f64 * step, g as f64 * step, b as f64 * step], k);
                out.push(o.map(|v| v as f32));
            }
        }
    }
    out
}

/// The weighted sum of the keys' tables (`GRADE_KEYS` order).
pub fn blend_luts(luts: &[Vec<[f32; 3]>], weights: &[f64; KEY_COUNT]) -> Vec<[f32; 3]> {
    let len = luts.first().map(|l| l.len()).unwrap_or(0);
    let mut out = vec![[0f32; 3]; len];
    for (l, w) in luts.iter().zip(weights) {
        if *w <= 0.0 {
            continue;
        }
        let w = *w as f32;
        for (o, v) in out.iter_mut().zip(l) {
            for i in 0..3 {
                o[i] += w * v[i];
            }
        }
    }
    out
}

/// A table's value at a display colour, trilinear between its texels (as the
/// GPU samples it).
pub fn lut_sample(lut: &[[f32; 3]], s: Rgb) -> Rgb {
    let n = LUT_SIZE;
    let p = s.map(|v| clamp(v, 0.0, 1.0) * (n - 1) as f64);
    let i0 = p.map(|v| (v.floor() as usize).min(n - 2));
    let f = [p[0] - i0[0] as f64, p[1] - i0[1] as f64, p[2] - i0[2] as f64];
    let at = |r: usize, g: usize, b: usize| lut[(b * n + g) * n + r];
    let mut o = [0.0; 3];
    for corner in 0..8 {
        let (dr, dg, db) = (corner & 1, (corner >> 1) & 1, (corner >> 2) & 1);
        let w = (if dr == 1 { f[0] } else { 1.0 - f[0] }) * (if dg == 1 { f[1] } else { 1.0 - f[1] }) * (if db == 1 { f[2] } else { 1.0 - f[2] });
        let v = at(i0[0] + dr, i0[1] + dg, i0[2] + db);
        for i in 0..3 {
            o[i] += w * v[i] as f64;
        }
    }
    o
}

/// The display vignette's gain at `uv` (0..1 across and down).
pub fn vignette(uv: [f64; 2]) -> f64 {
    let cu = [uv[0] - 0.5, uv[1] - 0.5];
    1.0 - smoothstep(0.12, 0.5, cu[0] * cu[0] + cu[1] * cu[1]) * CINE_VIGNETTE
}

/// The cinematic display transform on one pixel (no dither, no bloom, no
/// lens): `tone`, sRGB, the blended grade, the vignette. CPU reference of
/// eo-render look_cine.wgsl.
pub fn cinematic_display(c: Rgb, exposure: f64, weights: &[f64; KEY_COUNT], uv: [f64; 2]) -> Rgb {
    let t = tone(c.map(|v| v * exposure));
    let s = t.map(linear_to_srgb);
    let v = vignette(uv);
    graded(s, weights).map(|x| clamp(x * v, 0.0, 1.0))
}

/// `cinematic_display` through a baked table (as the GPU does it).
pub fn cinematic_display_lut(c: Rgb, exposure: f64, lut: &[[f32; 3]], uv: [f64; 2]) -> Rgb {
    let t = tone(c.map(|v| v * exposure));
    let s = t.map(linear_to_srgb);
    let v = vignette(uv);
    lut_sample(lut, s).map(|x| clamp(x * v, 0.0, 1.0))
}

/// The largest change of the keys' weights the table may lag by before it
/// is baked and uploaded again: 1 %, under an 8-bit step of any colour the
/// keys grade differently by up to a quarter of the range.
pub const LUT_STEP: f64 = 0.01;

/// Whether the table baked for `last` must be redone for `now`.
pub fn lut_needs_update(last: Option<&[f64; KEY_COUNT]>, now: &[f64; KEY_COUNT]) -> bool {
    match last {
        None => true,
        Some(l) => l.iter().zip(now).any(|(a, b)| (a - b).abs() > LUT_STEP),
    }
}

// ---------------------------------------------------------------------------
// Bloom and glare under the look (eo-render sky.rs, sky_bloom.wgsl, look_cine.wgsl, sky_fx.wgsl).

/// The bloom's threshold (scene value, radiance x exposure): 0.9, 2.3 EV over
/// middle grey (today's 2.1: +3.5 EV, the display's knee; the photo finish's
/// 1.44): lamps, lit signs and lit windows glow, not a sunlit wall.
pub const CINE_BLOOM_THRESHOLD_X: f64 = 0.9;
/// The threshold's knee, a share of it (a soft start).
pub const CINE_BLOOM_KNEE: f64 = 1.5;
/// Each source texel held to this many times the threshold (today's 4).
pub const CINE_BLOOM_CEILING: f64 = 24.0;
/// The share of the bright part the bloom spreads: the pixel keeps 1 - this
/// of its bright part and the pyramid gives it back spread out (energy kept;
/// today's bloom only adds, about 0.78 of the bright part).
pub const CINE_BLOOM_SHARE: f64 = 0.22;
/// The five levels' relative weights (half resolution to 1/32), normalised to
/// `CINE_BLOOM_SHARE`: most of the spread in the wide levels, a long soft tail.
pub const CINE_BLOOM_LEVELS: [f64; 5] = [1.0, 0.9, 0.85, 0.8, 0.75];

/// The levels' weights, summing to `CINE_BLOOM_SHARE`.
pub fn bloom_weights() -> [f64; 5] {
    let s: f64 = CINE_BLOOM_LEVELS.iter().sum();
    CINE_BLOOM_LEVELS.map(|v| v / s * CINE_BLOOM_SHARE)
}

/// The lamp glare (sky_fx.wgsl): a starburst of `CINE_GLARE_RAYS` rays (an
/// aperture of half as many blades: 3 lines, 6 points) round the brightest
/// point lights, drawn with their sprite. A light's glare grows with how
/// many threshold-bright pixels its light is worth on the sensor (its
/// intensity toward the eye x the exposure over a pixel's area at its
/// distance): none under 2^`CINE_GLARE_EV[0]`, full length from
/// 2^`CINE_GLARE_EV[1]`.
pub const CINE_GLARE_RAYS: u32 = 6;
pub const CINE_GLARE_EV: [f64; 2] = [6.0, 13.0];
/// The longest ray, pixels on a 1080-line frame.
pub const CINE_GLARE_LENGTH_PX: f64 = 70.0;
/// The share of the light's energy the rays carry (subtle).
pub const CINE_GLARE_SHARE: f64 = 0.015;
/// The rays' angle off the horizontal (deg): fixed, as a lens's blades are.
pub const CINE_GLARE_ANGLE_DEG: f64 = 15.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_curve_is_smooth_increasing_and_lands_grey_and_white() {
        let c = curve_params();
        assert!(c.x0 > 0.0 && c.x0 < GREY && c.x1 > GREY && c.x1 < CINE_WHITE_X, "{c:?}");
        assert!((curve(GREY) - CINE_GREY_DISPLAY).abs() < 1e-12);
        assert!((1.0 - curve(CINE_WHITE_X) - CINE_WHITE_RESIDUAL).abs() < 1e-9);
        // continuous with its slope at the joins.
        for x in [c.x0, c.x1] {
            let h = 1e-7;
            assert!((curve(x - h) - curve(x + h)).abs() < 1e-6);
            let d0 = (curve(x - h) - curve(x - 2.0 * h)) / h;
            let d1 = (curve(x + 2.0 * h) - curve(x + h)) / h;
            assert!((d0 - d1).abs() < 1e-3 * d0.max(1e-3), "{x}: {d0} {d1}");
        }
        let mut last = -1.0;
        for i in 0..4000 {
            let x = 1e-5 * (1.006f64).powi(i);
            let y = curve(x);
            assert!((y > last || 1.0 - y < 1e-12) && y <= 1.0, "{x}: {y}");
            last = y;
        }
        assert_eq!(curve(0.0), 0.0);
        assert_eq!(curve(f64::NAN), 0.0);
        assert!(curve(f64::INFINITY) <= 1.0);
        // contrast at grey and a deeper toe than today's ACES fit.
        let k = (curve(GREY * 1.001).ln() - curve(GREY / 1.001).ln()) / (2.0 * 1.001f64.ln());
        assert!((k - CINE_GREY_CONTRAST).abs() < 1e-3, "{k}");
        let aces = crate::tone::neutral_display(0.018);
        assert!(curve(0.018) < aces, "{} vs {aces}", curve(0.018));
    }

    #[test]
    fn the_tone_keeps_hue_until_white_and_never_clips_a_channel() {
        let hue = |c: Rgb| {
            let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
            (c[1] - mn) / (mx - mn).max(1e-12)
        };
        let lamp = [1.0, 0.55, 0.2];
        for ev in -8..2 {
            let x = lamp.map(|v| v * GREY * 2f64.powi(ev));
            let t = tone(x);
            assert!((hue(t) - hue(lamp)).abs() < 1e-9, "{ev}: {t:?}");
            // kept saturation well under white.
            if ev < 1 {
                assert!((t[2] / t[0] - 0.2).abs() < 1e-9, "{ev}: {t:?}");
            }
        }
        // far over white: white.
        let t = tone(lamp.map(|v| v * CINE_WHITE_X * 8.0));
        assert!(t.iter().all(|v| *v > 0.995), "{t:?}");
        // monotone per channel in brightness, a grey is the curve.
        let mut last = [0.0; 3];
        for i in 0..2000 {
            let s = 1e-4 * (1.01f64).powi(i);
            let t = tone(lamp.map(|v| v * s));
            assert!((0..3).all(|k| t[k] >= last[k] - 1e-12 && t[k] <= 1.0), "{s}: {t:?}");
            last = t;
        }
        assert_eq!(tone([0.3; 3]), [curve(0.3); 3]);
        assert_eq!(tone([f64::NAN, -1.0, 0.0]), [0.0; 3]);
        assert!(tone([f64::INFINITY, 1.0, 1.0]).iter().all(|v| v.is_finite()));
    }

    #[test]
    fn the_day_grade_keeps_greys_and_skin_neutral_and_every_key_is_tasteful() {
        for i in 0..=255 {
            let v = i as f64 / 255.0;
            let g = grade([v; 3], &KEY_DAY);
            let spread = g[0].max(g[1]).max(g[2]) - g[0].min(g[1]).min(g[2]);
            assert!(spread < 0.5 / 255.0, "{v}: {g:?}");
        }
        // a skin-like tone (sRGB 224, 172, 140) keeps its hue within 2 deg by day.
        let hue_deg = |c: Rgb| {
            let (r, g, b) = (c[0], c[1], c[2]);
            (3f64.sqrt() * (g - b)).atan2(2.0 * r - g - b).to_degrees()
        };
        let skin = [224.0 / 255.0, 172.0 / 255.0, 140.0 / 255.0];
        assert!((hue_deg(grade(skin, &KEY_DAY)) - hue_deg(skin)).abs() < 2.0);
        // no key moves a mid grey more than 4 % a channel, or skin's hue more than 8 deg.
        for k in GRADE_KEYS {
            let g = grade([0.46; 3], &k);
            assert!(g.iter().all(|v| (v / luma(g) - 1.0).abs() < 0.04), "{}: {g:?}", k.name);
            assert!((hue_deg(grade(skin, &k)) - hue_deg(skin)).abs() < 8.0, "{}", k.name);
            // black stays black, white stays near white.
            assert!(grade([0.0; 3], &k).iter().all(|v| *v < 1e-9), "{}", k.name);
            assert!(grade([1.0; 3], &k).iter().all(|v| *v > 0.97), "{}", k.name);
        }
        // the night key cools the dark and keeps a warm light's colour.
        let dark = grade([0.06; 3], &KEY_NIGHT);
        assert!(dark[2] > dark[0], "{dark:?}");
        // (saturation as 1 - min / max: a bright light keeps 85 % of it as the
        // contrast lifts it toward white, a lamp-lit wall in the mid tones all of it.)
        let sat = |c: Rgb| 1.0 - c[0].min(c[1]).min(c[2]) / c[0].max(c[1]).max(c[2]);
        let warm = [0.95, 0.62, 0.30];
        let w = grade(warm, &KEY_NIGHT);
        assert!(sat(w) >= 0.85 * sat(warm), "{w:?}");
        let lit = [0.55, 0.36, 0.17];
        assert!(sat(grade(lit, &KEY_NIGHT)) >= sat(lit), "{:?}", grade(lit, &KEY_NIGHT));
    }

    #[test]
    fn the_weights_sum_to_one_and_move_continuously_through_the_day() {
        let mut last: Option<[f64; KEY_COUNT]> = None;
        let mut last_lut: Option<Vec<[f32; 3]>> = None;
        let luts: Vec<_> = GRADE_KEYS.iter().map(bake_lut).collect();
        // every 6 s of the game's clock.
        for m in 0..(24 * 600) {
            let hour = m as f64 / 600.0;
            let alt = crate::sun::sun_direction(hour).altitude_deg;
            for o in [0.0, 0.6] {
                let w = key_weights(alt, hour, o);
                assert!((w.iter().sum::<f64>() - 1.0).abs() < 1e-12 && w.iter().all(|v| *v >= 0.0), "{hour} {w:?}");
                if o > 0.0 {
                    continue;
                }
                if let Some(l) = last {
                    // 6 s of the clock move no weight by the table's update step.
                    assert!(l.iter().zip(&w).all(|(a, b)| (a - b).abs() < LUT_STEP), "{hour}: {l:?} -> {w:?}");
                }
                last = Some(w);
                // and a minute moves the blended table by under four 8-bit steps (6 s of
                // the clock under half a step: no visible step at any clock rate the game runs).
                if m % 10 == 0 {
                    let lut = blend_luts(&luts, &w);
                    if let Some(p) = &last_lut {
                        let d = lut.iter().zip(p).map(|(a, b)| (0..3).map(|i| (a[i] - b[i]).abs()).fold(0f32, f32::max)).fold(0f32, f32::max);
                        assert!(d < 4.0 / 255.0, "{hour}: table moved {d} in a minute");
                    }
                    last_lut = Some(lut);
                }
            }
        }
        // the hours the key views are drawn at land on their keys.
        let at = |h: f64| key_weights(crate::sun::sun_direction(h).altitude_deg, h, 0.0);
        assert_eq!(at(12.0)[4], 1.0);
        assert!(at(18.5)[3] > 0.9, "{:?}", at(18.5));
        assert_eq!(at(22.0)[0], 1.0);
        assert!(at(7.5)[2] > 0.5, "{:?}", at(7.5));
        // overcast by day takes its share; by night it does not.
        assert!((key_weights(40.0, 12.0, 1.0)[5] - OVERCAST_SHARE).abs() < 1e-12);
        assert_eq!(key_weights(-30.0, 1.0, 1.0)[5], 0.0);
    }

    #[test]
    fn the_table_matches_the_grade_and_is_deterministic() {
        let w = key_weights(5.0, 18.5, 0.2);
        let luts: Vec<_> = GRADE_KEYS.iter().map(bake_lut).collect();
        let lut = blend_luts(&luts, &w);
        assert_eq!(lut, blend_luts(&GRADE_KEYS.iter().map(bake_lut).collect::<Vec<_>>(), &w));
        let mut worst: f64 = 0.0;
        let mut seed = 0x1234_5678u32;
        for _ in 0..4000 {
            let mut r = || {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                (seed >> 8) as f64 / 16777216.0
            };
            let s = [r(), r(), r()];
            let a = graded(s, &w);
            let b = lut_sample(&lut, s);
            worst = worst.max((0..3).map(|i| (a[i] - b[i]).abs()).fold(0.0, f64::max));
        }
        // trilinear over 32 steps is within two and a half 8-bit steps of the exact grade.
        assert!(worst < 2.5 / 255.0, "{worst}");
        assert!(!lut_needs_update(Some(&w), &w) && lut_needs_update(None, &w));
        let mut w2 = w;
        w2[3] += 0.02;
        assert!(lut_needs_update(Some(&w), &w2));
    }

    #[test]
    fn the_bloom_weights_carry_the_share() {
        assert!((bloom_weights().iter().sum::<f64>() - CINE_BLOOM_SHARE).abs() < 1e-12);
        assert!(CINE_BLOOM_THRESHOLD_X < 2.101);
    }
}
