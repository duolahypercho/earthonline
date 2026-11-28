//! Triangulate a junction boundary that the node cannot see as a single fan.

type Point = [f64; 2];
type Vertex = [f64; 3];

const AREA_EPS: f64 = 1e-9;
// Only pads with nearly vertical facets need this extra relief pass.
const STEEP_FACET_GRADE: f64 = 2.0;
const RELIEF_FACET_GRADE: f64 = 0.5;
const GRADE_EPS: f64 = 1e-6;

fn cross(a: Point, b: Point, c: Point) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

// Both pavement and the traffic exporter store these points as f32.
fn planar(v: Vertex) -> Point {
    [f64::from(v[0] as f32), f64::from(v[2] as f32)]
}

fn half_ulp(x: f64) -> f64 {
    let v = x.abs() as f32;
    0.5 * f64::from(v.next_up() - v)
}

fn edges(points: &[Point]) -> impl Iterator<Item = (Point, Point)> + '_ {
    points.iter().copied().zip(points.iter().copied().cycle().skip(1))
}

fn meet(a: Point, b: Point, c: Point, d: Point) -> bool {
    let spans_overlap = (0..2).all(|i| a[i].min(b[i]) <= c[i].max(d[i]) && c[i].min(d[i]) <= a[i].max(b[i]));
    if !spans_overlap {
        return false;
    }
    let straddles = |u: f64, v: f64| u.min(v) <= AREA_EPS && u.max(v) >= -AREA_EPS;
    straddles(cross(a, b, c), cross(a, b, d)) && straddles(cross(c, d, a), cross(c, d, b))
}

pub(crate) fn faces(rim_len: usize, triangles: &[[usize; 3]]) -> impl Iterator<Item = [usize; 3]> + '_ {
    let count = if triangles.is_empty() { rim_len } else { triangles.len() };
    (0..count).map(move |i| triangles.get(i).copied().unwrap_or_else(|| [0, i + 1, (i + 1) % rim_len + 1]))
}

fn inside([x, z]: Point, rim: &[Point]) -> bool {
    edges(rim).filter(|&(a, b)| {
        (a[1] > z) != (b[1] > z) && x < a[0] + (b[0] - a[0]) * (z - a[1]) / (b[1] - a[1])
    }).count() % 2 == 1
}

fn covers(p: Point, rim: &[Point]) -> bool {
    inside(p, rim) || edges(rim).any(|(a, b)| {
        // Exact mouth sections can round to either side of their split edge.
        let error = [0, 1].map(|k| 2.0 * half_ulp(p[k].abs().max(a[k].abs()).max(b[k].abs())));
        (0..2).all(|k| p[k] >= a[k].min(b[k]) - error[k] && p[k] <= a[k].max(b[k]) + error[k])
            && cross(a, b, p).abs() <= AREA_EPS + error[0] * (b[1] - a[1]).abs() + error[1] * (b[0] - a[0]).abs()
    })
}

fn first_fold(points: &[Point]) -> Option<(usize, usize, f64, f64, Point)> {
    for (i, (a, b)) in edges(points).enumerate() {
        for (j, (c, d)) in edges(points).enumerate().skip(i + 2) {
            if i == 0 && j + 1 == points.len() {
                continue;
            }
            let u = [b[0] - a[0], b[1] - a[1]];
            let v = [d[0] - c[0], d[1] - c[1]];
            let w = [c[0] - a[0], c[1] - a[1]];
            let origin = [0.0, 0.0];
            let div = cross(origin, u, v);
            if div.abs() <= AREA_EPS {
                continue;
            }
            let t = cross(origin, w, v) / div;
            let s = cross(origin, w, u) / div;
            if t > 0.0 && t < 1.0 && s > 0.0 && s < 1.0 {
                return Some((i, j, t, s, [a[0] + t * u[0], a[1] + t * u[1]]));
            }
        }
    }
    for (j, &p) in points.iter().enumerate() {
        if let Some(i) = points[..j].iter().position(|&q| q == p) {
            return Some((i, j, 0.0, 0.0, p));
        }
    }
    None
}

fn dedup_ring(rim: &mut Vec<Vertex>) {
    rim.dedup_by_key(|v| planar(*v));
    if rim.len() > 1 && planar(rim[0]) == planar(*rim.last().unwrap()) {
        rim.pop();
    }
}

