//! The planting map (Docs/HILLS-PLAN.md, stage H6a): the vegetation
//! planting's NAIP rules (`eo_landmarks::vegetation::naip_shares`), cooked
//! citywide so the runtime can feed the same rules everywhere, not only
//! inside the Golden Gate survey's 20 NAIP tiles.
//!
//! The cook resamples the citywide 0.3 m NAIP RGB (as delivered: not
//! dehazed, not colour-transferred; the rules were tuned on the survey's
//! raw pixels) to the survey's own pixel (square in degrees,
//! `eo_farshore::survey::naip::GRID.dlon`: 0.600 m east-west, 0.754 m
//! north-south), classifies every pixel with the real rules (each pixel's
//! [`pixel_counts`]: `naip_shares` over the 5 x 5 pixels round it, which
//! counts that pixel alone with its 5 x 5 brightness), and counts the
//! classes per 2 m texel of the macro map's grid. A window's shares are the
//! sums of its texels' counts divided as `naip_shares` divides them
//! ([`shares_of`]).
//!
//! # City calibrations
//!
//! Two departures from the survey-tuned rules, for this citywide map only
//! (the survey's own path is unchanged), recorded in [`Header::calibration`]
//! ([`city_counts`]):
//!
//! * Bare by brightness: the city's pale cured grass is brighter than the
//!   rules' `NAIP_BARE_V`. A pixel the rules call bare only for its 5 x 5
//!   brightness (it would be low cover with that brightness at the rules'
//!   threshold: not grey-blue, not steep) is low cover up to the city's
//!   threshold, measured by the cook on mapped cured grass against mapped
//!   bare ground and trails.
//! * Warm and soil: in the city's imagery cured grass is redder than trails
//!   and bare ground, so the rules' red-over-green tests are counted only on
//!   mapped trails and bare ground (the rules' own tests there), else 0.
//!
//! # File
//!
//! `<cache>/hills/planting-v1.bin` ([`file_path`]): `b"EOHP"`, u32 version
//! (1), u32 header length, header JSON ([`Header`]), then the gzip of the
//! block index (`blocks_x` x `blocks_y` little-endian u32, row-major from the
//! north-west: the stored block's number, or `u32::MAX` for none) and the
//! stored blocks' texels (each block [`BLOCK`] x [`BLOCK`] texels, row-major,
//! each a little-endian u32 of eight 4-bit counts, nibble k = [`COUNTS`]`[k]`).
//! Only blocks holding open land with data are stored (the cook's rule,
//! `Header::calibration.block_rule`); a texel elsewhere has no data.

use anyhow::{Context, Result, bail};
use eo_landmarks::vegetation::{Naip, naip_shares};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const MAGIC: &[u8; 4] = b"EOHP";
pub const VERSION: u32 = 1;
pub const FILE_NAME: &str = "planting-v1.bin";
pub const FORMAT: &str = "u32-nibbles-blocks-gzip";
pub const GENERATOR: &str = "eo-hills cook planting v1.1 (city calibrations, blocks)";
/// Texels per block side (64 m).
pub const BLOCK: usize = 32;
/// The counts per texel, in nibble order: the three classes and unknown
/// (`NaipClass` order), then the flags `naip_shares` counts.
pub const COUNTS: [&str; 8] = ["canopy", "low", "bare", "unknown", "lush", "soil", "warm", "green"];
pub const CANOPY: usize = 0;
pub const LOW: usize = 1;
pub const BARE: usize = 2;
pub const UNKNOWN: usize = 3;
pub const LUSH: usize = 4;
pub const SOIL: usize = 5;
pub const WARM: usize = 6;
pub const GREEN: usize = 7;
/// The largest box side the runtime reads, in survey pixels (eo-app `naip_box`).
pub const MAX_BOX_PX: i64 = 15;

/// `<cache>/hills/planting-v1.bin`.
pub fn file_path(cache: &Path) -> PathBuf {
    cache.join("hills").join(FILE_NAME)
}

