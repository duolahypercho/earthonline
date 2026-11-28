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
    /// The volumetric cloudscape's cumulus cover takes the day's sky state as the
    /// low deck does (eo-sky `clouds::volumetric_decks`): a cloudless or cirrus
    /// day leaves it as little cumulus as it leaves the low deck.
    /// `--no-vol-sky-state` (the weather's cover whatever the day's sky).
    pub vol_sky_state: bool,
}

impl DaylightFeatures {
    pub const ALL: Self = Self { ground_albedo: true, photo_date: true, clear_sky_state: true, cirrus: true, vol_sky_state: true };
    pub const NONE: Self = Self { ground_albedo: false, photo_date: false, clear_sky_state: false, cirrus: false, vol_sky_state: false };

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
            vol_sky_state: !has("--no-vol-sky-state"),
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
pub const SWITCHES: &[&str] = &["--no-gg2-daylight", "--no-ground-albedo", "--no-photo-date", "--no-clear-sky-state", "--no-cirrus", "--no-vol-sky-state"];

/// The colour and camera track's switch names (the roll-up first, then the
/// kill switches); eo-app accepts them. `--no-gg3` (every round-3 track's
/// roll-up) turns the colour track off too.
pub const GG3_SWITCHES: &[&str] = &["--no-gg3-colour", "--no-sun-spectrum2", "--no-sky-calibration", "--no-camera-style", "--no-meter-highlights", "--no-haze-visibility"];

/// Which of the colour and camera track's features are on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColourFeatures {
    /// The direct sun's colour from its spectrum through the air
    /// (`spectral::beam_srgb`), not the three channels. `--no-sun-spectrum2`.
    pub sun_spectrum2: bool,
    /// The scene meter drops its highlight hold (eo-sky
    /// `night::scene_meter_factor`): it puts the frame's mean luminance at
    /// the meter's grey, K = 12.5 (ISO 2720 gives K, the calibration
    /// constant; averaging the whole frame is the averaging meter type, a
    /// choice the standard does not prescribe). `--no-meter-highlights`
    /// keeps the hold. The centre weighting goes too only once T1's
    /// scene_meter.rs writes the meter uniform's `grey.y` from
    /// `night::meter_averaging()` (a hand-off); until then this switch
    /// changes the hold alone. Measured on the acceptance views: the hold
    /// never bound by day, so on and off match within the noise floor.
    pub meter_highlights: bool,
}

impl ColourFeatures {
    pub const ALL: Self = Self { sun_spectrum2: true, meter_highlights: true };
    pub const NONE: Self = Self { sun_spectrum2: false, meter_highlights: false };

    /// From the command line.
    pub fn from_args(args: &[String]) -> Self {
        let has = |s: &str| args.iter().any(|a| a == s);
        if has("--no-gg3-colour") || has("--no-gg3") {
            return Self::NONE;
        }
        Self { sun_spectrum2: !has("--no-sun-spectrum2"), meter_highlights: !has("--no-meter-highlights") }
    }
}

static COLOUR: OnceLock<ColourFeatures> = OnceLock::new();

/// The process's colour features (from `std::env::args` on first use).
pub fn colour() -> ColourFeatures {
    *COLOUR.get_or_init(|| ColourFeatures::from_args(&std::env::args().collect::<Vec<_>>()))
}

/// Pin the colour features before anything reads them (a test, a tool).
pub fn pin_colour(f: ColourFeatures) -> bool {
    *COLOUR.get_or_init(|| f) == f
}

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

    #[test]
    fn the_colour_roll_ups_turn_every_colour_feature_off() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(ColourFeatures::from_args(&a(&["eo-app"])), ColourFeatures::ALL);
        assert_eq!(ColourFeatures::from_args(&a(&["eo-app", "--no-gg3-colour"])), ColourFeatures::NONE);
        assert_eq!(ColourFeatures::from_args(&a(&["eo-app", "--no-gg3"])), ColourFeatures::NONE);
        let f = ColourFeatures::from_args(&a(&["eo-app", "--no-sun-spectrum2"]));
        assert!(!f.sun_spectrum2 && f.meter_highlights);
        // the registry holds kill switches only
        for s in GG3_SWITCHES {
            assert!(s.starts_with("--no-"), "{s}");
        }
    }
}
