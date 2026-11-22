//! the land around the city out to the horizon (the East Bay
//! hills and flats, Marin, the Peninsula, Angel Island, Mount Tamalpais,
//! Mount Diablo), cooked from the USGS 3DEP 1 arc-second DEM.
//!
//! * Source: the six 1 x 1 degree tiles named in `native/assets/dem-manifest.json`,
//!   read from `<repo>/external/dem` (git-ignored, filled by
//!   `scripts/setup-assets.py`) or the folder `EO_DEM_ROOT` names instead, each
//!   checked against its SHA-256 before it is decoded. Never in the game's
//!   runtime path: the game reads only the cooked file.
//! * Frame: the city's own (`eo_data::Frame::sf()`, an equirectangular
//!   projection about 37.778 N, 122.4194 W, x east, z south) in surveyed metres;
//!   heights as game y (NAVD88 + `NAVD88_TO_GAME_Y`, so mean sea level is
//!   `SEA_LEVEL_Y`). The ground plan's scale and the height scale are applied
//!   at draw time exactly as `groundscale::plan` and `groundscale::lift_by` do,
//!   so the cooked file does not depend on them.
//! * Inside the city's elevation grid the cook takes the grid itself
//!   (`ElevationGrid::sample_surveyed`); outside it the DEM, blended from the
//!   grid's edge over `EDGE_BLEND_M`, so the two meet without a step.
//! * Cooked file (`cache/farshore/<key>.eofs`, gzip): "EOFS" | u32 header bytes |
//!   header JSON | i16 heights (decimetres of game y, row-delta coded) |
//!   RGBA8 classes at half resolution (urban, lake, wood, marsh) | SHA-256 of
//!   everything before it. A browser can read it as it is.
//! * No DEM and no cooked file: `load_or_bake` returns `Ok(None)` and the game
//!   draws no far terrain, as before.

pub mod dem;
pub mod survey; // the Golden Gate survey set under EO_SURVEY_ROOT (NAIP, 1 m DEM, CUDEM, scans); called by nothing yet
pub use half; // the far layer uploads its heights as half floats

use anyhow::{Context, Result, anyhow, bail};
use eo_data::{ElevationGrid, Frame, LandCover, NAVD88_TO_GAME_Y, SEA_FLOOR_Y, SEA_LEVEL_Y};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Bump when the cook changes what it writes (part of the cache key, with this crate's source).
pub const COOK_VERSION: &str = "farshore-cook-2"; // + the shape plane
/// The depth the far layer writes past the camera's far plane, as clip z
/// (reverse Z: depth = DEPTH_K / view depth; the city's own depth reaches 0 at
/// its far plane, so everything past it sorts behind the city and in true
/// order among itself). The hooks in water.wgsl and sky_volume.wgsl use the
/// same literal (`farshore:`), checked by `hooks_use_the_same_constant`.
pub const DEPTH_K: f32 = 1e-9;
/// Cooked texel, surveyed metres (the DEM is 1 arc-second: 30.7 m north-south, 24.4 m east-west here).
pub const TEXEL_M: f64 = 32.0;
/// Cooked grid: west and north edges (surveyed metres) and size in texels
/// (multiples of 64 so six mip levels stay aligned).
pub const ORIGIN_X: f64 = -61_440.0;
pub const ORIGIN_Z: f64 = -69_632.0;
pub const WIDTH: usize = 4480;
pub const HEIGHT: usize = 4224;
/// Width of the band outside the city grid over which its edge hands over to the DEM.
pub const EDGE_BLEND_M: f64 = 480.0;
/// The outermost band of the cooked grid falls to the sea floor (no cliff at the data's end).
pub const OUTER_TAPER_M: f64 = 4000.0;
/// 3DEP hydro-flattened water at or below this NAVD88 height is the bay (or a salt pond on it): sea.
pub const BAY_FLAT_MAX_NAVD88_M: f32 = 2.5;

/// What the cook wrote (the cooked file's header).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CookHeader {
    pub version: String,
    pub frame: String,
    #[serde(rename = "originX")]
    pub origin_x: f64,
    #[serde(rename = "originZ")]
    pub origin_z: f64,
    #[serde(rename = "texelM")]
    pub texel_m: f64,
    pub width: usize,
    pub height: usize,
    #[serde(rename = "seaLevelY")]
    pub sea_level_y: f64,
    pub heights: String,
    pub classes: String,
    /// the shape plane after the classes.
    pub shape: String,
    pub source: String,
    pub tiles: Vec<String>,
    #[serde(rename = "cityGrid")]
    pub city_grid: String,
}

