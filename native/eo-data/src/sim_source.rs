//! What the world simulation (`eo-sim`) reads from `sf-city.json.gz` that the
//! renderer's `City` does not keep: every road way with its `oneway`, `lanes`
//! and `sidewalk` tags, and the surveyed traffic signal positions (`signals`,
//! `[x, z]` in the map frame). Added for the simulation core; nothing else in
//! this crate depends on it.

use anyhow::{Result, bail};
use serde::Deserialize;

use crate::{DataPaths, RawRoad, parse_json};

#[derive(Debug, Deserialize)]
struct RawSimMeta {
    projection: crate::Projection,
}

#[derive(Debug, Deserialize)]
struct RawSimCity {
    meta: RawSimMeta,
    #[serde(default)]
    roads: Vec<RawRoad>,
    #[serde(default)]
    signals: Vec<[f64; 2]>,
}

/// Direction of travel a way permits, from its `oneway` tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Oneway {
    Both,
    /// One way along the way's point order (`oneway: true`).
    Forward,
    /// One way against it (`oneway: -1`).
    Reverse,
}

/// One road way as the simulation reads it.
#[derive(Clone, Debug)]
pub struct SimRoad {
    pub id: String,
    pub name: String,
    pub highway: String,
    pub oneway: Oneway,
    /// The `lanes` tag, if the way carries one.
    pub lanes: Option<u32>,
    pub maxspeed: String,
    /// The `sidewalk` tag (`both`, `left`, `right`, `no`, `separate`, ...; empty when untagged).
    pub sidewalk: String,
    pub bridge: bool,
    pub tunnel: bool,
    /// The OSM `layer` tag (`crate::layer_of`): 0 at grade or untagged.
    pub layer: i32,
    /// World metres (x east, z south), the file's precision.
    pub points: Vec<[f64; 2]>,
}

pub struct SimSource {
    pub roads: Vec<SimRoad>,
    /// Surveyed traffic signal positions, world metres.
    pub signals: Vec<[f64; 2]>,
}

/// A stop-control node joined to the packed map by its original OSM evidence.
#[derive(Clone, Debug, Deserialize)]
pub struct TrafficStop {
    pub id: u64,
    pub position: [f64; 2],
    #[serde(flatten)]
    pub control: TrafficStopControl,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum TrafficStopControl {
    /// An explicitly tagged all-way stop at the junction node.
    Junction { junction: Vec<String> },
    /// One signed approach with a direction measured on the source way.
    Approach { way: String, toward: [f64; 2], all_way: bool },
}

impl TrafficStop {
    pub fn load() -> Result<Vec<Self>> {
        Self::parse_str(include_str!("../data/sf-traffic-stops.json"))
    }

