//! Each street segment's pavement condition and last resurfacing
//! (`public/data/sf/sf-pavement.json.gz`, built by
//! `scripts/world-data/build-sf-pavement.py` from DataSF's Pavement Condition
//! Index tables, San Francisco Public Works, PDDL 1.0; see its NOTICE).
//!
//! Positions are the city file's local metres (sf-city.json `meta`: x east, z
//! south), the frame of the surveyed plan before the ground-plan scale. The
//! pavement's age is the snapshot date (`asOf`) less the latest resurfacing,
//! not the game's clock, so two runs read the same street.

use anyhow::{Context, Result, bail};
use std::collections::HashMap;

/// The file under `public/data/sf/`.
pub const FILE: &str = "sf-pavement.json.gz";
pub const VERSION: &str = "sf-pavement-v1";
/// The grid the segments are indexed on (m).
const CELL_M: f64 = 32.0;

/// One street segment (a CNN between two cross streets).
#[derive(Clone, Debug)]
pub struct Segment {
    pub cnn: u32,
    /// Pavement Condition Index, 0 (failed) .. 100 (new).
    pub pci: u8,
    /// Years from the latest resurfacing to the snapshot; None where the
    /// history records none (laid before 1992, or never resurfaced).
    pub age_years: Option<f32>,
    /// Functional class: b'A' arterial, b'C' collector, b'R' residential, 0 unknown.
    pub class: u8,
    /// The segment's line, local metres.
    pub line: Vec<[f64; 2]>,
}

/// The pavement table and its index.
pub struct Pavement {
    pub as_of: String,
    pub segments: Vec<Segment>,
    grid: HashMap<(i32, i32), Vec<u32>>,
}

