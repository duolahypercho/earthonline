//! With the cinematic look, the agents' draw module is compiled in Metal's
//! relaxed math mode: its label carries the mark the vendored wgpu-hal reads
//! (native/vendor/wgpu-hal/EARTH-ONLINE.md).

const HAL_METAL_MOD: &str = include_str!("../../vendor/wgpu-hal/src/metal/mod.rs");
const HAL_METAL_DEVICE: &str = include_str!("../../vendor/wgpu-hal/src/metal/device.rs");
const WORKSPACE: &str = include_str!("../../Cargo.toml");

fn hal_mark() -> &'static str {
    let line = HAL_METAL_MOD.lines().find(|l| l.starts_with("pub const RELAXED_MATH_LABEL_MARK: &str = ")).expect("the vendored wgpu-hal declares RELAXED_MATH_LABEL_MARK");
    let value = line.split('"').nth(1).expect("a string literal");
    assert!(!value.is_empty());
    value
}

#[test]
fn the_draw_module_label_carries_the_vendored_mark() {
    let mark = hal_mark();
    assert!(eo_render::agents::DRAW_MODULE_LABEL.contains(mark), "{} lacks {mark}", eo_render::agents::DRAW_MODULE_LABEL);
    assert!(!eo_render::agents::DRAW_MODULE_LABEL_FAST_MATH.contains(mark), "the label without the mark must compile as before");
}

#[test]
fn only_the_cinematic_look_takes_the_mark_and_the_switch_drops_it() {
    use eo_render::agents::{DRAW_MODULE_LABEL, DRAW_MODULE_LABEL_FAST_MATH, draw_module_label_for};
    assert_eq!(draw_module_label_for(true, true), DRAW_MODULE_LABEL);
    assert_eq!(draw_module_label_for(true, false), DRAW_MODULE_LABEL_FAST_MATH, "the default look compiles as before");
    assert_eq!(draw_module_label_for(false, true), DRAW_MODULE_LABEL_FAST_MATH, "--no-agents-relaxed-math");
}

#[test]
fn the_vendored_hal_compiles_the_mark_relaxed_and_the_workspace_uses_it() {
    assert!(HAL_METAL_DEVICE.contains("l.contains(super::RELAXED_MATH_LABEL_MARK)"));
    assert!(HAL_METAL_DEVICE.contains("options.setMathMode(objc2_metal::MTLMathMode::Relaxed);"));
    let patch = WORKSPACE.split("[patch.crates-io]").nth(1).expect("native/Cargo.toml patches crates.io");
    assert!(patch.lines().any(|l| l.trim() == "wgpu-hal = { path = \"vendor/wgpu-hal\" }"), "{patch}");
}