    pub fn parse_str(raw: &str) -> Result<Vec<Self>> {
        #[derive(Deserialize)]
        struct Pack {
            version: u32,
            stops: Vec<TrafficStop>,
        }
        let Pack { version, mut stops } = serde_json::from_str(raw)?;
        if version != 1 || stops.windows(2).any(|pair| pair[0].id >= pair[1].id) {
            bail!("sf-traffic-stops.json: unsupported version or unordered stop IDs");
        }
        for stop in &mut stops {
            let valid_control = match &stop.control {
                TrafficStopControl::Junction { junction } => !junction.is_empty() && junction.iter().all(|way| !way.is_empty()),
                TrafficStopControl::Approach { way, toward: [dx, dz], .. } => {
                    !way.is_empty() && (dx.hypot(*dz) - 1.0).abs() < 1e-6
                }
            };
            if !valid_control || !stop.position.iter().all(|x| x.is_finite()) {
                bail!("sf-traffic-stops.json: invalid stop node {}", stop.id);
            }
            stop.position = crate::groundscale::plan(stop.position);
        }
        Ok(stops)
    }
}

/// A source way's portion of a paved street run, in world metres.
#[derive(Clone, Debug, serde::Serialize, Deserialize)]
pub struct TrafficSection {
    pub way: String,
    pub points: Vec<[f64; 2]>,
    pub half: f64,
    /// Walk widths on the positive and negative sides of (-dz, dx).
    pub walks: [f64; 2],
    /// Stations where the drawn sidewalk gives way to another carriageway.
    /// Measured from this piece's first point, in the same side order as `walks`.
    #[serde(default, skip_serializing_if = "walk_gaps_empty")]
    pub walk_gaps: [Vec<[f64; 2]>; 2],
    pub lanes: Vec<TrafficLane>,
    /// Travel in both directions uses the same lane in the paved plan.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub shared_bidirectional: bool,
    /// Painted portion of this piece, measured from its first point.
    #[serde(default)]
    pub paint_span: Option<[f64; 2]>,
    #[serde(default)]
    pub paint: Vec<TrafficPaint>,
    /// The paint runs past another road, so it cannot establish a passing window.
    #[serde(default)]
    pub crosses_road: bool,
    /// Samples along this piece's own drawn carriageway centre, as [x, y, z].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub surface: Vec<[f64; 3]>,
    /// Inclusive indices into `surface` that bound each continuous drawn road strip.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub carriageway_spans: Vec<[usize; 2]>,
}

fn walk_gaps_empty(gaps: &[Vec<[f64; 2]>; 2]) -> bool {
    gaps.iter().all(Vec::is_empty)
}

/// Drawn asphalt at a junction, with the source ways that meet at its mouths.
#[derive(Clone, Debug, serde::Serialize, Deserialize)]
pub struct TrafficJunction {
    pub ways: Vec<String>,
    /// Junction node and boundary vertices in world metres [x, y, z].
    pub center: [f32; 3],
    pub rim: Vec<[f32; 3]>,
    /// Faces of the near mesh, indexing the center at zero and then the rim.
    /// An empty list uses the original fan around the center.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub triangles: Vec<[usize; 3]>,
}

#[derive(Clone, Debug, Default, serde::Serialize, Deserialize)]
pub struct TrafficPlan {
    pub sections: Vec<TrafficSection>,
    pub junctions: Vec<TrafficJunction>,
    /// Local surveyed routing carried by the same pavement preview.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surveyed_routes: Option<SurveyedTrafficRoutes>,
}

#[derive(Clone, Debug, serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SurveyedTrafficRoutes {
    pub collapsed_way: String,
    pub vehicle_envelope_m: [f64; 2],
    pub approaches: Vec<SurveyedTrafficApproach>,
    pub circulation: Vec<SurveyedTrafficLine>,
}

#[derive(Clone, Debug, serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SurveyedTrafficApproach {
    pub lane: String,
    /// The original native lane's downstream tile.
    pub tile: [i32; 2],
    pub inbound: bool,
    #[serde(default)]
    pub stop_id: Option<u64>,
    pub line: SurveyedTrafficLine,
}

#[derive(Clone, Debug, serde::Serialize, Deserialize)]
pub struct SurveyedTrafficLine {
    /// Complete directed lane in world metres [x, z].
    pub points: Vec<[f64; 2]>,
    /// Carriageway heights supplied by the pavement exporter.
    #[serde(default)]
    pub heights: Vec<f64>,
}

/// A drawn structure's surface for one source way, in world metres [x, y, z].
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct TrafficDeckSurface {
    pub way: String,
    pub points: Vec<[f64; 3]>,
    /// Painted lane centre lines, in the source way's point order and from
    /// the median out. Empty for a deck that uses the mapped lane layout.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lanes: Vec<TrafficDeckLane>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct TrafficDeckLane {
    pub width: f64,
    pub points: Vec<[f64; 3]>,
}

/// Road surfaces and the level labels of built structures used by traffic.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct TrafficStructures {
    pub levels: std::collections::BTreeMap<String, i8>,
    pub surfaces: Vec<TrafficDeckSurface>,
}

