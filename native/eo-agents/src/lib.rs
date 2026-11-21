//! Moving things on screen, without the GPU: the JS game's traffic bodies and
//! the Rocketbox people cooked into one mesh library, the people's motion
//! baked into joint matrices, and the per-frame instance records the renderer
//! (`eo-render/src/agents.rs`) culls, LOD-selects and draws on the GPU.
//!
//! Sources:
//!   * vehicles: `src/vehicles/hero-vehicle-manifest.js` (the hash-locked LOD
//!     GLBs), `src/vehicles/vehicle-catalogue.js` (`TRAFFIC_KIND_MAP`,
//!     `PAINT_SETS`, `RIM_FINISHES`), `src/render/passes/vehicle-hero-overlay.js`
//!     (wheel rig `Wheel_XX` / `Wheel_XX_Spin`, lamp channels by material name,
//!     `HERO_LAMP`, the hero tiers' radii and caps), `src/citygen/traffic.js`
//!     (`spawnCar`'s keyed catalogue pick);
//!   * people: the Microsoft Rocketbox avatars (`rocketbox.rs`) moved by their
//!     own clips and the 100STYLE walks (`motion.rs`), all read from outside
//!     the repository and checked against the manifests in `native/assets/`;
//!     the crowd bands, caps and gait come from the earlier browser version
//!     (`pedestrian-presentation.js`).
//!
//! The numbers the JS computes are exported by the earlier browser version's exporter
//! into `data/agents-js.json`.

pub mod catalog;
pub mod feed;
pub mod gait;
pub mod library;
pub mod motion;
pub mod packed_glb; // people
pub mod people;
pub mod rocketbox; // people
pub mod vehicles;

use bytemuck::{Pod, Zeroable};

/// One vertex of the agent library (32 bytes).
///
/// `tag`: bits 0..16 material index (into `Library::materials`), bits 16..24
/// the wheel part (0 none; 1..=4 a spinning part of wheel FL, FR, RL, RR,
/// which also steers on the front axle; 5..=8 a steer-only part of the same
/// wheel, the caliper), bits 24..32 reserved.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable, PartialEq)]
pub struct AgentVertex {
    pub pos: [f32; 3],
    pub normal: [i8; 4],
    /// Texture coordinate, unorm16 (atlas space 0..1).
    pub uv: [u16; 2],
    pub joints: [u8; 4],
    pub weights: [u8; 4],
    pub tag: u32,
}
const _: () = assert!(std::mem::size_of::<AgentVertex>() == 32);

pub const WHEEL_NONE: u32 = 0;

pub fn tag(material: u32, wheel: u32) -> u32 {
    (material & 0xffff) | ((wheel & 0xff) << 16)
}

/// One drawable mesh (an LOD of a body or a character tier): an opaque index
/// range and a transparent one, both relative to `base_vertex`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable, PartialEq)]
pub struct GpuMesh {
    pub first_index: u32,
    pub index_count: u32,
    pub glass_first: u32,
    pub glass_count: u32,
    pub base_vertex: i32,
    pub pad: [u32; 3],
}

pub const NO_MESH: u32 = u32::MAX;
pub const CLASS_VEHICLE: u32 = 0;
pub const CLASS_PERSON: u32 = 1;
/// LOD slots per model. Vehicles: GLB LOD0, LOD1, LOD2, box. People: hero,
/// near, mid, far tier.
pub const LODS: usize = 4;

/// One model: a vehicle body or a person (128 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable, PartialEq)]
pub struct GpuModel {
    pub meshes: [u32; LODS],
    /// Vehicles: wheel pivots FL, FR, RL, RR in the body frame (xyz).
    pub wheel: [[f32; 4]; 4],
    /// Bounding sphere in the body frame.
    pub center: [f32; 3],
    pub radius: f32,
    pub class: u32,
    /// People: first joint matrix of this skeleton's bake (in matrices).
    pub skeleton: u32,
    pub joints: u32,
    pub wheel_radius: f32,
    pub pad: [u32; 4],
}
const _: () = assert!(std::mem::size_of::<GpuModel>() == 128);

/// One agent instance as the GPU reads it (64 bytes), world space.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable, PartialEq)]
pub struct GpuAgent {
    pub pos: [f32; 3],
    pub model: u32,
    /// Orientation quaternion (x, y, z, w).
    pub rot: [f32; 4],
    /// Paint (vehicles) as linear-encoded sRGB bytes; unused.
    pub color: u32,
    /// `eo_sim::output::FLAG_*` in the low 16 bits; `AGENT_*` above.
    pub flags: u32,
    /// Vehicles: wheel spin angle, radians. People: walk cycle phase 0..1
    /// (0 = left heel strike); with walk styles, the place in
    /// the walk slot's frames 0..1 (`motion::MotionMeta::walk_anim`).
    pub anim: f32,
    /// Vehicles: front wheel steer angle, radians.
    pub steer: f32,
    pub speed: f32,
    pub scale: f32,
    /// People: phase 0..1 of the standing clip; with
    /// standing loops, the place in the slot's frames (`MotionMeta::clip_phase`).
    pub clip_phase: f32,
    /// People: the standing clip (`CLIP_*`).
    pub clip: u32,
}
const _: () = assert!(std::mem::size_of::<GpuAgent>() == 64);

