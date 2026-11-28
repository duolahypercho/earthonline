//! Four-wheel rigid-body vehicles on the fixed simulation clock.
//! Fleet vehicles use axle-specific suspension, Ackermann steering, geared
//! engine torque and load-limited tire impulses. The catalogue sedan retains
//! its original drive envelope, rack steering and Rapier side friction.

mod fleet;
mod tires;
mod water;
mod peer;
pub use peer::resolve_peer_contact;
pub(crate) use peer::approaching_traffic;
pub use fleet::{Transmission, VehicleConfig, VehicleModel};

use glam::{DMat3, DQuat, DVec3};
use rapier3d::control::{DynamicRayCastVehicleController, WheelTuning};
use rapier3d::prelude::*;

use crate::tuning;
use crate::world::{CollisionWorld, layer};

/// Dimensions, loaded mass properties and mechanical settings for one chassis.
#[derive(Clone, Copy, Debug)]
pub struct VehicleSpec {
    pub config: VehicleConfig,
    powertrain: Option<&'static fleet::Powertrain>,
    pub com_offset_z: f64,
    pub length: f64,
    pub width: f64,
    pub height: f64,
    pub wheelbase: f64,
    pub track_front: f64,
    pub track_rear: f64,
    pub wheel_radius: f64,
    pub front_axle_z: f64,
    pub rear_axle_z: f64,
    pub sill_y: f64,
    pub mass_kg: f64,
    pub curb_mass_kg: f64,
    pub(crate) curb_front_share: f64,
    pub(crate) front_torque_share: f64,
    pub com_offset_y: f64,
    pub yaw_inertia: f64,
    pub pitch_inertia: f64,
    pub roll_inertia: f64,
    pub rolling_resistance_n: f64,
    pub aero_factor: f64,
    pub engine_brake_force_n: f64,
    pub top_speed_mps: f64,
    pub reverse_top_speed_mps: f64,
    /// After the arcade scale and the 8 m/s^2 traction ceiling.
    pub peak_tractive_force_n: f64,
    pub engine_power_w: f64,
    pub corner_speed_mps: f64,
    pub reverse_tractive_force_n: f64,
    pub service_brake_decel_mps2: f64,
    pub tractive_scale: f64,
    pub braking_scale: f64,
    pub fade_exponent: f64,
    pub tyre_friction: f64,
    pub suspension_stiffness: f64,
    pub suspension_compression: f64,
    pub suspension_relaxation: f64,
    pub handbrake_decel_mps2: f64,
    pub max_suspension_force: f64,
    pub max_suspension_travel: f64,
    pub max_steer_rad: f64,
}

impl VehicleSpec {
    pub fn sedan() -> Self {
        // vehicle-catalogue.js `sedan`, class `car` dynamics; arcade-handling.js.
        let mass: f64 = 1600.0;
        // CLASS_DRIVING_DYNAMICS.car: peakTractiveAccelMps2 5.6, reverseAccelFraction 0.45;
        // the power is solved from the 60 m/s top speed (`vehicleDrivingDynamics`).
        let base_peak = mass * 5.6;
        let base_power = 99247.38180000002;
        let tractive_scale = 2.0;
        let accel_ceiling = 8.0; // traffic-vehicle-physics.js maximumAccelerationMps2 for a car
        let peak = (base_peak * tractive_scale).min(mass * accel_ceiling * tractive_scale);
        let power = base_power * tractive_scale;
        Self {
            config: VehicleConfig::default(), powertrain: None, com_offset_z: 0.0,
            length: 4.85,
            width: 1.84,
            height: 1.45,
            wheelbase: 2.82,
            track_front: 1.58,
            track_rear: 1.57,
            wheel_radius: 0.34,
            front_axle_z: 1.465,
            rear_axle_z: -1.355,
            sill_y: 0.205,
            mass_kg: mass,
            curb_mass_kg: mass,
            curb_front_share: 0.5,
            front_torque_share: 0.0,
            com_offset_y: -0.1885,
            yaw_inertia: 2798.4424,
            pitch_inertia: 2733.333333333333,
            roll_inertia: 585.3973333333334,
            rolling_resistance_n: 203.97832,
            aero_factor: 0.402817975,
            engine_brake_force_n: 1840.0,
            top_speed_mps: 60.0,
            reverse_top_speed_mps: 11.0,
            peak_tractive_force_n: peak,
            engine_power_w: power,
            corner_speed_mps: power / peak,
            reverse_tractive_force_n: (base_peak * 0.45 * tractive_scale).min(mass * accel_ceiling * tractive_scale),
            service_brake_decel_mps2: 9.5,
            tractive_scale,
            braking_scale: 1.6,
            fade_exponent: 4.0,
            tyre_friction: 1.3,
            suspension_stiffness: 22.0,
            suspension_compression: 1.4,
            suspension_relaxation: 2.3,
            handbrake_decel_mps2: 4.2,
            max_suspension_force: 120000.0,
            max_suspension_travel: 0.3,
            max_steer_rad: 0.5,
        }
    }

