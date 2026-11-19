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
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicU8, Ordering};

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
    /// The second high deck, the cirrus (`--no-cirrus2`: none, the cirrus is
    /// drawn in the high deck above): its height (m; 0 when off) and its
    /// amount, the share of the sky it covers (the METAR eighths / 8) x its
    /// fade (`Decks::cirrus`).
    pub cirrus_alt_m: f64,
    pub cirrus_amount: f64,
}

/// The second high deck (`Decks::cirrus`), as the sky uniform carries it
/// (sky_common.wgsl `cirrus`, read by sky_volume.wgsl `vol_cirrus2`): its
/// height (m; 0: no second deck, round 2's one high deck) and its opacity in
/// the sense of the METAR sky cover, the share of the sky the deck hides: the
/// cloud amount in eighths / 8 x its fade. How thick the cloud is where it
/// covers is fixed (`CIRRUS_TAU`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CirrusDeck {
    pub alt_m: f64,
    pub opacity: f64,
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

/// The KSFO clear-day table, generated by eo-look/data/ksfo-clear-day-sky.py
/// from the airport's METARs (see its NOTICE): per month, the counts of
/// days with low cloud, cloudless, cirrus-only and fair.
const KSFO_CLEAR_DAY_CSV: &str = include_str!("../../eo-look/data/ksfo-clear-day-sky.csv");

/// The share of a month's clear days at San Francisco airport (KSFO) that
/// are cloudless, cirrus-only and fair (scattered low cloud), January first:
/// the routine METARs of 2014-2023, 10:00 to 16:59 local time, as
/// `KSFO_CLEAR_DAY_CSV` counts them (182-286 clear days a month).
pub fn ksfo_clear_day_shares() -> &'static [[f64; 3]; 12] {
    static T: OnceLock<[[f64; 3]; 12]> = OnceLock::new();
    T.get_or_init(|| {
        let mut t = [[0.0; 3]; 12];
        for l in KSFO_CLEAR_DAY_CSV.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("month")) {
            let v: Vec<f64> = l.split(',').map(|x| x.trim().parse().expect("ksfo-clear-day-sky.csv: a number")).collect();
            let clear = v[2] + v[3] + v[4];
            t[v[0] as usize - 1] = [v[2] / clear, v[3] / clear, v[4] / clear];
        }
        t
    })
}
/// The KSFO cirrus-day amounts, generated by eo-look/data/ksfo-clear-day-sky.py
/// --amount from the same METARs (see ksfo-cirrus-amount.NOTICE.txt): per
/// month, the cirrus days by their high-cloud code FEW, SCT, BKN, OVC.
const KSFO_CIRRUS_AMOUNT_CSV: &str = include_str!("../../eo-look/data/ksfo-cirrus-amount.csv");

/// The eighths each METAR cover code stands for, the middle of its range:
/// FEW 1-2 oktas, SCT 3-4, BKN 5-7, OVC 8 (WMO-No. 306, FM 15 METAR
/// regulation 15.9.1.1; NWS Federal Meteorological Handbook No. 1, chapter 9).
pub const COVER_OKTAS: [f64; 4] = [1.5, 3.5, 6.0, 8.0];

/// The share of a month's cirrus days (January first) whose high cloud is
/// FEW, SCT, BKN and OVC (`KSFO_CIRRUS_AMOUNT_CSV`).
pub fn ksfo_cirrus_amounts() -> &'static [[f64; 4]; 12] {
    static T: OnceLock<[[f64; 4]; 12]> = OnceLock::new();
    T.get_or_init(|| {
        let mut t = [[0.0; 4]; 12];
        for l in KSFO_CIRRUS_AMOUNT_CSV.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("month")) {
            let v: Vec<f64> = l.split(',').map(|x| x.trim().parse().expect("ksfo-cirrus-amount.csv: a number")).collect();
            let n = v[1] + v[2] + v[3] + v[4];
            t[v[0] as usize - 1] = [v[1] / n, v[2] / n, v[3] / n, v[4] / n];
        }
        t
    })
}

/// The month's most frequent cirrus amount, eighths: a held cirrus sky (a
/// shot's `sky`) without its own `sky_oktas`.
pub fn modal_oktas(day_of_year: f64) -> f64 {
    let a = ksfo_cirrus_amounts()[month_of(day_of_year)];
    let k = (0..4).fold(0, |b, i| if a[i] > a[b] { i } else { b });
    COVER_OKTAS[k]
}

