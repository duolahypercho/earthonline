//! Plain mesh accumulation for the landmark generators: a port of
//! src/world/landmarks/landmark-geometry.js (`MeshArrays`, `srgbHex`,
//! `gridSampler`, `sampleLandmarkGround`, the ring helpers). Every part is a
//! flat-shaded convex solid: a vertex carries its face normal and a linear
//! vertex colour, the indices are plain triangles. Arithmetic is f64 as the
//! JS's; `freeze` turns it into f32 as the JS's Float32Arrays do.

pub type V3 = [f64; 3];
pub type Rgb = [f64; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn length(a: V3) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}
fn normalise(a: V3) -> V3 {
    let l = length(a);
    let l = if l == 0.0 || l.is_nan() { 1.0 } else { l };
    [a[0] / l, a[1] / l, a[2] / l]
}

/// sRGB hex (0xRRGGBB) to the linear triple a vertex colour wants (`srgbHex`).
pub fn srgb_hex(hex: u32) -> Rgb {
    let lin = |c: u32| {
        let s = c as f64 / 255.0;
        if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    };
    [lin((hex >> 16) & 255), lin((hex >> 8) & 255), lin(hex & 255)]
}

pub fn shade(c: Rgb, k: f64) -> Rgb {
    [c[0] * k, c[1] * k, c[2] * k]
}

/// Which caps a box or hexahedron draws (`{ top, bottom }`, both by default).
#[derive(Clone, Copy, Debug)]
pub struct Faces {
    pub top: bool,
    pub bottom: bool,
}
pub const ALL: Faces = Faces { top: true, bottom: true };
pub const NO_BOTTOM: Faces = Faces { top: true, bottom: false };
pub const NO_TOP: Faces = Faces { top: false, bottom: true };

/// `MeshArrays`.
#[derive(Clone, Debug)]
pub struct MeshArrays {
    pub name: String,
    pub wire: bool,
    /// Wire members are drawn at this radius (`WIRE_DRAWN_RADIUS_M`).
    pub wire_drawn_radius: f64,
    pub positions: Vec<f64>,
    pub normals: Vec<f64>,
    pub colors: Vec<f64>,
    pub indices: Vec<u32>,
    /// First vertex of each painted road quad, in near-left, near-right,
    /// far-right, far-left order across the marking.
    pub marking_quads: Vec<u32>,
    /// First vertex of each traffic-bearing road panel, excluding the slab underside.
    pub pavement_quads: Vec<u32>,
    pub axes: Vec<f64>,
    pub dirs: Vec<f64>,
    pub radii: Vec<f64>,
    pub min_y: f64,
    pub max_y: f64,
}

/// A frozen record (`MeshArrays.freeze()`): f32 arrays, as the JS hands them to the page.
#[derive(Clone, Debug, Default)]
pub struct Frozen {
    pub name: String,
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    pub colors: Vec<f32>,
    pub indices: Vec<u32>,
    pub marking_quads: Vec<u32>,
    /// First vertex of each traffic-bearing road panel, excluding the slab underside.
    pub pavement_quads: Vec<u32>,
    /// Wire records only: a point on the member's axis, its direction, its true radius.
    pub axes: Option<Vec<f32>>,
    pub dirs: Option<Vec<f32>>,
    pub radii: Option<Vec<f32>>,
    pub min_y: f64,
    pub max_y: f64,
    /// A landmark deck's interior surfaces only (access_sky.rs): each
    /// vertex's baked sky visibility, 0..1, which the city shader spends on
    /// its indirect light; None elsewhere (the open sky).
    pub sky: Option<Vec<f32>>,
}

impl Frozen {
    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }
    pub fn vertices(&self) -> usize {
        self.positions.len() / 3
    }
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

