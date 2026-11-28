//! The lidar trees (Docs/HILLS-PLAN.md, stage H6c): every tree crown the
//! USGS 3DEP lidar finds in the city's hill parks, with its position, height
//! and crown radius, cooked by `cook trees` (`examples/cook/trees.rs`) from
//! the point cloud `native/tools/fetch-lidar.py` fetches into
//! `external/lidar/<area>/`.
//!
//! An area is a set of the city's mapped green polygons (the parks'
//! rings, surveyed plan); the file holds the trees whose top lies inside
//! them and the rings themselves, so the runtime knows where the lidar
//! speaks for the trees ([`LidarTrees::covers`]) and where the greens'
//! lattice still plants them.
//!
//! # File
//!
//! `<cache>/hills/trees-v1.bin` ([`file_path`]; [`ENV`] names another file):
//! `b"EOHT"`, u32 version (1), u32 header length, the header JSON
//! ([`Header`]: the sources and their digests, the method's constants, the
//! areas with their rings and statistics), then [`Header::trees`] records of
//! [`RECORD_BYTES`] bytes, little-endian: x, z (surveyed plan metres, x east,
//! z south), height above the ground at the trunk (m), crown radius (m, the
//! radius of a circle of the crown's area), the ground under the trunk
//! (NAVD88 m) as f32, then the area's index (u16) and flags (u16,
//! [`FLAG_GRID_GROUND`]).

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const MAGIC: &[u8; 4] = b"EOHT";
pub const VERSION: u32 = 1;
pub const FILE_NAME: &str = "trees-v1.bin";
pub const FORMAT: &str = "f32x5-u16x2-records";
pub const RECORD_BYTES: usize = 24;
/// A file to read instead of `<cache>/hills/trees-v1.bin`.
pub const ENV: &str = "EO_HILLS_TREES";
/// [`Tree::flags`]: the height is over the 8 m elevation grid's ground (no
/// lidar ground return round the trunk, and the filled lidar ground parted from the grid's).
pub const FLAG_GRID_GROUND: u16 = 1;
/// The coverage raster's cell (surveyed plan metres).
pub const COVER_CELL_M: f64 = 2.0;
/// The runtime index's cell (surveyed plan metres).
const INDEX_CELL_M: f64 = 64.0;

/// `<cache>/hills/trees-v1.bin`.
pub fn file_path(cache: &Path) -> PathBuf {
    cache.join("hills").join(FILE_NAME)
}

/// The file the runtime reads: [`ENV`] when set and not empty, else
/// [`file_path`] in the hill builder's cache.
pub fn resolve_path() -> PathBuf {
    match std::env::var_os(ENV).filter(|v| !v.is_empty()) {
        Some(p) => PathBuf::from(p),
        None => file_path(&crate::cache_dir()),
    }
}

/// One tree the lidar measured.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tree {
    /// The crown's top, surveyed plan metres (x east, z south).
    pub x: f32,
    pub z: f32,
    /// The top over the ground at the trunk (m).
    pub height_m: f32,
    /// The radius of a circle of the crown's area (m).
    pub crown_r_m: f32,
    /// The ground under the trunk (NAVD88 m; the lidar's, or the grid's with [`FLAG_GRID_GROUND`]).
    pub ground_navd88_m: f32,
    /// Index into [`Header::areas`].
    pub area: u16,
    pub flags: u16,
}

impl Tree {
    fn to_bytes(self) -> [u8; RECORD_BYTES] {
        let mut b = [0u8; RECORD_BYTES];
        for (k, v) in [self.x, self.z, self.height_m, self.crown_r_m, self.ground_navd88_m].iter().enumerate() {
            b[4 * k..4 * k + 4].copy_from_slice(&v.to_le_bytes());
        }
        b[20..22].copy_from_slice(&self.area.to_le_bytes());
        b[22..24].copy_from_slice(&self.flags.to_le_bytes());
        b
    }
    fn from_bytes(b: &[u8]) -> Tree {
        let f = |k: usize| f32::from_le_bytes(b[4 * k..4 * k + 4].try_into().unwrap());
        Tree { x: f(0), z: f(1), height_m: f(2), crown_r_m: f(3), ground_navd88_m: f(4), area: u16::from_le_bytes([b[20], b[21]]), flags: u16::from_le_bytes([b[22], b[23]]) }
    }
}

