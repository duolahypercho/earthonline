//! Which legs of a junction carry a painted crossing, from the city's own
//! data: a port of src/world/streets/crossing-evidence.js
//! (`crossingEvidenceFor`, `annotateJunctionCrossings`, `legCarriesCrossing`,
//! `earnsTreatment`). The OSM crossing ways and crossing nodes, and the DataSF
//! continental-crosswalk junction list, come from the export
//! (`crossings`, `crossingNodes`, `continental` in street-city.json.gz). The
//! curb-ramp survey half (`rampSurvey`, `cornerRampBudget`) is not ported.

use crate::plan::{Entry, Node, P2};
use std::collections::{HashMap, HashSet};

pub const LEG_REACH_M: f64 = 30.0;
pub const NODE_MATCH_M: f64 = 18.0;

/// the survey's matching distances on the scaled plan (an OSM
/// crossing or a DataSF continental point stands 1.25 x as far from its
/// junction as surveyed; the same crossings must still match).
fn plan_m(m: f64) -> f64 {
    m * eo_data::groundscale::scale()
}
const CELL: f64 = 40.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Marked,
    Unmarked,
    Unknown,
}

fn way_kind(tag: &str) -> Kind {
    match tag {
        "marked" | "zebra" | "traffic_signals" | "uncontrolled" | "pedestrian_signals" | "marking" | "markrd" | "ladder" => Kind::Marked,
        "unmarked" | "no" | "unmarkrd" | "informal" => Kind::Unmarked,
        _ => Kind::Unknown,
    }
}

fn node_kind(attr: &str) -> Kind {
    match attr {
        "S" | "I" | "m" | "J" => Kind::Marked,
        "u" => Kind::Unmarked,
        _ => Kind::Unknown,
    }
}

struct Grid<T> {
    map: HashMap<(i64, i64), Vec<usize>>,
    items: Vec<T>,
}

impl<T> Grid<T> {
    fn new() -> Self {
        Self { map: HashMap::new(), items: vec![] }
    }
    fn push(&mut self, item: T) -> usize {
        self.items.push(item);
        self.items.len() - 1
    }
    fn add(&mut self, x: f64, z: f64, id: usize) {
        let b = self.map.entry(((x / CELL).floor() as i64, (z / CELL).floor() as i64)).or_default();
        if b.last() != Some(&id) {
            b.push(id);
        }
    }
    fn near(&self, x: f64, z: f64, r: f64) -> Vec<usize> {
        let mut out: Vec<usize> = vec![];
        let mut seen = HashSet::new();
        for i in ((x - r) / CELL).floor() as i64..=((x + r) / CELL).floor() as i64 {
            for j in ((z - r) / CELL).floor() as i64..=((z + r) / CELL).floor() as i64 {
                if let Some(b) = self.map.get(&(i, j)) {
                    for &id in b {
                        if seen.insert(id) {
                            out.push(id);
                        }
                    }
                }
            }
        }
        out
    }
}

pub struct Evidence {
    ways: Grid<(Vec<P2>, Kind)>,
    nodes: Grid<(P2, Kind)>,
    /// Plan node indices DataSF lists as continental-crosswalk junctions.
    pub continental: HashSet<usize>,
}

/// One leg's verdict.
#[derive(Clone, Copy, Debug, Default)]
pub struct Leg {
    /// Some(true/false) where the data speaks for the leg.
    pub marked: Option<bool>,
    /// `crossingSource === LISTED_THROUGH_LEG`.
    pub listed_through: bool,
}

