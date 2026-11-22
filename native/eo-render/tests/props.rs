//! the instanced street-prop class on the GPU. A planted scene: a
//! slab, a wall, prop instances in front of the camera, behind it, behind the
//! wall, and far away. Checks (frame three, occlusion on and off): the camera
//! lists hold exactly the instances in the frustum, unoccluded and within
//! their model's draw distance; the Hi-Z test removes the one behind the wall
//! (and rising over the wall brings it back the same frame); a tree casts a
//! sun shadow through the props caster path (darker than with props off);
//! a lit vehicle-head lens follows the lamp channel. Needs a GPU and the JS
//! street-prop GLBs / vegetation atlas (or proxies when missing).

use eo_props::Model;
use eo_render::camera::Camera;
use eo_render::props::GpuPropInstance;
use eo_render::props_lib::PropLibrary;
use eo_render::scene::{GpuObject, ObjectKind, Vertex};
use eo_render::upload::Target;
use eo_render::{GpuContext, GpuOptions, Renderer, Targets, read_mapped, wgpu};
use glam::Vec3;

const W: u32 = 320;
const H: u32 = 180;

fn ctx() -> GpuContext {
    let opts = GpuOptions::default();
    GpuContext::new(GpuContext::create_instance(&opts), None, opts).expect("a GPU adapter is required")
}

fn boxed(verts: &mut Vec<Vertex>, idx: &mut Vec<u32>, objects: &mut Vec<GpuObject>, min: Vec3, max: Vec3) {
    let base = verts.len() as i32;
    let first = idx.len() as u32;
    for i in 0..8 {
        let p = Vec3::new(if i & 1 != 0 { max.x } else { min.x }, if i & 2 != 0 { max.y } else { min.y }, if i & 4 != 0 { max.z } else { min.z });
        verts.push(Vertex { pos: p.to_array(), normal: 0, color: 0xff80_8080 });
    }
    for f in [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]] {
        idx.extend_from_slice(&[f[0], f[1], f[2], f[0], f[2], f[3]]);
    }
    objects.push(GpuObject::new(ObjectKind::Terrain, min.to_array(), max.to_array(), first, 36, base));
}

fn setup(ctx: &GpuContext) -> Renderer {
    let (mut v, mut i, mut o) = (vec![], vec![], vec![]);
    boxed(&mut v, &mut i, &mut o, Vec3::new(-300.0, -1.0, -300.0), Vec3::new(300.0, 0.0, 300.0));
    // A wall 30 m north, 8 m high, 40 m wide.
    boxed(&mut v, &mut i, &mut o, Vec3::new(-20.0, 0.0, -31.0), Vec3::new(20.0, 8.0, -30.0));
    let r = Renderer::from_parts(ctx, &v, &i, &o, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
    let paths = eo_data::DataPaths::discover().expect("repo data");
    let js = eo_props::JsProps::load().unwrap();
    let ca = eo_data::CaStandards::load(&paths).unwrap();
    let lens = ca.metric_key("signal-lens-size", "circularNewFaces").unwrap();
    let lib = PropLibrary::cook(&paths.repo_root, &js, lens).unwrap();
    r.attach_props(ctx, &lib).unwrap();
    r
}

fn inst(model: Model, p: Vec3, scale: [f32; 3]) -> GpuPropInstance {
    GpuPropInstance { pos: p.to_array(), yaw: 0.0, scale, model: model as u32, variant: 0, lamp: 0, slope: 0.0, pad: 0 }
}

fn frame(ctx: &GpuContext, r: &Renderer, cam: &Camera, frames: usize) -> Vec<f32> {
    let d = &ctx.device;
    let out = d.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let ov = out.create_view(&Default::default());
    let targets = Targets::new(d, W, H, wgpu::TextureFormat::Rgba8UnormSrgb, 1);
    for _ in 0..frames {
        r.update(&ctx.queue, cam);
        let mut enc = d.create_command_encoder(&Default::default());
        r.encode(&mut enc, &ov, None, &targets.depth, None);
        ctx.queue.submit([enc.finish()]);
    }
    let hdr = r.look.hdr_target(W, H);
    let row = (W * 8).div_ceil(256) * 256;
    let b = d.create_buffer(&wgpu::BufferDescriptor { label: None, size: (row * H) as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let mut enc = d.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        hdr.texture().as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &b, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(H) } },
        wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
    );
    ctx.queue.submit([enc.finish()]);
    let raw = read_mapped(ctx, &b).unwrap();
    let f16 = |h: u16| -> f32 {
        let s = if h >> 15 != 0 { -1.0 } else { 1.0 };
        let e = ((h >> 10) & 31) as i32;
        let m = (h & 1023) as f32;
        s * if e == 0 { m * 2f32.powi(-24) } else { (1.0 + m / 1024.0) * 2f32.powi(e - 15) }
    };
    let mut px = vec![];
    for y in 0..H as usize {
        for x in 0..W as usize {
            let o = y * row as usize + x * 8;
            let c = |k: usize| f16(u16::from_le_bytes([raw[o + 2 * k], raw[o + 2 * k + 1]]));
            px.extend_from_slice(&[c(0), c(1), c(2)]);
        }
    }
    px
}

