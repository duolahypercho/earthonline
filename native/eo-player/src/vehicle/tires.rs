//! Four contact patches. Positive forward is +z, and every force applied at
//! the road has a moment arm about the actual loaded centre of mass.
use super::{Controls, Vehicle};
use crate::world::{CollisionWorld, layer};
use crate::{WeatherPhysics, tuning};
use rapier3d::control::Wheel;
use rapier3d::prelude::*;

#[derive(Clone, Copy)]
struct BrakePatch {
    point: Vector,
    ground: Option<RigidBodyHandle>,
    forward: Vector,
    side: Vector,
    grip: [f32; 2],
    brake_limit: f32,
    effective_mass: [f32; 2],
}

impl BrakePatch {
    fn effective_mass(
        body: &RigidBody,
        ground: Option<&RigidBody>,
        at: Vector,
        direction: Vector,
    ) -> f32 {
        let inverse = |body: &RigidBody| {
            let props = body.mass_properties();
            let arm = (at - body.center_of_mass()).cross(direction);
            direction.dot(props.effective_inv_mass * direction)
                + arm.dot(props.effective_world_inv_inertia * arm)
        };
        let inverse_mass = inverse(body) + ground.map_or(0.0, inverse);
        // A degenerate contact tangent cannot transmit an impulse.
        if inverse_mass > 0.0 { inverse_mass.recip() } else { 0.0 }
    }

    fn solve(
        &self,
        wheel: &mut Wheel,
        bodies: &mut RigidBodySet,
        chassis: RigidBodyHandle,
        gravity: Vector,
    ) {
        let ground_velocity = self
            .ground
            .map_or(Vector::ZERO, |h| bodies[h].velocity_at_point(self.point));
        let relative = bodies[chassis].velocity_at_point(self.point) + gravity - ground_velocity;
        let x = (wheel.forward_impulse - relative.dot(self.forward) * self.effective_mass[0])
            .clamp(-self.brake_limit, self.brake_limit);
        let y = wheel.side_impulse - relative.dot(self.side) * self.effective_mass[1];
        let ellipse = (x / self.grip[0]).hypot(y / self.grip[1]).max(1.0);
        let (x, y) = (x / ellipse, y / ellipse);
        let change =
            self.forward * (x - wheel.forward_impulse) + self.side * (y - wheel.side_impulse);
        bodies[chassis].apply_impulse_at_point(change, self.point, false);
        if let Some(h) = self.ground
            && bodies[h].is_dynamic()
        {
            bodies[h].apply_impulse_at_point(-change, self.point, true);
        }
        wheel.forward_impulse = x;
        wheel.side_impulse = y;
    }
}

// The tyre impulse acts at the beginning of the tick. Use the mean velocity
// over the semi-implicit substeps to predict the contact patch displacement.
fn mean_gravity_velocity(physics: &PhysicsWorld) -> Vector {
    let steps = physics.integration_parameters.num_solver_iterations as f32;
    physics.gravity * (physics.integration_parameters.dt * (steps + 1.0) / (2.0 * steps))
}

