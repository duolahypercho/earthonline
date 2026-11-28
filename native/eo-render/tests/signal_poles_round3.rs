//! Round 3 of the signal poles in the prop library: the mast pole cut at its shaft (its foot
//! and its top at their true size), the street lights that stand on a mast pole (the district
//! lights' own meshes without their post's base: the post inside the pole, eo-app
//! `--no-signal-luminaires`), and the cull pass's mesh limit (what it costs and that the shader
//! and the library agree on it).
use eo_props::signal_poles::{FINISHES, PARTS, Part};
use eo_render::props_lib::{NONE, P_LAMP_GLOW, PropLibrary};
use eo_render::props_lib2::{self as lib2, LibExtras};

/// The cull pass's mesh limit: props_cull.wgsl's constant and its workgroup array are the
/// library's (the device check for its workgroup memory is in props.rs `PropsGpu::new`).
#[test]
fn mesh_limit_matches_the_cull_shader() {
    let src = include_str!("../src/props_cull.wgsl");
    let max: u32 = src.lines().find_map(|l| l.trim().strip_prefix("const PR_MESH_MAX: u32 = ")).and_then(|v| v.trim_end_matches(';').trim_end_matches('u').parse().ok()).expect("PR_MESH_MAX");
    assert_eq!(max, eo_render::props::PROP_MESH_MAX);
    let wg: u32 = src.lines().find_map(|l| l.trim().strip_prefix("var<workgroup> pr_wg: array<u32, ")).and_then(|v| v.split('>').next()?.parse().ok()).expect("pr_wg");
    assert_eq!(wg, 6 * max, "pr_wg holds PR_VIEWS (6) x PR_MESH_MAX");
    // the cull records keep a mesh index in 12 bits
    assert!(max <= 1 << 12);
}

fn glb_points(path: &std::path::Path) -> Vec<[f32; 3]> {
    let g = gltf::Gltf::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let blob = g.blob.clone().unwrap();
    let mut out = vec![];
    for mesh in g.document.meshes() {
        for p in mesh.primitives() {
            let r = p.reader(|_| Some(&blob[..]));
            out.extend(r.read_positions().unwrap());
        }
    }
    out
}

