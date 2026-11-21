//! the sea state and the wave spectrum the GPU's FFT cascades carry,
//! per sea and weather (native/README.md "Waves").
//!
//! - [`sea_state`]: the open Pacific's swell and wind sea and the bay's
//!   fetch-limited wind sea for an hour and a weather: calm mornings, the
//!   afternoon westerly in clear weather (10 m/s over the bay: Hs 0.6 m,
//!   3.3 s), the weather's wind over the water otherwise (17 m/s in a storm).
//! - [`Plan`]: what the GPU builds for a state. JONSWAP systems scaled to
//!   their significant heights (fetch-limited growth, g Hs / U^2 =
//!   1.6e-3 (gF/U^2)^(1/2) and g Tp / U = 0.286 (gF/U^2)^(1/3), capped at the
//!   fully developed sea); the long-period swell from the west-north-west
//!   with a lead train on a lattice mode of cascade 0 (the surf strip's
//!   waves continue it); which cascade carries which system; how much of
//!   each cascade each place keeps (the bay is sheltered from the swell and
//!   has its own wind sea); and the whitecap thresholds on the displacement
//!   Jacobian per cascade and place, set so foam covers the fraction the wind
//!   gives (active whitecaps 3.84e-6 U^3.41, Monahan and O'Muircheartaigh
//!   1980, times the foam they leave behind).
//! - [`build`]: the CPU twin of the GPU's spectrum (`water_spectrum.wgsl`):
//!   the same per-texel numbers in f64, for tests.
//!
//! Directions are travel directions, atan2(z, x) with x east and z south.
//! Each mode carries one draw, travelling along +k with the energy of
//! direction k (the second of h0's two; the first is 0), so the wind sea and
//! the swell run downwind. The JS port, `spectrum`, gives both draws the
//! energy of k: half of its sea runs upwind, a standing sea with twice the
//! variance its spectrum asks for.

use crate::spectrum::{N, Spectrum};

const G: f64 = 9.81;
const TAU: f64 = std::f64::consts::TAU;
const PI: f64 = std::f64::consts::PI;

pub const CASCADES: usize = 4;
/// Cascade tiles (m), not related by simple ratios. Cascade 0 is
/// 315 m x sqrt(68): the lead swell, lattice mode (8, 2), is a 315 m,
/// 14.2 s train travelling toward (0.970, 0.243).
pub const SIZES: [f64; CASCADES] = [2597.55, 397.0, 61.0, 9.7];
/// Band edges by wavelength (m): cascade c carries [CUT_M[c + 1], CUT_M[c]).
/// The bay's wind sea (peak 5-25 m) lives in cascade 1, the swell and the
/// open Pacific's longer sea in cascade 0, the chop in 2, the ripples in 3.
pub const CUT_M: [f64; CASCADES + 1] = [1.0e9, 50.0, 7.0, 1.1, 0.1];
pub const DEPTH_M: f64 = 500.0;
/// The lead swell mode (kx, kz lattice index in cascade 0).
pub const LEAD: [i32; 2] = [8, 2];
/// The lead train's share of the swell's variance.
pub const LEAD_SHARE: f64 = 0.4;
/// Its phase: h = a cos(k.x - w t + phase); -pi/2 continues water.wgsl `w_swell`'s sine.
pub const LEAD_PHASE: f64 = -std::f64::consts::FRAC_PI_2;
/// Choppiness magnitude (the horizontal displacement's gain), raised in a storm.
/// The sign the GPU uses is negative: points gather toward the crests, which
/// sharpen (Gerstner), and the Jacobian falls below 1 on them.
pub const CHOP: f64 = 0.9;
pub const CHOP_STORM: f64 = 0.3;
pub const SEED: u32 = 0x7a7e_5eed;
/// Fully developed caps of the fetch laws (g Hs / U^2, g Tp / U).
const HS_FULL: f64 = 0.24;
const TP_FULL: f64 = 7.5;
/// Wind over open water against the weather's wind (the city's).
pub const OVER_WATER: f64 = 1.4;
/// The afternoon sea breeze over the bay (m/s): calm morning, westerly peak.
pub const BAY_CALM: f64 = 1.0;
pub const BAY_WESTERLY: f64 = 10.0;
/// The open Pacific's north-westerly (m/s): morning, afternoon; and under cloud.
pub const PACIFIC_WIND: [f64; 3] = [4.0, 9.0, 3.0];
/// Travel directions (rad): the bay's westerly through the Gate (toward the
/// east), the Pacific's north-westerly (toward the south-east), the swell
/// from the west-north-west (toward (0.970, 0.243)).
pub const BAY_DIR: f64 = 10.0 * PI / 180.0;
pub const PACIFIC_DIR: f64 = 40.0 * PI / 180.0;
pub const SWELL_DIR: f64 = 0.244_978_663; // atan2(2, 8)
/// Fetch (m): the bay from the Gate; the Pacific clear and in a storm.
pub const BAY_FETCH_M: f64 = 15_000.0;
pub const PACIFIC_FETCH_M: [f64; 2] = [80_000.0, 150_000.0];
/// The swell: Hs (m), Tp (s), cos-2s spread, clear and storm.
pub const SWELL_HS: [f64; 2] = [1.8, 3.0];
pub const SWELL_TP: [f64; 2] = [14.2, 13.0];
pub const SWELL_SPREAD: [f64; 2] = [25.0, 10.0];
/// The swell's share in the bay (through the Gate, past the shelter).
pub const SWELL_LEAK_BAY: f64 = 0.03;
/// Short-wave fade exp(-(k l)^2) (l in m).
pub const SHORT_FADE_M: f64 = 0.012;
/// Visible foam over active whitecaps (the foam trails and streaks they
/// leave): the earlier reading, which took the Monahan and O'Muircheartaigh
/// fit for the active crests alone (kept for `--no-water-foam-cover`).
pub const FOAM_OVER_ACTIVE: f64 = 2.5;
/// The visible foam over the fit (water r1): 1. Monahan and O'Muircheartaigh
/// 1980 (J. Phys. Oceanogr. 10(12), 2094) fit W = 3.84e-6 U^3.41 to
/// photographic whitecap coverage from Monahan (1971) and Toba and Chaen
/// (1973), which counts every whitecap in the frame, the active crests
/// (stage A) and the decaying foam patches they leave (stage B), so it is
/// already the visible foam; multiplying it by the trails a second time drew
/// two and a half times the foam the wind gives (a 6 m/s bay read as confetti).
pub const FOAM_OVER_TOTAL: f64 = 1.0;
/// Visible foam coverage (foam over 0.3) over the instantaneous fraction
/// below the threshold, per cascade, at a 3.3 s lifetime: the trail each
/// breaking crest leaves (longer for the faster-moving pattern of the finer
/// cascades); it grows as the lifetime to the 0.5 (the test
/// `whitecap_coverage_follows_the_wind` measures these).
pub const K_TRAIL: [f64; 3] = [4.5, 5.2, 3.7];
/// The Jacobian's lower tail is thinner than a Gaussian's (its products of
/// strains): the threshold sits at this share of the Gaussian quantile.
pub const Z_TAIL: f64 = 0.95;
/// Foam e-folding lifetime (s), light wind and storm.
pub const FOAM_TAU_S: [f64; 2] = [5.0, 11.0];
/// Foam injection rate (1/s) where the surface folds.
pub const FOAM_RATE: f64 = 8.0;
/// Along-wind spreading of the foam (texel^2 / s): streaks.
pub const FOAM_SPREAD: f64 = 1.5;
/// Foam drift downwind (x U10, m/s).
pub const FOAM_DRIFT: f64 = 0.03;
/// Whitecap cascades: 0..WHITECAP_CASCADES break (the ripples do not).
pub const WHITECAP_CASCADES: usize = 3;
/// (iteration 2, exposure) the fetch factors (B = sqrt(F / `BAY_FETCH_M`),
/// from the exposure field's floor to 1) and the wind ratios (G, from the
/// full lee at the narrowest gain to the most the exposure texture holds, 2;
/// exposure.rs `FETCH_FLOOR`, `LEE_TAKE`, `GAIN_RANGE`) at which
/// `Plan::expo_amp` tabulates the bay's wind sea per cascade.
/// Eight of each: the spectrum's peak moving through a cascade's band makes
/// its amplitude bend between nodes (four left 20 % between 0.8 and 1.3).
pub const EXPO_FETCH_NODES: [f64; 8] = [crate::exposure::FETCH_FLOOR, 0.25, 0.35, 0.45, 0.55, 0.7, 0.85, 1.0];
pub const EXPO_WIND_NODES: [f64; 8] = [(1.0 - crate::exposure::LEE_TAKE) * crate::exposure::GAIN_RANGE[0], 0.5, 0.7, 0.85, 1.0, 1.25, 1.6, 2.0];
/// The most a place's cascade may carry of the GPU field (the earlier cap:
/// `amp_bay`'s 1.4 times the exposure's 1.6).
pub const EXPO_AMP_CAP: f64 = 1.4 * 1.6;

