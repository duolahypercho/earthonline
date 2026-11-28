//! Ground shaping around open tunnel approaches and portal canopies.

/// An open cut (`canopy == false`) lowers the terrain vertices inside its
/// ring and within this many terrain cells of it to its floor, so no
/// terrain triangle reaching into the ring slopes across the cut's roadway
/// (a grid vertex's triangles reach its neighbours one cell away). One cell
/// is under the portal canopy's reach back over the bore
/// (eo-structures `tunnel::PORTAL_OPEN_M`, 12 m), so no vertex over a
/// covered bore past its canopy is lowered.
pub const CUT_REACH_CELLS: f64 = 1.0;

/// Horizontal distance from a point to a segment in plan.
pub fn distance_to_segment(a: [f64; 2], b: [f64; 2], x: f64, z: f64) -> f64 {
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dz * dz;
    let t = if l2 > 1e-12 {
        (((x - a[0]) * dx + (z - a[1]) * dz) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (x - a[0] - t * dx).hypot(z - a[1] - t * dz)
}

/// (x, z) is inside `c`'s ring or within `reach` of it.
fn near_cut(c: &crate::TerrainCut, x: f64, z: f64, reach: f64) -> bool {
    if x < c.min[0] - reach || x > c.max[0] + reach || z < c.min[1] - reach || z > c.max[1] + reach
    {
        return false;
    }
    if c.contains(x, z) {
        return true;
    }
    let n = c.ring.len();
    (0..n).any(|i| distance_to_segment(c.ring[i], c.ring[(i + 1) % n], x, z) <= reach)
}

/// Whether (x, z) lies behind a portal's face (over or beside the bore,
/// within `reach` of the face's ends): the face is a canopy ring's first and
/// last points (eo-structures `tunnel::cuts`: the ring runs from the face back
/// over the bore on one side and returns on the other).
fn behind_portal(c: &crate::TerrainCut, x: f64, z: f64, reach: f64) -> bool {
    let n = c.ring.len();
    if !c.canopy || n < 4 {
        return false;
    }
    let (a, b) = (c.ring[0], c.ring[n - 1]);
    let (mx, mz) = ((a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0);
    let (ax, az) = (b[0] - a[0], b[1] - a[1]);
    let half = ax.hypot(az) / 2.0;
    if half < 1e-6 {
        return false;
    }
    let (ux, uz) = (ax / (2.0 * half), az / (2.0 * half));
    // into the bore: the face's normal toward the ring's far end.
    let far = c.ring[n / 2];
    let (mut ix, mut iz) = (-uz, ux);
    if (far[0] - mx) * ix + (far[1] - mz) * iz < 0.0 {
        (ix, iz) = (-ix, -iz);
    }
    let (dx, dz) = (x - mx, z - mz);
    dx * ix + dz * iz > 0.0 && (dx * ux + dz * uz).abs() <= half + reach
}

/// The floor a terrain vertex at (x, z) is lowered to: the lowest of the open
/// cuts (`canopy == false`) holding it, or within `reach` of their ring and
/// not behind a portal's face outside its canopy (a canopy in `cuts`: the
/// hillside beside the bore keeps its height there; the headwall stands on
/// the face).
pub fn terrain_cut_floor(cuts: &[&crate::TerrainCut], x: f64, z: f64, reach: f64) -> Option<f64> {
    // (inside a canopy itself the ground is lowered: that part is left out, `minus_canopies`,
    // and what stays round it slopes down to the headwall, never across the mouth.)
    cuts.iter()
        .filter(|open| {
            if open.canopy {
                return false;
            }
            if open.contains(x, z) {
                return true;
            }
            if !near_cut(open, x, z, reach) {
                return false;
            }
            // A portal shields the hillside behind its own approach. The far
            // mouth and adjacent or stacked tubes must not veto this cut.
            let joins = |portal: &&crate::TerrainCut| {
                portal.canopy
                    && portal.chain == open.chain
                    && (portal.s0 == open.s0
                        || portal.s0 == open.s1
                        || portal.s1 == open.s0
                        || portal.s1 == open.s1)
            };
            let mut mouths = cuts.iter().copied().filter(joins);
            !mouths.clone().any(|p| behind_portal(p, x, z, reach))
                || mouths.any(|p| p.contains(x, z))
        })
        .map(|open| open.floor)
        .min_by(f64::total_cmp)
}

type Point = [f64; 3];

/// One station interval of a curved approach or canopy.
#[derive(Clone)]
pub struct GroundSection {
    ring: [[f64; 2]; 4],
    pub min: [f64; 2],
    pub max: [f64; 2],
    pub floor: f64,
    pub canopy: bool,
}

impl GroundSection {
    fn new(ring: [[f64; 2]; 4], floor: f64, canopy: bool) -> Self {
        let min = std::array::from_fn(|k| ring.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min));
        let max =
            std::array::from_fn(|k| ring.iter().map(|p| p[k]).fold(f64::NEG_INFINITY, f64::max));
        Self {
            ring,
            min,
            max,
            floor,
            canopy,
        }
    }

    pub fn intersects(&self, min: [f64; 2], max: [f64; 2]) -> bool {
        (0..2).all(|k| self.min[k] <= max[k] && self.max[k] >= min[k])
    }

    /// Keep the terrain outside this footprint. Under an open approach keep
    /// the lower of the ground and the excavation floor. A canopy is a hole.
    pub fn shape(&self, triangle: [Point; 3]) -> Option<Vec<Vec<Point>>> {
        if !self.canopy && triangle.iter().all(|p| p[1] <= self.floor) {
            return None;
        }
        let (mut outside, inside) = partition(triangle, &self.ring)?;
        if !self.canopy {
            let floor = self.floor;
            if inside.iter().all(|p| p[1] <= floor) {
                return None;
            }
            let (low, mut high) = split(&inside, |p| floor - p[1]);
            for p in &mut high {
                p[1] = floor;
            }
            if low.len() >= 3 {
                outside.push(low);
            }
            if high.len() >= 3 {
                outside.push(high);
            }
        }
        Some(outside)
    }
}

impl crate::TerrainCut {
    /// The source sweep stores one edge going forward and the other coming
    /// back. Each pair of neighbouring cross-sections is convex, even when
    /// the outline of the complete approach bends around a hillside.
    pub fn sections(&self) -> impl Iterator<Item = GroundSection> + '_ {
        let ring = &self.ring;
        let stations = ring.len() / 2;
        // A straight sweep is a single trapezoid. Internal station lines
        // do not bound the excavation and must not cut the ground beside it.
        let straight = stations > 2 && ring.len() % 2 == 0 && [0, stations].into_iter().all(|start| {
            let side = &ring[start..start + stations];
            side[1..stations - 1].iter().all(|&[x, z]| {
                distance_to_segment(side[0], side[stations - 1], x, z) < 1e-8
            })
        });
        let count = if straight { 1 } else { stations.saturating_sub(1) };
        (0..count).map(move |i| {
            let end = if straight { stations - 1 } else { i + 1 };
            let last = ring.len() - 1;
            GroundSection::new(
                [ring[i], ring[end], ring[last - end], ring[last - i]],
                self.floor,
                self.canopy,
            )
        })
    }
}

fn split(poly: &[Point], distance: impl Fn(Point) -> f64) -> (Vec<Point>, Vec<Point>) {
    let (mut inside, mut outside) = (Vec::new(), Vec::new());
    let mut prev = *poly.last().unwrap();
    for &p in poly {
        let (a, b) = (distance(prev), distance(p));
        if a * b < 0.0 {
            let t = a / (a - b);
            let cross = std::array::from_fn(|k| prev[k] + (p[k] - prev[k]) * t);
            inside.push(cross);
            outside.push(cross);
        }
        if b >= 0.0 {
            inside.push(p);
        }
        if b <= 0.0 {
            outside.push(p);
        }
        prev = p;
    }
    (inside, outside)
}

fn partition(tri: [Point; 3], ring: &[[f64; 2]]) -> Option<(Vec<Vec<Point>>, Vec<Point>)> {
    let area: f64 = (0..ring.len())
        .map(|i| {
            let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
            (a[0] - ring[0][0]) * (b[1] - ring[0][1]) - (a[1] - ring[0][1]) * (b[0] - ring[0][0])
        })
        .sum();
    if area.abs() < 1e-8 {
        return None;
    }
    let mut rest = tri.to_vec();
    let mut pieces = Vec::new();
    for i in 0..ring.len() {
        let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
        if (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-9 {
            continue;
        }
        let distance = |p: Point| {
            area.signum() * ((b[0] - a[0]) * (p[2] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]))
        };
        if rest.iter().all(|&p| distance(p) <= 1e-9) {
            return None;
        }
        if rest.iter().all(|&p| distance(p) >= 0.0) {
            continue;
        }
        let (inside, outside) = split(&rest, distance);
        if inside.len() < 3 {
            return None;
        }
        if outside.len() >= 3 {
            pieces.push(outside);
        }
        rest = inside;
    }
    Some((pieces, rest))
}

/// A triangle with a convex footprint removed, preserving its plane.
pub fn minus_convex(tri: [Point; 3], ring: &[[f64; 2]]) -> Vec<Vec<Point>> {
    partition(tri, ring).map_or_else(|| vec![tri.to_vec()], |(outside, _)| outside)
}
