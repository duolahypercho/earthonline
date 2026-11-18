//! the far layer.
//!
//!  - every building the city draws as a building (kit, Blender-built,
//!    code-built authored; not a landmark's claimed records) is in exactly one
//!    cell, its group names that cell, and a code-built building's box got a
//!    group of its own (real data, no GPU);
//!  - a real cell bakes deterministically, every vertex names one of the
//!    cell's buildings, every building has a summary, and the cache refuses a
//!    damaged or truncated file and bakes it again (real data, no GPU);
//!  - the LOD pass never leaves a building drawn at no level, and draws two
//!    only as a cross-fade pair inside its band (a near level with a far one,
//!    or a cell with its district), never a box beside anything; a building
//!    past the mid band is never a box while a far level or its mid rung is
//!    resident (synthetic table, GPU).

use eo_assets::kit::KitLibrary;
use eo_assets::placement::PlacementConstants;
use eo_kit::city::KitCity;
use eo_render::authored::{AuthoredSet, GpuLodGroup, GpuMaterial};
use eo_render::camera::Camera;
use eo_render::hlod_bake::{self, BakeContext, BakeOptions, BakedCell, HlodSet, MaterialColours, NO_CELL, Source};
use eo_render::kit::KitSet;
use eo_render::scene::{GpuObject, LOD_FAR, LOD_KIT_BOX, LOD_KIT_MID, LOD_KIT_NEAR, LOD_NEAR, ObjectKind, SceneInputs, Vertex, build_scene};
use eo_render::{GpuContext, GpuOptions, Renderer, RendererInit, wgpu};
use glam::Vec3;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

struct Real {
    paths: eo_data::DataPaths,
    city: Arc<eo_data::City>,
    kit: Arc<KitSet>,
    authored: Arc<AuthoredSet>,
    set: HlodSet,
    scene: eo_render::scene::Scene,
}

fn real() -> Real {
    let paths = eo_data::DataPaths::discover().unwrap();
    let city = Arc::new(eo_data::City::load(&paths).unwrap());
    let elev = eo_data::ElevationGrid::load(&paths).unwrap();
    let ca = eo_data::CaStandards::load(&paths).unwrap();
    let lib = KitLibrary::load().unwrap();
    let constants = PlacementConstants::load().unwrap();
    let mut authored = AuthoredSet::prepare(&paths.repo_root, &city, &elev, &lib, &constants).unwrap();
    let mut excluded: HashSet<String> = authored.group_of_record.keys().cloned().collect();
    for id in eo_kit::city::load_exclusions() {
        excluded.insert(id.trim_start_matches("sf-building-").to_string());
    }
    let group_base = authored.groups.len() as u32;
    let kit = Arc::new(KitSet::prepare(&city, &elev, &excluded, group_base, &mut authored.materials, &lib));
    let mut scene = build_scene(&SceneInputs {
        city: &city,
        elevation: &elev,
        land: None,
        standards: &ca,
        authored: &authored.group_of_record,
        terrain_lod: true,
        kit_groups: &kit.building_group,
        carpet: None,
        street: None,
        skirts: &[],
        box_floor: None,
        skip_building: None,
        ground_tone: None,
        smooth_terrain: false,
    })
    .unwrap();
    let before = authored.groups.len() + kit.groups.len();
    let set = HlodSet::prepare(&city, &kit, &authored, &HashSet::new(), &mut scene, before as u32);
    Real { paths, city, kit, authored: Arc::new(authored), set, scene }
}

