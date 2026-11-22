//! Lamp and headlight shadows (`--light-shadows`, `--headlight-shadows`;
//! light_shadow.rs) on the GPU (needs a GPU): a box between a street lamp
//! and the ground darkens the ground behind it only with the feature on (and
//! leaves the ground the lamp sees as it was); the lamps selected and the
//! tiles they hold are the same frame after frame under a still camera and in
//! a second renderer, and so is the frame list's order; no frame draws more
//! new lamp maps than `NEW_MAPS_PER_FRAME`; a lamp behind a wall lights no fog
//! in front of it; the timestamp slots are written and nothing raises a
//! validation error.

use eo_render::camera::Camera;
use eo_render::light_shadow::{self, NEW_MAPS_PER_FRAME};
use eo_render::look::{LightingOptions, LookOptions};
use eo_render::scene::{GpuObject, ObjectKind, Vertex};
use eo_render::sky::{SkyOptions, WeatherMode};
use eo_render::{GpuContext, GpuOptions, Renderer, Targets, lighting, read_mapped, wgpu};
use eo_sky::lights::{LampKind, LampLight};
use glam::Vec3;

fn ctx() -> GpuContext {
    let opts = GpuOptions::default();
    GpuContext::new(GpuContext::create_instance(&opts), None, opts).expect("a GPU adapter is required")
}

const W: u32 = 320;
const H: u32 = 180;

fn quad_box(min: Vec3, max: Vec3, verts: &mut Vec<Vertex>, idx: &mut Vec<u32>, objects: &mut Vec<GpuObject>) {
    let base = verts.len() as i32;
    let first = idx.len() as u32;
    for i in 0..8 {
        let p = Vec3::new(if i & 1 != 0 { max.x } else { min.x }, if i & 2 != 0 { max.y } else { min.y }, if i & 4 != 0 { max.z } else { min.z });
        verts.push(Vertex { pos: p.to_array(), normal: 0, color: 0xff80_8080 });
    }
    let faces = [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]];
    for f in faces {
        idx.extend_from_slice(&[f[0], f[1], f[2], f[0], f[2], f[3]]);
    }
    objects.push(GpuObject::new(ObjectKind::Building, min.to_array(), max.to_array(), first, 36, base));
}

/// A ground slab and `boxes`, at night, clear, with the options given (no
/// occlusion pass, no temporal resolve, no wet ground or screen-space
/// reflection, no screen-space occlusion: none of their histories blur what
/// is measured here).
fn renderer(ctx: &GpuContext, lighting: LightingOptions, boxes: &[(Vec3, Vec3)], weather: eo_sky::Kind) -> Renderer {
    let (mut v, mut i, mut o) = (vec![], vec![], vec![]);
    quad_box(Vec3::new(-200.0, -1.0, -200.0), Vec3::new(200.0, 0.0, 200.0), &mut v, &mut i, &mut o);
    for (a, b) in boxes {
        quad_box(*a, *b, &mut v, &mut i, &mut o);
    }
    let mut r = Renderer::from_parts(ctx, &v, &i, &o, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
    let opts = LookOptions { ao: false, lighting, ..r.look.options };
    r.look.set_options(&ctx.device, opts);
    r.look.set_time_of_day(21.0);
    r.look.set_sky_options(SkyOptions { weather: WeatherMode::Preset(weather), taa: false, rain: false, wet: false, ssr: false, ..SkyOptions::default() });
    r
}

fn lamps_on(n: u8) -> LightingOptions {
    LightingOptions { light_shadows: n, ..LightingOptions::OFF }
}

/// A full-cutoff street luminaire at `head`, ~30 lux at its foot, in white
/// (not the street lamp role's colour, which the option turns to the LED's).
fn street_lamp(head: Vec3) -> LampLight {
    LampLight {
        pos: [head.x as f64, head.y as f64, head.z as f64],
        candela: 30.0 * (head.y as f64).powi(2),
        color: [1.0, 1.0, 1.0],
        range: head.y as f64 * 2.6,
        dir: [0.0, -1.0, 0.0],
        cos_outer: 75f64.to_radians().cos(),
        cos_inner: 60f64.to_radians().cos(),
        kind: LampKind::Street,
    }
}

struct Frame {
    out: wgpu::Texture,
    view: wgpu::TextureView,
    targets: Targets,
}

fn frame(ctx: &GpuContext) -> Frame {
    let out = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = out.create_view(&Default::default());
    Frame { out, view, targets: Targets::new(&ctx.device, W, H, wgpu::TextureFormat::Rgba8UnormSrgb, 1) }
}

fn draw(ctx: &GpuContext, r: &Renderer, f: &Frame, cam: &Camera, set: Option<&wgpu::QuerySet>) {
    r.set_lighting_timestamps(set.map(|q| (q.clone(), 0)));
    r.update(&ctx.queue, cam);
    let mut enc = ctx.device.create_command_encoder(&Default::default());
    r.encode(&mut enc, &f.view, None, &f.targets.depth, None);
    ctx.queue.submit([enc.finish()]);
    r.set_lighting_timestamps(None);
}

fn read(ctx: &GpuContext, f: &Frame) -> Vec<u8> {
    let row = (W * 4).div_ceil(256) * 256;
    let b = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (row * H) as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let mut enc = ctx.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        f.out.as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &b, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(H) } },
        wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
    );
    ctx.queue.submit([enc.finish()]);
    let raw = read_mapped(ctx, &b).unwrap();
    raw.chunks(row as usize).flat_map(|r| r[..(W * 4) as usize].to_vec()).collect()
}

