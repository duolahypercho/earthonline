//! A physical clear-sky atmosphere as pure functions: the medium, the
//! transmittance, single and multiple scattering, the sky's radiance, the
//! sun's colour through the air and the sky's irradiance, all on the same
//! model the GPU's atmosphere tables integrate (eo-render sky_atmo.wgsl and
//! sky_atmo_luts.wgsl; Hillaire 2020, "A Scalable and Production Ready Sky
//! and Atmosphere Rendering Technique", EGSR, after Bruneton and Neyret 2008).
//!
//! Units: kilometres for the medium, radiance and irradiance per unit of
//! solar illuminance at the top of the atmosphere (a white sun), three
//! channels at 680, 550 and 440 nm (Hillaire's and Bruneton's). (Evaluating
//! the channels at the dominant wavelengths of the sRGB primaries, 611 / 549 /
//! 464 nm, was tried: with one wavelength per channel the red primary's
//! negative lobe in the blue is missed and the clear sky came out lavender.)
//!
//! The medium (`Atmosphere::coastal_clear`):
//!
//! * Rayleigh scattering (5.802, 13.558, 33.1) e-3 / km at sea level, scale
//!   height 8 km (Hillaire 2020 table 1, after Bucholtz 1995, "Rayleigh-scattering
//!   calculations for the terrestrial atmosphere", Appl. Opt. 34).
//! * Aerosol: clean coastal marine air on a clear day. Aerosol optical depth
//!   0.04 at 550 nm (clean maritime AOD(500) about 0.03-0.1: Smirnov et al.
//!   2002, "Optical properties of atmospheric aerosol in maritime
//!   environments", J. Atmos. Sci. 59, 501-523), Angstrom exponent 1.0 (the
//!   coastal mix of sea salt, 0.3-0.7 in Smirnov et al., and urban fine mode,
//!   1.2-1.8), single-scattering albedo 0.95 (Dubovik et al. 2002,
//!   "Variability of absorption and optical properties of key aerosol types
//!   observed in worldwide locations", J. Atmos. Sci. 59, 590-608: oceanic
//!   0.97-0.98, urban 0.90-0.98), in a boundary layer of scale height 1.0 km
//!   (the marine layer's inversion over San Francisco is typically 0.4-1 km).
//!   Visibility at the ground (Koschmieder, 3.912 / extinction at 550 nm):
//!   3.912 / (0.040 + 0.0137) ~ 73 km, a clear day's (San Francisco's clear-day
//!   visibility is 30-80 km). The phase is the GPU's: 0.55
//!   Cornette-Shanks g 0.6 + 0.45 Henyey-Greenstein g 0.93 (an effective
//!   asymmetry of 0.78, within the maritime coarse mode's 0.75-0.8 in
//!   Dubovik et al.).
//! * Ozone: (1.881, 1.881, 0.085) e-3 / km at its peak in a 30 km tent about
//!   25 km (peak absorption = optical depth / 15 km): a column of about 300
//!   Dobson units (8.07e18 molecules / cm2) through the Chappuis band's
//!   cross-section of ~3.5e-21 cm2 at 550 nm (Serdyuchenko et al. 2014, "High
//!   spectral resolution ozone absorption cross-sections", Atmos. Meas. Tech.
//!   7, 625-636), optical depth 0.028; the red channel at the green's (the
//!   band's mean over the red primary, which peaks at 603 nm), as the GPU
//!   tables had it.
//!
//! The sun at the top of the atmosphere: the luminous solar constant 133.8
//! klx (Darula, Kittler and Gueymard 2005, "Reference luminous solar constant
//! and solar luminance for illuminance calculations", Solar Energy 79,
//! 559-565), in the colour of a 5772 K Planckian radiator (the Sun's
//! effective temperature, IAU 2015 resolution B3; chromaticity from Kim et
//! al. 2002's fit of the Planckian locus), in linear sRGB.

use crate::math::*;
use std::sync::{Mutex, OnceLock};

