//! Integration checks against the load carried by the four actual wheel rays.
use eo_player::tuning::GRAVITY_MPS2;
use eo_player::vehicle::{VehicleConfig, VehicleModel};
use eo_player::world::{SurfaceMesh, TileKey};
use eo_player::{BuildingSet, CollisionWorld, GroundProvider, Player, StepInput, WeatherPhysics};
use glam::DVec3;
use std::sync::Arc;

struct Road(f64);
impl GroundProvider for Road {
    fn ground_y(&self, _: f64, z: f64) -> f64 {
        self.0 * z
    }
}

fn car(model: VehicleModel, cargo: u16, slope: f64) -> (Player, CollisionWorld) {
    let mut world = CollisionWorld::new(Arc::new(Road(slope)), Arc::new(BuildingSet::empty()), 1);
    let start = DVec3::new(8.0, slope * 120.0, 120.0);
    world.load_blocking(start, 180.0);
    let mut player = Player::new(start, 0.0);
    player.spawn_config = VehicleConfig {
        model,
        cargo_kg: cargo,
        occupants_kg: 0,
    }
    .validated()
    .unwrap();
    let i = player.spawn_vehicle(&mut world, start.x, start.z, 0.0);
    player.enter_vehicle(i);
    for _ in 0..240 {
        player.fixed_step(
            &mut world,
            &StepInput {
                handbrake: true,
                ..Default::default()
            },
        );
    }
    (player, world)
}

#[test]
fn payload_changes_axle_load_ride_height_and_acceleration_with_the_same_engine() {
    let run = |cargo| {
        let (mut p, mut w) = car(VehicleModel::DeliveryVan, cargo, 0.0);
        let v = &p.vehicles[0];
        let loads: Vec<_> = v
            .controller
            .wheels()
            .iter()
            .map(|w| f64::from(w.wheel_suspension_force))
            .collect();
        let force: f64 = loads.iter().sum();
        let expected = v.spec.mass_kg * GRAVITY_MPS2;
        assert!(
            (force - expected).abs() < 0.015 * expected,
            "springs {force} N, weight {expected} N"
        );
        let front = (v.spec.com_offset_z - v.spec.rear_axle_z) / v.spec.wheelbase;
        assert!(((loads[0] + loads[1]) / force - front).abs() < 0.02);
        let z = v.datum(&w).y;
        let rear = loads[2] + loads[3];
        for _ in 0..120 {
            p.fixed_step(
                &mut w,
                &StepInput {
                    throttle: 0.35,
                    ..Default::default()
                },
            );
        }
        (z, rear, p.vehicles[0].forward_speed(&w))
    };
    let empty = run(0);
    let laden = run(600);
    eprintln!(
        "van empty/loaded (ride datum m, rear axle N, speed after 2s m/s) {empty:?} / {laden:?}"
    );
    assert!(laden.0 < empty.0 - 0.015);
    assert!(laden.1 > empty.1 + 0.8 * 600.0 * GRAVITY_MPS2);
    assert!(empty.2 > 1.0 && laden.2 < empty.2 * 0.90);
}

#[test]
fn flat_bend_and_flush_tile_seam_do_not_launch_the_fleet() {
    for model in [
        VehicleModel::Sedan,
        VehicleModel::Hatchback,
        VehicleModel::DeliveryVan,
    ] {
        let (mut p, mut w) = car(model, 0, 0.0);
        let mut crossed = false;
        let mut turn = false;
        for tick in 0..360 {
            let input = StepInput {
                throttle: 0.65,
                steer: if tick > 160 { 0.30 } else { 0.0 },
                analog_steer: true,
                ..Default::default()
            };
            p.fixed_step(&mut w, &input);
            let v = &p.vehicles[0];
            let at = v.datum(&w);
            crossed |= at.z > eo_player::world::TILE_SIZE_M;
            turn |= v.yaw(&w).abs() > 0.4;
            assert!(
                w.physics.bodies[v.body].linvel().y.abs() < 0.6
                    && at.y.abs() < 0.15
                    && v.wheels_in_contact() >= 2,
                "{model:?}, tick {tick}, datum {at:?}, velocity {:?}",
                w.physics.bodies[v.body].linvel()
            );
        }
        assert!(
            crossed && turn,
            "{model:?} did not drive both parts of the route"
        );
    }
}

#[test]
fn a_parked_van_holds_a_grade_and_wet_braking_spends_more_distance() {
    let (mut p, mut w) = car(VehicleModel::DeliveryVan, 300, 0.12);
    let before = p.position(&w);
    for _ in 0..600 {
        p.fixed_step(
            &mut w,
            &StepInput {
                handbrake: true,
                ..Default::default()
            },
        );
    }
    let drift = before.distance(p.position(&w));
    assert!(drift < 0.10, "parked drift {drift:.3} m");
    let stop = |wetness| {
        let (mut p, mut w) = car(VehicleModel::Sedan, 0, 0.0);
        let handle = p.vehicles[0].body;
        w.physics.bodies[handle].set_linvel([0.0, 0.0, 15.0].into(), true);
        p.set_weather(WeatherPhysics {
            wetness,
            ..Default::default()
        });
        let from = p.position(&w);
        for _ in 0..600 {
            p.fixed_step(
                &mut w,
                &StepInput {
                    brake: 1.0,
                    ..Default::default()
                },
            );
            if p.vehicles[0].forward_speed(&w) < 0.3 {
                return p.position(&w).z - from.z;
            }
        }
        panic!("failed to stop");
    };
    let (dry, wet) = (stop(0.0), stop(1.0));
    eprintln!("15 m/s stop: dry {dry:.2} m, wet {wet:.2} m");
    assert!(dry > 10.0 && wet > dry * 1.10);
}

