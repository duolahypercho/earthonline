//! The network built natively from the extract (`sf-city.json.gz` via
//! `eo_data::sim_source`), one delivery of 256 m tiles at a time - the path
//! that scales past the JS window to any tile of the city (and, with a tile
//! store in place of the one source file, of the Earth).
//!
//! WHAT IT BUILDS, AND FROM WHICH RULES
//!
//!   lanes      Every vehicle way split at shared surveyed vertices and at
//!              drawn junctions that the simplified survey omits. Use the
//!              paved through lanes where a section fits, or the
//!              `lanes` tag (one each way, or one, when untagged); right-hand
//!              traffic; fallback lane width the class model the
//!              renderer slice already uses (`classLaneWidthOf`: the 10 ft SF
//!              lane, the HDM State-route lanes) through `caMetric`; driven
//!              polylines offset through the node as `junctionLanePoints` draws
//!              them. Successors on shared centreline nodes, plans by the ported
//!              `buildJunctionPlan`, roles all `through` (no painted arrows in the
//!              extract), so every movement is legal and weighted by class.
//!   signals    Each surveyed signal controls the nodes within
//!              `JUNCTION_NODE_MAX_M` where two or more streets meet, grouped
//!              into one controller per physical crossing (`SPLIT_JUNCTION_M`);
//!              approaches are the lanes driven toward those nodes, clustered
//!              into axes by the ported `clusterAxes`. The stop bar is the
//!              conflicting carriageway's half-width plus a crosswalk and the
//!              limit line's setback from it (`crosswalk-min-width`,
//!              `limit-line-setback-from-crosswalk`); the timing is then the
//!              ported CA MUTCD plan.
//!   footways   A walking line each side of every street with a sidewalk, at the
//!              middle of the throughway zone: carriageway half + the SF sidewalk
//!              minimum for the street type - frontage zone - half the
//!              throughway (`sidewalk-width-sf-*`, `sidewalk-frontage-zone-sf`,
//!              `sidewalk-throughway-zone-sf`). Corners stop at the crossing
//!              street's kerb; the crossing band sits a half crosswalk beyond it.
//!
//! WHAT IT DOES NOT HAVE that the JS window has: the surveyed kerb-to-kerb
//! widths and lane layouts of `sf-data.js` (bike lanes, parking lanes, turn
//! bays, centre turn lanes) and lane-use arrows throughout the city. When
//! supplied, the street plan provides paved cross sections and junction pads.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use eo_data::sim_source::{Oneway, SimRoad, SimSource, TrafficDeckLane, TrafficDeckSurface, TrafficPlan, TrafficSection, TrafficStop};

use crate::consts::{PERSON_CAPSULE_R_M, WALKER_LATERAL_LIMIT_M};
use crate::geom::{self, LaneRole, P2};
use crate::network::*;
use crate::signals::{AxisInput, JunctionInput, MemberInput, cluster_axes};
use crate::standards::{SignalTiming, StreetDims};

mod junctions;
mod stops;
mod surveyed_routes;
mod walk_gaps;

/// `SIGNAL_TIMING.JUNCTION_NODE_MAX_M`, `SPLIT_JUNCTION_M`, `APPROACH_MIN_CLOSURE_M`.
const JUNCTION_NODE_MAX_M: f64 = 25.0;
const SPLIT_JUNCTION_M: f64 = 20.0;
const APPROACH_MIN_CLOSURE_M: f64 = 5.5;

pub struct NativeSource {
    roads: Vec<SimRoad>,
    collapsed_roads: HashMap<String, SimRoad>,
    surveyed_routes: Option<surveyed_routes::Routes>,
    signals: Vec<P2>,
    road_tiles: HashMap<TileKey, Vec<u32>>,
    signal_tiles: HashMap<TileKey, Vec<u32>>,
    pub dims: StreetDims,
    pub timing: SignalTiming,
    model_footprints: Option<std::sync::Arc<crate::vehicle_catalogue::ScaleReport>>,
    /// Structure level by OSM way id (`SimRoad::id`), from whoever builds the
    /// grade-separated roads (eo-app's `--structures`: the structures'
    /// `StructureWorld::way_levels`). Empty (every lane at grade, the
    /// network exactly as without it) unless set (`with_levels`).
    levels: HashMap<String, i8>,
    /// The building fronts the footways are measured against (`--crowd`;
    /// `with_frontage`). None: every footway's activity zero, as before.
    frontage: Option<std::sync::Arc<FrontageIndex>>,
    /// The walks that are not a street's sidewalk (`--crowd`; `with_paths`):
    /// a plaza's, a promenade's and a park's paths, by tile. Empty: none, as before.
    paths: Vec<PathWay>,
    path_tiles: HashMap<TileKey, Vec<u32>>,
    /// The drawn street's cross-sections (`--crowd`; `with_sections`): the
    /// footways stand on the drawn walks and stop at the drawn kerbs. None:
    /// the class model's, as before.
    sections: Option<std::sync::Arc<SectionIndex>>,
    /// Paved cross-sections keyed by the source OSM way, for traffic and walks.
    traffic_sections: HashMap<String, Vec<TrafficSection>>,
    walk_gaps: HashMap<String, Vec<walk_gaps::WalkGap>>,
    walk_lane_clearance: Option<walk_gaps::WalkLaneClearance>,
    junctions: junctions::DrawnJunctions,
    stops: stops::StopIndex,
    /// Neighboring one-way carriageways that may cut into a median walk.
    divided_pavement: HashMap<String, DividedPavement>,
    deck_surfaces: HashMap<String, Vec<Vec<[f64; 3]>>>,
    deck_lanes: HashMap<String, Vec<TrafficDeckLane>>,
    passing: bool,
    bend_braking: bool,
    /// The mapped crosswalks (`--crowd`; `with_crossings`): an unsignalled
    /// street is crossed only where one is mapped. None: wherever the JS
    /// crosses (every junction), as before.
    crossings: Option<std::sync::Arc<CrossingIndex>>,
    /// The zebras the street draws (`--people-crosswalks`; `with_zebras`):
    /// each street end is crossed on its drawn zebra, and not where none is
    /// drawn. None: the class band and the rules above, as before.
    zebras: Option<std::sync::Arc<CrossingIndex>>,
}

/// One straight piece of a drawn carriageway's centreline (`--crowd`;
/// `NativeSource::with_sections`, eo-app from eo-street's plan): its ends,
/// its half width and the drawn walks either side, `walks[0]` on the left of
/// a -> b ((-dz, dx), eo-street's `frame_at` normal), world metres on the
/// plan the roads are on.
#[derive(Clone, Copy, Debug)]
pub struct DrawnSection {
    pub a: P2,
    pub b: P2,
    pub half: f64,
    pub walks: [f64; 2],
}

/// The drawn pieces on a grid of `FRONT_CELL_M` cells.
struct SectionIndex {
    pieces: Vec<DrawnSection>,
    cells: HashMap<(i32, i32), Vec<u32>>,
}

struct DividedPavement {
    strips: SectionIndex,
    /// Source way of each indexed strip.
    ways: Vec<String>,
}

/// How far a drawn centreline may stand from the simulation's (the same OSM
/// line, merged and re-split by the street planner), and how parallel it
/// must run (cos of 20 degrees), to stand for it.
const SECTION_MATCH_M: f64 = 3.0;
/// Registration tolerance at the built structure's mouth, whose asphalt
/// fan uses f32 vertices and a separate crown from the deck profile.
const STRUCTURE_MOUTH_MATCH_M: f64 = 0.05;
const SECTION_MATCH_COS: f64 = 0.94;
/// Maximum change in lateral registration across the samples for a match.
const SECTION_REGISTER_VARIATION_M: f64 = 0.15;
/// A walk's sample spacing against the drawn carriageways (m).
const PATH_SAMPLE_M: f64 = 1.0;
const WALK_CLEARANCE_M: f64 = WALKER_LATERAL_LIMIT_M + PERSON_CAPSULE_R_M;

struct PavedMatch<'a> {
    section: &'a TrafficSection,
    /// Displacement from the mapped centreline along its (dz, -dx) normal.
    lateral: f64,
    reversed: bool,
    drawn_chord: Option<[P2; 2]>,
}

#[derive(Default)]
struct PavedRegistration<'a> {
    profile: Option<PavedMatch<'a>>,
    uses_shared_lane: bool,
}

impl PavedMatch<'_> {
    fn walks(&self) -> [f64; 2] {
        let [left, right] = self.section.walks;
        if self.reversed { [left, right] } else { [right, left] }
    }

    fn direction(&self, decreasing: bool) -> i8 {
        let orientation = if self.reversed { -1 } else { 1 };
        if decreasing { -orientation } else { orientation }
    }

    fn lane_counts(&self, mapped_forward: u32, mapped_backward: u32) -> Option<(u32, u32)> {
        let forward = self.section.lanes.iter()
            .filter(|lane| lane.direction == self.direction(false))
            .count();
        let backward = self.section.lanes.len() - forward;
        // Use the through lanes that actually fit on the paved cross-section.
        // The mapped direction still owns which way traffic can legally travel.
        ((forward > 0) == (mapped_forward > 0) && (backward > 0) == (mapped_backward > 0))
            .then_some((forward as u32, backward as u32))
    }

    fn lane(&self, decreasing: bool, index: u32) -> Option<(f64, f64)> {
        let direction = self.direction(decreasing);
        let mut lanes = self.section.lanes.iter().filter(|lane| lane.direction == direction);
        // Keep the native lane key ordering, using the authored point order.
        let lane = if direction > 0 {
            lanes.rev().nth(index as usize)?
        } else {
            lanes.nth(index as usize)?
        };
        Some((self.lateral + lane.lateral * f64::from(self.direction(false)), lane.width))
    }

    fn passing_span(&self, pts: &[P2], decreasing: bool) -> Option<[f64; 2]> {
        let [lo, hi] = self.section.paint_span?;
        let painted = &self.section.points;
        let painted_cum = geom::cumulative(painted);
        let painted_len = *painted_cum.last()?;
        let [a, b] = if decreasing { [*pts.last()?, pts[0]] } else { [pts[0], *pts.last()?] };
        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
        let len = dx.hypot(dz);
        // A lane change on this chord stays in the two painted lane ribbons.
        if self.section.crosses_road || pts.len() != 2 || painted.len() != 2
            || !(0.0 <= lo && lo < hi && hi <= painted_len && len > 0.0)
        {
            return None;
        }
        let mut span = [lo, hi].map(|s| {
            let (x, z, _, _, _) = geom::at_arc(painted, &painted_cum, s);
            (((x - a[0]) * dx + (z - a[1]) * dz) / len).clamp(0.0, len)
        });
        span.sort_by(f64::total_cmp);
        (span[0] < span[1]).then_some(span)
    }

    fn broken_divider(&self, decreasing: bool, a: f64, b: f64) -> bool {
        let middle = ((a + b) / 2.0 - self.lateral) * f64::from(self.direction(false));
        self.section.paint.iter().any(|mark| mark.divider == self.direction(decreasing)
            && mark.broken && !mark.yellow && (mark.lateral - middle).abs() <= mark.width / 2.0)
    }
}

impl SectionIndex {
    fn new(pieces: Vec<DrawnSection>) -> Self {
        Self::with_margin(pieces, 0.0)
    }

