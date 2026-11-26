//! San Francisco's bikeway network per street segment
//! (`public/data/sf/sf-bikeways.json.gz`, built by
//! `scripts/world-data/build-sf-bikeways.py` from DataSF's MTA Bike Network
//! Linear Features, SFMTA, PDDL 1.0; see its NOTICE).
//!
//! The rows are per facility and direction; a street segment (CNN) gathers
//! them into one `Bikeway`: its class (the highest on-street one, II lane or
//! IV separated), whether its lanes carry both directions on one side (a
//! contraflow row beside a with-flow row on the same side: a two-way
//! bikeway), and the union of their buffers, barriers and green paint.
//! Positions are the city file's local metres (sf-city.json `meta`).

use anyhow::{Context, Result, bail};
use std::collections::HashMap;

pub const FILE: &str = "sf-bikeways.json.gz";
pub const VERSION: &str = "sf-bikeways-v1";
const CELL_M: f64 = 32.0;

pub const F_CONTRAFLOW: u32 = 1;
pub const F_BUFFERED: u32 = 2;
pub const F_RAISED: u32 = 4;
pub const F_GREEN: u32 = 8;
pub const F_OTHER_SIDE: u32 = 16;
pub const B_CONCRETE: u32 = 1;
pub const B_POSTS: u32 = 2;
pub const B_PARKING: u32 = 4;
pub const B_SIDEWALK_LEVEL: u32 = 8;

/// One street segment's bikeway.
#[derive(Clone, Debug)]
pub struct Bikeway {
    pub cnn: u32,
    /// 1 path, 2 lane, 3 route, 4 separated bikeway (the highest on-street class of its rows).
    pub class: u8,
    /// Both directions in one facility on one side of the street.
    pub two_way: bool,
    pub green: bool,
    pub buffered: bool,
    pub raised: bool,
    /// `B_*` bits, the union of its rows'.
    pub barrier: u32,
    pub line: Vec<[f64; 2]>,
}

pub struct Bikeways {
    pub as_of: String,
    pub segments: Vec<Bikeway>,
    grid: HashMap<(i32, i32), Vec<u32>>,
}

