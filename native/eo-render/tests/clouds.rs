//! the cloud noise bake on the GPU (needs a GPU). The base-shape and
//! detail volumes and the curl field fill their range, tile seamlessly on every
//! axis (the step across the wrap is no larger than a step inside), and carry no
//! layering along an axis: the mean step between neighbouring texels is the same along
//! every axis (every lattice of the bake has the same period on all three axes, so the
//! noise is isotropic; slices made apart from each other step far more along one axis).
//! `EO_CLOUD_NOISE_DUMP=<dir>` writes slices of each as PNG for a look.

use eo_render::clouds::{BASE_RES, CURL_RES, CloudNoise, DETAIL_RES, VOL_DETAIL_RES, VOL_WEATHER_RES, VolCloudNoise};
use eo_render::{GpuContext, GpuOptions, read_mapped, wgpu};

fn ctx() -> GpuContext {
    let opts = GpuOptions::default();
    GpuContext::new(GpuContext::create_instance(&opts), None, opts).expect("a GPU adapter is required")
}

/// Level 0 of a texture as RGBA8, tightly packed (x fastest, then y, then z).
fn read(ctx: &GpuContext, tex: &wgpu::Texture, res: u32, depth: u32) -> Vec<u8> {
    let row = (res * 4).next_multiple_of(256);
    let buf = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (row * res * depth) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    let mut enc = ctx.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(res) } },
        wgpu::Extent3d { width: res, height: res, depth_or_array_layers: depth },
    );
    ctx.queue.submit([enc.finish()]);
    let raw = read_mapped(ctx, &buf).expect("readback");
    let mut out = Vec::with_capacity((res * res * depth * 4) as usize);
    for z in 0..depth {
        for y in 0..res {
            let o = ((z * res + y) * row) as usize;
            out.extend_from_slice(&raw[o..o + (res * 4) as usize]);
        }
    }
    out
}

fn dump(dir: &str, name: &str, w: u32, h: u32, px: &[u8]) {
    let f = std::fs::File::create(format!("{dir}/{name}.png")).unwrap();
    let mut e = png::Encoder::new(std::io::BufWriter::new(f), w, h);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().unwrap().write_image_data(px).unwrap();
}

/// For channel `c` of a cubic volume: (min, max, mean, the mean step across the wrap over the
/// mean step inside, per axis; the mean step between neighbouring texels, per axis).
fn stats(v: &[u8], res: usize, c: usize) -> (f64, f64, f64, [f64; 3], [f64; 3]) {
    let at = |x: usize, y: usize, z: usize| v[((z * res + y) * res + x) * 4 + c] as f64 / 255.0;
    let (mut lo, mut hi, mut sum) = (1.0f64, 0.0f64, 0.0);
    for i in 0..res * res * res {
        let a = v[i * 4 + c] as f64 / 255.0;
        lo = lo.min(a);
        hi = hi.max(a);
        sum += a;
    }
    let mean = sum / (res * res * res) as f64;
    let mut seam = [0.0; 3];
    let mut layer = [0.0f64; 3];
    for axis in 0..3 {
        let p = |i: usize, j: usize, k: usize| match axis {
            0 => at(k, i, j),
            1 => at(i, k, j),
            _ => at(i, j, k),
        };
        let (mut wrap, mut inside) = (0.0, 0.0);
        for i in 0..res {
            for j in 0..res {
                wrap += (p(i, j, res - 1) - p(i, j, 0)).abs();
                inside += (p(i, j, res / 2) - p(i, j, res / 2 - 1)).abs();
            }
        }
        seam[axis] = wrap / inside.max(1e-9);
        let mut step = 0.0;
        for k in 0..res {
            for i in 0..res {
                for j in 0..res {
                    step += (p(i, j, (k + 1) % res) - p(i, j, k)).abs();
                }
            }
        }
        layer[axis] = step / (res * res * res) as f64;
    }
    (lo, hi, mean, seam, layer)
}

