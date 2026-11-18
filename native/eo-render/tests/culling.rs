//! Culling tests. They need a GPU adapter (the compute shader is the thing
//! under test); without one they fail loudly rather than pass silently.

use eo_render::camera::{Camera, aabb_visible, frustum_planes};
use eo_render::scene::{GpuObject, LOD_TERRAIN, ObjectKind, Vertex};
use eo_render::{GpuContext, GpuOptions, Renderer, TERRAIN_LOD_DISTANCE_M, wgpu};
use glam::Vec3;

fn ctx(force_no_indirect_count: bool, force_no_bounded_draws: bool) -> GpuContext {
    let opts = GpuOptions { force_no_indirect_count, force_no_bounded_draws, ..Default::default() };
    GpuContext::new(GpuContext::create_instance(&opts), None, opts).expect("a GPU adapter is required for the culling test")
}

/// A unit box object at `c` with a distinctive first_index so it can be found.
fn cube(c: Vec3, tag: u32, verts: &mut Vec<Vertex>, objects: &mut Vec<GpuObject>) {
    let base = verts.len() as i32;
    for i in 0..8 {
        let p = c + Vec3::new((i & 1) as f32, ((i >> 1) & 1) as f32, ((i >> 2) & 1) as f32) - Vec3::splat(0.5);
        verts.push(Vertex { pos: p.to_array(), normal: 0, color: 0xffff_ffff });
    }
    objects.push(GpuObject::new(
        ObjectKind::Building,
        (c - Vec3::splat(0.5)).to_array(),
        (c + Vec3::splat(0.5)).to_array(),
        tag * 3,
        3,
        base,
    ));
}