/// The cooked far terrain, decoded.
pub struct FarTerrain {
    pub header: CookHeader,
    /// Game y (surveyed heights, before the height scale), row 0 north.
    pub heights: Vec<f32>,
    /// Half resolution, RGBA8: urban, lake, wood, marsh (0..255).
    pub classes: Vec<[u8; 4]>,
    /// Half resolution, RG8: sky visibility, valley (0.5 level).
    pub shape: Vec<[u8; 2]>,
    pub class_width: usize,
    pub class_height: usize,
    /// Where it came from ("cache", "baked").
    pub from: &'static str,
}

impl FarTerrain {
    /// Surveyed (x, z) of texel (i, j)'s centre.
    pub fn texel_centre(&self, i: usize, j: usize) -> (f64, f64) {
        (self.header.origin_x + (i as f64 + 0.5) * self.header.texel_m, self.header.origin_z + (j as f64 + 0.5) * self.header.texel_m)
    }

    /// Bilinear game y (surveyed heights) at a surveyed point, between texel centres (clamped at the edges).
    pub fn sample_surveyed(&self, x: f64, z: f64) -> f64 {
        let h = &self.header;
        let fx = (x - h.origin_x) / h.texel_m - 0.5;
        let fz = (z - h.origin_z) / h.texel_m - 0.5;
        let i0 = (fx.floor() as i64).clamp(0, h.width as i64 - 1);
        let j0 = (fz.floor() as i64).clamp(0, h.height as i64 - 1);
        let i1 = (i0 + 1).min(h.width as i64 - 1);
        let j1 = (j0 + 1).min(h.height as i64 - 1);
        let (tx, tz) = ((fx - i0 as f64).clamp(0.0, 1.0), (fz - j0 as f64).clamp(0.0, 1.0));
        let at = |i: i64, j: i64| self.heights[j as usize * h.width + i as usize] as f64;
        let a = at(i0, j0) * (1.0 - tx) + at(i1, j0) * tx;
        let b = at(i0, j1) * (1.0 - tx) + at(i1, j1) * tx;
        a * (1.0 - tz) + b * tz
    }

    /// Game y on the game's ground plan at a game point (plan scale `s`,
    /// height scale `hs`): the transform the far layer's vertex shader applies.
    pub fn sample_game(&self, gx: f64, gz: f64, s: f64, hs: f64) -> f64 {
        eo_data::groundscale::lift_by(self.sample_surveyed(gx / s, gz / s), hs)
    }
}

