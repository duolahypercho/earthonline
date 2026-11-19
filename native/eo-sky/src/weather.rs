//! Weather as shared world state: the JS game's severity schedule and ground
//! water, ported, keyed on a world seed and the world clock so that every
//! client and the server derive the same weather without exchanging it.
//!
//! Sources (all under src/simulation/physics/weather/ unless named):
//! * `weather-schedule.js`: the severity chain clear 0 - fog 1 - drizzle 2 -
//!   heavy drizzle 3 - storm 4 (`WEATHER_RUNGS`: dwell ranges, next-rung
//!   weights), 30-75 s transitions with a smoothstep on the severity,
//!   `rateForSeverity` (0.5 mm/h drizzle, 1 mm/h heavy drizzle, 22 mm/h
//!   storm), `filmForSeverity` (fog 0.01 mm, Gallaway's exponent 0.59), the
//!   renderer bucket rule (`bucketForPhysical`), the mean wind anchors and the
//!   +-35 % gust;
//! * `weather-accumulation.js`: the ground film (rain in, 0.6 mm drain
//!   threshold with a 40 s time constant, fog condensation toward 0.01 mm over
//!   60 s, evaporation 0.25 mm per game hour of 120 s outside rain and fog),
//!   puddles (capacity 1.2 mm, 1800 s decay);
//! * src/citygen/main.js: `wetness = 1 - exp(-film / 0.08 mm)`;
//! * src/render/environment-ibl.js: the per-bucket look profiles (overcast,
//!   isotropy, brightness, beam transmission, desaturation, tint, urban glow,
//!   shadow scale), the cloud decks (`CLOUD_DECKS`), the visibility grade and
//!   the haze density; src/render/passes/lamp-haze.js: the lamp air-light gain.
//!
//! Departure from the JS (documented): the JS starts each client's schedule
//! at that client's own boot tick, so two players see different weather. Here
//! the schedule's clock is the shared world clock (`world_s`, seconds since
//! the world's epoch), which is what an online world needs; the arithmetic is
//! the JS's.

use crate::hash::hash01;

/// The renderer-facing kinds (`WEATHER_KINDS` in environment-ibl.js, without
/// snow, which the schedule never produces).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Clear,
    Fog,
    Overcast,
    Drizzle,
    Rain,
    Storm,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Clear => "clear",
            Kind::Fog => "fog",
            Kind::Overcast => "overcast",
            Kind::Drizzle => "drizzle",
            Kind::Rain => "rain",
            Kind::Storm => "storm",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "clear" => Kind::Clear,
            "fog" => Kind::Fog,
            "overcast" => Kind::Overcast,
            "drizzle" => Kind::Drizzle,
            "rain" => Kind::Rain,
            "storm" => Kind::Storm,
            _ => return None,
        })
    }
}

/// One bucket's look (environment-ibl.js `WEATHER_PROFILES`, `CLOUD_DECKS`,
/// the visibility grade, `HAZE_DENSITY`, lamp-haze.js `weatherGain`), plus
/// this port's marine-layer density (see `marine_fog`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Profile {
    pub overcast: f64,
    pub isotropy: f64,
    /// Dome energy relative to clear.
    pub brightness: f64,
    /// Direct-beam transmission through the deck where it covers the sun.
    pub beam: f64,
    /// 1 for the continuous decks (drizzle, rain, storm).
    pub continuous: f64,
    pub desaturation: f64,
    pub tint: [f64; 3],
    pub urban_glow: f64,
    pub shadow_scale: f64,
    /// Cloud decks: coverage of the sky, opacity of a covered low / high pixel.
    pub deck_coverage: f64,
    pub deck_low: f64,
    pub deck_high: f64,
    /// Visibility grade: scale on the fog's near and far distances.
    pub vis_near: f64,
    pub vis_far: f64,
    pub haze: f64,
    pub lamp_haze: f64,
    /// Marine-layer extinction inside the bank (1/m), 0 when there is none.
    pub marine: f64,
    /// The bucket's air (`weatherProfile`): turbidity, Mie coefficient, Mie g.
    pub atmos: [f64; 3],
}

