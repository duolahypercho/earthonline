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
    // A player's (no distance cap demotes it, and it takes no car's place in the cull's tiers).
    Some(GpuAgent { pos: centre.to_array(), model, rot: rot.to_array(), color: 0xffffffff, flags: crate::AGENT_PLAYER, anim: 0.0, steer: 0.0, speed: 0.0, scale: 1.0, clip_phase: 0.0, clip: 0 })
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

/// The ground-contact patch as a body (`library` appends it as a model,
/// `Library::add_contact_model`): a quad in the ground plane, x and z in
/// [-1, 1] (the instance scale is its half size), facing up, drawn only in
/// the glass pass: no opaque part, so it casts no shadow and writes no depth
/// or motion. One black material flagged `M_CONTACT`: the shader works out
/// how much of the ground's light the instance's box takes away.
pub fn contact_body() -> CookedBody {
    let m = AgentMaterial { base: [0.0, 0.0, 0.0, 1.0], params: [1.0, 0.0, 0.0, 0.0], layer: -1, flags: crate::M_CONTACT | crate::F_BLEND, ..Default::default() };
    let mut b = CookedBody { materials: vec![m], material_names: vec!["contact".into()], ..Default::default() };
    for (x, z) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        b.vertices.push(AgentVertex { pos: [x, 0.0, z], normal: pack_normal(Vec3::Y), tag: tag(0, 0), ..Default::default() });
    }
    b.glass = vec![0, 1, 2, 0, 2, 3];
    b.triangles = 2;
    b.min = [-1.0, 0.0, -1.0];
    b.max = [1.0, 0.0, 1.0];
    b
}

/// One box of a drawn body over the ground, as the contact patch's shader
/// reads it (eo-render `agents_draw.wgsl` `ag_contact`): the share of the
/// sky a ground point sees that the box hides (the exact view factor of a
/// box standing on or floating over the plane, Howell's catalogue of
/// radiation configuration factors: a plane element to a perpendicular
/// rectangle, and to a parallel one, summed over the faces the point
/// sees), and, for a low box, whether it stands between the point and the
/// sun (a few centimetres from a sole, which the shadow cascades and the
/// screen-space contact ray cannot resolve).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContactBox {
    /// Centre of the footprint (world x, z).
    pub centre: [f32; 2],
    /// The box's z axis is world +z turned `yaw` about +y.
    pub yaw: f32,
    /// Half width (box x) and half length (box z), metres.
    pub half: [f32; 2],
    /// Bottom and top above the ground, metres.
    pub h0: f32,
    pub h1: f32,
    /// The patch also takes the sun the box blocks.
    pub sun: bool,
    /// 0..1: a foot lifting off the ground fades its patch out before it
    /// swings (a moving patch writes no motion for the temporal resolve).
    pub weight: f32,
}

/// The patch quad's half size: the footprint and as far out as the box
/// still hides 1-3 % of the sky (a wall of height h hides
/// (1 - c / sqrt(c^2 + h^2)) / 2 of it c away: 2.6 % at 3 h).
pub fn contact_reach(b: &ContactBox) -> f32 {
    let m = if b.h0 < 0.02 { (2.5 * b.h1).clamp(0.08, 0.9) } else { (2.0 * b.h1).clamp(0.2, 0.9) };
    b.half[0].max(b.half[1]) + m
}

// Proportions of the shod foot (ANSUR 1988 summary statistics, Gordon et
// al. 1989, means of men and women, about): the ball of the foot (first
// metatarsophalangeal joint, the Bip01 Toe0 joint) 0.73 of the foot's
// length from the heel; the ankle (lateral malleolus, the Bip01 Foot
// joint) about 0.22 of it; foot breadth 0.37 of it (100.9 / 269.7 mm men,
// 91.4 / 244.5 mm women). A shoe adds about 1 cm behind the heel, 1.5 cm
// past the toes and 1 cm across; its upper averages 0.8 of the ankle's
// height (high at the collar, low over the toes).
const BALL_OF_FOOT: f32 = 0.73;
const ANKLE_OF_FOOT: f32 = 0.22;
const BREADTH_OF_FOOT: f32 = 0.37;
// Limb radii, metres (about: calf 0.37 m and thigh 0.58 m round, adult means).
const SHIN_R: f32 = 0.055;
const THIGH_R: f32 = 0.085;

