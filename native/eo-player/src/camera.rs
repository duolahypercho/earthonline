//! Camera rigs, ported from the JS game:
//!
//! * the on-foot shoulder camera, `updateOnFootCamera` (src/player/on-foot-camera.js,
//!   `on-foot-shoulder-camera-v9-over-head-wall-aware`): boom, rest tilt, run and
//!   sprint pull-back, shoulder offset with the walled-shoulder swap, five-rail
//!   plus swept-ball wall probe, anti-crush rise, swing round occluders, back-to-
//!   the-wall swing, whiskers, curb-step pivot filter, body-cull hysteresis;
//! * the drive orbit and pose, `createDriveCameraState` / `applyDriveLook` /
//!   `updateDriveCamera` / `driveCameraPose` / `liftedAim` / `resolveCameraCollision`
//!   (src/citygen/player-camera-rig.js) with `vehicleCameraSizing` and
//!   `fitVehicleCameraDistance` (src/player/drive-camera-framing.js).
//!
//! Not ported: the drive camera's dynamics layer (src/player/drive-camera-dynamics.js,
//! heading lag, speed pull-back, shake, look-ahead) and its obstruction recovery
//! (drive-camera-obstruction.js); the swept near-plane box is a ray here.
//!
//! The JS has no separate first-person mode: when the boom is crushed below
//! `bodyCullDistance` the avatar is dropped and the view reads as first person
//! (`bodyVisible`). That flag is carried through.
//!
//! Probes are a trait so the same code runs against the Rapier world or an
//! analytic scene (the JS oracle test). All arithmetic is f64 as in the JS.

use glam::DVec3;

#[derive(Clone, Copy, Debug)]
pub struct ProbeHit {
    pub distance: f64,
    /// A traffic body (soft obstruction), not a wall.
    pub soft: bool,
}

/// `probe({ origin, direction, maxDistance, radius })`.
pub trait CameraProbe {
    fn probe(&mut self, origin: DVec3, direction: DVec3, max_distance: f64, radius: Option<f64>) -> Option<ProbeHit>;
}

fn clamp(v: f64, a: f64, b: f64) -> f64 {
    v.max(a).min(b)
}
fn damp(a: f64, b: f64, rate: f64, dt: f64) -> f64 {
    a + (b - a) * (1.0 - (-rate * dt).exp())
}

/// `ON_FOOT_CAMERA`.
pub mod on_foot {
    const WALK_DISTANCE_M: f64 = 4.43;
    const PIVOT_HEIGHT_M: f64 = 1.43;
    const WALK_LENS_HEIGHT_M: f64 = 2.5;
    pub const WALK_DISTANCE: f64 = WALK_DISTANCE_M;
    pub const RUN_DISTANCE: f64 = 4.83;
    pub const AIM_DISTANCE: f64 = 2.2;
    pub const COVER_DISTANCE: f64 = 3.0;
    pub const WALK_FOV: f64 = 50.0;
    pub const RUN_FOV: f64 = 53.0;
    pub const AIM_FOV: f64 = 42.0;
    pub const SPRINT_FROM_MPS: f64 = 5.8;
    pub const SPRINT_FULL_MPS: f64 = 7.2;
    pub const SPRINT_FOV_KICK: f64 = 5.0;
    pub const SPRINT_DISTANCE_ADD: f64 = 0.4;
    pub const SPRINT_RATE: f64 = 3.0;
    pub const PIVOT_HEIGHT: f64 = PIVOT_HEIGHT_M;
    pub const PIVOT_SLACK: f64 = 0.18;
    pub fn rest_elevation() -> f64 {
        if eo_data::groundscale::low_camera() {
            return ((LOW_LENS_HEIGHT_M - PIVOT_HEIGHT_M) / LOW_WALK_DISTANCE_M).asin();
        }
        ((WALK_LENS_HEIGHT_M - PIVOT_HEIGHT_M) / WALK_DISTANCE_M).asin()
    }
    // the lower on-foot camera (the street-size choice (d)): the lens at
    // 1.9 m instead of 2.5 m (7.1 degrees over the pivot instead of 14.0), the boom
    // 3.8 m walking / 4.2 m running instead of 4.43 / 4.83: the default (groundscale::low_camera); `--default-camera` gives the JS values above.
    const LOW_LENS_HEIGHT_M: f64 = 1.9;
    const LOW_WALK_DISTANCE_M: f64 = 3.8;
    const LOW_RUN_DISTANCE_M: f64 = 4.2;
    pub fn walk_distance() -> f64 {
        if eo_data::groundscale::low_camera() { LOW_WALK_DISTANCE_M } else { WALK_DISTANCE }
    }
    pub fn run_distance() -> f64 {
        if eo_data::groundscale::low_camera() { LOW_RUN_DISTANCE_M } else { RUN_DISTANCE }
    }
    pub const ELEVATION_MIN: f64 = -1.0;
    pub const ELEVATION_MAX: f64 = 1.15;
    pub const SHOULDER: f64 = 0.5;
    pub const AIM_SHOULDER: f64 = 0.7;
    pub const AIM_RISE: f64 = 0.5;
    pub const RADIUS: f64 = 0.22;
    pub const CLEARANCE: f64 = 0.16;
    pub const RECOVER_RATE: f64 = 4.0;
    pub const RISE_STEPS: [f64; 3] = [0.1, 0.22, 0.45];
    pub const RISE_TARGET: f64 = 0.75;
    pub const RISE_RATE: f64 = 6.0;
    pub const RISE_MIN_GAIN_M: f64 = 0.6;
    pub const MIN_DISTANCE: f64 = 0.35;
    pub const BODY_CULL_DISTANCE: f64 = 0.95;
    pub const BODY_REVEAL_DISTANCE: f64 = 1.18;
    pub const SOFT_IGNORE_DISTANCE: f64 = 0.34;
    pub const SOFT_MIN_DISTANCE: f64 = 1.35;
    pub const HANDOVER_SECONDS: f64 = 0.55;
    pub const HANDOVER_RATE: f64 = 7.0;
    pub const HANDOVER_MAX_STEP_M: f64 = 0.75;
    pub const AIM_IN_BOOM_MPS: f64 = 15.0;
    pub const SWING_STEPS: [f64; 2] = [0.4, 0.75];
    pub const SWING_COST_PER_RAD: f64 = 0.55;
    pub const SWING_RATE: f64 = 6.0;
    pub const SWING_RETURN_RATE: f64 = 2.2;
    pub const MIN_VISIBLE_DISTANCE: f64 = 0.7;
    pub const WALL_SWING_STEPS: [f64; 3] = [1.25, 1.42, 1.57];
    pub const WALL_SWING_ENTER_M: f64 = 0.95;
    pub const WALL_SWING_REACH: f64 = 0.6;
    pub const SWEEP_RADIUS: f64 = 0.16;
    pub const WALL_SENSE_M: f64 = 1.25;
    pub const SWAP_MARGIN_M: f64 = 0.45;
    pub const SWAP_RELEASE_M: f64 = 0.6;
    pub const SWAP_RATE: f64 = 3.2;
    pub const WHISKER_M: f64 = 1.0;
    pub const WHISKER_MAX_PUSH_M: f64 = 0.75;
    pub const WHISKER_RATE: f64 = 7.0;
    pub const FOLLOW_RATE: f64 = 20.0;
}