/// Eight counts (each at most 15) as one u32.
pub fn pack(c: [u8; 8]) -> u32 {
    c.iter().enumerate().fold(0u32, |v, (k, &n)| v | ((n.min(15) as u32) << (4 * k)))
}

pub fn unpack(v: u32) -> [u8; 8] {
    std::array::from_fn(|k| ((v >> (4 * k)) & 15) as u8)
}

/// One pixel's counts under the real rules: `naip_shares` over the 5 x 5
/// pixels round it (row-major, the pixel in the middle; None outside the
/// image) counts that pixel alone, with its 5 x 5 brightness. None when the
/// pixel itself has no data; unknown (shadow, water) when the rules read nothing.
pub fn pixel_counts(block: &[Option<[u8; 3]>; 25], slope_deg: f64) -> Option<[u8; 8]> {
    block[12]?;
    let mut c = [0u8; 8];
    match naip_shares(block, 5, 5, slope_deg) {
        None => c[UNKNOWN] = 1,
        Some(n) => {
            let class = if n.canopy > 0.5 {
                CANOPY
            } else if n.low > 0.5 {
                LOW
            } else {
                BARE
            };
            c[class] = 1;
            c[LUSH] = (n.lush > 0.5) as u8;
            c[SOIL] = (n.soil > 0.5) as u8;
            c[WARM] = (n.warm > 0.5) as u8;
            c[GREEN] = (n.green > 0.5) as u8;
        }
    }
    Some(c)
}

/// The 5 x 5 mean brightness `naip_shares` reads for the middle pixel (its
/// arithmetic, in its order). None without any pixel.
pub fn v5(block: &[Option<[u8; 3]>; 25]) -> Option<f32> {
    let (mut sum, mut k) = (0.0f32, 0u32);
    for p in block.iter().flatten() {
        sum += (p[0] as f32 + p[1] as f32 + p[2] as f32) / 765.0;
        k += 1;
    }
    (k > 0).then(|| sum / k as f32)
}

/// One pixel's counts with the city calibrations ([module](self)): the
/// rules' ([`pixel_counts`]), then bare by brightness alone is low cover up
/// to `bare_v5`, and warm and soil count only on a mapped trail or bare
/// ground (`trail`).
pub fn city_counts(block: &[Option<[u8; 3]>; 25], slope_deg: f64, bare_v5: f32, trail: bool) -> Option<[u8; 8]> {
    use eo_landmarks::vegetation::{NAIP_BARE_V, NaipClass, naip_class};
    let mut c = pixel_counts(block, slope_deg)?;
    if c[BARE] == 1 && v5(block).is_some_and(|v| v <= bare_v5) && naip_class(block[12]?, NAIP_BARE_V, slope_deg) == NaipClass::Low {
        c[BARE] = 0;
        c[LOW] = 1;
    }
    if !trail {
        c[WARM] = 0;
        c[SOIL] = 0;
    }
    Some(c)
}

/// Shares from summed counts, divided as `naip_shares` divides them (over
/// the pixels it could read: canopy, low, bare). None with no readable pixel,
/// or when fewer than half the `expected` pixels have data (eo-app
/// `naip_shares_at`'s rule).
pub fn shares_of(sum: [u32; 8], expected: f64) -> Option<Naip> {
    let data = (sum[CANOPY] + sum[LOW] + sum[BARE] + sum[UNKNOWN]) as f64;
    if data < 0.5 * expected {
        return None;
    }
    let known = (sum[CANOPY] + sum[LOW] + sum[BARE]) as f32;
    (known > 0.0).then(|| Naip {
        canopy: sum[CANOPY] as f32 / known,
        low: sum[LOW] as f32 / known,
        bare: sum[BARE] as f32 / known,
        lush: sum[LUSH] as f32 / known,
        soil: sum[SOIL] as f32 / known,
        warm: sum[WARM] as f32 / known,
        green: sum[GREEN] as f32 / known,
    })
}

