//! The bridges' fidelity features (suspension.rs `BridgeFeatures`): their
//! switches, the proportions the structure scale keeps, and the night
//! values (lights.rs), on the repo's data at the default ground plan.

use eo_landmarks::LandmarksJs;
use eo_landmarks::data::num;
use eo_landmarks::suspension::{BridgeFeatures, SWITCHES};

#[test]
fn every_switch_turns_its_feature_off() {
    assert_eq!(BridgeFeatures::from_args(&["eo-app", "--shots", "x.json"]), BridgeFeatures::ALL);
    assert_eq!(BridgeFeatures::from_args(&["eo-app", "--no-bridges2"]), BridgeFeatures::OFF);
    let one = |f: &str| BridgeFeatures::from_args(&[f]);
    assert!(!one("--no-bridge-scale").scale && one("--no-bridge-scale").paint);
    assert!(!one("--no-bridge-paint").paint && one("--no-bridge-paint").wires);
    assert!(!one("--no-bridge-detail").detail && one("--no-bridge-detail").night);
    assert!(!one("--no-bridge-wires").wires && one("--no-bridge-wires").detail);
    assert!(!one("--no-bridge-night").night && one("--no-bridge-night").scale);
    for s in SWITCHES {
        assert_ne!(BridgeFeatures::from_args(&[*s]), BridgeFeatures::ALL, "{s} turns something off");
    }
}

/// With the structure scale the towers stand over their (plan-scaled) span
/// in the published proportion: the Golden Gate's 227 m towers over its
/// 1,280 m main span, the west span's 160 m towers over 704 m.
#[test]
fn the_structure_scale_keeps_the_published_proportions() {
    let js = LandmarksJs::get();
    let s = eo_data::groundscale::scale();
    let h = eo_landmarks::golden_gate::structure_scale();
    if eo_landmarks::suspension::features().scale {
        assert_eq!(h, eo_data::groundscale::height_scale());
    }
    let g = eo_landmarks::golden_gate::data(js);
    let site = eo_landmarks::golden_gate::Site::new(&g);
    let gd = &g.dimensions;
    let drawn = num(gd, "towerAboveWaterM") / site.main_span_measured;
    let published = 227.38 / 1280.16;
    println!("Golden Gate: plan x{s}, structure x{h}: tower / span {drawn:.4} (published {published:.4})");
    if h == s {
        assert!((drawn / published - 1.0).abs() < 0.03, "tower / span {drawn} vs {published}");
        assert!((num(gd, "clearanceM") / num(gd, "mainSpanM") / (67.056 / 1280.16) - 1.0).abs() < 1e-9);
    }
    // the real-size members stay real: the cable, the ropes, the railing.
    assert_eq!(num(gd, "cableDiameterM"), num(&js.golden_gate.dimensions, "cableDiameterM"));
    assert_eq!(num(gd, "suspenderDiameterM"), num(&js.golden_gate.dimensions, "suspenderDiameterM"));
    assert_eq!(num(gd, "railingM"), num(&js.golden_gate.dimensions, "railingM"));
    let b = eo_landmarks::bay_bridge::data(js);
    let span = {
        let (p, q) = (b.towers[0].centre, b.towers[1].centre);
        (q[0] - p[0]).hypot(q[1] - p[1])
    };
    let drawn = b.tower_top_above_water_m / span;
    let published = 160.0 / 704.088;
    println!("west span: tower / span {drawn:.4} (published {published:.4})");
    if h == s {
        assert!((drawn / published - 1.0).abs() < 0.05, "tower / span {drawn} vs {published}");
    }
    // the decks still meet the elevated plan's decks at their ends.
    for (k, e) in &b.deck_ends {
        assert_eq!(e.clearance, js.bay_bridge_west.deck_ends[k].clearance);
    }
}

/// The towers' floods: 24 x 400 W HPS over the legs above the deck give a
/// mean in the range floodlit landmarks are lit to (tens of lux), and the
/// LEDs' pitch matches 25,000 over 300 cables of a few tens of metres.
#[test]
fn the_night_values_are_in_their_published_range() {
    let js = LandmarksJs::get();
    let f = eo_landmarks::lights::gg_floodlit(js);
    println!("Golden Gate floods: {:.1} lux mean over the legs above the deck ({:.0} to {:.0} m)", f.lux, f.deck_y, f.top_y);
    assert!(f.lux > 5.0 && f.lux < 60.0, "{} lux", f.lux);
    assert!(f.top_y > f.deck_y + 100.0 && f.half_along > f.half_across);
    let per_cable = 25_000.0 / 300.0;
    let mean_cable_m = eo_landmarks::lights::LED_PITCH_M * per_cable;
    assert!((15.0..40.0).contains(&mean_cable_m), "{mean_cable_m} m a cable");
}
