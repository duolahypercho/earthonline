//! The USDA NAIP 0.3 m RGB tiles `native/tools/fetch-aerial.mjs` writes into
//! the folder named by `EO_AERIAL_DIR` (outside the repository): the tile
//! grid, the manifest (source, licence, credit, a SHA-256 per file) and the
//! survey's haze. The grid and the haze follow `eo-kit/examples/roof_survey.rs`
//! (the roof colour survey reads the same tiles), so the two agree.
//!
//! The pixels are square in degrees: the image service serves an EPSG:4326
//! request with pixels square in degrees (asked for 2000 x 2000 pixels of a
//! box that is not square in degrees, it widens the box north and south
//! about its centre until it is), so only a grid planned square is served
//! exactly over the box the manifest records. [`Manifest::parse`] refuses
//! any other grid, and any file whose recorded box is not its tile's.

use anyhow::{Context, Result, bail};
use eo_data::roof_colours::PlanFrame;

/// The tile side in pixels.
pub const TILE: usize = 2000;

/// A plain lon/lat pixel grid: pixel (gx, gy) covers lon west + gx*dlon ..
/// +dlon, lat north - gy*dlat .. -dlat; tile (r, c) holds gx in
/// [c*TILE, (c+1)*TILE) and gy in [r*TILE, (r+1)*TILE).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub west: f64,
    pub north: f64,
    pub dlon: f64,
    pub dlat: f64,
    pub cols: usize,
    pub rows: usize,
}

impl Grid {
    /// A grid as the manifest records it (`grids.rgb`), of `TILE` px tiles.
    pub fn from_json(v: &serde_json::Value) -> Option<Grid> {
        let f = |k: &str| v[k].as_f64();
        let u = |k: &str| v[k].as_u64().map(|n| n as usize);
        (u("tile")? == TILE).then_some(())?;
        Some(Grid { west: f("west")?, north: f("north")?, dlon: f("dlon")?, dlat: f("dlat")?, cols: u("cols")?, rows: u("rows")? })
    }
    /// Pixels square in degrees (the box asked for is the box served).
    pub fn square(&self) -> bool {
        (self.dlon - self.dlat).abs() <= 1e-9 * self.dlat
    }
    /// Ground metres of one pixel, east-west and north-south.
    pub fn pixel_m(&self, frame: &PlanFrame) -> [f64; 2] {
        [self.dlon * frame.m_per_deg_lon, self.dlat * frame.m_per_deg_lat]
    }
    /// Continuous pixel coordinates of (lon, lat) (pixel centres at +0.5).
    pub fn px(&self, lon: f64, lat: f64) -> (f64, f64) {
        ((lon - self.west) / self.dlon, (self.north - lat) / self.dlat)
    }
    /// (lon, lat) of continuous pixel coordinates.
    pub fn lonlat(&self, gx: f64, gy: f64) -> (f64, f64) {
        (self.west + gx * self.dlon, self.north - gy * self.dlat)
    }
    /// Tile (r, c)'s box: [west, south, east, north].
    pub fn tile_bbox(&self, r: usize, c: usize) -> [f64; 4] {
        let w = self.west + (c * TILE) as f64 * self.dlon;
        let e = w + TILE as f64 * self.dlon;
        let n = self.north - (r * TILE) as f64 * self.dlat;
        let s = n - TILE as f64 * self.dlat;
        [w, s, e, n]
    }
    /// The whole grid's box: [west, south, east, north].
    pub fn bbox(&self) -> [f64; 4] {
        let e = self.west + self.width_px() as f64 * self.dlon;
        let s = self.north - self.height_px() as f64 * self.dlat;
        [self.west, s, e, self.north]
    }
    pub fn width_px(&self) -> usize {
        self.cols * TILE
    }
    pub fn height_px(&self) -> usize {
        self.rows * TILE
    }
}

/// `<set>/rRRR_cCCC.jpg`, the file of tile (r, c) of a set (`rgb`, `cir`).
pub fn tile_file(set: &str, r: usize, c: usize) -> String {
    format!("{set}/r{r:03}_c{c:03}.jpg")
}

/// One tile the manifest lists.
#[derive(Clone, Debug)]
pub struct TileEntry {
    pub r: usize,
    pub c: usize,
    /// Relative to the aerial folder (`rgb/r000_c025.jpg`, `cir/r000_c012.jpg`).
    pub file: String,
    /// Lower-case hex SHA-256 of the file as fetched.
    pub sha256: String,
}