#[test]
fn every_building_is_in_exactly_one_cell() {
    let r = real();
    let set = &r.set;
    let mut seen: HashMap<u32, u32> = HashMap::new();
    for (ci, c) in set.cells.iter().enumerate() {
        for cb in &c.buildings {
            assert!(seen.insert(cb.building, ci as u32).is_none(), "building {} is in two cells", cb.building);
            assert_eq!(set.group_cell[cb.group as usize], ci as u32, "building {}'s group names another cell", cb.building);
            // Its centroid lies in its cell's square.
            let ring = &r.city.buildings[cb.building as usize].ring;
            let n = ring.len() as f64;
            let (sx, sz) = ring.iter().fold((0.0f64, 0.0f64), |a, p| (a.0 + p[0] as f64, a.1 + p[1] as f64));
            assert_eq!(hlod_bake::cell_key(sx / n, sz / n), c.key);
        }
    }
    let mut missing = vec![];
    for (i, b) in r.city.buildings.iter().enumerate() {
        if b.ring.len() >= 3 && !seen.contains_key(&(i as u32)) {
            missing.push(b.id.clone());
        }
    }
    assert!(missing.is_empty(), "{} buildings in no cell: {:?}", missing.len(), &missing[..missing.len().min(10)]);
    assert_eq!(set.covered, seen.len());
    assert_eq!(set.unmatched_plain, 0, "every code-built building's box was found");
    assert_eq!(set.by_source[0], r.kit.buildings, "every kit building is covered");
    assert_eq!(set.by_source[1], r.authored.buildings.len(), "every Blender-built building is covered");
    // A group is in at most one cell; the code-built boxes now have groups of their own.
    let groups: Vec<u32> = set.cells.iter().flat_map(|c| c.buildings.iter().map(|b| b.group)).collect();
    assert_eq!(groups.len(), groups.iter().copied().collect::<HashSet<_>>().len(), "a group in two cells");
    let plain: Vec<_> = set.cells.iter().flat_map(|c| c.buildings.iter().filter(|b| b.source == Source::Plain)).collect();
    assert_eq!(plain.len(), set.extra_groups.len());
    for b in &plain {
        assert!(b.group >= set.extra_group_base);
        let adopted = r.scene.objects.iter().filter(|o| o.lod_group == b.group && o.lod_role == LOD_FAR).count();
        assert_eq!(adopted, 1, "the code-built building's box switches with its group");
    }
    assert!(set.group_cell.iter().filter(|&&c| c != NO_CELL).count() == set.covered);
    println!("{} cells, {} buildings ({:?} kit/Blender/code-built), {} groups", set.cells.len(), set.covered, set.by_source, set.group_cell.len());
}