/// The survey pixel the counts were made on.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct PixelGrid {
    /// Side in degrees (square in degrees; the survey's `GRID.dlon`).
    pub deg: f64,
    /// Its ground size on the surveyed plan, east-west and north-south (m).
    pub col_m: f64,
    pub row_m: f64,
    pub west: f64,
    pub north: f64,
    pub cols: u32,
    pub rows: u32,
    /// How the 0.3 m pixels were combined.
    pub resampling: String,
    /// Per-channel gain applied to the 0.3 m set before the rules (1: none).
    pub gain: [f32; 3],
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Stats {
    pub texels_with_data: u64,
    pub pixels: u64,
    /// Pixels per class and flag, in [`COUNTS`] order.
    pub totals: [u64; 8],
    pub max_pixels_per_texel: u32,
    pub seconds: f32,
    /// The comparison with the survey's own pixels (`naip_shares_at`), when it was run.
    #[serde(default)]
    pub validation: BTreeMap<String, f64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Inputs {
    pub naip_manifest_sha256: String,
    pub naip_tiles_used: u32,
    pub naip_source: String,
    pub naip_licence: String,
    pub naip_credit: String,
    /// The city files read (`public/data/sf/`), name to SHA-256.
    pub city_files: BTreeMap<String, String>,
}

/// The city calibrations ([module](self)) and the blocks kept.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Calibration {
    /// The city's bare threshold on the 5 x 5 brightness, and the rules' own.
    pub bare_v5: f64,
    pub survey_bare_v5: f64,
    pub bare_rule: String,
    /// The two brightness populations it was measured on: name to statistics (count, p10, p25, median, p75, p90).
    pub populations: BTreeMap<String, BTreeMap<String, f64>>,
    pub warm_soil_rule: String,
    pub block_rule: String,
}

/// The file's JSON header.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Header {
    pub format: String,
    /// Plan metres (surveyed, 1:1) of texel (0, 0)'s north-west corner; the macro map's grid.
    pub origin_x: f64,
    pub origin_z: f64,
    pub texel_m: f64,
    pub width: u32,
    pub height: u32,
    pub pixel: PixelGrid,
    pub counts: Vec<String>,
    /// The rules' constants (eo_landmarks::vegetation NAIP_*) the counts were made with.
    pub rules: BTreeMap<String, f64>,
    /// How a pixel's slope was read.
    pub slope: String,
    /// Texels per block side, the block grid, and the blocks stored.
    pub block_texels: u32,
    pub blocks_x: u32,
    pub blocks_y: u32,
    pub blocks: u32,
    pub calibration: Calibration,
    pub stats: Stats,
    pub inputs: Inputs,
    pub generator: String,
}

/// The rules' constants as cooked (a file made with others is stale).
pub fn rule_constants() -> BTreeMap<String, f64> {
    use eo_landmarks::vegetation as v;
    [
        ("NAIP_CANOPY_V", v::NAIP_CANOPY_V as f64),
        ("NAIP_SAGE_V", v::NAIP_SAGE_V as f64),
        ("NAIP_BARE_V", v::NAIP_BARE_V as f64),
        ("NAIP_DARK_V", v::NAIP_DARK_V as f64),
        ("NAIP_SHADOW_V", v::NAIP_SHADOW_V as f64),
        ("NAIP_BARE_SLOPE_DEG", v::NAIP_BARE_SLOPE_DEG),
        ("NAIP_WARM_RG", v::NAIP_WARM_RG as f64),
        ("NAIP_SOIL_RG", v::NAIP_SOIL_RG as f64),
        ("NAIP_GREEN_STRICT", v::NAIP_GREEN_STRICT as f64),
    ]
    .into_iter()
    .map(|(k, x)| (k.to_string(), x))
    .collect()
}

