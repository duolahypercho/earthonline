//! The analytic sky: `computeSkyModel`, `sampleSkyRadiance`,
//! `skyDomeRadiance` and `domeHorizonRadiance` of
//! src/render/environment-ibl.js, clear weather, with the shipped look's
//! sky profile and dome grade (src/render/look-profiles.js, coastal-heat).
//!
//! Ported exactly: the Preetham dome (constants, extinction row, phase
//! functions, the horizon term, the night floor), the twilight and night
//! structure (afterglow, ozone blue hour, belt of Venus, Earth's shadow, deep
//! twilight, urban skyglow, airglow), the energy basis on the 0.25 h bucket
//! with the geometric interpolation between buckets, the probes (zenith,
//! horizon, sunward, anti-sun), the ground radiance at the nadir, the look's
//! dome grade and the horizon haze band.
//!
//! Not ported: the lower hemisphere's street field (`makeGroundField`: kerb
//! bands, grazing specular, night pools). Below the horizon `sample` returns
//! the nadir ground radiance; the visible dome (`dome`) below the horizon is
//! 94 % the horizon haze, so the drawn sky barely sees it. The ground
//! hemisphere's irradiance the light rig needs comes from the oracle table.

use crate::look::{CoastalHeat, LOOK};
use crate::math::*;
use crate::sun::{Sun, sun_direction, wrap_hour};

const TOTAL_RAYLEIGH: Rgb = [5.804542996261093e-6, 1.3562911419845635e-5, 3.0265902468824876e-5];
const MIE_CONST: Rgb = [1.8399918514433978e14, 2.7798023919660528e14, 4.0790479543861094e14];
const RAYLEIGH_ZENITH_LENGTH: f64 = 8.4e3;
const MIE_ZENITH_LENGTH: f64 = 1.25e3;
const CUTOFF_ANGLE: f64 = 1.6110731556870734;
const STEEPNESS: f64 = 1.5;
const SUN_ENERGY: f64 = 1000.0;
const THREE_OVER_SIXTEEN_PI: f64 = 0.05968310365946075;
const ONE_OVER_FOUR_PI: f64 = 0.07957747154594767;
const TWILIGHT_COLOR: Rgb = [0.160, 0.075, 0.055];
const OZONE_BLUE_COLOR: Rgb = [0.0075, 0.0225, 0.0640];
const OZONE_SUNWARD_FLOOR: f64 = 0.55;
const BELT_OF_VENUS_COLOR: Rgb = [0.0780, 0.0345, 0.0300];
const EARTH_SHADOW_REFRACTION_DEG: f64 = 0.55;
const EARTH_SHADOW_MAX_DEG: f64 = 18.0;
const EARTH_SHADOW_SOFT_SIN: f64 = 0.052;
const DEEP_TWILIGHT_COLOR: Rgb = [0.0520, 0.0370, 0.0420];
const AIRGLOW_COLOR: Rgb = [0.00107, 0.00161, 0.00129];
const AIRGLOW_HORIZON_MAX: f64 = 4.0;
const AIRGLOW_DAWN_SHARE: f64 = 0.42;
const GROUND_BLEND_DEG: f64 = 0.9;
const PREETHAM_RADIANCE_SCALE: f64 = 0.04;
const NIGHT_FLOOR: Rgb = [0.0, 0.0003, 0.00075];
const SKY_RADIANCE_SCALE: f64 = 0.2;
/// `BASELINE_LIGHT_RIG.sun`, the key the ground bounce assumes.
pub const SUN_KEY_IRRADIANCE: f64 = 2.75;
const GROUND_SUN_TINT: Rgb = [1.06, 1.0, 0.9];
const URBAN_GROUND_TINT: Rgb = [1.18, 1.0, 0.72];
const URBAN_GROUND_STRENGTH: f64 = 0.22;
const URBAN_GROUND_HORIZON_SHARE: f64 = 0.75;
pub const IRRADIANCE_SAMPLES: usize = 512;
const HORIZON_PROBES: usize = 8;
/// The environment bucket (`hourQuantum` default).
pub const HOUR_QUANTUM: f64 = 0.25;

fn twilight_begin_sin() -> f64 {
    (1.0 * DEG).sin()
}
fn twilight_end_sin() -> f64 {
    (-14.0 * DEG).sin()
}
fn airglow_layer_k2() -> f64 {
    (6371.0f64 / (6371.0 + 87.0)).powi(2)
}

/// Atmosphere inputs (`weatherProfile`, optionally biased by a look's `sky`).
#[derive(Clone, Copy, Debug)]
pub struct Profile {
    pub turbidity: f64,
    pub rayleigh: f64,
    pub mie_coefficient: f64,
    pub mie_directional_g: f64,
    pub overcast: f64,
    pub isotropy: f64,
    pub desaturation: f64,
    pub brightness: f64,
    pub tint: Rgb,
    pub ground_albedo: Rgb,
    pub urban_glow: f64,
}

