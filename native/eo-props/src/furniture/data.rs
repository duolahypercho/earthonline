//! What the furniture walk reads from the JS, exported by
//! the earlier browser version's exporter into `data/furniture-js.json.gz`: the
//! kinds table in catalogue order, `DEFAULT_OPTIONS`, the walk's constants, the
//! surveyed inputs city-wide, the block overlays, the building attributes the
//! frontage rules read and the lamp optics, with the SHA-256 of each source.

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::Read;

pub const FURNITURE_JS_GZ: &[u8] = include_bytes!("../../data/furniture-js.json.gz");

/// `ZONES` of street-furniture.js.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Zone {
    Curb,
    Mid,
    Building,
}

/// One `STREET_FURNITURE_KINDS` entry.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KindSpec {
    pub id: String,
    pub zone: Zone,
    pub radius: f64,
    pub depth: f64,
    pub min_walk: f64,
    pub gap: f64,
    pub height: f64,
    pub weight: [f64; 4],
    pub corner: f64,
    #[serde(default)]
    pub exclusive: Option<f64>,
    #[serde(default)]
    pub through: Option<f64>,
    #[serde(default)]
    pub variants: Option<f64>,
    #[serde(default)]
    pub installed_only: bool,
    #[serde(default)]
    pub near_only: bool,
    #[serde(default)]
    pub fixture: bool,
    #[serde(default)]
    pub parking_only: bool,
    #[serde(default)]
    pub frontage_deny: Option<Vec<String>>,
    #[serde(default)]
    pub frontage_trade: bool,
    #[serde(default)]
    pub retail_frontage: Option<bool>,
    #[serde(default)]
    pub frontage_evidence: bool,
    #[serde(default)]
    pub shop_front: Option<bool>,
    #[serde(default)]
    pub work_zone: Option<String>,
    #[serde(default)]
    pub wall_stand_off: Option<f64>,
    #[serde(default)]
    pub catalogue_available: Option<bool>,
    #[serde(default)]
    pub reach: Option<f64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Options {
    pub corner_zone: f64,
    pub through_route: f64,
    pub wall_reach: f64,
    pub kerb_ramp_clear_m: f64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LampPracticals {
    pub color: u32,
    pub angle: f64,
    pub penumbra: f64,
    pub source_lift_m_h: f64,
    pub pool_inner_m_h: f64,
    pub pool_outer_m_h: f64,
    pub distance: f64,
    pub decay: f64,
    pub intensity: f64,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LampHeadSpec {
    pub height_m: f64,
    pub reach_m: f64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    pub lat: f64,
    pub lon: f64,
    pub meters_per_degree_lat: f64,
    pub meters_per_degree_lon: f64,
    pub z_sign: f64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Consts {
    pub surveyed_cell_m: f64,
    pub kerb_shifts: Vec<f64>,
    pub stop_shifts: Vec<f64>,
    pub unstated_shelter_share: f64,
    pub shelter_fallback_kinds: Vec<String>,
    pub surveyed_dressing_kinds: BTreeMap<String, String>,
    pub frontage_evidence_usages: Vec<String>,
    pub frontage_trade_usages: Vec<String>,
    pub street_light_kind: BTreeMap<String, String>,
    pub near_ring: Value,
    pub street_tree_species: Vec<String>,
    pub street_tree_variant_count_near: usize,
    pub street_tree_max_half_crown: f64,
    pub street_tree_min_leaf_clearance_m: f64,
    pub palm_height_m: Option<f64>,
    pub street_light_through_m: f64,
    pub lamp_practicals: LampPracticals,
    pub lamp_head: BTreeMap<String, LampHeadSpec>,
    #[serde(rename = "lampPoolMH")]
    pub lamp_pool_mh: Value,
    pub frame: Frame,
    pub seed_tag: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct StreetLights {
    pub spacing: BTreeMap<String, Value>,
    pub ways: Vec<f64>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct XzBounds {
    pub min_x: f64,
    pub max_x: f64,
    pub min_z: f64,
    pub max_z: f64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Meters {
    pub kinds: Vec<String>,
    pub bounds: XzBounds,
    pub data: Vec<f64>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct BikeRacks {
    pub data: Vec<f64>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LatLonBox {
    pub min_lat: f64,
    pub max_lat: f64,
    pub min_lon: f64,
    pub max_lon: f64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PackedPoints {
    pub xz: Vec<f64>,
    pub attr: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct StreetPoints {
    pub bbox: Option<LatLonBox>,
    pub features: BTreeMap<String, PackedPoints>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeOverride {
    pub i: u32,
    pub form: String,
    pub crown_m: Option<f64>,
    pub height_m: Option<f64>,
    pub leaf: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct Xz {
    pub x: f64,
    pub z: f64,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct Axis {
    pub from: Xz,
    pub to: Xz,
}

/// One overlay window entry (`side` and `s` or `s0`/`s1`), or a mailbox point.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayEntry {
    #[serde(default)]
    pub side: Option<String>,
    #[serde(default)]
    pub s: Option<f64>,
    #[serde(default)]
    pub s0: Option<f64>,
    #[serde(default)]
    pub s1: Option<f64>,
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub z: Option<f64>,
    #[serde(default)]
    pub kinds: Option<Vec<String>>,
    #[serde(default)]
    pub form: Option<String>,
    #[serde(default)]
    pub crown_m: Option<f64>,
    #[serde(default)]
    pub height_m: Option<f64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Overlay {
    pub id: String,
    pub street_ids: Vec<String>,
    pub axis: Axis,
    #[serde(default)]
    pub hydrants: Vec<OverlayEntry>,
    #[serde(default)]
    pub pay_stations: Vec<OverlayEntry>,
    #[serde(default)]
    pub bollards: Vec<OverlayEntry>,
    #[serde(default)]
    pub bike_racks: Vec<OverlayEntry>,
    #[serde(default)]
    pub mailboxes: Vec<OverlayEntry>,
    #[serde(default)]
    pub trees_absent: Vec<OverlayEntry>,
    #[serde(default)]
    pub trees_added: Vec<OverlayEntry>,
    #[serde(default)]
    pub furniture_absent: Vec<OverlayEntry>,
}

impl Overlay {
    pub fn list(&self, key: &str) -> &[OverlayEntry] {
        match key {
            "hydrants" => &self.hydrants,
            "payStations" => &self.pay_stations,
            "bollards" => &self.bollards,
            "bikeRacks" => &self.bike_racks,
            "mailboxes" => &self.mailboxes,
            "treesAbsent" => &self.trees_absent,
            "treesAdded" => &self.trees_added,
            "furnitureAbsent" => &self.furniture_absent,
            _ => &[],
        }
    }
}

/// Per building (`ids[i]`): `attrs[5i..5i+5]` = measured year (0 = none), Assessor use index (-1 none),
/// trade evidence (0/1), contract usage index (-1 none), shop ground floor (0/1).
#[derive(Clone, Debug, Deserialize)]
pub struct BuildingAttrs {
    pub uses: Vec<String>,
    pub usages: Vec<String>,
    pub ids: Vec<String>,
    pub attrs: Vec<i32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FurnitureJs {
    pub sources: BTreeMap<String, String>,
    pub kinds: Vec<KindSpec>,
    pub options: Options,
    pub consts: Consts,
    pub street_lights: StreetLights,
    pub distribution_ways: Vec<f64>,
    pub meters: Meters,
    pub bike_racks: BikeRacks,
    pub street_points: StreetPoints,
    pub tree_overrides: Vec<TreeOverride>,
    pub overlays: Vec<Overlay>,
    pub buildings: BuildingAttrs,
}

impl FurnitureJs {
    /// The export compiled into the crate.
    pub fn load() -> Result<Self> {
        let mut f = Self::from_gz(FURNITURE_JS_GZ)?;
        f.apply_ground_scale();
        Ok(f)
    }

    /// groundscale (the street size, 1.25 by default): the surveyed furniture
    /// (stops, hydrants, dressing points, meters, racks) and the observed-block
    /// overlays (axis, points, along-axis stations) at their scaled positions;
    /// the lat/lon frame scaled so a scaled point reads its real lat/lon.
    pub fn apply_ground_scale(&mut self) {
        let s = eo_data::groundscale::scale();
        if s == 1.0 {
            return;
        }
        for p in self.street_points.features.values_mut() {
            eo_data::groundscale::scale_flat(&mut p.xz);
        }
        // [x10, z10, attr] triples.
        for d in [&mut self.bike_racks.data, &mut self.meters.data] {
            for i in 0..d.len() / 3 {
                d[3 * i] = (d[3 * i] * s).round();
                d[3 * i + 1] = (d[3 * i + 1] * s).round();
            }
        }
        let b = &mut self.meters.bounds;
        (b.min_x, b.max_x, b.min_z, b.max_z) = (b.min_x * s, b.max_x * s, b.min_z * s, b.max_z * s);
        self.consts.frame.meters_per_degree_lat *= s;
        self.consts.frame.meters_per_degree_lon *= s;
        let opt = |v: &mut Option<f64>| {
            if let Some(x) = v.as_mut() {
                *x *= s;
            }
        };
        for o in &mut self.overlays {
            for p in [&mut o.axis.from, &mut o.axis.to] {
                p.x *= s;
                p.z *= s;
            }
            for e in o.hydrants.iter_mut().chain(o.pay_stations.iter_mut()).chain(o.bollards.iter_mut()).chain(o.bike_racks.iter_mut()).chain(o.mailboxes.iter_mut()).chain(o.trees_absent.iter_mut()).chain(o.trees_added.iter_mut()).chain(o.furniture_absent.iter_mut()) {
                opt(&mut e.s);
                opt(&mut e.s0);
                opt(&mut e.s1);
                opt(&mut e.x);
                opt(&mut e.z);
            }
        }
    }

    pub fn from_gz(gz: &[u8]) -> Result<Self> {
        let mut s = String::new();
        flate2::read::GzDecoder::new(gz).read_to_string(&mut s).context("furniture-js.json.gz")?;
        serde_json::from_str(&s).context("furniture-js.json")
    }
}
