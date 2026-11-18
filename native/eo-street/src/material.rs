//! The street surface's material numbers, as the JS builds them
//! (`data/street-material.json`, written by the earlier browser version's exporter
//! from src/world/streets/street-surface-v2.js and street-surface-v2-render.js):
//! the per-class albedo tints (`STREET_SURFACE_V2_PALETTES.sf`), the scanned
//! PBR sets (`STREET_SURFACE_PBR_SETS`: asphalt and brushed concrete, their
//! files, tile sizes, means and the normalisation `surfaceSetNodes` applies),
//! the footway's base fold, the reflection classes and the shading constants
//! the native shader ports.

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

const MATERIAL_JSON: &str = include_str!("../data/street-material.json");

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Aggregate {
    pub tile_metres: f64,
    pub albedo_contrast: f64,
    pub albedo_clamp: [f64; 2],
    pub normal_strength: f64,
    pub roughness_gain: f64,
    pub roughness_clamp: [f64; 2],
    pub near_m: f64,
    pub far_m: f64,
    pub far_contrast: f64,
    pub relief_near_m: f64,
    pub relief_far_m: f64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PbrSet {
    pub id: String,
    pub provenance: String,
    pub diffuse: String,
    pub nra: String,
    pub tile_metres: f64,
    pub base_mean_linear: Option<[f64; 3]>,
    pub mean_linear: [f64; 3],
    pub mean_ao: f64,
    pub mean_roughness: f64,
    pub albedo_contrast: f64,
    pub albedo_clamp: [f64; 2],
    pub roughness_gain: f64,
    pub roughness_clamp: [f64; 2],
    pub normal_strength: f64,
    pub ao_strength: f64,
    pub detail_near_m: f64,
    pub detail_far_m: f64,
    pub far_contrast: f64,
    pub aggregate: Option<Aggregate>,
    pub patch: Option<Patch>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Shading {
    pub macro_coarse: f64,
    pub macro_fine: f64,
    pub macro_amplitude: f64,
    pub cross_fall_darken: f64,
    pub cross_fall_reach_m: f64,
    pub gutter_silt_reach_m: f64,
    pub gutter_silt_darken: f64,
    pub gutter_pan_pour_silt: f64,
    /// The footway scan lands every this many metres (the JS rescales the
    /// concrete set's repeat by `tileMetres / concreteScanPeriodM`).
    pub concrete_scan_period_m: f64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Patch {
    pub frequency: f64,
    pub phase: f64,
    pub start: f64,
    pub full: f64,
    pub scale: f64,
    pub offset: [f64; 2],
    pub tone: f64,
    pub roughness: f64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PbrSets {
    pub carriageway: PbrSet,
    pub concrete: PbrSet,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreetMaterial {
    pub sources: HashMap<String, String>,
    pub palette: HashMap<String, serde_json::Value>,
    pub uv_meters_per_repeat: HashMap<String, f64>,
    pub gutter_width: f64,
    pub concrete_base_mean_linear: f64,
    pub env_classes: HashMap<String, String>,
    pub pbr_sets: PbrSets,
    pub shading: Shading,
    /// The ground materials' own roughness: carriageway, concrete, markings.
    pub roughness: HashMap<String, f64>,
    /// Clear-weather envMapIntensity of the street's classes.
    pub env_base: HashMap<String, f64>,
    /// street-surface-age.js: the asphalt bleach.
    pub surface_age: SurfaceAge,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceAge {
    pub bleach_fresh: f64,
    pub bleach_aged: f64,
    pub cycle_overrun: f64,
    pub samples: Vec<AgeSample>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AgeSample {
    pub key: String,
    pub bleach: f64,
}

/// `unit(key)` in street-surface-age.js.
fn unit(key: &str) -> f64 {
    (crate::hash32(key) % 100003) as f64 / 100003.0
}

/// `scaleColor` (street-surface-v2.js): an sRGB code multiplier, clamped.
fn scale_srgb(c: [u8; 3], k: f64) -> [u8; 3] {
    c.map(|v| ((v as f64 / 255.0 * k).clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// sRGB bytes of a `#rrggbb` string.
pub fn hex_rgb(hex: &str) -> [u8; 3] {
    let n = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0x808080);
    [(n >> 16) as u8, (n >> 8) as u8, n as u8]
}

impl StreetMaterial {
    pub fn get() -> &'static StreetMaterial {
        static M: OnceLock<StreetMaterial> = OnceLock::new();
        M.get_or_init(|| serde_json::from_str(MATERIAL_JSON).expect("street-material.json"))
    }

    /// A flat palette entry (`sidewalk`, `curbTop`, `markingWhite`, ...).
    pub fn tint(&self, key: &str) -> [u8; 3] {
        hex_rgb(self.palette.get(key).and_then(|v| v.as_str()).unwrap_or("#808080"))
    }

    /// The carriageway tint for a highway class (`_link` folds into its class,
    /// anything unlisted is `default`), as the JS palette ladder names them.
    pub fn asphalt_tint(&self, highway: &str) -> [u8; 3] {
        let table = self.palette.get("asphalt").and_then(|v| v.as_object());
        let key = highway.trim_end_matches("_link");
        let hex = table.and_then(|t| t.get(key).or_else(|| t.get("default"))).and_then(|v| v.as_str()).unwrap_or("#e5e8e9");
        hex_rgb(hex)
    }

    /// `streetSurfaceAge(segment).bleach`: the oxidised binder's tone on the
    /// class albedo, keyed on the street (`streetId`, else the segment id).
    pub fn bleach(&self, key: &str) -> f64 {
        let a = &self.surface_age;
        let age_share = unit(&format!("surface-age:{key}")) * a.cycle_overrun;
        let t = age_share.min(1.0);
        a.bleach_fresh + (a.bleach_aged - a.bleach_fresh) * t
    }

    /// The carriageway's vertex tint as `emitSegment` writes it: the class
    /// hex x the +-2 % segment jitter x the street's bleach, on the sRGB code.
    pub fn carriageway_tint(&self, segment_id: &str, street_id: Option<&str>, highway: &str) -> [u8; 3] {
        let jitter = 1.0 + ((crate::hash32(segment_id) % 41) as f64 - 20.0) / 1000.0;
        let key = street_id.unwrap_or(segment_id);
        scale_srgb(self.asphalt_tint(highway), jitter * self.bleach(key))
    }

    /// The footway's per-street pour tone (`sidewalkStreetJitter`, +-3.5 % on the sRGB code).
    pub fn footway_tint(&self, segment_id: &str, street_id: Option<&str>) -> [u8; 3] {
        let key = street_id.filter(|s| !s.is_empty()).unwrap_or(segment_id);
        let jitter = 1.0 + ((crate::hash32(&format!("walk:{key}")) % 71) as f64 - 35.0) / 1000.0;
        scale_srgb(self.tint("sidewalk"), jitter)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn bleach_equals_js() {
        let m = super::StreetMaterial::get();
        for s in &m.surface_age.samples {
            assert!((m.bleach(&s.key) - s.bleach).abs() < 1e-9, "{}: {} vs JS {}", s.key, m.bleach(&s.key), s.bleach);
        }
    }
}
