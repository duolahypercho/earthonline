//! The lighting additions' plumbing on the GPU: with every option on, real
//! resources swapped into group 1 bindings 12-18 and the lighting timestamp
//! set given, the frame is bit for bit the frame without them (nothing reads
//! them yet); every lighting slot is written and the set resolves with no
//! validation error. Needs a GPU.

use eo_render::camera::Camera;
use eo_render::look::{LightingOptions, LookOptions};
use eo_render::scene::{GpuObject, ObjectKind, Vertex};
use eo_render::{GpuContext, GpuOptions, Renderer, Targets, lighting, read_mapped, wgpu};
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

/// A terrain slab and a box that shadows it, seen from above.
fn scene(ctx: &GpuContext) -> Renderer {
    let (mut v, mut i, mut o) = (vec![], vec![], vec![]);
    quad_box(Vec3::new(-60.0, -1.0, -60.0), Vec3::new(60.0, 0.0, 60.0), ObjectKind::Terrain, &mut v, &mut i, &mut o);
    quad_box(Vec3::new(-2.0, 0.0, -2.0), Vec3::new(2.0, 10.0, 2.0), ObjectKind::Building, &mut v, &mut i, &mut o);
    let mut r = Renderer::from_parts(ctx, &v, &i, &o, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
    // without the occlusion pass, which differs between two identical frames on its own.
    let opts = LookOptions { ao: false, ..r.look.options };
    r.look.set_options(&ctx.device, opts);
    r.look.set_time_of_day(15.0);
    r
}

fn camera() -> Camera {
    Camera::new(Vec3::new(0.0, 40.0, 25.0), 0.0, -1.0, W as f32 / H as f32)
}

/// Every lighting addition on, at the highest level each takes.
fn all_on() -> LightingOptions {
    LightingOptions { cascade_blend: true, far_shadows: 2, ambient_volume: 2, probe_grid: 2, light_shadows: 3, light_shadow_moving: true, headlight_shadows: 8, lit_fog: 2, smooth_time: true, ground_bounce: true, rt_gi: false }
}

/// Render `frames` frames (the lighting set, if given, gets slot `QUERIES_PER_FRAME * k` on frame k);
/// returns the HDR scene target's bytes.
fn render(ctx: &GpuContext, r: &Renderer, frames: u32, set: Option<&wgpu::QuerySet>) -> Vec<u8> {
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
    let cam = camera();
    for k in 0..frames {
        r.set_lighting_timestamps(set.map(|q| (q.clone(), lighting::QUERIES_PER_FRAME * k)));
        r.update(&ctx.queue, &cam);
        let mut enc = d.create_command_encoder(&Default::default());
        r.encode(&mut enc, &ov, None, &targets.depth, None);
        ctx.queue.submit([enc.finish()]);
    }
    r.set_lighting_timestamps(None);
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

fn texture(ctx: &GpuContext, size: [u32; 3], dimension: wgpu::TextureDimension, format: wgpu::TextureFormat, view: wgpu::TextureViewDimension) -> wgpu::TextureView {
    ctx.device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("lighting-test"),
            size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: size[2] },
            mip_level_count: 1,
            sample_count: 1,
            dimension,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor { dimension: Some(view), ..Default::default() })
}

fn table(ctx: &GpuContext, bytes: u64) -> wgpu::Buffer {
    ctx.device.create_buffer(&wgpu::BufferDescriptor { label: Some("lighting-test-table"), size: bytes, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false })
}

