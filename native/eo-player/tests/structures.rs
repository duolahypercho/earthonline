//! The grade-separated roads (eo-structures, `--structures`) in the
//! collision world, on the real city:
//!
//! * without structures (or with an empty world) every tile, query and
//!   spawn is what it was;
//! * every structure tile cooks its deck tops (STREET) and concrete
//!   (WORLD), a tile with only a deck too, the same bytes when cooked
//!   twice; the cook time per tile with and without them is printed;
//! * the level queries pick the surface nearest below a reference height;
//!   the floor clamp never pushes a body up through the ground over it;
//! * a car driven from a street up an on-ramp, along the I-80 approach and
//!   down an off-ramp (`routes::freeway_route`) stays on the deck; a walker
//!   under a deck stays on the street; cars on the Bay Bridge's two decks
//!   and the Golden Gate's stay on their own deck.

use eo_player::routes::{self, DrivePilot, WalkPilot};
use eo_player::street_ground::StreetGround;
use eo_player::world::{self, BuildingSet, BuiltTile, CollisionWorld, GroundProvider, LEVEL_REACH_M, SurfaceMesh, TileKey, cook_tile};
use eo_player::{Mode, Player, StepInput};
use eo_structures::{Chain, ChainKind, LandmarkDeck, StructureOptions, StructureWorld};
use glam::DVec3;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

struct City {
    city: eo_data::City,
    buildings: Arc<BuildingSet>,
    /// The street surface alone (no structures).
    plain: Arc<StreetGround>,
    /// The same street world with the grade-separated roads over it.
    decks: Arc<StreetGround>,
}

fn city() -> &'static City {
    static C: OnceLock<City> = OnceLock::new();
    C.get_or_init(|| {
        let paths = eo_data::DataPaths::discover().unwrap();
        let city = eo_data::City::load(&paths).unwrap();
        let elevation = Arc::new(eo_data::ElevationGrid::load(&paths).unwrap());
        let lift: serde_json::Value = serde_json::from_slice(&std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../eo-assets/data/authored-placement.json")).unwrap()).unwrap();
        let buildings = Arc::new(BuildingSet::from_city(&city, lift["constructionLiftM"].as_f64().unwrap()));
        let plain = Arc::new(StreetGround::build(&city, elevation).unwrap());
        let ca = eo_data::CaStandards::load(&paths).unwrap();
        let t = Instant::now();
        let sw = StructureWorld::build(&plain.world, &city, &ca, eo_landmarks::LandmarksJs::get(), StructureOptions::default()).unwrap();
        println!("structures built in {:.2} s", t.elapsed().as_secs_f64());
        let decks = Arc::new(StreetGround::new(plain.world.clone(), Some(Arc::new(sw))));
        City { city, buildings, plain, decks }
    })
}

fn structures(c: &City) -> &StructureWorld {
    c.decks.decks().expect("built decks")
}

/// Every byte of a cooked tile that collision reads: each collider's shape
/// (trimesh vertices and indices, heightfield samples and scale), pose,
/// groups and friction, and the analytic heights and prisms.
fn tile_digest(t: &BuiltTile) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    t.key.hash(&mut h);
    t.triangles.hash(&mut h);
    for c in &t.colliders {
        let p = c.translation();
        [p.x, p.y, p.z].map(f32::to_bits).hash(&mut h);
        c.friction().to_bits().hash(&mut h);
        let g = c.collision_groups();
        (g.memberships.bits(), g.filter.bits()).hash(&mut h);
        if let Some(m) = c.shape().as_trimesh() {
            1u8.hash(&mut h);
            for v in m.vertices() {
                [v.x, v.y, v.z].map(f32::to_bits).hash(&mut h);
            }
            m.indices().hash(&mut h);
        } else if let Some(f) = c.shape().as_heightfield() {
            2u8.hash(&mut h);
            f.heights().data().iter().map(|v| v.to_bits()).collect::<Vec<u32>>().hash(&mut h);
            let s = f.scale();
            [s.x, s.y, s.z].map(f32::to_bits).hash(&mut h);
        } else {
            3u8.hash(&mut h);
        }
    }
    t.heights.h.iter().map(|v| v.to_bits()).collect::<Vec<u32>>().hash(&mut h);
    for p in t.prisms.iter() {
        [p.base_y, p.top_y].map(f64::to_bits).hash(&mut h);
        p.ring.iter().flat_map(|q| q.map(f64::to_bits)).collect::<Vec<u64>>().hash(&mut h);
    }
    h.finish()
}

fn street_triangles(s: &Option<SurfaceMesh>) -> (usize, usize) {
    s.as_ref().map(|s| (s.street.len(), s.barrier.len())).unwrap_or((0, 0))
}

fn percentile(v: &mut [f64], q: f64) -> f64 {
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * q).round() as usize]
}