    /// Design gap `max(0.24, min(0.46, r * 0.94))`.
    pub fn design_gap(&self) -> f64 {
        (self.wheel_radius * 0.94).min(0.46).max(0.24)
    }
    /// Free length = design gap + static deflection `2.18 / k`.
    pub fn suspension_free_length(&self) -> f64 {
        self.design_gap() + 2.18 / self.suspension_stiffness
    }
    pub fn wheel_connection_height(&self) -> f64 {
        self.wheel_radius + self.design_gap() - self.height / 2.0
    }

    /// Chassis pose with the wheel datum at the requested point and the nose
    /// facing the requested map heading. Used for both spawn and fast travel.
    pub fn ground_pose(&self, world: &CollisionWorld, datum: DVec3, yaw: f64) -> (DVec3, DQuat) {
        // Starting horizontal on a grade buries one axle and leaves the
        // other beyond suspension travel. Probe only at the chosen level.
        let level = DQuat::from_rotation_y(yaw);
        let reach = tuning::kerb::GROUND_CONTACT_TOLERANCE_M;
        let hit = world.raycast(datum + DVec3::Y * reach, DVec3::NEG_Y, 2.0 * reach, layer::WORLD | layer::STREET, None);
        let rot = match hit {
            Some(hit) if hit.normal.y >= tuning::capsule::MAX_SLOPE_DEGREES.to_radians().cos() => {
                let up = hit.normal.normalize();
                let forward = ((level * DVec3::X).cross(up)).normalize();
                DQuat::from_mat3(&DMat3::from_cols(up.cross(forward), up, forward))
            }
            _ => level,
        };
        (datum + rot * (DVec3::Y * (self.height / 2.0)), rot)
    }

    /// `vehicleTractiveForceN` x the arcade power fade.
    pub fn tractive_force(&self, speed: f64, reverse: bool) -> f64 {
        if let Some(engine) = self.powertrain {
            return engine.wheel_force(speed, self.wheel_radius, reverse);
        }
        let s = speed.abs();
        if reverse {
            return if s >= self.reverse_top_speed_mps { 0.0 } else { self.reverse_tractive_force_n };
        }
        let f = if s <= self.corner_speed_mps { self.peak_tractive_force_n } else { self.engine_power_w / s };
        let share = (s / self.top_speed_mps).min(1.0);
        let fade = if self.tractive_scale > 1.0 { 1.0 - (1.0 - 1.0 / self.tractive_scale) * share.powf(self.fade_exponent) } else { 1.0 };
        f * fade
    }

    /// `vehicleResistanceForceN`.
    pub fn resistance_force(&self, speed: f64, throttle: f64) -> f64 {
        let s = speed.abs();
        let creep = 0.5;
        let ramp = (s / creep).clamp(0.0, 1.0);
        let engine_brake = if throttle.abs() > 0.02 { 0.0 } else { ((s - creep) / 3.0).clamp(0.0, 1.0) };
        self.rolling_resistance_n * ramp + self.aero_factor * s * s + self.engine_brake_force_n * engine_brake
    }

    pub fn max_brake_impulse_per_wheel(&self) -> f64 {
        self.mass_kg * self.service_brake_decel_mps2 * self.braking_scale * tuning::FIXED_STEP_S / 4.0
    }
    pub fn handbrake_impulse_per_rear_wheel(&self) -> f64 {
        self.mass_kg * self.handbrake_decel_mps2 * tuning::FIXED_STEP_S / 2.0
    }
}

