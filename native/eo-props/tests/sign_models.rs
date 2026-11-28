//! The SFMTA sign plates' Blender models (`eo_props::sign_models`): the provenance names one
//! model per inventory face (the official art's, or the face the game draws itself where no
//! layout is published), at the inventory's own face index, every file present with the size
//! and digest the provenance records.
use sha2::Digest;

#[test]
fn every_sign_model_file_is_the_one_its_provenance_names() {
    let paths = eo_data::DataPaths::discover().expect("repo data");
    let js = eo_props::JsProps::load().unwrap();
    let signs = eo_props::sign_models::load(&paths.repo_root, &js.signs.faces).unwrap();
    assert_eq!(signs.faces.len(), js.signs.faces.len(), "a model for every inventory face");
    assert!((signs.post_radius_m - js.consts["signPostRadiusM"].as_f64().unwrap()).abs() < 1e-9, "built round the post the game draws");
    for f in &signs.faces {
        assert_eq!(js.signs.faces[f.face_index as usize], f.face, "face index");
        assert!(f.frame_m[0] > 0.05 && f.frame_m[1] > 0.05, "{}: frame {:?}", f.face, f.frame_m);
        assert_eq!(f.variants.len(), eo_props::sign_models::WEAR.len(), "{}: every wear variant", f.face);
        assert_eq!(f.variants[0].lods[0].sha256, f.lods[0].sha256, "{}: the first variant is the plate's own", f.face);
        for l in f.variants.iter().flat_map(|v| v.lods.iter()) {
            let bytes = std::fs::read(paths.repo_root.join(&l.path)).unwrap_or_else(|e| panic!("{}: {e}", l.path));
            assert_eq!(bytes.len() as u64, l.bytes, "{}: size", l.path);
            let hex: String = sha2::Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(hex, l.sha256, "{}: digest", l.path);
        }
    }
}

/// The wear a plate draws is fixed by where it stands, the plates of one pole wear alike,
/// and across the city most plates are aged, a quarter rusty, a sixth battered.
#[test]
fn a_plate_wears_by_where_it_stands() {
    use eo_props::sign_models::wear_of;
    let at = |x: f64, z: f64| [x, 9.0, z];
    assert_eq!(wear_of(at(1953.71, -622.13), 3), wear_of(at(1953.71, -622.13), 3), "stable");
    assert_eq!(wear_of(at(1953.71, -622.13), 1), 0, "one variant: the first");
    let mut n = [0usize; 3];
    for i in 0..20_000 {
        let (x, z) = ((i % 200) as f64 * 3.7 - 400.0, (i / 200) as f64 * 5.3 - 300.0);
        n[wear_of(at(x, z), 3)] += 1;
    }
    let share = |k: usize| n[k] as f64 / 20_000.0;
    assert!((share(0) - 0.60).abs() < 0.03 && (share(1) - 0.25).abs() < 0.03 && (share(2) - 0.15).abs() < 0.03, "shares {n:?}");
    // the plates of one pole stand within its 0.03 m radius of it: one 0.5 m cell but at its edges.
    let same = (0..1000).filter(|i| {
        let (x, z) = (*i as f64 * 1.37 + 0.25, *i as f64 * 0.91 + 0.25);
        wear_of(at(x - 0.03, z), 3) == wear_of(at(x + 0.03, z), 3)
    });
    assert!(same.count() > 850, "the plates of one pole mostly wear alike");
}
