//! The rear yards' garden shrubs as ez-tree species of our own (`--yard-detail`,
//! eo-props `yards::plants`; drawn only where the yards plant them): the
//! clipped and screening evergreens San Francisco's back gardens are planted
//! with most, grown from the export's bush presets through the same generator
//! as every other species (`build_ez_tree`), cooked with them (`cook.rs`).
//!
//! Which shrubs: INFERRED from street-level imagery of the city's rear yards
//! and the San Francisco Urban Forest Plan's lists; sizes and leaf colours are
//! the species' published descriptions (REF), every branch number INFERRED
//! (the generator's parameters have no published counterpart). Sizes are the
//! shrub at the 1.0 instance scale; the yards scale each to the lidar's height.
//!
//! * Pittosporum (*P. tenuifolium*, *P. eugenioides*): a screen shrub or small
//!   tree, 3-6 m in gardens, dense to the ground, small wavy leaves, mid green.
//!   Drawn 3 m, half crown 1.3 m.
//! * Boxwood, privet, euonymus: clipped hedges and balls, 0.5-3 m, small
//!   glossy dark leaves, the densest canopy. Drawn 1.3 m, half crown 0.9 m.
//! * Rosemary and lavender: the beds' grey Mediterranean shrubs, 0.5-1.5 m,
//!   needle-like leaves, grey-green. Drawn 0.9 m, half crown 0.7 m.
//!
//! The leaf tints are (as every species' `leafTint`) a factor on the leaf atlas
//! cell's own colour, whose mean is a saturated yellow-green (about 0.08-0.13 /
//! 0.18-0.23 / 0.03-0.04 linear: blue a fifth of green): each tint takes it to
//! a mature leaf's reflectance, about 0.07 / 0.125 / 0.056 linear (luminance
//! about 0.11 before the crown's inner shading, inside LOPEX93's 0.07-0.11
//! after it, as `--crown-light`'s), so its blue is over its green, as the
//! street species' tints are (theirs about 0.6 / 0.6 / 1.0). Each species then
//! its own cast: the pittosporum's and rosemary's grey-green, the boxwood's
//! and the conifer's darker. Measured against photographs of the city's
//! gardens and Pittosporum hedges in sun (Wikimedia Commons, CC BY-SA), sunlit
//! foliage reads sRGB green over red 1.1-1.4 and blue at 0.7-0.85 of green,
//! saturation 0.18-0.32; the first garden tints (blue at 0.65 of green on a
//! cell with blue at a fifth of it) drew 0.6-0.8, a lime no garden shows.

use super::{EzSpecies, EzTier};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// The ids, in render order (the props library gives each a model, vegetation_models `V_GARDEN`).
pub const GARDEN_SHRUBS: [&str; 3] = ["garden-pittosporum", "garden-boxwood", "garden-rosemary"];
/// The yards' trees, after the shrubs (vegetation_models `V_GARDEN_TREE`).
/// (Three: the props library holds at most 256 meshes, eo-render props.rs; a grey
/// olive or acacia is the broadleaf under a grey tint, eo-app yards.rs.)
pub const GARDEN_TREES: [&str; 3] = ["garden-tree-broadleaf", "garden-tree-evergreen", "garden-tree-conifer"];

/// The trees' levels, as the coastal trees' (`coastal.rs`): fewer sections and
/// leaf cards further out, each card larger so the crown keeps its cover.
fn tree_tiers() -> BTreeMap<String, EzTier> {
    let tier = |section_stride: f64, segment_factor: f64, leaf_stride: f64, leaf_scale: f64, single: bool| EzTier {
        section_stride: Some(section_stride),
        segment_factor: Some(segment_factor),
        leaf_stride: Some(leaf_stride),
        leaf_scale: Some(leaf_scale),
        billboard: single.then(|| "single".to_string()),
        lift_leaves: false,
    };
    let mut t = BTreeMap::new();
    t.insert("near".into(), tier(2.0, 0.7, 1.0, 1.0, false));
    t.insert("coarse".into(), tier(6.0, 0.4, 3.0, 1.7, true));
    t.insert("distant".into(), tier(12.0, 0.25, 8.0, 2.6, true));
    t
}

