//! Parts to triangles (the emitters of src/world/buildings/authored/kit/build.js):
//! boxes as six quads, profiles as swept quads plus earcut end caps, plates
//! and prisms likewise, every vertex clamped to the footprint's plan bounds
//! plus the part's projection allowance (`makeClamp`). Flat normals per face.
//! UVs are not emitted: the renderer projects the kit maps in world space.

use crate::elevation::{Part, Plan, Shape};
use crate::frame::{DEFAULT_PROJECTION_ALLOWANCE, FITTING_PROJECTION_ALLOWANCE, Frame, point_in_polygon};

/// Triangles of one material: positions, normals, indices local to `positions`.
#[derive(Clone, Debug, Default)]
pub struct Sink {
    pub positions: Vec<[f64; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
}

impl Sink {
    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }

    fn quad(&mut self, a: [f64; 3], b: [f64; 3], c: [f64; 3], d: [f64; 3], n: [f64; 3]) {
        let base = self.positions.len() as u32;
        let nf = [n[0] as f32, n[1] as f32, n[2] as f32];
        for p in [a, b, c, d] {
            self.positions.push(p);
            self.normals.push(nf);
        }
        self.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    /// A planar polygon, triangulated on `plan2d` (or on its own first two
    /// non-degenerate axes); `reverse` flips the winding.
    fn polygon(&mut self, points: &[[f64; 3]], n: [f64; 3], reverse: bool, plan2d: &[[f64; 2]]) {
        if points.len() < 3 {
            return;
        }
        let mut tri: Vec<u32> = vec![];
        let mut ear = earcut::Earcut::<f64>::new();
        ear.earcut(plan2d.iter().copied(), &[] as &[u32], &mut tri);
        let base = self.positions.len() as u32;
        let nf = [n[0] as f32, n[1] as f32, n[2] as f32];
        for p in points {
            self.positions.push(*p);
            self.normals.push(nf);
        }
        for t in tri.chunks_exact(3) {
            if reverse {
                self.indices.extend_from_slice(&[base + t[0], base + t[2], base + t[1]]);
            } else {
                self.indices.extend_from_slice(&[base + t[0], base + t[1], base + t[2]]);
            }
        }
    }
}

/// The vertex clamp of `makeClamp(plan, frame)`.
pub struct Clamp<'a> {
    frame: &'a Frame,
    y_min: f64,
    base_y: f64,
    roof_top: f64,
    plant_allowance: f64,
    pub clamped: usize,
}

impl<'a> Clamp<'a> {
    pub fn new(plan: &Plan, frame: &'a Frame) -> Self {
        Self {
            frame,
            y_min: plan.base_y - 0.12,
            base_y: plan.base_y,
            roof_top: plan.envelope.roof_top,
            plant_allowance: plan.envelope.roof_plant_allowance,
            clamped: 0,
        }
    }

    fn allowance(&self, part: &Part) -> f64 {
        if part.fitting {
            return FITTING_PROJECTION_ALLOWANCE;
        }
        if let Some(e) = part.edge.and_then(|e| self.frame.edges.get(e)) {
            return e.allowance;
        }
        if self.frame.allowance.is_finite() { self.frame.allowance } else { DEFAULT_PROJECTION_ALLOWANCE }
    }

    pub fn apply(&mut self, part: &Part, p: [f64; 3]) -> [f64; 3] {
        let allowance = self.allowance(part);
        let roof = part.meta_flag("roof");
        let y_max = self.base_y + self.roof_top + if roof { self.plant_allowance } else { 0.02 };
        let b = &self.frame.plan_bounds;
        let local = self.frame.to_plan(p[0], p[2]);
        let (mut x, mut z, mut y) = (local.x, local.z, p[1]);
        let mut clamped = false;
        let (min_x, max_x, min_z, max_z) = (b.min_x - allowance, b.max_x + allowance, b.min_z - allowance, b.max_z + allowance);
        let over = (min_x - x).max(x - max_x).max(min_z - z).max(z - max_z).max(0.0);
        if over > 1e-6 {
            clamped = true;
            x = x.clamp(min_x, max_x);
            z = z.clamp(min_z, max_z);
        }
        let floor = if part.meta_flag("belowFloor") { part.y0 } else { self.y_min };
        if y < floor {
            y = floor;
            clamped = true;
        }
        if y > y_max {
            y = y_max;
            clamped = true;
        }
        if clamped {
            self.clamped += 1;
            let w = self.frame.to_world(x, z);
            return [w.x, y, w.z];
        }
        p
    }
}

