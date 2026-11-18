//! Cloud decks: the coverage sheet the GPU marches and casts cloud shadows
//! from, and each weather's deck heights, coverage, density and drift.
//!
//! The sheet follows sky-atmosphere.js `bakeCloudSheet` /
//! environment-ibl.js `renderCloudSheet`: tileable value-noise fBm (5
//! octaves, the silhouette gain 0.56, the first octave one below the
//! lattice) whose threshold is a quantile, so that a coverage c covers
//! exactly c of the sheet. Here every channel is histogram-equalised at
//! bake time (stored as its rank), so the shader's threshold is simply
//! `1 - c`. Channels: R the low deck (lattice 8 over 3,200 m, as the JS low
//! deck's 256 px sheet), G the high deck (lattice 6 over 10,000 m), B the
//! marine layer's bank (lattice 4), A detail (lattice 16, 3 octaves).
//!
//! Heights: the JS draws one deck at 940 m (280 m slab) in every weather.
//! This port keeps 940 m for fair-weather cumulus and lowers the deck in fog
//! and rain to the San Francisco marine stratus: the Bay Area's marine layer
//! is typically 1,000-2,000 ft (300-600 m) deep (NWS San Francisco Bay Area
//! forecast office, marine layer climatology) and on fog days its top sits
//! at or under the city's hills; the fog bucket's deck is that layer, 60-300
//! m, and the drizzle/rain decks sit between it and the cumulus base.

use crate::hash::{hash01, lattice01};
use crate::weather::{Kind, WeatherState};
use std::sync::atomic::{AtomicU8, Ordering};

pub const SHEET_RES: usize = 256;
/// World metres one sheet tile spans (sky-atmosphere.js tile sizes).
pub const LOW_TILE_M: f64 = 3200.0;
pub const HIGH_TILE_M: f64 = 10000.0;
/// The marine bank's own tile (a coarser field over the same sheet).
pub const MARINE_TILE_M: f64 = 9600.0;
const SILHOUETTE_GAIN: f64 = 0.56;

/// periodic gradient (Perlin) noise with the quintic fade. The JS sheet
/// is value noise (a random value per lattice corner, smoothstep between):
/// cut at a sharp threshold, its contours run along the lattice axes, and a
/// cumulus field drawn with a defined edge came out as stacked rectangles.
/// A gradient per corner moves the extrema off the lattice points and the
/// contours turn; the sheet is histogram-equalised after, so the coverage law
/// is unchanged. (`lattice01` gives the angle.)
fn tile_noise(x: f64, y: f64, period: i32, seed: u32) -> f64 {
    let (xi, yi) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - xi as f64, y - yi as f64);
    let s = |v: f64| v * v * v * (v * (v * 6.0 - 15.0) + 10.0);
    let (ux, uy) = (s(fx), s(fy));
    let g = |i: i32, j: i32, dx: f64, dy: f64| {
        let a = lattice01(i.rem_euclid(period), j.rem_euclid(period), 0, seed) * std::f64::consts::TAU;
        a.cos() * dx + a.sin() * dy
    };
    let a = g(xi, yi, fx, fy);
    let b = g(xi + 1, yi, fx - 1.0, fy);
    let c = g(xi, yi + 1, fx, fy - 1.0);
    let d = g(xi + 1, yi + 1, fx - 1.0, fy - 1.0);
    let v = (a + (b - a) * ux) + ((c + (d - c) * ux) - (a + (b - a) * ux)) * uy;
    0.5 + 0.7 * v
}

/// Tileable fBm at `u, v` in 0..1 over a lattice of `lattice` cells.
pub fn fbm(u: f64, v: f64, lattice: i32, octaves: u32, seed: u32) -> f64 {
    let mut sum = 0.0;
    let mut amp = 1.0;
    let mut norm = 0.0;
    // One octave below the lattice first (`renderCloudSheet`).
    let mut period = (lattice / 2).max(1);
    for o in 0..octaves {
        sum += amp * tile_noise(u * period as f64, v * period as f64, period, seed.wrapping_add(o * 1013));
        norm += amp;
        amp *= SILHOUETTE_GAIN;
        period *= 2;
    }
    sum / norm
}

