//! The FFT ocean spectrum: `buildSpectrum` of src/render/materials/ocean-fft.js
//! (tidewater's Tessendorf / JONSWAP-TMA two-system spectrum), ported line for
//! line, with the JS's PCG and its double-precision overflow reproduced so the
//! same seed gives the same sea. Plus the CPU reference of the transform the
//! GPU runs (`height_at`, the sum `createOceanHeightSampler` evaluates) and
//! `cascadeBandWeights`.

/// `FFT_SIZE`.
pub const N: usize = 256;
/// `CASCADES`.
pub const CASCADES: usize = 4;
/// `CASCADE_SIZES` (metres).
pub const CASCADE_SIZES: [f64; CASCADES] = [733.0, 157.0, 33.3, 7.1];
const GRAVITY: f64 = 9.81;
const TWO_PI: f64 = std::f64::consts::PI * 2.0;

/// OceanFFT's live uniforms (constructor defaults).
pub const CHOPPINESS: f32 = 0.9;
pub const FOAM_BIAS: f32 = 0.58;
pub const FOAM_GAIN: f32 = 3.0;
pub const FOAM_DECAY: f32 = 0.35;
pub const FOAM_ADD: f32 = 2.5;
/// `buildSpectrum` defaults.
pub const DEPTH_M: f64 = 500.0;
pub const SEED: f64 = 1337.0;
pub const SIGNIFICANT_HEIGHT_M: f64 = 2.0;

#[derive(Clone, Copy, Debug)]
pub struct WaveSystem {
    pub scale: f64,
    pub wind_speed: f64,
    pub wind_direction: f64,
    pub fetch: f64,
    pub spread_blend: f64,
    pub swell: f64,
    pub peak_enhancement: f64,
    pub short_waves_fade: f64,
}

impl WaveSystem {
    /// `waveSystem(o)` defaults.
    pub const fn default_system() -> Self {
        Self { scale: 1.0, wind_speed: 8.0, wind_direction: 20.0, fetch: 200.0, spread_blend: 0.9, swell: 0.2, peak_enhancement: 3.3, short_waves_fade: 0.01 }
    }
    /// The local wind sea `buildSpectrum` uses by default.
    pub const fn js_local() -> Self {
        Self { wind_speed: 7.0, wind_direction: 25.0, fetch: 120.0, spread_blend: 0.85, swell: 0.05, ..Self::default_system() }
    }
    /// The swell `buildSpectrum` uses by default.
    pub const fn js_swell() -> Self {
        Self { scale: 0.48, wind_speed: 6.0, wind_direction: 5.0, fetch: 1200.0, spread_blend: 1.0, swell: 0.9, short_waves_fade: 0.1, ..Self::default_system() }
    }
}

fn jonswap_alpha(fetch: f64, wind: f64) -> f64 {
    0.076 * (GRAVITY * fetch / (wind * wind)).powf(-0.22)
}

fn jonswap_peak_omega(fetch: f64, wind: f64) -> f64 {
    22.0 * ((GRAVITY * GRAVITY) / (wind * fetch)).powf(1.0 / 3.0)
}

fn tma_correction(omega: f64, depth: f64) -> f64 {
    let omega_h = omega * (depth / GRAVITY).sqrt();
    if omega_h <= 1.0 {
        return 0.5 * omega_h * omega_h;
    }
    if omega_h < 2.0 {
        return 1.0 - 0.5 * (2.0 - omega_h) * (2.0 - omega_h);
    }
    1.0
}

fn jonswap(omega: f64, depth: f64, s: &WaveSystem) -> f64 {
    let alpha = jonswap_alpha(s.fetch * 1000.0, s.wind_speed);
    let peak = jonswap_peak_omega(s.fetch * 1000.0, s.wind_speed);
    let sigma = if omega <= peak { 0.07 } else { 0.09 };
    let r = (-((omega - peak).powi(2)) / (2.0 * sigma * sigma * peak * peak)).exp();
    let one_over = 1.0 / omega.max(1e-6);
    let pr = peak * one_over;
    tma_correction(omega, depth) * alpha * GRAVITY * GRAVITY * one_over.powi(5) * (-1.25 * pr.powi(4)).exp() * s.peak_enhancement.powf(r)
}

fn normalisation_factor(s: f64) -> f64 {
    let (s2, s3, s4) = (s * s, s * s * s, s * s * s * s);
    if s < 5.0 {
        return -0.000564 * s4 + 0.00776 * s3 - 0.044 * s2 + 0.192 * s + 0.163;
    }
    -4.80e-8 * s4 + 1.07e-5 * s3 - 9.53e-4 * s2 + 5.90e-2 * s + 3.93e-1
}

fn cosine2s(theta: f64, s: f64) -> f64 {
    normalisation_factor(s) * (theta / 2.0).cos().abs().powf(2.0 * s)
}