/// The JS per-bucket air (environment-ibl.js): clear 2.4 / .005 / .8, fog
/// 9.5 / .021 / .72, overcast 4.0 / .009 / .74, drizzle 6.0 / .013 / .78, rain
/// 7.5 / .016 / .76, storm 9.0 / .02 / .7.
pub fn atmos(kind: Kind) -> [f64; 3] {
    match kind {
        Kind::Clear => [2.4, 0.005, 0.8],
        Kind::Fog => [9.5, 0.021, 0.72],
        Kind::Overcast => [4.0, 0.009, 0.74],
        Kind::Drizzle => [6.0, 0.013, 0.78],
        Kind::Rain => [7.5, 0.016, 0.76],
        Kind::Storm => [9.0, 0.02, 0.7],
    }
}

/// The weather's atmosphere profile for eo-look's sky model: the shipped
/// look's clear air (coastal-heat turbidity 3.2, Rayleigh 1.2, Mie 0.0065)
/// moved by the bucket's departure from clear air, and the bucket's
/// overcast, isotropy, desaturation, brightness, tint and urban glow.
pub fn sky_profile(p: &Profile) -> eo_look::sky::Profile {
    let base = eo_look::sky::Profile::clear_with_look(&eo_look::look::LOOK);
    let c = atmos(Kind::Clear);
    eo_look::sky::Profile {
        turbidity: base.turbidity + (p.atmos[0] - c[0]),
        mie_coefficient: base.mie_coefficient + (p.atmos[1] - c[1]),
        mie_directional_g: base.mie_directional_g + (p.atmos[2] - c[2]),
        overcast: p.overcast,
        isotropy: p.isotropy,
        desaturation: p.desaturation,
        brightness: p.brightness,
        tint: p.tint,
        urban_glow: p.urban_glow,
        ..base
    }
}

const fn p(
    overcast: f64,
    isotropy: f64,
    brightness: f64,
    beam: f64,
    continuous: f64,
    desaturation: f64,
    tint: [f64; 3],
    urban_glow: f64,
    shadow_scale: f64,
    deck: [f64; 3],
    vis: [f64; 2],
    haze: f64,
    lamp_haze: f64,
    marine: f64,
    atmos: [f64; 3],
) -> Profile {
    Profile {
        overcast,
        isotropy,
        brightness,
        beam,
        continuous,
        desaturation,
        tint,
        urban_glow,
        shadow_scale,
        deck_coverage: deck[0],
        deck_low: deck[1],
        deck_high: deck[2],
        vis_near: vis[0],
        vis_far: vis[1],
        haze,
        lamp_haze,
        marine,
        atmos,
    }
}

/// Marine-layer extinction in fog: Koschmieder, sigma = 3.912 / V, at a
/// meteorological visibility V = 900 m (WMO: fog is visibility under 1 km;
/// the JS fog bucket's visibility grade .40/.55 puts its far fog at about
/// 900-1400 m).
pub const MARINE_FOG_SIGMA: f64 = 3.912 / 900.0;