/// A limb from joint `p` to joint `q` (world), `r` round, as `n` boxes along it.
fn limb(out: &mut Vec<ContactBox>, p: Vec3, q: Vec3, r: f32, ground: f32, n: usize) {
    let d = glam::Vec2::new(q.x - p.x, q.z - p.z);
    let len = d.length();
    let yaw = if len > 1e-4 { d.x.atan2(d.y) } else { 0.0 };
    for k in 0..n {
        let a = p.lerp(q, k as f32 / n as f32);
        let b = p.lerp(q, (k + 1) as f32 / n as f32);
        let c = (a + b) * 0.5;
        let h0 = (a.y.min(b.y) - r - ground).max(0.0);
        let h1 = a.y.max(b.y) + r - ground;
        if h1 <= h0 + 1e-3 {
            continue;
        }
        out.push(ContactBox { centre: [c.x, c.z], yaw, half: [r, (len / n as f32 * 0.5).max(r)], h0, h1, sun: h1 < 0.25, weight: 1.0 });
    }
}

/// The boxes of `a`'s body that darken the ground under it this frame
/// (`ground`: the height its feet stand on): each shoe, each shin and thigh
/// as two boxes along it, and, crouched (hips under 0.6 m), the hips.
pub fn contacts(lib: &Library, a: &GpuAgent, ground: f32) -> Option<Vec<ContactBox>> {
    let bind = bind_of(lib, a)?;
    let s = a.scale;
    let forward = Quat::from_array(a.rot) * Vec3::Z;
    let mut out = Vec::with_capacity(12);
    let mut hips = Vec3::ZERO;
    for side in [Side::Left, Side::Right] {
        let (fj, tj) = (side.j("Foot"), side.j("Toe0"));
        let ankle = joint_at(lib, a, fj)?;
        let ball = joint_at(lib, a, tj)?;
        let knee = joint_at(lib, a, side.j("Calf"))?;
        let hip = joint_at(lib, a, side.j("Thigh"))?;
        hips += hip * 0.5;
        let (a0, b0) = (Vec3::from(*bind.get(fj)?) * s, Vec3::from(*bind.get(tj)?) * s);
        let ab = glam::Vec2::new(b0.x - a0.x, b0.z - a0.z).length();
        if ab > 0.02 {
            let foot = ab / (BALL_OF_FOOT - ANKLE_OF_FOOT);
            let mut d = glam::Vec2::new(ball.x - ankle.x, ball.z - ankle.z);
            if d.length() < 1e-3 {
                d = glam::Vec2::new(forward.x, forward.z);
            }
            let d = d.normalize_or_zero();
            let heel = glam::Vec2::new(ankle.x, ankle.z) - d * (ANKLE_OF_FOOT * foot + 0.01);
            let long = foot + 0.025;
            let c = heel + d * (long * 0.5);
            let lift = (((ankle.y - ground - a0.y) + (ball.y - ground - b0.y)) * 0.5).max(0.0);
            let weight = 1.0 - smoothstep(0.04, 0.12, lift);
            if weight > 0.0 {
                // The box a little inside the shoe's outline (0.85 of its
                // breadth, 0.9 of its length), so its corners stay under the
                // rounded toe and heel.
                out.push(ContactBox { centre: [c.x, c.y], yaw: d.x.atan2(d.y), half: [0.85 * (BREADTH_OF_FOOT * foot + 0.01) * 0.5, 0.9 * long * 0.5], h0: lift, h1: lift + 0.8 * a0.y, sun: true, weight });
            }
        }
        limb(&mut out, knee, ankle, SHIN_R * s, ground, 2);
        limb(&mut out, hip, knee, THIGH_R * s, ground, 2);
    }
    // The hips and seat of a crouch (about 0.36 m across and 0.25 m deep,
    // from 8 cm under the hip joints).
    let low = hips.y - ground - 0.08;
    if low < 0.6 {
        out.push(ContactBox { centre: [hips.x, hips.z], yaw: forward.x.atan2(forward.z), half: [0.18 * s, 0.125 * s], h0: low.max(0.0), h1: low.max(0.0) + 0.3, sun: false, weight: 1.0 });
    }
    Some(out)
}

