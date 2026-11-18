//! Physical references the look is checked against, each with its source,
//! and the conversion between the game's light units and photometric ones.
//!
//! The game's units come from the JS (environment-ibl.js `sceneIlluminance`
//! and sky-atmosphere.js `SUN_NORMAL_ILLUMINANCE_LUX`): 1e5 lux of normal sun
//! is irradiance 2.75 in three's units, so one unit of irradiance is
//! 36,364 lux and one unit of radiance is 36,364 cd/m^2 (a white Lambertian
//! surface under E units returns E / pi units of radiance).

/// The JS's nominal scale (`sceneIlluminance`: 1e5 lux of normal sun = 2.75
/// units). Its own light rig does not follow it: the rig's noon key alone is
/// 2.4x that (6.8 units of normal key at noon), so the absolute scale is
/// calibrated on one documented point instead (`CLEAR_NOON_CALIBRATION_LUX`)
/// and every other band is checked through that one calibration.
pub const JS_NOMINAL_LUX_PER_UNIT: f64 = 1e5 / 2.75;

/// Clear-sky global horizontal illuminance with the sun 50 deg up (San
/// Francisco's noon on September 22): direct normal ~95 klx at air mass 1.3
/// (Littlefair 1985 luminous efficacy ~100 lm/W on ~950 W/m^2) gives ~73 klx
/// on the horizontal, plus ~15-20 klx of sky: 90 klx.
pub const CLEAR_NOON_CALIBRATION_LUX: f64 = 90_000.0;

/// Lux per unit of irradiance (and cd/m^2 per unit of radiance): the clear
/// noon rig's global horizontal illuminance (key on the horizontal plus the
/// fill) set to `CLEAR_NOON_CALIBRATION_LUX`.
pub fn lux_per_unit() -> f64 {
    static S: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    *S.get_or_init(|| {
        let f = eo_look::LookFrame::new(12.0);
        let key = eo_look::math::luminance(f.key) * f.key_dir[1].max(0.0);
        let sh = eo_look::sh_eval(&f.env_sh, [0.0, 1.0, 0.0]);
        let env = f.rig.environment_intensity * crate::night::SIDEWALK_ENV;
        let fill = eo_look::math::luminance([0, 1, 2].map(|i| f.hemi_sky[i] + f.ambient[i] + sh[i] * env));
        CLEAR_NOON_CALIBRATION_LUX / (key + fill)
    })
}

pub fn lux(e_units: f64) -> f64 {
    e_units * lux_per_unit()
}

pub fn cd_m2(l_units: f64) -> f64 {
    l_units * lux_per_unit()
}

/// A documented band.
#[derive(Clone, Copy, Debug)]
pub struct Band {
    pub what: &'static str,
    pub lo: f64,
    pub hi: f64,
    pub unit: &'static str,
    pub source: &'static str,
}

impl Band {
    pub fn holds(&self, v: f64) -> bool {
        v >= self.lo && v <= self.hi
    }
}

/// Direct + sky on a horizontal surface, clear sky, sun 50-60 degrees up.
pub const CLEAR_NOON_GLOBAL: Band = Band {
    what: "clear-sky global horizontal illuminance, sun 50-60 deg",
    lo: 60_000.0,
    hi: 120_000.0,
    unit: "lux",
    source: "IES Lighting Handbook (10th ed.) daylight tables: direct sun 32,000-100,000 lux plus sky 10,000-25,000 lux; CIE 110-1994 clear-sky models",
};

/// Sky (diffuse) illuminance in open shade, clear noon.
pub const CLEAR_NOON_SHADE: Band = Band {
    what: "clear-sky diffuse (open shade) horizontal illuminance at noon",
    lo: 10_000.0,
    hi: 25_000.0,
    unit: "lux",
    source: "IES Lighting Handbook: full daylight, not direct sun, 10,000-25,000 lux",
};

/// Sunlit over shade on a horizontal surface at high sun.
pub const SUN_OVER_SHADE: Band = Band {
    what: "sunlit / open-shade illuminance ratio, sun 50-60 deg, clear",
    lo: 4.0,
    hi: 9.0,
    unit: "ratio",
    source: "the two IES bands above (global 60-120 klx over diffuse 10-25 klx); the JS 11:00 clear card's delivered lit/shadow band 5.0-7.0 (environment-ibl.js groundAlbedo record)",
};

/// Overcast-day horizontal illuminance at midday.
pub const OVERCAST_NOON_GLOBAL: Band = Band {
    what: "overcast-day horizontal illuminance at midday",
    lo: 5_000.0,
    hi: 40_000.0,
    unit: "lux",
    source: "IES Lighting Handbook: overcast day 1,000-10,000 lux (dark to bright overcast), thin overcast to 40,000 lux; CIE 110-1994 overcast sky (E_h = 7 pi L_z / 9)",
};

