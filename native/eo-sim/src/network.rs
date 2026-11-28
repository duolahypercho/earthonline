//! The street network the simulation drives and walks on, keyed by tile.
//!
//! WHAT IS IN IT. Directed lanes (one per moving lane, right-hand traffic,
//! offset through the node as `junctionLanePoints` draws them), the links
//! between them with each link's movement class, legality, route weight and
//! turning plan; per lane the junction hold (`junctionHoldFor`); the signal
//! junctions; footway paths with their street nodes, kerb trims, crossing
//! stations and far kerb; and the street carriageways (for the
//! on-carriageway measure).
//!
//! TILES, NOT A CITY. Nothing here is sized to a city. Content arrives as
//! `NetworkBuild`s - one or more 256 m tiles at a time, from the JS window
//! oracle (`import_js.rs`) or built natively from the extract
//! (`native.rs`) - and leaves a tile at a time (`remove_tile`). Lanes,
//! nodes, footways, footway nodes and junctions live in slabs with free
//! lists, so a long-running world that streams the whole Earth past its
//! players holds only what is resident. Links are resolved on node identity
//! the way `traffic-graph-store.js` resolves them (successors re-derived for
//! every lane ending at a touched node, sorted by id so arrival order never
//! changes a route), and a removed lane is tombstoned (`alive == false`) so
//! an agent riding it is respawned by its region before the slot is reused.

use std::collections::HashMap;
use std::sync::Arc;

use crate::consts::{APPROACH_COMFORT_DECEL, TURN_LATERAL_ACCEL, TURN_RADIUS_MIN};
use crate::geom::{self, LaneRole, Movement, P2, Plan, SiblingExit};
use crate::signals::{JunctionInput, SignalJunction};
use crate::standards::SignalTiming;

mod surface;
pub use surface::JunctionSurface;
use surface::LaneJunction;

/// Tile edge, metres. Matches the renderer's 256 m terrain/road tiles.
pub const TILE_M: f64 = 256.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey {
    pub x: i32,
    pub z: i32,
}

impl TileKey {
    pub fn of(x: f64, z: f64) -> Self {
        TileKey { x: (x / TILE_M).floor() as i32, z: (z / TILE_M).floor() as i32 }
    }
    pub fn origin(self) -> P2 {
        [self.x as f64 * TILE_M, self.z as f64 * TILE_M]
    }
}

pub type LaneId = u32;
pub type FootwayId = u32;
pub type NodeId = u32;
pub type JunctionId = u32;
pub const NONE: u32 = u32::MAX;

/// Quantised node identity (0.25 m lattice, the JS connection tolerance).
pub type NodeKey = (i64, i64);
pub const NODE_TOLERANCE_M: f64 = 0.25;