struct UpperFloor;
impl GroundProvider for UpperFloor {
    fn ground_y(&self, _: f64, _: f64) -> f64 {
        -8.0
    }
    fn multi_level(&self) -> bool {
        true
    }
    fn surface_tile(&self, key: TileKey) -> Option<SurfaceMesh> {
        let (x, z) = key.origin();
        let t = eo_player::world::TILE_SIZE_M;
        let (a, b, c, d) = (
            [x, 5.0, z],
            [x, 5.0, z + t],
            [x + t, 5.0, z + t],
            [x + t, 5.0, z],
        );
        Some(SurfaceMesh {
            street: vec![[a, b, c], [a, c, d]],
            ..Default::default()
        })
    }
}

#[test]
fn first_step_after_travel_waits_for_the_real_upper_floor() {
    let mut w = CollisionWorld::new(Arc::new(UpperFloor), Arc::new(BuildingSet::empty()), 1);
    let at = DVec3::new(1200.0, 5.0, -501.0);
    let mut p = Player::new(at, 0.0);
    assert!(!w.is_resident(at.x, at.z));
    for _ in 0..120 {
        p.fixed_step(&mut w, &StepInput::default());
        assert!(p.character.feet().y > 4.98);
    }
    assert!(p.character.grounded && w.is_resident(at.x, at.z));
    assert!((w.surface_below(at.x, at.z, at.y + 0.1) - at.y).abs() < 0.01);
}

#[test]
fn parked_car_keeps_its_pose_while_its_streamed_floor_is_away() {
    let (mut player, mut world) = car(VehicleModel::Sedan, 0, 0.0);
    let car = player.vehicles[0].body;
    let home = player.position(&world);
    player.mode = eo_player::Mode::OnFoot;
    let away = home + DVec3::X * 1000.0;
    player.character.teleport_feet(away);
    for _ in 0..240 {
        world.pump(away);
        player.fixed_step(&mut world, &StepInput::default());
    }
    assert!(!world.is_resident(home.x, home.z) && !world.physics.bodies[car].is_enabled());
    let stored = world.physics.bodies[car].translation();
    for _ in 0..60 {
        player.fixed_step(&mut world, &StepInput::default());
    }
    assert_eq!(world.physics.bodies[car].translation(), stored);
    player.character.teleport_feet(home + DVec3::X * 3.0);
    for _ in 0..120 {
        player.fixed_step(&mut world, &StepInput::default());
    }
    let back = player.vehicles[0].datum(&world);
    assert!(
        world.physics.bodies[car].is_enabled() && back.distance(home) < 0.08,
        "{home:?} -> {back:?}"
    );
}

#[test]
fn an_unsupported_car_falls_under_gravity_and_lands_on_its_springs() {
    let (mut p, mut w) = car(VehicleModel::Hatchback, 0, 0.0);
    let h = p.vehicles[0].body;
    let translation = w.physics.bodies[h].translation() + rapier3d::prelude::Vector::Y * 2.0;
    w.physics.bodies[h].set_translation(translation, true);
    w.physics.bodies[h].set_linvel([0.0, 0.0, 5.0].into(), true);
    let mut airborne = 0;
    let mut lowest = f64::INFINITY;
    for _ in 0..240 {
        let vy = w.physics.bodies[h].linvel().y;
        p.fixed_step(&mut w, &StepInput::default());
        if p.vehicles[0].wheels_in_contact() == 0 && p.position(&w).y > 0.5 {
            let acceleration =
                f64::from(w.physics.bodies[h].linvel().y - vy) / eo_player::tuning::FIXED_STEP_S;
            assert!(
                (acceleration + GRAVITY_MPS2).abs() < 0.08,
                "free fall {acceleration} m/s²"
            );
            airborne += 1;
        }
        lowest = lowest.min(p.position(&w).y);
    }
    assert!(airborne >= 25 && p.vehicles[0].wheels_in_contact() == 4);
    assert!(
        lowest > -p.vehicles[0].spec.sill_y - 0.02 && p.position(&w).y.abs() < 0.04,
        "lowest datum {lowest}"
    );
}

#[test]
fn a_local_traffic_car_receives_the_players_collision_impulse() {
    traffic_impact(None);
}

#[test]
fn a_parked_kerbside_car_keeps_its_displaced_pose_after_the_block_reloads() {
    traffic_impact(Some(VehicleModel::Sedan));
}

