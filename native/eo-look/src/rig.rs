//! The light rig `CityRenderer.setTimeOfDay` applies (src/citygen/renderer.js,
//! the environment branch), as the JS computes it, sampled on the JS's own
//! 0.25 h environment bucket by the earlier browser version's exporter and
//! interpolated here. The solver behind it (`keyFillBalance`,
//! `recommendedLightRig`, the analytic probe yield) is not re-derived in Rust.
//!
//! Also ported (pure, tested against the JS): `recommendedExposure` with its
//! `sceneIlluminance`, `directBeamTransmittance` and `keyEnvelope`.

use crate::math::*;
use crate::sky::SkyModel;
use crate::sun::{sun_direction, wrap_hour};
use serde::Deserialize;
use std::sync::OnceLock;

pub const ORACLE_JSON: &str = include_str!("../data/look-oracle.json");

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RigRow {
    pub hour: f64,
    pub sun_altitude_deg: f64,
    pub daylight: f64,
    pub sun_color: Rgb,
    pub sun_intensity: f64,
    pub hemi_sky: Rgb,
    pub hemi_ground: Rgb,
    pub hemi_intensity: f64,
    pub ambient_color: Rgb,
    pub ambient_intensity: f64,
    pub rim_color: Rgb,
    pub rim_intensity: f64,
    pub environment_intensity: f64,
    pub env_map_intensity: std::collections::BTreeMap<String, f64>,
    pub exposure: f64,
    pub light_rig_sun_scale: f64,
    pub shadow_intensity: f64,
    pub key_envelope: f64,
    pub ground_irradiance: Rgb,
    /// The diffuse yield of the probe this row was solved for (`keyFillBalance`'s
    /// `environmentDiffuseYield`), and the analytic prefilter's at that hour.
    #[serde(default = "one")]
    pub environment_diffuse_yield: f64,
    #[serde(default = "one")]
    pub analytic_diffuse_yield: f64,
}

fn one() -> f64 {
    1.0
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OracleRig {
    rig: Vec<RigRow>,
    #[serde(default)]
    rig_baked: Option<Vec<RigRow>>,
}

fn oracle_rig() -> &'static OracleRig {
    static T: OnceLock<OracleRig> = OnceLock::new();
    T.get_or_init(|| serde_json::from_str::<OracleRig>(ORACLE_JSON).expect("look-oracle.json"))
}

/// The rig the running game solves (surfaces): with its street-level scene
/// bake bound as `scene.environment`, whose diffuse yield the JS publishes
/// (`bakedProbeDiffuseYield`; without a readback `ENVIRONMENT_DIFFUSE_YIELD`,
/// 0.038). `keyFillBalance` then moves the environment's shortfall into the
/// hemisphere and ambient fill. The analytic-prefilter rig (`table_analytic`)
/// is what the JS draws only on frames with no bake bound.
pub fn table() -> &'static [RigRow] {
    let o = oracle_rig();
    o.rig_baked.as_deref().unwrap_or(&o.rig)
}

/// The rig solved with the analytic prefilter bound (boot frames, no bake).
pub fn table_analytic() -> &'static [RigRow] {
    &oracle_rig().rig
}

