//! Plan-view indexes: a 32 m cell grid of line pieces and discs, the paved
//! network of the street plan (carriageways, footways, junction pads), and
//! the small geometry the solve needs.

use std::collections::HashMap;

use eo_street::plan::{P2, Plan};

/// Cell size of every plan index here (metres).
pub(crate) const CELL_M: f64 = 32.0;

pub(crate) fn cell_of(v: f64) -> i32 {
    (v / CELL_M).floor() as i32
}

/// Values filed under every cell a padded box touches.
#[derive(Debug)]
pub(crate) struct Grid<T> {
    map: HashMap<(i32, i32), Vec<T>>,
}

impl<T> Default for Grid<T> {
    fn default() -> Self {
        Self { map: HashMap::new() }
    }
}

impl<T: Copy> Grid<T> {
    pub fn insert_seg(&mut self, a: P2, b: P2, pad: f64, v: T) {
        let (x0, x1) = (a[0].min(b[0]) - pad, a[0].max(b[0]) + pad);
        let (z0, z1) = (a[1].min(b[1]) - pad, a[1].max(b[1]) + pad);
        for gz in cell_of(z0)..=cell_of(z1) {
            for gx in cell_of(x0)..=cell_of(x1) {
                self.map.entry((gx, gz)).or_default().push(v);
            }
        }
    }

    pub fn into_map(self) -> HashMap<(i32, i32), Vec<T>> {
        self.map
    }

    pub fn at(&self, x: f64, z: f64) -> &[T] {
        self.map.get(&(cell_of(x), cell_of(z))).map(|v| v.as_slice()).unwrap_or(&[])
    }
}

/// Closest point of segment `a b` to `p`: (parameter in [0, 1], distance, point).
pub(crate) fn project(p: P2, a: P2, b: P2) -> (f64, f64, P2) {
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dz * dz;
    let t = if len2 < 1e-12 { 0.0 } else { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / len2).clamp(0.0, 1.0) };
    let q = [a[0] + dx * t, a[1] + dz * t];
    (t, (p[0] - q[0]).hypot(p[1] - q[1]), q)
}

/// Distance between segments `a b` and `c d`: (distance, the point of `c d`
/// nearest `a b`).
pub(crate) fn seg_distance(a: P2, b: P2, c: P2, d: P2) -> (f64, P2) {
    if let Some((_, u)) = crossing(a, b, c, d) {
        return (0.0, [c[0] + (d[0] - c[0]) * u, c[1] + (d[1] - c[1]) * u]);
    }
    let mut best = (f64::INFINITY, c);
    for p in [a, b] {
        let (_, dist, q) = project(p, c, d);
        if dist < best.0 {
            best = (dist, q);
        }
    }
    for q in [c, d] {
        let dist = project(q, a, b).1;
        if dist < best.0 {
            best = (dist, q);
        }
    }
    best
}

/// Proper crossing of segments `a b` and `c d` (both parameters strictly
/// inside): the parameters along each.
pub(crate) fn crossing(a: P2, b: P2, c: P2, d: P2) -> Option<(f64, f64)> {
    let r = [b[0] - a[0], b[1] - a[1]];
    let s = [d[0] - c[0], d[1] - c[1]];
    let den = r[0] * s[1] - r[1] * s[0];
    if den.abs() < 1e-12 {
        return None;
    }
    let q = [c[0] - a[0], c[1] - a[1]];
    let t = (q[0] * s[1] - q[1] * s[0]) / den;
    let u = (q[0] * r[1] - q[1] * r[0]) / den;
    let e = 1e-6;
    (t > e && t < 1.0 - e && u > e && u < 1.0 - e).then_some((t, u))
}

/// The paved network of the street plan: each run's centreline with its
/// carriageway half width and its two footways, and each junction's pad
/// (the reach of its widest approach), as `StreetWorld::terrain_cap` reads
/// the paved corridor.
pub(crate) struct Paved<'a> {
    plan: &'a Plan,
    /// (entry, its point index, the piece's ends).
    pieces: Vec<(u32, u32, P2, P2)>,
    /// `StreetOptions::min_sidewalk_width`: a narrower footway is not drawn.
    min_walk: f64,
    /// `StreetOptions::max_step`: the street's longest station step.
    step: f64,
    grid: Grid<u32>,
    pads: Vec<(P2, f64)>,
    pad_grid: Grid<u32>,
}

