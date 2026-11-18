//! `createPavedRoadwayIndex` (street-surface-v2.js): every plan ribbon's
//! carriageway (|u| < half along its polyline, the stretches it gave away
//! (`yieldSpans`) kept as vehicle mouths) and every junction pad (the corner
//! paths' kerb ring), with `hit(x, z, radius)`; and `rampHit`: every kerb ramp
//! the ring cuts (ribbon crossing ramps, reserved and resolved, from the kerb
//! line back `curbTopWidth + rampRun`; corner-return spans whose kerb is
//! depressed). The one shared answer to "is this the road / a ramp?" that the
//! JS signal, sign and furniture passes all read.

use eo_street::StreetOptions;
use eo_street::plan::{Plan, frame_at, segment_kerb_ramps};
use std::collections::HashMap;

pub type P2 = [f64; 2];
const CELL: f64 = 24.0;

#[derive(Clone)]
struct SegPiece {
    a: P2,
    b: P2,
    half: f64,
    entry: u32,
}

#[derive(Clone)]
struct Ring {
    ring: Vec<P2>,
    min: P2,
    max: P2,
    id: u32,
}

impl Ring {
    fn new(ring: Vec<P2>, id: u32) -> Self {
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        for p in &ring {
            min = [min[0].min(p[0]), min[1].min(p[1])];
            max = [max[0].max(p[0]), max[1].max(p[1])];
        }
        Ring { ring, min, max, id }
    }
    fn touches(&self, x: f64, z: f64, r: f64) -> bool {
        if x < self.min[0] - r || x > self.max[0] + r || z < self.min[1] - r || z > self.max[1] + r {
            return false;
        }
        if point_in_ring(&self.ring, x, z) {
            return true;
        }
        let n = self.ring.len();
        let mut l = n - 1;
        for k in 0..n {
            if seg_distance(x, z, self.ring[l], self.ring[k]) < r {
                return true;
            }
            l = k;
        }
        false
    }
}

