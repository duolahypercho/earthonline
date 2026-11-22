//! the far terrain's outer ring (stage L5, the cinematic look only): the land
//! past the 32 m cook out to the edges of the six USGS DEM tiles already on
//! disk (37-39 N, 121-124 W), at 128 m texels, so a camera high over the
//! city sees hills and valley floor fading into the haze instead of the
//! cooked grid's rim. Drawn by the far layer (farshore.rs) as a second
//! quadtree with the inner cook's extent cut out; past the ring's own edge
//! its edge texels are stretched to the horizon (the skirt, farshore.wgsl).
//!
//! The cook lives here and not in eo-farshore because eo-farshore's cache key
//! hashes its lib.rs and dem.rs: a new module declared there would change the
//! default cook's key. It reads the tiles through `eo_farshore::dem` and writes
//! the same file format (`eo_farshore::encode`/`decode`) under its own key
//! (`OUTER_COOK_VERSION` + this file + the tiles' digests), so the default cook,
//! its key and its file are untouched.
//!
//! Heights: each texel the mean of 4 x 4 bilinear DEM samples (one every
//! 32 m), game y (NAVD88 + `NAVD88_TO_GAME_Y`); 3DEP hydro-flattened water at
//! or under `eo_farshore::BAY_FLAT_MAX_NAVD88_M` is the bay (2 m under sea
//! level, as the inner cook); no data (the open Pacific) is the sea floor.
//! No taper at the ring's rim: the skirt carries the edge on to the horizon.
//! Classes at half resolution from heights and slopes alone (no built-up
//! guess: the valley floors out here are farmland, not town); the shape plane
//! is level (sky visibility 0.92, valley 0.5).

use anyhow::{Context, Result, anyhow, bail};
use eo_data::{Frame, NAVD88_TO_GAME_Y, SEA_FLOOR_Y, SEA_LEVEL_Y};
use eo_farshore::{CookHeader, DemManifest, FarTerrain, dem};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

/// Bump when this cook changes what it writes (part of its cache key, with this file's source).
pub const OUTER_COOK_VERSION: &str = "farshore-outer-1";
/// Texel, surveyed metres: four of the inner cook's (eo-farshore `TEXEL_M`), so
/// a level-0 node (32 texels) is 4,096 m, the inner cook's level 2.
pub const OUTER_TEXEL_M: f64 = 128.0;
/// The ring's grid: west and north edges (surveyed m) and size in texels. Edges
/// on multiples of 4,096 m (one level-0 node) as the inner cook's are, so the
/// ring's nodes meet the inner extent exactly; inside the tiles' data on the
/// land sides (east 121 W is x 124.9 km, south 37 N z 86.0 km, north 39 N
/// z -135.1 km; a texel past the data would read as sea floor), the west
/// (124 W, x -139.1 km) is open sea either way.
pub const OUTER_ORIGIN_X: f64 = -135_168.0;
pub const OUTER_ORIGIN_Z: f64 = -131_072.0;
pub const OUTER_WIDTH: usize = 2016;
pub const OUTER_HEIGHT: usize = 1696;
/// DEM samples a texel side (one every 32 m).
const SUB: usize = 4;
/// The shape plane's level value (sky visibility, valley).
const LEVEL_SHAPE: [u8; 2] = [235, 128];

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The ring's cache key: its version, this file, the tiles' digests.
pub fn cache_key() -> String {
    let mut h = Sha256::new();
    h.update(OUTER_COOK_VERSION.as_bytes());
    h.update(include_str!("outer.rs").as_bytes());
    for t in DemManifest::get().tiles {
        h.update(t.sha256.as_bytes());
    }
    hex(&h.finalize())[..24].to_string()
}

/// The ring's extent, surveyed metres: min x, min z, max x, max z.
pub fn extent() -> [f64; 4] {
    [OUTER_ORIGIN_X, OUTER_ORIGIN_Z, OUTER_ORIGIN_X + OUTER_WIDTH as f64 * OUTER_TEXEL_M, OUTER_ORIGIN_Z + OUTER_HEIGHT as f64 * OUTER_TEXEL_M]
}

/// Game y of one texel from a height source (surveyed x, z of the texel's
/// west-north corner): the mean of SUB x SUB samples, and whether most of
/// them are hydro-flattened water above the sea (a lake).
fn texel(sample: &impl Fn(f64, f64) -> Option<(f64, bool)>, x0: f64, z0: f64) -> (f32, bool) {
    let step = OUTER_TEXEL_M / SUB as f64;
    let (mut sum, mut lakes) = (0.0, 0usize);
    for sj in 0..SUB {
        for si in 0..SUB {
            let (x, z) = (x0 + (si as f64 + 0.5) * step, z0 + (sj as f64 + 0.5) * step);
            let (y, lake) = sample(x, z).unwrap_or((SEA_FLOOR_Y, false));
            sum += y;
            lakes += lake as usize;
        }
    }
    let y = sum / (SUB * SUB) as f64;
    (y as f32, lakes * 2 > SUB * SUB && y > SEA_LEVEL_Y)
}

