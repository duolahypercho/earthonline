//! The people's shared types: a cooked tier, a node at rest, an atlas, and
//! an animation clip (its channels by node name, sampled linearly).
//!
//! WHY A BAKED JOINT-MATRIX TABLE: every clip is sampled once at load, per
//! skeleton, into skinning matrices (joint world x inverse bind) at a fixed
//! rate; a person on screen is then just (model, clip, phase) in its 64-byte
//! instance, and the vertex shader fetches and blends the two frames around
//! the phase. Nothing is posed per person on the CPU and nothing per person
//! is uploaded but the instance, so the cost grows with vertices drawn, not
//! with people simulated. A per-instance palette would need a CPU or compute
//! pose pass and an upload per person per frame; a vertex animation texture
//! would need a bake per tier and per clip (vertices x frames) and could not
//! blend clips. What the table cannot do: per-person procedural layers (foot
//! IK, activity arm overlays, look-at).

use crate::AgentVertex;
use glam::{Quat, Vec3};

/// One tier of a person, in mesh space (skinned by the bake).
#[derive(Clone, Debug, Default)]
pub struct CookedTier {
    pub vertices: Vec<AgentVertex>,
    pub indices: Vec<u32>,
    pub min: [f32; 3],
    pub max: [f32; 3],
}

#[derive(Clone, Debug)]
pub struct RestNode {
    pub name: String,
    pub parent: Option<usize>,
    pub t: Vec3,
    pub r: Quat,
    pub s: Vec3,
}

/// The atlas of a person, RGBA8: basecolor (sRGB), normal, ORM.
#[derive(Clone, Debug, Default)]
pub struct Atlas {
    pub res: u32,
    pub maps: [Vec<u8>; 3],
    /// block-compressed mip chains instead (BC3 sRGB basecolor + cutout,
    /// BC5 normal xy, BC1 ORM; levels `res` down to 4 px, concatenated); empty
    /// for the RGBA8 path above, whose mips the GPU side builds.
    pub bc: [Vec<u8>; 3],
    pub bc_levels: u32,
}

/// One animation channel of a clip.
#[derive(Clone, Debug)]
pub struct Channel {
    pub node: String,
    /// 0 translation, 1 rotation, 2 scale.
    pub path: u8,
    pub times: Vec<f32>,
    pub values: Vec<[f32; 4]>,
}

#[derive(Clone, Debug, Default)]
pub struct Clip {
    pub name: String,
    pub duration: f32,
    pub channels: Vec<Channel>,
}

impl Clip {
    pub(crate) fn sample(&self, c: &Channel, t: f32) -> [f32; 4] {
        let n = c.times.len();
        if n == 1 || t <= c.times[0] {
            return c.values[0];
        }
        if t >= c.times[n - 1] {
            return c.values[n - 1];
        }
        let k = c.times.partition_point(|&x| x <= t).max(1) - 1;
        let (t0, t1) = (c.times[k], c.times[k + 1]);
        let f = ((t - t0) / (t1 - t0).max(1e-9)).clamp(0.0, 1.0);
        let (a, b) = (c.values[k], c.values[k + 1]);
        if c.path == 1 {
            Quat::from_array(a).slerp(Quat::from_array(b), f).to_array()
        } else {
            [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f, 0.0]
        }
    }
}
