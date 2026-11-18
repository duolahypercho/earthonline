//! What a renderer uploads: one compact instance record per agent.
//!
//! `AgentInstance` is 32 bytes, `#[repr(C)]` and `Pod`, so a frame's buffer is
//! one `bytemuck::cast_slice` away from a GPU instance buffer. Positions are
//! f32 metres RELATIVE TO `AgentBuffer::origin` (the region's focus tile
//! corner, f64), so the instance data keeps centimetre precision anywhere on
//! Earth; the renderer adds `origin - camera` in f64 once per region.
//!
//! Layout (bytes):
//!   0  pos      [f32; 3]  x east, y up (0: the renderer grounds it), z south
//!  12  heading  f32       yaw about +y, radians, `atan2(dx, dz)` (three.js / the JS sim)
//!  16  speed    f32       ground speed, m/s
//!  20  anim     f32       vehicles: metres travelled (wheel spin = anim / wheel radius);
//!                         walkers: gait phase, radians (sin(anim) is the JS `walk.gait`)
//!  24  kind     u16       see `KIND_*`
//!  26  flags    u16       see `FLAG_*`
//!  28  id       u32       stable id within the region (car entity id / walker key)

use bytemuck::{Pod, Zeroable};

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