pub const CLEAR: Profile = p(0.0, 0.0, 1.0, 1.0, 0.0, 0.0, [1.0, 1.0, 1.0], 0.6, 1.0, [0.14, 0.97, 0.04], [1.0, 1.18], 0.10, 1.0, 0.0, [2.4, 0.005, 0.8]);
pub const FOG: Profile = p(0.9, 0.7, 0.62, 0.25, 0.0, 0.7, [0.98, 1.0, 1.02], 0.85, 1.0, [0.92, 0.98, 0.35], [0.40, 0.55], 0.52, 2.0, MARINE_FOG_SIGMA, [9.5, 0.021, 0.72]);
pub const OVERCAST: Profile = p(0.92, 0.9, 0.84, 0.18, 0.0, 0.78, [0.99, 1.0, 1.015], 0.9, 1.0, [0.88, 0.2, 0.90], [1.0, 1.18], 0.20, 1.15, 0.0, [4.0, 0.009, 0.74]);
pub const DRIZZLE: Profile = p(0.85, 0.35, 0.38, 0.12, 1.0, 0.82, [0.99, 1.0, 1.02], 1.0, 1.0, [0.85, 0.97, 0.42], [0.50, 0.66], 0.34, 1.6, MARINE_FOG_SIGMA * 0.35, [6.0, 0.013, 0.78]);
pub const RAIN: Profile = p(0.95, 0.5, 0.26, 0.07, 1.0, 0.86, [0.985, 1.0, 1.02], 1.1, 0.8, [0.95, 0.98, 0.5], [0.42, 0.56], 0.40, 1.4, MARINE_FOG_SIGMA * 0.2, [7.5, 0.016, 0.76]);
pub const STORM: Profile = p(1.0, 0.65, 0.13, 0.03, 1.0, 0.8, [0.95, 1.0, 0.965], 1.2, 0.55, [0.98, 0.99, 0.6], [0.32, 0.45], 0.50, 1.25, MARINE_FOG_SIGMA * 0.15, [9.0, 0.02, 0.7]);

