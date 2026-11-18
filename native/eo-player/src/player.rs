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
use crate::vehicle::{Vehicle, VehicleSpec};
use crate::world::{CollisionWorld, layer};

/// One fixed step's commands (what a network peer would send).
#[derive(Clone, Copy, Debug, Default)]
pub struct StepInput {
    /// x right, y forward; keyboard gives -1/0/1 per axis, a stick its curve.
    pub move_x: f64,
    pub move_y: f64,
    pub sprint: bool,
    pub crouch: bool,
    pub jump: bool,
    pub aim: bool,
    pub vehicle_toggle: bool,
    pub throttle: f64,
    pub brake: f64,
    /// Positive left.
    pub steer: f64,
    pub analog_steer: bool,
    pub handbrake: bool,
    pub look_behind: bool,
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
            throttle: f.throttle,
            brake: f.brake,
            steer: f.steer,
            analog_steer: f.steer.abs() > 0.0 && f.steer.abs() < 0.999,
            handbrake: f.held(Action::Handbrake),
            look_behind: f.held(Action::LookBehind),
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
    pub mode: Mode,
    pub ticks: u64,
    accumulator: f64,
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
            mode: Mode::OnFoot,
            ticks: 0,
            accumulator: 0.0,
            last_camera_distance: camera::on_foot::walk_distance(), // walk_distance()
        }
    }

    /// Place a sedan with its road datum on the ground at (x, z).
    pub fn spawn_vehicle(&mut self, world: &mut CollisionWorld, x: f64, z: f64, yaw: f64) -> usize {
        let y = world.surface_y(x, z);
        let v = Vehicle::spawn(world, VehicleSpec::sedan(), DVec3::new(x, y, z), yaw);
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
        let mut i = *input;
        while self.accumulator >= tuning::FIXED_STEP_S - 1e-9 {
            self.accumulator -= tuning::FIXED_STEP_S;
            self.fixed_step(world, &i);
            i.jump = false;
            i.vehicle_toggle = false;
            n += 1;
        }
        n
    }

    /// One 1/60 s step of the player and the physics world.
    pub fn fixed_step(&mut self, world: &mut CollisionWorld, input: &StepInput) {
        let dt = tuning::FIXED_STEP_S;
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
        match self.mode {
            Mode::OnFoot => {
                self.walk_step(world, input, dt);
                for v in &mut self.vehicles {
                    v.park(world);
                }
            }
            Mode::Driving(i) => {
                for (k, v) in self.vehicles.iter_mut().enumerate() {
                    if k == i {
                        v.pre_step(world, input.steer, input.throttle, input.brake, input.handbrake, input.analog_steer);
                    } else {
                        v.park(world);
                    }
                }
            }
        }
        world.physics.step();
        if let Mode::Driving(i) = self.mode {
            // The body rides in the seat.
            let d = self.vehicles[i].datum(world);
            self.character.teleport_feet(d);
            self.momentum.reset();
            self.drive_cam.update(dt, input.look_behind);
        }
        self.ticks += 1;
    }

    fn walk_step(&mut self, world: &CollisionWorld, input: &StepInput, dt: f64) {
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
        let (dx, dz) = self.momentum.step(lx * target_speed, lz * target_speed, dt, input.aim || crouched, before.x, before.z);
        if input.jump && !crouched {
            self.character.jump(speed::JUMP_MPS);
        }
        self.character.step(world, dx, dz, true);
        let after = self.character.feet();
        self.momentum.settle(after.x - before.x, after.z - before.z, dt, after.x, after.z);
        let moved = (after.x - before.x).hypot(after.z - before.z);
        self.ground_speed = moved / dt;
        let travel = if dx.hypot(dz) > 1e-5 { Some(dx.atan2(dz)) } else if moved > 1e-4 { Some((after.x - before.x).atan2(after.z - before.z)) } else { None };
        let travel = self.momentum.plant_yaw.or(travel);
        let max_rate = if travel.is_none() && !input.aim {
            speed::STANDING_TURN_RATE_RAD_PER_S
        } else if self.momentum.plant_yaw.is_some() && !input.aim {
            momentum::c::PLANT_TURN_RAD_PER_S
        } else {
            f64::INFINITY
        };
        self.facing_yaw = momentum::facing_yaw(self.facing_yaw, self.yaw, travel, input.aim, dt, max_rate);
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
        }
    }

    /// put the player back in a state `state` returned (the vehicles
    /// must be the same cars, spawned in the same order).
    pub fn set_state(&mut self, world: &mut CollisionWorld, s: &PlayerState) {
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
        for (v, vs) in self.vehicles.iter_mut().zip(&s.vehicles) {
            v.set_state(world, vs);
        }
    }

    /// Enter the nearest car within the interaction range.
    pub fn try_enter(&mut self, world: &CollisionWorld) -> bool {
        let feet = self.character.feet();
        let mut best: Option<(usize, f64)> = None;
        for (i, v) in self.vehicles.iter().enumerate() {
            let (p, r) = v.pose(world);
            let local = r.inverse() * (feet - p);
            let dx = (local.x.abs() - v.spec.width / 2.0).max(0.0);
            let dz = (local.z.abs() - v.spec.length / 2.0).max(0.0);
            let d = dx.hypot(dz);
            let vertical = (feet.y - v.datum(world).y).abs();
            if d <= tuning::VEHICLE_INTERACT_RANGE_M && vertical <= 3.0 && best.map(|b| d < b.1).unwrap_or(true) {
                best = Some((i, d));
            }
        }
        let Some((i, _)) = best else { return false };
        self.mode = Mode::Driving(i);
        self.drive_cam = DriveCamera::default();
        self.momentum.reset();
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
            let at = p + flat * local;
            let top = world.raycast(DVec3::new(at.x, p.y + 3.0, at.z), DVec3::NEG_Y, 8.0, layer::WORLD | layer::STREET, Some(v.body));
            let floor = world.floor_y(at.x, at.z);
            let y = top.map(|h| h.point.y).unwrap_or(floor).max(floor);
            let feet = DVec3::new(at.x, y, at.z);
            let center = Character::new(feet).position;
            let shape = Capsule::new_y(tuning::capsule::HALF_HEIGHT_M as f32, tuning::capsule::RADIUS_M as f32);
            let pose = Pose::from_translation(Vector::new(center.x as f32, center.y as f32 + 0.02, center.z as f32));
            let f = QueryFilter::default().groups(InteractionGroups::new(Group::ALL, layer::WORLD | layer::STREET | layer::VEHICLE, InteractionTestMode::And));
            if world.physics.intersect_shape(pose, &shape, f).next().is_some() {
                continue;
            }
            self.character.teleport_feet(feet);
            self.momentum.reset();
            self.mode = Mode::OnFoot;
            self.yaw = yaw;
            self.on_foot_cam.seed(feet, self.last_camera_distance);
            return true;
        }
        false
    }

    /// The camera for this drawn frame (advances the camera filters by `frame_dt`).
    pub fn camera(&mut self, world: &CollisionWorld, frame_dt: f64, aspect: f64) -> CameraView {
        match self.mode {
            Mode::OnFoot => {
                let mut probe = WorldProbe { world, exclude: None, filter: layer::WORLD | layer::STREET | layer::VEHICLE };
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
                let mut probe = WorldProbe { world, exclude: Some(v.body), filter: layer::WORLD | layer::STREET };
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
