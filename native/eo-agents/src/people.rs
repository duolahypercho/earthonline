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

/// (`--people-contact`) The joint that poses each of a person's contact
/// anchors, in eo-render's order (people_contact.wgsl `PC_JOINTS`): the left
/// shoe's heel and toe halves (both on the foot: the shoe is stiff), the
/// right shoe's, the shins, the thighs and the pelvis (`rocketbox::JOINTS`
/// indices: L Foot 49, R Foot 53, L Calf 48, R Calf 52, L Thigh 47, R Thigh
/// 51, Pelvis 0).
pub const CONTACT_JOINTS: [usize; 9] = [49, 49, 53, 53, 48, 52, 47, 51, 0];
/// The vertices each anchor stands for: those carried most by these joints
/// (a shoe: its foot and toe joints), and for a shoe's halves the rear or
/// the front half of them along the foot (+z, the body's forward).
const CONTACT_CARRIERS: [&[usize]; 9] = [&[49, 50], &[49, 50], &[53, 54], &[53, 54], &[48], &[52], &[47], &[51], &[0]];
const CONTACT_HALF: [i8; 9] = [-1, 1, -1, 1, 0, 0, 0, 0, 0];

/// (`--people-contact`) Each person model's contact anchors: for every
/// `CONTACT_JOINTS` entry the centroid of the vertices of its near tier
/// (LOD 1) that `CONTACT_CARRIERS` carry most (a shoe split at its median
/// along z), in the bind pose and the cooked frame, and their RMS distance
/// from it (a sphere of that radius has about their projected area),
/// appended to the bone table after its matrices (`[x, y, z, radius]` each),
/// its first index in `pad[0]` of the model and the count in `pad[1]`. Made
/// at load, never cached; without it nothing is appended and the pads stay
/// zero. An anchor nothing carries gets a zero radius: it hides nothing.
pub fn add_contact_anchors(lib: &mut crate::library::Library) {
    let n = CONTACT_JOINTS.len();
    for mi in 0..lib.models.len() {
        let m = lib.models[mi];
        if m.class != crate::CLASS_PERSON {
            continue;
        }
        let Some(mesh) = lib.meshes.get(m.meshes[1] as usize).copied() else { continue };
        let (first, count) = (mesh.first_index as usize, mesh.index_count as usize);
        let mut seen = std::collections::HashSet::new();
        // each vertex's carrying joint, once
        let mut verts: Vec<(usize, [f32; 3])> = vec![];
        for &ix in lib.indices.get(first..first + count).unwrap_or(&[]) {
            let vi = (mesh.base_vertex as i64 + ix as i64) as usize;
            if !seen.insert(vi) {
                continue;
            }
            let Some(v) = lib.vertices.get(vi) else { continue };
            let (mut best, mut bw) = (0usize, 0u8);
            for k in 0..4 {
                if v.weights[k] > bw {
                    bw = v.weights[k];
                    best = v.joints[k] as usize;
                }
            }
            verts.push((best, v.pos));
        }
        let at = lib.bones.len() as u32;
        for k in 0..n {
            let mut pts: Vec<[f32; 3]> = verts.iter().filter(|(j, _)| CONTACT_CARRIERS[k].contains(j)).map(|(_, p)| *p).collect();
            if CONTACT_HALF[k] != 0 && !pts.is_empty() {
                pts.sort_by(|a, b| a[2].total_cmp(&b[2]));
                let h = pts.len() / 2;
                pts = if CONTACT_HALF[k] < 0 { pts[..h.max(1)].to_vec() } else { pts[h..].to_vec() };
            }
            let c = if pts.is_empty() {
                [0.0f64; 3]
            } else {
                let s = pts.iter().fold([0.0f64; 3], |a, p| [a[0] + p[0] as f64, a[1] + p[1] as f64, a[2] + p[2] as f64]);
                [s[0] / pts.len() as f64, s[1] / pts.len() as f64, s[2] / pts.len() as f64]
            };
            let r = if pts.is_empty() { 0.0 } else { (pts.iter().map(|p| (0..3).map(|a| (p[a] as f64 - c[a]).powi(2)).sum::<f64>()).sum::<f64>() / pts.len() as f64).sqrt() };
            lib.bones.push([c[0] as f32, c[1] as f32, c[2] as f32, r as f32]);
        }
        lib.models[mi].pad = [at, n as u32, 0, 0];
    }
}

/// (`--people-capsules`) The joint that poses each of a person's body
/// capsules, in eo-render's order (people_capsules.wgsl `PK_JOINTS`): the
/// shoes (heel to toe, on the feet), the shins, the thighs, the hips, the
/// chest, the head and neck, the upper arms, the forearms with the hands
/// (`rocketbox::JOINTS` indices).
pub const CAPSULE_JOINTS: [usize; 13] = [49, 53, 48, 52, 47, 51, 0, 2, 5, 10, 29, 11, 30];
/// The vertices each capsule stands for: those carried most by these joints.
const CAPSULE_CARRIERS: [&[usize]; 13] = [
    &[49, 50],
    &[53, 54],
    &[48],
    &[52],
    &[47],
    &[51],
    &[0],
    &[1, 2, 3, 9, 28],
    &[4, 5, 6, 7, 8],
    &[10],
    &[29],
    &[11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27],
    &[30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46],
];
/// The share of a part's vertices left out at either end of its axis when
/// its length is measured (a stray vertex, a strap, a hem's corner).
const CAPSULE_TRIM: f64 = 0.02;

