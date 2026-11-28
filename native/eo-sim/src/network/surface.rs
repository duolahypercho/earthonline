use eo_data::sim_source::TrafficJunction;
use std::sync::Arc;

use crate::consts::JUNCTION_SWEEP_STEP_M;
use crate::geom::{P2, Plan};

/// One drawn junction pad, shared by the lane ends that open onto it.
#[derive(Debug)]
pub struct JunctionSurface {
    pub(crate) position: P2,
    elevation: f64,
    bounds: [P2; 2],
    rim: Vec<RimVertex>,
    triangles: Vec<Triangle>,
}

#[derive(Debug)]
struct RimVertex {
    offset: [f64; 3],
    inv_det_to_next: f64,
}

impl RimVertex {
    fn weights(&self, [dx, dz]: P2, next: &Self) -> [f64; 3] {
        let (u, v) = (self.offset, next.offset);
        let a = (dx * v[2] - dz * v[0]) * self.inv_det_to_next;
        let b = (u[0] * dz - u[2] * dx) * self.inv_det_to_next;
        [a, b, 1.0 - a - b]
    }
}

#[derive(Debug)]
struct Triangle {
    a: [f64; 3],
    u: [f64; 3],
    v: [f64; 3],
    inv_det: f64,
}

impl Triangle {
    fn new(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> Option<Self> {
        let u: [f64; 3] = std::array::from_fn(|i| b[i] - a[i]);
        let v: [f64; 3] = std::array::from_fn(|i| c[i] - a[i]);
        let det = u[0] * v[2] - u[2] * v[0];
        (det.abs() >= 1e-12).then(|| Self { a, u, v, inv_det: det.recip() })
    }

    fn weights(&self, [x, z]: P2) -> [f64; 3] {
        let (dx, dz) = (x - self.a[0], z - self.a[2]);
        let a = (dx * self.v[2] - dz * self.v[0]) * self.inv_det;
        let b = (self.u[0] * dz - self.u[2] * dx) * self.inv_det;
        [a, b, 1.0 - a - b]
    }
}

impl JunctionSurface {
    pub(crate) fn from_drawn(pad: &TrafficJunction) -> Option<Self> {
        if pad.rim.len() < 3 || !pad.center.iter().chain(pad.rim.iter().flatten()).all(|v| v.is_finite()) {
            return None;
        }
        let [x, elevation, z] = pad.center.map(f64::from);
        let position = [x, z];
        let mut bounds = [position; 2];
        let offset = |p: [f32; 3]| std::array::from_fn(|i| f64::from(p[i]) - f64::from(pad.center[i]));
        let rim: Vec<_> = pad.rim.iter().zip(pad.rim.iter().cycle().skip(1)).map(|(&p, &next)| {
            for (i, value) in [f64::from(p[0]), f64::from(p[2])].into_iter().enumerate() {
                bounds[0][i] = bounds[0][i].min(value);
                bounds[1][i] = bounds[1][i].max(value);
            }
            let u = offset(p);
            let v = offset(next);
            let det = u[0] * v[2] - u[2] * v[0];
            let inv_det_to_next = if det.abs() < 1e-12 { 0.0 } else { det.recip() };
            RimVertex { offset: u, inv_det_to_next }
        }).collect();
        let vertex = |i: usize| {
            if i == 0 { Some([0.0; 3]) } else { rim.get(i - 1).map(|p| p.offset) }
        };
        let mut triangles = Vec::with_capacity(pad.triangles.len());
        for &[a, b, c] in &pad.triangles {
            if let Some(triangle) = Triangle::new(vertex(a)?, vertex(b)?, vertex(c)?) {
                triangles.push(triangle);
            }
        }
        let valid = if pad.triangles.is_empty() {
            rim.iter().any(|v| v.inv_det_to_next != 0.0)
        } else {
            !triangles.is_empty()
        };
        valid.then_some(Self { position, elevation, bounds, rim, triangles })
    }

