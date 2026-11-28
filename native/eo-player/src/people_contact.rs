//! Query colliders for the people shown near the local on-foot player.
use crate::{CollisionWorld, tuning::capsule, world::layer};
use rapier3d::parry::query::ShapeCastOptions;
use rapier3d::prelude::*;
use std::collections::BTreeMap;

#[derive(Default)]
pub struct PeopleBodies {
    handles: BTreeMap<usize, (ColliderHandle, u64, f32)>,
    generation: u64,
}

impl PeopleBodies {
    /// A drawn slot may change identity after a crowd handoff. Update its
    /// current shape directly so recycling it never sweeps through a street.
    pub fn sync(
        &mut self,
        world: &mut CollisionWorld,
        feet: impl IntoIterator<Item = (usize, [f32; 3], f32)>,
    ) {
        self.generation += 1;
        for (id, point, scale) in feet {
            let radius = capsule::RADIUS_M as f32 * scale;
            let half = capsule::HALF_HEIGHT_M as f32 * scale;
            let centre = Vector::from_array(point) + Vector::Y * (half + radius);
            if let Some((handle, seen, last_scale)) = self.handles.get_mut(&id) {
                let collider = &mut world.physics.colliders[*handle];
                collider.set_translation(centre);
                if *last_scale != scale {
                    collider.set_shape(SharedShape::capsule_y(half, radius));
                }
                *last_scale = scale;
                *seen = self.generation;
            } else {
                let collider = ColliderBuilder::capsule_y(half, radius)
                    .translation(centre)
                    .collision_groups(InteractionGroups::new(
                        layer::PEDESTRIAN,
                        layer::CHARACTER,
                        InteractionTestMode::And,
                    ));
                let handle = world.physics.insert_collider(collider, None);
                self.handles.insert(id, (handle, self.generation, scale));
            }
        }
        self.handles.retain(|_, (handle, seen, _)| {
            if *seen == self.generation {
                return true;
            }
            world.physics.remove_collider(*handle);
            false
        });
    }
}

/// The unadjusted route identifies the walker across frames. The feed's
/// supported foot position is the walking target at handoff and afterwards.
#[derive(Clone, Copy)]
pub struct WalkerTarget {
    pub id: u64,
    pub model: u32,
    pub feet: [f32; 3],
    pub drawn: [f32; 3],
    pub scale: f32,
    pub heading: f32,
    pub phase: f32,
    pub stride_m: f32,
    pub speed: f32,
}

impl WalkerTarget {
    /// Keep the tangential gait when the crowd has moved the stance clear of an obstacle.
    fn travel(&self) -> Vector {
        let mut velocity = Rotation::from_rotation_y(self.heading) * Vector::Z * self.speed;
        let mut clearance = Vector::from_array(self.drawn) - Vector::from_array(self.feet);
        clearance.y = 0.;
        let outward = clearance.normalize_or_zero();
        velocity -= outward * velocity.dot(outward).min(0.);
        velocity
    }
}

#[derive(Clone, Copy)]
pub struct WalkerPose {
    pub model: u32,
    pub route_feet: [f32; 3],
    pub feet: [f32; 3],
    pub rotation: [f32; 4],
    pub walking_speed: f32,
    pub phase: f32,
}

struct WalkingBody {
    handle: RigidBodyHandle,
    collider: ColliderHandle,
    target: WalkerTarget,
    seen: u64,
    unbalanced: u16,
    struck: bool,
    phase: f32,
    walking_speed: f32,
}

#[derive(Default)]
pub struct WalkingBodies {
    bodies: BTreeMap<u64, WalkingBody>,
    pub vehicle_impacts: u64,
    generation: u64,
}

impl WalkingBody {
    fn extent(&self) -> f32 {
        capsule::CENTER_FROM_FEET_M as f32 * self.target.scale
    }

    fn pose(&self, physics: &PhysicsWorld) -> WalkerPose {
        let body = &physics.bodies[self.handle];
        WalkerPose {
            model: self.target.model,
            route_feet: self.target.feet,
            feet: (body.translation() - body.rotation() * Vector::Y * self.extent()).to_array(),
            rotation: body.rotation().to_array(),
            walking_speed: self.walking_speed,
            phase: self.phase,
        }
    }

