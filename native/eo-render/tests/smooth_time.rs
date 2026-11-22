//! `--smooth-time` on the GPU renderer: the drawn hour moved every frame
//! through the soft path (`LookGpu::set_time_of_day_smooth`) refits the sun
//! cascades only when the light has turned, builds its keyframes off the
//! render thread, lets the probe finish its captures and changes the look a
//! little each frame; a turn of the camera moves the exposure a little each
//! frame where the default snaps it. Needs a GPU.

use eo_render::camera::Camera;
use eo_render::look::{LightingOptions, LookOptions};
use eo_render::scene::{GpuObject, ObjectKind, Vertex};
use eo_render::{GpuContext, GpuOptions, Renderer, Targets, smooth_time, wgpu};
use glam::Vec3;

fn ctx() -> GpuContext {
    let opts = GpuOptions::default();
    GpuContext::new(GpuContext::create_instance(&opts), None, opts).expect("a GPU adapter is required")
}

const W: u32 = 192;
const H: u32 = 108;

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

/// A terrain slab and a box, the sky stages on (the weather's frame, the exposure's
/// adaptation and meter), the occlusion pass off; `smooth` the option.
fn scene(ctx: &GpuContext, smooth: bool) -> Renderer {
    let (mut v, mut i, mut o) = (vec![], vec![], vec![]);
    quad_box(Vec3::new(-60.0, -1.0, -60.0), Vec3::new(60.0, 0.0, 60.0), ObjectKind::Terrain, &mut v, &mut i, &mut o);
    quad_box(Vec3::new(-2.0, 0.0, -2.0), Vec3::new(2.0, 10.0, 2.0), ObjectKind::Building, &mut v, &mut i, &mut o);
    let mut r = Renderer::from_parts(ctx, &v, &i, &o, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
    let opts = LookOptions { ao: false, sky: eo_render::sky::SkyOptions::default(), lighting: LightingOptions { smooth_time: smooth, ..LightingOptions::OFF }, ..r.look.options };
    r.look.set_options(&ctx.device, opts);
    // the dome-estimate meter this test was written for, not the scene meter
    // (on by default since the Golden Gate merge), which barely moves here.
    r.look.cam_switches.meter = false;
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

    /// One frame; returns the cascades it refreshes (bit c).
    fn draw(&self, ctx: &GpuContext, r: &Renderer, cam: &Camera) -> u32 {
        r.update(&ctx.queue, cam);
        let mask = r.look.encoded_shadow_mask();
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        r.encode(&mut enc, &self.out, None, &self.targets.depth, None);
        ctx.queue.submit([enc.finish()]);
        mask
    }
}

/// The numbers of the look in force that the look uniform takes.
fn numbers(r: &Renderer) -> Vec<f64> {
    let f = r.look.frame();
    let t = r.look.sky_terms();
    let mut v = vec![];
    v.extend(f.key_dir);
    v.extend(f.key);
    v.extend(f.rim_dir);
    v.extend(f.hemi_sky);
    v.extend(f.hemi_ground);
    v.extend(f.ambient);
    for c in &f.env_sh {
        v.extend(c);
    }
    v.extend([f.aerial.near, f.aerial.far, f.shadow_strength, f.sky_irradiance_up]);
    v.extend(f.model.zenith_radiance);
    v.extend(f.model.horizon_radiance);
    v.extend(f.model.ground_radiance);
    if let Some(t) = t {
        v.extend([t.exposure_target, t.lamps_on, t.vehicle_lamps, t.nightness, t.dusk, t.lamp_units_per_lux]);
    }
    v
}

#[test]
fn the_soft_path_moves_the_hour_without_refitting_every_frame() {
    let ctx = ctx();
    let frames = Frames::new(&ctx);
    let cam = Camera::new(Vec3::new(0.0, 40.0, 25.0), 0.0, -1.0, W as f32 / H as f32);
    // a clock running at 60 x across sunset: one clock minute (a keyframe) every 60 frames, 10 minutes.
    let (start, n) = (18.95, 600usize);
    let hour = |k: usize| start + k as f64 / 3600.0;
    let mut runs = vec![];
    for smooth in [false, true] {
        let r = scene(&ctx, smooth);
        r.look.set_time_of_day(start);
        for _ in 0..4 {
            frames.draw(&ctx, &r, &cam);
        }
        // frames refitting all three cascades: all of them, and those where the key turned under 0.05 deg since the frame before.
        let (mut all_three, mut idle_refits, mut active, mut seen) = (0usize, 0usize, 0usize, vec![]);
        let mut key = r.look.frame().key_dir;
        let mut worst_capture = 0u32;
        for k in 1..=n {
            if smooth {
                r.look.set_time_of_day_smooth(hour(k));
            } else {
                r.look.set_time_of_day(hour(k));
            }
            let mask = frames.draw(&ctx, &r, &cam);
            all_three += (mask == 0b111) as usize;
            let now = r.look.frame().key_dir;
            idle_refits += (mask == 0b111 && smooth_time::angle_deg(key, now) < 0.05) as usize;
            key = now;
            active += (mask != 0) as usize; // shadows drawn (the sun or the moon keys)
            seen.push(numbers(&r));
            worst_capture = worst_capture.max(r.probe.stats().last_capture_frames);
            if k % 60 == 0 {
                // the worker's keyframe gets its frames' worth of time, as a 60 Hz clock gives it.
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        let probe = r.probe.stats();
        runs.push((active, (all_three, idle_refits), seen, r.look.smooth_time_keys(), probe, worst_capture));
    }
    let (hard_active, (hard_all, _), hard_seen, _, hard_probe, _) = &runs[0];
    let (soft_active, (soft_all, soft_idle), soft_seen, (built, prefetched), soft_probe, soft_capture) = &runs[1];
    println!("frames refitting all three cascades: hard {hard_all} of {hard_active} with shadows, soft {soft_all} of {soft_active} ({soft_idle} with the key turning under 0.05 deg a frame); keyframes built on the render thread {built}, by the worker {prefetched}");
    println!("probe: hard {} captures ({} faces deferred), soft {} captures ({} deferred), soft longest capture {soft_capture} frames", hard_probe.captures, hard_probe.deferred, soft_probe.captures, soft_probe.deferred);
    assert!(*hard_active > n / 2 && *soft_active > n / 2, "shadows on {hard_active} / {soft_active} of {n} frames");
    assert!(*hard_all >= hard_active - 2, "the hard path refits every cascade every frame ({hard_all} of {hard_active})");
    // (at this 60 x clock the moon's handover turns the key a few degrees a frame for a clock minute: those frames refit, as they should)
    assert!(*soft_idle <= 1, "the soft path refitted every cascade on {soft_idle} frames where the light had hardly turned ({soft_all} in all)");
    assert!(*soft_all <= soft_active / 4, "the soft path refitted every cascade on {soft_all} of {soft_active} frames");
    assert!(*built + *prefetched >= 9 && *built <= 4, "keyframes: {built} on the render thread, {prefetched} from the worker");
    if soft_probe.captures > 0 {
        assert!(*soft_capture <= 6 * (smooth_time::PROBE_DEFER_MAX + 1) + 6, "a probe capture took {soft_capture} frames with the hour moving");
    } else {
        println!("probe: no capture on this adapter (no compacted lists); its bound is not checked");
    }
    // no step at a keyframe: the change into each frame that crosses a clock
    // minute (k a multiple of 60) is within 2 x the larger of the changes into
    // the frames either side, and no number moves by a fifth of its whole
    // move over the ten minutes in one frame.
    let len = soft_seen[0].len();
    let mut hard_steps = 0;
    for i in 0..len {
        let steps: Vec<f64> = soft_seen.windows(2).map(|w| (w[1][i] - w[0][i]).abs()).collect();
        let range = soft_seen.iter().map(|v| v[i]).fold(f64::MIN, f64::max) - soft_seen.iter().map(|v| v[i]).fold(f64::MAX, f64::min);
        if range <= 1e-7 {
            continue;
        }
        // steps[j] is the change into frame k = j + 2 (seen[0] is frame 1).
        for j in 1..steps.len() - 1 {
            if (j + 2) % 60 == 0 {
                let around = steps[j - 1].max(steps[j + 1]);
                assert!(steps[j] <= 2.0 * around + 1e-6 * range, "number {i}: a step of {} at the keyframe into frame {} (around it {around}, range {range})", steps[j], j + 2);
            }
        }
        let worst = steps.iter().cloned().fold(0.0, f64::max);
        assert!(worst <= range / 5.0, "number {i}: a step of {worst} in one frame (range {range})");
        let hard_worst = hard_seen.windows(2).map(|w| (w[1][i] - w[0][i]).abs()).fold(0.0, f64::max);
        if hard_worst > range / 4.0 {
            hard_steps += 1;
        }
    }
    println!("numbers that step by over a quarter of their range in one frame on the hard path: {hard_steps} of {len}");
}

#[test]
fn a_turn_moves_the_smooth_exposure_a_little_each_frame() {
    let ctx = ctx();
    let frames = Frames::new(&ctx);
    let mut gaps = vec![];
    for smooth in [false, true] {
        let r = scene(&ctx, smooth);
        // golden hour: the meter takes the exposure down looking into the low sun.
        r.look.set_time_of_day(18.5);
        let sun = r.look.frame().model.sun;
        let toward = (sun.x as f32).atan2(-(sun.z as f32)); // forward = (sin yaw, 0, -cos yaw)
        let eye = Vec3::new(0.0, 2.0, 30.0);
        let away = Camera::new(eye, toward + std::f32::consts::PI, 0.05, W as f32 / H as f32);
        let into = Camera::new(eye, toward, 0.05, W as f32 / H as f32);
        for _ in 0..120 {
            frames.draw(&ctx, &r, &away);
        }
        let before = r.look.sky.drawn_exposure().unwrap();
        let mut seen = vec![before];
        for _ in 0..120 {
            frames.draw(&ctx, &r, &into);
            seen.push(r.look.sky.drawn_exposure().unwrap());
        }
        let after = *seen.last().unwrap();
        let worst = seen.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f64::max);
        println!("smooth {smooth}: exposure {before:.4} looking away, {after:.4} into the sun; largest frame step {worst:.4}");
        gaps.push((before, after, worst));
    }
    let (b0, a0, w0) = gaps[0];
    let (b1, a1, w1) = gaps[1];
    let gap = (a0 - b0).abs();
    assert!(gap > 0.02 * b0, "the meter should move the exposure on this turn ({b0} -> {a0})");
    assert!(w0 >= 0.9 * gap, "the default snaps on a turn (largest step {w0} of {gap})");
    assert!((a1 - a0).abs() <= 0.02 * a0 && (b1 - b0).abs() <= 0.02 * b0, "both settle on the same exposures ({b1} -> {a1} against {b0} -> {a0})");
    assert!(w1 <= 0.1 * gap, "the smooth exposure stepped by {w1} of {gap} in one frame");
}