fn smooth(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// A wind sea: U10 (m/s), travel direction (rad), fetch (m).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindSea {
    pub u10: f64,
    pub dir: f64,
    pub fetch_m: f64,
}

impl WindSea {
    fn u(&self) -> f64 {
        self.u10.max(0.5)
    }
    fn nondim(&self) -> f64 {
        G * self.fetch_m / (self.u() * self.u())
    }
    /// Significant height (m) by fetch-limited growth.
    pub fn hs(&self) -> f64 {
        (1.6e-3 * self.nondim().sqrt()).min(HS_FULL) * self.u() * self.u() / G
    }
    /// Peak period (s).
    pub fn tp(&self) -> f64 {
        (0.286 * self.nondim().cbrt()).min(TP_FULL) * self.u() / G
    }
}

/// The long-period swell: Hs (m), Tp (s), travel direction (rad), cos-2s spread.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Swell {
    pub hs: f64,
    pub tp: f64,
    pub dir: f64,
    pub spread: f64,
}

/// The sea at one hour and weather.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeaState {
    pub pacific: WindSea,
    pub bay: WindSea,
    pub swell: Swell,
    /// 0 calm .. 1 a storm (from the wind over the water).
    pub storm: f64,
}

fn blend_dir(a: f64, b: f64, t: f64) -> f64 {
    let (x, z) = (lerp(a.cos(), b.cos(), t), lerp(a.sin(), b.sin(), t));
    z.atan2(x)
}

/// The sea state for an hour of the day, the weather's wind (m/s, the
/// direction the air moves, x east, z south; eo-sky `WeatherState::wind`) and
/// its severity (0 clear, 1 fog, 2 drizzle, 3 rain, 4 storm).
pub fn sea_state(hour: f64, wind: [f64; 2], severity: f64) -> SeaState {
    sea_state_with(hour, wind, severity, None)
}

/// The sea state with the strait's measured wind (iteration 2): the bay's
/// breeze at least the mean wind at NOAA station 9414290 for the day and hour
/// (`climate::strait_wind`: a morning floor of 2-3 m/s, so the strait is never
/// glassy in daylight; the glassy calm is left to the lee the exposure field
/// finds) with `floor`, and in place of the `BAY_WESTERLY` 10 m/s afternoon
/// peak (about 5-6 m/s measured) with `measured_peak`.
pub fn sea_state_on(day_of_year: f64, hour: f64, wind: [f64; 2], severity: f64, floor: bool, measured_peak: bool) -> SeaState {
    if !floor && !measured_peak {
        return sea_state(hour, wind, severity);
    }
    let breeze = smooth(10.5, 15.0, hour) * (1.0 - smooth(19.5, 22.5, hour));
    let old = lerp(BAY_CALM, BAY_WESTERLY, breeze);
    let station = crate::climate::strait_wind(day_of_year, hour);
    let u = if measured_peak { station } else { old.max(station) };
    sea_state_with(hour, wind, severity, Some(u))
}

