//! each building's roof colour as measured from straight-down aerial
//! imagery (USDA NAIP through the USGS National Map, public domain), and the
//! mapping between the game's plan metres and longitude/latitude the
//! measuring tool uses (`eo-kit/examples/roof_survey.rs`).
//!
//! The table (`native/assets/roof-colours-v1.bin.gz`) is keyed by the source
//! record's key (`12345` for OSM, `ds12` for the DataSF pack) through a 32-bit
//! FNV-1a hash; a hash two records share is left out by the tool. Format, all
//! little-endian, the whole file gzipped:
//!
//! ```text
//! b"EORC" u8 version (1) u8 columns (10 in v1; a reader takes the columns it knows and
//!   skips later ones, so fields such as a wall tint can be appended) u16 0
//! u32 count
//! u32 imagery digest (first 4 bytes of the SHA-256 of the imagery manifest)
//! count x LEB128 varint: key hashes, ascending, each the delta from the last
//! 10 columns of count bytes: r g b (sRGB of the roof), confidence (0-255),
//!   r2 g2 b2 (a second roof colour), mix (high nibble: the second colour's
//!   share of the roof in 1/15, low nibble: vegetation's share of the
//!   footprint in 1/15), dx dz (i8: where the second colour lies, its
//!   centroid from the footprint centroid in units of half the square root of
//!   the footprint area, x east z south, /127)
//! ```
//!
//! With no file (or `EO_ROOF_COLOURS=off`, eo-app `--no-roof-colours`) nothing
//! is installed and every caller keeps today's behaviour exactly.

use anyhow::{Result, bail};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The table's file under the repository root.
pub const TABLE_PATH: &str = "native/assets/roof-colours-v1.bin.gz";
pub const MAGIC: &[u8; 4] = b"EORC";
pub const VERSION: u8 = 1;
/// Byte columns per record this build writes and reads (later ones are skipped).
pub const COLUMNS: usize = 10;

/// 32-bit FNV-1a of a record key.
pub fn key_hash(key: &str) -> u32 {
    let mut h: u32 = 0x811c9dc5;
    for b in key.as_bytes() {
        h ^= *b as u32;
        h = h.wrapping_mul(0x01000193);
    }
    h
}

/// One building's measured roof.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoofMeasure {
    /// The roof's colour, sRGB bytes (a trimmed median in OKLab).
    pub rgb: [u8; 3],
    /// 0-1: how far the colour can be trusted (sample count, spread, lean, trees).
    pub confidence: f32,
    /// A second roof colour and its share of the roof (0 when none).
    pub second: [u8; 3],
    pub second_share: f32,
    /// Where the second colour lies: its centroid from the footprint
    /// centroid, in units of half the square root of the footprint area.
    pub second_at: [f32; 2],
    /// Vegetation's share of the footprint (trees over the roof, roof gardens).
    pub vegetation: f32,
}

/// One record as the tool writes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawEntry {
    pub hash: u32,
    pub bytes: [u8; COLUMNS],
}