/// Persistent filter state of the on-foot rig (the JS `state` object).
#[derive(Clone, Debug, Default)]
pub struct OnFootCamera {
    feet: Option<DVec3>,
    crouch: f64,
    cover_lift: f64,
    aim: f64,
    run: f64,
    sprint: Option<f64>,
    height: f64,
    follow: (f64, f64),
    rise: f64,
    boom: Option<f64>,
    handover: f64,
    cover: Option<f64>,
    auto_side: Option<f64>,
    auto_swap: bool,
    shoulder_side: Option<f64>,
    swing: f64,
    whisker: Option<f64>,
    body_visible: Option<bool>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct OnFootInput {
    pub feet: DVec3,
    pub yaw: f64,
    pub pitch: f64,
    pub speed: f64,
    pub aiming: bool,
    pub crouched: bool,
    pub cover_lift: f64,
    pub shoulder_side: f64,
    pub in_cover: bool,
    pub delta: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct CameraPose {
    pub position: DVec3,
    pub target: DVec3,
    pub pivot: DVec3,
    pub fov_deg: f64,
    pub boom: f64,
    pub blocked: bool,
    pub body_visible: bool,
    pub swing: f64,
    pub rise: f64,
    pub whisker: f64,
    pub shoulder_side: f64,
}

#[derive(Clone, Copy)]
struct Reach {
    allowed: f64,
    blocked: bool,
    soft_blocked: bool,
    hard_blocked: bool,
}

impl OnFootCamera {
    /// `seedOnFootCamera`: hand the walk rig the lens distance it already has
    /// (the drive -> walk handover).
    pub fn seed(&mut self, feet: DVec3, distance: f64) {
        use on_foot as c;
        if !(distance.is_finite() && distance > 0.0) {
            return;
        }
        self.feet = Some(feet);
        self.height = feet.y + c::PIVOT_HEIGHT;
        self.follow = (feet.x, feet.z);
        self.rise = 0.0;
        self.aim = 0.0;
        self.run = 0.0;
        let b = clamp(distance, c::MIN_DISTANCE, c::walk_distance() * 3.0); // walk_distance()
        self.boom = Some(b);
        self.body_visible = Some(b > c::BODY_CULL_DISTANCE);
        self.handover = c::HANDOVER_SECONDS;
    }

    /// `updateOnFootCamera`.
    pub fn update(&mut self, i: &OnFootInput, probe: &mut dyn CameraProbe) -> CameraPose {
        use on_foot as c;
        let dt = clamp(i.delta, 0.0, 0.1);
        let feet = i.feet;
        let yaw = i.yaw;
        let teleported = match self.feet {
            None => true,
            Some(f) => (feet - f).length() > 8.0,
        };
        self.crouch = if teleported { if i.crouched { 1.0 } else { 0.0 } } else { damp(self.crouch, if i.crouched { 1.0 } else { 0.0 }, 14.0, dt) };
        let lift = clamp(i.cover_lift, 0.0, 0.5);
        self.cover_lift = if teleported { lift } else { damp(self.cover_lift, lift, 8.0, dt) };
        let pivot_height = c::PIVOT_HEIGHT - 0.38 * self.crouch + self.cover_lift;
        if teleported {
            self.aim = if i.aiming { 1.0 } else { 0.0 };
            self.run = 0.0;
            self.height = feet.y + pivot_height;
            self.follow = (feet.x, feet.z);
            self.rise = 0.0;
            self.boom = None;
            self.handover = 0.0;
        }
        let in_cover = if i.in_cover { 1.0 } else { 0.0 };
        self.cover = Some(match self.cover {
            Some(cv) if !teleported => damp(cv, in_cover, 5.0, dt),
            _ => in_cover,
        });
        let cover = self.cover.unwrap();
        let owns_side = !i.in_cover && !i.aiming && self.aim < 0.5;
        let mut auto_side = match self.auto_side {
            Some(s) if !teleported => s,
            _ => 1.0,
        };
        if owns_side {
            let mut room_on = |sign: f64| {
                let dir = DVec3::new(-yaw.cos() * sign, 0.0, yaw.sin() * sign);
                let mut room = f64::INFINITY;
                for back in [0.0, c::walk_distance() * 0.85] { // walk_distance()
                    let origin = DVec3::new(feet.x - yaw.sin() * back, feet.y + pivot_height, feet.z - yaw.cos() * back);
                    if let Some(h) = probe.probe(origin, dir, c::WALL_SENSE_M + c::SWAP_RELEASE_M + 0.5, None) {
                        if h.distance.is_finite() && !h.soft {
                            room = room.min(h.distance);
                        }
                    }
                }
                room
            };
            let right_room = room_on(1.0);
            let left_room = room_on(-1.0);
            if auto_side > 0.0 {
                if right_room < c::WALL_SENSE_M && left_room > right_room + c::SWAP_MARGIN_M {
                    auto_side = -1.0;
                }
            } else if right_room >= c::WALL_SENSE_M + c::SWAP_RELEASE_M || right_room > left_room + c::SWAP_RELEASE_M {
                auto_side = 1.0;
            }
        } else {
            auto_side = 1.0;
        }
        self.auto_side = Some(auto_side);
        let side_target = (if i.shoulder_side < 0.0 { -1.0 } else { 1.0 }) * auto_side;
        if auto_side < 0.0 {
            self.auto_swap = true;
        } else if self.auto_swap && (self.shoulder_side.unwrap_or(1.0) - side_target).abs() < 0.02 {
            self.auto_swap = false;
        }
        self.shoulder_side = Some(match self.shoulder_side {
            Some(s) if !teleported => damp(s, side_target, if self.auto_swap && !i.in_cover { c::SWAP_RATE } else { 6.0 }, dt),
            _ => side_target,
        });
        let shoulder_side = self.shoulder_side.unwrap();
        self.feet = Some(feet);
        self.aim = damp(self.aim, if i.aiming { 1.0 } else { 0.0 }, if i.aiming { 12.0 } else { 9.0 }, dt);
        self.run = damp(self.run, if i.aiming { 0.0 } else { clamp((i.speed - 2.0) / 3.6, 0.0, 1.0) }, 5.0, dt);
        let sprint_target = if i.aiming { 0.0 } else { clamp((i.speed - c::SPRINT_FROM_MPS) / (c::SPRINT_FULL_MPS - c::SPRINT_FROM_MPS), 0.0, 1.0) };
        self.sprint = Some(match self.sprint {
            Some(s) if !teleported => damp(s, sprint_target, if i.aiming { 12.0 } else { c::SPRINT_RATE }, dt),
            _ => 0.0,
        });
        let sprint = self.sprint.unwrap();
        self.height = clamp(damp(self.height, feet.y + pivot_height, 14.0, dt), feet.y + pivot_height - c::PIVOT_SLACK, feet.y + pivot_height + c::PIVOT_SLACK);
        self.follow = (damp(self.follow.0, feet.x, c::FOLLOW_RATE, dt), damp(self.follow.1, feet.z, c::FOLLOW_RATE, dt));
        let right = DVec3::new(-yaw.cos(), 0.0, yaw.sin());
        let side_sign = if shoulder_side < 0.0 { -1.0 } else { 1.0 };
        let shoulder = (c::SHOULDER + self.aim * c::AIM_SHOULDER) * shoulder_side.abs();
        let lateral = right * side_sign;
        let mut pivot = DVec3::new(self.follow.0, self.height, self.follow.1);
        let mut side_distance = shoulder;
        if shoulder > 1e-3 {
            if let Some(h) = probe.probe(pivot, lateral, shoulder + c::RADIUS, None) {
                if h.distance.is_finite() {
                    side_distance = clamp(h.distance - c::RADIUS, 0.0, shoulder);
                }
            }
        }
        pivot.x += lateral.x * side_distance;
        pivot.z += lateral.z * side_distance;
        pivot.y += self.aim * c::AIM_RISE;
        let rest = clamp(i.pitch + c::rest_elevation() * (1.0 - self.aim), c::ELEVATION_MIN, c::ELEVATION_MAX);
        let walk_distance = c::walk_distance() + (c::COVER_DISTANCE - c::walk_distance()) * cover; // walk_distance()
        let free_distance = walk_distance + (c::run_distance() - walk_distance) * self.run + c::SPRINT_DISTANCE_ADD * sprint; // run_distance()
        let desired = free_distance + (c::AIM_DISTANCE - free_distance) * self.aim;
        let arm_for = |angle: f64, swing: f64| DVec3::new(-(yaw + swing).sin() * angle.cos(), angle.sin(), -(yaw + swing).cos() * angle.cos());
        let mut reach = |arm: DVec3| -> Reach {
            let mut r = Reach { allowed: desired, blocked: false, soft_blocked: false, hard_blocked: false };
            let flat = arm.x.hypot(arm.z);
            let flat = if flat == 0.0 { 1.0 } else { flat };
            let (rail_x, rail_z) = (-arm.z / flat, arm.x / flat);
            for (lat, vert) in [(0.0, 0.0), (-c::RADIUS, 0.0), (c::RADIUS, 0.0), (0.0, -c::RADIUS), (0.0, c::RADIUS)] {
                let origin = DVec3::new(pivot.x + rail_x * lat, pivot.y + vert, pivot.z + rail_z * lat);
                let Some(h) = probe.probe(origin, arm, desired + c::CLEARANCE, None) else { continue };
                if !h.distance.is_finite() || h.distance >= desired + c::CLEARANCE {
                    continue;
                }
                if h.soft {
                    if h.distance <= c::SOFT_IGNORE_DISTANCE {
                        continue;
                    }
                    let floor = c::SOFT_MIN_DISTANCE.max(c::MIN_DISTANCE);
                    let short = h.distance - c::CLEARANCE;
                    r.allowed = r.allowed.min(if short >= floor { short } else { c::MIN_DISTANCE.max(short) });
                    r.blocked = true;
                    r.soft_blocked = true;
                    continue;
                }
                r.allowed = r.allowed.min(c::MIN_DISTANCE.max(h.distance - c::CLEARANCE));
                r.blocked = true;
                r.hard_blocked = true;
            }
            if let Some(s) = probe.probe(pivot, arm, desired + c::CLEARANCE, Some(c::SWEEP_RADIUS)) {
                if s.distance.is_finite() && s.distance < desired + c::CLEARANCE && !s.soft {
                    r.allowed = r.allowed.min(c::MIN_DISTANCE.max(s.distance + c::SWEEP_RADIUS - c::CLEARANCE));
                    r.blocked = true;
                    r.hard_blocked = true;
                }
            }
            r
        };
        let mut best = reach(arm_for(rest, 0.0));
        let mut rise = 0.0;
        let aiming_against_car = self.aim >= 0.5 && best.soft_blocked && !best.hard_blocked;
        if best.blocked && best.allowed < desired * c::RISE_TARGET && !aiming_against_car {
            let at_eye = best.allowed;
            let gain_needed = if at_eye >= c::BODY_REVEAL_DISTANCE { c::RISE_MIN_GAIN_M } else { 0.0 };
            for step in c::RISE_STEPS {
                let angle = clamp(rest + step, c::ELEVATION_MIN, c::ELEVATION_MAX);
                let lifted = reach(arm_for(angle, 0.0));
                if lifted.allowed > best.allowed && lifted.allowed - at_eye >= gain_needed {
                    best = lifted;
                    rise = angle - rest;
                }
                if best.allowed >= desired * c::RISE_TARGET {
                    break;
                }
            }
        }
        let mut swing = 0.0;
        if best.blocked && best.allowed < desired * c::RISE_TARGET && self.aim < 0.5 {
            let current = self.swing;
            let preferred = if current != 0.0 { current.signum() } else { 1.0 };
            let mut score = best.allowed;
            for step in c::SWING_STEPS {
                for sign in [preferred, -preferred] {
                    let offset = sign * step;
                    for lift in [0.0, c::RISE_STEPS[0], c::RISE_STEPS[1], c::RISE_STEPS[2]] {
                        let angle = clamp(rest + lift, c::ELEVATION_MIN, c::ELEVATION_MAX);
                        let trial = reach(arm_for(angle, offset));
                        if trial.allowed < desired * c::RISE_TARGET {
                            continue;
                        }
                        let value = trial.allowed - c::SWING_COST_PER_RAD * step;
                        if value > score + 1e-3 {
                            score = value;
                            best = trial;
                            swing = offset;
                            rise = angle - rest;
                        }
                    }
                }
                if best.allowed >= desired * c::RISE_TARGET {
                    break;
                }
            }
            if best.allowed < c::WALL_SWING_ENTER_M {
                let mut wall_score = best.allowed;
                for step in c::WALL_SWING_STEPS {
                    for sign in [preferred, -preferred] {
                        let offset = sign * step;
                        let trial = reach(arm_for(rest, offset));
                        if trial.allowed < desired * c::WALL_SWING_REACH {
                            continue;
                        }
                        let value = trial.allowed - c::SWING_COST_PER_RAD * step;
                        if value > wall_score + 1e-3 {
                            wall_score = value;
                            best = trial;
                            swing = offset;
                            rise = 0.0;
                        }
                    }
                    if swing != 0.0 && swing.abs() == step {
                        break;
                    }
                }
            }
        }
        let swing_target = swing;
        let prior_swing = self.swing;
        self.swing = if teleported {
            swing_target
        } else {
            damp(prior_swing, swing_target, if i.aiming { 14.0 } else if swing_target.abs() >= prior_swing.abs() { c::SWING_RATE } else { c::SWING_RETURN_RATE }, dt)
        };
        if self.swing.abs() < 1e-4 {
            self.swing = 0.0;
        }
        self.rise = damp(self.rise, rise, c::RISE_RATE, dt);
        let elevation = clamp(rest + self.rise, c::ELEVATION_MIN, c::ELEVATION_MAX);
        let back = arm_for(elevation, self.swing);
        let r = if self.rise == rise && self.swing == swing { best } else { reach(back) };
        let (allowed, blocked, soft_blocked) = (r.allowed, r.blocked, r.soft_blocked);
        let handover = (self.handover - dt).max(0.0);
        self.handover = handover;
        let boom = match self.boom {
            None => allowed,
            Some(b) if handover > 0.0 => {
                let next = damp(b, allowed, c::HANDOVER_RATE, dt);
                b + clamp(next - b, -c::HANDOVER_MAX_STEP_M, c::HANDOVER_MAX_STEP_M)
            }
            Some(b) if i.aiming && !blocked && !soft_blocked && allowed < b => allowed.max(b - c::AIM_IN_BOOM_MPS * dt),
            Some(b) => {
                if allowed < b {
                    allowed
                } else {
                    damp(b, allowed, c::RECOVER_RATE, dt)
                }
            }
        };
        self.boom = Some(boom);
        let mut position = pivot + back * boom;
        let mut target = pivot - back * 8.0;
        let whisker_weight = (1.0 - self.aim) * (1.0 - cover);
        let mut whisker_target = 0.0;
        let lens_yaw = yaw + self.swing;
        let lens_right = DVec3::new(-lens_yaw.cos(), 0.0, lens_yaw.sin());
        if whisker_weight > 1e-3 {
            let reach_m = c::WHISKER_M + c::WHISKER_MAX_PUSH_M + c::RADIUS;
            let mut room_along = |sign: f64| match probe.probe(position, lens_right * sign, reach_m, None) {
                Some(h) if h.distance.is_finite() && !h.soft => h.distance,
                _ => f64::INFINITY,
            };
            let d_r = room_along(1.0);
            let d_l = room_along(-1.0);
            if d_r < c::WHISKER_M && d_r <= d_l {
                whisker_target = -(c::WHISKER_M - d_r).min((d_l - d_r) / 2.0);
            } else if d_l < c::WHISKER_M && d_l < d_r {
                whisker_target = (c::WHISKER_M - d_l).min((d_r - d_l) / 2.0);
            }
            whisker_target = clamp(whisker_target, -c::WHISKER_MAX_PUSH_M, c::WHISKER_MAX_PUSH_M) * whisker_weight;
            if whisker_target.abs() > 1e-3 {
                let pushed = DVec3::new(position.x + lens_right.x * whisker_target, position.y, position.z + lens_right.z * whisker_target);
                let o = pushed - pivot;
                let length = o.length();
                if length > 1e-3 {
                    if let Some(s) = probe.probe(pivot, o / length, length, None) {
                        if s.distance.is_finite() && !s.soft && s.distance < length - 0.05 {
                            whisker_target = 0.0;
                        }
                    }
                }
            }
        }
        self.whisker = Some(match self.whisker {
            Some(w) if !teleported => damp(w, whisker_target, c::WHISKER_RATE, dt),
            _ => whisker_target,
        });
        let mut whisker = self.whisker.unwrap();
        if whisker.abs() < 1e-4 {
            whisker = 0.0;
        }
        self.whisker = Some(whisker);
        if whisker != 0.0 {
            position.x += lens_right.x * whisker;
            position.z += lens_right.z * whisker;
            target.x += lens_right.x * whisker;
            target.z += lens_right.z * whisker;
        }
        let free_fov = c::WALK_FOV + (c::RUN_FOV - c::WALK_FOV) * self.run + c::SPRINT_FOV_KICK * sprint;
        let cull_at = c::BODY_CULL_DISTANCE.min(c::MIN_VISIBLE_DISTANCE);
        let reveal_at = c::BODY_REVEAL_DISTANCE.min(c::MIN_VISIBLE_DISTANCE + 0.23);
        let bv = match self.body_visible {
            None => boom > cull_at,
            Some(true) if boom < cull_at => false,
            Some(false) if boom > reveal_at => true,
            Some(v) => v,
        };
        self.body_visible = Some(bv);
        CameraPose {
            position,
            target,
            pivot,
            fov_deg: free_fov + (c::AIM_FOV - free_fov) * self.aim,
            boom,
            blocked,
            body_visible: bv,
            swing: self.swing,
            rise: self.rise,
            whisker,
            shoulder_side,
        }
    }
}

/// `WALK_PITCH_LIMITS` (player-camera-rig.js): up is negative.
pub const WALK_PITCH_MIN: f64 = -1.05;
pub const WALK_PITCH_MAX: f64 = 0.95;

/// `DRIVE_CAMERA` (player-camera-rig.js).
pub mod drive {
    pub const DISTANCE: f64 = 4.9;
    pub const HEIGHT: f64 = 1.7;
    pub const LOOK_AHEAD: f64 = 8.0;
    pub const TARGET_AHEAD: f64 = 4.0;
    pub const FRAME_ANGLE: f64 = 0.05;
    pub const PULL_IN_LIFT_RATE: f64 = 8.0;
    pub const MIN_STANDOFF: f64 = 3.2;
    pub const REST_FOV: f64 = 55.0;
    pub const FAST_FOV: f64 = 59.0;
    pub const FAST_FOV_AT_MPS: f64 = 28.0;
    pub const DEFAULT_PITCH: f64 = 0.16;
    pub const PITCH_MIN: f64 = -0.12;
    pub const PITCH_MAX: f64 = 0.9;
    pub const RECENTRE_DELAY_SEC: f64 = 1.5;
    pub const RECENTRE_RESPONSIVENESS: f64 = 4.5;
    pub const LOOK_BEHIND_RESPONSIVENESS: f64 = 14.0;
    /// `CAMERA_COLLISION`.
    pub const COLLISION_PADDING: f64 = 0.35;
    pub const COLLISION_MIN_DISTANCE: f64 = 1.1;
    /// `VEHICLE_CAMERA_SIZING` (drive-camera-framing.js).
    pub const ROOF_HEIGHT_SCALE: f64 = 1.5;
    pub const TALL_ROOF_HEIGHT_SCALE: f64 = 1.7;
    pub const TALL_BLEND_M: f64 = 0.8;
    pub const REFERENCE_HEIGHT_M: f64 = 1.8;
    pub const LIFT_REDUCTION_RAD_PER_M: f64 = 0.06;
    pub const MINIMUM_FRAME_ANGLE_RAD: f64 = 0.06;
    /// `DRIVE_CAMERA_FRAMING`.
    pub const FRAME_LIMIT: f64 = 0.92;
    pub const MINIMUM_DEPTH_M: f64 = 0.15;
    pub const SEARCH_STEPS: usize = 16;
    // the lower drive camera (the street-size choice (d)): rest height
    // 1.3 m (not 1.7), boom 4.6 m (not 4.9), pitch 0.10 rad (not 0.16), the roof
    // clearance 1.25 / 1.45 x the body height (not 1.5 / 1.7): about 1.8 m over the
    // road behind a sedan instead of about 2.2 m: the default; `--default-camera` gives the JS values above.
    fn low() -> bool {
        eo_data::groundscale::low_camera()
    }
    pub fn height() -> f64 {
        if low() { 1.3 } else { HEIGHT }
    }
    pub fn distance() -> f64 {
        if low() { 4.6 } else { DISTANCE }
    }
    pub fn default_pitch() -> f64 {
        if low() { 0.10 } else { DEFAULT_PITCH }
    }
    pub fn roof_height_scale() -> f64 {
        if low() { 1.25 } else { ROOF_HEIGHT_SCALE }
    }
    pub fn tall_roof_height_scale() -> f64 {
        if low() { 1.45 } else { TALL_ROOF_HEIGHT_SCALE }
    }
}

fn wrap_angle(a: f64) -> f64 {
    a.sin().atan2(a.cos())
}

/// `createDriveCameraState`.
#[derive(Clone, Debug)]
pub struct DriveCamera {
    pub orbit_yaw: f64,
    pub pitch: f64,
    pub idle_sec: f64,
    pub look_behind: bool,
    pub lift_scale: f64,
}

impl Default for DriveCamera {
    fn default() -> Self {
        Self { orbit_yaw: 0.0, pitch: drive::default_pitch(), idle_sec: drive::RECENTRE_DELAY_SEC, look_behind: false, lift_scale: 1.0 } // default_pitch()
    }
}

/// The vehicle as the drive camera sees it.
#[derive(Clone, Copy, Debug)]
pub struct DriveCameraVehicle {
    pub length_m: f64,
    pub width_m: f64,
    pub height_m: f64,
    /// Road datum (the body spans y = 0..height above it) and attitude.
    pub datum: DVec3,
    pub rotation: glam::DQuat,
}

#[derive(Clone, Copy, Debug)]
pub struct DrivePose {
    pub pivot: DVec3,
    pub position: DVec3,
    pub look_dir: DVec3,
    pub fov_deg: f64,
    pub framing_distance: Option<f64>,
}

/// `liftedAim`.
pub fn lifted_aim(position: DVec3, pivot: DVec3, lift: f64) -> DVec3 {
    let d = pivot - position;
    let horizontal = d.x.hypot(d.z);
    if !(horizontal > 1e-6) {
        return DVec3::new(0.0, if d.y >= 0.0 { 1.0 } else { -1.0 }, 0.0);
    }
    let aim = (-d.y).atan2(horizontal) - lift;
    let cos = aim.cos();
    DVec3::new(d.x / horizontal * cos, -aim.sin(), d.z / horizontal * cos)
}

/// `vehicleCameraSizing`: (heightOffsetM, frameAngleRad).
pub fn vehicle_camera_sizing(height_m: f64) -> (f64, f64) {
    use drive as d;
    let rest_height = d::height() + d::default_pitch().sin() * d::distance() * 0.6; // height(), default_pitch(), distance()
    let tall = ((height_m - d::REFERENCE_HEIGHT_M) / d::TALL_BLEND_M).clamp(0.0, 1.0);
    let roof_scale = d::roof_height_scale() + (d::tall_roof_height_scale() - d::roof_height_scale()) * tall;
    (
        (height_m * roof_scale - rest_height).max(0.0),
        d::MINIMUM_FRAME_ANGLE_RAD.max(d::FRAME_ANGLE - (height_m - d::REFERENCE_HEIGHT_M).max(0.0) * d::LIFT_REDUCTION_RAD_PER_M),
    )
}

impl DriveCamera {
    /// `noteDriveCameraPullIn`.
    pub fn note_pull_in(&mut self, kept_share: f64, delta: f64) {
        if !(delta > 0.0) || !kept_share.is_finite() {
            return;
        }
        self.lift_scale = damp(self.lift_scale, clamp(kept_share, 0.0, 1.0), drive::PULL_IN_LIFT_RATE, delta);
    }

    /// `applyDriveLook`.
    pub fn apply_look(&mut self, look_yaw: f64, look_pitch: f64) {
        if look_yaw == 0.0 && look_pitch == 0.0 {
            return;
        }
        self.orbit_yaw = wrap_angle(self.orbit_yaw + look_yaw);
        self.pitch = clamp(self.pitch + look_pitch, drive::PITCH_MIN, drive::PITCH_MAX);
        self.idle_sec = 0.0;
    }

    /// `updateDriveCamera`.
    pub fn update(&mut self, delta: f64, look_behind: bool) {
        let step = delta.max(0.0);
        self.look_behind = look_behind;
        let damp_r = |cur: f64, tgt: f64, resp: f64| cur + (tgt - cur) * (1.0 - (-resp * step).exp());
        if look_behind {
            let s = if self.orbit_yaw == 0.0 { 1.0 } else { self.orbit_yaw.signum() };
            self.orbit_yaw = wrap_angle(damp_r(self.orbit_yaw, std::f64::consts::PI * s, drive::LOOK_BEHIND_RESPONSIVENESS));
            self.idle_sec = 0.0;
            return;
        }
        self.idle_sec += step;
        if self.idle_sec >= drive::RECENTRE_DELAY_SEC {
            self.orbit_yaw = wrap_angle(damp_r(self.orbit_yaw, 0.0, drive::RECENTRE_RESPONSIVENESS));
            self.pitch = damp_r(self.pitch, drive::default_pitch(), drive::RECENTRE_RESPONSIVENESS); // default_pitch()
            if self.orbit_yaw.abs() < 1e-4 {
                self.orbit_yaw = 0.0;
            }
        }
    }

    /// `driveCameraPose` for a chassis at road datum `(x, y, z)` heading `yaw`.
    pub fn pose(&self, datum: DVec3, yaw: f64, speed: f64, vehicle: Option<&DriveCameraVehicle>, aspect: f64) -> DrivePose {
        use drive as d;
        let (height_offset, frame_angle) = vehicle.map(|v| vehicle_camera_sizing(v.height_m)).unwrap_or((0.0, d::FRAME_ANGLE));
        let pitch = clamp(self.pitch, d::PITCH_MIN, d::PITCH_MAX);
        let orbit = yaw + self.orbit_yaw;
        let pivot = DVec3::new(datum.x, datum.y + 1.2, datum.z);
        let lift = clamp(frame_angle, 0.0, d::FRAME_ANGLE) * clamp(self.lift_scale, 0.0, 1.0);
        let pose_at = |distance: f64| {
            let horizontal = pitch.cos() * distance;
            let position = DVec3::new(
                datum.x - orbit.sin() * horizontal,
                datum.y + d::height() + pitch.sin() * d::distance() * 0.6 + height_offset.max(0.0), // height(), distance()
                datum.z - orbit.cos() * horizontal,
            );
            (position, lifted_aim(position, pivot, lift))
        };
        let framing = vehicle.and_then(|v| fit_vehicle_camera_distance(d::distance(), v, aspect, d::REST_FOV, &pose_at)); // distance()
        let (position, look_dir) = pose_at(framing.unwrap_or(d::distance()));
        let fov = d::REST_FOV + (d::FAST_FOV - d::REST_FOV) * clamp(speed.abs() / d::FAST_FOV_AT_MPS, 0.0, 1.0);
        DrivePose { pivot, position, look_dir, fov_deg: fov, framing_distance: framing }
    }
}

/// `fitVehicleCameraDistance`: the boom that fits the body envelope in frame.
pub fn fit_vehicle_camera_distance(distance: f64, v: &DriveCameraVehicle, aspect: f64, fov_deg: f64, pose_at: &dyn Fn(f64) -> (DVec3, DVec3)) -> Option<f64> {
    use drive as d;
    let q = v.rotation.normalize();
    let mut corners = Vec::with_capacity(8);
    for x in [-v.width_m / 2.0, v.width_m / 2.0] {
        for y in [0.0, v.height_m] {
            for z in [-v.length_m / 2.0, v.length_m / 2.0] {
                corners.push(v.datum + q * DVec3::new(x, y, z));
            }
        }
    }
    let tangent = (fov_deg * std::f64::consts::PI / 360.0).tan();
    let project = |dist: f64| -> Option<bool> {
        let (position, forward) = pose_at(dist);
        let fl = forward.length();
        let horizontal = forward.x.hypot(forward.z);
        if fl < 1e-9 || horizontal < 1e-9 {
            return None;
        }
        let f = forward / fl;
        let right = DVec3::new(-forward.z / horizontal, 0.0, forward.x / horizontal);
        let up = DVec3::new(-right.z * f.y, right.z * f.x - right.x * f.z, right.x * f.y);
        let (mut mx, mut my) = (0.0f64, 0.0f64);
        for c in &corners {
            let delta = *c - position;
            let depth = delta.dot(f);
            if depth <= d::MINIMUM_DEPTH_M {
                return Some(false);
            }
            mx = mx.max((delta.dot(right) / (depth * tangent * aspect)).abs());
            my = my.max((delta.dot(up) / (depth * tangent)).abs());
        }
        Some(mx <= d::FRAME_LIMIT && my <= d::FRAME_LIMIT)
    };
    let before = project(distance)?;
    if before {
        return Some(distance);
    }
    let max_d = (distance * 3.0).max((v.length_m * v.length_m + v.width_m * v.width_m + v.height_m * v.height_m).sqrt() * 3.0);
    if project(max_d) != Some(true) {
        return Some(distance);
    }
    let (mut low, mut high) = (distance, max_d);
    for _ in 0..d::SEARCH_STEPS {
        let mid = (low + high) / 2.0;
        if project(mid) == Some(true) {
            high = mid;
        } else {
            low = mid;
        }
    }
    Some(high)
}

#[derive(Clone, Copy, Debug)]
pub struct ResolvedLens {
    pub position: DVec3,
    pub blocked: bool,
    pub distance: f64,
}

/// `resolveCameraCollision` (the ray path; the volume recovery is not ported).
pub fn resolve_camera_collision(probe: &mut dyn CameraProbe, pivot: DVec3, desired: DVec3, min_distance: f64, hard_occlusion: bool) -> ResolvedLens {
    let d = desired - pivot;
    let length = d.length();
    if !(length > 1e-6) {
        return ResolvedLens { position: desired, blocked: false, distance: length };
    }
    let dir = d / length;
    let padding = drive::COLLISION_PADDING;
    let hit = probe.probe(pivot, dir, length + padding, None);
    match hit {
        Some(h) if h.distance.is_finite() && h.distance < length + padding => {
            let distance = clamp(h.distance - padding, if hard_occlusion { 0.0 } else { min_distance }, length);
            ResolvedLens { position: pivot + dir * distance, blocked: true, distance }
        }
        _ => ResolvedLens { position: desired, blocked: false, distance: length },
    }
}
