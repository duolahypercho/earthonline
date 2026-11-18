//! The Blender-built buildings in the renderer: model and footprint are
//! never drawn together, a footprint stands in until the model is resident,
//! the LOD hysteresis follows the JS radii, and the material a pixel is shaded
//! with is its own object's (a planted swap is caught). Needs a GPU.

use eo_assets::kit::{self, KitLibrary};
use eo_assets::placement::PlacementConstants;
use eo_render::authored::{ArenaBase, AuthoredSet, model_upload};
use eo_render::camera::Camera;
use eo_render::scene::{GpuObject, LOD_FAR, LOD_NEAR, ObjectKind, SceneInputs, build_scene};
use eo_render::{GpuContext, GpuOptions, IDS_FORMAT, KitUpload, Renderer, RendererInit, Targets, read_mapped, wgpu};
use glam::Vec3;

/// the spawn on the shipped (scaled) plan.
static SPAWN: std::sync::LazyLock<[f32; 2]> = std::sync::LazyLock::new(|| eo_data::groundscale::plan32([1539.0, -366.0]));

struct Fixture {
    ctx: GpuContext,
    renderer: Renderer,
    set: AuthoredSet,
    objects: Vec<GpuObject>,
    elev: eo_data::ElevationGrid,
    loaded: Vec<usize>,
}

fn fixture(debug_ids: bool, load_within_m: f32) -> Fixture {
    let paths = eo_data::DataPaths::discover().unwrap();
    let city = eo_data::City::load(&paths).unwrap();
    let elev = eo_data::ElevationGrid::load(&paths).unwrap();
    let ca = eo_data::CaStandards::load(&paths).unwrap();
    let kit_lib = KitLibrary::load().unwrap();
    let constants = PlacementConstants::load().unwrap();
    let set = AuthoredSet::prepare(&paths.repo_root, &city, &elev, &kit_lib, &constants).unwrap();
    let scene = build_scene(&SceneInputs {
        city: &city,
        elevation: &elev,
        land: None,
        standards: &ca,
        authored: &set.group_of_record,
        terrain_lod: true,
        kit_groups: &[],
        carpet: None,
        street: None,
        skirts: &[],
        box_floor: None,
        skip_building: None,
        ground_tone: None,
        smooth_terrain: false,
    })
    .unwrap();
    let opts = GpuOptions::default();
    let ctx = GpuContext::new(GpuContext::create_instance(&opts), None, opts).expect("a GPU adapter is required");
    let base = ArenaBase {
        vertex: scene.vertices.len() as u32,
        index: scene.indices.len() as u32,
        object: scene.objects.len() as u32,
    };
    let mut objects = scene.objects.clone();
    objects.extend(set.placeholder_objects());
    let renderer = Renderer::from_init(
        &ctx,
        &RendererInit {
            vertices: &scene.vertices,
            indices: &scene.indices,
            objects: &objects,
            extra_vertices: set.vertex_capacity,
            extra_indices: set.index_capacity,
            materials: &set.materials.gpu,
            groups: &set.groups,
            hlod_enter_m: set.hlod_enter_m,
            hlod_exit_m: set.hlod_exit_m,
            kit_layers: set.materials.layers.len() as u32,
            kit_res: kit::HIGH_RES as u32,
            color_format: wgpu::TextureFormat::Rgba8UnormSrgb,
            sample_count: 1,
            multiplier: 1,
            debug_ids,
            blend_first_object: base.object,
        },
    )
    .unwrap();
    let cache = eo_assets::default_cache_dir();
    let maps = kit::bake_all(&kit_lib, &set.materials.layers, kit::HIGH_RES, Some(&cache)).unwrap();
    renderer.upload_kit(&ctx.queue, &KitUpload::build(&maps)).unwrap();
    let spawn = Vec3::new(SPAWN[0], 0.0, SPAWN[1]);
    let mut loaded = vec![];
    for (i, b) in set.buildings.iter().enumerate() {
        let c = Vec3::from(set.groups[b.group as usize].center);
        if Vec3::new(c.x, 0.0, c.z).distance(spawn) > load_within_m {
            continue;
        }
        let (model, _) = eo_assets::load_model(&paths.repo_root, Some(&cache), &b.asset).unwrap();
        let up = model_upload(&set, i, &model, base, &set.materials.gpu).unwrap();
        let at = base.object + b.slot.object_offset;
        renderer.upload_model(
            &ctx.queue,
            base.vertex + b.slot.vertex_offset,
            &up.vertices,
            base.index + b.slot.index_offset,
            &up.indices,
            at,
            &up.objects,
            b.group,
        );
        objects[at as usize..at as usize + up.objects.len()].copy_from_slice(&up.objects);
        loaded.push(i);
    }
    Fixture { ctx, renderer, set, objects, elev, loaded }
}

