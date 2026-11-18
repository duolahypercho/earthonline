//! the sea on the GPU. A planted scene: a seabed slab, a land block
//! standing out of the sea along x < -100 m, and the flat sea quad.
//!
//! - `fft_matches_the_cpu_sum`: the two-dispatch transform (water_fft.wgsl)
//!   writes, per cascade, the height eo-water's CPU sum over the same
//!   spectrum gives at the layer's texels (half-float precision). waves: the
//!   spectrum is the sea state's, built on the GPU (water_spectrum.wgsl); the
//!   CPU sum runs over eo-water's twin of it (`waves::build`), and the GPU's
//!   h0 matches that twin.
//! - `whitecaps_form_on_the_gpu` (waves): after a capture's warm-up the FFT's
//!   foam layers cover about the fraction the plan aims for in the
//!   afternoon westerly (within a factor of three), and none on a calm morning.
//! - `water_draws_where_the_field_says_water`: with the water pass on, the
//!   flat sea quad stands down, patches over the water are drawn, the water
//!   changes the frame over the sea and leaves the land block alone; a view
//!   of the sky draws no patch; `--no-water` (off) draws the quad as before.

use eo_render::camera::Camera;
use eo_render::scene::{GpuObject, ObjectKind, Vertex};
use eo_render::{GpuContext, GpuOptions, Renderer, Targets, read_mapped, wgpu};
use eo_water::field::{ShoreField, to_half_three};
use glam::Vec3;

const W: u32 = 320;
const H: u32 = 180;

fn ctx() -> GpuContext {
    let opts = GpuOptions::default();
    GpuContext::new(GpuContext::create_instance(&opts), None, opts).expect("a GPU adapter is required")
}

fn boxed(verts: &mut Vec<Vertex>, idx: &mut Vec<u32>, objects: &mut Vec<GpuObject>, kind: ObjectKind, min: Vec3, max: Vec3, color: u32) {
    let base = verts.len() as i32;
    let first = idx.len() as u32;
    for i in 0..8 {
        let p = Vec3::new(if i & 1 != 0 { max.x } else { min.x }, if i & 2 != 0 { max.y } else { min.y }, if i & 4 != 0 { max.z } else { min.z });
        verts.push(Vertex { pos: p.to_array(), normal: 0, color });
    }
    for f in [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]] {
        idx.extend_from_slice(&[f[0], f[1], f[2], f[0], f[2], f[3]]);
    }
    objects.push(GpuObject::new(kind, min.to_array(), max.to_array(), first, 36, base));
}

/// Water east of x = -100 m: signed distance x + 100, capped.
fn test_field() -> ShoreField {
    let (w, h) = (65usize, 65usize);
    let (origin, size) = ([-512.0, -512.0], [1024.0, 1024.0]);
    let mut half = vec![0u16; w * h * 2];
    for r in 0..h {
        for c in 0..w {
            let x = origin[0] + size[0] * c as f64 / (w - 1) as f64;
            half[(r * w + c) * 2] = to_half_three((x + 100.0).clamp(-400.0, 400.0));
            half[(r * w + c) * 2 + 1] = to_half_three(0.0);
        }
    }
    ShoreField { origin, size, width: w, height: h, half, range: [-400.0, 400.0], beach_texels: 0, bake_ms: 0.0 }
}

fn setup(ctx: &GpuContext) -> (Renderer, eo_water::WaterData) {
    let (mut v, mut i, mut o) = (vec![], vec![], vec![]);
    let sand = 0xff60_7080;
    boxed(&mut v, &mut i, &mut o, ObjectKind::Terrain, Vec3::new(-400.0, -8.0, -400.0), Vec3::new(400.0, -6.0, 400.0), sand);
    boxed(&mut v, &mut i, &mut o, ObjectKind::Building, Vec3::new(-400.0, -6.0, -400.0), Vec3::new(-100.0, 3.0, 400.0), 0xff40_8040);
    // The flat sea quad, as scene.rs builds it.
    let base = v.len() as i32;
    let first = i.len() as u32;
    let sea = -0.55f32;
    for (x, z) in [(-400.0f32, -400.0f32), (400.0, -400.0), (-400.0, 400.0), (400.0, 400.0)] {
        v.push(Vertex { pos: [x, sea, z], normal: 0, color: 0xff60_4a2e });
    }
    i.extend_from_slice(&[0, 2, 1, 1, 2, 3]);
    o.push(GpuObject::new(ObjectKind::Sea, [-400.0, sea, -400.0], [400.0, sea, 400.0], first, 6, base));
    let mut r = Renderer::from_parts(ctx, &v, &i, &o, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
    // No screen-space occlusion pass: it reads the HDR the water changes.
    r.look.set_options(&ctx.device, eo_render::look::LookOptions { ao: false, ..r.look.options });
    let data = eo_water::WaterData { field: test_field(), field_from_cache: false, ms: 0.0 };
    r.attach_water(ctx, &eo_render::water::WaterInit { data: &data, extent: [-400.0, -400.0, 400.0, 400.0] });
    (r, data)
}

fn frame(ctx: &GpuContext, r: &Renderer, cam: &Camera, frames: usize) -> (Vec<u8>, [u32; 4]) {
    let color = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("water-test"),
        size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = color.create_view(&Default::default());
    let targets = Targets::new(&ctx.device, W, H, wgpu::TextureFormat::Rgba8UnormSrgb, 1);
    let row = (W * 4).div_ceil(256) * 256;
    let buf = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: row as u64 * H as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let counts = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 16, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    for k in 0..frames {
        r.update(&ctx.queue, cam);
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        r.encode(&mut enc, &view, None, &targets.depth, None);
        if k + 1 == frames {
            enc.copy_texture_to_buffer(
                color.as_image_copy(),
                wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(H) } },
                wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
            );
            if let Some(w) = r.water.get().filter(|_| r.water_active()) {
                enc.copy_buffer_to_buffer(w.counter_buffer(), 0, &counts, 0, 16);
            }
        }
        ctx.queue.submit([enc.finish()]);
        ctx.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    }
    let bytes = read_mapped(ctx, &buf).unwrap();
    let mut rgba = vec![];
    for y in 0..H as usize {
        rgba.extend_from_slice(&bytes[y * row as usize..y * row as usize + W as usize * 4]);
    }
    let c: [u32; 4] = if r.water_active() { bytemuck::pod_read_unaligned(&read_mapped(ctx, &counts).unwrap()[..16]) } else { [0; 4] };
    (rgba, c)
}