pub const R_GROUND_KM: f64 = 6360.0;
pub const R_TOP_KM: f64 = 6460.0;
/// The luminous solar constant (lx), Darula, Kittler and Gueymard 2005.
pub const LUMINOUS_SOLAR_CONSTANT_LX: f64 = 133_800.0;
/// The Sun's effective temperature (K), IAU 2015 resolution B3.
pub const SUN_EFFECTIVE_TEMPERATURE_K: f64 = 5772.0;
/// The channels' wavelengths (nm).
pub const CHANNEL_NM: [f64; 3] = [680.0, 550.0, 440.0];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Atmosphere {
    /// Rayleigh scattering at sea level (1/km).
    pub rayleigh: Rgb,
    pub rayleigh_h_km: f64,
    /// Aerosol scattering and absorption at the ground at 550 nm (1/km).
    pub aerosol_scat: f64,
    pub aerosol_abs: f64,
    pub aerosol_h_km: f64,
    /// The aerosol's spectral factor per channel, (lambda / 550)^-alpha.
    pub aerosol_spectral: Rgb,
    /// Ozone absorption at its peak (1/km), in a tent of half width 15 km about 25 km.
    pub ozone: Rgb,
}

impl Atmosphere {
    /// Clean coastal marine air on a clear day (see the module notes).
    pub fn coastal_clear() -> Self {
        Self::coastal(COASTAL_AOD_550, COASTAL_ANGSTROM, COASTAL_SSA, COASTAL_AEROSOL_H_KM)
    }

    /// A coastal medium from an aerosol optical depth at 550 nm, an Angstrom
    /// exponent, a single-scattering albedo and the aerosol's scale height.
    pub fn coastal(aod_550: f64, angstrom: f64, ssa: f64, h_km: f64) -> Self {
        let ext = aod_550 / h_km;
        Self {
            rayleigh: RAYLEIGH_SEA_LEVEL,
            rayleigh_h_km: 8.0,
            aerosol_scat: ext * ssa,
            aerosol_abs: ext * (1.0 - ssa),
            aerosol_h_km: h_km,
            aerosol_spectral: CHANNEL_NM.map(|l| (l / 550.0).powf(-angstrom)),
            ozone: OZONE_PEAK,
        }
    }

    /// A medium between two (by `w`, 0 `a`, 1 `b`), channel by channel.
    pub fn lerp(a: &Self, b: &Self, w: f64) -> Self {
        let m3 = |x: Rgb, y: Rgb| [0, 1, 2].map(|i| mix(x[i], y[i], w));
        Self {
            rayleigh: m3(a.rayleigh, b.rayleigh),
            rayleigh_h_km: mix(a.rayleigh_h_km, b.rayleigh_h_km, w),
            aerosol_scat: mix(a.aerosol_scat, b.aerosol_scat, w),
            aerosol_abs: mix(a.aerosol_abs, b.aerosol_abs, w),
            aerosol_h_km: mix(a.aerosol_h_km, b.aerosol_h_km, w),
            aerosol_spectral: m3(a.aerosol_spectral, b.aerosol_spectral),
            ozone: m3(a.ozone, b.ozone),
        }
    }

    /// The medium the GPU tables used before this model (Hillaire's standard
    /// aerosol, 3.996e-3 / 4.4e-3 per km, scale height 1.2 km, Angstrom factors
    /// 0.76 / 1 / 1.34): `--no-phys-sky` keeps it.
    pub fn legacy() -> Self {
        Self {
            rayleigh: RAYLEIGH_SEA_LEVEL,
            rayleigh_h_km: 8.0,
            aerosol_scat: 3.996e-3,
            aerosol_abs: 4.40e-3,
            aerosol_h_km: 1.2,
            aerosol_spectral: [0.76, 1.0, 1.34],
            ozone: OZONE_PEAK,
        }
    }

