//! Night and exposure: the practical clock (environment-ibl.js
//! `nightPracticalProfile`), the night exposure solve (sky-atmosphere.js:
//! unlit ground of albedo 0.18 at display 16/255), the weather's effect on
//! the day exposure curve (`EXPOSURE_CURVE` over `sceneIlluminance` with the
//! weather's dome brightness and beam), the exposure's adaptation filter
//! (renderer.js: one pole, 0.8 s brightening, 0.3 s darkening) and the
//! photometric scale street lamps are drawn at.

use crate::photometry::NIGHT_UNLIT_GROUND_LUX;
use crate::weather::WeatherState;
use eo_look::math::{luminance, smoothstep};
use eo_look::{LookFrame, sh_eval, tone};
use std::sync::OnceLock;

/// `lampsOn = 1 - smoothstep(-3, 8, sun altitude)`.
pub fn lamps_on(sun_alt_deg: f64) -> f64 {
    1.0 - smoothstep(-3.0, 8.0, sun_alt_deg)
}

// ---- the lamp photocell (`--no-lamp-photocell` keeps `lamps_on`) ----
//
// Street lamps switch on a photocontrol that reads the horizontal
// illuminance, not the sun's altitude: ANSI C136.10 practice (as utility
// specifications cite it, e.g. Riverside Public Utilities spec. 1-6.3) turns
// a lamp on at 1.5 fc (16 lx) +- 0.5 fc and off at no more than 1.5 x its
// turn-on level. Across a city's lamps the calibration spreads over that
// tolerance, so the share of lamps lit is the share of photocells whose
// level the light has crossed: on through the evening as the light falls
// past 21.5 .. 10.8 lx, off through the morning as it rises past 16 .. 32 lx.
// The clear sky's light through twilight falls log-linearly from sunset to
// the end of civil twilight (AMS Glossary of Meteorology, "civil twilight":
// ~585-410 lx at sunset to ~3.5-2 lx at the sun's centre 6 deg under the
// horizon; the geometric means are taken), scaled by the weather's light.

pub const FOOTCANDLE_LX: f64 = 10.7639;
/// ANSI C136.10 turn-on: 1.5 fc +- 0.5 fc.
pub const PHOTOCELL_ON_FC: [f64; 2] = [1.0, 2.0];
/// ANSI C136.10: turn-off at most 1.5 x turn-on.
pub const PHOTOCELL_OFF_OVER_ON: f64 = 1.5;
/// Horizontal illuminance of a clear sky at sunset and at the end of civil twilight (lx).
pub const SUNSET_LUX: f64 = 490.0;
pub const CIVIL_TWILIGHT_END_LUX: f64 = 2.65;

/// The clear sky's horizontal illuminance (lx) about the horizon, log-linear in
/// the sun's altitude through the two AMS anchors (and on either side of them).
pub fn twilight_lux(sun_alt_deg: f64) -> f64 {
    SUNSET_LUX * (CIVIL_TWILIGHT_END_LUX / SUNSET_LUX).powf(-sun_alt_deg / 6.0)
}

/// The share of street lamps lit for a horizontal illuminance (lx): evening
/// (the light falling, the turn-on levels) or morning (rising, the turn-off levels).
pub fn photocell_share(lux: f64, morning: bool) -> f64 {
    let k = if morning { PHOTOCELL_OFF_OVER_ON } else { 1.0 };
    let (lo, hi) = (PHOTOCELL_ON_FC[0] * FOOTCANDLE_LX * k, PHOTOCELL_ON_FC[1] * FOOTCANDLE_LX * k);
    ((hi - lux) / (hi - lo)).clamp(0.0, 1.0)
}

/// The photocell's lamp share for a frame: the twilight light at the sun's
/// altitude under the weather's light (`weather_light_ratio`), the morning
/// by the sun's hour angle (before solar noon).
pub fn lamps_on_photocell(f: &LookFrame, w: &WeatherState) -> f64 {
    let sun = &f.model.sun;
    let lux = twilight_lux(sun.altitude_deg) * weather_light_ratio(f, w);
    photocell_share(lux, sun.hour_angle_deg < 0.0)
}