fn traffic_impact(parked: Option<VehicleModel>) {
    use eo_player::traffic::{TrafficBodies, TrafficChassis, TrafficPose};
    use eo_player::traffic_contact::{ContactTraffic, TrafficTarget};
    let (mut p, mut w) = car(VehicleModel::Sedan, 0, 0.0);
    let at = p.position(&w);
    let target = at + DVec3::Z * 8.0;
    let mut scripted = TrafficBodies::default();
    let h = p.vehicles[0].body;
    w.physics.bodies[h].set_linvel([0.0, 0.0, 8.0].into(), true);
    let mut max_traffic_speed = 0.0_f32;
    let mut final_traffic = target;
    for _ in 0..120 {
        let near = p.position(&w);
        let speed = p.vehicles[0].forward_speed(&w);
        let traffic = TrafficTarget {
            speed: 0.0,
            model: eo_player::traffic_contact::TrafficModel::Road(parked.unwrap_or(VehicleModel::Sedan)),
            parked: parked.is_some(),
            pose: TrafficPose {
                id: 17,
                datum: target.as_vec3().to_array(),
                yaw: 0.0,
                pitch: 0.0,
                chassis: TrafficChassis::Class { width: 1.84, length: 4.85 },
            },
        };
        ContactTraffic::sync(
            &mut w,
            &mut scripted,
            near,
            speed,
            WeatherPhysics::default(),
            [traffic],
        );
        p.fixed_step(&mut w, &StepInput::default());
        if let Some((_, pos, _, v)) = w.contact_traffic.poses(&w).next() {
            max_traffic_speed = max_traffic_speed.max(v);
            final_traffic = DVec3::from(pos.map(f64::from));
        }
    }
    assert!(
        max_traffic_speed > 1.5 && final_traffic.z > target.z + 0.6,
        "struck car at {final_traffic:?}, highest speed {max_traffic_speed}"
    );
    let player = p.position(&w);
    assert!(
        final_traffic.z - player.z > 4.40,
        "chassis overlap {player:?} {final_traffic:?}"
    );
    assert!(final_traffic.y.abs() < 0.15 && player.y.abs() < 0.15);
    if parked.is_some() {
        let before = w.contact_traffic.poses(&w).next().unwrap();
        ContactTraffic::sync(
            &mut w,
            &mut scripted,
            player + 300.0 * DVec3::X,
            0.0,
            Default::default(),
            [],
        );
        let after = w
            .contact_traffic
            .poses(&w)
            .next()
            .expect("displaced pose survives streaming");
        assert_eq!((before.1, before.2), (after.1, after.2));
        let restored = TrafficTarget {
            speed: 0.0,
            model: eo_player::traffic_contact::TrafficModel::Road(parked.unwrap_or(VehicleModel::Sedan)),
            parked: parked.is_some(),
            pose: TrafficPose {
                id: 17,
                datum: target.as_vec3().to_array(),
                yaw: 0.0,
                pitch: 0.0,
                chassis: TrafficChassis::Class { width: 1.84, length: 4.85 },
            },
        };
        ContactTraffic::sync(
            &mut w,
            &mut scripted,
            final_traffic,
            0.0,
            Default::default(),
            [restored],
        );
        let after = w.contact_traffic.poses(&w).next().unwrap();
        assert_eq!((before.1, before.2), (after.1, after.2));
    }
}

#[test]
fn a_car_entering_deep_water_loses_speed_then_floods_and_settles_on_the_bed() {
    struct Basin;
    impl GroundProvider for Basin {
        fn ground_y(&self, _: f64, _: f64) -> f64 {
            -8.0
        }
        fn water_column(&self, _: f64, _: f64) -> Option<(f64, f64)> {
            Some((-8.0, 0.0))
        }
    }
    let mut w = CollisionWorld::new(Arc::new(Basin), Arc::new(BuildingSet::empty()), 1);
    let at = DVec3::new(8.0, 0.0, 8.0);
    w.load_blocking(at, 100.0);
    let mut p = Player::new(at, 0.0);
    p.spawn_config = VehicleConfig {
        model: VehicleModel::Sedan,
        ..Default::default()
    };
    let i = p.spawn_vehicle(&mut w, at.x, at.z, 0.0);
    p.enter_vehicle(i);
    let h = p.vehicles[i].body;
    w.physics.bodies[h].set_translation([8.0, 0.5, 8.0].into(), true);
    w.physics.bodies[h].set_linvel([0.0, 0.0, 7.0].into(), true);
    for _ in 0..240 {
        p.fixed_step(&mut w, &Default::default());
    }
    assert!(p.position(&w).y > -2.0 && p.position(&w).y < 0.1);
    assert!(p.vehicles[i].forward_speed(&w).abs() < 2.0);
    for _ in 0..5400 {
        p.fixed_step(
            &mut w,
            &StepInput {
                throttle: 1.0,
                ..Default::default()
            },
        );
    }
    let car = &p.vehicles[i];
    assert!(
        car.state(&w).flood_ticks > 3000 && car.wheels_in_contact() == 4,
        "settled at {:?}, orientation {:?}, flooded {}, contacts {}",
        car.datum(&w),
        car.pose(&w).1,
        car.state(&w).flood_ticks,
        car.wheels_in_contact()
    );
    assert!((car.datum(&w).y + 8.0).abs() < 0.18 && car.forward_speed(&w).abs() < 0.25);
}

