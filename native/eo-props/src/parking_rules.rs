//! props2 parking: the kerb rules a parked car is refused by, ported from the
//! JS game and built once per world (`KerbRules`):
//!
//! * `kerbRestrictionsFor` (src/world/streets/kerb-restrictions.js): red kerb
//!   (`planCurbPaintRuns`: the corner daylighting of each kerb piece, an
//!   observed overlay's red, cut by an overlay's parked kerb), the kerb line
//!   inside another street (`kerbBlockedWindows`), an overlay's empty kerb,
//!   the surveyed hydrants' and bus stops' windows, the turn pockets
//!   (`laneAssignment` on the mapped `turn:lanes`), and the footprint zones
//!   (junction boxes, crossing bands, other carriageways, other footways).
//! * `planVehicleCrossovers` (street-surface-v2.js): the driveway and surface-lot
//!   crossovers cut in the kerb (`stallOnVehicleCrossover`).
//! * `createMappedCrossingTest` (street-city-kerb.js): mapped crossings and ways
//!   closed to cars, with the 20 ft before them.
//! * `buildRunningCarriagewayIndex` (vehicle-presentation.js).
//! * the observed block overlays (src/world/streets/observed/): `kerbParked`,
//!   `kerbBodies`, `kerbCast`, `kerbCastAt`, `kerbPaints`, `kerbClear`, `redKerb`.
//!
//! What the native street contract does not carry (surveyed hydrants and bus
//! stops, lot frontages, ways closed to cars, mapped turn lanes, the overlays)
//! and the JS constants of these rules come from `data/parking-js.json.gz`
//! (the earlier browser version's exporter). Every California value is `caMetric`
//! through `eo_data::CaStandards` (`Standards`).

use crate::{P2, PropsWorld, station_at};
use anyhow::{Context, Result};
use eo_street::plan::{Entry, crossing_band_depth};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Read;

pub const PARKING_JS_GZ: &[u8] = include_bytes!("../data/parking-js.json.gz");

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LotFrontage {
    pub lot: String,
    pub host: String,
    pub x: f64,
    pub z: f64,
    pub dir_x: f64,
    pub dir_z: f64,
    pub width: f64,
}

#[derive(Deserialize, Clone, Debug)]
pub struct AxisPt {
    pub x: f64,
    pub z: f64,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Axis {
    pub from: AxisPt,
    pub to: AxisPt,
}

/// One observed block overlay's kerb records (the JS objects, as exported).
#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Overlay {
    pub id: String,
    pub street_ids: Vec<String>,
    pub axis: Axis,
    #[serde(default)]
    pub red_kerb: Vec<Value>,
    #[serde(default)]
    pub kerb_clear: Vec<Value>,
    #[serde(default)]
    pub kerb_parked: Vec<Value>,
    #[serde(default)]
    pub kerb_bodies: Vec<Value>,
    #[serde(default)]
    pub kerb_cast: Vec<Value>,
    #[serde(default)]
    pub kerb_cast_at: Vec<Value>,
    #[serde(default)]
    pub kerb_paints: Vec<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawParking {
    sources: BTreeMap<String, String>,
    consts: Value,
    hydrants: Vec<f64>,
    bus_stops: Vec<f64>,
    lot_frontages: Vec<LotFrontage>,
    closed_ways: Vec<String>,
    turn_lanes: HashMap<String, (Option<String>, Option<String>)>,
    overlays: Vec<Overlay>,
}

/// `data/parking-js.json.gz`.
pub struct ParkingJs {
    pub sources: BTreeMap<String, String>,
    pub consts: Value,
    pub hydrants: Vec<P2>,
    pub bus_stops: Vec<P2>,
    pub lot_frontages: Vec<LotFrontage>,
    pub closed_ways: HashSet<String>,
    /// `sf-street-<way>` -> (turnLanesForward, turnLanesBackward).
    pub turn_lanes: HashMap<String, (Option<String>, Option<String>)>,
    pub overlays: Vec<Overlay>,
}

impl ParkingJs {
    pub fn load() -> Result<Self> {
        let mut s = String::new();
        flate2::read::GzDecoder::new(PARKING_JS_GZ).read_to_string(&mut s).context("parking-js.json.gz")?;
        let mut raw: RawParking = serde_json::from_str(&s).context("parking-js.json")?;
        // groundscale (the street size, 1.25 by default): the surveyed hydrants, stops,
        // lot frontages and the observed-block overlays at their scaled positions
        // (a frontage's curb-cut width and a car's reach stay real).
        let gs = eo_data::groundscale::scale();
        if gs != 1.0 {
            for f in &mut raw.lot_frontages {
                f.x *= gs;
                f.z *= gs;
            }
            for o in &mut raw.overlays {
                for p in [&mut o.axis.from, &mut o.axis.to] {
                    p.x *= gs;
                    p.z *= gs;
                }
                for list in [&mut o.red_kerb, &mut o.kerb_clear, &mut o.kerb_parked, &mut o.kerb_bodies, &mut o.kerb_cast, &mut o.kerb_cast_at] {
                    for e in list.iter_mut() {
                        eo_data::groundscale::scale_json_keys(e, eo_data::groundscale::POSITION_KEYS);
                    }
                }
            }
        }
        let pts = |v: &[f64]| v.chunks_exact(2).map(|c| [c[0] * gs, c[1] * gs]).collect::<Vec<P2>>(); // * gs
        Ok(Self {
            sources: raw.sources,
            hydrants: pts(&raw.hydrants),
            bus_stops: pts(&raw.bus_stops),
            consts: raw.consts,
            lot_frontages: raw.lot_frontages,
            closed_ways: raw.closed_ways.into_iter().collect(),
            turn_lanes: raw.turn_lanes,
            overlays: raw.overlays,
        })
    }

    /// `consts.<group>.<key>` of the export.
    pub fn num(&self, group: &str, key: &str) -> f64 {
        self.consts[group][key].as_f64().unwrap_or_else(|| panic!("parking-js.json: consts.{group}.{key} is not a number"))
    }

    pub fn num1(&self, key: &str) -> f64 {
        self.consts[key].as_f64().unwrap_or_else(|| panic!("parking-js.json: consts.{key} is not a number"))
    }
}

/// A reserved station window on one kerb.
#[derive(Clone, Debug)]
pub struct Win {
    pub lo: f64,
    pub hi: f64,
    pub reason: &'static str,
}

// ---------------------------------------------------------------------------
// geometry
// ---------------------------------------------------------------------------

/// Separating-axis overlap depth of two convex polygons (`lapDepth`), <= 0 apart.
pub fn lap_depth(a: &[P2], b: &[P2]) -> f64 {
    let mut depth = f64::INFINITY;
    for poly in [a, b] {
        for i in 0..poly.len() {
            let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
            let (mut ax, mut az) = (-(q[1] - p[1]), q[0] - p[0]);
            let l = ax.hypot(az);
            if !(l > 1e-9) {
                continue;
            }
            ax /= l;
            az /= l;
            let (mut min_a, mut max_a, mut min_b, mut max_b) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
            for v in a {
                let d = v[0] * ax + v[1] * az;
                min_a = min_a.min(d);
                max_a = max_a.max(d);
            }
            for v in b {
                let d = v[0] * ax + v[1] * az;
                min_b = min_b.min(d);
                max_b = max_b.max(d);
            }
            let ov = max_a.min(max_b) - min_a.max(min_b);
            if ov < depth {
                depth = ov;
            }
            if depth <= 0.0 {
                return depth;
            }
        }
    }
    depth
}

/// `quadsOverlap` (vehicle-presentation.js): touching counts.
/// Plan distance from `p` to a convex quad (0 inside it).
pub fn quad_distance(q: &[P2; 4], p: P2) -> f64 {
    let mut inside = true;
    let mut best = f64::INFINITY;
    for i in 0..4 {
        let (a, b) = (q[i], q[(i + 1) % 4]);
        let (ex, ez) = (b[0] - a[0], b[1] - a[1]);
        let l2 = (ex * ex + ez * ez).max(1e-12);
        let t = (((p[0] - a[0]) * ex + (p[1] - a[1]) * ez) / l2).clamp(0.0, 1.0);
        best = best.min((p[0] - a[0] - ex * t).hypot(p[1] - a[1] - ez * t));
        // (the corners wind one way round: inside is on one side of every edge.)
        let side = ex * (p[1] - a[1]) - ez * (p[0] - a[0]);
        let first = (q[1][0] - q[0][0]) * (q[2][1] - q[0][1]) - (q[1][1] - q[0][1]) * (q[2][0] - q[0][0]);
        if side * first < 0.0 {
            inside = false;
        }
    }
    if inside { 0.0 } else { best }
}

pub fn quads_overlap(a: &[P2], b: &[P2]) -> bool {
    for poly in [a, b] {
        for i in 0..poly.len() {
            let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
            let (ax, az) = (-(q[1] - p[1]), q[0] - p[0]);
            let (mut min_a, mut max_a, mut min_b, mut max_b) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
            for v in a {
                let d = v[0] * ax + v[1] * az;
                min_a = min_a.min(d);
                max_a = max_a.max(d);
            }
            for v in b {
                let d = v[0] * ax + v[1] * az;
                min_b = min_b.min(d);
                max_b = max_b.max(d);
            }
            if max_a < min_b || max_b < min_a {
                return false;
            }
        }
    }
    true
}

/// `vehicleFootprint(spec, x, z, yaw, margin)`: front left, front right, rear right, rear left.
pub fn vehicle_footprint(length: f64, width: f64, x: f64, z: f64, yaw: f64, margin: f64) -> [P2; 4] {
    let (c, s) = (yaw.cos(), yaw.sin());
    let (hl, hw) = (length / 2.0 + margin, width / 2.0 + margin);
    let mut out = [[0.0; 2]; 4];
    for (k, (sw, sl)) in [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)].into_iter().enumerate() {
        let (lx, lz) = (sw * hw, sl * hl);
        out[k] = [x + lx * c + lz * s, z - lx * s + lz * c];
    }
    out
}