/// A cirrus day's amount, eighths, drawn from the month's amounts with a hash
/// of the date and the game day (as `drawn_state`).
pub fn drawn_oktas(day_of_year: f64, game_day: u32) -> f64 {
    let u = hash01(SKY_DRAW_SEED ^ 0x0c7a_5eed ^ game_day.wrapping_mul(0x9e37_79b9), day_of_year.floor().max(0.0) as u32, 29);
    let a = ksfo_cirrus_amounts()[month_of(day_of_year)];
    let mut acc = 0.0;
    for k in 0..4 {
        acc += a[k];
        if u < acc {
            return COVER_OKTAS[k];
        }
    }
    COVER_OKTAS[3]
}

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

const SKY_DRAW_SEED: u32 = 0x5c1e_a7e5;

/// The clear rung's sky on a game day (the world clock's day, `WeatherState::day`)
/// whose date is `day_of_year`: drawn from the month's shares
/// (`ksfo_clear_day_shares`) with a hash of the date and the game day, so
/// the same day draws the same sky and each new game day draws afresh.
pub fn drawn_state(day_of_year: f64, game_day: u32) -> SkyState {
    let u = hash01(SKY_DRAW_SEED ^ game_day.wrapping_mul(0x632b_e5ab), day_of_year.floor().max(0.0) as u32, 17);
    let [cloudless, cirrus, fair] = ksfo_clear_day_shares()[month_of(day_of_year)];
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
static GAME_DAY: AtomicU32 = AtomicU32::new(u32::MAX);

/// Hold the clear rung's sky (a shot's `sky`, the observation of its day),
/// or None to draw it from the day (`drawn_state`).
pub fn set_sky_state(s: Option<SkyState>) {
    STATE.store(SkyState::code(s), Ordering::Relaxed);
}

/// The held cirrus amount (eighths), as f64 bits; NaN: none.
static OKTAS: AtomicU64 = AtomicU64::new(0x7ff8_0000_0000_0000);

/// Hold a cirrus sky's amount in eighths (a shot's `sky_oktas`, from the day's
/// METAR), or None (a held cirrus sky takes the month's most frequent,
/// `modal_oktas`; a drawn one its own draw, `drawn_oktas`).
pub fn set_sky_oktas(o: Option<f64>) {
    OKTAS.store(o.map_or(f64::NAN, |v| v.clamp(0.0, 8.0)).to_bits(), Ordering::Relaxed);
}

/// The held cirrus amount, if any.
pub fn held_sky_oktas() -> Option<f64> {
    Some(f64::from_bits(OKTAS.load(Ordering::Relaxed))).filter(|v| !v.is_nan())
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

/// Hold the game day the clear sky is drawn for (a shot's `game_day`), or
/// None to take the weather's own (`WeatherState::day`).
pub fn set_game_day(d: Option<u32>) {
    GAME_DAY.store(d.unwrap_or(u32::MAX), Ordering::Relaxed);
}

/// The game day a weather's clear sky is drawn for.
pub fn game_day_of(w: &WeatherState) -> u32 {
    match GAME_DAY.load(Ordering::Relaxed) {
        u32::MAX => w.day,
        d => d,
    }
}

/// The clear rung's sky in force on a game day (`--no-clear-sky-state`:
/// always the fair-weather sky).
pub fn sky_state(game_day: u32) -> SkyState {
    if !eo_look::features::daylight().clear_sky_state {
        return SkyState::FairCumulus;
    }
    held_sky_state().unwrap_or_else(|| drawn_state(eo_look::sun::day_of_year(), game_day))
}

/// The clear rung's sky in force for a weather.
pub fn sky_state_for(w: &WeatherState) -> SkyState {
    sky_state(game_day_of(w))
}

/// The date a game day's clear sky is drawn for: the held date (a shot's), or
/// the live calendar's date of that game day (eo-look `sun::date_of_game_day`;
/// the game's 22 September with `--no-live-calendar`).
pub fn draw_date(game_day: u32) -> f64 {
    if eo_look::sun::day_held() { eo_look::sun::day_of_year() } else { eo_look::sun::date_of_game_day(game_day) }
}

/// A game day's clear sky and, on a cirrus day, its amount in eighths.
pub fn day_sky(game_day: u32) -> (SkyState, f64) {
    let date = draw_date(game_day);
    if !eo_look::features::daylight().clear_sky_state {
        return (SkyState::FairCumulus, 0.0);
    }
    let (state, oktas) = match held_sky_state() {
        Some(s) => (s, held_sky_oktas().unwrap_or_else(|| modal_oktas(date))),
        None => (drawn_state(date, game_day), held_sky_oktas().unwrap_or_else(|| drawn_oktas(date, game_day))),
    };
    (state, if state == SkyState::Cirrus { oktas } else { 0.0 })
}

/// The clear rung's sky as weights of its three states (they sum to 1) and
/// the cirrus's amount (the share of the sky it covers, weighted the same):
/// one state on most of a game day; across the world clock's night window
/// (eo-look `sun::day_change`, quantised to `SKY_CHANGE_STEPS`) the game day's
/// state gives way to the next one's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyMix {
    pub cloudless: f64,
    pub cirrus: f64,
    pub fair: f64,
    pub cirrus_amount: f64,
}

/// The sky change's steps through its window (the look frame, and its decks,
/// are made again on each: a 1/64 step of a faint deck's opacity is invisible).
pub const SKY_CHANGE_STEPS: f64 = 64.0;

impl SkyMix {
    pub fn pure(state: SkyState, oktas: f64) -> Self {
        let mut m = SkyMix { cloudless: 0.0, cirrus: 0.0, fair: 0.0, cirrus_amount: 0.0 };
        m.add(state, oktas, 1.0);
        m
    }
    fn add(&mut self, state: SkyState, oktas: f64, w: f64) {
        match state {
            SkyState::Cloudless => self.cloudless += w,
            SkyState::Cirrus => {
                self.cirrus += w;
                self.cirrus_amount += w * oktas / 8.0;
            }
            SkyState::FairCumulus => self.fair += w,
        }
    }
    /// The state with the most weight (the first of a tie, the game day's own).
    pub fn dominant(&self) -> SkyState {
        if self.cirrus > self.cloudless.max(self.fair) {
            SkyState::Cirrus
        } else if self.fair > self.cloudless {
            SkyState::FairCumulus
        } else {
            SkyState::Cloudless
        }
    }
}

/// The clear rung's sky for a weather (`SkyMix`): a held sky or game day (a
/// shot's) and `--no-sky-day-fade` give the game day's own state.
pub fn sky_mix(w: &WeatherState) -> SkyMix {
    let g = game_day_of(w);
    let (s0, o0) = day_sky(g);
    let held = held_sky_state().is_some() || GAME_DAY.load(Ordering::Relaxed) != u32::MAX;
    if held || !eo_look::season::features().sky_day_fade {
        return SkyMix::pure(s0, o0);
    }
    let f = (eo_look::sun::day_change(w.clock_s) * SKY_CHANGE_STEPS).round() / SKY_CHANGE_STEPS;
    if f <= 0.0 {
        return SkyMix::pure(s0, o0);
    }
    let (s1, o1) = day_sky(g.saturating_add(1));
    let mut m = SkyMix { cloudless: 0.0, cirrus: 0.0, fair: 0.0, cirrus_amount: 0.0 };
    m.add(s0, o0, 1.0 - f);
    m.add(s1, o1, f);
    m
}

/// What the look frame's decks depend on beyond the weather's own key
/// (eo-render sky.rs `weather_key`): the sun's date and the clear sky's mix,
/// quantised as they step. A look frame made under another key is stale (the
/// hand-off to eo-render look.rs `frame` compares it).
pub fn sky_key(w: &WeatherState) -> [i64; 5] {
    let m = sky_mix(w);
    let q = |v: f64| (v * 4096.0).round() as i64;
    [q(eo_look::sun::day_of_year()), q(m.cloudless), q(m.cirrus), q(m.fair), q(m.cirrus_amount)]
}

/// How much a weather is the clear rung, 0..1: 1 under an overcast of 0.1,
/// 0 from 0.3 (the rung's old hard edge), smooth between, so that the clear
/// sky's state fades as the schedule's severity carries clear into fog
/// (`profile_for_severity` blends the overcast continuously).
pub fn clear_weight(overcast: f64) -> f64 {
    let t = ((overcast - 0.1) / 0.2).clamp(0.0, 1.0);
    1.0 - t * t * (3.0 - 2.0 * t)
}

/// The cirrus deck's height: 9.5 km. The WMO International Cloud Atlas
/// (2017, "Definitions: approximate height ranges of the etages") puts the
/// high etage, where cirrus forms, at 5-13 km in temperate regions; under a
/// mid-latitude tropopause at 10-12 km it sits mostly at 8-11 km, whose
/// middle this is (INFERRED from the etage and the tropopause height).
pub const CIRRUS_ALT_M: f64 = 9500.0;
/// A cirrus streak's peak visible optical depth: thin cirrus has 0.03-0.3
/// (Sassen and Cho 1992, "Subvisual-thin cirrus lidar dataset for satellite
/// verification and climatological research", J. Appl. Meteor. 31: thin
/// cirrus 0.03 < tau < 0.3); 0.3 at a streak's core. The shader turns it
/// into an opacity, 1 - exp(-tau x the streak's shape) (sky_volume.wgsl
/// `vol_cirrus`).
pub const CIRRUS_TAU: f64 = 0.3;
/// The cirrus deck's peak optical depth in the second deck (sky_volume.wgsl
/// `VOL_CI2_TAU`): a sheet's core, the same thin-cirrus 0.3.
pub const CIRRUS2_TAU: f64 = CIRRUS_TAU;
/// The fair-weather high sheet's height (the port's 5.2 km), and the band
/// above it that carries the sheet's fade while the clear sky's state
/// changes: the deck uniform has one altitude and one opacity, and the
/// sheet's opacity has a floor in the shader (sky_volume.wgsl, the fair
/// branch), so a sheet at `FAIR_HIGH_ALT_M + FADE_BAND_M x (1 - f)` is drawn
/// at `FAIR_HIGH_ALT_M` with its opacity x f. (A second deck uniform would be
/// cleaner; the uniform's layout is eo-render sky.rs, not this crate's.)
pub const FAIR_HIGH_ALT_M: f64 = 5200.0;
pub const FADE_BAND_M: f64 = 1000.0;

/// The decks as the weather gives them, before the clear sky's state.
/// `smooth`: the clear rung's cumulus share (`FAIR_WEATHER_COVER`) follows
/// `clear_weight` instead of switching at an overcast of 0.3 (the same at 0
/// and from 0.3).
fn raw_decks(w: &WeatherState, smooth: bool) -> Decks {
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
    let fair_share = if smooth {
        1.0 + (FAIR_WEATHER_COVER - 1.0) * clear_weight(p.overcast)
    } else if p.overcast < 0.3 {
        FAIR_WEATHER_COVER
    } else {
        1.0
    };
    Decks {
        low_base_m: h[0],
        low_top_m: h[1],
        // a fair-weather day in San Francisco is mostly cloudless or carries a
        // few small cumulus (with the marine layer offshore); the JS clear deck's
        // 0.14 cover drew the same scattered field over every clear hour. The
        // clear rung keeps 0.4 of it (0.056; r1 0.6); the other rungs are unchanged.
        coverage: p.deck_coverage * fair_share,
        low_opacity: p.deck_low,
        extinction: CUMULUS_SIGMA + (STRATUS_SIGMA - CUMULUS_SIGMA) * stratus.min(1.0),
        high_alt_m: FAIR_HIGH_ALT_M,
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
        cirrus_alt_m: 0.0,
        cirrus_amount: 0.0,
    }
}

pub fn decks(w: &WeatherState) -> Decks {
    let f = eo_look::features::daylight();
    let wc = if f.clear_sky_state { clear_weight(w.profile.overcast) } else { 0.0 };
    let m = sky_mix(w);
    if eo_look::season::features().cirrus2 {
        raw_decks(w, f.clear_sky_state).with_sky_mix(wc, m, f.cirrus)
    } else {
        // round 2's one high deck draws one state: across a day's change, the
        // one with the most weight
        raw_decks(w, f.clear_sky_state).with_sky(wc, m.dominant(), f.cirrus)
    }
}

impl Decks {
    /// The clear rung's sky, by how much the weather is that rung (`wc`,
    /// `clear_weight`): a cloudless day fades the low deck and the high sheet
    /// out; a cirrus day fades the low deck out and, across `wc` 0 to 0.5,
    /// the 5.2 km sheet out, then across 0.5 to 1 the cirrus in at its own
    /// height (with `cirrus` off, the fair-weather sky); a fair day keeps the
    /// decks. Every field is continuous in `wc`; the deck's height moves only
    /// where its opacity is 0.
    pub fn with_sky(mut self, wc: f64, state: SkyState, cirrus: bool) -> Self {
        let wc = wc.clamp(0.0, 1.0);
        if wc <= 0.0 {
            return self;
        }
        match state {
            SkyState::Cloudless => {
                self.coverage *= 1.0 - wc;
                self.fade_high(1.0 - wc);
            }
            SkyState::Cirrus if cirrus => {
                self.coverage *= 1.0 - wc;
                if wc > 0.5 {
                    self.high_alt_m = CIRRUS_ALT_M;
                    self.high_opacity = CIRRUS_TAU * (2.0 * wc - 1.0);
                } else {
                    self.fade_high(1.0 - 2.0 * wc);
                }
            }
            _ => {}
        }
        self
    }

    /// The clear rung's sky as a mix of its states (`SkyMix`), with the
    /// cirrus in its own deck (`--no-cirrus2` keeps `with_sky`): by how much
    /// the weather is that rung (`wc`), the cloudless and cirrus shares take
    /// the low deck and the 5.2 km sheet away, each sheet at its own height,
    /// and the cirrus share brings the cirrus deck in at its METAR amount. The
    /// sheet's opacity floor (sky_volume.wgsl's fair branch, `0.10` while the
    /// low cover is under 0.3) is applied here, before the fade, and the
    /// shader takes the opacity as it is when the second deck is on. Every
    /// field is continuous in `wc` and in the mix.
    pub fn with_sky_mix(mut self, wc: f64, m: SkyMix, cirrus: bool) -> Self {
        let wc = wc.clamp(0.0, 1.0);
        let (gone_ci, amount) = if cirrus { (m.cirrus, m.cirrus_amount) } else { (0.0, 0.0) };
        let gone = (wc * (m.cloudless + gone_ci)).clamp(0.0, 1.0);
        let raw_cover = self.coverage;
        self.coverage *= 1.0 - gone;
        if self.high_opacity > 0.0 && raw_cover > 0.0 {
            let ci = ((0.3 - self.coverage) / 0.3).clamp(0.0, 1.0);
            self.high_opacity = self.high_opacity.max(0.10 * ci) * (1.0 - gone);
        } else {
            // (the volume pass draws no high sheet without a low cover)
            self.high_opacity = 0.0;
        }
        self.cirrus_alt_m = CIRRUS_ALT_M;
        self.cirrus_amount = (wc * amount).clamp(0.0, 1.0);
        self
    }

    /// The second high deck (the cirrus beside the 5.2 km sheet, each at its
    /// own height): `CIRRUS_ALT_M` and the amount with `with_sky_mix`; height
    /// 0 (none: the shader's round-2 high deck) otherwise.
    pub fn cirrus(&self) -> CirrusDeck {
        CirrusDeck { alt_m: self.cirrus_alt_m, opacity: self.cirrus_amount }
    }

    /// The fair sheet at `f` (0..1) of its opacity (`FADE_BAND_M`).
    fn fade_high(&mut self, f: f64) {
        if f <= 0.0 {
            self.high_opacity = 0.0;
        } else if f < 1.0 {
            self.high_alt_m = FAIR_HIGH_ALT_M + FADE_BAND_M * (1.0 - f);
        }
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
    fn the_second_high_deck_carries_the_cirrus() {
        let w = crate::weather::preset(Kind::Clear);
        let d = raw_decks(&w, true);
        // round 2's path: no second deck
        assert_eq!(d.with_sky(1.0, SkyState::Cirrus, true).cirrus(), CirrusDeck { alt_m: 0.0, opacity: 0.0 });
        // a held SCT cirrus day: the cirrus at its height with 3.5 / 8 of the sky, no sheet, no low deck
        let ci = d.with_sky_mix(1.0, SkyMix::pure(SkyState::Cirrus, 3.5), true);
        assert_eq!(ci.cirrus(), CirrusDeck { alt_m: CIRRUS_ALT_M, opacity: 3.5 / 8.0 });
        assert_eq!((ci.coverage, ci.high_opacity, ci.high_alt_m), (0.0, 0.0, FAIR_HIGH_ALT_M));
        // a fair day keeps the round-2 decks (the sheet's floor applied on the CPU)
        let fair = d.with_sky_mix(1.0, SkyMix::pure(SkyState::FairCumulus, 0.0), true);
        let ci0 = ((0.3 - d.coverage) / 0.3).clamp(0.0, 1.0);
        assert_eq!((fair.coverage, fair.high_alt_m, fair.cirrus().opacity), (d.coverage, FAIR_HIGH_ALT_M, 0.0));
        assert!((fair.high_opacity - d.high_opacity.max(0.10 * ci0)).abs() < 1e-12);
        // cloudless: nothing
        let none = d.with_sky_mix(1.0, SkyMix::pure(SkyState::Cloudless, 0.0), true);
        assert_eq!((none.coverage, none.high_opacity, none.cirrus().opacity), (0.0, 0.0, 0.0));
        // --no-cirrus: a cirrus day is the fair sky
        let off = d.with_sky_mix(1.0, SkyMix::pure(SkyState::Cirrus, 3.5), false);
        assert_eq!((off.coverage, off.high_opacity, off.cirrus().opacity), (fair.coverage, fair.high_opacity, 0.0));
        // other weathers: the sheet as round 2 drew it, no cirrus
        for k in [Kind::Overcast, Kind::Fog, Kind::Rain] {
            let w = crate::weather::preset(k);
            let r = raw_decks(&w, true);
            let m = r.with_sky_mix(clear_weight(w.profile.overcast), SkyMix::pure(SkyState::Cirrus, 6.0), true);
            let ci = ((0.3 - r.coverage) / 0.3).clamp(0.0, 1.0);
            assert_eq!((m.coverage, m.cirrus().opacity), (r.coverage, 0.0), "{k:?}");
            assert!((m.high_opacity - if r.high_opacity > 0.0 { r.high_opacity.max(0.10 * ci) } else { 0.0 }).abs() < 1e-12, "{k:?}");
        }
    }

    #[test]
    fn the_cirrus_amounts_come_from_the_metars() {
        let t = ksfo_cirrus_amounts();
        for (m, a) in t.iter().enumerate() {
            assert!((a.iter().sum::<f64>() - 1.0).abs() < 1e-9, "month {m}: {a:?}");
        }
        // the generated table: April 38 / 54 / 49 / 1, June 50 / 33 / 20 / 0
        assert!((t[3][1] - 54.0 / 142.0).abs() < 1e-12);
        // the photographs' months: April's most frequent is SCT (the headland's METAR: SCT200),
        // June's FEW (Crissy's: FEW200)
        assert_eq!(modal_oktas(103.0), 3.5);
        assert_eq!(modal_oktas(157.0), 1.5);
        // the draws follow the month's amounts
        let n = 4000;
        let few = (0..n).filter(|g| drawn_oktas(265.0, *g) == 1.5).count() as f64 / n as f64;
        assert!((few - t[8][0]).abs() < 0.03, "{few} against {}", t[8][0]);
        assert!(COVER_OKTAS.windows(2).all(|p| p[0] < p[1]) && COVER_OKTAS[3] == 8.0);
    }

    /// A clock `h` hours into game day `d`.
    fn clock(d: u32, h: f64) -> f64 {
        d as f64 * 86400.0 + h * 3600.0
    }

    #[test]
    fn a_clear_day_gives_way_to_the_next_over_the_night() {
        // two neighbouring game days with different skies (on the live calendar's dates)
        let g = (0..200).find(|g| day_sky(*g).0 != day_sky(g + 1).0).expect("a change of sky");
        let w = |t: f64| WeatherState { day: crate::weather::game_day(t), clock_s: t, ..crate::weather::preset(Kind::Clear) };
        // the day's own sky until the window, the next day's from midnight
        assert_eq!(sky_mix(&w(clock(g, 12.0))), SkyMix::pure(day_sky(g).0, day_sky(g).1));
        assert_eq!(sky_mix(&w(clock(g, 20.9))), SkyMix::pure(day_sky(g).0, day_sky(g).1));
        assert_eq!(sky_mix(&w(clock(g + 1, 0.0))), SkyMix::pure(day_sky(g + 1).0, day_sky(g + 1).1));
        // through the window no drawn field steps by more than a small amount
        let raw = raw_decks(&crate::weather::preset(Kind::Clear), true);
        let drawn = |t: f64| {
            let d = raw.with_sky_mix(1.0, sky_mix(&w(t)), true);
            (d.coverage, d.high_opacity, d.cirrus().opacity)
        };
        let mut prev = drawn(clock(g, 20.0));
        let mut moved = 0.0;
        for k in 1..=5 * 360 {
            let t = clock(g, 20.0) + k as f64 * 10.0;
            let d = drawn(t);
            let step = (d.0 - prev.0).abs().max((d.1 - prev.1).abs()).max((d.2 - prev.2).abs());
            assert!(step < 0.02, "a step of {step} at {t} s: {prev:?} -> {d:?}");
            moved += step;
            prev = d;
        }
        assert!(moved > 0.01, "the sky did not change across the night: {moved}");
        // with the fade held off by a shot's game day: the day's own sky throughout
        set_game_day(Some(g));
        assert_eq!(sky_mix(&w(clock(g, 22.5))), SkyMix::pure(day_sky(g).0, day_sky(g).1));
        set_game_day(None);
    }

    #[test]
    fn the_second_deck_fades_with_the_weather_without_a_jump() {
        for m in [SkyMix::pure(SkyState::Cloudless, 0.0), SkyMix::pure(SkyState::Cirrus, 6.0), SkyMix::pure(SkyState::FairCumulus, 0.0)] {
            let at = |s: f64| {
                let p = crate::weather::profile_for_severity(s);
                let w = WeatherState { severity: s, profile: p, ..crate::weather::preset(Kind::Clear) };
                raw_decks(&w, true).with_sky_mix(clear_weight(p.overcast), m, true)
            };
            let mut prev = at(0.0);
            for k in 1..=1000 {
                let d = at(k as f64 * 0.001);
                assert!((d.coverage - prev.coverage).abs() < 0.01, "{m:?} at {k}");
                assert!((d.high_opacity - prev.high_opacity).abs() < 0.01, "{m:?} at {k}: sheet {} -> {}", prev.high_opacity, d.high_opacity);
                assert!((d.cirrus().opacity - prev.cirrus().opacity).abs() < 0.01, "{m:?} at {k}: cirrus");
                assert_eq!(d.high_alt_m, FAIR_HIGH_ALT_M);
                prev = d;
            }
        }
    }

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
        let shares = ksfo_clear_day_shares();
        for sh in shares {
            assert!((sh.iter().sum::<f64>() - 1.0).abs() < 1e-9, "{sh:?}");
        }
        // the generated table: September 104 / 107 / 50 clear days
        assert!((shares[8][1] - 107.0 / 261.0).abs() < 1e-12, "{:?}", shares[8]);
        assert_eq!(month_of(1.0), 0);
        assert_eq!(month_of(103.0), 3);
        assert_eq!(month_of(265.0), 8);
        assert_eq!(month_of(365.0), 11);
        assert_eq!(drawn_state(150.0, 3), drawn_state(150.0, 3));
        // over a year the draws follow the shares: every state turns up, summer is the most cloudless
        let count = |m: std::ops::Range<u32>, k: SkyState| m.filter(|d| drawn_state(*d as f64, 0) == k).count();
        let all: Vec<usize> = [SkyState::Cloudless, SkyState::Cirrus, SkyState::FairCumulus].iter().map(|k| count(1..366, *k)).collect();
        assert!(all.iter().all(|n| *n > 30), "{all:?}");
        assert!(count(182..244, SkyState::Cloudless) > count(1..60, SkyState::Cloudless), "July-August against January-February");
        // on one date, successive game days follow the month's shares too (not one fixed sky)
        let days: Vec<SkyState> = (0..2000).map(|g| drawn_state(265.0, g)).collect();
        for (k, share) in [SkyState::Cloudless, SkyState::Cirrus, SkyState::FairCumulus].iter().zip(shares[8]) {
            let n = days.iter().filter(|s| *s == k).count() as f64 / days.len() as f64;
            assert!((n - share).abs() < 0.04, "{k:?}: {n} against {share}");
        }
    }

    #[test]
    fn the_game_day_comes_from_the_world_clock() {
        use crate::weather::{game_day, preset_at};
        assert_eq!((game_day(0.0), game_day(86399.0), game_day(86400.0), game_day(3.5 * 86400.0)), (0, 0, 1, 3));
        assert_eq!(preset_at(Kind::Clear, 2.0 * 86400.0 + 5.0).day, 2);
        assert_eq!(crate::weather::preset(Kind::Clear).day, 0);
        let seed = crate::hash::schedule_seed("sf-world", 0);
        assert_eq!(crate::weather::state_at(seed, 86400.0 * 4.0 + 100.0).day, 4);
    }

    #[test]
    fn the_sky_state_only_moves_the_clear_rung() {
        let w = crate::weather::preset(Kind::Clear);
        let d = raw_decks(&w, true);
        // the raw clear decks are the port's fair-weather sky
        assert_eq!(d.high_alt_m, FAIR_HIGH_ALT_M);
        assert!((d.coverage - 0.14 * FAIR_WEATHER_COVER).abs() < 1e-12, "{}", d.coverage);
        let fair = d.with_sky(1.0, SkyState::FairCumulus, true);
        assert_eq!((fair.coverage, fair.high_alt_m, fair.high_opacity), (d.coverage, d.high_alt_m, d.high_opacity));
        let none = d.with_sky(1.0, SkyState::Cloudless, true);
        assert_eq!((none.coverage, none.high_opacity), (0.0, 0.0));
        let ci = d.with_sky(1.0, SkyState::Cirrus, true);
        assert!(ci.high_alt_m >= 8000.0 && ci.high_alt_m <= 11000.0 && ci.coverage == 0.0 && ci.high_opacity == CIRRUS_TAU, "{ci:?}");
        // cirrus off: the fair sky, unchanged
        let off = d.with_sky(1.0, SkyState::Cirrus, false);
        assert_eq!((off.coverage, off.high_alt_m, off.high_opacity), (d.coverage, FAIR_HIGH_ALT_M, d.high_opacity));
        // not the clear rung: unchanged
        let rain = raw_decks(&crate::weather::preset(Kind::Rain), true);
        let r2 = rain.with_sky(clear_weight(rain_overcast()), SkyState::Cloudless, true);
        assert_eq!((r2.coverage, r2.high_alt_m, r2.high_opacity), (rain.coverage, rain.high_alt_m, rain.high_opacity));
        // the smooth cumulus share is the old switch at the rungs themselves
        for k in [Kind::Clear, Kind::Fog, Kind::Overcast, Kind::Rain] {
            let w = crate::weather::preset(k);
            assert!((raw_decks(&w, true).coverage - raw_decks(&w, false).coverage).abs() < 1e-12, "{k:?}");
        }
    }

    fn rain_overcast() -> f64 {
        crate::weather::preset(Kind::Rain).profile.overcast
    }

    /// What the volume pass draws of the high deck (sky_volume.wgsl): the
    /// cirrus's peak opacity and the fair sheet's, with its floor and fade.
    fn high_drawn(d: &Decks) -> (f64, f64) {
        if d.high_opacity <= 0.0 {
            return (0.0, 0.0);
        }
        if d.high_alt_m > 7000.0 {
            return (1.0 - (-d.high_opacity).exp(), 0.0);
        }
        let ci = ((0.3 - d.coverage) / 0.3).clamp(0.0, 1.0);
        let f = 1.0 - ((d.high_alt_m - FAIR_HIGH_ALT_M) / FADE_BAND_M).clamp(0.0, 1.0);
        (0.0, d.high_opacity.max(0.10 * ci) * f)
    }

    #[test]
    fn the_clear_sky_fades_with_the_weather_without_a_jump() {
        // the schedule's clear-to-fog blend, in steps of 0.001 severity: no field the
        // shader draws moves by more than a small step between neighbours
        for state in [SkyState::Cloudless, SkyState::Cirrus, SkyState::FairCumulus] {
            let at = |s: f64| {
                let p = crate::weather::profile_for_severity(s);
                let w = WeatherState { severity: s, profile: p, ..crate::weather::preset(Kind::Clear) };
                raw_decks(&w, true).with_sky(clear_weight(p.overcast), state, true)
            };
            let mut prev = at(0.0);
            for k in 1..=1000 {
                let d = at(k as f64 * 0.001);
                let (c0, f0) = high_drawn(&prev);
                let (c1, f1) = high_drawn(&d);
                assert!((d.coverage - prev.coverage).abs() < 0.01, "{state:?} at {k}: coverage {} -> {}", prev.coverage, d.coverage);
                assert!((c1 - c0).abs() < 0.01 && (f1 - f0).abs() < 0.01, "{state:?} at {k}: cirrus {c0} -> {c1}, sheet {f0} -> {f1}");
                prev = d;
            }
        }
        // and the old hard edge did jump (what this replaces)
        let w = |o: f64| WeatherState { profile: crate::weather::Profile { overcast: o, ..crate::weather::CLEAR }, ..crate::weather::preset(Kind::Clear) };
        assert!(raw_decks(&w(0.3001), false).coverage > 2.0 * raw_decks(&w(0.2999), false).coverage);
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
