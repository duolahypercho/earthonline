//! Ray-traced one-bounce diffuse light (`--rt-gi`; opt-in, no tier turns it
//! on): hardware ray queries (wgpu `EXPERIMENTAL_RAY_QUERY`) against the
//! static city (rt_scene.rs), half resolution, one ray a texel from the
//! G-buffer's normal (rt_composite.rs), accumulated over frames and filtered
//! (rt_gi.wgsl); rt_composite.rs adds it in place of the rig's indirect
//! diffuse in the same frame.
//!
//! Stage R1 read the previous frame's result inside every lit pipeline,
//! which cost the city's opaque draw 1-2.5 ms in registers and occupancy even
//! where nothing was traced; stage R2 moved the read to the deferred
//! composite.
//!
//! The switch: `ShaderFeatures::rt_gi` composes the G-buffer's hook, values
//! and `fs_rtg` into the city shaders; without it the shader text, the
//! layouts and every pass are as before (`--no-rt-gi` is today's frame byte
//! for byte). The pass itself (`RtGi`) exists only on a device with ray
//! queries (`GpuContext::ray_query`: Metal only, the one backend tested)
//! with a single-sample target; eo-app turns the option off with one line
//! elsewhere.
//!
//! What it leaves out: the surfaces the city's opaque pass does not shade
//! (props, agents, trees, the far layer, glass drawn blended, water) keep the
//! rig's light; the traced scene has no props, agents, trees or water.
//!
//! Frame-cost diagnostics: `EO_RT_GI_NO_HOOK=1` composes the G-buffer's
//! outputs but not the hook (they stay zero, the composite leaves the frame);
//! eo-app's `EO_RT_GI_DEVICE_ONLY=1` asks for the ray-query device with the
//! option off.

use crate::lighting;
use crate::rt_scene::RtScene;
use glam::{Mat4, Vec3};
use std::sync::Mutex;
use wgpu::util::DeviceExt;

/// The result's format (rgb irradiance, a the environment's open share) and its depth's.
pub const GI_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const GI_Z_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
const G_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
/// The history's moments: frame count, the filter guide's mean and mean square.
const MOMENTS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
/// The pass's tunables. Defaults are the stage R1 prototype's; every one
/// can be overridden at start for experiments (`from_env`, `EO_RT_GI_*`;
/// the traced scene's own are `rt_scene::RtSceneConfig`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RtGiConfig {
    /// A ray's reach (m).
    pub reach_m: f32,
    /// The ray's origin off the surface along its normal: a constant (m) and per metre of view depth.
    pub origin_m: f32,
    pub origin_per_m: f32,
    /// The share of the rig's open-sky light a hit surface takes (its own sky
    /// visibility is not traced: one bounce).
    pub hit_sky: f32,
    /// The most frames the history averages.
    pub max_frames: f32,
    /// The traced light fades out from this view depth to that (m); past it the rig's.
    pub fade_start_m: f32,
    pub fade_end_m: f32,
    /// The filter's normal power and plane distance scale (m).
    pub normal_power: f32,
    pub plane_m: f32,
    /// A-trous passes (0: none; steps 1, 2, 4, 8).
    pub atrous: u32,
    /// How much of the screen-space occlusion stays on the indirect diffuse (0: replaced).
    pub ssao_keep: f32,
    /// The traced light's weight against the rig's (1: all of it).
    pub weight: f32,
    /// 1: draw the traced diffuse light alone (irradiance x the surface's diffuse
    /// albedo), 2 the open share, 3 the irradiance without the environment.
    pub debug: u32,
}

impl Default for RtGiConfig {
    fn default() -> Self {
        Self { reach_m: 200.0, origin_m: 0.05, origin_per_m: 0.002, hit_sky: 0.6, max_frames: 24.0, fade_start_m: 260.0, fade_end_m: 340.0, normal_power: 32.0, plane_m: 0.25, atrous: 3, ssao_keep: 0.5, weight: 1.0, debug: 0 }
    }
}

