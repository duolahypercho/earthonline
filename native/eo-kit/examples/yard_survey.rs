//! Rear yards measured for `--yard-detail`: the fences on the lot lines and
//! the ground between them, from the parcel map, the lidar and the aerial.
//!
//! Inputs, all in folders OUTSIDE the repository (never copied into it):
//! * parcels: DataSF "Parcels - Active and Retired" (acdm-wktn, PDDL 1.0),
//!   fetched for one area by `native/tools/fetch-parcels.py` into
//!   `$EO_PARCELS_DIR/<name>/` (parcels.geojson + manifest.json, digest checked);
//! * lidar: USGS 3DEP CA_SanFrancisco_B23 (public domain), fetched by
//!   `native/tools/fetch-lidar.py` into `$EO_LIDAR_DIR/<name>/` (digests checked),
//!   and that folder's `trees.json` from `lidar_survey` (its measured plan shift
//!   between the cloud and the footprints is applied here too);
//! * aerial: USDA NAIP natural colour at 0.6 m (public domain) in
//!   `$EO_AERIAL_DIR` (`manifest.json`, `rgb06/` tiles, digests checked).
//!
//!   cargo run --release -p eo-kit --example yard_survey -- liberty-hill
//!
//! Writes into `$EO_PARCELS_DIR/<name>/`:
//! * `yards.json`: every lot line (the boundary two parcels share), whether the
//!   lidar finds a fence on it, and each fenced span's ends (surveyed 1:1
//!   plan metres, x east, z south, the footprints' frame) and measured height;
//!   the ground classes' measured tones and areas; the method's numbers.
//! * `yard-ground.bin.gz`: the yard ground class of every 0.5 m cell
//!   (`YARD_GROUND_MAGIC`, then nx, nz as u32, x0, z0, cell as f32, then one
//!   byte a cell: 0 not yard, 1 lawn, 2 planting, 3 paving, 4 deck).
//! * `yard-canopy.bin.gz`: every yard cell's vegetation and colour, for the
//!   yards' plants and ground (`YARD_CANOPY_MAGIC`, the same header as the
//!   ground's, then five bytes a cell: the lidar's vegetation height over the
//!   ground in 0.1 m, 0 off the vegetation; the aerial's dehazed colour as
//!   sRGB8, a shadowed cell taking the nearest sunlit cell's of its class and
//!   parcel within `SHADOW_REACH_M`, else its class's median tone; the class
//!   with the paving squared (`PAVE_RECT_*`; the ground file keeps it as
//!   classified); zeros off the yards).
//! * `evidence-*.png`: the aerial with the lot lines and the fences found, the
//!   lidar's height above ground, the ground classes.
//!
//! Method (0.5 m grid over the lidar area, as `lidar_survey`):
//! * ground: the lowest class-2 return in a cell, holes filled from the nearest;
//!   surface: the highest return (noise 7 and 18 left out); height above ground.
//! * lot lines: the stretches of a parcel's outline within `SHARED_M` of
//!   another parcel's (condominium copies of one outline, `mapblklot`, counted
//!   once): the side and rear lines; a street frontage has no neighbour. A
//!   station (every `STATION_M` along a line) is in a yard when it is more than
//!   `CLEAR_OF_FOOTPRINT_M` from every footprint.
//! * fences: across each station, the returns within `STATION_M / 2` along and
//!   `PROFILE_HALF_M` across, binned every `PROFILE_BIN_M`, each bin's highest
//!   return under `PROFILE_CEILING_M` over its ground (a tree's crown or an eave
//!   over the line is left out). A fence at an offset:
//!   the highest return within `TOP_BAND_M` of it stands `FENCE_MIN_M` to
//!   `FENCE_MAX_M` over the higher side's ground (a bare retaining wall does
//!   not count), the raised part is at most `FENCE_WIDTH_M` wide (a hedge or a
//!   shed is wider), and on at least one side, `SIDE_NEAR_M` to `SIDE_FAR_M`
//!   off, nothing stands within `FENCE_DROP_M` of its top. The parcel map's
//!   shift against the cloud is measured once (the shift, within
//!   `SHIFT_MAX_M`, that puts the most stations on a fence), then each line's
//!   own offset within `LINE_SLACK_M`. A fenced span: a run of yard stations
//!   (gaps of at most `GAP_STATIONS` bridged) of which at least
//!   `MIN_SPAN_STATIONS` and `CONFIRM_SHARE` pass (a fence under a crown or
//!   behind a shrub fails some stations it stands at), drawn over the run; its
//!   height the `HEIGHT_QUANTILE` (median) of its passing stations' heights.
//! * ground classes, yard cells only (inside a parcel, off the footprints by
//!   `CLEAR_OF_FOOTPRINT_M`, not a structure): the aerial is shifted onto the
//!   plan by the shift (within `SHIFT_MAX_M`) at which its green best matches
//!   the lidar's vegetation, and dehazed as the roof survey does (0.2 % darkest
//!   level per channel). Structure: non-vegetation at least `STRUCTURE_M` up,
//!   wider than a fence (a 3 x 3 opening), or raised flat ground of more than
//!   `DECK_MAX_M2` (a roof the game may not draw). Deck: non-vegetation
//!   `DECK_MIN_M` to `STRUCTURE_M` up, flat (3 x 3 range under `DECK_FLAT_M`),
//!   `DECK_MIN_M2` to `DECK_MAX_M2`. Planting: the lidar's vegetation (at least
//!   `VEG_SHARE` of the returns split by leaves, `VEG_MIN_M` up), or an aerial
//!   green (excess green over Otsu's threshold of the yard cells) that is dark
//!   or rough, or bare soil (warm, unsaturated green). Lawn: aerial green,
//!   bright and smooth (luma and 3 x 3 luma range against the green cells'
//!   medians). Paving: the rest at ground level. Cells in deep shadow (luma
//!   under `SHADOW_LUMA`) take their class from the nearest sunlit cell of the
//!   same parcel within `SHADOW_REACH_M`, else paving. Then `MODE_PASSES` of
//!   a 3 x 3 majority over the yard cells (a cell's lone class is the
//!   classifier's speckle, not a bed). Then each paving region that fills
//!   enough of its least-area rectangle becomes that rectangle (`PAVE_RECT_*`). A class's tone: the median dehazed
//!   colour of its sunlit cells (recorded; the game draws the city's own
//!   ground tones, the aerial's colour being greyed by haze and compression).

use las::Reader;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

const CELL: f64 = 0.5;
const R_MERC: f64 = 6378137.0;
const YARD_GROUND_MAGIC: &[u8; 8] = b"EOYARDG1";
const YARD_CANOPY_MAGIC: &[u8; 8] = b"EOYARDC2";

// lot lines
const SHARED_M: f64 = 0.3;
const STATION_M: f64 = 0.5;
const CLEAR_OF_FOOTPRINT_M: f64 = 0.5;
const MIN_LINE_M: f64 = 1.0;
// fences
const PROFILE_HALF_M: f64 = 2.0;
const PROFILE_BIN_M: f64 = 0.1;
const TOP_BAND_M: f64 = 0.2;
const FENCE_MIN_M: f64 = 0.9;
const FENCE_MAX_M: f64 = 3.0;
const PROFILE_CEILING_M: f64 = 3.3;
const FENCE_WIDTH_M: f64 = 0.8;
const FENCE_DROP_M: f64 = 0.6;
const SIDE_NEAR_M: f64 = 0.5;
const SIDE_FAR_M: f64 = 1.2;
const SHIFT_MAX_M: f64 = 1.5;
const LINE_SLACK_M: f64 = 0.4;
const GAP_STATIONS: usize = 2;
const MIN_SPAN_STATIONS: usize = 3;
const HEIGHT_QUANTILE: f64 = 0.5;
const CONFIRM_SHARE: f64 = 0.3;
// ground
const VEG_SHARE: f64 = 0.25;
const VEG_MIN_M: f64 = 0.5;
const STRUCTURE_M: f64 = 3.5;
const DECK_MIN_M: f64 = 0.25;
const DECK_FLAT_M: f64 = 0.3;
const DECK_MIN_M2: f64 = 2.0;
const DECK_MAX_M2: f64 = 80.0;
const SHADOW_LUMA: f64 = 0.06;
const SHADOW_REACH_M: f64 = 6.0;
const MODE_PASSES: usize = 2;
/// Paving laid in rectangles: a connected paving region of one parcel (a
/// region the classifier ran across a lot line is each yard's own patio) of at
/// least `PAVE_RECT_MIN_M2` that fills `PAVE_RECT_FILL` of its least-area
/// bounding rectangle (orientations every degree) becomes that rectangle (a
/// patio is poured or laid square; the classifier's edge is the aerial's blur
/// and the shadows, not the slab's). A region that fills less is cut across its
/// rectangle's long side where the two parts fill theirs best, up to
/// `PAVE_RECT_SPLITS` times (an L-shaped patio, a path beside a slab), each part
/// laid the same way; what no rectangle takes goes to its neighbours' class.
const PAVE_RECT_MIN_M2: f64 = 2.0;
const PAVE_RECT_FILL: f64 = 0.55;
const PAVE_RECT_SPLITS: u32 = 2;
/// A lot larger than this (m^2) is a park, a school's or an institution's
/// grounds, not a house's yard (the city's residential lots are 25 x 100 ft,
/// 232 m^2, a few times that at most; the pilot's largest house lot is 911 m^2
/// and Mission Dolores Park's lot 27,019 m^2): the canopy file draws none of
/// its ground (class 0), so the yards plant and furnish none of it. The ground
/// file keeps it, as it was.
const YARD_LOT_MAX_M2: f64 = 4000.0;

