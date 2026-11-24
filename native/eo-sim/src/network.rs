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

use crate::geom::{self, LaneRole, Movement, P2, Plan, SiblingExit};
use crate::signals::{JunctionInput, SignalJunction};
use crate::standards::SignalTiming;

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
    /// Far kerb of the same street (index into this build's footways).
    pub cross_index: Option<u32>,
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

/// One delivery of network content: the lanes, footways, junctions and carriageways of some tiles.
#[derive(Clone, Debug, Default)]
pub struct NetworkBuild {
    pub tiles: Vec<TileKey>,
    pub lanes: Vec<LaneBuild>,
    pub footways: Vec<FootwayBuild>,
    pub foot_nodes: Vec<FootNodeBuild>,
    pub junctions: Vec<JunctionInput>,
    pub segments: Vec<SegmentBuild>,
    /// Widest sideways step a through lane's one continuation may take as a merge
    /// (`THROUGH_MERGE_MAX_LATERAL_M` = caMetric('travel-lane-clearance-at-bulbout-auto-sf')).
    pub merge_max_lateral_m: f64,
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
    /// Structure level (`LaneBuild::level`): 0 at grade.
    pub level: i8,
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
    pub cross: FootwayId,
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
    /// The longest signal cycle resident: a walker waits at least one whole cycle (`kerbWaitMaxS`).
    pub kerb_wait_max_s: f64,
}

impl Network {
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
            kerb_wait_max_s: crate::consts::KERB_WAIT_MAX_S,
        }
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
        if build.merge_max_lateral_m > 0.0 {
            self.merge_max_lateral_m = build.merge_max_lateral_m;
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
                level: lb.level,
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
                FootEnd { node: NONE, kerb_trim: fb.ends[0].kerb_trim, crossing_arc: fb.ends[0].crossing_arc },
                FootEnd { node: NONE, kerb_trim: fb.ends[1].kerb_trim, crossing_arc: fb.ends[1].crossing_arc },
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
                cross: NONE,
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
            if let Some(ci) = fb.cross_index {
                self.footways[id as usize].cross = foot_ids[ci as usize];
            }
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
        succ.retain(|(l, _)| self.lanes[*l as usize].alive);
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
            let plan = geom::junction_plan(
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
    /// length onto the exit lane. (x, z, yaw, length, width) per sample, and the bounds.
    fn junction_samples(&self, next: LaneId, plan: &Plan, body: (f64, f64), clearance: f64) -> (Vec<[f64; 5]>, [f64; 4]) {
        let out = &self.lanes[next as usize];
        let release = body.0 / 2.0 + 1.0;
        let total = plan.arc_len + release;
        let count = ((total / 0.5).ceil() as usize).max(1);
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
        let poses: Vec<(f64, f64, f64, f64)> = stations
            .iter()
            .map(|&d| {
                let (x, z, yaw) = if d <= plan.arc_len {
                    plan.pose_at(d)
                } else {
                    let (x, z, dx, dz, _) = geom::at_arc(&out.pts, &out.cum, plan.exit_arc + d - plan.arc_len);
                    (x, z, dx.atan2(dz))
                };
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
            samples.push([p.0, p.1, p.2, l, w]);
            bounds = [bounds[0].min(p.0 - r), bounds[1].min(p.1 - r), bounds[2].max(p.0 + r), bounds[3].max(p.1 + r)];
        }
        (samples, bounds)
    }

    /// `junctionHoldFor(edge, car)` for every lane arriving at a node, every presented kind.
    fn rebuild_holds(&mut self, incoming: &[LaneId]) {
        let bus = (crate::consts::PRESENTED_LENGTH[3] as f64, crate::consts::PRESENTED_WIDTH[3] as f64);
        let mut sweeps: Vec<(Vec<[f64; 5]>, [f64; 4])> = vec![];
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
                    sweeps.push(self.junction_samples(link.to, plan, bus, 0.3));
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
                                let (ol, ow) = (sm[3].max(bus.0), sm[4].max(bus.1));
                                if let Some(gap) = crate::vehicles::cross_corridor_gap(
                                    s[0], s[1], yaw, body.0, body.1, sm[0], sm[1], sm[2], ol, ow, length,
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
            ControlKind::Signal { stop_offset_m, .. } => stop_offset_m,
            _ => crate::consts::LEGACY_STOP_LINE,
        }
    }
}
