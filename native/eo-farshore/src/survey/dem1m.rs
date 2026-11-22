//! The USGS 3DEP 1 m DEMs (survey group E1): two 10 x 10 km GeoTIFF tiles of
//! the same footprint (UTM zone 10N on NAD83, `x54y419`: easting 540-550 km,
//! northing 4180-4190 km, with a 6 m overlap ring), heights NAVD88 metres,
//! float32 in 512 or 256 px tiles, LZW. Read a chunk at a time
//! ([`super::TiledRaster`]); a 10 km tile is never decoded whole.
//!
//! The two are different projects: `CA_SanFrancisco_B23` (the newer) and
//! `CA_NoCal_Wildfires_B5b_QL1_2018`; a point takes the newer's height where
//! it has one, else the older's.
//!
//! Longitude/latitude to UTM and back: the transverse Mercator on the GRS80
//! ellipsoid (NAD83's), the series of Snyder, J. P. (1987), Map Projections: A
//! Working Manual, USGS Professional Paper 1395, pp. 60-64 (eqs. 8-9 to 8-11
//! forward, 3-21 and 8-18 to 8-25 inverse), scale 0.9996 on the central
//! meridian 123 W, false easting 500 km.

use super::{Root, TiledRaster, Window};
use anyhow::{Result, anyhow};

/// GRS80: semi-major axis (m) and flattening.
pub const GRS80_A: f64 = 6_378_137.0;
pub const GRS80_F: f64 = 1.0 / 298.257_222_101;

pub fn grs80_e2() -> f64 {
    GRS80_F * (2.0 - GRS80_F)
}

/// A transverse Mercator: ellipsoid (a, e^2), central meridian (degrees),
/// central scale, false easting and northing (m).
#[derive(Clone, Copy, Debug)]
pub struct TransverseMercator {
    pub a: f64,
    pub e2: f64,
    pub lon0: f64,
    pub k0: f64,
    pub false_e: f64,
    pub false_n: f64,
}

/// UTM zone 10 north on NAD83 (EPSG:26910).
pub const UTM10N_NAD83: TransverseMercator = TransverseMercator { a: GRS80_A, e2: GRS80_F * (2.0 - GRS80_F), lon0: -123.0, k0: 0.9996, false_e: 500_000.0, false_n: 0.0 };

impl TransverseMercator {
    /// Snyder (3-21): the meridian distance from the equator to latitude `phi` (radians).
    fn meridian(&self, phi: f64) -> f64 {
        let e2 = self.e2;
        let (e4, e6) = (e2 * e2, e2 * e2 * e2);
        self.a * ((1.0 - e2 / 4.0 - 3.0 * e4 / 64.0 - 5.0 * e6 / 256.0) * phi - (3.0 * e2 / 8.0 + 3.0 * e4 / 32.0 + 45.0 * e6 / 1024.0) * (2.0 * phi).sin() + (15.0 * e4 / 256.0 + 45.0 * e6 / 1024.0) * (4.0 * phi).sin() - (35.0 * e6 / 3072.0) * (6.0 * phi).sin())
    }

    /// (easting, northing) m of (lon, lat) degrees (Snyder 8-9 to 8-11).
    pub fn forward(&self, lon: f64, lat: f64) -> (f64, f64) {
        let phi = lat.to_radians();
        let ep2 = self.e2 / (1.0 - self.e2);
        let (s, c) = phi.sin_cos();
        let n = self.a / (1.0 - self.e2 * s * s).sqrt();
        let t = (s / c).powi(2);
        let cc = ep2 * c * c;
        let a = (lon - self.lon0).to_radians() * c;
        let m = self.meridian(phi);
        let x = self.k0 * n * (a + (1.0 - t + cc) * a.powi(3) / 6.0 + (5.0 - 18.0 * t + t * t + 72.0 * cc - 58.0 * ep2) * a.powi(5) / 120.0);
        let y = self.k0 * (m + n * (s / c) * (a * a / 2.0 + (5.0 - t + 9.0 * cc + 4.0 * cc * cc) * a.powi(4) / 24.0 + (61.0 - 58.0 * t + t * t + 600.0 * cc - 330.0 * ep2) * a.powi(6) / 720.0));
        (x + self.false_e, y + self.false_n)
    }

