//! two-phase hierarchical-Z occlusion culling.
//!
//! Frame order with occlusion on (`Renderer::encode_with_ids`):
//!
//! 1. **Early cull** (`cull.wgsl` `cull`): LOD + frustum as before, but only
//!    the opaque objects that passed the occlusion test LAST frame
//!    (`visible_last[i] == 1`) are appended, to the opaque list.
//! 2. **Early city pass**: those objects, depth stored.
//! 3. **Pyramid** (`occlusion_hzb.wgsl`): the early depth reduced to a
//!    hierarchical Z (farthest depth per texel), one dispatch per level.
//! 4. **Late cull** (`cull_late`): every object in the frustum is tested
//!    against the pyramid (`occlusion.wgsl`); the result is written as next
//!    frame's early set. What is visible and was not drawn early (newly
//!    visible, or glass) is appended to the late opaque / transparent lists.
//! 5. **Late city pass**: the late opaque list, the sky, the glass.
//!
//! An object that becomes visible is drawn in the same frame by the late
//! pass (no frame of popping). The test is conservative: an object is culled
//! only when its nearest point lies behind the farthest depth the early pass
//! drew over every pixel its screen rectangle can touch; that depth is part
//! of the final image, so the object would have failed the depth test on
//! every one of those pixels. The early depth is a subset of the final
//! depth, so the late test can only keep too much, never too little.
//!
//! Requires `MULTI_DRAW_INDIRECT_COUNT` (the compacted lists); without it
//! the renderer draws as before (one pass, no occlusion).

use std::sync::Mutex;

pub const HZB_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

/// The pyramid test as WGSL (needs a `texture_2d<f32>` global named `hzb`):
/// `hzb_occluded(min, max, view_proj, screen_px) -> bool`. For other culls
/// (moving agents) that run after `Renderer` built the pyramid.
pub const OCCLUSION_WGSL: &str = include_str!("occlusion.wgsl");