/// In a vehicle state, y is a reference height of its route's road surface.
/// The vehicle and pedestrian flag spaces are separate.
pub const TRAFFIC_SURFACE_Y: u16 = 0x1000;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, Deserialize)]
pub struct TrafficLane {
    /// +1 along the section's point order, -1 against it.
    pub direction: i8,
    /// Signed displacement along (dz, -dx).
    pub lateral: f64,
    pub width: f64,
}

#[derive(Clone, Debug, serde::Serialize, Deserialize)]
pub struct TrafficPaint {
    /// Signed displacement along (dz, -dx).
    pub lateral: f64,
    pub width: f64,
    pub broken: bool,
    pub yellow: bool,
    /// Direction of travel beside a broken white motor-lane divider, else 0.
    pub divider: i8,
    pub points: Vec<[f64; 2]>,
    /// Stripe length, gap, and phase in metres along the run.
    pub dash: Option<[f64; 3]>,
}

impl SimSource {
    pub fn load(paths: &DataPaths) -> Result<Self> {
        let mut raw: RawSimCity = parse_json(&paths.sf("sf-city.json.gz"))?;
        if raw.meta.projection.axes != "x east, z south, metres" {
            bail!("sf-city.json: unexpected frame '{}'", raw.meta.projection.axes);
        }
        let gs = crate::groundscale::scale();
        crate::movable_spans::add_to(&mut raw.roads);
        let roads = raw
            .roads
            .into_iter()
            .map(|r| SimRoad {
                id: r.id.key(),
                name: r.name,
                highway: r.highway,
                oneway: match &r.oneway {
                    serde_json::Value::Bool(true) => Oneway::Forward,
                    serde_json::Value::Number(n) if n.as_f64() == Some(-1.0) => Oneway::Reverse,
                    serde_json::Value::Number(n) if n.as_f64() == Some(1.0) => Oneway::Forward,
                    serde_json::Value::String(s) if s == "yes" || s == "true" || s == "1" => Oneway::Forward,
                    serde_json::Value::String(s) if s == "-1" => Oneway::Reverse,
                    _ => Oneway::Both,
                },
                lanes: r.lanes.filter(|l| l.is_finite() && *l >= 1.0).map(|l| l as u32),
                maxspeed: r.maxspeed,
                sidewalk: r.sidewalk,
                bridge: r.bridge,
                tunnel: r.tunnel,
                layer: crate::layer_of(&r.layer),
                points: r.points.chunks_exact(2).map(|c| [c[0] * gs, c[1] * gs]).collect(), // * gs
            })
            .collect();
        let mut signals = raw.signals;
        crate::groundscale::scale_points(&mut signals);
        Ok(Self { roads, signals })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_traffic_stops_load_cleanly_and_are_sorted() {
        let stops = TrafficStop::load().expect("embedded sf-traffic-stops.json must load cleanly");
        assert!(stops.len() > 3000, "expected thousands of SF stop controls, got {}", stops.len());
        assert!(stops.windows(2).all(|w| w[0].id < w[1].id));
    }

    #[test]
    fn traffic_stop_parser_rejects_bad_version_order_and_vectors() {
        assert!(TrafficStop::parse_str(r#"{"version":2,"stops":[]}"#).is_err());
        assert!(TrafficStop::parse_str(
            r#"{"version":1,"stops":[{"id":2,"position":[0,0],"junction":["1"]},{"id":1,"position":[0,0],"junction":["1"]}]}"#
        ).is_err());
        assert!(TrafficStop::parse_str(
            r#"{"version":1,"stops":[{"id":1,"position":[0,0],"way":"1","toward":[2.0,0.0],"all_way":false}]}"#
        ).is_err());
        assert!(TrafficStop::parse_str(r#"{"version":1,"stops":[]}"#).unwrap().is_empty());
    }
}
