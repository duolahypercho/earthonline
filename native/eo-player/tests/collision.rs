//! Collision behaviour of the ported controller and car, in Rapier:
//!
//! * a synthetic street (flat ground, a kerb whose height is the CA standard
//!   `curb-height-sf-standard`, a building): the walker climbs the kerb and is
//!   stopped by the wall;
//! * the real city (sf-city.json, the footprints, the elevation grid): a walk
//!   of more than 20 blocks along the steepest-street route, and the steepest
//!   street itself, never below the ground and never inside a building; runs
//!   straight at real building walls; a car driven over the hills stays on its
//!   wheels; entering and leaving the car.

use eo_player::routes::{self, DrivePilot, WalkPilot};
use eo_player::world::{self, BuildingSet, CollisionWorld, ElevationGround, Footprint, GroundProvider, SurfaceMesh, TileKey};
use eo_player::{Mode, Player, StepInput};
use glam::DVec3;
use std::sync::{Arc, OnceLock};

// ---------------------------------------------------------------------------
// Synthetic street

struct KerbStreet {
    curb_m: f64,
}

/// x < 10: carriageway (y 0); 10 <= x <= 20: sidewalk at kerb height; a
/// building from x = 25 to 35.
impl GroundProvider for KerbStreet {
    fn ground_y(&self, _x: f64, _z: f64) -> f64 {
        0.0
    }
    fn surface_tile(&self, key: TileKey) -> Option<SurfaceMesh> {
        if key != (TileKey { x: 0, z: -1 }) && key != (TileKey { x: 0, z: 0 }) {
            return None;
        }
        let (z0, z1) = if key.z < 0 { (-60.0, 0.0) } else { (0.0, 60.0) };
        let h = self.curb_m;
        let q = |a: [f64; 3], b: [f64; 3], c: [f64; 3], d: [f64; 3]| [[a, b, c], [a, c, d]];
        let mut s = SurfaceMesh::default();
        s.street.extend(q([10.0, h, z0], [10.0, h, z1], [20.0, h, z1], [20.0, h, z0]));
        s.barrier.extend(q([10.0, 0.0, z0], [10.0, 0.0, z1], [10.0, h, z1], [10.0, h, z0]));
        s.barrier.extend(q([20.0, 0.0, z0], [20.0, h, z0], [20.0, h, z1], [20.0, 0.0, z1]));
        Some(s)
    }
}

fn curb_height() -> f64 {
    let paths = eo_data::DataPaths::discover().unwrap();
    eo_data::CaStandards::load(&paths).unwrap().metric("curb-height-sf-standard").unwrap()
}

#[test]
fn climbs_a_kerb_and_is_stopped_by_a_wall() {
    let curb = curb_height();
    assert!((0.1..0.2).contains(&curb), "CA curb height {curb}");
    let building = Footprint { ring: vec![[25.0, -20.0], [35.0, -20.0], [35.0, 20.0], [25.0, 20.0]], height: 10.0 };
    let mut world = CollisionWorld::new(Arc::new(KerbStreet { curb_m: curb }), Arc::new(BuildingSet::new(vec![building], 0.0)), 2);
    world.load_blocking(DVec3::new(10.0, 0.0, 0.0), 150.0);
    let mut player = Player::new(DVec3::new(4.0, 0.0, 0.0), std::f64::consts::FRAC_PI_2);
    let input = StepInput { move_y: 1.0, ..Default::default() };
    let mut on_kerb_y = vec![];
    let mut max_x: f64 = 0.0;
    for _ in 0..(60 * 20) {
        player.fixed_step(&mut world, &input);
        let feet = player.character.feet();
        max_x = max_x.max(feet.x);
        assert!(feet.y > -0.03, "fell below the carriageway: {feet:?}");
        if feet.x > 11.0 && feet.x < 19.0 {
            on_kerb_y.push(feet.y);
        }
        // Never inside the building, never closer than the capsule radius.
        assert!(feet.x <= 25.0 - 0.33, "inside the wall's reach: {feet:?}");
    }
    assert!(!on_kerb_y.is_empty(), "never reached the sidewalk (stopped at x {max_x:.2})");
    for y in &on_kerb_y {
        assert!((y - curb).abs() < 0.03, "on the sidewalk the feet stand at {y:.3}, the kerb top is {curb:.3}");
    }
    // Pressed against the wall at the end.
    assert!(max_x > 25.0 - 0.45, "stopped short of the wall at {max_x:.3}");
    // A jump against the wall gets no further.
    let jump = StepInput { move_y: 1.0, jump: true, ..Default::default() };
    for i in 0..120 {
        player.fixed_step(&mut world, if i % 40 == 0 { &jump } else { &input });
        assert!(player.character.feet().x <= 25.0 - 0.33);
    }
}

#[test]
fn a_car_settles_at_its_datum_accelerates_and_brakes_on_the_flat() {
    let mut world = CollisionWorld::new(Arc::new(KerbStreet { curb_m: 0.15 }), Arc::new(BuildingSet::empty()), 1);
    world.load_blocking(DVec3::new(-60.0, 0.0, 0.0), 150.0);
    let mut player = Player::new(DVec3::new(-60.0, 0.0, 3.0), 0.0);
    player.spawn_vehicle(&mut world, -60.0, 0.0, 0.0);
    for _ in 0..120 {
        player.fixed_step(&mut world, &StepInput::default());
    }
    let v = &player.vehicles[0];
    let settled = v.datum(&world).y;
    // The free length carries the static deflection: the body settles at its road datum.
    assert!(settled.abs() < 0.02, "settled datum {settled:.4}");
    assert!(world.physics.quarantine().bodies().is_empty());
    player.fixed_step(&mut world, &StepInput { vehicle_toggle: true, ..Default::default() });
    assert_eq!(player.mode, Mode::Driving(0));
    for _ in 0..120 {
        player.fixed_step(&mut world, &StepInput { throttle: 1.0, ..Default::default() });
    }
    let speed = player.vehicles[0].forward_speed(&world);
    assert!(speed > 10.0, "2 s of full throttle reached only {speed:.2} m/s");
    let z0 = player.vehicles[0].datum(&world).z;
    let mut steps = 0;
    while player.vehicles[0].forward_speed(&world) > 0.3 && steps < 600 {
        player.fixed_step(&mut world, &StepInput { brake: 1.0, ..Default::default() });
        steps += 1;
    }
    let stop = player.vehicles[0].datum(&world).z - z0;
    println!("0 -> {speed:.2} m/s in 2 s; stopped in {stop:.1} m ({steps} steps)");
    assert!(steps < 600 && stop < speed * speed / (2.0 * 6.0), "braking from {speed:.2} m/s took {stop:.1} m");
}