/// The least-area rectangle over cell centres `pts` (each cell `CELL` square):
/// its area, angle and bounds along (u, v) = the angle's axes.
fn least_rect(pts: &[(f64, f64)]) -> (f64, f64, [f64; 4]) {
    let mut best = (f64::MAX, 0.0, [0.0f64; 4]);
    for deg in 0..90 {
        let a = (deg as f64).to_radians();
        let (s, c) = a.sin_cos();
        let (mut u0, mut u1, mut v0, mut v1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for &(x, z) in pts {
            let (u, v) = (x * c + z * s, -x * s + z * c);
            u0 = u0.min(u);
            u1 = u1.max(u);
            v0 = v0.min(v);
            v1 = v1.max(v);
        }
        let h = 0.5 * CELL;
        let r = (u1 - u0 + 2.0 * h) * (v1 - v0 + 2.0 * h);
        if r < best.0 {
            best = (r, a, [u0 - h, u1 + h, v0 - h, v1 + h]);
        }
    }
    best
}

/// A paving region's rectangles (angle, bounds; `least_rect`): its own if it
/// fills `PAVE_RECT_FILL` of it, else its two parts' after the best cut across
/// the long side (up to `splits` deep); parts under `PAVE_RECT_MIN_M2` lay none.
fn pave_rects(pts: &[(f64, f64)], splits: u32) -> Vec<(f64, [f64; 4])> {
    let area = pts.len() as f64 * CELL * CELL;
    if area < PAVE_RECT_MIN_M2 {
        return vec![];
    }
    let (rect_area, a, b) = least_rect(pts);
    if area >= PAVE_RECT_FILL * rect_area {
        return vec![(a, b)];
    }
    if splits == 0 {
        return vec![];
    }
    let (s, c) = a.sin_cos();
    let along_u = b[1] - b[0] >= b[3] - b[2];
    let key = |&(x, z): &(f64, f64)| if along_u { x * c + z * s } else { -x * s + z * c };
    let (lo, hi) = if along_u { (b[0], b[1]) } else { (b[2], b[3]) };
    let mut best: Option<(f64, Vec<(f64, f64)>, Vec<(f64, f64)>)> = None;
    for step in 4..=16 {
        let cut = lo + (hi - lo) * step as f64 / 20.0;
        let (p, q): (Vec<_>, Vec<_>) = pts.iter().partition(|pt| key(pt) < cut);
        if p.is_empty() || q.is_empty() {
            continue;
        }
        let fill = |v: &[(f64, f64)]| v.len() as f64 * CELL * CELL / least_rect(v).0;
        let score = fill(&p) * p.len() as f64 + fill(&q) * q.len() as f64;
        if best.as_ref().map_or(true, |b| score > b.0) {
            best = Some((score, p, q));
        }
    }
    let Some((_, p, q)) = best else { return vec![] };
    let mut out = pave_rects(&p, splits - 1);
    out.extend(pave_rects(&q, splits - 1));
    out
}

const NONE: u8 = 0;
const LAWN: u8 = 1;
const PLANTING: u8 = 2;
const PAVING: u8 = 3;
const DECK: u8 = 4;
const CLASS_NAMES: [&str; 5] = ["none", "lawn", "planting", "paving", "deck"];

fn env_dir(var: &str, what: &str) -> PathBuf {
    PathBuf::from(std::env::var(var).unwrap_or_else(|_| panic!("set {var} to the folder {what} wrote")))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|v| format!("{v:02x}")).collect()
}

fn read_checked(p: &Path, sha: &str) -> Vec<u8> {
    let bytes = std::fs::read(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    assert_eq!(hex(&Sha256::digest(&bytes)), sha, "{} does not match its manifest digest", p.display());
    bytes
}

fn write_png(path: &Path, w: usize, h: usize, rgb: &[u8]) {
    let f = std::fs::File::create(path).expect("png file");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().expect("png header").write_image_data(rgb).expect("png data");
}

fn point_in_ring(x: f64, z: f64, ring: &[[f64; 2]]) -> bool {
    let mut inside = false;
    let n = ring.len();
    let mut j = n - 1;
    for i in 0..n {
        let (xi, zi, xj, zj) = (ring[i][0], ring[i][1], ring[j][0], ring[j][1]);
        if (zi > z) != (zj > z) && x < (xj - xi) * (z - zi) / (zj - zi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn dist_to_ring(x: f64, z: f64, ring: &[[f64; 2]]) -> f64 {
    let mut best = f64::MAX;
    let n = ring.len();
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
        let l2 = dx * dx + dz * dz;
        let t = if l2 > 0.0 { (((x - a[0]) * dx + (z - a[1]) * dz) / l2).clamp(0.0, 1.0) } else { 0.0 };
        best = best.min((a[0] + t * dx - x).hypot(a[1] + t * dz - z));
    }
    best
}

fn bbox(ring: &[[f64; 2]]) -> [f64; 4] {
    ring.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])])
}

fn quantile(v: &mut [f64], q: f64) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    v[((v.len() - 1) as f64 * q).round() as usize]
}

struct Grid {
    x0: f64,
    z0: f64,
    n: usize,
}

impl Grid {
    fn idx(&self, x: f64, z: f64) -> Option<usize> {
        let i = ((x - self.x0) / CELL).floor();
        let j = ((z - self.z0) / CELL).floor();
        if i < 0.0 || j < 0.0 || i >= self.n as f64 || j >= self.n as f64 {
            return None;
        }
        Some(j as usize * self.n + i as usize)
    }
    fn centre(&self, k: usize) -> (f64, f64) {
        (self.x0 + ((k % self.n) as f64 + 0.5) * CELL, self.z0 + ((k / self.n) as f64 + 0.5) * CELL)
    }
}

/// A parcel's outline (one polygon of its multipolygon) on the 1:1 plan.
struct Parcel {
    id: String,
    ring: Vec<[f64; 2]>,
    bb: [f64; 4],
}

/// One lot line: a stretch of parcel `a`'s outline that parcel `b` shares.
struct LotLine {
    a: String,
    b: String,
    p: [f64; 2],
    q: [f64; 2],
}

/// One station across a lot line: its centre, along and across directions,
/// whether it is in a yard, and its profile across (highest return and ground per bin).
struct Station {
    c: [f64; 2],
    nrm: [f64; 2],
    yard: bool,
    top: Vec<f64>,
    /// per bin: a return above `PROFILE_CEILING_M` with none under it (a roof or a wall's top: solid).
    solid: Vec<bool>,
    ground: Vec<f64>,
}

const BINS: usize = (2.0 * PROFILE_HALF_M / PROFILE_BIN_M) as usize;

impl Station {
    fn bin_of(u: f64) -> i64 {
        ((u + PROFILE_HALF_M) / PROFILE_BIN_M).floor() as i64
    }
    /// The fence test at offset `u0` (m across): Some(height over the higher side's ground).
    fn fence_at(&self, u0: f64) -> Option<f64> {
        self.fence_test(u0).ok()
    }
    /// `fence_at`, or why not: 0 no return, 1 no ground beside, 2 too low, 3 too high, 4 too wide, 5 both sides raised.
    fn fence_test(&self, u0: f64) -> Result<f64, (u8, f64)> {
        let b = |u: f64| Self::bin_of(u).clamp(0, BINS as i64 - 1) as usize;
        let (c0, c1) = (b(u0 - TOP_BAND_M), b(u0 + TOP_BAND_M));
        let top = (c0..=c1).map(|k| self.top[k]).fold(f64::NEG_INFINITY, f64::max);
        if !top.is_finite() {
            return Err((0, 0.0));
        }
        let side_ground = |lo: f64, hi: f64| {
            let mut g: Vec<f64> = (b(lo)..=b(hi)).map(|k| self.ground[k]).filter(|v| v.is_finite()).collect();
            if g.is_empty() { None } else { Some(quantile(&mut g, 0.5)) }
        };
        let (Some(gl), Some(gr)) = (side_ground(u0 - SIDE_FAR_M, u0 - SIDE_NEAR_M), side_ground(u0 + SIDE_NEAR_M, u0 + SIDE_FAR_M)) else { return Err((1, 0.0)) };
        let h = top - gl.max(gr);
        if h < FENCE_MIN_M {
            return Err((2, h));
        }
        if h > FENCE_MAX_M {
            return Err((3, h));
        }
        // the raised part's width: contiguous bins round the top at least half its height up.
        let raised = |k: usize| self.top[k].is_finite() && self.top[k] - self.ground[k].max(gl.min(gr)) >= 0.5 * h;
        let centre = (c0..=c1).max_by(|&x, &y| self.top[x].total_cmp(&self.top[y])).unwrap_or(c0);
        let (mut lo, mut hi) = (centre, centre);
        while lo > 0 && raised(lo - 1) {
            lo -= 1;
        }
        while hi + 1 < BINS && raised(hi + 1) {
            hi += 1;
        }
        if (hi - lo + 1) as f64 * PROFILE_BIN_M > FENCE_WIDTH_M {
            return Err((4, h));
        }
        // one side clear: nothing there within FENCE_DROP_M of the top.
        // a side is clear when no bin there stands within FENCE_DROP_M of the top and none is solid.
        let side_max = |lo: f64, hi: f64| (b(lo)..=b(hi)).map(|k| if self.solid[k] { f64::INFINITY } else if self.top[k].is_finite() { self.top[k] - self.ground[k] } else { 0.0 }).fold(0.0f64, f64::max);
        let (sl, sr) = (side_max(u0 - SIDE_FAR_M, u0 - SIDE_NEAR_M), side_max(u0 + SIDE_NEAR_M, u0 + SIDE_FAR_M));
        if sl.min(sr) > h - FENCE_DROP_M {
            return Err((5, h));
        }
        Ok(h)
    }
}

/// The aerial: a lon/lat pixel grid of 2000 px tiles (fetch-aerial's `grids.rgb`).
/// `square`: each tile as the image service served it, its requested box widened
/// north and south about its centre until its pixels are square in degrees
/// (`dlon` each way; the request asked 2000 x 2000 px of a box `dlon` x `dlat` a
/// pixel), not the box the manifest records. Which of the two holds is measured
/// (`main`: the one whose green matches the lidar's vegetation).
struct Aerial {
    square: bool,
    west: f64,
    north: f64,
    dlon: f64,
    dlat: f64,
    tile: usize,
    tiles: HashMap<(usize, usize), Vec<u8>>,
    haze: [f32; 3],
    files: Vec<serde_json::Value>,
}

