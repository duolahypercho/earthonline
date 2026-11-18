//! the constants of the JS sky's lower hemisphere
//! (`DEFAULT_GROUND_FIELD` and `makeGroundField`'s set-up, environment-ibl.js),
//! read from data/ground-field.json (the earlier browser version's exporter) and
//! resolved the way `makeGroundField` resolves them (dry, no snow).

use crate::math::{DEG, Rgb, clamp, luminance};
use serde_json::Value;
use std::sync::OnceLock;

const JSON: &str = include_str!("../data/ground-field.json");

#[derive(Clone, Debug)]
pub struct GroundField {
    pub s_kerb: f64,
    pub s_far_kerb: f64,
    pub s_far_walk: f64,
    /// (sin, -cos) of the street bearing: azimuth 0 looks -Z.
    pub axis: [f64; 2],
    pub cross_share: f64,
    pub road_ratio: f64,
    pub wall_ratio: f64,
    pub road_tint: Rgb,
    pub wall_tint: Rgb,
    pub spec_max: f64,
    pub walled_share: f64,
    pub clutter: f64,
    pub clutter_sin: f64,
    /// Dry sharpness (wetness 0).
    pub sharpness: f64,
    pub lamp_pool: f64,
    pub shop_spill: f64,
    pub pool_color: Rgb,
    pub spill_color: Rgb,
}

fn chroma_only(t: Rgb) -> Rgb {
    let l = luminance(t).max(1e-9);
    [t[0] / l, t[1] / l, t[2] / l]
}

impl GroundField {
    pub fn get() -> &'static GroundField {
        static G: OnceLock<GroundField> = OnceLock::new();
        G.get_or_init(|| {
            let v: Value = serde_json::from_str(JSON).expect("ground-field.json");
            let f = &v["field"];
            let n = |k: &str| f[k].as_f64().unwrap_or(0.0);
            let rgb = |k: &str| {
                let a = f[k].as_array().expect(k);
                [a[0].as_f64().unwrap(), a[1].as_f64().unwrap(), a[2].as_f64().unwrap()]
            };
            let eye = n("eyeHeightM").max(0.2);
            let sin_at = |dist: f64| eye / eye.hypot(dist.max(0.01));
            let kerb = n("kerbDistanceM").max(0.05);
            let cw = n("carriagewayWidthM").max(1.0);
            let fw = n("farSidewalkWidthM").max(0.0);
            let ax = n("streetAzimuthDeg") * DEG;
            GroundField {
                s_kerb: sin_at(kerb),
                s_far_kerb: sin_at(kerb + cw),
                s_far_walk: sin_at(kerb + cw + fw),
                axis: [ax.sin(), -ax.cos()],
                cross_share: clamp(n("crossStreetShare"), 0.0, 1.0),
                road_ratio: n("carriagewayAlbedoRatio").max(0.0),
                wall_ratio: n("streetwallAlbedoRatio").max(0.0),
                road_tint: chroma_only(rgb("carriagewayTint")),
                wall_tint: chroma_only(rgb("streetwallTint")),
                spec_max: clamp(n("specularMax"), 0.0, 1.0),
                walled_share: clamp(n("specularWalledShare"), 0.0, 1.0),
                clutter: clamp(n("horizonClutter"), 0.0, 1.0),
                clutter_sin: (n("horizonClutterDeg").max(0.0) * DEG).sin(),
                sharpness: clamp(n("specularSharpness"), 0.0, 1.0),
                lamp_pool: n("lampPoolRadiance").max(0.0),
                shop_spill: n("shopSpillRadiance").max(0.0),
                pool_color: chroma_only(rgb("lampPoolColor")),
                spill_color: chroma_only(rgb("shopSpillColor")),
            }
        })
    }
}

/// The JS's own `sampleSkyRadiance` below the horizon (test vectors).
pub fn oracle() -> Value {
    serde_json::from_str(JSON).expect("ground-field.json")
}
