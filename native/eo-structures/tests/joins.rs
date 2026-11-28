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
use eo_structures::{ChainKind, StructureOptions, StructureWorld};

fn plain_streets(city: &eo_data::City, grid: Arc<eo_data::ElevationGrid>) -> StreetWorld {
    let o = StreetOptions::load().expect("street options");
    let sc = StreetCity::load().expect("street city");
    let plan = eo_street::plan::build_plan(&sc, &o);
    let rings = city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
    let lift = o.open_ground_lift;
    StreetWorld::from_plan(o, sc, plan, Terrain { grid, carpet_lift: lift }, BuildingIndex::new(rings))
}

fn hit_height([a, b, c]: [[f64; 3]; 3], x: f64, z: f64) -> Option<f64> {
    let (ax, az, bx, bz, cx, cz) = (a[0], a[2], b[0], b[2], c[0], c[2]);
    let det = (bz - cz) * (ax - cx) + (cx - bx) * (az - cz);
    if det.abs() < 1e-12 {
        return None;
    }
    let wa = ((bz - cz) * (x - cx) + (cx - bx) * (z - cz)) / det;
    let wb = ((cz - az) * (x - cx) + (ax - cx) * (z - cz)) / det;
    let wc = 1.0 - wa - wb;
    (wa >= -1e-6 && wb >= -1e-6 && wc >= -1e-6).then(|| wa * a[1] + wb * b[1] + wc * c[1])
}

/// A vertical wheel ray into the same deck triangles the collision world cooks.
/// Neighbour tiles can own a span crossing a tile boundary.
fn deck_hit(w: &StructureWorld, x: f64, z: f64, y: f64) -> bool {
    let (kx, kz) = eo_street::tile_key(x, z);
    (-1..=1).any(|dx| {
        (-1..=1).any(|dz| {
            w.surface_tile_shared((kx + dx, kz + dz)).is_some_and(|m| {
                m.street
                    .iter()
                    .any(|&t| hit_height(t, x, z).is_some_and(|h| (h - y).abs() < 0.35))
            })
        })
    })
}