impl Profile {
    /// `weatherProfile('clear')`, unbiased.
    pub fn clear() -> Self {
        Self {
            turbidity: 2.4,
            rayleigh: 1.6,
            mie_coefficient: 0.005,
            mie_directional_g: 0.8,
            overcast: 0.0,
            isotropy: 0.0,
            desaturation: 0.0,
            brightness: 1.0,
            tint: [1.0, 1.0, 1.0],
            ground_albedo: [0.17, 0.165, 0.15],
            urban_glow: 0.6,
        }
    }
    /// Clear air under the look the game ships.
    pub fn clear_with_look(look: &CoastalHeat) -> Self {
        Self {
            turbidity: look.sky_turbidity,
            rayleigh: look.sky_rayleigh,
            mie_coefficient: look.sky_mie_coefficient,
            mie_directional_g: look.sky_mie_g,
            ..Self::clear()
        }
    }
}

/// Sun-dependent Preetham terms (`preethamState`).
#[derive(Clone, Copy, Debug)]
pub struct Preetham {
    pub sun_e: f64,
    pub beta_r: Rgb,
    pub beta_m: Rgb,
    pub beta_sum: Rgb,
    pub horizon_up: f64,
    pub g: f64,
}

fn sun_intensity_at(zenith_cos: f64) -> f64 {
    let c = clamp(zenith_cos, -1.0, 1.0);
    SUN_ENERGY * (1.0 - (-((CUTOFF_ANGLE - c.acos()) / STEEPNESS)).exp()).max(0.0)
}

impl Preetham {
    pub fn new(sun_y: f64, p: &Profile) -> Self {
        let sun_e = sun_intensity_at(sun_y);
        let sun_fade = 1.0 - clamp(1.0 - sun_y.exp(), 0.0, 1.0);
        let rc = p.rayleigh - (1.0 - sun_fade);
        let beta_r = [TOTAL_RAYLEIGH[0] * rc, TOTAL_RAYLEIGH[1] * rc, TOTAL_RAYLEIGH[2] * rc];
        let c = 0.2 * p.turbidity * 1e-17;
        let mie = [0.434 * c * MIE_CONST[0], 0.434 * c * MIE_CONST[1], 0.434 * c * MIE_CONST[2]];
        let beta_m = [mie[0] * p.mie_coefficient, mie[1] * p.mie_coefficient, mie[2] * p.mie_coefficient];
        Self {
            sun_e,
            beta_r,
            beta_m,
            beta_sum: [beta_r[0] + beta_m[0], beta_r[1] + beta_m[1], beta_r[2] + beta_m[2]],
            horizon_up: clamp((1.0 - sun_y).powi(5), 0.0, 1.0),
            g: p.mie_directional_g,
        }
    }

    /// Extinction for a view elevation (`preethamRow`).
    pub fn row(&self, dy: f64) -> Rgb {
        let up = dy.max(0.0);
        let za = up.acos();
        let inv = 1.0 / (za.cos() + 0.15 * (93.885 - za / DEG).powf(-1.253));
        let sr = RAYLEIGH_ZENITH_LENGTH * inv;
        let sm = MIE_ZENITH_LENGTH * inv;
        [
            (-(self.beta_r[0] * sr + self.beta_m[0] * sm)).exp(),
            (-(self.beta_r[1] * sr + self.beta_m[1] * sm)).exp(),
            (-(self.beta_r[2] * sr + self.beta_m[2] * sm)).exp(),
        ]
    }

    /// `preethamRadianceRow` without the disc (the model is built with
    /// `sunDiscIntensity` 0; the drawn sun is the sky pass's own disc).
    pub fn radiance_row(&self, fex: Rgb, cos_theta: f64) -> Rgb {
        let r_cos = cos_theta * 0.5 + 0.5;
        let r_phase = THREE_OVER_SIXTEEN_PI * (1.0 + r_cos * r_cos);
        let g = self.g;
        let g2 = g * g;
        let q = (1.0 - 2.0 * g * cos_theta + g2).max(1e-6);
        let m_phase = ONE_OVER_FOUR_PI * ((1.0 - g2) / (q * q.sqrt()));
        let mut out = [0.0; 3];
        for i in 0..3 {
            let f = fex[i];
            let beta_theta = self.beta_r[i] * r_phase + self.beta_m[i] * m_phase;
            let ratio = beta_theta / self.beta_sum[i];
            let in_scatter = (self.sun_e * ratio * (1.0 - f)).max(0.0);
            let mut lin = in_scatter * in_scatter.sqrt();
            let horizon_term = (self.sun_e * ratio * f).max(0.0).sqrt();
            lin *= mix(1.0, horizon_term, self.horizon_up);
            let l0 = 0.1 * f;
            out[i] = (lin + l0) * PREETHAM_RADIANCE_SCALE + NIGHT_FLOOR[i];
        }
        out
    }

    pub fn radiance(&self, d: [f64; 3], sun: &Sun) -> Rgb {
        let cos_theta = clamp(d[0] * sun.x + d[1] * sun.y + d[2] * sun.z, -1.0, 1.0);
        self.radiance_row(self.row(d[1]), cos_theta)
    }
}

