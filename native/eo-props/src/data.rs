//! The JS data and constants the placement rules read, exported by
//! the earlier browser version's exporter into `data/props-js.json.gz`: the DataSF
//! street tree list (`SF_STREET_TREES`), the SFMTA sign inventory
//! (`SF_STREET_SIGNS`) and the JS passes' own constants, with the SHA-256 of
//! every JS source read. Nothing here is a literal of this crate.

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::io::Read;

pub const PROPS_JS_GZ: &[u8] = include_bytes!("../data/props-js.json.gz");

#[derive(Deserialize)]
pub struct TreeSurvey {
    pub stride: usize,
    pub forms: Vec<String>,
    pub species: Vec<String>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub sites: Vec<String>,
    pub bounds: Bounds,
    pub data: Vec<i32>,
    pub source: Value,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct Bounds {
    pub min_x: f64,
    pub max_x: f64,
    pub min_z: f64,
    pub max_z: f64,
}

#[derive(Deserialize)]
pub struct SignInventory {
    pub faces: Vec<String>,
    pub mounts: Vec<String>,
    /// [x10, z10, mountIndex, plateCount] per post.
    pub posts: Vec<i32>,
    /// [faceIndex, widthIn, heightIn, yawCode] per plate.
    pub plates: Vec<i32>,
    pub source: Value,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct VehicleSpec {
    pub id: String,
    pub length: f64,
    pub width: f64,
    pub height: f64,
    pub paint_set: String,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Paint {
    /// 0xrrggbb.
    pub hex: u32,
    pub weight: f64,
    pub name: String,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StreetProp {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
    pub triangles: Option<u64>,
    pub dimensions_m: Option<Vec<f64>>,
}

#[derive(Deserialize)]
struct Raw {
    sources: BTreeMap<String, String>,
    consts: Value,
    trees: TreeSurvey,
    signs: SignInventory,
}

pub struct JsProps {
    pub sources: BTreeMap<String, String>,
    pub consts: Value,
    pub trees: TreeSurvey,
    pub signs: SignInventory,
    pub vehicle_specs: Vec<VehicleSpec>,
    pub parking_weight: HashMap<String, [f64; 4]>,
    pub paint_sets: HashMap<String, Vec<Paint>>,
    pub street_props: BTreeMap<String, StreetProp>,
}

impl JsProps {
    /// The export compiled into the crate.
    pub fn load() -> Result<Self> {
        let mut p = Self::from_gz(PROPS_JS_GZ)?;
        // groundscale (the street size, 1.25 by default): the surveyed trees and sign
        // posts stand at their scaled positions (decimetre integers, as packed).
        let gs = eo_data::groundscale::scale();
        if gs != 1.0 {
            let st = p.trees.stride.max(2);
            for i in 0..p.trees.data.len() / st {
                for k in 0..2 {
                    let v = &mut p.trees.data[i * st + k];
                    *v = (*v as f64 * gs).round() as i32;
                }
            }
            let b = &mut p.trees.bounds;
            (b.min_x, b.max_x, b.min_z, b.max_z) = (b.min_x * gs, b.max_x * gs, b.min_z * gs, b.max_z * gs);
            for i in 0..p.signs.posts.len() / 4 {
                for k in 0..2 {
                    let v = &mut p.signs.posts[i * 4 + k];
                    *v = (*v as f64 * gs).round() as i32;
                }
            }
        }
        Ok(p)
    }

    pub fn from_gz(gz: &[u8]) -> Result<Self> {
        let mut s = String::new();
        flate2::read::GzDecoder::new(gz).read_to_string(&mut s).context("props-js.json.gz")?;
        let raw: Raw = serde_json::from_str(&s).context("props-js.json")?;
        let c = &raw.consts;
        let vehicle_specs: Vec<VehicleSpec> = serde_json::from_value(c["vehicleSpecs"].clone())?;
        let parking_weight: HashMap<String, [f64; 4]> = serde_json::from_value(c["parkingWeight"].clone())?;
        let paint_sets: HashMap<String, Vec<Paint>> = serde_json::from_value(c["paintSets"].clone())?;
        let street_props: BTreeMap<String, StreetProp> = serde_json::from_value(c["streetProps"].clone())?;
        Ok(Self { sources: raw.sources, consts: raw.consts, trees: raw.trees, signs: raw.signs, vehicle_specs, parking_weight, paint_sets, street_props })
    }

    /// A number of the JS constants: `group.key` (e.g. `signalHardware`, `kerbSetbackM`).
    pub fn num(&self, group: &str, key: &str) -> f64 {
        self.consts[group][key].as_f64().unwrap_or_else(|| panic!("props-js.json: consts.{group}.{key} is not a number"))
    }

    pub fn try_num(&self, group: &str, key: &str) -> Option<f64> {
        self.consts[group][key].as_f64()
    }

    pub fn list(&self, key: &str) -> Vec<f64> {
        self.consts[key].as_array().map(|a| a.iter().filter_map(|v| v.as_f64()).collect()).unwrap_or_default()
    }

    pub fn value(&self, key: &str) -> &Value {
        &self.consts[key]
    }


    pub fn spec(&self, id: &str) -> Option<&VehicleSpec> {
        self.vehicle_specs.iter().find(|s| s.id == id)
    }

    pub fn street_prop(&self, id: &str) -> Result<&StreetProp> {
        self.street_props.get(id).ok_or_else(|| anyhow!("street prop {id} is not in the JS manifest"))
    }
}

/// SHA-256 (hex) of bytes.
pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}
