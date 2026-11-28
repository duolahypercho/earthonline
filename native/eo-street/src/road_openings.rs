//! Clip faces against the drivable space swept by a road through a structure.
//! Clipping uses all three dimensions so crossings above the opening survive.
use std::collections::HashMap;

type Point = [f64; 3];
type Plane = [f64; 4];
type Bounds = [Point; 2];

fn bounds(points: &[Point]) -> Bounds {
    [
        std::array::from_fn(|k| points.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min)),
        std::array::from_fn(|k| {
            points
                .iter()
                .map(|p| p[k])
                .fold(f64::NEG_INFINITY, f64::max)
        }),
    ]
}

fn cells(b: Bounds) -> impl Iterator<Item = (i32, i32)> {
    let cell = |v: f64| (v / 32.0).floor() as i32;
    (cell(b[0][0])..=cell(b[1][0]))
        .flat_map(move |x| (cell(b[0][2])..=cell(b[1][2])).map(move |z| (x, z)))
}

#[derive(Debug)]
struct Opening {
    road: u32,
    planes: [Plane; 6],
    bounds: Bounds,
    below_surface: bool,
}

#[derive(Debug, Default)]
pub struct RoadOpenings {
    cuts: Vec<Opening>,
    grid: HashMap<(i32, i32), Vec<usize>>,
}

/// A point of the surviving face, with barycentric weights for its vertex attributes.
#[derive(Clone, Copy)]
pub struct Vertex {
    pub pos: Point,
    weights: [f64; 3],
}

impl Vertex {
    pub fn interpolate<const N: usize>(&self, values: [[f32; N]; 3]) -> [f32; N] {
        std::array::from_fn(|i| {
            (0..3)
                .map(|j| self.weights[j] * f64::from(values[j][i]))
                .sum::<f64>() as f32
        })
    }

    /// Round a split on an existing mesh edge to the containing side of
    /// that face. At city-coordinate magnitudes an f32 intersection can
    /// otherwise pull an edge away from its unsplit adjoining triangle.
    pub(crate) fn mesh_position(&self, source: [[f32; 3]; 3]) -> [f32; 3] {
        let mut position = self.pos.map(|x| x as f32);
        let Some(i) = self.weights.iter().position(|&w| w == 0.0) else {
            return position;
        };
        if self.weights[(i + 1) % 3] == 0.0 || self.weights[(i + 2) % 3] == 0.0 {
            return position;
        }
        let a = source[(i + 1) % 3].map(f64::from);
        let b = source[(i + 2) % 3].map(f64::from);
        let c = source[i].map(f64::from);
        let n = [a[2] - b[2], b[0] - a[0]];
        let side = n[0] * (c[0] - a[0]) + n[1] * (c[2] - a[2]);
        if side == 0.0 {
            return position;
        }
        let n = n.map(|v| v * side.signum());
        if n[0] * (f64::from(position[0]) - a[0]) + n[1] * (f64::from(position[2]) - a[2]) > 0.0 {
            let step = |v: f32, normal: f64| {
                if normal > 0.0 {
                    v.next_down()
                } else {
                    v.next_up()
                }
            };
            let x = step(position[0], n[0]);
            let z = step(position[2], n[1]);
            if (f64::from(x - position[0]) * n[0]).abs()
                >= (f64::from(z - position[2]) * n[1]).abs()
            {
                position[0] = x;
            } else {
                position[2] = z;
            }
        }
        position
    }

    pub fn color(&self, colors: [u32; 3]) -> u32 {
        let channels = colors.map(|c| c.to_le_bytes().map(f32::from));
        u32::from_le_bytes(self.interpolate(channels).map(|k| k.round() as u8))
    }
}

