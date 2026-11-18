//! The water surface's wind field and shore collar: src/render/materials/
//! water-surface.js. `buildWindField` / `buildSwellField` solve the short
//! wave field and the significant height from one wind speed; the collar's
//! contested terms (`surfWashWeight`, `collarWeight`, `spindriftCarry`,
//! `spindriftWeight`, `foamCoverage`, `sceneReflectionWeight`,
//! `reflectedEmission`) are the JS's "one formula, two arithmetics" functions,
//! here over f64 (tested against the JS's NUMBER_OPS) and in eo-render's WGSL.

use std::f64::consts::PI;

const GRAVITY: f64 = 9.81;
/// `REFERENCE_WIND` (m/s): the material's default `windSpeed`.
pub const REFERENCE_WIND: f64 = 6.0;
const WIND: [f64; 2] = [0.83, 0.55];
const BASE_WAVELENGTHS: [f64; 4] = [6.2, 3.4, 1.35, 0.72];
const OCTAVE_SHARE: [f64; 4] = [0.22, 0.18, 0.15, 0.13];
const SINE_SHARE: f64 = 0.54;
const PEAK_SPREAD: f64 = 25.0 * PI / 180.0;
const SPREAD_FAN: [f64; 4] = [-0.30, 0.55, 0.85, -1.0];
const LONG_TRAIN_SPLIT: f64 = 0.07;
const LONG_TRAIN_FAN: f64 = 14.0 * PI / 180.0;
const CHOP_WAVELENGTH: f64 = 0.515;
const CREST_HARMONIC: f64 = 0.35;
const DIFF_STEP_FRACTION: f64 = 0.2;
const NOISE_GRADIENT_RMS: f64 = 0.976;

/// `CHOP_UNDER_FFT`, `TRAINS_UNDER_FFT`.
pub const CHOP_UNDER_FFT: f64 = 0.7;
pub const TRAINS_UNDER_FFT: f64 = 0.1;
/// `CHOP_OCTAVES` (v15): scale, turn off the wind (rad), share of the chop variance.
pub const CHOP_OCTAVES: [(f64, f64, f64); 3] = [(0.43, -0.33, 0.38), (1.0, 0.12, 0.34), (2.37, 0.47, 0.28)];
/// `CHOP_PATCH_M`, `CHOP_PATCH_RANGE`.
pub const CHOP_PATCH_M: f64 = 37.0;
pub const CHOP_PATCH_RANGE: [f64; 2] = [0.4, 1.3];
/// `FINE_FADE`, `SWELL_FADE` (view metres), `RESOLVE_START/END`.
pub const FINE_FADE: [f64; 2] = [25.0, 130.0];
pub const SWELL_FADE: [f64; 2] = [400.0, 2500.0];
pub const RESOLVE: [f64; 2] = [0.18, 0.45];
/// `COLLAR_DISSOLVE`, `DRIFT_TEAR`, `DRIFT_EJECTION_SHARE`, `DRIFT_TAKE`, `SURF_RUNUP_RATIO`.
pub const COLLAR_DISSOLVE: [f64; 2] = [0.30, 0.92];
pub const DRIFT_TEAR: [f64; 2] = [0.52, 0.74];
pub const DRIFT_EJECTION_SHARE: f64 = 0.4;
pub const DRIFT_TAKE: f64 = 0.5;
pub const SURF_RUNUP_RATIO: f64 = 4.0;
/// `FOAM_GRAIN_FREQ`.
pub const FOAM_GRAIN_FREQ: f64 = 1.3;
/// `SCENE_REFLECTION_TAKE`, `SCENE_RADIANCE_FLOOR`, `SCENE_UV_EDGE_FADE`,
/// `SCENE_RADIANCE_CEILING`, `SCENE_RADIANCE_CEILING_FLOOR`.
pub const SCENE_REFLECTION_TAKE: f64 = 0.8;
pub const SCENE_RADIANCE_FLOOR: f64 = 2e-3;
pub const SCENE_UV_EDGE_FADE: f64 = 0.06;
pub const SCENE_RADIANCE_CEILING: f64 = 4.0;
pub const SCENE_RADIANCE_CEILING_FLOOR: f64 = 2.5;
/// The reflection march (`MARCH_*`, tidewater's `_waterSSR` as the JS tunes it).
pub const MARCH_STEPS: u32 = 12;
pub const MARCH_FAR_STEPS: u32 = 4;
pub const MARCH_REFINE_STEPS: u32 = 6;
pub const MARCH_START_M: f64 = 0.15;
pub const MARCH_FIRST_STEP_M: f64 = 0.25;
pub const MARCH_GROWTH: f64 = 1.7;
pub const MARCH_STEP_REFERENCE_M: f64 = 60.0;
pub const MARCH_MAX_ELEVATION_SIN: f64 = 0.45;
pub const MARCH_MIN_FRESNEL: f64 = 0.05;
pub const REFLECTION_SMEAR_GAIN: f64 = 0.12;
pub const REFLECTION_SMEAR_MAX_UV: f64 = 0.03;
/// `FAR_TAP_*` (the vanishing-point tap where the march finds nothing).
pub const FAR_TAP_AHEAD: [f64; 2] = [0.02, 0.12];
pub const FAR_TAP_RISE: [f64; 2] = [0.0, 0.03];
pub const FAR_TAP_BEYOND: [[f64; 2]; 2] = [[1.15, 20.0], [1.5, 60.0]];

