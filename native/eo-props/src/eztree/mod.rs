//! The browser game's ez-tree trees, ported: the vendored generator
//! (`src/render/vegetation/ez-tree/vendor/`, `generator.rs`), the game's
//! post-processing of it (`geometry.js` `buildEzTreeGeometry`: seed hash,
//! crown-cover leaf scale search, trunk bend, crown squeeze, leaf lift and
//! prune, atlas UVs and baked tones, crown normals, merge; `geometry.rs`), the
//! street pass's tree table and its near / window / far forms
//! (`street-furniture.js`; `street.rs`), and a cooker that bakes every form
//! the street pass and the streamed tiles draw once, in parallel, into one
//! cached file (`cook.rs`). Not from the JS: the crown core the window and
//! far forms take under `--tree-crowns` (`crown.rs`).
//!
//! Provenance and licence: the generator is ez-tree by Daniel Greenheck,
//! https://github.com/dgreenheck/ez-tree `src/lib` at commit
//! dcf309bd86bd521083d9c70f01f2de45fdc7c457, MIT licence (the game vendors it
//! at src/render/vegetation/ez-tree/vendor/ with its LICENSE). This port keeps
//! that licence for the ported generator code.
//!
//! Every JS design value (presets, species, tiers, crown-cover rule, crown
//! normal weights, atlas cells and opaque fractions, street tables) comes from
//! `data/eztree-js.json.gz`, written by the earlier browser version's exporter; the
//! generator's and geometry.js's own algorithm constants are code here, as
//! they are code there.
//!
//! Output is what the JS builds, up to float rounding: the same vertex order,
//! the same index, positions / normals / UVs / colours as the JS's
//! `Float32Array`s hold them (UVs y-up, as three.js), `part` 0 wood / 1 leaf.

mod coastal;
mod cook;
mod garden;
pub mod crown;
mod generator;
mod geometry;
mod street;

pub use coastal::{COASTAL_SHRUBS, COASTAL_TREES};
pub use garden::{GARDEN_SHRUBS, GARDEN_TREES};
pub use cook::{EzLibrary, StreetForm, cook_all, cook_all_with};
pub use crown::{PART_CORE, crown_core, tree_crowns_enabled};
pub use geometry::build_ez_tree;
pub use street::{
    StreetTreeMesh, StreetTreeMode, build_street_tree_geometry, street_tree_silhouette_for, street_tree_species,
    street_tree_species_id, street_tree_trunk_radius, street_tree_variant_count, street_tree_variant_for,
    surveyed_form_variant, surveyed_tree_variant,
};

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::Read;
use std::sync::OnceLock;

/// The export compiled into the crate.
pub const EZTREE_JS_GZ: &[u8] = include_bytes!("../../data/eztree-js.json.gz");

/// One tree as a single mesh, root at the origin, +Y up (metres).
#[derive(Clone, Debug, Default)]
pub struct EzMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// Atlas UVs, y-up (three.js).
    pub uvs: Vec<[f32; 2]>,
    /// The JS `color` attribute: linear tint x baked tone.
    pub colors: Vec<[f32; 3]>,
    /// 0 wood, 1 leaf (the JS `plantPart`); 2 a crown core (`crown_core`, not JS).
    pub part: Vec<u8>,
    pub indices: Vec<u32>,
    pub user: EzUserData,
}

/// The JS `userData.ezTree` numbers.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EzUserData {
    pub seed: u32,
    pub height_m: f64,
    pub half_crown_m: f64,
    pub trunk_radius_m: f64,
    pub wood_vertex_count: u32,
    pub wood_triangles: u32,
    pub leaf_triangles: u32,
    pub pruned_leaf_triangles: u32,
    pub lifted_leaf_cards: u32,
    pub clearance_m: f64,
    pub crown_cover: f64,
    pub crown_leaf_scale: f64,
    /// NaN where the JS has null (a species that opts out of the floor).
    pub crown_cover_before_rule: f64,
}

impl EzMesh {
    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }
}

