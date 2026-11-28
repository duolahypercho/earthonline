//! Controller regressions on the measured Mariposa Street rail crossing.
use super::{city, streets};
use eo_data::{bridge_survey, groundscale::plan};
use eo_player::routes::DrivePilot;
use eo_player::street_ground::StreetGround;
use eo_player::vehicle::{VehicleConfig, VehicleModel};
use eo_player::world::{self, CollisionWorld};
use eo_player::{Player, StepInput};
use glam::DVec3;
use std::sync::{Arc, OnceLock};

fn ground(structures: bool) -> Arc<StreetGround> {
    static JOINED: OnceLock<Arc<StreetGround>> = OnceLock::new();
    if structures {
        JOINED
            .get_or_init(|| {
                let c = city();
                Arc::new(
                    StreetGround::build_joined(
                        &c.city,
                        c.elevation.clone(),
                        Some(Default::default()),
                    )
                    .unwrap(),
                )
            })
            .clone()
    } else {
        streets().clone()
    }
}

#[test]
fn walk_the_corner_and_apron_in_both_directions() {
    for structures in [false, true] {
        let ground = ground(structures);
        let street = &ground.world;
        let mut world = CollisionWorld::new(ground.clone(), city().buildings.clone(), 1);
        // Recorded southwest curb, the deck behind it, and the neighbouring control.
        for (a, b) in [
            ([2298.20, 1550.60], [2300.40, 1550.60]),
            ([2299.32, 1549.12], [2299.32, 1551.20]),
            ([2300.36, 1549.12], [2300.36, 1551.20]),
        ] {
            for [a, b] in [[plan(a), plan(b)], [plan(b), plan(a)]] {
                let delta = DVec3::new(b[0] - a[0], 0., b[1] - a[1]);
                let dir = delta.normalize();
                let lower = |x, z| {
                    let y = bridge_survey::game_y(12.9);
                    street
                        .paved_heights(x, z)
                        .into_iter()
                        .filter(|h| (h - y).abs() < 0.5)
                        .reduce(f64::max)
                        .expect("continuous surveyed lower pavement")
                };
                let p = DVec3::new(a[0], lower(a[0], a[1]), a[1]);
                world.load_blocking(p, world::LOAD_RADIUS_M);
                let mut walker = Player::new(p, dir.x.atan2(dir.z));
                let mut max_error = 0.0_f64;
                for _ in 0..240 {
                    let previous = walker.character.feet();
                    walker.fixed_step(
                        &mut world,
                        &StepInput {
                            move_y: 1.,
                            ..Default::default()
                        },
                    );
                    let feet = walker.character.feet();
                    let error = (feet.y - lower(feet.x, feet.z)).abs();
                    max_error = max_error.max(error);
                    assert!(
                        error < 0.10,
                        "lost sidewalk at {feet}, structures={structures}"
                    );
                    assert!(previous.y - feet.y < 0.10, "dropped at {feet}");
                    if (feet - p).dot(dir) >= delta.length() {
                        break;
                    }
                }
                assert!(
                    (walker.character.feet() - p).dot(dir) >= delta.length(),
                    "blocked from {a:?} toward {b:?}"
                );
                println!(
                    "walk {a:?} to {b:?}, structures={structures}, largest foot-to-surface distance {max_error:.3} m"
                );
            }
        }
    }
}

