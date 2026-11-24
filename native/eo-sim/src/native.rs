//! The network built natively from the extract (`sf-city.json.gz` via
//! `eo_data::sim_source`), one delivery of 256 m tiles at a time - the path
//! that scales past the JS window to any tile of the city (and, with a tile
//! store in place of the one source file, of the Earth).
//!
//! WHAT IT BUILDS, AND FROM WHICH RULES
//!
//!   lanes      Every vehicle way split at the vertices it shares with another
//!              way. Lane count from the `lanes` tag (one each way, or one, when
//!              untagged); right-hand traffic; lane width the class model the
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
//! bays, centre turn lanes), painted lane-use arrows, the drawn junction pads
//! and corner fillets the JS stop bars and kerb trims are measured on, and the
//! at-grade junction normalisation of the source connectivity. Those belong to
//! the streets worker's geometry; when it lands, this builder should read its
//! cross sections instead of the class model.

use std::collections::HashMap;

use anyhow::Result;
use eo_data::sim_source::{Oneway, SimRoad, SimSource};

use crate::geom::{self, LaneRole, P2};
use crate::network::*;
use crate::signals::{AxisInput, JunctionInput, MemberInput, cluster_axes};
use crate::standards::{SignalTiming, StreetDims};

/// `SIGNAL_TIMING.JUNCTION_NODE_MAX_M`, `SPLIT_JUNCTION_M`, `APPROACH_MIN_CLOSURE_M`.
const JUNCTION_NODE_MAX_M: f64 = 25.0;
const SPLIT_JUNCTION_M: f64 = 20.0;
const APPROACH_MIN_CLOSURE_M: f64 = 5.5;

pub struct NativeSource {
    roads: Vec<SimRoad>,
    signals: Vec<P2>,
    road_tiles: HashMap<TileKey, Vec<u32>>,
    signal_tiles: HashMap<TileKey, Vec<u32>>,
    pub dims: StreetDims,
    pub timing: SignalTiming,
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
            }
        }
        let n = n as f64;
        crate::consts::FootActivity { shops: (shops as f64 / n) as f32, night: (night as f64 / n) as f32, storeys: (storeys / n) as f32, place: 0.0 }
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
    half: f64,
    lane_w: f64,
    forward: u32,
    backward: u32,
}