impl MeshArrays {
    pub fn new(name: &str) -> Self {
        Self::with(name, false, 1.0)
    }
    pub fn wire(name: &str, drawn_radius: f64) -> Self {
        Self::with(name, true, drawn_radius)
    }
    fn with(name: &str, wire: bool, wire_drawn_radius: f64) -> Self {
        Self {
            name: name.to_string(),
            wire,
            wire_drawn_radius,
            positions: Vec::new(),
            normals: Vec::new(),
            colors: Vec::new(),
            indices: Vec::new(),
            marking_quads: Vec::new(),
            pavement_quads: Vec::new(),
            axes: Vec::new(),
            dirs: Vec::new(),
            radii: Vec::new(),
            min_y: f64::INFINITY,
            max_y: f64::NEG_INFINITY,
        }
    }

    pub fn vertex_count(&self) -> u32 {
        (self.positions.len() / 3) as u32
    }
    /// One vertex with its own normal (a smooth surface's), returning its index.
    pub fn vertex(&mut self, p: V3, n: V3, c: Rgb) -> u32 {
        let i = self.vertex_count();
        self.push_vertex(p, normalise(n), c);
        self.push_axis(1);
        i
    }
    /// A triangle of vertices already pushed (`vertex`).
    pub fn tri_idx(&mut self, a: u32, b: u32, c: u32) {
        self.indices.extend_from_slice(&[a, b, c]);
    }
    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }

    fn push_vertex(&mut self, p: V3, n: V3, c: Rgb) {
        assert!(p.iter().all(|v| v.is_finite()), "{}: non-finite vertex", self.name);
        self.positions.extend_from_slice(&p);
        self.normals.extend_from_slice(&n);
        self.colors.extend_from_slice(&c);
        if p[1] < self.min_y {
            self.min_y = p[1];
        }
        if p[1] > self.max_y {
            self.max_y = p[1];
        }
    }

    /// `pushAxis`: radius-0 axis data for vertices not made by `wire_segment` (wire records only).
    fn push_axis(&mut self, count: usize) {
        if !self.wire {
            return;
        }
        for _ in 0..count {
            self.axes.extend_from_slice(&[0.0, 0.0, 0.0]);
            self.dirs.extend_from_slice(&[0.0, 1.0, 0.0]);
            self.radii.push(0.0);
        }
    }

    /// `wireSegment`: four side faces at the drawn radius, no caps, each
    /// vertex carrying its axis point, the axis direction and the true radius.
    pub fn wire_segment(&mut self, p0: V3, p1: V3, diameter: f64, color: Rgb) {
        assert!(self.wire, "{}: wireSegment needs a wire record", self.name);
        let axis = sub(p1, p0);
        let len = length(axis);
        if !(len > 1e-6) {
            return;
        }
        let dir = scale(axis, 1.0 / len);
        let mut up = [0.0, 1.0, 0.0];
        if dir[1].abs() > 0.98 {
            up = [1.0, 0.0, 0.0];
        }
        let side = normalise(cross(dir, up));
        let norm = normalise(cross(side, dir));
        let r = diameter / 2.0;
        let drawn = r.max(self.wire_drawn_radius);
        let corner = |p: V3, sx: f64, sy: f64| add(add(p, scale(side, sx * drawn)), scale(norm, sy * drawn));
        let ring = |p: V3| [corner(p, -1.0, -1.0), corner(p, -1.0, 1.0), corner(p, 1.0, 1.0), corner(p, 1.0, -1.0)];
        let (r0, r1) = (ring(p0), ring(p1));
        for i in 0..4 {
            let j = (i + 1) % 4;
            let n = normalise(sub(scale(add(r0[i], r0[j]), 0.5), p0));
            let base = self.vertex_count();
            for p in [r0[i], r0[j], r1[j], r1[i]] {
                self.push_vertex(p, n, color);
            }
            self.axes.extend_from_slice(&[p0[0], p0[1], p0[2], p0[0], p0[1], p0[2], p1[0], p1[1], p1[2], p1[0], p1[1], p1[2]]);
            for _ in 0..4 {
                self.dirs.extend_from_slice(&dir);
            }
            self.radii.extend_from_slice(&[r, r, r, r]);
            self.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }

    /// One planar quad a-b-c-d (counter-clockwise seen from outside).
    pub fn quad(&mut self, a: V3, b: V3, c: V3, d: V3, color: Rgb) {
        let n = normalise(cross(sub(b, a), sub(d, a)));
        self.quad_n(a, b, c, d, color, n);
    }

    pub fn pavement_quad(&mut self, a: V3, b: V3, c: V3, d: V3, color: Rgb) {
        self.pavement_quads.push(self.vertex_count());
        self.quad(a, b, c, d, color);
    }

    pub fn marking_quad(&mut self, a: V3, b: V3, c: V3, d: V3, color: Rgb) {
        self.marking_quads.push(self.vertex_count());
        self.quad(a, b, c, d, color);
    }

    pub fn quad_n(&mut self, a: V3, b: V3, c: V3, d: V3, color: Rgb, n: V3) {
        let base = self.vertex_count();
        for p in [a, b, c, d] {
            self.push_vertex(p, n, color);
        }
        self.push_axis(4);
        self.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    /// A hexahedron from its eight corners (bottom ring b, top ring t).
    pub fn hexahedron(&mut self, b: [V3; 4], t: [V3; 4], color: Rgb, faces: Faces) {
        for i in 0..4 {
            let j = (i + 1) % 4;
            self.quad(b[i], b[j], t[j], t[i], color);
        }
        if faces.top {
            self.quad_n(t[0], t[1], t[2], t[3], color, [0.0, 1.0, 0.0]);
        }
        if faces.bottom {
            self.quad_n(b[3], b[2], b[1], b[0], color, [0.0, -1.0, 0.0]);
        }
    }

    /// A vertical box yawed about +y (`yaw`: local +x from world +x towards world +z).
    #[allow(clippy::too_many_arguments)]
    pub fn box_(&mut self, cx: f64, cz: f64, y0: f64, y1: f64, half_x: f64, half_z: f64, yaw: f64, color: Rgb, faces: Faces) {
        let (c, s) = (yaw.cos(), yaw.sin());
        let ring = |y: f64| {
            let mut r = [[-half_x, -half_z], [half_x, -half_z], [half_x, half_z], [-half_x, half_z]].map(|[x, z]| [cx + x * c - z * s, y, cz + x * s + z * c]);
            r.reverse();
            r
        };
        self.hexahedron(ring(y0), ring(y1), color, faces);
    }

    /// A straight member of rectangular section between two points (`beam`).
    pub fn beam(&mut self, p0: V3, p1: V3, width: f64, depth: f64, color: Rgb) {
        self.beam_up(p0, p1, width, depth, color, None);
    }

    pub fn beam_up(&mut self, p0: V3, p1: V3, width: f64, depth: f64, color: Rgb, up_hint: Option<V3>) {
        let axis = sub(p1, p0);
        let len = length(axis);
        if !(len > 1e-6) {
            return;
        }
        let dir = scale(axis, 1.0 / len);
        let mut up = up_hint.unwrap_or([0.0, 1.0, 0.0]);
        if (dir[0] * up[0] + dir[1] * up[1] + dir[2] * up[2]).abs() > 0.98 {
            up = [1.0, 0.0, 0.0];
        }
        let side = normalise(cross(dir, up));
        let norm = normalise(cross(side, dir));
        let (hw, hd) = (width / 2.0, depth / 2.0);
        let corner = |p: V3, sx: f64, sy: f64| add(add(p, scale(side, sx * hw)), scale(norm, sy * hd));
        let r0 = [corner(p0, -1.0, -1.0), corner(p0, -1.0, 1.0), corner(p0, 1.0, 1.0), corner(p0, 1.0, -1.0)];
        let r1 = [corner(p1, -1.0, -1.0), corner(p1, -1.0, 1.0), corner(p1, 1.0, 1.0), corner(p1, 1.0, -1.0)];
        for i in 0..4 {
            let j = (i + 1) % 4;
            self.quad(r0[i], r0[j], r1[j], r1[i], color);
        }
        self.quad(r0[3], r0[2], r0[1], r0[0], color);
        self.quad(r1[0], r1[1], r1[2], r1[3], color);
    }

    /// One triangle a-b-c (counter-clockwise seen from outside).
    pub fn tri(&mut self, a: V3, b: V3, c: V3, color: Rgb) {
        let n = normalise(cross(sub(b, a), sub(c, a)));
        let base = self.vertex_count();
        for p in [a, b, c] {
            self.push_vertex(p, n, color);
        }
        self.push_axis(3);
        self.indices.extend_from_slice(&[base, base + 1, base + 2]);
    }

    /// A vertical prism over a plan ring (`prism`; top a fan from the centroid).
    pub fn prism(&mut self, ring: &[[f64; 2]], y0: f64, y1: f64, color: Rgb, top: bool, bottom: bool) {
        let pts = orient_ring(ring);
        let n = pts.len();
        for i in 0..n {
            let [ax, az] = pts[i];
            let [bx, bz] = pts[(i + 1) % n];
            self.quad([ax, y0, az], [bx, y0, bz], [bx, y1, bz], [ax, y1, az], color);
        }
        let [cx, cz] = ring_centroid(&pts);
        if top {
            for i in 0..n {
                let (p, q) = (pts[i], pts[(i + 1) % n]);
                self.tri([cx, y1, cz], [p[0], y1, p[1]], [q[0], y1, q[1]], color);
            }
        }
        if bottom {
            for i in 0..n {
                let (p, q) = (pts[i], pts[(i + 1) % n]);
                self.tri([cx, y0, cz], [q[0], y0, q[1]], [p[0], y0, p[1]], color);
            }
        }
    }

    /// A loft between two plan rings of equal length at y0 and y1 (`loft`).
    pub fn loft(&mut self, ring0: &[[f64; 2]], y0: f64, ring1: &[[f64; 2]], y1: f64, color: Rgb, top: bool) {
        assert_eq!(ring0.len(), ring1.len(), "{}: loft rings differ in length", self.name);
        let flip = signed_area(ring0) > 0.0;
        let (a, b): (Vec<[f64; 2]>, Vec<[f64; 2]>) = if flip {
            (ring0.iter().rev().copied().collect(), ring1.iter().rev().copied().collect())
        } else {
            (ring0.to_vec(), ring1.to_vec())
        };
        let n = a.len();
        for i in 0..n {
            let j = (i + 1) % n;
            self.quad([a[i][0], y0, a[i][1]], [a[j][0], y0, a[j][1]], [b[j][0], y1, b[j][1]], [b[i][0], y1, b[i][1]], color);
        }
        if top {
            let [cx, cz] = ring_centroid(&b);
            for i in 0..n {
                let (p, q) = (b[i], b[(i + 1) % n]);
                self.tri([cx, y1, cz], [p[0], y1, p[1]], [q[0], y1, q[1]], color);
            }
        }
    }

    /// `lathe`: a surface of revolution about the vertical axis at
    /// (cx, cz); `profile` is [radius, y] bottom to top. `radius_at(angle, r)`
    /// may modulate the radius (fluting), `color_at(segment, mid_y)` the
    /// colour. A last radius of 0 closes the top.
    #[allow(clippy::too_many_arguments)]
    pub fn lathe(&mut self, cx: f64, cz: f64, profile: &[[f64; 2]], segments: usize, color: Rgb, radius_at: Option<&dyn Fn(f64, f64) -> f64>, color_at: Option<&dyn Fn(usize, f64) -> Rgb>) {
        let ring = |r: f64, k: usize| {
            let t = (k as f64 / segments as f64) * std::f64::consts::PI * 2.0;
            let rr = match radius_at {
                Some(f) => f(t, r),
                None => r,
            };
            [cx + t.cos() * rr, cz - t.sin() * rr]
        };
        for p in 0..profile.len().saturating_sub(1) {
            let [r0, y0] = profile[p];
            let [r1, y1] = profile[p + 1];
            let c = match color_at {
                Some(f) => f(p, (y0 + y1) / 2.0),
                None => color,
            };
            for k in 0..segments {
                let (a0, b0) = (ring(r0, k), ring(r0, k + 1));
                let (a1, b1) = (ring(r1, k), ring(r1, k + 1));
                if r1 <= 1e-6 {
                    self.tri([a0[0], y0, a0[1]], [b0[0], y0, b0[1]], [cx, y1, cz], c);
                } else {
                    self.quad([a0[0], y0, a0[1]], [b0[0], y0, b0[1]], [b1[0], y1, b1[1]], [a1[0], y1, a1[1]], c);
                }
            }
        }
    }

    /// `shadeOcclusion`: bake the occlusion a member's surroundings throw on
    /// it into its vertex colours (faces turned down take less sky, faces
    /// turned in toward the structure's core look at its other members).
    pub fn shade_occlusion(&mut self, inward_at: Option<&dyn Fn(f64, f64, f64) -> Option<[f64; 2]>>, down: f64, inward: f64, min_inward_m: f64) {
        for v in (0..self.positions.len()).step_by(3) {
            let mut k = 1.0;
            let ny = self.normals[v + 1];
            if ny < 0.0 {
                k *= 1.0 + (down - 1.0) * (-ny).min(1.0);
            }
            if let Some(f) = inward_at {
                if let Some(dir) = f(self.positions[v], self.positions[v + 2], min_inward_m) {
                    let facing = self.normals[v] * dir[0] + self.normals[v + 2] * dir[1];
                    if facing > 0.0 {
                        k *= 1.0 + (inward - 1.0) * facing.min(1.0);
                    }
                }
            }
            self.colors[v] *= k;
            self.colors[v + 1] *= k;
            self.colors[v + 2] *= k;
        }
    }

    pub fn freeze(&self) -> Frozen {
        let f = |v: &Vec<f64>| v.iter().map(|x| *x as f32).collect::<Vec<f32>>();
        Frozen {
            name: self.name.clone(),
            positions: f(&self.positions),
            normals: f(&self.normals),
            colors: f(&self.colors),
            indices: self.indices.clone(),
            marking_quads: self.marking_quads.clone(),
            pavement_quads: self.pavement_quads.clone(),
            axes: self.wire.then(|| f(&self.axes)),
            dirs: self.wire.then(|| f(&self.dirs)),
            radii: self.wire.then(|| f(&self.radii)),
            min_y: self.min_y,
            max_y: self.max_y,
            sky: None,
        }
    }
}

/// `truePositions`: a wire record's vertices at its members' TRUE radius (the
/// shadow casts the rope, not its drawn envelope); others unchanged.
pub fn true_positions(r: &Frozen, drawn_radius: f64) -> Vec<f32> {
    let (Some(axes), Some(radii)) = (&r.axes, &r.radii) else { return r.positions.clone() };
    let mut out = vec![0f32; r.positions.len()];
    for v in (0..out.len()).step_by(3) {
        let rad = radii[v / 3] as f64;
        let k = if rad > 0.0 { rad / rad.max(drawn_radius) } else { 1.0 };
        for c in 0..3 {
            out[v + c] = (axes[v + c] as f64 + (r.positions[v + c] as f64 - axes[v + c] as f64) * k) as f32;
        }
    }
    out
}

/// A ground grid spec (`ground` in the registry) and, once sampled, its heights.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct GroundGrid {
    pub origin: [f64; 2],
    pub u: [f64; 2],
    pub v: [f64; 2],
    pub cell: [f64; 2],
    pub nu: usize,
    pub nv: usize,
    #[serde(default)]
    pub heights: Vec<f32>,
}

