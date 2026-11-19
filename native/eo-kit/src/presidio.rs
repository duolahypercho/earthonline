//! The Presidio and Golden Gate National Recreation Area rule: the federal
//! post's buildings in their documented finishes instead of the city's
//! street palette.
//!
//! Sources:
//! - NPS, "Mission Revival Architecture" (nps.gov/articles/mission-revival-architecture.htm):
//!   from Fort Winfield Scott (1912) on, "large flat stucco surfaces" and
//!   gable and hip roofs "typically sheathed in striking red clay tile"; by the
//!   1930s the Army "repainted most of their older buildings off-white and
//!   replaced dark roofing material with new red clay tiles".
//! - NPS / NOAA, Fort Point Coast Guard Station Cultural Landscape Report
//!   (FtPtStaCLR, linked from OSM way 288365549): the Presidio's buildings
//!   "are white wood-frame and stucco or red brick with shingle or tile
//!   roofs"; the station is wood-frame "clad in wood shingles" under a
//!   gambrel roof, its outbuilding's shingled hip roof "now painted red".
//! - OpenStreetMap (ODbL, (c) OpenStreetMap contributors): the boundary of
//!   relation 8346137 "Presidio of San Francisco" as Nominatim simplifies it
//!   (0.001 degree), and the Warming Hut, way 32777524 (`roof:shape=gabled`,
//!   `roof:colour=red`).
//!
//! The rule applies to a record whose footprint centroid is inside the
//! boundary and that has no assessor parcel (no civic join): the federal
//! post has none, so the Sea Cliff and Marina lots the simplified ring grazes
//! keep the city's rule. What it does (`apply`, from `pipeline::drawn_materials`):
//! - a painted wall finish is painted off-white or cream (`PALETTE`), on the
//!   neutral primer as the city's paint is; brick and concrete stay as planned
//!   (the post's red brick barracks);
//! - a flat roof of a building of at most three storeys takes red clay tile
//!   (large flat-roofed warehouses and civic blocks keep their membrane);
//! - the Warming Hut and the old Coast Guard station take their documented
//!   finishes (`OVERRIDES`).
//! Not done: pitched roof geometry. The records carry no LiDAR roof peak
//! (no assessor join) and no `roof:shape` tag, so the planner's roof forms
//! stay as they are; the tile reads on the flat cap.
//!
//! Switch: `--no-presidio-rule` (or the roll-up `--no-gg2-far`), and the rule
//! is off whenever the wall paint is (`--no-wall-paint`).

use crate::pipeline::{FENESTRATED_SUFFIX, Tints};
use crate::rng::hash_seed;
use std::collections::BTreeMap;

/// Bump when the boundary, palette or rule changes (part of the kit cache key).
pub const VERSION: u32 = 2;

/// OSM relation 8346137 (Presidio of San Francisco), Nominatim's 0.001
/// degree simplification: (lon, lat), closed.
pub const BOUNDARY_LON_LAT: [(f64, f64); 12] = [
    (-122.4858751, 37.7908179),
    (-122.4837596, 37.7891148),
    (-122.4846686, 37.7874077),
    (-122.4745198, 37.7868011),
    (-122.4464893, 37.7921443),
    (-122.4479338, 37.8014334),
    (-122.4498345, 37.8028324),
    (-122.4487136, 37.8067428),
    (-122.4632009, 37.8050656),
    (-122.4778038, 37.8110005),
    (-122.4793373, 37.8034716),
    (-122.4858751, 37.7908179),
];

/// The off-white and cream the post was repainted in (sRGB albedo targets,
/// weights). INFERRED: the sources name the colours ("off-white", "white",
/// cream stucco) but give no numbers; these are pale near-neutrals chosen to
/// read as those words (OKLab L above 0.88, a slight warm cast for cream and
/// weathering) and to fit under the kit's primer, as the city's paint does.
pub const PALETTE: [([u8; 3], f32); 4] = [
    ([238, 236, 228], 4.0), // off-white
    ([234, 228, 212], 2.5), // cream
    ([241, 240, 236], 1.5), // white
    ([222, 219, 208], 1.0), // weathered off-white
];

/// The red roof: the kit's clay tile.
pub const RED_TILE: &str = "clay-barrel-tile";
/// The white shingle and clapboard finishes of the overrides.
pub const SHINGLE: &str = "painted-shingle";
pub const CLAPBOARD: &str = "painted-clapboard";

/// Archetypes whose flat roof keeps its membrane (warehouses, civic blocks, towers).
const FLAT_KEPT: [&str; 4] = ["soma-warehouse", "civic-concrete", "industrial-shed", "downtown-tower"];
/// The tallest a building takes the tile on its flat cap.
const TILE_MAX_STOREYS: f64 = 3.0;

