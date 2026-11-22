//! The season: how dry (golden) the annual grass is on a day of the year,
//! the one curve every land layer reads (the far layer's grass through the
//! look uniform's per-frame `season`, the city terrain's far land through
//! `eo_render::farshore::season_dryness`; plants and terrain colour read
//! `dryness` / `current` too), and the photo-date sky track's switches.
//!
//! The curve is the phenology of coastal California annual grassland as
//! satellites measure it. Bart, Tague and Dennison (2017), "Modeling annual
//! grassland phenology along the central coast of California", Ecosphere
//! 8(7): e01875, MODIS MOD13Q1 NDVI 2000-2014 over two coastal grassland sites
//! (Morro Bay and Santa Barbara), green-up and senescence picked with
//! TIMESAT at 20 % and 80 % of each year's NDVI amplitude:
//!
//! * green-up initiation (the first substantial rains) spreads from the start
//!   of October to early January, mean 6-11 December (water-year day 67-72);
//! * NDVI rises through the growing season and peaks shortly before
//!   senescence;
//! * senescence initiation spreads from the start of April to the end of May,
//!   mean 1-2 May (water-year day 213-214), and lasts 46-51 days, ending on
//!   17-21 June on average.
//!
//! Closer to the Golden Gate, the Jasper Ridge annual grassland (San Mateo
//! County) peaked in NDVI between late February and late May across plots and
//! years, its season starting with the first fall rains (Cleland, Chiariello,
//! Loarie, Mooney and Field 2006, "Diverse responses of phenology to global
//! changes in a grassland ecosystem", PNAS 103(37): 13740-13744): the same
//! window. No published NDVI phenology of the Marin headlands themselves was
//! found; the central-coast dates are used as they stand.
//!
//! `dryness` maps that onto 0 green .. 1 golden, for the mean year (the
//! spread between years, green-up starting from October in the earliest, about
//! 65 days before the mean, is not modelled): golden from the mean end of
//! senescence until the green-up ramp; down to green over the green-up
//! (INFERRED: from 20 days before the mean initiation, so that on the mean
//! initiation date the curve has come about a fifth of the way to green, as
//! the TIMESAT 20 % of amplitude point that defines the date says, to 45 days
//! after it, when last year's thatch is under new growth); up to golden over
//! the senescence (from 15 days before the mean initiation, so that on that
//! date, the 80 % point, the curve has gone about a seventh of the way to
//! golden, to the mean end).

use std::sync::OnceLock;

/// The season's switch names (the photo-date season track), the roll-up
/// first; eo-app accepts them and `features` reads them.
pub const SWITCHES: &[&str] = &["--no-gg3-sky", "--no-photo-season", "--no-cirrus2", "--no-sky-day-fade", "--no-live-calendar"];

/// Which of the photo-date sky track's features are on (all by default).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SkyDateFeatures {
    /// The season follows the date: the phenology curve (`dryness`) and the far
    /// layer reading the per-frame season, so a shot's date sets it.
    /// `--no-photo-season`: the round-2 curve, read once at load.
    pub photo_season: bool,
    /// The cirrus drawn in its own deck as soft sheets and fibres with the
    /// METAR cloud amount (eo-sky `clouds::Decks::cirrus`, sky_volume.wgsl
    /// `vol_cirrus2`). `--no-cirrus2`: round 2's one high deck and streaks.
    pub cirrus2: bool,
    /// A clear day's sky changes to the next game day's over the world
    /// clock's night (eo-sky `clouds::sky_mix`). `--no-sky-day-fade`: it flips.
    pub sky_day_fade: bool,
    /// The date advances with the game day (`sun::calendar_day`).
    /// `--no-live-calendar`: always 22 September.
    pub live_calendar: bool,
}

impl SkyDateFeatures {
    pub const ALL: Self = Self { photo_season: true, cirrus2: true, sky_day_fade: true, live_calendar: true };
    pub const NONE: Self = Self { photo_season: false, cirrus2: false, sky_day_fade: false, live_calendar: false };

    pub fn from_args(args: &[String]) -> Self {
        let has = |s: &str| args.iter().any(|a| a == s);
        if has("--no-gg3-sky") || has("--no-gg3") {
            return Self::NONE;
        }
        Self {
            photo_season: !has("--no-photo-season"),
            cirrus2: !has("--no-cirrus2"),
            sky_day_fade: !has("--no-sky-day-fade"),
            live_calendar: !has("--no-live-calendar"),
        }
    }
}

static FEATURES: OnceLock<SkyDateFeatures> = OnceLock::new();

/// The process's features (from `std::env::args` on first use).
pub fn features() -> SkyDateFeatures {
    *FEATURES.get_or_init(|| SkyDateFeatures::from_args(&std::env::args().collect::<Vec<_>>()))
}

/// Pin the features before anything reads them (a test, a tool); false when
/// the process already reads others.
pub fn pin_features(f: SkyDateFeatures) -> bool {
    *FEATURES.get_or_init(|| f) == f
}