fn hzb_source(msaa: bool) -> String {
    let src = include_str!("occlusion_hzb.wgsl");
    let (keep, drop) = if msaa { ("msaa", "single") } else { ("single", "msaa") };
    let mut out = String::new();
    let mut skipping = false;
    for line in src.lines() {
        let t = line.trim();
        if t == format!("#ifdef_{drop}") {
            skipping = true;
            continue;
        }
        if t == format!("#endif_{drop}") {
            skipping = false;
            continue;
        }
        if t == format!("#ifdef_{keep}") || t == format!("#endif_{keep}") {
            continue;
        }
        if !skipping {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// The pyramid for one depth target.
struct Hzb {
    depth: wgpu::TextureView,
    view: wgpu::TextureView,
    dims: Vec<(u32, u32)>,
    first_bg: wgpu::BindGroup,
    down_bgs: Vec<wgpu::BindGroup>,
    late_bg: wgpu::BindGroup,
    bytes: u64,
}

pub struct OcclusionGpu {
    /// One u32 per object: passed the occlusion test last frame.
    pub vis_buf: wgpu::Buffer,
    first_pipeline: wgpu::ComputePipeline,
    down_pipeline: wgpu::ComputePipeline,
    first_bgl: wgpu::BindGroupLayout,
    down_bgl: wgpu::BindGroupLayout,
    /// Group 1 of `cull_late` (and of any other cull using the pyramid): binding 0, the pyramid.
    pub late_bgl: wgpu::BindGroupLayout,
    msaa: bool,
    hzb: Mutex<Option<Hzb>>,
    device: wgpu::Device,
}

impl OcclusionGpu {
    pub fn new(device: &wgpu::Device, object_count: u32, sample_count: u32) -> Self {
        let msaa = sample_count > 1;
        let vis_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("occlusion-visible-last"),
            size: (object_count.max(1) as u64 * 4).next_multiple_of(16),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let cs = wgpu::ShaderStages::COMPUTE;
        let storage_out = wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: cs,
            ty: wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: HZB_FORMAT, view_dimension: wgpu::TextureViewDimension::D2 },
            count: None,
        };
        let float_tex = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: cs,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        };
        let first_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hzb-first-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: if msaa { 2 } else { 0 },
                    visibility: cs,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: msaa },
                    count: None,
                },
                storage_out,
            ],
        });
        let down_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("hzb-down-bgl"), entries: &[float_tex(3), storage_out] });
        let info_entry = wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: cs,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let late_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("hzb-test-bgl"), entries: &[float_tex(0), info_entry] });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("occlusion_hzb.wgsl"),
            source: wgpu::ShaderSource::Wgsl(hzb_source(msaa).into()),
        });
        let pipeline = |entry: &str, bgl: &wgpu::BindGroupLayout| {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(entry), bind_group_layouts: &[Some(bgl)], immediate_size: 0 });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let first_pipeline = pipeline("hzb_first", &first_bgl);
        let down_pipeline = pipeline("hzb_down", &down_bgl);
        Self { vis_buf, first_pipeline, down_pipeline, first_bgl, down_bgl, late_bgl, msaa, hzb: Mutex::new(None), device: device.clone() }
    }

    /// Make the pyramid for `depth` unless it exists (first frame, or the
    /// depth target changed: a resize). Nothing is created per frame.
    fn ensure(&self, depth: &wgpu::TextureView) {
        let mut g = self.hzb.lock().unwrap();
        if g.as_ref().is_some_and(|h| h.depth == *depth) {
            return;
        }
        let d = &self.device;
        let (w, h) = (depth.texture().width(), depth.texture().height());
        let w0 = w.div_ceil(2).max(1).next_power_of_two();
        let h0 = h.div_ceil(2).max(1).next_power_of_two();
        let mips = w0.max(h0).ilog2() + 1;
        let tex = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("occlusion-hzb"),
            size: wgpu::Extent3d { width: w0, height: h0, depth_or_array_layers: 1 },
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HZB_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        });
        let level = |m: u32| tex.create_view(&wgpu::TextureViewDescriptor { base_mip_level: m, mip_level_count: Some(1), ..Default::default() });
        let levels: Vec<wgpu::TextureView> = (0..mips).map(level).collect();
        let dims: Vec<(u32, u32)> = (0..mips).map(|m| ((w0 >> m).max(1), (h0 >> m).max(1))).collect();
        let first_bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hzb-first-bg"),
            layout: &self.first_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: if self.msaa { 2 } else { 0 }, resource: wgpu::BindingResource::TextureView(depth) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&levels[0]) },
            ],
        });
        let down_bgs = (1..mips as usize)
            .map(|m| {
                d.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("hzb-down-bg"),
                    layout: &self.down_bgl,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&levels[m - 1]) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&levels[m]) },
                    ],
                })
            })
            .collect();
        let view = tex.create_view(&Default::default());
        // The depth target's size, fixed for this pyramid (no per-frame write).
        let info = wgpu::util::DeviceExt::create_buffer_init(
            d,
            &wgpu::util::BufferInitDescriptor {
                label: Some("hzb-info"),
                contents: bytemuck::cast_slice(&[w as f32, h as f32, w0 as f32, h0 as f32]),
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        let late_bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hzb-test-bg"),
            layout: &self.late_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: info.as_entire_binding() },
            ],
        });
        let bytes = dims.iter().map(|(x, y)| *x as u64 * *y as u64 * 4).sum::<u64>() + 16;
        *g = Some(Hzb { depth: depth.clone(), view, dims, first_bg, down_bgs, late_bg, bytes });
    }

    /// Build the pyramid from `depth` (inside an open compute pass), then run
    /// `then` with the pyramid's test bind group (the late cull).
    pub fn build(&self, pass: &mut wgpu::ComputePass<'_>, depth: &wgpu::TextureView, then: impl FnOnce(&mut wgpu::ComputePass<'_>, &wgpu::BindGroup)) {
        self.ensure(depth);
        let g = self.hzb.lock().unwrap();
        let h = g.as_ref().unwrap();
        pass.set_pipeline(&self.first_pipeline);
        pass.set_bind_group(0, &h.first_bg, &[]);
        pass.dispatch_workgroups(h.dims[0].0.div_ceil(8), h.dims[0].1.div_ceil(8), 1);
        pass.set_pipeline(&self.down_pipeline);
        for (m, bg) in h.down_bgs.iter().enumerate() {
            let (x, y) = h.dims[m + 1];
            pass.set_bind_group(0, bg, &[]);
            pass.dispatch_workgroups(x.div_ceil(8), y.div_ceil(8), 1);
        }
        then(pass, &h.late_bg);
    }

    /// The pyramid (all levels) of the last depth target, for other culls.
    pub fn hzb_view(&self) -> Option<wgpu::TextureView> {
        self.hzb.lock().unwrap().as_ref().map(|h| h.view.clone())
    }

    /// GPU bytes: the per-object flags and the pyramid (once it exists).
    pub fn bytes(&self) -> u64 {
        self.vis_buf.size() + self.hzb.lock().unwrap().as_ref().map(|h| h.bytes).unwrap_or(0)
    }

    /// Forget last frame's visibility (every object goes through the late test).
    pub fn reset(&self, queue: &wgpu::Queue) {
        let zeros = vec![0u8; self.vis_buf.size() as usize];
        queue.write_buffer(&self.vis_buf, 0, &zeros);
    }
}