#[test]
fn an_on_foot_player_stops_at_another_person_and_walks_on_when_they_leave() {
    use eo_player::people_contact::PeopleBodies;
    let mut w = CollisionWorld::new(Arc::new(Road(0.0)), Arc::new(BuildingSet::empty()), 1);
    let at = DVec3::new(8.0, 0.0, 8.0);
    w.load_blocking(at, 64.0);
    let mut p = Player::new(at, 0.0);
    let mut people = PeopleBodies::default();
    people.sync(&mut w, [(4, [8.0, 0.0, 11.0], 1.0)]);
    let step = StepInput {
        move_y: 1.0,
        ..Default::default()
    };
    for _ in 0..180 {
        p.fixed_step(&mut w, &step);
    }
    assert!(
        (10.0..10.35).contains(&p.position(&w).z) && p.position(&w).y.abs() < 0.04,
        "{:?}",
        p.position(&w)
    );
    people.sync(&mut w, []);
    for _ in 0..90 {
        p.fixed_step(&mut w, &step);
    }
    assert!(p.position(&w).z > 12.0);
}

#[test]
fn an_impact_between_two_authoritative_worlds_conserves_total_momentum() {
    let (p, mut wa) = car(VehicleModel::Hatchback, 0, 0.0);
    let (q, mut wb) = car(VehicleModel::DeliveryVan, 500, 0.0);
    let (a, b) = (&p.vehicles[0], &q.vehicles[0]);
    let dz = 0.5 * (a.spec.length + b.spec.length) - 0.015;
    let at = wb.physics.bodies[b.body].translation();
    wb.physics.bodies[b.body].set_translation(at + rapier3d::prelude::Vector::Z * dz as f32, true);
    wb.physics.step();
    wa.physics.bodies[a.body].set_linvel([0.0, 0.0, 8.0].into(), true);
    let totals = |w: &CollisionWorld, v: &eo_player::vehicle::Vehicle| {
        let body = &w.physics.bodies[v.body];
        body.linvel() * body.mass()
    };
    let before = totals(&wa, a) + totals(&wb, b);
    assert!(eo_player::vehicle::resolve_peer_contact(
        a, &mut wa, b, &mut wb
    ));
    let after = totals(&wa, a) + totals(&wb, b);
    assert!(
        (after - before).length() < 0.02,
        "momentum {before:?} -> {after:?}"
    );
    assert!(
        wb.physics.bodies[b.body].linvel().z > 1.2 && wa.physics.bodies[a.body].linvel().z < 5.5
    );
    let pos = wb.physics.bodies[b.body].translation();
    wb.physics.bodies[b.body].set_translation(pos + rapier3d::prelude::Vector::Y * 6.0, true);
    wb.physics.step();
    assert!(!eo_player::vehicle::resolve_peer_contact(
        a, &mut wa, b, &mut wb
    ));
}

fn measure_straight_line(model: VehicleModel) -> (f64, f64) {
    let (mut p, mut w) = car(model, 0, 0.0);
    let start = p.position(&w);
    p.spawn_config.occupants_kg = 75;
    let i = p.spawn_vehicle(&mut w, start.x + 8.0, start.z, 0.0);
    p.enter_vehicle(i);
    for _ in 0..180 {
        p.fixed_step(
            &mut w,
            &StepInput {
                handbrake: true,
                ..Default::default()
            },
        );
    }
    let v = |p: &Player, w: &CollisionWorld| p.vehicles[i].forward_speed(w);
    let from = p.position(&w);
    let mut rollout = None;
    let mut sixty = None;
    for tick in 1..=1800 {
        p.fixed_step(
            &mut w,
            &StepInput {
                throttle: 1.0,
                ..Default::default()
            },
        );
        if p.position(&w).z - from.z > 0.3048 {
            rollout.get_or_insert(tick);
        }
        if v(&p, &w) >= 60.0 * 0.44704 {
            sixty.get_or_insert(tick);
        }
        if v(&p, &w) >= 70.0 * 0.44704 {
            break;
        }
    }
    assert!(v(&p, &w) >= 70.0 * 0.44704);
    let standing = f64::from(sixty.unwrap()) / 60.0;
    let rolled = standing - f64::from(rollout.unwrap()) / 60.0;
    let from = p.position(&w);
    for _ in 0..600 {
        p.fixed_step(
            &mut w,
            &StepInput {
                brake: 1.0,
                ..Default::default()
            },
        );
        if v(&p, &w) <= 0.10 {
            break;
        }
    }
    assert!(v(&p, &w) <= 0.10);
    let stop = p.position(&w).z - from.z;
    eprintln!(
        "{model:?}: 0–60 mph {standing:.2} s, after one foot {rolled:.2} s, 70–0 mph {stop:.2} m"
    );
    (standing, stop)
}

