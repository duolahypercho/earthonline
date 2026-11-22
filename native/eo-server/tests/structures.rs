//! With the grade-separated roads (`--structures`), server and client agree:
//! the server builds its world (`GameWorld::load_with`) and shares cooked
//! tiles through its feed (`tiles::SharedTiles`); a client builds the same
//! world on its own (`StreetGround::build_joined`, as eo-app's `--structures`
//! does: the street built again with the structures' joins) and cooks its own
//! tiles. Every deck tile (deck tops and concrete included, a tile with only a
//! deck too) and every tile holding a structure end the joins changed (a
//! junction leg, a capped, raised or lowered stub) is byte-identical cooked
//! twice and through the feed, and a car driven along the whole freeway route
//! (up an on-ramp, through its merge, along the mainline and off it) by the
//! same inputs is the same body on both, step for step. A client that built
//! the street without the joins would cook some of those tiles otherwise
//! (checked), and with `--tunnels` the tiles the bores' terrain cuts hollow
//! out agree too.

use eo_net::game::{GameWorld, spawn_player, step_player};
use eo_net::wire::NetInput;
use eo_player::routes::{self, DrivePilot};
use eo_player::street_ground::StreetGround;
use eo_player::world::{BuiltTile, CollisionWorld, TileFeed, TileKey, cook_tile};
use eo_player::{Mode, StepInput};
use eo_server::tiles::SharedTiles;
use eo_structures::StructureOptions;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// The drive compared step for step: the whole freeway route (up the
/// on-ramp, through its merge with the mainline, along the mainline and
/// down the off-ramp to the street), within this many steps.
const MAX_STEPS: usize = 60 * 60 * 6;

/// Every byte of a cooked tile collision reads (shapes, poses, groups,
/// friction, the analytic heights and prisms).
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