    /// Scattering (Rayleigh, aerosol) and extinction at a height (km), the
    /// aerosol x `density` (the weather's haze).
    pub fn medium(&self, h_km: f64, density: f64) -> (Rgb, Rgb, Rgb) {
        let hh = h_km.max(0.0);
        let dr = (-hh / self.rayleigh_h_km).exp();
        let dm = (-hh / self.aerosol_h_km).exp() * density;
        let doz = (1.0 - (hh - 25.0).abs() / 15.0).max(0.0);
        let ray = self.rayleigh.map(|b| b * dr);
        let mie = [0, 1, 2].map(|i| self.aerosol_scat * dm * self.aerosol_spectral[i]);
        let ext = [0, 1, 2].map(|i| ray[i] + (self.aerosol_scat + self.aerosol_abs) * dm * self.aerosol_spectral[i] + self.ozone[i] * doz);
        (ray, mie, ext)
    }

    /// Extinction at 550 nm at the ground (1/km), and the visibility it gives (km, Koschmieder).
    pub fn ground_extinction_550(&self, density: f64) -> f64 {
        self.medium(0.0, density).2[1]
    }
}

pub const RAYLEIGH_SEA_LEVEL: Rgb = [5.802e-3, 13.558e-3, 33.1e-3];
pub const OZONE_PEAK: Rgb = [1.881e-3, 1.881e-3, 0.085e-3];
pub const COASTAL_AOD_550: f64 = 0.04;
pub const COASTAL_ANGSTROM: f64 = 1.0;
pub const COASTAL_SSA: f64 = 0.95;
pub const COASTAL_AEROSOL_H_KM: f64 = 1.0;

pub fn rayleigh_phase(c: f64) -> f64 {
    3.0 / (16.0 * std::f64::consts::PI) * (1.0 + c * c)
}

/// The aerosol phase the GPU uses (sky_atmo.wgsl `at_mie_phase`).
pub fn aerosol_phase(c: f64) -> f64 {
    let pi = std::f64::consts::PI;
    let g: f64 = 0.6;
    let k = 3.0 / (8.0 * pi) * (1.0 - g * g) / (2.0 + g * g);
    let cs = k * (1.0 + c * c) / (1.0 + g * g - 2.0 * g * c).max(1e-4).powf(1.5);
    let h: f64 = 0.93;
    let hg = (1.0 - h * h) / (4.0 * pi * (1.0 + h * h - 2.0 * h * c).max(1e-5).powf(1.5));
    0.55 * cs + 0.45 * hg
}

/// Distance from radius r along cosine mu to the sphere of radius rs (the
/// nearer positive root), negative if it misses (sky_atmo.wgsl `at_sphere`).
pub fn sphere(r: f64, mu: f64, rs: f64) -> f64 {
    let b = r * mu;
    let c = r * r - rs * rs;
    let disc = b * b - c;
    if disc < 0.0 {
        return -1.0;
    }
    let s = disc.sqrt();
    let (t0, t1) = (-b - s, -b + s);
    if t0 > 0.0 { t0 } else { t1 }
}

pub fn ground_hit(r: f64, mu: f64) -> bool {
    mu < 0.0 && r * r * (mu * mu - 1.0) + R_GROUND_KM * R_GROUND_KM >= 0.0
}

/// The colour of a Planckian radiator in linear sRGB, luminance 1 (Kim et
/// al. 2002's cubic fit of the Planckian locus, 4000-25000 K, then CIE 1931
/// xyY to the sRGB primaries of IEC 61966-2-1).
pub fn planck_srgb(t: f64) -> Rgb {
    let t = t.clamp(4000.0, 25000.0);
    let x = -3.0258469e9 / (t * t * t) + 2.1070379e6 / (t * t) + 0.2226347e3 / t + 0.240390;
    let y = 3.0817580 * x * x * x - 5.87338670 * x * x + 3.75112997 * x - 0.37001483;
    xy_to_srgb(x, y)
}