fn equalise(v: &[f64]) -> Vec<u8> {
    let mut idx: Vec<usize> = (0..v.len()).collect();
    idx.sort_by(|a, b| v[*a].partial_cmp(&v[*b]).unwrap());
    let mut out = vec![0u8; v.len()];
    let n = v.len() as f64;
    for (rank, i) in idx.into_iter().enumerate() {
        out[i] = (((rank as f64 + 0.5) / n) * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    out
}

/// The baked sheet, RGBA8, `SHEET_RES` square, rows top first.
pub fn bake_sheet(seed: u32) -> Vec<[u8; 4]> {
    let n = SHEET_RES;
    let chan = |lattice: i32, oct: u32, salt: u32| {
        let v: Vec<f64> = (0..n * n)
            .map(|k| {
                let (x, y) = (k % n, k / n);
                fbm((x as f64 + 0.5) / n as f64, (y as f64 + 0.5) / n as f64, lattice, oct, seed ^ salt)
            })
            .collect();
        equalise(&v)
    };
    let (r, g, b, a) = (chan(8, 5, 0x51a7), chan(6, 5, 0x2be3), chan(4, 4, 0x7c11), chan(16, 3, 0x3d29));
    (0..n * n).map(|k| [r[k], g[k], b[k], a[k]]).collect()
}

/// Box-filtered mip chain of the sheet (level 0 first), wrapping.
pub fn sheet_mips(level0: &[[u8; 4]]) -> Vec<Vec<[u8; 4]>> {
    let mut out = vec![level0.to_vec()];
    let mut res = SHEET_RES;
    while res > 1 {
        let prev = out.last().unwrap();
        let h = res / 2;
        let mut next = vec![[0u8; 4]; h * h];
        for y in 0..h {
            for x in 0..h {
                let mut acc = [0u32; 4];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let p = prev[(2 * y + dy) * res + 2 * x + dx];
                    for c in 0..4 {
                        acc[c] += p[c] as u32;
                    }
                }
                next[y * h + x] = acc.map(|v| ((v + 2) / 4) as u8);
            }
        }
        out.push(next);
        res = h;
    }
    out
}

/// The decks for one weather.
#[derive(Clone, Copy, Debug)]
pub struct Decks {
    pub low_base_m: f64,
    pub low_top_m: f64,
    /// Fraction of the sky the low deck covers, and the opacity of a covered pixel.
    pub coverage: f64,
    pub low_opacity: f64,
    /// Extinction of the low deck's densest part, 1/m.
    pub extinction: f64,
    pub high_alt_m: f64,
    pub high_opacity: f64,
    /// Drift of the low and high decks, m/s (x east, z south).
    pub drift_low: [f64; 2],
    pub drift_high: [f64; 2],
    /// Direct-beam transmission where the deck is fully covered (the JS
    /// profile's `cloudBeamTransmission`): the floor cloud shadows reach.
    pub beam: f64,
    /// Marine layer: extinction inside the bank (1/m), its mean top and the
    /// top's swing (m), its drift (m/s).
    pub marine_sigma: f64,
    pub marine_top_m: f64,
    pub marine_swing_m: f64,
    pub marine_drift: [f64; 2],
}

/// Low-deck base and top per rung (clear, fog, drizzle, rain, storm) and
/// for the dry overcast preset.
/// In fog the deck IS the marine layer: stratus from 60 m to 300 m, so the
/// hills above it (Twin Peaks 280 m, Mount Sutro 254 m) stand at its top.
/// the clear rung's cumulus slab is 900-2100 m (the JS's 940-1220 m,
/// 280 m deep, made every cumulus a flat pancake, a cut-out seen from the
/// side): a fair-weather cumulus over the Bay Area has its flat base at the
/// lifting condensation level, ~800-1,200 m, and is about as tall as it is
/// wide, 300-1,500 m (humilis to mediocris; the core decides how high it
/// grows, sky_volume `vol_cloud_at`).
const HEIGHTS: [[f64; 2]; 5] = [[900.0, 2100.0], [60.0, 300.0], [450.0, 1150.0], [400.0, 1500.0], [350.0, 2200.0]];
const OVERCAST_HEIGHTS: [f64; 2] = [600.0, 1000.0];
/// Cumulus/stratus extinction: liquid water 0.2-0.5 g/m^3 at a 10 um
/// effective radius gives 0.03-0.08 1/m (sigma = 3 LWC / (2 rho r_e));
/// 0.045 for the cumulus cores, 0.025 for stratus sheets.
const CUMULUS_SIGMA: f64 = 0.045;
const STRATUS_SIGMA: f64 = 0.025;
/// The onshore drift: San Francisco's prevailing summer wind is from the
/// west-northwest (290 deg); low cloud moves with the 850 hPa wind, 5-10 m/s.
const DRIFT_LOW_MS: f64 = 7.0;
/// the share of the JS clear deck's cover kept on a clear day.
pub const FAIR_WEATHER_COVER: f64 = 0.4;
const DRIFT_HIGH_MS: f64 = 18.0;

/// A clear day's sky: the clear rung (the weather's overcast under 0.3)
/// draws one of three skies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkyState {
    /// no cloud at all.
    Cloudless,
    /// high ice cloud only: fibrous cirrus at `CIRRUS_ALT_M`.
    Cirrus,
    /// the fair-weather sky this port drew on every clear day: the high
    /// sheet at 5.2 km over the (opt-in) cumulus field.
    FairCumulus,
}