/// `dusk = 1 - smoothstep(2, 16, sun altitude)` (windows and interiors).
pub fn dusk(sun_alt_deg: f64) -> f64 {
    1.0 - smoothstep(2.0, 16.0, sun_alt_deg)
}

/// `nightness = 1 - smoothstep(-6, 2, sun altitude)` (the night exposure's blend).
pub fn nightness(sun_alt_deg: f64) -> f64 {
    1.0 - smoothstep(-6.0, 2.0, sun_alt_deg)
}

/// Vehicle lamps (vehicle-presentation.js): night factor 1 at -3 deg to 0 at
/// +6 deg; lamps lit above 0.35, or by the weather floors (drizzle 0.55,
/// rain 0.7, storm 0.8, fog 0.7).
pub fn vehicle_lamps(sun_alt_deg: f64, w: &WeatherState) -> f64 {
    let night = 1.0 - smoothstep(-3.0, 6.0, sun_alt_deg);
    let floor = match w.kind {
        crate::weather::Kind::Drizzle => 0.55,
        crate::weather::Kind::Rain => 0.7,
        crate::weather::Kind::Storm => 0.8,
        crate::weather::Kind::Fog => 0.7,
        _ => 0.0,
    };
    let f = night.max(floor);
    if f > 0.35 { f } else { 0.0 }
}

/// The ground's unlit fill irradiance (units) on an upward surface: the
/// hemisphere's sky lobe, the ambient, the environment's diffuse at the
/// sidewalk class, and the key on the horizontal.
pub fn ground_fill(f: &LookFrame, env_class: f64) -> f64 {
    let sh = sh_eval(&f.env_sh, [0.0, 1.0, 0.0]);
    let env = f.rig.environment_intensity * env_class;
    let fill = [0, 1, 2].map(|i| f.hemi_sky[i] + f.ambient[i] + sh[i] * env);
    luminance(fill)
}

pub fn key_on_ground(f: &LookFrame) -> f64 {
    luminance(f.key) * f.key_dir[1].max(0.0)
}

/// The sidewalk's envMapIntensity class (street-surface-v2 `sidewalk` 0.55).
pub const SIDEWALK_ENV: f64 = 0.55;

/// Units of irradiance per lux for street lamps: the full-night unlit
/// ground fill (the darkest clear hour's rig) stands for
/// `NIGHT_UNLIT_GROUND_LUX`. Constant over the day, so lamps read weaker
/// against the brighter fill of dusk, as they do.
pub fn lamp_units_per_lux() -> f64 {
    static S: OnceLock<f64> = OnceLock::new();
    *S.get_or_init(|| {
        let f = LookFrame::new(1.0);
        ground_fill(&f, SIDEWALK_ENV) / NIGHT_UNLIT_GROUND_LUX
    })
}

/// sRGB display (0..1) of a grey of linear radiance `v` at `exposure`, the
/// delivered transform (eo-look `tone::display`).
pub fn display_grey(v: f64, exposure: f64) -> f64 {
    // the display transform the frame is drawn with (`tone::set_filmic`)
    tone::display_active([v, v, v], exposure, [0.5, 0.5])[1]
}

/// the display (0-255) of unlit ground of albedo 0.18 at full night: 8,
/// not the browser game's 16 (a departure the owner approved on 2026-11-17: "Yes,
/// darker nights"). Unlit ground under a city sky is lit by that sky and a little
/// spill, so its radiance is about albedo x the sky's (0.18 x): with the night sky
/// over the city drawn at ~26/255 (linear ~0.010), the ground belongs near linear
/// ~0.002, 8/255. At 16 the unlit ground and walls showed 3-6 x the sky above
/// them, a day-for-night look; the lamp pools and windows now carry the scene.
pub const NIGHT_UNLIT_DISPLAY_255: f64 = 8.0;