#[test]
fn four_tires_stay_on_the_through_street_and_ascending_ramp() {
    for structures in [false, true] {
        let ground = ground(structures);
        let street = &ground.world;
        let fork = [[2284.0, 1542.2], [2315.0, 1539.5]];
        let lower_ramp = [[2307.8, 1548.0], [2314.8, 1605.0]];
        let rail_crossing = [
            [2261.8, 1543.8],
            [2306.9, 1540.3],
            [2313.7, 1539.7],
            [2356.1, 1536.2],
            [2367.9, 1535.1],
        ];
        let entrance = [
            [2307.8, 1548.0],
            [2314.0, 1599.4],
            [2315.9, 1615.6],
            [2324.8, 1691.3],
            [2336.3, 1760.7],
            [2338.0, 1774.7],
        ];
        for (route, stations, minimum_datum, deadline) in [
            ("fork", fork.as_slice(), Some(12.0), 1200),
            ("lower ramp", lower_ramp.as_slice(), Some(12.0), 1200),
            ("rail crossing", rail_crossing.as_slice(), None, 3000),
            ("entrance", entrance.as_slice(), Some(12.0), 6000),
        ] {
            let path: Vec<_> = stations.iter().copied().map(plan).collect();
            let a = path[0];
            let b = *path.last().unwrap();
            let yaw = (path[1][0] - a[0]).atan2(path[1][1] - a[1]);
            let reached = |p: DVec3| (p.x - b[0]).hypot(p.z - b[1]) < 3.;
            let start = DVec3::new(
                a[0],
                street
                    .carriageway_near(a[0], a[1], bridge_survey::game_y(12.9))
                    .unwrap(),
                a[1],
            );
            for model in [VehicleModel::Sedan, VehicleModel::DeliveryVan] {
                let mut world = CollisionWorld::new(ground.clone(), city().buildings.clone(), 1);
                world.load_blocking(start, world::LOAD_RADIUS_M);
                let mut driver = Player::new(start, yaw);
                driver.spawn_config = VehicleConfig {
                    model,
                    occupants_kg: 80,
                    cargo_kg: if model == VehicleModel::DeliveryVan {
                        600
                    } else {
                        0
                    },
                };
                let car = driver.spawn_vehicle_below(&mut world, start + DVec3::Y * 0.35, yaw);
                driver.enter_vehicle(car);
                assert!((world.surface_beside(start, start.x, start.z) - start.y).abs() < 0.05);
                for _ in 0..120 {
                    driver.fixed_step(
                        &mut world,
                        &StepInput {
                            handbrake: true,
                            ..Default::default()
                        },
                    );
                }
                let mut pilot = DrivePilot::new(path.clone(), 6.);
                let mut max_loss = 0.0_f64;
                for _ in 0..deadline {
                    let pos = driver.position(&world);
                    assert!(
                        minimum_datum.is_none_or(|y| pos.y > bridge_survey::game_y(y)),
                        "{model:?} below its measured road at {pos}"
                    );
                    if reached(pos) {
                        break;
                    }
                    world.ensure_step_tiles(pos);
                    let input = pilot.input(&driver, &world);
                    let before = driver.vehicles[car].forward_speed(&world);
                    driver.fixed_step(&mut world, &input);
                    assert_eq!(
                        driver.vehicles[car].wheels_in_contact(),
                        4,
                        "{model:?} at {pos}, structures={structures}"
                    );
                    for wheel in driver.vehicles[car].controller.wheels() {
                        let hit = wheel.raycast_info().ground_object.expect("wheel contact");
                        assert!(
                            world.physics.colliders[hit]
                                .collision_groups()
                                .memberships
                                .intersects(world::layer::STREET),
                            "{model:?} bearing on the terrain below the road at {pos}"
                        );
                    }
                    let loss = before - driver.vehicles[car].forward_speed(&world);
                    max_loss = max_loss.max(loss);
                    assert!(loss < 0.5, "{model:?} jolted at {pos}");
                }
                assert!(
                    reached(driver.position(&world)),
                    "{model:?} stopped at {}",
                    driver.position(&world)
                );
                println!(
                    "drive {route} {a:?} to {b:?}, structures={structures}, {model:?}, vehicle mass {:.0} kg, four tires on paving, largest step speed loss {max_loss:.3} m/s",
                    driver.spawn_config.spec().mass_kg
                );
            }
        }
    }
}

#[test]
fn lower_ramp_and_upper_freeway_have_separate_collision_surfaces() {
    let ground = ground(true);
    let street = &ground.world;
    let mut world = CollisionWorld::new(ground.clone(), city().buildings.clone(), 1);
    let decks = ground.decks().expect("joined road structures");
    let [x, z] = plan([2315.2, 1592.0]);
    let lower = street
        .carriageway_near(x, z, bridge_survey::game_y(14.8))
        .unwrap();
    let upper = decks.deck_at(x, z, 2).expect("I-280 above the entrance").y;
    assert!(upper - lower > 6., "upper deck collapsed onto the ramp");
    println!("stacked physical floors at ({x}, {z}), lower {lower:.3} m, upper {upper:.3} m");
    let p = DVec3::new(x, lower, z);
    world.load_blocking(p, world::LOAD_RADIUS_M);
    for y in [lower, upper] {
        let hit = world.raycast(
            DVec3::new(x, y + 0.15, z),
            DVec3::NEG_Y,
            0.30,
            world::layer::STREET,
            None,
        );
        assert!(hit.is_some(), "no physical floor at ({x}, {y}, {z})");
    }
    let clearance = eo_data::standards::CaStandards::bundled()
        .unwrap()
        .metric("structure-vertical-clearance-freeway")
        .unwrap();
    // The high bank beside this lane once generated a fill wall across it.
    for at in [[2318.6, 1638.3], [2319.1, 1643.3], [2319.5, 1645.9]] {
        let [x, z] = plan(at);
        let road = street
            .carriageway_near(x, z, bridge_survey::game_y(17.5))
            .unwrap();
        let origin = DVec3::new(x, road + 0.05, z);
        world.ensure_step_tiles(origin);
        let ceiling = world
            .raycast(
                origin,
                DVec3::Y,
                20.0,
                world::layer::WORLD | world::layer::STREET,
                None,
            )
            .expect("overhead freeway concrete");
        assert!(
            ceiling.point.y - road >= clearance,
            "obstructed ramp at {at:?}: {ceiling:?}"
        );
    }
    let [x, z] = plan([2299.32, 1551.20]);
    let deck = street
        .paved_heights(x, z)
        .into_iter()
        .find(|y| (*y - bridge_survey::game_y(12.9)).abs() < 0.5)
        .expect("lower concrete apron");
    let ground_y = street.ground(x, z);
    assert!(deck - ground_y > 1.0);
    assert!(
        world
            .raycast(
                DVec3::new(x, deck - 0.25, z),
                DVec3::NEG_Y,
                deck - ground_y - 0.5,
                world::layer::STREET,
                None,
            )
            .is_none(),
        "phantom floor between apron and rail bed"
    );
}