    /// (lon, lat) degrees of (easting, northing) m (Snyder 8-18 to 8-25, with 3-24 and 3-26).
    pub fn inverse(&self, e: f64, n: f64) -> (f64, f64) {
        let (x, y) = (e - self.false_e, n - self.false_n);
        let e2 = self.e2;
        let ep2 = e2 / (1.0 - e2);
        let m = y / self.k0;
        let mu = m / (self.a * (1.0 - e2 / 4.0 - 3.0 * e2 * e2 / 64.0 - 5.0 * e2 * e2 * e2 / 256.0));
        let r = (1.0 - e2).sqrt();
        let e1 = (1.0 - r) / (1.0 + r);
        let phi1 = mu + (3.0 * e1 / 2.0 - 27.0 * e1.powi(3) / 32.0) * (2.0 * mu).sin() + (21.0 * e1 * e1 / 16.0 - 55.0 * e1.powi(4) / 32.0) * (4.0 * mu).sin() + (151.0 * e1.powi(3) / 96.0) * (6.0 * mu).sin() + (1097.0 * e1.powi(4) / 512.0) * (8.0 * mu).sin();
        let (s1, c1) = phi1.sin_cos();
        let cc1 = ep2 * c1 * c1;
        let t1 = (s1 / c1).powi(2);
        let w = 1.0 - e2 * s1 * s1;
        let n1 = self.a / w.sqrt();
        let r1 = self.a * (1.0 - e2) / w.powf(1.5);
        let d = x / (n1 * self.k0);
        let phi = phi1 - (n1 * (s1 / c1) / r1) * (d * d / 2.0 - (5.0 + 3.0 * t1 + 10.0 * cc1 - 4.0 * cc1 * cc1 - 9.0 * ep2) * d.powi(4) / 24.0 + (61.0 + 90.0 * t1 + 298.0 * cc1 + 45.0 * t1 * t1 - 252.0 * ep2 - 3.0 * cc1 * cc1) * d.powi(6) / 720.0);
        let lam = (d - (1.0 + 2.0 * t1 + cc1) * d.powi(3) / 6.0 + (5.0 - 2.0 * cc1 + 28.0 * t1 - 3.0 * cc1 * cc1 + 8.0 * ep2 + 24.0 * t1 * t1) * d.powi(5) / 120.0) / c1;
        (self.lon0 + lam.to_degrees(), phi.to_degrees())
    }
}

/// The E1 tiles, newest first (the order a point's height is taken in).
pub const TILES: [&str; 2] = ["E1/USGS_1M_10_x54y419_CA_SanFrancisco_B23.tif", "E1/USGS_1m_x54y419_CA_NoCal_Wildfires_B5b_QL1_2018.tif"];

/// The 1 m DEM: both tiles, each opened (and checked) on first use.
pub struct Dem1m {
    tiles: Vec<TiledRaster>,
}

impl Dem1m {
    /// Every E1 tile from the survey root, each checked against the manifest.
    pub fn open(root: &Root) -> Result<Dem1m> {
        let mut tiles = vec![];
        for rel in TILES {
            let path = root.verified(rel)?;
            let t = TiledRaster::open(&path)?;
            if (t.dx - 1.0).abs() > 1e-6 || (t.dy - 1.0).abs() > 1e-6 {
                return Err(anyhow!("{rel}: {} x {} m pixels, not 1 m", t.dx, t.dy));
            }
            tiles.push(t);
        }
        Ok(Dem1m { tiles })
    }

    /// The NAVD88 height (m) at (lon, lat) NAD83, bilinear on the 1 m grid;
    /// None outside both tiles or where neither has data.
    pub fn height(&self, lon: f64, lat: f64) -> Option<f32> {
        let (e, n) = UTM10N_NAD83.forward(lon, lat);
        self.tiles.iter().find_map(|t| t.at(e, n))
    }

