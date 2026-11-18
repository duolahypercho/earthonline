//! `footprintFrame` and its polygon helpers (src/world/buildings/authored/kit/frame.js).

/// Outward projection allowance for body parts (m).
pub const DEFAULT_PROJECTION_ALLOWANCE: f64 = 0.45;
/// Outward allowance for canopies, awnings, blade signs and fire escapes (m).
pub const FITTING_PROJECTION_ALLOWANCE: f64 = 1.8;
/// Outward allowance handed to a party wall (m).
pub const PARTY_PROJECTION_ALLOWANCE: f64 = 0.05;
const PARTY_SAMPLES: [f64; 7] = [0.08, 0.22, 0.36, 0.5, 0.64, 0.78, 0.92];
const PARTY_HITS: usize = 5;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct P2 {
    pub x: f64,
    pub z: f64,
}

impl P2 {
    pub const fn new(x: f64, z: f64) -> Self {
        Self { x, z }
    }
}

/// `{origin:[x,z], u:[x,z], n:[x,z]}`: an edge (or plan) basis for boxes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Basis {
    pub origin: [f64; 2],
    pub u: [f64; 2],
    pub n: [f64; 2],
}

impl Basis {
    /// World (x, z) of the basis point (s along u, w along n).
    pub fn at(&self, s: f64, w: f64) -> (f64, f64) {
        (
            self.origin[0] + self.u[0] * s + self.n[0] * w,
            self.origin[1] + self.u[1] * s + self.n[1] * w,
        )
    }
}

#[derive(Clone, Debug)]
pub struct Edge {
    pub index: usize,
    pub start: P2,
    pub end: P2,
    pub midpoint: P2,
    pub length: f64,
    pub u: P2,
    pub n: P2,
    pub rotation_y: f64,
    /// 'front' | 'front-secondary' | 'left' | 'right' | 'back'
    pub role: &'static str,
    pub party: bool,
    /// Roof line of the tallest burying neighbour (`f64::INFINITY` when unknown).
    pub party_top: Option<f64>,
    pub allowance: f64,
    pub convex_start: bool,
    pub convex_end: bool,
    pub next_index: usize,
    pub prev_index: usize,
    pub basis: Basis,
    /// `edge.lotLine = { open }` (street-front.js markLotLineWalls).
    pub lot_line: Option<u32>,
    /// `edge.streetReturn` (street-front.js promoteStreetReturns).
    pub street_return: bool,
}

#[derive(Clone, Debug, Default)]
pub struct PlanBounds {
    pub min_x: f64,
    pub max_x: f64,
    pub min_z: f64,
    pub max_z: f64,
}

/// A row house's regularised street wall (row-house-front.js), as the frame publishes it.
#[derive(Clone, Debug, Default)]
pub struct RowHouseInfo {
    pub bays: serde_json::Value,
    pub notch: serde_json::Value,
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub polygon: Vec<P2>,
    pub flipped: bool,
    pub edges: Vec<Edge>,
    pub front_edge_index: usize,
    pub front: P2,
    pub right: P2,
    pub heading: f64,
    pub center: P2,
    pub centroid: P2,
    pub width: f64,
    pub depth: f64,
    pub height: Option<f64>,
    pub area: f64,
    pub perimeter: f64,
    pub allowance: f64,
    pub plan_bounds: PlanBounds,
    pub basis: Basis,
    /// A frame rebuilt from a regularised outline keeps the source footprint here.
    pub source_polygon: Option<Vec<P2>>,
    pub row_house: Option<RowHouseInfo>,
    /// 'street' | 'service' | 'footprint' (archetype frames only).
    pub front_source: &'static str,
    pub front_distance: Option<f64>,
    pub front_occluded: Option<bool>,
}

impl Frame {
    pub fn to_world(&self, px: f64, pz: f64) -> P2 {
        P2::new(
            self.center.x + self.right.x * px + self.front.x * pz,
            self.center.z + self.right.z * px + self.front.z * pz,
        )
    }
    pub fn to_plan(&self, x: f64, z: f64) -> P2 {
        let dx = x - self.center.x;
        let dz = z - self.center.z;
        P2::new(dx * self.right.x + dz * self.right.z, dx * self.front.x + dz * self.front.z)
    }
    pub fn contains_point(&self, x: f64, z: f64) -> bool {
        point_in_polygon(x, z, &self.polygon)
    }
    pub fn signed_distance(&self, x: f64, z: f64) -> f64 {
        signed_distance_to_footprint(x, z, &self.polygon)
    }
    pub fn edge_for_role(&self, role: &str) -> Option<&Edge> {
        self.edges.iter().find(|e| e.role == role)
    }
    pub fn edges_for_role(&self, role: &str) -> Vec<&Edge> {
        self.edges.iter().filter(|e| e.role == role).collect()
    }
}

