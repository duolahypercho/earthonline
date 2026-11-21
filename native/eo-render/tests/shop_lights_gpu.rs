//! Lit shop windows on the GPU (`--shop-light`, stage N1; sky_lights.wgsl
//! `lights_shops`; needs a GPU): an open shop's light is gathered into the
//! frame list as a shop light (kind 4) at its candela on the lamps' scale, a
//! closed one only as its security light or not at all, none beyond the
//! level's reach; every light cluster takes the street lamps before the shop
//! lights (with the lamp shadows' stable order too), so a crowded cluster
//! drops shops, never a lamp; the sidewalk in front of an open shop's window
//! is lit and the ground behind its glass is not; with the option off
//! nothing is gathered.

use eo_render::camera::Camera;
use eo_render::look::{LightingOptions, LookOptions};
use eo_render::scene::{GpuObject, ObjectKind, Vertex};
use eo_render::shop_lights::{GpuShopLight, SHOP_LIGHT_KIND};
use eo_render::sky::{GpuLight, SkyOptions, WeatherMode};
use eo_render::{GpuContext, GpuOptions, Renderer, Targets, read_mapped, wgpu};
use eo_sky::lights::{LampKind, LampLight};
use glam::Vec3;

const W: u32 = 320;
const H: u32 = 180;

fn ctx() -> GpuContext {
    let opts = GpuOptions::default();
    GpuContext::new(GpuContext::create_instance(&opts), None, opts).expect("a GPU adapter is required")
}

fn slab(verts: &mut Vec<Vertex>, idx: &mut Vec<u32>, objects: &mut Vec<GpuObject>) {
    let (min, max) = (Vec3::new(-300.0, -1.0, -300.0), Vec3::new(300.0, 0.0, 300.0));
    let base = verts.len() as i32;
    let first = idx.len() as u32;
    for i in 0..8 {
        let p = Vec3::new(if i & 1 != 0 { max.x } else { min.x }, if i & 2 != 0 { max.y } else { min.y }, if i & 4 != 0 { max.z } else { min.z });
        verts.push(Vertex { pos: p.to_array(), normal: 0, color: 0xff80_8080 });
    }
    for f in [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]] {
        idx.extend_from_slice(&[f[0], f[1], f[2], f[0], f[2], f[3]]);
    }
    objects.push(GpuObject::new(ObjectKind::Building, min.to_array(), max.to_array(), first, 36, base));
}

