//! (`--rt-gi`, `--rt-reflect`) The ray-traced passes on the GPU, on a small
//! scene: a ground slab with a canopy over part of it and a wall, at 15 h.
//! With the option compiled in and a device with ray queries, the pass runs
//! with no validation error, its result is finite, the open share it traces
//! under the canopy is well below the open ground's, and the frame differs
//! from the frame without it; with the option off the renderer makes no pass
//! and the device asks for no ray queries. Skipped, with a line, on a device
//! without ray queries (every backend but Metal). Needs a GPU.

use eo_render::camera::Camera;
use eo_render::look::{LookOptions, ShaderFeatures};
use eo_render::scene::{GpuObject, ObjectKind, Vertex};
use eo_render::{GpuContext, GpuOptions, Renderer, Targets, read_mapped, wgpu};
use glam::Vec3;

const W: u32 = 480;
const H: u32 = 270;

fn quad_box(min: Vec3, max: Vec3, kind: ObjectKind, colour: u32, verts: &mut Vec<Vertex>, idx: &mut Vec<u32>, objects: &mut Vec<GpuObject>) {
    let base = verts.len() as i32;
    let first = idx.len() as u32;
    for i in 0..8 {
        let p = Vec3::new(if i & 1 != 0 { max.x } else { min.x }, if i & 2 != 0 { max.y } else { min.y }, if i & 4 != 0 { max.z } else { min.z });
        verts.push(Vertex { pos: p.to_array(), normal: 0, color: colour });
    }
    let faces = [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]];
    for f in faces {
        idx.extend_from_slice(&[f[0], f[1], f[2], f[0], f[2], f[3]]);
    }
    objects.push(GpuObject::new(kind, min.to_array(), max.to_array(), first, 36, base));
}

fn scene(ctx: &GpuContext) -> Renderer {
    let (mut v, mut i, mut o) = (vec![], vec![], vec![]);
    let grey = 0xff80_8080;
    quad_box(Vec3::new(-100.0, -1.0, -100.0), Vec3::new(100.0, 0.0, 100.0), ObjectKind::Terrain, grey, &mut v, &mut i, &mut o);
    // a canopy 3 m over x 10..40, and a wall behind it.
    quad_box(Vec3::new(10.0, 3.0, -15.0), Vec3::new(40.0, 3.5, 15.0), ObjectKind::Building, grey, &mut v, &mut i, &mut o);
    quad_box(Vec3::new(45.0, 0.0, -30.0), Vec3::new(47.0, 25.0, 30.0), ObjectKind::Building, 0xffc0_c0c0, &mut v, &mut i, &mut o);
    let mut r = Renderer::from_parts(ctx, &v, &i, &o, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
    let opts = LookOptions { ao: false, ..r.look.options };
    r.look.set_options(&ctx.device, opts);
    r.look.set_time_of_day(15.0);
    r
}

/// 1.6 m up under the canopy, looking along -z and down at the ground under
/// it (the open ground past its edge at the top of the frame).
fn camera() -> Camera {
    Camera::new(Vec3::new(25.0, 1.6, 12.0), 0.0, -0.4, W as f32 / H as f32)
}

fn readback(ctx: &GpuContext, tex: &wgpu::Texture, texel: u32) -> Vec<u8> {
    let (w, h) = (tex.width(), tex.height());
    let row = (w * texel).div_ceil(256) * 256;
    let buf = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (row * h) as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let mut enc = ctx.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(tex.as_image_copy(), wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } }, wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 });
    ctx.queue.submit([enc.finish()]);
    let raw = read_mapped(ctx, &buf).unwrap();
    raw.chunks(row as usize).flat_map(|r| r[..(w * texel) as usize].to_vec()).collect()
}

/// Draw `frames` frames; the HDR target's bytes.
fn render(ctx: &GpuContext, r: &Renderer, frames: u32) -> Vec<u8> {
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
    let cam = camera();
    for _ in 0..frames {
        r.update(&ctx.queue, &cam);
        let mut enc = d.create_command_encoder(&Default::default());
        r.encode(&mut enc, &ov, None, &targets.depth, None);
        ctx.queue.submit([enc.finish()]);
    }
    readback(ctx, r.look.hdr_target(W, H).texture(), 8)
}

