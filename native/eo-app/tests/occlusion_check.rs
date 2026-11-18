//! the ground-truth guard. Runs the real `--bench --occlusion-check`
//! (offscreen, no window): on six cameras of every bench path and three
//! visibility histories each, every object owning a pixel with occlusion OFF
//! must be in the occlusion-ON drawn lists, and the ON frame must equal the
//! OFF frame (HDR bit-equal where the pixel's object is the same, output
//! within the dither; a pixel whose object differs only at a depth tie).
//! Needs a GPU and the city data. Run in release:
//!   cargo test --release -p eo-app --test occlusion_check

#[test]
fn occlusion_never_culls_a_visible_object_on_any_bench_path() {
    if cfg!(debug_assertions) {
        eprintln!("occlusion_check: debug build; run with --release");
        return;
    }
    let out = std::env::temp_dir().join("eo-native-occlusion-check");
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_eo-app"))
        .args(["--bench", "--occlusion-check", "--label", "occlusion-test", "--out"])
        .arg(&out)
        .status()
        .expect("run eo-app --bench --occlusion-check");
    assert!(status.success(), "eo-app --bench --occlusion-check failed (see its output): an object with a visible pixel was culled, or the images differ");
}
