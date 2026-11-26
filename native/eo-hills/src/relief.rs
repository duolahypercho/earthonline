//! The ground's relief at 2 m (Docs/HILLS-PLAN.md, stage H3): the high-pass
//! of the USGS 3DEP 1 m DEM on the terrain's 8 m grid, citywide, for the
//! renderer to add (times its height scale) to the drawn 8 m surface.
//!
//! # Definition (`eo-render/src/terrain_detail.rs`, kept exactly)
//!
//! The 8 m grid's vertex (cx, cz) sits at plan (`origin_x + cx * cell_m`,
//! `origin_z + cz * cell_m`) (surveyed plan metres, x east, z south; the
//! elevation grid's cell centres). Its low-pass is the DEM averaged over the
//! 8 m cell centred on it: 16 points 2 m apart, each the mean of 4 taps
//! 0.5 m off it (64 taps 1 m apart). A patch is 8 x 8 cells at 4 steps a
//! cell: 33 x 33 points 2 m apart. The relief at a point is the DEM's 2 m
//! mean there (4 taps 0.5 m off it) minus the low-pass drawn on the cell's
//! two triangles ([`cell_tri`], the diagonal from (1, 0) to (0, 1)); none
//! (0) where any of those is missing; clamped to [`MAX_RELIEF_M`] (a bigger
//! step is a wall or a building edge left in the bare-earth model). Every
//! value is a function of the point's global index alone, so a block's edge
//! is identical to its neighbour's.
//!
//! # File
//!
//! `<cache>/hills/relief-v1.bin` ([`file_path`]), little-endian:
//!
//! ```text
//! b"EOHR"  u32 version (1)  u32 header length
//! header   JSON ([`Header`])
//! index    per block: i32 px, i32 pz, u16 max |relief| cm, u16 built share x 10000, u32 clamped
//! data     per block, in index order: 33 x 33 i16 centimetres, row-major (index j * 33 + i,
//!          i along +x, j along +z), clamped to +-4 m
//! ```
//!
//! Block (px, pz) covers grid vertices cx in [8 px, 8 px + 8], cz in [8 pz, 8 pz + 8].
//! Only blocks with more than [`KEEP_M`] of relief outside the building
//! footprints are kept; a missing block has none.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const MAGIC: &[u8; 4] = b"EOHR";
pub const VERSION: u32 = 1;
pub const FILE_NAME: &str = "relief-v1.bin";
pub const FORMAT: &str = "i16-cm";
pub const GENERATOR: &str = "eo-hills cook relief v1";
/// The terrain grid's cell, surveyed metres.
pub const CELL_M: f64 = 8.0;
/// Detail steps per grid cell (2 m).
pub const STEPS: u32 = 4;
/// Grid cells per block side (64 m).
pub const PATCH_CELLS: u32 = 8;
/// Points per block side.
pub const SIDE: usize = (PATCH_CELLS * STEPS + 1) as usize;
/// Points per block.
pub const POINTS: usize = SIDE * SIDE;
/// The largest relief kept, m (terrain_detail `MAX_RELIEF_M`).
pub const MAX_RELIEF_M: f64 = 4.0;
/// A block is kept when its relief outside the footprints exceeds this, m.
pub const KEEP_M: f64 = 0.10;
/// Bytes of one index entry.
pub const INDEX_ENTRY: usize = 16;

/// `<cache>/hills/relief-v1.bin`.
pub fn file_path(cache: &Path) -> PathBuf {
    cache.join("hills").join(FILE_NAME)
}

/// One kept block's summary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockInfo {
    /// The largest |relief| of its points, m (after the clamp).
    pub max_abs_m: f32,
    /// The share of its points inside building footprints.
    pub built_share: f32,
    /// Its points whose relief was clamped (walls and building edges the DEM kept: stage H4's work).
    pub clamped: u32,
}

/// What the cook found.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Stats {
    /// Blocks inside the 8 m grid.
    pub candidate_blocks: u64,
    /// Blocks with any point the DEM covers.
    pub land_blocks: u64,
    pub kept_blocks: u64,
    /// Kept blocks with more than half their points inside footprints.
    pub kept_mostly_built: u64,
    /// Clamped points in the kept blocks.
    pub clamped_points: u64,
    /// The largest |relief| before the clamp, over all land blocks, and outside footprints, m.
    pub max_abs_m: f32,
    pub max_open_abs_m: f32,
    pub seconds: f32,
}

