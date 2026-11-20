//! Tests of the meshes and collision triangles on the synthetic inputs of
//! `synthetic.rs`: a closed deck top, the clearance under every crossing
//! seen by a vertical ray, columns off paving, the abutment on the paved
//! stub, determinism, levels of detail, tunnels and landmark decks.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use eo_street::Layer;
use eo_street::city::Segment;

use crate::landmark::LandmarkSpec;
use crate::synthetic::{Case, built, case, deck_along_street, deck_over_street, flat, seg, solve, standards, way};
use crate::{ChainKind, ColumnSite, EndKind, LandmarkDeck, StructureClass, StructureMesh, StructureOptions, StructureWorld};

type Tri = [[f64; 3]; 3];

/// Built as eo-app builds it: the solve, then the meshes (a fallback
/// section's lane is the state lane here).
fn meshed(c: &Case, landmarks: Vec<LandmarkSpec>, options: StructureOptions) -> StructureWorld {
    let mut w = solve(c, landmarks, options);
    let ca = standards();
    let lane = ca.metric("travel-lane-width-state-highway-min").unwrap();
    w.attach_meshes(&c.streets, &ca, &|_| lane).unwrap();
    w
}

fn tris_of(m: &StructureMesh, keep: &dyn Fn(StructureClass) -> bool) -> Vec<Tri> {
    let p = |i: u32| {
        let q = m.positions[i as usize];
        [q[0] as f64, q[1] as f64, q[2] as f64]
    };
    m.indices.chunks_exact(3).filter(|t| keep(StructureClass::of(m.classes[t[0] as usize]).unwrap())).map(|t| [p(t[0]), p(t[1]), p(t[2])]).collect()
}

/// Triangles of every tile at `lod` whose class `keep` accepts.
fn all_tris(w: &StructureWorld, lod: u8, keep: &dyn Fn(StructureClass) -> bool) -> Vec<Tri> {
    w.tile_keys().into_iter().filter_map(|k| w.render_tile(k, lod)).flat_map(|m| tris_of(&m, keep)).collect()
}

/// Height of triangle `t` over (x, z), if (x, z) is inside it in plan.
fn height_over(t: &Tri, x: f64, z: f64) -> Option<f64> {
    let (a, b, c) = (t[0], t[1], t[2]);
    let det = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
    if det.abs() < 1e-12 {
        return None;
    }
    let l1 = ((b[2] - c[2]) * (x - c[0]) + (c[0] - b[0]) * (z - c[2])) / det;
    let l2 = ((c[2] - a[2]) * (x - c[0]) + (a[0] - c[0]) * (z - c[2])) / det;
    let l3 = 1.0 - l1 - l2;
    let e = -1e-9;
    (l1 >= e && l2 >= e && l3 >= e).then(|| l1 * a[1] + l2 * b[1] + l3 * c[1])
}

/// The lowest triangle over (x, z) above `y0`.
fn ray_up(tris: &[Tri], x: f64, z: f64, y0: f64) -> Option<f64> {
    tris.iter().filter_map(|t| height_over(t, x, z)).filter(|&y| y > y0).fold(None, |m, y| Some(m.map_or(y, |v: f64| v.min(y))))
}

/// The highest triangle over (x, z).
fn ray_down(tris: &[Tri], x: f64, z: f64) -> Option<f64> {
    tris.iter().filter_map(|t| height_over(t, x, z)).fold(None, |m, y| Some(m.map_or(y, |v: f64| v.max(y))))
}

fn is_deck(c: StructureClass) -> bool {
    c == StructureClass::DeckSurface
}

/// A deck along a quarter circle of radius 150 m (24 pieces), at grade
/// beyond both ends, over a street crossing it.
fn curved_deck() -> Case {
    let mut s: Vec<Segment> = vec![];
    let arc = |t: f64| [-150.0 + 150.0 * (t * std::f64::consts::FRAC_PI_2).cos(), 150.0 * (t * std::f64::consts::FRAC_PI_2).sin()];
    way(&mut s, "2000", "motorway_link", 9.0, 0.0, [0.0, -200.0], [0.0, 0.0], false);
    let n = 24;
    for k in 0..n {
        s.push(seg("2001", k, "motorway_link", 9.0, 0.0, &[arc(k as f64 / n as f64), arc((k + 1) as f64 / n as f64)], true));
    }
    way(&mut s, "2002", "motorway_link", 9.0, 0.0, [-150.0, 150.0], [-350.0, 150.0], false);
    way(&mut s, "2100", "residential", 10.0, 3.0, [-200.0, 60.0], [100.0, 160.0], false);
    case(s, &[("2000", 0, false, false), ("2001", 1, true, false), ("2002", 0, false, false), ("2100", 0, false, false)], &flat)
}