fn hull(pts: &[P2]) -> Vec<P2> {
    let mut p = pts.to_vec();
    p.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap().then(a[1].partial_cmp(&b[1]).unwrap()));
    if p.len() < 3 {
        return p;
    }
    let cross = |o: P2, a: P2, b: P2| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let mut lo: Vec<P2> = vec![];
    for &q in &p {
        while lo.len() >= 2 && cross(lo[lo.len() - 2], lo[lo.len() - 1], q) <= 0.0 {
            lo.pop();
        }
        lo.push(q);
    }
    let mut up: Vec<P2> = vec![];
    for &q in p.iter().rev() {
        while up.len() >= 2 && cross(up[up.len() - 2], up[up.len() - 1], q) <= 0.0 {
            up.pop();
        }
        up.push(q);
    }
    up.pop();
    lo.pop();
    lo.extend(up);
    lo
}

/// `bandQuads(segment, s0, s1, v0, v1)`.
fn band_quads(e: &Entry, s0: f64, s1: f64, v0: f64, v1: f64) -> Vec<[P2; 4]> {
    let (pts, cum) = (&e.points, &e.cum);
    let mut out = vec![];
    for i in 1..pts.len() {
        let a = cum[i - 1].max(s0);
        let b = cum[i].min(s1);
        let len = cum[i] - cum[i - 1];
        if !(b - a > 0.01) || !(len > 1e-6) {
            continue;
        }
        let (tx, tz) = ((pts[i][0] - pts[i - 1][0]) / len, (pts[i][1] - pts[i - 1][1]) / len);
        let (ax, az) = (pts[i - 1][0] + tx * (a - cum[i - 1]), pts[i - 1][1] + tz * (a - cum[i - 1]));
        let (bx, bz) = (pts[i - 1][0] + tx * (b - cum[i - 1]), pts[i - 1][1] + tz * (b - cum[i - 1]));
        let (nx, nz) = (-tz, tx);
        out.push([[ax + nx * v0, az + nz * v0], [bx + nx * v0, bz + nz * v0], [bx + nx * v1, bz + nz * v1], [ax + nx * v1, az + nz * v1]]);
    }
    out
}

/// `project(segment, x, z)` of kerb-restrictions.js: (distance, station, signed lateral +n).
fn project_kr(e: &Entry, x: f64, z: f64) -> Option<(f64, f64, f64)> {
    let (pts, cum) = (&e.points, &e.cum);
    let mut best: Option<(f64, f64, f64)> = None;
    for i in 1..pts.len() {
        let (ax, az) = (pts[i - 1][0], pts[i - 1][1]);
        let (dx, dz) = (pts[i][0] - ax, pts[i][1] - az);
        let l2 = dx * dx + dz * dz;
        if !(l2 > 1e-12) {
            continue;
        }
        let t = (((x - ax) * dx + (z - az) * dz) / l2).clamp(0.0, 1.0);
        let d = (x - (ax + dx * t)).hypot(z - (az + dz * t));
        if best.is_none_or(|b| d < b.0) {
            let l = l2.sqrt();
            best = Some((d, cum[i - 1] + t * l, ((x - ax) * -dz + (z - az) * dx) / l));
        }
    }
    best
}

fn edge_dir(points: &[P2], i: usize) -> P2 {
    let (a, b) = (points[i], points[i + 1]);
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let len = dx.hypot(dz);
    let len = if len == 0.0 { 1.0 } else { len };
    [dx / len, dz / len]
}

type Cell = (i32, i32);

fn cells_of(x0: f64, x1: f64, z0: f64, z1: f64, cell: f64) -> impl Iterator<Item = Cell> {
    let (gx0, gx1, gz0, gz1) = ((x0 / cell).floor() as i32, (x1 / cell).floor() as i32, (z0 / cell).floor() as i32, (z1 / cell).floor() as i32);
    (gx0..=gx1).flat_map(move |gx| (gz0..=gz1).map(move |gz| (gx, gz)))
}

/// `makeGrid(cell)` of kerb-restrictions.js: polygons with a record, `first` lapped deeper than `min`.
#[derive(Default)]
pub struct PolyGrid {
    cell: f64,
    cells: HashMap<Cell, Vec<u32>>,
    polys: Vec<Vec<P2>>,
    /// (entry, street key index) per record; u32::MAX = none.
    recs: Vec<(u32, u32)>,
}

