//! the two-phase hierarchical-Z cull on a planted scene. A wall
//! in front of the camera hides three boxes; one box peeks over the wall's
//! top, one straddles its top edge, one stands in front of it. After two
//! frames the hidden boxes are not drawn, every box with a pixel is, the
//! counters agree, and a box that comes out from behind the wall is drawn in
//! the same frame (no pop). Without MSAA and with 4x MSAA (the pyramid reads
//! every sample). Needs a GPU.

use eo_render::camera::Camera;
use eo_render::scene::{GpuObject, ObjectKind, Vertex};
use eo_render::{GpuContext, GpuOptions, IDS_FORMAT, Renderer, RendererInit, Targets, authored::GpuMaterial, read_mapped, wgpu};
use glam::Vec3;
use std::collections::HashSet;

fn ctx() -> GpuContext {
    let opts = GpuOptions::default();
    GpuContext::new(GpuContext::create_instance(&opts), None, opts).expect("a GPU adapter is required for the occlusion test")
}

/// A closed box (12 triangles) from `min` to `max`.
fn boxed(min: Vec3, max: Vec3, verts: &mut Vec<Vertex>, indices: &mut Vec<u32>, objects: &mut Vec<GpuObject>) -> u32 {
    let base = verts.len() as i32;
    for i in 0..8 {
        let p = Vec3::new(
            if i & 1 != 0 { max.x } else { min.x },
            if i & 2 != 0 { max.y } else { min.y },
            if i & 4 != 0 { max.z } else { min.z },
        );
        verts.push(Vertex { pos: p.to_array(), normal: 0, color: 0xff80_8080 });
    }
    let first = indices.len() as u32;
    // Faces: -x, +x, -y, +y, -z, +z.
    let faces = [[0, 2, 6, 4], [1, 5, 7, 3], [0, 4, 5, 1], [2, 3, 7, 6], [0, 1, 3, 2], [4, 6, 7, 5]];
    for f in faces {
        indices.extend_from_slice(&[f[0], f[1], f[2], f[0], f[2], f[3]]);
    }
    objects.push(GpuObject::new(ObjectKind::Building, min.to_array(), max.to_array(), first, 36, base));
    objects.len() as u32 - 1
}

struct Scene {
    verts: Vec<Vertex>,
    indices: Vec<u32>,
    objects: Vec<GpuObject>,
    wall: u32,
    front: u32,
    peek: u32,
    straddle: u32,
    hidden: [u32; 3],
}

fn scene() -> Scene {
    let (mut verts, mut indices, mut objects) = (vec![], vec![], vec![]);
    let v = Vec3::new;
    // Camera at (0, 2, 0) looking north (-z); the wall's top is 6 m up at 20 m.
    let wall = boxed(v(-40.0, 0.0, -20.5), v(40.0, 6.0, -20.0), &mut verts, &mut indices, &mut objects);
    let front = boxed(v(2.5, 1.5, -10.5), v(3.5, 2.5, -9.5), &mut verts, &mut indices, &mut objects);
    let peek = boxed(v(-0.5, 19.5, -60.5), v(0.5, 20.5, -59.5), &mut verts, &mut indices, &mut objects);
    let straddle = boxed(v(-0.5, 9.5, -40.5), v(0.5, 10.5, -39.5), &mut verts, &mut indices, &mut objects);
    let hidden = [
        boxed(v(-0.5, 4.5, -50.5), v(0.5, 5.5, -49.5), &mut verts, &mut indices, &mut objects),
        boxed(v(-8.5, 2.5, -80.5), v(-7.5, 3.5, -79.5), &mut verts, &mut indices, &mut objects),
        boxed(v(5.0, 6.0, -205.0), v(7.0, 10.0, -195.0), &mut verts, &mut indices, &mut objects),
    ];
    Scene { verts, indices, objects, wall, front, peek, straddle, hidden }
}

fn renderer(ctx: &GpuContext, s: &Scene, samples: u32, ids: bool) -> Renderer {
    let materials = [GpuMaterial::vertex_colour()];
    Renderer::from_init(
        ctx,
        &RendererInit {
            vertices: &s.verts,
            indices: &s.indices,
            objects: &s.objects,
            extra_vertices: 0,
            extra_indices: 0,
            materials: &materials,
            groups: &[],
            hlod_enter_m: 0.0,
            hlod_exit_m: 0.0,
            kit_layers: 0,
            kit_res: 1,
            color_format: wgpu::TextureFormat::Rgba8UnormSrgb,
            sample_count: samples,
            multiplier: 1,
            debug_ids: ids,
            blend_first_object: 0,
        },
    )
    .unwrap()
}

const W: u32 = 640;
const H: u32 = 360;

struct Frame {
    drawn: HashSet<u32>,
    counters: [u32; eo_render::COUNTER_WORDS],
    /// Objects with at least one pixel (when rendered with ids).
    pixels: HashSet<u32>,
}