/// The night solve: the exposure in [0.3, 5.2] that shows unlit ground of
/// albedo 0.18 at display `NIGHT_UNLIT_DISPLAY_255` (sky-atmosphere.js solves for 16, 32 bisections). This
/// port solves against the delivered display transform (three's ACES with
/// the /0.6 and the grade), where the JS solves on the Narkowicz fit without
/// it: the card lands at 16 in the frame actually shown.
pub fn night_exposure(f: &LookFrame, extra_ground_units: f64) -> f64 {
    let e = ground_fill(f, SIDEWALK_ENV) + key_on_ground(f) + extra_ground_units;
    let radiance = 0.18 * e / std::f64::consts::PI;
    let target = NIGHT_UNLIT_DISPLAY_255 / 255.0;
    let (mut lo, mut hi) = (0.3, 5.2); // from 0.3 (0.9)
    for _ in 0..32 {
        let mid = 0.5 * (lo + hi);
        if display_grey(radiance, mid) < target { lo = mid } else { hi = mid }
    }
    0.5 * (lo + hi)
}

fn reference_illuminance() -> f64 {
    static R: OnceLock<f64> = OnceLock::new();
    *R.get_or_init(|| {
        let m = eo_look::sky::SkyModel::new(eo_look::rig::EXPOSURE_REFERENCE_HOUR);
        eo_look::rig::scene_illuminance(&m, eo_look::rig::rig_at(eo_look::rig::EXPOSURE_REFERENCE_HOUR).light_rig_sun_scale)
    })
}

/// The mean direct-beam factor of a weather's deck (environment-ibl.js
/// `directBeamCloudTransmittance`: open = 1 - o + o T; a continuous deck
/// keeps holes = (1 - o) clamp((0.80 - o) / 0.35) and lets T through the rest).
pub fn mean_beam(w: &WeatherState) -> f64 {
    let p = &w.profile;
    let o = p.overcast;
    let open = 1.0 - o + o * p.beam;
    let holes = (1.0 - o) * ((0.80 - o) / 0.35).clamp(0.0, 1.0);
    let cont = holes + (1.0 - holes) * p.beam;
    open + (cont - open) * p.continuous
}

/// How much of a weather's darkening the exposure makes up: the JS curve
/// makes up 0.2 of the time of day's (EXPOSURE_ADAPTATION, an evening stays
/// darker); a light meter makes up all of it (ISO 2720). 0.6: an overcast
/// noon shows an 18 % card at about middle grey, a rainy one about 1.2 EV
/// under it (darker than clear, as the JS profiles intend, but not the 2+ EV
/// that 0.2 leaves at a 10 klx rainy noon). A documented departure from the
/// JS, which applies its 0.2 to both.
pub const WEATHER_COMPENSATION: f64 = 0.6;

/// The day exposure (`recommendedExposure`) under a weather: the JS curve
/// for the hour's clear sky, x (clear / weather illuminance)^0.5. The
/// weather's illuminance: its sky model's sky term (dome brightness
/// included) and the sun x the deck's mean beam.
/// The weather's illuminance over the clear sky's at the frame's hour (the JS's
/// `sceneIlluminance` with the deck's dome brightness and mean beam): 1 when clear.
pub fn weather_light_ratio(f: &LookFrame, w: &WeatherState) -> f64 {
    let m = &f.model;
    let total = eo_look::rig::scene_illuminance(m, f.rig.light_rig_sun_scale);
    let sky = luminance(m.sky_irradiance());
    let e = sky + (total - sky).max(0.0) * mean_beam(w);
    let clear = eo_look::sky::SkyModel::new(f.hour);
    let e_clear = eo_look::rig::scene_illuminance(&clear, f.rig.light_rig_sun_scale);
    (e / e_clear.max(1e-9)).clamp(1e-3, 1.0)
}

pub fn day_exposure(f: &LookFrame, w: &WeatherState) -> f64 {
    let m = &f.model;
    let total = eo_look::rig::scene_illuminance(m, f.rig.light_rig_sun_scale);
    let sky = luminance(m.sky_irradiance());
    let sun = (total - sky).max(0.0);
    let e = sky + sun * mean_beam(w);
    let clear = eo_look::sky::SkyModel::new(f.hour);
    let e_clear = eo_look::rig::scene_illuminance(&clear, f.rig.light_rig_sun_scale);
    let raw = eo_look::rig::EXPOSURE_ANCHOR * (reference_illuminance() / e_clear.max(1e-9)).powf(eo_look::rig::EXPOSURE_ADAPTATION);
    let base = raw.clamp(eo_look::rig::EXPOSURE_MIN, eo_look::rig::EXPOSURE_MAX);
    base * (e_clear / e.max(1e-9)).max(1.0).powf(WEATHER_COMPENSATION)
}

