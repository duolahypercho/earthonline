//! The far sun shadow map and the cascade blend on the GPU (stage B2). Needs a GPU.
//!
//!  - a tall box 500 m from the camera shadows the ground beside it with
//!    `far_shadows` 1 and does not without; one 1500 m away needs ring 2
//!    (`far_shadows` 2, past ring 1's 1 km); the sunlit ground round them is
//!    the same with and without (no self-shadowing of the ground);
//!  - the map is not redrawn on frames where neither the sun nor the camera
//!    moved enough, and is when either did;
//!  - with `--cascade-blend` the frame changes only inside the blend bands
//!    before the splits (everything else bit for bit the frame without it).

use eo_render::camera::Camera;
use eo_render::look::{LightingOptions, LookOptions};
use eo_render::scene::{GpuObject, ObjectKind, Vertex};
use eo_render::{GpuContext, GpuOptions, Renderer, Targets, read_mapped, wgpu};
use glam::Vec3;

const W: u32 = 512;
const H: u32 = 288;
const HOUR: f64 = 16.0;

fn ctx() -> GpuContext {
    let opts = GpuOptions::default();
    GpuContext::new(GpuContext::create_instance(&opts), None, opts).expect("a GPU adapter is required")
}

fn quad_box(min: Vec3, max: Vec3, kind: ObjectKind, verts: &mut Vec<Vertex>, idx: &mut Vec<u32>, objects: &mut Vec<GpuObject>) {
    let base = verts.len() as i32;
    let first = idx.len() as u32;
    for i in 0..8 {
        let p = Vec3::new(if i & 1 != 0 { max.x } else { min.x }, if i & 2 != 0 { max.y } else { min.y }, if i & 4 != 0 { max.z } else { min.z });
        verts.push(Vertex { pos: p.to_array(), normal: 0, color: 0xff80_8080 });
    }
    for f in [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]] {
        idx.extend_from_slice(&[f[0], f[1], f[2], f[0], f[2], f[3]]);
    }
    objects.push(GpuObject::new(kind, min.to_array(), max.to_array(), first, 36, base));
}

fn renderer(ctx: &GpuContext, boxes: &[(Vec3, Vec3)], ground: f32, lighting: LightingOptions) -> Renderer {
    let (mut v, mut i, mut o) = (vec![], vec![], vec![]);
    quad_box(Vec3::new(-ground, -1.0, -ground), Vec3::new(ground, 0.0, ground), ObjectKind::Terrain, &mut v, &mut i, &mut o);
    for (a, b) in boxes {
        quad_box(*a, *b, ObjectKind::Building, &mut v, &mut i, &mut o);
    }
    let mut r = Renderer::from_parts(ctx, &v, &i, &o, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
    let opts = LookOptions { ao: false, lighting, ..r.look.options };
    r.look.set_options(&ctx.device, opts);
    r.look.set_time_of_day(HOUR);
    r
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

    fn draw(&self, ctx: &GpuContext, r: &Renderer, cam: &Camera, frames: u32) {
        for _ in 0..frames {
            r.update(&ctx.queue, cam);
            let mut enc = ctx.device.create_command_encoder(&Default::default());
            r.encode(&mut enc, &self.out, None, &self.targets.depth, None);
            ctx.queue.submit([enc.finish()]);
        }
    }
}

/// The HDR scene, raw (Rgba16Float) and as luminance per pixel.
fn hdr(ctx: &GpuContext, r: &Renderer) -> (Vec<u64>, Vec<f32>) {
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
    let (mut bits, mut lum) = (Vec::with_capacity((W * H) as usize), Vec::with_capacity((W * H) as usize));
    for y in 0..H as usize {
        for x in 0..W as usize {
            let o = y * row as usize + x * 8;
            bits.push(u64::from_le_bytes(raw[o..o + 8].try_into().unwrap()));
            let c = [0, 1, 2].map(|k| eo_render::ambient_bake::f16_value(u16::from_le_bytes([raw[o + 2 * k], raw[o + 2 * k + 1]])));
            lum.push(0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]);
        }
    }
    (bits, lum)
}

fn pixel(cam: &Camera, p: Vec3) -> (i32, i32) {
    let c = cam.view_proj() * p.extend(1.0);
    let (nx, ny) = (c.x / c.w, c.y / c.w);
    (((nx * 0.5 + 0.5) * W as f32) as i32, ((0.5 - ny * 0.5) * H as f32) as i32)
}