#[test]
fn without_structures_every_tile_and_query_is_as_before() {
    let c = city();
    let sw = structures(c);
    let none = StreetGround::new(c.plain.world.clone(), None);
    let empty = StreetGround::new(c.plain.world.clone(), Some(Arc::new(StructureWorld::empty())));
    assert!(!c.plain.multi_level() && !none.multi_level() && !empty.multi_level() && c.decks.multi_level());
    // the deck tiles: the same triangles and the same cooked bytes as the street alone.
    let keys = sw.tile_keys();
    for &(x, z) in keys.iter().step_by(23) {
        let k = TileKey { x, z };
        let a = c.plain.surface_tile(k);
        for b in [none.surface_tile(k), empty.surface_tile(k)] {
            assert_eq!(a.is_some(), b.is_some());
            if let (Some(a), Some(b)) = (&a, &b) {
                assert!(a.street == b.street && a.barrier == b.barrier, "tile {k:?}");
            }
        }
        assert_eq!(tile_digest(&cook_tile(k, &*c.plain, &c.buildings)), tile_digest(&cook_tile(k, &empty, &c.buildings)), "tile {k:?}");
    }
    // the level queries are the one-level ones, whatever the reference height,
    // also where a deck stands over the street.
    let mut world = CollisionWorld::new(c.plain.clone(), c.buildings.clone(), 2);
    let deck = sw.chains().iter().find(|ch| ch.is_built() && ch.kind == ChainKind::Deck && ch.crossings.iter().any(|x| x.chain.is_none())).unwrap();
    let x = deck.crossings.iter().find(|x| x.chain.is_none()).unwrap();
    let f = deck.frame((x.s0 + x.s1) / 2.0);
    world.load_blocking(DVec3::new(f.x, 0.0, f.z), 150.0);
    assert!(!world.multi_level());
    for (dx, dz) in [(0.0, 0.0), (3.0, 1.0), (-7.0, 4.0), (20.0, -15.0)] {
        let (px, pz) = (f.x + dx, f.z + dz);
        let y = world.surface_y(px, pz);
        for from in [y - 5.0, y, y + 1.0, deck.y_at(x.s0) + 1.0, 1000.0] {
            assert_eq!(world.surface_below(px, pz, from).to_bits(), y.to_bits());
            assert_eq!(world.floor_under(px, pz, from).to_bits(), world.floor_y(px, pz).to_bits());
            let p = DVec3::new(px, from, pz);
            assert_eq!(routes::clearance(&world, p).to_bits(), (from - world.ground.ground_y(px, pz)).to_bits());
        }
    }
    // a car spawned there stands on the street, as before.
    let mut player = Player::new(DVec3::new(f.x + 3.0, world.surface_y(f.x + 3.0, f.z), f.z), 0.0);
    player.spawn_vehicle(&mut world, f.x, f.z, 0.0);
    let d = player.vehicles[0].datum(&world);
    assert!((d.y - world.surface_y(f.x, f.z)).abs() < 1e-5, "{d:?}");
    // and no freeway drive exists without structures.
    assert!(routes::freeway_route(&c.city, &c.plain.world, &StructureWorld::empty()).is_none());
}

