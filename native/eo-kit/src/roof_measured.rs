//! a kit building's roof finish tinted to the colour measured for it
//! from straight-down aerial imagery (`eo_data::roof_colours`). Applied by
//! `pipeline::drawn_materials` to every rung when a table is installed; with
//! none (the JS-oracle tests, `--no-roof-colours`, no file) nothing changes.
//!
//! * A flat roof that takes a finish draws the membrane when the measured
//!   roof is lighter than the asphalt finish's own colour, else the asphalt,
//!   so the tint (a multiply, at most 1) can reach the measurement.
//! * Every material on a roof slot (`roof`, `pitchedRoof`) not also on a wall
//!   is tinted to `measured / material colour` in linear light, blended in by
//!   confidence (full from 0.6, none under 0.25): a confident measurement is
//!   not pulled back toward the old finish.
//! * contrast (`EO_ROOF_CONTRAST=off`, `--no-roof-contrast`): a roof whose
//!   lightness sits within 0.08 (OKLab) of its walls is moved 0.08 away from
//!   them, keeping its hue, so a roof reads as a roof against its own walls.
//! * neutral (`EO_ROOF_NEUTRAL=off`, `--no-roof-neutral`): the measurement's
//!   lightness is kept and its colour cast taken off. NAIP is natural-colour
//!   orthoimagery balanced for appearance, not surface reflectance (USDA FSA
//!   NAIP information sheet: 4-band imagery, no atmospheric correction), and
//!   over the city its roofs carry the path radiance's blue-violet cast: over
//!   the table's confident entries the mean OKLab (a, b) is (+0.0023,
//!   -0.0029), magenta-blue, which under the warm sun drew a pink-lavender
//!   city of roofs. Flat roofs are white or grey membranes, grey cap sheet,
//!   gravel ballast and dark bitumen, all near neutral (a gravel or aged
//!   membrane slightly warm); a measured warm hue (OKLab hue 15-95 degrees,
//!   chroma over 0.025: clay tile, a red-brown composition roof, rusted
//!   metal) keeps its hue with its chroma held to `ROOF_WARM_CHROMA_MAX`.

use crate::pipeline::Tints;
use eo_data::roof_colours::{self as rc, RoofMeasure};
use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

/// The slots a roof is drawn with.
pub const ROOF_SLOTS: [&str; 2] = ["roof", "pitchedRoof"];
/// OKLab lightness a roof keeps from its walls.
pub const WALL_CONTRAST_L: f32 = 0.08;

pub use eo_data::roof_colours::feature_on;

fn hex_linear(h: u32) -> [f32; 3] {
    rc::rgb8_linear([(h >> 16) as u8, (h >> 8) as u8, h as u8])
}

fn linear_hex(c: [f32; 3]) -> u32 {
    let b = c.map(|v| (rc::linear_to_srgb(v) * 255.0).round() as u32);
    (b[0] << 16) | (b[1] << 8) | b[2]
}

fn parse_hex(s: &str) -> Option<u32> {
    u32::from_str_radix(s.trim_start_matches('#'), 16).ok()
}

/// A kit material's own colour (linear), from its generator parameters
/// (`p.color`, or the mean of `p.colors`) in kit-material-specs.json.
pub fn material_colour(name: &str) -> Option<[f32; 3]> {
    static MEANS: OnceLock<HashMap<String, [f32; 3]>> = OnceLock::new();
    MEANS
        .get_or_init(|| {
            let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../eo-assets/data/kit-material-specs.json");
            let Ok(text) = std::fs::read_to_string(path) else { return HashMap::new() };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return HashMap::new() };
            let mut out = HashMap::new();
            if let Some(m) = v["specs"].as_object() {
                for (k, s) in m {
                    let p = &s["p"];
                    if let Some(c) = p["color"].as_str().and_then(parse_hex) {
                        out.insert(k.clone(), hex_linear(c));
                    } else if let Some(list) = p["colors"].as_array() {
                        let cs: Vec<[f32; 3]> = list.iter().filter_map(|c| c.as_str().and_then(parse_hex)).map(hex_linear).collect();
                        if !cs.is_empty() {
                            let n = cs.len() as f32;
                            out.insert(k.clone(), [0, 1, 2].map(|i| cs.iter().map(|c| c[i]).sum::<f32>() / n));
                        }
                    }
                }
            }
            out
        })
        .get(crate::pipeline::base_material_name(name))
        .copied()
}

