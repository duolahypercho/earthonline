//! why one kit building's skirt foot stands above the drawn surface.
//!   cargo run --release -p eo-render --example float_probe <building id>
use eo_kit::city::KitCity;
use eo_render::kit::kit_floor;
use eo_street::{BuildingIndex, StreetCity, StreetOptions, StreetWorld, Terrain};
use std::sync::Arc;

fn main() {
    let id = std::env::args().nth(1).unwrap_or_else(|| "ds92578".into());
    let paths = eo_data::DataPaths::discover().unwrap();
    let city = Arc::new(eo_data::City::load(&paths).unwrap());
    let elev = eo_data::ElevationGrid::load(&paths).unwrap();
    let o = StreetOptions::load().unwrap();
    let lift = o.open_ground_lift;
    let rings: Vec<Vec<[f64; 2]>> = city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
    let sw = Arc::new(StreetWorld::build(o, StreetCity::load().unwrap(), Terrain { grid: Arc::new(elev.clone()), carpet_lift: lift }, BuildingIndex::new(rings.clone())));
    let tenancy = eo_kit::tenancy::Tenancy::load(&paths.repo_root).unwrap();
    let kc = KitCity::build(&city, &tenancy, &eo_kit::city::load_exclusions());
    let i = city.buildings.iter().position(|b| b.id == id).expect("id");
    let (floor, f, front) = kit_floor(&kc, &sw, i as u32);
    let ring = &rings[i];
    let g = eo_street::grounding::measure(&sw, ring, floor.y, f.bottom_y, front.as_ref(), floor.clamped);
    let kring: Vec<[f64; 2]> = kc.records[i].polygon.iter().map(|p| [p.x, p.z]).collect();
    println!("floor {:.3} bottom {:.3} support(kit ring) {:.3} support(city ring) {:.3} float gap {:.3}", floor.y, f.bottom_y, f.support_min, sw.support_min(ring), g.float_gap);
    println!("kit ring {} pts, city ring {} pts, first {:?} vs {:?}", kring.len(), ring.len(), kring.first(), ring.first());
}