    pub(crate) fn height_near(&self, [x, z]: P2, reference: f64) -> Option<f64> {
        // The bounds reject distant probes. Keep points on the exact clipped
        // edge despite rounding in the lane's arc-to-point conversion.
        const EDGE_EPS: f64 = 1e-8;
        if !(self.bounds[0][0] - EDGE_EPS..=self.bounds[1][0] + EDGE_EPS).contains(&x)
            || !(self.bounds[0][1] - EDGE_EPS..=self.bounds[1][1] + EDGE_EPS).contains(&z)
        {
            return None;
        }
        let point = [x - self.position[0], z - self.position[1]];
        let closer = |a: &f64, b: &f64| (a - reference).abs().total_cmp(&(b - reference).abs());
        // The same containment tolerance as the built street mesh query.
        let inside = |a: f64, b: f64, c: f64| a >= -1e-6 && b >= -1e-6 && c >= -1e-6;
        if self.triangles.is_empty() {
            return self.rim.iter().zip(self.rim.iter().cycle().skip(1)).filter_map(|(v, next)| {
                if v.inv_det_to_next == 0.0 { return None; }
                let [a, b, c] = v.weights(point, next);
                inside(a, b, c).then_some(self.elevation + a * v.offset[1] + b * next.offset[1])
            }).min_by(closer);
        }
        self.triangles.iter().filter_map(|triangle| {
            let [a, b, c] = triangle.weights(point);
            inside(a, b, c)
                .then_some(self.elevation + triangle.a[1] + a * triangle.u[1] + b * triangle.v[1])
        }).min_by(closer)
    }

    /// The structure mouth is exported in f64 and the pad rim in f32.
    /// Match a mouth rounded just outside that edge to the nearest rim height.
    pub(crate) fn height_at_mouth(&self, [x, z]: P2, reference: f64) -> Option<f64> {
        self.height_near([x, z], reference).or_else(|| {
            let tolerance = f64::from(f32::EPSILON) * x.abs().max(z.abs()).max(1.0);
            let [lo, hi] = self.bounds;
            if !(lo[0] - tolerance..=hi[0] + tolerance).contains(&x)
                || !(lo[1] - tolerance..=hi[1] + tolerance).contains(&z)
            {
                return None;
            }
            let [x, z] = [x - self.position[0], z - self.position[1]];
            self.rim.iter().zip(self.rim.iter().cycle().skip(1)).filter_map(|(a, b)| {
                let [ax, ay, az] = a.offset;
                let [dx, dy, dz] = std::array::from_fn(|i| b.offset[i] - a.offset[i]);
                let square = dx * dx + dz * dz;
                if square == 0.0 {
                    return None;
                }
                let t = (((x - ax) * dx + (z - az) * dz) / square).clamp(0.0, 1.0);
                let distance = (x - ax - t * dx).hypot(z - az - t * dz);
                (distance <= tolerance).then_some((distance, self.elevation + ay + t * dy))
            }).min_by(|a, b| a.0.total_cmp(&b.0).then((a.1 - reference).abs().total_cmp(&(b.1 - reference).abs())))
                .map(|(_, height)| height)
        })
    }

    fn clip_triangles(&self, a: P2, b: P2, mut take: impl FnMut([f64; 2])) {
        let Some(bounds) = clip(a, b, self.bounds) else { return };
        let shift = |[x, z]: P2| [x - self.position[0], z - self.position[1]];
        let [a, b] = [shift(a), shift(b)];
        let mut add = |start: [f64; 3], end| {
            let mut range = bounds;
            for (a, b) in start.into_iter().zip(end) {
                if a < 0.0 && b < 0.0 {
                    range = [1.0, 0.0];
                    break;
                }
                if a < 0.0 { range[0] = range[0].max(a / (a - b)); }
                if b < 0.0 { range[1] = range[1].min(a / (a - b)); }
            }
            if range[0] <= range[1] {
                take(range);
            }
        };
        if self.triangles.is_empty() {
            for (v, next) in self.rim.iter().zip(self.rim.iter().cycle().skip(1)) {
                if v.inv_det_to_next != 0.0 {
                    add(v.weights(a, next), v.weights(b, next));
                }
            }
        } else {
            for triangle in &self.triangles {
                add(triangle.weights(a), triangle.weights(b));
            }
        }
    }

    pub(crate) fn lane_span(&self, points: &[P2], cum: &[f64]) -> Option<[f64; 2]> {
        let spans = self.lane_intervals(points, cum);
        Some([spans.first()?[0], spans.last()?[1]])
    }

