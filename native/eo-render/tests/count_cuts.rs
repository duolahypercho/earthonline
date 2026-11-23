//! The count emulation's fresh window on a cut (`counted::CutWindow`): once
//! a cut's recovery is over, the city lists are drawn to the new view's counts,
//! not to the view before the cut's (which, with `--no-cut-window`, bound them
//! until `WINDOW` read-backs had pushed them out; seen here without the
//! motion-aware bounds, which in a still view draw the early list to an exact
//! bound instead). The cut's own frame, whose late pass finds nearly everything
//! in view, joins neither window (seen in the late list's own reserve, with the
//! motion-aware bounds). No list overflows in any case. The occlusion turning
//! off is a cut too: behind a wall, the occluded frames' counts bound none of
//! the frames drawn without the occlusion, which before overflowed. Needs a
//! GPU; the emulation is forced on adapters with the indirect count.

use eo_render::camera::Camera;
use eo_render::counted::{CutWindow, FULL_FRAMES, MARGIN, MotionBounds, RING, Stats, WINDOW};
use eo_render::scene::{GpuObject, ObjectKind, Vertex};
use eo_render::{GpuContext, GpuOptions, Renderer, RendererInit, Targets, authored::GpuMaterial, wgpu};
use glam::Vec3;

const W: u32 = 320;
const H: u32 = 180;

fn ctx() -> GpuContext {
    let opts = GpuOptions { force_no_indirect_count: true, ..GpuOptions::default() };
    GpuContext::new(GpuContext::create_instance(&opts), None, opts).expect("a GPU adapter is required for the count cut test")
}

/// A closed box (12 triangles) from `min` to `max`.
fn boxed(min: Vec3, max: Vec3, verts: &mut Vec<Vertex>, indices: &mut Vec<u32>, objects: &mut Vec<GpuObject>) {
    let base = verts.len() as i32;
    for i in 0..8 {
        let p = Vec3::new(if i & 1 != 0 { max.x } else { min.x }, if i & 2 != 0 { max.y } else { min.y }, if i & 4 != 0 { max.z } else { min.z });
        verts.push(Vertex { pos: p.to_array(), normal: 0, color: 0xff80_8080 });
    }
    let first = indices.len() as u32;
    for f in [[0, 2, 6, 4], [1, 5, 7, 3], [0, 4, 5, 1], [2, 3, 7, 6], [0, 1, 3, 2], [4, 6, 7, 5]] {
        indices.extend_from_slice(&[f[0], f[1], f[2], f[0], f[2], f[3]]);
    }
    objects.push(GpuObject::new(ObjectKind::Building, min.to_array(), max.to_array(), first, 36, base));
}

/// A small box (0.4 m) at `c`.
fn small(c: Vec3) -> (Vec3, Vec3) {
    (c - Vec3::splat(0.2), c + Vec3::splat(0.2))
}

/// 1200 small boxes north of the origin (in view from `A`, behind `B`), and
/// 3000 east of both views and between them (behind both: list capacity only).
fn renderer(ctx: &GpuContext) -> Renderer {
    let near = (0..1200).map(|k| small(Vec3::new(-15.0 + (k % 40) as f32 * 0.75, 0.5 + (k / 40 % 6) as f32 * 0.75, -12.0 - (k / 240) as f32 * 6.0)));
    let aside = (0..3000).map(|k| small(Vec3::new(600.0 + (k % 50) as f32, 2.0, 100.0 + (k / 50) as f32 * 0.5)));
    from_boxes(ctx, near.chain(aside))
}

/// A wall 10 m north of the origin filling `A`'s view, 2000 small boxes behind
/// it (hidden with the occlusion, drawn without it), and 1000 aside (capacity).
fn walled(ctx: &GpuContext) -> Renderer {
    let wall = std::iter::once((Vec3::new(-40.0, -10.0, -10.5), Vec3::new(40.0, 30.0, -10.0)));
    let behind = (0..2000).map(|k| small(Vec3::new(-15.0 + (k % 40) as f32 * 0.75, 0.5 + (k / 40 % 5) as f32 * 0.75, -20.0 - (k / 200) as f32 * 4.0)));
    let aside = (0..1000).map(|k| small(Vec3::new(600.0 + (k % 50) as f32, 2.0, 100.0 + (k / 50) as f32 * 0.5)));
    from_boxes(ctx, wall.chain(behind).chain(aside))
}

