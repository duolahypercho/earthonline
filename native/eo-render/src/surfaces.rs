//! the far level of a building (past the kit's mid rung, 470 m) in
//! the JS far tier's colour, and its floor near the kit's.
//!
//! Colour: the JS horizon ring (src/world/horizon/coarse-massing.js
//! `buildCoarseMassing`): the record's material as the loader infers it
//! (src/citygen/sf-data.js `inferMaterial`: a declared material, else the
//! program's, else the type's residual table on `hashString('sf-material-<id>-<name>')`;
//! no civic sidecar ships, so `civicAppearance` is null), re-resolved for the
//! far ring (`coarseMaterialFor`: tall brick/painted/clapboard remapped, towers
//! glazed), one hex from the near tier's palette for that material by
//! `hash32('coarse-<id>')`, pulled 15 % toward the palette mean, value jitter
//! 0.09, warm/cool drift 0.075, the 0.8 gain, and the sky-to-street gradient
//! 0.14 split into a wall foot and head colour. The atlas cell the JS samples
//! on top of that (FACADE_ATLAS_V2) is not drawn here.

use eo_data::Building;
use eo_kit::classify::{RawRecord, infer_program, infer_type};
use eo_kit::pipeline::{flat_to_points, polygon_area};

fn fnv(s: &str) -> u32 {
    let mut h: u32 = 2166136261;
    for c in s.encode_utf16() {
        h ^= c as u32;
        h = h.wrapping_mul(16777619);
    }
    h
}

const PALETTES: [(&str, &[&str]); 7] = [
    ("painted", &["#7a524c", "#5a6a68", "#7a6a4c", "#5c6470", "#6e5848", "#5e6a58", "#6a5458"]),
    ("plaster", &["#c8b49a", "#b89a82", "#d0c0a8", "#c0a890", "#c8b496"]),
    ("brick", &["#7a4538", "#6a382e", "#8a5240", "#5c3028", "#7a4a3c"]),
    ("concrete", &["#c4bfb2", "#b4afa2", "#d0cbbf", "#bbb6a8", "#c8c3b6", "#d6dadc", "#cfd6da", "#e0e2e0"]),
    ("clapboard", &["#5a6a72", "#7a6c5c", "#4e5e52", "#6a5c5c", "#4e5c6a"]),
    ("glass", &["#7fa7bd", "#6d98ad", "#a7c4cf", "#8eb2bf", "#789eae", "#9dbbc5", "#88aab8", "#a9c3d2", "#b7cdd8", "#7ca99c"]),
    ("stone", &["#ddd4be", "#ccc0a8", "#e8dfca", "#d4c8b0", "#e0d6c2"]),
];
const TONE_JITTER: f64 = 0.09;
const HUE_DRIFT: f64 = 0.075;
const SKY_GRADIENT: f64 = 0.14;
const FAR_TOWER_MIN_HEIGHT_M: f64 = 60.0;

fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}
fn linear_to_srgb(c: f64) -> f64 {
    if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}
fn hex_linear(h: &str) -> [f64; 3] {
    let n = u32::from_str_radix(h.trim_start_matches('#'), 16).unwrap_or(0);
    [(n >> 16) & 255, (n >> 8) & 255, n & 255].map(|v| srgb_to_linear(v as f64 / 255.0))
}

/// `programMaterial` (sf-data.js).
fn program_material(program: &str, hash: u32) -> Option<&'static str> {
    let pick = |t: [&'static str; 6]| Some(t[(hash % 6) as usize]);
    match program {
        "parking" => Some("concrete"),
        "hall" => pick(["concrete", "glass", "concrete", "metal", "concrete", "glass"]),
        "big-box" => pick(["concrete", "painted", "metal", "concrete", "painted", "concrete"]),
        "warehouse" => pick(["brick", "concrete", "brick", "painted", "concrete", "metal"]),
        _ => None,
    }
}

