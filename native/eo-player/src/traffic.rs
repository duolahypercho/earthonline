//! Kinematic chassis for nearby traffic in local play.

use std::collections::BTreeMap;

use glam::DVec3;
use rapier3d::prelude::*;

use crate::vehicle::VehicleSpec;
use crate::world::{CollisionWorld, layer};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrafficChassis {
    /// Proxy for an actor without a measured mesh, or for a parked catalogue body.
    Class { length: f32, width: f32 },
    /// Chassis bounds and wheel placement in metres relative to the drawn road datum.
    Mesh { min: glam::Vec3, max: glam::Vec3, wheels: [[f32; 2]; 4], wheel_radius: f32 },
}

impl TrafficChassis {
    pub fn length(self) -> f32 {
        match self {
            Self::Class { length, .. } => length,
            Self::Mesh { min, max, .. } => max.z - min.z,
        }
    }

    /// Centre and half extents of the contact box, relative to the road datum.
    pub(crate) fn road_box(self, spec: &VehicleSpec) -> (DVec3, DVec3) {
        match self {
            Self::Class { length, width } => (
                DVec3::Y * ((spec.sill_y + spec.height) / 2.0),
                DVec3::new(width as f64, spec.height - spec.sill_y, length as f64) / 2.0,
            ),
            Self::Mesh { min, max, .. } => {
                let mut min = min.as_dvec3();
                let max = max.as_dvec3();
                if min.y <= 0.0 {
                    min.y = spec.sill_y.min(max.y / 2.0);
                }
                ((min + max) / 2.0, (max - min) / 2.0)
            }
        }
    }
}

#[derive(Clone, Copy)]
pub struct TrafficPose {
    pub id: u64,
    pub datum: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub chassis: TrafficChassis,
}

#[derive(Default)]
pub struct TrafficBodies {
    bodies: BTreeMap<u64, (RigidBodyHandle, TrafficChassis, u64)>,
    generation: u64,
}

impl TrafficBodies {
    /// Apply a batch from the traffic clock. A discontinuity starts a new
    /// collider, so a camera cut cannot sweep a chassis across the player.
    pub fn sync(
        &mut self,
        world: &mut CollisionWorld,
        poses: impl IntoIterator<Item = TrafficPose>,
    ) {
        self.generation += 1;
        let spec = VehicleSpec::sedan();
        for car in poses {
            let rot = Rotation::from_rotation_y(car.yaw) * Rotation::from_rotation_x(-car.pitch);
            let (offset, half) = car.chassis.road_box(&spec);
            let centre = Vector::from_array(car.datum) + rot * Vector::from_array(offset.as_vec3().to_array());
            let pose = Pose::from_parts(centre, rot);
            if let Some((handle, chassis, generation)) = self.bodies.get_mut(&car.id) {
                let body = &mut world.physics.bodies[*handle];
                if *chassis == car.chassis && (body.translation() - centre).length() < car.chassis.length() / 2.0 {
                    body.set_next_kinematic_position(pose);
                    *generation = self.generation;
                    continue;
                }
                world.physics.remove_body(*handle);
            }
            let body = world.physics.insert_body(
                RigidBodyBuilder::kinematic_position_based()
                    .pose(pose)
                    .build(),
            );
            let collider = ColliderBuilder::cuboid(half.x as f32, half.y as f32, half.z as f32)
                .collision_groups(InteractionGroups::new(
                    layer::VEHICLE,
                    layer::VEHICLE | layer::CHARACTER | layer::PEDESTRIAN,
                    InteractionTestMode::And,
                ))
                .build();
            world.physics.insert_collider(collider, Some(body));
            self.bodies.insert(car.id, (body, car.chassis, self.generation));
        }
        self.bodies.retain(|_, (body, _, generation)| {
            if *generation == self.generation {
                true
            } else {
                world.physics.remove_body(*body);
                false
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use glam::DVec3;

    use super::*;
    use crate::{BuildingSet, GroundProvider, Player, StepInput};

    #[test]
    fn a_local_driver_hits_a_traffic_chassis_and_can_drive_on_after_it_leaves() {
        struct Flat;
        impl GroundProvider for Flat {
            fn ground_y(&self, _: f64, _: f64) -> f64 {
                0.0
            }
        }
        let mut world = CollisionWorld::new(Arc::new(Flat), Arc::new(BuildingSet::empty()), 1);
        world.load_blocking(DVec3::ZERO, 60.0);
        let mut player = Player::new(DVec3::ZERO, 0.0);
        let car = player.spawn_vehicle(&mut world, 0.0, 0.0, 0.0);
        player.enter_vehicle(car);
        let driveway = TrafficPose {
            id: 1,
            datum: [0.0, 0.0, 14.0],
            yaw: 0.0,
            pitch: 0.0,
            chassis: TrafficChassis::Class { length: 5.0, width: 2.0 },
        };
        let mut traffic = TrafficBodies::default();
        traffic.sync(&mut world, [driveway]);
        let input = StepInput {
            throttle: 1.0,
            ..Default::default()
        };
        for _ in 0..180 {
            player.fixed_step(&mut world, &input);
        }
        let stopped = player.vehicles[car].datum(&world).z;
        assert!(
            (8.0..10.0).contains(&stopped),
            "player front should reach the traffic car, z = {stopped}"
        );
        traffic.sync(&mut world, []);
        for _ in 0..120 {
            player.fixed_step(&mut world, &input);
        }
        assert!(player.vehicles[car].datum(&world).z > 15.0);
    }
}
