//! The modelled signal heads (`signal_models`, eo-app `--no-signal-models`): the
//! provenance lists both heads and both images, every file is in the checkout with the digest
//! and size it records, the sources carry their licence and credit, and switched on, the props
//! manifest lists them (the vehicle head in place of v2's, the pedestrian head added); off,
//! as before. Its own binary: the switch is process-wide.
use eo_props::signal_models::{self, KINDS};

#[test]
fn signal_models_listed_verified_and_switched() {
    let paths = eo_data::DataPaths::discover().unwrap();
    let s = signal_models::load(&paths.repo_root).unwrap();
    let files = s.props.values().chain([&s.lenses.file, &s.ped_faces.file]);
    for f in files {
        let bytes = std::fs::read(paths.repo_root.join(&f.path)).unwrap_or_else(|e| panic!("{}: {e}", f.path));
        assert_eq!(bytes.len() as u64, f.bytes, "{}: size", f.path);
        assert_eq!(eo_props::data::sha256_hex(&bytes), f.sha256, "{}: digest", f.path);
    }
    for k in KINDS {
        assert!(s.props[k].path.starts_with("public/assets/street-signals/"), "{k}: {}", s.props[k].path);
    }
    // the vehicle head keeps the game's lens pitch; its lenses are 12 in class (the street standards' circularNewFaces, within 5 %)
    let head = &s.rows["signalHead"];
    assert_eq!(head["lensPitchM"].as_f64(), Some(0.35));
    let r = head["lensRadiusM"].as_f64().unwrap();
    assert!((r * 2.0 - 0.305).abs() < 0.305 * 0.05, "lens diameter {}", r * 2.0);
    for m in ["body", "lens_red", "lens_amber", "lens_green"] {
        assert!(head["materials"].as_array().unwrap().iter().any(|v| v == m), "head material {m}");
    }
    assert_eq!(s.rows["pedHead"]["materials"][1], "ped_face");
    assert_eq!(s.lenses.px, 512);
    // the licence and credit travel with the files (CC BY 4.0)
    let text = std::fs::read_to_string(paths.repo_root.join(signal_models::PROVENANCE)).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    for (key, src) in v["sources"].as_object().unwrap() {
        assert_eq!(src["license"], "CC-BY-4.0", "{key}");
        assert!(src["author"].is_string() && src["url"].as_str().unwrap().starts_with("https://sketchfab.com/"), "{key}");
    }
    assert_eq!(v["attribution"].as_array().unwrap().len(), v["sources"].as_object().unwrap().len());
    let notices = std::fs::read_to_string(paths.repo_root.join("THIRD_PARTY_NOTICES.md")).unwrap();
    for src in v["sources"].as_object().unwrap().values() {
        assert!(notices.contains(src["url"].as_str().unwrap()), "THIRD_PARTY_NOTICES credits {}", src["url"]);
    }

    // off (the library default): the manifest as before
    assert!(signal_models::active().is_none(), "off unless set");
    let off = eo_props::JsProps::load().unwrap();
    assert!(off.street_prop("pedHead").is_err(), "no pedestrian head file off");
    assert_ne!(off.street_prop("signalHead").unwrap().sha256, s.props["signalHead"].sha256);
    // on: these files
    signal_models::set_signal_models(true);
    let on = eo_props::JsProps::load().unwrap();
    signal_models::set_signal_models(false);
    for k in KINDS {
        assert_eq!(on.street_prop(k).unwrap().sha256, s.props[k].sha256, "{k} listed on");
    }
}