#[test]
fn spawning_on_a_banked_hill_loads_all_four_wheels_on_the_first_drive_step() {
    struct Hill;
    impl GroundProvider for Hill {
        fn ground_y(&self, x: f64, z: f64) -> f64 { 0.12 * x + 0.30 * z }
    }
    let mut world = CollisionWorld::new(Arc::new(Hill), Arc::new(BuildingSet::empty()), 1);
    let start = DVec3::new(8.0, Hill.ground_y(8.0, 8.0), 8.0);
    world.load_blocking(start, 60.0);
    let yaw = 0.37;
    let mut player = Player::new(start, yaw);
    let car = player.spawn_vehicle(&mut world, start.x, start.z, yaw);
    player.enter_vehicle(car);
    for _ in 0..60 {
        player.fixed_step(&mut world, &StepInput { throttle: 0.65, ..Default::default() });
        let car = &player.vehicles[0];
        let datum = car.datum(&world);
        assert_eq!(car.wheels_in_contact(), 4, "wheel contact on a continuous grade at {datum:?}");
        assert!((datum.y - Hill.ground_y(datum.x, datum.z)).abs() < 0.10);
        assert!((car.yaw(&world) - yaw).abs() < 0.05);
    }
}

#[test]
fn driving_past_an_elevated_deck_edge_loses_support_and_lands_on_the_lower_floor() {
    let y = 6.0;
    let (a, b, c, d) = ([-10.0, y, -12.0], [-10.0, y, 6.0], [10.0, y, 6.0], [10.0, y, -12.0]);
    let part = world::DeckPart { walk: vec![[a, b, c], [a, c, d]], ..Default::default() };
    let buildings = BuildingSet::empty().with_decks(world::Decks { parts: vec![part], ..Default::default() });
    let mut world = CollisionWorld::new(Arc::new(KerbStreet { curb_m: curb_height() }), Arc::new(buildings), 1);
    let start = DVec3::new(0.0, y, -6.0);
    world.load_blocking(start, 150.0);
    let mut player = Player::new(start, 0.0);
    let car = player.spawn_vehicle(&mut world, start.x, start.z, 0.0);
    player.enter_vehicle(car);
    let mut falling = false;
    for _ in 0..300 {
        let v = &player.vehicles[car];
        let d = v.datum(&world);
        let throttle = if d.z < 10.0 { 1.0 } else { 0.0 };
        player.fixed_step(&mut world, &StepInput { throttle, ..Default::default() });
        let v = &player.vehicles[car];
        if v.wheels_in_contact() == 0 && v.datum(&world).y < y - 0.5 { falling = true; }
    }
    let car = &player.vehicles[car];
    assert!(falling && car.datum(&world).z > 10.0);
    assert_eq!(car.wheels_in_contact(), 4);
    assert!(car.datum(&world).y.abs() < 0.05);
}

// ---------------------------------------------------------------------------
// The real city

struct City {
    city: eo_data::City,
    elevation: Arc<eo_data::ElevationGrid>,
    buildings: Arc<BuildingSet>,
}

fn city() -> &'static City {
    static C: OnceLock<City> = OnceLock::new();
    C.get_or_init(|| {
        let paths = eo_data::DataPaths::discover().unwrap();
        let city = eo_data::City::load(&paths).unwrap();
        let elevation = Arc::new(eo_data::ElevationGrid::load(&paths).unwrap());
        let lift: serde_json::Value = serde_json::from_slice(&std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../eo-assets/data/authored-placement.json")).unwrap()).unwrap();
        let buildings = Arc::new(BuildingSet::from_city(&city, lift["constructionLiftM"].as_f64().unwrap()));
        City { city, elevation, buildings }
    })
}

fn new_world(c: &City) -> CollisionWorld {
    CollisionWorld::new(Arc::new(ElevationGround(c.elevation.clone())), c.buildings.clone(), 4)
}

/// Whether a capsule centred at `p` overlaps a building prism (with a 2 cm tolerance).
fn inside_building(world: &CollisionWorld, p: DVec3, half_extent_y: f64) -> Option<f64> {
    for set in world.prisms_near(p.x, p.z, 2.0) {
        for b in set.iter() {
            if p.y + half_extent_y <= b.base_y || p.y - half_extent_y >= b.top_y {
                continue;
            }
            if p.x < b.min[0] - 1.0 || p.x > b.max[0] + 1.0 || p.z < b.min[1] - 1.0 || p.z > b.max[1] + 1.0 {
                continue;
            }
            let d = world::distance_to_ring(p.x, p.z, &b.ring);
            if world::point_in_ring(p.x, p.z, &b.ring) || d < 0.33 {
                return Some(if world::point_in_ring(p.x, p.z, &b.ring) { -d } else { d });
            }
        }
    }
    None
}

struct WalkReport {
    steps: usize,
    min_floor_gap: f64,
    min_ground_gap: f64,
    grounded_share: f64,
    stalls: usize,
    finished: bool,
}

fn walk(world: &mut CollisionWorld, route: &[[f64; 2]], max_steps: usize) -> WalkReport {
    let start = DVec3::new(route[0][0], world.ground.ground_y(route[0][0], route[0][1]), route[0][1]);
    world.load_blocking(start, world::LOAD_RADIUS_M);
    let mut player = Player::new(start, 0.0);
    let mut pilot = WalkPilot::new(route.to_vec(), true);
    let mut r = WalkReport { steps: 0, min_floor_gap: f64::MAX, min_ground_gap: f64::MAX, grounded_share: 0.0, stalls: 0, finished: false };
    let mut grounded = 0;
    let mut last_progress = (0usize, start);
    for step in 0..max_steps {
        let input = pilot.input(&mut player, world);
        if pilot.done() {
            r.finished = true;
            break;
        }
        player.fixed_step(world, &input);
        let feet = player.character.feet();
        world.pump(feet);
        r.steps += 1;
        let floor_gap = feet.y - world.floor_y(feet.x, feet.z);
        let ground_gap = feet.y - world.ground.ground_y(feet.x, feet.z);
        r.min_floor_gap = r.min_floor_gap.min(floor_gap);
        r.min_ground_gap = r.min_ground_gap.min(ground_gap);
        assert!(floor_gap > -0.03, "step {step}: feet {feet:?} {floor_gap:.3} m under the ground collider");
        assert!(ground_gap > -0.2, "step {step}: feet {feet:?} {ground_gap:.3} m under the terrain");
        if let Some(d) = inside_building(world, player.character.position, player.character.half_extent_y()) {
            panic!("step {step}: capsule at {:?} inside a building's reach ({d:.3} m)", player.character.position);
        }
        if player.character.grounded {
            grounded += 1;
        }
        // A stall: no 1 m of progress in 5 s; skip that waypoint (and count it).
        if (feet - last_progress.1).length() > 1.0 {
            last_progress = (step, feet);
        } else if step - last_progress.0 > 300 {
            r.stalls += 1;
            pilot.next += 1;
            last_progress = (step, feet);
        }
    }
    r.grounded_share = grounded as f64 / r.steps.max(1) as f64;
    r
}

