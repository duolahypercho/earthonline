//! The city's bedrock map, `native/assets/sf-lithology-v1.json.gz` (Docs/HILLS-PLAN.md
//! stage H7): the U.S. Geological Survey's geologic map units (MF-2337 north of
//! 37 45' N, OF 98-354 south of it, MF-2342 at Yerba Buena Island), each read to a rock
//! class from the map's own unit description, as polygons in the surveyed plan metres
//! (x east, z south of [`crate::Frame::sf`]). Built by
//! `scripts/world-data/build-sf-lithology.py`; sources, digests and the unit table are in
//! the file's `meta` and `sf-lithology-v1.NOTICE.txt` beside it.
//!
//! [`LithologyMap::load`] rasterises the polygons on a [`CELL_M`] grid in the file's paint
//! order (a later source wins where two maps overlap), so a query is one cell read.

use anyhow::{Context, Result, ensure};
use serde::Deserialize;

/// The table under `native/assets/` (`crate::roof_colours::native_assets`).
pub const FILE: &str = "sf-lithology-v1.json.gz";
/// The raster's cell (surveyed m). The maps are drawn at 1:24,000-1:62,500, their
/// lines good to tens of metres (their use constraints): a few metres resolve them.
pub const CELL_M: f64 = 5.0;

/// A map unit's rock class (the file's `meta.classes`, in that order).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RockClass {
    /// Franciscan radiolarian chert.
    Chert,
    /// Franciscan sandstone, greywacke and shale, and the melange's sheared matrix.
    Sandstone,
    Serpentinite,
    /// Altered basalt (Franciscan pillow lava).
    Greenstone,
    /// Quaternary deposits: fill, dune and beach sand, slope debris, landslides, alluvium.
    Surficial,
    /// Other bedrock: the Merced Formation's soft sediments, fault rocks, metamorphic rocks.
    Other,
    Water,
}

impl RockClass {
    pub const ALL: [RockClass; 7] = [Self::Chert, Self::Sandstone, Self::Serpentinite, Self::Greenstone, Self::Surficial, Self::Other, Self::Water];

    pub fn name(self) -> &'static str {
        ["chert", "sandstone", "serpentinite", "greenstone", "surficial", "other", "water"][self as usize]
    }

    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name() == s)
    }

    /// Mapped bedrock (not a Quaternary deposit, not water).
    pub fn bedrock(self) -> bool {
        !matches!(self, Self::Surficial | Self::Water)
    }
}

