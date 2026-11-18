//! stage 3, the far city's street lamps by night: one point sprite
//! per lamp of the citywide set the sky slice is given (`set_lamps`: the JS
//! fallback placement on the street contract's segments, standing on the
//! drawn surface), drawn past `NEAR_M` (where the frame's own gathered lights,
//! at most `sky::FRAME_LIGHT_CAP`, draw their luminaires and pools), faded in
//! to `FULL_M`. Energy-kept (the lamp's intensity over the area its pixels
//! cover), fogged as the city is, additive in the HDR target after the sky,
//! depth-tested against the city. The far cells' and districts' lit windows
//! are their own shader's (hlod_draw.wgsl `hl_windows`).

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

pub const NEAR_M: f32 = 450.0;
pub const FULL_M: f32 = 650.0;
/// Sprite radius, pixels.
pub const RADIUS_PX: f32 = 1.6;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct FarLamp {
    pub pos: [f32; 3],
    pub candela: f32,
    pub color: [f32; 3],
    pub pad: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    range: [f32; 4],
    px: [f32; 4],
}

pub const WGSL: &str = include_str!("hlod_lights.wgsl");

pub struct FarLights {
    pipeline: wgpu::RenderPipeline,
    bg: wgpu::BindGroup,
    params: wgpu::Buffer,
    pub count: u32,
    pub bytes: u64,
}

pub struct FarLightsInit<'a> {
    pub device: &'a wgpu::Device,
    pub draw_bgl: &'a wgpu::BindGroupLayout,
    pub main_bgl: &'a wgpu::BindGroupLayout,
    pub sample_count: u32,
    pub debug_ids: bool,
    pub lamps: &'a [eo_sky::lights::LampLight],
}

impl FarLights {
    pub fn new(i: &FarLightsInit) -> Self {
        let d = i.device;
        let lamps: Vec<FarLamp> = i
            .lamps
            .iter()
            .map(|l| FarLamp { pos: [l.pos[0] as f32, l.pos[1] as f32, l.pos[2] as f32], candela: l.candela as f32, color: [l.color[0] as f32, l.color[1] as f32, l.color[2] as f32], pad: 0.0 })
            .collect();
        let data: Vec<FarLamp> = if lamps.is_empty() { vec![FarLamp::zeroed()] } else { lamps.clone() };
        let buf = d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("hlod-far-lamps"), contents: bytemuck::cast_slice(&data), usage: wgpu::BufferUsages::STORAGE });
        let params = d.create_buffer(&wgpu::BufferDescriptor { label: Some("hlod-far-lamp-params"), size: std::mem::size_of::<Params>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hlod-far-lamps-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: vf, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: vf, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
            ],
        });
        let bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hlod-far-lamps-bg"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: params.as_entire_binding() }],
        });
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("hlod_lights.wgsl"), source: wgpu::ShaderSource::Wgsl(format!("{}\n{WGSL}", crate::look::city_shader_source()).into()) });
        let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("hlod-far-lamps-layout"), bind_group_layouts: &[Some(i.draw_bgl), Some(i.main_bgl), Some(&bgl)], immediate_size: 0 });
        let add = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
        };
        let color = wgpu::ColorTargetState { format: crate::look::HDR_FORMAT, blend: Some(add), write_mask: wgpu::ColorWrites::ALL };
        let ids = wgpu::ColorTargetState { format: crate::IDS_FORMAT, blend: None, write_mask: wgpu::ColorWrites::empty() };
        let targets_ids = [Some(color.clone()), Some(ids)];
        let targets = [Some(color)];
        let pipeline = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("hlod-far-lamps"),
            layout: Some(&layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("far_lamp_vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: crate::DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState { count: i.sample_count, ..Default::default() },
            fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("far_lamp_fs"), compilation_options: Default::default(), targets: if i.debug_ids { &targets_ids } else { &targets } }),
            multiview_mask: None,
            cache: None,
        });
        Self { pipeline, bg, params, count: lamps.len() as u32, bytes: (data.len() * std::mem::size_of::<FarLamp>()) as u64 }
    }

    /// The frame's pixel scale (2 tan(fov/2) / output height).
    pub fn update(&self, queue: &wgpu::Queue, fov_y_deg: f32, height: u32, on: bool) {
        let px = 2.0 * (fov_y_deg.to_radians() * 0.5).tan() / height.max(1) as f32;
        let p = Params { range: [NEAR_M, FULL_M, RADIUS_PX, if on { 1.0 } else { 0.0 }], px: [px, self.count as f32, 0.0, 0.0] };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&p));
    }

    /// After the sky, in the city pass (groups 0 and 1 bound).
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(2, &self.bg, &[]);
        pass.draw(0..6, 0..self.count);
    }
}
