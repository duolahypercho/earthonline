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

/// The second pass's switches: each turns its own feature off and leaves the
/// first pass's on; `--no-bridges3` is the first pass's output.
#[test]
fn the_second_pass_switches_turn_their_features_off() {
    let one = |f: &str| BridgeFeatures::from_args(&[f]);
    assert_eq!(one("--no-bridges3"), BridgeFeatures::FIRST);
    assert!(BridgeFeatures::FIRST.scale && BridgeFeatures::FIRST.wires && BridgeFeatures::FIRST.night);
    assert!(!one("--no-bridge-air").air && one("--no-bridge-air").finish);
    assert!(!one("--no-bridge-finish").finish && one("--no-bridge-finish").members);
    assert!(!one("--no-bridge-members").members && one("--no-bridge-members").floods);
    assert!(!one("--no-bridge-floods").floods && one("--no-bridge-floods").curtain);
    assert!(!one("--no-bridge-curtain").curtain && one("--no-bridge-curtain").air);
    // a first-pass switch with `--no-bridges3` still turns its own feature off.
    let both = BridgeFeatures::from_args(&["--no-bridges3", "--no-bridge-night"]);
    assert!(!both.night && !both.floods && both.wires);
}

/// Each Golden Gate tower's 24 placed lights: the 12 at the sidewalk (6 a
/// leg) and the 12 below the roadway (4 x 150, 4 x 250, 4 x 400 W HPS), the
/// sidewalk ones first; the below-roadway mean on the legs under the deck
/// (less flux over a shorter stretch of leg) is in the same floodlit range
/// as the sidewalk floods' over the legs above it.
#[test]
fn the_tower_lights_are_the_published_two_sets() {
    let js = LandmarksJs::get();
    let (floods, f) = eo_landmarks::lights::gg_tower_floods(js);
    assert_eq!(floods.len(), 24);
    assert!(floods[..12].iter().all(|x| !x.below) && floods[12..].iter().all(|x| x.below));
    let lm = eo_landmarks::lights::gg_below_lumens_per_leg();
    assert_eq!(lm, 2.0 * (16_000.0 + 27_500.0 + 50_000.0));
    let b = eo_landmarks::lights::gg_floodlit_below(js);
    println!("below the roadway: {:.1} lux mean over the legs from {:.0} to {:.0} m (above: {:.1} lux)", b.lux, b.base_y, b.under_y, f.lux);
    assert!(b.under_y < f.deck_y && b.base_y < b.under_y - 30.0);
    assert!(b.lux > 5.0 && b.lux < 60.0 && b.lux < 2.0 * f.lux, "{} lux below, {} above", b.lux, f.lux);
    for x in &floods {
        let y = x.pos[2];
        assert!(if x.below { (b.base_y..b.under_y).contains(&y) } else { y >= f.deck_y }, "{x:?}");
        let n = (x.aim[0] * x.aim[0] + x.aim[1] * x.aim[1] + x.aim[2] * x.aim[2]).sqrt();
        assert!((n - 1.0).abs() < 1e-9 && x.aim[2] > 0.99, "{x:?} aims up");
    }
}

/// `finish`: the bridges' painted steel, cables and far items are dielectric
/// films at the coat's low sheen; other landmarks keep the registry's values.
#[test]
fn the_finish_makes_the_coats_dielectric() {
    let mut m = eo_landmarks::data::Material { roughness: 0.45, metalness: 0.35, env_class: "painted-metal".into(), surface: None, draw: None, wire: true, surface_options: None };
    eo_landmarks::suspension::finish_material("bay-bridge-west", "cables", &mut m);
    if eo_landmarks::suspension::features().finish {
        assert_eq!(m.metalness, 0.0);
        assert!(m.roughness >= eo_landmarks::suspension::PAINT_ROUGHNESS);
    }
    let mut other = eo_landmarks::data::Material { roughness: 0.45, metalness: 0.35, env_class: "painted-metal".into(), surface: None, draw: None, wire: true, surface_options: None };
    eo_landmarks::suspension::finish_material("transamerica-pyramid", "cables", &mut other);
    assert_eq!((other.roughness, other.metalness), (0.45, 0.35));
}

