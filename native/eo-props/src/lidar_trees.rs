//! Residential trees measured from the USGS 3DEP lidar point cloud
//! (`--yard-lidar-trees`, off by default; `--no-yard-lidar-trees`, distinct
//! from the hill parks' default-on `--no-lidar-trees`). The survey writes
//! `trees.json` into `$EO_LIDAR_DIR/<area>/` (a folder outside the repository):
//! each crown's top on the surveyed 1:1 plan, its height above the lidar ground
//! and its crown radius. Every area found there is read once.
//!
//! The game draws a measured tree where nothing else stands: off the
//! footprints and the drawn street (the street tree survey keeps the kerb's
//! trees), clear of the trees already planted, outside the parks' own tree
//! classes (the ground cover keeps those). Its model is the street species
//! whose crown-to-height ratio is nearest the measured one, scaled to the
//! measured height and, within a third either way, to the measured crown.
//! Heights and crowns stay real size; only the position follows the ground
//! plan's scale (`eo_data::groundscale`).
//!
//! Under `--tree-crowns` (`eztree::tree_crowns_enabled`) a species' crown is
//! the one it shows past the near ring, its crown core's footprint
//! (`visible_crown_m`), not its reach cap (`maxHalfCrownM`, which a few cards
//! touch: the leaf mass is 0.55 to 0.75 of it), so a measured crown is drawn at
//! its measured area; and the crown's scale may part further from the height's
//! (`CROWN_SCALE_SPREAD_CROWNS`), the core keeping a widened crown whole.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

/// One measured tree, on the drawn (scaled) plan.
#[derive(Clone, Copy, Debug)]
pub struct LidarTree {
    pub x: f64,
    pub z: f64,
    pub height_m: f64,
    pub crown_r_m: f64,
}

/// `--yard-lidar-trees` sets `EO_YARD_LIDAR_TREES=on`, `--no-yard-lidar-trees` sets it `off`.
pub fn enabled() -> bool {
    std::env::var("EO_YARD_LIDAR_TREES").is_ok_and(|v| v == "on" || v == "1")
}

/// The species a measured tree may take: the street library's leafy ones
/// (not the bare pollard).
const SPECIES: [&str; 11] = [
    "street-ash", "street-oak", "street-mature", "street-pollard", "street-elm", "street-round", "street-canopy", "street-dome", "street-high", "street-full", "street-full-wrap",
];

/// How far the crown's own scale may part from the height's.
const CROWN_SCALE_SPREAD: f64 = 1.0 / 3.0;
/// The same under `--tree-crowns`.
const CROWN_SCALE_SPREAD_CROWNS: f64 = 0.6;

/// Clearance from a tree already planted, metres (drawn plan).
pub const CLEAR_OF_PLANTED_M: f64 = 2.5;

struct Installed {
    trees: Vec<LidarTree>,
    areas: Vec<String>,
}

fn load() -> Installed {
    let mut out = Installed { trees: vec![], areas: vec![] };
    let Some(root) = std::env::var_os("EO_LIDAR_DIR").map(PathBuf::from) else {
        eprintln!("lidar trees: EO_LIDAR_DIR not set; none drawn");
        return out;
    };
    let Ok(dir) = std::fs::read_dir(&root) else {
        eprintln!("lidar trees: {} not readable; none drawn", root.display());
        return out;
    };
    let mut entries: Vec<PathBuf> = dir.filter_map(|e| e.ok().map(|e| e.path().join("trees.json"))).filter(|p| p.is_file()).collect();
    entries.sort();
    for p in entries {
        let Ok(v) = std::fs::read(&p).map_err(|e| e.to_string()).and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).map_err(|e| e.to_string())) else {
            eprintln!("lidar trees: {} unreadable; skipped", p.display());
            continue;
        };
        let n0 = out.trees.len();
        for t in v["trees"].as_array().into_iter().flatten() {
            let (Some(x), Some(z), Some(h), Some(r)) = (t["x"].as_f64(), t["z"].as_f64(), t["height_m"].as_f64(), t["crown_r_m"].as_f64()) else { continue };
            let [x, z] = eo_data::groundscale::plan([x, z]);
            out.trees.push(LidarTree { x, z, height_m: h, crown_r_m: r });
        }
        let area = p.parent().and_then(|d| d.file_name()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        eprintln!("lidar trees: {} from {area}", out.trees.len() - n0);
        out.areas.push(area);
    }
    out
}

fn installed() -> &'static Installed {
    static I: OnceLock<Installed> = OnceLock::new();
    I.get_or_init(load)
}

/// The measured trees in [x0, x1) x [z0, z1) of the drawn plan (none unless `enabled`).
pub fn in_box(x0: f64, z0: f64, x1: f64, z1: f64) -> impl Iterator<Item = &'static LidarTree> {
    let list: &'static [LidarTree] = if enabled() { &installed().trees } else { &[] };
    list.iter().filter(move |t| t.x >= x0 && t.x < x1 && t.z >= z0 && t.z < z1)
}

/// Every measured tree (none unless `enabled`).
pub fn all() -> &'static [LidarTree] {
    if enabled() { &installed().trees } else { &[] }
}