/// Follow the loop containing the node through each fold. Commit the
/// resulting ring once it triangulates and retains every approach mouth.
pub(super) fn surface(center: Vertex, rim: &mut Vec<Vertex>, mouths: impl FnOnce() -> Vec<Point>) -> Vec<[usize; 3]> {
    if let Some(faces) = triangulate(center, rim) {
        return faces;
    }
    let mut ring = rim.clone();
    dedup_ring(&mut ring);
    let mut changed = false;
    while let Some((i, j, t, s, [x, z])) = first_fold(&ring.iter().copied().map(planar).collect::<Vec<_>>()) {
        let height = |k, t| {
            let (a, b): (Vertex, Vertex) = (ring[k], ring[(k + 1) % ring.len()]);
            a[1] + (b[1] - a[1]) * t
        };
        let joint = [x, height(i, t).midpoint(height(j, s)), z];
        let mut first = ring[..=i].to_vec();
        first.push(joint);
        first.extend_from_slice(&ring[j + 1..]);
        let mut second = vec![joint];
        second.extend_from_slice(&ring[i + 1..=j]);
        let contains_node = |loop_rim: &[Vertex]| inside(planar(center), &loop_rim.iter().copied().map(planar).collect::<Vec<_>>());
        ring = match (contains_node(&first), contains_node(&second)) {
            (true, false) => first,
            (false, true) => second,
            _ => return Vec::new(),
        };
        dedup_ring(&mut ring);
        if ring.len() < 3 {
            return Vec::new();
        }
        changed = true;
    }
    if changed {
        let boundary: Vec<_> = ring.iter().copied().map(planar).collect();
        if mouths().into_iter().all(|p| covers(planar([p[0], 0.0, p[1]]), &boundary))
            && let Some(faces) = triangulate(center, &ring)
        {
            *rim = ring;
            return faces;
        }
    }
    Vec::new()
}

fn clip_ears(points: &[Point], mut ring: Vec<usize>, faces: &mut Vec<[usize; 3]>) -> bool {
    while ring.len() > 3 {
        let mut ear = None;
        for (i, &b) in ring.iter().enumerate() {
            let a = ring[(i + ring.len() - 1) % ring.len()];
            let c = ring[(i + 1) % ring.len()];
            let [p, q, r] = [points[a], points[b], points[c]];
            if cross(p, q, r) <= AREA_EPS {
                continue;
            }
            let diagonal = (p[0] - r[0]).powi(2) + (p[1] - r[1]).powi(2);
            if ear.is_some_and(|(_, _, shortest)| diagonal >= shortest) {
                continue;
            }
            if ring.iter().any(|&i| {
                let v = points[i];
                i != a && i != b && i != c
                    && cross(p, q, v) >= -AREA_EPS && cross(q, r, v) >= -AREA_EPS && cross(r, p, v) >= -AREA_EPS
            }) {
                continue;
            }
            ear = Some((i, [a, b, c], diagonal));
        }
        let Some((i, triangle, _)) = ear else { return false };
        faces.push(triangle);
        ring.remove(i);
    }
    let last = [ring[0], ring[1], ring[2]];
    if cross(points[last[0]], points[last[1]], points[last[2]]) <= AREA_EPS {
        return false;
    }
    faces.push(last);
    true
}

fn from_visible_edges(points: &[Point], area: f64, faces: &mut Vec<[usize; 3]>) -> bool {
    let n = points.len() - 1;
    let (rim, center) = (&points[..n], points[n]);
    if !inside(center, rim) {
        return false;
    }
    // Valid radial edges preserve the original grade wherever the node
    // already sees the road boundary. Fill each pocket independently.
    let visible: Vec<_> = rim.iter().enumerate().filter_map(|(i, &p)| {
        let middle = [center[0].midpoint(p[0]), center[1].midpoint(p[1])];
        let crosses_edge = edges(rim).enumerate().any(|(j, (a, b))| {
            j != i && (j + 1) % n != i && meet(center, p, a, b)
        });
        (!crosses_edge && inside(middle, rim)).then_some(i)
    }).collect();
    if visible.len() < 2 {
        return false;
    }
    for (&a, &b) in visible.iter().zip(visible.iter().cycle().skip(1)) {
        let length = (b + n - a) % n;
        let ring = std::iter::once(n).chain((0..=length).map(|i| (a + i) % n)).collect();
        if !clip_ears(points, ring, faces) {
            return false;
        }
    }
    let covered: f64 = faces.iter().map(|&[a, b, c]| cross(points[a], points[b], points[c])).sum();
    (covered - area).abs() <= AREA_EPS * area.max(1.0)
}

