//! The opt-in lighting additions' shared plumbing: their own timestamp
//! query set and its slots. The options are `look::LightingOptions`; group 1
//! bindings 12-18 (`look::LightingBindings`) and the `Look.lx` rows
//! (`look::LX_*`) are in look.rs; the hooks are `far_shadow`, `ambient`,
//! `light_shadow`, `lit_fog` and `ProbeGpu::encode_grid`.
//!
//! The set is separate from the renderer's (`crate::QUERIES_PER_FRAME`
//! stays as it is), like the props' own. When the bench hands one to the
//! renderer (`Renderer::set_lighting_timestamps`) every hook writes its
//! begin/end pair each frame, an empty span when its feature is off, so the
//! set always resolves whole. Without one a hook whose feature is off
//! records nothing.

use std::sync::Mutex;

/// Timestamps the lighting hooks write per frame: begin/end pairs.
pub const QUERIES_PER_FRAME: u32 = 12;
/// The far sun shadow map (`far_shadow`).
pub const Q_FAR_SHADOW: u32 = 0;
/// The ambient volume's uploads (`ambient`).
pub const Q_AMBIENT_UPLOAD: u32 = 2;
/// The probe grid (`ProbeGpu::encode_grid`).
pub const Q_PROBE_GRID: u32 = 4;
/// The street lamps' shadow maps (`light_shadow`).
pub const Q_LIGHT_SHADOWS: u32 = 6;
/// The headlights' shadow maps (`light_shadow`).
pub const Q_HEADLIGHT_SHADOWS: u32 = 8;
/// The fog lit by the local lights (`lit_fog`).
pub const Q_LIT_FOG: u32 = 10;
/// Every pass and its slot, in slot order (the bench's `lighting_passes_ms` names).
pub const PASSES: [(&str, u32); 6] = [
    ("far_shadow", Q_FAR_SHADOW),
    ("ambient_upload", Q_AMBIENT_UPLOAD),
    ("probe_grid", Q_PROBE_GRID),
    ("light_shadows", Q_LIGHT_SHADOWS),
    ("headlight_shadows", Q_HEADLIGHT_SHADOWS),
    ("lit_fog", Q_LIT_FOG),
];

/// This frame's lighting timestamps: the set and its first slot for this frame.
pub type Ts<'a> = Option<(&'a wgpu::QuerySet, u32)>;

/// Begin and end of `slot` for a compute pass.
pub fn compute_writes(ts: Ts<'_>, slot: u32) -> Option<wgpu::ComputePassTimestampWrites<'_>> {
    ts.map(|(q, b)| wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(b + slot), end_of_pass_write_index: Some(b + slot + 1) })
}

/// Begin and end of `slot` for a render pass.
pub fn render_writes(ts: Ts<'_>, slot: u32) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
    ts.map(|(q, b)| wgpu::RenderPassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(b + slot), end_of_pass_write_index: Some(b + slot + 1) })
}

/// An empty span in `slot` (an empty compute pass carrying both writes),
/// when the frame is timed; nothing otherwise.
pub fn empty_span(enc: &mut wgpu::CommandEncoder, ts: Ts<'_>, slot: u32, label: &str) {
    if ts.is_some() {
        enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some(label), timestamp_writes: compute_writes(ts, slot) });
    }
}

/// The set the bench hands the renderer for the coming encodes, and the
/// first slot of the coming frame (`QUERIES_PER_FRAME` a frame).
#[derive(Default)]
pub struct LightingTimer(Mutex<Option<(wgpu::QuerySet, u32)>>);

impl LightingTimer {
    pub fn set(&self, ts: Option<(wgpu::QuerySet, u32)>) {
        *self.0.lock().unwrap() = ts;
    }