impl RawEntry {
    pub fn from_measure(hash: u32, m: &RoofMeasure) -> Self {
        let q = |v: f32| ((v.clamp(0.0, 1.0) * 15.0).round() as u8).min(15);
        let c = (m.confidence.clamp(0.0, 1.0) * 255.0).round() as u8;
        let s = |v: f32| ((v.clamp(-1.0, 1.0) * 127.0).round() as i8) as u8;
        RawEntry { hash, bytes: [m.rgb[0], m.rgb[1], m.rgb[2], c, m.second[0], m.second[1], m.second[2], (q(m.second_share) << 4) | q(m.vegetation), s(m.second_at[0]), s(m.second_at[1])] }
    }
    pub fn measure(&self) -> RoofMeasure {
        let b = self.bytes;
        RoofMeasure {
            rgb: [b[0], b[1], b[2]],
            confidence: b[3] as f32 / 255.0,
            second: [b[4], b[5], b[6]],
            second_share: (b[7] >> 4) as f32 / 15.0,
            vegetation: (b[7] & 15) as f32 / 15.0,
            second_at: [b[8] as i8 as f32 / 127.0, b[9] as i8 as f32 / 127.0],
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct RoofTable {
    hashes: Vec<u32>,
    data: Vec<[u8; COLUMNS]>,
    pub digest: u32,
}

impl RoofTable {
    /// From entries in any order; hashes that occur twice are dropped (both).
    pub fn from_entries(mut entries: Vec<RawEntry>, digest: u32) -> Self {
        entries.sort_by_key(|e| e.hash);
        let mut t = RoofTable { digest, ..Default::default() };
        let mut i = 0;
        while i < entries.len() {
            let mut j = i + 1;
            while j < entries.len() && entries[j].hash == entries[i].hash {
                j += 1;
            }
            if j == i + 1 {
                t.hashes.push(entries[i].hash);
                t.data.push(entries[i].bytes);
            }
            i = j;
        }
        t
    }

    pub fn len(&self) -> usize {
        self.hashes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.hashes.is_empty()
    }

    pub fn get(&self, key: &str) -> Option<RoofMeasure> {
        self.get_hash(key_hash(key))
    }

    pub fn get_hash(&self, h: u32) -> Option<RoofMeasure> {
        let i = self.hashes.binary_search(&h).ok()?;
        Some(RawEntry { hash: h, bytes: self.data[i] }.measure())
    }

    pub fn entries(&self) -> impl Iterator<Item = RawEntry> + '_ {
        self.hashes.iter().zip(&self.data).map(|(h, b)| RawEntry { hash: *h, bytes: *b })
    }

    /// The file's bytes before gzip.
    pub fn encode(&self) -> Vec<u8> {
        let n = self.hashes.len();
        let mut out = Vec::with_capacity(16 + n * 11);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&[VERSION, COLUMNS as u8, 0, 0]);
        out.extend_from_slice(&(n as u32).to_le_bytes());
        out.extend_from_slice(&self.digest.to_le_bytes());
        let mut last = 0u32;
        for &h in &self.hashes {
            let mut d = h - last;
            last = h;
            loop {
                let b = (d & 0x7f) as u8;
                d >>= 7;
                if d == 0 {
                    out.push(b);
                    break;
                }
                out.push(b | 0x80);
            }
        }
        for c in 0..COLUMNS {
            out.extend(self.data.iter().map(|d| d[c]));
        }
        out
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() < 16 || &b[0..4] != MAGIC {
            bail!("roof colours: not an EORC table");
        }
        if b[4] != VERSION {
            bail!("roof colours: version {} (this build reads {VERSION})", b[4]);
        }
        let n = u32::from_le_bytes(b[8..12].try_into().unwrap()) as usize;
        let digest = u32::from_le_bytes(b[12..16].try_into().unwrap());
        let mut at = 16;
        let mut hashes = Vec::with_capacity(n);
        let mut last = 0u32;
        for _ in 0..n {
            let mut v = 0u32;
            let mut shift = 0;
            loop {
                let Some(&byte) = b.get(at) else { bail!("roof colours: truncated hashes") };
                at += 1;
                if shift > 28 {
                    bail!("roof colours: bad varint");
                }
                v |= ((byte & 0x7f) as u32) << shift;
                shift += 7;
                if byte & 0x80 == 0 {
                    break;
                }
            }
            last = last.checked_add(v).ok_or_else(|| anyhow::anyhow!("roof colours: hash overflow"))?;
            if !hashes.is_empty() && v == 0 {
                bail!("roof colours: repeated hash");
            }
            hashes.push(last);
        }
        let cols = b[5] as usize;
        if cols < COLUMNS || b.len() != at + n * cols {
            bail!("roof colours: {} bytes after the hashes, expected {}", b.len() - at, n * cols);
        }
        let mut data = vec![[0u8; COLUMNS]; n];
        for c in 0..COLUMNS {
            for (i, d) in data.iter_mut().enumerate() {
                d[c] = b[at + c * n + i];
            }
        }
        Ok(RoofTable { hashes, data, digest })
    }

    pub fn to_gzip(&self) -> Vec<u8> {
        use std::io::Write;
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        e.write_all(&self.encode()).unwrap();
        e.finish().unwrap()
    }

    pub fn read(path: &Path) -> Result<Self> {
        Self::decode(&crate::read_maybe_gzip(path)?)
    }
}

/// This workspace's `native/assets` folder (beside the crates; the same folder
/// as `<repo root>/native/assets` in the repository).
pub fn native_assets() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets")
}