/// The rig at an hour: the solved quantities, linear between the bracketing
/// rows (intensities and colours); the key's direction and envelope are
/// re-evaluated at the exact hour by the caller.
#[derive(Clone, Copy, Debug, Default)]
pub struct Rig {
    pub sun_color: Rgb,
    pub sun_intensity: f64,
    pub hemi_sky: Rgb,
    pub hemi_ground: Rgb,
    pub hemi_intensity: f64,
    pub ambient_color: Rgb,
    pub ambient_intensity: f64,
    pub rim_color: Rgb,
    pub rim_intensity: f64,
    pub environment_intensity: f64,
    /// envMapIntensity of the classes the native materials map onto:
    /// facade-painted (walls), facade-glass, asphalt (roads), sidewalk (terrain).
    pub env_painted: f64,
    pub env_glass: f64,
    pub env_asphalt: f64,
    pub env_ground: f64,
    pub light_rig_sun_scale: f64,
    pub shadow_intensity: f64,
    /// The exposure the JS solved at the bracketing rows (for comparison; the
    /// renderer uses `recommended_exposure`).
    pub table_exposure: f64,
    /// what the bound probe's diffuse delivers against the analytic
    /// sky's (yield / analytic yield): the native renderer evaluates the
    /// analytic sky's irradiance and scales it by this, so its diffuse
    /// environment delivers what the JS's bound probe delivers.
    pub env_diffuse_scale: f64,
}

/// the rig the JS solves with a scene bake whose transfer was MEASURED
/// (`bakedProbeDiffuseYield(model, transfer)`), exported on a grid of transfers
/// x 0.25 h buckets (data/rig-by-transfer.json, export-look-oracle.mjs
/// --transfer-grid). The yield-dependent quantities (sun, hemisphere and
/// ambient intensities, rim, environment intensity, the hemisphere's ground
/// colour) are bilinear in (hour, transfer); everything else is `rig_at`'s.
/// `env_diffuse_scale` becomes the transfer itself (yield / analytic yield).
pub fn rig_at_transfer(hour: f64, transfer: f64) -> Rig {
    let g = transfer_grid();
    let mut r = rig_at(hour);
    let h = wrap_hour(hour);
    let i = ((h / 0.25).floor() as usize).min(g.rows.len() - 2);
    let wh = clamp((h - g.rows[i][0][0]) / 0.25, 0.0, 1.0);
    let t = transfer.clamp(g.transfers[0], *g.transfers.last().unwrap());
    let j = g.transfers.iter().rposition(|v| *v <= t).unwrap_or(0).min(g.transfers.len() - 2);
    let wt = clamp((t - g.transfers[j]) / (g.transfers[j + 1] - g.transfers[j]).max(1e-9), 0.0, 1.0);
    let at = |c: usize| {
        let a = mix(g.rows[i][j][c], g.rows[i][j + 1][c], wt);
        let b = mix(g.rows[i + 1][j][c], g.rows[i + 1][j + 1][c], wt);
        mix(a, b, wh)
    };
    r.sun_intensity = at(3);
    r.hemi_intensity = at(4);
    r.ambient_intensity = at(5);
    r.rim_intensity = at(6);
    r.environment_intensity = at(7);
    r.hemi_ground = [at(8), at(9), at(10)];
    r.env_diffuse_scale = t;
    r
}

struct TransferGrid {
    transfers: Vec<f64>,
    /// [hour bucket][transfer] -> the exported columns.
    rows: Vec<Vec<Vec<f64>>>,
}

fn transfer_grid() -> &'static TransferGrid {
    static G: OnceLock<TransferGrid> = OnceLock::new();
    G.get_or_init(|| {
        let v: serde_json::Value = serde_json::from_str(include_str!("../data/rig-by-transfer.json")).expect("rig-by-transfer.json");
        let nums = |x: &serde_json::Value| x.as_array().map(|a| a.iter().map(|n| n.as_f64().unwrap_or(0.0)).collect::<Vec<f64>>()).unwrap_or_default();
        TransferGrid {
            transfers: nums(&v["transfers"]),
            rows: v["rows"].as_array().unwrap().iter().map(|h| h.as_array().unwrap().iter().map(nums).collect()).collect(),
        }
    })
}

