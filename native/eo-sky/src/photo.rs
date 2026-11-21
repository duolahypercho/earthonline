//! The photographic finish's world side (`--photo-finish`, stage W4; the
//! camera itself is eo-look `tone`): San Francisco's clear-day air for the
//! atmosphere tables and the aerial perspective, and the exposure value of the
//! scene the camera's ISO (and so its noise) follows. Pure functions; eo-render
//! sky.rs reads them only with the option on.
//!
//! The air. Without the option the atmosphere tables carry 0.55 x the
//! reference aerosol (sky_atmo.wgsl: Mie scattering 3.996e-3 / km, scale height
//! 1.2 km) with an absorption of 4.40e-3 / km per unit, an aerosol optical
//! depth of 0.006 at 550 nm: an alpine sky, a deep saturated blue down to the
//! horizon, while the aerial perspective beside it takes a ground extinction
//! of 0.072 / km (sky.rs `AP_HAZE_PER_KM` x the clear preset's haze), twelve
//! times the tables'. The far city fades to a horizon colour made of clean air,
//! so the haze reads blue rather than the warm grey of the reference photos
//! (sheet section 5.10, Broadway; section 5.2, One Rincon Hill). Here both take
//! one measured air: `SF_AEROSOL_EXT_PER_KM` at the ground, the tables with the
//! reference implementation's single-scattering albedo.

use crate::weather::CLEAR;

/// Aerosol extinction at the ground, 550 nm, on a clear San Francisco
/// afternoon (1/km). IMPROVE's Point Reyes visual range by cumulative
/// frequency (the reference sheet, section 4.3; vista.cira.colostate.edu
/// IMPROVE PORE freq_0900): median 80 km (49 Mm^-1), 80 % 50 km (78 Mm^-1),
/// 90 % 40 km (98 Mm^-1); the 15:00 series is the hazier (its haziest 2-3 % at
/// 20-25 km). Point Reyes is a clean coastal site upwind of the city at
/// 09:00; the city's afternoon air adds its own aerosol and the sea breeze's
/// marine aerosol, so the 50 km row stands for it: 78 Mm^-1 less IMPROVE's
/// 10 Mm^-1 of Rayleigh, 68 Mm^-1 = 0.068 / km. (The aerial perspective drew
/// 0.072 / km before: the extinction barely moves; its colour does.)
pub const SF_AEROSOL_EXT_PER_KM: f64 = 0.068;
/// IMPROVE's Rayleigh extinction at a sea-level site (Mm^-1 -> 1/km), the
/// share of its visual ranges that is clean air.
pub const IMPROVE_RAYLEIGH_PER_KM: f64 = 0.010;
/// Koschmieder's constant (2 % contrast threshold), as IMPROVE's visual range.
pub const KOSCHMIEDER: f64 = 3.912;

/// The atmosphere tables' aerosol per unit density (sky_atmo.wgsl `AT_MIE_S`).
pub const LUT_MIE_SCATTER_PER_KM: f64 = 3.996e-3;
/// The tables' aerosol absorption per unit density under the photo finish
/// (sky_atmo_luts.wgsl `AT_MIE_A_PHOTO`): Hillaire 2020's reference
/// implementation (sebh/UnrealEngineSkyAtmosphere) sets the Mie extinction to
/// 4.440e-3 / km against a scattering of 3.996e-3 / km, an absorption of
/// 0.444e-3 / km (single-scattering albedo 0.9). The shader's `AT_MIE_A`
/// (4.40e-3 / km, the default path) is that extinction taken as the
/// absorption: an albedo of 0.48, soot rather than sea salt, so more aerosol
/// greyed and darkened the horizon instead of whitening it.
pub const LUT_MIE_ABSORB_PHOTO_PER_KM: f64 = 0.444e-3;

