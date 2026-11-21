//! The photo finish's camera (stage W4, eo-look `tone`): the camera response
//! is monotone, keeps middle grey where the old chain put it, pales bright
//! colours toward white on their own hue, never returns NaN; the soft white,
//! the lens's fall-off and the sensor noise behave as their comments say.

use eo_look::math::{Rgb, linear_to_srgb};
use eo_look::tone::*;

fn sat(c: Rgb) -> f64 {
    let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
    if mx <= 0.0 { 0.0 } else { (mx - mn) / mx }
}

/// HSV hue position: (mid - min) / (max - min) and which channel is the max and which the min.
fn hue(c: Rgb) -> (f64, usize, usize) {
    let mut i = [0usize, 1, 2];
    i.sort_by(|a, b| c[*b].total_cmp(&c[*a]));
    let (mx, md, mn) = (c[i[0]], c[i[1]], c[i[2]]);
    ((md - mn) / (mx - mn).max(1e-12), i[0], i[2])
}

/// Chromaticities to test: primaries, secondaries, skin, sky, sodium, foliage, near greys.
const COLOURS: [Rgb; 10] = [
    [1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0],
    [1.0, 1.0, 0.0],
    [1.0, 0.5, 0.1],
    [0.25, 0.45, 1.0],
    [1.0, 0.62, 0.22],
    [0.3, 0.55, 0.12],
    [0.9, 0.7, 0.6],
    [1.0, 0.98, 0.95],
    [0.6, 0.62, 0.65],
];

#[test]
fn middle_grey_stays_where_the_old_chain_put_it() {
    // the neutral axis is the old chain's: the shoulder and ACES on a grey.
    for s in -12..=14 {
        let x = 0.18 * (s as f64 * 0.5).exp2();
        let n = neutral_display(x);
        let r = camera_response([x; 3]);
        for c in r {
            assert!((c - n).abs() <= 1e-15 * n.max(1.0), "grey {x}: the response of a grey is the neutral curve ({c} vs {n})");
        }
        // the old chain on a grey (exposure 1): its shoulder, then ACES with its matrices.
        let start = aces_neutral_inverse(HIGHLIGHT_START_DISPLAY);
        let peak = x / 0.6;
        let held = peak.max(start);
        let rolled = start + HIGHLIGHT_SCALE * ((held - start) / HIGHLIGHT_SCALE + 1.0).ln();
        let old = aces_filmic([x * rolled / held; 3], 1.0);
        for c in old {
            assert!((c - n).abs() < 5e-5, "grey {x}: old {c} vs neutral {n}");
        }
    }
    // photographic middle grey: 0.213 linear, sRGB 127-128.
    let m = neutral_display(0.18);
    assert!((m - 0.2131).abs() < 1e-3, "{m}");
    assert!((127.0..=128.0).contains(&(linear_to_srgb(m) * 255.0).round()));
    // through the whole display transform at the frame's centre (no fall-off there): the grey
    // lands on the old chain's display within a fraction of a level (only the dropped vibrance
    // touches the split tone's tint).
    for x in [0.02, 0.05, 0.18, 0.4] {
        let old = display([x; 3], 1.0, [0.5, 0.5]);
        let new = photo_display([x; 3], 1.0, [0.5, 0.5], [0.87, 0.49]);
        for k in 0..3 {
            assert!((old[k] - new[k]).abs() < 1.5e-3, "grey {x} channel {k}: old {} new {}", old[k], new[k]);
        }
    }
}