// ---------------------------------------------------------------------------
// The DEM manifest and where the tiles are
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize)]
pub struct ManifestTile {
    pub id: String,
    pub file: String,
    pub url: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DemManifest {
    pub product: String,
    pub tiles: Vec<ManifestTile>,
}

impl DemManifest {
    pub fn get() -> DemManifest {
        serde_json::from_str(include_str!("../../assets/dem-manifest.json")).expect("native/assets/dem-manifest.json")
    }
}

/// The DEM's folder this run: the one `EO_DEM_ROOT` names, else
/// `<repo>/external/dem`; None when neither is known (only a cached cook is used).
pub fn dem_root() -> Option<PathBuf> {
    eo_data::external::DEM.root()
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

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The cache key: this crate's source, the cook version, the manifest's tile
/// digests and the city grid file's digest.
pub fn cache_key(grid_sha: &str, land_cover_sha: &str) -> String {
    let mut h = Sha256::new();
    h.update(COOK_VERSION.as_bytes());
    h.update(include_str!("lib.rs").as_bytes());
    h.update(include_str!("dem.rs").as_bytes());
    for t in DemManifest::get().tiles {
        h.update(t.sha256.as_bytes());
    }
    h.update(grid_sha.as_bytes());
    h.update(land_cover_sha.as_bytes());
    hex(&h.finalize())[..24].to_string()
}

// ---------------------------------------------------------------------------
// The cook
// ---------------------------------------------------------------------------

fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Value noise in [0, 1] on a lattice of `cell` metres (deterministic: the cook is reproducible).
fn vnoise(x: f64, z: f64, cell: f64, seed: u32) -> f64 {
    let (fx, fz) = (x / cell, z / cell);
    let (i, j) = (fx.floor(), fz.floor());
    let (tx, tz) = (fx - i, fz - j);
    let h = |a: i64, b: i64| {
        let mut v = (a as u32).wrapping_mul(0x8da6_b343) ^ (b as u32).wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
        v ^= v >> 13;
        v = v.wrapping_mul(0x5bd1_e995);
        v ^= v >> 15;
        (v & 0xffff) as f64 / 65535.0
    };
    let (i, j) = (i as i64, j as i64);
    let s = |t: f64| t * t * (3.0 - 2.0 * t);
    let a = h(i, j) * (1.0 - s(tx)) + h(i + 1, j) * s(tx);
    let b = h(i, j + 1) * (1.0 - s(tx)) + h(i + 1, j + 1) * s(tx);
    a * (1.0 - s(tz)) + b * s(tz)
}

/// Read and check every tile, then cook. Fails when a tile is missing or its
/// digest is not the manifest's.
pub fn bake(dem_dir: &Path, grid: &ElevationGrid, grid_sha: &str, land_cover: Option<&LandCover>) -> Result<FarTerrain> {
    let manifest = DemManifest::get();
    let t0 = std::time::Instant::now();
    // Check and decode the tiles in parallel (each about 52 MB of float32).
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
    let tiles = tiles.into_iter().collect::<Result<Vec<_>>>()?;
    let read_s = t0.elapsed().as_secs_f64();
    let mosaic = dem::DemMosaic::new(tiles)?;
    let frame = Frame::sf();
    let (gx0, gz0, gx1, gz1) = (grid.meta.origin_x, grid.meta.origin_z, grid.max_x(), grid.max_z());
    let (w, h) = (WIDTH, HEIGHT);
    let ext = (ORIGIN_X, ORIGIN_Z, ORIGIN_X + w as f64 * TEXEL_M, ORIGIN_Z + h as f64 * TEXEL_M);
    let mut heights = vec![0f32; w * h];
    let mut lake = vec![0u8; w * h];
    let threads = std::thread::available_parallelism().map(|p| p.get()).unwrap_or(4).min(24);
    let rows_per = h.div_ceil(threads);
    std::thread::scope(|s| {
        for (k, (hc, lc)) in heights.chunks_mut(rows_per * w).zip(lake.chunks_mut(rows_per * w)).enumerate() {
            let mosaic = &mosaic;
            s.spawn(move || {
                for (r, (hrow, lrow)) in hc.chunks_mut(w).zip(lc.chunks_mut(w)).enumerate() {
                    let j = k * rows_per + r;
                    let z = ORIGIN_Z + (j as f64 + 0.5) * TEXEL_M;
                    for i in 0..w {
                        let x = ORIGIN_X + (i as f64 + 0.5) * TEXEL_M;
                        // The DEM: hydro-flattened water low enough to be the bay is sea.
                        let (lat, lon) = frame.unproject(x, z);
                        let (dem_y, is_lake) = match mosaic.sample(lat, lon) {
                            Some(d) if d.flat && d.navd88 <= BAY_FLAT_MAX_NAVD88_M => ((SEA_LEVEL_Y - 2.0).min(d.navd88 as f64 + NAVD88_TO_GAME_Y), false),
                            Some(d) => (d.navd88 as f64 + NAVD88_TO_GAME_Y, d.flat),
                            None => (SEA_FLOOR_Y, false),
                        };
                        // The city grid inside its own extent; its edge handed over to the DEM outside.
                        let ox = (gx0 - x).max(x - gx1).max(0.0);
                        let oz = (gz0 - z).max(z - gz1).max(0.0);
                        let out = (ox * ox + oz * oz).sqrt();
                        let mut y = if out <= 0.0 {
                            grid.sample_surveyed(x, z).unwrap_or(SEA_FLOOR_Y)
                        } else if out < EDGE_BLEND_M {
                            // The DEM's own shape, offset by the two surveys' difference at the
                            // nearest edge point, the offset fading out across the band: at the
                            // edge exactly the grid's height, and a hill across the edge keeps its form.
                            let (ex, ez) = (x.clamp(gx0 + 0.5, gx1 - 0.5), z.clamp(gz0 + 0.5, gz1 - 0.5));
                            let edge = grid.sample_surveyed(ex, ez).unwrap_or(SEA_FLOOR_Y);
                            let (elat, elon) = frame.unproject(ex, ez);
                            let dem_edge = mosaic.sample(elat, elon).map(|d| d.navd88 as f64 + NAVD88_TO_GAME_Y).unwrap_or(SEA_FLOOR_Y);
                            let t = smoothstep(0.0, EDGE_BLEND_M, out);
                            dem_y + (edge - dem_edge) * (1.0 - t)
                        } else {
                            dem_y
                        };
                        // No cliff where the data ends: the outer band falls to the sea floor.
                        let rim = (x - ext.0).min(ext.2 - x).min(z - ext.1).min(ext.3 - z);
                        if rim < OUTER_TAPER_M {
                            let t = smoothstep(0.0, OUTER_TAPER_M, rim);
                            y = SEA_FLOOR_Y + (y - SEA_FLOOR_Y) * t;
                        }
                        hrow[i] = y as f32;
                        lrow[i] = (is_lake && out > 0.0 && y > SEA_LEVEL_Y) as u8;
                    }
                }
            });
        }
    });
    let classes_ms = std::time::Instant::now();
    let (cw, ch, classes, shape) = classify(&heights, &lake, w, h, land_cover);
    eprintln!(
        "farshore: cooked {w} x {h} at {TEXEL_M} m from {} DEM tiles ({read_s:.1} s to check and decode, {:.1} s to resample, {:.1} s to classify)",
        mosaic.tiles.len(),
        (classes_ms - t0).as_secs_f64() - read_s,
        classes_ms.elapsed().as_secs_f64()
    );
    let header = CookHeader {
        version: COOK_VERSION.into(),
        frame: format!(
            "eo_data::Frame::sf(): centre {} N {} E, {} m/deg lat, {:.6} m/deg lon; x east, z south, surveyed metres (the ground plan scale is applied at draw time)",
            frame.center_lat, frame.center_lon, frame.m_per_deg_lat, frame.m_per_deg_lon
        ),
        origin_x: ORIGIN_X,
        origin_z: ORIGIN_Z,
        texel_m: TEXEL_M,
        width: w,
        height: h,
        sea_level_y: SEA_LEVEL_Y,
        heights: "int16 decimetres of game y (NAVD88 + NAVD88_TO_GAME_Y), texel centres, row 0 north, each row delta coded from 0".into(),
        classes: format!("RGBA8 at {cw} x {ch} (2 x 2 texels each): built-up, lake (hydro-flattened water above sea level), wood, marsh; guesses from heights and slopes, except inside the city's land-cover raster (public/data/sf/sf-land-cover.bin.gz: ESA WorldCover 10 m 2021 v200, CC BY 4.0, (c) ESA WorldCover project 2021 / contains modified Copernicus Sentinel data) where its built, tree and wetland shares{}", if land_cover.is_some() { "" } else { " (not read: absent)" }),
        shape: format!("RG8 at {cw} x {ch}: sky visibility (cosine-weighted open sky from the horizon in 8 directions to 2.6 km), valley (0.5 + (mean height 190 m round - height) / 160 m)"),
        source: format!("{} ({}), public domain, USGS; inside the city's elevation grid the grid itself", manifest.product, manifest.tiles.iter().map(|t| t.id.as_str()).collect::<Vec<_>>().join(", ")),
        tiles: manifest.tiles.iter().map(|t| format!("{} sha256 {}", t.id, t.sha256)).collect(),
        city_grid: format!("{} sha256 {grid_sha}", eo_data::SF_ELEVATION_GRID_VERSION),
    };
    Ok(FarTerrain { header, heights, classes, shape, class_width: cw, class_height: ch, from: "baked" })
}

/// The class guesses at half resolution (see `CookHeader::classes`). Urban:
/// low, gentle ground within a few kilometres of the bay; wood: steep and
/// north-facing, higher, wetter (west, toward the sea fog); marsh: flat land
/// within a metre or two of the sea beside the bay. None of this is land-cover
/// data: it is what heights and slopes suggest.
#[allow(clippy::type_complexity)]
fn classify(heights: &[f32], lake: &[u8], w: usize, h: usize, land_cover: Option<&LandCover>) -> (usize, usize, Vec<[u8; 4]>, Vec<[u8; 2]>) {
    let (cw, ch) = (w / 2, h / 2);
    let cell = TEXEL_M * 2.0;
    // Mean height per class cell.
    let mut hm = vec![0f32; cw * ch];
    let mut lk = vec![0f32; cw * ch];
    for j in 0..ch {
        for i in 0..cw {
            let mut a = 0.0;
            let mut l = 0.0;
            for (di, dj) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                a += heights[(2 * j + dj) * w + 2 * i + di];
                l += lake[(2 * j + dj) * w + 2 * i + di] as f32;
            }
            hm[j * cw + i] = a * 0.25;
            lk[j * cw + i] = l * 0.25;
        }
    }
    let sea = |k: usize| (hm[k] as f64) < SEA_LEVEL_Y;
    // The bay: sea cells joined to a point east of Treasure Island, the Golden
    // Gate closed so the fill stays inside (the Pacific is not the bay).
    let to_cell = |x: f64, z: f64| (((x - ORIGIN_X) / cell) as i64, ((z - ORIGIN_Z) / cell) as i64);
    let (gg_x, _) = to_cell(-5_000.0, 0.0);
    let (_, gg_z0) = to_cell(0.0, -7_200.0);
    let (_, gg_z1) = to_cell(0.0, -3_600.0);
    let mut bay = vec![false; cw * ch];
    let (si, sj) = to_cell(6_986.0, -4_644.0);
    let mut seed = None;
    'find: for r in 0..20i64 {
        for dj in -r..=r {
            for di in -r..=r {
                let (i, j) = (si + di, sj + dj);
                if i >= 0 && j >= 0 && (i as usize) < cw && (j as usize) < ch && sea(j as usize * cw + i as usize) {
                    seed = Some(j as usize * cw + i as usize);
                    break 'find;
                }
            }
        }
    }
    if let Some(s0) = seed {
        let mut stack = vec![s0];
        bay[s0] = true;
        while let Some(k) = stack.pop() {
            let (i, j) = ((k % cw) as i64, (k / cw) as i64);
            for (di, dj) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                let (ni, nj) = (i + di, j + dj);
                if ni < 0 || nj < 0 || ni as usize >= cw || nj as usize >= ch {
                    continue;
                }
                if ni < gg_x && nj >= gg_z0 && nj <= gg_z1 {
                    continue;
                }
                let nk = nj as usize * cw + ni as usize;
                if !bay[nk] && sea(nk) {
                    bay[nk] = true;
                    stack.push(nk);
                }
            }
        }
    }
    // Distance to the bay (3-4 chamfer, metres).
    let big = 1e9f32;
    let mut dist: Vec<f32> = bay.iter().map(|&b| if b { 0.0 } else { big }).collect();
    let (a, d) = (cell as f32, cell as f32 * std::f32::consts::SQRT_2);
    for j in 0..ch {
        for i in 0..cw {
            let k = j * cw + i;
            let mut v = dist[k];
            if i > 0 {
                v = v.min(dist[k - 1] + a);
            }
            if j > 0 {
                v = v.min(dist[k - cw] + a);
                if i > 0 {
                    v = v.min(dist[k - cw - 1] + d);
                }
                if i + 1 < cw {
                    v = v.min(dist[k - cw + 1] + d);
                }
            }
            dist[k] = v;
        }
    }
    for j in (0..ch).rev() {
        for i in (0..cw).rev() {
            let k = j * cw + i;
            let mut v = dist[k];
            if i + 1 < cw {
                v = v.min(dist[k + 1] + a);
            }
            if j + 1 < ch {
                v = v.min(dist[k + cw] + a);
                if i + 1 < cw {
                    v = v.min(dist[k + cw + 1] + d);
                }
                if i > 0 {
                    v = v.min(dist[k + cw - 1] + d);
                }
            }
            dist[k] = v;
        }
    }
    let mut out = vec![[0u8; 4]; cw * ch];
    // The land-cover raster's extent in surveyed metres (cells outside it are not sampled).
    let lc_box = land_cover.map_or((0.0, 0.0, 0.0, 0.0), |lc| {
        let f = Frame::sf();
        let x0 = (lc.west - f.center_lon) * f.m_per_deg_lon - cell;
        let x1 = (lc.west + lc.width as f64 * lc.d_lon - f.center_lon) * f.m_per_deg_lon + cell;
        let z0 = -(lc.north - f.center_lat) * f.m_per_deg_lat - cell;
        let z1 = -(lc.north - lc.height as f64 * lc.d_lat - f.center_lat) * f.m_per_deg_lat + cell;
        (x0, x1, z0, z1)
    });
    let at = |i: i64, j: i64| hm[(j.clamp(0, ch as i64 - 1) as usize) * cw + i.clamp(0, cw as i64 - 1) as usize] as f64;
    for j in 0..ch {
        for i in 0..cw {
            let k = j * cw + i;
            let (ii, jj) = (i as i64, j as i64);
            let y = hm[k] as f64;
            let navd = y - NAVD88_TO_GAME_Y;
            let dx = (at(ii + 1, jj) - at(ii - 1, jj)) / (2.0 * cell);
            let dz = (at(ii, jj + 1) - at(ii, jj - 1)) / (2.0 * cell);
            let slope = (dx * dx + dz * dz).sqrt();
            // Normal (-dx, 1, -dz): it faces north (-z) where the ground rises to the south (dz > 0).
            let north = if slope > 1e-4 { dz / slope } else { 0.0 };
            let x = ORIGIN_X + (i as f64 + 0.5) * cell;
            let z = ORIGIN_Z + (j as f64 + 0.5) * cell;
            let water = lk[k] as f64;
            let land = if y > SEA_LEVEL_Y { 1.0 } else { 0.0 };
            let bd = dist[k] as f64;
            let n1 = vnoise(x, z, 900.0, 1);
            let n2 = vnoise(x, z, 260.0, 2);
            let urban = land
                * (1.0 - water)
                * smoothstep(150.0, 60.0, navd)
                * smoothstep(1.5, 4.0, navd)
                * smoothstep(0.16, 0.06, slope)
                * smoothstep(7_500.0, 2_500.0, bd)
                * (0.75 + 0.25 * n1);
            let marsh = land * (1.0 - water) * smoothstep(4.0, 2.0, navd) * smoothstep(0.03, 0.01, slope) * smoothstep(4_000.0, 1_000.0, bd);
            // Wood: north faces of the steeper hills, more of it high and toward the sea fog (west).
            let fog_belt = smoothstep(10_000.0, -30_000.0, x) * smoothstep(80.0, 300.0, navd);
            // (and oak woodland and scrub in patches on any hillside)
            let patches = smoothstep(0.48, 0.70, vnoise(x, z, 420.0, 3)) * smoothstep(0.04, 0.14, slope);
            // the hills that face the bay carry trees (oak, bay, eucalyptus and
            // the gardens of their houses): a guess by distance to the bay, not data
            let bayside = 0.35 * smoothstep(9_000.0, 3_000.0, bd) * smoothstep(0.05, 0.15, slope);
            let wood = (smoothstep(0.10, 0.40, slope) * smoothstep(-0.3, 0.8, north) * 0.85
                + 0.6 * patches
                + bayside
                + 0.30 * smoothstep(250.0, 700.0, navd) * smoothstep(0.05, 0.2, slope)
                + 0.35 * fog_belt
                + 0.25 * (n2 - 0.5)
                + 0.15 * (n1 - 0.5))
                .clamp(0.0, 1.0)
                * land
                * (1.0 - urban)
                * (1.0 - water)
                * (1.0 - marsh);
            let (mut urban, mut wood, mut marsh) = (urban, wood, marsh);
            // Inside the city's own land-cover raster (ESA WorldCover 10 m 2021, already in
            // the repository): its built, tree and wetland shares over the cell, not the guess.
            if let Some(lc) = land_cover.filter(|_| x > lc_box.0 && x < lc_box.1 && z > lc_box.2 && z < lc_box.3) {
                let (mut on, mut built, mut tree, mut wet) = (0.0, 0.0, 0.0, 0.0);
                for sj in 0..8 {
                    for si in 0..8 {
                        let c = lc.class_at(x + (si as f64 - 3.5) * cell / 8.0, z + (sj as f64 - 3.5) * cell / 8.0);
                        if c == 0 {
                            continue;
                        }
                        on += 1.0;
                        match c {
                            50 => built += 1.0,
                            10 => tree += 1.0,
                            90 | 95 => wet += 1.0,
                            _ => {}
                        }
                    }
                }
                if on > 0.0 {
                    let cover = on / 64.0;
                    urban = urban * (1.0 - cover) + land * built / 64.0;
                    wood = wood * (1.0 - cover) + land * tree / 64.0;
                    marsh = marsh * (1.0 - cover) + land * wet / 64.0;
                }
            }
            let q = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            out[k] = [q(urban), q(water), q(wood), q(marsh)];
        }
    }
    let sh = shape(&hm, cw, ch);
    (cw, ch, out, sh)
}