/// Mean green-up initiation, day of a common year: 8 December (Bart et al.
/// 2017: water-year days 67 and 72, 6 and 11 December).
pub const GREEN_UP_DAY: f64 = 342.0;
/// Mean senescence initiation: 1 May (water-year days 213-214).
pub const SENESCENCE_DAY: f64 = 121.0;
/// Mean end of senescence: 19 June (17 and 21 June at the two sites).
pub const SENESCENCE_END_DAY: f64 = 170.0;
/// The green-up ramp about `GREEN_UP_DAY`: from 20 days before (so the mean
/// initiation date, the 20 % of NDVI amplitude point, falls about a fifth of
/// the way down the ramp) to 45 after (INFERRED, see the module).
pub const GREEN_UP_LEAD_D: f64 = 20.0;
pub const GREEN_UP_TAIL_D: f64 = 45.0;
/// The senescence ramp starts this long before its initiation (INFERRED).
pub const SENESCENCE_LEAD_D: f64 = 15.0;

fn smooth(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How golden the annual grass is on a day of the year (1 = 1 January, a
/// common year, fractional days allowed, wrapping by the year): 0 green ..
/// 1 golden. The phenology curve of the module; `--no-photo-season` gives
/// round 2's curve (`dryness_r2`).
pub fn dryness(day_of_year: f64) -> f32 {
    if features().photo_season { dryness_phenology(day_of_year) } else { dryness_r2(day_of_year) }
}

/// The phenology curve (the module's sources).
pub fn dryness_phenology(day_of_year: f64) -> f32 {
    let d = day_of_year.rem_euclid(365.0);
    // senescence: green to golden
    let up = smooth((d - (SENESCENCE_DAY - SENESCENCE_LEAD_D)) / (SENESCENCE_END_DAY - SENESCENCE_DAY + SENESCENCE_LEAD_D));
    // green-up: golden to green, across the turn of the year
    let g0 = GREEN_UP_DAY - GREEN_UP_LEAD_D;
    let since = (d - g0).rem_euclid(365.0); // days since the green-up ramp began
    let span = GREEN_UP_LEAD_D + GREEN_UP_TAIL_D;
    // the green-up ramp runs `span` days from g0; outside it the senescence side holds
    if since < span {
        (1.0 - smooth(since / span)) as f32
    } else {
        up as f32
    }
}

/// Round 2's curve: green from the first winter rains to spring (Dec-Mar),
/// browning through April-May, golden from June until the rains.
pub fn dryness_r2(day_of_year: f64) -> f32 {
    let d = day_of_year.rem_euclid(365.0);
    // up from day 95 (Apr 5) to 165 (Jun 14); down from 305 (Nov 1) to 345 (Dec 11)
    (smooth((d - 95.0) / 70.0) * (1.0 - smooth((d - 305.0) / 40.0))) as f32
}

/// The dryness on the sun model's day (`sun::day_of_year`).
pub fn current() -> f32 {
    dryness(crate::sun::day_of_year())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn green_in_winter_golden_in_autumn() {
        for f in [dryness_phenology, dryness_r2] {
            assert!(f(265.0) > 0.99); // the game's date (22 Sep)
            assert!(f(45.0) < 0.01); // mid-February
            assert!(f(130.0) > 0.2 && f(130.0) < 0.8); // May: browning
            assert!(f(103.0) < 0.05); // 13 Apr (the headland photograph): still green
            assert!((f(400.0) - f(35.0)).abs() < 1e-6); // wraps by the year
        }
        assert_eq!(current(), dryness(crate::sun::day_of_year()));
    }

    #[test]
    fn the_phenology_follows_the_cited_dates() {
        let f = dryness_phenology;
        // golden from the mean end of senescence to the green-up ramp
        for d in [SENESCENCE_END_DAY, 200.0, 265.0, 297.0, GREEN_UP_DAY - GREEN_UP_LEAD_D] {
            assert!(f(d) > 0.999, "day {d}: {}", f(d));
        }
        // green from the end of the green-up ramp to the senescence ramp
        for d in [GREEN_UP_DAY + GREEN_UP_TAIL_D - 365.0, 30.0, 60.0, 90.0, 104.0, SENESCENCE_DAY - SENESCENCE_LEAD_D] {
            assert!(f(d) < 0.001, "day {d}: {}", f(d));
        }
        // at the mean green-up initiation (20 % of the NDVI amplitude) it is still mostly golden;
        // at the mean senescence initiation (80 %) still mostly green
        assert!(f(GREEN_UP_DAY) > 0.7 && f(GREEN_UP_DAY) < 0.95, "{}", f(GREEN_UP_DAY));
        assert!(f(SENESCENCE_DAY) > 0.05 && f(SENESCENCE_DAY) < 0.25, "{}", f(SENESCENCE_DAY));
        // continuous over the whole year, the turn of the year included
        let mut prev = f(0.0);
        for k in 1..=3650 {
            let v = f(k as f64 * 0.1);
            assert!((v - prev).abs() < 0.01, "jump at day {}: {prev} -> {v}", k as f64 * 0.1);
            prev = v;
        }
        assert!((f(365.0) - f(0.0)).abs() < 1e-6);
    }

    #[test]
    fn the_roll_up_switch_turns_every_feature_off() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(SkyDateFeatures::from_args(&a(&["eo-app"])), SkyDateFeatures::ALL);
        assert_eq!(SkyDateFeatures::from_args(&a(&["eo-app", "--no-gg3-sky"])), SkyDateFeatures::NONE);
        assert_eq!(SkyDateFeatures::from_args(&a(&["eo-app", "--no-gg3"])), SkyDateFeatures::NONE);
        let f = SkyDateFeatures::from_args(&a(&["eo-app", "--no-cirrus2"]));
        assert!(!f.cirrus2 && f.photo_season && f.sky_day_fade && f.live_calendar);
        for s in SWITCHES {
            assert!(s.starts_with("--no-"));
        }
    }
}