#[test]
fn walks_twenty_blocks_and_the_steepest_street_without_falling_through() {
    let c = city();
    let route = routes::walk_route(&c.city, &c.elevation);
    let steep = routes::steepest_route(&c.city, &c.elevation);
    let blocks = route.intersections.len() - 1;
    println!(
        "walk route from {}: {:.0} m, {} intersections ({} blocks), steepest {:.1} %; steepest street {}: {:.0} m, {:.1} %",
        c.city.roads[route.start_way].name,
        routes::polyline_length(&route.path),
        route.intersections.len(),
        blocks,
        route.steepest_grade * 100.0,
        c.city.roads[steep.start_way].name,
        routes::polyline_length(&steep.path),
        steep.steepest_grade * 100.0
    );
    assert!(blocks >= 20, "only {blocks} blocks");
    // the ground heights follow the ground plan, so the drawn grade is the surveyed
    // one again (Mount Vernon Avenue); the original form holds on the drawn street.
    assert!(steep.steepest_grade >= 0.30, "the steepest street is {:.1} %", steep.steepest_grade * 100.0);
    for (name, r) in [("walk route", &route), ("steepest street", &steep)] {
        let mut world = new_world(c);
        let rep = walk(&mut world, &r.path, 60 * 60 * 15);
        println!(
            "{name}: {} steps, finished {}, min gap to the ground collider {:.3} m, to the terrain {:.3} m, grounded {:.1} %, stalls {}, tiles inserted {}, pump max {:.2} ms, cook max {:.1} ms",
            rep.steps,
            rep.finished,
            rep.min_floor_gap,
            rep.min_ground_gap,
            rep.grounded_share * 100.0,
            rep.stalls,
            world.stats.tiles_inserted,
            world.stats.pump_ms_max,
            world.stats.cook_ms_max
        );
        assert!(rep.finished, "{name}: did not reach the end");
        assert!(rep.grounded_share > 0.9, "{name}: grounded only {:.1} %", rep.grounded_share * 100.0);
        assert!(rep.stalls <= 2, "{name}: {} stalls", rep.stalls);
    }
}

fn point_along(path: &[[f64; 2]], s: f64) -> [f64; 2] {
    let mut left = s;
    for w in path.windows(2) {
        let l = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
        if left <= l {
            let t = left / l.max(1e-9);
            return [w[0][0] + (w[1][0] - w[0][0]) * t, w[0][1] + (w[1][1] - w[0][1]) * t];
        }
        left -= l;
    }
    *path.last().unwrap()
}

#[test]
fn never_passes_through_real_building_walls() {
    let c = city();
    let route = routes::walk_route(&c.city, &c.elevation);
    let mut world = new_world(c);
    // Buildings along the route: run at each from 4 m outside its nearest wall, sprinting, for 4 s.
    let mut tested = 0;
    let mut stopped = 0;
    let total = routes::polyline_length(&route.path);
    let mut s = 20.0;
    while s < total {
        let w = point_along(&route.path, s);
        s += 90.0;
        let centre = DVec3::new(w[0], 0.0, w[1]);
        world.load_blocking(centre, 60.0);
        let sets = world.prisms_near(w[0], w[1], 40.0);
        let d = |p: &world::Prism| ((p.min[0] + p.max[0]) / 2.0 - w[0]).hypot((p.min[1] + p.max[1]) / 2.0 - w[1]);
        let b = sets.iter().flat_map(|s| s.iter()).filter(|b| b.top_y - b.base_y > 4.0 && d(b) < 40.0).min_by(|a, b| d(a).partial_cmp(&d(b)).unwrap()).cloned();
        let Some(b) = b else { continue };
        let n = b.ring.len() as f64;
        let (cx, cz) = (b.ring.iter().map(|p| p[0]).sum::<f64>() / n, b.ring.iter().map(|p| p[1]).sum::<f64>() / n);
        if !world::point_in_ring(cx, cz, &b.ring) {
            continue;
        }
        // Start 4 m out from the route side, walking at the centroid.
        let dir = DVec3::new(cx - w[0], 0.0, cz - w[1]).normalize();
        let mut start = DVec3::new(w[0], 0.0, w[1]);
        while world::point_in_ring(start.x, start.z, &b.ring) || world::distance_to_ring(start.x, start.z, &b.ring) < 4.0 {
            start -= dir;
        }
        if inside_building(&world, DVec3::new(start.x, world.floor_y(start.x, start.z) + 1.0, start.z), 1.0).is_some() {
            continue;
        }
        start.y = world.floor_y(start.x, start.z);
        let mut player = Player::new(start, dir.x.atan2(dir.z));
        let input = StepInput { move_y: 1.0, sprint: true, ..Default::default() };
        let mut min_d = f64::MAX;
        for step in 0..480 {
            player.fixed_step(&mut world, &input);
            let p = player.character.position;
            assert!(!world::point_in_ring(p.x, p.z, &b.ring) || p.y - player.character.half_extent_y() > b.top_y, "step {step}: inside building at {p:?}");
            if p.y - player.character.half_extent_y() < b.top_y {
                min_d = min_d.min(world::distance_to_ring(p.x, p.z, &b.ring));
            }
            if let Some(d) = inside_building(&world, p, player.character.half_extent_y()) {
                panic!("step {step}: capsule at {p:?} within {d:.3} m of a wall");
            }
        }
        tested += 1;
        if min_d < 0.5 {
            stopped += 1;
        }
        if tested >= 25 {
            break;
        }
    }
    println!("ran at {tested} real buildings, {stopped} reached and were stopped by the wall");
    assert!(tested >= 20, "only {tested} buildings tested");
    assert!(stopped >= tested * 3 / 4, "only {stopped} of {tested} runs reached the wall");
}

