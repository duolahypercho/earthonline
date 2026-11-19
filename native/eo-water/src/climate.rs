//! The Golden Gate's measured wind and tidal current (water iteration 2,
//! water-1), from `data/golden-gate-climate.json` (NOAA CO-OPS; its NOTICE
//! and its generator `data/build-golden-gate-climate.py` sit beside it).
//!
//! - [`strait_wind`]: the mean wind speed at NOAA station 9414290 (San
//!   Francisco, on the Presidio shore inside the Gate) for a day of the year
//!   and an hour, from three years of hourly records (2022-2024): about
//!   2-3 m/s at night and in the morning, 5-6 m/s in the afternoon from
//!   April to September, from 255-262 degrees.
//! - [`tidal_current`]: the current along the Golden Gate's axis at NOAA
//!   current station SFB1203 bin 18 (9 m down, 0.46 nm east of the bridge),
//!   predicted from its harmonic constants (amplitude and Greenwich phase per
//!   constituent) with the astronomical arguments and nodal factors of
//!   Schureman (1958), "Manual of Harmonic Analysis and Prediction of Tides",
//!   US Coast and Geodetic Survey Special Publication 98, tables 1-2 and 14
//!   (the simplified forms Pugh 1987, "Tides, Surges and Mean Sea-Level",
//!   table 4.3, gives). Within 10 cm/s and 6 minutes of NOAA's own published
//!   predictions for the station (test below).

use serde::Deserialize;
use std::sync::OnceLock;

/// The game's clock is Pacific daylight time (eo-look sun.rs `UTC_OFFSET_HOURS`);
/// the station's table is in local standard time (UTC-8).
pub const GAME_UTC_OFFSET_H: f64 = -7.0;
pub const TABLE_UTC_OFFSET_H: f64 = -8.0;
/// The game has a day of the year and no year; the tide is predicted in this
/// one (a common year). INFERRED: any year gives the Gate's real range of
/// currents; this one fixes which phase of the tide a given hour shows.
pub const TIDE_YEAR: i32 = 2025;

#[derive(Deserialize)]
struct Wind {
    mean_speed: Vec<Vec<f64>>,
    vector_mean_from_deg: Vec<Vec<f64>>,
}

#[derive(Deserialize)]
struct Constituent {
    name: String,
    amp_cm_s: f64,
    phase_gmt_deg: f64,
}

#[derive(Deserialize)]
struct Current {
    major_axis_azimuth_deg: f64,
    mean_cm_s: f64,
    constituents: Vec<Constituent>,
}

#[derive(Deserialize)]
struct Climate {
    wind: Wind,
    current: Current,
}

fn climate() -> &'static Climate {
    static C: OnceLock<Climate> = OnceLock::new();
    C.get_or_init(|| serde_json::from_str(include_str!("../data/golden-gate-climate.json")).expect("golden-gate-climate.json"))
}

/// Linear interpolation in a month x hour table: months at their middles
/// (day 15.2 + 30.4 m, wrapping over the year), hours at the hour.
fn table_at(t: &[Vec<f64>], day_of_year: f64, hour_game: f64) -> f64 {
    let hour = (hour_game - GAME_UTC_OFFSET_H + TABLE_UTC_OFFSET_H).rem_euclid(24.0);
    let mf = ((day_of_year - 15.2) / (365.0 / 12.0)).rem_euclid(12.0);
    let (m0, tm) = (mf.floor() as usize % 12, mf.fract());
    let m1 = (m0 + 1) % 12;
    let (h0, th) = (hour.floor() as usize % 24, hour.fract());
    let h1 = (h0 + 1) % 24;
    let at = |m: usize| t[m][h0] * (1.0 - th) + t[m][h1] * th;
    at(m0) * (1.0 - tm) + at(m1) * tm
}

/// The mean wind speed (m/s) at the strait's shore station for a day of the
/// year (1 = 1 January) and an hour of the game's clock.
pub fn strait_wind(day_of_year: f64, hour: f64) -> f64 {
    table_at(&climate().wind.mean_speed, day_of_year, hour)
}

/// The direction the station's mean wind blows from (compass degrees), its
/// vector mean (the westerly through the Gate in the afternoon).
pub fn strait_wind_from_deg(day_of_year: f64, hour: f64) -> f64 {
    // (interpolated as unit vectors so 350 and 10 average to 0)
    let t = &climate().wind.vector_mean_from_deg;
    let s: Vec<Vec<f64>> = t.iter().map(|r| r.iter().map(|d| d.to_radians().sin()).collect()).collect();
    let c: Vec<Vec<f64>> = t.iter().map(|r| r.iter().map(|d| d.to_radians().cos()).collect()).collect();
    table_at(&s, day_of_year, hour).atan2(table_at(&c, day_of_year, hour)).to_degrees().rem_euclid(360.0)
}

