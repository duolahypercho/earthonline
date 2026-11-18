//! The shipped San Francisco window as the JS game builds it, read from the
//! oracle the earlier browser version's exporter writes
//! (`eo-sim/data/sf-window-oracle.json.gz`): the exact lane graph, footway
//! contract and signal plan inputs TrafficSim runs on, so the Rust core can be
//! driven on the same streets and compared with the JS run recorded beside them.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

use crate::geom::{LaneRole, P2};
use crate::network::*;
use crate::signals::{AxisInput, JunctionInput, MemberInput};

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OJunctionControl {
    pub kind: String,
    pub all_way: bool,
    pub enforced: bool,
    pub hold_seconds: f64,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OControl {
    pub junction: u32,
    pub axis: u8,
    pub stop_offset_metres: f64,
    pub bar_depth_metres: f64,
}

#[derive(Deserialize, Clone, Debug)]
pub struct OMovement {
    pub next: u32,
    pub class: String,
    pub legal: bool,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OLane {
    pub id: String,
    pub segment_id: String,
    pub street_id: Option<String>,
    pub highway: String,
    pub lane_role: String,
    pub painted_through_mapped: bool,
    pub lane_width: Option<f64>,
    pub start_node: u32,
    pub end_node: u32,
    pub points: Vec<f64>,
    pub outgoing: Vec<u32>,
    pub maxspeed: String,
    pub maxspeed_kmh: Option<f64>,
    pub maxspeed_source: String,
    pub js_max_speed: f64,
    /// `junctionHoldFor(edge, car)` for sedan, taxi, truck, bus.
    #[serde(default)]
    pub holds: Vec<f64>,
    pub junction_control: Option<OJunctionControl>,
    pub control: Option<OControl>,
    pub movements: Vec<OMovement>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OFootEnd {
    pub kerb_trim: Option<f64>,
    pub crossing_arc: Option<f64>,
    pub junction: bool,
    pub signal: bool,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OFootway {
    pub points: Vec<f64>,
    pub highway: String,
    pub segment_id: String,
    pub road_side: i8,
    pub node_keys: Option<[String; 2]>,
    pub normal: Option<[f64; 2]>,
    pub cross_index: Option<u32>,
    pub ends: Vec<OFootEnd>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OFootNode {
    pub key: String,
    pub x: f64,
    pub z: f64,
    pub signal_junction: Option<u32>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct OSegment {
    pub id: String,
    pub half: f64,
    pub points: Vec<f64>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ONetwork {
    pub lanes: Vec<OLane>,
    pub nodes: Vec<[f64; 2]>,
    pub footway_paths: Vec<OFootway>,
    pub footway_nodes: Vec<OFootNode>,
    pub segments: Vec<OSegment>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OPlan {
    pub from: u32,
    pub to: u32,
    pub kind: Option<String>,
    pub theta: Option<f64>,
    pub radius: Option<f64>,
    pub arc_len: Option<f64>,
    pub entry_setback: Option<f64>,
    pub exit_arc: Option<f64>,
    pub speed_cap: Option<f64>,
    pub lateral_step_m: Option<f64>,
    pub turn_side: i32,
    pub drivable: bool,
    pub poses: Option<Vec<[f64; 3]>>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OPed {
    pub crossing_m: f64,
    pub walk_s: f64,
    pub clearance_s: f64,
    pub buffer_s: f64,
    pub span_s: f64,
    pub release_axis: i32,
    pub walk_start_s: f64,
    pub clearance_end_s: f64,
    pub buffer_end_s: f64,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OMember {
    pub lane: u32,
    pub highway: String,
    pub posted_mps: Option<f64>,
    pub recorded_mps: Option<f64>,
    pub stop_offset_metres: f64,
    pub carriageway_half_metres: Option<f64>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OAxis {
    pub bearing: f64,
    pub suppressed: bool,
    pub members: Vec<OMember>,
    pub green_base_s: f64,
    pub green_s: f64,
    pub yellow_s: f64,
    pub all_red_s: f64,
    pub yellow_table_s: f64,
    pub v85: f64,
    pub pedestrian: Option<OPed>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OJunction {
    pub signal_id: serde_json::Value,
    pub signal_ids: Vec<serde_json::Value>,
    pub centre: [f64; 2],
    pub phase_offset: f64,
    pub orphan: bool,
    pub degenerate: bool,
    pub single_axis: bool,
    pub phase_count: usize,
    pub primary_axis: usize,
    pub cycle_s: f64,
    pub offset_s: f64,
    pub intervals: Vec<f64>,
    pub axes: Vec<OAxis>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct OSignals {
    pub junctions: Vec<OJunction>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OSample {
    pub t: f64,
    pub cars: usize,
    pub moving: usize,
    pub mean_speed: f64,
    pub off_lane: usize,
    pub off_lane_max: f64,
    pub overlaps: usize,
    pub walkers: usize,
    pub walking: usize,
    pub waiting: usize,
    pub crossing: usize,
    pub paused: usize,
    pub mean_ground_speed: f64,
    pub on_road: usize,
    pub on_road_outside_crossing: usize,
    pub person_in_car: usize,
    pub crossings_started: usize,
    pub crossings_completed: usize,
    /// Walkers waiting at a signalised kerb.
    #[serde(default)]
    pub waiting_signal: usize,
    /// Signalised crossings started outside WALK so far (cumulative).
    #[serde(default)]
    pub starts_outside_walk: usize,
    pub kinds: std::collections::HashMap<String, usize>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct OStepMs {
    pub median: f64,
    pub p95: f64,
    pub max: f64,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ORun {
    /// The seeded fleet as constructed: (lane, arc, kind, speed, driver pace).
    pub initial_cars: Vec<(u32, f64, String, f64, f64)>,
    pub steps: usize,
    pub step_seconds: f64,
    pub vehicle_pool: usize,
    pub ambient_walkers: usize,
    pub logical_walkers: usize,
    pub step_ms: OStepMs,
    pub samples: Vec<OSample>,
    pub aspect_changes: Vec<(usize, Vec<u8>)>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OMeta {
    pub seed_int: u32,
    pub bounds: serde_json::Value,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Oracle {
    pub meta: OMeta,
    pub network: ONetwork,
    pub plans: Vec<OPlan>,
    pub signals: OSignals,
    pub movement_stats: serde_json::Value,
    pub constants: serde_json::Value,
    pub run: ORun,
}

impl Oracle {
    pub fn default_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("data/sf-window-oracle.json.gz")
    }

    pub fn load(path: &Path) -> Result<Self> {
        let bytes = eo_data::read_maybe_gzip(path)?;
        serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn load_default() -> Result<Self> {
        Self::load(&Self::default_path())
    }

    /// The window as a network delivery. `merge_max_lateral_m` is the caMetric city lane.
    pub fn network_build(&self, merge_max_lateral_m: f64) -> NetworkBuild {
        let pts = |flat: &[f64]| -> Vec<P2> { flat.chunks_exact(2).map(|c| [c[0], c[1]]).collect() };
        let n = &self.network;
        let lanes = n
            .lanes
            .iter()
            .map(|l| {
                let control = match (&l.control, &l.junction_control) {
                    (Some(c), _) => ControlKind::Signal {
                        junction: c.junction,
                        axis: c.axis,
                        stop_offset_m: c.stop_offset_metres,
                        bar_depth_m: c.bar_depth_metres,
                    },
                    (None, Some(j)) if j.kind == "stop" => {
                        ControlKind::Stop { enforced: j.enforced, all_way: j.all_way, hold_s: j.hold_seconds }
                    }
                    _ => ControlKind::None,
                };
                let legal = l
                    .outgoing
                    .iter()
                    .map(|o| l.movements.iter().find(|m| m.next == *o).map(|m| m.legal).unwrap_or(true))
                    .collect();
                LaneBuild {
                    key: l.id.clone(),
                    segment: l.segment_id.clone(),
                    street: l.street_id.clone().unwrap_or_default(),
                    highway: l.highway.clone(),
                    role: LaneRole::parse(&l.lane_role),
                    painted_through_mapped: l.painted_through_mapped,
                    width: l.lane_width.unwrap_or(merge_max_lateral_m),
                    points: pts(&l.points),
                    start_node: n.nodes[l.start_node as usize],
                    end_node: n.nodes[l.end_node as usize],
                    max_speed: crate::geom::autonomous_speed(&l.highway, &l.maxspeed, l.maxspeed_kmh, &l.maxspeed_source),
                    control,
                    outgoing: Some(l.outgoing.clone()),
                    legal: Some(legal),
                    tile: None,
                }
            })
            .collect();
        let footways = n
            .footway_paths
            .iter()
            .map(|f| {
                let end = |e: Option<&OFootEnd>| FootEndBuild {
                    kerb_trim: e.and_then(|e| e.kerb_trim).filter(|v| *v > 0.0).unwrap_or(0.0),
                    crossing_arc: e
                        .and_then(|e| e.crossing_arc)
                        .filter(|v| *v > 0.0)
                        .unwrap_or(crate::consts::CROSSING_SETBACK_M),
                };
                FootwayBuild {
                    points: pts(&f.points),
                    highway: f.highway.clone(),
                    segment: f.segment_id.clone(),
                    road_side: f.road_side,
                    node_keys: f.node_keys.clone(),
                    normal: f.normal.unwrap_or([0.0, 0.0]),
                    cross_index: f.cross_index,
                    ends: [end(f.ends.first()), end(f.ends.get(1))],
                    tile: None,
                }
            })
            .collect();
        let foot_nodes = n
            .footway_nodes
            .iter()
            .map(|f| FootNodeBuild { key: f.key.clone(), pos: [f.x, f.z], signal: f.signal_junction })
            .collect();
        let junctions = self.signals.junctions.iter().map(junction_input).collect();
        let segments = n.segments.iter().map(|s| SegmentBuild { half: s.half, points: pts(&s.points) }).collect();
        let mut tiles: Vec<TileKey> = n.lanes.iter().map(|l| TileKey::of(l.points[0], l.points[1])).collect();
        tiles.sort();
        tiles.dedup();
        NetworkBuild { tiles, lanes, footways, foot_nodes, junctions, segments, merge_max_lateral_m }
    }
}

fn id_text(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The plan inputs of one exported junction (its outputs are what the tests compare).
pub fn junction_input(j: &OJunction) -> JunctionInput {
    JunctionInput {
        signal_ids: j.signal_ids.iter().map(id_text).collect(),
        centre: j.centre,
        phase_offset: j.phase_offset,
        orphan: j.orphan,
        axes: j
            .axes
            .iter()
            .map(|a| AxisInput {
                bearing: a.bearing,
                members: a
                    .members
                    .iter()
                    .map(|m| MemberInput {
                        lane: m.lane,
                        highway: m.highway.clone(),
                        posted_mps: m.posted_mps.unwrap_or(0.0),
                        recorded_mps: m.recorded_mps,
                        stop_offset_m: m.stop_offset_metres,
                        carriageway_half_m: m.carriageway_half_metres,
                    })
                    .collect(),
            })
            .collect(),
        crossing_m: j.axes.iter().map(|a| a.pedestrian.as_ref().map_or(0.0, |p| p.crossing_m)).collect(),
        tile: None,
    }
}
