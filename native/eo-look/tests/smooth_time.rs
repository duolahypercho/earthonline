//! `--smooth-time`'s pure parts in eo-look: the monotone cubic through the
//! rig's quarter-hour rows (every row met, monotone where the rows are, no
//! slope jump at a row), the linear default left exactly as it was, and the
//! blend of two keyframes.

use eo_look::blend;
use eo_look::rig::{self, Interp};
use eo_look::{LookFrame, sky};

/// Hours of the rig's rows (0..24 by 0.25).
fn row_hours() -> Vec<f64> {
    rig::table().iter().map(|r| r.hour).collect()
}

#[test]
fn the_cubic_rig_passes_through_every_row() {
    for h in row_hours() {
        // the last row (24 h) is hour 0 again.
        let (lin, cub) = (rig::rig_at(h).pack(), rig::rig_at_interp(h, Interp::MonotoneCubic).pack());
        for (k, (a, b)) in lin.iter().zip(cub.iter()).enumerate() {
            assert!((a - b).abs() <= 1e-12 * a.abs().max(1.0), "row {h} h, value {k}: linear {a}, cubic {b}");
        }
        for t in [0.0125, 0.2, 0.7, 1.2] {
            let (lin, cub) = (rig::rig_at_transfer(h, t).pack(), rig::rig_at_transfer_interp(h, t, Interp::MonotoneCubic).pack());
            for (k, (a, b)) in lin.iter().zip(cub.iter()).enumerate() {
                assert!((a - b).abs() <= 1e-12 * a.abs().max(1.0), "row {h} h, transfer {t}, value {k}: linear {a}, cubic {b}");
            }
        }
    }
}

#[test]
fn the_cubic_rig_is_monotone_between_monotone_rows() {
    let hours = row_hours();
    let mut checked = 0;
    for w in hours.windows(2) {
        let (a, b) = (rig::rig_at(w[0]).pack(), rig::rig_at(w[1] - 1e-12).pack());
        let samples: Vec<[f64; rig::RIG_VALUES]> = (0..=32).map(|s| rig::rig_at_interp(w[0] + (w[1] - w[0]) * s as f64 / 32.0 - if s == 32 { 1e-12 } else { 0.0 }, Interp::MonotoneCubic).pack()).collect();
        for c in 0..rig::RIG_VALUES {
            let (lo, hi) = (a[c].min(b[c]), a[c].max(b[c]));
            let rising = b[c] >= a[c];
            for p in samples.windows(2) {
                let d = p[1][c] - p[0][c];
                let tol = 1e-9 * hi.abs().max(1.0);
                assert!(if rising { d >= -tol } else { d <= tol }, "{}..{} h, value {c}: not monotone ({} -> {})", w[0], w[1], p[0][c], p[1][c]);
            }
            for p in &samples {
                let tol = 1e-9 * hi.abs().max(1.0);
                assert!(p[c] >= lo - tol && p[c] <= hi + tol, "{}..{} h, value {c}: {} outside the rows' {lo}..{hi}", w[0], w[1], p[c]);
            }
            checked += 1;
        }
    }
    assert!(checked > 90 * rig::RIG_VALUES);
}

#[test]
fn the_cubic_rig_has_no_slope_jump_at_a_row() {
    // one-sided slopes 1e-5 h either side of each row: the linear rig's jump
    // there is the change of secant; the cubic's is a difference quotient's error.
    let e = 1e-5;
    let (mut lin_worst, mut cub_worst) = (0.0f64, 0.0f64);
    for h in row_hours().into_iter().skip(1).take(94) {
        for interp in [Interp::Linear, Interp::MonotoneCubic] {
            let v = |x: f64| rig::rig_at_interp(x, interp).pack();
            let (l, m, r) = (v(h - e), v(h), v(h + e));
            for c in 0..rig::RIG_VALUES {
                let scale = m[c].abs().max(1e-3);
                let jump = ((r[c] - m[c]) / e - (m[c] - l[c]) / e).abs() / scale;
                match interp {
                    Interp::Linear => lin_worst = lin_worst.max(jump),
                    Interp::MonotoneCubic => cub_worst = cub_worst.max(jump),
                }
            }
        }
    }
    assert!(lin_worst > 1.0, "the linear rig should show its slope jumps (worst {lin_worst} per hour)");
    assert!(cub_worst < 1e-3 * lin_worst, "the cubic rig jumps in slope at a row: {cub_worst} per hour (linear {lin_worst})");
}

#[test]
fn the_linear_default_is_unchanged() {
    for k in 0..=24 * 12 {
        let h = k as f64 / 12.0 + 0.013;
        assert_eq!(rig::rig_at_interp(h, Interp::Linear).pack().map(f64::to_bits), rig::rig_at(h).pack().map(f64::to_bits), "{h} h");
        assert_eq!(rig::rig_at_transfer_interp(h, 0.37, Interp::Linear).pack().map(f64::to_bits), rig::rig_at_transfer(h, 0.37).pack().map(f64::to_bits), "{h} h");
    }
    for h in [6.9, 7.1, 12.0, 18.6, 19.05, 19.4, 21.0] {
        let p = sky::Profile::clear_with_look(&eo_look::look::LOOK);
        for t in [None, Some(0.6)] {
            let a = LookFrame::with_sky_profile(h, t, p);
            let b = LookFrame::with_sky_profile_interp(h, t, p, Interp::Linear);
            assert_eq!(blend::frame_numbers(&a).iter().map(|v| v.to_bits()).collect::<Vec<_>>(), blend::frame_numbers(&b).iter().map(|v| v.to_bits()).collect::<Vec<_>>(), "{h} h {t:?}");
            let (ra, rb) = (a.retarget(Some(0.3)), a.retarget_interp(Some(0.3), Interp::Linear));
            assert_eq!(blend::frame_numbers(&ra).iter().map(|v| v.to_bits()).collect::<Vec<_>>(), blend::frame_numbers(&rb).iter().map(|v| v.to_bits()).collect::<Vec<_>>(), "{h} h retarget");
        }
    }
}