/// A record with documented finishes: (record id, wall finish, why).
pub const OVERRIDES: [(&str, &str, &str); 2] = [
    ("sf-building-32777524", CLAPBOARD, "Warming Hut (OSM way 32777524: roof:shape=gabled, roof:colour=red); a 1909 storehouse, white painted walls"),
    ("sf-building-288365549", SHINGLE, "Fort Point Coast Guard Station, 1915 (OSM way 288365549; the CLR: wood-frame clad in wood shingles, shingled roof painted red)"),
];

static PINNED: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Tests pin the rule (as `wall_paint::pin`).
pub fn pin(on: bool) {
    PINNED.store(if on { 2 } else { 1 }, std::sync::atomic::Ordering::Relaxed);
}

/// The rule is on: not switched off, and the wall paint is on.
pub fn enabled() -> bool {
    let on = match PINNED.load(std::sync::atomic::Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            static ARGS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
            *ARGS.get_or_init(|| !std::env::args().any(|a| a == "--no-presidio-rule" || a == "--no-gg2-far"))
        }
    };
    on && crate::wall_paint::enabled()
}

/// The kit cache key's part (empty when off: the key as before).
pub fn cache_tag() -> String {
    if enabled() { format!("+presidio-v{VERSION}") } else { String::new() }
}

/// Whether plan point (x, z) is inside `ring` (even-odd).
pub fn inside(ring: &[[f64; 2]], x: f64, z: f64) -> bool {
    let n = ring.len();
    if n < 3 {
        return false;
    }
    let mut c = false;
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (ring[i], ring[j]);
        if (a[1] > z) != (b[1] > z) && x < (b[0] - a[0]) * (z - a[1]) / (b[1] - a[1]) + a[0] {
            c = !c;
        }
        j = i;
    }
    c
}

/// Whether a record is on the post: listed in `OVERRIDES`, or its footprint
/// centroid (plan x, z) is inside `ring`. The listed records are taken by id
/// because the simplified ring is coarse at the shore: the Warming Hut's
/// centroid falls about 19 m (plan) north of the ring's edge from
/// (-122.4632, 37.8051) to (-122.4778, 37.8110), on the Crissy Field shore
/// the relation itself includes.
pub fn on_post(ring: &[[f64; 2]], record_id: &str, x: f64, z: f64) -> bool {
    OVERRIDES.iter().any(|o| o.0 == record_id) || inside(ring, x, z)
}

/// The boundary on the drawn ground plan (game x, z) of `city`.
pub fn boundary(meta: &eo_data::CityMeta) -> Vec<[f64; 2]> {
    let gf = eo_data::geo::GeoFrame::of_city(meta);
    BOUNDARY_LON_LAT.iter().map(|&(lon, lat)| gf.plan_xz(lat, lon)).collect()
}

fn lin(c: [u8; 3]) -> [f32; 3] {
    eo_data::roof_colours::rgb8_linear(c)
}

fn unit(seed: u32, salt: &str) -> f32 {
    (hash_seed(&format!("{seed}|presidio|{salt}")) % 1_000_003) as f32 / 1_000_003.0
}

/// One building's paint: a weighted draw from `PALETTE`, its value moved by
/// -8 % to 0 % (weathering, as the city's paint moves).
pub fn pick(seed: u32) -> [f32; 3] {
    let total: f32 = PALETTE.iter().map(|e| e.1).sum();
    let mut at = unit(seed, "pick") * total;
    let mut c = PALETTE[PALETTE.len() - 1].0;
    for e in &PALETTE {
        at -= e.1;
        if at < 0.0 {
            c = e.0;
            break;
        }
    }
    let v = 0.92 + 0.08 * unit(seed, "value");
    lin(c).map(|x| x * v)
}

fn on_primer(target: [f32; 3]) -> u32 {
    let p = lin(crate::wall_paint::PRIMER_SRGB);
    let b = [0, 1, 2].map(|k| (eo_data::roof_colours::linear_to_srgb((target[k] / p[k]).clamp(0.0, 1.0)) * 255.0).round() as u32);
    (b[0] << 16) | (b[1] << 8) | b[2]
}

