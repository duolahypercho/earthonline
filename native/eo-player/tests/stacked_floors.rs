//! Cross-floor interactions use the level occupied by the actor and the car.
use eo_player::character::Character;
use eo_player::world::{BuildingSet, CollisionWorld, DeckPart, Decks, GroundProvider};
use eo_player::{Mode, Player, StepInput, tuning};
use glam::DVec3;
use std::sync::Arc;

struct Flat;
impl GroundProvider for Flat {
    fn ground_y(&self, _: f64, _: f64) -> f64 {
        0.0
    }
}

fn platform(y: f64) -> Vec<[[f64; 3]; 3]> {
    let p = [[-8., y, -8.], [-8., y, 8.], [8., y, 8.], [8., y, -8.]];
    vec![[p[0], p[1], p[2]], [p[0], p[2], p[3]]]
}

fn garage(y: f64) -> CollisionWorld {
    garage_with_floor(platform(y))
}

#[test]
fn a_walker_rides_a_turning_vehicle_and_leaves_with_its_contact_velocity() {
    use eo_player::world::layer;
    use rapier3d::prelude::*;

    for kind in [
        RigidBodyType::Dynamic,
        RigidBodyType::KinematicVelocityBased,
        RigidBodyType::KinematicPositionBased,
    ] {
        let mut world = garage_with_floor(vec![]);
        let (vehicle, _) = world.physics.insert(
            RigidBodyBuilder::new(kind)
                .gravity_scale(0.)
                .translation(Vector::Y * 2.),
            ColliderBuilder::cuboid(1.2, 0.5, 2.4)
                .mass(1500.)
                .collision_groups(InteractionGroups::new(
                    layer::VEHICLE,
                    Group::ALL,
                    InteractionTestMode::And,
                )),
        );
        let mut player = Player::new(DVec3::new(0., 2.5, 1.), 0.);
        for _ in 0..10 {
            player.fixed_step(&mut world, &StepInput::default());
        }
        assert!(player.character.grounded);
        let feet = |p: &Player| Vector::from_array(p.character.feet().as_vec3().to_array());
        let local = world.physics.bodies[vehicle].position().inverse() * feet(&player);
        let velocity = Vector::new(2., 0.2, 0.4);
        let turn_rate = 0.45;
        let dt = tuning::FIXED_STEP_S as f32;
        let skin = tuning::capsule::CONTROLLER_OFFSET_M as f32;
        if kind != RigidBodyType::KinematicPositionBased {
            world.physics.bodies[vehicle].set_linvel(velocity, true);
            world.physics.bodies[vehicle].set_angvel(Vector::Y * turn_rate, true);
        }
        let mut max_roof_error = 0.0_f32;
        for tick in 0..90 {
            if kind == RigidBodyType::KinematicPositionBased {
                let time = (tick + 1) as f32 * dt;
                world.physics.bodies[vehicle].set_next_kinematic_position(Pose::from_parts(
                    Vector::Y * 2. + velocity * time,
                    Rotation::from_rotation_y(turn_rate * time),
                ));
            }
            player.fixed_step(&mut world, &StepInput::default());
            let expected = world.physics.bodies[vehicle].position() * local;
            let error = feet(&player).distance(expected);
            max_roof_error = max_roof_error.max(error);
            assert!(
                player.character.grounded && error < skin,
                "{kind:?}, tick {tick}, roof contact error {error:.4} m"
            );
            assert!(
                player.ground_speed < 0.06,
                "{kind:?}, tick {tick}, stationary rider walked at {} m/s, feet {:?}, local error {error:.4}, support {:?}",
                player.ground_speed,
                feet(&player),
                player.character.state().support_velocity
            );
        }
        let start = feet(&player);
        let velocity = world.physics.bodies[vehicle].velocity_at_point(start);
        player.fixed_step(
            &mut world,
            &StepInput {
                jump: true,
                ..Default::default()
            },
        );
        for _ in 1..12 {
            player.fixed_step(&mut world, &StepInput::default());
        }
        let travel = feet(&player) - start;
        let ticks = 12.;
        let error = travel - velocity * (ticks * dt);
        let height = ticks * dt * tuning::speed::JUMP_MPS as f32
            - (ticks * (ticks + 1.) / 2.) * tuning::GRAVITY_MPS2 as f32 * dt * dt;
        println!("{kind:?}: peak foot drift {max_roof_error:.4} m over 90 ticks, horizontal jump error {:.4} m, vertical jump error {:.4} m",
            error.x.hypot(error.z), (error.y - height).abs());
        assert!(
            !player.character.grounded && (error.y - height).abs() < skin,
            "{kind:?}, jump rose {:.3} m relative to the departing roof",
            error.y
        );
        assert!(
            error.x.hypot(error.z) < skin,
            "{kind:?}, jumped {travel:?} with roof velocity {velocity:?}"
        );
    }
}

