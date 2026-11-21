//! Tests against the real files in public/data/sf/. Expected numbers were read
//! from the data itself through the JavaScript loaders (src/citygen/sf-data.js
//! merge rule, `decodeSfElevation` + `createSfElevationSampler` with
//! `approachFills: false`, `decodeLandCover`), run in node on 2026-11-13.

use eo_data::standards::{self, CaStandards};
use eo_data::*;
use std::sync::OnceLock;

fn paths() -> DataPaths {
    DataPaths::discover().expect("repo data")
}

fn city() -> &'static City {
    static C: OnceLock<City> = OnceLock::new();
    C.get_or_init(|| City::load(&paths()).expect("city loads"))
}

#[test]
fn city_counts_match_meta_and_js_merge() {
    eo_data::groundscale::pin(1.0); // the JS game this compares with is 1:1
    let c = city();
    // meta.counts, as written by the atlas builder.
    assert_eq!(c.meta.counts.roads, 62025);
    assert_eq!(c.roads.len(), c.meta.counts.roads);
    assert_eq!(c.meta.counts.detail_buildings, 28000);
    assert_eq!(c.signals, c.meta.counts.signals);
    assert_eq!(c.signals, 1877);
    // fetchSfSources merge (footprint pack enabled), reproduced in node.
    let m = &c.merge;
    assert_eq!(m.extract_records, 28000);
    assert_eq!(m.pack_records, 142499);
    assert_eq!(m.supplement_records, 276);
    assert_eq!(m.pack_dropped_for_supplement, 142499 - 142224);
    assert_eq!(m.superseded_ids, 74);
    assert_eq!(m.extract_dropped_superseded, 28000 - 27926);
    assert_eq!(m.dropped_not_at_grade + m.dropped_landmark_excluded, 170426 - 170415);
    assert_eq!(m.dropped_degenerate, 0);
    assert_eq!(c.buildings.len(), 170415);
}

#[test]
fn building_heights_match_js_resolution() {
    eo_data::groundscale::pin(1.0); // the JS game this compares with is 1:1
    let c = city();
    let sum: f64 = c.buildings.iter().map(|b| b.height as f64).sum();
    // sum over the merged set of Math.max(3, resolveSourceHeight(b, sourceFootprintArea(b))).
    assert!((sum - 1318140.7).abs() < 2.0, "sum of heights {sum}");
    let tp = c.buildings.iter().find(|b| b.id == "24222973").expect("Transamerica Pyramid record");
    assert_eq!(tp.height, 260.0);
    assert!(c.buildings.iter().all(|b| b.height >= 3.0));
    let verts: usize = c.buildings.iter().map(|b| b.ring.len()).sum();
    // 1,645,873 listed vertices, minus the closing duplicate of the 6 closed rings.
    assert_eq!(verts, 1645873 - 6);
}

#[test]
fn vehicle_ways_and_lane_widths() {
    eo_data::groundscale::pin(1.0); // the JS game this compares with is 1:1
    let c = city();
    let vehicle = c.roads.iter().filter(|r| standards::is_vehicle_way(&r.highway)).count();
    assert_eq!(vehicle, 22478);
    let ca = CaStandards::load(&paths()).unwrap();
    assert_eq!(ca.metric(standards::CITY_LANE_ID).unwrap(), 3.048);
    assert_eq!(ca.metric(standards::STATE_LANE_ID).unwrap(), 3.658);
    let mut motorway = c.roads.iter().find(|r| r.highway == "motorway").unwrap().clone();
    assert_eq!(ca.class_lane_width(&motorway).unwrap(), 3.66);
    motorway.highway = "residential".into();
    assert_eq!(ca.class_lane_width(&motorway).unwrap(), 3.05);
    assert_eq!(standards::parse_speed_limit_kmh("25 mph"), 40);
    assert_eq!(standards::parse_speed_limit_kmh("US:urban"), 0);
}

#[test]
fn elevation_samples_match_js_sampler() {
    eo_data::groundscale::pin(1.0); // the JS game this compares with is 1:1
    let g = ElevationGrid::load(&paths()).unwrap();
    assert_eq!((g.meta.width, g.meta.height, g.meta.cell_size), (2431, 2187, 8.0));
    for (x, z, want) in [
        (1539.0, -366.0, 3.563812606036663),
        (-1500.0, 1500.0, 83.3010025024414),
        (0.0, 0.0, 16.205999851226807),
        (-7000.0, -2000.0, -18.109),
        (3000.0, -4000.0, -35.4415),
        (-9000.0, 7000.0, -15.294),
        (5000.0, -1000.0, -20.254),
    ] {
        let got = g.sample(x, z).unwrap();
        assert!((got - want).abs() < 1e-3, "({x},{z}) got {got} want {want}");
    }
    assert!(g.sample(-20000.0, 0.0).is_none());
}

#[test]
fn land_cover_matches_js_decoder() {
    eo_data::groundscale::pin(1.0); // the JS game this compares with is 1:1
    let lc = LandCover::load(&paths()).unwrap();
    assert_eq!((lc.width, lc.height), (2162, 1922));
    for (x, z, want) in [(1539.0, -366.0, 50), (-1500.0, 1500.0, 50), (-6000.0, 0.0, 50), (-4000.0, -3500.0, 80), (0.0, 0.0, 50)] {
        assert_eq!(lc.class_at(x, z), want, "({x},{z})");
    }
}

/// Every way keeps its OSM `layer` (the grade-separated roads' order), in the
/// renderer's `Road` and the simulation's `SimRoad` alike. Counts read from
/// sf-city.json.gz itself (every `layer` there is a numeric string).
#[test]
fn roads_keep_their_layer() {
    eo_data::groundscale::pin(1.0); // the JS game this compares with is 1:1
    let c = city();
    let mut by_layer = std::collections::BTreeMap::<i32, usize>::new();
    for r in &c.roads {
        *by_layer.entry(r.layer).or_default() += 1;
    }
    assert_eq!(by_layer.into_iter().collect::<Vec<_>>(), vec![(-2, 4), (-1, 160), (0, 61023), (1, 684), (2, 89), (3, 49), (4, 15), (5, 1)]);
    let sim = eo_data::sim_source::SimSource::load(&paths()).unwrap();
    assert_eq!(sim.roads.len(), c.roads.len());
    let layer: std::collections::HashMap<&str, i32> = c.roads.iter().map(|r| (r.id.as_str(), r.layer)).collect();
    for r in &sim.roads {
        assert_eq!(layer.get(r.id.as_str()), Some(&r.layer), "{}", r.id);
    }
}
