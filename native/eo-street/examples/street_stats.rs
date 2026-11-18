//! Build the street world once and report what it holds and what it costs.
//!   cargo run --release -p eo-street --example street_stats

use eo_data::{City, DataPaths, ElevationGrid};
use eo_street::{BuildingIndex, StreetCity, StreetOptions, StreetWorld, Terrain};
use std::sync::Arc;
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    let paths = DataPaths::discover()?;
    let t = Instant::now();
    let city = City::load(&paths)?;
    let elevation = ElevationGrid::load(&paths)?;
    println!("city + elevation {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
    let t = Instant::now();
    let o = StreetOptions::load()?;
    let sc = StreetCity::load()?;
    println!("street city load {:.0} ms: {} segments, {} intersections", t.elapsed().as_secs_f64() * 1e3, sc.segments.len(), sc.intersections.len());
    let t = Instant::now();
    let rings = city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
    let bi = BuildingIndex::new(rings);
    println!("building index {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
    let lift = o.open_ground_lift;
    let w = StreetWorld::build(o, sc, Terrain { grid: Arc::new(elevation), carpet_lift: lift }, bi);
    println!(
        "street world {:.0} ms: {} runs, {} nodes ({} inferred, {} signalised), {} tiles",
        w.build_ms,
        w.plan.entries.len(),
        w.plan.nodes.len(),
        w.plan.nodes.iter().filter(|n| n.inferred).count(),
        w.plan.nodes.iter().filter(|n| n.signal.is_some()).count(),
        w.tiles.len()
    );
    let keys = w.tile_keys();
    let t = Instant::now();
    let mut tris = 0usize;
    let mut worst = (0.0f64, (0, 0), 0usize);
    let sample: Vec<_> = keys.iter().step_by(25).copied().collect();
    for k in &sample {
        let t1 = Instant::now();
        let m = w.tile_mesh(*k);
        let ms = t1.elapsed().as_secs_f64() * 1e3;
        tris += m.triangles();
        if ms > worst.0 {
            worst = (ms, *k, m.triangles());
        }
    }
    let ms = t.elapsed().as_secs_f64() * 1e3;
    println!("{} sample tiles: {:.2} ms/tile, {:.0} triangles/tile; worst {:.1} ms at {:?} ({} tris)", sample.len(), ms / sample.len() as f64, tris as f64 / sample.len() as f64, worst.0, worst.1, worst.2);
    let t = Instant::now();
    let mut ftris = 0;
    for k in &keys {
        ftris += w.far_tile_mesh(*k).triangles();
    }
    println!("far level, all {} tiles: {:.0} ms, {} triangles", keys.len(), t.elapsed().as_secs_f64() * 1e3, ftris);
    Ok(())
}