#[derive(Deserialize)]
struct Doc {
    meta: Meta,
    polygons: Vec<Poly>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Meta {
    version: String,
    classes: Vec<String>,
    projection: Projection,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Projection {
    center: Centre,
    meters_per_deg_lat: f64,
    meters_per_deg_lon: f64,
}

#[derive(Deserialize)]
struct Centre {
    lat: f64,
    lon: f64,
}

#[derive(Deserialize)]
struct Poly {
    /// The class's index in `meta.classes`.
    c: usize,
    /// Rings as flat x, z pairs; filled even-odd (outer rings and holes alike).
    rings: Vec<Vec<f64>>,
}

/// The bedrock map as a raster of rock classes.
pub struct LithologyMap {
    x0: f64,
    z0: f64,
    w: usize,
    h: usize,
    /// 0: no map; else 1 + the class.
    cells: Vec<u8>,
    pub version: String,
}

impl LithologyMap {
    /// The table in `native/assets/` (the workspace's own, or the one under the repository
    /// root of `paths` for a moved build), as the other derived tables are found.
    pub fn load(paths: &crate::DataPaths) -> Result<Self> {
        let own = crate::roof_colours::native_assets().join(FILE);
        let path = if own.is_file() { own } else { paths.repo_root.join("native/assets").join(FILE) };
        let bytes = crate::read_maybe_gzip(&path)?;
        Self::from_json(&bytes).with_context(|| format!("reading {}", path.display()))
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        let doc: Doc = serde_json::from_slice(bytes)?;
        let f = crate::Frame::sf();
        let p = &doc.meta.projection;
        ensure!(
            p.center.lat == f.center_lat && p.center.lon == f.center_lon && p.meters_per_deg_lat == f.m_per_deg_lat && (p.meters_per_deg_lon - f.m_per_deg_lon).abs() < 1e-6,
            "the map's plan frame is not the city's"
        );
        let classes: Vec<RockClass> = doc.meta.classes.iter().map(|n| RockClass::from_name(n).with_context(|| format!("unknown rock class {n:?}"))).collect::<Result<_>>()?;
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for poly in &doc.polygons {
            ensure!(poly.c < classes.len(), "class index {} out of range", poly.c);
            for r in &poly.rings {
                ensure!(r.len() >= 6 && r.len() % 2 == 0 && r.iter().all(|v| v.is_finite()), "a ring needs three finite points");
                for q in r.chunks_exact(2) {
                    for k in 0..2 {
                        lo[k] = lo[k].min(q[k]);
                        hi[k] = hi[k].max(q[k]);
                    }
                }
            }
        }
        ensure!(lo[0] < hi[0] && lo[1] < hi[1], "no polygons");
        let (x0, z0) = ((lo[0] / CELL_M).floor() * CELL_M, (lo[1] / CELL_M).floor() * CELL_M);
        let w = ((hi[0] - x0) / CELL_M).ceil() as usize + 1;
        let h = ((hi[1] - z0) / CELL_M).ceil() as usize + 1;
        let mut cells = vec![0u8; w * h];
        let mut hits: Vec<(u32, f64)> = Vec::new();
        for poly in &doc.polygons {
            let v = 1 + classes[poly.c] as u8;
            // even-odd scanlines through the cell centres: each edge's crossings per row,
            // then the spans between pairs of crossings filled.
            hits.clear();
            for r in &poly.rings {
                let n = r.len() / 2;
                for i in 0..n {
                    let (ax, az) = (r[2 * i], r[2 * i + 1]);
                    let j = (i + 1) % n;
                    let (bx, bz) = (r[2 * j], r[2 * j + 1]);
                    if az == bz {
                        continue;
                    }
                    let (zlo, zhi) = (az.min(bz), az.max(bz));
                    // rows whose centre z0 + (row + 0.5) cell lies in [zlo, zhi)
                    let r0 = ((zlo - z0) / CELL_M - 0.5).ceil().max(0.0) as i64;
                    let r1 = ((zhi - z0) / CELL_M - 0.5).ceil().min(h as f64) as i64;
                    for row in r0..r1 {
                        let zc = z0 + (row as f64 + 0.5) * CELL_M;
                        hits.push((row as u32, ax + (zc - az) * (bx - ax) / (bz - az)));
                    }
                }
            }
            hits.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
            for pair in hits.chunks_exact(2) {
                let ((row, xa), (rb, xb)) = (pair[0], pair[1]);
                if row != rb {
                    continue; // a row with an odd count (a vertex exactly on its centre line)
                }
                // columns whose centre x0 + (col + 0.5) cell lies in [xa, xb)
                let c0 = ((xa - x0) / CELL_M - 0.5).ceil().max(0.0) as usize;
                let c1 = (((xb - x0) / CELL_M - 0.5).ceil().max(0.0) as usize).min(w);
                let base = row as usize * w;
                cells[base + c0.min(c1)..base + c1].fill(v);
            }
        }
        Ok(Self { x0, z0, w, h, cells, version: doc.meta.version })
    }

    /// The mapped rock class at surveyed plan (x, z) (None off the maps).
    pub fn class_at(&self, x: f64, z: f64) -> Option<RockClass> {
        let (i, j) = (((x - self.x0) / CELL_M).floor(), ((z - self.z0) / CELL_M).floor());
        if i < 0.0 || j < 0.0 || i >= self.w as f64 || j >= self.h as f64 {
            return None;
        }
        let v = self.cells[j as usize * self.w + i as usize];
        (v > 0).then(|| RockClass::ALL[(v - 1) as usize])
    }

    /// Every mapped cell's centre (surveyed plan) and class.
    pub fn cells(&self) -> Vec<(f64, f64, RockClass)> {
        let mut out = vec![];
        for j in 0..self.h {
            for i in 0..self.w {
                let v = self.cells[j * self.w + i];
                if v > 0 {
                    out.push((self.x0 + (i as f64 + 0.5) * CELL_M, self.z0 + (j as f64 + 0.5) * CELL_M, RockClass::ALL[(v - 1) as usize]));
                }
            }
        }
        out
    }

    /// The share of mapped cells per class (a diagnostic).
    pub fn shares(&self) -> Vec<(RockClass, f64)> {
        let mut n = [0usize; 8];
        for &v in &self.cells {
            n[v as usize] += 1;
        }
        let total = n[1..].iter().sum::<usize>().max(1) as f64;
        RockClass::ALL.iter().map(|&c| (c, n[1 + c as usize] as f64 / total)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plan (x, z) of a geographic point.
    fn plan(lat: f64, lon: f64) -> (f64, f64) {
        let f = crate::Frame::sf();
        ((lon - f.center_lon) * f.m_per_deg_lon, (f.center_lat - lat) * f.m_per_deg_lat)
    }

    #[test]
    fn rasterises_rings_even_odd_in_paint_order() {
        let f = crate::Frame::sf();
        let doc = serde_json::json!({
            "meta": {"version": "t", "classes": ["chert", "sandstone", "serpentinite", "greenstone", "surficial", "other", "water"],
                     "projection": {"center": {"lat": f.center_lat, "lon": f.center_lon}, "metersPerDegLat": f.m_per_deg_lat, "metersPerDegLon": f.m_per_deg_lon}},
            "polygons": [
                // a 100 m sandstone square with a 40 m hole, then a chert square painted over its corner
                {"c": 1, "rings": [[0.0, 0.0, 100.0, 0.0, 100.0, 100.0, 0.0, 100.0], [30.0, 30.0, 70.0, 30.0, 70.0, 70.0, 30.0, 70.0]]},
                {"c": 0, "rings": [[80.0, 80.0, 120.0, 80.0, 120.0, 120.0, 80.0, 120.0]]}
            ]
        });
        let m = LithologyMap::from_json(doc.to_string().as_bytes()).unwrap();
        assert_eq!(m.class_at(10.0, 10.0), Some(RockClass::Sandstone));
        assert_eq!(m.class_at(50.0, 50.0), None, "the hole");
        assert_eq!(m.class_at(90.0, 90.0), Some(RockClass::Chert), "the later polygon wins");
        assert_eq!(m.class_at(110.0, 10.0), None);
        assert_eq!(m.class_at(-500.0, 10.0), None);
    }

    #[test]
    fn rejects_another_frame() {
        let doc = serde_json::json!({
            "meta": {"version": "t", "classes": ["chert"], "projection": {"center": {"lat": 37.0, "lon": -122.0}, "metersPerDegLat": 1.0, "metersPerDegLon": 1.0}},
            "polygons": [{"c": 0, "rings": [[0.0, 0.0, 1.0, 0.0, 1.0, 1.0]]}]
        });
        assert!(LithologyMap::from_json(doc.to_string().as_bytes()).is_err());
    }

    /// The shipped map: the city's chert hills are chert, Potrero Hill serpentinite,
    /// Telegraph Hill sandstone, the Sunset's dunes surficial (USGS MF-2337, OF 98-354).
    #[test]
    fn the_shipped_map_reads_the_hills() {
        let m = LithologyMap::load(&crate::DataPaths::discover().unwrap()).unwrap();
        let at = |lat: f64, lon: f64| {
            let (x, z) = plan(lat, lon);
            m.class_at(x, z)
        };
        assert_eq!(at(37.7516, -122.4477), Some(RockClass::Chert), "Twin Peaks");
        assert_eq!(at(37.7383, -122.4537), Some(RockClass::Chert), "Mount Davidson");
        assert_eq!(at(37.7432, -122.4145), Some(RockClass::Chert), "Bernal Heights");
        assert_eq!(at(37.7653, -122.4386), Some(RockClass::Chert), "Corona Heights");
        assert_eq!(at(37.7576, -122.3995), Some(RockClass::Serpentinite), "Potrero Hill");
        assert_eq!(at(37.8025, -122.4058), Some(RockClass::Sandstone), "Telegraph Hill");
        assert_eq!(at(37.7600, -122.4900), Some(RockClass::Surficial), "the Sunset's dunes");
        assert_eq!(at(37.8105, -122.3655), Some(RockClass::Sandstone), "Yerba Buena Island");
        assert_eq!(at(37.90, -122.48), None, "Marin, off the box");
        let s = m.shares();
        let share = |c: RockClass| s.iter().find(|(k, _)| *k == c).unwrap().1;
        assert!(share(RockClass::Surficial) > 0.4 && share(RockClass::Chert) > 0.005 && share(RockClass::Serpentinite) > 0.005, "{s:?}");
        // the two maps close on their seam (37 45' N NAD27, 37.74989 N WGS84): every
        // cell across it is mapped, from the ocean off Ocean Beach (OF 98-354's west edge
        // is 122.5087 W) to the bay.
        let (_, zs) = plan(37.749_89, -122.45);
        let gaps: Vec<f64> = (0..10_900)
            .map(|k| -7_900.0 + k as f64)
            .filter(|&x| (-2..=2).any(|dz| m.class_at(x, zs + dz as f64 * 2.0).is_none()))
            .collect();
        let runs: Vec<(f64, f64)> = gaps.iter().fold(Vec::new(), |mut v: Vec<(f64, f64)>, &x| {
            match v.last_mut() {
                Some(r) if x - r.1 <= 1.0 => r.1 = x,
                _ => v.push((x, x)),
            }
            v
        });
        assert!(gaps.is_empty(), "{} unmapped metres along the seam: {runs:?}", gaps.len());
    }
}