/// `sea_state` with the bay's own breeze (m/s, before the weather) given.
pub fn sea_state_with(hour: f64, wind: [f64; 2], severity: f64, bay_wind: Option<f64>) -> SeaState {
    // The summer sea breeze: calm until late morning, the westerly from
    // noon, strongest from mid-afternoon to sunset, easing in the evening.
    let breeze = smooth(10.5, 15.0, hour) * (1.0 - smooth(19.5, 22.5, hour));
    // Clear, overcast and fog days keep it; under rain the weather's wind rules.
    let clearish = 1.0 - smooth(0.6, 2.0, severity);
    let ww = wind[0].hypot(wind[1]) * OVER_WATER;
    let wdir = wind[1].atan2(wind[0]);
    let storm = smooth(9.0, 16.0, ww);
    let bay_breeze = bay_wind.unwrap_or_else(|| lerp(BAY_CALM, BAY_WESTERLY, breeze)) * clearish;
    let pac_breeze = lerp(PACIFIC_WIND[0], PACIFIC_WIND[1], breeze) * clearish + PACIFIC_WIND[2] * (1.0 - clearish);
    let take = |own: f64| smooth(0.8, 1.25, ww / own.max(0.1));
    SeaState {
        pacific: WindSea { u10: pac_breeze.max(ww), dir: blend_dir(PACIFIC_DIR, wdir, take(pac_breeze)), fetch_m: lerp(PACIFIC_FETCH_M[0], PACIFIC_FETCH_M[1], storm) },
        bay: WindSea { u10: bay_breeze.max(ww).max(0.5), dir: blend_dir(BAY_DIR, wdir, take(bay_breeze)), fetch_m: BAY_FETCH_M },
        swell: Swell {
            hs: lerp(SWELL_HS[0], SWELL_HS[1], storm),
            tp: lerp(SWELL_TP[0], SWELL_TP[1], storm),
            dir: SWELL_DIR,
            spread: lerp(SWELL_SPREAD[0], SWELL_SPREAD[1], storm),
        },
        storm,
    }
}

/// Active whitecap coverage (fraction) at U10 (Monahan and O'Muircheartaigh 1980).
pub fn active_whitecaps(u10: f64) -> f64 {
    3.84e-6 * u10.max(0.0).powf(3.41)
}

/// The visible foam coverage the plan aims for at U10 (none under a gentle
/// breeze: crests start to break at about 4 m/s), by the earlier reading.
pub fn foam_cover(u10: f64) -> f64 {
    foam_cover_over(u10, FOAM_OVER_ACTIVE)
}

/// The visible foam coverage at U10 for `over` times the fit
/// (`FOAM_OVER_TOTAL` or the earlier `FOAM_OVER_ACTIVE`).
pub fn foam_cover_over(u10: f64, over: f64) -> f64 {
    (over * active_whitecaps(u10) * smooth(3.0, 4.5, u10)).min(0.45)
}

/// One JONSWAP system: S(w) = scale w^-5 exp(-1.25 (wp/w)^4) gamma^r, times a
/// cos-2s spread around `dir` (fixed s, or the wind sea's s(w) when 0), on
/// the cascades in `mask` (bit c).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct System {
    pub hs: f64,
    pub tp: f64,
    pub dir: f64,
    pub gamma: f64,
    pub spread: f64,
    pub mask: u32,
    pub scale: f64,
}

impl System {
    fn new(hs: f64, tp: f64, dir: f64, gamma: f64, spread: f64, mask: u32) -> Self {
        let mut s = Self { hs, tp, dir, gamma, spread, mask, scale: 1.0 };
        let wp = TAU / tp.max(0.05);
        let m0 = integrate(wp * 0.3, wp * 30.0, 384, |w| s.shape(w));
        s.scale = if m0 > 0.0 { (hs / 4.0).powi(2) / m0 } else { 0.0 };
        s
    }
    pub fn omega_p(&self) -> f64 {
        TAU / self.tp.max(0.05)
    }
    fn shape(&self, w: f64) -> f64 {
        let wp = self.omega_p();
        let sigma = if w <= wp { 0.07 } else { 0.09 };
        let r = (-((w - wp).powi(2)) / (2.0 * sigma * sigma * wp * wp)).exp();
        w.powi(-5) * (-1.25 * (wp / w).powi(4)).exp() * self.gamma.powf(r)
    }
    /// S(w) (m^2 s).
    pub fn s(&self, w: f64) -> f64 {
        self.scale * self.shape(w)
    }
    /// The cos-2s exponent at w.
    pub fn spread_at(&self, w: f64) -> f64 {
        if self.spread > 0.0 {
            return self.spread;
        }
        let x = w / self.omega_p();
        (if x > 1.0 { 9.77 * x.powf(-2.5) } else { 9.77 * x.powi(5) }).max(0.3)
    }
}

/// cos-2s normalisation: 2^(2s) Gamma(s+1)^2 / (2 pi Gamma(2s+1)). The JS's
/// polynomial below 5 (exact to 1e-3), Stirling's series above.
pub fn cos2s_norm(s: f64) -> f64 {
    if s < 5.0 {
        let (s2, s3, s4) = (s * s, s * s * s, s * s * s * s);
        return -0.000564 * s4 + 0.00776 * s3 - 0.044 * s2 + 0.192 * s + 0.163;
    }
    (s / (4.0 * PI)).sqrt() * (1.0 + 1.0 / (8.0 * s))
}