fn drawn(out: &eo_render::CullReadback) -> std::collections::HashSet<u32> {
    out.draws.iter().chain(&out.blend_draws).filter(|d| d[1] == 1).map(|d| d[4]).collect()
}

#[test]
fn model_and_footprint_are_exclusive_and_lod_follows_the_js_radii() {
    let f = fixture(false, 250.0);
    assert!(f.loaded.len() >= 5, "only {} models near the spawn", f.loaded.len());
    let ground = f.elev.ground(SPAWN[0] as f64, SPAWN[1] as f64) as f32;
    let eye = Vec3::new(SPAWN[0], ground + 1.7, SPAWN[1]);
    let mut checked_models = 0;
    let mut checked_standins = 0;
    for yaw in [0.0f32, 0.8, 1.6, 2.4, 3.2, 4.0, 4.8, 5.6] {
        let cam = Camera::new(eye, yaw, 0.0, 16.0 / 9.0);
        let out = f.renderer.cull_readback(&f.ctx, &cam).unwrap();
        let groups = f.renderer.groups_readback(&f.ctx).unwrap();
        let on = drawn(&out);
        let mut footprint_on = vec![false; groups.len()];
        let mut parts_on = vec![false; groups.len()];
        for &i in &on {
            let o = &f.objects[i as usize];
            match o.lod_role {
                LOD_FAR => footprint_on[o.lod_group as usize] = true,
                LOD_NEAR => parts_on[o.lod_group as usize] = true,
                _ => {}
            }
        }
        for (g, s) in groups.iter().enumerate() {
            assert!(!(footprint_on[g] && parts_on[g]), "group {g}: footprint and model drawn together");
            if s.ready == 0 {
                assert!(!parts_on[g], "group {g}: model drawn before it was resident");
            }
            if s.ready == 1 && s.detailed == 1 {
                assert!(!footprint_on[g], "group {g}: footprint drawn although its model is resident and near");
            }
            checked_models += parts_on[g] as usize;
            checked_standins += (footprint_on[g] && s.ready == 0) as usize;
        }
        // The counters agree with the command lists.
        let model_cmds = on.iter().filter(|&&i| f.objects[i as usize].object_kind() == ObjectKind::Model as u32).count();
        assert_eq!(out.counters[3] as usize, model_cmds);
    }
    assert!(checked_models > 0, "no resident model was ever drawn");
    assert!(checked_standins > 0, "no footprint ever stood in for a model that is not resident");

    // Hysteresis on one resident building: start detailed, sit between the
    // radii (stays detailed), leave past the exit radius, come back between
    // the radii (stays far), come inside the enter radius (detailed again).
    let b = &f.set.buildings[f.loaded[0]];
    let g = f.set.groups[b.group as usize];
    let c = Vec3::from(g.center);
    let at = |surface_m: f32| Camera::new(c + Vec3::new(0.0, 0.0, g.radius + surface_m), 0.0, 0.0, 16.0 / 9.0);
    let state = |cam: Camera| {
        f.renderer.cull_readback(&f.ctx, &cam).unwrap();
        f.renderer.groups_readback(&f.ctx).unwrap()[b.group as usize].detailed
    };
    let (enter, exit) = (f.set.hlod_enter_m, f.set.hlod_exit_m);
    assert_eq!((enter, exit), (320.0, 380.0));
    assert_eq!(state(at(100.0)), 1);
    assert_eq!(state(at(350.0)), 1, "left the detailed level inside the exit radius");
    assert_eq!(state(at(400.0)), 0);
    assert_eq!(state(at(350.0)), 0, "entered the detailed level outside the enter radius");
    assert_eq!(state(at(300.0)), 1);
}

