//! the surf strip along the mapped beaches (see native/README.md "Surf strip").
//!
//! One grid per beach (eo-water `surf_strip`): columns along the waterline
//! polyline, graded around the camera; rows that move with the waves and
//! crowd at the crests, so a breaking face, its thrown lip and the bore are
//! geometry. Drawn after the water pass in a pass of its own: the HDR target
//! (loaded) and the strip's own depth (cleared), so a crest hides the water
//! behind it and a lip its face; the opaque scene is tested in the shader
//! against the frame's depth (sampled, never written). The shading is the
//! sea's own (`w_shade`) with the strip's slope, foam and face handed in, and
//! across the strip's outer ring it is blended over the sea, which is the same
//! surface there; inside the ring the sea stands down (`surf_cover`).
//!
//! Off by default; `EO_SURF=1` draws it (see `enabled`).

use crate::camera::{Camera, frustum_planes};
use bytemuck::{Pod, Zeroable};
use eo_water::surf_strip::{LEVELS, Strip};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// Columns along the shore per strip.
pub const COLUMNS: u32 = 448;
/// Rows per wave: the Pacific, the bay.
/// (26 on the Pacific keeps the crest rows under 0.15 m at an eighth less vertex work than 30)
pub const ROWS_PER_WAVE: [u32; 2] = [26, 12];
/// How strongly the rows crowd at a crest (0: even; the density ratio is (1 + w) / (1 - w)).
pub const CREST_WARP: f32 = 0.88;
/// A strip whose nearest point is further than this (m) is drawn on every
/// fourth row and column (a far strip in the frustum cost its full grid).
pub const FAR_LOD_M: f64 = 2500.0;
/// Vertical reach of the strip's surface (m), for the frustum test.
const REACH_M: f64 = 8.0;

static HANDS_OVER: AtomicBool = AtomicBool::new(false);

/// 1 while the strip is attached and on: the sea's analytic surf stands down
/// where the strip draws (`water_surf.wgsl` `surf_cover`).
pub fn hands_over() -> f32 {
    HANDS_OVER.load(Ordering::Relaxed) as u32 as f32
}

/// `EO_SURF_DEBUG=1`: the strip draws its surf's height over the still sea (red, 2.5 m full),
/// how far its crest is breaking (green) and its foam (blue); the wet glaze blue.
fn debug_mode() -> f32 {
    static MODE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| std::env::var("EO_SURF_DEBUG").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0))
}