#[test]
fn every_structure_tile_cooks_its_decks_the_same_twice() {
    let c = city();
    let sw = structures(c);
    let keys = sw.tile_keys();
    let (mut plain_ms, mut cold_ms, mut warm_ms, mut surface_ms) = (vec![], vec![], vec![], vec![]);
    let (mut deck_only, mut added_street, mut added_barrier, mut max_tile) = (0usize, 0usize, 0usize, (0usize, (0, 0)));
    for &(x, z) in &keys {
        let k = TileKey { x, z };
        let _ = cook_tile(k, &*c.plain, &c.buildings); // the street mesh cached
        let p = cook_tile(k, &*c.plain, &c.buildings);
        let t = Instant::now();
        let surf = sw.surface_tile((x, z));
        surface_ms.push(t.elapsed().as_secs_f64() * 1e3);
        let cold = cook_tile(k, &*c.decks, &c.buildings);
        let warm = cook_tile(k, &*c.decks, &c.buildings);
        plain_ms.push(p.cook_ms);
        cold_ms.push(cold.cook_ms);
        warm_ms.push(warm.cook_ms);
        assert_eq!(tile_digest(&cold), tile_digest(&warm), "tile {k:?} cooked twice");
        let (s, b) = surf.as_ref().map(|s| (s.street.len(), s.barrier.len())).unwrap_or((0, 0));
        let (ps, pb) = street_triangles(&c.plain.surface_tile(k));
        let (ds, db) = street_triangles(&c.decks.surface_tile(k));
        assert_eq!((ds, db), (ps + s, pb + b), "tile {k:?}: the deck triangles are added to the street's");
        assert_eq!(warm.triangles, p.triangles + s + b, "tile {k:?}");
        if ps + pb == 0 && s + b > 0 {
            // a deck alone (over the water, freeway only): it still collides.
            deck_only += 1;
            assert!(c.plain.surface_tile(k).is_none() && c.decks.surface_tile(k).is_some());
            assert!(warm.colliders.len() > p.colliders.len());
        }
        added_street += s;
        added_barrier += b;
        if s + b > max_tile.0 {
            max_tile = (s + b, (x, z));
        }
    }
    let n = keys.len();
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
    println!(
        "{n} structure tiles ({deck_only} with no street mesh): {added_street} deck-top and {added_barrier} concrete triangles added; largest tile {:?} +{} triangles",
        max_tile.1, max_tile.0
    );
    println!(
        "cook ms per tile, one thread: street only mean {:.2} p95 {:.2} max {:.2}; with structures (surface cached) mean {:.2} p95 {:.2} max {:.2}; first cook (builds the structure mesh) mean {:.2} p95 {:.2} max {:.2}; structure surface alone mean {:.3} max {:.3}",
        mean(&plain_ms),
        percentile(&mut plain_ms.clone(), 0.95),
        percentile(&mut plain_ms.clone(), 1.0),
        mean(&warm_ms),
        percentile(&mut warm_ms.clone(), 0.95),
        percentile(&mut warm_ms.clone(), 1.0),
        mean(&cold_ms),
        percentile(&mut cold_ms.clone(), 0.95),
        percentile(&mut cold_ms.clone(), 1.0),
        mean(&surface_ms),
        percentile(&mut surface_ms.clone(), 1.0),
    );
    assert!(deck_only > 0, "no deck-only tile (the Golden Gate and Bay Bridge decks are over the water)");
}

/// A crossing of a built deck over a paved street: (the deck, the street's
/// plan entry, the entry's station under the deck's centreline), the one
/// with the least clearance under the deck whose street runs on 25 m either
/// side within its own run.
fn low_crossing(c: &City) -> (&Chain, &eo_street::plan::Entry, f64) {
    let sw = structures(c);
    let plan = &c.plain.world.plan;
    let mut best: Option<(f64, &Chain, &eo_street::plan::Entry, f64)> = None;
    for ch in sw.chains().iter().filter(|ch| ch.is_built() && ch.kind == ChainKind::Deck) {
        for x in ch.crossings.iter().filter(|x| x.chain.is_none() && x.achieved.is_finite()) {
            let Some(id) = x.over.rsplit_once(" (").map(|(_, id)| id.trim_end_matches(')')) else { continue };
            let Some(e) = plan.entries.iter().find(|e| e.id == id) else { continue };
            let f = ch.frame((x.s0 + x.s1) / 2.0);
            let mut on = (f64::INFINITY, 0.0);
            for i in 0..e.points.len() - 1 {
                let (a, b) = (e.points[i], e.points[i + 1]);
                let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
                let l2 = dx * dx + dz * dz;
                let t = (((f.x - a[0]) * dx + (f.z - a[1]) * dz) / l2).clamp(0.0, 1.0);
                let d = (a[0] + dx * t - f.x).hypot(a[1] + dz * t - f.z);
                if d < on.0 {
                    on = (d, e.cum[i] + t * (e.cum[i + 1] - e.cum[i]));
                }
            }
            if on.0 > 1.0 || on.1 - 25.0 < e.s0() || on.1 + 25.0 > e.s1() {
                continue;
            }
            if best.as_ref().is_none_or(|b| x.achieved < b.0) {
                best = Some((x.achieved, ch, e, on.1));
            }
        }
    }
    let (_, ch, e, s) = best.expect("a deck over a street");
    (ch, e, s)
}

