//! The structures on the real city, loaded as eo-app loads it: prints the
//! census and the held chains (`--nocapture`), and checks that the I-80
//! approach gets a deck and that every held chain says why.

use std::sync::Arc;

use eo_street::{BuildingIndex, StreetCity, StreetOptions, StreetWorld, Terrain};
use eo_structures::{ChainKind, ColumnSite, Hold, RoadClass, StructureOptions, StructureWorld};

fn world() -> StructureWorld {
    let paths = eo_data::DataPaths::discover().expect("repository data");
    let city = eo_data::City::load(&paths).expect("city");
    let grid = Arc::new(eo_data::ElevationGrid::load(&paths).expect("elevation"));
    let o = StreetOptions::load().expect("street options");
    let sc = StreetCity::load().expect("street city");
    let plan = eo_street::plan::build_plan(&sc, &o);
    let rings = city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
    let lift = o.open_ground_lift;
    let streets = StreetWorld::from_plan(o, sc, plan, Terrain { grid, carpet_lift: lift }, BuildingIndex::new(rings));
    let ca = eo_data::CaStandards::load(&paths).expect("standards");
    StructureWorld::build(&streets, &city, &ca, eo_landmarks::LandmarksJs::get(), StructureOptions::default()).expect("structures build")
}

#[test]
fn the_city_builds_the_i80_approach_and_every_hold_has_a_reason() {
    let w = world();
    let census = w.census();
    println!("{census}");
    for c in w.held() {
        println!("held: chain {} {} level {} {:.0} m ways {} [{}]: {}", c.id, c.class.name(), c.level, c.length(), c.ways().join(","), c.name(), c.hold().unwrap());
    }
    // the I-80 approach over SoMa, from 5th Street to the Bay Bridge
    // anchorage (on the plan before the ground scale: x 1200..2900, z -1300..300).
    let s = eo_data::groundscale::scale();
    let in_soma = |p: &[f64; 2]| (1200.0 * s..2900.0 * s).contains(&p[0]) && (-1300.0 * s..300.0 * s).contains(&p[1]);
    let approach: Vec<&eo_structures::Chain> = w
        .chains()
        .iter()
        .filter(|c| c.is_built() && c.kind == ChainKind::Deck && c.class == RoadClass::Motorway)
        .filter(|c| c.pieces.iter().any(|p| p.name.contains("80") || p.name.contains("Eisenhower")))
        .filter(|c| c.points.iter().any(in_soma))
        .collect();
    println!("I-80 approach decks built: {}", approach.iter().map(|c| format!("{} ({:.0} m)", c.id, c.length())).collect::<Vec<_>>().join(", "));
    assert!(!approach.is_empty(), "no deck built over the I-80 approach");
    for c in &approach {
        // a deck: above the street datum somewhere, every way on level 1 or more.
        assert!(c.level >= 1 && c.heights.len() == c.stations.len());
        assert!(c.ways().iter().all(|wy| w.way_level(wy) >= 1));
    }
    // every held chain says why, naming what it could not clear or cross.
    assert!(census.held > 0 && census.held == w.held().count());
    for c in w.held() {
        let h = c.hold().expect("a held chain has a reason");
        assert!(!h.to_string().is_empty());
        match h {
            Hold::EndTooLow { over } => assert!(!over.is_empty()),
            Hold::UnknownCrossing { with } => assert!(!with.is_empty()),
            Hold::NoBentRoom { span } => assert!(*span as f64 > eo_structures::LONGEST_SPAN_M - 1.0),
            Hold::Grade | Hold::NoSourcedClearance => {}
        }
    }
    // what was built keeps its own limits.
    assert_eq!(census.clearance_short, 0);
    assert_eq!(census.grade_over, 0);
    assert!(census.landmark >= 3, "landmark decks: {}", census.landmark);
}

/// Height of the triangle over plan point (x, z), if it is inside it.
fn over(t: &[[f32; 3]; 3], x: f64, z: f64) -> Option<f64> {
    let p = |i: usize| [t[i][0] as f64, t[i][1] as f64, t[i][2] as f64];
    let (a, b, c) = (p(0), p(1), p(2));
    let det = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
    if det.abs() < 1e-12 {
        return None;
    }
    let l1 = ((b[2] - c[2]) * (x - c[0]) + (c[0] - b[0]) * (z - c[2])) / det;
    let l2 = ((c[2] - a[2]) * (x - c[0]) + (a[0] - c[0]) * (z - c[2])) / det;
    let l3 = 1.0 - l1 - l2;
    (l1 >= -1e-9 && l2 >= -1e-9 && l3 >= -1e-9).then(|| l1 * a[1] + l2 * b[1] + l3 * c[1])
}