fn frame(ctx: &GpuContext, r: &Renderer, cam: &Camera, samples: u32) -> Frame {
    let d = &ctx.device;
    let mk = |format, usage, n| {
        d.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: n,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    let color = mk(wgpu::TextureFormat::Rgba8UnormSrgb, wgpu::TextureUsages::RENDER_ATTACHMENT, 1);
    let ids = mk(IDS_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC, 1);
    let targets = Targets::new(d, W, H, wgpu::TextureFormat::Rgba8UnormSrgb, samples);
    let (cv, iv) = (color.create_view(&Default::default()), ids.create_view(&Default::default()));
    r.update(&ctx.queue, cam);
    let mut enc = d.create_command_encoder(&Default::default());
    let (attach, resolve) = match &targets.msaa {
        Some(m) => (m, Some(&cv)),
        None => (&cv, None),
    };
    r.encode_with_ids(&mut enc, attach, resolve, &targets.depth, r.debug_ids.then_some(&iv), None);
    ctx.queue.submit([enc.finish()]);
    let (counters, drawn) = r.drawn_readback(ctx).unwrap();
    let mut pixels = HashSet::new();
    if r.debug_ids {
        let row = (W * 8).div_ceil(256) * 256;
        let buf = d.create_buffer(&wgpu::BufferDescriptor { label: None, size: (row * H) as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut enc = d.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            ids.as_image_copy(),
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(H) } },
            wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        );
        ctx.queue.submit([enc.finish()]);
        let bytes = read_mapped(ctx, &buf).unwrap();
        for y in 0..H as usize {
            for x in 0..W as usize {
                let o = y * row as usize + x * 8;
                let id = u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
                if id != u32::MAX {
                    pixels.insert(id);
                }
            }
        }
    }
    Frame { drawn: drawn.into_iter().collect(), counters, pixels }
}

fn check(samples: u32) {
    let ctx = ctx();
    if !ctx.compact_lists() {
        panic!("the occlusion test needs the compacted lists (occlusion is off without them)");
    }
    let s = scene();
    let ids = samples == 1;
    let r = renderer(&ctx, &s, samples, ids);
    assert!(r.occlusion_on(), "occlusion is on by default with the compacted lists");
    let cam = Camera::new(Vec3::new(0.0, 2.0, 0.0), 0.0, 0.0, W as f32 / H as f32);
    // Frame 1 is cold (no early depth: everything drawn); frame 2's early pass
    // draws everything frame 1 kept, and its late test finds the hidden boxes.
    let f1 = frame(&ctx, &r, &cam, samples);
    assert_eq!(f1.drawn.len(), s.objects.len(), "a cold frame draws everything in the frustum");
    let _f2 = frame(&ctx, &r, &cam, samples);
    let f3 = frame(&ctx, &r, &cam, samples);
    for o in [s.wall, s.front, s.peek, s.straddle] {
        assert!(f3.drawn.contains(&o), "object {o} is visible but was not drawn (samples {samples})");
    }
    for o in s.hidden {
        assert!(!f3.drawn.contains(&o), "object {o} is behind the wall but was drawn (samples {samples})");
    }
    assert_eq!(f3.counters[11], s.objects.len() as u32, "objects in the frustum");
    assert_eq!(f3.counters[14], 3, "objects found occluded");
    assert_eq!(f3.counters[13], 4, "objects drawn early (visible last frame)");
    if ids {
        // Ground truth: every object with a pixel was drawn.
        assert!(f3.pixels.is_subset(&f3.drawn), "pixels of objects not drawn: {:?}", f3.pixels.difference(&f3.drawn).collect::<Vec<_>>());
        for o in [s.wall, s.front, s.peek, s.straddle] {
            assert!(f3.pixels.contains(&o), "planted visible object {o} has no pixel: the scene is wrong");
        }
    }
    // Rise above the wall: the hidden boxes become visible and are drawn in
    // that same frame by the late pass (their flags still say hidden).
    let up = Camera::new(Vec3::new(0.0, 40.0, 0.0), 0.0, -0.45, W as f32 / H as f32);
    let f4 = frame(&ctx, &r, &up, samples);
    for o in s.hidden {
        assert!(f4.drawn.contains(&o), "object {o} came into view but was not drawn in that frame (samples {samples})");
    }
    if ids {
        assert!(f4.pixels.is_subset(&f4.drawn));
        for o in s.hidden {
            assert!(f4.pixels.contains(&o), "object {o} should be visible from above: the scene is wrong");
        }
    }
    // Off: the frustum alone.
    r.set_occlusion(false);
    let f5 = frame(&ctx, &r, &cam, samples);
    assert_eq!(f5.drawn.len(), s.objects.len(), "occlusion off draws everything in the frustum");
}

#[test]
fn hidden_boxes_are_culled_and_visible_ones_drawn() {
    check(1);
}

#[test]
fn hidden_boxes_are_culled_with_msaa() {
    check(4);
}
