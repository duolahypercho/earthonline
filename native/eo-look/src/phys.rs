//! Daylight from the physical atmosphere (`atmosphere`), in the look's units:
//! the sun through the air, the sky's and the ground's light as an SH9
//! irradiance, the scale the GPU's sky tables are drawn at, and the exposure
//! in EV100 an incident-light meter gives for that light. `apply` puts it
//! into a `LookFrame` in place of the browser game's solved rig (the warm
//! key, the hemisphere, the ambient and the rim light) while the sun is up.
//!
//! The look's units: the irradiance a white sun at the top of the atmosphere
//! delivers on a normal is `e_toa_units()`, chosen so that the physical key's
//! luminance at the rig's exposure-reference hour (12 h) equals the rig's own
//! key there; everything else the renderer draws (lamps, windows, the water)
//! keeps its scale by day. One unit of irradiance is then
//! `lux_per_unit()` lux, from the luminous solar constant.

use crate::LookFrame;
use crate::atmosphere::{self, Atmosphere, Tables, URBAN_GROUND_ALBEDO};
use crate::math::*;
use std::sync::{Arc, OnceLock};

/// How much of the frame the physical daylight takes at a solar altitude:
/// all of it from 3 deg up, none from 2 deg under the horizon (there the
/// night solve, the lamps and the analytic twilight dome take over).
pub fn weight(sun_alt_deg: f64) -> f64 {
    smoothstep(-2.0, 3.0, sun_alt_deg)
}

/// The weight under a weather: a deck's light is not the clear sky's, so the
/// physical daylight fades out as the weather's overcast rises past 0.1 and
/// is gone at 0.6 (the drizzle, rain, storm and fog decks keep the rig).
pub fn weather_weight(sun_alt_deg: f64, overcast: f64) -> f64 {
    weight(sun_alt_deg) * (1.0 - smoothstep(0.1, 0.6, overcast))
}

/// Irradiance (look units) of a white sun at the top of the atmosphere, on a normal.
pub fn e_toa_units() -> f64 {
    static E: OnceLock<f64> = OnceLock::new();
    *E.get_or_init(|| {
        let h = crate::rig::EXPOSURE_REFERENCE_HOUR;
        let rig_key = luminance(LookFrame::new(h).key);
        let s = crate::sun::sun_direction(h);
        let t = atmosphere::tables(&Atmosphere::coastal_clear(), 1.0);
        let c = atmosphere::sun_colour_toa();
        let tr = atmosphere::spectral_chroma(t.sun_transmittance(2.0, s.y));
        rig_key / luminance([c[0] * tr[0], c[1] * tr[1], c[2] * tr[2]]).max(1e-9)
    })
}

/// Lux per unit of the look's irradiance by day.
pub fn lux_per_unit() -> f64 {
    atmosphere::LUMINOUS_SOLAR_CONSTANT_LX / e_toa_units()
}

/// The incident-light meter's calibration constant for a flat receptor
/// (ISO 2720:1974, C = 250 lx s), and the reflected-light meter's (K = 12.5 cd s / m2).
pub const METER_C: f64 = 250.0;
pub const METER_K: f64 = 12.5;

/// EV100 an incident-light meter reads for an illuminance (lx): log2(E S / C), S = 100.
pub fn ev100_incident(lux: f64) -> f64 {
    (lux.max(1e-6) * 100.0 / METER_C).log2()
}

/// The middle grey a meter set to EV100 renders (cd/m2): the luminance that
/// reads EV100 on the reflected-light scale, K 2^EV100 / S.
pub fn mid_grey_luminance(ev100: f64) -> f64 {
    METER_K * 2f64.powf(ev100) / 100.0
}

/// The look's exposure (the post pass's `exposure`: the tone curve's input
/// is radiance x exposure / 0.6) that renders the metered middle grey at
/// scene value 0.18, at EV100 `ev100` with `compensation_ev` (positive brighter).
pub fn exposure_at_ev100(ev100: f64, compensation_ev: f64) -> f64 {
    0.6 * 0.18 * lux_per_unit() / mid_grey_luminance(ev100) * 2f64.powf(compensation_ev)
}

/// Daylight for one sun direction, haze density and weather weight.
#[derive(Clone)]
pub struct Daylight {
    pub weight: f64,
    pub density: f64,
    pub sun: [f64; 3],
    /// a white sun at the top of the atmosphere x the Sun's colour (look
    /// units): what the GPU's sky tables (per unit solar illuminance) are drawn x.
    pub sun_toa: Rgb,
    /// the direct beam's irradiance on a normal (look units).
    pub key: Rgb,
    /// the sky's and the ground's irradiance (look units), cosine-convolved SH9.
    pub env_sh: [Rgb; 9],
    /// the illuminance on a horizontal surface, sun and sky (look units, luminance).
    pub e_h: f64,
    tables: Arc<Tables>,
}