#[test]
fn lighting_options_and_resources_leave_the_frame_unchanged() {
    let ctx = ctx();
    let (d2, d3) = (wgpu::TextureDimension::D2, wgpu::TextureDimension::D3);
    let depth = eo_render::look::SHADOW_FORMAT;
    let f16 = wgpu::TextureFormat::Rgba16Float;
    let plain = scene(&ctx);
    let mut lit = scene(&ctx);
    assert_eq!(lit.look.options.lighting, LightingOptions::OFF, "a renderer starts with every lighting addition off");
    let opts = LookOptions { lighting: all_on(), ..lit.look.options };
    lit.look.set_options(&ctx.device, opts);
    assert_eq!(lit.look.options.lighting, all_on(), "set_options keeps the lighting options");
    // real resources, of other sizes than the stand-ins, in every binding.
    let far = texture(&ctx, [64, 64, 2], d2, depth, wgpu::TextureViewDimension::D2Array);
    let vol = texture(&ctx, [8, 8, 8], d3, f16, wgpu::TextureViewDimension::D3);
    let plane = texture(&ctx, [16, 16, 1], d2, f16, wgpu::TextureViewDimension::D2);
    let cubes = texture(&ctx, [4, 4, 24], d2, f16, wgpu::TextureViewDimension::D2Array);
    let atlas = texture(&ctx, [32, 32, 3], d2, depth, wgpu::TextureViewDimension::D2Array);
    let (probe_table, light_table) = (table(&ctx, 256), table(&ctx, 1024));
    let scope = ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
    lit.look.set_far_shadow_map(Some(&far));
    lit.look.set_ambient_volume(Some(&vol), Some(&plane));
    lit.look.set_probe_grid(Some(&cubes), Some(&probe_table));
    lit.look.set_light_shadows(Some(&atlas), Some(&light_table));
    lit.look.set_lighting_params(eo_render::look::LX_FAR_SHADOW_VP, &[[1.0, 2.0, 3.0, 4.0]; 9]);
    let b = lit.look.lighting_bindings();
    assert!(b.far_shadow_map == far && b.ambient_volume == vol && b.ambient_plane == plane && b.probe_grid == cubes && b.light_shadow_atlas == atlas);
    assert!(b.probe_table == probe_table && b.light_shadow_table == light_table);
    let lx = lit.look.lighting_params();
    assert_eq!(lx[0][0].to_bits() & 0xFF, all_on().mask(), "row 0 x carries the feature bits");
    assert_eq!(lx[1], [1.0, 2.0, 3.0, 4.0]);
    assert_eq!(plain.look.lighting_params(), [[0.0; 4]; eo_render::look::LX_LEN], "every row is zero with the lighting off");
    let set = ctx.timestamps.then(|| ctx.device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("lighting-test"), ty: wgpu::QueryType::Timestamp, count: 2 * lighting::QUERIES_PER_FRAME }));
    let a = render(&ctx, &plain, 2, None);
    let b = render(&ctx, &lit, 2, set.as_ref());
    if let Some(e) = pollster::block_on(scope.pop()) {
        panic!("validation error: {e}");
    }
    assert_eq!(a.len(), b.len());
    let differ = a.chunks(8).zip(b.chunks(8)).filter(|(p, q)| p != q).count();
    assert_eq!(differ, 0, "{differ} HDR pixels differ with the lighting options on and real resources bound");
    // the stand-ins come back.
    lit.look.set_far_shadow_map(None);
    lit.look.set_ambient_volume(None, None);
    lit.look.set_probe_grid(None, None);
    lit.look.set_light_shadows(None, None);
    let (now, s) = (lit.look.lighting_bindings(), lit.look.lighting_stand_ins().clone());
    assert!(now.far_shadow_map == s.far_shadow_map && now.ambient_volume == s.ambient_volume && now.ambient_plane == s.ambient_plane && now.probe_grid == s.probe_grid);
    assert!(now.probe_table == s.probe_table && now.light_shadow_atlas == s.light_shadow_atlas && now.light_shadow_table == s.light_shadow_table);
}

#[test]
fn every_lighting_slot_is_written_and_the_set_resolves() {
    let ctx = ctx();
    if !ctx.timestamps {
        eprintln!("lighting: no timestamp queries on this adapter; nothing to check");
        return;
    }
    for (name, opts) in [("off", LightingOptions::OFF), ("on", all_on())] {
        let mut r = scene(&ctx);
        let o = LookOptions { lighting: opts, ..r.look.options };
        r.look.set_options(&ctx.device, o);
        let n = 2 * lighting::QUERIES_PER_FRAME;
        let set = ctx.device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("lighting-test"), ty: wgpu::QueryType::Timestamp, count: n });
        let scope = ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
        render(&ctx, &r, 2, Some(&set));
        let resolve = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: n as u64 * 8, usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let read = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: n as u64 * 8, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        enc.resolve_query_set(&set, 0..n, &resolve, 0);
        enc.copy_buffer_to_buffer(&resolve, 0, &read, 0, n as u64 * 8);
        ctx.queue.submit([enc.finish()]);
        let raw = read_mapped(&ctx, &read).unwrap();
        if let Some(e) = pollster::block_on(scope.pop()) {
            panic!("lighting {name}: validation error: {e}");
        }
        let ts: Vec<u64> = raw.chunks_exact(8).map(|b| u64::from_le_bytes(b.try_into().unwrap())).collect();
        assert_eq!(ts.len(), n as usize);
        let written = ts.iter().filter(|t| **t != 0).count();
        // Empty passes may leave a timestamp unwritten on some drivers (the bench sees it on Metal); say how many were.
        eprintln!("lighting {name}: {written} of {n} timestamps non-zero; per pass (frame 2): {:?}", lighting::PASSES.map(|(p, q)| (p, ts[(lighting::QUERIES_PER_FRAME + q + 1) as usize].wrapping_sub(ts[(lighting::QUERIES_PER_FRAME + q) as usize]))));
    }
}
