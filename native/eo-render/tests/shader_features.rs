//! (stage P1) The opt-in features' shader code compiled out when they are
//! off (look.rs `ShaderFeatures`): a renderer whose pipelines carry none of
//! it draws the default frame bit for bit as one that carries all of it, and a
//! renderer carrying only the lighting additions it is asked for draws them
//! as one carrying every feature. Needs a GPU.

use eo_render::camera::Camera;
use eo_render::look::{self, LightingOptions, LookOptions, ShaderFeatures};
use eo_render::scene::{GpuObject, ObjectKind, Vertex};
use eo_render::{GpuContext, GpuOptions, Renderer, Targets, read_mapped, wgpu};
use glam::Vec3;

const W: u32 = 192;
const H: u32 = 108;

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
    let faces = [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]];
    for f in faces {
        idx.extend_from_slice(&[f[0], f[1], f[2], f[0], f[2], f[3]]);
    }
    objects.push(GpuObject::new(kind, min.to_array(), max.to_array(), first, 36, base));
}

/// A terrain slab and a box that shadows it, seen from above, its pipelines
/// made with `features` compiled in, its look options `lighting`.
fn scene(ctx: &GpuContext, features: ShaderFeatures, lighting: LightingOptions) -> Renderer {
    look::with_shader_features(features, || {
        let (mut v, mut i, mut o) = (vec![], vec![], vec![]);
        quad_box(Vec3::new(-60.0, -1.0, -60.0), Vec3::new(60.0, 0.0, 60.0), ObjectKind::Terrain, &mut v, &mut i, &mut o);
        quad_box(Vec3::new(-2.0, 0.0, -2.0), Vec3::new(2.0, 10.0, 2.0), ObjectKind::Building, &mut v, &mut i, &mut o);
        let mut r = Renderer::from_parts(ctx, &v, &i, &o, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
        // without the occlusion pass, which differs between two identical frames on its own.
        let opts = LookOptions { ao: false, lighting, ..r.look.options };
        r.look.set_options(&ctx.device, opts);
        r.look.set_time_of_day(15.0);
        r
    })
}

/// Two frames (pipelines made on demand are made in `features`' scope); the HDR target's bytes.
fn render(ctx: &GpuContext, r: &Renderer, features: ShaderFeatures) -> Vec<u8> {
    look::with_shader_features(features, || {
        let d = &ctx.device;
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
        for _ in 0..2 {
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
    })
}

fn differ(a: &[u8], b: &[u8]) -> usize {
    assert_eq!(a.len(), b.len());
    a.chunks(8).zip(b.chunks(8)).filter(|(p, q)| p != q).count()
}

#[test]
fn compiling_the_features_out_leaves_the_default_frame_unchanged() {
    let ctx = ctx();
    let scope = ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let full = scene(&ctx, ShaderFeatures::ALL, LightingOptions::OFF);
    let lean = scene(&ctx, ShaderFeatures::NONE, LightingOptions::OFF);
    let a = render(&ctx, &full, ShaderFeatures::ALL);
    let b = render(&ctx, &lean, ShaderFeatures::NONE);
    if let Some(e) = pollster::block_on(scope.pop()) {
        panic!("validation error: {e}");
    }
    let n = differ(&a, &b);
    assert_eq!(n, 0, "{n} HDR pixels differ with every feature compiled out");
    assert!(a.chunks(8).any(|p| p.iter().any(|&x| x != 0)), "the frame is drawn");
}

#[test]
fn a_feature_compiled_in_alone_draws_as_with_every_feature_in() {
    let ctx = ctx();
    let scope = ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
    // the lighting additions that need no attached resources (their stand-ins are bound).
    for lighting in [
        LightingOptions { cascade_blend: true, ..LightingOptions::OFF },
        LightingOptions { smooth_time: true, ..LightingOptions::OFF },
        LightingOptions { cascade_blend: true, far_shadows: 1, probe_grid: 1, light_shadows: 1, headlight_shadows: 2, lit_fog: 1, smooth_time: true, ..LightingOptions::OFF },
    ] {
        let own = ShaderFeatures::of(&LookOptions { lighting, ..LookOptions::default() }, false, false, false);
        assert_eq!(own.lx_bits, lighting.mask());
        let full = scene(&ctx, ShaderFeatures::ALL, lighting);
        let lean = scene(&ctx, own, lighting);
        let a = render(&ctx, &full, ShaderFeatures::ALL);
        let b = render(&ctx, &lean, own);
        let n = differ(&a, &b);
        assert_eq!(n, 0, "{lighting:?}: {n} HDR pixels differ with only its own code compiled in");
    }
    if let Some(e) = pollster::block_on(scope.pop()) {
        panic!("validation error: {e}");
    }
}