/// CIE xy (luminance 1) to linear sRGB.
pub fn xy_to_srgb(x: f64, y: f64) -> Rgb {
    let (xx, yy, zz) = (x / y, 1.0, (1.0 - x - y) / y);
    [
        3.2406 * xx - 1.5372 * yy - 0.4986 * zz,
        -0.9689 * xx + 1.8758 * yy + 0.0415 * zz,
        0.0557 * xx - 0.2040 * yy + 1.0570 * zz,
    ]
}

/// The sun's colour above the atmosphere (linear sRGB, luminance 1).
pub fn sun_colour_toa() -> Rgb {
    planck_srgb(SUN_EFFECTIVE_TEMPERATURE_K)
}

const T_NH: usize = 32;
const T_NMU: usize = 128;
const T_HMAX: f64 = 100.0;
const MS_N: usize = 16;

/// The transmittance and multiple-scattering tables of one medium (and haze density).
pub struct Tables {
    pub atm: Atmosphere,
    pub density: f64,
    trans: Vec<Rgb>,
    ms: Vec<Rgb>,
}

fn h_of(u: f64) -> f64 {
    u * u * T_HMAX
}
fn u_of(h: f64) -> f64 {
    (h / T_HMAX).clamp(0.0, 1.0).sqrt()
}
fn mu_of(v: f64) -> f64 {
    let c = v * 2.0 - 1.0;
    c.signum() * c * c
}
fn v_of(mu: f64) -> f64 {
    0.5 + 0.5 * mu.signum() * mu.abs().sqrt()
}

impl Tables {
    pub fn new(atm: Atmosphere, density: f64) -> Self {
        let mut t = Self { atm, density, trans: vec![[0.0; 3]; T_NH * T_NMU], ms: vec![[0.0; 3]; MS_N * MS_N] };
        for j in 0..T_NH {
            let h = h_of(j as f64 / (T_NH - 1) as f64);
            let r = R_GROUND_KM + h.max(0.001);
            for i in 0..T_NMU {
                let mu = mu_of(i as f64 / (T_NMU - 1) as f64);
                t.trans[j * T_NMU + i] = if ground_hit(r, mu) { [0.0; 3] } else { t.integrate_transmittance(r, mu) };
            }
        }
        for j in 0..MS_N {
            for i in 0..MS_N {
                let mu_s = (i as f64 + 0.5) / MS_N as f64 * 2.0 - 1.0;
                let v = ((j as f64 + 0.5) / MS_N as f64).max(0.002);
                let r = R_GROUND_KM + v * (R_TOP_KM - R_GROUND_KM);
                t.ms[j * MS_N + i] = t.integrate_ms(r, mu_s);
            }
        }
        t
    }

    fn integrate_transmittance(&self, r: f64, mu: f64) -> Rgb {
        let len = sphere(r, mu, R_TOP_KM).max(0.0);
        let steps = 40;
        let dt = len / steps as f64;
        let mut tau = [0.0; 3];
        for i in 0..steps {
            let t = (i as f64 + 0.5) * dt;
            let h = (r * r + t * t + 2.0 * r * mu * t).sqrt() - R_GROUND_KM;
            let e = self.atm.medium(h, self.density).2;
            for c in 0..3 {
                tau[c] += e[c] * dt;
            }
        }
        tau.map(|x| (-x).exp())
    }

    /// Transmittance from radius r along mu to the top of the atmosphere (0 into the ground).
    pub fn transmittance(&self, r: f64, mu: f64) -> Rgb {
        if ground_hit(r, mu) {
            return [0.0; 3];
        }
        let u = u_of(r - R_GROUND_KM) * (T_NH - 1) as f64;
        let v = v_of(mu.clamp(-1.0, 1.0)) * (T_NMU - 1) as f64;
        let (j0, i0) = ((u.floor() as usize).min(T_NH - 2), (v.floor() as usize).min(T_NMU - 2));
        let (fu, fv) = (u - j0 as f64, v - i0 as f64);
        let at = |j: usize, i: usize| self.trans[j * T_NMU + i];
        let (a, b, c, d) = (at(j0, i0), at(j0, i0 + 1), at(j0 + 1, i0), at(j0 + 1, i0 + 1));
        [0, 1, 2].map(|k| mix(mix(a[k], b[k], fv), mix(c[k], d[k], fv), fu))
    }