#[test]
fn walking_off_a_moving_roof_carries_momentum_into_the_fall() {
    use eo_player::world::layer;
    use rapier3d::prelude::*;
    let mut world = garage_with_floor(vec![]);
    let (body, _) = world.physics.insert(
        RigidBodyBuilder::dynamic()
            .gravity_scale(0.)
            .translation(Vector::Y * 3.),
        ColliderBuilder::cuboid(1., 0.4, 1.8)
            .mass(1400.)
            .collision_groups(InteractionGroups::new(
                layer::VEHICLE,
                Group::ALL,
                InteractionTestMode::And,
            )),
    );
    let mut player = Player::new(DVec3::new(0., 3.4, 0.), 0.);
    for _ in 0..10 {
        player.fixed_step(&mut world, &StepInput::default());
    }
    let lateral_speed = 1.5;
    world.physics.bodies[body].set_linvel(Vector::X * lateral_speed, true);
    let walking = StepInput {
        move_y: 1.,
        ..Default::default()
    };
    let mut in_flight = 0;
    for _ in 0..180 {
        let before = player.character.feet();
        player.fixed_step(&mut world, &walking);
        let after = player.character.feet();
        if after.y < 3.0 && after.y > 0.5 {
            let speed = (after.x - before.x) / tuning::FIXED_STEP_S;
            assert!(
                (speed - f64::from(lateral_speed)).abs() < 0.03,
                "airborne lateral speed {speed:.3} m/s"
            );
            in_flight += 1;
        }
    }
    assert!(in_flight > 20 && player.character.grounded && player.character.feet().y.abs() < 0.02);
}

#[test]
fn a_standing_rider_tracks_the_sprung_chassis_during_a_gentle_turn() {
    use eo_player::vehicle::{VehicleConfig, VehicleModel};
    use eo_player::world::layer;
    use rapier3d::prelude::Vector;
    for (model, cargo_kg) in [(VehicleModel::Sedan, 0), (VehicleModel::DeliveryVan, 600)] {
        let mut world = garage_with_floor(vec![]);
        let mut driver = Player::new(DVec3::ZERO, 0.);
        driver.spawn_config = VehicleConfig {
            model,
            cargo_kg,
            occupants_kg: 80,
        };
        let index = driver.spawn_vehicle(&mut world, 0., 0., 0.);
        driver.enter_vehicle(index);
        for _ in 0..180 {
            driver.fixed_step(&mut world, &StepInput::default());
        }
        let chassis = driver.vehicles[index].body;
        let roof = world
            .raycast(
                DVec3::new(0., 6., 0.),
                DVec3::NEG_Y,
                6.,
                layer::VEHICLE,
                None,
            )
            .expect("chassis roof")
            .point;
        let mut rider = Character::new(roof);
        for _ in 0..10 {
            let (_, floor) = rider.step_before_physics(&world, 0., 0., true);
            driver.fixed_step(&mut world, &StepInput::default());
            if let Some(floor) = floor {
                floor.finish(&mut rider, &mut world);
            }
        }
        let local = world.physics.bodies[chassis].position().inverse()
            * Vector::from_array(rider.feet().as_vec3().to_array());
        let mut max_error = 0.0_f32;
        for tick in 0..240 {
            let (_, floor) = rider.step_before_physics(&world, 0., 0., true);
            driver.fixed_step(
                &mut world,
                &StepInput {
                    throttle: 0.35,
                    steer: if tick >= 90 { 0.15 } else { 0. },
                    analog_steer: true,
                    ..Default::default()
                },
            );
            if let Some(floor) = floor {
                floor.finish(&mut rider, &mut world);
            }
            let expected = world.physics.bodies[chassis].position() * local;
            let error = Vector::from_array(rider.feet().as_vec3().to_array()).distance(expected);
            max_error = max_error.max(error);
            assert!(
                error < tuning::capsule::CONTROLLER_OFFSET_M as f32 && rider.grounded,
                "{model:?}, tick {tick}, foot drift {error:.4} m"
            );
            assert_eq!(driver.vehicles[index].wheels_in_contact(), 4);
        }
        assert!(driver.vehicles[index].datum(&world).length() > 4.);
        assert!(driver.vehicles[index].yaw(&world).abs() > 0.05);
        println!(
            "{model:?} with {cargo_kg} kg cargo: peak foot drift {max_error:.4} m over 240 powered ticks"
        );
    }
}

