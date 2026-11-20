//! The fog lit by the local lights (`--lit-fog`, lit_fog.rs) on the GPU (needs
//! a GPU): a street lamp in fog adds light along the rays through its light
//! and none along the rays that miss it (rays from an eye over the
//! luminaire, going up: a full cutoff sends nothing there); by day, with the
//! lamps off, the frame is bit for bit the frame without the feature and the
//! passes do not run (and in fog by day, with the street lamp off, nothing
//! changes either); two renders of the same frames are bit-equal; a moving
//! headlamp's glow leaves no trail behind it, and a walking eye's volume is
//! the standing eye's (the history reprojected); the lighting slot is written
//! and the passes raise no validation error.

use eo_render::camera::Camera;
use eo_render::look::{LightingOptions, LookOptions};
use eo_render::scene::{GpuObject, ObjectKind, Vertex};
use eo_render::sky::{SkyOptions, WeatherMode};
use eo_render::{GpuContext, GpuOptions, Renderer, Targets, lighting, read_mapped, wgpu};
use eo_sky::lights::{LampKind, LampLight};
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

/// A ground slab, at `hour` in `weather`, with or without the lit fog (no
/// occlusion pass and no temporal resolve: both keep their own histories,
/// which would blur what is measured here; no wet ground or screen-space
/// reflection, which read the previous frame).
fn renderer(ctx: &GpuContext, lit_fog: u8, hour: f64, weather: eo_sky::Kind) -> Renderer {
    let (mut v, mut i, mut o) = (vec![], vec![], vec![]);
    quad_box(Vec3::new(-200.0, -1.0, -200.0), Vec3::new(200.0, 0.0, 200.0), &mut v, &mut i, &mut o);
    let mut r = Renderer::from_parts(ctx, &v, &i, &o, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
    let opts = LookOptions { ao: false, lighting: LightingOptions { lit_fog, ..LightingOptions::OFF }, ..r.look.options };
    r.look.set_options(&ctx.device, opts);
    r.look.set_time_of_day(hour);
    r.look.set_sky_options(SkyOptions { weather: WeatherMode::Preset(weather), taa: false, rain: false, wet: false, ssr: false, ..SkyOptions::default() });
    r
}

/// A street luminaire at `head` (a full-cutoff batwing), ~30 lux at its foot.
fn street_lamp(head: Vec3) -> LampLight {
    LampLight {
        pos: [head.x as f64, head.y as f64, head.z as f64],
        candela: 30.0 * (head.y as f64).powi(2),
        color: eo_sky::lights::street_lamp_color(),
        range: head.y as f64 * 2.6,
        dir: [0.0, -1.0, 0.0],
        cos_outer: 75f64.to_radians().cos(),
        cos_inner: 60f64.to_radians().cos(),
        kind: LampKind::Street,
    }
}

/// A low-beam headlamp at `pos` aimed along the horizontal `aim` (the agents' gather's cone and its 1.15 m
/// drop over 14 m).
fn headlamp(pos: Vec3, aim: Vec3) -> LampLight {
    let h = Vec3::new(aim.x, 0.0, aim.z).normalize();
    let d = Vec3::new(h.x, -0.0819, h.z).normalize();
    LampLight {
        pos: [pos.x as f64, pos.y as f64, pos.z as f64],
        candela: eo_sky::photometry::LOW_BEAM_PEAK_CD,
        color: [1.0, 0.96, 0.88],
        range: 45.0,
        dir: [d.x as f64, d.y as f64, d.z as f64],
        cos_outer: 0.42f64.cos(),
        cos_inner: (0.42f64 * 0.45).cos(),
        kind: LampKind::Headlight,
    }
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

/// One frame drawn (the lighting set, if given, at slot 0).
fn draw(ctx: &GpuContext, r: &Renderer, f: &Frame, cam: &Camera, set: Option<&wgpu::QuerySet>) {
    r.set_lighting_timestamps(set.map(|q| (q.clone(), 0)));
    r.update(&ctx.queue, cam);
    let mut enc = ctx.device.create_command_encoder(&Default::default());
    r.encode(&mut enc, &f.view, None, &f.targets.depth, None);
    ctx.queue.submit([enc.finish()]);
    r.set_lighting_timestamps(None);
}

/// The output's sRGB bytes (RGBA, row by row).
fn read(ctx: &GpuContext, f: &Frame) -> Vec<u8> {
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
    raw.chunks(row as usize).flat_map(|r| r[..(W * 4) as usize].to_vec()).collect()
}

fn luma(px: &[u8], x: u32, y: u32) -> f64 {
    let o = ((y * W + x) * 4) as usize;
    0.2126 * px[o] as f64 + 0.7152 * px[o + 1] as f64 + 0.0722 * px[o + 2] as f64
}

fn mean_luma(px: &[u8], xs: std::ops::Range<u32>, ys: std::ops::Range<u32>) -> f64 {
    let n = (xs.len() * ys.len()) as f64;
    ys.flat_map(|y| xs.clone().map(move |x| (x, y))).map(|(x, y)| luma(px, x, y)).sum::<f64>() / n
}

/// The lamp's head 8 m up, 20 m ahead of the eyes.
const HEAD: Vec3 = Vec3::new(0.0, 8.0, -20.0);

/// From the street looking up at the lamp (rays through its light, its glow
/// straight on), then from 6 m over its head (the upper half of the frame:
/// rays that never come down to it). 12 frames each; both outputs.
fn the_lamp_views(ctx: &GpuContext, r: &Renderer) -> (Vec<u8>, Vec<u8>) {
    let f = frame(ctx);
    r.look.sky.set_lamps(&ctx.queue, &[street_lamp(HEAD)]);
    let aspect = W as f32 / H as f32;
    let below = Camera::new(Vec3::new(0.0, 1.7, 0.0), 0.0, ((HEAD.y - 1.7) / 20.0).atan(), aspect);
    let above = Camera::new(Vec3::new(0.0, HEAD.y + 6.0, 0.0), 0.0, 0.0, aspect);
    for _ in 0..12 {
        draw(ctx, r, &f, &below, None);
    }
    let a = read(ctx, &f);
    for _ in 0..12 {
        draw(ctx, r, &f, &above, None);
    }
    (a, read(ctx, &f))
}

#[test]
fn a_lamp_in_fog_lights_the_rays_through_its_light_and_not_the_rays_that_miss_it() {
    let ctx = ctx();
    let off = renderer(&ctx, 0, 21.0, eo_sky::Kind::Fog);
    let on = renderer(&ctx, 1, 21.0, eo_sky::Kind::Fog);
    let scope = ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let (off_below, off_above) = the_lamp_views(&ctx, &off);
    let (on_below, on_above) = the_lamp_views(&ctx, &on);
    let err = pollster::block_on(scope.pop());
    assert!(err.is_none(), "validation error: {err:?}");
    assert!(on.lit_fog.stats().frames > 0 && on.lit_fog.stats().level == 1, "the lit fog's passes did not run: {:?}", on.lit_fog.stats());
    assert_eq!(off.lit_fog.stats().frames, 0);
    // the lamp at the frame's centre: round it, its glow; a ring 8-10 % of the width out, the halo fading.
    let (cx, cy) = (W / 2, H / 2);
    let core_off = mean_luma(&off_below, cx - 6..cx + 6, cy - 6..cy + 6);
    let core_on = mean_luma(&on_below, cx - 6..cx + 6, cy - 6..cy + 6);
    let ring_off = mean_luma(&off_below, cx + 26..cx + 32, cy - 3..cy + 3);
    let ring_on = mean_luma(&on_below, cx + 26..cx + 32, cy - 3..cy + 3);
    let far_off = mean_luma(&off_below, cx + 110..cx + 120, cy - 3..cy + 3);
    let far_on = mean_luma(&on_below, cx + 110..cx + 120, cy - 3..cy + 3);
    println!("from the street: luma round the lamp {core_off:.1} -> {core_on:.1}, 8 % out {ring_off:.1} -> {ring_on:.1}, 35 % out {far_off:.1} -> {far_on:.1}");
    assert!(core_on > core_off + 10.0, "the lamp has no glow in the fog");
    assert!(ring_on > ring_off + 3.0, "the glow does not reach out round the lamp");
    assert!(core_on - core_off > 2.0 * (ring_on - ring_off) && ring_on - ring_off > far_on - far_off, "the glow does not fade away from the lamp");
    // from over the luminaire: the rays going up never meet its light. The lit fog along them is
    // exactly zero (every froxel column of the volume's upper half, to its last slice), and no pixel
    // there moves by more than the renderer's own run-to-run rounding (a level).
    let vol = on.lit_fog.integrated_bytes(&ctx).expect("the integrated volume");
    let [cols, rows, slices] = eo_render::lit_fog::LEVELS[0];
    let texel = |i: u32, j: u32, k: u32| {
        let o = (((k * rows + j) * cols + i) * 8) as usize;
        [0, 1, 2].map(|c| half(u16::from_le_bytes([vol[o + 2 * c], vol[o + 2 * c + 1]])))
    };
    let lit_up = (0..rows / 2 - 1).flat_map(|j| (0..cols).map(move |i| (i, j))).filter(|&(i, j)| texel(i, j, slices - 1) != [0.0; 3]).count();
    assert_eq!(lit_up, 0, "rays that miss the lamp's light carry lit fog");
    let lit_down = (rows / 2 + 4..rows).flat_map(|j| (0..cols).map(move |i| (i, j))).filter(|&(i, j)| texel(i, j, slices - 1)[1] > 0.0).count();
    assert!(lit_down > 0, "the rays down into the lamp's light carry none");
    let worst = (0..H / 2 - 4).flat_map(|y| (0..W).map(move |x| (x, y))).map(|(x, y)| (luma(&on_above, x, y) - luma(&off_above, x, y)).abs()).fold(0.0, f64::max);
    assert!(worst <= 1.0, "rays that miss the lamp's light changed by {worst} levels");
    // (and below the horizon the lamp lights the fog under it)
    assert!(mean_luma(&on_above, 0..W, H / 2 + 8..H) > mean_luma(&off_above, 0..W, H / 2 + 8..H), "the fog under the lamp is not lit from above either");
}

#[test]
fn by_day_the_lamps_are_off_and_nothing_changes() {
    let ctx = ctx();
    // a clear noon: no lamp of any kind is lit, the passes do not run and the resolve adds nothing:
    // the frames are the frames without the feature (within the renderer's own run-to-run level).
    let close = |a: &[u8], b: &[u8]| a.iter().zip(b.iter()).all(|(p, q)| (*p as i32 - *q as i32).abs() <= 1);
    let off = renderer(&ctx, 0, 12.0, eo_sky::Kind::Clear);
    let on = renderer(&ctx, 2, 12.0, eo_sky::Kind::Clear);
    let (a0, a1) = the_lamp_views(&ctx, &off);
    let (b0, b1) = the_lamp_views(&ctx, &on);
    assert!(close(&a0, &b0) && close(&a1, &b1), "the lit fog changed a daytime frame");
    let s = on.lit_fog.stats();
    assert_eq!((s.frames, s.bytes), (0, 0), "the passes ran by day: {s:?}");
    assert!(s.skipped >= 24, "{s:?}");
    assert!(on.lit_fog.integrated_bytes(&ctx).is_none());
    // a foggy noon: vehicles drive with their lamps on (eo-sky night::vehicle_lamps), so the passes run,
    // but the street lamp is off: the volume holds no light at all.
    let on = renderer(&ctx, 1, 12.0, eo_sky::Kind::Fog);
    the_lamp_views(&ctx, &on);
    assert!(on.lit_fog.stats().frames > 0, "{:?}", on.lit_fog.stats());
    let vol = on.lit_fog.integrated_bytes(&ctx).unwrap();
    let lit = vol.chunks(8).filter(|t| t[..6].iter().any(|b| *b != 0)).count();
    assert_eq!(lit, 0, "a street lamp lit the fog by day");
}

#[test]
fn two_renders_of_the_same_frames_are_bit_equal() {
    // the lit fog's volume after the same frames on two renderers: bit for bit (its jitter is the frame
    // index's, its history the last frame's, no atomics). (The rest of the frame may differ by a level
    // in a few pixels between two renderers on its own: the run-to-run noise the occlusion check
    // records; the outputs are held to that.)
    let ctx = ctx();
    let a = renderer(&ctx, 1, 21.0, eo_sky::Kind::Fog);
    let b = renderer(&ctx, 1, 21.0, eo_sky::Kind::Fog);
    let (a0, a1) = the_lamp_views(&ctx, &a);
    let (b0, b1) = the_lamp_views(&ctx, &b);
    let (va, vb) = (a.lit_fog.integrated_bytes(&ctx).unwrap(), b.lit_fog.integrated_bytes(&ctx).unwrap());
    assert!(va == vb, "the lit fog's volume differs between two renders");
    for (x, y) in [(&a0, &b0), (&a1, &b1)] {
        let off = x.iter().zip(y.iter()).filter(|(p, q)| (**p as i32 - **q as i32).abs() > 1).count();
        assert!(off * 1000 < x.len(), "{off} bytes of the frames differ by more than a level between two renders");
    }
}

#[test]
fn a_moving_headlamp_leaves_no_trail() {
    // a headlamp 0.8 m up, 14 m ahead of the eye, moving across the view (+x) 0.25 m a frame
    // (15 m/s) for 40 frames, aimed ahead and partly toward the eye (its glow straight on), against
    // the same lamp standing at its last place for 40 frames: light the moving lamp's last frame has
    // over the standing lamp's (by more than the 8-bit rounding) is the trail it left.
    let ctx = ctx();
    let off = renderer(&ctx, 0, 21.0, eo_sky::Kind::Fog);
    let still = renderer(&ctx, 1, 21.0, eo_sky::Kind::Fog);
    let moving = renderer(&ctx, 1, 21.0, eo_sky::Kind::Fog);
    let cam = Camera::new(Vec3::new(0.0, 1.7, 0.0), 0.0, -0.05, W as f32 / H as f32);
    let at = |k: usize| Vec3::new(-10.0 + 0.25 * k as f32, 0.8, -14.0);
    let aim = Vec3::new(1.0, 0.0, 0.7);
    let last = 39;
    let mut out = vec![];
    for (r, moves) in [(&off, false), (&still, false), (&moving, true)] {
        let f = frame(&ctx);
        for k in 0..=last {
            r.look.sky.set_lamps(&ctx.queue, &[headlamp(at(if moves { k } else { last }), aim)]);
            draw(&ctx, r, &f, &cam, None);
        }
        out.push(read(&ctx, &f));
    }
    let (o, s, m) = (&out[0], &out[1], &out[2]);
    let (mut glow, mut trail, mut lag, mut worst) = (0.0, 0.0, 0.0, 0.0f64);
    for y in 0..H {
        for x in 0..W {
            let (lo, ls, lm) = (luma(o, x, y), luma(s, x, y), luma(m, x, y));
            glow += (ls - lo - 1.0).max(0.0); // the lit fog's light round the standing lamp
            trail += (lm - ls - 1.0).max(0.0); // light the moving lamp has where the standing one has none
            lag += (ls - lm - 1.0).max(0.0); // light the moving lamp has not reached yet
            worst = worst.max(lm - ls);
        }
    }
    println!("glow {glow:.0}, trail {trail:.0} ({:.1} %), lag {lag:.0} ({:.1} %), the largest excess {worst:.0} levels", 100.0 * trail / glow.max(1.0), 100.0 * lag / glow.max(1.0));
    assert!(glow > 500.0, "the headlamp lights no fog");
    assert!(trail < 0.05 * glow && worst < 8.0, "the moving beam leaves a trail ({:.1} % of its light, {worst} levels)", 100.0 * trail / glow);
    assert!(lag < 0.25 * glow, "the moving beam lags far behind ({:.1} % of its light)", 100.0 * lag / glow);
}

#[test]
fn a_moving_camera_keeps_the_glow_where_it_stands() {
    // the eye walking sideways 0.1 m a frame (6 m/s) past the lamp (the froxels' history reprojected each
    // frame), against an eye standing at its last place: their volumes agree, froxel by froxel,
    // as two stills taken there would (a wrong reprojection would drag the glow behind the eye).
    let ctx = ctx();
    let aspect = W as f32 / H as f32;
    let eye = |k: usize| Camera::new(Vec3::new(-2.0 + 0.1 * k as f32, 1.7, 0.0), 0.0, ((HEAD.y - 1.7) / 20.0).atan(), aspect);
    let last = 39;
    let mut vols = vec![];
    for moves in [false, true] {
        let r = renderer(&ctx, 1, 21.0, eo_sky::Kind::Fog);
        r.look.sky.set_lamps(&ctx.queue, &[street_lamp(HEAD)]);
        let f = frame(&ctx);
        for k in 0..=last {
            draw(&ctx, &r, &f, &eye(if moves { k } else { last }), None);
        }
        let v = r.lit_fog.integrated_bytes(&ctx).unwrap();
        vols.push(v.chunks(8).map(|t| half(u16::from_le_bytes([t[2], t[3]]))).collect::<Vec<f32>>()); // green
    }
    let (sum, diff) = vols[0].iter().zip(&vols[1]).fold((0.0f64, 0.0f64), |(s, d), (a, b)| (s + *a as f64, d + (*a - *b).abs() as f64));
    println!("standing vs walking eye: the volume's green {sum:.1}, differing by {diff:.2} ({:.2} %)", 100.0 * diff / sum);
    assert!(sum > 0.0);
    // (a standing eye's own volume moves by ~3.5 % from one frame to the next with the depth jitter;
    // without the history giving way as the view turns, the walking eye's differed by 12 %, its glow
    // leaning 25 % toward where the eye had been)
    assert!(diff < 0.05 * sum, "the walking eye's glow differs by {:.1} % from the standing eye's", 100.0 * diff / sum);
}

#[test]
fn the_lit_fog_slot_is_timed_and_empty_when_off() {
    let ctx = ctx();
    if !ctx.timestamps {
        println!("no timestamp queries on this adapter: skipped");
        return;
    }
    let n = lighting::QUERIES_PER_FRAME;
    let mut spans = vec![];
    for (level, hour, weather) in [(1, 21.0, eo_sky::Kind::Fog), (0, 21.0, eo_sky::Kind::Fog), (1, 12.0, eo_sky::Kind::Clear)] {
        let r = renderer(&ctx, level, hour, weather);
        r.look.sky.set_lamps(&ctx.queue, &[street_lamp(HEAD)]);
        let f = frame(&ctx);
        let cam = Camera::new(Vec3::new(0.0, 1.7, 0.0), 0.0, 0.3, W as f32 / H as f32);
        // a fresh set each case: a slot no pass wrote resolves to 0.
        let set = ctx.device.create_query_set(&wgpu::QuerySetDescriptor { label: None, ty: wgpu::QueryType::Timestamp, count: n });
        let buf = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: n as u64 * 8, usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let rb = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: n as u64 * 8, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let scope = ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
        for _ in 0..3 {
            draw(&ctx, &r, &f, &cam, Some(&set));
        }
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        enc.resolve_query_set(&set, 0..n, &buf, 0);
        enc.copy_buffer_to_buffer(&buf, 0, &rb, 0, buf.size());
        ctx.queue.submit([enc.finish()]);
        let t: Vec<u64> = read_mapped(&ctx, &rb).unwrap().chunks(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect();
        let err = pollster::block_on(scope.pop());
        assert!(err.is_none(), "level {level} at {hour} h: validation error {err:?}");
        let q = lighting::Q_LIT_FOG as usize;
        spans.push(t[q + 1].wrapping_sub(t[q]));
        println!("level {level} at {hour} h ({weather:?}): lit fog span {} ticks, {:?}", spans.last().unwrap(), r.lit_fog.stats());
    }
    // the passes take time at night; with nothing to do (off, or a clear noon) the slot is an empty span.
    assert!(spans[0] > 0 && spans[1] == 0 && spans[2] == 0, "{spans:?}");
}

/// An IEEE half (the volume's texels).
fn half(h: u16) -> f32 {
    let (s, e, m) = ((h >> 15) as u32, ((h >> 10) & 0x1f) as u32, (h & 0x3ff) as u32);
    let v = if e == 0 { m as f32 * 2f32.powi(-24) } else if e == 31 { f32::INFINITY } else { (1.0 + m as f32 / 1024.0) * 2f32.powi(e as i32 - 15) };
    if s == 1 { -v } else { v }
}
