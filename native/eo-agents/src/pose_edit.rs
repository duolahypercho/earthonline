//! Bake-time edits of a standing loop's poses (`motion::Edit`): what the
//! players' emotes need that no Rocketbox take holds as it is. A pose here
//! is every `rocketbox::JOINTS` entry's position and rotation in the cooked
//! frame (the avatar facing +Z, feet on y = 0), one frame of a loop, before
//! it becomes skinning matrices. Every edit is a rigid turn of a joint and
//! everything hanging from it, so a limb keeps its bone lengths and the
//! skin follows as it follows any take.
//!
//! * `Edit::Wave`: the right arm (clavicle to fingertips) as it is at one
//!   instant of another take, set on this loop's torso (the arm's own local
//!   rotations, so it hangs from wherever the idle's shoulder is), and the
//!   forearm swung about the elbow across the body's front.
//! * `Edit::Point`: the pointing hand's index finger as at bind (straight),
//!   the other fingers and the thumb curled toward the palm, the head and
//!   neck turned toward where the arm points.
//! * `Edit::Selfie`: the right hand reaches out, up and to the side of the face
//!   (an analytic two-bone IK, the elbow down and out), the hand turned so
//!   the palm faces the face with the index side up (a phone held upright
//!   in it, screen toward the face), the fingers round it, the head toward it.
//!
//! Angles and reaches are a person's ordinary ones: a finger's three joints
//! flex up to about 90, 100 and 80 degrees (a loose fist is about 70 / 85 /
//! 50); a selfie is taken at most of an arm's reach (the hand about 0.4 m
//! from the eyes, about their height, off to the side).

use crate::motion::Edit;
use crate::rocketbox::{BipSkeleton, JOINTS};
use glam::{Mat3, Quat, Vec3};