/// Emit one part into `sink` (the JS emitters' faces, in their order).
pub fn emit_part(sink: &mut Sink, part: &Part, clamp: &mut Clamp) {
    match &part.shape {
        Shape::Box => emit_box(sink, part, clamp),
        Shape::Profile(p) => emit_profile(sink, part, p, clamp),
        Shape::Plate(p) => emit_plate(sink, part, p, clamp),
        Shape::Prism(_) => emit_prism(sink, part, clamp),
    }
}

fn basis_point(part: &Part, s: f64, y: f64, w: f64) -> [f64; 3] {
    let (x, z) = part.basis.at(s, w);
    [x, y, z]
}

fn emit_box(sink: &mut Sink, part: &Part, clamp: &mut Clamp) {
    let [ux, uz] = part.basis.u;
    let [nx, nz] = part.basis.n;
    let (s0, s1, y0, y1, w0, w1) = (part.s0, part.s1, part.y0, part.y1, part.w0, part.w1);
    let mut p = |s, y, w| clamp.apply(part, basis_point(part, s, y, w));
    let q = [
        ([p(s0, y0, w1), p(s1, y0, w1), p(s1, y1, w1), p(s0, y1, w1)], [nx, 0.0, nz]),
        ([p(s1, y0, w0), p(s0, y0, w0), p(s0, y1, w0), p(s1, y1, w0)], [-nx, 0.0, -nz]),
        ([p(s1, y0, w1), p(s1, y0, w0), p(s1, y1, w0), p(s1, y1, w1)], [ux, 0.0, uz]),
        ([p(s0, y0, w0), p(s0, y0, w1), p(s0, y1, w1), p(s0, y1, w0)], [-ux, 0.0, -uz]),
        ([p(s0, y1, w1), p(s1, y1, w1), p(s1, y1, w0), p(s0, y1, w0)], [0.0, 1.0, 0.0]),
        ([p(s0, y0, w0), p(s1, y0, w0), p(s1, y0, w1), p(s0, y0, w1)], [0.0, -1.0, 0.0]),
    ];
    for ([a, b, c, d], n) in q {
        sink.quad(a, b, c, d, n);
    }
}

fn emit_profile(sink: &mut Sink, part: &Part, profile: &[[f64; 2]], clamp: &mut Clamp) {
    let [ux, uz] = part.basis.u;
    let [nx, nz] = part.basis.n;
    let n = profile.len();
    let mut area = 0.0;
    for i in 0..n {
        let [w0, y0] = profile[i];
        let [w1, y1] = profile[(i + 1) % n];
        area += w0 * y1 - w1 * y0;
    }
    let ccw = area > 0.0;
    for i in 0..n {
        let [wa, ya] = profile[i];
        let [wb, yb] = profile[(i + 1) % n];
        let (dw, dy) = (wb - wa, yb - ya);
        let len = dw.hypot(dy);
        if len < 1e-6 {
            continue;
        }
        let (mut pw, mut py) = (dy / len, -dw / len);
        if !ccw {
            pw = -pw;
            py = -py;
        }
        let normal = [nx * pw, py, nz * pw];
        let a0 = clamp.apply(part, basis_point(part, part.s0, ya, wa));
        let b0 = clamp.apply(part, basis_point(part, part.s1, ya, wa));
        let b1 = clamp.apply(part, basis_point(part, part.s1, yb, wb));
        let a1 = clamp.apply(part, basis_point(part, part.s0, yb, wb));
        if ccw {
            sink.quad(a0, b0, b1, a1, normal);
        } else {
            sink.quad(b0, a0, a1, b1, normal);
        }
    }
    let plan: Vec<[f64; 2]> = profile.to_vec();
    let end: Vec<[f64; 3]> = profile.iter().map(|[w, y]| clamp.apply(part, basis_point(part, part.s1, *y, *w))).collect();
    let start: Vec<[f64; 3]> = profile.iter().map(|[w, y]| clamp.apply(part, basis_point(part, part.s0, *y, *w))).collect();
    sink.polygon(&end, [ux, 0.0, uz], true, &plan);
    sink.polygon(&start, [-ux, 0.0, -uz], false, &plan);
}

