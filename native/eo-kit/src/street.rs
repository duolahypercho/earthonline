//! Which edge faces the street, street returns, lot-line walls
//! (src/world/buildings/archetypes/street-front.js).

use crate::frame::{Edge, Frame, P2};
use std::collections::HashMap;

pub const STREET_FRONT_REACH_M: f64 = 45.0;
const SERVICE_PENALTY_M: f64 = 30.0;
const MIN_FRONT_EDGE_M: f64 = 2.5;
const MIN_BAY_FRONT_EDGE_M: f64 = 1.2;
const MIN_PARALLEL: f64 = 0.8;
const CELL_M: f64 = 60.0;
const STREET_CLASSES: &[&str] =
    &["primary", "primary_link", "secondary", "secondary_link", "tertiary", "tertiary_link", "residential", "unclassified", "living_street", "trunk", "trunk_link"];
const SERVICE_CLASSES: &[&str] = &["service", "pedestrian"];
const ARTERIAL_CLASSES: &[&str] = &["primary", "primary_link", "secondary", "secondary_link", "tertiary", "tertiary_link", "trunk", "trunk_link"];
const LOCAL_STREET_PENALTY_M: f64 = 12.0;
const MIN_ARTERIAL_FRONT_M: f64 = 6.0;
const OCCLUDED_FRONT_PENALTY_M: f64 = 60.0;
pub const STREET_RETURN_REACH_M: f64 = 20.0;
pub const STREET_RETURN_BLIND_REACH_M: f64 = 14.0;
pub const SHOP_RETURN_REACH_M: f64 = 34.0;
pub const MIN_STREET_RETURN_M: f64 = 4.0;
pub const LOT_LINE_PROBE_M: f64 = 0.6;
pub const LOT_LINE_MIN_EXPOSED_M: f64 = 3.2;
const LOT_LINE_SAMPLES: [f64; 7] = [0.07, 0.21, 0.36, 0.5, 0.64, 0.79, 0.93];
const OCCLUSION_CELL: f64 = 40.0;

