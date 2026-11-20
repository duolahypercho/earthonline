//! The hill rock past the props tiles: the mid ring of 128 m tiles that
//! carry rock outcrops only (`eo_landmarks::hill_rock::plant`) from the props
//! tiles' reach out to `MID_LOAD_M`, on the vegetation's drawn ground
//! (`vegetation::VegGround`). Inside the props tiles the coarse vegetation
//! layer plants the same rocks.

use super::vegetation;
use super::{Key, RING_MID, TilePack};
use std::collections::HashMap;
use std::time::Instant;

/// The mid ring (hill rock past the props tiles, `props::LOAD_M`): out to this
/// (3D, m), dropped past the second; the rock models' draw distance
/// (`vegetation_models::THIN`) is the drop distance.
pub const MID_LOAD_M: f32 = 900.0;
pub const MID_DROP_M: f32 = 1000.0;

/// Whether the props pipeline plants hill rock this run (`--no-hill-rock`,
/// `--no-gg2-vegetation`: no).
pub fn props_rock_enabled(veg: &vegetation::VegGround) -> bool {
    veg.switches.hill_rock
}

/// Plant one mid-ring tile (hill rock only) as instances (worker thread).
pub(super) fn cook_mid(veg: Option<&vegetation::VegGround>, key: Key) -> TilePack {
    let t0 = Instant::now();
    let mut instances = vec![];
    let mut extra_counts: HashMap<&'static str, usize> = HashMap::new();
    if let Some(v) = veg.filter(|v| props_rock_enabled(v)) {
        let t = eo_street::STREET_TILE_M;
        let (x0, z0) = (key.0 as f64 * t, key.1 as f64 * t);
        let mut plants = vec![];
        eo_landmarks::hill_rock::plant(x0, z0, x0 + t, z0 + t, &|x, z| v.site(x, z), &mut plants);
        vegetation::instances(&plants, &mut instances, &mut extra_counts);
    }
    TilePack { key, ring: RING_MID, instances, lives: vec![], parked: vec![], counts: HashMap::new(), ms: t0.elapsed().as_secs_f64() * 1e3, wires: vec![], lights: vec![], overhead_ms: 0.0, extra_counts }
}
