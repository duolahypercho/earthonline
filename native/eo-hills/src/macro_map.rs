//! The aerial macro colour map (Docs/HILLS-PLAN.md, stage H2): the open
//! ground's base colour, baked from USDA NAIP 0.3 m imagery with the haze,
//! the photo's exposure and its cast shadows taken out, for the terrain
//! shader to sample.
//!
//! # File
//!
//! `<cache>/hills/aerial-macro-v1.bin` ([`file_path`]), little-endian:
//!
//! ```text
//! b"EOHM"  u32 version (1)  u32 header length
//! header   JSON (`Header`)
//! data     the BC7 blocks of every level, level 0 first; a level's
//!          `offset` counts from the start of the data (`read_header`'s
//!          second value is that start in the file)
//! ```
//!
//! Format `bc7-srgb`: the RGB is sRGB-encoded (sample it through an sRGB
//! BC7 format to get linear albedo); alpha is 255 on open ground and 0 where
//! the texel centre is inside a building footprint or the imagery has no
//! data there. Level `l` is `max(1, width >> l)` x `max(1, height >> l)`
//! texels (the GPU's own mip sizes), its blocks cover that rounded up to 4.
//!
//! The grid is on the surveyed 1:1 plan (x east, z south, metres): texel
//! (0, 0) covers `origin_x .. origin_x + texel_m`, `origin_z .. origin_z +
//! texel_m`. The drawn plan is that times `eo_data::groundscale::scale()`.
//!
//! # Steps (the cook, `examples/cook.rs`)
//!
//! 1. Each texel is the box average, in linear light, of the 0.3 m pixels
//!    whose centres fall in it, with the survey's haze taken out per channel
//!    (`naip::dehaze`).
//! 2. The photo's colours are carried to the eye's. The photo's true colour
//!    does not match the eye's on these surfaces (dry grass reads
//!    pinkish-brown, a lawn as bright as dry grass), which one gain cannot fix.
//!    `colour_mode` "classes" (the default): a classified colour transfer
//!    (`crate::transfer`), each texel a soft mix of lawn, dry grass, soil,
//!    paving, scrub and canopy read from its NDVI, lightness and chroma,
//!    redrawn in the renderer's own class colours with the photo's pattern.
//!    "matrix": one 3 x 3 colour matrix fitted by least squares from four
//!    classes' medians to their targets ([`fit_colour_matrix`]); kept for
//!    comparison (a linear map can tell lawn from dry grass only by blowing
//!    up red against green).
//! 3. Cast shadows are lifted toward their neighbourhood ([`shadow_lift`]).
//! 4. The albedo is held to [`ALBEDO_MIN`, `ALBEDO_MAX`] per channel.
//! 5. The mips are box-filtered in linear light over open texels only
//!    ([`next_level`]); then every closed texel takes the colour of the
//!    coarser level under it ([`fill_holes`]), so a filtered sample at a
//!    footprint's edge does not pull in the roof.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Texel side on the surveyed plan, metres.
pub const TEXEL_M: f64 = 2.0;
pub const MAGIC: &[u8; 4] = b"EOHM";
pub const VERSION: u32 = 1;
pub const FILE_NAME: &str = "aerial-macro-v1.bin";
pub const FORMAT: &str = "bc7-srgb";
pub const GENERATOR: &str = "eo-hills cook macro v1.2";
/// A texel darker than this share of its neighbourhood's median luminance is
/// in a cast shadow and lifted to it.
pub const SHADOW_RATIO: f32 = 0.55;
/// The neighbourhood of the shadow lift, metres (a square this wide).
pub const SHADOW_WINDOW_M: f64 = 24.0;
/// The ridge of the colour matrix fit, as a share of the mean diagonal of
/// the classes' normal matrix: how hard the fit is held toward a per-channel
/// scale.
pub const COLOUR_RIDGE: f32 = 0.02;
/// The albedo's range, per channel (linear).
pub const ALBEDO_MIN: f32 = 0.01;
pub const ALBEDO_MAX: f32 = 0.6;

/// `<cache>/hills/aerial-macro-v1.bin`.
pub fn file_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join("hills").join(FILE_NAME)
}

/// One mip level: its size in texels, and its blocks' place in the data.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Level {
    pub width: u32,
    pub height: u32,
    /// Bytes from the start of the data.
    pub offset: u64,
    pub length: u64,
}

/// What the cook found.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Stats {
    /// Level 0 texels in all.
    pub texels: u64,
    /// Open texels the shadow lift raised.
    pub lifted_texels: u64,
    /// Texels whose centre is inside a building footprint (alpha 0).
    pub built_texels: u64,
    /// Texels outside every footprint with no imagery (alpha 0).
    pub no_data_texels: u64,
    /// The lawn texels the calibration took its median from.
    pub calibration_texels: u64,
    /// Median luminance (linear, Rec. 709) by class of texel; `raw_` before
    /// the gain, `final_` after the lift and the clamp, the rest after the gain.
    pub medians: BTreeMap<String, f32>,
}

