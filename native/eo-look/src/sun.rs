//! Solar position: `computeSunDirection` of src/render/environment-ibl.js
//! (NOAA approximation) on the canonical site and date the game uses:
//! San Francisco, September 22 (day 265 of a common year, `day_of_year`), UTC-7.

use crate::math::{DEG, TAU, clamp};
use std::sync::atomic::{AtomicU64, Ordering};

pub const LATITUDE_DEG: f64 = 37.7749;
pub const LONGITUDE_DEG: f64 = -122.4194;
pub const UTC_OFFSET_HOURS: f64 = -7.0;
/// `dayOfYearFromMonthDay(9, 22)`.
pub const DAY_OF_YEAR: f64 = 265.0;

/// The day of the year the sun is held on (`set_day_of_year`), as f64 bits;
/// NaN (from the start, and after `release_day`): not held, the live calendar
/// (`calendar_day`) places the sun, `DAY_OF_YEAR` on game day 0.
static DAY: AtomicU64 = AtomicU64::new(0x7ff8_0000_0000_0000);
/// The world clock the live calendar was last told (`set_calendar_clock`), s.
static CLOCK: AtomicU64 = AtomicU64::new(0f64.to_bits());

/// The game day of a world clock (s): eo-sky `weather::game_day`.
fn game_day_of(world_s: f64) -> f64 {
    (world_s.max(0.0) / 86400.0).floor()
}

/// The live calendar's window on the world clock's day, hours: over it the
/// date (and eo-sky's clear-sky state) moves on to the next game day's, so
/// nothing changes at the game day's boundary itself. The last three hours
/// of the world clock's day, its night (INFERRED: the world clock is not the
/// hour drawn in live play, which is the player's setting; any window works,
/// a night one keeps each game day's daytime its own).
pub const DAY_CHANGE_FROM_H: f64 = 21.0;
pub const DAY_CHANGE_TO_H: f64 = 24.0;
/// The live calendar's date moves through the window in this many steps (the
/// look frame's sky is integrated again on each: 0.05 deg of sun a step at
/// most, under the 0.4 deg a whole day moves it).
pub const DAY_CHANGE_STEPS: f64 = 8.0;

