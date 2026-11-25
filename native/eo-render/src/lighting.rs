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
            // the far layer's draw as hlod.rs composes it (`--far-facades` lives in it).
            ("far layer draw", format!("{}\n{}{}", look::city_shader_source_far(), crate::hlod::far_constants_wgsl(&crate::hlod::HlodOptions::from_env()), crate::hlod::draw_wgsl())),
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
        sets.push(("facade glass".into(), look::ShaderFeatures { facade_glass: true, ..none })); // (`--facade-glass`)
        sets.push(("facade glass with grime and street life".into(), look::ShaderFeatures { facade_glass: true, grime: true, street_life: true, shop_light: true, ..none }));
        sets.push(("far facades".into(), look::ShaderFeatures { far_facades: true, ..none })); // (`--far-facades`)
        sets.push(("facade weather".into(), look::ShaderFeatures { facade_weather: true, ..none })); // (`--facade-weather`)
        sets.push(("facade weather with grime, glass and clutter".into(), look::ShaderFeatures { facade_weather: true, facade_glass: true, grime: true, street_life: true, facade_clutter: true, ..none }));
        sets.push(("canyon glass".into(), look::ShaderFeatures { canyon_glass: true, ..none })); // (`--canyon-glass`)
        sets.push(("canyon glass with the pane and every lighting part".into(), look::ShaderFeatures { canyon_glass: true, facade_glass: true, lx_bits: u32::MAX, cinematic: true, ..none }));
        sets.push(("far relief".into(), look::ShaderFeatures { far_relief: true, ..none })); // (`--far-relief`)
        sets.push(("far relief with the far facades and every lighting part".into(), look::ShaderFeatures { far_relief: true, far_facades: true, lx_bits: u32::MAX, cinematic: true, ..none }));
        sets.push(("facade patina".into(), look::ShaderFeatures { facade_patina: true, ..none })); // (`--facade-patina`)
        sets.push(("facade patina with the weather, glass, grime and every lighting part".into(), look::ShaderFeatures { facade_patina: true, facade_weather: true, facade_glass: true, grime: true, street_life: true, lx_bits: u32::MAX, cinematic: true, ..none }));
        sets.push(("window depth".into(), look::ShaderFeatures { window_depth: true, ..none })); // (`--window-depth`)
        sets.push(("window depth with the glass, canyon, far relief and every lighting part".into(), look::ShaderFeatures { window_depth: true, facade_glass: true, canyon_glass: true, far_relief: true, far_facades: true, soft_shadows: true, lx_bits: u32::MAX, cinematic: true, ..none }));
        sets.push(("street pbr".into(), look::ShaderFeatures { grime: true, street_pbr: true, ..none })); // (`--street-pbr`)
        sets.push(("street pbr alone".into(), look::ShaderFeatures { street_pbr: true, ..none }));
        // (`--street-wear`, `--street-glare`) the street track's round-2 passes, alone, with the street pass and the cinematic look's coat
        sets.push(("street wear and glare".into(), look::ShaderFeatures { street: look::STREET_WEAR | look::STREET_GLARE, ..none }));
        sets.push(("street wear and glare over street pbr, cinematic".into(), look::ShaderFeatures { grime: true, street_pbr: true, street: look::STREET_WEAR | look::STREET_GLARE, cinematic: true, lx_bits: u32::MAX, ..none }));
        sets.push(("street aggregate and plaza setts".into(), look::ShaderFeatures { street: look::STREET_AGGREGATE | look::PLAZA_SETTS, ..none }));
        sets.push(("street aggregate, setts, wear and glare over street pbr, cinematic".into(), look::ShaderFeatures { grime: true, street_pbr: true, street: look::STREET_WEAR | look::STREET_GLARE | look::STREET_AGGREGATE | look::PLAZA_SETTS, cinematic: true, lx_bits: u32::MAX, ..none }));
        // (`--walk-relief`) alone, and over the street pass with every street bit and the cinematic look
        sets.push(("walk relief".into(), look::ShaderFeatures { street: look::WALK_RELIEF, ..none }));
        sets.push(("walk relief with every street bit over street pbr, cinematic".into(), look::ShaderFeatures { grime: true, street_pbr: true, street: look::STREET_WEAR | look::STREET_GLARE | look::STREET_AGGREGATE | look::PLAZA_SETTS | look::WALK_RELIEF, cinematic: true, lx_bits: u32::MAX, ..none }));
        sets.push(("sun bounce".into(), look::ShaderFeatures { sun_bounce: true, ..none })); // (`--sun-bounce`)
        sets.push(("ao albedo".into(), look::ShaderFeatures { ao_albedo: true, ..none })); // (`--ao-albedo`)
        sets.push(("soft shadows".into(), look::ShaderFeatures { soft_shadows: true, ..none })); // (`--soft-shadows`)
        sets.push(("screen gi".into(), look::ShaderFeatures { screen_gi: true, ..none })); // (`--screen-gi`)
        sets.push(("screen gi + sun bounce".into(), look::ShaderFeatures { screen_gi: true, sun_bounce: true, ..none }));
        sets.push(("occluder light".into(), look::ShaderFeatures { occluder_light: true, ..none })); // (`--occluder-light`)
        sets.push(("occluder light + sun bounce + every lighting part".into(), look::ShaderFeatures { occluder_light: true, sun_bounce: true, screen_gi: true, lx_bits: u32::MAX, car_materials: true, cinematic: true, ..none }));
        sets.push(("wet surfaces".into(), look::ShaderFeatures { wet_surfaces: true, ..none })); // (`--wet-surfaces`)
        sets.push(("wet surfaces, cinematic".into(), look::ShaderFeatures { wet_surfaces: true, cinematic: true, ..none }));
        sets.push(("screen reflect".into(), look::ShaderFeatures { screen_reflect: true, ..none })); // (`--screen-reflect`)
        sets.push(("screen reflect, wet surfaces, cinematic".into(), look::ShaderFeatures { screen_reflect: true, wet_surfaces: true, cinematic: true, ..none }));
        sets.push(("probe sky".into(), look::ShaderFeatures { probe_sky: true, ..none })); // (`--probe-sky`)
        sets.push(("wet relief".into(), look::ShaderFeatures { wet_relief: true, ..none })); // (`--wet-relief`)
        sets.push(("wet relief, wet surfaces, screen reflect, street pbr, cinematic".into(), look::ShaderFeatures { wet_relief: true, wet_surfaces: true, screen_reflect: true, street_pbr: true, cinematic: true, ..none }));
        sets.push(("lamp streaks".into(), look::ShaderFeatures { lamp_streaks: true, ..none })); // (`--lamp-streaks`)
        sets.push(("lamp streaks, wet relief, wet surfaces, screen reflect, street pbr, cinematic".into(), look::ShaderFeatures { lamp_streaks: true, wet_relief: true, wet_surfaces: true, screen_reflect: true, street_pbr: true, cinematic: true, ..none }));
        sets.push(("probe sky, cinematic".into(), look::ShaderFeatures { probe_sky: true, cinematic: true, lx_bits: u32::MAX, ..none }));
        sets.push(("car materials".into(), look::ShaderFeatures { car_materials: true, ..none })); // (`--car-materials`)
        sets.push(("car materials, cinematic, grime".into(), look::ShaderFeatures { car_materials: true, cinematic: true, grime: true, ..none }));
        sets.push(("car reflections, cinematic".into(), look::ShaderFeatures { car_materials: true, car_reflections: true, cinematic: true, ..none })); // (`--car-reflections`)
        sets.push(("car lamps and wet, cinematic".into(), look::ShaderFeatures { car_materials: true, car_reflections: true, car_lamps: true, car_wet: true, cinematic: true, ..none })); // (`--car-lamps`, `--car-wet`)
        sets.push(("car finish, cinematic, grime".into(), look::ShaderFeatures { car_materials: true, car_reflections: true, car_lamps: true, car_wet: true, car_finish: true, cinematic: true, grime: true, ..none })); // (`--car-finish`)
        sets.push(("car finish alone".into(), look::ShaderFeatures { car_materials: true, car_finish: true, ..none }));
        sets.push(("night photometry".into(), look::ShaderFeatures { night_photometry: true, ..none })); // (`--night-photometry`)
        sets.push(("lamp bounce".into(), look::ShaderFeatures { lamp_bounce: true, ..none })); // (`--lamp-bounce`)
        sets.push(("urban sky".into(), look::ShaderFeatures { urban_sky: true, night_photometry: true, ..none })); // (`--urban-sky`)
        sets.push(("office floors".into(), look::ShaderFeatures { office_floors: true, ..none })); // (`--office-floors`)
        sets.push(("room depth".into(), look::ShaderFeatures { room_depth: true, ..none })); // (`--room-depth`)
        sets.push(("lamp units".into(), look::ShaderFeatures { lamp_units: true, ..none })); // (`--lamp-units`)
        sets.push(("luminaires".into(), look::ShaderFeatures { luminaires: true, ..none })); // (`--luminaires`)
        sets.push(("lamp units, luminaires, cinematic, night photometry".into(), look::ShaderFeatures { lamp_units: true, luminaires: true, cinematic: true, night_photometry: true, ..none }));
        sets.push(("rear lamps, cinematic".into(), look::ShaderFeatures { rear_lamps: true, cinematic: true, ..none })); // (`--rear-lamps`)
        sets.push(("signal spill, cinematic".into(), look::ShaderFeatures { signal_spill: true, cinematic: true, ..none })); // (`--signal-spill`)
        sets.push(("lit rooms, room depth".into(), look::ShaderFeatures { lit_rooms: true, room_depth: true, ..none })); // (`--lit-rooms`)
        sets.push(("rear lamps, signal spill, night photometry".into(), look::ShaderFeatures { rear_lamps: true, signal_spill: true, night_photometry: true, ..none })); // (`--rear-lamps`, `--signal-spill`: their lobes)
        sets.push(("wire light".into(), look::ShaderFeatures { wire_light: true, ..none })); // (`--wire-light`)
        for bit in [
            look::NATURE_FOLIAGE,
            look::NATURE_LOD_FADE,
            look::NATURE_TURF,
            look::NATURE_PALM,
            look::NATURE_HILLS,
            look::NATURE_COVER,
            look::NATURE_BARE,
            look::NATURE_PALM_VARIETY,
            look::NATURE_FOLIAGE | look::NATURE_PALM,
            look::NATURE_TURF | look::NATURE_COVER,
            look::NATURE_TURF | look::NATURE_BARE,
            look::NATURE_COVER | look::NATURE_BARE,
            look::NATURE_PALM | look::NATURE_PALM_VARIETY,
            look::NATURE_FOLIAGE | look::NATURE_PALM | look::NATURE_PALM_VARIETY,
            look::NATURE_CROWN,
            look::NATURE_FAR_PALMS,
            look::NATURE_PROP_FAR,
            look::NATURE_CROWN | look::NATURE_FAR_PALMS,
            look::NATURE_FOLIAGE | look::NATURE_CROWN,
            look::NATURE_FOLIAGE | look::NATURE_FAR_PALMS,
            look::NATURE_LOD_FADE | look::NATURE_CROWN,
            look::NATURE_LOD_FADE | look::NATURE_FAR_PALMS,
            look::NATURE_FOLIAGE | look::NATURE_PROP_FAR,
            look::NATURE_LEAF_SHADE,
            look::NATURE_WET_FOLIAGE,
            look::NATURE_FOLIAGE | look::NATURE_LEAF_SHADE,
            look::NATURE_FOLIAGE | look::NATURE_WET_FOLIAGE,
            look::NATURE_CROWN | look::NATURE_LEAF_SHADE,
            look::NATURE_FOLIAGE | look::NATURE_CROWN | look::NATURE_LEAF_SHADE | look::NATURE_WET_FOLIAGE,
            look::NATURE_LAWN_ALBEDO,
            look::NATURE_TURF | look::NATURE_LAWN_ALBEDO,
            look::NATURE_ALL,
        ] {
            sets.push((format!("nature {bit}"), look::ShaderFeatures { nature: bit, ..none })); // (`--foliage-light`, `--temporal-lod-fade`, `--turf`, `--palm-detail`, `--hill-cover`, `--ground-cover`, `--bare-ground`, `--palm-variety`, `--crown-light`, `--far-palms`, `--prop-far-shadow`)
        }
        sets.push(("all + nature".into(), look::ShaderFeatures { nature: look::NATURE_ALL, ..look::ShaderFeatures::ALL }));
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
                let props = format!("{}\n{}\n{}", look::props_city_source(look::city_shader_source_variant_probes(crate::ambient::ShaderLevels::Plane, look::ShadowLevels::Near, false)), include_str!("props_types.wgsl"), look::sf_select(include_str!("props_draw.wgsl")));
                parse(&format!("{name}: props"), &props);
                // the natural-detail blocks (`// sf:`): without the bits the props' shader holds
                // the old back-light and none of the new code; with foliage, the new code alone.
                let foliage = props.contains("fn pr_foliage(");
                assert_eq!(foliage, f.nature & look::NATURE_FOLIAGE != 0, "{name}: the foliage block follows its bit");
                assert_eq!(props.contains("// back-lit leaves."), !foliage, "{name}: the old back-light is there without the foliage only");
                assert!(!props.contains("// sf:"), "{name}: no marker is left");
                let palm = props.contains("fn pr_palm(");
                assert_eq!(palm, f.nature & look::NATURE_PALM != 0, "{name}: the palm block follows its bit");
                if foliage || palm {
                    assert!(props.contains(&format!("const PR_PALM: u32 = {}u;", eo_props::Model::Palm as u32)), "PR_PALM is eo_props::Model::Palm");
                }
                assert_eq!(props.contains("var palm_leaf"), foliage || palm, "{name}: the shared block is in with either bit");
                // (`--palm-variety`) with `--palm-detail` only: the joined blocks (`// sf:A&B`)
                let variety = f.nature & look::NATURE_PALM_VARIETY != 0 && palm;
                assert_eq!(props.contains("fn pr_palm_variety("), variety, "{name}: the variety block follows both bits");
                assert_eq!(props.contains("let pv = pr_palm_variety(inst);"), variety, "{name}: the trunk's hook follows both bits");
                assert_eq!(props.contains("let pvc = pr_palm_variety(inst);"), variety, "{name}: the crown's hook follows both bits");
                // (`--ground-cover`) the terrain's mosaic and its hook follow the bit
                let city = look::city_shader_source_variant_probes(crate::ambient::ShaderLevels::Plane, look::ShadowLevels::Near, false);
                assert_eq!(city.contains("fn nt_cover("), f.nature & look::NATURE_COVER != 0, "{name}: the cover block follows its bit");
                assert_eq!(city.contains("nt_cover(&s, nc_w"), f.nature & look::NATURE_COVER != 0, "{name}: the cover hook follows its bit");
                // (`--bare-ground`) the bare ground and the revetments, their hooks, and the ground scans skipped under them
                let bare = f.nature & look::NATURE_BARE != 0;
                assert_eq!(city.contains("fn nt_bare("), bare, "{name}: the bare ground's block follows its bit");
                assert_eq!(city.contains("nt_bare(&s, nb_w"), bare, "{name}: the bare ground's hook follows its bit");
                assert_eq!(city.contains("nb_w.x < 0.999"), bare, "{name}: the scans' skip follows its bit");
                assert_eq!(city.matches("cc0_ground(&s, in.world, n, dpx, dpy);").count(), 1, "{name}: one ground-scan call either way");
                assert_eq!(city.contains("nt_w = nt_w * (1.0 - nb_w.x);"), bare && f.nature & look::NATURE_TURF != 0, "{name}: the turf's share joins both bits");
                assert_eq!(city.contains("nc_w.x = nc_w.x * (1.0 - nb_w.x);"), bare && f.nature & look::NATURE_COVER != 0, "{name}: the cover's share joins both bits");
                assert!(!city.contains("// sf:"), "{name}: no marker is left in the city shader");
                let far_trees = format!("{}\nconst FT_VIEWS: u32 = {}u;\nconst FT_NORMALS: i32 = {};\n{}", look::city_shader_source_far(), crate::cove_canopy::VIEWS, crate::cove_canopy::LAYERS, look::sf_select(crate::hlod_trees::WGSL));
                parse(&format!("{name}: far trees"), &far_trees);
                // (`--crown-light`, `--far-palms`) the far trees' new shading and its hooks follow the bits
                let crown = f.nature & look::NATURE_CROWN != 0;
                let far_palms = f.nature & look::NATURE_FAR_PALMS != 0;
                assert_eq!(far_trees.contains("fn ft_crown("), crown, "{name}: the crown's volume follows its bit");
                assert_eq!(far_trees.contains("fn ft_palm("), far_palms, "{name}: the far palm follows its bit");
                assert_eq!(far_trees.contains("return ft_shade2(in);"), crown || far_palms, "{name}: the new shading is the entry's with either bit");
                assert_eq!(far_trees.contains("key = key * lk_leaf_sun;"), crown, "{name}: look_light's hook follows the crown's bit");
                assert_eq!(props.contains("pr_crown_sky = exp(-PR_CROWN_KL * (1.0 - f));"), crown, "{name}: the street trees' crown light follows the crown's bit");
                assert_eq!(props.contains("ao = ao * pr_crown_sky;"), crown && foliage, "{name}: the light through the leaves takes it with both bits");
                assert_eq!(far_trees.contains("hlod_dither(floor(in.clip.xy) + shift)"), crown || far_palms, "{name}");
                assert!(!far_trees.contains("// sf:"), "{name}: no marker is left in the far trees");
                if far_palms {
                    assert!(far_trees.contains(&format!("const FT_PALM: u32 = {}u;", crate::hlod_trees::FAR_PALM)), "FT_PALM is hlod_trees::FAR_PALM");
                }
                // (`--prop-far-shadow`) the props' vertex lookup and look_light's reading of it
                let prop_far = f.nature & look::NATURE_PROP_FAR != 0;
                assert_eq!(props.contains("out.far_sun = pr_far_sun(world);"), prop_far, "{name}: the props' far lookup follows its bit");
                assert_eq!(props.contains("lk_far_vertex = in.far_sun;"), prop_far, "{name}");
                assert_eq!(props.contains("return mix(1.0, lk_far_vertex, look.key_dir.w);"), prop_far, "{name}: lk_shadow reads it with the bit only");
                // (`--leaf-shade`) the crown's transmittance in the lights' loop, set by the props; the
                // leaves' transmittance over reflectance by band, in the props and the far crowns
                let leaf_shade = f.nature & look::NATURE_LEAF_SHADE != 0;
                assert_eq!(city.contains("fn sky_leaf_t("), leaf_shade, "{name}: the crown's transmittance follows its bit");
                assert_eq!(city.contains("e = e * sky_leaf_t(world, li.pos);"), leaf_shade, "{name}: the lights' loop takes it with the bit only");
                assert_eq!(props.contains("const SKY_LEAF_ON: bool = true;"), leaf_shade, "{name}: compiled into the props' loop with the bit");
                assert!(!city.contains("const SKY_LEAF_ON: bool = true;"), "{name}: and into no other pipeline's");
                assert_eq!(props.contains("sky_leaf_k = vec2<f32>(PR_LS_PALM_K.x, PR_LS_PALM_K.y * sc);"), leaf_shade, "{name}: the palms set their crown with the bit only");
                assert_eq!(props.contains("@location(9) sway: f32,"), leaf_shade || crown, "{name}: the wind weight reaches the fragment with either bit");
                if leaf_shade {
                    assert!(props.contains(&format!("const PR_LS_PALM: u32 = {}u;", eo_props::Model::Palm as u32)), "PR_LS_PALM is eo_props::Model::Palm");
                    assert!(props.contains(&format!("const PR_LS_TREE: u32 = {}u;", eo_props::Model::Tree as u32)), "PR_LS_TREE is eo_props::Model::Tree");
                }
                if foliage {
                    let by_band = props.contains("const PR_LEAF_T_OVER_R: vec3<f32> = vec3<f32>(0.8, 0.95, 0.35);");
                    assert_eq!(by_band, leaf_shade, "{name}: the leaves' ratio by band with the bit only");
                    assert_eq!(props.matches("const PR_LEAF_T_OVER_R").count(), 1, "{name}: one ratio");
                    assert_eq!(far_trees.contains("let t = albedo * vec3<f32>(0.8, 0.95, 0.35);"), leaf_shade, "{name}: the far crowns' too");
                }
                if crown {
                    assert_eq!(far_trees.matches("const FT_LEAF_T_OVER_R").count(), 1, "{name}: one far ratio");
                }
                // (`--wet-foliage`) with `--foliage-light`: the leaves' and the bark's water
                let wet_foliage = f.nature & look::NATURE_WET_FOLIAGE != 0 && foliage;
                assert_eq!(props.contains("fn pr_wet_leaf("), wet_foliage, "{name}: the wet leaves follow both bits");
                assert_eq!(props.contains("pr_wet_bark(&s, skyu.wet.x, run);"), wet_foliage, "{name}: the wet bark follows both bits");
                assert_eq!(props.contains("s.albedo = s.albedo * mix(1.0, 0.85, wet);"), foliage && !wet_foliage, "{name}: the old wet leaf without it");
                if wet_foliage {
                    assert!(props.contains(&format!("const PR_WET_PALM: u32 = {}u;", eo_props::Model::Palm as u32)), "PR_WET_PALM is eo_props::Model::Palm");
                    assert!(props.contains(&format!("const PR_WET_TREE: u32 = {}u;", eo_props::Model::Tree as u32)), "PR_WET_TREE is eo_props::Model::Tree");
                }
                // (`--lawn-albedo`) with `--turf`: the lawn's tone held to a measured turf's
                let lawn_albedo = f.nature & look::NATURE_LAWN_ALBEDO != 0 && f.nature & look::NATURE_TURF != 0;
                assert_eq!(city.contains("alb = alb * mix(1.0, nt_lawn_k, w.x);"), lawn_albedo, "{name}: the lawn's scale follows both bits");
                assert_eq!(city.contains("nt_lawn_k = clamp(l, NT_TURF_LUM.x, NT_TURF_LUM.y)"), lawn_albedo, "{name}: set from the tone with both bits");
                let stages = if prop_far { wgpu::ShaderStages::VERTEX_FRAGMENT } else { wgpu::ShaderStages::FRAGMENT };
                assert_eq!(look::far_map_stages(), stages, "{name}: the far map reaches the vertex stage with the bit only");
                let agents = format!(
                    "{}\n{}\n{}\n{}\n{}\n{}\n{}",
                    look::city_shader_source_variant_probes(crate::ambient::ShaderLevels::Full, look::ShadowLevels::Full, false),
                    crate::agents::people_shading_wgsl(), // (`--people-shading`)
                    crate::agents::car_wgsl(), // (`--car-panels`, `--car-lamps`)
                    include_str!("agents_types.wgsl"),
                    include_str!("agents_common.wgsl"),
                    include_str!("agents_draw.wgsl"),
                    include_str!("sky_velocity.wgsl")
                );
                parse(&format!("{name}: agents"), &agents);
                // (`--sun-bounce`) the occlusion pass, with its gather or the stub.
                let ao = look::ao_shader_source();
                assert_eq!(ao.contains("var ao_far_map"), f.sun_bounce, "{name}: the gather's binding only with the feature");
                parse(&format!("{name}: occlusion pass"), &ao);
                // (`--screen-gi`) the gather's entry and its history only with the feature; the filter's own module.
                assert_eq!(ao.contains("fn ao_gi_main"), f.screen_gi, "{name}: the gather only with the feature");
                assert_eq!(ao.contains("var gi_hist"), f.screen_gi, "{name}");
                if f.screen_gi {
                    parse(&format!("{name}: occlusion filter"), &look::ao_gi_filter_source());
                }
                // (`--probe-sky`) the probe grid's sky and levels: their module is made only with the feature.
                if f.probe_sky {
                    parse(&format!("{name}: probe sky"), &crate::probe_grid::sky_source());
                }
                // (`--soft-shadows`) the penumbra in the full shadow variant only (props keep the tent).
                assert!(look::shadow_wgsl(look::ShadowLevels::Full).contains("const LK_SHADOW_SOFT: bool = true;"));
                assert!(look::shadow_wgsl(look::ShadowLevels::Near).contains("const LK_SHADOW_SOFT: bool = false;"));
                assert!(common.contains(&format!("const SF_SOFT_SHADOWS: bool = {};", f.soft_shadows)), "{name}");
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
            sky: crate::sky::SkyOptions { shop_light: 2, night_photometry: true, lamp_bounce: true, urban_sky: true, office_floors: true, room_depth: true, lamp_units: true, luminaires: true, rear_lamps: true, signal_spill: true, lit_rooms: true, wire_light: true, ..o.sky },
            ..o
        };
        let f = look::ShaderFeatures::of(&on, true, true, true);
        // (the look profile is its own, below; the facades track's switches are read from the
        // command line, `facade::glass_from_args` and `facade::far_from_args`, never from the look's options;
        // the sun bounce, the albedo's multi-bounce, the penumbra and the screen-space indirect light,
        // the reflect track's switches, the camera model and the cars' materials are their own, below)
        assert_eq!(f, look::ShaderFeatures { lx_bits: 0xFF, cinematic: false, facade_glass: false, far_facades: false, facade_weather: false, canyon_glass: false, sun_bounce: false, ao_albedo: false, soft_shadows: false, screen_gi: false, occluder_light: false, wet_surfaces: false, screen_reflect: false, probe_sky: false, wet_relief: false, lamp_streaks: false, camera: false, car_materials: false, car_reflections: false, ..look::ShaderFeatures::ALL });
        assert!(!f.street_pbr && !look::ShaderFeatures::ALL.street_pbr, "the street pass comes in with its switch only (eo-app)");
        assert!(f.street == 0 && look::ShaderFeatures::ALL.street == 0, "the street track's round-2 passes come in with their switches only (eo-app)");
        let same = |a: &look::LookOptions, b: &look::LookOptions| format!("{a:?}") == format!("{b:?}"); // (LookOptions has no PartialEq)
        assert!(same(&f.limit(&on), &on), "what is compiled in is left as asked");
        assert!(same(&look::ShaderFeatures::ALL.limit(&on), &on));
        let none = look::ShaderFeatures::NONE.limit(&on);
        assert_eq!(none.lighting, LightingOptions::OFF, "a feature compiled out stays off");
        assert_eq!((none.grime, none.street_life, none.sky.shop_light), (0, 0, 0));
        assert!(!none.sky.night_photometry, "(`--night-photometry`) compiled out, off");
        assert!(!none.sky.lamp_bounce && !none.sky.urban_sky, "(`--lamp-bounce`, `--urban-sky`) compiled out, off");
        assert!(!none.sky.office_floors && !none.sky.room_depth, "(`--office-floors`, `--room-depth`) compiled out, off");
        assert!(!none.sky.lamp_units && !none.sky.luminaires && !none.sky.rear_lamps && !none.sky.signal_spill && !none.sky.lit_rooms, "(`--lamp-units`, `--luminaires`, `--rear-lamps`, `--signal-spill`, `--lit-rooms`) compiled out, off");
        assert!(!none.sky.wire_light, "(`--wire-light`) compiled out, off");
        assert!(same(&none, &look::LookOptions { lighting: LightingOptions::OFF, grime: 0, street_life: 0, sky: o.sky, ..on }), "nothing else changes");
        // each lighting bit keeps only its own option.
        let blend_only = look::ShaderFeatures { lx_bits: look::LX_BIT_CASCADE_BLEND, ..look::ShaderFeatures::NONE }.limit(&on);
        assert_eq!(blend_only.lighting, LightingOptions { cascade_blend: true, ..LightingOptions::OFF });
        // (`--sun-bounce`) compiled in with its option, left off when compiled out.
        let sb = look::LookOptions { sun_bounce: true, ..on };
        assert!(look::ShaderFeatures::of(&sb, false, false, false).sun_bounce);
        assert!(!look::ShaderFeatures::NONE.limit(&sb).sun_bounce);
        assert!(look::ShaderFeatures::ALL.limit(&sb).sun_bounce);
        assert!(look::ShaderFeatures::NONE.wgsl().contains("const SF_SUN_BOUNCE: bool = false;"));
        // (`--ao-albedo`) the same.
        let aa = look::LookOptions { ao_albedo: true, ..on };
        assert!(look::ShaderFeatures::of(&aa, false, false, false).ao_albedo);
        assert!(!look::ShaderFeatures::NONE.limit(&aa).ao_albedo);
        assert!(look::ShaderFeatures::NONE.wgsl().contains("const SF_AO_ALBEDO: bool = false;"));
        // (`--soft-shadows`) the same.
        let ss = look::LookOptions { soft_shadows: true, ..on };
        assert!(look::ShaderFeatures::of(&ss, false, false, false).soft_shadows);
        assert!(!look::ShaderFeatures::NONE.limit(&ss).soft_shadows);
        assert!(look::ShaderFeatures::ALL.limit(&ss).soft_shadows);
        assert!(look::ShaderFeatures::NONE.wgsl().contains("const SF_SOFT_SHADOWS: bool = false;"));
        assert!(look::ShaderFeatures::ALL.wgsl().contains(&format!("const PS_TAN_DIAMETER: f32 = {:.8};", eo_look::penumbra::tan_diameter())), "the penumbra's constants reach the shader");
        assert!(look::ShaderFeatures::ALL.wgsl().contains(&format!("const PS_MIN_CASTER_M: f32 = {:.8};", eo_look::penumbra::MIN_CASTER_M)));
        assert!(look::ShaderFeatures::ALL.wgsl().contains(&format!("const PS_SUBPIXEL_PX: f32 = {:.8};", eo_look::penumbra::SUBPIXEL_PX)), "the penumbra's sub-pixel limit reaches the shader");
        // (`--screen-gi`) the same.
        let gi = look::LookOptions { screen_gi: true, ..on };
        assert!(look::ShaderFeatures::of(&gi, false, false, false).screen_gi);
        assert!(!look::ShaderFeatures::NONE.limit(&gi).screen_gi);
        assert!(look::ShaderFeatures::ALL.limit(&gi).screen_gi);
        assert!(look::ShaderFeatures::NONE.wgsl().contains("const SF_SCREEN_GI: bool = false;"));
        assert!(look::ao_gi_filter_source().contains(&format!("const GI_FILTER_PLANE_M: f32 = {:.8};", eo_look::screen_gi::FILTER_PLANE_M)), "the gather's constants reach the filter");
        // (`--occluder-light`) the same.
        let ol = look::LookOptions { occluder_light: true, ..on };
        assert!(look::ShaderFeatures::of(&ol, false, false, false).occluder_light);
        assert!(!look::ShaderFeatures::NONE.limit(&ol).occluder_light);
        assert!(look::ShaderFeatures::ALL.limit(&ol).occluder_light);
        assert!(look::ShaderFeatures::NONE.wgsl().contains("const SF_OCCLUDER_LIGHT: bool = false;"));
        assert!(look::ShaderFeatures::ALL.wgsl().contains("const SF_OCCLUDER_LIGHT: bool = true;"));
        let wa = eo_look::occluder::WALL_ALBEDO;
        assert!(look::ShaderFeatures::ALL.wgsl().contains(&format!("const OL_WALL_ALBEDO: vec3<f32> = vec3<f32>({:.8}, {:.8}, {:.8});", wa[0], wa[1], wa[2])), "the walls' albedo reaches the shader");
        // the decks' undersides: their albedo and the lid's reading.
        let w = look::ShaderFeatures::ALL.wgsl();
        assert!(w.contains(&format!("const OL_DECK_ALBEDO: f32 = {:.8};", eo_look::occluder::DECK_ALBEDO)), "the decks' albedo reaches the shader");
        assert!(w.contains(&format!("const OL_LID_R0: f32 = {:.8};", eo_look::occluder::LID_R0)) && w.contains(&format!("const OL_LID_R1: f32 = {:.8};", eo_look::occluder::LID_R1)), "the lid's reading reaches the shader");
        // (`--soft-shadows`) the penumbra's discs are eo-look's Vogel taps.
        let d = eo_look::penumbra::vogel_disc(eo_look::penumbra::SEARCH_TAPS);
        assert!(look::ShaderFeatures::ALL.wgsl().contains(&format!("const PS_SEARCH_DISC: array<vec2<f32>, {}> = array<vec2<f32>, {}>(vec2<f32>({:.8}, {:.8}),", d.len(), d.len(), d[0][0], d[0][1])));
    }

    #[test]
    fn the_reflect_tracks_switches_are_compiled_in_only_when_asked_and_limited_with_them() {
        // (`--wet-surfaces`, `--screen-reflect`, `--probe-sky`) the default compiles them out; each switch compiles in its own.
        let o = look::LookOptions::default();
        let f = look::ShaderFeatures::of(&o, false, false, false);
        assert!(!f.wet_surfaces && !f.screen_reflect && !f.probe_sky && !f.wet_relief && !f.lamp_streaks);
        for (k, v) in [("SF_WET_SURFACES", false), ("SF_SCREEN_REFLECT", false), ("SF_PROBE_SKY", false), ("SF_WET_RELIEF", false), ("SF_LAMP_STREAKS", false)] {
            assert!(look::ShaderFeatures::NONE.wgsl().contains(&format!("const {k}: bool = {v};")), "{k}");
            assert!(look::ShaderFeatures::ALL.wgsl().contains(&format!("const {k}: bool = {};", !v)), "{k}");
        }
        let same = |a: &look::LookOptions, b: &look::LookOptions| format!("{a:?}") == format!("{b:?}");
        for (wet, sr, ps, wr, ls) in [(true, false, false, false, false), (false, true, false, false, false), (false, false, true, false, false), (true, false, false, true, false), (false, false, false, false, true), (true, true, true, true, true)] {
            let on = look::LookOptions { sky: crate::sky::SkyOptions { wet_surfaces: wet, screen_reflect: sr, probe_sky: ps, wet_relief: wr, lamp_streaks: ls, ..o.sky }, ..o };
            let f = look::ShaderFeatures::of(&on, false, false, false);
            assert_eq!(f, look::ShaderFeatures { wet_surfaces: wet, screen_reflect: sr, probe_sky: ps, wet_relief: wr, lamp_streaks: ls, ..look::ShaderFeatures::NONE }, "each alone compiles in nothing else");
            assert!(same(&f.limit(&on), &on), "compiled in, it stays as asked");
            assert!(same(&look::ShaderFeatures::NONE.limit(&on), &o), "compiled out, it stays off (the default frame)");
        }
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
