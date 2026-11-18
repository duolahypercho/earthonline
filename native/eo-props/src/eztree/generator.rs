//! The vendored ez-tree generator (`src/render/vegetation/ez-tree/vendor/`
//! tree.js, branch.js, rng.js, options.js), ported line for line: the same
//! RNG and call order, the same three.js math (Euler XYZ, quaternions, the
//! matrix round trip `Euler.setFromQuaternion` takes), the same vertex and
//! index order. Doubles throughout, as JS; the buffers are rounded to f32 where
//! the JS builds its `Float32Array`s.
//!
//! Source: https://github.com/dgreenheck/ez-tree `src/lib` at commit
//! dcf309bd86bd521083d9c70f01f2de45fdc7c457 (MIT, Copyright (c) Daniel
//! Greenheck; see native/eo-props/src/eztree/LICENSE).
//! Not ported: the trellis (no species enables it; asked for, it is an error),
//! the materials and wind shader (never rendered by the game either).

use anyhow::{Result, bail};
use serde_json::Value;
use std::f64::consts::PI;

// ---------------------------------------------------------------------------
// JS number semantics
// ---------------------------------------------------------------------------

/// `Math.max(a, b)`: NaN if either is NaN.
pub(crate) fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { f64::NAN } else if a > b { a } else if b > a { b } else if a == 0.0 && b == 0.0 { if a.is_sign_positive() { a } else { b } } else { a }
}

/// `Math.min(a, b)`: NaN if either is NaN.
pub(crate) fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { f64::NAN } else if a < b { a } else if b < a { b } else if a == 0.0 && b == 0.0 { if a.is_sign_negative() { a } else { b } } else { a }
}

/// `Math.round(x)`: ties toward +infinity.
pub(crate) fn js_round(x: f64) -> f64 {
    if !x.is_finite() { return x; }
    let f = x.floor();
    if x - f >= 0.5 { f + 1.0 } else { f }
}

/// `Math.hypot(...)` as V8 computes it (max scaling, compensated sum).
pub(crate) fn js_hypot(values: &[f64]) -> f64 {
    let mut max = 0.0f64;
    let mut nan = false;
    let mut abs = [0.0f64; 4];
    for (i, v) in values.iter().enumerate() {
        let a = v.abs();
        if a.is_nan() { nan = true; } else if a > max { max = a; }
        abs[i] = a;
    }
    if max == f64::INFINITY { return f64::INFINITY; }
    if nan { return f64::NAN; }
    if max == 0.0 { return 0.0; }
    let mut sum = 0.0f64;
    let mut compensation = 0.0f64;
    for a in abs.iter().take(values.len()) {
        let n = a / max;
        let summand = n * n - compensation;
        let preliminary = sum + summand;
        compensation = (preliminary - sum) - summand;
        sum = preliminary;
    }
    sum.sqrt() * max
}

/// `clamp` of three's MathUtils.
fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    js_max(lo, js_min(hi, v))
}

// ---------------------------------------------------------------------------
// three.js math, in three's operation order
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct V3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl V3 {
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }
    fn add(self, o: V3) -> V3 {
        V3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
    fn sub(self, o: V3) -> V3 {
        V3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
    fn mul(self, s: f64) -> V3 {
        V3::new(self.x * s, self.y * s, self.z * s)
    }
    fn dot(self, o: V3) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    fn cross(a: V3, b: V3) -> V3 {
        V3::new(a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x)
    }
    fn length(self) -> f64 {
        (self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
    }
    /// `normalize()` = `divideScalar(length() || 1)` = `multiplyScalar(1 / l)`.
    fn normalize(self) -> V3 {
        let l = self.length();
        let l = if l == 0.0 || l.is_nan() { 1.0 } else { l };
        self.mul(1.0 / l)
    }
    fn lerp(a: V3, b: V3, t: f64) -> V3 {
        V3::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t, a.z + (b.z - a.z) * t)
    }
    fn apply_quat(self, q: Quat) -> V3 {
        let (vx, vy, vz) = (self.x, self.y, self.z);
        let tx = 2.0 * (q.y * vz - q.z * vy);
        let ty = 2.0 * (q.z * vx - q.x * vz);
        let tz = 2.0 * (q.x * vy - q.y * vx);
        V3::new(vx + q.w * tx + q.y * tz - q.z * ty, vy + q.w * ty + q.z * tx - q.x * tz, vz + q.w * tz + q.x * ty - q.y * tx)
    }
    fn apply_euler(self, e: Euler) -> V3 {
        self.apply_quat(Quat::from_euler(e))
    }
}

/// An XYZ Euler (three's default order).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Euler {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Quat {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub w: f64,
}