/// The strip is drawn only with `EO_SURF=1`. A paired comparison of the nine beach cards
/// preferred the sea's analytic surf on eight of
/// them, so the default is off again until the strip wins that comparison.
pub fn enabled() -> bool {
    std::env::var("EO_SURF").map(|v| v != "0").unwrap_or(false)
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct StripInfo {
    a: [u32; 4],
    b: [u32; 4],
    c: [f32; 4],
    d: [f32; 4],
    e: [f32; 4],
    f: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SurfUniform {
    misc: [f32; 4],
}

struct CpuStrip {
    points: Vec<[f64; 2]>,
    normals: Vec<[f64; 2]>,
    length: f64,
    bounds: [f64; 4],
    rows: u32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SurfStats {
    pub frames: u64,
    /// Frames at least one strip was drawn.
    pub drawn_frames: u64,
    /// Strips drawn in the last frame.
    pub last_drawn: u32,
}

pub struct SurfGpu {
    device: wgpu::Device,
    pub bytes: u64,
    strips: Vec<CpuStrip>,
    uniform_buf: wgpu::Buffer,
    axis_buf: wgpu::Buffer,
    index_buf: wgpu::Buffer,
    /// every fourth row and column of the grid (the far LOD)
    index_far: wgpu::Buffer,
    pipeline: wgpu::RenderPipeline,
    spray: wgpu::RenderPipeline,
    waves: Vec<u32>,
    bg: wgpu::BindGroup,
    depth: Mutex<Option<(u32, u32, wgpu::TextureView)>>,
    /// This frame's strips to draw.
    visible: Mutex<Vec<(u32, bool)>>,
    stats: Mutex<SurfStats>,
}

/// The strip's shader: the water's common part, the look, the sea and its surf, then surf.wgsl
/// (every shading feature and the parts this process runs with compiled in, no debug view, no wave optics).
pub fn shader_source() -> String {
    shader_source_with_cove(false, crate::water::ShaderBits { features: crate::water::FEAT_ALL, parts: eo_water::switches::on(), debug: 0.0, optics: false })
}

/// `bits`: what the sea's pipelines compile in (`WaterGpu::shader_bits`, water.rs `common_source`).
fn shader_source_with_cove(cove_detail: bool, bits: crate::water::ShaderBits) -> String {
    format!(
        "{}
{}
{}
{}
{}
{}
{}",
        crate::water::common_source(cove_detail, bits),
        crate::look::look_common_source(),
        include_str!("sky_light_core.wgsl"),
        crate::look::LOOK_EXT_WGSL, // group 1 bindings 12-18
        crate::look::sf_select(include_str!("water.wgsl")),
        include_str!("water_surf.wgsl"),
        include_str!("surf.wgsl")
    ) + "\n" + crate::far_shadow::WGSL // the far sun shadow map's lookup (water.wgsl `w_shadow`, lighting)
        + "\n" + &crate::probe_grid::wgsl(true) // the probe grid's lookups (water.wgsl `w_shade`, lighting)
}

impl SurfGpu {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, look: &crate::look::LookGpu, water: &crate::water::WaterGpu, strips: &[Strip]) -> Self {
        let d = device;
        let mut infos = vec![];
        let mut vecs: Vec<[f32; 4]> = vec![];
        let mut flts: Vec<f32> = vec![];
        let mut cpu = vec![];
        let mut max_rows = 0u32;
        let mut waves_per = vec![];
        for (k, st) in strips.iter().enumerate() {
            let t = &st.tables;
            let pts_off = vecs.len() as u32;
            for (p, n) in st.points.iter().zip(&st.normals) {
                vecs.push([p[0] as f32, p[1] as f32, n[0] as f32, n[1] as f32]);
            }
            let along_off = vecs.len() as u32;
            vecs.extend_from_slice(&t.along);
            let waves_off = vecs.len() as u32;
            vecs.extend_from_slice(&t.waves);
            let ntau_off = flts.len() as u32;
            flts.extend_from_slice(&t.n_of_tau);
            let bw_off = flts.len() as u32;
            flts.extend_from_slice(&st.beach_w);
            let c = &st.climate;
            let tau_max = t.along[0][1] as f64;
            let shore = ((0.0 - t.n0) / t.step).round() as usize;
            let tau_shore = t.along[shore.min(t.along.len() - 1)][1];
            let waves = (tau_max / c.period).ceil() as u32 + 3;
            let per = ROWS_PER_WAVE[if c.pacific { 0 } else { 1 }];
            let rows = waves * per;
            max_rows = max_rows.max(rows);
            waves_per.push(waves);
            infos.push(StripInfo {
                a: [pts_off, st.points.len() as u32, along_off, t.along.len() as u32],
                b: [waves_off, ntau_off, t.n_of_tau.len() as u32, bw_off],
                c: [t.n0 as f32, c.period as f32, c.h0 as f32, c.hmax as f32],
                d: [c.n_max as f32, c.pacific as u32 as f32, per as f32, waves as f32],
                e: [tau_shore, tau_max as f32, 0.0, 0.0],
                f: [k as u32 * (COLUMNS + 1), COLUMNS, rows, 0],
            });
            let mut bounds = st.bounds();
            bounds = [bounds[0] - 20.0, bounds[1] - 20.0, bounds[2] + 20.0, bounds[3] + 20.0];
            cpu.push(CpuStrip { points: st.points.clone(), normals: st.normals.clone(), length: st.length(), bounds, rows });
        }
        let _ = LEVELS;
        if vecs.is_empty() {
            vecs.push([0.0; 4]);
        }
        if flts.is_empty() {
            flts.push(0.0);
        }
        if infos.is_empty() {
            infos.push(StripInfo::zeroed());
        }
        use wgpu::util::DeviceExt;
        let storage = |label: &str, bytes: &[u8]| d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents: bytes, usage: wgpu::BufferUsages::STORAGE });
        let info_buf = storage("surf-strips", bytemuck::cast_slice(&infos));
        let vec_buf = storage("surf-tables-v4", bytemuck::cast_slice(&vecs));
        let f_buf = storage("surf-tables-f", bytemuck::cast_slice(&flts));
        let axis_len = (strips.len().max(1) as u64) * (COLUMNS as u64 + 1) * 4;
        let axis_buf = d.create_buffer(&wgpu::BufferDescriptor { label: Some("surf-axis"), size: axis_len, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let uniform_buf = d.create_buffer(&wgpu::BufferDescriptor { label: Some("surf-uniform"), size: std::mem::size_of::<SurfUniform>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        // the grid: COLUMNS x max_rows quads, rows contiguous (a strip draws its first rows)
        let max_rows = max_rows.max(1);
        let mut idx: Vec<u32> = Vec::with_capacity((COLUMNS * max_rows * 6) as usize);
        let w = COLUMNS + 1;
        for r in 0..max_rows {
            for i in 0..COLUMNS {
                let v0 = r * w + i;
                let (v1, v2, v3) = (v0 + 1, v0 + w, v0 + w + 1);
                idx.extend_from_slice(&[v0, v2, v1, v1, v2, v3]);
            }
        }
        let index_buf = d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("surf-indices"), contents: bytemuck::cast_slice(&idx), usage: wgpu::BufferUsages::INDEX });
        let mut far: Vec<u32> = vec![];
        for r in (0..max_rows.saturating_sub(4)).step_by(4) {
            for i in (0..COLUMNS.saturating_sub(4) + 1).step_by(4) {
                let v0 = r * w + i;
                let (v1, v2, v3) = (v0 + 4, v0 + 4 * w, v0 + 4 * w + 4);
                far.extend_from_slice(&[v0, v2, v1, v1, v2, v3]);
            }
        }
        if far.is_empty() {
            far.extend_from_slice(&[0, 0, 0]);
        }
        let index_far = d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("surf-indices-far"), contents: bytemuck::cast_slice(&far), usage: wgpu::BufferUsages::INDEX });
        let bytes = info_buf.size() + vec_buf.size() + f_buf.size() + axis_buf.size() + uniform_buf.size() + index_buf.size();
        queue.write_buffer(&uniform_buf, 0, bytemuck::bytes_of(&SurfUniform { misc: [CREST_WARP, 0.0, 1.0, 0.0] }));

        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry { binding, visibility: vf, ty, count: None };
        let sbuf = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None };
        let bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("surf-bgl"),
            entries: &[
                entry(0, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
                entry(1, sbuf),
                entry(2, sbuf),
                entry(3, sbuf),
                entry(4, sbuf),
            ],
        });
        let bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("surf-bg"),
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: info_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: vec_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: f_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: axis_buf.as_entire_binding() },
            ],
        });
        let src = shader_source_with_cove(look.options.cove_detail, water.shader_bits());
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("surf.wgsl"), source: wgpu::ShaderSource::Wgsl(src.into()) });
        let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("surf"), bind_group_layouts: &[Some(water.surf_layout()), Some(look.main_bind_group_layout()), Some(&bgl)], immediate_size: 0 });
        let premul = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
        };
        let make = |label: &str, vs: &str, fs: &str, write: bool| {
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState { module: &module, entry_point: Some(vs), compilation_options: Default::default(), buffers: &[] },
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(write),
                    depth_compare: Some(wgpu::CompareFunction::Greater),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState { count: 1, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: crate::look::HDR_FORMAT, blend: Some(premul), write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipeline = make("surf", "vs_surf", "fs_surf", true);
        // the spray after the strip: tested against its depth, not written
        let spray = make("surf-spray", "vs_spray", "fs_spray", false);
        HANDS_OVER.store(!strips.is_empty(), Ordering::Relaxed);
        let waves = waves_per;
        Self {
            device: d.clone(),
            bytes,
            strips: cpu,
            uniform_buf,
            axis_buf,
            index_buf,
            index_far,
            pipeline,
            spray,
            waves,
            bg,
            depth: Mutex::new(None),
            visible: Mutex::new(vec![]),
            stats: Mutex::new(SurfStats::default()),
        }
    }

    pub fn stats(&self) -> SurfStats {
        *self.stats.lock().unwrap()
    }

    pub fn strip_count(&self) -> usize {
        self.strips.len()
    }

    /// Per frame, before encoding: which strips are in the frustum, their
    /// columns graded around the camera, the weather's height.
    /// `swell_hs`: the sea plan's swell significant height (m; eo-water `waves`), which scales the strip's incoming .
    pub fn update(&self, queue: &wgpu::Queue, cam: &Camera, active: bool, storm: f32, swell_hs: f32) {
        let mut vis = self.visible.lock().unwrap();
        vis.clear();
        let mut stats = self.stats.lock().unwrap();
        stats.frames += 1;
        if !active || !HANDS_OVER.load(Ordering::Relaxed) {
            stats.last_drawn = 0;
            return;
        }
        let planes = frustum_planes(cam.view_proj());
        let sea = eo_water::SEA_LEVEL_Y;
        for (k, st) in self.strips.iter().enumerate() {
            let b = st.bounds;
            let (lo, hi) = (glam::Vec3::new(b[0] as f32, (sea - REACH_M) as f32, b[1] as f32), glam::Vec3::new(b[2] as f32, (sea + REACH_M) as f32, b[3] as f32));
            // reverse-Z far plane (index 4) skipped: the strip is drawn to the horizon
            let outside = planes.iter().enumerate().any(|(i, p)| {
                if i == 4 {
                    return false;
                }
                let n = glam::Vec3::new(p.x, p.y, p.z);
                let v = glam::Vec3::new(if n.x >= 0.0 { hi.x } else { lo.x }, if n.y >= 0.0 { hi.y } else { lo.y }, if n.z >= 0.0 { hi.z } else { lo.z });
                n.dot(v) + p.w < 0.0
            });
            if outside {
                continue;
            }
            let axis = column_axis(st, cam.eye.x as f64, cam.eye.z as f64, (cam.eye.y as f64 - sea).max(0.0));
            queue.write_buffer(&self.axis_buf, (k as u64) * (COLUMNS as u64 + 1) * 4, bytemuck::cast_slice(&axis));
            let (ex, ez) = (cam.eye.x as f64, cam.eye.z as f64);
            let dx = (b[0] - ex).max(ex - b[2]).max(0.0);
            let dz = (b[1] - ez).max(ez - b[3]).max(0.0);
            vis.push((k as u32, dx.hypot(dz) > FAR_LOD_M));
        }
        stats.last_drawn = vis.len() as u32;
        if !vis.is_empty() {
            stats.drawn_frames += 1;
        }
        // the weather: the swell's Hs against the clear day's 1.8 m (from the sea's plan, which carries the storm; it was a fixed 2.1 x in a storm)
        let height = (swell_hs / 1.8).clamp(0.3, 2.5);
        queue.write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&SurfUniform { misc: [CREST_WARP, storm, height, debug_mode()] }));
    }

    /// The strip's pass, after the water pass (`water` holds this frame's group-0 bind group).
    pub fn encode(&self, enc: &mut wgpu::CommandEncoder, water: &crate::water::WaterGpu, look: &crate::look::LookGpu, hdr: &wgpu::TextureView, probe_front: usize) {
        let vis = self.visible.lock().unwrap();
        if vis.is_empty() || water.surf_fogged() {
            return;
        }
        let Some(wbg) = water.surf_bind_group(probe_front) else { return };
        let ht = hdr.texture();
        let (w, h) = (ht.width(), ht.height());
        let depth = {
            let mut dd = self.depth.lock().unwrap();
            if dd.as_ref().is_none_or(|x| x.0 != w || x.1 != h) {
                let t = self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("surf-depth"),
                    size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Depth32Float,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                });
                *dd = Some((w, h, t.create_view(&Default::default())));
            }
            dd.as_ref().unwrap().2.clone()
        };
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("surf"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: hdr, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Discard }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &wbg, &[]);
        look.bind_main(&mut pass);
        pass.set_bind_group(2, &self.bg, &[]);
        pass.set_index_buffer(self.index_buf.slice(..), wgpu::IndexFormat::Uint32);
        for &(k, far) in vis.iter() {
            if !far {
                let rows = self.strips[k as usize].rows;
                pass.draw_indexed(0..COLUMNS * rows * 6, 0, k..k + 1);
            }
        }
        // the far strips on every fourth row and column
        if vis.iter().any(|v| v.1) {
            pass.set_index_buffer(self.index_far.slice(..), wgpu::IndexFormat::Uint32);
            for &(k, far) in vis.iter() {
                if far {
                    let rows = self.strips[k as usize].rows;
                    let quads = (rows.saturating_sub(4)).div_ceil(4) * (COLUMNS / 4);
                    pass.draw_indexed(0..quads * 6, 0, k..k + 1);
                }
            }
        }
        // the spray on the breaking crests and the haze over the break (no index buffer: six vertices a quad), near strips only
        pass.set_pipeline(&self.spray);
        for &(k, far) in vis.iter() {
            if !far {
                pass.draw(0..COLUMNS * (self.waves[k as usize] + 2) * 6, k..k + 1); // (+ the two haze bands)
            }
        }
    }
}