/// The planting map in memory: the stored blocks' texels (4 bytes each) and the block index.
pub struct Planting {
    header: Header,
    index: Vec<u32>,
    texels: Vec<u32>,
}

impl Planting {
    /// From a dense texel array (`width` x `height`, row-major), storing the
    /// blocks `keep` says (by block number, row-major); the header's block
    /// fields are set.
    pub fn from_dense(mut header: Header, dense: &[u32], keep: &dyn Fn(usize, usize) -> bool) -> Result<Planting> {
        let (w, h) = (header.width as usize, header.height as usize);
        if dense.len() != w * h {
            bail!("planting map: {} texels, the header says {w} x {h}", dense.len());
        }
        let (bx, by) = (w.div_ceil(BLOCK), h.div_ceil(BLOCK));
        let mut index = vec![u32::MAX; bx * by];
        let mut texels = vec![];
        for b in 0..bx * by {
            let (px, pz) = (b % bx, b / bx);
            if !keep(px, pz) {
                continue;
            }
            index[b] = (texels.len() / (BLOCK * BLOCK)) as u32;
            for j in 0..BLOCK {
                for i in 0..BLOCK {
                    let (x, z) = (px * BLOCK + i, pz * BLOCK + j);
                    texels.push(if x < w && z < h { dense[z * w + x] } else { 0 });
                }
            }
        }
        header.block_texels = BLOCK as u32;
        header.blocks_x = bx as u32;
        header.blocks_y = by as u32;
        header.blocks = (texels.len() / (BLOCK * BLOCK)) as u32;
        Ok(Planting { header, index, texels })
    }

    /// From a dense texel array, storing every block with any count.
    pub fn new(header: Header, dense: Vec<u32>) -> Result<Planting> {
        let w = header.width as usize;
        let h = header.height as usize;
        let any = |px: usize, pz: usize| (pz * BLOCK..((pz + 1) * BLOCK).min(h)).any(|z| dense[z * w + px * BLOCK..z * w + ((px + 1) * BLOCK).min(w)].iter().any(|&v| v != 0));
        Planting::from_dense(header, &dense, &any)
    }

