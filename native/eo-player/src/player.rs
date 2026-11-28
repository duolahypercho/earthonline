//! The player: on foot or driving, stepped at the fixed 60 Hz of the JS
//! physics clock, with the walk camera or the drive camera on top.
//!
//! On foot this is the per-step body of main.js's walk update: the stick
//! through the camera yaw (nine-way), the walk / run / sprint tiers and the
//! sprint build (`PLAYER_SPRINT_*`), crouch/aim speed, momentum, the swept
//! capsule, momentum settle, facing. Driving is the catalogue car with the
//! drive-feel shaper. Enter/exit uses the vehicle interaction range and the
//! door station of the entry layout.
//!
//! Everything here is deterministic for a given input sequence: fixed step,
//! no wall-clock reads, Rapier with `enhanced-determinism`. A server can run
//! `fixed_step` with the same inputs and get the same body.

use glam::{DQuat, DVec3};
use rapier3d::prelude::*;

use crate::camera::{self, CameraPose, CameraProbe, DriveCamera, DriveCameraVehicle, OnFootCamera, OnFootInput, ProbeHit};
use crate::character::Character;
use crate::input::{Action, InputFrame};
use crate::momentum::{self, Momentum};
use crate::tuning::{self, speed};
use crate::vehicle::{Vehicle, VehicleConfig};
use crate::world::{CollisionWorld, layer};

/// One fixed step's commands (what a network peer would send).
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Default)]
pub struct StepInput {
    /// x right, y forward; keyboard gives -1/0/1 per axis, a stick its curve.
    pub move_x: f64,
    pub move_y: f64,
    pub sprint: bool,
    pub crouch: bool,
    pub jump: bool,
    pub aim: bool,
    pub vehicle_toggle: bool,
    #[serde(default)]
    pub shift_direction: bool,
    pub throttle: f64,
    pub brake: f64,
    /// Positive left.
    pub steer: f64,
    pub analog_steer: bool,
    pub handbrake: bool,
    pub look_behind: bool,
    /// A tap of the interact action: ride a landmark lift from its stop (lift.rs).
    pub interact: bool,
}

impl StepInput {
    pub fn from_frame(f: &InputFrame) -> Self {
        Self {
            move_x: f.move_x,
            move_y: f.move_y,
            sprint: f.held(Action::Sprint),
            crouch: f.held(Action::Crouch),
            jump: f.tapped(Action::Jump),
            aim: f.held(Action::Aim),
            vehicle_toggle: f.tapped(Action::VehicleToggle),
            shift_direction: f.tapped(Action::ShiftDirection),
            throttle: f.throttle,
            brake: f.brake,
            steer: f.steer,
            analog_steer: f.analog_steer || (f.steer.abs() > 0.0 && f.steer.abs() < 0.999),
            handbrake: f.held(Action::Handbrake),
            look_behind: f.held(Action::LookBehind),
            interact: f.tapped(Action::Interact),
        }
    }
}

/// `Player::state` (plain data).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerState {
    pub character: crate::character::CharacterState,
    /// vx, vz, ax, az.
    pub momentum: [f64; 4],
    pub momentum_last: Option<(f64, f64)>,
    pub plant_yaw: Option<f64>,
    pub yaw: f64,
    pub pitch: f64,
    pub facing_yaw: f64,
    pub sprint_held_s: f64,
    pub sprint: f64,
    pub ground_speed: f64,
    /// The car being driven (index into `vehicles`), None on foot.
    pub driving: Option<u32>,
    pub ticks: u64,
    pub vehicles: Vec<crate::vehicle::VehicleState>,
    /// A landmark lift ride in progress.
    pub lift: Option<crate::lift::LiftRide>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    OnFoot,
    Driving(usize),
}

/// What the renderer needs.
#[derive(Clone, Copy, Debug)]
pub struct CameraView {
    pub eye: DVec3,
    pub forward: DVec3,
    pub fov_deg: f64,
    /// False when the boom is crushed into the body (reads as first person).
    pub body_visible: bool,
}

/// The camera probe against the collision world.
pub struct WorldProbe<'a> {
    pub world: &'a CollisionWorld,
    pub exclude: Option<RigidBodyHandle>,
    pub filter: Group,
}