#[test]
fn a_driven_car_stays_on_the_ground_over_the_hills() {
    let c = city();
    let route = routes::drive_route(&c.city, &c.elevation);
    let mut world = new_world(c);
    let p0 = route.path[0];
    let p1 = route.path[3.min(route.path.len() - 1)];
    let start = DVec3::new(p0[0], 0.0, p0[1]);
    world.load_blocking(start, world::LOAD_RADIUS_M);
    let mut player = Player::new(DVec3::new(p0[0] + 3.0, world.floor_y(p0[0] + 3.0, p0[1]), p0[1]), 0.0);
    let yaw = (p1[0] - p0[0]).atan2(p1[1] - p0[1]);
    player.spawn_vehicle(&mut world, p0[0], p0[1], yaw);
    // Let it settle on its springs, then get in.
    for _ in 0..60 {
        player.fixed_step(&mut world, &StepInput::default());
    }
    player.fixed_step(&mut world, &StepInput { vehicle_toggle: true, ..Default::default() });
    assert!(matches!(player.mode, Mode::Driving(0)), "could not enter the car");
    let mut pilot = DrivePilot::new(route.path.clone(), 12.0);
    let (mut steps, mut contact4, mut airborne, mut max_up_tilt) = (0usize, 0usize, 0usize, 0f64);
    let mut min_gap = f64::MAX;
    let mut max_gap = f64::MIN;
    let mut max_speed: f64 = 0.0;
    while !pilot.done() && steps < 60 * 60 * 8 {
        let input = pilot.input(&player, &world);
        player.fixed_step(&mut world, &input);
        let v = &player.vehicles[0];
        let d = v.datum(&world);
        world.pump(d);
        steps += 1;
        let gap = d.y - world.floor_y(d.x, d.z);
        min_gap = min_gap.min(gap);
        max_gap = max_gap.max(gap);
        max_speed = max_speed.max(v.forward_speed(&world).abs());
        let (_, rot) = v.pose(&world);
        max_up_tilt = max_up_tilt.max((rot * DVec3::Y).angle_between(DVec3::Y));
        let n = v.wheels_in_contact();
        if n == 4 {
            contact4 += 1;
        }
        if n == 0 {
            airborne += 1;
        }
        assert!(gap > -0.25, "step {steps}: car datum {gap:.3} m under the ground at {d:?}");
        assert!(gap < 1.0, "step {steps}: car datum {gap:.3} m over the ground at {d:?}");
    }
    println!(
        "drove {:.0} m in {:.1} s (max {:.1} m/s): all four wheels down {:.1} % of steps, airborne {} steps, datum gap {:.3}..{:.3} m, max tilt {:.1} deg",
        routes::polyline_length(&route.path),
        steps as f64 / 60.0,
        max_speed,
        contact4 as f64 * 100.0 / steps as f64,
        airborne,
        min_gap,
        max_gap,
        max_up_tilt.to_degrees()
    );
    assert!(pilot.done(), "the car did not finish the route in {steps} steps");
    assert!(contact4 as f64 / steps as f64 > 0.95, "four wheels down only {:.1} % of the time", contact4 as f64 * 100.0 / steps as f64);
    assert!(max_up_tilt.to_degrees() < 35.0, "the car tipped {:.1} deg", max_up_tilt.to_degrees());
    // And out again: the body stands beside the car, on the ground, not in a wall.
    let stop = StepInput { brake: 1.0, ..Default::default() };
    for _ in 0..240 {
        player.fixed_step(&mut world, &stop);
    }
    player.fixed_step(&mut world, &StepInput { vehicle_toggle: true, ..Default::default() });
    assert_eq!(player.mode, Mode::OnFoot, "could not leave the car");
    let feet = player.character.feet();
    assert!((feet.y - world.floor_y(feet.x, feet.z)).abs() < 0.5, "left the car at {feet:?}");
    assert!(inside_building(&world, player.character.position, player.character.half_extent_y()).is_none());
}

fn streets() -> &'static Arc<eo_player::street_ground::StreetGround> {
    static S: OnceLock<Arc<eo_player::street_ground::StreetGround>> = OnceLock::new();
    S.get_or_init(|| {
        let c = city();
        Arc::new(eo_player::street_ground::StreetGround::build(&c.city, c.elevation.clone()).unwrap())
    })
}

#[test]
fn steps_up_real_kerbs_onto_the_drawn_sidewalk_and_back_down() {
    // On the drawn street surface (eo-street): from the middle of a street,
    // straight across to the sidewalk beyond its kerb, at many streets.
    let c = city();
    let sg = streets();
    let sw = sg.world.clone();
    let mut world = CollisionWorld::new(sg.clone(), c.buildings.clone(), 4);
    let route = routes::walk_route(&c.city, &c.elevation);
    let total = routes::polyline_length(&route.path);
    let (mut tried, mut climbed) = (0, 0);
    let mut s = 30.0;
    while s < total - 30.0 && tried < 20 {
        let a = point_along(&route.path, s);
        let b = point_along(&route.path, s + 2.0);
        s += 137.0;
        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
        let l = dx.hypot(dz);
        let n = [-dz / l, dx / l];
        // March across until the paving steps up by a kerb.
        let Some(y0) = sw.paved_at(a[0], a[1]) else { continue };
        let mut prev = y0;
        let mut kerb = None;
        let mut d = 0.5;
        while d < 20.0 {
            let Some(y) = sw.paved_at(a[0] + n[0] * d, a[1] + n[1] * d) else { break };
            if y - prev > 0.08 {
                kerb = Some((d, y - prev));
                break;
            }
            prev = y;
            d += 0.25;
        }
        let Some((kd, rise)) = kerb else { continue };
        let target = [a[0] + n[0] * (kd + 1.2), a[1] + n[1] * (kd + 1.2)];
        let Some(top) = sw.paved_at(target[0], target[1]) else { continue };
        world.load_blocking(DVec3::new(a[0], 0.0, a[1]), 150.0);
        let start = DVec3::new(a[0], world.surface_y(a[0], a[1]), a[1]);
        if inside_building(&world, DVec3::new(target[0], top + 1.0, target[1]), 0.9).is_some() {
            continue;
        }
        tried += 1;
        let mut player = Player::new(start, n[0].atan2(n[1]));
        let mut pilot = WalkPilot::new(vec![a, target], false);
        pilot.reach = 0.3;
        for _ in 0..600 {
            let i = pilot.input(&mut player, &world);
            if pilot.done() {
                break;
            }
            player.fixed_step(&mut world, &i);
            let f = player.character.feet();
            assert!(f.y > world.floor_y(f.x, f.z) - 0.03, "under the ground at {f:?}");
        }
        let f = player.character.feet();
        let on_top = (f.x - target[0]).hypot(f.z - target[1]) < 0.5 && (f.y - top).abs() < 0.06;
        println!(
            "street at ({:.0}, {:.0}): kerb {:.3} m at {kd:.1} m; walked {:.2} m, {:.2} m short; feet {:.3} vs sidewalk {:.3}; kerb lifts {}",
            a[0],
            a[1],
            rise,
            (f.x - a[0]).hypot(f.z - a[1]),
            (f.x - target[0]).hypot(f.z - target[1]),
            f.y,
            top,
            player.character.kerb_steps
        );
        if on_top {
            climbed += 1;
            // And back down onto the carriageway.
            let mut back = WalkPilot::new(vec![target, a], false);
            back.reach = 0.3;
            for _ in 0..600 {
                let i = back.input(&mut player, &world);
                if back.done() {
                    break;
                }
                player.fixed_step(&mut world, &i);
            }
            let f = player.character.feet();
            assert!((f.y - y0).abs() < 0.3, "back on the carriageway at {f:?} (paving {y0:.3})");
        }
    }
    println!("{climbed} of {tried} kerbs climbed onto the drawn sidewalk");
    assert!(tried >= 8, "only {tried} kerbs found");
    assert!(climbed * 10 >= tried * 9, "only {climbed} of {tried} kerbs climbed");
}

#[test]
fn the_same_inputs_give_the_same_body() {
    // Determinism: two worlds, the same input sequence, bit-identical bodies.
    let c = city();
    let route = routes::walk_route(&c.city, &c.elevation);
    let run = || {
        let mut world = new_world(c);
        let start = DVec3::new(route.path[0][0], c.elevation.ground(route.path[0][0], route.path[0][1]), route.path[0][1]);
        world.load_blocking(start, 300.0);
        let mut player = Player::new(start, 0.0);
        let mut pilot = WalkPilot::new(route.path.clone(), true);
        for _ in 0..1200 {
            let i = pilot.input(&mut player, &world);
            player.fixed_step(&mut world, &i);
        }
        player.character.position
    };
    let a = run();
    let b = run();
    assert_eq!(a.to_array().map(f64::to_bits), b.to_array().map(f64::to_bits), "{a:?} vs {b:?}");
}

