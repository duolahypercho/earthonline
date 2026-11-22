//! Same-route overlaps: where OSM draws a paved (or lower) piece of the very
//! way or named route a deck carries, connected to the deck and running
//! along it rather than across, the deck does not pass over it; it is the
//! same road (a split way, a parallel piece of one carriageway), not a road
//! crossed. "Connected" is through pieces of the same route only, within a
//! few hundred metres along them, so a route's other carriageway (which
//! meets this one only at an interchange far away) still counts as crossed
//! where a double deck stacks it under this one.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashSet};

use eo_street::plan::{P2, arc_table};

use crate::index::project;

/// Two pieces run along each other when their headings differ by less than
/// this. INFERRED (a solve rule, not a street dimension): wide of a ramp's
/// diverge angle, narrow of any street crossing.
pub(crate) const ALONG_DEG: f64 = 30.0;

/// The most a same-route connection may run through the network (metres
/// along the pieces, from the overlap on one to the overlap on the other).
/// INFERRED: "a few hundred metres", about one SoMa block and a half.
pub(crate) const REACH_M: f64 = 400.0;

/// One piece of the route: a chain's or a paved run's centreline.
#[derive(Clone, Debug)]
pub(crate) struct Elem {
    pub pts: Vec<P2>,
    pub cum: Vec<f64>,
}

impl Elem {
    pub fn new(pts: Vec<P2>) -> Self {
        let cum = arc_table(&pts);
        Self { pts, cum }
    }

    fn len(&self) -> f64 {
        self.cum.last().copied().unwrap_or(0.0)
    }

    /// Station of the point of the centreline nearest `p`, and the distance.
    pub fn project(&self, p: P2) -> (f64, f64) {
        let mut best = (0.0, f64::INFINITY);
        for i in 0..self.pts.len().saturating_sub(1) {
            let (t, d, _) = project(p, self.pts[i], self.pts[i + 1]);
            if d < best.1 {
                best = (self.cum[i] + t * (self.cum[i + 1] - self.cum[i]), d);
            }
        }
        best
    }

    /// Unit heading at station `s`.
    pub fn dir(&self, s: f64) -> P2 {
        let n = self.pts.len();
        let mut i = 0;
        while i + 2 < n && self.cum[i + 1] < s {
            i += 1;
        }
        let (a, b) = (self.pts[i], self.pts[(i + 1).min(n - 1)]);
        let l = (b[0] - a[0]).hypot(b[1] - a[1]).max(1e-12);
        [(b[0] - a[0]) / l, (b[1] - a[1]) / l]
    }
}

/// Headings `a` and `b` (unit) run along each other (either way).
pub(crate) fn along(a: P2, b: P2) -> bool {
    (a[0] * b[0] + a[1] * b[1]).abs() >= ALONG_DEG.to_radians().cos()
}

/// Where `a` touches `b`: an end point of either within `snap` of the
/// other's centreline. (station on a, station on b).
fn touches(a: &Elem, b: &Elem, snap: f64) -> Vec<(f64, f64)> {
    let mut out = vec![];
    for (s, p) in [(0.0, a.pts[0]), (a.len(), *a.pts.last().unwrap())] {
        let (t, d) = b.project(p);
        if d <= snap {
            out.push((s, t));
        }
    }
    for (t, p) in [(0.0, b.pts[0]), (b.len(), *b.pts.last().unwrap())] {
        let (s, d) = a.project(p);
        if d <= snap {
            out.push((s, t));
        }
    }
    out
}

#[derive(PartialEq)]
struct Item(f64, usize, f64);

impl Eq for Item {}

impl Ord for Item {
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.total_cmp(&self.0).then(o.1.cmp(&self.1)).then(o.2.total_cmp(&self.2))
    }
}

impl PartialOrd for Item {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// `elems[0]` (over its stations `from`) reaches `elems[1]` (at its station
/// `to`) through the elements, within `REACH_M` along them.
pub(crate) fn connected(elems: &[Elem], from: [f64; 2], to: f64, snap: f64) -> bool {
    let n = elems.len();
    let mut links: Vec<Vec<(f64, usize, f64)>> = vec![vec![]; n];
    for i in 0..n {
        for j in i + 1..n {
            for (si, sj) in touches(&elems[i], &elems[j], snap) {
                links[i].push((si, j, sj));
                links[j].push((sj, i, si));
            }
        }
    }
    let mut heap = BinaryHeap::new();
    let mut seen: HashSet<(usize, i64)> = HashSet::new();
    for &(si, j, sj) in &links[0] {
        let cost = (from[0] - si).max(si - from[1]).max(0.0);
        if cost <= REACH_M {
            heap.push(Item(cost, j, sj));
        }
    }
    while let Some(Item(cost, j, s)) = heap.pop() {
        if !seen.insert((j, (s * 10.0).round() as i64)) {
            continue;
        }
        if j == 1 {
            if cost + (s - to).abs() <= REACH_M {
                return true;
            }
            continue;
        }
        if j == 0 {
            continue;
        }
        for &(sj, k, sk) in &links[j] {
            let c = cost + (sj - s).abs();
            if c <= REACH_M {
                heap.push(Item(c, k, sk));
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_route_reach() {
        let line = |a: P2, b: P2| Elem::new(vec![a, b]);
        // a deck from x 0..100, a paved piece running back under it from x 100
        // to 40 (joined at x 100), and a far piece joined only through 600 m.
        let deck = line([0.0, 0.0], [100.0, 0.0]);
        let under = line([100.0, 0.0], [40.0, 3.0]);
        assert!(connected(&[deck.clone(), under.clone()], [40.0, 90.0], 30.0, 0.6));
        let far = line([700.0, 0.0], [600.0, 3.0]);
        let between = line([100.0, 0.0], [700.0, 0.0]);
        assert!(!connected(&[deck.clone(), far.clone(), between.clone()], [40.0, 90.0], 50.0, 0.6));
        assert!(!connected(&[deck, far], [40.0, 90.0], 50.0, 0.6));
        assert!(along([1.0, 0.0], [-0.95, 0.312]) && !along([1.0, 0.0], [0.7, 0.714]));
    }
}