/// The ground's shape at half resolution (`CookHeader::shape`): R the sky
/// visibility (the open share of the sky dome, cosine-weighted, from the
/// horizon in 8 directions out to ~2.6 km: valleys and the foot of a slope
/// see less sky), G how much the cell lies below its surroundings 190 m round
/// (0.5 level, 1 a ravine 80 m down, 0 a ridge 80 m up). From the heights
/// alone; angles do not change with the ground plan (plan and heights scale alike).
fn shape(hm: &[f32], cw: usize, ch: usize) -> Vec<[u8; 2]> {
    let cell = TEXEL_M * 2.0;
    let steps = [1.0f64, 2.0, 3.0, 5.0, 8.0, 12.0, 18.0, 27.0, 40.0];
    let dirs: Vec<(f64, f64)> = (0..8).map(|k| (k as f64 * std::f64::consts::FRAC_PI_4).sin_cos()).map(|(s, c)| (c, s)).collect();
    let at = |x: f64, y: f64| -> f64 {
        let (i, j) = ((x.round() as i64).clamp(0, cw as i64 - 1) as usize, (y.round() as i64).clamp(0, ch as i64 - 1) as usize);
        hm[j * cw + i] as f64
    };
    let mut out = vec![[0u8; 2]; cw * ch];
    let threads = std::thread::available_parallelism().map(|p| p.get()).unwrap_or(4).min(24);
    let rows = ch.div_ceil(threads);
    std::thread::scope(|s| {
        for (t, chunk) in out.chunks_mut(rows * cw).enumerate() {
            let (dirs, at, steps) = (&dirs, &at, &steps);
            s.spawn(move || {
                for (r, row) in chunk.chunks_mut(cw).enumerate() {
                    let j = t * rows + r;
                    for (i, o) in row.iter_mut().enumerate() {
                        let (x, y) = (i as f64, j as f64);
                        let h0 = (hm[j * cw + i] as f64).max(SEA_LEVEL_Y);
                        let mut vis = 0.0;
                        let mut ring = 0.0;
                        for &(dx, dy) in dirs.iter() {
                            let mut tan_max: f64 = 0.0;
                            for &d in steps.iter() {
                                let hh = at(x + dx * d, y + dy * d).max(SEA_LEVEL_Y);
                                tan_max = tan_max.max((hh - h0) / (d * cell));
                            }
                            vis += 1.0 - tan_max / (1.0 + tan_max * tan_max).sqrt();
                            ring += at(x + dx * 3.0, y + dy * 3.0).max(SEA_LEVEL_Y);
                        }
                        let valley = 0.5 + (ring / 8.0 - h0) / 160.0;
                        let q = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                        *o = [q(vis / 8.0), q(valley)];
                    }
                }
            });
        }
    });
    out
}