/// The parts of `manifest.json` the cooks read.
#[derive(Clone, Debug)]
pub struct Manifest {
    pub rgb: Grid,
    /// The RGB tiles (0.3 m; bands red, green, blue), in (row, column) order.
    pub tiles: Vec<TileEntry>,
    /// The colour-infrared grid and tiles (0.6 m; bands near infrared, red,
    /// green), when fetched.
    pub cir: Option<Grid>,
    pub cir_tiles: Vec<TileEntry>,
    pub source: String,
    pub licence: String,
    pub credit: String,
    pub downloaded: String,
    /// The plan frame the tiles were planned in (`grids.frame`), when recorded:
    /// centre lat, centre lon, metres per degree of latitude and of longitude.
    pub frame: Option<[f64; 4]>,
}

impl Manifest {
    /// Parse `manifest.json`: the RGB grid and the CIR grid if any (square
    /// pixels), and every file's name, recorded box (its tile's) and digest.
    pub fn parse(bytes: &[u8]) -> Result<Manifest> {
        let m: serde_json::Value = serde_json::from_slice(bytes).context("manifest.json")?;
        let grid = |set: &str| -> Result<Option<Grid>> {
            if m["grids"][set].is_null() {
                return Ok(None);
            }
            let g = Grid::from_json(&m["grids"][set]).with_context(|| format!("manifest.json: grids.{set} is not a grid of 2000 px tiles"))?;
            if !g.square() {
                bail!("manifest.json: the {set} tiles were planned with pixels {:.3e} x {:.3e} degrees, not square; the image service serves such a box stretched north-south about its centre. Plan into an empty folder and fetch again.", g.dlon, g.dlat);
            }
            Ok(Some(g))
        };
        let rgb = grid("rgb")?.context("manifest.json: no grids.rgb")?;
        let cir = grid("cir")?;
        let (mut tiles, mut cir_tiles) = (vec![], vec![]);
        for f in m["files"].as_array().context("manifest.json: no files")? {
            let name = f["file"].as_str().unwrap_or_default();
            let Some((set, rest)) = name.split_once('/') else { continue };
            let (g, list) = match (set, cir) {
                ("rgb", _) => (rgb, &mut tiles),
                ("cir", Some(c)) => (c, &mut cir_tiles),
                _ => continue,
            };
            let (r, c) = (rest.get(1..4).and_then(|s| s.parse().ok()), rest.get(6..9).and_then(|s| s.parse().ok()));
            let (Some(r), Some(c)) = (r, c) else { bail!("manifest.json: {name}: not a tile name") };
            if r >= g.rows || c >= g.cols || tile_file(set, r, c) != name {
                bail!("manifest.json: {name}: not a tile of the {} x {} grid", g.cols, g.rows);
            }
            let want = g.tile_bbox(r, c);
            let got: Vec<f64> = f["bbox"].as_array().map(|a| a.iter().filter_map(|v| v.as_f64()).collect()).unwrap_or_default();
            if !(got.len() == 4 && (0..4).all(|k| (got[k] - want[k]).abs() < 1e-9)) {
                bail!("manifest.json: {name}: recorded box {got:?} is not its tile's {want:?}");
            }
            let sha256 = f["sha256"].as_str().unwrap_or_default().to_ascii_lowercase();
            if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
                bail!("manifest.json: {name}: no SHA-256");
            }
            list.push(TileEntry { r, c, file: name.to_string(), sha256 });
        }
        for list in [&mut tiles, &mut cir_tiles] {
            list.sort_by_key(|t| (t.r, t.c));
            list.dedup_by_key(|t| (t.r, t.c));
        }
        let s = |k: &str| m[k].as_str().unwrap_or_default().to_string();
        let fr = &m["grids"]["frame"];
        let frame = (|| Some([fr["centerLat"].as_f64()?, fr["centerLon"].as_f64()?, fr["metersPerDegreeLat"].as_f64()?, fr["metersPerDegreeLon"].as_f64()?]))();
        Ok(Manifest { rgb, tiles, cir, cir_tiles, source: s("source"), licence: s("licence"), credit: s("credit"), downloaded: s("downloaded"), frame })
    }

    /// The tiles the survey takes its haze from: every 9th tile present, in
    /// (row, column) order (`roof_survey`'s `set_haze`).
    pub fn haze_sample<'a>(present: impl Iterator<Item = &'a TileEntry>) -> Vec<(usize, usize)> {
        present.step_by(9).map(|t| (t.r, t.c)).collect()
    }
}