impl GroundGrid {
    /// `gridPoints`: the sample points in row order.
    pub fn points(&self) -> Vec<[f64; 2]> {
        let mut out = Vec::with_capacity(self.nu * self.nv);
        for j in 0..self.nv {
            for i in 0..self.nu {
                out.push([
                    self.origin[0] + self.u[0] * i as f64 * self.cell[0] + self.v[0] * j as f64 * self.cell[1],
                    self.origin[1] + self.u[1] * i as f64 * self.cell[0] + self.v[1] * j as f64 * self.cell[1],
                ]);
            }
        }
        out
    }

    /// `sampleLandmarkGround`: the grid sampled through `height_at` into f32
    /// heights (None, the JS's null outside the elevation grid, reads 0).
    pub fn sampled(&self, height_at: &dyn Fn(f64, f64) -> Option<f64>) -> GroundGrid {
        let heights = self.points().into_iter().map(|[x, z]| height_at(x, z).unwrap_or(0.0) as f32).collect();
        GroundGrid { heights, ..self.clone() }
    }

    /// `gridSampler`: bilinear inside, clamped at the edge.
    pub fn at(&self, x: f64, z: f64) -> f64 {
        let (o, u, v, cell, nu, nv) = (self.origin, self.u, self.v, self.cell, self.nu, self.nv);
        let det = u[0] * v[1] - u[1] * v[0];
        let dx = x - o[0];
        let dz = z - o[1];
        let a = (dx * v[1] - dz * v[0]) / det / cell[0];
        let b = (u[0] * dz - u[1] * dx) / det / cell[1];
        let fi = a.max(0.0).min(nu as f64 - 1.0);
        let fj = b.max(0.0).min(nv as f64 - 1.0);
        let i0 = (fi.floor() as i64).min(if nu < 2 { 0 } else { nu as i64 - 2 }) as usize;
        let j0 = (fj.floor() as i64).min(if nv < 2 { 0 } else { nv as i64 - 2 }) as usize;
        let ti = if nu > 1 { fi - i0 as f64 } else { 0.0 };
        let tj = if nv > 1 { fj - j0 as f64 } else { 0.0 };
        let h = |i: usize, j: usize| self.heights[j.min(nv - 1) * nu + i.min(nu - 1)] as f64;
        let h0 = h(i0, j0) * (1.0 - ti) + h(i0 + 1, j0) * ti;
        let h1 = h(i0, j0 + 1) * (1.0 - ti) + h(i0 + 1, j0 + 1) * ti;
        h0 * (1.0 - tj) + h1 * tj
    }
}

