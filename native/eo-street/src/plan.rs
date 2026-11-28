//! The streetscape plan: a port of `buildStreetscapePlan`'s geometric passes
//! in src/world/streets/street-surface-v2.js, in the order the JS runs them:
//!
//!   prepareSegments -> mergeShortRuns -> collectNodes (inferNodes as the
//!   stream tiles compile) -> planJunction per node -> reconcileTrims ->
//!   planNodePaint (+ planCrossingRamps) -> planCarriagewayYield ->
//!   finaliseJunction (corner arcs, corner curb paths, chord fallbacks,
//!   ramp windows).
//!
//! Each function names the JS function it ports. Not ported: the frontage
//! profile and yard aprons (`computeFrontageProfile`, a simpler frontage march
//! is done at emission: see mesh.rs), crossing evidence (`crossing-evidence.js`:
//! the city's crossing data overriding which legs are painted), through-join
//! paint reservation, vehicle crossovers, the lane-drop paint, observed
//! (reference-photo) markings and metered-kerb stall ticks.

use crate::city::{Segment, StreetCity};
use crate::options::StreetOptions;
use std::collections::{HashMap, HashSet};
use std::f64::consts::PI;

pub type P2 = [f64; 2];

#[inline]
fn hypot(x: f64, z: f64) -> f64 {
    (x * x + z * z).sqrt()
}
#[inline]
fn perp_ccw(u: P2) -> P2 {
    [-u[1], u[0]]
}
#[inline]
fn cross2(a: P2, b: P2) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
/// `normAngle`: [0, 2 pi).
pub fn norm_angle(a: f64) -> f64 {
    let mut v = a % (2.0 * PI);
    if v < 0.0 {
        v += 2.0 * PI;
    }
    v
}
/// `signedAngle`: (-pi, pi].
pub fn signed_angle(a: f64) -> f64 {
    let mut v = a % (2.0 * PI);
    if v > PI {
        v -= 2.0 * PI;
    }
    if v <= -PI {
        v += 2.0 * PI;
    }
    v
}
#[inline]
fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    v.max(lo).min(hi)
}

/// `streetClassRank`.
pub fn street_class_rank(class: &str) -> i32 {
    match class {
        "path" | "steps" | "footway" | "cycleway" | "pedestrian" | "corridor" | "platform" => 0,
        "track" | "service" | "alley" | "driveway" => 1,
        "living_street" | "unclassified" => 2,
        "residential" => 3,
        "tertiary" | "tertiary_link" => 4,
        "secondary" | "secondary_link" => 5,
        "primary" | "primary_link" => 6,
        "trunk" | "trunk_link" => 7,
        "motorway" | "motorway_link" => 8,
        _ => 3,
    }
}

/// `dedupePoints`.
pub fn dedupe_points(points: &[P2]) -> Vec<P2> {
    let mut out: Vec<P2> = Vec::with_capacity(points.len());
    for p in points {
        if !p[0].is_finite() || !p[1].is_finite() {
            continue;
        }
        if let Some(l) = out.last() {
            if hypot(p[0] - l[0], p[1] - l[1]) < 1e-4 {
                continue;
            }
        }
        out.push(*p);
    }
    out
}

/// `arcTable`.
pub fn arc_table(points: &[P2]) -> Vec<f64> {
    let mut cum = vec![0.0];
    for i in 1..points.len() {
        cum.push(cum[i - 1] + hypot(points[i][0] - points[i - 1][0], points[i][1] - points[i - 1][1]));
    }
    cum
}

fn edge_dir(points: &[P2], i: usize) -> P2 {
    let (a, b) = (points[i], points[i + 1]);
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let mut len = hypot(dx, dz);
    if len == 0.0 {
        len = 1.0;
    }
    [dx / len, dz / len]
}

fn edge_index_at(cum: &[f64], s: f64) -> usize {
    let mut lo = 0;
    for i in 0..cum.len() - 1 {
        if s >= cum[i] - 1e-9 {
            lo = i;
        }
    }
    lo.min(cum.len() - 2)
}

/// A station frame on a centreline (`frameAt`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub s: f64,
    pub x: f64,
    pub z: f64,
    pub nx: f64,
    pub nz: f64,
    pub miter: f64,
    pub tx: f64,
    pub tz: f64,
}

impl Frame {
    /// `offsetPoint` in plan.
    pub fn at(&self, u: f64) -> P2 {
        [self.x + self.nx * u * self.miter, self.z + self.nz * u * self.miter]
    }

    /// Split the drawn ribbon. Each lateral point of the new section lies
    /// on the edge joining the corresponding points of the original pair.
    pub(crate) fn split(self, next: Self, s: f64) -> Self {
        let distance = next.s - self.s;
        let t = (s - self.s) / distance;
        let mix = |a: f64, b: f64| a + t * (b - a);
        let nx = mix(self.nx * self.miter, next.nx * next.miter);
        let nz = mix(self.nz * self.miter, next.nz * next.miter);
        let miter = nx.hypot(nz);
        let (nx, nz) = if miter > f64::EPSILON {
            (nx / miter, nz / miter)
        } else {
            (self.nx, self.nz)
        };
        Self {
            s,
            x: mix(self.x, next.x),
            z: mix(self.z, next.z),
            nx,
            nz,
            miter,
            tx: (next.x - self.x) / distance,
            tz: (next.z - self.z) / distance,
        }
    }
}

/// `frameAt`.
pub fn frame_at(points: &[P2], cum: &[f64], s: f64, allow_miter: bool) -> Frame {
    let i = edge_index_at(cum, s);
    let (a, b) = (points[i], points[i + 1]);
    let mut seg_len = cum[i + 1] - cum[i];
    if seg_len == 0.0 {
        seg_len = 1.0;
    }
    let t = (s - cum[i]) / seg_len;
    let d = edge_dir(points, i);
    let (mut nx, mut nz) = (-d[1], d[0]);
    let mut miter = 1.0;
    if allow_miter && i > 0 && (s - cum[i]).abs() < 1e-6 {
        let dp = edge_dir(points, i - 1);
        let (mut ax, mut az) = (-dp[1] + nx, dp[0] + nz);
        let al = hypot(ax, az);
        if al > 1e-6 {
            ax /= al;
            az /= al;
            let c = ax * nx + az * nz;
            if c > 0.25 {
                nx = ax;
                nz = az;
                miter = 1.0 / c;
            }
        }
    }
    Frame { s, x: a[0] + (b[0] - a[0]) * t, z: a[1] + (b[1] - a[1]) * t, nx, nz, miter, tx: d[0], tz: d[1] }
}

/// `buildStations`: one frame every `max_step` metres and at every vertex.
pub fn build_stations(points: &[P2], cum: &[f64], s0: f64, s1: f64, max_step: f64) -> Vec<Frame> {
    let total = *cum.last().unwrap();
    let start = clamp(s0, 0.0, total);
    let end = clamp(s1, 0.0, total);
    let mut keys = vec![start];
    for &c in &cum[1..cum.len() - 1] {
        if c > start + 1e-6 && c < end - 1e-6 {
            keys.push(c);
        }
    }
    keys.push(end);
    let mut arcs = vec![];
    for w in keys.windows(2) {
        let span = w[1] - w[0];
        if span <= 1e-6 {
            continue;
        }
        let steps = ((span / max_step).ceil() as usize).max(1);
        for k in 0..steps {
            arcs.push(w[0] + span * (k as f64 / steps as f64));
        }
    }
    arcs.push(end);
    arcs.into_iter().map(|s| frame_at(points, cum, s, true)).collect()
}

/// A kerb ramp window, in the frame its owner uses (ribbon arc length, or corner arc angle).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RampWin {
    pub lo: f64,
    pub hi: f64,
    pub flare_lo: f64,
    pub flare_hi: f64,
    /// Node index that reserved it (ribbon crossing ramps), or usize::MAX.
    pub node: usize,
    pub at_start: bool,
}

/// One paved run of street (`prepareSegments` + `mergeShortRuns`).
#[derive(Clone, Debug)]
pub struct Entry {
    /// The run's id: its first piece's segment id.
    pub id: String,
    /// Source segment indices of the pieces, in order.
    pub pieces: Vec<usize>,
    /// The segment whose roadway (lanes, layout, kerb bands, class) the run carries.
    pub road: usize,
    pub points: Vec<P2>,
    pub cum: Vec<f64>,
    pub length: f64,
    pub half: f64,
    /// Footway on the +n side (`sidewalkLeft`) and the -n side (`sidewalkRight`).
    pub walks: [f64; 2],
    pub trim_start: f64,
    pub trim_end: f64,
    pub marking_trim_start: f64,
    pub marking_trim_end: f64,
    pub class_rank: i32,
    pub highway: String,
    pub lanes: i32,
    pub oneway: bool,
    pub approach_start: Option<(usize, usize)>,
    pub approach_end: Option<(usize, usize)>,
    pub mouth_start_m: Option<f64>,
    pub mouth_end_m: Option<f64>,
    /// Crossing ramps reserved by `planCrossingRamps`, [left, right].
    pub kerb_ramps: [Vec<RampWin>; 2],
    /// Arc spans this ribbon gives up to a busier street it crosses without a node.
    pub yield_spans: Vec<[f64; 2]>,
    /// Receiving street at a terminal driveway crossing, at the start or end.
    pub(crate) driveway_owners: [Option<usize>; 2],
    pub(crate) driveway_cuts: [Vec<RampWin>; 2],
    /// Footway spans lying on somebody else's road, [left, right].
    pub walk_yield_spans: [Vec<[f64; 2]>; 2],
    /// Only with `StreetOptions::structures` (`crate::joins`): this run is a
    /// grade-separated road's leg into a junction, the index of its
    /// `StructureEnd`. It is not drawn (the structure's deck is there); the
    /// junction gets a mouth for it. None for every street run.
    pub structure_leg: Option<usize>,
    /// Only with `StreetOptions::structures`: its [start, end] meets a
    /// structure end with no junction there (a stub), the index of that
    /// `StructureEnd`: the end is capped, and raised where the structure asks.
    pub structure_ends: [Option<usize>; 2],
    /// Only with `StreetOptions::structures`: a stub's approach embankment may
    /// climb on over this run (joined to the stub end to end through runs
    /// with no junction between): (the `StructureEnd` index, this run's end
    /// toward it: true for its start, the street's distance from that end to
    /// the structure's end).
    /// A connected bridge embankment reaching this run at its given station.
    pub(crate) landings: Vec<(f64, crate::joins::Rise)>,
    pub structure_ramps: Vec<(usize, bool, f64)>,
}

/// Publish the geometry used by the painted motor lanes. Each piece keeps its
/// OSM way identity, including pieces merged into a longer paved run.
pub fn traffic_sections(city: &StreetCity, plan: &Plan, options: &StreetOptions) -> Vec<eo_data::sim_source::TrafficSection> {
    traffic_sections_with_lines(city, plan, options, &crate::paint::plan_markings(city, plan, options), |_, _, _| Default::default())
}