/// Where the map came from.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Inputs {
    /// SHA-256 of the NAIP folder's `manifest.json` (which holds each tile's).
    pub naip_manifest_sha256: String,
    /// Tiles read (each checked against its manifest digest).
    pub naip_tiles_used: u32,
    /// Tiles the manifest lists that were not on disk.
    pub naip_tiles_missing: u32,
    /// Colour-infrared tiles read (the dry grass class's NDVI).
    #[serde(default)]
    pub naip_cir_tiles_used: u32,
    pub naip_source: String,
    pub naip_licence: String,
    pub naip_credit: String,
    pub naip_downloaded: String,
    /// The city files read (`public/data/sf/`), name to SHA-256.
    pub city_files: BTreeMap<String, String>,
    /// Where the class targets come from.
    pub lawn_target_source: String,
}

/// A 3 x 3 colour matrix on linear RGB: `out[r] = sum_c m[r][c] * in[c]`.
pub type Mat3 = [[f32; 3]; 3];
pub const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

fn identity() -> Mat3 {
    IDENTITY
}

/// `m` applied to linear RGB `c`.
pub fn mat_apply(m: &Mat3, c: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|r| m[r][0] * c[0] + m[r][1] * c[1] + m[r][2] * c[2])
}

/// One class of ground the colour matrix is fitted on (all colours linear RGB).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct ClassFit {
    pub name: String,
    /// How it selects its texels.
    pub rule: String,
    pub weight: f32,
    pub texels: u64,
    /// The per-channel median of its texels in the photo, haze taken out.
    pub naip_median: [f32; 3],
    pub target: [f32; 3],
    /// `M` times `naip_median`.
    pub fitted: [f32; 3],
    /// The per-channel median of its texels after `M`, and after the shadow lift and the clamp.
    pub after_median: [f32; 3],
    pub final_median: [f32; 3],
}

/// One class of the colour transfer (colours linear).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct TransferClass {
    pub name: String,
    pub rule: String,
    /// The class colour (soil: on gentle ground; `soil_eroded` in the params' source).
    pub palette: [f32; 3],
    /// Open texels it wins (largest weight).
    pub texels: u64,
    /// Over those texels, in the photo (haze taken out): the per-channel
    /// median, the median luminance (the pattern's reference) and the median
    /// chromaticity (colour over luminance).
    pub photo_median: [f32; 3],
    pub photo_median_lum: f32,
    pub photo_chroma: [f32; 3],
    /// Over the same texels, after the transfer, and after the lift and the clamp.
    pub after_median: [f32; 3],
    pub final_median: [f32; 3],
}

/// The classified colour transfer's record.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ColourTransfer {
    pub classes: Vec<TransferClass>,
    pub params: crate::transfer::Params,
    pub soil_eroded: [f32; 3],
    /// Paving's gain: the roads' median luminance in the photo to the target.
    pub paving_gain: f32,
    pub paving_road_lum: f32,
    /// How NDVI is made, and the colour-infrared haze (NIR, red; 8-bit values).
    pub ndvi: String,
    pub cir_haze: [f32; 2],
    pub palette_source: String,
}

/// The file's JSON header.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Header {
    pub format: String,
    /// Surveyed plan metres of the texture's north-west corner.
    pub origin_x: f64,
    pub origin_z: f64,
    pub texel_m: f64,
    pub width: u32,
    pub height: u32,
    pub levels: Vec<Level>,
    /// A global gain (the first cook's calibration); 1.0 since the colour
    /// matrix carries the calibration.
    #[serde(rename = "K")]
    pub k: f32,
    /// The lawn luminance the lawn class aims at.
    pub lawn_target: f32,
    /// "classes" (`colour_transfer`) or "matrix" (`colour_matrix`).
    #[serde(default)]
    pub colour_mode: String,
    #[serde(default)]
    pub colour_transfer: Option<ColourTransfer>,
    /// The colour matrix (step 2, mode "matrix"; identity otherwise), applied after the haze removal.
    #[serde(default = "identity")]
    pub colour_matrix: Mat3,
    #[serde(default)]
    pub colour_ridge: f32,
    /// Sum over the classes of weight x |M x median - target|^2.
    #[serde(default)]
    pub colour_residual: f32,
    #[serde(default)]
    pub colour_classes: Vec<ClassFit>,
    /// The survey's haze per channel (linear), taken out before the gain.
    pub haze: [f32; 3],
    pub shadow_ratio: f32,
    pub shadow_window_m: f64,
    pub albedo_range: [f32; 2],
    pub stats: Stats,
    pub inputs: Inputs,
    pub generator: String,
}