/// `DRIVE_FEEL` (src/player/drive-feel.js), the values the port uses.
pub mod feel {
    pub const LATERAL_ACCEL_TARGET_MPS2: f64 = 8.5;
    pub const MAX_STEER_ANGLE_RAD: f64 = 0.5;
    pub const AUTHORITY_HOLD_SPEED_MPS: f64 = 17.5;
    pub const MIN_STEER_AUTHORITY: f64 = 0.12;
    pub const STEER_RISE_SECONDS: f64 = 0.16;
    pub const STEER_RELEASE_SECONDS: f64 = 0.125;
    pub const STEER_RACK_REFERENCE_WHEELBASE_M: f64 = 2.82;
    pub const STEER_RACK_WHEELBASE_EXPONENT: f64 = 0.6;
    pub const MAX_RANGE_SHIFT_SPEED_MPS: f64 = 0.25;
    pub const MAX_DELTA_SECONDS: f64 = 0.1;
}

/// The drive input shaper (`createDriveInputShaper` with counter-steer and
/// steering feel off).
#[derive(Clone, Debug)]
pub struct DriveShaper {
    pub wheelbase: f64,
    pub(crate) steer_normalized: f64, // pub(crate) for `Vehicle::state`
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Controls {
    pub throttle: f64,
    pub brake: f64,
    pub steer: f64,
    pub handbrake: bool,
}

impl DriveShaper {
    pub fn new(wheelbase: f64) -> Self {
        Self { wheelbase, steer_normalized: 0.0 }
    }

    pub fn authority_floor(&self) -> f64 {
        use feel::*;
        let held = (LATERAL_ACCEL_TARGET_MPS2 * self.wheelbase) / (AUTHORITY_HOLD_SPEED_MPS * AUTHORITY_HOLD_SPEED_MPS * MAX_STEER_ANGLE_RAD);
        MIN_STEER_AUTHORITY.clamp(0.0, 1.0).max(held).clamp(0.0, 1.0)
    }

    /// `steerAuthorityAtSpeed`.
    pub fn authority_at(&self, speed: f64) -> f64 {
        use feel::*;
        let s = speed.abs();
        if !(s > 1e-3) {
            return 1.0;
        }
        ((LATERAL_ACCEL_TARGET_MPS2 * self.wheelbase) / (s * s * MAX_STEER_ANGLE_RAD)).clamp(self.authority_floor(), 1.0)
    }

    /// `steerRampSeconds`: (rise, release).
    pub fn ramp_seconds(&self) -> (f64, f64) {
        use feel::*;
        let scale = (self.wheelbase / STEER_RACK_REFERENCE_WHEELBASE_M).powf(STEER_RACK_WHEELBASE_EXPONENT);
        (STEER_RISE_SECONDS * scale, STEER_RELEASE_SECONDS * scale)
    }

    pub fn update(&mut self, steer_in: f64, throttle_in: f64, brake_in: f64, handbrake: bool, analog: bool, speed: f64, delta: f64) -> Controls {
        let dt = delta.clamp(0.0, feel::MAX_DELTA_SECONDS);
        let steer_input = steer_in.clamp(-1.0, 1.0);
        let (rise, release) = self.ramp_seconds();
        let analog = analog || (steer_input != 0.0 && steer_input.abs() < 0.999);
        if analog {
            self.steer_normalized = steer_input;
        } else if steer_input != 0.0 {
            let crossing = self.steer_normalized != 0.0 && steer_input.signum() != self.steer_normalized.signum();
            let rate = if crossing { 1.0 / release } else { 1.0 / rise };
            let step = rate * dt;
            self.steer_normalized = if self.steer_normalized < steer_input { steer_input.min(self.steer_normalized + step) } else { steer_input.max(self.steer_normalized - step) };
        } else {
            let step = dt / release;
            self.steer_normalized = if self.steer_normalized > 0.0 { (self.steer_normalized - step).max(0.0) } else { (self.steer_normalized + step).min(0.0) };
        }
        self.steer_normalized = self.steer_normalized.clamp(-1.0, 1.0);
        let steer = (self.steer_normalized * self.authority_at(speed)).clamp(-1.0, 1.0);
        let brake = brake_in.clamp(0.0, 1.0);
        let throttle = if brake > 0.0 { 0.0 } else { throttle_in.clamp(0.0, 1.0) };
        Controls { throttle, brake, steer, handbrake }
    }
}

/// One car in the world.
pub struct Vehicle {
    transmission: Transmission,
    flood_ticks: u16,
    pub spec: VehicleSpec,
    pub body: RigidBodyHandle,
    pub collider: ColliderHandle,
    pub controller: DynamicRayCastVehicleController,
    pub shaper: DriveShaper,
    pub controls: Controls,
}

/// Wheel roles in catalogue order: front-left, front-right, rear-left, rear-right.
pub const DRIVEN: [bool; 4] = [false, false, true, true];

impl Vehicle {
    /// Spawn with the road datum (bottom of the wheels) at `datum`, nose along `yaw`
    /// (forward = (sin yaw, 0, cos yaw)).
    pub fn spawn(world: &mut CollisionWorld, spec: VehicleSpec, datum: DVec3, yaw: f64) -> Self {
        let pose = spec.ground_pose(world, datum, yaw);
        let offset = DVec3::Y * (spec.sill_y / 2.0);
        let half = DVec3::new(spec.width, spec.height - spec.sill_y, spec.length) / 2.0;
        Self::spawn_chassis(world, spec, pose, offset, half)
    }