impl CameraProbe for WorldProbe<'_> {
    fn probe(&mut self, origin: DVec3, direction: DVec3, max_distance: f64, radius: Option<f64>) -> Option<ProbeHit> {
        if !(max_distance > 0.0) {
            return None;
        }
        match radius {
            Some(r) => self.world.sphere_cast(origin, direction, max_distance, r, self.filter, self.exclude).map(|(d, v)| ProbeHit { distance: d, soft: v }),
            None => self.world.raycast(origin, direction, max_distance, self.filter, self.exclude).map(|h| ProbeHit { distance: h.distance, soft: h.vehicle }),
        }
    }
}

pub struct Player {
    pub character: Character,
    pub momentum: Momentum,
    /// Camera yaw, the JS convention: forward = (sin yaw, 0, cos yaw).
    pub yaw: f64,
    /// Positive looks down (the JS walk pitch).
    pub pitch: f64,
    pub facing_yaw: f64,
    pub sprint_held_s: f64,
    pub sprint: f64,
    pub ground_speed: f64,
    pub on_foot_cam: OnFootCamera,
    pub drive_cam: DriveCamera,
    pub vehicles: Vec<Vehicle>,
    pub spawn_config: VehicleConfig,
    pub mode: Mode,
    pub ticks: u64,
    /// A landmark lift ride in progress (lift.rs).
    pub lift: Option<crate::lift::LiftRide>,
    /// Active weather physics conditions (default dry & calm).
    pub weather: crate::weather::WeatherPhysics,
    accumulator: f64,
    pending_direction_shift: bool,
    last_camera_distance: f64,
}

impl Player {
    pub fn new(feet: DVec3, yaw: f64) -> Self {
        Self {
            character: Character::new(feet),
            momentum: Momentum::default(),
            yaw,
            pitch: 0.0,
            facing_yaw: yaw,
            sprint_held_s: 0.0,
            sprint: 0.0,
            ground_speed: 0.0,
            on_foot_cam: OnFootCamera::default(),
            drive_cam: DriveCamera::default(),
            vehicles: vec![],
            spawn_config: VehicleConfig::default(),
            mode: Mode::OnFoot,
            ticks: 0,
            lift: None,
            weather: crate::weather::WeatherPhysics::default(),
            accumulator: 0.0,
            pending_direction_shift: false,
            last_camera_distance: camera::on_foot::walk_distance(), // walk_distance()
        }
    }

    /// Update the weather physics applied during `fixed_step` (wet grip,
    /// standing puddles and wind forces).
    pub fn set_weather(&mut self, weather: crate::weather::WeatherPhysics) {
        self.weather = weather;
    }

    /// Park beside the walker, following the supporting floor to the requested spot.
    pub fn spawn_vehicle(&mut self, world: &mut CollisionWorld, x: f64, z: f64, yaw: f64) -> usize {
        let y = world.surface_beside(self.character.feet(), x, z);
        self.spawn_vehicle_on(world, DVec3::new(x, y, z), yaw)
    }

    /// Place a sedan on the surface nearest below `from` (x, reference
    /// height, z): `CollisionWorld::surface_below` (`surface_y` on one level).
    pub fn spawn_vehicle_below(&mut self, world: &mut CollisionWorld, from: DVec3, yaw: f64) -> usize {
        let y = world.surface_below(from.x, from.z, from.y);
        self.spawn_vehicle_on(world, DVec3::new(from.x, y, from.z), yaw)
    }

    fn spawn_vehicle_on(&mut self, world: &mut CollisionWorld, datum: DVec3, yaw: f64) -> usize {
        let v = Vehicle::spawn(world, self.spawn_config.spec(), datum, yaw);
        self.vehicles.push(v);
        self.vehicles.len() - 1
    }

    /// Where the body is: the feet on foot, the road datum when driving.
    pub fn position(&self, world: &CollisionWorld) -> DVec3 {
        match self.mode {
            Mode::OnFoot => self.character.feet(),
            Mode::Driving(i) => self.vehicles[i].datum(world),
        }
    }

