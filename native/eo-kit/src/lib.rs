//! Earth Online native slice 3: a Rust port of the browser game's archetype
//! ("kit") building planner.
//!
//! From one source record (footprint ring, height, use tags, the stable id
//! that seeds it) to the same archetype, storeys, bay rhythm and material
//! choices the JS makes, and to triangles per material for each LOD rung.
//! Deterministic: the same record gives the same bytes on every run and thread.
//!
//!   classify.rs    classify.js (v3 with the district/era prior) + sf-data.js program/usage
//!   frame.rs       kit/frame.js footprintFrame
//!   street.rs      street-front.js (front edge, street returns, lot-line walls)
//!   elevation.rs   kit/elevation.js planElevation
//!   archetypes/    the eleven archetype modules (spec + detail)
//!   shop_ground.rs, roof_plant.rs, row_house.rs   the factory's helper modules
//!   pipeline.rs    create-archetype-building.js (factory passes, materials, paint, roof finish)
//!   lod.rs         lod.js (rungs 1-2 filters, wall-band collapse, envelope)
//!   mesh.rs        kit/build.js emitters (box, profile, plate, prism; vertex clamp)
//!   tenancy.rs     building-metadata ground-floor units
//!   clutter.rs     facade clutter (fire escapes, awnings, signs, services, roofs; off by default)

pub mod appearance; // facade
pub mod archetypes;
pub mod city;
pub mod classify;
pub mod clutter; // facade clutter (`--facade-clutter`): fire escapes, awnings, signs, services, roof clutter
pub mod elevation;
pub mod far_facade; // the far box's facade from the archetype's own spec
pub mod frame;
pub mod js;
pub mod lod;
pub mod mesh;
pub mod pipeline;
pub mod presidio; // the Presidio and GGNRA rule (off-white walls, red tile roofs)
pub mod rng;
pub mod roof_measured; // roof finishes tinted to the measured aerial colour
pub mod roof_plant;
pub mod row_house;
pub mod shop_ground;
pub mod street;
pub mod tenancy;
pub mod typology; // the window module and a tower's facade system per building
pub mod wall_paint; // the wall paint per building on a neutral primer

/// The kit's rule switches for the tile cache key (typology, the measured
/// roofs' neutral rule; the wall paint has its own `wall_paint::cache_tag`):
/// empty with all of them off.
pub fn rules_cache_tag() -> String {
    [typology::cache_tag(), roof_measured::cache_tag()].into_iter().filter(|t| !t.is_empty()).collect::<Vec<_>>().join("+")
}

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

/// Version of the planner port: part of every cooked tile's cache key.
pub const KIT_PORT_VERSION: &str = "eo-kit-port-v1";
/// Digest of this crate's sources and data at build time (build.rs).
pub const KIT_SOURCE_HASH: &str = env!("KIT_SOURCE_HASH");

/// The repo root: `EO_REPO_ROOT`, or the first ancestor of the working
/// directory (or of the executable, or of this crate) holding `public/data/sf`.
pub fn repo_root() -> Option<PathBuf> {
    static ROOT: OnceLock<Option<PathBuf>> = OnceLock::new();
    ROOT.get_or_init(|| {
        if let Ok(r) = std::env::var("EO_REPO_ROOT") {
            return Some(PathBuf::from(r));
        }
        let starts = [std::env::current_dir().ok(), std::env::current_exe().ok(), Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")))];
        for start in starts.into_iter().flatten() {
            let mut d: Option<&std::path::Path> = Some(&start);
            while let Some(p) = d {
                if p.join("public/data/sf").is_dir() {
                    return Some(p.to_path_buf());
                }
                d = p.parent();
            }
        }
        None
    })
    .clone()
}

/// Kit material name -> spec kind (`glass`, `brick`, ...), from the exported
/// registry native/eo-assets/data/kit-material-specs.json.
pub fn kit_material_kinds() -> &'static HashMap<String, String> {
    static KINDS: OnceLock<HashMap<String, String>> = OnceLock::new();
    KINDS.get_or_init(|| {
        let path = eo_data::native_dir().join("eo-assets/data/kit-material-specs.json");
        let Ok(text) = std::fs::read_to_string(path) else { return HashMap::new() };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return HashMap::new() };
        v["specs"]
            .as_object()
            .map(|m| m.iter().map(|(k, s)| (k.clone(), s["kind"].as_str().unwrap_or("").to_string())).collect())
            .unwrap_or_default()
    })
}
