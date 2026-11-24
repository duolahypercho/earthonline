//! The screen-space indirect light (eo-render `--screen-gi`,
//! surfaces_ao_gi.wgsl and surfaces_ao_gi_filter.wgsl): the occlusion pass's
//! horizon search (the JS post chain's GTAO and its screen-space bounce)
//! rebuilt as a visibility-bitmask gather that reaches across a street, with
//! a temporal history and an edge-aware spatial filter, so the bounce it
//! carries is clean enough to reach that far.
//!
//! The gather (Therrien, Levesque and Gilet, "Screen Space Indirect
//! Lighting with Visibility Bitmasks", 2023): each slice through the view
//! vector splits the hemisphere over the point into `SECTORS` sectors of
//! equal cosine-weighted share (`cumulative`, from GTAO's arc integral).
//! Every sample on screen is a surface `thickness` metres deep along its
//! view ray; it hides the sectors between its front and its back
//! (`sector_bits`), and the sectors it is the first to hide take its lit
//! colour (its radiance toward the camera, for a Lambertian surface its
//! radiance toward the point too). A horizon search counts everything
//! under the highest sample as hidden, so a fire escape's rail or a lamp
//! pole hid the whole wall and sky behind it and charged that arc with the
//! rail's black (the r1i2 prototype blackened the far facades that way);
//! with a thickness, a thin rail hides only its own sliver.
//!
//! What the pass hands the lit shader is what it handed before: the
//! visible share (through the occlusion's intensity 0.9) and the bounce
//! (the hidden shares x the radiance that hides them), now out to
//! `RADIUS_M` instead of 4 m. The lit shader takes the rig's light away
//! over the hidden share and adds the bounce in its place, so a canyon's
//! floor trades the rig's assumed occluder light (ambient.rs `OCCLUDER_OVER_GROUND`,
//! inferred) for the light of the walls it sees: warm off a sunlit facade,
//! dark off a shaded one; a deck's underside takes the sunlit road's colour.
//!
//! The samples are few per frame (two slices of `STEPS` per side, turned
//! per pixel and per frame); the result is blended into a history
//! reprojected with the camera it was made with (`HISTORY_WEIGHT`, refused
//! where the depth it recorded is not this surface's, as `ao_fetch` refuses
//! it), then filtered over 5 x 5 half-resolution texels with weights that
//! fall with the distance from the point's plane (`plane_weight`), so the
//! bounce never crosses a silhouette.
//!
//! A sample's radiance counts at most what a diffuse surface of albedo
//! `MAX_DIFFUSE_ALBEDO` exits under the whole rig (`diffuse_cap`): a
//! sun glint on glass or paint is light sent one way, not to the point, and
//! as a bounce it read as fireflies.

use crate::math::*;

/// Slices through the view vector per pixel and frame (turned per pixel and per frame).
pub const SLICES: u32 = 2;
/// Samples per side of a slice, at `t^2` of the reach (dense near the point).
pub const STEPS: u32 = 12;
/// Sectors of a slice's hemisphere (the bits of a u32).
pub const SECTORS: u32 = 32;
/// The gather's reach in the world, metres: a street's width with its
/// sidewalks, a deck's height over the road, the facing wall four floors up.
pub const RADIUS_M: f64 = 12.0;
/// The reach on screen is at most this share of the frame's height: a deck's
/// underside at the top of the frame reaches the sunlit road under it in the
/// lower half.
pub const MAX_REACH_SHARE: f64 = 0.5;
/// A sample's assumed depth behind its visible surface: this plus
/// `THICKNESS_PER_M` x its distance from the eye (the depth buffer's
/// surfaces stand for more at a distance), and at least `THICKNESS_SHARE`
/// of its distance from the point: the samples thin out with the reach
/// (`t^2` steps), and a street or a wall seen between two of them is one
/// surface, not a gap of open sky; a rail 5 m off still hides a sliver.
pub const THICKNESS_M: f64 = 0.75;
pub const THICKNESS_PER_M: f64 = 0.04;
pub const THICKNESS_SHARE: f64 = 0.3;
/// The history's weight for the new frame (an exponential mean over about
/// `2 / w - 1` = 19 frames).
pub const HISTORY_WEIGHT: f64 = 0.1;
/// The spatial filter's plane tolerance: a neighbour this far (metres, plus
/// the share of the view depth) off the point's plane has weight 1/e.
pub const FILTER_PLANE_M: f64 = 0.05;
pub const FILTER_PLANE_REL: f64 = 0.01;
/// The brightest diffuse albedo a bounce takes (fresh white paint ~0.8).
pub const MAX_DIFFUSE_ALBEDO: f64 = 0.8;

/// GTAO's arc integral: the cosine-weighted share of a slice from the view
/// vector to the horizon angle `h` on one side (radians from the view
/// vector, signed by side), for the normal's angle `n` in the slice.
pub fn arc(h: f64, n: f64) -> f64 {
    0.25 * (-(2.0 * h - n).cos() + n.cos() + 2.0 * h * n.sin())
}

/// The slice's whole cosine-weighted hemisphere, `arc(n + pi/2) + arc(n - pi/2)` = `cos n + n sin n`.
pub fn total(n: f64) -> f64 {
    arc(n + std::f64::consts::FRAC_PI_2, n) + arc(n - std::f64::consts::FRAC_PI_2, n)
}

