//! Street props v2 (eo-props `props_v2`): the manifest's files are on disk at
//! their recorded sizes, and only with the switch on do the JS props and the
//! overhead list them (one test: the switch is process-wide).

#[test]
fn street_props_v2_replace_the_v1_files_only_when_on() {
    let paths = eo_data::DataPaths::discover().expect("repo data");
    let v2 = eo_props::props_v2::load(&paths.repo_root).unwrap();
    for (kind, p) in &v2.props {
        assert_eq!(std::fs::metadata(paths.repo_root.join(&p.path)).map(|m| m.len()).ok(), Some(p.bytes), "{kind}: {} at its recorded size", p.path);
        assert_eq!(p.sha256.len(), 64);
    }
    assert!(v2.lens_face_z().is_some_and(|z| z > 0.0 && z < 0.1), "the signal head's lens face");
    let off = eo_props::JsProps::load().unwrap();
    let oh_off = eo_props::overhead::OverheadJs::load().unwrap();
    assert!(off.street_props.values().all(|p| !v2.is_v2(p)), "off: the JS manifest as exported");
    eo_props::props_v2::set_street_props_v2(true);
    let on = eo_props::JsProps::load().unwrap();
    let oh_on = eo_props::overhead::OverheadJs::load().unwrap();
    eo_props::props_v2::set_street_props_v2(false);
    let mut replaced = 0;
    for (kind, p) in &on.street_props {
        match v2.props.get(kind) {
            Some(q) => {
                assert_eq!(p.sha256, q.sha256, "{kind}: the v2 file");
                replaced += 1;
            }
            None => assert_eq!(p.sha256, off.street_props[kind].sha256, "{kind}: no v2 file, the v1 one kept"),
        }
    }
    assert!(replaced >= 20, "{replaced} kinds replaced");
    assert_eq!(on.street_props.len(), off.street_props.len(), "no kind added or dropped");
    // The v2 pole's wire seats.
    let row = &v2.rows["utilityPole"]["attach"];
    assert_eq!(oh_on.seats.len(), row.as_array().unwrap().len());
    assert!((oh_on.seats[0].z - row[0]["z"].as_f64().unwrap()).abs() < 1e-9);
    assert!(oh_on.seats.iter().zip(&oh_off.seats).any(|(a, b)| (a.z - b.z).abs() > 1e-3), "the seats moved with the pole");
    assert!(eo_props::JsProps::load().unwrap().street_props.values().all(|p| !v2.is_v2(p)), "off again");
}
