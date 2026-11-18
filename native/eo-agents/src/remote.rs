//! Other players' bodies, from what the network says about them: a
//! position (on the smooth path `eo_net::snapshot::interpolate_smooth`
//! draws), the facing, the ground speed the server measured, a few state
//! flags and an emote. Nothing about their limbs crosses the wire; the
//! locomotion here turns those into a Rocketbox person's walk and standing
//! loops the way the street's people are moved (`motion.rs`):
//!
//! * the walk: a 100STYLE style drawn per player for its pace (the fastest
//!   street style when it sprints), its cycle advanced by the distance it
//!   covers over the stride that style makes on that body, so the planted
//!   foot does not slide; the shader blends walk and stand by the speed;
//! * speed eases (0.12 s up, 0.25 s down), so a start and a stop blend
//!   instead of snapping at the 20 Hz the snapshots arrive at;
//! * stopping: the cycle runs on to the next heel strike (phase 0 or 0.5,
//!   both feet down) as the walk fades, so a player never freezes mid-stride;
//! * turning on the spot: a body that turns faster than 0.8 rad/s while
//!   standing steps through the turn (a slow walk weight, 0.22 m of
//!   footwork a radian) instead of spinning on planted feet;
//! * standing: an idle loop drawn per player; an emote plays its loop
//!   from the moment it is first seen (the wave, the point, the photo, the
//!   crouch for a group's front row).
//!
//! What this cannot do (the baked tables blend one walk and one standing
//! frame per vertex): a run clip (none exists in the open sets on disk:
//! a sprint is the fastest walk style, stride-limited), cross-fading one
//! standing loop into another (an emote starts on a cut), foot IK on steps.

use crate::GpuAgent;
use crate::library::Library;
use crate::motion::Family;
use glam::Quat;

/// What the network says about one other player this frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct RemoteInput {
    /// Feet, game metres.
    pub feet: [f64; 3],
    /// Facing (the game's yaw: forward = (sin yaw, 0, cos yaw)).
    pub yaw: f64,
    /// Ground speed the server measured, m/s.
    pub speed: f64,
    pub sprint: bool,
    pub airborne: bool,
    /// The emote held (None: standing about).
    pub emote: Option<Family>,
    /// Changes each time an emote starts (a repeated one restarts).
    pub emote_count: u8,
    /// The player's id (draws its style and idle loop) and look (index into `Meta::people_models`).
    pub key: u32,
    pub look: usize,
    /// The world clock, seconds (standing loops' phase), and this frame's step.
    pub time: f64,
    pub dt: f64,
}

/// One other player's locomotion memory.
#[derive(Clone, Copy, Debug, Default)]
pub struct RemoteBody {
    pub shown_speed: f64,
    pub phase: f64,
    style: Option<(usize, bool)>,
    last_yaw: Option<f64>,
    pub yaw_rate: f64,
    emote: Option<(Family, u8, f64)>,
    pub stepping_turn: bool,
}

const RISE_S: f64 = 0.12;
const FALL_S: f64 = 0.25;
/// Under this the walk weight is nil (`gait::walk_weight`, the shader's 0.15 m/s).
const STAND_MPS: f64 = 0.15;
const TURN_STEP_RAD_PER_S: f64 = 0.8;
const TURN_FOOTWORK_M_PER_RAD: f64 = 0.22;

