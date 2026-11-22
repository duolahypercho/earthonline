//! The ambient volume on the GPU (`LightingOptions::ambient_volume`): a
//! street canyon between two 30 m blocks and open ground far from them, seen
//! from above. With the volume baked, uploaded and resident, the canyon's
//! floor takes less light than without it while the open ground keeps its
//! light; the uploads raise no validation error and the volume settles.
//! And the ground bounce (`LightingOptions::ground_bounce`): a slab over open
//! ground takes light bounced up from the sunlit ground on its underside,
//! more towards its open edge, none on its top; a slab over a closed
//! courtyard takes none (it loses the rig's lobe's sun part, never going
//! black); at night nothing changes.
//! Needs a GPU.

use eo_render::ambient::{AmbientBuildings, AmbientGround, AmbientInputs};
use eo_render::ambient_bake::DeckPiece;
use eo_render::camera::Camera;
use eo_render::look::{LightingOptions, LookOptions};
use eo_render::scene::{GpuObject, ObjectKind, Vertex};
use eo_render::{GpuContext, GpuOptions, Renderer, Targets, read_mapped, wgpu};
use glam::Vec3;

fn ctx() -> GpuContext {
    let opts = GpuOptions::default();
    GpuContext::new(GpuContext::create_instance(&opts), None, opts).expect("a GPU adapter is required")
}

const W: u32 = 384;
const H: u32 = 192;

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

/// The two blocks (min x, max x), each 160 m long and 30 m tall: an 8 m street between them on x = 0.
const BLOCKS: [(f32, f32); 2] = [(-24.0, -4.0), (4.0, 24.0)];
const BLOCK_H: f32 = 30.0;

fn scene(ctx: &GpuContext, ambient: u8) -> Renderer {
    let (mut v, mut i, mut o) = (vec![], vec![], vec![]);
    quad_box(Vec3::new(-400.0, -1.0, -400.0), Vec3::new(400.0, 0.0, 400.0), ObjectKind::Terrain, &mut v, &mut i, &mut o);
    for (x0, x1) in BLOCKS {
        quad_box(Vec3::new(x0, 0.0, -80.0), Vec3::new(x1, BLOCK_H, 80.0), ObjectKind::Building, &mut v, &mut i, &mut o);
    }
    let mut r = Renderer::from_parts(ctx, &v, &i, &o, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
    let lighting = LightingOptions { ambient_volume: ambient, ..LightingOptions::OFF };
    let opts = LookOptions { ao: false, lighting, ..r.look.options };
    r.look.set_options(&ctx.device, opts);
    r.look.set_time_of_day(15.0);
    r
}

fn camera() -> Camera {
    // over the street, low enough for the sun's cascades (160 m): the floor in the west block's shadow at 15 h.
    Camera::new(Vec3::new(0.0, 110.0, 20.0), 0.0, -1.45, W as f32 / H as f32)
}

struct Frames {
    out: wgpu::TextureView,
    targets: Targets,
}

impl Frames {
    fn new(ctx: &GpuContext) -> Self {
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
        Frames { out: out.create_view(&Default::default()), targets: Targets::new(&ctx.device, W, H, wgpu::TextureFormat::Rgba8UnormSrgb, 1) }
    }

    fn draw(&self, ctx: &GpuContext, r: &Renderer, cam: &Camera) {
        r.update(&ctx.queue, cam);
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        r.encode(&mut enc, &self.out, None, &self.targets.depth, None);
        ctx.queue.submit([enc.finish()]);
    }
}

fn half(b: u16) -> f32 {
    eo_render::ambient_bake::f16_value(b)
}

/// The HDR scene's luminance per pixel.
fn hdr(ctx: &GpuContext, r: &Renderer) -> Vec<f32> {
    let d = &ctx.device;
    let view = r.look.hdr_target(W, H);
    let row = (W * 8).div_ceil(256) * 256;
    let buf = d.create_buffer(&wgpu::BufferDescriptor { label: None, size: (row * H) as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let mut enc = d.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        view.texture().as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(H) } },
        wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
    );
    ctx.queue.submit([enc.finish()]);
    let raw = read_mapped(ctx, &buf).unwrap();
    let mut out = Vec::with_capacity((W * H) as usize);
    for y in 0..H as usize {
        for x in 0..W as usize {
            let o = y * row as usize + x * 8;
            let c = [0, 1, 2].map(|k| half(u16::from_le_bytes([raw[o + 2 * k], raw[o + 2 * k + 1]])));
            out.push(0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]);
        }
    }
    out
}