impl SkyState {
    pub fn name(self) -> &'static str {
        match self {
            SkyState::Cloudless => "cloudless",
            SkyState::Cirrus => "cirrus",
            SkyState::FairCumulus => "fair_cumulus",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "cloudless" => SkyState::Cloudless,
            "cirrus" => SkyState::Cirrus,
            "fair_cumulus" | "cumulus" => SkyState::FairCumulus,
            _ => return None,
        })
    }
    fn code(s: Option<Self>) -> u8 {
        match s {
            None => 0,
            Some(SkyState::Cloudless) => 1,
            Some(SkyState::Cirrus) => 2,
            Some(SkyState::FairCumulus) => 3,
        }
    }
}

/// The share of a month's clear days at San Francisco airport (KSFO) that
/// are cloudless, cirrus-only and fair (scattered low cloud), January first.
/// From the routine hourly METARs of 2014-2023 (NOAA/NWS ASOS observations,
/// public domain, as archived by the Iowa Environmental Mesonet,
/// mesonet.agron.iastate.edu/request/download.phtml), the reports of 10:00 to
/// 16:59 local time: a day with broken or overcast cloud under 18,000 ft in
/// over half of them is not a clear day and is left out; of the rest, a day
/// with scattered or more cloud under 18,000 ft in two or more reports is
/// fair, else one with any layer at or over 18,000 ft (cirrus height) in two
/// or more is cirrus, else cloudless (INFERRED: the 18,000 ft line and the
/// two-report thresholds). 182-286 days a month.
pub const KSFO_CLEAR_DAY_SHARES: [[f64; 3]; 12] = [
    [0.14, 0.53, 0.32],
    [0.15, 0.60, 0.25],
    [0.16, 0.51, 0.33],
    [0.16, 0.61, 0.22],
    [0.24, 0.46, 0.30],
    [0.38, 0.38, 0.24],
    [0.47, 0.33, 0.20],
    [0.43, 0.31, 0.26],
    [0.40, 0.41, 0.19],
    [0.30, 0.50, 0.20],
    [0.13, 0.62, 0.24],
    [0.09, 0.58, 0.33],
];
const MONTH_DAYS: [f64; 12] = [31.0, 28.0, 31.0, 30.0, 31.0, 30.0, 31.0, 31.0, 30.0, 31.0, 30.0, 31.0];

/// The month (0 = January) of a day of a common year (1 = 1 January).
pub fn month_of(day_of_year: f64) -> usize {
    let mut d = day_of_year.floor().clamp(1.0, 365.0);
    for (m, n) in MONTH_DAYS.iter().enumerate() {
        if d <= *n {
            return m;
        }
        d -= n;
    }
    11
}

/// The clear rung's sky on a day of the year, drawn from the month's shares
/// (`KSFO_CLEAR_DAY_SHARES`) with a hash of the day: the same day draws the
/// same sky.
pub fn drawn_state(day_of_year: f64) -> SkyState {
    let u = hash01(0x5c1e_a7e5, day_of_year.floor().max(0.0) as u32, 17);
    let [cloudless, cirrus, fair] = KSFO_CLEAR_DAY_SHARES[month_of(day_of_year)];
    let u = u * (cloudless + cirrus + fair);
    if u < cloudless {
        SkyState::Cloudless
    } else if u < cloudless + cirrus {
        SkyState::Cirrus
    } else {
        SkyState::FairCumulus
    }
}

static STATE: AtomicU8 = AtomicU8::new(0);

/// Hold the clear rung's sky (a shot's `sky`, the observation of its day),
/// or None to draw it from the day (`drawn_state`).
pub fn set_sky_state(s: Option<SkyState>) {
    STATE.store(SkyState::code(s), Ordering::Relaxed);
}