#[test]
fn the_response_is_monotone_in_brightness() {
    for c in COLOURS {
        let mut last = [0.0; 3];
        for i in 0..=400 {
            let k = 1e-4 * (i as f64 * 0.06).exp2(); // 1e-4 .. 1.7e3
            let r = camera_response([c[0] * k, c[1] * k, c[2] * k]);
            for ch in 0..3 {
                assert!(r[ch] >= last[ch] - 1e-12, "{c:?} x {k}: channel {ch} fell {} -> {}", last[ch], r[ch]);
                assert!((0.0..=1.0).contains(&r[ch]));
            }
            last = r;
        }
    }
    // and the neutral curve itself, over the whole range.
    let mut last = -1.0;
    for i in 0..=2000 {
        let x = 1e-6 * (i as f64 * 0.02).exp2();
        let n = neutral_display(x);
        assert!(n >= last, "{x}");
        last = n;
    }
}

#[test]
fn highlights_pale_toward_white_on_their_own_hue() {
    for c in COLOURS.iter().filter(|c| sat(**c) > 0.3) {
        let at = |ev: f64| {
            let k = 0.18 * ev.exp2() / c[0].max(c[1]).max(c[2]);
            camera_response([c[0] * k, c[1] * k, c[2] * k])
        };
        // below middle grey the colour keeps its saturation (the ratios are kept; a colour with a
        // channel under the sensor's crosstalk share pales by a fraction of a per cent).
        let low = at(-2.0);
        let mn = c[0].min(c[1]).min(c[2]) / c[0].max(c[1]).max(c[2]);
        let tol = if mn >= SENSOR_CROSSTALK { 1e-9 } else { 0.01 };
        assert!((sat(low) - sat(*c)).abs() < tol, "{c:?}: {low:?}");
        // through the shoulder it pales, never gaining saturation, and is near white high up.
        let mut last = sat(at(0.0));
        for i in 1..=40 {
            let s = sat(at(i as f64 * 0.25));
            assert!(s <= last + 1e-9, "{c:?} at +{} EV: {s} after {last}", i as f64 * 0.25);
            last = s;
        }
        assert!(sat(at(10.0)) < 0.1, "{c:?} +10 EV: {:?}", at(10.0));
        assert!(sat(at(5.0)) < 0.7 * sat(*c), "{c:?} +5 EV: {:?}", at(5.0));
        // near its own hue all the way: the same channel order, and (mid - min) / (max - min)
        // held where a per-channel curve turns it toward the primary (the toe), turned toward
        // the secondary by at most `CAMERA_HUE_SKEW` of the per-channel curve's turn (the shoulder).
        let h0 = hue(*c);
        for ev in [-3.0, -1.0, 0.0, 2.0, 4.0, 6.0, 8.0] {
            let r = at(ev);
            if sat(r) < 1e-4 {
                continue;
            }
            let h = hue(r);
            assert_eq!((h.1, h.2), (h0.1, h0.2), "{c:?} +{ev}: {r:?}");
            let k = 0.18 * f64::exp2(ev) / c[0].max(c[1]).max(c[2]);
            let per_channel = hue([c[0] * k, c[1] * k, c[2] * k].map(neutral_display)).0;
            let (turn, moved) = (per_channel - h0.0, h.0 - h0.0);
            assert!(moved >= -1e-6, "{c:?} +{ev}: turned toward the primary ({} vs {})", h.0, h0.0);
            assert!(moved <= (CAMERA_HUE_SKEW * turn).max(0.0) + 0.01, "{c:?} +{ev}: hue {} vs {} (per-channel {per_channel})", h.0, h0.0);
        }
    }
    // a bright sodium-orange source (+6 EV) comes out a warm near-white, not orange.
    let lamp = camera_response([1.0 * 11.5, 0.62 * 11.5, 0.22 * 11.5]);
    assert!(lamp[2] > 0.6 && lamp[0] - lamp[2] < 0.35, "{lamp:?}");
    // the old chain kept it orange.
    let old = aces_filmic([11.5 * 0.6 / 4.2, 0.62 * 11.5 * 0.6 / 4.2, 0.22 * 11.5 * 0.6 / 4.2], 1.0);
    assert!(sat(lamp) < sat(old), "{lamp:?} vs the old {old:?}");
}