    /// Look input (applied per drawn frame, as the JS does).
    pub fn look(&mut self, look_yaw: f64, look_pitch: f64) {
        match self.mode {
            Mode::OnFoot => {
                self.yaw = (self.yaw + look_yaw).sin().atan2((self.yaw + look_yaw).cos());
                self.pitch = (self.pitch + look_pitch).clamp(camera::WALK_PITCH_MIN, camera::WALK_PITCH_MAX);
            }
            Mode::Driving(_) => self.drive_cam.apply_look(look_yaw, look_pitch),
        }
    }

    /// Advance by a drawn frame's time: whole fixed steps, the remainder kept.
    /// Taps (jump, vehicle toggle) act on the first step only. Returns steps run.
    pub fn advance(&mut self, world: &mut CollisionWorld, input: &StepInput, frame_dt: f64) -> usize {
        self.accumulator += frame_dt.clamp(0.0, 0.25);
        let mut n = 0;
        self.pending_direction_shift |= input.shift_direction && matches!(self.mode, Mode::Driving(_));
        let mut i = *input;
        while self.accumulator >= tuning::FIXED_STEP_S - 1e-9 {
            self.accumulator -= tuning::FIXED_STEP_S;
            i.shift_direction = std::mem::take(&mut self.pending_direction_shift);
            self.fixed_step(world, &i);
            if !self.is_swimming(world) { i.jump = false; }
            i.vehicle_toggle = false;
            i.interact = false;
            n += 1;
        }
        n
    }

    /// One 1/60 s step of the player and the physics world.
    pub fn fixed_step(&mut self, world: &mut CollisionWorld, input: &StepInput) {
        let dt = tuning::FIXED_STEP_S;
        if !world.lockstep {
            let start = self.position(world);
            let (end, radius) = match self.mode {
                Mode::OnFoot => (start + self.character.support_velocity * dt, tuning::capsule::RADIUS_M + speed::SPRINT_MPS * dt),
                Mode::Driving(i) => {
                    let car = &self.vehicles[i];
                    let velocity = world.physics.bodies[car.body].linvel().to_array().map(f64::from);
                    (start + DVec3::from_array(velocity) * dt, car.spec.length.hypot(car.spec.width) * 0.5)
                }
            };
            world.prepare_support(start, end, radius);
        }
        // A landmark lift: the ride holds the body still; it changes stops in the dark.
        if self.lift.is_none() && input.interact && self.mode == Mode::OnFoot {
            if let Some((i, up)) = crate::lift::stop_near(&world.buildings.decks.lifts, self.character.feet()) {
                self.lift = Some(crate::lift::LiftRide { lift: i as u32, up, step: 0 });
                self.momentum.reset();
            }
        }
        if let Some(mut r) = self.lift {
            r.step += 1;
            if r.step == crate::lift::MOVE_STEP {
                if let Some(l) = world.buildings.decks.lifts.get(r.lift as usize) {
                    let (feet, yaw) = crate::lift::arrival(world, l, r.up);
                    self.character.teleport_feet(feet);
                    self.yaw = yaw;
                    self.facing_yaw = yaw;
                    self.on_foot_cam.seed(feet, self.last_camera_distance);
                }
            }
            self.momentum.reset();
            self.sprint_held_s = 0.0;
            self.sprint = 0.0;
            self.ground_speed = 0.0;
            self.lift = (r.step < crate::lift::RIDE_STEPS).then_some(r);
            for v in &mut self.vehicles {
                v.park_with_weather(world, &self.weather);
            }
            world.step_physics();
            self.ticks += 1;
            return;
        }
        if input.vehicle_toggle {
            match self.mode {
                Mode::OnFoot => {
                    let _ = self.try_enter(world);
                }
                Mode::Driving(_) => {
                    let _ = self.try_exit(world);
                }
            }
        }
        let mut floor = None;
        match self.mode {
            Mode::OnFoot => {
                if self.is_swimming(world) {
                    self.swim_step(world, input, dt);
                } else {
                    floor = self.walk_step(world, input, dt);
                }
                for v in &mut self.vehicles {
                    v.park_with_weather(world, &self.weather);
                }
            }
            Mode::Driving(i) => {
                let wx = self.weather;
                for (k, v) in self.vehicles.iter_mut().enumerate() {
                    if k == i {
                        if input.shift_direction && !input.vehicle_toggle { v.toggle_drive_direction(world); }
                        v.pre_step_with_weather(world, input.steer, input.throttle, input.brake, input.handbrake, input.analog_steer, &wx);
                    } else {
                        v.park_with_weather(world, &wx);
                    }
                }
            }
        }
        world.step_physics();
        if let Some(floor) = floor { floor.finish(&mut self.character, world); }
        if let Mode::Driving(i) = self.mode {
            // The body rides in the seat.
            let d = self.vehicles[i].datum(world);
            self.character.teleport_feet(d);
            self.momentum.reset();
            self.drive_cam.update(dt, input.look_behind);
        }
        self.ticks += 1;
    }

