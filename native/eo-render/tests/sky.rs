//! the sky slice on the GPU (needs a GPU): the cloud deck covers the sky
//! the volume pass marches (an overcast deck turns the blue dome grey; clear
//! air leaves it blue), a street lamp lights the ground under it at night
//! through the clustered lights (and nothing by day), a lamp by the water is
//! seen in the sea in front of it (its glitter) and one inland is not, and the temporal
//! resolve converges on a still scene (the last two frames of a still camera
//! within 2/255 of each other).

use eo_render::camera::Camera;
use eo_render::scene::{GpuObject, ObjectKind, Vertex};
use eo_render::sky::{SkyOptions, WeatherMode};
use eo_render::{GpuContext, GpuOptions, Renderer, Targets, read_mapped, wgpu};
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

fn scene() -> (Vec<Vertex>, Vec<u32>, Vec<GpuObject>) {
    let (mut v, mut i, mut o) = (vec![], vec![], vec![]);
    // A ground slab and a block beside it.
    quad_box(Vec3::new(-200.0, -1.0, -200.0), Vec3::new(200.0, 0.0, 200.0), &mut v, &mut i, &mut o);
    quad_box(Vec3::new(12.0, 0.0, -40.0), Vec3::new(30.0, 25.0, -20.0), &mut v, &mut i, &mut o);
    (v, i, o)
}

/// Render `frames` frames, read the output back (sRGB bytes).
fn render(ctx: &GpuContext, r: &Renderer, cam: &Camera, frames: usize) -> Vec<[u8; 4]> {
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
    let row = (W * 4).div_ceil(256) * 256;
    let b = d.create_buffer(&wgpu::BufferDescriptor { label: None, size: (row * H) as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let mut enc = d.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        out.as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &b, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(H) } },
        wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
    );
    ctx.queue.submit([enc.finish()]);
    let raw = read_mapped(ctx, &b).unwrap();
    (0..H as usize).flat_map(|y| (0..W as usize).map(move |x| (y, x))).map(|(y, x)| {
        let o = y * row as usize + x * 4;
        [raw[o], raw[o + 1], raw[o + 2], raw[o + 3]]
    }).collect()
}

fn mean(px: &[[u8; 4]], rows: std::ops::Range<u32>) -> [f64; 3] {
    let mut s = [0.0; 3];
    let mut n = 0.0;
    for y in rows {
        for x in 0..W {
            let p = px[(y * W + x) as usize];
            for c in 0..3 {
                s[c] += p[c] as f64;
            }
            n += 1.0;
        }
    }
    s.map(|v| v / n)
}

