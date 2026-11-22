//! `--smooth-time`'s pure parts in eo-sky: the sun and moon keys blended
//! instead of switched, the vehicle lamps ramped on, the exposure converging
//! instead of snapping, and the defaults left exactly as they were.

use eo_look::math::luminance;
use eo_sky::frame::{self, FrameOptions};
use eo_sky::night::{self, Adaptation};
use eo_sky::{Conditions, weather};

fn cond(hour: f64, moon_day: f64) -> Conditions {
    Conditions { hour, weather: weather::preset(weather::Kind::Clear), moon_day }
}

fn angle_deg(a: [f64; 3], b: [f64; 3]) -> f64 {
    let c = (a[0] * b[0] + a[1] * b[1] + a[2] * b[2]).clamp(-1.0, 1.0);
    c.acos().to_degrees()
}

/// A day of the lunar cycle on which the moon takes the key over from the
/// setting sun between `from` and `to` h, and the hour it does (the first
/// one-second step with the default `apply` keyed by the moon).
fn takeover(from: f64, to: f64) -> Option<(f64, f64)> {
    for d in 0..60 {
        let day = d as f64 * 0.5;
        let steps = ((to - from) * 3600.0) as usize;
        let mut was = None;
        for s in 0..=steps {
            let h = from + s as f64 / 3600.0;
            let (_, t) = frame::look_frame(&cond(h, day), None);
            let now = t.moon_weight > 0.5;
            if was == Some(false) && now {
                return Some((day, h));
            }
            was = Some(now);
            if s % 60 == 0 && s > 0 && !now && t.moon_key == 0.0 && t.nightness > 0.99 {
                break; // full night with no moon key: not this day
            }
        }
    }
    None
}

#[test]
fn the_moon_takes_the_key_over_without_a_step() {
    let (day, at) = takeover(18.9, 19.5).expect("some day of the cycle has the moon up at sunset");
    // one-second steps across the takeover, 3 clock minutes either side.
    let (mut hard_dir, mut soft_dir, mut hard_key, mut soft_key, mut soft_shadow) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let mut prev: Option<(eo_look::LookFrame, eo_look::LookFrame)> = None;
    let mut range = (f64::MAX, 0.0f64);
    let mut weights = vec![];
    for s in -180..=180 {
        let h = at + s as f64 / 3600.0;
        let (hard, _) = frame::look_frame(&cond(h, day), None);
        let (soft, ts) = frame::look_frame_with(&cond(h, day), None, FrameOptions::SMOOTH);
        weights.push(ts.moon_weight);
        let k = luminance(soft.key);
        range = (range.0.min(k), range.1.max(k));
        if let Some((ph, ps)) = prev {
            hard_dir = hard_dir.max(angle_deg(ph.key_dir, hard.key_dir));
            soft_dir = soft_dir.max(angle_deg(ps.key_dir, soft.key_dir));
            let rel = |a: f64, b: f64| (a - b).abs() / a.abs().max(b.abs()).max(1e-12);
            hard_key = hard_key.max(rel(luminance(ph.key), luminance(hard.key)));
            soft_key = soft_key.max(rel(luminance(ps.key), luminance(soft.key)));
            soft_shadow = soft_shadow.max((ps.shadow_strength - soft.shadow_strength).abs());
        }
        prev = Some((hard, soft));
    }
    // (the default frame before its takeover: the sun's key is its key less the moon's)
    for m in [-12.0f64, -8.0, -6.0, -4.0, -3.0, -2.0, -1.0, -0.5] {
        let h = at + m / 60.0;
        let (f, t) = frame::look_frame(&cond(h, day), None);
        let moon = luminance(eo_sky::moon::COLOR) * t.moon_intensity;
        let sun = (luminance(f.key) - moon).max(0.0);
        println!("  {m:+5.1} min: sun {:.3} deg, sun key {sun:.6}, moon key {moon:.6}, weight {:.3}", f.model.sun.altitude_deg, frame::moon_key_weight(t.moon_intensity, f.model.sun.altitude_deg, t.moon.altitude_deg));
    }
    println!("moon day {day}, takeover at {at:.4} h: key direction step hard {hard_dir:.2} deg, smooth {soft_dir:.2} deg; key step hard {:.1} %, smooth {:.1} %; shadow step smooth {soft_shadow:.4}", hard_key * 100.0, soft_key * 100.0);
    assert!(hard_dir > 20.0, "the default should switch the key's direction at the takeover ({hard_dir} deg)");
    assert!(soft_dir < 0.1 * hard_dir && soft_dir < 8.0, "the smooth key turns {soft_dir} deg in one clock second (default {hard_dir})");
    assert!(soft_key < 0.25, "the smooth key changes by {:.1} % in one clock second", soft_key * 100.0);
    assert!(soft_shadow < 0.02, "the smooth shadow strength steps by {soft_shadow}");
    // the weight never falls back over the window.
    assert!(weights.first().unwrap() < &0.05, "weights {:?} .. {:?}", weights.first(), weights.last());
    assert!(weights.windows(2).all(|w| w[1] >= w[0] - 1e-9), "the moon's weight falls back somewhere");
    // across the whole of civil twilight (`frame::MOON_KEY_TWILIGHT_DEG`, the
    // weight's span): one-second steps from the window's start until the sun
    // is half a degree past the twilight's end. The weight goes from the sun
    // to the moon (times the moon's own rise), never back, and the key never
    // steps (the same per-second bounds as at the takeover).
    let (mut weights, mut prev, mut dir, mut key, mut shadow) = (vec![], None::<eo_look::LookFrame>, 0.0f64, 0.0f64, 0.0f64);
    let mut end = None;
    for s in -180..=3 * 3600 {
        let h = at + s as f64 / 3600.0;
        let (soft, ts) = frame::look_frame_with(&cond(h, day), None, FrameOptions::SMOOTH);
        let rise = eo_look::math::smoothstep(eo_sky::moon::KEY_MIN_ALTITUDE_DEG, eo_sky::moon::KEY_MIN_ALTITUDE_DEG + 3.0, ts.moon.altitude_deg);
        weights.push((ts.moon_weight, rise));
        if let Some(p) = &prev {
            let rel = |a: f64, b: f64| (a - b).abs() / a.abs().max(b.abs()).max(1e-12);
            dir = dir.max(angle_deg(p.key_dir, soft.key_dir));
            key = key.max(rel(luminance(p.key), luminance(soft.key)));
            shadow = shadow.max((p.shadow_strength - soft.shadow_strength).abs());
        }
        let past = soft.model.sun.altitude_deg < frame::MOON_KEY_TWILIGHT_DEG[1] - 0.5;
        prev = Some(soft);
        if past {
            end = Some(s);
            break;
        }
    }
    let end = end.expect("the sun leaves civil twilight within three hours of the takeover");
    let (first, last) = (weights[0], weights[weights.len() - 1]);
    println!("civil twilight: {:.1} clock minutes from the window's start; weight {:.3} -> {:.3} (moon's rise {:.3}); per second: key turns at most {dir:.3} deg, changes {:.2} %, shadow {shadow:.5}", (end + 180) as f64 / 60.0, first.0, last.0, last.1, key * 100.0);
    assert!(first.0 < 0.05, "the moon keys before the twilight ({first:?})");
    assert!(last.0 > 0.95 * last.1, "the moon has not taken the key at the twilight's end ({last:?})");
    assert!(weights.windows(2).all(|w| w[1].0 >= w[0].0 - 1e-9 || w[1].1 < w[0].1), "the moon's weight falls back while its rise does not");
    assert!(end + 180 > 20 * 60, "the handover should take the twilight's length, not minutes ({} s)", end + 180);
    assert!(dir < 8.0 && key < 0.25 && shadow < 0.02, "a step across the twilight: {dir} deg, {key} relative, shadow {shadow}");
}

