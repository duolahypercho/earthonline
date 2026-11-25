//! The on-foot body: a kinematic capsule swept by Rapier's character
//! controller, ported from `createPhysicsPlayerController(...).move()`
//! (src/player/physics-player-controller.js) with the JS configuration
//! (`registerKinematicMotor`, `createCharacterController`): offset 0.01 m,
//! autostep 0.3 m / 0.2 m including dynamic bodies, 45 deg climb, 50 deg slide,
//! snap 0.15 m, normal nudge 1 mm; capsule radius 0.35 m, half height 0.55 m.
//!
//! Ported from `move()`, in its order: gravity into a banked vertical velocity;
//! the analytic building-sweep guard (`limitExteriorBuildingSweep`, which never
//! lets the capsule advance into a footprint prism even where a collider is
//! missing); the floor clamp (the target never below the ground under it,
//! `containmentFloorY`; with decks, the ground only where it is under the
//! body, `CollisionWorld::floor_under`); the perch lift (`PERCH_*`); the kerb step-up lift and
//! hold (`KERB_STEP_*`, `kerbContactToward`, `probeKerbRise`, `kerbStandsOnTop`);
//! the swept move with snap-to-ground off while lifting or holding; landing,
//! and the refused-fall press. `jump()` and crouch are ported.
//!
//! Not ported: the impact hand-off to a dynamic body, mantles and
//! the open-water hazard, interiors, loose-prop pushes, vehicle-contact events,
//! obstacle steering round poles (there are no street props here yet).
//!
//! Positions are f64; Rapier runs f32.

use glam::DVec3;
use rapier3d::control::{CharacterAutostep, CharacterCollision, CharacterLength, KinematicCharacterController};
use rapier3d::prelude::*;

use crate::tuning::{self, capsule, kerb};
use crate::world::{CollisionWorld, distance_to_ring, layer, point_in_ring};

#[derive(Clone, Copy, Debug, Default)]
pub struct MoveResult {
    /// The movement Rapier applied (after every guard).
    pub moved: DVec3,
    pub grounded: bool,
    pub building_limited: bool,
    pub kerb_lift: bool,
}

pub struct Character {
    /// Capsule centre.
    pub position: DVec3,
    pub vertical_velocity: f64,
    pub grounded: bool,
    pub crouched: bool,
    half_height: f64,
    controller: KinematicCharacterController,
    perched_steps: u32,
    perch_lift_cooldown: u32,
    kerb_step_lift: f64,
    kerb_hold_steps: u32,
    kerb_blocked_steps: u32,
    /// Counters (diagnostics, tests).
    pub kerb_steps: u32,
    pub perch_lifts: u32,
    pub last_kerb_rise_m: f64,
}

/// `Character::state` (plain data: a network peer sends it, a client rewinds to it).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CharacterState {
    pub position: [f64; 3],
    pub vertical_velocity: f64,
    pub grounded: bool,
    pub crouched: bool,
    pub half_height: f64,
    pub perched_steps: u32,
    pub perch_lift_cooldown: u32,
    pub kerb_step_lift: f64,
    pub kerb_hold_steps: u32,
    pub kerb_blocked_steps: u32,
}

/// Crouched half height: `Math.min(0.25, halfHeightM)`.
pub const CROUCHED_HALF_HEIGHT_M: f64 = 0.25;

impl Character {
    /// A body standing with its feet at `feet`.
    pub fn new(feet: DVec3) -> Self {
        let controller = KinematicCharacterController {
            up: Vector::Y,
            offset: CharacterLength::Absolute(capsule::CONTROLLER_OFFSET_M as f32),
            slide: true,
            autostep: Some(CharacterAutostep {
                max_height: CharacterLength::Absolute(capsule::MAX_STEP_HEIGHT_M as f32),
                min_width: CharacterLength::Absolute(capsule::MIN_STEP_WIDTH_M as f32),
                include_dynamic_bodies: true,
            }),
            max_slope_climb_angle: (capsule::MAX_SLOPE_DEGREES as f32).to_radians(),
            min_slope_slide_angle: (capsule::MIN_SLIDE_DEGREES as f32).to_radians(),
            snap_to_ground: Some(CharacterLength::Absolute(capsule::SNAP_TO_GROUND_M as f32)),
            normal_nudge_factor: capsule::NORMAL_NUDGE_M as f32,
        };
        let mut c = Self {
            position: DVec3::ZERO,
            vertical_velocity: 0.0,
            grounded: false,
            crouched: false,
            half_height: capsule::HALF_HEIGHT_M,
            controller,
            perched_steps: 0,
            perch_lift_cooldown: 0,
            kerb_step_lift: 0.0,
            kerb_hold_steps: 0,
            kerb_blocked_steps: 0,
            kerb_steps: 0,
            perch_lifts: 0,
            last_kerb_rise_m: 0.0,
        };
        c.teleport_feet(feet);
        c
    }

