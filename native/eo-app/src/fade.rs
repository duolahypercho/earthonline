//! The landmark lift's fade: one full-screen triangle of black over the
//! finished frame, blended by the blend constant (eo-player `lift::fade`:
//! 0 clear to 1 black), so no buffer or bind group is needed. Nothing is
//! drawn while the fade is 0.

use eo_render::wgpu;

const SHADER: &str = r#"
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(i & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs() -> @location(0) vec4<f32> {
    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
}
"#;

pub struct Fade {
    pipeline: wgpu::RenderPipeline,
}

impl Fade {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("lift-fade"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("lift-fade"), bind_group_layouts: &[], immediate_size: 0 });
        // out = black * k + frame * (1 - k), k the blend constant.
        let blend = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Constant, dst_factor: wgpu::BlendFactor::OneMinusConstant, operation: wgpu::BlendOperation::Add };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("lift-fade"),
            layout: Some(&layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState { color: blend, alpha: blend }), write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Self { pipeline }
    }

    /// Darken `view` by `k` (0..1); nothing when `k` is 0.
    pub fn draw(&self, enc: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, k: f64) {
        if !(k > 1e-4) {
            return;
        }
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("lift-fade"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        let k = k.min(1.0);
        pass.set_blend_constant(wgpu::Color { r: k, g: k, b: k, a: k });
        pass.draw(0..3, 0..1);
    }
}

/// The prompt's words at a lift stop.
pub fn prompt_text(p: &eo_player::lift::LiftPrompt) -> String {
    if p.up { format!("Ride up to the {}", p.name) } else { format!("Ride down from the {}", p.name) }
}
