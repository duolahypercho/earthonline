//! The Golden Gate's coastal trees and scrub as ez-tree species of our own
//! (not in the browser export): mature Monterey cypress, Monterey pine and
//! blue gum eucalyptus for the Presidio stands and the bluff groves, and
//! coyote brush and California sagebrush for the coastal scrub. They grow
//! from the export's presets (`Oak Large`, `Pine Large`, `Ash Large`,
//! `Bush 1`, `Bush 3`) with the overrides below, through the same generator
//! and post-processing as every other species (`build_ez_tree`), and are
//! cooked with them (`cook.rs` builds every tier of every species).
//!
//! Sizes are published values; the forms are the published habit described
//! in words, so every branch number below is INFERRED (the generator's
//! parameters have no published counterpart). Sizes here are the species'
//! mature size at the 1.0 instance scale; the planting draws a spread round
//! them (`eo_landmarks::vegetation`).
//!
//! * Monterey cypress (*Hesperocyparis macrocarpa*): trees "to 25 m tall",
//!   the crown "generally broad-spreading, especially on exposed headlands",
//!   "a few major limbs emerging from near the base of the trunk", foliage
//!   "not glaucous" (The Gymnosperm Database, conifers.org, Hesperocyparis
//!   macrocarpa, read 2026-11-18). Drawn 16 m tall, crown up to 10 m half
//!   width (INFERRED: a mature bluff tree below the 25 m bound, crown as wide
//!   as tall on exposed sites).
//! * Monterey pine (*Pinus radiata*): "15-30 m tall", the crown "broadly
//!   conic" young, "becoming rounded to flattened", needles "deep
//!   yellow-green" (conifers.org, Pinus radiata, read 2026-11-18). Drawn 24 m,
//!   half crown 7 m (INFERRED).
//! * Blue gum (*Eucalyptus globulus*): "tall (150-180 foot), aromatic,
//!   straight-growing", frequently "100 feet or more"; bark peeling "in long
//!   strips" to a smooth pale trunk (Cal-IPC plant report and fact sheet,
//!   read 2026-11-18); about half the Presidio forest's trees (Presidio Trust
//!   forestry fact sheet, June 2023, search snippet). Drawn 34 m (INFERRED:
//!   a planted stand tree between the 30 m and 46 m figures), half crown 7.5 m,
//!   the glaucous blue-green of its foliage as a leaf tint (INFERRED).
//! * Coyote brush (*Baccharis pilularis*): the erect form "grows up to 12 feet
//!   (4 m) high", the matted form "1-4 m wide" (USDA Forest Service FEIS,
//!   Baccharis pilularis, read 2026-11-18). Drawn as a mound 1.6 m tall, half
//!   width 1.3 m (INFERRED, inside those bounds).
//! * California sagebrush (*Artemisia californica*): a grey-green coastal
//!   scrub shrub with coyote brush in the headlands' scrub (NPS Golden Gate,
//!   Scrublands). Drawn 1.2 m tall, half width 0.9 m (INFERRED).

use super::{EzSpecies, EzTier};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// The ids, in render order (the props library gives each a model).
pub const COASTAL_TREES: [&str; 3] = ["coastal-cypress", "coastal-pine", "coastal-gum"];
pub const COASTAL_SHRUBS: [&str; 2] = ["coastal-coyote-brush", "coastal-sage"];

fn tier(section_stride: f64, segment_factor: f64, leaf_stride: f64, leaf_scale: f64, single: bool, lift: bool) -> EzTier {
    EzTier {
        section_stride: Some(section_stride),
        segment_factor: Some(segment_factor),
        leaf_stride: Some(leaf_stride),
        leaf_scale: Some(leaf_scale),
        billboard: single.then(|| "single".to_string()),
        lift_leaves: lift,
    }
}

/// The three levels a coastal tree draws (near, coarse, distant), as the
/// street species' tiers are shaped: fewer sections and leaf cards further
/// out, each card larger so the crown keeps its cover.
fn tree_tiers() -> BTreeMap<String, EzTier> {
    let mut t = BTreeMap::new();
    t.insert("near".into(), tier(2.0, 0.7, 1.0, 1.0, false, false));
    t.insert("coarse".into(), tier(6.0, 0.4, 3.0, 1.65, true, false));
    t.insert("distant".into(), tier(12.0, 0.25, 7.0, 2.4, true, false));
    t
}