/// The tables' aerosol scale height under the photo finish (km;
/// sky_atmo_luts.wgsl `AT_MIE_H_PHOTO`, against the reference's 1.2 km): the
/// coast's aerosol stays in the marine boundary layer under the inversion,
/// whose base is at 400-500 m on the northern California coast (the reference
/// sheet, section 4.1: Johnstone and Dawson 2010); the aerial perspective's
/// path density already thins over 400 m (sky_common.wgsl `sky_path_density`).
/// The horizon looks through all of it, the sky overhead through little: a
/// pale band low down, the zenith still blue (at 1.2 km the whole golden-hour
/// sky filled with the aerosol's light and went khaki).
pub const LUT_AEROSOL_SCALE_KM: f64 = 0.4;

/// The weather's aerosol at the ground (1/km): the clear preset's is
/// `SF_AEROSOL_EXT_PER_KM`, every weather scales it as the aerial perspective
/// always did (x (1 + 2 haze), sky.rs).
pub fn aerosol_ext_per_km(haze: f64) -> f64 {
    SF_AEROSOL_EXT_PER_KM * (1.0 + 2.0 * haze.max(0.0)) / (1.0 + 2.0 * CLEAR.haze)
}

/// The atmosphere tables' aerosol density (`Sky.atmo2.z`) that puts
/// `aerosol_ext_per_km` at the ground (scale height `LUT_AEROSOL_SCALE_KM`: an
/// aerosol optical depth of 0.027 at 550 nm in clear weather, all of it in the
/// boundary layer, against 0.006 over 1.2 km without the option).
pub fn lut_density(haze: f64) -> f64 {
    aerosol_ext_per_km(haze) / (LUT_MIE_SCATTER_PER_KM + LUT_MIE_ABSORB_PHOTO_PER_KM)
}

/// The aerial perspective's aerosol extinction at the ground (`Sky.atmo3.y`, 1/m).
pub fn ap_extinction_per_m(haze: f64) -> f64 {
    aerosol_ext_per_km(haze) * 1e-3
}

/// The meteorological visual range (km) of that air, IMPROVE's way.
pub fn visual_range_km(haze: f64) -> f64 {
    KOSCHMIEDER / (aerosol_ext_per_km(haze) + IMPROVE_RAYLEIGH_PER_KM)
}

/// Clear-sky global horizontal illuminance (lux) by the sun's altitude
/// (degrees): the reference sheet, sections 3.1 and 3.2 (P. Schlyter,
/// "Radiometry and photometry in astronomy", section 10).
const GLOBAL_LUX: [(f64, f64); 21] = [
    (-18.0, 0.000645),
    (-15.0, 0.00202),
    (-12.0, 0.00806),
    (-10.0, 0.0421),
    (-9.0, 0.116),
    (-8.0, 0.348),
    (-7.0, 1.09),
    (-6.0, 3.41),
    (-5.0, 10.4),
    (-4.0, 29.9),
    (-3.0, 79.1),
    (-2.0, 189.0),
    (-1.0, 405.0),
    (0.0, 759.0),
    (5.0, 4540.0),
    (10.0, 11100.0),
    (20.0, 27400.0),
    (30.0, 46300.0),
    (45.0, 77800.0),
    (60.0, 103000.0),
    (90.0, 129000.0),
];

/// `GLOBAL_LUX` at an altitude, log-linear between its rows, held at its ends.
pub fn clear_global_lux(sun_alt_deg: f64) -> f64 {
    let a = if sun_alt_deg.is_nan() { -90.0 } else { sun_alt_deg };
    let (first, last) = (GLOBAL_LUX[0], GLOBAL_LUX[GLOBAL_LUX.len() - 1]);
    if a <= first.0 {
        return first.1;
    }
    if a >= last.0 {
        return last.1;
    }
    for w in GLOBAL_LUX.windows(2) {
        let (lo, hi) = (w[0], w[1]);
        if a <= hi.0 {
            let t = (a - lo.0) / (hi.0 - lo.0);
            return (lo.1.ln() + (hi.1.ln() - lo.1.ln()) * t).exp();
        }
    }
    last.1
}

