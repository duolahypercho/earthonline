//! Near the player, a traffic chassis can exchange momentum through Rapier.
//! The traffic route supplies the driver's target. Only the solver moves the body.
use crate::traffic::{TrafficBodies, TrafficChassis, TrafficPose};
use crate::vehicle::{Vehicle, VehicleConfig, VehicleModel, VehicleSpec};
use crate::{CollisionWorld, WeatherPhysics};
use glam::{DQuat, DVec3};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy)]
pub struct TrafficTarget {
    pub pose: TrafficPose,
    pub speed: f32,
    pub model: TrafficModel,
    pub parked: bool,
}

/// Curb configurations for the fictional fleet, with reference vehicles and sources
/// in Docs/VEHICLE-PHYSICS.md. Cosmetic trims share their base mechanical setup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrafficModel {
    Road(VehicleModel),
    Wagon,
    Coupe,
    MidEngineCoupe,
    CompactSuv,
    FullSizeSuv,
    Minivan,
    Pickup,
    Taxi,
    PatrolSedan,
    PatrolSuv,
    BoxTruck,
    CityBus,
}

impl TrafficModel {
    pub fn from_body(name: &str) -> Self {
        match name {
            "compactHatch" | "compactHatchSport" | "compactHatchCross" => Self::Road(VehicleModel::Hatchback),
            "deliveryVan" => Self::Road(VehicleModel::DeliveryVan),
            "wagon" => Self::Wagon,
            "coupe" => Self::Coupe,
            "exoticCoupe" => Self::MidEngineCoupe,
            "compactSuv" => Self::CompactSuv,
            "fullSizeSuv" => Self::FullSizeSuv,
            "minivan" => Self::Minivan,
            "pickup" => Self::Pickup,
            "taxi" | "taxiAdBoard" | "taxiCheckered" => Self::Taxi,
            "patrolSedan" => Self::PatrolSedan,
            "patrolSuv" => Self::PatrolSuv,
            "boxTruck" => Self::BoxTruck,
            "cityBus" => Self::CityBus,
            _ => Self::Road(VehicleModel::Sedan),
        }
    }

    fn reference(self) -> VehicleSpec {
        let base = match self {
            Self::Road(model) => model,
            Self::Pickup | Self::FullSizeSuv | Self::BoxTruck | Self::CityBus => VehicleModel::DeliveryVan,
            _ => VehicleModel::Sedan,
        };
        let mut spec = VehicleConfig { model: base, ..Default::default() }.spec();
        let (curb_lb, front_share, front_torque) = match self {
            Self::Road(_) => return spec,
            Self::Wagon => (3770., spec.curb_front_share, 0.5),
            Self::Coupe => (2815., 0.53, 0.0),
            Self::MidEngineCoupe => (3199., 0.44, 0.0),
            Self::CompactSuv => (3542., spec.curb_front_share, 1.0),
            Self::FullSizeSuv => (5534., spec.curb_front_share, 0.0),
            Self::Minivan => (3457., spec.curb_front_share, 1.0),
            Self::Pickup => (4465., 2588. / 4465., 0.0),
            Self::Taxi => (3668., spec.curb_front_share, 1.0),
            Self::PatrolSedan => (4371., spec.curb_front_share, 0.5),
            Self::PatrolSuv => (4672., spec.curb_front_share, 0.5),
            Self::BoxTruck => (8800., spec.curb_front_share, 0.0),
            Self::CityBus => (27730., 8750. / 27730., 0.0),
        };
        let mass = curb_lb * 0.453_592_37;
        let ratio = mass / spec.curb_mass_kg;
        spec.rolling_resistance_n *= ratio;
        spec.engine_brake_force_n *= ratio;
        spec.max_suspension_force *= ratio;
        spec.curb_mass_kg = mass;
        spec.mass_kg = mass;
        spec.curb_front_share = front_share;
        spec.front_torque_share = front_torque;
        spec
    }
}

