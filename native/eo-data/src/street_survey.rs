//! The city's kerbside parking and mid-block signalised crossings
//! (`public/data/sf/sf-street-survey.json.gz`, built by
//! `scripts/world-data/build-sf-street-survey.py` from DataSF's parking
//! regulations, parking meters and traffic signals, SFMTA, PDDL 1.0; see its
//! NOTICE). Positions are the city file's local metres (sf-city.json `meta`),
//! the surveyed plan before the ground-plan scale.

use anyhow::{Context, Result, bail};
use std::collections::HashMap;

pub const FILE: &str = "sf-street-survey.json.gz";
pub const VERSION: &str = "sf-street-survey-v1";
const CELL_M: f64 = 32.0;

/// A regulated stretch of kerb.
#[derive(Clone, Debug)]
pub struct ParkingStretch {
    /// A car may park here (time limited, permit, paid ...); false: no parking or stopping.
    pub allowed: bool,
    pub line: Vec<[f64; 2]>,
}

/// A signalised, beaconed or flashing crossing between junctions.
#[derive(Clone, Copy, Debug)]
pub struct Midblock {
    pub cnn: u32,
    /// 1 signal, 2 beacon, 3 flasher.
    pub kind: u8,
    pub at: [f64; 2],
}

pub struct StreetSurvey {
    pub parking: Vec<ParkingStretch>,
    pub meters: Vec<[f64; 2]>,
    pub midblock: Vec<Midblock>,
    parking_grid: HashMap<(i32, i32), Vec<u32>>,
    meter_grid: HashMap<(i32, i32), Vec<u32>>,
}

fn cell(p: [f64; 2]) -> (i32, i32) {
    ((p[0] / CELL_M).floor() as i32, (p[1] / CELL_M).floor() as i32)
}