/// The deterministic Fibonacci lattice over the upper hemisphere (`hemisphereSamples`).
pub fn hemisphere_samples(count: usize) -> Vec<[f64; 3]> {
    let golden = std::f64::consts::PI * (3.0 - 5f64.sqrt());
    (0..count)
        .map(|i| {
            let y = (i as f64 + 0.5) / count as f64;
            let r = (1.0 - y * y).max(0.0).sqrt();
            let phi = i as f64 * golden;
            [phi.cos() * r, y, phi.sin() * r]
        })
        .collect()
}

fn descending_lobe(x: f64, high: f64, peak: f64, low: f64) -> f64 {
    smoothstep(high, peak, x) * (1.0 - smoothstep(peak, low, x))
}

fn elevation_band(s: f64, a: f64, b: f64, c: f64) -> f64 {
    smoothstep(a, b, s) * (1.0 - smoothstep(b, c, s))
}

fn night_arc(hour_angle_deg: f64) -> f64 {
    let ha = ((hour_angle_deg % 360.0) + 540.0) % 360.0 - 180.0;
    clamp(if ha >= 0.0 { (ha - 90.0) / 180.0 } else { (ha + 270.0) / 180.0 }, 0.0, 1.0)
}

/// Daylight ramp, night and the afterglow amplitude (`solarTerms`), exposure 1.
pub fn solar_terms(sun_y: f64, p: &Profile) -> (f64, f64, f64) {
    let daylight = smoothstep(-0.10453, 0.10453, sun_y);
    let night = 1.0 - daylight;
    let twilight = smoothstep(twilight_end_sin(), twilight_begin_sin(), sun_y) * (1.0 - daylight) * (1.0 - p.overcast * 0.7);
    (daylight, night, twilight)
}

/// `makeSkyField`: the upper hemisphere for one sun, profile and basis.
#[derive(Clone, Copy, Debug)]
pub struct Field {
    pub sun: Sun,
    pub state: Preetham,
    pub profile: Profile,
    pub mean: Rgb,
    pub grade: f64,
    pub twilight: f64,
    pub night: f64,
    pub ozone: f64,
    pub belt: f64,
    pub shadow_top_sin: f64,
    pub deep_twilight: f64,
    pub airglow: f64,
}

/// Elevation-only terms of one direction (`rowState`).
#[derive(Clone, Copy, Debug)]
pub struct Row {
    pub fex: Rgb,
    pub cie_shape: f64,
    pub horizon_weight: f64,
    pub glow: Rgb,
    pub air: Rgb,
    pub lit: f64,
    pub ozone_row: f64,
    pub belt_row: f64,
    pub deep_row: f64,
}

impl Field {
    pub fn new(sun: Sun, state: Preetham, profile: Profile, mean: Rgb, grade: f64, twilight: f64, night: f64) -> Self {
        let overcast = clamp(profile.overcast, 0.0, 1.0);
        let alt = sun.altitude_deg;
        let ozone = descending_lobe(alt, 1.5, -4.5, -12.5) * (1.0 - overcast * 0.85);
        let belt = descending_lobe(alt, 2.5, -2.0, -7.5) * (1.0 - overcast * 0.9);
        let shadow_top_sin = (clamp((-alt).max(0.0) + EARTH_SHADOW_REFRACTION_DEG, 0.0, EARTH_SHADOW_MAX_DEG) * DEG).sin();
        let deep_twilight = descending_lobe(alt, -5.0, -12.0, -19.0) * (1.0 - overcast * 0.85);
        let airglow = night * mix(1.0, AIRGLOW_DAWN_SHARE, night_arc(sun.hour_angle_deg)) * (1.0 - overcast * 0.6);
        Self { sun, state, profile, mean, grade, twilight, night, ozone, belt, shadow_top_sin, deep_twilight, airglow }
    }

    fn twilight_structure(&self) -> bool {
        self.ozone > 0.0 || self.belt > 0.0 || self.deep_twilight > 0.0
    }

    pub fn row(&self, dy: f64) -> Row {
        let s = clamp(dy.abs(), 0.0, 1.0);
        let horizon_weight = (1.0 - s).powi(3);
        let mut r = Row {
            fex: self.state.row(dy),
            cie_shape: (1.0 + 2.0 * dy.max(0.0)) / 3.0,
            horizon_weight,
            glow: [0.0; 3],
            air: [0.0; 3],
            lit: 1.0,
            ozone_row: 0.0,
            belt_row: 0.0,
            deep_row: 0.0,
        };
        let ts = self.twilight_structure();
        if self.night > 0.0 || ts {
            if self.night > 0.0 {
                let glow = self.night * self.profile.urban_glow;
                let hw = horizon_weight * horizon_weight * horizon_weight;
                r.glow = [glow * (0.0090 + 0.0520 * hw), glow * (0.0104 + 0.0328 * hw), glow * (0.0176 + 0.0176 * hw)];
                let air = self.airglow * AIRGLOW_HORIZON_MAX.min(1.0 / (1.0 - airglow_layer_k2() * (1.0 - s * s)).max(1e-6).sqrt());
                r.air = scale3(AIRGLOW_COLOR, air);
            }
            if ts {
                r.lit = smoothstep(self.shadow_top_sin - EARTH_SHADOW_SOFT_SIN, self.shadow_top_sin + EARTH_SHADOW_SOFT_SIN, s);
                r.ozone_row = self.ozone * (0.55 + 0.45 * (1.0 - s) * (1.0 - s));
                r.belt_row = self.belt * elevation_band(s, 0.0, (8.0 * DEG).sin(), (26.0 * DEG).sin());
                r.deep_row = self.deep_twilight * mix(0.22, 1.0, horizon_weight);
            }
        }
        r
    }

