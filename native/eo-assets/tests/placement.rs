//! Placement of the Blender-built buildings against what the JS computes
//! (the earlier browser version's exporter runs authoredFootprintFrame on
//! the reflected record and streetFrontGrades with the game's elevation
//! sampler, in node).

use eo_assets::manifest::load_blender_buildings;
use eo_assets::placement::{PlacementConstants, place};
use glam::{DVec2, DVec3};

#[test]
fn placement_matches_the_js_registry() {
    eo_data::groundscale::pin(1.0); // the JS game this compares with is 1:1
    let paths = eo_data::DataPaths::discover().unwrap();
    let city = eo_data::City::load(&paths).unwrap();
    let elev = eo_data::ElevationGrid::load(&paths).unwrap();
    let c = PlacementConstants::load().unwrap();
    let list = load_blender_buildings(&paths.repo_root).unwrap();
    let by_key: std::collections::HashMap<&str, &eo_data::Building> =
        city.buildings.iter().map(|b| (b.id.as_str(), b)).collect();
    let height_at = |x: f64, z: f64| elev.sample(x, z);
    assert!(c.samples.len() >= 30);
    let mut worst_frame = 0.0f64;
    let mut worst_ground = (0.0f64, String::new());
    let mut placed = 0;
    for s in &c.samples {
        let b = list.iter().find(|b| b.id == s.id).unwrap();
        let rec = by_key.get(b.record_key()).unwrap_or_else(|| panic!("{} has no record in the merged city", s.id));
        let ring: Vec<DVec2> = rec.record_polygon().iter().map(|p| DVec2::new(p[0] as f64, p[1] as f64)).collect();
        let p = place(b, &ring, rec.height as f64, &c, &height_at).unwrap();
        placed += 1;
        let f = &s.frame;
        for (got, want, what) in [
            (p.frame.center.x, f.center_x, "centre x"),
            (p.frame.center.y, f.center_z, "centre z"),
            (p.frame.width, f.width, "width"),
            (p.frame.depth, f.depth, "depth"),
            (p.frame.heading, f.heading, "heading"),
        ] {
            let d = (got - want).abs();
            worst_frame = worst_frame.max(d);
            // The ring is stored as f32 (0.12 mm at x ~ 1600 m).
            assert!(d < 1e-3, "{} {what}: {got} vs JS {want}", s.id);
        }
        if let Some(js_base) = s.base_y {
            let d = (p.base_y - js_base).abs();
            if d > worst_ground.0 {
                worst_ground = (d, s.id.clone());
            }
            // The JS sampler adds the bridge-approach fills; this one reads
            // the grid as written (see the README), so allow 5 cm.
            assert!(d < 0.05, "{} baseY {} vs JS {js_base}", s.id, p.base_y);
        }
        // The frame origin lands at the JS world position of the reflected root:
        // (centre.x, baseY, -centre.z).
        let o = p.world_from_model.transform_point3(DVec3::ZERO);
        assert!((o - DVec3::new(f.center_x, p.base_y, -f.center_z)).length() < 1e-3);
        // Mirror: determinant -1.
        assert!((p.world_from_model.determinant() + 1.0).abs() < 1e-9);
        // +Z of the model (out of the front edge) points away from the footprint
        // centroid in the world, through the front edge's midpoint side.
        let out = p.world_from_model.transform_vector3(DVec3::Z);
        let n = ring.len();
        let (a, bb) = (ring[b.frame.edge_index], ring[(b.frame.edge_index + 1) % n]);
        let mid = (a + bb) / 2.0;
        let cen = ring.iter().fold(DVec2::ZERO, |s, q| s + *q) / n as f64;
        assert!((mid - cen).dot(DVec2::new(out.x, out.z)) > 0.0, "{}: model +Z does not face the front edge", s.id);
    }
    eprintln!(
        "{placed} sampled buildings placed; worst frame error {worst_frame:.2e} m; worst baseY difference {:.4} m ({})",
        worst_ground.0, worst_ground.1
    );
}
