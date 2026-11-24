//! (`--screen-reflect`, the reflect track) The reflection source pyramid.
//!
//! The screen-space reflection reads the previous resolved frame. A glossy
//! lobe covers many pixels of it, and reading it with a handful of jittered
//! taps left speckle in every reflection of a small bright thing (tail lamps,
//! lit windows, signals) that the history's neighbourhood clamp keeps. So
//! the two resolved histories carry their own mip chain while the switch is
//! compiled in: each level the 13-tap mean of the one above
//! (sky_reflect.wgsl), built after each resolve, and the city pass binds every
//! level of the previous one (`SkyGpu::reflect_view`) and reads it at the
//! lobe's footprint (sky_light.wgsl `sky_sr_cone`). The histories' level 0
//! and every other reader are unchanged: their views stay level 0 alone.
//! Made only with SF_SCREEN_REFLECT compiled in (`look::shader_features`), the
//! levels built only while the switch is on.

/// The pyramid's levels: the full resolution and five halvings (1/32: a
/// 60 x 34 level at 1080p, the widest lobe the trace filters).
pub(crate) const REFLECT_MIPS: u32 = 6;

/// The pyramid's pipelines (made once).
pub(crate) struct ReflectPyramid {
    bgl: wgpu::BindGroupLayout,
    /// level 1 (the taps held to the bloom's ceiling), and the levels after it.
    pipes: [wgpu::ComputePipeline; 2],
    lin: wgpu::Sampler,
}

/// The size-dependent part: every level of each history, and the bind
/// groups that build them.
pub(crate) struct ReflectTargets {
    /// Every level of history p (the city pass's previous frame).
    pub(crate) all: [wgpu::TextureView; 2],
    /// Per parity, one per level 1.. (reads the level above, writes the level).
    bgs: [Vec<wgpu::BindGroup>; 2],
    sizes: Vec<(u32, u32)>,
    /// The levels' bytes past level 0, both histories.
    pub(crate) bytes: u64,
}

impl ReflectPyramid {
    pub(crate) fn new(d: &wgpu::Device) -> Self {
        let cs = wgpu::ShaderStages::COMPUTE;
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry { binding, visibility: cs, ty, count: None };
        let bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky-reflect-pyramid-bgl"),
            entries: &[
                entry(0, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
                entry(1, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                entry(2, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                entry(3, wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: wgpu::TextureFormat::Rgba16Float, view_dimension: wgpu::TextureViewDimension::D2 }),
            ],
        });
        let src = format!("{}\n{}", include_str!("sky_common.wgsl"), include_str!("sky_reflect.wgsl"));
        let m = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("sky_reflect.wgsl"), source: wgpu::ShaderSource::Wgsl(src.into()) });
        let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("sky-reflect-pyramid-layout"), bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
        let pipe = |e: &str| d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(e), layout: Some(&layout), module: &m, entry_point: Some(e), compilation_options: Default::default(), cache: None });
        let lin = d.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sky-reflect-pyramid-linear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self { bgl, pipes: [pipe("rp_first"), pipe("rp_down")], lin }
    }

    /// The two histories with their levels: level-0 views (the resolve's
    /// target and every other reader, as without the pyramid) and the targets.
    /// `format` must be Rgba16Float (the levels are written as storage).
    pub(crate) fn histories(&self, d: &wgpu::Device, sky_uniform: &wgpu::Buffer, size: (u32, u32), format: wgpu::TextureFormat, usage: wgpu::TextureUsages) -> ([wgpu::TextureView; 2], ReflectTargets) {
        let mips = REFLECT_MIPS.min(32 - size.0.max(size.1).max(1).leading_zeros());
        let make = |label: &str| {
            d.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: size.0.max(1), height: size.1.max(1), depth_or_array_layers: 1 },
                mip_level_count: mips,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: usage | wgpu::TextureUsages::STORAGE_BINDING,
                view_formats: &[],
            })
        };
        let texs = [make("sky-history-0"), make("sky-history-1")];
        let level = |t: &wgpu::Texture, m: u32| t.create_view(&wgpu::TextureViewDescriptor { base_mip_level: m, mip_level_count: Some(1), ..Default::default() });
        let hist = [level(&texs[0], 0), level(&texs[1], 0)];
        let all = [texs[0].create_view(&Default::default()), texs[1].create_view(&Default::default())];
        let sizes: Vec<(u32, u32)> = (1..mips).map(|m| ((size.0 >> m).max(1), (size.1 >> m).max(1))).collect();
        let bytes = 2 * sizes.iter().map(|s| s.0 as u64 * s.1 as u64 * 8).sum::<u64>();
        let bgs = [0, 1].map(|p| {
            (1..mips)
                .map(|m| {
                    d.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("sky-reflect-pyramid-bg"),
                        layout: &self.bgl,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: sky_uniform.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&level(&texs[p], m - 1)) },
                            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.lin) },
                            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&level(&texs[p], m)) },
                        ],
                    })
                })
                .collect()
        });
        (hist, ReflectTargets { all, bgs, sizes, bytes })
    }

    /// Build history `parity`'s levels (after the resolve wrote its level 0).
    pub(crate) fn encode(&self, enc: &mut wgpu::CommandEncoder, t: &ReflectTargets, parity: usize) {
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("sky-reflect-pyramid"), timestamp_writes: None });
        for (i, (bg, s)) in t.bgs[parity].iter().zip(t.sizes.iter()).enumerate() {
            pass.set_pipeline(&self.pipes[if i == 0 { 0 } else { 1 }]);
            pass.set_bind_group(0, bg, &[]);
            pass.dispatch_workgroups(s.0.div_ceil(8), s.1.div_ceil(8), 1);
        }
    }
}