    /// Read the file (gunzips the stored blocks into memory: [`Planting::held_bytes`]).
    pub fn open(path: &Path) -> Result<Planting> {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        if bytes.len() < 12 || &bytes[0..4] != MAGIC {
            bail!("{}: not a planting map", path.display());
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
        if header.format != FORMAT || header.counts != COUNTS.map(String::from) || header.block_texels as usize != BLOCK {
            bail!("{}: format {} with counts {:?} in blocks of {}", path.display(), header.format, header.counts, header.block_texels);
        }
        if header.rules != rule_constants() {
            bail!("{}: cooked with rule constants {:?}, the rules now hold {:?}: cook it again", path.display(), header.rules, rule_constants());
        }
        let (nb, ns) = (header.blocks_x as usize * header.blocks_y as usize, header.blocks as usize);
        if header.blocks_x as usize != (header.width as usize).div_ceil(BLOCK) || header.blocks_y as usize != (header.height as usize).div_ceil(BLOCK) {
            bail!("{}: a {} x {} block grid for {} x {} texels", path.display(), header.blocks_x, header.blocks_y, header.width, header.height);
        }
        let mut raw = Vec::with_capacity((nb + ns * BLOCK * BLOCK) * 4);
        flate2::read::GzDecoder::new(&bytes[12 + hl..]).read_to_end(&mut raw).with_context(|| format!("{}: blocks", path.display()))?;
        if raw.len() != (nb + ns * BLOCK * BLOCK) * 4 {
            bail!("{}: {} bytes of blocks, expected {}", path.display(), raw.len(), (nb + ns * BLOCK * BLOCK) * 4);
        }
        let mut words = raw.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]));
        let index: Vec<u32> = words.by_ref().take(nb).collect();
        if index.iter().any(|&b| b != u32::MAX && b as usize >= ns) {
            bail!("{}: a block index past the {ns} blocks", path.display());
        }
        let texels = words.collect();
        Ok(Planting { header, index, texels })
    }

    /// Write the map (gzip).
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
            let mut gz = flate2::write::GzEncoder::new(f, flate2::Compression::default());
            for chunk in self.index.chunks(1 << 16).chain(self.texels.chunks(1 << 16)) {
                let buf: Vec<u8> = chunk.iter().flat_map(|v| v.to_le_bytes()).collect();
                gz.write_all(&buf)?;
            }
            gz.finish()?.flush()?;
        }
        std::fs::rename(&tmp, path).with_context(|| format!("renaming {} to {}", tmp.display(), path.display()))?;
        Ok(())
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    /// Replace the statistics (the cook's validation, after the map is made).
    pub fn set_stats(&mut self, stats: Stats) {
        self.header.stats = stats;
    }

    /// Bytes held in memory (the texels and the block index).
    pub fn held_bytes(&self) -> usize {
        (self.texels.len() + self.index.len()) * 4
    }

    /// Texel (i, j)'s counts (None off the grid or in a block not stored).
    pub fn counts(&self, i: i64, j: i64) -> Option<[u8; 8]> {
        let h = &self.header;
        if i < 0 || j < 0 || i >= h.width as i64 || j >= h.height as i64 {
            return None;
        }
        let (i, j) = (i as usize, j as usize);
        let b = self.index[(j / BLOCK) * h.blocks_x as usize + i / BLOCK];
        (b != u32::MAX).then(|| unpack(self.texels[b as usize * BLOCK * BLOCK + (j % BLOCK) * BLOCK + i % BLOCK]))
    }

    /// The window (m, east-west and north-south) a box of side `side_m`
    /// (real metres) reads: the survey's box (eo-app `naip_box` at scale 1:
    /// odd pixel counts, 1 to 15, about the point), at least one texel.
    pub fn window_m(&self, side_m: f64) -> (f64, f64) {
        let odd = |v: f64| {
            let k = (v.round() as i64).clamp(1, MAX_BOX_PX);
            if k % 2 == 0 { k + 1 } else { k }
        };
        let p = &self.header.pixel;
        let t = self.header.texel_m;
        ((odd(side_m / p.col_m) as f64 * p.col_m).max(t), (odd(side_m / p.row_m) as f64 * p.row_m).max(t))
    }

    /// The NAIP shares in a box of side `side_m` (surveyed plan metres) round
    /// plan point (x, z), as `eo_landmarks::vegetation::naip_shares` gives
    /// them on the survey: the counts of the texels whose centres fall in the
    /// box ([`Planting::window_m`]), divided as it divides. None where under
    /// half the box has data, or nothing in it is readable.
    pub fn shares(&self, plan_x: f64, plan_z: f64, side_m: f64) -> Option<Naip> {
        let h = &self.header;
        let (wx, wz) = self.window_m(side_m);
        let t = h.texel_m;
        // texel centres origin + (i + 0.5) t in [x - wx / 2, x + wx / 2)
        let i0 = ((plan_x - wx / 2.0 - h.origin_x) / t - 0.5).ceil() as i64;
        let i1 = ((plan_x + wx / 2.0 - h.origin_x) / t - 0.5).ceil() as i64;
        let j0 = ((plan_z - wz / 2.0 - h.origin_z) / t - 0.5).ceil() as i64;
        let j1 = ((plan_z + wz / 2.0 - h.origin_z) / t - 0.5).ceil() as i64;
        let mut sum = [0u32; 8];
        let mut n = 0u32;
        for j in j0..j1.max(j0 + 1) {
            for i in i0..i1.max(i0 + 1) {
                n += 1;
                if let Some(c) = self.counts(i, j) {
                    for k in 0..8 {
                        sum[k] += c[k] as u32;
                    }
                }
            }
        }
        let per_texel = t * t / (h.pixel.col_m * h.pixel.row_m);
        shares_of(sum, n as f64 * per_texel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nibbles_round_trip() {
        let c = [12, 0, 3, 15, 1, 9, 2, 7];
        assert_eq!(unpack(pack(c)), c);
        assert_eq!(unpack(pack([16, 0, 0, 0, 0, 0, 0, 0]))[0], 15, "saturates");
    }

    /// The per-pixel counts summed over a box give `naip_shares` over that box.
    #[test]
    fn pixel_counts_sum_to_the_rules_shares() {
        let (w, h) = (11usize, 9usize);
        // a mix: dark green crowns, bright green sward, tan cured grass, red soil, pale paving, shadow, a hole
        let palette: [Option<[u8; 3]>; 8] = [Some([40, 60, 35]), Some([120, 150, 90]), Some([150, 140, 110]), Some([170, 130, 100]), Some([200, 200, 205]), Some([30, 30, 35]), None, Some([95, 105, 80])];
        let px: Vec<Option<[u8; 3]>> = (0..w * h).map(|k| palette[(k * 7 + k / w * 3) % palette.len()]).collect();
        for slope in [10.0, 40.0] {
            let want = naip_shares(&px, w, h, slope).unwrap();
            let mut sum = [0u32; 8];
            for j in 2..h - 2 {
                for i in 2..w - 2 {
                    let block: [Option<[u8; 3]>; 25] = std::array::from_fn(|k| px[(j + k / 5 - 2) * w + i + k % 5 - 2]);
                    if let Some(c) = pixel_counts(&block, slope) {
                        for k in 0..8 {
                            sum[k] += c[k] as u32;
                        }
                    }
                }
            }
            let got = shares_of(sum, 0.0).unwrap();
            assert_eq!(got, want, "slope {slope}");
        }
    }

    fn header(w: u32, h: u32) -> Header {
        Header {
            format: FORMAT.into(),
            origin_x: -100.0,
            origin_z: 50.0,
            texel_m: 2.0,
            width: w,
            height: h,
            pixel: PixelGrid { deg: 6.8e-6, col_m: 0.6, row_m: 0.75, ..Default::default() },
            counts: COUNTS.map(String::from).to_vec(),
            rules: rule_constants(),
            slope: "test".into(),
            block_texels: 0,
            blocks_x: 0,
            blocks_y: 0,
            blocks: 0,
            calibration: Calibration::default(),
            stats: Stats::default(),
            inputs: Inputs::default(),
            generator: GENERATOR.into(),
        }
    }

    #[test]
    fn city_calibrations() {
        let flat = |p: [u8; 3]| -> [Option<[u8; 3]>; 25] { [Some(p); 25] };
        // pale cured grass (v5 0.654, not green, not grey-blue): bare by the rules, low up to the city's threshold
        let pale = flat([180, 170, 150]);
        assert_eq!(pixel_counts(&pale, 10.0).unwrap()[BARE], 1);
        assert_eq!(city_counts(&pale, 10.0, 0.70, false).unwrap()[LOW], 1);
        assert_eq!(city_counts(&pale, 10.0, 0.60, false).unwrap()[BARE], 1, "brighter than the city's threshold");
        assert_eq!(city_counts(&pale, 40.0, 0.70, false).unwrap()[BARE], 1, "steep: bare whatever the brightness");
        assert_eq!(city_counts(&flat([150, 150, 165]), 10.0, 0.70, false).unwrap()[BARE], 1, "grey-blue: bare whatever the brightness");
        assert!((v5(&pale).unwrap() - 500.0 / 765.0).abs() < 1e-6);
        // a warm tan pixel: warm on a trail only
        let tan = flat([150, 135, 120]);
        assert_eq!(pixel_counts(&tan, 10.0).unwrap()[WARM], 1);
        assert_eq!(city_counts(&tan, 10.0, 0.70, false).unwrap()[WARM], 0);
        assert_eq!(city_counts(&tan, 10.0, 0.70, true).unwrap(), pixel_counts(&tan, 10.0).unwrap());
    }

    #[test]
    fn windows_and_shares() {
        let (w, h) = (10u32, 8u32);
        let mut texels = vec![0u32; (w * h) as usize];
        // texel (3, 2): 6 canopy, 2 low (1 lush), 1 unknown; texel (4, 2): 8 low (all warm), 1 bare
        texels[(2 * w + 3) as usize] = pack([6, 2, 0, 1, 1, 0, 0, 3]);
        texels[(2 * w + 4) as usize] = pack([0, 8, 1, 0, 0, 0, 8, 0]);
        let p = Planting::new(header(w, h), texels).unwrap();
        // a 1 m box reads one texel (the survey's 3 x 1 pixels, at least a texel)
        assert_eq!(p.window_m(1.0), (2.0, 2.0));
        assert_eq!(p.window_m(8.0), (13.0 * 0.6, 11.0 * 0.75));
        assert_eq!(p.window_m(100.0), (15.0 * 0.6, 15.0 * 0.75));
        let (cx, cz) = (-100.0 + 3.5 * 2.0, 50.0 + 2.5 * 2.0);
        let s = p.shares(cx, cz, 1.0).unwrap();
        assert_eq!((s.canopy, s.low, s.lush, s.green), (0.75, 0.25, 0.125, 0.375));
        // a 3 m box round the shared edge reads both texels
        let s = p.shares(cx + 1.0, cz, 3.0).unwrap();
        assert!((s.canopy - 6.0 / 17.0).abs() < 1e-6 && (s.warm - 8.0 / 17.0).abs() < 1e-6 && (s.bare - 1.0 / 17.0).abs() < 1e-6, "{s:?}");
        // no data: None; off the grid: None
        assert_eq!(p.shares(-100.0 + 8.5 * 2.0, 50.0 + 6.5 * 2.0, 1.0), None);
        assert_eq!(p.shares(-500.0, 0.0, 8.0), None);
    }

    #[test]
    fn file_round_trip() {
        let (w, h) = (300u32, 200u32);
        let texels: Vec<u32> = (0..w * h).map(|k| pack([(k % 13) as u8, (k % 5) as u8, 1, 0, 0, 2, 3, (k % 16) as u8])).collect();
        let mut texels = texels;
        // a block of nothing (texels 32..64 x 32..64) is not stored
        for j in 32..64 {
            for i in 32..64 {
                texels[j * w as usize + i] = 0;
            }
        }
        let p = Planting::new(header(w, h), texels.clone()).unwrap();
        assert_eq!((p.header().blocks_x, p.header().blocks_y, p.header().blocks), (10, 7, 69));
        assert_eq!(p.held_bytes(), (69 * 1024 + 70) * 4);
        let dir = std::env::temp_dir().join(format!("eo-hills-planting-test-{}", std::process::id()));
        let path = file_path(&dir);
        p.write(&path).unwrap();
        let q = Planting::open(&path).unwrap();
        assert_eq!(q.header(), p.header());
        assert_eq!(q.counts(7, 9), Some(unpack(texels[9 * w as usize + 7])));
        assert_eq!(q.counts(299, 199), Some(unpack(texels[199 * w as usize + 299])));
        assert_eq!(q.counts(40, 40), None, "a block not stored");
        assert_eq!(q.counts(300, 0), None);
        // only kept blocks
        let some = Planting::from_dense(header(w, h), &texels, &|px, pz| px == 2 && pz == 1).unwrap();
        assert_eq!(some.header().blocks, 1);
        assert_eq!(some.counts(70, 40), Some(unpack(texels[40 * w as usize + 70])));
        assert_eq!(some.counts(7, 9), None);
        std::fs::remove_dir_all(&dir).ok();
    }
}
