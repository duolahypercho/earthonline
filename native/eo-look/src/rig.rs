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

/// The rim (`aimRimOppositeKey`): opposite the key's heading, 28 degrees up.
pub fn rim_direction(key: [f64; 3]) -> [f64; 3] {
    let h = key[0].hypot(key[2]);
    let a = 28.0 * DEG;
    let (x, z) = if h > 1e-6 { (-key[0] / h * a.cos(), -key[2] / h * a.cos()) } else { (0.0, -a.cos()) };
    [x, a.sin(), z]
}