pub(crate) fn traffic_sections_with_lines(
    city: &StreetCity,
    plan: &Plan,
    options: &StreetOptions,
    markings: &[Vec<crate::paint::Line>],
    surface: impl Fn(usize, f64, f64) -> (Vec<[f64; 3]>, Vec<[usize; 2]>),
) -> Vec<eo_data::sim_source::TrafficSection> {
    use crate::paint::Paint;
    use eo_data::sim_source::{TrafficLane, TrafficPaint, TrafficSection};

    let mut sections = Vec::new();
    for (ei, (entry, lines)) in plan.entries.iter().zip(markings).enumerate() {
        if entry.structure_leg.is_some() {
            continue;
        }
        let layout = city.segments[entry.road].lane_layout.as_deref().unwrap_or_default();
        let shared_bidirectional = matches!(layout, [lane] if lane.role == "shared-bidirectional");
        let lanes: Vec<_> = layout
            .iter()
            .filter(|lane| lane.motor && lane.role == "through" && matches!(lane.dir, -1 | 1))
            .map(|lane| TrafficLane { direction: lane.dir, lateral: lane.lateral, width: lane.width })
            .collect();
        let (mark, gap) = crate::paint::broken_line_pattern(&city.segments[entry.road], &entry.highway, options);
        let mut start = 0;
        for &index in &entry.pieces {
            let source = &city.segments[index];
            // The merge keeps the first point at each seam and appends the
            // remaining prepared points of the next piece.
            let end = start + dedupe_points(&source.points).len() - 1;
            if let Some(way) = source.street_id.as_deref().and_then(|id| id.strip_prefix("sf-street-")) {
                if !source.separated {
                    let (begin, finish) = (entry.cum[start], entry.cum[end]);
                    let lo = begin.max(entry.marking_trim_start).max(entry.s0());
                    let hi = finish.min(entry.length - entry.marking_trim_end).min(entry.s1());
                    let mut paint = Vec::new();
                    if hi > lo {
                        let stations = std::iter::once(lo)
                            .chain(entry.cum[start..=end].iter().copied().filter(|s| *s > lo && *s < hi))
                            .chain(std::iter::once(hi));
                        for l in lines.iter().filter(|l| matches!(l.paint, Paint::White | Paint::Yellow) && l.width > 0.0) {
                            let (mark, gap) = if l.role == "centre-turn-boundary" {
                                (options.twltl_dash_mark, options.twltl_dash_gap)
                            } else {
                                (mark, gap)
                            };
                            paint.push(TrafficPaint {
                                lateral: -l.u,
                                width: l.width,
                                broken: l.dashed,
                                yellow: l.paint == Paint::Yellow,
                                divider: if l.paint == Paint::White && l.dashed && l.role == "divider" { l.direction } else { 0 },
                                points: stations.clone().map(|s| entry.frame(s, true).at(l.u)).collect(),
                                dash: l.dashed.then_some([mark, gap, (lo - entry.marking_trim_start).rem_euclid(mark + gap)]),
                            });
                        }
                    }
                    let (surface, carriageway_spans) = surface(ei, begin, finish);
                    sections.push(TrafficSection {
                        way: way.into(),
                        points: entry.points[start..=end].to_vec(),
                        half: entry.half,
                        walks: entry.walks,
                        walk_gaps: std::array::from_fn(|side| {
                            entry.yield_spans.iter().chain(&entry.walk_yield_spans[side]).filter_map(|&[a, b]| {
                                let a = a.max(begin);
                                let b = b.min(finish);
                                (a < b).then_some([a - begin, b - begin])
                            }).collect()
                        }),
                        lanes: lanes.clone(),
                        shared_bidirectional,
                        paint_span: (hi > lo).then_some([lo - entry.cum[start], hi - entry.cum[start]]),
                        paint,
                        crosses_road: entry.yield_spans.iter().any(|[a, b]| *a < entry.cum[end] && *b > entry.cum[start]),
                        surface,
                        carriageway_spans,
                    });
                }
            }
            start = end;
        }
    }
    sections
}

impl Entry {
    pub fn frame(&self, s: f64, miter: bool) -> Frame {
        frame_at(&self.points, &self.cum, clamp(s, 0.0, self.length), miter)
    }
    pub fn s0(&self) -> f64 {
        clamp(self.trim_start, 0.0, self.length)
    }
    pub fn s1(&self) -> f64 {
        clamp(self.length - self.trim_end, 0.0, self.length)
    }
    pub fn in_yield_span(&self, s: f64) -> bool {
        self.yield_spans.iter().any(|sp| s >= sp[0] && s <= sp[1])
    }
}

#[derive(Clone, Debug)]
pub struct Approach {
    pub entry: usize,
    pub at_start: bool,
    /// Unit direction away from the node along the approach.
    pub u: P2,
    pub angle: f64,
    pub half: f64,
    pub run_length: f64,
    pub mouth_length: f64,
    pub width_ccw: f64,
    pub width_cw: f64,
    pub oneway: bool,
    pub flows_toward: bool,
    pub trim: f64,
    pub class_rank: i32,
    pub crossing_painted: bool,
    pub paint_reach: f64,
    /// Crossing ramps reserved on the [ccw, cw] side.
    pub crossing_ramps: [bool; 2],
    /// (`seams::road_seams`) a slip road joining or leaving a through road
    /// alongside it (within `MERGE_COS`; the slip with no walk): the trim of that
    /// through leg. The slip's ribbon is drawn from there (clipped where the
    /// through road covers it, `seams`) instead of from its own trim (cut back
    /// until clear of the through road, often 100 m and more), and the pad
    /// is the through legs' alone: it had fanned over the through road's
    /// lanes to the slip's far mouth at its own level, up to 0.6 m off them.
    pub merge_trim: Option<f64>,
}

/// (`merge_trim`) a slip road runs alongside a through leg within 25 degrees.
pub const MERGE_COS: f64 = 0.906;

/// `solveCornerArc`'s fillet.
#[derive(Clone, Copy, Debug)]
pub struct Corner {
    pub centre: P2,
    pub radius: f64,
    pub ta: P2,
    pub tb: P2,
    pub d_a: f64,
    pub d_b: f64,
    pub corner: P2,
    pub sweep: f64,
    pub bisector: P2,
}

/// One station of a corner's curb path (`approachCurbStation`, arc and chord stations).
#[derive(Clone, Copy, Debug)]
pub struct CurbStation {
    pub x: f64,
    pub z: f64,
    /// Unit direction away from the road (toward the footway).
    pub out: P2,
    pub scale: f64,
    pub walk: f64,
    /// Arc angle (corner frame), when the station is on an arc or a stamped chord.
    pub ang: Option<f64>,
    /// The ribbon mouth this station is: (entry, arc length, lateral u) - its
    /// position is that ribbon's own end vertex.
    pub mouth: Option<(usize, f64, f64)>,
}

#[derive(Clone, Debug)]
pub struct CornerPath {
    pub stations: Vec<CurbStation>,
    pub ramps: Vec<RampWin>,
    /// Approach indices A (start) and B (end) of this corner.
    pub a: usize,
    pub b: usize,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub id: String,
    pub key: Option<String>,
    pub position: P2,
    pub signal: Option<String>,
    pub inferred: bool,
    pub approaches: Vec<Approach>,
    pub corners: Vec<Option<Corner>>,
    pub paths: Vec<CornerPath>,
    pub earns_paint: bool,
    pub band_depth: f64,
    pub max_class_rank: i32,
}

impl Node {
    pub fn signalised(&self) -> bool {
        self.signal.is_some()
    }
}

pub struct Plan {
    pub entries: Vec<Entry>,
    pub nodes: Vec<Node>,
}

// ---------------------------------------------------------------------------
// prepare + merge
// ---------------------------------------------------------------------------

/// `readSegmentContract` + `sidewalkWidths`: (half, [left, right] walks, lanes), or None.
fn contract(seg: &Segment) -> Option<(Vec<P2>, f64, [f64; 2], i32)> {
    let points = dedupe_points(&seg.points);
    if points.len() < 2 {
        return None;
    }
    let width = seg.width;
    if !width.is_finite() || width <= 0.2 {
        return None;
    }
    let walk = seg.sidewalk_w.filter(|w| w.is_finite() && *w > 0.0).unwrap_or(0.0);
    let left = seg.sidewalk_left.filter(|w| w.is_finite() && *w >= 0.0).unwrap_or(walk);
    let right = seg.sidewalk_right.filter(|w| w.is_finite() && *w >= 0.0).unwrap_or(walk);
    let lanes_raw = seg.lanes.filter(|l| *l != 0.0 && l.is_finite());
    let lanes = match lanes_raw {
        Some(l) => (l.round() as i32).max(1),
        None => ((width / 3.2).round() as i32).max(1),
    };
    Some((points, width / 2.0, [left.max(0.0), right.max(0.0)], lanes))
}

/// `prepareSegments`, in stable-key order.
fn prepare(city: &StreetCity, o: &StreetOptions) -> Vec<Entry> {
    let mut order: Vec<usize> = (0..city.segments.len()).collect();
    order.sort_by(|&a, &b| city.segments[a].seed().cmp(city.segments[b].seed()));
    let mut out = vec![];
    for i in order {
        let seg = &city.segments[i];
        if o.exclude_highways.contains(&seg.highway) {
            continue;
        }
        // This surveyed city street crosses the tracks. Keep its driven
        // floor when the separate structures pass is disabled.
        let measured_span = o.pavement_repairs && o.structures.is_none()
            && seg.street_id.as_deref() == Some("sf-street-544431983");
        if !o.pave_grade_separated && seg.separated && !measured_span {
            continue;
        }
        let Some((points, half, walks, lanes)) = contract(seg) else { continue };
        let cum = arc_table(&points);
        let length = *cum.last().unwrap();
        if !(length > 0.2) {
            continue;
        }
        let highway = if seg.highway.is_empty() { "residential".to_string() } else { seg.highway.clone() };
        out.push(Entry {
            id: seg.id.clone(),
            pieces: vec![i],
            road: i,
            points,
            cum,
            length,
            half,
            walks,
            trim_start: 0.0,
            trim_end: 0.0,
            marking_trim_start: 0.0,
            marking_trim_end: 0.0,
            class_rank: street_class_rank(&highway),
            highway,
            lanes,
            oneway: seg.is_oneway(),
            approach_start: None,
            approach_end: None,
            mouth_start_m: None,
            mouth_end_m: None,
            kerb_ramps: [vec![], vec![]],
            yield_spans: vec![],
            driveway_owners: [None; 2],
            driveway_cuts: Default::default(),
            walk_yield_spans: [vec![], vec![]],
            structure_leg: None,
            structure_ends: [None, None],
            landings: vec![],
            structure_ramps: vec![],
        });
    }
    out
}

/// How far along the street from a structure's end the runs an approach
/// embankment may climb over are looked for (more than any rise the table's
/// grades let a ramp need). A search reach, not a street dimension.
const RAMP_WALK_M: f64 = 600.0;

/// Only with `StreetOptions::structures` (`crate::joins`): each structure end
/// that lands where two or more paved runs end becomes a leg (a run that is
/// not drawn, appended after the paved runs) for node collection to build
/// the junction's mouth from; one landing on a single run's end marks that
/// stub (`Entry::structure_ends`). One landing on nothing (a run's middle)
/// is left alone.
fn add_structure_legs(entries: &mut Vec<Entry>, city: &StreetCity, o: &StreetOptions) {
    let Some(joins) = o.structures.as_ref() else { return };
    let ix = build_endpoint_index(entries, o);
    for (k, end) in joins.ends.iter().enumerate() {
        let near = query_endpoints(&ix, end.at[0], end.at[1], o.node_snap);
        if near.len() == 1 {
            let it = ix.list[near[0]];
            entries[it.entry].structure_ends[if it.at_start { 0 } else { 1 }] = Some(k);
            // the runs the stub continues into end to end (one other run end
            // at each joint, no structure end there), for its ramp.
            let (mut cur, mut toward) = (it.entry, it.at_start);
            let mut offset = 0.0;
            loop {
                let e = &entries[cur];
                offset += e.length;
                if offset > RAMP_WALK_M {
                    break;
                }
                let far = if toward { *e.points.last().unwrap() } else { e.points[0] };
                let next: Vec<usize> = query_endpoints(&ix, far[0], far[1], o.node_snap).into_iter().filter(|&q| !(ix.list[q].entry == cur && ix.list[q].at_start != toward)).collect();
                if next.len() != 1 || joins.ends.iter().any(|j| hypot(j.at[0] - far[0], j.at[1] - far[1]) <= o.node_snap) {
                    break;
                }
                let nx = ix.list[next[0]];
                if nx.entry == it.entry || entries[nx.entry].structure_ramps.iter().any(|r| r.0 == k) {
                    break;
                }
                entries[nx.entry].structure_ramps.push((k, nx.at_start, offset));
                (cur, toward) = (nx.entry, nx.at_start);
            }
            continue;
        }
        if near.len() < 2 {
            continue;
        }
        let Some(seg) = city.segments.get(end.segment) else { continue };
        let mut points = dedupe_points(&end.line);
        if points.len() < 2 || !(end.half > 0.1) {
            continue;
        }
        // a one-way leg runs along its traffic (`Entry::oneway`): reversed
        // when the traffic arrives at the junction (the leg's end is there).
        let along = {
            let (f, l) = (seg.points[0], *seg.points.last().unwrap());
            hypot(f[0] - end.at[0], f[1] - end.at[1]) <= hypot(l[0] - end.at[0], l[1] - end.at[1])
        };
        let flows_along = seg.oneway != "decreasing";
        if seg.is_oneway() && along != flows_along {
            points.reverse();
        }
        let cum = arc_table(&points);
        let length = *cum.last().unwrap();
        if !(length > 0.2) {
            continue;
        }
        let highway = if seg.highway.is_empty() { "residential".to_string() } else { seg.highway.clone() };
        let lanes = match seg.lanes.filter(|l| *l != 0.0 && l.is_finite()) {
            Some(l) => (l.round() as i32).max(1),
            None => ((end.half * 2.0 / 3.2).round() as i32).max(1),
        };
        entries.push(Entry {
            id: format!("{}:structure", seg.id),
            pieces: vec![end.segment],
            road: end.segment,
            points,
            cum,
            length,
            half: end.half,
            walks: [0.0, 0.0],
            trim_start: 0.0,
            trim_end: 0.0,
            marking_trim_start: 0.0,
            marking_trim_end: 0.0,
            class_rank: street_class_rank(&highway),
            highway,
            lanes,
            oneway: seg.is_oneway(),
            approach_start: None,
            approach_end: None,
            mouth_start_m: None,
            mouth_end_m: None,
            kerb_ramps: [vec![], vec![]],
            yield_spans: vec![],
            driveway_owners: [None; 2],
            driveway_cuts: Default::default(),
            walk_yield_spans: [vec![], vec![]],
            structure_leg: Some(k),
            structure_ends: [None, None],
            landings: vec![],
            structure_ramps: vec![],
        });
    }
}

