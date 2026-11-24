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
//!
//! The levels' alpha is the trace's hierarchical depth (Hi-Z): level 1's the
//! occlusion pass's half-resolution view depth of the same frame (the trace's
//! own depth, `lk_ao_z`), each level after it the nearest of the texels under
//! it, rounded toward the eye (sky and nothing: `HIZ_FAR_M`). The trace
//! (sky_light.wgsl `sky_sr_trace`) skips a whole cell where its ray stays in
//! front of the cell's nearest depth: every texel of its path is still looked
//! at, in about a third of the steps the linear march took.

/// The pyramid's levels: the full resolution and seven halvings. The cone
/// reads levels 1-5 at most (1/32: a 60 x 34 level at 1080p, the widest lobe
/// the trace filters, `sky_light.wgsl SKY_SR_CONE_TOP`); the Hi-Z all of them
/// (its cells to 64 half-resolution texels).
pub(crate) const REFLECT_MIPS: u32 = 8;

/// The pyramid's pipelines (made once).
pub(crate) struct ReflectPyramid {
    bgl: wgpu::BindGroupLayout,
    /// level 1 (the taps held to the bloom's ceiling), and the levels after it.
    pipes: [wgpu::ComputePipeline; 2],
    lin: wgpu::Sampler,
    /// binding 4 of the levels past the first (they read no depth).
    z_dummy: wgpu::TextureView,
}

/// The size-dependent part: every level of each history, and the bind
/// groups that build them.
pub(crate) struct ReflectTargets {
    /// Every level of history p (the city pass's previous frame).
    pub(crate) all: [wgpu::TextureView; 2],
    /// Per parity, one per level 2.. (reads the level above, writes the level).
    bgs: [Vec<wgpu::BindGroup>; 2],
    /// Per parity, level 1's (the history's level 0 and the occlusion pass's
    /// depth, made for the depth texture the pass last wrote).
    first: std::sync::Mutex<Option<(wgpu::Texture, [wgpu::BindGroup; 2])>>,
    /// Level 1's groups with no depth (the stand-in: the alpha far).
    first_none: [wgpu::BindGroup; 2],
    /// Level 0 and level 1 of each history (level 1's group reads and writes them).
    level01: [[wgpu::TextureView; 2]; 2],
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
                entry(4, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
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
        let z_dummy = d
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("sky-reflect-z-dummy"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R32Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        Self { bgl, pipes: [pipe("rp_first"), pipe("rp_down")], lin, z_dummy }
    }

    fn bind(&self, d: &wgpu::Device, sky_uniform: &wgpu::Buffer, src: &wgpu::TextureView, dst: &wgpu::TextureView, z: &wgpu::TextureView) -> wgpu::BindGroup {
        d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky-reflect-pyramid-bg"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: sky_uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(src) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.lin) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(dst) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(z) },
            ],
        })
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
        let bgs = [0, 1].map(|p| (2..mips).map(|m| self.bind(d, sky_uniform, &level(&texs[p], m - 1), &level(&texs[p], m), &self.z_dummy)).collect());
        let level01 = [0, 1].map(|p| [level(&texs[p], 0), level(&texs[p], 1.min(mips - 1))]);
        let first_none = [0, 1].map(|p| self.bind(d, sky_uniform, &level01[p][0], &level01[p][1], &self.z_dummy));
        (hist, ReflectTargets { all, bgs, first: std::sync::Mutex::new(None), first_none, level01, sizes, bytes })
    }

    /// Build history `parity`'s levels (after the resolve wrote its level 0);
    /// `z`: the occlusion pass's half-resolution view depth this frame (the
    /// levels' Hi-Z alpha, level 1's; `None`: no depth, the alpha far).
    pub(crate) fn encode(&self, d: &wgpu::Device, sky_uniform: &wgpu::Buffer, enc: &mut wgpu::CommandEncoder, t: &ReflectTargets, parity: usize, z: Option<&wgpu::TextureView>) {
        if t.sizes.is_empty() {
            return;
        }
        let Some(z) = z else {
            Self::dispatch(&self.pipes, enc, &t.first_none[parity], &t.bgs[parity], &t.sizes);
            return;
        };
        let mut first = t.first.lock().unwrap();
        if first.as_ref().is_none_or(|(k, _)| k != z.texture()) {
            crate::hitch::mark(crate::hitch::Mark::BindGroupCreated, 2);
            *first = Some((z.texture().clone(), [0, 1].map(|p| self.bind(d, sky_uniform, &t.level01[p][0], &t.level01[p][1], z))));
        }
        let (_, bg1) = first.as_ref().expect("level 1's groups");
        Self::dispatch(&self.pipes, enc, &bg1[parity], &t.bgs[parity], &t.sizes);
    }

    fn dispatch(pipes: &[wgpu::ComputePipeline; 2], enc: &mut wgpu::CommandEncoder, first: &wgpu::BindGroup, rest: &[wgpu::BindGroup], sizes: &[(u32, u32)]) {
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("sky-reflect-pyramid"), timestamp_writes: None });
        pass.set_pipeline(&pipes[0]);
        pass.set_bind_group(0, first, &[]);
        pass.dispatch_workgroups(sizes[0].0.div_ceil(8), sizes[0].1.div_ceil(8), 1);
        pass.set_pipeline(&pipes[1]);
        for (bg, s) in rest.iter().zip(sizes[1..].iter()) {
            pass.set_bind_group(0, bg, &[]);
            pass.dispatch_workgroups(s.0.div_ceil(8), s.1.div_ceil(8), 1);
        }
    }
}