/// Days from J2000.0 (2000-01-01 12:00 UT) to `hour_ut` hours of day `doy` (1-based) of `year`.
pub fn days_from_j2000(year: i32, doy: f64, hour_ut: f64) -> f64 {
    let mut days = 0.0;
    if year >= 2000 {
        for y in 2000..year {
            days += if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 { 366.0 } else { 365.0 };
        }
    } else {
        for y in year..2000 {
            days -= if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 { 366.0 } else { 365.0 };
        }
    }
    days + (doy - 1.0) + hour_ut / 24.0 - 0.5
}

/// Schureman's equilibrium argument V (degrees) of a constituent at mean
/// longitudes s (moon), h (sun), p (lunar perigee) and the hour angle T of the
/// mean sun at Greenwich, and its nodal factor f and correction u (degrees)
/// at the moon's node N. None for a constituent not listed.
fn argument(name: &str, s: f64, h: f64, p: f64, n_deg: f64, t: f64) -> Option<(f64, f64, f64)> {
    let n = n_deg.to_radians();
    let (c1, c2, c3) = (n.cos(), (2.0 * n).cos(), (3.0 * n).cos());
    let (s1, s2, s3) = (n.sin(), (2.0 * n).sin(), (3.0 * n).sin());
    let m2 = (1.0004 - 0.0373 * c1 + 0.0002 * c2, -2.14 * s1);
    let k1 = (1.0060 + 0.1150 * c1 - 0.0088 * c2 + 0.0006 * c3, -8.86 * s1 + 0.68 * s2 - 0.07 * s3);
    let o1 = (1.0089 + 0.1871 * c1 - 0.0147 * c2 + 0.0014 * c3, 10.80 * s1 - 1.34 * s2 + 0.19 * s3);
    let k2 = (1.0241 + 0.2863 * c1 + 0.0083 * c2 - 0.0015 * c3, -17.74 * s1 + 0.68 * s2 - 0.04 * s3);
    let j1 = (1.1029 + 0.1676 * c1 - 0.0170 * c2 + 0.0016 * c3, -12.94 * s1 + 1.34 * s2 - 0.19 * s3);
    let oo1 = (1.1027 + 0.6504 * c1 + 0.0317 * c2 - 0.0014 * c3, -36.68 * s1 + 4.02 * s2 - 0.57 * s3);
    let vm2 = 2.0 * t - 2.0 * s + 2.0 * h;
    let vn2 = 2.0 * t - 3.0 * s + 2.0 * h + p;
    let vk1 = t + h - 90.0;
    Some(match name {
        "M2" => (vm2, m2.0, m2.1),
        "S2" => (2.0 * t, 1.0, 0.0),
        "N2" => (vn2, m2.0, m2.1),
        "K2" => (2.0 * t + 2.0 * h, k2.0, k2.1),
        "L2" => (2.0 * t - s + 2.0 * h - p + 180.0, m2.0, m2.1),
        "K1" => (vk1, k1.0, k1.1),
        "O1" => (t - 2.0 * s + h + 90.0, o1.0, o1.1),
        "P1" => (t - h + 90.0, 1.0, 0.0),
        "Q1" => (t - 3.0 * s + h + p + 90.0, o1.0, o1.1),
        "J1" => (t + s + h - p - 90.0, j1.0, j1.1),
        "OO1" => (t + 2.0 * s + h - 90.0, oo1.0, oo1.1),
        "M4" => (2.0 * vm2, m2.0 * m2.0, 2.0 * m2.1),
        "MS4" => (vm2 + 2.0 * t, m2.0, m2.1),
        "MN4" => (vm2 + vn2, m2.0 * m2.0, 2.0 * m2.1),
        "M6" => (3.0 * vm2, m2.0.powi(3), 3.0 * m2.1),
        "MK3" => (vm2 + vk1, m2.0 * k1.0, m2.1 + k1.1),
        "2MK3" => (2.0 * vm2 - vk1, m2.0 * m2.0 * k1.0, 2.0 * m2.1 - k1.1),
        _ => return None,
    })
}

