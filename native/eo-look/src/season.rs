//! The season: how dry (golden) the annual grass is on a day of the year,
//! the one curve every land layer reads (the far layer's grass and the city
//! terrain's far land through `eo_render::farshore::season_dryness`; the look
//! uniform carries `current()` to the GPU each frame, read by nothing yet).

/// The season's switch names (the photo-date season track); eo-app accepts
/// them, nothing reads them yet.
pub const SWITCHES: &[&str] = &["--no-gg3-sky", "--no-photo-season", "--no-cirrus2", "--no-sky-day-fade", "--no-live-calendar"];

/// How golden the annual grass is on a day of the year: green from the first
/// winter rains to spring (Dec-Mar), browning through April-May, golden from
/// June until the rains (Bay Area annual grassland, a Mediterranean climate).
/// 0 green .. 1 golden.
pub fn dryness(day_of_year: f64) -> f32 {
    let d = day_of_year.rem_euclid(365.0);
    let s = |a: f64, b: f64, x: f64| {
        let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    // up from day 95 (Apr 5) to 165 (Jun 14); down from 305 (Nov 1) to 345 (Dec 11)
    (s(95.0, 165.0, d) * (1.0 - s(305.0, 345.0, d))) as f32
}

/// The dryness on the sun model's day (`sun::day_of_year`).
pub fn current() -> f32 {
    dryness(crate::sun::day_of_year())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn green_in_winter_golden_in_autumn() {
        assert!(dryness(265.0) > 0.99); // the game's date (22 Sep)
        assert!(dryness(45.0) < 0.01); // mid-February
        assert!(dryness(130.0) > 0.2 && dryness(130.0) < 0.8); // May: browning
        assert!(dryness(103.0) < 0.05); // 13 Apr (the headland photograph): still green
        assert!((dryness(400.0) - dryness(35.0)).abs() < 1e-6); // wraps by the year
        assert_eq!(current(), dryness(crate::sun::day_of_year()));
    }
}
