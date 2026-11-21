//! Stage W2c (`--grime`): the far layer's per-building tone (hlod_draw.wgsl
//! `hl_age`) and the street-life and weathering code composed into the far
//! layer's shader parse and validate (naga; no GPU). The pipelines themselves
//! are built by the app (`eo-app` occlusion check).

use eo_render::hlod::{self, HlodOptions};

fn validate(name: &str, src: &str) {
    let module = wgpu::naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(src)));
    wgpu::naga::valid::Validator::new(wgpu::naga::valid::ValidationFlags::all(), wgpu::naga::valid::Capabilities::all())
        .validate(&module)
        .unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(src)));
}

#[test]
fn the_far_layers_shader_with_its_grime_tone_validates() {
    let o = HlodOptions::from_env();
    let src = format!("{}\n{}{}", eo_render::look::city_shader_source_far(), hlod::far_constants_wgsl(&o), hlod::DRAW_WGSL);
    // the tone is reached only behind the weathering level (default frames unchanged)
    assert!(hlod::DRAW_WGSL.contains("if (wx_level() != 0u && !roofish) {"), "hl_age behind wx_level()");
    validate("hlod_draw", &src);
}