/// `MARCH_SPAN`.
pub fn march_span() -> f64 {
    MARCH_START_M + (MARCH_FIRST_STEP_M * (MARCH_GROWTH.powi((MARCH_STEPS + MARCH_FAR_STEPS) as i32) - 1.0)) / (MARCH_GROWTH - 1.0)
}

#[derive(Clone, Debug)]
pub struct Train {
    pub amp: f64,
    pub k: f64,
    pub dir: [f64; 2],
    pub omega: f64,
    pub long: bool,
}

#[derive(Clone, Debug)]
pub struct Chop {
    pub along: f64,
    pub cross: f64,
    pub gain: f64,
    pub step: f64,
    pub drift: f64,
}

#[derive(Clone, Debug)]
pub struct WindField {
    pub mss: f64,
    pub waves: Vec<Train>,
    pub chop: Chop,
    pub swell_variance: f64,
    pub fine_variance: f64,
}

/// `buildWindField(windSpeed)`.
pub fn wind_field(wind_speed: f64) -> WindField {
    let u = wind_speed.max(1.0);
    let mss_along = 0.00316 * u;
    let mss_cross = 0.003 + 0.00192 * u;
    let mss = mss_along + mss_cross;
    let length_scale = (u / REFERENCE_WIND).sqrt();
    let share_sum: f64 = OCTAVE_SHARE.iter().sum();
    let wind_angle = WIND[1].atan2(WIND[0]);
    let mut train_along = 0.0;
    let mut waves = vec![];
    for (i, base) in BASE_WAVELENGTHS.iter().enumerate() {
        let fraction = (OCTAVE_SHARE[i] / share_sum) * SINE_SHARE;
        let theta = PEAK_SPREAD * SPREAD_FAN[i];
        let parts: Vec<(f64, f64)> = if i < 2 { vec![(1.0 - LONG_TRAIN_SPLIT, -LONG_TRAIN_FAN), (1.0 + LONG_TRAIN_SPLIT, LONG_TRAIN_FAN)] } else { vec![(1.0, 0.0)] };
        let np = parts.len() as f64;
        for (scale, fan) in parts {
            let part_fraction = fraction / np;
            let wavelength = base * length_scale * scale;
            let k = PI * 2.0 / wavelength;
            let slope = ((2.0 * part_fraction * mss) / (1.0 + CREST_HARMONIC * CREST_HARMONIC)).sqrt();
            let part_theta = theta + fan;
            train_along += part_fraction * part_theta.cos().powi(2);
            let bearing = wind_angle + part_theta;
            waves.push(Train { amp: slope / k, k, dir: [bearing.cos(), bearing.sin()], omega: (GRAVITY * k).sqrt(), long: i < 2 });
        }
    }
    let chop_fraction = 1.0 - SINE_SHARE;
    let target = mss_along / (mss_along + mss_cross);
    let along_share = ((target - train_along) / chop_fraction).clamp(0.02, 0.45);
    let ratio = (along_share / (1.0 - along_share)).sqrt();
    let chop_base = 1.0 / (CHOP_WAVELENGTH * length_scale);
    let along = chop_base * ratio.sqrt();
    let cross = chop_base / ratio.sqrt();
    let rms = NOISE_GRADIENT_RMS * ((along * along + cross * cross) / 2.0).sqrt();
    let chop = Chop { along, cross, gain: (chop_fraction * mss).sqrt() / rms, step: DIFF_STEP_FRACTION / along.max(cross), drift: (GRAVITY / (PI * 2.0 * chop_base)).sqrt() };
    let swell_variance = mss * ((OCTAVE_SHARE[0] + OCTAVE_SHARE[1]) / share_sum) * SINE_SHARE;
    WindField { mss, waves, chop, swell_variance, fine_variance: mss - swell_variance }
}