#[test]
fn the_deck_top_is_closed_along_its_length() {
    for c in [deck_over_street(), curved_deck()] {
        let w = meshed(&c, vec![], StructureOptions::default());
        let decks: Vec<&crate::Chain> = w.chains().iter().filter(|d| d.is_built() && d.kind == ChainKind::Deck).collect();
        assert_eq!(decks.len(), 1);
        let d = decks[0];
        let tris = all_tris(&w, 0, &is_deck);
        assert!(tris.len() > 100);
        let q = |p: [f64; 3]| [(p[0] * 1000.0).round() as i64, (p[1] * 1000.0).round() as i64, (p[2] * 1000.0).round() as i64];
        let mut edges: HashMap<([i64; 3], [i64; 3]), (usize, [f64; 3], [f64; 3])> = HashMap::new();
        for t in &tris {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let (ka, kb) = (q(a), q(b));
                let key = if ka < kb { (ka, kb) } else { (kb, ka) };
                edges.entry(key).or_insert((0, a, b)).0 += 1;
            }
        }
        let len = d.length();
        let mut boundary = 0;
        for (n, a, b) in edges.values() {
            assert!(*n <= 2, "an edge of the deck top is shared by {n} triangles");
            if *n == 2 {
                continue;
            }
            boundary += 1;
            // a boundary edge is on a side of the deck or across one of its ends.
            for p in [a, b] {
                let (s, dist) = d.project(p[0], p[2]);
                let side = (dist - d.half_at(s)).abs() < 0.01;
                let end = s < 0.01 || s > len - 0.01;
                assert!(side || end, "a hole in the deck top at {p:?} (station {s:.2}, {dist:.3} m off the centreline)");
            }
        }
        assert!(boundary > 0);
    }
}

#[test]
fn every_crossing_has_its_clearance_under_the_meshes() {
    let ca = standards();
    // a deck over a street.
    let c = deck_over_street();
    let w = meshed(&c, vec![], StructureOptions::default());
    let all = all_tris(&w, 0, &|_| true);
    let d = built(&w, "101");
    assert!(!d.crossings.is_empty());
    let clearance = ca.metric("structure-vertical-clearance-local").unwrap();
    let mut rays = 0;
    let street_half = c.streets.plan.entries.iter().find(|e| e.highway == "residential").unwrap().half;
    for x in [-6.5, -3.0, 0.0, 3.0, 6.5] {
        for z in [-(street_half - 0.3), -2.0, 0.0, 2.0, street_half - 0.3] {
            let surface = c.streets.paved_at(x, z).expect("the crossed street is paved here");
            let hit = ray_up(&all, x, z, surface).expect("the deck is over the street");
            assert!(hit - surface >= clearance - 1e-3, "at ({x}, {z}): {hit} over the street at {surface}");
            rays += 1;
        }
    }
    assert_eq!(rays, 25);
    // a level 2 deck over a level 1 deck: from the lower deck's top up.
    let mut s = vec![];
    way(&mut s, "300", "motorway", 14.0, 0.0, [-400.0, 0.0], [-200.0, 0.0], false);
    way(&mut s, "301", "motorway", 14.0, 0.0, [-200.0, 0.0], [200.0, 0.0], true);
    way(&mut s, "302", "motorway", 14.0, 0.0, [200.0, 0.0], [400.0, 0.0], false);
    way(&mut s, "400", "motorway", 14.0, 0.0, [0.0, -450.0], [0.0, -250.0], false);
    way(&mut s, "401", "motorway", 14.0, 0.0, [0.0, -250.0], [0.0, 250.0], true);
    way(&mut s, "402", "motorway", 14.0, 0.0, [0.0, 250.0], [0.0, 450.0], false);
    let roads = [("300", 0, false, false), ("301", 1, true, false), ("302", 0, false, false), ("400", 0, false, false), ("401", 2, true, false), ("402", 0, false, false)];
    let c2 = case(s, &roads, &flat);
    let w2 = meshed(&c2, vec![], StructureOptions::default());
    let decks = all_tris(&w2, 0, &is_deck);
    let all2 = all_tris(&w2, 0, &|_| true);
    let fw = ca.metric("structure-vertical-clearance-freeway").unwrap();
    let lower = built(&w2, "301");
    let mut checked = 0;
    for x in [-6.0, -3.0, 0.0, 3.0, 6.0] {
        for z in [-6.0, 0.0, 6.0] {
            let low = lower.y_at(lower.project(x, z).0);
            let top = ray_down(&decks.iter().filter(|t| (t[0][1] - low).abs() < 0.5).copied().collect::<Vec<_>>(), x, z).expect("the lower deck's top");
            let hit = ray_up(&all2, x, z, top + 0.05).expect("the upper deck over it");
            assert!(hit - top >= fw - 1e-3, "at ({x}, {z}): {hit} over the lower deck at {top}");
            checked += 1;
        }
    }
    assert_eq!(checked, 15);
}