    /// `upperFromRow`.
    pub fn upper_from_row(&self, row: &Row, cos_theta: f64) -> Rgb {
        let mut out = self.state.radiance_row(row.fex, cos_theta);
        let p = &self.profile;
        let oz = [self.mean[0] * 1.5, self.mean[1] * 1.5, self.mean[2] * 1.5];
        for i in 0..3 {
            let v = mix(out[i], oz[i] * row.cie_shape, p.overcast);
            out[i] = mix(v, self.mean[i], p.isotropy);
        }
        let des = clamp(p.desaturation, 0.0, 1.0);
        if des > 0.0 {
            let l = luminance(out);
            out = [mix(out[0], l, des), mix(out[1], l, des), mix(out[2], l, des)];
        }
        for i in 0..3 {
            out[i] *= self.grade * p.tint[i];
        }
        if self.twilight > 0.0 && cos_theta > 0.0 {
            let band = self.twilight * row.horizon_weight * cos_theta * cos_theta;
            for i in 0..3 {
                out[i] += band * TWILIGHT_COLOR[i];
            }
        }
        let ts = self.twilight_structure();
        if self.night > 0.0 || ts {
            if self.night > 0.0 {
                for i in 0..3 {
                    out[i] += row.glow[i] + row.air[i];
                }
            }
            if ts {
                let sunward = cos_theta.max(0.0);
                let antisun = (-cos_theta).max(0.0);
                let antisun2 = antisun * antisun;
                if row.ozone_row > 0.0 {
                    let a = row.ozone_row * mix(1.0, row.lit, antisun2) * (OZONE_SUNWARD_FLOOR + (1.0 - OZONE_SUNWARD_FLOOR) * sunward);
                    for i in 0..3 {
                        out[i] += a * OZONE_BLUE_COLOR[i];
                    }
                }
                if row.belt_row > 0.0 {
                    let a = row.belt_row * row.lit * antisun * antisun;
                    for i in 0..3 {
                        out[i] += a * BELT_OF_VENUS_COLOR[i];
                    }
                }
                if row.deep_row > 0.0 {
                    let a = row.deep_row * sunward * sunward;
                    for i in 0..3 {
                        out[i] += a * DEEP_TWILIGHT_COLOR[i];
                    }
                }
            }
        }
        out
    }

    pub fn cos_to_sun(&self, d: [f64; 3]) -> f64 {
        clamp(d[0] * self.sun.x + d[1] * self.sun.y + d[2] * self.sun.z, -1.0, 1.0)
    }

    pub fn sample_upper(&self, d: [f64; 3]) -> Rgb {
        self.upper_from_row(&self.row(d[1]), self.cos_to_sun(d))
    }
}

/// The expensive half at one exact hour (`measureSkyBasis` minus the ground
/// diagnostics, which need the street field).
#[derive(Clone, Copy, Debug)]
pub struct Basis {
    pub hour: f64,
    pub mean: Rgb,
    pub energy_scale: f64,
    pub sky_irradiance: Rgb,
}

fn clear_reference_irradiance_luminance(sun: &Sun, dirs: &[[f64; 3]]) -> f64 {
    let st = Preetham::new(sun.y, &Profile::clear());
    let mut sum = 0.0;
    for d in dirs {
        sum += luminance(st.radiance(*d, sun)) * d[1];
    }
    (sum / dirs.len() as f64).max(1e-9)
}

pub fn measure_basis(hour: f64, p: &Profile, dirs: &[[f64; 3]]) -> Basis {
    let sun = sun_direction(hour);
    let (_, night, twilight) = solar_terms(sun.y, p);
    let st = Preetham::new(sun.y, p);
    let raw: Vec<Rgb> = dirs.iter().map(|d| st.radiance(*d, &sun)).collect();
    let n = dirs.len() as f64;
    let mut mean = [0.0; 3];
    for c in &raw {
        for i in 0..3 {
            mean[i] += c[i];
        }
    }
    mean = scale3(mean, 1.0 / n);
    let oz = scale3(mean, 1.5);
    let des = clamp(p.desaturation, 0.0, 1.0);
    let mut shaped_lum = 0.0;
    for (k, d) in dirs.iter().enumerate() {
        let cie = (1.0 + 2.0 * d[1].max(0.0)) / 3.0;
        let mut s = [0.0; 3];
        for c in 0..3 {
            let v = mix(raw[k][c], oz[c] * cie, p.overcast);
            s[c] = mix(v, mean[c], p.isotropy);
        }
        if des > 0.0 {
            let l = luminance(s);
            s = [mix(s[0], l, des), mix(s[1], l, des), mix(s[2], l, des)];
        }
        shaped_lum += luminance([s[0] * p.tint[0], s[1] * p.tint[1], s[2] * p.tint[2]]) * d[1];
    }
    let shaped_lum = (shaped_lum / n).max(1e-12);
    let energy_scale = clear_reference_irradiance_luminance(&sun, dirs) / shaped_lum;
    let grade = p.brightness * SKY_RADIANCE_SCALE * energy_scale;
    let field = Field::new(sun, st, *p, mean, grade, twilight, night);
    let mut irr = [0.0; 3];
    for d in dirs {
        let c = field.sample_upper(*d);
        for i in 0..3 {
            irr[i] += c[i] * d[1];
        }
    }
    Basis { hour, mean, energy_scale, sky_irradiance: scale3(irr, TAU / n) }
}