/// The strip's columns: spacing that grows with the distance along the shore
/// from the camera (finest where the camera stands, about 1.5 % of the
/// distance further off), scaled so the strip's whole length fits.
fn column_axis(st: &CpuStrip, x: f64, z: f64, height: f64) -> Vec<f32> {
    let strip = StripView { points: &st.points, normals: &st.normals };
    let (sc, nc) = strip.project(x, z);
    let off = (nc.abs() - 60.0).max(0.0).max(height);
    let min = (0.35 + 0.004 * off).clamp(0.35, 30.0);
    let len = st.length.max(1.0);
    let steps = (len / 2.0).ceil() as usize;
    let ds = len / steps as f64;
    let mut cdf = vec![0.0f64; steps + 1];
    for i in 0..steps {
        let s = (i as f64 + 0.5) * ds;
        let g = min + 0.015 * (s - sc).abs();
        cdf[i + 1] = cdf[i] + ds / g;
    }
    let total = cdf[steps];
    let mut out = Vec::with_capacity(COLUMNS as usize + 1);
    let mut j = 0;
    for i in 0..=COLUMNS {
        let target = total * i as f64 / COLUMNS as f64;
        while j + 1 < steps && cdf[j + 1] < target {
            j += 1;
        }
        let (a, b) = (cdf[j], cdf[j + 1]);
        let f = if b > a { ((target - a) / (b - a)).clamp(0.0, 1.0) } else { 0.0 };
        out.push(((j as f64 + f) * ds) as f32);
    }
    out
}