#[test]
fn the_level_queries_pick_the_surface_nearest_below() {
    let c = city();
    let (deck, e, s) = low_crossing(c);
    let f = e.frame(s, false);
    let mut world = CollisionWorld::new(c.decks.clone(), c.buildings.clone(), 4);
    world.load_blocking(DVec3::new(f.x, 0.0, f.z), 150.0);
    assert!(world.multi_level());
    let (ds, _) = deck.project(f.x, f.z);
    let top = deck.y_at(ds);
    let street = c.plain.world.paved_at(f.x, f.z).expect("the street is drawn under the deck");
    let floor = world.floor_y(f.x, f.z);
    // from over the deck: the deck; from the street's level: the street (as surface_y).
    let on = world.surface_below(f.x, f.z, top + 1.0);
    let under = world.surface_below(f.x, f.z, street + LEVEL_REACH_M);
    println!("{} over {} at ({:.1}, {:.1}): deck top {top:.3}, street {street:.3}, floor {floor:.3}; surface below: from over the deck {on:.3}, from the street {under:.3}", deck.name(), e.id, f.x, f.z);
    assert!((on - top).abs() < 0.3, "on the deck {on:.3} vs its top {top:.3}");
    assert!((under - street).abs() < 0.05 && (under - world.surface_y(f.x, f.z)).abs() < 1e-4, "{under} vs {}", world.surface_y(f.x, f.z));
    // the floor clamp under the deck and on it is the ground, as before.
    assert_eq!(world.floor_under(f.x, f.z, street + 0.9).to_bits(), floor.to_bits());
    assert_eq!(world.floor_under(f.x, f.z, top + 0.9).to_bits(), floor.to_bits());
    // clearance: of the feet over the surface under them.
    assert!((routes::clearance(&world, DVec3::new(f.x, top + 0.5, f.z)) - 0.5).abs() < 0.3);
    assert!((routes::clearance(&world, DVec3::new(f.x, street + 0.2, f.z)) - 0.2).abs() < 0.05);
}

/// A street surface 10 m under the ground (a tunnel's road, which no stage
/// builds yet): `multi_level` says whether the provider has several levels.
struct UnderGround {
    multi: bool,
}

impl GroundProvider for UnderGround {
    fn ground_y(&self, _x: f64, _z: f64) -> f64 {
        10.0
    }
    fn surface_tile(&self, key: TileKey) -> Option<SurfaceMesh> {
        let (x0, z0) = key.origin();
        let q = |a: [f64; 3], b: [f64; 3], cc: [f64; 3], d: [f64; 3]| [[a, b, cc], [a, cc, d]];
        let mut s = SurfaceMesh::default();
        let t = world::TILE_SIZE_M;
        s.street.extend(q([x0, 0.0, z0], [x0, 0.0, z0 + t], [x0 + t, 0.0, z0 + t], [x0 + t, 0.0, z0]));
        Some(s)
    }
    fn multi_level(&self) -> bool {
        self.multi
    }
}

#[test]
fn the_floor_clamp_never_pushes_a_body_up_through_the_ground_over_it() {
    for multi in [true, false] {
        let mut world = CollisionWorld::new(Arc::new(UnderGround { multi }), Arc::new(BuildingSet::empty()), 1);
        world.load_blocking(DVec3::ZERO, 150.0);
        let mut player = Player::new(DVec3::new(0.0, 0.0, 0.0), 0.0);
        for _ in 0..240 {
            player.fixed_step(&mut world, &StepInput { move_y: 1.0, ..Default::default() });
        }
        let f = player.character.feet();
        assert!(f.z > 5.0, "walked to {f:?}");
        if multi {
            // several levels: on its own street under the ground.
            assert!(f.y.abs() < 0.05, "feet {f:?}");
            assert!(world.surface_below(f.x, f.z, f.y + LEVEL_REACH_M).abs() < 1e-6);
        } else {
            // one level (today's world): the clamp pushes it up into the ground over it.
            assert!(f.y > 5.0, "feet {f:?}");
        }
    }
}

/// The built decks of the same levels as the drive's within 60 m of its
/// path (a drive along a ramp into a merge is over the mainline's deck too).
fn decks_near(sw: &StructureWorld, route: &routes::FreewayRoute) -> Vec<u32> {
    let levels: Vec<i8> = route.chains.iter().map(|&(id, _)| sw.chain(id).unwrap().level).collect();
    sw.chains()
        .iter()
        .filter(|ch| ch.is_built() && ch.kind == ChainKind::Deck && levels.contains(&ch.level))
        .filter(|ch| route.chains.iter().any(|&(id, _)| id == ch.id) || ch.points.iter().any(|p| route.path.iter().any(|q| (p[0] - q[0]).hypot(p[1] - q[1]) < 60.0)))
        .map(|ch| ch.id)
        .collect()
}

/// The barrier's base: the deck surface reaches this far past the roadway
/// under each barrier (and where a merge drops it).
fn barrier_base() -> f64 {
    eo_data::CaStandards::load(&eo_data::DataPaths::discover().unwrap()).unwrap().metric("concrete-barrier-type-60m-base-width").unwrap()
}

/// The deck under (x, z) of the chains `ids` whose top (profile,
/// centreline) is nearest `y`, when the point is on any of them (on its
/// deck surface: within its half width plus `edge`, past its ends):
/// (chain, its top, how far apart the tops of all the decks there are).
fn deck_under(sw: &StructureWorld, ids: &[u32], x: f64, z: f64, y: f64, edge: f64) -> Option<(u32, f64, f64)> {
    let mut best: Option<(f64, u32, f64)> = None;
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for &id in ids {
        let ch = sw.chain(id).unwrap();
        let (s, d) = ch.project(x, z);
        if d <= ch.half_at(s) + edge && s > 0.25 && s < ch.length() - 0.25 {
            let top = ch.y_at(s);
            lo = lo.min(top);
            hi = hi.max(top);
            if best.is_none_or(|b| (top - y).abs() < b.0) {
                best = Some(((top - y).abs(), id, top));
            }
        }
    }
    best.map(|b| (b.1, b.2, hi - lo))
}

