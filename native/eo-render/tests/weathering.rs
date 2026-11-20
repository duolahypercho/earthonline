//! Stage W1 (`--grime`): the weathering decals' placement rules on the real
//! street plan (no GPU). The kerb paint (the corner daylighting's red, CVC
//! 22500(n)) and the building-line dirt strips are placed per street tile by
//! deterministic rules keyed on the run and station, like every street decal:
//!
//! * without weathering the tile's decals are exactly those of `place` with it
//!   off (the default frame's records are untouched);
//! * with it, the other decals are the same decals in the same order, and the
//!   new ones are only kerb paint and walk-edge strips;
//! * the same tile gives the same decals twice (deterministic);
//! * a weathering decal that reaches into a neighbouring tile is placed by
//!   that tile too, identically (no seam at a tile border);
//! * each kerb paint run is at most the daylighting length long
//!   (`no-parking-crosswalk-daylighting`, approach side), on a kerb line.

use eo_render::cc0_decals::{DK_KERB_PAINT, DK_WALK_EDGE, Decal, KERB_PAINT_HALF_M, place_with, standards};
use eo_street::{BuildingIndex, StreetCity, StreetOptions, StreetWorld, Terrain};
use std::sync::Arc;

fn same(a: &Decal, b: &Decal) -> bool {
    a.c == b.c && a.a == b.a && a.half == b.half && a.kind == b.kind && a.seed.to_bits() == b.seed.to_bits()
}

fn weathering(d: &Decal) -> bool {
    d.kind == DK_KERB_PAINT || d.kind == DK_WALK_EDGE
}

/// Whether a decal's plan bounding box touches a tile.
fn touches(d: &Decal, key: (i32, i32), tile: f32) -> bool {
    let ex = (d.a[0] * d.half[0]).abs() + (d.a[1] * d.half[1]).abs();
    let ez = (d.a[1] * d.half[0]).abs() + (d.a[0] * d.half[1]).abs();
    let (x0, z0) = (key.0 as f32 * tile, key.1 as f32 * tile);
    d.c[0] + ex >= x0 && d.c[0] - ex <= x0 + tile && d.c[1] + ez >= z0 && d.c[1] - ez <= z0 + tile
}

#[test]
fn weathering_decals_are_deterministic_seamless_and_leave_the_rest() {
    let paths = eo_data::DataPaths::discover().unwrap();
    let city = Arc::new(eo_data::City::load(&paths).unwrap());
    let elev = eo_data::ElevationGrid::load(&paths).unwrap();
    let o = StreetOptions::load().unwrap();
    let lift = o.open_ground_lift;
    let rings: Vec<Vec<[f64; 2]>> = city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
    let sw = StreetWorld::build(o, StreetCity::load().unwrap(), Terrain { grid: Arc::new(elev), carpet_lift: lift }, BuildingIndex::new(rings));
    let tile = eo_street::STREET_TILE_M as f32;
    let day = standards().daylighting_m as f32;
    assert!((day - 6.096).abs() < 1e-3, "caMetric('no-parking-crosswalk-daylighting').approachSide is 20 ft, read {day}");
    // Every tile in a block of downtown and the Mission (dense, with approaches and walks).
    let mut keys: Vec<(i32, i32)> = sw.tiles.keys().copied().collect();
    keys.sort();
    let mid = keys[keys.len() / 2];
    let keys: Vec<(i32, i32)> = keys.into_iter().filter(|k| (k.0 - mid.0).abs() <= 6 && (k.1 - mid.1).abs() <= 6).collect();
    assert!(keys.len() > 40, "{} tiles", keys.len());
    let (mut kerb, mut edge, mut shared) = (0usize, 0usize, 0usize);
    let mut placed: std::collections::HashMap<(i32, i32), Vec<Decal>> = std::collections::HashMap::new();
    for &key in &keys {
        let off = place_with(&sw, key, false);
        let on = place_with(&sw, key, true);
        // the rest untouched: the same decals, in the same order.
        let rest: Vec<&Decal> = on.iter().filter(|d| !weathering(d)).collect();
        assert_eq!(rest.len(), off.len(), "{key:?}");
        assert!(rest.iter().zip(off.iter()).all(|(a, b)| same(a, b)), "{key:?}: a decal other than the weathering ones moved");
        assert!(!off.iter().any(weathering), "{key:?}: weathering decals placed with it off");
        // deterministic.
        let again = place_with(&sw, key, true);
        assert!(again.len() == on.len() && again.iter().zip(on.iter()).all(|(a, b)| same(a, b)), "{key:?}");
        for d in on.iter().filter(|d| weathering(d)) {
            if d.kind == DK_KERB_PAINT {
                kerb += 1;
                assert!(d.half[0] * 2.0 <= day + 1e-3, "{key:?}: kerb paint {} m long, more than the daylighting {day} m", d.half[0] * 2.0);
                assert!((d.half[1] - KERB_PAINT_HALF_M as f32).abs() < 1e-6);
                assert!(d.seed >= 0.0 && d.seed < 1.0, "red is colour code 0");
            } else {
                edge += 1;
            }
            // no seam: every neighbour it reaches places it too.
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let nk = (key.0 + dx, key.1 + dz);
                    if (dx, dz) == (0, 0) || !touches(d, key, tile) || !touches(d, nk, tile) || !sw.tiles.contains_key(&nk) {
                        continue;
                    }
                    let theirs = placed.entry(nk).or_insert_with(|| place_with(&sw, nk, true));
                    assert!(theirs.iter().any(|t| same(t, d)), "{key:?} -> {nk:?}: a weathering decal reaching the neighbour is missing there");
                    shared += 1;
                }
            }
        }
    }
    println!("{} tiles: {kerb} kerb paint runs, {edge} building-line strips, {shared} checked across a tile border", keys.len());
    assert!(kerb > 10 && edge > 50 && shared > 10, "kerb {kerb}, edge {edge}, shared {shared}");
}