/// The size of each level: `max(1, w >> l)` x `max(1, h >> l)` down to 1 x 1.
pub fn level_dims(w: u32, h: u32) -> Vec<(u32, u32)> {
    let mut v = vec![(w, h)];
    let mut l = 1;
    while v.last().is_some_and(|&(a, b)| a > 1 || b > 1) {
        v.push(((w >> l).max(1), (h >> l).max(1)));
        l += 1;
    }
    v
}

/// Bytes of BC7 blocks covering a level (16 per 4 x 4 block).
pub fn bc7_len(w: u32, h: u32) -> u64 {
    w.div_ceil(4) as u64 * h.div_ceil(4) as u64 * 16
}

/// The levels' records for a `w` x `h` map, packed one after another.
pub fn level_records(w: u32, h: u32) -> Vec<Level> {
    let mut offset = 0;
    level_dims(w, h)
        .into_iter()
        .map(|(width, height)| {
            let length = bc7_len(width, height);
            let l = Level { width, height, offset, length };
            offset += length;
            l
        })
        .collect()
}

fn check_levels(h: &Header) -> Result<u64> {
    if h.format != FORMAT {
        bail!("aerial macro map: format {:?}, expected {FORMAT:?}", h.format);
    }
    if h.levels != level_records(h.width, h.height) {
        bail!("aerial macro map: the level records do not match a {} x {} BC7 chain", h.width, h.height);
    }
    Ok(h.levels.iter().map(|l| l.length).sum())
}

/// Write the map: magic, version, header, then each level's blocks.
/// `blocks[l]` must be level `l`'s blocks as `header.levels` records them.
pub fn write(path: &Path, header: &Header, blocks: &[Vec<u8>]) -> Result<()> {
    check_levels(header)?;
    if blocks.len() != header.levels.len() || blocks.iter().zip(&header.levels).any(|(b, l)| b.len() as u64 != l.length) {
        bail!("aerial macro map: the blocks do not match the level records");
    }
    let json = serde_json::to_vec(header)?;
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
            f.write_all(b)?;
        }
        f.flush()?;
    }
    std::fs::rename(&tmp, path).with_context(|| format!("renaming {} to {}", tmp.display(), path.display()))?;
    Ok(())
}

/// The map's header, and the byte offset in the file where its data starts.
/// Checks the magic, the version, the level records and the file's length.
pub fn read_header(path: &Path) -> Result<(Header, u64)> {
    let mut f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut head = [0u8; 12];
    f.read_exact(&mut head).with_context(|| format!("{}: too short", path.display()))?;
    if &head[0..4] != MAGIC {
        bail!("{}: not an aerial macro map (magic {:?})", path.display(), &head[0..4]);
    }
    let version = u32::from_le_bytes(head[4..8].try_into().unwrap());
    if version != VERSION {
        bail!("{}: version {version}, expected {VERSION}", path.display());
    }
    let len = u32::from_le_bytes(head[8..12].try_into().unwrap()) as usize;
    if len > 16 << 20 {
        bail!("{}: header of {len} bytes", path.display());
    }
    let mut json = vec![0u8; len];
    f.read_exact(&mut json).with_context(|| format!("{}: header cut short", path.display()))?;
    let header: Header = serde_json::from_slice(&json).with_context(|| format!("{}: header", path.display()))?;
    let data = check_levels(&header).with_context(|| path.display().to_string())?;
    let start = 12 + len as u64;
    let size = f.metadata()?.len();
    if size != start + data {
        bail!("{}: {size} bytes, the header records {}", path.display(), start + data);
    }
    Ok((header, start))
}

// ---------------------------------------------------------------------------
// The image steps.
// ---------------------------------------------------------------------------