impl Aerial {
    fn load(dir: &Path, lon0: f64, lat0: f64, lon1: f64, lat1: f64) -> Aerial {
        let m: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).expect("aerial manifest.json")).expect("aerial manifest");
        let g = &m["grids"]["rgb"];
        let (west, north, dlon, dlat, tile) = (g["west"].as_f64().unwrap(), g["north"].as_f64().unwrap(), g["dlon"].as_f64().unwrap(), g["dlat"].as_f64().unwrap(), g["tile"].as_u64().unwrap() as usize);
        let sums: HashMap<String, String> = m["files"].as_array().unwrap().iter().map(|f| (f["file"].as_str().unwrap().to_string(), f["sha256"].as_str().unwrap().to_string())).collect();
        let (c0, c1) = (((lon0 - west) / dlon) as usize / tile, ((lon1 - west) / dlon) as usize / tile);
        let (r0, r1) = (((north - lat1) / dlat) as usize / tile, ((north - lat0) / dlat) as usize / tile);
        let mut tiles = HashMap::new();
        let mut files = vec![];
        for r in r0..=r1 {
            for c in c0..=c1 {
                let name = format!("rgb06/r{r:03}_c{c:03}.jpg");
                let sha = sums.get(&name).unwrap_or_else(|| panic!("{name} is not in the aerial manifest"));
                let bytes = read_checked(&dir.join(&name), sha);
                let mut d = jpeg_decoder::Decoder::new(&bytes[..]);
                let px = d.decode().expect("jpeg");
                let info = d.info().unwrap();
                assert!(info.pixel_format == jpeg_decoder::PixelFormat::RGB24 && info.width as usize == tile && info.height as usize == tile, "{name}: not a {tile} px RGB tile");
                tiles.insert((r, c), px);
                files.push(serde_json::json!({ "file": name, "sha256": sha }));
            }
        }
        // haze: the 0.2 % darkest level per channel over these tiles (roof_survey's dehaze).
        let mut hist = [[0u64; 256]; 3];
        let mut n = 0u64;
        for px in tiles.values() {
            for p in px.chunks_exact(3) {
                if p != [0, 0, 0] {
                    for k in 0..3 {
                        hist[k][p[k] as usize] += 1;
                    }
                    n += 1;
                }
            }
        }
        let mut haze = [0f32; 3];
        for k in 0..3 {
            let mut acc = 0;
            for (v, c) in hist[k].iter().enumerate() {
                acc += c;
                if acc as f64 >= n as f64 * 0.002 {
                    haze[k] = eo_data::roof_colours::srgb_to_linear(v as f32 / 255.0).min(0.2);
                    break;
                }
            }
        }
        Aerial { square: false, west, north, dlon, dlat, tile, tiles, haze, files }
    }
    /// Dehazed linear RGB at (lon, lat), bilinear.
    fn at(&self, lon: f64, lat: f64) -> Option<[f64; 3]> {
        let gx = (lon - self.west) / self.dlon - 0.5;
        let mut gy = (self.north - lat) / self.dlat - 0.5;
        if self.square {
            let r = ((self.north - lat) / self.dlat / self.tile as f64).floor();
            let lat_c = self.north - (r + 0.5) * self.tile as f64 * self.dlat;
            gy = r * self.tile as f64 + (lat_c - lat) / self.dlon + self.tile as f64 / 2.0 - 0.5;
        }
        let (x0, y0) = (gx.floor(), gy.floor());
        let (tx, ty) = (gx - x0, gy - y0);
        let mut acc = [0.0; 3];
        for (dx, dy, w) in [(0.0, 0.0, (1.0 - tx) * (1.0 - ty)), (1.0, 0.0, tx * (1.0 - ty)), (0.0, 1.0, (1.0 - tx) * ty), (1.0, 1.0, tx * ty)] {
            let (px, py) = ((x0 + dx) as i64, (y0 + dy) as i64);
            if px < 0 || py < 0 {
                return None;
            }
            let (px, py) = (px as usize, py as usize);
            let t = self.tiles.get(&(py / self.tile, px / self.tile))?;
            let o = ((py % self.tile) * self.tile + px % self.tile) * 3;
            for k in 0..3 {
                let l = eo_data::roof_colours::srgb_to_linear(t[o + k] as f32 / 255.0);
                let d = self.haze[k];
                acc[k] += w * ((l - d) / (1.0 - d)).max(0.0) as f64;
            }
        }
        Some(acc)
    }
}

fn luma(c: [f64; 3]) -> f64 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// Excess green on the chromaticity: 2g - r - b over r + g + b.
fn exg(c: [f64; 3]) -> f64 {
    let s = (c[0] + c[1] + c[2]).max(1e-6);
    (2.0 * c[1] - c[0] - c[2]) / s
}

/// Otsu's threshold of `v` (256 bins between its extremes).
fn otsu(v: &[f64]) -> f64 {
    let (lo, hi) = v.iter().fold((f64::MAX, f64::MIN), |a, &x| (a.0.min(x), a.1.max(x)));
    let mut hist = [0f64; 256];
    for &x in v {
        hist[(((x - lo) / (hi - lo).max(1e-9)) * 255.0) as usize] += 1.0;
    }
    let total: f64 = hist.iter().sum();
    let sum_all: f64 = hist.iter().enumerate().map(|(i, h)| i as f64 * h).sum();
    let (mut wb, mut sb, mut best, mut at) = (0.0, 0.0, 0.0, 0usize);
    for (i, h) in hist.iter().enumerate() {
        wb += h;
        if wb == 0.0 || wb == total {
            continue;
        }
        sb += i as f64 * h;
        let (mb, mf) = (sb / wb, (sum_all - sb) / (total - wb));
        let between = wb * (total - wb) * (mb - mf) * (mb - mf);
        if between > best {
            best = between;
            at = i;
        }
    }
    lo + (at as f64 + 0.5) / 255.0 * (hi - lo)
}