fn shrub_tiers() -> BTreeMap<String, EzTier> {
    let mut t = BTreeMap::new();
    t.insert("near".into(), tier(3.0, 0.6, 1.0, 1.0, false, false));
    t.insert("coarse".into(), tier(12.0, 0.4, 3.0, 1.6, true, false));
    t.insert("distant".into(), tier(24.0, 0.25, 7.0, 2.4, true, false));
    t
}

#[allow(clippy::too_many_arguments)]
fn species(id: &str, preset: &str, leaf: &str, height_m: f64, half_crown_m: f64, clearance_m: Option<f64>, bark: [f64; 3], leaf_tint: [f64; 3], tiers: BTreeMap<String, EzTier>, overrides: Value) -> EzSpecies {
    EzSpecies {
        id: id.to_string(),
        preset: preset.to_string(),
        leaf: leaf.to_string(),
        height_m,
        max_half_crown_m: half_crown_m,
        clearance_m,
        max_height_m: None,
        height_factor: None,
        crown_factor: None,
        facade_reach_m: None,
        street_lean_m: None,
        trunk_bend: None,
        observed_only: false,
        surveyed_only: false,
        // the crown-cover rule (`crownLeafScale`) brings a thin crown up to the export's cover.
        crown_cover: true,
        seed_id: None,
        leaf_states: BTreeMap::new(),
        bark_tint: bark,
        leaf_tint,
        tiers,
        overrides,
    }
}

