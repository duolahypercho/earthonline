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

/// The day of the year the sun is placed on (`day_of_year`), as f64 bits;
/// `DAY_OF_YEAR` until `set_day_of_year` changes it.
static DAY: AtomicU64 = AtomicU64::new(DAY_OF_YEAR.to_bits());

/// The day of the year (1 = 1 January, a common year) the sun is placed on:
/// `DAY_OF_YEAR` unless `set_day_of_year` set another.
pub fn day_of_year() -> f64 {
    f64::from_bits(DAY.load(Ordering::Relaxed))
}

/// Place the sun on another day of the year (process-wide).
pub fn set_day_of_year(day: f64) {
    DAY.store(day.to_bits(), Ordering::Relaxed);
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
    let h = wrap_hour(hour);
    let gamma = (TAU / 365.0) * (day_of_year() - 1.0 + (h - UTC_OFFSET_HOURS - 12.0) / 24.0);
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
}