    fn with_margin(pieces: Vec<DrawnSection>, margin: f64) -> Self {
        let mut cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, p) in pieces.iter().enumerate() {
            let r = SECTION_MATCH_M.max(p.half + margin);
            let lo = front_cell(p.a[0].min(p.b[0]) - r, p.a[1].min(p.b[1]) - r);
            let hi = front_cell(p.a[0].max(p.b[0]) + r, p.a[1].max(p.b[1]) + r);
            for cx in lo.0..=hi.0 {
                for cz in lo.1..=hi.1 {
                    cells.entry((cx, cz)).or_default().push(i as u32);
                }
            }
        }
        SectionIndex { pieces, cells }
    }

    /// The drawn cross-section of a simulated street piece: its half width and
    /// walks as the simulation sides them (`[side +1, side -1]`: the +offset
    /// side is (dz, -dx) of its direction), read at the quarter points of its
    /// polyline (the median half of the matches), or None where nothing drawn
    /// runs along it.
    fn of(&self, pts: &[P2]) -> Option<(f64, [f64; 2])> {
        let cum = geom::cumulative(pts);
        let len = *cum.last()?;
        if len <= 0.0 {
            return None;
        }
        let mut found: Vec<(f64, [f64; 2])> = vec![];
        for f in [0.25, 0.5, 0.75] {
            let (x, z, dx, dz, _) = geom::at_arc(pts, &cum, f * len);
            let Some(list) = self.cells.get(&front_cell(x, z)) else { continue };
            let mut best: Option<(f64, f64, [f64; 2])> = None;
            for &k in list {
                let p = &self.pieces[k as usize];
                let (ux, uz) = (p.b[0] - p.a[0], p.b[1] - p.a[1]);
                let ul = geom::hypot(ux, uz);
                if ul < 1e-6 {
                    continue;
                }
                let c = (ux * dx + uz * dz) / ul;
                if c.abs() < SECTION_MATCH_COS {
                    continue;
                }
                let (d, _) = seg_nearest(p.a, p.b, x, z);
                if d > SECTION_MATCH_M || best.is_some_and(|(bd, _, _)| bd <= d) {
                    continue;
                }
                // The drawn left (-dz, dx) is the simulation's -1 side when the
                // two run the same way, its +1 side when they run opposite.
                let walks = if c > 0.0 { [p.walks[1], p.walks[0]] } else { [p.walks[0], p.walks[1]] };
                best = Some((d, p.half, walks));
            }
            if let Some((_, h, w)) = best {
                found.push((h, w));
            }
        }
        if found.is_empty() {
            return None;
        }
        found.sort_by(|a, b| a.0.total_cmp(&b.0));
        Some(found[found.len() / 2])
    }

    /// Whether (x, z) lies on a drawn carriageway (within a piece's half width).
    fn on_carriageway(&self, x: f64, z: f64) -> bool {
        self.cells.get(&front_cell(x, z)).is_some_and(|list| {
            list.iter().any(|&k| self.pieces[k as usize].covers([x, z], 0.0))
        })
    }

    /// A polyline's runs off the drawn carriageways, in order (sampled every
    /// `PATH_SAMPLE_M`: each run keeps the polyline's vertices off them and
    /// ends at its last sample before one).
    fn off_carriageway(&self, pts: &[P2]) -> Vec<Vec<P2>> {
        walk_runs(pts, |p, _, _, _| self.on_carriageway(p[0], p[1]))
            .into_iter().map(|run| run.points).collect()
    }
}

impl DrawnSection {
    fn covers(&self, [x, z]: P2, margin: f64) -> bool {
        let (dx, dz) = (self.b[0] - self.a[0], self.b[1] - self.a[1]);
        let l2 = dx * dx + dz * dz;
        if l2 < 1e-12 {
            return false;
        }
        let t = ((x - self.a[0]) * dx + (z - self.a[1]) * dz) / l2;
        (0.0..=1.0).contains(&t)
            && geom::hypot(self.a[0] + dx * t - x, self.a[1] + dz * t - z) < self.half + margin
    }
}

struct WalkRun {
    points: Vec<P2>,
    start: f64,
    end: f64,
}

fn walk_runs(pts: &[P2], blocked: impl Fn(P2, P2, usize, f64) -> bool) -> Vec<WalkRun> {
    let mut runs = Vec::new();
    let mut cur = Vec::<P2>::new();
    let mut start = 0.0;
    let mut end = 0.0;
    let mut arc = 0.0;
    for (piece, w) in pts.windows(2).enumerate() {
        let (a, b) = (w[0], w[1]);
        let heading = [b[0] - a[0], b[1] - a[1]];
        let l = geom::hypot(heading[0], heading[1]);
        let n = (l / PATH_SAMPLE_M).ceil().max(1.0) as usize;
        let at = |k: usize| {
            let t = k as f64 / n as f64;
            ([a[0] + heading[0] * t, a[1] + heading[1] * t], t)
        };
        let blocked = |(p, t)| blocked(p, heading, piece, t);
        for k in 0..=n {
            let (q, t) = at(k);
            if blocked((q, t)) {
                if cur.len() >= 2 {
                    runs.push(WalkRun { points: std::mem::take(&mut cur), start, end });
                } else {
                    cur.clear();
                }
                continue;
            }
            let fresh = cur.last().is_none_or(|p| geom::hypot(p[0] - q[0], p[1] - q[1]) > 1e-6);
            let edge = k == 0 || k == n || cur.is_empty() || blocked(at(k + 1));
            if fresh && edge {
                end = arc + l * (k as f64 / n as f64);
                if cur.is_empty() {
                    start = end;
                }
                cur.push(q);
            }
        }
        arc += l;
    }
    if cur.len() >= 2 {
        runs.push(WalkRun { points: cur, start, end });
    }
    runs
}

impl DividedPavement {
    fn covers_sidewalk(&self, road: &SimRoad, levels: &HashMap<String, i8>, p: P2, [dx, dz]: P2) -> bool {
        let direction = if road.oneway == Oneway::Reverse { -1.0 } else { 1.0 };
        self.strips.cells.get(&front_cell(p[0], p[1])).is_some_and(|indices| {
            indices.iter().any(|&i| {
                let other = &self.ways[i as usize];
                let strip = &self.strips.pieces[i as usize];
                let dot = dx * (strip.b[0] - strip.a[0]) + dz * (strip.b[1] - strip.a[1]);
                other != &road.id && dot * direction < 0.0
                    && levels.get(other).copied().unwrap_or_default() == 0
                    && strip.covers(p, WALK_CLEARANCE_M)
            })
        })
    }
}

/// Keep the original junction connection wherever the sidewalk still reaches
/// its kerb or the eligible crossing band beside it.
fn sidewalk_parts(footway: FootwayBuild, runs: Vec<WalkRun>) -> Vec<(FootwayBuild, [bool; 2])> {
    let length = geom::cumulative(&footway.points).last().copied().unwrap_or_default();
    let trim = footway.ends[0].kerb_trim + footway.ends[1].kerb_trim;
    if runs.len() == 1 && runs[0].start == 0.0 && runs[0].end == length {
        return if length > trim { vec![(footway, [true; 2])] } else { Vec::new() };
    }
    runs.into_iter().enumerate().filter_map(|(i, run)| {
        let mut part = FootwayBuild { points: run.points, original_arc_start: Some(run.start), ..footway.clone() };
        let removed = [run.start, length - run.end];
        let retain = std::array::from_fn(|e| {
            let end = &part.ends[e];
            let band = if e == 0 { end.crossing_arc } else { length - end.crossing_arc };
            removed[e] <= end.kerb_trim || (!end.no_crossing && (run.start..=run.end).contains(&band))
        });
        for e in 0..2 {
            let end = &mut part.ends[e];
            if retain[e] {
                end.kerb_trim = (end.kerb_trim - removed[e]).max(0.0);
                end.crossing_arc = (end.crossing_arc - removed[e]).max(0.0);
            } else {
                // An interior cut is a dead end. Its key cannot join a
                // neighboring sidewalk or a coincident crossing node.
                *end = FootEndBuild { kerb_trim: 0.0, crossing_arc: 0.0, no_crossing: true, zebra_only: false };
                part.node_keys.as_mut().unwrap()[e] = format!("{}:walk:{}:{i}:{e}", part.segment, part.road_side);
            }
        }
        let trims = part.ends[0].kerb_trim + part.ends[1].kerb_trim;
        (run.end - run.start > trims).then_some((part, retain))
    }).collect()
}

/// The mapped crosswalks' middles on a grid of `FRONT_CELL_M` cells.
struct CrossingIndex {
    cells: HashMap<(i32, i32), Vec<P2>>,
}

impl CrossingIndex {
    fn new(points: Vec<P2>) -> Self {
        let mut cells: HashMap<(i32, i32), Vec<P2>> = HashMap::new();
        for p in points {
            cells.entry(front_cell(p[0], p[1])).or_default().push(p);
        }
        CrossingIndex { cells }
    }

    /// The nearest middle within `r` of (x, z) that `keep` admits.
    fn nearest_by(&self, x: f64, z: f64, r: f64, keep: impl Fn(P2) -> bool) -> Option<P2> {
        let lo = front_cell(x - r, z - r);
        let hi = front_cell(x + r, z + r);
        let mut best: Option<(f64, P2)> = None;
        for cx in lo.0..=hi.0 {
            for cz in lo.1..=hi.1 {
                for p in self.cells.get(&(cx, cz)).into_iter().flatten() {
                    let d = geom::hypot(p[0] - x, p[1] - z);
                    if d <= r && keep(*p) && best.is_none_or(|(b, q)| d < b || (d == b && (p[0], p[1]) < (q[0], q[1]))) {
                        best = Some((d, *p));
                    }
                }
            }
        }
        best.map(|(_, p)| p)
    }

    /// Whether a mapped crosswalk's middle lies within `r` of (x, z).
    fn near(&self, x: f64, z: f64, r: f64) -> bool {
        let lo = front_cell(x - r, z - r);
        let hi = front_cell(x + r, z + r);
        (lo.0..=hi.0).any(|cx| (lo.1..=hi.1).any(|cz| self.cells.get(&(cx, cz)).is_some_and(|v| v.iter().any(|p| geom::hypot(p[0] - x, p[1] - z) <= r))))
    }
}

/// A plaza too small to cross (m2), the outline's station spacing, a walk's
/// least length (m), one walk to this much of the plaza's mean width (m), at
/// most this many walks.
const PLAZA_MIN_AREA_M2: f64 = 400.0;
const PLAZA_STATION_M: f64 = 6.0;
const PLAZA_WALK_MIN_M: f64 = 12.0;
const PLAZA_WALK_SPACING_M: f64 = 18.0;
const PLAZA_WALKS_MAX: usize = 48;

/// A paved plaza's walks (`--crowd`; eo-app from the landmarks' plaza plan):
/// people cross an open plaza from edge to edge rather than along one line,
/// so a plaza holds straight walks between points of its outline (`ring`: its
/// outer ring, resampled every `PLAZA_STATION_M`), each wholly inside it,
/// about one to every `PLAZA_WALK_SPACING_M` of the plaza's mean width, no
/// two from neighbouring stations, and its outline cut at their ends: a
/// network the walkers route through at every end. Deterministic in `id` and
/// the ring; the walks avoid nothing inside the plaza.
pub fn plaza_walks(id: &str, ring: &[P2]) -> Vec<PathWay> {
    let mut r: Vec<P2> = ring.to_vec();
    if r.len() >= 2 && geom::hypot(r[0][0] - r[r.len() - 1][0], r[0][1] - r[r.len() - 1][1]) < 0.01 {
        r.pop();
    }
    if r.len() < 3 {
        return vec![];
    }
    let area = (0..r.len()).map(|i| {
        let (p, q) = (r[i], r[(i + 1) % r.len()]);
        p[0] * q[1] - q[0] * p[1]
    }).sum::<f64>().abs() / 2.0;
    if area < PLAZA_MIN_AREA_M2 {
        return vec![];
    }
    let mut st: Vec<P2> = vec![];
    for i in 0..r.len() {
        let (p, q) = (r[i], r[(i + 1) % r.len()]);
        let l = geom::hypot(q[0] - p[0], q[1] - p[1]);
        let n = (l / PLAZA_STATION_M).ceil().max(1.0) as usize;
        for k in 0..n {
            let t = k as f64 / n as f64;
            st.push([p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t]);
        }
    }
    let inside = |x: f64, z: f64| {
        let mut c = false;
        let mut j = r.len() - 1;
        for i in 0..r.len() {
            let (a, b) = (r[i], r[j]);
            if (a[1] > z) != (b[1] > z) && x < (b[0] - a[0]) * (z - a[1]) / (b[1] - a[1]) + a[0] {
                c = !c;
            }
            j = i;
        }
        c
    };
    // Station pairs whose chord stays inside (sampled every station spacing).
    let mut cands: Vec<(f64, usize, usize, f64)> = vec![];
    for i in 0..st.len() {
        for j in i + 2..st.len() {
            if i == 0 && j == st.len() - 1 {
                continue;
            }
            let (a, b) = (st[i], st[j]);
            let l = geom::hypot(b[0] - a[0], b[1] - a[1]);
            if l < PLAZA_WALK_MIN_M {
                continue;
            }
            let n = (l / PLAZA_STATION_M).ceil() as usize;
            if (1..n).all(|k| {
                let t = k as f64 / n as f64;
                inside(a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t)
            }) {
                cands.push((crate::rng::keyed_random(&format!("plaza-{id}"), &format!("{i}-{j}")), i, j, l));
            }
        }
    }
    if cands.is_empty() {
        return vec![];
    }
    cands.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mean = cands.iter().map(|c| c.3).sum::<f64>() / cands.len() as f64;
    let want = ((area / (PLAZA_WALK_SPACING_M * mean)).round() as usize).clamp(1, PLAZA_WALKS_MAX);
    let mut used: Vec<usize> = vec![];
    let mut chosen: Vec<(usize, usize)> = vec![];
    for &(_, i, j, _) in &cands {
        if chosen.len() >= want {
            break;
        }
        if used.iter().any(|&u| u.abs_diff(i) <= 1 || u.abs_diff(j) <= 1) {
            continue;
        }
        used.push(i);
        used.push(j);
        chosen.push((i, j));
    }
    let mut out: Vec<PathWay> = chosen.iter().enumerate().map(|(k, &(i, j))| PathWay { id: format!("{id}w{k}"), highway: "pedestrian".into(), points: vec![st[i], st[j]] }).collect();
    // The outline, cut at the walks' ends.
    used.sort_unstable();
    used.dedup();
    for k in 0..used.len() {
        let (s0, e) = (used[k], used[(k + 1) % used.len()]);
        let mut pts = vec![st[s0]];
        let mut i = s0;
        loop {
            i = (i + 1) % st.len();
            pts.push(st[i]);
            if i == e || pts.len() > st.len() {
                break;
            }
        }
        out.push(PathWay { id: format!("{id}e{k}"), highway: "pedestrian".into(), points: pts });
    }
    out
}