    pub fn is_swimming(&self, world: &CollisionWorld) -> bool {
        let feet = self.character.feet();
        self.mode == Mode::OnFoot && self.lift.is_none() && world.ground.water_column(feet.x, feet.z)
            .is_some_and(|(floor, surface)| surface - floor > 1.22 && feet.y <= surface - 1.09 && feet.y >= floor - 0.35)
    }

    fn swim_step(&mut self, world: &CollisionWorld, input: &StepInput, dt: f64) {
        let feet = self.character.feet();
        let (x, z) = momentum::stick_world(input.move_x, input.move_y * self.pitch.cos(), self.yaw);
        let vertical = f64::from(input.jump as u8) - f64::from(input.crouch as u8) - input.move_y * self.pitch.sin();
        let speed = if input.sprint { 2.3 } else { 1.35 };
        let mut delta = DVec3::new(x, vertical, z).clamp_length_max(1.) * speed * dt;
        if let Some((floor, surface)) = world.ground.water_column(feet.x + delta.x, feet.z + delta.z) {
            delta.y = (feet.y + delta.y).clamp(floor + 0.03, (surface - 1.10).max(floor + 0.03)) - feet.y;
        }
        self.character.swim(world, delta, dt);
        self.momentum.reset();
        self.sprint_held_s = 0.;
        self.sprint = 0.;
        self.ground_speed = self.character.feet().distance(feet) / dt;
        self.facing_yaw = self.yaw;
    }

    fn walk_step(&mut self, world: &CollisionWorld, input: &StepInput, dt: f64) -> Option<crate::character::FloorStep> {
        let (mut lx, mut lz) = momentum::stick_world(input.move_x, input.move_y, self.yaw);
        let m = lx.hypot(lz);
        if m > 1.0 {
            lx /= m;
            lz /= m;
        }
        let _ = self.character.set_crouched(world, input.crouch);
        let crouched = self.character.crouched;
        let sprint_forward = lx * self.yaw.sin() + lz * self.yaw.cos();
        let eligible = input.sprint && !crouched && !input.aim && sprint_forward > speed::SPRINT_MIN_FORWARD && self.ground_speed > speed::RUN_MPS * 0.8;
        self.sprint_held_s = if eligible { self.sprint_held_s + dt } else { 0.0 };
        let blend = ((self.sprint_held_s - speed::SPRINT_BUILD_S) / speed::SPRINT_RAMP_S).clamp(0.0, 1.0);
        self.sprint = blend * blend * (3.0 - 2.0 * blend);
        let target_speed = if crouched || input.aim {
            speed::CROUCH_AIM_MPS
        } else if input.sprint {
            speed::RUN_MPS + (speed::SPRINT_MPS - speed::RUN_MPS) * self.sprint
        } else {
            speed::WALK_MPS
        };
        let before = self.character.feet();
        let (wx_speed_scale, wx_push_x, wx_push_z) = self.weather.on_foot_modifiers(before.x, before.z, lx, lz);
        let eff_speed = target_speed * wx_speed_scale;
        let (dx, dz) = self.momentum.step(lx * eff_speed + wx_push_x, lz * eff_speed + wx_push_z, dt, input.aim || crouched, before.x, before.z);
        if input.jump && !crouched {
            self.character.jump(speed::JUMP_MPS);
        }
        let (result, floor) = self.character.step_before_physics(world, dx, dz, true);
        let after = self.character.feet();
        let relative = after - before - result.support_motion;
        self.momentum.settle(relative.x, relative.z, dt, after.x, after.z);
        let moved = relative.x.hypot(relative.z);
        self.ground_speed = moved / dt;
        let travel = if dx.hypot(dz) > 1e-5 { Some(dx.atan2(dz)) } else if moved > 1e-4 { Some(relative.x.atan2(relative.z)) } else { None };
        let travel = self.momentum.plant_yaw.or(travel);
        let max_rate = if travel.is_none() && !input.aim {
            speed::STANDING_TURN_RATE_RAD_PER_S
        } else if self.momentum.plant_yaw.is_some() && !input.aim {
            momentum::c::PLANT_TURN_RAD_PER_S
        } else {
            f64::INFINITY
        };
        self.facing_yaw = momentum::facing_yaw(self.facing_yaw, self.yaw, travel, input.aim, dt, max_rate);
        floor
    }

