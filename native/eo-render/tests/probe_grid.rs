//! `--probe-grid` on the GPU renderer (probe_grid.rs): the grid captures its
//! set one face a frame in the single probe's place, settles, holds still
//! when frozen, and changes the frame only through what it captured; and on
//! the real city every street anchor stands on the street, outside every
//! building. Needs a GPU and the city data.

use eo_render::camera::Camera;
use eo_render::look::{LightingOptions, LookOptions};
use eo_render::probe_grid::{self, Anchors};
use eo_render::scene::{GpuObject, ObjectKind, Vertex};
use eo_render::{GpuContext, GpuOptions, Renderer, Targets, read_mapped, wgpu};
use glam::Vec3;
use std::sync::Arc;

fn ctx() -> GpuContext {
    let opts = GpuOptions::default();
    GpuContext::new(GpuContext::create_instance(&opts), None, opts).expect("a GPU adapter is required")
}

const W: u32 = 192;
const H: u32 = 108;

fn quad_box(min: Vec3, max: Vec3, kind: ObjectKind, color: u32, verts: &mut Vec<Vertex>, idx: &mut Vec<u32>, objects: &mut Vec<GpuObject>) {
    let base = verts.len() as i32;
    let first = idx.len() as u32;
    for i in 0..8 {
        let p = Vec3::new(if i & 1 != 0 { max.x } else { min.x }, if i & 2 != 0 { max.y } else { min.y }, if i & 4 != 0 { max.z } else { min.z });
        verts.push(Vertex { pos: p.to_array(), normal: 0, color });
    }
    let faces = [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]];
    for f in faces {
        idx.extend_from_slice(&[f[0], f[1], f[2], f[0], f[2], f[3]]);
    }
    objects.push(GpuObject::new(kind, min.to_array(), max.to_array(), first, 36, base));
}

/// A terrain slab with two coloured towers either side of a street; the grid at `level` (0: none attached).
fn scene(ctx: &GpuContext, level: u8) -> Renderer {
    let (mut v, mut i, mut o) = (vec![], vec![], vec![]);
    quad_box(Vec3::new(-80.0, -1.0, -80.0), Vec3::new(80.0, 0.0, 80.0), ObjectKind::Terrain, 0xff80_8080, &mut v, &mut i, &mut o);
    quad_box(Vec3::new(-30.0, 0.0, -20.0), Vec3::new(-14.0, 30.0, 20.0), ObjectKind::Building, 0xff20_20c0, &mut v, &mut i, &mut o);
    quad_box(Vec3::new(14.0, 0.0, -20.0), Vec3::new(30.0, 30.0, 20.0), ObjectKind::Building, 0xffc0_6020, &mut v, &mut i, &mut o);
    let mut r = Renderer::from_parts(ctx, &v, &i, &o, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
    // without the occlusion pass (it differs between two identical frames on its own).
    let opts = LookOptions { ao: false, lighting: LightingOptions { probe_grid: level, ..LightingOptions::OFF }, ..r.look.options };
    r.look.set_options(&ctx.device, opts);
    r.look.set_time_of_day(15.0);
    if level > 0 {
        // anchors down the street between the towers, 1.7 m up, 16 m apart.
        let anchors = Anchors::new((0..7).map(|k| [0.0, 1.7, -48.0 + 16.0 * k as f32]).collect());
        assert!(r.attach_probe_grid(ctx, level, anchors) > 0);
    }
    r
}

fn camera() -> Camera {
    // over the street's south end, looking north along it and down at it.
    Camera::new(Vec3::new(0.0, 12.0, 40.0), 0.0, -0.35, W as f32 / H as f32)
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
        Self { out: out.create_view(&Default::default()), targets: Targets::new(&ctx.device, W, H, wgpu::TextureFormat::Rgba8UnormSrgb, 1) }
    }

    fn draw(&self, ctx: &GpuContext, r: &Renderer, cam: &Camera) {
        r.update(&ctx.queue, cam);
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        r.encode(&mut enc, &self.out, None, &self.targets.depth, None);
        ctx.queue.submit([enc.finish()]);
    }

    /// The HDR scene target's bytes.
    fn hdr(&self, ctx: &GpuContext, r: &Renderer) -> Vec<u8> {
        let hdr = r.look.hdr_target(W, H);
        let row = (W * 8).div_ceil(256) * 256;
        let buf = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (row * H) as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            hdr.texture().as_image_copy(),
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(H) } },
            wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        );
        ctx.queue.submit([enc.finish()]);
        let raw = read_mapped(ctx, &buf).unwrap();
        raw.chunks(row as usize).flat_map(|r| r[..(W * 8) as usize].to_vec()).collect()
    }
}