// ---- (`--lamp-streaks`) the cars' rear lamps as streak lights ----

/// The cars whose rear lamps streak a frame (two lamps each; sky_lamp_streaks.wgsl
/// `LS_CARS`, sky_light.wgsl `SKY_LS_LAMPS` = twice it).
pub(crate) const LS_CARS: u32 = 6;
/// A rear position (tail) lamp's intensity, cd: ECE R7's 4-17 cd on the axis
/// for a single lamp, their geometric mean.
pub(crate) const TAIL_LAMP_CD: f64 = 8.2;
/// A stop lamp's, cd: ECE R7's 60-185 cd, their geometric mean.
pub(crate) const STOP_LAMP_CD: f64 = 105.0;
/// The lamps' light cone round the car's rear axis, degrees (full within the
/// first, none past the second): ECE R7 asks a rear lamp be seen 80 degrees
/// outboard and 45 inboard, its intensity falling to a tenth or so at the edges.
pub(crate) const LAMP_CONE_DEG: [f64; 2] = [20.0, 75.0];
/// A rear lamp's lit lens, m^2 (a cluster's tail and stop segments, ~25 x 12 cm):
/// its size widens its streak's lobe as a street lamp's lens does.
pub(crate) const LAMP_AREA_M2: f64 = 0.03;
/// Where the lamps stand on a car with wheels: inboard of the rear wheels'
/// pivots (this share of their half-track), this far over them (m: a tail
/// lamp's centre ~0.75-0.85 m up on a car whose axle stands ~0.3 m up), this
/// far behind the rear axle (m: the rear overhang); on one without, half a
/// lane-width apart, this high, this share of its bounding radius back.
pub(crate) const LAMP_ON_WHEELS: [f64; 3] = [0.85, 0.45, 0.85];
pub(crate) const LAMP_NO_WHEELS: [f64; 3] = [0.62, 0.8, 0.85];
/// A car's rear is looked at within the cluster's range and the frustum widened by this (m).
pub(crate) const LAMP_VIEW_M: f64 = 10.0;
/// The rear lamps' records' kind (any >= 4: the streak loop's cone and lens size, sky_light.wgsl).
pub(crate) const LAMP_KIND: u32 = 64;
/// The agent's braking flag (agents_types.wgsl `AG_FLAG_BRAKING`).
pub(crate) const FLAG_BRAKING: u32 = 1;

/// The lens's red at unit luminance: ECE R7's red boundary lies outside the
/// sRGB gamut (x ~0.665-0.71, y ~0.29-0.335); clipped to it, its primary.
pub(crate) fn lamp_red() -> [f64; 3] {
    [1.0 / 0.2126, 0.0, 0.0]
}