/// OKLab chroma a warm measured roof keeps at most (a weathered clay barrel
/// tile, sRGB about #a8735b, is about 0.08).
pub const ROOF_WARM_CHROMA_MAX: f32 = 0.08;
/// The slight warm b of a neutral roof (OKLab): gravel ballast and aged
/// membranes are warm greys, not blue ones.
pub const ROOF_NEUTRAL_B: f32 = 0.003;

/// The measured roof colour without the imagery's cast (see the module
/// notes); unchanged with `EO_ROOF_NEUTRAL=off`.
pub fn neutral(target: [f32; 3]) -> [f32; 3] {
    if !feature_on("neutral") {
        return target;
    }
    let mut l = rc::oklab(target);
    let c = l[1].hypot(l[2]);
    let h = l[2].atan2(l[1]).to_degrees();
    if c > 0.025 && (15.0..=95.0).contains(&h) {
        let k = (ROOF_WARM_CHROMA_MAX / c).min(1.0);
        l[1] *= k;
        l[2] *= k;
    } else {
        l[1] = 0.0;
        l[2] = ROOF_NEUTRAL_B * (l[0] / 0.65).min(1.0);
    }
    rc::oklab_to_linear(l).map(|v| v.clamp(0.0, 1.0))
}

/// The colour a roof is drawn toward: the measurement without its cast,
/// moved off its walls when the two are close.
pub fn target(m: &RoofMeasure, wall: Option<[f32; 3]>) -> [f32; 3] {
    let mut target = neutral(rc::rgb8_linear(m.rgb));
    if feature_on("contrast") {
        target = separated(target, wall);
    }
    target
}

/// A tag for the kit tile cache key: the neutral rule when on.
pub fn cache_tag() -> String {
    if rc::table().is_some() && feature_on("neutral") { "roof-neutral-v1".into() } else { String::new() }
}

/// How much of the measurement a roof takes, from its confidence.
pub fn weight(confidence: f32) -> f32 {
    ((confidence - 0.25) / 0.35).clamp(0.0, 1.0)
}

/// The measured colour, moved off its walls' lightness when the two are close.
pub fn separated(target: [f32; 3], wall: Option<[f32; 3]>) -> [f32; 3] {
    let Some(wall) = wall else { return target };
    let (mut r, w) = (rc::oklab(target), rc::oklab(wall));
    let d = r[0] - w[0];
    if d.abs() >= WALL_CONTRAST_L {
        return target;
    }
    // Lighter walls (most of the city's paint): the roof goes darker; dark walls: lighter.
    r[0] = if w[0] > 0.5 { w[0] - WALL_CONTRAST_L } else { (w[0] + WALL_CONTRAST_L).min(0.97) };
    rc::oklab_to_linear(r).map(|v| v.clamp(0.0, 1.0))
}