    pub fn half_extent_y(&self) -> f64 {
        self.half_height + capsule::RADIUS_M
    }

    /// Feet: capsule bottom minus the controller offset (as main.js mirrors the pose).
    pub fn feet(&self) -> DVec3 {
        DVec3::new(self.position.x, self.position.y - self.half_extent_y() - capsule::CONTROLLER_OFFSET_M, self.position.z)
    }

    /// `capsuleCenterPosition`: feet + centre height + offset.
    pub fn teleport_feet(&mut self, feet: DVec3) {
        self.position = DVec3::new(feet.x, feet.y + self.half_extent_y() + capsule::CONTROLLER_OFFSET_M, feet.z);
        self.vertical_velocity = 0.0;
        self.kerb_step_lift = 0.0;
        self.kerb_hold_steps = 0;
        self.perched_steps = 0;
    }

    /// Sweep the same body used on foot, with no stair assist or downward snap.
    pub(crate) fn swim(&mut self, world: &CollisionWorld, delta: DVec3, dt: f64) {
        let mut controller = self.controller;
        controller.snap_to_ground = None;
        controller.autostep = None;
        let filter = QueryFilter::default().groups(InteractionGroups::new(layer::CHARACTER, Self::filter(), InteractionTestMode::And));
        let queries = world.physics.query_pipeline_with_filter(filter);
        let pose = Pose::from_translation(Vector::new(self.position.x as f32, self.position.y as f32, self.position.z as f32));
        let result = controller.move_shape(dt as f32, &queries, &self.shape(), &pose, Vector::new(delta.x as f32, delta.y as f32, delta.z as f32), |_| {});
        self.position += DVec3::new(result.translation.x as f64, result.translation.y as f64, result.translation.z as f64);
        self.vertical_velocity = 0.;
        self.grounded = false;
        self.perched_steps = 0;
        self.kerb_step_lift = 0.;
        self.kerb_hold_steps = 0;
    }

    fn shape(&self) -> Capsule {
        Capsule::new_y(self.half_height as f32, capsule::RADIUS_M as f32)
    }

    fn filter() -> Group {
        layer::WORLD | layer::STREET | layer::VEHICLE
    }

    /// the body between steps: everything `step` reads and writes.
    pub fn state(&self) -> CharacterState {
        CharacterState {
            position: self.position.to_array(),
            vertical_velocity: self.vertical_velocity,
            grounded: self.grounded,
            crouched: self.crouched,
            half_height: self.half_height,
            perched_steps: self.perched_steps,
            perch_lift_cooldown: self.perch_lift_cooldown,
            kerb_step_lift: self.kerb_step_lift,
            kerb_hold_steps: self.kerb_hold_steps,
            kerb_blocked_steps: self.kerb_blocked_steps,
        }
    }

    /// put the body back in a state `state` returned (here or on a server).
    pub fn set_state(&mut self, s: &CharacterState) {
        self.position = DVec3::from_array(s.position);
        self.vertical_velocity = s.vertical_velocity;
        self.grounded = s.grounded;
        self.crouched = s.crouched;
        self.half_height = s.half_height;
        self.perched_steps = s.perched_steps;
        self.perch_lift_cooldown = s.perch_lift_cooldown;
        self.kerb_step_lift = s.kerb_step_lift;
        self.kerb_hold_steps = s.kerb_hold_steps;
        self.kerb_blocked_steps = s.kerb_blocked_steps;
    }

