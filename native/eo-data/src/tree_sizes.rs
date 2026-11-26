//! The DataSF street trees' trunk sizes (`sf-tree-sizes.json.gz`) and the USDA
//! Forest Service Urban Tree Database allometry that sizes a tree from its
//! trunk (`sf-tree-allometry.json`), both written by
//! `scripts/world-data/build-sf-tree-sizes.py` (sources, digests and licences
//! in their NOTICE files).
//!
//! Positions are the city data's local metres (the sf-city.json frame: x east,
//! z south) before the ground-plan scale. `TreeSizes::size` gives a tree's
//! crown diameter and height (metres) from its recorded DBH with the equations
//! of the database record its species is matched to (McPherson, van Doorn and
//! Peper 2016, PSW-GTR-253): the DBH held to that species' measured range, the
//! prediction to the equation's application range. A tree with no DBH, or a
//! palm, has no size here (the caller keeps what it had).

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use std::collections::HashMap;

/// One DBH equation (`TS6_Growth_coefficients.csv`; forms in `TS4_Growth_eqn_forms.csv`).
#[derive(Clone, Debug, Deserialize)]
pub struct Equation {
    pub eq: String,
    #[serde(default)]
    pub a: f64,
    #[serde(default)]
    pub b: f64,
    #[serde(default)]
    pub c: f64,
    #[serde(default)]
    pub d: f64,
    #[serde(default)]
    pub e: f64,
    /// The equation's application range (the predicted value's).
    pub min: Option<f64>,
    pub max: Option<f64>,
}

impl Equation {
    /// The prediction at `x` (DBH, cm; or age, years), held to `[min, max]`.
    pub fn eval(&self, x: f64) -> f64 {
        let (a, b, c, d, e) = (self.a, self.b, self.c, self.d, self.e);
        let ll = |x: f64| (x + 1.0).ln().ln();
        // the log forms' bias correction: mse (the `c` column) / 2, weighted.
        let w = |k: &str| match k {
            "1" => 1.0,
            "2" => x.sqrt(),
            "3" => x,
            _ => x * x,
        };
        let v = match self.eq.as_str() {
            "lin" => a + b * x,
            "quad" => a + b * x + c * x * x,
            "cub" => a + b * x + c * x * x + d * x * x * x,
            "quart" => a + b * x + c * x * x + d * x * x * x + e * x * x * x * x,
            q if q.starts_with("loglogw") => (a + b * ll(x) + w(&q[7..]) * c / 2.0).exp(),
            q if q.starts_with("expow") => (a + b * x + w(&q[5..]) * c / 2.0).exp(),
            _ => f64::NAN,
        };
        let lo = self.min.unwrap_or(f64::NEG_INFINITY);
        let hi = self.max.unwrap_or(f64::INFINITY);
        if v.is_finite() { v.clamp(lo, hi) } else { v }
    }
}

/// One database record: the crown diameter and tree height equations of a species in a region.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeciesAllometry {
    /// "<region>/<scientific name>", e.g. "NoCalC/Ulmus parvifolia".
    pub key: String,
    pub crown: Equation,
    pub height: Equation,
    pub age_dbh: Option<Equation>,
    /// The species' measured DBH range (cm).
    pub dbh_cm: [Option<f64>; 2],
}

impl SpeciesAllometry {
    /// (crown diameter, height), metres, at `dbh_cm` held to the measured range.
    pub fn size(&self, dbh_cm: f64) -> (f64, f64) {
        let x = dbh_cm.clamp(self.dbh_cm[0].unwrap_or(0.0), self.dbh_cm[1].unwrap_or(f64::INFINITY));
        (self.crown.eval(x), self.height.eval(x))
    }
}

#[derive(Deserialize)]
struct AllometryFile {
    equations: Vec<SpeciesAllometry>,
    broadleaf: Vec<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SizesFile {
    species: Vec<String>,
    species_allometry: Vec<i32>,
    species_match: Vec<String>,
    plots: Vec<String>,
    sites: Vec<String>,
    treeid: Vec<i64>,
    x: Vec<i32>,
    z: Vec<i32>,
    sp: Vec<u32>,
    dbh: Vec<i32>,
    plot: Vec<i32>,
    site: Vec<i32>,
    planted: Vec<u32>,
}

/// One list record.
#[derive(Clone, Copy, Debug)]
pub struct TreeRow {
    pub treeid: i64,
    /// City data metres (x east, z south), before the ground-plan scale.
    pub x: f64,
    pub z: f64,
    /// Index into `TreeSizes::species`.
    pub species: u32,
    /// Trunk diameter at breast height, inches, as recorded.
    pub dbh_in: Option<f64>,
    pub plot: Option<u32>,
    pub site: Option<u32>,
    /// Planting date as yyyymmdd.
    pub planted: Option<u32>,
}

/// How a list species is sized.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sizing {
    /// A database record (index into `TreeSizes::allometry`).
    Record(usize),
    /// The median over the region's broadleaf records.
    BroadleafMedian,
    /// Not sized (a palm, a record with no name).
    None,
}

