//! The cinematic look's night (`--look cinematic`, stage L1): the night solve
//! through the cinematic display, the lamp pools' tone under it, the fill's
//! kept tint. The default's night (tests/photometric.rs) is untouched.

use eo_look::math::luminance;
use eo_sky::{Conditions, frame, lights, night, weather};

fn cond(hour: f64, k: weather::Kind) -> Conditions {
    Conditions { hour, weather: weather::preset(k), moon_day: eo_sky::moon::DEFAULT_MOON_DAY }
}

#[test]
fn the_cinematic_night_lands_unlit_ground_low_and_lamp_pools_high_but_unclipped() {
    let c = cond(21.0, weather::Kind::Clear);
    let (f, t) = frame::look_frame(&c, None);
    let e_fill = night::ground_fill(&f, night::SIDEWALK_ENV) + night::key_on_ground(&f);
    let lamp = lights::place_street_lamps(
        &[lights::SegmentIn { highway: "primary", width: 12.0, sidewalk_left: 3.0, sidewalk_right: 3.0, points: &[[0.0, 0.0], [64.0, 0.0]] }],
        |_, _| 0.0,
    )[0];
    let peak = lights::street_lamp_lux(&lamp, 0.0);
    let mid = lights::street_lamp_lux(&lamp, 16.0);
    let k = t.lamp_units_per_lux;
    let e = night::cinematic_target_exposure(&f, &c.weather);
    let default = night::target_exposure(&f, &c.weather);
    let card = |irr: f64| night::cinematic_display_grey(0.18 * irr / std::f64::consts::PI, e) * 255.0;
    let (unlit, pool, between) = (card(e_fill), card(e_fill + peak * k), card(e_fill + mid * k));
    println!("21 h clear, cinematic: exposure {e:.3} (default {default:.3}); grey card unlit {unlit:.1}, under the lamp {pool:.1}, 16 m out {between:.1}");
    // full night: the exposure is the cinematic solve, unlit ground at its target.
    assert!((e - night::cinematic_night_exposure(&f, 0.0)).abs() < 1e-12);
    // the shift the renderer adds to a target moves only the night's share.
    for h in [12.0, 18.5, 19.25, 19.6, 20.0, 21.0, 2.0] {
        let (fh, _) = frame::look_frame(&cond(h, weather::Kind::Clear), None);
        let want = night::cinematic_target_exposure(&fh, &c.weather);
        let got = night::target_exposure(&fh, &c.weather) + night::cinematic_night_shift(&fh);
        assert!((want - got).abs() < 1e-9 * want, "{h}: {want} vs {got}");
    }
    assert!((unlit - night::cine_unlit_display_255()).abs() < 0.2, "unlit ground at {unlit}"); // (7 under `--night-lift`, else 3.5)
    // a lamp's pool lands high in the mid tones, under white; between lamps darker but readable.
    assert!(pool > 120.0 && pool < 235.0, "pool at {pool}");
    assert!(between < pool && between > unlit, "{between}");
    // the solve is deterministic and is the default's by day.
    assert_eq!(night::cinematic_night_exposure(&f, 0.0).to_bits(), night::cinematic_night_exposure(&f, 0.0).to_bits());
    let (fd, _) = frame::look_frame(&cond(12.0, weather::Kind::Clear), None);
    assert_eq!(night::cinematic_target_exposure(&fd, &c.weather), night::target_exposure(&fd, &c.weather));
}

#[test]
fn the_cinematic_night_fill_keeps_a_tint_and_its_luminance() {
    let c = cond(22.0, weather::Kind::Clear);
    let (f, t) = frame::look_frame(&c, None);
    let mut g = f;
    night::cinematic_night_fill(&mut g, t.nightness);
    let chroma = |c: [f64; 3]| (c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])) / luminance(c).max(1e-12);
    for (a, b) in [(f.hemi_sky, g.hemi_sky), (f.ambient, g.ambient)] {
        assert!((luminance(a) - luminance(b)).abs() <= 1e-9 * luminance(a).max(1e-9));
        if chroma(a) > 1e-6 {
            // 0.7 of the colour kept against 0.4: 1.75 x the chroma.
            let r = chroma(b) / chroma(a);
            assert!((r - (1.0 - night::CINE_NIGHT_FILL_DESATURATION) / (1.0 - frame::NIGHT_FILL_DESATURATION)).abs() < 1e-6, "{r}");
        }
    }
    // by day nothing moves.
    let (fd, td) = frame::look_frame(&cond(12.0, weather::Kind::Clear), None);
    let mut gd = fd;
    night::cinematic_night_fill(&mut gd, td.nightness);
    assert_eq!(gd.hemi_sky, fd.hemi_sky);
}