#[test]
fn fft_matches_the_cpu_sum() {
    let ctx = ctx();
    let (r, data) = setup(&ctx);
    let w = r.water.get().expect("water attached");
    w.set_always(true);
    // a fixed sea (the clear afternoon: swell, the Pacific's and the bay's wind seas).
    let state = eo_water::waves::sea_state(16.0, [0.0, 0.0], 0.0);
    w.set_sea(Some(eo_render::water::SeaInput::Fixed(state)));
    let t = 3.5;
    w.set_time(t);
    let cam = Camera::new(Vec3::new(0.0, 30.0, 200.0), 0.0, -0.4, W as f32 / H as f32);
    frame(&ctx, &r, &cam, 1);
    // The displacement layers: (dx, height, dz, foam) as half floats.
    let n = eo_water::spectrum::N as u32;
    let tex = w.displacement_texture();
    let row = n * 8;
    let buf = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (row * n * 4) as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let mut enc = ctx.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        tex.as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(n) } },
        wgpu::Extent3d { width: n, height: n, depth_or_array_layers: 4 },
    );
    ctx.queue.submit([enc.finish()]);
    let b = read_mapped(&ctx, &buf).unwrap();
    let height = |c: u32, x: u32, z: u32| {
        let at = ((c * n * n + z * n + x) * 8 + 2) as usize;
        eo_water::field::half_to_f32(u16::from_le_bytes([b[at], b[at + 1]])) as f64
    };
    let _ = &data;
    let plan = w.plan().expect("waves: a plan");
    let sp = &eo_water::waves::build(&plan, &eo_water::waves::gaussians());
    // the GPU's h0 is the CPU twin's (f32 against f64).
    {
        let h0b = w.h0_buffer();
        let rb = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: h0b.size(), usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(h0b, 0, &rb, 0, h0b.size());
        ctx.queue.submit([enc.finish()]);
        let raw = read_mapped(&ctx, &rb).unwrap();
        let gpu: Vec<[f32; 4]> = bytemuck::pod_collect_to_vec(&raw[..sp.h0.len() * 16]);
        let peak = sp.h0.iter().flat_map(|t| t.iter()).fold(0f32, |m, v| m.max(v.abs())) as f64;
        let mut worst_h0 = 0f64;
        for (g, c) in gpu.iter().zip(sp.h0.iter()) {
            for k in 0..4 {
                let e = (g[k] as f64 - c[k] as f64).abs() / (c[k].abs() as f64 * 1e-3 + peak * 1e-6);
                worst_h0 = worst_h0.max(e);
            }
        }
        eprintln!("waves: GPU h0 against the CPU twin, worst error over (1e-3 |h0| + 1e-6 peak): {worst_h0:.3}");
        assert!(worst_h0 <= 1.0, "the GPU's spectrum is not the CPU twin's ({worst_h0})");
    }
    let mut worst = [0f64; 4];
    let mut scale = [0f64; 4];
    for c in 0..4u32 {
        let size = sp.sizes[c as usize];
        for k in 0..24u32 {
            let (x, z) = ((k * 37 + 5) % n, (k * 91 + 11) % n);
            let cpu = sp.cascade_height_at(c as usize, x as f64 * size / n as f64, z as f64 * size / n as f64, t);
            let gpu = height(c, x, z);
            worst[c as usize] = worst[c as usize].max((cpu - gpu).abs());
            scale[c as usize] = scale[c as usize].max(cpu.abs());
        }
    }
    eprintln!("fft: worst |GPU - CPU| per cascade {worst:?} (largest |h| {scale:?})");
    for c in 0..4 {
        // f16 keeps 11 bits: 1e-3 of the field's scale, plus f32 summation.
        assert!(worst[c] <= 2e-3 * scale[c].max(0.01) + 2e-4, "cascade {c}: GPU height off the CPU sum by {} m", worst[c]);
    }
}

