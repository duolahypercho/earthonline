//! The city's opaque pass split by class (lib.rs `CityClasses`): a street
//! surface, a landmark's painted steel and a plain box, each drawn by its
//! class's pipeline from its own list, give the frame the one pipeline gives,
//! bit for bit, with and without the two-phase occlusion. Needs a GPU.

use eo_render::authored::GpuMaterial;
use eo_render::camera::Camera;
use eo_render::look::LookOptions;
use eo_render::scene::{GpuObject, ObjectKind, Vertex};
use eo_render::{CityClasses, GpuContext, GpuOptions, Renderer, RendererInit, Targets, read_mapped, wgpu};
use glam::Vec3;

const W: u32 = 192;
const H: u32 = 108;

fn ctx() -> GpuContext {
    let opts = GpuOptions::default();
    GpuContext::new(GpuContext::create_instance(&opts), None, opts).expect("a GPU adapter is required")
}

fn quad_box(min: Vec3, max: Vec3, kind: ObjectKind, material: u32, verts: &mut Vec<Vertex>, idx: &mut Vec<u32>, objects: &mut Vec<GpuObject>) {
    let base = verts.len() as i32;
    let first = idx.len() as u32;
    for i in 0..8 {
        let p = Vec3::new(if i & 1 != 0 { max.x } else { min.x }, if i & 2 != 0 { max.y } else { min.y }, if i & 4 != 0 { max.z } else { min.z });
        verts.push(Vertex { pos: p.to_array(), normal: 0, color: 0xff80_8080 });
    }
    for f in [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]] {
        idx.extend_from_slice(&[f[0], f[1], f[2], f[0], f[2], f[3]]);
    }
    let mut o = GpuObject::new(kind, min.to_array(), max.to_array(), first, 36, base);
    o.material = material;
    objects.push(o);
}

/// Ground, a plain box (material 0), a street slab (the street material, 1)
/// and a landmark box in painted steel (2), seen from above.
fn scene(ctx: &GpuContext, classes: bool) -> Renderer {
    CityClasses::with(classes, || {
        let (mut v, mut i, mut o) = (vec![], vec![], vec![]);
        quad_box(Vec3::new(-60.0, -1.0, -60.0), Vec3::new(60.0, 0.0, 60.0), ObjectKind::Terrain, 0, &mut v, &mut i, &mut o);
        quad_box(Vec3::new(-14.0, 0.0, -2.0), Vec3::new(-8.0, 10.0, 2.0), ObjectKind::Building, 0, &mut v, &mut i, &mut o);
        quad_box(Vec3::new(-4.0, 0.0, -6.0), Vec3::new(4.0, 0.2, 6.0), ObjectKind::Street, 1, &mut v, &mut i, &mut o);
        quad_box(Vec3::new(8.0, 0.0, -2.0), Vec3::new(14.0, 8.0, 2.0), ObjectKind::Landmark, 2, &mut v, &mut i, &mut o);
        let street = eo_render::street::street_material();
        let mut steel = GpuMaterial::vertex_colour();
        steel.flags |= 1 << eo_render::landmarks::SURFACE_SHIFT; // surface kind 1, painted steel
        let materials = [GpuMaterial::vertex_colour(), street, steel];
        let mut r = Renderer::from_init(
            ctx,
            &RendererInit {
                vertices: &v,
                indices: &i,
                objects: &o,
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
        .unwrap();
        // without the occlusion pass's screen-space term, which differs between two identical frames on its own.
        let opts = LookOptions { ao: false, ..r.look.options };
        r.look.set_options(&ctx.device, opts);
        r.look.set_time_of_day(15.0);
        r
    })
}

/// Three frames (the occlusion's early list needs one first); the HDR target's bytes.
fn render(ctx: &GpuContext, r: &Renderer, occlusion: bool) -> Vec<u8> {
    let d = &ctx.device;
    r.set_occlusion(occlusion);
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
    let cam = Camera::new(Vec3::new(0.0, 40.0, 25.0), 0.0, -1.0, W as f32 / H as f32);
    for _ in 0..3 {
        r.update(&ctx.queue, &cam);
        let mut enc = d.create_command_encoder(&Default::default());
        r.encode(&mut enc, &ov, None, &targets.depth, None);
        ctx.queue.submit([enc.finish()]);
    }
    let hdr = r.look.hdr_target(W, H);
    let row = (W * 8).div_ceil(256) * 256;
    let buf = d.create_buffer(&wgpu::BufferDescriptor { label: None, size: (row * H) as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let mut enc = d.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        hdr.texture().as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(H) } },
        wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
    );
    ctx.queue.submit([enc.finish()]);
    let raw = read_mapped(ctx, &buf).unwrap();
    raw.chunks(row as usize).flat_map(|r| r[..(W * 8) as usize].to_vec()).collect()
}

#[test]
fn the_split_classes_draw_the_frame_the_one_pipeline_draws() {
    let ctx = ctx();
    if !ctx.compaction() {
        return; // (the classes split with the compacted lists only)
    }
    let scope = ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let one = scene(&ctx, false);
    let split = scene(&ctx, true);
    for occlusion in [false, true] {
        let a = render(&ctx, &one, occlusion);
        let b = render(&ctx, &split, occlusion);
        let n = a.chunks(8).zip(b.chunks(8)).filter(|(p, q)| p != q).count();
        assert_eq!(n, 0, "occlusion {occlusion}: {n} HDR pixels differ with the classes split");
        assert!(a.chunks(8).any(|p| p.iter().any(|&x| x != 0)), "the frame is drawn");
    }
    // every object drawn, each by its own class's list (the street and the landmark in theirs)
    let drawn = split.cull_readback(&ctx, &Camera::new(Vec3::new(0.0, 40.0, 25.0), 0.0, -1.0, W as f32 / H as f32)).unwrap();
    let c = eo_render::CLASS_WORD as usize;
    assert_eq!((drawn.counters[0], drawn.counters[c], drawn.counters[c + 2]), (2, 1, 1), "general, street, landmark");
    assert_eq!(drawn.draws.len(), 4);
    if let Some(e) = pollster::block_on(scope.pop()) {
        panic!("validation error: {e}");
    }
}
