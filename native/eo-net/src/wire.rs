//! What a client sends (one quantised input per fixed step, resent until
//! the server has run it) and the player state a server sends back when the
//! client's prediction has gone wrong.
//!
//! QUANTISATION IS PART OF THE SIMULATION: a client steps its own player
//! with the quantised input it sends (`NetInput::step_input`, `NetInput::yaw`),
//! never with the raw device values, so the server, running the same input
//! through the same deterministic step, arrives at the same body.

use crate::bits::{BitReader, BitWriter, ByteReader, PutLe, Truncated};
use eo_player::character::CharacterState;
use eo_player::vehicle::VehicleState;
use eo_player::{PlayerState, StepInput};

pub mod button {
    pub const SPRINT: u16 = 1;
    pub const CROUCH: u16 = 2;
    pub const JUMP: u16 = 4;
    pub const AIM: u16 = 8;
    pub const VEHICLE_TOGGLE: u16 = 16;
    pub const ANALOG_STEER: u16 = 32;
    pub const HANDBRAKE: u16 = 64;
    pub const LOOK_BEHIND: u16 = 128;
    /// A tap: ride a landmark lift (eo-player lift.rs).
    pub const INTERACT: u16 = 256;
    /// The taps (act on one step; a stood-in input drops them).
    pub const TAPS: u16 = JUMP | VEHICLE_TOGGLE | INTERACT;
}

/// One fixed step's input as sent (9 bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NetInput {
    pub move_x: i8,
    pub move_y: i8,
    pub throttle: u8,
    pub brake: u8,
    pub steer: i8,
    pub buttons: u16,
    /// Camera yaw, 2 pi / 65536.
    pub yaw: u16,
}

fn q_signed(v: f64) -> i8 {
    (v.clamp(-1.0, 1.0) * 127.0).round() as i8
}

fn q_unit(v: f64) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

pub fn quantize_yaw(yaw: f64) -> u16 {
    (yaw.rem_euclid(std::f64::consts::TAU) / std::f64::consts::TAU * 65536.0).round() as u32 as u16
}

pub fn yaw_of(q: u16) -> f64 {
    let a = q as f64 * std::f64::consts::TAU / 65536.0;
    if a > std::f64::consts::PI { a - std::f64::consts::TAU } else { a }
}

impl NetInput {
    pub fn quantize(s: &StepInput, yaw: f64) -> Self {
        let mut b = 0u16;
        for (on, bit) in [
            (s.sprint, button::SPRINT),
            (s.crouch, button::CROUCH),
            (s.jump, button::JUMP),
            (s.aim, button::AIM),
            (s.vehicle_toggle, button::VEHICLE_TOGGLE),
            (s.analog_steer, button::ANALOG_STEER),
            (s.handbrake, button::HANDBRAKE),
            (s.look_behind, button::LOOK_BEHIND),
            (s.interact, button::INTERACT),
        ] {
            if on {
                b |= bit;
            }
        }
        NetInput { move_x: q_signed(s.move_x), move_y: q_signed(s.move_y), throttle: q_unit(s.throttle), brake: q_unit(s.brake), steer: q_signed(s.steer), buttons: b, yaw: quantize_yaw(yaw) }
    }

    pub fn step_input(&self) -> StepInput {
        let b = |bit: u16| self.buttons & bit != 0;
        StepInput {
            move_x: self.move_x as f64 / 127.0,
            move_y: self.move_y as f64 / 127.0,
            sprint: b(button::SPRINT),
            crouch: b(button::CROUCH),
            jump: b(button::JUMP),
            aim: b(button::AIM),
            vehicle_toggle: b(button::VEHICLE_TOGGLE),
            throttle: self.throttle as f64 / 255.0,
            brake: self.brake as f64 / 255.0,
            steer: self.steer as f64 / 127.0,
            analog_steer: b(button::ANALOG_STEER),
            handbrake: b(button::HANDBRAKE),
            look_behind: b(button::LOOK_BEHIND),
            interact: b(button::INTERACT),
        }
    }

