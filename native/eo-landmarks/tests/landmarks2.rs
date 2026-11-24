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
    let mut curve = edges.clone();
    let mut tally = c.clone();
    wf::align_municipal_edges(&mut curve, &mut tally, &piers);
    assert_eq!(tally.pier + tally.wall, c.pier + c.wall);
    let pier_bed = wf::ShorePress::new(js.sea_level_y, &curve, &piers).with_municipal_pier(&piers);
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

/// The beach sand fitted to the city shore (`beach_sand_fit`): Aquatic Park's
/// mapped lawns behind the beach and the Jefferson Street quay east of the
/// Hyde Street Pier are no longer sand (the JS reach rule spreads it there),
/// the beach itself still is, and the sand round Ocean Beach and in Crissy
/// Field's dune scrub stays.
#[test]
fn the_beach_sand_fits_the_city_shore() {
    eo_data::groundscale::pin(1.0);
    let w = world();
    let cover = GroundCover::load(&w.root).unwrap();
    let (greens, _) = eo_landmarks::plaza::load_areas(&w.root).unwrap();
    let shore = |x: f64, z: f64| w.oracle.edge_distance(x, z, 400.0);
    let land = |x: f64, z: f64| w.oracle.is_land(x, z);
    // Aquatic Park: the beach polygon is sand either way.
    let ap = w.beaches.iter().find(|b| b.osm.ends_with("224941181")).expect("Aquatic Park beach");
    let n = ap.polygon.len() as f64;
    let c = ap.polygon.iter().fold([0.0, 0.0], |s, p| [s[0] + p[0] / n, s[1] + p[1] / n]);
    let inside = ap.polygon.iter().map(|p| [(p[0] + c[0]) / 2.0, (p[1] + c[1]) / 2.0]).find(|p| eo_landmarks::waterfront::point_in_ring(&ap.polygon, p[0], p[1])).expect("a point inside the beach");
    assert!(cover.beach_sand_fit(inside[0], inside[1], &shore));
    // Its lawns (the Maritime Garden's grass, mapped inside the park behind
    // the beach): sand under the JS rule, lawn now.
    let mut lawn = 0;
    for g in greens.iter().filter(|g| g.kind == "grass" && g.polygon.iter().all(|p| p[0] > -560.0 && p[0] < -100.0 && p[1] > -3300.0 && p[1] < -3100.0)) {
        let n = g.polygon.len() as f64;
        let p = g.polygon.iter().fold([0.0, 0.0], |s, q| [s[0] + q[0] / n, s[1] + q[1] / n]);
        if !eo_landmarks::waterfront::point_in_ring(&g.polygon, p[0], p[1]) || !cover.beach_sand(p[0], p[1], shore(p[0], p[1])) {
            continue;
        }
        assert!(!cover.beach_sand_fit(p[0], p[1], &shore), "{} is a lawn, not sand", g.id);
        lawn += 1;
    }
    assert!(lawn >= 3, "only {lawn} Aquatic Park lawns lay under the JS sand");
    // The Jefferson Street quay east of the Hyde Street Pier: the sea's edge is
    // the harbour's seawall, the beach off to the side.
    let mut quay = 0;
    for k in 0..40 {
        let (x, z) = (-30.0 + k as f64 * 2.5, -3338.0);
        if !land(x, z) || !cover.beach_sand(x, z, shore(x, z)) {
            continue;
        }
        assert!(!cover.beach_sand_fit(x, z, &shore), "({x}, {z}) on the quay is sand");
        quay += 1;
    }
    assert!(quay >= 5, "only {quay} quay points lay under the JS sand");
    // Ocean Beach: the band round the polygon keeps (nearly all) its sand.
    let ob = w.beaches.iter().find(|b| b.polygon.iter().any(|p| p[0] < -8000.0)).expect("Ocean Beach");
    let (mut was, mut kept) = (0, 0);
    for (i, p) in ob.polygon.iter().enumerate().step_by(3) {
        let q = ob.polygon[(i + 1) % ob.polygon.len()];
        let (dx, dz) = (q[0] - p[0], q[1] - p[1]);
        let l = dx.hypot(dz).max(1e-6);
        for side in [-1.0, 1.0] {
            for off in [20.0, 50.0, 80.0] {
                let (x, z) = (p[0] - dz / l * off * side, p[1] + dx / l * off * side);
                if !land(x, z) || eo_landmarks::waterfront::point_in_ring(&ob.polygon, x, z) || !cover.beach_sand(x, z, shore(x, z)) {
                    continue;
                }
                was += 1;
                kept += cover.beach_sand_fit(x, z, &shore) as usize;
            }
        }
    }
    println!("Ocean Beach band: {kept} of {was} sampled sand points kept");
    assert!(was >= 20 && kept * 10 >= was * 9, "Ocean Beach lost its sand: {kept} of {was}");
    // Crissy Field: the dune scrub mapped behind the beach is wild cover, still sand.
    let (mut was, mut kept) = (0, 0);
    for g in greens.iter().filter(|g| g.kind == "scrub" && g.polygon.iter().all(|p| p[0] > -4500.0 && p[0] < -3000.0 && p[1] > -3250.0 && p[1] < -2950.0)) {
        for p in &g.polygon {
            let n = g.polygon.len() as f64;
            let c = g.polygon.iter().fold([0.0, 0.0], |s, q| [s[0] + q[0] / n, s[1] + q[1] / n]);
            let (x, z) = ((p[0] * 3.0 + c[0]) / 4.0, (p[1] * 3.0 + c[1]) / 4.0);
            if !land(x, z) || !cover.beach_sand(x, z, shore(x, z)) {
                continue;
            }
            was += 1;
            kept += cover.beach_sand_fit(x, z, &shore) as usize;
        }
    }
    println!("Crissy Field dune scrub: {kept} of {was} sampled sand points kept");
    assert!(was >= 10 && kept * 10 >= was * 9, "Crissy Field's dunes lost their sand: {kept} of {was}");
    // The share of Aquatic Park (the mapped park, beach and all) the two rules paint as sand.
    let park = greens.iter().find(|g| g.id.ends_with("w32839638")).expect("Aquatic Park");
    let b = park.polygon.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]);
    let (mut n, mut js, mut fit) = (0, 0, 0);
    let mut z = b[1];
    while z < b[3] {
        let mut x = b[0];
        while x < b[2] {
            if eo_landmarks::waterfront::point_in_ring(&park.polygon, x, z) && land(x, z) {
                n += 1;
                js += cover.beach_sand(x, z, shore(x, z)) as usize;
                fit += cover.beach_sand_fit(x, z, &shore) as usize;
            }
            x += 4.0;
        }
        z += 4.0;
    }
    println!("Aquatic Park drawn as sand: JS rule {:.0} %, fitted {:.0} % ({n} points)", 100.0 * js as f64 / n as f64, 100.0 * fit as f64 / n as f64);
    // (85 % and 46 % on the 2026-11 data: the beach, and the promenade behind it the park alone maps)
    assert!(fit * 5 < js * 3, "the fitted rule still paints {fit} of Aquatic Park's {n} points sand (JS {js})");
}