fn spread_power(omega: f64, peak: f64) -> f64 {
    if omega > peak { 9.77 * (omega / peak).powf(-2.5) } else { 6.97 * (omega / peak).powf(5.0) }
}

fn direction_spectrum(theta: f64, omega: f64, s: &WaveSystem) -> f64 {
    let peak = jonswap_peak_omega(s.fetch * 1000.0, s.wind_speed);
    let sp = spread_power(omega, peak) + 16.0 * (omega / peak).min(20.0).tanh() * s.swell * s.swell;
    let angle = s.wind_direction * std::f64::consts::PI / 180.0;
    let a = 2.0 / std::f64::consts::PI * (theta - angle).cos().powi(2);
    let b = cosine2s(theta - angle, sp);
    a + (b - a) * s.spread_blend
}

/// JS `x >>> 0` of a double: its integer value modulo 2^32.
fn to_u32(v: f64) -> u32 {
    v.trunc().rem_euclid(4294967296.0) as u32
}

/// JS `x ^ y` operands: ToInt32.
fn to_i32(v: u32) -> i32 {
    v as i32
}

/// `pcg(seed)`: the JS arithmetic, doubles and all.
fn pcg(seed: f64) -> f64 {
    let state = to_u32(seed * 747796405.0 + 2891336453.0);
    let shifted = state >> ((state >> 28) + 4);
    let xored = to_i32(shifted) ^ to_i32(state);
    let word = to_u32(xored as f64 * 277803737.0);
    let out = ((to_i32(word >> 22) ^ to_i32(word)) as i64).rem_euclid(1 << 32) as u32;
    out as f64 / 4294967296.0
}

fn gaussian_pair(seed: f64) -> (f64, f64) {
    let u1 = pcg(seed).max(1e-7);
    let u2 = pcg(seed + 2654435769.0); // 0x9e3779b9
    let r = (-2.0 * u1.ln()).sqrt();
    (r * (TWO_PI * u2).cos(), r * (TWO_PI * u2).sin())
}

pub struct Spectrum {
    /// Per texel (cascade, m, n): h0(k).re, h0(k).im, and the second draw.
    pub h0: Vec<[f32; 4]>,
    /// Per texel: kx, kz, |k|, omega.
    pub wave: Vec<[f32; 4]>,
    pub significant_height: f64,
    pub raw_significant_height: f64,
    pub scale: f64,
    pub sizes: [f64; CASCADES],
}

/// `buildSpectrum({})` with the JS defaults.
pub fn build_spectrum() -> Spectrum {
    build_spectrum_with(&CASCADE_SIZES, DEPTH_M, SEED, SIGNIFICANT_HEIGHT_M, &[WaveSystem::js_local(), WaveSystem::js_swell()])
}

pub fn build_spectrum_with(sizes: &[f64; CASCADES], depth: f64, seed: f64, significant_height: f64, systems: &[WaveSystem]) -> Spectrum {
    let total = N * N * CASCADES;
    let mut h0 = vec![[0f32; 4]; total];
    let mut wave = vec![[0f32; 4]; total];
    let half = (N / 2) as f64;
    for c in 0..CASCADES {
        let size = sizes[c];
        let cut_low = if c == 0 { 0.0001 } else { (TWO_PI * half) / sizes[c - 1] * 0.5 };
        let cut_high = if c == CASCADES - 1 { 1e9 } else { (TWO_PI * half) / size * 0.5 };
        for m in 0..N {
            for n in 0..N {
                let index = c * N * N + m * N + n;
                let kx = (n as f64 - half) * TWO_PI / size;
                let kz = (m as f64 - half) * TWO_PI / size;
                let k = kx.hypot(kz);
                let kd = (k * depth).min(20.0);
                let mut omega = (GRAVITY * k * kd.tanh()).sqrt();
                if omega.is_nan() {
                    omega = 0.0;
                }
                wave[index] = [kx as f32, kz as f32, k as f32, omega as f32];
                if k < cut_low || k >= cut_high || k < 1e-6 {
                    continue;
                }
                let theta = kz.atan2(kx);
                let d_omega_dk = if omega > 1e-6 { GRAVITY * (kd.tanh() + kd * (1.0 - kd.tanh().powi(2))) / (2.0 * omega) } else { 0.0 };
                let mut spectrum = 0.0;
                for s in systems {
                    let v = jonswap(omega, depth, s) * direction_spectrum(theta, omega, s) * s.scale;
                    spectrum += v * (-(k * k) * s.short_waves_fade * s.short_waves_fade).exp();
                }
                let dk = TWO_PI / size;
                let amplitude = (2.0 * spectrum * d_omega_dk.abs() / k.max(1e-6) * dk * dk).sqrt();
                let base = seed + index as f64 * 2654435761.0;
                let (g0, g1) = gaussian_pair(base);
                let (g2, g3) = gaussian_pair(base + 2246822507.0); // 0x85ebca6b
                let r = std::f64::consts::FRAC_1_SQRT_2;
                h0[index] = [(g0 * amplitude * r) as f32, (g1 * amplitude * r) as f32, (g2 * amplitude * r) as f32, (-g3 * amplitude * r) as f32];
            }
        }
    }
    let mut variance = 0.0f64;
    for t in &h0 {
        for v in t {
            variance += (*v as f64) * (*v as f64);
        }
    }
    let measured = 4.0 * variance.sqrt();
    let scale = if measured > 1e-6 { significant_height / measured } else { 1.0 };
    for t in &mut h0 {
        for v in t.iter_mut() {
            *v = (*v as f64 * scale) as f32;
        }
    }
    Spectrum { h0, wave, significant_height, raw_significant_height: measured, scale, sizes: *sizes }
}