#[test]
fn a_vehicle_cannot_carry_a_standing_rider_through_a_wall() {
    use eo_player::world::layer;
    use rapier3d::prelude::*;
    let mut world = garage_with_floor(vec![]);
    let (body, _) = world.physics.insert(
        RigidBodyBuilder::kinematic_velocity_based().translation(Vector::Y * 2.),
        ColliderBuilder::cuboid(3., 0.5, 1.).collision_groups(InteractionGroups::new(
            layer::VEHICLE,
            Group::ALL,
            InteractionTestMode::And,
        )),
    );
    world.physics.insert_collider(
        ColliderBuilder::cuboid(0.05, 1., 2.)
            .translation(Vector::new(1.5, 3.6, 0.))
            .build(),
        None,
    );
    let mut player = Player::new(DVec3::new(0., 2.5, 0.), 0.);
    for _ in 0..10 {
        player.fixed_step(&mut world, &StepInput::default());
    }
    world.physics.bodies[body].set_linvel(Vector::X * 2., true);
    for _ in 0..90 {
        player.fixed_step(&mut world, &StepInput::default());
    }
    let feet = player.character.feet();
    let limit = 1.5 - 0.05 - tuning::capsule::RADIUS_M;
    assert!(
        (limit - 0.04..=limit + 0.001).contains(&feet.x),
        "rider at {feet:?}, wall at {limit:.3}"
    );
    assert!(player.character.grounded && (feet.y - 2.5).abs() < 0.02);
}

#[test]
fn a_fast_moving_roof_and_a_departing_rider_prepare_the_next_city_tile() {
    use eo_player::world::{TILE_SIZE_M, layer};
    use rapier3d::prelude::*;
    let boundary = 2. * TILE_SIZE_M;
    let start = DVec3::new(boundary - 0.6, 2.5, 5.);
    for airborne in [false, true] {
        let mut world = CollisionWorld::new(Arc::new(Flat), Arc::new(BuildingSet::empty()), 1);
        world.load_blocking(DVec3::new(boundary - TILE_SIZE_M * 0.5, 0., start.z), 0.);
        let mut player = Player::new(start, 0.);
        if airborne {
            let mut state = player.character.state();
            state.grounded = false;
            state.support_velocity = [40., 0., 0.];
            player.character.set_state(&state);
        } else {
            let (body, _) = world.physics.insert(
                RigidBodyBuilder::kinematic_velocity_based().translation(Vector::new(
                    start.x as f32,
                    2.,
                    start.z as f32,
                )),
                ColliderBuilder::cuboid(3., 0.5, 1.).collision_groups(InteractionGroups::new(
                    layer::VEHICLE,
                    Group::ALL,
                    InteractionTestMode::And,
                )),
            );
            for _ in 0..10 {
                player.fixed_step(&mut world, &StepInput::default());
            }
            assert!(player.character.grounded);
            world.physics.bodies[body].set_linvel(Vector::X * 40., true);
        }
        assert!(!world.is_resident(boundary + 0.1, start.z));
        player.fixed_step(&mut world, &StepInput::default());
        assert!(player.character.feet().x > boundary, "airborne={airborne}");
        assert!(
            world.is_resident(boundary + 0.1, start.z),
            "airborne={airborne}, rider crossed into an unloaded floor tile"
        );
    }
}

