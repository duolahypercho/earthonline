//! The modelled signal poles (`signal_poles`, eo-app `--no-signal-poles`): the provenance
//! lists every part at three levels, every file is in the checkout with the digest and size it
//! records, the parts are built at the game's own signal-hardware constants, the sources are
//! cited, and on a real junction every pole and arm is drawn as a part and every head mounted.
use eo_props::Model;
use eo_props::signal_poles::{self, FINISHES, ONE_FINISH, PARTS, Part};

#[test]
fn signal_poles_listed_and_verified() {
    let paths = eo_data::DataPaths::discover().unwrap();
    let s = signal_poles::load(&paths.repo_root).unwrap();
    assert_eq!(s.parts.len(), FINISHES.len() * PARTS.len());
    for (k, lods) in s.parts.iter().enumerate() {
        let name = PARTS[k % PARTS.len()].1;
        for f in lods {
            assert!(f.path.starts_with("public/assets/signal-poles/"), "{name}: {}", f.path);
            let bytes = std::fs::read(paths.repo_root.join(&f.path)).unwrap_or_else(|e| panic!("{}: {e}", f.path));
            assert_eq!(bytes.len() as u64, f.bytes, "{}: size", f.path);
            assert_eq!(eo_props::data::sha256_hex(&bytes), f.sha256, "{}: digest", f.path);
        }
        // level 0 carries the most triangles
        let t: Vec<u64> = lods.iter().map(|f| f.triangles.unwrap()).collect();
        assert!(t[0] >= t[1] && t[1] >= t[2], "{name}: levels {t:?}");
        // the finishes: the same geometry, level 0 its own maps, the far levels shared
        let painted = &s.parts[k % PARTS.len()];
        if k >= PARTS.len() && ONE_FINISH.contains(&PARTS[k % PARTS.len()].0) {
            assert_eq!(lods[0].sha256, painted[0].sha256, "{name}: the same in every finish");
        } else if k >= PARTS.len() {
            assert_ne!(lods[0].sha256, painted[0].sha256, "{name}: a galvanised level 0 of its own");
            assert_eq!(lods[0].triangles, painted[0].triangles, "{name}: same geometry");
            assert_eq!(lods[1].sha256, painted[1].sha256, "{name}: the far levels shared");
        }
    }
    // built at the game's signal hardware (props-js.json.gz)
    let js = eo_props::JsProps::load().unwrap();
    let d = s.dims;
    assert_eq!(d.mast_pole_height_m, js.num("signalHardware", "mastPoleHeightM"));
    assert_eq!(d.pedestal_height_m, js.num("signalHardware", "pedestalPoleHeightM"));
    assert_eq!(d.post_height_m, js.num("pedHead", "postHeightM"));
    assert_eq!(d.side_reach_m, js.num("signalHardware", "poleHeadLateralM"));
    let mast = &s.parts[Part::MastPole as usize][0];
    let dim = mast.dimensions_m.as_ref().unwrap();
    // from the foundation's underside to the cap's dome
    assert!(dim[1] > d.mast_pole_height_m && dim[1] < d.mast_pole_height_m + 0.12, "mast pole height {dim:?}");
    // the sources: a cited sheet, no third-party geometry
    let text = std::fs::read_to_string(paths.repo_root.join(signal_poles::PROVENANCE)).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let es7c = &v["sources"]["caltransEs7c"];
    assert!(es7c["url"].as_str().unwrap().starts_with("https://") && es7c["edition"].is_string());
    assert_eq!(es7c["read"]["basePlateIn"], 18.0);
    assert!(v["modellingValues"].as_object().is_some_and(|m| !m.is_empty()));
    assert!(v["scriptSha256"].is_string());
}

#[test]
fn every_pole_and_arm_drawn_and_every_head_mounted() {
    let paths = eo_data::DataPaths::discover().unwrap();
    let s = signal_poles::load(&paths.repo_root).unwrap();
    let w = eo_props::load_world(&paths).unwrap();
    // a few tiles with signalised junctions
    let mut seen = 0;
    for key in w.tile_keys().into_iter().take(400) {
        let t = w.plan_tile(key);
        let props = t.props;
        let o = signal_poles::Options { variants: true, ..Default::default() };
        let hw = signal_poles::hardware(&props, &t.fittings, &s.dims, &o);
        for (p, r) in props.iter().zip(&hw.replace) {
            match p.model {
                Model::SignalPole | Model::MastArm => assert!(r.is_some(), "{:?} at {:?} drawn", p.model, p.pos),
                _ => assert!(r.is_none()),
            }
        }
        let heads = props.iter().filter(|p| matches!(p.model, Model::SignalHead | Model::PedHead)).count();
        let arms = props.iter().filter(|p| p.model == Model::MastArm).count();
        let mounts = hw.added.iter().filter(|p| !matches!(p.part, Part::ArmRoot | Part::ArmTip)).count();
        assert_eq!(hw.added.len() - mounts, 2 * arms, "a root and an end cap per arm");
        // every head finds what it hangs from (a head left without: at most a few per tile)
        assert!(mounts + heads / 20 + 1 >= heads, "tile {key:?}: {mounts} mounts for {heads} heads");
        seen += heads;
    }
    assert!(seen > 50, "{seen} heads seen");
}