/// `contacts` as instances of the contact model (`Meta::contact_model`),
/// each 1 cm over the ground (no fight with it; the patch's own depth test
/// keeps it under the feet that stand on it). The box rides in the fields a
/// vehicle without wheels leaves unused: `anim` half width, `steer` half
/// length, `speed` top, `clip_phase` bottom; `color` the weight (low byte)
/// and the sun bit (bit 8). Flagged as a player's, so no distance cap
/// demotes it and it takes no car's place in the cull's tiers.
pub fn contact_instances(lib: &Library, a: &GpuAgent, ground: f32, out: &mut Vec<GpuAgent>) -> usize {
    let (Some(model), Some(boxes)) = (lib.meta.contact_model, contacts(lib, a, ground)) else { return 0 };
    let n0 = out.len();
    for b in boxes {
        let w = (b.weight.clamp(0.0, 1.0) * 255.0).round() as u32;
        if w == 0 {
            continue;
        }
        out.push(GpuAgent {
            pos: [b.centre[0], ground + CONTACT_LIFT_M, b.centre[1]],
            model,
            rot: Quat::from_rotation_y(b.yaw).to_array(),
            color: w | (u32::from(b.sun) << 8),
            flags: crate::AGENT_PLAYER,
            anim: b.half[0],
            steer: b.half[1],
            speed: b.h1,
            scale: contact_reach(&b),
            clip_phase: b.h0,
            clip: 0,
        });
    }
    out.len() - n0
}

/// How far over the ground a contact patch is drawn.
pub const CONTACT_LIFT_M: f32 = 0.01;

// The shader's view factors, mirrored term for term (tests).
const INV_2PI: f32 = 0.159_154_94;

/// A ground element to a vertical rectangle `h` high and `b` wide whose
/// bottom corner is the foot of the element's perpendicular, `c` away
/// (Howell's catalogue: plane element to a rectangle in a plane at 90
/// degrees to it).
pub fn vf_wall(c: f32, h: f32, b: f32) -> f32 {
    if h <= 0.0 || b <= 0.0 {
        return 0.0;
    }
    let (x, y) = (h / b, c.max(1e-4) / b);
    let s = (x * x + y * y).sqrt();
    INV_2PI * ((1.0 / y).atan() - y / s * (1.0 / s).atan())
}

/// A ground element to a horizontal rectangle `a` by `b`, `c` over it,
/// with one corner straight above it (Howell's catalogue: plane element to
/// a parallel rectangle, the normal through a corner).
pub fn vf_ceiling(c: f32, a: f32, b: f32) -> f32 {
    if a <= 0.0 || b <= 0.0 {
        return 0.0;
    }
    let (x, y) = (a / c.max(1e-4), b / c.max(1e-4));
    let (sx, sy) = ((1.0 + x * x).sqrt(), (1.0 + y * y).sqrt());
    INV_2PI * (x / sx * (y / sx).atan() + y / sy * (x / sy).atan())
}

fn signed(w: f32, f: impl Fn(f32) -> f32) -> f32 {
    if w >= 0.0 { f(w) } else { -f(-w) }
}