#[test]
fn hatchback_acceleration_and_stopping_distance_follow_the_reference_road_test() {
    let (standing, stop) = measure_straight_line(VehicleModel::Hatchback);
    // Cross-model-year envelope for the US 120 hp, five-speed hatchback.
    assert!((standing - 10.1).abs() <= 1.01);
    assert!((stop - 181.0 * 0.3048).abs() < 0.10 * 181.0 * 0.3048);
}

#[test]
fn sedan_acceleration_and_braking_follow_the_four_cylinder_road_tests() {
    let (standing, stop) = measure_straight_line(VehicleModel::Sedan);
    // Earlier four-cylinder references and transmission differences are in the design notes.
    assert!((standing - 8.9).abs() < 0.89, "standing time {standing} s");
    assert!(
        (stop - 180.0 * 0.3048).abs() < 0.10 * 180.0 * 0.3048,
        "stop {stop} m"
    );
}

struct SkidpadRun {
    yaw_g: f64,
    lateral_g: f64,
    peak_radius_error: f64,
    mean_speed: f64,
}

fn skidpad(model: VehicleModel, cargo_kg: u16, target_g: f64, direction: f64, wetness: f64) -> SkidpadRun {
    // A 300 ft diameter skidpad, flat in both directions.
    let radius = 150.0 * 0.3048;
    let (mut p, mut w) = car(model, cargo_kg, 0.0);
    p.set_weather(WeatherPhysics {
        wetness,
        ..Default::default()
    });
    let start = p.position(&w);
    let centre = start + DVec3::X * (radius * direction);
    let v_ref = (target_g * GRAVITY_MPS2 * radius).sqrt();
    let car = &p.vehicles[0];
    w.physics.bodies[car.body].set_linvel([0.0, 0.0, v_ref as f32].into(), true);
    let mut peak_radius_error = 0.0_f64;
    let mut speed_sum = 0.0;
    let mut yaw_gain = 0.0;
    let mut lateral_accel = 0.0;
    for tick in 0..1200 {
        let car = &p.vehicles[0];
        let at = car.datum(&w);
        let radial = at - centre;
        let angle = radial.z.atan2(-direction * radial.x);
        let ahead = angle + 0.28;
        let goal = centre + DVec3::new(-direction * ahead.cos(), 0.0, ahead.sin()) * radius;
        let delta = goal - at;
        let yaw = car.yaw(&w);
        let error = delta.x.atan2(delta.z) - yaw;
        let rack = (2.0 * car.spec.wheelbase * error.sin() / delta.x.hypot(delta.z)).atan();
        let speed = car.forward_speed(&w);
        let body = car.body;
        let before = w.physics.bodies[body].linvel();
        let input = StepInput {
            steer: (rack / car.spec.max_steer_rad).clamp(-1.0, 1.0),
            analog_steer: true,
            throttle: (0.17 + (v_ref - speed) * 0.5).clamp(0.0, 1.0),
            ..Default::default()
        };
        p.fixed_step(&mut w, &input);
        assert!(p.vehicles[0].wheels_in_contact() >= 3);
        if tick >= 240 {
            let after = w.physics.bodies[body].linvel();
            // Horizontal acceleration perpendicular to the actual direction of travel,
            // independent of the direction in which a sliding chassis is pointing.
            let side_delta = f64::from(before.z) * f64::from(after.x - before.x)
                - f64::from(before.x) * f64::from(after.z - before.z);
            lateral_accel += direction * side_delta
                / (f64::from(before.x.hypot(before.z)) * eo_player::tuning::FIXED_STEP_S);
            let d = p.vehicles[0].yaw(&w) - yaw;
            yaw_gain += direction * d.sin().atan2(d.cos());
            peak_radius_error = peak_radius_error.max((radial.x.hypot(radial.z) - radius).abs());
            speed_sum += speed;
        }
    }
    SkidpadRun {
        yaw_g: (speed_sum / 960.0) * (yaw_gain / 16.0) / GRAVITY_MPS2,
        lateral_g: lateral_accel / (960.0 * GRAVITY_MPS2),
        peak_radius_error,
        mean_speed: speed_sum / 960.0,
    }
}

#[test]
fn hatchback_sustains_a_measured_radius_turn_without_lifting_off_the_flat() {
    let run = skidpad(VehicleModel::Hatchback, 0, 0.81, 1.0, 0.0);
    eprintln!(
        "hatchback skidpad: {:.3} g, peak radius error {:.2} m",
        run.yaw_g, run.peak_radius_error
    );
    assert!(run.peak_radius_error < 2.5 && (run.yaw_g - 0.81).abs() < 0.08);
}