/// The exposure a camera takes on a lit San Francisco street at night,
/// EV100: the sheet's EXIF anchors (section 3.8: night street 3-6; section 5.8:
/// Castro and 18th, the LED-era neighbourhood street, 4.3). The camera meters
/// the street, its lamps and its windows, not the dark sky.
pub const NIGHT_STREET_EV100: f64 = 4.3;

/// The scene's EV100 for the camera: the sky's global illuminance (x the
/// weather's dome brightness) as an incident reading, plus the lit street's
/// equivalent illuminance as the lamps come on (`lamps_on` 0..1). A clear
/// noon is EV 15.6 (the sheet's EXIF on the I-80 deck: 15.2), the end of civil
/// twilight 0.4 without lamps and 4.4 with them, the night 4.3.
pub fn scene_ev100(sun_alt_deg: f64, lamps_on: f64, dome_brightness: f64) -> f64 {
    let sky = clear_global_lux(sun_alt_deg) * dome_brightness.clamp(0.0, 1.0);
    let street = 2.5 * NIGHT_STREET_EV100.exp2() * lamps_on.clamp(0.0, 1.0);
    crate::photometry::ev100_incident(sky + street)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_air_is_the_measured_air() {
        // the clear preset's visual range is IMPROVE's 50 km row.
        let v = visual_range_km(CLEAR.haze);
        assert!((v - 50.0).abs() < 0.5, "clear visual range {v} km");
        // inside the measured distribution: hazier than the morning median (80 km), clearer than
        // the afternoon's haziest (20-25 km).
        assert!(v < 80.0 && v > 25.0);
        // a hazier weather is hazier; the aerial perspective and the tables take the same air.
        assert!(aerosol_ext_per_km(0.5) > aerosol_ext_per_km(CLEAR.haze));
        let lut = lut_density(CLEAR.haze) * (LUT_MIE_SCATTER_PER_KM + LUT_MIE_ABSORB_PHOTO_PER_KM);
        assert!((lut - ap_extinction_per_m(CLEAR.haze) * 1e3).abs() < 1e-12);
        // and the tables' aerosol optical depth (the boundary layer's) a few times the default's 0.006.
        let aod = lut * LUT_AEROSOL_SCALE_KM;
        assert!(aod > 0.02 && aod < 0.1, "aod {aod}");
        // the albedo is the reference implementation's 0.9.
        let ssa = LUT_MIE_SCATTER_PER_KM / (LUT_MIE_SCATTER_PER_KM + LUT_MIE_ABSORB_PHOTO_PER_KM);
        assert!((ssa - 0.9).abs() < 1e-3);
    }

    #[test]
    fn the_scene_ev_follows_the_light() {
        // clear noon: the sheet's EV 15.2 on the I-80 deck (Schlyter 103-129 klx: 15.3-15.7).
        let noon = scene_ev100(60.0, 0.0, 1.0);
        assert!((15.0..16.0).contains(&noon), "noon {noon}");
        // the table's own rows, and between them in log.
        assert!((clear_global_lux(-6.0) - 3.41).abs() < 1e-9);
        let mid = clear_global_lux(-5.5);
        assert!(mid > 3.41 && mid < 10.4 && (mid - (3.41f64 * 10.4).sqrt()).abs() < 1e-6);
        // monotone in the altitude.
        let mut last = 0.0;
        for i in -200..=900 {
            let l = clear_global_lux(i as f64 * 0.1);
            assert!(l >= last, "{i}");
            last = l;
        }
        // night with lamps: the street's EV; without: far darker. Never NaN.
        assert!((scene_ev100(-30.0, 1.0, 1.0) - NIGHT_STREET_EV100).abs() < 0.01);
        assert!(scene_ev100(-30.0, 0.0, 1.0) < -5.0);
        assert!(scene_ev100(f64::NAN, f64::NAN, f64::NAN).is_finite());
        // a darker weather lowers the day's EV.
        assert!(scene_ev100(30.0, 0.0, 0.3) < scene_ev100(30.0, 0.0, 1.0));
    }
}