#[test]
fn no_column_stands_on_paving_or_under_a_lower_deck() {
    for c in [deck_over_street(), deck_along_street(&|g| {
        g.width = 14.0;
        g.parking = [3.0, 3.0];
    })] {
        let w = meshed(&c, vec![], StructureOptions::default());
        let cols = w.all_column_footprints();
        assert!(!cols.is_empty());
        let mut on_street = 0;
        for f in &cols {
            for k in 0..=16 {
                let (x, z) = if k == 16 {
                    (f.x, f.z)
                } else {
                    let a = k as f64 * std::f64::consts::TAU / 16.0;
                    (f.x + 0.98 * f.radius * a.cos(), f.z + 0.98 * f.radius * a.sin())
                };
                let layer = c.streets.paved_layer_at(x, z);
                match f.site {
                    ColumnSite::OffPaving => assert!(c.streets.paved_at(x, z).is_none() || layer == Some(Layer::Verge), "column of chain {} bent {} on {layer:?} at ({x}, {z})", f.chain, f.bent),
                    ColumnSite::ParkingLane => assert_eq!(layer, Some(Layer::Carriageway)),
                    ColumnSite::FurnitureZone => assert_eq!(layer, Some(Layer::Sidewalk)),
                    ColumnSite::Median => {}
                }
                let level = w.chain(f.chain).unwrap().level;
                assert!(w.decks_at(x, z).iter().all(|(l, _)| *l >= level), "column of chain {} under a lower deck", f.chain);
            }
            on_street += usize::from(f.site.on_street());
            // the column is in the tile its footprint query answers for.
            assert!(w.column_footprints(eo_street::tile_key(f.x, f.z)).contains(f));
        }
        assert!(on_street == 0 || cols.iter().any(|f| f.street.as_ref().is_some_and(|s| s.name == "Way 1400")));
        // and its mesh is there: concrete round the footprint.
        let concrete = all_tris(&w, 0, &|c| c == StructureClass::Concrete);
        for f in &cols {
            let near = concrete.iter().filter(|t| t.iter().all(|p| ((p[0] - f.x).hypot(p[2] - f.z) - f.radius).abs() < 0.02)).count();
            assert!(near >= 16, "column of chain {} bent {}: {near} triangles", f.chain, f.bent);
        }
    }
}

#[test]
fn the_abutment_meets_the_paved_stub() {
    // the ground slopes 0.5 % along the deck and 1 % across it.
    let mut s = vec![];
    way(&mut s, "100", "motorway", 14.0, 0.0, [0.0, -400.0], [0.0, -200.0], false);
    way(&mut s, "101", "motorway", 14.0, 0.0, [0.0, -200.0], [0.0, 200.0], true);
    way(&mut s, "102", "motorway", 14.0, 0.0, [0.0, 200.0], [0.0, 400.0], false);
    way(&mut s, "200", "residential", 10.0, 3.0, [-300.0, 0.0], [300.0, 0.0], false);
    let c = case(s, &[("100", 0, false, false), ("101", 1, true, false), ("102", 0, false, false), ("200", 0, false, false)], &|x, z| 20.0 + 0.005 * z + 0.01 * x);
    let w = meshed(&c, vec![], StructureOptions::default());
    let d = built(&w, "101");
    assert!(d.is_built(), "{:?}", d.status);
    let decks = all_tris(&w, 0, &is_deck);
    let concrete = all_tris(&w, 0, &|c| c == StructureClass::Concrete);
    let mut checked = 0;
    for (e, end) in d.ends.iter().enumerate() {
        assert_eq!(end.kind, EndKind::Paved);
        let s_end = if e == 0 { 0.0 } else { d.length() };
        let f = d.frame(s_end);
        let out = if e == 0 { [-f.tx, -f.tz] } else { [f.tx, f.tz] };
        let h = d.half_at(s_end);
        for u in [-(h - 0.3), -h / 2.0, 0.0, h / 2.0, h - 0.3] {
            let at = |k: f64| [f.x + f.nx * u + out[0] * k, f.z + f.nz * u + out[1] * k];
            let p_stub = at(0.05);
            let p_deck = at(-0.05);
            let stub = c.streets.paved_at(p_stub[0], p_stub[1]).expect("the stub's ribbon");
            let deck = ray_down(&decks, p_deck[0], p_deck[1]).expect("the deck's top");
            assert!((stub - deck).abs() <= 0.02, "end {e} at u {u}: deck {deck} vs stub {stub}");
            checked += 1;
        }
        // the end wall closing the stub: concrete across the end, facing out.
        let wall = concrete.iter().filter(|t| t.iter().all(|p| ((p[0] - f.x) * out[0] + (p[2] - f.z) * out[1]).abs() < 0.01)).count();
        assert!(wall >= 4, "end {e}: {wall} end-wall triangles");
    }
    assert_eq!(checked, 10);
}

/// Hash of every tile's meshes at every level and its collision triangles.
fn world_hash(w: &StructureWorld) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for key in w.tile_keys() {
        key.hash(&mut h);
        for lod in 0..3 {
            if let Some(m) = w.render_tile(key, lod) {
                for p in &m.positions {
                    p.map(f32::to_bits).hash(&mut h);
                }
                for n in &m.normals {
                    n.map(f32::to_bits).hash(&mut h);
                }
                for uv in &m.uv {
                    uv.map(f32::to_bits).hash(&mut h);
                }
                m.colors.hash(&mut h);
                m.classes.hash(&mut h);
                m.indices.hash(&mut h);
            }
        }
        if let Some(s) = w.surface_tile(key) {
            for t in s.street.iter().chain(&s.barrier) {
                for p in t {
                    p.map(f64::to_bits).hash(&mut h);
                }
            }
        }
    }
    for f in w.all_column_footprints() {
        format!("{f:?}").hash(&mut h);
    }
    h.finish()
}