    /// Hillaire 2020 section 5.5 (sky_atmo_luts.wgsl `at_ms_main`): second order
    /// over 64 directions, isotropic, and its geometric series.
    fn integrate_ms(&self, r: f64, mu_s: f64) -> Rgb {
        let sun = [(1.0 - mu_s * mu_s).max(0.0).sqrt(), mu_s, 0.0];
        let mut l2 = [0.0; 3];
        let mut fms = [0.0; 3];
        let nd = 64;
        for k in 0..nd {
            let z = 1.0 - (k as f64 + 0.5) * 2.0 / nd as f64;
            let rr = (1.0 - z * z).max(0.0).sqrt();
            let ph = k as f64 * 2.39996323;
            let dir = [rr * ph.cos(), z, rr * ph.sin()];
            let mu = dir[1];
            let mut len = sphere(r, mu, R_TOP_KM);
            let g = sphere(r, mu, R_GROUND_KM);
            if g > 0.0 {
                len = g;
            }
            let len = len.max(0.0);
            let steps = 16;
            let dt = len / steps as f64;
            let mut trans = [1.0; 3];
            for i in 0..steps {
                let t = (i as f64 + 0.5) * dt;
                let p = [dir[0] * t, r + dir[1] * t, dir[2] * t];
                let pr = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
                let (ray, mie, ext) = self.atm.medium(pr - R_GROUND_KM, self.density);
                let up = [p[0] / pr, p[1] / pr, p[2] / pr];
                let ts = self.transmittance(pr, up[0] * sun[0] + up[1] * sun[1] + up[2] * sun[2]);
                for c in 0..3 {
                    let st = (-ext[c] * dt).exp();
                    let kint = (1.0 - st) / ext[c].max(1e-7);
                    let scat = ray[c] + mie[c];
                    l2[c] += trans[c] * scat * ts[c] / (4.0 * std::f64::consts::PI) * kint;
                    fms[c] += trans[c] * scat * kint;
                    trans[c] *= st;
                }
            }
        }
        [0, 1, 2].map(|c| (l2[c] / nd as f64) / (1.0 - fms[c] / nd as f64).max(1e-3))
    }

    fn multi(&self, r: f64, mu_s: f64) -> Rgb {
        let u = ((mu_s * 0.5 + 0.5) * MS_N as f64 - 0.5).clamp(0.0, (MS_N - 1) as f64);
        let v = (((r - R_GROUND_KM) / (R_TOP_KM - R_GROUND_KM)) * MS_N as f64 - 0.5).clamp(0.0, (MS_N - 1) as f64);
        let (i0, j0) = ((u.floor() as usize).min(MS_N - 2), (v.floor() as usize).min(MS_N - 2));
        let (fu, fv) = (u - i0 as f64, v - j0 as f64);
        let at = |j: usize, i: usize| self.ms[j * MS_N + i];
        let (a, b, c, d) = (at(j0, i0), at(j0, i0 + 1), at(j0 + 1, i0), at(j0 + 1, i0 + 1));
        [0, 1, 2].map(|k| mix(mix(a[k], b[k], fu), mix(c[k], d[k], fu), fv))
    }