/// A walk of its own (`NativeSource::with_paths`, eo-app `--crowd`): an OSM
/// `pedestrian`, `footway` or `path` way that is not a street's sidewalk or
/// crossing, at grade, on the plan the roads are on.
#[derive(Clone, Debug)]
pub struct PathWay {
    pub id: String,
    pub highway: String,
    pub points: Vec<P2>,
}

/// One building edge on the street (`NativeSource::with_frontage`, eo-app
/// `--crowd`): its ends (world metres, on the plan the roads are on), the
/// building's storeys and its use at street level.
#[derive(Clone, Copy, Debug)]
pub struct FrontEdge {
    pub a: P2,
    pub b: P2,
    pub storeys: f32,
    /// A walk-in shop, cafe or service.
    pub shops: bool,
    /// A bar, pub, restaurant, club or theatre.
    pub night: bool,
}

/// The fronts on a grid of `FRONT_CELL_M` cells.
struct FrontageIndex {
    edges: Vec<FrontEdge>,
    cells: HashMap<(i32, i32), Vec<u32>>,
}

/// The frontage survey's grid cell, its sample spacing along a footway, its
/// slack past the building line (OSM footprints against the class model's
/// walk lines offset from the centreline) and how far on the carriageway's
/// side of the walk line a front may stand (a footprint over the sidewalk),
/// metres.
const FRONT_CELL_M: f64 = 32.0;
const FRONT_SAMPLE_M: f64 = 4.0;
const FRONT_SLACK_M: f64 = 4.0;
const FRONT_INWARD_M: f64 = 1.0;

fn front_cell(x: f64, z: f64) -> (i32, i32) {
    ((x / FRONT_CELL_M).floor() as i32, (z / FRONT_CELL_M).floor() as i32)
}

/// The distance from (x, z) to a segment and the segment's nearest point.
fn seg_nearest(a: P2, b: P2, x: f64, z: f64) -> (f64, P2) {
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let len2 = (dx * dx + dz * dz).max(1e-9);
    let t = (((x - a[0]) * dx + (z - a[1]) * dz) / len2).clamp(0.0, 1.0);
    let q = [a[0] + dx * t, a[1] + dz * t];
    (geom::hypot(q[0] - x, q[1] - z), q)
}

impl FrontageIndex {
    fn new(edges: Vec<FrontEdge>) -> Self {
        let mut cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, e) in edges.iter().enumerate() {
            let lo = front_cell(e.a[0].min(e.b[0]), e.a[1].min(e.b[1]));
            let hi = front_cell(e.a[0].max(e.b[0]), e.a[1].max(e.b[1]));
            for cx in lo.0..=hi.0 {
                for cz in lo.1..=hi.1 {
                    cells.entry((cx, cz)).or_default().push(i as u32);
                }
            }
        }
        FrontageIndex { edges, cells }
    }

    /// The shares of a walking line fronted by shops and by the evening
    /// economy, and the mean storeys of its fronts: at every sample along it,
    /// the nearest building edge within `reach` on its outer side (`outward`:
    /// away from the carriageway, so never a front across the street; none:
    /// nothing fronts it there).
    fn activity(&self, points: &[P2], outward: P2, reach: f64) -> crate::consts::FootActivity {
        let cum = geom::cumulative(points);
        let len = *cum.last().unwrap_or(&0.0);
        if len <= 0.0 {
            return Default::default();
        }
        let n = (len / FRONT_SAMPLE_M).ceil().max(1.0) as usize;
        let (mut shops, mut night, mut storeys) = (0u32, 0u32, 0.0f64);
        // each sample's front a walk-in shop (`FootActivity::shop_fronts`)
        let mut shop_at = vec![false; n];
        for i in 0..n {
            let (x, z, _, _, _) = geom::at_arc(points, &cum, (i as f64 + 0.5) * len / n as f64);
            let lo = front_cell(x - reach, z - reach);
            let hi = front_cell(x + reach, z + reach);
            let mut best: Option<(f64, u32)> = None;
            for cx in lo.0..=hi.0 {
                for cz in lo.1..=hi.1 {
                    for &k in self.cells.get(&(cx, cz)).map(|v| v.as_slice()).unwrap_or(&[]) {
                        let e = &self.edges[k as usize];
                        let (d, q) = seg_nearest(e.a, e.b, x, z);
                        let out = (q[0] - x) * outward[0] + (q[1] - z) * outward[1];
                        if d <= reach && out >= -FRONT_INWARD_M && best.is_none_or(|(bd, bk)| d < bd || (d == bd && k < bk)) {
                            best = Some((d, k));
                        }
                    }
                }
            }
            if let Some((_, k)) = best {
                let e = &self.edges[k as usize];
                shops += e.shops as u32;
                night += e.night as u32;
                storeys += e.storeys as f64;
                shop_at[i] = e.shops;
            }
        }
        // each of the walk's stretches takes the sample nearest its middle
        let slots = crate::consts::SHOP_FRONT_SLOTS;
        let shop_fronts = (0..slots).filter(|&j| shop_at[(((j as f64 + 0.5) / slots as f64) * n as f64) as usize % n]).fold(0u64, |m, j| m | 1u64 << j);
        let n = n as f64;
        crate::consts::FootActivity { shops: (shops as f64 / n) as f32, night: (night as f64 / n) as f32, storeys: (storeys / n) as f32, place: 0.0, shop_fronts }
    }
}

fn vehicle_way(h: &str) -> bool {
    eo_data::standards::is_vehicle_way(h)
}

fn tiles_of_polyline(points: &[P2]) -> Vec<TileKey> {
    let mut out = vec![];
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let steps = (geom::hypot(b[0] - a[0], b[1] - a[1]) / (TILE_M / 4.0)).ceil().max(1.0) as usize;
        for s in 0..=steps {
            let t = s as f64 / steps as f64;
            out.push(TileKey::of(a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t));
        }
    }
    if points.len() == 1 {
        out.push(TileKey::of(points[0][0], points[0][1]));
    }
    out.sort();
    out.dedup();
    out
}

fn qkey(p: P2) -> (i64, i64) {
    ((p[0] * 10.0).round() as i64, (p[1] * 10.0).round() as i64)
}

fn node_text(p: P2) -> String {
    format!("{:.1},{:.1}", p[0], p[1])
}

struct Seg {
    road: u32,
    key: String,
    pts: Vec<P2>,
    nodes: [P2; 2],
    half: f64,
    lane_w: f64,
    forward: u32,
    backward: u32,
}

struct LaneMeta {
    seg: usize,
    end: P2,
    reverse: bool,
}

fn is_divided_candidate(road: &SimRoad) -> bool {
    !road.name.is_empty() && road.oneway != Oneway::Both && !road.bridge && !road.tunnel && road.layer == 0
}

/// The street planner may orient a merged strip against its source way.
fn along_traffic([a, b]: [P2; 2], road: &SimRoad) -> [P2; 2] {
    let mid = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
    let source = road.points.windows(2).min_by(|p, q| {
        seg_nearest(p[0], p[1], mid[0], mid[1]).0.total_cmp(&seg_nearest(q[0], q[1], mid[0], mid[1]).0)
    }).unwrap();
    let dot = (b[0] - a[0]) * (source[1][0] - source[0][0]) + (b[1] - a[1]) * (source[1][1] - source[0][1]);
    if (dot < 0.0) == (road.oneway == Oneway::Reverse) { [a, b] } else { [b, a] }
}

impl NativeSource {
    pub fn new(source: SimSource, dims: StreetDims, timing: SignalTiming) -> Self {
        let mut roads = Vec::new();
        let mut collapsed_roads = HashMap::new();
        for road in source.roads {
            if !vehicle_way(&road.highway) || road.points.len() < 2 {
                continue;
            }
            // Simplification can collapse a one-way circuit into a chord
            // traversed twice. Its line cannot determine either carriageway
            // or kerb, and joining it would create phantom movements.
            let collapsed_loop = road.oneway != Oneway::Both
                && matches!(road.points.as_slice(), [start, _, end] if start == end);
            if collapsed_loop {
                collapsed_roads.insert(road.id.clone(), road);
            } else {
                roads.push(road);
            }
        }
        let mut road_tiles: HashMap<TileKey, Vec<u32>> = HashMap::new();
        for (i, r) in roads.iter().enumerate() {
            for t in tiles_of_polyline(&r.points) {
                road_tiles.entry(t).or_default().push(i as u32);
            }
        }
        let mut signal_tiles: HashMap<TileKey, Vec<u32>> = HashMap::new();
        for (i, s) in source.signals.iter().enumerate() {
            signal_tiles.entry(TileKey::of(s[0], s[1])).or_default().push(i as u32);
        }
        NativeSource { roads, collapsed_roads, surveyed_routes: None, signals: source.signals, road_tiles, signal_tiles, dims, timing, model_footprints: None, levels: HashMap::new(), frontage: None, paths: vec![], path_tiles: HashMap::new(), sections: None, traffic_sections: HashMap::new(), walk_gaps: HashMap::new(), walk_lane_clearance: None, junctions: Default::default(), stops: Default::default(), divided_pavement: HashMap::new(), deck_surfaces: HashMap::new(), deck_lanes: HashMap::new(), passing: true, bend_braking: false, crossings: None, zebras: None }
    }

    pub fn with_model_footprints(mut self, report: std::sync::Arc<crate::vehicle_catalogue::ScaleReport>) -> Self {
        self.model_footprints = Some(report);
        self
    }

    /// Build a network whose vehicle IDs select the same models as the game feed.
    pub fn new_network(&self) -> crate::Network {
        let net = crate::Network::new(self.timing.clone());
        match &self.model_footprints {
            Some(report) => net.with_model_footprints(report.clone(), Some(crate::vehicle_catalogue::RENDERED_SEED)),
            None => net,
        }
    }

    pub fn with_bend_braking(mut self, enabled: bool) -> Self {
        self.bend_braking = enabled;
        self
    }

    pub fn with_passing(mut self, passing: bool) -> Self {
        self.passing = passing;
        self
    }

    pub fn with_stops(mut self, stops: Vec<TrafficStop>) -> Self {
        self.stops = stops::StopIndex::new(stops);
        self
    }

