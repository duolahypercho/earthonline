//! The cinematic look's golden hour (`--look cinematic`, the cinematic look
//! plan's stage L4), as pure functions: the sun's colour by altitude, the
//! directional sky fill, the rim's share and the day exposure's adaptation.
//! eo-sky `frame::apply_with` applies them when `FrameOptions::cinematic` is
//! set; the default frame never calls them. Every value here that departs
//! from the default's reference is a row in `cinematic::CINEMATIC_DEVIATIONS`.

use crate::math::*;
use crate::sky::SkyModel;

/// The sun's colour by its altitude under the cinematic look (degrees, linear
/// sRGB at luminance 1): the direct beam's correlated colour temperature,
/// 5900 K from 40 deg up, 5200 K at 20, 4300 K at 12, 3600 K at 8, 3300 K at
/// 6, 2900 K at 4, 2400 K at 2, 2100 K on the horizon (a measured clear-sky
/// beam runs ~5500-5900 K by day and falls through ~3500 K near 5 deg to
/// ~2000 K at sunset; the low end pushed a little warmer for the look). Each
/// knot is Planck's law integrated against the CIE 1931 2-degree observer
/// (the multi-lobe fit of Wyman, Sloan & Shirley, JCGT 2(2), 2013), XYZ to
/// linear sRGB (IEC 61966-2-1), divided by its Rec. 709 luminance. Red falls
/// and blue rises from knot to knot, so the curve's red/blue ratio falls
/// monotonically with altitude. Default: the JS rig table's colour (red/blue
/// 1.75 at noon, 3.4 at 18.5 h).
pub const SUN_COLOUR_KNOTS: [(f64, Rgb); 8] = [
    (0.0, [2.3744, 0.6883, 0.0410]),  // 2100 K
    (2.0, [2.1154, 0.7580, 0.1129]),  // 2400 K
    (4.0, [1.8040, 0.8373, 0.2442]),  // 2900 K
    (6.0, [1.6257, 0.8795, 0.3516]),  // 3300 K
    (8.0, [1.5200, 0.9029, 0.4308]),  // 3600 K
    (12.0, [1.3359, 0.9400, 0.6052]), // 4300 K
    (20.0, [1.1803, 0.9663, 0.8026]), // 5200 K
    (40.0, [1.0968, 0.9778, 0.9347]), // 5900 K
];

/// The sun's colour at an altitude (degrees): linear between the knots of
/// `SUN_COLOUR_KNOTS`, held past the first and the last; luminance 1.
pub fn sun_colour(altitude_deg: f64) -> Rgb {
    let k = &SUN_COLOUR_KNOTS;
    if altitude_deg <= k[0].0 {
        return k[0].1;
    }
    for w in k.windows(2) {
        let ((a0, c0), (a1, c1)) = (w[0], w[1]);
        if altitude_deg <= a1 {
            return mix3(c0, c1, (altitude_deg - a0) / (a1 - a0));
        }
    }
    k[k.len() - 1].1
}

/// The share of the daytime fill on a wall (the hemisphere's sky lobe and the
/// ambient, averaged over four horizontal normals) that the cinematic
/// look gives to the sky's own directional irradiance (the analytic sky's
/// upper hemisphere, warm toward a low sun and blue away from it), the rest
/// of the fill scaled down (`directional_split`): `DIRECTIONAL_SKY_SHARE_LOW_SUN`
/// with the sun under 10 deg, `DIRECTIONAL_SKY_SHARE_HIGH_SUN` over 30 deg
/// (a high sun's sky lights a wall nearly as much as the ground, where the
/// rig's lobe gives the ground twice a wall's: a large share there moved the
/// light from the ground to the walls), smooth between. Default: the bound
/// probe's yield, ~3 % of a wall's fill at golden hour (the rig's constant
/// colours carry the rest).
pub const DIRECTIONAL_SKY_SHARE_LOW_SUN: f64 = 0.45;
pub const DIRECTIONAL_SKY_SHARE_HIGH_SUN: f64 = 0.15;

/// The directional sky's share of the fill at a sun altitude (degrees).
pub fn directional_sky_share(altitude_deg: f64) -> f64 {
    mix(DIRECTIONAL_SKY_SHARE_HIGH_SUN, DIRECTIONAL_SKY_SHARE_LOW_SUN, smoothstep(30.0, 10.0, altitude_deg))
}

