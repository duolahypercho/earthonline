//! The USDA NAIP 0.6 m RGB aerial imagery (survey group A1): a 4 x 5 grid
//! of 2048 x 2048 JPEG tiles in plain longitude/latitude (EPSG:4326), row 0
//! the north row, from the USGS National Map's NAIP image service. Each tile
//! is decoded (with `jpeg-decoder`) on first use and kept; a tile is 12 MB
//! decoded, the whole grid 252 MB.
//!
//! Grid (the download's A1/manifest.json, copied into
//! native/assets/survey-manifest.json): west -122.5065, north 37.8345, tile
//! boxes 2048 x 6.8165e-6 degrees of longitude by 2048 x 5.4057e-6 of
//! latitude. The image service delivered square pixels of `dlon` degrees
//! (0.6 m east-west, 0.754 m north-south) about each requested box's centre,
//! so each tile reaches 212 rows past its box north and south: tile r001's
//! row 0 is tile r000's row 1624 (measured on the tiles). Column c of tile
//! col covers longitude `west + (col 2048 + c) dlon ..`; row r of tile row
//! covers latitude `centre(row) + (1024 - r) dlon ..`, centre(row) =
//! `north - (row + 0.5) 2048 dlat`.

use super::{Root, Window};
use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub west: f64,
    pub north: f64,
    pub dlon: f64,
    pub dlat: f64,
    pub cols: usize,
    pub rows: usize,
    pub tile: usize,
}

/// The A1 grid.
pub const GRID: Grid = Grid { west: -122.5065, north: 37.8345, dlon: 6.816516843464343e-06, dlat: 5.405703074068548e-06, cols: 4, rows: 5, tile: 2048 };

impl Grid {
    /// The grid's box (west, south, east, north), degrees.
    pub fn bounds(&self) -> [f64; 4] {
        let n = self.tile as f64;
        [self.west, self.north - self.rows as f64 * n * self.dlat, self.west + self.cols as f64 * n * self.dlon, self.north]
    }

    /// The global pixel (column, tile row x 2048 + row) holding (lon, lat),
    /// if inside: the tile whose box holds the latitude, the row in that
    /// tile's square-pixel image about its box's centre (module doc).
    pub fn pixel(&self, lon: f64, lat: f64) -> Option<(usize, usize)> {
        let n = self.tile as f64;
        let tile_lat = n * self.dlat;
        let t = ((self.north - lat) / tile_lat).floor();
        let c = ((lon - self.west) / self.dlon).floor();
        if t < 0.0 || t >= self.rows as f64 || c < 0.0 || c >= (self.cols * self.tile) as f64 {
            return None;
        }
        let centre = self.north - (t + 0.5) * tile_lat;
        let r = (n / 2.0 + (centre - lat) / self.dlon).floor().clamp(0.0, n - 1.0);
        Some((c as usize, t as usize * self.tile + r as usize))
    }

    /// The root-relative file of tile (row, col).
    pub fn file(row: usize, col: usize) -> String {
        format!("A1/rgb/r{row:03}_c{col:03}.jpg")
    }
}

/// The NAIP mosaic over the survey root.
pub struct Naip {
    root: Root,
    tiles: Mutex<HashMap<(usize, usize), Option<Arc<Vec<u8>>>>>,
}

impl Naip {
    pub fn open(root: &Root) -> Naip {
        Naip { root: root.clone(), tiles: Mutex::new(HashMap::new()) }
    }

    /// Tile (row, col) decoded, RGB8 row-major (None: missing or refused, said once).
    fn tile(&self, row: usize, col: usize) -> Option<Arc<Vec<u8>>> {
        if let Some(t) = self.tiles.lock().unwrap().get(&(row, col)) {
            return t.clone();
        }
        let t = decode_tile(&self.root, row, col).map_err(|e| eprintln!("survey: NAIP tile r{row} c{col}: {e:#}")).ok().map(Arc::new);
        self.tiles.lock().unwrap().insert((row, col), t.clone());
        t
    }

    /// The NAIP colour (sRGB bytes, as delivered) of the pixel holding (lon, lat).
    pub fn sample_rgb(&self, lon: f64, lat: f64) -> Option<[u8; 3]> {
        let (c, r) = GRID.pixel(lon, lat)?;
        let n = GRID.tile;
        let t = self.tile(r / n, c / n)?;
        let k = ((r % n) * n + c % n) * 3;
        Some([t[k], t[k + 1], t[k + 2]])
    }