impl PolyGrid {
    fn new(cell: f64) -> Self {
        Self { cell, ..Default::default() }
    }
    fn range(&self, poly: &[P2]) -> (f64, f64, f64, f64) {
        let (mut x0, mut x1, mut z0, mut z1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
        for v in poly {
            x0 = x0.min(v[0]);
            x1 = x1.max(v[0]);
            z0 = z0.min(v[1]);
            z1 = z1.max(v[1]);
        }
        (x0, x1, z0, z1)
    }
    fn add(&mut self, poly: Vec<P2>, rec: (u32, u32)) {
        let id = self.polys.len() as u32;
        let (x0, x1, z0, z1) = self.range(&poly);
        for c in cells_of(x0, x1, z0, z1, self.cell) {
            self.cells.entry(c).or_default().push(id);
        }
        self.polys.push(poly);
        self.recs.push(rec);
    }
    fn first(&self, poly: &[P2], min_depth: f64, filter: impl Fn((u32, u32)) -> bool) -> bool {
        let (x0, x1, z0, z1) = self.range(poly);
        let mut seen: HashSet<u32> = HashSet::new();
        for c in cells_of(x0, x1, z0, z1, self.cell) {
            let Some(list) = self.cells.get(&c) else { continue };
            for &id in list {
                if !seen.insert(id) {
                    continue;
                }
                if !filter(self.recs[id as usize]) {
                    continue;
                }
                if lap_depth(poly, &self.polys[id as usize]) > min_depth {
                    return true;
                }
            }
        }
        false
    }
    pub fn len(&self) -> usize {
        self.polys.len()
    }
    pub fn is_empty(&self) -> bool {
        self.polys.is_empty()
    }
}

// ---------------------------------------------------------------------------
// the rules, built once
// ---------------------------------------------------------------------------

/// California and JS values the kerb rules read.
#[derive(Clone, Debug)]
pub struct KerbConsts {
    /// `KERB_ZONE_PLAN.cornerRedM` = `caMetric('no-parking-crosswalk-daylighting').approachSide` (20 ft).
    pub corner_red: f64,
    pub min_run: f64,
    pub alley_half_max: f64,
    pub alley_walk_max: f64,
    pub corner_max_share: f64,
    pub block_step: f64,
    pub block_clear: f64,
    /// `caMetric('no-parking-fire-hydrant')`.
    pub hydrant_clear: f64,
    /// No California length exists (`no-parking-bus-zone` has none): the JS design value.
    pub bus_zone_half: f64,
    pub point_inside: f64,
    pub point_outside: f64,
    /// `caMetric('no-parking-signalized-approach').nearSide`.
    pub turn_pocket: f64,
    pub min_lap: f64,
    pub footway_lap: f64,
    pub crossing_cell: f64,
    pub lane_arrow_min_rank: i32,
    pub x_min_width: f64,
    pub x_max_width: f64,
    pub x_lip_widen: f64,
    pub x_min_angle_deg: f64,
    pub x_mouth_slack: f64,
    pub x_trim_margin: f64,
    pub far_radius: f64,
}

pub struct KerbRules {
    pub js: ParkingJs,
    pub c: KerbConsts,
    /// Per entry: running half (`runningHalf`) and the street key index (`streetId`, else name).
    pub running_half: Vec<f64>,
    street_ix: Vec<u32>,
    street_id_ix: Vec<u32>,
    /// Entries by 32 m cell (`blockIndex`).
    block: HashMap<Cell, Vec<u32>>,
    /// Surveyed points and turn pockets by (entry, side).
    windows: HashMap<(u32, i8), Vec<Win>>,
    pub boxes: PolyGrid,
    pub crossings: PolyGrid,
    pub carriageways: PolyGrid,
    pub footways: PolyGrid,
    /// `buildRunningCarriagewayIndex` (vehicle-presentation.js), cell 16.
    pub running: PolyGrid,
    /// Vehicle crossovers by (entry, side): [lo, hi].
    pub crossovers: HashMap<(u32, i8), Vec<[f64; 2]>>,
    /// Mapped crossing segments by `crossingCellM` cell.
    crossing_lines: HashMap<Cell, Vec<(P2, P2)>>,
    /// Overlay index of each entry (by its roadway's street id).
    pub overlay_of: Vec<Option<u16>>,
    /// Each run planned alone (`parking::plan_entry`'s raw kerb), cached.
    pub raw: std::sync::Mutex<HashMap<u32, std::sync::Arc<Vec<crate::parking::Stall>>>>,
    pub stats: BTreeMap<&'static str, usize>,
    pub build_ms: f64,
}

/// The travel width of a segment (`source.travelWidth`): the asphalt less its bays.
pub fn travel_width(s: &eo_street::city::Segment) -> f64 {
    s.width - s.parking[0] - s.parking[1] - s.cycle[0] - s.cycle[1] - s.shoulder[0] - s.shoulder[1]
}

impl KerbRules {
    pub fn build(w: &PropsWorld) -> Result<Self> {
        let t0 = std::time::Instant::now();
        let js = ParkingJs::load()?;
        let st = &w.std;
        let zp = |k: &str| js.num("kerbZonePlan", k);
        let rr = |k: &str| js.num("kerbRestrictionRules", k);
        let xo = |k: &str| js.num("crossover", k);
        let c = KerbConsts {
            corner_red: st.no_parking_daylighting,
            min_run: zp("minRunM"),
            alley_half_max: zp("alleyHalfMaxM"),
            alley_walk_max: zp("alleyWalkMaxM"),
            corner_max_share: js.num1("cornerMaxShare"),
            block_step: js.num1("blockStepM"),
            block_clear: js.num1("blockClearM"),
            hydrant_clear: st.hydrant_clear,
            bus_zone_half: rr("busZoneHalfM"),
            point_inside: rr("pointKerbInsideM"),
            point_outside: rr("pointKerbOutsideM"),
            turn_pocket: st.signal_approach_near_side,
            min_lap: rr("minLapM"),
            footway_lap: rr("footwayLapM"),
            crossing_cell: js.num1("crossingCellM"),
            lane_arrow_min_rank: js.num1("laneArrowMinClassRank") as i32,
            x_min_width: xo("minWidth"),
            x_max_width: xo("maxWidth"),
            x_lip_widen: xo("lipWiden"),
            x_min_angle_deg: xo("minAngleDeg"),
            x_mouth_slack: xo("mouthSlack"),
            x_trim_margin: xo("trimMargin"),
            far_radius: js.num1("farRadius"),
        };
        let plan = &w.street.plan;
        let o = &w.street.o;
        let segs = &w.street.city.segments;
        let mut stats: BTreeMap<&'static str, usize> = BTreeMap::new();

        // Street identity (`sameStreet`): streetId, else the street name.
        let mut street_keys: HashMap<String, u32> = HashMap::new();
        let mut key_of = |s: &str| -> u32 {
            let n = street_keys.len() as u32;
            *street_keys.entry(s.to_string()).or_insert(n)
        };
        let mut street_ix = vec![];
        let mut street_id_ix = vec![];
        let mut running_half = vec![];
        for e in &plan.entries {
            let s = &segs[e.road];
            street_id_ix.push(s.street_id.as_deref().map(|id| key_of(&format!("id:{id}"))).unwrap_or(u32::MAX));
            street_ix.push(if !s.street_name.is_empty() { key_of(&format!("name:{}", s.street_name)) } else { u32::MAX });
            let travel = travel_width(s);
            running_half.push(if travel.is_finite() && travel > 0.0 { e.half.min(travel / 2.0) } else { (e.half - s.parking[0].max(s.parking[1]).max(0.0)).max(0.0) });
        }

        // blockIndex: every entry by 32 m cell over its spans, reach half + walk + 1.
        let mut block: HashMap<Cell, Vec<u32>> = HashMap::new();
        for (ei, e) in plan.entries.iter().enumerate() {
            // (a structure's leg into a junction, eo-street joins, is not drawn.)
            if e.points.len() < 2 || e.structure_leg.is_some() {
                continue;
            }
            let reach = e.half + e.walks[0].max(e.walks[1]) + 1.0;
            let mut cells: HashSet<Cell> = HashSet::new();
            for i in 1..e.points.len() {
                let (a, b) = (e.points[i - 1], e.points[i]);
                cells.extend(cells_of(a[0].min(b[0]) - reach, a[0].max(b[0]) + reach, a[1].min(b[1]) - reach, a[1].max(b[1]) + reach, 32.0));
            }
            for cc in cells {
                block.entry(cc).or_default().push(ei as u32);
            }
        }
        for v in block.values_mut() {
            v.sort_unstable();
        }

        // Surveyed points (centreGrid, cell 24, reach half + walk + 2): hydrants, then bus stops.
        let mut windows: HashMap<(u32, i8), Vec<Win>> = HashMap::new();
        {
            let mut centre: HashMap<Cell, Vec<u32>> = HashMap::new();
            for (ei, e) in plan.entries.iter().enumerate() {
                if e.points.len() < 2 || e.structure_leg.is_some() {
                    continue;
                }
                let reach = e.half + e.walks[0].max(e.walks[1]) + 2.0;
                let mut cells: HashSet<Cell> = HashSet::new();
                for i in 1..e.points.len() {
                    let (a, b) = (e.points[i - 1], e.points[i]);
                    cells.extend(cells_of(a[0].min(b[0]) - reach, a[0].max(b[0]) + reach, a[1].min(b[1]) - reach, a[1].max(b[1]) + reach, 24.0));
                }
                for cc in cells {
                    centre.entry(cc).or_default().push(ei as u32);
                }
            }
            for list in centre.values_mut() {
                list.sort_unstable();
            }
            for (pts, reach, reason) in [(&js.hydrants, c.hydrant_clear, "hydrant"), (&js.bus_stops, c.bus_zone_half, "bus-stop")] {
                for p in pts.iter() {
                    let Some(cands) = centre.get(&((p[0] / 24.0).floor() as i32, (p[1] / 24.0).floor() as i32)) else { continue };
                    for &ei in cands {
                        let e = &plan.entries[ei as usize];
                        let Some((_, s, lat_s)) = project_kr(e, p[0], p[1]) else { continue };
                        let side: i8 = if lat_s >= 0.0 { 1 } else { -1 };
                        let walk = if side > 0 { e.walks[0] } else { e.walks[1] };
                        let lat = lat_s.abs();
                        if lat < e.half - c.point_inside || lat > e.half + walk + c.point_outside {
                            continue;
                        }
                        if s < -reach || s > e.length + reach {
                            continue;
                        }
                        windows.entry((ei, side)).or_default().push(Win { lo: s - reach, hi: s + reach, reason });
                        *stats.entry("surveyed-assigned").or_default() += 1;
                    }
                }
            }
        }

        // Turn pockets: a signalised approach whose kerb lane the source maps as an exclusive turn.
        for n in &plan.nodes {
            if n.signal.is_none() {
                continue;
            }
            for app in &n.approaches {
                let e = &plan.entries[app.entry];
                if !(app.class_rank >= c.lane_arrow_min_rank && app.flows_toward) {
                    continue;
                }
                let Some(lanes) = mapped_lane_movements(w, &js, n, app) else { continue };
                let count = lanes.len();
                let exclusive = |m: &Option<&'static str>| matches!(m, Some("near") | Some("far"));
                if !lanes.iter().any(|m| m.is_some_and(|m| m.starts_with("through"))) {
                    continue;
                }
                let minus_m: i8 = if app.at_start { -1 } else { 1 };
                let mut kerbs: Vec<i8> = vec![];
                if exclusive(&lanes[0]) {
                    kerbs.push(minus_m);
                }
                if app.oneway && count > 1 && exclusive(&lanes[count - 1]) {
                    kerbs.push(-minus_m);
                }
                if kerbs.is_empty() {
                    continue;
                }
                let stop_line = app.trim + o.crosswalk_clearance + crossing_band_depth(n.max_class_rank, o);
                let reach = stop_line + c.turn_pocket;
                let (lo, hi) = if app.at_start { (-1.0, reach) } else { (e.length - reach, e.length + 1.0) };
                for side in kerbs {
                    windows.entry((app.entry as u32, side)).or_default().push(Win { lo, hi, reason: "turn-pocket" });
                    *stats.entry("turn-pockets").or_default() += 1;
                }
            }
        }

        // Footprint zones.
        let mut boxes = PolyGrid::new(24.0);
        let mut crossings = PolyGrid::new(16.0);
        for n in &plan.nodes {
            if n.approaches.len() < 3 {
                continue;
            }
            let mut pts = vec![n.position];
            for a in &n.approaches {
                let (mx, mz) = (-a.u[1], a.u[0]);
                for sgn in [1.0, -1.0] {
                    pts.push([n.position[0] + a.u[0] * a.trim + mx * a.half * sgn, n.position[1] + a.u[1] * a.trim + mz * a.half * sgn]);
                }
            }
            let h = hull(&pts);
            if h.len() >= 3 {
                boxes.add(h, (u32::MAX, u32::MAX));
            }
            let depth = crossing_band_depth(n.max_class_rank, o);
            for a in &n.approaches {
                let e = &plan.entries[a.entry];
                let b0 = a.trim + o.crosswalk_clearance;
                let b1 = b0 + depth;
                let (lo, hi) = if a.at_start { (b0, b1) } else { (e.length - b1, e.length - b0) };
                for q in band_quads(e, lo.max(0.0), hi.min(e.length), -a.half, a.half) {
                    crossings.add(q.to_vec(), (a.entry as u32, u32::MAX));
                }
            }
        }
        let mut carriageways = PolyGrid::new(16.0);
        let mut footways = PolyGrid::new(16.0);
        let mut running = PolyGrid::new(16.0);
        for (ei, e) in plan.entries.iter().enumerate() {
            if e.structure_leg.is_some() {
                continue;
            }
            let s0 = e.trim_start.max(0.0);
            let s1 = e.length - e.trim_end.max(0.0);
            let run = running_half[ei];
            // buildRunningCarriagewayIndex: every contract street (the native contract carries bays on every segment).
            {
                let s = &segs[e.road];
                let travel = travel_width(s);
                let half = if travel.is_finite() && travel > 0.0 { travel / 2.0 } else { (e.half - s.parking[0].max(s.parking[1])).max(0.0) };
                if half > 0.0 && e.points.len() >= 2 {
                    for q in band_quads(e, s0, s1, -half, half) {
                        running.add(q.to_vec(), (ei as u32, u32::MAX));
                    }
                }
            }
            if !(s1 - s0 > 0.05) {
                continue;
            }
            if run > 0.0 {
                for q in band_quads(e, s0, s1, -run, run) {
                    carriageways.add(q.to_vec(), (ei as u32, street_id_ix[ei]));
                }
            }
            let (wl, wr) = (e.walks[0], e.walks[1]);
            if wl >= o.min_sidewalk_width && wl > 0.0 {
                for q in band_quads(e, s0, s1, e.half, e.half + wl) {
                    footways.add(q.to_vec(), (ei as u32, u32::MAX));
                }
            }
            if wr >= o.min_sidewalk_width && wr > 0.0 {
                for q in band_quads(e, s0, s1, -e.half - wr, -e.half) {
                    footways.add(q.to_vec(), (ei as u32, u32::MAX));
                }
            }
        }

        let crossovers = plan_vehicle_crossovers(w, &js, &c, &mut stats);

        // Mapped crossings and ways closed to cars (`keptClearLines`).
        let mut crossing_lines: HashMap<Cell, Vec<(P2, P2)>> = HashMap::new();
        {
            let mut add_line = |points: &[P2]| {
                for i in 1..points.len() {
                    let (a, b) = (points[i - 1], points[i]);
                    for cc in cells_of(a[0].min(b[0]), a[0].max(b[0]), a[1].min(b[1]), a[1].max(b[1]), c.crossing_cell) {
                        crossing_lines.entry(cc).or_default().push((a, b));
                    }
                }
            };
            for (pts, _) in &w.street.city.crossings {
                add_line(pts);
            }
            for s in segs {
                if s.street_id.as_ref().is_some_and(|id| js.closed_ways.contains(id)) {
                    add_line(&s.points);
                }
            }
        }

        let overlay_of = plan
            .entries
            .iter()
            .map(|e| segs[e.road].street_id.as_ref().and_then(|id| js.overlays.iter().position(|o| o.street_ids.iter().any(|s| s == id)).map(|i| i as u16)))
            .collect();

        stats.insert("junction-boxes", boxes.len());
        stats.insert("crossing-quads", crossings.len());
        stats.insert("carriageway-quads", carriageways.len());
        stats.insert("footway-quads", footways.len());
        stats.insert("running-quads", running.len());
        Ok(Self {
            js,
            c,
            running_half,
            street_ix,
            street_id_ix,
            block,
            windows,
            boxes,
            crossings,
            carriageways,
            footways,
            running,
            crossovers,
            crossing_lines,
            overlay_of,
            raw: std::sync::Mutex::new(HashMap::new()),
            stats,
            build_ms: t0.elapsed().as_secs_f64() * 1e3,
        })
    }