    /// Heights of the drawn structures, keyed like their traffic level table.
    pub fn with_deck_surfaces(mut self, surfaces: Vec<TrafficDeckSurface>) -> Self {
        self.deck_surfaces.clear();
        self.deck_lanes.clear();
        for surface in surfaces {
            if !surface.lanes.is_empty() && surface.lanes.iter().all(|lane| {
                lane.width.is_finite() && lane.width > 0.0 && lane.points.len() >= 2
                    && lane.points.iter().flatten().all(|v| v.is_finite())
            }) {
                self.deck_lanes.insert(surface.way.clone(), surface.lanes);
            }
            if surface.points.len() >= 2 && surface.points.iter().flatten().all(|v| v.is_finite()) {
                self.deck_surfaces.entry(surface.way).or_default().push(surface.points);
            }
        }
        self
    }

    /// Register survey samples to the drive arc for this portion of their
    /// own source way. At a shared end keep the nearest sample from that way.
    fn lane_surface(
        &self, road: &SimRoad, points: &[P2], reach: f64, pads: &[Option<Arc<JunctionSurface>>; 2],
    ) -> (Vec<[f64; 2]>, Vec<[f64; 2]>) {
        let deck = self.deck_surfaces.get(&road.id).filter(|_| self.level_of(road) != 0);
        let street = self.traffic_sections.get(&road.id).filter(|_| self.level_of(road) == 0);
        if deck.is_none() && street.is_none() {
            return Default::default();
        }
        let cum = geom::cumulative(points);
        let mut heights = Vec::new();
        let mut register = |&[x, y, z]: &[f64; 3]| {
            if ![x, y, z].iter().all(|p| p.is_finite()) {
                return None;
            }
            let best = points.windows(2).zip(&cum).map(|(segment, &start)| {
                let (dist, p) = seg_nearest(segment[0], segment[1], x, z);
                let arc = start + (p[0] - segment[0][0]).hypot(p[1] - segment[0][1]);
                (dist, arc)
            }).min_by(|a, b| a.0.total_cmp(&b.0));
            let (dist, arc) = best.filter(|(dist, _)| *dist <= reach)?;
            heights.push((arc, dist, y));
            Some(arc)
        };
        for sample in deck.into_iter().flatten().flatten() {
            register(sample);
        }
        let mut carriageway = Vec::new();
        let use_carriageway = pads.iter().any(Option::is_some);
        for section in street.into_iter().flatten() {
            if !use_carriageway || section.carriageway_spans.is_empty() {
                for sample in &section.surface {
                    register(sample);
                }
                continue;
            }
            let arcs: Vec<_> = section.surface.iter().map(&mut register).collect();
            for &[lo, hi] in &section.carriageway_spans {
                let Some(span) = arcs.get(lo..=hi) else { continue };
                for run in span.split(|arc| arc.is_none()) {
                    let range = run.iter().flatten().fold([f64::INFINITY, f64::NEG_INFINITY], |[lo, hi], &arc| {
                        [lo.min(arc), hi.max(arc)]
                    });
                    if range[0] < range[1] {
                        carriageway.push(range);
                    }
                }
            }
        }
        heights.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        heights.dedup_by(|a, b| a.0 == b.0);
        if heights.len() < 2 {
            return Default::default();
        }
        let length = cum[cum.len() - 1];
        let ends = [heights[0].0, heights[heights.len() - 1].0];
        for (i, end) in [0.0, length].into_iter().enumerate() {
            if (end - ends[i]).abs() <= reach {
                continue;
            }
            // A deck or trimmed street can end at the drawn mouth while
            // its lane continues into the pad. Require a single pad interval
            // to reach both the end and the support samples on a street.
            let joins_end = |[lo, hi]: [f64; 2]| (lo..=hi).contains(&end) && (lo - reach..=hi + reach).contains(&ends[i]);
            let joined = pads[i].as_ref().is_some_and(|pad| {
                if deck.is_some() {
                    return pad.lane_span(points, &cum).is_some_and(joins_end);
                }
                !carriageway.is_empty() && pad.lane_intervals(points, &cum).into_iter().any(joins_end)
            });
            if !joined {
                return Default::default();
            }
        }
        (heights.into_iter().map(|(arc, _, y)| [arc, y]).collect(), carriageway)
    }

    /// Use the paved cross-sections of the same source ways.
    pub fn with_traffic_sections(self, sections: Vec<TrafficSection>) -> Self {
        self.with_traffic_plan(TrafficPlan { sections, ..Default::default() })
    }

    pub fn with_traffic_plan(mut self, plan: TrafficPlan) -> Self {
        self.surveyed_routes = None;
        self.traffic_sections.clear();
        self.walk_gaps.clear();
        let had_pads = self.junctions.has_pads();
        self.junctions = junctions::DrawnJunctions::new(plan.junctions);
        for mut section in plan.sections {
            section.lanes.sort_by(|a, b| a.lateral.total_cmp(&b.lateral));
            if section.half.is_finite() && section.half > 0.0
                && section.lanes.iter().all(|lane| {
                    matches!(lane.direction, -1 | 1) && lane.width.is_finite()
                        && lane.width > 0.0 && lane.lateral.is_finite()
                })
            {
                if section.walk_gaps.iter().any(|spans| !spans.is_empty()) {
                    walk_gaps::WalkGap::extend(self.walk_gaps.entry(section.way.clone()).or_default(), &section);
                }
                self.traffic_sections.entry(section.way.clone()).or_default().push(section);
            }
        }
        self.walk_lane_clearance = walk_gaps::WalkLaneClearance::from_plan(&self.roads, &self.traffic_sections);
        let mut named: HashMap<&str, (Vec<DrawnSection>, Vec<String>)> = HashMap::new();
        for road in self.roads.iter().filter(|r| is_divided_candidate(r)) {
            let Some(sections) = self.traffic_sections.get(&road.id) else { continue };
            let (strips, ways) = named.entry(&road.name).or_default();
            for section in sections {
                for edge in section.points.windows(2) {
                    let [a, b] = along_traffic([edge[0], edge[1]], road);
                    strips.push(DrawnSection { a, b, half: section.half, walks: section.walks });
                    ways.push(road.id.clone());
                }
            }
        }
        self.divided_pavement = named.into_iter().filter_map(|(name, (strips, ways))| {
            let multiple_ways = ways.first().is_some_and(|first| ways.iter().any(|way| way != first));
            multiple_ways.then(|| (name.to_owned(), DividedPavement {
                strips: SectionIndex::with_margin(strips, WALK_CLEARANCE_M), ways,
            }))
        }).collect();
        if had_pads || self.junctions.has_pads() {
            self.index_lane_roads();
        }
        if let Some(routes) = plan.surveyed_routes {
            match surveyed_routes::Routes::compile(&self, routes) {
                Ok(routes) => self.surveyed_routes = Some(routes),
                Err(err) => eprintln!("surveyed traffic routes disabled: {err:#}"),
            }
        }
        self
    }

    /// Compare the same paved graph using only its older divided-road cuts.
    pub fn without_drawn_walk_gaps(mut self) -> Self {
        self.walk_gaps.clear();
        self.walk_lane_clearance = None;
        self
    }

    fn junction_pad(&self, road: &SimRoad, node: P2, lane_end: P2) -> Option<Arc<JunctionSurface>> {
        let structure = self.level_of(road) != 0 || road.bridge || road.tunnel;
        let deck = self.deck_surfaces.get(&road.id);
        let joins_deck = |pad: &JunctionSurface| {
            deck.into_iter().flatten().any(|line| {
                [line[0], line[line.len() - 1]].into_iter().any(|[x, y, z]| {
                    pad.height_at_mouth([x, z], y).is_some_and(|h| (h - y).abs() <= STRUCTURE_MOUTH_MATCH_M)
                })
            })
        };
        let distance = |pad: &JunctionSurface| geom::hypot(node[0] - pad.position[0], node[1] - pad.position[1]);
        self.junctions.pads(&road.id)?.iter()
            .filter(|pad| pad.height_near(node, 0.0).is_some() || pad.height_near(lane_end, 0.0).is_some())
            .filter(|pad| !structure || joins_deck(pad))
            .min_by(|a, b| distance(a).total_cmp(&distance(b)))
            .cloned()
    }