/// Percentiles 10, 25, 50, 75 and 90, and the largest value.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
pub struct Spread {
    pub p10: f32,
    pub p25: f32,
    pub p50: f32,
    pub p75: f32,
    pub p90: f32,
    pub max: f32,
}

impl Spread {
    pub fn of(v: &mut [f32]) -> Spread {
        if v.is_empty() {
            return Spread::default();
        }
        v.sort_by(|a, b| a.total_cmp(b));
        let q = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
        Spread { p10: q(0.1), p25: q(0.25), p50: q(0.5), p75: q(0.75), p90: q(0.9), max: v[v.len() - 1] }
    }
}

/// What the cook found in one area.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct AreaStats {
    pub trees: u32,
    pub height_m: Spread,
    pub crown_r_m: Spread,
    /// The rings' area (m^2) and the share of it the lidar reads as canopy (vegetation 2 m or more up).
    pub area_m2: f64,
    pub canopy_share: f64,
    /// Points read inside the cook's box, and their density over the box (points a square metre).
    pub points: u64,
    pub points_per_m2: f64,
    /// Ground returns a square metre (the floor under the canopy).
    pub ground_per_m2: f64,
    /// Returns by ASPRS class (class number to count), inside the box.
    pub classes: BTreeMap<String, u64>,
    /// The share of the trees whose top the city's NAIP planting map (eo-hills `planting`)
    /// reads as canopy (the park trees' rule: canopy share at least
    /// `NAIP_TREE_CANOPY` in a box of `NAIP_TREE_BOX_M` drawn metres); None without the map.
    pub naip_canopy_agree: Option<f64>,
    /// The share of the rings' canopy cells (lidar) whose 2 m texel the NAIP map
    /// classes as canopy, and of the NAIP map's canopy texels in the rings the lidar
    /// reads as canopy; None without the map.
    pub lidar_canopy_in_naip: Option<f64>,
    pub naip_canopy_in_lidar: Option<f64>,
    /// Trees whose ground is the elevation grid's ([`FLAG_GRID_GROUND`]); trees
    /// over the cook's tallest, kept at it (a crown over a drop read against its foot).
    pub ground_from_grid: u32,
    pub clamped_tall: u32,
    /// |lidar ground - grid ground| under the trunks (m), and the grid's datum
    /// offset to the lidar's NAVD88 (the median over measured ground cells).
    pub ground_gap_m: Spread,
    pub grid_offset_m: f64,
}

/// One area: the parks' rings and the files the trees came from.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Area {
    pub name: String,
    pub label: String,
    /// The OSM ids of the greens (`public/data/sf/sf-lots-parks.json.gz`).
    pub greens: Vec<String>,
    /// The rings on the surveyed plan (metres), clipped as fetched.
    pub rings: Vec<Vec<[f64; 2]>>,
    /// The deepest octree level fetched.
    pub max_depth: u32,
    pub files: u32,
    pub bytes: u64,
    /// SHA-256 of the area's `PROVENANCE.json`, and of its files' digests in path order.
    pub provenance_sha256: String,
    pub files_sha256: String,
    pub stats: AreaStats,
}

/// The file's JSON header.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Header {
    pub format: String,
    pub record_bytes: u32,
    pub generator: String,
    /// The plan the positions are on.
    pub plan: String,
    pub source: String,
    pub source_url: String,
    pub licence: String,
    pub attribution: String,
    /// The method's constants (the cook's), name to value.
    pub params: BTreeMap<String, f64>,
    /// The city files read (`public/data/sf/`), name to SHA-256.
    pub city_files: BTreeMap<String, String>,
    pub areas: Vec<Area>,
    pub trees: u32,
}

/// The trees and the ground they speak for.
pub struct LidarTrees {
    header: Header,
    trees: Vec<Tree>,
    /// 64 m cell to the run of `trees` in it (the trees are kept in cell order).
    index: HashMap<(i32, i32), (u32, u32)>,
    cover: Vec<Coverage>,
}

/// One area's rings as a raster of [`COVER_CELL_M`] cells.
struct Coverage {
    grid: crate::raster::PlanGrid,
    cells: Vec<bool>,
}

fn cell_of(x: f64, z: f64) -> (i32, i32) {
    ((x / INDEX_CELL_M).floor() as i32, (z / INDEX_CELL_M).floor() as i32)
}