#[test]
fn meshes_are_the_same_built_twice_or_in_another_order() {
    let make = |rev: bool| {
        let mut s = vec![];
        way(&mut s, "600", "motorway", 14.0, 0.0, [-500.0, 0.0], [-300.0, 0.0], false);
        way(&mut s, "601", "motorway", 14.0, 0.0, [-300.0, 0.0], [0.0, 0.0], true);
        way(&mut s, "602", "motorway", 14.0, 0.0, [0.0, 0.0], [300.0, 0.0], true);
        way(&mut s, "603", "motorway", 14.0, 0.0, [300.0, 0.0], [500.0, 0.0], false);
        way(&mut s, "604", "motorway_link", 8.0, 0.0, [0.0, 0.0], [120.0, 250.0], true);
        way(&mut s, "605", "residential", 8.0, 0.0, [120.0, 250.0], [120.0, 450.0], false);
        way(&mut s, "700", "residential", 10.0, 2.0, [-40.0, -300.0], [-40.0, 300.0], false);
        way(&mut s, "701", "motorway", 14.0, 0.0, [-200.0, -250.0], [200.0, 250.0], true);
        if rev {
            s.reverse();
            let (even, odd): (Vec<(usize, Segment)>, Vec<(usize, Segment)>) = s.into_iter().enumerate().partition(|(i, _)| i % 2 == 0);
            s = even.into_iter().chain(odd).map(|(_, x)| x).collect();
        }
        let roads = [("600", 0, false, false), ("601", 1, true, false), ("602", 1, true, false), ("603", 0, false, false), ("604", 1, true, false), ("605", 0, false, false), ("700", 0, false, false), ("701", 2, true, false)];
        case(s, &roads, &|x, z| 3.0 + 0.01 * x - 0.005 * z)
    };
    let a = make(false);
    let first = meshed(&a, vec![], StructureOptions::default());
    let again = meshed(&a, vec![], StructureOptions::default());
    let other = meshed(&make(true), vec![], StructureOptions::default());
    assert!(first.tile_keys().len() >= 4);
    let h = world_hash(&first);
    assert_eq!(h, world_hash(&again));
    assert_eq!(h, world_hash(&other));
    // and the shared copies are the same as the built ones.
    let k = first.tile_keys()[0];
    assert_eq!(first.render_tile_shared(k, 0).map(|m| (*m).clone()), first.render_tile(k, 0));
}

#[test]
fn levels_of_detail_get_coarser() {
    let c = curved_deck();
    let w = meshed(&c, vec![], StructureOptions::default());
    let mut sums = [0usize; 3];
    let mut busiest = ((0, 0), 0usize);
    for key in w.tile_keys() {
        let n: Vec<usize> = (0..3).map(|lod| w.render_tile(key, lod).map_or(0, |m| m.triangles())).collect();
        for lod in 0..3 {
            sums[lod] += n[lod];
        }
        if n[0] > busiest.1 {
            busiest = (key, n[0]);
        }
        if n[2] > 0 {
            assert!(n[0] > n[1] && n[1] > n[2], "tile {key:?}: {n:?}");
        }
    }
    assert!(sums[0] > sums[1] && sums[1] > sums[2] && sums[2] > 0, "{sums:?}");
    let k = busiest.0;
    let lod = |l: u8| w.render_tile(k, l).unwrap();
    assert!(lod(0).triangles() > lod(1).triangles() && lod(1).triangles() > lod(2).triangles());
    // paint at the near level only, edge lines only at the far level.
    let paint = |m: &StructureMesh| m.classes.iter().filter(|&&c| c == StructureClass::Paint as u8).count();
    assert!(paint(&lod(0)) > paint(&lod(1)) && paint(&lod(1)) > 0 && paint(&lod(2)) == 0);
}