    /// `jump(speed)`: only from the ground, never crouched.
    pub fn jump(&mut self, speed: f64) -> bool {
        if !self.grounded || self.crouched || !(speed > 0.0) {
            return false;
        }
        self.grounded = false;
        self.vertical_velocity = speed;
        true
    }

    /// `setCrouched`: the capsule shortens about its feet; standing up again
    /// needs the taller capsule's room.
    pub fn set_crouched(&mut self, world: &CollisionWorld, crouched: bool) -> bool {
        if crouched == self.crouched {
            return true;
        }
        let next = if crouched { CROUCHED_HALF_HEIGHT_M } else { capsule::HALF_HEIGHT_M };
        let pos = DVec3::new(self.position.x, self.position.y + next - self.half_height, self.position.z);
        if !crouched {
            let shape = Capsule::new_y(next as f32, capsule::RADIUS_M as f32);
            let pose = Pose::from_translation(Vector::new(pos.x as f32, pos.y as f32, pos.z as f32));
            let f = QueryFilter::default().groups(InteractionGroups::new(Group::ALL, Self::filter(), InteractionTestMode::And));
            if world.physics.intersect_shape(pose, &shape, f).next().is_some() {
                return false;
            }
        }
        self.position = pos;
        self.half_height = next;
        self.crouched = crouched;
        true
    }

    fn ray_down(world: &CollisionWorld, origin: DVec3, max: f64) -> Option<crate::world::RayHit> {
        world.raycast(origin, DVec3::NEG_Y, max, layer::WORLD | layer::STREET, None)
    }

    /// `kerbStandsOnTop`.
    fn kerb_stands_on_top(&self, world: &CollisionWorld, center: DVec3) -> bool {
        Self::ray_down(world, center, self.half_extent_y() + capsule::CONTROLLER_OFFSET_M + kerb::STEP_CLEARANCE_M + 0.04).is_some()
    }

    /// `probeKerbRise`: the rise of a walkable step just ahead, or None when
    /// what blocks the body is not a step.
    fn probe_kerb_rise(&self, world: &CollisionWorld, center: DVec3, dx: f64, dz: f64) -> Option<f64> {
        let feet_y = center.y - self.half_extent_y() - capsule::CONTROLLER_OFFSET_M;
        let under = Self::ray_down(world, center, self.half_extent_y() + capsule::CONTROLLER_OFFSET_M + 0.08)?;
        let foot_y = feet_y.min(under.point.y);
        let reach = capsule::RADIUS_M + 0.12;
        let origin = DVec3::new(center.x + dx * reach, foot_y + kerb::STEP_MAX_RISE_M + 0.05, center.z + dz * reach);
        let top = Self::ray_down(world, origin, kerb::STEP_MAX_RISE_M + 0.05 + 0.02)?;
        if !(top.distance > 1e-3) || top.normal.y < 0.7 {
            return None;
        }
        let rise = top.point.y - foot_y;
        if !(kerb::STEP_MIN_RISE_M..=kerb::STEP_MAX_RISE_M).contains(&rise) {
            return None;
        }
        for lift in [0.04, 0.45] {
            let o = DVec3::new(center.x, foot_y + rise + lift, center.z);
            if world.raycast(o, DVec3::new(dx, 0.0, dz), capsule::RADIUS_M + 0.3, layer::WORLD | layer::STREET | layer::VEHICLE, None).is_some() {
                return None;
            }
        }
        Some(rise)
    }

    /// `exteriorBuildingBlocked`.
    fn building_blocked(&self, prisms: &[std::sync::Arc<Vec<crate::world::Prism>>], p: DVec3) -> bool {
        let vmin = p.y - self.half_extent_y();
        let vmax = p.y + self.half_extent_y();
        let clearance = capsule::RADIUS_M + capsule::CONTROLLER_OFFSET_M;
        for set in prisms {
            for b in set.iter() {
                if vmax <= b.base_y || vmin >= b.top_y {
                    continue;
                }
                if p.x < b.min[0] - clearance || p.x > b.max[0] + clearance || p.z < b.min[1] - clearance || p.z > b.max[1] + clearance {
                    continue;
                }
                if point_in_ring(p.x, p.z, &b.ring) || distance_to_ring(p.x, p.z, &b.ring) <= clearance {
                    return true;
                }
            }
        }
        false
    }