    fn paved_registration(&self, sim: &Seg) -> PavedRegistration<'_> {
        let road = &self.roads[sim.road as usize];
        // Freeway and ramp connectors belong to the structure alignment,
        // including their at-grade ends.
        if geom::freeway(&road.highway) || road.bridge || road.tunnel || road.layer != 0 || self.level_of(road) != 0 {
            return PavedRegistration::default();
        }
        let Some(candidates) = self.traffic_sections.get(&road.id) else { return PavedRegistration::default() };
        let cum = geom::cumulative(&sim.pts);
        let mut samples: Vec<_> = [0.25, 0.5, 0.75].into_iter().filter_map(|fraction| {
            let (x, z, dx, dz, _) = geom::at_arc(&sim.pts, &cum, fraction * cum.last()?);
            let mut closest = None;
            for section in candidates {
                for edge in section.points.windows(2) {
                    let [a, b] = [edge[0], edge[1]];
                    let ux = b[0] - a[0];
                    let uz = b[1] - a[1];
                    let len = geom::hypot(ux, uz);
                    if len < 1e-6 {
                        continue;
                    }
                    let heading = (ux * dx + uz * dz) / len;
                    let (distance, p) = seg_nearest(a, b, x, z);
                    if heading.abs() < SECTION_MATCH_COS || distance > sim.half + section.half
                        || closest.as_ref().is_some_and(|(d, _, _)| *d <= distance)
                    {
                        continue;
                    }
                    let along = ((x - a[0]) * ux + (z - a[1]) * uz) / len;
                    closest = Some((distance, (0.0..=len).contains(&along), PavedMatch {
                        section,
                        lateral: (p[0] - x) * dz - (p[1] - z) * dx,
                        reversed: heading < 0.0,
                        drawn_chord: None,
                    }));
                }
            }
            closest.map(|(_, inside, sample)| (inside, sample))
        }).collect();
        // A registered shared lane remains shared when the segment also has
        // a different cross-section or needs more than one lateral offset.
        // Use samples inside the drawn run, without extending its ends.
        let uses_shared_lane = samples.iter().any(|(inside, m)| *inside && m.section.shared_bidirectional);
        let mut registration = PavedRegistration { profile: None, uses_shared_lane };
        let [(_, first), ..] = samples.as_slice() else { return registration };
        // A mapped segment may span a change in the paved cross-section.
        // Keep its existing profile until the graph can represent that change.
        if samples.len() != 3 || samples.iter().any(|(_, m)| m.reversed != first.reversed
            || m.section.half != first.section.half || m.section.walks != first.section.walks
            || m.section.shared_bidirectional != first.section.shared_bidirectional
            || m.section.lanes != first.section.lanes)
        {
            return registration;
        }
        if samples.iter().any(|(_, m)| (m.lateral - first.lateral).abs() > SECTION_REGISTER_VARIATION_M) {
            // A constant offset cannot register diverging chords. When
            // the mapped lanes would reintroduce a lane that the pavement
            // cannot fit, use the complete drawn one-way lane and keep its
            // existing routing nodes.
            if road.oneway == Oneway::Both
                || first.section.lanes.len() != 1
                || sim.forward + sim.backward <= 1
                || first.lane_counts(sim.forward, sim.backward).is_none()
                || !samples.iter().all(|(inside, m)| *inside && std::ptr::eq(first.section, m.section))
            {
                return registration;
            }
            let ([a, b], [p, q]) = (sim.pts.as_slice(), first.section.points.as_slice()) else {
                return registration;
            };
            let [p, q] = if first.reversed { [q, p] } else { [p, q] };
            let matching_end = |a: &P2, b: &P2| geom::hypot(a[0] - b[0], a[1] - b[1]) <= SECTION_MATCH_M;
            if matching_end(a, p) && matching_end(b, q) {
                registration.profile = Some(PavedMatch {
                    section: first.section, lateral: 0.0, reversed: first.reversed, drawn_chord: Some([*p, *q]),
                });
            }
            return registration;
        }
        samples.sort_by(|(_, a), (_, b)| a.lateral.total_cmp(&b.lateral));
        registration.profile = Some(samples.swap_remove(1).1);
        registration
    }

    /// The same source with the drawn street's cross-sections (`--crowd`):
    /// each street's footways stand on its drawn walks (their walking line
    /// half a throughway in from the walk's frontage zone, or the walk's middle
    /// on a narrow one) and stop, and cross, at the drawn kerbs of the streets
    /// they meet, so the walkers walk where the walks are drawn; a walk of its
    /// own is cut where it runs over a drawn carriageway. A street nothing
    /// drawn runs along (or a side drawn without a walk) keeps the class
    /// model. The lanes and the stop bars are not touched. Empty: the class
    /// model everywhere, as before.
    pub fn with_sections(mut self, pieces: Vec<DrawnSection>) -> Self {
        self.sections = (!pieces.is_empty()).then(|| std::sync::Arc::new(SectionIndex::new(pieces)));
        self
    }

    /// The same source with the mapped crosswalks' middles (`--crowd`): a
    /// walker crosses a street at an unsignalled junction only where a
    /// crosswalk is mapped within two crosswalk widths of its crossing band
    /// (a signal's crossings always: its heads serve them). Empty: at every
    /// junction, as before.
    pub fn with_crossings(mut self, points: Vec<P2>) -> Self {
        self.crossings = (!points.is_empty()).then(|| std::sync::Arc::new(CrossingIndex::new(points)));
        self
    }

    /// The same source with the zebras the street draws (`--people-crosswalks`;
    /// eo-app from eo-street's plan: the middle of each painted approach's
    /// crossing band): a footway's crossing band at a street end is the drawn
    /// zebra's, so walkers cross on the paint, and a street end with no zebra
    /// drawn is not crossed, at a signal too. Empty: as before.
    pub fn with_zebras(mut self, points: Vec<P2>) -> Self {
        self.zebras = (!points.is_empty()).then(|| std::sync::Arc::new(CrossingIndex::new(points)));
        self
    }

    /// The same source with walks of their own (`--crowd`): each becomes a
    /// footway of its own along its line (routable through the ends it shares
    /// with another), delivered with the tile of its middle vertex, its
    /// activity's `place` by its kind. Without it there are none.
    pub fn with_paths(mut self, paths: Vec<PathWay>) -> Self {
        self.path_tiles.clear();
        for (i, p) in paths.iter().enumerate() {
            if p.points.len() >= 2 {
                let mid = p.points[p.points.len() / 2];
                self.path_tiles.entry(TileKey::of(mid[0], mid[1])).or_default().push(i as u32);
            }
        }
        self.paths = paths;
        self
    }

    /// The same source with the building fronts its footways are measured
    /// against (`--crowd`: each footway's `activity`, what the crowd's density
    /// by place reads). Without it every footway's activity is zero.
    pub fn with_frontage(mut self, edges: Vec<FrontEdge>) -> Self {
        self.frontage = (!edges.is_empty()).then(|| std::sync::Arc::new(FrontageIndex::new(edges)));
        self
    }

    /// Whether a frontage survey is set (`with_frontage`).
    pub fn has_frontage(&self) -> bool {
        self.frontage.is_some()
    }

    /// The same source with lane levels (`levels`: OSM way id -> structure
    /// level; plain data, the simulation learns no deck geometry). A way on a
    /// level other than 0 builds its lanes and carriageway on that level and
    /// no footway. An empty table leaves everything as before.
    pub fn with_levels(mut self, levels: HashMap<String, i8>) -> Self {
        self.levels = levels;
        if self.junctions.has_pads() {
            self.index_lane_roads();
        }
        self
    }

    fn index_lane_roads(&mut self) {
        self.junctions.align_roads(&self.roads, &self.levels);
        self.road_tiles.clear();
        for (i, road) in self.roads.iter().enumerate() {
            let mut tiles = tiles_of_polyline(&road.points);
            if let Some(aligned) = self.junctions.road(&road.id) {
                tiles.extend(aligned.nodes.iter().map(|p| TileKey::of(p[0], p[1])));
                tiles.sort_unstable();
                tiles.dedup();
            }
            for tile in tiles {
                self.road_tiles.entry(tile).or_default().push(i as u32);
            }
        }
    }

    /// The level table (`with_levels`).
    pub fn levels(&self) -> &HashMap<String, i8> {
        &self.levels
    }

    /// The structure level of a road (0 when the table does not list it).
    fn level_of(&self, road: &SimRoad) -> i8 {
        if self.levels.is_empty() { 0 } else { self.levels.get(&road.id).copied().unwrap_or(0) }
    }

    pub fn road_count(&self) -> usize {
        self.roads.len()
    }

    /// Tiles indexed by the vehicle-road source, in reproducible sweep order.
    pub fn vehicle_road_tiles(&self) -> Vec<TileKey> {
        let mut tiles: Vec<_> = self.road_tiles.keys().copied().collect();
        tiles.sort_unstable();
        tiles
    }

    /// A lane is delivered with its downstream tile. Keep both ends of a
    /// bridge resident wherever its span intersects the requested window.
    pub fn resident_tiles(&self, x: f64, z: f64, radius: f64) -> Vec<TileKey> {
        let mut tiles = tiles_around(x, z, radius);
        let mut ends = Vec::new();
        for tile in &tiles {
            if let Some(roads) = self.road_tiles.get(tile) {
                for &id in roads {
                    let road = &self.roads[id as usize];
                    if road.bridge && self.level_of(road) != 0 {
                        let [a, b] = [road.points[0], road.points[road.points.len() - 1]];
                        ends.extend([TileKey::of(a[0], a[1]), TileKey::of(b[0], b[1])]);
                    }
                }
            }
        }
        tiles.extend(ends);
        if let Some(routes) = self.active_surveyed_routes()
            && tiles.iter().any(|tile| routes.tiles.contains(tile))
        {
            tiles.extend_from_slice(&routes.tiles);
        }
        tiles.sort_unstable();
        tiles.dedup();
        tiles
    }

    /// Surveyed connections spanning tile ownership arrive as one delivery.
    pub fn delivery_tiles(&self, tile: TileKey) -> Vec<TileKey> {
        if let Some(routes) = self.active_surveyed_routes().filter(|r| r.tiles.contains(&tile)) {
            return routes.tiles.clone();
        }
        vec![tile]
    }

    fn active_surveyed_routes(&self) -> Option<&surveyed_routes::Routes> {
        self.model_footprints.as_ref()?;
        self.surveyed_routes.as_ref()
    }

    /// `classLaneWidthOf` (sf-data.js) through caMetric.
    fn lane_width(&self, r: &SimRoad) -> f64 {
        let hw = r.highway.trim().to_ascii_lowercase();
        let state = if eo_data::standards::is_sf_state_route(&hw, &r.name) {
            if hw == "motorway" || hw == "motorway_link" {
                self.dims.state_lane_m
            } else {
                let posted = if r.maxspeed.is_empty() { 0 } else { eo_data::standards::parse_speed_limit_kmh(&r.maxspeed) };
                if posted > eo_data::standards::parse_speed_limit_kmh("40 mph") {
                    self.dims.state_lane_m
                } else {
                    self.dims.state_urban_lane_m
                }
            }
        } else {
            0.0
        };
        StreetDims::ceil_to_cm(if state > 0.0 { state } else { self.dims.city_lane_m })
    }

    fn sidewalk_width(&self, highway: &str) -> f64 {
        match highway {
            "primary" | "secondary" | "trunk" | "primary_link" | "secondary_link" => self.dims.sidewalk_commercial_m,
            "service" | "living_street" => self.dims.sidewalk_alley_m,
            "unclassified" => self.dims.sidewalk_industrial_m,
            _ => self.dims.sidewalk_residential_m,
        }
    }

    /// Build one delivery: every lane ending, footway centred and signal standing in `tiles`.
    pub fn build(&self, tiles: &[TileKey]) -> NetworkBuild {
        let want: std::collections::HashSet<TileKey> = tiles.iter().copied().collect();
        // Roads touching the tiles and one ring around them (for node degrees and the far ends of lanes).
        let mut road_ids: Vec<u32> = vec![];
        for t in tiles {
            for dx in -1..=1 {
                for dz in -1..=1 {
                    if let Some(v) = self.road_tiles.get(&TileKey { x: t.x + dx, z: t.z + dz }) {
                        road_ids.extend_from_slice(v);
                    }
                }
            }
        }
        road_ids.sort_unstable();
        road_ids.dedup();
        let splits = self.junctions.split_nodes();
        let mut local_degree: HashMap<(i64, i64), u16> = HashMap::new();
        if splits.is_none() {
            // The raw-source mode uses degree over the delivered roads.
            for &r in &road_ids {
                let mut seen: Vec<_> = self.roads[r as usize].points.iter().map(|p| qkey(*p)).collect();
                seen.sort_unstable();
                seen.dedup();
                for key in seen {
                    *local_degree.entry(key).or_default() += 1;
                }
            }
        }
        let splits_at = |p| {
            let key = qkey(p);
            splits.map_or_else(|| local_degree.get(&key).copied().unwrap_or(0) >= 2, |nodes| nodes.contains(&key))
        };
        // Split at shared vertices.
        let mut segs: Vec<Seg> = vec![];
        for &r in &road_ids {
            let road = &self.roads[r as usize];
            let (points, nodes) = self.junctions.road(&road.id)
                .map_or((&road.points, &road.points), |a| (&a.points, &a.nodes));
            let lane_w = self.lane_width(road);
            let n = road.lanes.unwrap_or(if road.oneway == Oneway::Both { 2 } else { 1 }).max(1);
            let (forward, backward) = match road.oneway {
                Oneway::Forward => (n, 0),
                Oneway::Reverse => (0, n),
                Oneway::Both => ((n + 1) / 2, (n / 2).max(1)),
            };
            let half = (forward + backward) as f64 * lane_w / 2.0;
            let mut start = 0;
            let mut k = 0;
            for i in 1..points.len() {
                let last = i == points.len() - 1;
                if last || splits_at(nodes[i]) {
                    let pts: Vec<P2> = points[start..=i].to_vec();
                    let len: f64 = pts.windows(2).map(|w| geom::hypot(w[1][0] - w[0][0], w[1][1] - w[0][1])).sum();
                    if len > junctions::MIN_SEGMENT_M {
                        segs.push(Seg { road: r, key: format!("n{}:{}", road.id, k), pts, nodes: [nodes[start], nodes[i]], half, lane_w, forward, backward });
                        k += 1;
                    }
                    start = i;
                }
            }
        }
        let mut build = NetworkBuild {
            tiles: tiles.to_vec(),
            merge_max_lateral_m: self.dims.city_lane_m,
            following_time_gap_s: Some(self.dims.following_time_gap_s),
            continuous_lane_heading: true,
            bend_braking: self.bend_braking,
            contain_walk_corners: !self.traffic_sections.is_empty(),
            ..Default::default()
        };
        let paved: Vec<_> = segs.iter().map(|s| self.paved_registration(s)).collect();
        // Lanes, kept when their end node lies in the delivered tiles.
        let mut metas: Vec<LaneMeta> = vec![];
        for (si, s) in segs.iter().enumerate() {
            let road = &self.roads[s.road as usize];
            // A single shared lane needs alternating traffic even where the
            // raw road extract reports two lanes. Keep the narrower tagged
            // case for plans exported before the shared role was recorded.
            if paved[si].uses_shared_lane || paved[si].profile.as_ref().is_some_and(|p| {
                p.section.shared_bidirectional
                    || (road.oneway == Oneway::Both && road.lanes == Some(1)
                        && p.section.lanes.is_empty() && p.section.half < s.half)
            }) {
                continue;
            }
            let (paved_lanes, forward, backward) = paved[si].profile.as_ref()
                .and_then(|p| p.lane_counts(s.forward, s.backward).map(|(f, b)| (Some(p), f, b)))
                .unwrap_or((None, s.forward, s.backward));
            let centreline = paved_lanes.and_then(|p| p.drawn_chord.as_ref()).map_or(s.pts.as_slice(), |p| p.as_slice());
            // Painted bridge tracks register to an entire one-way way. Keep
            // the mapped endpoints for routing and tile ownership.
            let road_nodes = self.junctions.road(&road.id).map_or(&road.points, |p| &p.nodes);
            let deck_lanes = self.deck_lanes.get(&road.id).filter(|lanes| {
                self.level_of(road) != 0 && road.oneway != Oneway::Both
                    && lanes.len() == (forward + backward) as usize
                    && s.nodes == [road_nodes[0], *road_nodes.last().unwrap()]
            });
            for (dir_decreasing, count) in [(false, forward), (true, backward)] {
                let passing = paved_lanes.filter(|_| self.passing && count > 1)
                    .and_then(|p| p.passing_span(centreline, dir_decreasing).map(|span| (p, span)));
                let mut adjacent = None;
                for k in 0..count {
                    let offset = if s.forward > 0 && s.backward > 0 {
                        (k as f64 + 0.5) * s.lane_w * if dir_decreasing { 1.0 } else { -1.0 }
                    } else {
                        // One-way: lanes spread about the centreline, rightmost first.
                        let centred = (k as f64 - (count as f64 - 1.0) / 2.0) * s.lane_w;
                        if dir_decreasing { centred } else { -centred }
                    };
                    let (offset, mut width) = paved_lanes.and_then(|p| p.lane(dir_decreasing, k)).unwrap_or((offset, s.lane_w));
                    let (start_node, end_node) = if dir_decreasing {
                        (s.nodes[1], s.nodes[0])
                    } else {
                        (s.nodes[0], s.nodes[1])
                    };
                    let tile = TileKey::of(end_node[0], end_node[1]);
                    if !want.contains(&tile) {
                        continue;
                    }
                    let mut deck_surface = None;
                    let points = if let Some(lane) = deck_lanes.and_then(|lanes| lanes.get(k as usize)) {
                        width = lane.width;
                        let mut points: Vec<_> = lane.points.iter().map(|&[x, _, z]| [x, z]).collect();
                        let mut heights: Vec<_> = lane.points.iter().map(|p| p[1]).collect();
                        if dir_decreasing {
                            points.reverse();
                            heights.reverse();
                        }
                        deck_surface = Some(geom::cumulative(&points).into_iter().zip(heights).map(|(s, y)| [s, y]).collect());
                        points
                    } else {
                        geom::junction_lane_points(centreline, offset, dir_decreasing)
                    };
                    let index = build.lanes.len() as LaneId;
                    let pads = [self.junction_pad(road, start_node, points[0]),
                        self.junction_pad(road, end_node, points[points.len() - 1])];
                    let (surface, carriageway) = deck_surface.map_or_else(
                        || self.lane_surface(road, &points, s.half + SECTION_MATCH_M, &pads),
                        |surface| (surface, Vec::new()),
                    );
                    if !surface.is_empty() {
                        build.lane_surfaces.push((index, surface));
                    }
                    if !carriageway.is_empty() {
                        build.lane_carriageway_spans.push((index, carriageway));
                    }
                    if pads.iter().any(Option::is_some) {
                        build.lane_junctions.push((index, pads));
                    }
                    if let Some((p, span)) = passing {
                        if let Some((left, left_offset)) = adjacent {
                            if p.broken_divider(dir_decreasing, offset, left_offset) {
                                build.passing.push((index, LanePass { to: left, span }));
                            }
                        }
                        adjacent = Some((index, offset));
                    }
                    build.lanes.push(LaneBuild {
                        key: format!("{}-{}-{}", s.key, if dir_decreasing { "reverse" } else { "forward" }, k + 1),
                        segment: s.key.clone(),
                        street: road.name.clone(),
                        highway: road.highway.clone(),
                        role: LaneRole::Through,
                        painted_through_mapped: false,
                        width,
                        points,
                        start_node,
                        end_node,
                        max_speed: geom::autonomous_speed(&road.highway, &road.maxspeed, None, ""),
                        control: ControlKind::None,
                        outgoing: None,
                        legal: None,
                        tile: Some(tile),
                        level: self.level_of(road),
                    });
                    metas.push(LaneMeta { seg: si, end: end_node, reverse: dir_decreasing });
                }
            }
        }
        // Junction nodes in the delivery: lanes arriving per node (at grade:
        // a street's signal never controls a deck's lanes over it).
        let mut arriving: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        for (li, m) in metas.iter().enumerate() {
            if build.lanes[li].level == 0 {
                arriving.entry(qkey(m.end)).or_default().push(li);
            }
        }
        // Signals.
        let mut claimed_nodes: std::collections::HashSet<(i64, i64)> = Default::default();
        // Signals standing in the tiles or one ring out: a signal controls the delivered
        // nodes near it, whichever tile the record itself stands in.
        let mut signal_ids: Vec<u32> = vec![];
        for t in tiles {
            for dx in -1..=1 {
                for dz in -1..=1 {
                    if let Some(v) = self.signal_tiles.get(&TileKey { x: t.x + dx, z: t.z + dz }) {
                        signal_ids.extend_from_slice(v);
                    }
                }
            }
        }
        signal_ids.sort_unstable();
        signal_ids.dedup();
        let mut node_junction: HashMap<(i64, i64), u32> = HashMap::new();
        for sid in signal_ids {
            let sp = self.signals[sid as usize];
            // Street nodes near the signal where two or more streets meet.
            let mut nodes: Vec<((i64, i64), P2)> = arriving
                .iter()
                .filter(|(k, lanes)| {
                    let p = metas[lanes[0]].end;
                    let streets: std::collections::HashSet<&str> =
                        lanes.iter().map(|&l| self.roads[segs[metas[l].seg].road as usize].name.as_str()).collect();
                    let _ = k;
                    geom::hypot(p[0] - sp[0], p[1] - sp[1]) <= JUNCTION_NODE_MAX_M && (streets.len() >= 2 || lanes.len() >= 3)
                })
                .map(|(k, lanes)| (*k, metas[lanes[0]].end))
                .collect();
            nodes.sort_by(|a, b| {
                geom::hypot(a.1[0] - sp[0], a.1[1] - sp[1]).total_cmp(&geom::hypot(b.1[0] - sp[0], b.1[1] - sp[1]))
            });
            let Some(&(_, first)) = nodes.first() else { continue };
            nodes.retain(|n| geom::hypot(n.1[0] - first[0], n.1[1] - first[1]) <= SPLIT_JUNCTION_M);
            if nodes.iter().any(|n| claimed_nodes.contains(&n.0)) {
                continue; // a duplicate record of a crossing already controlled
            }
            let centre = {
                let (sx, sz) = nodes.iter().fold((0.0, 0.0), |a, n| (a.0 + n.1[0], a.1 + n.1[1]));
                [sx / nodes.len() as f64, sz / nodes.len() as f64]
            };
            // Approaches: lanes driven toward the nodes.
            let mut approaches: Vec<usize> = vec![];
            for n in &nodes {
                for &l in &arriving[&n.0] {
                    let pts = &build.lanes[l].points;
                    let closure = geom::hypot(pts[0][0] - centre[0], pts[0][1] - centre[1])
                        - geom::hypot(pts[pts.len() - 1][0] - centre[0], pts[pts.len() - 1][1] - centre[1]);
                    if closure >= APPROACH_MIN_CLOSURE_M {
                        approaches.push(l);
                    }
                }
            }
            if approaches.is_empty() {
                continue;
            }
            for n in &nodes {
                claimed_nodes.insert(n.0);
            }
            let bearings: Vec<(f64, String)> = approaches
                .iter()
                .map(|&l| {
                    let p = &build.lanes[l].points;
                    let (a, b) = (p[p.len() - 2], p[p.len() - 1]);
                    ((b[1] - a[1]).atan2(b[0] - a[0]), build.lanes[l].key.clone())
                })
                .collect();
            let mut axes = cluster_axes(&bearings, self.timing.axis_tolerance_rad);
            if axes.len() > self.timing.max_phases {
                let rank = |m: &Vec<usize>| m.iter().map(|&k| geom::class_rank(&build.lanes[approaches[k]].highway)).max().unwrap_or(0);
                let mut order: Vec<usize> = (0..axes.len()).collect();
                order.sort_by(|&a, &b| rank(&axes[b].1).cmp(&rank(&axes[a].1)).then(axes[b].1.len().cmp(&axes[a].1.len())));
                let kept: std::collections::HashSet<usize> = order[..self.timing.max_phases].iter().copied().collect();
                let mut reordered = vec![];
                for (i, a) in axes.iter().enumerate() {
                    if kept.contains(&i) {
                        reordered.push(a.clone());
                    }
                }
                for (i, a) in axes.iter().enumerate() {
                    if !kept.contains(&i) {
                        reordered.push(a.clone());
                    }
                }
                axes = reordered;
            }
            let half_of = |l: usize| segs[metas[l].seg].half;
            let junction_index = build.junctions.len() as u32;
            let mut input = JunctionInput {
                signal_ids: vec![format!("native-signal-{sid}")],
                centre,
                phase_offset: 0.0,
                orphan: false,
                axes: vec![],
                crossing_m: vec![],
                tile: {
                    // Owned by the tile of its nearest controlled node (a delivered tile).
                    let t = TileKey::of(first[0], first[1]);
                    Some((t.x, t.z))
                },
            };
            for (ai, (bearing, members)) in axes.iter().enumerate() {
                let other_half = axes
                    .iter()
                    .enumerate()
                    .filter(|(k, _)| *k != ai)
                    .flat_map(|(_, a)| a.1.iter().map(|&m| half_of(approaches[m])))
                    .fold(0.0f64, f64::max);
                let stop = other_half + self.dims.crosswalk_m + self.dims.limit_line_setback_m;
                let mut ms = vec![];
                let mut crossing = 0.0f64;
                for &m in members {
                    let l = approaches[m];
                    let road = &self.roads[segs[metas[l].seg].road as usize];
                    let recorded = geom::parse_posted_speed_mps(&road.maxspeed);
                    // Bar measured back from the lane's end: the lane ends `end_gap` short of the centre.
                    let p = &build.lanes[l].points;
                    let end_gap = geom::hypot(p[p.len() - 1][0] - centre[0], p[p.len() - 1][1] - centre[1]);
                    let stop_offset = (stop - end_gap).max(0.0);
                    build.lanes[l].control = ControlKind::Signal {
                        junction: junction_index,
                        axis: ai as u8,
                        stop_offset_m: stop_offset,
                        bar_depth_m: self.dims.limit_line_width_m,
                    };
                    crossing = crossing.max(2.0 * half_of(l));
                    ms.push(MemberInput {
                        lane: l as u32,
                        highway: road.highway.clone(),
                        posted_mps: recorded.unwrap_or(self.timing.prima_facie_mps),
                        recorded_mps: recorded,
                        stop_offset_m: stop_offset,
                        carriageway_half_m: Some(half_of(l)),
                    });
                }
                input.axes.push(AxisInput { bearing: *bearing, members: ms });
                input.crossing_m.push(crossing);
            }
            for n in &nodes {
                node_junction.insert(n.0, junction_index);
            }
            build.junctions.push(input);
        }
        // Footways, kept when their midpoint lies in the delivered tiles.
        // (`--crowd`) each street's drawn cross-section, where one is drawn
        // along it: the footways' kerbs are the drawn ones.
        let drawn: Vec<Option<(f64, [f64; 2])>> = match &self.sections {
            Some(ix) => segs.iter().map(|s| ix.of(&s.pts)).collect(),
            None => vec![None; segs.len()],
        };
        let kerb_half = |si: usize| paved[si].profile.as_ref().map(|p| p.section.half)
            .or_else(|| drawn[si].map(|d| d.0)).unwrap_or(segs[si].half);
        let mut node_half: HashMap<(i64, i64), Vec<(usize, f64)>> = HashMap::new();
        for (si, s) in segs.iter().enumerate() {
            for p in s.nodes {
                node_half.entry(qkey(p)).or_default().push((si, kerb_half(si)));
            }
        }
        if self.junctions.has_pads() {
            self.apply_stops(&segs, &metas, &node_half, &claimed_nodes, &mut build);
        }
        let mut foot_nodes: HashMap<String, FootNodeBuild> = HashMap::new();
        for (si, s) in segs.iter().enumerate() {
            let road = &self.roads[s.road as usize];
            let hw = road.highway.as_str();
            // A street's carriageway and two footways go with the tile of its
            // centreline's middle vertex, so a delivery never leaves content in a
            // tile it did not deliver and a kerb's far kerb is always resident.
            let mid = s.pts[s.pts.len() / 2];
            let seg_tile = TileKey::of(mid[0], mid[1]);
            if !want.contains(&seg_tile) {
                continue;
            }
            let level = self.level_of(road);
            let centreline = paved[si].profile.as_ref().and_then(|p| p.drawn_chord.as_ref())
                .map_or(s.pts.as_slice(), |p| p.as_slice());
            if !(road.bridge || road.tunnel) {
                let (half, points) = match &paved[si].profile {
                    Some(p) => (p.section.half, geom::junction_lane_points(centreline, p.lateral, false)),
                    None => (s.half, s.pts.clone()),
                };
                build.segments.push(SegmentBuild { half, points, level });
            }
            // (no walkers on a structure level: footways stay at grade.)
            if matches!(hw, "motorway" | "motorway_link" | "trunk" | "trunk_link")
                || matches!(road.sidewalk.as_str(), "no" | "none" | "separate")
                || road.bridge
                || road.tunnel
                || level != 0
            {
                continue;
            }
            let walk_line =
                s.half + self.sidewalk_width(hw) - self.dims.frontage_zone_m - self.dims.throughway_m / 2.0;
            // (`--crowd`) on the drawn walk: half a throughway in from its
            // frontage zone, or its middle where it is narrower than that.
            let prepared_section = paved[si].profile.as_ref().map(|p| (p.section.half, p.walks()));
            let line_on = |side: i8| -> f64 {
                let section = prepared_section.or(drawn[si]);
                match section {
                    Some((h, w)) => {
                        let walk = w[if side > 0 { 0 } else { 1 }];
                        if walk >= self.dims.throughway_m / 2.0 {
                            h + (walk - self.dims.frontage_zone_m - self.dims.throughway_m / 2.0).max(walk / 2.0)
                        } else {
                            walk_line
                        }
                    }
                    None => walk_line,
                }
            };
            let ends_info = |neighbors: Option<&[(usize, f64)]>| {
                let neighbors = neighbors.unwrap_or_default();
                let half = neighbors.iter().filter(|(k, _)| *k != si).map(|(_, h)| *h).fold(0.0f64, f64::max);
                (half, neighbors.len() > 2)
            };
            let a = centreline[0];
            let b = centreline[centreline.len() - 1];
            let adjoining = s.nodes.map(|p| node_half.get(&qkey(p)).map(Vec::as_slice));
            let [(ha, ja), (hb, jb)] = adjoining.map(ends_info);
            // The drawn junction can cover a short segment between two
            // traffic nodes. Its lanes still cross this sidewalk.
            let joins = |other: usize| {
                adjoining.iter().flatten().any(|neighbors| {
                    neighbors.iter().any(|&(near, _)| near != si && segs[near].road as usize == other)
                }) || self.junctions.share_pad_at(&road.id, &self.roads[other].id, s.nodes)
            };
            let divided = is_divided_candidate(road).then(|| self.divided_pavement.get(&road.name)).flatten();
            let looped_way = road.points.first() == road.points.last();
            let shared_crosses = paved[si].profile.as_ref()
                .is_some_and(|p| p.section.shared_bidirectional && p.section.crosses_road);
            let gaps = self.walk_gaps.get(&road.id);
            let clipped_bend = gaps.is_some() && paved[si].profile.is_some() && centreline.windows(3).any(|points| {
                let [a, b, c] = [points[0], points[1], points[2]];
                let [dx, dz] = [b[0] - a[0], b[1] - a[1]];
                let [ux, uz] = [c[0] - b[0], c[1] - b[1]];
                dx * ux + dz * uz < SECTION_MATCH_COS * dx.hypot(dz) * ux.hypot(uz)
            });
            let same_way_margin = if looped_way {
                Some(0.0)
            } else {
                clipped_bend.then_some(-PERSON_CAPSULE_R_M)
            };
            let lateral = paved[si].profile.as_ref().map_or(0.0, |p| p.lateral);
            let centres = (gaps.is_some() || lateral != 0.0).then(|| geom::junction_lane_points(centreline, lateral, false));
            let centre = centres.as_deref().unwrap_or(centreline);
            let mut crossing_ends = [[None; 2]; 2];
            for (k, side) in [1i8, -1i8].iter().enumerate() {
                // The matched source way explicitly has no sidewalk on this side.
                if prepared_section.is_some_and(|(_, walks)| walks[k] == 0.0) {
                    continue;
                }
                let walk_offset = line_on(*side);
                let offset = walk_offset * *side as f64 + lateral;
                let points = geom::junction_lane_points(centreline, offset, false);
                let d = [b[0] - a[0], b[1] - a[1]];
                let dl = geom::hypot(d[0], d[1]).max(1e-9);
                // Same normal convention as the lane offsets: +offset moves along (dz, -dx).
                let normal = [d[1] / dl, -d[0] / dl];
                let trim = |h: f64, junction: bool| if junction || h > 0.0 { h } else { 0.0 };
                // (`--crowd`) whether a crosswalk is mapped across this street at
                // an end's crossing band (its centreline there: `arc` from the
                // node `p` toward `toward`), within two crosswalk widths.
                let mapped = |p: P2, toward: P2, arc: f64| -> bool {
                    let Some(cx) = &self.crossings else { return true };
                    let (dx, dz) = (toward[0] - p[0], toward[1] - p[1]);
                    let l = geom::hypot(dx, dz).max(1e-9);
                    let t = arc.min(l);
                    cx.near(p[0] + dx / l * t, p[1] + dz / l * t, self.dims.crosswalk_m * 2.0)
                };
                // (`--people-crosswalks`) the zebra the street draws across
                // this street at this end (eo-street's plan: a painted
                // approach's band middle, on its centreline): the band moves
                // onto it, and with none drawn the street is not crossed
                // here, signal or not. The zebra is taken within two
                // crosswalk widths of the class band, ahead of the node and
                // within half a crosswalk of the centreline.
                let zebra = |p: P2, toward: P2, arc: f64| -> Option<Option<f64>> {
                    let zx = self.zebras.as_ref()?;
                    let (dx, dz) = (toward[0] - p[0], toward[1] - p[1]);
                    let l = geom::hypot(dx, dz).max(1e-9);
                    let (ux, uz) = (dx / l, dz / l);
                    let t = arc.min(l);
                    let reach = self.dims.crosswalk_m * 2.0;
                    let found = zx.nearest_by(p[0] + ux * t, p[1] + uz * t, reach, |m| {
                        let (mx, mz) = (m[0] - p[0], m[1] - p[1]);
                        mx * ux + mz * uz > 0.0 && (mx * uz - mz * ux).abs() <= self.dims.crosswalk_m / 2.0
                    });
                    Some(found.map(|m| (m[0] - p[0]) * ux + (m[1] - p[1]) * uz))
                };
                let end = |h: f64, junction: bool, p: P2, toward: P2| {
                    let arc = trim(h, junction) + self.dims.crosswalk_m / 2.0;
                    match zebra(p, toward, arc) {
                        Some(Some(at)) => FootEndBuild { kerb_trim: trim(h, junction), crossing_arc: at.max(trim(h, junction)), no_crossing: false, zebra_only: true },
                        Some(None) => FootEndBuild { kerb_trim: trim(h, junction), crossing_arc: arc, no_crossing: true, zebra_only: true },
                        None => FootEndBuild { kerb_trim: trim(h, junction), crossing_arc: arc, no_crossing: !mapped(p, toward, arc), zebra_only: false },
                    }
                };
                let footway = FootwayBuild {
                    points,
                    highway: road.highway.clone(),
                    segment: s.key.clone(),
                    road_side: *side,
                    node_keys: Some(s.nodes.map(node_text)),
                    normal,
                    cross_index: [None; 2],
                    original_arc_start: None,
                    ends: [end(ha, ja, a, centreline[1]), end(hb, jb, b, centreline[centreline.len() - 2])],
                    tile: Some(seg_tile),
                };
                let at_grade = |r: usize| self.level_of(&self.roads[r]) == 0;
                let overlap_margin = |other: usize| {
                    if other == s.road as usize {
                        // A bent, clipped sidewalk can run along its own
                        // motor lane. Remove the part well inside the lane.
                        return same_way_margin;
                    }
                    if joins(other) {
                        return Some(WALK_CLEARANCE_M);
                    }
                    // A shared road yielding to another street can have a
                    // sidewalk continuation inside its live lanes.
                    if shared_crosses {
                        return Some(0.0);
                    }
                    // A road that ends inside the walking corridor can
                    // occupy the sidewalk with no common junction node.
                    // Through crossings still use the route's kerb rules.
                    let points = &self.roads[other].points;
                    let (first, last) = points.first().zip(points.last())?;
                    (first != last && [first, last].into_iter().any(|p| {
                        geom::dist_to_polyline(centre, p[0], p[1]) <= walk_offset
                    })).then_some(0.0)
                };
                let lane_clearance = self.walk_lane_clearance.as_ref()
                    .filter(|lanes| lanes.may_block(&footway.points, s.road as usize, at_grade, &overlap_margin));
                if divided.is_some() || gaps.is_some() || lane_clearance.is_some() {
                    let runs = walk_runs(&footway.points, |p, heading, piece, fraction| {
                        let blocked_by_gap = if let (Some(gaps), Some(centres)) = (gaps, &centres) {
                            let [a, b] = [centres[piece], centres[piece + 1]];
                            let d = [b[0] - a[0], b[1] - a[1]];
                            let middle = [a[0] + d[0] * fraction, a[1] + d[1] * fraction];
                            gaps.iter().any(|gap| gap.covers(middle, d, *side))
                        } else {
                            false
                        };
                        blocked_by_gap || divided.is_some_and(|pavement| pavement.covers_sidewalk(road, &self.levels, p, heading))
                            || lane_clearance.is_some_and(|lanes| lanes.blocks(p, heading, s.road as usize, at_grade, &overlap_margin))
                    });
                    for (part, retain) in sidewalk_parts(footway, runs) {
                        for (e, kept) in retain.into_iter().enumerate() {
                            if kept {
                                crossing_ends[k][e] = Some(build.footways.len() as u32);
                            }
                        }
                        build.footways.push(part);
                    }
                } else {
                    if !self.traffic_sections.is_empty() {
                        let length: f64 = footway.points.windows(2).map(|p| geom::hypot(p[1][0] - p[0][0], p[1][1] - p[0][1])).sum();
                        // Both kerbs can consume a short street connector.
                        // There is no sidewalk between them to spawn or route on.
                        if length <= footway.ends[0].kerb_trim + footway.ends[1].kerb_trim {
                            continue;
                        }
                    }
                    crossing_ends[k] = [Some(build.footways.len() as u32); 2];
                    build.footways.push(footway);
                }
            }
            for e in 0..2 {
                if let [Some(left), Some(right)] = [crossing_ends[0][e], crossing_ends[1][e]] {
                    build.footways[left as usize].cross_index[e] = Some(right);
                    build.footways[right as usize].cross_index[e] = Some(left);
                }
            }
            for p in s.nodes {
                let key = node_text(p);
                let signal = node_junction.get(&qkey(p)).copied();
                foot_nodes.entry(key.clone()).or_insert(FootNodeBuild { key, pos: p, signal });
            }
        }
        // (`--crowd`) the walks of their own in these tiles, in tile then path order.
        let first_path = build.footways.len();
        let mut path_place: Vec<f32> = vec![];
        let mut path_ids: Vec<u32> = tiles.iter().filter_map(|t| self.path_tiles.get(t)).flatten().copied().collect();
        path_ids.sort_unstable();
        path_ids.dedup();
        for i in path_ids {
            let p = &self.paths[i as usize];
            // (with the drawn sections) a walk is cut where it runs over a drawn
            // carriageway: its walkers keep to the paving either side.
            let runs: Vec<Vec<P2>> = match &self.sections {
                Some(ix) => ix.off_carriageway(&p.points),
                None => vec![p.points.clone()],
            };
            for (ri, run) in runs.into_iter().enumerate() {
                let (a, b) = (run[0], run[run.len() - 1]);
                let d = [b[0] - a[0], b[1] - a[1]];
                let dl = geom::hypot(d[0], d[1]);
                let len: f64 = run.windows(2).map(|w| geom::hypot(w[1][0] - w[0][0], w[1][1] - w[0][1])).sum();
                if len < 4.0 || dl < 1e-6 {
                    continue;
                }
                let mid = run[run.len() / 2];
                build.footways.push(FootwayBuild {
                    points: run,
                    highway: p.highway.clone(),
                    segment: if ri == 0 { format!("p{}", p.id) } else { format!("p{}r{ri}", p.id) },
                    road_side: 1,
                    node_keys: Some([node_text(a), node_text(b)]),
                    normal: [d[1] / dl, -d[0] / dl],
                    cross_index: [None; 2],
                    original_arc_start: None,
                    ends: [FootEndBuild { kerb_trim: 0.0, crossing_arc: 0.0, no_crossing: false, zebra_only: false }, FootEndBuild { kerb_trim: 0.0, crossing_arc: 0.0, no_crossing: false, zebra_only: false }],
                    tile: Some(TileKey::of(mid[0], mid[1])),
                });
                path_place.push(if p.highway == "pedestrian" { crate::consts::PLACE_PEDESTRIAN } else { crate::consts::PLACE_FOOTPATH });
                for q in [a, b] {
                    let key = node_text(q);
                    foot_nodes.entry(key.clone()).or_insert(FootNodeBuild { key, pos: q, signal: None });
                }
            }
        }
        let mut nodes: Vec<FootNodeBuild> = foot_nodes.into_values().collect();
        nodes.sort_by(|a, b| a.key.cmp(&b.key));
        build.foot_nodes = nodes;
        if !path_place.is_empty() && build.footway_activity.is_empty() {
            build.footway_activity = vec![Default::default(); build.footways.len()];
        }
        if let Some(fr) = &self.frontage {
            // The class model's walk line stands half a throughway in from the
            // frontage zone's edge, on a carriageway of travel lanes alone: the
            // surveyed building line can be a parking lane and a commercial
            // sidewalk further out (the model draws neither the parking lanes
            // nor the surveyed widths), plus the slack.
            let reach = self.dims.frontage_zone_m + self.dims.throughway_m + self.dims.city_lane_m + self.dims.sidewalk_commercial_m + FRONT_SLACK_M;
            build.footway_activity = build.footways.iter().map(|f| fr.activity(&f.points, [f.normal[0] * f.road_side as f64, f.normal[1] * f.road_side as f64], reach)).collect();
        }
        for (k, place) in path_place.into_iter().enumerate() {
            build.footway_activity[first_path + k].place = place;
        }
        if let Some(routes) = self.active_surveyed_routes() {
            routes.apply(&mut build, &want);
        }
        if !build.lane_surfaces.is_empty() || !build.lane_junctions.is_empty() {
            build.surface_clearance_m = Some(self.dims.surface_clearance_m);
        }
        build
    }
}