/// The rim light's share kept under the cinematic look by day (its blue,
/// unshadowed light opposite the sun was ~55 % of a wall's fill at golden
/// hour, lighting faces deep in shade; the directional sky fill now carries
/// the light from the sky opposite the sun). Default 1.
pub const RIM_KEEP: f64 = 0.25;

/// The day exposure's adaptation exponent under the cinematic look
/// (`rig::EXPOSURE_ADAPTATION` is 0.20): the exposure opens up less as the
/// light falls toward evening (1.30 -> ~1.24 at 18.5 h; 0.12, 1.10, left the street near
/// black under the cinematic tone curve, and the final level is the tone curve's), so shade sits darker
/// and the warm key carries the frame. Default 0.20 (the JS curve).
pub const EXPOSURE_ADAPTATION: f64 = 0.18;

/// The upper sky's irradiance as 9 SH coefficients, cosine-convolved (as
/// `LookFrame::env_sh` before its yield): the dome's radiance above the
/// horizon only (`field.sample_upper`), nothing below (the ground's light
/// is the hemisphere's ground lobe and the scene's bounce). 16 x 32 samples
/// of the upper hemisphere.
pub fn upper_sky_sh(model: &SkyModel) -> [Rgb; 9] {
    let (rings, sectors) = (16, 32);
    let mut c = [[0.0; 3]; 9];
    let half_pi = 0.5 * std::f64::consts::PI;
    let dtheta = half_pi / rings as f64;
    let dphi = TAU / sectors as f64;
    for i in 0..rings {
        let theta = (i as f64 + 0.5) * dtheta;
        let (st, ct) = theta.sin_cos();
        for j in 0..sectors {
            let phi = (j as f64 + 0.5) * dphi;
            let d = [st * phi.cos(), ct, st * phi.sin()];
            let l = model.field.sample_upper(d);
            let y = crate::sh_basis(d);
            let w = st * dtheta * dphi; // the solid angle of the cell
            for k in 0..9 {
                for ch in 0..3 {
                    c[k][ch] += l[ch] * y[k] * w;
                }
            }
        }
    }
    let band = [std::f64::consts::PI, 2.0 * std::f64::consts::PI / 3.0, std::f64::consts::PI / 4.0];
    for (k, ck) in c.iter_mut().enumerate() {
        let a = band[if k == 0 { 0 } else if k < 4 { 1 } else { 2 }];
        for v in ck.iter_mut() {
            *v *= a;
        }
    }
    c
}

/// The normals the fill's energy is kept over: up and the four horizontals.
pub const FILL_NORMALS: [[f64; 3]; 5] = [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, -1.0]];