/// A renderer of the boxes (`min`, `max`) alone.
fn from_boxes(ctx: &GpuContext, boxes: impl Iterator<Item = (Vec3, Vec3)>) -> Renderer {
    let (mut verts, mut indices, mut objects) = (vec![], vec![], vec![]);
    for (min, max) in boxes {
        boxed(min, max, &mut verts, &mut indices, &mut objects);
    }
    let materials = [GpuMaterial::vertex_colour()];
    Renderer::from_init(
        ctx,
        &RendererInit {
            vertices: &verts,
            indices: &indices,
            objects: &objects,
            extra_vertices: 0,
            extra_indices: 0,
            materials: &materials,
            groups: &[],
            hlod_enter_m: 0.0,
            hlod_exit_m: 0.0,
            kit_layers: 0,
            kit_res: 1,
            color_format: wgpu::TextureFormat::Rgba8UnormSrgb,
            sample_count: 1,
            multiplier: 1,
            debug_ids: false,
            blend_first_object: 0,
        },
    )
    .unwrap()
}

/// The depth target and a colour view to draw into.
fn target(ctx: &GpuContext) -> (Targets, wgpu::TextureView) {
    let targets = Targets::new(&ctx.device, W, H, wgpu::TextureFormat::Rgba8UnormSrgb, 1);
    let color = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = color.create_view(&Default::default());
    (targets, view)
}

/// The city lists' stats now.
fn city(r: &Renderer) -> Stats {
    r.counts.as_ref().unwrap().stats().into_iter().find(|s| s.label == "city").unwrap()
}

/// One frame from `cam`, waited for, as two frames in flight would let it land.
fn frame(ctx: &GpuContext, r: &Renderer, targets: &Targets, view: &wgpu::TextureView, cam: &Camera) {
    r.update(&ctx.queue, cam);
    let mut enc = ctx.device.create_command_encoder(&Default::default());
    r.encode(&mut enc, view, None, &targets.depth, None);
    ctx.queue.submit([enc.finish()]);
    ctx.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
}

/// The city lists' stats with a window full of `A`, after a cut to `B` (drawn at once, as a
/// teleport in play), and after a cut back to `A` (updated without a frame first, as the
/// bench's settle at a path's start).
fn cuts(ctx: &GpuContext, r: &Renderer, targets: &Targets, view: &wgpu::TextureView) -> (Stats, Stats, Stats) {
    let aspect = W as f32 / H as f32;
    // `A` looks north at the boxes; `B`, 200 m south, looks south at nothing (a cut: past COUNT_CUT_M, and a half turn).
    let a = Camera::new(Vec3::new(0.0, 2.0, 0.0), 0.0, 0.0, aspect);
    let b = Camera::new(Vec3::new(0.0, 2.0, 200.0), std::f32::consts::PI, 0.0, aspect);
    // frames past a cut by which its recovery is over and its read-backs are in.
    let settled = FULL_FRAMES as usize + 2 * RING;
    for _ in 0..30 + WINDOW {
        frame(ctx, r, targets, view, &a);
    }
    let at_a = city(r);
    for _ in 0..settled {
        frame(ctx, r, targets, view, &b);
    }
    let at_b = city(r);
    for _ in 0..20 {
        r.update(&ctx.queue, &a);
    }
    for _ in 0..settled {
        frame(ctx, r, targets, view, &a);
    }
    (at_a, at_b, city(r))
}

