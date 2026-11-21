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
    /// 0..1: how far the key has gone over from the sun to the moon (`apply`:
    /// 0 or 1, the moon taking over outright; `apply_with` smooth: the blend
    /// weight, `moon_key_weight`).
    pub moon_weight: f64,
    /// The moon's key (luminance, the scene's key units; 0 with the moon down or by
    /// day): `apply` adds it to the sun's key or keys with it alone.
    pub moon_intensity: f64,
    /// the physical daylight's share of the frame (eo-look `phys`; 0 without
    /// it, `apply_daylight`), and the day's EV100 when the incident meter sets
    /// the exposure (`apply_ev100`; None with the JS curve).
    pub daylight_weight: f64,
    pub ev100: Option<f64>,
}

/// What `apply_with` changes from `apply` (all off: `apply`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameOptions {
    /// `--smooth-time`: the rig rows through a monotone cubic
    /// (`eo_look::rig::Interp::MonotoneCubic`), the sun and moon keys blended
    /// (`moon_key_weight`) instead of switched, and the vehicle lamps ramped on
    /// (`night::vehicle_lamps_smooth`) instead of stepping on at 0.35.
    pub smooth: bool,
    /// `--look cinematic` (the cinematic look plan, stage L4): the sun's
    /// colour by altitude, the directional sky fill, the rim cut back and a
    /// darker evening exposure (`apply_cinematic_*`,
    /// eo-look `golden`). False: today's frame, bit for bit.
    pub cinematic: bool,
}

impl FrameOptions {
    pub const SMOOTH: Self = Self { smooth: true, cinematic: false };

    /// The rig interpolation these options ask for.
    pub fn interp(&self) -> eo_look::rig::Interp {
        if self.smooth { eo_look::rig::Interp::MonotoneCubic } else { eo_look::rig::Interp::Linear }
    }
}

/// The sun altitudes (degrees) across which the key goes over from the sun
/// to the moon (`moon_key_weight`): civil twilight, from sunset (0 deg) to
/// -6 deg (Spitschan, Aguirre, Brainard & Sweeney 2016, Sci. Rep. 6:26756,
/// the twilight phases; reference sheet 3.2). Over it the light falls about
/// 200 x (759 lx at 0 deg to 3.4 lx at -6 deg, Schlyter), so the handover
/// takes the twilight's own half hour at San Francisco's latitude instead of
/// the minute and a half a weight on the two keys' ratio gave (the sun's key
/// runs to nothing within a degree of the horizon).
pub const MOON_KEY_TWILIGHT_DEG: [f64; 2] = [0.0, -6.0];

/// The moon's weight in the key (`--smooth-time`): a smoothstep on the
/// sun's altitude across civil twilight (`MOON_KEY_TWILIGHT_DEG`: 0 with
/// the sun up, 1 from -6 deg down), times the moon's own rise through the
/// altitudes at which it starts to key (`moon::KEY_MIN_ALTITUDE_DEG` to
/// 3 deg above it, the ramp `moon_on` takes), so a moon rising at night
/// takes the key over smoothly too; 0 with no moon key.
pub fn moon_key_weight(moon_i: f64, sun_alt_deg: f64, moon_alt_deg: f64) -> f64 {
    if !(moon_i > 0.0) {
        return 0.0;
    }
    let [up, down] = MOON_KEY_TWILIGHT_DEG;
    smoothstep(up, down, sun_alt_deg) * smoothstep(moon::KEY_MIN_ALTITUDE_DEG, moon::KEY_MIN_ALTITUDE_DEG + 3.0, moon_alt_deg)
}

/// `look_frame` with options (`FrameOptions::default()` is `look_frame`).
pub fn look_frame_with(c: &Conditions, transfer: Option<f64>, o: FrameOptions) -> (LookFrame, SkyTerms) {
    let mut f = base_frame_with(c, transfer, o);
    let t = apply_with(&mut f, c, o);
    (f, t)
}

/// `base_frame` with options (the rig's interpolation).
pub fn base_frame_with(c: &Conditions, transfer: Option<f64>, o: FrameOptions) -> LookFrame {
    LookFrame::with_sky_profile_interp(c.hour, transfer, weather::sky_profile(&c.weather.profile), o.interp())
}

