//! The JS design values the generators read, exported by
//! the earlier browser version's exporter into `data/landmarks-js.json` (site
//! data and published / inferred dimensions from the `*-site.js` modules, the
//! registry's materials, ground grids and set rules, the landmark surfaces'
//! tuning), with the SHA-256 of every JS source. No coordinate or dimension
//! below is a literal of this crate.

use crate::geometry::GroundGrid;
use anyhow::{Result, anyhow};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::OnceLock;

pub const LANDMARKS_JS: &str = include_str!("../data/landmarks-js.json");

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Material {
    pub roughness: f64,
    pub metalness: f64,
    pub env_class: String,
    #[serde(default)]
    pub surface: Option<String>,
    #[serde(default)]
    pub draw: Option<String>,
    #[serde(default)]
    pub wire: bool,
    /// a surface's options (the precast surface's `bayM`, `storeyM`).
    #[serde(default)]
    pub surface_options: Option<Value>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Registry {
    pub id: String,
    pub name: String,
    pub version: String,
    pub anchor: Value,
    pub radius_m: f64,
    pub height_m: f64,
    pub excludes: Vec<String>,
    pub carries_ways: Vec<i64>,
    pub materials: BTreeMap<String, Material>,
    pub budget: Value,
    pub ground: GroundGrid,
    pub water: bool,
    /// building records the landmark draws in place of the generic kit (`claims`).
    #[serde(default)]
    pub claims: Vec<String>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Rules {
    pub near_m: f64,
    pub projection_radius_m: f64,
    pub hysteresis_m: f64,
    pub fog_cap_m: f64,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Surfaces {
    pub version: String,
    pub wire_min_pixels: f64,
    pub kinds: BTreeMap<String, Value>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Support {
    pub centre: [f64; 2],
    pub extent: [f64; 2],
    #[serde(default)]
    pub top_m: Option<f64>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Legs {
    pub east: [f64; 2],
    pub west: [f64; 2],
}

#[derive(Deserialize, Debug, Clone)]
pub struct KeepClear {
    pub id: i64,
    pub name: String,
    pub half: f64,
    pub points: Vec<f64>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GoldenGate {
    pub id: String,
    pub version: String,
    pub tower_legs: BTreeMap<String, Legs>,
    pub carriageways: Vec<Vec<f64>>,
    pub supports: BTreeMap<String, Support>,
    pub leg_stages: Vec<[f64; 4]>,
    pub struts: Vec<[f64; 3]>,
    pub keep_clear: Vec<KeepClear>,
    pub way_ends: BTreeMap<String, [f64; 2]>,
    pub dimensions: Value,
    pub carried_ways: Vec<i64>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Tower {
    #[serde(rename = "ref")]
    pub reference: String,
    pub centre: [f64; 2],
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CenterAnchorage {
    pub centre: [f64; 2],
    pub plan_m: Value,
    pub height_m: f64,
}

#[derive(Deserialize, Debug, Clone)]
pub struct DeckEnd {
    pub x: f64,
    pub z: f64,
    pub clearance: f64,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BayBridgeWest {
    pub id: String,
    pub version: String,
    pub towers: Vec<Tower>,
    pub tower_top_above_water_m: f64,
    pub tower_plan_m: [f64; 2],
    pub center_anchorage: CenterAnchorage,
    pub deck_ends: BTreeMap<String, DeckEnd>,
    pub deck_step_m: f64,
    pub dimensions: Value,
    pub carried_ways: Vec<i64>,
    pub excluded_records: Vec<String>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FerryPier {
    pub source_way_id: i64,
    pub outline: Vec<[f64; 2]>,
    pub presentation: Value,
    pub sheds: Value,
    pub deck_srgb: u32,
    pub shed_srgb: u32,
}

/// The shore edge's JS values (shoreline.js's seawall constants, renderer.js's
/// call, the beach rule's reaches, the Ferry Plaza pier).
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Shore {
    pub seawall: crate::shore::SeawallConsts,
    pub crest_above_road_m: f64,
    pub bottom_below_sea_m: f64,
    pub road_lift: f64,
    pub beach_wall_reach_m: f64,
    pub wide_foreshore_sources: Vec<String>,
    pub wide_foreshore_reach_m: f64,
    pub ferry_pier: FerryPier,
    pub wall_srgb: u32,
}

/// The paved greens' JS values (plaza-paving.js's plan constants, the street
/// surface's plaza unit, bay and palette).
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OpenSpace {
    pub plaza: crate::plaza::PlazaConsts,
    pub plaza_unit: f64,
    pub plaza_bay_units: f64,
    pub plaza_band_units: f64,
    /// `plazaUnitTone`, a bay's tone spread.
    #[serde(default)]
    pub plaza_unit_tone: f64,
    pub palette: String,
    pub colors: BTreeMap<String, String>,
}

/// the landmarks whose plan is scaled with the ground (the
/// bridges); every other landmark is moved whole, real size.
pub const PLAN_SCALED_LANDMARKS: &[&str] = &["golden-gate-bridge", "bay-bridge-west"];

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LandmarksJs {
    pub version: String,
    pub sources: BTreeMap<String, String>,
    pub data: BTreeMap<String, String>,
    pub sea_level_y: f64,
    pub seabed_y: f64,
    pub default_road_lift_m: f64,
    pub wire_drawn_radius_m: f64,
    pub rules: Rules,
    pub registry: BTreeMap<String, Registry>,
    pub surfaces: Surfaces,
    pub golden_gate: GoldenGate,
    pub bay_bridge_west: BayBridgeWest,
    pub shore: Shore,
    pub open_space: OpenSpace,
    /// The Ocean Beach and Sunset Dunes seawalls' section constants and the structure palette.
    pub promenades: Value,
    /// the landmark buildings' site data (`<id>-site.js`: the claimed
    /// record, the dimensions, the frame or centre the generator builds on).
    #[serde(default)]
    pub buildings: Value,
}

impl LandmarksJs {
    pub fn get() -> &'static LandmarksJs {
        static J: OnceLock<LandmarksJs> = OnceLock::new();
        J.get_or_init(|| {
            let mut j: LandmarksJs = serde_json::from_str(LANDMARKS_JS).expect("eo-landmarks: data/landmarks-js.json does not parse");
            j.apply_ground_scale();
            j
        })
    }

    /// the landmarks' ground-plan pieces on the scaled plan.
    ///
    /// * The shore: the Ferry Plaza pier's outline and the Ocean Beach / Sunset
    ///   Dunes seawall lines (their stair flights moved, not stretched).
    /// * The bridges are part of the ground plan, like the streets they carry:
    ///   every plan position (tower legs, piers, anchorages, carriageways, way
    ///   and deck ends) is scaled, so each bridge lands on its scaled shores and
    ///   approach roads and its spans are 1.25 x; the deck widths (roadway,
    ///   walks, the cable planes, and so each tower's leg spacing, which
    ///   straddles the deck) are scaled as the streets' widths are, so the deck
    ///   carries the scaled lanes the traffic drives. The members (legs, struts,
    ///   cables, suspenders, truss and girder depth, bents, railings), a pier's
    ///   or an anchorage's size along the span and every height stay real; the
    ///   suspenders and panels keep their real spacing (there are more of them).
    ///   The registry's anchor, radius and sampled ground grid follow.
    /// * The landmark buildings are built at their real site, real size, and
    ///   moved whole (`set::ground_scale_shift`).
    fn apply_ground_scale(&mut self) {
        let s = eo_data::groundscale::scale();
        if s == 1.0 {
            return;
        }
        let pt = |p: &mut [f64; 2]| {
            p[0] *= s;
            p[1] *= s;
        };
        let scale_num = |v: &mut Value, k: &str| {
            if let Some(x) = v[k].as_f64() {
                v[k] = serde_json::json!(x * s);
            }
        };
        let g = &mut self.golden_gate;
        for legs in g.tower_legs.values_mut() {
            pt(&mut legs.east);
            pt(&mut legs.west);
        }
        for c in &mut g.carriageways {
            eo_data::groundscale::scale_flat(c);
        }
        for sup in g.supports.values_mut() {
            pt(&mut sup.centre);
            sup.extent[0] *= s; // across the span (it carries the scaled deck); along stays real
        }
        for k in &mut g.keep_clear {
            eo_data::groundscale::scale_flat(&mut k.points);
            k.half *= s;
        }
        for e in g.way_ends.values_mut() {
            pt(e);
        }
        for k in ["widthM", "roadwayM", "sidewalkM", "mainSpanM", "sideSpanM", "archSpanM"] {
            scale_num(&mut g.dimensions, k);
        }
        let b = &mut self.bay_bridge_west;
        for t in &mut b.towers {
            pt(&mut t.centre);
        }
        b.tower_plan_m[0] *= s; // across; along ([1]) stays real
        pt(&mut b.center_anchorage.centre);
        scale_num(&mut b.center_anchorage.plan_m, "across");
        for e in b.deck_ends.values_mut() {
            e.x *= s;
            e.z *= s;
        }
        for k in ["roadwayM", "halfPlaneM", "mainSpanM"] {
            scale_num(&mut b.dimensions, k);
        }
        if let Some(p) = b.dimensions.get_mut("pierPlanM").and_then(|v| v.as_array_mut()) {
            // [along, across]: across scaled.
            if let Some(x) = p.get(1).and_then(|x| x.as_f64()) {
                p[1] = serde_json::json!(x * s);
            }
        }
        for id in PLAN_SCALED_LANDMARKS {
            if let Some(r) = self.registry.get_mut(*id) {
                scale_num(&mut r.anchor, "x");
                scale_num(&mut r.anchor, "z");
                r.radius_m *= s;
                pt(&mut r.ground.origin);
                r.ground.cell[0] *= s;
                r.ground.cell[1] *= s;
            }
        }
        eo_data::groundscale::scale_points(&mut self.shore.ferry_pier.outline);
        let p = &mut self.promenades;
        eo_data::groundscale::scale_json_keys(&mut p["oceanBeach"]["ESPLANADE"], &["x", "z"]);
        eo_data::groundscale::scale_json_numbers(&mut p["oceanBeach"]["STAIRS_Z"]);
        eo_data::groundscale::scale_json_numbers(&mut p["sunsetDunes"]["AXIS"]);
        eo_data::groundscale::scale_json_numbers(&mut p["sunsetDunes"]["WALL_Z"]);
        if let Some(steps) = p["sunsetDunes"]["steps"].as_array_mut() {
            for f in steps {
                let Some(pts) = f["pts"].as_array_mut() else { continue };
                let n = pts.len().max(1) as f64;
                let c = pts.iter().fold([0.0, 0.0], |a, q| [a[0] + q[0].as_f64().unwrap_or(0.0) / n, a[1] + q[1].as_f64().unwrap_or(0.0) / n]);
                for q in pts.iter_mut() {
                    for k in 0..2 {
                        if let Some(v) = q[k].as_f64() {
                            q[k] = serde_json::json!(v + (s - 1.0) * c[k]);
                        }
                    }
                }
            }
        }
    }
}

/// A number under `key` of a JSON object (the exported dimensions tables).
pub fn num(v: &Value, key: &str) -> f64 {
    v.get(key).and_then(Value::as_f64).ok_or_else(|| anyhow!("landmarks-js.json: no number {key}")).unwrap()
}

pub fn num_opt(v: &Value, key: &str) -> Result<f64> {
    v.get(key).and_then(Value::as_f64).ok_or_else(|| anyhow!("landmarks-js.json: no number {key}"))
}

pub fn pair(v: &Value, key: &str) -> [f64; 2] {
    let a = v.get(key).and_then(Value::as_array).unwrap_or_else(|| panic!("landmarks-js.json: no pair {key}"));
    [a[0].as_f64().unwrap(), a[1].as_f64().unwrap()]
}

pub fn hex(v: &Value, key: &str) -> u32 {
    num(v, key) as u32
}
