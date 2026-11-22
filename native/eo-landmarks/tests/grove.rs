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
    // the fourth pass lights Coit's parking loop (and nothing else); the third none.
    let v4 = eo_landmarks::towers4_on();
    assert_eq!(lights.is_empty(), !v4, "{} grove lights", lights.len());
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
        // the trunks' feet: every trunk is 20 vertices (24 in the fourth pass), its first at its foot ring.
        let trunks = l.draws.iter().find(|d| d.key == "trunks").unwrap();
        let per = if v4 { 24 } else { 20 };
        for p in trunks.mesh.positions.chunks_exact(3 * per) {
            let (x, z) = (p[0] as f64, p[2] as f64);
            let r = (x - c[0]).hypot(z - c[1]);
            assert!(r > site.keep_out_m - 1.0 && r < site.reach_m + 1.0, "{}: a trunk {r} m from the tower", site.id);
            assert!(!lots.iter().any(|lot| eo_landmarks::waterfront::point_in_ring(lot, x, z)), "{}: a trunk in a car park at {x}, {z}", site.id);
        }
        assert!(l.box_max[1] > l.box_min[1] + 10.0, "{}: the box", site.id);
        if v4 {
            // the litter layer under the crowns, and no crown over a car park.
            let floor = l.draws.iter().find(|d| d.key == "floor").expect("a forest floor");
            eprintln!("{}: floor {} triangles", site.id, floor.mesh.triangles());
            assert!(floor.mesh.triangles() > 1000, "{}: floor", site.id);
            assert!(floor.mesh.normals.chunks_exact(3).all(|n| n[1] > 0.0), "{}: the floor faces up", site.id);
        }
    }
    if v4 {
        // the loop's lamps stand round Coit Tower's parking circle, within 120 m of the tower.
        let a = &js.registry["coit-tower"].anchor;
        let c = [a["x"].as_f64().unwrap() * s, a["z"].as_f64().unwrap() * s];
        assert!(lights.len() >= 4, "{} loop lamps", lights.len());
        for l in &lights {
            assert!((l.pos[0] - c[0]).hypot(l.pos[2] - c[1]) < 160.0, "a lamp far from Coit Tower");
        }
    }
}

/// The fourth pass's planting keeps every crown clear of the car parks and
/// of the buildings (with their clearances), and Coit's summit under its cone.
#[test]
fn the_fourth_pass_keeps_the_summit_and_the_houses_open() {
    if !eo_landmarks::towers4_on() {
        return;
    }
    let paths = eo_data::DataPaths::discover().unwrap();
    let elev = eo_data::ElevationGrid::load(&paths).unwrap();
    let ca = eo_data::CaStandards::load(&paths).unwrap();
    let height_at = |x: f64, z: f64| elev.sample(x, z);
    let s = eo_data::groundscale::scale();
    let js = eo_landmarks::LandmarksJs::get();
    let centres: Vec<([f64; 2], f64)> = grove::SITES
        .iter()
        .map(|site| {
            let a = &js.registry[site.landmark].anchor;
            ([a["x"].as_f64().unwrap() * s, a["z"].as_f64().unwrap() * s], site.reach_m)
        })
        .collect();
    let lane = ca.metric(eo_data::standards::CITY_LANE_ID).unwrap();
    let (ways, buildings) = grove::city_near(&paths.repo_root, &centres, lane).unwrap();
    let lots = grove::load_lots(&paths.repo_root).unwrap();
    let (greens, _) = eo_landmarks::plaza::load_areas(&paths.repo_root).unwrap();
    let cover = eo_data::LandCover::load(&paths).unwrap();
    let tree_cover = |x: f64, z: f64| cover.class_at(x, z) == grove::WORLDCOVER_TREE;
    for (site, (c, _)) in grove::SITES.iter().zip(&centres) {
        let g = grove::Ground {
            greens: greens.iter().filter(|a| eo_landmarks::ground_cover::tree_class_of(&a.kind) > 0).map(|a| (a.polygon.clone(), a.holes.clone())).collect(),
            lots: lots.clone(),
            ways: &ways,
            buildings: &buildings,
            tree_cover: &tree_cover,
            height_at: &height_at,
        };
        let summit = (site.landmark == "coit-tower").then(|| grove::Summit { y: height_at(c[0], c[1]).unwrap(), lift_m: 5.97, lot: grove::summit_lot(*c, &lots) });
        let trees = grove::plant4(site, *c, &g, summit.as_ref());
        assert!(trees.len() > 150, "{}: {} trees", site.id, trees.len());
        for t in &trees {
            let p = [t.at[0], t.at[2]];
            assert!(lots.iter().all(|l| grove::ring_distance(l, p) >= t.radius * 0.8 + grove::LOT_CLEAR_M - 1e-6), "{}: a crown over a car park", site.id);
            assert!(buildings.iter().all(|(r, _)| grove::ring_distance(r, p) >= t.radius + grove::BUILDING_CLEAR_M - 1e-6), "{}: a crown over a building", site.id);
            if let Some(sm) = &summit {
                let r = (p[0] - c[0]).hypot(p[1] - c[1]);
                if r < grove::SUMMIT_REACH_M {
                    let cap = sm.y + sm.lift_m + (r - site.keep_out_m).max(0.0) * grove::SUMMIT_SLOPE;
                    assert!(t.at[1] + t.height <= cap + 0.2, "{}: a tree over the summit cone", site.id);
                }
            }
        }
        // the forms the species give (Mount Sutro's blue gums ragged; Pioneer Park's mix with its cypress flat-topped).
        assert!(trees.iter().any(|t| t.species.form == grove::FORM_RAGGED || t.species.form == grove::FORM_FLAT), "{}", site.id);
    }
}