/// Rec. 709 luminance of linear RGB.
pub fn luminance(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// The median of `v` (reordered; the upper middle of an even count), None if empty.
pub fn median(v: &mut [f32]) -> Option<f32> {
    if v.is_empty() {
        return None;
    }
    let m = v.len() / 2;
    Some(*v.select_nth_unstable_by(m, f32::total_cmp).1)
}

/// The colour matrix that best carries each sample's photo colour `n` to its
/// target `t`: it minimises sum w |M n - t|^2 + lambda |M - D|^2, `D` the
/// per-channel scale fitted alone (each channel's own least squares) and
/// lambda `ridge` times the mean diagonal of sum w n n^T, so a fit on few
/// or nearly collinear colours cannot blow up. Samples are (n, t, w). None
/// if the system is singular (no samples).
pub fn fit_colour_matrix(samples: &[([f32; 3], [f32; 3], f32)], ridge: f32) -> Option<Mat3> {
    let mut a = [[0f64; 3]; 3];
    let mut d = [0f64; 3];
    for k in 0..3 {
        let (mut num, mut den) = (0.0, 0.0);
        for (n, t, w) in samples {
            num += *w as f64 * n[k] as f64 * t[k] as f64;
            den += *w as f64 * (n[k] as f64).powi(2);
        }
        d[k] = if den > 0.0 { num / den } else { 1.0 };
        for j in 0..3 {
            a[k][j] = samples.iter().map(|(n, _, w)| *w as f64 * n[k] as f64 * n[j] as f64).sum();
        }
    }
    let lambda = ridge as f64 * (a[0][0] + a[1][1] + a[2][2]) / 3.0;
    let mut m = [[0f32; 3]; 3];
    for r in 0..3 {
        let mut lhs = a;
        let mut rhs = [0f64; 3];
        for j in 0..3 {
            lhs[j][j] += lambda;
            rhs[j] = samples.iter().map(|(n, t, w)| *w as f64 * n[j] as f64 * t[r] as f64).sum::<f64>() + if j == r { lambda * d[r] } else { 0.0 };
        }
        let row = solve3(lhs, rhs)?;
        m[r] = row.map(|v| v as f32);
    }
    Some(m)
}

/// Sum over the samples of w |M n - t|^2.
pub fn fit_residual(m: &Mat3, samples: &[([f32; 3], [f32; 3], f32)]) -> f32 {
    samples.iter().map(|(n, t, w)| w * mat_apply(m, *n).iter().zip(t).map(|(a, b)| (a - b).powi(2)).sum::<f32>()).sum()
}

/// `a x = b` by Gaussian elimination with partial pivoting.
fn solve3(mut a: [[f64; 3]; 3], mut b: [f64; 3]) -> Option<[f64; 3]> {
    for col in 0..3 {
        let p = (col..3).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[p][col].abs() < 1e-14 {
            return None;
        }
        a.swap(col, p);
        b.swap(col, p);
        for row in col + 1..3 {
            let f = a[row][col] / a[col][col];
            for k in col..3 {
                a[row][k] -= f * a[col][k];
            }
            b[row] -= f * b[col];
        }
    }
    let mut x = [0f64; 3];
    for row in (0..3).rev() {
        let s: f64 = (row + 1..3).map(|k| a[row][k] * x[k]).sum();
        x[row] = (b[row] - s) / a[row][row];
    }
    Some(x)
}

/// The per-channel median of `px` at `idx` (each channel's own median).
pub fn channel_medians(px: &[[f32; 4]], idx: &[usize]) -> [f32; 3] {
    let mut v: Vec<f32> = Vec::with_capacity(idx.len());
    [0, 1, 2].map(|c| {
        v.clear();
        v.extend(idx.iter().map(|&i| px[i][c]));
        median(&mut v).unwrap_or(f32::NAN)
    })
}

/// A linear RGBA image, row-major from the north-west: alpha 1 on open
/// ground, 0 where closed (built, or no imagery).
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub w: usize,
    pub h: usize,
    pub px: Vec<[f32; 4]>,
}

impl Image {
    pub fn new(w: usize, h: usize) -> Image {
        Image { w, h, px: vec![[0.0; 4]; w * h] }
    }
}