/// A controlled-access class (no at-grade crossing to keep clear).
pub(crate) fn freeway_class(highway: &str) -> bool {
    matches!(highway, "motorway" | "motorway_link" | "trunk" | "trunk_link")
}

/// One paved run near a point: its nearest piece.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Near {
    pub entry: u32,
    /// Distance from the run's centreline.
    pub dist: f64,
    /// The side the point is on: 0 = the run's +n (left, `walks[0]`), 1 = -n.
    pub side: usize,
    /// The run's station nearest the point.
    pub s: f64,
}

/// One paved run under a point.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Under {
    pub entry: u32,
    pub dist: f64,
    pub at: P2,
    /// The distance signed: positive on the run's +n (left) side.
    pub side: f64,
}

impl<'a> Paved<'a> {
    /// `reach`: the most any query reaches past a run's paved corridor;
    /// `min_walk`: `StreetOptions::min_sidewalk_width`; `step`:
    /// `StreetOptions::max_step`.
    pub fn new(plan: &'a Plan, reach: f64, min_walk: f64, step: f64) -> Self {
        let mut pieces = vec![];
        let mut grid = Grid::default();
        for (ei, e) in plan.entries.iter().enumerate() {
            // (a structure's own leg into a junction, stage A6, is not paving.)
            if e.structure_leg.is_some() {
                continue;
            }
            let pad = e.half + e.walks[0].max(e.walks[1]) + reach;
            for (i, w) in e.points.windows(2).enumerate() {
                let id = pieces.len() as u32;
                pieces.push((ei as u32, i as u32, w[0], w[1]));
                grid.insert_seg(w[0], w[1], pad, id);
            }
        }
        let mut pads = vec![];
        let mut pad_grid = Grid::default();
        for n in &plan.nodes {
            let r = n
                .approaches
                .iter()
                .map(|a| {
                    let e = &plan.entries[a.entry];
                    a.trim + a.half + e.walks[0].max(e.walks[1])
                })
                .fold(0.0, f64::max);
            let id = pads.len() as u32;
            pads.push((n.position, r));
            pad_grid.insert_seg(n.position, n.position, r + reach, id);
        }
        Self { plan, pieces, min_walk, step, grid, pads, pad_grid }
    }

    /// A paved run ends within `tol` of `p`.
    pub fn run_ends_at(&self, p: P2, tol: f64) -> bool {
        self.grid.at(p[0], p[1]).iter().any(|&id| {
            let (ei, i, _, _) = self.pieces[id as usize];
            let e = &self.plan.entries[ei as usize];
            let ends = [(0usize, e.points[0]), (e.points.len() - 2, e.points[e.points.len() - 1])];
            ends.iter().any(|&(k, q)| k == i as usize && (p[0] - q[0]).hypot(p[1] - q[1]) <= tol)
        })
    }

    /// A paved centreline passes within `tol` of `p` (a run end or a run's middle).
    pub fn touches(&self, p: P2, tol: f64) -> bool {
        self.grid.at(p[0], p[1]).iter().any(|&id| {
            let (_, _, a, b) = self.pieces[id as usize];
            project(p, a, b).1 <= tol
        })
    }

    /// The runs whose carriageway (half width either side of the centreline)
    /// the cross-section `l r` of a deck (centre `p`) reaches over: nearest
    /// point of each run's centreline to the cross-section, and `p`'s signed
    /// offset from that run, in run order.
    pub fn under_section(&self, p: P2, l: P2, r: P2) -> Vec<Under> {
        let mut out: Vec<Under> = vec![];
        for &id in self.grid.at(p[0], p[1]) {
            let (ei, _, a, b) = self.pieces[id as usize];
            let e = &self.plan.entries[ei as usize];
            let (d, q) = seg_distance(l, r, a, b);
            if d > e.half {
                continue;
            }
            let (_, dp, qp) = project(p, a, b);
            let side = if (p[0] - qp[0]) * -(b[1] - a[1]) + (p[1] - qp[1]) * (b[0] - a[0]) >= 0.0 { dp } else { -dp };
            let u_new = Under { entry: ei, dist: d, at: q, side };
            match out.iter_mut().find(|u| u.entry == ei) {
                Some(u) if d < u.dist || (d == u.dist && dp < u.side.abs()) => *u = u_new,
                Some(_) => {}
                None => out.push(u_new),
            }
        }
        out.sort_by_key(|u| u.entry);
        out
    }

