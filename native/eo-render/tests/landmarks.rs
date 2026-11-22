//! the set's near/far switch on the GPU (cull.wgsl `lod`, group
//! kind 3), the static draws' roles and materials, and the wall chunks. Needs
//! a GPU adapter and the repo's data (the bridges are built on the real ground).

use eo_render::authored::MaterialTable;
use eo_render::camera::Camera;
use eo_render::landmarks::{F_LANDMARK, GROUP_KIND, append};
use eo_render::scene::{LOD_LANDMARK_FAR, LOD_LANDMARK_NEAR, Scene, SceneStats};
use eo_render::{GpuContext, GpuOptions, Renderer, RendererInit, wgpu};
use glam::Vec3;

fn scene_of_landmarks() -> (Scene, MaterialTable, eo_render::landmarks::LandmarkStatic, eo_landmarks::LandmarkSet) {
    let paths = eo_data::DataPaths::discover().unwrap();
    let elevation = eo_data::ElevationGrid::load(&paths).unwrap();
    let ca = eo_data::CaStandards::load(&paths).unwrap();
    let set = eo_landmarks::LandmarkSet::build(&paths.repo_root, &|x, z| elevation.sample(x, z), &ca).unwrap();
    let mut scene = Scene { vertices: vec![], indices: vec![], objects: vec![], stats: SceneStats::default(), world_min: [f32::MAX; 3], world_max: [f32::MIN; 3] };
    let mut materials = MaterialTable::new();
    let st = append(&mut scene, &set, &mut materials, 0);
    (scene, materials, st, set)
}

#[test]
fn near_and_far_switch_on_the_gpu() {
    let (scene, materials, st, set) = scene_of_landmarks();
    // Every landmark and wall chunk has one group; every object names one of them.
    assert_eq!(st.groups.len(), set.landmarks.len() + set.walls.len());
    assert!(st.groups.iter().all(|g| g.kind == GROUP_KIND));
    let bridges = set.landmarks.len() as u32;
    for o in &scene.objects {
        if o.lod_role == LOD_LANDMARK_NEAR || o.lod_role == LOD_LANDMARK_FAR {
            assert!(o.lod_group < st.groups.len() as u32);
        }
        if o.lod_group < bridges {
            // A bridge's draws carry the set's fog cap.
            assert_ne!(materials.gpu[o.material as usize].flags & F_LANDMARK, 0);
        }
    }
    let opts = GpuOptions::default();
    let ctx = GpuContext::new(GpuContext::create_instance(&opts), None, opts).expect("a GPU adapter is required");
    let r = Renderer::from_init(
        &ctx,
        &RendererInit {
            vertices: &scene.vertices,
            indices: &scene.indices,
            objects: &scene.objects,
            extra_vertices: 0,
            extra_indices: 0,
            materials: &materials.gpu,
            groups: &st.groups,
            hlod_enter_m: 320.0,
            hlod_exit_m: 380.0,
            kit_layers: 0,
            kit_res: 1,
            color_format: wgpu::TextureFormat::Rgba8UnormSrgb,
            sample_count: 1,
            multiplier: 1,
            debug_ids: false,
            blend_first_object: scene.objects.len() as u32,
        },
    )
    .unwrap();
    // Each bridge: looked at from 600 m and from 4 km, beside its box's middle.
    for (li, l) in set.landmarks.iter().enumerate() {
        let (mn, mx) = (Vec3::from(l.box_min), Vec3::from(l.box_max));
        let c = (mn + mx) * 0.5;
        let half = (mx - mn) * 0.5;
        // Off the box's shorter plan side, looking back at its centre.
        let side = if half.x < half.z { Vec3::X } else { Vec3::Z };
        let mut counts = vec![];
        for d in [600.0f32, 4000.0] {
            let eye = Vec3::new(c.x, 60.0, c.z) + side * (if side == Vec3::X { half.x } else { half.z } + d);
            let to = c - eye;
            let cam = Camera { far: 9000.0, ..Camera::new(eye, to.x.atan2(-to.z), (to.y / Vec3::new(to.x, 0.0, to.z).length()).atan(), 16.0 / 9.0) };
            // Twice: the switch has hysteresis; the second frame reads the first's state.
            r.cull_readback(&ctx, &cam).unwrap();
            let out = r.cull_readback(&ctx, &cam).unwrap();
            // the drawn commands: the readback's compacted list holds only
            // them; the uncompacted fallback (no multi-draw-indirect-count)
            // holds one slot per object, an undrawn one with no instance.
            let drawn: Vec<u32> = out.draws.iter().filter(|d| d[1] > 0).map(|d| d[4]).collect();
            let near = drawn.iter().filter(|&&i| scene.objects[i as usize].lod_group == li as u32 && scene.objects[i as usize].lod_role == LOD_LANDMARK_NEAR).count();
            let far = drawn.iter().filter(|&&i| scene.objects[i as usize].lod_group == li as u32 && scene.objects[i as usize].lod_role == LOD_LANDMARK_FAR).count();
            counts.push((d, near, far));
        }
        println!("{}: (distance, near objects drawn, far objects drawn) {counts:?}", l.id);
        assert!(counts[0].1 > 0 && counts[0].2 == 0, "{}: near at 600 m draws the near meshes only", l.id);
        assert!(counts[1].1 == 0 && counts[1].2 > 0, "{}: at 4 km only the far item", l.id);
    }
    println!(
        "objects {}, near {} / far {} / wire {} triangles; wall chunks {} (near {} / far {} triangles), always {}",
        st.objects, st.near_triangles, st.far_triangles, st.wire_triangles, set.walls.len(), st.wall_near_triangles, st.wall_far_triangles, st.always_triangles
    );
}