/// Copy the finite vertices, dropping duplicates and a closing repeat.
pub fn clean_polygon(polygon: &[P2]) -> Vec<P2> {
    let mut out: Vec<P2> = Vec::with_capacity(polygon.len());
    for p in polygon {
        if !p.x.is_finite() || !p.z.is_finite() {
            continue;
        }
        if let Some(prev) = out.last() {
            if (prev.x - p.x).hypot(prev.z - p.z) < 1e-6 {
                continue;
            }
        }
        out.push(*p);
    }
    if out.len() > 1 {
        let (f, l) = (out[0], out[out.len() - 1]);
        if (f.x - l.x).hypot(f.z - l.z) < 1e-6 {
            out.pop();
        }
    }
    out
}

pub fn polygon_signed_area(polygon: &[P2]) -> f64 {
    let n = polygon.len();
    let mut sum = 0.0;
    for i in 0..n {
        let a = polygon[i];
        let b = polygon[(i + 1) % n];
        sum += a.x * b.z - b.x * a.z;
    }
    sum / 2.0
}

pub fn point_in_polygon(x: f64, z: f64, polygon: &[P2]) -> bool {
    let n = polygon.len();
    if n == 0 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let a = polygon[i];
        let b = polygon[j];
        if (a.z > z) != (b.z > z) {
            let cross = ((b.x - a.x) * (z - a.z)) / (b.z - a.z) + a.x;
            if x < cross {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

pub fn distance_to_outline(x: f64, z: f64, polygon: &[P2]) -> f64 {
    let n = polygon.len();
    let mut best = f64::INFINITY;
    for i in 0..n {
        let a = polygon[i];
        let b = polygon[(i + 1) % n];
        let dx = b.x - a.x;
        let dz = b.z - a.z;
        let len2 = dx * dx + dz * dz;
        let mut t = if len2 > 0.0 { ((x - a.x) * dx + (z - a.z) * dz) / len2 } else { 0.0 };
        t = t.clamp(0.0, 1.0);
        let d = (x - (a.x + dx * t)).hypot(z - (a.z + dz * t));
        if d < best {
            best = d;
        }
    }
    best
}

pub fn signed_distance_to_footprint(x: f64, z: f64, polygon: &[P2]) -> f64 {
    let d = distance_to_outline(x, z, polygon);
    if point_in_polygon(x, z, polygon) { -d } else { d }
}

pub fn rotation_y_for_direction(u: P2) -> f64 {
    (-u.z).atan2(u.x)
}

/// A neighbour for the party-wall probe: its footprint and height (None: unknown).
pub struct Neighbour<'a> {
    pub polygon: &'a [P2],
    pub height: Option<f64>,
}

#[derive(Default)]
pub struct FrameOptions<'a> {
    pub allowance: Option<f64>,
    pub neighbours: &'a [Neighbour<'a>],
    pub height: Option<f64>,
}

