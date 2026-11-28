//! Round 5 of the signal poles in the prop library: the corner hardware. The controller cabinet
//! (the city's MSF60 on its pad) drawn as far as the poles, casting; the pull box flush with the
//! sidewalk, casting nothing, gone past `PULL_BOX_LOD`; the sign channel as the hardware; and the
//! feature bits the shader reads for the arm name signs (`--no-retro-viewer`, `--no-sign-back-aged`).
use eo_props::signal_poles::{FINISHES, PARTS, Part};
use eo_render::props_lib::PropLibrary;
use eo_render::props_lib2::{self as lib2, LibExtras};

#[test]
fn corner_hardware_in_the_library() {
    let paths = eo_data::DataPaths::discover().expect("repo data");
    let js = eo_props::JsProps::load().unwrap();
    let ca = eo_data::CaStandards::load(&paths).unwrap();
    let lens = ca.metric_key("signal-lens-size", "circularNewFaces").unwrap();
    let dome = lib2::dome_standards(&ca).unwrap();
    let poles = eo_props::signal_poles::load(&paths.repo_root).unwrap();
    let x = LibExtras { signal_poles: Some(&poles), ..Default::default() };
    let lib = PropLibrary::cook_full(&paths.repo_root, &js, lens, &x, dome).unwrap();
    let missed: Vec<&String> = lib.missing.iter().filter(|m| m.starts_with("signal pole part")).collect();
    assert!(missed.is_empty(), "every signal pole part cooked: {missed:?}");
    let id = |f: usize, p: Part| (lib2::SIGNAL_POLE_BASE as usize) + f * PARTS.len() + p as usize;
    for f in 0..FINISHES.len() {
        let cab = lib.models[id(f, Part::Cabinet)];
        assert_eq!(cab.lod, lib2::SIGNAL_POLE_LOD, "the cabinet as far as the poles");
        assert_eq!(cab.cast[0], 2, "the cabinet casts");
        let pb = lib.models[id(f, Part::PullBox)];
        assert_eq!(pb.lod, lib2::PULL_BOX_LOD);
        assert_eq!(pb.cast[0], 0, "a flush lid casts nothing");
        let ch = lib.models[id(f, Part::SignChannel)];
        assert_eq!(ch.lod, lib2::SIGNAL_HARDWARE_LOD, "the channel as the hardware");
        // both finishes draw the same files
        assert_eq!(lib.models[id(f, Part::Cabinet)].meshes, lib.models[id(0, Part::Cabinet)].meshes);
    }
    // the cabinet's size as the provenance has it (MSF60: 61 x 36 x 18.5 in) over its pad
    let d = &poles.dims;
    assert!((d.cabinet_m[0] - 36.0 * 0.0254).abs() < 1e-3 && (d.cabinet_m[1] - 18.5 * 0.0254).abs() < 1e-3 && (d.cabinet_m[2] - 61.0 * 0.0254).abs() < 1e-3);
    let files = &poles.parts[Part::Cabinet as usize][0];
    let dim = files.dimensions_m.as_ref().unwrap();
    assert!((dim[0] - (d.cabinet_m[0] + 2.0 * d.cabinet_pad_m[1])).abs() < 1e-3 && (dim[2] - (d.cabinet_m[1] + 2.0 * d.cabinet_pad_m[1])).abs() < 1e-3, "the pad round the cabinet: {dim:?}");
    assert!((dim[1] - (d.cabinet_m[2] + d.cabinet_pad_m[0] + 0.05)).abs() < 0.01, "the cabinet on its pad (set 5 cm into the walk): {dim:?}");
}

#[test]
fn arm_sign_feature_bits_match_the_shader() {
    use eo_render::props::{F_ALL, F_RETRO_VIEWER, F_SIGN_BACK_AGED};
    let src = include_str!("../src/props_types.wgsl");
    assert!(src.contains(&format!("const PR_F_RETRO_VIEWER: u32 = {F_RETRO_VIEWER}u;")));
    assert!(src.contains(&format!("const PR_F_SIGN_BACK_AGED: u32 = {F_SIGN_BACK_AGED}u;")));
    assert_eq!(F_ALL & (F_RETRO_VIEWER | F_SIGN_BACK_AGED), F_RETRO_VIEWER | F_SIGN_BACK_AGED, "on by default");
    let draw = include_str!("../src/props_draw.wgsl");
    assert!(draw.contains("(pr.mode.z & PR_F_RETRO_VIEWER) != 0u") && draw.contains("(pr.mode.z & PR_F_SIGN_BACK_AGED) != 0u"));
}