/// D(theta) for a travel direction `theta` against the system's `dir`.
pub fn spread(theta: f64, dir: f64, s: f64) -> f64 {
    cos2s_norm(s) * ((theta - dir) * 0.5).cos().abs().powf(2.0 * s)
}

/// Log-spaced trapezoid of f over [a, b].
fn integrate(a: f64, b: f64, steps: usize, f: impl Fn(f64) -> f64) -> f64 {
    if b <= a {
        return 0.0;
    }
    let (la, lb) = (a.ln(), b.ln());
    let mut sum = 0.0;
    let mut prev = (a, f(a) * a);
    for i in 1..=steps {
        let w = (la + (lb - la) * i as f64 / steps as f64).exp();
        let v = f(w) * w; // d(ln w) = dw / w
        sum += 0.5 * (prev.1 + v) * ((w.ln()) - prev.0.ln());
        prev = (w, v);
    }
    sum
}

fn omega_of(k: f64) -> f64 {
    (G * k * (k * DEPTH_M).min(20.0).tanh()).sqrt()
}

fn k_of(w: f64) -> f64 {
    // Deep water (the bands' edges; DEPTH_M is deep for every band).
    w * w / G
}

/// The band of cascade c as wavenumbers [lo, hi).
pub fn band_k(c: usize) -> (f64, f64) {
    (TAU / CUT_M[c], TAU / CUT_M[c + 1])
}

/// Integrals of a system over cascade c's band: (variance, slope variance, mean k).
fn band_moments(sys: &System, c: usize) -> (f64, f64, f64) {
    let (klo, khi) = band_k(c);
    let wp = sys.omega_p();
    let (wlo, whi) = (omega_of(klo).max(wp * 0.3), omega_of(khi).min(wp * 60.0));
    // One log-spaced trapezoid for the three moments (128 steps: the plan is
    // rebuilt on the render thread when the weather moves).
    let (mut e, mut mss, mut km) = (0.0, 0.0, 0.0);
    if whi > wlo {
        let steps = 128;
        let (la, lb) = (wlo.ln(), whi.ln());
        let h = (lb - la) / steps as f64;
        for i in 0..=steps {
            let w = (la + h * i as f64).exp();
            let k = k_of(w);
            let v = sys.s(w) * (-(k * SHORT_FADE_M).powi(2)).exp() * w * h * if i == 0 || i == steps { 0.5 } else { 1.0 };
            e += v;
            mss += k * k * v;
            km += k * v;
        }
    }
    (e, mss, if e > 0.0 { km / e } else { 0.5 * (klo + khi.min(1e3)) })
}

/// Upper-tail standard normal quantile: z with P(Z > z) = p (Abramowitz and
/// Stegun 26.2.23, |error| < 4.5e-4), p in (0, 0.5].
pub fn upper_quantile(p: f64) -> f64 {
    let p = p.clamp(1e-12, 0.5);
    let t = (-2.0 * p.ln()).sqrt();
    t - (2.515517 + 0.802853 * t + 0.010328 * t * t) / (1.0 + 1.432788 * t + 0.189269 * t * t + 0.001308 * t * t * t)
}

/// Systems: 0 the swell and 1 the open Pacific's wind sea (cascade 0), 2 the
/// short sea (cascades 1-3: the bay's fetch-limited shape, which each place
/// rescales per cascade).
pub const SYSTEMS: usize = 3;

/// What the GPU builds for a sea state.
#[derive(Clone, Debug)]
pub struct Plan {
    pub state: SeaState,
    pub systems: [System; SYSTEMS],
    /// The lead swell train: amplitude (m) and phase (rad).
    pub lead_amp: f64,
    pub lead_phase: f64,
    /// Variance per cascade: what the GPU field carries, and what the open
    /// Pacific and the bay should carry there.
    pub e_gpu: [f64; CASCADES],
    pub e_pac: [f64; CASCADES],
    pub e_bay: [f64; CASCADES],
    /// Amplitude multipliers per cascade (the field times these is each place's sea).
    pub amp_pac: [f64; CASCADES],
    pub amp_bay: [f64; CASCADES],
    /// Slope variance per cascade of the GPU field.
    pub mss_gpu: [f64; CASCADES],
    /// Signed choppiness (negative: crests sharpen).
    pub chop: f64,
    /// Visible foam coverage aimed for: Pacific, bay.
    pub cover: [f64; 2],
    /// Jacobian thresholds and softness per place (0 Pacific, 1 bay) and cascade.
    pub thr: [[f64; CASCADES]; 2],
    pub soft: [[f64; CASCADES]; 2],
    /// Foam lifetime (s) per place.
    pub foam_tau: [f64; 2],
    /// (iteration 2, exposure) the bay's amplitude per cascade where the wind
    /// is `EXPO_WIND_NODES[g]` of the strait's and the fetch factor
    /// `EXPO_FETCH_NODES[b]`: `[g][b][c]`. Each place's wind sea grows by the
    /// fetch law (`WindSea::hs`, `tp`) at its own wind and fetch, so a short
    /// fetch takes the long waves and moves the peak to shorter ones, while the
    /// short waves above the peak keep their level (in the JONSWAP fit a young
    /// sea's tail is, if anything, higher: alpha = 0.076 (g F / U^2)^-0.22,
    /// Hasselmann et al. 1973), and a lee's lighter wind lowers them all.
    pub expo_amp: [[[f64; CASCADES]; EXPO_FETCH_NODES.len()]; EXPO_WIND_NODES.len()],
}