    /// The runs whose centreline passes within `reach` of `p`, each once (its
    /// nearest piece), in run order.
    pub fn runs_near(&self, p: P2, reach: f64) -> Vec<Near> {
        let mut out: Vec<Near> = vec![];
        for &id in self.grid.at(p[0], p[1]) {
            let (ei, i, a, b) = self.pieces[id as usize];
            let (t, d, q) = project(p, a, b);
            if d > reach {
                continue;
            }
            let e = &self.plan.entries[ei as usize];
            let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
            let left = (p[0] - q[0]) * -dz + (p[1] - q[1]) * dx > 0.0;
            let s = e.cum[i as usize] + t * (e.cum[i as usize + 1] - e.cum[i as usize]);
            let n = Near { entry: ei, dist: d, side: if left { 0 } else { 1 }, s };
            match out.iter_mut().find(|u| u.entry == ei) {
                Some(u) if d < u.dist => *u = n,
                Some(_) => {}
                None => out.push(n),
            }
        }
        out.sort_by_key(|u| u.entry);
        out
    }

    /// The junction pads (by plan node index) whose disc holds `p`.
    pub fn pads_at(&self, p: P2) -> Vec<u32> {
        self.pad_grid.at(p[0], p[1]).iter().copied().filter(|&id| {
            let (c, r) = self.pads[id as usize];
            (p[0] - c[0]).hypot(p[1] - c[1]) <= r
        }).collect()
    }

    /// `p` is within `margin` of a junction pad.
    pub fn on_pad(&self, p: P2, margin: f64) -> bool {
        self.pad_grid.at(p[0], p[1]).iter().any(|&id| {
            let (c, r) = self.pads[id as usize];
            (p[0] - c[0]).hypot(p[1] - c[1]) <= r + margin
        })
    }

    /// The footway on `side` (0 = +n, `walks[0]`) of run `entry` may be drawn
    /// at its station `s`, as the street draws it (`mesh.rs`): at least
    /// `min_sidewalk_width` wide and not where it yields to another roadway
    /// (`Entry::walk_yield_spans`). The street decides per station step (at
    /// its middle), so a footway is taken as drawn within a step
    /// (`StreetOptions::max_step`) of where it is drawn.
    pub fn walk_drawn(&self, entry: u32, side: usize, s: f64) -> bool {
        let e = &self.plan.entries[entry as usize];
        let yielded = |v: f64| e.walk_yield_spans[side].iter().any(|sp| v >= sp[0] && v <= sp[1]);
        e.walks[side] >= self.min_walk && [s - self.step, s, s + self.step].iter().any(|&v| !yielded(v.clamp(0.0, e.length)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_geometry() {
        let (t, d, q) = project([1.0, 1.0], [0.0, 0.0], [2.0, 0.0]);
        assert_eq!((t, d, q), (0.5, 1.0, [1.0, 0.0]));
        assert_eq!(project([-1.0, 0.0], [0.0, 0.0], [2.0, 0.0]).1, 1.0);
        assert!(crossing([0.0, -1.0], [0.0, 1.0], [-1.0, 0.0], [1.0, 0.0]).is_some());
        assert!(crossing([0.0, 0.0], [0.0, 1.0], [-1.0, 0.0], [1.0, 0.0]).is_none());
        let mut g: Grid<u32> = Grid::default();
        g.insert_seg([0.0, 0.0], [100.0, 0.0], 5.0, 7);
        assert_eq!(g.at(50.0, 3.0), &[7]);
        assert!(g.at(50.0, 40.0).is_empty());
    }
}