#[test]
fn an_overcast_deck_greys_the_sky_and_clear_air_keeps_it_blue() {
    let ctx = ctx();
    let (v, i, o) = scene();
    let r = Renderer::from_parts(&ctx, &v, &i, &o, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
    r.look.set_time_of_day(12.0);
    let cam = Camera::new(Vec3::new(0.0, 2.0, 0.0), 0.0, 0.5, W as f32 / H as f32);
    let mut out = vec![];
    for (name, w) in [("clear", eo_sky::Kind::Clear), ("overcast", eo_sky::Kind::Overcast)] {
        r.look.set_sky_options(SkyOptions { weather: WeatherMode::Preset(w), taa: false, ..SkyOptions::default() });
        let px = render(&ctx, &r, &cam, 4);
        let m = mean(&px, 0..H / 3);
        println!("{name}: upper sky mean sRGB {:?} (B/R {:.2})", m.map(|x| x.round()), m[2] / m[0].max(1.0));
        out.push(m);
    }
    assert!(out[0][2] / out[0][0] > 1.15, "the clear sky is not blue");
    assert!(out[1][2] / out[1][0] < out[0][2] / out[0][0] - 0.1, "the overcast deck did not grey the sky");
}

#[test]
fn a_street_lamp_lights_the_ground_at_night_only() {
    let ctx = ctx();
    let (v, i, o) = scene();
    let r = Renderer::from_parts(&ctx, &v, &i, &o, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
    let lamp = eo_sky::lights::place_street_lamps(
        &[eo_sky::lights::SegmentIn { highway: "residential", width: 8.0, sidewalk_left: 3.0, sidewalk_right: 0.0, points: &[[-10.0, -12.0], [10.0, -12.0]] }],
        |_, _| 0.0,
    );
    assert_eq!(lamp.len(), 1);
    // Looking down at the ground round the lamp's foot.
    let foot = Vec3::new(lamp[0].pos[0] as f32, 0.0, lamp[0].pos[2] as f32);
    let cam = Camera::new(foot + Vec3::new(0.0, 6.0, 12.0), 0.0, -0.45, W as f32 / H as f32);
    let mut lum = vec![];
    for hour in [21.0, 12.0] {
        r.look.set_time_of_day(hour);
        let mut res = vec![];
        for lights in [false, true] {
            r.look.set_sky_options(SkyOptions { lights, taa: false, clouds: false, fog: false, shafts: false, bloom: false, ..SkyOptions::default() });
            r.look.sky.set_lamps(&ctx.queue, if lights { &lamp } else { &[] });
            let px = render(&ctx, &r, &cam, 3);
            let m = mean(&px, H / 3..H);
            res.push(0.2126 * m[0] + 0.7152 * m[1] + 0.0722 * m[2]);
        }
        println!("{hour} h: ground luma without the lamp {:.1}, with it {:.1}", res[0], res[1]);
        lum.push(res);
    }
    assert!(lum[0][1] > lum[0][0] + 8.0, "the lamp does not light the ground at night");
    assert!((lum[1][1] - lum[1][0]).abs() < 1.0, "the lamp lights the ground by day");
}

#[test]
fn a_waterside_lamp_glitters_on_the_sea_and_an_inland_one_does_not() {
    let ctx = ctx();
    // The seabed half a metre under the sea, and a quay 80 m out with a lamp on it.
    let sea = eo_water::SEA_LEVEL_Y as f32;
    let (mut v, mut i, mut o) = (vec![], vec![], vec![]);
    quad_box(Vec3::new(-200.0, sea - 2.0, -60.0), Vec3::new(200.0, sea - 0.5, 200.0), &mut v, &mut i, &mut o);
    quad_box(Vec3::new(-200.0, sea - 2.0, -200.0), Vec3::new(200.0, 1.0, -60.0), &mut v, &mut i, &mut o);
    let r = Renderer::from_parts(&ctx, &v, &i, &o, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
    let lamp = eo_sky::lights::place_street_lamps(
        &[eo_sky::lights::SegmentIn { highway: "residential", width: 8.0, sidewalk_left: 3.0, sidewalk_right: 0.0, points: &[[-10.0, -76.0], [10.0, -76.0]] }],
        |_, _| 1.0,
    );
    assert_eq!(lamp.len(), 1);
    // From a boat 3 m over the sea, looking at the quay.
    let cam = Camera::new(Vec3::new(lamp[0].pos[0] as f32, sea + 3.0, 0.0), 0.0, -0.08, W as f32 / H as f32);
    r.look.set_time_of_day(21.0);
    r.look.set_sky_options(SkyOptions { taa: false, clouds: false, fog: false, shafts: false, bloom: false, ..SkyOptions::default() });
    let mut res = vec![];
    for by_water in [false, true] {
        r.look.sky.set_lamps(&ctx.queue, &lamp);
        // One cell over the whole scene, by the water or not.
        r.look.sky.set_waterside(&ctx.queue, [-400.0, -400.0], 800.0, 1, 1, &[by_water]);
        let px = render(&ctx, &r, &cam, 3);
        let m = mean(&px, H / 2..H);
        res.push(0.2126 * m[0] + 0.7152 * m[1] + 0.0722 * m[2]);
    }
    println!("sea luma under the quay lamp: inland {:.2}, by the water {:.2}", res[0], res[1]);
    assert!(res[1] > res[0] + 1.0, "the waterside lamp is not seen in the sea");
}

#[test]
fn the_temporal_resolve_converges_on_a_still_scene() {
    let ctx = ctx();
    let (v, i, o) = scene();
    let r = Renderer::from_parts(&ctx, &v, &i, &o, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
    r.look.set_time_of_day(15.0);
    r.look.set_sky_options(SkyOptions { clouds: false, fog: false, shafts: false, ..SkyOptions::default() });
    let cam = Camera::new(Vec3::new(0.0, 2.0, 10.0), 0.3, -0.05, W as f32 / H as f32);
    let a = render(&ctx, &r, &cam, 24);
    let b = render(&ctx, &r, &cam, 1);
    let diff: f64 = a.iter().zip(&b).map(|(p, q)| (0..3).map(|c| (p[c] as f64 - q[c] as f64).abs()).sum::<f64>() / 3.0).sum::<f64>() / a.len() as f64;
    println!("mean abs difference between consecutive converged frames: {diff:.3} / 255");
    assert!(diff < 2.0, "the resolve does not converge ({diff:.2} / 255)");
}
