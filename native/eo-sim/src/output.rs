//! What a renderer uploads: one compact instance record per agent.
//!
//! `AgentInstance` is 32 bytes, `#[repr(C)]` and `Pod`, so a frame's buffer is
//! one `bytemuck::cast_slice` away from a GPU instance buffer. Positions are
//! f32 metres RELATIVE TO `AgentBuffer::origin` (the region's focus tile
//! corner, f64), so the instance data keeps centimetre precision anywhere on
//! Earth; the renderer adds `origin - camera` in f64 once per region.
//!
//! Layout (bytes):
//!   0  pos      [f32; 3]  x east, y up (road height with `FLAG_SURFACE_Y`), z south
//!  12  heading  f32       yaw about +y, radians, `atan2(dx, dz)` (three.js / the JS sim)
//!  16  speed    f32       ground speed, m/s
//!  20  anim     f32       vehicles: metres travelled (wheel spin = anim / wheel radius);
//!                         walkers: gait phase, radians (sin(anim) is the JS `walk.gait`)
//!  24  kind     u16       see `KIND_*`
//!  26  flags    u16       see `FLAG_*`; bits 8..11 the structure level of a
//!                         car's lane (`level_of_flags`), 0 at grade, so 0
//!                         everywhere without the grade-separated roads
//!  28  id       u32       stable id within the region (car entity id / walker key)

use bytemuck::{Pod, Zeroable};
pub use eo_data::sim_source::TRAFFIC_SURFACE_Y as FLAG_SURFACE_Y;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable, PartialEq)]
pub struct AgentInstance {
    pub pos: [f32; 3],
    pub heading: f32,
    pub speed: f32,
    pub anim: f32,
    pub kind: u16,
    pub flags: u16,
    pub id: u32,
}

impl AgentInstance {
    pub fn vehicle_surface_y(&self) -> Option<f64> {
        (self.kind < KIND_WALKER_BASE && self.flags & FLAG_SURFACE_Y != 0)
            .then_some(f64::from(self.pos[1]))
    }
}

const _: () = assert!(std::mem::size_of::<AgentInstance>() == 32);

/// Vehicle kinds 0..=3 (`TRAFFIC_KIND_MIX` order).
pub const KIND_SEDAN: u16 = 0;
pub const KIND_TAXI: u16 = 1;
pub const KIND_TRUCK: u16 = 2;
pub const KIND_BUS: u16 = 3;
/// Walkers: `KIND_WALKER_BASE + Activity as u16` (walk, wait, talk, phone, browse, stand, carry).
pub const KIND_WALKER_BASE: u16 = 16;

pub const FLAG_BRAKING: u16 = 1;
pub const FLAG_STOPPED: u16 = 2;
pub const FLAG_TURN_LEFT: u16 = 4;
pub const FLAG_TURN_RIGHT: u16 = 8;
pub const FLAG_CROSSING: u16 = 16;
pub const FLAG_KERB_WAIT: u16 = 32;
/// Stepped at the reduced far cadence this frame (level of detail).
pub const FLAG_FAR: u16 = 64;
pub const FLAG_CORNERING: u16 = 128;
/// A vehicle warning pulse issued by the simulation.
pub const FLAG_HORN: u16 = 0x8000;
/// (`--people-groups`) A walker in a social group that keeps together: bit
/// 14 set, its place in the group in bits 12..13 (0 the leader; members'
/// keys follow their leader's, so the leader's key is `id - place`).
pub const FLAG_PARTY: u16 = 0x4000;
pub const FLAG_PARTY_SLOT_SHIFT: u16 = 12;
pub const FLAG_PARTY_SLOT_MASK: u16 = 0x3000;
/// A car's structure level (`network::Lane::level`, -8..=7) in bits 8..11
/// of `flags`, two's complement: 0 at grade (and so for every car without
/// a level table), 1 the first deck level, negative in a tunnel. The feed
/// reads it to set a car on its deck; it is not a lamp or state flag.
pub const FLAG_LEVEL_SHIFT: u16 = 8;
pub const FLAG_LEVEL_MASK: u16 = 0x0F00;

/// `level`'s bits for `flags` (clamped to -8..=7).
#[inline]
pub fn level_flags(level: i8) -> u16 {
    (((level.clamp(-8, 7) as u8) & 0x0F) as u16) << FLAG_LEVEL_SHIFT
}

/// The structure level a car's `flags` carry.
#[inline]
pub fn level_of_flags(flags: u16) -> i8 {
    let v = ((flags & FLAG_LEVEL_MASK) >> FLAG_LEVEL_SHIFT) as u8;
    ((v << 4) as i8) >> 4
}

/// One corridor vehicle or walker (`corridor`): absolute world metres with
/// its height (the corridor's centreline carries it; nothing grounds it),
/// heading as `AgentInstance`, and the grade it rides, radians (nose up +).
/// 36 bytes, `Pod`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable, PartialEq)]
pub struct CorridorInstance {
    pub pos: [f32; 3],
    pub heading: f32,
    pub pitch: f32,
    pub speed: f32,
    /// vehicles: metres travelled; walkers: gait phase, radians.
    pub anim: f32,
    pub kind: u16,
    pub flags: u16,
    pub id: u32,
}

const _: () = assert!(std::mem::size_of::<CorridorInstance>() == 36);

/// One region's agents after a step.
#[derive(Clone, Debug, Default)]
pub struct AgentBuffer {
    pub origin: [f64; 2],
    pub time: f64,
    pub step: u64,
    pub vehicles: Vec<AgentInstance>,
    pub walkers: Vec<AgentInstance>,
}

impl AgentBuffer {
    pub fn bytes(&self) -> (&[u8], &[u8]) {
        (bytemuck::cast_slice(&self.vehicles), bytemuck::cast_slice(&self.walkers))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_ride_in_the_flags_and_are_zero_at_grade() {
        assert_eq!(level_flags(0), 0);
        for l in -8..=7i8 {
            let f = level_flags(l) | FLAG_BRAKING | FLAG_CORNERING;
            assert_eq!(level_of_flags(f), l);
            assert_eq!(f & !FLAG_LEVEL_MASK, FLAG_BRAKING | FLAG_CORNERING);
        }
        assert_eq!(level_of_flags(level_flags(9)), 7);
        assert_eq!(level_of_flags(level_flags(-20)), -8);
    }
}
