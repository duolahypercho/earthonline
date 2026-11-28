//! Takeoff and landing through the fixed-step driver on a rounded test track.
use eo_player::tuning::{FIXED_STEP_S, GRAVITY_MPS2};
use eo_player::vehicle::{VehicleConfig, VehicleModel};
use eo_player::world::{DeckPart, Decks, SurfaceMesh, TILE_SIZE_M, TileKey};
use eo_player::{BuildingSet, CollisionWorld, GroundProvider, Player, StepInput, WeatherPhysics};
use glam::DVec3;
use std::sync::Arc;

const RADIUS: f64 = 25.0;
const GRADE: f64 = 0.32;

// A parabolic summit with a constant-grade approach, a rounded transition
// back to level pavement. Some runs put the tile boundary in the downhill dip.
fn height_from_summit(distance: f64) -> f64 {
    let d = distance.abs();
    let bend = RADIUS * GRADE;
    let straight_end = bend + 18.0;
    if d < bend {
        return -d * d / (2.0 * RADIUS);
    }
    let run = d.clamp(bend, straight_end) - bend;
    let u = (d - straight_end).clamp(0.0, bend);
    -bend * GRADE * 0.5 - GRADE * (run + u) + u * u / (2.0 * RADIUS)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Track {
    Street,
    StackedStreet,
    DipAtTileEdge,
    Deck,
}

impl Track {
    fn summit(self) -> f64 {
        if self == Self::DipAtTileEdge {
            TILE_SIZE_M - 29.0
        } else {
            TILE_SIZE_M
        }
    }

    fn road(self, z: f64) -> f64 {
        height_from_summit(z - self.summit())
    }

    fn tile(self, key: TileKey) -> SurfaceMesh {
        let (x, z) = key.origin();
        let mut mesh = SurfaceMesh::default();
        for i in 0..(TILE_SIZE_M as usize * 2) {
            let a = z + i as f64 * 0.5;
            let b = a + 0.5;
            let [al, ar] = [x, x + TILE_SIZE_M].map(|x| [x, self.road(a), a]);
            let [bl, br] = [x, x + TILE_SIZE_M].map(|x| [x, self.road(b), b]);
            mesh.street.extend([[al, bl, br], [al, br, ar]]);
        }
        mesh
    }
}

impl GroundProvider for Track {
    fn ground_y(&self, _: f64, z: f64) -> f64 {
        self.road(z) - 1.0
    }
    fn multi_level(&self) -> bool {
        *self == Track::StackedStreet
    }
    fn surface_tile(&self, key: TileKey) -> Option<SurfaceMesh> {
        (*self != Track::Deck).then(|| self.tile(key))
    }
}

fn driver(config: VehicleConfig, track: Track) -> (Player, CollisionWorld) {
    let mut buildings = BuildingSet::empty();
    if track == Track::Deck {
        let walk = (0..=1)
            .flat_map(|z| track.tile(TileKey { x: 0, z }).street)
            .collect();
        buildings = buildings.with_decks(Decks {
            parts: vec![DeckPart {
                name: "test deck".into(),
                walk,
                ..Default::default()
            }],
            ..Default::default()
        });
    }
    let mut world = CollisionWorld::new(Arc::new(track), Arc::new(buildings), 1);
    let z = track.summit() - 20.0;
    let start = DVec3::new(16.0, track.road(z), z);
    world.load_blocking(start, 50.0);
    let mut player = Player::new(start, 0.0);
    player.spawn_config = config;
    let index = player.spawn_vehicle(&mut world, start.x, z, 0.0);
    player.enter_vehicle(index);
    for _ in 0..180 {
        player.fixed_step(
            &mut world,
            &StepInput {
                brake: 1.0,
                ..Default::default()
            },
        );
    }
    (player, world)
}

#[test]
fn speed_and_curvature_determine_whether_a_car_leaves_a_crest() {
    for (model, cargo_kg, track) in [
        (VehicleModel::Hatchback, 0, Track::Street),
        (VehicleModel::Hatchback, 0, Track::DipAtTileEdge),
        (VehicleModel::Sedan, 0, Track::Street),
        (VehicleModel::Sedan, 0, Track::Deck),
        (VehicleModel::DeliveryVan, 600, Track::StackedStreet),
    ] {
        for speed in [13.5, 24.0] {
            let (mut player, mut world) = driver(
                VehicleConfig {
                    model,
                    cargo_kg,
                    ..Default::default()
                },
                track,
            );
            let body = player.vehicles[0].body;
            let forward = DVec3::new(0.0, GRADE, 1.0).normalize();
            world.physics.bodies[body]
                .set_linvel((forward * speed).as_vec3().to_array().into(), true);
            let mut max_gap = 0.0_f64;
            let mut worst_unbraked_loss = 0.0_f64;
            let mut measured_apex_error = None;
            let mut takeoff = None;
            let mut apex = f64::NEG_INFINITY;
            let mut air_ticks = 0;
            let mut longest = 0;
            let mut flight_error = 0.0_f64;
            let mut summit_speed = None;
            for _ in 0..600 {
                let state = &world.physics.bodies[body];
                let v = DVec3::from_array(state.linvel().to_array().map(f64::from));
                let com_y = f64::from(state.center_of_mass().y);
                let z = player.position(&world).z;
                let stop = z > track.summit() + 45.0;
                let in_air = player.vehicles[0].wheels_in_contact() == 0;
                let above_road = player.position(&world).y - track.road(z);
                player.fixed_step(
                    &mut world,
                    &StepInput {
                        brake: if stop { 1.0 } else { 0.0 },
                        steer: if in_air { 0.6 } else { 0.0 },
                        throttle: if in_air && above_road > 1.0 { 1.0 } else { 0.0 },
                        analog_steer: true,
                        ..Default::default()
                    },
                );
                let car = &player.vehicles[0];
                let feet = car.datum(&world);
                let actual = DVec3::from_array(
                    world.physics.bodies[body]
                        .linvel()
                        .to_array()
                        .map(f64::from),
                );
                if !stop {
                    worst_unbraked_loss = worst_unbraked_loss.max(v.z - actual.z);
                }
                if summit_speed.is_none() && feet.z >= track.summit() {
                    summit_speed = Some(v.z);
                }
                // A bumper or the underbody can touch first during landing.
                // The ballistic interval ends at the first chassis or wheel impulse.
                let hit_body = world
                    .physics
                    .narrow_phase
                    .contact_pairs_with(car.collider)
                    .any(|pair| pair.total_impulse_magnitude() > 0.0);
                if car.wheels_in_contact() == 0 && !hit_body {
                    air_ticks += 1;
                    longest = longest.max(air_ticks);
                    let expected = v
                        - DVec3::Y * (GRAVITY_MPS2 * FIXED_STEP_S)
                        - v * (v.length() * car.spec.aero_factor / car.spec.mass_kg * FIXED_STEP_S);
                    flight_error = flight_error.max(actual.distance(expected));
                    let (y, vy) = *takeoff.get_or_insert((com_y, v.y));
                    if v.y > 0.0 && actual.y <= 0.0 {
                        let ballistic_apex = y + vy * vy / (2.0 * GRAVITY_MPS2);
                        measured_apex_error = Some((com_y - ballistic_apex).abs());
                    }
                } else {
                    air_ticks = 0;
                    takeoff = None;
                }
                max_gap = max_gap.max(feet.y - track.road(feet.z));
                apex = apex.max(feet.y);
                if stop && car.forward_speed(&world).abs() < 0.01 && car.wheels_in_contact() == 4 {
                    break;
                }
            }
            let summit = summit_speed.expect("reached the summit");
            let car = &player.vehicles[0];
            println!(
                "{track:?} {model:?} cargo {cargo_kg} kg, entry {speed:.1} m/s, summit {summit:.2} m/s, longest flight {longest} ticks, peak road gap {max_gap:.3} m, apex {apex:.3} m, max free-flight velocity residual {flight_error:.6} m/s, ballistic apex error {measured_apex_error:?} m, largest unbraked forward speed loss {worst_unbraked_loss:.3} m/s"
            );
            assert!(car.datum(&world).z > track.summit() + 40.0 && car.wheels_in_contact() == 4);
            if speed == 13.5 {
                assert!(
                    worst_unbraked_loss < 0.5,
                    "a smooth road seam arrested the car"
                );
                assert!(summit * summit / RADIUS < GRAVITY_MPS2 && longest == 0 && max_gap < 0.15);
            } else {
                assert!(
                    measured_apex_error.is_some_and(|e| e < 0.05),
                    "missed the ballistic apex"
                );
                assert!(summit * summit / RADIUS > GRAVITY_MPS2 && longest >= 20 && max_gap > 0.3);
                assert!(
                    flight_error < 0.001,
                    "{model:?} acquired an unaccounted impulse in free flight"
                );
            }
        }
    }
}

fn airborne_sedan(track: Track, height: f32) -> (Player, CollisionWorld) {
    let (mut p, mut w) = driver(
        VehicleConfig {
            model: VehicleModel::Sedan,
            ..Default::default()
        },
        track,
    );
    let car = p.vehicles[0].body;
    let centre = w.physics.bodies[car].translation();
    w.physics.bodies[car].set_translation(centre + rapier3d::prelude::Vector::Y * height, true);
    w.physics.bodies[car].set_linvel([0.0, 2.0, 12.0].into(), true);
    p.fixed_step(&mut w, &StepInput::default());
    (p, w)
}

#[test]
fn a_freely_rotating_chassis_preserves_angular_momentum() {
    let (mut player, mut world) = airborne_sedan(Track::Street, 20.0);
    let body = player.vehicles[0].body;
    world.physics.bodies[body].set_angvel([1.1, 1.7, -0.9].into(), true);
    let momentum_and_energy = |player: &Player, world: &CollisionWorld| {
        let car = &player.vehicles[0];
        let inertia = DVec3::new(
            car.spec.pitch_inertia,
            car.spec.yaw_inertia,
            car.spec.roll_inertia,
        );
        let (_, orientation) = car.pose(world);
        let rate = DVec3::from_array(
            world.physics.bodies[body]
                .angvel()
                .to_array()
                .map(f64::from),
        );
        let momentum = orientation * (inertia * (orientation.inverse() * rate));
        (momentum, 0.5 * momentum.dot(rate))
    };
    let (initial_momentum, initial_energy) = momentum_and_energy(&player, &world);
    let mut max_momentum_error = 0.0_f64;
    let mut max_energy_error = 0.0_f64;
    for _ in 0..60 {
        player.fixed_step(&mut world, &StepInput::default());
        assert_eq!(player.vehicles[0].wheels_in_contact(), 0);
        assert!(
            world
                .physics
                .narrow_phase
                .contact_pairs_with(player.vehicles[0].collider)
                .all(|pair| pair.total_impulse_magnitude() == 0.0)
        );
        let (momentum, energy) = momentum_and_energy(&player, &world);
        max_momentum_error =
            max_momentum_error.max(momentum.distance(initial_momentum) / initial_momentum.length());
        max_energy_error = max_energy_error.max((energy / initial_energy - 1.0).abs());
    }
    println!(
        "torque-free sedan, 60 ticks, peak relative angular momentum error {max_momentum_error:.6}, energy error {max_energy_error:.6}"
    );
    assert!(max_momentum_error < 0.01 && max_energy_error < 0.01);
}

#[test]
fn loading_ground_during_free_flight_does_not_advance_simulation_time() {
    for track in [Track::Street, Track::StackedStreet] {
        let (mut p, mut w) = airborne_sedan(track, 4.0);
        let (mut control, mut ordinary) = airborne_sedan(track, 4.0);
        let car = p.vehicles[0].body;
        assert_eq!(p.vehicles[0].wheels_in_contact(), 0);
        let initial = p.state(&w);
        let at = p.position(&w);
        for center in [at, at + DVec3::Z * 2.0 * TILE_SIZE_M, at] {
            w.load_blocking(center, 40.0);
            assert!(
                (w.surface_y(center.x, center.z) - track.road(center.z)).abs() < 0.01,
                "new collision has to be immediately queryable"
            );
            let v = w.physics.bodies[car].linvel();
            assert_eq!(
                p.state(&w),
                initial,
                "tile loading moved an airborne car without a fixed tick ({track:?}, velocity {v:?})"
            );
        }
        // Continue through touchdown. A query refresh must preserve the
        // flight and the response of the springs when the wheels meet the road.
        for tick in 0..180 {
            let input = StepInput {
                brake: 1.0,
                ..Default::default()
            };
            p.fixed_step(&mut w, &input);
            control.fixed_step(&mut ordinary, &input);
            assert_eq!(
                p.state(&w),
                control.state(&ordinary),
                "trajectory after tile load, tick {tick}"
            );
        }
        assert_eq!(p.vehicles[0].wheels_in_contact(), 4);
    }
}

#[test]
fn taking_the_driver_out_of_an_airborne_car_preserves_its_wind_and_gravity() {
    let setup = || {
        let (mut p, w) = airborne_sedan(Track::Street, 9.0);
        p.set_weather(WeatherPhysics {
            wind: [20.0, 0.0],
            ..Default::default()
        });
        (p, w)
    };
    let (mut driven, mut a) = setup();
    let (mut parked, mut b) = setup();
    parked.mode = eo_player::Mode::OnFoot;
    let side = parked.vehicles[0].datum(&b) + DVec3::X * 6.0;
    parked
        .character
        .teleport_feet(DVec3::new(side.x, Track::Street.road(side.z), side.z));
    for tick in 0..55 {
        driven.fixed_step(
            &mut a,
            &StepInput {
                handbrake: true,
                ..Default::default()
            },
        );
        parked.fixed_step(&mut b, &StepInput::default());
        let car = &driven.vehicles[0];
        let empty = &parked.vehicles[0];
        assert_eq!(car.wheels_in_contact(), 0);
        assert_eq!(empty.wheels_in_contact(), 0);
        let velocity = a.physics.bodies[car.body].linvel();
        let other = b.physics.bodies[empty.body].linvel();
        assert_eq!(
            velocity, other,
            "weather force changed when the driver left at tick {tick}"
        );
        assert_eq!(car.pose(&a), empty.pose(&b));
    }
    assert!(a.physics.bodies[driven.vehicles[0].body].linvel().x > 0.05);
}