    pub(crate) fn spawn_chassis(world: &mut CollisionWorld, spec: VehicleSpec, pose: (DVec3, DQuat), offset: DVec3, half: DVec3) -> Self {
        let (center, rot) = pose;
        let body = RigidBodyBuilder::dynamic()
            .pose(Pose::from_parts(Vector::new(center.x as f32, center.y as f32, center.z as f32), Rotation::from_array(rot.as_quat().to_array())))
            .ccd_enabled(true)
            .additional_mass_properties(MassProperties::new(
                Vector::new(0.0, spec.com_offset_y as f32, spec.com_offset_z as f32),
                spec.mass_kg as f32,
                Vector::new(spec.pitch_inertia as f32, spec.yaw_inertia as f32, spec.roll_inertia as f32),
            ))
            .build();
        let body = world.physics.insert_body(body);
        let collider = ColliderBuilder::cuboid(half.x as f32, half.y as f32, half.z as f32)
            .translation(Vector::from_array(offset.as_vec3().to_array()))
            .density(0.0)
            .friction(0.4)
            .restitution(0.05)
            .collision_groups(InteractionGroups::new(layer::VEHICLE, layer::WORLD | layer::STREET | layer::VEHICLE | layer::CHARACTER | layer::BOAT | layer::PEDESTRIAN, InteractionTestMode::And))
            .build();
        let collider = world.physics.insert_collider(collider, Some(body));
        // Mass properties are otherwise only computed on the next world step,
        // and the wheel solve before it would divide by a zero mass.
        let (bodies, colliders) = (&mut world.physics.bodies, &world.physics.colliders);
        bodies[body].recompute_mass_properties_from_colliders(colliders);
        let mut controller = DynamicRayCastVehicleController::new(body);
        controller.index_up_axis = 1;
        controller.index_forward_axis = 2;
        let tuning = WheelTuning {
            suspension_stiffness: spec.suspension_stiffness as f32,
            suspension_compression: spec.suspension_compression as f32,
            suspension_damping: spec.suspension_relaxation as f32,
            max_suspension_travel: spec.max_suspension_travel as f32,
            side_friction_stiffness: 1.0,
            friction_slip: spec.tyre_friction as f32,
            max_suspension_force: spec.max_suspension_force as f32,
        };
        let y = spec.wheel_connection_height() as f32;
        for (x, z) in [
            (spec.track_front / 2.0, spec.front_axle_z),
            (-spec.track_front / 2.0, spec.front_axle_z),
            (spec.track_rear / 2.0, spec.rear_axle_z),
            (-spec.track_rear / 2.0, spec.rear_axle_z),
        ] {
            // Rapier expresses stiffness relative to chassis mass. A payload
            // must compress the same spring, not make it stiffer.
            let mut axle_tuning = tuning;
            let mut rest = spec.suspension_free_length();
            if spec.powertrain.is_some() {
                let curb_fraction = spec.curb_mass_kg / spec.mass_kg;
                let share = if z == spec.front_axle_z { spec.curb_front_share } else { 1.0 - spec.curb_front_share };
                let per_wheel = 0.5 * share;
                let stiffness = spec.suspension_stiffness * per_wheel * 4.0;
                axle_tuning.suspension_stiffness = (stiffness * curb_fraction) as f32;
                axle_tuning.suspension_compression *= (curb_fraction * per_wheel * 4.0) as f32;
                axle_tuning.suspension_damping *= (curb_fraction * per_wheel * 4.0) as f32;
                rest = spec.design_gap() + tuning::GRAVITY_MPS2 * per_wheel / stiffness;
            }
            controller.add_wheel(Vector::new(x as f32, y, z as f32), Vector::new(0.0, -1.0, 0.0), Vector::new(-1.0, 0.0, 0.0), rest as f32, spec.wheel_radius as f32, &axle_tuning);
        }
        Self { transmission: Transmission::default(), flood_ticks: 0, spec, body, collider, controller, shaper: DriveShaper::new(spec.wheelbase), controls: Controls::default() }
    }