fn class_rank(highway: &str) -> i32 {
    if STREET_CLASSES.contains(&highway) {
        0
    } else if SERVICE_CLASSES.contains(&highway) {
        1
    } else {
        -1
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Seg {
    pub ax: f64,
    pub az: f64,
    pub ux: f64,
    pub uz: f64,
    pub len: f64,
    pub rank: i32,
    pub local: bool,
}

/// `buildArchetypeStreetIndex` over raw extract ways.
#[derive(Default)]
pub struct StreetIndex {
    pub segs: Vec<Seg>,
    pub cells: HashMap<(i64, i64), Vec<u32>>,
}

impl StreetIndex {
    /// Ways as `(highway, points)` in the extract's order.
    pub fn build<'a>(ways: impl IntoIterator<Item = (&'a str, &'a [[f64; 2]])>) -> Self {
        let mut idx = StreetIndex::default();
        for (highway, points) in ways {
            let rank = class_rank(highway);
            if rank < 0 {
                continue;
            }
            let local = rank == 0 && !ARTERIAL_CLASSES.contains(&highway);
            for w in points.windows(2) {
                let (ax, az, bx, bz) = (w[0][0], w[0][1], w[1][0], w[1][1]);
                let len = (bx - ax).hypot(bz - az);
                if !(len > 0.5) {
                    continue;
                }
                let id = idx.segs.len() as u32;
                idx.segs.push(Seg { ax, az, ux: (bx - ax) / len, uz: (bz - az) / len, len, rank, local });
                let x0 = (ax.min(bx) / CELL_M).floor() as i64;
                let x1 = (ax.max(bx) / CELL_M).floor() as i64;
                let z0 = (az.min(bz) / CELL_M).floor() as i64;
                let z1 = (az.max(bz) / CELL_M).floor() as i64;
                for x in x0..=x1 {
                    for z in z0..=z1 {
                        idx.cells.entry((x, z)).or_default().push(id);
                    }
                }
            }
        }
        idx
    }

    pub fn count(&self) -> usize {
        self.segs.len()
    }

    /// Segments in the cells around (x, z), each once, in first-seen order
    /// (the JS collects a Set while walking the cells).
    fn nearby(&self, x: f64, z: f64, reach: f64) -> Vec<u32> {
        let x0 = ((x - reach) / CELL_M).floor() as i64;
        let x1 = ((x + reach) / CELL_M).floor() as i64;
        let z0 = ((z - reach) / CELL_M).floor() as i64;
        let z1 = ((z + reach) / CELL_M).floor() as i64;
        let mut out: Vec<u32> = vec![];
        let mut seen = std::collections::HashSet::new();
        for cx in x0..=x1 {
            for cz in z0..=z1 {
                if let Some(list) = self.cells.get(&(cx, cz)) {
                    for &s in list {
                        if seen.insert(s) {
                            out.push(s);
                        }
                    }
                }
            }
        }
        out
    }
}

#[derive(Clone, Copy, Debug)]
pub struct StreetScore {
    pub score: f64,
    pub distance: f64,
    pub rank: i32,
    pub local: bool,
}

/// `edgeStreetScore(edge, index, minLength)`.
pub fn edge_street_score(edge: &Edge, index: &StreetIndex, min_length: f64) -> Option<StreetScore> {
    if edge.length < min_length {
        return None;
    }
    let (mx, mz) = (edge.midpoint.x, edge.midpoint.z);
    let mut best: Option<StreetScore> = None;
    for s in index.nearby(mx, mz, STREET_FRONT_REACH_M) {
        let seg = &index.segs[s as usize];
        let parallel = (seg.ux * edge.u.x + seg.uz * edge.u.z).abs();
        if parallel < MIN_PARALLEL {
            continue;
        }
        let t = ((mx - seg.ax) * seg.ux + (mz - seg.az) * seg.uz).clamp(0.0, seg.len);
        let (px, pz) = (seg.ax + seg.ux * t, seg.az + seg.uz * t);
        let (dx, dz) = (px - mx, pz - mz);
        let outward = dx * edge.n.x + dz * edge.n.z;
        if outward <= 0.5 {
            continue;
        }
        let distance = dx.hypot(dz);
        if distance > STREET_FRONT_REACH_M {
            continue;
        }
        if outward < 0.7 * distance {
            continue;
        }
        let score = distance + seg.rank as f64 * SERVICE_PENALTY_M;
        if best.map(|b| score < b.score).unwrap_or(true) {
            best = Some(StreetScore { score, distance, rank: seg.rank, local: seg.local });
        }
    }
    best
}

/// `chooseFootprintFrontEdge(frame)`.
pub fn choose_footprint_front_edge(frame: &Frame) -> usize {
    let edges = &frame.edges;
    let mut longest = &edges[0];
    for e in edges {
        if e.length > longest.length {
            longest = e;
        }
    }
    let (ux, uz) = (longest.u.x, longest.u.z);
    let (mut min_u, mut max_u, mut min_v, mut max_v) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
    for p in &frame.polygon {
        let u = p.x * ux + p.z * uz;
        let v = -p.x * uz + p.z * ux;
        min_u = min_u.min(u);
        max_u = max_u.max(u);
        min_v = min_v.min(v);
        max_v = max_v.max(v);
    }
    let short_along_u = (max_u - min_u) < (max_v - min_v);
    let (ax, az) = if short_along_u { (ux, uz) } else { (-uz, ux) };
    let mut pick: Option<&Edge> = None;
    for e in edges {
        if e.length < MIN_FRONT_EDGE_M {
            continue;
        }
        if (e.u.x * ax + e.u.z * az).abs() < MIN_PARALLEL {
            continue;
        }
        if pick.map(|p| e.length > p.length).unwrap_or(true) {
            pick = Some(e);
        }
    }
    pick.map(|p| p.index).unwrap_or(longest.index)
}

/// One footprint in the occlusion index.
pub struct Shape {
    pub id: u32,
    pub polygon: Vec<P2>,
    pub min_x: f64,
    pub max_x: f64,
    pub min_z: f64,
    pub max_z: f64,
    pub height: Option<f64>,
}

/// `buildFootprintOcclusionIndex` over the loaded records.
#[derive(Default)]
pub struct OcclusionIndex {
    pub shapes: Vec<Shape>,
    pub cells: HashMap<(i64, i64), Vec<u32>>,
}

fn inside(x: f64, z: f64, polygon: &[P2]) -> bool {
    let n = polygon.len();
    let mut hit = false;
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (polygon[i], polygon[j]);
        if (a.z > z) != (b.z > z) && x < ((b.x - a.x) * (z - a.z)) / (b.z - a.z) + a.x {
            hit = !hit;
        }
        j = i;
    }
    hit
}