#[test]
fn the_smooth_frame_retargets_as_it_builds() {
    for kind in [weather::Kind::Clear, weather::Kind::Fog, weather::Kind::Rain] {
        for hour in [6.98, 12.0, 19.06, 19.4, 21.0] {
            let c = Conditions { hour, weather: weather::preset(kind), moon_day: 7.0 };
            let base = frame::base_frame_with(&c, Some(0.9), FrameOptions::SMOOTH);
            for t in [None, Some(0.4), Some(1.6)] {
                let (a, ta) = frame::look_frame_with(&c, t, FrameOptions::SMOOTH);
                let (b, tb) = frame::look_frame_from_with(&base, &c, t, FrameOptions::SMOOTH);
                let (na, nb) = (eo_look::blend::frame_numbers(&a), eo_look::blend::frame_numbers(&b));
                for (k, (x, y)) in na.iter().zip(&nb).enumerate() {
                    assert!((x - y).abs() <= 1e-9 * x.abs().max(y.abs()).max(1e-6), "{kind:?} {hour} {t:?}: number {k}: {x} vs {y}");
                }
                assert!((ta.exposure_target - tb.exposure_target).abs() <= 1e-9 * ta.exposure_target && (ta.moon_weight - tb.moon_weight).abs() < 1e-12, "{kind:?} {hour} {t:?}: terms");
            }
        }
    }
}

#[test]
fn the_default_frame_is_unchanged() {
    for kind in [weather::Kind::Clear, weather::Kind::Fog, weather::Kind::Storm] {
        for hour in [6.98, 12.0, 19.06, 19.4, 21.0, 23.9] {
            for day in [0.0, 7.0, 14.0] {
                let c = Conditions { hour, weather: weather::preset(kind), moon_day: day };
                let (a, ta) = frame::look_frame(&c, Some(0.7));
                let (b, tb) = frame::look_frame_with(&c, Some(0.7), FrameOptions::default());
                let bits = |f: &eo_look::LookFrame| eo_look::blend::frame_numbers(f).iter().map(|v| v.to_bits()).collect::<Vec<_>>();
                assert_eq!(bits(&a), bits(&b), "{kind:?} {hour} h day {day}");
                assert_eq!(ta.exposure_target.to_bits(), tb.exposure_target.to_bits());
                assert_eq!(ta.vehicle_lamps.to_bits(), tb.vehicle_lamps.to_bits());
                assert_eq!(ta.moon_weight.to_bits(), tb.moon_weight.to_bits());
            }
        }
    }
}

