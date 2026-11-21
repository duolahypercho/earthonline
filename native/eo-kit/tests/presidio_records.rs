//! The Presidio rule's documented records (presidio.rs `OVERRIDES`) are on the
//! post in the shipped city and draw their documented finishes.

use eo_kit::city::KitCity;
use eo_kit::pipeline::{drawn_materials, plan_building};
use eo_kit::presidio;

#[test]
fn every_documented_record_is_on_the_post_and_draws_its_finish() {
    eo_kit::wall_paint::pin(true);
    presidio::pin(true);
    let root = eo_kit::repo_root().expect("repo root");
    let (city, kc) = KitCity::load(&root).expect("city");
    let ring = presidio::boundary(&city.meta);
    for (id, finish, why) in presidio::OVERRIDES {
        let i = kc.records.iter().position(|r| r.id == id).unwrap_or_else(|| panic!("{id} ({why}) is not in the city"));
        let rec = &kc.records[i];
        assert!(rec.presidio, "{id} is not on the post");
        let n = rec.polygon.len() as f64;
        let (x, z) = rec.polygon.iter().fold((0.0, 0.0), |a, p| (a.0 + p.x, a.1 + p.z));
        assert!(presidio::on_post(&ring, id, x / n, z / n));
        let planned = plan_building(rec, &kc.world, i as u32, 0.0).expect("plans");
        assert!(planned.classified.presidio, "{id} has an assessor join");
        let drawn = drawn_materials(&planned, 0);
        assert!(drawn.materials.values().any(|m| m.starts_with(finish)), "{id}: no {finish} in {:?}", drawn.materials);
        assert_eq!(drawn.materials.get("roof").map(String::as_str), Some(presidio::RED_TILE), "{id}: roof {:?}", drawn.materials);
    }
}
