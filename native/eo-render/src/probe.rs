//! the SCENE PROBE (src/render/environment-ibl.js `SCENE_PROBE_DEFAULTS`,
//! `bakeSceneCube`; how glass reads it: src/render/materials/glazing-environment.js).
//!
//! The JS bakes ONE 128 px cube of the city at standing height where the
//! viewer is (a 128 m anchor lattice with 16 m hysteresis), hides the display
//! sky and paints the analytic radiance as the background, prefilters it
//! (PMREM) and binds it as `scene.environment`: every material's specular
//! environment and, through the measured transfer, the diffuse yield the
//! light rig is solved against. Glass reads it never sharper than a 4-texel
//! footprint (roughness 0.152) and with the street-canyon parallax lift.
//!
//! Here (the form chosen for the 10 ms budget): one 128 px cube captured at
//! the viewer, re-captured when the viewer has moved more than the JS
//! hysteresis (16 m) from the capture point or the clock has moved a 0.25 h
//! bucket, ONE FACE PER FRAME (six frames a capture, nothing blocking):
//! the GPU-driven city drawn into the face (its own cull dispatch over the
//! same object table, at the LOD the camera's pass chose, terrain coarse,
//! nothing transparent, far 1,400 m as the JS), then on the sixth face the
//! analytic sky filled where nothing was drawn, 2 x 2 box levels (standing in
//! for the PMREM's GGX levels), and the cosine-lobe transfer about +Y
//! (`probeBakeTransfer`) reduced on the GPU and read back without waiting.
//! The finished cube becomes the front one (two cubes: shading never reads
//! the one being captured). Not a lattice: an anchor lattice over the whole
//! city puts nodes inside buildings; the capture point is the viewer itself.

use crate::camera::frustum_planes;
use glam::{Mat4, Vec3};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

pub const MIPS: u32 = 6;
/// Kit buildings past this distance from the capture point draw as their box
/// in the probe (the kit's near rung radius, `LOD_DISTANCE_M[0]`).
pub const PROBE_KIT_BOX_M: f32 = 65.0;
pub const PROBE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// The JS's probe numbers (data/facade-oracle.json `tables.probe`).
pub struct ProbeConstants {
    pub face_size: u32,
    pub near: f32,
    pub far: f32,
    pub hysteresis: f32,
    pub canyon_m: f32,
    pub glazing_floor: f32,
    pub fallback_yield: f64,
    pub yield_max: f64,
}

pub fn constants() -> ProbeConstants {
    let p = &crate::facade::oracle()["tables"]["probe"];
    ProbeConstants {
        face_size: p["faceSize"].as_u64().unwrap_or(128) as u32,
        near: p["near"].as_f64().unwrap_or(0.35) as f32,
        far: p["far"].as_f64().unwrap_or(1400.0) as f32,
        hysteresis: p["anchorHysteresis"].as_f64().unwrap_or(16.0) as f32,
        canyon_m: p["glazingCanyonMetres"].as_f64().unwrap_or(21.0) as f32,
        glazing_floor: p["glazingRoughnessFloor"].as_f64().unwrap_or(0.152) as f32,
        fallback_yield: p["environmentDiffuseYield"].as_f64().unwrap_or(0.038),
        yield_max: p["environmentYieldMax"].as_f64().unwrap_or(3.0),
    }
}

/// Face f: (forward, up); right = forward x up (probe_fill.wgsl `probe_basis`).
pub fn face_basis(f: u32) -> (Vec3, Vec3) {
    match f {
        0 => (Vec3::X, Vec3::Y),
        1 => (-Vec3::X, Vec3::Y),
        2 => (Vec3::Y, -Vec3::Z),
        3 => (-Vec3::Y, Vec3::Z),
        4 => (Vec3::Z, Vec3::Y),
        _ => (-Vec3::Z, Vec3::Y),
    }
}