impl NativeSource {
    pub fn new(source: SimSource, dims: StreetDims, timing: SignalTiming) -> Self {
        let roads: Vec<SimRoad> =
            source.roads.into_iter().filter(|r| vehicle_way(&r.highway) && r.points.len() >= 2).collect();
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
        NativeSource { roads, signals: source.signals, road_tiles, signal_tiles, dims, timing, levels: HashMap::new(), frontage: None, paths: vec![], path_tiles: HashMap::new() }
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
        self
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
        // Vertex degree over those roads.
        let mut degree: HashMap<(i64, i64), u16> = HashMap::new();
        for &r in &road_ids {
            let pts = &self.roads[r as usize].points;
            let mut seen: Vec<(i64, i64)> = pts.iter().map(|p| qkey(*p)).collect();
            seen.sort_unstable();
            seen.dedup();
            for k in seen {
                *degree.entry(k).or_default() += 1;
            }
        }
        // Split at shared vertices.
        let mut segs: Vec<Seg> = vec![];
        for &r in &road_ids {
            let road = &self.roads[r as usize];
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
            for i in 1..road.points.len() {
                let last = i == road.points.len() - 1;
                if last || degree.get(&qkey(road.points[i])).copied().unwrap_or(0) >= 2 {
                    let pts: Vec<P2> = road.points[start..=i].to_vec();
                    let len: f64 = pts.windows(2).map(|w| geom::hypot(w[1][0] - w[0][0], w[1][1] - w[0][1])).sum();
                    if len > 0.5 {
                        segs.push(Seg { road: r, key: format!("n{}:{}", road.id, k), pts, half, lane_w, forward, backward });
                        k += 1;
                    }
                    start = i;
                }
            }
        }
        let mut build = NetworkBuild { tiles: tiles.to_vec(), merge_max_lateral_m: self.dims.city_lane_m, ..Default::default() };
        // Lanes, kept when their end node lies in the delivered tiles.
        struct LaneMeta {
            seg: usize,
            end: P2,
        }
        let mut metas: Vec<LaneMeta> = vec![];
        for (si, s) in segs.iter().enumerate() {
            let road = &self.roads[s.road as usize];
            for (dir_decreasing, count) in [(false, s.forward), (true, s.backward)] {
                for k in 0..count {
                    let offset = if s.forward > 0 && s.backward > 0 {
                        (k as f64 + 0.5) * s.lane_w * if dir_decreasing { 1.0 } else { -1.0 }
                    } else {
                        // One-way: lanes spread about the centreline, rightmost first.
                        let centred = (k as f64 - (count as f64 - 1.0) / 2.0) * s.lane_w;
                        if dir_decreasing { centred } else { -centred }
                    };
                    let points = geom::junction_lane_points(&s.pts, offset, dir_decreasing);
                    let (start_node, end_node) = if dir_decreasing {
                        (s.pts[s.pts.len() - 1], s.pts[0])
                    } else {
                        (s.pts[0], s.pts[s.pts.len() - 1])
                    };
                    let tile = TileKey::of(end_node[0], end_node[1]);
                    if !want.contains(&tile) {
                        continue;
                    }
                    build.lanes.push(LaneBuild {
                        key: format!("{}-{}-{}", s.key, if dir_decreasing { "reverse" } else { "forward" }, k + 1),
                        segment: s.key.clone(),
                        street: road.name.clone(),
                        highway: road.highway.clone(),
                        role: LaneRole::Through,
                        painted_through_mapped: false,
                        width: s.lane_w,
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
                    metas.push(LaneMeta { seg: si, end: end_node });
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
        let mut node_half: HashMap<(i64, i64), Vec<(usize, f64)>> = HashMap::new();
        for (si, s) in segs.iter().enumerate() {
            node_half.entry(qkey(s.pts[0])).or_default().push((si, s.half));
            node_half.entry(qkey(s.pts[s.pts.len() - 1])).or_default().push((si, s.half));
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
            if !(road.bridge || road.tunnel) {
                build.segments.push(SegmentBuild { half: s.half, points: s.pts.clone(), level });
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
            let ends_info = |p: P2| -> (f64, bool) {
                let others = node_half.get(&qkey(p)).map(|v| v.iter().filter(|(k, _)| *k != si).map(|(_, h)| *h).fold(0.0f64, f64::max));
                let degree = node_half.get(&qkey(p)).map_or(1, |v| v.len());
                (others.unwrap_or(0.0), degree > 2)
            };
            let a = s.pts[0];
            let b = s.pts[s.pts.len() - 1];
            let (ha, ja) = ends_info(a);
            let (hb, jb) = ends_info(b);
            let base = build.footways.len() as u32;
            for (k, side) in [1i8, -1i8].iter().enumerate() {
                let offset = walk_line * *side as f64;
                let points = geom::junction_lane_points(&s.pts, offset, false);
                let d = [b[0] - a[0], b[1] - a[1]];
                let dl = geom::hypot(d[0], d[1]).max(1e-9);
                // Same normal convention as the lane offsets: +offset moves along (dz, -dx).
                let normal = [d[1] / dl, -d[0] / dl];
                let trim = |h: f64, junction: bool| if junction || h > 0.0 { h } else { 0.0 };
                let end = |h: f64, junction: bool| FootEndBuild {
                    kerb_trim: trim(h, junction),
                    crossing_arc: trim(h, junction) + self.dims.crosswalk_m / 2.0,
                };
                build.footways.push(FootwayBuild {
                    points,
                    highway: road.highway.clone(),
                    segment: s.key.clone(),
                    road_side: *side,
                    node_keys: Some([node_text(a), node_text(b)]),
                    normal,
                    cross_index: Some(base + 1 - k as u32),
                    ends: [end(ha, ja), end(hb, jb)],
                    tile: Some(seg_tile),
                });
            }
            for p in [a, b] {
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
            let (a, b) = (p.points[0], p.points[p.points.len() - 1]);
            let d = [b[0] - a[0], b[1] - a[1]];
            let dl = geom::hypot(d[0], d[1]);
            let len: f64 = p.points.windows(2).map(|w| geom::hypot(w[1][0] - w[0][0], w[1][1] - w[0][1])).sum();
            if len < 4.0 || dl < 1e-6 {
                continue;
            }
            let mid = p.points[p.points.len() / 2];
            build.footways.push(FootwayBuild {
                points: p.points.clone(),
                highway: p.highway.clone(),
                segment: format!("p{}", p.id),
                road_side: 1,
                node_keys: Some([node_text(a), node_text(b)]),
                normal: [d[1] / dl, -d[0] / dl],
                cross_index: None,
                ends: [FootEndBuild { kerb_trim: 0.0, crossing_arc: 0.0 }, FootEndBuild { kerb_trim: 0.0, crossing_arc: 0.0 }],
                tile: Some(TileKey::of(mid[0], mid[1])),
            });
            path_place.push(if p.highway == "pedestrian" { crate::consts::PLACE_PEDESTRIAN } else { crate::consts::PLACE_FOOTPATH });
            for q in [a, b] {
                let key = node_text(q);
                foot_nodes.entry(key.clone()).or_insert(FootNodeBuild { key, pos: q, signal: None });
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
    Ok(NativeSource::new(SimSource::load(paths)?, dims, timing))
}

/// Keep the resident network to the tiles around the world's foci: build the
/// missing tiles within `radius` of any region's focus, drop tiles farther
/// than `radius + TILE_M` from every focus (hysteresis, so a focus pacing a
/// tile seam does not thrash). Regions are told after each change, removal
/// first, so no agent ever reads a slot that was reused. Returns (added, removed).
pub fn stream(world: &mut crate::World, source: &NativeSource, radius: f64) -> (usize, usize) {
    let foci: Vec<[f64; 2]> = world.regions.iter().map(|r| [r.focus.x, r.focus.z]).collect();
    let mut wanted: Vec<TileKey> = foci.iter().flat_map(|f| tiles_around(f[0], f[1], radius)).collect();
    wanted.sort();
    wanted.dedup();
    let keep = radius + TILE_M;
    let near = |t: &TileKey| {
        let o = t.origin();
        let c = [o[0] + TILE_M / 2.0, o[1] + TILE_M / 2.0];
        foci.iter().any(|f| (f[0] - c[0]).abs() <= keep + TILE_M / 2.0 && (f[1] - c[1]).abs() <= keep + TILE_M / 2.0)
    };
    let mut drop: Vec<TileKey> = world.net.tiles.keys().copied().filter(|t| !near(t)).collect();
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