    /// everything a fixed step reads and writes (not the cameras): a
    /// server sends it, a client rewinds to it and replays its inputs.
    pub fn state(&self, world: &CollisionWorld) -> PlayerState {
        let m = &self.momentum;
        PlayerState {
            character: self.character.state(),
            momentum: [m.vx, m.vz, m.ax, m.az],
            momentum_last: m.last,
            plant_yaw: m.plant_yaw,
            yaw: self.yaw,
            pitch: self.pitch,
            facing_yaw: self.facing_yaw,
            sprint_held_s: self.sprint_held_s,
            sprint: self.sprint,
            ground_speed: self.ground_speed,
            driving: match self.mode {
                Mode::OnFoot => None,
                Mode::Driving(i) => Some(i as u32),
            },
            ticks: self.ticks,
            vehicles: self.vehicles.iter().map(|v| v.state(world)).collect(),
            lift: self.lift,
        }
    }

    /// put the player back in a state `state` returned (the vehicles
    /// must be the same cars, spawned in the same order).
    pub fn set_state(&mut self, world: &mut CollisionWorld, s: &PlayerState) {
        self.pending_direction_shift = false;
        self.character.set_state(&s.character);
        self.momentum = Momentum { vx: s.momentum[0], vz: s.momentum[1], ax: s.momentum[2], az: s.momentum[3], last: s.momentum_last, plant_yaw: s.plant_yaw };
        self.yaw = s.yaw;
        self.pitch = s.pitch;
        self.facing_yaw = s.facing_yaw;
        self.sprint_held_s = s.sprint_held_s;
        self.sprint = s.sprint;
        self.ground_speed = s.ground_speed;
        let mode = match s.driving {
            Some(i) if (i as usize) < self.vehicles.len() => Mode::Driving(i as usize),
            _ => Mode::OnFoot,
        };
        if mode != self.mode {
            match mode {
                Mode::Driving(_) => self.drive_cam = DriveCamera::default(),
                Mode::OnFoot => self.on_foot_cam.seed(self.character.feet(), self.last_camera_distance),
            }
        }
        self.mode = mode;
        self.ticks = s.ticks;
        self.lift = s.lift;
        for (v, vs) in self.vehicles.iter_mut().zip(&s.vehicles) {
            v.set_state(world, vs);
        }
    }

    /// The lift prompt: on foot at a lift's stop, not riding.
    pub fn lift_prompt(&self, world: &CollisionWorld) -> Option<crate::lift::LiftPrompt> {
        if self.lift.is_some() || self.mode != Mode::OnFoot {
            return None;
        }
        let lifts = &world.buildings.decks.lifts;
        crate::lift::stop_near(lifts, self.character.feet()).map(|(i, up)| crate::lift::LiftPrompt { name: lifts[i].name.clone(), up })
    }

    /// The screen's darkness of a lift ride this step (0 clear, 1 black).
    pub fn lift_fade(&self) -> f64 {
        crate::lift::fade(self.lift)
    }

    /// Test shoulder clearance between the cabin and the walker or exit station.
    fn vehicle_path_clear(world: &CollisionWorld, v: &Vehicle, center: DVec3) -> bool {
        let cabin = v.pose(world).0;
        let path = center - cabin;
        let length = path.length();
        length <= f64::EPSILON || world.sphere_cast(cabin, path / length, length,
            tuning::capsule::RADIUS_M, Character::filter(), Some(v.body)).is_none()
    }