    /// `sameStreet(a, b)`.
    fn same_street(&self, a: usize, b: usize) -> bool {
        if self.street_id_ix[a] != u32::MAX && self.street_id_ix[a] == self.street_id_ix[b] {
            return true;
        }
        self.street_ix[a] != u32::MAX && self.street_ix[a] == self.street_ix[b]
    }

    /// Surveyed point and turn pocket windows of one kerb, in insertion order.
    pub fn point_windows(&self, ei: usize, side: i8) -> &[Win] {
        self.windows.get(&(ei as u32, side)).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// `kerbBlockedWindows(plan, segment, side)`: (lo, hi, corner).
    pub fn blocked_windows(&self, w: &PropsWorld, ei: usize, side: i8) -> Vec<(f64, f64, bool)> {
        let plan = &w.street.plan;
        let segs = &w.street.city.segments;
        let e = &plan.entries[ei];
        if e.points.len() < 2 {
            return vec![];
        }
        let lo = e.trim_start.max(0.0);
        let hi = e.length - e.trim_end.max(0.0);
        let own = [e.points[0], e.points[e.points.len() - 1]];
        let joined = |o: &Entry| {
            for p in [o.points[0], o.points[o.points.len() - 1]] {
                for f in own {
                    if (p[0] - f[0]).hypot(p[1] - f[1]) < 1.5 {
                        return true;
                    }
                }
            }
            false
        };
        let inside_other = |p: P2| -> Option<usize> {
            let cands = self.block.get(&((p[0] / 32.0).floor() as i32, (p[1] / 32.0).floor() as i32))?;
            for &oi in cands {
                let oi = oi as usize;
                if oi == ei {
                    continue;
                }
                let other = &plan.entries[oi];
                // (the JS: a grade-separated way over the kerb does not block it.
                // The plan never holds one: `paveGradeSeparated` is off, so no
                // entry's segment is separated (checked). With `--structures`
                // a deck over the kerb is not an entry either, so it never
                // blocks it; its columns standing in the parking lane do, over
                // their footprint plus the margin: `structure_reason`.)
                if segs[other.road].separated {
                    continue;
                }
                let Some((_, s, lat)) = project_kr(other, p[0], p[1]) else { continue };
                if s <= 0.05 || s >= other.length - 0.05 {
                    continue;
                }
                let same = self.same_street(ei, oi);
                let walk = if lat >= 0.0 { other.walks[0] } else { other.walks[1] };
                let reach = if same { self.running_half[oi] } else { other.half + walk + self.c.block_clear };
                if lat.abs() >= reach {
                    continue;
                }
                if joined(other) {
                    continue;
                }
                return Some(oi);
            }
            None
        };
        let kerb_point = |s: f64| -> Option<P2> {
            let (pts, cum) = (&e.points, &e.cum);
            let mut i = 1;
            while i < pts.len() - 1 && cum[i] < s {
                i += 1;
            }
            let len = cum[i] - cum[i - 1];
            if !(len > 1e-9) {
                return None;
            }
            let (tx, tz) = ((pts[i][0] - pts[i - 1][0]) / len, (pts[i][1] - pts[i - 1][1]) / len);
            let t = s - cum[i - 1];
            let sd = side as f64;
            Some([pts[i - 1][0] + tx * t + -tz * e.half * sd, pts[i - 1][1] + tz * t + tx * e.half * sd])
        };
        let step = self.c.block_step;
        let mut out = vec![];
        let mut open: Option<(f64, f64, bool)> = None;
        if hi - lo > step {
            let steps = ((hi - lo) / step).ceil() as usize;
            for i in 0..=steps {
                let s = lo + ((hi - lo) * i as f64) / steps as f64;
                let blocked = kerb_point(s).and_then(inside_other);
                if blocked.is_some() && open.is_none() {
                    open = Some((lo.max(s - step / 2.0), s, false));
                }
                if let Some(oi) = blocked {
                    let w = open.as_mut().unwrap();
                    w.1 = hi.min(s + step / 2.0);
                    let other = &plan.entries[oi];
                    let driveway = other.highway == "service" || other.half <= self.c.alley_half_max;
                    if !driveway && !self.same_street(ei, oi) {
                        w.2 = true;
                    }
                }
                if blocked.is_none()
                    && let Some(w) = open.take()
                {
                    out.push(w);
                }
            }
            if let Some(w) = open {
                out.push(w);
            }
        }
        out
    }

    /// `redWindows(segment, side)` of `buildKerbRestrictions`: red paint, other street, observed empty kerb.
    pub fn red_windows(&self, w: &PropsWorld, ei: usize, side: i8) -> Vec<Win> {
        let e = &w.street.plan.entries[ei];
        let o = &w.street.o;
        let c = &self.c;
        let blocked = self.blocked_windows(w, ei, side);
        let mut list: Vec<Win> = vec![];
        let walk = if side > 0 { e.walks[0] } else { e.walks[1] };
        let painted = e.half > 0.0 && o.curb_face_height > 0.0 && walk >= o.min_sidewalk_width;
        if painted {
            // kerbPieces
            let lo = e.trim_start;
            let hi = e.length - e.trim_end;
            let mut pieces: Vec<(f64, f64, bool, bool)> = vec![];
            let mut at = lo;
            let mut at_corner = e.approach_start.is_some();
            let mut sorted = blocked.clone();
            sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            for bw in &sorted {
                if bw.1 <= at || bw.0 >= hi {
                    continue;
                }
                if bw.0 > at {
                    pieces.push((at, bw.0.min(hi), at_corner, bw.2));
                }
                if bw.1 > at {
                    at = bw.1;
                    at_corner = bw.2;
                }
            }
            if at < hi {
                pieces.push((at, hi, at_corner, e.approach_end.is_some()));
            }
            // planCurbPaintPiece: only the corner runs are red (the alley's too).
            let mut runs: Vec<(f64, f64, bool)> = vec![]; // (s0, s1, observed)
            for &(a, b, c_lo, c_hi) in &pieces {
                let run = b - a;
                if !(run > c.min_run * 2.0) {
                    continue;
                }
                let corner = c.corner_red.min(run * c.corner_max_share);
                if corner < c.min_run {
                    continue;
                }
                if c_lo {
                    runs.push((a, a + corner, false));
                }
                if c_hi {
                    runs.push((b - corner, b, false));
                }
            }
            // withObservedRedKerb, then withObservedParkedKerb.
            if let Some(ov) = self.overlay(ei) {
                let frame = station_at(e, (lo + hi) / 2.0, false);
                for red in &ov.red_kerb {
                    if observed_side(ov, [frame.nx, frame.nz], red["side"].as_str().unwrap_or("")) != side {
                        continue;
                    }
                    let (Some(a), Some(b)) = (observed_station(e, ov, red["s0"].as_f64().unwrap_or(f64::NAN)), observed_station(e, ov, red["s1"].as_f64().unwrap_or(f64::NAN))) else { continue };
                    for &(p0, p1, _, _) in &pieces {
                        let r0 = a.min(b).max(p0);
                        let r1 = a.max(b).min(p1);
                        if !(r1 - r0 >= c.min_run) {
                            continue;
                        }
                        let mut kept = vec![];
                        for &run in &runs {
                            if run.1 <= r0 || run.0 >= r1 {
                                kept.push(run);
                                continue;
                            }
                            if run.0 < r0 && r0 - run.0 >= c.min_run {
                                kept.push((run.0, r0, run.2));
                            }
                            if run.1 > r1 && run.1 - r1 >= c.min_run {
                                kept.push((r1, run.1, run.2));
                            }
                        }
                        kept.push((r0, r1, true));
                        kept.sort_by(|p, q| p.0.partial_cmp(&q.0).unwrap());
                        runs = kept;
                    }
                }
                for pw in self.overlay_windows(w, ei, "kerbParked").into_iter().filter(|q| q.0 == side) {
                    let mut kept = vec![];
                    for &run in &runs {
                        if run.2 || run.1 <= pw.1 || run.0 >= pw.2 {
                            kept.push(run);
                            continue;
                        }
                        if run.0 < pw.1 && pw.1 - run.0 >= c.min_run {
                            kept.push((run.0, pw.1, run.2));
                        }
                        if run.1 > pw.2 && run.1 - pw.2 >= c.min_run {
                            kept.push((pw.2, run.1, run.2));
                        }
                    }
                    kept.sort_by(|p, q| p.0.partial_cmp(&q.0).unwrap());
                    runs = kept;
                }
            }
            for (s0, s1, _) in runs {
                if s1 - s0 >= c.min_run {
                    list.push(Win { lo: s0, hi: s1, reason: "red-kerb" });
                }
            }
        }
        for bw in blocked {
            list.push(Win { lo: bw.0, hi: bw.1, reason: "other-street" });
        }
        // observedKerbClear
        if let Some(ov) = self.overlay(ei) {
            let frame = station_at(e, e.length / 2.0, false);
            for entry in &ov.kerb_clear {
                if observed_side(ov, [frame.nx, frame.nz], entry["side"].as_str().unwrap_or("")) != side {
                    continue;
                }
                let (Some(a), Some(b)) = (observed_station(e, ov, entry["s0"].as_f64().unwrap_or(f64::NAN)), observed_station(e, ov, entry["s1"].as_f64().unwrap_or(f64::NAN))) else { continue };
                list.push(Win { lo: a.min(b), hi: a.max(b), reason: "observed-empty-kerb" });
            }
        }
        list
    }

    pub fn overlay(&self, ei: usize) -> Option<&Overlay> {
        self.overlay_of.get(ei).copied().flatten().map(|i| &self.js.overlays[i as usize])
    }

    /// `observedSegmentWindows(segment, key, streetStationAt(segment, length / 2))`:
    /// (side, lo, hi, entry).
    pub fn overlay_windows<'a>(&'a self, w: &PropsWorld, ei: usize, key: &str) -> Vec<(i8, f64, f64, &'a Value)> {
        let Some(ov) = self.overlay(ei) else { return vec![] };
        let list = match key {
            "kerbParked" => &ov.kerb_parked,
            "kerbBodies" => &ov.kerb_bodies,
            "kerbCast" => &ov.kerb_cast,
            _ => return vec![],
        };
        let e = &w.street.plan.entries[ei];
        if list.is_empty() || e.points.len() < 2 {
            return vec![];
        }
        let frame = station_at(e, e.length / 2.0, false);
        let (from, t) = axis_frame(ov);
        let axis_s: Vec<f64> = e.points.iter().map(|q| (q[0] - from[0]) * t[0] + (q[1] - from[1]) * t[1]).collect();
        let s_min = axis_s.iter().copied().fold(f64::INFINITY, f64::min);
        let s_max = axis_s.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let to_station = |s: f64| -> Option<f64> {
            for i in 0..e.points.len() - 1 {
                let (p, q) = (axis_s[i], axis_s[i + 1]);
                if (s - p) * (s - q) <= 0.0 && (q - p).abs() > 1e-9 {
                    return Some(e.cum[i] + (e.cum[i + 1] - e.cum[i]) * ((s - p) / (q - p)));
                }
            }
            None
        };
        let mut out = vec![];
        for entry in list {
            let e0 = entry.get("s0").or_else(|| entry.get("s")).and_then(|v| v.as_f64()).unwrap_or(f64::NAN);
            let e1 = entry.get("s1").or_else(|| entry.get("s")).and_then(|v| v.as_f64()).unwrap_or(f64::NAN);
            let lo = e0.min(e1).max(s_min);
            let hi = e0.max(e1).min(s_max);
            if !(hi >= lo) {
                continue;
            }
            let (Some(a), Some(b)) = (to_station(lo), to_station(hi)) else { continue };
            out.push((observed_side(ov, [frame.nx, frame.nz], entry["side"].as_str().unwrap_or("")), a.min(b), a.max(b), entry));
        }
        out
    }