/// A tier of `species.js` (`STREET_TIERS`, `FULL_TIERS`, `PARK_TIERS`).
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EzTier {
    pub section_stride: Option<f64>,
    pub segment_factor: Option<f64>,
    pub leaf_stride: Option<f64>,
    pub leaf_scale: Option<f64>,
    pub billboard: Option<String>,
    #[serde(default)]
    pub lift_leaves: bool,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrunkBend {
    pub sway_m: Option<f64>,
    pub sway_to: Option<f64>,
    pub lean_m: Option<f64>,
}

/// One `EZ_TREE_SPECIES` entry.
#[derive(Clone, Debug)]
pub struct EzSpecies {
    pub id: String,
    pub preset: String,
    /// Atlas cell of its leaf cards (`oak`, `ash`, `aspen`, `pine`).
    pub leaf: String,
    pub height_m: f64,
    pub max_half_crown_m: f64,
    pub clearance_m: Option<f64>,
    pub max_height_m: Option<f64>,
    pub height_factor: Option<f64>,
    pub crown_factor: Option<f64>,
    pub facade_reach_m: Option<f64>,
    pub street_lean_m: Option<f64>,
    pub trunk_bend: Option<TrunkBend>,
    pub observed_only: bool,
    pub surveyed_only: bool,
    /// False where the JS says `crownCover: false`.
    pub crown_cover: bool,
    pub seed_id: Option<String>,
    pub leaf_states: BTreeMap<String, String>,
    pub bark_tint: [f64; 3],
    pub leaf_tint: [f64; 3],
    pub tiers: BTreeMap<String, EzTier>,
    pub overrides: Value,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct AtlasRect {
    pub u0: f64,
    pub v0: f64,
    pub u1: f64,
    pub v1: f64,
}

/// street-furniture.js's street-tree table and thresholds.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreetTables {
    /// `STREET_TREE_SPECIES` ids, in variant order.
    pub species: Vec<String>,
    /// The genus mode's window/far silhouettes (`SILHOUETTES_4`, `SILHOUETTES_2`,
    /// `SILHOUETTE_BY_SPECIES`). Empty where the JS has no model modes: there
    /// every ring draws `STREET_TREE_SPECIES[variant % n]` and the rings past the
    /// near one carry one variant (see `has_modes`).
    #[serde(default)]
    pub silhouettes4: Vec<String>,
    #[serde(default)]
    pub silhouettes2: Vec<String>,
    #[serde(default)]
    pub silhouette_by_species: BTreeMap<String, usize>,
    /// `STREET_TREE_MAX_HALF_CROWN`.
    pub max_half_crown_m: f64,
    /// `STREET_TREE_MIN_LEAF_CLEARANCE_M`.
    pub min_leaf_clearance_m: f64,
    /// `STREET_TREE_SMALL_FOOTWAY_M`.
    pub small_footway_m: f64,
    /// `STREET_TREE_YOUNG_FRONTAGE_YEAR`.
    pub young_frontage_year: f64,
    /// `STREET_TREE_MATURE_MIN_HALF_M`.
    pub mature_min_half_m: f64,
    /// `STREET_TREE_BUDGET`.
    pub budget: BTreeMap<String, f64>,
    /// `FOLIAGE_MOTION.maximumDisplacementM / 0.8`, added to a street tree's bounding radius.
    pub wind_bound_pad_m: f64,
    /// The page's model mode the export saw ('genus' / 'legacy'); None without modes.
    pub default_mode: Option<String>,
    pub default_silhouettes: Option<usize>,
}

impl StreetTables {
    /// Whether the JS street pass has the genus/legacy model modes and silhouettes.
    pub fn has_modes(&self) -> bool {
        !self.silhouettes4.is_empty()
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CrownCoverRule {
    pub min_cover: f64,
    pub max_leaf_scale: f64,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct CrownNormal {
    pub outward: f64,
    pub card: f64,
    pub up: f64,
}

/// Everything read from `eztree-js.json.gz`.
pub struct EzData {
    pub sources: BTreeMap<String, String>,
    /// `EZ_TREE_GEOMETRY_VERSION` (seeds every tree).
    pub geometry_version: String,
    pub presets: Value,
    pub default_options: Value,
    pub species: BTreeMap<String, EzSpecies>,
    pub crown_cover: CrownCoverRule,
    /// `EZ_CROWN_NORMAL`; None where the JS keeps the generator's leaf normals.
    pub crown_normal: Option<CrownNormal>,
    pub crown_normals_enabled: bool,
    pub bark: AtlasRect,
    pub cells: BTreeMap<String, AtlasRect>,
    pub leaf_opaque_fraction: BTreeMap<String, f64>,
    /// `EZ_STREET_TREE_SPECIES`.
    pub street_species: Vec<String>,
    /// `EZ_PARK_TREE_SPECIES`.
    pub park_species: Vec<String>,
    pub street: StreetTables,
}

fn opt(v: &Value) -> Option<f64> {
    v.as_f64()
}

fn tint(v: &Value) -> Result<[f64; 3]> {
    let a = v.as_array().ok_or_else(|| anyhow!("tint is not an array"))?;
    Ok([a[0].as_f64().unwrap_or(0.0), a[1].as_f64().unwrap_or(0.0), a[2].as_f64().unwrap_or(0.0)])
}

impl EzData {
    /// The export compiled into the crate (parsed once).
    pub fn get() -> Result<&'static EzData> {
        static DATA: OnceLock<std::result::Result<EzData, String>> = OnceLock::new();
        DATA.get_or_init(|| EzData::from_gz(EZTREE_JS_GZ).map_err(|e| format!("{e:#}"))).as_ref().map_err(|e| anyhow!("{e}"))
    }

    pub fn from_gz(gz: &[u8]) -> Result<Self> {
        let mut s = String::new();
        flate2::read::GzDecoder::new(gz).read_to_string(&mut s).context("eztree-js.json.gz")?;
        let raw: Value = serde_json::from_str(&s).context("eztree-js.json")?;
        let mut species = BTreeMap::new();
        for (id, v) in raw["species"].as_object().ok_or_else(|| anyhow!("species"))? {
            let tiers: BTreeMap<String, EzTier> = serde_json::from_value(v["tiers"].clone()).with_context(|| format!("{id} tiers"))?;
            let trunk_bend: Option<TrunkBend> = if v["trunkBend"].is_object() { Some(serde_json::from_value(v["trunkBend"].clone())?) } else { None };
            let leaf_states = v["leafStates"]
                .as_object()
                .map(|m| m.iter().filter_map(|(k, x)| x.as_str().map(|s| (k.clone(), s.to_string()))).collect())
                .unwrap_or_default();
            species.insert(
                id.clone(),
                EzSpecies {
                    id: id.clone(),
                    preset: v["preset"].as_str().unwrap_or_default().to_string(),
                    leaf: v["leaf"].as_str().unwrap_or_default().to_string(),
                    height_m: v["heightM"].as_f64().ok_or_else(|| anyhow!("{id}.heightM"))?,
                    max_half_crown_m: v["maxHalfCrownM"].as_f64().ok_or_else(|| anyhow!("{id}.maxHalfCrownM"))?,
                    clearance_m: opt(&v["clearanceM"]),
                    max_height_m: opt(&v["maxHeightM"]),
                    height_factor: opt(&v["heightFactor"]),
                    crown_factor: opt(&v["crownFactor"]),
                    facade_reach_m: opt(&v["facadeReachM"]),
                    street_lean_m: opt(&v["streetLeanM"]),
                    trunk_bend,
                    observed_only: v["observedOnly"].as_bool().unwrap_or(false),
                    surveyed_only: v["surveyedOnly"].as_bool().unwrap_or(false),
                    crown_cover: v["crownCover"].as_bool() != Some(false),
                    seed_id: v["seedId"].as_str().map(str::to_string),
                    leaf_states,
                    bark_tint: tint(&v["barkTint"])?,
                    leaf_tint: tint(&v["leafTint"])?,
                    tiers,
                    overrides: v["overrides"].clone(),
                },
            );
        }
        // the Golden Gate's coastal trees and scrub (coastal.rs), beside the export's own.
        for sp in coastal::species_list() {
            species.insert(sp.id.clone(), sp);
        }
        // the rear yards' garden shrubs (garden.rs).
        for sp in garden::species_list() {
            species.insert(sp.id.clone(), sp);
        }
        let strings = |v: &Value| -> Vec<String> { v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default() };
        Ok(Self {
            sources: serde_json::from_value(raw["sources"].clone())?,
            geometry_version: raw["geometryVersion"].as_str().ok_or_else(|| anyhow!("geometryVersion"))?.to_string(),
            presets: raw["presets"].clone(),
            default_options: raw["defaultOptions"].clone(),
            species,
            crown_cover: serde_json::from_value(raw["crownCover"].clone())?,
            crown_normal: serde_json::from_value(raw["crownNormal"].clone())?,
            crown_normals_enabled: raw["crownNormalsEnabled"].as_bool().unwrap_or(false),
            bark: serde_json::from_value(raw["atlas"]["bark"].clone())?,
            cells: serde_json::from_value(raw["atlas"]["cells"].clone())?,
            leaf_opaque_fraction: serde_json::from_value(raw["leafOpaqueFraction"].clone())?,
            street_species: strings(&raw["streetSpecies"]),
            park_species: strings(&raw["parkSpecies"]),
            street: serde_json::from_value(raw["street"].clone()).context("street tables")?,
        })
    }

    pub fn species(&self, id: &str) -> Result<&EzSpecies> {
        self.species.get(id).ok_or_else(|| anyhow!("Unknown ez-tree species {id}"))
    }

    /// The generator options a species grows from with `seed` (geometry.js
    /// `generateRaw`: preset, overrides, the atlas bark wrap, then
    /// `TreeOptions.copy` onto the defaults), as JSON.
    pub fn options_value(&self, id: &str, seed: f64) -> Result<Value> {
        let sp = self.species(id)?;
        let mut options = self.presets.get(&sp.preset).cloned().unwrap_or_else(|| self.default_options.clone());
        generator::merge_into(&mut options, &sp.overrides);
        options["seed"] = seed.into();
        let mut bark = options["bark"].as_object().cloned().unwrap_or_default();
        bark.insert("textured".into(), false.into());
        bark.insert("textureScale".into(), serde_json::json!({ "x": 0, "y": 1 }));
        bark.insert("maps".into(), serde_json::json!({}));
        options["bark"] = Value::Object(bark);
        let mut leaves = options["leaves"].as_object().cloned().unwrap_or_default();
        leaves.insert("map".into(), Value::Null);
        options["leaves"] = Value::Object(leaves);
        let mut target = self.default_options.clone();
        generator::options_copy(&mut target, &options);
        Ok(target)
    }

    /// The species a street-tile or park pass draws: (species, tier) pairs, in
    /// species then tier order, every tier each species declares.
    pub fn all_builds(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for (id, sp) in &self.species {
            for tier in sp.tiers.keys() {
                out.push((id.clone(), tier.clone()));
            }
        }
        out
    }
}

/// `STREET_TREE_MAX_HALF_CROWN` (street-furniture.js).
pub fn street_tree_max_half_crown() -> Result<f64> {
    Ok(EzData::get()?.street.max_half_crown_m)
}

/// `STREET_TREE_MIN_LEAF_CLEARANCE_M` (street-furniture.js).
pub fn street_tree_min_leaf_clearance_m() -> Result<f64> {
    Ok(EzData::get()?.street.min_leaf_clearance_m)
}

/// An `EZ_TREE_SPECIES` entry by id.
pub fn ez_species(id: &str) -> Result<&'static EzSpecies> {
    EzData::get()?.species(id)
}
