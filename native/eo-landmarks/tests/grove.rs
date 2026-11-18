//! The groves round Coit and Sutro towers (grove.rs), on the repo's data.

use eo_landmarks::grove;

/// Each site plants a wood of its own species on the mapped greens, clear
/// of its tower, of the car parks and of the ways, inside its triangle budget.
#[test]
fn the_groves_stand_on_the_mapped_woods() {
    let paths = eo_data::DataPaths::discover().unwrap();
    let elev = eo_data::ElevationGrid::load(&paths).unwrap();
    let ca = eo_data::CaStandards::load(&paths).unwrap();
    let height_at = |x: f64, z: f64| elev.sample(x, z);
    let (groves, lights) = grove::build_all(&paths.repo_root, &height_at, &ca).unwrap();
    assert!(lights.is_empty());
    assert_eq!(groves.len(), grove::SITES.len(), "every site plants trees");
    let s = eo_data::groundscale::scale();
    let js = eo_landmarks::LandmarksJs::get();
    let lots = grove::load_lots(&paths.repo_root).unwrap();
    for (site, l) in grove::SITES.iter().zip(&groves) {
        let a = &js.registry[site.landmark].anchor;
        let c = [a["x"].as_f64().unwrap() * s, a["z"].as_f64().unwrap() * s];
        let n = l.stats["trees"].as_u64().unwrap();
        let crowns = l.draws.iter().find(|d| d.key == "crowns").unwrap();
        let far = l.draws.iter().find(|d| d.key == "far").unwrap();
        eprintln!("{}: {} trees {:?}, {} crown triangles, {} far", site.id, n, l.stats["species"], crowns.mesh.triangles(), far.mesh.triangles());
        assert!(n > 150, "{}: only {n} trees", site.id);
        assert!(crowns.mesh.triangles() < 900_000, "{}: crown triangles", site.id);
        // the trunks' feet: every trunk is 20 vertices, its first at its foot ring.
        let trunks = l.draws.iter().find(|d| d.key == "trunks").unwrap();
        for p in trunks.mesh.positions.chunks_exact(3 * 20) {
            let (x, z) = (p[0] as f64, p[2] as f64);
            let r = (x - c[0]).hypot(z - c[1]);
            assert!(r > site.keep_out_m - 1.0 && r < site.reach_m + 1.0, "{}: a trunk {r} m from the tower", site.id);
            assert!(!lots.iter().any(|lot| eo_landmarks::waterfront::point_in_ring(lot, x, z)), "{}: a trunk in a car park at {x}, {z}", site.id);
        }
        assert!(l.box_max[1] > l.box_min[1] + 10.0, "{}: the box", site.id);
    }
}