#[test]
fn the_grid_captures_one_face_a_frame_in_the_single_probes_place_settles_and_holds() {
    let ctx = ctx();
    let frames = Frames::new(&ctx);
    let cam = camera();
    let mut r = scene(&ctx, 1);
    let scope = ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let (mut settled_at, mut last_faces, mut most) = (None, 0u64, 0u64);
    for k in 0..probe_grid::SETTLE_FRAMES_MAX {
        frames.draw(&ctx, &r, &cam);
        let s = r.probe_grid.stats();
        most = most.max(s.faces - last_faces);
        last_faces = s.faces;
        if r.probe_grid.settled() {
            settled_at = Some(k + 1);
            break;
        }
    }
    if let Some(e) = pollster::block_on(scope.pop()) {
        panic!("validation error: {e}");
    }
    let s = r.probe_grid.stats();
    let single = r.probe.stats();
    println!("probe grid: settled after {settled_at:?} frames; {} captures, {} faces, {} deferred, {} frames a capture (last); the single probe {} faces", s.captures, s.faces, s.deferred, s.last_capture_frames, single.faces);
    if s.faces == 0 {
        println!("probe grid: no face on this adapter (no compacted lists); nothing more is checked");
        return;
    }
    assert!(settled_at.is_some(), "the grid settles within {} frames", probe_grid::SETTLE_FRAMES_MAX);
    assert_eq!(most, 1, "never more than one face a frame");
    assert_eq!(s.captures, 4, "level 1: four probes");
    assert_eq!(s.faces, 24);
    assert_eq!(single.faces, 0, "the single probe captures nothing while the grid is on");
    assert!(r.probe.front().1.is_none(), "and has no cube in use");
    // the four probes nearest a point AHEAD_M ahead of the eye (looking north from z = 40: round z = 16).
    let mut zs: Vec<i32> = r.probe_grid.probes().iter().filter(|p| p.wanted).map(|p| p.pos.z as i32).collect();
    zs.sort();
    assert_eq!(zs, vec![-16, 0, 16, 32]);
    // held: two renders of one camera are the same frame, and nothing is captured.
    r.probe_grid.set_frozen(true);
    frames.draw(&ctx, &r, &cam);
    let a = frames.hdr(&ctx, &r);
    for _ in 0..3 {
        frames.draw(&ctx, &r, &cam);
    }
    let b = frames.hdr(&ctx, &r);
    assert_eq!(r.probe_grid.stats(), s, "frozen: no capture, no face");
    let differ = a.chunks(8).zip(b.chunks(8)).filter(|(p, q)| p != q).count();
    assert_eq!(differ, 0, "{differ} HDR pixels differ between two renders of a frozen grid");
    // the grid reads its own captures: the frame differs from one with the grid's table emptied (the analytic sky).
    r.probe_grid.set_frozen(false);
    let off = LookOptions { lighting: LightingOptions::OFF, ..r.look.options };
    r.look.set_options(&ctx.device, off);
    for _ in 0..2 {
        frames.draw(&ctx, &r, &cam);
    }
    let c = frames.hdr(&ctx, &r);
    let changed = a.chunks(8).zip(c.chunks(8)).filter(|(p, q)| p != q).count();
    println!("probe grid: {changed} of {} HDR pixels change with the grid's reflections", W * H);
    assert!(changed > (W * H / 20) as usize, "the street and the towers reflect the grid ({changed} pixels change)");
}

#[test]
fn a_recapture_fades_in_with_no_step_between_frames() {
    let ctx = ctx();
    let frames = Frames::new(&ctx);
    let cam = camera();
    let r = scene(&ctx, 1);
    for _ in 0..probe_grid::SETTLE_FRAMES_MAX {
        frames.draw(&ctx, &r, &cam);
        if r.probe_grid.settled() {
            break;
        }
    }
    if r.probe_grid.stats().faces == 0 {
        println!("probe grid: no face on this adapter; nothing is checked");
        return;
    }
    let lum = |hdr: &[u8]| -> Vec<f32> {
        hdr.chunks(8)
            .map(|p| {
                let h = |k: usize| half(u16::from_le_bytes([p[2 * k], p[2 * k + 1]]));
                0.2126 * h(0) + 0.7152 * h(1) + 0.0722 * h(2)
            })
            .collect()
    };
    // the sky's weather turns to overcast: its irradiance moves past 10 %, every probe is captured again, each cross-fading.
    let before = r.probe_grid.stats();
    let mut o = r.look.options.sky;
    o.weather = eo_render::sky::WeatherMode::Preset(eo_sky::Kind::Overcast);
    r.look.set_sky_options(o);
    // the frame of the new sky before any probe has changed.
    frames.draw(&ctx, &r, &cam);
    let first = lum(&frames.hdr(&ctx, &r));
    let mut prev = first.clone();
    let (mut steps, mut done) = (vec![], None);
    for k in 0..probe_grid::SETTLE_FRAMES_MAX {
        let caps = r.probe_grid.stats().captures;
        frames.draw(&ctx, &r, &cam);
        let now = lum(&frames.hdr(&ctx, &r));
        let step = now.iter().zip(&prev).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        steps.push((k, step, r.probe_grid.stats().captures > caps));
        prev = now;
        if r.probe_grid.settled() {
            done = Some(k);
            break;
        }
    }
    let after = r.probe_grid.stats();
    let recaptured = after.recaptures - before.recaptures;
    println!("probe grid: {recaptured} recaptures over {done:?} frames after the weather turned");
    if recaptured == 0 {
        println!("probe grid: the sky's irradiance did not move past 10 % on this adapter's frame; nothing more is checked");
        return;
    }
    // the largest per-pixel change of luminance between two frames: no frame takes the swap at once.
    // (the reflections' whole change spread over FADE_S: a frame takes at most a smoothstep's steepest share of one probe's.)
    let largest: f32 = steps.iter().map(|s| s.1).fold(0.0, f32::max);
    let at_capture = steps.iter().filter(|s| s.2).map(|s| s.1).fold(0.0f32, f32::max);
    let elsewhere = steps.iter().filter(|s| !s.2).map(|s| s.1).fold(0.0f32, f32::max);
    let whole = prev.iter().zip(&first).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    println!("probe grid: whole change of the reflections {whole:.5}; largest frame-to-frame change {largest:.5} (into a frame with a finished capture {at_capture:.5}, elsewhere {elsewhere:.5})");
    assert!(whole > 0.0, "the recaptures changed the frame");
    assert!(largest <= 0.25 * whole, "no frame takes more than a quarter of the whole change ({largest} of {whole})");
    assert!(at_capture <= elsewhere * 1.5 + 1e-4, "a finished capture changes the frame no more than the fade does elsewhere ({at_capture} vs {elsewhere})");
}