impl Evidence {
    /// `cityEvidence` + `crossingEvidenceFor`: each continental point bound to
    /// the one plan node nearest it within `NODE_MATCH_M`.
    pub fn new(ways: &[(Vec<P2>, String)], nodes: &[(P2, String)], continental: &[P2], plan_nodes: &[Node]) -> Self {
        let mut g = Grid::new();
        for (pts, tag) in ways {
            if pts.len() < 2 {
                continue;
            }
            let id = g.push((pts.clone(), way_kind(tag)));
            for k in 1..pts.len() {
                let (a, b) = (pts[k - 1], pts[k]);
                let n = (((b[0] - a[0]).hypot(b[1] - a[1])) / (CELL / 2.0)).ceil().max(1.0) as usize;
                for s in 0..=n {
                    let t = s as f64 / n as f64;
                    g.add(a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, id);
                }
            }
        }
        let mut ng = Grid::new();
        for (p, attr) in nodes {
            let id = ng.push((*p, node_kind(attr)));
            ng.add(p[0], p[1], id);
        }
        let mut pn: Grid<usize> = Grid::new();
        for (i, n) in plan_nodes.iter().enumerate() {
            let id = pn.push(i);
            pn.add(n.position[0], n.position[1], id);
        }
        let mut bound = HashSet::new();
        for p in continental {
            let mut best: Option<(usize, f64)> = None;
            for id in pn.near(p[0], p[1], plan_m(NODE_MATCH_M)) {
                let ni = pn.items[id];
                let q = plan_nodes[ni].position;
                let d = (q[0] - p[0]).hypot(q[1] - p[1]);
                if d <= plan_m(NODE_MATCH_M) && best.map(|b| d < b.1).unwrap_or(true) {
                    best = Some((ni, d));
                }
            }
            if let Some((ni, _)) = best {
                bound.insert(ni);
            }
        }
        Self { ways: g, nodes: ng, continental: bound }
    }

    /// `annotateJunctionCrossings` for plan node `ni` (approach `class_rank` set).
    pub fn annotate(&self, ni: usize, node: &Node, entries: &[Entry], street_of: &dyn Fn(usize) -> String) -> Vec<Leg> {
        let pos = node.position;
        let legs: Vec<(f64, Vec<P2>)> = node
            .approaches
            .iter()
            .map(|app| {
                let reach = plan_m(LEG_REACH_M).min(app.run_length.max(app.trim + 12.0).max(8.0));
                let pts = &entries[app.entry].points;
                let ordered: Vec<P2> = if app.at_start { pts.clone() } else { pts.iter().rev().copied().collect() };
                let mut out = vec![pos];
                let mut run = 0.0;
                let mut k = 1;
                while k < ordered.len() && run < reach {
                    let a = if k == 1 { out[0] } else { ordered[k - 1] };
                    let b = ordered[k];
                    let l = (b[0] - a[0]).hypot(b[1] - a[1]);
                    if run + l >= reach && l > 1e-9 {
                        let t = (reach - run) / l;
                        out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
                        run = reach;
                        break;
                    }
                    out.push(b);
                    run += l;
                    k += 1;
                }
                if run < reach - 1e-6 && out.len() >= 2 {
                    let (a, b) = (out[out.len() - 2], out[out.len() - 1]);
                    let mut l = (b[0] - a[0]).hypot(b[1] - a[1]);
                    if l == 0.0 {
                        l = 1.0;
                    }
                    let t = reach - run;
                    out.push([b[0] + (b[0] - a[0]) / l * t, b[1] + (b[1] - a[1]) / l * t]);
                }
                (reach, out)
            })
            .collect();
        let n = node.approaches.len();
        let mut hits: Vec<HashSet<u8>> = vec![HashSet::new(); n];
        let mut way_marked = vec![false; n];
        let key = |k: Kind| k as u8;
        for (i, (reach, leg)) in legs.iter().enumerate() {
            for id in self.ways.near(pos[0], pos[1], reach + CELL / 2.0) {
                let (pts, kind) = &self.ways.items[id];
                let mut hit = false;
                'outer: for a in 1..leg.len() {
                    for b in 1..pts.len() {
                        if seg_cross(leg[a - 1], leg[a], pts[b - 1], pts[b]) {
                            hit = true;
                            break 'outer;
                        }
                    }
                }
                if hit {
                    hits[i].insert(key(*kind));
                    if *kind == Kind::Marked {
                        way_marked[i] = true;
                    }
                }
            }
        }
        let reach_all = legs.iter().map(|l| l.0).fold(0.0, f64::max);
        for id in self.nodes.near(pos[0], pos[1], reach_all) {
            let (p, kind) = self.nodes.items[id];
            let mut best: Option<(usize, f64)> = None;
            for (i, app) in node.approaches.iter().enumerate() {
                if !(app.half > 0.0) {
                    continue;
                }
                let (d, s) = project(&legs[i].1, p);
                if s > 0.0 && s <= legs[i].0 && d <= app.half && best.map(|b| d < b.1).unwrap_or(true) {
                    best = Some((i, d));
                }
            }
            if let Some((i, _)) = best {
                hits[i].insert(key(kind));
            }
        }
        let mapped = hits.iter().any(|h| !h.is_empty());
        let signalised = node.signal.is_some();
        let continental = self.continental.contains(&ni);
        let names: Vec<String> = (0..n).map(street_of).collect();
        let carries_on = |i: usize| !names[i].is_empty() && (0..n).any(|j| j != i && names[j] == names[i]);
        let has = |i: usize, k: Kind| hits[i].contains(&key(k));
        let listing_has_a_home = (0..n).any(|i| has(i, Kind::Marked) || !carries_on(i) || node.approaches[i].class_rank < 3);
        let distinct: HashSet<&String> = names.iter().collect();
        let fills_crossroads = !listing_has_a_home && distinct.len() >= 2 && !(0..n).any(|i| has(i, Kind::Unmarked));
        (0..n)
            .map(|i| {
                let app = &node.approaches[i];
                let continental_leg = app.width_ccw.max(app.width_cw) > 0.5 && !(app.class_rank < 3);
                let _ = way_marked[i];
                if has(i, Kind::Marked) {
                    Leg { marked: Some(true), listed_through: false }
                } else if has(i, Kind::Unmarked) {
                    Leg { marked: Some(false), listed_through: false }
                } else if continental && !signalised && !fills_crossroads && !has(i, Kind::Unknown) && carries_on(i) && continental_leg {
                    Leg { marked: Some(false), listed_through: true }
                } else if continental && (has(i, Kind::Unknown) || mapped) {
                    Leg { marked: Some(continental_leg), listed_through: false }
                } else if has(i, Kind::Unknown) {
                    Leg { marked: if signalised { None } else { Some(false) }, listed_through: false }
                } else if mapped {
                    Leg { marked: Some(false), listed_through: false }
                } else if continental {
                    Leg { marked: Some(continental_leg), listed_through: false }
                } else if !signalised {
                    Leg { marked: Some(false), listed_through: false }
                } else {
                    Leg { marked: None, listed_through: false }
                }
            })
            .collect()
    }
}