fn project(cam: &Camera, p: Vec3) -> (usize, usize) {
    let c = cam.view_proj() * p.extend(1.0);
    (((c.x / c.w * 0.5 + 0.5) * W as f32) as usize, ((0.5 - c.y / c.w * 0.5) * H as f32) as usize)
}

#[test]
fn props_cull_lod_occlusion_shadow_and_lamps() {
    let ctx = ctx();
    let r = setup(&ctx);
    let p = r.props.get().unwrap();
    let cam = Camera::new(Vec3::new(0.0, 1.7, 0.0), 0.0, 0.0, W as f32 / H as f32);
    let list = vec![
        inst(Model::SignalPole, Vec3::new(2.0, 0.0, -10.0), [1.0, 6.9, 1.0]), // 0 visible
        inst(Model::Tree, Vec3::new(-3.0, 0.0, -15.0), [5.0, 9.0, 5.0]),      // 1 visible
        inst(Model::SignalHead, Vec3::new(0.0, 2.0, -8.0), [1.0; 3]),        // 2 visible (lamp 0)
        inst(Model::SignPost, Vec3::new(0.0, 0.0, 12.0), [1.0, 3.0, 1.0]),   // 3 behind the camera
        inst(Model::SignPost, Vec3::new(0.0, 0.0, -45.0), [1.0, 3.0, 1.0]),  // 4 behind the wall
        inst(Model::SignPost, Vec3::new(0.0, 0.0, -900.0), [1.0, 3.0, 1.0]), // 5 past its draw distance
        inst(Model::TactilePad, Vec3::new(1.0, 0.01, -6.0), [1.6, 1.0, 0.914]), // 6 visible
    ];
    r.write(&ctx.queue, Target::PropInstances, 0, bytemuck::cast_slice(&list));
    p.set_high_water(list.len() as u32);
    p.write_lamps(&ctx.queue, &[2]); // the head shows green
    let _ = frame(&ctx, &r, &cam, 3);
    let s = p.stats_readback(&ctx).unwrap();
    assert_eq!(s[0], 4, "camera view: the four visible instances drawn (got {s:?})");
    // The sign post behind the wall is culled by the Hi-Z test only: with occlusion off it is drawn.
    r.set_occlusion(false);
    let _ = frame(&ctx, &r, &cam, 3);
    let s = p.stats_readback(&ctx).unwrap();
    assert_eq!(s[0], 5, "occlusion off: the post behind the wall is drawn too (got {s:?})");
    // Rising over the wall: the post it hid is drawn in that same frame (late
    // pass), i.e. the first occlusion-on frame up there draws what occlusion off draws.
    let high = Camera::new(Vec3::new(0.0, 20.0, 0.0), 0.0, -0.36, W as f32 / H as f32);
    let _ = frame(&ctx, &r, &high, 3);
    let off = p.stats_readback(&ctx).unwrap()[0];
    r.set_occlusion(true);
    let _ = frame(&ctx, &r, &cam, 3);
    let _ = frame(&ctx, &r, &high, 1);
    let s = p.stats_readback(&ctx).unwrap();
    assert!(off >= 1 && s[0] == off, "over the wall the hidden post is drawn in the first frame ({} with occlusion, {off} without; {s:?})", s[0]);
    // Casters: some cascade has the tree.
    assert!(s[2] + s[4] + s[6] > 0, "props cast into the sun cascades (got {s:?})");
    // The lamp channel drives the lens: green lit vs red lit changes the head's pixels.
    let green = frame(&ctx, &r, &cam, 3);
    p.write_lamps(&ctx.queue, &[0]);
    let red = frame(&ctx, &r, &cam, 3);
    let (hx, hy) = project(&cam, Vec3::new(0.0, 2.0, -8.0));
    let mut changed = 0;
    for y in hy.saturating_sub(12)..(hy + 12).min(H as usize) {
        for x in hx.saturating_sub(8)..(hx + 8).min(W as usize) {
            let o = (y * W as usize + x) * 3;
            if (green[o] - red[o]).abs() + (green[o + 1] - red[o + 1]).abs() > 0.05 {
                changed += 1;
            }
        }
    }
    assert!(changed > 4, "the head's lens follows its lamp channel ({changed} pixels changed)");
    // A tree's sun shadow: the ground around it is darker with props than without.
    let with = frame(&ctx, &r, &high, 3);
    p.set_enabled(false);
    let without = frame(&ctx, &r, &high, 3);
    p.set_enabled(true);
    let lum = |v: &[f32], i: usize| 0.2126 * v[i * 3] + 0.7152 * v[i * 3 + 1] + 0.0722 * v[i * 3 + 2];
    let darker = (0..(W * H) as usize).filter(|&i| lum(&without, i) - lum(&with, i) > 0.02).count();
    assert!(darker > 50, "props darken the frame (drawn and shadowing): {darker} pixels");
}