#[test]
fn the_cloud_noise_fills_its_range_tiles_and_has_no_layers() {
    let ctx = ctx();
    let noise = CloudNoise::new(&ctx.device);
    let mut enc = ctx.device.create_command_encoder(&Default::default());
    {
        let mut pass = enc.begin_compute_pass(&Default::default());
        noise.encode(&mut pass);
    }
    ctx.queue.submit([enc.finish()]);
    let base = read(&ctx, &noise.base, BASE_RES, BASE_RES);
    let detail = read(&ctx, &noise.detail, DETAIL_RES, DETAIL_RES);
    let curl = read(&ctx, &noise.curl, CURL_RES, 1);
    if let Ok(dir) = std::env::var("EO_CLOUD_NOISE_DUMP") {
        let n = BASE_RES as usize;
        // an xy slice (z = 0) and an xz slice (y = 0) of the base, and the detail's xy slice
        dump(&dir, "base-xy", BASE_RES, BASE_RES, &base[..n * n * 4]);
        let mut xz = vec![0u8; n * n * 4];
        for z in 0..n {
            for x in 0..n {
                xz[(z * n + x) * 4..(z * n + x) * 4 + 4].copy_from_slice(&base[(z * n * n + x) * 4..(z * n * n + x) * 4 + 4]);
            }
        }
        dump(&dir, "base-xz", BASE_RES, BASE_RES, &xz);
        let d = DETAIL_RES as usize;
        dump(&dir, "detail-xy", DETAIL_RES, DETAIL_RES, &detail[..d * d * 4]);
        dump(&dir, "curl", CURL_RES, CURL_RES, &curl);
    }
    for (name, v, res, chans) in [("base", &base, BASE_RES as usize, 4), ("detail", &detail, DETAIL_RES as usize, 3)] {
        for c in 0..chans {
            let (lo, hi, mean, seam, layer) = stats(v, res, c);
            eprintln!("{name} channel {c}: {lo:.2}..{hi:.2} mean {mean:.2}, wrap step / inner step {seam:.2?}, mean neighbour step by axis {layer:.4?}");
            assert!(lo < 0.2 && hi > 0.75, "{name} channel {c} does not fill its range: {lo}..{hi}");
            assert!(mean > 0.2 && mean < 0.8, "{name} channel {c} mean {mean}");
            for a in 0..3 {
                assert!(seam[a] < 1.6, "{name} channel {c} has a seam across axis {a}: x{:.2}", seam[a]);
            }
            // isotropy: the steps along the three axes agree within 15 % (each is a mean over the
            // whole volume, so chance moves it by a few per cent; a layered axis steps far more)
            let (lmin, lmax) = (layer.iter().cloned().fold(1.0, f64::min), layer.iter().cloned().fold(0.0, f64::max));
            assert!(lmax < 1.15 * lmin, "{name} channel {c} is anisotropic (layered): mean neighbour step by axis {layer:.4?}");
        }
    }
    // the curl field: centred on 0.5 and not flat
    let n = (CURL_RES * CURL_RES) as usize;
    let (mut s, mut s2) = (0.0f64, 0.0f64);
    for i in 0..n {
        let a = curl[i * 4] as f64 / 255.0;
        s += a;
        s2 += a * a;
    }
    let (m, var) = (s / n as f64, s2 / n as f64 - (s / n as f64).powi(2));
    eprintln!("curl x: mean {m:.3}, sd {:.3}", var.sqrt());
    assert!((m - 0.5).abs() < 0.05 && var.sqrt() > 0.05, "curl field: mean {m}, sd {}", var.sqrt());
}

#[test]
fn the_volumetric_cloud_noise_fills_its_range_tiles_and_is_isotropic() {
    let ctx = ctx();
    let vol = VolCloudNoise::new(&ctx.device);
    let mut enc = ctx.device.create_command_encoder(&Default::default());
    {
        let mut pass = enc.begin_compute_pass(&Default::default());
        vol.encode(&mut pass);
    }
    ctx.queue.submit([enc.finish()]);
    let detail = read(&ctx, &vol.detail, VOL_DETAIL_RES, VOL_DETAIL_RES);
    let weather = read(&ctx, &vol.weather, VOL_WEATHER_RES, 1);
    for c in 0..4 {
        let (lo, hi, mean, seam, layer) = stats(&detail, VOL_DETAIL_RES as usize, c);
        eprintln!("vol_detail channel {c}: {lo:.2}..{hi:.2} mean {mean:.2}, wrap/inner {seam:.2?}, axis step {layer:.4?}");
        assert!(lo < 0.2 && hi > 0.8, "vol_detail channel {c} does not fill range: {lo}..{hi}");
        assert!(mean > 0.2 && mean < 0.8, "vol_detail channel {c} mean {mean}");
        for a in 0..3 {
            assert!(seam[a] < 1.6, "vol_detail channel {c} has a seam on axis {a}: x{:.2}", seam[a]);
        }
        let (lmin, lmax) = (layer.iter().cloned().fold(1.0, f64::min), layer.iter().cloned().fold(0.0, f64::max));
        assert!(lmax < 1.15 * lmin, "vol_detail channel {c} is anisotropic: {layer:.4?}");
    }
    let w = VOL_WEATHER_RES as usize;
    let at2 = |x: usize, y: usize, c: usize| weather[(y * w + x) * 4 + c] as f64 / 255.0;
    for c in 0..4 {
        let (mut lo, mut hi, mut sum) = (1.0f64, 0.0f64, 0.0);
        for i in 0..w * w {
            let v = weather[i * 4 + c] as f64 / 255.0;
            lo = lo.min(v);
            hi = hi.max(v);
            sum += v;
        }
        let mean = sum / (w * w) as f64;
        let (mut wrap_x, mut in_x, mut wrap_y, mut in_y) = (0.0, 0.0, 0.0, 0.0);
        for i in 0..w {
            wrap_x += (at2(w - 1, i, c) - at2(0, i, c)).abs();
            wrap_y += (at2(i, w - 1, c) - at2(i, 0, c)).abs();
            for j in 0..(w - 1) {
                in_x += (at2(j + 1, i, c) - at2(j, i, c)).abs();
                in_y += (at2(i, j + 1, c) - at2(i, j, c)).abs();
            }
        }
        let sx = (wrap_x / w as f64) / (in_x / ((w - 1) * w) as f64).max(1e-9);
        let sy = (wrap_y / w as f64) / (in_y / ((w - 1) * w) as f64).max(1e-9);
        eprintln!("vol_weather channel {c}: {lo:.2}..{hi:.2} mean {mean:.2}, wrap/inner [{sx:.2}, {sy:.2}]");
        assert!(lo < 0.25 && hi > 0.75, "vol_weather channel {c} narrow range: {lo}..{hi}");
        assert!(sx < 1.6 && sy < 1.6, "vol_weather channel {c} seam: [{sx:.2}, {sy:.2}]");
    }
}
