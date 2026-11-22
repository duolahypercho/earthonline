//! Stage W2b (`--street-life`): the street-life decals' placement on the
//! real street plan, with the real land-use survey (no GPU).
//!
//! * level 0 places nothing: a tile's records are exactly the ones before;
//! * with it, the decals before are untouched, in the same order, and keep
//!   their places in the packed records and in every cell of the grid (the
//!   street-life decals sort after them);
//! * the same tile gives the same decals twice (deterministic), and a decal
//!   that reaches a neighbouring tile is placed there identically (no seam);
//! * each life decal lies on its run: from just off the gutter to past the
//!   building line, its +across toward the buildings, at most a stretch long;
//! * the densities follow the land use (no graffiti on civic buildings,
//!   more on industrial fronts than on shops, on shops than on homes);
//! * (stage W2c) each seed is its anchors' codes, an exact integer, and the
//!   piles' anchors are found: storm drains at the kerb, bins at stops and
//!   corners.

use eo_render::cc0_decals::{Decal, Hotspots, pack, place_with};
use eo_render::street_life::{self, Context, DK_LIFE, KIND_SHIFT, LandUse, Params, Trees};
use eo_street::{BuildingIndex, StreetCity, StreetOptions, StreetWorld, Terrain};
use std::collections::HashMap;
use std::sync::Arc;

fn same(a: &Decal, b: &Decal) -> bool {
    a.c == b.c && a.a == b.a && a.half == b.half && a.kind == b.kind && a.seed.to_bits() == b.seed.to_bits()
}

fn touches(d: &Decal, key: (i32, i32), tile: f32) -> bool {
    let ex = (d.a[0] * d.half[0]).abs() + (d.a[1] * d.half[1]).abs();
    let ez = (d.a[1] * d.half[0]).abs() + (d.a[0] * d.half[1]).abs();
    let (x0, z0) = (key.0 as f32 * tile, key.1 as f32 * tile);
    d.c[0] + ex >= x0 && d.c[0] - ex <= x0 + tile && d.c[1] + ez >= z0 && d.c[1] - ez <= z0 + tile
}

/// The building-metadata packs' `use` by id (as eo-app street_life.rs reads them).
fn uses(repo: &std::path::Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for base in ["public/data/sf/building-metadata/v1", "public/data/sf/building-metadata/v1-pack"] {
        let dir = repo.join(base);
        let Ok(text) = std::fs::read(dir.join("index.json")) else { continue };
        let index: serde_json::Value = serde_json::from_slice(&text).unwrap();
        let mut files: Vec<String> = index["tiles"].as_object().unwrap().values().map(|t| t["file"].as_str().unwrap().to_string()).collect();
        files.sort();
        let parts: Vec<Vec<(String, String)>> = std::thread::scope(|s| {
            let hs: Vec<_> = files
                .chunks(files.len().div_ceil(8).max(1))
                .map(|c| {
                    let dir = &dir;
                    s.spawn(move || {
                        let mut v = vec![];
                        for f in c {
                            let t: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join(f)).unwrap()).unwrap();
                            let mut recs: Vec<(String, String)> = t["buildings"].as_object().map(|m| m.iter().filter_map(|(id, r)| r["use"].as_str().map(|u| (id.clone(), u.to_string()))).collect()).unwrap_or_default();
                            recs.sort();
                            v.extend(recs);
                        }
                        v
                    })
                })
                .collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        for (id, u) in parts.into_iter().flatten() {
            out.entry(id).or_insert(u);
        }
    }
    out
}