/// `inferMaterial(building, type, program, null)` (sf-data.js). The extract
/// carries no `material` tag in this checkout's records the native loader reads,
/// so the declared-material branch is not taken.
pub fn infer_material(b: &Building) -> &'static str {
    let polygon = flat_to_points(&b.points);
    let area = polygon_area(&polygon);
    let raw = RawRecord {
        id_text: &b.id,
        building: &b.tags.building,
        amenity: &b.tags.amenity,
        name: &b.tags.name,
        levels: b.tags.levels,
        height: b.tags.height,
        area: b.tags.area,
        points: &polygon,
    };
    let program = infer_program(&raw, area);
    let type_ = infer_type(&raw, program);
    let hash = fnv(&format!("sf-material-{}-{}", b.id, b.tags.name));
    if let Some(m) = program_material(program, hash) {
        return m;
    }
    let t: [&'static str; 6] = match type_ {
        "rowhouse" => ["painted", "painted", "clapboard", "brick", "plaster", "stone"],
        "shop" => ["painted", "brick", "plaster", "stone", "clapboard", "brick"],
        "tower" => ["glass", "concrete", "brick", "painted", "glass", "concrete"],
        "civic" => ["stone", "stone", "concrete", "painted", "brick", "plaster"],
        "warehouse" => ["brick", "brick", "concrete", "painted", "stone", "plaster"],
        _ => ["painted", "brick", "concrete", "glass", "stone", "plaster"],
    };
    t[(hash % 6) as usize]
}

/// `coarseMaterialFor` (coarse-massing.js).
pub fn coarse_material(material: &str, height: f64, hash: u32) -> &'static str {
    let mut known: &'static str = PALETTES.iter().find(|p| p.0 == material).map(|p| p.0).unwrap_or("concrete");
    if height > 18.0 && matches!(known, "brick" | "painted" | "clapboard") {
        let r = (hash >> 9) % 10;
        if r < 4 {
            known = "concrete";
        } else if r < 6 {
            known = "stone";
        } else if r < 8 {
            known = "glass";
        }
    }
    if !(height > FAR_TOWER_MIN_HEIGHT_M) || matches!(known, "glass" | "concrete" | "stone") {
        return known;
    }
    if height >= 120.0 {
        return "glass";
    }
    if (hash >> 5) % 3 == 0 { "concrete" } else { "glass" }
}

/// Wall foot and head colour (sRGB bytes) of a building's far level.
pub fn far_wall_tones(b: &Building) -> ([u8; 3], [u8; 3]) {
    let hash = fnv(&format!("coarse-{}", b.id));
    let material = coarse_material(infer_material(b), b.height as f64, hash);
    let pal = PALETTES.iter().find(|p| p.0 == material).map(|p| p.1).unwrap_or(PALETTES[0].1);
    let colors: Vec<[f64; 3]> = pal.iter().map(|h| hex_linear(h)).collect();
    let mut mean = [0.0; 3];
    for c in &colors {
        for k in 0..3 {
            mean[k] += c[k] / colors.len() as f64;
        }
    }
    let pick = colors[(hash as usize) % colors.len()];
    let jitter = 1.0 + ((((hash >> 8) & 0xff) as f64 / 255.0) * 2.0 - 1.0) * TONE_JITTER;
    let drift = ((((hash >> 17) & 0xff) as f64 / 255.0) * 2.0 - 1.0) * HUE_DRIFT;
    let warm = [1.0 + drift * 0.9, 1.0 + drift * 0.1, 1.0 - drift * 0.8];
    let tone: [f64; 3] = [0, 1, 2].map(|k| (pick[k] * 0.85 + mean[k] * 0.15) * jitter * warm[k] * 0.8);
    let s = SKY_GRADIENT;
    let foot = [tone[0] * (1.0 - s), tone[1] * (1.0 - s), tone[2] * (1.0 - s * 0.94)];
    let head = [(tone[0] * (1.0 + s * 0.62)).min(1.0), (tone[1] * (1.0 + s * 0.64)).min(1.0), (tone[2] * (1.0 + s * 0.72)).min(1.0)];
    let enc = |c: [f64; 3]| c.map(|v| (linear_to_srgb(v.clamp(0.0, 1.0)) * 255.0).round() as u8);
    (enc(foot), enc(head))
}