/// Days from 1970-01-01 of a `YYYY-MM-DD` date (proleptic Gregorian).
fn days(s: &str) -> Option<i64> {
    let mut it = s.get(..10)?.split('-');
    let (y, m, d): (i64, i64, i64) = (it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

fn dist_to_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> (f64, [f64; 2]) {
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dz * dz;
    let t = if l2 > 1e-12 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / l2).clamp(0.0, 1.0) } else { 0.0 };
    let q = [a[0] + dx * t, a[1] + dz * t];
    let l = l2.sqrt().max(1e-9);
    (((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt(), [dx / l, dz / l])
}

impl Pavement {
    /// Read the table from the repository (`DataPaths::sf`).
    pub fn load(paths: &crate::DataPaths) -> Result<Self> {
        let p = paths.sf(FILE);
        Self::parse(&crate::read_maybe_gzip(&p)?).with_context(|| format!("{}", p.display()))
    }

    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let v: serde_json::Value = serde_json::from_slice(bytes)?;
        if v["version"].as_str() != Some(VERSION) {
            bail!("version {:?}, expected {VERSION}", v["version"]);
        }
        let as_of = v["asOf"].as_str().unwrap_or_default().to_string();
        let now = days(&as_of).context("asOf is not a date")?;
        let mut segments = vec![];
        for r in v["segments"].as_array().context("segments")? {
            let r = r.as_array().context("segment record")?;
            let num = |i: usize| r.get(i).and_then(|x| x.as_f64());
            let (Some(cnn), Some(pci)) = (num(0), num(1)) else { continue };
            let age_years = r.get(5).and_then(|x| x.as_str()).and_then(days).map(|d| ((now - d).max(0) as f64 / 365.25) as f32);
            let class = r.get(4).and_then(|x| x.as_str()).and_then(|s| s.bytes().next()).unwrap_or(0);
            let flat: Vec<f64> = r.get(6).and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.as_f64()).collect()).unwrap_or_default();
            let line: Vec<[f64; 2]> = flat.chunks_exact(2).map(|c| [c[0], c[1]]).collect();
            if line.len() < 2 {
                continue;
            }
            segments.push(Segment { cnn: cnn as u32, pci: pci.clamp(0.0, 100.0) as u8, age_years, class, line });
        }
        let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, s) in segments.iter().enumerate() {
            let (mut x0, mut z0, mut x1, mut z1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for p in &s.line {
                x0 = x0.min(p[0]);
                z0 = z0.min(p[1]);
                x1 = x1.max(p[0]);
                z1 = z1.max(p[1]);
            }
            for cz in (z0 / CELL_M).floor() as i32..=(z1 / CELL_M).floor() as i32 {
                for cx in (x0 / CELL_M).floor() as i32..=(x1 / CELL_M).floor() as i32 {
                    grid.entry((cx, cz)).or_default().push(i as u32);
                }
            }
        }
        Ok(Self { as_of, segments, grid })
    }

    /// The segment nearest `p` (local metres) within `reach`, among those
    /// running within `max_angle_deg` of `dir` (either way) when a direction
    /// is given: the segment and its distance.
    pub fn segment_at(&self, p: [f64; 2], dir: Option<[f64; 2]>, reach: f64, max_angle_deg: f64) -> Option<(&Segment, f64)> {
        let cos_min = max_angle_deg.to_radians().cos();
        let r = (reach / CELL_M).ceil() as i32;
        let (cx, cz) = ((p[0] / CELL_M).floor() as i32, (p[1] / CELL_M).floor() as i32);
        let mut best: Option<(usize, f64)> = None;
        let mut seen = std::collections::HashSet::new();
        for dz in -r..=r {
            for dx in -r..=r {
                for &i in self.grid.get(&(cx + dx, cz + dz)).map(|v| v.as_slice()).unwrap_or(&[]) {
                    if !seen.insert(i) {
                        continue;
                    }
                    let s = &self.segments[i as usize];
                    for w in s.line.windows(2) {
                        let (d, u) = dist_to_segment(p, w[0], w[1]);
                        if d > reach {
                            continue;
                        }
                        if let Some(t) = dir {
                            if (u[0] * t[0] + u[1] * t[1]).abs() < cos_min {
                                continue;
                            }
                        }
                        if best.is_none_or(|b| d < b.1) {
                            best = Some((i as usize, d));
                        }
                    }
                }
            }
        }
        best.map(|(i, d)| (&self.segments[i], d))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_from_civil() {
        assert_eq!(days("1970-01-01"), Some(0));
        assert_eq!(days("2000-03-01"), Some(11017));
        assert_eq!(days("2026-07-27").unwrap() - days("2026-01-01").unwrap(), 207);
        assert_eq!(days("2026-13-01"), None);
    }

    #[test]
    fn parses_and_finds_by_position_and_heading() {
        let doc = br#"{"version":"sf-pavement-v1","asOf":"2026-11-25","segments":[
            [5664000,100,"2026-07-27","T","A","2026-07-27",[1604.9,-445.0,1408.9,-250.4]],
            [1,40,"2020-01-01","S","R",null,[0.0,0.0,0.0,50.0]]],"points":[]}"#;
        let p = Pavement::parse(doc).unwrap();
        assert_eq!(p.segments.len(), 2);
        // the Folsom 800 block's middle, heading along it
        let (s, d) = p.segment_at([1506.9, -347.7], Some([0.709, -0.705]), 10.0, 25.0).unwrap();
        assert_eq!(s.cnn, 5664000);
        assert!(d < 1.0);
        assert!((s.age_years.unwrap() - 0.33).abs() < 0.02);
        // across it: refused by the heading
        assert!(p.segment_at([1506.9, -347.7], Some([0.705, 0.709]), 10.0, 25.0).is_none());
        // the second has no resurfacing on record
        let (s2, _) = p.segment_at([1.0, 20.0], None, 5.0, 25.0).unwrap();
        assert_eq!((s2.pci, s2.age_years, s2.class), (40, None, b'R'));
    }

    /// The shipped table: every segment parses, and the Folsom 800 block is
    /// the 2026 resurfacing (needs the repository's data file).
    #[test]
    fn shipped_table_reads() {
        let Ok(paths) = crate::DataPaths::discover() else { return };
        if !paths.sf(FILE).is_file() {
            return;
        }
        let p = Pavement::load(&paths).unwrap();
        assert!(p.segments.len() > 10_000, "{} segments", p.segments.len());
        let (s, _) = p.segment_at([1503.1, -350.2], Some([0.7099, -0.7041]), 12.0, 25.0).unwrap();
        assert_eq!(s.cnn, 5664000);
        assert_eq!(s.pci, 100);
        assert!(s.age_years.unwrap() < 1.0);
    }
}
