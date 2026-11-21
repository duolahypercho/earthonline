//! The grade-separated roads in the renderer (`--structures`). Needs a GPU.
//!
//!  - the structures' concrete casts sun shadows at exactly the level its
//!    group shows (far until the near pack is resident, then near), the deck
//!    surface (kind 11) and the street (kind 6) on the same roles never;
//!  - on the real city: every tile's group sphere holds all three levels'
//!    geometry; with the near packs streaming in, around a built deck and
//!    after a jump that evicts them, every structure object the cull pass
//!    draws is at its group's level and no group is drawn at two levels at
//!    once; near packs become resident and drawn where the camera is; the
//!    near part of the arena never fills; the drawn set equals a CPU
//!    reference (the group's level, then the frustum) over the same table.

use eo_render::authored::GpuLodGroup;
use eo_render::camera::Camera;
use eo_render::scene::{GpuObject, LOD_STREET_COARSE, LOD_STREET_FAR, LOD_STREET_NEAR, ObjectKind, Vertex};
use eo_render::structures::{Placement, StructurePrepared, StructuresArena, StructuresRuntime};
use eo_render::upload::{Target, UploadQueue};
use eo_render::{GpuContext, GpuOptions, Renderer, RendererInit, Targets, read_mapped, wgpu};
use eo_street::{BuildingIndex, StreetCity, StreetOptions, StreetWorld, Terrain};
use glam::Vec3;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

fn ctx() -> GpuContext {
    let opts = GpuOptions::default();
    GpuContext::new(GpuContext::create_instance(&opts), None, opts).expect("a GPU adapter is required")
}

/// `n` boxes side by side as one object (12 triangles each).
fn boxes(min: Vec3, n: usize, kind: ObjectKind, verts: &mut Vec<Vertex>, idx: &mut Vec<u32>, objects: &mut Vec<GpuObject>) -> usize {
    let base = verts.len() as i32;
    let first = idx.len() as u32;
    let size = Vec3::new(2.0, 6.0, 2.0);
    for b in 0..n {
        let o = min + Vec3::new(3.0 * b as f32, 0.0, 0.0);
        let at = verts.len() as u32 - base as u32;
        for i in 0..8 {
            let p = o + Vec3::new(if i & 1 != 0 { size.x } else { 0.0 }, if i & 2 != 0 { size.y } else { 0.0 }, if i & 4 != 0 { size.z } else { 0.0 });
            verts.push(Vertex { pos: p.to_array(), normal: 0, color: 0xff80_8080 });
        }
        for f in [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]] {
            idx.extend_from_slice(&[at + f[0], at + f[1], at + f[2], at + f[0], at + f[2], at + f[3]]);
        }
    }
    let max = min + Vec3::new(3.0 * n as f32 - 1.0, size.y, size.z);
    objects.push(GpuObject::new(kind, min.to_array(), max.to_array(), first, 36 * n as u32, base));
    objects.len() - 1
}