impl LidarTrees {
    /// The trees (any order) and their header (its `trees` count is set here).
    pub fn new(mut header: Header, mut trees: Vec<Tree>) -> LidarTrees {
        trees.sort_by(|a, b| cell_of(a.x as f64, a.z as f64).cmp(&cell_of(b.x as f64, b.z as f64)).then(a.z.total_cmp(&b.z)).then(a.x.total_cmp(&b.x)));
        let mut index = HashMap::new();
        let mut i = 0usize;
        while i < trees.len() {
            let c = cell_of(trees[i].x as f64, trees[i].z as f64);
            let mut j = i + 1;
            while j < trees.len() && cell_of(trees[j].x as f64, trees[j].z as f64) == c {
                j += 1;
            }
            index.insert(c, (i as u32, j as u32));
            i = j;
        }
        header.trees = trees.len() as u32;
        let cover = header.areas.iter().map(|a| Coverage::of(&a.rings)).collect();
        LidarTrees { header, trees, index, cover }
    }

    pub fn open(path: &Path) -> Result<LidarTrees> {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        if bytes.len() < 12 || &bytes[0..4] != MAGIC {
            bail!("{}: not a lidar tree file", path.display());
        }
        let u32_at = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
        if u32_at(4) != VERSION {
            bail!("{}: version {}, expected {VERSION}", path.display(), u32_at(4));
        }
        let hl = u32_at(8) as usize;
        if 12 + hl > bytes.len() {
            bail!("{}: header cut short", path.display());
        }
        let header: Header = serde_json::from_slice(&bytes[12..12 + hl]).with_context(|| format!("{}: header", path.display()))?;
        if header.format != FORMAT || header.record_bytes as usize != RECORD_BYTES {
            bail!("{}: format {} with {}-byte records", path.display(), header.format, header.record_bytes);
        }
        let body = &bytes[12 + hl..];
        if body.len() != header.trees as usize * RECORD_BYTES {
            bail!("{}: {} bytes of records for {} trees", path.display(), body.len(), header.trees);
        }
        let trees: Vec<Tree> = body.chunks_exact(RECORD_BYTES).map(Tree::from_bytes).collect();
        if let Some(t) = trees.iter().find(|t| t.area as usize >= header.areas.len() || !(t.height_m.is_finite() && t.crown_r_m.is_finite() && t.x.is_finite() && t.z.is_finite())) {
            bail!("{}: a record out of range: {t:?}", path.display());
        }
        Ok(LidarTrees::new(header, trees))
    }

    pub fn write(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_vec(&self.header)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let tmp = path.with_extension("bin.part");
        {
            let mut f = std::io::BufWriter::new(std::fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?);
            f.write_all(MAGIC)?;
            f.write_all(&VERSION.to_le_bytes())?;
            f.write_all(&(json.len() as u32).to_le_bytes())?;
            f.write_all(&json)?;
            for t in &self.trees {
                f.write_all(&t.to_bytes())?;
            }
            f.flush()?;
        }
        std::fs::rename(&tmp, path).with_context(|| format!("renaming {} to {}", tmp.display(), path.display()))?;
        Ok(())
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    pub fn trees(&self) -> &[Tree] {
        &self.trees
    }

    /// Whether the lidar speaks for the trees at surveyed plan (x, z): inside an area's rings.
    pub fn covers(&self, x: f64, z: f64) -> bool {
        self.cover.iter().any(|c| c.grid.texel_of(x, z).is_some_and(|(i, j)| c.cells[j * c.grid.width + i]))
    }

    /// The trees whose top lies in [x0, x1) x [z0, z1) of the surveyed plan.
    pub fn in_box(&self, x0: f64, z0: f64, x1: f64, z1: f64) -> impl Iterator<Item = &Tree> + '_ {
        let (c0, c1) = (cell_of(x0, z0), cell_of(x1, z1));
        (c0.1..=c1.1)
            .flat_map(move |cz| (c0.0..=c1.0).map(move |cx| (cx, cz)))
            .filter_map(|c| self.index.get(&c).copied())
            .flat_map(|(a, b)| self.trees[a as usize..b as usize].iter())
            .filter(move |t| (t.x as f64) >= x0 && (t.x as f64) < x1 && (t.z as f64) >= z0 && (t.z as f64) < z1)
    }
}

