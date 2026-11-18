//! One frame's sky terms: the weather, the decks, the moon, the night
//! practicals and the exposure target, and what they change in the JS
//! clear-weather `LookFrame` (eo-look) the renderer shades with.

use crate::clouds::{self, Decks};
use crate::moon::{self, Moon};
use crate::night;
use crate::weather::{self, WeatherState};
use eo_look::LookFrame;
use eo_look::math::{luminance, smoothstep};

#[derive(Clone, Copy, Debug)]
pub struct Conditions {
    pub hour: f64,
    pub weather: WeatherState,
    pub moon_day: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct SkyTerms {
    pub weather: WeatherState,
    pub decks: Decks,
    pub moon: Moon,
    /// Drawn disc radiance (sky units, before the moon colour).
    pub moon_disc: f64,
    /// 0..1: how much the moon keys the scene.
    pub moon_key: f64,
    pub star_level: f64,
    pub lamps_on: f64,
    pub dusk: f64,
    pub nightness: f64,
    pub vehicle_lamps: f64,
    pub exposure_target: f64,
    pub lamp_units_per_lux: f64,
    /// The fill's weather scale (dome brightness by day, urban glow at night).
    pub fill_scale: f64,
    /// in fog, the dome gain that makes the weather-free dome (overcast,
    /// isotropy, desaturation and tint off) as bright at the zenith as the clear
    /// sky of the hour: the sky seen from above the marine layer (1 otherwise).
    pub above_fog_gain: f64,
}

/// `NIGHT_KEY_CEILING`: the moon key never exceeds 0.02 (sky-atmosphere.js).
pub const NIGHT_KEY_CEILING: f64 = 0.02;

/// Blue hour: the unlit ground's fill at most the sky's irradiance x this
/// (the sky plus ~15 % spill: RP-8's ~4 lux minimum against ~30 lux of sky
/// at sun -4 deg).
pub const TWILIGHT_FILL_OVER_SKY: f64 = 1.15;
/// the night fill's share of the rig's (1.6 of its 4 lux on unlit ground).
pub const NIGHT_GROUND_FILL: f64 = 0.4;
/// the unlit ground's albedo for the night fill's ground lobe.
pub const NIGHT_GROUND_ALBEDO: f64 = 0.18;

/// the low-sun dome grade kept (see `apply`): the veil's share and the
/// horizon lift's ceiling.
pub const LOW_SUN_VEIL_KEEP: f64 = 0.15;
pub const LOW_SUN_HORIZON_LIFT: f64 = 0.25;
/// the afterglow arch's horizon radiance over the zenith's (see `apply`).
pub const AFTERGLOW_OVER_ZENITH: f64 = 4.0;
/// the share of the night fill's colour removed at full night.
pub const NIGHT_FILL_DESATURATION: f64 = 0.6;

/// The look frame of an hour under a weather (the weather's sky model, the
/// JS rig table), with `apply` done.
pub fn look_frame(c: &Conditions, transfer: Option<f64>) -> (LookFrame, SkyTerms) {
    let mut f = base_frame(c, transfer);
    let t = apply(&mut f, c);
    (f, t)
}

/// the frame `look_frame` starts from (the weather's sky model and
/// the rig, before `apply`): the sky integration, 0.75-0.93 ms measured.
pub fn base_frame(c: &Conditions, transfer: Option<f64>) -> LookFrame {
    LookFrame::with_sky_profile(c.hour, transfer, weather::sky_profile(&c.weather.profile))
}

/// `look_frame` for `transfer` from a `base_frame` of the same hour
/// and weather built for any transfer: only the rig's fill is re-solved
/// (`LookFrame::retarget`, microseconds), so a probe capture that moves the
/// measured transfer does not integrate the sky again on the render thread.
pub fn look_frame_from(base: &LookFrame, c: &Conditions, transfer: Option<f64>) -> (LookFrame, SkyTerms) {
    let mut f = base.retarget(transfer);
    let t = apply(&mut f, c);
    (f, t)
}

/// Apply the weather and the night to a look frame built on the weather's
/// sky model (`look_frame`) and return the terms the renderer's sky passes
/// need. The rig table is the JS's clear-weather solve: its hemisphere and
/// ambient are scaled here by the dome's energy (the environment's diffuse
/// already comes from the weather's sky). Pure.
pub fn apply(f: &mut LookFrame, c: &Conditions) -> SkyTerms {
    let w = c.weather;
    let p = w.profile;
    let alt = f.model.sun.altitude_deg;
    let nightness = night::nightness(alt);
    let o = p.overcast;
    // Fill: by day the dome's energy relative to clear (environment-ibl.js
    // `brightness`); at night the deck reflects the city's glow back
    // (`urbanGlow` rises from 0.6 clear to 1.2 in a storm), half of it.
    // x `NIGHT_GROUND_FILL` (owner-approved darker nights, 2026-11-17): the
    // rig's night fill stood for 4 lux on unlit ground, a lit street's minimum
    // between lamps; ground lit only by the city's skyglow and spill gets ~0.5-2 lux
    // (sky ~0.02-0.05 cd/m2, x pi). The lamps keep their lux scale (`lamp_units_per_lux`
    // reads the rig unscaled), so a pool stands ~19 x the unlit ground, not 7.5 x.
    let night_fill = (1.0 + 0.5 * (p.urban_glow / weather::CLEAR.urban_glow - 1.0)) * NIGHT_GROUND_FILL;
    let fill_scale = p.brightness + (night_fill - p.brightness) * nightness;
    let desat = p.desaturation * 0.5 * (1.0 - nightness);
    let grade = |c: [f64; 3], s: f64| {
        let l = luminance(c);
        [0, 1, 2].map(|i| (c[i] + (l - c[i]) * desat) * p.tint[i] * s)
    };
    f.hemi_sky = grade(f.hemi_sky, fill_scale);
    // The ground lobe carries the sunlit street's bounce: under a deck the
    // JS cuts the ground bounce by 0.8 x overcast; the lobe here is bounce and
    // sky together, so it takes 0.6 x overcast by day.
    f.hemi_ground = grade(f.hemi_ground, fill_scale * (1.0 - 0.6 * o * (1.0 - nightness)));
    f.ambient = grade(f.ambient, fill_scale);
    // the scene probe's diffuse and the sky SH it stands in for take the same
    // `NIGHT_GROUND_FILL` by nightness (the rig's environment intensity lifts them).
    f.rig.environment_intensity *= 1.0 + (NIGHT_GROUND_FILL - 1.0) * nightness;
    // Blue hour: the unlit ground is lit by the twilight sky plus a little
    // city spill, never by more. The JS rig keeps its night fill (fill
    // authority 3.4) through twilight, which made the unlit walk at sun -4
    // deg brighter than the zenith above it (radiance ratio 0.58; under a
    // Lambertian ground the sky is ~1/albedo brighter). Physically the
    // civil-twilight sky gives 3-50 lux (Bond & Henderson 1963; Spitschan et
    // al. 2016) against ~4 lux of RP-8 spill, so the fill is capped at the
    // weather sky's own irradiance x `TWILIGHT_FILL_OVER_SKY` over the blue
    // hour (sun -1 .. -8 deg, full from -3 to -5); the night solve then
    // exposes for the darker ground and the sky reads brighter than the city.
    let tw = smoothstep(-8.0, -5.0, alt) * (1.0 - smoothstep(-3.0, -1.0, alt));
    // The lamps (and lit windows) keep the night's ratio to the unlit ground
    // while the fill is capped (pool 7.5 x the ground). Physically the ratio
    // is ~1-2 at sun -4 deg (lamp and sky both ~30 lux); the JS twilight sky
    // is too dim against its own night for one scale to hold both.
    let mut lamp_scale = 1.0;
    if tw > 0.0 {
        let sky_e = luminance(f.model.sky_irradiance());
        let sh = eo_look::sh_eval(&f.env_sh, [0.0, 1.0, 0.0]);
        let env = luminance(sh) * f.rig.environment_intensity * night::SIDEWALK_ENV;
        let hemi_amb = luminance(f.hemi_sky) + luminance(f.ambient);
        let cap = sky_e * TWILIGHT_FILL_OVER_SKY;
        if hemi_amb > 1e-9 && hemi_amb + env > cap {
            let target = ((cap - env).max(0.0) / hemi_amb).min(1.0);
            let s = 1.0 + (target - 1.0) * tw;
            f.hemi_sky = f.hemi_sky.map(|v| v * s);
            f.hemi_ground = f.hemi_ground.map(|v| v * s);
            f.ambient = f.ambient.map(|v| v * s);
            lamp_scale = (s * hemi_amb + env) / (hemi_amb + env);
        }
    }
    // at night the fill's ground lobe is what the unlit ground reflects up,
    // ~albedo 0.18 x the light falling on it (the sky lobe and the ambient), not the JS
    // rig's lit-street bounce (twice the sky lobe at 21 h): walls take half of that lobe
    // and showed 3-6 x the night sky above them. The non-directional ambient goes into
    // the sky lobe, so a horizontal surface (the night solve's card) keeps its fill and
    // the exposure and the lamps do not move; a wall sees half the sky. By nightness;
    // owner-approved darker nights, 2026-11-17.
    if nightness > 0.0 {
        let up = [0, 1, 2].map(|i| f.hemi_sky[i] + f.ambient[i]);
        let lobe = up.map(|v| v * NIGHT_GROUND_ALBEDO);
        for i in 0..3 {
            f.hemi_ground[i] += (lobe[i] - f.hemi_ground[i]) * nightness;
            f.hemi_sky[i] += (up[i] - f.hemi_sky[i]) * nightness;
            f.ambient[i] *= 1.0 - nightness;
        }
    }
    // The moon (sky-atmosphere.js `lunarKey`): 0.35 x the night sky's
    // irradiance x the phase's flux x the beam, under a clear sky, at most
    // 0.02, keying the scene above 3 deg; colour [0.94, 0.95, 1.0].
    let m = moon::placement(c.hour, c.moon_day);
    let clear_sky = (1.0 - o).clamp(0.0, 1.0);
    let flux = moon::phase_flux(m.phase_angle_deg);
    let t = eo_look::rig::direct_beam_transmittance(m.altitude_deg);
    let moon_on = smoothstep(moon::KEY_MIN_ALTITUDE_DEG, moon::KEY_MIN_ALTITUDE_DEG + 3.0, m.altitude_deg) * nightness;
    let moon_i = (moon::KEY_SKY_SHARE * f.sky_irradiance_up * flux * t * clear_sky).min(NIGHT_KEY_CEILING) * moon_on;
    let sun_key = luminance(f.key);
    if moon_i > sun_key && moon_i > 0.0 {
        f.key_dir = m.dir;
        f.key = moon::COLOR.map(|v| v * moon_i);
        f.rim_dir = eo_look::rig::rim_direction(m.dir);
        // Moonlight casts soft, weak shadows.
        f.shadow_strength = 0.55 * moon_on * clear_sky;
    } else if moon_i > 0.0 {
        f.key = [0, 1, 2].map(|i| f.key[i] + moon::COLOR[i] * moon_i);
    }
    // Fog: the visibility grade relative to clear's (clear 1.0 / 1.18 is the
    // native port's base), and the haze colour toward the deck's grey.
    f.aerial.near *= p.vis_near / weather::CLEAR.vis_near;
    f.aerial.far *= p.vis_far / weather::CLEAR.vis_far;
    f.model.profile.urban_glow = p.urban_glow;
    f.model.profile.overcast = p.overcast;
    // the visible dome's grade (eo-look `DomeGrade`, the shipped look's
    // `activeDomeGrade`) at a low sun: a veil of 0.8 x the horizon's
    // luminance over the whole upper sky, the dome x 1.5 and the horizon x 2.
    // At sun 6.6 deg that puts the zenith at ~17 x a sunlit 18 % card where a
    // measured clear sky is ~2-3 x (zenith ~1,500-2,500 cd/m^2 against
    // ~10 klx on the ground: CIE 110-1994 clear sky, Kittler et al.), and the
    // anti-sun horizon at ~19 x where the Earth's shadow is darker than the
    // sky above it: the golden-hour white-out. The veil keeps 0.15 of itself
    // (the Preetham zenith alone is ~0.15 x the card: too dark without it),
    // the gain and the lift go; the desaturation toward the zenith is halved
    // (the clear zenith is the deepest blue of the day, not grey-lavender).
    // The irradiance and the probes are integrated from the ungraded sky, so
    // only what the sky pass and the reflections draw changes. A departure
    // from the JS look.
    if let Some(g) = f.model.grade.as_mut() {
        g.veil_luminance *= LOW_SUN_VEIL_KEEP;
        g.dome_gain = 1.0;
        g.horizon_lift = g.horizon_lift.min(LOW_SUN_HORIZON_LIFT);
        g.zenith_desaturation *= 0.5;
        // The skyline band (at a low sun the horizon colour, taken toward the
        // sun, spread 17 deg up all round at 0.85): the anti-sun horizon was
        // the brightest, whitest part of the sky (the milky wall over the city
        // at golden hour) where the Earth's shadow is the darkest. The base
        // band (5-12 deg, 0.42) stays.
        g.skyline_share = 0.0;
    }
    // the dome's additions (eo-look `SkyExt`): each azimuth's own horizon
    // band, the veil kept to the horizon, the narrowed aureole, at every hour
    // (the band at twilight was the one horizon colour all round: the west
    // after sunset looked like the east); and the afterglow arch toward the
    // set sun. Its horizon radiance is AFTERGLOW_OVER_ZENITH x the zenith's
    // (a clear west horizon 10-30 min after sunset is the brightest part of
    // the sky, ~3-6 x the zenith: Lee & Hernandez-Andres 2005, twilight
    // colorimetry), from sun +2 deg to full at -1 .. -5 and gone by -11 deg,
    // and none under a deck.
    let zen = luminance(f.model.sample([0.0, 1.0, 0.0]));
    let arch = smoothstep(2.0, -1.0, alt) * smoothstep(-11.0, -5.0, alt) * (1.0 - o).clamp(0.0, 1.0);
    f.model.ext = eo_look::sky::SkyExt { on: true, afterglow: AFTERGLOW_OVER_ZENITH * zen * arch };
    // a night fill is a dim light; under it colour vision fades
    // (mesopic vision, CIE 191:2010), and a city's night sky is orange-grey,
    // not the saturated blue of the JS night lobe (B/R 4): the unlit
    // street's fill loses 60 % of its colour with the night (luminance kept).
    let nd = NIGHT_FILL_DESATURATION * nightness;
    if nd > 0.0 {
        let grey = |c: [f64; 3]| {
            let l = luminance(c);
            c.map(|v| v + (l - v) * nd)
        };
        f.hemi_sky = grey(f.hemi_sky);
        f.hemi_ground = grey(f.hemi_ground);
        f.ambient = grey(f.ambient);
    }
    // the sky over the marine layer (see `SkyTerms::above_fog_gain`).
    let above_fog_gain = if w.kind == crate::weather::Kind::Fog {
        let mut m0 = f.model;
        m0.profile.overcast = 0.0;
        m0.profile.isotropy = 0.0;
        m0.profile.desaturation = 0.0;
        m0.profile.tint = [1.0; 3];
        let z0 = luminance(m0.dome([0.0, 1.0, 0.0]));
        let zc = luminance(eo_look::sky::SkyModel::new(c.hour).dome([0.0, 1.0, 0.0]));
        if z0 > 1e-9 { (zc / z0).clamp(0.25, 8.0) } else { 1.0 }
    } else {
        1.0
    };
    let exposure_target = night::target_exposure(f, &w);
    f.exposure = exposure_target;
    let night_level = f.model.field.night;
    SkyTerms {
        weather: w,
        decks: clouds::decks(&w),
        moon: m,
        moon_disc: moon::FULL_RADIANCE * moon::disc_factor(m.phase_angle_deg) * night_level * clear_sky * smoothstep(-1.1, -0.2, m.altitude_deg),
        moon_key: moon_on,
        star_level: night_level * clear_sky * 0.95,
        lamps_on: night::lamps_on(alt),
        dusk: night::dusk(alt),
        nightness,
        vehicle_lamps: night::vehicle_lamps(alt, &w),
        exposure_target,
        lamp_units_per_lux: night::lamp_units_per_lux() * lamp_scale,
        fill_scale,
        above_fog_gain,
    }
}

#[cfg(test)]
mod hitch2_tests {
    use super::*;