impl RtGiConfig {
    /// The defaults with any `EO_RT_GI_<NAME>` given (reach, origin, origin_per_m,
    /// hit_sky, frames, fade_start, fade_end, normal_power, plane, atrous, ssao,
    /// weight, debug).
    pub fn from_env() -> Self {
        let mut c = Self::default();
        let f = |k: &str, v: &mut f32| {
            if let Some(x) = std::env::var(format!("EO_RT_GI_{k}")).ok().and_then(|s| s.parse::<f32>().ok()) {
                *v = x;
            }
        };
        f("REACH", &mut c.reach_m);
        f("ORIGIN", &mut c.origin_m);
        f("ORIGIN_PER_M", &mut c.origin_per_m);
        f("HIT_SKY", &mut c.hit_sky);
        f("FRAMES", &mut c.max_frames);
        f("FADE_START", &mut c.fade_start_m);
        f("FADE_END", &mut c.fade_end_m);
        f("NORMAL_POWER", &mut c.normal_power);
        f("PLANE", &mut c.plane_m);
        f("SSAO", &mut c.ssao_keep);
        f("WEIGHT", &mut c.weight);
        let mut x = c.atrous as f32;
        f("ATROUS", &mut x);
        c.atrous = (x as u32).min(4);
        let mut x = c.debug as f32;
        f("DEBUG", &mut x);
        c.debug = x as u32;
        c.max_frames = c.max_frames.max(1.0);
        c
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "reach_m": self.reach_m, "origin_m": self.origin_m, "origin_per_m": self.origin_per_m, "hit_sky": self.hit_sky,
            "max_frames": self.max_frames, "fade_start_m": self.fade_start_m, "fade_end_m": self.fade_end_m,
            "normal_power": self.normal_power, "plane_m": self.plane_m, "atrous": self.atrous, "ssao_keep": self.ssao_keep,
            "weight": self.weight, "debug": self.debug,
        })
    }
}

/// The trace uniform (rt_gi.wgsl `RtTrace`).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct RtTrace {
    cam: [f32; 4],
    fwd: [f32; 4],
    prev_vp: [[f32; 4]; 4],
    prev_cam: [f32; 4],
    prev_fwd: [f32; 4],
    params: [f32; 4],
    fade: [f32; 4],
    size: [u32; 4],
}

/// What the pass did, for the bench JSON (`RtGi::stats`).
#[derive(Clone, Copy, Debug, Default)]
pub struct RtGiStats {
    pub frames: u64,
    pub traced_frames: u64,
    pub encode_ms_sum: f64,
    pub encode_ms_max: f64,
}

impl RtGiStats {
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "frames": self.frames, "traced_frames": self.traced_frames,
            "encode_cpu_ms_mean": if self.frames > 0 { self.encode_ms_sum / self.frames as f64 } else { 0.0 },
            "encode_cpu_ms_max": self.encode_ms_max,
        })
    }
}

#[derive(Clone, Copy)]
struct Cam {
    vp: Mat4,
    eye: Vec3,
    fwd: Vec3,
}

/// The half-resolution targets and their bind groups (per history parity).
struct Targets {
    size: (u32, u32),
    gi_view: wgpu::TextureView,
    z_view: wgpu::TextureView,
    trace: [wgpu::BindGroup; 2],
    temporal: [wgpu::BindGroup; 2],
    atrous: [Vec<wgpu::BindGroup>; 2],
    bytes: u64,
}

struct State {
    frame: u32,
    /// The camera the history in the targets was made with.
    made: Option<Cam>,
    targets: Option<Targets>,
    /// Group 1 of the trace (its uniform, the depth, the normal), for one depth and normal texture.
    frame_bg: Option<((wgpu::Texture, wgpu::Texture), wgpu::BindGroup)>,
    stats: RtGiStats,
}

/// The pass (made only with the option and a device with ray queries).
pub struct RtGi {
    pub config: RtGiConfig,
    device: wgpu::Device,
    queue: wgpu::Queue,
    trace_buf: wgpu::Buffer,
    step_bufs: Vec<wgpu::Buffer>,
    bgl_frame: wgpu::BindGroupLayout,
    bgl_trace: wgpu::BindGroupLayout,
    bgl_temporal: wgpu::BindGroupLayout,
    bgl_atrous: wgpu::BindGroupLayout,
    trace: wgpu::ComputePipeline,
    temporal: wgpu::ComputePipeline,
    atrous: wgpu::ComputePipeline,
    state: Mutex<State>,
    /// Bytes allocated on the GPU (uniforms; the targets are `target_bytes`).
    pub bytes: u64,
}