impl Vehicle {
    pub(super) fn road_step(
        &mut self,
        world: &mut CollisionWorld,
        c: Controls,
        weather: &WeatherPhysics,
        speed: f64,
    ) {
        let dt = tuning::FIXED_STEP_S as f32;
        let s = &self.spec;
        let drive = s.powertrain.expect("road vehicle has a powertrain");
        let chassis = &mut world.physics.bodies[self.body];
        // The solver must integrate every commanded impulse, including a held
        // service brake. Letting the chassis go to sleep between those impulses
        // makes the stop depend on the island's pre-checkpoint sleep timer.
        if c.throttle != 0.0 || c.steer != 0.0 || c.brake != 0.0 || !weather.is_dry_calm() {
            chassis.wake_up(true);
        } else if chassis.is_sleeping() {
            return;
        }
        let wind = weather.effective_wind();
        let air = chassis.linvel() - Vector::new(wind[0] as f32, 0.0, wind[1] as f32);
        chassis.apply_impulse(-air * (air.length() * s.aero_factor as f32 * dt), false);

        let rack = c.steer * s.max_steer_rad;
        for (i, w) in self.controller.wheels_mut().iter_mut().enumerate() {
            // The inside front wheel steers farther than the outside one.
            w.steering = if i < 2 && rack.abs() > 1e-6 {
                let side = if i == 0 { 0.5 } else { -0.5 };
                (s.wheelbase / (s.wheelbase / rack.tan() - side * s.track_front)).atan() as f32
            } else {
                0.0
            };
            w.engine_force = 0.0;
            w.brake = 0.0;
            w.side_friction_stiffness = 0.0;
        }
        let p = &mut world.physics;
        let f = QueryFilter::default()
            .predicate(&crate::world::wheel_supports_contact)
            .groups(InteractionGroups::new(
                layer::VEHICLE,
                layer::WORLD | layer::STREET | layer::BOAT,
                InteractionTestMode::And,
            ))
            .exclude_rigid_body(self.body);
        let queries = p.broad_phase.as_query_pipeline_mut(
            p.narrow_phase.query_dispatcher(),
            &mut p.bodies,
            &mut p.colliders,
            f,
        );
        self.controller.update_vehicle(dt, queries);
        let wheel_force = drive.step_force(
            speed,
            s.wheel_radius,
            self.transmission.reverse_selected,
            &mut self.transmission,
        );
        let force = if c.handbrake {
            0.0
        } else {
            c.throttle * wheel_force
        };
        let mut impulses = [(Vector::ZERO, Vector::ZERO, None, Vector::ZERO); 4];
        let gravity_step = mean_gravity_velocity(p);
        for (i, w) in self.controller.wheels_mut().iter_mut().enumerate() {
            let hit = *w.raycast_info();
            let Some(collider) = hit.ground_object else {
                continue;
            };
            let other = p.colliders[collider].parent();
            let at = hit.contact_point_ws;
            let n = hit.contact_normal_ws;
            let side = (w.axle() - n * w.axle().dot(n)).normalize_or_zero();
            let forward = n.cross(side).normalize_or_zero();
            let ground_velocity = other.map_or(Vector::ZERO, |h| p.bodies[h].velocity_at_point(at));
            let relative =
                p.bodies[self.body].velocity_at_point(at) + gravity_step - ground_velocity;
            let longitudinal = relative.dot(forward);
            let lateral = relative.dot(side);
            let load = w.wheel_suspension_force.min(w.max_suspension_force);
            let (mu_x, mu_y) = weather.tyre_grip_scale(at.x.into(), at.z.into(), speed.abs());
            let grip = s.tyre_friction as f32 * load * dt;
            // Allocate low-speed stopping impulses by normal load. This also
            // holds a parked vehicle on a slope within static tire friction.
            let supported_mass = load / tuning::GRAVITY_MPS2 as f32;
            let axle_share = if i < 2 { s.front_torque_share } else { 1.0 - s.front_torque_share };
            let wheel_share = 0.5 * axle_share;
            let mut retarding =
                0.013 * load + c.brake as f32 * s.service_brake_decel_mps2 as f32 * supported_mass;
            if i >= 2 && c.handbrake {
                retarding += grip / dt;
            }
            if c.throttle == 0.0 {
                retarding += (s.engine_brake_force_n * wheel_share * (speed.abs() / 3.0).min(1.0)) as f32;
            }
            retarding += weather.wheel_puddle_drag_n(
                speed.abs(),
                weather.puddle_depth_at(at.x.into(), at.z.into()),
            ) as f32;
            let drive_j = force as f32 * dt * wheel_share as f32;
            let mut jx =
                drive_j - (longitudinal * supported_mass).clamp(-retarding * dt, retarding * dt);
            // Cornering stiffness expressed per unit normal load. At walking
            // speed limit the impulse to that required to arrest lateral slip.
            let slip_angle = lateral.atan2(longitudinal.abs().max(0.1));
            let mut jy = -(lateral * supported_mass)
                .abs()
                .min(load * 8.0 * slip_angle.abs() * dt)
                * lateral.signum();
            let ellipse = (jx / (grip * mu_x as f32).max(1e-7))
                .hypot(jy / (grip * mu_y as f32).max(1e-7))
                .max(1.0);
            jx /= ellipse;
            jy /= ellipse;
            // Also supplies the low-speed, coupled brake solve below.
            w.brake = retarding * dt;
            w.forward_impulse = jx;
            w.side_impulse = jy;
            impulses[i] = (at, forward * jx + side * jy, other, n * (load * dt));
        }
        for (at, impulse, other, spring) in impulses {
            p.bodies[self.body].apply_impulse_at_point(impulse, at, false);
            if let Some(h) = other
                && p.bodies[h].is_dynamic()
            {
                p.bodies[h].apply_impulse_at_point(-impulse - spring, at, true);
            }
        }
        self.hold_brakes(world, c, weather, speed);
    }

    pub(super) fn hold_brakes(
        &mut self,
        world: &mut CollisionWorld,
        c: Controls,
        weather: &WeatherPhysics,
        speed: f64,
    ) {
        if c.brake == 0.0 && !c.handbrake {
            return;
        }
        let p = &mut world.physics;
        let chassis = &p.bodies[self.body];
        let gravity_step = mean_gravity_velocity(p);
        let mut patches = [None; 4];
        for (patch, wheel) in patches.iter_mut().zip(self.controller.wheels()) {
            let hit = wheel.raycast_info();
            let Some(collider) = hit.ground_object else {
                continue;
            };
            let ground = p.colliders[collider].parent();
            let body = ground.map(|h| &p.bodies[h]);
            let at = hit.contact_point_ws;
            let relative = chassis.velocity_at_point(at)
                - body.map_or(Vector::ZERO, |b| b.velocity_at_point(at));
            if relative.length_squared() >= 1.0 {
                continue;
            }
            let load = wheel.wheel_suspension_force.min(wheel.max_suspension_force);
            if load <= 0.0 {
                continue;
            }
            let n = hit.contact_normal_ws;
            let side = (wheel.axle() - n * wheel.axle().dot(n)).normalize_or_zero();
            let forward = n.cross(side).normalize_or_zero();
            let (mu_x, mu_y) = weather.tyre_grip_scale(at.x.into(), at.z.into(), speed.abs());
            let grip = self.spec.tyre_friction as f32 * load * p.integration_parameters.dt;
            *patch = Some(BrakePatch {
                point: at,
                ground,
                forward,
                side,
                grip: [
                    (grip * mu_x as f32).max(1e-7),
                    (grip * mu_y as f32).max(1e-7),
                ],
                brake_limit: wheel.brake,
                effective_mass: [forward, side]
                    .map(|dir| BrakePatch::effective_mass(chassis, body, at, dir)),
            });
        }
        if patches.iter().all(Option::is_none) {
            return;
        }
        // A locked wheel resists motion at its contact point. An impulse under
        // one axle also pitches the other, so solve the four patches together.
        for _ in 0..p.integration_parameters.num_solver_iterations {
            for (wheel, patch) in self.controller.wheels_mut().iter_mut().zip(&patches) {
                if let Some(patch) = patch {
                    patch.solve(wheel, &mut p.bodies, self.body, gravity_step);
                }
            }
        }
    }
}