pub fn profile(kind: Kind) -> Profile {
    match kind {
        Kind::Clear => CLEAR,
        Kind::Fog => FOG,
        Kind::Overcast => OVERCAST,
        Kind::Drizzle => DRIZZLE,
        Kind::Rain => RAIN,
        Kind::Storm => STORM,
    }
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// Two profiles blended (environment-ibl.js `blendWeatherProfiles`: linear,
/// the beam transmission in log space).
pub fn blend(a: &Profile, b: &Profile, t: f64) -> Profile {
    let t = t.clamp(0.0, 1.0);
    let l = |x: f64, y: f64| lerp(x, y, t);
    Profile {
        overcast: l(a.overcast, b.overcast),
        isotropy: l(a.isotropy, b.isotropy),
        brightness: l(a.brightness, b.brightness),
        beam: (a.beam.max(1e-6).ln() * (1.0 - t) + b.beam.max(1e-6).ln() * t).exp(),
        continuous: l(a.continuous, b.continuous),
        desaturation: l(a.desaturation, b.desaturation),
        tint: [l(a.tint[0], b.tint[0]), l(a.tint[1], b.tint[1]), l(a.tint[2], b.tint[2])],
        urban_glow: l(a.urban_glow, b.urban_glow),
        shadow_scale: l(a.shadow_scale, b.shadow_scale),
        deck_coverage: l(a.deck_coverage, b.deck_coverage),
        deck_low: l(a.deck_low, b.deck_low),
        deck_high: l(a.deck_high, b.deck_high),
        vis_near: l(a.vis_near, b.vis_near),
        vis_far: l(a.vis_far, b.vis_far),
        haze: l(a.haze, b.haze),
        lamp_haze: l(a.lamp_haze, b.lamp_haze),
        marine: l(a.marine, b.marine),
        atmos: [l(a.atmos[0], b.atmos[0]), l(a.atmos[1], b.atmos[1]), l(a.atmos[2], b.atmos[2])],
    }
}

// ---- the schedule (weather-schedule.js) ---------------------------------

struct Rung {
    severity: f64,
    dwell: [f64; 2],
    next: &'static [(usize, f64)],
}

const RUNGS: [Rung; 5] = [
    Rung { severity: 0.0, dwell: [300.0, 960.0], next: &[(1, 1.0)] },
    Rung { severity: 1.0, dwell: [150.0, 480.0], next: &[(0, 0.7), (2, 0.3)] },
    Rung { severity: 2.0, dwell: [90.0, 300.0], next: &[(1, 0.78), (3, 0.22)] },
    Rung { severity: 3.0, dwell: [60.0, 150.0], next: &[(2, 0.85), (4, 0.15)] },
    Rung { severity: 4.0, dwell: [60.0, 180.0], next: &[(3, 1.0)] },
];
const TRANSITION_S: [f64; 2] = [30.0, 75.0];
const SALT_DWELL: u32 = 1;
const SALT_NEXT: u32 = 2;
const SALT_TRANSITION: u32 = 3;
const SALT_GUST_A: u32 = 4;
const SALT_GUST_B: u32 = 5;
const GUST_SPREAD: f64 = 0.35;
const GUST_PERIOD_A_S: f64 = 11.3;
const GUST_PERIOD_B_S: f64 = 29.7;
pub const STORM_ONSET_RATE_MM_H: f64 = 10.0;
const WET_ONSET_SEVERITY: f64 = 0.5;
const FOG_FILM_MM: f64 = 0.01;
const DRIZZLE_FILM_MM: f64 = 0.0508;
const DRIZZLE_RATE_MM_H: f64 = 0.5;
const HEAVY_DRIZZLE_RATE_MM_H: f64 = 1.0;
const STORM_RATE_MM_H: f64 = 22.0;
const FILM_RATE_EXPONENT: f64 = 0.59;
const WIND_X: [f64; 5] = [0.0, 0.5, 2.0, 4.4, 11.5];
const WIND_Z: [f64; 5] = [0.0, 0.2, 0.5, 1.1, 4.2];

fn smooth01(t: f64) -> f64 {
    let c = t.clamp(0.0, 1.0);
    c * c * (3.0 - 2.0 * c)
}

fn anchor_lerp(v: &[f64; 5], s: f64) -> f64 {
    let low = (s.floor() as usize).min(3);
    lerp(v[low], v[low + 1], s - low as f64)
}

/// `rateForSeverity`, mm/h.
pub fn rate_for_severity(s: f64) -> f64 {
    let s = s.clamp(0.0, 4.0);
    if s <= 1.0 {
        return 0.0;
    }
    if s > 3.0 {
        return HEAVY_DRIZZLE_RATE_MM_H + (s.min(4.0) - 3.0) * (STORM_RATE_MM_H - HEAVY_DRIZZLE_RATE_MM_H);
    }
    let above = s - 1.0;
    let r = if above <= 1.0 { above * DRIZZLE_RATE_MM_H } else { DRIZZLE_RATE_MM_H + (above - 1.0) * (HEAVY_DRIZZLE_RATE_MM_H - DRIZZLE_RATE_MM_H) };
    if r < 1e-6 { 0.0 } else { r }
}

/// `filmForSeverity`, mm (the schedule's authored anchor film).
pub fn film_for_severity(s: f64) -> f64 {
    if s < WET_ONSET_SEVERITY {
        return 0.0;
    }
    if s <= 1.0 {
        return FOG_FILM_MM;
    }
    let rate = rate_for_severity(s);
    FOG_FILM_MM + (DRIZZLE_FILM_MM - FOG_FILM_MM) * (rate / DRIZZLE_RATE_MM_H).powf(FILM_RATE_EXPONENT)
}

/// `bucketForPhysical`.
pub fn bucket(film_mm: f64, rate_mm_h: f64) -> Kind {
    if rate_mm_h >= STORM_ONSET_RATE_MM_H {
        Kind::Storm
    } else if rate_mm_h > HEAVY_DRIZZLE_RATE_MM_H {
        Kind::Rain
    } else if rate_mm_h > 0.0 {
        Kind::Drizzle
    } else if film_mm > 0.0 {
        Kind::Fog
    } else {
        Kind::Clear
    }
}

#[derive(Clone, Copy, Debug)]
struct Front {
    index: u32,
    rung: usize,
    next: usize,
    dwell: f64,
    transition: f64,
}

fn front_plan(seed: u32, index: u32, rung: usize) -> Front {
    let r = &RUNGS[rung];
    let dwell = lerp(r.dwell[0], r.dwell[1], hash01(seed, index, SALT_DWELL));
    let total: f64 = r.next.iter().map(|o| o.1).sum();
    let mut cursor = hash01(seed, index, SALT_NEXT) * total;
    let mut next = r.next[r.next.len() - 1].0;
    for o in r.next {
        cursor -= o.1;
        if cursor <= 0.0 {
            next = o.0;
            break;
        }
    }
    let transition = lerp(TRANSITION_S[0], TRANSITION_S[1], hash01(seed, index, SALT_TRANSITION));
    Front { index, rung, next, dwell, transition }
}

/// One sample of the schedule (`sampleAtTick` at `elapsed` seconds).
#[derive(Clone, Copy, Debug)]
pub struct ScheduleSample {
    pub severity: f64,
    pub front: u32,
    pub progress: f64,
    pub gust_factor: f64,
}

/// The schedule as a pure function of (seed, start rung, seconds since the
/// world's epoch). Fronts are walked from the epoch, as the JS walks them
/// from its base tick; a cursor caches the walk for monotonic queries.
#[derive(Clone, Debug)]
pub struct Schedule {
    pub seed: u32,
    pub start_rung: usize,
    cursor: Front,
    cursor_start: f64,
    gust_phase: [f64; 2],
}

impl Schedule {
    pub fn new(seed: u32, start_rung: usize) -> Self {
        let start_rung = start_rung.min(4);
        let tau = std::f64::consts::TAU;
        Self {
            seed,
            start_rung,
            cursor: front_plan(seed, 0, start_rung),
            cursor_start: 0.0,
            gust_phase: [hash01(seed, 0, SALT_GUST_A) * tau, hash01(seed, 0, SALT_GUST_B) * tau],
        }
    }

    pub fn sample(&mut self, elapsed: f64) -> ScheduleSample {
        let t = elapsed.max(0.0);
        if t < self.cursor_start {
            self.cursor = front_plan(self.seed, 0, self.start_rung);
            self.cursor_start = 0.0;
        }
        let mut guard = 0;
        while t >= self.cursor_start + self.cursor.dwell + self.cursor.transition && guard < 10_000_000 {
            self.cursor_start += self.cursor.dwell + self.cursor.transition;
            self.cursor = front_plan(self.seed, self.cursor.index + 1, self.cursor.next);
            guard += 1;
        }
        let into = t - self.cursor_start;
        let from = RUNGS[self.cursor.rung].severity;
        let to = RUNGS[self.cursor.next].severity;
        let settled = into < self.cursor.dwell;
        let progress = if settled { 0.0 } else { ((into - self.cursor.dwell) / self.cursor.transition).min(1.0) };
        let severity = if settled { from } else { lerp(from, to, smooth01(progress)) };
        let tau = std::f64::consts::TAU;
        let a = (t / GUST_PERIOD_A_S * tau + self.gust_phase[0]).sin();
        let b = (t / GUST_PERIOD_B_S * tau + self.gust_phase[1]).sin();
        ScheduleSample { severity, front: self.cursor.index, progress, gust_factor: 1.0 + GUST_SPREAD / 2.0 * (a + b) }
    }
}

// ---- ground water (weather-accumulation.js) ------------------------------

/// One game hour in real seconds (the accumulator's clock).
const GAME_HOUR_S: f64 = 120.0;
const DRAIN_THRESHOLD_MM: f64 = 0.6;
const DRAIN_TAU_S: f64 = 40.0;
const CONDENSE_TAU_S: f64 = 60.0;
const EVAPORATION_MM_PER_GAME_HOUR: f64 = 0.25;
const PUDDLE_CAPACITY_MM: f64 = 1.2;
const PUDDLE_TAU_S: f64 = 1800.0;
/// `wetness = 1 - exp(-film / 0.08)` (src/citygen/main.js).
const WETNESS_FILM_MM: f64 = 0.08;

fn saturation_vapour_hpa(k: f64) -> f64 {
    let c = k - 273.15;
    6.1094 * (17.625 * c / (c + 243.04)).exp()
}

const AIR_K: [f64; 5] = [288.15, 285.15, 284.15, 283.15, 283.65];

/// Ground water state.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Ground {
    pub film_mm: f64,
    pub puddle_mm: f64,
}