    pub fn pose(&self, world: &CollisionWorld) -> (DVec3, DQuat) {
        let b = &world.physics.bodies[self.body];
        let p = b.translation();
        let r = b.rotation();
        (DVec3::new(p.x as f64, p.y as f64, p.z as f64), DQuat::from_xyzw(r.x as f64, r.y as f64, r.z as f64, r.w as f64))
    }

    /// Road datum under the body (centre minus half the body height along its up).
    pub fn datum(&self, world: &CollisionWorld) -> DVec3 {
        let (p, r) = self.pose(world);
        p - r * DVec3::new(0.0, self.spec.height / 2.0, 0.0)
    }

    pub fn yaw(&self, world: &CollisionWorld) -> f64 {
        let (_, r) = self.pose(world);
        let f = r * DVec3::Z;
        f.x.atan2(f.z)
    }

    pub fn forward_speed(&self, world: &CollisionWorld) -> f64 {
        let b = &world.physics.bodies[self.body];
        let v = b.linvel();
        let (_, r) = self.pose(world);
        let f = r * DVec3::Z;
        v.x as f64 * f.x + v.y as f64 * f.y + v.z as f64 * f.z
    }

    pub fn wheels_in_contact(&self) -> usize {
        self.controller.wheels().iter().filter(|w| w.raycast_info().is_in_contact).count()
    }

    /// Range changes are a driver action. A request made in motion is discarded.
    pub fn toggle_drive_direction(&mut self, world: &CollisionWorld) {
        let velocity = world.physics.bodies[self.body].linvel();
        if velocity.length() <= feel::MAX_RANGE_SHIFT_SPEED_MPS as f32 {
            self.transmission.reverse_selected = !self.transmission.reverse_selected;
            self.transmission.gear = 0;
            if let Some(drive) = self.spec.powertrain {
                self.transmission.shift_left = drive.shift_duration_ticks;
            }
        }
    }

    pub fn reverse_selected(&self) -> bool {
        self.transmission.reverse_selected
    }

    /// Submit this step's controls and run the wheel solve (before the world step),
    /// using dry calm road conditions.
    pub fn pre_step(&mut self, world: &mut CollisionWorld, steer_in: f64, throttle_in: f64, brake_in: f64, handbrake: bool, analog: bool) {
        self.pre_step_with_weather(world, steer_in, throttle_in, brake_in, handbrake, analog, &crate::weather::WeatherPhysics::default());
    }