/// Lift cast shadows: where an open texel's luminance is under `ratio` times
/// the median luminance of the open texels round it (a square of `window`
/// texels), scale its RGB up to that (its chroma kept). Returns the number
/// of texels lifted.
///
/// The median is taken on a lattice every `window / 2` texels, each over
/// the `window` x `window` square centred on its node (needing an eighth of
/// it open), and blended bilinearly between nodes, so the reference has no
/// steps.
pub fn shadow_lift(img: &mut Image, window: usize, ratio: f32) -> u64 {
    let (w, h) = (img.w, img.h);
    if w == 0 || h == 0 {
        return 0;
    }
    let window = window.max(2);
    let s = window / 2;
    let need = (window * window / 8).max(1);
    let lum: Vec<f32> = img.px.iter().map(|p| if p[3] > 0.0 { luminance([p[0], p[1], p[2]]) } else { f32::NAN }).collect();
    // Node (a, b) sits at texel coordinates (a*s - 0.5, b*s - 0.5) and holds
    // the median over texels a*s - s .. a*s + s (each axis).
    let (nx, nz) = (w / s + 2, h / s + 2);
    let mut nodes = vec![f32::NAN; nx * nz];
    crate::par_rows(&mut nodes, nx, |b0, rows| {
        let mut v: Vec<f32> = Vec::with_capacity(window * window);
        for (k, row) in rows.chunks_mut(nx).enumerate() {
            let b = b0 + k;
            let (y0, y1) = ((b * s).saturating_sub(s), (b * s + s).min(h));
            for (a, out) in row.iter_mut().enumerate() {
                let (x0, x1) = ((a * s).saturating_sub(s), (a * s + s).min(w));
                v.clear();
                for y in y0..y1 {
                    v.extend(lum[y * w + x0..y * w + x1].iter().filter(|l| !l.is_nan()));
                }
                if v.len() >= need {
                    *out = median(&mut v).unwrap();
                }
            }
        }
    });
    let lifted = std::sync::atomic::AtomicU64::new(0);
    crate::par_rows(&mut img.px, w, |y0, rows| {
        let mut n = 0;
        for (k, row) in rows.chunks_mut(w).enumerate() {
            let y = y0 + k;
            let fy = (y as f64 + 0.5) / s as f64;
            let (b, ty) = (fy.floor() as usize, (fy - fy.floor()) as f32);
            for (x, p) in row.iter_mut().enumerate() {
                if p[3] <= 0.0 {
                    continue;
                }
                let fx = (x as f64 + 0.5) / s as f64;
                let (a, tx) = (fx.floor() as usize, (fx - fx.floor()) as f32);
                let (mut sum, mut wsum) = (0.0f32, 0.0f32);
                for (da, db, wt) in [(0, 0, (1.0 - tx) * (1.0 - ty)), (1, 0, tx * (1.0 - ty)), (0, 1, (1.0 - tx) * ty), (1, 1, tx * ty)] {
                    let m = nodes[(b + db).min(nz - 1) * nx + (a + da).min(nx - 1)];
                    if !m.is_nan() && wt > 0.0 {
                        sum += m * wt;
                        wsum += wt;
                    }
                }
                if wsum <= 0.0 {
                    continue;
                }
                let target = ratio * sum / wsum;
                let l = luminance([p[0], p[1], p[2]]);
                if l < target {
                    if l > 1e-6 {
                        let k = target / l;
                        for c in 0..3 {
                            p[c] *= k;
                        }
                    } else {
                        p[0] = target;
                        p[1] = target;
                        p[2] = target;
                    }
                    n += 1;
                }
            }
        }
        lifted.fetch_add(n, std::sync::atomic::Ordering::Relaxed);
    });
    lifted.into_inner()
}

/// Hold every channel to [lo, hi].
pub fn clamp_albedo(img: &mut Image, lo: f32, hi: f32) {
    for p in &mut img.px {
        for c in 0..3 {
            p[c] = p[c].clamp(lo, hi);
        }
    }
}

/// The next mip level (`max(1, w / 2)` x `max(1, h / 2)`): each texel the box
/// average of its 2 x 2 children (an odd last row or column joins the last
/// parent) over the open ones only; open if any child is. A texel with no
/// open child takes the plain average (its colour is replaced by
/// [`fill_holes`]).
pub fn next_level(src: &Image) -> Image {
    let (w2, h2) = ((src.w / 2).max(1), (src.h / 2).max(1));
    let mut out = Image::new(w2, h2);
    let span = |p: usize, n: usize, n2: usize| (2 * p).min(n - 1)..if p + 1 == n2 { n } else { (2 * p + 2).min(n) };
    crate::par_rows(&mut out.px, w2, |y0, rows| {
        for (k, row) in rows.chunks_mut(w2).enumerate() {
            let y = y0 + k;
            for (x, o) in row.iter_mut().enumerate() {
                let (mut open, mut all) = ([0f32; 4], [0f32; 4]);
                for yy in span(y, src.h, h2) {
                    for xx in span(x, src.w, w2) {
                        let p = src.px[yy * src.w + xx];
                        let a = if p[3] > 0.0 { 1.0 } else { 0.0 };
                        for c in 0..3 {
                            open[c] += p[c] * a;
                            all[c] += p[c];
                        }
                        open[3] += a;
                        all[3] += 1.0;
                    }
                }
                *o = if open[3] > 0.0 { [open[0] / open[3], open[1] / open[3], open[2] / open[3], 1.0] } else { [all[0] / all[3], all[1] / all[3], all[2] / all[3], 0.0] };
            }
        }
    });
    out
}

/// Level 0 and every level under it, down to 1 x 1 (`level_dims`).
pub fn mip_chain(level0: Image) -> Vec<Image> {
    let mut chain = vec![level0];
    while chain.last().is_some_and(|l| l.w > 1 || l.h > 1) {
        let next = next_level(chain.last().unwrap());
        chain.push(next);
    }
    chain
}