/// `resolveSkyBasis`: the bracketing buckets, interpolated.
pub fn resolve_basis(hour: f64, p: &Profile, dirs: &[[f64; 3]]) -> Basis {
    let wrapped = wrap_hour(hour);
    let index = (wrapped / HOUR_QUANTUM + 1e-9).floor();
    let lower_hour = wrap_hour(index * HOUR_QUANTUM);
    let t = clamp((wrapped - index * HOUR_QUANTUM) / HOUR_QUANTUM, 0.0, 1.0);
    let lower = measure_basis(lower_hour, p, dirs);
    if !(t > 0.0) {
        return lower;
    }
    let upper_hour = wrap_hour((index + 1.0) * HOUR_QUANTUM);
    if upper_hour == lower_hour {
        return lower;
    }
    let upper = measure_basis(upper_hour, p, dirs);
    let g3 = |a: Rgb, b: Rgb| [geo_mix(a[0], b[0], t), geo_mix(a[1], b[1], t), geo_mix(a[2], b[2], t)];
    Basis {
        hour: mix(lower.hour, upper.hour, t),
        mean: g3(lower.mean, upper.mean),
        energy_scale: geo_mix(lower.energy_scale, upper.energy_scale, t),
        sky_irradiance: g3(lower.sky_irradiance, upper.sky_irradiance),
    }
}

/// the elevation (sine) the directional horizon band samples: 2 deg.
pub const HAZE_BAND_SIN: f64 = 0.0349;

/// The look's dome grade (`activeDomeGrade`) for one sun altitude.
#[derive(Clone, Copy, Debug)]
pub struct DomeGrade {
    pub weight: f64,
    pub veil_luminance: f64,
    pub zenith_desaturation: f64,
    pub horizon_desaturation: f64,
    pub horizon_lift: f64,
    pub dome_gain: f64,
    pub skyline_share: f64,
    pub low_sun_share: f64,
    pub sunward_horizon_desaturation: f64,
}

/// what the sky slice adds to the visible dome (all off: the JS dome).
/// `on`: the horizon band takes the sky just over the horizon in the view's own
/// azimuth (2 deg up) instead of the one horizon colour, the grade's veil
/// stays to the horizon (it faded to nothing there, so the horizon was darker
/// than the sky above it: the dark band at golden hour), and the Preetham
/// aureole is narrowed: within 30 deg of the sun the sky is x (angle / 30
/// deg)^2, at least 0.06 (the model's Mie lobe left the sky 10 deg from a low
/// sun at ~120 x a sunlit card, where a measured aureole falls to ~10-20 x
/// within 10-15 deg). `afterglow`: the twilight arch toward the set sun, in
/// radiance at the horizon (0 by day and at night).
#[derive(Clone, Copy, Debug, Default)]
pub struct SkyExt {
    pub on: bool,
    pub afterglow: f64,
}

/// the afterglow arch's colour at the horizon and at its top, luminance 1.
pub const AFTERGLOW_LOW: Rgb = [2.05, 0.92, 0.36];
pub const AFTERGLOW_HIGH: Rgb = [1.20, 1.02, 0.66];

/// the aureole factor at an angle from the sun (radians), for the sun's
/// height (sine): only while the sun is up (fading in from -1 to +2 deg; under
/// the horizon there is no aureole to narrow, and the factor drew a dark disc
/// round the set sun's place).
pub fn aureole_factor(angle: f64, sun_y: f64) -> f64 {
    let a = angle / (30.0 * DEG);
    let f = if a >= 1.0 { 1.0 } else { (a * a).max(0.06) };
    let k = smoothstep(-0.0175, 0.035, sun_y);
    1.0 + (f - 1.0) * k
}

/// the afterglow arch at a direction (unit, y up) for a sun direction.
pub fn afterglow_arch(d: [f64; 3], sun: [f64; 3], strength: f64) -> Rgb {
    if strength <= 0.0 || d[1] < -0.02 {
        return [0.0; 3];
    }
    let (dh, sh) = ((d[0] * d[0] + d[2] * d[2]).sqrt(), (sun[0] * sun[0] + sun[2] * sun[2]).sqrt());
    if dh < 1e-6 || sh < 1e-6 {
        return [0.0; 3];
    }
    let c = ((d[0] * sun[0] + d[2] * sun[2]) / (dh * sh)).max(0.0);
    let el = d[1].max(0.0);
    let w = c * c * c * (-el / 0.13).exp();
    let t = smoothstep(0.0, 0.16, el);
    [0, 1, 2].map(|i| strength * w * (AFTERGLOW_LOW[i] + (AFTERGLOW_HIGH[i] - AFTERGLOW_LOW[i]) * t))
}