    /// The sky's radiance along `dir` from an eye `eye_m` metres up, per
    /// unit solar illuminance at the top of the atmosphere (a white sun), as
    /// the GPU's sky-view table integrates it (`at_view_main`, 32 steps).
    pub fn sky_radiance(&self, dir: [f64; 3], sun: [f64; 3], eye_m: f64) -> Rgb {
        let r = R_GROUND_KM + eye_m.max(1.0) * 0.001;
        let mu = dir[1];
        let mut len = sphere(r, mu, R_TOP_KM);
        let g = sphere(r, mu, R_GROUND_KM);
        if g > 0.0 {
            len = g;
        }
        let len = len.clamp(0.0, 1200.0);
        let steps = 32;
        let cth = dir[0] * sun[0] + dir[1] * sun[1] + dir[2] * sun[2];
        let (pr_, pm_) = (rayleigh_phase(cth), aerosol_phase(cth));
        let mut lum = [0.0; 3];
        let mut trans = [1.0; 3];
        let mut t_prev = 0.0;
        for i in 0..steps {
            let f = (i as f64 + 0.5) / steps as f64;
            let t = len * f * f;
            let f1 = (i as f64 + 1.0) / steps as f64;
            let t_next = len * f1 * f1;
            let dt = t_next - t_prev;
            t_prev = t_next;
            let p = [dir[0] * t, r + dir[1] * t, dir[2] * t];
            let pr = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
            let (ray, mie, ext) = self.atm.medium(pr - R_GROUND_KM, self.density);
            let up = [p[0] / pr, p[1] / pr, p[2] / pr];
            let mus = up[0] * sun[0] + up[1] * sun[1] + up[2] * sun[2];
            let ts = self.transmittance(pr, mus);
            let ms = self.multi(pr, mus);
            for c in 0..3 {
                let single = (ray[c] * pr_ + mie[c] * pm_) * ts[c];
                let multi = (ray[c] + mie[c]) * ms[c];
                let st = (-ext[c] * dt).exp();
                let kint = (1.0 - st) / ext[c].max(1e-7);
                lum[c] += trans[c] * (single + multi) * kint;
                trans[c] *= st;
            }
        }
        lum
    }

    /// The direct sun's transmittance to a height (m) for a sun at `sun_y`
    /// (sine of its altitude); zero where the planet hides it.
    pub fn sun_transmittance(&self, h_m: f64, sun_y: f64) -> Rgb {
        self.transmittance(R_GROUND_KM + h_m.max(1.0) * 0.001, sun_y)
    }
}

/// The tables for a medium and density, cached (the last few asked for).
pub fn tables(atm: &Atmosphere, density: f64) -> std::sync::Arc<Tables> {
    static CACHE: OnceLock<Mutex<Vec<std::sync::Arc<Tables>>>> = OnceLock::new();
    let c = CACHE.get_or_init(|| Mutex::new(Vec::new()));
    let mut v = c.lock().unwrap();
    if let Some(t) = v.iter().find(|t| t.atm == *atm && t.density == density) {
        return t.clone();
    }
    let t = std::sync::Arc::new(Tables::new(*atm, density));
    v.push(t.clone());
    if v.len() > 6 {
        v.remove(0);
    }
    t
}

/// SH9 basis (real, l <= 2), the order of `LookFrame::env_sh`.
fn sh_basis(d: [f64; 3]) -> [f64; 9] {
    let [x, y, z] = d;
    [
        0.282095,
        0.488603 * y,
        0.488603 * z,
        0.488603 * x,
        1.092548 * x * y,
        1.092548 * y * z,
        0.315392 * (3.0 * z * z - 1.0),
        1.092548 * x * z,
        0.546274 * (x * x - y * y),
    ]
}

/// The clear sky's light at the ground for a sun direction, per unit solar
/// illuminance at the top of the atmosphere (a white sun):
#[derive(Clone, Copy, Debug)]
pub struct SkyLight {
    /// the direct beam's transmittance (irradiance on a normal to the sun).
    pub sun: Rgb,
    /// the sky's irradiance on a horizontal surface.
    pub sky_h: Rgb,
    /// the cosine-convolved SH9 of the sky (upper hemisphere) and of the
    /// ground below the horizon (a Lambertian ground of `ground_albedo` under
    /// the sun and the sky): E(n) = sum c_i Y_i(n).
    pub sh: [Rgb; 9],
    /// the ground's share of `sh` alone (the sky's is `sh` less it).
    pub sh_ground: [Rgb; 9],
}

