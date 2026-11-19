//! The NOAA NCEI CUDEM topobathy (survey group B1): two 1/9 arc-second
//! (about 3.4 m) GeoTIFF tiles, 0.25 x 0.25 degree with a 6-cell overlap
//! ring, NAD83 geographic, heights NAVD88 metres (negative below the datum,
//! the sea bed), float32 in 512 px tiles, deflate. Read a chunk at a time
//! ([`super::TiledRaster`]); a tile is never decoded whole.
//!
//! The tiles' GeoTIFF raster type is pixel-is-area (GTRasterTypeGeoKey 1) and
//! the tie point is the outer corner of pixel (0, 0), exactly 6 cells outside
//! the tile's 0.25-degree edge, so a pixel (c, r) covers longitude
//! `west + c d ..` and latitude `north - r d ..`.

use super::{Root, TiledRaster, Window};
use anyhow::{Result, bail};

/// The B1 tiles: n38.00 w122.50 (east) and n38.00 w122.75 (west).
pub const TILES: [&str; 2] = ["B1/ncei19_n38x00_w122x50_2022v1.tif", "B1/ncei19_n38x00_w122x75_2022v1.tif"];

/// 1/9 arc-second in degrees.
pub const CELL_DEG: f64 = 1.0 / 9.0 / 3600.0;

pub struct Bathy {
    tiles: Vec<TiledRaster>,
}

impl Bathy {
    /// Both tiles from the survey root, each checked against the manifest.
    pub fn open(root: &Root) -> Result<Bathy> {
        let mut tiles = vec![];
        for rel in TILES {
            let t = TiledRaster::open(&root.verified(rel)?)?;
            if (t.dx - CELL_DEG).abs() > 1e-9 || (t.dy - CELL_DEG).abs() > 1e-9 {
                bail!("{rel}: {} x {} degree cells, not 1/9 arc-second", t.dx, t.dy);
            }
            tiles.push(t);
        }
        Ok(Bathy { tiles })
    }

    /// The NAVD88 height (m; the sea bed is negative) at (lon, lat) NAD83,
    /// bilinear; None outside both tiles or at no data.
    pub fn height(&self, lon: f64, lat: f64) -> Option<f32> {
        self.tiles.iter().find_map(|t| t.at(lon, lat))
    }

    /// Heights over a longitude/latitude box at about `step_m` (NaN where none).
    pub fn window(&self, lon0: f64, lat0: f64, lon1: f64, lat1: f64, step_m: f64) -> Result<Window> {
        Window::sample(lon0, lat0, lon1, lat1, step_m, &|lon, lat| self.height(lon, lat))
    }

    /// Each tile's box (west, south, east, north), degrees.
    pub fn bounds(&self) -> Vec<[f64; 4]> {
        self.tiles.iter().map(|t| t.bounds()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plausibility check, not a calibration: the strait under the bridge
    /// is the bay's deep scoured channel (of the order of 100 m; checked only
    /// as deeper than 50 m), Crissy Field's beach is near the datum, and the
    /// tiles meet at 122.5 W without a step.
    #[test]
    fn the_channel_is_deep_and_the_beach_is_not() {
        let Some(root) = super::super::tests::root_or_skip("bathy") else { return };
        let b = Bathy::open(&root).expect("B1 tiles");
        let bb = b.bounds();
        assert!((bb[0][3] - 38.0).abs() < 7.0 * CELL_DEG && (bb[0][0] + 122.5).abs() < 7.0 * CELL_DEG, "{bb:?}");
        let deep = b.height(-122.4780, 37.8160).expect("mid-strait");
        println!("mid-strait under the bridge: {deep:.1} m NAVD88");
        assert!(deep < -50.0, "{deep}");
        let beach = b.height(-122.46523, 37.80510).expect("Crissy Field beach");
        println!("Crissy Field beach: {beach:.2} m NAVD88");
        assert!((-3.0..6.0).contains(&beach), "{beach}");
        // across the tiles' seam (122.5 W) in the Pacific off Point Bonita.
        let (w, e) = (b.height(-122.5001, 37.80), b.height(-122.4999, 37.80));
        if let (Some(w), Some(e)) = (w, e) {
            assert!((w - e).abs() < 1.5, "{w} {e}");
        }
        let win = b.window(-122.4800, 37.8150, -122.4760, 37.8170, 10.0).unwrap();
        assert!(win.values.iter().all(|v| v.is_finite() && *v < 0.0));
    }
}