fn grade(points: &[Point], heights: &[f64], [a, b, c]: [usize; 3]) -> f64 {
    let [p, q, r] = [points[a], points[b], points[c]];
    let [bx, bz] = [q[0] - p[0], q[1] - p[1]];
    let [cx, cz] = [r[0] - p[0], r[1] - p[1]];
    let [by, cy] = [heights[b] - heights[a], heights[c] - heights[a]];
    (by * cz - cy * bz).hypot(bx * cy - cx * by) / cross(p, q, r).abs()
}

fn relieve_steep_facets(points: &[Point], heights: &[f64], faces: &mut [[usize; 3]]) {
    let mut grades: Vec<_> = faces.iter().map(|&face| grade(points, heights, face)).collect();
    if grades.iter().all(|&g| g < STEEP_FACET_GRADE) {
        return;
    }
    let mut edges = std::collections::HashMap::with_capacity(faces.len() * 2);
    loop {
        edges.clear();
        let mut best = None;
        let mut gain = GRADE_EPS;
        for (i, &[a, b, c]) in faces.iter().enumerate() {
            for [a, b, c] in [[a, b, c], [b, c, a], [c, a, b]] {
                let Some(&(j, d)) = edges.get(&(b, a)) else {
                    edges.insert((a, b), (i, c));
                    continue;
                };
                let previous = grades[i].max(grades[j]);
                // Continue spreading the rise after the first flip. A
                // larger replacement face can still form a sharp ridge.
                if previous < RELIEF_FACET_GRADE {
                    continue;
                }
                // The alternate diagonal must stay inside the two old faces.
                let alternate = [[c, a, d], [d, b, c]];
                if alternate.iter().any(|&[a, b, c]| cross(points[a], points[b], points[c]) <= AREA_EPS) {
                    continue;
                }
                let next = alternate.map(|face| grade(points, heights, face));
                let reduction = previous - next[0].max(next[1]);
                if reduction > gain {
                    gain = reduction;
                    best = Some(([i, j], alternate, next));
                }
            }
        }
        let Some(([i, j], [a, b], [ga, gb])) = best else { break };
        (faces[i], faces[j]) = (a, b);
        (grades[i], grades[j]) = (ga, gb);
    }
}