/// The exposure target: the day curve blended into the night solve by
/// nightness (sky-atmosphere.js `mix(dayCurve, solved, nightness)`).
/// the share of the night solve in the exposure: 1 - smoothstep(-9, -3, sun
/// altitude), not the JS `nightness` (1 - smoothstep(-6, 2)): the exposure follows the day
/// curve into civil twilight (a camera does not open up until the sky has dimmed), so the
/// blue hour comes out darker than golden hour (it was ~1 stop brighter: 2.5 against 1.3)
/// with the twilight sky the brightest area. Owner-approved darker dusk, 2026-11-17.
pub fn exposure_nightness(sun_alt_deg: f64) -> f64 {
    1.0 - smoothstep(-9.0, -3.0, sun_alt_deg)
}

pub fn target_exposure(f: &LookFrame, w: &WeatherState) -> f64 {
    let n = exposure_nightness(f.model.sun.altitude_deg);
    let day = day_exposure(f, w);
    if n <= 0.0 {
        return day;
    }
    let solved = night_exposure(f, 0.0);
    day + (solved - day) * n
}

/// renderer.js exposure adaptation: a one-pole filter, tau 0.8 s while the
/// exposure rises (the scene got darker), 0.3 s while it falls; snaps on a
/// clock jump of a quarter hour or more.
#[derive(Clone, Copy, Debug, Default)]
pub struct Adaptation {
    pub current: Option<f64>,
    pub last_hour: Option<f64>,
}

pub const ADAPT_TAU_RISE_S: f64 = 0.8;
pub const ADAPT_TAU_FALL_S: f64 = 0.3;

impl Adaptation {
    pub fn step(&mut self, target: f64, hour: f64, dt: f64) -> f64 {
        let jumped = self.last_hour.is_none_or(|h| (h - hour).abs() >= 0.25);
        self.last_hour = Some(hour);
        let cur = match self.current {
            Some(c) if !jumped => {
                let tau = if target > c { ADAPT_TAU_RISE_S } else { ADAPT_TAU_FALL_S };
                let next = c + (target - c) * (1.0 - (-dt.max(0.0) / tau).exp());
                if (next - target).abs() < 1e-4 { target } else { next }
            }
            _ => target,
        };
        self.current = Some(cur);
        cur
    }
}

/// the camera meter. The exposure curve above is a function of the hour
/// and the weather only, so a view into a low sun (or toward the noon sun)
/// keeps the exposure made for the ground and most of the frame clips to
/// white. A reflected-light meter (ISO 2720, K = 12.5) holds a frame's mean
/// luminance near middle grey; a camera pointed at a low sun therefore comes
/// down by one to two stops, keeps the sky's colour and lets the ground go
/// dark (the disc and its aureole still clip). Here the frame's mean is
/// estimated from the drawn dome along the view's sample directions (each
/// held to `METER_CEILING` cards, so a direction near the sun counts as a
/// clipped highlight, not as the sun), the ground below the horizon counting
/// as a sunlit 18 % card; the exposure only ever comes down, by
/// (`METER_KEY` card / mean)^`METER_GAIN`, at most to `METER_FLOOR` (1.5
/// EV), and not at night (the night solve meters the unlit street). `dirs`:
/// (direction, weight). Pure.
pub const METER_KEY: f64 = 2.0;
pub const METER_GAIN: f64 = 0.75;
pub const METER_FLOOR: f64 = 0.35;
pub const METER_CEILING: f64 = 40.0;

pub fn meter(f: &LookFrame, w: &WeatherState, dirs: &[([f64; 3], f64)]) -> f64 {
    let card = 0.18 * (key_on_ground(f) * mean_beam(w) + ground_fill(f, SIDEWALK_ENV)) / std::f64::consts::PI;
    meter_with(f, dirs, card, &|d| luminance(f.model.dome(d)))
}