pub fn rig_at(hour: f64) -> Rig {
    let t = table();
    let h = wrap_hour(hour);
    let i = ((h / 0.25).floor() as usize).min(t.len() - 2);
    let (a, b) = (&t[i], &t[i + 1]);
    let w = clamp((h - a.hour) / (b.hour - a.hour), 0.0, 1.0);
    let m = |x: f64, y: f64| mix(x, y, w);
    let env = |r: &RigRow, k: &str| r.env_map_intensity.get(k).copied().unwrap_or(1.0);
    Rig {
        sun_color: mix3(a.sun_color, b.sun_color, w),
        sun_intensity: m(a.sun_intensity, b.sun_intensity),
        hemi_sky: mix3(a.hemi_sky, b.hemi_sky, w),
        hemi_ground: mix3(a.hemi_ground, b.hemi_ground, w),
        hemi_intensity: m(a.hemi_intensity, b.hemi_intensity),
        ambient_color: mix3(a.ambient_color, b.ambient_color, w),
        ambient_intensity: m(a.ambient_intensity, b.ambient_intensity),
        rim_color: mix3(a.rim_color, b.rim_color, w),
        rim_intensity: m(a.rim_intensity, b.rim_intensity),
        environment_intensity: m(a.environment_intensity, b.environment_intensity),
        env_painted: m(env(a, "facade-painted"), env(b, "facade-painted")),
        env_glass: m(env(a, "facade-glass"), env(b, "facade-glass")),
        env_asphalt: m(env(a, "asphalt"), env(b, "asphalt")),
        env_ground: m(env(a, "sidewalk"), env(b, "sidewalk")),
        light_rig_sun_scale: m(a.light_rig_sun_scale, b.light_rig_sun_scale),
        shadow_intensity: m(a.shadow_intensity, b.shadow_intensity),
        table_exposure: m(a.exposure, b.exposure),
        env_diffuse_scale: {
            let s = |r: &RigRow| if r.analytic_diffuse_yield > 1e-9 { (r.environment_diffuse_yield / r.analytic_diffuse_yield).clamp(0.0, 1.0) } else { 1.0 };
            m(s(a), s(b))
        },
    }
}

/// How the rig's quarter-hour rows are interpolated between them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Interp {
    /// Linear between the bracketing rows: the JS, and every default.
    #[default]
    Linear,
    /// A monotone cubic through the same rows (`--smooth-time`): every row is
    /// still met exactly, and the slope no longer jumps at each quarter hour
    /// (a drawn hour that follows the clock showed the jump as a change of
    /// pace in the light every 15 minutes).
    MonotoneCubic,
}

/// The tangent at a row of a monotone cubic through evenly spaced values,
/// from the secants before and after it (per row step): 0 at an extremum or
/// a flat neighbour, else the mean secant held to 3 x the smaller one. With
/// both tangents of an interval within 3 x its secant and of its sign, the
/// Hermite cubic is monotone on it (Fritsch & Carlson, SIAM J. Numer. Anal.
/// 17(2), 1980, the square 0 <= alpha, beta <= 3 inside their region).
pub fn monotone_tangent(d_prev: f64, d_next: f64) -> f64 {
    if d_prev * d_next <= 0.0 {
        return 0.0;
    }
    let m = 0.5 * (d_prev + d_next);
    let cap = 3.0 * d_prev.abs().min(d_next.abs());
    m.signum() * m.abs().min(cap)
}

/// The monotone cubic through `y[1]` (t = 0) and `y[2]` (t = 1) of four
/// evenly spaced rows `y[0..4]`; exactly `y[1]` at 0 and `y[2]` at 1.
pub fn monotone_cubic(y: [f64; 4], t: f64) -> f64 {
    let d = [y[1] - y[0], y[2] - y[1], y[3] - y[2]];
    let (m1, m2) = (monotone_tangent(d[0], d[1]), monotone_tangent(d[1], d[2]));
    let (t2, t3) = (t * t, t * t * t);
    let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
    let h10 = t3 - 2.0 * t2 + t;
    let h01 = -2.0 * t3 + 3.0 * t2;
    let h11 = t3 - t2;
    h00 * y[1] + h10 * m1 + h01 * y[2] + h11 * m2
}