#[test]
fn vertex_arrays_hold_together() {
    let c = deck_over_street();
    let w = meshed(&c, vec![], StructureOptions::default());
    let d = built(&w, "101");
    let mut wear_top = vec![];
    let mut wear_mid = vec![];
    for key in w.tile_keys() {
        for lod in 0..3 {
            let Some(m) = w.render_tile(key, lod) else { continue };
            let n = m.positions.len();
            assert!(m.normals.len() == n && m.colors.len() == n && m.classes.len() == n && m.uv.len() == n);
            assert!(m.indices.len() % 3 == 0 && m.indices.iter().all(|&i| (i as usize) < n));
            for (k, nn) in m.normals.iter().enumerate() {
                let l = (nn[0] * nn[0] + nn[1] * nn[1] + nn[2] * nn[2]).sqrt();
                assert!((l - 1.0).abs() < 1e-3, "normal {nn:?}");
                let class = StructureClass::of(m.classes[k]).expect("a known class");
                let alpha = m.colors[k] >> 24;
                if matches!(class, StructureClass::DeckSurface | StructureClass::Paint) {
                    assert_eq!(alpha, 255);
                }
                // column vertices: the stained top against the clean shaft.
                if lod == 0 && class == StructureClass::Concrete && d.supports.iter().flat_map(|b| &b.columns).any(|col| ((m.positions[k][0] as f64 - col.x).hypot(m.positions[k][2] as f64 - col.z) - col.radius).abs() < 1e-3) {
                    let ground = c.streets.ground(m.positions[k][0] as f64, m.positions[k][2] as f64);
                    let top = d.y_at(d.project(m.positions[k][0] as f64, m.positions[k][2] as f64).0) - 2.0;
                    if m.positions[k][1] as f64 > top {
                        wear_top.push(alpha);
                    } else if m.positions[k][1] as f64 > ground + 2.0 {
                        wear_mid.push(alpha);
                    }
                }
            }
            // paint stands just above the deck.
            let decks = tris_of(&m, &is_deck);
            for (k, &cl) in m.classes.iter().enumerate() {
                if cl == StructureClass::Paint as u8 {
                    let p = m.positions[k];
                    if let Some(y) = ray_down(&decks, p[0] as f64, p[2] as f64) {
                        assert!((p[1] as f64 - y - crate::PAINT_LIFT_M).abs() < 2e-3, "paint {} over deck {y}", p[1]);
                    }
                }
            }
        }
    }
    let mean = |v: &[u32]| v.iter().sum::<u32>() as f64 / v.len().max(1) as f64;
    assert!(!wear_top.is_empty() && !wear_mid.is_empty());
    assert!(mean(&wear_top) > mean(&wear_mid) + 30.0, "column wear: top {} mid {}", mean(&wear_top), mean(&wear_mid));
}

#[test]
fn tunnels_without_their_switch_and_landmark_decks_draw_nothing_here() {
    // a tunnel without --tunnels: nothing to draw or collide with. (With
    // it, stage A7 draws its bore: `joins_tests`. Until A7 this test built
    // the tunnel with --tunnels and asserted nothing drawn, as A2 drew no
    // tunnel; that half is now `a_tunnel_through_a_hill_has_its_bore_..`.)
    let hill = |x: f64, _z: f64| 40.0 * (-(x * x) / (2.0 * 110.0 * 110.0)).exp();
    let mut s = vec![];
    way(&mut s, "500", "primary", 12.0, 0.0, [-500.0, 0.0], [-320.0, 0.0], false);
    way(&mut s, "501", "primary", 12.0, 0.0, [-320.0, 0.0], [320.0, 0.0], true);
    way(&mut s, "502", "primary", 12.0, 0.0, [320.0, 0.0], [500.0, 0.0], false);
    let c = case(s, &[("500", 0, false, false), ("501", -1, false, true), ("502", 0, false, false)], &hill);
    let w = meshed(&c, vec![], StructureOptions { tunnels: false });
    assert_eq!(built(&w, "501").status, crate::Status::TunnelOff);
    assert!(w.tile_keys().is_empty());
    for key in [(0, 0), (-1, 0), (2, -1)] {
        assert!(w.surface_tile(key).is_none() && w.render_tile(key, 0).is_none());
    }
    assert!(w.terrain_cuts().is_empty() && w.all_tunnel_lights().is_empty());
    // a landmark deck: its deck top collides, nothing of it is drawn.
    let mut s = vec![];
    way(&mut s, "1000", "motorway", 14.0, 0.0, [-400.0, 0.0], [-250.0, 0.0], false);
    way(&mut s, "1001", "motorway", 14.0, 0.0, [-250.0, 0.0], [300.0, 0.0], true);
    let c = case(s, &[("1000", 0, false, false), ("1001", 1, true, false)], &flat);
    let spec = LandmarkSpec {
        deck: LandmarkDeck::BayBridgeUpper,
        ways: vec!["1001".into()],
        corridor: vec![],
        corridor_half: 0.0,
        c0: 0.0,
        c1: 1000.0,
        chainage: Box::new(|x, _| x),
        top: Box::new(|c| 6.0 + 0.01 * c),
    };
    let w = meshed(&c, vec![spec], StructureOptions::default());
    let key = eo_street::tile_key(200.0, 0.0);
    let surf = w.surface_tile(key).expect("the landmark deck collides");
    let y = surf.street.iter().filter_map(|t| height_over(t, 200.0, 0.0)).fold(f64::NEG_INFINITY, f64::max);
    assert!((y - 8.0).abs() < 1e-3, "landmark deck top {y}");
    assert!(w.render_tile(key, 0).is_none());
    // the approach deck is drawn and collides.
    let k2 = eo_street::tile_key(-100.0, 0.0);
    assert!(w.render_tile(k2, 0).is_some_and(|m| m.classes.contains(&(StructureClass::DeckSurface as u8))));
    assert!(w.surface_tile(k2).is_some_and(|s| !s.street.is_empty() && !s.barrier.is_empty()));
}