// ---------------------------------------------------------------------------
// The cooked file
// ---------------------------------------------------------------------------

pub fn encode(t: &FarTerrain) -> Result<Vec<u8>> {
    let mut header = serde_json::to_vec(&t.header)?;
    while (8 + header.len()) % 2 != 0 {
        header.push(b' ');
    }
    let mut raw = Vec::with_capacity(8 + header.len() + t.heights.len() * 2 + t.classes.len() * 4 + 32);
    raw.extend_from_slice(b"EOFS");
    raw.extend_from_slice(&(header.len() as u32).to_le_bytes());
    raw.extend_from_slice(&header);
    let w = t.header.width;
    for row in t.heights.chunks(w) {
        let mut prev: i32 = 0;
        for &v in row {
            let dm = (v as f64 * 10.0).round() as i32;
            let d = dm - prev;
            if d < i16::MIN as i32 || d > i16::MAX as i32 {
                bail!("farshore: a row step of {d} dm does not fit int16");
            }
            raw.extend_from_slice(&(d as i16).to_le_bytes());
            prev = dm;
        }
    }
    for c in &t.classes {
        raw.extend_from_slice(c);
    }
    for c in &t.shape {
        raw.extend_from_slice(c);
    }
    let digest = Sha256::digest(&raw);
    raw.extend_from_slice(&digest);
    let mut gz = GzEncoder::new(Vec::new(), Compression::new(6));
    gz.write_all(&raw)?;
    Ok(gz.finish()?)
}