impl StreetSurvey {
    pub fn load(paths: &crate::DataPaths) -> Result<Self> {
        let p = paths.sf(FILE);
        Self::parse(&crate::read_maybe_gzip(&p)?).with_context(|| format!("{}", p.display()))
    }

    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let v: serde_json::Value = serde_json::from_slice(bytes)?;
        if v["version"].as_str() != Some(VERSION) {
            bail!("version {:?}, expected {VERSION}", v["version"]);
        }
        let pts = |a: &serde_json::Value| -> Vec<[f64; 2]> {
            let flat: Vec<f64> = a.as_array().map(|a| a.iter().filter_map(|x| x.as_f64()).collect()).unwrap_or_default();
            flat.chunks_exact(2).map(|c| [c[0], c[1]]).collect()
        };
        let mut parking = vec![];
        for r in v["parking"].as_array().context("parking")? {
            let line = pts(&r[1]);
            if line.len() >= 2 {
                parking.push(ParkingStretch { allowed: r[0].as_u64() == Some(1), line });
            }
        }
        let meters = pts(&v["meters"]);
        let mut midblock = vec![];
        for r in v["midblock"].as_array().context("midblock")? {
            let (Some(cnn), Some(kind), Some(x), Some(z)) = (r[0].as_f64(), r[1].as_u64(), r[2].as_f64(), r[3].as_f64()) else { continue };
            midblock.push(Midblock { cnn: cnn as u32, kind: kind as u8, at: [x, z] });
        }
        let mut parking_grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, s) in parking.iter().enumerate() {
            let mut seen = std::collections::HashSet::new();
            for p in &s.line {
                if seen.insert(cell(*p)) {
                    parking_grid.entry(cell(*p)).or_default().push(i as u32);
                }
            }
        }
        let mut meter_grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, p) in meters.iter().enumerate() {
            meter_grid.entry(cell(*p)).or_default().push(i as u32);
        }
        Ok(Self { parking, meters, midblock, parking_grid, meter_grid })
    }

    /// The kerb's parking along one side of a polyline (surveyed metres):
    /// (metres of kerb that allow parking, metres that do not, meters) whose
    /// points project onto the polyline between its ends, on the side
    /// `side` (+1: the side of its normal (-dz, dx), -1 the other), between
    /// `lat_min` and `lat_max` from it.
    pub fn parking_along(&self, line: &[[f64; 2]], side: f64, lat_min: f64, lat_max: f64) -> (f64, f64, usize) {
        let probe = |p: [f64; 2]| -> bool {
            let mut best: Option<(f64, f64)> = None; // (|lateral|, signed lateral)
            for w in line.windows(2) {
                let (a, b) = (w[0], w[1]);
                let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
                let l2 = dx * dx + dz * dz;
                if l2 < 1e-9 {
                    continue;
                }
                let t = ((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / l2;
                if !(0.0..=1.0).contains(&t) {
                    continue;
                }
                let l = l2.sqrt();
                let lat = ((p[0] - a[0]) * -dz + (p[1] - a[1]) * dx) / l;
                if best.is_none_or(|b| lat.abs() < b.0) {
                    best = Some((lat.abs(), lat));
                }
            }
            best.is_some_and(|(d, lat)| lat * side > 0.0 && d >= lat_min && d <= lat_max)
        };
        let mut cells = std::collections::HashSet::new();
        let r = (lat_max / CELL_M).ceil() as i32 + 1;
        for p in line {
            let (cx, cz) = cell(*p);
            for dz in -r..=r {
                for dx in -r..=r {
                    cells.insert((cx + dx, cz + dz));
                }
            }
        }
        // a line's cells along its length too
        for w in line.windows(2) {
            let n = (((w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])) / CELL_M).ceil() as i32;
            for k in 0..=n {
                let t = k as f64 / n.max(1) as f64;
                let (cx, cz) = cell([w[0][0] + (w[1][0] - w[0][0]) * t, w[0][1] + (w[1][1] - w[0][1]) * t]);
                for dz in -r..=r {
                    for dx in -r..=r {
                        cells.insert((cx + dx, cz + dz));
                    }
                }
            }
        }
        let (mut ok, mut banned, mut meters) = (0.0, 0.0, 0usize);
        let mut seen = std::collections::HashSet::new();
        for c in &cells {
            for &i in self.parking_grid.get(c).map(|v| v.as_slice()).unwrap_or(&[]) {
                if !seen.insert(i) {
                    continue;
                }
                let s = &self.parking[i as usize];
                for w in s.line.windows(2) {
                    let mid = [(w[0][0] + w[1][0]) / 2.0, (w[0][1] + w[1][1]) / 2.0];
                    if probe(mid) {
                        let l = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
                        if s.allowed {
                            ok += l;
                        } else {
                            banned += l;
                        }
                    }
                }
            }
            for &i in self.meter_grid.get(c).map(|v| v.as_slice()).unwrap_or(&[]) {
                if probe(self.meters[i as usize]) {
                    meters += 1;
                }
            }
        }
        (ok, banned, meters)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parking_is_read_by_side() {
        let doc = br#"{"version":"sf-street-survey-v1","parking":[[1,[2.0,10.0,2.0,40.0]],[0,[-2.0,10.0,-2.0,20.0]]],
            "meters":[3.0,5.0,3.0,15.0,-3.0,5.0],"midblock":[[7,1,0.5,25.0]]}"#;
        let s = StreetSurvey::parse(doc).unwrap();
        // a line along +z: its normal (-dz, dx) = (-1, 0), the -x side
        let line = [[0.0, 0.0], [0.0, 50.0]];
        let (ok, banned, m) = s.parking_along(&line, -1.0, 1.0, 6.0);
        assert_eq!((ok.round(), banned, m), (30.0, 0.0, 2));
        let (ok2, banned2, m2) = s.parking_along(&line, 1.0, 1.0, 6.0);
        assert_eq!((ok2, banned2.round(), m2), (0.0, 10.0, 1));
        assert_eq!(s.midblock.len(), 1);
    }

    /// The shipped table: metered parking on both sides of Folsom 4th to 5th, and its mid-block signal.
    #[test]
    fn shipped_table_reads() {
        let Ok(paths) = crate::DataPaths::discover() else { return };
        if !paths.sf(FILE).is_file() {
            return;
        }
        let s = StreetSurvey::load(&paths).unwrap();
        assert!(s.meters.len() > 20_000 && s.parking.len() > 5_000);
        let line = [[1405.1, -253.0], [1601.1, -447.4]];
        for side in [1.0, -1.0] {
            let (_, _, m) = s.parking_along(&line, side, 3.0, 20.0);
            assert!(m >= 15, "side {side}: {m} meters");
        }
        assert!(s.midblock.iter().any(|b| b.cnn == 5664000));
    }
}