impl OcclusionIndex {
    /// `records`: (id, polygon as the record carries it, height).
    pub fn build(records: impl IntoIterator<Item = (u32, Vec<P2>, f64)>) -> Self {
        let mut idx = OcclusionIndex::default();
        for (id, polygon, height) in records {
            if polygon.len() < 3 {
                continue;
            }
            let (mut x0, mut x1, mut z0, mut z1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
            for p in &polygon {
                x0 = x0.min(p.x);
                x1 = x1.max(p.x);
                z0 = z0.min(p.z);
                z1 = z1.max(p.z);
            }
            if !x0.is_finite() {
                continue;
            }
            let k = idx.shapes.len() as u32;
            for x in (x0 / OCCLUSION_CELL).floor() as i64..=(x1 / OCCLUSION_CELL).floor() as i64 {
                for z in (z0 / OCCLUSION_CELL).floor() as i64..=(z1 / OCCLUSION_CELL).floor() as i64 {
                    idx.cells.entry((x, z)).or_default().push(k);
                }
            }
            idx.shapes.push(Shape { id, polygon, min_x: x0, max_x: x1, min_z: z0, max_z: z1, height: (height.is_finite() && height > 0.0).then_some(height) });
        }
        idx
    }

    pub fn count(&self) -> usize {
        self.shapes.len()
    }

    fn cell(&self, x: f64, z: f64) -> Option<&Vec<u32>> {
        self.cells.get(&((x / OCCLUSION_CELL).floor() as i64, (z / OCCLUSION_CELL).floor() as i64))
    }

    /// Is (x, z) inside any footprint other than `self_id`?
    pub fn occupied(&self, x: f64, z: f64, self_id: u32) -> bool {
        let Some(list) = self.cell(x, z) else { return false };
        for &k in list {
            let s = &self.shapes[k as usize];
            if s.id == self_id {
                continue;
            }
            if x < s.min_x || x > s.max_x || z < s.min_z || z > s.max_z {
                continue;
            }
            if inside(x, z, &s.polygon) {
                return true;
            }
        }
        false
    }

    /// Tallest footprint other than `self_id` covering (x, z): None when open,
    /// `INFINITY` when covered by a footprint of unknown height.
    pub fn cover_top(&self, x: f64, z: f64, self_id: u32) -> Option<f64> {
        let list = self.cell(x, z)?;
        let mut top: Option<f64> = None;
        for &k in list {
            let s = &self.shapes[k as usize];
            if s.id == self_id {
                continue;
            }
            if x < s.min_x || x > s.max_x || z < s.min_z || z > s.max_z {
                continue;
            }
            if !inside(x, z, &s.polygon) {
                continue;
            }
            let h = s.height.unwrap_or(f64::INFINITY);
            top = Some(top.map(|t| t.max(h)).unwrap_or(h));
        }
        top
    }
}

fn return_occluded(edge: &Edge, distance: f64, occ: &OcclusionIndex, self_id: u32) -> bool {
    let reach = (distance - 6.0).max(1.5);
    for t in [0.25, 0.5, 0.75] {
        for d in [1.0, reach * 0.5, reach] {
            let x = edge.start.x + edge.u.x * edge.length * t + edge.n.x * d;
            let z = edge.start.z + edge.u.z * edge.length * t + edge.n.z * d;
            if occ.occupied(x, z, self_id) {
                return true;
            }
        }
    }
    false
}

/// `promoteStreetReturns(frame, index, { occlusion, selfId, shopFront })`.
pub fn promote_street_returns(frame: &mut Frame, index: &StreetIndex, occ: Option<&OcclusionIndex>, self_id: u32, shop_front: bool) -> Vec<usize> {
    let mut promoted = vec![];
    if index.count() == 0 {
        return promoted;
    }
    let proven = occ.map(|o| o.count() > 0).unwrap_or(false);
    let reach = if proven {
        if shop_front { SHOP_RETURN_REACH_M } else { STREET_RETURN_REACH_M }
    } else {
        STREET_RETURN_BLIND_REACH_M
    };
    for i in 0..frame.edges.len() {
        let e = &frame.edges[i];
        if e.role == "front" || e.role == "front-secondary" || e.party || !(e.length >= MIN_STREET_RETURN_M) {
            continue;
        }
        let Some(score) = edge_street_score(e, index, MIN_FRONT_EDGE_M) else { continue };
        if score.rank != 0 || score.distance > reach {
            continue;
        }
        if let Some(o) = occ.filter(|o| o.count() > 0) {
            if return_occluded(e, score.distance, o, self_id) {
                continue;
            }
        }
        let e = &mut frame.edges[i];
        e.role = "front-secondary";
        e.street_return = true;
        promoted.push(e.index);
    }
    promoted
}

/// `markLotLineWalls(frame, occlusion, { selfId, height })`.
pub fn mark_lot_line_walls(frame: &mut Frame, occ: Option<&OcclusionIndex>, self_id: u32, height: f64) -> Vec<usize> {
    let mut marked = vec![];
    let Some(occ) = occ.filter(|o| o.count() > 0) else { return marked };
    if !(height >= LOT_LINE_MIN_EXPOSED_M) {
        return marked;
    }
    for e in frame.edges.iter_mut() {
        if e.role != "left" && e.role != "right" {
            continue;
        }
        if e.street_return || !(e.length >= MIN_STREET_RETURN_M) {
            continue;
        }
        let mut open = 0;
        for t in LOT_LINE_SAMPLES {
            let x = e.start.x + e.u.x * e.length * t + e.n.x * LOT_LINE_PROBE_M;
            let z = e.start.z + e.u.z * e.length * t + e.n.z * LOT_LINE_PROBE_M;
            let cover = occ.cover_top(x, z, self_id);
            if height - cover.unwrap_or(0.0) >= LOT_LINE_MIN_EXPOSED_M {
                open += 1;
            }
        }
        if open == 0 {
            continue;
        }
        e.lot_line = Some(open);
        marked.push(e.index);
    }
    marked
}

/// `wallHiddenByNeighbours(edge, occlusion, { selfId, height })`.
pub fn wall_hidden_by_neighbours(edge: &Edge, occ: Option<&OcclusionIndex>, self_id: u32, height: Option<f64>) -> bool {
    let Some(occ) = occ.filter(|o| o.count() > 0) else { return true };
    for t in LOT_LINE_SAMPLES {
        let x = edge.start.x + edge.u.x * edge.length * t + edge.n.x * LOT_LINE_PROBE_M;
        let z = edge.start.z + edge.u.z * edge.length * t + edge.n.z * LOT_LINE_PROBE_M;
        let Some(cover) = occ.cover_top(x, z, self_id) else { return false };
        if let Some(own) = height.filter(|h| h.is_finite()) {
            if own - cover >= LOT_LINE_MIN_EXPOSED_M {
                return false;
            }
        }
    }
    true
}

#[derive(Clone, Debug)]
pub struct FrontChoice {
    pub index: usize,
    pub source: &'static str,
    pub distance: Option<f64>,
    pub occluded: Option<bool>,
}

/// `chooseArchetypeFrontEdge(frame, index, { occlusion, selfId })`.
pub fn choose_archetype_front_edge(frame: &Frame, index: Option<&StreetIndex>, occ: Option<&OcclusionIndex>, self_id: u32) -> FrontChoice {
    if let Some(index) = index.filter(|i| i.count() > 0) {
        let can_occlude = occ.map(|o| o.count() > 0).unwrap_or(false);
        let mut best: Option<(f64, usize, StreetScore, Option<bool>)> = None;
        for min_length in [MIN_FRONT_EDGE_M, MIN_BAY_FRONT_EDGE_M] {
            if best.is_some() {
                break;
            }
            for e in &frame.edges {
                let Some(score) = edge_street_score(e, index, min_length) else { continue };
                let arterial = score.rank == 0 && !score.local && e.length >= MIN_ARTERIAL_FRONT_M;
                let mut penalty = if score.rank == 0 && !arterial { LOCAL_STREET_PENALTY_M } else { 0.0 };
                let occluded = if can_occlude { Some(return_occluded(e, score.distance, occ.unwrap(), self_id)) } else { None };
                if occluded == Some(true) {
                    penalty += OCCLUDED_FRONT_PENALTY_M;
                }
                let ranked = score.score + penalty - e.length.min(30.0) * 0.05;
                if best.map(|b| ranked < b.0).unwrap_or(true) {
                    best = Some((ranked, e.index, score, occluded));
                }
            }
        }
        if let Some((_, i, score, occluded)) = best {
            return FrontChoice { index: i, source: if score.rank == 0 { "street" } else { "service" }, distance: Some(score.distance), occluded };
        }
    }
    FrontChoice { index: choose_footprint_front_edge(frame), source: "footprint", distance: None, occluded: None }
}