pub struct TreeSizes {
    pub species: Vec<String>,
    /// Per species: how it was matched ("species/NoCalC", "genus/SoCalC", "family-myrtaceae", ...).
    pub species_match: Vec<String>,
    pub sizing: Vec<Sizing>,
    pub plots: Vec<String>,
    pub sites: Vec<String>,
    pub trees: Vec<TreeRow>,
    pub allometry: Vec<SpeciesAllometry>,
    broadleaf: Vec<usize>,
    cells: HashMap<(i32, i32), Vec<u32>>,
}

/// The spatial index's cell (metres).
const CELL_M: f64 = 2.0;

impl TreeSizes {
    pub fn load(paths: &crate::DataPaths) -> Result<Self> {
        let raw = crate::read_maybe_gzip(&paths.sf("sf-tree-sizes.json.gz"))?;
        let s: SizesFile = serde_json::from_slice(&raw).context("sf-tree-sizes.json.gz")?;
        let a: AllometryFile = serde_json::from_slice(&std::fs::read(paths.sf("sf-tree-allometry.json")).context("sf-tree-allometry.json")?).context("sf-tree-allometry.json")?;
        Self::from_parts(s, a)
    }

    fn from_parts(s: SizesFile, a: AllometryFile) -> Result<Self> {
        let n = s.treeid.len();
        ensure!(
            [s.x.len(), s.z.len(), s.sp.len(), s.dbh.len(), s.plot.len(), s.site.len(), s.planted.len()].iter().all(|&l| l == n),
            "sf-tree-sizes: columns of unequal length"
        );
        ensure!(s.species_allometry.len() == s.species.len() && s.species_match.len() == s.species.len(), "sf-tree-sizes: species columns");
        let sizing = s
            .species_allometry
            .iter()
            .map(|&e| match e {
                -2 => Ok(Sizing::BroadleafMedian),
                -1 => Ok(Sizing::None),
                i if (i as usize) < a.equations.len() => Ok(Sizing::Record(i as usize)),
                i => bail!("sf-tree-sizes: equation {i} of {}", a.equations.len()),
            })
            .collect::<Result<Vec<_>>>()?;
        ensure!(a.broadleaf.iter().all(|&i| i < a.equations.len()) && !a.broadleaf.is_empty(), "sf-tree-allometry: broadleaf list");
        let mut trees = Vec::with_capacity(n);
        let mut cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for i in 0..n {
            ensure!((s.sp[i] as usize) < s.species.len(), "sf-tree-sizes: species index");
            let t = TreeRow {
                treeid: s.treeid[i],
                x: s.x[i] as f64 / 10.0,
                z: s.z[i] as f64 / 10.0,
                species: s.sp[i],
                dbh_in: (s.dbh[i] > 0).then(|| s.dbh[i] as f64 / 10.0),
                plot: (s.plot[i] >= 0).then_some(s.plot[i] as u32),
                site: (s.site[i] >= 0).then_some(s.site[i] as u32),
                planted: (s.planted[i] > 0).then_some(s.planted[i]),
            };
            cells.entry(((t.x / CELL_M).floor() as i32, (t.z / CELL_M).floor() as i32)).or_default().push(i as u32);
            trees.push(t);
        }
        Ok(Self { species: s.species, species_match: s.species_match, sizing, plots: s.plots, sites: s.sites, trees, allometry: a.equations, broadleaf: a.broadleaf, cells })
    }

    /// The list record nearest (x, z) (city data metres) within `r` (at most `CELL_M`).
    pub fn nearest(&self, x: f64, z: f64, r: f64) -> Option<usize> {
        let r = r.min(CELL_M);
        let (cx, cz) = ((x / CELL_M).floor() as i32, (z / CELL_M).floor() as i32);
        let mut best: Option<(f64, usize)> = None;
        for gx in cx - 1..=cx + 1 {
            for gz in cz - 1..=cz + 1 {
                for &i in self.cells.get(&(gx, gz)).map(|v| v.as_slice()).unwrap_or(&[]) {
                    let t = &self.trees[i as usize];
                    let d = (t.x - x).hypot(t.z - z);
                    if d <= r && best.is_none_or(|b| d < b.0 || (d == b.0 && (i as usize) < b.1)) {
                        best = Some((d, i as usize));
                    }
                }
            }
        }
        best.map(|b| b.1)
    }

    /// (crown diameter, height), metres, of a species at `dbh_in` inches; None where the species is not sized.
    pub fn species_size(&self, species: u32, dbh_in: f64) -> Option<(f64, f64)> {
        let cm = dbh_in * 2.54;
        match self.sizing.get(species as usize)? {
            Sizing::Record(i) => Some(self.allometry[*i].size(cm)),
            Sizing::BroadleafMedian => {
                let mut c: Vec<f64> = vec![];
                let mut h: Vec<f64> = vec![];
                for &i in &self.broadleaf {
                    let (a, b) = self.allometry[i].size(cm);
                    c.push(a);
                    h.push(b);
                }
                Some((median(&mut c), median(&mut h)))
            }
            Sizing::None => None,
        }
    }

