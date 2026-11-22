//! Stage A6 on the real city: the street built with the structures' joins
//! (`eo_structures::build_joined`). What the joins rescue (the Central
//! Freeway over 13th Street, the short bridges held as end too low), that
//! nothing built without them is lost, that every leg's deck starts at its
//! junction's mouth with no kerb across it, that every raised stub meets its
//! deck, and that the street surface's two known defects (fighting surfaces
//! and verge banks over paving, `eo-street/tests/surface.rs`) are no worse
//! with the joins than without, on that test's sample of tiles and on every
//! tile a structure end lies in.

use std::collections::BTreeSet;
use std::sync::Arc;

use eo_street::{BuildingIndex, Layer, StreetCity, StreetOptions, StreetWorld, Terrain, TileMesh};
use eo_structures::{StructureOptions, StructureWorld};

fn plain_streets(city: &eo_data::City, grid: Arc<eo_data::ElevationGrid>) -> StreetWorld {
    let o = StreetOptions::load().expect("street options");
    let sc = StreetCity::load().expect("street city");
    let plan = eo_street::plan::build_plan(&sc, &o);
    let rings = city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
    let lift = o.open_ground_lift;
    StreetWorld::from_plan(o, sc, plan, Terrain { grid, carpet_lift: lift }, BuildingIndex::new(rings))
}

/// eo-street's surface test's probes over `keys`: (carriageway probes,
/// fights, verge samples over paving, pokes).
fn defects(w: &StreetWorld, keys: &[(i32, i32)]) -> (usize, usize, usize, usize) {
    let (mut probes, mut fights) = (0usize, 0usize);
    for k in keys {
        let Some(refs) = w.tiles.get(k) else { continue };
        for &(ei, i) in &refs.spans {
            let (ei, i) = (ei as usize, i as usize);
            let e = &w.plan.entries[ei];
            let g = &w.geoms[ei];
            let s = (g.stations[i].s + g.stations[i + 1].s) / 2.0;
            if s < e.s0() + 1.0 || s > e.s1() - 1.0 || e.in_yield_span(s) {
                continue;
            }
            let f = e.frame(s, false);
            for q in -3..=3 {
                let p = f.at(e.half * 0.9 * q as f64 / 3.0);
                probes += 1;
                let mut hs = w.paved_heights(p[0], p[1]);
                hs.sort_by(|a, b| a.partial_cmp(b).unwrap());
                if hs.windows(2).any(|x| x[1] - x[0] > 0.0005 && x[1] - x[0] < 0.008) {
                    fights += 1;
                }
            }
        }
    }
    let tri_h = |m: &TileMesh, t: usize, x: f64, z: f64| -> Option<f64> {
        let [a, b, c] = [m.indices[3 * t], m.indices[3 * t + 1], m.indices[3 * t + 2]].map(|i| m.positions[i as usize]);
        let (ax, az, bx, bz, cx, cz) = (a[0] as f64, a[2] as f64, b[0] as f64, b[2] as f64, c[0] as f64, c[2] as f64);
        let det = (bz - cz) * (ax - cx) + (cx - bx) * (az - cz);
        if det.abs() < 1e-12 {
            return None;
        }
        let wa = ((bz - cz) * (x - cx) + (cx - bx) * (z - cz)) / det;
        let wb = ((cz - az) * (x - cx) + (ax - cx) * (z - cz)) / det;
        let wc = 1.0 - wa - wb;
        (wa >= -1e-6 && wb >= -1e-6 && wc >= -1e-6).then(|| wa * a[1] as f64 + wb * b[1] as f64 + wc * c[1] as f64)
    };
    let (mut samples, mut pokes) = (0usize, 0usize);
    for &k in keys {
        let centre = w.cached_mesh(k);
        let around: Vec<_> = (-1..=1).flat_map(|j| (-1..=1).map(move |i| (k.0 + i, k.1 + j))).map(|kk| w.cached_mesh(kk)).collect();
        for (t, layer) in centre.layers.iter().enumerate() {
            if *layer != Layer::Verge {
                continue;
            }
            let [a, b, c] = [centre.indices[3 * t], centre.indices[3 * t + 1], centre.indices[3 * t + 2]].map(|i| centre.positions[i as usize]);
            for (wa, wb) in [(1.0 / 3.0, 1.0 / 3.0), (0.6, 0.2), (0.2, 0.6), (0.2, 0.2)] {
                let wc = 1.0 - wa - wb;
                let x = a[0] as f64 * wa + b[0] as f64 * wb + c[0] as f64 * wc;
                let z = a[2] as f64 * wa + b[2] as f64 * wb + c[2] as f64 * wc;
                let vy = a[1] as f64 * wa + b[1] as f64 * wb + c[1] as f64 * wc;
                let mut paved: Option<f64> = None;
                for m in &around {
                    for (u, l2) in m.layers.iter().enumerate() {
                        if matches!(l2, Layer::Sidewalk | Layer::CurbTop | Layer::Ramp | Layer::Carriageway) {
                            if let Some(h) = tri_h(m, u, x, z) {
                                paved = Some(paved.map_or(h, |p: f64| p.max(h)));
                            }
                        }
                    }
                }
                let Some(p) = paved else { continue };
                samples += 1;
                if vy > p + 0.01 {
                    pokes += 1;
                }
            }
        }
    }
    (probes, fights, samples, pokes)
}

