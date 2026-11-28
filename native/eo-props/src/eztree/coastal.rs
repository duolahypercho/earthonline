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

/// The ids, in render order (the props library gives each a model; the order is
/// `eo_landmarks::vegetation::Kind::Tree`'s): the three first species, then the
/// hill parks' own (each place its own mix: eo-app props/lidar_trees.rs).
pub const COASTAL_TREES: [&str; 17] = [
    "coastal-cypress",
    "coastal-pine",
    "coastal-gum",
    "coastal-gum-stand",
    "coastal-cypress-wind",
    "coastal-pine-old",
    "coastal-cypress-row",
    "coastal-oak",
    "coastal-oak-multi",
    "coastal-acacia",
    "coastal-willow",
    // second builds of the commonest species ([`VARIANTS`])
    "coastal-gum-b",
    "coastal-gum-stand-b",
    "coastal-cypress-b",
    "coastal-pine-b",
    "coastal-oak-b",
    "coastal-acacia-b",
];

/// Second builds of the commonest species: another seed and the limbs a little
/// otherwise (their angles, how many), so a stand of one species is not one tree
/// repeated: (the variant's id, its species', the seed, the angle's turn in degrees,
/// the trunk's children's change).
pub const VARIANTS: [(&str, &str, u32, f64, i64); 6] = [
    ("coastal-gum-b", "coastal-gum", 1201, 6.0, 1),
    ("coastal-gum-stand-b", "coastal-gum-stand", 4977, -4.0, -1),
    ("coastal-cypress-b", "coastal-cypress", 3313, -6.0, 1),
    ("coastal-pine-b", "coastal-pine", 6151, 5.0, -1),
    ("coastal-oak-b", "coastal-oak", 7023, -5.0, 1),
    ("coastal-acacia-b", "coastal-acacia", 9137, 6.0, 1),
];

/// A species' second build: its definition with `seed`, every branch level's
/// angle turned by `turn` degrees and the trunk's children changed by `more`.
fn variant(base: &EzSpecies, id: &str, seed: u32, turn: f64, more: i64) -> EzSpecies {
    let mut v = base.clone();
    v.id = id.to_string();
    let o = &mut v.overrides;
    o["seed"] = json!(seed);
    if let Some(angle) = o.pointer_mut("/branch/angle").and_then(Value::as_object_mut) {
        for a in angle.values_mut() {
            if let Some(x) = a.as_f64() {
                *a = json!((x + turn).clamp(5.0, 89.0));
            }
        }
    }
    if let Some(c) = o.pointer_mut("/branch/children/0") {
        if let Some(n) = c.as_i64() {
            *c = json!((n + more).max(2));
        }
    }
    v
}

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

/// Every coastal species (added to the export's table by `EzData::from_gz`), the
/// variants ([`VARIANTS`]) after them.
pub fn species_list() -> Vec<EzSpecies> {
    let mut list = base_species();
    for (id, of, seed, turn, more) in VARIANTS {
        let b = list.iter().find(|s| s.id == of).cloned().expect("a variant's species");
        list.push(variant(&b, id, seed, turn, more));
    }
    list
}

