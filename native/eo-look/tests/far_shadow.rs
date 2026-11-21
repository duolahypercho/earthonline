//! The far sun shadow rings' fit and the cascade blend (eo-look `shadow`):
//!
//!  - a ring's box holds every receiver of its disc (ground to the highest
//!    roof) and every caster toward the sun that can shadow one, its texel
//!    grid stays put when the camera moves under a still sun, and its tiles
//!    partition the map;
//!  - the blend between two cascades is continuous across their split
//!    (a ramp of view depths through each split, two cascades that disagree:
//!    no step larger than the ramp's own slope allows).

use eo_look::shadow::{self, CASCADES, CascadeCamera, FAR_MAP_SIZE, FAR_RING_RADIUS_M, far_ring_radius, fit_far_ring};

fn sun(alt_deg: f64, az_deg: f64) -> [f64; 3] {
    let (a, z) = (alt_deg.to_radians(), az_deg.to_radians());
    [a.cos() * z.sin(), a.sin(), a.cos() * z.cos()]
}

fn inside(c: [f64; 3]) -> bool {
    let e = 1e-9;
    c[0] >= -1.0 - e && c[0] <= 1.0 + e && c[1] >= -1.0 - e && c[1] <= 1.0 + e && c[2] >= -e && c[2] <= 1.0 + e
}

#[test]
fn a_ring_holds_its_receivers_and_the_casters_that_reach_them() {
    let eye = [1234.5, 80.0, -987.25];
    let y = (-5.0, 330.0);
    for alt in [3.0, 8.0, 20.0, 45.0, 80.0] {
        for az in [0.0, 37.0, 135.0, 250.0] {
            let s = sun(alt, az);
            for r in FAR_RING_RADIUS_M {
                let f = fit_far_ring(eye, r, s, FAR_MAP_SIZE, y);
                // the disc's rim and centre, at the lowest ground and the highest top.
                for k in 0..64 {
                    let t = k as f64 / 64.0 * std::f64::consts::TAU;
                    for h in [y.0, y.1] {
                        for rr in [0.0, r] {
                            let p = [eye[0] + rr * t.cos(), h, eye[2] + rr * t.sin()];
                            assert!(inside(f.clip(p)), "alt {alt} az {az} r {r}: receiver {p:?} at {:?}", f.clip(p));
                        }
                    }
                }
                // a caster on the ray from a receiver toward the sun, up to the scene's top, is inside too.
                let rim = [eye[0] - r * s[0] / (s[0] * s[0] + s[2] * s[2]).sqrt().max(1e-9), y.0, eye[2] - r * s[2] / (s[0] * s[0] + s[2] * s[2]).sqrt().max(1e-9)];
                let along = (y.1 - y.0) / s[1];
                if along < shadow::FAR_CASTER_REACH_M && alt >= shadow::FAR_MIN_ALTITUDE_DEG {
                    for k in 0..=16 {
                        let d = along * k as f64 / 16.0;
                        let c = [rim[0] + s[0] * d, rim[1] + s[1] * d, rim[2] + s[2] * d];
                        let q = f.clip(c);
                        assert!(inside(q) && q[2] <= f.clip(rim)[2] + 1e-9, "alt {alt} az {az} r {r}: caster {c:?} at {q:?}");
                    }
                }
                // the texels are no coarser than the square box would give, and finer across the light at low sun.
                assert!(f.texel[0] <= 2.0 * r / FAR_MAP_SIZE as f64 * 1.01 + 1e-9);
                if alt <= 20.0 {
                    assert!(f.texel[1] < f.texel[0], "alt {alt}: the box is foreshortened up the light ({:?})", f.texel);
                }
                assert!(f.normal_bias >= 0.0 && f.depth_bias_m > 0.0 && f.depth_range > 2.0 * r * 0.0);
            }
        }
    }
}