    /// The colours over a longitude/latitude box at about `step_m`, as three
    /// windows (R, G, B; 0-255; NaN outside the grid).
    pub fn window(&self, lon0: f64, lat0: f64, lon1: f64, lat1: f64, step_m: f64) -> Result<[Window; 3]> {
        let ch = |k: usize| Window::sample(lon0, lat0, lon1, lat1, step_m, &|lon, lat| self.sample_rgb(lon, lat).map(|p| p[k] as f32));
        Ok([ch(0)?, ch(1)?, ch(2)?])
    }
}

fn decode_tile(root: &Root, row: usize, col: usize) -> Result<Vec<u8>> {
    if row >= GRID.rows || col >= GRID.cols {
        bail!("outside the grid");
    }
    let path = root.verified(&Grid::file(row, col))?;
    let f = std::fs::File::open(&path).with_context(|| format!("{}", path.display()))?;
    let mut dec = jpeg_decoder::Decoder::new(std::io::BufReader::new(f));
    let px = dec.decode().with_context(|| format!("{}: decoding", path.display()))?;
    let info = dec.info().context("no JPEG info")?;
    if info.pixel_format != jpeg_decoder::PixelFormat::RGB24 || info.width as usize != GRID.tile || info.height as usize != GRID.tile {
        bail!("{}: {:?} {} x {}, not RGB {n} x {n}", path.display(), info.pixel_format, info.width, info.height, n = GRID.tile);
    }
    Ok(px)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grid_matches_the_tiles_boxes() {
        // the download's first tile box: -122.5065 .. -122.4925398, 37.8234291 .. 37.8345.
        let b = GRID.bounds();
        assert!((GRID.west + 2048.0 * GRID.dlon - (-122.49253977350459)).abs() < 1e-9);
        assert!((GRID.north - 2048.0 * GRID.dlat - 37.82342912010431).abs() < 1e-9);
        assert!((b[2] - (-122.45065909401835)).abs() < 1e-9);
        // the box's north edge lies 212 rows into the first tile (square pixels about its centre)
        assert_eq!(GRID.pixel(GRID.west + 1e-9, GRID.north - 1e-9), Some((0, 211)));
        // tile r001's first row is tile r000's row 1624 (as measured on the tiles)
        let centre1 = GRID.north - 1.5 * 2048.0 * GRID.dlat;
        let lat = centre1 + 1023.5 * GRID.dlon;
        assert_eq!(GRID.pixel(GRID.west + 1e-9, lat), Some((0, 1624)));
        assert_eq!(GRID.pixel(-122.6, 37.8), None);
        assert_eq!(Grid::file(4, 3), "A1/rgb/r004_c003.jpg");
    }

    /// One known reading per surface, as relations (a check of the decode,
    /// the channel order and the grid's orientation, not a calibration; each
    /// a 7 x 7 pixel mean): the strait 700 m east of the bridge (clear of the
    /// deck) is bluer than it is red; Baker Beach's dry sand is warm and
    /// brighter than the water; the Presidio's forest is green and darker
    /// than both.
    #[test]
    fn water_sand_and_forest_read_as_themselves() {
        let Some(root) = super::super::tests::root_or_skip("naip") else { return };
        let n = Naip::open(&root);
        let mean = |lon: f64, lat: f64| {
            let mut acc = [0.0f64; 3];
            for dy in -3..=3 {
                for dx in -3..=3 {
                    let p = n.sample_rgb(lon + dx as f64 * GRID.dlon, lat + dy as f64 * GRID.dlat).expect("inside the grid");
                    for c in 0..3 {
                        acc[c] += p[c] as f64 / 49.0;
                    }
                }
            }
            acc
        };
        let y = |c: [f64; 3]| (c[0] + c[1] + c[2]) / 3.0;
        let water = mean(-122.4700, 37.8160);
        let sand = mean(-122.48375, 37.7935);
        let forest = mean(-122.4700, 37.7950);
        println!("strait {water:.0?}, Baker sand {sand:.0?}, Presidio forest {forest:.0?}");
        assert!(water[2] > water[0], "{water:?}");
        assert!(sand[0] >= sand[2] && y(sand) > y(water), "{sand:?} {water:?}");
        assert!(forest[1] > forest[0] && forest[1] > forest[2] && y(forest) < y(water), "{forest:?}");
        let [r, _, _] = n.window(-122.4710, 37.8155, -122.4690, 37.8165, 10.0).unwrap();
        assert!(r.values.iter().all(|v| v.is_finite()));
    }
}