/// the fraction of cascade c's texels whose foam (z Pacific, w bay) is over 0.3.
fn foam_cover(ctx: &GpuContext, tex: &wgpu::Texture, c: u32, ch: usize) -> f64 {
    let n = eo_water::spectrum::N as u32;
    let row = n * 8;
    let buf = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (row * n) as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let mut enc = ctx.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d { x: 0, y: 0, z: c }, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(n) } },
        wgpu::Extent3d { width: n, height: n, depth_or_array_layers: 1 },
    );
    ctx.queue.submit([enc.finish()]);
    let b = read_mapped(ctx, &buf).unwrap();
    let mut on = 0usize;
    for t in 0..(n * n) as usize {
        let at = t * 8 + ch * 2;
        if eo_water::field::half_to_f32(u16::from_le_bytes([b[at], b[at + 1]])) > 0.3 {
            on += 1;
        }
    }
    on as f64 / (n * n) as f64
}

#[test]
fn whitecaps_form_on_the_gpu() {
    let ctx = ctx();
    let (r, _data) = setup(&ctx);
    let w = r.water.get().expect("water attached");
    w.set_always(true);
    w.set_time(20.0);
    let cam = Camera::new(Vec3::new(0.0, 30.0, 200.0), 0.0, -0.4, W as f32 / H as f32);
    let mut got = vec![];
    for (name, hour) in [("afternoon", 16.0), ("morning", 7.5)] {
        w.set_sea(Some(eo_render::water::SeaInput::Fixed(eo_water::waves::sea_state(hour, [0.0, 0.0], 0.0))));
        frame(&ctx, &r, &cam, 2);
        let plan = w.plan().unwrap();
        let tex = w.derivative_texture();
        let per: Vec<f64> = (0..3).map(|c| foam_cover(&ctx, &tex, c, 3)).collect();
        let cover = 1.0 - per.iter().map(|v| 1.0 - v).product::<f64>();
        eprintln!("waves: GPU bay foam, {name}: cover {cover:.4} (aim {:.4}), per cascade {per:.4?}; thresholds {:.3?}", plan.cover[1], plan.thr[1]);
        got.push((cover, plan.cover[1]));
    }
    assert!(got[0].0 > got[0].1 / 3.0 && got[0].0 < got[0].1 * 3.0, "afternoon foam cover {:?}", got[0]);
    assert!(got[1].0 < 1e-3, "foam on a calm morning {:?}", got[1]);
}

#[test]
fn water_draws_where_the_field_says_water() {
    let ctx = ctx();
    let (r, _data) = setup(&ctx);
    let w = r.water.get().expect("water attached");
    w.set_always(true);
    w.set_time(2.0);
    let cam = Camera::new(Vec3::new(0.0, 30.0, 200.0), 0.0, -0.3, W as f32 / H as f32);
    let (on, counts) = frame(&ctx, &r, &cam, 3);
    w.set_on(false);
    let (off, _) = frame(&ctx, &r, &cam, 3);
    w.set_on(true);
    eprintln!("water: patches drawn {}, in the frustum {}, land {}, occluded {}", counts[0], counts[1], counts[2], counts[3]);
    assert!(counts[0] > 0, "no water patch drawn over the sea");
    assert!(counts[2] > 0, "no patch culled as land");
    // Project two world points: one over the sea, one on the land block's top.
    let vp = cam.view_proj();
    let px = |p: Vec3| {
        let c = vp * p.extend(1.0);
        let (x, y) = (c.x / c.w * 0.5 + 0.5, 0.5 - c.y / c.w * 0.5);
        ((x * W as f32) as usize, (y * H as f32) as usize)
    };
    let at = |img: &Vec<u8>, (x, y): (usize, usize)| -> [i32; 3] {
        let i = (y * W as usize + x) * 4;
        [img[i] as i32, img[i + 1] as i32, img[i + 2] as i32]
    };
    let sea_px = px(Vec3::new(80.0, -0.55, 60.0));
    let land_px = px(Vec3::new(-160.0, 3.0, 60.0));
    let (s_on, s_off) = (at(&on, sea_px), at(&off, sea_px));
    let (l_on, l_off) = (at(&on, land_px), at(&off, land_px));
    eprintln!("sea pixel {sea_px:?}: water {s_on:?}, flat sea {s_off:?}; land pixel {land_px:?}: {l_on:?} / {l_off:?}");
    let diff = |a: [i32; 3], b: [i32; 3]| (0..3).map(|k| (a[k] - b[k]).abs()).max().unwrap();
    assert!(diff(s_on, s_off) > 4, "the water pass did not change the sea");
    assert!(diff(l_on, l_off) <= 3, "the water pass changed the land block");
    // Looking at the sky: no patch in the frustum.
    let up = Camera::new(Vec3::new(0.0, 30.0, 200.0), 0.0, 1.2, W as f32 / H as f32);
    let (_, c_up) = frame(&ctx, &r, &up, 2);
    assert_eq!(c_up[0], 0, "a patch drawn while looking at the sky");
}