impl Ground {
    /// One step of `dt` seconds at a severity.
    pub fn step(&mut self, severity: f64, wind: f64, dt: f64) {
        let rate = rate_for_severity(severity);
        let fog = severity >= WET_ONSET_SEVERITY && rate <= 0.0;
        self.film_mm += rate / 3600.0 * dt;
        if fog && self.film_mm < FOG_FILM_MM {
            self.film_mm += (FOG_FILM_MM - self.film_mm) * (1.0 - (-dt / CONDENSE_TAU_S).exp());
        }
        let excess = (self.film_mm - DRAIN_THRESHOLD_MM).max(0.0);
        let drained = excess * (1.0 - (-dt / DRAIN_TAU_S).exp());
        self.film_mm -= drained;
        let fill = (self.puddle_mm / PUDDLE_CAPACITY_MM).clamp(0.0, 1.0);
        self.puddle_mm += drained * (1.0 - fill);
        self.puddle_mm *= (-dt / PUDDLE_TAU_S).exp();
        if rate <= 0.0 && !fog {
            let es = saturation_vapour_hpa(anchor_lerp(&AIR_K, severity.clamp(0.0, 4.0))) / saturation_vapour_hpa(288.15);
            let evap = EVAPORATION_MM_PER_GAME_HOUR / GAME_HOUR_S * es * (1.0 + 0.1 * wind) * dt;
            let taper = (self.film_mm / 0.02).clamp(0.0, 1.0);
            self.film_mm = (self.film_mm - evap * taper.max(0.25)).max(0.0);
            self.puddle_mm = (self.puddle_mm - evap * 0.5).max(0.0);
            if self.film_mm < 0.005 {
                self.film_mm = 0.0;
            }
        }
    }
    pub fn wetness(&self) -> f64 {
        1.0 - (-self.film_mm / WETNESS_FILM_MM).exp()
    }
    pub fn puddle_fill(&self) -> f64 {
        (self.puddle_mm / PUDDLE_CAPACITY_MM).clamp(0.0, 1.0)
    }
}