impl Plan {
    /// The plan with the earlier foam reading (`FOAM_OVER_ACTIVE`).
    pub fn new(state: &SeaState) -> Self {
        Self::with_foam(state, FOAM_OVER_ACTIVE)
    }

    /// The plan whose whitecaps cover `foam_over` times the Monahan and
    /// O'Muircheartaigh fit (`FOAM_OVER_TOTAL`: the fit itself).
    pub fn with_foam(state: &SeaState, foam_over: f64) -> Self {
        let st = *state;
        let sw = &st.swell;
        let lead_amp = (2.0 * LEAD_SHARE).sqrt() * sw.hs / 4.0;
        let swell = System::new(sw.hs * (1.0 - LEAD_SHARE).sqrt(), sw.tp, sw.dir, 5.0, sw.spread, 0b0001);
        let pac = System::new(st.pacific.hs(), st.pacific.tp(), st.pacific.dir, 3.3, 0.0, 0b0001);
        // The short sea's shape: the bay's at the stronger of the two winds (at least 3 m/s, so a calm bay
        // under a breezy Pacific still has a shape to scale).
        let u_shape = st.bay.u10.max(0.7 * st.pacific.u10).max(3.0);
        let shape_sea = WindSea { u10: u_shape, dir: if st.bay.u10 >= 0.7 * st.pacific.u10 { st.bay.dir } else { blend_dir(st.bay.dir, st.pacific.dir, 0.5) }, fetch_m: BAY_FETCH_M };
        let short = System::new(shape_sea.hs(), shape_sea.tp(), shape_sea.dir, 3.3, 0.0, 0b1110);
        let bay = System::new(st.bay.hs(), st.bay.tp(), st.bay.dir, 3.3, 0.0, 0b1111);
        let pac_all = System { mask: 0b1111, ..pac };
        let lead_var = lead_amp * lead_amp / 2.0;
        let mut p = Plan {
            state: st,
            systems: [swell, pac, short],
            lead_amp,
            lead_phase: LEAD_PHASE,
            e_gpu: [0.0; CASCADES],
            e_pac: [0.0; CASCADES],
            e_bay: [0.0; CASCADES],
            amp_pac: [1.0; CASCADES],
            amp_bay: [1.0; CASCADES],
            mss_gpu: [0.0; CASCADES],
            chop: -(CHOP + CHOP_STORM * st.storm),
            cover: [foam_cover_over(st.pacific.u10, foam_over), foam_cover_over(st.bay.u10, foam_over)],
            thr: [[-1e3; CASCADES]; 2],
            soft: [[1.0; CASCADES]; 2],
            foam_tau: [lerp(FOAM_TAU_S[0], FOAM_TAU_S[1], smooth(9.0, 16.0, st.pacific.u10)), lerp(FOAM_TAU_S[0], FOAM_TAU_S[1], smooth(9.0, 16.0, st.bay.u10))],
            expo_amp: [[[1.0; CASCADES]; EXPO_FETCH_NODES.len()]; EXPO_WIND_NODES.len()],
        };
        let lead_k = TAU / SIZES[0] * ((LEAD[0] * LEAD[0] + LEAD[1] * LEAD[1]) as f64).sqrt();
        let mut kc = [[0.0; CASCADES]; 2];
        for c in 0..CASCADES {
            let sw_m = band_moments(&swell, c);
            let pa_m = band_moments(&pac_all, c);
            let ba_m = band_moments(&bay, c);
            let lead = if c == 0 { lead_var } else { 0.0 };
            let (gpu_e, gpu_mss) = if c == 0 { (sw_m.0 + pa_m.0 + lead, sw_m.1 + pa_m.1 + lead * lead_k * lead_k) } else { let s = band_moments(&short, c); (s.0, s.1) };
            p.e_gpu[c] = gpu_e;
            p.mss_gpu[c] = gpu_mss;
            p.e_pac[c] = sw_m.0 + lead + pa_m.0;
            p.e_bay[c] = ba_m.0 + SWELL_LEAK_BAY * SWELL_LEAK_BAY * (sw_m.0 + lead);
            kc[0][c] = if p.e_pac[c] > 0.0 { (sw_m.0 * sw_m.2 + lead * lead_k + pa_m.0 * pa_m.2) / p.e_pac[c] } else { 0.0 };
            kc[1][c] = ba_m.2;
            if gpu_e > 1e-14 {
                p.amp_pac[c] = (p.e_pac[c] / gpu_e).sqrt().min(1.4);
                p.amp_bay[c] = (p.e_bay[c] / gpu_e).sqrt().min(1.4);
            }
        }
        // (exposure) the bay's wind sea at each node's wind and fetch, over the same GPU field
        for (g, &gk) in EXPO_WIND_NODES.iter().enumerate() {
            for (b, &bk) in EXPO_FETCH_NODES.iter().enumerate() {
                let ws = WindSea { u10: st.bay.u10 * gk, dir: st.bay.dir, fetch_m: BAY_FETCH_M * bk * bk };
                let sys = System::new(ws.hs(), ws.tp(), ws.dir, 3.3, 0.0, 0b1111);
                for c in 1..CASCADES {
                    let leak = p.e_bay[c] - band_moments(&bay, c).0;
                    let e = band_moments(&sys, c).0 + leak.max(0.0);
                    p.expo_amp[g][b][c] = if p.e_gpu[c] > 1e-14 { (e / p.e_gpu[c]).sqrt().min(EXPO_AMP_CAP) } else { 1.0 };
                }
                p.expo_amp[g][b][0] = p.amp_bay[0];
            }
        }
        // Whitecaps: each place's foam target shared over the breaking
        // cascades by their steepness (amplitude x wavenumber), each turned
        // into a threshold on that cascade's Jacobian (Gaussian to first order:
        // J - 1 ~ chop x amp x divergence, whose deviation is sqrt(mss)).
        for place in 0..2 {
            let (e, amp) = if place == 0 { (&p.e_pac, &p.amp_pac) } else { (&p.e_bay, &p.amp_bay) };
            // (a cascade the place keeps less than a fifth of, as the bay its swell, does not break)
            let steep: Vec<f64> = (0..WHITECAP_CASCADES).map(|c| if amp[c] < 0.2 { 0.0 } else { e[c].max(0.0).sqrt() * kc[place][c] }).collect();
            let total: f64 = steep.iter().sum();
            let trail = (p.foam_tau[place] / 3.3).powf(0.5);
            for c in 0..WHITECAP_CASCADES {
                let sigma = p.chop.abs() * amp[c] * p.mss_gpu[c].sqrt();
                let share = if total > 0.0 { steep[c] / total } else { 0.0 };
                let inst = (p.cover[place] * share / (K_TRAIL[c] * trail)).min(0.3);
                if inst > 1e-5 && sigma > 1e-6 {
                    p.thr[place][c] = 1.0 - sigma * Z_TAIL * upper_quantile(inst);
                    p.soft[place][c] = (0.12 * sigma).max(1e-4); // (a crest that breaks is white at once)
                }
            }
        }
        p
    }