/// Integrate the sky's light at the ground (24 x 48 directions, the upper half marched).
pub fn sky_light(t: &Tables, sun: [f64; 3], ground_albedo: Rgb) -> SkyLight {
    let rings = 24;
    let sectors = 48;
    let pi = std::f64::consts::PI;
    let ts = t.sun_transmittance(2.0, sun[1]);
    let mut sky_c = [[0.0; 3]; 9];
    let mut gnd_c = [0.0; 9];
    let mut sky_h = [0.0; 3];
    let dw = (pi / rings as f64) * (TAU / sectors as f64);
    for i in 0..rings {
        let theta = (i as f64 + 0.5) / rings as f64 * pi;
        let (st, ct) = theta.sin_cos();
        for j in 0..sectors {
            let phi = (j as f64 + 0.5) / sectors as f64 * TAU;
            let d = [st * phi.cos(), ct, st * phi.sin()];
            let y = sh_basis(d);
            let w = st * dw;
            if ct > 0.0 {
                let l = t.sky_radiance(d, sun, 2.0);
                for c in 0..3 {
                    sky_h[c] += l[c] * ct * w;
                }
                for k in 0..9 {
                    for c in 0..3 {
                        sky_c[k][c] += l[c] * y[k] * w;
                    }
                }
            } else {
                for k in 0..9 {
                    gnd_c[k] += y[k] * w;
                }
            }
        }
    }
    // the ground below the horizon: Lambertian, lit by the sun and the sky.
    let e_h = [0, 1, 2].map(|c| ts[c] * sun[1].max(0.0) + sky_h[c]);
    let l_g = [0, 1, 2].map(|c| ground_albedo[c] * e_h[c] / pi);
    let band = [pi, 2.0 * pi / 3.0, pi / 4.0];
    let mut sh = [[0.0; 3]; 9];
    let mut sh_ground = [[0.0; 3]; 9];
    for k in 0..9 {
        let a = band[if k == 0 { 0 } else if k < 4 { 1 } else { 2 }];
        for c in 0..3 {
            sh[k][c] = (sky_c[k][c] + gnd_c[k] * l_g[c]) * a;
            sh_ground[k][c] = gnd_c[k] * l_g[c] * a;
        }
    }
    SkyLight { sun: spectral_chroma(ts), sky_h: spectral_chroma(sky_h), sh: sh.map(spectral_chroma), sh_ground: sh_ground.map(spectral_chroma) }
}

/// Evaluate a cosine-convolved SH9.
pub fn sh_eval(c: &[Rgb; 9], n: [f64; 3]) -> Rgb {
    let y = sh_basis(n);
    let mut out = [0.0; 3];
    for k in 0..9 {
        for ch in 0..3 {
            out[ch] += c[k][ch] * y[k];
        }
    }
    out
}

/// The three-channel model's chroma correction: one wavelength per channel
/// overstates the colour of a broad spectrum. For a lambda^-4 sky under a
/// 5800 K sun the channels give 0.175 : 0.41 : 1 (linear sRGB) where the
/// spectrum taken through the CIE 1931 observer gives 0.25 : 0.47 : 1; the
/// same holds for the reddened beam. Taking the result `RGB_CHROMA` of the way
/// from its luminance to itself reproduces the spectral colour of both
/// (0.8: 0.250 : 0.464 : 1 for the Rayleigh sky). Applied to the sky's
/// radiance, its irradiance and the sun's transmittance wherever the physical
/// daylight lights the frame (the GPU's sky tables too: sky_volume.wgsl, look_light.wgsl).
pub const RGB_CHROMA: f64 = 0.8;

/// `RGB_CHROMA` on one colour.
pub fn spectral_chroma(c: Rgb) -> Rgb {
    let l = luminance(c);
    c.map(|v| l + (v - l) * RGB_CHROMA)
}

/// The mean albedo of the ground about the city, for the ground's light on
/// walls and the underside of things: 0.15, the middle of the 0.10-0.20
/// measured for the urban surface of US cities (Taha 1997, "Modeling the
/// impacts of large-scale albedo changes on ozone air quality in the South
/// Coast Air Basin", Atmos. Environ. 31; Akbari et al. 2009), a warm grey
/// (asphalt, concrete and roofs).
pub const URBAN_GROUND_ALBEDO: Rgb = [0.165, 0.150, 0.130];