/// The triangles each refreshed cascade's casters hold after one frame.
fn shadow_triangles(ctx: &GpuContext, r: &Renderer, cam: &Camera) -> Vec<u32> {
    let d = &ctx.device;
    let (w, h) = (160, 90);
    let out = d.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let ov = out.create_view(&Default::default());
    let targets = Targets::new(d, w, h, wgpu::TextureFormat::Rgba8UnormSrgb, 1);
    r.update(&ctx.queue, cam);
    let mut enc = d.create_command_encoder(&Default::default());
    r.encode(&mut enc, &ov, None, &targets.depth, None);
    let n = (eo_render::look::SHADOW_COUNTER_WORDS * 4) as u64;
    let staging = d.create_buffer(&wgpu::BufferDescriptor { label: None, size: n, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    enc.copy_buffer_to_buffer(r.look.shadow_counter_buffer(), 0, &staging, 0, n);
    ctx.queue.submit([enc.finish()]);
    let words: Vec<u32> = bytemuck::pod_collect_to_vec(&read_mapped(ctx, &staging).unwrap());
    let mask = r.look.encoded_shadow_mask();
    (0..3).filter(|c| mask & (1 << c) != 0).map(|c| words[3 + c]).collect()
}

#[test]
fn the_concrete_casts_at_the_level_its_group_shows() {
    let ctx = ctx();
    let (mut v, mut i, mut o) = (vec![], vec![], vec![]);
    // Group 0: a structure tile. Its concrete at the near (1 box), far (2
    // boxes) and coarse (3 boxes) levels; its deck surface (kind 11) at the far
    // level, which must never cast. Group 1: a street tile (kind 6), never casting.
    let near = boxes(Vec3::new(0.0, 0.0, -20.0), 1, ObjectKind::Structure, &mut v, &mut i, &mut o);
    let far = boxes(Vec3::new(0.0, 0.0, -20.0), 2, ObjectKind::Structure, &mut v, &mut i, &mut o);
    let coarse = boxes(Vec3::new(0.0, 0.0, -20.0), 3, ObjectKind::Structure, &mut v, &mut i, &mut o);
    let deck = boxes(Vec3::new(0.0, 0.0, -10.0), 4, ObjectKind::StructureDeck, &mut v, &mut i, &mut o);
    let street = boxes(Vec3::new(0.0, 0.0, -30.0), 5, ObjectKind::Street, &mut v, &mut i, &mut o);
    for (k, role, group) in [(near, LOD_STREET_NEAR, 0), (far, LOD_STREET_FAR, 0), (coarse, LOD_STREET_COARSE, 0), (deck, LOD_STREET_FAR, 0), (street, LOD_STREET_FAR, 1)] {
        o[k].lod_role = role;
        o[k].lod_group = group;
    }
    let group = |c: Vec3| GpuLodGroup { center: c.to_array(), radius: 12.0, ready: 0, detailed: 2, kind: eo_render::street::GROUP_KIND_STREET, level: 1 };
    let groups = [group(Vec3::new(4.0, 3.0, -19.0)), group(Vec3::new(7.0, 3.0, -29.0))];
    let materials = [eo_render::authored::GpuMaterial::vertex_colour()];
    let r = Renderer::from_init(
        &ctx,
        &RendererInit {
            vertices: &v,
            indices: &i,
            objects: &o,
            extra_vertices: 0,
            extra_indices: 0,
            materials: &materials,
            groups: &groups,
            hlod_enter_m: 0.0,
            hlod_exit_m: 0.0,
            kit_layers: 0,
            kit_res: 1,
            color_format: wgpu::TextureFormat::Rgba8UnormSrgb,
            sample_count: 1,
            multiplier: 1,
            debug_ids: false,
            blend_first_object: o.len() as u32,
        },
    )
    .unwrap();
    r.look.set_time_of_day(12.0);
    let cam = Camera::new(Vec3::new(5.0, 8.0, 10.0), 0.0, -0.3, 16.0 / 9.0);
    // Near the camera but not resident: the far level shows, and only it casts (2 boxes, 24 triangles).
    for _ in 0..2 {
        let t = shadow_triangles(&ctx, &r, &cam);
        assert!(!t.is_empty() && t.iter().all(|&n| n == 24 || n == 0) && t.contains(&24), "far level: {t:?}");
    }
    // The near pack resident (its ready bit): only the near level casts (12 triangles).
    r.write(&ctx.queue, Target::Group, 16, bytemuck::bytes_of(&1u32));
    for _ in 0..2 {
        let t = shadow_triangles(&ctx, &r, &cam);
        assert!(!t.is_empty() && t.iter().all(|&n| n == 12 || n == 0) && t.contains(&12), "near level: {t:?}");
    }
    // Far away (past the mid radius) the coarse level shows: nothing of it is in a cascade box near the camera,
    // and nothing casts at two levels: every count is one level's or none.
    let away = Camera::new(Vec3::new(5.0, 8.0, 3000.0), std::f32::consts::PI, -0.3, 16.0 / 9.0);
    let t = shadow_triangles(&ctx, &r, &away);
    assert!(t.iter().all(|&n| n == 0 || n == 36), "coarse level: {t:?}");
    let groups = r.groups_readback(&ctx).unwrap();
    assert_eq!(groups[0].level, 2, "the far camera puts the tile at its coarse level");
}

struct City {
    ctx: GpuContext,
    renderer: Renderer,
    prepared: StructurePrepared,
    placement: Placement,
    runtime: StructuresRuntime,
    uploads: UploadQueue,
}

fn city() -> City {
    let paths = eo_data::DataPaths::discover().unwrap();
    let city = Arc::new(eo_data::City::load(&paths).unwrap());
    let elev = eo_data::ElevationGrid::load(&paths).unwrap();
    let ca = eo_data::CaStandards::load(&paths).unwrap();
    let o = StreetOptions::load().unwrap();
    let lift = o.open_ground_lift;
    let rings: Vec<Vec<[f64; 2]>> = city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
    let sw = StreetWorld::build(o, StreetCity::load().unwrap(), Terrain { grid: Arc::new(elev.clone()), carpet_lift: lift }, BuildingIndex::new(rings));
    let t = std::time::Instant::now();
    let world = Arc::new(eo_structures::StructureWorld::build(&sw, &city, &ca, eo_landmarks::LandmarksJs::get(), Default::default()).unwrap());
    assert!(!world.is_empty(), "the city builds no structure");
    let prepared = StructurePrepared::prepare(world, eo_render::street::StreetRadii::default(), 8);
    eprintln!(
        "structures built and prepared in {:.1} s: {} tiles, {} tints, triangles near {} / far {} / coarse {}, static {} objects, near bound {:?}, arena {:?}",
        t.elapsed().as_secs_f64(),
        prepared.set.tiles.len(),
        prepared.set.tints.rgb.len(),
        prepared.set.totals.triangles[0],
        prepared.set.totals.triangles[1],
        prepared.set.totals.triangles[2],
        prepared.statics.objects.len(),
        prepared.set.near_bound,
        prepared.arena_needs()
    );
    let ctx = ctx();
    let (av, ai, ao) = prepared.arena_needs();
    let arena = StructuresArena { vertex_base: 0, vertex_capacity: av, index_base: 0, index_capacity: ai, object_base: 0, object_capacity: ao };
    let placement = Placement { group_base: 0, street_material: 1, concrete_material: 2 };
    let mut materials = vec![eo_render::authored::GpuMaterial::vertex_colour(), eo_render::street::street_material()];
    materials.extend(prepared.materials());
    let objects = prepared.objects(placement, arena);
    let groups = prepared.groups();
    let renderer = Renderer::from_init(
        &ctx,
        &RendererInit {
            vertices: &[],
            indices: &[],
            objects: &objects,
            extra_vertices: av,
            extra_indices: ai,
            materials: &materials,
            groups: &groups,
            hlod_enter_m: 0.0,
            hlod_exit_m: 0.0,
            kit_layers: 0,
            kit_res: 1,
            color_format: wgpu::TextureFormat::Rgba8UnormSrgb,
            sample_count: 1,
            multiplier: 1,
            debug_ids: false,
            blend_first_object: objects.len() as u32,
        },
    )
    .unwrap();
    renderer.attach_structures(&ctx, &prepared, placement, arena);
    let runtime = StructuresRuntime::start(prepared.set.clone(), placement, prepared.near_arena(arena), 4, Vec3::ZERO);
    City { ctx, renderer, prepared, placement, runtime, uploads: UploadQueue::new(4 << 20) }
}

fn frame(c: &mut City, cam: &Camera) {
    c.runtime.begin_frame();
    c.runtime.update(&c.renderer, &c.ctx.queue, &mut c.uploads, cam.eye);
    let (_, done) = c.uploads.pump(&c.renderer, &c.ctx.queue);
    c.runtime.after_pump(&c.renderer, &c.ctx.queue, &done);
    c.renderer.update(&c.ctx.queue, cam);
    c.ctx.queue.submit([]);
}

fn settle(c: &mut City, cam: &Camera, mut check: impl FnMut(&mut City)) {
    for k in 0..20_000 {
        frame(c, cam);
        if k % 20 == 0 {
            check(c);
        }
        if c.runtime.settled() && c.uploads.pending() == 0 {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("the near packs did not settle");
}

/// Per structure group: the objects drawn by role (near, far, coarse);
/// asserts no group is drawn at two levels and every drawn object is at its
/// group's level. Returns (groups drawn, groups drawn near).
fn exclusive(c: &City, cam: &Camera) -> (usize, usize) {
    let out = c.renderer.cull_readback(&c.ctx, cam).unwrap();
    let objects = c.renderer.objects_readback(&c.ctx).unwrap();
    let groups = c.renderer.groups_readback(&c.ctx).unwrap();
    let mut by_group: HashMap<u32, [u32; 3]> = HashMap::new();
    for d in out.draws.iter().filter(|d| d[1] == 1) {
        let o = &objects[d[4] as usize];
        let kind = o.object_kind();
        assert!(kind == ObjectKind::Structure as u32 || kind == ObjectKind::StructureDeck as u32, "object {} of kind {kind} drawn", d[4]);
        let level = (o.lod_role - LOD_STREET_NEAR) as usize;
        assert!(level < 3, "object {} drawn with role {}", d[4], o.lod_role);
        assert_eq!(groups[o.lod_group as usize].level as usize, level, "object {} of group {} drawn at level {level}, its group shows {}", d[4], o.lod_group, groups[o.lod_group as usize].level);
        by_group.entry(o.lod_group).or_default()[level] += 1;
    }
    for (g, n) in &by_group {
        assert!(n.iter().filter(|x| **x > 0).count() <= 1, "group {g} drawn at two levels at once: {n:?}");
    }
    // the GPU's drawn set is the CPU reference's: every object at its group's level in the frustum.
    let planes = eo_render::camera::frustum_planes(cam.view_proj());
    let cpu: HashSet<u32> = objects
        .iter()
        .enumerate()
        .filter(|(_, o)| o.index_count > 0 && (LOD_STREET_NEAR..=LOD_STREET_COARSE).contains(&o.lod_role))
        .filter(|(_, o)| groups[o.lod_group as usize].level == o.lod_role - LOD_STREET_NEAR)
        .filter(|(_, o)| eo_render::camera::aabb_visible(&planes, Vec3::from(o.min), Vec3::from(o.max)))
        .map(|(i, _)| i as u32)
        .collect();
    let gpu: HashSet<u32> = out.draws.iter().filter(|d| d[1] == 1).map(|d| d[4]).collect();
    let diff = cpu.symmetric_difference(&gpu).count();
    assert!(diff <= 4, "the GPU drew {} structure objects, the CPU reference {} ({diff} differ)", gpu.len(), cpu.len());
    // the cull counters' structure triangles are what was drawn.
    let tris: u32 = out.draws.iter().filter(|d| d[1] == 1).map(|d| d[0] / 3).sum();
    assert_eq!(out.counters[15], tris, "counter 15 holds the structures' triangles drawn");
    (by_group.len(), by_group.values().filter(|n| n[0] > 0).count())
}

#[test]
fn real_city_structures_draw_each_tile_at_one_level_while_streaming() {
    let mut c = city();
    let set = c.prepared.set.clone();
    // Every tile's group sphere holds its geometry at all three levels.
    for (t, tile) in set.tiles.iter().enumerate() {
        let g = set.group(t);
        let ctr = Vec3::from(g.center);
        for lod in 0..3u8 {
            let Some(m) = set.world.render_tile_shared(tile.key, lod) else { continue };
            for p in &m.positions {
                let d = Vec3::from(*p).distance(ctr);
                assert!(d <= g.radius + 1e-3, "tile {:?} level {lod}: a vertex {d:.2} m from the centre, radius {:.2}", tile.key, g.radius);
            }
        }
    }
    // A camera over the longest built deck, beside it, looking at it.
    let chain = set.world.chains().iter().filter(|c| c.is_built() && c.kind == eo_structures::ChainKind::Deck).max_by(|a, b| a.length().total_cmp(&b.length())).unwrap();
    let f = chain.frame(chain.length() / 2.0);
    let deck = Vec3::new(f.x as f32, chain.y_at(chain.length() / 2.0) as f32, f.z as f32);
    let n = Vec3::new(f.nx as f32, 0.0, f.nz as f32);
    let eye = deck + n * 40.0 + Vec3::Y * 6.0;
    let to = deck - eye;
    let cam = Camera::new(eye, to.x.atan2(-to.z), -0.15, 16.0 / 9.0);
    eprintln!("camera at ({:.0}, {:.0}, {:.0}) beside chain {} ({}, {:.0} m)", eye.x, eye.y, eye.z, chain.id, chain.name(), chain.length());
    let mut seen = 0;
    settle(&mut c, &cam, |c| {
        exclusive(c, &cam);
    });
    for _ in 0..3 {
        frame(&mut c, &cam);
    }
    let (drawn, near) = exclusive(&c, &cam);
    seen += drawn;
    eprintln!("beside the deck: {drawn} tiles drawn, {near} at the near level; {} near packs resident ({} triangles)", c.runtime.stats.resident, c.runtime.stats.resident_triangles);
    assert!(near > 0, "no tile drawn at the near level beside a built deck");
    assert!(c.runtime.stats.resident > 0);
    // Turning round on the spot: still one level a tile.
    for k in 1..=6 {
        let turn = Camera::new(eye, cam.yaw + std::f32::consts::PI * k as f32 / 6.0, -0.15, 16.0 / 9.0);
        frame(&mut c, &turn);
        seen += exclusive(&c, &turn).0;
    }
    // A 2 km jump: the near packs are evicted (a few a frame) while new ones stream in.
    let far_eye = eye + Vec3::new(1500.0, 300.0, 1500.0);
    let far = Camera::new(far_eye, -2.3, -0.4, 16.0 / 9.0);
    let evicted = c.runtime.stats.packs_evicted;
    settle(&mut c, &far, |c| {
        exclusive(c, &far);
    });
    let (drawn_far, _) = exclusive(&c, &far);
    eprintln!(
        "after the jump: {drawn_far} tiles drawn; {} packs evicted; arena use {:?}; {} deferred for a full arena",
        c.runtime.stats.packs_evicted - evicted,
        c.runtime.arena_use(),
        c.runtime.stats.packs_skipped_arena_full
    );
    assert!(c.runtime.stats.packs_evicted > evicted, "the jump evicted nothing");
    assert_eq!(c.runtime.stats.packs_skipped_arena_full, 0, "the near part of the arena filled");
    assert!(seen > 0);
    let _ = c.placement;
}
