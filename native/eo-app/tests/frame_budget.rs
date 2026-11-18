//! Frame-budget guard: runs the real `--bench` (offscreen, no window) on every
//! camera path with `--check-budget`, which exits non-zero when a standard
//! path's median frame (CPU streaming pump + encode + submit + GPU) reaches
//! 10 ms (street_walk, elevated_sweep, authored_street, kit_street), or when
//! any frame wrote more than the upload budget (the drive streams kit packs
//! in and out while it is measured; shortened here to 20 s), and (hitch2)
//! with `--check-max`, which exits non-zero when ANY path's worst single
//! frame reaches 10 ms (every path: the standard ones, the drive, the
//! waterfront and bay, and the sky's night, rain and fog paths).
//! Needs a GPU. Run in release for meaningful numbers:
//!   cargo test --release -p eo-app --test frame_budget

#[test]
fn standard_paths_stay_under_10ms_median() {
    if cfg!(debug_assertions) {
        eprintln!("frame_budget: debug build; the budget is only enforced in --release");
        return;
    }
    let out = std::env::temp_dir().join("eo-native-frame-budget");
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_eo-app"))
        .args(["--bench", "--check-budget", "--check-max", "--drive-frames", "1200", "--label", "budget-test", "--out"]) // + --check-max
        .arg(&out)
        .status()
        .expect("run eo-app --bench");
    assert!(status.success(), "eo-app --bench --check-budget --check-max failed (see its output): a path's median or worst frame reached the 10 ms budget, or the bench could not run");
}