    /// a probe capture's new transfer re-solves the fill from the
    /// base frame; the result equals the full rebuild it replaced.
    #[test]
    fn look_frame_from_a_base_equals_the_full_rebuild() {
        for kind in [crate::Kind::Clear, crate::Kind::Fog, crate::Kind::Rain, crate::Kind::Overcast] {
            for hour in [6.5, 12.0, 17.0, 19.4, 21.0] {
                let c = Conditions { hour, weather: weather::preset(kind), moon_day: 3.0 };
                let base = base_frame(&c, Some(0.9));
                for t in [None, Some(0.4), Some(0.9), Some(1.6)] {
                    let (a, ta) = look_frame(&c, t);
                    let (b, tb) = look_frame_from(&base, &c, t);
                    let close = |x: f64, y: f64| (x - y).abs() <= 1e-9 * x.abs().max(y.abs()).max(1e-6);
                    let close3 = |x: [f64; 3], y: [f64; 3]| (0..3).all(|i| close(x[i], y[i]));
                    assert!(close3(a.key, b.key) && close3(a.rim, b.rim), "{kind:?} {hour} {t:?}: key/rim");
                    assert!(close3(a.hemi_sky, b.hemi_sky) && close3(a.hemi_ground, b.hemi_ground) && close3(a.ambient, b.ambient), "{kind:?} {hour} {t:?}: fill");
                    assert!((0..9).all(|k| close3(a.env_sh[k], b.env_sh[k])), "{kind:?} {hour} {t:?}: environment");
                    assert!(close(a.exposure, b.exposure) && close(a.shadow_strength, b.shadow_strength) && close(a.sky_irradiance_up, b.sky_irradiance_up), "{kind:?} {hour} {t:?}: exposure/shadow");
                    assert!(close(ta.exposure_target, tb.exposure_target) && close(ta.fill_scale, tb.fill_scale) && close(ta.lamp_units_per_lux, tb.lamp_units_per_lux), "{kind:?} {hour} {t:?}: terms");
                }
            }
        }
    }
}
