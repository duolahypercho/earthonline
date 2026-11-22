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