#[test]
fn a_car_driven_up_a_ramp_and_along_a_deck_stays_on_it() {
    let c = city();
    let sw = structures(c);
    let route = routes::freeway_route(&c.city, &c.decks.world, sw).expect("a freeway drive");
    let total = routes::polyline_length(&route.path);
    let names: Vec<String> = route.chains.iter().map(|&(id, f)| format!("{id}{} {} ({:?}, {:.0} m)", if f { "+" } else { "-" }, sw.chain(id).unwrap().name(), sw.chain(id).unwrap().class, sw.chain(id).unwrap().length())).collect();
    println!(
        "freeway drive: from ({:.1}, {:.1}) to ({:.1}, {:.1}), {total:.0} m: {:.0} m of street, the deck from {:.0} m to {:.0} m over chains [{}], then {:.0} m of street",
        route.path[0][0],
        route.path[0][1],
        route.path.last().unwrap()[0],
        route.path.last().unwrap()[1],
        route.deck_from,
        route.deck_from,
        route.deck_to,
        names.join(", "),
        total - route.deck_to
    );
    let mut world = CollisionWorld::new(c.decks.clone(), c.buildings.clone(), 4);
    let p0 = route.path[0];
    let p1 = route.path[2];
    world.load_blocking(DVec3::new(p0[0], 0.0, p0[1]), world::LOAD_RADIUS_M);
    let feet = DVec3::new(p0[0] + 3.0, world.surface_y(p0[0] + 3.0, p0[1]), p0[1]);
    let mut player = Player::new(feet, 0.0);
    player.spawn_vehicle(&mut world, p0[0], p0[1], (p1[0] - p0[0]).atan2(p1[1] - p0[1]));
    for _ in 0..60 {
        player.fixed_step(&mut world, &StepInput::default());
    }
    player.fixed_step(&mut world, &StepInput { vehicle_toggle: true, ..Default::default() });
    assert!(matches!(player.mode, Mode::Driving(0)), "could not enter the car");
    let mut pilot = DrivePilot::new(route.path.clone(), 14.0);
    let near = decks_near(sw, &route);
    let edge = barrier_base();
    let (mut steps, mut on_deck, mut contact4, mut max_dev, mut max_up) = (0usize, 0usize, 0usize, 0f64, f64::MIN);
    let (mut street_dev, mut overlap, mut foreign): (f64, (f64, [f64; 2]), usize) = (0.0, (0.0, [0.0; 2]), 0);
    let mut visited: Vec<u32> = vec![];
    while !pilot.done() && steps < 60 * 60 * 6 {
        let input = pilot.input(&player, &world);
        player.fixed_step(&mut world, &input);
        let v = &player.vehicles[0];
        let d = v.datum(&world);
        world.pump(d);
        steps += 1;
        if v.wheels_in_contact() == 4 {
            contact4 += 1;
        }
        match deck_under(sw, &near, d.x, d.z, d.y, edge) {
            Some((id, top, apart)) => {
                on_deck += 1;
                if !route.chains.iter().any(|x| x.0 == id) {
                    // on another deck: the mainline a ramp merges into.
                    foreign += 1;
                } else if !visited.contains(&id) {
                    visited.push(id);
                }
                if apart > overlap.0 {
                    overlap = (apart, [d.x, d.z]);
                }
                let dev = d.y - top;
                max_dev = max_dev.max(dev.abs());
                max_up = max_up.max(d.y - world.floor_y(d.x, d.z));
                assert!(dev.abs() < 0.3, "step {steps}: the car's datum {:.3} m off the deck top of chain {id} at {d:?}", dev);
            }
            None => {
                // on the street before and after: never under the drawn
                // street (where it is drawn under the datum: a paved stub's
                // end need not be square to the deck's) nor the ground. (The
                // stubs' own humps by the paved ends can throw it up a little.)
                if let Some(s) = c.plain.world.paved_at(d.x, d.z) {
                    street_dev = street_dev.max((d.y - s).abs());
                    assert!(d.y - s > -0.25, "step {steps}: datum {:.3} m under the street at {d:?}", d.y - s);
                }
                assert!(d.y > world.floor_y(d.x, d.z) - 0.25, "step {steps}: datum under the ground at {d:?}");
            }
        }
    }
    println!(
        "drove {total:.0} m in {:.1} s: {on_deck} steps on the deck over chains {visited:?} (datum within {max_dev:.3} m of the deck top, up to {max_up:.1} m over the ground), off it within {street_dev:.3} m of the street; four wheels down {:.1} %; overlapping decks' tops up to {:.3} m apart (at {:.1}, {:.1}), {foreign} steps on another chain's deck there; tiles inserted {}, insert max {:.2} ms, pump max {:.2} ms, cook max {:.1} ms (on 4 worker threads)",
        steps as f64 / 60.0,
        contact4 as f64 * 100.0 / steps as f64,
        overlap.0,
        overlap.1[0],
        overlap.1[1],
        world.stats.tiles_inserted,
        world.stats.insert_ms_max,
        world.stats.pump_ms_max,
        world.stats.cook_ms_max
    );
    assert!(pilot.done(), "did not finish the drive in {steps} steps");
    assert_eq!(visited.len(), route.chains.len(), "every chain of the drive was driven on");
    assert!(max_up > 5.0, "never higher than {max_up:.1} m over the ground");
    assert!(contact4 as f64 / steps as f64 > 0.95);
    // and out of the car on the street at the end.
    for _ in 0..240 {
        player.fixed_step(&mut world, &StepInput { brake: 1.0, ..Default::default() });
    }
    player.fixed_step(&mut world, &StepInput { vehicle_toggle: true, ..Default::default() });
    assert_eq!(player.mode, Mode::OnFoot, "could not leave the car");
    let f = player.character.feet();
    assert!((f.y - world.surface_below(f.x, f.z, f.y + 0.5)).abs() < 0.3, "left the car at {f:?}");
}

