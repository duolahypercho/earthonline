//! Client-side prediction and server reconciliation of the local player.
//!
//! The client steps its own player at once with every input it sends (no
//! waiting a round trip), keeps each unacknowledged input with the position
//! it predicted, and compares with what the server reports for the same
//! input sequence. Because the step is deterministic and the client steps
//! with the quantised input the server receives, the two agree bit for bit
//! in the normal case and nothing is corrected. When they do not (the
//! server substituted a lost input, or the worlds differ), the server sends
//! its whole player state after input `k`; the client puts its player in
//! that state and replays inputs `k+1..` again. The jump that makes on
//! screen is not shown at once: the drawn body and camera carry an offset
//! that decays over `SMOOTH_S`.

use crate::game::{check_of, step_player};
use crate::snapshot::{Correction, Own};
use crate::wire::{NetInput, read_player_state};
use eo_player::world::CollisionWorld;
use eo_player::{Player, StepInput};
use glam::DVec3;
use std::collections::VecDeque;
use std::time::Instant;

pub const STEP_S: f64 = eo_player::tuning::FIXED_STEP_S;
/// The visible correction decays with this time constant.
pub const SMOOTH_S: f64 = 0.1;
/// Fixed steps one drawn frame may catch up at most.
pub const MAX_STEPS_PER_FRAME: usize = 8;

struct Hist {
    seq: u32,
    input: NetInput,
    pos_mm: [i32; 3],
}

#[derive(Clone, Debug, Default)]
pub struct PredictStats {
    pub steps: u64,
    pub corrections: u64,
    pub replayed_steps: u64,
    pub replay_ms_max: f64,
    /// |server - predicted| at each acknowledged input, mm (after any correction).
    pub error_mm: Vec<f64>,
    /// The visible jump each correction would have made, m (before smoothing).
    pub correction_m: Vec<f64>,
}

pub struct Predictor {
    pub player: Player,
    pub cw: CollisionWorld,
    /// The last input sequence stepped.
    pub seq: u32,
    history: VecDeque<Hist>,
    acc: f64,
    tap_jump: bool,
    tap_toggle: bool,
    /// Drawn offset left by the last correction (decays to zero).
    pub smooth: DVec3,
    /// The newest correction applied.
    pub applied: u16,
    pub stats: PredictStats,
}

impl Predictor {
    pub fn new(player: Player, cw: CollisionWorld, seq: u32) -> Self {
        Self { player, cw, seq, history: VecDeque::new(), acc: 0.0, tap_jump: false, tap_toggle: false, smooth: DVec3::ZERO, applied: 0, stats: PredictStats::default() }
    }

    /// Reset to a player rebuilt from the state the server sent (a reconnect),
    /// in the world it was rebuilt in.
    pub fn reset(&mut self, player: Player, cw: CollisionWorld, seq: u32) {
        self.player = player;
        self.cw = cw;
        self.seq = seq;
        self.history.clear();
        self.acc = 0.0;
        self.smooth = DVec3::ZERO;
    }

    /// One drawn frame: run the fixed steps due with this frame's input (taps
    /// act on the first step that runs), streaming the collision tiles after.
    /// The new inputs are appended to `out` to be sent.
    pub fn frame(&mut self, input: &StepInput, dt: f64, out: &mut Vec<(u32, NetInput)>) -> usize {
        self.acc += dt.clamp(0.0, 0.25);
        self.tap_jump |= input.jump;
        self.tap_toggle |= input.vehicle_toggle;
        let mut n = 0;
        while self.acc >= STEP_S - 1e-9 {
            self.acc -= STEP_S;
            let mut s = *input;
            s.jump = std::mem::take(&mut self.tap_jump);
            s.vehicle_toggle = std::mem::take(&mut self.tap_toggle);
            let q = NetInput::quantize(&s, self.player.yaw);
            self.seq = self.seq.wrapping_add(1);
            step_player(&mut self.player, &mut self.cw, &q);
            let c = check_of(self.seq, &self.player, &self.cw);
            self.history.push_back(Hist { seq: self.seq, input: q, pos_mm: c.pos_mm });
            if self.history.len() > 600 {
                self.history.pop_front();
            }
            out.push((self.seq, q));
            self.stats.steps += 1;
            n += 1;
            if n >= MAX_STEPS_PER_FRAME {
                self.acc = self.acc.min(STEP_S);
                break;
            }
        }
        let p = self.player.position(&self.cw);
        self.cw.pump(p);
        n
    }

    /// The check of the newest predicted step (sent with the inputs).
    pub fn check(&self) -> Option<crate::wire::Check> {
        self.history.back().map(|h| crate::wire::Check { seq: h.seq, pos_mm: h.pos_mm, driving: matches!(self.player.mode, eo_player::Mode::Driving(_)) })
    }

    /// What the server reported for this client's player: the error metric,
    /// and inputs it has run are forgotten.
    pub fn on_own(&mut self, own: &Own) {
        if let Some(h) = self.history.iter().find(|h| h.seq == own.ack_input) {
            let d = (0..3).map(|k| ((h.pos_mm[k] - own.check.pos_mm[k]) as f64).powi(2)).sum::<f64>().sqrt();
            self.stats.error_mm.push(d);
        }
        while self.history.front().is_some_and(|h| h.seq <= own.ack_input) {
            self.history.pop_front();
        }
    }

    /// Apply a correction (once): the server's state after input `c.seq`,
    /// then every later input again. Returns the visible jump, metres.
    pub fn correct(&mut self, c: &Correction) -> Option<f64> {
        if c.id == 0 || (c.id.wrapping_sub(self.applied) as i16) <= 0 {
            return None;
        }
        let state = read_player_state(&c.state).ok()?;
        let t0 = Instant::now();
        let before = self.player.position(&self.cw) + self.smooth;
        self.player.set_state(&mut self.cw, &state);
        let mut replayed = 0;
        for h in self.history.iter_mut() {
            if h.seq < c.seq {
                continue;
            }
            if h.seq == c.seq {
                // The server's own state at that input is what the client now holds for it.
                h.pos_mm = check_of(h.seq, &self.player, &self.cw).pos_mm;
                continue;
            }
            step_player(&mut self.player, &mut self.cw, &h.input);
            h.pos_mm = check_of(h.seq, &self.player, &self.cw).pos_mm;
            replayed += 1;
        }
        let after = self.player.position(&self.cw);
        if std::env::var_os("EO_NET_DEBUG").is_some() {
            eprintln!("client: correction {} at seq {} (state pos {:?}), replayed {replayed} to seq {}, before {before:?} after {after:?}", c.id, c.seq, state.character.position, self.seq);
        }
        self.smooth = before - after;
        self.applied = c.id;
        let jump = self.smooth.length();
        self.stats.corrections += 1;
        self.stats.replayed_steps += replayed;
        self.stats.correction_m.push(jump);
        self.stats.replay_ms_max = self.stats.replay_ms_max.max(t0.elapsed().as_secs_f64() * 1e3);
        Some(jump)
    }

    /// The drawn offset this frame (decaying), metres.
    pub fn offset(&mut self, dt: f64) -> DVec3 {
        self.smooth *= (-dt / SMOOTH_S).exp();
        if self.smooth.length() < 1e-4 {
            self.smooth = DVec3::ZERO;
        }
        self.smooth
    }

    pub fn unacked(&self) -> usize {
        self.history.len()
    }
}