/// What the renderer reads: the weather at one instant.
#[derive(Clone, Copy, Debug)]
pub struct WeatherState {
    pub severity: f64,
    pub kind: Kind,
    pub profile: Profile,
    pub rain_mm_h: f64,
    pub wetness: f64,
    pub puddle_fill: f64,
    /// Wind toward which the air moves, m/s (x east, z south).
    pub wind: [f64; 2],
    pub gust: f64,
    /// 0..1 lightning flash and its distance (m), storm only.
    pub flash: f64,
    pub flash_distance_m: f64,
    /// The game day of the world clock, `floor(world_s / 86400)` (0 for a
    /// preset without a clock): a clear day's sky is drawn per game day
    /// (clouds.rs `drawn_state`).
    pub day: u32,
    /// The world clock the state was taken at (s; 0 for a preset without a
    /// clock): a clear day's sky changes to the next game day's over the end
    /// of the world clock's day (clouds.rs `sky_mix`).
    pub clock_s: f64,
}

/// The game day of a world clock (s).
pub fn game_day(world_s: f64) -> u32 {
    (world_s.max(0.0) / 86400.0).floor().min(u32::MAX as f64) as u32
}

/// The visual profile along the severity axis: the rung buckets clear, fog,
/// drizzle, rain (heavy drizzle draws the JS 'rain' bucket's look once its rate
/// passes 1 mm/h), storm, blended between neighbours.
pub fn profile_for_severity(s: f64) -> Profile {
    let rungs = [CLEAR, FOG, DRIZZLE, RAIN, STORM];
    let s = s.clamp(0.0, 4.0);
    let i = (s.floor() as usize).min(3);
    blend(&rungs[i], &rungs[i + 1], s - i as f64)
}

fn wind_for(s: f64, gust: f64) -> [f64; 2] {
    let s = s.clamp(0.0, 4.0);
    [anchor_lerp(&WIND_X, s) * gust, anchor_lerp(&WIND_Z, s) * gust]
}

