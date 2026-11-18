//! Earth Online's world simulation, headless: traffic on the lane graph,
//! signal control, walkers on the footway network, the population rules that
//! decide who exists near a focus, level of detail for far agents, a fixed
//! step on one clock, and compact per-agent instance buffers for a renderer.
//!
//! No rendering, no GPU, no window. The source of truth is the browser game's
//! simulation (src/citygen/traffic.js `TrafficSim`, src/simulation/traffic/,
//! src/simulation/pedestrians/, src/render/performance-settings.js); each
//! module names the JS it ports, and `tests/` holds the Rust to the JS:
//! exactly where the JS is a pure function (signal timing, turning plans,
//! movement classes, deterministic draws, walking-speed draws, the standards
//! the timing reads), statistically for the run as a whole.
//!
//! Layout:
//!   `network`   tile-keyed lanes, links, plans, junctions, footways
//!   `import_js` the shipped JS window, exported by the earlier browser version's exporter
//!   `native`    the same structures built from the extract (`sf-city.json.gz`), any tile
//!   `signals`   CA MUTCD phase plans and pedestrian windows
//!   `vehicles`  the driving model
//!   `pedestrians` the walker model
//!   `region`    one focus: population, level of detail, output
//!   `world`     the clock, the worker pool, the off-thread runner
//!   `output`    the 32-byte instance record

pub mod consts;
pub mod geom;
pub mod grid;
pub mod import_js;
pub mod native;
pub mod network;
pub mod output;
pub mod pedestrians;
pub mod region;
pub mod rng;
pub mod signals;
pub mod standards;
pub mod stats;
pub mod vehicles;
pub mod world;

pub use network::{Network, NetworkBuild, TileKey};
pub use output::{AgentBuffer, AgentInstance};
pub use region::{Focus, People, Region, RegionConfig};
pub use standards::{SignalTiming, Standards, StreetDims};
pub use world::{AsyncSim, World};

/// A network loaded with the shipped JS window (the oracle), ready to step.
pub fn window_network(paths: &eo_data::DataPaths) -> anyhow::Result<(Network, import_js::Oracle)> {
    let standards = Standards::load(paths)?;
    let timing = SignalTiming::from_standards(&standards)?;
    let dims = StreetDims::from_standards(&standards)?;
    let oracle = import_js::Oracle::load_default()?;
    let mut net = Network::new(timing);
    net.add(oracle.network_build(dims.city_lane_m));
    Ok((net, oracle))
}