#[test]
fn a_still_sun_keeps_the_texel_grid_wherever_the_camera_stands() {
    let s = sun(12.0, 250.0);
    let y = (0.0, 300.0);
    let a = fit_far_ring([100.0, 40.0, 200.0], 1000.0, s, FAR_MAP_SIZE, y);
    let probe = [431.7, 12.0, -55.3];
    for (dx, dz) in [(0.37, 0.0), (13.1, -7.9), (151.0, 97.0)] {
        let b = fit_far_ring([100.0 + dx, 40.0, 200.0 + dz], 1000.0, s, FAR_MAP_SIZE, y);
        assert_eq!(a.texel, b.texel);
        // the same world point falls at the same position within its texel in both maps.
        for k in 0..2 {
            let ta = (a.clip(probe)[k] * 0.5 + 0.5) * FAR_MAP_SIZE as f64;
            let tb = (b.clip(probe)[k] * 0.5 + 0.5) * FAR_MAP_SIZE as f64;
            let frac = |t: f64| t - t.floor();
            assert!((frac(ta) - frac(tb)).abs() < 1e-6, "axis {k}: {ta} vs {tb}");
        }
    }
    assert!(a.moved_m([100.0 + 3.0, 0.0, 200.0 + 4.0]) - 5.0 < 1e-9);
    assert!(a.sun_turn_deg(s) < 1e-6 && (a.sun_turn_deg(sun(13.0, 250.0)) - 1.0).abs() < 1e-6);
}

#[test]
fn the_tiles_partition_the_map_and_the_box() {
    let f = fit_far_ring([0.0, 0.0, 0.0], 1000.0, sun(25.0, 200.0), FAR_MAP_SIZE, (0.0, 200.0));
    let mut area = 0u64;
    for k in 0..4 {
        let (rect, planes) = f.tile(k, 2);
        area += rect[2] as u64 * rect[3] as u64;
        assert_eq!((rect[2], rect[3]), (FAR_MAP_SIZE / 2, FAR_MAP_SIZE / 2));
        // the tile's centre texel is inside its planes, the other tiles' centres are not.
        for j in 0..4 {
            let (r, _) = f.tile(j, 2);
            let (u, v) = ((r[0] as f64 + 0.5 * r[2] as f64) / FAR_MAP_SIZE as f64, (r[1] as f64 + 0.5 * r[3] as f64) / FAR_MAP_SIZE as f64);
            // back to world: solve the cast rows for clip (2u - 1, 1 - 2v, 0.5).
            let m = f.cast_rows();
            let want = [2.0 * u - 1.0, 1.0 - 2.0 * v, 0.5];
            let a = [[m[0][0], m[0][1], m[0][2]], [m[1][0], m[1][1], m[1][2]], [m[2][0], m[2][1], m[2][2]]];
            let b = [want[0] - m[0][3], want[1] - m[1][3], want[2] - m[2][3]];
            // the rows are orthogonal (an orthographic light frame): p = sum(b_i a_i / |a_i|^2).
            let mut p = [0.0; 3];
            for i in 0..3 {
                let l2 = a[i][0] * a[i][0] + a[i][1] * a[i][1] + a[i][2] * a[i][2];
                for c in 0..3 {
                    p[c] += b[i] * a[i][c] / l2;
                }
            }
            let ins = planes.iter().all(|pl| pl[0] as f64 * p[0] + pl[1] as f64 * p[1] + pl[2] as f64 * p[2] + pl[3] as f64 >= -1e-3);
            assert_eq!(ins, j == k, "tile {k}: centre of tile {j} at {p:?}");
        }
    }
    assert_eq!(area, FAR_MAP_SIZE as u64 * FAR_MAP_SIZE as u64);
}