#[test]
fn a_car_left_on_a_deck_is_left_on_the_deck() {
    // stopping on the deck and getting out: the body stands on the deck beside the car.
    let c = city();
    let sw = structures(c);
    let route = routes::freeway_route(&c.city, &c.decks.world, sw).expect("a freeway drive");
    // the middle of the mainline chain.
    let &(id, _) = route.chains.iter().find(|&&(id, _)| sw.chain(id).unwrap().class == eo_structures::RoadClass::Motorway).unwrap();
    let ch = sw.chain(id).unwrap();
    let s = ch.length() / 2.0;
    let f = ch.frame(s);
    let top = ch.y_at(s);
    let mut world = CollisionWorld::new(c.decks.clone(), c.buildings.clone(), 4);
    world.load_blocking(DVec3::new(f.x, top, f.z), 150.0);
    let mut player = Player::new(DVec3::new(f.x, top + 0.3, f.z), 0.0);
    for _ in 0..30 {
        player.fixed_step(&mut world, &StepInput::default());
    }
    let feet = player.character.feet();
    assert!((feet.y - top).abs() < 0.3, "a body put on the deck stands at {feet:?}, the deck top is {top:.3}");
    // a car beside it is spawned on the deck too, not on the street under it.
    let yaw = f.tx.atan2(f.tz);
    player.spawn_vehicle(&mut world, f.x + f.nx * 1.5, f.z + f.nz * 1.5, yaw);
    for _ in 0..60 {
        player.fixed_step(&mut world, &StepInput::default());
    }
    let d = player.vehicles[0].datum(&world);
    assert!((d.y - top).abs() < 0.3, "the car stands at {d:?}, the deck top is {top:.3}");
    player.fixed_step(&mut world, &StepInput { vehicle_toggle: true, ..Default::default() });
    assert!(matches!(player.mode, Mode::Driving(0)));
    player.fixed_step(&mut world, &StepInput { vehicle_toggle: true, ..Default::default() });
    assert_eq!(player.mode, Mode::OnFoot, "could not leave the car on the deck");
    let out = player.character.feet();
    assert!((out.y - top).abs() < 0.4, "left the car at {out:?}, the deck top is {top:.3}");
    assert!(routes::clearance(&world, out).abs() < 0.1, "clearance {:.3}", routes::clearance(&world, out));
}

