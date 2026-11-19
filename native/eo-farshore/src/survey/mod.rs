//! The owner-approved survey set around the Golden Gate, read where it lies
//! (the folder `EO_SURVEY_ROOT` names; no default), every file checked
//! against its size and SHA-256 in `native/assets/survey-manifest.json`
//! (written by `native/tools/survey-manifest.mjs`) before it is read:
//!
//! * [`naip`]: USDA NAIP 0.6 m RGB aerial tiles (group A1);
//! * [`dem1m`]: USGS 3DEP 1 m DEMs (group E1, UTM zone 10N NAD83, NAVD88);
//! * [`bathy`]: NOAA NCEI CUDEM 1/9 arc-second topobathy (group B1);
//! * [`scan`]: Poly Haven scanned rocks and plants (groups S1, S2).
//!
//! Real readers, tested, called by nothing yet. With the variable unset, its
//! folder missing or a file not matching the manifest, `Root::discover` or
//! the reader answers None or an error and the caller draws as before.
//! Heights stay in their survey's datum (NAVD88 metres) and positions in
//! longitude and latitude (NAD83, which this area's survey-grade frames hold
//! within a metre of WGS84); game coordinates are the caller's
//! (`eo_data::Frame::sf`, the ground plan and the height scale).

pub mod bathy;
pub mod dem1m;
pub mod naip;
pub mod scan;

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use tiff::decoder::{Decoder, DecodingResult, Limits};
use tiff::tags::Tag;

/// The variable naming the survey folder.
pub const ROOT_ENV: &str = "EO_SURVEY_ROOT";

/// One file of the manifest.
#[derive(Clone, Debug, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
    #[serde(default)]
    pub url: Option<String>,
}

/// One set (a NAIP mosaic, a DEM tile, a scan) and its provenance.
#[derive(Clone, Debug, Deserialize)]
pub struct Set {
    pub group: String,
    pub id: String,
    pub publisher: String,
    pub licence: String,
    pub attribution: String,
    pub files: Vec<FileEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub root_env: String,
    pub sets: Vec<Set>,
}

/// The manifest built into the binary.
pub fn manifest() -> &'static Manifest {
    static M: OnceLock<Manifest> = OnceLock::new();
    M.get_or_init(|| serde_json::from_str(include_str!("../../../assets/survey-manifest.json")).expect("native/assets/survey-manifest.json"))
}

/// The manifest's entry for a root-relative path.
pub fn entry(rel: &str) -> Option<&'static FileEntry> {
    manifest().sets.iter().flat_map(|s| s.files.iter()).find(|f| f.path == rel)
}

/// The set holding `id` in `group`.
pub fn set(group: &str, id: &str) -> Option<&'static Set> {
    manifest().sets.iter().find(|s| s.group == group && s.id == id)
}

/// The survey folder.
#[derive(Clone, Debug)]
pub struct Root {
    dir: PathBuf,
}

impl Root {
    /// The folder `EO_SURVEY_ROOT` names, if it is set and is a folder.
    pub fn discover() -> Option<Root> {
        let v = std::env::var_os(ROOT_ENV).filter(|v| !v.is_empty())?;
        Root::at(PathBuf::from(v))
    }