/// Load the extract and the standards, ready to build tiles.
pub fn load(paths: &eo_data::DataPaths) -> Result<NativeSource> {
    let standards = crate::standards::Standards::load(paths)?;
    let mut dims = StreetDims::from_standards(&standards)?;
    // groundscale (the street size, 1.25 by default): the class street model's lane and
    // footway widths follow the scaled ground plan the renderer draws; the
    // standards themselves, the crosswalk and the limit line are not touched.
    let gs = eo_data::groundscale::scale();
    if gs != 1.0 {
        for w in [&mut dims.city_lane_m, &mut dims.state_lane_m, &mut dims.state_urban_lane_m, &mut dims.sidewalk_commercial_m, &mut dims.sidewalk_residential_m, &mut dims.sidewalk_alley_m, &mut dims.sidewalk_industrial_m, &mut dims.frontage_zone_m, &mut dims.throughway_m] {
            *w *= gs;
        }
    }
    let timing = SignalTiming::from_standards(&standards)?;
    let stops = match TrafficStop::load() {
        Ok(stops) => stops,
        Err(e) => {
            static WARNED: std::sync::Once = std::sync::Once::new();
            WARNED.call_once(|| {
                eprintln!("eo-sim: sf-traffic-stops.json unavailable ({e:#}); continuing with sf-city.json.gz roads and signals only");
            });
            Vec::new()
        }
    };
    Ok(NativeSource::new(SimSource::load(paths)?, dims, timing).with_stops(stops))
}

