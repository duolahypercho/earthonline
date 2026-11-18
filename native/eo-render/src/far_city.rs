//! The far layer and the built city's switches (read from the command line,
//! as `hlod::HlodOptions::from_env` reads its own): each one turns one of
//! this pass's changes off and restores what was drawn before it.
//!
//! - `--no-far-fallback`: on an adapter without multi-draw-indirect-count the
//!   far layer (the cells, the districts and the distant trees) stays
//!   unattached, as before; with the feature nothing changes either way.
//! - `--no-landmark-far-models`: the authored landmark records keep the
//!   kit's model load radius (eo-app models.rs).
//! - `--no-far-tree-density`: the park trees of the distant layer are
//!   sampled in 512 m blocks again (hlod_trees.rs).
//! - `--no-presidio-rule`: read by eo-kit itself (the district palette).
//! - `--no-far-paint`: the district and cell rungs' paint as before (hlod_bake.rs).
//! - `--no-gg2-far`: all of the above at once.

/// Which of this pass's far-layer changes are on (all, unless switched off).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FarCity {
    pub fallback: bool,
    pub landmark_models: bool,
    pub tree_density: bool,
    pub presidio_rule: bool,
    pub far_paint: bool,
}

impl Default for FarCity {
    fn default() -> Self {
        Self { fallback: true, landmark_models: true, tree_density: true, presidio_rule: true, far_paint: true }
    }
}

/// Every switch this module reads (the roll-up first).
pub const SWITCHES: [&str; 6] = ["--no-gg2-far", "--no-far-fallback", "--no-landmark-far-models", "--no-far-tree-density", "--no-presidio-rule", "--no-far-paint"];

impl FarCity {
    /// From a command line (`args` as `std::env::args` gives them).
    pub fn from_args<S: AsRef<str>>(args: &[S]) -> Self {
        let has = |k: &str| args.iter().any(|a| a.as_ref() == k);
        if has("--no-gg2-far") {
            return Self { fallback: false, landmark_models: false, tree_density: false, presidio_rule: false, far_paint: false };
        }
        Self {
            fallback: !has("--no-far-fallback"),
            landmark_models: !has("--no-landmark-far-models"),
            tree_density: !has("--no-far-tree-density"),
            presidio_rule: !has("--no-presidio-rule"),
            far_paint: !has("--no-far-paint"),
        }
    }

    /// From this process's command line.
    pub fn from_env() -> Self {
        Self::from_args(&std::env::args().collect::<Vec<_>>())
    }

    /// `from_env`, read once (for per-frame callers).
    pub fn current() -> Self {
        static ONCE: std::sync::OnceLock<FarCity> = std::sync::OnceLock::new();
        *ONCE.get_or_init(Self::from_env)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everything_is_on_by_default_and_the_roll_up_turns_all_off() {
        assert_eq!(FarCity::from_args::<&str>(&[]), FarCity::default());
        let off = FarCity::from_args(&["eo-app", "--no-gg2-far"]);
        assert!(!off.fallback && !off.landmark_models && !off.tree_density && !off.presidio_rule && !off.far_paint);
        let one = FarCity::from_args(&["eo-app", "--no-far-paint"]);
        assert!(one.fallback && one.landmark_models && one.tree_density && one.presidio_rule && !one.far_paint);
    }
}