fn emit_plate(sink: &mut Sink, part: &Part, pts: &[[f64; 2]], clamp: &mut Clamp) {
    let [ux, uz] = part.basis.u;
    let [nx, nz] = part.basis.n;
    let count = pts.len();
    let mut area = 0.0;
    for i in 0..count {
        let [sa, ya] = pts[i];
        let [sb, yb] = pts[(i + 1) % count];
        area += sa * yb - sb * ya;
    }
    let ordered: Vec<[f64; 2]> = if area > 0.0 { pts.to_vec() } else { pts.iter().rev().copied().collect() };
    let front: Vec<[f64; 3]> = ordered.iter().map(|[s, y]| clamp.apply(part, basis_point(part, *s, *y, part.w1))).collect();
    let back: Vec<[f64; 3]> = ordered.iter().map(|[s, y]| clamp.apply(part, basis_point(part, *s, *y, part.w0))).collect();
    sink.polygon(&front, [nx, 0.0, nz], false, &ordered);
    sink.polygon(&back, [-nx, 0.0, -nz], true, &ordered);
    for i in 0..count {
        let [sa, ya] = ordered[i];
        let [sb, yb] = ordered[(i + 1) % count];
        let (ds, dy) = (sb - sa, yb - ya);
        let len = ds.hypot(dy);
        if len < 1e-6 {
            continue;
        }
        let (es, ey) = (dy / len, -ds / len);
        let normal = [ux * es, ey, uz * es];
        let a0 = clamp.apply(part, basis_point(part, sa, ya, part.w0));
        let b0 = clamp.apply(part, basis_point(part, sb, yb, part.w0));
        let b1 = clamp.apply(part, basis_point(part, sb, yb, part.w1));
        let a1 = clamp.apply(part, basis_point(part, sa, ya, part.w1));
        sink.quad(a0, b0, b1, a1, normal);
    }
}

fn emit_prism(sink: &mut Sink, part: &Part, clamp: &mut Clamp) {
    let Shape::Prism(pts) = &part.shape else { return };
    let count = pts.len();
    if part.side_faces {
        for i in 0..count {
            let (a, b) = (pts[i], pts[(i + 1) % count]);
            let (dx, dz) = (b.x - a.x, b.z - a.z);
            let len = dx.hypot(dz);
            if len < 1e-6 {
                continue;
            }
            let (mut nx, mut nz) = (-dz / len, dx / len);
            let (mx, mz) = ((a.x + b.x) / 2.0 + nx * 0.05, (a.z + b.z) / 2.0 + nz * 0.05);
            if point_in_polygon(mx, mz, pts) {
                nx = -nx;
                nz = -nz;
            }
            let p00 = clamp.apply(part, [a.x, part.y0, a.z]);
            let p10 = clamp.apply(part, [b.x, part.y0, b.z]);
            let p11 = clamp.apply(part, [b.x, part.y1, b.z]);
            let p01 = clamp.apply(part, [a.x, part.y1, a.z]);
            let e1 = [p10[0] - p00[0], p10[2] - p00[2]];
            let h = part.y1 - part.y0;
            let cross = [-e1[1] * h, e1[0] * h];
            let normal = [nx, 0.0, nz];
            if cross[0] * nx + cross[1] * nz >= 0.0 {
                sink.quad(p00, p10, p11, p01, normal);
            } else {
                sink.quad(p10, p00, p01, p11, normal);
            }
        }
    }
    let plan: Vec<[f64; 2]> = pts.iter().map(|p| [p.x, p.z]).collect();
    if part.top_cap {
        let top: Vec<[f64; 3]> = pts.iter().map(|p| clamp.apply(part, [p.x, part.y1, p.z])).collect();
        sink.polygon(&top, [0.0, 1.0, 0.0], true, &plan);
    }
    if part.bottom_cap {
        let bottom: Vec<[f64; 3]> = pts.iter().map(|p| clamp.apply(part, [p.x, part.y0, p.z])).collect();
        sink.polygon(&bottom, [0.0, -1.0, 0.0], false, &plan);
    }
}