/// The held sky, if any.
pub fn held_sky_state() -> Option<SkyState> {
    match STATE.load(Ordering::Relaxed) {
        1 => Some(SkyState::Cloudless),
        2 => Some(SkyState::Cirrus),
        3 => Some(SkyState::FairCumulus),
        _ => None,
    }
}

/// The clear rung's sky in force (`--no-clear-sky-state`: always the fair-weather sky).
pub fn sky_state() -> SkyState {
    if !eo_look::features::daylight().clear_sky_state {
        return SkyState::FairCumulus;
    }
    held_sky_state().unwrap_or_else(|| drawn_state(eo_look::sun::day_of_year()))
}

/// The cirrus deck's height: 9.5 km. The WMO International Cloud Atlas
/// (2017, "Definitions: approximate height ranges of the etages") puts the
/// high etage, where cirrus forms, at 5-13 km in temperate regions; under a
/// mid-latitude tropopause at 10-12 km it sits mostly at 8-11 km, whose
/// middle this is (INFERRED from the etage and the tropopause height).
pub const CIRRUS_ALT_M: f64 = 9500.0;
/// A cirrus streak's peak opacity: thin cirrus has a visible optical depth
/// of 0.03-0.3 (Sassen and Cho 1992, "Subvisual-thin cirrus lidar dataset
/// for satellite verification and climatological research", J. Appl.
/// Meteor. 31: thin cirrus 0.03 < tau < 0.3); 0.3 at a streak's core.
pub const CIRRUS_OPACITY: f64 = 0.3;

pub fn decks(w: &WeatherState) -> Decks {
    let p = &w.profile;
    let s = w.severity.clamp(0.0, 4.0);
    let i = (s.floor() as usize).min(3);
    let t = s - i as f64;
    let mut h = [HEIGHTS[i][0] + (HEIGHTS[i + 1][0] - HEIGHTS[i][0]) * t, HEIGHTS[i][1] + (HEIGHTS[i + 1][1] - HEIGHTS[i][1]) * t];
    if w.kind == Kind::Overcast {
        h = OVERCAST_HEIGHTS;
    }
    // Toward ESE (from 290 deg): east 0.94, south 0.34; the weather's own wind adds.
    let dir = [0.94, 0.34];
    let storm = 1.0 + 0.6 * (s - 3.0).max(0.0);
    let drift_low = [dir[0] * DRIFT_LOW_MS * storm + w.wind[0] * 0.5, dir[1] * DRIFT_LOW_MS * storm + w.wind[1] * 0.5];
    let drift_high = [DRIFT_HIGH_MS, 2.0];
    let stratus = (p.overcast - 0.5).max(0.0) * 2.0;
    let marine_top = match w.kind {
        Kind::Fog => 230.0,
        Kind::Drizzle | Kind::Rain | Kind::Storm => 150.0,
        _ => 180.0,
    };
    Decks {
        low_base_m: h[0],
        low_top_m: h[1],
        // a fair-weather day in San Francisco is mostly cloudless or carries a
        // few small cumulus (with the marine layer offshore); the JS clear deck's
        // 0.14 cover drew the same scattered field over every clear hour. The
        // clear rung keeps 0.4 of it (0.056; r1 0.6); the other rungs are unchanged.
        coverage: p.deck_coverage * if p.overcast < 0.3 { FAIR_WEATHER_COVER } else { 1.0 },
        low_opacity: p.deck_low,
        extinction: CUMULUS_SIGMA + (STRATUS_SIGMA - CUMULUS_SIGMA) * stratus.min(1.0),
        high_alt_m: 5200.0,
        // in fog the low deck is the marine layer and the sky over it is
        // clear (the fog bucket's 0.35 high deck, at full cover since its
        // threshold follows the low deck's 0.92, drew a grey mackerel sky
        // over the fog seen from above): a trace of it is kept.
        high_opacity: p.deck_high * if w.kind == Kind::Fog { 0.15 } else { 1.0 },
        drift_low,
        drift_high,
        beam: p.beam,
        marine_sigma: p.marine,
        marine_top_m: marine_top,
        marine_swing_m: 110.0,
        marine_drift: [3.0 * dir[0], 3.0 * dir[1]],
    }
    .with_sky(p.overcast < 0.3, sky_state(), eo_look::features::daylight().cirrus)
}

