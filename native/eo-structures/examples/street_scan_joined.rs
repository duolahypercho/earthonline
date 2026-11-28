//! The street geometry scorecard (eo-street `street_scan`) on the street as
//! `--play` builds it: the street world built again with the grade-separated
//! roads' joins (stage A6, `build_joined`, eo-app's `--structures`), the
//! structures' cuts left open in the street ground as eo-app leaves them.
//!
//! `cargo run --release -p eo-structures --example street_scan_joined -- [hits.csv] [--tunnels]`

use std::sync::Arc;

use eo_street::{BuildingIndex, StreetCity, StreetOptions, StreetWorld, Terrain};
use eo_structures::StructureOptions;

#[path = "../../eo-street/examples/street_scan.rs"]
mod street_scan;

fn main() -> anyhow::Result<()> {
    let paths = eo_data::DataPaths::discover()?;
    let city = eo_data::City::load(&paths)?;
    let grid = Arc::new(eo_data::ElevationGrid::load(&paths)?);
    let o = StreetOptions::load()?;
    let lift = o.open_ground_lift;
    let rings = city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
    let streets = StreetWorld::build(o, StreetCity::load()?, Terrain { grid, carpet_lift: lift }, BuildingIndex::new(rings));
    let ca = eo_data::CaStandards::load(&paths)?;
    let tunnels = std::env::args().any(|a| a == "--tunnels");
    let j = eo_structures::build_joined(&streets, &city, &ca, eo_landmarks::LandmarksJs::get(), StructureOptions { tunnels })?;
    let w = j.streets.unwrap_or(streets);
    if !j.structures.is_empty() {
        let s = Arc::new(j.structures);
        w.set_ground_cut(Arc::new(move |x, z| s.terrain_cut_at(x, z).map(|c| c.floor)));
    }
    println!("street as --play builds it: with the structures' joins (tunnels {})", if tunnels { "on" } else { "off" });
    street_scan::scan(&w, std::env::args().skip(1).find(|a| !a.starts_with("--")));
    Ok(())
}