/// Give every closed texel the colour of the (already filled) coarser level
/// under it, bilinearly, from the second-coarsest level up: the colour of
/// the open ground round a footprint runs on under it. Alpha is kept.
pub fn fill_holes(chain: &mut [Image]) {
    for l in (0..chain.len().saturating_sub(1)).rev() {
        let (fine, coarse) = chain.split_at_mut(l + 1);
        let (fine, coarse) = (&mut fine[l], &coarse[0]);
        let (fw, fh) = (fine.w, fine.h);
        let (sx, sy) = (coarse.w as f32 / fw as f32, coarse.h as f32 / fh as f32);
        crate::par_rows(&mut fine.px, fw, |y0, rows| {
            for (k, row) in rows.chunks_mut(fw).enumerate() {
                let y = y0 + k;
                for (x, p) in row.iter_mut().enumerate() {
                    if p[3] > 0.0 {
                        continue;
                    }
                    let c = sample_bilinear(coarse, (x as f32 + 0.5) * sx - 0.5, (y as f32 + 0.5) * sy - 0.5);
                    p[0] = c[0];
                    p[1] = c[1];
                    p[2] = c[2];
                }
            }
        });
    }
}

/// The RGB of `img` at continuous texel coordinates (centres at integers), clamped to the edge.
fn sample_bilinear(img: &Image, x: f32, y: f32) -> [f32; 3] {
    let x = x.clamp(0.0, (img.w - 1) as f32);
    let y = y.clamp(0.0, (img.h - 1) as f32);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(img.w - 1), (y0 + 1).min(img.h - 1));
    let (tx, ty) = (x - x0 as f32, y - y0 as f32);
    let at = |xx: usize, yy: usize| img.px[yy * img.w + xx];
    let mut out = [0f32; 3];
    for c in 0..3 {
        let top = at(x0, y0)[c] * (1.0 - tx) + at(x1, y0)[c] * tx;
        let bot = at(x0, y1)[c] * (1.0 - tx) + at(x1, y1)[c] * tx;
        out[c] = top * (1.0 - ty) + bot * ty;
    }
    out
}