/// A ground slab at `hour`, clear, the shop lights at `level`, the lamp shadows' `lighting`
/// (no bloom: its glow would spread a lit patch onto the points beside it).
fn renderer(ctx: &GpuContext, level: u8, lighting: LightingOptions, hour: f64) -> Renderer {
    let (mut v, mut i, mut o) = (vec![], vec![], vec![]);
    slab(&mut v, &mut i, &mut o);
    let mut r = Renderer::from_parts(ctx, &v, &i, &o, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
    let opts = LookOptions { ao: false, lighting, ..r.look.options };
    r.look.set_options(&ctx.device, opts);
    r.look.set_time_of_day(hour);
    r.look.set_sky_options(SkyOptions { weather: WeatherMode::Preset(eo_sky::Kind::Clear), taa: false, rain: false, wet: false, ssr: false, bloom: false, shop_light: level, ..SkyOptions::default() });
    r
}

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

/// A shop window's light at `pos` facing `dir` (plan), open `open`..`open + span` h.
fn shop(pos: Vec3, dir: Vec3, open: f32, span: f32, security: f32) -> GpuShopLight {
    let d = Vec3::new(dir.x, -0.18, dir.z).normalize();
    GpuShopLight { pos: pos.to_array(), range: 12.0, colour: [1.2, 0.95, 0.75], candela: 300.0, dir: d.to_array(), area: 4.0, open, span, security, kind: 0 }
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

fn draw(ctx: &GpuContext, r: &Renderer, f: &Frame, cam: &Camera, frames: usize) {
    for _ in 0..frames {
        r.update(&ctx.queue, cam);
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        r.encode(&mut enc, &f.view, None, &f.targets.depth, None);
        ctx.queue.submit([enc.finish()]);
    }
}

fn luma(ctx: &GpuContext, f: &Frame) -> Vec<f32> {
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
    let mut out = vec![];
    for y in 0..H as usize {
        for x in 0..W as usize {
            let o = y * row as usize + x * 4;
            out.push(0.2126 * raw[o] as f32 + 0.7152 * raw[o + 1] as f32 + 0.0722 * raw[o + 2] as f32);
        }
    }
    out
}

fn project(cam: &Camera, p: Vec3) -> usize {
    let c = cam.view_proj() * p.extend(1.0);
    let (x, y) = ((c.x / c.w * 0.5 + 0.5) * W as f32, (0.5 - c.y / c.w * 0.5) * H as f32);
    assert!(c.w > 0.0 && (0.0..W as f32).contains(&x) && (0.0..H as f32).contains(&y), "{p} is off the frame");
    (y as u32 * W + x as u32) as usize
}

fn shops_in(list: &[GpuLight]) -> Vec<&GpuLight> {
    list.iter().filter(|l| l.kind == SHOP_LIGHT_KIND).collect()
}

/// The gather: open shops in, closed ones by their security light or not at
/// all, past the reach none; nothing with the option off.
#[test]
fn open_shops_are_gathered_and_closed_ones_by_their_security_light() {
    let ctx = ctx();
    let f = frame(&ctx);
    let cam = Camera::new(Vec3::new(0.0, 1.7, 0.0), 0.0, -0.1, W as f32 / H as f32);
    let n = -Vec3::Z;
    // four shops 20 m ahead (facing the eye, +z): open 10-22 h, closed (17-19 h, dark), closed with a
    // security light, round the clock; one open shop 200 m ahead (past level 1's 160 m, inside level 2's 300 m).
    let list = [
        shop(Vec3::new(-6.0, 1.8, -20.0), -n, 10.0, 12.0, 0.0),
        shop(Vec3::new(-2.0, 1.8, -20.0), -n, 17.0, 2.0, 0.0),
        shop(Vec3::new(2.0, 1.8, -20.0), -n, 9.0, 10.0, 0.08),
        shop(Vec3::new(6.0, 1.8, -20.0), -n, 0.0, 24.0, 0.0),
        shop(Vec3::new(0.0, 1.8, -200.0), -n, 10.0, 12.0, 0.0),
    ];
    for (level, want) in [(1u8, 3usize), (2, 4)] {
        let r = renderer(&ctx, level, LightingOptions::OFF, 21.0);
        r.look.sky.set_shop_lights(&ctx.queue, &list);
        assert_eq!(r.look.sky.shop_light_count(), 5);
        draw(&ctx, &r, &f, &cam, 2);
        let (lights, _) = r.look.sky.lights_readback(&ctx).unwrap();
        let s = shops_in(&lights);
        println!("level {level} at 21 h: {} shop lights gathered of 5: {:?}", s.len(), s.iter().map(|l| (l.pos[0], l.pos[2], l.intensity)).collect::<Vec<_>>());
        assert_eq!(s.len(), want, "level {level}");
        let at = |x: f32| s.iter().find(|l| (l.pos[0] - x).abs() < 0.01 && l.pos[2] > -100.0).map(|l| l.intensity);
        let (open, sec, always) = (at(-6.0).expect("the open shop"), at(2.0).expect("the security light"), at(6.0).expect("the all-night shop"));
        assert!(at(-2.0).is_none(), "the closed shop is dark");
        assert!((sec / open - 0.08).abs() < 1e-3 && (always - open).abs() < 1e-6 * open, "security {sec} vs open {open}, all night {always}");
        assert!(s.iter().all(|l| l.size == 4.0 && l.cos_outer == 0.0 && l.cos_inner == 1.0), "the window's area and lobe");
        assert!(open > 0.0);
        // by day nothing is gathered (the rooms' dusk is 0).
        r.look.set_time_of_day(13.0);
        draw(&ctx, &r, &f, &cam, 2);
        let (day, _) = r.look.sky.lights_readback(&ctx).unwrap();
        assert!(shops_in(&day).is_empty(), "by day no shop light");
    }
    // the option off: the lights handed over but none gathered.
    let r = renderer(&ctx, 0, LightingOptions::OFF, 21.0);
    r.look.sky.set_shop_lights(&ctx.queue, &list);
    draw(&ctx, &r, &f, &cam, 2);
    let (off, _) = r.look.sky.lights_readback(&ctx).unwrap();
    assert!(shops_in(&off).is_empty());
}

/// The ranking: in a cluster crowded past its 32 lights, every street lamp
/// is kept and listed before any shop light, the shops fill what is left,
/// with the gather's own order and with the lamp shadows' stable one.
#[test]
fn a_crowded_cluster_keeps_its_street_lamps_and_drops_shops() {
    let ctx = ctx();
    let f = frame(&ctx);
    let cam = Camera::new(Vec3::new(0.0, 1.7, 0.0), 0.0, -0.05, W as f32 / H as f32);
    // 12 lamps and 60 shops packed round one spot 40 m ahead: their spheres all meet its clusters.
    let lamps: Vec<LampLight> = (0..12).map(|k| street_lamp(Vec3::new(-3.0 + 0.5 * k as f32, 8.0, -40.0 - 0.3 * k as f32))).collect();
    let shops: Vec<GpuShopLight> = (0..60).map(|k| shop(Vec3::new(-4.0 + 0.13 * k as f32, 1.8, -38.0 - 0.07 * k as f32), Vec3::Z, 0.0, 24.0, 0.0)).collect();
    for lighting in [LightingOptions::OFF, LightingOptions { light_shadows: 1, ..LightingOptions::OFF }] {
        let r = renderer(&ctx, 1, lighting, 21.0);
        r.look.sky.set_lamps(&ctx.queue, &lamps);
        r.look.sky.set_shop_lights(&ctx.queue, &shops);
        draw(&ctx, &r, &f, &cam, 3);
        let (lights, clusters) = r.look.sky.lights_readback(&ctx).unwrap();
        let lamp_n = lights.iter().filter(|l| l.kind == 0).count();
        let shop_n = shops_in(&lights).len();
        assert_eq!((lamp_n, shop_n), (12, 60), "every light in the frame list");
        let (mut crowded, mut checked) = (0, 0);
        for list in &clusters {
            let kinds: Vec<u32> = list.iter().map(|&i| lights[i as usize].kind).collect();
            let lamps_here = kinds.iter().filter(|k| **k == 0).count();
            if lamps_here == 0 {
                continue;
            }
            checked += 1;
            // the shop lights come after every lamp in the list.
            let first_shop = kinds.iter().position(|k| *k == SHOP_LIGHT_KIND).unwrap_or(kinds.len());
            assert!(kinds[first_shop..].iter().all(|k| *k == SHOP_LIGHT_KIND), "a lamp after a shop: {kinds:?}");
            if list.len() == 32 {
                crowded += 1;
                // a full cluster holds all of the lamps whose sphere meets it.
                assert!(lamps_here >= 12 || kinds.iter().filter(|k| **k == SHOP_LIGHT_KIND).count() == 0, "a lamp dropped for a shop: {kinds:?}");
            }
        }
        println!("light shadows {}: {checked} clusters hold lamps, {crowded} of them full (32)", lighting.light_shadows);
        assert!(checked > 0 && crowded > 0, "{checked} clusters with lamps, {crowded} full");
    }
}

/// The spill: in front of an open shop's window the ground is lit, behind
/// its glass (inside the shop) not; closed, no spill.
#[test]
fn an_open_shop_lights_the_sidewalk_in_front_of_its_window() {
    let ctx = ctx();
    let f = frame(&ctx);
    // looking down at the spot from 6 m up and 8 m back (+x side), the window facing +z.
    let (eye, at) = (Vec3::new(8.0, 6.0, 4.0), Vec3::ZERO);
    let look = (at - eye).normalize();
    let cam = Camera::new(eye, look.x.atan2(-look.z), look.y.asin(), W as f32 / H as f32);
    let window = shop(Vec3::new(0.0, 1.8, 0.0), Vec3::Z, 10.0, 12.0, 0.0);
    let r = renderer(&ctx, 1, LightingOptions::OFF, 21.0);
    let lum = |r: &Renderer, list: &[GpuShopLight]| {
        r.look.sky.set_shop_lights(&ctx.queue, list);
        draw(&ctx, r, &f, &cam, 4);
        luma(&ctx, &f)
    };
    let none = lum(&r, &[]);
    let open = lum(&r, &[window]);
    let front = project(&cam, Vec3::new(0.0, 0.0, 2.5));
    let behind = project(&cam, Vec3::new(0.0, 0.0, -2.5));
    println!("ground 2.5 m in front of the window: {:.1} -> {:.1}; 2.5 m behind it: {:.1} -> {:.1}", none[front], open[front], none[behind], open[behind]);
    assert!(open[front] > none[front] + 20.0, "the sidewalk in front is lit: {} vs {}", open[front], none[front]);
    assert!((open[behind] - none[behind]).abs() < 3.0, "nothing behind the glass: {} vs {}", open[behind], none[behind]);
    // after closing (23:00 for a 10-22 h shop): no spill.
    r.look.set_time_of_day(23.0);
    let closed = lum(&r, &[window]);
    let dark = lum(&r, &[]);
    assert!((closed[front] - dark[front]).abs() < 3.0, "a closed shop throws no light: {} vs {}", closed[front], dark[front]);
}