fn wrap(a: f64) -> f64 {
    (a + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
}

fn key_str(key: u32) -> String {
    format!("player-{key}")
}

impl RemoteBody {
    /// Advance by one frame and return the instance to draw (None without people).
    pub fn step(&mut self, lib: &Library, inp: &RemoteInput) -> Option<GpuAgent> {
        let meta = &lib.meta;
        if meta.people_models.is_empty() {
            return None;
        }
        let look = inp.look % meta.people_models.len();
        let mo = &meta.motion;
        let dt = inp.dt.max(0.0);
        let scale = eo_data::groundscale::people();
        // Speed, eased.
        let target = if inp.airborne { self.shown_speed } else { inp.speed.max(0.0) };
        let tau = if target > self.shown_speed { RISE_S } else { FALL_S };
        self.shown_speed += (target - self.shown_speed) * (1.0 - (-dt / tau).exp());
        if dt == 0.0 && self.last_yaw.is_none() {
            self.shown_speed = target;
        }
        // Turning on the spot.
        if let Some(y0) = self.last_yaw {
            if dt > 0.0 {
                let r = wrap(inp.yaw - y0) / dt;
                self.yaw_rate += (r - self.yaw_rate) * (1.0 - (-dt / 0.1).exp());
            }
        }
        self.last_yaw = Some(inp.yaw);
        self.stepping_turn = self.shown_speed < 0.3 && self.yaw_rate.abs() > TURN_STEP_RAD_PER_S && inp.emote.is_none();
        let footwork = if self.stepping_turn { (self.yaw_rate.abs() * TURN_FOOTWORK_M_PER_RAD).min(0.55) } else { 0.0 };
        let v = self.shown_speed.max(footwork);
        // The walk style for the pace (redrawn when it starts or stops sprinting).
        let (anim, speed_field) = if mo.styled() {
            let sprint = inp.sprint || v > 2.4;
            if self.style.is_none_or(|(_, s)| s != sprint) && v > 0.05 {
                let st = if sprint { mo.styles.iter().position(|s| s.name == "Rushed").unwrap_or(0) } else { mo.pick_style(look, v.max(1.0), scale as f32, false, &key_str(inp.key)) };
                self.style = Some((st, sprint));
            }
            let style = self.style.map(|s| s.0).unwrap_or(0);
            let stride = (mo.walk_of(look, style)[0] as f64 * scale).max(0.3);
            let mut dphase = v * dt / stride;
            // Stopping: run on to the next heel strike while the walk fades.
            if target < STAND_MPS && self.shown_speed < 0.6 && !self.stepping_turn {
                let next = if self.phase < 0.5 { 0.5 } else { 1.0 };
                let left = next - self.phase;
                if left > 1e-3 && left < 0.5 {
                    dphase = dphase.max((0.9 * dt).min(left));
                }
            }
            self.phase = (self.phase + dphase).rem_euclid(1.0);
            (mo.walk_anim(style, self.phase), v)
        } else {
            self.phase = crate::gait::advance_phase(self.phase, v, dt, meta.family_heights.first().copied().unwrap_or(1.75) as f64);
            (self.phase as f32, v)
        };
        // Standing: the emote's loop from when it was first seen, else an idle loop of its own.
        let emote = inp.emote.map(|f| (f, inp.emote_count));
        match (emote, self.emote) {
            (Some((f, c)), Some((g, d, _))) if f == g && c == d => {}
            (Some((f, c)), _) => self.emote = Some((f, c, inp.time)),
            (None, _) => self.emote = None,
        }
        let (clip, clip_phase) = match self.emote.and_then(|(f, _, t0)| mo.pick_loop(f, &key_str(inp.key)).map(|sl| (sl, t0))) {
            Some(((slot, l), t0)) => {
                let secs = mo.loop_seconds(slot, l).max(0.1);
                (slot as u32, mo.clip_phase(slot, l, (inp.time - t0) / secs))
            }
            None => match mo.pick_loop(Family::Idle, &key_str(inp.key)) {
                Some((slot, l)) => {
                    let secs = mo.loop_seconds(slot, l).max(0.1);
                    // Each player's idle at its own place in the loop.
                    let offset = crate::gait::LEG_OF_HEIGHT * (inp.key % 97) as f64;
                    (slot as u32, mo.clip_phase(slot, l, (inp.time + offset) / secs))
                }
                None => (crate::CLIP_IDLE, ((inp.time / meta.clip_seconds[crate::CLIP_IDLE as usize].max(0.1) as f64).rem_euclid(1.0)) as f32),
            },
        };
        Some(GpuAgent {
            pos: [inp.feet[0] as f32, inp.feet[1] as f32, inp.feet[2] as f32],
            model: meta.people_models[look],
            rot: Quat::from_rotation_y(inp.yaw as f32).to_array(),
            color: 0xffffffff,
            flags: crate::AGENT_PLAYER,
            anim,
            steer: 0.0,
            speed: speed_field as f32,
            scale: scale as f32,
            clip_phase,
            clip,
        })
    }
}

/// A player's emote byte (`eo_net::presence::Emote`, wire values 1..4) as a standing family.
pub fn emote_family(wire: u8) -> Option<Family> {
    match wire & 0x0F {
        1 => Some(Family::Wave),
        2 => Some(Family::Point),
        3 => Some(Family::Sit),
        4 => Some(Family::Photo),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A remote walker that stops ends on a heel strike, its walk faded out,
    /// and the speed it is drawn at never steps by more than the ease allows.
    #[test]
    fn a_stop_settles_on_a_planted_foot_and_eases() {
        let mut lib = Library::default();
        lib.meta.people_models = vec![0];
        let mut m = crate::motion::MotionMeta { walk: "100style".into(), walk_frames: crate::motion::WALK_FRAMES, ..Default::default() };
        m.styles = vec![crate::motion::StyleMeta { name: "Neutral".into(), ..Default::default() }];
        m.person_group = vec![0];
        m.group_walk = vec![vec![[1.4, 1.3]]];
        m.slot_frames = [33, 0, 0, 0];
        lib.meta.motion = m;
        let mut b = RemoteBody::default();
        let dt = 1.0 / 60.0;
        let mut last_speed = 0.0;
        for f in 0..600 {
            let t = f as f64 * dt;
            let speed = if t < 4.0 { 1.4 } else { 0.0 };
            let a = b.step(&lib, &RemoteInput { feet: [t * speed, 0.0, 0.0], yaw: 1.57, speed, key: 7, time: t, dt, ..Default::default() }).unwrap();
            let ds = (a.speed as f64 - last_speed).abs();
            assert!(ds < 1.4 * dt / RISE_S + 1e-6, "frame {f}: the drawn speed stepped by {ds}");
            last_speed = a.speed as f64;
        }
        assert!(b.shown_speed < STAND_MPS, "the walk faded out ({})", b.shown_speed);
        let to_strike = b.phase.min((b.phase - 0.5).abs()).min(1.0 - b.phase);
        assert!(to_strike < 0.02, "stopped at phase {} (not on a heel strike)", b.phase);
        // Turning on the spot steps; standing still does not.
        let mut c = RemoteBody::default();
        for f in 0..60 {
            c.step(&lib, &RemoteInput { yaw: f as f64 * 0.05, key: 3, time: f as f64 * dt, dt, ..Default::default() });
        }
        assert!(c.stepping_turn, "a 3 rad/s turn steps (rate {})", c.yaw_rate);
        assert_eq!(emote_family(1), Some(Family::Wave));
        assert_eq!(emote_family(0x34), Some(Family::Photo));
    }
}