/// Per-channel 8-bit histograms of a tile, the no-data fill (0, 0, 0) left out.
pub type Hist = [[u32; 256]; 3];

/// The histograms of interleaved RGB8 pixels and how many pixels they hold.
pub fn histogram(px: &[u8]) -> (Box<Hist>, u32) {
    let mut h = Box::new([[0u32; 256]; 3]);
    let mut n = 0;
    for p in px.chunks_exact(3) {
        if p == [0, 0, 0] {
            continue;
        }
        for k in 0..3 {
            h[k][p[k] as usize] += 1;
        }
        n += 1;
    }
    (h, n)
}

/// A tile's haze per channel (linear light): its 0.2 % darkest level, at most
/// 0.2 (`roof_survey`'s `haze_of`). None under 10 000 pixels.
///
/// Dark-object subtraction: the photo's haze (light scattered into every
/// pixel, which lifts dark ground toward blue-grey) is the level of its
/// deepest shadows, taken out per channel in linear light.
pub fn haze_of(hist: &Hist, n: u32) -> Option<[f32; 3]> {
    dark_level(hist, n).map(|d| d.map(|v| eo_data::roof_colours::srgb_to_linear(v as f32 / 255.0).min(0.2)))
}

/// Each channel's 0.2 % darkest 8-bit level (the haze as delivered). None
/// under 10 000 pixels.
pub fn dark_level(hist: &Hist, n: u32) -> Option<[u8; 3]> {
    if n < 10_000 {
        return None;
    }
    let mut out = [0u8; 3];
    for k in 0..3 {
        let mut acc = 0;
        for (v, c) in hist[k].iter().enumerate() {
            acc += c;
            if acc as f32 >= n as f32 * 0.002 {
                out[k] = v as u8;
                break;
            }
        }
    }
    Some(out)
}

/// The survey's one haze level: the per-channel median of the tiles' own
/// (so no tile edge shows as a step).
pub fn median_haze(levels: &[[f32; 3]]) -> Option<[f32; 3]> {
    if levels.is_empty() {
        return None;
    }
    let mut med = [0f32; 3];
    for k in 0..3 {
        let mut v: Vec<f32> = levels.iter().map(|l| l[k]).collect();
        v.sort_by(f32::total_cmp);
        med[k] = v[v.len() / 2];
    }
    Some(med)
}

/// NDVI of a colour-infrared pixel's near-infrared and red values.
pub fn ndvi(nir: f32, red: f32) -> f32 {
    (nir - red) / (nir + red).max(1e-6)
}