#[test]
fn the_joins_rescue_what_they_should_lose_nothing_and_leave_the_street_no_worse() {
    let paths = eo_data::DataPaths::discover().expect("repository");
    let city = eo_data::City::load(&paths).expect("city");
    let grid = Arc::new(eo_data::ElevationGrid::load(&paths).expect("elevation"));
    let ca = eo_data::CaStandards::load(&paths).expect("standards");
    let plain = plain_streets(&city, grid);
    let first = StructureWorld::build(&plain, &city, &ca, eo_landmarks::LandmarksJs::get(), StructureOptions::default()).expect("structures");
    let t = std::time::Instant::now();
    let joined = eo_structures::build_joined(&plain, &city, &ca, eo_landmarks::LandmarksJs::get(), StructureOptions::default()).expect("joined");
    let secs = t.elapsed().as_secs_f64();
    let streets = joined.streets.as_ref().expect("the street built with joins");
    let w = &joined.structures;
    let stats = w.census().joins.clone().expect("join stats");
    println!("{stats}\n(the joined build, every pass, in {secs:.1} s)");
    assert!(stats.lost.is_empty(), "built without the joins and held with them: {:?}", stats.lost);
    // the Central Freeway's twin decks over 13th Street.
    let central: Vec<_> = w.chains().iter().filter(|c| c.name() == "Central Freeway" && c.class == eo_structures::RoadClass::Motorway).collect();
    assert!(central.len() >= 2 && central.iter().all(|c| c.is_built()), "Central Freeway: {:?}", central.iter().map(|c| (c.id, c.status.clone())).collect::<Vec<_>>());
    let held_low = |s: &StructureWorld| s.held().filter(|c| matches!(c.hold(), Some(eo_structures::Hold::EndTooLow { .. }))).count();
    println!("held as end too low: {} without the joins, {} with", held_low(&first), held_low(w));
    assert!(held_low(&first) >= held_low(w) + 10);
    assert!(w.census().built >= first.census().built + 12);
    // every leg in a junction: its deck starts at the junction's mouth, and
    // no kerb runs across the mouth.
    let joins = streets.o.structures.as_ref().expect("joins");
    let plan = &streets.plan;
    let mut legs = 0;
    for (ei, e) in plan.entries.iter().enumerate() {
        let Some(k) = e.structure_leg else { continue };
        let start = eo_street::plan::leg_at_start(e, &streets.o);
        let Some((n, a)) = (if start { e.approach_start } else { e.approach_end }) else { continue };
        legs += 1;
        let app = &plan.nodes[n].approaches[a];
        let mouth = eo_street::plan::approach_end_frame(&plan.entries, app);
        let seg = joins.ends[k].segment;
        let c = w.chains().iter().find(|c| c.is_built() && [c.pieces.first().unwrap().segment, c.pieces.last().unwrap().segment].contains(&seg)).expect("the leg's chain is built");
        let tip = [c.points[0], *c.points.last().unwrap()].into_iter().map(|p| (p[0] - mouth.x).hypot(p[1] - mouth.z)).fold(f64::INFINITY, f64::min);
        assert!(tip < 0.05, "leg {ei}: chain {} ({:?}, {:.1} m) ends {tip:.3} m from the mouth (trim {:.2})", c.id, c.kind, c.length(), app.trim);
        let m = streets.tile_mesh(eo_street::tile_key(mouth.x, mouth.z));
        for q in 1..8 {
            let u = -e.half * 0.8 + 1.6 * e.half * q as f64 / 8.0;
            let p = mouth.at(u);
            let across = m.indices.chunks_exact(3).zip(&m.layers).any(|(t, l)| {
                *l == Layer::CurbFace
                    && t.iter().any(|&i| {
                        let v = m.positions[i as usize];
                        (v[0] as f64 - p[0]).hypot(v[2] as f64 - p[1]) < 0.05
                    })
            });
            assert!(!across, "leg {ei}: a kerb face at its mouth, offset {u:.2}");
        }
    }
    assert_eq!(legs, stats.legs);
    assert!(legs >= 4, "{legs} legs");
    // every raised stub meets its deck at the end.
    let mut raised = 0;
    for c in w.chains().iter().filter(|c| c.is_built()) {
        for e in 0..2 {
            let end = &c.ends[e];
            let Some(r) = joins.ends.iter().find(|j| (j.at[0] - end.at[0]).hypot(j.at[1] - end.at[1]) < 0.7).and_then(|j| j.rise) else { continue };
            let Some(fixed) = end.fixed else { continue };
            if (fixed - r.to).abs() > 0.05 {
                continue;
            }
            raised += 1;
            let f = c.frame(if e == 0 { 0.0 } else { c.length() });
            let out = if e == 0 { -1.0 } else { 1.0 };
            let q = [f.x + f.tx * 0.05 * out, f.z + f.tz * 0.05 * out];
            let drawn = streets.paved_at(q[0], q[1]).expect("the stub");
            let crown = streets.o.cross_slope * if e == 0 { c.pieces[0].half } else { c.pieces.last().unwrap().half };
            assert!((drawn - crown - fixed).abs() < 0.1, "chain {} end {e}: stub {drawn:.3} vs deck {fixed:.3}", c.id);
        }
    }
    println!("raised stubs meeting their decks: {raised}");
    assert!(raised >= 10);
    // the street surface: no worse with the joins.
    let mut keys: BTreeSet<(i32, i32)> = plain.tile_keys().into_iter().step_by(20).collect();
    for e in &joins.ends {
        keys.insert(eo_street::tile_key(e.at[0], e.at[1]));
    }
    let keys: Vec<(i32, i32)> = keys.into_iter().collect();
    let before = defects(&plain, &keys);
    let after = defects(streets, &keys);
    println!("{} tiles: without the joins {before:?}, with {after:?} (carriageway probes, fights, verge samples over paving, verge above it)", keys.len());
    assert!(after.1 <= before.1, "fighting surfaces {} -> {}", before.1, after.1);
    assert!(after.3 <= before.3, "verge banks over paving {} -> {}", before.3, after.3);
}