/// The ring from a height source (surveyed x, z -> game y, flat water above
/// the sea): heights, lake flags. Rows in parallel; deterministic.
pub fn resample(sample: &(impl Fn(f64, f64) -> Option<(f64, bool)> + Sync)) -> (Vec<f32>, Vec<u8>) {
    let (w, h) = (OUTER_WIDTH, OUTER_HEIGHT);
    let mut heights = vec![0f32; w * h];
    let mut lake = vec![0u8; w * h];
    let threads = std::thread::available_parallelism().map(|p| p.get()).unwrap_or(4).min(24);
    let rows_per = h.div_ceil(threads);
    std::thread::scope(|s| {
        for (k, (hc, lc)) in heights.chunks_mut(rows_per * w).zip(lake.chunks_mut(rows_per * w)).enumerate() {
            s.spawn(move || {
                for (r, (hrow, lrow)) in hc.chunks_mut(w).zip(lc.chunks_mut(w)).enumerate() {
                    let j = k * rows_per + r;
                    let z0 = OUTER_ORIGIN_Z + j as f64 * OUTER_TEXEL_M;
                    for i in 0..w {
                        let (y, l) = texel(sample, OUTER_ORIGIN_X + i as f64 * OUTER_TEXEL_M, z0);
                        hrow[i] = y;
                        lrow[i] = l as u8;
                    }
                }
            });
        }
    });
    (heights, lake)
}

/// Classes at half resolution (RGBA8: built-up, lake, wood, marsh) from the
/// heights alone, and the level shape plane.
pub fn classify(heights: &[f32], lake: &[u8], w: usize, h: usize) -> (usize, usize, Vec<[u8; 4]>, Vec<[u8; 2]>) {
    let (cw, ch) = (w / 2, h / 2);
    let cell = OUTER_TEXEL_M * 2.0;
    let mut hm = vec![0f32; cw * ch];
    let mut lk = vec![0f32; cw * ch];
    for j in 0..ch {
        for i in 0..cw {
            let (mut a, mut l) = (0.0, 0.0);
            for (di, dj) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                a += heights[(2 * j + dj) * w + 2 * i + di];
                l += lake[(2 * j + dj) * w + 2 * i + di] as f32;
            }
            hm[j * cw + i] = a * 0.25;
            lk[j * cw + i] = l * 0.25;
        }
    }
    let at = |i: i64, j: i64| hm[(j.clamp(0, ch as i64 - 1) as usize) * cw + i.clamp(0, cw as i64 - 1) as usize] as f64;
    let q = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let mut out = vec![[0u8; 4]; cw * ch];
    for j in 0..ch {
        for i in 0..cw {
            let k = j * cw + i;
            let (ii, jj) = (i as i64, j as i64);
            let y = hm[k] as f64;
            let navd = y - NAVD88_TO_GAME_Y;
            let dx = (at(ii + 1, jj) - at(ii - 1, jj)) / (2.0 * cell);
            let dz = (at(ii, jj + 1) - at(ii, jj - 1)) / (2.0 * cell);
            let slope = (dx * dx + dz * dz).sqrt();
            let north = if slope > 1e-4 { dz / slope } else { 0.0 };
            let water = lk[k] as f64;
            let land = if y > SEA_LEVEL_Y { 1.0 } else { 0.0 };
            // the Delta and the bay's edges: flat land within a few metres of the sea
            let marsh = land * (1.0 - water) * smoothstep(5.0, 2.0, navd) * smoothstep(0.02, 0.005, slope);
            // oak and chaparral on the steeper and north faces, more of it high up
            let wood = (smoothstep(0.06, 0.30, slope) * smoothstep(-0.3, 0.8, north) * 0.8 + 0.3 * smoothstep(300.0, 900.0, navd) * smoothstep(0.03, 0.12, slope))
                .clamp(0.0, 1.0)
                * land
                * (1.0 - water)
                * (1.0 - marsh);
            out[k] = [0, q(water), q(wood), q(marsh)];
        }
    }
    (cw, ch, out, vec![LEVEL_SHAPE; cw * ch])
}