pub fn face_view_proj(eye: Vec3, f: u32, near: f32, far: f32) -> Mat4 {
    let (fwd, up) = face_basis(f);
    let view = glam::camera::rh::view::look_to_mat4(eye, fwd, up);
    let proj = glam::camera::rh::proj::directx::perspective(90f32.to_radians(), 1.0, far, near);
    proj * view
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ProbeCull {
    planes: [[f32; 4]; 6],
    cam: [f32; 4],
    object_count: u32,
    kit_box_m: f32,
    /// the bounded list's guard (bounds.rs) reaches this far from the capture point.
    guard_reach: f32,
    pad: u32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ProbeStats {
    pub captures: u64,
    pub faces: u64,
    /// The last measured transfer (`probeBakeTransfer`: the capture's cosine
    /// lobe about +Y over the open sky's), when read back.
    pub transfer: Option<f64>,
    /// The capture's cosine-weighted mean radiance about +Y (luminance).
    pub lobe_radiance: Option<f64>,
    /// Frames from a capture's start to its cube in use.
    pub last_capture_frames: u32,
    /// faces moved off a frame that carried a heavy pass (`defer_this_frame`).
    pub deferred: u64,
}

struct State {
    /// Where the front cube was captured (None until the first capture finishes).
    front_pos: Option<Vec3>,
    front: usize,
    /// The capture in progress: its point, the next face, the frame it began.
    capturing: Option<(Vec3, u32, u64)>,
    hour: f64,
    /// the open sky's irradiance the front cube was captured under.
    sky_up: f64,
    frame: u64,
    /// The face this frame draws (decided in `update`, encoded in `encode`).
    this_frame: Option<(u32, Vec3)>,
    lobe_pending: bool,
    /// Hold the front cube: no capture starts or advances (tests comparing frames).
    frozen: bool,
    stats: ProbeStats,
}

pub struct ProbeGpu {
    pub c: ProbeConstants,
    device: wgpu::Device,
    raw_layers: Vec<wgpu::TextureView>,
    cubes: [wgpu::Texture; 2],
    /// Sampled views of each cube (all levels, 2D array).
    pub cube_views: [wgpu::TextureView; 2],
    depth: wgpu::TextureView,
    pub frame_buf: wgpu::Buffer,
    cull_buf: wgpu::Buffer,
    draws: wgpu::Buffer,
    counters: wgpu::Buffer,
    cull_pipeline: wgpu::ComputePipeline,
    /// the bounded list's count pass (bounds.rs) and its faces' planes.
    count_pipeline: wgpu::ComputePipeline,
    faces_buf: wgpu::Buffer,
    cull_bg: wgpu::BindGroup,
    fill_pipeline: wgpu::ComputePipeline,
    mip_pipeline: wgpu::ComputePipeline,
    lobe_pipeline: wgpu::ComputePipeline,
    fill_bgs: [wgpu::BindGroup; 2],
    mip_bgs: [Vec<wgpu::BindGroup>; 2],
    lobe_bgs: [wgpu::BindGroup; 2],
    /// The capture's lobe sums and irradiance SH9 (draw.wgsl binding 16).
    pub lobe_buf: wgpu::Buffer,
    lobe_read: wgpu::Buffer,
    lobe_mapped: Arc<AtomicU8>,
    pipeline: wgpu::RenderPipeline,
    pub sampler: wgpu::Sampler,
    state: Mutex<State>,
    object_count: u32,
    pub bytes: u64,
}

pub struct ProbeInit<'a> {
    pub object_buf: &'a wgpu::Buffer,
    pub group_buf: &'a wgpu::Buffer,
    pub object_count: u32,
    pub look_buf: &'a wgpu::Buffer,
    pub draw_module: &'a wgpu::ShaderModule,
    pub draw_layout: &'a wgpu::PipelineLayout,
    pub vertex_stride: u64,
    pub attrs: &'a [wgpu::VertexAttribute],
    pub hdr_format: wgpu::TextureFormat,
    pub depth_format: wgpu::TextureFormat,
}

impl ProbeGpu {
    pub fn new(d: &wgpu::Device, init: &ProbeInit) -> Self {
        let c = constants();
        let size = c.face_size;
        let tex = |label: &str, mips: u32, usage: wgpu::TextureUsages, format: wgpu::TextureFormat| {
            d.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: if format == init.depth_format { 1 } else { 6 } },
                mip_level_count: mips,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let raw = tex("probe-raw", 1, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, PROBE_FORMAT);
        let raw_layers: Vec<wgpu::TextureView> = (0..6)
            .map(|l| raw.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2), base_array_layer: l, array_layer_count: Some(1), ..Default::default() }))
            .collect();
        let raw_all = raw.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        let cubes = [0, 1].map(|i| tex(&format!("probe-cube-{i}"), MIPS, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING, PROBE_FORMAT));
        let cube_views = [0, 1].map(|i| cubes[i].create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() }));
        let level = |i: usize, m: u32| {
            cubes[i].create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), base_mip_level: m, mip_level_count: Some(1), ..Default::default() })
        };
        let depth_tex = tex("probe-depth", 1, wgpu::TextureUsages::RENDER_ATTACHMENT, init.depth_format);
        let depth = depth_tex.create_view(&Default::default());
        let buf = |label: &str, size: u64, usage: wgpu::BufferUsages| d.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size.max(16), usage, mapped_at_creation: false });
        let frame_buf = buf("probe-frame", 160, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
        let cull_buf = buf("probe-cull", std::mem::size_of::<ProbeCull>() as u64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
        let draws = buf("probe-draws", init.object_count.max(1) as u64 * 20, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST);
        let counters = buf("probe-counters", 48, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC);
        // the six faces' planes for the count pass (bounds.rs).
        let faces_buf = buf("probe-faces", 36 * 16, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
        let lobe_buf = buf("probe-lobe", 160, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC);
        let lobe_read = buf("probe-lobe-read", 16, wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST);

        // Cull.
        let cs = wgpu::ShaderStages::COMPUTE;
        let be = |binding: u32, ty: wgpu::BufferBindingType| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: cs,
            ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let st = |ro: bool| wgpu::BufferBindingType::Storage { read_only: ro };
        let cull_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("probe-cull-bgl"),
            entries: &[be(0, wgpu::BufferBindingType::Uniform), be(1, st(true)), be(2, st(true)), be(3, st(false)), be(4, st(false)), be(5, wgpu::BufferBindingType::Uniform)],
        });
        let cull_module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("probe-cull"), source: wgpu::ShaderSource::Wgsl(include_str!("probe_cull.wgsl").into()) });
        let cull_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("probe-cull-layout"), bind_group_layouts: &[Some(&cull_bgl)], immediate_size: 0 });
        let cull_pipeline = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("probe_cull"),
            layout: Some(&cull_layout),
            module: &cull_module,
            entry_point: Some("probe_cull"),
            compilation_options: Default::default(),
            cache: None,
        });
        let count_pipeline = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("probe_count"),
            layout: Some(&cull_layout),
            module: &cull_module,
            entry_point: Some("probe_count"),
            compilation_options: Default::default(),
            cache: None,
        });
        let cull_bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("probe-cull-bg"),
            layout: &cull_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: cull_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: init.object_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: init.group_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: draws.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: counters.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: faces_buf.as_entire_binding() },
            ],
        });

        // Fill, levels and the lobe.
        let fill_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("probe-fill-bgl"),
            entries: &[
                be(0, wgpu::BufferBindingType::Uniform),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: cs,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: cs,
                    ty: wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: PROBE_FORMAT, view_dimension: wgpu::TextureViewDimension::D2Array },
                    count: None,
                },
                be(3, st(false)),
            ],
        });
        let fill_src = format!("{}\n{}", crate::look::look_common_source(), include_str!("probe_fill.wgsl"));
        let fill_module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("probe-fill"), source: wgpu::ShaderSource::Wgsl(fill_src.into()) });
        let fill_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("probe-fill-layout"), bind_group_layouts: &[Some(&fill_bgl)], immediate_size: 0 });
        let compute = |entry: &str| {
            d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&fill_layout),
                module: &fill_module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let fill_pipeline = compute("fill");
        let mip_pipeline = compute("mip");
        let lobe_pipeline = compute("lobe_sum");
        let fbg = |src: &wgpu::TextureView, dst: &wgpu::TextureView| {
            d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("probe-fill-bg"),
                layout: &fill_bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: init.look_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(src) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(dst) },
                    wgpu::BindGroupEntry { binding: 3, resource: lobe_buf.as_entire_binding() },
                ],
            })
        };
        let fill_bgs = [0, 1].map(|i| fbg(&raw_all, &level(i, 0)));
        let mip_bgs = [0, 1].map(|i| (1..MIPS).map(|m| fbg(&level(i, m - 1), &level(i, m))).collect::<Vec<_>>());
        let lobe_bgs = [0, 1].map(|i| fbg(&level(i, 3), &level(i, MIPS - 1)));

        // The face pass: the city pipeline, single-sampled, into the HDR face.
        let pipeline = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("probe-draw"),
            layout: Some(init.draw_layout),
            vertex: wgpu::VertexState {
                module: init.draw_module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout { array_stride: init.vertex_stride, step_mode: wgpu::VertexStepMode::Vertex, attributes: init.attrs })],
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: init.depth_format,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState { count: 1, ..Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: init.draw_module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: init.hdr_format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = d.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("probe-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let level_bytes: u64 = (0..MIPS).map(|m| ((size >> m).max(1) as u64).pow(2) * 8 * 6).sum();
        let bytes = 3 * level_bytes.max(size as u64 * size as u64 * 8 * 6) + size as u64 * size as u64 * 4 + init.object_count as u64 * 20;
        Self {
            c,
            device: d.clone(),
            raw_layers,
            cubes,
            cube_views,
            depth,
            frame_buf,
            cull_buf,
            draws,
            counters,
            cull_pipeline,
            count_pipeline,
            faces_buf,
            cull_bg,
            fill_pipeline,
            mip_pipeline,
            lobe_pipeline,
            fill_bgs,
            mip_bgs,
            lobe_bgs,
            lobe_buf,
            lobe_read,
            lobe_mapped: Arc::new(AtomicU8::new(0)),
            pipeline,
            sampler,
            state: Mutex::new(State { front_pos: None, front: 0, capturing: None, hour: f64::NAN, sky_up: f64::NAN, frame: 0, this_frame: None, lobe_pending: false, frozen: false, stats: ProbeStats::default() }),
            object_count: init.object_count,
            bytes,
        }
    }

    /// The front cube (shading reads it) and where it was captured.
    pub fn front(&self) -> (usize, Option<Vec3>) {
        let s = self.state.lock().unwrap();
        (s.front, s.front_pos)
    }

    pub fn stats(&self) -> ProbeStats {
        self.state.lock().unwrap().stats
    }

    /// Hold (true) or release the front cube: while held no capture starts or
    /// advances, so two renders of one camera see the same environment.
    pub fn set_frozen(&self, frozen: bool) {
        self.state.lock().unwrap().frozen = frozen;
    }

    /// A capture is under way (its faces still to draw).
    pub fn capturing(&self) -> bool {
        self.state.lock().unwrap().capturing.is_some()
    }

    /// Forget the captures (a new world, or the probe switched off and on).
    pub fn reset(&self) {
        let mut s = self.state.lock().unwrap();
        s.front_pos = None;
        s.capturing = None;
        s.this_frame = None;
    }

    /// Per frame, before `encode`: decide whether a face is drawn this frame
    /// and write its uniforms. `frame_bytes` is the city's frame uniform for
    /// this frame (the probe pass's copy gets the face's matrix and eye, the
    /// probe-pass flag in copy_step.z, and no probe in its own lookups).
    pub fn update(&self, queue: &wgpu::Queue, eye: Vec3, hour: f64, sky_up: f64, on: bool, frame_template: &[f32; 40]) {
        let mut s = self.state.lock().unwrap();
        s.frame += 1;
        s.this_frame = None;
        // A finished lobe read: map it (non-blocking) and pick it up when ready.
        if s.lobe_pending && self.lobe_mapped.load(Ordering::Acquire) == 0 {
            self.lobe_mapped.store(1, Ordering::Release);
            let flag = self.lobe_mapped.clone();
            self.lobe_read.slice(..).map_async(wgpu::MapMode::Read, move |r| flag.store(if r.is_ok() { 2 } else { 3 }, Ordering::Release));
            s.lobe_pending = false;
        }
        match self.lobe_mapped.load(Ordering::Acquire) {
            1 => {
                let _ = self.device.poll(wgpu::PollType::Poll);
            }
            2 => {
                {
                    if let Ok(v) = self.lobe_read.slice(..).get_mapped_range() {
                        let f: &[f32] = bytemuck::cast_slice(&v);
                        if f[2] > 1e-12 {
                            let l = f[0] as f64 / f[2] as f64;
                            s.stats.lobe_radiance = Some(l);
                            // reference: the open sky's irradiance up / pi (as `probeBakeTransfer`).
                            s.stats.transfer = (sky_up > 1e-9).then(|| l / (sky_up / std::f64::consts::PI));
                        }
                    }
                }
                self.lobe_read.unmap();
                self.lobe_mapped.store(0, Ordering::Release);
            }
            3 => {
                self.lobe_read.unmap();
                self.lobe_mapped.store(0, Ordering::Release);
            }
            _ => {}
        }
        if !on || s.frozen {
            return;
        }
        // Start a capture when the viewer has left the front cube's point (or the
        // clock's 0.25 h bucket moved), unless one is under way.
        let bucket_moved = !s.hour.is_finite() || (hour / 0.25).floor() != (s.hour / 0.25).floor();
        // or the weather changed the sky (its irradiance moved by more than
        // 10 %): a cube kept from a clear sky turned the sea blue under rain.
        let sky_moved = !s.sky_up.is_finite() || (sky_up - s.sky_up).abs() > 0.1 * s.sky_up.abs().max(1e-9);
        let bucket_moved = bucket_moved || sky_moved;
        if s.capturing.is_none() {
            let moved = s.front_pos.is_none_or(|p| {
                let d = eye - p;
                Vec3::new(d.x, 0.0, d.z).length() > self.c.hysteresis || d.y.abs() > self.c.hysteresis * 0.5
            });
            if moved || bucket_moved {
                s.capturing = Some((eye, 0, s.frame));
                s.hour = hour;
                s.sky_up = sky_up;
            }
        }
        let Some((pos, face, _)) = s.capturing else { return };
        s.this_frame = Some((face, pos));
        let vp = face_view_proj(pos, face, self.c.near, self.c.far);
        let planes = frustum_planes(vp).map(|p| p.to_array());
        let cull = ProbeCull { planes, cam: [pos.x, pos.y, pos.z, self.c.far], object_count: self.object_count, kit_box_m: PROBE_KIT_BOX_M, guard_reach: self.c.far + crate::bounds::RING_MOVE_M, pad: 0 };
        queue.write_buffer(&self.cull_buf, 0, bytemuck::bytes_of(&cull));
        let mut fu = *frame_template;
        fu[..16].copy_from_slice(&vp.to_cols_array());
        fu[16..20].copy_from_slice(&[pos.x, pos.y, pos.z, 1.0]);
        // probe_info (frame.probe_info): none inside the probe pass.
        fu[32..36].copy_from_slice(&[0.0; 4]);
        // copy_step.z = 1: the probe pass (no screen-space AO, fog by distance).
        fu[38] = 1.0;
        queue.write_buffer(&self.frame_buf, 0, bytemuck::cast_slice(&fu));
    }

    /// drop this frame's face (it is drawn on a later frame; the capture
    /// keeps its point and face). For a frame that already carries a heavy
    /// pass: a middle or far cascade refresh (up to 1.0-1.7 ms of GPU) or a
    /// large upload. A face measured up to 2.4 ms on its own, and the two
    /// stacking was the shape of the frames near the budget.
    /// This frame's face and capture point (None: no face this frame).
    pub fn job(&self) -> Option<(u32, Vec3)> {
        self.state.lock().unwrap().this_frame
    }

    /// The face cull's counters: [0] commands, [1] triangles, [2] the bounded
    /// list's guard, [4..10) the count pass's per-face counts.
    pub fn counter_buffer(&self) -> &wgpu::Buffer {
        &self.counters
    }

    /// Ready the count pass for a capture at `pos` (this frame's face is the
    /// capture's, deferred: the face cull's uniform already holds `pos`).
    pub fn write_count(&self, queue: &wgpu::Queue, pos: Vec3) {
        let mut planes = [[0f32; 4]; 36];
        for f in 0..6u32 {
            let p = frustum_planes(face_view_proj(pos, f, self.c.near, self.c.far));
            for k in 0..6 {
                planes[f as usize * 6 + k] = p[k].to_array();
            }
        }
        queue.write_buffer(&self.faces_buf, 0, bytemuck::cast_slice(&planes));
    }

    /// The count pass: what each of the capture's six faces will draw
    /// (counters [4..10), read back before its faces are drawn with those bounds).
    pub fn encode_count(&self, enc: &mut wgpu::CommandEncoder) {
        enc.clear_buffer(&self.counters, 0, None);
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("probe-count"), timestamp_writes: None });
        pass.set_pipeline(&self.count_pipeline);
        pass.set_bind_group(0, &self.cull_bg, &[]);
        pass.dispatch_workgroups(self.object_count.div_ceil(64), 1, 1);
    }

    pub fn defer_this_frame(&self) -> bool {
        let mut s = self.state.lock().unwrap();
        let had = s.this_frame.is_some();
        if had {
            s.this_frame = None;
            s.stats.deferred += 1;
            crate::hitch::mark(crate::hitch::Mark::ProbeDeferred, 1);
        }
        had
    }

    /// Encode this frame's face (cull + draw), and on the sixth the fill, the
    /// levels and the lobe. Returns true when a capture finished (the front
    /// cube changed: the caller rebinds).
    #[allow(clippy::too_many_arguments)]
    pub fn encode(
        &self,
        enc: &mut wgpu::CommandEncoder,
        vertex: &wgpu::Buffer,
        index: &wgpu::Buffer,
        draw_bg: &wgpu::BindGroup,
        look: &crate::look::LookGpu,
        ts: Option<(&wgpu::QuerySet, u32)>,
        bound: Option<u32>,
    ) -> bool {
        let job = self.state.lock().unwrap().this_frame;
        let Some((face, _pos)) = job else {
            if let Some((q, b)) = ts {
                enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("probe idle"),
                    timestamp_writes: Some(wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(b), end_of_pass_write_index: Some(b + 1) }),
                });
            }
            return false;
        };
        enc.clear_buffer(&self.counters, 0, None);
        if let Some(b) = bound {
            crate::clear_list(enc, &self.draws, 0, b, crate::DRAW_ARGS_BYTES); // the bounded list (bounds.rs)
        }
        {
            let tw = ts.map(|(q, b)| wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(b), end_of_pass_write_index: None });
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("probe-cull"), timestamp_writes: tw });
            pass.set_pipeline(&self.cull_pipeline);
            pass.set_bind_group(0, &self.cull_bg, &[]);
            pass.dispatch_workgroups(self.object_count.div_ceil(64), 1, 1);
        }
        let last = face == 5;
        {
            let tw = ts.filter(|_| !last).map(|(q, b)| wgpu::RenderPassTimestampWrites { query_set: q, beginning_of_pass_write_index: None, end_of_pass_write_index: Some(b + 1) });
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("probe-face"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.raw_layers[face as usize],
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                timestamp_writes: tw,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, draw_bg, &[]);
            look.bind_main(&mut pass);
            pass.set_vertex_buffer(0, vertex.slice(..));
            pass.set_index_buffer(index.slice(..), wgpu::IndexFormat::Uint32);
            crate::draw_compacted(&mut pass, &self.draws, 0, &self.counters, 0, self.object_count, bound);
        }
        let mut s = self.state.lock().unwrap();
        s.stats.faces += 1;
        crate::hitch::mark(crate::hitch::Mark::ProbeFace, 1);
        let (pos, _, started) = s.capturing.unwrap();
        if !last {
            s.capturing = Some((pos, face + 1, started));
            return false;
        }
        // The sixth face: finish into the back cube and make it the front.
        let back = 1 - s.front;
        {
            let tw = ts.map(|(q, b)| wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: None, end_of_pass_write_index: Some(b + 1) });
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("probe-fill"), timestamp_writes: tw });
            let size = self.c.face_size;
            pass.set_pipeline(&self.fill_pipeline);
            pass.set_bind_group(0, &self.fill_bgs[back], &[]);
            pass.dispatch_workgroups(size.div_ceil(8), size.div_ceil(8), 6);
            pass.set_pipeline(&self.mip_pipeline);
            for m in 1..MIPS {
                let ms = (size >> m).max(1);
                pass.set_bind_group(0, &self.mip_bgs[back][(m - 1) as usize], &[]);
                pass.dispatch_workgroups(ms.div_ceil(8), ms.div_ceil(8), 6);
            }
            pass.set_pipeline(&self.lobe_pipeline);
            pass.set_bind_group(0, &self.lobe_bgs[back], &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        if self.lobe_mapped.load(Ordering::Acquire) == 0 {
            enc.copy_buffer_to_buffer(&self.lobe_buf, 0, &self.lobe_read, 0, 16);
            s.lobe_pending = true;
        }
        s.front = back;
        s.front_pos = Some(pos);
        s.capturing = None;
        s.stats.captures += 1;
        crate::hitch::mark(crate::hitch::Mark::ProbeCaptureDone, 1);
        s.stats.last_capture_frames = (s.frame - started + 1) as u32;
        let _ = &self.cubes;
        true
    }
}