/// Shoelace area in (x, z); this module's outward winding is the negative one.
pub fn signed_area(ring: &[[f64; 2]]) -> f64 {
    let mut a = 0.0;
    for i in 0..ring.len() {
        let [x0, z0] = ring[i];
        let [x1, z1] = ring[(i + 1) % ring.len()];
        a += x0 * z1 - x1 * z0;
    }
    a / 2.0
}

/// The ring in the winding whose side quads face outward.
pub fn orient_ring(ring: &[[f64; 2]]) -> Vec<[f64; 2]> {
    if signed_area(ring) > 0.0 { ring.iter().rev().copied().collect() } else { ring.to_vec() }
}

/// `pairs`: flat [x, z, ...] to [[x, z], ...] without a repeated closing point.
pub fn pairs(flat: &[f64]) -> Vec<[f64; 2]> {
    let mut out: Vec<[f64; 2]> = flat.chunks_exact(2).map(|c| [c[0], c[1]]).collect();
    if out.len() > 2 {
        let (a, b) = (out[0], out[out.len() - 1]);
        if (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-6 {
            out.pop();
        }
    }
    out
}

/// `scaleRing`: a ring scaled about a point.
pub fn scale_ring(ring: &[[f64; 2]], cx: f64, cz: f64, k: f64) -> Vec<[f64; 2]> {
    ring.iter().map(|[x, z]| [cx + (x - cx) * k, cz + (z - cz) * k]).collect()
}

pub fn ring_centroid(ring: &[[f64; 2]]) -> [f64; 2] {
    let (mut x, mut z) = (0.0, 0.0);
    for p in ring {
        x += p[0];
        z += p[1];
    }
    [x / ring.len() as f64, z / ring.len() as f64]
}
