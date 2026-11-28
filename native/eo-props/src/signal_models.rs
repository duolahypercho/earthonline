//! Street signals v1 (eo-app `--no-signal-models` keeps the v2 vehicle head and the
//! pedestrian head built in code): the vehicle and pedestrian signal heads
//! `scripts/assets/build-street-signals-v1.py` builds in Blender on the geometry of two
//! published models (CC BY 4.0; credited in the provenance and THIRD_PARTY_NOTICES.md), in
//! `public/assets/street-signals/`, every file's digest in the provenance beside them and
//! checked when the renderer reads it.
//!
//! On (`set_signal_models`, once per world load, before any props are read):
//! [`crate::JsProps::load`] lists `signalHead-v1.glb` in place of the v2 head and adds a
//! `pedHead` street prop. The renderer draws each file's lenses (`lens_red`, `lens_amber`,
//! `lens_green`) and pedestrian face (`ped_face`) from their own primitives, lit by the
//! signal's state through the LED images the build draws ([`SignalModels::lenses`],
//! [`SignalModels::ped_faces`]; the sources' photographs are not used). Off (the default
//! outside eo-app): the manifest as before.

use crate::data::StreetProp;
use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// The provenance (the digests every file is checked against).
pub const PROVENANCE: &str = "public/assets/street-signals/street-signals-v1.provenance.json";

/// The kinds the provenance must carry (street-prop ids).
pub const KINDS: [&str; 2] = ["signalHead", "pedHead"];

/// The lens and face images: a 2 x 2 grid of cells, each lens or face inset by `cell_inset` of
/// the image on every side of its cell (the shader's mip gutter).
#[derive(Clone, Debug)]
pub struct SignalImage {
    pub file: StreetProp,
    pub px: u32,
    pub cell_inset: f64,
}

pub struct SignalModels {
    /// Per kind (`signalHead`, `pedHead`), the file as the JS manifest lists a street prop.
    pub props: BTreeMap<String, StreetProp>,
    /// Per kind, the provenance row (lens pitch and radius, face size).
    pub rows: BTreeMap<String, Value>,
    /// The lenses: unlit red, amber, green (cells (0, 0), (1, 0), (0, 1), from the top left);
    /// the lit modules' LED pattern in cell (1, 1), red, amber, green in R, G, B.
    pub lenses: SignalImage,
    /// The pedestrian face unlit in cell (0, 0); the lit hand (R) and person (G) masks in (1, 0).
    pub ped_faces: SignalImage,
}

impl SignalModels {
    /// Is this street prop one of these files?
    pub fn is_mine(&self, p: &StreetProp) -> bool {
        self.props.values().any(|q| q.sha256 == p.sha256)
    }
}

fn file_of(what: &str, a: &Value) -> Result<StreetProp> {
    let s = |k: &str| a[k].as_str().map(str::to_string).ok_or_else(|| anyhow!("{PROVENANCE}: {what} has no {k}"));
    Ok(StreetProp {
        path: s("path")?,
        sha256: s("sha256")?,
        bytes: a["bytes"].as_u64().ok_or_else(|| anyhow!("{PROVENANCE}: {what} has no bytes"))?,
        triangles: a["triangles"].as_u64(),
        dimensions_m: a["dimensionsM"].as_array().map(|d| d.iter().filter_map(Value::as_f64).collect()),
    })
}

fn image_of(what: &str, a: &Value) -> Result<SignalImage> {
    let px = a["px"].as_u64().ok_or_else(|| anyhow!("{PROVENANCE}: {what} has no px"))? as u32;
    let cell_inset = a["cellInset"].as_f64().ok_or_else(|| anyhow!("{PROVENANCE}: {what} has no cellInset"))?;
    if !(px >= 64 && px.is_power_of_two() && (0.0..0.1).contains(&cell_inset)) {
        bail!("{PROVENANCE}: {what} px {px}, cellInset {cell_inset}");
    }
    Ok(SignalImage { file: file_of(what, a)?, px, cell_inset })
}

/// Read the provenance (the files are not read here).
pub fn load(repo: &Path) -> Result<SignalModels> {
    let path = repo.join(PROVENANCE);
    let text = std::fs::read_to_string(&path).with_context(|| format!("{}", path.display()))?;
    let v: Value = serde_json::from_str(&text).with_context(|| format!("{}", path.display()))?;
    let assets = v["assets"].as_object().ok_or_else(|| anyhow!("{PROVENANCE}: no assets"))?;
    let (mut props, mut rows) = (BTreeMap::new(), BTreeMap::new());
    for kind in KINDS {
        let a = assets.get(kind).ok_or_else(|| anyhow!("{PROVENANCE}: no {kind}"))?;
        props.insert(kind.to_string(), file_of(kind, a)?);
        rows.insert(kind.to_string(), a.clone());
    }
    let lenses = image_of("signalLenses", &v["images"]["signalLenses"])?;
    let ped_faces = image_of("pedFaces", &v["images"]["pedFaces"])?;
    Ok(SignalModels { props, rows, lenses, ped_faces })
}

static WANTED: AtomicBool = AtomicBool::new(false);
static MODELS: OnceLock<Option<SignalModels>> = OnceLock::new();

/// Turn the signal models on or off for the loads that follow (eo-app, at each world load).
pub fn set_signal_models(on: bool) {
    WANTED.store(on, Ordering::Relaxed);
}

/// The signal models when on and readable (read once; a failure is reported and the manifest kept).
pub fn active() -> Option<&'static SignalModels> {
    if !WANTED.load(Ordering::Relaxed) {
        return None;
    }
    MODELS
        .get_or_init(|| {
            let paths = eo_data::DataPaths::discover().map_err(|e| eprintln!("props: signal models not read ({e:#})")).ok()?;
            load(&paths.repo_root).map_err(|e| eprintln!("props: signal models not read ({e:#})")).ok()
        })
        .as_ref()
}

/// These files in place of the manifest's vehicle head, and the pedestrian head added.
pub(crate) fn apply(street_props: &mut BTreeMap<String, StreetProp>) {
    if let Some(s) = active() {
        for (kind, p) in &s.props {
            street_props.insert(kind.clone(), p.clone());
        }
    }
}