/// `SWELL_BANDS`: wavelength, amplitude, bearing, steepness, phase, group.
const SWELL_BANDS: [(f64, f64, f64); 7] = [(72.0, 0.078, 0.58), (55.0, 0.070, 0.74), (38.0, 0.046, 0.86), (28.0, 0.036, 0.44), (19.0, 0.022, 0.30), (13.0, 0.016, 0.98), (9.5, 0.011, 0.64)];

/// `buildSwellField(windSpeed)`: the elevation and slope variance and the
/// significant height (the collar's break line scales with it).
pub struct Swell {
    pub variance: f64,
    pub elevation_variance: f64,
    pub significant_height_m: f64,
}

pub fn swell_field(wind_speed: f64) -> Swell {
    let wind_scale = (wind_speed.max(1.0) / REFERENCE_WIND).sqrt().clamp(0.55, 1.65);
    let (mut variance, mut elev) = (0.0, 0.0);
    for (wavelength, amplitude, _) in SWELL_BANDS {
        let k = PI * 2.0 / wavelength;
        let amp = amplitude * wind_scale;
        variance += (amp * k).powi(2) / 2.0;
        elev += amp * amp / 2.0;
    }
    Swell { variance, elevation_variance: elev, significant_height_m: 4.0 * elev.sqrt() }
}

fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// `surfWashWeight`.
pub fn surf_wash_weight(shore_dist: f64, break_line: f64) -> f64 {
    (1.0 - shore_dist / break_line).clamp(0.0, 1.0)
}

/// `collarWeight`.
pub fn collar_weight(wash: f64, grain: f64) -> f64 {
    smoothstep(COLLAR_DISSOLVE[0], COLLAR_DISSOLVE[1], wash * (grain * 0.86 + 0.42))
}

/// `spindriftCarry(windSpeed, significantHeightM)`.
pub fn spindrift_carry(wind_speed: f64, significant_height_m: f64) -> f64 {
    let height = DRIFT_EJECTION_SHARE * significant_height_m.max(0.05);
    (wind_speed.max(0.0) * ((2.0 * height) / GRAVITY).sqrt()).max(0.4)
}

/// `spindriftWeight`.
pub fn spindrift_weight(grain: f64, shore_foam: f64, shore_dist: f64, break_line: f64, carry: f64) -> f64 {
    let torn = smoothstep(DRIFT_TEAR[0], DRIFT_TEAR[1], grain);
    let beyond = (shore_dist - break_line).max(0.0);
    let runout = (beyond / -carry).exp();
    torn * shore_foam * runout * DRIFT_TAKE
}

/// `foamCoverage`.
pub fn foam_coverage(collar: f64, drift: f64, whitecap: f64) -> f64 {
    let spray = (drift + whitecap).clamp(0.0, 1.0);
    (collar + spray * (1.0 - collar)).clamp(0.0, 1.0)
}

/// `sceneReflectionWeight`.
pub fn scene_reflection_weight(march_weight: f64, sample_luma: f64, available: f64) -> f64 {
    let live = smoothstep(0.0, SCENE_RADIANCE_FLOOR, sample_luma);
    march_weight * live * available * SCENE_REFLECTION_TAKE
}

/// `reflectedEmission`.
pub fn reflected_emission(sky: f64, scene: f64, scene_weight: f64, fresnel: f64, already: f64) -> f64 {
    let deficit = (fresnel - already).max(0.0);
    let sky_share = sky * deficit;
    let scene_share = scene * (already + deficit);
    sky_share + (scene_share - sky_share) * scene_weight
}

/// tidewater's mulberry32 (util/Noise.js), as ocean-sea-detail.js runs it.
fn mulberry32(seed: u32) -> impl FnMut() -> f64 {
    let mut a = seed as i32;
    move || {
        a = a.wrapping_add(0x6D2B_79F5);
        let au = a as u32;
        let mut t = ((au ^ (au >> 15)) as i32).wrapping_mul(1 | a);
        let tu = t as u32;
        t = t.wrapping_add(((tu ^ (tu >> 7)) as i32).wrapping_mul(61 | t)) ^ t;
        let tu = t as u32;
        (tu ^ (tu >> 14)) as f64 / 4294967296.0
    }
}

