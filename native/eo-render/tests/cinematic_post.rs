//! The cinematic look's post pass on the GPU (`--look cinematic`, stage L1):
//! it equals eo-look's CPU reference (`cinematic::grade::cinematic_display_lut`)
//! by day and at night, it is deterministic, the grade's table is uploaded
//! once for a held hour (not every frame), and the default frame does not
//! change when the look is compiled in (tests/look.rs covers the default's
//! own transform). Needs a GPU.

use eo_look::cinematic::grade;
use eo_render::camera::Camera;
use eo_render::look::{LookOptions, LookProfile};
use eo_render::scene::{GpuObject, ObjectKind, Vertex};
use eo_render::{GpuContext, GpuOptions, Renderer, Targets, read_mapped, wgpu};
use glam::Vec3;

fn ctx() -> GpuContext {
    let opts = GpuOptions::default();
    GpuContext::new(GpuContext::create_instance(&opts), None, opts).expect("a GPU adapter is required")
}

const W: u32 = 320;
const H: u32 = 180;

fn tiny_scene() -> (Vec<Vertex>, Vec<u32>, Vec<GpuObject>) {
    // One box far behind the camera: the frame is all sky.
    let (min, max) = (Vec3::new(-1.0, 0.0, 5000.0), Vec3::new(1.0, 1.0, 5002.0));
    let mut v = vec![];
    for i in 0..8 {
        let p = Vec3::new(if i & 1 != 0 { max.x } else { min.x }, if i & 2 != 0 { max.y } else { min.y }, if i & 4 != 0 { max.z } else { min.z });
        v.push(Vertex { pos: p.to_array(), normal: 0, color: 0xff80_8080 });
    }
    let mut idx = vec![];
    for f in [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]] {
        idx.extend_from_slice(&[f[0], f[1], f[2], f[0], f[2], f[3]]);
    }
    (v, idx, vec![GpuObject::new(ObjectKind::Building, min.to_array(), max.to_array(), 0, 36, 0)])
}

fn f16_to_f32(h: u16) -> f32 {
    let s = ((h >> 15) & 1) as u32;
    let e = ((h >> 10) & 0x1f) as i32;
    let m = (h & 0x3ff) as u32;
    let v = if e == 0 {
        (m as f32) * 2f32.powi(-24)
    } else if e == 31 {
        f32::INFINITY
    } else {
        (1.0 + m as f32 / 1024.0) * 2f32.powi(e - 15)
    };
    if s == 1 { -v } else { v }
}

/// `frames` frames, then the HDR scene target and the output (rgba8 sRGB).
fn render(ctx: &GpuContext, r: &Renderer, cam: &Camera, frames: usize) -> (Vec<[f32; 4]>, Vec<[u8; 4]>) {
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
    let row16 = (W * 8).div_ceil(256) * 256;
    let row8 = (W * 4).div_ceil(256) * 256;
    let b16 = d.create_buffer(&wgpu::BufferDescriptor { label: None, size: (row16 * H) as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let b8 = d.create_buffer(&wgpu::BufferDescriptor { label: None, size: (row8 * H) as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let mut enc = d.create_command_encoder(&Default::default());
    let ext = wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 };
    enc.copy_texture_to_buffer(hdr.texture().as_image_copy(), wgpu::TexelCopyBufferInfo { buffer: &b16, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row16), rows_per_image: Some(H) } }, ext);
    enc.copy_texture_to_buffer(out.as_image_copy(), wgpu::TexelCopyBufferInfo { buffer: &b8, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row8), rows_per_image: Some(H) } }, ext);
    ctx.queue.submit([enc.finish()]);
    let raw16 = read_mapped(ctx, &b16).unwrap();
    let raw8 = read_mapped(ctx, &b8).unwrap();
    let mut hdr_px = vec![];
    let mut out_px = vec![];
    for y in 0..H as usize {
        for x in 0..W as usize {
            let o = y * row16 as usize + x * 8;
            let h = |k: usize| f16_to_f32(u16::from_le_bytes([raw16[o + 2 * k], raw16[o + 2 * k + 1]]));
            hdr_px.push([h(0), h(1), h(2), h(3)]);
            let o8 = y * row8 as usize + x * 4;
            out_px.push([raw8[o8], raw8[o8 + 1], raw8[o8 + 2], raw8[o8 + 3]]);
        }
    }
    (hdr_px, out_px)
}