/// Where the table is read from: `EO_ROOF_COLOURS=<path>` overrides, `off`
/// (or `0`) disables; else `roof-colours-v1.bin.gz` in `native_assets()`, or
/// failing that `TABLE_PATH` under the repository root.
pub fn table_path(repo_root: &Path) -> Option<PathBuf> {
    match std::env::var("EO_ROOF_COLOURS") {
        Ok(v) if v == "off" || v == "0" => None,
        Ok(v) if !v.is_empty() => Some(PathBuf::from(v)),
        _ => {
            let own = native_assets().join("roof-colours-v1.bin.gz");
            Some(if own.is_file() { own } else { repo_root.join(TABLE_PATH) })
        }
    }
}

static TABLE: OnceLock<Option<RoofTable>> = OnceLock::new();

/// Load the table once a process (absent or unreadable: none, today's look).
/// Returns the table in use.
pub fn install(repo_root: &Path) -> Option<&'static RoofTable> {
    TABLE
        .get_or_init(|| {
            let p = table_path(repo_root)?;
            if !p.is_file() {
                eprintln!("roofs: no roof colour table at {} (the finishes as before)", p.display());
                return None;
            }
            match RoofTable::read(&p) {
                Ok(t) => {
                    eprintln!("roofs: {} measured roof colours from {}", t.len(), p.display());
                    Some(t)
                }
                Err(e) => {
                    eprintln!("roofs: {e:#} (the finishes as before)");
                    None
                }
            }
        })
        .as_ref()
}

/// The installed table (None until `install`, and when absent or switched off).
pub fn table() -> Option<&'static RoofTable> {
    TABLE.get().and_then(|t| t.as_ref())
}

/// Tests and tools: install a given table (first call wins).
pub fn install_table(t: Option<RoofTable>) -> bool {
    TABLE.set(t).is_ok()
}

/// A tag for the kit tile cache key: empty with no table, else the table's
/// digest and size (and the contrast switch, which changes the tints).
pub fn cache_tag() -> String {
    match table() {
        Some(t) => format!("roof-colours-v{VERSION}-{:08x}-{}{}", t.digest, t.len(), if feature_on("contrast") { "" } else { "-nocontrast" }),
        None => String::new(),
    }
}

/// The far roof features (rules, applied with or without a table) and their
/// switches, for the far layer's cache key: `EO_ROOF_PARAPET`, `EO_ROOF_PATCHES`,
/// `EO_ROOF_PLANT` (=off; eo-app `--no-roof-parapet` and so on). With all of
/// them off the tag is empty and the far key is today's.
pub const FAR_FEATURES: [&str; 3] = ["parapet", "patches", "plant"];
/// Bump when what a far feature bakes changes.
pub const FAR_FEATURES_VERSION: u32 = 2; // 2: the patch splits along the building

pub fn far_cache_tag() -> String {
    let on: Vec<&str> = FAR_FEATURES.iter().copied().filter(|f| feature_on(f)).collect();
    if on.is_empty() { String::new() } else { format!("roof-far-v{FAR_FEATURES_VERSION}-{}", on.join("+")) }
}

