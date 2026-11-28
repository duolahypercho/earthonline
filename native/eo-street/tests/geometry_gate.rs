//! The street geometry gate: the scorecard (examples/street_scan.rs, the
//! citywide `street_scan`) on a sample of real tiles, every 8th street tile
//! of the city (about 1,300) and the 3+-leg junctions in them, held to the
//! rates the street world reached with its ground, seams, junction pads and
//! walk yields (2026-11-27), so a change that brings back the mess fails here
//! instead of in a capture:
//!
//!   stacked paving   two walkable surfaces 0.2-4 m apart at one point (two
//!                    runs drawn at their own datums where they overlap)
//!   ground poking    drawn ground more than 5 cm over the paving
//!   cliff edges      a paving edge more than 2 m over the open ground past it
//!   sunken gaps      open ground enclosed by paving, more than 15 cm under it
//!   junction holes   no paving inside a junction pad's own ring
//!   junction steps   the carriageway's top breaking from its slope in a pad
//!   junction stacks  stacked paving in a pad
//!
//! Each a rate per km2 of the sample's paving (junctions: per junction), with
//! a margin over the rates measured (`LIMITS`); with the street switches off
//! (`--no-street-ground`, `--no-junction-ground`, `--no-road-seams`, ...)
//! six of the seven fail, the stacked paving at 3.5 times its limit. Lower a
//! limit when the street gets better; never raise one to pass. (Release
//! build: about two minutes, the street ground's 2 m lattice most of it.)

use eo_data::{City, DataPaths, ElevationGrid};
use eo_street::{BuildingIndex, StreetCity, StreetOptions, StreetWorld, Terrain};
use std::sync::Arc;

#[path = "../examples/street_scan.rs"]
#[allow(dead_code)]
mod street_scan;

/// (measure, limit): m2 or m per km2 of paving, or per junction. Measured
/// 2026-11-27 on this sample (6.44 km2 of paving, 1,294 junctions), with the
/// street switches on / all off: stacked paving 2165 / 8283, ground poking
/// 291 / 503, cliff edges 1664 / 3084, sunken gaps 1326 / 2849, junction
/// holes 0 / 0, steps 64 / 101, junction stacks 2.98 / 9.48. The limits are
/// those rates and about a tenth (no hole at all).
const LIMITS: &[(&str, f64)] = &[
    ("stacked paving m2/km2", 2400.0),
    ("ground poking m2/km2", 330.0),
    ("cliff edges m/km2", 1850.0),
    ("sunken gaps m2/km2", 1460.0),
    ("junction holes per 1000 junctions", 0.5),
    ("junction steps per 1000 junctions", 72.0),
    ("junction stacks m2 per junction", 3.3),
];

#[test]
fn the_street_geometry_holds_its_scorecard() {
    let paths = DataPaths::discover().unwrap();
    let city = City::load(&paths).unwrap();
    let elev = ElevationGrid::load(&paths).unwrap();
    let o = StreetOptions::load().unwrap();
    let lift = o.open_ground_lift;
    let rings = city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
    let w = StreetWorld::build(o, StreetCity::load().unwrap(), Terrain { grid: Arc::new(elev), carpet_lift: lift }, BuildingIndex::new(rings));
    let keys: Vec<(i32, i32)> = w.tile_keys().into_iter().step_by(8).collect();
    let c = street_scan::measure(&w, &keys, true);
    let km2 = c.t.paving_m2 / 1e6;
    let per_j = |v: f64| v / c.junctions.max(1) as f64;
    let measured = [
        c.t.stack_m2 / km2,
        c.t.poking_m2 / km2,
        c.t.cliff2_m / km2,
        c.t.sunk_m2 / km2,
        1000.0 * per_j(c.junction_hole as f64),
        1000.0 * per_j(c.junction_step as f64),
        per_j(c.junction_stack_m2),
    ];
    println!("sample: {} tiles, {:.2} km2 of paving, {} junctions", c.tiles, km2, c.junctions);
    let mut over = vec![];
    for (&(name, limit), &v) in LIMITS.iter().zip(&measured) {
        println!("  {name:36} {v:10.2}   limit {limit:10.2}");
        if v > limit {
            over.push(format!("{name}: {v:.2} over its limit {limit:.2}"));
        }
    }
    assert!(over.is_empty(), "the street geometry regressed (scorecard: cargo run --release -p eo-street --example street_scan -- hits.csv):\n{}", over.join("\n"));
}