/// The start of sky_light.wgsl's reflect-track sections (`--wet-surfaces`,
/// then `--screen-reflect`, to the end of the file).
const SECTIONS_MARK: &str = "// ---- (`--wet-surfaces`, the reflect track;";

/// sky_light.wgsl for a shader variant: whole where `reflect`; else (the
/// agents' variant, look.rs `city_shader_source_variant_probes`) with the
/// reflect track's sections replaced by their off stand-ins
/// (sky_reflect_stub.wgsl): agent-opaque stands at Apple's shader compiler's
/// limit, and every function in the module costs it compile time, called or not.
pub(crate) fn sky_light_source(reflect: bool) -> std::borrow::Cow<'static, str> {
    const SRC: &str = include_str!("sky_light.wgsl");
    if reflect {
        return SRC.into();
    }
    let at = SRC.find(SECTIONS_MARK).expect("sky_light.wgsl: the reflect track's sections");
    format!("{}{}", &SRC[..at], include_str!("sky_reflect_stub.wgsl")).into()
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_agents_variant_takes_the_stand_ins_and_every_other_the_whole_file() {
        let whole = super::sky_light_source(true);
        assert_eq!(whole, include_str!("sky_light.wgsl"));
        let lean = super::sky_light_source(false);
        assert!(lean.len() < whole.len() && !lean.contains("fn sky_sr_trace") && !lean.contains("fn sky_w2_puddle"));
        // every reflect-track name the rest of the city shader calls has a stand-in
        for name in ["fn sky_w2_on(", "fn sky_w2_env(", "fn sky_w2_street(", "fn sky_w2_generic(", "fn sky_sr_on(", "fn sky_sr_glass_mix(", "fn sky_sr_ssr(", "const SKY_W2_FULL", "const SKY_W2_RING_LOBE", "var<private> sky_w2_pud_alpha", "var<private> sky_w2_ring", "var<private> sky_w2_water", "var<private> sky_sr_glass"] {
            assert!(whole.contains(name) && lean.contains(name), "{name}");
        }
    }
}