/// The fourth pass's switches: each turns its own feature off and leaves the
/// rest on; `--no-bridges4` is the third pass's output, and `--no-bridges3`
/// and `--no-bridges2` take the fourth pass's features off with their own.
#[test]
fn the_fourth_pass_switches_turn_their_features_off() {
    let one = |f: &str| BridgeFeatures::from_args(&[f]);
    assert_eq!(one("--no-bridges4"), BridgeFeatures::THIRD);
    assert!(BridgeFeatures::THIRD.finish && BridgeFeatures::THIRD.members && BridgeFeatures::THIRD.curtain);
    let four = |f: BridgeFeatures| [f.shade2, f.dielectric, f.haunch, f.bracing, f.pylons, f.arch2, f.fender2, f.crest, f.collar];
    assert!(four(one("--no-bridges3")).iter().all(|on| !on));
    assert!(four(one("--no-bridges2")).iter().all(|on| !on));
    let names = ["--no-bridge-shade2", "--no-bridge-dielectric", "--no-bridge-haunch", "--no-bridge-bracing", "--no-bridge-pylons", "--no-bridge-arch2", "--no-bridge-fender2", "--no-bridge-crest", "--no-bridge-collar"];
    for (i, n) in names.iter().enumerate() {
        assert!(SWITCHES.contains(n), "{n} is accepted");
        let f = four(one(n));
        for (j, on) in f.iter().enumerate() {
            assert_eq!(*on, i != j, "{n}: feature {j}");
        }
        assert!(one(n).finish && one(n).members, "{n} keeps the third pass");
    }
}

/// The scale audit (rule: every published dimension the fourth pass adds is
/// drawn through the ground plan's or the structure's scale): each drawn
/// value is its real one times one of the two, and the key ones are what
/// the 1.25 plan and heights make of them.
#[test]
fn the_fourth_pass_dimensions_are_on_the_drawn_scales() {
    let (gs, h) = (eo_data::groundscale::scale(), eo_landmarks::golden_gate::structure_scale());
    let dims = eo_landmarks::golden_gate::published_dimensions();
    for (what, real, drawn) in &dims {
        println!("{what}: {real:.3} m real, {drawn:.3} m drawn");
        assert!(((drawn / real) - gs).abs() < 1e-9 || ((drawn / real) - h).abs() < 1e-9, "{what}: {drawn} is not {real} x {gs} or x {h}");
    }
    let get = |k: &str| dims.iter().find(|d| d.0.starts_with(k)).unwrap_or_else(|| panic!("{k}")).2;
    if gs == 1.25 && h == 1.25 {
        assert!((get("fender across") - 118.5).abs() < 0.1, "{}", get("fender across"));
        assert!((get("fender along") - 64.8).abs() < 0.1, "{}", get("fender along"));
        assert!((get("roadway at midspan") - 102.03).abs() < 0.05, "{}", get("roadway at midspan"));
        assert!((get("arch outer ribs") - 39.05).abs() < 0.05, "{}", get("arch outer ribs"));
        assert!((get("pylon shaft across") - 12.19).abs() < 0.01 && (get("pylon shaft along") - 13.72).abs() < 0.01);
        // the outer ribs stand 6 ft 3 in outside the cable planes (Strauss 1937), on the drawn plan.
        let g = eo_landmarks::golden_gate::data(LandmarksJs::get());
        let site = eo_landmarks::golden_gate::Site::new(&g);
        assert!((get("arch outer ribs") / 2.0 - site.half_plane - 6.25 * 0.3048 * gs).abs() < 0.05);
    }
}