/// A linear value with the haze `d` taken out (`roof_survey`'s `apply_haze`).
pub fn dehaze(l: f32, d: f32) -> f32 {
    ((l - d) / (1.0 - d)).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> PlanFrame {
        PlanFrame { center_lat: 37.778, center_lon: -122.4194, m_per_deg_lat: 110574.0, m_per_deg_lon: 87986.24747640654 }
    }

    fn grid() -> Grid {
        let d = 0.3 / frame().m_per_deg_lat;
        Grid { west: -122.5157, north: 37.8331, dlon: d, dlat: d, cols: 30, rows: 24 }
    }

    #[test]
    fn pixel_lonlat_plan_round_trip() {
        let (g, f) = (grid(), frame());
        assert!(g.square());
        let pm = g.pixel_m(&f);
        assert!((pm[1] - 0.3).abs() < 1e-9 && pm[0] < pm[1], "{pm:?}");
        for &(gx, gy) in &[(0.5, 0.5), (12345.25, 678.75), (59999.5, 47999.5), (30000.0, 24000.0)] {
            let (lon, lat) = g.lonlat(gx, gy);
            let (x, z) = f.to_plan(lon, lat);
            let (lon2, lat2) = f.to_lonlat(x, z);
            let (gx2, gy2) = g.px(lon2, lat2);
            assert!((gx2 - gx).abs() < 1e-6 && (gy2 - gy).abs() < 1e-6, "({gx}, {gy}) -> ({gx2}, {gy2})");
        }
        // The plan is linear in lon and lat: one pixel is a fixed plan step,
        // east along x and south along z.
        let (x0, z0) = f.to_plan(g.lonlat(100.0, 100.0).0, g.lonlat(100.0, 100.0).1);
        let (x1, z1) = f.to_plan(g.lonlat(101.0, 101.0).0, g.lonlat(101.0, 101.0).1);
        assert!((x1 - x0 - pm[0]).abs() < 1e-9 && (z1 - z0 - pm[1]).abs() < 1e-9);
        // A tile's box is its pixels' box.
        let bb = g.tile_bbox(3, 7);
        let (w, n) = g.lonlat((7 * TILE) as f64, (3 * TILE) as f64);
        let (e, s) = g.lonlat((8 * TILE) as f64, (4 * TILE) as f64);
        assert!([bb[0] - w, bb[1] - s, bb[2] - e, bb[3] - n].iter().all(|d| d.abs() < 1e-12));
        assert_eq!(g.bbox()[0], g.west);
    }

    #[test]
    fn manifest_checks_boxes_and_digests() {
        let g = grid();
        let cir = Grid { dlon: 2.0 * g.dlon, dlat: 2.0 * g.dlat, cols: 15, rows: 12, ..g };
        let bb = g.tile_bbox(0, 25);
        let doc = |bbox: [f64; 4], sha: &str| {
            serde_json::to_vec(&serde_json::json!({
                "source": "s", "licence": "l", "credit": "c", "downloaded": "d",
                "grids": {
                    "rgb": { "west": g.west, "north": g.north, "dlon": g.dlon, "dlat": g.dlat, "cols": g.cols, "rows": g.rows, "metres": 0.3, "tile": TILE },
                    "cir": { "west": g.west, "north": g.north, "dlon": 2.0 * g.dlon, "dlat": 2.0 * g.dlat, "cols": 15, "rows": 12, "metres": 0.6, "tile": TILE },
                },
                "files": [ { "file": "rgb/r000_c025.jpg", "bbox": bbox, "sha256": sha }, { "file": "cir/r000_c012.jpg", "bbox": cir.tile_bbox(0, 12), "sha256": sha } ],
            }))
            .unwrap()
        };
        let sha = "81a40ec6731a204effcbf5a4c301b318c4b7ffdc41072909d061464324306cdd";
        let m = Manifest::parse(&doc(bb, sha)).unwrap();
        assert_eq!(m.tiles.len(), 1);
        assert_eq!((m.tiles[0].r, m.tiles[0].c), (0, 25));
        assert_eq!(m.rgb, g);
        assert_eq!(m.cir, Some(cir));
        assert_eq!((m.cir_tiles.len(), m.cir_tiles[0].file.as_str()), (1, "cir/r000_c012.jpg"));
        assert!(Manifest::parse(&doc([bb[0], bb[1] + 1e-6, bb[2], bb[3]], sha)).is_err(), "a box off its tile");
        assert!(Manifest::parse(&doc(bb, "abc")).is_err(), "no digest");
    }

    #[test]
    fn haze_is_the_darkest_fifth_percent() {
        let mut px = vec![];
        for i in 0..20_000u32 {
            let v = if i < 20 { 10 } else if i < 60 { 30 } else { 120 };
            px.extend_from_slice(&[v, v, v]);
        }
        px.extend_from_slice(&[0, 0, 0]);
        let (h, n) = histogram(&px);
        assert_eq!(n, 20_000);
        let d = haze_of(&h, n).unwrap();
        // 0.2 % of 20 000 is 40: the 20 at 10 and 20 more at 30.
        assert!((d[0] - eo_data::roof_colours::srgb_to_linear(30.0 / 255.0)).abs() < 1e-6);
        assert_eq!(median_haze(&[[0.01, 0.02, 0.03], [0.03, 0.01, 0.02], [0.02, 0.03, 0.01]]), Some([0.02; 3]));
        assert_eq!(dehaze(0.01, 0.02), 0.0);
        assert!((dehaze(0.5, 0.0) - 0.5).abs() < 1e-7);
        assert!((ndvi(150.0, 50.0) - 0.5).abs() < 1e-7);
    }
}