#[test]
fn a_real_cell_bakes_deterministically_and_the_cache_refuses_damage() {
    let r = real();
    let tenancy = eo_kit::tenancy::Tenancy::load(&r.paths.repo_root).unwrap();
    let kc = Arc::new(KitCity::build(&r.city, &tenancy, &eo_kit::city::load_exclusions()));
    let colours = MaterialColours::new(&r.authored.materials.gpu, &vec![[0.5; 3]; r.authored.materials.layers.len()]);
    let dir = std::env::temp_dir().join(format!("eo-hlod-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let opts = BakeOptions::default();
    let key = hlod_bake::inputs_key("test", &colours, &r.authored, &opts);
    let set = Arc::new(r.set);
    // The cell holding the bench's kit street (Castro Street).
    let ks = eo_data::groundscale::plan([-1348.0, 2246.0]); // on the shipped plan
    let cell = set.cell_of_key[&hlod_bake::cell_key(ks[0], ks[1])];
    let ctx = BakeContext {
        set: set.clone(),
        kit: r.kit.clone(),
        kit_city: kc,
        authored: r.authored.clone(),
        city: r.city.clone(),
        colours: Arc::new(colours),
        repo_root: r.paths.repo_root.clone(),
        cache_dir: Some(dir.clone()),
        key: key.clone(),
        opts,
    };
    let a = hlod_bake::bake_cell(&ctx, cell);
    let b = hlod_bake::bake_cell(&ctx, cell);
    let strip = |c: &BakedCell| BakedCell { stats: hlod_bake::CellStats { bake_ms: 0.0, ..c.stats.clone() }, ..c.clone() };
    assert_eq!(strip(&a).encode(), strip(&b).encode(), "the bake is deterministic");
    let members: HashSet<u32> = set.cells[cell as usize].buildings.iter().map(|b| b.group).collect();
    assert_eq!(a.summaries.len(), members.len(), "every building has a summary");
    assert!(a.vertices.iter().all(|v| members.contains(&v.group)), "every vertex names one of the cell's buildings");
    assert!(a.indices.iter().all(|&i| (i as usize) < a.vertices.len()));
    assert!(!a.pages.is_empty() && a.pages.len() <= opts.max_pages, "{} pages", a.pages.len());
    assert!(a.stats.kit > 0 && a.stats.fallback * 10 <= a.stats.kit, "{:?}", a.stats);
    assert!(a.summaries.iter().any(|s| s.glass > 0.01), "the facades see windows");
    println!("cell {:?}: {} buildings, {} triangles, {} pages at {:.2} m texels, {} charts, {:.0} ms", a.key, a.summaries.len(), a.triangles(), a.pages.len(), a.texel_m, a.stats.charts, a.stats.bake_ms);
    // Cache: written by the first load, read back by the second.
    let (c1, s1) = hlod_bake::load_or_bake(&ctx, cell);
    assert_eq!(s1, hlod_bake::CellSource::Baked);
    let (c2, s2) = hlod_bake::load_or_bake(&ctx, cell);
    assert_eq!(s2, hlod_bake::CellSource::Cache);
    assert_eq!(c1, c2);
    assert_eq!(hlod_bake::read_summaries(&ctx, cell).unwrap(), c1.summaries);
    // Damage and truncation: refused, baked again, the file replaced.
    let path = hlod_bake::cache_path(&dir, &key, set.cells[cell as usize].key);
    let mut bytes = std::fs::read(&path).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0x10;
    std::fs::write(&path, &bytes).unwrap();
    assert!(BakedCell::decode(&bytes).is_err());
    let (_, s3) = hlod_bake::load_or_bake(&ctx, cell);
    assert_eq!(s3, hlod_bake::CellSource::Baked, "a damaged file is baked again");
    let good = std::fs::read(&path).unwrap();
    assert!(BakedCell::decode(&good).is_ok(), "and replaced");
    std::fs::write(&path, &good[..good.len() - 100]).unwrap();
    let (_, s4) = hlod_bake::load_or_bake(&ctx, cell);
    assert_eq!(s4, hlod_bake::CellSource::Baked, "a truncated file is baked again");
    // Another key is another folder.
    assert_ne!(key, hlod_bake::inputs_key("test2", &ctx.colours, &ctx.authored, &opts));
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// GPU: the level choice.
// ---------------------------------------------------------------------------

fn cube(c: Vec3, h: f32) -> (Vec<Vertex>, Vec<u32>) {
    let v: Vec<Vertex> = (0..8)
        .map(|k| Vertex { pos: [c.x + if k & 1 == 0 { -h } else { h }, c.y + if k & 2 == 0 { -h } else { h }, c.z + if k & 4 == 0 { -h } else { h }], normal: 0, color: u32::MAX })
        .collect();
    let i = vec![0, 1, 3, 0, 3, 2, 4, 6, 7, 4, 7, 5, 0, 4, 5, 0, 5, 1, 2, 3, 7, 2, 7, 6, 0, 2, 6, 0, 6, 4, 1, 5, 7, 1, 7, 3];
    (v, i)
}

#[test]
fn the_lod_pass_draws_every_building_at_one_level_or_a_crossfade_pair() {
    let opts = GpuOptions::default();
    let ctx = GpuContext::new(GpuContext::create_instance(&opts), None, opts).expect("a GPU adapter is required");
    let centre = Vec3::new(0.0, 10.0, 0.0);
    let (cv, ci) = cube(centre, 5.0);
    let mut vertices = vec![];
    let mut indices = vec![];
    let mut objects = vec![];
    // (group, kind, role) per object; groups: 0 Blender (model resident), 1 kit (both rungs),
    // 2 kit (mid rung only), 3 kit (no rung), 4 code-built (never resident).
    let table: [(u32, ObjectKind, u32); 11] = [
        (0, ObjectKind::Model, LOD_NEAR),
        (0, ObjectKind::Building, LOD_FAR),
        (1, ObjectKind::Kit, LOD_KIT_NEAR),
        (1, ObjectKind::Kit, LOD_KIT_MID),
        (1, ObjectKind::Building, LOD_KIT_BOX),
        (2, ObjectKind::Kit, LOD_KIT_NEAR),
        (2, ObjectKind::Kit, LOD_KIT_MID),
        (2, ObjectKind::Building, LOD_KIT_BOX),
        (3, ObjectKind::Kit, LOD_KIT_MID),
        (3, ObjectKind::Building, LOD_KIT_BOX),
        (4, ObjectKind::Building, LOD_FAR),
    ];
    for &(g, kind, role) in &table {
        let bv = vertices.len() as i32;
        let fi = indices.len() as u32;
        vertices.extend_from_slice(&cv);
        indices.extend_from_slice(&ci);
        let mut o = GpuObject::new(kind, [-5.0, 5.0, -5.0], [5.0, 15.0, 5.0], fi, ci.len() as u32, bv);
        o.lod_group = g;
        o.lod_role = role;
        objects.push(o);
    }
    let radius = 0.5 * (Vec3::splat(10.0)).length();
    let grp = |ready: u32, kind: u32| GpuLodGroup { center: centre.to_array(), radius, ready, detailed: if kind == 1 { 2 } else { 0 }, kind, level: 2 };
    let groups = vec![grp(1, 0), grp(3, 1), grp(2, 1), grp(0, 1), grp(0, 0)];
    let materials = [GpuMaterial::vertex_colour()];
    let r = Renderer::from_init(
        &ctx,
        &RendererInit {
            vertices: &vertices,
            indices: &indices,
            objects: &objects,
            extra_vertices: 0,
            extra_indices: 0,
            materials: &materials,
            groups: &groups,
            hlod_enter_m: 320.0,
            hlod_exit_m: 380.0,
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
    r.write_hlod_map(&ctx.queue, &[0, 0, 0, 0, 0]);
    let kit = eo_render::kit::DEFAULT_KIT_RADII;
    let far = eo_render::hlod::HlodOptions::default().far_band;
    // Camera distances to the groups' bounding sphere, out and back (the zone hysteresis both ways).
    let mut ds: Vec<f32> = (0..=440).map(|k| k as f32 * 5.0).collect();
    let back: Vec<f32> = ds.iter().rev().copied().collect();
    ds.extend(back);
    let mut fades = [0usize; 2];
    for flags in [0u32, 1, 2, 3] {
        r.write_hlod_flags(&ctx.queue, &[flags]);
        for &d in &ds {
            // Looking at the groups along -x from d metres off their sphere.
            let eye = centre + Vec3::new(d + radius, 0.0, 0.0);
            let cam = Camera { far: 4200.0, ..Camera::new(eye, -std::f32::consts::FRAC_PI_2, 0.0, 16.0 / 9.0) };
            let out = r.cull_readback(&ctx, &cam).unwrap();
            let state = r.hlod_state_readback(&ctx).unwrap();
            let drawn: HashSet<u32> = out.draws.iter().filter(|c| c[1] == 1).map(|c| c[4]).collect();
            for g in 0..5u32 {
                let objs: Vec<usize> = (0..table.len()).filter(|&k| table[k].0 == g && drawn.contains(&(k as u32))).collect();
                let near = objs.iter().filter(|&&k| matches!(table[k].2, LOD_NEAR | LOD_KIT_NEAR | LOD_KIT_MID)).count();
                let boxes = objs.iter().filter(|&&k| matches!(table[k].2, LOD_FAR | LOD_KIT_BOX)).count();
                let st = state[g as usize];
                let cell = (st & 1 != 0) as usize;
                let district = (st & 2 != 0) as usize;
                let what = format!("flags {flags} d {d} group {g}: near {near} box {boxes} state {st:#x}");
                assert!(near <= 1 && boxes <= 1, "two near levels or two boxes: {what}");
                assert!(cell == 0 || flags & 1 != 0, "a cell drawn that is not resident: {what}");
                assert!(district == 0 || flags & 2 != 0, "a district drawn that is not resident: {what}");
                let n = near + boxes + cell + district;
                assert!(n >= 1, "drawn at no level: {what}");
                if n == 2 {
                    assert_eq!(boxes, 0, "a box beside another level: {what}");
                    let t = (st >> 8) & 255;
                    if near == 1 {
                        assert!(st & 4 != 0, "the near level and a far one without a cross-fade: {what}");
                        let (a, b) = if g == 0 || g == 4 { (320.0, 380.0) } else { (kit[2], kit[3]) };
                        assert!(d > a - 1.0 && d < b + 1.0, "a near/far cross-fade outside its band: {what}");
                        fades[0] += 1;
                    } else {
                        assert!(cell == 1 && district == 1 && st & 4 == 0, "{what}");
                        assert!(d > far[0] - 1.0 && d < far[1] + 1.0, "a cell/district cross-fade outside its band: {what}");
                        fades[1] += 1;
                    }
                    assert!(t <= 255, "{what}");
                } else {
                    assert_eq!(n, 1, "{what}");
                }
                // Past the mid band a kit building is never a box while a far level or its mid rung is resident.
                // (Groups 1 and 2 have their mid rung resident, group 3 none.)
                if (1..=3).contains(&g) && d > kit[3] + 1.0 && (flags != 0 || g != 3) {
                    assert_eq!(boxes, 0, "a box past the mid band with a far level or its mid rung resident: {what}");
                }
                if flags == 0 && g == 3 {
                    assert_eq!(boxes, 1, "nothing else resident: the box ({what})");
                }
            }
        }
    }
    assert!(fades[0] > 0 && fades[1] > 0, "both kinds of cross-fade were seen: {fades:?}");
}