/// `meter` for a card of luminance `card` (look units) and the sky's luminance
/// along a direction as the frame draws it (`sky`): the physical daylight's
/// sky and card (eo-look `phys`) when it lights the frame.
pub fn meter_with(f: &LookFrame, dirs: &[([f64; 3], f64)], card: f64, sky: &dyn Fn([f64; 3]) -> f64) -> f64 {
    let n = nightness(f.model.sun.altitude_deg);
    if n >= 1.0 || dirs.is_empty() {
        return 1.0;
    }
    if card <= 0.0 {
        return 1.0;
    }
    let (mut sum, mut wsum) = (0.0, 0.0);
    for (d, wt) in dirs {
        let v = if d[1] > 0.0 { sky(*d).min(card * METER_CEILING) } else { card };
        sum += v * wt;
        wsum += wt;
    }
    let mean = sum / wsum.max(1e-9);
    let c = (METER_KEY * card / mean.max(1e-9)).powf(METER_GAIN).clamp(METER_FLOOR, 1.0);
    1.0 + (c - 1.0) * (1.0 - n)
}

/// The illuminance on a horizontal surface (look units, luminance) the frame
/// lights it with: the key x the weather's mean beam, the hemisphere's sky
/// lobe, the ambient and the diffuse environment, the environment at the
/// sidewalk class where the rig lights the frame and at its own intensity
/// where the physical daylight does (`phys_weight`, eo-look `phys`).
pub fn horizontal_illuminance(f: &LookFrame, w: &WeatherState, phys_weight: f64) -> f64 {
    let sh = sh_eval(&f.env_sh, [0.0, 1.0, 0.0]);
    let env = f.rig.environment_intensity * SIDEWALK_ENV;
    let env = env + (1.0 - env) * phys_weight.clamp(0.0, 1.0);
    let fill = [0, 1, 2].map(|i| f.hemi_sky[i] + f.ambient[i] + sh[i] * env);
    key_on_ground(f) * mean_beam(w) + luminance(fill)
}

/// How far over the incident meter's middle grey the day is rendered (EV):
/// an incident reading (ISO 2720) renders a card of reflectance pi K / C =
/// 0.157 at middle grey, where an sRGB camera's picture of a sunlit scene
/// (evaluative metering, its rendering curve) puts a sunlit 18 % card near L*
/// 60 (sRGB ~150), about 0.85 EV over it; the same offset keeps the frame's
/// day brightness near where the browser game's exposure curve had it at noon.
pub const EV100_COMPENSATION: f64 = 0.85;

/// `EV100_COMPENSATION` under the camera look (eo-look `tone::camera`,
/// `--no-camera-look` keeps 0.85): 0. The camera look carries the ISO 12232
/// SOS placement itself (the meter's grey at scene 0.18 and the SOS point at
/// 0.2215 shown at sRGB 118), so an incident reading renders a sunlit 18 %
/// card at sRGB ~114, as a correctly rated stills camera does; any further
/// offset is the scene meter's (a frame of dark water and deep sky opens up,
/// one of sand and sunlit cloud closes down), not a fixed bias.
pub const CAMERA_EV100_COMPENSATION: f64 = 0.0;

/// The incident offset in force (the camera look's or the filmic transform's).
pub fn ev100_compensation() -> f64 {
    if tone::camera::camera_look_on() { CAMERA_EV100_COMPENSATION } else { EV100_COMPENSATION }
}

/// The day exposure an incident-light meter gives (`--no-ev100`: the JS curve,
/// `day_exposure`): EV100 = log2(E S / C) of the horizontal illuminance `e_h`
/// (look units, lux through eo-look `phys::lux_per_unit`), the metered middle
/// grey at scene 0.18 + `EV100_COMPENSATION`. Returns (EV100, exposure).
pub fn ev100_day_exposure(e_h: f64) -> (f64, f64) {
    let ev = eo_look::phys::ev100_incident(e_h * eo_look::phys::lux_per_unit());
    (ev, eo_look::phys::exposure_at_ev100(ev, ev100_compensation()))
}