impl Coverage {
    fn of(rings: &[Vec<[f64; 2]>]) -> Coverage {
        let (mut x0, mut z0, mut x1, mut z1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for p in rings.iter().flatten() {
            x0 = x0.min(p[0]);
            z0 = z0.min(p[1]);
            x1 = x1.max(p[0]);
            z1 = z1.max(p[1]);
        }
        if x0 > x1 {
            let grid = crate::raster::PlanGrid { origin_x: 0.0, origin_z: 0.0, texel: COVER_CELL_M, width: 0, height: 0 };
            return Coverage { grid, cells: vec![] };
        }
        let c = COVER_CELL_M;
        let (ox, oz) = ((x0 / c).floor() * c, (z0 / c).floor() * c);
        let grid = crate::raster::PlanGrid { origin_x: ox, origin_z: oz, texel: c, width: ((x1 - ox) / c).ceil() as usize + 1, height: ((z1 - oz) / c).ceil() as usize + 1 };
        let mut cells = vec![false; grid.width * grid.height];
        // each ring on its own (an area's greens may overlap: their union, not even-odd).
        for r in rings.iter().filter(|r| r.len() >= 3) {
            crate::raster::fill_rings(&grid, &[r.as_slice()], |i, j| cells[j * grid.width + i] = true);
        }
        Coverage { grid, cells }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(rings: Vec<Vec<[f64; 2]>>) -> Header {
        Header {
            format: FORMAT.into(),
            record_bytes: RECORD_BYTES as u32,
            generator: "test".into(),
            plan: "surveyed".into(),
            source: String::new(),
            source_url: String::new(),
            licence: String::new(),
            attribution: String::new(),
            params: BTreeMap::new(),
            city_files: BTreeMap::new(),
            areas: vec![Area { name: "a".into(), rings, ..Default::default() }],
            trees: 0,
        }
    }

    #[test]
    fn the_file_round_trips_and_indexes_by_box() {
        let square = vec![[0.0, 0.0], [200.0, 0.0], [200.0, 200.0], [0.0, 200.0]];
        // a second ring overlapping the first: the union is covered (not even-odd).
        let other = vec![[100.0, 100.0], [300.0, 100.0], [300.0, 300.0], [100.0, 300.0]];
        let trees: Vec<Tree> = (0..50).map(|k| Tree { x: 5.0 + 6.0 * k as f32, z: 3.0 + 4.0 * k as f32, height_m: 10.0 + k as f32 * 0.5, crown_r_m: 3.0, ground_navd88_m: 50.0, area: 0, flags: 0 }).collect();
        let lt = LidarTrees::new(header(vec![square, other]), trees.clone());
        let dir = std::env::temp_dir().join(format!("eo-hills-trees-{}", std::process::id()));
        let path = file_path(&dir);
        lt.write(&path).unwrap();
        let back = LidarTrees::open(&path).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(back.header().trees, 50);
        let mut a: Vec<_> = back.trees().to_vec();
        let mut b = trees.clone();
        let key = |t: &Tree| (t.x.to_bits(), t.z.to_bits());
        a.sort_by_key(key);
        b.sort_by_key(key);
        assert_eq!(a, b);
        // the box query is the brute-force filter
        for (x0, z0, x1, z1) in [(0.0, 0.0, 64.0, 64.0), (30.0, 20.0, 170.0, 130.0), (-10.0, -10.0, 400.0, 400.0), (64.0, 0.0, 64.5, 300.0)] {
            let mut q: Vec<_> = back.in_box(x0, z0, x1, z1).map(key).collect();
            let mut want: Vec<_> = trees.iter().filter(|t| (t.x as f64) >= x0 && (t.x as f64) < x1 && (t.z as f64) >= z0 && (t.z as f64) < z1).map(key).collect();
            q.sort();
            want.sort();
            assert_eq!(q, want, "box {x0} {z0} {x1} {z1}");
        }
        assert!(back.covers(50.0, 50.0) && back.covers(150.0, 150.0) && back.covers(250.0, 250.0));
        assert!(!back.covers(250.0, 50.0) && !back.covers(-5.0, 50.0) && !back.covers(50.0, 250.0));
    }

    #[test]
    fn spread_reads_percentiles() {
        let mut v: Vec<f32> = (1..=101).map(|k| k as f32).collect();
        let s = Spread::of(&mut v);
        assert_eq!((s.p10, s.p50, s.p90, s.max), (11.0, 51.0, 91.0, 101.0));
    }
}