/// Read and check every tile, then cook the ring.
pub fn bake(dem_dir: &Path) -> Result<FarTerrain> {
    let manifest = DemManifest::get();
    let t0 = std::time::Instant::now();
    let tiles: Vec<Result<dem::DemTile>> = std::thread::scope(|s| {
        let hs: Vec<_> = manifest
            .tiles
            .iter()
            .map(|t| {
                let path = dem_dir.join(&t.file);
                let want = t.sha256.clone();
                s.spawn(move || -> Result<dem::DemTile> {
                    let got = sha256_file(&path)?;
                    if got != want {
                        bail!("{}: SHA-256 {got} is not the manifest's {want}", path.display());
                    }
                    dem::DemTile::read(&path)
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap_or_else(|_| Err(anyhow!("tile thread panicked")))).collect()
    });
    let mosaic = dem::DemMosaic::new(tiles.into_iter().collect::<Result<Vec<_>>>()?)?;
    let frame = Frame::sf();
    let sample = |x: f64, z: f64| {
        let (lat, lon) = frame.unproject(x, z);
        mosaic.sample(lat, lon).map(|d| {
            if d.flat && d.navd88 <= eo_farshore::BAY_FLAT_MAX_NAVD88_M {
                ((SEA_LEVEL_Y - 2.0).min(d.navd88 as f64 + NAVD88_TO_GAME_Y), false)
            } else {
                (d.navd88 as f64 + NAVD88_TO_GAME_Y, d.flat)
            }
        })
    };
    let (heights, lake) = resample(&sample);
    let (w, h) = (OUTER_WIDTH, OUTER_HEIGHT);
    let (cw, ch, classes, shape) = classify(&heights, &lake, w, h);
    eprintln!("farshore: cooked the outer ring {w} x {h} at {OUTER_TEXEL_M} m from {} DEM tiles in {:.1} s", mosaic.tiles.len(), t0.elapsed().as_secs_f64());
    let header = CookHeader {
        // the file format's version (eo_farshore::decode checks it); this cook's own is in `source` and the key
        version: eo_farshore::COOK_VERSION.into(),
        frame: format!("eo_data::Frame::sf(): centre {} N {} E; x east, z south, surveyed metres", frame.center_lat, frame.center_lon),
        origin_x: OUTER_ORIGIN_X,
        origin_z: OUTER_ORIGIN_Z,
        texel_m: OUTER_TEXEL_M,
        width: w,
        height: h,
        sea_level_y: SEA_LEVEL_Y,
        heights: "int16 decimetres of game y, texel means of 4 x 4 DEM samples, row 0 north, each row delta coded from 0".into(),
        classes: format!("RGBA8 at {cw} x {ch}: built-up (none), lake, wood, marsh; guesses from heights and slopes"),
        shape: format!("RG8 at {cw} x {ch}: level ({}, {})", LEVEL_SHAPE[0], LEVEL_SHAPE[1]),
        source: format!("{OUTER_COOK_VERSION}: {} ({}), public domain, USGS", manifest.product, manifest.tiles.iter().map(|t| t.id.as_str()).collect::<Vec<_>>().join(", ")),
        tiles: manifest.tiles.iter().map(|t| format!("{} sha256 {}", t.id, t.sha256)).collect(),
        city_grid: "none (the outer ring)".into(),
    };
    Ok(FarTerrain { header, heights, classes, shape, class_width: cw, class_height: ch, from: "baked" })
}

/// Checks a decoded file is this ring (extent and texel), not some other cook.
fn is_ring(t: &FarTerrain) -> bool {
    let h = &t.header;
    h.origin_x == OUTER_ORIGIN_X && h.origin_z == OUTER_ORIGIN_Z && h.texel_m == OUTER_TEXEL_M && h.width == OUTER_WIDTH && h.height == OUTER_HEIGHT && h.source.starts_with(OUTER_COOK_VERSION)
}

/// The outer ring for this process: from the cache (`<cache>/farshore/outer-<key>.eofs`),
/// else cooked from the DEM folder's tiles (`eo_farshore::dem_root`: `external/dem`
/// or the folder `EO_DEM_ROOT` names; cached), else None (no ring: the
/// inner cook's rim is skirted to the horizon instead).
pub fn load_or_bake(cache: Option<&Path>) -> Result<Option<FarTerrain>> {
    let file = cache.map(|c| c.join("farshore").join(format!("outer-{}.eofs", cache_key())));
    if let Some(f) = &file {
        if f.is_file() {
            match std::fs::read(f).map_err(anyhow::Error::from).and_then(|b| eo_farshore::decode(&b)) {
                Ok(t) if is_ring(&t) => return Ok(Some(t)),
                Ok(_) => eprintln!("farshore: {} is not the outer ring; cooking again", f.display()),
                Err(e) => eprintln!("farshore: {} refused ({e:#}); cooking again", f.display()),
            }
        }
    }
    let Some(dem) = eo_farshore::dem_root() else {
        eprintln!("farshore: no outer ring: no DEM folder ({}) and no cooked ring is cached", eo_data::external::DEM.looked_in());
        return Ok(None);
    };
    let manifest = DemManifest::get();
    if manifest.tiles.iter().any(|t| !dem.join(&t.file).is_file()) {
        eprintln!("farshore: no outer ring: the DEM tiles are not all under {} (fetch them with `python3 scripts/setup-assets.py --only dem`, or set EO_DEM_ROOT)", dem.display());
        return Ok(None);
    }
    let t = bake(&dem)?;
    if let Some(f) = &file {
        let bytes = eo_farshore::encode(&t)?;
        if let Some(d) = f.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = f.with_extension("tmp");
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, f)?;
        eprintln!("farshore: wrote {} ({:.1} MB)", f.display(), bytes.len() as f64 / 1e6);
    }
    Ok(Some(t))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ring_meets_the_inner_cook_on_node_edges() {
        let node0 = OUTER_TEXEL_M * crate::farshore::PATCH as f64;
        assert_eq!(node0, 4096.0);
        let inner = [eo_farshore::ORIGIN_X, eo_farshore::ORIGIN_Z, eo_farshore::ORIGIN_X + eo_farshore::WIDTH as f64 * eo_farshore::TEXEL_M, eo_farshore::ORIGIN_Z + eo_farshore::HEIGHT as f64 * eo_farshore::TEXEL_M];
        let e = extent();
        for v in inner.iter().chain(e.iter()) {
            assert_eq!((v / node0).fract(), 0.0, "{v} is not on a 4,096 m node edge");
        }
        // the ring holds the inner cook with room round it
        assert!(e[0] < inner[0] && e[1] < inner[1] && e[2] > inner[2] && e[3] > inner[3]);
        assert_eq!(OUTER_WIDTH % crate::farshore::PATCH as usize, 0);
        assert_eq!(OUTER_HEIGHT % crate::farshore::PATCH as usize, 0);
        // inside the tiles' data on the land sides (37-39 N, 121 W)
        let f = Frame::sf();
        let (lat_n, _) = f.unproject(0.0, e[1]);
        let (lat_s, lon_e) = f.unproject(e[2], e[3]);
        assert!(lat_n <= 39.0 && lat_s >= 37.0 && lon_e <= -121.0, "{lat_n} {lat_s} {lon_e}");
    }

    #[test]
    fn the_ring_is_deterministic_and_reads_its_source() {
        // a synthetic source: a ridge rising east, the sea west of x = 0, a lake
        let src = |x: f64, z: f64| -> Option<(f64, bool)> {
            if x < -100_000.0 {
                None
            } else if x < 0.0 {
                Some((SEA_LEVEL_Y - 2.0, false))
            } else if (x - 50_000.0).abs() < 1_000.0 && (z - 10_000.0).abs() < 1_000.0 {
                Some((40.0, true))
            } else {
                Some((x / 100.0 + (z / 3_000.0).sin() * 50.0, false))
            }
        };
        let (h1, l1) = resample(&src);
        let (h2, l2) = resample(&src);
        assert!(h1.iter().zip(&h2).all(|(a, b)| a.to_bits() == b.to_bits()));
        assert_eq!(l1, l2);
        let idx = |x: f64, z: f64| ((z - OUTER_ORIGIN_Z) / OUTER_TEXEL_M) as usize * OUTER_WIDTH + ((x - OUTER_ORIGIN_X) / OUTER_TEXEL_M) as usize;
        assert_eq!(h1[idx(-120_000.0, 0.0)], SEA_FLOOR_Y as f32); // no data: the sea floor
        assert!(h1[idx(-50_000.0, 0.0)] < SEA_LEVEL_Y as f32); // the bay
        assert_eq!(l1[idx(50_000.0, 10_000.0)], 1); // the lake
        assert!(h1[idx(100_000.0, 0.0)] > 900.0); // the ridge
        let (cw, ch, classes, shape) = classify(&h1, &l1, OUTER_WIDTH, OUTER_HEIGHT);
        assert_eq!((cw, ch, classes.len(), shape.len()), (OUTER_WIDTH / 2, OUTER_HEIGHT / 2, cw * ch, cw * ch));
        assert!(classes[(idx(50_000.0, 10_000.0) / OUTER_WIDTH / 2) * cw + (idx(50_000.0, 10_000.0) % OUTER_WIDTH) / 2][1] > 128);
    }

    #[test]
    fn the_cache_key_is_its_own() {
        // never the default cook's key (that one stays as it was)
        assert_ne!(cache_key(), eo_farshore::cache_key("", ""));
        assert_eq!(cache_key(), cache_key());
    }
}