/// A structure leg's end at its junction (the end at its `StructureEnd::at`):
/// true for its start.
pub fn leg_at_start(e: &Entry, o: &StreetOptions) -> bool {
    let Some(at) = e.structure_leg.and_then(|k| o.structures.as_ref()?.ends.get(k)).map(|end| end.at) else { return true };
    let (f, l) = (e.points[0], *e.points.last().unwrap());
    hypot(f[0] - at[0], f[1] - at[1]) <= hypot(l[0] - at[0], l[1] - at[1])
}

const RUN_MERGE_SHORT_M: f64 = 15.0;

fn roadway_composition_differs(a: &Segment, b: &Segment) -> bool {
    let la = a.lanes.unwrap_or(0.0).round() as i64;
    let lb = b.lanes.unwrap_or(0.0).round() as i64;
    if la != lb {
        return true;
    }
    for (x, y) in [(a.parking, b.parking), (a.cycle, b.cycle), (a.shoulder, b.shoulder)] {
        for k in 0..2 {
            if (x[k].max(0.0) - y[k].max(0.0)).abs() > 0.02 {
                return true;
            }
        }
    }
    false
}

/// `mergeShortRuns`.
fn merge_short_runs(mut entries: Vec<Entry>, city: &StreetCity) -> Vec<Entry> {
    let signals: Vec<P2> = city.intersections.iter().filter(|it| it.sig.is_some()).map(|it| [it.x, it.z]).collect();
    let signalised_at = |p: P2| signals.iter().any(|q| hypot(q[0] - p[0], q[1] - p[1]) < 3.0);
    let key = |p: P2| ((p[0] * 20.0).round() as i64, (p[1] * 20.0).round() as i64);
    let mut degree: HashMap<(i64, i64), u32> = HashMap::new();
    for e in &entries {
        for p in [e.points[0], *e.points.last().unwrap()] {
            *degree.entry(key(p)).or_default() += 1;
        }
    }
    let mut by_start: HashMap<(i64, i64), usize> = HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        by_start.insert(key(e.points[0]), i);
    }
    let street_of = |e: &Entry| -> Option<String> {
        let s = &city.segments[e.road];
        if !s.street_name.is_empty() { Some(s.street_name.clone()) } else { s.street_id.clone() }
    };
    let mut removed = vec![false; entries.len()];
    for ei in 0..entries.len() {
        if removed[ei] {
            continue;
        }
        loop {
            let e = &entries[ei];
            let end_key = key(*e.points.last().unwrap());
            if degree.get(&end_key) != Some(&2) {
                break;
            }
            let Some(&ni) = by_start.get(&end_key) else { break };
            if ni == ei || removed[ni] {
                break;
            }
            let next = &entries[ni];
            if !(e.length < RUN_MERGE_SHORT_M || next.length < RUN_MERGE_SHORT_M) {
                break;
            }
            let (se, sn) = (street_of(e), street_of(next));
            if se.is_none() || se != sn {
                break;
            }
            let mut adopt_roadway_next = false;
            if (e.half - next.half).abs() > 0.02 || roadway_composition_differs(&city.segments[e.road], &city.segments[next.road]) {
                let (short_len, long_len) = if e.length <= next.length { (e.length, next.length) } else { (next.length, e.length) };
                if !(short_len < RUN_MERGE_SHORT_M) || !(long_len >= RUN_MERGE_SHORT_M) {
                    break;
                }
                if signalised_at(*next.points.last().unwrap()) {
                    break;
                }
                adopt_roadway_next = e.length <= next.length;
            }
            let walks_differ = (e.walks[0] - next.walks[0]).abs() > 0.02 || (e.walks[1] - next.walks[1]).abs() > 0.02;
            let mut adopt_walks = None;
            if walks_differ {
                let (short, long) = if e.length <= next.length { (e, next) } else { (next, e) };
                if !(short.length < RUN_MERGE_SHORT_M) {
                    break;
                }
                if !(short.walks[0] >= long.walks[0] - 0.02 && short.walks[1] >= long.walks[1] - 0.02) {
                    break;
                }
                adopt_walks = Some(long.walks);
            }
            let d0 = edge_dir(&e.points, e.points.len() - 2);
            let d1 = edge_dir(&next.points, 0);
            let slip = matches!(e.highway.as_str(), "primary_link" | "secondary_link" | "tertiary_link");
            let max_turn = if slip { (45.0f64).to_radians().cos() } else { (6.0f64).to_radians().cos() };
            if d0[0] * d1[0] + d0[1] * d1[1] < max_turn {
                break;
            }
            let next_len = next.length;
            let next_points = next.points.clone();
            let (next_half, next_road, next_class, next_hw, next_lanes, next_oneway) =
                (next.half, next.road, next.class_rank, next.highway.clone(), next.lanes, next.oneway);
            let next_pieces = next.pieces.clone();
            let e = &mut entries[ei];
            if slip {
                if e.mouth_start_m.is_none() {
                    e.mouth_start_m = Some(e.length);
                }
                e.mouth_end_m = Some(next_len);
            }
            if let Some(w) = adopt_walks {
                e.walks = w;
            }
            e.points.extend_from_slice(&next_points[1..]);
            e.cum = arc_table(&e.points);
            e.length = *e.cum.last().unwrap();
            e.pieces.extend(next_pieces);
            if adopt_roadway_next {
                e.half = next_half;
                e.road = next_road;
                e.class_rank = next_class;
                e.highway = next_hw;
                e.lanes = next_lanes;
                e.oneway = next_oneway;
            }
            removed[ni] = true;
        }
    }
    entries.into_iter().zip(removed).filter(|(_, r)| !*r).map(|(e, _)| e).collect()
}

// ---------------------------------------------------------------------------
// nodes
// ---------------------------------------------------------------------------

fn make_approach(entries: &[Entry], ei: usize, at_start: bool) -> Approach {
    let e = &entries[ei];
    let last = e.points.len() - 1;
    let u = if at_start {
        edge_dir(&e.points, 0)
    } else {
        let d = edge_dir(&e.points, last - 1);
        [-d[0], -d[1]]
    };
    Approach {
        entry: ei,
        at_start,
        u,
        angle: norm_angle(u[1].atan2(u[0])),
        half: e.half,
        run_length: e.length,
        mouth_length: (if at_start { e.mouth_start_m } else { e.mouth_end_m }).unwrap_or(e.length),
        width_ccw: if at_start { e.walks[0] } else { e.walks[1] },
        width_cw: if at_start { e.walks[1] } else { e.walks[0] },
        flows_toward: if e.oneway { !at_start } else { true },
        oneway: e.oneway,
        trim: 0.0,
        class_rank: e.class_rank,
        crossing_painted: false,
        paint_reach: 0.0,
        crossing_ramps: [false, false],
        merge_trim: None,
    }
}

#[derive(Clone, Copy)]
struct EndItem {
    entry: usize,
    at_start: bool,
    x: f64,
    z: f64,
}

/// Project a point to the nearest station of the run. Mapped roads can curve
/// back toward a landing, so the ramp follows distance travelled on the road.
pub(crate) fn nearest_station(e: &Entry, at: P2) -> (f64, f64) {
    let mut best = (0.0, f64::INFINITY);
    for (i, w) in e.points.windows(2).enumerate() {
        let (dx, dz) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
        let len = e.cum[i + 1] - e.cum[i];
        if len <= 0.0 {
            continue;
        }
        let t = (((at[0] - w[0][0]) * dx + (at[1] - w[0][1]) * dz) / (len * len)).clamp(0.0, 1.0);
        let dist = (at[0] - w[0][0] - t * dx).hypot(at[1] - w[0][1] - t * dz);
        if dist < best.1 {
            best = (e.cum[i] + t * len, dist);
        }
    }
    best
}

/// Carry an approach over connected paved runs, including every arm of a
/// junction. A street merely passing underneath has no shared endpoint.
pub(crate) fn connect_landings(entries: &mut [Entry], o: &StreetOptions, terrain: &crate::Terrain) {
    if o.landings.is_empty() {
        return;
    }
    let ix = build_endpoint_index(entries, o);
    for &(at, rise) in &o.landings {
        let mut queue = std::collections::VecDeque::new();
        let mut best = HashMap::new();
        let ends = query_endpoints(&ix, at[0], at[1], o.node_snap);
        for k in &ends {
            let end = ix.list[*k];
            queue.push_back((end.entry, if end.at_start { 0.0 } else { entries[end.entry].length }, 0.0));
        }
        if ends.is_empty() {
            for (ei, e) in entries.iter().enumerate() {
                let (s, d) = nearest_station(e, at);
                if d <= o.node_snap {
                    queue.push_back((ei, s, 0.0));
                }
            }
        }
        while let Some((ei, s, travelled)) = queue.pop_front() {
            let key = (ei, s.to_bits());
            if best.get(&key).is_some_and(|d| *d <= travelled) {
                continue;
            }
            best.insert(key, travelled);
            let e = &mut entries[ei];
            // A junction pad meets all of its arms at their trimmed mouths.
            // The longitudinal taper starts outside the pad on each arm.
            let s = if s == 0.0 && e.approach_start.is_some() {
                e.s0()
            } else if s == e.length && e.approach_end.is_some() {
                e.s1()
            } else {
                s
            };
            let to = rise.to - rise.grade * travelled;
            e.landings.push((s, crate::joins::Rise { to, grade: rise.grade }));
            if e.structure_leg.is_some() {
                continue;
            }
            for (end_s, q) in [(0.0, e.points[0]), (e.length, *e.points.last().unwrap())] {
                let distance = (end_s - s).abs();
                if distance == 0.0 {
                    continue;
                }
                let y = to - rise.grade * distance;
                if y <= terrain.ground(q[0], q[1]) + o.road_lift {
                    continue;
                }
                for k in query_endpoints(&ix, q[0], q[1], o.node_snap) {
                    let end = ix.list[k];
                    if end.entry != ei {
                        queue.push_back((end.entry, if end.at_start { 0.0 } else { *entries[end.entry].cum.last().unwrap() }, travelled + distance));
                    }
                }
            }
        }
    }
}

struct EndpointIndex {
    cell: f64,
    map: HashMap<(i64, i64), Vec<usize>>,
    list: Vec<EndItem>,
}

fn build_endpoint_index(entries: &[Entry], o: &StreetOptions) -> EndpointIndex {
    let cell = (o.node_snap * 4.0).max(1.0);
    let mut map: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    let mut list = vec![];
    for (ei, e) in entries.iter().enumerate() {
        let (f, l) = (e.points[0], *e.points.last().unwrap());
        // a structure leg joins a junction at one end only (its far end is
        // on the structure).
        let leg = e.structure_leg.map(|_| leg_at_start(e, o));
        for it in [EndItem { entry: ei, at_start: true, x: f[0], z: f[1] }, EndItem { entry: ei, at_start: false, x: l[0], z: l[1] }] {
            if leg.is_some_and(|start| start != it.at_start) {
                continue;
            }
            let k = ((it.x / cell).floor() as i64, (it.z / cell).floor() as i64);
            map.entry(k).or_default().push(list.len());
            list.push(it);
        }
    }
    EndpointIndex { cell, map, list }
}

fn query_endpoints(ix: &EndpointIndex, x: f64, z: f64, radius: f64) -> Vec<usize> {
    let (cx, cz) = ((x / ix.cell).floor() as i64, (z / ix.cell).floor() as i64);
    let mut out = vec![];
    for i in -1..=1 {
        for j in -1..=1 {
            if let Some(b) = ix.map.get(&(cx + i, cz + j)) {
                for &k in b {
                    let it = ix.list[k];
                    if hypot(it.x - x, it.z - z) <= radius {
                        out.push(k);
                    }
                }
            }
        }
    }
    out.sort_by(|&a, &b| {
        let (ia, ib) = (ix.list[a], ix.list[b]);
        ia.entry.cmp(&ib.entry).then((!ia.at_start).cmp(&!ib.at_start))
    });
    out
}

fn is_link(hw: &str) -> bool {
    hw.ends_with("_link")
}