/// The player (or the player's car): drawn whatever its distance (no cap demotion).
pub const AGENT_PLAYER: u32 = 1 << 16;
/// Turn signal lamps lit this frame (the blink is decided on the CPU clock).
pub const AGENT_BLINK_ON: u32 = 1 << 17;
/// In a tunnel's bore (`--structures --tunnels`; the feed's `bore_at` for a
/// car on a lane under grade): the renderer lights it as the bore's surfaces
/// are lit (eo-render agents_draw.wgsl, structures.wgsl `sx_bore_light_at`),
/// not by the sun and sky. Never set without bores.
pub const AGENT_IN_BORE: u32 = 1 << 18;
/// With `AGENT_IN_BORE`, the flags' bits from here up carry how far in the
/// agent stands from the bore's nearer portal, in `1 / BORE_DEPTH_QUANT` m
/// steps (saturating at the field's top: deep inside, or no portal).
pub const BORE_DEPTH_SHIFT: u32 = 19;
pub const BORE_DEPTH_QUANT: f64 = 8.0;
pub const BORE_DEPTH_MAX_Q: u32 = (1 << (32 - BORE_DEPTH_SHIFT)) - 1;

/// The flag bits of an agent in a bore `depth` m in from its nearer portal
/// (`f64::INFINITY` with no portal).
pub fn bore_flags(depth: f64) -> u32 {
    let q = if depth.is_finite() { (depth.max(0.0) * BORE_DEPTH_QUANT).round().min(BORE_DEPTH_MAX_Q as f64) as u32 } else { BORE_DEPTH_MAX_Q };
    AGENT_IN_BORE | (q << BORE_DEPTH_SHIFT)
}

/// The depth in from the portal that `flags` carry (None: not in a bore).
pub fn bore_depth_of_flags(flags: u32) -> Option<f64> {
    (flags & AGENT_IN_BORE != 0).then(|| (flags >> BORE_DEPTH_SHIFT) as f64 / BORE_DEPTH_QUANT)
}

/// Clip slots of the bake (the walk is slot 0; the others are standing clips).
pub const CLIP_WALK: u32 = 0;
pub const CLIP_IDLE: u32 = 1;
pub const CLIP_TALK: u32 = 2;
pub const CLIP_PHONE: u32 = 3;
pub const CLIPS: usize = 4;

/// Material, the same 64-byte layout as `Material` in eo-render's draw.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable, PartialEq)]
pub struct AgentMaterial {
    pub base: [f32; 4],
    pub emissive: [f32; 4],
    /// roughness, metalness, normal strength, unused
    pub params: [f32; 4],
    pub layer: i32,
    pub flags: u32,
    pub rough_flatten: f32,
    pub thin_absorption: f32,
}

/// draw.wgsl's flags this library uses.
pub const F_BLEND: u32 = 4;
pub const F_GLASS: u32 = 8;
/// Agent flags (above draw.wgsl's own bits).
pub const M_PAINT: u32 = 1 << 8;
pub const M_TAIL: u32 = 1 << 9;
pub const M_BRAKE: u32 = 1 << 10;
pub const M_INDICATOR: u32 = 1 << 11;
pub const M_HEAD: u32 = 1 << 12;
pub const M_DRL: u32 = 1 << 13;
pub const M_REVERSE: u32 = 1 << 14;
/// Textured from the person's atlas (layer = person), alpha-tested at 0.5.
pub const M_ATLAS: u32 = 1 << 15;
/// The rim finish multiplies this material (VehicleAlloy).
pub const M_RIM: u32 = 1 << 16;

/// sRGB byte to linear.
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// `#rrggbb` to linear RGB.
pub fn hex_linear(hex: &str) -> [f32; 3] {
    let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0xffffff);
    [((v >> 16) & 255) as f32 / 255.0, ((v >> 8) & 255) as f32 / 255.0, (v & 255) as f32 / 255.0].map(srgb_to_linear)
}

/// `#rrggbb` packed as the instance colour (r, g, b, 255 in bytes, sRGB).
pub fn hex_packed(hex: &str) -> u32 {
    let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0xffffff);
    ((v >> 16) & 255) | (((v >> 8) & 255) << 8) | ((v & 255) << 16) | (255 << 24)
}