pub fn node_key(p: P2) -> NodeKey {
    ((p[0] / NODE_TOLERANCE_M).floor() as i64, (p[1] / NODE_TOLERANCE_M).floor() as i64)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ControlKind {
    None,
    Stop { enforced: bool, all_way: bool, hold_s: f64 },
    MappedStop { id: u64, all_way: bool, stop_offset_m: f64, bar_depth_m: f64 },
    Signal { junction: JunctionId, axis: u8, stop_offset_m: f64, bar_depth_m: f64 },
}

// ---------------------------------------------------------------- build input

#[derive(Clone, Debug)]
pub struct LaneBuild {
    pub key: String,
    pub segment: String,
    pub street: String,
    pub highway: String,
    pub role: LaneRole,
    pub painted_through_mapped: bool,
    pub width: f64,
    /// Driven polyline (offset through the node).
    pub points: Vec<P2>,
    /// Topology: the centreline node this lane leaves and the one it reaches.
    pub start_node: P2,
    pub end_node: P2,
    pub max_speed: f64,
    pub control: ControlKind,
    /// Explicit successors (indices into this build's lanes); `None` resolves by node.
    pub outgoing: Option<Vec<u32>>,
    /// Explicit movement legality per explicit successor (from the JS contract).
    pub legal: Option<Vec<bool>>,
    /// The tile that delivered this lane (None: the tile of its midpoint).
    pub tile: Option<TileKey>,
    /// Structure level of the lane: 0 at grade, 1 the first level above,
    /// negative in a tunnel (`NativeSource::with_levels`). Carried to
    /// `Lane::level`: bodies on different levels never block or see each
    /// other (`Fleet::sense`, `insert_blocked`, the walkers' traffic gap).
    pub level: i8,
}

#[derive(Clone, Debug)]
pub struct FootEndBuild {
    pub kerb_trim: f64,
    pub crossing_arc: f64,
    /// (`--crowd`, `NativeSource::with_crossings`) no crosswalk is mapped at
    /// this end's crossing band: the street is crossed here only on a signal.
    /// False (crossed as the JS crosses) unless the crosswalks are known.
    pub no_crossing: bool,
    /// (`--people-crosswalks`, `NativeSource::with_zebras`) the band is the
    /// drawn zebra's, and `no_crossing` (no zebra drawn across this end)
    /// holds at a signal too. False as before.
    pub zebra_only: bool,
}

#[derive(Clone, Debug)]
pub struct FootwayBuild {
    pub points: Vec<P2>,
    pub highway: String,
    pub segment: String,
    pub road_side: i8,
    /// Street node each end hangs off (key text as the JS contract writes it).
    pub node_keys: Option<[String; 2]>,
    pub normal: P2,
    /// Far kerb at each end (index into this build's footways).
    pub cross_index: [Option<u32>; 2],
    /// Start station on the original sidewalk, for a clipped fragment.
    pub original_arc_start: Option<f64>,
    pub ends: [FootEndBuild; 2],
    /// The tile that delivered this footway (None: the tile of its midpoint).
    pub tile: Option<TileKey>,
}

#[derive(Clone, Debug)]
pub struct FootNodeBuild {
    pub key: String,
    pub pos: P2,
    /// Index into this build's signal junctions.
    pub signal: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct SegmentBuild {
    pub half: f64,
    pub points: Vec<P2>,
    /// Structure level of the carriageway (`LaneBuild::level`): 0 at grade.
    pub level: i8,
}

/// A broken white divider between adjacent through lanes, clear of junctions.
#[derive(Clone, Copy, Debug)]
pub struct LanePass {
    pub to: LaneId,
    pub span: [f64; 2],
}

/// Maximum length and width of a centred chassis whose swept envelope fits a lane.
#[derive(Clone, Copy, Debug)]
pub struct VehicleEnvelope {
    length: f64,
    width: f64,
}

impl VehicleEnvelope {
    pub fn new(length: f64, width: f64) -> Option<Self> {
        (length.is_finite() && length > 0.0 && width.is_finite() && width > 0.0)
            .then_some(Self { length, width })
    }
}

/// One delivery of network content: the lanes, footways, junctions and carriageways of some tiles.
#[derive(Clone, Debug, Default)]
pub struct NetworkBuild {
    pub tiles: Vec<TileKey>,
    pub lanes: Vec<LaneBuild>,
    /// Source lane index and its passing lane index within this delivery.
    pub passing: Vec<(LaneId, LanePass)>,
    /// Measured envelopes for lanes with a verified chassis restriction, indexed in this delivery.
    pub vehicle_envelopes: Vec<(LaneId, VehicleEnvelope)>,
    pub footways: Vec<FootwayBuild>,
    pub foot_nodes: Vec<FootNodeBuild>,
    pub junctions: Vec<JunctionInput>,
    pub segments: Vec<SegmentBuild>,
    /// Widest sideways step a through lane's one continuation may take as a merge
    /// (`THROUGH_MERGE_MAX_LATERAL_M` = caMetric('travel-lane-clearance-at-bulbout-auto-sf')).
    pub merge_max_lateral_m: f64,
    /// Freeway following guidance from the native street standards.
    pub following_time_gap_s: Option<f64>,
    /// Native lane bodies turn progressively through internal polyline vertices.
    pub continuous_lane_heading: bool,
    /// Preview geometry-based braking in native lane bends.
    pub bend_braking: bool,
    /// The paved preview keeps corner continuations on their own corner.
    pub contain_walk_corners: bool,
    /// Lane index and its drawn carriageway heights [arc, y].
    pub lane_surfaces: Vec<(LaneId, Vec<[f64; 2]>)>,
    /// Lane intervals with asphalt from the road's own strip, even across a pad.
    pub lane_carriageway_spans: Vec<(LaneId, Vec<[f64; 2]>)>,
    /// Drawn junction surfaces attached to the start and end of a delivered lane.
    pub lane_junctions: Vec<(LaneId, [Option<Arc<JunctionSurface>>; 2])>,
    pub surface_clearance_m: Option<f64>,
    /// What fronts each of `footways` (`--crowd`; `NativeSource::with_frontage`):
    /// empty (every footway's activity zero, the network as before) without a survey.
    pub footway_activity: Vec<crate::consts::FootActivity>,
}

// ---------------------------------------------------------------- runtime

#[derive(Clone, Debug)]
pub struct Link {
    pub to: LaneId,
    pub movement: Movement,
    pub legal: bool,
    pub weight: f64,
    pub drivable: bool,
    pub plan: Option<Plan>,
}

#[derive(Clone, Debug)]
pub struct Lane {
    pub alive: bool,
    pub key: String,
    pub tile: TileKey,
    pub segment: u32,
    pub street: u32,
    pub highway: String,
    pub priority: i32,
    pub role: LaneRole,
    pub painted_through_mapped: bool,
    pub width: f64,
    pub pts: Vec<P2>,
    pub cum: Vec<f64>,
    pub length: f64,
    pub start_node: NodeId,
    pub end_node: NodeId,
    pub max_speed: f64,
    pub control: ControlKind,
    pub links: Vec<Link>,
    /// Explicit successor keys from the source contract (kept so a relink honours them).
    explicit: Option<Vec<(String, Option<bool>)>>,
    /// No drivable successor: a dead end.
    pub terminus: bool,
    /// `junctionHoldFor(edge, car)` per presented kind (sedan, taxi, truck, bus):
    /// how far back from the lane's end a body must stand to stay clear of
    /// every movement's swept path through the node.
    pub hold: [f64; 4],
    pub passing: Option<LanePass>,
    /// Structure level (`LaneBuild::level`): 0 at grade.
    pub level: i8,
    continuous_heading: bool,
    vehicle_envelope: Option<VehicleEnvelope>,
    bends: Vec<LaneBend>,
    surface: Vec<[f64; 2]>,
    carriageway_spans: Box<[[f64; 2]]>,
    junction_pads: [Option<LaneJunction>; 2],
}

#[derive(Clone, Debug)]
struct LaneBend {
    arc: f64,
    tan_half_turn: f64,
}

impl Lane {
    pub fn fits_vehicle(&self, length: f64, width: f64) -> bool {
        self.vehicle_envelope
            .is_none_or(|envelope| length <= envelope.length && width <= envelope.width)
    }

    pub fn surface_at(&self, arc: f64) -> Option<f64> {
        let fallback = self.road_surface_at(arc);
        let arc = arc.clamp(0.0, self.length);
        if self.on_carriageway(arc) {
            return fallback;
        }
        let mut pads = self.junction_pads.iter().flatten()
            .filter(|p| p.contains(arc)).peekable();
        if pads.peek().is_none() {
            return fallback;
        }
        let (x, z, ..) = geom::at_arc(&self.pts, &self.cum, arc);
        let reference = fallback.unwrap_or_default();
        pads.filter_map(|p| p.pad.height_near([x, z], reference))
            .min_by(|a, b| (a - reference).abs().total_cmp(&(b - reference).abs()))
            .or(fallback)
    }

    pub(crate) fn junction_to(&self, exit: &Self) -> Option<&JunctionSurface> {
        let a = &self.junction_pads[1].as_ref()?.pad;
        let b = &exit.junction_pads[0].as_ref()?.pad;
        Arc::ptr_eq(a, b).then_some(a.as_ref())
    }

    fn on_carriageway(&self, arc: f64) -> bool {
        self.carriageway_spans.iter().any(|&[lo, hi]| (lo..=hi).contains(&arc))
    }

    fn stitch_junction_grades(&mut self) {
        if self.surface.is_empty() {
            return;
        }
        // Approach profiles extend past their trimmed mouths. Replace the
        // projected samples inside the pad and meet its actual edge height.
        let mut seams = Vec::new();
        for junction in self.junction_pads.iter().flatten() {
            for &arc in junction.intervals.iter().flatten() {
                if self.on_carriageway(arc) { continue; }
                let (x, z, ..) = geom::at_arc(&self.pts, &self.cum, arc);
                let reference = self.road_surface_at(arc).unwrap_or_default();
                if let Some(y) = junction.pad.height_near([x, z], reference) {
                    seams.push([arc, y]);
                }
            }
        }
        if seams.is_empty() {
            return;
        }
        let carriageway = &self.carriageway_spans;
        self.surface.retain(|sample| {
            carriageway.iter().any(|&[lo, hi]| (lo..=hi).contains(&sample[0]))
                || !self.junction_pads.iter().flatten().any(|pad| pad.contains(sample[0]))
        });
        self.surface.extend(seams);
        self.surface.sort_by(|a, b| a[0].total_cmp(&b[0]));
        self.surface.dedup_by(|a, b| a[0] == b[0]);
    }

    fn road_surface_at(&self, arc: f64) -> Option<f64> {
        let first = *self.surface.first()?;
        let i = self.surface.partition_point(|p| p[0] <= arc);
        if i == 0 {
            return Some(first[1]);
        }
        let a = self.surface[i - 1];
        let Some(&b) = self.surface.get(i) else { return Some(a[1]) };
        Some(a[1] + (b[1] - a[1]) * ((arc - a[0]) / (b[0] - a[0])))
    }

    pub(crate) fn bend_within(&self, arc: f64, look: f64, length: f64) -> bool {
        let start = self.bends.partition_point(|bend| bend.arc < arc - length / 2.0);
        self.bends.get(start).is_some_and(|bend| bend.arc <= arc + look + length / 2.0)
    }

    /// Speed envelope for bends on the lane itself. Start braking before the
    /// body enters the same heading blend used to present it and test clearance.
    pub(crate) fn bend_speed_target(&self, arc: f64, length: f64, cruise: f64) -> f64 {
        if self.bends.is_empty() {
            return cruise;
        }
        let half = length / 2.0;
        let cruise_sq = cruise * cruise;
        let min_bend_speed_sq = TURN_LATERAL_ACCEL * TURN_RADIUS_MIN;
        let brake = 2.0 * APPROACH_COMFORT_DECEL;
        let end = arc + half + cruise_sq / brake;
        let start = self.bends.partition_point(|bend| bend.arc < arc - half);
        let mut target_sq = cruise_sq;
        for bend in self.bends[start..].iter().take_while(|bend| bend.arc <= end) {
            let span = half.min(bend.arc).min(self.length - bend.arc);
            // The footprint chord gives an effective radius that grows away
            // from the vertex. A polyline vertex lacks a surveyed curve
            // radius, so keep the turn model's nominal minimum at the apex.
            let radius = (span / bend.tan_half_turn).max(f64::EPSILON);
            let offset = arc - bend.arc;
            let floor_offset = ((TURN_RADIUS_MIN - radius).max(0.0) * radius).sqrt();
            let braking_offset = APPROACH_COMFORT_DECEL * radius / TURN_LATERAL_ACCEL;
            let sample = offset.max(-floor_offset.max(braking_offset).min(half));
            let local_radius = radius + sample * sample / radius;
            let limit_sq = (TURN_LATERAL_ACCEL * local_radius).max(min_bend_speed_sq);
            target_sq = target_sq.min(limit_sq + brake * (sample - offset));
        }
        if target_sq < cruise_sq { target_sq.sqrt() } else { cruise }
    }

    /// Keep the centre on the lane and orient the body along its footprint.
    /// The span tapers to the lane tangent at junction joins.
    pub(crate) fn vehicle_pose(&self, arc: f64, length: f64) -> (f64, f64, f64) {
        let (x, z, dx, dz, piece) = geom::at_arc(&self.pts, &self.cum, arc);
        let span = (length / 2.0).min(arc).min(self.length - arc);
        if span > 0.0
            && (arc - span < self.cum[piece] || arc + span > self.cum[piece + 1])
            && self.bend_within(arc, 0.0, length)
        {
            let (ax, az, ..) = geom::at_arc(&self.pts, &self.cum, arc - span);
            let (bx, bz, ..) = geom::at_arc(&self.pts, &self.cum, arc + span);
            if ax != bx || az != bz {
                return (x, z, (bx - ax).atan2(bz - az));
            }
        }
        (x, z, dx.atan2(dz))
    }
}

/// Continuous body pose through a link and onto its receiving lane.
pub(crate) struct MovementPose<'a> {
    exit: &'a Lane,
    plan: &'a Plan,
    length: f64,
    blend: f64,
    heading_delta: [f64; 2],
}

pub(crate) struct JunctionSample {
    pub(crate) body: (f64, f64, f64, f64, f64),
    pub(crate) distance: f64,
}

impl<'a> MovementPose<'a> {
    pub(crate) fn new(from: &Lane, exit: &'a Lane, plan: &'a Plan, length: f64) -> Self {
        let entry_arc = (from.length - plan.entry_setback).clamp(0.0, from.length);
        let exit_arc = plan.exit_arc.clamp(0.0, exit.length);
        let entry_delta = geom::wrap_pi(from.vehicle_pose(entry_arc, length).2 - plan.pose_at(0.0).2);
        let exit_delta = geom::wrap_pi(exit.vehicle_pose(exit_arc, length).2 - plan.pose_at(plan.arc_len).2);
        Self { exit, plan, length, blend: length.min(plan.arc_len) / 2.0, heading_delta: [entry_delta, exit_delta] }
    }

    pub(crate) fn at(&self, progress: f64) -> (f64, f64, f64) {
        if progress > self.plan.arc_len {
            let arc = self.plan.exit_arc + progress - self.plan.arc_len;
            return self.exit.vehicle_pose(arc.clamp(0.0, self.exit.length), self.length);
        }
        let (x, z, yaw) = self.plan.pose_at(progress);
        if self.blend <= f64::EPSILON {
            return (x, z, yaw);
        }
        // The path tangent and the chassis heading diverge when a bend is
        // already within half a body length of the link seam.
        let weight = |distance: f64| {
            let t = (1.0 - distance / self.blend).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        let heading = yaw + self.heading_delta[0] * weight(progress)
            + self.heading_delta[1] * weight(self.plan.arc_len - progress);
        (x, z, heading)
    }
}

#[derive(Clone, Debug)]
pub struct Node {
    pub key: NodeKey,
    pub pos: P2,
    pub incoming: Vec<LaneId>,
    pub outgoing: Vec<LaneId>,
    /// Every link through this node as (from lane, link index), for the box mutex.
    pub movements: Vec<(LaneId, u16)>,
    /// Conflict matrix over `movements`, one bitset row per movement.
    pub conflicts: Vec<u64>,
    pub words: usize,
}

#[derive(Clone, Debug)]
pub struct FootEnd {
    pub node: u32,
    pub kerb_trim: f64,
    pub crossing_arc: f64,
    /// `FootEndBuild::no_crossing`.
    pub no_crossing: bool,
    /// `FootEndBuild::zebra_only`.
    pub zebra_only: bool,
}

#[derive(Clone, Debug)]
pub struct Footway {
    pub alive: bool,
    pub tile: TileKey,
    pub pts: Vec<P2>,
    pub cum: Vec<f64>,
    pub length: f64,
    pub weight: f64,
    pub segment: u32,
    pub road_side: i8,
    pub normal: P2,
    pub cross: [FootwayId; 2],
    pub original_arc_start: Option<f64>,
    pub ends: [FootEnd; 2],
    pub routable: bool,
    /// What fronts it (`--crowd`; zero without a frontage survey).
    pub activity: crate::consts::FootActivity,
}

#[derive(Clone, Debug)]
pub struct FootNode {
    pub key: String,
    pub pos: P2,
    pub ends: Vec<(FootwayId, u8)>,
    pub signal: JunctionId,
}

#[derive(Clone, Debug, Default)]
pub struct TileContent {
    pub lanes: Vec<LaneId>,
    pub footways: Vec<FootwayId>,
    /// Street carriageways (centreline + half-width) centred in this tile.
    pub segments: Vec<SegmentBuild>,
    pub junctions: Vec<JunctionId>,
}

pub struct Network {
    pub timing: SignalTiming,
    pub lanes: Vec<Lane>,
    free_lanes: Vec<LaneId>,
    lane_by_key: HashMap<String, LaneId>,
    pub nodes: Vec<Node>,
    free_nodes: Vec<NodeId>,
    node_by_key: HashMap<NodeKey, NodeId>,
    pub footways: Vec<Footway>,
    free_footways: Vec<FootwayId>,
    pub foot_nodes: Vec<FootNode>,
    free_foot_nodes: Vec<u32>,
    foot_node_by_key: HashMap<String, u32>,
    pub junctions: Vec<SignalJunction>,
    free_junctions: Vec<JunctionId>,
    interned: HashMap<String, u32>,
    pub tiles: HashMap<TileKey, TileContent>,
    pub merge_max_lateral_m: f64,
    pub following_time_gap_s: Option<f64>,
    pub(crate) bend_braking: bool,
    pub(crate) contain_walk_corners: bool,
    pub surface_clearance_m: Option<f64>,
    pub(crate) model_footprints: Option<(Arc<crate::vehicle_catalogue::ScaleReport>, Option<u32>)>,
    /// The longest signal cycle resident: a walker waits at least one whole cycle (`kerbWaitMaxS`).
    pub kerb_wait_max_s: f64,
}

impl Network {
    /// Are the two occupants close enough in height to share road space?
    /// An older lane build without surveyed heights still uses its levels.
    pub fn shares_surface(&self, (a_level, a_y): (i8, f64), (b_level, b_y): (i8, f64)) -> bool {
        match self.surface_clearance_m {
            Some(clearance) if a_y.is_finite() && b_y.is_finite() => (a_y - b_y).abs() < clearance,
            _ => a_level == b_level,
        }
    }

    pub fn new(timing: SignalTiming) -> Self {
        Network {
            timing,
            lanes: vec![],
            free_lanes: vec![],
            lane_by_key: HashMap::new(),
            nodes: vec![],
            free_nodes: vec![],
            node_by_key: HashMap::new(),
            footways: vec![],
            free_footways: vec![],
            foot_nodes: vec![],
            free_foot_nodes: vec![],
            foot_node_by_key: HashMap::new(),
            junctions: vec![],
            free_junctions: vec![],
            interned: HashMap::new(),
            tiles: HashMap::new(),
            merge_max_lateral_m: 0.0,
            following_time_gap_s: None,
            bend_braking: false,
            contain_walk_corners: false,
            surface_clearance_m: None,
            model_footprints: None,
            kerb_wait_max_s: crate::consts::KERB_WAIT_MAX_S,
        }
    }

    /// Use cooked model dimensions for the traffic this network admits.
    /// Install before creating its fleet.
    pub fn with_model_footprints(mut self, report: Arc<crate::vehicle_catalogue::ScaleReport>, selection_seed: Option<u32>) -> Self {
        self.model_footprints = Some((report, selection_seed));
        self
    }

    /// Model selected for this ID by the configured fleet asset report.
    pub fn driving_model(&self, seed: u32, id: u32, kind: u8) -> Option<&crate::vehicle_catalogue::Vehicle> {
        let (report, selection_seed) = self.model_footprints.as_ref()?;
        report.vehicle(selection_seed.unwrap_or(seed), id, kind)
    }

    fn intern(&mut self, s: &str) -> u32 {
        if let Some(&i) = self.interned.get(s) {
            return i;
        }
        let i = self.interned.len() as u32;
        self.interned.insert(s.to_string(), i);
        i
    }

    fn node_for(&mut self, p: P2) -> NodeId {
        let k = node_key(p);
        for dx in -1..=1 {
            for dz in -1..=1 {
                if let Some(&id) = self.node_by_key.get(&(k.0 + dx, k.1 + dz)) {
                    let n = &self.nodes[id as usize];
                    if geom::hypot(n.pos[0] - p[0], n.pos[1] - p[1]) <= NODE_TOLERANCE_M {
                        return id;
                    }
                }
            }
        }
        let node = Node { key: k, pos: p, incoming: vec![], outgoing: vec![], movements: vec![], conflicts: vec![], words: 0 };
        let id = if let Some(free) = self.free_nodes.pop() {
            self.nodes[free as usize] = node;
            free
        } else {
            self.nodes.push(node);
            (self.nodes.len() - 1) as NodeId
        };
        self.node_by_key.insert(k, id);
        id
    }

    fn foot_node_for(&mut self, key: &str, pos: P2) -> u32 {
        if let Some(&id) = self.foot_node_by_key.get(key) {
            return id;
        }
        let node = FootNode { key: key.to_string(), pos, ends: vec![], signal: NONE };
        let id = if let Some(free) = self.free_foot_nodes.pop() {
            self.foot_nodes[free as usize] = node;
            free
        } else {
            self.foot_nodes.push(node);
            (self.foot_nodes.len() - 1) as u32
        };
        self.foot_node_by_key.insert(key.to_string(), id);
        id
    }

    /// Resident counts: (lanes, footways, nodes, foot nodes, junctions, tiles).
    pub fn resident(&self) -> (usize, usize, usize, usize, usize, usize) {
        (
            self.lanes.iter().filter(|l| l.alive).count(),
            self.footways.iter().filter(|f| f.alive).count(),
            self.nodes.len() - self.free_nodes.len(),
            self.foot_nodes.len() - self.free_foot_nodes.len(),
            self.junctions.len() - self.free_junctions.len(),
            self.tiles.len(),
        )
    }

    /// Admit one delivery of tiles.
    pub fn add(&mut self, build: NetworkBuild) {
        self.contain_walk_corners |= build.contain_walk_corners;
        self.bend_braking |= build.bend_braking;
        self.surface_clearance_m = build.surface_clearance_m.or(self.surface_clearance_m);
        if build.merge_max_lateral_m > 0.0 {
            self.merge_max_lateral_m = build.merge_max_lateral_m;
        }
        if let Some(gap) = build.following_time_gap_s {
            self.following_time_gap_s = Some(gap);
        }
        for t in &build.tiles {
            self.tiles.entry(*t).or_default();
        }
        // Junctions first: lanes and footways refer to them.
        let mut junction_ids: Vec<JunctionId> = Vec::with_capacity(build.junctions.len());
        for input in build.junctions.iter() {
            let j = SignalJunction::build(input, &self.timing);
            if j.cycle_s > self.kerb_wait_max_s {
                self.kerb_wait_max_s = j.cycle_s;
            }
            let tile = input.tile.map_or_else(|| TileKey::of(j.centre[0], j.centre[1]), |(x, z)| TileKey { x, z });
            let id = if let Some(free) = self.free_junctions.pop() {
                self.junctions[free as usize] = j;
                free
            } else {
                self.junctions.push(j);
                (self.junctions.len() - 1) as JunctionId
            };
            self.tiles.entry(tile).or_default().junctions.push(id);
            junction_ids.push(id);
        }
        // Lanes.
        let mut ids: Vec<LaneId> = Vec::with_capacity(build.lanes.len());
        for lb in &build.lanes {
            let start = self.node_for(lb.start_node);
            let end = self.node_for(lb.end_node);
            let segment = self.intern(&lb.segment);
            let street = self.intern(&lb.street);
            let cum = geom::cumulative(&lb.points);
            let length = *cum.last().unwrap_or(&0.0);
            let bends = if build.continuous_lane_heading {
                lb.points.windows(3).zip(cum.iter().skip(1)).filter_map(|(p, &arc)| {
                    let (ax, az) = (p[1][0] - p[0][0], p[1][1] - p[0][1]);
                    let (bx, bz) = (p[2][0] - p[1][0], p[2][1] - p[1][1]);
                    let scale = ax.hypot(az) * bx.hypot(bz);
                    let cross = (ax * bz - az * bx).abs();
                    let dot = ax * bx + az * bz;
                    if scale == 0.0 || (cross <= 1e-6 * scale && dot >= 0.0) {
                        return None;
                    }
                    let tan_half_turn = if dot >= 0.0 {
                        cross / (scale + dot)
                    } else {
                        (scale - dot) / cross
                    };
                    Some(LaneBend { arc, tan_half_turn })
                }).collect()
            } else {
                Vec::new()
            };
            let mid = lb.points[lb.points.len() / 2];
            let tile = lb.tile.unwrap_or_else(|| TileKey::of(mid[0], mid[1]));
            let control = match lb.control {
                ControlKind::Signal { junction, axis, stop_offset_m, bar_depth_m } => ControlKind::Signal {
                    junction: junction_ids[junction as usize],
                    axis,
                    stop_offset_m,
                    bar_depth_m,
                },
                other => other,
            };
            let lane = Lane {
                alive: true,
                key: lb.key.clone(),
                tile,
                segment,
                street,
                highway: lb.highway.clone(),
                priority: geom::highway_priority(&lb.highway),
                role: lb.role,
                painted_through_mapped: lb.painted_through_mapped,
                width: lb.width,
                pts: lb.points.clone(),
                cum,
                length,
                start_node: start,
                end_node: end,
                max_speed: lb.max_speed,
                control,
                links: vec![],
                explicit: None,
                terminus: true,
                hold: [0.0; 4],
                passing: None,
                level: lb.level,
                continuous_heading: build.continuous_lane_heading,
                vehicle_envelope: None,
                bends,
                surface: Vec::new(),
                carriageway_spans: Box::default(),
                junction_pads: [None, None],
            };
            let id = if let Some(free) = self.free_lanes.pop() {
                self.lanes[free as usize] = lane;
                free
            } else {
                self.lanes.push(lane);
                (self.lanes.len() - 1) as LaneId
            };
            self.lane_by_key.insert(lb.key.clone(), id);
            self.tiles.entry(tile).or_default().lanes.push(id);
            ids.push(id);
        }
        for (k, lb) in build.lanes.iter().enumerate() {
            if let Some(out) = &lb.outgoing {
                let explicit = out
                    .iter()
                    .enumerate()
                    .map(|(i, &o)| (build.lanes[o as usize].key.clone(), lb.legal.as_ref().map(|l| l[i])))
                    .collect();
                self.lanes[ids[k] as usize].explicit = Some(explicit);
            }
        }
        for (from, pass) in &build.passing {
            self.lanes[ids[*from as usize] as usize].passing = Some(LanePass { to: ids[pass.to as usize], span: pass.span });
        }
        for (source, envelope) in build.vehicle_envelopes {
            self.lanes[ids[source as usize] as usize].vehicle_envelope = Some(envelope);
        }
        for (source, surface) in build.lane_surfaces {
            self.lanes[ids[source as usize] as usize].surface = surface;
        }
        for (source, spans) in build.lane_carriageway_spans {
            self.lanes[ids[source as usize] as usize].carriageway_spans = surface::merge_intervals(spans).into_boxed_slice();
        }
        for (source, pads) in build.lane_junctions {
            let lane = &mut self.lanes[ids[source as usize] as usize];
            lane.junction_pads = pads.map(|pad| pad.and_then(|p| LaneJunction::new(p, &lane.pts, &lane.cum, &lane.carriageway_spans)));
            lane.stitch_junction_grades();
        }
        // Node in/out lists for the new lanes, then relink everything touching them.
        let mut touched: Vec<NodeId> = vec![];
        for &id in &ids {
            let (s, e) = (self.lanes[id as usize].start_node, self.lanes[id as usize].end_node);
            self.nodes[s as usize].outgoing.push(id);
            self.nodes[e as usize].incoming.push(id);
            touched.push(s);
            touched.push(e);
        }
        // Footways.
        for fnb in &build.foot_nodes {
            let id = self.foot_node_for(&fnb.key, fnb.pos);
            if let Some(sig) = fnb.signal {
                self.foot_nodes[id as usize].signal = junction_ids[sig as usize];
            }
        }
        let mut foot_ids: Vec<FootwayId> = Vec::with_capacity(build.footways.len());
        for (k, fb) in build.footways.iter().enumerate() {
            let cum = geom::cumulative(&fb.points);
            let length = *cum.last().unwrap_or(&0.0);
            let mid = fb.points[fb.points.len() / 2];
            let tile = fb.tile.unwrap_or_else(|| TileKey::of(mid[0], mid[1]));
            let segment = self.intern(&fb.segment);
            let mut ends = [
                FootEnd { node: NONE, kerb_trim: fb.ends[0].kerb_trim, crossing_arc: fb.ends[0].crossing_arc, no_crossing: fb.ends[0].no_crossing, zebra_only: fb.ends[0].zebra_only },
                FootEnd { node: NONE, kerb_trim: fb.ends[1].kerb_trim, crossing_arc: fb.ends[1].crossing_arc, no_crossing: fb.ends[1].no_crossing, zebra_only: fb.ends[1].zebra_only },
            ];
            if let Some(keys) = &fb.node_keys {
                for (e, end) in ends.iter_mut().enumerate() {
                    // A footway whose node the build did not list: the node sits at its end.
                    let pos = fb.points[if e == 0 { 0 } else { fb.points.len() - 1 }];
                    end.node = self.foot_node_for(&keys[e], pos);
                }
            }
            let fw = Footway {
                alive: true,
                tile,
                pts: fb.points.clone(),
                cum,
                length,
                weight: crate::consts::sidewalk_class_weight(&fb.highway).max(0.01),
                segment,
                road_side: fb.road_side,
                normal: fb.normal,
                cross: [NONE; 2],
                original_arc_start: fb.original_arc_start,
                ends,
                routable: fb.node_keys.is_some(),
                activity: build.footway_activity.get(k).copied().unwrap_or_default(),
            };
            let id = if let Some(free) = self.free_footways.pop() {
                self.footways[free as usize] = fw;
                free
            } else {
                self.footways.push(fw);
                (self.footways.len() - 1) as FootwayId
            };
            foot_ids.push(id);
            self.tiles.entry(tile).or_default().footways.push(id);
        }
        for (k, fb) in build.footways.iter().enumerate() {
            let id = foot_ids[k];
            self.footways[id as usize].cross = fb.cross_index.map(|ci| {
                ci.map_or(NONE, |ci| foot_ids[ci as usize])
            });
            for e in 0..2u8 {
                let node = self.footways[id as usize].ends[e as usize].node;
                if node != NONE {
                    self.foot_nodes[node as usize].ends.push((id, e));
                }
            }
        }
        for seg in build.segments {
            let mid = seg.points[seg.points.len() / 2];
            self.tiles.entry(TileKey::of(mid[0], mid[1])).or_default().segments.push(seg);
        }
        touched.sort_unstable();
        touched.dedup();
        self.relink_nodes(&touched);
    }

    /// Withdraw one tile's lanes and footways. Agents on them are moved off by their regions.
    pub fn remove_tile(&mut self, tile: TileKey) -> usize {
        let Some(content) = self.tiles.remove(&tile) else { return 0 };
        let mut touched = vec![];
        for &id in &content.lanes {
            let lane = &mut self.lanes[id as usize];
            if !lane.alive {
                continue;
            }
            lane.alive = false;
            lane.links.clear();
            let (s, e, key) = (lane.start_node, lane.end_node, lane.key.clone());
            self.lane_by_key.remove(&key);
            self.nodes[s as usize].outgoing.retain(|&l| l != id);
            self.nodes[e as usize].incoming.retain(|&l| l != id);
            touched.push(s);
            touched.push(e);
            self.free_lanes.push(id);
        }
        for &id in &content.footways {
            let fw = &mut self.footways[id as usize];
            if !fw.alive {
                continue;
            }
            fw.alive = false;
            let nodes = [fw.ends[0].node, fw.ends[1].node];
            for n in nodes {
                if n == NONE {
                    continue;
                }
                let node = &mut self.foot_nodes[n as usize];
                node.ends.retain(|&(f, _)| f != id);
                if node.ends.is_empty() && !node.key.is_empty() {
                    let key = std::mem::take(&mut node.key);
                    node.signal = NONE;
                    self.foot_node_by_key.remove(&key);
                    self.free_foot_nodes.push(n);
                }
            }
            self.free_footways.push(id);
        }
        // A junction whose tile leaves controls nothing; its slot is reused.
        for &j in &content.junctions {
            let junction = &mut self.junctions[j as usize];
            junction.orphan = true;
            junction.degenerate = true;
            junction.axes.clear();
            self.free_junctions.push(j);
            for node in self.foot_nodes.iter_mut() {
                if node.signal == j {
                    node.signal = NONE;
                }
            }
        }
        touched.sort_unstable();
        touched.dedup();
        self.relink_nodes(&touched);
        // Nodes nothing arrives at or leaves from any more are freed.
        for &n in &touched {
            let node = &mut self.nodes[n as usize];
            if node.incoming.is_empty() && node.outgoing.is_empty() && node.pos[0].is_finite() {
                self.node_by_key.remove(&node.key);
                node.pos = [f64::NAN, f64::NAN];
                node.movements.clear();
                node.conflicts.clear();
                self.free_nodes.push(n);
            }
        }
        content.lanes.len() + content.footways.len()
    }

    /// Re-derive successors, plans, the box conflict matrix and the junction
    /// holds for every lane arriving at these nodes.
    fn relink_nodes(&mut self, nodes: &[NodeId]) {
        for &n in nodes {
            let mut incoming = self.nodes[n as usize].incoming.clone();
            incoming.sort_by(|a, b| self.lanes[*a as usize].key.cmp(&self.lanes[*b as usize].key));
            for &lane in &incoming {
                self.relink_lane(lane);
            }
            self.rebuild_conflicts(n);
            self.rebuild_holds(&incoming);
        }
    }

    fn relink_lane(&mut self, id: LaneId) {
        let lane = &self.lanes[id as usize];
        if !lane.alive {
            return;
        }
        let mut succ: Vec<(LaneId, Option<bool>)> = match &lane.explicit {
            Some(list) => list.iter().filter_map(|(k, legal)| self.lane_by_key.get(k).map(|&l| (l, *legal))).collect(),
            None => {
                let mut v: Vec<LaneId> = self.nodes[lane.end_node as usize]
                    .outgoing
                    .iter()
                    .copied()
                    .filter(|&o| o != id && self.lanes[o as usize].segment != lane.segment)
                    .collect();
                v.sort_by(|a, b| self.lanes[*a as usize].key.cmp(&self.lanes[*b as usize].key));
                v.into_iter().map(|l| (l, None)).collect()
            }
        };
        succ.retain(|(l, _)| {
            let exit = &self.lanes[*l as usize];
            let separated = match (self.surface_clearance_m, lane.surface_at(lane.length), exit.surface_at(0.0)) {
                (Some(clearance), Some(a), Some(b)) => (a - b).abs() >= clearance,
                _ => false,
            };
            exit.alive && !separated
        });
        let lane = &self.lanes[id as usize];
        let siblings: Vec<SiblingExit> = succ
            .iter()
            .map(|(l, _)| {
                let o = &self.lanes[*l as usize];
                SiblingExit { points: &o.pts, length: o.length, allows_through: o.role.allows(Movement::Through) }
            })
            .collect();
        let mut links = Vec::with_capacity(succ.len());
        for (k, (to, legal)) in succ.iter().enumerate() {
            let o = &self.lanes[*to as usize];
            let movement = geom::classify_movement(&lane.pts, &o.pts);
            let mut plan = geom::junction_plan(
                &lane.pts,
                lane.length,
                &o.pts,
                o.length,
                &siblings,
                k,
                lane.painted_through_mapped,
                self.merge_max_lateral_m,
                f64::INFINITY,
            );
            if let Some(p) = plan.as_mut().filter(|p| p.kind == geom::PlanKind::Line && p.drivable()) {
                if lane.continuous_heading && o.continuous_heading && lane.level == o.level
                    && lane.pts.last() == o.pts.first()
                    && lane.surface_at(lane.length) == o.surface_at(0.0)
                {
                    // Keep the movement and its claims. The aligned lanes already
                    // meet and have no intervening distance to spend at the node.
                    p.arc_len = 0.0;
                }
            }
            let drivable = plan.as_ref().is_none_or(|p| p.drivable());
            let legal = legal.unwrap_or_else(|| lane.role.allows(movement));
            let weight = if legal { lane.role.route_weight(movement) } else { 0.0 };
            links.push(Link { to: *to, movement, legal, weight, drivable, plan });
        }
        let terminus = !links.iter().any(|l| l.drivable);
        let lane = &mut self.lanes[id as usize];
        lane.links = links;
        lane.terminus = terminus;
    }

    /// Two movements through one node conflict when their driven paths cross
    /// or pass within a body width of each other, or they land on the same
    /// lane - the box mutex then admits them one at a time. Movements from the
    /// same lane never conflict here (the queue on that lane orders them).
    fn rebuild_conflicts(&mut self, n: NodeId) {
        let mut movements: Vec<(LaneId, u16)> = vec![];
        let mut paths: Vec<[P2; 9]> = vec![];
        for &l in &self.nodes[n as usize].incoming {
            let lane = &self.lanes[l as usize];
            if !lane.alive {
                continue;
            }
            for (k, link) in lane.links.iter().enumerate() {
                if !link.drivable {
                    continue;
                }
                let mut samples = [[0.0; 2]; 9];
                let out = &self.lanes[link.to as usize];
                for (i, s) in samples.iter_mut().enumerate() {
                    let u = i as f64 / 8.0;
                    *s = match &link.plan {
                        Some(p) => {
                            let (x, z, _) = p.pose_at(u * p.arc_len);
                            [x, z]
                        }
                        None => {
                            let a = lane.pts[lane.pts.len() - 1];
                            let b = out.pts[0];
                            [a[0] + (b[0] - a[0]) * u, a[1] + (b[1] - a[1]) * u]
                        }
                    };
                }
                movements.push((l, k as u16));
                paths.push(samples);
            }
        }
        let m = movements.len();
        let words = m.div_ceil(64).max(1);
        let mut conflicts = vec![0u64; m * words];
        // A presented body width plus margin (QUEUE_SEPARATION widths are 1.86-2.59 m).
        let clearance = 2.6f64;
        for a in 0..m {
            for b in (a + 1)..m {
                let (la, ka) = movements[a];
                let (lb, kb) = movements[b];
                if la == lb {
                    continue;
                }
                let ta = self.lanes[la as usize].links[ka as usize].to;
                let tb = self.lanes[lb as usize].links[kb as usize].to;
                let mut conflict = ta == tb;
                if !conflict {
                    'outer: for i in 0..8 {
                        for j in 0..8 {
                            if geom::segments_cross(paths[a][i], paths[a][i + 1], paths[b][j], paths[b][j + 1]) {
                                conflict = true;
                                break 'outer;
                            }
                        }
                    }
                }
                if !conflict {
                    'near: for pa in &paths[a] {
                        for pb in &paths[b] {
                            if geom::hypot(pa[0] - pb[0], pa[1] - pb[1]) < clearance {
                                conflict = true;
                                break 'near;
                            }
                        }
                    }
                }
                if conflict {
                    conflicts[a * words + b / 64] |= 1 << (b % 64);
                    conflicts[b * words + a / 64] |= 1 << (a % 64);
                }
            }
        }
        let node = &mut self.nodes[n as usize];
        node.movements = movements;
        node.conflicts = conflicts;
        node.words = words;
    }

    /// `junctionSamples(next, plan, body, clearance)`: the swept body of a
    /// movement, sampled every half metre through the fillet and one release
    /// length onto the exit lane. A held movement can include the return
    /// to the junction when the receiving lane loops back to it.
    pub(crate) fn junction_samples(
        &self, from: &Lane, next: LaneId, plan: &Plan, body: (f64, f64), clearance: f64, include_return: bool,
    ) -> (Vec<JunctionSample>, [f64; 4]) {
        let out = &self.lanes[next as usize];
        let available = (out.length - plan.exit_arc).max(0.0);
        let release = if include_return && out.end_node == from.end_node {
            available
        } else {
            available.min(body.0 / 2.0 + crate::consts::CLAIM_RELEASE_MARGIN_M + crate::consts::JUNCTION_SWEEP_STEP_M)
        };
        let total = plan.arc_len + release;
        let count = ((total / crate::consts::JUNCTION_SWEEP_STEP_M).ceil() as usize).max(1);
        let half_diag = geom::hypot(body.0, body.1) / 2.0;
        let mut stations: Vec<f64> = vec![0.0, plan.arc_len, total];
        for i in 0..=count {
            stations.push(total * i as f64 / count as f64);
        }
        for &arc in &out.cum {
            let d = plan.arc_len + arc - plan.exit_arc;
            if d > plan.arc_len && d < total {
                stations.push((d - 1e-7).max(plan.arc_len));
                stations.push((d + 1e-7).min(total));
            }
        }
        stations.sort_by(|a, b| a.total_cmp(b));
        stations.dedup();
        let motion = MovementPose::new(from, out, plan, body.0);
        let poses: Vec<(f64, f64, f64, f64)> = stations
            .iter()
            .map(|&d| {
                let (x, z, yaw) = motion.at(d);
                (x, z, yaw, d)
            })
            .collect();
        let mut samples = Vec::with_capacity(poses.len());
        let mut bounds = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for i in 0..poses.len() {
            let p = poses[i];
            let mut envelope = 0.0f64;
            for adj in [i.checked_sub(1), (i + 1 < poses.len()).then_some(i + 1)].into_iter().flatten() {
                let a = poses[adj];
                envelope = envelope.max((a.3 - p.3).abs() / 2.0 + half_diag * geom::wrap_pi(a.2 - p.2).abs() / 2.0);
            }
            let pad = envelope + clearance;
            let (w, l) = (body.1 + 2.0 * pad, body.0 + 2.0 * pad);
            let r = geom::hypot(w, l) / 2.0;
            samples.push(JunctionSample { body: (p.0, p.1, p.2, l, w), distance: p.3 });
            bounds = [bounds[0].min(p.0 - r), bounds[1].min(p.1 - r), bounds[2].max(p.0 + r), bounds[3].max(p.1 + r)];
        }
        (samples, bounds)
    }

    /// `junctionHoldFor(edge, car)` for every lane arriving at a node, every presented kind.
    fn rebuild_holds(&mut self, incoming: &[LaneId]) {
        let bus = (crate::consts::PRESENTED_LENGTH[3] as f64, crate::consts::PRESENTED_WIDTH[3] as f64);
        let mut sweeps = Vec::new();
        for &a in incoming {
            let lane = &self.lanes[a as usize];
            if !lane.alive {
                continue;
            }
            for link in &lane.links {
                if !link.drivable {
                    continue;
                }
                if let Some(plan) = &link.plan {
                    sweeps.push(self.junction_samples(lane, link.to, plan, bus, 0.3, false));
                }
            }
        }
        for &l in incoming {
            let lane = &self.lanes[l as usize];
            if !lane.alive {
                continue;
            }
            let mut hold = [0.0; 4];
            if !lane.links.is_empty() {
                for (kind, h) in hold.iter_mut().enumerate() {
                    let body = (crate::consts::PRESENTED_LENGTH[kind] as f64, crate::consts::PRESENTED_WIDTH[kind] as f64);
                    let radius = geom::hypot(body.1, body.0) / 2.0;
                    let mut setback = 0.0f64;
                    for (samples, b) in &sweeps {
                        for seg in 0..lane.pts.len() - 1 {
                            let (s, e) = (lane.pts[seg], lane.pts[seg + 1]);
                            let length = lane.cum[seg + 1] - lane.cum[seg];
                            if s[0].max(e[0]) + radius < b[0]
                                || s[0].min(e[0]) - radius > b[2]
                                || s[1].max(e[1]) + radius < b[1]
                                || s[1].min(e[1]) - radius > b[3]
                            {
                                continue;
                            }
                            let yaw = (e[0] - s[0]).atan2(e[1] - s[1]);
                            for sm in samples {
                                // The sample body as `presentedBody('bus', dims)` reads it.
                                let (x, z, heading, len, wid) = sm.body;
                                let (ol, ow) = (len.max(bus.0), wid.max(bus.1));
                                if let Some(gap) = crate::vehicles::cross_corridor_gap(
                                    s[0], s[1], yaw, body.0, body.1, x, z, heading, ol, ow, length,
                                ) {
                                    setback = setback.max(lane.length - lane.cum[seg] - gap);
                                }
                            }
                        }
                    }
                    *h = setback;
                }
            }
            self.lanes[l as usize].hold = hold;
        }
    }

    /// `cornerFilletFromArc(edge, next, arc)`: the plan a car at `arc` can still
    /// drive, re-planned with a smaller tangent when it is past the memoised entry.
    pub fn plan_from_arc(&self, lane: LaneId, link: usize, arc: f64) -> Option<Plan> {
        let l = &self.lanes[lane as usize];
        let k = l.links.get(link)?;
        if !k.drivable {
            return None;
        }
        let plan = k.plan.as_ref()?;
        let remaining = l.length - arc;
        if remaining >= plan.entry_setback - 1e-6 {
            return Some(plan.clone());
        }
        let cap = remaining + plan.along_in;
        if !(cap > 0.0) {
            return None;
        }
        let o = &self.lanes[k.to as usize];
        let siblings: Vec<SiblingExit> = l
            .links
            .iter()
            .map(|s| {
                let t = &self.lanes[s.to as usize];
                SiblingExit { points: &t.pts, length: t.length, allows_through: t.role.allows(Movement::Through) }
            })
            .collect();
        let replan =
            geom::junction_plan(&l.pts, l.length, &o.pts, o.length, &siblings, link, l.painted_through_mapped, self.merge_max_lateral_m, cap)?;
        let steerable =
            matches!(replan.kind, geom::PlanKind::Arc | geom::PlanKind::Merge) && replan.radius >= crate::consts::TURN_RADIUS_MIN;
        (steerable && (replan.entry_setback - remaining).abs() <= 1e-6).then_some(replan)
    }

    /// Is link `link` of `lane` still drivable from `arc` (`cornerFilletFromArc` non-null)?
    pub fn link_reachable(&self, lane: LaneId, link: usize, arc: f64) -> bool {
        let l = &self.lanes[lane as usize];
        let Some(k) = l.links.get(link) else { return false };
        if !k.drivable {
            return false;
        }
        match &k.plan {
            Some(p) if l.length - arc >= p.entry_setback - 1e-6 => true,
            Some(_) => self.plan_from_arc(lane, link, arc).is_some(),
            None => true,
        }
    }

    /// `reachableSuccessors(edge, arc)` is non-empty.
    pub fn any_reachable(&self, lane: LaneId, arc: f64) -> bool {
        (0..self.lanes[lane as usize].links.len()).any(|k| self.link_reachable(lane, k, arc))
    }

    pub(crate) fn any_reachable_for(&self, lane: LaneId, arc: f64, length: f64, width: f64) -> bool {
        let from = &self.lanes[lane as usize];
        from.fits_vehicle(length, width) && from.links.iter().enumerate().any(|(k, link)| {
            self.lanes[link.to as usize].fits_vehicle(length, width) && self.link_reachable(lane, k, arc)
        })
    }

    /// Index of a movement in its node's table.
    pub fn movement_index(&self, node: NodeId, from: LaneId, link: u16) -> Option<usize> {
        self.nodes[node as usize].movements.iter().position(|&(l, k)| l == from && k == link)
    }

    #[inline]
    pub fn movements_conflict(&self, node: NodeId, a: usize, b: usize) -> bool {
        let n = &self.nodes[node as usize];
        n.conflicts[a * n.words + b / 64] & (1 << (b % 64)) != 0
    }

    pub fn lane_by_key(&self, key: &str) -> Option<LaneId> {
        self.lane_by_key.get(key).copied()
    }

    pub fn alive_lanes(&self) -> impl Iterator<Item = LaneId> + '_ {
        self.lanes.iter().enumerate().filter(|(_, l)| l.alive).map(|(i, _)| i as LaneId)
    }

    pub fn alive_footways(&self) -> impl Iterator<Item = FootwayId> + '_ {
        self.footways.iter().enumerate().filter(|(_, f)| f.alive).map(|(i, _)| i as FootwayId)
    }

    /// Tile contents within `radius` of a point.
    pub fn tiles_near(&self, x: f64, z: f64, radius: f64) -> impl Iterator<Item = &TileContent> + '_ {
        let lo = TileKey::of(x - radius, z - radius);
        let hi = TileKey::of(x + radius, z + radius);
        (lo.x..=hi.x).flat_map(move |tx| (lo.z..=hi.z).filter_map(move |tz| self.tiles.get(&TileKey { x: tx, z: tz })))
    }

    /// Is the point (at grade) inside a street carriageway by more than `inset` metres?
    pub fn on_carriageway(&self, x: f64, z: f64, inset: f64) -> bool {
        self.on_carriageway_at(x, z, inset, 0)
    }

    /// `on_carriageway` for a point on structure level `level`: only the
    /// carriageways of that level count.
    pub fn on_carriageway_at(&self, x: f64, z: f64, inset: f64, level: i8) -> bool {
        for t in self.tiles_near(x, z, 40.0) {
            for seg in &t.segments {
                if seg.level == level && geom::dist_to_polyline(&seg.points, x, z) < seg.half - inset {
                    return true;
                }
            }
        }
        false
    }

    /// The stop offset of a lane (`stopOffsetForEdge`): the painted bar of a controlled approach, else the legacy hold.
    #[inline]
    pub fn stop_offset(&self, lane: LaneId) -> f64 {
        match self.lanes[lane as usize].control {
            ControlKind::Signal { stop_offset_m, .. } | ControlKind::MappedStop { stop_offset_m, .. } => stop_offset_m,
            _ => crate::consts::LEGACY_STOP_LINE,
        }
    }
}