fn garage_with_floor(walk: Vec<[[f64; 3]; 3]>) -> CollisionWorld {
    let decks = Decks {
        parts: vec![DeckPart {
            name: "upper floor".into(),
            walk,
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut world = CollisionWorld::new(
        Arc::new(Flat),
        Arc::new(BuildingSet::empty().with_decks(decks)),
        1,
    );
    world.load_blocking(DVec3::ZERO, 64.);
    world
}

#[test]
fn an_idle_stance_settles_on_a_stair_nose_without_pulling_a_walker_across_a_drop() {
    let half_floor = |y| {
        let p = [[0., y, -2.], [0., y, 2.], [3., y, 2.], [3., y, -2.]];
        vec![[p[0], p[1], p[2]], [p[0], p[2], p[3]]]
    };
    for lower_tread in [true, false] {
        let mut walk = half_floor(7.18);
        if lower_tread { walk.extend(platform(7.)); }
        let world = garage_with_floor(walk);
        let start = DVec3::new(-0.045, 7.68, 0.5);
        let mut body = Character::new(start);
        for _ in 0..120 {
            body.step(&world, 0., 0., true);
        }
        let feet = body.feet();
        if lower_tread {
            let under = world.raycast(feet + DVec3::Y * 0.03, DVec3::NEG_Y, 0.06,
                eo_player::world::layer::STREET, None)
                .unwrap_or_else(|| panic!("stance at {feet:?}, grounded {}, speed {}, lacks support under its centre", body.grounded, body.vertical_velocity));
            assert!(body.grounded && (feet.y - under.point.y).abs() < 0.025);
            assert!((feet.x - start.x).abs() < tuning::capsule::RADIUS_M);
        } else {
            assert!(feet.x < 0., "a ledge drew the walker onto it from {start:?} to {feet:?}");
        }
    }
}

#[test]
fn a_car_spawned_beside_a_walker_stays_on_their_storey() {
    for feet_y in [0., 2.6] {
        let mut w = garage(2.6);
        let mut p = Player::new(DVec3::new(2., feet_y, 0.), 0.);
        let i = p.spawn_vehicle(&mut w, 0., 0., 0.);
        let y = p.vehicles[i].datum(&w).y;
        assert!(
            (y - feet_y).abs() < 0.03,
            "walker on {feet_y} m floor spawned a car at {y:.3} m"
        );
    }
}

#[test]
fn entering_and_leaving_a_parked_car_keeps_a_player_on_its_storey() {
    for road in [0.0, 2.6] {
        let mut w = garage(2.6);
        let mut p = Player::new(DVec3::new(2., road, 0.), 0.);
        let i = p.spawn_vehicle_below(&mut w, DVec3::new(0., road + 0.4, 0.), 0.);
        for _ in 0..30 {
            p.fixed_step(&mut w, &StepInput::default());
        }
        let initial = p.character.feet();
        let toggle = StepInput {
            vehicle_toggle: true,
            ..Default::default()
        };
        p.fixed_step(&mut w, &toggle);
        assert_eq!(p.mode, Mode::Driving(i));
        p.fixed_step(&mut w, &StepInput::default());
        p.fixed_step(&mut w, &toggle);
        assert_eq!(p.mode, Mode::OnFoot);
        assert!(
            (p.character.feet().y - initial.y).abs() < 0.03,
            "exit moved walker from {initial:?} to {:?}",
            p.character.feet()
        );
    }
}

#[test]
fn a_floor_and_a_wall_between_a_walker_and_a_car_block_entry() {
    let mut w = garage(2.6);
    let mut p = Player::new(DVec3::new(1.7, 2.6, 0.), 0.);
    p.spawn_vehicle_below(&mut w, DVec3::new(0., 0.4, 0.), 0.);
    assert!(
        !p.try_enter(&w),
        "entered the lower car through the upper floor"
    );
    p.character.teleport_feet(DVec3::new(1.7, 0., 0.));
    assert!(p.nearby_vehicle(&w).is_some());
    use rapier3d::prelude::*;
    w.physics.insert_collider(
        ColliderBuilder::cuboid(0.04, 1.4, 4.)
            .translation(Vector::new(1.25, 1.4, 0.))
            .collision_groups(InteractionGroups::new(
                eo_player::world::layer::WORLD,
                Group::ALL,
                InteractionTestMode::And,
            ))
            .build(),
        None,
    );
    w.physics.detect_collisions(&(), &());
    assert!(!p.try_enter(&w), "entered through the wall");
    assert!(!p.finish_entry(&w, 0), "animated entry crossed the wall");
    p.enter_vehicle(0);
    assert!(p.try_exit(&w));
    assert!(
        p.character.feet().x < 0.,
        "exit crossed the driver's-side wall: {:?}",
        p.character.feet()
    );
}

#[test]
fn a_jump_into_a_ceiling_loses_upward_speed_at_contact() {
    let roof = 1.98;
    let world = garage(roof);
    let mut c = Character::new(DVec3::ZERO);
    for _ in 0..8 {
        c.step(&world, 0., 0., true);
    }
    assert!(c.jump(tuning::speed::JUMP_MPS));
    for tick in 0..24 {
        let expected = c.vertical_velocity - tuning::GRAVITY_MPS2 * tuning::FIXED_STEP_S;
        let movement = c.step(&world, 0., 0., true).moved.y;
        let head = c.position.y + c.half_extent_y();
        assert!(
            head <= roof + 0.001,
            "head penetrated ceiling by {:.4}",
            head - roof
        );
        if expected > 0. && movement + 0.0001 < expected * tuning::FIXED_STEP_S {
            assert!(
                c.vertical_velocity <= 0.001,
                "head hit on tick {tick} but upward speed remained {:.3} m/s",
                c.vertical_velocity
            );
            assert!(c.step(&world, 0., 0., true).moved.y < 0.0);
            return;
        }
    }
    panic!("the ascending capsule never met the underside");
}

#[test]
fn a_grade_is_reachable_and_a_drop_at_the_deck_edge_uses_the_other_door() {
    struct Grade;
    impl GroundProvider for Grade {
        fn ground_y(&self, x: f64, _: f64) -> f64 {
            0.36 * x
        }
    }
    // One inclined upper slab, clear of the 36% road for the whole route.
    let mut deck = platform(2.6);
    for point in deck.iter_mut().flatten() {
        point[1] += 0.36 * point[0];
    }
    let decks = Decks {
        parts: vec![DeckPart {
            name: "inclined floor".into(),
            walk: deck,
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut w = CollisionWorld::new(
        Arc::new(Grade),
        Arc::new(BuildingSet::empty().with_decks(decks)),
        1,
    );
    w.load_blocking(DVec3::ZERO, 64.);
    for floor in [0., 2.6] {
        let x = -6.;
        let mut player = Player::new(DVec3::new(x, 0.36 * x + floor, 0.), 0.);
        let i = player.spawn_vehicle(&mut w, 6., 0., 0.);
        let datum = player.vehicles[i].datum(&w);
        assert!(
            (datum.y - (2.16 + floor)).abs() < 0.02,
            "lost the selected grade at {datum:?}"
        );
        player.enter_vehicle(i);
        assert!(player.try_exit(&w), "blocked on a walkable incline");
        let feet = player.character.feet();
        assert!(
            (feet.y - (feet.x * 0.36 + floor)).abs() < 0.06,
            "wrong sloped floor: {feet:?}"
        );
    }
    let mut w = garage(2.6);
    let mut player = Player::new(DVec3::new(7.3, 2.6, 0.), 0.);
    let i = player.spawn_vehicle(&mut w, 7.3, 0., 0.);
    player.enter_vehicle(i);
    assert!(
        player
            .vehicle_footing(&w, i, DVec3::new(9., 0., 0.))
            .is_none(),
        "exit onto distant ground"
    );
    assert!(player.try_exit(&w));
    assert!(player.character.feet().x < 7.3 && player.character.feet().y > 2.5);
}

#[test]
fn a_low_step_is_a_reachable_door_station() {
    let mut walk = platform(0.16);
    for point in walk.iter_mut().flatten() {
        point[0] = 1.5 + (point[0] + 8.) / 4.;
    }
    let decks = Decks {
        parts: vec![DeckPart {
            name: "raised paving".into(),
            walk,
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut w = CollisionWorld::new(
        Arc::new(Flat),
        Arc::new(BuildingSet::empty().with_decks(decks)),
        1,
    );
    w.load_blocking(DVec3::ZERO, 64.);
    let mut player = Player::new(DVec3::ZERO, 0.);
    let car = player.spawn_vehicle(&mut w, 0., 0., 0.);
    let feet = player
        .vehicle_footing(&w, car, DVec3::new(2., 0., 0.))
        .expect("kerbside door");
    assert!((feet.y - 0.16).abs() < 0.002);
}

#[test]
fn a_jump_along_a_wall_keeps_its_upward_momentum() {
    use rapier3d::prelude::*;
    let mut w = garage(5.);
    w.physics.insert_collider(
        ColliderBuilder::cuboid(0.1, 2.5, 3.)
            .translation(Vector::new(0.5, 2.5, 0.))
            .build(),
        None,
    );
    w.physics.detect_collisions(&(), &());
    let mut walker = Character::new(DVec3::ZERO);
    for _ in 0..4 {
        walker.step(&w, 0., 0., true);
    }
    assert!(walker.jump(tuning::speed::JUMP_MPS));
    for _ in 0..10 {
        let predicted = walker.vertical_velocity - tuning::GRAVITY_MPS2 * tuning::FIXED_STEP_S;
        walker.step(&w, 0.05, 0.02, true);
        assert!((walker.vertical_velocity - predicted).abs() < 0.002);
    }
    assert!(walker.feet().x < 0.06 && walker.feet().z > 0.15);
}

#[test]
fn a_moving_ceiling_limits_a_jump_to_its_contact_velocity() {
    use rapier3d::prelude::*;
    for speed in [-0.25_f32, 0.25] {
        let mut w = garage(5.);
        w.physics.insert(
            RigidBodyBuilder::kinematic_velocity_based()
                .translation(Vector::new(0., 2.04, 0.))
                .linvel(Vector::Y * speed),
            ColliderBuilder::cuboid(3., 0.06, 3.),
        );
        w.physics.detect_collisions(&(), &());
        let mut walker = Character::new(DVec3::ZERO);
        for _ in 0..4 {
            walker.step(&w, 0., 0., true);
        }
        assert!(walker.jump(tuning::speed::JUMP_MPS));
        let mut hit = false;
        for _ in 0..16 {
            let predicted = walker.vertical_velocity - tuning::GRAVITY_MPS2 * tuning::FIXED_STEP_S;
            w.physics.step();
            walker.step(&w, 0., 0., true);
            if walker.vertical_velocity < predicted - 0.01 {
                assert!(
                    (walker.vertical_velocity - f64::from(speed)).abs() < 0.002,
                    "head impact at {} m/s under a {speed} m/s ceiling",
                    walker.vertical_velocity
                );
                hit = true;
                break;
            }
        }
        assert!(hit, "the jump never hit the moving ceiling");
    }
}

#[test]
fn occupied_door_stations_keep_the_driver_seated_until_someone_moves() {
    use eo_player::people_contact::PeopleBodies;
    let mut world = garage(2.6);
    let mut player = Player::new(DVec3::ZERO, 0.);
    let car = player.spawn_vehicle(&mut world, 0., 0., 0.);
    player.enter_vehicle(car);
    let spec = player.vehicles[car].spec;
    let side = (spec.width / 2.0 + tuning::DOOR_STATION_OUTSET_M) as f32;
    let back = -(spec.length / 2.0 + 0.8) as f32;
    let mut people = PeopleBodies::default();
    let mut stations = [
        (0, [side, 0., 0.], 1.),
        (1, [-side, 0., 0.], 1.),
        (2, [0., 0., back], 1.),
    ];
    people.sync(&mut world, stations);
    world.physics.detect_collisions(&(), &());
    assert!(!player.try_exit(&world));
    assert_eq!(player.mode, Mode::Driving(car));
    // The driver can use the same horizontal spot as a walker on another storey.
    stations[0].1[1] = 2.6;
    people.sync(&mut world, stations);
    world.physics.detect_collisions(&(), &());
    assert!(player.try_exit(&world));
    assert!((player.character.feet() - DVec3::new(f64::from(side), 0., 0.)).length() < 0.02);
}

/// Put both ends of a route on real deck triangles. Exercise the body motor
/// near a parked sedan and return the position after crossing and turning back.
fn walk_between_decks(gap: f64, step: f64, grade: f64) -> ([f32; 3], [f32; 3], f32) {
    use eo_player::people_contact::{WalkerTarget, WalkingBodies};
    let mut floor = platform(4.);
    floor.extend(
        platform(4.)
            .into_iter()
            .map(|tri| tri.map(|[x, y, z]| [x, y + step + grade * (z + 8.), z + 16. + gap])),
    );
    let mut world = garage_with_floor(floor);
    let mut player = Player::new(DVec3::new(2., 4., 6.), 0.);
    let i = player.spawn_vehicle_below(&mut world, DVec3::new(0., 4.4, 6.), 0.);
    player.enter_vehicle(i);
    let brake = StepInput {
        handbrake: true,
        ..Default::default()
    };
    for _ in 0..120 {
        player.fixed_step(&mut world, &brake);
    }
    let mut target = WalkerTarget {
        id: 41,
        model: 0,
        feet: [2., 4., 6.5],
        drawn: [2., 4., 6.5],
        scale: 1.,
        heading: 0.,
        phase: 0.,
        stride_m: 1.4,
        speed: 0.,
    };
    let mut lowest = 4_f32;
    let mut run = |target: WalkerTarget, steps| {
        let mut feet = target.drawn;
        for _ in 0..steps {
            WalkingBodies::sync(&mut world, [target]);
            player.fixed_step(&mut world, &brake);
            feet = world
                .walking_bodies
                .poses(&world.physics)
                .next()
                .unwrap()
                .1
                .feet;
            lowest = lowest.min(feet[1]);
        }
        feet
    };
    run(target, 60);
    target.feet = [2., (4. + step + grade * (3.5 - gap)) as f32, 11.5];
    target.drawn = target.feet;
    let outward = run(target, 300);
    target.feet = [2., 4., 6.];
    target.drawn = target.feet;
    let returned = run(target, 300);
    (outward, returned, lowest)
}

#[test]
fn a_walking_body_brakes_at_a_gap_and_can_turn_back() {
    let (edge, returned, lowest) = walk_between_decks(2., 0., 0.);
    eprintln!("walker at deck gap {edge:?}, returned {returned:?}, lowest feet {lowest:.3} m");
    assert!(
        lowest > 3.94 && edge[2] > 6.8 && edge[2] < 8.,
        "walker left the supported deck"
    );
    assert!(
        (returned[2] - 6.).abs() < 0.15,
        "walker could not leave the edge"
    );
}

#[test]
fn a_walking_body_crosses_a_continuous_grade_and_a_low_step() {
    for (step, grade) in [(0., 0.16), (-0.12, 0.)] {
        let (outward, back, lowest) = walk_between_decks(0., step, grade);
        eprintln!("walked step {step}, grade {grade}, outward {outward:?}, returned {back:?}");
        assert!(
            (outward[2] - 11.5).abs() < 0.2
                && (outward[1] - (4. + step + grade * 3.5) as f32).abs() < 0.1
        );
        assert!(lowest > (4. + step - 0.08) as f32 && (back[2] - 6.).abs() < 0.2);
    }
}
