//! The hill builder: offline cooks for the city's open ground
//! (Docs/HILLS-PLAN.md), and the readers of what they write.
//!
//! The cooks run as an example, so the runtime that reads their output does
//! not build the JPEG decoder or the block compressor:
//!
//! ```text
//! EO_AERIAL_DIR=<NAIP folder> cargo run --release -p eo-hills --example cook -- macro [--preview out.png]
//! cargo run --release -p eo-hills --example cook -- relief [--preview <dir>]
//! EO_AERIAL_DIR=<NAIP folder> cargo run --release -p eo-hills --example cook -- planting
//! cargo run --release -p eo-hills --example cook -- decks [--survey]
//! cargo run --release -p eo-hills --example cook -- trees [--preview <dir>]
//! ```
//!
//! * [`macro_map`]: the aerial macro colour map (stage H2), a de-lit ground
//!   colour baked from USDA NAIP imagery on the surveyed plan, BC7 sRGB with
//!   its mips, in `<cache>/hills/aerial-macro-v1.bin`.
//! * [`naip`]: the NAIP tile grid and manifest `native/tools/fetch-aerial.mjs`
//!   writes (the grid of `eo-kit/examples/roof_survey.rs`), and the survey's haze.
//! * [`raster`]: plan polygons (footprints, greens, roads) onto a texel grid.
//! * [`transfer`]: the macro map's classified colour transfer.
//! * [`planting`]: the vegetation planting's NAIP rules counted per 2 m
//!   texel citywide (stage H6a), in `<cache>/hills/planting-v1.bin`.
//! * [`relief`]: the ground's relief at 2 m from the USGS 1 m DEM (stage H3),
//!   in `<cache>/hills/relief-v1.bin`.
//! * [`trees`]: the hill parks' trees measured from the USGS 3DEP lidar
//!   (stage H6c: position, height, crown), in `<cache>/hills/trees-v1.bin`.
//! * `cook decks` (stage H4) writes the roof decks the bare-earth DEM keeps
//!   as ground into `native/assets/sf-decks-v1.json` (read by eo-data `decks`).

pub mod macro_map;
pub mod naip;
pub mod planting;
pub mod raster;
pub mod relief;
pub mod transfer;
pub mod trees;

use std::path::PathBuf;

/// The cooked-data folder: `EO_NATIVE_CACHE`, else `native/cache`. The rule
/// of `eo_assets::default_cache_dir`, repeated here so the hill builder does
/// not depend on the model cooker.
pub fn cache_dir() -> PathBuf {
    if let Ok(p) = std::env::var("EO_NATIVE_CACHE") {
        return p.into();
    }
    eo_data::native_dir().join("cache")
}

/// Worker threads for the cooks' parallel loops (two cores left free).
pub fn threads() -> usize {
    std::thread::available_parallelism().map(|v| v.get()).unwrap_or(8).saturating_sub(2).max(1)
}

/// Run `f(first_row, rows)` over `data` (rows of `row_len` items) split into
/// one contiguous band of rows per thread.
pub fn par_rows<T: Send>(data: &mut [T], row_len: usize, f: impl Fn(usize, &mut [T]) + Sync) {
    if data.is_empty() || row_len == 0 {
        return;
    }
    let rows = data.len() / row_len;
    let per = rows.div_ceil(threads()).max(1);
    std::thread::scope(|s| {
        for (k, chunk) in data.chunks_mut(per * row_len).enumerate() {
            let f = &f;
            s.spawn(move || f(k * per, chunk));
        }
    });
}