#[test]
fn the_blend_is_continuous_across_every_split() {
    let cam = CascadeCamera { eye: [0.0, 1.7, 0.0], forward: [0.0, 0.0, -1.0], right: [1.0, 0.0, 0.0], up: [0.0, 1.0, 0.0], fov_y_deg: 52.0, aspect: 16.0 / 9.0, near: 0.5, far: 4200.0 };
    let fit = shadow::fit_cascades(&cam, sun(30.0, 200.0), 3072);
    let splits = [fit[0].split_far, fit[1].split_far, fit[2].split_far];
    // two cascades that disagree everywhere: cascade c says vis_c(vd).
    let vis = |c: usize, vd: f64| [0.2, 0.7, 0.45][c] + 0.001 * vd * (c as f64 + 1.0);
    let blended = |vd: f64| {
        let (c, w) = shadow::cascade_blend_weight(vd, splits);
        if c + 1 < CASCADES { vis(c, vd) + (vis(c + 1, vd) - vis(c, vd)) * w } else { vis(c, vd) }
    };
    let unblended = |vd: f64| vis(shadow::cascade_blend_weight(vd, splits).0, vd);
    let step = 1e-3;
    for (s, split) in splits.iter().take(CASCADES - 1).enumerate() {
        // across the split itself: the hard switch jumps by the two cascades' difference, the blend does not.
        let (a, b) = (split - step * 0.5, split + step * 0.5);
        let hard = (unblended(b) - unblended(a)).abs();
        let soft = (blended(b) - blended(a)).abs();
        assert!(hard > 0.1, "split {s}: the test's cascades disagree ({hard})");
        assert!(soft < 1e-3, "split {s} at {split:.2} m: blended step {soft}");
        // through the whole band: no step past what a smoothstep over the band allows.
        let start = if s == 0 { 0.0 } else { splits[s - 1] };
        let band = (split - start) * shadow::CASCADE_BLEND_BAND;
        let max_slope = 1.5 / band * 0.6 + 0.01; // smoothstep's peak slope x the cascades' difference (< 0.6) + the ramps
        let mut vd = split - band - 1.0;
        while vd < split + 1.0 {
            let d = (blended(vd + step) - blended(vd)).abs();
            assert!(d <= max_slope * step + 1e-9, "split {s}: step {d} at {vd:.3} m");
            vd += step;
        }
        // outside the band the blend is the cascade itself, bit for bit.
        for vd in [start + 0.5 * (split - start) * (1.0 - shadow::CASCADE_BLEND_BAND), split - band - 1e-6] {
            assert_eq!(blended(vd).to_bits(), unblended(vd).to_bits(), "split {s}: {vd} m before the band");
        }
    }
    // the last cascade hands over to nothing.
    assert_eq!(shadow::cascade_blend_weight(splits[2] - 1.0, splits), (2, 0.0));
}

#[test]
fn ring_1_grows_with_the_eyes_height_and_does_not_flicker_at_a_step() {
    let far = FAR_RING_RADIUS_M[1]; // the game's far plane
    // a fresh map takes the radius of its height; ring 2 always reaches the far plane.
    for (h, want) in [(1.7, 1000.0), (60.0, 1000.0), (149.0, 1000.0), (150.0, 2000.0), (299.0, 2000.0), (300.0, far), (640.0, far), (18000.0, far)] {
        assert_eq!(far_ring_radius(0, h, far, None), want, "{h} m");
        assert_eq!(far_ring_radius(1, h, far, None), far);
    }
    // a longer far plane (the bench's overview from 18 km, far 27 km): the ring keeps growing with the height, to the far plane.
    assert_eq!(far_ring_radius(0, 700.0, 27000.0, None), 8000.0);
    assert_eq!(far_ring_radius(0, 18000.0, 27000.0, None), 27000.0);
    assert_eq!(far_ring_radius(1, 10.0, 27000.0, None), 27000.0);
    // an eye hovering round a step keeps the map's radius until it is a tenth past it.
    let mut r = far_ring_radius(0, 140.0, far, None);
    let mut changes = 0;
    for k in 0..200 {
        let h = 150.0 + 12.0 * (k as f64 * 0.7).sin();
        let n = far_ring_radius(0, h, far, Some(r));
        changes += (n != r) as u32;
        r = n;
    }
    assert_eq!(changes, 0, "a 12 m wobble round 150 m redraws nothing");
    assert_eq!(far_ring_radius(0, 166.0, far, Some(1000.0)), 2000.0);
    assert_eq!(far_ring_radius(0, 134.0, far, Some(2000.0)), 1000.0);
    assert_eq!(far_ring_radius(0, 140.0, far, Some(2000.0)), 2000.0);
    // a jump goes straight to its radius.
    assert_eq!(far_ring_radius(0, 900.0, far, Some(1000.0)), far);
    assert_eq!(far_ring_radius(0, 2.0, far, Some(far)), 1000.0);
}
