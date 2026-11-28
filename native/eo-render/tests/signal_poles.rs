//! The signal poles' Blender models in the prop library (eo-props `signal_poles`, eo-app
//! `--no-signal-poles`): cooked only when given (off, the key and the library as before), one
//! model of three levels per part on three texture layers (the pole's, the posts', the
//! hardware's), the poles at the code-built pole's level edges, the library within the cull
//! pass's mesh limit with every other optional model as well.
use eo_props::signal_poles::{FINISHES, PARTS, Part};
use eo_render::props_lib::{NONE, PropLibrary};
use eo_render::props_lib2::{self as lib2, LibExtras};

#[test]
fn signal_poles_cook_only_when_given() {
    let paths = eo_data::DataPaths::discover().expect("repo data");
    let js = eo_props::JsProps::load().unwrap();
    let ca = eo_data::CaStandards::load(&paths).unwrap();
    let lens = ca.metric_key("signal-lens-size", "circularNewFaces").unwrap();
    let dome = lib2::dome_standards(&ca).unwrap();
    let poles = eo_props::signal_poles::load(&paths.repo_root).unwrap();
    let signs = eo_props::sign_models::load(&paths.repo_root, &js.signs.faces).unwrap();
    let bodies = eo_props::boats::load_bodies(&paths.repo_root).unwrap();
    let off = LibExtras { boats: Some(&bodies), sign_models: Some(&signs), ..Default::default() };
    let on = LibExtras { signal_poles: Some(&poles), ..off };
    assert!(LibExtras::default().signal_poles.is_none(), "off by default");
    assert_ne!(PropLibrary::key_with(&js, &off, dome), PropLibrary::key_with(&js, &on, dome), "on and off cook different libraries");
    let lib_off = PropLibrary::cook_full(&paths.repo_root, &js, lens, &off, dome).unwrap();
    let lib_on = PropLibrary::cook_full(&paths.repo_root, &js, lens, &on, dome).unwrap();
    assert!(lib_on.missing.iter().all(|m| !m.starts_with("signal pole part")), "every part read: {:?}", lib_on.missing);
    assert_eq!(lib_on.layers.len(), lib_off.layers.len() + 4 * FINISHES.len() + 2, "four atlases per finish: the mast pole's, the posts', the hardware's, round 2's; round 5's cabinet's and pull box's, one for both");
    assert!(lib_on.meshes.len() as u32 <= eo_render::props::PROP_MESH_MAX, "within the cull pass's mesh limit: {}", lib_on.meshes.len());
    // off: the code-built pole and arm, and nothing at the parts' ids
    assert_eq!(lib2::SIGNAL_POLE_MODELS as usize, FINISHES.len() * PARTS.len());
    for k in 0..lib2::SIGNAL_POLE_MODELS {
        let (part, name) = PARTS[k as usize % PARTS.len()];
        let id = (lib2::SIGNAL_POLE_BASE + k) as usize;
        assert!(id < lib2::RENDER_MODEL_COUNT);
        assert!(lib_off.models[id].meshes[0] == NONE, "off: no {name}");
        let m = lib_on.models[id];
        assert!(m.meshes.iter().take(3).all(|&k| k != NONE), "{name}: three levels");
        assert!(lib_on.triangles(m.meshes[0]) >= lib_on.triangles(m.meshes[2]), "{name}: the far level the lighter");
        let pole_like = matches!(part, Part::MastPole | Part::PedestalPole | Part::PedPost | Part::ArmTube | Part::MastFoot | Part::MastMid | Part::MastTop) || eo_props::signal_poles::ARM_LEVEL_PARTS.contains(&part);
        let lod = match part {
            Part::Cabinet => lib2::SIGNAL_POLE_LOD,
            Part::PullBox => lib2::PULL_BOX_LOD,
            _ if pole_like => lib2::SIGNAL_POLE_LOD,
            _ => lib2::SIGNAL_HARDWARE_LOD,
        };
        assert_eq!(m.lod, lod, "{name}");
    }
    for m in [eo_props::Model::SignalPole, eo_props::Model::MastArm] {
        assert_eq!(lib_on.models[m as usize].meshes, lib_off.models[m as usize].meshes, "{m:?}: the code-built model kept");
    }
    // the mast pole stands 6.9 m on its foot: its sphere holds it, centred half way up
    let mast = lib_on.models[(lib2::SIGNAL_POLE_BASE + Part::MastPole as u32) as usize];
    assert!(mast.radius > 3.4 && mast.radius < 4.0 && (mast.center[1] - 3.45).abs() < 0.3, "mast pole sphere {:?} {}", mast.center, mast.radius);
    // near, the bolts, nuts and handhole; far, a plain tube
    assert!(lib_on.triangles(mast.meshes[0]) > 4 * lib_on.triangles(mast.meshes[2]));
}