impl Quat {
    fn from_euler(e: Euler) -> Quat {
        let c1 = (e.x / 2.0).cos();
        let c2 = (e.y / 2.0).cos();
        let c3 = (e.z / 2.0).cos();
        let s1 = (e.x / 2.0).sin();
        let s2 = (e.y / 2.0).sin();
        let s3 = (e.z / 2.0).sin();
        Quat {
            x: s1 * c2 * c3 + c1 * s2 * s3,
            y: c1 * s2 * c3 - s1 * c2 * s3,
            z: c1 * c2 * s3 + s1 * s2 * c3,
            w: c1 * c2 * c3 - s1 * s2 * s3,
        }
    }
    fn from_axis_angle(axis: V3, angle: f64) -> Quat {
        let half = angle / 2.0;
        let s = half.sin();
        Quat { x: axis.x * s, y: axis.y * s, z: axis.z * s, w: half.cos() }
    }
    /// `multiplyQuaternions(a, b)`.
    fn mul(a: Quat, b: Quat) -> Quat {
        let (qax, qay, qaz, qaw) = (a.x, a.y, a.z, a.w);
        let (qbx, qby, qbz, qbw) = (b.x, b.y, b.z, b.w);
        Quat {
            x: qax * qbw + qaw * qbx + qay * qbz - qaz * qby,
            y: qay * qbw + qaw * qby + qaz * qbx - qax * qbz,
            z: qaz * qbw + qaw * qbz + qax * qby - qay * qbx,
            w: qaw * qbw - qax * qbx - qay * qby - qaz * qbz,
        }
    }
    /// `this.slerp(qb, t)`.
    fn slerp(self, qb: Quat, t: f64) -> Quat {
        if t == 0.0 {
            return self;
        }
        if t == 1.0 {
            return qb;
        }
        let (x, y, z, w) = (self.x, self.y, self.z, self.w);
        let mut cos_half = w * qb.w + x * qb.x + y * qb.y + z * qb.z;
        let mut out = if cos_half < 0.0 {
            cos_half = -cos_half;
            Quat { x: -qb.x, y: -qb.y, z: -qb.z, w: -qb.w }
        } else {
            qb
        };
        if cos_half >= 1.0 {
            return self;
        }
        let sqr_sin = 1.0 - cos_half * cos_half;
        if sqr_sin <= f64::EPSILON {
            let s = 1.0 - t;
            out = Quat { w: s * w + t * out.w, x: s * x + t * out.x, y: s * y + t * out.y, z: s * z + t * out.z };
            let l = (out.x * out.x + out.y * out.y + out.z * out.z + out.w * out.w).sqrt();
            return if l == 0.0 {
                Quat { x: 0.0, y: 0.0, z: 0.0, w: 1.0 }
            } else {
                let l = 1.0 / l;
                Quat { x: out.x * l, y: out.y * l, z: out.z * l, w: out.w * l }
            };
        }
        let sin_half = sqr_sin.sqrt();
        let half = sin_half.atan2(cos_half);
        let ra = ((1.0 - t) * half).sin() / sin_half;
        let rb = (t * half).sin() / sin_half;
        Quat { w: w * ra + out.w * rb, x: x * ra + out.x * rb, y: y * ra + out.y * rb, z: z * ra + out.z * rb }
    }
}

