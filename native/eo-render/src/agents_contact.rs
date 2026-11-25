//! (`--car-contact`) The cars' occlusion of the street round them
//! (agents_contact.wgsl): one pass after the city pass, before the water and
//! the occlusion pass, that multiplies the HDR target under and round each
//! car near the eye by the share of the light there the car's body takes
//! away (the form factor of its occluder boxes, eo-agents
//! `vehicles::contact_shape`). Made only with the switch on: without it the
//! agents' module, pipelines and frame are as before.

use std::sync::Mutex;

/// The pass's pipeline (in the agents' draw module and layout, with the
/// depth as group 3) and the depth's bind group, kept while the depth
/// texture is the same.
pub(crate) struct Contact {
    pipeline: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    bind: Mutex<Option<(wgpu::Texture, wgpu::BindGroup)>>,
}

impl Contact {
    /// `module`: the agents' draw module with agents_contact.wgsl appended;
    /// `groups`: its groups 0..2 (the agents' draw group, the look's, the agent data).
    pub(crate) fn new(device: &wgpu::Device, module: &wgpu::ShaderModule, groups: [&wgpu::BindGroupLayout; 3]) -> Self {
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("agent-contact-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("agent-contact-layout"),
            bind_group_layouts: &[Some(groups[0]), Some(groups[1]), Some(groups[2]), Some(&bgl)],
            immediate_size: 0,
        });
        // dst x src: the colour multiplied by what the car leaves, alpha kept.
        let multiply = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::Src, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("agent-contact"),
            layout: Some(&layout),
            vertex: wgpu::VertexState { module, entry_point: Some("ag_contact_vs"), compilation_options: Default::default(), buffers: &[] },
            // the box's back faces: every pixel whose ray crosses the box, once, the eye inside it or not.
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: Some(wgpu::Face::Front), ..Default::default() },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module,
                entry_point: Some("ag_contact_fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: crate::look::HDR_FORMAT, blend: Some(multiply), write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self { pipeline, bgl, bind: Mutex::new(None) }
    }

    /// Whether the pass can read this depth (single-sample, sampleable).
    pub(crate) fn reads(depth: &wgpu::TextureView) -> bool {
        let t = depth.texture();
        t.sample_count() == 1 && t.usage().contains(wgpu::TextureUsages::TEXTURE_BINDING)
    }

    /// The pass over `hdr` (single-sample) with `count` agents, on `depth`;
    /// `draw_bg` and `data_bg` the agents' groups 0 and 2 (the look binds 1).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode(&self, device: &wgpu::Device, enc: &mut wgpu::CommandEncoder, draw_bg: &wgpu::BindGroup, data_bg: &wgpu::BindGroup, look: &crate::look::LookGpu, hdr: &wgpu::TextureView, depth: &wgpu::TextureView, count: u32) {
        if count == 0 || !Self::reads(depth) || hdr.texture().sample_count() != 1 {
            return;
        }
        let dt = depth.texture();
        let mut bind = self.bind.lock().unwrap();
        if bind.as_ref().is_none_or(|(t, _)| t != dt) {
            crate::hitch::mark(crate::hitch::Mark::BindGroupCreated, 1);
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("agent-contact-bg"),
                layout: &self.bgl,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(depth) }],
            });
            *bind = Some((dt.clone(), bg));
        }
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("agents-contact"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: hdr, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, draw_bg, &[]);
        look.bind_main(&mut pass);
        pass.set_bind_group(2, data_bg, &[]);
        pass.set_bind_group(3, &bind.as_ref().unwrap().1, &[]);
        pass.draw(0..36, 0..count);
    }
}