/// `target_exposure` with the day term from the incident meter, by `weight`
/// (eo-look `phys::weight`: the sun up), blended in log exposure into the JS
/// curve as the sun sets; the night solve takes over as before.
pub fn target_exposure_ev100(f: &LookFrame, w: &WeatherState, e_h: f64, weight: f64) -> f64 {
    let n = exposure_nightness(f.model.sun.altitude_deg);
    let js = day_exposure(f, w);
    // the meter reads the hour's clear-sky light; a weather's darkening is made up
    // `WEATHER_COMPENSATION` of the way, as `day_exposure` does (a rainy noon stays darker
    // than a clear one): the frame's illuminance over the weather's share of the clear's.
    let k = weather_light_ratio(f, w);
    let day_ev = ev100_day_exposure(e_h / k).1 * (1.0 / k).max(1.0).powf(WEATHER_COMPENSATION);
    let day = eo_look::math::geo_mix(js, day_ev, weight.clamp(0.0, 1.0));
    if n <= 0.0 {
        return day;
    }
    let solved = night_exposure(f, 0.0);
    day + (solved - day) * n
}

// ---- the scene meter (`--no-scene-meter` keeps `meter` above) ----
//
// A reflected-light meter on the drawn frame (ISO 2720:1974, K = 12.5): the
// GPU (eo-render look_meter.wgsl) bins the HDR frame's luminance, sampled at
// a quarter of its resolution, by stops about the incident meter's grey (the
// prior: `target_exposure_ev100`'s scene 0.18), each sample weighted by a
// centre-weighted averaging pattern. The exposure then puts the frame's
// weighted mean luminance at the meter's grey (an averaging reflected meter
// renders the mean at middle grey), within `SCENE_METER_RANGE_EV` of the
// incident prior, and comes down further only to keep the top
// `SCENE_METER_HIGHLIGHT_SHARE` of the frame under the display's shoulder
// (a camera's highlight-weighted evaluative metering). By night the night
// solve keeps the exposure (blended out by `nightness`, as `meter` is).

/// Histogram bins, their width (1/8 stop) and the lowest bin's lower edge,
/// in stops about the prior grey. Samples above the top bin (+5 stops, 32
/// cards: the sun's disc and its glints, as `METER_CEILING` holds them) count
/// in the top bin, as a clipped highlight and not as the sun.
pub const SCENE_METER_BINS: usize = 128;
pub const SCENE_METER_BINS_PER_EV: f64 = 8.0;
pub const SCENE_METER_LO_EV: f64 = -11.0;

/// The reflected meter may move the exposure this far from the incident
/// prior (stops). A scene of mean reflectance rho reads log2(0.157 / rho)
/// stops off an incident meter (ISO 2720: pi K / C = 0.157); the broadband
/// albedos of sunlit natural surfaces run from open water at a high sun
/// (0.03-0.10) to old and fresh snow (0.40-0.95) (Oke, "Boundary Layer
/// Climates", 2nd ed., 1987, table 1.1), so a frame's mean of ~0.04-0.6
/// reads about +2 to -2 stops off the incident meter.
pub const SCENE_METER_RANGE_EV: f64 = 2.0;

/// The share of the weighted frame held under the shoulder (the top 2 %:
/// lit cloud, foam, sunlit white walls; the sun's disc and glints above it
/// are let clip).
pub const SCENE_METER_HIGHLIGHT_SHARE: f64 = 0.02;

/// One reading: the weighted histogram and the prior grey it was binned about.
#[derive(Clone, Debug)]
pub struct SceneMeterReading {
    pub hist: Vec<u32>,
    /// the prior exposure the histogram's stops are about (grey = 0.18 x 0.6 / prior).
    pub prior: f64,
    /// the renderer's frame the histogram was taken from.
    pub frame: u32,
}

/// The display's shoulder as a scene value at the meter's convention (0.18 =
/// the metered grey): where the neutral tone curve's log-log slope falls to
/// half of its slope at middle grey (the display transform in force).
pub fn display_shoulder() -> f64 {
    let curve = |x: f64| luminance(tone::display_active([x, x, x], 0.6, [0.5, 0.5]).map(eo_look::math::srgb_to_linear)).max(1e-9);
    let slope = |x: f64| (curve(x * 1.05).ln() - curve(x / 1.05).ln()) / (2.0 * 1.05f64.ln());
    let s0 = slope(0.18);
    let mut x = 0.18;
    while x < 1e3 && slope(x) > 0.5 * s0 {
        x *= 1.02;
    }
    x
}