/// Keep the resident network to the tiles around the world's foci: build the
/// missing tiles within `radius` of any region's focus, drop tiles farther
/// than `radius + TILE_M` from every focus (hysteresis, so a focus pacing a
/// tile seam does not thrash). Regions are told after each change, removal
/// first, so no agent ever reads a slot that was reused. Returns (added, removed).
pub fn stream(world: &mut crate::World, source: &NativeSource, radius: f64) -> (usize, usize) {
    let foci: Vec<[f64; 2]> = world.regions.iter().map(|r| [r.focus.x, r.focus.z]).collect();
    let mut wanted: Vec<TileKey> = foci.iter().flat_map(|f| source.resident_tiles(f[0], f[1], radius)).collect();
    wanted.sort();
    wanted.dedup();
    let keep: std::collections::HashSet<_> =
        foci.iter().flat_map(|f| source.resident_tiles(f[0], f[1], radius + TILE_M)).collect();
    let mut drop: Vec<TileKey> = world.net.tiles.keys().copied().filter(|t| !keep.contains(t)).collect();
    drop.sort();
    for t in &drop {
        world.net.remove_tile(*t);
    }
    if !drop.is_empty() {
        for r in world.regions.iter_mut() {
            r.network_changed(&world.net);
        }
    }
    let add: Vec<TileKey> = wanted.into_iter().filter(|t| !world.net.tiles.contains_key(t)).collect();
    if !add.is_empty() {
        world.net.add(source.build(&add));
        for r in world.regions.iter_mut() {
            r.network_changed(&world.net);
        }
    }
    (add.len(), drop.len())
}