/// `buildSeaDetailNoise(size)` (ocean-sea-detail.js, tidewater SeaDetail.js
/// `makeNoiseTexture`): four channels of tileable fbm, normalised to 0..1,
/// rgba per texel. Seeds, base frequencies and octaves are tidewater's.
pub fn sea_detail_noise(size: usize) -> Vec<f32> {
    const CHANNELS: [(u32, usize, usize); 4] = [(11, 4, 4), (23, 5, 4), (37, 4, 3), (53, 6, 3)];
    let mut data = vec![0f32; size * size * 4];
    for (c, &(seed, freq, oct)) in CHANNELS.iter().enumerate() {
        let mut rand = mulberry32(seed);
        let tables: Vec<(usize, Vec<f32>)> = (0..oct)
            .map(|o| {
                let n = freq << o;
                let mut g = vec![0f32; n * n * 2];
                for i in 0..n * n {
                    let a = rand() * PI * 2.0;
                    g[i * 2] = a.cos() as f32;
                    g[i * 2 + 1] = a.sin() as f32;
                }
                (n, g)
            })
            .collect();
        let (mut mn, mut mx) = (f64::INFINITY, f64::NEG_INFINITY);
        let mut vals = vec![0f32; size * size];
        for y in 0..size {
            for x in 0..size {
                let (mut v, mut amp, mut norm) = (0.0f64, 1.0f64, 0.0f64);
                for (n, g) in &tables {
                    let n = *n as i64;
                    let fx = (x as f64 / size as f64) * n as f64;
                    let fy = (y as f64 / size as f64) * n as f64;
                    let (xi, yi) = (fx.floor() as i64, fy.floor() as i64);
                    let (xf, yf) = (fx - xi as f64, fy - yi as f64);
                    let grad = |ix: i64, iy: i64, dx: f64, dy: f64| {
                        let k = (iy.rem_euclid(n) * n + ix.rem_euclid(n)) as usize;
                        g[k * 2] as f64 * dx + g[k * 2 + 1] as f64 * dy
                    };
                    let u = xf * xf * xf * (xf * (xf * 6.0 - 15.0) + 10.0);
                    let w = yf * yf * yf * (yf * (yf * 6.0 - 15.0) + 10.0);
                    let a0 = grad(xi, yi, xf, yf);
                    let a1 = grad(xi + 1, yi, xf - 1.0, yf);
                    let b0 = grad(xi, yi + 1, xf, yf - 1.0);
                    let b1 = grad(xi + 1, yi + 1, xf - 1.0, yf - 1.0);
                    let nv = (a0 + (a1 - a0) * u) + ((b0 + (b1 - b0) * u) - (a0 + (a1 - a0) * u)) * w;
                    v += nv * amp;
                    norm += amp;
                    amp *= 0.5;
                }
                v /= norm;
                vals[y * size + x] = v as f32;
                mn = mn.min(v);
                mx = mx.max(v);
            }
        }
        for i in 0..size * size {
            data[i * 4 + c] = ((vals[i] as f64 - mn) / (mx - mn)) as f32;
        }
    }
    data
}