/// The mean luminance of the 3 x 3 pixels round the world point `p`.
fn at(lum: &[f32], cam: &Camera, p: Vec3) -> f32 {
    let (px, py) = pixel(cam, p);
    assert!(px >= 1 && py >= 1 && px < W as i32 - 1 && py < H as i32 - 1, "{p} is off the frame ({px}, {py})");
    let mut s = 0.0;
    for dy in -1..=1 {
        for dx in -1..=1 {
            s += lum[((py + dy) as u32 * W + (px + dx) as u32) as usize];
        }
    }
    s / 9.0
}

fn yaw_of(f: Vec3) -> f32 {
    f.x.atan2(-f.z)
}

/// The sun at `HOUR` (from a renderer's look), its altitude, and the level direction away from it.
fn sun(r: &Renderer) -> (Vec3, f32, Vec3) {
    let k = r.look.frame().key_dir;
    let s = Vec3::new(k[0] as f32, k[1] as f32, k[2] as f32);
    let away = -Vec3::new(s.x, 0.0, s.z).normalize();
    (s, s.y.asin(), away)
}

const BOX_HALF: f32 = 30.0;
const BOX_H: f32 = 120.0;

/// Boxes `dist` metres from an eye `height` up along the view (level), the view square to the sun.
fn layout(r: &Renderer, dists: &[f32], height: f32) -> (Camera, Vec<(Vec3, Vec3)>, Vec<(Vec3, Vec3)>) {
    let (s, alt, away) = sun(r);
    let view = Vec3::new(-away.z, 0.0, away.x); // level, square to the sun
    let eye = Vec3::new(0.0, height, 0.0);
    let far = dists.iter().cloned().fold(0.0, f32::max);
    let cam = Camera::new(eye, yaw_of(view), -(eye.y / (0.8 * far)).atan(), W as f32 / H as f32);
    let shadow_len = BOX_H / alt.tan();
    let mut boxes = vec![];
    let mut probes = vec![];
    for &d in dists {
        let c = Vec3::new(eye.x, 0.0, eye.z) + view * d;
        boxes.push((c - Vec3::new(BOX_HALF, 0.0, BOX_HALF), c + Vec3::new(BOX_HALF, BOX_H, BOX_HALF)));
        // the ground in the box's shadow (half its length out), and as far out on the sunward side (sunlit).
        let reach = (BOX_HALF + 0.5 * shadow_len).min(BOX_HALF + 60.0);
        probes.push((c + away * reach, c - away * reach));
    }
    let _ = s;
    (cam, boxes, probes)
}

#[test]
fn a_far_box_shadows_the_ground_with_the_far_map_and_not_without() {
    let ctx = ctx();
    let dists = [500.0, 1500.0];
    let probe_r = renderer(&ctx, &[], 4000.0, LightingOptions::OFF);
    // 420 m up (ring 1 grows to the far plane's 4.2 km there), and 60 m up (ring 1 is 2 km across).
    let (cam, boxes, probes) = layout(&probe_r, &dists, 420.0);
    drop(probe_r);
    let frames = Frames::new(&ctx);
    let scope = ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut rows = vec![];
    for level in [0u8, 1, 2] {
        let r = renderer(&ctx, &boxes, 4000.0, LightingOptions { far_shadows: level, ..LightingOptions::OFF });
        if level > 0 {
            assert!(r.attach_far_shadows(&ctx, level) > 0);
        }
        frames.draw(&ctx, &r, &cam, 2 + 2 * eo_render::far_shadow::SLICES);
        if level > 0 {
            assert!(r.far_shadow.settled(level), "level {level}: every ring drawn");
        }
        let (_, lum) = hdr(&ctx, &r);
        let row: Vec<(f32, f32)> = probes.iter().map(|(shade, sunlit)| (at(&lum, &cam, *shade), at(&lum, &cam, *sunlit))).collect();
        eprintln!("far shadows {level}: (shadowed side, sunlit side) at {dists:?} m: {row:?}; stats {:?}", r.far_shadow.stats());
        rows.push(row);
    }
    if let Some(e) = pollster::block_on(scope.pop()) {
        panic!("validation error: {e}");
    }
    let (off, one, two) = (&rows[0], &rows[1], &rows[2]);
    for (k, d) in dists.iter().enumerate() {
        // without: both sides sunlit alike (past the cascades nothing shadows).
        assert!((off[k].0 / off[k].1 - 1.0).abs() < 0.05, "{d} m: without far shadows the box's shadow side is lit ({:?})", off[k]);
        // the sunlit side never darkens (no self-shadowing of the ground or a ring's edge).
        for (lvl, r) in [(1, one), (2, two)] {
            assert!((r[k].1 / off[k].1 - 1.0).abs() < 0.03, "{d} m, level {lvl}: the sunlit ground keeps its light ({:?} vs {:?})", r[k], off[k]);
        }
    }
    // from 420 m up ring 1 reaches past both; ring 2 adds nothing there.
    for (k, d) in dists.iter().enumerate() {
        assert!(one[k].0 < 0.5 * one[k].1, "{d} m, level 1: shadowed ({:?})", one[k]);
        assert!(two[k].0 < 0.5 * two[k].1, "{d} m, level 2: shadowed ({:?})", two[k]);
    }
}