/// `EO_ROOF_<NAME>=off` (or 0) switches one roof feature off (eo-app `--no-roof-<name>`).
pub fn feature_on(name: &str) -> bool {
    !matches!(std::env::var(format!("EO_ROOF_{}", name.to_ascii_uppercase())).as_deref(), Ok("off") | Ok("0"))
}

// ---------------------------------------------------------------------------
// Colour helpers shared by the tool and the renderers.
// ---------------------------------------------------------------------------

pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

pub fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

pub fn rgb8_linear(c: [u8; 3]) -> [f32; 3] {
    c.map(|v| srgb_to_linear(v as f32 / 255.0))
}

/// Linear sRGB to OKLab.
pub fn oklab(c: [f32; 3]) -> [f32; 3] {
    let l = 0.4122214708 * c[0] + 0.5363325363 * c[1] + 0.0514459929 * c[2];
    let m = 0.2119034982 * c[0] + 0.6806995451 * c[1] + 0.1073969566 * c[2];
    let s = 0.0883024619 * c[0] + 0.2817188376 * c[1] + 0.6299787005 * c[2];
    let (l, m, s) = (l.max(0.0).cbrt(), m.max(0.0).cbrt(), s.max(0.0).cbrt());
    [
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    ]
}

/// OKLab to linear sRGB.
pub fn oklab_to_linear(c: [f32; 3]) -> [f32; 3] {
    let l = c[0] + 0.3963377774 * c[1] + 0.2158037573 * c[2];
    let m = c[0] - 0.1055613458 * c[1] - 0.0638541728 * c[2];
    let s = c[0] - 0.0894841775 * c[1] - 1.2914855480 * c[2];
    let (l, m, s) = (l * l * l, m * m * m, s * s * s);
    [
        4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
        -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
        -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s,
    ]
}

// ---------------------------------------------------------------------------
// Plan metres <-> longitude/latitude.
// ---------------------------------------------------------------------------

/// The game's survey frame (`sf-city.json` `meta`): x east, z south, metres
/// about `center`, `metersPerDegree*` per degree. Footprints are surveyed at
/// 1:1 (`City::load_at(paths, 1.0)`); the drawn ground plan is that times
/// `groundscale::scale()` about the origin, so a drawn point unprojects
/// through `drawn_to_lonlat` with the scale divided out.
#[derive(Clone, Copy, Debug)]
pub struct PlanFrame {
    pub center_lat: f64,
    pub center_lon: f64,
    pub m_per_deg_lat: f64,
    pub m_per_deg_lon: f64,
}

impl PlanFrame {
    pub fn from_meta(meta: &crate::CityMeta) -> Self {
        PlanFrame {
            center_lat: meta.center.lat,
            center_lon: meta.center.lon,
            m_per_deg_lat: meta.projection.meters_per_degree_lat,
            m_per_deg_lon: meta.projection.meters_per_degree_lon,
        }
    }
    /// Surveyed plan metres (x east, z south) to (lon, lat).
    pub fn to_lonlat(&self, x: f64, z: f64) -> (f64, f64) {
        (self.center_lon + x / self.m_per_deg_lon, self.center_lat - z / self.m_per_deg_lat)
    }
    /// (lon, lat) to surveyed plan metres.
    pub fn to_plan(&self, lon: f64, lat: f64) -> (f64, f64) {
        ((lon - self.center_lon) * self.m_per_deg_lon, (self.center_lat - lat) * self.m_per_deg_lat)
    }
    /// A point of the drawn (scaled) ground plan to (lon, lat).
    pub fn drawn_to_lonlat(&self, x: f64, z: f64, scale: f64) -> (f64, f64) {
        self.to_lonlat(x / scale, z / scale)
    }
}

// ---------------------------------------------------------------------------
// The ground between buildings: green share per cell (written by the tool for
// the far ground; not read by the game in this pass).
// ---------------------------------------------------------------------------

