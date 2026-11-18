//! how well cheap estimates of a kit building's street-front floor
//! (the floor its near and mid rungs are planned at) agree with the real one,
//! for the far box. Every Nth kit building.
//!   cargo run --release -p eo-render --example far_floor_probe [N=40]

use eo_kit::city::KitCity;
use eo_render::kit::kit_floor;
use eo_street::{BuildingIndex, StreetCity, StreetOptions, StreetWorld, Terrain};
use std::sync::Arc;

fn pct(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((p / 100.0) * (v.len() - 1) as f64).round() as usize]
}

fn main() {
    let step: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(40);
    let paths = eo_data::DataPaths::discover().unwrap();
    let city = Arc::new(eo_data::City::load(&paths).unwrap());
    let elev = eo_data::ElevationGrid::load(&paths).unwrap();
    let o = StreetOptions::load().unwrap();
    let lift = o.open_ground_lift;
    let rings: Vec<Vec<[f64; 2]>> = city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
    let sw = Arc::new(StreetWorld::build(o, StreetCity::load().unwrap(), Terrain { grid: Arc::new(elev.clone()), carpet_lift: lift }, BuildingIndex::new(rings)));
    let tenancy = eo_kit::tenancy::Tenancy::load(&paths.repo_root).unwrap();
    let kc = KitCity::build(&city, &tenancy, &eo_kit::city::load_exclusions());
    let fl = sw.o.footway_lift;
    let names = ["lowest ground (today's box base)", "mean ring ground + footway lift", "centroid ground + footway lift", "street estimate"];
    let mut err: Vec<Vec<f64>> = vec![vec![]; names.len()];
    for b in (0..kc.records.len()).step_by(step) {
        let rec = &kc.records[b];
        let ring: Vec<[f64; 2]> = rec.polygon.iter().map(|p| [p.x, p.z]).collect();
        if ring.len() < 3 {
            continue;
        }
        let (floor, _, _) = kit_floor(&kc, &sw, b as u32);
        let g: Vec<f64> = ring.iter().map(|p| elev.ground(p[0], p[1])).collect();
        let lowest = g.iter().copied().fold(f64::MAX, f64::min);
        let mean = g.iter().sum::<f64>() / g.len() as f64;
        let c = ring.iter().fold([0.0, 0.0], |a, p| [a[0] + p[0], a[1] + p[1]]);
        let c = [c[0] / ring.len() as f64, c[1] / ring.len() as f64];
        let street = eo_render::kit::far_floor_estimate(&sw, &ring);
        for (k, e) in [lowest, mean + fl, elev.ground(c[0], c[1]) + fl, street].iter().enumerate() {
            err[k].push(e - floor.y);
        }
    }
    println!("{} kit buildings (every {step}th): estimate - street-front floor (m)", err[0].len());
    for (k, n) in names.iter().enumerate() {
        let mut a: Vec<f64> = err[k].iter().map(|e| e.abs()).collect();
        let mut s = err[k].clone();
        println!("  {n:40} |err| p50 {:.3} p95 {:.3} p99 {:.3} max {:.3}; signed p5 {:.3} p95 {:.3}", pct(&mut a, 50.0), pct(&mut a, 95.0), pct(&mut a, 99.0), pct(&mut a, 100.0), pct(&mut s, 5.0), pct(&mut s, 95.0));
    }
}