fn luma(px: &[u8], x: u32, y: u32) -> f64 {
    let o = ((y * W + x) * 4) as usize;
    0.2126 * px[o] as f64 + 0.7152 * px[o + 1] as f64 + 0.0722 * px[o + 2] as f64
}

/// The mean luma of a small window round the pixel a world point lands on.
fn luma_at(px: &[u8], cam: &Camera, p: Vec3) -> f64 {
    let c = cam.view_proj() * p.extend(1.0);
    let (x, y) = (((c.x / c.w * 0.5 + 0.5) * W as f32) as i32, ((0.5 - c.y / c.w * 0.5) * H as f32) as i32);
    assert!(x >= 3 && y >= 3 && x < W as i32 - 3 && y < H as i32 - 3, "{p} lands off the frame ({x}, {y})");
    let mut s = 0.0;
    for dy in -2..=2 {
        for dx in -2..=2 {
            s += luma(px, (x + dx) as u32, (y + dy) as u32);
        }
    }
    s / 25.0
}

const HEAD: Vec3 = Vec3::new(0.0, 8.0, -20.0);
/// A 4 m tall box 2-3 m from the lamp's foot: its shadow on the ground runs from 3 m to 6 m out.
const BOX: (Vec3, Vec3) = (Vec3::new(2.0, 0.0, -20.5), Vec3::new(3.0, 4.0, -19.5));

fn the_view(ctx: &GpuContext, r: &Renderer, frames: usize) -> (Vec<u8>, Camera) {
    let f = frame(ctx);
    r.look.sky.set_lamps(&ctx.queue, &[street_lamp(HEAD)]);
    let aspect = W as f32 / H as f32;
    // from behind and above, looking down at the lamp's pool.
    let eye = Vec3::new(0.0, 14.0, -2.0);
    let at = Vec3::new(1.0, 0.0, -20.0);
    let to = at - eye;
    let cam = Camera::new(eye, to.x.atan2(-to.z), (to.y / Vec3::new(to.x, 0.0, to.z).length()).atan(), aspect);
    for _ in 0..frames {
        draw(ctx, r, &f, &cam, None);
    }
    (read(ctx, &f), cam)
}

#[test]
fn a_box_between_a_lamp_and_the_ground_darkens_the_ground_only_with_the_feature_on() {
    let ctx = ctx();
    let scope = ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let off = renderer(&ctx, LightingOptions::OFF, &[BOX], eo_sky::Kind::Clear);
    let on = renderer(&ctx, lamps_on(1), &[BOX], eo_sky::Kind::Clear);
    let bare = renderer(&ctx, lamps_on(1), &[], eo_sky::Kind::Clear);
    let (px_off, cam) = the_view(&ctx, &off, 6);
    let (px_on, _) = the_view(&ctx, &on, 6);
    let (px_bare, _) = the_view(&ctx, &bare, 6);
    let err = pollster::block_on(scope.pop());
    assert!(err.is_none(), "validation error: {err:?}");
    assert_eq!(off.light_shadow.stats().frames, 0, "the passes ran with the option off");
    assert!(on.light_shadow.stats().frames > 0, "the passes did not run: {:?}", on.light_shadow.stats());
    let sel = on.light_shadow.readback(&ctx).expect("a selection");
    assert_eq!(sel.selected, 1, "{sel:?}");
    assert_eq!(sel.lamps[0].1, 0, "the lamp holds tile 0: {sel:?}");
    assert_eq!(sel.new_total, 1, "one map drawn, then kept: {sel:?}");
    // behind the box (4.5 m from the foot, in its shadow), beside it on the ground the lamp sees
    // (4.5 m out the other way), and the ground under the bare lamp at the same place.
    let shadowed = Vec3::new(4.5, 0.0, -20.0);
    let open = Vec3::new(-4.5, 0.0, -20.0);
    let (s_off, s_on, s_bare) = (luma_at(&px_off, &cam, shadowed), luma_at(&px_on, &cam, shadowed), luma_at(&px_bare, &cam, shadowed));
    let (o_off, o_on) = (luma_at(&px_off, &cam, open), luma_at(&px_on, &cam, open));
    println!("behind the box: off {s_off:.1}, on {s_on:.1}, no box (on) {s_bare:.1}; the open side: off {o_off:.1}, on {o_on:.1}");
    assert!(s_off > 40.0, "the lamp does not light the ground behind the box without the feature ({s_off})");
    assert!(s_on < 0.5 * s_off, "the box casts no shadow with the feature on ({s_off} -> {s_on})");
    assert!((s_bare - s_off).abs() < 3.0, "without the box the ground is lit as before ({s_off} -> {s_bare})");
    assert!((o_on - o_off).abs() < 3.0, "the ground the lamp sees changed ({o_off} -> {o_on})");
}