#[test]
fn weather_physics_changes_driving_and_walking_deterministically() {
    let run_drive = |wx: eo_player::WeatherPhysics| {
        let mut world = CollisionWorld::new(Arc::new(KerbStreet { curb_m: 0.15 }), Arc::new(BuildingSet::empty()), 1);
        world.load_blocking(DVec3::new(-60.0, 0.0, 0.0), 150.0);
        let mut player = Player::new(DVec3::new(-60.0, 0.0, 3.0), 0.0);
        player.set_weather(wx);
        player.spawn_vehicle(&mut world, -60.0, 0.0, 0.0);
        for _ in 0..60 {
            player.fixed_step(&mut world, &StepInput::default());
        }
        player.fixed_step(&mut world, &StepInput { vehicle_toggle: true, ..Default::default() });
        assert_eq!(player.mode, Mode::Driving(0));
        // Drive straight +Z at full throttle, then brake hard in wet/storm vs dry.
        for _ in 0..140 {
            player.fixed_step(&mut world, &StepInput { throttle: 1.0, ..Default::default() });
        }
        let peak_pos = player.position(&world);
        for _ in 0..90 {
            player.fixed_step(&mut world, &StepInput { brake: 1.0, ..Default::default() });
        }
        (peak_pos, player.position(&world), player.vehicles[0].wheels_in_contact())
    };
    let dry = eo_player::WeatherPhysics::default();
    let storm = eo_player::WeatherPhysics { wetness: 0.99, puddle_fill: 0.90, rain_mm_h: 22.0, wind: [12.0, 0.0], gust: 1.1 };
    let (dry_peak, dry_stop, dry_w) = run_drive(dry);
    let (st_peak, st_stop, st_w) = run_drive(storm);
    let (st_peak2, st_stop2, _) = run_drive(storm);
    assert_eq!(dry_w, 4);
    assert_eq!(st_w, 4);
    // Bit-identical determinism under storm weather physics.
    assert_eq!(st_peak.to_array().map(f64::to_bits), st_peak2.to_array().map(f64::to_bits));
    assert_eq!(st_stop.to_array().map(f64::to_bits), st_stop2.to_array().map(f64::to_bits));
    // Strong crosswind + asymmetric puddles deflect the car laterally compared to dry calm.
    assert!((st_stop.x - dry_stop.x).abs() > 0.01, "lateral storm deflection: dry={dry_stop:?} storm={st_stop:?}");
    // Wet storm surface physics measurably alters stopping distance vs dry pavement.
    let dry_brake = dry_stop.z - dry_peak.z;
    let st_brake = st_stop.z - st_peak.z;
    assert!(st_brake > dry_brake + 0.25, "wet braking distance {st_brake:.2} m should exceed dry {dry_brake:.2} m");
    // On foot: walking into a storm headwind covers less ground than walking with a tailwind.
    let run_walk = |wind: [f64; 2]| {
        let mut world = CollisionWorld::new(Arc::new(KerbStreet { curb_m: 0.15 }), Arc::new(BuildingSet::empty()), 1);
        world.load_blocking(DVec3::new(-60.0, 0.0, 0.0), 150.0);
        let mut player = Player::new(DVec3::new(-60.0, 0.0, 0.0), 0.0);
        player.set_weather(eo_player::WeatherPhysics { wetness: 0.99, puddle_fill: 0.90, rain_mm_h: 22.0, wind, gust: 1.1 });
        for _ in 0..120 {
            player.fixed_step(&mut world, &StepInput { move_y: 1.0, ..Default::default() });
        }
        player.character.feet().z
    };
    let z_head = run_walk([0.0, -12.0]);
    let z_tail = run_walk([0.0, 12.0]);
    assert!(z_head < z_tail - 0.1, "headwind z={z_head:.2} m should be shorter than tailwind z={z_tail:.2} m");
}

#[test]
fn a_standard_kerb_at_city_speed_does_not_launch_a_car() {
    use eo_player::vehicle::{VehicleConfig, VehicleModel};
    let curb = curb_height();
    for model in [VehicleModel::Hatchback, VehicleModel::Sedan, VehicleModel::DeliveryVan] {
        let mut world = CollisionWorld::new(Arc::new(KerbStreet { curb_m: curb }), Arc::new(BuildingSet::empty()), 1);
        let start = DVec3::new(2.0, 0.0, 2.0);
        world.load_blocking(start, 100.0);
        let yaw = std::f64::consts::FRAC_PI_2;
        let mut player = Player::new(start, yaw);
        player.spawn_config = VehicleConfig { model, ..Default::default() };
        let i = player.spawn_vehicle(&mut world, start.x, start.z, yaw);
        player.enter_vehicle(i);
        for _ in 0..120 { player.fixed_step(&mut world, &StepInput::default()); }
        let handle = player.vehicles[i].body;
        world.physics.bodies[handle].set_linvel([8.0, 0.0, 0.0].into(), true);
        let mut longest_air = 0;
        let mut air = 0;
        let mut highest: f64 = 0.0;
        for _ in 0..200 {
            player.fixed_step(&mut world, &StepInput { throttle: 0.15, ..Default::default() });
            let car = &player.vehicles[i];
            let up = car.pose(&world).1 * DVec3::Y;
            let pos = car.datum(&world);
            highest = highest.max(pos.y);
            air = if car.wheels_in_contact() == 0 { air + 1 } else { 0 };
            longest_air = longest_air.max(air);
            assert!(up.y > 0.8 && pos.y > -0.15, "{model:?} at {pos:?}");
        }
        assert!(player.position(&world).x > 23.0, "{model:?} did not cross the test kerb");
        assert!(highest < curb + 0.4 && longest_air < 15, "{model:?}: peak {highest} m, airborne {longest_air} ticks");
    }
}

#[test]
fn rendered_walkable_street_layers_have_physics_support_across_the_city() {
    let sg = streets();
    let keys = sg.world.tile_keys();
    let coverage = std::env::var_os("EO_AUDIT_ALL_STREET_TILES");
    let dense = coverage.as_deref() == Some(std::ffi::OsStr::new("dense"));
    let stride = if coverage.is_some() { 1 } else { keys.len().div_ceil(96).max(1) };
    let mut totals = [0usize; 7];
    let mut visited = 0;
    let mut missed = 0;
    let mut misses = Vec::new();
    for &(tx, tz) in keys.iter().step_by(stride) {
        let mesh = sg.world.cached_mesh((tx, tz));
        let mut checked = [false; 7];
        let mut samples = Vec::new();
        for (&layer, triangle) in mesh.layers.iter().zip(mesh.indices.chunks_exact(3)) {
            if !layer.walkable() || (!dense && checked[layer as usize]) { continue; }
            let vertex = |i: u32| DVec3::from_array(mesh.positions[i as usize].map(f64::from));
            let (a, b, c) = (vertex(triangle[0]), vertex(triangle[1]), vertex(triangle[2]));
            let at = (a + b + c) / 3.0;
            if (b-a).cross(c-a).y.abs() < 0.005 { continue; }
            // The dense audit also casts against faces that project beyond
            // their mesh's tile boundary, with only the owning tile loaded.
            if !dense && eo_street::tile_key(at.x, at.z) != (tx,tz) { continue; }
            samples.push((layer, at));
            checked[layer as usize] = true;
        }
        if samples.is_empty() { continue; }
        // A fresh world gives every sampled tile the same cold-load test.
        let mut world = CollisionWorld::new(sg.clone(), Arc::new(BuildingSet::empty()), 1);
        let width = world::TILE_SIZE_M;
        world.load_blocking(DVec3::new((f64::from(tx)+0.5)*width, 0.0, (f64::from(tz)+0.5)*width), 0.0);
        for (layer, at) in samples {
            let hit = world.raycast(at + DVec3::Y * 0.025, DVec3::NEG_Y, 0.05, world::layer::STREET, None);
            if hit.is_none() {
                missed += 1;
                if misses.len() < 12 { misses.push((tx,tz,layer,at)); }
            }
            totals[layer as usize] += 1;
        }
        visited += 1;
    }
    eprintln!("street floor audit: {visited} nonempty tiles from {} map tiles, floor samples by layer {totals:?}", keys.len());
    assert!(visited >= 15 && totals[eo_street::Layer::Sidewalk as usize] >= 10);
    assert_eq!(missed, 0, "missing physical floors, first locations {misses:?}");
}