/// The surface kinds ride in material flag bits 25-27 and, for the fourth
/// bit, 31 (`surface_flags`): every kind has its own bits, none touches the
/// landmark, wet-ground, floodlight or vertex-colour flags, and the authored
/// facade (7) and the Golden Gate's far item (8) stay apart.
#[test]
fn the_surface_kinds_keep_their_own_flag_bits() {
    use eo_render::landmarks::*;
    let others = F_LANDMARK | F_WET_GROUND | F_LM_FLOOD | eo_render::authored::F_VERTEX_COLOUR;
    let mut seen = std::collections::HashSet::new();
    for k in 0..=SURFACE_FLOODLIT_FAR {
        let f = surface_flags(k);
        assert_eq!(f & others, 0, "kind {k}");
        assert!(seen.insert(f), "kind {k} has its own bits");
    }
    assert_ne!(SURFACE_AUTHORED, SURFACE_FLOODLIT_FAR);
    // the decks' glazing (`DRAW_GLASS_PANE`): the glass and blend bits, none of the landmark words.
    let pane = eo_render::authored::F_GLASS | eo_render::authored::F_BLEND;
    assert_eq!(pane & (others | surface_flags(15)), 0, "the glass pane's bits");
    assert_eq!(surface_code(Some("authored")), SURFACE_AUTHORED);
    assert!(landmarks_constants_wgsl().contains(&format!("const LM_SURFACE_HIGH_SHIFT: u32 = {SURFACE_HIGH_SHIFT}u;")));
}

#[test]
fn the_wgsl_constants_are_the_js_rules() {
    let js = eo_landmarks::LandmarksJs::get();
    let c = eo_render::landmarks::cull_constants_wgsl();
    assert!(c.contains(&format!("LM_NEAR_M: f32 = {:.3}", js.rules.near_m)) && c.contains(&format!("LM_HYSTERESIS_M: f32 = {:.3}", js.rules.hysteresis_m)));
    let s = eo_render::landmarks::landmarks_constants_wgsl();
    assert!(s.contains(&format!("LM_FOG_CAP_M: f32 = {:.6}", js.rules.fog_cap_m)));
    assert!(s.contains(&format!("LM_WIRE_MIN_PIXELS: f32 = {:.6}", js.surfaces.wire_min_pixels)));
}