impl std::fmt::Debug for Daylight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Daylight").field("weight", &self.weight).field("key", &self.key).field("e_h", &self.e_h).finish()
    }
}

impl Daylight {
    /// `sun`: toward the sun; `density`: the aerosol over the clear medium's (the
    /// weather's haze); `weight`: `weather_weight`.
    pub fn new(sun: [f64; 3], density: f64, weight: f64) -> Self {
        let tables = atmosphere::tables(&Atmosphere::coastal_clear(), density);
        let l = atmosphere::sky_light(&tables, sun, URBAN_GROUND_ALBEDO);
        let e = e_toa_units();
        let c = atmosphere::sun_colour_toa();
        let sun_toa = c.map(|v| v * e);
        let key = [0, 1, 2].map(|i| sun_toa[i] * l.sun[i]);
        let env_sh = l.sh.map(|k| [k[0] * sun_toa[0], k[1] * sun_toa[1], k[2] * sun_toa[2]]);
        let e_h = luminance(key) * sun[1].max(0.0) + luminance([0, 1, 2].map(|i| l.sky_h[i] * sun_toa[i]));
        Self { weight, density, sun, sun_toa, key, env_sh, e_h, tables }
    }

    /// The sky's radiance (look units) along a direction from an eye `eye_m` up.
    pub fn sky(&self, d: [f64; 3], eye_m: f64) -> Rgb {
        let l = self.tables.sky_radiance(d, self.sun, eye_m);
        atmosphere::spectral_chroma([0, 1, 2].map(|i| l[i] * self.sun_toa[i]))
    }

    /// The horizontal illuminance in lux.
    pub fn lux(&self) -> f64 {
        self.e_h * lux_per_unit()
    }

    pub fn ev100(&self) -> f64 {
        ev100_incident(self.lux())
    }
}

/// Put the daylight into a frame by its weight: the key becomes the sun
/// through the air, the diffuse environment the sky's and the ground's
/// irradiance; the rig's hemisphere, ambient and rim (stand-ins for them)
/// fade out. With weight 0 the frame is unchanged. The renderer reads the
/// weight to use the environment at its own intensity (look.rs `hemi_sky.w`).
pub fn apply(f: &mut LookFrame, d: &Daylight) {
    let w = d.weight.clamp(0.0, 1.0);
    if w <= 0.0 {
        return;
    }
    f.key = mix3(f.key, d.key, w);
    f.rim = scale3(f.rim, 1.0 - w);
    f.hemi_sky = scale3(f.hemi_sky, 1.0 - w);
    f.hemi_ground = scale3(f.hemi_ground, 1.0 - w);
    f.ambient = scale3(f.ambient, 1.0 - w);
    for k in 0..9 {
        f.env_sh[k] = mix3(f.env_sh[k], d.env_sh[k], w);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noon_reads_sunny_sixteen() {
        // "sunny 16": a clear, sunlit scene reads EV100 ~15 (ISO 2720 / Kodak's
        // published exposure tables: bright sun on light sand or snow 16, ordinary
        // sunlit subjects 15)
        let s = crate::sun::sun_direction(13.0);
        let d = Daylight::new(s.dir(), 1.0, 1.0);
        let ev = d.ev100();
        assert!((14.5..15.8).contains(&ev), "EV100 {ev} ({} lx)", d.lux());
        // the key keeps the rig's scale at the reference hour
        let r = LookFrame::new(13.0);
        let k = luminance(d.key) / luminance(r.key);
        assert!((0.85..1.15).contains(&k), "key / rig key {k}");
    }

    #[test]
    fn weight_zero_leaves_the_frame_alone() {
        let mut f = LookFrame::new(13.0);
        let g = f;
        let s = crate::sun::sun_direction(13.0);
        apply(&mut f, &Daylight::new(s.dir(), 1.0, 0.0));
        assert_eq!(f.key, g.key);
        assert_eq!(f.env_sh, g.env_sh);
        assert_eq!(f.hemi_sky, g.hemi_sky);
    }

    #[test]
    fn a_shadowed_wall_is_lit_bluer_than_a_sunlit_one() {
        let s = crate::sun::sun_direction(13.0);
        let d = Daylight::new(s.dir(), 1.0, 1.0);
        // a north wall in shadow sees the sky and the ground only
        let n = [0.0, 0.0, -1.0];
        let e = atmosphere::sh_eval(&d.env_sh, n);
        assert!(e[2] > e[0], "{e:?}");
        // sun-to-shadow ratio on a south wall at 13 h: 3-8 (direct ~55-70 klx on the wall)
        let south = [0.0, 0.0, 1.0];
        let es = atmosphere::sh_eval(&d.env_sh, south);
        let lit = luminance(d.key) * (s.dir()[2]).max(0.0) + luminance(es);
        let ratio = lit / luminance(es);
        assert!((3.0..9.0).contains(&ratio), "sunlit / shadowed {ratio}");
    }
}