    pub fn yaw(&self) -> f64 {
        yaw_of(self.yaw)
    }

    fn fields(&self) -> [u32; 7] {
        [self.move_x as u8 as u32, self.move_y as u8 as u32, self.throttle as u32, self.brake as u32, self.steer as u8 as u32, self.buttons as u32, self.yaw as u32]
    }
}

const FIELD_BITS: [u32; 7] = [8, 8, 8, 8, 8, 16, 16];

/// The client's position check for one step: the server compares it with
/// its own body after the same step and sends a correction when they differ.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Check {
    pub seq: u32,
    /// Feet on foot, the car's road datum when driving: millimetres.
    pub pos_mm: [i32; 3],
    pub driving: bool,
}

/// A client's input datagram payload.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InputPacket {
    /// Newest snapshot the client decoded (0: none).
    pub ack_snapshot: u32,
    /// The newest correction the client applied (its checks before it are stale).
    pub correction_applied: u16,
    pub check: Option<Check>,
    /// Inputs `first_seq, first_seq + 1, ...` (every input the server has not acknowledged, up to `MAX_INPUTS`).
    pub first_seq: u32,
    pub inputs: Vec<NetInput>,
    /// The client's clock when sent (ms, wrapping): the server echoes the newest
    /// with how long it held it, so ping excludes the wait for the next snapshot.
    pub ping_ms: u16,
}

/// The most inputs one packet resends (0.5 s at 60 Hz).
pub const MAX_INPUTS: usize = 30;

impl InputPacket {
    pub fn write(&self, out: &mut Vec<u8>) {
        let mut w = BitWriter::with_capacity(64);
        w.write(self.ack_snapshot, 32);
        w.write(self.correction_applied as u32, 16);
        w.write(self.ping_ms as u32, 16);
        match self.check {
            Some(c) => {
                w.bit(true);
                w.write(c.seq, 32);
                for v in c.pos_mm {
                    w.write(v as u32, 32);
                }
                w.bit(c.driving);
            }
            None => w.bit(false),
        }
        w.write(self.first_seq, 32);
        w.write(self.inputs.len() as u32, 6);
        let mut prev: Option<[u32; 7]> = None;
        for i in &self.inputs {
            let f = i.fields();
            match prev {
                Some(p) => {
                    let mut mask = 0u32;
                    for k in 0..7 {
                        if f[k] != p[k] {
                            mask |= 1 << k;
                        }
                    }
                    if mask == 0 {
                        w.bit(false);
                    } else {
                        w.bit(true);
                        w.write(mask, 7);
                        for k in 0..7 {
                            if mask & (1 << k) != 0 {
                                w.write(f[k], FIELD_BITS[k]);
                            }
                        }
                    }
                }
                None => {
                    for k in 0..7 {
                        w.write(f[k], FIELD_BITS[k]);
                    }
                }
            }
            prev = Some(f);
        }
        out.extend_from_slice(&w.finish());
    }

    pub fn read(data: &[u8]) -> Result<Self, Truncated> {
        let mut r = BitReader::new(data);
        let ack_snapshot = r.read(32)?;
        let correction_applied = r.read(16)? as u16;
        let ping_ms = r.read(16)? as u16;
        let check = if r.bit()? { Some(Check { seq: r.read(32)?, pos_mm: [r.read(32)? as i32, r.read(32)? as i32, r.read(32)? as i32], driving: r.bit()? }) } else { None };
        let first_seq = r.read(32)?;
        let n = r.read(6)? as usize;
        let mut inputs = Vec::with_capacity(n);
        let mut prev = [0u32; 7];
        for k in 0..n {
            let mut f = prev;
            if k == 0 {
                for j in 0..7 {
                    f[j] = r.read(FIELD_BITS[j])?;
                }
            } else if r.bit()? {
                let mask = r.read(7)?;
                for j in 0..7 {
                    if mask & (1 << j) != 0 {
                        f[j] = r.read(FIELD_BITS[j])?;
                    }
                }
            }
            inputs.push(NetInput { move_x: f[0] as u8 as i8, move_y: f[1] as u8 as i8, throttle: f[2] as u8, brake: f[3] as u8, steer: f[4] as u8 as i8, buttons: f[5] as u16, yaw: f[6] as u16 });
            prev = f;
        }
        Ok(InputPacket { ack_snapshot, correction_applied, check, first_seq, inputs, ping_ms })
    }
}

