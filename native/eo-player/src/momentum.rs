//! On-foot momentum: port of `stepLocomotionMomentum` / `settleLocomotionMomentum`
//! (src/player/locomotion-momentum.js), and the nine-way stick resolver and
//! facing solve of src/player/nine-way-locomotion.js. Pure, f64, step for step
//! the JS arithmetic (compared with node-run traces in tests/js_oracle.rs).

/// `LOCOMOTION_MOMENTUM`.
pub mod c {
    pub const ACCEL_MPS2: f64 = 11.0;
    pub const BRAKE_MPS2: f64 = 20.0;
    pub const TURN_MPS2: f64 = 24.0;
    pub const PIVOT_SPEED_MPS: f64 = 0.35;
    pub const REVERSE_RAD: f64 = 2.4;
    pub const REVERSE_BRAKE_MPS2: f64 = 26.0;
    pub const PLANT_TURN_RAD_PER_S: f64 = 14.0;
    pub const COMBAT_ACCEL_MPS2: f64 = 18.0;
    pub const COMBAT_BRAKE_MPS2: f64 = 26.0;
    pub const COMBAT_TURN_MPS2: f64 = 30.0;
    pub const TELEPORT_M: f64 = 0.75;
}

#[derive(Clone, Debug, Default)]
pub struct Momentum {
    pub vx: f64,
    pub vz: f64,
    pub ax: f64,
    pub az: f64,
    pub last: Option<(f64, f64)>,
    /// A planted reversal's new heading (the facing turns onto it while braking).
    pub plant_yaw: Option<f64>,
}

fn approach(current: f64, target: f64, max_step: f64) -> f64 {
    let diff = target - current;
    if diff.abs() <= max_step {
        return target;
    }
    current + diff.signum() * max_step
}

impl Momentum {
    pub fn reset(&mut self) {
        *self = Momentum::default();
    }

    /// One step toward `desired` (m/s, world xz); returns the displacement to
    /// request from the character controller.
    pub fn step(&mut self, desired_x: f64, desired_z: f64, dt: f64, combat: bool, x: f64, z: f64) -> (f64, f64) {
        if let Some((lx, lz)) = self.last {
            if (x - lx).hypot(z - lz) > c::TELEPORT_M {
                self.reset();
            }
        }
        if dt <= 0.0 {
            return (0.0, 0.0);
        }
        let accel = if combat { c::COMBAT_ACCEL_MPS2 } else { c::ACCEL_MPS2 };
        let brake = if combat { c::COMBAT_BRAKE_MPS2 } else { c::BRAKE_MPS2 };
        let turn = if combat { c::COMBAT_TURN_MPS2 } else { c::TURN_MPS2 };
        let (vx0, vz0) = (self.vx, self.vz);
        let speed = vx0.hypot(vz0);
        let want = desired_x.hypot(desired_z);
        let mut heading = if speed > 1e-4 { vx0.atan2(vz0) } else { desired_x.atan2(desired_z) };
        let next_speed;
        self.plant_yaw = None;
        if speed < c::PIVOT_SPEED_MPS {
            if want > 0.0 {
                heading = desired_x.atan2(desired_z);
            }
            next_speed = approach(speed, want, if want > speed { accel } else { brake } * dt);
        } else {
            let want_heading = if want > 0.0 { desired_x.atan2(desired_z) } else { heading };
            let error = (want_heading - heading).sin().atan2((want_heading - heading).cos());
            if want > 0.0 && error.abs() > c::REVERSE_RAD {
                next_speed = approach(speed, 0.0, if combat { brake } else { brake.max(c::REVERSE_BRAKE_MPS2) } * dt);
                if !combat {
                    self.plant_yaw = Some(want_heading);
                }
            } else {
                let omega = turn / speed.max(1.0);
                heading += error.signum() * error.abs().min(omega * dt);
                next_speed = approach(speed, want, if want > speed { accel } else { brake } * dt);
            }
        }
        let mut vx = heading.sin() * next_speed;
        let mut vz = heading.cos() * next_speed;
        if vx.hypot(vz) < 1e-3 && desired_x.hypot(desired_z) < 1e-6 {
            vx = 0.0;
            vz = 0.0;
        }
        self.ax = (vx - vx0) / dt;
        self.az = (vz - vz0) / dt;
        self.vx = vx;
        self.vz = vz;
        (vx * dt, vz * dt)
    }

    /// Feed back what the controller achieved: a wall that stopped the body
    /// also stops the momentum (slides along it survive).
    pub fn settle(&mut self, moved_x: f64, moved_z: f64, dt: f64, x: f64, z: f64) {
        if dt > 0.0 {
            let (ax, az) = (moved_x / dt, moved_z / dt);
            let achieved = ax.hypot(az);
            let commanded = self.vx.hypot(self.vz);
            let (rx, rz) = (self.vx, self.vz);
            let reference = rx.hypot(rz);
            if commanded > 1e-4 && reference > 1e-4 && achieved < reference - 0.05 {
                let scale = ((ax * rx + az * rz) / (reference * reference)).max(0.0);
                self.vx *= scale.min(1.0);
                self.vz *= scale.min(1.0);
            }
        }
        self.last = Some((x, z));
    }
}

/// `resolveNineWayLocomotion`: the stick in world space for a camera yaw.
/// Returns (x, z); (0, 0) at idle.
pub fn nine_way(forward: bool, backward: bool, left: bool, right: bool, camera_yaw: f64) -> (f64, f64) {
    let forward_axis = forward as i32 as f64 - backward as i32 as f64;
    let strafe_axis = right as i32 as f64 - left as i32 as f64;
    if forward_axis == 0.0 && strafe_axis == 0.0 {
        return (0.0, 0.0);
    }
    let diagonal = forward_axis != 0.0 && strafe_axis != 0.0;
    let k = if diagonal { std::f64::consts::FRAC_1_SQRT_2 } else { 1.0 };
    let (f, s) = (forward_axis * k, strafe_axis * k);
    (camera_yaw.sin() * f - camera_yaw.cos() * s, camera_yaw.cos() * f + camera_yaw.sin() * s)
}

/// Analog stick (x right, y forward, magnitude <= 1) in world space.
pub fn stick_world(x: f64, y: f64, camera_yaw: f64) -> (f64, f64) {
    (camera_yaw.sin() * y - camera_yaw.cos() * x, camera_yaw.cos() * y + camera_yaw.sin() * x)
}

/// `dampLocomotionYaw`.
pub fn damp_yaw(current: f64, target: f64, dt: f64, responsiveness: f64) -> f64 {
    let d = (target - current).sin().atan2((target - current).cos());
    current + d * (1.0 - (-responsiveness.max(0.0) * dt.max(0.0)).exp())
}

/// `resolvePlayerFacingYaw` with `idleFollowsCamera: false` (as main.js calls it).
pub fn facing_yaw(current: f64, camera: f64, travel: Option<f64>, aiming: bool, dt: f64, max_rate: f64) -> f64 {
    let target = if aiming {
        camera
    } else if let Some(t) = travel {
        t
    } else {
        current
    };
    let next = damp_yaw(current, target, dt, if aiming { 18.0 } else { 12.0 });
    if max_rate.is_finite() {
        let max = max_rate * dt.max(0.0);
        let d = (next - current).sin().atan2((next - current).cos());
        if d.abs() > max {
            return current + d.signum() * max;
        }
    }
    next
}