#[test]
fn nothing_is_nan() {
    let odd = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0, 0.0, 1e-300, 1e-12, 1e300, f64::MAX, f64::MIN_POSITIVE];
    for a in odd {
        for b in odd {
            for c in [0.0, 0.5, f64::NAN, f64::INFINITY] {
                let r = camera_response([a, b, c]);
                assert!(r.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)), "{a} {b} {c}: {r:?}");
                let d = photo_display([a, b, c], 1.0, [0.1, 0.9], [0.87, 0.49]);
                assert!(d.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)), "{a} {b} {c}: {d:?}");
            }
        }
        assert!(neutral_display(a).is_finite());
    }
    assert_eq!(camera_response([0.0; 3]), [0.0; 3]);
    assert!(camera_iso(f64::NAN).is_finite());
}

#[test]
fn the_soft_white_rolls_off_instead_of_clipping() {
    let top = 1.05;
    assert_eq!(soft_white(0.5, top), 0.5);
    assert_eq!(soft_white(PHOTO_SOFT_WHITE_KNEE, top), PHOTO_SOFT_WHITE_KNEE);
    assert!((soft_white(top, top) - 1.0).abs() < 1e-12);
    let mut last = 0.0;
    for i in 0..=1100 {
        let y = i as f64 * 0.001;
        let v = soft_white(y, top);
        assert!(v >= last && v <= 1.0, "{y}");
        // slope 1 at the knee: no kink.
        if (y - PHOTO_SOFT_WHITE_KNEE).abs() < 1e-9 {
            let d = (soft_white(y + 1e-6, top) - v) / 1e-6;
            assert!((d - 1.0).abs() < 1e-3);
        }
        last = v;
    }
    // what the old grade clipped (display 0.952-1.0) keeps distinct levels.
    let a = soft_white(0.96 * top, top);
    let b = soft_white(0.99 * top, top);
    assert!(b - a > 0.005 && b < 1.0);
}

#[test]
fn the_lens_falls_off_gently_toward_the_corners() {
    // the game's 52 deg vertical field on 16:9.
    let ty = (26f64).to_radians().tan();
    let th = [ty * 16.0 / 9.0, ty];
    assert_eq!(vignette_gain([0.5, 0.5], th), 1.0);
    let corner = vignette_gain([0.0, 0.0], th);
    let ev = corner.log2();
    assert!((-0.35..-0.15).contains(&ev), "corner {corner} ({ev} EV)");
    assert!(vignette_gain([0.0, 0.5], th) > corner && vignette_gain([0.5, 0.0], th) > vignette_gain([0.0, 0.5], th));
}

#[test]
fn the_noise_grows_in_the_dark() {
    // a clear day is shot at base ISO, a lit street at night at a few hundred.
    assert_eq!(camera_iso(15.2), CAMERA_ISO_BASE);
    assert_eq!(camera_iso(7.7), CAMERA_ISO_BASE);
    assert!((camera_iso(6.6) - 100.0).abs() < 1.0);
    let night = camera_iso(4.3);
    assert!((300.0..800.0).contains(&night), "{night}");
    assert_eq!(camera_iso(-10.0), CAMERA_ISO_MAX);
    // relative noise at middle grey: under one 8-bit level by day, a few per cent at night.
    let day = noise_sigma(0.18, camera_iso(15.0)) / 0.18;
    let nite = noise_sigma(0.18, night) / 0.18;
    assert!(day < 0.008, "day {day}");
    assert!((0.012..0.04).contains(&nite), "night {nite}");
    // darker tones are relatively noisier; more ISO, more noise.
    assert!(noise_sigma(0.02, night) / 0.02 > nite);
    let mut last = 0.0;
    for ev in (-20..=30).rev() {
        let s = noise_sigma(0.18, camera_iso(ev as f64 * 0.5));
        assert!(s >= last);
        last = s;
    }
}