impl Euler {
    /// `Euler.setFromQuaternion(q)` (XYZ): `makeRotationFromQuaternion` then
    /// `setFromRotationMatrix`.
    fn from_quat(q: Quat) -> Euler {
        let (x, y, z, w) = (q.x, q.y, q.z, q.w);
        let (x2, y2, z2) = (x + x, y + y, z + z);
        let (xx, xy, xz) = (x * x2, x * y2, x * z2);
        let (yy, yz, zz) = (y * y2, y * z2, z * z2);
        let (wx, wy, wz) = (w * x2, w * y2, w * z2);
        let m11 = (1.0 - (yy + zz)) * 1.0;
        let m12 = (xy - wz) * 1.0;
        let m13 = (xz + wy) * 1.0;
        let m22 = (1.0 - (xx + zz)) * 1.0;
        let m23 = (yz - wx) * 1.0;
        let m32 = (yz + wx) * 1.0;
        let m33 = (1.0 - (xx + yy)) * 1.0;
        let ey = clamp(m13, -1.0, 1.0).asin();
        if m13.abs() < 0.9999999 {
            Euler { x: (-m23).atan2(m33), y: ey, z: (-m12).atan2(m11) }
        } else {
            Euler { x: m32.atan2(m22), y: ey, z: 0.0 }
        }
    }
}

// ---------------------------------------------------------------------------
// rng.js
// ---------------------------------------------------------------------------

pub(crate) struct Rng {
    m_w: i32,
    m_z: i32,
}

impl Rng {
    pub fn new(seed: f64) -> Self {
        // `(123456789 + seed) & 0xffffffff`: ToInt32.
        let to_i32 = |v: f64| (v as i64) as i32;
        Self { m_w: to_i32(123456789.0 + seed), m_z: to_i32(987654321.0 - seed) }
    }
    /// `random(max = 1, min = 0)`.
    pub fn random(&mut self, max: f64, min: f64) -> f64 {
        self.m_z = (36969i64 * (self.m_z & 65535) as i64 + (self.m_z >> 16) as i64) as i32;
        self.m_w = (18000i64 * (self.m_w & 65535) as i64 + (self.m_w >> 16) as i64) as i32;
        let result = ((self.m_z.wrapping_shl(16)) as i64 + (self.m_w & 65535) as i64) as u32;
        let result = result as f64 / 4294967296.0;
        (max - min) * result + min
    }
    pub fn unit(&mut self) -> f64 {
        self.random(1.0, 0.0)
    }
}

// ---------------------------------------------------------------------------
// options.js (the fields the generator reads), presets merged by the caller
// ---------------------------------------------------------------------------

/// A per-level table (`{0: .., 1: .., 2: .., 3: ..}`); a missing level is
/// `undefined` in JS, NaN here.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Levels(pub [f64; 4]);