/// WGSL constants for eo-render's water shader (the reference wind's field).
pub fn wgsl_constants() -> String {
    let w = wind_field(REFERENCE_WIND);
    let sw = swell_field(REFERENCE_WIND);
    let f = |v: f64| format!("{v:.8}");
    let mut s = String::from("// generated by eo-water surface::wgsl_constants (src/render/materials/water-surface.js)\n");
    let mut c = |n: &str, t: &str, v: String| s.push_str(&format!("const {n}: {t} = {v};\n"));
    c("WS_WIND", "f32", f(REFERENCE_WIND));
    c("WS_WIND_DIR", "vec2<f32>", format!("vec2<f32>({}, {})", f(WIND[0] / WIND[0].hypot(WIND[1])), f(WIND[1] / WIND[0].hypot(WIND[1]))));
    c("WS_MSS", "f32", f(w.mss));
    c("WS_HS", "f32", f(sw.significant_height_m));
    c("WS_CHOP_ALONG", "f32", f(w.chop.along));
    c("WS_CHOP_CROSS", "f32", f(w.chop.cross));
    c("WS_CHOP_GAIN", "f32", f(w.chop.gain));
    c("WS_CHOP_STEP", "f32", f(w.chop.step));
    c("WS_CHOP_DRIFT", "f32", f(w.chop.drift));
    let wind_var: f64 = w.waves.iter().map(|t| (t.amp * t.k).powi(2) * (1.0 + CREST_HARMONIC * CREST_HARMONIC) / 2.0).sum();
    c("WS_CHOP_VARIANCE", "f32", f((w.mss - wind_var).max(0.0)));
    c("WS_CHOP_UNDER_FFT", "f32", f(CHOP_UNDER_FFT));
    for (i, (scale, turn, share)) in CHOP_OCTAVES.iter().enumerate() {
        c(&format!("WS_OCT{i}"), "vec3<f32>", format!("vec3<f32>({}, {}, {})", f(*scale), f(*turn), f(*share)));
    }
    c("WS_PATCH_M", "f32", f(CHOP_PATCH_M));
    c("WS_PATCH_RANGE", "vec2<f32>", format!("vec2<f32>({}, {})", f(CHOP_PATCH_RANGE[0]), f(CHOP_PATCH_RANGE[1])));
    c("WS_FINE_FADE", "vec2<f32>", format!("vec2<f32>({}, {})", f(FINE_FADE[0]), f(FINE_FADE[1])));
    c("WS_RESOLVE", "vec2<f32>", format!("vec2<f32>({}, {})", f(RESOLVE[0]), f(RESOLVE[1])));
    c("WS_COLLAR", "vec2<f32>", format!("vec2<f32>({}, {})", f(COLLAR_DISSOLVE[0]), f(COLLAR_DISSOLVE[1])));
    c("WS_DRIFT_TEAR", "vec2<f32>", format!("vec2<f32>({}, {})", f(DRIFT_TEAR[0]), f(DRIFT_TEAR[1])));
    c("WS_DRIFT_TAKE", "f32", f(DRIFT_TAKE));
    c("WS_DRIFT_CARRY", "f32", f(spindrift_carry(REFERENCE_WIND, sw.significant_height_m)));
    c("WS_RUNUP", "f32", f(SURF_RUNUP_RATIO));
    c("WS_GRAIN_FREQ", "f32", f(FOAM_GRAIN_FREQ));
    c("WS_SCENE_TAKE", "f32", f(SCENE_REFLECTION_TAKE));
    c("WS_SCENE_FLOOR", "f32", f(SCENE_RADIANCE_FLOOR));
    c("WS_UV_EDGE", "f32", f(SCENE_UV_EDGE_FADE));
    c("WS_SCENE_CEIL", "f32", f(SCENE_RADIANCE_CEILING));
    c("WS_SCENE_CEIL_FLOOR", "f32", f(SCENE_RADIANCE_CEILING_FLOOR));
    c("WS_MARCH_STEPS", "u32", format!("{}u", MARCH_STEPS + MARCH_FAR_STEPS));
    c("WS_MARCH_REFINE", "u32", format!("{}u", MARCH_REFINE_STEPS));
    c("WS_MARCH_START", "f32", f(MARCH_START_M));
    c("WS_MARCH_FIRST", "f32", f(MARCH_FIRST_STEP_M));
    c("WS_MARCH_GROWTH", "f32", f(MARCH_GROWTH));
    c("WS_MARCH_REF", "f32", f(MARCH_STEP_REFERENCE_M));
    c("WS_MARCH_SPAN", "f32", f(march_span()));
    c("WS_MARCH_MAX_SIN", "f32", f(MARCH_MAX_ELEVATION_SIN));
    c("WS_MARCH_MIN_F", "f32", f(MARCH_MIN_FRESNEL));
    c("WS_SMEAR_GAIN", "f32", f(REFLECTION_SMEAR_GAIN));
    c("WS_SMEAR_MAX", "f32", f(REFLECTION_SMEAR_MAX_UV));
    c("WS_FAR_AHEAD", "vec2<f32>", format!("vec2<f32>({}, {})", f(FAR_TAP_AHEAD[0]), f(FAR_TAP_AHEAD[1])));
    c("WS_FAR_RISE", "vec2<f32>", format!("vec2<f32>({}, {})", f(FAR_TAP_RISE[0]), f(FAR_TAP_RISE[1])));
    c("WS_FAR_BEYOND", "vec4<f32>", format!("vec4<f32>({}, {}, {}, {})", f(FAR_TAP_BEYOND[0][0]), f(FAR_TAP_BEYOND[0][1]), f(FAR_TAP_BEYOND[1][0]), f(FAR_TAP_BEYOND[1][1])));
    s
}
