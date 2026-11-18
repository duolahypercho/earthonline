//! Where a drawn person's head and hands are, on the CPU: the blend of baked
//! joint matrices the agent vertex shader skins with (eo-render
//! `agents_common.wgsl` `ag_pose` / `ag_joint_rows`, mirrored here term for
//! term), carried through one joint at a time. For what hangs on another
//! player's body: the name tag over the head (pose-aware: a crouch lowers
//! it), the head the tag's occlusion test looks for, and the phone in the
//! selfie's hand.

use crate::library::Library;
use crate::pose_edit::{Side, joint};
use crate::vehicles::{CookedBody, pack_normal};
use crate::{AgentMaterial, AgentVertex, CLASS_PERSON, GpuAgent, tag};
use glam::{Mat3, Quat, Vec3, Vec4};

fn fract(x: f32) -> f32 {
    x - x.floor()
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Joint `j`'s skinning rows for instance `a`, blended over two frames of
/// the walk and two of the standing clip by the ground speed, as the shader does.
pub fn joint_rows(lib: &Library, a: &GpuAgent, j: usize) -> Option<[Vec4; 3]> {
    let m = lib.models.get(a.model as usize)?;
    if m.class != CLASS_PERSON {
        return None;
    }
    let mesh = lib.meshes.get(m.meshes[0] as usize)?;
    let (base, joints) = (mesh.pad[0] as usize, mesh.pad[1] as usize);
    if j >= joints {
        return None;
    }
    let meta = &lib.meta;
    let wn = meta.clip_frames[0].max(1) as usize;
    let wf = fract(a.anim) * wn as f32;
    let wf0 = (wf.floor() as usize) % wn;
    let wf1 = (wf0 + 1) % wn;
    let wt = fract(wf);
    let c = (a.clip as usize).min(3);
    let cn = meta.clip_frames[c].max(1) as usize;
    let cb = meta.clip_base[c] as usize;
    let cf = fract(a.clip_phase) * cn as f32;
    let cf0 = cb + (cf.floor() as usize) % cn;
    let cf1 = cb + (cf.floor() as usize + 1) % cn;
    let ct = fract(cf);
    let ww = smoothstep(0.15, 0.60, a.speed);
    let wb = meta.clip_base[0] as usize;
    let row = |f: usize, k: usize| lib.bones.get((base + f * joints + j) * 3 + k).map(|r| Vec4::from(*r));
    let mut out = [Vec4::ZERO; 3];
    for (k, o) in out.iter_mut().enumerate() {
        let (w0, w1, c0, c1) = (row(wb + wf0, k)?, row(wb + wf1, k)?, row(cf0, k)?, row(cf1, k)?);
        *o = c0.lerp(c1, ct).lerp(w0.lerp(w1, wt), ww);
    }
    Some(out)
}

/// The bind positions of `a`'s skeleton (`MotionMeta::group_bind`).
fn bind_of<'a>(lib: &'a Library, a: &GpuAgent) -> Option<&'a [[f32; 3]]> {
    let person = lib.meta.people_models.iter().position(|&m| m == a.model)?;
    let g = *lib.meta.motion.person_group.get(person)? as usize;
    lib.meta.motion.group_bind.get(g).map(|v| v.as_slice())
}

/// A point fixed to joint `j` (at `bind`, in the bind's cooked frame), in the world.
pub fn point(lib: &Library, a: &GpuAgent, j: usize, bind: Vec3) -> Option<Vec3> {
    let r = joint_rows(lib, a, j)?;
    let p = bind.extend(1.0);
    let local = Vec3::new(r[0].dot(p), r[1].dot(p), r[2].dot(p)) * a.scale;
    Some(Vec3::from(a.pos) + Quat::from_array(a.rot) * local)
}

/// Joint `j` itself, in the world.
pub fn joint_at(lib: &Library, a: &GpuAgent, j: usize) -> Option<Vec3> {
    let b = *bind_of(lib, a)?.get(j)?;
    point(lib, a, j, Vec3::from(b))
}

/// A head as a name tag needs it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Head {
    /// The cranium's centre (about 7.5 cm behind the eyes, at their height).
    pub centre: Vec3,
    /// The crown (the top of the head as the body stands at bind).
    pub crown: Vec3,
    /// The head joint (the critics' "head bone").
    pub joint: Vec3,
}