/// A joint's index in `JOINTS` (the names are this crate's own table).
pub fn joint(name: &str) -> usize {
    JOINTS.iter().position(|(b, _)| *b == name).unwrap_or_else(|| panic!("no joint {name}"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    fn tag(self) -> &'static str {
        match self {
            Side::Left => "L",
            Side::Right => "R",
        }
    }
    /// The palm normal's sign (`hand_frame`): the right hand's is cross(lateral, fingers).
    fn sign(self) -> f32 {
        match self {
            Side::Left => -1.0,
            Side::Right => 1.0,
        }
    }
    pub fn j(self, part: &str) -> usize {
        joint(&format!("Bip01 {} {part}", self.tag()))
    }
}

/// `k` and every joint hanging from it, parents first.
fn subtree(sk: &BipSkeleton, k: usize) -> Vec<usize> {
    let mut out = vec![k];
    let mut i = 0;
    while i < out.len() {
        let p = out[i];
        for c in 0..JOINTS.len() {
            if sk.parent[c] == Some(p) && !out.contains(&c) {
                out.push(c);
            }
        }
        i += 1;
    }
    out
}

/// Turn joint `k` and everything hanging from it by `d` about `pivot`.
pub fn rotate(sk: &BipSkeleton, p: &mut [Vec3], q: &mut [Quat], k: usize, pivot: Vec3, d: Quat) {
    for c in subtree(sk, k) {
        p[c] = pivot + d * (p[c] - pivot);
        q[c] = (d * q[c]).normalize();
    }
}

/// Set `k`'s rotation relative to its parent to `local` (in the bind's
/// terms: identity is the bind's own relation), carrying its subtree.
fn set_local(sk: &BipSkeleton, p: &mut [Vec3], q: &mut [Quat], k: usize, local: Quat) {
    let Some(par) = sk.parent[k] else { return };
    let bind_rel = sk.rot[par].inverse() * sk.rot[k];
    let want = (q[par] * local * bind_rel).normalize();
    let d = (want * q[k].inverse()).normalize();
    let at = p[k];
    rotate(sk, p, q, k, at, d);
}

/// The local rotations of `root`'s subtree (root's own, relative to its
/// parent, included) copied from another pose of the same skeleton.
pub fn copy_chain(sk: &BipSkeleton, p: &mut [Vec3], q: &mut [Quat], src_q: &[Quat], root: usize) {
    for k in subtree(sk, root) {
        let Some(par) = sk.parent[k] else { continue };
        let want = (q[par] * (src_q[par].inverse() * src_q[k])).normalize();
        let d = (want * q[k].inverse()).normalize();
        // The joint itself stays where it hangs from its parent (bind offset in the parent's frame).
        let at = p[par] + q[par] * (sk.rot[par].inverse() * (sk.pos[k] - sk.pos[par]));
        let shift = at - p[k];
        for c in subtree(sk, k) {
            p[c] += shift;
        }
        rotate(sk, p, q, k, at, d);
    }
}

/// The hand's frame: the fingers' direction (wrist to the middle finger's
/// knuckle), the palm normal (toward the palm side) and the lateral axis
/// (pinky to index knuckle).
pub fn hand_frame(p: &[Vec3], side: Side) -> (Vec3, Vec3, Vec3) {
    let h = p[side.j("Hand")];
    let f = (p[side.j("Finger2")] - h).normalize_or(Vec3::NEG_Y);
    let l = (p[side.j("Finger1")] - p[side.j("Finger4")]).normalize_or(Vec3::Z);
    let n = (side.sign() * l.cross(f)).normalize_or(Vec3::X);
    (f, n, f.cross(n).normalize_or(l) * side.sign())
}

/// Flex finger `finger` (0 the thumb .. 4 the pinky) by `deg` at each of its
/// three joints, toward the palm.
pub fn curl(sk: &BipSkeleton, p: &mut [Vec3], q: &mut [Quat], side: Side, finger: usize, deg: [f32; 3]) {
    let names = if finger == 0 { ["Finger0".to_string(), "Finger01".to_string(), "Finger02".to_string()] } else { [format!("Finger{finger}"), format!("Finger{finger}1"), format!("Finger{finger}2")] };
    for (i, n) in names.iter().enumerate() {
        let k = side.j(n);
        let (_, normal, _) = hand_frame(p, side);
        // The segment's direction: to the next joint, or the last segment's own.
        let d = if i < 2 { p[side.j(&names[i + 1])] - p[k] } else { p[k] - p[side.j(&names[i - 1])] };
        let axis = d.normalize_or(Vec3::Y).cross(normal).normalize_or(Vec3::X);
        let at = p[k];
        rotate(sk, p, q, k, at, Quat::from_axis_angle(axis, deg[i].to_radians()));
    }
}

/// A finger's three joints as at bind (the bind hand is open, fingers nearly straight).
pub fn straighten(sk: &BipSkeleton, p: &mut [Vec3], q: &mut [Quat], side: Side, finger: usize) {
    let names = if finger == 0 { ["Finger0".to_string(), "Finger01".to_string(), "Finger02".to_string()] } else { [format!("Finger{finger}"), format!("Finger{finger}1"), format!("Finger{finger}2")] };
    for n in &names {
        set_local(sk, p, q, side.j(n), Quat::IDENTITY);
    }
}

/// Two-bone IK: the hand of `side` to `target`, the elbow toward `pole`.
pub fn reach(sk: &BipSkeleton, p: &mut [Vec3], q: &mut [Quat], side: Side, target: Vec3, pole: Vec3) {
    let (u, e, h) = (side.j("UpperArm"), side.j("Forearm"), side.j("Hand"));
    let a = (p[e] - p[u]).length();
    let b = (p[h] - p[e]).length();
    let c = (target - p[u]).length().clamp(0.05, (a + b) * 0.999);
    // The elbow's interior angle for that reach.
    let want = ((a * a + b * b - c * c) / (2.0 * a * b)).clamp(-1.0, 1.0).acos();
    let (ue, he) = (p[u] - p[e], p[h] - p[e]);
    let now = ue.angle_between(he);
    let mut axis = ue.cross(he);
    if axis.length_squared() < 1e-8 {
        axis = ue.cross(pole);
    }
    let axis = axis.normalize_or(Vec3::X);
    let pe = p[e];
    rotate(sk, p, q, e, pe, Quat::from_axis_angle(axis, want - now));
    // Aim the arm at the target.
    let pu = p[u];
    let d = Quat::from_rotation_arc((p[h] - pu).normalize(), (target - pu).normalize());
    rotate(sk, p, q, u, pu, d);
    // Swing the elbow about the shoulder-target line toward the pole.
    let w = (target - pu).normalize();
    let flat = |v: Vec3| (v - w * v.dot(w)).normalize_or_zero();
    let (now_e, want_e) = (flat(p[e] - pu), flat(pole));
    if now_e != Vec3::ZERO && want_e != Vec3::ZERO {
        let ang = now_e.angle_between(want_e);
        let sign = if now_e.cross(want_e).dot(w) < 0.0 { -1.0 } else { 1.0 };
        rotate(sk, p, q, u, pu, Quat::from_axis_angle(w, sign * ang));
    }
}

/// Turn the hand of `side` (and the fingers) so its fingers point along
/// `fingers` and its palm faces `normal` (both re-made orthogonal).
pub fn orient_hand(sk: &BipSkeleton, p: &mut [Vec3], q: &mut [Quat], side: Side, fingers: Vec3, normal: Vec3) {
    let h = side.j("Hand");
    let (f0, n0, _) = hand_frame(p, side);
    let n1 = normal.normalize();
    let f1 = (fingers - n1 * fingers.dot(n1)).normalize_or(f0);
    let m0 = Mat3::from_cols(f0, n0, f0.cross(n0));
    let m1 = Mat3::from_cols(f1, n1, f1.cross(n1));
    let d = Quat::from_mat3(&(m1 * m0.transpose())).normalize();
    let at = p[h];
    rotate(sk, p, q, h, at, d);
}

/// Where the face points (the bind faces +Z), from the head's rotation.
fn face_dir(sk: &BipSkeleton, q: &[Quat]) -> Vec3 {
    let hd = joint("Bip01 Head");
    (q[hd] * sk.rot[hd].inverse() * Vec3::Z).normalize_or(Vec3::Z)
}

/// The eyes' midpoint (the head joint's, raised to eye height, where the skeleton has no eyes).
pub fn eyes(p: &[Vec3]) -> Vec3 {
    (p[joint("Bip01 LEye")] + p[joint("Bip01 REye")]) * 0.5
}

/// Turn the neck and head a share `weight` of the way to look at `target`
/// (the turn at most `max_deg`), 40 % at the neck and 60 % at the head.
pub fn look_at(sk: &BipSkeleton, p: &mut [Vec3], q: &mut [Quat], target: Vec3, weight: f32, max_deg: f32) {
    let (neck, head) = (joint("Bip01 Neck"), joint("Bip01 Head"));
    let from = face_dir(sk, q);
    let to = (target - eyes(p)).normalize_or(from);
    let full = Quat::from_rotation_arc(from, to);
    let (axis, ang) = full.to_axis_angle();
    let ang = (ang * weight).min(max_deg.to_radians());
    if ang < 1e-4 {
        return;
    }
    let pn = p[neck];
    rotate(sk, p, q, neck, pn, Quat::from_axis_angle(axis, ang * 0.4));
    let ph = p[head];
    rotate(sk, p, q, head, ph, Quat::from_axis_angle(axis, ang * 0.6));
}

/// The chest's facing, level (the bind faces +Z), and its right.
fn body_frame(sk: &BipSkeleton, q: &[Quat]) -> (Vec3, Vec3) {
    let s2 = joint("Bip01 Spine2");
    let f = q[s2] * sk.rot[s2].inverse() * Vec3::Z;
    let f = Vec3::new(f.x, 0.0, f.z).normalize_or(Vec3::Z);
    (f, f.cross(Vec3::Y))
}

/// Apply `edit` to frame `i` of a closed loop of `n` frames (`layer`: the
/// edit's other take's pose, for the wave).
pub fn apply(sk: &BipSkeleton, edit: &Edit, p: &mut [Vec3], q: &mut [Quat], i: usize, n: usize, layer: Option<&[Quat]>) {
    let u = if n > 1 { i as f32 / (n - 1) as f32 } else { 0.0 };
    match *edit {
        Edit::None => {}
        Edit::Wave { cycles, swing_deg, .. } => {
            let r = Side::Right;
            if let Some(src) = layer {
                copy_chain(sk, p, q, src, r.j("Clavicle"));
            }
            // The forearm swings across the body's front about the elbow.
            let (f, _) = body_frame(sk, q);
            let ang = swing_deg.to_radians() * (std::f32::consts::TAU * cycles as f32 * u).sin();
            let e = r.j("Forearm");
            let pe = p[e];
            rotate(sk, p, q, e, pe, Quat::from_axis_angle(f, ang));
            // A wave is open-handed: the fingers as at bind.
            for k in 1..5 {
                straighten(sk, p, q, r, k);
            }
        }
        Edit::Point => {
            let r = Side::Right;
            straighten(sk, p, q, r, 1);
            for (k, deg) in [(2, [72.0, 88.0, 55.0]), (3, [76.0, 90.0, 55.0]), (4, [80.0, 90.0, 55.0])] {
                curl(sk, p, q, r, k, deg);
            }
            curl(sk, p, q, r, 0, [10.0, 25.0, 20.0]);
            // The gaze along the arm, 3 m out.
            let (sh, hd) = (p[r.j("UpperArm")], p[r.j("Hand")]);
            let dir = (hd - sh).normalize_or(Vec3::Z);
            let target = sh + dir * 3.0;
            look_at(sk, p, q, target, 0.75, 60.0);
        }
        Edit::Selfie => {
            let r = Side::Right;
            let (f, right) = body_frame(sk, q);
            let sh = p[r.j("UpperArm")];
            let reach_m = (p[r.j("Forearm")] - sh).length() + (p[r.j("Hand")] - p[r.j("Forearm")]).length();
            // Out in front, up and to the right: the hand a little over eye
            // height, about 0.4 m from the eyes and 60 degrees off the face's
            // line (the face stays clear of the phone from the front).
            let dir = (f * 0.70 + Vec3::Y * 0.55 + right * 0.46).normalize();
            let target = sh + dir * reach_m * 0.93;
            reach(sk, p, q, r, target, Vec3::NEG_Y * 0.7 + right * 0.7);
            // The palm toward the face, the index side up (a phone upright in it).
            let face = eyes(p);
            let hd = p[r.j("Hand")];
            let normal = (face - hd).normalize_or(-f);
            let lateral = (Vec3::Y - normal * Vec3::Y.dot(normal)).normalize_or(Vec3::Y);
            orient_hand(sk, p, q, r, normal.cross(lateral), normal);
            for k in 1..5 {
                curl(sk, p, q, r, k, [38.0, 42.0, 25.0]);
            }
            curl(sk, p, q, r, 0, [5.0, 12.0, 10.0]);
            let phone = p[r.j("Hand")] + (p[r.j("Finger2")] - p[r.j("Hand")]) * 0.6;
            look_at(sk, p, q, phone, 0.55, 30.0);
        }
    }
}