    /// A survey folder at `dir`, if it is one.
    pub fn at(dir: PathBuf) -> Option<Root> {
        dir.is_dir().then_some(Root { dir })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The root-relative path `rel`, checked against the manifest (size and
    /// SHA-256; each file hashed once per process).
    pub fn verified(&self, rel: &str) -> Result<PathBuf> {
        let e = entry(rel).ok_or_else(|| anyhow!("{rel}: not in native/assets/survey-manifest.json"))?;
        let path = self.dir.join(rel);
        let meta = std::fs::metadata(&path).with_context(|| format!("{}", path.display()))?;
        if meta.len() != e.bytes {
            bail!("{rel}: {} bytes, the manifest says {}", meta.len(), e.bytes);
        }
        static DONE: OnceLock<Mutex<HashMap<PathBuf, (u64, Option<std::time::SystemTime>)>>> = OnceLock::new();
        let key = (meta.len(), meta.modified().ok());
        let done = DONE.get_or_init(Default::default);
        if done.lock().unwrap().get(&path) == Some(&key) {
            return Ok(path);
        }
        let digest = sha256_file(&path)?;
        if digest != e.sha256 {
            bail!("{rel}: sha256 {digest}, the manifest says {}", e.sha256);
        }
        done.lock().unwrap().insert(path.clone(), key);
        Ok(path)
    }
}

/// The SHA-256 of a file, hex.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path).with_context(|| format!("{}", path.display()))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 8 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Metres per degree of latitude and of longitude at `lat` (the WGS84/GRS80
/// ellipsoid's meridian and parallel radii), for a window's step.
pub fn metres_per_degree(lat: f64) -> (f64, f64) {
    let (a, e2) = (dem1m::GRS80_A, dem1m::grs80_e2());
    let s = lat.to_radians().sin();
    let w = (1.0 - e2 * s * s).sqrt();
    let m = a * (1.0 - e2) / (w * w * w);
    let n = a / w;
    (m * std::f64::consts::PI / 180.0, n * lat.to_radians().cos() * std::f64::consts::PI / 180.0)
}

/// A regular longitude/latitude grid of samples: sample (i, j) at
/// (`west + (i + 0.5) dlon`, `north - (j + 0.5) dlat`), row-major from the
/// north-west corner; NaN where the survey has no value.
#[derive(Clone, Debug)]
pub struct Window {
    pub west: f64,
    pub north: f64,
    pub dlon: f64,
    pub dlat: f64,
    pub cols: usize,
    pub rows: usize,
    pub values: Vec<f32>,
}

impl Window {
    /// The grid over [lon0, lon1] x [lat0, lat1] at about `step_m` metres,
    /// each sample from `f(lon, lat)`.
    pub fn sample(lon0: f64, lat0: f64, lon1: f64, lat1: f64, step_m: f64, f: &dyn Fn(f64, f64) -> Option<f32>) -> Result<Window> {
        let (west, east, south, north) = (lon0.min(lon1), lon0.max(lon1), lat0.min(lat1), lat0.max(lat1));
        if !(step_m > 0.0) || !(east > west) || !(north > south) {
            bail!("window: an empty box or a step of {step_m} m");
        }
        let (mlat, mlon) = metres_per_degree(0.5 * (north + south));
        let (dlat, dlon) = (step_m / mlat, step_m / mlon);
        let cols = ((east - west) / dlon).ceil().max(1.0) as usize;
        let rows = ((north - south) / dlat).ceil().max(1.0) as usize;
        if cols * rows > 64 << 20 {
            bail!("window: {cols} x {rows} samples is too many");
        }
        let mut values = Vec::with_capacity(cols * rows);
        for j in 0..rows {
            for i in 0..cols {
                values.push(f(west + (i as f64 + 0.5) * dlon, north - (j as f64 + 0.5) * dlat).unwrap_or(f32::NAN));
            }
        }
        Ok(Window { west, north, dlon, dlat, cols, rows, values })
    }

    pub fn at(&self, i: usize, j: usize) -> f32 {
        self.values[j * self.cols + i]
    }
}

/// A tiled float GeoTIFF read a 2-D chunk at a time (never the whole image):
/// pixel-is-area, pixel (c, r) covers model x `x0 + c dx ..`, y `y0 - r dy ..`
/// (a projected or a geographic model space). Chunks are kept in a small
/// cache; the decoder is behind a lock, so one raster serves many threads.
pub struct TiledRaster {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub tile: (u32, u32),
    /// The outer corner of pixel (0, 0) and the pixel size, model units.
    pub x0: f64,
    pub y0: f64,
    pub dx: f64,
    pub dy: f64,
    pub nodata: f32,
    dec: Mutex<Decoder<std::io::BufReader<std::fs::File>>>,
    chunks: Mutex<HashMap<u32, Arc<Vec<f32>>>>,
}

/// Chunks a raster keeps decoded (512 x 512 f32 = 1 MB each).
const CHUNK_CACHE: usize = 96;

impl TiledRaster {
    pub fn open(path: &Path) -> Result<TiledRaster> {
        let name = path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let mut limits = Limits::default();
        limits.decoding_buffer_size = 16 << 20;
        let mut dec = Decoder::new(std::io::BufReader::new(file)).with_context(|| format!("{name}: not a TIFF"))?.with_limits(limits);
        let (width, height) = dec.dimensions()?;
        if dec.get_chunk_type() != tiff::decoder::ChunkType::Tile {
            bail!("{name}: not tiled");
        }
        let tile = dec.chunk_dimensions();
        let scale = dec.get_tag(Tag::ModelPixelScaleTag)?.into_f64_vec()?;
        let tie = dec.get_tag(Tag::ModelTiepointTag)?.into_f64_vec()?;
        if scale.len() < 2 || tie.len() < 6 || tie[0] != 0.0 || tie[1] != 0.0 {
            bail!("{name}: no GeoTIFF pixel scale / tie point at pixel (0, 0)");
        }
        let nodata = match dec.find_tag(Tag::GdalNodata)? {
            Some(v) => v.into_string()?.trim_matches(char::from(0)).trim().parse::<f64>().unwrap_or(-999999.0) as f32,
            None => -999999.0,
        };
        Ok(TiledRaster { name, width, height, tile, x0: tie[3], y0: tie[4], dx: scale[0], dy: scale[1], nodata, dec: Mutex::new(dec), chunks: Mutex::new(HashMap::new()) })
    }

    fn chunk(&self, index: u32) -> Result<Arc<Vec<f32>>> {
        if let Some(c) = self.chunks.lock().unwrap().get(&index) {
            return Ok(c.clone());
        }
        let data = match self.dec.lock().unwrap().read_chunk(index).with_context(|| format!("{}: chunk {index}", self.name))? {
            DecodingResult::F32(v) => v,
            DecodingResult::F64(v) => v.into_iter().map(|x| x as f32).collect(),
            DecodingResult::I16(v) => v.into_iter().map(|x| x as f32).collect(),
            _ => bail!("{}: unexpected sample format", self.name),
        };
        let c = Arc::new(data);
        let mut cache = self.chunks.lock().unwrap();
        if cache.len() >= CHUNK_CACHE {
            cache.clear();
        }
        cache.insert(index, c.clone());
        Ok(c)
    }

    /// Pixel (c, r)'s value (None off the raster or at no data).
    pub fn pixel(&self, c: i64, r: i64) -> Option<f32> {
        if c < 0 || r < 0 || c >= self.width as i64 || r >= self.height as i64 {
            return None;
        }
        let (tw, th) = (self.tile.0 as i64, self.tile.1 as i64);
        let across = self.width.div_ceil(self.tile.0) as i64;
        let index = ((r / th) * across + c / tw) as u32;
        let chunk = self.chunk(index).map_err(|e| eprintln!("survey: {e:#}")).ok()?;
        // an edge chunk holds only its data columns (tiff `chunk_data_dimensions`).
        let data_w = (self.width as i64 - (c / tw) * tw).min(tw);
        let v = *chunk.get(((r % th) * data_w + c % tw) as usize)?;
        (v.is_finite() && v != self.nodata && v > -1.0e30).then_some(v)
    }

    /// The value at model (x, y), bilinear between pixel centres (None when
    /// any of the four is missing).
    pub fn at(&self, x: f64, y: f64) -> Option<f32> {
        let fc = (x - self.x0) / self.dx - 0.5;
        let fr = (self.y0 - y) / self.dy - 0.5;
        let (c, r) = (fc.floor() as i64, fr.floor() as i64);
        let (u, v) = ((fc - c as f64) as f32, (fr - r as f64) as f32);
        let p00 = self.pixel(c, r)?;
        let p10 = self.pixel(c + 1, r)?;
        let p01 = self.pixel(c, r + 1)?;
        let p11 = self.pixel(c + 1, r + 1)?;
        Some((p00 * (1.0 - u) + p10 * u) * (1.0 - v) + (p01 * (1.0 - u) + p11 * u) * v)
    }

    /// The model box: (x min, y min, x max, y max).
    pub fn bounds(&self) -> [f64; 4] {
        [self.x0, self.y0 - self.height as f64 * self.dy, self.x0 + self.width as f64 * self.dx, self.y0]
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The survey root for a test, or None (the test then skips, saying so).
    pub fn root_or_skip(what: &str) -> Option<Root> {
        let r = Root::discover();
        if r.is_none() {
            eprintln!("{what}: skipped, {ROOT_ENV} not set or not a folder");
        }
        r
    }

    #[test]
    fn the_manifest_lists_every_group_with_its_licence() {
        let m = manifest();
        assert_eq!(m.root_env, ROOT_ENV);
        for g in ["A1", "E1", "B1", "S1", "S2"] {
            let sets: Vec<_> = m.sets.iter().filter(|s| s.group == g).collect();
            assert!(!sets.is_empty(), "{g}");
            for s in sets {
                assert!(!s.files.is_empty() && !s.licence.is_empty() && !s.attribution.is_empty(), "{g} {}", s.id);
                assert!(s.files.iter().all(|f| f.sha256.len() == 64 && f.bytes > 0));
            }
        }
        assert_eq!(m.sets.iter().filter(|s| s.group == "A1").flat_map(|s| &s.files).count(), 20);
        assert!(entry("MANIFEST.json").is_none());
    }

    #[test]
    fn a_missing_root_is_none_and_a_wrong_file_is_refused() {
        assert!(Root::at(std::env::temp_dir().join(format!("eo-survey-missing-{}", std::process::id()))).is_none());
        let dir = std::env::temp_dir().join(format!("eo-survey-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("A1/rgb")).unwrap();
        std::fs::write(dir.join("A1/rgb/r000_c000.jpg"), b"not the tile").unwrap();
        let r = Root::at(dir.clone()).unwrap();
        assert!(r.verified("A1/rgb/r000_c000.jpg").is_err());
        assert!(r.verified("A1/rgb/r009_c009.jpg").is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn windows_cover_their_box_at_the_step() {
        let w = Window::sample(-122.48, 37.80, -122.47, 37.81, 10.0, &|lon, lat| Some((lon + lat) as f32)).unwrap();
        // 0.01 degree is about 1111 m north-south and 879 m east-west here.
        assert!((w.rows as i64 - 112).abs() <= 1 && (w.cols as i64 - 88).abs() <= 1, "{} x {}", w.cols, w.rows);
        assert!((w.at(0, 0) as f64 - (-122.48 + 0.5 * w.dlon + 37.81 - 0.5 * w.dlat)).abs() < 1e-3);
        let (mlat, mlon) = metres_per_degree(37.8);
        assert!((mlat - 111_012.0).abs() < 50.0 && (mlon - 88_000.0).abs() < 100.0, "{mlat} {mlon}");
    }
}