/// `roadsidePointInRing`.
pub fn point_in_ring(ring: &[P2], x: f64, z: f64) -> bool {
    let mut inside = false;
    let n = ring.len();
    if n == 0 {
        return false;
    }
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (ring[i], ring[j]);
        let dz = b[1] - a[1];
        if (a[1] > z) != (b[1] > z) && x < ((b[0] - a[0]) * (z - a[1])) / (if dz == 0.0 { 1e-12 } else { dz }) + a[0] {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// `roadsideSegmentDistance`.
pub fn seg_distance(x: f64, z: f64, a: P2, b: P2) -> f64 {
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dz * dz;
    let t = if l2 > 1e-12 { (((x - a[0]) * dx + (z - a[1]) * dz) / l2).clamp(0.0, 1.0) } else { 0.0 };
    (x - (a[0] + dx * t)).hypot(z - (a[1] + dz * t))
}

fn cell_of(v: f64) -> i32 {
    (v / CELL).floor() as i32
}

pub struct RoadwayIndex {
    segments: HashMap<(i32, i32), Vec<u32>>,
    pieces: Vec<SegPiece>,
    pads: HashMap<(i32, i32), Vec<u32>>,
    pad_rings: Vec<Ring>,
    ramps: HashMap<(i32, i32), Vec<u32>>,
    ramp_rings: Vec<Ring>,
}

/// What a roadway hit found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Carriageway(u32),
    Pad(u32),
}

impl RoadwayIndex {
    pub fn new(plan: &Plan, o: &StreetOptions) -> Self {
        let mut segments: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        let mut pieces = vec![];
        for (ei, e) in plan.entries.iter().enumerate() {
            if e.points.len() < 2 || !(e.half > 0.0) {
                continue;
            }
            let cum = &e.cum;
            let given = &e.yield_spans;
            let mut add = |a: P2, b: P2| {
                let id = pieces.len() as u32;
                pieces.push(SegPiece { a, b, half: e.half, entry: ei as u32 });
                let reach = e.half + 3.0;
                for gx in cell_of(a[0].min(b[0]) - reach)..=cell_of(a[0].max(b[0]) + reach) {
                    for gz in cell_of(a[1].min(b[1]) - reach)..=cell_of(a[1].max(b[1]) + reach) {
                        segments.entry((gx, gz)).or_default().push(id);
                    }
                }
            };
            for i in 0..e.points.len() - 1 {
                if given.is_empty() {
                    add(e.points[i], e.points[i + 1]);
                    continue;
                }
                let (s0, s1) = (cum[i], cum[i + 1]);
                let length = s1 - s0;
                if !(length > 1e-9) {
                    continue;
                }
                let at = |s: f64| {
                    let t = (s - s0) / length;
                    [e.points[i][0] + (e.points[i + 1][0] - e.points[i][0]) * t, e.points[i][1] + (e.points[i + 1][1] - e.points[i][1]) * t]
                };
                let mut kept = vec![[s0, s1]];
                for span in given {
                    let mut next = vec![];
                    for [lo, hi] in kept {
                        if span[1] <= lo || span[0] >= hi {
                            next.push([lo, hi]);
                            continue;
                        }
                        if span[0] > lo {
                            next.push([lo, span[0]]);
                        }
                        if span[1] < hi {
                            next.push([span[1], hi]);
                        }
                    }
                    kept = next;
                }
                for [lo, hi] in kept {
                    if hi - lo > 0.01 {
                        add(at(lo), at(hi));
                    }
                }
                // THE MOUTH: the given arc hits like carriageway.
                for span in given {
                    let lo = s0.max(span[0]);
                    let hi = s1.min(span[1]);
                    if hi - lo > 0.01 {
                        add(at(lo), at(hi));
                    }
                }
            }
        }
        let mut pads: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        let mut pad_rings = vec![];
        for (ni, n) in plan.nodes.iter().enumerate() {
            let ring: Vec<P2> = n.paths.iter().flat_map(|p| p.stations.iter().map(|s| [s.x, s.z])).collect();
            if ring.len() < 3 {
                continue;
            }
            let r = Ring::new(ring, ni as u32);
            let id = pad_rings.len() as u32;
            for gx in cell_of(r.min[0])..=cell_of(r.max[0]) {
                for gz in cell_of(r.min[1])..=cell_of(r.max[1]) {
                    pads.entry((gx, gz)).or_default().push(id);
                }
            }
            pad_rings.push(r);
        }
        let mut idx = RoadwayIndex { segments, pieces, pads, pad_rings, ramps: HashMap::new(), ramp_rings: vec![] };
        idx.build_ramps(plan, o);
        idx
    }

    fn add_ramp(&mut self, ring: Vec<P2>, id: u32) {
        if ring.len() < 3 {
            return;
        }
        let r = Ring::new(ring, id);
        let k = self.ramp_rings.len() as u32;
        for gx in cell_of(r.min[0])..=cell_of(r.max[0]) {
            for gz in cell_of(r.min[1])..=cell_of(r.max[1]) {
                self.ramps.entry((gx, gz)).or_default().push(k);
            }
        }
        self.ramp_rings.push(r);
    }

    fn build_ramps(&mut self, plan: &Plan, o: &StreetOptions) {
        let reach = o.curb_top_width + o.ramp_run;
        if !(reach > 0.0) {
            return;
        }
        for (ei, e) in plan.entries.iter().enumerate() {
            if e.points.len() < 2 || e.cum.len() != e.points.len() || !(e.half > 0.0) {
                continue;
            }
            let resolved = segment_kerb_ramps(e, o);
            let total = *e.cum.last().unwrap();
            for k in 0..2 {
                let side = if k == 0 { 1.0 } else { -1.0 };
                for w in e.kerb_ramps[k].iter().chain(resolved[k].iter()) {
                    let flo = if w.flare_lo.is_finite() { w.flare_lo } else { w.lo };
                    let fhi = if w.flare_hi.is_finite() { w.flare_hi } else { w.hi };
                    let lo = flo.min(w.lo).max(0.0);
                    let hi = fhi.max(w.hi).min(total);
                    if !(hi > lo) {
                        continue;
                    }
                    let steps = ((hi - lo) / 0.75).ceil().max(1.0) as usize;
                    let mut kerb = vec![];
                    let mut back = vec![];
                    for j in 0..=steps {
                        let f = frame_at(&e.points, &e.cum, lo + (hi - lo) * j as f64 / steps as f64, false);
                        kerb.push([f.x + f.nx * side * e.half, f.z + f.nz * side * e.half]);
                        back.push([f.x + f.nx * side * (e.half + reach), f.z + f.nz * side * (e.half + reach)]);
                    }
                    back.reverse();
                    kerb.extend(back);
                    self.add_ramp(kerb, ei as u32);
                }
            }
        }
        for (ni, n) in plan.nodes.iter().enumerate() {
            for path in &n.paths {
                if path.ramps.is_empty() || path.stations.len() < 2 {
                    continue;
                }
                let depressed = |ang: Option<f64>| {
                    let Some(ang) = ang else { return false };
                    path.ramps.iter().any(|w| {
                        let lo = (if w.flare_lo.is_finite() { w.flare_lo } else { w.lo }).min(w.lo);
                        let hi = (if w.flare_hi.is_finite() { w.flare_hi } else { w.hi }).max(w.hi);
                        ang >= lo - 1e-9 && ang <= hi + 1e-9
                    })
                };
                for k in 0..path.stations.len() - 1 {
                    let (a, b) = (path.stations[k], path.stations[k + 1]);
                    if !depressed(a.ang) && !depressed(b.ang) {
                        continue;
                    }
                    self.add_ramp(
                        vec![[a.x, a.z], [b.x, b.z], [b.x + b.out[0] * reach, b.z + b.out[1] * reach], [a.x + a.out[0] * reach, a.z + a.out[1] * reach]],
                        u32::MAX - ni as u32,
                    );
                }
            }
        }
    }

    /// `hit(x, z, radius)`: the carriageway or junction pad the circle touches.
    pub fn hit(&self, x: f64, z: f64, radius: f64) -> Option<Hit> {
        self.hit_ignoring(x, z, radius, None)
    }

    pub fn hit_ignoring(&self, x: f64, z: f64, radius: f64, ignore_entry: Option<u32>) -> Option<Hit> {
        if !x.is_finite() || !z.is_finite() {
            return None;
        }
        let r = radius.max(0.0);
        let (gx, gz) = (cell_of(x), cell_of(z));
        for i in -1..=1 {
            for j in -1..=1 {
                let key = (gx + i, gz + j);
                if let Some(v) = self.segments.get(&key) {
                    for &id in v {
                        let e = &self.pieces[id as usize];
                        if Some(e.entry) == ignore_entry {
                            continue;
                        }
                        if seg_distance(x, z, e.a, e.b) < e.half + r {
                            return Some(Hit::Carriageway(e.entry));
                        }
                    }
                }
                if let Some(v) = self.pads.get(&key) {
                    for &id in v {
                        let p = &self.pad_rings[id as usize];
                        if p.touches(x, z, r) {
                            return Some(Hit::Pad(p.id));
                        }
                    }
                }
            }
        }
        None
    }

    /// `rampHit(x, z, radius)`.
    pub fn ramp_hit(&self, x: f64, z: f64, radius: f64) -> bool {
        if !x.is_finite() || !z.is_finite() {
            return false;
        }
        let r = radius.max(0.0);
        let (gx, gz) = (cell_of(x), cell_of(z));
        for i in -1..=1 {
            for j in -1..=1 {
                if let Some(v) = self.ramps.get(&(gx + i, gz + j)) {
                    for &id in v {
                        if self.ramp_rings[id as usize].touches(x, z, r) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
}