/// Tiles within `radius` of a point.
pub fn tiles_around(x: f64, z: f64, radius: f64) -> Vec<TileKey> {
    let lo = TileKey::of(x - radius, z - radius);
    let hi = TileKey::of(x + radius, z + radius);
    let mut out = vec![];
    for tx in lo.x..=hi.x {
        for tz in lo.z..=hi.z {
            out.push(TileKey { x: tx, z: tz });
        }
    }
    out
}

/// One directed run of a named road for the corridor layer (`corridor`): its
/// centreline in travel order, the lanes it carries that way, their width,
/// the posted speed (the survey's `maxspeed`), and per point whether the way
/// after it is a tunnel or a bridge.
#[derive(Clone, Debug)]
pub struct Chain {
    pub points: Vec<P2>,
    pub lanes: u32,
    pub lane_w: f64,
    pub speed_mps: Option<f64>,
    pub tunnel: Vec<bool>,
    pub bridge: Vec<bool>,
    pub names: Vec<String>,
}

impl NativeSource {
    /// Drop the roads `pred` picks (the corridor layer drives them instead), so
    /// the local simulation never places a car on them.
    pub fn without_roads(&mut self, pred: impl Fn(&SimRoad) -> bool) -> usize {
        let before = self.roads.len();
        self.roads.retain(|r| !pred(r));
        self.index_lane_roads();
        before - self.roads.len()
    }

    /// The directed runs of the vehicle ways named in `names`: every way split
    /// into its directions of travel (one-way tags respected), joined end to
    /// start where they share a vertex, the straightest continuation first;
    /// each run as long as the joins allow. Runs shorter than `min_len_m` are left out.
    pub fn chains(&self, names: &[&str], min_len_m: f64) -> Vec<Chain> {
        struct Edge {
            road: usize,
            pts: Vec<P2>,
            lanes: u32,
        }
        let mut edges: Vec<Edge> = vec![];
        for (ri, r) in self.roads.iter().enumerate() {
            if !names.iter().any(|n| r.name == *n) || r.points.len() < 2 {
                continue;
            }
            let n = r.lanes.unwrap_or(if r.oneway == Oneway::Both { 2 } else { 1 }).max(1);
            let (fwd, back) = match r.oneway {
                Oneway::Forward => (n, 0),
                Oneway::Reverse => (0, n),
                Oneway::Both => ((n + 1) / 2, (n / 2).max(1)),
            };
            if fwd > 0 {
                edges.push(Edge { road: ri, pts: r.points.clone(), lanes: fwd });
            }
            if back > 0 {
                edges.push(Edge { road: ri, pts: r.points.iter().rev().copied().collect(), lanes: back });
            }
        }
        let mut starts: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        let mut ends: HashMap<(i64, i64), usize> = HashMap::new();
        for (i, e) in edges.iter().enumerate() {
            starts.entry(qkey(e.pts[0])).or_default().push(i);
            *ends.entry(qkey(e.pts[e.pts.len() - 1])).or_default() += 1;
        }
        let dir = |a: P2, b: P2| {
            let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
            let l = dx.hypot(dz).max(1e-9);
            [dx / l, dz / l]
        };
        let mut used = vec![false; edges.len()];
        // sources first (no way arrives at their start), then whatever is left (loops).
        let mut order: Vec<usize> = (0..edges.len()).filter(|&i| !ends.contains_key(&qkey(edges[i].pts[0]))).collect();
        order.extend(0..edges.len());
        let mut out = vec![];
        for first in order {
            if used[first] {
                continue;
            }
            let mut cur = first;
            let mut points: Vec<P2> = vec![];
            let mut tunnel = vec![];
            let mut bridge = vec![];
            let mut lanes = vec![];
            let mut speeds = vec![];
            let mut road_names: Vec<String> = vec![];
            loop {
                used[cur] = true;
                let e = &edges[cur];
                let r = &self.roads[e.road];
                let skip = if points.is_empty() { 0 } else { 1 };
                for p in &e.pts[skip..] {
                    points.push(*p);
                    tunnel.push(r.tunnel);
                    bridge.push(r.bridge);
                }
                let len: f64 = e.pts.windows(2).map(|w| geom::hypot(w[1][0] - w[0][0], w[1][1] - w[0][1])).sum();
                lanes.push((e.lanes, len));
                if let Some(v) = geom::parse_posted_speed_mps(&r.maxspeed) {
                    speeds.push((v, len));
                }
                if !road_names.contains(&r.name) {
                    road_names.push(r.name.clone());
                }
                let n = e.pts.len();
                let d = dir(e.pts[n - 2], e.pts[n - 1]);
                let next = starts.get(&qkey(e.pts[n - 1])).and_then(|c| {
                    c.iter().copied().filter(|&k| !used[k]).max_by(|&a, &b| {
                        let da = dir(edges[a].pts[0], edges[a].pts[1]);
                        let db = dir(edges[b].pts[0], edges[b].pts[1]);
                        (d[0] * da[0] + d[1] * da[1]).total_cmp(&(d[0] * db[0] + d[1] * db[1]))
                    })
                });
                // no U-turns onto the other direction of the same street.
                match next {
                    Some(k) if {
                        let dk = dir(edges[k].pts[0], edges[k].pts[1]);
                        d[0] * dk[0] + d[1] * dk[1] > -0.5
                    } =>
                    {
                        cur = k
                    }
                    _ => break,
                }
            }
            let total: f64 = points.windows(2).map(|w| geom::hypot(w[1][0] - w[0][0], w[1][1] - w[0][1])).sum();
            if total < min_len_m {
                continue;
            }
            // the lane count carried over most of the run; the speed as posted over most of it.
            let mode = |v: &[(f64, f64)]| v.iter().fold(None::<(f64, f64)>, |m, &(x, l)| {
                let w: f64 = v.iter().filter(|(y, _)| *y == x).map(|(_, l)| l).sum();
                let _ = l;
                if m.is_none_or(|(_, mw)| w > mw) { Some((x, w)) } else { m }
            });
            let lanes_f: Vec<(f64, f64)> = lanes.iter().map(|&(n, l)| (n as f64, l)).collect();
            let n = mode(&lanes_f).map(|(x, _)| x as u32).unwrap_or(1);
            let first_road = &self.roads[edges[first].road];
            out.push(Chain { points, lanes: n, lane_w: self.lane_width(first_road), speed_mps: mode(&speeds).map(|(x, _)| x), tunnel, bridge, names: road_names });
        }
        out
    }
}