impl Levels {
    fn from(v: &Value) -> Self {
        let mut out = [f64::NAN; 4];
        for (i, o) in out.iter_mut().enumerate() {
            if let Some(x) = v.get(i.to_string()).and_then(Value::as_f64) {
                *o = x;
            }
        }
        Levels(out)
    }
    fn at(&self, level: usize) -> f64 {
        self.0.get(level).copied().unwrap_or(f64::NAN)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TreeType {
    Deciduous,
    Evergreen,
    Other,
}

#[derive(Clone, Debug)]
pub(crate) struct TreeOptions {
    pub seed: f64,
    pub kind: TreeType,
    pub texture_scale_x: f64,
    pub levels: usize,
    pub angle: Levels,
    pub children: Levels,
    pub force_dir: V3,
    pub force_strength: f64,
    pub gnarliness: Levels,
    pub length: Levels,
    pub radius: Levels,
    pub sections: Levels,
    pub segments: Levels,
    pub start: Levels,
    pub taper: Levels,
    pub twist: Levels,
    pub leaf_double: bool,
    pub leaf_angle: f64,
    pub leaf_count: f64,
    pub leaf_start: f64,
    pub leaf_size: f64,
    pub leaf_size_variance: f64,
    pub rounded_normals: bool,
}

/// geometry.js `mergeInto(target, source)`.
pub(crate) fn merge_into(target: &mut Value, source: &Value) {
    let Some(src) = source.as_object() else { return };
    if !target.is_object() {
        *target = Value::Object(Default::default());
    }
    let t = target.as_object_mut().unwrap();
    for (k, v) in src {
        if v.is_object() {
            let mut sub = match t.get(k) {
                Some(x) if x.is_object() || x.is_array() => x.clone(),
                _ => Value::Object(Default::default()),
            };
            merge_into(&mut sub, v);
            t.insert(k.clone(), sub);
        } else {
            t.insert(k.clone(), v.clone());
        }
    }
}

/// options.js `TreeOptions.copy(source, target)`: only keys the target has.
pub(crate) fn options_copy(target: &mut Value, source: &Value) {
    let (Some(src), Some(t)) = (source.as_object(), target.as_object_mut()) else { return };
    for (k, v) in src {
        let Some(slot) = t.get_mut(k) else { continue };
        if v.is_object() {
            options_copy(slot, v);
        } else {
            *slot = v.clone();
        }
    }
}

impl TreeOptions {
    /// The typed view of a merged options object (as `tree.options` holds it).
    pub fn from_value(o: &Value) -> Result<Self> {
        let num = |v: &Value| v.as_f64().unwrap_or(f64::NAN);
        let b = &o["branch"];
        let l = &o["leaves"];
        if o["trellis"]["enabled"].as_bool() == Some(true) {
            bail!("ez-tree trellis is not ported");
        }
        let kind = match o["type"].as_str() {
            Some("deciduous") => TreeType::Deciduous,
            Some("evergreen") => TreeType::Evergreen,
            _ => TreeType::Other,
        };
        let levels = num(&b["levels"]);
        if !(levels >= 0.0) || levels.fract() != 0.0 || levels > 3.0 {
            bail!("ez-tree branch.levels {levels} unsupported");
        }
        let d = &b["force"]["direction"];
        Ok(Self {
            seed: num(&o["seed"]),
            kind,
            texture_scale_x: num(&o["bark"]["textureScale"]["x"]),
            levels: levels as usize,
            angle: Levels::from(&b["angle"]),
            children: Levels::from(&b["children"]),
            force_dir: V3::new(num(&d["x"]), num(&d["y"]), num(&d["z"])),
            force_strength: num(&b["force"]["strength"]),
            gnarliness: Levels::from(&b["gnarliness"]),
            length: Levels::from(&b["length"]),
            radius: Levels::from(&b["radius"]),
            sections: Levels::from(&b["sections"]),
            segments: Levels::from(&b["segments"]),
            start: Levels::from(&b["start"]),
            taper: Levels::from(&b["taper"]),
            twist: Levels::from(&b["twist"]),
            leaf_double: l["billboard"].as_str() == Some("double"),
            leaf_angle: num(&l["angle"]),
            leaf_count: num(&l["count"]),
            leaf_start: num(&l["start"]),
            leaf_size: num(&l["size"]),
            leaf_size_variance: num(&l["sizeVariance"]),
            rounded_normals: l["roundedNormals"].as_bool().unwrap_or(true),
        })
    }
}

/// tree.js `LODDetail`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Detail {
    pub section_stride: f64,
    pub segment_factor: f64,
    pub leaf_stride: f64,
    pub leaf_scale: f64,
    /// None: the options' billboard.
    pub billboard_double: Option<bool>,
}

// ---------------------------------------------------------------------------
// tree.js
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Section {
    origin: V3,
    orientation: Euler,
    radius: f64,
}

struct SkelBranch {
    sections: Vec<Section>,
    segment_count: f64,
    base_radius: f64,
}

struct SkelLeaf {
    origin: V3,
    orientation: Euler,
    size: f64,
}

struct Branch {
    origin: V3,
    orientation: Euler,
    length: f64,
    radius: f64,
    level: usize,
    section_count: f64,
    segment_count: f64,
}

/// One part's buffers, as `#buildBufferGeometry` makes them (f32; indices as the
/// JS's Uint16Array holds them).
#[derive(Clone, Debug, Default)]
pub(crate) struct Buffers {
    pub verts: Vec<f32>,
    pub normals: Vec<f32>,
    pub uvs: Vec<f32>,
    pub indices: Vec<u32>,
}

#[derive(Default)]
struct RawBuffers {
    verts: Vec<f64>,
    normals: Vec<f64>,
    uvs: Vec<f64>,
    indices: Vec<f64>,
}

impl RawBuffers {
    fn finish(self) -> Buffers {
        Buffers {
            verts: self.verts.iter().map(|&v| v as f32).collect(),
            normals: self.normals.iter().map(|&v| v as f32).collect(),
            uvs: self.uvs.iter().map(|&v| v as f32).collect(),
            // new Uint16Array(indices): ToUint16 (modulo 65536).
            indices: self.indices.iter().map(|&v| ((v as i64) & 0xffff) as u32).collect(),
        }
    }
    fn vertex_count(&self) -> usize {
        self.verts.len() / 3
    }
}

pub(crate) struct Tree {
    o: TreeOptions,
    rng: Rng,
    queue: std::collections::VecDeque<Branch>,
    branches: Vec<SkelBranch>,
    leaves: Vec<SkelLeaf>,
}

impl Tree {
    /// `generateLODs([{ distance: 0, detail }])`: the skeleton, then one mesh.
    pub fn generate(o: TreeOptions, detail: &Detail) -> (Buffers, Buffers) {
        let rng = Rng::new(o.seed);
        let mut t = Tree { o, rng, queue: Default::default(), branches: vec![], leaves: vec![] };
        t.generate_skeleton();
        t.mesh_skeleton(detail)
    }