/// The rows around an hour for a cubic: (row before, row, row after, the one
/// after that, weight within the pair). The table runs 0..24 h; it wraps
/// (row n-1 is 24 h, row 0's neighbour before is 23.75 h).
pub fn cubic_rows(n: usize, hours: impl Fn(usize) -> f64, hour: f64) -> ([usize; 4], f64) {
    let h = wrap_hour(hour);
    let i = ((h / 0.25).floor() as usize).min(n - 2);
    let w = clamp((h - hours(i)) / (hours(i + 1) - hours(i)), 0.0, 1.0);
    let periodic = (hours(n - 1) - hours(0) - 24.0).abs() < 1e-9;
    let prev = match (i, periodic) {
        (0, true) => n - 2,
        (0, false) => 0,
        _ => i - 1,
    };
    let next = match (i + 2 >= n, periodic) {
        (true, true) => 1,
        (true, false) => n - 1,
        _ => i + 2,
    };
    ([prev, i, i + 1, next], w)
}

/// Scalars of a `Rig`, in `pack` order.
pub const RIG_VALUES: usize = 28;

fn row_values(r: &RigRow) -> [f64; RIG_VALUES] {
    let env = |k: &str| r.env_map_intensity.get(k).copied().unwrap_or(1.0);
    let s = if r.analytic_diffuse_yield > 1e-9 { (r.environment_diffuse_yield / r.analytic_diffuse_yield).clamp(0.0, 1.0) } else { 1.0 };
    let [sc, hs, hg, ac, rc] = [r.sun_color, r.hemi_sky, r.hemi_ground, r.ambient_color, r.rim_color];
    [
        sc[0], sc[1], sc[2], r.sun_intensity, hs[0], hs[1], hs[2], hg[0], hg[1], hg[2], r.hemi_intensity, ac[0], ac[1], ac[2], r.ambient_intensity, rc[0], rc[1], rc[2], r.rim_intensity, r.environment_intensity,
        env("facade-painted"), env("facade-glass"), env("asphalt"), env("sidewalk"), r.light_rig_sun_scale, r.shadow_intensity, r.exposure, s,
    ]
}

impl Rig {
    /// Every scalar of the rig, in a fixed order (`unpack` reverses it).
    pub fn pack(&self) -> [f64; RIG_VALUES] {
        let [sc, hs, hg, ac, rc] = [self.sun_color, self.hemi_sky, self.hemi_ground, self.ambient_color, self.rim_color];
        [
            sc[0], sc[1], sc[2], self.sun_intensity, hs[0], hs[1], hs[2], hg[0], hg[1], hg[2], self.hemi_intensity, ac[0], ac[1], ac[2], self.ambient_intensity, rc[0], rc[1], rc[2], self.rim_intensity,
            self.environment_intensity, self.env_painted, self.env_glass, self.env_asphalt, self.env_ground, self.light_rig_sun_scale, self.shadow_intensity, self.table_exposure, self.env_diffuse_scale,
        ]
    }

    pub fn unpack(v: &[f64; RIG_VALUES]) -> Self {
        Rig {
            sun_color: [v[0], v[1], v[2]],
            sun_intensity: v[3],
            hemi_sky: [v[4], v[5], v[6]],
            hemi_ground: [v[7], v[8], v[9]],
            hemi_intensity: v[10],
            ambient_color: [v[11], v[12], v[13]],
            ambient_intensity: v[14],
            rim_color: [v[15], v[16], v[17]],
            rim_intensity: v[18],
            environment_intensity: v[19],
            env_painted: v[20],
            env_glass: v[21],
            env_asphalt: v[22],
            env_ground: v[23],
            light_rig_sun_scale: v[24],
            shadow_intensity: v[25],
            table_exposure: v[26],
            env_diffuse_scale: v[27],
        }
    }