/// The level as sRGB-encoded RGBA8, padded to a multiple of 4 texels each way
/// by repeating the last row and column (the blocks cover the padding):
/// (pixels, padded width, padded height). Alpha 255 open, 0 closed.
pub fn to_srgb8_padded(img: &Image) -> (Vec<u8>, usize, usize) {
    let (pw, ph) = (img.w.div_ceil(4) * 4, img.h.div_ceil(4) * 4);
    let mut lut = vec![0u8; 4097];
    for (i, v) in lut.iter_mut().enumerate() {
        *v = (eo_data::roof_colours::linear_to_srgb(i as f32 / 4096.0) * 255.0).round() as u8;
    }
    let enc = |c: f32| {
        let t = c.clamp(0.0, 1.0) * 4096.0;
        // the curve is steep near black: the exact value there, the table above
        if t < 64.0 { (eo_data::roof_colours::linear_to_srgb(c.clamp(0.0, 1.0)) * 255.0).round() as u8 } else { lut[t.round() as usize] }
    };
    let mut out = vec![0u8; pw * ph * 4];
    crate::par_rows(&mut out, pw * 4, |y0, rows| {
        for (k, row) in rows.chunks_mut(pw * 4).enumerate() {
            let sy = (y0 + k).min(img.h - 1);
            for x in 0..pw {
                let p = img.px[sy * img.w + x.min(img.w - 1)];
                row[x * 4..x * 4 + 4].copy_from_slice(&[enc(p[0]), enc(p[1]), enc(p[2]), if p[3] > 0.0 { 255 } else { 0 }]);
            }
        }
    });
    (out, pw, ph)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(w: usize, h: usize, c: [f32; 3]) -> Image {
        Image { w, h, px: vec![[c[0], c[1], c[2], 1.0]; w * h] }
    }

    #[test]
    fn lift_raises_shadows_to_the_neighbourhood_keeping_chroma() {
        let ground = [0.10, 0.12, 0.08];
        let mut img = flat(48, 48, ground);
        // A cast shadow, bluish, 3 x 3 texels; a closed (built) block as dark as night beside it.
        let shade = [0.010, 0.014, 0.020];
        for y in 20..23 {
            for x in 20..23 {
                img.px[y * 48 + x] = [shade[0], shade[1], shade[2], 1.0];
            }
        }
        for y in 10..16 {
            for x in 30..36 {
                img.px[y * 48 + x] = [0.001, 0.001, 0.001, 0.0];
            }
        }
        // A texel only a little darker than its ground: not a shadow.
        img.px[5 * 48 + 5] = [0.08, 0.10, 0.06, 1.0];
        let before = img.clone();
        let n = shadow_lift(&mut img, 12, SHADOW_RATIO);
        assert_eq!(n, 9, "the shadow's texels and nothing else");
        let target = SHADOW_RATIO * luminance(ground);
        let p = img.px[21 * 48 + 21];
        assert!((luminance([p[0], p[1], p[2]]) - target).abs() < 1e-4, "lifted to {} of the ground", SHADOW_RATIO);
        assert!((p[2] / p[0] - shade[2] / shade[0]).abs() < 1e-4 && (p[1] / p[0] - shade[1] / shade[0]).abs() < 1e-4, "chroma kept");
        assert_eq!(img.px[5 * 48 + 5], before.px[5 * 48 + 5]);
        assert_eq!(img.px[12 * 48 + 32], before.px[12 * 48 + 32], "closed texels untouched");
        assert_eq!(img.px[0], before.px[0]);
        // Dark ground everywhere (a forest) is its own neighbourhood: nothing lifts.
        let mut wood = flat(30, 30, [0.02, 0.03, 0.015]);
        assert_eq!(shadow_lift(&mut wood, 12, SHADOW_RATIO), 0);
    }

    #[test]
    fn colour_matrix_fit() {
        // Exact samples of a known matrix (5 colours, independent): the fit recovers it.
        let m: Mat3 = [[0.8, 0.3, -0.1], [-0.05, 1.1, 0.0], [0.02, -0.2, 0.9]];
        let ns = [[0.1, 0.12, 0.09], [0.3, 0.25, 0.2], [0.05, 0.08, 0.03], [0.2, 0.2, 0.22], [0.15, 0.1, 0.05]];
        let samples: Vec<_> = ns.iter().map(|&n| (n, mat_apply(&m, n), 1.0)).collect();
        let fit = fit_colour_matrix(&samples, 1e-7).unwrap();
        for r in 0..3 {
            for c in 0..3 {
                assert!((fit[r][c] - m[r][c]).abs() < 2e-3, "{fit:?}");
            }
        }
        assert!(fit_residual(&fit, &samples) < 1e-8);
        // Greys only (collinear): the ridge keeps the fit at the per-channel scale.
        let greys: Vec<_> = [0.1f32, 0.2, 0.3].iter().map(|&g| ([g; 3], [0.5 * g, 0.6 * g, 0.7 * g], 1.0)).collect();
        let fit = fit_colour_matrix(&greys, 0.02).unwrap();
        for (n, t, _) in &greys {
            let o = mat_apply(&fit, *n);
            assert!((0..3).all(|k| (o[k] - t[k]).abs() < 0.01 * t[k] + 1e-4), "{o:?} vs {t:?}");
        }
        assert!(fit.iter().flatten().all(|v| v.abs() < 1.0), "bounded: {fit:?}");
        assert_eq!(fit_colour_matrix(&[], 0.02), None);
        // The per-channel medians.
        let px = vec![[0.1, 0.5, 0.0, 1.0], [0.3, 0.1, 0.0, 1.0], [0.2, 0.3, 9.0, 1.0]];
        assert_eq!(channel_medians(&px, &[0, 1, 2]), [0.2, 0.3, 0.0]);
    }

    #[test]
    fn mips_average_open_texels_only() {
        // 4 x 4: the left half open (two colours), the right half closed (a roof).
        let mut img = Image::new(4, 4);
        for y in 0..4 {
            for x in 0..4 {
                img.px[y * 4 + x] = match (x, y) {
                    (0, _) => [0.2, 0.2, 0.2, 1.0],
                    (1, 0) | (1, 1) => [0.4, 0.4, 0.4, 1.0],
                    (1, _) => [0.2, 0.2, 0.2, 0.0],
                    _ => [0.9, 0.0, 0.0, 0.0],
                };
            }
        }
        let mut chain = mip_chain(img);
        assert_eq!(chain.iter().map(|l| (l.w, l.h)).collect::<Vec<_>>(), vec![(4, 4), (2, 2), (1, 1)]);
        assert_eq!(level_dims(4, 4), vec![(4, 4), (2, 2), (1, 1)]);
        let l1 = &chain[1];
        let near = |p: [f32; 4], q: [f32; 4]| (0..4).all(|c| (p[c] - q[c]).abs() < 1e-6);
        assert!(near(l1.px[0], [0.3, 0.3, 0.3, 1.0]), "four open children: {:?}", l1.px[0]);
        assert!(near(l1.px[2], [0.2, 0.2, 0.2, 1.0]), "two open children, the closed ones left out: {:?}", l1.px[2]);
        assert_eq!(l1.px[1][3], 0.0, "no open child: closed");
        assert_eq!(l1.px[3][3], 0.0);
        let top = chain[2].px[0];
        assert!((top[0] - 0.25).abs() < 1e-6 && top[3] == 1.0, "the open texels' mean, the roof's red absent: {top:?}");
        fill_holes(&mut chain);
        // The closed texels now carry the open ground's colour, not the roof's; alpha kept.
        for l in &chain {
            for p in &l.px {
                assert!(p[0] < 0.45 && (p[0] - p[1]).abs() < 1e-6, "{p:?}");
            }
        }
        assert_eq!(chain[0].px[3][3], 0.0);
        assert_eq!(chain[0].px[0], [0.2, 0.2, 0.2, 1.0], "open texels unchanged");
        // Odd sizes follow the GPU's mip sizes; the odd last column joins the last parent.
        assert_eq!(level_dims(7, 3), vec![(7, 3), (3, 1), (1, 1)]);
        let mut odd = flat(3, 1, [0.0, 0.0, 0.0]);
        odd.px[2] = [0.3, 0.3, 0.3, 1.0];
        assert!((next_level(&odd).px[0][0] - 0.1).abs() < 1e-6);
    }

    #[test]
    fn srgb_padding_and_alpha() {
        let mut img = flat(5, 3, [0.5, 0.0, 1.0]);
        img.px[0][3] = 0.0;
        let (px, pw, ph) = to_srgb8_padded(&img);
        assert_eq!((pw, ph), (8, 4));
        assert_eq!(&px[0..4], &[188, 0, 255, 0]);
        assert_eq!(&px[4..8], &[188, 0, 255, 255]);
        assert_eq!(&px[(3 * 8 + 7) * 4..(3 * 8 + 7) * 4 + 4], &[188, 0, 255, 255], "padding repeats the edge");
    }

    #[test]
    fn header_round_trip() {
        let (w, h) = (10u32, 6u32);
        let levels = level_records(w, h);
        assert_eq!(levels.iter().map(|l| (l.width, l.height)).collect::<Vec<_>>(), vec![(10, 6), (5, 3), (2, 1), (1, 1)]);
        assert_eq!(levels[0].length, 3 * 2 * 16);
        assert_eq!(levels[1].offset, 96);
        let mut stats = Stats { texels: 60, lifted_texels: 3, built_texels: 4, no_data_texels: 5, calibration_texels: 6, ..Default::default() };
        stats.medians.insert("lawn".into(), 0.08);
        let header = Header {
            format: FORMAT.into(),
            origin_x: -7120.0,
            origin_z: -6100.5,
            texel_m: TEXEL_M,
            width: w,
            height: h,
            levels: levels.clone(),
            k: 1.0,
            lawn_target: 0.08,
            colour_mode: "classes".into(),
            colour_transfer: Some(ColourTransfer {
                classes: vec![TransferClass { name: "dry".into(), palette: [0.26, 0.24, 0.14], texels: 5, ..Default::default() }],
                params: crate::transfer::Params::default(),
                soil_eroded: [0.2, 0.1, 0.06],
                paving_gain: 1.1,
                paving_road_lum: 0.27,
                ndvi: "test".into(),
                cir_haze: [12.0, 20.0],
                palette_source: "test".into(),
            }),
            colour_matrix: [[0.9, 0.1, 0.0], [0.0, 1.0, 0.05], [0.0, -0.1, 1.1]],
            colour_ridge: COLOUR_RIDGE,
            colour_residual: 0.001,
            colour_classes: vec![ClassFit { name: "roads".into(), weight: 1.0, texels: 9, naip_median: [0.1, 0.1, 0.12], target: [0.1; 3], ..Default::default() }],
            haze: [0.01, 0.02, 0.03],
            shadow_ratio: SHADOW_RATIO,
            shadow_window_m: SHADOW_WINDOW_M,
            albedo_range: [ALBEDO_MIN, ALBEDO_MAX],
            stats,
            inputs: Inputs { naip_manifest_sha256: "ab".repeat(32), naip_tiles_used: 2, naip_licence: "Public domain".into(), ..Default::default() },
            generator: GENERATOR.into(),
        };
        let blocks: Vec<Vec<u8>> = levels.iter().enumerate().map(|(i, l)| vec![i as u8; l.length as usize]).collect();
        let dir = std::env::temp_dir().join(format!("eo-hills-test-{}", std::process::id()));
        let path = file_path(&dir);
        write(&path, &header, &blocks).unwrap();
        let (back, start) = read_header(&path).unwrap();
        assert_eq!(back, header);
        let bytes = std::fs::read(&path).unwrap();
        for (i, l) in back.levels.iter().enumerate() {
            let at = (start + l.offset) as usize;
            assert!(bytes[at..at + l.length as usize].iter().all(|&b| b == i as u8), "level {i} where the header says");
        }
        // Wrong blocks, a cut file and a bad magic are refused.
        assert!(write(&path.with_extension("x"), &header, &blocks[..2]).is_err());
        std::fs::write(&path, &bytes[..bytes.len() - 1]).unwrap();
        assert!(read_header(&path).is_err());
        let mut bad = bytes.clone();
        bad[0] = b'X';
        std::fs::write(&path, &bad).unwrap();
        assert!(read_header(&path).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