/// Every coastal species (added to the export's table by `EzData::from_gz`).
pub fn species_list() -> Vec<EzSpecies> {
    vec![
        // A short bole parting into a few great ascending limbs low down, then
        // wide-angled side limbs carrying flat, dense sprays: the broad,
        // flat-topped crown of an exposed cypress. Dark, not glaucous.
        species(
            "coastal-cypress",
            "Oak Large",
            "pine",
            16.0,
            10.0,
            Some(2.2),
            [0.62, 0.52, 0.44],
            [0.34, 0.42, 0.40],
            tree_tiers(),
            json!({
                "branch": {
                    "levels": 3,
                    "angle": { "1": 54, "2": 62, "3": 60 },
                    "children": { "0": 6, "1": 6, "2": 4 },
                    "length": { "0": 22, "1": 34, "2": 15, "3": 7 },
                    "radius": { "0": 3.6, "1": 0.8, "2": 0.6, "3": 0.9 },
                    "start": { "1": 0.3, "2": 0.3, "3": 0.1 },
                    "gnarliness": { "0": 0.1, "1": 0.18, "2": 0.16, "3": 0.1 },
                    "force": { "direction": { "x": 0, "y": 1, "z": 0 }, "strength": 0.025 },
                    "taper": { "0": 0.6, "1": 0.7, "2": 0.7, "3": 0.7 }
                },
                "leaves": { "billboard": "double", "angle": 80, "count": 9, "start": 0.15, "size": 7.5, "sizeVariance": 0.45 }
            }),
        ),
        // A long clean bole and a rounded, irregular crown of dense needle
        // tufts on the upper half.
        species(
            "coastal-pine",
            "Ash Large",
            "pine",
            24.0,
            7.0,
            Some(7.0),
            [0.58, 0.47, 0.40],
            [0.40, 0.46, 0.45],
            tree_tiers(),
            json!({
                "branch": {
                    "levels": 3,
                    "angle": { "1": 64, "2": 52, "3": 45 },
                    "children": { "0": 9, "1": 5, "2": 4 },
                    "length": { "0": 46, "1": 25, "2": 11, "3": 4 },
                    "radius": { "0": 2.8, "1": 0.55, "2": 0.6, "3": 0.9 },
                    "start": { "1": 0.5, "2": 0.3, "3": 0.1 },
                    "gnarliness": { "0": 0.03, "1": 0.16, "2": 0.12, "3": 0.08 },
                    "force": { "direction": { "x": 0, "y": 1, "z": 0 }, "strength": 0.02 }
                },
                "leaves": { "billboard": "double", "angle": 40, "count": 9, "start": 0.15, "size": 6.5, "sizeVariance": 0.4 }
            }),
        ),
        // Tall and straight, the crown open and irregular in the upper third,
        // drooping blue-green foliage on long limbs; the trunk pale.
        species(
            "coastal-gum",
            "Aspen Large",
            "ash",
            34.0,
            7.5,
            Some(14.0),
            [0.95, 0.90, 0.80],
            [0.42, 0.50, 0.60],
            tree_tiers(),
            json!({
                "branch": {
                    "levels": 3,
                    "angle": { "1": 32, "2": 46, "3": 50 },
                    "children": { "0": 9, "1": 5, "2": 4 },
                    "length": { "0": 62, "1": 32, "2": 13, "3": 4 },
                    "radius": { "0": 2.6, "1": 0.5, "2": 0.6, "3": 0.8 },
                    "start": { "1": 0.4, "2": 0.3, "3": 0.1 },
                    "gnarliness": { "0": 0.06, "1": 0.16, "2": 0.18, "3": 0.1 },
                    "force": { "direction": { "x": 0, "y": 1, "z": 0 }, "strength": -0.004 },
                    "taper": { "0": 0.7, "1": 0.7, "2": 0.7, "3": 0.7 }
                },
                "leaves": { "billboard": "double", "angle": 25, "count": 10, "start": 0.2, "size": 6.0, "sizeVariance": 0.5 }
            }),
        ),
        // A dense rounded mound of many stems from the ground, small leaves.
        species(
            "coastal-coyote-brush",
            "Bush 1",
            "aspen",
            1.6,
            1.3,
            None,
            [0.55, 0.47, 0.40],
            [0.40, 0.48, 0.40],
            shrub_tiers(),
            json!({
                "branch": {
                    "levels": 2,
                    "children": { "0": 11, "1": 5, "2": 3 },
                    "angle": { "1": 40, "2": 55, "3": 50 },
                    "start": { "1": 0.0, "2": 0.2, "3": 0.0 }
                },
                "leaves": { "billboard": "double", "angle": 60, "count": 14, "start": 0.0, "size": 3.6, "sizeVariance": 0.5 }
            }),
        ),
        // A looser, feathery grey-green shrub.
        species(
            "coastal-sage",
            "Bush 3",
            "pine",
            1.2,
            0.9,
            None,
            [0.55, 0.50, 0.44],
            [0.52, 0.56, 0.80],
            shrub_tiers(),
            json!({
                "leaves": { "billboard": "double", "angle": 50, "count": 6, "start": 0.1, "size": 3.0, "sizeVariance": 0.5 }
            }),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coastal_species_are_in_the_table_and_build() {
        let d = super::super::EzData::get().unwrap();
        for id in COASTAL_TREES.iter().chain(COASTAL_SHRUBS.iter()) {
            let sp = d.species(id).unwrap();
            for t in ["near", "coarse", "distant"] {
                assert!(sp.tiers.contains_key(t), "{id} {t}");
            }
            let m = super::super::build_ez_tree(id, "near", 0).unwrap();
            assert!(m.triangles() > 50, "{id}: {} triangles", m.triangles());
            // the build is scaled to the species' height.
            assert!((m.user.height_m - sp.height_m).abs() < sp.height_m * 0.02, "{id}: {} m against {} m", m.user.height_m, sp.height_m);
            assert!(m.user.half_crown_m <= sp.max_half_crown_m * 1.02, "{id}: half crown {}", m.user.half_crown_m);
        }
    }

    #[test]
    fn published_sizes_bound_the_drawn_ones() {
        // conifers.org: cypress to 25 m, pine 15-30 m; Cal-IPC: blue gum 100-180 ft;
        // FEIS: coyote brush to 4 m.
        let d = super::super::EzData::get().unwrap();
        let h = |id: &str| d.species(id).unwrap().height_m;
        assert!(h("coastal-cypress") <= 25.0);
        assert!((15.0..=30.0).contains(&h("coastal-pine")));
        assert!((30.0..=55.0).contains(&h("coastal-gum")));
        assert!(h("coastal-coyote-brush") <= 4.0);
    }
}