#[test]
fn ground_probes_keep_tile_local_precision_at_distant_road_seams() {
    let current = streets();
    let c = city();
    let mut options = current.world.o.clone();
    options.pavement_repairs = false;
    // These fixed triangle centres were recorded with the original paving.
    // Keep the centimetre-sensitive cases even where the repaired mesh removes a face.
    let original = Arc::new(eo_player::street_ground::StreetGround::build_with_options(
        &c.city, c.elevation.clone(), None, options,
    ).unwrap());
    for (tx, tz, at) in [
        (-80, 3, DVec3::new(-10108.826497395834, 8.958022435506185, 388.4601745605469)),
        (-60, 47, DVec3::new(-7662.472493489583, 43.02194595336914, 6116.948079427083)),
        (-44, 17, DVec3::new(-5630.824544270833, 101.34500885009766, 2208.6476236979165)),
    ] {
        let mut world = CollisionWorld::new(original.clone(), Arc::new(BuildingSet::empty()), 1);
        let width = world::TILE_SIZE_M;
        let centre = DVec3::new((f64::from(tx) + 0.5) * width, at.y, (f64::from(tz) + 0.5) * width);
        world.load_blocking(centre, 0.0);
        let probe = |world: &CollisionWorld, point| {
            world.raycast(point + DVec3::Y * 0.025, DVec3::NEG_Y, 0.05, world::layer::STREET, None)
        };
        assert!(probe(&world, at).is_some(), "missing original street surface at {at:?}; drawn heights {:?}, current {:?}, resident tile top {:?}",
            original.world.paved_heights(at.x, at.z),
            current.world.paved_heights(at.x, at.z),
            world.raycast(at + DVec3::Y, DVec3::NEG_Y, 2., world::layer::STREET, None).map(|hit| hit.point.y));

        // Neighbouring tiles can own triangles that reach these coordinates.
        let mut world = CollisionWorld::new(current.clone(), Arc::new(BuildingSet::empty()), 1);
        world.load_blocking(centre, width);
        let heights = current.world.paved_heights(at.x, at.z);
        assert!(!heights.is_empty(), "missing drawn paving at {at:?}");
        for y in heights {
            let point = DVec3::new(at.x, y, at.z);
            assert!(probe(&world, point).is_some(), "missing current street surface at {point:?}");
        }
    }
}

#[test]
fn market_street_tire_rays_hit_the_drawn_asphalt_across_the_seam() {
    let ground = streets();
    // Opposing carriageways meet here. A tire ray used to reach the ground
    // about half a metre below the road through a submillimetre gap.
    let [x, z] = eo_data::groundscale::plan([-568.730954205614, 876.533983100553]);
    let road = &ground.world;
    let height = road.carriageway_near(x, z, road.ground(x, z)).expect("Market Street asphalt");
    let mut collision = CollisionWorld::new(ground.clone(), Arc::new(BuildingSet::empty()), 1);
    collision.load_blocking(DVec3::new(x, height, z), world::LOAD_RADIUS_M);
    let (tx, tz) = eo_street::tile_key(x, z);
    let mut faces = Vec::new();
    for dx in -1..=1 {
        for dz in -1..=1 {
            let mesh = road.cached_mesh((tx + dx, tz + dz));
            for (face, layer) in mesh.indices.chunks_exact(3).zip(&mesh.layers) {
                if *layer == eo_street::Layer::Carriageway {
                    faces.push([0, 1, 2].map(|i| DVec3::from_array(mesh.positions[face[i] as usize].map(f64::from))));
                }
            }
        }
    }
    let mesh_step = |v: f64| {
        let v = v.abs() as f32;
        f64::from(v.next_up() - v)
    };
    for i in -4..=4 {
        for j in -4..=4 {
            let qx = x + f64::from(i) * mesh_step(x);
            let qz = z + f64::from(j) * mesh_step(z);
            assert!(road.only_carriageway_at(qx, qz), "missing asphalt at {qx}, {qz}");
            // The gameplay floor query allows a small margin outside an edge.
            // At this scale the reference must be the exact drawn footprint.
            let heights: Vec<_> = faces.iter().filter_map(|[a, b, c]| {
                let (ab, ac) = (*b - *a, *c - *a);
                let q = DVec3::new(qx - a.x, 0.0, qz - a.z);
                let area = ab.cross(ac).y;
                if area == 0.0 { return None; }
                let (u, v) = (q.cross(ac).y / area, ab.cross(q).y / area);
                (u >= 0.0 && v >= 0.0 && u + v <= 1.0).then_some(a.y + u * ab.y + v * ac.y)
            }).collect();
            let top = heights.iter().copied().reduce(f64::max).expect("seam inside the drawn asphalt");
            for expected in heights {
                // Approach the top from tire height. Probe any overlapping
                // lower face from just above it, underneath the top face.
                let clearance = if expected == top { 0.1 } else { 0.00025 };
                let start = DVec3::new(qx, expected + clearance, qz);
                let hit = collision.raycast(start, DVec3::NEG_Y, clearance + 0.0005, world::layer::STREET, None)
                    .unwrap_or_else(|| panic!("missing physical asphalt at {qx}, {expected}, {qz}"));
                assert!((hit.point.y - expected).abs() <= 0.0005,
                    "tire ray hit {} at {qx}, {qz}, drawn road {expected}", hit.point.y);
            }
        }
    }
}

/// This drive meets the steep sidewalk on Saint Charles Avenue at an angle.
/// Its two front tires reach the receiving pavement at different heights.
fn saint_charles_drive(street: &eo_street::StreetWorld) -> &eo_street::plan::Entry {
    street
        .plan
        .entries
        .iter()
        .find(|e| {
            e.id == "sf-seg-652"
                && street.city.segments[e.road].street_id.as_deref() == Some("sf-street-459121915")
        })
        .expect("Saint Charles service drive")
}