/// The share of the sky a ground point (box frame x, z) sees that box
/// `half` x [h0, h1] hides: the faces it sees, summed.
pub fn box_view_factor(px: f32, pz: f32, half: [f32; 2], h0: f32, h1: f32) -> f32 {
    let [hx, hz] = half;
    let inside = px.abs() <= hx && pz.abs() <= hz;
    if inside && h0 < 0.002 {
        let r = 0.9 * hx.min(hz);
        let (ex, ez) = (px.abs() - hx + r, pz.abs() - hz + r);
        let sd = (ex.max(0.0).powi(2) + ez.max(0.0).powi(2)).sqrt() + ex.max(ez).min(0.0) - r;
        return 0.45 + 0.55 * smoothstep(0.0, 0.01, -sd);
    }
    let side = |c: f32, t: f32, half_w: f32| {
        let w = |e: f32| signed(e, |u| vf_wall(c, h1, u) - if h0 > 0.002 { vf_wall(c, h0, u) } else { 0.0 });
        w(half_w - t) - w(-half_w - t)
    };
    let mut f = 0.0;
    if h0 > 0.002 {
        let p = |u: f32, v: f32| u.signum() * v.signum() * vf_ceiling(h0, u.abs(), v.abs());
        f += p(hx - px, hz - pz) - p(-hx - px, hz - pz) - p(hx - px, -hz - pz) + p(-hx - px, -hz - pz);
    }
    if px > hx {
        f += side(px - hx, pz, hz);
    }
    if px < -hx {
        f += side(-hx - px, pz, hz);
    }
    if pz > hz {
        f += side(pz - hz, px, hx);
    }
    if pz < -hz {
        f += side(-hz - pz, px, hx);
    }
    f.clamp(0.0, 1.0)
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

    /// The view factors meet their known limits: a long wall hides
    /// (1 - c / sqrt(c^2 + h^2)) / 2 of the sky, a tall narrow one
    /// atan(b / c) / 2 pi per side, a wide ceiling a quarter per quadrant;
    /// a box hides all of it under its sole, less with distance, and a
    /// box lifted off the ground hides less right beside it.
    #[test]
    fn contact_view_factors_meet_their_limits() {
        for (c, h) in [(0.01f32, 0.07f32), (0.05, 0.07), (0.2, 0.5)] {
            let long = 2.0 * vf_wall(c, h, 1e4);
            let exact = 0.5 * (1.0 - c / (c * c + h * h).sqrt());
            assert!((long - exact).abs() < 1e-4, "wall c {c} h {h}: {long} vs {exact}");
        }
        let tall = vf_wall(0.3, 1e5, 0.2);
        assert!((tall - (0.2f32 / 0.3).atan() / std::f32::consts::TAU).abs() < 1e-4, "{tall}");
        assert!((vf_ceiling(0.5, 1e5, 1e5) - 0.25).abs() < 1e-3);
        let half = [0.05, 0.14];
        assert_eq!(box_view_factor(0.0, 0.0, half, 0.0, 0.07), 1.0);
        assert!((box_view_factor(0.05, 0.14, half, 0.0, 0.07) - 0.45).abs() < 1e-3, "a corner keeps the faces' value");
        let edge = box_view_factor(0.06, 0.0, half, 0.0, 0.07);
        let near = box_view_factor(0.1, 0.0, half, 0.0, 0.07);
        let far = box_view_factor(0.3, 0.0, half, 0.0, 0.07);
        assert!(edge > 0.35 && edge < 0.5 && near < edge && far < 0.03 && far > 0.0, "{edge} {near} {far}");
        let lifted = box_view_factor(0.06, 0.0, half, 0.05, 0.12);
        assert!(lifted < edge && lifted > 0.0, "{lifted}");
        // Under a seat 0.35 m up, 0.36 x 0.25 m: about A / (pi c^2) straight under it.
        let seat = box_view_factor(0.0, 0.0, [0.18, 0.125], 0.35, 0.65);
        assert!(seat > 0.15 && seat < 0.3, "{seat}");
    }

    #[test]
    fn the_phone_is_a_handset() {
        let b = phone_body();
        let size: Vec<f32> = (0..3).map(|k| b.max[k] - b.min[k]).collect();
        assert!((size[1] - 0.1467).abs() < 1e-4 && (size[0] - 0.0715).abs() < 1e-4, "{size:?}");
        assert_eq!(b.triangles, 24);
    }
}