#[test]
fn a_shared_bent_is_one_cap_over_both_decks_and_its_columns_once() {
    let c = crate::synthetic::twin_decks_over_a_median();
    let w = meshed(&c, vec![], StructureOptions::default());
    let (a, b) = (built(&w, "1700"), built(&w, "1710"));
    let concrete = all_tris(&w, 0, &|cl| cl == StructureClass::Concrete);
    let cols = w.all_column_footprints();
    assert!(!cols.is_empty());
    // the columns once: each on the line's owning bent, none overlapping another.
    for f in &cols {
        let bent = &w.chain(f.chain).unwrap().supports[f.bent as usize];
        assert!(bent.twin.is_some_and(|t| !t.carried), "chain {} bent {}", f.chain, f.bent);
    }
    for (i, f) in cols.iter().enumerate() {
        for g in &cols[i + 1..] {
            assert!((f.x - g.x).hypot(f.z - g.z) >= f.radius + g.radius - 1e-6, "columns at ({}, {}) and ({}, {}) overlap", f.x, f.z, g.x, g.z);
        }
    }
    let soffit = |d: &crate::Chain, s: f64| d.y_at(s) - d.depth_at(s - 1e-6).min(d.depth_at(s + 1e-6));
    let mut checked = 0;
    for (k, bent) in a.supports.iter().enumerate() {
        let f = a.frame(bent.s);
        let Some(t) = bent.twin.filter(|t| !t.carried) else { continue };
        if f.z.abs() > 140.0 || bent.columns.is_empty() {
            continue;
        }
        let o = &b.supports[t.bent as usize];
        assert_eq!(o.twin.map(|x| (x.chain, x.bent as usize)), Some((a.id, k)));
        let r = bent.columns[0].radius;
        // in the gap between the decks, clear of the column: the cap, at each side's soffit.
        for (x, d, s) in [(-(r + 0.4), a, bent.s), (r + 0.4, b, o.s)] {
            let hit = ray_up(&concrete, x, f.z, 0.0).expect("the shared cap over the gap");
            assert!((hit - soffit(d, s)).abs() < 0.05, "cap at x {x}: {hit} vs soffit {}", soffit(d, s));
            // and nothing between the decks away from the bent line.
            assert!(ray_up(&concrete, x, f.z + 8.0, 0.0).is_none(), "concrete in the gap at z {}", f.z + 8.0);
        }
        // the column reaches the deck over it.
        let col = &bent.columns[0];
        let top = concrete.iter().filter(|t| t.iter().all(|p| ((p[0] - col.x).hypot(p[2] - col.z) - col.radius).abs() < 0.02)).flat_map(|t| t.iter().map(|p| p[1])).fold(f64::NEG_INFINITY, f64::max);
        assert!((top - soffit(a, bent.s).min(soffit(b, o.s))).abs() < 0.6, "column top {top}");
        checked += 1;
    }
    assert!(checked >= 6, "shared lines checked: {checked}");
}

/// A motorway deck (ways 2201, 2202 through node (0, 0), at grade past x
/// -400 and 400) over a street at x = -100, and a ramp (2204) leaving it at
/// the node at 6 degrees down to a street end at x = 420: for the first
/// 80 m or so the ramp's deck lies over the mainline's.
fn ramp_merge() -> Case {
    let mut s = vec![];
    way(&mut s, "2200", "motorway", 14.0, 0.0, [-700.0, 0.0], [-400.0, 0.0], false);
    way(&mut s, "2201", "motorway", 14.0, 0.0, [-400.0, 0.0], [0.0, 0.0], true);
    way(&mut s, "2202", "motorway", 14.0, 0.0, [0.0, 0.0], [400.0, 0.0], true);
    way(&mut s, "2203", "motorway", 14.0, 0.0, [400.0, 0.0], [700.0, 0.0], false);
    let a = 6f64.to_radians();
    way(&mut s, "2204", "motorway_link", 9.0, 0.0, [0.0, 0.0], [420.0 * a.cos(), 420.0 * a.sin()], true);
    way(&mut s, "2205", "residential", 9.0, 2.0, [420.0 * a.cos(), 420.0 * a.sin()], [600.0, 420.0 * a.sin()], false);
    way(&mut s, "2300", "residential", 10.0, 2.0, [-100.0, -200.0], [-100.0, 200.0], false);
    let roads = [("2200", 0, false, false), ("2201", 1, true, false), ("2202", 1, true, false), ("2203", 0, false, false), ("2204", 1, true, false), ("2205", 0, false, false), ("2300", 0, false, false)];
    case(s, &roads, &flat)
}