#[test]
fn server_and_client_cook_the_same_deck_tiles_and_drive_the_same_car_on_them() {
    let paths = eo_data::DataPaths::discover().unwrap();
    let opts = Some(StructureOptions::default());
    let server = Arc::new(GameWorld::load_with(&paths, true, opts).unwrap());
    assert_eq!(server.structures, opts);
    assert!(server.ground.multi_level(), "the server's ground has its decks");
    // the client's own build of the same world: the joined street (stage A6), as eo-app's.
    let city = Arc::new(eo_data::City::load(&paths).unwrap());
    let elevation = Arc::new(eo_data::ElevationGrid::load(&paths).unwrap());
    let sg = Arc::new(StreetGround::build_joined(&city, elevation.clone(), opts).unwrap());
    let decks = sg.decks().expect("built decks");
    let joins = sg.world.o.structures.clone().expect("the street is built with the structures' joins");
    assert!(!sg.world.o.structures_tag().is_empty() && joins.ends.len() > 100, "{} structure ends joined", joins.ends.len());
    let client = GameWorld::from_parts(city.clone(), elevation.clone(), sg.clone(), server.buildings.clone(), true, opts);
    let route = routes::freeway_route(&city, &sg.world, decks).expect("a freeway drive");

    // the tiles: the drive's, every tenth structure tile, those with a deck
    // but no street, and those holding a raised or lowered structure end (and every eighth other end).
    let mut keys: Vec<TileKey> = route.path.iter().map(|p| TileKey::of(p[0], p[1])).collect();
    let all = decks.tile_keys();
    keys.extend(all.iter().step_by(10).map(|&(x, z)| TileKey { x, z }));
    let deck_only: Vec<TileKey> = all.iter().map(|&(x, z)| TileKey { x, z }).filter(|k| sg.world.cached_mesh((k.x, k.z)).indices.is_empty()).collect();
    keys.extend(deck_only.iter().take(12));
    let end_tiles: Vec<TileKey> = joins.ends.iter().enumerate().filter(|(i, e)| e.rise.is_some() || e.cut.is_some() || i % 8 == 0).map(|(_, e)| TileKey::of(e.at[0], e.at[1])).collect();
    keys.extend(&end_tiles);
    keys.sort();
    keys.dedup();
    // a client that built the street without the joins (as before stage A8) cooks some of those tiles otherwise.
    let unjoined = StreetGround::build_with(&city, elevation, opts).unwrap();
    let differ = end_tiles.iter().filter(|k| tile_digest(&cook_tile(**k, &unjoined, &client.buildings)) != tile_digest(&cook_tile(**k, &*client.ground, &client.buildings))).count();
    println!("{differ} of {} structure-end tiles cook otherwise on a street built without the joins", end_tiles.len());
    assert!(differ > 0, "the joins change no structure end's tile");
    drop(unjoined);
    let feed = SharedTiles::start(server.ground.clone(), server.buildings.clone(), 3);
    let (mut deck_tris, mut checked) = (0usize, 0usize);
    for &k in &keys {
        let a = cook_tile(k, &*client.ground, &client.buildings);
        let b = cook_tile(k, &*client.ground, &client.buildings);
        assert_eq!(tile_digest(&a), tile_digest(&b), "tile {k:?} cooked twice");
        let served = loop {
            if let Some(t) = feed.fetch(k, (0.0, 0.0)) {
                break t;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        assert_eq!(tile_digest(&a), tile_digest(&served), "tile {k:?}: the server's differs from the client's");
        if let Some(s) = decks.surface_tile((k.x, k.z)) {
            deck_tris += s.street.len() + s.barrier.len();
            assert!(a.triangles > s.street.len() + s.barrier.len());
        }
        checked += 1;
    }
    println!("{checked} tiles ({} with only a deck) byte-identical cooked twice and through the server's feed, {deck_tris} structure triangles among them", deck_only.len().min(12));
    assert!(deck_tris > 0 && !deck_only.is_empty());

    // a car along the whole freeway route: the server fed by the shared
    // tiles, the client cooking its own, the same inputs; the tiles reach
    // each world in its own time, and each inserts them in the same order
    // (`CollisionWorld`'s canonical insertion), so every contact is solved
    // alike.
    let p0 = route.path[0];
    let yaw = (route.path[2][0] - p0[0]).atan2(route.path[2][1] - p0[1]);
    let mut server_cw = CollisionWorld::fed(server.ground.clone(), server.buildings.clone(), feed.clone());
    let (mut on_server, _) = spawn_player(&mut server_cw, p0[0], p0[1], yaw, true);
    let mut client_cw = client.collision_world(2);
    let (mut on_client, _) = spawn_player(&mut client_cw, p0[0], p0[1], yaw, true);
    assert!(matches!(on_client.mode, Mode::Driving(0)));
    assert_eq!(on_server.state(&server_cw), on_client.state(&client_cw), "spawned alike");
    let mut pilot = DrivePilot::new(route.path.clone(), 14.0);
    let mut highest = f64::MIN;
    let mut k = 0;
    while !pilot.done() && k < MAX_STEPS {
        let s: StepInput = pilot.input(&on_client, &client_cw);
        let i = NetInput::quantize(&s, on_client.yaw);
        step_player(&mut on_server, &mut server_cw, &i);
        step_player(&mut on_client, &mut client_cw, &i);
        server_cw.pump(on_server.position(&server_cw));
        client_cw.pump(on_client.position(&client_cw));
        assert_eq!(on_server.state(&server_cw), on_client.state(&client_cw), "step {k}: the server and the client differ");
        let p = on_client.position(&client_cw);
        highest = highest.max(p.y - client_cw.floor_y(p.x, p.z));
        k += 1;
    }
    let end = on_client.position(&client_cw);
    let last = route.path[route.path.len() - 1];
    println!("{k} steps driven alike on server and client along the freeway route, up to {highest:.1} m over the ground; ended {:.1} m from the route's end", (end.x - last[0]).hypot(end.z - last[1]));
    assert!(pilot.done(), "the drive did not reach the route's end in {MAX_STEPS} steps");
    assert!(highest > 5.0, "the drive never got onto the deck ({highest:.1} m)");

    // and a car steered into the mainline deck's barrier and scraped along
    // it (its body against concrete of several colliders, across tile
    // seams): still the same body on both, step for step.
    let mut server_cw = CollisionWorld::fed(server.ground.clone(), server.buildings.clone(), feed.clone());
    let (mut on_server, _) = spawn_player(&mut server_cw, p0[0], p0[1], yaw, true);
    let mut client_cw = client.collision_world(2);
    let (mut on_client, _) = spawn_player(&mut client_cw, p0[0], p0[1], yaw, true);
    let mut pilot = DrivePilot::new(route.path.clone(), 14.0);
    let main = route.chains.iter().map(|c| decks.chain(c.0).unwrap()).find(|c| c.class == eo_structures::RoadClass::Motorway).expect("the mainline");
    let (mut k, mut hit, mut against) = (0usize, None::<usize>, 0usize);
    let mut last = on_client.position(&client_cw);
    while k < 4000 {
        let mut s: StepInput = pilot.input(&on_client, &client_cw);
        let p = on_client.position(&client_cw);
        let (sm, dist) = main.project(p.x, p.z);
        let on_main = sm > 60.0 && sm < main.length() - 60.0 && dist < main.half_at(sm) + 1.0 && p.y > main.y_at(sm) - 1.0;
        if on_main && hit.is_none() {
            hit = Some(k);
        }

        // from there: hard over to one side, then the other, full throttle, for 12 s.
        if let Some(h) = hit.filter(|&h| k < h + 720) {
            let side = if (k - h) / 240 % 2 == 0 { -1.0 } else { 1.0 };
            s = StepInput { throttle: 1.0, steer: side, ..s };
        }
        // (held by concrete: full throttle, barely moving.)
        if hit.is_some() && s.throttle > 0.9 && (p - last).length() < 0.02 {
            against += 1;
        }
        last = p;
        let i = NetInput::quantize(&s, on_client.yaw);
        step_player(&mut on_server, &mut server_cw, &i);
        step_player(&mut on_client, &mut client_cw, &i);
        server_cw.pump(on_server.position(&server_cw));
        client_cw.pump(on_client.position(&client_cw));
        assert_eq!(on_server.state(&server_cw), on_client.state(&client_cw), "crash step {k}: the server and the client differ");
        k += 1;
        if hit.is_some_and(|h| k > h + 1500) {
            break;
        }
    }
    println!("{k} steps alike with the car steered into the mainline's barriers ({against} steps held against concrete at full throttle)");
    assert!(hit.is_some() && against > 30, "the car never struck the mainline's barrier ({against} steps)");
}

/// With `--tunnels` too (stage A8): a dedicated server and a client that
/// each build the joined street and the bores agree on every tile a bore's
/// terrain cut hollows out or a bore runs under (the cut cells left out of
/// both heightfields, the floors the same), byte for byte through the feed.
#[test]
fn with_tunnels_server_and_client_cook_the_same_cut_and_bore_tiles() {
    let paths = eo_data::DataPaths::discover().unwrap();
    let opts = Some(StructureOptions { tunnels: true });
    let server = Arc::new(GameWorld::load_with(&paths, true, opts).unwrap());
    assert_eq!(server.structures, opts);
    let city = Arc::new(eo_data::City::load(&paths).unwrap());
    let elevation = Arc::new(eo_data::ElevationGrid::load(&paths).unwrap());
    let sg = Arc::new(StreetGround::build_joined(&city, elevation.clone(), opts).unwrap());
    let decks = sg.decks().expect("built decks");
    assert!(!decks.terrain_cuts().is_empty() && !decks.bores().is_empty(), "{} cuts, {} bores", decks.terrain_cuts().len(), decks.bores().len());
    let client = GameWorld::from_parts(city, elevation, sg.clone(), server.buildings.clone(), true, opts);
    let mut keys: Vec<TileKey> = decks.terrain_cuts().iter().flat_map(|c| c.ring.iter().map(|q| TileKey::of(q[0], q[1]))).collect();
    keys.extend(decks.bores().iter().flat_map(|b| b.points.iter().step_by(6).map(|p| TileKey::of(p.x, p.z))));
    keys.sort();
    keys.dedup();
    let feed = SharedTiles::start(server.ground.clone(), server.buildings.clone(), 3);
    for &k in &keys {
        let a = cook_tile(k, &*client.ground, &client.buildings);
        let served = loop {
            if let Some(t) = feed.fetch(k, (0.0, 0.0)) {
                break t;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        assert_eq!(tile_digest(&a), tile_digest(&served), "tile {k:?}: the server's differs from the client's");
    }
    // every cut's floor is the same on both, where the cut is.
    for c in decks.terrain_cuts() {
        let (x, z) = c.ring.iter().fold((0.0, 0.0), |a, q| (a.0 + q[0] / c.ring.len() as f64, a.1 + q[1] / c.ring.len() as f64));
        assert_eq!(server.ground.terrain_cut(x, z).map(f64::to_bits), client.ground.terrain_cut(x, z).map(f64::to_bits), "cut of chain {} at ({x:.0}, {z:.0})", c.chain);
    }
    println!("{} cut and bore tiles byte-identical on server and client ({} cuts, {} bores)", keys.len(), decks.terrain_cuts().len(), decks.bores().len());
}

/// One server tick of the regions, the network builder given real time (as `tests/handoff.rs`).
fn tick(regions: &mut eo_server::regions::Regions, v: eo_server::regions::Viewer) {
    regions.update(&[v]);
    let t = std::time::Instant::now();
    loop {
        if let Some(p) = regions.take_build() {
            regions.apply_build(p);
            break;
        }
        if regions.pending_tiles() == 0 || t.elapsed() > std::time::Duration::from_millis(30) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_micros(200));
    }
    regions.step();
}

/// What one run of a server's regions about a player shows of the deck levels.
#[derive(Default)]
struct DeckTraffic {
    lanes: usize,
    cars: std::collections::BTreeSet<u32>,
    over_deck: usize,
    flagged: usize,
    at_grade: usize,
}

/// `steps` server ticks of fresh regions over `source` about `v`, sampled every half second.
fn deck_traffic(source: &std::sync::Arc<eo_sim::native::NativeSource>, decks: &eo_structures::StructureWorld, v: eo_server::regions::Viewer, steps: usize) -> DeckTraffic {
    let mut regions = eo_server::regions::Regions::new(source.clone(), 7, 2);
    let mut t = DeckTraffic::default();
    for step in 0..steps {
        tick(&mut regions, v);
        if step % 30 != 29 {
            continue;
        }
        for r in &regions.world.regions {
            for i in 0..r.fleet.len() {
                let level = r.fleet.level[i];
                if level == 0 {
                    t.at_grade += 1;
                    continue;
                }
                t.cars.insert(r.fleet.cars[i].id);
                // the deck of that level is under the car (past a deck's end its lane falls back to the street).
                if decks.deck_at(r.fleet.x[i], r.fleet.z[i], level).is_some() {
                    t.over_deck += 1;
                }
            }
            t.flagged += r.out.vehicles.iter().filter(|a| eo_sim::output::level_of_flags(a.flags) != 0).count();
        }
    }
    t.lanes = regions.world.net.lanes.iter().filter(|l| l.alive && l.level != 0).count();
    t
}

/// A dedicated server with `--structures` (`eo_server::load_with`) keeps the
/// decks it built (`GameWorld::decks`) and hands their level table to its
/// simulation source, so its regions run the cars on a deck's lanes on that
/// deck's level (and say so in the flags its snapshots carry), as eo-app's
/// own simulation does; without structures the table is empty, and the same
/// regions over a source without it keep every car at grade.
#[test]
fn a_server_with_structures_runs_deck_traffic_on_the_decks_level() {
    use eo_server::regions::Viewer;
    let opts = Some(StructureOptions::default());
    // without the street surface there are no structures, and no levels.
    let (bare, bare_source) = eo_server::load_with(false, opts).unwrap();
    assert!(bare.structures.is_none() && bare.decks.is_none() && bare_source.levels().is_empty());
    let (game, source) = eo_server::load_with(true, opts).unwrap();
    let decks = game.decks.clone().expect("the server keeps the decks it built");
    assert!(!decks.is_empty());
    assert_eq!(source.levels(), decks.way_levels(), "the source's level table is the decks'");
    assert!(source.levels().len() > 100, "{} ways on structure levels", source.levels().len());
    // a player on the longest built city-street bridge: the server's recycler
    // places cars on the lanes of city street classes about a player (never on
    // a freeway's, which cars reach by driving), so its own deck lanes get cars.
    let city_class = |h: &str| matches!(h, "primary" | "secondary" | "tertiary" | "residential" | "unclassified" | "service");
    let main = decks
        .chains()
        .iter()
        .filter(|c| c.is_built() && c.kind == eo_structures::ChainKind::Deck && c.landmark.is_none() && c.pieces.iter().all(|p| city_class(&p.highway)))
        .max_by(|a, b| a.length().total_cmp(&b.length()))
        .expect("a city street on a deck");
    let f = main.frame(main.length() / 2.0);
    let v = Viewer { player: 1, x: f.x, z: f.z, yaw: f.tx.atan2(f.tz) };
    let t = deck_traffic(&source, &decks, v, 2400);
    println!(
        "server regions about chain {} ({:.0}, {:.0}): {} lanes on structure levels, {} cars on a deck level, {} samples over their level's deck, {} snapshot records flagged with a level, {} samples at grade",
        main.id,
        f.x,
        f.z,
        t.lanes,
        t.cars.len(),
        t.over_deck,
        t.flagged,
        t.at_grade
    );
    assert!(t.lanes >= 4, "lanes on a structure level: {}", t.lanes);
    assert!(!t.cars.is_empty() && t.over_deck >= 10 && t.flagged >= 10, "{} cars on decks, {} over their deck, {} flagged", t.cars.len(), t.over_deck, t.flagged);
    assert!(t.at_grade > 0, "traffic at grade too");
    // the same server without the table (the source `load` gives): every car at grade.
    let plain = std::sync::Arc::new(eo_sim::native::load(&eo_data::DataPaths::discover().unwrap()).unwrap());
    assert!(plain.levels().is_empty());
    let p = deck_traffic(&plain, &decks, v, 1200);
    assert!(p.lanes == 0 && p.cars.is_empty() && p.flagged == 0 && p.at_grade > 0, "without levels: {} lanes, {} cars, {} flagged", p.lanes, p.cars.len(), p.flagged);
}