#[test]
fn saint_charles_driveway_and_sidewalk_can_be_walked_in_both_directions() {
    let ground = streets();
    let street = &ground.world;
    let drive = saint_charles_drive(street);
    let avenue = street
        .plan
        .entries
        .iter()
        .find(|e| {
            e.id == "sf-seg-44"
                && street.city.segments[e.road].street_id.as_deref() == Some("sf-street-8917011")
        })
        .expect("Saint Charles Avenue");
    let mut paths = Vec::new();
    for fraction in [-0.7, 0., 0.7] {
        let offset = fraction * drive.half;
        paths.push([
            drive.frame(drive.length - 14., false).at(offset),
            drive.frame(drive.length, false).at(offset),
        ]);
    }
    for fraction in [0.15, 0.5, 0.85] {
        let offset = avenue.half + fraction * avenue.walks[0];
        paths.push([
            avenue.frame(avenue.length - 13., false).at(offset),
            avenue.frame(avenue.length - 1., false).at(offset),
        ]);
    }
    for [a, b] in paths {
        for [from, to] in [[a, b], [b, a]] {
            let start = DVec3::new(
                from[0],
                street.paved_at(from[0], from[1]).expect("drawn pavement"),
                from[1],
            );
            let delta = DVec3::new(to[0] - from[0], 0., to[1] - from[1]);
            let direction = delta.normalize();
            let mut world = CollisionWorld::new(ground.clone(), city().buildings.clone(), 1);
            world.load_blocking(start, world::LOAD_RADIUS_M);
            let mut player = Player::new(start, direction.x.atan2(direction.z));
            for _ in 0..30 {
                player.fixed_step(&mut world, &StepInput::default());
            }
            let mut crossed = false;
            for _ in 0..1200 {
                let previous = player.character.feet();
                player.fixed_step(
                    &mut world,
                    &StepInput {
                        move_y: 1.,
                        ..Default::default()
                    },
                );
                let feet = player.character.feet();
                let surface = street.paved_at(feet.x, feet.z).expect("continuous paving");
                assert!(surface - feet.y < 0.05, "feet below the entrance at {feet}");
                assert!(
                    previous.y - feet.y < 0.10,
                    "dropped while crossing at {feet}"
                );
                if (feet - start).dot(direction) >= delta.length() {
                    crossed = true;
                    break;
                }
            }
            assert!(
                crossed,
                "walk from {from:?} to {to:?} stopped at {}",
                player.character.feet()
            );
        }
    }
}

#[test]
fn the_saint_charles_driveway_carries_both_wheel_tracks_onto_the_sidewalk() {
    let entry = saint_charles_drive(&streets().world);
    let mouth = entry
        .yield_spans
        .last()
        .expect("drive ends at the through street")[0];
    check_wheel_tracks(entry, [(mouth - 20.0).max(entry.s0()), entry.length], entry.length, 0.30);
}

#[path = "collision/continuous_streets.rs"]
mod continuous_streets;

#[path = "collision/mariposa.rs"]
mod mariposa;

#[test]
fn the_san_jose_avenue_corner_keeps_both_wheel_tracks_on_the_carriageway() {
    let street = &streets().world;
    let entry = street.plan.entries.iter().find(|e| {
        street.city.segments[e.road].street_id.as_deref() == Some("sf-street-27145708")
    }).expect("San Jose Avenue by the divided road junction");
    let station = 22.2776 * eo_data::groundscale::scale();
    check_wheel_tracks(
        entry,
        [(station - 12.).max(entry.s0()), (station + 10.).min(entry.s1())],
        station + 3.,
        -0.60,
    );
}

fn check_wheel_tracks(entry: &eo_street::plan::Entry, route: [f64; 2], finish_s: f64, wheel_fraction: f64) {
    use eo_player::vehicle::{VehicleConfig, VehicleModel};

    let ground = streets();
    let street = &ground.world;
    let finish = entry.frame(finish_s, false);
    for model in [VehicleModel::Sedan, VehicleModel::DeliveryVan] {
        let config = VehicleConfig {
            model,
            occupants_kg: 80,
            cargo_kg: if model == VehicleModel::DeliveryVan {
                600
            } else {
                0
            },
        };
        let offset = wheel_fraction * entry.half - wheel_fraction.signum() * config.spec().track_front * 0.5;
        let path: Vec<_> = (0..=36)
            .map(|i| {
                let s = route[0] + (route[1] - route[0]) * f64::from(i) / 36.0;
                entry.frame(s, false).at(offset)
            })
            .collect();
        let [x, z] = path[0];
        let start = DVec3::new(x, street.paved_at(x, z).expect("road surface"), z);
        let yaw = (path[1][0] - x).atan2(path[1][1] - z);
        let mut world = CollisionWorld::new(ground.clone(), city().buildings.clone(), 1);
        world.load_blocking(start, world::LOAD_RADIUS_M);
        let mut player = Player::new(start, yaw);
        player.spawn_config = config;
        let car = player.spawn_vehicle_below(&mut world, start + DVec3::Y * 0.35, yaw);
        player.enter_vehicle(car);
        for _ in 0..120 {
            player.fixed_step(
                &mut world,
                &StepInput {
                    handbrake: true,
                    ..Default::default()
                },
            );
        }
        let mut pilot = DrivePilot::new(path, 6.0);
        let mut crossed = false;
        let mut max_loss = 0.0_f64;
        for _ in 0..900 {
            let position = player.position(&world);
            if (position.x - finish.x) * finish.tx + (position.z - finish.z) * finish.tz >= 0.0 {
                crossed = true;
                break;
            }
            world.ensure_step_tiles(position);
            let input = pilot.input(&player, &world);
            let before = player.vehicles[car].forward_speed(&world);
            player.fixed_step(&mut world, &input);
            let vehicle = &player.vehicles[car];
            if vehicle.wheels_in_contact() != 4 {
                for (wheel, suspension) in vehicle.controller.wheels().iter().enumerate() {
                    let ray = suspension.raycast_info();
                    let p = ray.hard_point_ws;
                    let hit = ray.contact_point_ws;
                    let collider = ray.ground_object.and_then(|h| world.physics.colliders.get(h));
                    let kind = collider.map(|c| (c.shape().shape_type(), c.collision_groups().memberships));
                    println!("wheel {wheel} contact {} at {p:?}, hit {hit:?}, collider {kind:?}, terrain {:.3}, drawn heights {:?}",
                        ray.is_in_contact, street.carpet_y(hit.x.into(), hit.z.into()), street.paved_heights(hit.x.into(), hit.z.into()));
                }
            }
            assert_eq!(vehicle.wheels_in_contact(), 4, "{model:?} at {position}");
            let loss = before - vehicle.forward_speed(&world);
            max_loss = max_loss.max(loss);
            assert!(loss < 0.5, "{model:?} jolted at {position}");
        }
        assert!(crossed, "{model:?} stopped before station {finish_s} on {}", entry.id);
        println!(
            "{model:?} {:.0} kg crossed {:.2} m on four tires, largest one-step speed loss {max_loss:.3} m/s",
            config.spec().mass_kg,
            player.position(&world).distance(start)
        );
    }
}