#[test]
fn a_walker_under_a_deck_stays_on_the_street() {
    let c = city();
    let (deck, e, s) = low_crossing(c);
    let sw = &c.plain.world;
    let a = e.frame(s - 25.0, false);
    let mut world = CollisionWorld::new(c.decks.clone(), c.buildings.clone(), 4);
    world.load_blocking(DVec3::new(a.x, 0.0, a.z), 150.0);
    let start = DVec3::new(a.x, world.surface_below(a.x, a.z, sw.paved_at(a.x, a.z).unwrap() + LEVEL_REACH_M), a.z);
    let mut player = Player::new(start, 0.0);
    let waypoints: Vec<[f64; 2]> = (0..=10).map(|k| e.frame(s - 25.0 + 5.0 * k as f64, false)).map(|f| [f.x, f.z]).collect();
    let mut pilot = WalkPilot::new(waypoints, false);
    let (mut under, mut max_dev, mut lowest_gap) = (0usize, 0f64, f64::INFINITY);
    for step in 0..60 * 90 {
        let i = pilot.input(&mut player, &world);
        if pilot.done() {
            break;
        }
        player.fixed_step(&mut world, &i);
        let f = player.character.feet();
        world.pump(f);
        let street = sw.paved_at(f.x, f.z).unwrap_or(world.floor_y(f.x, f.z));
        max_dev = max_dev.max((f.y - street).abs());
        assert!((f.y - street).abs() < 0.3, "step {step}: feet {f:?}, the street is at {street:.3}");
        let (ds, dd) = deck.project(f.x, f.z);
        if dd <= deck.half_at(ds) {
            under += 1;
            let soffit = deck.y_at(ds) - deck.depth_at(ds);
            lowest_gap = lowest_gap.min(soffit - f.y);
            assert!(f.y < soffit - 1.0, "step {step}: feet {f:?} up at the deck (soffit {soffit:.3})");
        }
    }
    println!("walked {} under {} (chain {}): {under} steps under the deck, feet within {max_dev:.3} m of the street, the soffit {lowest_gap:.2} m over them at the least", e.id, deck.name(), deck.id);
    assert!(pilot.done(), "did not walk the street under the deck");
    assert!(under > 30, "only {under} steps under the deck");
}

#[test]
fn cars_on_the_bay_bridge_decks_and_the_golden_gate_stay_on_their_own_deck() {
    let c = city();
    let sw = structures(c);
    let landmark = |d: LandmarkDeck| sw.chains().iter().find(|ch| ch.landmark == Some(d) && ch.is_built()).unwrap_or_else(|| panic!("{d:?}"));
    let upper = landmark(LandmarkDeck::BayBridgeUpper);
    let lower = landmark(LandmarkDeck::BayBridgeLower);
    for ch in [upper, lower, landmark(LandmarkDeck::GoldenGate)] {
        // over the water, 40 % along, driven 250 m along the centreline.
        let s0 = ch.length() * 0.4;
        let path: Vec<[f64; 2]> = (0..=60).map(|k| ch.frame(s0 + 5.0 * k as f64)).map(|f| [f.x, f.z]).collect();
        let f = ch.frame(s0);
        let top = ch.y_at(s0);
        let mut world = CollisionWorld::new(c.decks.clone(), c.buildings.clone(), 4);
        world.load_blocking(DVec3::new(f.x, top, f.z), world::LOAD_RADIUS_M);
        let mut player = Player::new(DVec3::new(f.x, top, f.z), 0.0);
        player.spawn_vehicle_below(&mut world, DVec3::new(f.x, top + 1.0, f.z), f.tx.atan2(f.tz));
        let d = player.vehicles[0].datum(&world);
        assert!((d.y - top).abs() < 0.3, "{:?}: spawned at {d:?}, the deck top is {top:.3}", ch.landmark);
        player.character.teleport_feet(d + DVec3::new(f.nx * 2.0, 0.0, f.nz * 2.0));
        for _ in 0..60 {
            player.fixed_step(&mut world, &StepInput::default());
        }
        player.fixed_step(&mut world, &StepInput { vehicle_toggle: true, ..Default::default() });
        assert!(matches!(player.mode, Mode::Driving(0)), "{:?}: could not enter", ch.landmark);
        let mut pilot = DrivePilot::new(path, 14.0);
        let (mut steps, mut max_dev) = (0usize, 0f64);
        while !pilot.done() && steps < 60 * 60 {
            let input = pilot.input(&player, &world);
            player.fixed_step(&mut world, &input);
            let d = player.vehicles[0].datum(&world);
            world.pump(d);
            steps += 1;
            let (s, _) = ch.project(d.x, d.z);
            let dev = d.y - ch.y_at(s);
            max_dev = max_dev.max(dev.abs());
            assert!(dev.abs() < 0.3, "{:?} step {steps}: datum {dev:.3} m off its deck at {d:?}", ch.landmark);
            if ch.landmark == Some(LandmarkDeck::BayBridgeLower) {
                // the upper deck stands over it all along.
                let (su, du) = upper.project(d.x, d.z);
                if du < upper.half_at(su) {
                    assert!(upper.y_at(su) - d.y > 5.0, "step {steps}: the lower deck's car at {d:?} is up at the upper deck");
                }
            }
        }
        println!("{:?} (chain {}, {:.1} m up): drove 250 m in {:.1} s, datum within {max_dev:.3} m of the deck", ch.landmark, ch.id, top, steps as f64 / 60.0);
        assert!(pilot.done(), "{:?}: did not finish", ch.landmark);
    }
    // the two Bay Bridge decks are both there over the same water.
    let s = upper.length() * 0.4;
    let f = upper.frame(s);
    assert!(lower.project(f.x, f.z).1 < lower.half_at(lower.project(f.x, f.z).0), "the lower deck runs under the upper");
}