/// One hour's sky (`computeSkyModel`, clear, exposure 1, no sun disc).
#[derive(Clone, Copy, Debug)]
pub struct SkyModel {
    pub hour: f64,
    pub sun: Sun,
    pub profile: Profile,
    pub daylight: f64,
    pub night: f64,
    pub field: Field,
    pub basis: Basis,
    pub zenith_radiance: Rgb,
    pub horizon_radiance: Rgb,
    pub sunward_horizon_radiance: Rgb,
    pub antisun_horizon_radiance: Rgb,
    pub ground_radiance: Rgb,
    pub grade: Option<DomeGrade>,
    /// the sky slice's additions to the visible dome (default: none).
    pub ext: SkyExt,
}

impl SkyModel {
    /// The sky the game draws at `hour` (clear, coastal-heat).
    pub fn new(hour: f64) -> Self {
        Self::with_profile(hour, Profile::clear_with_look(&LOOK), true)
    }

    pub fn with_profile(hour: f64, profile: Profile, look_grade: bool) -> Self {
        let dirs = hemisphere_samples(IRRADIANCE_SAMPLES);
        let requested = wrap_hour(hour);
        let basis = resolve_basis(requested, &profile, &dirs);
        let sun = sun_direction(requested);
        let (daylight, night, twilight) = solar_terms(sun.y, &profile);
        let st = Preetham::new(sun.y, &profile);
        let grade = profile.brightness * SKY_RADIANCE_SCALE * basis.energy_scale;
        let field = Field::new(sun, st, profile, basis.mean, grade, twilight, night);
        // Probes (`measureSkyProbes`).
        let zenith = field.sample_upper([0.0, 1.0, 0.0]);
        let hy = (2.0 * DEG).sin();
        let hr = (2.0 * DEG).cos();
        let probe = |az: f64| field.sample_upper([az.sin() * hr, hy, -az.cos() * hr]);
        let sun_az = sun.azimuth_deg * DEG;
        let sunward = probe(sun_az);
        let antisun = probe(sun_az + std::f64::consts::PI);
        let mut acc = [0.0; 3];
        for i in 0..HORIZON_PROBES {
            let c = probe(i as f64 / HORIZON_PROBES as f64 * TAU);
            for k in 0..3 {
                acc[k] += c[k];
            }
        }
        let horizon = scale3(acc, 1.0 / HORIZON_PROBES as f64);
        // Ground at the nadir (`makeLowerHemisphere`'s groundRadiance).
        let albedo = profile.ground_albedo;
        let sky_irr = basis.sky_irradiance;
        let direct = sun.y.max(0.0) * SUN_KEY_IRRADIANCE / std::f64::consts::PI * profile.brightness * (1.0 - profile.overcast * 0.8);
        let urban_raw = smoothstep(10.0, -2.0, sun.altitude_deg) * profile.urban_glow * URBAN_GROUND_STRENGTH;
        let ground_at = |urban: f64| -> Rgb {
            let mut g = [0.0; 3];
            for i in 0..3 {
                g[i] = albedo[i] * (sky_irr[i] / std::f64::consts::PI + direct * GROUND_SUN_TINT[i] + urban * URBAN_GROUND_TINT[i]);
            }
            g
        };
        let mut urban = urban_raw;
        if urban_raw > 0.0 {
            let candidate = luminance(ground_at(urban_raw));
            let ceiling = URBAN_GROUND_HORIZON_SHARE * luminance(horizon);
            if candidate > ceiling {
                let base = luminance(ground_at(0.0));
                let per = (candidate - base) / urban_raw;
                urban = if per > 1e-12 { ((ceiling - base) / per).max(0.0) } else { 0.0 };
            }
        }
        let ground = ground_at(urban);
        let mut m = Self {
            hour: requested,
            sun,
            profile,
            daylight,
            night,
            field,
            basis,
            zenith_radiance: zenith,
            horizon_radiance: horizon,
            sunward_horizon_radiance: sunward,
            antisun_horizon_radiance: antisun,
            ground_radiance: ground,
            grade: None,
            ext: SkyExt::default(),
        };
        if look_grade {
            m.grade = m.active_dome_grade(&LOOK);
        }
        m
    }

    pub fn sky_irradiance(&self) -> Rgb {
        self.basis.sky_irradiance
    }

    /// `sampleSkyRadiance`: the dome above the horizon, the JS's street field
    /// below it (`ground_field`), crossfaded over `GROUND_BLEND_DEG`.
    pub fn sample(&self, d: [f64; 3]) -> Rgb {
        let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let d = [d[0] / l, d[1] / l, d[2] / l];
        let sky = self.field.sample_upper(d);
        if d[1] >= 0.0 {
            return sky;
        }
        let t = smoothstep(0.0, (GROUND_BLEND_DEG * DEG).sin(), -d[1]);
        let ground = self.ground_field(d);
        if t >= 1.0 {
            return ground;
        }
        mix3(sky, ground, t)
    }