    /// (exposure) `expo_amp` at wind ratio `g` and fetch factor `b`, bilinear
    /// between the nodes and held to their ends (water_common.wgsl `wg_bay_amp`).
    pub fn expo_amp_at(&self, g: f64, b: f64) -> [f64; CASCADES] {
        fn node(v: f64, n: &[f64; 8]) -> (usize, f64) {
            let x = v.clamp(n[0], n[7]);
            let i = (1..7).filter(|k| x > n[*k]).count();
            (i, ((x - n[i]) / (n[i + 1] - n[i]).max(1e-6)).clamp(0.0, 1.0))
        }
        let (gi, gt) = node(g, &EXPO_WIND_NODES);
        let (bi, bt) = node(b, &EXPO_FETCH_NODES);
        let a = &self.expo_amp;
        std::array::from_fn(|c| {
            let lo = a[gi][bi][c] + (a[gi][bi + 1][c] - a[gi][bi][c]) * bt;
            let hi = a[gi + 1][bi][c] + (a[gi + 1][bi + 1][c] - a[gi + 1][bi][c]) * bt;
            lo + (hi - lo) * gt
        })
    }

    /// Significant height (m) a place carries (amplitudes `amp`) per the plan's band integrals.
    pub fn place_hs(&self, pacific: bool) -> f64 {
        let e = if pacific { &self.e_pac } else { &self.e_bay };
        4.0 * e.iter().sum::<f64>().sqrt()
    }
}

/// The per-texel normal draws (four per texel: the -k draw's re, im, the +k
/// draw's re, im), from `SEED` (PCG32, Box-Muller).
pub fn gaussians() -> Vec<[f32; 4]> {
    let total = N * N * CASCADES;
    let mut state: u64 = 0x853c_49e6_748f_ea9b ^ SEED as u64;
    let inc: u64 = 0xda3e_39cb_94b9_5bdb | 1;
    let mut next = || {
        let old = state;
        state = old.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(inc);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 59) as u32;
        xorshifted.rotate_right(rot)
    };
    let mut unit = || ((next() >> 8) as f64 + 0.5) / 16_777_216.0;
    let mut out = Vec::with_capacity(total);
    for _ in 0..total {
        let mut g = [0f32; 4];
        for pair in 0..2 {
            let (u1, u2) = (unit(), unit());
            let r = (-2.0 * u1.ln()).sqrt();
            g[pair * 2] = (r * (TAU * u2).cos()) as f32;
            g[pair * 2 + 1] = (r * (TAU * u2).sin()) as f32;
        }
        out.push(g);
    }
    out
}

/// The lead mode's texel index (cascade 0).
pub fn lead_index() -> usize {
    let half = (N / 2) as i32;
    ((half + LEAD[1]) as usize) * N + (half + LEAD[0]) as usize
}

/// h0 at one texel (cascade c, row m, column n): the GPU's `spectrum_init`.
pub fn texel_h0(plan: &Plan, g: [f32; 4], c: usize, m: usize, n: usize) -> [f64; 4] {
    if c == 0 && m * N + n == lead_index() {
        return [0.0, 0.0, plan.lead_amp * plan.lead_phase.cos(), plan.lead_amp * plan.lead_phase.sin()];
    }
    let half = (N / 2) as f64;
    let size = SIZES[c];
    let kx = (n as f64 - half) * TAU / size;
    let kz = (m as f64 - half) * TAU / size;
    let k = kx.hypot(kz);
    let (klo, khi) = band_k(c);
    if k < 1e-6 || k < klo || k >= khi {
        return [0.0; 4];
    }
    let kd = (k * DEPTH_M).min(20.0);
    let w = (G * k * kd.tanh()).sqrt();
    let dwdk = G * (kd.tanh() + kd * (1.0 - kd.tanh().powi(2))) / (2.0 * w);
    let theta = kz.atan2(kx);
    let fade = (-(k * SHORT_FADE_M).powi(2)).exp();
    let mut eb = 0.0;
    for s in &plan.systems {
        if s.mask & (1 << c) == 0 {
            continue;
        }
        eb += s.s(w) * fade * spread(theta, s.dir, s.spread_at(w));
    }
    let dk = TAU / size;
    let jac = dwdk / k * dk * dk;
    let ab = (2.0 * eb * jac).sqrt();
    let r = std::f64::consts::FRAC_1_SQRT_2;
    [0.0, 0.0, g[2] as f64 * ab * r, g[3] as f64 * ab * r]
}

