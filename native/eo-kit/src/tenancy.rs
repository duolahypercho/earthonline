//! Ground-floor tenancy as sf-data.js `attachBusinessTenancy` attaches it:
//! the building-metadata tiles (public/data/sf/building-metadata/v1, then
//! v1-pack for the footprint pack's ds ids), units whose category is not
//! `office`. Only what the classifier and the shop ground read is kept.

use crate::classify::TenancyUnit;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Deserialize)]
struct Index {
    tiles: HashMap<String, TileRef>,
}
#[derive(Deserialize)]
struct TileRef {
    file: String,
}
#[derive(Deserialize)]
struct Tile {
    #[serde(default)]
    buildings: HashMap<String, Record>,
}
#[derive(Deserialize)]
struct Record {
    #[serde(rename = "groundFloor", default)]
    ground_floor: Option<GroundFloor>,
}
#[derive(Deserialize)]
struct GroundFloor {
    #[serde(default)]
    units: Vec<Unit>,
}
#[derive(Deserialize)]
struct Unit {
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    subtype: Option<String>,
    #[serde(default)]
    edge: Option<serde_json::Value>,
    #[serde(rename = "alongM", default)]
    along_m: Option<f64>,
    #[serde(rename = "widthM", default)]
    width_m: Option<f64>,
}

/// Raw id (`12345`, `ds12`) -> units. Only records that HAVE a metadata
/// entry are present (an empty list means "recorded, no shop").
pub struct Tenancy {
    pub units: HashMap<String, Vec<TenancyUnit>>,
    pub tiles_read: usize,
}

impl Tenancy {
    pub fn empty() -> Self {
        Self { units: HashMap::new(), tiles_read: 0 }
    }

    pub fn load(repo_root: &Path) -> anyhow_lite::Result<Self> {
        let mut out = Self::empty();
        for base in ["public/data/sf/building-metadata/v1", "public/data/sf/building-metadata/v1-pack"] {
            let dir = repo_root.join(base);
            let Ok(text) = std::fs::read(dir.join("index.json")) else { continue };
            let index: Index = serde_json::from_slice(&text).map_err(|e| format!("{base}/index.json: {e}"))?;
            let mut files: Vec<&String> = index.tiles.values().map(|t| &t.file).collect();
            files.sort();
            for f in files {
                let bytes = std::fs::read(dir.join(f)).map_err(|e| format!("{base}/{f}: {e}"))?;
                let tile: Tile = serde_json::from_slice(&bytes).map_err(|e| format!("{base}/{f}: {e}"))?;
                out.tiles_read += 1;
                for (id, rec) in tile.buildings {
                    if out.units.contains_key(&id) {
                        continue;
                    }
                    let units = rec
                        .ground_floor
                        .map(|g| g.units)
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|u| u.category.as_deref().map(|c| !c.is_empty() && c != "office").unwrap_or(false))
                        .map(|u| TenancyUnit {
                            category: u.category.unwrap_or_default(),
                            subtype: u.subtype.unwrap_or_default(),
                            edge: u.edge.and_then(|e| e.as_f64()).filter(|e| e.fract() == 0.0).map(|e| e as i64),
                            along_m: u.along_m.map(|a| a * eo_data::groundscale::scale()), // along the scaled frontage
                            width_m: u.width_m,
                        })
                        .collect();
                    out.units.insert(id, units);
                }
            }
        }
        Ok(out)
    }

    pub fn units_for(&self, key: &str) -> Vec<TenancyUnit> {
        self.units.get(key).cloned().unwrap_or_default()
    }
}

/// Minimal error alias so this crate does not need anyhow.
pub mod anyhow_lite {
    pub type Result<T> = std::result::Result<T, String>;
}