/// The storm's lightning (weather-dynamics.js): one 6 s slot holds a strike
/// with probability 0.4 x activity; flash = power x (0.35 + 0.65 min(1,
/// 1500 / d)) x (1 - since / 0.3)^2 for 0.3 s.
pub fn lightning(seed: u32, world_s: f64, rate_mm_h: f64) -> (f64, f64) {
    if rate_mm_h < STORM_ONSET_RATE_MM_H {
        return (0.0, 0.0);
    }
    let activity = (0.4 + 0.6 * (rate_mm_h - 10.0) / 12.0).clamp(0.4, 1.0);
    let slot = (world_s / 6.0).floor();
    let mut best = (0.0, 0.0);
    for k in [slot - 1.0, slot] {
        if k < 0.0 {
            continue;
        }
        let i = k as u32;
        if hash01(seed, i, 41) >= 0.4 * activity {
            continue;
        }
        let at = (k + 0.05 + 0.9 * hash01(seed, i, 42)) * 6.0;
        let since = world_s - at;
        if !(0.0..0.3).contains(&since) {
            continue;
        }
        let d = lerp(400.0, 6000.0, hash01(seed, i, 43));
        let power = 0.6 + 0.4 * hash01(seed, i, 44);
        let f = (power * (0.35 + 0.65 * (1500.0 / d).min(1.0)) * (1.0 - since / 0.3).powi(2)).min(1.0);
        if f > best.0 {
            best = (f, d);
        }
    }
    best
}

/// Seconds of history the ground water is integrated over (3 x the slowest
/// time constant that matters on screen, the 40 s drain and minutes of
/// evaporation; puddles keep a longer memory but their 1800 s decay is
/// bounded by the 1.2 mm capacity).
pub const GROUND_WINDOW_S: f64 = 3600.0;

/// The scheduled weather at `world_s` (seconds since the world's epoch) for
/// a world `seed`. Pure: the same inputs give the same state everywhere. It
/// also tells the live calendar the clock (`eo_look::sun::set_calendar_clock`):
/// the renderer takes its weather at the world clock every frame, and that
/// moves live play's date (eo-render sky.rs `weather_of`).
pub fn state_at(seed: u32, world_s: f64) -> WeatherState {
    eo_look::sun::set_calendar_clock(world_s);
    let mut sch = Schedule::new(seed, 0);
    let mut g = Ground::default();
    let start = (world_s - GROUND_WINDOW_S).max(0.0);
    let mut t = start.floor();
    while t < world_s {
        let dt = (world_s - t).min(1.0);
        let s = sch.sample(t);
        let w = wind_for(s.severity, 1.0);
        g.step(s.severity, w[0].hypot(w[1]), dt);
        t += dt;
    }
    let s = sch.sample(world_s);
    let rate = rate_for_severity(s.severity);
    let (flash, flash_d) = lightning(seed, world_s, rate);
    let film = g.film_mm;
    WeatherState {
        severity: s.severity,
        kind: bucket(film, rate),
        profile: profile_for_severity(s.severity),
        rain_mm_h: rate,
        wetness: 1.0 - (-film / WETNESS_FILM_MM).exp(),
        puddle_fill: g.puddle_fill(),
        wind: wind_for(s.severity, s.gust_factor),
        gust: s.gust_factor,
        flash,
        flash_distance_m: flash_d,
        day: game_day(world_s),
        clock_s: world_s,
    }
}