#[test]
fn from_the_street_ring_1_ends_softly_and_ring_2_reaches_on() {
    let ctx = ctx();
    let dists = [500.0, 1500.0];
    let probe_r = renderer(&ctx, &[], 4000.0, LightingOptions::OFF);
    let (mut cam, boxes, probes) = layout(&probe_r, &dists, 420.0);
    drop(probe_r);
    let frames = Frames::new(&ctx);
    let mut rows = vec![];
    for level in [1u8, 2] {
        let r = renderer(&ctx, &boxes, 4000.0, LightingOptions { far_shadows: level, ..LightingOptions::OFF });
        r.attach_far_shadows(&ctx, level);
        // fit the rings at a street-level eye (ring 1 then 2 km across), then hold them and look from above.
        let mut street = cam;
        street.eye.y = 30.0;
        frames.draw(&ctx, &r, &street, 2 + 2 * eo_render::far_shadow::SLICES);
        let fronts = r.far_shadow.fronts();
        assert_eq!(fronts[0].map(|f| f.radius), Some(1000.0), "a street-level eye: ring 1 is 1 km in radius");
        r.far_shadow.set_frozen(true);
        cam.eye.y = 420.0;
        frames.draw(&ctx, &r, &cam, 2);
        let (_, lum) = hdr(&ctx, &r);
        let row: Vec<(f32, f32)> = probes.iter().map(|(shade, sunlit)| (at(&lum, &cam, *shade), at(&lum, &cam, *sunlit))).collect();
        eprintln!("from the street, level {level}: (shadowed side, sunlit side) at {dists:?} m: {row:?}");
        rows.push(row);
    }
    let (one, two) = (&rows[0], &rows[1]);
    assert!(one[0].0 < 0.5 * one[0].1, "500 m, level 1: inside ring 1 ({:?})", one[0]);
    assert!((one[1].0 / one[1].1 - 1.0).abs() < 0.05, "1500 m, level 1: past ring 1's fade, lit ({:?})", one[1]);
    assert!(two[1].0 < 0.5 * two[1].1, "1500 m, level 2: ring 2 shadows it ({:?})", two[1]);
}

#[test]
fn the_far_map_is_redrawn_only_when_the_sun_or_the_camera_moved_enough() {
    let ctx = ctx();
    let r0 = renderer(&ctx, &[], 4000.0, LightingOptions::OFF);
    let (cam, boxes, _) = layout(&r0, &[500.0], 60.0);
    drop(r0);
    let r = renderer(&ctx, &boxes, 4000.0, LightingOptions { far_shadows: 1, ..LightingOptions::OFF });
    r.attach_far_shadows(&ctx, 1);
    let frames = Frames::new(&ctx);
    let slices = eo_render::far_shadow::SLICES as u64;
    frames.draw(&ctx, &r, &cam, slices as u32 + 2);
    let first = r.far_shadow.stats();
    assert_eq!((first.maps_completed, first.tiles_drawn, first.for_first), (1, slices, 1), "{first:?}");
    // still: nothing redrawn, no tile planned.
    frames.draw(&ctx, &r, &cam, 30);
    assert_eq!(r.far_shadow.stats(), first, "a still camera and sun redraw nothing");
    assert!(!r.far_shadow.draws_this_frame());
    // a small move (under an eighth of the 1 km radius) and a turn: nothing.
    let mut moved = cam;
    moved.eye += Vec3::new(40.0, -10.0, 60.0);
    moved.yaw += 1.0;
    frames.draw(&ctx, &r, &moved, 10);
    assert_eq!(r.far_shadow.stats(), first, "a 72 m move and a turn redraw nothing");
    // a 200 m move: one redraw, in SLICES tiles.
    moved.eye = cam.eye + Vec3::new(200.0, 0.0, 0.0);
    frames.draw(&ctx, &r, &moved, slices as u32 + 2);
    let s = r.far_shadow.stats();
    assert_eq!((s.maps_completed, s.tiles_drawn, s.for_camera), (2, 2 * slices, 1), "{s:?}");
    // the sun: a tenth of a degree's turn (24 s of clock) redraws, a hundredth does not.
    let h = r.look.time_of_day();
    r.look.set_time_of_day(h + 0.0005);
    frames.draw(&ctx, &r, &moved, 10);
    assert_eq!(r.far_shadow.stats().maps_completed, 2, "a 0.007 degree turn: nothing redrawn ({:?})", r.far_shadow.stats());
    r.look.set_time_of_day(h + 0.01);
    frames.draw(&ctx, &r, &moved, slices as u32 + 2);
    let t = r.far_shadow.stats();
    assert_eq!((t.maps_completed, t.for_sun), (3, 1), "a 0.15 degree turn redraws ({t:?})");
    // frozen: nothing, whatever moves.
    r.far_shadow.set_frozen(true);
    r.look.set_time_of_day(h + 0.5);
    frames.draw(&ctx, &r, &cam, slices as u32 + 2);
    assert_eq!(r.far_shadow.stats(), t, "frozen: nothing redrawn");
}