fn dist_to_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> (f64, [f64; 2]) {
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dz * dz;
    let t = if l2 > 1e-12 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / l2).clamp(0.0, 1.0) } else { 0.0 };
    let q = [a[0] + dx * t, a[1] + dz * t];
    let l = l2.sqrt().max(1e-9);
    (((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt(), [dx / l, dz / l])
}

impl Bikeways {
    pub fn load(paths: &crate::DataPaths) -> Result<Self> {
        let p = paths.sf(FILE);
        Self::parse(&crate::read_maybe_gzip(&p)?).with_context(|| format!("{}", p.display()))
    }

    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let v: serde_json::Value = serde_json::from_slice(bytes)?;
        if v["version"].as_str() != Some(VERSION) {
            bail!("version {:?}, expected {VERSION}", v["version"]);
        }
        // rows gathered per CNN: (class, dir, flags, barrier), and the longest line
        let mut by: HashMap<u32, (Vec<(u8, String, u32, u32)>, Vec<[f64; 2]>)> = HashMap::new();
        for r in v["records"].as_array().context("records")? {
            let r = r.as_array().context("record")?;
            let (Some(cnn), Some(class)) = (r.first().and_then(|x| x.as_f64()), r.get(1).and_then(|x| x.as_f64())) else { continue };
            let dir = r.get(2).and_then(|x| x.as_str()).unwrap_or("").to_string();
            let flags = r.get(3).and_then(|x| x.as_u64()).unwrap_or(0) as u32;
            let barrier = r.get(4).and_then(|x| x.as_u64()).unwrap_or(0) as u32;
            let flat: Vec<f64> = r.get(5).and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.as_f64()).collect()).unwrap_or_default();
            let line: Vec<[f64; 2]> = flat.chunks_exact(2).map(|c| [c[0], c[1]]).collect();
            let e = by.entry(cnn as u32).or_default();
            e.0.push((class as u8, dir, flags, barrier));
            if line.len() > e.1.len() {
                e.1 = line;
            }
        }
        let mut segments = vec![];
        let mut cnns: Vec<u32> = by.keys().copied().collect();
        cnns.sort_unstable();
        for cnn in cnns {
            let (rows, line) = by.remove(&cnn).unwrap();
            if line.len() < 2 {
                continue;
            }
            let lanes: Vec<&(u8, String, u32, u32)> = rows.iter().filter(|r| r.0 == 2 || r.0 == 4).collect();
            let class = lanes.iter().map(|r| r.0).max().unwrap_or_else(|| rows.iter().map(|r| r.0).max().unwrap_or(0));
            let two_way = lanes.iter().any(|a| a.2 & F_CONTRAFLOW != 0 && lanes.iter().any(|b| b.2 & F_CONTRAFLOW == 0 && b.1 != a.1 && (a.2 & F_OTHER_SIDE) == (b.2 & F_OTHER_SIDE)));
            let any = |f: u32| lanes.iter().any(|r| r.2 & f != 0);
            let barrier = lanes.iter().fold(0, |acc, r| acc | r.3);
            segments.push(Bikeway { cnn, class, two_way, green: any(F_GREEN), buffered: any(F_BUFFERED), raised: any(F_RAISED), barrier, line });
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
        Ok(Self { as_of: v["asOf"].as_str().unwrap_or_default().to_string(), segments, grid })
    }

    /// The bikeway nearest `p` (local metres) within `reach` whose line runs
    /// within `max_angle_deg` of `dir` (either way); and its distance.
    pub fn segment_at(&self, p: [f64; 2], dir: [f64; 2], reach: f64, max_angle_deg: f64) -> Option<(&Bikeway, f64)> {
        let cos_min = max_angle_deg.to_radians().cos();
        let r = (reach / CELL_M).ceil() as i32;
        let (cx, cz) = ((p[0] / CELL_M).floor() as i32, (p[1] / CELL_M).floor() as i32);
        let mut best: Option<(usize, f64)> = None;
        for dz in -r..=r {
            for dx in -r..=r {
                for &i in self.grid.get(&(cx + dx, cz + dz)).map(|v| v.as_slice()).unwrap_or(&[]) {
                    for w in self.segments[i as usize].line.windows(2) {
                        let (d, u) = dist_to_segment(p, w[0], w[1]);
                        if d <= reach && (u[0] * dir[0] + u[1] * dir[1]).abs() >= cos_min && best.is_none_or(|b| d < b.1) {
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
    fn rows_gather_into_a_two_way_bikeway() {
        let doc = br#"{"version":"sf-bikeways-v1","asOf":"2026-10-24","records":[
            [5664000,4,"W",1,1,[1604.9,-445.0,1408.9,-250.4]],
            [5664000,4,"E",0,1,[1604.9,-445.0,1408.9,-250.4]],
            [7,2,"N",8,0,[0,0,0,50]],[7,2,"S",24,0,[0,0,0,50]]]}"#;
        let b = Bikeways::parse(doc).unwrap();
        let (f, _) = b.segment_at([1506.9, -347.7], [0.709, -0.705], 10.0, 25.0).unwrap();
        assert_eq!((f.cnn, f.class, f.two_way, f.green, f.barrier), (5664000, 4, true, false, B_CONCRETE));
        // a lane each side of a two-way street is not a two-way bikeway
        let (g, _) = b.segment_at([1.0, 20.0], [0.0, 1.0], 5.0, 25.0).unwrap();
        assert_eq!((g.class, g.two_way, g.green), (2, false, true));
    }

    /// The shipped table: Folsom Street 4th to 5th is a two-way bikeway behind concrete.
    #[test]
    fn shipped_table_reads() {
        let Ok(paths) = crate::DataPaths::discover() else { return };
        if !paths.sf(FILE).is_file() {
            return;
        }
        let b = Bikeways::load(&paths).unwrap();
        assert!(b.segments.len() > 1000, "{} segments", b.segments.len());
        let (f, _) = b.segment_at([1503.1, -350.2], [0.7099, -0.7041], 12.0, 25.0).unwrap();
        assert_eq!(f.cnn, 5664000);
        assert!(f.two_way && f.class == 4 && f.barrier & B_CONCRETE != 0);
    }
}