/// The triangles of the near meshes of a tile and its eight neighbours (a
/// piece belongs to the tile of its midpoint, so it may reach into the next)
/// (`keep`: by class).
fn tile_tris(w: &StructureWorld, key: (i32, i32), keep: &dyn Fn(u8) -> bool) -> Vec<[[f32; 3]; 3]> {
    let mut out = vec![];
    for dz in -1..=1 {
        for dx in -1..=1 {
            let Some(m) = w.render_tile_shared((key.0 + dx, key.1 + dz), 0) else { continue };
            out.extend(m.indices.chunks_exact(3).filter(|t| keep(m.classes[t[0] as usize])).map(|t| [m.positions[t[0] as usize], m.positions[t[1] as usize], m.positions[t[2] as usize]]));
        }
    }
    out
}

#[test]
fn the_city_meshes_clear_their_crossings_stand_off_paving_and_meet_their_streets() {
    let paths = eo_data::DataPaths::discover().expect("repository data");
    let city = eo_data::City::load(&paths).expect("city");
    let grid = Arc::new(eo_data::ElevationGrid::load(&paths).expect("elevation"));
    let o = StreetOptions::load().expect("street options");
    let sc = StreetCity::load().expect("street city");
    let plan = eo_street::plan::build_plan(&sc, &o);
    let rings = city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
    let lift = o.open_ground_lift;
    let streets = StreetWorld::from_plan(o, sc, plan, Terrain { grid, carpet_lift: lift }, BuildingIndex::new(rings));
    let ca = eo_data::CaStandards::load(&paths).expect("standards");
    let w = StructureWorld::build(&streets, &city, &ca, eo_landmarks::LandmarksJs::get(), StructureOptions::default()).expect("structures build");
    let st = w.mesh_stats();
    println!("meshes: {} tiles; triangles near {} far {} coarse {}; {:.1} / {:.1} / {:.1} MB", st.tiles, st.triangles[0], st.triangles[1], st.triangles[2], st.bytes[0] as f64 / 1048576.0, st.bytes[1] as f64 / 1048576.0, st.bytes[2] as f64 / 1048576.0);
    println!("collision: {} deck-surface, {} barrier triangles", st.street_triangles, st.barrier_triangles);
    assert!(st.tiles > 100 && st.triangles[0] > st.triangles[1] && st.triangles[1] > st.triangles[2] && st.triangles[2] > 0);
    // every tile listed has something to draw at some level or to collide with.
    for key in w.tile_keys() {
        assert!((0..3).any(|l| w.render_tile_shared(key, l).is_some()) || w.surface_tile(key).is_some(), "tile {key:?} has nothing");
    }

    // clearance: a vertical ray from the crossed street (on its carriageway,
    // under the deck's centreline) or the crossed deck, up to the first
    // structure triangle.
    let mut rays = 0usize;
    let mut worst = f64::INFINITY;
    // (not where a column stands in a parking lane: its capital is over it.)
    let cols_all = w.all_column_footprints();
    let by_column = |x: f64, z: f64| cols_all.iter().any(|f| (f.x - x).hypot(f.z - z) < f.radius * std::f64::consts::SQRT_2 + 0.6);
    for c in w.chains().iter().filter(|c| c.is_built() && c.kind == ChainKind::Deck) {
        for x in &c.crossings {
            if x.chain.is_some_and(|d| !w.chain(d).unwrap().is_built()) {
                continue;
            }
            let n = (((x.s1 - x.s0) / 1.0).ceil() as usize).max(1);
            for k in 0..=n {
                let s = x.s0 + (x.s1 - x.s0) * k as f64 / n as f64;
                let f = c.frame(s);
                if by_column(f.x, f.z) {
                    continue;
                }
                let surface = match x.chain {
                    None => {
                        if streets.paved_layer_at(f.x, f.z) != Some(eo_street::Layer::Carriageway) {
                            continue;
                        }
                        let Some(y) = streets.paved_at(f.x, f.z) else { continue };
                        y
                    }
                    Some(d) => {
                        let lower = w.chain(d).unwrap();
                        let (sd, dist) = lower.project(f.x, f.z);
                        if dist > lower.half_at(sd) - 0.3 {
                            continue;
                        }
                        // the lower deck as drawn (its crown near a paved end).
                        let y0 = lower.y_at(sd);
                        let decks = tile_tris(&w, eo_street::tile_key(f.x, f.z), &|cl| cl == eo_structures::StructureClass::DeckSurface as u8);
                        decks.iter().filter_map(|t| over(t, f.x, f.z)).filter(|y| (y - y0).abs() < 0.5).fold(y0, f64::max)
                    }
                };
                let tris = tile_tris(&w, eo_street::tile_key(f.x, f.z), &|_| true);
                let hit = tris.iter().filter_map(|t| over(t, f.x, f.z)).filter(|&y| y > surface + 0.05).fold(f64::INFINITY, f64::min);
                assert!(hit.is_finite(), "chain {} over {}: no structure over the crossing at station {s:.1}", c.id, x.over);
                worst = worst.min(hit - surface - x.clearance);
                rays += 1;
            }
        }
    }
    println!("clearance rays: {rays}, least margin over the clearance asked {worst:.3} m");
    assert!(rays > 1000);

    // columns: off paving (or wholly in a parking lane / furniture zone, by
    // their site), never under a lower deck.
    let cols = w.all_column_footprints();
    let mut sampled = 0;
    for f in &cols {
        let level = w.chain(f.chain).unwrap().level;
        for k in 0..=8 {
            let (x, z) = if k == 8 {
                (f.x, f.z)
            } else {
                let a = k as f64 * std::f64::consts::TAU / 8.0;
                (f.x + 0.98 * f.radius * a.cos(), f.z + 0.98 * f.radius * a.sin())
            };
            let layer = streets.paved_layer_at(x, z);
            match f.site {
                ColumnSite::OffPaving => assert!(layer.is_none() || layer == Some(eo_street::Layer::Verge), "chain {} bent {} column at ({x:.1}, {z:.1}) on {layer:?}", f.chain, f.bent),
                ColumnSite::ParkingLane => assert!(matches!(layer, Some(eo_street::Layer::Carriageway | eo_street::Layer::Marking)), "chain {} bent {} parking-lane column on {layer:?}", f.chain, f.bent),
                ColumnSite::FurnitureZone => assert!(matches!(layer, Some(eo_street::Layer::Sidewalk | eo_street::Layer::CurbTop | eo_street::Layer::Marking)), "chain {} bent {} furniture-zone column on {layer:?}", f.chain, f.bent),
                ColumnSite::Median => {}
            }
            assert!(w.decks_at(x, z).iter().all(|(l, _)| *l >= level), "chain {} bent {} column under a lower deck", f.chain, f.bent);
            sampled += 1;
        }
    }
    let street_cols = cols.iter().filter(|f| f.site.on_street()).count();
    println!("columns: {} ({} in a street), {sampled} footprint samples", cols.len(), street_cols);
    assert!(street_cols > 0 && cols.iter().filter(|f| f.site.on_street()).all(|f| f.street.is_some()));

    // abutments: the deck top meets the paved stub at its end. A stub is a
    // street run ending at the chain's end away from any junction; an end in
    // a junction's pad or on a street's side meets a surface the street
    // shapes for itself (stage A6 joins those), so it is only reported.
    let snap = streets.o.node_snap;
    // (the stub run's half width, when the end is a stub.)
    let stub_at = |p: [f64; 2]| -> Option<f64> {
        let run = streets.plan.entries.iter().find(|e| [e.points[0], *e.points.last().unwrap()].iter().any(|q| (q[0] - p[0]).hypot(q[1] - p[1]) <= snap))?;
        let junction = streets.plan.nodes.iter().any(|n| (n.position[0] - p[0]).hypot(n.position[1] - p[1]) < 30.0);
        (!junction).then_some(run.half)
    };
    let mut stub_gaps: Vec<f64> = vec![];
    let mut gaps: Vec<f64> = vec![];
    for c in w.chains().iter().filter(|c| c.is_built() && c.kind == ChainKind::Deck) {
        for (e, end) in c.ends.iter().enumerate() {
            if end.kind != eo_structures::EndKind::Paved {
                continue;
            }
            let s_end = if e == 0 { 0.0 } else { c.length() };
            let f = c.frame(s_end);
            let out = if e == 0 { [-f.tx, -f.tz] } else { [f.tx, f.tz] };
            let tris = tile_tris(&w, eo_street::tile_key(f.x, f.z), &|cl| cl == eo_structures::StructureClass::DeckSurface as u8);
            for u in [-c.half_at(s_end) / 2.0, 0.0, c.half_at(s_end) / 2.0] {
                let at = |k: f64| [f.x + f.nx * u + out[0] * k, f.z + f.nz * u + out[1] * k];
                let (ps, pd) = (at(0.05), at(-0.05));
                // (this chain's deck: the deck surface nearest its own end height.)
                let y_end = c.y_at(s_end);
                let deck = tris.iter().filter_map(|t| over(t, pd[0], pd[1])).fold(None, |m: Option<f64>, y| Some(m.map_or(y, |v| if (y - y_end).abs() < (v - y_end).abs() { y } else { v })));
                let (Some(stub), Some(deck)) = (streets.paved_at(ps[0], ps[1]), deck) else { continue };
                gaps.push((stub - deck).abs());
                // (on the stub's own carriageway, inside its width.)
                let on_stub = stub_at(end.at).is_some_and(|half| u.abs() <= half - 0.3) && streets.paved_layer_at(ps[0], ps[1]) == Some(eo_street::Layer::Carriageway);
                if on_stub {
                    stub_gaps.push((stub - deck).abs());
                }
            }
        }
    }
    gaps.sort_by(|a, b| a.total_cmp(b));
    let within = gaps.iter().filter(|&&g| g <= 0.02).count();
    let pct = |p: f64| gaps[((gaps.len() - 1) as f64 * p) as usize];
    println!("abutments: {} samples at paved ends, {} within 2 cm; median {:.3} m, 90th {:.3} m, max {:.3} m", gaps.len(), within, pct(0.5), pct(0.9), gaps.last().copied().unwrap_or(0.0));
    stub_gaps.sort_by(|a, b| a.total_cmp(b));
    let stub_within = stub_gaps.iter().filter(|&&g| g <= 0.02).count();
    println!("  on stubs' carriageways away from junctions: {} samples, {} within 2 cm, median {:.3} m, max {:.3} m", stub_gaps.len(), stub_within, stub_gaps.get(stub_gaps.len() / 2).copied().unwrap_or(0.0), stub_gaps.last().copied().unwrap_or(0.0));
    assert!(gaps.len() > 100 && stub_gaps.len() > 30);
    assert!(stub_within * 100 >= stub_gaps.len() * 95, "{stub_within} of {} stub samples within 2 cm", stub_gaps.len());
}