    fn step(&mut self, world: &mut CollisionWorld) {
        let physics = &world.physics;
        let mut ground_normal = Vector::ZERO;
        self.struck = false;
        let mass = physics.bodies[self.handle].mass();
        for pair in physics.narrow_phase.contact_pairs_with(self.collider) {
            if !pair.has_any_active_contact() {
                continue;
            }
            let first = pair.collider1 == self.collider;
            let other = if first {
                pair.collider2
            } else {
                pair.collider1
            };
            let groups = physics.colliders[other].collision_groups().memberships;
            if groups.contains(layer::VEHICLE) && pair.total_impulse_magnitude() > mass {
                // A contact impulse changing speed by over 1 m/s interrupts the gait.
                self.struck |= self.unbalanced == 0;
                self.unbalanced = 90;
            }
            if groups.intersects(layer::WORLD | layer::STREET | layer::BOAT) {
                let sign = if first { -1.0 } else { 1.0 };
                for manifold in pair.manifolds() {
                    let normal = sign * manifold.data.normal;
                    if normal.y > ground_normal.y {
                        ground_normal = normal;
                    }
                }
            }
        }
        self.walking_speed = 0.0;
        if self.unbalanced > 0 {
            self.unbalanced -= 1;
            return;
        }
        if ground_normal.y <= 0.55 {
            return;
        }
        let extent = self.extent();
        let body = &physics.bodies[self.handle];
        let dt = crate::tuning::FIXED_STEP_S as f32;
        let route_heading = Rotation::from_rotation_y(self.target.heading);
        let upright = body.rotation() * Vector::Y;
        // Drive the centre of mass over the stance. Chasing a tilting foot
        // feeds the balance error back into horizontal acceleration.
        let centre = body.translation();
        let horizontal_plane = Vector::new(1., 0., 1.);
        let stance = Vector::from_array(self.target.drawn);
        let mut delta = (stance - centre) * horizontal_plane;
        // At a stop the sole can still lag the centre as the capsule balances.
        // Blend in its actual position over the final capsule radius.
        if self.target.speed == 0. {
            let radius = capsule::RADIUS_M as f32 * self.target.scale;
            let weight = (1. - delta.length() / radius).clamp(0., 1.);
            delta += upright * horizontal_plane * (extent * weight);
        }
        let horizontal = body.linvel() * horizontal_plane;
        let correction_rate = if self.target.speed == 0. { 4. } else { 2. };
        let mut goal = (self.target.travel() + delta * correction_rate)
            .clamp_length_max(self.target.speed.max(1.6) + 0.6);
        // A route can pass across two valid but disconnected floor triangles.
        // Brake while the leading foot still has support, allowing for traction
        // and the contact velocity. A real external shove remains a rigid-body impulse.
        const GAIT_ACCEL: f32 = 7.;
        let radius = capsule::RADIUS_M as f32 * self.target.scale;
        let feet = centre - upright * extent;
        // A struck walker may recover on the opposite side of the stopped car.
        // Choose the nearest end of the blocking face with enough room for the capsule.
        if delta.length_squared() > radius * radius && upright.y >= 0.85 {
            goal = around_vehicle(physics, centre, goal, radius, delta.length());
        }
        let direction = goal.normalize_or_zero();
        if direction != Vector::ZERO && upright.y >= 0.85 {
            let speed = horizontal.dot(direction).max(0.);
            let stop_distance = speed * dt + speed * speed / (2. * GAIT_ACCEL);
            let step = capsule::MAX_STEP_HEIGHT_M;
            let slope_limit = capsule::MAX_SLOPE_DEGREES.to_radians().cos();
            for distance in [radius, radius + stop_distance] {
                let ahead = direction * distance;
                let along_floor =
                    feet + ahead - Vector::Y * (ground_normal.dot(ahead) / ground_normal.y);
                let at = glam::DVec3::from(along_floor.to_array().map(f64::from));
                let support = world.raycast(
                    at + glam::DVec3::Y * step,
                    glam::DVec3::NEG_Y,
                    2. * step,
                    layer::STREET | layer::WORLD | layer::BOAT,
                    None,
                );
                if !support.is_some_and(|hit| hit.normal.y + 1e-5 >= slope_limit) {
                    goal = Vector::ZERO;
                    break;
                }
                if stop_distance < 0.01 {
                    break;
                }
            }
        }
        let body = &mut world.physics.bodies[self.handle];
        let facing = if goal.length_squared() > 0.15 * 0.15 {
            Rotation::from_rotation_y(goal.x.atan2(goal.z))
        } else {
            route_heading
        };
        // Balance about the point of support. Standing up lifts the centre
        // of mass over the end of the capsule against gravity.
        let tilt = upright.cross(Vector::Y);
        let lean = tilt.length().atan2(upright.y);
        let axis = if tilt.length_squared() > 1.0e-8 {
            tilt.normalize()
        } else {
            *body.rotation() * Vector::X
        };
        let spin = body.angvel();
        let tilt_speed = spin - Vector::Y * spin.y;
        let tilt_accel = axis * (lean * 22.0) - tilt_speed * 8.0;
        let forward = *body.rotation() * Vector::Z;
        let wanted = facing * Vector::Z;
        let yaw_error = (forward.z * wanted.x - forward.x * wanted.z)
            .atan2(forward.x * wanted.x + forward.z * wanted.z);
        let yaw_accel = Vector::Y * ((yaw_error * 14.0 - spin.y * 7.0) * upright.y.max(0.0));
        let inertia = body.mass_properties().effective_angular_inertia();
        let lever = capsule::HALF_HEIGHT_M as f32 * self.target.scale;
        let gravity = crate::tuning::GRAVITY_MPS2 as f32;
        let torque = inertia * (tilt_accel + yaw_accel)
            + mass * lever * lever * tilt_accel
            + mass * gravity * lever * tilt;
        let impulse = torque.clamp_length_max(mass * gravity * extent * 1.45) * dt;
        if impulse.length_squared() > 1.0e-8 {
            body.apply_torque_impulse(impulse, true);
        }
        if upright.y < 0.85 {
            return;
        }
        let gravity_along_floor = gravity * ground_normal.y * (ground_normal * horizontal_plane);
        let impulse = ((goal - horizontal).clamp_length_max(GAIT_ACCEL * dt)
            - gravity_along_floor * dt)
            * mass;
        if impulse.length_squared() > 1.0e-8 {
            body.apply_impulse(impulse, true);
        }
        self.walking_speed = horizontal
            .dot(goal.normalize_or_zero())
            .clamp(0.0, goal.length());
        self.phase = (self.phase + self.walking_speed * dt / self.target.stride_m.max(0.2)).fract();
    }
}