    /// Submit this step's controls and run the wheel solve under `weather`
    /// (wet-pavement grip, standing-puddle hydroplaning and splash drag, and
    /// aerodynamic wind / crosswind forces). Dry/calm weather is an exact no-op.
    #[allow(clippy::too_many_arguments)]
    pub fn pre_step_with_weather(
        &mut self,
        world: &mut CollisionWorld,
        steer_in: f64,
        throttle_in: f64,
        brake_in: f64,
        handbrake: bool,
        analog: bool,
        weather: &crate::weather::WeatherPhysics,
    ) {
        if !world.physics.bodies[self.body].is_enabled() { world.physics.bodies[self.body].set_enabled(true); }
        let speed = self.forward_speed(world);
        let dt = tuning::FIXED_STEP_S;
        let mut c = self.shaper.update(steer_in, throttle_in, brake_in, handbrake, analog, speed, dt);
        if self.transmission.reverse_selected {
            c.throttle = -c.throttle;
        }
        if analog && self.spec.powertrain.is_some() {
            // A steering-axis command requests an actual rack angle. Keyboard
            // steering retains the ramp and high-speed assistance.
            c.steer = self.shaper.steer_normalized;
        }
        self.controls = c;
        if self.spec.powertrain.is_some() {
            let mut road = c;
            if self.displace_water(world) { road.throttle = 0.0; }
            self.road_step(world, road, weather, speed);
            return;
        }
        let s = &self.spec;
        let demand = if c.handbrake { 0.0 } else { c.throttle };
        let reverse = demand < 0.0;
        let tractive = if demand.abs() < 1e-6 { 0.0 } else { s.tractive_force(speed, reverse) * demand.abs() };
        let resistance = s.resistance_force(speed, demand);
        let resistance_per_wheel = resistance * dt / 4.0;
        let service = c.brake * s.max_brake_impulse_per_wheel();
        let front = resistance_per_wheel + service;
        let rear = if c.handbrake { resistance_per_wheel + service.max(s.handbrake_impulse_per_rear_wheel()) } else { front };
        // Per driven wheel: half the tractive force, netted against that wheel's brake.
        let engine_per_wheel = (if reverse { -tractive } else { tractive }) / 2.0;
        let drive_impulse = engine_per_wheel * dt;
        let steer = (c.steer * s.max_steer_rad) as f32;
        // A SLEEPING CHASSIS MUST NOT BE DRIVEN (catalogue-vehicle-controller.js
        // `preStep`): the wheel solve would bank velocity on a body the solver
        // no longer integrates. An undemanded sleeping car is left alone; the
        // first frame of demand wakes it.
        let demanded = c.throttle.abs() > 0.0 || c.brake > 0.0 || steer != 0.0;
        if demanded {
            world.physics.bodies[self.body].wake_up(true);
        } else if world.physics.bodies[self.body].is_sleeping() {
            return;
        }
        let dry_calm = weather.is_dry_calm();
        let (pos, rot) = self.pose(world);
        let vel = {
            let v = world.physics.bodies[self.body].linvel();
            DVec3::new(v.x as f64, v.y as f64, v.z as f64)
        };
        let speed_mag = vel.x.hypot(vel.z);
        for (i, w) in self.controller.wheels_mut().iter_mut().enumerate() {
            w.steering = if i < 2 { steer } else { 0.0 };
            let mut extra_drag_imp = 0.0;
            if dry_calm {
                w.friction_slip = s.tyre_friction as f32;
                w.side_friction_stiffness = 1.0;
            } else {
                let cp = w.chassis_connection_point_cs;
                let wp = pos + rot * DVec3::new(cp.x as f64, cp.y as f64, cp.z as f64);
                let (grip_s, lat_s) = weather.tyre_grip_scale(wp.x, wp.z, speed_mag);
                w.friction_slip = (s.tyre_friction * grip_s) as f32;
                w.side_friction_stiffness = lat_s as f32;
                if w.raycast_info().is_in_contact {
                    let pud = weather.puddle_depth_at(wp.x, wp.z);
                    extra_drag_imp = weather.wheel_puddle_drag_n(speed_mag, pud) * dt;
                }
            }
            if DRIVEN[i] {
                let brake = if i < 2 { front } else { rear };
                let driven_engine = drive_impulse.signum() * (drive_impulse.abs() - brake).max(0.0) / dt;
                let driven_brake = (brake - drive_impulse.abs()).max(0.0);
                w.engine_force = driven_engine as f32;
                w.brake = (driven_brake + extra_drag_imp) as f32;
            } else {
                w.engine_force = 0.0;
                w.brake = (if i < 2 { front } else { rear } + extra_drag_imp) as f32;
            }
        }
        if !dry_calm {
            let fwd = rot * DVec3::Z;
            let f_wind = weather.vehicle_wind_force_n(vel, fwd, s.width, s.height, s.length);
            if f_wind.length_squared() > 1e-6 {
                let imp = f_wind * dt;
                world.physics.bodies[self.body].apply_impulse(Vector::new(imp.x as f32, 0.0, imp.z as f32), false);
            }
        }
        let filter = QueryFilter::default()
            .groups(InteractionGroups::new(layer::VEHICLE, layer::WORLD | layer::STREET, InteractionTestMode::And))
            .predicate(&crate::world::wheel_supports_contact)
            .exclude_rigid_body(self.body);
        let p = &mut world.physics;
        let queries = p.broad_phase.as_query_pipeline_mut(p.narrow_phase.query_dispatcher(), &mut p.bodies, &mut p.colliders, filter);
        self.controller.update_vehicle(dt as f32, queries);
        self.hold_brakes(world, c, weather, speed);
    }

