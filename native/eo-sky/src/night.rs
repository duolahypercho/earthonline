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
    tone::display([v, v, v], exposure, [0.5, 0.5])[1]
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
    let n = nightness(f.model.sun.altitude_deg);
    if n >= 1.0 || dirs.is_empty() {
        return 1.0;
    }
    let card = 0.18 * (key_on_ground(f) * mean_beam(w) + ground_fill(f, SIDEWALK_ENV)) / std::f64::consts::PI;
    if card <= 0.0 {
        return 1.0;
    }
    let (mut sum, mut wsum) = (0.0, 0.0);
    for (d, wt) in dirs {
        let v = if d[1] > 0.0 { luminance(f.model.dome(*d)).min(card * METER_CEILING) } else { card };
        sum += v * wt;
        wsum += wt;
    }
    let mean = sum / wsum.max(1e-9);
    let c = (METER_KEY * card / mean.max(1e-9)).powf(METER_GAIN).clamp(METER_FLOOR, 1.0);
    1.0 + (c - 1.0) * (1.0 - n)
}

/// The most the meter opens up for a camera inside a room (eo-render
/// `interior_meter`: a landmark deck's room), 2.3 EV. A reflected-light
/// meter pointed into a room lit through its windows opens up until the
/// room's surfaces read toward middle grey and the windows go bright (an
/// office's 500 lx against a clear sky's 20 klx of diffuse light outside is
/// over 5 EV); held to 2.3 EV (INFERRED: the windows keep some of their
/// view, as a bracketed interior with a view is exposed, instead of
/// clipping to white).
pub const METER_ROOM_MAX: f64 = 5.0;

/// `meter` for a camera inside a room: a direction whose third member is
/// `Some(k)` meets the room's own surfaces, not the sky or the open ground,
/// and counts as an 18 % card in k of the open ground's fill (k: the room's
/// mean sky visibility, eo-landmarks access_sky.rs, its sun share
/// included); a direction with None leaves through an opening and counts as
/// `meter` counts it. The exposure may also rise, up to 1 plus the room's
/// weighted share of the directions times (`METER_ROOM_MAX` - 1), so a
/// view out of a window that shows a strip of sill keeps the open view's
/// exposure; with no room direction this is `meter`. Pure.
pub fn meter_room(f: &LookFrame, w: &WeatherState, dirs: &[([f64; 3], f64, Option<f64>)]) -> f64 {
    if dirs.iter().all(|d| d.2.is_none()) {
        let open: Vec<([f64; 3], f64)> = dirs.iter().map(|d| (d.0, d.1)).collect();
        return meter(f, w, &open);
    }
    let n = nightness(f.model.sun.altitude_deg);
    if n >= 1.0 {
        return 1.0;
    }
    let card = 0.18 * (key_on_ground(f) * mean_beam(w) + ground_fill(f, SIDEWALK_ENV)) / std::f64::consts::PI;
    if card <= 0.0 {
        return 1.0;
    }
    let room_card = 0.18 * ground_fill(f, SIDEWALK_ENV) / std::f64::consts::PI;
    let (mut sum, mut wsum, mut wroom) = (0.0, 0.0, 0.0);
    for (d, wt, k) in dirs {
        let v = match k {
            Some(k) => {
                wroom += wt;
                room_card * k.clamp(0.0, 1.0)
            }
            None if d[1] > 0.0 => luminance(f.model.dome(*d)).min(card * METER_CEILING),
            None => card,
        };
        sum += v * wt;
        wsum += wt;
    }
    let mean = sum / wsum.max(1e-9);
    let ceiling = 1.0 + (METER_ROOM_MAX - 1.0) * (wroom / wsum.max(1e-9));
    let c = (METER_KEY * card / mean.max(1e-9)).powf(METER_GAIN).clamp(METER_FLOOR, ceiling);
    1.0 + (c - 1.0) * (1.0 - n)
}