    /// Linear between two rigs (the soft path's keyframes, a clock minute apart).
    pub fn lerp(a: &Rig, b: &Rig, t: f64) -> Rig {
        let (pa, pb) = (a.pack(), b.pack());
        Rig::unpack(&std::array::from_fn(|k| mix(pa[k], pb[k], t)))
    }
}

/// `rig_at` with the rows interpolated by `interp` (`Linear` is `rig_at`).
pub fn rig_at_interp(hour: f64, interp: Interp) -> Rig {
    if interp == Interp::Linear {
        return rig_at(hour);
    }
    let t = table();
    let (rows, w) = cubic_rows(t.len(), |k| t[k].hour, hour);
    let v = rows.map(|k| row_values(&t[k]));
    Rig::unpack(&std::array::from_fn(|c| monotone_cubic([v[0][c], v[1][c], v[2][c], v[3][c]], w)))
}

/// `rig_at_transfer` with the hour rows interpolated by `interp` (the
/// transfer stays linear; `Linear` is `rig_at_transfer`).
pub fn rig_at_transfer_interp(hour: f64, transfer: f64, interp: Interp) -> Rig {
    if interp == Interp::Linear {
        return rig_at_transfer(hour, transfer);
    }
    let g = transfer_grid();
    let mut r = rig_at_interp(hour, interp);
    let n = g.rows.len();
    let (rows, wh) = cubic_rows(n, |k| g.rows[k][0][0], hour);
    let t = transfer.clamp(g.transfers[0], *g.transfers.last().unwrap());
    let j = g.transfers.iter().rposition(|v| *v <= t).unwrap_or(0).min(g.transfers.len() - 2);
    let wt = clamp((t - g.transfers[j]) / (g.transfers[j + 1] - g.transfers[j]).max(1e-9), 0.0, 1.0);
    let at = |c: usize| {
        let y = rows.map(|i| mix(g.rows[i][j][c], g.rows[i][j + 1][c], wt));
        monotone_cubic(y, wh)
    };
    r.sun_intensity = at(3);
    r.hemi_intensity = at(4);
    r.ambient_intensity = at(5);
    r.rim_intensity = at(6);
    r.environment_intensity = at(7);
    r.hemi_ground = [at(8), at(9), at(10)];
    r.env_diffuse_scale = t;
    r
}

/// `keyEnvelope`: the key's smoothstep through 0..4 degrees of solar altitude.
pub fn key_envelope(altitude_deg: f64) -> f64 {
    smoothstep(0.0, 4.0, altitude_deg)
}

/// `directBeamTransmittance(altitudeDeg).transmittance`.
pub fn direct_beam_transmittance(altitude_deg: f64) -> f64 {
    const SET_DEG: f64 = -1.0;
    if altitude_deg <= SET_DEG {
        return 0.0;
    }
    let air_mass = 1.0 / ((altitude_deg * DEG).sin() + 0.50572 * (altitude_deg + 6.07995).powf(-1.6364));
    let beam = 0.7f64.powf(air_mass.powf(0.678));
    if altitude_deg >= 0.0 {
        return beam;
    }
    let s = (altitude_deg - SET_DEG) / -SET_DEG;
    beam * s * s * (3.0 - 2.0 * s)
}

/// `EXPOSURE_CURVE`.
pub const EXPOSURE_REFERENCE_HOUR: f64 = 12.0;
pub const EXPOSURE_ANCHOR: f64 = 0.82;
pub const EXPOSURE_ADAPTATION: f64 = 0.20;
pub const EXPOSURE_MIN: f64 = 0.68;
pub const EXPOSURE_MAX: f64 = 1.55;

/// `sceneIlluminance(model).total` (sky irradiance luminance + direct key on a
/// horizontal surface), with the rig's sun scale for that hour.
pub fn scene_illuminance(m: &SkyModel, rig_sun_scale: f64) -> f64 {
    let alt = m.sun.altitude_deg;
    let key = m.sun.y.max(0.0) * 2.75 * rig_sun_scale * direct_beam_transmittance(alt) * key_envelope(alt);
    luminance(m.sky_irradiance()) + key
}