/// A landmark light enters the sky's gather as the props pass reads a JS
/// practical: the source lifted, aimed down, the cone and penumbra, the
/// intensity in candela; the JS bridges carry none (the set as the first
/// landmarks slice built it), and (landmarks2) the bridges' night lights
/// enter at their documented intensities: L-864 aviation red at the tower
/// tops, the deck luminaires as the street lamps' pools, the necklace.
#[test]
fn landmark_lights_join_the_sky_gather() {
    let l = eo_landmarks::LandmarkLight { pos: [10.0, 70.0, -5.0], base: [10.0, 60.0, -5.0], colour_linear: [1.0, 0.8, 0.6], intensity: 150.0, range_m: 40.0, decay: 2.0, angle: 0.9, penumbra: 0.5, source_lift_m: 2.0 };
    let s = eo_render::landmarks::sky_lamps(&[l])[0];
    assert_eq!(s.pos, [10.0, 72.0, -5.0]);
    assert!((s.candela - 150.0 / eo_sky::night::lamp_units_per_lux()).abs() < 1e-9);
    let angle = 0.9f32 as f64; // the record's f32 cone, widened
    assert!((s.cos_outer - angle.cos()).abs() < 1e-12 && (s.cos_inner - (angle * 0.5).cos()).abs() < 1e-12);
    assert_eq!((s.dir, s.range, s.kind), ([0.0, -1.0, 0.0], 40.0, eo_sky::lights::LampKind::Street));
    let omni = eo_render::landmarks::sky_lamps(&[eo_landmarks::LandmarkLight { angle: std::f32::consts::FRAC_PI_2, ..l }])[0];
    assert_eq!(omni.kind, eo_sky::lights::LampKind::Point);
    let paths = eo_data::DataPaths::discover().unwrap();
    let elevation = eo_data::ElevationGrid::load(&paths).unwrap();
    let ca = eo_data::CaStandards::load(&paths).unwrap();
    let off = eo_landmarks::LandmarkSet::build_with(&paths.repo_root, &|x, z| elevation.sample(x, z), &ca, eo_landmarks::set::Options::off()).unwrap();
    assert!(off.lights.is_empty(), "the JS bridges carry no lights");
    // the bridges' lights.
    let set = eo_landmarks::LandmarkSet::build(&paths.repo_root, &|x, z| elevation.sample(x, z), &ca).unwrap();
    let lamps = eo_render::landmarks::sky_lamps(&set.lights);
    let red = lamps.iter().filter(|l| l.kind == eo_sky::lights::LampKind::Point && l.color[0] > 3.0 * l.color[1]).collect::<Vec<_>>();
    let deck = lamps.iter().filter(|l| l.kind == eo_sky::lights::LampKind::Street).collect::<Vec<_>>();
    let necklace = lamps.len() - red.len() - deck.len();
    println!("bridge lights: {} aviation, {} deck, {necklace} necklace", red.len(), deck.len());
    // Four Golden Gate legs, two legs on each of the Bay Bridge's four towers.
    assert_eq!(red.len(), 4 + 8);
    for l in &red {
        assert!((l.candela - eo_landmarks::lights::AVIATION_RED_CD).abs() < 1e-3 * eo_landmarks::lights::AVIATION_RED_CD, "L-864: {} cd", l.candela);
        assert!(l.pos[1] > 150.0, "an aviation light stands on a tower top, not at {}", l.pos[1]);
    }
    // A deck lamp every 50 m along both edges of both decks (2.7 + 2.9 km as
    // surveyed; the decks span the scaled shores, s x as long).
    let s = eo_data::groundscale::scale();
    assert!(deck.len() as f64 > 200.0 * s && (deck.len() as f64) < 260.0 * s, "{} deck lamps (x{s} plan)", deck.len());
    let nadir = eo_sky::photometry::nadir_candela_for_pool(eo_landmarks::lights::DECK_HEAD_M);
    assert!(deck.iter().all(|l| (l.candela - nadir).abs() < 1e-3 * nadir && l.pos[1] > 10.0));
    assert!(necklace > 300, "{necklace} necklace lamps on the west span's main cables");
}