#[test]
fn the_same_inputs_give_the_same_car_on_a_deck() {
    let c = city();
    let sw = structures(c);
    let route = routes::freeway_route(&c.city, &c.decks.world, sw).expect("a freeway drive");
    let run = || {
        let mut world = CollisionWorld::new(c.decks.clone(), c.buildings.clone(), 4);
        let p0 = route.path[0];
        world.load_blocking(DVec3::new(p0[0], 0.0, p0[1]), 300.0);
        let mut player = Player::new(DVec3::new(p0[0] + 3.0, world.surface_y(p0[0] + 3.0, p0[1]), p0[1]), 0.0);
        player.spawn_vehicle(&mut world, p0[0], p0[1], 0.0);
        player.fixed_step(&mut world, &StepInput { vehicle_toggle: true, ..Default::default() });
        let mut pilot = DrivePilot::new(route.path.clone(), 14.0);
        for _ in 0..1500 {
            let i = pilot.input(&player, &world);
            player.fixed_step(&mut world, &i);
        }
        player.vehicles[0].state(&world)
    };
    let (a, b) = (run(), run());
    assert_eq!(a, b);
}

/// Every collider of a world by its handle: (index, generation, the
/// collider's translation), sorted.
fn handles(w: &CollisionWorld) -> Vec<(u32, u32, [u32; 3])> {
    let mut v: Vec<(u32, u32, [u32; 3])> = w
        .physics
        .colliders
        .iter()
        .map(|(h, c)| {
            let (i, g) = h.into_raw_parts();
            let t = c.translation();
            (i, g, [t.x, t.y, t.z].map(f32::to_bits))
        })
        .collect();
    v.sort();
    v
}

#[test]
fn with_structures_two_worlds_stream_the_same_tiles_into_the_same_handles() {
    // (stage A5: the physics solves a body's contacts in the order of the
    // colliders' handles, so worlds that cook their tiles at different
    // speeds must still insert and drop them alike.)
    let c = city();
    let sw = structures(c);
    let route = routes::freeway_route(&c.city, &c.decks.world, sw).expect("a freeway route");
    let mut worlds: Vec<CollisionWorld> = [1usize, 4].iter().map(|&n| CollisionWorld::new(c.decks.clone(), c.buildings.clone(), n)).collect();
    assert!(worlds.iter().all(|w| w.multi_level()));
    let p0 = route.path[0];
    for w in worlds.iter_mut() {
        w.load_blocking(DVec3::new(p0[0], 0.0, p0[1]), world::LOAD_RADIUS_M);
    }
    assert_eq!(handles(&worlds[0]), handles(&worlds[1]), "loaded alike");
    // a centre along the route and back at 20 m/s (60 pumps a second).
    let mut path: Vec<[f64; 2]> = route.path.clone();
    path.extend(route.path.iter().rev().skip(1));
    let cum: Vec<f64> = std::iter::once(0.0).chain(path.windows(2).scan(0.0, |s, w| { *s += (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]); Some(*s) })).collect();
    let total = *cum.last().unwrap();
    let (mut d, mut pumps, mut checks) = (0.0, 0usize, 0usize);
    while d < total {
        let i = cum.partition_point(|&x| x <= d).clamp(1, cum.len() - 1);
        let t = (d - cum[i - 1]) / (cum[i] - cum[i - 1]).max(1e-9);
        let p = [path[i - 1][0] + (path[i][0] - path[i - 1][0]) * t, path[i - 1][1] + (path[i][1] - path[i - 1][1]) * t];
        for (k, w) in worlds.iter_mut().enumerate() {
            w.pump(DVec3::new(p[0], 0.0, p[1]));
            if k == 1 {
                // the faster cooker's world also steps its physics between pumps.
                w.physics.step();
            }
        }
        pumps += 1;
        if pumps % 120 == 0 {
            assert_eq!(handles(&worlds[0]), handles(&worlds[1]), "pump {pumps} at ({:.0}, {:.0})", p[0], p[1]);
            checks += 1;
        }
        d += 20.0 / 60.0;
    }
    let (a, b) = (&worlds[0].stats, &worlds[1].stats);
    println!("{pumps} pumps along {total:.0} m: {} tiles inserted and {} dropped in each world, the same handles at {checks} checks", a.tiles_inserted, a.tiles_removed);
    assert_eq!((a.tiles_inserted, a.tiles_removed), (b.tiles_inserted, b.tiles_removed));
    assert!(a.tiles_removed > 5 && checks > 20, "{} dropped, {checks} checks", a.tiles_removed);
    assert_eq!(handles(&worlds[0]), handles(&worlds[1]));
}