/// The surveyed storm-drain inlets as eo-app streets.rs `cc0_survey` hands them
/// over (DataSF, on the scaled plan), set before the hotspots are first read.
fn set_drains(city: &eo_data::City, paths: &eo_data::DataPaths) -> usize {
    let frame = eo_data::Frame::from_meta(&city.meta);
    let drains: Vec<[f64; 2]> = eo_data::read_maybe_gzip(&paths.sf("sf-storm-drains.json.gz"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|v| v["drains"].as_array().cloned())
        .map(|rows| {
            rows.iter()
                .filter_map(|r| {
                    let (lat, lon) = (r.get(0)?.as_f64()?, r.get(1)?.as_f64()?);
                    Some(eo_data::groundscale::plan([(lon - frame.center_lon) * frame.m_per_deg_lon, -(lat - frame.center_lat) * frame.m_per_deg_lat]))
                })
                .collect()
        })
        .unwrap_or_default();
    let n = drains.len();
    eo_render::cc0_decals::set_points(vec![], drains);
    n
}

#[test]
fn street_life_decals_are_deterministic_seamless_and_by_land_use() {
    let paths = eo_data::DataPaths::discover().unwrap();
    let city = Arc::new(eo_data::City::load(&paths).unwrap());
    let drains_surveyed = set_drains(&city, &paths);
    assert!(drains_surveyed > 1000, "the storm-drain survey ({drains_surveyed} inlets)");
    let elev = eo_data::ElevationGrid::load(&paths).unwrap();
    let o = StreetOptions::load().unwrap();
    let lift = o.open_ground_lift;
    let rings: Vec<Vec<[f64; 2]>> = city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
    let sw = StreetWorld::build(o, StreetCity::load().unwrap(), Terrain { grid: Arc::new(elev), carpet_lift: lift }, BuildingIndex::new(rings));
    let uses = uses(&paths.repo_root);
    let classes: Vec<u8> = city
        .buildings
        .iter()
        .map(|b| uses.get(&b.id).and_then(|u| LandUse::from_metadata(u)).unwrap_or_else(|| LandUse::from_osm(&b.tags.building, &b.tags.amenity)) as u8)
        .collect();
    let classed = classes.iter().filter(|&&c| c != 0).count();
    println!("{} building uses read; {classed} of {} buildings classed", uses.len(), classes.len());
    // the packs cover the extract's records (the DataSF footprint pack's mostly not): the rest by their OSM tags
    assert!(classed > 25_000, "the extract's buildings have a land use ({classed} of {})", classes.len());
    let trees = Trees::get();
    assert!(trees.count > 10_000, "the street tree list ({} trees)", trees.count);
    let cx = Context { level: 1, classes: &classes, structures: None, trees, hot: Hotspots::get() };
    let off = Context { level: 0, classes: &classes, structures: None, trees, hot: Hotspots::get() };
    let tile = eo_street::STREET_TILE_M as f32;
    let mut keys: Vec<(i32, i32)> = sw.tiles.keys().copied().collect();
    keys.sort();
    let mid = keys[keys.len() / 2];
    let block: Vec<(i32, i32)> = keys.iter().copied().filter(|k| (k.0 - mid.0).abs() <= 5 && (k.1 - mid.1).abs() <= 5).collect();
    assert!(block.len() > 40, "{} tiles", block.len());
    let (mut life, mut shared, mut dropped_life, mut max_records) = (0usize, 0usize, 0usize, 0usize);
    let (mut drains, mut bins) = (0usize, 0usize);
    let mut placed: HashMap<(i32, i32), Vec<Decal>> = HashMap::new();
    for &key in &block {
        let before = place_with(&sw, key, false);
        let mut none = before.clone();
        street_life::place(&sw, key, &off, &mut none);
        assert!(none.len() == before.len(), "{key:?}: level 0 placed street life");
        let mut on = before.clone();
        street_life::place(&sw, key, &cx, &mut on);
        assert!(on[..before.len()].iter().zip(before.iter()).all(|(a, b)| same(a, b)), "{key:?}: the decals before moved");
        // deterministic
        let mut again = before.clone();
        street_life::place(&sw, key, &cx, &mut again);
        assert!(again.len() == on.len() && again.iter().zip(on.iter()).all(|(a, b)| same(a, b)), "{key:?}");
        // packed: the decals before keep their records and their cells exactly; the
        // street life's are listed in cells of their own
        let pb = pack(key, tile, before.clone());
        let pa = pack(key, tile, on.clone());
        assert_eq!(&pa.records[..pb.records.len()], &pb.records[..], "{key:?}: a record before moved");
        assert_eq!(pa.cells, pb.cells, "{key:?}: the wear decals' cells changed");
        assert!(pb.life_cells.is_empty(), "{key:?}: life cells without street life");
        dropped_life += pa.dropped - pb.dropped;
        max_records = max_records.max(pa.count);
        for d in &on[before.len()..] {
            assert_eq!(d.kind & 255, DK_LIFE, "no column without structures");
            life += 1;
            assert!(d.half[0] <= (street_life::STRETCH_MAX_M / 2.0 + street_life::PAD_M) as f32 + 1e-3, "{key:?}: {} m half length", d.half[0]);
            assert!(d.half[1] > 0.5 && d.half[1] < 12.0, "{key:?}: {} m half width", d.half[1]);
            assert!((d.a[0].hypot(d.a[1]) - 1.0).abs() < 1e-4);
            // the seed: its anchors' 4-bit codes (trees, drain, bin), an exact integer below 2^16
            assert!(d.seed >= 0.0 && d.seed < 65536.0 && d.seed.floor() == d.seed, "{key:?}: seed {}", d.seed);
            let codes = street_life::seed_codes(d.seed);
            assert_eq!(street_life::seed_of(codes).to_bits(), d.seed.to_bits());
            drains += (codes[2] > 0) as usize;
            bins += (codes[3] > 0) as usize;
            // no seam: every neighbour it reaches places it too
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let nk = (key.0 + dx, key.1 + dz);
                    if (dx, dz) == (0, 0) || !touches(d, key, tile) || !touches(d, nk, tile) || !sw.tiles.contains_key(&nk) {
                        continue;
                    }
                    let theirs = placed.entry(nk).or_insert_with(|| {
                        let mut v = place_with(&sw, nk, false);
                        street_life::place(&sw, nk, &cx, &mut v);
                        v
                    });
                    assert!(theirs.iter().any(|t| same(t, d)), "{key:?} -> {nk:?}: a life decal reaching the neighbour is missing there");
                    shared += 1;
                }
            }
        }
    }
    println!("{} tiles: {life} life decals, {shared} checked across a tile border, {dropped_life} of their cell entries dropped (cells full), at most {max_records} records a tile", block.len());
    // stage W2c: the piles' anchors, a surveyed storm drain at the kerb and a bin at a stop or a corner
    println!("anchors: {drains} stretch sides with a drain's pile, {bins} with a bin's");
    assert!(drains > 20 && bins > 50 && drains < life && bins < life, "drains {drains}, bins {bins} of {life}");
    // with the weathering decals too (`--grime`): how many cell entries of the street life drop
    let (mut d_w, mut n_w) = (0usize, 0usize);
    for &key in &block {
        let before = place_with(&sw, key, true);
        let mut on = before.clone();
        street_life::place(&sw, key, &cx, &mut on);
        let (pb, pa) = (pack(key, tile, before.clone()), pack(key, tile, on.clone()));
        d_w += pa.dropped - pb.dropped;
        n_w += pa.count - pb.count;
        assert_eq!(pa.cells, pb.cells, "{key:?}: the wear decals' cells changed (weathering on)");
    }
    println!("with the weathering decals: {n_w} life decals in their tiles, {d_w} of their cell entries dropped");
    assert!(d_w * 200 < n_w * 8, "{d_w} dropped");
    assert!(life > 500 && shared > 50, "life {life}, shared {shared}");
    assert!(dropped_life * 200 < life, "next to no street-life entry dropped from a full cell: {dropped_life} of {life} decals");
    assert!(max_records <= eo_render::cc0_decals::MAX_DECALS, "{max_records}");

    // +across points toward the buildings: the decal's inner edge (its middle) on a
    // carriageway with the outer edge off it (a building, the walk) is the rule; the
    // other way round (a flipped decal) is rare (a run whose walk backs on another street).
    let (mut right, mut flipped) = (0, 0);
    for &key in block.iter().take(12) {
        let mut v = vec![];
        street_life::place(&sw, key, &cx, &mut v);
        for d in &v {
            let px = [-d.a[1], d.a[0]];
            let inner = [(d.c[0] - px[0] * (d.half[1] - 0.2)) as f64, (d.c[1] - px[1] * (d.half[1] - 0.2)) as f64];
            let outer = [(d.c[0] + px[0] * (d.half[1] - 0.2)) as f64, (d.c[1] + px[1] * (d.half[1] - 0.2)) as f64];
            let road = |p: [f64; 2]| sw.paved_layer_at(p[0], p[1]) == Some(eo_street::Layer::Carriageway) && sw.buildings.at(p[0], p[1]).is_none();
            match (road(inner), road(outer)) {
                (true, false) => right += 1,
                (false, true) => flipped += 1,
                _ => {}
            }
        }
    }
    println!("orientation: {right} decals from the carriageway to off it, {flipped} the other way");
    assert!(right > 50 && flipped * 10 < right, "{right} right, {flipped} flipped");

    // Densities by land use over a city-wide sample (every 5th tile).
    let mut by: HashMap<u8, (usize, [u64; 5])> = HashMap::new();
    let mut deck = 0;
    for key in keys.iter().step_by(5) {
        let mut v = vec![];
        street_life::place(&sw, *key, &cx, &mut v);
        for d in v.iter().filter(|d| touches(d, *key, tile)) {
            let p = Params::unpack(d.kind >> KIND_SHIFT);
            deck += p.deck as usize;
            // the land use the placement read: the building at the stretch's building line
            let px = [-d.a[1], d.a[0]];
            let back = [(d.c[0] + px[0] * (d.half[1] - street_life::WALL_REACH_M as f32 + 0.15)) as f64, (d.c[1] + px[1] * (d.half[1] - street_life::WALL_REACH_M as f32 + 0.15)) as f64];
            let lu = sw.buildings.at(back[0], back[1]).map(|b| classes[b as usize]).unwrap_or(255);
            let e = by.entry(lu).or_default();
            e.0 += 1;
            for (i, v) in [p.tags, p.posters, p.litter, p.leaves, p.grit].iter().enumerate() {
                e.1[i] += *v as u64;
            }
        }
    }
    let mean = |lu: u8, i: usize| by.get(&lu).map(|e| e.1[i] as f64 / e.0.max(1) as f64).unwrap_or(0.0);
    let name = |lu: u8| if lu == 255 { "no wall".to_string() } else { format!("{:?}", LandUse::from_u8(lu)) };
    let mut lus: Vec<u8> = by.keys().copied().collect();
    lus.sort();
    println!("densities by land use (mean 0..7; grit 0..3), stretch sides:");
    for lu in lus {
        println!("  {:>10}: {:>6} sides  tags {:.2}  posters {:.2}  litter {:.2}  leaves {:.2}  grit {:.2}", name(lu), by[&lu].0, mean(lu, 0), mean(lu, 1), mean(lu, 2), mean(lu, 3), mean(lu, 4));
    }
    assert_eq!(deck, 0, "no deck without structures");
    let civic = LandUse::Civic as u8;
    assert_eq!(by.get(&civic).map(|e| e.1[0]).unwrap_or(0), 0, "no graffiti on civic buildings");
    assert!(mean(LandUse::Industrial as u8, 0) > mean(LandUse::Retail as u8, 0), "industrial fronts more tagged than shops");
    assert!(mean(LandUse::Retail as u8, 0) > mean(LandUse::House as u8, 0), "shops more tagged than homes");
    assert!(mean(LandUse::Retail as u8, 2) > mean(LandUse::House as u8, 2), "more litter in front of shops than homes");
}