/// The trace's shader source (compiled with ray queries).
pub fn shader_source() -> String {
    crate::rt_scene::module_source(include_str!("rt_gi.wgsl"))
}

fn target(device: &wgpu::Device, label: &str, size: (u32, u32), format: wgpu::TextureFormat, usage: wgpu::TextureUsages) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: usage | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

impl RtGi {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, scene: &RtScene) -> Self {
        let config = RtGiConfig::from_env();
        let uni = wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST;
        let trace_buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("rt-gi-trace"), size: std::mem::size_of::<RtTrace>() as u64, usage: uni, mapped_at_creation: false });
        // the a-trous steps (x the step, y 1 on the last pass, z the pass).
        let passes = config.atrous.max(1);
        let step_bufs = (0..passes)
            .map(|k| {
                let step = if config.atrous == 0 { 0 } else { 1u32 << k };
                let last = (k + 1 == passes) as u32;
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("rt-gi-step"), contents: bytemuck::cast_slice(&[step, last, k, 0u32]), usage: wgpu::BufferUsages::UNIFORM })
            })
            .collect();

        let entry = |vis| move |binding, ty| wgpu::BindGroupLayoutEntry { binding, visibility: vis, ty, count: None };
        let e = entry(wgpu::ShaderStages::COMPUTE);
        let ubuf = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None };
        let tex = wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false };
        let depth_tex = wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false };
        let store = |format| wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format, view_dimension: wgpu::TextureViewDimension::D2 };
        let bgl_frame = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("rt-gi-bgl-frame"), entries: &[e(0, ubuf), e(1, depth_tex), e(2, tex)] });
        let bgl_trace = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("rt-gi-bgl-trace"), entries: &[e(4, store(GI_FORMAT)), e(5, store(G_FORMAT))] });
        let bgl_temporal = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rt-gi-bgl-temporal"),
            entries: &[e(0, tex), e(1, tex), e(2, tex), e(3, tex), e(8, tex), e(4, store(GI_FORMAT)), e(5, store(MOMENTS_FORMAT))],
        });
        let bgl_atrous = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rt-gi-bgl-atrous"),
            entries: &[e(0, tex), e(1, tex), e(2, tex), e(4, store(GI_FORMAT)), e(6, store(GI_Z_FORMAT)), e(7, ubuf)],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("rt_gi.wgsl"), source: wgpu::ShaderSource::Wgsl(shader_source().into()) });
        let pipe = |label: &str, bgl: &wgpu::BindGroupLayout| {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(label), bind_group_layouts: &[Some(scene.bind_group_layout()), Some(&bgl_frame), Some(bgl)], immediate_size: 0 });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(label), layout: Some(&layout), module: &module, entry_point: Some(label), compilation_options: Default::default(), cache: None })
        };
        let trace = pipe("rt_trace", &bgl_trace);
        let temporal = pipe("rt_temporal", &bgl_temporal);
        let atrous = pipe("rt_atrous", &bgl_atrous);
        Self {
            config,
            device: device.clone(),
            queue: queue.clone(),
            trace_buf,
            step_bufs,
            bgl_frame,
            bgl_trace,
            bgl_temporal,
            bgl_atrous,
            trace,
            temporal,
            atrous,
            state: Mutex::new(State { frame: 0, made: None, targets: None, frame_bg: None, stats: RtGiStats::default() }),
            bytes: 512,
        }
    }

    pub fn stats(&self) -> RtGiStats {
        self.state.lock().unwrap().stats
    }

    /// Bytes of the targets (once made).
    pub fn target_bytes(&self) -> u64 {
        self.state.lock().unwrap().targets.as_ref().map(|t| t.bytes).unwrap_or(0)
    }

    fn make_targets(&self, size: (u32, u32)) -> Targets {
        let d = &self.device;
        let s = wgpu::TextureUsages::STORAGE_BINDING;
        let cur = target(d, "rt-gi-sample", size, GI_FORMAT, s);
        let g = [target(d, "rt-gi-g0", size, G_FORMAT, s), target(d, "rt-gi-g1", size, G_FORMAT, s)];
        let hist = [target(d, "rt-gi-hist0", size, GI_FORMAT, s), target(d, "rt-gi-hist1", size, GI_FORMAT, s)];
        let cnt = [target(d, "rt-gi-moments0", size, MOMENTS_FORMAT, s), target(d, "rt-gi-moments1", size, MOMENTS_FORMAT, s)];
        let ping = [target(d, "rt-gi-a", size, GI_FORMAT, s), target(d, "rt-gi-b", size, GI_FORMAT, s)];
        let out = target(d, "rt-gi", size, GI_FORMAT, s);
        let z = target(d, "rt-gi-z", size, GI_Z_FORMAT, s);
        use wgpu::BindingResource::TextureView as T;
        let bg = |layout: &wgpu::BindGroupLayout, entries: &[wgpu::BindGroupEntry]| d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("rt-gi-bg"), layout, entries });
        let be = |binding: u32, resource| wgpu::BindGroupEntry { binding, resource };
        let trace = [0, 1].map(|p| bg(&self.bgl_trace, &[be(4, T(&cur)), be(5, T(&g[p]))]));
        let temporal = [0, 1].map(|p| {
            bg(&self.bgl_temporal, &[be(0, T(&cur)), be(1, T(&g[p])), be(2, T(&g[1 - p])), be(3, T(&hist[1 - p])), be(8, T(&cnt[1 - p])), be(4, T(&hist[p])), be(5, T(&cnt[p]))])
        });
        let passes = self.step_bufs.len();
        let atrous = [0, 1].map(|p| {
            let mut src = &hist[p];
            let mut v = vec![];
            for k in 0..passes {
                let dst = if k + 1 == passes { &out } else { &ping[k % 2] };
                v.push(bg(&self.bgl_atrous, &[be(0, T(src)), be(1, T(&g[p])), be(2, T(&cnt[p])), be(4, T(dst)), be(6, T(&z)), be(7, self.step_bufs[k].as_entire_binding())]));
                src = dst;
            }
            v
        });
        let px = size.0 as u64 * size.1 as u64;
        let bytes = px * (8 + 2 * 16 + 2 * 8 + 2 * 16 + 2 * 8 + 8 + 4);
        Targets { size, gi_view: out, z_view: z, trace, temporal, atrous, bytes }
    }

    /// The frame's pass, after the city's opaque pass (its depth and G-buffer
    /// stored) and the scene's `update`: trace, temporal and a-trous at half
    /// resolution (`lighting::Q_RT_GI`; the BLAS and TLAS builds are outside
    /// it). Returns the result and its view depth for the composite, or None
    /// when nothing was traced (the rig's light stays).
    pub fn encode(&self, enc: &mut wgpu::CommandEncoder, scene: &RtScene, look: &crate::look::LookGpu, depth: &wgpu::TextureView, gb_n: &wgpu::TextureView, ts: lighting::Ts<'_>) -> Option<(wgpu::TextureView, wgpu::TextureView)> {
        let t0 = std::time::Instant::now();
        let mut guard = self.state.lock().unwrap();
        let st: &mut State = &mut guard;
        st.stats.frames += 1;
        let dt = depth.texture();
        let size_full = (dt.width(), dt.height());
        let usable = dt.sample_count() == 1 && dt.usage().contains(wgpu::TextureUsages::TEXTURE_BINDING) && dt.format() == crate::DEPTH_FORMAT && gb_n.texture().width() == size_full.0 && gb_n.texture().height() == size_full.1;
        let cam = look.current_camera().filter(|_| usable && scene.live());
        let Some((vp, eye, fwd, _)) = cam else {
            st.made = None;
            lighting::empty_span(enc, ts, lighting::Q_RT_GI, "rt-gi (idle)");
            return None;
        };
        let cur = Cam { vp, eye, fwd };
        let cfg = self.config;

        let size = ((size_full.0 / 2).max(1), (size_full.1 / 2).max(1));
        if st.targets.as_ref().map(|t| t.size) != Some(size) {
            st.targets = Some(self.make_targets(size));
            st.made = None;
        }
        let key = (dt.clone(), gb_n.texture().clone());
        if st.frame_bg.as_ref().is_none_or(|(k, _)| *k != key) {
            crate::hitch::mark(crate::hitch::Mark::BindGroupCreated, 1);
            let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("rt-gi-bg-frame"),
                layout: &self.bgl_frame,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: self.trace_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(depth) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(gb_n) },
                ],
            });
            st.frame_bg = Some((key, bg));
        }
        st.frame = st.frame.wrapping_add(1);
        let prev = st.made;
        let tr = RtTrace {
            cam: [eye.x, eye.y, eye.z, (st.frame % (1 << 23)) as f32],
            fwd: [fwd.x, fwd.y, fwd.z, cfg.reach_m],
            prev_vp: prev.map(|p| p.vp.to_cols_array_2d()).unwrap_or([[0.0; 4]; 4]),
            prev_cam: prev.map(|p| p.eye.extend(1.0).to_array()).unwrap_or([0.0; 4]),
            prev_fwd: prev.map(|p| p.fwd.extend(0.0).to_array()).unwrap_or([0.0; 4]),
            params: [cfg.origin_m, cfg.origin_per_m, cfg.hit_sky, cfg.max_frames],
            fade: [cfg.fade_start_m, cfg.fade_end_m, cfg.normal_power, cfg.plane_m],
            size: [size.0, size.1, size_full.0, size_full.1],
        };
        self.queue.write_buffer(&self.trace_buf, 0, bytemuck::bytes_of(&tr));
        let p = (st.frame % 2) as usize;
        let t = st.targets.as_ref().unwrap();
        {
            let groups = (size.0.div_ceil(8), size.1.div_ceil(8));
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("rt-gi"), timestamp_writes: lighting::compute_writes(ts, lighting::Q_RT_GI) });
            pass.set_bind_group(0, scene.bind_group(), &[]);
            pass.set_bind_group(1, &st.frame_bg.as_ref().unwrap().1, &[]);
            pass.set_pipeline(&self.trace);
            pass.set_bind_group(2, &t.trace[p], &[]);
            pass.dispatch_workgroups(groups.0, groups.1, 1);
            pass.set_pipeline(&self.temporal);
            pass.set_bind_group(2, &t.temporal[p], &[]);
            pass.dispatch_workgroups(groups.0, groups.1, 1);
            pass.set_pipeline(&self.atrous);
            for b in &t.atrous[p] {
                pass.set_bind_group(2, b, &[]);
                pass.dispatch_workgroups(groups.0, groups.1, 1);
            }
        }
        let out = (t.gi_view.clone(), t.z_view.clone());
        st.made = Some(cur);
        st.stats.traced_frames += 1;
        let ms = t0.elapsed().as_secs_f64() * 1e3;
        st.stats.encode_ms_sum += ms;
        st.stats.encode_ms_max = st.stats.encode_ms_max.max(ms);
        Some(out)
    }

    /// What the composite takes from a result (rt_composite.rs `RtResult`).
    pub fn composite_input<'a>(&self, result: &'a wgpu::TextureView, z: &'a wgpu::TextureView) -> crate::rt_composite::RtResult<'a> {
        let c = self.config;
        crate::rt_composite::RtResult { result, z, weight: c.weight, fade: (c.fade_start_m, c.fade_end_m), keep: c.ssao_keep, debug: c.debug }
    }

    /// The result's views (tests).
    pub fn result_views(&self) -> Option<(wgpu::TextureView, wgpu::TextureView)> {
        self.state.lock().unwrap().targets.as_ref().map(|t| (t.gi_view.clone(), t.z_view.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniforms_match_their_wgsl_structs() {
        let span = |src: &str, name: &str| {
            let m = wgpu::naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
            wgpu::naga::valid::Validator::new(wgpu::naga::valid::ValidationFlags::all(), wgpu::naga::valid::Capabilities::all()).validate(&m).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
            let (_, ty) = m.types.iter().find(|(_, t)| t.name.as_deref() == Some(name)).expect(name);
            let wgpu::naga::TypeInner::Struct { span, .. } = ty.inner else { panic!() };
            span as usize
        };
        assert_eq!(span(&shader_source(), "RtTrace"), std::mem::size_of::<RtTrace>());
    }

    #[test]
    fn the_config_reads_its_defaults() {
        let c = RtGiConfig::default();
        let s = crate::rt_scene::RtSceneConfig::default();
        assert!(c.fade_start_m < c.fade_end_m && c.fade_end_m <= s.scene_radius_m, "the traced light fades out inside the traced set");
        assert!(c.reach_m <= s.scene_radius_m);
    }
}