/// The rule on one building's drawn materials (`presidio`: the record is on
/// the post). Returns whether the roof slot was replaced (the roof finish is then skipped).
pub fn apply(presidio: bool, record_id: &str, archetype: &str, storeys: f64, seed: u32, materials: &mut BTreeMap<String, String>, tints: &mut Tints) -> bool {
    if !presidio || !enabled() {
        return false;
    }
    let paint = on_primer(pick(seed));
    // the documented wall finish, on every wall slot that drew the planned wall
    if let Some((_, finish, _)) = OVERRIDES.iter().find(|o| o.0 == record_id) {
        let old = materials.get("wall").cloned();
        if let Some(old) = old {
            let fen = format!("{old}{FENESTRATED_SUFFIX}");
            for v in materials.values_mut() {
                if *v == old {
                    *v = finish.to_string();
                } else if *v == fen {
                    *v = format!("{finish}{FENESTRATED_SUFFIX}");
                }
            }
        }
    }
    // off-white on every painted finish (brick, block and concrete stay)
    let painted: Vec<String> = materials.values().filter(|n| crate::wall_paint::finish_colour(n).is_some()).map(|n| crate::pipeline::base_material_name(n).to_string()).collect();
    for base in painted {
        tints.insert(format!("{base}{FENESTRATED_SUFFIX}"), paint);
        tints.insert(base, paint);
    }
    // the red tile roof on the low buildings' flat roof
    let documented = OVERRIDES.iter().any(|o| o.0 == record_id);
    let tile = materials.contains_key("roof") && (documented || (!FLAT_KEPT.contains(&archetype) && storeys <= TILE_MAX_STOREYS));
    if tile {
        materials.insert("roof".into(), RED_TILE.into());
    }
    tile
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_palette_fits_under_the_primer_and_is_pale() {
        for (c, _) in PALETTE {
            for k in 0..3 {
                assert!(c[k] <= crate::wall_paint::PRIMER_SRGB[k], "{c:?}");
            }
            assert!(eo_data::roof_colours::oklab(lin(c))[0] > 0.88, "{c:?} is not off-white");
        }
    }

    #[test]
    fn the_ring_holds_the_main_post_and_not_the_marina() {
        // a plain equirectangular frame round the city centre is enough to test the ring
        let ring: Vec<[f64; 2]> = BOUNDARY_LON_LAT.iter().map(|&(lon, lat)| [lon, -lat]).collect();
        // Main Post parade ground, about 37.7990 N 122.4590 W
        assert!(inside(&ring, -122.4590, -37.7990));
        // Fort Winfield Scott, about 37.8030 N 122.4760 W
        assert!(inside(&ring, -122.4760, -37.8030));
        // the Marina Green, 37.8065 N 122.4400 W, and Sea Cliff, 37.7860 N 122.4880 W
        assert!(!inside(&ring, -122.4400, -37.8065));
        assert!(!inside(&ring, -122.4880, -37.7860));
    }

    #[test]
    fn off_the_post_nothing_changes() {
        pin(true);
        let mut m: BTreeMap<String, String> = [("wall".to_string(), "painted-stucco".to_string()), ("roof".to_string(), "membrane-roof".to_string())].into();
        let mut t = Tints::new();
        let (m0, t0) = (m.clone(), t.clone());
        assert!(!apply(false, "sf-building-1", "bay-window-apartment", 2.0, 7, &mut m, &mut t));
        assert_eq!((m, t), (m0, t0));
    }

    #[test]
    fn on_the_post_walls_turn_off_white_and_low_roofs_red_tile() {
        crate::wall_paint::pin(true);
        pin(true);
        assert!(enabled());
        let mut m: BTreeMap<String, String> = [("wall".to_string(), "painted-stucco".to_string()), ("roof".to_string(), "membrane-roof".to_string())].into();
        let mut t = Tints::new();
        assert!(apply(true, "sf-building-1", "bay-window-apartment", 2.0, 7, &mut m, &mut t));
        assert_eq!(m["roof"], RED_TILE);
        assert!(t.contains_key("painted-stucco"));
        let mut w: BTreeMap<String, String> = [("wall".to_string(), "brick-running-bond".to_string()), ("roof".to_string(), "membrane-roof".to_string())].into();
        let mut tw = Tints::new();
        assert!(!apply(true, "sf-building-2", "soma-warehouse", 2.0, 7, &mut w, &mut tw), "a warehouse keeps its membrane");
        assert_eq!(w["wall"], "brick-running-bond");
        assert!(tw.is_empty(), "brick is not painted");
        let mut h: BTreeMap<String, String> = [("wall".to_string(), "painted-stucco".to_string()), ("roof".to_string(), "membrane-roof".to_string())].into();
        let mut th = Tints::new();
        assert!(apply(true, "sf-building-32777524", "retail-strip", 2.0, 7, &mut h, &mut th));
        assert_eq!(h["wall"], CLAPBOARD);
        assert_eq!(h["roof"], RED_TILE);
    }
}