impl Decks {
    /// The clear rung's sky (`clear`: the weather's overcast under 0.3): a
    /// cloudless day has no deck, a cirrus day only the cirrus (with
    /// `cirrus` off, the fair-weather sky), a fair day the decks unchanged.
    pub fn with_sky(mut self, clear: bool, state: SkyState, cirrus: bool) -> Self {
        if !clear {
            return self;
        }
        match state {
            SkyState::Cloudless => {
                self.coverage = 0.0;
                self.high_opacity = 0.0;
            }
            SkyState::Cirrus if cirrus => {
                self.coverage = 0.0;
                self.high_alt_m = CIRRUS_ALT_M;
                self.high_opacity = CIRRUS_OPACITY;
            }
            _ => {}
        }
        self
    }
}

/// Where the drift has moved a deck at `world_s`, wrapped to the tile (so
/// the GPU's f32 offset stays small).
pub fn drift_offset(drift: [f64; 2], world_s: f64, tile_m: f64) -> [f64; 2] {
    [(drift[0] * world_s).rem_euclid(tile_m), (drift[1] * world_s).rem_euclid(tile_m)]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_sheet_covers_exactly_its_coverage() {
        let s = bake_sheet(7);
        for c in [0.14, 0.5, 0.92] {
            let thr = (1.0 - c) * 255.0;
            let covered = s.iter().filter(|p| p[0] as f64 > thr).count() as f64 / s.len() as f64;
            assert!((covered - c).abs() < 0.01, "coverage {c}: {covered}");
        }
    }
    #[test]
    fn a_clear_day_draws_its_sky_from_the_month() {
        for sh in KSFO_CLEAR_DAY_SHARES {
            assert!((sh.iter().sum::<f64>() - 1.0).abs() < 0.02, "{sh:?}");
        }
        assert_eq!(month_of(1.0), 0);
        assert_eq!(month_of(103.0), 3);
        assert_eq!(month_of(265.0), 8);
        assert_eq!(month_of(365.0), 11);
        assert_eq!(drawn_state(150.0), drawn_state(150.0));
        // over a year the draws follow the shares: every state turns up, summer is the most cloudless
        let count = |m: std::ops::Range<u32>, k: SkyState| m.filter(|d| drawn_state(*d as f64) == k).count();
        let all: Vec<usize> = [SkyState::Cloudless, SkyState::Cirrus, SkyState::FairCumulus].iter().map(|k| count(1..366, *k)).collect();
        assert!(all.iter().all(|n| *n > 30), "{all:?}");
        assert!(count(182..244, SkyState::Cloudless) > count(1..60, SkyState::Cloudless), "July-August against January-February");
    }

    #[test]
    fn the_sky_state_only_moves_the_clear_rung() {
        let w = crate::weather::preset(Kind::Clear);
        let d = decks(&w);
        let fair = d.with_sky(true, SkyState::FairCumulus, true);
        assert_eq!(fair.high_alt_m, d.high_alt_m);
        let none = d.with_sky(true, SkyState::Cloudless, true);
        assert_eq!((none.coverage, none.high_opacity), (0.0, 0.0));
        let ci = d.with_sky(true, SkyState::Cirrus, true);
        assert!(ci.high_alt_m >= 8000.0 && ci.high_alt_m <= 11000.0 && ci.coverage == 0.0);
        let off = d.with_sky(true, SkyState::Cirrus, false);
        assert_eq!((off.coverage, off.high_alt_m, off.high_opacity), (d.coverage, d.high_alt_m, d.high_opacity));
        let rain = decks(&crate::weather::preset(Kind::Rain));
        let r2 = rain.with_sky(false, SkyState::Cloudless, true);
        assert_eq!(r2.coverage, rain.coverage);
    }

    #[test]
    fn the_sheet_tiles() {
        // Opposite edges continue each other (no seam): mean step across the edge like inside.
        let n = SHEET_RES as f64;
        let d_in: f64 = (0..64).map(|y| (fbm(0.5, y as f64 / 64.0, 8, 5, 1) - fbm(0.5 + 1.0 / n, y as f64 / 64.0, 8, 5, 1)).abs()).sum();
        let d_edge: f64 = (0..64).map(|y| (fbm(1.0 - 0.5 / n, y as f64 / 64.0, 8, 5, 1) - fbm(0.5 / n, y as f64 / 64.0, 8, 5, 1)).abs()).sum();
        assert!(d_edge < d_in * 3.0 + 0.05, "edge step {d_edge} inside {d_in}");
    }
}
