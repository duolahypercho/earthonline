//! Assets of the native slice that come from the game's hand-built buildings:
//! the Blender building manifest, hash-verified GLB loading and cooking, the
//! binary model cache, the authored kit's materials, and placement.

pub mod cache;
pub mod cc0; // the CC0 texture sets read from outside the repository
pub mod facade_detail; // facade
pub mod glb;
pub mod jpeg;
pub mod kit;
pub mod street_maps;
pub mod manifest;
pub mod placement;

pub use cache::{LoadSource, load_model};
pub use glb::{CookedMaterial, CookedModel, CookedPart, GlbCounts};
pub use kit::{KitLibrary, KitSpec};
pub use manifest::{AssetRef, BlenderBuilding, load_blender_buildings};
pub use placement::{Placement, PlacementConstants};

/// `native/cache` beside this crate (git-ignored), or `EO_NATIVE_CACHE`.
pub fn default_cache_dir() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("EO_NATIVE_CACHE") {
        return p.into();
    }
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../cache")
}
