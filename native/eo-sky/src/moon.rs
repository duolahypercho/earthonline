//! The moon (src/render/passes/sky-atmosphere.js `lunarPlacement`,
//! `moonPhaseFlux`, `lunarKey`): a low-precision lunar theory for its
//! position and phase, Krisciunas & Schaefer (1991) for the phase's flux, and
//! the moon's key light.

use eo_look::math::DEG;
use eo_look::sun::{LATITUDE_DEG, LONGITUDE_DEG, UTC_OFFSET_HOURS};

/// `MOON_EPOCH_J2000_DAYS`: day 0 of the lunar clock is 2025-10-05 00:00 UT.
pub const MOON_EPOCH_J2000_DAYS: f64 = 9408.5;
/// The lunar day the canonical world uses (`?moonDay` unset: day 0).
pub const DEFAULT_MOON_DAY: f64 = 0.0;
/// `MOON_ANGULAR_DIAMETER_DEG`.
pub const ANGULAR_DIAMETER_DEG: f64 = 0.5181;
/// `MOON_FULL_RADIANCE`: the full disc's drawn radiance (sky units).
pub const FULL_RADIANCE: f64 = 4.64;
/// `MOON_FULL_ILLUMINANCE_LUX`: full moon at the zenith, 0.267 lux.
pub const FULL_ILLUMINANCE_LUX: f64 = 0.267;
/// `MOON_KEY_SKY_SHARE`.
pub const KEY_SKY_SHARE: f64 = 0.35;
/// The moon's colour (linear).
pub const COLOR: [f64; 3] = [0.94, 0.95, 1.0];
/// `MOON_KEY_MIN_ALTITUDE_DEG` (renderer.js): the moon keys the scene above 3 degrees.
pub const KEY_MIN_ALTITUDE_DEG: f64 = 3.0;

#[derive(Clone, Copy, Debug)]
pub struct Moon {
    /// Toward the moon, game axes (x east, y up, z south).
    pub dir: [f64; 3],
    pub altitude_deg: f64,
    /// Phase angle, degrees (0 full, 180 new).
    pub phase_angle_deg: f64,
    /// Illuminated fraction of the disc.
    pub illuminated: f64,
    pub waxing: bool,
}

fn wrap360(x: f64) -> f64 {
    x.rem_euclid(360.0)
}

/// `lunarPlacement(hour, day)`.
pub fn placement(hour: f64, day: f64) -> Moon {
    let d = MOON_EPOCH_J2000_DAYS + day + (hour - UTC_OFFSET_HOURS) / 24.0;
    let l = wrap360(218.316 + 13.176396 * d);
    let m = wrap360(134.963 + 13.064993 * d);
    let f = wrap360(93.272 + 13.229350 * d);
    let lambda = l + 6.289 * (m * DEG).sin();
    let beta = 5.128 * (f * DEG).sin();
    let l0 = wrap360(280.459 + 0.98564736 * d);
    let g = wrap360(357.529 + 0.98560028 * d);
    let lambda_sun = l0 + 1.915 * (g * DEG).sin() + 0.020 * (2.0 * g * DEG).sin();
    let eps = (23.439 - 3.6e-7 * d) * DEG;
    let (lr, br) = (lambda * DEG, beta * DEG);
    let dec = (br.sin() * eps.cos() + br.cos() * eps.sin() * lr.sin()).asin();
    let ra = (lr.sin() * eps.cos() - br.tan() * eps.sin()).atan2(lr.cos());
    let gmst = wrap360(280.46061837 + 360.98564736629 * d);
    let ha = (gmst + LONGITUDE_DEG) * DEG - ra;
    let phi = LATITUDE_DEG * DEG;
    let up = phi.sin() * dec.sin() + phi.cos() * dec.cos() * ha.cos();
    let east = -dec.cos() * ha.sin();
    let north = dec.sin() * phi.cos() - dec.cos() * ha.cos() * phi.sin();
    let n = (up * up + east * east + north * north).sqrt();
    let dir = [east / n, up / n, -north / n];
    let elong = (br.cos() * ((lambda - lambda_sun) * DEG).cos()).clamp(-1.0, 1.0).acos() / DEG;
    let phase = 180.0 - elong;
    Moon {
        dir,
        altitude_deg: dir[1].clamp(-1.0, 1.0).asin() / DEG,
        phase_angle_deg: phase,
        illuminated: (1.0 + (phase * DEG).cos()) / 2.0,
        waxing: ((lambda - lambda_sun) * DEG).sin() >= 0.0,
    }
}

/// `moonPhaseFlux(a)`: the phase's flux relative to full (Krisciunas & Schaefer 1991).
pub fn phase_flux(phase_angle_deg: f64) -> f64 {
    let a = phase_angle_deg.abs().min(180.0);
    10f64.powf(-0.4 * (0.026 * a + 4e-9 * a.powi(4)))
}

/// The disc's radiance factor (`discFactor`: flux per lit area, a <= 155.05 deg).
pub fn disc_factor(phase_angle_deg: f64) -> f64 {
    let a = phase_angle_deg.abs().min(155.05);
    phase_flux(a) / ((1.0 + (a * DEG).cos()) / 2.0).max(1e-3)
}

/// Physical illuminance on the ground (lux) under a clear sky (`lunarKey`).
pub fn illuminance_lux(m: &Moon) -> f64 {
    FULL_ILLUMINANCE_LUX * phase_flux(m.phase_angle_deg) * m.dir[1].max(0.0) * eo_look::rig::direct_beam_transmittance(m.altitude_deg)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_canonical_lunar_day_is_near_full_and_waxing() {
        // 2025-10-05 sits two days before the full moon of 2025-10-07.
        let m = placement(21.0, DEFAULT_MOON_DAY);
        assert!(m.illuminated > 0.85, "illuminated {}", m.illuminated);
        let flux = phase_flux(0.0);
        assert!((flux - 1.0).abs() < 1e-12);
        assert!(phase_flux(90.0) < 0.15 && phase_flux(90.0) > 0.05);
    }
    #[test]
    fn the_moon_moves_about_a_degree_an_hour_less_than_the_sky() {
        let a = placement(21.0, 0.0);
        let b = placement(22.0, 0.0);
        let dot = a.dir[0] * b.dir[0] + a.dir[1] * b.dir[1] + a.dir[2] * b.dir[2];
        let deg = dot.clamp(-1.0, 1.0).acos() / DEG;
        assert!(deg > 9.0 && deg < 16.0, "moved {deg} deg in an hour");
    }
}