/// `native/assets/ground-green-v1.bin.gz`, little-endian, gzipped:
///
/// ```text
/// b"EOGG" u8 version (1) u8 0 u16 0
/// f32 x0, f32 z0   surveyed plan metres of the grid's north-west corner
/// f32 cell         cell side, surveyed metres (drawn: x groundscale)
/// u32 nx, u32 nz
/// nx*nz u8  green: vegetation's share of the cell's open ground (pixels
///           outside every footprint), 0-250; 255 = no open ground sampled
/// nx*nz u8  built: footprints' share of the cell, 0-250
/// ```
/// Cell (i, j) covers x0 + i*cell .. x0 + (i+1)*cell, z0 + j*cell .. (row-major, j outer).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GreenGrid {
    pub x0: f32,
    pub z0: f32,
    pub cell: f32,
    pub nx: u32,
    pub nz: u32,
    pub green: Vec<u8>,
    pub built: Vec<u8>,
}

pub const GREEN_PATH: &str = "native/assets/ground-green-v1.bin.gz";

impl GreenGrid {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(28 + self.green.len() * 2);
        out.extend_from_slice(b"EOGG");
        out.extend_from_slice(&[1, 0, 0, 0]);
        for v in [self.x0, self.z0, self.cell] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&self.nx.to_le_bytes());
        out.extend_from_slice(&self.nz.to_le_bytes());
        out.extend_from_slice(&self.green);
        out.extend_from_slice(&self.built);
        out
    }
    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() < 28 || &b[0..4] != b"EOGG" || b[4] != 1 {
            bail!("ground green: not an EOGG v1 grid");
        }
        let f = |at: usize| f32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        let u = |at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        let (nx, nz) = (u(20), u(24));
        let n = nx as usize * nz as usize;
        if b.len() != 28 + 2 * n {
            bail!("ground green: size mismatch");
        }
        Ok(GreenGrid { x0: f(8), z0: f(12), cell: f(16), nx, nz, green: b[28..28 + n].to_vec(), built: b[28 + n..].to_vec() })
    }
    /// Green share of open ground at a surveyed point (None: no data).
    pub fn green_at(&self, x: f64, z: f64) -> Option<f32> {
        let i = ((x - self.x0 as f64) / self.cell as f64).floor();
        let j = ((z - self.z0 as f64) / self.cell as f64).floor();
        if i < 0.0 || j < 0.0 || i >= self.nx as f64 || j >= self.nz as f64 {
            return None;
        }
        let v = self.green[j as usize * self.nx as usize + i as usize];
        (v != 255).then(|| v as f32 / 250.0)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn green_grid_round_trips() {
        let g = super::GreenGrid { x0: -100.0, z0: -50.0, cell: 16.0, nx: 3, nz: 2, green: vec![0, 125, 250, 255, 10, 20], built: vec![1, 2, 3, 4, 5, 6] };
        let u = super::GreenGrid::decode(&g.encode()).unwrap();
        assert_eq!(u, g);
        assert_eq!(u.green_at(-99.0, -49.0), Some(0.0));
        assert_eq!(u.green_at(-80.0, -49.0), Some(0.5));
        assert_eq!(u.green_at(-60.0, -49.0), Some(1.0));
        assert_eq!(u.green_at(-99.0, -20.0), None);
        assert_eq!(u.green_at(-101.0, -49.0), None);
    }

    use super::*;

    fn sample(n: u32) -> Vec<RawEntry> {
        (0..n)
            .map(|i| {
                let m = RoofMeasure { rgb: [i as u8, (i * 7) as u8, 200], confidence: (i % 256) as f32 / 255.0, second: [10, 20, 30], second_share: 0.4, second_at: [0.5, -1.0], vegetation: 0.2 };
                RawEntry::from_measure(key_hash(&format!("ds{i}")), &m)
            })
            .collect()
    }

    #[test]
    fn table_round_trips() {
        let t = RoofTable::from_entries(sample(5000), 0xdeadbeef);
        assert_eq!(t.len(), 5000);
        let b = t.encode();
        let u = RoofTable::decode(&b).unwrap();
        assert_eq!(u.digest, 0xdeadbeef);
        assert_eq!(u.len(), 5000);
        let m = u.get("ds77").unwrap();
        assert_eq!(m.rgb, [77, (77 * 7) as u8, 200]);
        assert!((m.confidence - 77.0 / 255.0).abs() < 1e-6);
        assert_eq!(m.second, [10, 20, 30]);
        assert!((m.second_share - 6.0 / 15.0).abs() < 1e-6);
        assert!((m.vegetation - 3.0 / 15.0).abs() < 1e-6);
        assert!((m.second_at[0] - 64.0 / 127.0).abs() < 1e-6 && m.second_at[1] == -1.0);
        assert!(u.get("ds5000").is_none());
        // gzip path
        let g = t.to_gzip();
        let dir = std::env::temp_dir().join(format!("eo-roofs-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.bin.gz");
        std::fs::write(&p, &g).unwrap();
        let r = RoofTable::read(&p).unwrap();
        assert_eq!(r.entries().collect::<Vec<_>>(), t.entries().collect::<Vec<_>>());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn later_columns_are_skipped() {
        // A later writer appends a column: this reader still takes its own ten.
        let t = RoofTable::from_entries(sample(50), 7);
        let b = t.encode();
        let n = t.len();
        let mut x = b[..b.len() - n * COLUMNS].to_vec();
        x[5] = (COLUMNS + 1) as u8;
        x.extend_from_slice(&b[b.len() - n * COLUMNS..]);
        x.extend(std::iter::repeat_n(9u8, n));
        let u = RoofTable::decode(&x).unwrap();
        assert_eq!(u.entries().collect::<Vec<_>>(), t.entries().collect::<Vec<_>>());
    }

    #[test]
    fn colliding_hashes_are_dropped() {
        let mut e = sample(10);
        let dup = RawEntry { hash: e[3].hash, bytes: [1; COLUMNS] };
        e.push(dup);
        let t = RoofTable::from_entries(e, 0);
        assert_eq!(t.len(), 9);
        assert!(t.get("ds3").is_none());
    }

    #[test]
    fn rejects_corrupt_tables() {
        let t = RoofTable::from_entries(sample(100), 1);
        let mut b = t.encode();
        assert!(RoofTable::decode(&b[..b.len() - 1]).is_err());
        b[0] = b'X';
        assert!(RoofTable::decode(&b).is_err());
        let mut b = t.encode();
        b[4] = 9;
        assert!(RoofTable::decode(&b).is_err());
    }

    #[test]
    fn plan_frame_round_trips_and_scales() {
        let f = PlanFrame { center_lat: 37.778, center_lon: -122.4194, m_per_deg_lat: 110574.0, m_per_deg_lon: 111320.0 * 37.778f64.to_radians().cos() };
        let (lon, lat) = f.to_lonlat(1000.0, -2000.0);
        assert!(lat > 37.778 && lon > -122.4194);
        // 2 km north is 2000 / 110574 degrees.
        assert!((lat - 37.778 - 2000.0 / 110574.0).abs() < 1e-12);
        let (x, z) = f.to_plan(lon, lat);
        assert!((x - 1000.0).abs() < 1e-6 && (z + 2000.0).abs() < 1e-6);
        let (lon2, lat2) = f.drawn_to_lonlat(1250.0, -2500.0, 1.25);
        assert!((lon2 - lon).abs() < 1e-12 && (lat2 - lat).abs() < 1e-12);
    }

    #[test]
    fn oklab_round_trips() {
        for c in [[0.2f32, 0.5, 0.8], [0.9, 0.9, 0.85], [0.02, 0.03, 0.01]] {
            let b = oklab_to_linear(oklab(c));
            for k in 0..3 {
                assert!((b[k] - c[k]).abs() < 1e-4, "{c:?} {b:?}");
            }
        }
        assert!((oklab([1.0, 1.0, 1.0])[0] - 1.0).abs() < 1e-3);
    }
}