/// Every drawn triangle of the city's decks (every tile's near level but
/// the downspouts, which props keep their margin from, and the open roads
/// below other ways, drawn at grade) lies inside the clearance envelope
/// props keep out of (`StructureWorld::obstructions`, stage A6): its
/// centroid and its corners pulled 2 cm toward it.
#[test]
fn the_city_clearance_envelope_holds_every_drawn_triangle() {
    let w = world();
    // (a road below others is drawn on the ground: nothing to keep out of.)
    let below: Vec<&eo_structures::Chain> = w.chains().iter().filter(|c| c.is_built() && c.kind == ChainKind::Below).collect();
    let on_below = |x: f64, z: f64| below.iter().any(|c| {
        let (s, d) = c.project(x, z);
        d <= c.half_at(s) + 1.5
    });
    let (mut points, mut tris, mut outside) = (0usize, 0usize, vec![]);
    for key in w.tile_keys() {
        let Some(m) = w.render_tile_shared(key, 0) else { continue };
        for t in m.indices.chunks_exact(3) {
            if m.classes[t[0] as usize] == eo_structures::StructureClass::Metal as u8 {
                continue;
            }
            tris += 1;
            let p = |i: u32| m.positions[i as usize].map(|v| v as f64);
            let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
            let g = [(a[0] + b[0] + c[0]) / 3.0, (a[1] + b[1] + c[1]) / 3.0, (a[2] + b[2] + c[2]) / 3.0];
            let mut pts = vec![g];
            for q in [a, b, c] {
                let d = ((g[0] - q[0]).powi(2) + (g[1] - q[1]).powi(2) + (g[2] - q[2]).powi(2)).sqrt().max(1e-9);
                let k = (0.02 / d).min(1.0);
                pts.push([q[0] + (g[0] - q[0]) * k, q[1] + (g[1] - q[1]) * k, q[2] + (g[2] - q[2]) * k]);
            }
            for q in pts {
                points += 1;
                let obs = w.obstructions(q[0], q[2], 0.0);
                // (the mesh is f32: a millimetre of rounding.)
                let gap = obs.iter().map(|o| if q[1] < o.lo { o.lo - q[1] } else if q[1] > o.hi { q[1] - o.hi } else { 0.0 }).fold(f64::INFINITY, f64::min);
                if gap > 2e-3 && !on_below(q[0], q[2]) {
                    outside.push((q, gap, key));
                }
            }
        }
    }
    outside.sort_by(|a, b| b.1.total_cmp(&a.1));
    println!("{points} points of {tris} drawn triangles: {} outside the envelope; worst {:?}", outside.len(), outside.iter().take(5).collect::<Vec<_>>());
    assert!(tris > 500_000, "{tris} triangles");
    assert!(outside.is_empty(), "{} drawn points outside the clearance envelope", outside.len());
}
