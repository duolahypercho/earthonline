//! The daylight, ambient, sky and cloud work's kill switches, read from the
//! command line (eo-app passes them through): each restores the output
//! before its feature, and `--no-gg2-daylight` restores all of them at once.

use std::sync::OnceLock;

/// Which of the daylight work's features are on (all by default).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DaylightFeatures {
    /// The sky light's ground is the land cover's albedo round the eye
    /// (`phys::ground_albedo_at`), not the city's everywhere. `--no-ground-albedo`.
    pub ground_albedo: bool,
    /// A shot's `day_of_year` places the sun on the photograph's date. `--no-photo-date`.
    pub photo_date: bool,
    /// A clear day's sky is cloudless, cirrus or fair cumulus by the day's
    /// draw (eo-sky `clouds::sky_state`). `--no-clear-sky-state`.
    pub clear_sky_state: bool,
    /// A cirrus day's high deck is fibrous ice cloud at cirrus height. `--no-cirrus`.
    pub cirrus: bool,
}

impl DaylightFeatures {
    pub const ALL: Self = Self { ground_albedo: true, photo_date: true, clear_sky_state: true, cirrus: true };
    pub const NONE: Self = Self { ground_albedo: false, photo_date: false, clear_sky_state: false, cirrus: false };

    pub fn from_args(args: &[String]) -> Self {
        let has = |s: &str| args.iter().any(|a| a == s);
        if has("--no-gg2-daylight") {
            return Self::NONE;
        }
        Self {
            ground_albedo: !has("--no-ground-albedo"),
            photo_date: !has("--no-photo-date"),
            clear_sky_state: !has("--no-clear-sky-state"),
            cirrus: !has("--no-cirrus"),
        }
    }
}

static FEATURES: OnceLock<DaylightFeatures> = OnceLock::new();

/// The process's features (from `std::env::args` on first use).
pub fn daylight() -> DaylightFeatures {
    *FEATURES.get_or_init(|| DaylightFeatures::from_args(&std::env::args().collect::<Vec<_>>()))
}

/// Pin the features before anything reads them (a test, a tool); false when
/// the process already reads others.
pub fn pin_daylight(f: DaylightFeatures) -> bool {
    *FEATURES.get_or_init(|| f) == f
}

/// The switches `daylight` reads (eo-app accepts them).
pub const SWITCHES: &[&str] = &["--no-gg2-daylight", "--no-ground-albedo", "--no-photo-date", "--no-clear-sky-state", "--no-cirrus"];

/// The colour and camera track's switch names (the roll-up first); eo-app
/// accepts them. Reserved: nothing reads them yet.
pub const GG3_SWITCHES: &[&str] = &["--no-gg3-colour", "--no-sun-spectrum2", "--no-sky-calibration", "--no-camera-style", "--no-meter-highlights", "--no-haze-visibility"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_roll_up_switch_turns_every_feature_off() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(DaylightFeatures::from_args(&a(&["eo-app"])), DaylightFeatures::ALL);
        assert_eq!(DaylightFeatures::from_args(&a(&["eo-app", "--no-gg2-daylight"])), DaylightFeatures::NONE);
        let f = DaylightFeatures::from_args(&a(&["eo-app", "--no-cirrus"]));
        assert!(!f.cirrus && f.ground_albedo && f.clear_sky_state);
    }
}