impl TrafficTarget {
    fn spec(&self) -> VehicleSpec {
        let mut spec = self.model.reference();
        let chassis = self.pose.chassis;
        let (offset, half) = chassis.road_box(&spec);
        let (length, width) = (half.z * 2.0, half.x * 2.0);
        let mass = spec.mass_kg;
        let mut height = match self.model {
            TrafficModel::BoxTruck => 2.85,
            TrafficModel::CityBus => 3.15,
            _ => spec.height,
        };
        let (sx, sz) = (width / spec.width, length / spec.length);
        spec.front_axle_z *= sz;
        spec.rear_axle_z *= sz;
        spec.wheelbase *= sz;
        spec.track_front *= sx;
        spec.track_rear *= sx;
        if let TrafficChassis::Mesh { wheels, wheel_radius, .. } = chassis {
            height = offset.y + half.y;
            spec.sill_y = offset.y - half.y;
            let [fl, fr, rl, rr] = wheels;
            let front = f64::from(fl[1] + fr[1]) / 2.0;
            let rear = f64::from(rl[1] + rr[1]) / 2.0;
            let front_track = f64::from((fl[0] - fr[0]).abs());
            let rear_track = f64::from((rl[0] - rr[0]).abs());
            if front > rear && front_track > 0.0 && rear_track > 0.0 && wheel_radius > 0.0 {
                spec.front_axle_z = front;
                spec.rear_axle_z = rear;
                spec.wheelbase = front - rear;
                spec.track_front = front_track;
                spec.track_rear = rear_track;
                spec.wheel_radius = f64::from(wheel_radius);
            }
        }
        // The longitudinal centre belongs between the drawn axles, even when
        // their midpoint differs from the drivetrain reference's midpoint.
        spec.com_offset_z = spec.rear_axle_z + spec.wheelbase * spec.curb_front_share;
        spec.length = length;
        spec.width = width;
        spec.com_offset_y *= height / spec.height;
        spec.height = height;
        spec.pitch_inertia = mass * (length * length + height * height) / 12.0;
        spec.yaw_inertia = mass * (length * length + width * width) / 12.0;
        spec.roll_inertia = mass * (width * width + height * height) / 12.0;
        spec
    }

    fn spawn(&self, world: &mut CollisionWorld, datum: DVec3) -> Vehicle {
        let spec = self.spec();
        let yaw = f64::from(self.pose.yaw);
        if let TrafficChassis::Mesh { .. } = self.pose.chassis {
            let rot = DQuat::from_rotation_y(yaw) * DQuat::from_rotation_x(-f64::from(self.pose.pitch));
            let center = datum + rot * DVec3::Y * (spec.height / 2.0);
            let (offset, half) = self.pose.chassis.road_box(&spec);
            Vehicle::spawn_chassis(world, spec, (center, rot), offset - DVec3::Y * (spec.height / 2.0), half)
        } else {
            Vehicle::spawn(world, spec, datum, yaw)
        }
    }

    pub fn rotation(&self) -> DQuat {
        DQuat::from_rotation_y(f64::from(self.pose.yaw)) * DQuat::from_rotation_x(-f64::from(self.pose.pitch))
    }

    pub fn approaching(&self, other: &Vehicle, world: &CollisionWorld) -> bool {
        let (offset, half) = self.pose.chassis.road_box(&self.spec());
        let rotation = self.rotation();
        let centre = DVec3::from(self.pose.datum.map(f64::from)) + rotation * offset;
        let velocity = rotation * DVec3::Z * f64::from(self.speed);
        crate::vehicle::approaching_traffic(other, world, centre, rotation, half, velocity)
    }
}

pub struct TrafficDriver {
    pub car: Vehicle,
    pub target: TrafficTarget,
}

impl TrafficDriver {
    pub fn spawn(world: &mut CollisionWorld, target: TrafficTarget) -> Self {
        let spec = target.spec();
        let rot = target.rotation();
        let datum = DVec3::from(target.pose.datum.map(f64::from));
        let half_height = DVec3::Y * (spec.height / 2.0);
        let (offset, half) = target.pose.chassis.road_box(&spec);
        let car = Vehicle::spawn_chassis(world, spec, (datum + rot * half_height, rot), offset - half_height, half);
        world.physics.bodies[car.body].set_linvel((rot * DVec3::Z * f64::from(target.speed)).as_vec3().to_array().into(), true);
        Self { car, target }
    }

    pub fn drive(&mut self, world: &mut CollisionWorld, weather: &WeatherPhysics) {
        let t = &self.target;
        let car = &mut self.car;
        if t.parked {
            car.pre_step_with_weather(world, 0.0, 0.0, 0.0, true, false, weather);
            return;
        }
        let dir = DQuat::from_rotation_y(f64::from(t.pose.yaw)) * DVec3::Z;
        let at = car.datum(world);
        let goal = DVec3::from(t.pose.datum.map(f64::from));
        let speed = car.forward_speed(world);
        let lookahead = 3.0 + f64::from(t.speed) * 0.55;
        let to = goal + dir * lookahead - at;
        let error = to.x.atan2(to.z) - car.yaw(world);
        let heading_error = error.sin().atan2(error.cos());
        let desired_speed = (f64::from(t.speed) + 0.65 * (goal - at).dot(dir))
            .clamp(0.0, f64::from(t.speed) + 2.0);
        let dv = desired_speed - speed;
        let steer = (heading_error / car.spec.max_steer_rad).clamp(-1.0, 1.0);
        let park = desired_speed < 0.2 && speed.abs() < 0.5;
        // Brake demand at rest can select reverse under player controls.
        let brake = if speed > 0.4 { (-dv / 4.0).clamp(0.0, 1.0) } else { 0.0 };
        car.pre_step_with_weather(world, steer, (dv / 3.0).clamp(0.0, 1.0), brake, park, true, weather);
    }
}