/// A renderer with the cinematic look on (the bloom off: the pass's own transform alone).
fn cinematic(ctx: &GpuContext) -> Renderer {
    let (v, i, o) = tiny_scene();
    let mut r = Renderer::from_parts(ctx, &v, &i, &o, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
    let base = r.look.options;
    let opts = LookOptions { look: LookProfile::Cinematic, sky: eo_render::sky::SkyOptions { cinematic: true, bloom: false, ..base.sky }, ..base };
    r.look.set_options(&ctx.device, opts);
    r
}

#[test]
fn gpu_cinematic_post_equals_the_cpu_display_by_day_and_at_night() {
    let ctx = ctx();
    let r = cinematic(&ctx);
    let luts: Vec<_> = grade::GRADE_KEYS.iter().map(grade::bake_lut).collect();
    for (hour, yaw, pitch) in [(15.0, 2.0f32, 0.25f32), (18.6, -1.6, 0.05), (22.0, 0.0, 0.6)] {
        r.look.set_time_of_day(hour);
        let cam = Camera::new(Vec3::new(0.0, 2.0, 0.0), yaw, pitch, W as f32 / H as f32);
        let (hdr, out) = render(&ctx, &r, &cam, 2);
        let f = r.look.frame();
        let overcast = r.look.sky_terms().map(|t| t.weather.profile.overcast).unwrap_or(0.0);
        let w = grade::key_weights(f.model.sun.altitude_deg, f.hour, overcast);
        assert_eq!(r.look.cine().and_then(|c| c.weights()), Some(w), "{hour}: the table is the hour's");
        let lut = grade::blend_luts(&luts, &w);
        let mut worst = 0i32;
        let mut lit = 0;
        for (k, (h, o)) in hdr.iter().zip(&out).enumerate() {
            let uv = [((k as u32 % W) as f64 + 0.5) / W as f64, ((k as u32 / W) as f64 + 0.5) / H as f64];
            let want = grade::cinematic_display_lut([h[0] as f64, h[1] as f64, h[2] as f64], f.exposure, &lut, uv);
            for c in 0..3 {
                let w8 = (want[c] * 255.0).round() as i32;
                worst = worst.max((w8 - o[c] as i32).abs());
            }
            lit += (o[1] > 8) as usize;
        }
        // The dither moves a pixel by at most one 8-bit step; f16 (the frame and
        // the table) and the sRGB round trip by about one more.
        assert!(worst <= 2, "{hour}: the cinematic post differs from the CPU display by {worst} steps");
        assert!(lit > (W * H / 4) as usize, "{hour}: the frame is not black");
    }
}

#[test]
fn the_cinematic_post_is_deterministic_and_bakes_its_table_once_for_a_held_hour() {
    let ctx = ctx();
    let cam = Camera::new(Vec3::new(0.0, 2.0, 0.0), -1.6, 0.05, W as f32 / H as f32);
    let run = || {
        let r = cinematic(&ctx);
        r.look.set_time_of_day(19.25);
        let (_, out) = render(&ctx, &r, &cam, 4);
        (out, r.look.cine().map(|c| c.uploads()).unwrap_or(0))
    };
    let (a, ua) = run();
    let (b, ub) = run();
    assert!(a == b, "two runs of the same frames differ");
    assert_eq!((ua, ub), (1, 1), "the table is baked once for a held hour, not every frame");
}

#[test]
fn the_look_off_draws_the_default_frame_with_the_look_compiled_in() {
    let ctx = ctx();
    let (v, i, o) = tiny_scene();
    let cam = Camera::new(Vec3::new(0.0, 2.0, 0.0), 2.0, 0.25, W as f32 / H as f32);
    // the default renderer (every feature compiled in, as the renderer's tests run)...
    let r0 = Renderer::from_parts(&ctx, &v, &i, &o, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
    r0.look.set_time_of_day(15.0);
    let (_, a) = render(&ctx, &r0, &cam, 2);
    // ... and one that had the look on and turned it off again: the old post pass, bit for bit.
    let mut r1 = cinematic(&ctx);
    let opts = LookOptions { look: LookProfile::Default, sky: eo_render::sky::SkyOptions { cinematic: false, ..r1.look.options.sky }, ..r1.look.options };
    let opts = LookOptions { sky: eo_render::sky::SkyOptions { bloom: r0.look.options.sky.bloom, ..opts.sky }, ..opts };
    r1.look.set_options(&ctx.device, opts);
    r1.look.set_time_of_day(15.0);
    let (_, b) = render(&ctx, &r1, &cam, 2);
    assert!(a == b, "the look off is not the default frame");
    // and the cinematic frame is another one.
    let r2 = cinematic(&ctx);
    r2.look.set_time_of_day(15.0);
    let (_, c) = render(&ctx, &r2, &cam, 2);
    assert!(a != c);
}