    /// Distance to a car the walker can reach from their current position.
    fn entry_distance(&self, world: &CollisionWorld, v: &Vehicle) -> Option<f64> {
        let feet = self.character.feet();
        let (p, r) = v.pose(world);
        let local = r.inverse() * (feet - p);
        let dx = (local.x.abs() - v.spec.width / 2.0).max(0.0);
        let dz = (local.z.abs() - v.spec.length / 2.0).max(0.0);
        let distance = dx.hypot(dz);
        let vertical = (feet.y - v.datum(world).y).abs();
        (distance <= tuning::VEHICLE_INTERACT_RANGE_M && vertical <= 3.0
            && Self::vehicle_path_clear(world, v, self.character.position)).then_some(distance)
    }

    /// The nearest car with a clear path to the walker.
    pub fn nearby_vehicle(&self, world: &CollisionWorld) -> Option<usize> {
        self.vehicles.iter().enumerate()
            .filter_map(|(i, v)| self.entry_distance(world, v).map(|d| (i, d)))
            .min_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| i)
    }

    /// At the end of an entry animation the walker and the selected car may have moved.
    pub fn finish_entry(&mut self, world: &CollisionWorld, i: usize) -> bool {
        if self.mode != Mode::OnFoot || self.vehicles.get(i).and_then(|v| self.entry_distance(world, v)).is_none() {
            return false;
        }
        self.enter_vehicle(i);
        true
    }

    /// Enter the nearest car within the interaction range.
    pub fn try_enter(&mut self, world: &CollisionWorld) -> bool {
        let Some(i) = self.nearby_vehicle(world) else { return false };
        self.enter_vehicle(i);
        true
    }

    /// Take control of the car selected at the start of a local entry animation.
    pub fn enter_vehicle(&mut self, i: usize) {
        self.mode = Mode::Driving(i);
        self.drive_cam = DriveCamera::default();
        self.momentum.reset();
    }

    /// Find clear standing room at a vehicle door station, on the actual walkable surface.
    pub fn vehicle_footing(&self, world: &CollisionWorld, i: usize, at: DVec3) -> Option<DVec3> {
        let v = self.vehicles.get(i)?;
        let up = v.pose(world).1 * DVec3::Y;
        let walkable = tuning::capsule::MAX_SLOPE_DEGREES.to_radians().cos();
        if up.y < walkable {
            return None;
        }
        // Extend the sill plane to the door on a pitched or banked road.
        let sill = v.datum(world) + up * v.spec.sill_y;
        let sill_y = sill.y - (up.x * (at.x - sill.x) + up.z * (at.z - sill.z)) / up.y;
        let step = tuning::capsule::MAX_STEP_HEIGHT_M;
        let origin = DVec3::new(at.x, sill_y + step, at.z);
        let reach = 2.0 * step + v.spec.sill_y / up.y + tuning::kerb::GROUND_CONTACT_TOLERANCE_M;
        let hit = world.raycast(origin, DVec3::NEG_Y, reach, layer::WORLD | layer::STREET | layer::BOAT, Some(v.body))?;
        let floor = &world.physics.colliders[hit.collider];
        if hit.normal.y < walkable || !(floor.collision_groups().memberships.intersects(layer::STREET | layer::BOAT)
            || floor.shape().as_heightfield().is_some()) {
            return None;
        }
        // The side of the rounded sole reaches a slope before its lowest point does.
        let feet = hit.point + DVec3::Y * tuning::capsule::RADIUS_M * (1.0 / hit.normal.y - 1.0);
        let center = Character::new(feet).position;
        if !Self::vehicle_path_clear(world, v, center) {
            return None;
        }
        let shape = Capsule::new_y(tuning::capsule::HALF_HEIGHT_M as f32, tuning::capsule::RADIUS_M as f32);
        let pose = Pose::from_translation(Vector::new(center.x as f32, center.y as f32 + tuning::kerb::GROUND_CONTACT_TOLERANCE_M as f32, center.z as f32));
        let f = QueryFilter::default().groups(InteractionGroups::new(Group::ALL, Character::filter(), InteractionTestMode::And));
        world.physics.intersect_shape(pose, &shape, f).next().is_none().then_some(feet)
    }

    fn leave_vehicle(&mut self, feet: DVec3, yaw: f64) {
        self.character.teleport_feet(feet);
        self.momentum.reset();
        self.mode = Mode::OnFoot;
        self.yaw = yaw;
        self.on_foot_cam.seed(feet, self.last_camera_distance);
    }

    /// Return controls to the walker at an animated exit point after rechecking it.
    pub fn exit_vehicle_at(&mut self, world: &CollisionWorld, i: usize, at: DVec3, yaw: f64) -> bool {
        if self.mode != Mode::Driving(i) {
            return false;
        }
        let Some(feet) = self.vehicle_footing(world, i, at) else { return false };
        self.leave_vehicle(feet, yaw);
        self.facing_yaw = yaw;
        self.ground_speed = 0.0;
        true
    }

    /// Leave the car at the driver's (left) door station, else the right, else behind it.
    pub fn try_exit(&mut self, world: &CollisionWorld) -> bool {
        let Mode::Driving(i) = self.mode else { return false };
        let v = &self.vehicles[i];
        let (p, _) = v.pose(world);
        let yaw = v.yaw(world);
        let flat = DQuat::from_rotation_y(yaw);
        let side = v.spec.width / 2.0 + tuning::DOOR_STATION_OUTSET_M;
        let candidates = [DVec3::new(side, 0.0, 0.0), DVec3::new(-side, 0.0, 0.0), DVec3::new(0.0, 0.0, -(v.spec.length / 2.0 + 0.8))];
        for local in candidates {
            if let Some(feet) = self.vehicle_footing(world, i, p + flat * local) {
                self.leave_vehicle(feet, yaw);
                return true;
            }
        }
        false
    }

    /// The camera for this drawn frame (advances the camera filters by `frame_dt`).
    pub fn camera(&mut self, world: &CollisionWorld, frame_dt: f64, aspect: f64) -> CameraView {
        if self.is_swimming(world) {
            return CameraView {
                eye: self.character.feet() + DVec3::Y * 1.49,
                forward: DVec3::new(self.yaw.sin() * self.pitch.cos(), -self.pitch.sin(), self.yaw.cos() * self.pitch.cos()),
                fov_deg: 64., body_visible: false,
            };
        }
        match self.mode {
            Mode::OnFoot => {
                let mut probe = WorldProbe { world, exclude: None, filter: layer::WORLD | layer::STREET | layer::VEHICLE | layer::BOAT };
                let feet = self.character.feet();
                let pose: CameraPose = self.on_foot_cam.update(
                    &OnFootInput {
                        feet,
                        yaw: self.yaw,
                        pitch: self.pitch,
                        speed: self.ground_speed,
                        aiming: false,
                        crouched: self.character.crouched,
                        cover_lift: 0.0,
                        shoulder_side: 1.0,
                        in_cover: false,
                        delta: frame_dt,
                    },
                    &mut probe,
                );
                self.last_camera_distance = (pose.position - feet).length();
                CameraView { eye: pose.position, forward: (pose.target - pose.position).normalize_or(DVec3::Z), fov_deg: pose.fov_deg, body_visible: pose.body_visible }
            }
            Mode::Driving(i) => {
                let v = &self.vehicles[i];
                let (_, rot) = v.pose(world);
                let datum = v.datum(world);
                let yaw = v.yaw(world);
                let speed = v.forward_speed(world);
                let cv = DriveCameraVehicle { length_m: v.spec.length, width_m: v.spec.width, height_m: v.spec.height, datum, rotation: rot };
                let pose = self.drive_cam.pose(datum, yaw, speed, Some(&cv), aspect);
                let mut probe = WorldProbe { world, exclude: Some(v.body), filter: layer::WORLD | layer::STREET | layer::BOAT };
                let lens = camera::resolve_camera_collision(&mut probe, pose.pivot, pose.position, camera::drive::MIN_STANDOFF, true);
                let full = (pose.position - pose.pivot).length();
                if full > 0.0 {
                    self.drive_cam.note_pull_in(lens.distance / full, frame_dt);
                }
                self.last_camera_distance = (lens.position - datum).length();
                CameraView { eye: lens.position, forward: pose.look_dir, fov_deg: pose.fov_deg, body_visible: true }
            }
        }
    }
}