#[test]
fn the_cubic_frame_retargets_as_it_builds() {
    let p = sky::Profile::clear_with_look(&eo_look::look::LOOK);
    for h in [7.1, 12.3, 19.02] {
        let full = LookFrame::with_sky_profile_interp(h, Some(0.5), p, Interp::MonotoneCubic);
        let re = LookFrame::with_sky_profile_interp(h, Some(0.9), p, Interp::MonotoneCubic).retarget_interp(Some(0.5), Interp::MonotoneCubic);
        let (a, b) = (blend::frame_numbers(&full), blend::frame_numbers(&re));
        for (k, (x, y)) in a.iter().zip(&b).enumerate() {
            assert!((x - y).abs() <= 1e-9 * x.abs().max(1e-6), "{h} h, number {k}: {x} vs {y}");
        }
    }
}

#[test]
fn a_blend_meets_its_keyframes_and_stays_between_them() {
    let p = sky::Profile::clear_with_look(&eo_look::look::LOOK);
    let (h0, h1) = (19.0, 19.0 + 1.0 / 60.0);
    let a = LookFrame::with_sky_profile_interp(h0, None, p, Interp::MonotoneCubic);
    let b = LookFrame::with_sky_profile_interp(h1, None, p, Interp::MonotoneCubic);
    let (na, nb) = (blend::frame_numbers(&a), blend::frame_numbers(&b));
    let at0 = blend::frame_numbers(&blend::frame(&a, &b, 0.0, h0));
    let at1 = blend::frame_numbers(&blend::frame(&a, &b, 1.0, h1));
    for k in 0..na.len() {
        assert!((at0[k] - na[k]).abs() <= 1e-12 * na[k].abs().max(1.0), "number {k} at t 0: {} vs {}", at0[k], na[k]);
        assert!((at1[k] - nb[k]).abs() <= 1e-12 * nb[k].abs().max(1.0), "number {k} at t 1: {} vs {}", at1[k], nb[k]);
    }
    let mid = blend::frame_numbers(&blend::frame(&a, &b, 0.5, 0.5 * (h0 + h1)));
    for k in 0..na.len() {
        let (lo, hi) = (na[k].min(nb[k]), na[k].max(nb[k]));
        let slack = 1e-6 * hi.abs().max(1.0) + (hi - lo) * 0.01;
        assert!(mid[k] >= lo - slack && mid[k] <= hi + slack, "number {k}: {} outside {lo}..{hi}", mid[k]);
    }
}

#[test]
fn slerp_keeps_unit_length_and_meets_its_ends() {
    let dirs = [[0.0, 1.0, 0.0], [0.6, 0.8, 0.0], [-0.6, -0.8, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0], [0.577, 0.577, -0.577]];
    for a in dirs {
        for b in dirs {
            assert_eq!(blend::slerp_dir(a, b, 0.0), a);
            assert_eq!(blend::slerp_dir(a, b, 1.0), b);
            let mut prev = a;
            for s in 1..=64 {
                let d = blend::slerp_dir(a, b, s as f64 / 64.0);
                let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                assert!((l - 1.0).abs() < 1e-3, "{a:?} -> {b:?} at {s}/64: length {l}");
                let step = ((d[0] - prev[0]).powi(2) + (d[1] - prev[1]).powi(2) + (d[2] - prev[2]).powi(2)).sqrt();
                assert!(step < 0.1, "{a:?} -> {b:?} at {s}/64: a step of {step}");
                prev = d;
            }
        }
    }
}

#[test]
fn the_smooth_rim_turns_over_the_top() {
    // one-second steps through dawn and dusk (sun -8 .. +2 deg): the default rim
    // turns half round in a step where the key passes overhead, the smooth one does not.
    for (from, to) in [(6.45, 7.1), (18.95, 19.65)] {
        let (mut hard, mut soft) = (0.0f64, 0.0f64);
        let steps = ((to - from) * 3600.0) as usize;
        let angle = |a: [f64; 3], b: [f64; 3]| (a[0] * b[0] + a[1] * b[1] + a[2] * b[2]).clamp(-1.0, 1.0).acos().to_degrees();
        let mut prev: Option<([f64; 3], [f64; 3])> = None;
        for k in 0..=steps {
            let h = from + k as f64 / 3600.0;
            let (r, s) = (rig::rim_direction(rig::key_direction(h)), rig::rim_direction_smooth(h));
            let len = (s[0] * s[0] + s[1] * s[1] + s[2] * s[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-6, "{h} h: rim length {len}");
            if let Some((pr, ps)) = prev {
                hard = hard.max(angle(pr, r));
                soft = soft.max(angle(ps, s));
            }
            prev = Some((r, s));
        }
        println!("{from}..{to} h: the rim's largest turn in a clock second: default {hard:.2} deg, smooth {soft:.3} deg");
        assert!(hard > 90.0, "the default rim should flip ({hard} deg)");
        assert!(soft < 0.5, "the smooth rim turns {soft} deg in a clock second");
    }
    // the same rim with the sun up or 6 degrees down.
    for h in [3.0, 6.0, 12.0, 18.0, 22.0] {
        assert_eq!(rig::rim_direction_smooth(h), rig::rim_direction(rig::key_direction(h)), "{h} h");
    }
}
