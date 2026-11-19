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
use std::sync::{Arc, Mutex, OnceLock, RwLock};

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

/// The land-cover class (ESA WorldCover code) at a ground-plan point (game
/// x, z metres), as `set_ground_classes` installed it.
pub type GroundClasses = Box<dyn Fn(f64, f64) -> u8 + Send + Sync>;

static GROUND_CLASSES: RwLock<Option<GroundClasses>> = RwLock::new(None);

/// Install the land-cover lookup the ground's albedo is read from (eo-app
/// main.rs, `LandCover::class_at`; 0 off the raster).
pub fn set_ground_classes(f: GroundClasses) {
    if let Ok(mut g) = GROUND_CLASSES.write() {
        *g = Some(f);
    }
    if let Ok(mut c) = GROUND_CACHE.lock() {
        *c = None;
    }
}

/// The open sea's albedo under a clear sky: 0.06 (Payne 1972, "Albedo of the
/// sea surface", J. Atmos. Sci. 29, 959-970: the monthly mean of the measured
/// ocean albedo, 0.06, at solar altitudes over about 30 deg; Jin et al. 2004,
/// "A parameterization of ocean surface albedo", GRL 31, L22301: 0.066 under a
/// diffuse sky). Neutral: the surface's Fresnel reflection is grey.
pub const SEA_ALBEDO: f64 = 0.06;

/// Tree cover's visible-band albedo: 0.05 (INFERRED). Meant as the MODIS
/// MCD43 visible-band white-sky mean of the forest classes; the per-class
/// table (Gao, Schaaf et al. 2005, J. Geophys. Res. 110, D01104) could not be
/// read, so the value is owed a primary source (data/ground-albedo.NOTICE.txt).
/// The visible band, not the plan's broadband: the renderer's light is
/// visible light, and a canopy's near-infrared reflectance does not light a wall.
pub const TREE_ALBEDO: f64 = 0.05;

/// Grass, shrub, crop and wetland cover's visible-band albedo: 0.06
/// (INFERRED, as `TREE_ALBEDO`: the MCD43 grassland class's visible mean is
/// owed a primary source). San Francisco's summer-dry grass is lighter; no
/// seasonal value is applied (data/ground-albedo.NOTICE.txt).
pub const LOW_VEGETATION_ALBEDO: f64 = 0.06;

/// Bare or sparse ground's visible-band albedo: INFERRED as the city's mean,
/// `URBAN_GROUND_ALBEDO` (beaches, dunes and rock; 1.3 % of the raster).
pub const BARE_ALBEDO: Rgb = URBAN_GROUND_ALBEDO;

/// The radius round the eye the ground's albedo is the area mean over (m):
/// the strait, the shores and the hills the near scene's shaded faces look
/// down on (INFERRED: the plan's "about 2 km"; a wall 30 m up sees half its
/// ground light from within ~30 m, so this is the scene's ground, not one face's).
pub const GROUND_ALBEDO_RADIUS_M: f64 = 2000.0;
/// The sample pitch of that mean (m), and the eye cell it is cached on.
const GROUND_ALBEDO_STEP_M: f64 = 100.0;

/// The visible-band albedo of an ESA WorldCover class (None: off the raster,
/// or a class the city has none of).
pub fn class_albedo(class: u8) -> Option<Rgb> {
    match class {
        80 => Some([SEA_ALBEDO; 3]),
        10 => Some([TREE_ALBEDO; 3]),
        20 | 30 | 40 | 90 => Some([LOW_VEGETATION_ALBEDO; 3]),
        50 => Some(URBAN_GROUND_ALBEDO),
        60 => Some(BARE_ALBEDO),
        _ => None,
    }
}

static GROUND_CACHE: Mutex<Option<((i64, i64), Rgb)>> = Mutex::new(None);

/// The area-weighted mean albedo of the land cover within
/// `GROUND_ALBEDO_RADIUS_M` of a ground-plan point; `URBAN_GROUND_ALBEDO` with
/// no land cover installed or none under the disc.
pub fn ground_albedo_mean(x: f64, z: f64) -> Rgb {
    let Ok(g) = GROUND_CLASSES.read() else { return URBAN_GROUND_ALBEDO };
    let Some(f) = g.as_ref() else { return URBAN_GROUND_ALBEDO };
    albedo_mean_over(f.as_ref(), x, z)
}