pub fn decode(bytes: &[u8]) -> Result<FarTerrain> {
    let mut raw = Vec::with_capacity(bytes.len() * 4);
    GzDecoder::new(bytes).read_to_end(&mut raw).context("farshore: gunzip")?;
    if raw.len() < 8 + 32 || &raw[0..4] != b"EOFS" {
        bail!("farshore: bad magic");
    }
    let (body, digest) = raw.split_at(raw.len() - 32);
    if Sha256::digest(body).as_slice() != digest {
        bail!("farshore: payload digest does not match (damaged or truncated)");
    }
    let hb = u32::from_le_bytes(body[4..8].try_into().unwrap()) as usize;
    let header: CookHeader = serde_json::from_slice(&body[8..8 + hb]).context("farshore: header")?;
    if header.version != COOK_VERSION {
        bail!("farshore: cooked {}, this build reads {COOK_VERSION}", header.version);
    }
    let (w, h) = (header.width, header.height);
    let (cw, ch) = (w / 2, h / 2);
    let off = 8 + hb;
    if body.len() != off + w * h * 2 + cw * ch * 6 {
        bail!("farshore: {} bytes, expected {}", body.len(), off + w * h * 2 + cw * ch * 6);
    }
    let mut heights = vec![0f32; w * h];
    for (j, row) in heights.chunks_mut(w).enumerate() {
        let mut prev: i32 = 0;
        for (i, v) in row.iter_mut().enumerate() {
            let p = off + (j * w + i) * 2;
            prev += i16::from_le_bytes([body[p], body[p + 1]]) as i32;
            *v = prev as f32 / 10.0;
        }
    }
    let co = off + w * h * 2;
    let classes = body[co..co + cw * ch * 4].chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
    let so = co + cw * ch * 4;
    let shape = body[so..so + cw * ch * 2].chunks_exact(2).map(|c| [c[0], c[1]]).collect();
    Ok(FarTerrain { header, heights, classes, shape, class_width: cw, class_height: ch, from: "cache" })
}