/// The harbours' floating docks (floating_docks.rs) from the map's pier lines:
/// Hyde Street Harbor's two main walks and their fingers are floats, the
/// Aquatic Park breakwater (a decked pier) is not, and the docks offer
/// berths a boat can lie in, over open water and clear of every float.
#[test]
fn the_harbour_docks_float_and_offer_berths() {
    use eo_landmarks::floating_docks as fd;
    eo_data::groundscale::pin(1.0);
    let w = world();
    let js = LandmarksJs::get();
    let h = |x: f64, z: f64| w.elevation.sample(x, z);
    let rings = wf::load_pier_rings(&w.root).unwrap();
    let piers = wf::plan_piers(&rings, &w.oracle, &h, js.sea_level_y, js.shore.road_lift);
    let lines = fd::load_pier_lines(&w.root).unwrap();
    let docks = fd::plan_docks(&lines, &w.oracle, &piers);
    // Hyde Street Harbor, between the Hyde Street Pier and Pier 45.
    let in_harbour = |p: [f64; 2]| p[0] > -230.0 && p[0] < -30.0 && p[1] > -3600.0 && p[1] < -3450.0;
    let here: Vec<&fd::Dock> = docks.iter().filter(|d| d.line.iter().all(|p| in_harbour(*p))).collect();
    let (mains, fingers) = (here.iter().filter(|d| d.main).count(), here.iter().filter(|d| !d.main).count());
    println!("{} docks citywide; Hyde Street Harbor: {mains} main walks, {fingers} fingers", docks.len());
    assert!(mains >= 2 && fingers >= 15, "{mains} main walks, {fingers} fingers");
    // Every float is over open water and off the decked piers.
    for d in &docks {
        let m = d.line[d.line.len() / 2];
        assert!(w.oracle.is_water(m[0], m[1]), "a float at {m:?} on land");
        assert!(d.line.len() >= 2 && (fd::FINGER_W_M..=fd::MAIN_WALK_W_M).contains(&d.width));
    }
    let t = std::time::Instant::now();
    let berths = fd::plan_berths(&docks, &|x, z| w.oracle.is_water(x, z));
    println!("plan_berths: {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
    let here_b: Vec<&fd::Berth> = berths.iter().filter(|b| in_harbour(b.centre)).collect();
    println!("{} berths citywide, {} in Hyde Street Harbor", berths.len(), here_b.len());
    assert!(here_b.len() >= 20, "{} berths in Hyde Street Harbor", here_b.len());
    assert!(berths.iter().all(|b| b.beam >= 1.0 && b.length > 2.0 && w.oracle.is_water(b.centre[0], b.centre[1])));
    if let Ok(p) = std::env::var("EO_DUMP_BERTHS") {
        let v: Vec<serde_json::Value> = berths.iter().map(|b| serde_json::json!({"c": b.centre, "h": b.heading, "l": b.length, "b": b.beam, "f": b.finger})).collect();
        let d: Vec<serde_json::Value> = docks.iter().map(|d| serde_json::json!({"line": d.line, "w": d.width})).collect();
        std::fs::write(p, serde_json::to_string(&serde_json::json!({"berths": v, "docks": d})).unwrap()).unwrap();
    }
}

/// (`--quay-fill`) The promenade is carried on from the quay apron to a walk
/// standing beyond a sunken strip: a ramp (out on the apron where the walk is
/// much higher), a terrace with a step face where no ramp fits; nothing where the ground is
/// not sunken, where a building stands in the way, or where no walk is near.
#[test]
fn the_quay_fill_paves_only_a_sunken_strip_up_to_the_walk() {
    use std::sync::Arc;
    let edge = wf::ShoreEdge { a: [0.0, 0.0], b: [40.0, 0.0], land: [0.0, 1.0], pier: None };
    let set_with = || {
        let mut s = eo_landmarks::LandmarkSet::default();
        s.press = Some(Arc::new(wf::ShorePress::new(0.0, &[edge.clone()], &[])));
        s
    };
    let js = LandmarksJs::get();
    let crest = 1.0 + js.shore.road_lift + js.shore.crest_above_road_m;
    let height = |_: f64, _: f64| Some(1.0);
    let w0 = wf::CAP_W_M + wf::COPING_LAND_M;
    let walk_from = w0 + 5.0;
    let names = |s: &eo_landmarks::LandmarkSet| s.structures.iter().map(|x| x.name.clone()).collect::<Vec<_>>();
    // a walk 5 m past the apron, its edge level with the crest: a ramp, no step
    let mut s = set_with();
    let (filled, tried) = s.add_quay_fill(&height, &|_, z| (z >= walk_from).then_some(crest + 0.2), &|_, _| false, &|_, _| crest - 1.0, 0.083);
    assert!(filled > 0 && filled == tried, "{filled} of {tried}");
    assert_eq!(names(&s), vec!["waterfront:quay-fill".to_string()]);
    // the walk 0.7 m over the crest: the ramp starts out on the apron, still no step
    let mut s = set_with();
    s.add_quay_fill(&height, &|_, z| (z >= walk_from).then_some(crest + 0.7), &|_, _| false, &|_, _| crest - 1.0, 0.083);
    assert_eq!(names(&s), vec!["waterfront:quay-fill".to_string()]);
    // the walk 0.9 m over the crest a metre past the apron: no ramp fits, a terrace at the walk and a step face down to the apron
    let mut s = set_with();
    s.add_quay_fill(&height, &|_, z| (z >= w0 + 1.0).then_some(crest + 0.9), &|_, _| false, &|_, _| crest - 1.0, 0.083);
    assert_eq!(names(&s), vec!["waterfront:quay-fill".to_string(), "waterfront:quay-fill-steps".to_string()]);
    // the ground between at the paving's level, a building in the way, or no walk: nothing
    for (walk, blocked, ground) in [(true, false, crest + 0.1), (true, true, crest - 1.0), (false, false, crest - 1.0)] {
        let mut s = set_with();
        let (filled, _) = s.add_quay_fill(&height, &|_, z| (walk && z >= walk_from).then_some(crest), &|_, z| blocked && z > w0 + 2.0, &|_, _| ground, 0.083);
        assert_eq!((filled, s.structures.len()), (0, 0));
    }
}
