//! Spectral colour of the direct sun through the air (`--no-sun-spectrum2`
//! keeps the three-channel colour): the atmosphere's three channels (680,
//! 550, 440 nm, `atmosphere::CHANNEL_NM`) give the beam's transmittance at
//! three wavelengths, which overstate the colour of a broad spectrum and miss
//! the red primary's band; `atmosphere::RGB_CHROMA` corrects the chroma's
//! size but not its direction. Here the same medium's slant optical depths
//! (Rayleigh, aerosol, ozone, integrated along the sun's ray as the
//! transmittance table does) are spread over 380-780 nm with each
//! constituent's own spectral law, the Sun taken as its 5772 K Planckian
//! radiator (`atmosphere::SUN_EFFECTIVE_TEMPERATURE_K`), and the transmitted
//! spectrum is taken through the CIE 1931 2 deg observer to linear sRGB:
//! the spectral method of Bird and Riordan 1986 ("Simple solar spectral model
//! for direct and diffuse irradiance on horizontal and tilted planes at the
//! earth's surface for cloudless atmospheres", J. Climate Appl. Meteor. 25,
//! 87-97: the beam as the product of the constituents' spectral
//! transmittances) on this model's own medium.
//!
//! * Rayleigh: lambda^-4, the law the model's three coefficients follow
//!   (5.802 / 13.558 / 33.1 e-3 per km at 680 / 550 / 440 nm are 550^4 /
//!   lambda^4 x 13.558e-3 within 0.1 %).
//! * Aerosol: the Angstrom law, (lambda / 550)^-alpha (`Atmosphere::aerosol_spectral`).
//! * Ozone: the Chappuis band, a Gaussian in wavelength through the
//!   cross-section's peak, 5.2e-21 cm2 at 603 nm, and its 3.5e-21 cm2 at
//!   550 nm (Serdyuchenko et al. 2014, Atmos. Meas. Tech. 7, 625-636, the
//!   values the model's ozone was built from); the Gaussian shape is INFERRED
//!   (it keeps the model's blue channel's 1/22 of the green within 30 %).
//! * The CIE 1931 colour-matching functions: the multi-lobe piecewise
//!   Gaussian fit of Wyman, Sloan and Shirley 2013 ("Simple Analytic
//!   Approximations to the CIE XYZ Color Matching Functions", JCGT 2(2),
//!   1-11), within about 1 % of the tabulated functions.

use crate::atmosphere::{Atmosphere, R_GROUND_KM, R_TOP_KM, SUN_EFFECTIVE_TEMPERATURE_K, ground_hit, sphere, xy_to_srgb};
use crate::math::*;

const L0_NM: f64 = 380.0;
const L1_NM: f64 = 780.0;
const DL_NM: f64 = 5.0;

/// Ozone's Chappuis cross-section peak (cm2) and its wavelength (nm), and
/// the Gaussian's width that passes through 3.5e-21 cm2 at 550 nm.
const OZONE_PEAK_CM2: f64 = 5.2e-21;
const OZONE_PEAK_NM: f64 = 603.0;
const OZONE_550_CM2: f64 = 3.5e-21;

fn ozone_cross_section(l: f64) -> f64 {
    let w = (OZONE_PEAK_NM - 550.0) / (2.0 * (OZONE_PEAK_CM2 / OZONE_550_CM2).ln()).sqrt();
    OZONE_PEAK_CM2 * (-0.5 * ((l - OZONE_PEAK_NM) / w).powi(2)).exp()
}

fn lobe(l: f64, mu: f64, s1: f64, s2: f64) -> f64 {
    let s = if l < mu { s1 } else { s2 };
    (-0.5 * ((l - mu) / s).powi(2)).exp()
}

/// The CIE 1931 2 deg colour-matching functions (Wyman, Sloan and Shirley 2013).
pub fn cmf(l: f64) -> [f64; 3] {
    [
        1.056 * lobe(l, 599.8, 37.9, 31.0) + 0.362 * lobe(l, 442.0, 16.0, 26.7) - 0.065 * lobe(l, 501.1, 20.4, 26.2),
        0.821 * lobe(l, 568.8, 46.9, 40.5) + 0.286 * lobe(l, 530.9, 16.3, 31.1),
        1.217 * lobe(l, 437.0, 11.8, 36.0) + 0.681 * lobe(l, 459.0, 26.0, 13.8),
    ]
}

