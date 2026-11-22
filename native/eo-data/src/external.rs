//! Where the optional open assets are read from.
//!
//! Each set (the street people, their motions and walking styles, the
//! scanned CC0 surfaces, the sounds, the sign art, the far-shore elevation)
//! lives in `<repo>/external/<set>/`, a git-ignored folder that
//! `scripts/setup-assets.py` fills from the set's manifest. The set's
//! environment variable, when set and not empty, names another folder
//! instead. The repository is found as [`crate::DataPaths::discover`] finds
//! it (`EO_REPO_ROOT`, or the walk up to `public/data/sf/sf-city.json.gz`);
//! no path here is absolute.
//!
//! This only says where to look. Whether the folder holds a copy that
//! matches its manifest is each reader's own check, which refuses a folder
//! that does not and says why in one log line.
//!
//! `--no-external` ([`set_default_enabled`]) turns the default folder off:
//! only the variables are read, as before the folder existed.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// The folder under the repository root that holds the sets.
pub const DIR: &str = "external";

/// One optional set: the variable that names its folder, and its folder under `external/`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Set {
    pub env: &'static str,
    pub dir: &'static str,
}

/// Rocketbox avatars (`native/assets/people-manifest.json`).
pub const PEOPLE: Set = Set { env: "EO_PEOPLE_ROOT", dir: "people" };
/// Rocketbox motion clips (`native/assets/rocketbox-motions-manifest.json`).
pub const PEOPLE_MOTIONS: Set = Set { env: "EO_PEOPLE_MOTIONS_ROOT", dir: "people-motions" };
/// 100STYLE walking styles (`native/assets/100style-manifest.json`).
pub const PEOPLE_STYLES: Set = Set { env: "EO_PEOPLE_STYLES_ROOT", dir: "people-styles" };
/// Scanned CC0 surface sets (`native/assets/cc0-manifest.json`).
pub const CC0: Set = Set { env: "EO_CC0_ROOT", dir: "cc0" };
/// Sound clips (`native/assets/audio-manifest.json`).
pub const AUDIO: Set = Set { env: "EO_AUDIO_ROOT", dir: "audio" };
/// Official sign art and the sign font (`native/eo-props/data/sign-art-manifest.json`).
pub const SIGN_ART: Set = Set { env: "EO_SIGN_ART_ROOT", dir: "sign-art" };
/// Far-shore elevation tiles (`native/assets/dem-manifest.json`).
pub const DEM: Set = Set { env: "EO_DEM_ROOT", dir: "dem" };

/// Every set, in the order `scripts/setup-assets.py` fills them.
pub const ALL: [Set; 7] = [PEOPLE, PEOPLE_MOTIONS, PEOPLE_STYLES, CC0, AUDIO, SIGN_ART, DEM];

static DEFAULT_ON: AtomicBool = AtomicBool::new(true);

/// `false` (`--no-external`): a set is read only from the folder its variable names.
pub fn set_default_enabled(on: bool) {
    DEFAULT_ON.store(on, Ordering::Relaxed);
}

pub fn default_enabled() -> bool {
    DEFAULT_ON.load(Ordering::Relaxed)
}

impl Set {
    /// The set's folder this run: the variable's folder when it is set and
    /// not empty, else `<repo>/external/<dir>`; None when the variable is
    /// unset and the repository is not found (or `--no-external`).
    pub fn root(&self) -> Option<PathBuf> {
        let var = std::env::var_os(self.env);
        let repo = if default_enabled() && var.as_ref().is_none_or(|v| v.is_empty()) { crate::DataPaths::discover().ok().map(|p| p.repo_root) } else { None };
        self.resolve(var, repo.as_deref())
    }

    /// [`Set::root`] from given inputs: the variable's value and the repository root.
    pub fn resolve(&self, var: Option<OsString>, repo_root: Option<&Path>) -> Option<PathBuf> {
        match var.filter(|v| !v.is_empty()) {
            Some(v) => Some(PathBuf::from(v)),
            None => repo_root.map(|r| r.join(DIR).join(self.dir)),
        }
    }

    /// Whether this run's folder comes from the variable (else the default folder).
    pub fn from_env(&self) -> bool {
        std::env::var_os(self.env).is_some_and(|v| !v.is_empty())
    }

    /// For a log line: where the set was looked for, and how to fill it.
    pub fn looked_in(&self) -> String {
        match self.root() {
            Some(p) if self.from_env() => format!("{} (from {})", p.display(), self.env),
            Some(p) => format!("{} (fill it with `python3 scripts/setup-assets.py --only {}`, or set {} to another copy)", p.display(), self.dir, self.env),
            None if !default_enabled() => format!("nowhere ({} is not set and --no-external is on)", self.env),
            None => format!("nowhere ({} is not set and the repository was not found)", self.env),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_variable_wins_over_the_default_folder() {
        let repo = Path::new("repo");
        assert_eq!(CC0.resolve(Some("elsewhere/cc0-copy".into()), Some(repo)), Some(PathBuf::from("elsewhere/cc0-copy")));
        assert_eq!(CC0.resolve(Some("elsewhere".into()), None), Some(PathBuf::from("elsewhere")));
    }

    #[test]
    fn unset_or_empty_means_the_external_folder() {
        let repo = Path::new("repo");
        for set in ALL {
            let want = repo.join("external").join(set.dir);
            assert_eq!(set.resolve(None, Some(repo)), Some(want.clone()), "{}", set.env);
            assert_eq!(set.resolve(Some(OsString::new()), Some(repo)), Some(want), "{} empty", set.env);
        }
    }

    #[test]
    fn no_variable_and_no_repository_is_nowhere() {
        for set in ALL {
            assert_eq!(set.resolve(None, None), None, "{}", set.env);
        }
    }

    #[test]
    fn sets_have_distinct_variables_and_folders() {
        for (i, a) in ALL.iter().enumerate() {
            assert!(a.env.starts_with("EO_") && a.env.ends_with("_ROOT"), "{}", a.env);
            assert!(!a.dir.is_empty() && a.dir.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'), "{}", a.dir);
            for b in &ALL[i + 1..] {
                assert_ne!(a.env, b.env);
                assert_ne!(a.dir, b.dir);
            }
        }
    }

    /// In this process (no variable set for a set): the folder under the
    /// repository this checkout is. A set whose variable is set is checked
    /// against the variable instead.
    #[test]
    fn this_checkout_resolves_to_its_own_external_folder() {
        let repo = crate::DataPaths::discover().expect("the repository").repo_root;
        for set in ALL {
            let got = set.root();
            match std::env::var_os(set.env).filter(|v| !v.is_empty()) {
                Some(v) => assert_eq!(got, Some(PathBuf::from(v)), "{}", set.env),
                None => {
                    assert_eq!(got, Some(repo.join("external").join(set.dir)), "{}", set.env);
                    assert!(!set.from_env());
                    assert!(set.looked_in().contains("setup-assets.py"), "{}", set.looked_in());
                }
            }
        }
    }

    /// The setup script fills the same folders the game reads, and names the same variables.
    #[test]
    fn the_setup_script_fills_these_folders() {
        let repo = crate::DataPaths::discover().expect("the repository").repo_root;
        let script = std::fs::read_to_string(repo.join("scripts/setup-assets.py")).expect("scripts/setup-assets.py");
        for set in ALL {
            assert!(script.contains(&format!("('{}', '{}', ", set.dir, set.env)), "scripts/setup-assets.py has no SETS row for {} ({})", set.dir, set.env);
        }
        assert!(script.contains("REPO / 'external'"), "the script's default root is not <repo>/external");
    }
}
