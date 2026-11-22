//! Landmark lifts: the ride between a landmark's ground entrance and its
//! deck (`world::LiftDef`, from eo-landmarks access.rs through the host).
//!
//! On foot within reach of a stop, the interact action (`Action::Interact`,
//! E; the pad's Y) starts a ride. The ride is counted in fixed steps and
//! reads no clock: the screen fades out (`FADE_OUT_STEPS`), holds black
//! (`HOLD_STEPS`; the body is moved to the other stop halfway through it)
//! and fades back in (`FADE_IN_STEPS`), with the body held still and the
//! stick ignored. The ride is part of the player's state, so a server runs
//! it from the same input on the same step, a correction carries it, and a
//! rewind replays it (eo-net `wire::write_player_state`).

use glam::DVec3;

use crate::world::{CollisionWorld, LiftDef};

/// 0.5 s out, 0.5 s held dark, 0.5 s back in, at 60 steps a second.
pub const FADE_OUT_STEPS: u32 = 30;
pub const HOLD_STEPS: u32 = 30;
pub const FADE_IN_STEPS: u32 = 30;
pub const RIDE_STEPS: u32 = FADE_OUT_STEPS + HOLD_STEPS + FADE_IN_STEPS;
/// The step of the ride the body changes stops on (halfway through the dark).
pub const MOVE_STEP: u32 = FADE_OUT_STEPS + HOLD_STEPS / 2;
/// A stop's reach: horizontal distance and height from its feet (eo-landmarks `STOP_REACH_M`, `STOP_RISE_M`).
pub const REACH_M: f64 = 1.8;
pub const RISE_M: f64 = 2.0;
/// Coming down, the body is let go this far over the higher of the stop
/// and the ground provider's height, and settles onto what is there.
pub const DROP_M: f64 = 0.25;

/// A ride in progress.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LiftRide {
    /// Index into the world's lifts.
    pub lift: u32,
    /// Riding up (from the bottom stop).
    pub up: bool,
    /// Steps since it started.
    pub step: u32,
}

/// What the interface says near a stop.
#[derive(Clone, Debug, PartialEq)]
pub struct LiftPrompt {
    pub name: String,
    pub up: bool,
}

/// The lift whose stop `feet` stand at, and whether it goes up from there:
/// the nearest stop in reach, first lift first on a tie.
pub fn stop_near(lifts: &[LiftDef], feet: DVec3) -> Option<(usize, bool)> {
    let mut best: Option<(usize, bool, f64)> = None;
    for (i, l) in lifts.iter().enumerate() {
        for (stop, up) in [(l.bottom, true), (l.top, false)] {
            let d = (feet.x - stop.x).hypot(feet.z - stop.z);
            if d <= REACH_M && (feet.y - stop.y).abs() <= RISE_M && best.is_none_or(|b| d < b.2) {
                best = Some((i, up, d));
            }
        }
    }
    best.map(|b| (b.0, b.1))
}

/// Where a ride leaves the body (feet) and the yaw it faces.
pub fn arrival(world: &CollisionWorld, l: &LiftDef, up: bool) -> (DVec3, f64) {
    if up {
        (l.top + DVec3::Y * 0.02, l.top_yaw)
    } else {
        let g = world.ground.ground_y(l.bottom.x, l.bottom.z);
        (DVec3::new(l.bottom.x, l.bottom.y.max(g) + DROP_M, l.bottom.z), l.bottom_yaw)
    }
}

/// The screen's darkness at this ride step, 0 (clear) to 1 (black).
pub fn fade(ride: Option<LiftRide>) -> f64 {
    let Some(r) = ride else { return 0.0 };
    let s = r.step as f64;
    let t = if r.step < FADE_OUT_STEPS {
        s / FADE_OUT_STEPS as f64
    } else if r.step < FADE_OUT_STEPS + HOLD_STEPS {
        1.0
    } else {
        1.0 - (s - (FADE_OUT_STEPS + HOLD_STEPS) as f64) / FADE_IN_STEPS as f64
    };
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
