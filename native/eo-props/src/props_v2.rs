//! Street props v2 (eo-app `--no-street-props-v2` leaves them out): the
//! furniture, lamp, pole and palm models `scripts/assets/build-street-props-v2.py`
//! builds in Blender (`public/assets/street-props/<kind>-v2.glb`, each file's
//! digest in the provenance beside them). Made for the v1 slots (the JS
//! dimensions, within 5 % but for the deviations their rows record), they take
//! the place of the v1 files the checkout does not carry, so a kind draws a
//! model instead of its grey proxy or procedural stand-in.
//!
//! On (`set_street_props_v2`, once per world load, before any props are
//! read): [`crate::JsProps::load`] lists each v2 kind's file in place of the
//! v1 one, and [`crate::overhead::OverheadJs::load`] takes the v2 utility
//! pole's wire seats, shaft radius and hardware height. Off (the default
//! outside eo-app): the JS manifest as exported.

use crate::data::StreetProp;
use anyhow::{Context, Result, anyhow};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// The v2 provenance (the digests every file is checked against).
pub const PROVENANCE: &str = "public/assets/street-props/street-props-v2.provenance.json";

pub struct PropsV2 {
    /// Per kind, the v2 file as the JS manifest lists a street prop.
    pub props: BTreeMap<String, StreetProp>,
    /// Per kind, the provenance row (fronts, lights, the signal head's lens face, the pole's seats).
    pub rows: BTreeMap<String, Value>,
}

impl PropsV2 {
    /// The signal head's lens face (z of the lens discs, behind the visors' mouths).
    pub fn lens_face_z(&self) -> Option<f64> {
        self.rows.get("signalHead")?["lensFaceZ"].as_f64()
    }

    /// Is this street prop a v2 file (its colours as authored: no load-time repaint)?
    pub fn is_v2(&self, p: &StreetProp) -> bool {
        self.props.values().any(|q| q.sha256 == p.sha256)
    }
}

/// Read the v2 provenance (the files are not read here).
pub fn load(repo: &Path) -> Result<PropsV2> {
    let path = repo.join(PROVENANCE);
    let text = std::fs::read_to_string(&path).with_context(|| format!("{}", path.display()))?;
    let v: Value = serde_json::from_str(&text).with_context(|| format!("{}", path.display()))?;
    let assets = v["assets"].as_object().ok_or_else(|| anyhow!("{PROVENANCE}: no assets"))?;
    let (mut props, mut rows) = (BTreeMap::new(), BTreeMap::new());
    for (kind, a) in assets {
        let s = |k: &str| a[k].as_str().map(str::to_string).ok_or_else(|| anyhow!("{PROVENANCE}: {kind} has no {k}"));
        let prop = StreetProp {
            path: s("path")?,
            sha256: s("sha256")?,
            bytes: a["bytes"].as_u64().ok_or_else(|| anyhow!("{PROVENANCE}: {kind} has no bytes"))?,
            triangles: a["triangles"].as_u64(),
            dimensions_m: a["dimensionsM"].as_array().map(|d| d.iter().filter_map(Value::as_f64).collect()),
        };
        props.insert(kind.clone(), prop);
        rows.insert(kind.clone(), a.clone());
    }
    Ok(PropsV2 { props, rows })
}

static WANTED: AtomicBool = AtomicBool::new(false);
static V2: OnceLock<Option<PropsV2>> = OnceLock::new();

/// Turn the v2 props on or off for the loads that follow (eo-app, at each world load).
pub fn set_street_props_v2(on: bool) {
    WANTED.store(on, Ordering::Relaxed);
}

/// The v2 props when on and readable (read once; a failure is reported and the v1 manifest kept).
pub fn active() -> Option<&'static PropsV2> {
    if !WANTED.load(Ordering::Relaxed) {
        return None;
    }
    V2.get_or_init(|| {
        let paths = eo_data::DataPaths::discover().map_err(|e| eprintln!("props: street props v2 not read ({e:#})")).ok()?;
        load(&paths.repo_root).map_err(|e| eprintln!("props: street props v2 not read ({e:#})")).ok()
    })
    .as_ref()
}

/// The v2 files in place of the JS manifest's, for the kinds both list.
pub(crate) fn apply(street_props: &mut BTreeMap<String, StreetProp>) {
    if let Some(v2) = active() {
        for (kind, p) in &v2.props {
            if let Some(slot) = street_props.get_mut(kind) {
                *slot = p.clone();
            }
        }
    }
}

/// The v2 utility pole's wire seats, shaft radius and hardware height in place of the v1 pole's.
pub(crate) fn apply_overhead(consts: &mut Value) -> Result<()> {
    let Some(row) = active().and_then(|v| v.rows.get("utilityPole")) else { return Ok(()) };
    for k in ["attach", "shaftRadiusM", "hardwareFromM"] {
        if !row[k].is_null() {
            consts["utilityPole"][k] = row[k].clone();
        }
    }
    Ok(())
}