/// Under `--tree-crowns`: the crown radius a species shows past the near ring,
/// metres at its model size: the mean of its window and far forms' crown-core
/// footprints (`eztree::crown::footprint_radius`), the area-equivalent radius
/// as the survey's `crown_r_m` is. Baked once (the trees are cooked without the
/// cache here, about a tenth of a second).
pub fn visible_crown_m(id: &str) -> Option<f64> {
    static MAP: OnceLock<HashMap<String, f64>> = OnceLock::new();
    let map = MAP.get_or_init(|| {
        let mut out = HashMap::new();
        let Ok(lib) = crate::eztree::cook_all(None) else { return out };
        use crate::eztree::{StreetForm, crown};
        for id in SPECIES {
            let r: Vec<f64> = [(StreetForm::Window, crown::WINDOW_SUBDIVISIONS), (StreetForm::Far, crown::FAR_SUBDIVISIONS)]
                .iter()
                .filter_map(|(f, sub)| lib.street_form(id, *f).map(|m| (m, *sub)))
                .map(|(m, sub)| crown::footprint_radius(&crown::crown_core(&m.mesh, sub, crown::INSET)))
                .filter(|r| *r > 0.0)
                .collect();
            if !r.is_empty() {
                out.insert(id.to_string(), r.iter().sum::<f64>() / r.len() as f64);
            }
        }
        out
    });
    map.get(id).copied()
}

/// The street species for a measured tree and its (crown, height) scales:
/// the species whose half crown over height is nearest the measured ratio;
/// height scale = measured / model height, crown scale = measured / model
/// half crown, held within `CROWN_SCALE_SPREAD` of the height scale. Under
/// `--tree-crowns` the model's crown is its `visible_crown_m` and the spread
/// `CROWN_SCALE_SPREAD_CROWNS` (see the module note).
pub fn model_for(t: &LidarTree) -> Option<(&'static str, f64, f64)> {
    model_for_with(t, crate::eztree::tree_crowns_enabled())
}

/// `model_for` with `--tree-crowns` given.
pub fn model_for_with(t: &LidarTree, crowns: bool) -> Option<(&'static str, f64, f64)> {
    let spread = if crowns { CROWN_SCALE_SPREAD_CROWNS } else { CROWN_SCALE_SPREAD };
    let ratio = t.crown_r_m / t.height_m.max(0.5);
    let mut best: Option<(&'static str, f64, f64, f64)> = None;
    for id in SPECIES {
        let Ok(s) = crate::eztree::ez_species(id) else { continue };
        let crown = if crowns { visible_crown_m(id).unwrap_or(s.max_half_crown_m) } else { s.max_half_crown_m };
        let d = (crown / s.height_m - ratio).abs();
        if best.is_none_or(|b| d < b.3) {
            best = Some((id, s.height_m, crown, d));
        }
    }
    let (id, h, c, _) = best?;
    let sy = t.height_m / h;
    let sx = (t.crown_r_m / c).clamp(sy * (1.0 - spread), sy * (1.0 + spread));
    Some((id, sx, sy))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(height_m: f64, crown_r_m: f64) -> LidarTree {
        LidarTree { x: 0.0, z: 0.0, height_m, crown_r_m }
    }

    /// Off: the reach cap is the crown, as before the switch. On: the crown a
    /// species shows (smaller than its cap), so the scale is larger and the
    /// drawn crown is the measured one wherever the spread allows.
    #[test]
    fn crowns_size_by_the_visible_crown() {
        for id in SPECIES {
            let s = crate::eztree::ez_species(id).unwrap();
            let v = visible_crown_m(id).expect("every leafy species has a core");
            assert!(v > 0.3 * s.max_half_crown_m && v < s.max_half_crown_m, "{id}: visible {v:.2} of cap {:.2}", s.max_half_crown_m);
        }
        for (h, r) in [(7.2, 2.1), (5.0, 1.2), (11.0, 4.0), (8.0, 3.4)] {
            let t = tree(h, r);
            let (id, sx, sy) = model_for_with(&t, false).unwrap();
            let s = crate::eztree::ez_species(id).unwrap();
            assert!((sy * s.height_m - h).abs() < 1e-9);
            let want = (r / s.max_half_crown_m).clamp(sy * (1.0 - CROWN_SCALE_SPREAD), sy * (1.0 + CROWN_SCALE_SPREAD));
            assert!((sx - want).abs() < 1e-9, "off: the scale as before the switch");
            let (id, sx, sy) = model_for_with(&t, true).unwrap();
            let s = crate::eztree::ez_species(id).unwrap();
            assert!((sy * s.height_m - h).abs() < 1e-9, "the height stays measured");
            let drawn = sx * visible_crown_m(id).unwrap();
            let free = (r / visible_crown_m(id).unwrap() - sx).abs() < 1e-9;
            assert!(!free || (drawn - r).abs() < 1e-9, "on: the visible crown is the measured one");
            assert!(sx >= sy * (1.0 - CROWN_SCALE_SPREAD_CROWNS) - 1e-9 && sx <= sy * (1.0 + CROWN_SCALE_SPREAD_CROWNS) + 1e-9);
        }
    }
}