fn seg_cross(a: P2, b: P2, c: P2, d: P2) -> bool {
    let (d1x, d1z) = (b[0] - a[0], b[1] - a[1]);
    let (d2x, d2z) = (d[0] - c[0], d[1] - c[1]);
    let den = d1x * d2z - d1z * d2x;
    if den.abs() < 1e-9 {
        return false;
    }
    let t = ((c[0] - a[0]) * d2z - (c[1] - a[1]) * d2x) / den;
    let s = ((c[0] - a[0]) * d1z - (c[1] - a[1]) * d1x) / den;
    (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&s)
}

/// `projectOnPolyline`: (lateral distance, arc length).
fn project(pts: &[P2], p: P2) -> (f64, f64) {
    let mut best = (f64::INFINITY, 0.0);
    let mut run = 0.0;
    for i in 1..pts.len() {
        let (a, b) = (pts[i - 1], pts[i]);
        let (vx, vz) = (b[0] - a[0], b[1] - a[1]);
        let len2 = vx * vx + vz * vz;
        let len = len2.sqrt();
        if len2 > 1e-12 {
            let t = (((p[0] - a[0]) * vx + (p[1] - a[1]) * vz) / len2).clamp(0.0, 1.0);
            let d = (p[0] - (a[0] + vx * t)).hypot(p[1] - (a[1] + vz * t));
            if d < best.0 {
                best = (d, run + len * t);
            }
        }
        run += len;
    }
    best
}