    /// the lower hemisphere as the JS builds it (`makeGroundField`
    /// evaluated by `sampleSkyRadiance`, environment-ibl.js): bands from the
    /// street cross-section seen from the probe's eye (pavement, carriageway,
    /// far footway / streetwall), a four-lobe openness on the measured grid
    /// bearing, the grazing sheen that mirrors the sky at the MIRROR direction,
    /// and the night practicals. Clear and dry (wetness 0), no snow, the
    /// model's exposure 1. Constants: data/ground-field.json.
    pub fn ground_field(&self, d: [f64; 3]) -> Rgb {
        let g = crate::ground_field::GroundField::get();
        let s = clamp(-d[1], 0.0, 1.0);
        let pavement = smoothstep(g.s_kerb * 0.8, g.s_kerb * 1.2, s);
        let far = 1.0 - smoothstep(g.s_far_walk * 0.75, g.s_far_kerb * 1.35, s);
        let carriage = (1.0 - pavement - far).max(0.0);
        let grazing = 1.0 - s;
        let clearing = 1.0 - g.clutter * (1.0 - smoothstep(0.0, g.clutter_sin, s));
        let spec = g.spec_max * grazing.powi(5) * clearing;
        let alt = self.sun.altitude_deg;
        let lamps_on = 1.0 - smoothstep(-3.0, 8.0, alt);
        let dusk = 1.0 - smoothstep(2.0, 16.0, alt);
        let pool_level = g.lamp_pool * lamps_on;
        let spill_level = g.shop_spill * dusk;
        let band = |s: f64, a: f64, b: f64, c: f64| smoothstep(a, b, s) * (1.0 - smoothstep(b, c, s));
        let pool = if pool_level > 0.0 { band(s, 0.030, 0.132, 0.40) } else { 0.0 };
        let spill = if spill_level > 0.0 { band(s, 0.16, 0.38, 0.80) } else { 0.0 };
        let r = (d[0] * d[0] + d[2] * d[2]).sqrt();
        let axis_dot = if r > 1e-9 { clamp((d[0] * g.axis[0] + d[2] * g.axis[1]) / r, -1.0, 1.0) } else { 0.0 };
        let two = 2.0 * axis_dot * axis_dot - 1.0;
        let four = 2.0 * two * two - 1.0;
        let open = clamp(0.5 + 0.5 * mix(two, four, g.cross_share), 0.0, 1.0);
        let far_open = far * open;
        let far_wall = far - far_open;
        let road = (carriage + far_open) * g.road_ratio;
        let wall = far_wall * g.wall_ratio;
        let sheen = spec * (g.walled_share + (1.0 - g.walled_share) * open);
        let mirror = self.field.sample_upper([d[0], -d[1], d[2]]);
        let pool_amp = if pool > 0.0 { pool_level * pool * (0.45 + 0.55 * open) } else { 0.0 };
        let spill_amp = if spill > 0.0 { spill_level * spill * (1.0 - open).powf(1.5) } else { 0.0 };
        let mut out = [0.0; 3];
        for i in 0..3 {
            let diffuse = self.ground_radiance[i] * (pavement + road * g.road_tint[i] + wall * g.wall_tint[i]);
            let reflected = mix(self.horizon_radiance[i], mirror[i], g.sharpness);
            out[i] = diffuse * (1.0 - sheen) + reflected * sheen + pool_amp * g.pool_color[i] + spill_amp * g.spill_color[i];
        }
        out
    }

    fn active_dome_grade(&self, look: &CoastalHeat) -> Option<DomeGrade> {
        let alt = self.sun.altitude_deg;
        let weight = smoothstep(look.dome_altitude_band[0], look.dome_altitude_band[1], alt);
        if !(weight > 0.0) {
            return None;
        }
        let high01 = smoothstep(look.dome_low_sun_band[0], look.dome_low_sun_band[1], alt);
        let skyline_share = (1.0 - smoothstep(look.skyline_band[0], look.skyline_band[1], alt)) * weight;
        let veil_share = 1.0 - smoothstep(look.veil_band[0], look.veil_band[1], alt);
        Some(DomeGrade {
            weight,
            veil_luminance: if veil_share > 0.0 { look.veil_amount * veil_share * weight * luminance(self.horizon_radiance) } else { 0.0 },
            zenith_desaturation: mix(look.low_sun_zenith_desaturation, look.zenith_desaturation, high01),
            horizon_desaturation: mix(look.low_sun_horizon_desaturation, look.horizon_desaturation, high01),
            horizon_lift: mix(look.horizon_lift, look.skyline_lift, skyline_share),
            dome_gain: mix(1.0, look.skyline_dome_gain, skyline_share),
            skyline_share,
            low_sun_share: 1.0 - high01,
            sunward_horizon_desaturation: look.low_sun_sunward_horizon_desaturation,
        })
    }

    fn grade_dome(&self, rgb: Rgb, g: &DomeGrade, to_horizon: f64, horizon_desaturation: f64) -> Rgb {
        self.grade_dome_veil(rgb, g, to_horizon, horizon_desaturation, to_horizon)
    }