#[derive(Default)]
pub struct ContactTraffic {
    drivers: BTreeMap<u64, TrafficDriver>,
    weather: WeatherPhysics,
    displaced: BTreeMap<u64, (crate::vehicle::VehicleState, PosedTraffic)>,
}

/// A physical pose for the existing traffic instance, absolute game metres.
pub type PosedTraffic = (u64, [f32; 3], [f32; 4], f32);

impl ContactTraffic {
    pub(crate) fn is_empty(&self) -> bool {
        self.drivers.is_empty()
    }

    pub fn poses<'a>(
        &'a self,
        world: &'a CollisionWorld,
    ) -> impl Iterator<Item = PosedTraffic> + 'a {
        self.drivers
            .iter()
            .map(move |(&id, d)| {
                (
                    id,
                    d.car.datum(world).as_vec3().to_array(),
                    d.car.pose(world).1.as_quat().to_array(),
                    d.car.forward_speed(world) as f32,
                )
            })
            .chain(self.displaced.values().map(|(_, pose)| *pose))
    }

    pub(crate) fn step(&mut self, world: &mut CollisionWorld) {
        for driver in self.drivers.values_mut() {
            driver.drive(world, &self.weather);
        }
    }

    pub fn sync(
        world: &mut CollisionWorld,
        scripted: &mut TrafficBodies,
        near: DVec3,
        actor_speed: f64,
        weather: WeatherPhysics,
        targets: impl IntoIterator<Item = TrafficTarget>,
    ) {
        let mut fleet = std::mem::take(&mut world.contact_traffic);
        fleet.weather = weather;
        let mut scripted_poses = Vec::new();
        let mut seen = BTreeSet::new();
        for mut t in targets {
            let id = t.pose.id;
            let parked_state = fleet.displaced.get(&id).copied();
            let physical_at = parked_state.map_or(t.pose.datum, |(_, p)| p.1);
            let datum = DVec3::from(physical_at.map(f64::from));
            let reach = 3.5
                + 0.5 * f64::from(t.pose.chassis.length())
                + 0.3 * (actor_speed.abs() + f64::from(t.speed.abs()));
            if let Some(d) = fleet.drivers.get_mut(&id).filter(|d|
                d.target.pose.chassis == t.pose.chassis && d.target.model == t.model && d.target.parked == t.parked) {
                d.target = t;
                seen.insert(id);
                continue;
            }
            if let Some(d) = fleet.drivers.remove(&id) {
                world.physics.remove_body(d.car.body);
            }
            if datum.distance(near) < reach
                && (datum.y - near.y).abs() < 2.5
                && world.is_resident(datum.x, datum.z)
            {
                let mut car = t.spawn(world, datum);
                let forward = car.pose(world).1 * DVec3::Z;
                world.physics.bodies[car.body].set_linvel(
                    (forward * f64::from(t.speed)).as_vec3().to_array().into(),
                    true,
                );
                if let Some((state, _)) = fleet.displaced.remove(&id) {
                    car.set_state(world, &state);
                }
                fleet.drivers.insert(id, TrafficDriver { car, target: t });
                seen.insert(id);
            } else {
                if let Some((_, p)) = parked_state {
                    let fwd = glam::Quat::from_array(p.2) * glam::Vec3::Z;
                    t.pose.datum = p.1;
                    t.pose.yaw = fwd.x.atan2(fwd.z);
                    t.pose.pitch = fwd.y.asin();
                }
                scripted_poses.push(t.pose);
            }
        }
        fleet.drivers.retain(|id, d| {
            if seen.contains(id) {
                return true;
            }
            let at = d.car.datum(world);
            if d.target.parked
                && at.distance(DVec3::from(d.target.pose.datum.map(f64::from))) > 0.15
            {
                let pose = (
                    *id,
                    at.as_vec3().to_array(),
                    d.car.pose(world).1.as_quat().to_array(),
                    0.0,
                );
                fleet.displaced.insert(*id, (d.car.state(world), pose));
            }
            world.physics.remove_body(d.car.body);
            false
        });
        scripted.sync(world, scripted_poses);
        world.contact_traffic = fleet;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BuildingSet, GroundProvider, tuning::GRAVITY_MPS2};
    use std::sync::Arc;

    struct Flat;

    impl GroundProvider for Flat {
        fn ground_y(&self, _: f64, _: f64) -> f64 { 0.0 }
    }

    fn world() -> CollisionWorld {
        CollisionWorld::new(Arc::new(Flat), Arc::new(BuildingSet::empty()), 1)
    }

    fn target(model: TrafficModel, length: f32) -> TrafficTarget {
        TrafficTarget {
            model,
            speed: 0.0,
            parked: true,
            pose: TrafficPose {
                id: 1, datum: [0.; 3], yaw: 0., pitch: 0.,
                chassis: TrafficChassis::Class { length, width: 2.0 },
            },
        }
    }

    #[test]
    fn settled_traffic_wheels_carry_the_reference_axle_weights() {
        let mut world = world();
        world.load_blocking(DVec3::ZERO, crate::world::STEP_RADIUS_M);
        for (model, length, front_lb, rear_lb) in [
            (TrafficModel::CityBus, 12.2, 8750., 18980.),
            (TrafficModel::Pickup, 5.89, 2588., 1877.),
            (TrafficModel::Coupe, 4.5, 0.53 * 2815., 0.47 * 2815.),
        ] {
            let mut target = target(model, length);
            // An offset wheelbase exercises the mesh path as well as the weight distribution.
            target.pose.chassis = TrafficChassis::Mesh {
                min: glam::Vec3::new(-1., 0.35, -length / 2.),
                max: glam::Vec3::new(1., 2., length / 2.),
                wheels: [[0.8, 0.31 * length], [-0.8, 0.31 * length],
                         [0.8, -0.27 * length], [-0.8, -0.27 * length]],
                wheel_radius: 0.38,
            };
            let mut vehicle = target.spawn(&mut world, DVec3::ZERO);
            for _ in 0..360 {
                vehicle.pre_step(&mut world, 0., 0., 0., true, false);
                world.step_physics();
            }
            let wheel = vehicle.controller.wheels();
            let axle_kg = |i: usize| f64::from(wheel[i].wheel_suspension_force + wheel[i + 1].wheel_suspension_force) / GRAVITY_MPS2;
            let (front, rear) = (axle_kg(0), axle_kg(2));
            eprintln!("{model:?} supported front/rear {front:.1}/{rear:.1} kg");
            for (actual, reference_lb) in [(front, front_lb), (rear, rear_lb)] {
                let expected = reference_lb * 0.453_592_37;
                assert!((actual - expected).abs() < expected * 0.015, "{model:?}: {actual} kg vs {expected} kg");
            }
            world.physics.remove_body(vehicle.body);
        }
    }

    #[test]
    fn traffic_impact_conserves_horizontal_momentum_under_gravity() {
        let impact = |model, length| {
            // Both chassis are in free fall so the road cannot supply a horizontal impulse.
            let mut world = world();
            let striking = target(TrafficModel::Road(VehicleModel::Sedan), 4.85);
            let struck = target(model, length);
            let datum = DVec3::Y * 20.;
            let a = striking.spawn(&mut world, datum);
            let b = struck.spawn(&mut world, datum + DVec3::Z * (0.5 * (4.85 + f64::from(length)) + 0.2));
            world.physics.bodies[a.body].set_linvel([0., 0., 10.].into(), true);
            for _ in 0..15 { world.step_physics(); }
            let (a, b) = (&world.physics.bodies[a.body], &world.physics.bodies[b.body]);
            let transferred = b.mass() * b.linvel().z;
            let lost = a.mass() * (10.0 - a.linvel().z);
            eprintln!("{model:?} struck at 10 m/s: target {:.3} m/s, impulse {transferred:.1} N s, momentum residual {:.3} N s", b.linvel().z, lost - transferred);
            assert!(b.linvel().z > 0.5 && a.linvel().z < 9.5, "chassis must collide");
            assert!((lost - transferred).abs() < 0.0001 * a.mass() * 10., "unequal impulses");
            b.linvel().z
        };
        let coupe = impact(TrafficModel::Coupe, 4.5);
        let pickup = impact(TrafficModel::Pickup, 5.89);
        let bus = impact(TrafficModel::CityBus, 12.2);
        assert!(coupe > pickup && pickup > bus);
    }
}