/// The cosine-weighted share of the slice from its side -1 edge (`n - pi/2`,
/// the tangent plane) to the angle `h`, as a fraction of `total(n)`: the
/// coordinate the sectors divide evenly.
pub fn cumulative(h: f64, n: f64) -> f64 {
    let lo = arc(n - std::f64::consts::FRAC_PI_2, n);
    let c = if h <= 0.0 { lo - arc(h, n) } else { lo + arc(h, n) };
    (c / total(n).max(1e-9)).clamp(0.0, 1.0)
}

/// The sectors between two cumulative coordinates (a sample's front and
/// back), as bits (sector 0 at the side -1 edge).
pub fn sector_bits(u0: f64, u1: f64) -> u32 {
    let a = (u0.min(u1) * SECTORS as f64).round().clamp(0.0, SECTORS as f64) as u32;
    let b = (u0.max(u1) * SECTORS as f64).round().clamp(0.0, SECTORS as f64) as u32;
    if b <= a {
        return 0;
    }
    let n = b - a;
    let m = if n >= 32 { u32::MAX } else { (1u32 << n) - 1 };
    m << a
}

/// A sample's thickness, metres, `eye` metres from the eye and `point` from the point.
pub fn thickness(eye: f64, point: f64) -> f64 {
    (THICKNESS_M + THICKNESS_PER_M * eye).max(THICKNESS_SHARE * point)
}

/// The share of the history after `frames` frames of the exponential mean
/// that is still the first frame's (the history starts from it).
pub fn first_frame_share(frames: u32) -> f64 {
    (1.0 - HISTORY_WEIGHT).powi(frames as i32)
}

/// The filter's weight for a neighbour `off_plane` metres off the point's
/// plane, the point `depth` metres away.
pub fn plane_weight(off_plane: f64, depth: f64) -> f64 {
    (-off_plane.abs() / (FILTER_PLANE_M + FILTER_PLANE_REL * depth)).exp()
}

/// The brightest radiance a bounce sample counts (luminance): a diffuse
/// surface of `MAX_DIFFUSE_ALBEDO` facing the key with the whole fill on it
/// (`key`, `fill` irradiance luminances, the rig's units).
pub fn diffuse_cap(key: f64, fill: f64) -> f64 {
    MAX_DIFFUSE_ALBEDO * (key + fill) / std::f64::consts::PI
}

/// The luminance the cap is taken on (Rec. 709).
pub fn luma(c: Rgb) -> f64 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{FRAC_PI_2, PI};

    #[test]
    fn the_sectors_divide_the_cosine_weighted_slice() {
        for n in [-1.2, -0.5, 0.0, 0.3, 1.0] {
            assert!((total(n) - (n.cos() + n * n.sin())).abs() < 1e-12, "{n}");
            assert!(cumulative(n - FRAC_PI_2, n).abs() < 1e-12);
            assert!((cumulative(n + FRAC_PI_2, n) - 1.0).abs() < 1e-12);
            // monotone, and the numeric integral of cos(t - n) |sin t| agrees.
            let mut last = -1.0;
            let k = 2000;
            let mut acc = 0.0;
            for i in 0..=k {
                let h = n - FRAC_PI_2 + PI * i as f64 / k as f64;
                let c = cumulative(h, n);
                assert!(c >= last - 1e-12);
                last = c;
                if i > 0 {
                    let tm = h - 0.5 * PI / k as f64;
                    acc += (tm - n).cos() * tm.sin().abs() * PI / k as f64;
                    assert!((acc / total(n) - c).abs() < 2e-3, "n {n} h {h}: {} vs {c}", acc / total(n));
                }
            }
        }
    }

    #[test]
    fn a_sample_hides_the_sectors_between_its_front_and_back() {
        assert_eq!(sector_bits(0.0, 1.0), u32::MAX);
        assert_eq!(sector_bits(1.0, 0.0), u32::MAX);
        assert_eq!(sector_bits(0.5, 0.5), 0);
        assert_eq!(sector_bits(0.0, 0.25), 0xFF);
        assert_eq!(sector_bits(0.75, 1.0), 0xFF00_0000);
        assert_eq!(sector_bits(0.5, 0.5 + 1.0 / 32.0).count_ones(), 1);
        // a thin rail 5 m off (the eye along the view vector, far off): its back is its front pushed
        // `thickness` away from the eye, so it hides a sliver, not the whole arc under it.
        let n: f64 = 0.0;
        let front: f64 = 0.6;
        let back = (5.0 * front.sin()).atan2(5.0 * front.cos() - thickness(10.0, 5.0));
        let u = |h: f64| cumulative(h, n);
        let rail = sector_bits(u(front), u(back)).count_ones();
        let horizon = sector_bits(u(front), u(FRAC_PI_2)).count_ones();
        assert!(rail >= 1 && rail < 5 && horizon > 9, "{rail} {horizon}");
    }

    #[test]
    fn the_history_settles_within_a_capture() {
        // a card renders at least twenty frames at its camera before it is read.
        assert!(first_frame_share(20) < 0.13);
        assert!((first_frame_share(0) - 1.0).abs() < 1e-12);
        assert!(plane_weight(0.0, 30.0) == 1.0);
        assert!((plane_weight(FILTER_PLANE_M + FILTER_PLANE_REL * 30.0, 30.0) - (-1.0f64).exp()).abs() < 1e-12);
    }

    #[test]
    fn the_cap_is_a_white_wall_in_full_sun() {
        // noon key (7.40, 6.60, 6.31) and about 0.94 of fill: a white wall facing the sun exits ~1.9.
        let key = luma([7.40, 6.60, 6.31]);
        let c = diffuse_cap(key, 0.94);
        assert!((c - 0.8 * (key + 0.94) / PI).abs() < 1e-12);
        assert!(c > 1.8 && c < 2.0, "{c}");
    }
}