/// The mean luminance of the 3 x 3 pixels round the world point `p`.
fn at(lum: &[f32], cam: &Camera, p: Vec3) -> f32 {
    let c = cam.view_proj() * p.extend(1.0);
    let (nx, ny) = (c.x / c.w, c.y / c.w);
    let (px, py) = (((nx * 0.5 + 0.5) * W as f32) as i32, ((0.5 - ny * 0.5) * H as f32) as i32);
    assert!(px >= 1 && py >= 1 && px < W as i32 - 1 && py < H as i32 - 1, "{p} is off the frame ({px}, {py})");
    let mut s = 0.0;
    for dy in -1..=1 {
        for dx in -1..=1 {
            s += lum[((py + dy) * W as i32 + px + dx) as usize];
        }
    }
    s / 9.0
}

#[test]
fn a_canyon_takes_less_sky_light_than_open_ground() {
    let ctx = ctx();
    let frames = Frames::new(&ctx);
    let cam = camera();
    let plain = scene(&ctx, 0);
    let lit = scene(&ctx, 1);
    let rings: Vec<(Vec<[f64; 2]>, f64)> = BLOCKS.iter().map(|&(x0, x1)| (vec![[x0 as f64, -80.0], [x1 as f64, -80.0], [x1 as f64, 80.0], [x0 as f64, 80.0]], BLOCK_H as f64)).collect();
    let inputs = AmbientInputs { buildings: AmbientBuildings::Rings(rings), lift: 0.0, ground: AmbientGround::Flat(0.0), structures: None, decks: vec![], cache_dir: None, threads: 2 };
    let scope = ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let bytes = lit.ambient.attach(&ctx, &lit.look, inputs, [cam.eye.x as f64, cam.eye.z as f64]);
    assert!(bytes > 0 && lit.ambient.attached());
    // stream until resident (the bake runs on its own threads).
    let t = std::time::Instant::now();
    while !lit.ambient.settled() {
        frames.draw(&ctx, &lit, &cam);
        let _ = ctx.device.poll(wgpu::PollType::Poll);
        assert!(t.elapsed().as_secs() < 120, "the volume never settled: {}", lit.ambient.unsettled());
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    for _ in 0..3 {
        frames.draw(&ctx, &lit, &cam);
        frames.draw(&ctx, &plain, &cam);
    }
    let lx = lit.look.lighting_params();
    assert!(lx[11][2] > 0.0 && lx[10][0] != lx[10][1], "the plane and the volume are published ({:?} {:?})", lx[10], lx[11]);
    let (on, off) = (hdr(&ctx, &lit), hdr(&ctx, &plain));
    if let Some(e) = pollster::block_on(scope.pop()) {
        panic!("validation error: {e}");
    }
    if let Ok(dir) = std::env::var("EO_AMBIENT_TEST_PNG") {
        for (name, img) in [("on", &on), ("off", &off)] {
            let f = std::fs::File::create(format!("{dir}/ambient-test-{name}.png")).unwrap();
            let mut e = png::Encoder::new(std::io::BufWriter::new(f), W, H);
            e.set_color(png::ColorType::Grayscale);
            let px: Vec<u8> = img.iter().map(|v| ((v / (1.0 + v)).powf(1.0 / 2.2) * 255.0) as u8).collect();
            e.write_header().unwrap().write_image_data(&px).unwrap();
        }
    }
    let canyon = Vec3::new(0.0, 0.05, 10.0);
    let open = Vec3::new(95.0, 0.05, 10.0); // 71 m from the east block, in the sun: a few hundredths of its sky taken
    let (c_on, c_off, o_on, o_off) = (at(&on, &cam, canyon), at(&off, &cam, canyon), at(&on, &cam, open), at(&off, &cam, open));
    let s = lit.ambient.stats();
    println!("canyon floor {c_off:.4} -> {c_on:.4} ({:.3}), open ground {o_off:.4} -> {o_on:.4} ({:.3}); {} tiles ({} baked, {:.1} ms a tile), plane {} blocks, {:.2} MB uploaded, {:.1} MB on the GPU", c_on / c_off, o_on / o_off, s.tiles_resident, s.tiles_baked, s.tile_ms / s.tiles_baked.max(1) as f64, s.plane_blocks, s.uploaded_bytes as f64 / 1e6, bytes as f64 / 1e6);
    assert!((o_on / o_off - 1.0).abs() < 0.02, "open ground changed: {o_off} -> {o_on}");
    assert!(c_on / c_off < 0.85, "the canyon's floor kept its light: {c_off} -> {c_on}");
    assert!(c_on > 0.05 * c_off, "the canyon's floor went black: {c_off} -> {c_on}");
}

/// The slab over open ground: 120 m long along x, 20 m wide (z -10..10), its soffit 9 m and its top 10 m up.
const SLAB: (Vec3, Vec3) = (Vec3::new(-60.0, 9.0, -10.0), Vec3::new(60.0, 10.0, 10.0));
/// The closed courtyard 300 m east: a 60 m ring of 20 m blocks round a 20 m yard (x 290-310, z -10..10), roofed by a slab at 9-10 m.
const YARD_X: f32 = 300.0;

fn yard_blocks() -> [(Vec3, Vec3); 4] {
    let (x, h) = (YARD_X, 20.0);
    [
        (Vec3::new(x - 30.0, 0.0, -30.0), Vec3::new(x + 30.0, h, -10.0)),
        (Vec3::new(x - 30.0, 0.0, 10.0), Vec3::new(x + 30.0, h, 30.0)),
        (Vec3::new(x - 30.0, 0.0, -10.0), Vec3::new(x - 10.0, h, 10.0)),
        (Vec3::new(x + 10.0, 0.0, -10.0), Vec3::new(x + 30.0, h, 10.0)),
    ]
}

fn yard_lid() -> (Vec3, Vec3) {
    (Vec3::new(YARD_X - 10.0, 9.0, -10.0), Vec3::new(YARD_X + 10.0, 10.0, 10.0))
}

/// The slabs and the courtyard, with the ambient volume on and the ground bounce on or off, at `hour`.
fn slab_scene(ctx: &GpuContext, bounce: bool, hour: f64) -> Renderer {
    let (mut v, mut i, mut o) = (vec![], vec![], vec![]);
    quad_box(Vec3::new(-400.0, -1.0, -400.0), Vec3::new(600.0, 0.0, 400.0), ObjectKind::Terrain, &mut v, &mut i, &mut o);
    for (a, b) in [SLAB, yard_lid()].into_iter().chain(yard_blocks()) {
        quad_box(a, b, ObjectKind::Building, &mut v, &mut i, &mut o);
    }
    let mut r = Renderer::from_parts(ctx, &v, &i, &o, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
    let lighting = LightingOptions { ambient_volume: 1, ground_bounce: bounce, ..LightingOptions::OFF };
    let opts = LookOptions { ao: false, lighting, ..r.look.options };
    r.look.set_options(&ctx.device, opts);
    r.look.set_time_of_day(hour);
    r
}

/// The bake's view of the same: the courtyard's blocks as prisms, the two slabs as decks.
fn slab_inputs() -> AmbientInputs {
    let ring = |a: Vec3, b: Vec3| vec![[a.x as f64, a.z as f64], [b.x as f64, a.z as f64], [b.x as f64, b.z as f64], [a.x as f64, b.z as f64]];
    let rings: Vec<(Vec<[f64; 2]>, f64)> = yard_blocks().iter().map(|&(a, b)| (ring(a, b), b.y as f64)).collect();
    let deck = |(a, b): (Vec3, Vec3)| {
        let zc = 0.5 * (a.z + b.z) as f64;
        let half = 0.5 * (b.z - a.z) as f64;
        DeckPiece { a: [a.x as f64, zc], b: [b.x as f64, zc], half: [half, half], top: [b.y as f64; 2], bottom: [a.y as f64; 2] }
    };
    AmbientInputs { buildings: AmbientBuildings::Rings(rings), lift: 0.0, ground: AmbientGround::Flat(0.0), structures: None, decks: vec![deck(SLAB), deck(yard_lid())], cache_dir: None, threads: 2 }
}

fn settle(ctx: &GpuContext, frames: &Frames, r: &Renderer, cam: &Camera) {
    let t = std::time::Instant::now();
    while !r.ambient.settled() {
        frames.draw(ctx, r, cam);
        let _ = ctx.device.poll(wgpu::PollType::Poll);
        assert!(t.elapsed().as_secs() < 180, "the volume never settled: {}", r.ambient.unsettled());
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[test]
fn the_sunlit_ground_bounces_onto_a_slabs_underside_not_its_top_nor_at_night() {
    let ctx = ctx();
    let frames = Frames::new(&ctx);
    // under the slab, near its west end, looking east along it and up at the soffit (yaw pi/2: +x).
    let under = Camera::new(Vec3::new(-45.0, 2.0, 0.0), std::f32::consts::FRAC_PI_2, 0.35, W as f32 / H as f32);
    // over it, looking down at its top (yaw 0: -z).
    let over = Camera::new(Vec3::new(0.0, 60.0, 40.0), 0.0, -1.0, W as f32 / H as f32);
    // inside the closed courtyard, looking north up at the lid from its south side (yaw pi: +z).
    let yard = Camera::new(Vec3::new(YARD_X, 1.7, -8.0), std::f32::consts::PI, 0.9, W as f32 / H as f32);
    let soffit_mid = Vec3::new(-20.0, 8.98, 0.0);
    // 1.5 m in from its two long edges (z -10 and 10).
    let soffit_edges = [Vec3::new(-20.0, 8.98, -8.5), Vec3::new(-20.0, 8.98, 8.5)];
    let top = Vec3::new(0.0, 10.02, 0.0);
    let lid = Vec3::new(YARD_X, 8.98, 0.0);
    let scope = ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut rows = vec![];
    for hour in [15.0, 23.0] {
        let (on, off) = (slab_scene(&ctx, true, hour), slab_scene(&ctx, false, hour));
        for r in [&on, &off] {
            assert!(r.ambient.attach(&ctx, &r.look, slab_inputs(), [0.0, 0.0]) > 0);
            settle(&ctx, &frames, r, &under);
        }
        let lx = on.look.lighting_params();
        // the edge on the sun's side (its shadow falls the other way: the ground under that edge is sunlit).
        let sun = on.look.frame().key_dir;
        let (soffit_edge, shaded_edge) = if sun[2] > 0.0 { (soffit_edges[1], soffit_edges[0]) } else { (soffit_edges[0], soffit_edges[1]) };
        println!("{hour} h: the ground bounce's sun exitance {:?}, toward the sun {sun:?}", &lx[0][1..4]);
        assert_eq!(off.look.lighting_params()[0][1..4], [0.0; 3], "off: no bounce row");
        let take = |r: &Renderer, cam: &Camera, p: Vec3| {
            for _ in 0..3 {
                frames.draw(&ctx, r, cam);
            }
            at(&hdr(&ctx, r), cam, p)
        };
        let (mid_on, mid_off) = (take(&on, &under, soffit_mid), take(&off, &under, soffit_mid));
        let (edge_on, edge_off) = (take(&on, &under, soffit_edge), take(&off, &under, soffit_edge));
        let (shade_on, shade_off) = (take(&on, &under, shaded_edge), take(&off, &under, shaded_edge));
        let (top_on, top_off) = (take(&on, &over, top), take(&off, &over, top));
        let (lid_on, lid_off) = (take(&on, &yard, lid), take(&off, &yard, lid));
        println!(
            "{hour} h: soffit middle {mid_off:.4} -> {mid_on:.4} ({:.2}), near its sunward edge {edge_off:.4} -> {edge_on:.4} ({:.2}), near its other edge {shade_off:.4} -> {shade_on:.4} ({:.2}), top {top_off:.4} -> {top_on:.4} ({:.3}), courtyard lid {lid_off:.4} -> {lid_on:.4} ({:.2})",
            mid_on / mid_off,
            edge_on / edge_off,
            shade_on / shade_off,
            top_on / top_off,
            lid_on / lid_off
        );
        rows.push((hour, lx[0], mid_on / mid_off, edge_on / edge_off, shade_on / shade_off, top_on / top_off, lid_on / lid_off));
    }
    if let Some(e) = pollster::block_on(scope.pop()) {
        panic!("validation error: {e}");
    }
    let (_, lx, mid, edge, shade, top, lid) = rows[0];
    assert!(lx[1] > 0.0 && lx[2] > 0.0 && lx[3] > 0.0, "15 h: the bounce row is set: {lx:?}");
    assert!(mid > 1.05, "15 h: the soffit's middle takes the bounce: x {mid}");
    assert!(edge > mid + 0.3, "15 h: more towards the open edge on the sun's side: x {edge} against x {mid}");
    assert!(edge > shade + 0.2, "15 h: the edge whose ground lies in the deck's shadow takes less: x {shade} against x {edge}");
    assert!(edge < 4.0, "15 h: never glowing: x {edge}");
    assert!((top - 1.0).abs() < 0.01, "15 h: the top takes none: x {top}");
    // the closed yard's lid sees no sunlit ground: the lobe's sun part leaves it, nothing comes back; never black.
    assert!(lid < 1.0 && lid > 0.4, "15 h: the closed courtyard's lid takes none: x {lid} against x {mid}");
    let (_, lx, mid, edge, shade, top, lid) = rows[1];
    assert_eq!(lx[1..4], [0.0; 3], "23 h: no bounce at night");
    for (name, v) in [("soffit middle", mid), ("soffit edge", edge), ("other edge", shade), ("top", top), ("lid", lid)] {
        assert!((v - 1.0).abs() < 1e-3, "23 h: the {name} changed at night: x {v}");
    }
}
