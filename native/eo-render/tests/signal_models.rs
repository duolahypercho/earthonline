//! The modelled signal heads in the prop library (eo-props `signal_models`, eo-app
//! `--no-signal-models`): on, the vehicle head draws its own three lenses near (no lens discs
//! of the code's) and the pedestrian head its own face, each lit through its image (the lens
//! materials and the face textured on the two added layers); far, the box heads with the
//! backplate. Off, the library as before. Its own binary: the switch is process-wide.
use eo_render::props_lib::{self, PropLibrary};

fn cook() -> PropLibrary {
    let paths = eo_data::DataPaths::discover().unwrap();
    let js = eo_props::JsProps::load().unwrap();
    let ca = eo_data::CaStandards::load(&paths).unwrap();
    let lens = ca.metric_key("signal-lens-size", "circularNewFaces").unwrap();
    PropLibrary::cook(&paths.repo_root, &js, lens).unwrap()
}

#[test]
fn signal_models_cook_lenses_and_faces_through_their_images() {
    // the shader's cell inset is the provenance's
    let src = include_str!("../src/props_draw.wgsl");
    assert!(src.contains(&format!("const PR_SIGNAL_INSET: f32 = {};", props_lib::SIGNAL_CELL_INSET)), "PR_SIGNAL_INSET is SIGNAL_CELL_INSET");
    assert!(src.contains(&format!("const PR_SIGNAL_CELL: f32 = {};", 0.5 - 2.0 * props_lib::SIGNAL_CELL_INSET)), "PR_SIGNAL_CELL is 0.5 - 2 x the inset");
    // the pedestrian masks' gain is one over their LEDs' cover of the symbol (the provenance's)
    let paths = eo_data::DataPaths::discover().unwrap();
    let models = eo_props::signal_models::load(&paths.repo_root).unwrap();
    let prov: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(paths.repo_root.join(eo_props::signal_models::PROVENANCE)).unwrap()).unwrap();
    let cover = prov["images"]["pedFaces"]["maskCoverOfSymbol"].as_f64().unwrap();
    let gain: f64 = src.split("const PR_SIGNAL_MASK_GAIN: f32 = ").nth(1).and_then(|t| t.split(';').next()).and_then(|t| t.parse().ok()).unwrap();
    assert!((gain * cover - 1.0).abs() < 0.01, "PR_SIGNAL_MASK_GAIN {gain} x cover {cover}");
    assert!(models.lenses.file.path.ends_with(".png") && models.ped_faces.file.path.ends_with(".png"), "lossless images (packed channels)");
    // (`--signal-light`) the shader's lens and visor are the built head's (the provenance the build writes)
    let head = &prov["assets"]["signalHead"];
    use eo_render::props as p;
    for (key, want) in [
        ("lensRadiusM", p::SL_LENS_RADIUS_M),
        ("lensFaceZ", p::SL_LENS_APEX_Z_M),
        ("lensDomeSagM", p::SL_LENS_DOME_SAG_M),
        ("visorInsideRadiusM", p::SL_VISOR_RADIUS_M),
        ("visorDoorZM", p::SL_VISOR_DOOR_Z_M),
        ("visorMouthZM", p::SL_VISOR_MOUTH_Z_M),
        ("visorCutMouthOfRadius", p::SL_VISOR_CUT_MOUTH),
    ] {
        let got = head[key].as_f64().unwrap_or(f64::NAN) as f32;
        assert!((got - want).abs() < 2e-4, "props.rs {key} {want} is not the provenance's {got}");
    }
    eo_props::props_v2::set_street_props_v2(true);
    let off = cook();
    eo_props::signal_models::set_signal_models(true);
    let on = cook();
    eo_props::signal_models::set_signal_models(false);
    assert!(on.missing.iter().all(|m| !m.contains("signal") && !m.contains("pedHead")), "every signal file read: {:?}", on.missing);
    assert_eq!(on.layers.len(), off.layers.len() + 3, "the lens and face images and the pedestrian head's maps");
    let lens_mats = |lib: &PropLibrary| lib.materials.iter().filter(|m| m.flags & props_lib::P_LENS != 0).copied().collect::<Vec<_>>();
    assert!(lens_mats(&off).iter().all(|m| m.flags & props_lib::P_TEX == 0), "off: flat lenses");
    let on_lens = lens_mats(&on);
    assert_eq!(on_lens.len(), 3);
    assert!(on_lens.iter().all(|m| m.flags & props_lib::P_TEX != 0 && m.layer > 0), "on: lenses through the lens image");
    let face = props_lib::P_PED_HAND | props_lib::P_PED_WALK | props_lib::P_TEX;
    assert_eq!(on.materials.iter().filter(|m| m.flags & face == face).count(), 1, "one modelled pedestrian face");
    let head = eo_props::Model::SignalHead as usize;
    let ped = eo_props::Model::PedHead as usize;
    // near: the modelled head (its 4.7 k triangles) with no lens discs of the code's; far as before, a box
    let near_on = on.triangles(on.models[head].meshes[0]);
    let near_off = off.triangles(off.models[head].meshes[0]);
    assert!(near_on > 4000 && near_on != near_off, "near head {near_on} triangles (off {near_off})");
    let lens_tris = |lib: &PropLibrary, mesh: u32| {
        let m = lib.meshes[mesh as usize];
        lib.indices[m.first as usize..(m.first + m.count) as usize]
            .chunks_exact(3)
            .filter(|t| lib.materials[lib.vertices[(m.base_vertex + t[0] as i32) as usize].material as usize].flags & props_lib::P_LENS != 0)
            .count()
    };
    assert!(lens_tris(&on, on.models[head].meshes[0]) > 200, "the head's own domed lenses");
    assert!(lens_tris(&on, on.models[head].meshes[1]) == 3 * 8, "far: the three 8-sided discs");
    assert!(on.triangles(on.models[head].meshes[1]) < 200, "far head light");
    // the pedestrian head: near its housing and face, far the box
    assert_ne!(on.models[ped].meshes[0], on.models[ped].meshes[1], "on: two levels");
    assert_eq!(off.models[ped].meshes[0], off.models[ped].meshes[1], "off: one level, as before");
    assert!(on.triangles(on.models[ped].meshes[0]) > 2000);
}
