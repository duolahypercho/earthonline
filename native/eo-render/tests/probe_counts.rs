//! The scene probe's faces in the count emulation (`probe::ProbeFaceCounts`):
//! a capture whose first face sees far more than the faces the read-back
//! window last saw is drawn to its own count, so its list never overflows
//! (an overflow recaptures and starts the recovery whose read-back the render
//! thread waits for). With `--no-probe-face-counts` the same capture
//! overflows, as before. Needs a GPU; the emulation is forced on adapters
//! with the indirect count.

use eo_render::camera::Camera;
use eo_render::scene::{GpuObject, ObjectKind, Vertex};
use eo_render::{GpuContext, GpuOptions, Renderer, RendererInit, Targets, authored::GpuMaterial, wgpu};
use glam::Vec3;

const W: u32 = 320;
const H: u32 = 180;

fn ctx() -> GpuContext {
    let opts = GpuOptions { force_no_indirect_count: true, ..GpuOptions::default() };
    GpuContext::new(GpuContext::create_instance(&opts), None, opts).expect("a GPU adapter is required for the probe count test")
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

/// 600 small boxes east of the probe (only its +X face sees them; that face
/// is a capture's first), nothing anywhere else.
fn renderer(ctx: &GpuContext) -> Renderer {
    let (mut verts, mut indices, mut objects) = (vec![], vec![], vec![]);
    for k in 0..600 {
        let c = Vec3::new(40.0 + (k / 20) as f32 * 4.0, 2.0, -9.5 + (k % 20) as f32 + 10.0);
        boxed(c - Vec3::splat(0.2), c + Vec3::splat(0.2), &mut verts, &mut indices, &mut objects);
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

/// One frame at `eye` (looking north), waited for, as two frames in flight would let it land.
fn frame(ctx: &GpuContext, r: &Renderer, targets: &Targets, view: &wgpu::TextureView, eye: Vec3) {
    r.update(&ctx.queue, &Camera::new(eye, 0.0, 0.0, W as f32 / H as f32));
    let mut enc = ctx.device.create_command_encoder(&Default::default());
    r.encode(&mut enc, view, None, &targets.depth, None);
    ctx.queue.submit([enc.finish()]);
    ctx.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
}

#[test]
fn a_capture_s_crowded_first_face_is_drawn_to_its_own_count() {
    let ctx = ctx();
    if !ctx.count_emulation {
        eprintln!("probe_counts: no count emulation on this adapter (--no-count-emulation); nothing to check");
        return;
    }
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
    for on in [true, false] {
        let r = eo_render::probe::ProbeFaceCounts::with(on, || renderer(&ctx));
        // a first capture at the origin, then long enough idle that the
        // read-back window holds only its last face (which sees nothing).
        let a = Vec3::new(0.0, 2.0, 0.0);
        for _ in 0..30 + eo_render::counted::WINDOW {
            frame(&ctx, &r, &targets, &view, a);
        }
        let first = r.probe.stats();
        assert_eq!(first.captures, 1, "the first capture finished (face counts {on}): {first:?}");
        // a second capture 20 m on (past the 16 m hysteresis): its +X face sees the 600 boxes again.
        let b = Vec3::new(0.0, 2.0, 20.0);
        for _ in 0..30 {
            frame(&ctx, &r, &targets, &view, b);
        }
        let p = r.probe.stats();
        let probe = r.counts.as_ref().unwrap().stats().into_iter().find(|s| s.label == "probe").unwrap();
        println!("face counts {on}: captures {}, count passes {}, faces waiting {}, faces without a count {}; probe list overflows {}, peak count {:?}, peak bound {:?}", p.captures, p.count_passes, p.count_waits, p.uncounted_faces, probe.overflows, probe.peak_counts, probe.peak_bounds);
        assert!(probe.peak_counts[0] >= 590, "the +X face drew the boxes: {probe:?}");
        if on {
            assert_eq!(probe.overflows, 0, "with the face counts no face went past its bound: {probe:?}");
            assert!(p.captures >= 2 && p.count_passes >= 2 && p.uncounted_faces == 0, "every capture's faces waited for their count pass: {p:?}");
        } else {
            assert!(probe.overflows >= 1, "from the read-back history alone the second capture's first face overflows: {probe:?}");
            assert_eq!(p.count_passes, 0, "{p:?}");
        }
    }
}