fn reference_illuminance() -> f64 {
    static R: OnceLock<f64> = OnceLock::new();
    *R.get_or_init(|| {
        let m = SkyModel::new(EXPOSURE_REFERENCE_HOUR);
        scene_illuminance(&m, rig_at(EXPOSURE_REFERENCE_HOUR).light_rig_sun_scale)
    })
}

/// `recommendedExposure(model).exposure` (rounded to 1e-4 as the JS returns it).
pub fn recommended_exposure(m: &SkyModel, rig_sun_scale: f64) -> f64 {
    let ratio = reference_illuminance() / scene_illuminance(m, rig_sun_scale).max(1e-9);
    let raw = EXPOSURE_ANCHOR * ratio.powf(EXPOSURE_ADAPTATION);
    let e = clamp(raw, EXPOSURE_MIN, EXPOSURE_MAX);
    (e * 10000.0 + 0.5).floor() / 10000.0
}

/// The key's direction (`setKeyDirectionFromSun`): the sun while it is up;
/// below the horizon it hands over to the anti-solar stand-in at 52 degrees
/// across -6..0 degrees (the moon key is not ported). The key's intensity is
/// already 0 below the horizon (`keyEnvelope`).
pub fn key_direction(hour: f64) -> [f64; 3] {
    let s = sun_direction(hour);
    let alt = 52.0 * DEG;
    let h = s.x.hypot(s.z);
    let moon = if h > 1e-6 { [-s.x / h * alt.cos(), alt.sin(), -s.z / h * alt.cos()] } else { [0.0, 1.0, 0.0] };
    let edge = clamp((s.altitude_deg + 6.0) / 6.0, 0.0, 1.0);
    let w = edge * edge * (3.0 - 2.0 * edge);
    let d = [s.x * w + moon[0] * (1.0 - w), s.y * w + moon[1] * (1.0 - w), s.z * w + moon[2] * (1.0 - w)];
    let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    if l < 1e-4 { moon } else { [d[0] / l, d[1] / l, d[2] / l] }
}

/// `--smooth-time`'s rim for `key_direction(hour)`: while the key blends from
/// the sun to its anti-solar stand-in (sun -6..0 deg) it passes overhead and
/// its heading, so `rim_direction`'s, turns half a circle at once (a step at
/// dawn and at dusk without a moon: the rim lit one side of a street, then the
/// other). Here the rim turns over the top instead, through the same weight;
/// equal to `rim_direction(key_direction(hour))` with the sun up or 6 deg down.
pub fn rim_direction_smooth(hour: f64) -> [f64; 3] {
    let s = sun_direction(hour);
    let h = s.x.hypot(s.z);
    let key = key_direction(hour);
    if h <= 1e-6 {
        return rim_direction(key);
    }
    let edge = clamp((s.altitude_deg + 6.0) / 6.0, 0.0, 1.0);
    let w = edge * edge * (3.0 - 2.0 * edge);
    if w >= 1.0 || w <= 0.0 {
        return rim_direction(key);
    }
    // the stand-in's heading is the sun's turned half round.
    let standin_rim = rim_direction([-s.x, 0.0, -s.z]);
    crate::blend::slerp_dir(standin_rim, rim_direction([s.x, 0.0, s.z]), w)
}

/// The rim (`aimRimOppositeKey`): opposite the key's heading, 28 degrees up.
pub fn rim_direction(key: [f64; 3]) -> [f64; 3] {
    let h = key[0].hypot(key[2]);
    let a = 28.0 * DEG;
    let (x, z) = if h > 1e-6 { (-key[0] / h * a.cos(), -key[2] / h * a.cos()) } else { (0.0, -a.cos()) };
    [x, a.sin(), z]
}