/// The tidal current (m/s) along the Golden Gate's axis at SFB1203, positive
/// on the flood (toward [`current_axis_deg`], into the bay), at `days`
/// from J2000.0 (UT).
pub fn current_at_j2000(days: f64) -> f64 {
    let c = &climate().current;
    let s = 218.3164 + 13.176_396_48 * days;
    let h = 280.4661 + 0.985_647_36 * days;
    let p = 83.3532 + 0.111_403_53 * days;
    let n = 125.0445 - 0.052_953_77 * days;
    let hour_ut = (days + 0.5).rem_euclid(1.0) * 24.0;
    let t = 180.0 + 15.0 * hour_ut;
    let mut v = c.mean_cm_s;
    for k in &c.constituents {
        if let Some((arg, f, u)) = argument(&k.name, s, h, p, n, t) {
            v += f * k.amp_cm_s * (arg + u - k.phase_gmt_deg).to_radians().cos();
        }
    }
    v / 100.0
}

/// The tidal current (m/s, + flood) at the Gate for a day of the year and an
/// hour of the game's clock, in [`TIDE_YEAR`].
pub fn tidal_current(day_of_year: f64, hour: f64) -> f64 {
    current_at_j2000(days_from_j2000(TIDE_YEAR, day_of_year, hour - GAME_UTC_OFFSET_H))
}

/// The flood's direction at the station (compass degrees, toward).
pub fn current_axis_deg() -> f64 {
    climate().current.major_axis_azimuth_deg
}

#[cfg(test)]
mod tests {
    use super::*;

    // NOAA CO-OPS current predictions for SFB1203 bin 18 (MAX_SLACK, GMT,
    // fetched 2026-11-18 from api.tidesandcurrents.noaa.gov), 20-21 September
    // 2025: time (day of year, hour UT) and the velocity along the major axis (cm/s).
    const NOAA: [(f64, f64, f64); 8] = [
        (263.0, 4.0 + 10.0 / 60.0, 127.1),
        (263.0, 10.0 + 5.0 / 60.0, -112.5),
        (263.0, 16.0 + 52.0 / 60.0, 157.5),
        (263.0, 22.0 + 34.0 / 60.0, -105.8),
        (264.0, 5.0 + 1.0 / 60.0, 130.6),
        (264.0, 10.0 + 46.0 / 60.0, -107.5),
        (263.0, 7.0 + 3.0 / 60.0, 0.5),
        (263.0, 13.0 + 42.0 / 60.0, 0.0),
    ];

    #[test]
    fn the_current_matches_noaa_predictions() {
        let minute = 1.0 / 1440.0;
        for (doy, h, want) in NOAA {
            let d = days_from_j2000(2025, doy, h);
            let got = current_at_j2000(d) * 100.0;
            assert!((got - want).abs() < 10.0, "day {doy} {h:.2} h UT: {got:.1} cm/s, NOAA {want}");
            // when it happens: the reconstruction's own extremum (the largest
            // speed) or slack (the sign change) nearest NOAA's, searched a minute
            // at a time over an hour and a half either side, within 6 minutes of NOAA's
            let at = |m: i32| current_at_j2000(d + m as f64 * minute);
            let off = if want.abs() > 50.0 {
                let best = (-90..=90).max_by(|a, b| (at(*a) * want.signum()).total_cmp(&(at(*b) * want.signum()))).unwrap();
                assert!(best.abs() < 90, "no extremum near {doy} {h:.2}");
                best
            } else {
                let cross = (-90..90).filter(|m| at(*m).signum() != at(m + 1).signum()).min_by_key(|m| m.abs());
                cross.unwrap_or_else(|| panic!("no slack near {doy} {h:.2}"))
            };
            eprintln!("day {doy} {h:5.2} h UT: {got:6.1} cm/s (NOAA {want:6.1}), {off:+} min");
            assert!(off.abs() <= 6, "day {doy} {h:.2} h UT: the reconstruction's {} is {off} minutes from NOAA's", if want.abs() > 50.0 { "extremum" } else { "slack" });
        }
    }

    #[test]
    fn the_strait_wind_is_the_stations() {
        // 22 September (day 265): a light morning, the afternoon westerly about 5 m/s.
        let m = strait_wind(265.0, 10.0);
        let a = strait_wind(265.0, 16.0);
        assert!((2.0..3.5).contains(&m), "morning {m}");
        assert!((4.5..6.5).contains(&a), "afternoon {a}");
        let d = strait_wind_from_deg(265.0, 16.0);
        assert!((240.0..275.0).contains(&d), "from {d}");
        // 13 April (day 103) at 17 h, the headland photograph's afternoon
        assert!((4.5..6.5).contains(&strait_wind(103.0, 17.0)));
    }
}
