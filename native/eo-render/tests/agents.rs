//! Moving things on the GPU (agents.rs): the GPU cull draws exactly the
//! agents in the frustum, applies the JS tier caps nearest first, a car casts
//! a sun shadow through the agent caster path (gone with the agents off), and
//! a skinned person and a car put pixels on screen. Needs a GPU and the
//! game's vehicle and character GLBs (or the agent cache).

use eo_agents::library::Library;
use eo_agents::{GpuAgent, CLIP_IDLE};
use eo_render::camera::Camera;
use eo_render::scene::{GpuObject, ObjectKind, Vertex};
use eo_render::{GpuContext, GpuOptions, Renderer, Targets, read_mapped, wgpu};
use glam::{Quat, Vec3};

const W: u32 = 320;
const H: u32 = 180;

fn ctx() -> GpuContext {
    let opts = GpuOptions::default();
    GpuContext::new(GpuContext::create_instance(&opts), None, opts).expect("a GPU adapter is required")
}

fn library() -> Library {
    let paths = eo_data::DataPaths::discover().expect("repo data");
    Library::load(&paths.repo_root, Some(&eo_assets::default_cache_dir())).expect("agent library")
}

fn slab(verts: &mut Vec<Vertex>, idx: &mut Vec<u32>, objects: &mut Vec<GpuObject>) {
    let (min, max) = (Vec3::new(-200.0, -1.0, -200.0), Vec3::new(200.0, 0.0, 200.0));
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

fn renderer(ctx: &GpuContext, lib: &Library) -> Renderer {
    let (mut v, mut i, mut o) = (vec![], vec![], vec![]);
    slab(&mut v, &mut i, &mut o);
    let r = Renderer::from_parts(ctx, &v, &i, &o, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
    r.attach_agents(ctx, lib).unwrap();
    r
}

fn car(lib: &Library, name: &str, pos: Vec3, heading: f32) -> GpuAgent {
    GpuAgent { pos: pos.to_array(), model: lib.model(name).unwrap(), rot: Quat::from_rotation_y(heading).to_array(), color: 0xff2b33a8, scale: 1.0, ..Default::default() }
}

fn person(lib: &Library, pos: Vec3, speed: f32) -> GpuAgent {
    GpuAgent { pos: pos.to_array(), model: lib.meta.people_models[0], rot: Quat::IDENTITY.to_array(), color: 0xffffffff, speed, anim: 0.3, scale: 1.0, clip: CLIP_IDLE, clip_phase: 0.2, ..Default::default() }
}

/// Render frames and read back the HDR target's luminance.
fn render(ctx: &GpuContext, r: &Renderer, cam: &Camera, frames: usize) -> Vec<f32> {
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
    let mut lum = vec![];
    for y in 0..H as usize {
        for x in 0..W as usize {
            let o = y * row as usize + x * 8;
            let c = |k: usize| f16(u16::from_le_bytes([raw[o + 2 * k], raw[o + 2 * k + 1]]));
            lum.push(0.2126 * c(0) + 0.7152 * c(1) + 0.0722 * c(2));
        }
    }
    lum
}

fn project(cam: &Camera, p: Vec3) -> (u32, u32) {
    let c = cam.view_proj() * p.extend(1.0);
    (((c.x / c.w * 0.5 + 0.5) * W as f32) as u32, ((0.5 - c.y / c.w * 0.5) * H as f32) as u32)
}

#[test]
fn gpu_cull_draws_the_frustum_and_caps_the_near_tier() {
    let ctx = ctx();
    let lib = library();
    let r = renderer(&ctx, &lib);
    let a = r.agents.get().unwrap();
    // Camera at the origin looking north (-z).
    let cam = Camera::new(Vec3::new(0.0, 1.7, 0.0), 0.0, 0.0, W as f32 / H as f32);
    let mut list = vec![];
    // Six sedans 8-18 m ahead (inside the 22 m LOD0 radius), one behind the camera.
    for k in 0..6 {
        list.push(car(&lib, "sedan", Vec3::new(-6.0 + k as f32 * 2.5, 0.0, -8.0 - k as f32 * 2.0), 0.0));
    }
    list.push(car(&lib, "sedan", Vec3::new(0.0, 0.0, 30.0), 0.0));
    // Three people ahead, one far to the side out of view.
    for k in 0..3 {
        list.push(person(&lib, Vec3::new(k as f32, 0.0, -5.0), 1.2));
    }
    list.push(person(&lib, Vec3::new(400.0, 0.0, 0.0), 0.0));
    a.upload(&ctx.queue, &list);
    render(&ctx, &r, &cam, 1);
    let stats = a.stats_readback(&ctx).unwrap();
    assert_eq!(stats[8] + stats[9], list.len() as u32, "every agent submitted");
    assert_eq!(stats[0], 9, "camera draws the 6 cars and 3 people in view, not the one behind nor the one aside");
    let cmds = a.commands_readback(&ctx).unwrap();
    let m = lib.models[lib.model("sedan").unwrap() as usize];
    let lod = |k: usize| cmds[m.meshes[k] as usize][1];
    // `HERO_OVERLAY.nearMax` 2: the two nearest at LOD0, the rest demoted to LOD1.
    assert_eq!(lod(0), 2, "LOD0 capped at 2: {:?}", (0..4).map(lod).collect::<Vec<_>>());
    assert_eq!(lod(1), 4);
    // The person tiers: 5 m is the hero tier (within 6 m).
    let p = lib.models[lib.meta.people_models[0] as usize];
    assert_eq!(cmds[p.meshes[0] as usize][1], 3);
    // Shadow casters: cascade 0 casts a car at LOD1 or coarser; the car behind the camera casts too.
    let n = a.mesh_count as usize;
    let cascade0: u32 = (0..n).map(|k| cmds[2 * n + k][1]).sum();
    assert!(cascade0 >= 9, "cascade 0 casters {cascade0}");
    assert_eq!(cmds[2 * n + m.meshes[0] as usize][1], 0, "no LOD0 caster");
}

#[test]
fn a_car_casts_a_sun_shadow_and_people_draw() {
    let ctx = ctx();
    let lib = library();
    let r = renderer(&ctx, &lib);
    r.look.set_time_of_day(12.0);
    let a = r.agents.get().unwrap();
    let f = r.look.frame();
    let sun = Vec3::new(f.key_dir[0] as f32, f.key_dir[1] as f32, f.key_dir[2] as f32);
    let away = -Vec3::new(sun.x, 0.0, sun.z).normalize();
    // Looking down from the shadow side (so the bus does not hide its own shadow).
    let look = -away;
    let cam = Camera::new(away * 25.0 + Vec3::Y * 40.0, look.x.atan2(-look.z), -1.0, W as f32 / H as f32);
    // A bus (12.5 m, 3 m tall) across the sun direction at the origin.
    a.upload(&ctx.queue, &[car(&lib, "cityBus", Vec3::ZERO, away.x.atan2(away.z) + std::f32::consts::FRAC_PI_2)]);
    let on = render(&ctx, &r, &cam, 6);
    let cmds = a.commands_readback(&ctx).unwrap();
    let n = a.mesh_count as usize;
    let per_list: Vec<u32> = (0..5).map(|l| (0..n).map(|k| cmds[l * n + k][1]).sum()).collect();
    assert_eq!(per_list[0], 1, "the bus is drawn by the camera: lists {per_list:?}");
    assert!(per_list[2] + per_list[3] + per_list[4] >= 1, "the bus casts into a cascade: lists {per_list:?} mask {}", r.look.encoded_shadow_mask());
    a.set_enabled(false);
    let off = render(&ctx, &r, &cam, 6);
    a.set_enabled(true);
    let at = |lum: &[f32], p: Vec3| {
        let (x, y) = project(&cam, p);
        lum[(y.min(H - 1) * W + x.min(W - 1)) as usize]
    };
    // 2.4 m from the bus's centre line on its far side from the sun: inside the
    // shadow of a 3 m body under a ~55 degree noon sun, in front of it as the camera sees it.
    let horizontal = (1.0 - sun.y * sun.y).sqrt().max(1e-3);
    let reach = 1.25 + 3.0 * horizontal / sun.y.max(0.1);
    assert!(reach > 2.6, "shadow reach {reach}");
    let shadow_pt = away * 2.4;
    assert!(at(&on, shadow_pt) < 0.6 * at(&off, shadow_pt), "shadow {} vs no agents {}", at(&on, shadow_pt), at(&off, shadow_pt));
    let lit_pt = -away * 9.0;
    assert!((at(&on, lit_pt) - at(&off, lit_pt)).abs() < 0.05 * at(&off, lit_pt).max(1e-3), "sunward ground unchanged");
    // The bus itself is drawn: pixels on its roof differ from the bare ground.
    assert!((at(&on, Vec3::new(0.0, 3.0, 0.0)) - at(&off, Vec3::new(0.0, 3.0, 0.0))).abs() > 1e-3);
    // A skinned person, walking, 4 m in front of a camera at eye height.
    let cam2 = Camera::new(Vec3::new(0.0, 1.6, 4.0), 0.0, -0.05, W as f32 / H as f32);
    a.upload(&ctx.queue, &[person(&lib, Vec3::ZERO, 1.3)]);
    let with = render(&ctx, &r, &cam2, 3);
    a.upload(&ctx.queue, &[]);
    let without = render(&ctx, &r, &cam2, 3);
    let changed = with.iter().zip(&without).filter(|(p, q)| (*p - *q).abs() > 1e-3).count();
    // 1.75 m at 4 m fills ~40 % of 180 px in height; a body is ~15-25 px wide.
    assert!(changed > 600, "person changed {changed} pixels");
}
