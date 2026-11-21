//! The Blender-built building manifest: one generated JS module per building
//! under `native/assets/blender-buildings/`, listed by its `index.js`
//! (written by the browser build's building exporter, since
//! removed). Each module is
//! `export const BLENDER_BUILDING = Object.freeze({...JSON...});`; the JSON
//! carries the building id, its footprint frame, its storey levels and the
//! hash-locked asset list (`url`, `sha256`, `bytes`) the JS runtime
//! (`blender/runtime.js` `loadVerified`) checks before it parses a GLB.

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub const GENERATED_DIR: &str = "native/assets/blender-buildings";

#[derive(Clone, Debug, Deserialize)]
pub struct AssetRef {
    pub url: String,
    pub sha256: String,
    pub bytes: u64,
    #[serde(default)]
    pub storey: Option<u32>,
}

impl AssetRef {
    /// File under `public/` the url names (`publicAsset` resolves it there).
    pub fn path(&self, repo_root: &Path) -> PathBuf {
        repo_root.join("public").join(self.url.trim_start_matches('/'))
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct XZ {
    pub x: f64,
    pub z: f64,
}

/// The frame the GLB was built in, in the z-north authoring frame
/// (`authoredFootprintFrame` of the reflected record at build time).
#[derive(Clone, Copy, Debug, Deserialize)]
pub struct BuiltFrame {
    #[serde(rename = "edgeIndex")]
    pub edge_index: usize,
    pub center: XZ,
    pub heading: f64,
    pub width: f64,
    pub depth: f64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Assets {
    pub exterior: AssetRef,
}

#[derive(Clone, Debug, Deserialize)]
pub struct BlenderBuilding {
    pub id: String,
    pub slug: String,
    pub pass: String,
    #[serde(rename = "heightM")]
    pub height_m: f64,
    pub frame: BuiltFrame,
    #[serde(default)]
    pub levels: Vec<f64>,
    /// the module's weathering override (`null`, `false` or an object).
    #[serde(default)]
    pub weathering: serde_json::Value,
    #[serde(default)]
    pub supersedes: bool,
    #[serde(default)]
    pub stream: bool,
    pub assets: Assets,
}

impl BlenderBuilding {
    /// The dataset key the record carries in the merged city (`sf-building-<key>`).
    pub fn record_key(&self) -> &str {
        self.id.trim_start_matches("sf-building-")
    }
}

/// Extract the JSON object of `Object.freeze(...)` from a generated module.
fn module_json(text: &str) -> Result<&str> {
    let start = text.find("Object.freeze(").ok_or_else(|| anyhow!("no Object.freeze("))? + "Object.freeze(".len();
    let end = text.rfind(");").ok_or_else(|| anyhow!("no closing );"))?;
    if end <= start {
        bail!("malformed generated module");
    }
    Ok(&text[start..end])
}

/// Every Blender building the generated index lists, in its order.
pub fn load_blender_buildings(repo_root: &Path) -> Result<Vec<BlenderBuilding>> {
    let dir = repo_root.join(GENERATED_DIR);
    let index = std::fs::read_to_string(dir.join("index.js")).with_context(|| format!("reading {}", dir.join("index.js").display()))?;
    let mut out = Vec::new();
    for line in index.lines() {
        let line = line.trim();
        if !line.starts_with("import ") {
            continue;
        }
        let Some(q) = line.find("from '") else { continue };
        let rest = &line[q + 6..];
        let file = rest.split('\'').next().unwrap_or_default().trim_start_matches("./");
        let p = dir.join(file);
        let text = std::fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?;
        let json = module_json(&text).with_context(|| format!("parsing {}", p.display()))?;
        let b: BlenderBuilding = serde_json::from_str(json).with_context(|| format!("decoding {}", p.display()))?;
        out.push(b);
    }
    if out.is_empty() {
        bail!("{} lists no Blender buildings", dir.join("index.js").display());
    }
    Ok(out)
}