/// One input DEM tile.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct TileInput {
    pub path: String,
    /// "primary" (its height wins) or "gap-fill" (only where no primary has one).
    pub role: String,
    pub bytes: u64,
    pub sha256: String,
    pub url: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Inputs {
    pub dem_source: String,
    pub dem_licence: String,
    pub dem_attribution: String,
    /// SHA-256 of the DEM folder's PROVENANCE.json.
    pub dem_provenance_sha256: String,
    pub tiles: Vec<TileInput>,
    /// The city files read (`public/data/sf/`), name to SHA-256.
    pub city_files: BTreeMap<String, String>,
}

/// The file's JSON header.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Header {
    pub format: String,
    /// Plan metres (surveyed, 1:1) of grid vertex (0, 0).
    pub origin_x: f64,
    pub origin_z: f64,
    pub cell_m: f64,
    pub steps: u32,
    pub patch_cells: u32,
    /// The 8 m grid's vertices (the elevation grid's cells).
    pub grid_width: u32,
    pub grid_height: u32,
    pub definition: String,
    pub datum: String,
    pub keep_m: f64,
    pub clamp_m: f64,
    pub blocks: u32,
    pub stats: Stats,
    pub inputs: Inputs,
    pub generator: String,
}

/// The relief file in memory.
pub struct Relief {
    header: Header,
    index: HashMap<(i64, i64), usize>,
    keys: Vec<(i64, i64)>,
    info: Vec<BlockInfo>,
    data: Vec<i16>,
}

impl Relief {
    /// Read the file: header, index and every block.
    pub fn open(path: &Path) -> Result<Relief> {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        if bytes.len() < 12 || &bytes[0..4] != MAGIC {
            bail!("{}: not a relief file", path.display());
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
        if header.format != FORMAT || header.steps != STEPS || header.patch_cells != PATCH_CELLS {
            bail!("{}: format {} with {} steps and {} cells a block, expected {FORMAT}, {STEPS}, {PATCH_CELLS}", path.display(), header.format, header.steps, header.patch_cells);
        }
        let n = header.blocks as usize;
        let (ix, dx) = (12 + hl, 12 + hl + n * INDEX_ENTRY);
        if bytes.len() != dx + n * POINTS * 2 {
            bail!("{}: {} bytes, the header's {n} blocks need {}", path.display(), bytes.len(), dx + n * POINTS * 2);
        }
        let (mut index, mut keys, mut info) = (HashMap::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n));
        for b in 0..n {
            let e = &bytes[ix + b * INDEX_ENTRY..ix + (b + 1) * INDEX_ENTRY];
            let key = (i32::from_le_bytes(e[0..4].try_into().unwrap()) as i64, i32::from_le_bytes(e[4..8].try_into().unwrap()) as i64);
            if index.insert(key, b).is_some() {
                bail!("{}: block {key:?} twice", path.display());
            }
            keys.push(key);
            info.push(BlockInfo {
                max_abs_m: u16::from_le_bytes(e[8..10].try_into().unwrap()) as f32 / 100.0,
                built_share: u16::from_le_bytes(e[10..12].try_into().unwrap()) as f32 / 10000.0,
                clamped: u32::from_le_bytes(e[12..16].try_into().unwrap()),
            });
        }
        let data = bytes[dx..].chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
        Ok(Relief { header, index, keys, info, data })
    }

    /// Block (px, pz)'s 33 x 33 relief values, centimetres (index j * 33 + i; i along +x, j along +z).
    pub fn block(&self, px: i64, pz: i64) -> Option<&[i16]> {
        self.index.get(&(px, pz)).map(|&b| &self.data[b * POINTS..(b + 1) * POINTS])
    }

    pub fn info(&self, px: i64, pz: i64) -> Option<&BlockInfo> {
        self.index.get(&(px, pz)).map(|&b| &self.info[b])
    }

    /// The kept blocks' keys, in file order.
    pub fn keys(&self) -> impl Iterator<Item = (i64, i64)> + '_ {
        self.keys.iter().copied()
    }

    pub fn header(&self) -> &Header {
        &self.header
    }
}