#[test]
fn cornering_reaches_a_grounded_grip_limit_in_both_directions() {
    let mut peaks = [[0.0_f64; 2]; 2];
    for (surface, wetness) in [0.0, 1.0].into_iter().enumerate() {
        for (turn, direction) in [-1.0, 1.0].into_iter().enumerate() {
            let mut peak_on_circle = 0.0_f64;
            for target_g in [0.55, 0.65, 0.75, 0.81, 0.85, 0.90, 1.0, 1.15] {
                let r = skidpad(VehicleModel::Hatchback, 0, target_g, direction, wetness);
                println!("skidpad wet {wetness:.0}, direction {direction}, demand {target_g:.2} g, measured {:.3} g, speed {:.2} m/s, max radius error {:.2} m", r.lateral_g, r.mean_speed, r.peak_radius_error);
                peaks[surface][turn] = peaks[surface][turn].max(r.lateral_g);
                if r.peak_radius_error < 2.5 {
                    peak_on_circle = peak_on_circle.max(r.lateral_g);
                }
                if target_g == 0.55 {
                    assert!(r.peak_radius_error < 1.0 && (r.lateral_g - target_g).abs() < 0.03);
                }
                if target_g > 1.0 {
                    assert!(r.peak_radius_error > 5.0, "the car must run wide beyond available grip");
                }
            }
            if surface == 0 {
                // Same reference and tolerance as the single-speed dry test.
                assert!((peak_on_circle - 0.81).abs() < 0.08);
                assert!((peaks[surface][turn] - 0.81).abs() < 0.08);
            } else {
                assert!(peaks[surface][turn] < peaks[0][turn] - 0.05);
            }
        }
        assert!((peaks[surface][0] - peaks[surface][1]).abs() < 0.01);
    }
}

#[test]
fn sedan_cornering_tracks_the_four_cylinder_skidpad_reference() {
    // The earlier 2.5 L reference car reached 0.82 g on a 300 ft circle.
    // Use the same 10% cross-model-year envelope as the braking comparison.
    for direction in [-1.0, 1.0] {
        let mut peak_on_circle = 0.0_f64;
        for demand in [0.65, 0.75, 0.80, 0.82, 0.86, 0.90, 0.95, 1.05] {
            let r = skidpad(VehicleModel::Sedan, 0, demand, direction, 0.0);
            if r.peak_radius_error < 2.5 {
                peak_on_circle = peak_on_circle.max(r.lateral_g);
            }
            if demand == 1.05 {
                assert!(r.peak_radius_error > 5.0, "excess demand must widen the turn");
            }
        }
        eprintln!("sedan, direction {direction}: sustained {peak_on_circle:.3} g within 2.5 m of the circle");
        assert!((peak_on_circle - 0.82).abs() < 0.082);
    }
}

#[test]
fn an_empty_and_loaded_van_track_a_moderate_bend_on_dry_and_wet_pavement() {
    for cargo in [0, 600] {
        for wetness in [0.0, 1.0] {
            for direction in [-1.0, 1.0] {
                let r = skidpad(VehicleModel::DeliveryVan, cargo, 0.40, direction, wetness);
                eprintln!("van {cargo} kg cargo, film {wetness}, direction {direction}: {:.3} g, maximum radius error {:.2} m", r.lateral_g, r.peak_radius_error);
                assert!((r.lateral_g - 0.40).abs() < 0.03 && r.peak_radius_error < 1.0);
            }
        }
    }
}

#[test]
fn the_full_analog_steering_range_reaches_the_rack() {
    use eo_player::input::{ActionMap, GamepadState, RawInput};
    let mut map = ActionMap::default();
    let mut raw = RawInput {
        gamepad: Some(GamepadState::default()),
        ..Default::default()
    };
    for axis in [0.99, 1.0, 0.0, -1.0] {
        raw.gamepad.as_mut().unwrap().axes[0] = axis;
        let step = StepInput::from_frame(&map.resolve(&mut raw, 1.0 / 60.0, false));
        assert!(step.analog_steer);
        if axis.abs() == 1.0 {
            assert_eq!(step.steer, -axis);
        }
    }
    raw.gamepad = None;
    raw.held.insert("KeyA".to_owned());
    let step = StepInput::from_frame(&map.resolve(&mut raw, 1.0 / 60.0, false));
    assert!(!step.analog_steer && step.steer == 1.0);
}