/// How far a world clock (s) is through its game day's change window, 0..1
/// (smoothstep over `DAY_CHANGE_FROM_H`..`DAY_CHANGE_TO_H`).
pub fn day_change(world_s: f64) -> f64 {
    let h = (world_s.max(0.0) - game_day_of(world_s) * 86400.0) / 3600.0;
    let t = ((h - DAY_CHANGE_FROM_H) / (DAY_CHANGE_TO_H - DAY_CHANGE_FROM_H)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A day of the year wrapped to a common year, 1 <= d < 366.
pub fn wrap_day(d: f64) -> f64 {
    (d - 1.0).rem_euclid(365.0) + 1.0
}

/// The live calendar's date of a game day: the game's start date
/// (`DAY_OF_YEAR`) plus the game day, wrapping (no live calendar: the start
/// date).
pub fn date_of_game_day(game_day: u32) -> f64 {
    if crate::season::features().live_calendar { wrap_day(DAY_OF_YEAR + game_day as f64) } else { DAY_OF_YEAR }
}

/// The live calendar's date at a world clock (s): the game day's date, moved
/// on toward the next one's through the night window in `DAY_CHANGE_STEPS`
/// steps (`--no-live-calendar`: always `DAY_OF_YEAR`).
pub fn calendar_day(world_s: f64) -> f64 {
    if !crate::season::features().live_calendar {
        return DAY_OF_YEAR;
    }
    let f = (day_change(world_s) * DAY_CHANGE_STEPS).round() / DAY_CHANGE_STEPS;
    wrap_day(DAY_OF_YEAR + game_day_of(world_s) + f)
}

/// Whether the live calendar listens to `set_calendar_clock` (`arm_calendar`).
static ARMED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Let `set_calendar_clock` move the date: the renderer arms it when it makes
/// its sky (eo-render clouds.rs), so a process without a world clock drawn on
/// screen (a unit test asking for the weather of day 4) never moves the sun.
pub fn arm_calendar() {
    ARMED.store(true, Ordering::Relaxed);
}

/// Tell the live calendar the world clock (s) (eo-sky's weather at a world
/// clock does, each frame in live play); ignored until `arm_calendar`.
pub fn set_calendar_clock(world_s: f64) {
    if ARMED.load(Ordering::Relaxed) {
        CLOCK.store(world_s.to_bits(), Ordering::Relaxed);
    }
}

/// The world clock the live calendar was last told.
pub fn calendar_clock() -> f64 {
    f64::from_bits(CLOCK.load(Ordering::Relaxed))
}

/// The day of the year (1 = 1 January, a common year) the sun is placed on:
/// the held day (`set_day_of_year`, a shot's date), else the live calendar's
/// (`calendar_day`: `DAY_OF_YEAR` on game day 0, and always with
/// `--no-live-calendar`).
pub fn day_of_year() -> f64 {
    let d = f64::from_bits(DAY.load(Ordering::Relaxed));
    if d.is_nan() { calendar_day(calendar_clock()) } else { d }
}

/// Hold the sun on a day of the year (process-wide): a shot's date.
pub fn set_day_of_year(day: f64) {
    DAY.store(day.to_bits(), Ordering::Relaxed);
}

/// Let the live calendar place the sun (`calendar_day` at the world clock
/// `set_calendar_clock` was told); with `--no-live-calendar` that is
/// `DAY_OF_YEAR`, as held.
pub fn release_day() {
    DAY.store(f64::NAN.to_bits(), Ordering::Relaxed);
}

/// Whether the sun's day is held (a shot's date) rather than the live calendar's.
pub fn day_held() -> bool {
    !f64::from_bits(DAY.load(Ordering::Relaxed)).is_nan()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sun {
    /// Unit direction toward the sun (+x east, +y up, -z north).
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub altitude_deg: f64,
    /// Clockwise from north.
    pub azimuth_deg: f64,
    pub hour_angle_deg: f64,
}

impl Sun {
    pub fn dir(&self) -> [f64; 3] {
        [self.x, self.y, self.z]
    }
}

/// Wrap an hour into [0, 24) (`wrapHour`).
pub fn wrap_hour(hour: f64) -> f64 {
    let w = hour % 24.0;
    if w < 0.0 { w + 24.0 } else { w }
}

pub fn sun_direction(hour: f64) -> Sun {
    sun_at(day_of_year(), hour)
}

/// The sun on a day of the year at a local hour.
pub fn sun_at(day_of_year: f64, hour: f64) -> Sun {
    let h = wrap_hour(hour);
    let gamma = (TAU / 365.0) * (day_of_year - 1.0 + (h - UTC_OFFSET_HOURS - 12.0) / 24.0);
    let eq_time = 229.18
        * (0.000075 + 0.001868 * gamma.cos() - 0.032077 * gamma.sin() - 0.014615 * (2.0 * gamma).cos() - 0.040849 * (2.0 * gamma).sin());
    let decl = 0.006918 - 0.399912 * gamma.cos() + 0.070257 * gamma.sin() - 0.006758 * (2.0 * gamma).cos() + 0.000907 * (2.0 * gamma).sin()
        - 0.002697 * (3.0 * gamma).cos()
        + 0.00148 * (3.0 * gamma).sin();
    let time_offset = eq_time + 4.0 * LONGITUDE_DEG - 60.0 * UTC_OFFSET_HOURS;
    let true_solar = h * 60.0 + time_offset;
    let hour_angle = (true_solar / 4.0 - 180.0) * DEG;
    let lat = LATITUDE_DEG * DEG;
    let cos_zenith = clamp(lat.sin() * decl.sin() + lat.cos() * decl.cos() * hour_angle.cos(), -1.0, 1.0);
    let altitude = cos_zenith.asin();
    let az_south = hour_angle.sin().atan2(hour_angle.cos() * lat.sin() - decl.tan() * lat.cos());
    let mut az = az_south + std::f64::consts::PI;
    if az < 0.0 {
        az += TAU;
    }
    if az >= TAU {
        az -= TAU;
    }
    let ca = altitude.cos();
    Sun {
        x: az.sin() * ca,
        y: altitude.sin(),
        z: -az.cos() * ca,
        altitude_deg: altitude / DEG,
        azimuth_deg: az / DEG,
        hour_angle_deg: hour_angle / DEG,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_day_defaults_to_the_game_date() {
        assert_eq!(day_of_year(), DAY_OF_YEAR);
    }

    #[test]
    fn the_calendar_moves_with_the_game_day_through_the_night() {
        // (the process's features are the defaults: the live calendar on)
        let h = |d: f64, hour: f64| d * 86400.0 + hour * 3600.0;
        assert_eq!(calendar_day(0.0), DAY_OF_YEAR);
        assert_eq!(calendar_day(h(0.0, 20.9)), DAY_OF_YEAR);
        assert_eq!(calendar_day(h(1.0, 0.0)), DAY_OF_YEAR + 1.0);
        assert_eq!(calendar_day(h(3.0, 12.0)), DAY_OF_YEAR + 3.0);
        // the year wraps: 100 game days from 22 September is 31 December + 0 .. 1 January
        assert_eq!(calendar_day(h(100.0, 12.0)), 365.0);
        assert_eq!(calendar_day(h(101.0, 12.0)), 1.0);
        assert_eq!(date_of_game_day(101), 1.0);
        // through the window the date moves in small steps and never jumps
        let mut prev = calendar_day(h(5.0, 20.0));
        let mut steps = 0;
        for k in 0..=600 {
            let d = calendar_day(h(5.0, 20.0 + k as f64 * 0.01));
            let step = d - prev;
            assert!((0.0..=1.0 / DAY_CHANGE_STEPS + 1e-9).contains(&step), "step {step} at {k}");
            steps += (step > 0.0) as usize;
            prev = d;
        }
        assert_eq!(steps, DAY_CHANGE_STEPS as usize);
        assert!((prev - (DAY_OF_YEAR + 6.0)).abs() < 1e-9);
        // and the sun at a fixed hour moves at most 0.05 deg a step
        let a = sun_at(DAY_OF_YEAR + 5.0, 13.0);
        let b = sun_at(DAY_OF_YEAR + 5.0 + 1.0 / DAY_CHANGE_STEPS, 13.0);
        assert!((a.altitude_deg - b.altitude_deg).abs() < 0.06 && (a.azimuth_deg - b.azimuth_deg).abs() < 0.06, "{a:?} {b:?}");
    }
}
