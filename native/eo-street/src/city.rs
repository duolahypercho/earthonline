//! The street contract (`city.segments`, `city.intersections` of
//! src/citygen/sf-data.js `loadSfData`), as exported for the whole city by
//! the earlier browser version's exporter into `data/street-city.json.gz`.

use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use serde::Deserialize;
use serde_json::Value;
use std::io::Read;
use std::path::Path;

/// One lane of a segment's `laneLayout.lanes`.
#[derive(Clone, Debug)]
pub struct Lane {
    /// +1 increasing, -1 decreasing, 0 both/none.
    pub dir: i8,
    pub role: String,
    pub width: f64,
    pub lateral: f64,
    pub motor: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct RawSegment {
    k: Option<String>,
    id: String,
    #[serde(default)]
    n: String,
    sid: Option<String>,
    #[serde(default)]
    hw: String,
    w: Option<f64>,
    ln: Option<f64>,
    ow: Option<Value>,
    sw: Option<f64>,
    sl: Option<f64>,
    sr: Option<f64>,
    p: Vec<f64>,
    #[serde(default)]
    sep: u8,
    ll: Option<Vec<(i8, String, Option<f64>, Option<f64>, u8)>>,
    pl: Option<f64>,
    pr: Option<f64>,
    cl: Option<f64>,
    cr: Option<f64>,
    #[serde(default)]
    ckl: u8,
    #[serde(default)]
    ckr: u8,
    cbl: Option<f64>,
    cbr: Option<f64>,
    shl: Option<f64>,
    shr: Option<f64>,
    med: Option<f64>,
    st: Option<String>,
    kmh: Option<f64>,
    #[serde(default)]
    ms: String,
    row: Option<f64>,
    #[serde(default = "one")]
    ped: u8,
}

fn one() -> u8 {
    1
}

/// A loader segment, with the fields the street surface reads.
#[derive(Clone, Debug)]
pub struct Segment {
    pub key: Option<String>,
    pub id: String,
    pub street_name: String,
    pub street_id: Option<String>,
    pub highway: String,
    pub width: f64,
    pub lanes: Option<f64>,
    /// `oneway` as the loader writes it: "increasing", "decreasing", "both", true, ...
    pub oneway: String,
    pub sidewalk_w: Option<f64>,
    pub sidewalk_left: Option<f64>,
    pub sidewalk_right: Option<f64>,
    pub points: Vec<[f64; 2]>,
    /// Bridge, tunnel or non-zero layer (grade-separation.js `isSeparated`).
    pub separated: bool,
    pub lane_layout: Option<Vec<Lane>>,
    pub parking: [f64; 2],
    pub cycle: [f64; 2],
    pub cycle_kerbside: [bool; 2],
    pub cycle_buffer: [f64; 2],
    pub shoulder: [f64; 2],
    pub median: f64,
    pub street_type: Option<String>,
    pub maxspeed_kmh: Option<f64>,
    pub maxspeed: String,
    pub right_of_way: Option<f64>,
    pub pedestrian_eligible: bool,
}

impl Segment {
    /// `isOneway` (street-surface-v2.js).
    pub fn is_oneway(&self) -> bool {
        matches!(self.oneway.as_str(), "true" | "yes" | "1" | "increasing" | "decreasing")
    }
    /// `segSeed`: the stable key, else the window id.
    pub fn seed(&self) -> &str {
        self.key.as_deref().unwrap_or(&self.id)
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Intersection {
    pub id: String,
    pub k: Option<String>,
    pub x: f64,
    pub z: f64,
    pub sig: Option<String>,
}

#[derive(Deserialize)]
struct RawCrossing {
    p: Vec<f64>,
    #[serde(default)]
    c: String,
}

#[derive(Deserialize)]
struct RawCity {
    segments: Vec<RawSegment>,
    intersections: Vec<Intersection>,
    #[serde(default)]
    crossings: Vec<RawCrossing>,
    #[serde(default, rename = "crossingNodes")]
    crossing_nodes: Vec<(f64, f64, String)>,
    #[serde(default)]
    continental: Vec<f64>,
}

pub struct StreetCity {
    pub segments: Vec<Segment>,
    pub intersections: Vec<Intersection>,
    /// OSM crossing ways (points, crossing tag), for crossing evidence.
    pub crossings: Vec<(Vec<[f64; 2]>, String)>,
    /// streetPoints crossing nodes (position, attr code).
    pub crossing_nodes: Vec<([f64; 2], String)>,
    /// DataSF continental-crosswalk junction points.
    pub continental: Vec<[f64; 2]>,
}

fn crossings_of(c: Vec<RawCrossing>) -> Vec<(Vec<[f64; 2]>, String)> {
    c.into_iter().map(|w| (w.p.chunks_exact(2).map(|q| [q[0], q[1]]).collect(), w.c)).collect()
}

fn oneway_string(v: &Option<Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn convert(r: RawSegment) -> Segment {
    let points = r.p.chunks_exact(2).map(|c| [c[0], c[1]]).collect();
    let z = |v: Option<f64>| v.unwrap_or(0.0);
    Segment {
        key: r.k,
        id: r.id,
        street_name: r.n,
        street_id: r.sid,
        highway: r.hw.to_ascii_lowercase(),
        width: r.w.unwrap_or(f64::NAN),
        lanes: r.ln,
        oneway: oneway_string(&r.ow),
        sidewalk_w: r.sw,
        sidewalk_left: r.sl,
        sidewalk_right: r.sr,
        points,
        separated: r.sep != 0,
        lane_layout: r.ll.map(|l| {
            l.into_iter()
                .map(|(dir, role, w, lat, m)| Lane { dir, role, width: w.unwrap_or(0.0), lateral: lat.unwrap_or(0.0), motor: m != 0 })
                .collect()
        }),
        parking: [z(r.pl), z(r.pr)],
        cycle: [z(r.cl), z(r.cr)],
        cycle_kerbside: [r.ckl != 0, r.ckr != 0],
        cycle_buffer: [z(r.cbl), z(r.cbr)],
        shoulder: [z(r.shl), z(r.shr)],
        median: z(r.med),
        street_type: r.st,
        maxspeed_kmh: r.kmh,
        maxspeed: r.ms,
        right_of_way: r.row,
        pedestrian_eligible: r.ped != 0,
    }
}

impl StreetCity {
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        let raw: RawCity = serde_json::from_slice(bytes)?;
        Ok(Self {
            segments: raw.segments.into_iter().map(convert).collect(),
            intersections: raw.intersections,
            crossings: crossings_of(raw.crossings),
            crossing_nodes: raw.crossing_nodes.into_iter().map(|(x, z, a)| ([x, z], a)).collect(),
            continental: raw.continental.chunks_exact(2).map(|q| [q[0], q[1]]).collect(),
        })
    }

    pub fn load_gz(path: &Path) -> Result<Self> {
        let raw = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let mut out = Vec::with_capacity(raw.len() * 8);
        GzDecoder::new(&raw[..]).read_to_end(&mut out).with_context(|| format!("gunzip {}", path.display()))?;
        Self::from_json(&out)
    }

    /// The exported whole-city contract shipped beside this crate.
    pub fn load() -> Result<Self> {
        let mut c = Self::load_gz(&Path::new(env!("CARGO_MANIFEST_DIR")).join("data/street-city.json.gz"))?;
        c.apply_ground_scale();
        Ok(c)
    }

    /// groundscale (the street size, 1.25 by default): the street plan scaled
    /// horizontally about the map origin: centrelines, nodes, crossing evidence
    /// and every cross-section width (carriageway, lanes, parking, cycle,
    /// buffers, shoulders, median, footways, right of way). The standards'
    /// own values (kerb height, paint widths, ramp slopes) are not touched.
    pub fn apply_ground_scale(&mut self) {
        let s = eo_data::groundscale::scale();
        if s == 1.0 {
            return;
        }
        let m = |v: &mut Option<f64>| {
            if let Some(x) = v.as_mut() {
                *x *= s;
            }
        };
        for g in &mut self.segments {
            eo_data::groundscale::scale_points(&mut g.points);
            g.width *= s;
            m(&mut g.sidewalk_w);
            m(&mut g.sidewalk_left);
            m(&mut g.sidewalk_right);
            m(&mut g.right_of_way);
            if let Some(l) = g.lane_layout.as_mut() {
                for lane in l {
                    lane.width *= s;
                    lane.lateral *= s;
                }
            }
            for k in 0..2 {
                g.parking[k] *= s;
                g.cycle[k] *= s;
                g.cycle_buffer[k] *= s;
                g.shoulder[k] *= s;
            }
            g.median *= s;
        }
        for i in &mut self.intersections {
            i.x *= s;
            i.z *= s;
        }
        for (pts, _) in &mut self.crossings {
            eo_data::groundscale::scale_points(pts);
        }
        for (p, _) in &mut self.crossing_nodes {
            p[0] *= s;
            p[1] *= s;
        }
        eo_data::groundscale::scale_points(&mut self.continental);
    }

    /// Build from already-parsed compact records (the oracle windows carry the same shape).
    pub fn from_values(segments: &Value, intersections: &Value) -> Result<Self> {
        let segs: Vec<RawSegment> = serde_json::from_value(segments.clone())?;
        let ints: Vec<Intersection> = serde_json::from_value(intersections.clone())?;
        Ok(Self { segments: segs.into_iter().map(convert).collect(), intersections: ints, crossings: vec![], crossing_nodes: vec![], continental: vec![] })
    }

    /// An oracle window (its crossings and crossing nodes) with the city-wide continental list.
    pub fn from_window(w: &Value, continental: &[[f64; 2]]) -> Result<Self> {
        let mut c = Self::from_values(&w["segments"], &w["intersections"])?;
        let ways: Vec<RawCrossing> = serde_json::from_value(w["crossings"].clone()).unwrap_or_default();
        c.crossings = crossings_of(ways);
        let nodes: Vec<(f64, f64, String)> = serde_json::from_value(w["crossingNodes"].clone()).unwrap_or_default();
        c.crossing_nodes = nodes.into_iter().map(|(x, z, a)| ([x, z], a)).collect();
        c.continental = continental.to_vec();
        Ok(c)
    }
}