/// `look_frame_from` with options; equal to `look_frame_with`.
pub fn look_frame_from_with(base: &LookFrame, c: &Conditions, transfer: Option<f64>, o: FrameOptions) -> (LookFrame, SkyTerms) {
    let mut f = base.retarget_interp(transfer, o.interp());
    let t = apply_with(&mut f, c, o);
    (f, t)
}

/// Two frames' terms `t` (0..1) of the way from `a` to `b` (keyframes a
/// clock minute apart under the same weather, `--smooth-time`): numbers
/// linear, the moon's direction through a slerp; the weather, the decks and
/// the moon's waxing are `a`'s (the same weather and night on both sides).
pub fn blend_terms(a: &SkyTerms, b: &SkyTerms, t: f64) -> SkyTerms {
    let t = t.clamp(0.0, 1.0);
    let m = |x: f64, y: f64| x + (y - x) * t;
    SkyTerms {
        weather: a.weather,
        decks: a.decks,
        moon: Moon {
            dir: eo_look::blend::slerp_dir(a.moon.dir, b.moon.dir, t),
            altitude_deg: m(a.moon.altitude_deg, b.moon.altitude_deg),
            phase_angle_deg: m(a.moon.phase_angle_deg, b.moon.phase_angle_deg),
            illuminated: m(a.moon.illuminated, b.moon.illuminated),
            waxing: a.moon.waxing,
        },
        moon_disc: m(a.moon_disc, b.moon_disc),
        moon_key: m(a.moon_key, b.moon_key),
        star_level: m(a.star_level, b.star_level),
        lamps_on: m(a.lamps_on, b.lamps_on),
        dusk: m(a.dusk, b.dusk),
        nightness: m(a.nightness, b.nightness),
        vehicle_lamps: m(a.vehicle_lamps, b.vehicle_lamps),
        exposure_target: m(a.exposure_target, b.exposure_target),
        lamp_units_per_lux: m(a.lamp_units_per_lux, b.lamp_units_per_lux),
        fill_scale: m(a.fill_scale, b.fill_scale),
        above_fog_gain: m(a.above_fog_gain, b.above_fog_gain),
        moon_weight: m(a.moon_weight, b.moon_weight),
        moon_intensity: m(a.moon_intensity, b.moon_intensity),
        daylight_weight: m(a.daylight_weight, b.daylight_weight),
        ev100: match (a.ev100, b.ev100) {
            (Some(x), Some(y)) => Some(m(x, y)),
            (x, y) => x.or(y),
        },
    }
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

/// (`--look cinematic`, stage L4) the sun's key in eo-look `golden::sun_colour`
/// of its altitude, its luminance kept (the rig table's intensity, envelope
/// and beam): a whiter noon and a golden-to-red low sun. The moon's key is
/// added after this, untouched.
pub fn apply_cinematic_key(f: &mut LookFrame) {
    let l = luminance(f.key);
    f.key = eo_look::golden::sun_colour(f.model.sun.altitude_deg).map(|v| v * l);
}

/// (`--look cinematic`, stage L4) by day, `golden::directional_sky_share` of
/// the fill moves from the rig's constant sky lobe and ambient to the sky's
/// own directional irradiance (the upper dome, `golden::upper_sky_sh`), added
/// to the diffuse environment so a face toward a low sun's glowing west is
/// warm and one toward the east blue, an upward face's fill kept
/// (`golden::directional_split`); the environment's
/// intensity is the shader's x the sidewalk's class (`night::SIDEWALK_ENV`),
/// so the unlit ground's fill the camera meter and the night solve read
/// (`night::ground_fill`) is kept exactly (with the painted and asphalt
/// classes' mean the meter saw ~9 % less fill and took the frame down with it).
/// The rim keeps `golden::RIM_KEEP`, the rest moved into the sky lobe at what
/// it gave an upward face. Both fade out as the sun sets (3 deg up to 1 deg down).
pub fn apply_cinematic_fill(f: &mut LookFrame, nightness: f64) {
    // (with the sun up: from 1 deg down to 3 deg up, as the exposure; the blue hour is left as it is)
    let day = (1.0 - nightness).clamp(0.0, 1.0) * smoothstep(-1.0, 3.0, f.model.sun.altitude_deg);
    if day <= 0.0 {
        return;
    }
    // the rim's removed share moves into the hemisphere's sky lobe at what it gave an upward
    // face (its 28 deg elevation: x sin 28 deg), so the ground in shade keeps its light (cut
    // outright, ~20 % of the shaded ground's fill at golden hour went with it); a wall turned
    // from the sun keeps half of that where the rim gave it ~cos 28 deg.
    let cut = (1.0 - eo_look::golden::RIM_KEEP) * day;
    let up_share = f.rim_dir[1].max(0.0);
    for i in 0..3 {
        f.hemi_sky[i] += f.rim[i] * cut * up_share;
    }
    f.rim = f.rim.map(|v| v * (1.0 - cut));
    let env = f.rig.environment_intensity * night::SIDEWALK_ENV;
    if !(env > 1e-9) {
        return;
    }
    let upper = eo_look::golden::upper_sky_sh(&f.model);
    let (k, s) = eo_look::golden::directional_split(f.hemi_sky, f.ambient, &upper, eo_look::golden::directional_sky_share(f.model.sun.altitude_deg));
    let s = 1.0 + (s - 1.0) * day;
    let k = k * day / env;
    f.hemi_sky = f.hemi_sky.map(|v| v * s);
    f.ambient = f.ambient.map(|v| v * s);
    for (c, u) in f.env_sh.iter_mut().zip(upper.iter()) {
        for ch in 0..3 {
            c[ch] += u[ch] * k;
        }
    }
}

/// (`--look cinematic`, stage L4) the exposure target with the day curve's
/// adaptation at `golden::EXPOSURE_ADAPTATION` instead of the JS's 0.20:
/// the day part of the target x (curve / anchor)^(0.12 / 0.20 - 1), where
/// `curve` is the base frame's `recommended_exposure` (the anchor at noon, so
/// noon is unchanged; 1.30 -> ~1.24 at 18.5 h), faded out as the sun sets
/// (from 3 deg up to 1 deg down: the blue hour is left as it is). The night
/// solve's share is left as it is.
pub fn cinematic_exposure(f: &LookFrame, w: &WeatherState, target: f64, curve: f64) -> f64 {
    let n = night::exposure_nightness(f.model.sun.altitude_deg);
    if n >= 1.0 {
        return target;
    }
    let ratio = (curve / eo_look::rig::EXPOSURE_ANCHOR).max(1.0);
    let g = ratio.powf(eo_look::golden::EXPOSURE_ADAPTATION / eo_look::rig::EXPOSURE_ADAPTATION - 1.0);
    let g = 1.0 + (g - 1.0) * smoothstep(-1.0, 3.0, f.model.sun.altitude_deg);
    let day = night::day_exposure(f, w);
    target + day * (g - 1.0) * (1.0 - n)
}

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
    apply_with(f, c, FrameOptions::default())
}