/// Planck's law (relative spectral radiance) at `l` nm for `t` K.
pub fn planck(l: f64, t: f64) -> f64 {
    let lm = l * 1e-9;
    let c2 = 1.438_776_877e-2; // h c / k (m K)
    1.0 / (lm.powi(5) * ((c2 / (lm * t)).exp() - 1.0))
}

/// XYZ of a spectrum given as a function of wavelength (nm).
pub fn xyz_of(s: &dyn Fn(f64) -> f64) -> [f64; 3] {
    let mut o = [0.0; 3];
    let n = ((L1_NM - L0_NM) / DL_NM).round() as usize;
    for i in 0..=n {
        let l = L0_NM + i as f64 * DL_NM;
        let w = if i == 0 || i == n { 0.5 } else { 1.0 };
        let c = cmf(l);
        let v = s(l) * w;
        for k in 0..3 {
            o[k] += v * c[k];
        }
    }
    o
}

/// The sun's slant optical depths along its ray to a height (m): Rayleigh
/// at 550 nm, aerosol extinction at 550 nm, and the ozone's absorption
/// at the model's green channel (its 550 nm), for a sun at `sun_y` (sine of
/// its altitude). None where the planet hides the sun.
pub fn slant_depths(atm: &Atmosphere, density: f64, h_m: f64, sun_y: f64) -> Option<[f64; 3]> {
    let r = R_GROUND_KM + h_m.max(1.0) * 0.001;
    if ground_hit(r, sun_y) {
        return None;
    }
    let len = sphere(r, sun_y, R_TOP_KM).max(0.0);
    let steps = 200;
    let dt = len / steps as f64;
    let mut tau = [0.0; 3];
    for i in 0..steps {
        let t = (i as f64 + 0.5) * dt;
        let h = (r * r + t * t + 2.0 * r * sun_y * t).sqrt() - R_GROUND_KM;
        let hh = h.max(0.0);
        tau[0] += atm.rayleigh[1] * (-hh / atm.rayleigh_h_km).exp() * dt;
        tau[1] += (atm.aerosol_scat + atm.aerosol_abs) * (-hh / atm.aerosol_h_km).exp() * density * dt;
        tau[2] += atm.ozone[1] * (1.0 - (hh - 25.0).abs() / 15.0).max(0.0) * dt;
    }
    Some(tau)
}

/// The Angstrom exponent the medium's aerosol spectral factors carry.
fn angstrom(atm: &Atmosphere) -> f64 {
    // aerosol_spectral = (lambda / 550)^-alpha at 440 nm
    -atm.aerosol_spectral[2].ln() / (440.0f64 / 550.0).ln()
}

/// The direct beam's colour at the ground in linear sRGB, luminance = the
/// beam's luminous transmittance (the Sun's light above the air has
/// luminance 1), from the spectrum; None where the planet hides the sun.
pub fn beam_srgb(atm: &Atmosphere, density: f64, h_m: f64, sun_y: f64) -> Option<Rgb> {
    let [tr, ta, to] = slant_depths(atm, density, h_m, sun_y)?;
    let alpha = angstrom(atm);
    let t = SUN_EFFECTIVE_TEMPERATURE_K;
    let toa = xyz_of(&|l| planck(l, t));
    let s = |l: f64| {
        let tau = tr * (550.0 / l).powi(4) + ta * (l / 550.0).powf(-alpha) + to * ozone_cross_section(l) / OZONE_550_CM2;
        planck(l, t) * (-tau).exp()
    };
    let xyz = xyz_of(&s);
    let y = xyz[1] / toa[1];
    let sum = xyz[0] + xyz[1] + xyz[2];
    if sum <= 0.0 || y <= 1e-12 {
        return Some([0.0; 3]);
    }
    let c = xy_to_srgb(xyz[0] / sum, xyz[1] / sum);
    let l = luminance(c).max(1e-12);
    Some(c.map(|v| v / l * y))
}