/// The CPU twin of the GPU's spectrum for a plan (tests; the renderer builds it on the GPU).
pub fn build(plan: &Plan, gauss: &[[f32; 4]]) -> Spectrum {
    let total = N * N * CASCADES;
    let mut h0 = vec![[0f32; 4]; total];
    let mut wave = vec![[0f32; 4]; total];
    let half = (N / 2) as f64;
    for c in 0..CASCADES {
        for m in 0..N {
            for n in 0..N {
                let i = c * N * N + m * N + n;
                let kx = (n as f64 - half) * TAU / SIZES[c];
                let kz = (m as f64 - half) * TAU / SIZES[c];
                let k = kx.hypot(kz);
                wave[i] = [kx as f32, kz as f32, k as f32, omega_of(k) as f32];
                let h = texel_h0(plan, gauss[i], c, m, n);
                h0[i] = [h[0] as f32, h[1] as f32, h[2] as f32, h[3] as f32];
            }
        }
    }
    let hs = field_hs(&h0, &[1.0; CASCADES]);
    Spectrum { h0, wave, significant_height: hs, raw_significant_height: hs, scale: 1.0, sizes: SIZES }
}

/// Significant height (m) of the real field the transform gives: 4 sqrt of
/// its variance, sum over modes of (|A|^2 + |B|^2) / 2, cascade c times amp[c].
pub fn field_hs(h0: &[[f32; 4]], amp: &[f64; CASCADES]) -> f64 {
    let mut v = 0.0;
    for (i, t) in h0.iter().enumerate() {
        let a = amp[i / (N * N)];
        v += a * a * t.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / 2.0;
    }
    4.0 * v.sqrt()
}

/// Inputs quantised so the plan is rebuilt only when they move (5 minutes of
/// the hour, 0.5 m/s of wind, 0.1 of severity): a rebuild is about 0.1 ms of
/// CPU (off the render thread outside captures) and one spectrum dispatch.
pub fn quantise(hour: f64, wind: [f64; 2], severity: f64) -> [i64; 4] {
    [(hour * 12.0).round() as i64, (wind[0] * 2.0).round() as i64, (wind[1] * 2.0).round() as i64, (severity * 10.0).round() as i64]
}

/// The inputs a quantised key stands for.
pub fn unquantise(key: [i64; 4]) -> (f64, [f64; 2], f64) {
    (key[0] as f64 / 12.0, [key[1] as f64 / 2.0, key[2] as f64 / 2.0], key[3] as f64 / 10.0)
}

/// The GPU's spectrum parameters (`water_spectrum.wgsl` `SpecParams`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct SpecParamsRaw {
    pub sizes: [f32; 4],
    pub k_lo: [f32; 4],
    pub k_hi: [f32; 4],
    /// Per system: (scale, omega_p, ln gamma, dir), (fixed s or 0, cascade mask, 0, 0).
    pub sa: [[f32; 4]; SYSTEMS],
    pub sb: [[f32; 4]; SYSTEMS],
    /// x lead texel (cascade 0), y amplitude, z phase, w short fade (m).
    pub lead: [f32; 4],
    /// x depth (m).
    pub misc: [f32; 4],
}

