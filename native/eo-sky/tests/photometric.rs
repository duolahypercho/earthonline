//! The model's light against documented photometric values (each band names
//! its source in eo-sky/src/photometry.rs), in the game's units converted by
//! the JS's own scale (1e5 lux of normal sun = 2.75 units).

use eo_look::math::luminance;
use eo_look::{LookFrame, sh_eval};
use eo_sky::photometry::{self as ph, Band};
use eo_sky::{Conditions, frame, lights, night, weather};

fn cond(hour: f64, k: weather::Kind) -> Conditions {
    Conditions { hour, weather: weather::preset(k), moon_day: eo_sky::moon::DEFAULT_MOON_DAY }
}

/// Horizontal illuminance (units): key on the horizontal (x the deck's mean
/// beam), and the fill (hemisphere sky lobe + ambient + environment diffuse).
fn horizontal(f: &LookFrame, w: &weather::WeatherState) -> (f64, f64) {
    let key = luminance(f.key) * f.key_dir[1].max(0.0) * night::mean_beam(w);
    let sh = sh_eval(&f.env_sh, [0.0, 1.0, 0.0]);
    let env = f.rig.environment_intensity * night::SIDEWALK_ENV;
    let fill = luminance([0, 1, 2].map(|i| f.hemi_sky[i] + f.ambient[i] + sh[i] * env));
    (key, fill)
}

fn report(b: &Band, v: f64) -> bool {
    let ok = b.holds(v);
    println!("  {:<70} {:>12.3} {:<6} band {:.3}..{:.3}  {}  [{}]", b.what, v, b.unit, b.lo, b.hi, if ok { "IN" } else { "OUT" }, b.source);
    ok
}

#[test]
fn daylight_against_the_illuminance_tables() {
    let c = cond(12.0, weather::Kind::Clear);
    let (f, _) = frame::look_frame(&c, None);
    let (key, fill) = horizontal(&f, &c.weather);
    println!("noon clear: key on horizontal {:.0} lux, fill {:.0} lux, sun alt {:.1} deg", ph::lux(key), ph::lux(fill), f.model.sun.altitude_deg);
    // The absolute scale is calibrated here (ph::lux_per_unit), so this one holds by construction.
    let global = report(&ph::CLEAR_NOON_GLOBAL, ph::lux(key + fill));
    println!("  (calibration: {:.0} lux per unit; the JS nominal {:.0})", ph::lux_per_unit(), ph::JS_NOMINAL_LUX_PER_UNIT);
    let ratio = report(&ph::SUN_OVER_SHADE, (key + fill) / fill);
    println!("  EV100 (incident, ISO 2720) at noon clear: {:.1} (sunny-16: 15)", ph::ev100_incident(ph::lux(key + fill)));
    let shade = report(&ph::CLEAR_NOON_SHADE, ph::lux(fill));
    let co = cond(12.0, weather::Kind::Overcast);
    let (fo, _) = frame::look_frame(&co, None);
    let (ko, fo_fill) = horizontal(&fo, &co.weather);
    let overcast = report(&ph::OVERCAST_NOON_GLOBAL, ph::lux(ko + fo_fill));
    // Grey card under the noon sun: the delivered display transform at the frame's exposure.
    let card = night::display_grey(0.18 * (key + fill) / std::f64::consts::PI, f.exposure) * 255.0;
    let card_ok = report(&ph::GREY_CARD_MIDDLE, card);
    // The JS rig's day (the rig table) and this port's weather, held to the tables.
    assert!(global && ratio && card_ok && shade && overcast, "noon outside its bands");
}