/// The exposure factor over the prior a reading asks for (1 with nothing
/// binned), before the night blend. `shoulder`: `display_shoulder()`.
///
/// With the colour track's metering (eo-look `features::colour().meter_highlights`,
/// `--no-meter-highlights` keeps the hold) the meter is an averaging
/// reflected-light meter alone: the frame's mean luminance at the meter's
/// grey, within `SCENE_METER_RANGE_EV` of the incident prior. ISO 2720
/// supplies the calibration constant (K = 12.5), not the averaging: a
/// whole-frame average is one meter type among those the standard covers,
/// chosen here because it has no free parameter. The highlight hold
/// (`SCENE_METER_HIGHLIGHT_SHARE` under the shoulder) had no published basis.
/// Measured on the acceptance views, the hold never bound (the top 2 % sat
/// about one stop over grey by day, three stops at Baker's sunset, all under
/// the shoulder), so removing it changes no capture beyond the noise floor.
pub fn scene_meter_factor(hist: &[u32], shoulder: f64) -> f64 {
    scene_meter_factor_with(hist, shoulder, !eo_look::features::colour().meter_highlights)
}

/// Whether the scene meter's pass should average the whole frame rather
/// than weight its centre (the meter uniform's `grey.y`, look_meter.wgsl):
/// the centre-weighted pattern (4x at the centre, a Gaussian of 0.25 frame
/// heights) had no published basis. Nothing reads this yet: the uniform is
/// written by eo-render look/scene_meter.rs (another track's file; the
/// hand-off writes `grey.y` from it), so the centre weighting is still live
/// whatever `--no-meter-highlights` says.
pub fn meter_averaging() -> bool {
    eo_look::features::colour().meter_highlights
}

/// `scene_meter_factor` with the highlight hold on or off.
pub fn scene_meter_factor_with(hist: &[u32], shoulder: f64, highlight_hold: bool) -> f64 {
    let total: f64 = hist.iter().map(|&w| w as f64).sum();
    if total <= 0.0 {
        return 1.0;
    }
    let centre = |b: usize| SCENE_METER_LO_EV + (b as f64 + 0.5) / SCENE_METER_BINS_PER_EV;
    // the weighted mean luminance over the prior grey
    let mean: f64 = hist.iter().enumerate().map(|(b, &w)| w as f64 * centre(b).exp2()).sum::<f64>() / total;
    let range = SCENE_METER_RANGE_EV.exp2();
    let mut k = (1.0 / mean.max(1e-9)).clamp(1.0 / range, range);
    if !highlight_hold {
        return k;
    }
    // the highlight hold: the level the top share starts at, under the shoulder
    let mut acc = 0.0;
    let mut top = centre(hist.len() - 1);
    for b in (0..hist.len()).rev() {
        acc += hist[b] as f64;
        if acc >= SCENE_METER_HIGHLIGHT_SHARE * total {
            top = centre(b);
            break;
        }
    }
    let cap = shoulder / (0.18 * top.exp2());
    if k > cap {
        k = cap.max(1.0 / range);
    }
    k
}

/// `scene_meter_factor` blended out by night (the night solve keeps the exposure).
pub fn scene_meter(sun_alt_deg: f64, hist: &[u32], shoulder: f64) -> f64 {
    let n = nightness(sun_alt_deg);
    if n >= 1.0 {
        return 1.0;
    }
    scene_meter_factor(hist, shoulder).powf(1.0 - n)
}

#[cfg(test)]
mod scene_meter_tests {
    use super::*;

    fn hist_of(samples: &[(f64, u32)]) -> Vec<u32> {
        let mut h = vec![0u32; SCENE_METER_BINS];
        for &(ev, w) in samples {
            let b = ((ev - SCENE_METER_LO_EV) * SCENE_METER_BINS_PER_EV).floor().clamp(0.0, (SCENE_METER_BINS - 1) as f64) as usize;
            h[b] += w;
        }
        h
    }

