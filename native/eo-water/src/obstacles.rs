//! Obstacles in the surf: rocks and piers that waves break on, registered by
//! whatever places them (game metres: centre x, z, radius, height of the top
//! above the sea level), for the water to foam round. A process-wide list
//! with a revision, so a reader can tell when to rebuild what it derived from
//! it. `set` is the rock placement's list; `set_named` keeps further lists
//! under a name (this crate's load registers the bridge's south fender,
//! `south_fender`), and `get` answers them all.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

/// One obstacle: centre (x, z), radius (m) and its top over the sea level (m), game metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Obstacle {
    pub x: f64,
    pub z: f64,
    pub radius: f64,
    pub height: f64,
}

static LIST: RwLock<Option<Arc<Vec<Obstacle>>>> = RwLock::new(None);
static NAMED: RwLock<Vec<(String, Arc<Vec<Obstacle>>)>> = RwLock::new(Vec::new());
static REVISION: AtomicU64 = AtomicU64::new(0);

/// Replace the list (bumps `revision`).
pub fn set(obstacles: Vec<Obstacle>) {
    *LIST.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(obstacles));
    REVISION.fetch_add(1, Ordering::AcqRel);
}

/// Replace the list registered under `name` (bumps `revision`).
pub fn set_named(name: &str, obstacles: Vec<Obstacle>) {
    let mut v = NAMED.write().unwrap_or_else(|e| e.into_inner());
    v.retain(|(n, _)| n != name);
    v.push((name.to_string(), Arc::new(obstacles)));
    REVISION.fetch_add(1, Ordering::AcqRel);
}

/// The lists in force: `set`'s, then every named one (empty before any).
pub fn get() -> Arc<Vec<Obstacle>> {
    let base = LIST.read().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default();
    let named = NAMED.read().unwrap_or_else(|e| e.into_inner());
    if named.is_empty() {
        return base;
    }
    let mut all: Vec<Obstacle> = base.as_ref().clone();
    for (_, l) in named.iter() {
        all.extend_from_slice(l);
    }
    Arc::new(all)
}

/// The Golden Gate Bridge's south tower fender, published at 311 x 170 ft
/// outside (Strauss 1937, the dimensions eo-landmarks golden_gate.rs
/// `FENDER_FT` draws), its long side across the bridge, its top at +15 ft
/// MLLW, about 3.6 m over mean sea level (NOAA 9414290: MSL 0.951 m over
/// MLLW, `beaches::MLLW_BELOW_MSL_M`). Its centre and the bridge's axis from
/// the south and north piers in eo-landmarks' data (landmarks-js.json,
/// `goldenGate.supports`), on the ground plan; its wall as a chain of
/// `FENDER_LINKS` discs of the wall's half thickness at sea level (10 ft) a
/// few metres apart, which the field's land distance (`extra::with_obstacles`)
/// reads as the oval's outline within a few tens of centimetres.
pub fn south_fender(repo_root: &std::path::Path) -> anyhow::Result<Vec<Obstacle>> {
    use anyhow::Context;
    let p = repo_root.join("native/eo-landmarks/data/landmarks-js.json");
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).with_context(|| format!("{}", p.display()))?)?;
    let sup = &v["goldenGate"]["supports"];
    let c = |k: &str| -> anyhow::Result<[f64; 2]> {
        let a = sup[k]["centre"].as_array().with_context(|| format!("goldenGate.supports.{k}.centre"))?;
        Ok([a[0].as_f64().unwrap_or(0.0), a[1].as_f64().unwrap_or(0.0)])
    };
    let gs = crate::ground_scale();
    let (s, n) = (c("southPier")?, c("northPier")?);
    let (s, n) = ([s[0] * gs, s[1] * gs], [n[0] * gs, n[1] * gs]);
    let l = (n[0] - s[0]).hypot(n[1] - s[1]);
    anyhow::ensure!(l > 100.0, "the piers are {l} m apart");
    let axis = [(n[0] - s[0]) / l, (n[1] - s[1]) / l];
    let across = [-axis[1], axis[0]];
    let ft = 0.3048 * gs;
    let (ra, rb) = (SOUTH_FENDER_FT[0] * ft / 2.0, SOUTH_FENDER_FT[1] * ft / 2.0);
    let wall = SOUTH_FENDER_FT[2] * ft / 2.0;
    let top = (SOUTH_FENDER_FT[3] * 0.3048 - crate::beaches::MLLW_BELOW_MSL_M) * eo_data::groundscale::height_scale();
    Ok((0..FENDER_LINKS)
        .map(|i| {
            let t = std::f64::consts::TAU * i as f64 / FENDER_LINKS as f64;
            let (al, ac) = (t.cos() * (rb - wall), t.sin() * (ra - wall));
            Obstacle { x: s[0] + axis[0] * al + across[0] * ac, z: s[1] + axis[1] * al + across[1] * ac, radius: wall, height: top }
        })
        .collect())
}

/// The fender's published outside across and along the bridge, its wall at the top, its top over MLLW (ft).
pub const SOUTH_FENDER_FT: [f64; 4] = [311.0, 170.0, 10.0, 15.0];
/// Discs along the fender's wall.
pub const FENDER_LINKS: usize = 160;

/// How many times the list has been set (0: never).
pub fn revision() -> u64 {
    REVISION.load(Ordering::Acquire)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_south_fender_is_an_oval_round_the_pier() {
        let mut root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        while !root.join("native/eo-landmarks/data/landmarks-js.json").exists() {
            assert!(root.pop());
        }
        let f = south_fender(&root).unwrap();
        assert_eq!(f.len(), FENDER_LINKS);
        let gs = crate::ground_scale();
        let cx = f.iter().map(|o| o.x).sum::<f64>() / f.len() as f64;
        let cz = f.iter().map(|o| o.z).sum::<f64>() / f.len() as f64;
        let rmax = f.iter().map(|o| (o.x - cx).hypot(o.z - cz) + o.radius).fold(0.0, f64::max);
        let rmin = f.iter().map(|o| (o.x - cx).hypot(o.z - cz) + o.radius).fold(f64::MAX, f64::min);
        // (rule 6: the published 311 x 170 ft on the ground plan)
        assert!((rmax - 311.0 * 0.3048 * gs / 2.0).abs() < 0.5, "{rmax}");
        assert!((rmin - 170.0 * 0.3048 * gs / 2.0).abs() < 0.5, "{rmin}");
        assert!(f.iter().all(|o| o.height > 3.0 && (o.radius - 5.0 * 0.3048 * gs).abs() < 1e-9));
        // a few metres between the discs
        let gap = (f[0].x - f[1].x).hypot(f[0].z - f[1].z);
        assert!(gap < 2.0 * f[0].radius, "{gap}");
    }

    #[test]
    fn set_replaces_and_counts() {
        assert_eq!(revision(), 0);
        assert!(get().is_empty());
        let a = Obstacle { x: 1.0, z: 2.0, radius: 3.0, height: 0.5 };
        set(vec![a]);
        assert_eq!(revision(), 1);
        assert_eq!(*get(), vec![a]);
        set(vec![]);
        assert_eq!(revision(), 2);
        assert!(get().is_empty());
    }
}