/// Where `a`'s head is this frame (None: not a person, or no bind table).
pub fn head(lib: &Library, a: &GpuAgent) -> Option<Head> {
    let bind = bind_of(lib, a)?;
    let hj = joint("Bip01 Head");
    let h = Vec3::from(*bind.get(hj)?);
    let eyes = (Vec3::from(*bind.get(joint("Bip01 LEye"))?) + Vec3::from(*bind.get(joint("Bip01 REye"))?)) * 0.5;
    let person = lib.meta.people_models.iter().position(|&m| m == a.model)?;
    let height = lib.meta.family_heights.get(person).copied().unwrap_or(h.y + 0.2).max(h.y + 0.12);
    Some(Head {
        centre: point(lib, a, hj, Vec3::new(eyes.x, eyes.y + 0.015, eyes.z - 0.075))?,
        crown: point(lib, a, hj, Vec3::new(h.x, height - 0.005, h.z - 0.01))?,
        joint: point(lib, a, hj, h)?,
    })
}

/// The right hand's frame this frame: the wrist, the fingers' direction,
/// the palm normal and the lateral axis (pinky to index), as `pose_edit::hand_frame`.
pub fn right_hand(lib: &Library, a: &GpuAgent) -> Option<(Vec3, Vec3, Vec3, Vec3)> {
    let r = Side::Right;
    let w = joint_at(lib, a, r.j("Hand"))?;
    let f = (joint_at(lib, a, r.j("Finger2"))? - w).normalize_or_zero();
    let l = (joint_at(lib, a, r.j("Finger1"))? - joint_at(lib, a, r.j("Finger4"))?).normalize_or_zero();
    let n = l.cross(f).normalize_or_zero();
    if f == Vec3::ZERO || n == Vec3::ZERO {
        return None;
    }
    Some((w, f, n, f.cross(n)))
}

/// A 6.1-inch handset: 146.7 x 71.5 x 7.8 mm (a common published handset
/// size), in its own frame (x across, y along, z out of the screen).
pub const PHONE_M: [f32; 3] = [0.0715, 0.1467, 0.0078];

/// The phone in `a`'s right hand, held upright with its screen toward the
/// palm side (the selfie's face): its back against the palm a hand's
/// thickness off the bones (about 2 cm), gripped below its middle (its top
/// stands about 6 cm past the index side of the hand, where the camera is).
pub fn phone(lib: &Library, a: &GpuAgent) -> Option<GpuAgent> {
    let model = lib.meta.phone_model?;
    let (w, f, n, l) = right_hand(lib, a)?;
    let s = a.scale;
    let centre = w + f * (0.062 * s) + n * (0.021 * s + PHONE_M[2] * 0.5) + l * (0.03 * s);
    let (y, z) = (l, n);
    let x = y.cross(z);
    let rot = Quat::from_mat3(&Mat3::from_cols(x, y, z)).normalize();
    Some(GpuAgent { pos: centre.to_array(), model, rot: rot.to_array(), color: 0xffffffff, flags: 0, anim: 0.0, steer: 0.0, speed: 0.0, scale: 1.0, clip_phase: 0.0, clip: 0 })
}