    #[test]
    fn a_grey_card_frame_keeps_the_prior() {
        let k = scene_meter_factor_with(&hist_of(&[(0.0, 1000)]), 2.0, true);
        assert!((k.log2()).abs() < 0.1, "{k}");
    }

    #[test]
    fn dark_water_opens_up_and_sand_closes_down_within_the_range() {
        // a frame of open water, 2.5 stops under the card: the meter opens up, to the bound
        let k = scene_meter_factor_with(&hist_of(&[(-2.5, 1000)]), 100.0, true);
        assert!((k.log2() - SCENE_METER_RANGE_EV).abs() < 0.05, "{k}");
        // a frame of bright sand 1.5 stops over the card: down by about 1.5
        let k = scene_meter_factor_with(&hist_of(&[(1.5, 1000)]), 100.0, true);
        assert!((k.log2() + 1.5).abs() < 0.1, "{k}");
    }

    #[test]
    fn the_highlights_hold_the_exposure_under_the_shoulder() {
        // a dark frame with 5 % lit cloud 2 stops over the card: the mean asks to open up,
        // the cloud keeps it where the cloud sits at the shoulder
        let h = hist_of(&[(-2.0, 950), (2.0, 50)]);
        let shoulder = 0.18 * 4.0 * 1.5;
        let k = scene_meter_factor_with(&h, shoulder, true);
        assert!((k - 1.5).abs() < 0.1, "{k}");
    }

    #[test]
    fn the_averaging_meter_puts_the_mean_at_grey_whatever_the_highlights() {
        // the same frame: the mean (0.95 x 2^-2 + 0.05 x 2^2 = 0.4375 of the grey) asks for
        // log2(1 / 0.4375) = 1.19 stops, and the lit cloud does not hold it back
        let h = hist_of(&[(-2.0, 950), (2.0, 50)]);
        let k = scene_meter_factor_with(&h, 0.18 * 4.0 * 1.5, false);
        assert!((k.log2() - 1.19).abs() < 0.08, "{}", k.log2());
    }

    #[test]
    fn night_leaves_the_exposure_to_the_night_solve() {
        assert_eq!(scene_meter(-10.0, &hist_of(&[(-3.0, 100)]), 2.0), 1.0);
    }
}

#[cfg(test)]
mod photocell_tests {
    use super::*;

    #[test]
    fn the_twilight_light_passes_the_ams_anchors() {
        assert!((twilight_lux(0.0) - SUNSET_LUX).abs() < 1e-9);
        assert!((twilight_lux(-6.0) - CIVIL_TWILIGHT_END_LUX).abs() < 1e-9);
    }

    #[test]
    fn lamps_are_off_at_a_low_sun_and_on_by_the_end_of_civil_twilight() {
        // the sun 3 deg up (the Baker Beach photograph): no lamp lit
        assert_eq!(photocell_share(twilight_lux(3.0), false), 0.0);
        // the old ramp had 43 % lit there
        assert!(lamps_on(3.0) > 0.4);
        // all lit by -5 deg in the evening, still some lit at -3.5 deg in the morning
        assert_eq!(photocell_share(twilight_lux(-5.0), false), 1.0);
        let e = photocell_share(twilight_lux(-4.0), false);
        assert!(e > 0.2 && e < 0.9, "{e}");
        // hysteresis: at the same light more lamps are lit in the morning than in the evening
        let l = twilight_lux(-3.6);
        assert!(photocell_share(l, true) > photocell_share(l, false));
    }

    #[test]
    fn a_darker_weather_lights_the_lamps_earlier() {
        let clear = crate::weather::preset(crate::Kind::Clear);
        let rain = crate::weather::preset(crate::Kind::Rain);
        // find the hour the sun stands ~2 deg down in the evening
        let mut h = 18.0;
        while LookFrame::new(h).model.sun.altitude_deg > -2.0 {
            h += 0.01;
        }
        let f = LookFrame::new(h);
        assert!(lamps_on_photocell(&f, &rain) >= lamps_on_photocell(&f, &clear));
    }
}