/// A grid of 60 lamps 12 m apart round the eye.
fn many_lamps() -> Vec<LampLight> {
    (0..60).map(|k| street_lamp(Vec3::new((k % 10) as f32 * 12.0 - 54.0, 8.0, -(k / 10) as f32 * 12.0 - 8.0))).collect()
}

fn grid_camera() -> Camera {
    Camera::new(Vec3::new(0.0, 20.0, 10.0), 0.0, -0.45, W as f32 / H as f32)
}

#[test]
fn the_selection_is_stable_and_no_frame_draws_more_than_its_budget_of_new_maps() {
    let ctx = ctx();
    let scope = ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let run = |frames: usize| {
        let r = renderer(&ctx, lamps_on(1), &[BOX], eo_sky::Kind::Clear);
        r.look.sky.set_lamps(&ctx.queue, &many_lamps());
        let f = frame(&ctx);
        let cam = grid_camera();
        let mut per_frame = vec![];
        let mut sels = vec![];
        for _ in 0..frames {
            draw(&ctx, &r, &f, &cam, None);
            let s = r.light_shadow.readback(&ctx).expect("a selection");
            per_frame.push(s.new_maps);
            sels.push(s);
        }
        let order = r.light_shadow.order_readback(&ctx, &r.look.sky).expect("the order");
        (per_frame, sels, order)
    };
    let (per_frame, sels, order) = run(14);
    let (per_frame_b, sels_b, order_b) = run(14);
    let err = pollster::block_on(scope.pop());
    assert!(err.is_none(), "validation error: {err:?}");
    println!("new maps a frame: {per_frame:?}; lamps in the frame list {}", order.len());
    assert!(order.len() >= 20, "the frame list holds the grid's lamps in view ({})", order.len());
    // the budget: never more than NEW_MAPS_PER_FRAME new maps a frame, 16 maps after 8 frames, then none.
    assert!(per_frame.iter().all(|&n| n <= NEW_MAPS_PER_FRAME), "{per_frame:?}");
    assert_eq!(per_frame.iter().sum::<u32>(), 16, "{per_frame:?}");
    assert!(per_frame[..8].iter().all(|&n| n == NEW_MAPS_PER_FRAME) && per_frame[8..].iter().all(|&n| n == 0), "{per_frame:?}");
    let last = sels.last().unwrap();
    assert_eq!(last.selected, 16);
    assert!(last.lamps.iter().all(|l| l.1 != u32::MAX), "every selected lamp holds a tile: {last:?}");
    // a still camera: the same lamps in the same tiles, frame after frame and in a second renderer.
    for s in &sels[8..] {
        assert_eq!(s.lamps, last.lamps, "the selection moved under a still camera");
        assert_eq!(s.tiles, last.tiles);
    }
    assert_eq!(sels_b.last().unwrap().lamps, last.lamps, "a second run selects otherwise");
    assert_eq!(sels_b.last().unwrap().tiles, last.tiles);
    assert_eq!(per_frame_b, per_frame);
    assert_eq!(order_b, order, "the frame list's lights come in another order in a second run");
    // ranked nearest first, and every selected lamp's record names its tile (spare word = tile + 1), the others -1.
    let eye = grid_camera().eye;
    let od: Vec<f32> = order.iter().map(|(p, _, _)| Vec3::from(*p).distance(eye)).collect();
    assert!(od.windows(2).all(|w| w[0] <= w[1]), "{od:?}");
    let marked = order.iter().filter(|(_, _, pad)| *pad >= 1.0).count();
    assert_eq!(marked, 16, "{order:?}");
    assert!(order.iter().all(|(_, _, pad)| *pad >= 1.0 || *pad == -1.0), "{order:?}");
    // nearest first: each selected lamp is no farther from the eye than the next.
    let d: Vec<f32> = last.lamps.iter().map(|(k, _)| Vec3::new(f32::from_bits(k[0]), f32::from_bits(k[1]), f32::from_bits(k[2])).distance(eye)).collect();
    assert!(d.windows(2).all(|w| w[0] <= w[1] + 1e-4), "{d:?}");
}