fn main() {
    let name = std::env::args().nth(1).expect("usage: yard_survey <area name>");
    let pdir = env_dir("EO_PARCELS_DIR", "fetch-parcels.py").join(&name);
    let ldir = env_dir("EO_LIDAR_DIR", "fetch-lidar.py").join(&name);
    let adir = env_dir("EO_AERIAL_DIR", "fetch-aerial.mjs");
    let root = eo_kit::repo_root().expect("repo root (EO_REPO_ROOT)");
    let city = eo_data::City::load_at(&eo_data::DataPaths { repo_root: root }, 1.0).expect("city at 1:1");
    let frame = eo_data::roof_colours::PlanFrame::from_meta(&city.meta);

    // ---- the area: the lidar's, less its nodes' edge
    let lman: serde_json::Value = serde_json::from_slice(&std::fs::read(ldir.join("manifest.json")).expect("lidar manifest.json")).unwrap();
    let area = &lman["area"];
    let (lat, lon, half) = (area["lat"].as_f64().unwrap(), area["lon"].as_f64().unwrap(), area["half_m"].as_f64().unwrap() - 2.0);
    let (cx, cz) = frame.to_plan(lon, lat);
    let n = (2.0 * half / CELL).floor() as usize;
    let grid = Grid { x0: cx - half, z0: cz - half, n };
    let cells = n * n;
    let inside = |x: f64, z: f64| (x - cx).abs() < half && (z - cz).abs() < half;

    // ---- parcels (one outline per mapblklot)
    let pman: serde_json::Value = serde_json::from_slice(&std::fs::read(pdir.join("manifest.json")).expect("parcels manifest.json")).unwrap();
    let pfile = &pman["files"][0];
    let pbytes = read_checked(&pdir.join(pfile["file"].as_str().unwrap()), pfile["sha256"].as_str().unwrap());
    let gj: serde_json::Value = serde_json::from_slice(&pbytes).expect("parcels geojson");
    let mut parcels: Vec<Parcel> = vec![];
    let mut seen: HashMap<String, String> = HashMap::new();
    let mut records = 0;
    for f in gj["features"].as_array().unwrap() {
        records += 1;
        let p = &f["properties"];
        let id = p["mapblklot"].as_str().or(p["blklot"].as_str()).unwrap_or_default().to_string();
        let blklot = p["blklot"].as_str().unwrap_or_default().to_string();
        if let Some(prev) = seen.get(&id) {
            if *prev != blklot {
                continue; // a condominium copy of an outline already read
            }
        }
        seen.insert(id.clone(), blklot);
        for poly in f["geometry"]["coordinates"].as_array().into_iter().flatten() {
            let outer = &poly[0];
            let mut ring: Vec<[f64; 2]> = outer.as_array().unwrap().iter().map(|c| {
                let (x, z) = frame.to_plan(c[0].as_f64().unwrap(), c[1].as_f64().unwrap());
                [x, z]
            }).collect();
            if ring.len() > 1 && ring[0] == ring[ring.len() - 1] {
                ring.pop();
            }
            if ring.len() >= 3 {
                parcels.push(Parcel { id: id.clone(), bb: bbox(&ring), ring });
            }
        }
    }
    println!("parcels: {records} records, {} outlines of {} distinct parcels", parcels.len(), seen.len());

    // ---- footprints at 1:1, the yard mask's walls
    let fps: Vec<Vec<[f64; 2]>> = city
        .buildings
        .iter()
        .filter(|b| b.ring.iter().any(|p| (p[0] as f64 - cx).abs() < half + 40.0 && (p[1] as f64 - cz).abs() < half + 40.0))
        .map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect())
        .collect();
    let mut fp_mask = vec![false; cells];
    for ring in &fps {
        let bb = bbox(ring);
        let (i0, j0) = (((bb[0] - CLEAR_OF_FOOTPRINT_M - grid.x0) / CELL).floor().max(0.0) as usize, ((bb[1] - CLEAR_OF_FOOTPRINT_M - grid.z0) / CELL).floor().max(0.0) as usize);
        let (i1, j1) = ((((bb[2] + CLEAR_OF_FOOTPRINT_M - grid.x0) / CELL).ceil().max(0.0) as usize).min(n - 1), (((bb[3] + CLEAR_OF_FOOTPRINT_M - grid.z0) / CELL).ceil().max(0.0) as usize).min(n - 1));
        for j in j0..=j1 {
            for i in i0..=i1 {
                let k = j * n + i;
                let (x, z) = grid.centre(k);
                if point_in_ring(x, z, ring) || dist_to_ring(x, z, ring) < CLEAR_OF_FOOTPRINT_M {
                    fp_mask[k] = true;
                }
            }
        }
    }
    let near_footprint = |x: f64, z: f64| fps.iter().any(|r| {
        let bb = bbox(r);
        x > bb[0] - 1.0 && x < bb[2] + 1.0 && z > bb[1] - 1.0 && z < bb[3] + 1.0 && (point_in_ring(x, z, r) || dist_to_ring(x, z, r) < CLEAR_OF_FOOTPRINT_M)
    });
    // the parcel of each cell (index into `parcels`, or none: a street)
    let mut parcel_of = vec![u32::MAX; cells];
    for (pi, p) in parcels.iter().enumerate() {
        let (i0, j0) = (((p.bb[0] - grid.x0) / CELL).floor().max(0.0) as usize, ((p.bb[1] - grid.z0) / CELL).floor().max(0.0) as usize);
        let (i1, j1) = ((((p.bb[2] - grid.x0) / CELL).ceil().max(0.0) as usize).min(n - 1), (((p.bb[3] - grid.z0) / CELL).ceil().max(0.0) as usize).min(n - 1));
        for j in j0..=j1 {
            for i in i0..=i1 {
                let k = j * n + i;
                let (x, z) = grid.centre(k);
                if point_in_ring(x, z, &p.ring) {
                    parcel_of[k] = pi as u32;
                }
            }
        }
    }

    // ---- the cloud, in the footprints' frame (lidar_survey's measured shift)
    let trees: serde_json::Value = serde_json::from_slice(&std::fs::read(ldir.join("trees.json")).expect("trees.json (run lidar_survey first)")).unwrap();
    let ps = &trees["provenance"]["plan_shift_m"];
    let cloud_shift = (ps[0].as_f64().unwrap(), ps[1].as_f64().unwrap());
    let mut digest = Sha256::new();
    let mut pts: Vec<(f32, f32, f32, u8, bool)> = vec![];
    let mut class_count = [0u64; 256];
    for f in lman["files"].as_array().unwrap() {
        let p = ldir.join(f["file"].as_str().unwrap());
        let sha = f["sha256"].as_str().unwrap();
        read_checked(&p, sha);
        digest.update(sha.as_bytes());
        let mut r = Reader::from_path(&p).expect("laz reader");
        let all = r.read_all().expect("laz points");
        let (xs, ys, zs) = (all.x().collect::<Vec<_>>(), all.y().collect::<Vec<_>>(), all.z().collect::<Vec<_>>());
        let (cl, rn, nr) = (all.classification().collect::<Vec<_>>(), all.return_number().collect::<Vec<_>>(), all.number_of_returns().collect::<Vec<_>>());
        for i in 0..xs.len() {
            if cl[i] == 7 || cl[i] == 18 {
                continue;
            }
            let lon = (xs[i] / R_MERC).to_degrees();
            let lat = (2.0 * (ys[i] / R_MERC).exp().atan() - std::f64::consts::FRAC_PI_2).to_degrees();
            let (x, z) = frame.to_plan(lon, lat);
            let (x, z) = (x + cloud_shift.0, z + cloud_shift.1);
            if !inside(x, z) {
                continue;
            }
            class_count[cl[i] as usize] += 1;
            pts.push((x as f32, z as f32, zs[i] as f32, cl[i], rn[i] < nr[i]));
        }
    }
    let cloud_digest = hex(&digest.finalize());
    println!("lidar: {} points in the area; classes {:?}; cloud shift {:?}", pts.len(), class_count.iter().enumerate().filter(|c| *c.1 > 0).map(|(k, c)| (k, *c)).collect::<Vec<_>>(), cloud_shift);
    // points by cell (counting sort)
    let mut start = vec![0u32; cells + 1];
    for p in &pts {
        if let Some(k) = grid.idx(p.0 as f64, p.1 as f64) {
            start[k + 1] += 1;
        }
    }
    for k in 0..cells {
        start[k + 1] += start[k];
    }
    let mut order = vec![0u32; start[cells] as usize];
    let mut fill = start.clone();
    for (i, p) in pts.iter().enumerate() {
        if let Some(k) = grid.idx(p.0 as f64, p.1 as f64) {
            order[fill[k] as usize] = i as u32;
            fill[k] += 1;
        }
    }
    let cell_pts = |k: usize| order[start[k] as usize..start[k + 1] as usize].iter().map(|&i| &pts[i as usize]);

    // ---- ground and surface
    let mut ground = vec![f64::INFINITY; cells];
    let mut surface = vec![f64::NEG_INFINITY; cells];
    let mut hits = vec![0u32; cells];
    let mut through = vec![0u32; cells];
    for k in 0..cells {
        for p in cell_pts(k) {
            if p.3 == 2 {
                ground[k] = ground[k].min(p.2 as f64);
            }
            surface[k] = surface[k].max(p.2 as f64);
            hits[k] += 1;
            if p.4 {
                through[k] += 1;
            }
        }
    }
    let measured_ground: Vec<bool> = ground.iter().map(|g| g.is_finite()).collect();
    let mut queue: VecDeque<usize> = (0..cells).filter(|&k| ground[k].is_finite()).collect();
    while let Some(k) = queue.pop_front() {
        let (i, j) = ((k % n) as i64, (k / n) as i64);
        for (di, dj) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            let (a, b) = (i + di, j + dj);
            if a >= 0 && b >= 0 && a < n as i64 && b < n as i64 {
                let q = b as usize * n + a as usize;
                if !ground[q].is_finite() {
                    ground[q] = ground[k];
                    queue.push_back(q);
                }
            }
        }
    }
    let ndsm: Vec<f64> = (0..cells).map(|k| if surface[k].is_finite() { (surface[k] - ground[k]).max(0.0) } else { 0.0 }).collect();
    let ground_at = |x: f64, z: f64| grid.idx(x, z).map(|k| ground[k]).unwrap_or(f64::NAN);

    // ---- lot lines: the stretches two parcels share
    let mut lines: Vec<LotLine> = vec![];
    for (ai, a) in parcels.iter().enumerate() {
        let m = a.ring.len();
        for e in 0..m {
            let (p, q) = (a.ring[e], a.ring[(e + 1) % m]);
            let len = (q[0] - p[0]).hypot(q[1] - p[1]);
            if len < 0.05 {
                continue;
            }
            let steps = (len / 0.25).ceil() as usize;
            // per sample: the neighbour sharing it (index), if any
            let mut owner: Vec<Option<usize>> = Vec::with_capacity(steps);
            for s in 0..steps {
                let t = (s as f64 + 0.5) / steps as f64;
                let (x, z) = (p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t);
                let mut best: Option<(usize, f64)> = None;
                for (bi, b) in parcels.iter().enumerate() {
                    if bi == ai || b.id == a.id || x < b.bb[0] - SHARED_M || x > b.bb[2] + SHARED_M || z < b.bb[1] - SHARED_M || z > b.bb[3] + SHARED_M {
                        continue;
                    }
                    let d = dist_to_ring(x, z, &b.ring);
                    if d < SHARED_M && best.is_none_or(|v| d < v.1) {
                        best = Some((bi, d));
                    }
                }
                owner.push(best.map(|v| v.0));
            }
            // runs of one neighbour; each pair once (the lower id keeps it)
            let mut s = 0;
            while s < steps {
                let Some(bi) = owner[s] else {
                    s += 1;
                    continue;
                };
                let mut t = s;
                while t + 1 < steps && owner[t + 1] == Some(bi) {
                    t += 1;
                }
                let b = &parcels[bi];
                let (t0, t1) = (s as f64 / steps as f64, (t + 1) as f64 / steps as f64);
                if a.id < b.id && (t1 - t0) * len >= MIN_LINE_M {
                    let at = |f: f64| [p[0] + (q[0] - p[0]) * f, p[1] + (q[1] - p[1]) * f];
                    lines.push(LotLine { a: a.id.clone(), b: b.id.clone(), p: at(t0), q: at(t1) });
                }
                s = t + 1;
            }
        }
    }
    let lines: Vec<LotLine> = lines.into_iter().filter(|l| inside(l.p[0], l.p[1]) && inside(l.q[0], l.q[1])).collect();
    let line_m: f64 = lines.iter().map(|l| (l.q[0] - l.p[0]).hypot(l.q[1] - l.p[1])).sum();
    println!("lot lines: {} shared stretches, {:.0} m", lines.len(), line_m);

    // ---- stations and their profiles
    let mut stations: Vec<Vec<Station>> = vec![];
    for l in &lines {
        let len = (l.q[0] - l.p[0]).hypot(l.q[1] - l.p[1]);
        let t = [(l.q[0] - l.p[0]) / len, (l.q[1] - l.p[1]) / len];
        let nrm = [-t[1], t[0]];
        let count = (len / STATION_M).floor().max(1.0) as usize;
        let mut sts = vec![];
        for s in 0..count {
            let f = (s as f64 + 0.5) * len / count as f64;
            let c = [l.p[0] + t[0] * f, l.p[1] + t[1] * f];
            let yard = !near_footprint(c[0], c[1]);
            let mut top = vec![f64::NEG_INFINITY; BINS];
            let mut high = vec![false; BINS];
            let mut gr = vec![f64::NAN; BINS];
            for (k, g) in gr.iter_mut().enumerate() {
                let u = -PROFILE_HALF_M + (k as f64 + 0.5) * PROFILE_BIN_M;
                *g = ground_at(c[0] + nrm[0] * u, c[1] + nrm[1] * u);
            }
            let r = PROFILE_HALF_M + STATION_M;
            let (i0, j0) = (((c[0] - r - grid.x0) / CELL).floor().max(0.0) as usize, ((c[1] - r - grid.z0) / CELL).floor().max(0.0) as usize);
            let (i1, j1) = ((((c[0] + r - grid.x0) / CELL).ceil().max(0.0) as usize).min(n - 1), (((c[1] + r - grid.z0) / CELL).ceil().max(0.0) as usize).min(n - 1));
            for j in j0..=j1 {
                for i in i0..=i1 {
                    for p in cell_pts(j * n + i) {
                        let (dx, dz) = (p.0 as f64 - c[0], p.1 as f64 - c[1]);
                        let along = dx * t[0] + dz * t[1];
                        let u = dx * nrm[0] + dz * nrm[1];
                        if along.abs() > STATION_M / 2.0 || u.abs() >= PROFILE_HALF_M {
                            continue;
                        }
                        let b = Station::bin_of(u) as usize;
                        // the canopy and eaves above a fence's reach are left out: a fence under a tree still shows.
                        if (p.2 as f64) - gr[b] <= PROFILE_CEILING_M {
                            top[b] = top[b].max(p.2 as f64);
                        } else {
                            high[b] = true;
                        }
                    }
                }
            }
            let solid = (0..BINS).map(|k| high[k] && !top[k].is_finite()).collect();
            sts.push(Station { c, nrm, yard, top, solid, ground: gr });
        }
        stations.push(sts);
    }
    let yard_stations: usize = stations.iter().flatten().filter(|s| s.yard).count();
    println!("stations: {} in yards of {}", yard_stations, stations.iter().map(|s| s.len()).sum::<usize>());

    // ---- the parcel map's shift against the cloud: the most yard stations on a fence
    let offs: Vec<f64> = (-((SHIFT_MAX_M + LINE_SLACK_M) / PROFILE_BIN_M).round() as i64..=((SHIFT_MAX_M + LINE_SLACK_M) / PROFILE_BIN_M).round() as i64).map(|k| k as f64 * PROFILE_BIN_M).collect();
    let pass: Vec<Vec<Vec<Option<f64>>>> = stations.iter().map(|sts| sts.iter().map(|s| if s.yard { offs.iter().map(|&u| s.fence_at(u)).collect() } else { vec![None; offs.len()] }).collect()).collect();
    let off_index = |u: f64| ((u - offs[0]) / PROFILE_BIN_M).round().clamp(0.0, (offs.len() - 1) as f64) as usize;
    let steps = (SHIFT_MAX_M / PROFILE_BIN_M).round() as i64;
    let mut best = (0usize, (0.0, 0.0));
    for sj in -steps..=steps {
        for si in -steps..=steps {
            let sh = (si as f64 * PROFILE_BIN_M, sj as f64 * PROFILE_BIN_M);
            let mut score = 0;
            for (li, sts) in stations.iter().enumerate() {
                for (k, s) in sts.iter().enumerate() {
                    if pass[li][k][off_index(s.nrm[0] * sh.0 + s.nrm[1] * sh.1)].is_some() {
                        score += 1;
                    }
                }
            }
            if score > best.0 {
                best = (score, sh);
            }
        }
    }
    let shift = best.1;
    let zero_score: usize = stations.iter().enumerate().map(|(li, sts)| sts.iter().enumerate().filter(|(k, _)| pass[li][*k][off_index(0.0)].is_some()).count()).sum();
    println!("parcel map shift onto the cloud: ({:+.2}, {:+.2}) m east/south: {} yard stations on a fence (unshifted {})", shift.0, shift.1, best.0, zero_score);

    // ---- per line: its own offset, its fenced spans
    let mut out_lines = vec![];
    let mut fences = vec![];
    let mut heights: Vec<f64> = vec![];
    let (mut fenced_lines, mut yard_lines, mut fence_m, mut yard_m, mut passing_m) = (0usize, 0usize, 0.0f64, 0.0f64, 0.0f64);
    for (li, l) in lines.iter().enumerate() {
        let sts = &stations[li];
        let ug = sts[0].nrm[0] * shift.0 + sts[0].nrm[1] * shift.1;
        let slack = (LINE_SLACK_M / PROFILE_BIN_M).round() as i64;
        let (mut bu, mut bc) = (ug, 0usize);
        for d in -slack..=slack {
            let u = ug + d as f64 * PROFILE_BIN_M;
            let c = (0..sts.len()).filter(|&k| pass[li][k][off_index(u)].is_some()).count();
            if c > bc || (c == bc && (u - ug).abs() < (bu - ug).abs()) {
                bu = u;
                bc = c;
            }
        }
        let oi = off_index(bu);
        let ys = sts.iter().filter(|s| s.yard).count();
        if ys > 0 {
            yard_lines += 1;
            yard_m += ys as f64 * STATION_M;
        }
        // yard runs (stations in a yard, gaps of at most GAP_STATIONS out of one bridged); a run
        // the lidar confirms (at least MIN_SPAN_STATIONS and CONFIRM_SHARE of its stations on a
        // fence) is fenced over its length, at the HEIGHT_QUANTILE of its passing stations' heights.
        let mut spans = vec![];
        let mut k = 0;
        while k < sts.len() {
            if !sts[k].yard {
                k += 1;
                continue;
            }
            let (s0, mut s1, mut gap) = (k, k, 0);
            let mut j = k + 1;
            while j < sts.len() {
                if sts[j].yard {
                    s1 = j;
                    gap = 0;
                } else {
                    gap += 1;
                    if gap > GAP_STATIONS {
                        break;
                    }
                }
                j += 1;
            }
            let run = (s0..=s1).filter(|&q| sts[q].yard).count();
            let mut hs: Vec<f64> = (s0..=s1).filter(|&q| sts[q].yard).filter_map(|q| pass[li][q][oi]).collect();
            passing_m += hs.len() as f64 * STATION_M;
            if hs.len() >= MIN_SPAN_STATIONS && hs.len() as f64 >= CONFIRM_SHARE * run as f64 {
                let h = quantile(&mut hs, HEIGHT_QUANTILE);
                let at = |q: usize, f: f64| {
                    let s = &sts[q];
                    let len = (l.q[0] - l.p[0]).hypot(l.q[1] - l.p[1]);
                    let t = [(l.q[0] - l.p[0]) / len, (l.q[1] - l.p[1]) / len];
                    [s.c[0] + s.nrm[0] * bu + t[0] * f, s.c[1] + s.nrm[1] * bu + t[1] * f]
                };
                let (a, b) = (at(s0, -STATION_M / 2.0), at(s1, STATION_M / 2.0));
                let len = (b[0] - a[0]).hypot(b[1] - a[1]);
                fence_m += len;
                heights.push(h);
                spans.push(serde_json::json!({ "a": [r2(a[0]), r2(a[1])], "b": [r2(b[0]), r2(b[1])], "height_m": r2(h), "length_m": r2(len), "yard_stations": run, "passing": hs.len() }));
                fences.push((a, b, h));
            }
            k = s1 + 1;
        }
        if !spans.is_empty() {
            fenced_lines += 1;
        }
        out_lines.push(serde_json::json!({ "parcels": [l.a, l.b], "p": [r2(l.p[0]), r2(l.p[1])], "q": [r2(l.q[0]), r2(l.q[1])], "offset_m": r2(bu), "yard_stations": ys, "passing": bc, "fenced": !spans.is_empty(), "spans": spans }));
    }
    let mut hs = heights.clone();
    let hq = if hs.is_empty() { [0.0; 5] } else { [0.0, 0.1, 0.5, 0.9, 1.0].map(|q| quantile(&mut hs, q)) };
    println!(
        "fences: {fenced_lines} of {yard_lines} lot lines with a yard stretch have a fence; {passing_m:.0} m of stations on a fence; {} spans drawn, {fence_m:.0} m of {yard_m:.0} m of yard lot line; heights min {:.2} p10 {:.2} median {:.2} p90 {:.2} max {:.2} m",
        heights.len(),
        hq[0],
        hq[1],
        hq[2],
        hq[3],
        hq[4]
    );

    if std::env::var_os("EO_YARD_DEBUG").is_some() {
        let mut why = [0usize; 7];
        let mut hist = [0usize; 12];
        for (li, sts) in stations.iter().enumerate() {
            let u = out_lines[li]["offset_m"].as_f64().unwrap();
            for s in sts.iter().filter(|s| s.yard) {
                match s.fence_test(u) {
                    Ok(h) => {
                        why[6] += 1;
                        hist[((h / 0.5) as usize).min(11)] += 1;
                    }
                    Err((r, h)) => {
                        why[r as usize] += 1;
                        if r >= 2 {
                            hist[((h.max(0.0) / 0.5) as usize).min(11)] += 1;
                        }
                    }
                }
            }
        }
        println!("debug: yard stations at their line's offset: no return {}, no ground {}, too low {}, too high {}, too wide {}, both sides raised {}, pass {}", why[0], why[1], why[2], why[3], why[4], why[5], why[6]);
        println!("debug: top over ground, 0.5 m bins: {hist:?}");
        let mut shown = 0;
        for (li, sts) in stations.iter().enumerate() {
            let u = out_lines[li]["offset_m"].as_f64().unwrap();
            for s in sts.iter().filter(|s| s.yard) {
                if shown < 12 && (li * 7 + shown) % 23 == 0 {
                    let prof: Vec<String> = (0..BINS).step_by(2).map(|k| if s.top[k].is_finite() { format!("{:.1}", s.top[k] - s.ground[k]) } else { "-".into() }).collect();
                    println!("debug: line {li} u {u:+.1} {:?}: {}", s.fence_test(u), prof.join(" "));
                    shown += 1;
                }
            }
        }
    }

    // ---- ground classes
    let veg: Vec<bool> = (0..cells).map(|k| hits[k] > 0 && through[k] as f64 >= VEG_SHARE * hits[k] as f64 && ndsm[k] >= VEG_MIN_M).collect();
    let raised: Vec<bool> = (0..cells).map(|k| !veg[k] && ndsm[k] >= DECK_MIN_M).collect();
    // opening (3 x 3 erosion then dilation): a fence's line is not a structure or a deck
    let erode = |m: &[bool]| -> Vec<bool> {
        (0..cells).map(|k| {
            let (i, j) = ((k % n) as i64, (k / n) as i64);
            m[k] && (-1..=1).all(|dj| (-1..=1).all(|di| {
                let (a, b) = (i + di, j + dj);
                a < 0 || b < 0 || a >= n as i64 || b >= n as i64 || m[b as usize * n + a as usize]
            }))
        }).collect()
    };
    let dilate = |m: &[bool]| -> Vec<bool> {
        (0..cells).map(|k| {
            let (i, j) = ((k % n) as i64, (k / n) as i64);
            (-1..=1).any(|dj| (-1..=1).any(|di| {
                let (a, b) = (i + di, j + dj);
                a >= 0 && b >= 0 && a < n as i64 && b < n as i64 && m[b as usize * n + a as usize]
            }))
        }).collect()
    };
    let opened = dilate(&erode(&raised));
    let raised_open: Vec<bool> = (0..cells).map(|k| opened[k] && raised[k]).collect();
    // flatness: the 3 x 3 range of the surface
    let flat: Vec<bool> = (0..cells).map(|k| {
        let (i, j) = ((k % n) as i64, (k / n) as i64);
        let (mut lo, mut hi) = (f64::MAX, f64::MIN);
        for dj in -1..=1 {
            for di in -1..=1 {
                let (a, b) = (i + di, j + dj);
                if a >= 0 && b >= 0 && a < n as i64 && b < n as i64 {
                    let s = surface[b as usize * n + a as usize];
                    if s.is_finite() {
                        lo = lo.min(s);
                        hi = hi.max(s);
                    }
                }
            }
        }
        hi - lo < DECK_FLAT_M
    }).collect();
    // raised regions: decks (flat, low, small) and structures
    let mut label = vec![u32::MAX; cells];
    let mut regions: Vec<Vec<usize>> = vec![];
    for k0 in 0..cells {
        if !raised_open[k0] || label[k0] != u32::MAX {
            continue;
        }
        let id = regions.len() as u32;
        let mut members = vec![];
        let mut q = VecDeque::from([k0]);
        label[k0] = id;
        while let Some(k) = q.pop_front() {
            members.push(k);
            let (i, j) = ((k % n) as i64, (k / n) as i64);
            for (di, dj) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                let (a, b) = (i + di, j + dj);
                if a >= 0 && b >= 0 && a < n as i64 && b < n as i64 {
                    let m = b as usize * n + a as usize;
                    if raised_open[m] && label[m] == u32::MAX {
                        label[m] = id;
                        q.push_back(m);
                    }
                }
            }
        }
        regions.push(members);
    }
    let mut structure = vec![false; cells];
    let mut deck = vec![false; cells];
    for r in &regions {
        let area = r.len() as f64 * CELL * CELL;
        let tall = r.iter().filter(|&&k| ndsm[k] >= STRUCTURE_M).count() * 2 > r.len();
        let flat_share = r.iter().filter(|&&k| flat[k]).count() as f64 / r.len() as f64;
        let is_deck = !tall && (DECK_MIN_M2..=DECK_MAX_M2).contains(&area) && flat_share >= 0.6;
        for &k in r {
            if is_deck {
                deck[k] = true;
            } else if area > DECK_MAX_M2 || tall || ndsm[k] >= STRUCTURE_M {
                structure[k] = true;
            }
        }
    }
    let yard: Vec<bool> = (0..cells).map(|k| parcel_of[k] != u32::MAX && !fp_mask[k] && !structure[k]).collect();

    // the aerial, and its shift onto the plan by the vegetation
    let (lon0, lat1) = frame.to_lonlat(grid.x0 - 5.0, grid.z0 - 5.0);
    let (lon1, lat0) = frame.to_lonlat(grid.x0 + n as f64 * CELL + 5.0, grid.z0 + n as f64 * CELL + 5.0);
    let mut aerial = Aerial::load(&adir, lon0, lat0, lon1, lat1);
    // the lidar's vegetation (2 m up) vs the aerial's green over the parcels' open cells (every 2nd
    // cell): the tiles' latitude mapping (as recorded, or square pixels), then a shift within SHIFT_MAX_M.
    let probe: Vec<usize> = (0..cells).filter(|&k| (k % n) % 2 == 0 && (k / n) % 2 == 0 && parcel_of[k] != u32::MAX && !structure[k]).collect();
    let veg_iou = |aerial: &Aerial, sh: (f64, f64), t: f64| {
        let (mut inter, mut uni) = (0usize, 0usize);
        for &k in &probe {
            let (x, z) = grid.centre(k);
            let (lo, la) = frame.to_lonlat(x - sh.0, z - sh.1);
            let Some(c) = aerial.at(lo, la) else { continue };
            let (g, v) = (exg(c) > t, veg[k] && ndsm[k] >= 2.0);
            inter += (g && v) as usize;
            uni += (g || v) as usize;
        }
        inter as f64 / uni.max(1) as f64
    };
    let green_t = |aerial: &Aerial| {
        let g: Vec<f64> = probe.iter().filter_map(|&k| {
            let (x, z) = grid.centre(k);
            let (lo, la) = frame.to_lonlat(x, z);
            aerial.at(lo, la).map(exg)
        }).collect();
        otsu(&g)
    };
    let mut mapping_iou = [0.0; 2];
    for (i, sq) in [false, true].into_iter().enumerate() {
        aerial.square = sq;
        mapping_iou[i] = veg_iou(&aerial, (0.0, 0.0), green_t(&aerial));
    }
    aerial.square = mapping_iou[1] > mapping_iou[0];
    println!("aerial tiles' latitude: as recorded vegetation IoU {:.3}, square pixels {:.3}: {}", mapping_iou[0], mapping_iou[1], if aerial.square { "square pixels" } else { "as recorded" });
    let t_green0 = green_t(&aerial);
    let ashift_steps = (SHIFT_MAX_M / 0.3).round() as i64;
    let mut abest = (f64::MIN, (0.0, 0.0));
    for sj in -ashift_steps..=ashift_steps {
        for si in -ashift_steps..=ashift_steps {
            let sh = (si as f64 * 0.3, sj as f64 * 0.3);
            let iou = veg_iou(&aerial, sh, t_green0);
            if iou > abest.0 {
                abest = (iou, sh);
            }
        }
    }
    let ashift = abest.1;
    println!("aerial shift onto the plan: ({:+.1}, {:+.1}) m east/south, vegetation IoU {:.3}; haze {:?}", ashift.0, ashift.1, abest.0, aerial.haze);
    let sample = |k: usize, sh: (f64, f64)| {
        let (x, z) = grid.centre(k);
        let (lo, la) = frame.to_lonlat(x - sh.0, z - sh.1);
        aerial.at(lo, la)
    };
    let rgb: Vec<Option<[f64; 3]>> = (0..cells).map(|k| sample(k, ashift)).collect();
    let lum: Vec<f64> = rgb.iter().map(|c| c.map(luma).unwrap_or(0.0)).collect();
    let roughness: Vec<f64> = (0..cells).map(|k| {
        let (i, j) = ((k % n) as i64, (k / n) as i64);
        let (mut lo, mut hi) = (f64::MAX, f64::MIN);
        for dj in -1..=1 {
            for di in -1..=1 {
                let (a, b) = (i + di, j + dj);
                if a >= 0 && b >= 0 && a < n as i64 && b < n as i64 {
                    let l = lum[b as usize * n + a as usize];
                    lo = lo.min(l);
                    hi = hi.max(l);
                }
            }
        }
        (hi - lo) / lum[k].max(0.02)
    }).collect();
    let sunlit = |k: usize| lum[k] >= SHADOW_LUMA;
    let yard_sun: Vec<usize> = (0..cells).filter(|&k| yard[k] && !deck[k] && !veg[k] && rgb[k].is_some() && sunlit(k)).collect();
    let t_green = otsu(&yard_sun.iter().map(|&k| exg(rgb[k].unwrap())).collect::<Vec<_>>());
    let greens: Vec<usize> = yard_sun.iter().copied().filter(|&k| exg(rgb[k].unwrap()) > t_green).collect();
    let mut gl: Vec<f64> = greens.iter().map(|&k| lum[k]).collect();
    let mut gr: Vec<f64> = greens.iter().map(|&k| roughness[k]).collect();
    let (l_med, r_med) = if greens.is_empty() { (0.0, 0.0) } else { (quantile(&mut gl, 0.5), quantile(&mut gr, 0.5)) };
    println!("aerial green: excess green over {t_green:.3} (Otsu over {} sunlit yard cells), the green cells' median luma {l_med:.3} and roughness {r_med:.3}", yard_sun.len());
    let mut class = vec![NONE; cells];
    for k in 0..cells {
        if !yard[k] {
            continue;
        }
        class[k] = if deck[k] {
            DECK
        } else if veg[k] {
            PLANTING
        } else if let Some(c) = rgb[k].filter(|_| sunlit(k)) {
            let g = exg(c);
            let sat = {
                let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
                (mx - mn) / mx.max(1e-6)
            };
            if g > t_green {
                if lum[k] >= l_med * 0.8 && roughness[k] <= r_med * 1.25 { LAWN } else { PLANTING }
            } else if c[0] > c[1] && c[1] > c[2] && sat > 0.25 && lum[k] < l_med * 1.5 {
                PLANTING // bare soil and mulch: warm, not green
            } else {
                PAVING
            }
        } else {
            255 // deep shadow: from the nearest sunlit cell of the parcel
        };
    }
    // shadow cells: nearest decided cell of the same parcel (BFS), else paving
    let reach = (SHADOW_REACH_M / CELL).round() as usize;
    let mut dist = vec![usize::MAX; cells];
    let mut q: VecDeque<usize> = (0..cells).filter(|&k| class[k] != NONE && class[k] != 255).collect();
    for &k in &q {
        dist[k] = 0;
    }
    let mut resolved = class.clone();
    while let Some(k) = q.pop_front() {
        if dist[k] >= reach {
            continue;
        }
        let (i, j) = ((k % n) as i64, (k / n) as i64);
        for (di, dj) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            let (a, b) = (i + di, j + dj);
            if a >= 0 && b >= 0 && a < n as i64 && b < n as i64 {
                let m = b as usize * n + a as usize;
                if class[m] == 255 && dist[m] == usize::MAX && parcel_of[m] == parcel_of[k] {
                    dist[m] = dist[k] + 1;
                    resolved[m] = resolved[k];
                    q.push_back(m);
                }
            }
        }
    }
    let shadow_cells = class.iter().filter(|&&c| c == 255).count();
    for c in resolved.iter_mut() {
        if *c == 255 {
            *c = PAVING;
        }
    }
    // the speckle of single cells smoothed: MODE_PASSES of a 3 x 3 majority over the yard cells.
    let mut class = resolved;
    for _ in 0..MODE_PASSES {
        let prev = class.clone();
        for k in 0..cells {
            if prev[k] == NONE {
                continue;
            }
            let (i, j) = ((k % n) as i64, (k / n) as i64);
            let mut votes = [0u8; 5];
            for dj in -1..=1 {
                for di in -1..=1 {
                    let (a, b) = (i + di, j + dj);
                    if a >= 0 && b >= 0 && a < n as i64 && b < n as i64 {
                        let c = prev[b as usize * n + a as usize];
                        if c != NONE {
                            votes[c as usize] += 1;
                        }
                    }
                }
            }
            let best = (1..5).max_by_key(|&c| (votes[c], c == prev[k] as usize)).unwrap();
            if votes[best] > votes[prev[k] as usize] {
                class[k] = best as u8;
            }
        }
    }
    // (the ground file keeps the classes as classified; the canopy file carries them squared.)
    let class_ground = class.clone();
    // (the canopy file's classes: none on a park-size lot, YARD_LOT_MAX_M2.)
    {
        let area = |r: &[[f64; 2]]| 0.5 * r.iter().zip(r.iter().cycle().skip(1)).map(|(a, b)| a[0] * b[1] - b[0] * a[1]).sum::<f64>().abs();
        let big: Vec<bool> = parcels.iter().map(|p| area(&p.ring) > YARD_LOT_MAX_M2).collect();
        let mut cut = 0usize;
        for k in 0..cells {
            if parcel_of[k] != u32::MAX && big[parcel_of[k] as usize] && class[k] != NONE {
                class[k] = NONE;
                cut += 1;
            }
        }
        println!("yards: {} park-size lots (over {YARD_LOT_MAX_M2} m^2) left out of the canopy file's classes, {:.0} m^2", big.iter().filter(|b| **b).count(), cut as f64 * CELL * CELL);
    }
    // paving in rectangles (PAVE_RECT_*): the rectangle's yard cells of the region's parcel that
    // are not vegetation, lawn or a deck become paving (a lawn the aerial sees is a lawn);
    // the region's cells outside it take their non-paving neighbours' most common class
    // (planting if none).
    let mut paved_regions = (0usize, 0usize);
    {
        let mut seen = vec![false; cells];
        for k0 in 0..cells {
            if class[k0] != PAVING || seen[k0] {
                continue;
            }
            let mut members = vec![k0];
            seen[k0] = true;
            let mut q = VecDeque::from([k0]);
            while let Some(k) = q.pop_front() {
                let (i, j) = ((k % n) as i64, (k / n) as i64);
                for (di, dj) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                    let (a, b) = (i + di, j + dj);
                    if a >= 0 && b >= 0 && a < n as i64 && b < n as i64 {
                        let m = b as usize * n + a as usize;
                        if !seen[m] && class[m] == PAVING && parcel_of[m] == parcel_of[k0] {
                            seen[m] = true;
                            members.push(m);
                            q.push_back(m);
                        }
                    }
                }
            }
            paved_regions.0 += 1;
            let area = members.len() as f64 * CELL * CELL;
            if area < PAVE_RECT_MIN_M2 {
                continue;
            }
            let pts: Vec<(f64, f64)> = members.iter().map(|&k| grid.centre(k)).collect();
            // the region's rectangles (the least-area one over the cells' footprints, or its parts').
            let rects = pave_rects(&pts, PAVE_RECT_SPLITS);
            if rects.is_empty() {
                continue;
            }
            paved_regions.1 += 1;
            let parcel = parcel_of[members[0]];
            let inside = |k: usize| {
                let (x, z) = grid.centre(k);
                rects.iter().any(|&(a, [u0, u1, v0, v1])| {
                    let (s, c) = a.sin_cos();
                    let (u, v) = (x * c + z * s, -x * s + z * c);
                    u >= u0 && u <= u1 && v >= v0 && v <= v1
                })
            };
            let bb = pts.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, &(x, z)| [b[0].min(x), b[1].min(z), b[2].max(x), b[3].max(z)]);
            let span = ((bb[2] - bb[0]).hypot(bb[3] - bb[1]) / CELL).ceil() as i64 + 2;
            let (ci, cj) = (((0.5 * (bb[0] + bb[2]) - grid.x0) / CELL) as i64, ((0.5 * (bb[1] + bb[3]) - grid.z0) / CELL) as i64);
            for j in (cj - span).max(0)..(cj + span).min(n as i64) {
                for i in (ci - span).max(0)..(ci + span).min(n as i64) {
                    let k = j as usize * n + i as usize;
                    if inside(k) && yard[k] && parcel_of[k] == parcel && !veg[k] && class[k] != DECK && class[k] != LAWN && class[k] != NONE {
                        class[k] = PAVING;
                    }
                }
            }
            for &k in &members {
                if inside(k) {
                    continue;
                }
                let (i, j) = ((k % n) as i64, (k / n) as i64);
                let mut votes = [0u32; 5];
                for (di, dj) in [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, 1), (-1, 1), (1, -1)] {
                    let (a, b) = (i + di, j + dj);
                    if a >= 0 && b >= 0 && a < n as i64 && b < n as i64 {
                        let c = class[b as usize * n + a as usize];
                        if c != NONE && c != PAVING {
                            votes[c as usize] += 1;
                        }
                    }
                }
                let pick = (1..5).max_by_key(|&c| votes[c]).filter(|&c| votes[c] > 0).unwrap_or(PLANTING as usize);
                class[k] = pick as u8;
            }
        }
    }
    println!("paving: {} of {} regions laid as rectangles (at least {PAVE_RECT_MIN_M2} m^2, filling {PAVE_RECT_FILL} of each, cut up to {PAVE_RECT_SPLITS} times)", paved_regions.1, paved_regions.0);
    // tones: the median sunlit colour per class (linear), and areas
    let mut tones = serde_json::Map::new();
    let mut areas = serde_json::Map::new();
    for (ci, cname) in CLASS_NAMES.iter().enumerate().skip(1) {
        let ks: Vec<usize> = (0..cells).filter(|&k| class[k] == ci as u8).collect();
        areas.insert(cname.to_string(), serde_json::json!(r2(ks.len() as f64 * CELL * CELL)));
        let lit: Vec<[f64; 3]> = ks.iter().filter(|&&k| sunlit(k)).filter_map(|&k| rgb[k]).collect();
        if !lit.is_empty() {
            let med = [0, 1, 2].map(|c| {
                let mut v: Vec<f64> = lit.iter().map(|p| p[c]).collect();
                (quantile(&mut v, 0.5) * 10000.0).round() / 10000.0
            });
            let srgb = med.map(|v| (eo_data::roof_colours::linear_to_srgb(v as f32) * 255.0).round() as u8);
            tones.insert(cname.to_string(), serde_json::json!({ "linear": med, "srgb8": srgb, "cells": lit.len() }));
        }
    }
    println!("ground classes (m^2): {}; {} deep-shadow cells resolved from their parcel's sunlit cells", serde_json::Value::Object(areas.clone()), shadow_cells);

    // ---- write
    let provenance = serde_json::json!({
        "generator": "native/eo-kit/examples/yard_survey.rs",
        "plan": "surveyed 1:1 plan metres (x east, z south), the footprints' frame; the game draws it times groundscale::scale()",
        "parcels": { "source": pman["source"], "dataset": pman["dataset"], "url": pman["url"], "license": pman["license"], "attribution": pman["attribution"], "fetched": pman["fetched"], "sha256": pfile["sha256"], "records": records, "distinct": seen.len() },
        "lidar": { "source": lman["source"], "url": lman["url"], "license": lman["license"], "attribution": lman["attribution"], "cloud_sha256_of_file_digests": cloud_digest, "cloud_shift_m": [cloud_shift.0, cloud_shift.1] },
        "aerial": { "source": "USDA NAIP natural colour 0.6 m via the USGS National Map (public domain)", "files": aerial.files, "haze_linear": aerial.haze, "tile_latitude": if aerial.square { "square pixels (the served box)" } else { "as recorded" }, "mapping_veg_iou": { "as_recorded": mapping_iou[0], "square_pixels": mapping_iou[1] }, "shift_m": [ashift.0, ashift.1], "veg_iou": abest.0 },
        "parcel_shift_m": [shift.0, shift.1],
        "method": {
            "cell_m": CELL, "shared_m": SHARED_M, "station_m": STATION_M, "clear_of_footprint_m": CLEAR_OF_FOOTPRINT_M,
            "fence_m": [FENCE_MIN_M, FENCE_MAX_M], "fence_width_m": FENCE_WIDTH_M, "fence_drop_m": FENCE_DROP_M, "side_m": [SIDE_NEAR_M, SIDE_FAR_M],
            "top_band_m": TOP_BAND_M, "shift_max_m": SHIFT_MAX_M, "line_slack_m": LINE_SLACK_M, "gap_stations": GAP_STATIONS, "min_span_stations": MIN_SPAN_STATIONS, "confirm_share": CONFIRM_SHARE, "height_quantile": HEIGHT_QUANTILE, "profile_ceiling_m": PROFILE_CEILING_M,
            "veg_share": VEG_SHARE, "veg_min_m": VEG_MIN_M, "structure_m": STRUCTURE_M, "deck_min_m": DECK_MIN_M, "deck_flat_m": DECK_FLAT_M, "deck_m2": [DECK_MIN_M2, DECK_MAX_M2],
            "shadow_luma": SHADOW_LUMA, "shadow_reach_m": SHADOW_REACH_M, "mode_passes": MODE_PASSES, "green_exg_otsu": t_green, "green_luma_median": l_med, "green_roughness_median": r_med,
        },
    });
    let stats = serde_json::json!({
        "parcels": seen.len(), "lot_lines": lines.len(), "lot_line_m": r2(line_m), "yard_lot_lines": yard_lines, "yard_lot_line_m": r2(yard_m),
        "fenced_lot_lines": fenced_lines, "fence_spans": heights.len(), "fence_m": r2(fence_m), "stations_on_fence_m": r2(passing_m),
        "fence_height_m": { "min": r2(hq[0]), "p10": r2(hq[1]), "median": r2(hq[2]), "p90": r2(hq[3]), "max": r2(hq[4]) },
        "ground_m2": areas, "shadow_cells": shadow_cells,
    });
    let out = serde_json::json!({ "provenance": provenance, "area": lman["area"], "stats": stats, "tones": tones, "lot_lines": out_lines });
    std::fs::write(pdir.join("yards.json"), serde_json::to_string_pretty(&out).unwrap()).unwrap();
    let mut bin = Vec::with_capacity(28 + cells);
    bin.extend_from_slice(YARD_GROUND_MAGIC);
    bin.extend_from_slice(&(n as u32).to_le_bytes());
    bin.extend_from_slice(&(n as u32).to_le_bytes());
    bin.extend_from_slice(&(grid.x0 as f32).to_le_bytes());
    bin.extend_from_slice(&(grid.z0 as f32).to_le_bytes());
    bin.extend_from_slice(&(CELL as f32).to_le_bytes());
    bin.extend_from_slice(&class_ground);
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    std::io::Write::write_all(&mut gz, &bin).unwrap();
    std::fs::write(pdir.join("yard-ground.bin.gz"), gz.finish().unwrap()).unwrap();

    // ---- the canopy: every yard cell's vegetation height and aerial colour (see the module note)
    let mut colour: Vec<Option<[f64; 3]>> = (0..cells).map(|k| if class[k] != NONE && sunlit(k) { rgb[k] } else { None }).collect();
    let mut q: VecDeque<usize> = (0..cells).filter(|&k| colour[k].is_some()).collect();
    let mut dist = vec![usize::MAX; cells];
    for &k in &q {
        dist[k] = 0;
    }
    while let Some(k) = q.pop_front() {
        if dist[k] >= reach {
            continue;
        }
        let (i, j) = ((k % n) as i64, (k / n) as i64);
        for (di, dj) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            let (a, b) = (i + di, j + dj);
            if a >= 0 && b >= 0 && a < n as i64 && b < n as i64 {
                let m = b as usize * n + a as usize;
                if class[m] == class[k] && colour[m].is_none() && parcel_of[m] == parcel_of[k] {
                    dist[m] = dist[k] + 1;
                    colour[m] = colour[k];
                    q.push_back(m);
                }
            }
        }
    }
    let class_tone = |c: u8| tones.get(CLASS_NAMES[c as usize]).and_then(|t| t["linear"].as_array().map(|a| [0, 1, 2].map(|q| a[q].as_f64().unwrap_or(0.0)))).unwrap_or([0.0; 3]);
    let mut canopy = Vec::with_capacity(36 + cells * 5);
    canopy.extend_from_slice(YARD_CANOPY_MAGIC);
    canopy.extend_from_slice(&(n as u32).to_le_bytes());
    canopy.extend_from_slice(&(n as u32).to_le_bytes());
    canopy.extend_from_slice(&(grid.x0 as f32).to_le_bytes());
    canopy.extend_from_slice(&(grid.z0 as f32).to_le_bytes());
    canopy.extend_from_slice(&(CELL as f32).to_le_bytes());
    let mut canopy_m2 = 0.0;
    for k in 0..cells {
        if class[k] == NONE {
            canopy.extend_from_slice(&[0; 5]);
            continue;
        }
        let h = if veg[k] { (ndsm[k] * 10.0).round().clamp(0.0, 255.0) as u8 } else { 0 };
        if h > 0 {
            canopy_m2 += CELL * CELL;
        }
        let c = colour[k].unwrap_or_else(|| class_tone(class[k]));
        canopy.push(h);
        canopy.extend(c.map(|v| (eo_data::roof_colours::linear_to_srgb(v as f32).clamp(0.0, 1.0) * 255.0).round() as u8));
        canopy.push(class[k]);
    }
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    std::io::Write::write_all(&mut gz, &canopy).unwrap();
    std::fs::write(pdir.join("yard-canopy.bin.gz"), gz.finish().unwrap()).unwrap();
    println!("canopy: {canopy_m2:.0} m^2 of yard vegetation measured; colours from {} sunlit cells, filled within {SHADOW_REACH_M} m of their class and parcel", (0..cells).filter(|&k| class[k] != NONE && sunlit(k) && rgb[k].is_some()).count());

    // ---- evidence: the aerial with lot lines (grey), fences (red), footprints (cyan);
    // the height above ground; the classes.
    let mut img = vec![0u8; cells * 3];
    for k in 0..cells {
        let c = rgb[k].unwrap_or([0.0; 3]);
        for q in 0..3 {
            img[k * 3 + q] = (eo_data::roof_colours::linear_to_srgb((c[q] * 1.6) as f32).clamp(0.0, 1.0) * 255.0) as u8;
        }
        if fp_mask[k] && !fp_mask.get(k + 1).copied().unwrap_or(true) {
            img[k * 3..k * 3 + 3].copy_from_slice(&[0, 220, 220]);
        }
    }
    let plot = |img: &mut Vec<u8>, a: [f64; 2], b: [f64; 2], c: [u8; 3]| {
        let len = (b[0] - a[0]).hypot(b[1] - a[1]);
        let steps = (len / (CELL * 0.5)).ceil().max(1.0) as usize;
        for s in 0..=steps {
            let f = s as f64 / steps as f64;
            if let Some(k) = grid.idx(a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f) {
                img[k * 3..k * 3 + 3].copy_from_slice(&c);
            }
        }
    };
    for l in &lines {
        plot(&mut img, l.p, l.q, [150, 150, 150]);
    }
    for (a, b, _) in &fences {
        plot(&mut img, *a, *b, [255, 30, 30]);
    }
    write_png(&pdir.join("evidence-fences.png"), n, n, &img);
    if std::env::var_os("EO_YARD_DEBUG").is_some() {
        let mut dbg = img.clone();
        let cols = [[255u8, 0, 255], [255, 0, 255], [120, 120, 120], [60, 60, 255], [255, 150, 0], [255, 255, 0]];
        for (li, sts) in stations.iter().enumerate() {
            let u = out_lines[li]["offset_m"].as_f64().unwrap();
            for st in sts.iter().filter(|st| st.yard) {
                let c = match st.fence_test(u) { Ok(_) => [255, 0, 0], Err((r, _)) => cols[r as usize] };
                if let Some(k) = grid.idx(st.c[0] + st.nrm[0] * u, st.c[1] + st.nrm[1] * u) {
                    dbg[k * 3..k * 3 + 3].copy_from_slice(&c);
                }
            }
        }
        write_png(&pdir.join("debug-reasons.png"), n, n, &dbg);
    }
    let mut hgt = vec![0u8; cells * 3];
    for k in 0..cells {
        let h = ndsm[k];
        let v = (h / 4.0).clamp(0.0, 1.0);
        let c = if !measured_ground[k] && h < 0.1 { [40, 0, 40] } else if veg[k] { [0, (60.0 + 195.0 * v) as u8, 0] } else { [(255.0 * v) as u8; 3] };
        hgt[k * 3..k * 3 + 3].copy_from_slice(&c);
    }
    for (a, b, _) in &fences {
        plot(&mut hgt, *a, *b, [255, 30, 30]);
    }
    write_png(&pdir.join("evidence-height.png"), n, n, &hgt);
    let palette = [[30u8, 30, 30], [120, 200, 80], [40, 100, 40], [180, 180, 175], [170, 110, 60]];
    let mut cls = vec![0u8; cells * 3];
    for k in 0..cells {
        let c = if class[k] == NONE { if fp_mask[k] { [70, 70, 90] } else if structure[k] { [110, 60, 110] } else { [30, 30, 30] } } else { palette[class[k] as usize] };
        cls[k * 3..k * 3 + 3].copy_from_slice(&c);
    }
    for (a, b, _) in &fences {
        plot(&mut cls, *a, *b, [255, 30, 30]);
    }
    write_png(&pdir.join("evidence-classes.png"), n, n, &cls);
    println!("wrote {}, {}, {} and evidence-*.png", pdir.join("yards.json").display(), pdir.join("yard-ground.bin.gz").display(), pdir.join("yard-canopy.bin.gz").display());
}

fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