fn around_vehicle(
    physics: &PhysicsWorld,
    centre: Vector,
    goal: Vector,
    radius: f32,
    remaining: f32,
) -> Vector {
    let direction = goal.normalize_or_zero();
    if direction == Vector::ZERO {
        return goal;
    }
    let vehicles = QueryFilter::default().groups(InteractionGroups::new(
        Group::ALL,
        layer::VEHICLE,
        InteractionTestMode::And,
    ));
    let travel = remaining.min(3. * radius);
    let probe = Ball::new(radius);
    let options = ShapeCastOptions {
        max_time_of_impact: travel,
        target_distance: 0.,
        stop_at_penetration: true,
        compute_impact_geometry_on_penetration: true,
    };
    let Some((handle, _)) = physics.cast_shape(
        &Pose::from_translation(centre),
        direction,
        &probe,
        options,
        vehicles,
    ) else {
        return goal;
    };
    let car = &physics.colliders[handle];
    let pose = car.position();
    let bounds = car.shape().compute_local_aabb();
    let local = pose.rotation.inverse() * (centre - pose.translation) - bounds.center();
    let half = bounds.half_extents() + Vector::splat(radius);
    let (normal, tangent) = if local.x.abs() - half.x > local.z.abs() - half.z {
        (Vector::X, Vector::Z)
    } else {
        (Vector::Z, Vector::X)
    };
    if (pose.rotation.inverse() * goal).dot(normal) * local.dot(normal) > 0. {
        return goal;
    }
    let offset = local.dot(tangent);
    let forward = (pose.rotation * tangent * Vector::new(1., 0., 1.)).normalize_or_zero();
    let along = goal.dot(forward);
    let sign = if along.abs() > goal.length() * 0.5 {
        along.signum()
    } else if offset < 0. {
        -1.
    } else {
        1.
    };
    forward * sign * goal.length()
}

