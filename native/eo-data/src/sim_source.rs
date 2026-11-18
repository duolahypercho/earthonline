//! What the world simulation (`eo-sim`) reads from `sf-city.json.gz` that the
//! renderer's `City` does not keep: every road way with its `oneway`, `lanes`
//! and `sidewalk` tags, and the surveyed traffic signal positions (`signals`,
//! `[x, z]` in the map frame). Added for the simulation core; nothing else in
//! this crate depends on it.

use anyhow::{Result, bail};
use serde::Deserialize;

use crate::{DataPaths, RecordId, parse_json};

#[derive(Debug, Deserialize)]
struct RawSimRoad {
    id: RecordId,
    #[serde(default)]
    name: String,
    #[serde(default)]
    highway: String,
    #[serde(default)]
    oneway: serde_json::Value,
    #[serde(default)]
    lanes: Option<f64>,
    #[serde(default)]
    maxspeed: String,
    #[serde(default)]
    sidewalk: String,
    #[serde(default)]
    bridge: bool,
    #[serde(default)]
    tunnel: bool,
    #[serde(default)]
    points: Vec<f64>,
}

#[derive(Debug, Deserialize)]
struct RawSimMeta {
    projection: crate::Projection,
}

#[derive(Debug, Deserialize)]
struct RawSimCity {
    meta: RawSimMeta,
    #[serde(default)]
    roads: Vec<RawSimRoad>,
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
    /// World metres (x east, z south), the file's precision.
    pub points: Vec<[f64; 2]>,
}

pub struct SimSource {
    pub roads: Vec<SimRoad>,
    /// Surveyed traffic signal positions, world metres.
    pub signals: Vec<[f64; 2]>,
}

impl SimSource {
    pub fn load(paths: &DataPaths) -> Result<Self> {
        let raw: RawSimCity = parse_json(&paths.sf("sf-city.json.gz"))?;
        if raw.meta.projection.axes != "x east, z south, metres" {
            bail!("sf-city.json: unexpected frame '{}'", raw.meta.projection.axes);
        }
        let gs = crate::groundscale::scale();
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
                points: r.points.chunks_exact(2).map(|c| [c[0] * gs, c[1] * gs]).collect(), // * gs
            })
            .collect();
        let mut signals = raw.signals;
        crate::groundscale::scale_points(&mut signals);
        Ok(Self { roads, signals })
    }
}