// ---- props2: the furniture GLB kinds, the ez-tree trees, sign legends, the overhead sign
// assemblies, the dithered level cross-fade, the wires (cull, Hi-Z, one draw), the lamp-light
// side table and the detectable-warning domes' standards.

use eo_render::props::{GpuLampLight, GpuWireSeg};
use eo_render::props_lib2::{self as lib2, LibExtras};

#[test]
fn props2_dome_standards_equal_the_js() {
    // `DETECTABLE_WARNING_PAD` as the JS computes it (street-surface-detail.js over
    // california-street-standards.js `caMetric`; evaluated headless with node:
    // depthM 0.914, domeBaseDiaM 0.023, domeHeightM 0.0055, domePitchMaxM 0.044).
    let paths = eo_data::DataPaths::discover().expect("repo data");
    let ca = eo_data::CaStandards::load(&paths).unwrap();
    let d = lib2::dome_standards(&ca).unwrap();
    for (got, js) in d.iter().zip([0.023f32, 0.0055, 0.044, 0.914]) {
        assert!((got - js).abs() < 1e-6, "dome standards {d:?} differ from the JS");
    }
}

fn setup2(ctx: &GpuContext) -> (Renderer, eo_render::props_lib::PropLibrary) {
    setup2_with(ctx, false)
}

