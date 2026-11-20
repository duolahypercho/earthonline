//! the waterfront (the shore-edge rule, the piers, the quay's
//! crest and the terrain press), the open-space ground and the bridges'
//! lights, on the repo's data.

use eo_landmarks::LandmarksJs;
use eo_landmarks::ground_cover::GroundCover;
use eo_landmarks::waterfront as wf;

struct World {
    elevation: eo_data::ElevationGrid,
    oracle: eo_water::shoreline::Oracle,
    beaches: Vec<eo_landmarks::shore::BeachFeature>,
    root: std::path::PathBuf,
}

fn world() -> World {
    let paths = eo_data::DataPaths::discover().unwrap();
    let elevation = eo_data::ElevationGrid::load(&paths).unwrap();
    let boundary = eo_water::load_boundary(&paths.repo_root).unwrap();
    let oracle = eo_water::shoreline::Oracle::citywide(&boundary).unwrap();
    let beaches = eo_landmarks::shore::load_beaches(&paths.repo_root).unwrap();
    World { elevation, oracle, beaches, root: paths.repo_root }
}

/// The Embarcadero between Rincon Park and the Bay Bridge (one 373 m ring
/// edge) is a quay: walled, its pieces at most 8 m, the crest on the quay's
/// top and the terrain behind the wall under the sea at the wall's foot; the
/// Ferry Plaza pier is a deck over the bay; the county line is still dropped.
#[test]
fn the_waterfront_is_built() {
    eo_data::groundscale::pin(1.0); // the JS game this compares with is 1:1
    let w = world();
    let js = LandmarksJs::get();
    let sh = &js.shore;
    let h = |x: f64, z: f64| w.elevation.sample(x, z);
    let rule = eo_landmarks::shore::BeachRule { beaches: &w.beaches, reach_m: sh.beach_wall_reach_m, wide_reach_m: sh.wide_foreshore_reach_m, wide_sources: &sh.wide_foreshore_sources };
    let rings = wf::load_pier_rings(&w.root).unwrap();
    let piers = wf::plan_piers(&rings, &w.oracle, &h, js.sea_level_y, sh.road_lift);
    let (edges, c) = wf::classify_edges(&w.oracle, &h, js.sea_level_y, &rule, &piers);
    println!("{c:?}; {} piers", piers.len());
    assert!(c.wall_m > 60_000.0 && c.pier_m > 15_000.0, "{c:?}");
    assert!(c.county_line > 0 && c.beach > 0);
    assert!(piers.len() > 80, "{} piers", piers.len());
    assert!(edges.iter().all(|e| (e.b[0] - e.a[0]).hypot(e.b[1] - e.a[1]) <= wf::EDGE_PIECE_M + 1e-9));
    // The 373 m edge from (2435, -1706) to (2692, -1435): every piece walled.
    let on_line = |e: &&wf::ShoreEdge| {
        let (ax, az, bx, bz) = (2435.1, -1706.0, 2691.7, -1435.0);
        let m = [(e.a[0] + e.b[0]) / 2.0, (e.a[1] + e.b[1]) / 2.0];
        let cross = ((bx - ax) * (m[1] - az) - (bz - az) * (m[0] - ax)).abs() / (bx - ax as f64).hypot(bz - az);
        cross < 0.5 && m[0] > ax && m[0] < bx
    };
    let quay: Vec<&wf::ShoreEdge> = edges.iter().filter(on_line).collect();
    assert!(quay.len() >= 40, "{} pieces on the Embarcadero edge", quay.len());
    let walled = quay.iter().filter(|e| e.pier.is_none()).count();
    assert!(walled * 10 >= quay.len() * 9, "{walled} of {} walled", quay.len());
    // The crest on the quay's top: over the sea and over the ground at the ring line.
    let crest = |x: f64, z: f64| wf::crest_ground(&h, x, z) + sh.road_lift + sh.crest_above_road_m;
    for e in quay.iter().step_by(5) {
        let m = [(e.a[0] + e.b[0]) / 2.0, (e.a[1] + e.b[1]) / 2.0];
        assert!(crest(m[0], m[1]) > js.sea_level_y + 1.0, "crest {} at {m:?}", crest(m[0], m[1]));
        assert!(crest(m[0], m[1]) >= h(m[0], m[1]).unwrap() + sh.road_lift);
    }
    // The press: behind a wall the terrain's 8 m cells end under the sea at the foot.
    let press = wf::ShorePress::new(js.sea_level_y, &edges, &piers);
    let e = quay[quay.len() / 2];
    let m = [(e.a[0] + e.b[0]) / 2.0 + e.land[0] * 1.0, (e.a[1] + e.b[1]) / 2.0 + e.land[1] * 1.0];
    assert!(press.press(m[0], m[1], 5.0) < js.sea_level_y, "a metre behind the wall {}", press.press(m[0], m[1], 5.0));
    let far = [m[0] + e.land[0] * 30.0, m[1] + e.land[1] * 30.0];
    assert_eq!(press.press(far[0], far[1], 5.0), 5.0, "30 m inland the ground is its own");
    // The Ferry Plaza pier (OSM way 661723975): a deck over the bay at its surveyed level.
    let fp = &sh.ferry_pier;
    let c0 = eo_landmarks::geometry::ring_centroid(&fp.outline);
    let p = piers.iter().find(|p| p.contains(c0[0], c0[1])).expect("the Ferry Plaza pier");
    assert!(p.lidar_share > 0.8 && p.deck_y > js.sea_level_y + 1.5, "deck {} lidar {}", p.deck_y, p.lidar_share);
    assert!(press.press(c0[0], c0[1], p.deck_y + 2.0) <= p.deck_y - 0.3, "the terrain stays under the deck");
    let m = wf::build_meshes(&edges, &piers, &w.oracle, &crest, js.sea_level_y, 0.42, [0.5; 3], wf::UNDER_DECK_LINEAR, [0.7; 3], [0.1; 3]);
    println!("waterfront: {} decks, {} piles; paving {} / concrete {} / under-deck {} triangles", m.decks, m.piles, m.paving.triangles(), m.concrete.triangles(), m.under_deck.triangles());
    assert!(m.decks == piers.len() && m.piles > 5_000);
    // Trace the parapet all the way from the Black Point approach to the
    // curved head. This also catches accidentally picking a neighbouring pier.
    let pier_bed = wf::ShorePress::new(js.sea_level_y, &edges, &piers).with_municipal_pier(&piers);
    assert!(pier_bed.press(-598.0, -3546.0, 3.0) < js.sea_level_y);
    assert_eq!(pier_bed.press(-598.0, -3546.0, -6.0), -6.0);
    let rail = wf::municipal_parapet(&piers).expect("Municipal Pier in the city polygons");
    for [x, z] in [[-645.5_f32, -3334.0_f32], [-598.0, -3546.0], [-437.0, -3587.5]] {
        assert!(rail.positions.chunks_exact(3).any(|p| (p[0] - x).hypot(p[2] - z) < 7.0));
    }
}