/// Raycast the two east-span ways from end to end, including the island
/// transition. Confirm a grade-limited surface joins each landmark approach.
fn bridge_surfaces(w: &StructureWorld) {
    for way in [
        "236348361",
        "237731428",
        "1343738800",
        "661905443",
        "661905445",
        "661905446",
        "537838948",
        "595194543",
    ] {
        let chains: Vec<_> = w
            .chains()
            .iter()
            .filter(|c| c.kind == eo_structures::ChainKind::Deck && c.ways().contains(&way))
            .collect();
        assert!(!chains.is_empty(), "missing bridge way {way}");
        for c in chains {
            assert!(c.is_built(), "bridge way {way}: {:?}", c.status);
            for (ys, (ss, grade)) in c
                .heights
                .windows(2)
                .zip(c.stations.windows(2).zip(&c.grade_limits))
            {
                assert!(
                    (ys[1] - ys[0]).abs() <= grade * (ss[1] - ss[0]) + 1e-5,
                    "grade on {way}"
                );
            }
            let check = |s: f64| {
                let f = c.frame(s);
                assert!(
                    deck_hit(w, f.x, f.z, c.y_at(s)),
                    "missing collision on {way} at {s}"
                );
            };
            check(c.length() - 0.1);
            check(0.1);
            for s in (1..c.length() as usize).step_by(25) {
                check(s as f64);
            }
            if let Some(profile) = eo_data::bridge_survey::east_span(way) {
                let f = c.frame(c.length() / 2.0);
                assert!(
                    (c.y_at(c.length() / 2.0) - eo_data::bridge_survey::height_at(profile, f.x))
                        .abs()
                        < 1.0,
                    "east span missed its survey on {way}"
                );
            }
        }
    }
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
    let tri_h = |m: &TileMesh, t: usize, x: f64, z: f64| {
        let tri = [m.indices[3*t], m.indices[3*t+1], m.indices[3*t+2]].map(|i| m.positions[i as usize].map(f64::from));
        hit_height(tri, x, z)
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
    bridge_surfaces(w);
    // At-grade through lanes at divided junctions. A wheel crossing one of
    // these positions used to find a folded road, a second shoulder, or the
    // corner of the opposite carriageway on top of its asphalt.
    for world in [&plain, streets] {
        for (name, survey) in [
            ("Lake Merced at Middlefield", [-5926.18919601, 5346.51141757]),
            ("San Jose through lane", [-3023.76057908, 7179.39558551]),
            ("San Jose at Broad", [-3019.65160000, 7174.08360000]),
            ("Market at Castro", [-1402.73676123, 1696.12605330]),
            ("Market at the bend", [-1403.09043735, 1698.62752994]),
            ("Bayshore Boulevard", [1800.01291184, 6926.51495674]),
        ] {
            let [x, z] = eo_data::groundscale::plan(survey);
            let level = world.carriageway_near(x, z, world.ground(x, z)).expect(name);
            let contacts = world.paved_heights(x, z);
            assert!(contacts.iter().all(|y| (y - level).abs() <= 0.0005),
                "{name} at {x}, {z}: asphalt at {level}, walkable heights {contacts:?}");
        }
    }
    // At these two lane positions a neighbouring shoulder used to cross
    // into the asphalt. Check the drawn triangles on both sides of tile seams.
    for (name, x, z, level) in [
        ("Saint Charles Avenue", -5492.07628285, 9619.54528420, 95.5354),
        ("Sickles Avenue", -3959.63982137, 9289.27133176, 113.8538),
    ] {
        let y = streets.carriageway_near(x, z, level).expect(name);
        assert!((y - level).abs() < 0.02, "{name}: pavement at {y}");
        let (kx, kz) = eo_street::tile_key(x, z);
        for dx in -1..=1 {
            for dz in -1..=1 {
                let mesh = streets.cached_mesh((kx + dx, kz + dz));
                for (indices, layer) in mesh.indices.chunks_exact(3).zip(&mesh.layers) {
                    if *layer != Layer::Verge { continue; }
                    let tri = [0, 1, 2].map(|i| mesh.positions[indices[i] as usize].map(f64::from));
                    if let Some(bank) = hit_height(tri, x, z) {
                        assert!((bank - y).abs() >= streets.o.marking_lift,
                            "{name}: shoulder at {bank}, carriageway at {y}");
                    }
                }
            }
        }
    }
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

/// Every sourced vehicle bridge and highway has a driving surface in the complete world.
#[test]
fn full_city_highways_and_bridge_wheel_tracks_are_continuous() {
    use std::collections::{HashMap, HashSet};

    let paths = eo_data::DataPaths::discover().expect("repository");
    let city = eo_data::City::load(&paths).expect("city");
    let grid = Arc::new(eo_data::ElevationGrid::load(&paths).expect("elevation"));
    let standards = eo_data::CaStandards::load(&paths).expect("standards");
    let plain = plain_streets(&city, grid);
    let joined = eo_structures::build_joined(
        &plain, &city, &standards, eo_landmarks::LandmarksJs::get(),
        StructureOptions { tunnels: true },
    ).expect("complete network");
    let decks = &joined.structures;
    let streets = joined.streets.as_ref().unwrap_or(&plain);
    check_golden_gate_abutment_pavement(decks, streets);
    check_veterans_boulevard_retaining_wall(decks);
    let census = decks.census();
    assert_eq!(census.chains, census.built + census.landmark);
    assert_eq!((census.unjoined, census.degenerate, census.clearance_short, census.grade_over), (0, 0, 0, 0));

    // Both ends of each closed leaf meet a fixed span at the surveyed level.
    for span in eo_data::movable_spans::spans() {
        let way = span.road.id.key();
        let leaf = decks.chains().iter().find(|c| c.ways().contains(&way.as_str())).expect("movable span");
        assert!(leaf.bents.is_empty(), "pier under movable way {way}");
        for (i, end) in leaf.ends.iter().enumerate() {
            assert_eq!(end.kind, eo_structures::EndKind::Shared, "unjoined hinge on {way}");
            let fixed: Vec<_> = decks.chains().iter().filter(|c| c.id != leaf.id && c.is_built())
                .flat_map(|c| c.ends.iter()).filter(|e| e.node == end.node).collect();
            assert_eq!(fixed.len(), 1, "fixed approach at hinge {i} of {way}");
            assert_eq!(end.fixed, fixed[0].fixed);
        }
        for &s in &leaf.stations {
            let y = eo_data::bridge_survey::movable_height(&way, leaf.frame(s).z).expect("lidar deck survey");
            assert!((leaf.y_at(s) - y).abs() < 0.35, "closed deck of {way} at {s}");
        }
    }

    // Index actual collision faces including pieces whose owner tile is across a seam.
    let cell = |x: f64| (x / 12.0).floor() as i32;
    let mut cells: HashMap<(i32, i32), Vec<[[f64; 3]; 3]>> = HashMap::new();
    for key in decks.tile_keys() {
        let Some(tile) = decks.surface_tile_shared(key) else { continue };
        for &triangle in &tile.street {
            let range = |axis: usize| {
                let coords = triangle.map(|p| cell(p[axis]));
                *coords.iter().min().unwrap()..=*coords.iter().max().unwrap()
            };
            for x in range(0) {
                for z in range(2) {
                    cells.entry((x, z)).or_default().push(triangle);
                }
            }
        }
    }
    let mut drawn = HashSet::new();
    let mut deck_probes = 0;
    for chain in decks.chains() {
        drawn.extend(chain.ways());
        let length = chain.length();
        let count = (length / 4.0).ceil().max(1.0) as usize;
        let inset = 0.01_f64.min(length / 2.0);
        for i in 0..=count {
            let s = (length * i as f64 / count as f64).clamp(inset, length - inset);
            let f = chain.frame(s);
            for fraction in [-0.45, 0.0, 0.45] {
                let [x, z] = f.at(chain.half_at(s) * fraction);
                let y = chain.y_at(s);
                let hit = cells.get(&(cell(x), cell(z))).into_iter().flatten()
                    .any(|&face| hit_height(face, x, z).is_some_and(|h| (h - y).abs() < 0.35));
                assert!(hit, "{} chain {} at {s:.2} m, lateral fraction {fraction}", chain.name(), chain.id);
                deck_probes += 1;
            }
        }
    }

    let source: HashSet<_> = city.roads.iter().filter(|r| {
        matches!(r.highway.as_str(), "motorway" | "motorway_link" | "trunk" | "trunk_link")
            || (matches!(r.highway.as_str(), "primary" | "primary_link")
                && matches!(r.name.as_str(), "CA 35" | "Sloat Boulevard" | "Skyline Boulevard"))
            || (eo_data::standards::is_vehicle_way(&r.highway)
                && (r.bridge || r.name.split_whitespace().any(|word| matches!(word, "Highway" | "Freeway" | "Expressway"))))
    }).map(|r| r.id.as_str()).collect();
    let mut paved_probes = 0;
    for (ei, e) in streets.plan.entries.iter().enumerate().filter(|(_, e)| e.structure_leg.is_none()) {
        let source_ways = e.pieces.iter().filter_map(|&i| streets.city.segments[i].street_id.as_deref())
            .filter_map(|s| s.strip_prefix("sf-street-")).filter(|id| source.contains(id)).collect::<Vec<_>>();
        if source_ways.is_empty() { continue; }
        drawn.extend(source_ways);
        for f in &streets.geoms[ei].stations {
            if f.s > e.s0() + 0.01 && f.s < e.s1() - 0.01 {
                assert!(streets.paved_at(f.x, f.z).is_some(), "paving gap on {} at {:.2} m", e.id, f.s);
                paved_probes += 1;
            }
        }
    }
    let absent: BTreeSet<_> = source.difference(&drawn).collect();
    assert!(absent.is_empty(), "no driving surface for OSM ways {absent:?}");
    println!("{} source ways, {deck_probes} structure wheel probes, {paved_probes} street probes", source.len());
}

fn check_veterans_boulevard_retaining_wall(world: &StructureWorld) {
    // Look across the wall at mid-height where the lower and upper paving
    // converge. A vertical wheel probe cannot see these retaining faces.
    let y = 52.5;
    for (z, wall_x) in [
        (-3216.3, -5517.0..=-5515.8),
        (-3222.4, -5517.5..=-5516.2),
    ] {
        let (kx, kz) = eo_street::tile_key(*wall_x.end(), z);
        let has_wall = world.surface_tile_shared((kx, kz)).is_some_and(|tile| {
            tile.barrier.iter().any(|&triangle| {
                let side_view = triangle.map(|[x, y, z]| [y, x, z]);
                hit_height(side_view, y, z).is_some_and(|x| wall_x.contains(&x))
            })
        });
        assert!(has_wall, "Veterans Boulevard is missing retaining concrete at y={y}, z={z}");
    }
}

/// At these three survey wheel tracks the carriageway ends diagonally across
/// the landmark's last panel. The joining pavement must be visible at every
/// drawing distance and support the tire at the same height.
fn check_golden_gate_abutment_pavement(world: &StructureWorld, streets: &StreetWorld) {
    // The northbound deck has a direct paved landfall at its southern end.
    // Use the street mesh on the approach side so a higher overlapping
    // bridge triangle cannot conceal a step in the street itself.
    let northbound = world.chains().iter()
        .find(|c| c.kind == ChainKind::Landmark && c.ways().contains(&"595194543"))
        .expect("northbound Golden Gate deck");
    let tip = northbound.frame(0.0);
    let approach = streets.plan.entries.iter()
        .find(|entry| streets.city.segments[entry.road].street_id.as_deref() == Some("sf-street-26968360"))
        .expect("northbound Golden Gate approach");
    let [previous, end] = approach.points.as_slice() else { panic!("northbound approach chord") };
    let length = (end[0] - previous[0]).hypot(end[1] - previous[1]);
    let [x, z] = [end[0] - (end[0] - previous[0]) / length, end[1] - (end[1] - previous[1]) / length];
    assert_eq!(*end, [tip.x, tip.z]);
    let deck = northbound.y_at(0.0);
    let pavement = streets.carriageway_near(x, z, deck).expect("northbound approach paving");
    assert!((deck - pavement).abs() < 0.15, "northbound Golden Gate joint steps from {pavement} to {deck}");

    for [x, y, z] in [
        [-6293.37424, 69.05151, -4157.37212],
        [-6274.60958, 69.17423, -4166.29145],
        [-6731.40817, 90.05128, -7497.11582],
    ] {
        let (kx, kz) = eo_street::tile_key(x, z);
        let keys = || (-1..=1).flat_map(move |dx| (-1..=1).map(move |dz| (kx + dx, kz + dz)));
        let surface = keys()
            .filter_map(|key| world.surface_tile_shared(key))
            .filter_map(|tile| tile.street.iter().filter_map(|&tri| hit_height(tri, x, z))
                .find(|&top| (top - y).abs() < 0.15))
            .next().expect("Golden Gate approach wheel track");
        for lod in 0..3 {
            let matches = keys().filter_map(|key| world.render_tile_shared(key, lod)).any(|tile| {
                tile.indices.chunks_exact(3).any(|tri| {
                    let i = tri[0] as usize;
                    if tile.classes[i] != eo_structures::StructureClass::DeckSurface as u8 {
                        return false;
                    }
                    let points = [tri[0], tri[1], tri[2]].map(|j| tile.positions[j as usize].map(f64::from));
                    hit_height(points, x, z).is_some_and(|h| (h - surface).abs() < 0.002)
                })
            });
            assert!(matches, "Golden Gate asphalt missing at ({x}, {z}), level of detail {lod}, tire height {surface}");
        }
    }
    // The west curb return previously crossed the south approach mouth,
    // 7 cm above the driving surface. Check the drawn street layers here.
    let (x, z) = (-6288.568, -4155.509);
    let (kx, kz) = eo_street::tile_key(x, z);
    for dx in -1..=1 {
        for dz in -1..=1 {
            let m = streets.cached_mesh((kx + dx, kz + dz));
            for (tri, layer) in m.indices.chunks_exact(3).zip(&m.layers) {
                if !matches!(layer, Layer::Sidewalk | Layer::CurbTop | Layer::Ramp) { continue; }
                let points = [tri[0], tri[1], tri[2]].map(|i| m.positions[i as usize].map(f64::from));
                assert!(!hit_height(points, x, z).is_some_and(|h| (h - 68.94).abs() < 0.15),
                    "{layer:?} crosses the south approach at ({x}, {z})");
            }
        }
    }
}

/// A sampling grid can see pavement on both sides of a vertical drop. Check
/// the actual height step across each lane at the joined lower island portal.
#[test]
fn the_eastbound_island_road_meets_its_street_in_every_lane() {
    const NAME: &str = "the_eastbound_island_road_meets_its_street_in_every_lane";
    if !std::env::args().any(|s| s == "--yerba-buena-section") {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", NAME, "--", "--yerba-buena-section"])
            .status().unwrap();
        assert!(status.success());
        return;
    }
    let paths = eo_data::DataPaths::discover().unwrap();
    let city = eo_data::City::load(&paths).unwrap();
    let ca = eo_data::CaStandards::load(&paths).unwrap();
    let plain = plain_streets(&city, Arc::new(eo_data::ElevationGrid::load(&paths).unwrap()));
    let joined = eo_structures::build_joined(
        &plain, &city, &ca, eo_landmarks::LandmarksJs::get(), StructureOptions { tunnels: true },
    ).unwrap();
    let streets = joined.streets.as_ref().unwrap();
    let world = &joined.structures;
    let bore = world.chains().iter().find(|c| c.ways().contains(&"50691047")).unwrap();
    let end = usize::from(bore.points[0][0] < bore.points.last().unwrap()[0]);
    let station = if end == 0 { 0.0 } else { bore.length() };
    let dir = if end == 0 { -1.0 } else { 1.0 };
    let frame = bore.frame(station);
    for fraction in [-0.8, -0.4, 0.0, 0.4, 0.8] {
        let [x,z] = frame.at(bore.half_at(station) * fraction);
        let dx = 0.25 * dir * frame.tx;
        let dz = 0.25 * dir * frame.tz;
        let deck = world.deck_top(bore.id, x-dx, z-dz).expect("tunnel floor");
        let street = streets.carriageway_near(x+dx, z+dz, deck).expect("next road");
        assert!((street-deck).abs() < 0.10, "lateral fraction {fraction}: {deck} to {street}");
        // The street's first spans and the tunnel's last spans must also
        // share a tangent. A continuous height alone allowed a 50 mph car
        // to lift all four tyres off the road over a sharp crest.
        if fraction == 0.0 {
            let d = 2.0;
            let tunnel = world.deck_top(bore.id, x-dir*d*frame.tx, z-dir*d*frame.tz).unwrap();
            let approach = streets.carriageway_near(x+dir*d*frame.tx, z+dir*d*frame.tz, street).unwrap();
            let slope_in = (deck-tunnel)/(d-0.25);
            let slope_out = (approach-street)/(d-0.25);
            assert!((slope_in-slope_out).abs() < streets.o.cross_slope,
                "abrupt change along the road from {slope_in:.4} to {slope_out:.4}");
        }
    }
}
