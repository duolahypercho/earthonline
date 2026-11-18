//! The look on the GPU: the WGSL sky equals eo-look's CPU sky (itself tested
//! against the JS), a box casts a sun shadow through the GPU-driven shadow
//! path, and the post pass equals eo-look's display transform. Needs a GPU.

use eo_render::camera::Camera;
use eo_render::scene::{GpuObject, ObjectKind, Vertex};
use eo_render::{GpuContext, GpuOptions, Renderer, Targets, read_mapped, wgpu};
use glam::{Vec3, Vec4};

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

/// Render one frame and read back the HDR scene target (rgba f16 -> f32) and the output (rgba8 sRGB).
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

/// World direction through a pixel centre.
fn pixel_dir(cam: &Camera, x: u32, y: u32) -> [f64; 3] {
    let ndc = Vec4::new((x as f32 + 0.5) / W as f32 * 2.0 - 1.0, 1.0 - (y as f32 + 0.5) / H as f32 * 2.0, 1.0, 1.0);
    let inv = cam.view_proj().inverse();
    let p = inv * ndc;
    let d = (p.truncate() / p.w - cam.eye).normalize();
    [d.x as f64, d.y as f64, d.z as f64]
}

fn tiny_scene() -> (Vec<Vertex>, Vec<u32>, Vec<GpuObject>) {
    let (mut v, mut i, mut o) = (vec![], vec![], vec![]);
    // One box far behind the camera: the frame is all sky.
    quad_box(Vec3::new(-1.0, 0.0, 5000.0), Vec3::new(1.0, 1.0, 5002.0), ObjectKind::Building, &mut v, &mut i, &mut o);
    (v, i, o)
}