/// The light gather's side (sky.rs `cine_lamps_wgsl`, with the streak pick):
/// the constants and sky_lamp_streaks.wgsl.
pub(crate) fn lamp_streaks_gather_wgsl() -> String {
    let v3 = |c: [f64; 3]| format!("vec3<f32>({:.6}, {:.6}, {:.6})", c[0], c[1], c[2]);
    format!(
        "\n// generated by sky_reflect.rs lamp_streaks_gather_wgsl (`--lamp-streaks`)\nconst LS_CARS: u32 = {LS_CARS}u;\nconst LS_FRAME_AT: u32 = {}u;\nconst LS_TAIL_CD: f32 = {TAIL_LAMP_CD:?};\nconst LS_STOP_CD: f32 = {STOP_LAMP_CD:?};\nconst LS_FLAG_BRAKING: u32 = {FLAG_BRAKING}u;\nconst LS_LAMP: vec3<f32> = {};\nconst LS_LAMP_NONE: vec3<f32> = {};\nconst LS_VIEW_M: f32 = {LAMP_VIEW_M:?};\nconst LS_RED: vec3<f32> = {};\nconst LS_COS: vec2<f32> = vec2<f32>({:.6}, {:.6});\nconst LS_KIND: u32 = {LAMP_KIND}u;\nconst LS_AREA_M2: f32 = {LAMP_AREA_M2:?};\n{}",
        crate::sky::FRAME_LIGHT_CAP,
        v3(LAMP_ON_WHEELS),
        v3(LAMP_NO_WHEELS),
        v3(lamp_red()),
        LAMP_CONE_DEG[1].to_radians().cos(),
        LAMP_CONE_DEG[0].to_radians().cos(),
        include_str!("sky_lamp_streaks.wgsl"),
    )
}

/// The rear lamps' room: their indices after the streak list's (sky.rs
/// `clusters`), their records after the frame list's cap (sky.rs `frame_lights`).
pub(crate) const fn lamp_streak_lamps() -> u64 {
    2 * LS_CARS as u64
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
        assert!(lean.len() < whole.len() && !lean.contains("fn sky_sr_trace") && !lean.contains("fn sky_w2_puddle") && !lean.contains("fn sky_wr_pond"));
        // every reflect-track name the rest of the city shader calls has a stand-in
        for name in ["fn sky_w2_on(", "fn sky_w2_env(", "fn sky_w2_street(", "fn sky_w2_generic(", "fn sky_sr_on(", "fn sky_sr_glass_mix(", "fn sky_sr_ssr(", "fn sky_sr_spec(", "const SKY_W2_FULL", "const SKY_W2_RING_LOBE", "var<private> sky_w2_pud_alpha", "var<private> sky_w2_ring", "var<private> sky_w2_water", "var<private> sky_sr_glass", "const SKY_WR_FULL", "var<private> sky_wr_alpha_k", "fn sky_ls_lamps("] {
            assert!(whole.contains(name) && lean.contains(name), "{name}");
        }
    }

    #[test]
    fn the_rear_lamps_follow_the_streak_list_past_the_frame_lists_cap() {
        let city = include_str!("sky_light.wgsl");
        // the city's reader and the gather agree on the room the list has past its cap
        assert!(city.contains(&format!("const SKY_LS_LAMPS: u32 = {}u;", 2 * super::LS_CARS)));
        assert_eq!(super::lamp_streak_lamps(), 2 * super::LS_CARS as u64);
        let gather = super::lamp_streaks_gather_wgsl();
        assert!(gather.contains(&format!("const LS_CARS: u32 = {}u;", super::LS_CARS)) && gather.contains("fn lights_lamp_streaks("));
        // the records past the frame list's cap, which the gather never fills (`lt_push`)
        assert!(gather.contains(&format!("const LS_FRAME_AT: u32 = {}u;", crate::sky::FRAME_LIGHT_CAP)));
        assert!(include_str!("sky_lights.wgsl").contains("if (i < lt.counts.z) {"));
        // a kind the streak loop draws as a cone with a lens of its size, no other light's
        assert!(super::LAMP_KIND >= 4 && super::LAMP_KIND != crate::shop_lights::SHOP_LIGHT_KIND && super::LAMP_KIND != crate::sky::BOUNCE_LIGHT_KIND);
        assert_eq!(super::FLAG_BRAKING, 1, "agents_types.wgsl AG_FLAG_BRAKING");
        assert!(include_str!("agents_types.wgsl").contains(&format!("const AG_FLAG_BRAKING: u32 = {}u;", super::FLAG_BRAKING)));
        // the red at unit luminance
        let r = super::lamp_red();
        assert!((0.2126 * r[0] + 0.7152 * r[1] + 0.0722 * r[2] - 1.0).abs() < 1e-9);
        // a stop lamp an order of magnitude over a tail lamp, both within ECE R7's ranges
        assert!((4.0..=17.0).contains(&super::TAIL_LAMP_CD) && (60.0..=185.0).contains(&super::STOP_LAMP_CD));
    }
}