/// Retint the roof of one building (`flat_finish`: the roof-finish step applied).
pub fn apply(materials: &mut BTreeMap<String, String>, tints: &mut Tints, m: &RoofMeasure, flat_finish: bool) {
    let w = weight(m.confidence);
    if w <= 0.0 {
        return;
    }
    // a painted wall stands on the neutral primer (wall_paint.rs).
    let wall = materials.get("wall").and_then(|n| crate::wall_paint::drawn_base(n).or_else(|| material_colour(n)).map(|c| (n.clone(), c))).map(|(n, c)| {
        let t = tints.get(&n).copied().map(hex_linear).unwrap_or([1.0; 3]);
        [c[0] * t[0], c[1] * t[1], c[2] * t[2]]
    });
    let target = target(m, wall);
    if flat_finish {
        let asphalt = material_colour("asphalt-roof").map(rc::oklab).map(|l| l[0]).unwrap_or(0.4);
        let name = if rc::oklab(target)[0] > asphalt + 0.06 { "membrane-roof" } else { "asphalt-roof" };
        materials.insert("roof".into(), name.into());
    }
    for slot in ROOF_SLOTS {
        let Some(name) = materials.get(slot).cloned() else { continue };
        if materials.iter().any(|(k, v)| *v == name && !ROOF_SLOTS.contains(&k.as_str())) {
            continue; // the same material is on a wall: leave its paint alone
        }
        let Some(mean) = crate::wall_paint::drawn_base(&name).or_else(|| material_colour(&name)) else { continue }; // the primer
        let old = tints.get(&name).copied().map(hex_linear).unwrap_or([1.0; 3]);
        let want = [0, 1, 2].map(|k| (target[k] / mean[k].max(1e-4)).clamp(0.0, 1.0));
        let t = [0, 1, 2].map(|k| old[k] + (want[k] - old[k]) * w);
        tints.insert(name, linear_hex(t));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measure(rgb: [u8; 3], confidence: f32) -> RoofMeasure {
        RoofMeasure { rgb, confidence, second: rgb, second_share: 0.0, second_at: [0.0; 2], vegetation: 0.0 }
    }

    #[test]
    fn material_colours_come_from_the_specs() {
        let a = material_colour("asphalt-roof").unwrap();
        assert!((a[0] - rc::srgb_to_linear(0x4a as f32 / 255.0)).abs() < 1e-5);
        assert!(material_colour("brick-running-bond").is_some(), "a brick's mean of its colours");
        assert!(material_colour("no-such-material").is_none());
    }

    #[test]
    fn a_light_roof_takes_the_membrane_and_its_measured_colour() {
        let mut mats: BTreeMap<String, String> = [("wall".to_string(), "painted-stucco".to_string()), ("roof".to_string(), "asphalt-roof".to_string())].into();
        let mut tints = Tints::new();
        apply(&mut mats, &mut tints, &measure([150, 152, 155], 0.9), true);
        assert_eq!(mats["roof"], "membrane-roof");
        let t = hex_linear(tints["membrane-roof"]);
        let mean = material_colour("membrane-roof").unwrap();
        let got = [t[0] * mean[0], t[1] * mean[1], t[2] * mean[2]];
        // the measurement, its imagery cast taken off (`neutral`; as measured with it off).
        let want = neutral(rc::rgb8_linear([150, 152, 155]));
        for k in 0..3 {
            assert!((got[k] - want[k]).abs() < 0.01, "{got:?} {want:?}");
        }
    }

    #[test]
    fn low_confidence_changes_nothing() {
        let mut mats: BTreeMap<String, String> = [("wall".to_string(), "painted-stucco".to_string()), ("roof".to_string(), "asphalt-roof".to_string())].into();
        let mut tints = Tints::new();
        apply(&mut mats, &mut tints, &measure([200, 200, 200], 0.2), true);
        assert_eq!(mats["roof"], "asphalt-roof");
        assert!(tints.is_empty());
    }

    #[test]
    fn a_roof_shared_with_a_wall_keeps_its_paint() {
        let mut mats: BTreeMap<String, String> = [("wall".to_string(), "painted-stucco".to_string()), ("roof".to_string(), "painted-stucco".to_string())].into();
        let mut tints = Tints::new();
        apply(&mut mats, &mut tints, &measure([60, 60, 60], 1.0), false);
        assert!(tints.is_empty());
    }

    #[test]
    fn the_imagery_cast_is_taken_off_and_warm_roofs_keep_their_hue() {
        if !feature_on("neutral") {
            return;
        }
        // A lavender grey (the mean cast) goes neutral at its own lightness.
        let lav = rc::rgb8_linear([168, 162, 172]);
        let n = rc::oklab(neutral(lav));
        assert!((n[0] - rc::oklab(lav)[0]).abs() < 1e-3);
        assert!(n[1].abs() < 1e-3 && n[2] >= 0.0 && n[2] <= ROOF_NEUTRAL_B + 1e-4, "{n:?}");
        // A clay tile roof keeps its hue, its chroma held.
        let tile = rc::rgb8_linear([180, 97, 63]);
        let t = rc::oklab(neutral(tile));
        let o = rc::oklab(tile);
        assert!((t[2].atan2(t[1]) - o[2].atan2(o[1])).abs() < 0.01);
        assert!(t[1].hypot(t[2]) <= ROOF_WARM_CHROMA_MAX + 1e-3);
    }

    #[test]
    fn a_roof_like_its_walls_is_moved_off_them() {
        let wall = rc::rgb8_linear([0xd9, 0xcf, 0xbd]);
        let s = separated(wall, Some(wall));
        assert!((rc::oklab(s)[0] - (rc::oklab(wall)[0] - WALL_CONTRAST_L)).abs() < 1e-3);
        let far = rc::rgb8_linear([40, 40, 40]);
        assert_eq!(separated(far, Some(wall)), far);
    }
}