/// The props slice's real street lights carry the JS lamp practical
/// (`LAMP_PRACTICALS`: 150 cd in the JS's light units with the pool gain
/// (1 + sourceLiftMH)^2 on a source lifted 0.57 head heights, so the nadir is
/// 150 / h^2 units) on the heads `lampHead` gives (column 5.12 m, district
/// 7.4 m, arterial 8.84 m). The JS's light units are this rig's; at its
/// intensity the column's pool is ~107 lux and a grey card under it clips
/// past middle grey + 1 EV, so the sky slice keeps the JS's optics (cone,
/// penumbra, lift, reach) and sets every head's nadir to the RP-8 pool peak
/// (`LAMP_POOL_PEAK_LUX`), as the fallback's. Printed: the JS's own pools;
/// asserted: the card under each head as drawn.
#[test]
fn the_props_lamps_js_practical_against_rp8() {
    let c = cond(21.0, weather::Kind::Clear);
    let (f, t) = frame::look_frame(&c, None);
    let e_fill = night::ground_fill(&f, night::SIDEWALK_ENV) + night::key_on_ground(&f);
    let card = |units: f64| night::display_grey(0.18 * (e_fill + units) / std::f64::consts::PI, f.exposure) * 255.0;
    let mut ok = true;
    for (head, h) in [("column", 5.12), ("district", 7.4), ("arterial", 8.84)] {
        let js = 150.0 / (h * h);
        let drawn = ph::LAMP_POOL_PEAK_LUX * t.lamp_units_per_lux;
        println!(
            "  {head} head {h} m: the JS practical's nadir {js:.3} units = {:.1} lux (card {:.1}); drawn {:.1} lux (card {:.1})",
            js / t.lamp_units_per_lux,
            card(js),
            drawn / t.lamp_units_per_lux,
            card(drawn)
        );
        ok &= report(&ph::GREY_CARD_MIDDLE, card(drawn));
    }
    assert!(ok);
}

#[test]
fn night_lamps_against_rp8() {
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
    println!("21 h clear: exposure {:.2} (target {:.2}), unlit ground {:.4} units ({:.1} lux at the lamp scale)", f.exposure, t.exposure_target, e_fill, e_fill / k);
    let unlit = night::display_grey(0.18 * e_fill / std::f64::consts::PI, f.exposure) * 255.0;
    let pool = night::display_grey(0.18 * (e_fill + peak * k) / std::f64::consts::PI, f.exposure) * 255.0;
    let between = night::display_grey(0.18 * (e_fill + mid * k) / std::f64::consts::PI, f.exposure) * 255.0;
    println!("  grey card: unlit {unlit:.1} (JS solve 16), under the lamp {pool:.1}, 16 m out {between:.1}");
    let uni = report(&ph::STREET_UNIFORMITY_MAX_MIN, (e_fill / k + peak) / (e_fill / k + mid.min(peak)));
    let card = report(&ph::GREY_CARD_MIDDLE, pool);
    // the unlit card at `night::NIGHT_UNLIT_DISPLAY_255` (8, the JS solves for 16):
    // a departure the owner approved on 2026-11-17 ("Yes, darker nights"). Unlit ground
    // is lit by the night sky and a little spill, so it shows at ~0.18-0.25 x the city's
    // night sky (~26/255): 8/255. The tolerance is the one the 16 had.
    assert!((unlit - night::NIGHT_UNLIT_DISPLAY_255).abs() < 1.5, "unlit ground at {unlit}");
    assert!(uni && card);
}

#[test]
fn moon_and_twilight() {
    let m = eo_sky::moon::placement(21.0, eo_sky::moon::DEFAULT_MOON_DAY);
    let full = eo_sky::moon::FULL_ILLUMINANCE_LUX;
    let ok = report(&ph::FULL_MOON, full);
    println!("  canonical moon at 21 h: altitude {:.1} deg, illuminated {:.2}, {:.3} lux on the ground", m.altitude_deg, m.illuminated, eo_sky::moon::illuminance_lux(&m));
    assert!(ok);
    // Civil twilight: the model's sky irradiance at sun -6 deg (reported; see README).
    for h in [18.9, 19.1, 19.3, 19.5, 19.7, 21.0] {
        let (f, _) = frame::look_frame(&cond(h, weather::Kind::Clear), None);
        let (key, fill) = horizontal(&f, &weather::preset(weather::Kind::Clear));
        let sky = luminance(f.model.sky_irradiance());
        println!("  {h:.1} h sun {:+.1} deg: sky irradiance {:.2} lux, fill {:.1} lux, key {:.2} lux, exposure {:.2}", f.model.sun.altitude_deg, ph::lux(sky), ph::lux(fill), ph::lux(key), f.exposure);
    }
}

#[test]
fn weather_dims_and_exposes_as_the_js_profiles() {
    let mut last = f64::INFINITY;
    for k in [weather::Kind::Clear, weather::Kind::Overcast, weather::Kind::Rain, weather::Kind::Storm] {
        let c = cond(12.0, k);
        let (f, t) = frame::look_frame(&c, None);
        let (key, fill) = horizontal(&f, &c.weather);
        println!("noon {:<9} global {:>8.0} lux  exposure {:.3}  fill scale {:.2}", k.name(), ph::lux(key + fill), t.exposure_target, t.fill_scale);
        assert!(key + fill < last, "{} not darker than the lighter weather", k.name());
        last = key + fill;
    }
}