impl Plan {
    pub fn spec_params(&self) -> SpecParamsRaw {
        let mut p = SpecParamsRaw::default();
        for c in 0..CASCADES {
            let (lo, hi) = band_k(c);
            p.sizes[c] = SIZES[c] as f32;
            p.k_lo[c] = lo as f32;
            p.k_hi[c] = hi.min(1e9) as f32;
        }
        for (i, s) in self.systems.iter().enumerate() {
            p.sa[i] = [s.scale as f32, s.omega_p() as f32, s.gamma.ln() as f32, s.dir as f32];
            p.sb[i] = [s.spread as f32, s.mask as f32, 0.0, 0.0];
        }
        p.lead = [lead_index() as f32, self.lead_amp as f32, self.lead_phase as f32, SHORT_FADE_M as f32];
        p.misc = [DEPTH_M as f32, 0.0, 0.0, 0.0];
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clear_afternoon() -> SeaState {
        sea_state(16.0, [0.0, 0.0], 0.0)
    }

    #[test]
    fn fetch_laws_give_the_brief_targets() {
        // The afternoon westerly over the bay: 0.4-0.7 m, about 3 s.
        let b = clear_afternoon().bay;
        assert!((b.u10 - BAY_WESTERLY).abs() < 1e-9);
        assert!((0.45..0.75).contains(&b.hs()), "bay afternoon Hs {}", b.hs());
        assert!((2.8..3.8).contains(&b.tp()), "bay afternoon Tp {}", b.tp());
        // A glassy morning: under 5 cm.
        let m = sea_state(7.5, [0.0, 0.0], 0.0);
        assert!(m.bay.hs() < 0.05, "bay morning Hs {}", m.bay.hs());
        // The storm preset's wind (11.5, 4.2) over the bay: 0.9-1.3 m, about 4 s; the
        // Pacific 3-6 m all told, its wind sea 8-11 s.
        let s = sea_state(15.0, [11.5, 4.2], 4.0);
        assert!(s.storm > 0.99);
        assert!((0.9..1.35).contains(&s.bay.hs()), "bay storm Hs {}", s.bay.hs());
        assert!((3.5..4.6).contains(&s.bay.tp()), "bay storm Tp {}", s.bay.tp());
        let pac = (s.pacific.hs().powi(2) + s.swell.hs.powi(2)).sqrt();
        assert!((3.0..6.0).contains(&pac), "Pacific storm Hs {pac}");
        assert!((8.0..11.5).contains(&s.pacific.tp()), "Pacific storm wind sea Tp {}", s.pacific.tp());
        // The Pacific on a clear afternoon: swell 1.5-3 m at 12-16 s under its wind sea.
        let c = clear_afternoon();
        assert!((1.5..3.0).contains(&c.swell.hs) && (12.0..16.0).contains(&c.swell.tp));
        // Whitecaps: a few tenths of a percent active at 15 kt, several percent in a gale.
        assert!((0.002..0.006).contains(&active_whitecaps(7.7)));
        assert!((0.04..0.1).contains(&active_whitecaps(17.0)));
    }

    // (iteration 2) the strait at the measured wind: never glassy by day, and
    // the afternoon at the station's 5-6 m/s, not the 10 m/s westerly
    #[test]
    fn the_strait_has_its_measured_wind() {
        let m = sea_state_on(265.0, 10.0, [0.0, 0.0], 0.0, true, false);
        assert!(m.bay.u10 > 2.0, "a morning strait at {} m/s", m.bay.u10);
        assert!(m.bay.hs() > 0.08, "a morning chop of {} m", m.bay.hs());
        let a = sea_state_on(265.0, 16.4, [0.0, 0.0], 0.0, true, true);
        assert!((4.5..6.5).contains(&a.bay.u10), "afternoon {}", a.bay.u10);
        // the measured afternoon breaks few crests: well under a percent of whitecaps
        assert!(foam_cover_over(a.bay.u10, FOAM_OVER_TOTAL) < 0.003);
        // the floor alone keeps the earlier afternoon westerly
        let f = sea_state_on(265.0, 16.4, [0.0, 0.0], 0.0, true, false);
        assert!(f.bay.u10 > 9.0);
        // both off: the earlier sea state exactly
        assert_eq!(sea_state_on(265.0, 10.0, [1.0, 2.0], 0.0, false, false), sea_state(10.0, [1.0, 2.0], 0.0));
    }

    #[test]
    fn plan_scales_each_place() {
        let p = Plan::new(&clear_afternoon());
        let pac = p.place_hs(true);
        let want = (p.state.swell.hs.powi(2) + p.state.pacific.hs().powi(2)).sqrt();
        assert!((pac / want - 1.0).abs() < 0.08, "Pacific Hs {pac} want {want}");
        let bay = p.place_hs(false);
        assert!((bay / p.state.bay.hs() - 1.0).abs() < 0.1, "bay Hs {bay} want {}", p.state.bay.hs());
        // The bay is sheltered: almost none of cascade 0, all of its own sea in cascade 1.
        assert!(p.amp_bay[0] < 0.08, "{:?}", p.amp_bay);
        assert!(p.amp_bay[1] > 0.9, "{:?}", p.amp_bay);
        // Whitecap thresholds exist where the wind breaks waves.
        assert!(p.thr[1][1] > -10.0 && p.thr[1][1] < 1.0, "{:?}", p.thr);
        let calm = Plan::new(&sea_state(7.5, [0.0, 0.0], 0.0));
        assert!(calm.thr[1].iter().all(|t| *t < -10.0), "a calm bay has no whitecaps {:?}", calm.thr);
    }

    // (iteration 2, exposure) a short fetch takes the long wind sea, not the
    // ripples: off Crissy Field (fetch factor about 0.37 at the station's own
    // wind) the ripples keep their height while the long cascade loses most of it
    #[test]
    fn a_short_fetch_takes_the_long_waves_not_the_ripples() {
        for (day, hour) in [(265.0, 13.0), (265.0, 17.0), (265.0, 10.0)] {
            let p = Plan::new(&sea_state_on(day, hour, [0.0, 0.0], 0.0, true, true));
            // the plan's own wind and fetch (ratio 1, factor 1, both nodes) is the plan's bay
            let own = p.expo_amp_at(1.0, 1.0);
            for c in 0..CASCADES {
                assert!((own[c].min(1.4) - p.amp_bay[c]).abs() < 1e-6, "{c}: {own:?} vs {:?}", p.amp_bay);
            }
            // between the nodes the table reads within 10 % of the fetch law's own (and 0.03 of the GPU field's amplitude)
            for (g, b) in [(0.6, 0.3), (0.92, 0.5), (1.4, 0.8), (1.1, 0.4)] {
                let p2 = WindSea { u10: p.state.bay.u10 * g, dir: p.state.bay.dir, fetch_m: BAY_FETCH_M * b * b };
                let sys = System::new(p2.hs(), p2.tp(), p2.dir, 3.3, 0.0, 0b1111);
                let got = p.expo_amp_at(g, b);
                for c in 1..CASCADES {
                    let e = band_moments(&sys, c).0 + SWELL_LEAK_BAY * SWELL_LEAK_BAY * band_moments(&p.systems[0], c).0; // (and the swell's leak)
                    let want = (e / p.e_gpu[c]).sqrt().min(EXPO_AMP_CAP);
                    assert!((got[c] - want).abs() <= 0.1 * want + 0.03, "{hour} h, g {g} b {b}, cascade {c}: {} want {want}", got[c]);
                }
            }
            let crissy = p.expo_amp_at(1.0, 0.37);
            let ratio = [1, 2, 3].map(|c| crissy[c] / own[c].max(1e-9));
            // the ripples (cascade 3) keep over three quarters (a single factor on every cascade gave 0.37)
            assert!(ratio[2] > 0.75, "{hour} h: ripples {ratio:?}");
            // the long wind sea (cascade 1) loses more than the ripples do
            assert!(ratio[0] < ratio[2], "{hour} h: {ratio:?}");
            // a calmer place has lower waves in every cascade at the same fetch, a windier one higher
            let (calm, windy) = (p.expo_amp_at(0.5, 1.0), p.expo_amp_at(1.7, 1.0));
            for c in 1..CASCADES {
                assert!(calm[c] <= own[c] + 1e-9 && windy[c] + 1e-9 >= own[c].min(EXPO_AMP_CAP), "{c}: {calm:?} {own:?} {windy:?}");
            }
        }
    }
}