impl Spectrum {
    /// h(k, t) as `evolve` forms it: h0(k) e^{iwt} + conj(h0'(k)) e^{-iwt}.
    pub fn h_at(&self, i: usize, t: f64) -> (f64, f64) {
        let h = self.h0[i];
        let w = self.wave[i][3] as f64;
        let (s, c) = (w * t).sin_cos();
        let (x, y, z, ww) = (h[0] as f64, h[1] as f64, h[2] as f64, h[3] as f64);
        (x * c - y * s + z * c + ww * s, x * s + y * c - z * s + ww * c)
    }

    /// Sea height at a world point: the sum over every component the GPU
    /// transform sums (the real part, as `finalize` keeps it).
    pub fn height_at(&self, x: f64, z: f64, t: f64) -> f64 {
        let mut h = 0.0;
        for i in 0..self.h0.len() {
            let w = self.wave[i];
            if self.h0[i] == [0.0; 4] {
                continue;
            }
            let (re, im) = self.h_at(i, t);
            let th = w[0] as f64 * x + w[1] as f64 * z;
            h += re * th.cos() - im * th.sin();
        }
        h
    }

    /// One cascade's height at a world point (the field the GPU writes to that cascade's layer).
    pub fn cascade_height_at(&self, c: usize, x: f64, z: f64, t: f64) -> f64 {
        let mut h = 0.0;
        for i in c * N * N..(c + 1) * N * N {
            if self.h0[i] == [0.0; 4] {
                continue;
            }
            let w = self.wave[i];
            let (re, im) = self.h_at(i, t);
            let th = w[0] as f64 * x + w[1] as f64 * z;
            h += re * th.cos() - im * th.sin();
        }
        h
    }

    /// Significant height carried by one cascade (4 sqrt of its variance).
    pub fn cascade_significant_height(&self, c: usize) -> f64 {
        let v: f64 = self.h0[c * N * N..(c + 1) * N * N].iter().map(|t| t.iter().map(|v| (*v as f64).powi(2)).sum::<f64>()).sum();
        4.0 * v.sqrt()
    }
}

/// `cascadeBandWeights(fft, footprint)`: a band fades out as the footprint
/// grows from half its shortest wave to half its longest.
pub fn band_weights(footprint: f64) -> [f64; CASCADES] {
    let mut w = [0.0; CASCADES];
    for c in 0..CASCADES {
        let lo = CASCADE_SIZES[c] / (N / 2) as f64;
        let hi = if c == 0 { lo * 8.0 } else { CASCADE_SIZES[c - 1] / (N / 2) as f64 };
        let t = ((footprint - lo) / (hi - lo)).clamp(0.0, 1.0);
        w[c] = 1.0 - t * t * (3.0 - 2.0 * t);
    }
    w
}

/// CPU reference of the GPU transform's schedule: an in-order inverse DFT
/// (radix-2 Stockham, e^{+2 pi i}), unnormalised, as `verifyOceanFftMath`.
pub fn stockham_inverse(re: &mut Vec<f64>, im: &mut Vec<f64>) {
    let n = re.len();
    let (mut sr, mut si) = (re.clone(), im.clone());
    let (mut dr, mut di) = (vec![0.0; n], vec![0.0; n]);
    let mut ns = 1;
    while ns < n {
        for j in 0..n / 2 {
            let k = j % ns;
            let angle = TWO_PI * k as f64 / (2 * ns) as f64;
            let (wr, wi) = (angle.cos(), angle.sin());
            let (ar, ai) = (sr[j], si[j]);
            let (br0, bi0) = (sr[j + n / 2], si[j + n / 2]);
            let (br, bi) = (br0 * wr - bi0 * wi, br0 * wi + bi0 * wr);
            let d = (j / ns) * ns * 2 + k;
            dr[d] = ar + br;
            di[d] = ai + bi;
            dr[d + ns] = ar - br;
            di[d + ns] = ai - bi;
        }
        std::mem::swap(&mut sr, &mut dr);
        std::mem::swap(&mut si, &mut di);
        ns *= 2;
    }
    *re = sr;
    *im = si;
}