#[cfg(test)]
mod tests {
    use super::*;

    fn sun_at(alt_deg: f64) -> [f64; 3] {
        let a = alt_deg * DEG;
        [a.cos(), a.sin(), 0.0]
    }

    #[test]
    fn the_chroma_correction_gives_the_spectral_rayleigh_sky() {
        let c = spectral_chroma([0.175, 0.41, 1.0]);
        assert!((c[0] / c[2] - 0.25).abs() < 0.01 && (c[1] / c[2] - 0.47).abs() < 0.01, "{c:?}");
    }

    #[test]
    fn the_coastal_air_sees_about_sixty_km() {
        let a = Atmosphere::coastal_clear();
        let vis = 3.912 / a.ground_extinction_550(1.0);
        assert!((55.0..90.0).contains(&vis), "visibility {vis} km");
    }

    #[test]
    fn the_sun_is_white_above_the_air_and_warm_near_the_horizon() {
        let c = sun_colour_toa();
        assert!((luminance(c) - 1.0).abs() < 1e-3);
        assert!(c[0] > c[1] && c[1] > c[2] && c[0] / c[2] < 1.3, "{c:?}");
        let t = tables(&Atmosphere::coastal_clear(), 1.0);
        let hi = t.sun_transmittance(2.0, (60.0 * DEG).sin());
        let lo = t.sun_transmittance(2.0, (5.0 * DEG).sin());
        assert!(hi[0] / hi[2] < 1.6 && lo[0] / lo[2] > 3.0, "{hi:?} {lo:?}");
        // a high sun's direct beam keeps 70-85 % of its light (Linke turbidity ~2.5-3)
        assert!((0.70..0.88).contains(&luminance(hi)), "{hi:?}");
    }

    #[test]
    fn the_clear_sky_gives_a_fifth_of_the_sun_on_the_ground() {
        // a clear sky's diffuse share of the global horizontal illuminance at a
        // sun 50-60 deg up: 0.10-0.20 (CIE 110-1994; Perez et al. 1990)
        let t = tables(&Atmosphere::coastal_clear(), 1.0);
        let s = sun_at(55.0);
        let l = sky_light(&t, s, URBAN_GROUND_ALBEDO);
        let direct = luminance(l.sun) * s[1];
        let diffuse = luminance(l.sky_h);
        let share = diffuse / (direct + diffuse);
        assert!((0.08..0.22).contains(&share), "diffuse share {share}");
        // the zenith is blue, and brighter than the sky's mean in the anti-sun
        let z = t.sky_radiance([0.0, 1.0, 0.0], s, 2.0);
        assert!(z[2] > 1.8 * z[0], "{z:?}");
        // the clear zenith's luminance against a sunlit 18 % card on the ground: ~0.5-2
        // (zenith 2-8 kcd/m2 at sun 50-60 deg, CIE 110-1994; card ~5 kcd/m2)
        let card = 0.18 * (direct + diffuse) / std::f64::consts::PI;
        let r = luminance(z) / card;
        assert!((0.3..2.5).contains(&r), "zenith / card {r}");
    }

    #[test]
    fn the_sky_irradiance_is_bluer_than_the_sun() {
        let t = tables(&Atmosphere::coastal_clear(), 1.0);
        let l = sky_light(&t, sun_at(50.0), URBAN_GROUND_ALBEDO);
        let up = sh_eval(&l.sh, [0.0, 1.0, 0.0]);
        let down = sh_eval(&l.sh, [0.0, -1.0, 0.0]);
        assert!(up[2] / up[0] > 1.6, "{up:?}");
        assert!(down[0] / down[2] > l.sun[0] / l.sun[2] * 0.7, "{down:?}");
        assert!((luminance(up) - luminance(l.sky_h)).abs() < 0.15 * luminance(l.sky_h), "{up:?} {:?}", l.sky_h);
    }
}
