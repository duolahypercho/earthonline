//! Kit buildings in the renderer (slice 3), on the real city around the
//! bench's kit street. Needs a GPU.
//!
//!  - a building is never drawn as both its footprint box and kit geometry,
//!    and never at two rungs, while packs stream in, around the street and
//!    after a jump that evicts them;
//!  - no frame writes more than the upload budget;
//!  - every kit pixel is shaded with a material its own object carries
//!    (checked against the packs cooked again on the CPU), and a planted
//!    GPU-side change of an object's material is caught;
//!  - (viewdetail) with the view gate on, near packs are resident only for
//!    tiles in the widened view, a 180 degree turn never shows a box within
//!    the near zone (the radial mid rung stands in), and the old view's near
//!    packs are evicted once past the hold.

use eo_assets::kit::{self, KitLibrary};
use eo_kit::city::KitCity;
use eo_render::authored::{GpuLodGroup, MaterialTable};
use eo_render::camera::Camera;
use eo_render::kit::{CookContext, KitArena, KitRadii, KitRuntime, KitSet, RUNG_NEAR, load_pack};
use eo_render::scene::{GpuObject, LOD_KIT_BOX, LOD_KIT_MID, LOD_KIT_NEAR, ObjectKind, SceneInputs, build_scene};
use eo_render::upload::UploadQueue;
use eo_render::{GpuContext, GpuOptions, IDS_FORMAT, KitUpload, Renderer, RendererInit, Targets, read_mapped, wgpu};
use glam::Vec3;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

const STREET_SURVEYED: [[f32; 2]; 2] = [[-1348.0, 2246.0], [-1369.0, 1949.0]];
/// the street on the shipped (scaled) plan.
static STREET: std::sync::LazyLock<[[f32; 2]; 2]> = std::sync::LazyLock::new(|| STREET_SURVEYED.map(eo_data::groundscale::plan32));
const BUDGET: u64 = 2 << 20;

struct Fixture {
    ctx: GpuContext,
    renderer: Renderer,
    set: Arc<KitSet>,
    kit: KitRuntime,
    uploads: UploadQueue,
    cook: CookContext,
    elev: eo_data::ElevationGrid,
    max_frame_bytes: u64,
}