/// A debugging aid: the life decals over a point (EO_SL_AT=x,z), their parameters.
#[test]
#[ignore]
fn street_life_at_a_point() {
    let at: Vec<f64> = std::env::var("EO_SL_AT").unwrap_or("-1714.9,2511.3".into()).split(',').map(|v| v.parse().unwrap()).collect();
    let paths = eo_data::DataPaths::discover().unwrap();
    let city = Arc::new(eo_data::City::load(&paths).unwrap());
    let elev = eo_data::ElevationGrid::load(&paths).unwrap();
    let o = StreetOptions::load().unwrap();
    let lift = o.open_ground_lift;
    let rings: Vec<Vec<[f64; 2]>> = city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
    let sw = StreetWorld::build(o, StreetCity::load().unwrap(), Terrain { grid: Arc::new(elev), carpet_lift: lift }, BuildingIndex::new(rings));
    let uses = uses(&paths.repo_root);
    let classes: Vec<u8> = city.buildings.iter().map(|b| uses.get(&b.id).and_then(|u| LandUse::from_metadata(u)).unwrap_or_else(|| LandUse::from_osm(&b.tags.building, &b.tags.amenity)) as u8).collect();
    let cx = Context { level: 1, classes: &classes, structures: None, trees: Trees::get(), hot: Hotspots::get() };
    let key = eo_street::tile_key(at[0], at[1]);
    // EO_SL_R: how far round the point (m, default 8)
    let reach: f32 = std::env::var("EO_SL_R").ok().and_then(|r| r.parse().ok()).unwrap_or(8.0);
    let mut v = vec![];
    street_life::place(&sw, key, &cx, &mut v);
    for d in &v {
        let q = [at[0] as f32 - d.c[0], at[1] as f32 - d.c[1]];
        let along = q[0] * d.a[0] + q[1] * d.a[1];
        let across = -q[0] * d.a[1] + q[1] * d.a[0];
        if along.abs() < d.half[0] + reach && across.abs() < d.half[1] + reach {
            println!("c {:?} a {:?} half {:?} along {along:.1} across {across:.1} {:?} seed {:.3}", d.c, d.a, d.half, Params::unpack(d.kind >> KIND_SHIFT), d.seed);
        }
    }
    // and the wear decals within 12 m (kind, centre, half sizes)
    for d in eo_render::cc0_decals::place_with(&sw, key, true) {
        let r = ((d.c[0] as f64 - at[0]).powi(2) + (d.c[1] as f64 - at[1]).powi(2)).sqrt();
        if r < 12.0 {
            println!("wear kind {} c {:?} a {:?} half {:?}", d.kind, d.c, d.a, d.half);
        }
    }
    if let Some(b) = sw.buildings.at(at[0], at[1] - 7.0) {
        println!("building north: {} {:?} {:?}", city.buildings[b as usize].id, LandUse::from_u8(classes[b as usize]), city.buildings[b as usize].tags.building);
    }
}
