//! (`--weather damp` and `--look cinematic`, the cinematic look plan's stage
//! L3) The wet street's shader constants are eo-sky `cine_wet`'s, the streak
//! lights' list fits where the shaders read it, the gather validates with
//! the streak pick compiled in and out, and the damp bit is the shader's.

use eo_sky::cine_wet::*;

#[test]
fn the_wet_streets_shader_constants_are_eo_skys() {
    let src = include_str!("../src/sky_light.wgsl");
    let has = |line: String| assert!(src.contains(&line), "sky_light.wgsl lacks `{line}`");
    has(format!("const SKY_DAMP_DRY_FLOOR: f32 = {DAMP_DRY_FLOOR:?};"));
    has(format!("const SKY_CINE_WET_DARKEN: vec2<f32> = vec2<f32>({:?}, {:?});", WET_DARKEN[0], WET_DARKEN[1]));
    has(format!("const SKY_CINE_WET_ROUGH: vec2<f32> = vec2<f32>({:?}, {:?});", WET_ROUGH[0], WET_ROUGH[1]));
    has(format!("const SKY_CINE_STREAK_F0: f32 = {STREAK_F0:?};"));
    has(format!("const SKY_CINE_STREAK_ALONG: f32 = {STREAK_ALPHA_ALONG:?};"));
    has(format!("const SKY_CINE_STREAK_ACROSS: f32 = {STREAK_ALPHA_ACROSS:?};"));
    has(format!("const SKY_CINE_PUDDLE_ALPHA: f32 = {PUDDLE_ALPHA:?};"));
    has(format!("const SKY_CINE_STREAK_LENS: f32 = {STREAK_LENS:?};"));
    has(format!("const SKY_CINE_STREAK_HEAD_LENS: f32 = {STREAK_HEAD_LENS:?};"));
    has(format!("const SKY_STREAK_CAP: u32 = {}u;", eo_render::sky::STREAK_CAP));
    has(format!("const SKY_CINE_SSR_ROUGH: vec2<f32> = vec2<f32>({:?}, {:?});", SSR_ROUGH[0], SSR_ROUGH[1]));
    has(format!("const SKY_CINE_SSR_STEPS: i32 = {SSR_STEPS};"));
    has(format!("const SKY_CINE_SSR_STEP: vec2<f32> = vec2<f32>({:?}, {:?});", SSR_STEP[0], SSR_STEP[1]));
    has(format!("const SKY_CINE_SSR_CAP: f32 = {SSR_CAP:?};"));
    let ag = include_str!("../src/agents_draw.wgsl");
    assert!(ag.contains(&format!("const AG_CINE_COAT_ROUGH: f32 = {COAT_ROUGH:?};")));
    assert!(ag.contains(&format!("const AG_CINE_COAT_GRIME_ROUGH: f32 = {COAT_GRIME_ROUGH:?};")));
    // the damp bit
    assert!(include_str!("../src/sky_common.wgsl").contains(&format!("const SKY_DAMP: u32 = {}u;", eo_render::sky::SKY_DAMP_BIT)));
    // every new term sits behind the look's compile-out const (and the street's variant)
    assert!(src.contains("const SKY_CINE_FULL: bool = SF_CINEMATIC && LK_SHADOW_CASCADES && LK_SHADOW_FAR;"));
}

#[test]
fn the_streak_pick_is_compiled_in_only_with_the_look_and_validates() {
    let on = eo_render::sky::cine_lamps_wgsl(true);
    let off = eo_render::sky::cine_lamps_wgsl(false);
    assert!(on.contains("fn lights_streaks") && on.contains(&format!("const LT_STREAK_CAP: u32 = {}u;", eo_render::sky::STREAK_CAP)));
    assert!(!off.contains("lights_streaks") && !off.contains("LT_STREAK_CAP"), "the default's gather is today's");
    for flag in [true, false] {
        let src = format!(
            "{}\n{}\nconst LT_TILE_M: f32 = 1.0;\n{}{}{}",
            include_str!("../src/sky_common.wgsl"),
            eo_render::sky::sea_wgsl(),
            eo_render::shop_lights::wgsl(),
            eo_render::sky::cine_lamps_wgsl(flag),
            include_str!("../src/sky_lights.wgsl")
        );
        let m = wgpu::naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
        wgpu::naga::valid::Validator::new(wgpu::naga::valid::ValidationFlags::all(), wgpu::naga::valid::Capabilities::all()).validate(&m).unwrap();
        assert_eq!(m.entry_points.iter().any(|e| e.name == "lights_streaks"), flag);
    }
}

#[test]
fn the_damp_preset_is_the_skys_weather_and_draws_no_rain() {
    use eo_render::sky::WeatherMode;
    let w = eo_render::sky::weather_of(&eo_render::sky::SkyOptions { weather: WeatherMode::Preset(eo_sky::Kind::Damp), ..Default::default() }, 0.0);
    assert_eq!(w.kind, eo_sky::Kind::Damp);
    assert_eq!(w.rain_mm_h, 0.0, "no drops, no rings (sky_fx.wgsl rain_vs and sky_light.wgsl sky_ripple read the rate)");
    assert!(w.wetness > 0.5 && w.puddle_fill > 0.3);
}