    fn generate_skeleton(&mut self) {
        let o = &self.o;
        self.queue.push_back(Branch {
            origin: V3::default(),
            orientation: Euler::default(),
            length: o.length.at(0),
            radius: o.radius.at(0),
            level: 0,
            section_count: o.sections.at(0),
            segment_count: o.segments.at(0),
        });
        while let Some(b) = self.queue.pop_front() {
            self.grow_branch(b);
        }
    }

    fn grow_branch(&mut self, branch: Branch) {
        let levels = self.o.levels;
        let mut orientation = branch.orientation;
        let mut origin = branch.origin;
        // `(type === 'Deciduous' ? levels - 1 : 1)`: never 'Deciduous' (case), so 1.
        let section_length = branch.length / branch.section_count / 1.0;
        let mut sections = Vec::new();
        let n = branch.section_count;
        let mut i = 0.0f64;
        while i <= n {
            let mut radius = branch.radius;
            if i == n && branch.level == levels {
                radius = 0.001;
            } else if self.o.kind == TreeType::Deciduous {
                radius *= 1.0 - self.o.taper.at(branch.level) * (i / n);
            } else if self.o.kind == TreeType::Evergreen {
                radius *= 1.0 - (i / n);
            }
            sections.push(Section { origin, orientation, radius });
            origin = origin.add(V3::new(0.0, section_length, 0.0).apply_euler(orientation));

            let g = js_max(1.0, 1.0 / radius.sqrt()) * self.o.gnarliness.at(branch.level);
            orientation.x += self.rng.random(g, -g);
            orientation.z += self.rng.random(g, -g);

            let mut q = Quat::from_euler(orientation);
            let twist = Quat::from_axis_angle(V3::new(0.0, 1.0, 0.0), self.o.twist.at(branch.level));
            q = Quat::mul(q, twist);
            let up = V3::new(0.0, 1.0, 0.0).apply_quat(q);
            let target = self.o.force_dir.normalize();
            let mut axis = V3::cross(up, target);
            let sin_full = axis.length();
            if sin_full > 1e-6 {
                axis = axis.mul(1.0 / sin_full);
                let full = sin_full.atan2(up.dot(target));
                let step = self.o.force_strength / radius;
                let clamped = js_max(-full, js_min(full, step));
                q = Quat::mul(Quat::from_axis_angle(axis, clamped), q);
            }
            orientation = Euler::from_quat(q);
            i += 1.0;
        }

        self.branches.push(SkelBranch { sections: sections.clone(), segment_count: branch.segment_count, base_radius: branch.radius });

        if self.o.kind == TreeType::Deciduous {
            let last = *sections.last().unwrap();
            if branch.level < levels {
                self.queue.push_back(Branch {
                    origin: last.origin,
                    orientation: last.orientation,
                    length: self.o.length.at(branch.level + 1),
                    radius: last.radius,
                    level: branch.level + 1,
                    section_count: branch.section_count,
                    segment_count: branch.segment_count,
                });
            } else {
                self.record_leaf(last.origin, last.orientation);
            }
        }

        if branch.level == levels {
            self.generate_leaves(&sections);
        } else if branch.level < levels {
            let count = self.o.children.at(branch.level);
            self.generate_child_branches(count, branch.level + 1, &sections);
        }
    }