/// `slipLiesInsideStreet`.
fn slip_lies_inside_street(entries: &[Entry], item: EndItem, items: &[EndItem]) -> bool {
    if !is_link(&entries[item.entry].highway) {
        return false;
    }
    let legs: Vec<&EndItem> = items.iter().filter(|o| !is_link(&entries[o.entry].highway)).collect();
    let off_leg = |p: P2, leg: &EndItem| -> f64 {
        let line = &entries[leg.entry].points;
        let past = entries[leg.entry].half * 3.0;
        let mut best = f64::INFINITY;
        for i in 1..line.len() {
            let (dx, dz) = (line[i][0] - line[i - 1][0], line[i][1] - line[i - 1][1]);
            let mut length = hypot(dx, dz);
            if length == 0.0 {
                length = 1.0;
            }
            let along = ((p[0] - line[i - 1][0]) * dx + (p[1] - line[i - 1][1]) * dz) / length;
            let lo = if i == 1 { -past } else { 0.0 };
            let hi = if i == line.len() - 1 { length + past } else { length };
            let t = clamp(along, lo, hi) / length;
            best = best.min(hypot(p[0] - line[i - 1][0] - dx * t, p[1] - line[i - 1][1] - dz * t));
        }
        best
    };
    let half = entries[item.entry].half;
    legs.len() >= 2
        && entries[item.entry]
            .points
            .iter()
            .all(|p| legs.iter().any(|leg| off_leg(*p, leg) <= entries[leg.entry].half + half * 0.3))
}

fn make_node(entries: &mut [Entry], node_index: usize, id: String, key: Option<String>, position: P2, items: &[EndItem], signal: Option<String>, inferred: bool) -> Node {
    let mut approaches: Vec<Approach> = items.iter().map(|it| make_approach(entries, it.entry, it.at_start)).collect();
    approaches.sort_by(|a, b| a.angle.partial_cmp(&b.angle).unwrap().then(a.half.partial_cmp(&b.half).unwrap()));
    for (k, app) in approaches.iter().enumerate() {
        if app.at_start {
            entries[app.entry].approach_start = Some((node_index, k));
        } else {
            entries[app.entry].approach_end = Some((node_index, k));
        }
    }
    Node {
        id,
        key,
        position,
        signal,
        inferred,
        approaches,
        corners: vec![],
        paths: vec![],
        earns_paint: false,
        band_depth: 0.0,
        max_class_rank: 0,
    }
}

/// `collectNodes`, with `inferNodes` as the stream tiles compile.
fn collect_nodes(city: &StreetCity, entries: &mut [Entry], o: &StreetOptions) -> Vec<Node> {
    let ix = build_endpoint_index(entries, o);
    let mut nodes = vec![];
    let mut consumed = vec![false; ix.list.len()];
    let pick = |entries: &[Entry], cands: Vec<usize>, consumed: &[bool]| -> Vec<usize> {
        let cands: Vec<usize> = cands.into_iter().filter(|k| !consumed[*k]).collect();
        let items: Vec<EndItem> = cands.iter().map(|k| ix.list[*k]).collect();
        cands.into_iter().filter(|k| !slip_lies_inside_street(entries, ix.list[*k], &items)).collect()
    };
    for it in &city.intersections {
        if !it.x.is_finite() || !it.z.is_finite() {
            continue;
        }
        let ks = pick(entries, query_endpoints(&ix, it.x, it.z, o.node_snap), &consumed);
        if ks.len() < 3 {
            continue;
        }
        for &k in &ks {
            consumed[k] = true;
        }
        let items: Vec<EndItem> = ks.iter().map(|k| ix.list[*k]).collect();
        let n = nodes.len();
        nodes.push(make_node(entries, n, it.id.clone(), it.k.clone(), [it.x, it.z], &items, it.sig.clone(), false));
    }
    if o.infer_nodes {
        for k0 in 0..ix.list.len() {
            if consumed[k0] {
                continue;
            }
            let it = ix.list[k0];
            let ks = pick(entries, query_endpoints(&ix, it.x, it.z, o.node_snap), &consumed);
            if ks.len() < 3 {
                continue;
            }
            for &k in &ks {
                consumed[k] = true;
            }
            let items: Vec<EndItem> = ks.iter().map(|k| ix.list[*k]).collect();
            let n = nodes.len();
            let id = format!("inferred:{:.3}:{:.3}", it.x, it.z);
            nodes.push(make_node(entries, n, id, None, [it.x, it.z], &items, None, true));
        }
    }
    nodes
}

// ---------------------------------------------------------------------------
// corners
// ---------------------------------------------------------------------------

struct CornerBase {
    corner: P2,
    sweep: f64,
    base_a: f64,
    base_b: f64,
    nominal_radius: f64,
}

/// `cornerBase`.
fn corner_base(position: P2, a: &Approach, b: &Approach, o: &StreetOptions) -> Option<CornerBase> {
    let sweep = norm_angle(b.angle - a.angle);
    let (min_a, max_a) = (o.corner_min_angle_deg.to_radians(), o.corner_max_angle_deg.to_radians());
    if !(sweep > min_a && sweep < max_a) {
        return None;
    }
    let ma = perp_ccw(a.u);
    let mb = perp_ccw(b.u);
    let a0 = [position[0] + ma[0] * a.half, position[1] + ma[1] * a.half];
    let b0 = [position[0] - mb[0] * b.half, position[1] - mb[1] * b.half];
    let den = cross2(a.u, b.u);
    if den.abs() < 1e-4 {
        return None;
    }
    let t = cross2([b0[0] - a0[0], b0[1] - a0[1]], b.u) / den;
    let corner = [a0[0] + a.u[0] * t, a0[1] + a.u[1] * t];
    if !corner[0].is_finite() || !corner[1].is_finite() {
        return None;
    }
    let walk = a.width_ccw.max(b.width_cw);
    let room = (0.9 * a.half.min(b.half) + 1.5).max(walk + 0.35);
    Some(CornerBase {
        corner,
        sweep,
        base_a: (corner[0] - position[0]) * a.u[0] + (corner[1] - position[1]) * a.u[1],
        base_b: (corner[0] - position[0]) * b.u[0] + (corner[1] - position[1]) * b.u[1],
        nominal_radius: o.corner_radius.max(walk + 0.35).min(room).min(2.5 * o.corner_radius),
    })
}

/// `solveCornerArc`.
pub fn solve_corner_arc(position: P2, a: &Approach, b: &Approach, o: &StreetOptions) -> Option<Corner> {
    let base = corner_base(position, a, b, o)?;
    let half_angle = base.sweep / 2.0;
    let slack = (a.trim - base.base_a).min(b.trim - base.base_b);
    if !(slack > 0.12) {
        return None;
    }
    let radius = base.nominal_radius.min(slack * half_angle.tan());
    if !(radius >= 0.4) {
        return None;
    }
    let d = radius / half_angle.tan();
    let d_a = base.base_a + d;
    let d_b = base.base_b + d;
    if !(d_a > 0.05 && d_b > 0.05) {
        return None;
    }
    let (mut bx, mut bz) = (a.u[0] + b.u[0], a.u[1] + b.u[1]);
    let bl = hypot(bx, bz);
    if bl < 1e-6 {
        return None;
    }
    bx /= bl;
    bz /= bl;
    let cd = radius / half_angle.sin();
    Some(Corner {
        centre: [base.corner[0] + bx * cd, base.corner[1] + bz * cd],
        radius,
        ta: [base.corner[0] + a.u[0] * d, base.corner[1] + a.u[1] * d],
        tb: [base.corner[0] + b.u[0] * d, base.corner[1] + b.u[1] * d],
        d_a,
        d_b,
        corner: base.corner,
        sweep: base.sweep,
        bisector: [bx, bz],
    })
}

/// Arc stations and ramp windows on a fillet (`arcStations`).
pub fn arc_stations(c: &Corner, o: &StreetOptions) -> (Vec<CurbStation>, Vec<RampWin>) {
    let a0 = (c.ta[1] - c.centre[1]).atan2(c.ta[0] - c.centre[0]);
    let a1 = (c.tb[1] - c.centre[1]).atan2(c.tb[0] - c.centre[0]);
    let delta = signed_angle(a1 - a0);
    let step = o.corner_arc_step_deg.to_radians();
    let count = clamp((delta.abs() / step).ceil(), 2.0, 16.0);
    let sweep = delta.abs();
    let ramp_half = (sweep * 0.34).min((o.ramp_width / 2.0) / c.radius);
    let has_ramp = ramp_half > 0.02;
    let sign = if delta >= 0.0 { 1.0 } else { -1.0 };
    let flare = (sweep * 0.12).min((0.24f64).max(o.ramp_run * 0.25) / c.radius);
    let mut windows: Vec<RampWin> = vec![];
    let win = |centre: f64| RampWin {
        lo: (centre - ramp_half * sign).min(centre + ramp_half * sign),
        hi: (centre - ramp_half * sign).max(centre + ramp_half * sign),
        flare_lo: 0.0,
        flare_hi: 0.0,
        node: usize::MAX,
        at_start: false,
    };
    if has_ramp {
        let pair_span = 2.0 * (2.0 * ramp_half + flare) + 0.5 * ramp_half;
        if sweep >= pair_span {
            windows.push(win(a0 + sign * (flare + ramp_half)));
            windows.push(win(a0 + delta - sign * (flare + ramp_half)));
        } else {
            windows.push(win(a0 + delta / 2.0));
        }
    }
    let arc_lo = a0.min(a0 + delta);
    let arc_hi = a0.max(a0 + delta);
    let wanted = if c.radius > 1e-6 { o.ramp_flare_run / c.radius } else { 0.0 };
    windows.sort_by(|p, q| p.lo.partial_cmp(&q.lo).unwrap());
    let n = windows.len();
    for i in 0..n {
        let room_lo = windows[i].lo - if i == 0 { arc_lo } else { windows[i - 1].hi };
        let room_hi = (if i == n - 1 { arc_hi } else { windows[i + 1].lo }) - windows[i].hi;
        let w = &mut windows[i];
        w.flare_lo = w.lo - wanted.min(if i == 0 { room_lo } else { room_lo * 0.45 }).max(0.0);
        w.flare_hi = w.hi + wanted.min(if i == n - 1 { room_hi } else { room_hi * 0.45 }).max(0.0);
    }
    let chord = sweep / count;
    let mut bounds = vec![a0];
    for w in &windows {
        for ang in [w.lo, w.hi] {
            let along = (ang - a0) * sign;
            if along > 1e-6 && along < sweep - 1e-6 {
                bounds.push(ang);
            }
        }
    }
    bounds.push(a0 + delta);
    if delta >= 0.0 {
        bounds.sort_by(|p, q| p.partial_cmp(q).unwrap());
    } else {
        bounds.sort_by(|p, q| q.partial_cmp(p).unwrap());
    }
    let mut angles = vec![];
    for k in 0..bounds.len() - 1 {
        let (lo, hi) = (bounds[k], bounds[k + 1]);
        let pieces = (((hi - lo).abs() / chord - 1e-6).ceil() as usize).max(1);
        for i in 0..pieces {
            angles.push(lo + (hi - lo) * i as f64 / pieces as f64);
        }
    }
    angles.push(a0 + delta);
    let mut stations: Vec<CurbStation> = vec![];
    for ang in angles {
        if let Some(l) = stations.last() {
            if (ang - l.ang.unwrap()).abs() < 1e-6 {
                continue;
            }
        }
        stations.push(CurbStation {
            x: c.centre[0] + ang.cos() * c.radius,
            z: c.centre[1] + ang.sin() * c.radius,
            out: [-ang.cos(), -ang.sin()],
            scale: 1.0,
            walk: 0.0,
            ang: Some(ang),
            mouth: None,
        });
    }
    (stations, windows)
}

// ---------------------------------------------------------------------------
// trims
// ---------------------------------------------------------------------------

