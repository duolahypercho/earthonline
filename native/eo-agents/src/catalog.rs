//! What the JS game says about its traffic bodies and its crowd: the exported
//! `data/agents-js.json` (see the earlier browser version's exporter).

use anyhow::{Context, Result, anyhow};
use eo_assets::AssetRef;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

pub const AGENTS_JS: &str = include_str!("../data/agents-js.json");

#[derive(Clone, Debug, Deserialize)]
pub struct Lod {
    pub url: String,
    pub sha256: String,
    pub bytes: u64,
}

impl Lod {
    pub fn asset(&self) -> AssetRef {
        AssetRef { url: self.url.clone(), sha256: self.sha256.clone(), bytes: self.bytes, storey: None }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Body {
    pub length_m: f64,
    pub wheel_radius_m: f64,
    pub type_ids: Vec<String>,
    pub lods: Vec<Lod>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Paint {
    pub hex: String,
    pub weight: f64,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Spec {
    pub paint_set: String,
    pub length_m: Option<f64>,
    pub wheel_radius_m: Option<f64>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct HeroOverlay {
    pub near_radius: f64,
    pub near_max: u32,
    pub radius: f64,
    pub max: u32,
    pub far_radius: f64,
    pub far_max: u32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LampChannel {
    pub hex: String,
    pub day: f64,
    pub night: f64,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct Bands {
    pub skinned: f64,
    pub instanced: f64,
    pub far: f64,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct Budget {
    pub skinned: u32,
    pub instanced: u32,
    pub far: u32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Crowd {
    pub band_distances: Bands,
    pub budget: Budget,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AgentsJs {
    pub vehicles: BTreeMap<String, Body>,
    pub traffic_kind_map: BTreeMap<String, Vec<String>>,
    pub paint_sets: BTreeMap<String, Vec<Paint>>,
    pub rim_finishes: Vec<Paint>,
    pub specs: BTreeMap<String, Spec>,
    pub hero_overlay: HeroOverlay,
    pub hero_lamp: BTreeMap<String, LampChannel>,
    pub hero_lamp_day_running_floor: f64,
    pub crowd: Crowd,
    pub sources: BTreeMap<String, String>,
}

impl AgentsJs {
    pub fn load() -> Result<Self> {
        serde_json::from_str(AGENTS_JS).context("eo-agents/data/agents-js.json")
    }


    /// The catalogue ids a simulation kind (0 sedan, 1 taxi, 2 truck, 3 bus) draws from.
    pub fn pool(&self, kind: u16) -> &[String] {
        let name = match kind {
            1 => "taxi",
            2 => "truck",
            3 => "bus",
            _ => "sedan",
        };
        self.traffic_kind_map.get(name).or_else(|| self.traffic_kind_map.get("sedan")).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// `spawnCar` (src/citygen/traffic.js): `pool[floor(keyedRandom(`${seedInt}:${entityId}`, `vehicle-catalogue:${kind}`) * pool.length)]`.
    pub fn pick_body(&self, seed_int: u32, entity: u32, kind: u16) -> Option<&str> {
        let pool = self.pool(kind);
        if pool.is_empty() {
            return None;
        }
        let name = match kind {
            1 => "taxi",
            2 => "truck",
            3 => "bus",
            _ => "sedan",
        };
        let seed = if seed_int == 0 { 1 } else { seed_int };
        let draw = eo_sim::rng::keyed_random(&format!("{seed}:{entity}"), &format!("vehicle-catalogue:{name}"));
        let i = ((draw * pool.len() as f64).floor() as usize).min(pool.len() - 1);
        Some(pool[i].as_str())
    }

    /// The car's paint: `pickWeighted(PAINT_SETS[spec.paintSet], draw)` with a
    /// keyed draw on the entity (the JS presentation pass draws from its own
    /// seeded stream; this keeps the paint stable per car without it).
    pub fn pick_paint(&self, seed_int: u32, entity: u32, body: &str) -> &Paint {
        let set = self.specs.get(body).map(|s| s.paint_set.as_str()).unwrap_or("civilian");
        let list = self.paint_sets.get(set).or_else(|| self.paint_sets.get("civilian")).expect("civilian paint set");
        let draw = eo_sim::rng::keyed_random(&format!("{seed_int}:{entity}"), "vehicle-paint");
        pick_weighted(list, draw)
    }

    pub fn pick_rim(&self, seed_int: u32, entity: u32) -> &Paint {
        let draw = eo_sim::rng::keyed_random(&format!("{seed_int}:{entity}"), "vehicle-rim");
        pick_weighted(&self.rim_finishes, draw)
    }
}

/// `pickWeighted` (vehicle-catalogue.js).
pub fn pick_weighted(list: &[Paint], draw: f64) -> &Paint {
    let total: f64 = list.iter().map(|p| p.weight.max(0.0)).sum();
    if total <= 0.0 {
        return &list[0];
    }
    let mut t = draw.clamp(0.0, 0.999999) * total;
    for p in list {
        t -= p.weight.max(0.0);
        if t <= 0.0 {
            return p;
        }
    }
    list.last().unwrap()
}

pub fn read_verified(repo_root: &Path, asset: &AssetRef) -> Result<Vec<u8>> {
    let p = asset.path(repo_root);
    if !p.exists() {
        return Err(anyhow!("{} is not in this checkout ({}); not downloaded", asset.url, p.display()));
    }
    eo_assets::glb::read_verified(&p, asset)
}