#[test]
fn the_cascade_blend_changes_only_the_bands_before_the_splits() {
    let ctx = ctx();
    let r0 = renderer(&ctx, &[], 300.0, LightingOptions::OFF);
    let (s, _, _) = sun(&r0);
    drop(r0);
    // a long wall along the view (-z), on the sun's side, whose shadow edge runs through every cascade.
    let view = Vec3::new(0.0, 0.0, -1.0);
    let eye = Vec3::new(0.0, 6.0, 0.0);
    let x: f32 = if s.x < 0.0 { -6.0 } else { 6.0 };
    let wall = (Vec3::new(x.min(x + 0.5), 0.0, -150.0), Vec3::new(x.max(x + 0.5), 8.0, -2.0));
    let cam = Camera::new(eye, yaw_of(view), -0.12, W as f32 / H as f32);
    let frames = Frames::new(&ctx);
    let mut out = vec![];
    for blend in [false, true] {
        let mut r = renderer(&ctx, &[wall], 300.0, LightingOptions { cascade_blend: blend, ..LightingOptions::OFF });
        // coarse maps (decimetre texels), so the cascades' edges disagree by whole pixels.
        let o = LookOptions { shadow_map: 256, ..r.look.options };
        r.look.set_options(&ctx.device, o);
        frames.draw(&ctx, &r, &cam, 3);
        let (bits, _) = hdr(&ctx, &r);
        out.push((bits, r.look.cascade_summary()));
    }
    let splits: Vec<f64> = out[0].1.iter().map(|c| c.0).collect();
    let band = eo_look::shadow::CASCADE_BLEND_BAND;
    let in_band = |vd: f64| (0..2).any(|c| {
        let start = if c == 0 { 0.0 } else { splits[c - 1] };
        vd >= splits[c] - (splits[c] - start) * band - 1.0 && vd <= splits[c] + 1.0
    });
    // each differing pixel's view depth: its ray's first hit on the wall or the ground (the scene is known).
    let inv = cam.view_proj().inverse();
    let hit = |dir: Vec3| -> f32 {
        let (mut t0, mut t1) = (0.0f32, f32::MAX);
        for k in 0..3 {
            let (o, d, lo, hi) = (eye[k], dir[k], wall.0[k], wall.1[k]);
            if d.abs() < 1e-9 {
                if o < lo || o > hi {
                    t0 = f32::MAX;
                }
            } else {
                let (a, b) = ((lo - o) / d, (hi - o) / d);
                t0 = t0.max(a.min(b));
                t1 = t1.min(a.max(b));
            }
        }
        let wall_t = if t0 <= t1 { t0 } else { f32::MAX };
        let ground_t = if dir.y < -1e-6 { eye.y / -dir.y } else { f32::MAX };
        wall_t.min(ground_t)
    };
    let (mut differ, mut outside) = (0, vec![]);
    for y in 0..H {
        for x in 0..W {
            let i = (y * W + x) as usize;
            if out[0].0[i] == out[1].0[i] {
                continue;
            }
            differ += 1;
            let ndc = glam::Vec2::new((x as f32 + 0.5) / W as f32 * 2.0 - 1.0, 1.0 - (y as f32 + 0.5) / H as f32 * 2.0);
            let dir = (inv.project_point3(ndc.extend(0.01)) - inv.project_point3(ndc.extend(1.0))).normalize();
            let vd = (dir * hit(dir)).dot(cam.forward()) as f64;
            if !in_band(vd) {
                outside.push((x, y, vd));
            }
        }
    }
    eprintln!("cascade blend: {differ} HDR pixels differ; splits {splits:?}; outside the bands: {:?}", &outside[..outside.len().min(10)]);
    assert!(differ > 20, "the blend changes the wall's shadow edge in the bands ({differ} pixels)");
    assert!(outside.is_empty(), "{} pixels changed outside the blend bands, e.g. {:?}", outside.len(), &outside[..outside.len().min(10)]);
}