/// The phone as a body (`library` appends it as a model): a slab with a
/// black glass screen on +z and a camera island on the back. Linear base
/// colours: the screen's dark panel about 0.01 (an off display behind glass
/// reflects about 4 % at normal incidence, which the shading's dielectric F0
/// carries), the anodised graphite back about 0.04.
pub fn phone_body() -> CookedBody {
    let back = AgentMaterial { base: [0.04, 0.042, 0.045, 1.0], params: [0.35, 0.0, 0.0, 0.0], layer: -1, flags: 0, ..Default::default() };
    let screen = AgentMaterial { base: [0.010, 0.011, 0.013, 1.0], params: [0.06, 0.0, 0.0, 0.0], layer: -1, flags: 0, ..Default::default() };
    let lens = AgentMaterial { base: [0.015, 0.015, 0.017, 1.0], params: [0.12, 0.0, 0.0, 0.0], layer: -1, flags: 0, ..Default::default() };
    let mut b = CookedBody { materials: vec![back, screen, lens], material_names: vec!["phone-back".into(), "phone-screen".into(), "phone-lens".into()], ..Default::default() };
    let [w, h, t] = PHONE_M;
    let mut cuboid = |lo: [f32; 3], hi: [f32; 3], front: u32, rest: u32| {
        let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
            ([1.0, 0.0, 0.0], [[hi[0], lo[1], lo[2]], [hi[0], hi[1], lo[2]], [hi[0], hi[1], hi[2]], [hi[0], lo[1], hi[2]]]),
            ([-1.0, 0.0, 0.0], [[lo[0], lo[1], hi[2]], [lo[0], hi[1], hi[2]], [lo[0], hi[1], lo[2]], [lo[0], lo[1], lo[2]]]),
            ([0.0, 1.0, 0.0], [[lo[0], hi[1], lo[2]], [lo[0], hi[1], hi[2]], [hi[0], hi[1], hi[2]], [hi[0], hi[1], lo[2]]]),
            ([0.0, -1.0, 0.0], [[lo[0], lo[1], hi[2]], [lo[0], lo[1], lo[2]], [hi[0], lo[1], lo[2]], [hi[0], lo[1], hi[2]]]),
            ([0.0, 0.0, 1.0], [[hi[0], lo[1], hi[2]], [hi[0], hi[1], hi[2]], [lo[0], hi[1], hi[2]], [lo[0], lo[1], hi[2]]]),
            ([0.0, 0.0, -1.0], [[lo[0], lo[1], lo[2]], [lo[0], hi[1], lo[2]], [hi[0], hi[1], lo[2]], [hi[0], lo[1], lo[2]]]),
        ];
        for (k, (nrm, quad)) in faces.into_iter().enumerate() {
            let m = if k == 4 { front } else { rest };
            let base = b.vertices.len() as u32;
            for p in quad {
                b.vertices.push(AgentVertex { pos: p, normal: pack_normal(Vec3::from(nrm)), tag: tag(m, 0), ..Default::default() });
            }
            b.opaque.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            b.triangles += 2;
        }
    };
    cuboid([-w / 2.0, -h / 2.0, -t / 2.0], [w / 2.0, h / 2.0, t / 2.0], 1, 0);
    // The camera island: 28 mm square, 1.5 mm proud, in the back's upper corner.
    let (cx, cy) = (-w / 2.0 + 0.006, h / 2.0 - 0.006);
    cuboid([cx, cy - 0.028, -t / 2.0 - 0.0015], [cx + 0.028, cy, -t / 2.0], 0, 2);
    b.min = [-w / 2.0, -h / 2.0, -t / 2.0 - 0.0015];
    b.max = [w / 2.0, h / 2.0, t / 2.0];
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The CPU blend is the shader's: at a pure standing frame it is that
    /// frame's rows; past the walk ramp it is the walk's.
    #[test]
    fn rows_blend_as_the_shader() {
        let mut lib = Library::default();
        let j = crate::rocketbox::JOINTS.len();
        lib.meta.clip_frames = [2, 2, 0, 0];
        lib.meta.clip_base = [0, 2, 4, 4];
        // Four frames of one joint table (2 walk, 2 standing), each joint's rows a translation by the frame number.
        for f in 0..4 {
            for _ in 0..j {
                lib.bones.push([1.0, 0.0, 0.0, f as f32]);
                lib.bones.push([0.0, 1.0, 0.0, 0.0]);
                lib.bones.push([0.0, 0.0, 1.0, 0.0]);
            }
        }
        lib.meshes.push(crate::GpuMesh { pad: [0, j as u32, 0], ..Default::default() });
        lib.models.push(crate::GpuModel { class: CLASS_PERSON, ..Default::default() });
        let mut a = GpuAgent { clip: 1, clip_phase: 0.0, speed: 0.0, scale: 1.0, rot: Quat::IDENTITY.to_array(), ..Default::default() };
        let r = joint_rows(&lib, &a, 5).unwrap();
        assert_eq!(r[0].w, 2.0, "standing frame 0 of slot 1 is table frame 2");
        a.clip_phase = 0.25;
        assert!((joint_rows(&lib, &a, 5).unwrap()[0].w - 2.5).abs() < 1e-6, "half way to frame 3");
        a.speed = 1.0;
        a.anim = 0.5;
        assert!((joint_rows(&lib, &a, 5).unwrap()[0].w - 1.0).abs() < 1e-6, "walking: walk frame 1");
        let p = point(&lib, &GpuAgent { pos: [10.0, 0.0, 0.0], ..a }, 5, Vec3::new(0.0, 1.7, 0.0)).unwrap();
        assert!((p - Vec3::new(11.0, 1.7, 0.0)).length() < 1e-5, "{p}");
    }

    #[test]
    fn the_phone_is_a_handset() {
        let b = phone_body();
        let size: Vec<f32> = (0..3).map(|k| b.max[k] - b.min[k]).collect();
        assert!((size[1] - 0.1467).abs() < 1e-4 && (size[0] - 0.0715).abs() < 1e-4, "{size:?}");
        assert_eq!(b.triangles, 24);
    }
}