    /// `stallOnVehicleCrossover(segment, side, station, half)`.
    pub fn on_crossover(&self, ei: usize, side: i8, station: f64, half: f64) -> bool {
        self.crossovers.get(&(ei as u32, side)).is_some_and(|v| v.iter().any(|w| station + half > w[0] && station - half < w[1]))
    }

    /// Not in the JS (`--structures`, stage A6): why a stall's footprint
    /// (`corners`, a car `height` tall) may not stand under the grade-separated
    /// roads: a column within `clearance::PROP_CLEAR_M` of it (a column in
    /// the parking lane takes the stalls it stands in, with the margin), or a
    /// deck, cap or approach fill lower over it than the car's roof and the
    /// margin. None without structures.
    pub fn structure_reason(&self, w: &PropsWorld, corners: &[P2; 4], height: f64) -> Option<&'static str> {
        let s = w.clear_structures()?;
        let m = crate::clearance::PROP_CLEAR_M;
        let c = [(corners[0][0] + corners[2][0]) / 2.0, (corners[0][1] + corners[2][1]) / 2.0];
        let half_diag = (corners[0][0] - c[0]).hypot(corners[0][1] - c[1]);
        // the columns near it, against the footprint itself.
        let t = eo_street::tile_key(c[0], c[1]);
        for dz in -1..=1 {
            for dx in -1..=1 {
                for f in s.column_footprints((t.0 + dx, t.1 + dz)) {
                    if (f.x - c[0]).hypot(f.z - c[1]) > half_diag + f.radius + m + 0.5 {
                        continue;
                    }
                    if quad_distance(corners, [f.x, f.z]) <= f.radius + eo_structures::ENVELOPE_PAD_M + m {
                        return Some("structure-column");
                    }
                }
            }
        }
        // anything lower than its roof over it: along its length, its width wide.
        let y = w.street.paved_at(c[0], c[1]).unwrap_or_else(|| w.ground(c[0], c[1]));
        let ax = [(corners[0][0] + corners[1][0]) / 2.0, (corners[0][1] + corners[1][1]) / 2.0];
        let bx = [(corners[2][0] + corners[3][0]) / 2.0, (corners[2][1] + corners[3][1]) / 2.0];
        let width = (corners[0][0] - corners[1][0]).hypot(corners[0][1] - corners[1][1]);
        for k in 0..=4 {
            let t = k as f64 / 4.0;
            let p = [ax[0] + (bx[0] - ax[0]) * t, ax[1] + (bx[1] - ax[1]) * t];
            let mut low = false;
            s.scan_obstructions(p[0], p[1], width / 2.0, &mut |o| {
                low |= o.part != eo_structures::Part::Column && o.lo <= y + height + m && o.hi >= y;
                !low
            });
            if low {
                return Some("under-structure");
            }
        }
        None
    }

