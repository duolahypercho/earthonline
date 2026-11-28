//! Round 4 of the signal poles in the prop library: the rising arms drawn as far as the poles
//! and the heads they carry (eo-app `--no-arm-far` stops them at the hardware's distance, where
//! their heads were left hanging in the air), and the level arms at their class's diameter
//! cooked at the poles' distances as the tube they replace.
use eo_props::signal_poles::{ARM_LEVEL_PARTS, ARM_RISE_PARTS, FINISHES, PARTS};
use eo_render::props_lib::PropLibrary;
use eo_render::props_lib2::{self as lib2, LibExtras};

#[test]
fn rising_arms_as_far_as_their_poles() {
    let paths = eo_data::DataPaths::discover().expect("repo data");
    let js = eo_props::JsProps::load().unwrap();
    let ca = eo_data::CaStandards::load(&paths).unwrap();
    let lens = ca.metric_key("signal-lens-size", "circularNewFaces").unwrap();
    let dome = lib2::dome_standards(&ca).unwrap();
    let poles = eo_props::signal_poles::load(&paths.repo_root).unwrap();
    let off = LibExtras { signal_poles: Some(&poles), ..Default::default() };
    let on = LibExtras { rising_arm_far: true, ..off };
    assert!(!LibExtras::default().rising_arm_far, "off by default");
    assert_ne!(PropLibrary::key_with(&js, &off, dome), PropLibrary::key_with(&js, &on, dome));
    let lib_off = PropLibrary::cook_full(&paths.repo_root, &js, lens, &off, dome).unwrap();
    let lib_on = PropLibrary::cook_full(&paths.repo_root, &js, lens, &on, dome).unwrap();
    assert_eq!(lib_on.meshes.len(), lib_off.meshes.len(), "the same meshes, drawn further");
    let head = lib_on.models[eo_props::Model::SignalHead as usize];
    for f in 0..FINISHES.len() {
        for (k, part) in ARM_RISE_PARTS.iter().enumerate() {
            let id = (lib2::SIGNAL_POLE_BASE as usize) + f * PARTS.len() + *part as usize;
            assert_eq!(lib_off.models[id].lod, lib2::SIGNAL_HARDWARE_LOD, "off: armRise{k} at the hardware's distance");
            assert_eq!(lib_on.models[id].lod, lib2::SIGNAL_POLE_LOD, "on: armRise{k} at the poles'");
            assert!(lib_on.models[id].lod[2] >= head.lod[2], "armRise{k} drawn as far as the heads it carries");
        }
        for part in ARM_LEVEL_PARTS {
            let id = (lib2::SIGNAL_POLE_BASE as usize) + f * PARTS.len() + part as usize;
            assert_eq!(lib_off.models[id].lod, lib2::SIGNAL_POLE_LOD, "the level arms as the tube");
        }
    }
}