/// The open spaces' ground: a lawn in a park, a park the JS paves left to the
/// paving, beach sand by the sea at Ocean Beach, the park-tree rule's lattice.
#[test]
fn the_open_space_ground_is_the_js_rule() {
    eo_data::groundscale::pin(1.0); // the JS game this compares with is 1:1
    let w = world();
    let cover = GroundCover::load(&w.root).unwrap();
    let (greens, _) = eo_landmarks::plaza::load_areas(&w.root).unwrap();
    let centre = |id: &str| {
        let g = greens.iter().find(|g| g.id == id).unwrap();
        let n = g.polygon.len() as f64;
        g.polygon.iter().fold([0.0, 0.0], |s, p| [s[0] + p[0] / n, s[1] + p[1] / n])
    };
    // Union Square (`ground: paved`): no tone, its paving is the plaza port's.
    let us = centre("osm-green-w25278818");
    assert!(cover.tone_at(us[0], us[1]).is_none());
    // A vegetated park is lawn: grass between its two tones.
    let park = greens.iter().find(|g| g.kind == "park" && g.ground == "vegetated" && g.polygon.len() > 8).unwrap();
    let probe = (0..400).map(|k| {
        let (a, b) = (park.polygon[0], park.polygon[park.polygon.len() / 2]);
        let t = (k as f64 + 0.5) / 400.0;
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
    });
    let grass: Vec<[f64; 3]> = probe.filter_map(|p| cover.tone_at(p[0], p[1])).collect();
    assert!(!grass.is_empty(), "no tone inside {}", park.id);
    assert!(grass.iter().any(|c| c[1] > c[0] && c[1] > c[2]), "a park's lawn is green");
    // The mottle is in [0, 1] and smooth at its 48 m cells.
    for k in 0..200 {
        let (x, z) = (k as f64 * 7.3 - 700.0, k as f64 * -3.1 + 200.0);
        let m = eo_landmarks::ground_cover::ground_mottle(x, z);
        assert!((0.0..=1.0).contains(&m));
        assert!((m - eo_landmarks::ground_cover::ground_mottle(x + 0.01, z)).abs() < 1e-3);
    }
    // Ocean Beach: sand on the beach and within reach of the sea, not a kilometre inland.
    let ob = w.beaches.iter().find(|b| b.polygon.iter().any(|p| p[0] < -8000.0)).expect("Ocean Beach");
    let p = ob.polygon[ob.polygon.len() / 3];
    assert!(cover.beach_sand(p[0], p[1], 10.0));
    assert!(!cover.beach_sand(p[0] + 1200.0, p[1], 1200.0));
    // The park trees: on the lattice, inside the tile, away from what blocks them.
    let b = park.polygon.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]);
    let trees = cover.park_trees(b[0], b[1], b[2], b[3], &|_, _| 10.0, &|_, _| false);
    println!("{}: {} park trees", park.id, trees.len());
    assert!(trees.iter().all(|t| t.x >= b[0] && t.x < b[2] && t.z >= b[1] && t.z < b[3] && (t.y - 9.92).abs() < 1e-9));
    assert!(cover.park_trees(b[0], b[1], b[2], b[3], &|_, _| 10.0, &|_, _| true).is_empty());
}

/// The registry the export holds: the two bridges and (landmarks2) the five
/// landmark buildings, with live draw rules.
#[test]
fn the_registry_holds_the_bridges_and_buildings() {
    eo_data::groundscale::pin(1.0);
    let js = LandmarksJs::get();
    assert_eq!(js.registry.len(), 2 + eo_landmarks::towers::BUILDING_IDS.len());
    assert!(js.rules.near_m > 0.0 && js.rules.fog_cap_m > 0.0);
}