    pub fn get(&self) -> Option<(wgpu::QuerySet, u32)> {
        self.0.lock().unwrap().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::{self, LightingOptions};
    use wgpu::naga;

    fn parse(label: &str, src: &str) -> naga::Module {
        let module = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(src)));
        module
    }

    /// Every shader of a pipeline on `look-bgl` that can be composed without a GPU.
    fn group1_shaders() -> Vec<(&'static str, String)> {
        vec![
            ("city", look::city_shader_source()),
            ("city, far layer", look::city_shader_source_far()),
            ("surf", crate::surf::shader_source()),
            // the far shore's, the water's and the prop wires' are composed in their `new`; this is their part.
            ("look common + sky light core + look ext", format!("{}\n{}\n{}\n@group(1) @binding(0) var<uniform> look: Look;\n", look::look_common_source(), include_str!("sky_light_core.wgsl"), look::LOOK_EXT_WGSL)),
        ]
    }

    #[test]
    fn options_are_off_by_default_and_each_sets_its_own_bit() {
        assert_eq!(LightingOptions::default(), LightingOptions::OFF);
        assert_eq!(look::LookOptions::default().lighting, LightingOptions::OFF);
        let off = LightingOptions::OFF;
        assert!(!off.any());
        assert_eq!(off.mask(), 0);
        assert_eq!(off.flags_word().to_bits(), 0, "row 0 x stays zero while everything is off");
        let one = [
            (LightingOptions { cascade_blend: true, ..off }, look::LX_BIT_CASCADE_BLEND),
            (LightingOptions { far_shadows: 1, ..off }, look::LX_BIT_FAR_SHADOWS),
            (LightingOptions { ambient_volume: 2, ..off }, look::LX_BIT_AMBIENT_VOLUME),
            (LightingOptions { probe_grid: 1, ..off }, look::LX_BIT_PROBE_GRID),
            (LightingOptions { light_shadows: 3, ..off }, look::LX_BIT_LIGHT_SHADOWS),
            (LightingOptions { headlight_shadows: 8, ..off }, look::LX_BIT_HEADLIGHT_SHADOWS),
            (LightingOptions { lit_fog: 2, ..off }, look::LX_BIT_LIT_FOG),
            (LightingOptions { smooth_time: true, ..off }, look::LX_BIT_SMOOTH_TIME),
        ];
        let mut all = 0;
        for (o, bit) in one {
            assert!(o.any());
            assert_eq!(o.mask(), bit, "{o:?}");
            let w = o.flags_word();
            assert!(w.is_normal(), "{o:?}: the flags word is a normal float ({w:e})");
            assert_eq!(w.to_bits() & 0xFF, bit, "{o:?}: the shader's bitcast reads the bit");
            all |= bit;
        }
        assert_eq!(all, 0xFF, "eight distinct bits");
        // light_shadow_moving refines light_shadows: no bit of its own.
        assert_eq!(LightingOptions { light_shadow_moving: true, ..off }.mask(), 0);
        assert_eq!(LightingOptions { light_shadows: 3, ..off }.shadowed_lamps(), 48);
    }

    #[test]
    fn the_look_uniform_matches_its_wgsl_struct() {
        let m = parse("city", &look::city_shader_source());
        let (_, ty) = m.types.iter().find(|(_, t)| t.name.as_deref() == Some("Look")).expect("struct Look");
        let naga::TypeInner::Struct { members, span } = &ty.inner else { panic!("Look is a struct") };
        assert_eq!(*span as usize, look::LOOK_UNIFORM_BYTES, "WGSL Look and Rust LookUniform have the same size");
        let lx = members.iter().find(|s| s.name.as_deref() == Some("lx")).expect("Look.lx");
        assert_eq!(lx.offset as usize, look::LOOK_LX_OFFSET, "lx at the same offset");
        assert_eq!(members.last().map(|s| s.name.as_deref()), Some(Some("lx")), "lx is the last member (appended after wx_tint)");
        let naga::TypeInner::Array { size, stride, .. } = m.types[lx.ty].inner else { panic!("lx is an array") };
        assert_eq!((size, stride), (naga::ArraySize::Constant(std::num::NonZeroU32::new(look::LX_LEN as u32).unwrap()), 16));
        assert_eq!(look::LOOK_LX_OFFSET + look::LX_LEN * 16, look::LOOK_UNIFORM_BYTES);
        // the bit constants agree.
        for (name, bit) in [
            ("LX_BIT_CASCADE_BLEND", look::LX_BIT_CASCADE_BLEND),
            ("LX_BIT_FAR_SHADOWS", look::LX_BIT_FAR_SHADOWS),
            ("LX_BIT_AMBIENT_VOLUME", look::LX_BIT_AMBIENT_VOLUME),
            ("LX_BIT_PROBE_GRID", look::LX_BIT_PROBE_GRID),
            ("LX_BIT_LIGHT_SHADOWS", look::LX_BIT_LIGHT_SHADOWS),
            ("LX_BIT_HEADLIGHT_SHADOWS", look::LX_BIT_HEADLIGHT_SHADOWS),
            ("LX_BIT_LIT_FOG", look::LX_BIT_LIT_FOG),
            ("LX_BIT_SMOOTH_TIME", look::LX_BIT_SMOOTH_TIME),
        ] {
            let (_, c) = m.constants.iter().find(|(_, c)| c.name.as_deref() == Some(name)).unwrap_or_else(|| panic!("WGSL {name}"));
            assert_eq!(m.global_expressions[c.init], naga::Expression::Literal(naga::Literal::U32(bit)), "{name}");
        }
    }

    #[test]
    fn group1_bindings_12_to_18_match_the_layout_in_every_shader() {
        let layout = look::LightingBindings::layout_entries();
        for (label, src) in group1_shaders() {
            let m = parse(label, &src);
            for e in &layout {
                assert_eq!(e.visibility, wgpu::ShaderStages::FRAGMENT, "binding {}: fragment only", e.binding);
                let found: Vec<_> = m.global_variables.iter().filter(|(_, g)| g.binding.as_ref().is_some_and(|b| b.group == 1 && b.binding == e.binding)).collect();
                assert_eq!(found.len(), 1, "{label}: group 1 binding {} declared once", e.binding);
                let g = found[0].1;
                let inner = &m.types[g.ty].inner;
                let ok = match e.ty {
                    wgpu::BindingType::Texture { sample_type, view_dimension, multisampled: false } => {
                        let (dim, arrayed) = match view_dimension {
                            wgpu::TextureViewDimension::D2 => (naga::ImageDimension::D2, false),
                            wgpu::TextureViewDimension::D2Array => (naga::ImageDimension::D2, true),
                            wgpu::TextureViewDimension::D3 => (naga::ImageDimension::D3, false),
                            other => panic!("binding {}: unexpected {other:?}", e.binding),
                        };
                        let class = match sample_type {
                            wgpu::TextureSampleType::Depth => naga::ImageClass::Depth { multi: false },
                            wgpu::TextureSampleType::Float { .. } => naga::ImageClass::Sampled { kind: naga::ScalarKind::Float, multi: false },
                            other => panic!("binding {}: unexpected {other:?}", e.binding),
                        };
                        *inner == naga::TypeInner::Image { dim, arrayed, class }
                    }
                    wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, .. } => {
                        g.space == naga::AddressSpace::Storage { access: naga::StorageAccess::LOAD }
                            && matches!(inner, naga::TypeInner::Array { size: naga::ArraySize::Dynamic, stride: 16, .. })
                    }
                    other => panic!("binding {}: unexpected {other:?}", e.binding),
                };
                assert!(ok, "{label}: group 1 binding {} is {inner:?} in {:?}, the layout says {:?}", e.binding, g.space, e.ty);
            }
        }
    }

    #[test]
    fn slots_are_disjoint_pairs_inside_the_frame() {
        let mut seen = vec![false; QUERIES_PER_FRAME as usize];
        for (name, q) in PASSES {
            assert_eq!(q % 2, 0, "{name}: a begin/end pair starts on an even slot");
            for s in [q, q + 1] {
                assert!(s < QUERIES_PER_FRAME, "{name}: slot {s} past the frame's {QUERIES_PER_FRAME}");
                assert!(!seen[s as usize], "{name}: slot {s} taken twice");
                seen[s as usize] = true;
            }
        }
        assert!(seen.iter().all(|s| *s), "every slot is written each frame (the set resolves whole)");
    }

    /// (stage P1) The feature sets a run can compile: none (the default),
    /// each feature alone, every one (the process default).
    fn feature_sets() -> Vec<(String, look::ShaderFeatures)> {
        let none = look::ShaderFeatures::NONE;
        let mut sets = vec![("none".to_string(), none), ("all".to_string(), look::ShaderFeatures::ALL)];
        for bit in 0..8 {
            sets.push((format!("lx bit {bit}"), look::ShaderFeatures { lx_bits: 1 << bit, ..none }));
        }
        sets.push(("grime".into(), look::ShaderFeatures { grime: true, ..none }));
        sets.push(("street life".into(), look::ShaderFeatures { grime: true, street_life: true, ..none }));
        sets.push(("structures".into(), look::ShaderFeatures { structures: true, ..none }));
        sets.push(("tunnels".into(), look::ShaderFeatures { structures: true, tunnels: true, ..none }));
        sets.push(("shop light".into(), look::ShaderFeatures { shop_light: true, ..none }));
        sets.push(("facade clutter".into(), look::ShaderFeatures { facade_clutter: true, ..none }));
        sets.push(("cinematic".into(), look::ShaderFeatures { cinematic: true, ..none })); // (`--look cinematic`, stage L0)
        sets
    }

    #[test]
    fn every_feature_set_composes_valid_lit_shaders() {
        for (name, f) in feature_sets() {
            look::with_shader_features(f, || {
                assert_eq!(look::shader_features(), f, "{name}: the scope's set is read");
                let common = look::look_common_source();
                assert!(common.contains(&format!("const SF_LX_BITS: u32 = {}u;", f.lx_bits)), "{name}: the constants are generated");
                assert!(common.contains(&format!("const SF_GRIME: bool = {};", f.grime)), "{name}");
                let ls = look::city_shader_source().contains("const LS_SHADER: bool = true;");
                assert_eq!(ls, f.light_shadows(), "{name}: the lamp shadow test is compiled in with its option only");
                for (label, src) in group1_shaders() {
                    parse(&format!("{name}: {label}"), &src);
                }
                let props = format!("{}\n{}\n{}", look::city_shader_source_variant_probes(crate::ambient::ShaderLevels::Plane, look::ShadowLevels::Near, false), include_str!("props_types.wgsl"), include_str!("props_draw.wgsl"));
                parse(&format!("{name}: props"), &props);
                let agents = format!(
                    "{}\n{}\n{}\n{}\n{}\n{}",
                    look::city_shader_source_variant_probes(crate::ambient::ShaderLevels::Full, look::ShadowLevels::Full, false),
                    crate::agents::people_shading_wgsl(), // (`--people-shading`)
                    include_str!("agents_types.wgsl"),
                    include_str!("agents_common.wgsl"),
                    include_str!("agents_draw.wgsl"),
                    include_str!("sky_velocity.wgsl")
                );
                parse(&format!("{name}: agents"), &agents);
            });
        }
        assert_eq!(look::shader_features(), look::ShaderFeatures::ALL, "the scope ends with its body; unset, every feature is in");
    }

    #[test]
    fn shader_features_follow_the_options_and_limit_them() {
        let o = look::LookOptions::default();
        assert_eq!(look::ShaderFeatures::of(&o, false, false, false), look::ShaderFeatures::NONE, "the default options compile nothing in");
        assert!(!look::ShaderFeatures::of(&o, false, true, false).tunnels, "no tunnels without the structures");
        let on = look::LookOptions {
            lighting: LightingOptions { cascade_blend: true, far_shadows: 2, ambient_volume: 2, probe_grid: 2, light_shadows: 3, light_shadow_moving: true, headlight_shadows: 8, lit_fog: 2, smooth_time: true, ground_bounce: true },
            grime: 2,
            street_life: 1,
            sky: crate::sky::SkyOptions { shop_light: 2, ..o.sky },
            ..o
        };
        let f = look::ShaderFeatures::of(&on, true, true, true);
        assert_eq!(f, look::ShaderFeatures { lx_bits: 0xFF, cinematic: false, ..look::ShaderFeatures::ALL }); // (the look profile is its own, below)
        let same = |a: &look::LookOptions, b: &look::LookOptions| format!("{a:?}") == format!("{b:?}"); // (LookOptions has no PartialEq)
        assert!(same(&f.limit(&on), &on), "what is compiled in is left as asked");
        assert!(same(&look::ShaderFeatures::ALL.limit(&on), &on));
        let none = look::ShaderFeatures::NONE.limit(&on);
        assert_eq!(none.lighting, LightingOptions::OFF, "a feature compiled out stays off");
        assert_eq!((none.grime, none.street_life, none.sky.shop_light), (0, 0, 0));
        assert!(same(&none, &look::LookOptions { lighting: LightingOptions::OFF, grime: 0, street_life: 0, sky: o.sky, ..on }), "nothing else changes");
        // each lighting bit keeps only its own option.
        let blend_only = look::ShaderFeatures { lx_bits: look::LX_BIT_CASCADE_BLEND, ..look::ShaderFeatures::NONE }.limit(&on);
        assert_eq!(blend_only.lighting, LightingOptions { cascade_blend: true, ..LightingOptions::OFF });
    }

    #[test]
    fn the_cinematic_look_is_compiled_in_only_when_asked_and_limited_with_it() {
        // (`--look cinematic`, stage L0) the default compiles SF_CINEMATIC out; the profile compiles it in.
        let o = look::LookOptions::default();
        assert!(!look::ShaderFeatures::of(&o, false, false, false).cinematic);
        assert!(look::ShaderFeatures::NONE.wgsl().contains("const SF_CINEMATIC: bool = false;"));
        assert!(look::ShaderFeatures::ALL.wgsl().contains("const SF_CINEMATIC: bool = true;"));
        let on = look::LookOptions { look: eo_look::cinematic::LookProfile::Cinematic, sky: crate::sky::SkyOptions { cinematic: true, ..o.sky }, ..o };
        let f = look::ShaderFeatures::of(&on, false, false, false);
        assert_eq!(f, look::ShaderFeatures { cinematic: true, ..look::ShaderFeatures::NONE }, "the profile alone compiles in nothing else");
        let same = |a: &look::LookOptions, b: &look::LookOptions| format!("{a:?}") == format!("{b:?}");
        assert!(same(&f.limit(&on), &on), "compiled in, it stays as asked");
        assert!(same(&look::ShaderFeatures::NONE.limit(&on), &o), "compiled out, it stays off (the default frame)");
    }
}