    /// `grade_dome` with the veil's own horizon weight.
    fn grade_dome_veil(&self, rgb: Rgb, g: &DomeGrade, to_horizon: f64, horizon_desaturation: f64, veil_to_horizon: f64) -> Rgb {
        let look = &LOOK;
        let des = mix(g.zenith_desaturation, horizon_desaturation, to_horizon) * g.weight;
        let lift = 1.0 + g.horizon_lift * to_horizon * g.weight;
        let y = luminance(rgb);
        let tint = look.haze_tint;
        let tint_y = luminance(tint);
        let tint_y = if tint_y == 0.0 { 1.0 } else { tint_y };
        let veil = g.veil_luminance * (1.0 - veil_to_horizon);
        let vt = look.veil_tint;
        let vt_y = luminance(vt);
        let vt_y = if vt_y == 0.0 { 1.0 } else { vt_y };
        let mut out = [0.0; 3];
        for c in 0..3 {
            let grey = y * mix(1.0, tint[c] / tint_y, to_horizon);
            out[c] = (mix(rgb[c], grey, des) * lift + veil * (vt[c] / vt_y)) * g.dome_gain;
        }
        out
    }

    fn horizon_desaturation_toward(&self, g: &DomeGrade, dx: f64, dz: f64) -> f64 {
        if !(g.low_sun_share > 0.0) {
            return g.horizon_desaturation;
        }
        let sun_h = self.sun.x.hypot(self.sun.z);
        let view_h = dx.hypot(dz);
        if !(sun_h > 1e-6) || !(view_h > 1e-6) {
            return g.horizon_desaturation;
        }
        let cos_gap = ((dx * self.sun.x + dz * self.sun.z) / (sun_h * view_h)).max(0.0);
        mix(g.horizon_desaturation, g.sunward_horizon_desaturation, cos_gap * cos_gap * g.low_sun_share)
    }

    /// `domeHorizonRadiance(model, rgb, sunwardShare)`.
    pub fn dome_horizon(&self, rgb: Rgb, sunward_share: f64) -> Rgb {
        match &self.grade {
            None => rgb,
            Some(g) => {
                let hd = mix(g.horizon_desaturation, g.sunward_horizon_desaturation, clamp(sunward_share, 0.0, 1.0) * g.low_sun_share);
                self.grade_dome(rgb, g, 1.0, hd)
            }
        }
    }

    /// `skyDomeRadiance(model, x, y, z)`: what the sky pass paints.
    pub fn dome(&self, d: [f64; 3]) -> Rgb {
        let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let dy = d[1] / l;
        let mut base = self.sample(d);
        let mut haze = self.horizon_radiance;
        let x = self.ext;
        let sun = [self.sun.x, self.sun.y, self.sun.z];
        let angle = |v: [f64; 3]| {
            let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            clamp((v[0] * sun[0] + v[1] * sun[1] + v[2] * sun[2]) / n, -1.0, 1.0).acos()
        };
        // the band toward this azimuth's own low sky, the narrowed aureole
        // and the afterglow arch (see `SkyExt`).
        if x.on {
            base = base.map(|v| v * aureole_factor(angle(d), sun[1]));
            let h = (d[0] * d[0] + d[2] * d[2]).sqrt();
            if h > 1e-9 {
                let b = [d[0] / h, HAZE_BAND_SIN, d[2] / h];
                haze = self.sample(b).map(|v| v * aureole_factor(angle(b), sun[1]));
            }
        }
        if let Some(g) = &self.grade {
            let to_horizon = (1.0 - clamp(dy, 0.0, 1.0)).powf(LOOK.dome_gradient_power);
            let hd = self.horizon_desaturation_toward(g, d[0], d[2]);
            let veil_th = if x.on { 0.0 } else { to_horizon };
            base = self.grade_dome_veil(base, g, to_horizon, hd, veil_th);
            haze = self.grade_dome_veil(haze, g, 1.0, hd, if x.on { 0.0 } else { 1.0 });
        }
        if x.afterglow > 0.0 {
            let n = [d[0] / l, dy, d[2] / l];
            let a = afterglow_arch(n, sun, x.afterglow);
            let hb = afterglow_arch([n[0], HAZE_BAND_SIN, n[2]], sun, x.afterglow);
            base = [0, 1, 2].map(|i| base[i] + a[i]);
            haze = [0, 1, 2].map(|i| haze[i] + hb[i]);
        }
        let mut band_width = mix(0.06, 0.22, clamp((self.profile.turbidity - 2.0) / 8.0, 0.0, 1.0));
        let mut band_peak = 0.42;
        let skyline_share = self.grade.map(|g| g.skyline_share).unwrap_or(0.0) * smoothstep(0.9, 1.1, luminance(haze) / luminance(base).max(1e-6));
        if skyline_share > 0.0 {
            band_width = mix(band_width, band_width.max(LOOK.skyline_band_width), skyline_share);
            band_peak = mix(band_peak, band_peak.max(LOOK.skyline_band_peak), skyline_share);
        }
        let band = (1.0 - clamp(dy.abs() / band_width, 0.0, 1.0)).powi(2) * band_peak;
        let below = smoothstep(0.035, -0.02, dy);
        let t = clamp(band + below * 0.94, 0.0, 1.0);
        mix3(base, haze, t)
    }
}