impl WalkingBodies {
    pub fn len(&self) -> usize {
        self.bodies.len()
    }
    pub fn is_empty(&self) -> bool {
        self.bodies.is_empty()
    }
    pub fn contains(&self, id: u64) -> bool {
        self.bodies.contains_key(&id)
    }

    pub fn poses<'a>(
        &'a self,
        physics: &'a PhysicsWorld,
    ) -> impl Iterator<Item = (u64, WalkerPose)> + 'a {
        self.bodies
            .iter()
            .map(move |(&id, body)| (id, body.pose(physics)))
    }

    pub(crate) fn step(&mut self, world: &mut CollisionWorld) {
        for body in self.bodies.values_mut() {
            body.step(world);
            self.vehicle_impacts += u64::from(body.struck);
        }
    }

    pub fn sync(world: &mut CollisionWorld, targets: impl IntoIterator<Item = WalkerTarget>) {
        let mut walkers = std::mem::take(&mut world.walking_bodies);
        walkers.generation += 1;
        for target in targets {
            let feet = Vector::from_array(target.drawn);
            if let Some(walker) = walkers.bodies.get_mut(&target.id) {
                if walker.target.model == target.model
                    && target.scale == walker.target.scale
                    && Vector::from_array(walker.target.feet)
                        .distance_squared(Vector::from_array(target.feet))
                        < 36.0
                {
                    walker.target = target;
                    walker.seen = walkers.generation;
                    continue;
                }
                world.physics.remove_body(walker.handle);
                walkers.bodies.remove(&target.id);
            }
            if !world.is_resident(feet.x.into(), feet.z.into()) {
                continue;
            }
            let extent = capsule::CENTER_FROM_FEET_M as f32 * target.scale;
            let centre = feet + Vector::Y * extent;
            // Includes the distance a vehicle can cover in the next fixed tick.
            let bounds = Aabb::from_half_extents(centre, Vector::new(2.0, extent + 0.4, 2.0));
            let vehicles = QueryFilter::default().groups(InteractionGroups::new(
                Group::ALL,
                layer::VEHICLE,
                InteractionTestMode::And,
            ));
            if world
                .physics
                .intersect_aabb_conservative(bounds, vehicles)
                .all(|(_, c)| !c.compute_aabb().intersects(&bounds))
            {
                continue;
            }
            let at = glam::DVec3::from(target.drawn.map(f64::from));
            if world
                .raycast(
                    at + glam::DVec3::Y * 0.30,
                    glam::DVec3::NEG_Y,
                    0.65,
                    layer::WORLD | layer::STREET | layer::BOAT,
                    None,
                )
                .is_none()
            {
                continue;
            }
            let heading = Rotation::from_rotation_y(target.heading);
            let handle = world.physics.insert_body(
                RigidBodyBuilder::dynamic()
                    .additional_solver_iterations(6)
                    .pose(Pose::from_parts(centre, heading))
                    .linvel(target.travel())
                    // Anticipate the first road contact as the capsule rotates after a hit.
                    .soft_ccd_prediction(capsule::RADIUS_M as f32 * target.scale)
                    .ccd_enabled(true)
                    .build(),
            );
            let collider = ColliderBuilder::capsule_y(
                capsule::HALF_HEIGHT_M as f32 * target.scale,
                capsule::RADIUS_M as f32 * target.scale,
            )
            .mass(75.0 * target.scale.powi(3))
            .friction(0.32)
            .restitution(0.0)
            .collision_groups(InteractionGroups::new(
                layer::PEDESTRIAN,
                layer::WORLD
                    | layer::STREET
                    | layer::VEHICLE
                    | layer::BOAT
                    | layer::CHARACTER
                    | layer::PEDESTRIAN,
                InteractionTestMode::And,
            ));
            let collider = world.physics.insert_collider(collider, Some(handle));
            walkers.bodies.insert(
                target.id,
                WalkingBody {
                    handle,
                    collider,
                    target,
                    seen: walkers.generation,
                    unbalanced: 0,
                    struck: false,
                    phase: target.phase,
                    walking_speed: target.travel().length(),
                },
            );
        }
        walkers.bodies.retain(|_, walker| {
            if walker.seen == walkers.generation {
                return true;
            }
            world.physics.remove_body(walker.handle);
            false
        });
        world.walking_bodies = walkers;
    }
}