#[test]
fn the_vehicle_lamps_ramp_on() {
    let w = weather::preset(weather::Kind::Clear);
    let (mut hard, mut soft) = (0.0f64, 0.0f64);
    let mut prev: Option<(f64, f64)> = None;
    // sun altitude from 8 deg down to -4 deg in steps of 0.01 deg (about 3 clock seconds).
    for s in 0..=1200 {
        let alt = 8.0 - s as f64 * 0.01;
        let (h, sm) = (night::vehicle_lamps(alt, &w), night::vehicle_lamps_smooth(alt, &w));
        assert!(sm >= h - 1e-12, "the ramp never dims a lamp the default has on ({alt} deg)");
        if let Some((ph, ps)) = prev {
            hard = hard.max((h - ph).abs());
            soft = soft.max((sm - ps).abs());
        }
        prev = Some((h, sm));
    }
    assert!(hard > 0.3, "the default steps on ({hard})");
    assert!(soft < 0.01, "the smooth lamps step by {soft} in 0.01 deg of sun");
    // a weather floor over the threshold is unchanged.
    let fog = weather::preset(weather::Kind::Fog);
    for alt in [30.0, 3.0, -5.0] {
        assert_eq!(night::vehicle_lamps(alt, &fog), night::vehicle_lamps_smooth(alt, &fog));
    }
}

#[test]
fn a_turn_does_not_step_the_smooth_exposure() {
    let dt = 1.0 / 60.0;
    // settled at 1.0; a 30 degree turn toward a low sun drops the metered target a stop.
    let (mut hard, mut soft) = (Adaptation::default(), Adaptation::default());
    for _ in 0..240 {
        hard.step(1.0, 19.0, dt);
        soft.step_smooth(1.0, 19.0, dt, false);
    }
    // the default: the turn resets the filter (eo-render sky.rs `turned`), the next step snaps.
    hard.current = None;
    let h = hard.step(0.5, 19.0, dt);
    assert_eq!(h, 0.5, "the default snaps on a turn");
    let mut prev = soft.current.unwrap();
    let mut worst: f64 = 0.0;
    let (mut near, mut settled_at) = (None, None);
    for k in 0..120 {
        let e = soft.step_smooth(0.5, 19.0, dt, false);
        worst = worst.max((e - prev).abs());
        prev = e;
        if near.is_none() && (e - 0.5).abs() < 0.05 {
            near = Some(k);
        }
        if settled_at.is_none() && (e - 0.5).abs() < 0.01 {
            settled_at = Some(k);
        }
    }
    println!("smooth exposure: largest frame step {worst:.4} of a 0.5 change; within 10 % after {near:?} frames, 2 % after {settled_at:?}");
    assert!(worst < 0.1 * 0.5, "the smooth exposure steps by {worst} in one frame");
    assert!(near.is_some_and(|k| k < 45), "the smooth exposure is not within 10 % after 0.75 s ({near:?} frames)");
    assert!(settled_at.is_some_and(|k| k < 90), "the smooth exposure is not within 2 % after 1.5 s ({settled_at:?} frames)");
}

#[test]
fn a_clock_jump_does_not_step_the_smooth_exposure_and_a_cut_does() {
    let dt = 1.0 / 60.0;
    let mut a = Adaptation::default();
    for _ in 0..120 {
        a.step_smooth(1.0, 18.0, dt, false);
    }
    // half an hour later the target is twice as high (a darker scene).
    let e = a.step_smooth(2.0, 18.5, dt, false);
    assert!(e < 1.1, "a clock jump stepped the smooth exposure to {e}");
    let mut d = Adaptation::default();
    for _ in 0..120 {
        d.step(1.0, 18.0, dt);
    }
    assert_eq!(d.step(2.0, 18.5, dt), 2.0, "the default snaps on a clock jump");
    // a camera cut snaps either way.
    assert_eq!(a.step_smooth(3.0, 18.5, dt, true), 3.0);
}

#[test]
fn blended_terms_meet_their_keyframes() {
    let (a, ta) = frame::look_frame_with(&cond(19.0, 7.0), None, FrameOptions::SMOOTH);
    let (b, tb) = frame::look_frame_with(&cond(19.0 + 1.0 / 60.0, 7.0), None, FrameOptions::SMOOTH);
    let _ = (a, b);
    let t0 = frame::blend_terms(&ta, &tb, 0.0);
    let t1 = frame::blend_terms(&ta, &tb, 1.0);
    for (x, y) in [(t0.exposure_target, ta.exposure_target), (t1.exposure_target, tb.exposure_target), (t0.vehicle_lamps, ta.vehicle_lamps), (t1.lamps_on, tb.lamps_on), (t1.moon_weight, tb.moon_weight)] {
        assert!((x - y).abs() <= 1e-12 * y.abs().max(1.0), "{x} vs {y}");
    }
    assert_eq!(t1.moon.dir, tb.moon.dir);
}