    /// The section pair, blend and parent orientation at `start` along `sections`.
    fn along(sections: &[Section], start: f64) -> (Section, Section, f64, Euler) {
        let last = (sections.len() - 1) as f64;
        let index = (start * last).floor() as usize;
        let a = sections[index];
        let b = if index == sections.len() - 1 { a } else { sections[index + 1] };
        let alpha = (start - index as f64 / last) / (1.0 / last);
        let qa = Quat::from_euler(a.orientation);
        let qb = Quat::from_euler(b.orientation);
        let parent = Euler::from_quat(qb.slerp(qa, alpha));
        (a, b, alpha, parent)
    }

    fn child_orientation(parent: Euler, angle_deg: f64, radial: f64) -> Euler {
        let q1 = Quat::from_axis_angle(V3::new(1.0, 0.0, 0.0), angle_deg / (180.0 / PI));
        let q2 = Quat::from_axis_angle(V3::new(0.0, 1.0, 0.0), radial);
        let q3 = Quat::from_euler(parent);
        let q2 = Quat::mul(q2, q1);
        Euler::from_quat(Quat::mul(q3, q2))
    }

    fn generate_child_branches(&mut self, count: f64, level: usize, sections: &[Section]) {
        let radial_offset = self.rng.unit();
        let start_min = self.o.start.at(level);
        let height_step = (1.0 - start_min) / count;
        let slots = self.shuffled_indices(count);
        let mut i = 0.0f64;
        while i < count {
            let start = start_min + (i + self.rng.unit()) * height_step;
            let (a, b, alpha, parent) = Self::along(sections, start);
            let origin = V3::lerp(a.origin, b.origin, alpha);
            let radius = self.o.radius.at(level) * ((1.0 - alpha) * a.radius + alpha * b.radius);
            let jitter = self.rng.random(0.5, -0.5);
            let radial = 2.0 * PI * (radial_offset + (slots[i as usize] + jitter) / count);
            let orientation = Self::child_orientation(parent, self.o.angle.at(level), radial);
            let length = self.o.length.at(level) * if self.o.kind == TreeType::Evergreen { 1.0 - start } else { 1.0 };
            self.queue.push_back(Branch {
                origin,
                orientation,
                length,
                radius,
                level,
                section_count: self.o.sections.at(level),
                segment_count: self.o.segments.at(level),
            });
            i += 1.0;
        }
    }

    fn generate_leaves(&mut self, sections: &[Section]) {
        let radial_offset = self.rng.unit();
        let count = self.o.leaf_count;
        let start_min = self.o.leaf_start;
        let height_step = (1.0 - start_min) / count;
        let slots = self.shuffled_indices(count);
        let mut i = 0.0f64;
        while i < count {
            let start = start_min + (i + self.rng.unit()) * height_step;
            let (a, b, alpha, parent) = Self::along(sections, start);
            let origin = V3::lerp(a.origin, b.origin, alpha);
            let jitter = self.rng.random(0.5, -0.5);
            let radial = 2.0 * PI * (radial_offset + (slots[i as usize] + jitter) / count);
            let orientation = Self::child_orientation(parent, self.o.leaf_angle, radial);
            self.record_leaf(origin, orientation);
            i += 1.0;
        }
    }

    fn record_leaf(&mut self, origin: V3, orientation: Euler) {
        let v = self.o.leaf_size_variance;
        let size = self.o.leaf_size * (1.0 + self.rng.random(v, -v));
        self.leaves.push(SkelLeaf { origin, orientation, size });
    }

    fn shuffled_indices(&mut self, count: f64) -> Vec<f64> {
        let n = if count > 0.0 { count as usize } else { 0 };
        let mut arr: Vec<f64> = (0..n).map(|k| k as f64).collect();
        let mut k = n as i64 - 1;
        while k > 0 {
            let r = (self.rng.unit() * (k + 1) as f64).floor() as usize;
            arr.swap(k as usize, r);
            k -= 1;
        }
        arr
    }