fn fixture() -> Fixture {
    let paths = eo_data::DataPaths::discover().unwrap();
    let city = Arc::new(eo_data::City::load(&paths).unwrap());
    let elev = eo_data::ElevationGrid::load(&paths).unwrap();
    let ca = eo_data::CaStandards::load(&paths).unwrap();
    let lib = KitLibrary::load().unwrap();
    let mut materials = MaterialTable::new();
    let mut excluded: HashSet<String> = eo_assets::load_blender_buildings(&paths.repo_root).unwrap().iter().map(|b| b.record_key().to_string()).collect();
    for id in eo_kit::city::load_exclusions() {
        excluded.insert(id.trim_start_matches("sf-building-").to_string());
    }
    let set = Arc::new(KitSet::prepare(&city, &elev, &excluded, 0, &mut materials, &lib));
    let scene = build_scene(&SceneInputs {
        city: &city,
        elevation: &elev,
        land: None,
        standards: &ca,
        authored: &HashMap::new(),
        terrain_lod: true,
        kit_groups: &set.building_group,
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
    let arena = KitArena {
        vertex_base: scene.vertices.len() as u32,
        vertex_capacity: 12_000_000,
        index_base: scene.indices.len() as u32,
        index_capacity: 20_000_000,
        object_base: scene.objects.len() as u32,
        object_capacity: 16384,
    };
    let mut objects = scene.objects.clone();
    objects.extend(std::iter::repeat_n(GpuObject::zeroed_kit(), arena.object_capacity as usize));
    let groups: Vec<GpuLodGroup> = set.groups.clone();
    let renderer = Renderer::from_init(
        &ctx,
        &RendererInit {
            vertices: &scene.vertices,
            indices: &scene.indices,
            objects: &objects,
            extra_vertices: arena.vertex_capacity,
            extra_indices: arena.index_capacity,
            materials: &materials.gpu,
            groups: &groups,
            hlod_enter_m: 0.0,
            hlod_exit_m: 0.0,
            kit_layers: materials.layers.len() as u32,
            kit_res: kit::HIGH_RES as u32,
            color_format: wgpu::TextureFormat::Rgba8UnormSrgb,
            sample_count: 1,
            multiplier: 1,
            debug_ids: true,
            blend_first_object: scene.objects.len() as u32,
        },
    )
    .unwrap();
    let cache = eo_assets::default_cache_dir();
    let maps = kit::bake_all(&lib, &materials.layers, kit::HIGH_RES, Some(&cache)).unwrap();
    renderer.upload_kit(&ctx.queue, &KitUpload::build(&maps)).unwrap();
    let tenancy = eo_kit::tenancy::Tenancy::load(&paths.repo_root).unwrap();
    let kc = Arc::new(KitCity::build(&city, &tenancy, &eo_kit::city::load_exclusions()));
    let key = eo_render::kit::inputs_key(&paths.repo_root, &set.palette, false);
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(Ok((kc.clone(), key.clone()))).unwrap();
    let start = Vec3::new(STREET[0][0], 0.0, STREET[0][1]);
    let kit = KitRuntime::start(set.clone(), rx, Some(cache.clone()), arena, KitRadii::default(), 6, start, false, std::time::Instant::now());
    let cook = CookContext { city: kc, set: set.clone(), cache_dir: Some(cache), inputs_key: key };
    Fixture { ctx, renderer, set, kit, uploads: UploadQueue::new(BUDGET - eo_render::kit::KIT_FRAME_RESERVE_BYTES), cook, elev, max_frame_bytes: 0 }
}

fn street_cam(f: &Fixture, t: f32, yaw_off: f32) -> Camera {
    let a = Vec3::new(STREET[0][0], 0.0, STREET[0][1]);
    let b = Vec3::new(STREET[1][0], 0.0, STREET[1][1]);
    let p = a + (b - a) * t;
    let g = f.elev.ground(p.x as f64, p.z as f64) as f32;
    let d = (b - a).normalize();
    Camera::new(Vec3::new(p.x, g + 1.7, p.z), d.x.atan2(-d.z) + yaw_off, 0.04, 16.0 / 9.0)
}

/// One frame: stream (budgeted), then cull and draw.
fn frame(f: &mut Fixture, cam: &Camera) {
    f.kit.begin_frame();
    f.kit.update(&f.renderer, &f.ctx.queue, &mut f.uploads, cam.eye);
    let (st, done) = f.uploads.pump(&f.renderer, &f.ctx.queue);
    f.kit.after_pump(&f.renderer, &f.ctx.queue, &done);
    let bytes = st.bytes + f.kit.last_group_bytes;
    f.max_frame_bytes = f.max_frame_bytes.max(bytes);
    assert!(bytes <= BUDGET, "a frame wrote {bytes} B, over the {BUDGET} B budget");
    f.renderer.update(&f.ctx.queue, cam);
    f.ctx.queue.submit([]);
}

fn settle(f: &mut Fixture, cam: &Camera, mut check: impl FnMut(&mut Fixture)) -> usize {
    let mut frames = 0;
    for _ in 0..20_000 {
        frame(f, cam);
        frames += 1;
        if frames % 25 == 0 {
            check(f);
        }
        if f.kit.settled() && f.uploads.pending() == 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(f.kit.settled(), "streaming did not settle");
    frames
}

/// Per group: what the cull pass drew (box, near rung, mid rung).
fn exclusive(f: &Fixture, cam: &Camera) -> (usize, usize) {
    let out = f.renderer.cull_readback(&f.ctx, cam).unwrap();
    let objects = f.renderer.objects_readback(&f.ctx).unwrap();
    let mut by_group: HashMap<u32, [u32; 3]> = HashMap::new();
    for d in out.draws.iter().filter(|d| d[1] == 1) {
        let o = &objects[d[4] as usize];
        let k = match o.lod_role {
            LOD_KIT_BOX => 0,
            LOD_KIT_NEAR => 1,
            LOD_KIT_MID => 2,
            _ => continue,
        };
        by_group.entry(o.lod_group).or_default()[k] += 1;
    }
    let mut kit_groups = 0;
    let mut boxes = 0;
    for (g, c) in &by_group {
        let levels = c.iter().filter(|n| **n > 0).count();
        assert!(levels <= 1, "group {g}: drawn at {levels} levels at once {c:?}");
        assert!(c[0] <= 1 && c[1] <= 1 && c[2] <= 1, "group {g}: drawn twice at one level {c:?}");
        kit_groups += (c[1] + c[2] > 0) as usize;
        boxes += (c[0] > 0) as usize;
    }
    assert_eq!(out.counters[6] as usize, out.draws.iter().filter(|d| d[1] == 1 && objects[d[4] as usize].object_kind() == ObjectKind::Kit as u32).count());
    (kit_groups, boxes)
}

#[test]
fn box_and_kit_are_exclusive_while_streaming_and_uploads_keep_the_budget() {
    let mut f = fixture();
    let cam = street_cam(&f, 0.0, 0.0);
    // Checked every 25 frames while the packs arrive.
    let frames = settle(&mut f, &cam, |f| {
        let c = street_cam(f, 0.0, 0.0);
        exclusive(f, &c);
    });
    let mut kit_seen = 0;
    for (t, yaw) in [(0.0f32, 0.0f32), (0.0, 1.6), (0.0, 3.2), (0.3, -1.2), (0.5, 1.2)] {
        let c = street_cam(&f, t, yaw);
        for _ in 0..3 {
            frame(&mut f, &c);
        }
        let (kit, boxes) = exclusive(&f, &c);
        kit_seen += kit;
        eprintln!("t {t} yaw {yaw}: {kit} buildings drawn as kit, {boxes} as boxes");
    }
    assert!(kit_seen > 200, "kit geometry was hardly drawn ({kit_seen})");
    // A 2 km jump: the street's packs are evicted (at most a few a frame)
    // while the new ones stream in; exclusivity holds throughout.
    let fp = eo_data::groundscale::plan32([-3300.0, 1600.0]);
    let far = Camera::new(Vec3::new(fp[0], 80.0, fp[1]), 1.0, -0.1, 16.0 / 9.0);
    let evicted_before = f.kit.stats.packs_evicted;
    settle(&mut f, &far, |f| {
        exclusive(f, &far);
    });
    assert!(f.kit.stats.packs_evicted > evicted_before + 20, "the jump evicted only {} packs", f.kit.stats.packs_evicted - evicted_before);
    let (kit, _) = exclusive(&f, &far);
    eprintln!(
        "settled in {frames} frames; after the jump {kit} buildings as kit, {} packs evicted; largest frame {} B (budget {BUDGET} B)",
        f.kit.stats.packs_evicted, f.max_frame_bytes
    );
    assert!(f.max_frame_bytes <= BUDGET);
    assert!(f.max_frame_bytes > BUDGET / 2, "the budget was never approached ({} B): the test proves nothing", f.max_frame_bytes);
}

/// Render (object, material) ids; for every kit pixel, check the material
/// against the palette entries its object's geometry carries.
fn check_pixels(f: &Fixture, cam: &Camera, expected: &HashMap<u32, HashSet<u32>>, w: u32, h: u32) -> (usize, usize, HashMap<u32, usize>) {
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
    let buf = d.create_buffer(&wgpu::BufferDescriptor { label: None, size: (row * h) as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
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
    let (mut px, mut bad) = (0, 0);
    let mut per_object = HashMap::new();
    for y in 0..h as usize {
        for x in 0..w as usize {
            let o = y * row as usize + x * 8;
            let obj = u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
            let mat = u32::from_le_bytes(bytes[o + 4..o + 8].try_into().unwrap());
            let Some(set) = expected.get(&obj) else { continue };
            px += 1;
            *per_object.entry(obj).or_insert(0usize) += 1;
            if !set.contains(&mat) {
                bad += 1;
            }
        }
    }
    (px, bad, per_object)
}

#[test]
fn cooked_tiles_are_deterministic_and_a_damaged_cache_file_is_refused() {
    let f = fixture();
    let key = ((STREET[0][0] as f64 / eo_render::kit::KIT_TILE_M).floor() as i32, (STREET[0][1] as f64 / eo_render::kit::KIT_TILE_M).floor() as i32);
    let t = *f.set.tile_of_key.get(&key).unwrap();
    let (a, sa) = eo_render::kit::cook_tile(&f.cook, t);
    let (b, _) = eo_render::kit::cook_tile(&f.cook, t);
    assert!(sa.buildings > 20, "tile has {} kit buildings", sa.buildings);
    for (pa, pb) in a.iter().zip(&b) {
        let (ea, eb) = (pa.encode(), pb.encode());
        assert_eq!(ea, eb, "the same tile cooked twice gives different bytes");
        assert_eq!(&eo_render::kit::Pack::decode(&ea).unwrap(), pa, "cache round trip");
        let mut bad = ea.clone();
        let k = bad.len() / 2;
        bad[k] ^= 0x40;
        assert!(eo_render::kit::Pack::decode(&bad).is_err(), "a flipped byte was accepted");
        assert!(eo_render::kit::Pack::decode(&ea[..ea.len() - 7]).is_err(), "a truncated file was accepted");
    }
    eprintln!("tile {t}: {} buildings, near {} / mid {} triangles", sa.buildings, a[0].triangles(), a[1].triangles());
}

#[test]
fn each_kit_pixel_resolves_a_material_of_its_own_object_and_a_planted_change_is_caught() {
    let mut f = fixture();
    let cam = street_cam(&f, 0.15, 1.3);
    settle(&mut f, &cam, |_| {});
    for _ in 0..3 {
        frame(&mut f, &cam);
    }
    // The packs as uploaded, cooked again (or read from the cache) on the CPU:
    // object k of a pack's range is the pack's k-th building.
    let mut expected: HashMap<u32, HashSet<u32>> = HashMap::new();
    for ((tile, rung), (first, count)) in f.kit.resident() {
        let pack = load_pack(&f.cook, tile, rung).pack;
        assert_eq!(pack.buildings.len() as u32, count);
        for (k, b) in pack.buildings.iter().enumerate() {
            let set: HashSet<u32> = b.splits.iter().map(|s| f.set.palette_base + s.2 as u32).collect();
            expected.insert(first + k as u32, set);
        }
        let _ = rung == RUNG_NEAR;
    }
    let (w, h) = (960, 540);
    let (px, bad, per_object) = check_pixels(&f, &cam, &expected, w, h);
    let mats: HashSet<u32> = per_object.keys().flat_map(|o| expected[o].iter().copied()).collect();
    eprintln!("{px} kit pixels over {} objects carrying {} materials; {bad} pixels with a material their object does not carry", per_object.len(), mats.len());
    assert!(px > 100_000, "kit buildings do not fill the frame ({px} pixels)");
    assert!(mats.len() >= 6, "too few materials in view ({})", mats.len());
    assert_eq!(bad, 0);
    // Plant: the biggest object's palette base moves by one entry (GPU only).
    let (&obj, &obj_px) = per_object.iter().max_by_key(|e| e.1).unwrap();
    f.renderer.debug_set_object_material(&f.ctx.queue, obj, f.set.palette_base + 1);
    let (_, bad, _) = check_pixels(&f, &cam, &expected, w, h);
    eprintln!("planted change on object {obj} ({obj_px} pixels): {bad} mismatching pixels");
    assert!(bad * 2 >= obj_px, "the planted change was not caught ({bad} of {obj_px})");
}

/// groups within the near zone and inside the camera frustum, and
/// what the cull pass drew for them (box, near rung, mid rung).
fn near_on_screen(f: &Fixture, cam: &Camera) -> (usize, usize, usize) {
    let out = f.renderer.cull_readback(&f.ctx, cam).unwrap();
    let objects = f.renderer.objects_readback(&f.ctx).unwrap();
    let mut by_group: HashMap<u32, [u32; 3]> = HashMap::new();
    for d in out.draws.iter().filter(|d| d[1] == 1) {
        let o = &objects[d[4] as usize];
        let k = match o.lod_role {
            LOD_KIT_BOX => 0,
            LOD_KIT_NEAR => 1,
            LOD_KIT_MID => 2,
            _ => continue,
        };
        by_group.entry(o.lod_group).or_default()[k] += 1;
    }
    let planes = eo_render::camera::frustum_planes(cam.view_proj());
    let (mut near, mut mid, mut boxed) = (0, 0, 0);
    for (g, c) in &by_group {
        let grp = &f.set.groups[(*g - f.set.group_base) as usize];
        let ctr = Vec3::from(grp.center);
        let d = ((ctr - cam.eye).length() - grp.radius).max(0.0);
        if d > eo_render::kit::DEFAULT_KIT_RADII[0] || !eo_render::camera::aabb_visible(&planes, ctr - grp.radius, ctr + grp.radius) {
            continue;
        }
        near += (c[1] > 0) as usize;
        mid += (c[2] > 0) as usize;
        boxed += (c[0] > 0) as usize;
    }
    (near, mid, boxed)
}

#[test]
fn view_detail_loads_near_packs_only_in_view_and_the_mid_rung_stands_in_while_turning() {
    let mut f = fixture();
    // A short hold so that the eviction after the turn is seen in a test's time.
    f.renderer.set_view_detail(eo_render::view_detail::ViewDetailConfig { hold_s: 0.4, ..eo_render::view_detail::ViewDetailConfig::on() });
    let ahead = street_cam(&f, 0.3, 0.0);
    settle(&mut f, &ahead, |_| {});
    let cone = f.renderer.view_cone().expect("view detail is on");
    let resident = f.kit.resident();
    let near: Vec<u32> = resident.iter().filter(|r| r.0.1 == RUNG_NEAR).map(|r| r.0.0).collect();
    let mids = resident.iter().filter(|r| r.0.1 != RUNG_NEAR).count();
    for t in &near {
        let (mn, mx) = f.set.tile_box(*t as usize, f.kit.tile_heights());
        assert!(cone.sees_box(mn, mx), "near pack of tile {t} resident out of view");
    }
    assert!(!near.is_empty(), "no near pack in view");
    assert!(f.kit.near_out_of_view > 0, "no near-radius tile was out of view: the gate did nothing here");
    let (n0, m0, b0) = near_on_screen(&f, &ahead);
    eprintln!("ahead: {} near + {mids} mid packs resident, {} near-radius tiles out of view; on screen within 65 m: {n0} rung 0, {m0} rung 2, {b0} boxes", near.len(), f.kit.near_out_of_view);
    assert!(n0 > 0 && b0 == 0, "ahead: {n0} at rung 0, {b0} as boxes");
    // Turn 180 degrees over ten frames: nothing within 65 m on screen is ever a
    // box (the radial mid packs stand in), and no group draws at two levels.
    let mut stood_in = 0;
    for k in 1..=10 {
        let c = street_cam(&f, 0.3, std::f32::consts::PI * k as f32 / 10.0);
        frame(&mut f, &c);
        exclusive(&f, &c);
        let (_, m, b) = near_on_screen(&f, &c);
        assert_eq!(b, 0, "turn frame {k}: {b} buildings within 65 m drawn as boxes");
        stood_in += m;
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    eprintln!("while turning: {stood_in} building-frames drew rung 2 in the near zone (the stand-in)");
    // Settle behind: its near packs arrive, and once past the hold those of the
    // old view are evicted.
    let behind = street_cam(&f, 0.3, std::f32::consts::PI);
    let evicted = f.kit.stats.packs_evicted;
    settle(&mut f, &behind, |f| {
        let c = street_cam(f, 0.3, std::f32::consts::PI);
        exclusive(f, &c);
    });
    std::thread::sleep(std::time::Duration::from_millis(500));
    // Past the hold: a few frames evict (at most four packs a frame).
    for _ in 0..30 {
        frame(&mut f, &behind);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let (n1, _, b1) = near_on_screen(&f, &behind);
    let cone = f.renderer.view_cone().unwrap();
    for ((t, r), _) in f.kit.resident() {
        if r == RUNG_NEAR {
            let (mn, mx) = f.set.tile_box(t as usize, f.kit.tile_heights());
            assert!(cone.sees_box(mn, mx), "behind: near pack of tile {t} still resident out of view past the hold");
        }
    }
    eprintln!("behind: on screen within 65 m {n1} rung 0, {b1} boxes; {} near packs evicted after the turn", f.kit.stats.packs_evicted - evicted);
    assert!(n1 > 0 && b1 == 0);
    assert!(f.kit.stats.packs_evicted > evicted, "the old view's near packs were not evicted");
}