/// `ground_albedo_mean` over a given class lookup.
pub fn albedo_mean_over(f: &dyn Fn(f64, f64) -> u8, x: f64, z: f64) -> Rgb {
    let n = (GROUND_ALBEDO_RADIUS_M / GROUND_ALBEDO_STEP_M).round() as i64;
    let mut sum = [0.0; 3];
    let mut count = 0.0;
    for j in -n..=n {
        for i in -n..=n {
            if i * i + j * j > n * n {
                continue;
            }
            if let Some(a) = class_albedo(f(x + i as f64 * GROUND_ALBEDO_STEP_M, z + j as f64 * GROUND_ALBEDO_STEP_M)) {
                for c in 0..3 {
                    sum[c] += a[c];
                }
                count += 1.0;
            }
        }
    }
    if count < 1.0 {
        return URBAN_GROUND_ALBEDO;
    }
    sum.map(|v| v / count)
}

/// The broadband albedo of the ground the sky light sees from an eye (game
/// metres): the land cover's mean round it (`ground_albedo_mean`, cached per
/// 100 m eye cell), or `URBAN_GROUND_ALBEDO` everywhere with
/// `--no-ground-albedo`.
pub fn ground_albedo_at(eye: [f64; 3]) -> Rgb {
    if !crate::features::daylight().ground_albedo {
        return URBAN_GROUND_ALBEDO;
    }
    let cell = ((eye[0] / GROUND_ALBEDO_STEP_M).floor() as i64, (eye[2] / GROUND_ALBEDO_STEP_M).floor() as i64);
    if let Ok(c) = GROUND_CACHE.lock()
        && let Some((k, a)) = *c
        && k == cell
    {
        return a;
    }
    let a = ground_albedo_mean((cell.0 as f64 + 0.5) * GROUND_ALBEDO_STEP_M, (cell.1 as f64 + 0.5) * GROUND_ALBEDO_STEP_M);
    if let Ok(mut c) = GROUND_CACHE.lock() {
        *c = Some((cell, a));
    }
    a
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
    /// the ground's share of `env_sh` alone.
    pub env_sh_ground: [Rgb; 9],
    /// the ground's albedo the environment was made over.
    pub ground: Rgb,
    /// the illuminance on a horizontal surface, sun and sky (look units, luminance).
    pub e_h: f64,
    tables: Arc<Tables>,
}

/// One face's light by source (`Daylight::face_split`).
#[derive(Clone, Copy, Debug)]
pub struct FaceLight {
    pub sun: Rgb,
    pub sky: Rgb,
    pub ground: Rgb,
}

static LAST: Mutex<Option<Daylight>> = Mutex::new(None);