#[test]
fn gpu_sky_equals_cpu_sky() {
    let ctx = ctx();
    let (v, i, o) = tiny_scene();
    let r = Renderer::from_parts(&ctx, &v, &i, &o, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
    let mut checked = 0;
    for (hour, yaw, pitch) in [(12.0, 2.7f32, 0.35f32), (9.0, 1.9, 0.1), (18.6, -1.6, 0.05), (19.4, -1.5, 0.3), (19.4, 1.5, 0.2), (22.0, 0.0, 0.6)] {
        r.look.set_time_of_day(hour);
        let cam = Camera::new(Vec3::new(0.0, 2.0, 0.0), yaw, pitch, W as f32 / H as f32);
        let (hdr, _) = render(&ctx, &r, &cam);
        let f = r.look.frame();
        for y in (0..H).step_by(9) {
            for x in (0..W).step_by(11) {
                let d = pixel_dir(&cam, x, y);
                let cos_sun = d[0] * f.model.sun.x + d[1] * f.model.sun.y + d[2] * f.model.sun.z;
                if (cos_sun - f.disc_radius.cos()).abs() < 2e-4 {
                    continue; // the disc's edge
                }
                let mut want = f.model.dome(d);
                if cos_sun > f.disc_radius.cos() {
                    for k in 0..3 {
                        want[k] += f.disc[k];
                    }
                }
                let got = hdr[(y * W + x) as usize];
                for k in 0..3 {
                    let e = (got[k] as f64 - want[k]).abs() / want[k].abs().max(2e-3);
                    assert!(e < 1.5e-2, "hour {hour} pixel ({x},{y}) dir {d:?} channel {k}: GPU {got:?} CPU {want:?}");
                }
                checked += 1;
            }
        }
    }
    assert!(checked > 2000);
}

#[test]
fn gpu_post_equals_cpu_display_transform() {
    let ctx = ctx();
    let (v, i, o) = tiny_scene();
    let r = Renderer::from_parts(&ctx, &v, &i, &o, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
    r.look.set_time_of_day(15.0);
    let cam = Camera::new(Vec3::new(0.0, 2.0, 0.0), 2.0, 0.25, W as f32 / H as f32);
    let (hdr, out) = render(&ctx, &r, &cam);
    let exposure = r.look.frame().exposure;
    let mut worst = 0i32;
    for (k, (h, o)) in hdr.iter().zip(&out).enumerate() {
        let uv = [((k as u32 % W) as f64 + 0.5) / W as f64, ((k as u32 / W) as f64 + 0.5) / H as f64];
        let want = eo_look::tone::display([h[0] as f64, h[1] as f64, h[2] as f64], exposure, uv);
        for c in 0..3 {
            let w8 = (want[c] * 255.0).round() as i32;
            worst = worst.max((w8 - o[c] as i32).abs());
        }
    }
    // The dither moves a pixel by at most one 8-bit step; f16 and the sRGB
    // round trip by about one more.
    assert!(worst <= 2, "post differs from the CPU display transform by {worst} steps");
}

#[test]
fn a_box_casts_a_sun_shadow() {
    let ctx = ctx();
    let (mut v, mut i, mut o) = (vec![], vec![], vec![]);
    // Receiver: a terrain slab (does not cast); caster: a 4 x 10 x 4 m building.
    quad_box(Vec3::new(-60.0, -1.0, -60.0), Vec3::new(60.0, 0.0, 60.0), ObjectKind::Terrain, &mut v, &mut i, &mut o);
    quad_box(Vec3::new(-2.0, 0.0, -2.0), Vec3::new(2.0, 10.0, 2.0), ObjectKind::Building, &mut v, &mut i, &mut o);
    let r = Renderer::from_parts(&ctx, &v, &i, &o, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
    r.look.set_time_of_day(12.0);
    // Looking down from 40 m, a little south of the box.
    let cam = Camera::new(Vec3::new(0.0, 40.0, 25.0), 0.0, -1.0, W as f32 / H as f32);
    let (hdr, _) = render(&ctx, &r, &cam);
    let f = r.look.frame();
    let sun = Vec3::new(f.key_dir[0] as f32, f.key_dir[1] as f32, f.key_dir[2] as f32);
    let away = -Vec3::new(sun.x, 0.0, sun.z).normalize();
    let sample = |p: Vec3| -> f32 {
        let c = cam.view_proj() * p.extend(1.0);
        let x = ((c.x / c.w * 0.5 + 0.5) * W as f32) as u32;
        let y = ((0.5 - c.y / c.w * 0.5) * H as f32) as u32;
        let px = hdr[(y.min(H - 1) * W + x.min(W - 1)) as usize];
        0.2126 * px[0] + 0.7152 * px[1] + 0.0722 * px[2]
    };
    if std::env::var("EO_LOOK_DEBUG").is_ok() {
        for y in (0..H).step_by(6) {
            let row: String = (0..W).step_by(4).map(|x| {
                let p = hdr[(y * W + x) as usize];
                let l = 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
                b" .:-=+*#%@"[((l * 4.0) as usize).min(9)] as char
            }).collect();
            eprintln!("{row}");
        }
        eprintln!("away {away:?}");
    }
    let shadowed = sample(away * 8.0);
    // 8 m out is past the box as the camera sees it (2.5-6 m is hidden behind it).
    let lit = sample(-away * 8.0);
    let lit2 = sample(away * 8.0 + Vec3::new(away.z, 0.0, -away.x) * 9.0);
    assert!(lit > 0.0 && lit2 > 0.0);
    assert!(shadowed < 0.5 * lit && shadowed < 0.5 * lit2, "shadow {shadowed} vs lit {lit} / {lit2}");
    // Shadows off: the same point is lit.
    let mut r2 = r;
    let opts = eo_render::look::LookOptions { shadows: false, ..r2.look.options };
    r2.look.set_options(&ctx.device, opts);
    let (hdr2, _) = render(&ctx, &r2, &cam);
    let c = cam.view_proj() * (away * 8.0).extend(1.0);
    let x = ((c.x / c.w * 0.5 + 0.5) * W as f32) as u32;
    let y = ((0.5 - c.y / c.w * 0.5) * H as f32) as u32;
    let px = hdr2[(y * W + x) as usize];
    let unshadowed = 0.2126 * px[0] + 0.7152 * px[1] + 0.0722 * px[2];
    assert!(unshadowed > 1.5 * shadowed, "without shadows {unshadowed} vs {shadowed}");
}