/// The scale audit on the built geometry (golden_gate.rs `build`): the
/// fourth pass's published dimensions measured on what is drawn, against
/// the real values (feet, here) times the ground plan's or the structure's
/// scale, not against the list `published_dimensions` makes of them. The
/// pylon shafts and anchorage housings from their colliders, the crest from
/// the plan, the build's stats and the deck mesh, the fender from its ring
/// in the concrete mesh, the arch from its ribs in the orange mesh.
#[test]
fn the_fourth_pass_geometry_is_drawn_at_its_published_size() {
    let f = eo_landmarks::suspension::features();
    if !(f.pylons && f.crest && f.fender2 && f.arch2 && f.members) {
        return; // a run with the fourth pass off draws the third pass's sizes
    }
    let (gs, h, ft) = (eo_data::groundscale::scale(), eo_landmarks::golden_gate::structure_scale(), 0.3048);
    let paths = eo_data::DataPaths::discover().unwrap();
    let el = eo_data::ElevationGrid::load(&paths).unwrap();
    let ca = eo_data::CaStandards::load(&paths).unwrap();
    let std = eo_landmarks::suspension::DeckStandards::load(&ca).unwrap();
    let js = LandmarksJs::get();
    let id = "golden-gate-bridge";
    let shift = eo_landmarks::set::ground_scale_shift(js, id);
    let height_at = |x: f64, z: f64| el.sample(x + shift[0], z + shift[1]);
    let ground = eo_landmarks::set::ground_of(js, id, &height_at);
    let built = eo_landmarks::golden_gate::build(js, &ground, &std);
    let g = eo_landmarks::golden_gate::data(js);
    let site = eo_landmarks::golden_gate::Site::new(&g);
    let plan = eo_landmarks::golden_gate::plan(js, &site, &|x, z| ground.at(x, z));
    let w = js.sea_level_y;
    let msl_over_mllw = 0.951; // NOAA 9414290, epoch 1983-2001
    let near = |a: f64, b: f64, tol: f64, what: &str| assert!((a - b).abs() < tol, "{what}: drawn {a:.3}, published x scale {b:.3}");

    // pylons S1, S2, N1, N2: each shaft 32 x 36 ft (across, along) on the plan.
    for name in ["S1", "S2", "N1", "N2"] {
        for side in ["west", "east"] {
            let c = built.colliders.iter().find(|c| c.part == format!("{name}-{side}")).unwrap_or_else(|| panic!("{name}-{side}"));
            near(2.0 * c.hz, 32.0 * ft * gs, 1e-6, &format!("{name} shaft across"));
            near(2.0 * c.hx, 36.0 * ft * gs, 1e-6, &format!("{name} shaft along"));
        }
    }
    // the anchorage housings: 235 ft (south) and 347 ft (north) along the span.
    let housing = |i: usize| 2.0 * built.colliders.iter().find(|c| c.part == format!("anchorage-housing-{i}")).unwrap().hx;
    near(housing(0), 235.0 * ft * gs, 1e-6, "south housing length");
    near(housing(1), 347.0 * ft * gs, 1e-6, "north housing length");

    // the crest: the roadway at midspan 270.9 ft MLLW less MSL's height over MLLW, on the height scale.
    let crest = (270.9 * ft - msl_over_mllw) * h;
    near(plan.y_mid - w, crest, 1e-6, "roadway at midspan over the sea (plan)");
    near(built.stats["deckTopMid"].as_f64().unwrap() - w, crest, 0.01, "roadway at midspan over the sea (stats)");
    // the deck mesh near midspan: its roadway on the crested line, nothing on it over the railing's height.
    let deck = &built.groups.iter().find(|(k, _)| k == "deck").unwrap().1;
    let (mut on_line, mut top) = (0, f64::MIN);
    for p in deck.positions.chunks_exact(3) {
        let c = site.chainage(p[0] as f64, p[2] as f64);
        if (c - plan.l / 2.0).abs() < 30.0 {
            let dy = p[1] as f64 - plan.deck_top(c);
            on_line += usize::from(dy.abs() < 0.01);
            top = top.max(dy);
        }
    }
    println!("crest: plan {:.3} m over the sea; the deck mesh within 30 m of midspan: {on_line} vertices on the roadway line, its top {top:.3} m over it", plan.y_mid - w);
    assert!(on_line >= 8 && (0.0..3.0).contains(&top), "{on_line} vertices on the deck line, top {top} m over it");

    // the fender: the ring's top at +15 ft MLLW, 311 x 170 ft outside (across, along).
    let concrete = &built.groups.iter().find(|(k, _)| k == "concrete").unwrap().1;
    let pier = g.supports["southPier"].centre;
    let fender_top = w + (15.0 * ft - msl_over_mllw) * h;
    let (mut across, mut along, mut n) = (0.0f64, 0.0f64, 0);
    for p in concrete.positions.chunks_exact(3) {
        let (dx, dz) = (p[0] as f64 - pier[0], p[2] as f64 - pier[1]);
        if (p[1] as f64 - fender_top).abs() < 0.01 && dx.hypot(dz) < 100.0 {
            across = across.max((dx * site.across[0] + dz * site.across[1]).abs());
            along = along.max((dx * site.axis[0] + dz * site.axis[1]).abs());
            n += 1;
        }
    }
    println!("fender: {n} vertices on its top, {:.2} x {:.2} m outside", 2.0 * across, 2.0 * along);
    assert!(n >= 64, "{n} vertices at the fender's top");
    near(2.0 * across, 311.0 * ft * gs, 0.01, "fender across");
    near(2.0 * along, 170.0 * ft * gs, 0.01, "fender along");

    // the arch: its ribs' bottom chords at the springing, 42.5 and 102.5 ft apart, 319 ft 7 in from springing to springing.
    let orange = &built.groups.iter().find(|(k, _)| k == "orange").unwrap().1;
    let mid = (plan.arch_south_c + plan.arch_north_c) / 2.0;
    let half_span = (319.0 + 7.0 / 12.0) * ft * gs / 2.0;
    let spring = plan.deck_top(mid + half_span) - 185.0 * ft * h;
    let lateral = |x: f64, z: f64| {
        let c = site.chainage(x, z);
        let a = site.axis_point(c, 0.0);
        (x - a[0]) * site.across[0] + (z - a[1]) * site.across[1] - site.centre_offset(c)
    };
    let (inner, outer) = (42.5 * ft * gs / 2.0, 102.5 * ft * gs / 2.0);
    let mut bands = [Vec::new(), Vec::new()];
    let (mut c_lo, mut c_hi) = (f64::MAX, f64::MIN);
    for p in orange.positions.chunks_exact(3) {
        let (x, y, z) = (p[0] as f64, p[1] as f64, p[2] as f64);
        let c = site.chainage(x, z);
        if (c - mid).abs() > half_span + 3.0 || (y - spring).abs() > 2.0 * h {
            continue;
        }
        let s = lateral(x, z).abs();
        bands[usize::from(s > (inner + outer) / 2.0)].push(s);
        if s > (inner + outer) / 2.0 {
            (c_lo, c_hi) = (c_lo.min(c), c_hi.max(c));
        }
    }
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
    let (mi, mo) = (mean(&bands[0]), mean(&bands[1]));
    println!("arch: ribs {:.2} and {:.2} m apart ({} and {} vertices), springings {:.2} m apart", 2.0 * mi, 2.0 * mo, bands[0].len(), bands[1].len(), c_hi - c_lo);
    assert!(bands[0].len() >= 16 && bands[1].len() >= 16);
    near(2.0 * mi, 2.0 * inner, 0.05, "arch inner ribs apart");
    near(2.0 * mo, 2.0 * outer, 0.05, "arch outer ribs apart");
    // the bottom chord's ends: the springings, within the chord's own depth.
    near(c_hi - c_lo, 2.0 * half_span, 2.0 * h, "arch span");
}