#[test]
fn round3_cut_pole_and_pole_lamps() {
    let paths = eo_data::DataPaths::discover().expect("repo data");
    // the street lights as the game draws them (street props v2)
    eo_props::props_v2::set_street_props_v2(true);
    let js = eo_props::JsProps::load().unwrap();
    let ca = eo_data::CaStandards::load(&paths).unwrap();
    let lens = ca.metric_key("signal-lens-size", "circularNewFaces").unwrap();
    let dome = lib2::dome_standards(&ca).unwrap();
    let poles = eo_props::signal_poles::load(&paths.repo_root).unwrap();
    let d = &poles.dims;
    // the cut pieces: the foot below the first cut, the top above the second, each a level-0
    // file of its own per finish, on the pole's atlas
    for f in 0..FINISHES.len() {
        for (part, lo, hi) in [(Part::MastFoot, -0.1, d.mast_cut_m[0]), (Part::MastMid, d.mast_cut_m[0], d.mast_cut_m[1]), (Part::MastTop, d.mast_cut_m[1], d.mast_pole_height_m + 0.1)] {
            let lods = &poles.parts[f * PARTS.len() + part as usize];
            for l in lods {
                let pts = glb_points(&paths.repo_root.join(&l.path));
                let (y0, y1) = pts.iter().fold((f32::MAX, f32::MIN), |a, p| (a.0.min(p[1]), a.1.max(p[1])));
                assert!(y0 as f64 >= lo - 1e-3 && y1 as f64 <= hi + 1e-3, "{:?} {} spans {y0}..{y1}", part, l.path);
            }
        }
    }
    // round 4: the level arms at their class's diameter: straight along +X at the class's
    // length, the sheet's diameter at the pole
    for (k, ad) in d.arms.iter().enumerate() {
        let pts = glb_points(&paths.repo_root.join(&poles.parts[eo_props::signal_poles::ARM_LEVEL_PARTS[k] as usize][0].path));
        let x1 = pts.iter().map(|p| p[0]).fold(f32::MIN, f32::max) as f64;
        let root = pts.iter().filter(|p| p[0] < 0.01).map(|p| p[1].hypot(p[2])).fold(0.0f32, f32::max) as f64;
        let rise = pts.iter().map(|p| p[1]).fold(f32::MIN, f32::max) as f64;
        assert!((x1 - ad.class.length).abs() < 0.01, "armLevel{k}: {x1} long");
        assert!((root * 2.0 - ad.class.od_at_pole).abs() < 0.003, "armLevel{k}: {} at the pole", root * 2.0);
        assert!(rise <= ad.root_radius_m + 1e-3, "armLevel{k}: level");
    }
    // the foot keeps the base plate's corners (18 in square) and the handhole; the mid is a plain shaft
    let foot = glb_points(&paths.repo_root.join(&poles.parts[Part::MastFoot as usize][0].path));
    assert!(foot.iter().any(|p| p[0].abs() > 0.2 && p[2].abs() > 0.2), "the foot's base plate");
    let mid = glb_points(&paths.repo_root.join(&poles.parts[Part::MastMid as usize][0].path));
    assert!(mid.iter().all(|p| p[0].hypot(p[2]) < 0.15), "the mid piece is the shaft only");

    // the district lights' posts end at LAMP_POST_TOP_M, inside a mast pole all the way up
    // (the pole's radius there from the JS signal hardware), their base cover below the cut
    let r_base = js.num("signalHardware", "poleRadiusBaseM") as f32;
    let r_top = js.num("signalHardware", "poleRadiusTopM") as f32;
    let h = js.num("signalHardware", "mastPoleHeightM") as f32;
    let top = eo_props::signal_lamps::LAMP_POST_TOP_M as f32;
    for kind in ["lampCobra", "lampBanner"] {
        let prop = js.street_prop(kind).unwrap();
        let pts = glb_points(&paths.repo_root.join(&prop.path));
        let ring = pts.iter().filter(|p| (p[1] - top).abs() < 0.005 && p[0].hypot(p[2]) < 0.1).count();
        assert!(ring >= 16, "{kind}: the arm fitting's foot ring at {top} m ({ring} vertices)");
        let cover = pts.iter().filter(|p| p[1] < lib2::SIGNAL_LAMP_CUT_M && p[0].hypot(p[2]) > 0.14).count();
        assert!(cover > 0, "{kind}: the base cover (wider than the pole) is below the cut");
        // over the cut and under the post's top, nothing past the pole but a banner's arms
        let out = pts.iter().filter(|p| p[1] >= lib2::SIGNAL_LAMP_CUT_M && p[1] < top - 0.005).filter(|p| p[0].hypot(p[2]) >= r_base + (r_top - r_base) * p[1] / h).filter(|p| p[0].abs() < 0.2).count();
        assert_eq!(out, 0, "{kind}: the post stands inside the mast pole");
    }

    let bodies = eo_props::boats::load_bodies(&paths.repo_root).unwrap();
    let signs = eo_props::sign_models::load(&paths.repo_root, &js.signs.faces).unwrap();
    let base = LibExtras { boats: Some(&bodies), sign_models: Some(&signs), signal_poles: Some(&poles), luminaires: true, ..Default::default() };
    let on = LibExtras { pole_lamps: true, ..base };
    assert!(!LibExtras::default().pole_lamps, "off by default");
    assert_ne!(PropLibrary::key_with(&js, &base, dome), PropLibrary::key_with(&js, &on, dome));
    let lib_off = PropLibrary::cook_full(&paths.repo_root, &js, lens, &base, dome).unwrap();
    let lib = PropLibrary::cook_full(&paths.repo_root, &js, lens, &on, dome).unwrap();
    println!("meshes: {} (pole lamps off {}), limit {}", lib.meshes.len(), lib_off.meshes.len(), eo_render::props::PROP_MESH_MAX);
    assert!(lib.meshes.len() as u32 <= eo_render::props::PROP_MESH_MAX);
    assert_eq!(lib.layers.len(), lib_off.layers.len(), "the pole lamps share the lights' maps");
    for k in 0..lib2::SIGNAL_LAMP_MODELS {
        let id = (lib2::SIGNAL_LAMP_BASE + k) as usize;
        assert_eq!(lib_off.models[id].meshes[0], NONE, "off: no pole lamp");
        let m = lib.models[id];
        let src = lib.models[[eo_props::Model::LampCobra, eo_props::Model::LampBanner][k as usize] as usize];
        assert!(m.meshes.iter().take(3).all(|&x| x != NONE));
        assert_eq!(m.lod, src.lod);
        assert!(lib.triangles(m.meshes[0]) < lib.triangles(src.meshes[0]), "the base left out");
        assert!(lib.triangles(m.meshes[0]) * 10 > lib.triangles(src.meshes[0]) * 7, "the post, arm and head kept");
        // the post kept whole (one tube from inside the base cover to over the fitting): no part floats
        let g = lib.meshes[m.meshes[0] as usize];
        let ys = |t: &[u32]| t.iter().map(|&v| lib.vertices[(g.base_vertex + v as i32) as usize].pos[1]).collect::<Vec<f32>>();
        let post = lib.indices[g.first as usize..(g.first + g.count) as usize].chunks_exact(3).map(ys).any(|y| y.iter().cloned().fold(f32::MAX, f32::min) < lib2::SIGNAL_LAMP_CUT_M && y.iter().cloned().fold(f32::MIN, f32::max) > top);
        assert!(post, "pole lamp {k}: its post");
        // its lit glass kept (the light's own material)
        let g = lib.meshes[m.meshes[0] as usize];
        let lit = lib.indices[g.first as usize..(g.first + g.count) as usize].iter().any(|&v| {
            let mat = lib.vertices[(g.base_vertex + v as i32) as usize].material;
            lib.materials[mat as usize].flags & P_LAMP_GLOW != 0
        });
        assert!(lit, "pole lamp {k}: its glass lit");
    }
}