/// The position a check compares (millimetres).
pub fn check_pos(p: glam::DVec3) -> [i32; 3] {
    [(p.x * 1000.0).round() as i32, (p.y * 1000.0).round() as i32, (p.z * 1000.0).round() as i32]
}

// ---------------------------------------------------------------- player state, exact

fn put_opt2(out: &mut Vec<u8>, v: Option<(f64, f64)>) {
    match v {
        Some((a, b)) => {
            out.put_u8(1);
            out.put_f64(a);
            out.put_f64(b);
        }
        None => out.put_u8(0),
    }
}

/// The whole player state, every float bit for bit (a correction must put
/// the client exactly where the server is).
pub fn write_player_state(s: &PlayerState, out: &mut Vec<u8>) {
    let c = &s.character;
    for v in c.position {
        out.put_f64(v);
    }
    out.put_f64(c.vertical_velocity);
    out.put_u8(c.grounded as u8 | (c.crouched as u8) << 1);
    out.put_f64(c.half_height);
    out.put_u32(c.perched_steps);
    out.put_u32(c.perch_lift_cooldown);
    out.put_f64(c.kerb_step_lift);
    out.put_u32(c.kerb_hold_steps);
    out.put_u32(c.kerb_blocked_steps);
    for v in s.momentum {
        out.put_f64(v);
    }
    put_opt2(out, s.momentum_last);
    put_opt2(out, s.plant_yaw.map(|y| (y, 0.0)));
    for v in [s.yaw, s.pitch, s.facing_yaw, s.sprint_held_s, s.sprint, s.ground_speed] {
        out.put_f64(v);
    }
    out.put_u32(s.driving.map(|d| d + 1).unwrap_or(0));
    out.put_u64(s.ticks);
    out.put_u8(s.vehicles.len() as u8);
    for v in &s.vehicles {
        for x in v.translation.iter().chain(v.rotation.iter()).chain(v.linvel.iter()).chain(v.angvel.iter()) {
            out.put_f32(*x);
        }
        out.put_u8(v.sleeping as u8 | (v.handbrake as u8) << 1);
        out.put_f32(v.time_since_can_sleep);
        out.put_f64(v.steer_normalized);
        for x in v.controls {
            out.put_f64(x);
        }
    }
    // a landmark lift ride (eo-player lift.rs).
    match s.lift {
        Some(r) => {
            out.put_u8(1 | (r.up as u8) << 1);
            out.put_u32(r.lift);
            out.put_u32(r.step);
        }
        None => out.put_u8(0),
    }
}