#[test]
fn a_physical_walker_keeps_the_crowds_clearance_beside_a_stopped_sedan() {
    use eo_player::people_contact::{WalkerTarget, WalkingBodies};
    let (mut player, mut world) = car(VehicleModel::Sedan, 0, 0.);
    let brake = StepInput { handbrake: true, ..Default::default() };
    let mut target = WalkerTarget {
        id: (1 << 63) | 7,
        model: 0,
        feet: [8., 0., 120.],
        drawn: [10., 0., 120.],
        scale: 1.,
        heading: -std::f32::consts::FRAC_PI_2,
        phase: 0.,
        stride_m: 1.4,
        speed: 1.4,
    };
    let mut max_drift = 0.0_f32;
    for _ in 0..240 {
        WalkingBodies::sync(&mut world, [target]);
        player.fixed_step(&mut world, &brake);
        let (id, pose) = world.walking_bodies.poses(&world.physics).next().unwrap();
        assert_eq!((id, pose.route_feet), (target.id, target.feet));
        max_drift = max_drift.max((pose.feet[0] - target.drawn[0]).abs());
    }
    println!("physical walker at a blocked route: lateral drift {max_drift:.3} m");
    assert!(max_drift < 0.10, "walker steered back into the sedan");
    // Walk alongside it, then resume the original line once it clears the bonnet.
    target.heading = 0.;
    for _ in 0..240 {
        target.feet[2] += target.speed * eo_player::tuning::FIXED_STEP_S as f32;
        target.drawn[2] = target.feet[2];
        WalkingBodies::sync(&mut world, [target]);
        player.fixed_step(&mut world, &brake);
    }
    target.drawn = target.feet;
    target.speed = 0.;
    for _ in 0..240 {
        WalkingBodies::sync(&mut world, [target]);
        player.fixed_step(&mut world, &brake);
    }
    let (_, pose) = world.walking_bodies.poses(&world.physics).next().unwrap();
    let error = glam::Vec3::from_array(pose.feet).distance(glam::Vec3::from_array(target.feet));
    println!("physical walker rejoined the open route with {error:.3} m error");
    assert!(error < 0.15);
}

#[test]
fn a_sedan_and_a_walker_exchange_momentum_then_the_walker_recovers_on_the_ground() {
    use eo_player::people_contact::{WalkerTarget, WalkingBodies};
    let (mut player, mut world) = car(VehicleModel::Sedan, 0, 0.0);
    let (mut empty_driver, mut empty_world) = car(VehicleModel::Sedan, 0, 0.0);
    for (p, w) in [(&player, &mut world), (&empty_driver, &mut empty_world)] {
        w.physics.bodies[p.vehicles[0].body].set_linvel([0., 0., 8.].into(), true);
    }
    let target = WalkerTarget {
        id: 42,
        model: 1,
        scale: 1.0,
        feet: [8.7, 0.0, 127.0],
        drawn: [8.7, 0.0, 127.0],
        heading: 0.0,
        speed: 0.0,
        phase: 0.25,
        stride_m: 1.45,
    };
    let upstairs = WalkerTarget {
        id: 43,
        feet: [8.7, 6.0, 127.0],
        drawn: [8.7, 6.0, 127.0],
        ..target
    };
    let mut displacement = 0.0_f32;
    let mut speed_loss = 0.0_f64;
    let mut min_up = 1.0_f32;
    let mut min_clearance = f32::MAX;
    for _ in 0..1800 {
        WalkingBodies::sync(&mut world, [target, upstairs]);
        player.fixed_step(&mut world, &StepInput::default());
        empty_driver.fixed_step(&mut empty_world, &StepInput::default());
        assert!(!world.walking_bodies.contains(upstairs.id));
        if let Some((_, pose)) = world.walking_bodies.poses(&world.physics).next() {
            let up = glam::Quat::from_array(pose.rotation) * glam::Vec3::Y;
            let feet = glam::Vec3::from_array(pose.feet);
            let centre = feet + 0.90 * up;
            min_clearance = min_clearance.min(centre.y - 0.35 - 0.55 * up.y.abs());
            min_up = min_up.min(up.y);
            displacement = displacement.max((feet - glam::Vec3::from_array(target.feet)).length());
            speed_loss = speed_loss.max(
                empty_driver.vehicles[0].forward_speed(&empty_world)
                    - player.vehicles[0].forward_speed(&world),
            );
        }
    }
    let (_, pose) = world.walking_bodies.poses(&world.physics).next().unwrap();
    let final_up = (glam::Quat::from_array(pose.rotation) * glam::Vec3::Y).y;
    let final_error =
        glam::Vec3::from_array(pose.feet).distance(glam::Vec3::from_array(target.feet));
    eprintln!(
        "walker moved {displacement:.3} m, car speed loss {speed_loss:.3} m/s, minimum up {min_up:.3}, ground clearance {min_clearance:.3} m, final up {final_up:.3}, route error {final_error:.3} m"
    );
    assert!(displacement > 2.0 && speed_loss > 0.1 && min_up < 0.6);
    assert!(min_clearance > -0.04 && final_up > 0.98 && final_error < 0.25);
    let recycled = WalkerTarget {
        feet: [50., 0., 160.],
        drawn: [50., 0., 160.],
        ..target
    };
    WalkingBodies::sync(&mut world, [recycled]);
    assert!(!world.walking_bodies.contains(recycled.id));
}