/// The photographic exposure scale: an incident-light meter's EV at ISO 100,
/// EV100 = log2(E / 2.5) with the ISO 2720 incident calibration C = 250
/// (lux, ISO 100). Sunny-16: EV 15 in clear sun.
pub fn ev100_incident(e_lux: f64) -> f64 {
    (e_lux.max(1e-9) / 2.5).log2()
}

/// Display (sRGB, 0..255) of a grey card exposed at photographic middle
/// grey: 18 % reflectance shown at 18 % display luminance (L* 50) = sRGB 118;
/// one stop either side = 84..161 (ISO 2720 / ISO 12232 middle-grey practice).
pub const GREY_CARD_MIDDLE: Band = Band {
    what: "18 % grey card under the scene's key light, photographic middle grey +-1 EV",
    lo: 84.0,
    hi: 161.0,
    unit: "sRGB",
    source: "ISO 12232 / ISO 2720 metering to middle grey (18 % card at L* 50, sRGB 118); +-1 EV latitude",
};

/// Moonlight.
pub const FULL_MOON: Band = Band {
    what: "full moon high in a clear sky, horizontal illuminance",
    lo: 0.05,
    hi: 0.3,
    unit: "lux",
    source: "Kyba et al. 2017 (Sci. Rep. 7:2321): full moon at most ~0.3 lux; the JS MOON_FULL_ILLUMINANCE_LUX 0.267",
};

/// Twilight illuminance at the end of civil twilight (sun -6 deg).
pub const CIVIL_TWILIGHT_END: Band = Band {
    what: "horizontal illuminance, sun at -6 deg, clear",
    lo: 1.0,
    hi: 10.0,
    unit: "lux",
    source: "Bond & Henderson 1963 / US Naval Observatory: about 3.4 lux at the end of civil twilight; Spitschan et al. 2016 (Sci. Rep. 6:26756) measured 1-10 lux at -6 deg",
};

/// Street lighting (ANSI/IES RP-8-18, roadway, medium pedestrian conflict):
/// local 4 lux, collector 6 lux, major 9 lux average maintained illuminance;
/// uniformity average:minimum 6:1 (local), 4:1 (collector), 3:1 (major).
pub const STREET_AVERAGE: Band = Band {
    what: "average maintained road illuminance under street lighting",
    lo: 4.0,
    hi: 17.0,
    unit: "lux",
    source: "ANSI/IES RP-8-18 roadway illuminance criteria: 4 (local) .. 9 (major) lux at medium pedestrian conflict, to 17 lux (major, high conflict)",
};

pub const STREET_UNIFORMITY_MAX_MIN: Band = Band {
    what: "street lighting maximum:minimum illuminance on the road",
    lo: 3.0,
    hi: 20.0,
    unit: "ratio",
    source: "ANSI/IES RP-8-18: average:minimum 3:1-6:1, and maximum:minimum well under 20:1 for the veiling-luminance criteria; the JS practical ratio, pool peak 7.6x the unlit ground (sky-atmosphere.js practical peaks)",
};

/// Low-beam headlamp on the road ahead.
pub const LOW_BEAM_AT_25M: Band = Band {
    what: "low-beam illuminance on the road 25 m ahead (hot spot)",
    lo: 12.0,
    hi: 60.0,
    unit: "lux",
    source: "UN ECE Regulation 112 low-beam test points (75R/50R >= 12 lux at 25 m on the screen); typical LED low beams 20-60 lux there",
};

/// The illuminance the game's unlit night ground stands for: the minimum
/// between lamps on a lit city street (RP-8 local: average 4 lux at 6:1,
/// collector 6 lux at 4:1) plus window spill and skyglow; set so that a lamp
/// pool of `LAMP_POOL_PEAK_LUX` is 7.5 x the unlit ground, the JS's own
/// practical ratio (7.6).
pub const NIGHT_UNLIT_GROUND_LUX: f64 = 4.0;
/// Peak illuminance under an arterial lamp: average 9-12 lux on a major road
/// with a 3:1 average:minimum gives peaks of 25-35 lux.
pub const LAMP_POOL_PEAK_LUX: f64 = 30.0;

/// Luminous intensity toward the nadir of a luminaire whose pool peaks at
/// `LAMP_POOL_PEAK_LUX` from `height_m` (inverse square: E = I / h^2).
pub fn nadir_candela_for_pool(height_m: f64) -> f64 {
    LAMP_POOL_PEAK_LUX * height_m * height_m
}

/// Low-beam peak intensity (ECE R112 class B low beams: 15,000-30,000 cd at
/// the hot spot; 20,000 cd puts 32 lux on the road 25 m ahead).
pub const LOW_BEAM_PEAK_CD: f64 = 20_000.0;