struct StripView<'a> {
    points: &'a [[f64; 2]],
    normals: &'a [[f64; 2]],
}

impl StripView<'_> {
    fn project(&self, x: f64, z: f64) -> (f64, f64) {
        let mut best = (f64::INFINITY, 0usize);
        for (i, p) in self.points.iter().enumerate() {
            let d = (p[0] - x).powi(2) + (p[1] - z).powi(2);
            if d < best.0 {
                best = (d, i);
            }
        }
        let i = best.1;
        let p = self.points[i];
        let nr = self.normals[i];
        let off = (x - p[0]) * nr[0] + (z - p[1]) * nr[1];
        (i as f64 * eo_water::surf_strip::SAMPLE_M, off)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surf_shader_parses_and_validates() {
        use wgpu::naga;
        let src = shader_source();
        let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("surf shader: {}", e.emit_to_string(&src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|e| panic!("surf shader: {}", e.emit_to_string(&src)));
    }

    #[test]
    fn columns_cover_the_strip_and_crowd_at_the_camera() {
        let n = 1001;
        let points: Vec<[f64; 2]> = (0..n).map(|i| [0.0, i as f64 * eo_water::surf_strip::SAMPLE_M]).collect();
        let normals = vec![[-1.0, 0.0]; n];
        let length = (n - 1) as f64 * eo_water::surf_strip::SAMPLE_M;
        let st = CpuStrip { points, normals, length, bounds: [0.0; 4], rows: 1 };
        let a = column_axis(&st, 10.0, 1500.0, 1.7);
        assert_eq!(a.len(), COLUMNS as usize + 1);
        assert!(a[0].abs() < 1e-3 && (a[COLUMNS as usize] as f64 - length).abs() < 1e-2);
        for w in a.windows(2) {
            assert!(w[1] > w[0]);
        }
        // finest at the camera's s (1500 m), coarse at the ends
        let near = a.windows(2).find(|w| w[0] <= 1500.0 && w[1] >= 1500.0).map(|w| w[1] - w[0]).unwrap();
        let far = a[1] - a[0];
        assert!(near < 2.0 && far > 8.0 * near, "near {near} far {far}");
    }
}