    /// `limitExteriorBuildingSweep`.
    fn limit_building_sweep(&self, world: &CollisionWorld, d: DVec3) -> (f64, f64, bool) {
        let distance = d.x.hypot(d.z);
        if distance <= 1e-12 {
            return (d.x, d.z, false);
        }
        let prisms = world.prisms_near(self.position.x, self.position.z, distance + 2.0);
        if prisms.is_empty() {
            return (d.x, d.z, false);
        }
        let sample = (capsule::RADIUS_M * 0.5).max(0.025);
        let samples = ((distance / sample).ceil() as usize).max(1);
        let mut previous_t = 0.0;
        for s in 1..=samples {
            let t = s as f64 / samples as f64;
            if !self.building_blocked(&prisms, self.position + d * t) {
                previous_t = t;
                continue;
            }
            let (mut low, mut high) = (previous_t, t);
            for _ in 0..14 {
                let mid = (low + high) / 2.0;
                if self.building_blocked(&prisms, self.position + d * mid) {
                    high = mid;
                } else {
                    low = mid;
                }
            }
            let safe = (low - capsule::CONTROLLER_OFFSET_M / distance.max(capsule::CONTROLLER_OFFSET_M)).max(0.0);
            return (d.x * safe, d.z * safe, true);
        }
        (d.x, d.z, false)
    }

    /// One fixed step: `desired` is the horizontal stride (x, z) this step.
    pub fn step(&mut self, world: &CollisionWorld, desired_x: f64, desired_z: f64, apply_gravity: bool) -> MoveResult {
        let step = tuning::FIXED_STEP_S;
        let mut d = DVec3::new(desired_x, 0.0, desired_z);
        if apply_gravity {
            self.vertical_velocity -= tuning::GRAVITY_MPS2 * step;
            d.y += self.vertical_velocity * step;
        }
        let (bx, bz, building_limited) = self.limit_building_sweep(world, d);
        d.x = bx;
        d.z = bz;
        // (the floor under the body: one level, the ground; with decks the
        // ground only where it is under the body, `CollisionWorld::floor_under`.)
        let target_ground = world.floor_under(self.position.x + d.x, self.position.z + d.z, self.position.y);
        d.y = d.y.max(target_ground + self.half_extent_y() - self.position.y);
        // KERB LIP: a perched capsule still driven forward gets one lift, then a cooldown.
        if self.perch_lift_cooldown > 0 {
            self.perch_lift_cooldown -= 1;
        } else if self.perched_steps >= kerb::PERCH_LIFT_AFTER_STEPS && d.x.hypot(d.z) > 1e-4 {
            d.y = d.y.max(kerb::PERCH_STEP_LIFT_M);
            self.vertical_velocity = 0.0;
            self.perch_lifts += 1;
            self.perched_steps = 0;
            self.perch_lift_cooldown = kerb::PERCH_LIFT_COOLDOWN_STEPS;
        }
        // KERB STEP-UP: the lift decided on the previous refused step, straight up.
        let mut kerb_lifting = false;
        if self.kerb_step_lift > 0.0 {
            d = DVec3::new(0.0, d.y.max(self.kerb_step_lift), 0.0);
            self.vertical_velocity = 0.0;
            self.kerb_steps += 1;
            kerb_lifting = true;
            self.kerb_hold_steps = kerb::STEP_HOLD_STEPS;
        } else if self.kerb_hold_steps > 0 {
            self.kerb_hold_steps -= 1;
            if self.kerb_stands_on_top(world, self.position) {
                self.kerb_hold_steps = 0;
            } else if d.y < 0.0 {
                d.y = 0.0;
                self.vertical_velocity = 0.0;
            }
        }
        self.kerb_step_lift = 0.0;

        let snap = !kerb_lifting && self.kerb_hold_steps == 0;
        let mut ctl = self.controller;
        if !snap {
            ctl.snap_to_ground = None;
        }
        let shape = self.shape();
        let pose = Pose::from_translation(Vector::new(self.position.x as f32, self.position.y as f32, self.position.z as f32));
        let filter = QueryFilter::default().groups(InteractionGroups::new(layer::CHARACTER, Self::filter(), InteractionTestMode::And));
        let queries = world.physics.query_pipeline_with_filter(filter);
        let mut collisions: Vec<CharacterCollision> = vec![];
        let r = ctl.move_shape(step as f32, &queries, &shape, &pose, Vector::new(d.x as f32, d.y as f32, d.z as f32), |c| collisions.push(c));
        let moved = DVec3::new(r.translation.x as f64, r.translation.y as f64, r.translation.z as f64);
        let start = self.position;
        self.position += moved;
        self.grounded = r.grounded;
        if self.grounded && self.vertical_velocity < 0.0 {
            self.vertical_velocity = 0.0;
        } else if !self.grounded && self.vertical_velocity < -kerb::PERCHED_FALL_PRESS_MPS && d.y < 0.0 && moved.y > d.y * 0.25 {
            // A fall the world refuses is not a fall: keep only a light press.
            self.vertical_velocity = -kerb::PERCHED_FALL_PRESS_MPS;
            self.perched_steps += 1;
        } else if self.grounded || moved.y <= d.y * 0.25 {
            self.perched_steps = 0;
        }
        if !kerb_lifting && !collisions.is_empty() {
            let feet_y = start.y - self.half_extent_y() - capsule::CONTROLLER_OFFSET_M;
            let toward = kerb_contact_toward(world, &collisions, d, feet_y);
            let into = toward.map(|t| d.x * t.0 + d.z * t.1).unwrap_or(0.0);
            let made = toward.map(|t| moved.x * t.0 + moved.z * t.1).unwrap_or(0.0);
            let refused = toward.is_some() && into > 1e-4 && made < into * kerb::STEP_REFUSED_SHARE;
            let rise = if refused { toward.and_then(|t| self.probe_kerb_rise(world, start, t.0, t.1)) } else { None };
            self.kerb_blocked_steps = if rise.is_none() { 0 } else { self.kerb_blocked_steps + 1 };
            if let Some(rise) = rise {
                if self.kerb_blocked_steps >= kerb::STEP_AFTER_BLOCKED_STEPS {
                    self.kerb_step_lift = rise + kerb::STEP_CLEARANCE_M;
                    self.last_kerb_rise_m = (rise * 1000.0).round() / 1000.0;
                    self.kerb_blocked_steps = 0;
                }
            }
        } else {
            self.kerb_blocked_steps = 0;
        }
        MoveResult { moved, grounded: self.grounded, building_limited, kerb_lift: kerb_lifting }
    }
}