/// `planJunction`: how far each approach carriageway is trimmed back.
fn plan_junction(node: &mut Node, entries: &mut [Entry], o: &StreetOptions) {
    let count = node.approaches.len();
    let mut want = vec![0.0; count];
    let slip: Vec<bool> = node.approaches.iter().map(|a| is_link(&entries[a.entry].highway)).collect();
    let has_through = slip.contains(&false);
    for i in 0..count {
        let app = &node.approaches[i];
        if has_through && slip[i] {
            let mut clear = app.half + 0.3;
            for j in 0..count {
                if slip[j] {
                    continue;
                }
                let sin = cross2(app.u, node.approaches[j].u).abs();
                clear = clear.max((node.approaches[j].half + app.half * (1.0 - sin * sin).max(0.0).sqrt()) / sin.max(0.05));
            }
            want[i] = clear;
            continue;
        }
        let legs: Vec<usize> = (0..count).filter(|&j| !has_through || !slip[j]).collect();
        let mut base = app.half + 0.3;
        for &j in &legs {
            if j != i {
                base = base.max(node.approaches[j].half + 0.3);
            }
        }
        let at = legs.iter().position(|&j| j == i).unwrap();
        let next = &node.approaches[legs[(at + 1) % legs.len()]];
        let prev = &node.approaches[legs[(at + legs.len() - 1) % legs.len()]];
        let tight = if legs.len() > 1 { norm_angle(next.angle - app.angle).min(norm_angle(app.angle - prev.angle)) } else { PI };
        let half_sweep = clamp(tight / 2.0, 0.09, PI / 2.0 - 1e-3);
        base = base.max((app.half / half_sweep.tan()).min(4.0 * app.half + 6.0));
        want[i] = base;
    }
    for i in 0..count {
        let j = (i + 1) % count;
        if i == j {
            continue;
        }
        let Some(geo) = corner_base(node.position, &node.approaches[i], &node.approaches[j], o) else { continue };
        let d = geo.nominal_radius / (geo.sweep / 2.0).tan();
        if !has_through || !slip[j] || slip[i] {
            want[i] = want[i].max(geo.base_a + d);
        }
        if !has_through || !slip[i] || slip[j] {
            want[j] = want[j].max(geo.base_b + d);
        }
    }
    for i in 0..count {
        let app = &mut node.approaches[i];
        let e = &mut entries[app.entry];
        let far_is_junction = if app.at_start { e.approach_end.is_some() } else { e.approach_start.is_some() };
        let no_kerb_return = !(e.walks[0] > 0.0) && !(e.walks[1] > 0.0);
        let cap = (if far_is_junction && !no_kerb_return { 0.45 } else { 0.85 }) * app.mouth_length;
        app.trim = clamp(want[i], 0.05, cap.max(0.05));
        if app.at_start {
            e.trim_start = e.trim_start.max(app.trim);
        } else {
            e.trim_end = e.trim_end.max(app.trim);
        }
    }
    // (`merge_trim`) a slip alongside a through leg, neither with a walk.
    if has_through && crate::seams::road_seams() {
        let walkless = |a: &Approach| !(entries[a.entry].walks[0] >= o.min_sidewalk_width) && !(entries[a.entry].walks[1] >= o.min_sidewalk_width);
        for i in 0..count {
            if !slip[i] || !walkless(&node.approaches[i]) {
                continue;
            }
            let ui = node.approaches[i].u;
            let partner = (0..count)
                .filter(|&j| !slip[j])
                .map(|j| (j, ui[0] * node.approaches[j].u[0] + ui[1] * node.approaches[j].u[1]))
                .filter(|&(_, c)| c > MERGE_COS)
                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
            if let Some((j, _)) = partner {
                let t = node.approaches[j].trim;
                if t < node.approaches[i].trim - 1e-6 {
                    node.approaches[i].merge_trim = Some(t);
                }
            }
        }
    }
}

/// `reconcileTrims`.
fn reconcile_trims(entries: &mut [Entry], nodes: &mut [Node]) {
    for e in entries.iter_mut() {
        let min_run = (0.5f64).min(0.34 * e.length);
        let total = e.trim_start + e.trim_end;
        let budget = (e.length - min_run).max(0.0);
        if total > budget && total > 1e-9 {
            let k = budget / total;
            e.trim_start *= k;
            e.trim_end *= k;
        }
        if let Some((n, a)) = e.approach_start {
            nodes[n].approaches[a].trim = e.trim_start;
        }
        if let Some((n, a)) = e.approach_end {
            nodes[n].approaches[a].trim = e.trim_end;
        }
    }
}

// ---------------------------------------------------------------------------
// node paint (heuristic) and the ribbon's crossing ramps
// ---------------------------------------------------------------------------

/// `crossingBandDepth`.
pub fn crossing_band_depth(max_rank: i32, o: &StreetOptions) -> f64 {
    let base = o.crosswalk_band_depth;
    if max_rank >= 6 {
        base + 1.2
    } else if max_rank >= 5 {
        base + 0.6
    } else {
        base
    }
}

/// `nodeEarnsJunctionPaint`.
pub fn node_earns_junction_paint(signalised: bool, degree: usize, max_rank: i32, footway_legs: usize) -> bool {
    if signalised {
        return true;
    }
    if degree < 3 {
        return false;
    }
    if max_rank < 4 {
        return false;
    }
    footway_legs >= 3
}

const RAMP_MIN_CLEAR_WIDTH: f64 = 1.2;

/// `planNodePaint` without crossing evidence, and `planCrossingRamps`.
fn plan_node_paint(ni: usize, node: &mut Node, entries: &mut [Entry], o: &StreetOptions, ev: Option<&crate::evidence::Evidence>, city: &StreetCity) {
    let mut max_rank = 0;
    let mut footway_legs = 0;
    for app in &mut node.approaches {
        let rank = entries[app.entry].class_rank;
        app.class_rank = rank;
        max_rank = max_rank.max(rank);
        if rank >= 3 && app.width_ccw.max(app.width_cw) >= o.min_sidewalk_width {
            footway_legs += 1;
        }
    }
    let signalised = node.signalised();
    node.max_class_rank = max_rank;
    let heuristic = node_earns_junction_paint(signalised, node.approaches.len(), max_rank, footway_legs);
    node.earns_paint = heuristic;
    for app in &mut node.approaches {
        app.crossing_painted = heuristic && (signalised || app.class_rank >= 3);
    }
    // WHICH LEGS ARE MARKED IS DATA (crossing-evidence.js): the data decides a
    // leg it speaks for; the class rule stands where it is silent.
    if let Some(ev) = ev {
        let street_of = |i: usize| city.segments[entries[node.approaches[i].entry].road].street_name.clone();
        let legs = ev.annotate(ni, node, entries, &street_of);
        for (app, leg) in node.approaches.iter_mut().zip(&legs) {
            if let Some(m) = leg.marked {
                app.crossing_painted = m;
            }
        }
        if !signalised && !heuristic {
            node.earns_paint = node.approaches.len() >= 3
                && node.approaches.iter().zip(&legs).any(|(a, l)| (a.crossing_painted || l.listed_through) && a.class_rank >= 3);
        }
    }
    // a structure leg carries no crosswalk (its mouth is the structure's deck).
    for app in &mut node.approaches {
        if entries[app.entry].structure_leg.is_some() {
            app.crossing_painted = false;
        }
    }
    node.band_depth = crossing_band_depth(max_rank, o);
    for app in &mut node.approaches {
        let reach = if node.earns_paint {
            app.trim + o.crosswalk_clearance + node.band_depth + o.stop_bar_clearance
        } else {
            app.trim + o.marking_box_clearance
        };
        app.paint_reach = reach;
        let e = &mut entries[app.entry];
        if app.at_start {
            e.marking_trim_start = e.marking_trim_start.max(reach);
        } else {
            e.marking_trim_end = e.marking_trim_end.max(reach);
        }
        // planCrossingRamps
        app.crossing_ramps = [false, false];
        if !node.earns_paint || (!signalised && !(app.class_rank >= 3)) {
            continue;
        }
        let available = app.run_length - if app.at_start { e.trim_end } else { e.trim_start };
        let band_start = app.trim + o.crosswalk_clearance;
        let band_end = band_start + node.band_depth;
        if band_end + 0.4 > available {
            continue;
        }
        let flat = clamp(o.ramp_width, RAMP_MIN_CLEAR_WIDTH, node.band_depth);
        let mid = (band_start + band_end) / 2.0;
        let (d_lo, d_hi) = (mid - flat / 2.0, mid + flat / 2.0);
        let inward = o.ramp_flare_run.min(d_lo - app.trim).max(0.0);
        let outward = o.ramp_flare_run.min(available - 0.05 - d_hi).max(0.0);
        let ramp_reach = o.curb_top_width + o.ramp_run;
        for (k, ccw) in [true, false].into_iter().enumerate() {
            let walk = if ccw { app.width_ccw } else { app.width_cw };
            if !(walk > ramp_reach + 0.05) {
                continue;
            }
            let side = (if app.at_start { 1 } else { -1 }) * (if ccw { 1 } else { -1 });
            let w = if app.at_start {
                RampWin { lo: d_lo, hi: d_hi, flare_lo: d_lo - inward, flare_hi: d_hi + outward, node: ni, at_start: true }
            } else {
                RampWin {
                    lo: e.length - d_hi,
                    hi: e.length - d_lo,
                    flare_lo: e.length - d_hi - outward,
                    flare_hi: e.length - d_lo + inward,
                    node: ni,
                    at_start: false,
                }
            };
            e.kerb_ramps[if side > 0 { 0 } else { 1 }].push(w);
            app.crossing_ramps[k] = true;
        }
    }
}

/// `resolveKerbRamps`: the reserved windows that fit the emitted run, [left, right].
pub fn resolve_kerb_ramps(e: &Entry) -> [Vec<RampWin>; 2] {
    let (s0, s1) = (e.s0(), e.s1());
    let mut out = [vec![], vec![]];
    for k in 0..2 {
        let mut wins = e.kerb_ramps[k].clone();
        wins.sort_by(|p, q| p.lo.partial_cmp(&q.lo).unwrap());
        for i in 0..wins.len() {
            let room_lo = if i == 0 { s0 } else { (wins[i - 1].hi + wins[i].lo) / 2.0 };
            let room_hi = if i == wins.len() - 1 { s1 } else { (wins[i].hi + wins[i + 1].lo) / 2.0 };
            let mut w = wins[i];
            if !(w.lo > room_lo + 1e-3 && w.hi < room_hi - 1e-3) {
                continue;
            }
            w.flare_lo = clamp(w.flare_lo, room_lo, w.lo);
            w.flare_hi = clamp(w.flare_hi, w.hi, room_hi);
            out[k].push(w);
        }
    }
    out
}

/// `segmentKerbRamps`: the crossing ramps a ribbon really cuts, [left, right].
pub fn segment_kerb_ramps(e: &Entry, o: &StreetOptions) -> [Vec<RampWin>; 2] {
    let resolved = resolve_kerb_ramps(e);
    let reach = o.curb_top_width + o.ramp_run;
    let mut out = [vec![], vec![]];
    for k in 0..2 {
        let walk = e.walks[k];
        if !(walk >= o.min_sidewalk_width) || !(walk > reach + 0.05) {
            continue;
        }
        out[k] = resolved[k]
            .iter()
            .filter(|w| {
                let mid = (w.lo + w.hi) / 2.0;
                !e.in_yield_span(mid) && !e.walk_yield_spans[k].iter().any(|g| mid >= g[0] && mid <= g[1])
            })
            .copied()
            .collect();
    }
    out
}

// ---------------------------------------------------------------------------
// carriageway yield (`planCarriagewayYield`)
// ---------------------------------------------------------------------------

struct CorridorIndex {
    cell: f64,
    buckets: HashMap<(i64, i64), Vec<usize>>,
    edges: Vec<CorridorEdge>,
}

#[derive(Clone, Copy)]
struct CorridorEdge {
    a: P2,
    b: P2,
    half: f64,
    owner: usize,
    first: bool,
    last: bool,
    arc0: f64,
}

impl CorridorIndex {
    fn new(cell: f64) -> Self {
        Self { cell, buckets: HashMap::new(), edges: vec![] }
    }
    fn add(&mut self, e: CorridorEdge) {
        let i = self.edges.len();
        self.edges.push(e);
        let (minx, maxx) = (e.a[0].min(e.b[0]) - e.half, e.a[0].max(e.b[0]) + e.half);
        let (minz, maxz) = (e.a[1].min(e.b[1]) - e.half, e.a[1].max(e.b[1]) + e.half);
        for gz in (minz / self.cell).floor() as i64..=(maxz / self.cell).floor() as i64 {
            for gx in (minx / self.cell).floor() as i64..=(maxx / self.cell).floor() as i64 {
                self.buckets.entry((gx, gz)).or_default().push(i);
            }
        }
    }
}

/// The junction pads (the hull of each node's approaches' trimmed mouths,
/// shrunk `PAD_SHRINK_M`), gridded, for the walk yield.
#[derive(Default)]
pub(crate) struct PadIndex {
    /// (hull, its node's approach entries, the node)
    hulls: Vec<(Vec<P2>, Vec<usize>, usize)>,
    cells: HashMap<(i64, i64), Vec<usize>>,
}

/// A walk point this far inside a pad's hull edge is on the pad (the mouths'
/// own walks meet the hull's edge there).
const PAD_SHRINK_M: f64 = 0.5;
const PAD_CELL_M: f64 = 24.0;