fn base_species() -> Vec<EzSpecies> {
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
        // (the hill parks' own) A blue gum grown in a dense even-aged stand (Mount
        // Davidson, Sutro, Glen Canyon's grove): a taller clean bole, a narrow crown
        // high up, foliage hanging in long sprays; fuller than the open-grown gum.
        species(
            "coastal-gum-stand",
            "Aspen Large",
            "ash",
            38.0,
            6.5,
            Some(16.0),
            [0.92, 0.88, 0.78],
            [0.44, 0.52, 0.62],
            tree_tiers(),
            json!({
                "seed": 4111,
                "branch": {
                    "levels": 3,
                    "angle": { "1": 26, "2": 42, "3": 52 },
                    "children": { "0": 10, "1": 5, "2": 5 },
                    "length": { "0": 70, "1": 28, "2": 12, "3": 4 },
                    "radius": { "0": 2.4, "1": 0.45, "2": 0.6, "3": 0.8 },
                    "start": { "1": 0.45, "2": 0.25, "3": 0.1 },
                    "gnarliness": { "0": 0.04, "1": 0.2, "2": 0.2, "3": 0.12 },
                    "force": { "direction": { "x": 0, "y": 1, "z": 0 }, "strength": -0.006 },
                    "taper": { "0": 0.75, "1": 0.7, "2": 0.7, "3": 0.7 }
                },
                "leaves": { "billboard": "double", "angle": 22, "count": 12, "start": 0.15, "size": 7.0, "sizeVariance": 0.5 }
            }),
        ),
        // Monterey cypress on the exposed headland (Lands End, Ocean Beach, the
        // park's west windbreaks): "broad-spreading, especially on exposed
        // headlands" (conifers.org), low and flat-topped, the crown swept to one
        // side by the onshore wind (the drawn lean is the planting's: lidar_trees.rs).
        species(
            "coastal-cypress-wind",
            "Oak Large",
            "pine",
            11.0,
            9.0,
            Some(1.4),
            [0.58, 0.50, 0.44],
            [0.32, 0.40, 0.38],
            tree_tiers(),
            json!({
                "seed": 2203,
                "branch": {
                    "levels": 3,
                    "angle": { "1": 70, "2": 68, "3": 62 },
                    "children": { "0": 5, "1": 6, "2": 4 },
                    "length": { "0": 14, "1": 36, "2": 14, "3": 6 },
                    "radius": { "0": 1.6, "1": 0.9, "2": 0.6, "3": 0.9 },
                    "start": { "1": 0.25, "2": 0.25, "3": 0.1 },
                    "gnarliness": { "0": 0.22, "1": 0.28, "2": 0.2, "3": 0.12 },
                    "force": { "direction": { "x": 1, "y": 0.15, "z": 0 }, "strength": 0.03 },
                    "taper": { "0": 0.55, "1": 0.7, "2": 0.7, "3": 0.7 }
                },
                "leaves": { "billboard": "double", "angle": 85, "count": 10, "start": 0.2, "size": 7.0, "sizeVariance": 0.45 }
            }),
        ),
        // An old Monterey pine, its crown "becoming rounded to flattened"
        // (conifers.org): a long bole, a few heavy upturned limbs, a flat top.
        species(
            "coastal-pine-old",
            "Pine Large",
            "pine",
            26.0,
            9.0,
            Some(9.0),
            [0.52, 0.42, 0.36],
            [0.38, 0.44, 0.40],
            tree_tiers(),
            json!({
                "seed": 3307,
                "branch": {
                    "levels": 3,
                    "angle": { "1": 58, "2": 48, "3": 40 },
                    "children": { "0": 7, "1": 5, "2": 4 },
                    "length": { "0": 40, "1": 30, "2": 12, "3": 4 },
                    "radius": { "0": 3.0, "1": 0.6, "2": 0.6, "3": 0.9 },
                    "start": { "1": 0.55, "2": 0.25, "3": 0.1 },
                    "gnarliness": { "0": 0.05, "1": 0.22, "2": 0.15, "3": 0.08 },
                    "force": { "direction": { "x": 0, "y": 1, "z": 0 }, "strength": 0.03 }
                },
                "leaves": { "billboard": "double", "angle": 45, "count": 10, "start": 0.2, "size": 7.0, "sizeVariance": 0.4 }
            }),
        ),
        // Monterey cypress planted in a windbreak row (Golden Gate Park's west
        // end): a taller, narrower crown kept dense by its neighbours.
        species(
            "coastal-cypress-row",
            "Pine Large",
            "pine",
            20.0,
            5.5,
            Some(2.0),
            [0.60, 0.50, 0.42],
            [0.33, 0.41, 0.39],
            tree_tiers(),
            json!({
                "seed": 5519,
                "branch": {
                    "levels": 3,
                    "angle": { "1": 48, "2": 52, "3": 50 },
                    "children": { "0": 10, "1": 5, "2": 4 },
                    "length": { "0": 34, "1": 18, "2": 10, "3": 5 },
                    "radius": { "0": 3.0, "1": 0.6, "2": 0.6, "3": 0.9 },
                    "start": { "1": 0.12, "2": 0.25, "3": 0.1 },
                    "gnarliness": { "0": 0.08, "1": 0.14, "2": 0.14, "3": 0.1 },
                    "force": { "direction": { "x": 0, "y": 1, "z": 0 }, "strength": 0.02 }
                },
                "leaves": { "billboard": "double", "angle": 70, "count": 11, "start": 0.1, "size": 6.5, "sizeVariance": 0.4 }
            }),
        ),
        // Coast live oak (Quercus agrifolia): "a dense, rounded crown" broader than
        // tall in the open, a short trunk parting into crooked limbs, small dark
        // evergreen leaves (Calflora; UC IPM). 12 m tall, 8 m half crown (INFERRED,
        // in the published 10-25 m).
        species(
            "coastal-oak",
            "Oak Large",
            "oak",
            12.0,
            8.0,
            Some(2.0),
            [0.42, 0.38, 0.34],
            [0.30, 0.36, 0.22],
            tree_tiers(),
            json!({
                "seed": 6421,
                "branch": {
                    "levels": 3,
                    "angle": { "1": 58, "2": 55, "3": 52 },
                    "children": { "0": 6, "1": 5, "2": 4 },
                    "length": { "0": 16, "1": 30, "2": 14, "3": 6 },
                    "radius": { "0": 1.8, "1": 0.8, "2": 0.6, "3": 0.9 },
                    "start": { "1": 0.3, "2": 0.25, "3": 0.1 },
                    "gnarliness": { "0": 0.2, "1": 0.3, "2": 0.25, "3": 0.15 },
                    "force": { "direction": { "x": 0, "y": 1, "z": 0 }, "strength": 0.0 },
                    "taper": { "0": 0.6, "1": 0.7, "2": 0.7, "3": 0.7 }
                },
                "leaves": { "billboard": "double", "angle": 70, "count": 18, "start": 0.2, "size": 3.4, "sizeVariance": 0.45 }
            }),
        ),
        // A coast live oak grown from a cut stump or in a grove: several stems from
        // the ground, a lopsided crown (INFERRED: the oaks of McLaren's and Glen
        // Canyon's slopes).
        species(
            "coastal-oak-multi",
            "Oak Medium",
            "oak",
            10.0,
            6.5,
            Some(1.2),
            [0.44, 0.40, 0.35],
            [0.31, 0.37, 0.22],
            tree_tiers(),
            json!({
                "seed": 7717,
                "branch": {
                    "levels": 3,
                    "angle": { "1": 42, "2": 55, "3": 50 },
                    "children": { "0": 4, "1": 6, "2": 4 },
                    "length": { "0": 6, "1": 30, "2": 13, "3": 5 },
                    "radius": { "0": 3.0, "1": 0.9, "2": 0.6, "3": 0.9 },
                    "start": { "1": 0.05, "2": 0.25, "3": 0.1 },
                    "gnarliness": { "0": 0.25, "1": 0.3, "2": 0.25, "3": 0.15 },
                    "force": { "direction": { "x": 0, "y": 1, "z": 0 }, "strength": 0.01 }
                },
                "leaves": { "billboard": "double", "angle": 70, "count": 17, "start": 0.2, "size": 3.2, "sizeVariance": 0.45 }
            }),
        ),
        // Blackwood acacia (Acacia melanoxylon): an invasive "tree to 15 m" with a
        // dense, dark, rounded to conical crown (Cal-IPC), in thickets on the
        // natural areas' slopes. 13 m, 4.5 m half crown (INFERRED).
        species(
            "coastal-acacia",
            "Ash Medium",
            "oak",
            13.0,
            4.5,
            Some(3.0),
            [0.40, 0.36, 0.32],
            [0.28, 0.36, 0.26],
            tree_tiers(),
            json!({
                "seed": 8803,
                "branch": {
                    "levels": 3,
                    "angle": { "1": 44, "2": 50, "3": 50 },
                    "children": { "0": 8, "1": 5, "2": 4 },
                    "length": { "0": 26, "1": 16, "2": 9, "3": 4 },
                    "radius": { "0": 2.4, "1": 0.6, "2": 0.6, "3": 0.9 },
                    "start": { "1": 0.25, "2": 0.25, "3": 0.1 },
                    "gnarliness": { "0": 0.08, "1": 0.2, "2": 0.18, "3": 0.12 },
                    "force": { "direction": { "x": 0, "y": 1, "z": 0 }, "strength": 0.015 }
                },
                "leaves": { "billboard": "double", "angle": 60, "count": 13, "start": 0.15, "size": 4.0, "sizeVariance": 0.4 }
            }),
        ),
        // Arroyo willow (Salix lasiolepis): "a shrub or small tree" to 10 m, many
        // stems from the base, along creeks and seeps (Calflora); light green
        // narrow leaves. 7 m, 4 m half crown (INFERRED).
        species(
            "coastal-willow",
            "Bush 2",
            "aspen",
            7.0,
            4.0,
            None,
            [0.50, 0.44, 0.38],
            [0.46, 0.56, 0.36],
            tree_tiers(),
            json!({
                "seed": 9901,
                "branch": {
                    "levels": 3,
                    "angle": { "1": 28, "2": 40, "3": 45 },
                    "children": { "0": 9, "1": 4, "2": 4 },
                    "length": { "0": 3, "1": 26, "2": 10, "3": 4 },
                    "radius": { "0": 1.8, "1": 0.5, "2": 0.5, "3": 0.8 },
                    "start": { "1": 0.0, "2": 0.3, "3": 0.1 },
                    "gnarliness": { "0": 0.1, "1": 0.18, "2": 0.2, "3": 0.15 },
                    "force": { "direction": { "x": 0, "y": 1, "z": 0 }, "strength": 0.01 }
                },
                "leaves": { "billboard": "double", "angle": 35, "count": 12, "start": 0.3, "size": 3.25, "sizeVariance": 0.45 }
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
            let tris = |t: &str| super::super::build_ez_tree(id, t, 0).map(|m| m.triangles()).unwrap_or(0);
            eprintln!("{id}: {} near, {} coarse, {} distant triangles", m.triangles(), tris("coarse"), tris("distant"));
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