    /// Heights over a longitude/latitude box at about `step_m` (NaN where none).
    pub fn window(&self, lon0: f64, lat0: f64, lon1: f64, lat1: f64, step_m: f64) -> Result<Window> {
        Window::sample(lon0, lat0, lon1, lat1, step_m, &|lon, lat| self.height(lon, lat))
    }

    /// The tiles' common box in UTM metres (x min, y min, x max, y max).
    pub fn bounds_utm(&self) -> [f64; 4] {
        self.tiles[0].bounds()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Snyder's worked example (PP 1395, p. 269): Clarke 1866, k0 0.9996,
    /// central meridian 75 W; 40 deg 30 min N, 73 deg 30 min W gives
    /// x 127,106.5 m, y 4,484,124.4 m.
    #[test]
    fn the_projection_matches_snyders_example_and_round_trips() {
        let clarke = TransverseMercator { a: 6_378_206.4, e2: 0.006_768_66, lon0: -75.0, k0: 0.9996, false_e: 0.0, false_n: 0.0 };
        let (x, y) = clarke.forward(-73.5, 40.5);
        assert!((x - 127_106.5).abs() < 0.5 && (y - 4_484_124.4).abs() < 0.5, "{x} {y}");
        let (lon, lat) = clarke.inverse(x, y);
        assert!((lon + 73.5).abs() < 1e-8 && (lat - 40.5).abs() < 1e-8, "{lon} {lat}");
        // the central meridian is easting 500 km; round trips across the E1 tile.
        let (e, _) = UTM10N_NAD83.forward(-123.0, 37.8);
        assert!((e - 500_000.0).abs() < 1e-6);
        for (lon, lat) in [(-122.4817, 37.8277), (-122.4652, 37.8051), (-122.55, 37.77)] {
            let (e, n) = UTM10N_NAD83.forward(lon, lat);
            let (lo, la) = UTM10N_NAD83.inverse(e, n);
            assert!((lo - lon).abs() < 1e-9 && (la - lat).abs() < 1e-9);
            assert!((540_000.0..=550_000.0).contains(&e) || lon < -122.54, "{lon} {lat} -> {e} {n}");
        }
    }

    /// The tiles open, the headland at Battery Spencer stands well above the
    /// sea (a plausibility range, 100-180 m), the strait under the bridge has
    /// no land height, and the 1 m height agrees with the far shore's
    /// independent 1 arc-second DEM there within 15 m (when the DEM folder, `external/dem` or EO_DEM_ROOT, holds it).
    #[test]
    fn heights_at_the_headland_and_the_strait() {
        let Some(root) = super::super::tests::root_or_skip("dem1m") else { return };
        let dem = Dem1m::open(&root).expect("E1 tiles");
        let b = dem.bounds_utm();
        assert!((b[0] - 539_994.0).abs() < 1.0 && (b[3] - 4_190_006.0).abs() < 1.0, "{b:?}");
        let h = dem.height(-122.4817, 37.8277).expect("Battery Spencer");
        println!("Battery Spencer: {h:.2} m NAVD88");
        assert!((100.0..180.0).contains(&h), "{h}");
        let strait = dem.height(-122.4780, 37.8160);
        println!("mid-strait: {strait:?}");
        assert!(strait.is_none_or(|v| v < 2.0), "{strait:?}");
        let w = dem.window(-122.4830, 37.8270, -122.4810, 37.8285, 5.0).unwrap();
        assert!(w.values.iter().filter(|v| v.is_finite()).count() as f64 > 0.9 * w.values.len() as f64);
        // the far shore's 1 arc-second tile over the headland (dem-manifest.json: raw/USGS_1_n38w123.tif).
        if let Some(t) = crate::dem_root().map(|d| d.join("raw/USGS_1_n38w123.tif")).filter(|p| p.is_file()).and_then(|p| crate::dem::DemTile::read(&p).ok()) {
            if let Some(s) = crate::dem::DemMosaic::new(vec![t]).ok().and_then(|m| m.sample(37.8277, -122.4817)) {
                println!("1 arc-second DEM there: {:.2} m", s.navd88);
                assert!((s.navd88 - h).abs() < 15.0, "1 m {h} against 1 arc-second {}", s.navd88);
            }
        }
    }
}
