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

pub mod access;
pub mod access_sky;
pub mod bay_bridge;
pub mod bridge_roads;
pub mod data;
pub mod features;
pub mod flora2; // the second flora pass: the Battery Spencer knoll's bare chert, the Baker bluff's serpentinite rock and boulders
pub mod flora; // the flora pass: the scrub by species, the grassland's season, chert and serpentinite ground, flowers
pub mod ferry_building;
pub mod floating_docks;
pub mod geometry;
pub mod hill_rock; // the hill rock outcrops and blocks (the coarse vegetation layer's third part)
pub mod golden_gate;
pub mod ground_cover;
pub mod grove; // the groves round Coit and Sutro towers
pub mod lights;
pub mod palms;
pub mod plaza;
pub mod promenade;
pub mod rocks; // the shore's boulders and the seawall's armour stone
pub mod set;
pub mod shore;
pub mod skyline; // the named skyline towers
pub mod suspension;
pub mod towers;
pub mod towers2; // the landmark buildings' second generators
pub mod towers3; // their third pass (projector floodlights, the base truss, the fluted shaft, the lattice) and fourth (`towers4_on`)
pub mod tram;
pub mod vegetation; // the coastal tree stands, ground vegetation and hill rock (empty for now)
pub mod waterfront;

pub use data::LandmarksJs;

/// The landmark buildings' and skyline towers' third pass (towers2.rs and
/// skyline.rs under `v3`; eo-render's projector floodlights, glazing and
/// night windows): on unless `eo-app --no-towers3` set `EO_TOWERS3=off`
/// before any thread started, which restores the previous output.
pub fn towers3_on() -> bool {
    std::env::var("EO_TOWERS3").map(|v| v != "off").unwrap_or(true)
}

/// The fourth pass (towers3.rs, skyline.rs and grove.rs under `v4`;
/// eo-render's facade relief shadows, spandrels and LED field): on unless
/// `eo-app --no-towers4` set `EO_TOWERS4=off` before any thread started
/// (and only with the third pass), which restores the third pass's output.
pub fn towers4_on() -> bool {
    towers3_on() && std::env::var("EO_TOWERS4").map(|v| v != "off").unwrap_or(true)
}

/// Which pass the switches leave on: 2 (`--no-towers3`), 3 (`--no-towers4`) or 4.
pub fn towers_level() -> u8 {
    if !towers3_on() {
        2
    } else if !towers4_on() {
        3
    } else {
        4
    }
}
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
