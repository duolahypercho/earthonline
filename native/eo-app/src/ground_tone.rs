//! The terrain's vertex tone and the far land's height, from the open-space
//! ground (eo-landmarks `ground_cover`), the shore field and the land-cover
//! raster: the closures the scene build reads (`SceneInputs::ground_tone`)
//! and the carpet press consults (`far_land`). main.rs builds them here from
//! the loaded inputs.

use eo_data::{ElevationGrid, LandCover};
use eo_landmarks::ground_cover::GroundCover;
use eo_water::WaterData;

/// Ground more than this over the sea, and more than `FAR_LAND_SHORE_M` out
/// from every ring edge, is far land (main.rs, the carpet press).
pub const FAR_LAND_ABOVE_SEA_M: f64 = 2.0;
pub const FAR_LAND_SHORE_M: f64 = 60.0;

/// Sand stops at the beach (`--no-beach-sand-limit`): the steepest ground,
/// rise over run across 12 m of the drawn grid (the slope `ground_tone`
/// measures), that still carries sand. Derived from the 1 m 3DEP DEM through
/// the 8 m elevation grid: the 99th percentile of that slope at the terrain
/// vertices inside the mapped beaches (OSM `natural=beach`), 0.356
/// (`the_sand_limits_match_the_mapped_beaches` re-measures it). The ground
/// plan and the heights are both scaled 1.25, so the slope is the surveyed one.
pub const SAND_SLOPE_MAX: f64 = 0.36;
/// The berm crest: the highest ground over the sea that carries sand,
/// surveyed metres (drawn at the height scale). Derived the same way: the
/// 99th percentile of the height over the sea at the vertices inside the
/// mapped beaches, 15.7 m drawn = 12.55 m surveyed.
pub const BERM_CREST_SURVEYED_M: f64 = 12.5;
/// The walk seaward from a sand vertex that looks for a break of slope
/// (a cliff between it and the sea): its step and its longest reach
/// (`BEACH_SAND_REACH_M`, the farthest sand lies from a mapped beach), metres.
pub const SAND_WALK_STEP_M: f64 = 4.0;
pub const SAND_WALK_MAX_M: f64 = eo_landmarks::ground_cover::BEACH_SAND_REACH_M;

/// The ground's gradient (rise over run) across 12 m at (x, z), as
/// `ground_tone` measures it; None off the grid.
fn slope_at(elev: &ElevationGrid, x: f64, z: f64) -> Option<f64> {
    let h = |dx: f64, dz: f64| elev.sample(x + dx, z + dz);
    match (h(-6.0, 0.0), h(6.0, 0.0), h(0.0, -6.0), h(0.0, 6.0)) {
        (Some(a), Some(b), Some(c), Some(d)) => Some(((b - a).hypot(d - c)) / 12.0),
        _ => None,
    }
}

/// Whether beach sand holds at (x, z): no steeper than `SAND_SLOPE_MAX`, no
/// higher than the berm crest, and no break of slope (ground steeper than
/// that) between it and the sea along the shore field's gradient. On a
/// cliff over a beach, or on the bluff top behind one, the sand stops.
pub fn sand_holds(elev: &ElevationGrid, field: &eo_water::field::ShoreField, x: f64, z: f64) -> bool {
    let Some(y) = elev.sample(x, z) else { return true };
    if (y - eo_water::SEA_LEVEL_Y) / elev.height_scale.max(1e-6) > BERM_CREST_SURVEYED_M {
        return false;
    }
    let (mut px, mut pz) = (x, z);
    let mut walked = 0.0;
    loop {
        if slope_at(elev, px, pz).is_some_and(|s| s > SAND_SLOPE_MAX) {
            return false;
        }
        let s = field.signed_at(px, pz);
        if s >= 0.0 || walked >= SAND_WALK_MAX_M {
            return true;
        }
        // seaward: up the signed distance (positive over the sea)
        let g = [field.signed_at(px + 1.0, pz) - field.signed_at(px - 1.0, pz), field.signed_at(px, pz + 1.0) - field.signed_at(px, pz - 1.0)];
        let gl = g[0].hypot(g[1]);
        if gl < 1e-6 {
            return true;
        }
        px += g[0] / gl * SAND_WALK_STEP_M;
        pz += g[1] / gl * SAND_WALK_STEP_M;
        walked += SAND_WALK_STEP_M;
    }
}