#[test]
fn a_cut_starts_the_read_back_window_afresh() {
    let ctx = ctx();
    if !ctx.count_emulation {
        eprintln!("count_cuts: no count emulation on this adapter (--no-count-emulation); nothing to check");
        return;
    }
    let (targets, view) = target(&ctx);
    for motion in [false, true] {
        for on in [true, false] {
            let r = MotionBounds::with(motion, || CutWindow::with(on, || renderer(&ctx)));
            let (at_a, at_b, back) = cuts(&ctx, &r, &targets, &view);
            println!(
                "motion bounds {motion}, cut window {on}: at A bounds {:?}; at B bounds {:?}; back at A bounds {:?}; peak counts {:?}; overflows {}, steady frames {}, chained frames {}",
                at_a.bounds, at_b.bounds, back.bounds, back.peak_counts, back.overflows, back.steady_frames, back.chained_frames
            );
            assert!(at_a.peak_counts[0] > 1000 && at_a.bounds[0] > 1500, "A's boxes are in the window: {at_a:?}");
            assert_eq!(back.overflows, 0, "no city list went past its bound (motion bounds {motion}, cut window {on}): {back:?}");
            match (motion, on) {
                // `B` sees nothing: every list at the margin alone, `A`'s counts gone.
                (false, true) => assert!(at_b.bounds.iter().all(|b| *b <= MARGIN), "after the cut the window holds B alone: {at_b:?}"),
                // before, `A`'s read-backs bound `B` until the window pushed them out.
                (false, false) => assert!(at_b.bounds[0] >= at_a.bounds[0] * 9 / 10, "without the fresh window A's counts stay: at A {at_a:?}, at B {at_b:?}"),
                // back at `A` the late list takes its own reserve again: the cut's own frame,
                // whose late pass found every box, is in no window.
                (true, true) => assert!(back.bounds[2] <= MARGIN, "the cut's own frame is not in the new window: {back:?}"),
                // before, the cut's own frame set the late list's own reserve.
                (true, false) => assert!(back.bounds[2] > at_a.bounds[0] / 2, "without the fresh window the cut's own frame stays: {back:?}"),
            }
        }
    }
}

#[test]
fn turning_the_occlusion_off_is_a_cut() {
    let ctx = ctx();
    if !ctx.count_emulation {
        eprintln!("count_cuts: no count emulation on this adapter (--no-count-emulation); nothing to check");
        return;
    }
    let (targets, view) = target(&ctx);
    let a = Camera::new(Vec3::new(0.0, 2.0, 0.0), 0.0, 0.0, W as f32 / H as f32);
    let settled = FULL_FRAMES as usize + 2 * RING;
    for on in [true, false] {
        let r = CutWindow::with(on, || walled(&ctx));
        // a window of occluded frames: the wall and nothing behind it.
        for _ in 0..30 + WINDOW {
            frame(&ctx, &r, &targets, &view, &a);
        }
        let occluded = city(&r);
        // without the occlusion every box behind the wall draws, and with it again.
        r.set_occlusion(false);
        for _ in 0..settled {
            frame(&ctx, &r, &targets, &view, &a);
        }
        let open = city(&r);
        r.set_occlusion(true);
        for _ in 0..settled {
            frame(&ctx, &r, &targets, &view, &a);
        }
        let back = city(&r);
        println!(
            "cut window {on}: occluded bounds {:?}, peak counts {:?}; without the occlusion bounds {:?}, peak counts {:?}, overflows {}; with it again bounds {:?}, overflows {}",
            occluded.bounds, occluded.peak_counts, open.bounds, open.peak_counts, open.overflows, back.bounds, back.overflows
        );
        // (a peak count is over the renderer's life: its first frame, with no history, drew every box)
        assert!(occluded.bounds[0] < MARGIN + 100 && occluded.overflows == 0, "the wall hides the boxes: {occluded:?}");
        assert!(open.bounds[0] >= 2000, "without the occlusion the boxes behind the wall draw: {open:?}");
        if on {
            assert_eq!(back.overflows, 0, "turning the occlusion off or on is a cut: no list went past its bound: {back:?}");
        } else {
            assert!(open.overflows >= 1, "before, the occluded frames' window bounded the first frames without the occlusion: {open:?}");
        }
    }
}