/// `apply` with options (`FrameOptions::default()` is `apply`).
pub fn apply_with(f: &mut LookFrame, c: &Conditions, opts: FrameOptions) -> SkyTerms {
    let w = c.weather;
    let p = w.profile;
    let alt = f.model.sun.altitude_deg;
    let nightness = night::nightness(alt);
    let o = p.overcast;
    // (`--look cinematic`, stage L4) the base frame's day curve, before the target replaces it.
    let curve_exposure = f.exposure;
    if opts.cinematic {
        apply_cinematic_key(f);
    }
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
    if opts.cinematic {
        apply_cinematic_fill(f, nightness);
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
    let mut moon_weight = 0.0;
    if opts.smooth {
        // the keys blended by the moon's weight (the sun's altitude across
        // civil twilight): before the takeover the moon adds to the sun's key
        // (as below), after it the moon is the key; the directions turn
        // through a slerp and the shadow follows the weight.
        let wm = moon_key_weight(moon_i, alt, m.altitude_deg);
        if moon_i > 0.0 {
            let moon_key = moon::COLOR.map(|v| v * moon_i);
            f.key = [0, 1, 2].map(|i| f.key[i] * (1.0 - wm) + moon_key[i]);
            f.key_dir = eo_look::blend::slerp_dir(f.key_dir, m.dir, wm);
            f.rim_dir = eo_look::blend::slerp_dir(f.rim_dir, eo_look::rig::rim_direction(m.dir), wm);
            f.shadow_strength += (0.55 * moon_on * clear_sky - f.shadow_strength) * wm;
        }
        moon_weight = wm;
    } else if moon_i > sun_key && moon_i > 0.0 {
        f.key_dir = m.dir;
        f.key = moon::COLOR.map(|v| v * moon_i);
        f.rim_dir = eo_look::rig::rim_direction(m.dir);
        // Moonlight casts soft, weak shadows.
        f.shadow_strength = 0.55 * moon_on * clear_sky;
        moon_weight = 1.0;
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
    let mut exposure_target = night::target_exposure(f, &w);
    if opts.cinematic {
        exposure_target = cinematic_exposure(f, &w, exposure_target, curve_exposure);
    }
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
        vehicle_lamps: if opts.smooth { night::vehicle_lamps_smooth(alt, &w) } else { night::vehicle_lamps(alt, &w) },
        exposure_target,
        lamp_units_per_lux: night::lamp_units_per_lux() * lamp_scale,
        fill_scale,
        above_fog_gain,
        moon_weight,
        moon_intensity: moon_i,
        daylight_weight: 0.0,
        ev100: None,
    }
}

/// The haze density the physical daylight is integrated at for a weather:
/// the clear coastal medium x (1 + 2 haze), as the GPU's tables take it.
pub fn daylight_density(w: &WeatherState) -> f64 {
    1.0 + 2.0 * w.profile.haze
}

/// The physical daylight's weight for a frame's sun and weather (eo-look `phys::weather_weight`).
pub fn daylight_weight(f: &LookFrame, w: &WeatherState) -> f64 {
    eo_look::phys::weather_weight(f.model.sun.altitude_deg, w.profile.overcast)
}

/// Light a frame `look_frame_from` built with the physical daylight
/// (`--no-phys-sky` leaves the rig's light): eo-look `phys::apply` by the
/// daylight's weight, the weather's dome brightness and tint on its diffuse
/// share as `apply` puts them on the rig's fill. The exposure target is
/// left to `apply_ev100` (or stays the JS curve's).
pub fn apply_daylight(f: &mut LookFrame, t: &mut SkyTerms, d: &eo_look::phys::Daylight) {
    let p = t.weather.profile;
    let mut d2 = d.clone();
    let grade = |c: [f64; 3]| [0, 1, 2].map(|i| c[i] * p.tint[i] * p.brightness);
    d2.env_sh = d.env_sh.map(grade);
    eo_look::phys::apply(f, &d2);
    t.daylight_weight = d.weight;
}

/// The night without the rig's rim light (`--no-night-shaping` keeps it): the
/// browser game's rig carries a rim of 0.3-0.45 units at full night (four to five
/// times the night sky's own fill on a wall), aimed 28 deg up opposite the key's
/// heading, a stand-in for moonlight that lit every wall facing it as a uniform
/// grey-blue glow (card 6's white wall read as lit by day). At night an unlit
/// surface is lit by the sky's glow (the hemisphere and the environment) and the
/// practicals (street and vehicle lamps, windows, shop spill) alone; the moon,
/// when it is up, keys the scene through `apply`. The rim fades with `nightness`.
pub fn apply_night_shaping(f: &mut LookFrame, t: &SkyTerms) {
    let k = 1.0 - t.nightness.clamp(0.0, 1.0);
    f.rim = f.rim.map(|v| v * k);
}

/// Set the frame's exposure from an incident-light meter (EV100) while the
/// sun is up (`--no-ev100` keeps the JS curve): `night::target_exposure_ev100`
/// on the frame's own horizontal illuminance.
pub fn apply_ev100(f: &mut LookFrame, t: &mut SkyTerms) {
    let w = &t.weather;
    let e_h = night::horizontal_illuminance(f, w, t.daylight_weight);
    let weight = eo_look::phys::weight(f.model.sun.altitude_deg);
    let e = night::target_exposure_ev100(f, w, e_h, weight);
    t.exposure_target = e;
    f.exposure = e;
    t.ev100 = Some(night::ev100_day_exposure(e_h).0);
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

#[cfg(test)]
mod cinematic_golden_tests {
    //! (`--look cinematic`, stage L4) the golden hour on the frame.
    use super::*;

    const CINE: FrameOptions = FrameOptions { smooth: false, cinematic: true };

    fn clear(hour: f64) -> Conditions {
        Conditions { hour, weather: weather::preset(crate::Kind::Clear), moon_day: 3.0 }
    }

    #[test]
    fn the_default_options_are_todays_frame() {
        assert!(!FrameOptions::default().cinematic && !FrameOptions::SMOOTH.cinematic);
        for hour in [7.0, 12.0, 18.5, 21.0] {
            let c = clear(hour);
            let (a, ta) = look_frame(&c, Some(0.8));
            let (b, tb) = look_frame_with(&c, Some(0.8), FrameOptions::default());
            assert_eq!((a.key, a.rim, a.hemi_sky, a.ambient, a.env_sh, a.exposure), (b.key, b.rim, b.hemi_sky, b.ambient, b.env_sh, b.exposure));
            assert_eq!(ta.exposure_target, tb.exposure_target);
        }
    }

    #[test]
    fn the_golden_hour_is_deterministic_and_the_same_on_both_paths() {
        for kind in [crate::Kind::Clear, crate::Kind::Overcast, crate::Kind::Fog] {
            for hour in [9.0, 12.0, 18.5, 19.0] {
                let c = Conditions { hour, weather: weather::preset(kind), moon_day: 3.0 };
                for smooth in [false, true] {
                    let o = FrameOptions { smooth, cinematic: true };
                    let (a, ta) = look_frame_with(&c, Some(0.9), o);
                    let (b, tb) = look_frame_with(&c, Some(0.9), o);
                    assert_eq!((a.key, a.rim, a.hemi_sky, a.ambient, a.env_sh, a.exposure), (b.key, b.rim, b.hemi_sky, b.ambient, b.env_sh, b.exposure), "{kind:?} {hour}: determinism");
                    assert_eq!(ta.exposure_target, tb.exposure_target);
                    // a probe's new transfer re-solves the fill from the base, as the full rebuild
                    let base = base_frame_with(&c, Some(0.4), o);
                    let (r, tr) = look_frame_from_with(&base, &c, Some(0.9), o);
                    let close = |x: f64, y: f64| (x - y).abs() <= 1e-9 * x.abs().max(y.abs()).max(1e-6);
                    let close3 = |x: [f64; 3], y: [f64; 3]| (0..3).all(|i| close(x[i], y[i]));
                    assert!(close3(a.key, r.key) && close3(a.hemi_sky, r.hemi_sky) && close3(a.ambient, r.ambient) && (0..9).all(|k| close3(a.env_sh[k], r.env_sh[k])), "{kind:?} {hour}: the base path");
                    assert!(close(ta.exposure_target, tr.exposure_target), "{kind:?} {hour}: exposure on the base path");
                }
            }
        }
    }

    #[test]
    fn the_hours_separate_and_noon_keeps_its_light() {
        let (d12, _) = look_frame(&clear(12.0), None);
        let (c12, _) = look_frame_with(&clear(12.0), None, CINE);
        let (d185, _) = look_frame(&clear(18.5), None);
        let (c185, _) = look_frame_with(&clear(18.5), None, CINE);
        let rb = |k: [f64; 3]| k[0] / k[2];
        // the key's luminance kept, a whiter noon and a warmer golden hour
        // (the knots' luminance is 1 to their four decimals)
        assert!((luminance(c12.key) / luminance(d12.key) - 1.0).abs() < 3e-3 && (luminance(c185.key) / luminance(d185.key) - 1.0).abs() < 3e-3);
        assert!(rb(c12.key) < 1.25 && rb(d12.key) > 1.7, "noon red/blue {} (today {})", rb(c12.key), rb(d12.key));
        assert!(rb(c185.key) / rb(c12.key) > 3.0, "golden over noon {} (today {})", rb(c185.key) / rb(c12.key), rb(d185.key) / rb(d12.key));
        // noon's exposure unchanged, golden hour's darker
        assert_eq!(c12.exposure, d12.exposure);
        assert!(c185.exposure < 0.97 * d185.exposure && c185.exposure > 0.9 * d185.exposure, "18.5 h exposure {} (today {})", c185.exposure, d185.exposure);
        // the fill at noon: the ground and the walls' mean within a few % of today's
        let fill = |f: &LookFrame, n: [f64; 3]| {
            let env = f.rig.environment_intensity * night::SIDEWALK_ENV;
            luminance(f.hemi_sky) * (0.5 * n[1] + 0.5) + luminance(f.hemi_ground) * (0.5 - 0.5 * n[1]) + luminance(f.ambient) + luminance(eo_look::sh_eval(&f.env_sh, n)) * env
        };
        let up = [0.0, 1.0, 0.0];
        assert!((fill(&c12, up) / fill(&d12, up) - 1.0).abs() < 0.06, "noon ground x{}", fill(&c12, up) / fill(&d12, up));
        let walls = |f: &LookFrame| eo_look::golden::FILL_NORMALS[1..].iter().map(|n| fill(f, *n)).sum::<f64>();
        assert!((walls(&c12) / walls(&d12) - 1.0).abs() < 0.06, "noon walls x{}", walls(&c12) / walls(&d12));
        // golden hour: a wall toward the sun warmer and brighter than one away from it, which is bluer
        let s = c185.model.sun;
        let h = s.x.hypot(s.z);
        let (sun, anti) = ([s.x / h, 0.0, s.z / h], [-s.x / h, 0.0, -s.z / h]);
        let e = |f: &LookFrame, n: [f64; 3]| eo_look::sh_eval(&f.env_sh, n);
        assert!(luminance(e(&c185, sun)) > 4.0 * luminance(e(&c185, anti)));
        assert!(rb(e(&c185, sun)) > 1.1 && rb(e(&c185, anti)) < 0.9, "sunward {:?} away {:?}", e(&c185, sun), e(&c185, anti));
        // the rim cut back by day
        assert!((luminance(c185.rim) / luminance(d185.rim) - eo_look::golden::RIM_KEEP).abs() < 1e-9);
    }

    #[test]
    fn the_unlit_grounds_fill_the_meter_reads_is_kept() {
        // (the camera meter and the night solve read `night::ground_fill`: the directional fill keeps it)
        for hour in [8.0, 12.0, 16.0, 18.5, 18.9] {
            let (d, _) = look_frame(&clear(hour), Some(0.7));
            let (c, _) = look_frame_with(&clear(hour), Some(0.7), CINE);
            // (plus the rim's removed share on an upward face, which the shader's ground had from the rim)
            let day = (1.0 - night::nightness(d.model.sun.altitude_deg)) * smoothstep(-1.0, 3.0, d.model.sun.altitude_deg);
            let moved = luminance(d.rim) * (1.0 - eo_look::golden::RIM_KEEP) * day * d.rim_dir[1].max(0.0);
            let (a, b) = (night::ground_fill(&d, night::SIDEWALK_ENV) + moved, night::ground_fill(&c, night::SIDEWALK_ENV));
            assert!((b / a - 1.0).abs() < 1e-9, "{hour}: {b} vs {a}");
        }
    }

    #[test]
    fn the_night_is_left_to_the_night() {
        for hour in [21.0, 22.0, 3.0] {
            let (d, td) = look_frame(&clear(hour), None);
            let (c, tc) = look_frame_with(&clear(hour), None, CINE);
            assert_eq!((d.key, d.rim, d.hemi_sky, d.hemi_ground, d.ambient, d.env_sh), (c.key, c.rim, c.hemi_sky, c.hemi_ground, c.ambient, c.env_sh), "{hour}");
            assert_eq!(td.exposure_target, tc.exposure_target, "{hour}");
        }
    }
}