#[test]
fn a_ramp_merging_into_its_mainline_is_one_surface_at_the_gore() {
    let c = ramp_merge();
    let w = meshed(&c, vec![], StructureOptions::default());
    let (main, ramp) = (built(&w, "2202"), built(&w, "2204"));
    assert!(main.is_built() && ramp.is_built(), "{:?} {:?}", main.status, ramp.status);
    assert!(ramp.merged_with.contains(&main.id) && main.merged_with.contains(&ramp.id), "{:?}", ramp.merged_with);
    let deck = all_tris(&w, 0, &is_deck);
    let concrete = all_tris(&w, 0, &|cl| cl == StructureClass::Concrete);
    let (mut overlap, mut worst, mut spread) = (0, 0.0f64, 0.0f64);
    let n = ramp.length().floor() as usize;
    for k in 0..=n {
        let s = k as f64;
        let f = ramp.frame(s);
        let (sm, dist) = main.project(f.x, f.z);
        if sm <= 1.0 || dist > main.half_at(sm) {
            continue;
        }
        overlap += 1;
        // the profiles meet where the ramp's centreline lies over the mainline's roadway.
        worst = worst.max((ramp.y_at(s) - main.y_at(sm)).abs());
        // one drawn surface there, no barrier standing on it.
        let ys: Vec<f64> = deck.iter().filter_map(|t| height_over(t, f.x, f.z)).collect();
        assert!(!ys.is_empty(), "no deck at ramp station {s}");
        let (lo, hi) = (ys.iter().copied().fold(f64::INFINITY, f64::min), ys.iter().copied().fold(f64::NEG_INFINITY, f64::max));
        spread = spread.max(hi - lo);
        assert!(ray_up(&concrete, f.x, f.z, hi + 0.01).is_none(), "concrete over the merged deck at ramp station {s}");
    }
    assert!(overlap >= 40, "metres of overlap: {overlap}");
    assert!(worst <= 0.02, "the ramp's deck top {worst:.3} m off the mainline's in the overlap");
    assert!(spread <= 0.02, "two deck surfaces {spread:.3} m apart in the overlap");
    // the outer barrier stands round the gore: the mainline's far side (-z) there.
    let top = main.y_at(main.project(40.0, 0.0).0);
    assert!(ray_down(&concrete, 40.0, -(main.half_at(0.0) + 0.3)).is_some_and(|y| y > top + 0.5), "the mainline's outer barrier at the gore");
    for d in [main, ramp] {
        crate::synthetic::steps_within_grade(d);
    }
}