/// The daylight made last in this process (the renderer's frame's, while
/// the physical daylight is on): what a shot's light split is logged from.
pub fn last_daylight() -> Option<Daylight> {
    LAST.lock().ok().and_then(|l| l.clone())
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
        Self::new_with_ground(sun, density, weight, URBAN_GROUND_ALBEDO)
    }

    /// `new` over a ground of broadband albedo `ground` (the sky light's
    /// lower hemisphere; `new` takes `URBAN_GROUND_ALBEDO`).
    pub fn new_with_ground(sun: [f64; 3], density: f64, weight: f64, ground: Rgb) -> Self {
        let tables = atmosphere::tables(&Atmosphere::coastal_clear(), density);
        let l = atmosphere::sky_light(&tables, sun, ground);
        let e = e_toa_units();
        let c = atmosphere::sun_colour_toa();
        let sun_toa = c.map(|v| v * e);
        let mut key = [0, 1, 2].map(|i| sun_toa[i] * l.sun[i]);
        // the beam's colour from its spectrum (`spectral`), at the three channels' luminance
        // (the exposure and every brightness keep their scale): `--no-sun-spectrum2`.
        if crate::features::colour().sun_spectrum2
            && let Some(b) = crate::spectral::beam_srgb(&tables.atm, density, 2.0, sun[1])
        {
            let (lk, lb) = (luminance(key), luminance(b));
            if lb > 1e-12 {
                key = b.map(|v| v / lb * lk);
            }
        }
        let env_sh = l.sh.map(|k| [k[0] * sun_toa[0], k[1] * sun_toa[1], k[2] * sun_toa[2]]);
        let env_sh_ground = l.sh_ground.map(|k| [k[0] * sun_toa[0], k[1] * sun_toa[1], k[2] * sun_toa[2]]);
        let e_h = luminance(key) * sun[1].max(0.0) + luminance([0, 1, 2].map(|i| l.sky_h[i] * sun_toa[i]));
        let d = Self { weight, density, sun, sun_toa, key, env_sh, env_sh_ground, ground, e_h, tables };
        if let Ok(mut last) = LAST.lock() {
            *last = Some(d.clone());
        }
        d
    }

    /// A face's light by source (look units, irradiance on the face, before
    /// shadow, occlusion and the material): the sun's direct beam, the sky's
    /// diffuse light and the ground's. The specular and the scene's bounce
    /// are view- and scene-dependent and drawn on the GPU only.
    pub fn face_split(&self, n: [f64; 3]) -> FaceLight {
        let ground = atmosphere::sh_eval(&self.env_sh_ground, n);
        let all = atmosphere::sh_eval(&self.env_sh, n);
        let cos = (n[0] * self.sun[0] + n[1] * self.sun[1] + n[2] * self.sun[2]).max(0.0);
        FaceLight { sun: self.key.map(|k| k * cos), sky: [0, 1, 2].map(|c| (all[c] - ground[c]).max(0.0)), ground }
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
    fn the_ground_seam_matches_new_at_the_urban_albedo() {
        let s = crate::sun::sun_direction(13.0);
        let a = Daylight::new(s.dir(), 1.0, 1.0);
        let b = Daylight::new_with_ground(s.dir(), 1.0, 1.0, ground_albedo_at([0.0, 1.7, 0.0]));
        assert_eq!(a.key, b.key);
        assert_eq!(a.env_sh, b.env_sh);
        assert_eq!(a.e_h, b.e_h);
    }

    #[test]
    fn the_ground_over_the_strait_is_the_sea_and_lights_a_shaded_wall_less() {
        // a disc that is all water reads the sea's albedo; half city, half water the mean
        let sea = albedo_mean_over(&|_, _| 80, 0.0, 0.0);
        assert!(sea.iter().all(|a| (a - SEA_ALBEDO).abs() < 1e-12), "{sea:?}");
        let half = albedo_mean_over(&|x, _| if x < 0.0 { 80 } else { 50 }, 0.0, 0.0);
        assert!(half[1] > SEA_ALBEDO && half[1] < URBAN_GROUND_ALBEDO[1], "{half:?}");
        // off the raster (class 0) everywhere: the city's
        assert_eq!(albedo_mean_over(&|_, _| 0, 0.0, 0.0), URBAN_GROUND_ALBEDO);
        // a north wall in shade over the sea gets less light, and the sun : shade ratio on a
        // lit west wall at 17 h rises
        let s = crate::sun::sun_direction(17.0);
        let city = Daylight::new_with_ground(s.dir(), 1.0, 1.0, URBAN_GROUND_ALBEDO);
        let water = Daylight::new_with_ground(s.dir(), 1.0, 1.0, sea);
        let north = [0.0, 0.0, -1.0];
        let (sc, sw) = (city.face_split(north), water.face_split(north));
        assert!(luminance(sw.ground) < 0.5 * luminance(sc.ground), "{sw:?} {sc:?}");
        assert!((luminance(sw.sky) - luminance(sc.sky)).abs() < 1e-9);
        let west = [-1.0, 0.0, 0.0];
        let ratio = |d: &Daylight| {
            let f = d.face_split(west);
            let g = d.face_split(north);
            luminance(f.sun) / luminance([0, 1, 2].map(|c| g.sky[c] + g.ground[c]))
        };
        assert!(ratio(&water) > 1.2 * ratio(&city), "{} {}", ratio(&water), ratio(&city));
    }

    #[test]
    fn the_face_split_adds_up_to_the_environment() {
        let s = crate::sun::sun_direction(10.0);
        let d = Daylight::new(s.dir(), 1.0, 1.0);
        for n in [[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 1.0, 0.0]] {
            let f = d.face_split(n);
            let e = atmosphere::sh_eval(&d.env_sh, n);
            for c in 0..3 {
                assert!((f.sky[c] + f.ground[c] - e[c].max(0.0)).abs() < 1e-6 * (1.0 + e[c].abs()), "{n:?} {f:?} {e:?}");
            }
        }
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