    pub(crate) fn lane_intervals(&self, points: &[P2], cum: &[f64]) -> Vec<[f64; 2]> {
        let mut intervals = Vec::new();
        for (edge, s) in points.windows(2).zip(cum.windows(2)) {
            self.clip_triangles(edge[0], edge[1], |span| {
                intervals.push(span.map(|fraction| s[0] + fraction * (s[1] - s[0])));
            });
        }
        merge_intervals(intervals)
    }

    /// Locate both sides of each gap between the curved path and the drawn pad.
    pub(crate) fn movement_seams(&self, plan: &Plan, reference: impl Fn(f64) -> f64) -> Box<[[f64; 2]]> {
        let height = |arc: f64| {
            let (x, z, _) = plan.pose_at(arc);
            self.height_near([x, z], reference(arc))
        };
        let count = (plan.arc_len / JUNCTION_SWEEP_STEP_M).ceil().max(1.0) as usize;
        let mut seams = Vec::new();
        let (mut previous, mut previous_height) = (0.0, height(0.0));
        for i in 1..=count {
            let current = plan.arc_len * i as f64 / count as f64;
            let current_height = height(current);
            if current_height.is_some() != previous_height.is_some() {
                let (mut inside, mut outside, mut rim_y) = match previous_height {
                    Some(y) => (previous, current, y),
                    None => (current, previous, current_height.unwrap()),
                };
                // Refine the bracket on the curve so the outer grade meets the
                // same height a car sees on the first covered part of its path.
                for _ in 0..20 {
                    let mid = inside.midpoint(outside);
                    if let Some(y) = height(mid) {
                        inside = mid;
                        rim_y = y;
                    } else {
                        outside = mid;
                    }
                }
                seams.push([inside, rim_y]);
            }
            (previous, previous_height) = (current, current_height);
        }
        seams.into_boxed_slice()
    }
}

#[derive(Clone, Debug)]
pub(super) struct LaneJunction {
    pub pad: Arc<JunctionSurface>,
    pub intervals: Box<[[f64; 2]]>,
}

impl LaneJunction {
    pub fn new(pad: Arc<JunctionSurface>, points: &[P2], cum: &[f64], carriageway: &[[f64; 2]]) -> Option<Self> {
        let intervals = pad.lane_intervals(points, cum);
        if intervals.is_empty() {
            return None;
        }
        if carriageway.is_empty() {
            return Some(Self { intervals: intervals.into_boxed_slice(), pad });
        }
        // Only attach the pad where the lane leaves its own drawn carriageway.
        let mut outside = Vec::new();
        for [mut lo, hi] in intervals {
            for &[a, b] in carriageway {
                if a >= hi { break; }
                if b <= lo { continue; }
                if a > lo { outside.push([lo, a]); }
                lo = b;
            }
            if lo < hi { outside.push([lo, hi]); }
        }
        Some(Self { intervals: outside.into_boxed_slice(), pad })
    }

    pub fn contains(&self, arc: f64) -> bool {
        self.intervals.iter().any(|&[a, b]| (a..=b).contains(&arc))
    }
}

pub(super) fn merge_intervals(mut intervals: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    intervals.sort_unstable_by(|a, b| a[0].total_cmp(&b[0]));
    let mut merged: Vec<[f64; 2]> = Vec::new();
    for interval in intervals {
        if let Some(last) = merged.last_mut().filter(|last| interval[0] <= last[1] + 1e-8) {
            last[1] = last[1].max(interval[1]);
        } else {
            merged.push(interval);
        }
    }
    merged
}

/// Clip a centreline segment against the pad bounds in segment parameters.
fn clip(a: P2, b: P2, bounds: [P2; 2]) -> Option<[f64; 2]> {
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for i in 0..2 {
        let d = b[i] - a[i];
        if d == 0.0 {
            if !(bounds[0][i]..=bounds[1][i]).contains(&a[i]) {
                return None;
            }
        } else {
            let enter = (bounds[0][i] - a[i]) / d;
            let exit = (bounds[1][i] - a[i]) / d;
            lo = lo.max(enter.min(exit));
            hi = hi.min(enter.max(exit));
        }
    }
    (lo <= hi).then_some([lo, hi])
}