#[test]
fn a_lamp_behind_a_wall_lights_no_fog_in_front_of_it_and_the_slots_are_timed() {
    let ctx = ctx();
    // a lamp 14 m up behind a wall taller than it, the eye in front of the wall: without the
    // shadows its light passes through the wall into the fog on the eye's side; with them that
    // fog (inside the lamp's map: under 72 deg from the nadir) is dark.
    let head = Vec3::new(0.0, 14.0, -20.0);
    let wall = (Vec3::new(-8.0, 0.0, -14.0), Vec3::new(8.0, 18.0, -13.0));
    let lighting = |n: u8| LightingOptions { light_shadows: n, lit_fog: 1, ..LightingOptions::OFF };
    let scope = ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let set = ctx.device.create_query_set(&wgpu::QuerySetDescriptor { label: None, ty: wgpu::QueryType::Timestamp, count: lighting::QUERIES_PER_FRAME });
    let mut vols = vec![];
    for n in [0u8, 1] {
        let r = renderer(&ctx, lighting(n), &[wall], eo_sky::Kind::Fog);
        r.look.sky.set_lamps(&ctx.queue, &[street_lamp(head)]);
        let f = frame(&ctx);
        let cam = Camera::new(Vec3::new(0.0, 1.7, 0.0), 0.0, 0.0, W as f32 / H as f32);
        for k in 0..12 {
            draw(&ctx, &r, &f, &cam, (k == 11).then_some(&set));
        }
        assert!(r.lit_fog.stats().frames > 0, "the lit fog did not run");
        vols.push(r.lit_fog.integrated_bytes(&ctx).expect("the lit fog's volume"));
    }
    let err = pollster::block_on(scope.pop());
    assert!(err.is_none(), "validation error: {err:?}");
    // the in-scatter integrated from the eye to just before the wall (12 m), over the frame's
    // middle columns (the wall's) from a quarter to three quarters of its height.
    let [cols, rows, slices] = eo_render::lit_fog::LEVELS[0];
    let k = ((eo_render::lit_fog::depth_slice(12.0) * slices as f32) as u32).saturating_sub(1);
    let lum = |vol: &[u8], i: u32, j: u32| {
        let o = (((k * rows + j) * cols + i) * 8) as usize;
        (0..3).map(|c| half(u16::from_le_bytes([vol[o + 2 * c], vol[o + 2 * c + 1]]))).sum::<f32>()
    };
    let middle = |vol: &[u8]| (cols / 2 - 10..cols / 2 + 10).flat_map(|i| (rows / 4..3 * rows / 4).map(move |j| (i, j))).map(|(i, j)| lum(vol, i, j)).sum::<f32>();
    let (m_off, m_on) = (middle(&vols[0]), middle(&vols[1]));
    println!("the lit fog in front of the wall (slice {k}): off {m_off:.5}, on {m_on:.5}");
    assert!(m_off > 0.0, "the lamp lights no fog in front of the wall even without the shadows");
    assert!(m_on < 0.3 * m_off, "the fog in the wall's shadow is still lit ({m_off} -> {m_on})");
}

fn half(b: u16) -> f32 {
    let s = if b & 0x8000 != 0 { -1.0 } else { 1.0 };
    let e = ((b >> 10) & 0x1f) as i32;
    let m = (b & 0x3ff) as f32;
    s * if e == 0 { m / 1024.0 * 2f32.powi(-14) } else if e == 31 { f32::INFINITY } else { (1.0 + m / 1024.0) * 2f32.powi(e - 15) }
}

#[test]
fn off_the_passes_never_run_and_nothing_is_made() {
    let ctx = ctx();
    let r = renderer(&ctx, LightingOptions::OFF, &[BOX], eo_sky::Kind::Clear);
    let _ = the_view(&ctx, &r, 3);
    assert_eq!(r.light_shadow.stats(), light_shadow::LightShadowStats::default());
    assert!(r.light_shadow.readback(&ctx).is_none());
}
