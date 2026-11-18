//! Earth Online native (landmarks): the browser game's hand-built landmarks
//! and the city's built edges and open spaces as plain meshes, ported from
//! its generators. No GPU; `eo-render::landmarks` draws what this builds.
//!
//! | Rust | JS source of truth (branch) |
//! | --- | --- |
//! | `geometry` | src/world/landmarks/landmark-geometry.js (main) |
//! | `suspension` | src/world/landmarks/suspension-bridge.js (main) |
//! | `golden_gate` | golden-gate-bridge.js, golden-gate-bridge-site.js (main) |
//! | `bay_bridge` | bay-bridge-west.js, bay-bridge-west-site.js (main) |
//! | `set` | landmark-registry.js, landmark-generators.js `buildLandmark` (draws merged by `draw`), landmark-set.js's near/far rules (main) |
//!
//! Every JS design value is read from `data/landmarks-js.json`
//! (the earlier browser version's exporter); every California street value
//! (the decks' barrier and markings) through `eo_data::CaStandards`.

pub mod bay_bridge;
pub mod data;
pub mod features;
pub mod ferry_building;
pub mod geometry;
pub mod golden_gate;
pub mod ground_cover;
pub mod lights;
pub mod palms;
pub mod plaza;
pub mod promenade;
pub mod rocks; // the shore's boulders and the seawall's armour stone
pub mod set;
pub mod shore;
pub mod suspension;
pub mod towers;
pub mod tram;
pub mod vegetation; // the coastal tree stands, ground vegetation and hill rock (empty for now)
pub mod waterfront;

pub use data::LandmarksJs;
pub use geometry::{Frozen, GroundGrid, MeshArrays};
pub use set::{Draw, Landmark, LandmarkSet, Role};

/// A collider box (`{ part, x, z, y0, y1, hx, hz, yaw }`).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Collider {
    pub part: String,
    pub x: f64,
    pub z: f64,
    pub y0: f64,
    pub y1: f64,
    pub hx: f64,
    pub hz: f64,
    pub yaw: f64,
}

/// What a generator returns (`{ id, version, groups, far, skirt, colliders, stats }`).
#[derive(Clone, Debug)]
pub struct Built {
    pub id: String,
    pub version: String,
    /// In the generator's own key order.
    pub groups: Vec<(String, Frozen)>,
    pub far: Frozen,
    pub skirt: Frozen,
    pub colliders: Vec<Collider>,
    pub stats: serde_json::Value,
}

/// A light a landmark carries, in the record layout of the street lamps
/// (`eo_props::furniture::LampLight`, field for field, without the props'
/// lamp `Model`), so the night lighting gathers both the same way; on the GPU
/// it is `eo_render::props::GpuLampLight`. The JS bridges carry none (no
/// tower, aviation, cable or deck light in landmark-registry.js or the
/// generators), so the set's list is empty until a JS source adds one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LandmarkLight {
    /// The luminaire; the source sits `source_lift_m` above it.
    pub pos: [f64; 3],
    /// What it stands on (a tower leg's foot, a deck).
    pub base: [f64; 3],
    pub colour_linear: [f32; 3],
    /// Candela.
    pub intensity: f32,
    /// Windowed cutoff, metres from the lifted source.
    pub range_m: f32,
    pub decay: f32,
    /// Cone half-angle (rad; pi for a point light) and penumbra fraction.
    pub angle: f32,
    pub penumbra: f32,
    pub source_lift_m: f32,
}
