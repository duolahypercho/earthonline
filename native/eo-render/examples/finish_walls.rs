//! find, near a point, one kit building's street wall per finish class
//! (the wall material its near rung draws) and print a close camera for it
//! (6 m out from the middle of its street face, square to it).
//!   cargo run --release -p eo-render --example finish_walls -- [x z radius]

use eo_kit::city::KitCity;
use eo_render::kit::kit_floor;
use eo_street::{BuildingIndex, StreetCity, StreetOptions, StreetWorld, Terrain};
use std::collections::BTreeMap;
use std::sync::Arc;

fn main() {
    let a: Vec<f64> = std::env::args().skip(1).filter_map(|s| s.parse().ok()).collect();
    let (ax, az, radius) = if a.len() >= 3 { (a[0], a[1], a[2]) } else { (-1350.0, 2150.0, 700.0) };
    let paths = eo_data::DataPaths::discover().unwrap();
    let city = Arc::new(eo_data::City::load(&paths).unwrap());
    let elev = eo_data::ElevationGrid::load(&paths).unwrap();
    let o = StreetOptions::load().unwrap();
    let lift = o.open_ground_lift;
    let rings: Vec<Vec<[f64; 2]>> = city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
    let sw = Arc::new(StreetWorld::build(o, StreetCity::load().unwrap(), Terrain { grid: Arc::new(elev.clone()), carpet_lift: lift }, BuildingIndex::new(rings)));
    let tenancy = eo_kit::tenancy::Tenancy::load(&paths.repo_root).unwrap();
    let kc = KitCity::build(&city, &tenancy, &eo_kit::city::load_exclusions());
    let mut best: BTreeMap<String, (f64, String)> = BTreeMap::new();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for b in 0..kc.records.len() {
        if kc.excluded.contains(&b) {
            continue;
        }
        let rec = &kc.records[b];
        if rec.polygon.len() < 3 {
            continue;
        }
        let c = rec.polygon.iter().fold((0.0, 0.0), |s, p| (s.0 + p.x, s.1 + p.z));
        let (cx, cz) = (c.0 / rec.polygon.len() as f64, c.1 / rec.polygon.len() as f64);
        let d = ((cx - ax).powi(2) + (cz - az).powi(2)).sqrt();
        if d > radius {
            continue;
        }
        let (floor, _, _) = kit_floor(&kc, &sw, b as u32);
        let Some(p) = eo_kit::pipeline::plan_building(rec, &kc.world, b as u32, floor.y) else { continue };
        if p.frame.front_source != "street" {
            continue;
        }
        let dm = eo_kit::pipeline::drawn_materials(&p, 0);
        let Some(wall) = dm.materials.get("wall").cloned() else { continue };
        let tint = dm.tints.get(&wall).map(|t| format!("#{t:06x}")).unwrap_or_else(|| "-".into());
        *counts.entry(wall.clone()).or_default() += 1;
        let e = &p.frame.edges[p.frame.front_edge_index];
        if e.length < 7.0 {
            continue;
        }
        // Outward: the side of the edge away from the footprint.
        let mut n = (e.n.x, e.n.z);
        if eo_kit::frame::point_in_polygon(e.midpoint.x + n.0 * 0.5, e.midpoint.z + n.1 * 0.5, &rec.polygon) {
            n = (-n.0, -n.1);
        }
        let eye = (e.midpoint.x + n.0 * 6.0, e.midpoint.z + n.1 * 6.0);
        let yaw = (-n.0).atan2(n.1); // look along -n: yaw = atan2(dx, -dz)
        let line = format!(
            "{wall:28} tint {tint:8} building {b:6} {} archetype {:18} edge {:.1} m, eye ({:.2}, {:.2}) floor {:.2} yaw {:.4}, {:.0} m from the anchor",
            rec.id, p.classified.archetype, e.length, eye.0, eye.1, floor.y, yaw, d
        );
        let score = d;
        if best.get(&wall).map(|v| score < v.0).unwrap_or(true) {
            best.insert(wall, (score, line));
        }
    }
    for (k, n) in &counts {
        println!("count {k:28} {n}");
    }
    for (_, (_, l)) in best {
        println!("{l}");
    }
}