/// A simple concave rim needs real diagonals when some fan wedges point
/// outside it. Preserve the boundary heights and the node where it lies
/// in the interior. An empty face list denotes a valid radial fan.
fn triangulate(center: Vertex, rim: &[Vertex]) -> Option<Vec<[usize; 3]>> {
    let center_y = f64::from(center[1] as f32);
    let center = planar(center);
    let (mut forward, mut backward) = (false, false);
    for (&a, &b) in rim.iter().zip(rim.iter().cycle().skip(1)) {
        let (a, b) = (planar(a), planar(b));
        let area = cross(center, a, b);
        if (area >= 0. && forward) || (area <= 0. && backward) || area.abs() <= AREA_EPS {
            continue;
        }
        // A boundary running radially into the node can round to alternating
        // sides of the ray. Count a reversed wedge only when its orientation
        // survives the rounding of the three stored vertex positions.
        let bound = |p: Point, q: Point, r: Point| {
            half_ulp(p[0]) * (q[1] - r[1]).abs() + half_ulp(p[1]) * (q[0] - r[0]).abs()
        };
        let dx = half_ulp(center[0].abs().max(a[0].abs()).max(b[0].abs()));
        let dz = half_ulp(center[1].abs().max(a[1].abs()).max(b[1].abs()));
        let uncertainty = AREA_EPS + bound(center, a, b) + bound(a, b, center) + bound(b, center, a) + 8. * dx * dz;
        forward |= area > uncertainty;
        backward |= area < -uncertainty;
        if forward && backward { break; }
    }
    if !forward || !backward {
        return Some(Vec::new());
    }
    let mut polygon = Vec::with_capacity(rim.len());
    for (i, &vertex) in rim.iter().enumerate() {
        let point = planar(vertex);
        if polygon.last().is_none_or(|&(_, last)| last != point) {
            polygon.push((i + 1, point));
        }
    }
    if polygon.len() > 1 && polygon[0].1 == polygon.last().unwrap().1 {
        polygon.pop();
    }
    let mut points: Vec<_> = polygon.iter().map(|&(_, p)| p).collect();
    if points.len() < 3 {
        return None;
    }
    let area: f64 = edges(&points).map(|(a, b)| cross(points[0], a, b)).sum();
    if area.abs() <= AREA_EPS {
        return None;
    }
    for (i, (a, b)) in edges(&points).enumerate() {
        for (j, (c, d)) in edges(&points).enumerate().skip(i + 2) {
            if (i != 0 || j + 1 != points.len()) && meet(a, b, c, d) {
                return None;
            }
        }
    }
    if edges(&points).all(|(a, b)| area.signum() * cross(center, a, b) >= -AREA_EPS) {
        return Some(Vec::new());
    }
    if area < 0.0 {
        polygon.reverse();
        points.reverse();
    }
    let node = points.len();
    points.push(center);
    let mut faces = Vec::with_capacity(points.len());
    if !from_visible_edges(&points, area.abs(), &mut faces) {
        faces.clear();
        if !clip_ears(&points, (0..node).collect(), &mut faces) {
            return None;
        }
        if let Some(i) = faces.iter().position(|&[a, b, c]| {
            cross(points[a], points[b], center) > AREA_EPS
                && cross(points[b], points[c], center) > AREA_EPS
                && cross(points[c], points[a], center) > AREA_EPS
        }) {
            let [a, b, c] = faces[i];
            faces[i] = [a, b, node];
            faces.extend([[b, c, node], [c, a, node]]);
        }
    }
    let heights: Vec<_> = polygon.iter().map(|&(i, _)| f64::from(rim[i - 1][1] as f32))
        .chain(std::iter::once(center_y)).collect();
    relieve_steep_facets(&points, &heights, &mut faces);
    Some(faces.into_iter().map(|face| face.map(|i| if i == node { 0 } else { polygon[i].0 })).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_better_diagonal_spreads_grade_away_from_a_nearly_vertical_facet() {
        let points = [[0., 0.], [3., -0.02], [4., 0.02], [1.5, 3.]];
        let heights = [3., 0., 0., 0.5];
        let mut faces = [[0, 1, 2], [0, 2, 3]];
        assert!(grade(&points, &heights, faces[0]) > STEEP_FACET_GRADE);
        relieve_steep_facets(&points, &heights, &mut faces);
        assert_eq!(faces, [[1, 2, 3], [3, 0, 1]]);
        assert!(faces.iter().all(|&face| grade(&points, &heights, face) < STEEP_FACET_GRADE));
    }

    #[test]
    fn concave_junction_triangles_stay_inside_the_road_boundary() {
        let rim = [[-4., 0., -4.], [4., 1., -4.], [4., 1., 4.], [1., 1., 4.],
            [1., 0., -1.], [-1., 0., -1.], [-1., 0., 4.], [-4., 0., 4.]];
        let center = [-3., 0., 2.];
        let faces = triangulate(center, &rim).unwrap();
        assert!(!faces.is_empty());
        assert!(faces.contains(&[0, 8, 1]), "preserve the grade along the visible west edge");
        let vertex = |i| {
            let [x, _, z] = if i == 0 { center } else { rim[i - 1] };
            [x, z]
        };
        let mut twice_area = 0.0;
        for face in faces {
            let [a, b, c] = face.map(vertex);
            let area = cross(a, b, c);
            assert!(area > 0.0);
            twice_area += area;
            let notch = [0., 2.];
            assert!(cross(a, b, notch) < 0.0 || cross(b, c, notch) < 0.0 || cross(c, a, notch) < 0.0);
        }
        assert_eq!(twice_area, 108.0);
        assert!(triangulate([0., 0., 0.], &rim[..4]).unwrap().is_empty());
    }

    #[test]
    fn a_folded_rim_keeps_connections_at_its_road_mouths() {
        let original = vec![[-4., 0., -4.], [4., 0., -4.], [4., 0., 4.], [1., 0., 4.],
            [1., 0., 6.], [2., 0., 6.], [2., 0., 2.], [-4., 0., 2.]];
        let center = [0., 0., 0.];
        let mut rim = original.clone();
        surface(center, &mut rim, || vec![[4., 0.], [0., -4.]]);
        assert!(rim.len() < original.len(), "the fold should be removed");
        assert!(triangulate(center, &rim).is_some());

        let mut rim = original.clone();
        surface(center, &mut rim, || vec![[4., 0.], [1.5, 5.]]);
        assert_eq!(rim, original, "preserve a separate branch with its own road mouth");
    }
}