/// The far terrain for this process: the cooked file from the cache, else
/// cooked from the DEM (and written to the cache), else None (no DEM: the game
/// draws no far terrain, as before). The city grid is read unscaled.
pub fn load_or_bake(repo_root: &Path, cache: Option<&Path>) -> Result<Option<FarTerrain>> {
    let grid_path = repo_root.join("public/data/sf/sf-elevation.bin.gz");
    let grid_bytes = std::fs::read(&grid_path).with_context(|| format!("reading {}", grid_path.display()))?;
    let grid_sha = hex(&Sha256::digest(&grid_bytes));
    // The city's land-cover raster (optional: without it the guesses stand everywhere).
    let lc_path = repo_root.join("public/data/sf/sf-land-cover.bin.gz");
    let lc_bytes = std::fs::read(&lc_path).ok();
    let lc_sha = lc_bytes.as_ref().map(|b| hex(&Sha256::digest(b))).unwrap_or_default();
    let key = cache_key(&grid_sha, &lc_sha);
    let file = cache.map(|c| c.join("farshore").join(format!("{key}.eofs")));
    if let Some(f) = &file {
        if f.is_file() {
            match std::fs::read(f).map_err(anyhow::Error::from).and_then(|b| decode(&b)) {
                Ok(t) => return Ok(Some(t)),
                Err(e) => eprintln!("farshore: {} refused ({e:#}); cooking again", f.display()),
            }
        }
    }
    let Some(dem) = dem_root() else {
        eprintln!("farshore: no far terrain: no DEM folder ({}) and no cooked file is cached", eo_data::external::DEM.looked_in());
        return Ok(None);
    };
    let manifest = DemManifest::get();
    let missing: Vec<&str> = manifest.tiles.iter().filter(|t| !dem.join(&t.file).is_file()).map(|t| t.id.as_str()).collect();
    if !missing.is_empty() {
        eprintln!(
            "farshore: no far terrain: the USGS DEM tiles {} are not under {} (fetch them with `python3 scripts/setup-assets.py --only dem`, or set EO_DEM_ROOT) and no cooked file is cached",
            missing.join(", "),
            dem.display()
        );
        return Ok(None);
    }
    eprintln!("farshore: cooking the far terrain from the USGS DEM tiles under {}", dem.display());
    let grid = ElevationGrid::decode(&eo_data::read_maybe_gzip(&grid_path)?)?;
    let land_cover = lc_bytes.as_ref().and_then(|_| eo_data::read_maybe_gzip(&lc_path).ok()).and_then(|b| LandCover::decode(&b, Frame::sf()).ok());
    let t = bake(&dem, &grid, &grid_sha, land_cover.as_ref())?;
    if let Some(f) = &file {
        let bytes = encode(&t)?;
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
mod tests;