/// A capsule fitted to points: the segment's ends and the radius. The axis
/// is the points' principal axis (their covariance's largest eigenvector,
/// by power iteration), the radius the RMS distance of the points from it
/// (a limb's vertices lie on its surface, so that is the limb's radius; for
/// an oval section, the mean of its half axes' squares), and the ends the
/// points' extent along the axis (the `CAPSULE_TRIM` share cut at each end)
/// drawn in by the radius, so the capsule's round caps end where the points
/// do; a part shorter than its width is a sphere at its middle.
pub fn fit_capsule(pts: &[[f32; 3]]) -> ([f32; 3], [f32; 3], f32) {
    if pts.is_empty() {
        return ([0.0; 3], [0.0; 3], 0.0);
    }
    let n = pts.len() as f64;
    let c = pts.iter().fold([0.0f64; 3], |a, p| [a[0] + p[0] as f64 / n, a[1] + p[1] as f64 / n, a[2] + p[2] as f64 / n]);
    let mut cov = [[0.0f64; 3]; 3];
    for p in pts {
        let d = [p[0] as f64 - c[0], p[1] as f64 - c[1], p[2] as f64 - c[2]];
        for i in 0..3 {
            for j in 0..3 {
                cov[i][j] += d[i] * d[j] / n;
            }
        }
    }
    // start from the axis of largest spread, so a part spread equally along two never starts orthogonal to both
    let k0 = (0..3).max_by(|&a, &b| cov[a][a].total_cmp(&cov[b][b])).unwrap_or(1);
    let mut e = [0.0f64; 3];
    e[k0] = 1.0;
    for _ in 0..32 {
        let m = [0, 1, 2].map(|i| cov[i][0] * e[0] + cov[i][1] * e[1] + cov[i][2] * e[2]);
        let l = (m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).sqrt();
        if l < 1e-12 {
            break;
        }
        e = m.map(|x| x / l);
    }
    let mut t: Vec<f64> = pts.iter().map(|p| (p[0] as f64 - c[0]) * e[0] + (p[1] as f64 - c[1]) * e[1] + (p[2] as f64 - c[2]) * e[2]).collect();
    let r2 = pts.iter().zip(&t).map(|(p, s)| (0..3).map(|a| (p[a] as f64 - c[a] - s * e[a]).powi(2)).sum::<f64>()).sum::<f64>() / n;
    let r = r2.sqrt();
    t.sort_by(|a, b| a.total_cmp(b));
    let cut = ((t.len() as f64) * CAPSULE_TRIM).floor() as usize;
    let (lo, hi) = (t[cut.min(t.len() - 1)], t[t.len() - 1 - cut.min(t.len() - 1)]);
    let (a, b) = if hi - lo > 2.0 * r { (lo + r, hi - r) } else { let m = 0.5 * (lo + hi); (m, m) };
    let at = |s: f64| [0, 1, 2].map(|i| (c[i] + e[i] * s) as f32);
    (at(a), at(b), r as f32)
}

/// (`--people-capsules`) Each person model's body as capsules: for every
/// `CAPSULE_JOINTS` entry the capsule fitted (`fit_capsule`) to the vertices
/// of its near tier (LOD 1) that `CAPSULE_CARRIERS` carry most, in the bind
/// pose and the cooked frame, appended to the bone table after its matrices
/// as two rows (`[a, radius]`, `[b, 0]`), its first index in `pad[0]` of the
/// model and the count of capsules in `pad[1]`. Made at load, never cached;
/// in place of the contact anchors (`add_contact_anchors`), never with them.
/// A part nothing carries gets a zero radius: it hides nothing.
pub fn add_capsule_anchors(lib: &mut crate::library::Library) {
    let n = CAPSULE_JOINTS.len();
    for mi in 0..lib.models.len() {
        let m = lib.models[mi];
        if m.class != crate::CLASS_PERSON {
            continue;
        }
        let Some(mesh) = lib.meshes.get(m.meshes[1] as usize).copied() else { continue };
        let verts = carried_vertices(lib, mesh.first_index as usize, mesh.index_count as usize, mesh.base_vertex as i64);
        let at = lib.bones.len() as u32;
        for k in 0..n {
            let pts: Vec<[f32; 3]> = verts.iter().filter(|(j, _)| CAPSULE_CARRIERS[k].contains(j)).map(|(_, p)| *p).collect();
            let (a, b, r) = fit_capsule(&pts);
            lib.bones.push([a[0], a[1], a[2], r]);
            lib.bones.push([b[0], b[1], b[2], 0.0]);
        }
        lib.models[mi].pad = [at, n as u32, 0, 0];
    }
}

/// Each vertex of an index range once, with the joint that carries it most.
fn carried_vertices(lib: &crate::library::Library, first: usize, count: usize, base_vertex: i64) -> Vec<(usize, [f32; 3])> {
    let mut seen = std::collections::HashSet::new();
    let mut verts = vec![];
    for &ix in lib.indices.get(first..first + count).unwrap_or(&[]) {
        let vi = (base_vertex + ix as i64) as usize;
        if !seen.insert(vi) {
            continue;
        }
        let Some(v) = lib.vertices.get(vi) else { continue };
        let (mut best, mut bw) = (0usize, 0u8);
        for k in 0..4 {
            if v.weights[k] > bw {
                bw = v.weights[k];
                best = v.joints[k] as usize;
            }
        }
        verts.push((best, v.pos));
    }
    verts
}