fn f16(b: &[u8]) -> f32 {
    half_to_f32(u16::from_le_bytes([b[0], b[1]]))
}

fn half_to_f32(h: u16) -> f32 {
    let s = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let e = ((h >> 10) & 0x1f) as i32;
    let m = (h & 0x3ff) as f32;
    match e {
        0 => s * m * 2f32.powi(-24),
        31 => if m == 0.0 { s * f32::INFINITY } else { f32::NAN },
        _ => s * (1.0 + m / 1024.0) * 2f32.powi(e - 15),
    }
}

// One test (the option's device request is process-wide): the frame without
// the option on a device that did not ask for ray queries, then on one that
// did, bit for bit; then the pass.
#[test]
fn traced_light_runs_and_sees_the_canopy() {
    let opts = GpuOptions::default();
    eo_render::rt_scene::set_wanted(false);
    let plain_ctx = GpuContext::new(GpuContext::create_instance(&opts), None, opts).expect("a GPU adapter is required");
    assert!(!plain_ctx.ray_query, "no ray queries unless asked for");
    let plain = eo_render::look::with_shader_features(ShaderFeatures::ALL, || render(&plain_ctx, &scene(&plain_ctx), 30));
    eo_render::rt_scene::set_wanted(true);
    let ctx = GpuContext::new(GpuContext::create_instance(&opts), None, opts).expect("a GPU adapter is required");
    if !ctx.ray_query {
        println!("SKIP: {} ({:?}) offers no inline ray queries (the pass is Metal only)", ctx.info.name, ctx.info.backend);
        return;
    }
    let scope = ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let off = eo_render::look::with_shader_features(ShaderFeatures::ALL, || {
        let r = scene(&ctx);
        assert!(!r.rt_scene_live() && r.rt_gi().is_none(), "no pass without the option compiled in");
        render(&ctx, &r, 30)
    });
    assert!(off == plain, "without the option the frame is bit for bit the frame of a device that never asked for ray queries");
    let rt = eo_render::look::with_shader_features(ShaderFeatures { rt_gi: true, ..ShaderFeatures::ALL }, || {
        let r = scene(&ctx);
        assert!(r.rt_scene_live() && r.rt_gi().is_some(), "the pass with the option and ray queries");
        let frame = render(&ctx, &r, 30);
        (r, frame)
    });
    let (r, lit) = rt;
    let err = pollster::block_on(scope.pop());
    assert!(err.is_none(), "validation: {err:?}");
    let g = r.rt_gi().unwrap();
    let s = g.stats();
    let sc = r.rt_scene().unwrap().stats();
    assert!(sc.instances >= 1 && s.traced_frames >= 29, "{s:?} {sc:?}");
    let (gi, z) = g.result_views().expect("the targets exist after a traced frame");
    let (gw, gh) = (gi.texture().width(), gi.texture().height());
    assert_eq!((gw, gh), (W / 2, H / 2));
    let gb = readback(&ctx, gi.texture(), 8);
    let zb = readback(&ctx, z.texture(), 4);
    let mut valid = 0;
    for k in 0..(gw * gh) as usize {
        let zv = f32::from_le_bytes(zb[k * 4..k * 4 + 4].try_into().unwrap());
        let px: Vec<f32> = (0..4).map(|c| f16(&gb[k * 8 + c * 2..])).collect();
        assert!(px.iter().all(|v| v.is_finite() && *v >= 0.0), "texel {k}: {px:?}");
        if zv >= 0.0 {
            valid += 1;
        }
    }
    assert!(valid > (gw * gh) as usize / 4, "the ground in view has a result ({valid} texels)");
    // the open share at a point under the canopy and in the open.
    let cam = camera();
    let at = |p: Vec3| -> (f32, f32) {
        let c = cam.view_proj() * p.extend(1.0);
        let (u, v) = (c.x / c.w * 0.5 + 0.5, 0.5 - c.y / c.w * 0.5);
        assert!((0.0..1.0).contains(&u) && (0.0..1.0).contains(&v), "{p} in view ({u}, {v})");
        let (x, y) = ((u * gw as f32) as usize, (v * gh as f32) as usize);
        let k = y.min(gh as usize - 1) * gw as usize + x.min(gw as usize - 1);
        (f16(&gb[k * 8 + 6..]), f32::from_le_bytes(zb[k * 4..k * 4 + 4].try_into().unwrap()))
    };
    let (open, zo) = at(Vec3::new(25.0, 0.0, -40.0));
    let (under, zu) = at(Vec3::new(25.0, 0.0, 9.0));
    println!("open share: open ground {open:.3} (view depth {zo:.1}), under the canopy {under:.3} (view depth {zu:.1}); stats {s:?}");
    assert!(zo > 0.0 && zu > 0.0, "both points have a result");
    assert!(open > 0.6, "open ground sees the sky ({open})");
    assert!(under < 0.2, "the canopy hides the sky ({under})");
    assert!(lit.chunks(2).all(|h| half_to_f32(u16::from_le_bytes([h[0], h[1]])).is_finite()), "the lit frame is finite");
    // the ground under the canopy is darker with the traced light (the rig gives it the open sky).
    let luma = |f: &[u8], p: Vec3| -> f32 {
        let c = cam.view_proj() * p.extend(1.0);
        let (x, y) = (((c.x / c.w * 0.5 + 0.5) * W as f32) as i32, ((0.5 - c.y / c.w * 0.5) * H as f32) as i32);
        let mut sum = 0.0;
        for dy in -2..=2 {
            for dx in -2..=2 {
                let k = ((y + dy).clamp(0, H as i32 - 1) * W as i32 + (x + dx).clamp(0, W as i32 - 1)) as usize * 8;
                let c: Vec<f32> = (0..3).map(|i| f16(&f[k + i * 2..])).collect();
                sum += 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
            }
        }
        sum / 25.0
    };
    let (a, b) = (luma(&off, Vec3::new(25.0, 0.0, 9.0)), luma(&lit, Vec3::new(25.0, 0.0, 9.0)));
    println!("under the canopy: {a:.4} without, {b:.4} with the traced light");
    assert!(b < a * 0.9, "the canopy's shade deepens ({b} vs {a})");
    // (`--rt-reflect`, stage R3) the reflections, alone and with the diffuse
    // light: no validation error, a finite frame and a finite result.
    for gi in [false, true] {
        let scope = ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let (r, frame) = eo_render::look::with_shader_features(ShaderFeatures { rt_gi: gi, rt_reflect: true, ..ShaderFeatures::ALL }, || {
            let r = scene(&ctx);
            assert!(r.rt_reflect().is_some() && r.rt_gi().is_some() == gi, "the passes the options ask for");
            let frame = render(&ctx, &r, 20);
            (r, frame)
        });
        let err = pollster::block_on(scope.pop());
        assert!(err.is_none(), "validation (with the diffuse light: {gi}): {err:?}");
        assert!(frame.chunks(2).all(|h| half_to_f32(u16::from_le_bytes([h[0], h[1]])).is_finite()), "the frame is finite");
        let p = r.rt_reflect().unwrap();
        assert!(p.stats().traced_frames >= 19, "{:?}", p.stats());
        let (out, z) = p.result_views().expect("the targets exist after a traced frame");
        let ob = readback(&ctx, out.texture(), 8);
        let zb = readback(&ctx, z.texture(), 4);
        let mut traced = 0;
        for k in 0..(out.texture().width() * out.texture().height()) as usize {
            let px: Vec<f32> = (0..4).map(|c| f16(&ob[k * 8 + c * 2..])).collect();
            assert!(px.iter().all(|v| v.is_finite() && *v >= 0.0), "texel {k}: {px:?}");
            traced += (f32::from_le_bytes(zb[k * 4..k * 4 + 4].try_into().unwrap()) >= 0.0) as usize;
        }
        println!("reflections (with the diffuse light: {gi}): {traced} texels traced; {:?}", p.stats());
    }
}