/// The ground where the sand stopped (a cliff or the bluff over a beach):
/// bare earth (tile-ground-cover.js `bareEarth`, the ochre of the bluffs'
/// sandstone and weathered serpentinite) where the measured vegetation share
/// (ground2's NAIP table) is low, coastal scrub (`wildScrub`) where it is
/// high, split per vertex as ground2's `hill_tone` splits dry grass from
/// scrub. Not the hills' straw: a slope too steep for sand is bare, not grass.
/// `bare`: the mapped bare surface there (bare rock), else None for bare earth.
pub fn cliff_tone(share: f64, bare: Option<[f64; 3]>, x: f64, z: f64) -> [f64; 3] {
    use eo_landmarks::ground_cover::named_tone;
    let scrub = named_tone("wildScrub", x, z).unwrap_or([0.08, 0.09, 0.05]);
    let earth = bare.or_else(|| named_tone("bareEarth", x, z)).unwrap_or([0.19, 0.17, 0.12]);
    let level = share * 2.2 + (eo_render::ground2::hash01(x, z, 7) - 0.5) * 0.9;
    let t = ((level - 0.4) / 0.35).clamp(0.0, 1.0);
    let t = t * t * (3.0 - 2.0 * t);
    [0, 1, 2].map(|k| earth[k] + (scrub[k] - earth[k]) * t)
}

/// The measured vegetation share (ground2's NAIP table) at a drawn point; None without it.
fn vegetation_share(x: f64, z: f64) -> Option<f64> {
    let s = eo_data::groundscale::scale();
    eo_render::ground2::grid().and_then(|g| eo_render::ground2::sample(g, x / s, z / s, false))
}

/// A linear colour as the terrain's sRGB vertex bytes (draw.wgsl decodes them
/// with a 2.2 power), with `a` in the alpha byte.
fn pack(c: [f64; 3], a: u8) -> u32 {
    let b = |v: f64| (v.max(0.0).powf(1.0 / 2.2).min(1.0) * 255.0).round() as u32;
    b(c[0]) | (b(c[1]) << 8) | (b(c[2]) << 16) | ((a as u32) << 24)
}

/// The far land's ground height at a point (None where it is not far land):
/// beyond the city's shoreline rings, more than `FAR_LAND_SHORE_M` out from
/// every ring edge and more than `FAR_LAND_ABOVE_SEA_M` over the sea. None
/// everywhere without the ground cover or the shore field.
pub fn far_land<'a>(cover: Option<&'a GroundCover>, water: Option<&'a WaterData>, elev: &'a ElevationGrid) -> impl Fn(f64, f64) -> Option<f64> + Sync + 'a {
    move |x: f64, z: f64| -> Option<f64> {
        cover?;
        let w = water?;
        if w.field.signed_at(x, z) <= FAR_LAND_SHORE_M {
            return None;
        }
        elev.sample(x, z).filter(|h| *h > eo_water::SEA_LEVEL_Y + FAR_LAND_ABOVE_SEA_M)
    }
}