    /// `restrictions.footprintReason(corners, segment)`.
    pub fn footprint_reason(&self, corners: &[P2], ei: usize) -> Option<&'static str> {
        let c = &self.c;
        if self.boxes.first(corners, c.min_lap, |_| true) {
            return Some("junction-box");
        }
        if self.crossings.first(corners, c.min_lap, |_| true) {
            return Some("crosswalk");
        }
        let own_street = self.street_id_ix[ei];
        if self.carriageways.first(corners, c.min_lap, |(oe, st)| oe as usize != ei && (own_street == u32::MAX || st != own_street)) {
            return Some("other-carriageway");
        }
        if self.footways.first(corners, c.footway_lap, |(oe, _)| oe as usize != ei) {
            return Some("other-footway");
        }
        None
    }

    /// `runningCarriageways.lapsOtherStreet(corners, segment.id)`.
    pub fn laps_other_street(&self, corners: &[P2], ei: usize) -> bool {
        let g = &self.running;
        let mut seen: HashSet<u32> = HashSet::new();
        for corner in corners {
            let Some(list) = g.cells.get(&((corner[0] / g.cell).floor() as i32, (corner[1] / g.cell).floor() as i32)) else { continue };
            for &id in list {
                if g.recs[id as usize].0 as usize == ei || !seen.insert(id) {
                    continue;
                }
                if quads_overlap(corners, &g.polys[id as usize]) {
                    return true;
                }
            }
        }
        false
    }

    /// `stallOnMappedCrossing(corners, yaw)`: the footprint run `cornerRedM` forward.
    pub fn on_mapped_crossing(&self, corners: &[P2; 4], yaw: f64) -> bool {
        let (fx, fz) = (yaw.sin() * self.c.corner_red, yaw.cos() * self.c.corner_red);
        let reach = [[corners[0][0] + fx, corners[0][1] + fz], [corners[1][0] + fx, corners[1][1] + fz], corners[2], corners[3]];
        let cross = |p: P2, q: P2, r: P2| (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
        let hits = |a: P2, b: P2, c: P2, d: P2| cross(a, b, c) * cross(a, b, d) < 0.0 && cross(c, d, a) * cross(c, d, b) < 0.0;
        let mut seen: Vec<Cell> = vec![];
        for corner in &reach {
            let cc = ((corner[0] / self.c.crossing_cell).floor() as i32, (corner[1] / self.c.crossing_cell).floor() as i32);
            if seen.contains(&cc) {
                continue;
            }
            seen.push(cc);
            let Some(list) = self.crossing_lines.get(&cc) else { continue };
            for &(a, b) in list {
                for n in 0..4 {
                    if hits(a, b, reach[n], reach[(n + 1) % 4]) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Entries whose kerb lies near (x, z) (the 32 m block grid).
    pub fn entries_near(&self, x: f64, z: f64) -> &[u32] {
        self.block.get(&((x / 32.0).floor() as i32, (z / 32.0).floor() as i32)).map(|v| v.as_slice()).unwrap_or(&[])
    }
}

/// The overlay's origin and unit axis.
fn axis_frame(ov: &Overlay) -> (P2, P2) {
    let (f, t) = (&ov.axis.from, &ov.axis.to);
    let l = (t.x - f.x).hypot(t.z - f.z);
    let l = if l == 0.0 { 1.0 } else { l };
    ([f.x, f.z], [(t.x - f.x) / l, (t.z - f.z) / l])
}

/// `observedSegmentSide(overlay, frame, side)`.
pub fn observed_side(ov: &Overlay, n: P2, side_name: &str) -> i8 {
    let (_, t) = axis_frame(ov);
    let (nx, nz) = (t[1], -t[0]);
    let plus_is_north = n[0] * nx + n[1] * nz >= 0.0;
    if (side_name == "N") == plus_is_north { 1 } else { -1 }
}

/// `observedStationOnSegment(segment, overlay, s)`.
pub fn observed_station(e: &Entry, ov: &Overlay, s: f64) -> Option<f64> {
    if !s.is_finite() {
        return None;
    }
    let (f, t) = axis_frame(ov);
    let p = [f[0] + t[0] * s, f[1] + t[1] * s];
    let mut best: Option<(f64, f64)> = None;
    for i in 0..e.points.len().saturating_sub(1) {
        let (a, b) = (e.points[i], e.points[i + 1]);
        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
        let l2 = dx * dx + dz * dz;
        if !(l2 > 1e-9) {
            continue;
        }
        let tt = ((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / l2;
        if !(-1e-6..=1.0 + 1e-6).contains(&tt) {
            continue;
        }
        let tc = tt.clamp(0.0, 1.0);
        let d = (p[0] - (a[0] + dx * tc)).hypot(p[1] - (a[1] + dz * tc));
        if best.is_some_and(|bb| d >= bb.0) {
            continue;
        }
        best = Some((d, e.cum[i] + l2.sqrt() * tc));
    }
    best.filter(|b| b.0 < 12.0).map(|b| b.1)
}

/// `observedKerbCastAt(x, z)`: (body, alt, paint).
pub fn observed_cast_at(js: &ParkingJs, x: f64, z: f64) -> Option<&Value> {
    let mut best: Option<(f64, &Value)> = None;
    for ov in &js.overlays {
        for car in &ov.kerb_cast_at {
            let d = (x - car["x"].as_f64().unwrap_or(f64::NAN)).hypot(z - car["z"].as_f64().unwrap_or(f64::NAN));
            let reach = car["reachM"].as_f64().filter(|r| *r != 0.0).unwrap_or(4.0);
            if !(d <= reach) || best.is_some_and(|b| d >= b.0) {
                continue;
            }
            best = Some((d, car));
        }
    }
    best.map(|b| b.1)
}

/// `mappedMovement(entry)` of lane-assignment.js.
fn mapped_movement(entry: &str) -> Option<&'static str> {
    let arrows: Vec<String> = entry.to_lowercase().split(';').map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).collect();
    if arrows.is_empty() || arrows.iter().all(|a| a == "none") {
        return None;
    }
    let left = arrows.iter().any(|a| a.ends_with("left"));
    let right = arrows.iter().any(|a| a.ends_with("right"));
    let through = arrows.iter().any(|a| a == "through" || a.starts_with("merge_to"));
    match (through, left, right) {
        (true, true, false) => Some("through-far"),
        (true, false, true) => Some("through-near"),
        (false, true, false) => Some("far"),
        (false, false, true) => Some("near"),
        (true, false, false) => Some("through"),
        _ => None,
    }
}

/// `laneAssignment(node, approach)` where the source maps `turn:lanes` for the
/// arriving lanes (`sourceMapsTurnLanes`): one movement per lane, kerb first
/// (None where the arrow is unmarked or the node does not offer it). None when
/// the approach has no mapped arrows or they do not name exactly its lanes.
fn mapped_lane_movements(w: &PropsWorld, js: &ParkingJs, n: &eo_street::plan::Node, app: &eo_street::plan::Approach) -> Option<Vec<Option<&'static str>>> {
    let plan = &w.street.plan;
    let e = &plan.entries[app.entry];
    let piece = if app.at_start { *e.pieces.first()? } else { *e.pieces.last()? };
    let src = &w.street.city.segments[piece];
    let (fwd, bwd) = js.turn_lanes.get(src.street_id.as_ref()?)?;
    let mapped = if app.oneway || !app.at_start { fwd.as_deref()? } else { bwd.as_deref()? };
    if mapped.is_empty() {
        return None;
    }
    // approachLaneFrame's lane count: the authored arriving moving lanes, else the stated count.
    let arriving_dir: i8 = if app.at_start { -1 } else { 1 };
    let authored = src
        .lane_layout
        .as_ref()
        .map(|l| l.iter().filter(|q| q.motor && matches!(q.role.as_str(), "through" | "left-turn" | "right-turn") && q.dir == arriving_dir && q.width > 0.0).count())
        .unwrap_or(0);
    let count = if authored > 0 {
        authored
    } else {
        let total = (src.lanes.unwrap_or(2.0).round() as i64).max(1) as usize;
        if app.oneway { total } else { (total / 2).max(1) }
    };
    let entries: Vec<&str> = mapped.split('|').collect();
    if entries.len() != count {
        return None;
    }
    // movementsFrom(node, approach)
    let u = app.u;
    let m = [-u[1], u[0]];
    let through_cos = (25.0f64).to_radians().cos();
    let (mut through, mut near, mut far) = (false, false, false);
    for other in &n.approaches {
        if std::ptr::eq(other, app) || (other.oneway && other.flows_toward) {
            continue;
        }
        let dot_f = -(other.u[0] * u[0] + other.u[1] * u[1]);
        let dot_l = other.u[0] * m[0] + other.u[1] * m[1];
        if dot_f > through_cos {
            through = true;
            continue;
        }
        if dot_f < -through_cos {
            continue;
        }
        if dot_l < -0.25 {
            near = true;
        } else if dot_l > 0.25 {
            far = true;
        }
    }
    let available = |mv: &str| match mv {
        "far" => far,
        "near" => near,
        "through-far" => through && far,
        "through-near" => through && near,
        _ => through,
    };
    Some((0..count).map(|k| mapped_movement(entries[count - 1 - k]).filter(|mv| available(mv))).collect())
}

/// `planVehicleCrossovers(entries, o)`: the kerb windows a driveway or a surface
/// lot cuts, by (entry, side).
fn plan_vehicle_crossovers(w: &PropsWorld, js: &ParkingJs, c: &KerbConsts, stats: &mut BTreeMap<&'static str, usize>) -> HashMap<(u32, i8), Vec<[f64; 2]>> {
    let plan = &w.street.plan;
    let o = &w.street.o;
    let entries = &plan.entries;
    let mut out: HashMap<(u32, i8), Vec<[f64; 2]>> = HashMap::new();
    let ramp_reach = o.curb_top_width + o.ramp_run;
    let hosts: Vec<usize> = (0..entries.len()).filter(|&i| entries[i].class_rank >= 2 && entries[i].length > 0.5 && entries[i].structure_leg.is_none()).collect();
    if hosts.is_empty() {
        return out;
    }
    let mut reach: f64 = 0.0;
    for &h in &hosts {
        let e = &entries[h];
        reach = reach.max(e.half + e.walks[0].max(e.walks[1]) + c.x_mouth_slack);
    }
    let cell = (12.0f64).max(reach * 2.0);
    // (host, edge index)
    let mut grid: HashMap<Cell, Vec<(u32, u32)>> = HashMap::new();
    for &h in &hosts {
        let e = &entries[h];
        for i in 0..e.points.len().saturating_sub(1) {
            let (a, b) = (e.points[i], e.points[i + 1]);
            for cc in cells_of(a[0].min(b[0]), a[0].max(b[0]), a[1].min(b[1]), a[1].max(b[1]), cell) {
                grid.entry(cc).or_default().push((h as u32, i as u32));
            }
        }
    }
    let edge_closest = |h: usize, i: usize, p: P2| -> (f64, f64, f64) {
        let e = &entries[h];
        let (a, b) = (e.points[i], e.points[i + 1]);
        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
        let l2 = dx * dx + dz * dz;
        let t = if l2 > 1e-12 { ((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / l2 } else { 0.0 }.clamp(0.0, 1.0);
        let (qx, qz) = (a[0] + dx * t, a[1] + dz * t);
        ((p[0] - qx).hypot(p[1] - qz), e.cum[i] + (e.cum[i + 1] - e.cum[i]) * t, t)
    };
    let id_lt = |a: usize, b: usize| entries[a].id < entries[b].id;
    // Why a window [mid +- width/2] fails (`crossoverWindowFault`).
    let fault = |mid: f64, width: f64, s0: f64, s1: f64, existing: &[eo_street::plan::RampWin]| -> bool {
        if !(mid - width / 2.0 > s0 && mid + width / 2.0 < s1) {
            return true;
        }
        if !(mid - width / 2.0 - o.ramp_flare_run > s0 && mid + width / 2.0 + o.ramp_flare_run < s1) {
            return true;
        }
        for q in existing {
            let lo = q.flare_lo.min(q.lo);
            let hi = q.flare_hi.max(q.hi);
            if mid - width / 2.0 - o.ramp_flare_run < hi && mid + width / 2.0 + o.ramp_flare_run > lo {
                return true;
            }
        }
        false
    };
    let min_angle_cos = c.x_min_angle_deg.to_radians().cos();
    // (host, side, mid, width)
    let mut pending: Vec<(usize, i8, f64, f64)> = vec![];
    for (di, drive) in entries.iter().enumerate() {
        if drive.class_rank != 1 || drive.points.len() < 2 || drive.structure_leg.is_some() {
            continue;
        }
        for at_start in [true, false] {
            let p = if at_start { drive.points[0] } else { drive.points[drive.points.len() - 1] };
            let u = if at_start { edge_dir(&drive.points, 0) } else { edge_dir(&drive.points, drive.points.len() - 2) };
            let (gx, gz) = ((p[0] / cell).floor() as i32, (p[1] / cell).floor() as i32);
            let mut best: Option<((f64, f64, f64), usize, usize)> = None;
            for ax in gx - 1..=gx + 1 {
                for az in gz - 1..=gz + 1 {
                    let Some(list) = grid.get(&(ax, az)) else { continue };
                    for &(h, i) in list {
                        let (h, i) = (h as usize, i as usize);
                        if h == di || entries[h].class_rank <= drive.class_rank {
                            continue;
                        }
                        let hit = edge_closest(h, i, p);
                        let he = &entries[h];
                        let limit = he.half + he.walks[0].max(he.walks[1]) + c.x_mouth_slack;
                        if !(hit.0 <= limit) {
                            continue;
                        }
                        if let Some((bh, bhost, _)) = best
                            && !(hit.0 < bh.0 - 1e-6 || ((hit.0 - bh.0).abs() <= 1e-6 && id_lt(h, bhost)))
                        {
                            continue;
                        }
                        best = Some((hit, h, i));
                    }
                }
            }
            let Some((hit, h, i)) = best else { continue };
            let host = &entries[h];
            let hd = edge_dir(&host.points, i);
            if (u[0] * hd[0] + u[1] * hd[1]).abs() > min_angle_cos {
                *stats.entry("crossover-rejected-shallow").or_default() += 1;
                continue;
            }
            let (nx, nz) = (-hd[1], hd[0]);
            let (a, b) = (host.points[i], host.points[i + 1]);
            let lateral = (p[0] - (a[0] + (b[0] - a[0]) * hit.2)) * nx + (p[1] - (a[1] + (b[1] - a[1]) * hit.2)) * nz;
            let side: i8 = if lateral.abs() > 1e-3 {
                if lateral > 0.0 { 1 } else { -1 }
            } else if (if at_start { -1.0 } else { 1.0 }) * (u[0] * nx + u[1] * nz) > 0.0 {
                1
            } else {
                -1
            };
            let walk = if side > 0 { host.walks[0] } else { host.walks[1] };
            if !(walk >= o.min_sidewalk_width) || !(walk > ramp_reach + 0.05) {
                *stats.entry("crossover-rejected-narrow-walk").or_default() += 1;
                continue;
            }
            let width0 = (drive.half * 2.0 + c.x_lip_widen).clamp(c.x_min_width, c.x_max_width);
            let s0 = host.trim_start + c.x_trim_margin;
            let s1 = host.length - host.trim_end - c.x_trim_margin;
            let existing = &host.kerb_ramps[if side > 0 { 0 } else { 1 }];
            // fitCrossoverWindow
            let mut fit: Option<(f64, f64)> = None;
            if !fault(hit.1, width0, s0, s1, existing) {
                fit = Some((hit.1, width0));
            } else {
                let min_width = c.x_min_width.max(drive.half * 2.0 + 0.2);
                let mut width = width0;
                'outer: while width >= min_width - 1e-6 {
                    let slack = (width / 2.0 - drive.half).max(0.0);
                    for f in [0.0, 0.25, -0.25, 0.5, -0.5, 0.75, -0.75, 1.0, -1.0] {
                        let mid = hit.1 + f * slack;
                        if !fault(mid, width, s0, s1, existing) {
                            fit = Some((mid, width));
                            break 'outer;
                        }
                    }
                    width -= 0.4;
                }
            }
            let Some((mid, width)) = fit else {
                *stats.entry("crossover-rejected-fit").or_default() += 1;
                continue;
            };
            pending.push((h, side, mid, width));
        }
    }
    // Surface lots with no mapped drive: the frontage on its host way.
    {
        let segs = &w.street.city.segments;
        let mut by_street: HashMap<&str, Vec<usize>> = HashMap::new();
        for (i, s) in segs.iter().enumerate() {
            if let Some(id) = s.street_id.as_deref() {
                by_street.entry(id).or_default().push(i);
            }
        }
        let mut seen: HashSet<&str> = HashSet::new();
        for lf in &js.lot_frontages {
            if !seen.insert(lf.lot.as_str()) {
                continue;
            }
            // sliceSfLotsParks: the nearest segment of the host way within 12 m carries it.
            let host_id = format!("sf-street-{}", lf.host);
            let carried = by_street.get(host_id.as_str()).is_some_and(|list| {
                list.iter().any(|&si| {
                    let pts = &segs[si].points;
                    (0..pts.len().saturating_sub(1)).any(|k| {
                        let (a, b) = (pts[k], pts[k + 1]);
                        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
                        let l2 = dx * dx + dz * dz;
                        let t = if l2 > 1e-12 { ((lf.x - a[0]) * dx + (lf.z - a[1]) * dz) / l2 } else { 0.0 }.clamp(0.0, 1.0);
                        (lf.x - a[0] - dx * t).hypot(lf.z - a[1] - dz * t) <= 12.0
                    })
                })
            });
            if !carried {
                continue;
            }
            let p = [lf.x, lf.z];
            let (gx, gz) = ((p[0] / cell).floor() as i32, (p[1] / cell).floor() as i32);
            let mut best: Option<((f64, f64, f64), usize, usize)> = None;
            for ax in gx - 1..=gx + 1 {
                for az in gz - 1..=gz + 1 {
                    let Some(list) = grid.get(&(ax, az)) else { continue };
                    for &(h, i) in list {
                        let (h, i) = (h as usize, i as usize);
                        let hit = edge_closest(h, i, p);
                        if !(hit.0 <= entries[h].half + c.x_mouth_slack) {
                            continue;
                        }
                        if let Some((bh, bhost, _)) = best
                            && !(hit.0 < bh.0 - 1e-6 || ((hit.0 - bh.0).abs() <= 1e-6 && id_lt(h, bhost)))
                        {
                            continue;
                        }
                        best = Some((hit, h, i));
                    }
                }
            }
            let Some((hit, h, i)) = best else { continue };
            let host = &entries[h];
            let hd = edge_dir(&host.points, i);
            let (nx, nz) = (-hd[1], hd[0]);
            let side: i8 = if lf.dir_x * nx + lf.dir_z * nz > 0.0 { 1 } else { -1 };
            let walk = if side > 0 { host.walks[0] } else { host.walks[1] };
            if !(walk >= o.min_sidewalk_width) || !(walk > ramp_reach + 0.05) {
                continue;
            }
            let width = (if lf.width != 0.0 && lf.width.is_finite() { lf.width } else { c.x_max_width }).clamp(c.x_min_width, c.x_max_width);
            let s0 = host.trim_start + c.x_trim_margin;
            let s1 = host.length - host.trim_end - c.x_trim_margin;
            let existing = &host.kerb_ramps[if side > 0 { 0 } else { 1 }];
            let mut placed = None;
            for shift in [0.0, 2.0, -2.0, 4.0, -4.0] {
                let mid = hit.1 + shift;
                if !(mid - width / 2.0 - o.ramp_flare_run > s0 && mid + width / 2.0 + o.ramp_flare_run < s1) {
                    continue;
                }
                let clash = existing.iter().any(|q| {
                    let lo = q.flare_lo.min(q.lo);
                    let hi = q.flare_hi.max(q.hi);
                    mid - width / 2.0 - o.ramp_flare_run < hi && mid + width / 2.0 + o.ramp_flare_run > lo
                });
                if clash {
                    continue;
                }
                placed = Some(mid);
                break;
            }
            if let Some(mid) = placed {
                pending.push((h, side, mid, width));
                *stats.entry("crossover-lot-frontages").or_default() += 1;
            }
        }
    }
    // Merge overlapping openings on one kerb.
    pending.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.partial_cmp(&b.2).unwrap()));
    let mut kept: Vec<(usize, i8, f64, f64)> = vec![];
    for cnd in pending {
        if let Some(last) = kept.last_mut()
            && last.0 == cnd.0
            && last.1 == cnd.1
            && cnd.2 - cnd.3 / 2.0 < last.2 + last.3 / 2.0
        {
            let lo = (last.2 - last.3 / 2.0).min(cnd.2 - cnd.3 / 2.0);
            let hi = (last.2 + last.3 / 2.0).max(cnd.2 + cnd.3 / 2.0);
            last.3 = (hi - lo).min(c.x_max_width);
            last.2 = (lo + hi) / 2.0;
            continue;
        }
        kept.push(cnd);
    }
    for (h, side, mid, width) in kept {
        out.entry((h as u32, side)).or_default().push([mid - width / 2.0, mid + width / 2.0]);
        *stats.entry("crossovers").or_default() += 1;
    }
    out
}