/// Render (object, material) ids and compare every model pixel's material
/// with its object's in the CPU table. Returns (model pixels, mismatches,
/// pixel count per object).
fn check_pixels(f: &Fixture, cam: &Camera, w: u32, h: u32) -> (usize, usize, std::collections::HashMap<u32, usize>) {
    let d = &f.ctx.device;
    let mk = |format, usage| {
        d.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    let color = mk(wgpu::TextureFormat::Rgba8UnormSrgb, wgpu::TextureUsages::RENDER_ATTACHMENT);
    let ids = mk(IDS_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC);
    let targets = Targets::new(d, w, h, wgpu::TextureFormat::Rgba8UnormSrgb, 1);
    let row = (w * 8).div_ceil(256) * 256;
    let buf = d.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (row * h) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let (cv, iv) = (color.create_view(&Default::default()), ids.create_view(&Default::default()));
    for _ in 0..2 {
        f.renderer.update(&f.ctx.queue, cam);
        let mut enc = d.create_command_encoder(&Default::default());
        f.renderer.encode_with_ids(&mut enc, &cv, None, &targets.depth, Some(&iv), None);
        f.ctx.queue.submit([enc.finish()]);
    }
    let mut enc = d.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        ids.as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    f.ctx.queue.submit([enc.finish()]);
    let bytes = read_mapped(&f.ctx, &buf).unwrap();
    let (mut model_px, mut bad) = (0, 0);
    let mut per_object = std::collections::HashMap::new();
    for y in 0..h as usize {
        for x in 0..w as usize {
            let o = y * row as usize + x * 8;
            let obj = u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
            let mat = u32::from_le_bytes(bytes[o + 4..o + 8].try_into().unwrap());
            if obj == u32::MAX {
                continue;
            }
            let cpu = &f.objects[obj as usize];
            if cpu.object_kind() != ObjectKind::Model as u32 {
                continue;
            }
            model_px += 1;
            *per_object.entry(obj).or_insert(0usize) += 1;
            if mat != cpu.material {
                bad += 1;
            }
        }
    }
    (model_px, bad, per_object)
}

#[test]
fn each_pixel_resolves_its_own_objects_material_and_a_planted_swap_is_caught() {
    let f = fixture(true, 120.0);
    // Stand in front of the nearest loaded building, looking at its front.
    let spawn = Vec3::new(SPAWN[0], 0.0, SPAWN[1]);
    let b = f
        .loaded
        .iter()
        .map(|&i| &f.set.buildings[i])
        .min_by(|a, b| {
            let d = |x: &eo_render::authored::AuthoredBuilding| Vec3::from(f.set.groups[x.group as usize].center).distance(spawn);
            d(a).partial_cmp(&d(b)).unwrap()
        })
        .unwrap();
    let m = b.placement.world_from_model.as_mat4();
    let front = m.transform_vector3(Vec3::Z).normalize();
    let centre = m.transform_point3(Vec3::new(0.0, 4.0, (b.placement.frame.depth / 2.0) as f32 + 14.0));
    let look = -front;
    let cam = Camera::new(centre, look.x.atan2(-look.z), 0.15, 16.0 / 9.0);
    let (w, h) = (960, 540);
    let (px, bad, per_object) = check_pixels(&f, &cam, w, h);
    let materials: std::collections::HashSet<u32> = per_object.keys().map(|o| f.objects[*o as usize].material).collect();
    eprintln!("{} looked at: {px} model pixels, {} objects, {} materials, {bad} mismatches", b.id, per_object.len(), materials.len());
    assert!(px > 20_000, "the building does not fill the frame ({px} model pixels)");
    assert!(materials.len() >= 3, "too few materials in view to test ({})", materials.len());
    assert_eq!(bad, 0, "pixels shaded with another object's material");

    // Plant a swap on the GPU only: the two biggest objects in view with
    // different materials exchange material indices.
    let mut by_size: Vec<(u32, usize)> = per_object.into_iter().collect();
    by_size.sort_by_key(|e| std::cmp::Reverse(e.1));
    let (a, a_px) = by_size[0];
    let (bb, _) = *by_size.iter().find(|(o, _)| f.objects[*o as usize].material != f.objects[a as usize].material).unwrap();
    let (ma, mb) = (f.objects[a as usize].material, f.objects[bb as usize].material);
    f.renderer.debug_set_object_material(&f.ctx.queue, a, mb);
    f.renderer.debug_set_object_material(&f.ctx.queue, bb, ma);
    let (_, bad, _) = check_pixels(&f, &cam, w, h);
    eprintln!("planted swap of objects {a} and {bb}: {bad} mismatching pixels");
    assert!(bad >= a_px, "the planted swap was not caught ({bad} mismatches, object {a} covers {a_px} pixels)");
}
