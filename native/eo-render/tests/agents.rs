//! Moving things on the GPU (agents.rs): the GPU cull draws exactly the
//! agents in the frustum, applies the JS tier caps nearest first, a car casts
//! a sun shadow through the agent caster path (gone with the agents off), and
//! a skinned person and a car put pixels on screen. Needs a GPU and the
//! game's vehicle and character GLBs (or the agent cache); the two tests that
//! draw people need the Rocketbox people set too (`external/people`, filled by
//! `scripts/setup-assets.py --only people`, or `EO_PEOPLE_ROOT`).

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
    let model = *lib.meta.people_models.first().unwrap_or_else(|| panic!("the agent library has no people: these tests need the Rocketbox people, looked for at {}", eo_data::external::PEOPLE.looked_in()));
    GpuAgent { pos: pos.to_array(), model, rot: Quat::IDENTITY.to_array(), color: 0xffffffff, speed, anim: 0.3, scale: 1.0, clip: CLIP_IDLE, clip_phase: 0.2, ..Default::default() }
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

/// `--car-detail 1`: the library's far slot is each body's own simplified
/// body (in the same mesh slots, so the people's mesh budget is as before),
/// a car 300 m out draws it, and the six near sedans all keep LOD0 (cap 6)
/// where level 0 keeps two.
#[test]
fn car_detail_draws_a_far_body_and_keeps_more_near_cars_whole() {
    let ctx = ctx();
    let paths = eo_data::DataPaths::discover().expect("repo data");
    let people = eo_agents::library::PeopleSource::from_env();
    let opts = eo_agents::library::CookOptions { far_body: true, ..Default::default() };
    let lib = Library::load_opts(&paths.repo_root, Some(&eo_assets::default_cache_dir()), &people, opts).expect("agent library with far bodies");
    let boxed = library();
    assert_eq!(lib.meshes.len(), boxed.meshes.len(), "the far bodies take the boxes' mesh slots");
    let (m, mb) = (lib.models[lib.model("sedan").unwrap() as usize], boxed.models[boxed.model("sedan").unwrap() as usize]);
    let (far, bx, lod2) = (lib.triangles(m.meshes[3]), boxed.triangles(mb.meshes[3]), lib.triangles(m.meshes[2]));
    assert!(far > 10 * bx && far < lod2, "far body {far} triangles: more than the box's {bx}, fewer than LOD 2's {lod2}");
    // Six sedans 8-18 m ahead and one 300 m out, the camera looking north.
    let cam = Camera::new(Vec3::new(0.0, 1.7, 0.0), 0.0, 0.0, W as f32 / H as f32);
    let mut list: Vec<GpuAgent> = (0..6).map(|k| car(&lib, "sedan", Vec3::new(-6.0 + k as f32 * 2.5, 0.0, -8.0 - k as f32 * 2.0), 0.0)).collect();
    list.push(car(&lib, "sedan", Vec3::new(0.0, 0.0, -300.0), 0.0));
    let lods = |level: u8, lib: &Library| {
        let (mut v, mut i, mut o) = (vec![], vec![], vec![]);
        slab(&mut v, &mut i, &mut o);
        let r = Renderer::from_parts(&ctx, &v, &i, &o, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
        r.attach_agents_with(&ctx, lib, level).unwrap();
        let a = r.agents.get().unwrap();
        a.upload(&ctx.queue, &list);
        render(&ctx, &r, &cam, 1);
        let cmds = a.commands_readback(&ctx).unwrap();
        let m = lib.models[lib.model("sedan").unwrap() as usize];
        (0..4).map(|k| cmds[m.meshes[k] as usize][1]).collect::<Vec<_>>()
    };
    assert_eq!(lods(0, &boxed), vec![2, 4, 0, 1], "level 0: two at LOD0, four at LOD1, the far car in the far slot");
    assert_eq!(lods(1, &lib), vec![6, 0, 0, 1], "level 1: all six at LOD0, the far car in the far slot");
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

/// A car in a tunnel's bore (eo_agents `AGENT_IN_BORE`, `--tunnels`) is lit by
/// the bore (structures.wgsl `sx_bore_light_at`), not by the sun and sky: at
/// noon its roof reads at the bore's interior level, far under the same car
/// in the open; the ground beside it is unchanged.
#[test]
fn a_car_in_a_bore_is_lit_by_the_bore_not_the_sun() {
    let ctx = ctx();
    let lib = library();
    let mut r = renderer(&ctx, &lib);
    r.look.set_time_of_day(12.0);
    // the sky's stages on (the clustered lights, the bore's interior level), as the app runs.
    r.look.set_options(&ctx.device, eo_render::look::LookOptions::default());
    // the bores' lamps lit (as a run with `--tunnels` draws them), the eye outside a bore.
    r.look.sky.set_bore(None, true);
    let a = r.agents.get().unwrap();
    // before the bores' pipelines exist a bore agent is drawn as any other (the bit taken off at upload).
    let open = car(&lib, "sedan", Vec3::ZERO, 0.3);
    let cam = Camera::new(Vec3::new(0.0, 9.0, 9.0), 0.0, -0.75, W as f32 / H as f32);
    a.upload(&ctx.queue, &[open]);
    let _ = render(&ctx, &r, &cam, 12); // the exposure and the histories settled
    let plain = render(&ctx, &r, &cam, 4);
    a.upload(&ctx.queue, &[GpuAgent { flags: eo_agents::bore_flags(200.0), ..open }]);
    assert!(!a.bores_drawn());
    let unmarked = render(&ctx, &r, &cam, 4);
    let total = |l: &[f32]| l.iter().map(|v| *v as f64).sum::<f64>();
    assert!((total(&unmarked) - total(&plain)).abs() <= 0.002 * total(&plain), "without the bores' pipelines a bore agent draws as before: {} vs {}", total(&unmarked), total(&plain));
    a.enable_bores();
    let deep = GpuAgent { flags: eo_agents::bore_flags(200.0), ..open };
    let near_portal = GpuAgent { flags: eo_agents::bore_flags(4.0), ..open };
    a.upload(&ctx.queue, &[open]);
    let lit = render(&ctx, &r, &cam, 4);
    a.upload(&ctx.queue, &[deep]);
    assert!(a.bores_drawn(), "a bore agent in the list draws the bores' pipelines");
    let bore = render(&ctx, &r, &cam, 4);
    a.upload(&ctx.queue, &[near_portal]);
    let mouth = render(&ctx, &r, &cam, 4);
    a.upload(&ctx.queue, &[]);
    let none = render(&ctx, &r, &cam, 4);
    assert!(!a.bores_drawn(), "no bore agent: the bores' pipelines draw nothing");
    // the car's pixels: those it changes against the bare slab, within its outline's box on screen.
    let (cx, cy) = project(&cam, Vec3::new(0.0, 0.8, 0.0));
    let (ex, _) = project(&cam, Vec3::new(2.5, 0.8, 0.0));
    let half = (ex as i32 - cx as i32).unsigned_abs().max(8);
    let inside = |i: usize| ((i as u32 % W) as i32 - cx as i32).unsigned_abs() <= half && ((i as u32 / W) as i32 - cy as i32).unsigned_abs() <= half;
    let body: Vec<usize> = (0..lit.len()).filter(|&i| inside(i) && (lit[i] - none[i]).abs() > 0.05 * none[i].max(1e-3)).collect();
    let at = |l: &[f32], p: Vec3| {
        let (x, y) = project(&cam, p);
        l[(y.min(H - 1) * W + x.min(W - 1)) as usize]
    };
    println!("roof: open {:.4}, bore {:.4}, mouth {:.4}, none {:.4}; ground: {:.4} {:.4} {:.4}", at(&lit, Vec3::new(0.0, 1.45, 0.0)), at(&bore, Vec3::new(0.0, 1.45, 0.0)), at(&mouth, Vec3::new(0.0, 1.45, 0.0)), at(&none, Vec3::new(0.0, 1.45, 0.0)), at(&lit, Vec3::new(5.0, 0.0, -3.0)), at(&bore, Vec3::new(5.0, 0.0, -3.0)), at(&none, Vec3::new(5.0, 0.0, -3.0)));
    assert!(body.len() > 400, "the car covers {} pixels", body.len());
    let mean = |l: &[f32]| body.iter().map(|&i| l[i] as f64).sum::<f64>() / body.len() as f64;
    let (m_lit, m_bore, m_mouth) = (mean(&lit), mean(&bore), mean(&mouth));
    println!("car mean luminance: in the open {m_lit:.4}, 200 m into a bore {m_bore:.4}, 4 m in from a portal {m_mouth:.4} ({} px)", body.len());
    assert!(m_bore < 0.2 * m_lit, "the car in the bore is not sunlit: {m_bore} vs {m_lit}");
    assert!(m_bore > 1e-5, "the bore's own light reaches the car: {m_bore}");
    assert!(m_mouth > m_bore, "the portal's daylight and the threshold zone near the mouth: {m_mouth} vs {m_bore}");
    // the slab away from the car: the same whatever the car's flag.
    let (x, y) = project(&cam, Vec3::new(5.0, 0.0, -3.0));
    let k = (y.min(H - 1) * W + x.min(W - 1)) as usize;
    assert!((lit[k] - bore[k]).abs() <= 0.02 * lit[k].max(1e-3), "the ground beside: {} vs {}", lit[k], bore[k]);
}

/// `--car-materials`: with every shader feature compiled in (the tests'
/// default, so the agents' pipelines carry the cars' own shading and Apple's
/// compiler takes them), a library cooked with the roles draws the frame of
/// the plain library while the level is 0, and with the level on the car's
/// pixels change and nothing else does.
#[test]
fn car_materials_change_the_car_alone_and_nothing_while_off() {
    let ctx = ctx();
    let paths = eo_data::DataPaths::discover().expect("repo data");
    let people = eo_agents::library::PeopleSource::Off;
    let cook = |car_materials| Library::load_opts(&paths.repo_root, Some(&eo_assets::default_cache_dir()), &people, eo_agents::library::CookOptions { car_materials, ..Default::default() }).expect("agent library");
    let (plain, roles) = (cook(false), cook(true));
    // A sedan side-on 7 m north of the camera, and the camera looking at it.
    let cam = Camera::new(Vec3::new(0.0, 1.5, 0.0), 0.0, -0.08, W as f32 / H as f32);
    let shot = |lib: &Library, level: u8| {
        let (mut v, mut i, mut o) = (vec![], vec![], vec![]);
        slab(&mut v, &mut i, &mut o);
        let mut r = Renderer::from_parts(&ctx, &v, &i, &o, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
        let options = eo_render::look::LookOptions { car_materials: level, ..r.look.options };
        r.look.set_options(&ctx.device, options);
        r.attach_agents(&ctx, lib).unwrap();
        let a = r.agents.get().unwrap();
        a.upload(&ctx.queue, &[car(lib, "sedan", Vec3::new(0.0, 0.0, -7.0), std::f32::consts::FRAC_PI_2)]);
        render(&ctx, &r, &cam, 2)
    };
    let (base, off, on) = (shot(&plain, 0), shot(&roles, 0), shot(&roles, 1));
    let differ = |a: &[f32], b: &[f32]| a.iter().zip(b).filter(|(x, y)| (*x - *y).abs() > 1e-3 * (1.0 + x.abs())).count();
    assert_eq!(differ(&base, &off), 0, "level 0: the roles and the plates' coordinates draw nothing");
    let changed = differ(&base, &on);
    assert!(changed > 200, "level 1 changes the car ({changed} pixels)");
    // Only pixels round the car: the slab's far corners are untouched.
    let corner = |img: &[f32]| (0..20).flat_map(|y| (0..20).map(move |x| y * W as usize + x)).chain((0..20).flat_map(|y| (0..20).map(move |x| (H as usize - 1 - y) * W as usize + W as usize - 1 - x))).map(|k| img[k]).collect::<Vec<_>>();
    assert_eq!(differ(&corner(&base), &corner(&on)), 0, "nothing but the car changes");
}

/// `--car-contact`: the cars' occlusion of the street (agents_contact.wgsl,
/// a pass of its own) darkens the slab at a car's foot and leaves the slab a
/// few metres off as it was; without the switch no pass is made. (The
/// screen-space occlusion is off here: the pass takes only what it left.)
#[test]
fn car_contact_darkens_the_street_at_the_car_alone() {
    let ctx = ctx();
    let paths = eo_data::DataPaths::discover().expect("repo data");
    let people = eo_agents::library::PeopleSource::Off;
    let lib = Library::load_opts(&paths.repo_root, Some(&eo_assets::default_cache_dir()), &people, eo_agents::library::CookOptions { car_materials: true, ..Default::default() }).expect("agent library");
    // A sedan side-on 7 m north of the camera (its nose to +x), the camera 3 m up looking down at it.
    let cam = Camera::new(Vec3::new(0.0, 3.0, 0.0), 0.0, -0.35, W as f32 / H as f32);
    let shot = |contact: u8| {
        let (mut v, mut i, mut o) = (vec![], vec![], vec![]);
        slab(&mut v, &mut i, &mut o);
        let mut r = Renderer::from_parts(&ctx, &v, &i, &o, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
        // (the screen-space occlusion off: where it already took the car's share the pass takes no more)
        let options = eo_render::look::LookOptions { car_contact: contact, ao: false, ..r.look.options };
        r.look.set_options(&ctx.device, options);
        r.attach_agents(&ctx, &lib).unwrap();
        let a = r.agents.get().unwrap();
        a.upload(&ctx.queue, &[car(&lib, "sedan", Vec3::new(0.0, 0.0, -7.0), std::f32::consts::FRAC_PI_2)]);
        let made = a.contact_reads(&Targets::new(&ctx.device, W, H, wgpu::TextureFormat::Rgba8UnormSrgb, 1).depth);
        (render(&ctx, &r, &cam, 2), made)
    };
    let ((off, made_off), (on, made_on)) = (shot(0), shot(1));
    assert!(!made_off && made_on, "the pass is made with the switch alone");
    let shape = eo_agents::vehicles::contact_shape(&eo_agents::vehicles::cook_body_with(&eo_agents::catalog::read_verified(&paths.repo_root, &eo_agents::catalog::AgentsJs::load().unwrap().vehicles["sedan"].lods[0].asset()).unwrap(), &eo_agents::vehicles::LampColours::from_js(&eo_agents::catalog::AgentsJs::load().unwrap()), true).unwrap());
    // the slab 5 cm out from the sill on the camera's side, along the car's
    // middle, and 10 cm in under the sill: darker (by the share of their light
    // that is the sky's: this slab is lit mostly by the key), under the sill most.
    let at = |p: Vec3| {
        let (x, y) = project(&cam, p);
        let k = (y.min(H - 1) * W + x.min(W - 1)) as usize;
        on[k] / off[k].max(1e-6)
    };
    let foot = at(Vec3::new(0.3, 0.0, -7.0 + shape.lower_half_w + 0.05));
    let under = at(Vec3::new(0.3, 0.0, -7.0 + shape.lower_half_w - 0.1));
    eprintln!("the slab at the car's foot keeps {foot}, under the sill {under}");
    assert!(foot < 0.99 && under < foot, "the slab at the car's foot keeps {foot}, under the sill {under}");
    // the slab's far corners (tens of metres off) are untouched.
    let corner = |img: &[f32]| (0..20).flat_map(|y| (0..20).map(move |x| (H as usize - 1 - y) * W as usize + x)).map(|k| img[k]).collect::<Vec<_>>();
    let (a, b) = (corner(&off), corner(&on));
    assert!(a.iter().zip(&b).all(|(p, q)| (p - q).abs() <= 1e-3 * (1.0 + p.abs())), "the far slab changed");
}

/// `--car-finish`: compiled in but at level 0 the frame is the one without it
/// (bit-identical, the one pipeline); at level 1 the car alone changes, drawn
/// with a person beside it by the cars' and the people's own pipelines (their
/// meshes held apart in the library: `person_mesh_split`), the person as before.
#[test]
fn car_finish_changes_the_car_alone_and_nothing_while_off() {
    let ctx = ctx();
    let lib = library();
    let paths = eo_data::DataPaths::discover().expect("repo data");
    let roles = Library::load_opts(&paths.repo_root, Some(&eo_assets::default_cache_dir()), &eo_agents::library::PeopleSource::Off, eo_agents::library::CookOptions { car_materials: true, ..Default::default() }).expect("agent library");
    let lib = if lib.meta.people_models.is_empty() { roles } else { lib };
    if !lib.meta.people_models.is_empty() {
        let k = eo_render::agents::person_mesh_split(&lib).expect("the bodies' meshes come before the people's");
        assert!(k > 0 && k < lib.meshes.len() as u32);
    }
    let cam = Camera::new(Vec3::new(0.0, 1.5, 0.0), 0.0, -0.08, W as f32 / H as f32);
    let all = eo_render::look::ShaderFeatures::ALL;
    let shot = |finish: bool, level: u8| {
        eo_render::look::with_shader_features(eo_render::look::ShaderFeatures { car_finish: finish, ..all }, || {
            let (mut v, mut i, mut o) = (vec![], vec![], vec![]);
            slab(&mut v, &mut i, &mut o);
            let mut r = Renderer::from_parts(&ctx, &v, &i, &o, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
            let options = eo_render::look::LookOptions { car_materials: 1, car_finish: level, ..r.look.options };
            r.look.set_options(&ctx.device, options);
            r.attach_agents(&ctx, &lib).unwrap();
            let a = r.agents.get().unwrap();
            let mut agents = vec![car(&lib, "sedan", Vec3::new(0.0, 0.0, -7.0), std::f32::consts::FRAC_PI_2)];
            if !lib.meta.people_models.is_empty() {
                agents.push(person(&lib, Vec3::new(-3.0, 0.0, -5.0), 0.0));
            }
            a.upload(&ctx.queue, &agents);
            render(&ctx, &r, &cam, 2)
        })
    };
    let (base, off, on) = (shot(false, 0), shot(true, 0), shot(true, 1));
    let differ = |a: &[f32], b: &[f32]| a.iter().zip(b).filter(|(x, y)| (*x - *y).abs() > 1e-3 * (1.0 + x.abs())).count();
    assert_eq!(differ(&base, &off), 0, "level 0: the finish draws nothing");
    let changed = differ(&base, &on);
    assert!(changed > 50, "level 1 changes the car ({changed} pixels)");
    let region = |img: &[f32], x0: u32, x1: u32| (0..H).flat_map(|y| (x0..x1).map(move |x| (y * W + x) as usize)).map(|k| img[k]).collect::<Vec<_>>();
    // the person, left of the car (its own pipeline): as before
    if !lib.meta.people_models.is_empty() {
        let (px, _) = project(&cam, Vec3::new(-3.0, 0.9, -5.0));
        let (x0, x1) = (px.saturating_sub(12), (px + 12).min(W));
        assert_eq!(differ(&region(&base, x0, x1), &region(&on, x0, x1)), 0, "the person is drawn as before");
    }
    let corner = |img: &[f32]| (0..20).flat_map(|y| (0..20).map(move |x| (H as usize - 1 - y) * W as usize + W as usize - 1 - x)).map(|k| img[k]).collect::<Vec<_>>();
    assert_eq!(differ(&corner(&base), &corner(&on)), 0, "nothing but the car changes");
}