#[test]
fn a_sedan_preserves_momentum_across_surveyed_city_grade_changes() {
    use eo_player::vehicle::{VehicleConfig, VehicleModel};
    let c = city();
    let sg = streets();
    // Survey-plan coordinates of three sampled changes of grade, with direction of travel.
    let sites = [
        ("22nd Street", [1475.8136, 2315.3928], [-0.997, 0.077]),
        ("Filbert Street", [84.6480, -2459.4888], [0.988, -0.156]),
        ("Broadway", [89.9700, -2019.8100], [0.989, -0.151]),
    ];
    for (name, site, heading) in sites {
        let [x, z] = eo_data::groundscale::plan(site);
        let dir = DVec3::new(heading[0], 0.0, heading[1]).normalize();
        let mut w = CollisionWorld::new(sg.clone(), c.buildings.clone(), 2);
        let mut start = DVec3::new(x, sg.ground_y(x, z), z) - dir * 16.0;
        w.load_blocking(start, 60.0);
        start.y = w.surface_y(start.x, start.z);
        let yaw = dir.x.atan2(dir.z);
        let mut p = Player::new(start, yaw);
        p.spawn_config = VehicleConfig {
            model: VehicleModel::Sedan,
            ..Default::default()
        };
        let i = p.spawn_vehicle(&mut w, start.x, start.z, yaw);
        p.enter_vehicle(i);
        for _ in 0..120 {
            p.fixed_step(
                &mut w,
                &StepInput {
                    brake: 1.0,
                    ..Default::default()
                },
            );
        }
        let body = p.vehicles[i].body;
        let f = p.vehicles[i].pose(&w).1 * DVec3::Z;
        w.physics.bodies[body].set_linvel((f * 13.5).as_vec3().to_array().into(), true);
        let mut travel = 0.0;
        let mut loss = 0.0_f64;
        let mut airborne = 0;
        let mut longest = 0;
        let mut max_gap = 0.0_f64;
        let mut largest_vertical_step = 0.0_f64;
        for _ in 0..300 {
            let previous = w.physics.bodies[body].linvel();
            let last_y = p.vehicles[i].datum(&w).y;
            p.fixed_step(&mut w, &Default::default());
            let car = &p.vehicles[i];
            let datum = car.datum(&w);
            travel = (datum - start).dot(dir);
            largest_vertical_step = largest_vertical_step.max((datum.y - last_y).abs());
            max_gap = max_gap.max(datum.y - w.surface_below(datum.x, datum.z, datum.y + 1.0));
            loss = loss.max(
                DVec3::from_array(
                    (previous - w.physics.bodies[body].linvel())
                        .to_array()
                        .map(f64::from),
                )
                .dot(dir),
            );
            if car.wheels_in_contact() == 0 {
                airborne += 1;
                longest = longest.max(airborne);
            } else {
                airborne = 0;
            }
            if travel >= 32.0 {
                break;
            }
        }
        println!(
            "{name} at ({x:.2},{z:.2}): travelled {travel:.2} m, max coasting speed loss per tick {loss:.3} m/s, max datum-road gap {max_gap:.3} m, max vertical step {largest_vertical_step:.3} m, longest airborne run {longest} ticks"
        );
        assert!(
            max_gap < 0.35 && largest_vertical_step < 0.2 && longest < 12,
            "{name}: lost the carriageway"
        );
        assert!(
            travel >= 32.0 && loss < 0.5,
            "{name}: blocked or lost momentum on the surveyed grade"
        );
    }
}

#[test]
fn sidewalk_corners_leave_the_us_101_exit_and_octavia_carriageways_drivable() {
    use eo_player::vehicle::VehicleModel;
    let city = city();
    let street = streets();
    let road = |id: &str| {
        &city
            .city
            .roads
            .iter()
            .find(|r| r.id == id)
            .unwrap()
            .points64
    };
    for (name, first, last) in [
        ("US 101 at Mission Street", "119237657", "1418586067"),
        ("Octavia Boulevard at Market Street", "24603573", "87554961"),
    ] {
        let mut path = road(first).clone();
        assert_eq!(path.last(), road(last).first());
        path.extend_from_slice(&road(last)[1..]);
        for (model, cargo_kg) in [
            (VehicleModel::Sedan, 0),
            (VehicleModel::CatalogueSedan, 0),
            (VehicleModel::Hatchback, 0),
            (VehicleModel::DeliveryVan, 0),
            (VehicleModel::DeliveryVan, 600),
        ] {
            let [x, z] = path[0];
            let mut world = CollisionWorld::new(street.clone(), city.buildings.clone(), 1);
            world.load_blocking(DVec3::new(x, street.ground_y(x, z), z), 120.0);
            let yaw = (path[1][0] - x).atan2(path[1][1] - z);
            let mut p = Player::new(DVec3::new(x, world.surface_y(x, z), z), yaw);
            p.spawn_config.model = model;
            p.spawn_config.cargo_kg = cargo_kg;
            let i = p.spawn_vehicle(&mut world, x, z, yaw);
            p.enter_vehicle(i);
            let mut pilot = DrivePilot::new(path.clone(), 14.0);
            let mut worst_loss = 0.0_f64;
            let mut rolling = false;
            let mut airborne = 0_u32;
            let mut longest_flight = 0;
            for _ in 0..900 {
                let input = pilot.input(&p, &world);
                let before = p.vehicles[i].forward_speed(&world);
                p.fixed_step(&mut world, &input);
                worst_loss = worst_loss.max(before - p.vehicles[i].forward_speed(&world));
                if p.vehicles[i].wheels_in_contact() > 0 {
                    rolling = true;
                    airborne = 0;
                } else if rolling {
                    airborne += 1;
                    longest_flight = longest_flight.max(airborne);
                }
                if pilot.done() {
                    break;
                }
            }
            let end = p.position(&world);
            let last = path.last().unwrap();
            let remaining = (last[0] - end.x).hypot(last[1] - end.z);
            println!(
                "{name} {model:?} + {cargo_kg} kg: distance to end {remaining:.2} m, largest speed loss {worst_loss:.3} m/s in a tick, longest flight {longest_flight} ticks"
            );
            assert!(pilot.done() && remaining < 13.0 && worst_loss < 1.0);
            assert!(rolling && longest_flight == 0, "{name} {model:?} lost the road");
        }
    }
    // The sidewalk alongside the ramp keeps a walkable top.
    let mut world = CollisionWorld::new(street.clone(), city.buildings.clone(), 1);
    let [x, z] = eo_data::groundscale::plan([-31.6, 838.16]);
    world.load_blocking(DVec3::new(x, street.ground_y(x, z), z), 64.0);
    assert_eq!(
        street.world.paved_layer_at(x, z),
        Some(eo_street::Layer::Sidewalk)
    );
    let y = street.world.paved_at(x, z).unwrap();
    let mut walker = Player::new(DVec3::new(x, y + 0.8, z), 0.0);
    for _ in 0..120 {
        walker.fixed_step(&mut world, &StepInput::default());
    }
    assert!((walker.position(&world).y - y).abs() < 0.03);
}