    fn mesh_skeleton(&self, d: &Detail) -> (Buffers, Buffers) {
        let section_stride = js_max(1.0, d.section_stride.floor()) as usize;
        let leaf_stride = js_max(1.0, d.leaf_stride.floor()) as usize;
        let double = d.billboard_double.unwrap_or(self.o.leaf_double);
        let mut wood = RawBuffers::default();
        let mut leaves = RawBuffers::default();
        for b in &self.branches {
            self.mesh_branch(&mut wood, b, section_stride, d.segment_factor);
        }
        let mut i = 0;
        while i < self.leaves.len() {
            self.mesh_leaf(&mut leaves, &self.leaves[i], d.leaf_scale, double);
            i += leaf_stride;
        }
        (wood.finish(), leaves.finish())
    }

    fn mesh_leaf(&self, buf: &mut RawBuffers, leaf: &SkelLeaf, scale: f64, double: bool) {
        let size = leaf.size * scale;
        let (w, l) = (size, size);
        let mut create = |rotation: f64| {
            let i = buf.vertex_count() as f64;
            let spin = Euler { x: 0.0, y: rotation, z: 0.0 };
            let v: Vec<V3> = [V3::new(-w / 2.0, l, 0.0), V3::new(-w / 2.0, 0.0, 0.0), V3::new(w / 2.0, 0.0, 0.0), V3::new(w / 2.0, l, 0.0)]
                .iter()
                .map(|p| p.apply_euler(spin).apply_euler(leaf.orientation).add(leaf.origin))
                .collect();
            for p in &v {
                buf.verts.extend_from_slice(&[p.x, p.y, p.z]);
            }
            let n = V3::new(0.0, 0.0, 1.0).apply_euler(leaf.orientation);
            for p in &v {
                let m = if self.o.rounded_normals { n.add(*p).sub(leaf.origin).normalize() } else { n };
                buf.normals.extend_from_slice(&[m.x, m.y, m.z]);
            }
            buf.uvs.extend_from_slice(&[0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0]);
            buf.indices.extend_from_slice(&[i, i + 1.0, i + 2.0, i, i + 2.0, i + 3.0]);
        };
        create(0.0);
        if double {
            create(PI / 2.0);
        }
    }

    fn mesh_branch(&self, buf: &mut RawBuffers, b: &SkelBranch, stride: usize, factor: f64) {
        let segments = js_max(3.0, js_round(b.segment_count * factor));
        let wraps_x = js_max(1.0, js_round(b.base_radius * self.o.texture_scale_x));
        let mut sampled: Vec<&Section> = b.sections.iter().step_by(stride).collect();
        if (b.sections.len() - 1) % stride != 0 {
            sampled.push(b.sections.last().unwrap());
        }
        let offset = buf.vertex_count() as f64;
        let seg_n = segments as usize;
        for (k, s) in sampled.iter().enumerate() {
            let v_uv = if k % 2 == 0 { 0.0 } else { 1.0 };
            let mut first = None;
            for j in 0..seg_n {
                let angle = (2.0 * PI * j as f64) / segments;
                let (c, sn) = (angle.cos(), angle.sin());
                let vertex = V3::new(c, 0.0, sn).mul(s.radius).apply_euler(s.orientation).add(s.origin);
                let normal = V3::new(c, 0.0, sn).apply_euler(s.orientation).normalize();
                let u = (j as f64 / segments) * wraps_x;
                buf.verts.extend_from_slice(&[vertex.x, vertex.y, vertex.z]);
                buf.normals.extend_from_slice(&[normal.x, normal.y, normal.z]);
                buf.uvs.extend_from_slice(&[u, v_uv]);
                if j == 0 {
                    first = Some((vertex, normal));
                }
            }
            let (fv, fnm) = first.unwrap();
            buf.verts.extend_from_slice(&[fv.x, fv.y, fv.z]);
            buf.normals.extend_from_slice(&[fnm.x, fnm.y, fnm.z]);
            buf.uvs.extend_from_slice(&[wraps_x, v_uv]);
        }
        let n = segments + 1.0;
        for i in 0..sampled.len().saturating_sub(1) {
            for j in 0..seg_n {
                let v1 = offset + i as f64 * n + j as f64;
                let v2 = offset + i as f64 * n + (j as f64 + 1.0);
                let v3 = v1 + n;
                let v4 = v2 + n;
                buf.indices.extend_from_slice(&[v1, v3, v2, v2, v3, v4]);
            }
        }
    }
}