fn tiers() -> BTreeMap<String, EzTier> {
    let tier = |section_stride: f64, segment_factor: f64, leaf_stride: f64, leaf_scale: f64, single: bool| EzTier {
        section_stride: Some(section_stride),
        segment_factor: Some(segment_factor),
        leaf_stride: Some(leaf_stride),
        leaf_scale: Some(leaf_scale),
        billboard: single.then(|| "single".to_string()),
        lift_leaves: false,
    };
    let mut t = BTreeMap::new();
    // (the far levels keep more of their leaves than the coastal shrubs' do: a garden's
    // shrubs are its canopy from above, and a sparse billboard reads as bare ground.)
    t.insert("near".into(), tier(3.0, 0.6, 1.0, 1.0, false));
    t.insert("coarse".into(), tier(12.0, 0.4, 5.0, 2.1, true));
    t.insert("distant".into(), tier(24.0, 0.25, 14.0, 3.4, true));
    t
}

#[allow(clippy::too_many_arguments)]
fn species_with(id: &str, preset: &str, leaf: &str, height_m: f64, half_crown_m: f64, bark: [f64; 3], leaf_tint: [f64; 3], tiers: BTreeMap<String, EzTier>, overrides: Value) -> EzSpecies {
    EzSpecies { tiers, ..species(id, preset, leaf, height_m, half_crown_m, bark, leaf_tint, overrides) }
}

fn species(id: &str, preset: &str, leaf: &str, height_m: f64, half_crown_m: f64, bark: [f64; 3], leaf_tint: [f64; 3], overrides: Value) -> EzSpecies {
    EzSpecies {
        id: id.to_string(),
        preset: preset.to_string(),
        leaf: leaf.to_string(),
        height_m,
        max_half_crown_m: half_crown_m,
        clearance_m: None,
        max_height_m: None,
        height_factor: None,
        crown_factor: None,
        facade_reach_m: None,
        street_lean_m: None,
        trunk_bend: None,
        observed_only: false,
        surveyed_only: false,
        crown_cover: true,
        seed_id: None,
        leaf_states: BTreeMap::new(),
        bark_tint: bark,
        leaf_tint,
        tiers: tiers(),
        overrides,
    }
}