#[test]
fn held_service_brakes_stop_and_hold_each_vehicle_in_both_directions() {
    for model in [
        VehicleModel::Hatchback,
        VehicleModel::Sedan,
        VehicleModel::DeliveryVan,
        VehicleModel::CatalogueSedan,
    ] {
        for (grade, speed) in [(0.0, 8.0), (0.0, -5.0), (0.12, 8.0), (0.12, -5.0), (-0.12, 8.0), (-0.12, -5.0)] {
            let (mut p, mut world) = car(model, 0, grade);
            let car = p.vehicles[0].body;
            let tangent = DVec3::new(0.0, grade, 1.0).normalize();
            world.physics.bodies[car]
                .set_linvel((tangent * speed).as_vec3().to_array().into(), true);
            let brake = StepInput {
                brake: 1.0,
                ..Default::default()
            };
            let mut stopped = None;
            let mut max_drift = 0.0_f64;
            let mut powered_ticks = 0;
            let mut settled = None;
            let mut hold_drift = 0.0_f64;
            for tick in 0..3600 {
                p.fixed_step(&mut world, &brake);
                let v = &p.vehicles[0];
                let pos = v.datum(&world);
                if tick == 240 {
                    settled = Some(pos);
                }
                if let Some(at) = settled {
                    hold_drift = hold_drift.max(pos.distance(at));
                }
                if stopped.is_none() && v.forward_speed(&world).abs() < 0.12 {
                    stopped = Some(pos);
                }
                if let Some(at) = stopped {
                    max_drift = max_drift.max(pos.distance(at));
                }
                powered_ticks += usize::from(v.controls.throttle != 0.0);
            }
            println!(
                "{model:?}, grade {grade:+.2}, approach {speed:+.1} m/s: drift {max_drift:.4} m, settled drift {hold_drift:.4} m, powered ticks {powered_ticks}"
            );
            assert!(
                hold_drift < 0.02,
                "{model:?} on {grade}: drifted {hold_drift:.4} m after settling"
            );
            assert!(
                stopped.is_some() && max_drift < 0.10 && powered_ticks == 0,
                "{model:?} on grade {grade}: holding the service brake moved {max_drift:.4} m and commanded power for {powered_ticks} ticks"
            );
        }
    }
}

#[test]
fn drive_range_taps_survive_render_timing_and_only_shift_at_a_stop() {
    use eo_player::input::{ActionMap, GamepadState, RawInput, gamepad};
    let (mut p, mut w) = car(VehicleModel::Sedan, 0, 0.0);
    let mut map = ActionMap::default();
    let mut raw = RawInput {
        direction_pressed: true,
        ..Default::default()
    };
    let tick = eo_player::tuning::FIXED_STEP_S;
    let tap = StepInput::from_frame(&map.resolve(&mut raw, tick / 2.0, false));
    assert_eq!(p.advance(&mut w, &tap, tick / 2.0), 0);
    let empty = StepInput::from_frame(&map.resolve(&mut raw, tick, false));
    assert!(!empty.shift_direction);
    assert_eq!(p.advance(&mut w, &empty, 2.5 * tick), 3);
    assert!(p.vehicles[0].reverse_selected());
    let restored = p.state(&w);
    p.fixed_step(&mut w, &tap);
    assert!(!p.vehicles[0].reverse_selected());
    p.set_state(&mut w, &restored);
    assert!(p.vehicles[0].reverse_selected());

    raw.held.insert("KeyW".into());
    let accelerator = StepInput::from_frame(&map.resolve(&mut raw, tick, false));
    for _ in 0..120 {
        p.fixed_step(&mut w, &accelerator);
    }
    assert!(p.vehicles[0].forward_speed(&w) < -2.0);
    raw.held.insert("KeyR".into());
    let moving_shift = StepInput::from_frame(&map.resolve(&mut raw, tick, false));
    assert!(moving_shift.shift_direction);
    p.fixed_step(&mut w, &moving_shift);
    assert!(p.vehicles[0].reverse_selected());

    raw.held.clear();
    raw.held.insert("KeyS".into());
    let brake = StepInput::from_frame(&map.resolve(&mut raw, tick, false));
    for _ in 0..240 {
        p.fixed_step(&mut w, &brake);
    }
    assert!(p.vehicles[0].forward_speed(&w).abs() < 0.1 && p.vehicles[0].reverse_selected());
    let mut buttons = vec![0.0; gamepad::RT as usize + 1];
    buttons[gamepad::X as usize] = 1.0;
    raw.gamepad = Some(GamepadState {
        buttons,
        ..Default::default()
    });
    let selector = StepInput::from_frame(&map.resolve(&mut raw, tick, false));
    p.fixed_step(&mut w, &selector);
    assert!(!p.vehicles[0].reverse_selected());
    assert!(!StepInput::from_frame(&map.resolve(&mut raw, tick, false)).shift_direction);
}

#[test]
fn releasing_the_brake_on_a_hill_lets_gravity_roll_the_car() {
    let (mut p, mut w) = car(VehicleModel::Sedan, 0, 0.12);
    let brake = StepInput {
        brake: 1.0,
        ..Default::default()
    };
    for _ in 0..180 {
        p.fixed_step(&mut w, &brake);
    }
    let held = p.vehicles[0].datum(&w);
    for _ in 0..180 {
        p.fixed_step(&mut w, &StepInput::default());
    }
    assert!(p.vehicles[0].datum(&w).z < held.z - 1.0);
    assert!(p.vehicles[0].forward_speed(&w) < -0.5);
    assert!(!p.vehicles[0].reverse_selected() && p.vehicles[0].controls.throttle == 0.0);
}