/// A terrain vertex's colour from the open-space ground: beach sand (its
/// shore distance, metres inland, in the alpha byte: 1.5 a metre from 64 m
/// offshore; 255 is no beach) or a green's tone; sRGB bytes as the terrain's
/// vertex colours are. None without the ground cover (the terrain keeps its
/// land-cover tint).
pub fn ground_tone<'a>(cover: Option<&'a GroundCover>, water: Option<&'a WaterData>, elev: &'a ElevationGrid, land: Option<&'a LandCover>) -> impl Fn(f32, f32) -> Option<u32> + Sync + 'a {
    let far_land_fn = far_land(cover, water, elev);
    let sand_limit = eo_landmarks::features::shore_ground("beach-sand-limit");
    move |x: f32, z: f32| -> Option<u32> {
        let cover = cover?;
        let (x, z) = (x as f64, z as f64);
        // the far land's ground by its land-cover class (tile-ground-cover.js
        // `DERIVED_TONES`: unmapped open ground, wild grass or coastal scrub; under trees understorey).
        if far_land_fn(x, z).is_some() {
            let class = land.map(|l| l.class_at(x, z)).unwrap_or(0);
            return Some(pack(eo_landmarks::ground_cover::far_land_tone(class, x, z), 255));
        }
        let slope = slope_at(elev, x, z).unwrap_or(0.0);
        if let Some(w) = water {
            let s = w.field.signed_at(x, z);
            if cover.beach_sand(x, z, s.abs()) {
                if !sand_limit || sand_holds(elev, &w.field, x, z) {
                    let a = ((64.0 - s) * 1.5).round().clamp(0.0, 253.0) as u8;
                    return Some(pack(cover.sand_at(x, z), a));
                }
            }
            // a cliff over a beach (steeper than sand holds, within the sand's
            // reach of a mapped beach, at any height): bare ground and scrub,
            // a mapped bare surface (bare rock) its bare ground. Flat ground
            // over the berm or behind the cliff takes the open ground's tone
            // below, as inland.
            if sand_limit && slope > SAND_SLOPE_MAX && s < 0.0 && cover.near_mapped_beach(x, z) {
                let bare = cover.tone_name_at(x, z).filter(|n| matches!(*n, "bareRock" | "bareEarth")).and_then(|_| cover.tone_at(x, z));
                return Some(pack(cliff_tone(vegetation_share(x, z).unwrap_or(0.3), bare, x, z), 255));
            }
        }
        // the open ground's measured green (yards, lots, hills), inland of the shore.
        let park = cover.tone_name_at(x, z).and_then(|n| cover.tone_at(x, z).map(|c| (n, c)));
        let class = land.map(|l| l.class_at(x, z)).unwrap_or(0);
        let shore = water.map(|w| w.field.signed_at(x, z).abs());
        if let Some(c) = eo_render::ground2::open_ground(x, z, class, park, shore, elev.sample(x, z).map(|h| h - eo_water::SEA_LEVEL_Y), slope) {
            return Some(pack(c, 255));
        }
        cover.tone_at(x, z).map(|c| pack(c, 255))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sand limits are the mapped beaches' own 99th percentiles of slope
    /// and height over the sea (the derivation `SAND_SLOPE_MAX` and
    /// `BERM_CREST_SURVEYED_M` state), at the drawn 1.25 plan and height
    /// scales; skipped where the city data is not checked out.
    #[test]
    fn the_sand_limits_match_the_mapped_beaches() {
        let Ok(paths) = eo_data::DataPaths::discover() else { return };
        let (Ok(elev), Ok(cover), Ok(water)) = (eo_data::ElevationGrid::load(&paths), eo_landmarks::ground_cover::GroundCover::load(&paths.repo_root), eo_water::load(&paths.repo_root, None)) else { return };
        let (mut slopes, mut heights) = (vec![], vec![]);
        let cell = elev.meta.cell_size;
        for j in 0..elev.meta.height {
            for i in 0..elev.meta.width {
                let x = elev.meta.origin_x + (i as f64 + 0.5) * cell;
                let z = elev.meta.origin_z + (j as f64 + 0.5) * cell;
                if water.field.signed_at(x, z) > 0.0 || !cover.in_mapped_beach(x, z) {
                    continue;
                }
                if let (Some(sl), Some(y)) = (slope_at(&elev, x, z), elev.sample(x, z)) {
                    slopes.push(sl);
                    heights.push((y - eo_water::SEA_LEVEL_Y) / elev.height_scale);
                }
            }
        }
        assert!(slopes.len() > 5000, "{} beach vertices", slopes.len());
        let p99 = |v: &mut Vec<f64>| {
            v.sort_by(|a, b| a.total_cmp(b));
            v[(v.len() - 1) * 99 / 100]
        };
        let (s99, h99) = (p99(&mut slopes), p99(&mut heights));
        assert!((s99 - SAND_SLOPE_MAX).abs() < 0.02, "slope p99 {s99}");
        assert!((h99 - BERM_CREST_SURVEYED_M).abs() < 0.5, "height p99 {h99} m surveyed");
        // drawn: the berm at the 1.25 height scale (the approved deviation)
        assert!((BERM_CREST_SURVEYED_M * 1.25 - 15.625).abs() < 1e-9);
    }

    #[test]
    fn the_cliff_runs_from_bare_earth_to_scrub_with_the_share() {
        let luma = |c: [f64; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        let (gentle, steep) = (cliff_tone(0.9, None, 4.0, 4.0), cliff_tone(0.0, None, 4.0, 4.0));
        assert!(gentle[1] > gentle[0], "scrub is green: {gentle:?}");
        assert!(steep[0] > steep[2] && luma(steep) > luma(gentle), "earth is warm and lighter: {steep:?}");
    }

    #[test]
    fn pack_puts_srgb_bytes_and_alpha_in_place() {
        assert_eq!(pack([0.0, 0.0, 0.0], 0), 0);
        assert_eq!(pack([1.0, 1.0, 1.0], 255), 0xffff_ffff);
        // linear 0.5 is sRGB-ish 0.5^(1/2.2) = 0.7297 -> 186
        assert_eq!(pack([0.5, 0.0, 2.0], 7), 186 | (255 << 16) | (7 << 24));
    }
}