impl PadIndex {
    pub(crate) fn new(nodes: &[Node], entries: &[Entry]) -> Self {
        let mut out = PadIndex::default();
        for (ni, n) in nodes.iter().enumerate() {
            if n.approaches.len() < 3 {
                continue;
            }
            let mut pts: Vec<P2> = vec![];
            for app in &n.approaches {
                let f = approach_end_frame(entries, app);
                pts.push(f.at(-app.half));
                pts.push(f.at(app.half));
            }
            let hull = convex_hull(pts);
            if hull.len() < 3 {
                continue;
            }
            let k = out.hulls.len();
            let (mut x0, mut x1, mut z0, mut z1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
            for p in &hull {
                x0 = x0.min(p[0]);
                x1 = x1.max(p[0]);
                z0 = z0.min(p[1]);
                z1 = z1.max(p[1]);
            }
            for gz in (z0 / PAD_CELL_M).floor() as i64..=(z1 / PAD_CELL_M).floor() as i64 {
                for gx in (x0 / PAD_CELL_M).floor() as i64..=(x1 / PAD_CELL_M).floor() as i64 {
                    out.cells.entry((gx, gz)).or_default().push(k);
                }
            }
            out.hulls.push((hull, n.approaches.iter().map(|a| a.entry).collect(), ni));
        }
        out
    }

    /// (x, z) lies on the pad of a junction run `me` is not an approach of.
    fn on_pad(&self, x: f64, z: f64, me: usize) -> bool {
        self.find(x, z, |apps, _| !apps.contains(&me))
    }

    /// (x, z) lies on the pad of a junction other than node `ni` (a corner
    /// return of a junction in a cluster, lying on its neighbour's pad).
    pub(crate) fn on_other_pad(&self, x: f64, z: f64, ni: usize) -> bool {
        self.find(x, z, |_, n| n != ni)
    }


    /// (x, z) lies more than `depth` inside node `ni`'s own pad: a corner
    /// return's walk there lies in its junction's roadway (an approach trimmed
    /// short, its mouth inside the cross street's).
    pub(crate) fn deep_in_own_pad(&self, x: f64, z: f64, ni: usize, depth: f64) -> bool {
        self.find_by(x, z, depth, |_, n| n == ni)
    }

    fn find(&self, x: f64, z: f64, keep: impl Fn(&[usize], usize) -> bool) -> bool {
        self.find_by(x, z, PAD_SHRINK_M, keep)
    }

    fn find_by(&self, x: f64, z: f64, shrink: f64, keep: impl Fn(&[usize], usize) -> bool) -> bool {
        let Some(list) = self.cells.get(&((x / PAD_CELL_M).floor() as i64, (z / PAD_CELL_M).floor() as i64)) else { return false };
        list.iter().any(|&k| {
            let (h, apps, n) = &self.hulls[k];
            keep(apps, *n)
                && (0..h.len()).all(|i| {
                    let (a, b) = (h[i], h[(i + 1) % h.len()]);
                    let (ex, ez) = (b[0] - a[0], b[1] - a[1]);
                    let l = hypot(ex, ez).max(1e-9);
                    (ex * (z - a[1]) - ez * (x - a[0])) / l >= shrink
                })
        })
    }
}

/// Monotone chain convex hull, counter-clockwise (x, z).
pub(crate) fn convex_hull(mut p: Vec<P2>) -> Vec<P2> {
    p.sort_by(|a, b| a.partial_cmp(b).unwrap());
    p.dedup();
    if p.len() < 3 {
        return p;
    }
    let cross = |o: P2, a: P2, b: P2| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let mut h: Vec<P2> = vec![];
    for pass in 0..2 {
        let start = h.len();
        let it: Box<dyn Iterator<Item = &P2>> = if pass == 0 { Box::new(p.iter()) } else { Box::new(p.iter().rev()) };
        for &q in it {
            while h.len() >= start + 2 && cross(h[h.len() - 2], h[h.len() - 1], q) <= 0.0 {
                h.pop();
            }
            h.push(q);
        }
        h.pop();
    }
    h
}

/// (`--no-walk-yield-strict`, `EO_WALK_YIELD_STRICT=0`) a walk keeps the
/// junction guard over any lesser road's mouth, as the JS did; on (the
/// default), only over the mouth of a road its own run meets at a junction.
pub fn walk_yield_strict() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("EO_WALK_YIELD_STRICT").map(|v| v.trim() != "0" && v.trim() != "off").unwrap_or(true))
}

pub(crate) fn carriageway_order(entries: &[Entry], city: &StreetCity) -> Vec<usize> {
    priority_order(entries, city).0
}

/// The drawn runs busiest first (class, then width, then the segment's seed),
/// and every run's position in that order (`usize::MAX` for a structure leg,
/// which is not drawn). The JS indexes each run's corridors after probing it,
/// in this order, so a run is probed against the runs before it: the same
/// answer as one full index read through `rank < mine`, which lets every run
/// be probed at once (`plan_carriageway_yield`; `seams`).
pub fn priority_order(entries: &[Entry], city: &StreetCity) -> (Vec<usize>, Vec<usize>) {
    let mut order: Vec<usize> = (0..entries.len()).filter(|&e| entries[e].structure_leg.is_none()).collect();
    order.sort_by(|&a, &b| {
        let (ea, eb) = (&entries[a], &entries[b]);
        eb.class_rank
            .cmp(&ea.class_rank)
            .then(eb.half.partial_cmp(&ea.half).unwrap())
            .then(city.segments[ea.pieces[0]].seed().cmp(city.segments[eb.pieces[0]].seed()))
    });
    let mut rank = vec![usize::MAX; entries.len()];
    for (i, &e) in order.iter().enumerate() {
        rank[e] = i;
    }
    (order, rank)
}