/// `footprintFrame(polygon, frontEdgeIndex, options)`.
pub fn footprint_frame(polygon: &[P2], front_edge_index: usize, options: &FrameOptions) -> Option<Frame> {
    let clean = clean_polygon(polygon);
    if clean.len() < 3 {
        return None;
    }
    let allowance = options.allowance.filter(|a| a.is_finite()).unwrap_or(DEFAULT_PROJECTION_ALLOWANCE);
    let count = clean.len();
    let area = polygon_signed_area(&clean);
    if !(area.abs() > 1e-3) {
        return None;
    }
    let flipped = {
        let mut longest = 0usize;
        for i in 1..count {
            let a = clean[i];
            let b = clean[(i + 1) % count];
            let l0 = (clean[(longest + 1) % count].x - clean[longest].x).hypot(clean[(longest + 1) % count].z - clean[longest].z);
            if (b.x - a.x).hypot(b.z - a.z) > l0 {
                longest = i;
            }
        }
        let a = clean[longest];
        let b = clean[(longest + 1) % count];
        let len = (b.x - a.x).hypot(b.z - a.z);
        let ux = (b.x - a.x) / len;
        let uz = (b.z - a.z) / len;
        let mx = (a.x + b.x) / 2.0 - uz * 0.05;
        let mz = (a.z + b.z) / 2.0 + ux * 0.05;
        point_in_polygon(mx, mz, &clean)
    };
    let mut cx = 0.0;
    let mut cz = 0.0;
    for p in &clean {
        cx += p.x / count as f64;
        cz += p.z / count as f64;
    }
    let centroid = P2::new(cx, cz);
    let mut edges: Vec<Edge> = Vec::with_capacity(count);
    for i in 0..count {
        let a = clean[i];
        let b = clean[(i + 1) % count];
        let (start, end) = if flipped { (b, a) } else { (a, b) };
        let dx = end.x - start.x;
        let dz = end.z - start.z;
        let length = dx.hypot(dz);
        let u = P2::new(dx / length, dz / length);
        let n = P2::new(-u.z, u.x);
        edges.push(Edge {
            index: i,
            start,
            end,
            midpoint: P2::new((start.x + end.x) / 2.0, (start.z + end.z) / 2.0),
            length,
            u,
            n,
            rotation_y: rotation_y_for_direction(u),
            role: "left",
            party: false,
            party_top: None,
            allowance,
            convex_start: true,
            convex_end: true,
            next_index: 0,
            prev_index: 0,
            basis: Basis { origin: [start.x, start.z], u: [u.x, u.z], n: [n.x, n.z] },
            lot_line: None,
            street_return: false,
        });
    }
    for i in 0..count {
        let next_i = if flipped { (i + count - 1) % count } else { (i + 1) % count };
        let prev_i = if flipped { (i + 1) % count } else { (i + count - 1) % count };
        let (nu, pu) = (edges[next_i].u, edges[prev_i].u);
        let e = &mut edges[i];
        e.convex_end = nu.x * e.n.x + nu.z * e.n.z < 1e-6;
        e.convex_start = pu.x * e.n.x + pu.z * e.n.z > -1e-6;
        e.next_index = next_i;
        e.prev_index = prev_i;
    }
    let front_index = front_edge_index.min(count - 1);
    let front = edges[front_index].n;
    let right = P2::new(front.z, -front.x);
    let heading = front.x.atan2(front.z);
    for e in edges.iter_mut() {
        let df = e.n.x * front.x + e.n.z * front.z;
        let dr = e.n.x * right.x + e.n.z * right.z;
        e.role = if e.index == front_index {
            "front"
        } else if df < -0.5 {
            "back"
        } else if df > 0.5 {
            "front-secondary"
        } else if dr > 0.0 {
            "right"
        } else {
            "left"
        };
    }
    // Party-wall probe.
    for e in edges.iter_mut() {
        let mut hits = 0;
        let mut top = 0.0f64;
        let probe = allowance + 0.05;
        for t in PARTY_SAMPLES {
            let s = e.length * t;
            let x = e.start.x + e.u.x * s + e.n.x * probe;
            let z = e.start.z + e.u.z * s + e.n.z * probe;
            let mut buried = false;
            for nb in options.neighbours {
                if nb.polygon.len() < 3 || !point_in_polygon(x, z, nb.polygon) {
                    continue;
                }
                buried = true;
                let h = nb.height.filter(|h| h.is_finite() && *h > 0.0).unwrap_or(f64::INFINITY);
                top = top.max(h);
            }
            if buried {
                hits += 1;
            }
        }
        if hits >= PARTY_HITS {
            e.party = true;
            e.party_top = Some(top);
            e.allowance = allowance.min(PARTY_PROJECTION_ALLOWANCE);
        }
    }
    let (mut min_x, mut max_x, mut min_z, mut max_z) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
    for p in &clean {
        let px = p.x * right.x + p.z * right.z;
        let pz = p.x * front.x + p.z * front.z;
        min_x = min_x.min(px);
        max_x = max_x.max(px);
        min_z = min_z.min(pz);
        max_z = max_z.max(pz);
    }
    let ccx = (min_x + max_x) / 2.0;
    let ccz = (min_z + max_z) / 2.0;
    let center = P2::new(right.x * ccx + front.x * ccz, right.z * ccx + front.z * ccz);
    let width = max_x - min_x;
    let depth = max_z - min_z;
    let perimeter = edges.iter().map(|e| e.length).sum();
    Some(Frame {
        polygon: clean,
        flipped,
        edges,
        front_edge_index: front_index,
        front,
        right,
        heading,
        center,
        centroid,
        width,
        depth,
        height: options.height.filter(|h| h.is_finite()),
        area: area.abs(),
        perimeter,
        allowance,
        plan_bounds: PlanBounds { min_x: -width / 2.0, max_x: width / 2.0, min_z: -depth / 2.0, max_z: depth / 2.0 },
        basis: Basis { origin: [center.x, center.z], u: [right.x, right.z], n: [front.x, front.z] },
        source_polygon: None,
        row_house: None,
        front_source: "footprint",
        front_distance: None,
        front_occluded: None,
    })
}

/// `pointBuriedInNeighbours`.
pub fn point_buried_in_neighbours(x: f64, z: f64, neighbours: &[Neighbour]) -> bool {
    neighbours.iter().any(|n| n.polygon.len() >= 3 && point_in_polygon(x, z, n.polygon))
}