impl RoadOpenings {
    /// The left and right ends of two adjacent cross-sections. The lower
    /// and upper planes follow the longitudinal road grade between them.
    pub fn add(&mut self, road: u32, a: [Point; 2], b: [Point; 2], below: f64, above: f64) {
        let ring = [a[0], a[1], b[1], b[0]];
        let centre =
            |p: [Point; 2]| -> Point { std::array::from_fn(|i| (p[0][i] + p[1][i]) * 0.5) };
        let (start, end) = (centre(a), centre(b));
        let (dx, dz) = (end[0] - start[0], end[2] - start[2]);
        let len2 = dx * dx + dz * dz;
        if len2 < 1e-8 {
            return;
        }
        let slope = (end[1] - start[1]) / len2;
        let base = start[1] - slope * (dx * start[0] + dz * start[2]);
        let mut planes = [[0.0; 4]; 6];
        for i in 0..4 {
            let (p, q) = (ring[i], ring[(i + 1) % 4]);
            let (nx, nz) = (p[2] - q[2], q[0] - p[0]);
            let off = -(nx * p[0] + nz * p[2]);
            let mid = (start[0] + end[0]) * nx * 0.5 + (start[2] + end[2]) * nz * 0.5 + off;
            let factor = mid.signum() / nx.hypot(nz).max(f64::EPSILON);
            planes[i] = [nx * factor, 0.0, nz * factor, off * factor];
        }
        planes[4] = [-slope * dx, 1.0, -slope * dz, below - base];
        planes[5] = [slope * dx, -1.0, slope * dz, base + above];
        let mut box3 = bounds(&ring);
        for p in &ring {
            let y = base + slope * (dx * p[0] + dz * p[2]);
            box3[0][1] = box3[0][1].min(y - below);
            box3[1][1] = box3[1][1].max(y + above);
        }
        self.insert(road, planes, box3, false);
    }

    /// The volume between two translated copies of a drawn triangle. Its
    /// lower face tracks both the transverse camber and the longitudinal grade.
    pub(crate) fn add_surface(&mut self, tri: [[f32; 3]; 3], below: f64, above: f64) {
        self.add_paving_face(tri, below, above, false);
    }

    pub(crate) fn add_paving_face(
        &mut self,
        tri: [[f32; 3]; 3],
        below: f64,
        above: f64,
        trim_earth: bool,
    ) {
        self.insert_surface(0, tri, below, [above; 3], trim_earth);
    }

    /// Only the part of the junction pad that also lies inside the mapped
    /// approach carries its through traffic. Preserve corners outside either.
    pub(crate) fn add_pad_approach(
        &mut self,
        road: u32,
        tri: [[f32; 3]; 3],
        approach: &Self,
        depth: f64,
    ) {
        let points = tri.map(|p| p.map(f64::from));
        let triangle = std::array::from_fn::<_, 3, _>(|i| Vertex {
            pos: points[i],
            weights: std::array::from_fn(|j| f64::from(i == j)),
        });
        for cut in approach.candidates(bounds(&points)) {
            if !intersects(&triangle, &cut.planes) {
                continue;
            }
            if let Some(part) = subtract(triangle.to_vec(), &cut.planes, &mut Vec::new()) {
                for i in 2..part.len() {
                    let face = [part[0], part[i - 1], part[i]].map(|v| v.pos.map(|p| p as f32));
                    self.insert_surface(road, face, depth, [depth; 3], false);
                }
            }
        }
    }

    /// A traffic opening between a graded road triangle and an independently
    /// sloping roof. Heights are relative to the corresponding road vertex.
    pub fn add_graded_roof(&mut self, road: u32, tri: [[f32; 3]; 3], below: f64, heights: [f64; 3]) {
        self.insert_surface(road, tri, below, heights, false);
    }

    fn insert_surface(
        &mut self,
        road: u32,
        tri: [[f32; 3]; 3],
        below: f64,
        heights: [f64; 3],
        trim_earth: bool,
    ) {
        let p = tri.map(|p| p.map(f64::from));
        let [a, b, c] = p;
        let (bx, bz, cx, cz) = (b[0] - a[0], b[2] - a[2], c[0] - a[0], c[2] - a[2]);
        let det = bx * cz - bz * cx;
        if det.abs() <= 1e-9 {
            return;
        }
        let mut planes = [[0.; 4]; 6];
        for i in 0..3 {
            let (u, v) = (p[i], p[(i + 1) % 3]);
            let (dx, dz) = (v[0] - u[0], v[2] - u[2]);
            let k = det.signum() / dx.hypot(dz);
            planes[i] = [-dz * k, 0., dx * k, (dz * u[0] - dx * u[2]) * k];
        }
        // Round an earth cut outwards by its mesh coordinates' precision.
        // A submillimetre strip rounded into the paving can otherwise stand
        // above the footway by the whole height of the bank.
        let ulp = |k: usize| {
            if trim_earth {
                tri.iter()
                    .map(|v| f64::from(v[k].abs().next_up() - v[k].abs()))
                    .fold(0.0, f64::max)
            } else {
                0.0
            }
        };
        let margin = [ulp(0), ulp(2)];
        for p in &mut planes[..3] {
            p[3] += p[0].abs() * margin[0] + p[2].abs() * margin[1];
        }
        planes[3] = planes[2];
        let gx = ((b[1] - a[1]) * cz - (c[1] - a[1]) * bz) / det;
        let gz = (bx * (c[1] - a[1]) - cx * (b[1] - a[1])) / det;
        let intercept = a[1] - gx * a[0] - gz * a[2];
        planes[4] = [-gx, 1., -gz, below - intercept];
        let [ha, hb, hc] = heights;
        let hx = ((hb - ha) * cz - (hc - ha) * bz) / det;
        let hz = (bx * (hc - ha) - cx * (hb - ha)) / det;
        planes[5] = [gx + hx, -1., gz + hz, ha + intercept - hx * a[0] - hz * a[2]];
        let mut ext = bounds(&p);
        for (k, d) in [(0, margin[0]), (2, margin[1])] {
            ext[0][k] -= d;
            ext[1][k] += d;
        }
        ext[0][1] -= below;
        ext[1][1] = (0..3).map(|i| p[i][1] + heights[i]).fold(f64::NEG_INFINITY, f64::max);
        self.insert(road, planes, ext, heights.iter().all(|&h| h < 0.));
    }