/// Every garden species (added to the export's table by `EzData::from_gz`).
pub fn species_list() -> Vec<EzSpecies> {
    vec![
        // The yards' trees (sizes at scale 1; the yards scale each to the lidar's
        // height and crown). A fruit or ornamental broadleaf (plum, apple, maple):
        // a short trunk parting low into a few spreading limbs, an open, irregular crown.
        species_with(
            "garden-tree-broadleaf",
            "Oak Medium",
            "oak",
            7.0,
            3.6,
            [0.55, 0.48, 0.42],
            [0.55, 0.56, 1.2],
            tree_tiers(),
            json!({
                "branch": { "levels": 3, "children": { "0": 5, "1": 4, "2": 3 }, "angle": { "1": 48, "2": 55, "3": 55 }, "start": { "1": 0.22, "2": 0.25, "3": 0.1 }, "gnarliness": { "0": 0.12, "1": 0.2, "2": 0.18, "3": 0.1 } },
                "leaves": { "billboard": "double", "angle": 60, "count": 13, "start": 0.1, "size": 4.2, "sizeVariance": 0.5 }
            }),
        ),
        // A dense evergreen (Victorian box, loquat, lemon): leafy to low on the trunk,
        // a rounded, closed crown.
        species_with(
            "garden-tree-evergreen",
            "Aspen Medium",
            "ash",
            6.0,
            3.0,
            [0.50, 0.45, 0.40],
            [0.58, 0.53, 1.08],
            tree_tiers(),
            json!({
                "branch": { "levels": 3, "children": { "0": 7, "1": 4, "2": 3 }, "angle": { "1": 42, "2": 50, "3": 50 }, "start": { "1": 0.15, "2": 0.2, "3": 0.05 } },
                "leaves": { "billboard": "double", "angle": 55, "count": 11, "start": 0.05, "size": 4.0, "sizeVariance": 0.45 }
            }),
        ),
        // A cypress or juniper: a dense, narrow, dark crown to near the ground.
        species_with(
            "garden-tree-conifer",
            "Pine Medium",
            "pine",
            9.0,
            2.4,
            [0.50, 0.42, 0.36],
            [0.5, 0.59, 1.5],
            tree_tiers(),
            json!({
                "branch": { "start": { "1": 0.08, "2": 0.2, "3": 0.1 } },
                "leaves": { "billboard": "double", "angle": 40, "count": 10, "start": 0.1, "size": 4.0, "sizeVariance": 0.4 }
            }),
        ),
        // Many stems from the ground, rising and leafy to the base: a loose column.
        species(
            "garden-pittosporum",
            "Bush 2",
            "aspen",
            3.0,
            1.3,
            [0.50, 0.46, 0.42],
            [0.8, 0.6, 1.8],
            json!({
                "branch": { "levels": 3, "children": { "0": 9, "1": 4, "2": 3 }, "angle": { "1": 30, "2": 50, "3": 55 }, "start": { "1": 0.0, "2": 0.05, "3": 0.0 }, "sections": { "0": 4, "1": 4, "2": 4, "3": 3 } },
                "leaves": { "billboard": "double", "angle": 55, "count": 7, "start": 0.0, "size": 2.6, "sizeVariance": 0.5 }
            }),
        ),
        // A dense rounded ball of short twiggy stems and small dark leaves.
        species(
            "garden-boxwood",
            "Bush 1",
            "aspen",
            1.3,
            0.9,
            [0.48, 0.42, 0.36],
            [0.66, 0.47, 1.5],
            json!({
                "branch": { "levels": 2, "children": { "0": 14, "1": 7, "2": 3 }, "angle": { "1": 55, "2": 55, "3": 50 }, "start": { "1": 0.0, "2": 0.0, "3": 0.0 }, "sections": { "0": 4, "1": 4, "2": 4, "3": 3 } },
                "leaves": { "billboard": "double", "angle": 65, "count": 9, "start": 0.0, "size": 2.3, "sizeVariance": 0.4 }
            }),
        ),
        // A loose mound of upright stems with needle leaves, grey-green.
        species(
            "garden-rosemary",
            "Bush 3",
            "pine",
            0.9,
            0.7,
            [0.52, 0.48, 0.42],
            [0.7, 0.72, 1.9],
            json!({
                "branch": { "sections": { "0": 4, "1": 4, "2": 4, "3": 3 } },
                "leaves": { "billboard": "double", "angle": 45, "count": 6, "start": 0.05, "size": 2.8, "sizeVariance": 0.5 }
            }),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn garden_species_are_in_the_table_and_build() {
        let d = super::super::EzData::get().unwrap();
        for id in GARDEN_SHRUBS.iter().chain(GARDEN_TREES.iter()) {
            let sp = d.species(id).unwrap();
            for t in ["near", "coarse", "distant"] {
                let m = super::super::build_ez_tree(id, t, 0).unwrap();
                assert!(m.triangles() > 50, "{id} {t}: {} triangles", m.triangles());
                // a garden holds thousands of shrubs: each level within a frame's share
                // (near 10k, coarse 3.5k, distant 2k triangles; the cost probe of 2026-11-25).
                let cap = [10_000, 3_500, 2_000][["near", "coarse", "distant"].iter().position(|x| *x == t).unwrap()];
                assert!(m.triangles() <= cap || GARDEN_TREES.contains(&id) && m.triangles() <= cap * 13 / 10, "{id} {t}: {} triangles over {cap}", m.triangles());
            }
            let m = super::super::build_ez_tree(id, "near", 0).unwrap();
            assert!((m.user.height_m - sp.height_m).abs() < sp.height_m * 0.02, "{id}: {} m against {} m", m.user.height_m, sp.height_m);
            assert!(m.user.half_crown_m <= sp.max_half_crown_m * 1.02, "{id}: half crown {}", m.user.half_crown_m);
        }
    }
}