fn plan_carriageway_yield(entries: &mut [Entry], nodes: &[Node], o: &StreetOptions, city: &StreetCity) {
    let mut neighbours: Vec<HashSet<usize>> = vec![HashSet::new(); entries.len()];
    for n in nodes {
        for a in &n.approaches {
            for b in &n.approaches {
                if a.entry != b.entry {
                    neighbours[a.entry].insert(b.entry);
                }
            }
        }
    }
    // (structure legs are not drawn: they neither yield nor cover.)
    let (order, rank) = priority_order(entries, city);
    let corner_close = 0.06;
    let mut cover = CorridorIndex::new(24.0);
    let mut road = CorridorIndex::new(24.0);
    for &ei in &order {
        let e = &entries[ei];
        let cov = e.half + (e.walks[0].min(e.walks[1]) - corner_close).max(0.0);
        for i in 0..e.points.len() - 1 {
            cover.add(CorridorEdge { a: e.points[i], b: e.points[i + 1], half: cov, owner: ei, first: i == 0, last: i == e.points.len() - 2, arc0: e.cum[i] });
            road.add(CorridorEdge { a: e.points[i], b: e.points[i + 1], half: e.half, owner: ei, first: false, last: false, arc0: e.cum[i] });
        }
    }
    // (`walk_yield_strict`) a run crossing an earlier one (more than 45 degrees
    // off it, no junction drawn there: two service roads mapped across each
    // other, a short stub's mouth inside the road it joins) gives the span its
    // centreline lies on the other's carriageway up: the clearance (`shrink`,
    // the run's whole reach) is for a run alongside, and kept both ribbons
    // drawn coplanar there, fighting.
    let strict = walk_yield_strict();
    let covered_by = |x: f64, z: f64, shrink: f64, mine: usize, ux: f64, uz: f64| -> Option<usize> {
        let (gx, gz) = ((x / cover.cell).floor() as i64, (z / cover.cell).floor() as i64);
        let mut best = None;
        let mut best_depth = 0.0;
        for j in -1..=1 {
            for i in -1..=1 {
                let Some(list) = cover.buckets.get(&(gx + i, gz + j)) else { continue };
                for &k in list {
                    let e = cover.edges[k];
                    if rank[e.owner] >= mine {
                        continue;
                    }
                    let (dx, dz) = (e.b[0] - e.a[0], e.b[1] - e.a[1]);
                    let len2 = dx * dx + dz * dz;
                    let mut t = if len2 > 1e-9 { ((x - e.a[0]) * dx + (z - e.a[1]) * dz) / len2 } else { 0.0 };
                    if (t < -1e-6 && e.first) || (t > 1.0 + 1e-6 && e.last) {
                        continue;
                    }
                    t = clamp(t, 0.0, 1.0);
                    let (ex, ez) = (x - (e.a[0] + dx * t), z - (e.a[1] + dz * t));
                    // (a stub raised to a structure too: where it crosses another run its
                    // ramp is back at the datum, and the two stood coplanar, fighting)
                    let crossing = strict && len2 > 1e-9 && ((dx * ux + dz * uz) / len2.sqrt()).abs() < std::f64::consts::FRAC_1_SQRT_2;
                    let reach = e.half - if crossing { 0.0 } else { shrink };
                    if ex > reach || ex < -reach || ez > reach || ez < -reach {
                        continue;
                    }
                    let depth = reach - hypot(ex, ez);
                    if depth >= 0.0 && depth > best_depth {
                        best_depth = depth;
                        best = Some(e.owner);
                    }
                }
            }
        }
        best
    };
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(16);
    let chunk = entries.len().div_ceil(threads).max(1);
    let ids: Vec<usize> = (0..entries.len()).collect();
    // (`walk_yield_strict`) the junction pads, for both passes.
    let pads = if walk_yield_strict() { PadIndex::new(nodes, entries) } else { PadIndex::default() };
    // Pass 1: the ribbon spans each run gives up.
    let spans1: Vec<Vec<(f64, f64, usize)>> = {
        let entries_r: &[Entry] = entries;
        let run = |ei: usize| -> Vec<(f64, f64, usize)> {
            let e = &entries_r[ei];
            if e.structure_leg.is_some() {
                return vec![];
            }
            let (half, length) = (e.half, e.length);
            let reach = e.half + e.walks[0].max(e.walks[1]);
            let clear = reach * clamp(o.yield_clear_fraction, 0.0, 2.0);
            let probe_step = (0.75f64).max((2.0f64).min(half + 0.5));
            let mine = rank[ei];
            let at = |s: f64| e.frame(s, false);
            let mut spans: Vec<(f64, f64, usize)> = vec![];
            let mut run_start = -1.0;
            let mut prev: Option<usize> = None;
            let mut s: f64 = 0.0;
            loop {
                let p = s.min(length);
                let f = at(p);
                // (`walk_yield_strict`) a centreline crossing a junction's pad it
                // is not an approach of gives that span up too (the pad draws it).
                let id = covered_by(f.x, f.z, clear, mine, f.tx, f.tz).or_else(|| pads.on_pad(f.x, f.z, ei).then_some(usize::MAX));
                if id.is_some() && prev.is_none() {
                    run_start = p;
                }
                if id.is_none() {
                    if let Some(pid) = prev {
                        spans.push((run_start, p, pid));
                        run_start = -1.0;
                    }
                }
                prev = id;
                if p >= length {
                    break;
                }
                s += probe_step;
                if s > length + 1e-6 {
                    s = length;
                }
            }
            if let Some(pid) = prev {
                if run_start >= 0.0 {
                    spans.push((run_start, length, pid));
                }
            }
            let refine = |lo: f64, hi: f64, want: bool| {
                let (mut a, mut b) = (lo, hi);
                for _ in 0..12 {
                    let mid = (a + b) / 2.0;
                    let f = at(mid);
                    if (covered_by(f.x, f.z, clear, mine, f.tx, f.tz).is_some() || pads.on_pad(f.x, f.z, ei)) == want {
                        b = mid;
                    } else {
                        a = mid;
                    }
                }
                if o.structures.is_none() { return (a + b) / 2.0; }
                // Triangles store f32 world positions. Keep the drawn road up
                // to the inside of the owning ribbon despite vertex rounding.
                let f = at(b);
                let rounding = 2.0 * f32::EPSILON as f64 * f.x.abs().max(f.z.abs());
                if want { (b + rounding).min(length) } else { (a - rounding).max(0.0) }
            };
            for sp in spans.iter_mut() {
                if sp.0 > 0.0 {
                    sp.0 = refine((sp.0 - probe_step).max(0.0), sp.0, true);
                }
                if sp.1 < length {
                    sp.1 = refine((sp.1 - probe_step).max(sp.0), sp.1, false);
                }
            }
            let (s0, s1) = (e.trim_start, length - e.trim_end);
            let drawable = (s1 - s0).max(1e-3);
            let slip_road = is_link(&e.highway);
            spans
                .into_iter()
                .filter(|&(a, b, id)| {
                    if slip_road || b - a < 1.0 || b <= s0 + 0.25 || a >= s1 - 0.25 {
                        return false;
                    }
                    if a > 0.25 && b < length - 0.25 {
                        return true;
                    }
                    if !neighbours[ei].contains(&id) {
                        return true;
                    }
                    if id == usize::MAX {
                        return true; // on another junction's pad
                    }
                    let cover_frac = (b.min(s1) - a.max(s0)) / drawable;
                    if cover_frac < o.presentation_duplicate_cover {
                        return false;
                    }
                    entries_r[id].half > half + o.presentation_duplicate_half_slack
                })
                .collect()
        };
        std::thread::scope(|sc| {
            let hs: Vec<_> = ids.chunks(chunk).map(|c| { let run = &run; sc.spawn(move || c.iter().map(|&ei| run(ei)).collect::<Vec<_>>()) }).collect();
            hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
        })
    };
    for (e, spans) in entries.iter_mut().zip(spans1) {
        e.yield_spans.clear();
        e.driveway_owners = [None; 2];
        for (a, b, owner) in spans {
            e.yield_spans.push([a, b]);
            if e.highway == "service" && e.walks.iter().all(|w| *w < o.min_sidewalk_width) {
                if a == 0.0 && e.approach_start.is_none() { e.driveway_owners[0] = Some(owner); }
                if b == e.length && e.approach_end.is_none() { e.driveway_owners[1] = Some(owner); }
            }
        }
    }
    // Pass 2: footway strips lying on somebody else's drawn road.
    let crossover_cos = (20.0f64).to_radians().cos();
    let entries_r: &[Entry] = entries;
    // (`walk_yield_strict`) and on a junction's pad its run is not an approach
    // of (`pads`): the pad's fan stands at its approaches' datums, and a walk
    // of another run under or over it is a slab.
    let on_roadway = |x: f64, z: f64, me: usize, ux: f64, uz: f64| -> bool {
        let (gx, gz) = ((x / road.cell).floor() as i64, (z / road.cell).floor() as i64);
        let Some(list) = road.buckets.get(&(gx, gz)) else { return false };
        for &k in list {
            let e = road.edges[k];
            let (dx, dz) = (e.b[0] - e.a[0], e.b[1] - e.a[1]);
            let len2 = dx * dx + dz * dz;
            let raw = if len2 > 1e-9 { ((x - e.a[0]) * dx + (z - e.a[1]) * dz) / len2 } else { 0.0 };
            let t = clamp(raw, 0.0, 1.0);
            if hypot(x - (e.a[0] + dx * t), z - (e.a[1] + dz * t)) > e.half - 0.35 {
                continue;
            }
            let owner = e.owner;
            if owner == me {
                continue;
            }
            let (ow, se) = (&entries_r[owner], &entries_r[me]);
            if ow.class_rank <= 1 && se.class_rank > ow.class_rank {
                let el = len2.sqrt().max(1e-12);
                let cos = ((dx * ux + dz * uz) / el).abs();
                if cos < crossover_cos {
                    continue;
                }
            }
            let station = e.arc0 + len2.sqrt() * raw;
            // (`seams`: a slip merging alongside a through road is drawn from the
            // through leg's mouth, `Approach::merge_trim`; a walk of that road lying
            // on it there gives way, no junction guard: the slip's lane takes its place)
            let merge = |end: Option<(usize, usize)>| end.and_then(|(n, a)| nodes[n].approaches[a].merge_trim);
            let (m0, m1) = (merge(ow.approach_start), merge(ow.approach_end));
            let (t0, t1) = (m0.map_or(ow.trim_start, |t| t.min(ow.trim_start)), m1.map_or(ow.trim_end, |t| t.min(ow.trim_end)));
            if station < t0 - 0.05 || station > ow.length - t1 + 0.05 || ow.in_yield_span(station) {
                continue;
            }
            let merging = (m0.is_some() && station < ow.trim_start) || (m1.is_some() && station > ow.length - ow.trim_end);
            // (the guard keeps a walk across a lesser road's mouth at a junction
            // both runs meet at, where the corner returns take over; two runs
            // that share no junction keep no such walk: it would lie on the
            // other's roadway at its own level, a slab over or under it.)
            if !merging && rank[owner] > rank[me] && (!walk_yield_strict() || neighbours[me].contains(&owner)) {
                let guard = se.half + se.walks[0].max(se.walks[1]) + 0.5;
                if station < ow.trim_start + guard || station > ow.length - ow.trim_end - guard {
                    continue;
                }
            }
            return true;
        }
        false
    };
    let run2 = |ei: usize| -> [Vec<[f64; 2]>; 2] {
        let e = &entries_r[ei];
        if e.structure_leg.is_some() {
            return [vec![], vec![]];
        }
        let probe_step = (0.75f64).max((2.0f64).min(e.half + 0.5));
        let mut result: [Vec<[f64; 2]>; 2] = [vec![], vec![]];
        for (k, side) in [(0usize, 1.0f64), (1, -1.0)] {
            let walk = e.walks[k];
            if !(walk >= o.min_sidewalk_width) {
                continue;
            }
            let laterals: Vec<f64> = [0.15, 0.5, 0.85].iter().map(|f| side * (e.half + walk * f)).collect();
            let probe = |sv: f64| -> bool {
                let f = e.frame(sv, false);
                let mut hits = 0;
                for (q, &lat) in laterals.iter().enumerate() {
                    let (px, pz) = (f.x + f.nx * lat * f.miter, f.z + f.nz * lat * f.miter);
                    if on_roadway(px, pz, ei, f.nz, -f.nx) || pads.on_pad(px, pz, ei) {
                        hits += 1;
                        // (`walk_yield_strict`) a walk whose outer edge lies on
                        // another road (a divided street's median-side walk,
                        // wider than its median, across the other carriageway)
                        // gives way too: it was a slab a kerb over those lanes.
                        if strict && q == laterals.len() - 1 {
                            return true;
                        }
                    }
                    if hits >= 2 {
                        return true;
                    }
                }
                false
            };
            let mut spans: Vec<[f64; 2]> = vec![];
            let mut run = -1.0;
            let mut prev = false;
            let mut sv: f64 = 0.0;
            loop {
                let pv = sv.min(e.length);
                let hit = probe(pv);
                if hit && !prev {
                    run = pv;
                }
                if !hit && prev {
                    spans.push([run, pv]);
                    run = -1.0;
                }
                prev = hit;
                if pv >= e.length {
                    break;
                }
                sv += probe_step;
                if sv > e.length + 1e-6 {
                    sv = e.length;
                }
            }
            if prev && run >= 0.0 {
                spans.push([run, e.length]);
            }
            let refine = |lo: f64, hi: f64, want: bool| {
                let (mut a, mut b) = (lo, hi);
                for _ in 0..12 {
                    let mid = (a + b) / 2.0;
                    if probe(mid) == want {
                        b = mid;
                    } else {
                        a = mid;
                    }
                }
                (a + b) / 2.0
            };
            for sp in spans.iter_mut() {
                if sp[0] > 0.0 {
                    sp[0] = refine((sp[0] - probe_step).max(0.0), sp[0], true);
                }
                if sp[1] < e.length {
                    sp[1] = refine((sp[1] - probe_step).max(sp[0]), sp[1], false);
                }
            }
            result[k] = spans.into_iter().filter(|s| s[1] - s[0] >= 0.5).collect();
        }
        result
    };
    let spans2: Vec<[Vec<[f64; 2]>; 2]> = std::thread::scope(|sc| {
        let hs: Vec<_> = ids.chunks(chunk).map(|c| { let run2 = &run2; sc.spawn(move || c.iter().map(|&ei| run2(ei)).collect::<Vec<_>>()) }).collect();
        hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    for (e, s) in entries.iter_mut().zip(spans2) {
        e.walk_yield_spans = s;
    }
}

// ---------------------------------------------------------------------------
// corner paths (`finaliseJunction`)
// ---------------------------------------------------------------------------

const CORNER_CLOSE_MARGIN: f64 = 0.06;

/// `approachEndFrame`.
pub fn approach_end_frame(entries: &[Entry], app: &Approach) -> Frame {
    let e = &entries[app.entry];
    let s = if app.at_start { app.trim } else { e.length - app.trim };
    e.frame(clamp(s, 0.0, e.length), true)
}

/// `approachCurbStation`.
fn approach_curb_station(entries: &[Entry], app: &Approach, ccw: bool, o: &StreetOptions) -> CurbStation {
    let frame = approach_end_frame(entries, app);
    let side = (if app.at_start { 1.0 } else { -1.0 }) * (if ccw { 1.0 } else { -1.0 });
    let raw = if ccw { app.width_ccw } else { app.width_cw };
    let p = frame.at(side * app.half);
    CurbStation {
        x: p[0],
        z: p[1],
        out: [side * frame.nx, side * frame.nz],
        scale: frame.miter,
        walk: if raw >= o.min_sidewalk_width { raw } else { 0.0 },
        ang: None,
        mouth: Some((app.entry, frame.s, side * app.half)),
    }
}

fn corner_reach(position: P2, app: &Approach, ccw: bool, st: &CurbStation, cap: f64) -> f64 {
    let m = perp_ccw(app.u);
    let sign = if ccw { 1.0 } else { -1.0 };
    let lateral = ((st.x - position[0]) * m[0] + (st.z - position[1]) * m[1]) * sign;
    let rate = (st.out[0] * m[0] + st.out[1] * m[1]) * sign;
    if !(rate > 1e-6) {
        return 0.0;
    }
    let outer = app.half + if ccw { app.width_ccw } else { app.width_cw };
    let reach = (outer - lateral) / rate + CORNER_CLOSE_MARGIN;
    if !(reach > 0.0) {
        return 0.0;
    }
    reach.min(cap)
}

fn mitre_station(position: P2, a: &Approach, b: &Approach) -> Option<CurbStation> {
    let ma = perp_ccw(a.u);
    let mb = perp_ccw(b.u);
    let den = cross2(a.u, b.u);
    if den.abs() < 1e-4 {
        return None;
    }
    let a0 = [position[0] + ma[0] * a.half, position[1] + ma[1] * a.half];
    let b0 = [position[0] - mb[0] * b.half, position[1] - mb[1] * b.half];
    let t = cross2([b0[0] - a0[0], b0[1] - a0[1]], b.u) / den;
    let k = [a0[0] + a.u[0] * t, a0[1] + a.u[1] * t];
    if !k[0].is_finite() || !k[1].is_finite() {
        return None;
    }
    let da = (k[0] - position[0]) * a.u[0] + (k[1] - position[1]) * a.u[1];
    let db = (k[0] - position[0]) * b.u[0] + (k[1] - position[1]) * b.u[1];
    if !(da > -1e-6 && da <= a.trim + 1e-6 && db > -1e-6 && db <= b.trim + 1e-6) {
        return None;
    }
    let (ox, oz) = (ma[0] - mb[0], ma[1] - mb[1]);
    let ol = hypot(ox, oz);
    if ol < 1e-6 {
        return None;
    }
    Some(CurbStation { x: k[0], z: k[1], out: [ox / ol, oz / ol], scale: 1.0, walk: 0.0, ang: None, mouth: None })
}

fn chord_ray_enters_road(position: P2, app: &Approach, st: &CurbStation) -> f64 {
    let m = perp_ccw(app.u);
    let (px, pz) = (st.x - position[0], st.z - position[1]);
    let lat0 = px * m[0] + pz * m[1];
    let lat_r = st.out[0] * m[0] + st.out[1] * m[1];
    let al0 = px * app.u[0] + pz * app.u[1];
    let al_r = st.out[0] * app.u[0] + st.out[1] * app.u[1];
    let h = app.half - 0.05;
    if !(h > 0.0) {
        return f64::INFINITY;
    }
    let (mut lo, mut hi): (f64, f64) = (1e-3, f64::INFINITY);
    if lat_r.abs() < 1e-9 {
        if !(lat0.abs() < h) {
            return f64::INFINITY;
        }
    } else {
        let t1 = (-h - lat0) / lat_r;
        let t2 = (h - lat0) / lat_r;
        lo = lo.max(t1.min(t2));
        hi = hi.min(t1.max(t2));
    }
    if al_r.abs() < 1e-9 {
        if !(al0 > 0.0) {
            return f64::INFINITY;
        }
    } else if al_r > 0.0 {
        lo = lo.max(-al0 / al_r);
    } else {
        hi = hi.min(-al0 / al_r);
    }
    if lo < hi { lo } else { f64::INFINITY }
}

fn chord_path(position: P2, a: &Approach, b: &Approach, o: &StreetOptions, start: &CurbStation, end: &CurbStation) -> Vec<CurbStation> {
    let max_walk = start.walk.max(end.walk) + CORNER_CLOSE_MARGIN;
    if !(max_walk > 0.0) {
        return vec![];
    }
    let cap = 2.0 * a.half.max(b.half) + max_walk + 1.0;
    let mut knots = vec![*start];
    if let Some(m) = mitre_station(position, a, b) {
        knots.push(m);
    } else if norm_angle(b.angle - a.angle) > o.corner_max_angle_deg.to_radians() {
        let ma = perp_ccw(a.u);
        let mb = perp_ccw(b.u);
        let ka = CurbStation { x: position[0] + ma[0] * a.half, z: position[1] + ma[1] * a.half, out: ma, scale: 1.0, walk: 0.0, ang: None, mouth: None };
        let kb = CurbStation { x: position[0] - mb[0] * b.half, z: position[1] - mb[1] * b.half, out: [-mb[0], -mb[1]], scale: 1.0, walk: 0.0, ang: None, mouth: None };
        let inside_road = |p: &CurbStation, app: &Approach| {
            let m = perp_ccw(app.u);
            let (dx, dz) = (p.x - position[0], p.z - position[1]);
            dx * app.u[0] + dz * app.u[1] > 1e-6 && (dx * m[0] + dz * m[1]).abs() < app.half - 1e-6
        };
        if !inside_road(&ka, b) {
            knots.push(ka);
        }
        if !inside_road(&kb, a) {
            knots.push(kb);
        }
    }
    knots.push(*end);
    let mut stations = vec![];
    for i in 0..knots.len() - 1 {
        let (ka, kb) = (knots[i], knots[i + 1]);
        let steps = clamp((hypot(kb.x - ka.x, kb.z - ka.z) / 2.0).ceil(), 1.0, 16.0) as usize;
        for k in 1..=steps {
            if i == knots.len() - 2 && k == steps {
                break;
            }
            let t = k as f64 / steps as f64;
            let (mut ox, mut oz) = (ka.out[0] + (kb.out[0] - ka.out[0]) * t, ka.out[1] + (kb.out[1] - ka.out[1]) * t);
            let ol = hypot(ox, oz);
            if ol < 1e-6 {
                continue;
            }
            ox /= ol;
            oz /= ol;
            let mut st = CurbStation { x: ka.x + (kb.x - ka.x) * t, z: ka.z + (kb.z - ka.z) * t, out: [ox, oz], scale: 1.0, walk: max_walk, ang: None, mouth: None };
            st.walk = cap.min(max_walk.max(corner_reach(position, a, true, &st, cap)).max(corner_reach(position, b, false, &st, cap)));
            let blocked = chord_ray_enters_road(position, a, &st).min(chord_ray_enters_road(position, b, &st));
            if st.walk > max_walk && blocked < st.walk {
                st.walk = max_walk.max(blocked - CORNER_CLOSE_MARGIN);
            }
            stations.push(st);
        }
    }
    stations
}

/// `stampChordRamps` (stations gain angles and the ramps' shoulder stations).
fn stamp_chord_ramps(stations: &mut Vec<CurbStation>, a: &Approach, b: &Approach, o: &StreetOptions) -> Vec<RampWin> {
    let sweep = norm_angle(b.angle - a.angle);
    if !(sweep > o.corner_min_angle_deg.to_radians() && sweep < o.corner_max_angle_deg.to_radians()) || stations.len() < 2 {
        return vec![];
    }
    let mut run = 0.0;
    for i in 1..stations.len() {
        run += hypot(stations[i].x - stations[i - 1].x, stations[i].z - stations[i - 1].z);
    }
    if !(run > 0.4) {
        return vec![];
    }
    let mid = stations[stations.len() / 2];
    let centre = [mid.x + mid.out[0], mid.z + mid.out[1]];
    for st in stations.iter_mut() {
        if st.ang.is_none() {
            st.ang = Some((st.z - centre[1]).atan2(st.x - centre[0]));
        }
    }
    let a0 = stations[0].ang.unwrap();
    let a1 = stations.last().unwrap().ang.unwrap();
    let delta = signed_angle(a1 - a0);
    if delta.abs() < 1e-4 {
        return vec![];
    }
    let sign = if delta >= 0.0 { 1.0 } else { -1.0 };
    let radius = (run / delta.abs().max(1e-4)).max(0.8);
    let rh = (delta.abs() * 0.34).min((o.ramp_width / 2.0) / radius);
    if !(rh > 0.02) {
        return vec![];
    }
    let ramp_run = if o.ramp_run != 0.0 { o.ramp_run } else { 1.2 };
    let flare = (delta.abs() * 0.12).min((0.24f64).max(ramp_run * 0.25) / radius);
    let win = |c: f64| RampWin {
        lo: (c - rh * sign).min(c + rh * sign),
        hi: (c - rh * sign).max(c + rh * sign),
        flare_lo: 0.0,
        flare_hi: 0.0,
        node: usize::MAX,
        at_start: false,
    };
    let mut windows = vec![];
    let pair_span = 2.0 * (2.0 * rh + flare) + 0.5 * rh;
    if delta.abs() >= pair_span {
        windows.push(win(a0 + sign * (flare + rh)));
        windows.push(win(a0 + delta - sign * (flare + rh)));
    } else {
        windows.push(win(a0 + delta / 2.0));
    }
    let (arc_lo, arc_hi) = (a0.min(a0 + delta), a0.max(a0 + delta));
    let wanted = if radius > 1e-6 { o.ramp_flare_run / radius } else { 0.0 };
    windows.sort_by(|p, q| p.lo.partial_cmp(&q.lo).unwrap());
    let n = windows.len();
    for i in 0..n {
        let room_lo = windows[i].lo - if i == 0 { arc_lo } else { windows[i - 1].hi };
        let room_hi = (if i == n - 1 { arc_hi } else { windows[i + 1].lo }) - windows[i].hi;
        let w = &mut windows[i];
        w.flare_lo = w.lo - wanted.min(if i == 0 { room_lo } else { room_lo * 0.45 }).max(0.0);
        w.flare_hi = w.hi + wanted.min(if i == n - 1 { room_hi } else { room_hi * 0.45 }).max(0.0);
    }
    let seed = |stations: &mut Vec<CurbStation>, target: f64| {
        for i in 0..stations.len().saturating_sub(1) {
            let (sa, sb) = (stations[i], stations[i + 1]);
            let (Some(aa), Some(ab)) = (sa.ang, sb.ang) else { continue };
            if (ab - aa).abs() < 1e-6 {
                continue;
            }
            let t = (target - aa) / (ab - aa);
            if !(t > 1e-4 && t < 1.0 - 1e-4) {
                continue;
            }
            if hypot(sa.x - sb.x, sa.z - sb.z) < 1e-4 {
                continue;
            }
            let (ox, oz) = (sa.out[0] + (sb.out[0] - sa.out[0]) * t, sa.out[1] + (sb.out[1] - sa.out[1]) * t);
            let ol = hypot(ox, oz).max(1e-12);
            stations.insert(
                i + 1,
                CurbStation {
                    x: sa.x + (sb.x - sa.x) * t,
                    z: sa.z + (sb.z - sa.z) * t,
                    out: [ox / ol, oz / ol],
                    scale: sa.scale + (sb.scale - sa.scale) * t,
                    walk: sa.walk + (sb.walk - sa.walk) * t,
                    ang: Some(target),
                    mouth: None,
                },
            );
            return;
        }
    };
    for w in &windows {
        for target in [w.flare_lo, w.lo, (w.lo + w.hi) / 2.0, w.hi, w.flare_hi] {
            if target.is_finite() {
                seed(stations, target);
            }
        }
    }
    windows
}

fn leg_carries_crossing_ramp(entries: &[Entry], drawn: &[[Vec<RampWin>; 2]], ni: usize, app: &Approach, ccw: bool) -> bool {
    let side = (if app.at_start { 1 } else { -1 }) * (if ccw { 1 } else { -1 });
    let wins = &drawn[app.entry][if side > 0 { 0 } else { 1 }];
    let _ = entries;
    wins.iter().any(|w| w.node == ni && w.at_start == app.at_start)
}

/// `finaliseJunction`.
fn finalise_junction(ni: usize, node: &mut Node, entries: &[Entry], drawn: &[[Vec<RampWin>; 2]], o: &StreetOptions) {
    let count = node.approaches.len();
    node.corners.clear();
    node.paths.clear();
    for i in 0..count {
        let j = (i + 1) % count;
        let (a, b) = (&node.approaches[i], &node.approaches[j]);
        let corner = if i == j { None } else { solve_corner_arc(node.position, a, b, o) };
        node.corners.push(corner);
        let start = approach_curb_station(entries, a, true, o);
        let end = approach_curb_station(entries, b, false, o);
        let mut stations = vec![start];
        let mut ramps: Vec<RampWin> = vec![];
        let (mut ang_a, mut ang_b) = (None, None);
        if let Some(c) = corner {
            let (arc, wins) = arc_stations(&c, o);
            let arc_walk = start.walk.max(end.walk) + CORNER_CLOSE_MARGIN;
            let reach_cap = arc_walk.max(c.radius);
            for (k, st) in arc.iter().enumerate() {
                let mut walk = arc_walk;
                walk = walk.max(corner_reach(node.position, a, true, st, reach_cap));
                walk = walk.max(corner_reach(node.position, b, false, st, reach_cap));
                let mut s = *st;
                s.walk = walk.min(reach_cap);
                // A tangent that is exactly the trim mouth is that mouth's vertex.
                if k == 0 && (c.d_a - a.trim).abs() < 1e-9 {
                    s.mouth = start.mouth;
                    s.x = start.x;
                    s.z = start.z;
                }
                if k == arc.len() - 1 && (c.d_b - b.trim).abs() < 1e-9 {
                    s.mouth = end.mouth;
                    s.x = end.x;
                    s.z = end.z;
                }
                stations.push(s);
            }
            ramps = wins;
            ang_a = arc.first().and_then(|s| s.ang);
            ang_b = arc.last().and_then(|s| s.ang);
        } else {
            stations.extend(chord_path(node.position, a, b, o, &start, &end));
        }
        stations.push(end);
        let served_a = leg_carries_crossing_ramp(entries, drawn, ni, a, true);
        let served_b = leg_carries_crossing_ramp(entries, drawn, ni, b, false);
        if ramps.is_empty() && !served_a && !served_b {
            let chord_windows = stamp_chord_ramps(&mut stations, a, b, o);
            if !chord_windows.is_empty() {
                let reach = o.curb_top_width + o.ramp_run;
                let spans: Vec<[f64; 2]> = chord_windows.iter().map(|w| [w.flare_lo.min(w.lo), w.flare_hi.max(w.hi)]).collect();
                let cuttable = stations.iter().all(|st| match st.ang {
                    None => true,
                    Some(ang) => st.walk > reach + 0.05 || !spans.iter().any(|sp| ang > sp[0] - 1e-9 && ang < sp[1] + 1e-9),
                }) && chord_windows
                    .iter()
                    .all(|w| stations.iter().any(|st| st.ang.map(|ang| ang > w.lo - 1e-9 && ang < w.hi + 1e-9).unwrap_or(false)));
                if cuttable {
                    ramps = chord_windows;
                }
            }
        }
        if !ramps.is_empty() && (served_a || served_b) {
            if let (Some(aa), Some(ab)) = (ang_a, ang_b) {
                ramps.retain(|w| {
                    let mid = (w.lo + w.hi) / 2.0;
                    if (mid - aa).abs() <= (mid - ab).abs() { !served_a } else { !served_b }
                });
            }
        }
        node.paths.push(CornerPath { stations, ramps, a: i, b: j });
    }
}

// ---------------------------------------------------------------------------
// the whole plan
// ---------------------------------------------------------------------------

/// `buildStreetscapePlan`'s geometric passes over a street city.
pub fn build_plan(city: &StreetCity, o: &StreetOptions) -> Plan {
    let trace = std::env::var("EO_STREET_TRACE").is_ok();
    let t0 = std::time::Instant::now();
    let lap = |what: &str| if trace { eprintln!("  plan {what}: {:.0} ms", t0.elapsed().as_secs_f64() * 1e3) };
    let prepared = prepare(city, o);
    let mut entries = merge_short_runs(prepared, city);
    // (only with `StreetOptions::structures`.)
    add_structure_legs(&mut entries, city, o);
    let mut nodes = collect_nodes(city, &mut entries, o);
    // planJunction in stable-key order (the order does not change the trims:
    // each end takes the max of what its node asks).
    for n in nodes.iter_mut() {
        plan_junction(n, &mut entries, o);
    }
    lap("nodes+junctions");
    reconcile_trims(&mut entries, &mut nodes);
    let ev = (!city.crossings.is_empty() || !city.continental.is_empty() || !city.crossing_nodes.is_empty())
        .then(|| crate::evidence::Evidence::new(&city.crossings, &city.crossing_nodes, &city.continental, &nodes));
    for (ni, n) in nodes.iter_mut().enumerate() {
        plan_node_paint(ni, n, &mut entries, o, ev.as_ref(), city);
    }
    lap("paint");
    plan_carriageway_yield(&mut entries, &nodes, o, city);
    if o.pavement_repairs { crate::driveway::plan_curb_cuts(&mut entries, city, o); }
    lap("yield");
    let drawn: Vec<[Vec<RampWin>; 2]> = entries.iter().map(|e| segment_kerb_ramps(e, o)).collect();
    for (ni, n) in nodes.iter_mut().enumerate() {
        finalise_junction(ni, n, &entries, &drawn, o);
    }
    lap("finalise");
    Plan { entries, nodes }
}