/// Every drawn piece of every deck (the near level: deck, paint, joints,
/// barriers, fascia, girder, caps, capitals, columns, fill, abutment and end
/// walls) lies inside the clearance envelope (`StructureWorld::obstructions`,
/// stage A6), sampled at each triangle's centroid and at its corners pulled
/// 2 cm toward it; the open air beside, under and over a deck is not in it;
/// and `headroom` under a deck is its soffit.
#[test]
fn the_clearance_envelope_holds_every_drawn_piece_and_nothing_else() {
    let parking = deck_along_street(&|g| {
        g.width = 14.0;
        g.parking = [3.0, 3.0];
    });
    let footways = deck_along_street(&|g| {
        g.sidewalk_left = Some(7.0);
        g.sidewalk_right = Some(7.0);
        g.sidewalk_w = Some(7.0);
    });
    let cases = [("deck over a street", deck_over_street()), ("parking-lane columns", parking), ("furniture-zone columns", footways), ("twin decks", crate::synthetic::twin_decks_over_a_median()), ("ramp merge", ramp_merge()), ("curved deck", curved_deck())];
    for (name, c) in &cases {
        let w = meshed(c, vec![], StructureOptions::default());
        // (the downspouts stand off their column by their own diameter: props keep their margin from it.)
        let tris = all_tris(&w, 0, &|cl| cl != StructureClass::Metal);
        assert!(!tris.is_empty(), "{name}: nothing drawn");
        let (mut checked, mut worst) = (0usize, 0.0f64);
        for t in &tris {
            let m = [(t[0][0] + t[1][0] + t[2][0]) / 3.0, (t[0][1] + t[1][1] + t[2][1]) / 3.0, (t[0][2] + t[1][2] + t[2][2]) / 3.0];
            let mut pts = vec![m];
            for p in t {
                let d = ((m[0] - p[0]).powi(2) + (m[1] - p[1]).powi(2) + (m[2] - p[2]).powi(2)).sqrt().max(1e-9);
                let k = (0.02 / d).min(1.0);
                pts.push([p[0] + (m[0] - p[0]) * k, p[1] + (m[1] - p[1]) * k, p[2] + (m[2] - p[2]) * k]);
            }
            for p in pts {
                let obs = w.obstructions(p[0], p[2], 0.0);
                let gap = obs.iter().map(|o| if p[1] < o.lo { o.lo - p[1] } else if p[1] > o.hi { p[1] - o.hi } else { 0.0 }).fold(f64::INFINITY, f64::min);
                worst = worst.max(gap);
                assert!(gap <= 1e-6, "{name}: drawn point {p:?} outside the envelope (by {gap:.3} m; {obs:?})");
                checked += 1;
            }
        }
        println!("{name}: {checked} points of {} triangles inside the envelope", tris.len());
        assert!(worst <= 1e-6);
    }
    // open air: beside the deck, under it, over it.
    let c = deck_over_street();
    let w = meshed(&c, vec![], StructureOptions::default());
    let d = built(&w, "101");
    let s = d.length() / 2.0;
    let f = d.frame(s);
    let (top, depth) = (d.y_at(s), d.depth_at(s));
    let edge = d.half_at(s) + standards().metric("concrete-barrier-type-60m-base-width").unwrap();
    // a point midway between two bents, off any column.
    let mid = d.bents.windows(2).map(|b| (b[0] + b[1]) / 2.0).find(|m| (m - s).abs() < 30.0).unwrap_or(s);
    let g = d.frame(mid);
    let (gtop, gdepth) = (d.y_at(mid), d.depth_at(mid));
    let ground = c.streets.ground(g.x, g.z);
    assert!(!w.blocked(g.x, g.z, 0.5, ground, gtop - gdepth - 0.3), "under the soffit between bents is open");
    assert!(w.blocked(g.x, g.z, 0.5, ground, gtop - gdepth + 0.05), "the soffit is in the envelope");
    let room = w.headroom(g.x, g.z, 0.5, ground).expect("a deck over it");
    assert!((room - (gtop - gdepth - crate::ENVELOPE_PAD_Y_M)).abs() < 0.05, "headroom {room} under a soffit at {}", gtop - gdepth);
    for off in [edge + 1.0, -(edge + 1.0)] {
        let (x, z) = (f.x + f.nx * off, f.z + f.nz * off);
        assert!(!w.blocked(x, z, 0.4, -10.0, top + 20.0), "beside the deck at {off} m");
        assert_eq!(w.headroom(x, z, 0.4, 0.0), None);
    }
    assert!(!w.blocked(f.x, f.z, 1.0, top + 1.067 + 0.3, top + 30.0), "over the barriers");
    assert!(w.blocked(f.x, f.z, 0.0, top - depth * 0.5, top - depth * 0.5), "inside the girder");
    // a column fills its disc from its footing to the soffit; its margin is the caller's.
    let b = d.supports.iter().find(|b| !b.columns.is_empty()).expect("a bent with columns");
    let col = &b.columns[0];
    assert!(w.blocked(col.x, col.z, 0.0, col.ground, col.ground + 1.0));
    assert!(w.obstructions(col.x, col.z, 0.0).iter().any(|o| o.part == crate::Part::Column));
    // nothing anywhere in a world without decks.
    let e = StructureWorld::empty();
    assert!(e.obstructions(f.x, f.z, 50.0).is_empty() && !e.blocked(f.x, f.z, 50.0, -1e9, 1e9) && e.headroom(f.x, f.z, 5.0, 0.0).is_none());
}

/// A luminaire's footing on a barrier (`barrier_at`): on top of the barrier
/// as drawn, at the deck's edge, facing over the roadway; none where the
/// barrier is left out (a merge).
#[test]
fn barrier_points_stand_on_the_drawn_barriers() {
    let c = deck_over_street();
    let w = meshed(&c, vec![], StructureOptions::default());
    let d = built(&w, "101");
    let concrete = all_tris(&w, 0, &|cl| cl == StructureClass::Concrete);
    let barrier_h = standards().metric("concrete-barrier-type-60m-height").unwrap();
    let mut n = 0;
    for k in 1..20 {
        let s = d.length() * k as f64 / 20.0;
        let pts = w.barrier_at(d.id, s);
        assert_eq!(pts.len(), 2, "both barriers at station {s}");
        for p in pts {
            let f = d.frame(s);
            assert!(((p.x - f.x) * p.inward[0] + (p.z - f.z) * p.inward[1]) < -d.half_at(s), "outside the roadway");
            assert!((p.top_y - p.deck_y - barrier_h).abs() < 1e-9);
            if d.ends.iter().all(|e| (e.at[0] - p.x).hypot(e.at[1] - p.z) > 15.0) {
                assert!((p.deck_y - d.y_at(s)).abs() < 0.01, "deck top {} vs the profile {}", p.deck_y, d.y_at(s));
            }
            // the drawn barrier's top under it.
            let top = ray_down(&concrete, p.x, p.z).expect("a barrier under the point");
            assert!((top - p.top_y).abs() < 0.02, "drawn barrier top {top} vs {}", p.top_y);
            n += 1;
        }
    }
    assert!(n == 38);
    assert!(w.barrier_at(d.id, -1.0).is_empty() && w.barrier_at(9999, 1.0).is_empty());
    // a merge drops the barrier between the two decks there.
    let m = ramp_merge();
    let w = meshed(&m, vec![], StructureOptions::default());
    let ramp = built(&w, "2204");
    let single = (1..40).filter(|&s| w.barrier_at(ramp.id, s as f64).len() < 2).count();
    assert!(single > 10, "stations with a barrier left out at the gore: {single}");
}
