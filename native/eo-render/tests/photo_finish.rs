//! The photographic finish's post pass on the GPU (`--photo-finish`, stage
//! W4): with the option the output is eo-look's `tone::photo_display` of the
//! HDR scene (within the noise, the dither and the 8-bit and f16 rounding);
//! the lateral colour leaves the frame's middle alone; turned off again, the
//! old pass is back. Needs a GPU.

use eo_render::camera::Camera;
use eo_render::look::LookOptions;
use eo_render::scene::{GpuObject, ObjectKind, Vertex};
use eo_render::{GpuContext, GpuOptions, Renderer, Targets, read_mapped, wgpu};
use glam::Vec3;

fn ctx() -> GpuContext {
    let opts = GpuOptions::default();
    GpuContext::new(GpuContext::create_instance(&opts), None, opts).expect("a GPU adapter is required")
}

const W: u32 = 320;
const H: u32 = 180;

fn quad_box(min: Vec3, max: Vec3, kind: ObjectKind, verts: &mut Vec<Vertex>, idx: &mut Vec<u32>, objects: &mut Vec<GpuObject>) {
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
    objects.push(GpuObject::new(kind, min.to_array(), max.to_array(), first, 36, base));
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

/// Two frames, then the HDR scene (rgba f16 -> f32) and the output (rgba8 sRGB).
fn render(ctx: &GpuContext, r: &Renderer, cam: &Camera) -> (Vec<[f32; 4]>, Vec<[u8; 4]>) {
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
    for _ in 0..2 {
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
    let (mut hdr_px, mut out_px) = (vec![], vec![]);
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

/// (worst, mean) difference in 8-bit steps between the output and a CPU transform of the HDR pixels.
fn compare(hdr: &[[f32; 4]], out: &[[u8; 4]], want: impl Fn([f64; 3], [f64; 2]) -> [f64; 3]) -> (i32, f64) {
    let (mut worst, mut sum, mut n) = (0i32, 0.0, 0.0);
    for (k, (h, o)) in hdr.iter().zip(out).enumerate() {
        let uv = [((k as u32 % W) as f64 + 0.5) / W as f64, ((k as u32 / W) as f64 + 0.5) / H as f64];
        let w = want([h[0] as f64, h[1] as f64, h[2] as f64], uv);
        for c in 0..3 {
            let d = ((w[c] * 255.0).round() as i32 - o[c] as i32).abs();
            worst = worst.max(d);
            sum += d as f64;
            n += 1.0;
        }
    }
    (worst, sum / n)
}

#[test]
fn the_photo_pass_is_the_cpu_camera_and_off_is_the_old_pass() {
    let ctx = ctx();
    // one box far behind the camera: the frame is all sky (a smooth gradient, so the lateral
    // colour moves nothing by more than rounding).
    let (mut v, mut i, mut o) = (vec![], vec![], vec![]);
    quad_box(Vec3::new(-1.0, 0.0, 5000.0), Vec3::new(1.0, 1.0, 5002.0), ObjectKind::Building, &mut v, &mut i, &mut o);
    let mut r = Renderer::from_parts(&ctx, &v, &i, &o, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
    r.look.set_time_of_day(15.0);
    let cam = Camera::new(Vec3::new(0.0, 2.0, 0.0), 2.0, 0.25, W as f32 / H as f32);
    let ty = (0.5 * cam.fov_y_deg as f64).to_radians().tan();
    let tan_half = [ty * W as f64 / H as f64, ty];

    let opts = LookOptions { photo_finish: 1, ..r.look.options };
    r.look.set_options(&ctx.device, opts);
    let (hdr, out) = render(&ctx, &r, &cam);
    let exposure = r.look.frame().exposure;
    let (worst, mean) = compare(&hdr, &out, |c, uv| eo_look::tone::photo_display(c, exposure, uv, tan_half));
    // the noise at base ISO (a clear afternoon: about half an 8-bit level), the dither (one
    // step), f16 and the sRGB round trip (about one more).
    assert!(worst <= 5 && mean < 0.8, "photo pass vs tone::photo_display: worst {worst} steps, mean {mean:.3}");
    // and the camera is not the old chain: the sky's blue is less saturated, its corners darker.
    let (_, old_mean) = compare(&hdr, &out, |c, uv| eo_look::tone::display(c, exposure, uv));
    assert!(old_mean > mean + 1.0, "the photo pass looks like the old one: mean {old_mean:.3} vs {mean:.3}");

    // off again: the old pass (the same bound as eo-render tests/look.rs).
    let opts = LookOptions { photo_finish: 0, ..r.look.options };
    r.look.set_options(&ctx.device, opts);
    let (hdr, out) = render(&ctx, &r, &cam);
    let (worst, _) = compare(&hdr, &out, |c, uv| eo_look::tone::display(c, exposure, uv));
    assert!(worst <= 2, "off: post differs from the CPU display transform by {worst} steps");
}