/// `setup2`, with the procedural stand-ins for absent GLBs (`LibExtras::stand_ins`) or without.
fn setup2_with(ctx: &GpuContext, stand_ins: bool) -> (Renderer, eo_render::props_lib::PropLibrary) {
    let (mut v, mut i, mut o) = (vec![], vec![], vec![]);
    boxed(&mut v, &mut i, &mut o, Vec3::new(-300.0, -1.0, -300.0), Vec3::new(300.0, 0.0, 300.0));
    boxed(&mut v, &mut i, &mut o, Vec3::new(-20.0, 0.0, -31.0), Vec3::new(20.0, 8.0, -30.0));
    let r = Renderer::from_parts(ctx, &v, &i, &o, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
    let paths = eo_data::DataPaths::discover().expect("repo data");
    let js = eo_props::JsProps::load().unwrap();
    let ca = eo_data::CaStandards::load(&paths).unwrap();
    let lens = ca.metric_key("signal-lens-size", "circularNewFaces").unwrap();
    let cache = eo_assets::default_cache_dir();
    let ez = eo_props::eztree::cook_all(Some(&cache)).unwrap();
    let fonts = eo_props::sign_atlas::Fonts::system().unwrap();
    let (signs, _) = eo_props::sign_atlas::load_or_cook(&cache, &js, &fonts, &[], eo_props::sign_atlas::SIGN_FALLBACK_STREETS).unwrap();
    let oh = eo_props::overhead::OverheadJs::load().unwrap();
    let x = LibExtras { ez: Some(&ez), signs: Some(&signs), overhead: Some(&oh), stand_ins, boats: None };
    let lib = eo_render::props_lib::PropLibrary::cook_full(&paths.repo_root, &js, lens, &x, lib2::dome_standards(&ca).unwrap()).unwrap();
    r.attach_props(ctx, &lib).unwrap();
    (r, lib)
}

fn inst_id(model: u32, p: Vec3, scale: [f32; 3], variant: u32) -> GpuPropInstance {
    GpuPropInstance { pos: p.to_array(), yaw: 0.0, scale, model, variant, lamp: 0, slope: 0.0, pad: 0 }
}

#[test]
fn props2_kinds_fade_wires_lights() {
    let ctx = ctx();
    let (r, lib) = setup2(&ctx);
    kinds_fade_wires_lights(&ctx, r, lib);
}

/// The same checks with the procedural stand-ins on (`--stand-ins`): a checkout
/// without the hydrant GLB then has a hydrant with the GLB's level edges, so its
/// level-0 fade band exists (without them its proxy has none).
#[test]
fn props2_kinds_fade_wires_lights_with_stand_ins() {
    let ctx = ctx();
    let (r, lib) = setup2_with(&ctx, true);
    kinds_fade_wires_lights(&ctx, r, lib);
}

/// The stand-ins switch: off, the library is the grey proxies' (no stand-in material
/// is pushed, an absent hydrant's levels are one proxy box); on, every absent kind with
/// a stand-in gets three levels. The cache keys differ.
#[test]
fn props2_stand_ins_only_with_their_switch() {
    let paths = eo_data::DataPaths::discover().expect("repo data");
    let js = eo_props::JsProps::load().unwrap();
    let ca = eo_data::CaStandards::load(&paths).unwrap();
    let lens = ca.metric_key("signal-lens-size", "circularNewFaces").unwrap();
    let dome = lib2::dome_standards(&ca).unwrap();
    let oh = eo_props::overhead::OverheadJs::load().unwrap();
    let off = LibExtras { overhead: Some(&oh), ..Default::default() };
    let on = LibExtras { stand_ins: true, ..off };
    assert!(!LibExtras::default().stand_ins, "off by default");
    let lib_off = eo_render::props_lib::PropLibrary::cook_full(&paths.repo_root, &js, lens, &off, dome).unwrap();
    let lib_on = eo_render::props_lib::PropLibrary::cook_full(&paths.repo_root, &js, lens, &on, dome).unwrap();
    assert_ne!(eo_render::props_lib::PropLibrary::key_with(&js, &off, dome), eo_render::props_lib::PropLibrary::key_with(&js, &on, dome), "on and off cook different libraries");
    let hydrant_absent = lib_off.missing.iter().any(|m| m.contains("hydrant"));
    if !hydrant_absent {
        // Every GLB of this checkout present: the switch changes nothing.
        assert_eq!(lib_off.materials.len(), lib_on.materials.len());
        return;
    }
    assert!(lib_on.materials.len() > lib_off.materials.len(), "the stand-ins push their materials only when on");
    let h = Model::Hydrant as usize;
    let (m_off, m_on) = (lib_off.models[h], lib_on.models[h]);
    assert!(m_on.meshes[0] != m_on.meshes[1] && m_on.meshes[1] != m_on.meshes[2], "on: the stand-in's three levels");
    let hydrant_note = lib_off.missing.iter().find(|m| m.contains("hydrant")).unwrap();
    if eo_render::props_lib2::furniture_models::enabled() {
        // off, with the furniture models (on by default): the hydrant is drawn in code, not as a box.
        assert!(hydrant_note.contains("drawn in code instead"), "off: the furniture model stands in ({hydrant_note})");
        assert!(m_off.meshes[0] != eo_render::props_lib::NONE, "off: the furniture model is built");
    } else {
        assert!(m_off.meshes[0] == m_off.meshes[1] && m_off.meshes[1] == m_off.meshes[2], "off: one proxy box at every level");
        assert!(m_on.lod[0] < m_off.lod[0], "on: the GLB's own level edges (off: the proxy's near ring)");
    }
}

fn kinds_fade_wires_lights(ctx: &GpuContext, r: Renderer, lib: eo_render::props_lib::PropLibrary) {
    let ctx = ctx;
    // A prop file left out of this checkout draws a proxy; one that is there must verify.
    let refused: Vec<&String> = lib.missing.iter().filter(|m| m.contains("refused")).collect();
    assert!(refused.is_empty(), "every props2 asset present verifies: {refused:?}");
    if !lib.missing.is_empty() {
        eprintln!("props2: not in this checkout, proxies drawn: {:?}", lib.missing);
    }
    assert!(!lib.tree_species.is_empty(), "the ez-tree species are in the library");
    for m in [Model::Hydrant as u32, Model::Planter as u32, Model::SignalCabinet as u32, lib2::TREE_BASE, lib2::OH_DIST_POLE, lib2::OH_SIGN_BASE] {
        assert!(lib.models[m as usize].lod[3] > 0.0 && lib.models[m as usize].meshes[0] != eo_render::props_lib::NONE, "model {m} is built");
    }
    let p = r.props.get().unwrap();
    let cam = Camera::new(Vec3::new(0.0, 1.7, 0.0), 0.0, 0.0, W as f32 / H as f32);
    // A hydrant whose sphere distance is inside its level-0 cross-fade band (edge lod.x, band 12 %).
    let hm = lib.models[Model::Hydrant as usize];
    let want = hm.lod[0] * (1.0 - 0.06);
    let z = -((want + hm.radius).powi(2) - (1.7 - hm.center[1]).powi(2)).max(1.0).sqrt();
    let stop = eo_props::JsProps::load().unwrap().signs.faces.iter().position(|f| f == "stop").unwrap_or(0) as u32;
    let list = vec![
        inst_id(lib2::TREE_BASE, Vec3::new(-7.0, 0.0, -15.0), [1.0; 3], 0),                 // 0 visible (ez-tree)
        inst_id(Model::Hydrant as u32, Vec3::new(1.5, 0.0, -6.0), [1.0; 3], 0),           // 1 visible (GLB, level 0)
        inst_id(Model::SignPlate as u32, Vec3::new(-1.0, 1.6, -5.0), [0.75, 0.75, 1.0], stop), // 2 visible (legend)
        inst_id(Model::Hydrant as u32, Vec3::new(0.0, 0.0, z), [1.0; 3], 0),               // 3 in the fade band: views 4 and 5
        inst_id(Model::Hydrant as u32, Vec3::new(0.0, 0.0, -140.0), [1.0; 3], 0),          // 4 past the JS near ring: not drawn
        inst_id(lib2::OH_SIGN_BASE, Vec3::new(4.0, 0.0, -12.0), [1.0; 3], 0),              // 5 visible (sign assembly)
    ];
    r.write(&ctx.queue, Target::PropInstances, 0, bytemuck::cast_slice(&list));
    p.set_high_water(list.len() as u32);
    // Wires: one in view, one behind the wall (Hi-Z), one far away (past the fade).
    let wires = [
        GpuWireSeg { a: [-10.0, 6.0, -20.0], radius: 0.014, b: [10.0, 6.0, -20.0], kind: 0 },
        GpuWireSeg { a: [-5.0, 2.0, -45.0], radius: 0.014, b: [5.0, 2.0, -45.0], kind: 0 },
        GpuWireSeg { a: [-5.0, 6.0, -600.0], radius: 0.014, b: [5.0, 6.0, -600.0], kind: 0 },
    ];
    r.write(&ctx.queue, Target::PropWires, 0, bytemuck::cast_slice(&wires));
    p.set_wire_high_water(wires.len() as u32);
    let without_wires = {
        p.set_wire_high_water(0);
        let f = frame(&ctx, &r, &cam, 3);
        p.set_wire_high_water(wires.len() as u32);
        f
    };
    let with_wires = frame(&ctx, &r, &cam, 3);
    let s = p.stats_readback(&ctx).unwrap();
    assert_eq!(s[0], 4, "camera view: tree, hydrant, plate, sign assembly (got {s:?})");
    assert_eq!((s[8], s[10]), (1, 1), "the fading hydrant is drawn twice, leaving level and arriving level (got {s:?})");
    assert_eq!(p.wire_drawn_readback(&ctx).unwrap(), 1, "only the wire in view and in front of the wall is drawn");
    r.set_occlusion(false);
    let _ = frame(&ctx, &r, &cam, 3);
    assert_eq!(p.wire_drawn_readback(&ctx).unwrap(), 2, "occlusion off: the wire behind the wall is drawn too");
    r.set_occlusion(true);
    // The wire darkens its pixels (a thin ribbon at least a pixel wide along its length).
    let (wx0, wy) = project(&cam, Vec3::new(-1.0, 6.0, -20.0));
    let (wx1, _) = project(&cam, Vec3::new(6.0, 6.0, -20.0));
    let lum = |v: &[f32], i: usize| 0.2126 * v[i * 3] + 0.7152 * v[i * 3 + 1] + 0.0722 * v[i * 3 + 2];
    let mut cols = 0;
    for x in wx0.min(wx1)..wx0.max(wx1) {
        if (wy.saturating_sub(2)..(wy + 3).min(H as usize)).any(|y| lum(&without_wires, y * W as usize + x) - lum(&with_wires, y * W as usize + x) > 0.01) {
            cols += 1;
        }
    }
    let span = wx0.abs_diff(wx1);
    assert!(cols * 10 >= span * 8, "the wire is continuous across the frame: {cols} of {span} columns darkened");
    // Without the fade feature the band hydrant is a plain level-0 instance.
    p.set_features(eo_render::props::F_ALL & !eo_render::props::F_FADE);
    let _ = frame(&ctx, &r, &cam, 3);
    let s = p.stats_readback(&ctx).unwrap();
    assert_eq!((s[0], s[8], s[10]), (5, 0, 0), "no cross-fade with the feature off (got {s:?})");
    p.set_features(eo_render::props::F_ALL);
    // The lamp-light side table round-trips.
    let lights = [GpuLampLight { pos: [1.0, 7.4, 2.0], range: 24.0, colour: [1.0, 0.8, 0.6], intensity: 150.0, angle: 1.1, penumbra: 0.4, decay: 2.0, source_lift: 0.5, kind: Model::LampCobra as u32, pad: [0; 3] }; 2];
    r.write(&ctx.queue, Target::PropLights, 64, bytemuck::cast_slice(&lights));
    p.set_light_count(&ctx.queue, 3);
    let read = |b: &wgpu::Buffer, n: u64| {
        let st = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: n, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(b, 0, &st, 0, n);
        ctx.queue.submit([enc.finish()]);
        read_mapped(&ctx, &st).unwrap()
    };
    let got: Vec<GpuLampLight> = bytemuck::pod_collect_to_vec(&read(p.lights_buffer(), 192));
    assert_eq!(&got[1..3], &lights, "the lights are where the tile's range put them");
    let count: Vec<u32> = bytemuck::pod_collect_to_vec(&read(p.light_count_buffer(), 16));
    assert_eq!(count[0], 3, "the light count is the high-water mark");
}

#[test]
fn props2_boats_cook_only_with_their_bodies() {
    let paths = eo_data::DataPaths::discover().expect("repo data");
    let js = eo_props::JsProps::load().unwrap();
    let ca = eo_data::CaStandards::load(&paths).unwrap();
    let lens = ca.metric_key("signal-lens-size", "circularNewFaces").unwrap();
    let dome = lib2::dome_standards(&ca).unwrap();
    let bodies = eo_props::boats::load_bodies(&paths.repo_root).unwrap();
    let off = LibExtras::default();
    let on = LibExtras { boats: Some(&bodies), ..off };
    assert!(LibExtras::default().boats.is_none(), "off by default");
    assert_ne!(PropLibrary::key_with(&js, &off, dome), PropLibrary::key_with(&js, &on, dome), "on and off cook different libraries");
    let lib_off = PropLibrary::cook_full(&paths.repo_root, &js, lens, &off, dome).unwrap();
    let lib_on = PropLibrary::cook_full(&paths.repo_root, &js, lens, &on, dome).unwrap();
    assert!(lib_on.missing.iter().all(|m| !m.starts_with("boat ")), "every boat file read: {:?}", lib_on.missing);
    assert_eq!(lib_on.meshes.len(), lib_off.meshes.len() + 3 * eo_props::boats::BODIES.len(), "three levels per body");
    assert!(lib_on.meshes.len() <= 256, "the cull pass takes 256 meshes");
    for (i, b) in bodies.iter().enumerate() {
        let id = lib2::BOAT_BASE as usize + i;
        assert!(lib_off.models[id].meshes[0] == eo_render::props_lib::NONE, "off: no {} model", b.name);
        let m = lib_on.models[id];
        assert!(m.meshes[0] != m.meshes[1] && m.meshes[1] != m.meshes[2], "{}: three levels", b.name);
        assert!(lib_on.triangles(m.meshes[0]) > lib_on.triangles(m.meshes[2]), "{}: the far level is the lighter", b.name);
        assert_eq!(m.lod, lib2::BOAT_LOD);
        // The bounding sphere holds the hull (half its length) and reaches the masthead.
        assert!(m.radius >= b.loa as f32 / 2.0 - 0.05 && m.radius < (b.loa + b.height + b.draft) as f32, "{}: radius {}", b.name, m.radius);
    }
}

/// A moored boat drawn in its pose on the sea (props_types.wgsl `pr_afloat`; the pose is
/// eo-player's physics, eo-app boats.rs): a roll and a pitch in its instance word turn it;
/// with the switch off (eo-app `--no-boat-motion`) it is drawn at rest whatever the word
/// says. The shader's constants are the Rust ones.
#[test]
fn props2_boats_drawn_in_their_pose_only_with_their_switch() {
    let src = include_str!("../src/props_types.wgsl");
    assert!(src.contains(&format!("const PR_F_AFLOAT: u32 = {}u;", eo_render::props::F_AFLOAT)), "PR_F_AFLOAT is F_AFLOAT");
    assert!(src.contains(&format!("const PR_AFLOAT_BIT: u32 = 0x{:08x}u;", eo_props::boats::AFLOAT_BIT)), "PR_AFLOAT_BIT is boats::AFLOAT_BIT");
    assert!(eo_render::props::F_ALL & eo_render::props::F_AFLOAT != 0, "on by default");
    let ctx = ctx();
    let (mut v, mut i, mut o) = (vec![], vec![], vec![]);
    boxed(&mut v, &mut i, &mut o, Vec3::new(-300.0, -3.0, -300.0), Vec3::new(300.0, -2.0, 300.0));
    let r = Renderer::from_parts(&ctx, &v, &i, &o, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
    let paths = eo_data::DataPaths::discover().expect("repo data");
    let js = eo_props::JsProps::load().unwrap();
    let ca = eo_data::CaStandards::load(&paths).unwrap();
    let lens = ca.metric_key("signal-lens-size", "circularNewFaces").unwrap();
    let bodies = eo_props::boats::load_bodies(&paths.repo_root).unwrap();
    let x = LibExtras { boats: Some(&bodies), ..Default::default() };
    let lib = PropLibrary::cook_full(&paths.repo_root, &js, lens, &x, lib2::dome_standards(&ca).unwrap()).unwrap();
    r.attach_props(&ctx, &lib).unwrap();
    // each frame on its own: no resolve history, no exposure adaptation.
    r.look.set_sky_options(eo_render::sky::SkyOptions { taa: false, adaptation: false, rain: false, wet: false, ssr: false, ..Default::default() });
    let p = r.props.get().unwrap();
    // The sail yacht side on, 16 m off, its mast in view.
    let cam = Camera::new(Vec3::new(0.0, 4.0, 0.0), 0.0, 0.15, W as f32 / H as f32);
    let boat = |pad: u32| GpuPropInstance { pos: [0.0, 0.0, -16.0], yaw: std::f32::consts::FRAC_PI_2, scale: [1.0; 3], model: lib2::BOAT_BASE, variant: 0, lamp: 0, slope: 0.0, pad };
    let heeled = eo_props::boats::afloat_word(0.08, 0.03);
    let draw = |inst: GpuPropInstance, features: u32| {
        r.write(&ctx.queue, Target::PropInstances, 0, bytemuck::cast_slice(&[inst]));
        p.set_high_water(1);
        p.set_features(features);
        frame(&ctx, &r, &cam, 3)
    };
    // This renderer's frames carry a little noise that turns with the frame count (dither,
    // shadow and sky sampling): a channel counts as changed past 0.1 (HDR), and two draws of
    // one still scene set the bound a still boat keeps (20 to 40 channels of 172,800 here).
    let changed = |a: &[f32], b: &[f32]| a.iter().zip(b).filter(|(x, y)| (*x - *y).abs() > 0.1).count();
    let on = eo_render::props::F_ALL;
    let off = on & !eo_render::props::F_AFLOAT;
    let (p1, p2) = (draw(boat(0), on), draw(boat(0), on));
    let noise = changed(&p1, &p2);
    assert!(noise < 200, "two draws of a still scene differ in {noise} channels");
    let still = 2 * noise + 20;
    let s = draw(boat(heeled), off);
    assert!(changed(&p2, &s) <= still, "--no-boat-motion: drawn at rest ({} channels)", changed(&p2, &s));
    let h = draw(boat(heeled), on);
    assert!(changed(&s, &h) > 10 * still, "a roll and a pitch turn the hull ({} channels)", changed(&s, &h));
}