/// A settled preset (`--weather <kind>`; environment-physics-state.js
/// `WEATHER_PRESETS`: drizzle 0.5 mm/h, rain 6 mm/h with a 0.19 mm film,
/// storm 22 mm/h with 0.4 mm and wind (11.5, 4.2); fog its 0.01 mm film).
/// Puddles stand as after a long rain (fill 0.6) in rain and storm.
pub fn preset(kind: Kind) -> WeatherState {
    let (rate, film, puddle, wind, severity): (f64, f64, f64, [f64; 2], f64) = match kind {
        Kind::Clear => (0.0, 0.0, 0.0, [0.0, 0.0], 0.0),
        Kind::Overcast => (0.0, 0.0, 0.0, [1.0, 0.4], 0.5),
        Kind::Fog => (0.0, 0.01, 0.0, [0.5, 0.2], 1.0),
        Kind::Drizzle => (0.5, 0.0508, 0.1, [2.0, 0.5], 2.0),
        Kind::Rain => (6.0, 0.19, 0.6, [4.4, 1.1], 3.2),
        Kind::Storm => (22.0, 0.4, 0.9, [11.5, 4.2], 4.0),
    };
    WeatherState {
        severity,
        kind,
        profile: profile(kind),
        rain_mm_h: rate,
        wetness: 1.0 - (-film / WETNESS_FILM_MM).exp(),
        puddle_fill: puddle,
        wind,
        gust: 1.0,
        flash: 0.0,
        flash_distance_m: 0.0,
        day: 0,
        clock_s: 0.0,
    }
}

/// A preset on the game day of a world clock (`preset` is day 0). The
/// renderer's preset weather should come from this so a clear day's sky is
/// drawn afresh each game day (eo-render sky.rs `weather_of`: a hand-off).
/// It tells the live calendar the clock too (`state_at`).
pub fn preset_at(kind: Kind, world_s: f64) -> WeatherState {
    eo_look::sun::set_calendar_clock(world_s);
    WeatherState { day: game_day(world_s), clock_s: world_s, ..preset(kind) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_is_a_pure_function_of_seed_and_clock() {
        let seed = crate::hash::schedule_seed("sf-world", 0);
        for t in [0.0, 100.0, 5000.0, 86400.0 * 3.0] {
            let a = state_at(seed, t);
            let b = state_at(seed, t);
            assert_eq!(a.severity, b.severity);
            assert_eq!(a.wetness, b.wetness);
        }
        // Severity moves only between adjacent rungs, and every rung is visited over a week.
        let mut sch = Schedule::new(seed, 0);
        let mut seen = [false; 5];
        let mut last = 0.0;
        let mut t = 0.0;
        while t < 7.0 * 86400.0 {
            let s = sch.sample(t).severity;
            assert!((s - last).abs() <= 1.0 + 1e-9, "jump {last} -> {s} at {t}");
            seen[s.round() as usize] = true;
            last = s;
            t += 5.0;
        }
        assert!(seen.iter().all(|v| *v), "rungs visited {seen:?}");
    }

    #[test]
    fn rates_and_film_follow_the_js() {
        assert_eq!(rate_for_severity(1.0), 0.0);
        assert!((rate_for_severity(2.0) - 0.5).abs() < 1e-12);
        assert!((rate_for_severity(3.0) - 1.0).abs() < 1e-12);
        assert!((rate_for_severity(4.0) - 22.0).abs() < 1e-12);
        assert!((film_for_severity(2.0) - 0.0508).abs() < 1e-12);
        assert_eq!(bucket(0.0, 0.0), Kind::Clear);
        assert_eq!(bucket(0.01, 0.0), Kind::Fog);
        assert_eq!(bucket(0.1, 6.0), Kind::Rain);
        assert_eq!(bucket(0.3, 12.0), Kind::Storm);
    }

    #[test]
    fn rain_wets_the_ground_and_it_dries_after() {
        let mut g = Ground::default();
        for _ in 0..600 {
            g.step(3.5, 3.0, 1.0); // ~11 mm/h for 10 minutes
        }
        assert!(g.wetness() > 0.95, "wetness after 10 min of rain {}", g.wetness());
        assert!(g.puddle_fill() > 0.05);
        for _ in 0..1800 {
            g.step(0.0, 0.0, 1.0);
        }
        assert!(g.wetness() < 0.05, "wetness 30 min after the rain {}", g.wetness());
    }

    #[test]
    fn beam_blends_in_log_space() {
        let m = blend(&CLEAR, &RAIN, 0.5);
        assert!((m.beam - (0.07f64).sqrt()).abs() < 1e-9);
    }
}