    /// No driver: brakes on, no drive (a parked car).
    pub fn park(&mut self, world: &mut CollisionWorld) {
        self.park_with_weather(world, &Default::default());
    }

    /// An unoccupied car receives the road grip and wind of its current world.
    pub fn park_with_weather(&mut self, world: &mut CollisionWorld, weather: &crate::weather::WeatherPhysics) {
        let at = self.datum(world);
        let margin = 0.5 * self.spec.length.hypot(self.spec.width);
        let resident = [-margin, margin].into_iter().all(|dx| {
            [-margin, margin].into_iter().all(|dz| world.is_resident(at.x + dx, at.z + dz))
        });
        if !resident {
            // Freeze outside the simulated area. The body's pose and velocity
            // survive while the collision stream has no supporting road.
            world.physics.bodies[self.body].set_enabled(false);
            return;
        }
        self.pre_step_with_weather(world, 0.0, 0.0, 0.0, true, false, weather);
    }

    /// the car between steps: the chassis body (pose, velocities, sleep)
    /// and the shaper. (The wheel solve recomputes its contacts every step.)
    pub fn state(&self, world: &CollisionWorld) -> VehicleState {
        let b = &world.physics.bodies[self.body];
        let (p, r, v, w) = (b.translation(), b.rotation(), b.linvel(), b.angvel());
        VehicleState {
            config: self.spec.config,
            flood_ticks: self.flood_ticks,
            transmission: self.transmission,
            translation: [p.x, p.y, p.z],
            rotation: [r.x, r.y, r.z, r.w],
            linvel: [v.x, v.y, v.z],
            angvel: [w.x, w.y, w.z],
            sleeping: b.is_sleeping(),
            time_since_can_sleep: b.activation().time_since_can_sleep,
            steer_normalized: self.shaper.steer_normalized,
            controls: [self.controls.throttle, self.controls.brake, self.controls.steer],
            handbrake: self.controls.handbrake,
        }
    }

    /// put the car back in a state `state` returned.
    pub fn set_state(&mut self, world: &mut CollisionWorld, s: &VehicleState) {
        self.transmission = s.transmission;
        self.flood_ticks = s.flood_ticks.min(water::FLOOD_TICKS);
        let b = &mut world.physics.bodies[self.body];
        let t = s.translation;
        let r = s.rotation;
        b.set_position(Pose::from_parts(Vector::new(t[0], t[1], t[2]), Rotation::from_xyzw(r[0], r[1], r[2], r[3])), false);
        b.set_linvel(Vector::new(s.linvel[0], s.linvel[1], s.linvel[2]), false);
        b.set_angvel(Vector::new(s.angvel[0], s.angvel[1], s.angvel[2]), false);
        if s.sleeping {
            b.sleep();
        } else {
            b.wake_up(true);
            b.activation_mut().time_since_can_sleep = s.time_since_can_sleep;
        }
        self.shaper.steer_normalized = s.steer_normalized;
        self.controls = Controls { throttle: s.controls[0], brake: s.controls[1], steer: s.controls[2], handbrake: s.handbrake };
    }
}

/// `Vehicle::state` (plain data).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VehicleState {
    pub transmission: Transmission,
    pub flood_ticks: u16,
    pub config: VehicleConfig,
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub linvel: [f32; 3],
    pub angvel: [f32; 3],
    pub sleeping: bool,
    pub time_since_can_sleep: f32,
    pub steer_normalized: f64,
    pub controls: [f64; 3],
    pub handbrake: bool,
}