pub fn read_player_state(data: &[u8]) -> Result<PlayerState, Truncated> {
    let mut r = ByteReader::new(data);
    let opt2 = |r: &mut ByteReader| -> Result<Option<(f64, f64)>, Truncated> { if r.u8()? != 0 { Ok(Some((r.f64()?, r.f64()?))) } else { Ok(None) } };
    let position = [r.f64()?, r.f64()?, r.f64()?];
    let vertical_velocity = r.f64()?;
    let gc = r.u8()?;
    let character = CharacterState {
        position,
        vertical_velocity,
        grounded: gc & 1 != 0,
        crouched: gc & 2 != 0,
        half_height: r.f64()?,
        perched_steps: r.u32()?,
        perch_lift_cooldown: r.u32()?,
        kerb_step_lift: r.f64()?,
        kerb_hold_steps: r.u32()?,
        kerb_blocked_steps: r.u32()?,
    };
    let momentum = [r.f64()?, r.f64()?, r.f64()?, r.f64()?];
    let momentum_last = opt2(&mut r)?;
    let plant_yaw = opt2(&mut r)?.map(|p| p.0);
    let mut f = [0.0; 6];
    for v in f.iter_mut() {
        *v = r.f64()?;
    }
    let d = r.u32()?;
    let ticks = r.u64()?;
    let n = r.u8()? as usize;
    let mut vehicles = Vec::with_capacity(n);
    for _ in 0..n {
        let mut x = [0f32; 13];
        for v in x.iter_mut() {
            *v = r.f32()?;
        }
        let flags = r.u8()?;
        vehicles.push(VehicleState {
            translation: [x[0], x[1], x[2]],
            rotation: [x[3], x[4], x[5], x[6]],
            linvel: [x[7], x[8], x[9]],
            angvel: [x[10], x[11], x[12]],
            sleeping: flags & 1 != 0,
            handbrake: flags & 2 != 0,
            time_since_can_sleep: r.f32()?,
            steer_normalized: r.f64()?,
            controls: [r.f64()?, r.f64()?, r.f64()?],
        });
    }
    let lf = r.u8()?;
    let lift = if lf & 1 != 0 { Some(eo_player::lift::LiftRide { up: lf & 2 != 0, lift: r.u32()?, step: r.u32()? }) } else { None };
    Ok(PlayerState {
        character,
        momentum,
        momentum_last,
        plant_yaw,
        yaw: f[0],
        pitch: f[1],
        facing_yaw: f[2],
        sprint_held_s: f[3],
        sprint: f[4],
        ground_speed: f[5],
        driving: if d == 0 { None } else { Some(d - 1) },
        ticks,
        vehicles,
        lift,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inputs_round_trip_and_quantise_exactly() {
        let s = StepInput { move_x: -1.0, move_y: 1.0, sprint: true, throttle: 0.5, steer: 0.3, ..Default::default() };
        let q = NetInput::quantize(&s, -2.5);
        let back = q.step_input();
        assert_eq!(back.move_x, -1.0);
        assert_eq!(back.move_y, 1.0);
        assert!(back.sprint);
        assert_eq!(NetInput::quantize(&back, q.yaw()), q, "quantising a dequantised input is the identity");
        let mut p = InputPacket { ack_snapshot: 9, correction_applied: 2, check: Some(Check { seq: 41, pos_mm: [-1, 2, 3_000_000], driving: true }), first_seq: 40, inputs: vec![], ping_ms: 65530 };
        for k in 0..20 {
            let mut i = q;
            i.yaw = i.yaw.wrapping_add(k * 3);
            if k % 5 == 0 {
                i.buttons ^= button::JUMP;
            }
            p.inputs.push(i);
        }
        let mut b = vec![];
        p.write(&mut b);
        assert_eq!(InputPacket::read(&b).unwrap(), p);
        assert!(b.len() < 120, "20 inputs in {} bytes", b.len());
    }

    #[test]
    fn the_lift_tap_and_a_ride_cross_the_wire() {
        let s = StepInput { interact: true, move_y: 1.0, ..Default::default() };
        let q = NetInput::quantize(&s, 1.0);
        assert_ne!(q.buttons & button::INTERACT, 0);
        assert!(q.step_input().interact);
        let p = InputPacket { inputs: vec![q, NetInput { buttons: 0, ..q }, q], first_seq: 7, ..Default::default() };
        let mut b = vec![];
        p.write(&mut b);
        assert_eq!(InputPacket::read(&b).unwrap(), p);
        for lift in [None, Some(eo_player::lift::LiftRide { lift: 3, up: true, step: 44 }), Some(eo_player::lift::LiftRide { lift: 0, up: false, step: 89 })] {
            let st = PlayerState { lift, ticks: 12, yaw: 0.5, ..Default::default() };
            let mut b = vec![];
            write_player_state(&st, &mut b);
            assert_eq!(read_player_state(&b).unwrap(), st, "the ride {lift:?} round trips");
        }
    }
}