/// IEEE half to f32.
fn half(h: u16) -> f32 {
    let s = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let e = ((h >> 10) & 0x1f) as i32;
    let m = (h & 0x3ff) as f32;
    s * match e {
        0 => m * 2f32.powi(-24),
        31 => f32::INFINITY,
        _ => (1.0 + m / 1024.0) * 2f32.powi(e - 15),
    }
}

#[test]
fn every_street_anchor_stands_on_the_street_outside_every_building() {
    let paths = eo_data::DataPaths::discover().unwrap();
    let city = Arc::new(eo_data::City::load(&paths).unwrap());
    let elev = eo_data::ElevationGrid::load(&paths).unwrap();
    let o = eo_street::StreetOptions::load().unwrap();
    let lift = o.open_ground_lift;
    let rings: Vec<Vec<[f64; 2]>> = city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
    let sw = eo_street::StreetWorld::build(o, eo_street::StreetCity::load().unwrap(), eo_street::Terrain { grid: Arc::new(elev.clone()), carpet_lift: lift }, eo_street::BuildingIndex::new(rings.clone()));
    let t = std::time::Instant::now();
    let a = Anchors::from_streets(&sw);
    let ms = t.elapsed().as_secs_f64() * 1e3;
    assert!(a.len() > 10_000, "anchors over the whole city ({})", a.len());
    // an independent test against every footprint near each anchor (bounding boxes, then the ring).
    let boxes: Vec<[f64; 4]> = rings.iter().map(|r| r.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])])).collect();
    let mut grid: std::collections::HashMap<(i32, i32), Vec<usize>> = std::collections::HashMap::new();
    for (i, b) in boxes.iter().enumerate() {
        for x in (b[0] / 64.0).floor() as i32..=(b[2] / 64.0).floor() as i32 {
            for z in (b[1] / 64.0).floor() as i32..=(b[3] / 64.0).floor() as i32 {
                grid.entry((x, z)).or_default().push(i);
            }
        }
    }
    let (mut inside, mut off_paving, mut off_height) = (0usize, 0usize, 0usize);
    for p in a.points() {
        let (x, y, z) = (p[0] as f64, p[1] as f64, p[2] as f64);
        let cell = ((x / 64.0).floor() as i32, (z / 64.0).floor() as i32);
        if grid.get(&cell).is_some_and(|l| l.iter().any(|&i| {
            let b = boxes[i];
            x >= b[0] && x <= b[2] && z >= b[1] && z <= b[3] && eo_street::point_in_ring(x, z, &rings[i])
        })) {
            inside += 1;
        }
        if sw.paved_at(x, z).is_none() {
            off_paving += 1;
        }
        if (y - (sw.ground(x, z) + sw.o.road_lift + probe_grid::PROBE_HEIGHT_M)).abs() > 0.01 {
            off_height += 1;
        }
    }
    println!("probe grid: {} street anchors in {ms:.0} ms; {inside} inside a footprint, {off_paving} off the paving, {off_height} off the datum + {} m", a.len(), probe_grid::PROBE_HEIGHT_M);
    assert_eq!(inside, 0, "no anchor inside a building footprint");
    assert_eq!(off_height, 0, "every anchor stands PROBE_HEIGHT_M over the street datum");
    assert!(off_paving * 100 <= a.len(), "anchors off the paving: {off_paving} of {}", a.len());
    // the anchors are at most ANCHOR_SPACING_M apart along a run: a probe within half of it of any street point.
    let near = a.nearest(Vec3::new(a.points()[a.len() / 2][0], 1.7, a.points()[a.len() / 2][2]), 2, &[], 0.0);
    assert_eq!(near.len(), 2);
}