    /// (crown diameter, height), metres, of list record `i` from its recorded DBH; None with
    /// no DBH or where its species is not sized.
    pub fn size(&self, i: usize) -> Option<(f64, f64)> {
        let t = self.trees.get(i)?;
        self.species_size(t.species, t.dbh_in?).filter(|(c, h)| c.is_finite() && h.is_finite() && *c > 0.0 && *h > 0.0)
    }

    /// The genus of list record `i` (the first word of its scientific name), or "".
    pub fn genus(&self, i: usize) -> &str {
        self.trees.get(i).and_then(|t| self.species.get(t.species as usize)).and_then(|s| s.split_whitespace().next()).unwrap_or("")
    }
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    let n = v.len();
    if n == 0 {
        return f64::NAN;
    }
    if n % 2 == 1 { v[n / 2] } else { 0.5 * (v[n / 2 - 1] + v[n / 2]) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eq(eq: &str, a: f64, b: f64, c: f64, d: f64) -> Equation {
        Equation { eq: eq.into(), a, b, c, d, e: 0.0, min: None, max: None }
    }

    #[test]
    fn equation_forms_match_the_database_definitions() {
        // TS6, NoCalC Ulmus parvifolia: crown dia loglogw1 (a -0.37252, b 2.26842, mse 0.02963);
        // tree ht quad (2.51638, 0.40448, -0.0036). At 10 cm, by hand from TS4's forms:
        let crown = eq("loglogw1", -0.37252, 2.26842, 0.02963, 0.0);
        let want = (-0.37252f64 + 2.26842 * (11.0f64).ln().ln() + 0.02963 / 2.0).exp();
        assert!((crown.eval(10.0) - want).abs() < 1e-12 && (want - 5.08).abs() < 0.01, "{}", crown.eval(10.0));
        let height = eq("quad", 2.51638, 0.40448, -0.0036, 0.0);
        assert!((height.eval(10.0) - 6.20118).abs() < 1e-9);
        // the weighted log forms add sqrt(x), x, x^2 times mse / 2.
        let w2 = eq("loglogw2", 0.1, 1.0, 0.04, 0.0);
        assert!((w2.eval(16.0) - (0.1 + (17.0f64).ln().ln() + 4.0 * 0.02).exp()).abs() < 1e-12);
        let e3 = eq("expow3", 0.5, 0.02, 0.01, 0.0);
        assert!((e3.eval(20.0) - (0.5f64 + 0.4 + 20.0 * 0.005).exp()).abs() < 1e-12);
        let cub = eq("cub", 1.0, 0.5, -0.01, 0.0001);
        assert!((cub.eval(10.0) - (1.0 + 5.0 - 1.0 + 0.1)).abs() < 1e-12);
        // held to the application range.
        let held = Equation { min: Some(2.0), max: Some(4.0), ..eq("lin", 0.0, 1.0, 0.0, 0.0) };
        assert_eq!((held.eval(1.0), held.eval(3.0), held.eval(9.0)), (2.0, 3.0, 4.0));
    }

    #[test]
    fn nearest_finds_within_radius_only() {
        let s = SizesFile {
            species: vec!["Ulmus parvifolia :: Chinese Elm".into()],
            species_allometry: vec![0],
            species_match: vec!["species/NoCalC".into()],
            plots: vec![],
            sites: vec![],
            treeid: vec![1, 2],
            x: vec![100, 130],
            z: vec![-50, -50],
            sp: vec![0, 0],
            dbh: vec![40, -1],
            plot: vec![-1, -1],
            site: vec![-1, -1],
            planted: vec![20070507, 0],
        };
        let rec = SpeciesAllometry { key: "NoCalC/Ulmus parvifolia".into(), crown: eq("lin", 1.0, 0.1, 0.0, 0.0), height: eq("lin", 2.0, 0.2, 0.0, 0.0), age_dbh: None, dbh_cm: [Some(2.5), Some(62.0)] };
        let t = TreeSizes::from_parts(s, AllometryFile { equations: vec![rec], broadleaf: vec![0] }).unwrap();
        assert_eq!(t.nearest(10.05, -5.0, 1.0), Some(0));
        assert_eq!(t.nearest(11.6, -5.0, 1.0), None);
        assert_eq!(t.nearest(12.9, -5.0, 1.0), Some(1));
        let (c, h) = t.size(0).unwrap();
        assert!((c - (1.0 + 0.1 * 4.0 * 2.54)).abs() < 1e-9 && (h - (2.0 + 0.2 * 4.0 * 2.54)).abs() < 1e-9);
        assert!(t.size(1).is_none(), "no DBH: no size");
        assert_eq!(t.trees[0].planted, Some(20070507));
        assert_eq!(t.genus(0), "Ulmus");
    }
}