/// `kerbContactToward`: the horizontal direction of the kerb face the sweep
/// presses into most, or None. Vehicles are never kerbs.
fn kerb_contact_toward(world: &CollisionWorld, contacts: &[CharacterCollision], desired: DVec3, feet_y: f64) -> Option<(f64, f64)> {
    let length = desired.x.hypot(desired.z);
    if !(length > 1e-6) {
        return None;
    }
    let mut best: Option<(f64, f64, f64)> = None;
    for c in contacts {
        let vehicle = world.physics.colliders.get(c.handle).map(|co| co.collision_groups().memberships.contains(layer::VEHICLE)).unwrap_or(true);
        if vehicle {
            continue;
        }
        let n = c.hit.normal1;
        let (nx, ny, nz) = (n.x as f64, n.y as f64, n.z as f64);
        let rise = c.hit.witness1.y as f64 - feet_y;
        if !(-0.02..=kerb::STEP_MAX_RISE_M + 0.04).contains(&rise) {
            continue;
        }
        let h = nx.hypot(nz);
        if !(h > 0.3 * (nx * nx + ny * ny + nz * nz).sqrt()) {
            continue;
        }
        let (mut tx, mut tz) = (nx / h, nz / h);
        if tx * desired.x + tz * desired.z < 0.0 {
            tx = -tx;
            tz = -tz;
        }
        let along = (tx * desired.x + tz * desired.z) / length;
        if along < 0.3 || best.map(|b| along <= b.2).unwrap_or(false) {
            continue;
        }
        best = Some((tx, tz, along));
    }
    best.map(|b| (b.0, b.1))
}