fn run(force_no_indirect_count: bool, force_no_bounded_draws: bool) {
    let ctx = ctx(force_no_indirect_count, force_no_bounded_draws);
    // Camera at the origin looking north (-z).
    let cam = Camera::new(Vec3::new(0.0, 2.0, 0.0), 0.0, 0.0, 16.0 / 9.0);
    let mut verts = vec![];
    let mut objects = vec![];
    // Visible: straight ahead at 10, 100 and 1000 m.
    cube(Vec3::new(0.0, 2.0, -10.0), 0, &mut verts, &mut objects);
    cube(Vec3::new(0.0, 2.0, -100.0), 1, &mut verts, &mut objects);
    cube(Vec3::new(5.0, 2.0, -1000.0), 2, &mut verts, &mut objects);
    // Planted off-screen: behind, far to the side, above, and beyond the far plane.
    let planted = [
        (Vec3::new(0.0, 2.0, 50.0), 10u32),
        (Vec3::new(500.0, 2.0, -20.0), 11),
        (Vec3::new(0.0, 400.0, -20.0), 12),
        (Vec3::new(0.0, 2.0, -5000.0), 13),
    ];
    for (c, tag) in planted {
        cube(c, tag, &mut verts, &mut objects);
    }
    let indices: Vec<u32> = (0..64).map(|i| i % 8).collect();
    let r = Renderer::from_parts(&ctx, &verts, &indices, &objects, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
    let out = r.cull_readback(&ctx, &cam).unwrap();

    let planes = frustum_planes(cam.view_proj());
    let cpu_visible: Vec<u32> = objects
        .iter()
        .filter(|o| aabb_visible(&planes, Vec3::from(o.min), Vec3::from(o.max)))
        .map(|o| o.first_index / 3)
        .collect();
    assert_eq!(cpu_visible, vec![0, 1, 2], "CPU reference");

    let drawn: Vec<u32> = out.draws.iter().filter(|d| d[1] == 1).map(|d| d[2] / 3).collect();
    let mut drawn_sorted = drawn.clone();
    drawn_sorted.sort();
    assert_eq!(out.draw_count(), 3);
    assert_eq!(out.triangles(), 3);
    assert_eq!(drawn_sorted, vec![0, 1, 2]);
    // first_instance is the object index.
    for d in out.draws.iter().filter(|d| d[1] == 1) {
        assert_eq!(objects[d[4] as usize].first_index, d[2]);
    }
    for (_, tag) in planted {
        assert!(!drawn.contains(&tag), "planted off-screen object {tag} reached the indirect buffer");
    }
    assert!(out.blend_draws.iter().all(|d| d[1] == 0), "an opaque object reached the transparent list");
    if r.compact {
        // Compacted: exactly the visible commands, nothing else.
        assert_eq!(out.draws.len(), 3);
    } else {
        // Fallback: one slot per object, culled slots have instance_count 0.
        assert_eq!(out.draws.len(), objects.len());
        for d in &out.draws {
            if [10, 11, 12, 13].contains(&(d[2] / 3)) {
                assert_eq!(d[1], 0);
            }
        }
    }
}

#[test]
fn planted_offscreen_objects_are_culled_compacted_path() {
    run(false, false);
}

/// The compacted lists drawn with a bounded fixed count (no indirect count).
#[test]
fn planted_offscreen_objects_are_culled_bounded_path() {
    run(true, false);
}

#[test]
fn planted_offscreen_objects_are_culled_fallback_path() {
    run(true, true);
}

/// The bounded lists' guards (bounds.rs) hold the frustum's objects: the cull
/// counts every LOD-selected object within the ring and, of those, every one
/// in the widened frustum, and the visible ones are among them.
#[test]
fn bounded_lists_count_their_guards() {
    let ctx = ctx(true, false);
    let cam = Camera::new(Vec3::new(0.0, 2.0, 0.0), 0.0, 0.0, 16.0 / 9.0);
    let mut verts = vec![];
    let mut objects = vec![];
    // ahead (3), 10 degrees off the frustum's side (in the widened one), behind
    // within the near sphere, behind past it, beyond the ring.
    cube(Vec3::new(0.0, 2.0, -10.0), 0, &mut verts, &mut objects);
    cube(Vec3::new(0.0, 2.0, -100.0), 1, &mut verts, &mut objects);
    cube(Vec3::new(5.0, 2.0, -1000.0), 2, &mut verts, &mut objects);
    let side = (40.9f32 + 10.0).to_radians();
    cube(Vec3::new(side.sin() * 200.0, 2.0, -side.cos() * 200.0), 3, &mut verts, &mut objects);
    cube(Vec3::new(0.0, 2.0, 20.0), 4, &mut verts, &mut objects);
    cube(Vec3::new(0.0, 2.0, 300.0), 5, &mut verts, &mut objects);
    cube(Vec3::new(0.0, 2.0, -6000.0), 6, &mut verts, &mut objects);
    let indices: Vec<u32> = (0..64).map(|i| i % 8).collect();
    let r = Renderer::from_parts(&ctx, &verts, &indices, &objects, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
    if !r.bounded {
        eprintln!("bounded_lists_count_their_guards: the adapter draws with the indirect count; the guards are not counted");
        return;
    }
    let out = r.cull_readback(&ctx, &cam).unwrap();
    assert_eq!(out.draw_count(), 3, "drawn");
    assert_eq!(out.counters[16], 5, "wide guard: the 3 in view, the one 10 degrees off, the one behind inside the near sphere");
    assert_eq!(out.counters[18], 6, "ring guard: all but the one past the far plane plus the margin");
    let fb = r.frame_bounds().expect("bounds planned");
    assert_eq!(fb.city[0], r.object_count, "no sample yet: drawn at capacity");
}

/// Whole real city at the bench spawn and from the air: the GPU's visible
/// set and triangle count equal the CPU reference (frustum test plus the
/// terrain level choice) over the same object table.
#[test]
fn real_city_gpu_cull_matches_cpu_reference() {
    let paths = eo_data::DataPaths::discover().unwrap();
    let city = eo_data::City::load(&paths).unwrap();
    let elev = eo_data::ElevationGrid::load(&paths).unwrap();
    let land = eo_data::LandCover::load(&paths).ok();
    let ca = eo_data::CaStandards::load(&paths).unwrap();
    let none = Default::default();
    let scene = eo_render::scene::build_scene(&eo_render::scene::SceneInputs {
        city: &city,
        elevation: &elev,
        land: land.as_ref(),
        standards: &ca,
        authored: &none,
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
    let ctx = ctx(false, false);
    let r = Renderer::new(&ctx, &scene, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
    // the spawn and the overview point on the shipped (scaled) plan.
    let sp = eo_data::groundscale::plan([1539.0, -366.0]);
    let ov = eo_data::groundscale::plan([0.0, 2500.0]);
    for (eye, yaw, pitch) in [
        (Vec3::new(sp[0] as f32, elev.ground(sp[0], sp[1]) as f32 + 1.7, sp[1] as f32), 0.8f32, 0.0f32),
        (Vec3::new(ov[0] as f32, 400.0, ov[1] as f32), 0.0, -0.3),
    ] {
        let cam = Camera::new(eye, yaw, pitch, 16.0 / 9.0);
        let planes = frustum_planes(cam.view_proj());
        let cpu: Vec<&GpuObject> =
            scene.objects.iter().filter(|o| aabb_visible(&planes, Vec3::from(o.min), Vec3::from(o.max))).collect();
        let cpu_tris: u64 = cpu
            .iter()
            .map(|o| {
                let far = o.lod_role == LOD_TERRAIN && {
                    let c = eye.clamp(Vec3::from(o.min), Vec3::from(o.max));
                    c.distance(eye) > TERRAIN_LOD_DISTANCE_M
                };
                (if far { o.far_index_count } else { o.index_count } / 3) as u64
            })
            .sum();
        let out = r.cull_readback(&ctx, &cam).unwrap();
        // Float rounding at a plane can flip an AABB that touches it exactly;
        // allow a handful.
        let diff = (out.draw_count() as i64 - cpu.len() as i64).abs();
        assert!(diff <= 4, "gpu {} vs cpu {}", out.draw_count(), cpu.len());
        assert!(out.draw_count() > 0 && (out.draw_count() as usize) < scene.objects.len());
        let tri_diff = (out.triangles() as i64 - cpu_tris as i64).abs();
        assert!(tri_diff <= 4 * 2048, "tris gpu {} vs cpu {}", out.triangles(), cpu_tris);
        eprintln!("eye {eye:?}: {} of {} objects visible, {} triangles", out.draw_count(), scene.objects.len(), out.triangles());
    }
}