/// One block to write.
pub struct Block {
    pub key: (i64, i64),
    pub info: BlockInfo,
    pub cm: Vec<i16>,
}

/// Write the file (`header.blocks` is set from `blocks`).
pub fn write(path: &Path, header: &Header, blocks: &[Block]) -> Result<()> {
    let mut header = header.clone();
    header.blocks = blocks.len() as u32;
    let json = serde_json::to_vec(&header)?;
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
        for b in blocks {
            if b.cm.len() != POINTS {
                bail!("block {:?}: {} values, expected {POINTS}", b.key, b.cm.len());
            }
            f.write_all(&(b.key.0 as i32).to_le_bytes())?;
            f.write_all(&(b.key.1 as i32).to_le_bytes())?;
            f.write_all(&((b.info.max_abs_m * 100.0).round().clamp(0.0, 65535.0) as u16).to_le_bytes())?;
            f.write_all(&((b.info.built_share * 10000.0).round().clamp(0.0, 10000.0) as u16).to_le_bytes())?;
            f.write_all(&b.info.clamped.to_le_bytes())?;
        }
        for b in blocks {
            let mut buf = Vec::with_capacity(POINTS * 2);
            for v in &b.cm {
                buf.extend_from_slice(&v.to_le_bytes());
            }
            f.write_all(&buf)?;
        }
        f.flush()?;
    }
    std::fs::rename(&tmp, path).with_context(|| format!("renaming {} to {}", tmp.display(), path.display()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// The definition.
// ---------------------------------------------------------------------------

/// The 8 m grid: vertex (cx, cz) at (`ox + cx * cell`, `oz + cz * cell`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lattice {
    pub ox: f64,
    pub oz: f64,
    pub cell: f64,
    /// Vertices each way.
    pub width: i64,
    pub height: i64,
}

impl Lattice {
    /// The step between points (2 m).
    pub fn step(&self) -> f64 {
        self.cell / STEPS as f64
    }
    /// Plan point of global point (gx, gz) (grid vertex (cx, cz) is point (4 cx, 4 cz)).
    pub fn point(&self, gx: i64, gz: i64) -> (f64, f64) {
        (self.ox + gx as f64 * self.step(), self.oz + gz as f64 * self.step())
    }
    /// The blocks inside the grid (terrain_detail `patch_keys`' bound: the
    /// block's far vertices inside the grid).
    pub fn blocks(&self) -> Vec<(i64, i64)> {
        let pc = PATCH_CELLS as i64;
        let (nx, nz) = ((self.width - 1) / pc, (self.height - 1) / pc);
        (0..nz).flat_map(|pz| (0..nx).map(move |px| (px, pz))).collect()
    }
}

/// A height on one grid cell's two triangles (the terrain tiles' split: the
/// diagonal from (1, 0) to (0, 1)), at (u, v) in [0, 1]^2 (terrain_detail `cell_tri`).
pub fn cell_tri(h00: f32, h10: f32, h01: f32, h11: f32, u: f32, v: f32) -> f32 {
    if u + v <= 1.0 { h00 + u * (h10 - h00) + v * (h01 - h00) } else { h11 + (1.0 - u) * (h01 - h11) + (1.0 - v) * (h10 - h11) }
}

/// The DEM's mean at (x, z) over 4 taps at (+-half, +-half) (terrain_detail `dem_at`).
pub fn dem_mean(dem: &impl Fn(f64, f64) -> Option<f32>, x: f64, z: f64, half: f64) -> Option<f64> {
    let mut acc = 0.0;
    for (dx, dz) in [(-half, -half), (half, -half), (-half, half), (half, half)] {
        acc += dem(x + dx, z + dz)? as f64;
    }
    Some(acc / 4.0)
}

/// The 8 m low-pass at plan (x, z): the DEM averaged over the cell of side
/// `cell` centred there, 4 x 4 points each a 4-tap mean (terrain_detail `low`).
pub fn low_pass(dem: &impl Fn(f64, f64) -> Option<f32>, x: f64, z: f64, cell: f64) -> Option<f32> {
    let mut acc = 0.0;
    for j in 0..4 {
        for i in 0..4 {
            let (dx, dz) = ((i as f64 - 1.5) * cell / 4.0, (j as f64 - 1.5) * cell / 4.0);
            acc += dem_mean(dem, x + dx, z + dz, cell / 16.0)?;
        }
    }
    Some((acc / 16.0) as f32)
}

/// The low-pass at every grid vertex and one more each way ((width + 1) x
/// (height + 1), NaN where the DEM has none), in parallel.
pub fn low_pass_grid(l: &Lattice, dem: &(impl Fn(f64, f64) -> Option<f32> + Sync)) -> Vec<f32> {
    let w = (l.width + 1) as usize;
    let mut out = vec![f32::NAN; w * (l.height + 1) as usize];
    crate::par_rows(&mut out, w, |r0, rows| {
        for (k, v) in rows.iter_mut().enumerate() {
            let (cx, cz) = ((k % w) as i64, (r0 + k / w) as i64);
            *v = low_pass(dem, l.ox + cx as f64 * l.cell, l.oz + cz as f64 * l.cell, l.cell).unwrap_or(f32::NAN);
        }
    });
    out
}

/// The relief (m, before the clamp) at global point (gx, gz), from the
/// low-pass grid `lp` ([`low_pass_grid`]); None where the DEM or a corner's
/// low-pass is missing.
pub fn relief_at(l: &Lattice, lp: &[f32], dem: &impl Fn(f64, f64) -> Option<f32>, gx: i64, gz: i64) -> Option<f64> {
    let s = STEPS as i64;
    let (qx, qz) = (gx.div_euclid(s), gz.div_euclid(s));
    let (u, v) = (gx.rem_euclid(s) as f32 / s as f32, gz.rem_euclid(s) as f32 / s as f32);
    let w = l.width + 1;
    let at = |cx: i64, cz: i64| -> Option<f32> {
        if cx < 0 || cz < 0 || cx > l.width || cz > l.height {
            return None;
        }
        let v = lp[(cz * w + cx) as usize];
        (!v.is_nan()).then_some(v)
    };
    let (a, b, c, d) = (at(qx, qz)?, at(qx + 1, qz)?, at(qx, qz + 1)?, at(qx + 1, qz + 1)?);
    let (x, z) = l.point(gx, gz);
    let dm = dem_mean(dem, x, z, l.step() / 4.0)?;
    Some(dm - cell_tri(a, b, c, d, u, v) as f64)
}

/// Block (px, pz)'s points (row-major, j * 33 + i), before the clamp.
pub fn block_relief(l: &Lattice, lp: &[f32], dem: &impl Fn(f64, f64) -> Option<f32>, key: (i64, i64)) -> Vec<Option<f64>> {
    let n = (PATCH_CELLS * STEPS) as i64;
    let (gx0, gz0) = (key.0 * n, key.1 * n);
    (0..POINTS).map(|k| relief_at(l, lp, dem, gx0 + (k % SIDE) as i64, gz0 + (k / SIDE) as i64)).collect()
}

/// A relief (m) as stored: clamped, centimetres.
pub fn to_cm(r: f64) -> i16 {
    (r.clamp(-MAX_RELIEF_M, MAX_RELIEF_M) * 100.0).round() as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lattice() -> Lattice {
        Lattice { ox: -100.0, oz: 40.0, cell: 8.0, width: 20, height: 18 }
    }

    #[test]
    fn a_plane_has_no_relief() {
        let l = lattice();
        let dem = |x: f64, z: f64| Some((12.0 + 0.031 * x - 0.017 * z) as f32);
        let lp = low_pass_grid(&l, &dem);
        for key in [(0, 0), (1, 1)] {
            for r in block_relief(&l, &lp, &dem, key) {
                assert!(r.unwrap().abs() < 1e-4, "{r:?}");
            }
        }
    }

    #[test]
    fn a_bump_is_its_height_less_its_cell_mean() {
        let l = lattice();
        // A 2 m square bump of 1.6 m round grid vertex (5, 6).
        let (bx, bz) = (l.ox + 5.0 * 8.0, l.oz + 6.0 * 8.0);
        let h = 1.6f32;
        let dem = move |x: f64, z: f64| Some(if (x - bx).abs() < 1.0 && (z - bz).abs() < 1.0 { h } else { 0.0 });
        let lp = low_pass_grid(&l, &dem);
        // its cell's mean: 4 of the 64 taps
        assert!((lp[(6 * 21 + 5) as usize] - h / 16.0).abs() < 1e-6);
        assert_eq!(lp[(6 * 21 + 6) as usize], 0.0);
        let r = relief_at(&l, &lp, &dem, 20, 24).unwrap();
        assert!((r - (h - h / 16.0) as f64).abs() < 1e-6, "{r}");
        // a step (2 m) off: no bump in its 2 m mean, three quarters of the vertex's low-pass under it
        let r = relief_at(&l, &lp, &dem, 21, 24).unwrap();
        assert!((r + 0.75 * (h / 16.0) as f64).abs() < 1e-6, "{r}");
        // a cell away: nothing
        assert_eq!(relief_at(&l, &lp, &dem, 32, 24).unwrap(), 0.0);
        // no DEM: no relief
        let none = |_: f64, _: f64| -> Option<f32> { None };
        assert_eq!(relief_at(&l, &low_pass_grid(&l, &none), &none, 20, 24), None);
    }

    #[test]
    fn blocks_share_their_edges() {
        let l = lattice();
        let dem = |x: f64, z: f64| Some((3.0 * (x * 0.37).sin() * (z * 0.23).cos() + 0.002 * x * z) as f32);
        let lp = low_pass_grid(&l, &dem);
        let (a, b, c) = (block_relief(&l, &lp, &dem, (0, 0)), block_relief(&l, &lp, &dem, (1, 0)), block_relief(&l, &lp, &dem, (0, 1)));
        for j in 0..SIDE {
            assert_eq!(a[j * SIDE + SIDE - 1], b[j * SIDE], "east edge, row {j}");
        }
        for i in 0..SIDE {
            assert_eq!(a[(SIDE - 1) * SIDE + i], c[i], "south edge, column {i}");
        }
        assert!(a.iter().any(|r| r.unwrap().abs() > 0.1), "the test DEM has relief");
        assert_eq!(l.blocks().len(), 2 * 2);
        assert_eq!(to_cm(5.0), 400);
        assert_eq!(to_cm(-0.1234), -12);
    }

    #[test]
    fn file_round_trip() {
        let header = Header {
            format: FORMAT.into(),
            origin_x: -9868.0,
            origin_z: -7508.0,
            cell_m: CELL_M,
            steps: STEPS,
            patch_cells: PATCH_CELLS,
            grid_width: 2431,
            grid_height: 2187,
            definition: "test".into(),
            datum: "test".into(),
            keep_m: KEEP_M,
            clamp_m: MAX_RELIEF_M,
            blocks: 0,
            stats: Stats { kept_blocks: 2, ..Default::default() },
            inputs: Inputs { tiles: vec![TileInput { path: "a.tif".into(), role: "primary".into(), bytes: 3, sha256: "ab".repeat(32), url: "u".into() }], ..Default::default() },
            generator: GENERATOR.into(),
        };
        let blocks = vec![
            Block { key: (3, -1), info: BlockInfo { max_abs_m: 0.42, built_share: 0.25, clamped: 7 }, cm: (0..POINTS).map(|k| (k as i16) - 500).collect() },
            Block { key: (4, 9), info: BlockInfo { max_abs_m: 4.0, built_share: 1.0, clamped: 0 }, cm: vec![-400; POINTS] },
        ];
        let dir = std::env::temp_dir().join(format!("eo-hills-relief-test-{}", std::process::id()));
        let path = file_path(&dir);
        write(&path, &header, &blocks).unwrap();
        let r = Relief::open(&path).unwrap();
        assert_eq!(r.header().blocks, 2);
        assert_eq!((r.header().origin_x, r.header().cell_m, r.header().steps, r.header().patch_cells), (-9868.0, 8.0, 4, 8));
        assert_eq!(r.keys().collect::<Vec<_>>(), vec![(3, -1), (4, 9)]);
        assert_eq!(r.block(3, -1).unwrap(), &blocks[0].cm[..]);
        assert_eq!(r.block(4, 9).unwrap()[1088], -400);
        assert_eq!(r.block(0, 0), None);
        assert_eq!(*r.info(3, -1).unwrap(), BlockInfo { max_abs_m: 0.42, built_share: 0.25, clamped: 7 });
        // a cut file is refused
        let bytes = std::fs::read(&path).unwrap();
        std::fs::write(&path, &bytes[..bytes.len() - 2]).unwrap();
        assert!(Relief::open(&path).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