/// The directional fill's split: (`k`, `s`) with the directional sky
/// `k x upper` added to the diffuse environment (in the environment's own
/// units: the caller divides by the environment's intensity) and the
/// hemisphere's sky lobe and the ambient scaled by `s`. The directional term
/// is `share` of the walls' mean fill (the four horizontals of
/// `FILL_NORMALS`: the sky lobe at half and the ambient), and `s` keeps an
/// upward face's fill (the ground, the roofs: most of the frame) what it was
/// (a low sun's sky is brighter toward the horizon than overhead, so keeping
/// the mean over every normal took a quarter of the ground's light at golden
/// hour). The walls' mean then changes by what the directional sky gives a
/// wall over an upward face; `s` is held to 0..1. (0, 1) with no fill or no sky.
pub fn directional_split(hemi_sky: Rgb, ambient: Rgb, upper: &[Rgb; 9], share: f64) -> (f64, f64) {
    let walls = &FILL_NORMALS[1..];
    let n = walls.len() as f64;
    let fill_w = 0.5 * luminance(hemi_sky) + luminance(ambient);
    let fill_up = luminance(hemi_sky) + luminance(ambient);
    let dir_w: f64 = walls.iter().map(|d| luminance(crate::sh_eval(upper, *d))).sum::<f64>() / n;
    let dir_up = luminance(crate::sh_eval(upper, FILL_NORMALS[0]));
    if !(fill_w > 1e-12) || !(dir_w > 1e-12) {
        return (0.0, 1.0);
    }
    let k = share.clamp(0.0, 1.0) * fill_w / dir_w;
    let s = ((fill_up - k * dir_up) / fill_up).clamp(0.0, 1.0);
    (k, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sun_colour_warms_monotonically_and_continuously_toward_the_horizon() {
        let mut prev = f64::INFINITY;
        let mut last = sun_colour(-2.0);
        for i in 0..=1000 {
            let a = -2.0 + i as f64 * 0.06; // -2 .. 58 deg
            let c = sun_colour(a);
            assert!((luminance(c) - 1.0).abs() < 2e-3, "{a}: luminance {}", luminance(c));
            let rb = c[0] / c[2];
            assert!(rb <= prev + 1e-12, "{a}: red/blue {rb} rose above {prev}");
            prev = rb;
            let step = (0..3).map(|k| (c[k] - last[k]).abs()).fold(0.0, f64::max);
            assert!(step < 0.03, "{a}: a jump of {step}");
            last = c;
        }
        // the hours separate: a whiter noon, golden at golden hour, near red at the horizon
        let noon = sun_colour(50.0);
        assert!((1.1..1.25).contains(&(noon[0] / noon[2])), "noon red/blue {}", noon[0] / noon[2]);
        let golden = sun_colour(6.6);
        assert!(golden[0] / golden[2] > 4.0, "golden hour red/blue {}", golden[0] / golden[2]);
        let low = sun_colour(1.0);
        assert!(low[0] / low[2] > 15.0, "the last 2 deg red/blue {}", low[0] / low[2]);
    }

    #[test]
    fn the_directional_split_keeps_the_ground_and_about_the_walls_fill() {
        for hour in [9.0, 12.0, 16.0, 18.5] {
            let f = crate::LookFrame::new(hour);
            let up = upper_sky_sh(&f.model);
            let share = directional_sky_share(f.model.sun.altitude_deg);
            let (k, s) = directional_split(f.hemi_sky, f.ambient, &up, share);
            assert!(k > 0.0 && s > 0.0 && s < 1.0, "{hour}: {k} {s}");
            let fill = |d: &[f64; 3]| luminance(f.hemi_sky) * (0.5 * d[1] + 0.5) + luminance(f.ambient);
            // an upward face keeps its fill exactly
            let n = [0.0, 1.0, 0.0];
            let up_ratio = (s * fill(&n) + k * luminance(crate::sh_eval(&up, n))) / fill(&n);
            assert!((up_ratio - 1.0).abs() < 1e-9, "{hour}: up x{up_ratio}");
            // the directional sky is `share` of the walls' mean fill; the walls' mean within 6 % at
            // noon, within 30 % at a low sun (the sky toward a low sun lights a wall facing it more than
            // the ground: 18.5 h +27 %, nearly all of it on the faces toward the sun)
            let walls = &FILL_NORMALS[1..];
            let w_before: f64 = walls.iter().map(fill).sum();
            let w_dir: f64 = walls.iter().map(|d| k * luminance(crate::sh_eval(&up, *d))).sum();
            assert!((w_dir / w_before - share).abs() < 1e-9);
            let w_after = s * w_before + w_dir;
            let tol = if hour == 12.0 { 0.06 } else { 0.3 };
            assert!((w_after / w_before - 1.0).abs() < tol, "{hour}: walls x{}", w_after / w_before);
        }
        assert_eq!(directional_split([0.0; 3], [0.0; 3], &[[0.0; 3]; 9], 0.5), (0.0, 1.0));
        // the share by altitude: monotone, its two ends
        assert!((directional_sky_share(60.0) - DIRECTIONAL_SKY_SHARE_HIGH_SUN).abs() < 1e-12);
        assert!((directional_sky_share(5.0) - DIRECTIONAL_SKY_SHARE_LOW_SUN).abs() < 1e-12);
        let mut prev = f64::INFINITY;
        for i in 0..=400 {
            let v = directional_sky_share(i as f64 * 0.15);
            assert!(v <= prev + 1e-12);
            prev = v;
        }
    }

    #[test]
    fn the_upper_sky_is_warmer_toward_a_low_sun_than_away_from_it() {
        let f = crate::LookFrame::new(18.5);
        let up = upper_sky_sh(&f.model);
        let s = f.model.sun;
        let h = s.x.hypot(s.z);
        let (sun, anti) = ([s.x / h, 0.0, s.z / h], [-s.x / h, 0.0, -s.z / h]);
        let (a, b) = (crate::sh_eval(&up, sun), crate::sh_eval(&up, anti));
        assert!(luminance(a) > 2.0 * luminance(b), "sunward {a:?} vs away {b:?}");
        assert!(a[0] / a[2] > b[0] / b[2], "sunward {a:?} warmer than away {b:?}");
        // deterministic
        assert_eq!(upper_sky_sh(&f.model), up);
    }
}