/// CIE xy of linear sRGB.
pub fn srgb_xy(c: Rgb) -> [f64; 2] {
    let x = 0.4124 * c[0] + 0.3576 * c[1] + 0.1805 * c[2];
    let y = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let z = 0.0193 * c[0] + 0.1192 * c[1] + 0.9505 * c[2];
    let s = (x + y + z).max(1e-12);
    [x / s, y / s]
}

/// McCamy 1992's correlated colour temperature of a chromaticity (K).
pub fn cct(xy: [f64; 2]) -> f64 {
    let n = (xy[0] - 0.3320) / (0.1858 - xy[1]);
    449.0 * n.powi(3) + 3525.0 * n * n + 6823.3 * n + 5520.33
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atmosphere::{self, Atmosphere};

    #[test]
    fn the_fit_integrates_an_equal_energy_white_to_its_white_point() {
        let e = xyz_of(&|_| 1.0);
        let s = e[0] + e[1] + e[2];
        // CIE illuminant E: x = y = 1/3
        assert!((e[0] / s - 1.0 / 3.0).abs() < 0.004 && (e[1] / s - 1.0 / 3.0).abs() < 0.004, "{e:?}");
        // a 5772 K Planckian: x 0.3256, y 0.3359 (Kim et al. 2002's fit of the locus)
        let p = xyz_of(&|l| planck(l, 5772.0));
        let s = p[0] + p[1] + p[2];
        assert!((p[0] / s - 0.3256).abs() < 0.003 && (p[1] / s - 0.3359).abs() < 0.003, "{} {}", p[0] / s, p[1] / s);
    }

    #[test]
    fn the_spectral_beam_against_the_three_channel_beam() {
        // the photographs' suns: 3 (Baker), 10, 31 (headland), 34 (Fort Point), 46 (Crissy) deg
        let atm = Atmosphere::coastal_clear();
        let t = atmosphere::tables(&atm, 1.0);
        let toa = atmosphere::sun_colour_toa();
        for el in [3.0, 10.0, 31.0, 34.0, 46.0] {
            let y = (el * DEG).sin();
            let s = beam_srgb(&atm, 1.0, 2.0, y).unwrap();
            let tr = atmosphere::spectral_chroma(t.sun_transmittance(2.0, y));
            let three = [0, 1, 2].map(|i| toa[i] * tr[i]);
            let (xs, x3) = (srgb_xy(s), srgb_xy(three));
            eprintln!(
                "sun {el:4.1} deg: spectral xy {:.4} {:.4} ({:.0} K, Y {:.3}), three-channel xy {:.4} {:.4} ({:.0} K, Y {:.3})",
                xs[0], xs[1], cct(xs), luminance(s), x3[0], x3[1], cct(x3), luminance(three)
            );
            // the luminous transmittance agrees within 6 % from 30 deg up (the same medium);
            // lower the three channels miss the band's reddening (10 % at 10 deg, 25 % at 3)
            let tol = if el >= 30.0 { 0.06 } else { 0.3 };
            assert!((luminance(s) / luminance(three) - 1.0).abs() < tol, "{el}");
            // a clear-sky beam is warmer than the Sun above the air and warms as the sun sinks
            assert!(cct(xs) < 5800.0);
        }
        // its cost (once per new Daylight: an hour, weather or ground change)
        let t0 = std::time::Instant::now();
        for _ in 0..100 {
            std::hint::black_box(beam_srgb(&atm, 1.0, 2.0, 0.5));
        }
        eprintln!("beam_srgb: {:.1} us", t0.elapsed().as_secs_f64() * 1e4);
        // the beam at the headland's 31 deg sun: about 4700-5400 K (INFERRED sanity band)
        let c31 = cct(srgb_xy(beam_srgb(&atm, 1.0, 2.0, (31.0 * DEG).sin()).unwrap()));
        assert!((4700.0..5400.0).contains(&c31), "{c31}");
    }
}