    fn insert(&mut self, road: u32, planes: [Plane; 6], bounds: Bounds, below_surface: bool) {
        let index = self.cuts.len();
        self.cuts.push(Opening {
            road,
            planes,
            bounds,
            below_surface,
        });
        for key in cells(bounds) {
            self.grid.entry(key).or_default().push(index);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.cuts.is_empty()
    }

    pub fn intersects_bounds(&self, lo: [f32; 3], hi: [f32; 3]) -> bool {
        !self.is_empty()
            && cells([lo.map(f64::from), hi.map(f64::from)]).any(|k| self.grid.contains_key(&k))
    }

    /// A world-space box touches the bounds of a road clearance volume in all three axes.
    pub fn overlaps_bounds(&self, lo: Point, hi: Point) -> bool {
        self.candidates([lo, hi]).next().is_some()
    }

    /// True when some area of a face actually enters the swept traffic space.
    /// The bounds check alone would also remove hillside faces beside a diagonal road.
    pub fn obstructed_by(&self, points: [Point; 3]) -> bool {
        let vertices = points.map(|pos| Vertex { pos, weights: [0.; 3] });
        self.candidates(bounds(&points)).any(|opening| {
            intersects(&vertices, &opening.planes)
                && subtract(vertices.to_vec(), &opening.planes, &mut Vec::new()).is_some()
        })
    }

    pub(crate) fn intersects_other(
        &self,
        lo: [f64; 3],
        hi: [f64; 3],
        owner: Option<usize>,
    ) -> bool {
        self.candidates([lo, hi])
            .any(|opening| Some(opening.road as usize) != owner)
    }

    /// None preserves the original triangle and its vertices verbatim.
    pub fn clip(&self, points: [[f32; 3]; 3]) -> Option<Vec<[Vertex; 3]>> {
        self.clip_where(points, |_| true)
    }

    /// Only the selected roads can open this face. A shared tunnel fork
    /// opens its lining to the adjoining road and retains its outside walls.
    pub fn clip_where(
        &self,
        points: [[f32; 3]; 3],
        accepts: impl Fn(u32) -> bool,
    ) -> Option<Vec<[Vertex; 3]>> {
        self.clip_covered(points, accepts, None)
    }

    /// Keep the shoulder supporting an immediately overlying road. At paved
    /// overlaps only earth protruding through the highest road is visible.
    pub(crate) fn clip_covered(
        &self,
        points: [[f32; 3]; 3],
        accepts: impl Fn(u32) -> bool,
        cover: Option<&Self>,
    ) -> Option<Vec<[Vertex; 3]>> {
        self.clip_from_base(points, accepts, cover, 0.0)
    }

    pub(crate) fn clip_from_base(
        &self, points: [[f32; 3]; 3], accepts: impl Fn(u32) -> bool, cover: Option<&Self>, bottom_inset: f64,
    ) -> Option<Vec<[Vertex; 3]>> {
        let points = points.map(|p| p.map(f64::from));
        let extent = bounds(&points);
        let triangle = std::array::from_fn::<_, 3, _>(|i| Vertex {
            pos: points[i],
            weights: std::array::from_fn(|j| f64::from(i == j)),
        });
        let mut pieces: Option<Vec<Vec<Vertex>>> = None;
        let mut buried = vec![];
        for cut in self.candidates(extent) {
            let mut planes = cut.planes;
            planes[4][3] -= bottom_inset;
            if cut.below_surface {
                planes[5][3] -= seam_roundoff(points, planes[5]);
            }
            if !accepts(cut.road) || !intersects(&triangle, &planes) {
                continue;
            }
            let mut out = vec![];
            for piece in pieces.take().unwrap_or_else(|| vec![triangle.to_vec()]) {
                if !intersects(&piece, &planes) {
                    out.push(piece);
                    continue;
                }
                if let Some(removed) = subtract(piece, &planes, &mut out) {
                    if let Some(cover) = cover {
                        let mut exposed = vec![removed];
                        for shelter in cover.candidates(extent) {
                            let mut next = vec![];
                            for part in exposed {
                                if intersects(&part, &shelter.planes) {
                                    if let Some(under) = subtract(part, &shelter.planes, &mut next)
                                    {
                                        buried.push(under)
                                    }
                                } else {
                                    next.push(part)
                                }
                            }
                            exposed = next;
                            if exposed.is_empty() {
                                break;
                            }
                        }
                    }
                }
            }
            pieces = Some(out);
            if pieces.as_ref().is_some_and(Vec::is_empty) {
                break;
            }
        }
        pieces.map(|mut polys| {
            polys.extend(buried);
            polys
                .into_iter()
                .flat_map(|poly| (2..poly.len()).map(move |i| [poly[0], poly[i - 1], poly[i]]))
                .collect()
        })
    }

    fn candidates(&self, extent: Bounds) -> impl Iterator<Item = &Opening> {
        let mut indices = vec![];
        for key in cells(extent) {
            if let Some(ids) = self.grid.get(&key) {
                indices.extend_from_slice(ids)
            }
        }
        indices.sort_unstable();
        indices.dedup();
        indices
            .into_iter()
            .map(|i| &self.cuts[i])
            .filter(move |cut| {
                (0..3).all(|k| extent[1][k] >= cut.bounds[0][k] && extent[0][k] <= cut.bounds[1][k])
            })
    }
}

/// Two roads cut at the same height difference emit their shared edge
/// independently into f32 meshes. Keep enough of the buried road to cover
/// the rounding of both edges. Project horizontal rounding onto the actual
/// difference in road grades so the allowance follows the slope at the seam.
fn seam_roundoff(points: [Point; 3], upper: Plane) -> f64 {
    let [a, b, c] = points;
    let (bx, bz, cx, cz) = (b[0] - a[0], b[2] - a[2], c[0] - a[0], c[2] - a[2]);
    let det = bx * cz - bz * cx;
    if det.abs() <= 1e-9 { return 0.0; }
    let height = |p: Point| upper[0] * p[0] - p[1] + upper[2] * p[2];
    let db = height(b) - height(a);
    let dc = height(c) - height(a);
    let dx = (db * cz - dc * bz) / det;
    let dz = (bx * dc - cx * db) / det;
    let unit = |axis: usize| {
        let largest = points.iter().map(|p| p[axis].abs() as f32).fold(0.0, f32::max);
        f64::from(largest.next_up() - largest)
    };
    2.0 * (dx.abs() * unit(0) + unit(1) + dz.abs() * unit(2))
}

fn distance(v: &Vertex, p: Plane) -> f64 {
    v.pos[0] * p[0] + v.pos[1] * p[1] + v.pos[2] * p[2] + p[3]
}

fn intersects(poly: &[Vertex], planes: &[Plane]) -> bool {
    planes
        .iter()
        .all(|p| poly.iter().any(|v| distance(v, *p) > 1e-5))
}

fn subtract(
    mut poly: Vec<Vertex>,
    planes: &[Plane],
    out: &mut Vec<Vec<Vertex>>,
) -> Option<Vec<Vertex>> {
    let start = out.len();
    let original = poly.clone();
    for &p in planes {
        if poly.iter().all(|v| distance(v, p) >= -1e-8) {
            continue;
        }
        if poly.iter().all(|v| distance(v, p) <= 0.0) {
            // The clipped polygon missed the finite opening. Splitting the
            // original at an extended side plane would introduce T-junctions
            // at unrelated street edges when vertices are rounded for the GPU.
            out.truncate(start);
            out.push(original);
            return None;
        }
        let (mut inside, mut outside) = (vec![], vec![]);
        let mut prev = *poly.last().unwrap();
        for v in poly {
            let (a, b) = (distance(&prev, p), distance(&v, p));
            if a * b < 0.0 {
                let t = a / (a - b);
                let mix = |x: Point, y: Point| std::array::from_fn(|k| x[k] + t * (y[k] - x[k]));
                let cross = Vertex {
                    pos: mix(prev.pos, v.pos),
                    weights: mix(prev.weights, v.weights),
                };
                inside.push(cross);
                outside.push(cross);
            }
            if b >= 0.0 {
                inside.push(v);
            }
            if b <= 0.0 {
                outside.push(v);
            }
            prev = v;
        }
        if outside.len() >= 3 {
            out.push(outside);
        }
        if inside.len() < 3 {
            out.truncate(start);
            out.push(original);
            return None;
        }
        poly = inside;
    }
    Some(poly)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clipped_street_covers_its_original_edge_at_city_coordinates() {
        let source = [
            [-7855.9, 40.0, 6249.9],
            [-7853.1, 41.0, 6248.2],
            [-7851.4, 40.4, 6250.1],
        ];
        let [a, b, c] = source.map(|p| p.map(f64::from));
        let side = |p: [f64; 3]| (b[0] - a[0]) * (p[2] - a[2]) - (b[2] - a[2]) * (p[0] - a[0]);
        for t in [0.25f64, 0.45, 0.7] {
            let v = Vertex {
                pos: std::array::from_fn(|k| a[k] + t * (b[k] - a[k])),
                weights: [1.0 - t, t, 0.0],
            };
            assert!(side(v.mesh_position(source).map(f64::from)) * side(c).signum() <= 1e-9);
        }
    }

    #[test]
    fn a_vault_panel_keeps_the_road_below_and_the_rock_above_its_slope() {
        let mut openings = RoadOpenings::default();
        openings.add_graded_roof(1, [[0., 4., 0.], [4., 4., 0.], [0., 5., 8.]], 0., [5., 2., 5.]);
        let face = |x, y| [[x, y, 0.2], [x + 0.1, y, 0.2], [x, y, 0.3]];
        assert!(openings.obstructed_by(face(0.1, 8.)));
        assert!(!openings.obstructed_by(face(3.5, 8.)));
        assert!(!openings.obstructed_by(face(0.1, 3.)));
    }

    #[test]
    fn rock_faces_must_enter_the_finite_graded_clearance() {
        let mut openings = RoadOpenings::default();
        openings.add(7, [[0., 0., -2.], [0., 0., 2.]], [[10., 1., -2.], [10., 1., 2.]], 0., 4.);
        // A tall near-vertical face extends through the lane from roots on the hillside.
        assert!(openings.obstructed_by([[4., -1., -4.], [4., 8., -4.], [4., 2., 3.]]));
        // The bounds of this diagonal face overlap the end corner; the face itself misses.
        let beside = [[9., 2., 4.], [11., 2., 2.], [11., 3., 2.]];
        assert!(openings.overlaps_bounds([9., 2., 2.], [11., 3., 4.]));
        assert!(!openings.obstructed_by(beside));
        assert!(!openings.obstructed_by([[2., 6., 0.], [7., 6., 1.], [2., 7., 2.]]));
        assert!(!openings.obstructed_by([[2., -1., 0.], [7., -1., 1.], [2., -2., 2.]]));
    }

    #[test]
    fn an_opening_follows_the_road_grade_and_keeps_the_wall_outside_it() {
        let mut holes = RoadOpenings::default();
        holes.add(
            7,
            [[0., 0., -2.], [0., 0., 2.]],
            [[10., 1., -2.], [10., 1., 2.]],
            0.,
            4.,
        );
        // The floor and a parallel elevated crossing must both survive.
        assert!(
            holes
                .clip([[0., 0., -1.], [10., 1., -1.], [10., 1., 1.]])
                .is_none()
        );
        assert!(
            holes
                .clip([[0., 6., -1.], [10., 6., -1.], [10., 6., 1.]])
                .is_none()
        );
        let wall = [[5., 1., -3.], [5., 3., 0.], [5., 1., 3.]];
        assert!(holes.clip_where(wall, |road| road != 7).is_none());
        let pieces = holes.clip(wall).unwrap();
        assert_eq!(pieces.len(), 2);
        for tri in pieces {
            assert!(tri.iter().all(|v| v.pos[2] <= -2.) || tri.iter().all(|v| v.pos[2] >= 2.));
            for v in tri {
                let reconstructed = v.interpolate(wall);
                assert!(
                    v.pos
                        .iter()
                        .zip(reconstructed)
                        .all(|(a, b)| (a - f64::from(b)).abs() < 1e-5)
                );
            }
        }
    }
}
