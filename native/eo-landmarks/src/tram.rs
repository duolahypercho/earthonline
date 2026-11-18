//! The streetcar tracks of the Embarcadero / Ferry Building corridor: the
//! historic F Market & Wharves line's and the E line's rails along The
//! Embarcadero's median and down Market Street, drawn flush in the paving.
//! Nothing else in the native build draws a rail.
//!
//! Where: the `railWays` of public/data/sf/sf-city.json.gz (OpenStreetMap
//! `railway=tram|light_rail`, ODbL; see sf-city.NOTICE.txt), every way with a
//! vertex within `CORRIDOR_M` of the Ferry Building's tower, on the scaled
//! ground plan (`groundscale`: the street plan they run in is scaled; the
//! gauge and the rail sections stay real). An OSM tram way is one track.
//!
//! What (cited):
//! * Track gauge 1435 mm, standard gauge, which the Muni streetcar and light
//!   rail network runs on (SFMTA; the UIC / AREMA standard gauge).
//! * The rails are girder groove rails set in the street (EN 14811:2006+A1:2009
//!   profile 59R2 / Ri59N, the common street-running tram rail): a running
//!   head 56 mm wide beside a 42 mm flangeway groove on its gauge side.
//! * Gauge is measured between the running heads' inner (gauge) faces.
//! The rails stand `lift` over the drawn paving (the street's own junction
//! paint lift, one step more), so they never fight its markings.

use crate::geometry::{Frozen, MeshArrays, Rgb};
use anyhow::Result;

/// Standard gauge (1435 mm) between the running heads' gauge faces, m.
pub const GAUGE_M: f64 = 1.435;
/// EN 14811 59R2 (Ri59N): the running head's width and the groove beside it, m.
pub const HEAD_M: f64 = 0.056;
pub const GROOVE_M: f64 = 0.042;
/// Track within this of the Ferry Building's tower (surveyed metres, before the plan scale).
pub const CORRIDOR_M: f64 = 1500.0;
/// The longest piece a rail is drawn in (it follows the paving's height between pieces), m.
pub const PIECE_M: f64 = 2.5;

/// The polished running head (iron's specular colour, linear; a metal's
/// albedo is its F0: Fe about 0.56, 0.57, 0.58) and the dirt-dark groove.
pub const HEAD_LINEAR: Rgb = [0.56, 0.57, 0.58];
pub const GROOVE_LINEAR: Rgb = [0.02, 0.019, 0.018];

/// The OSM tram and light-rail ways near the corridor, on the scaled plan.
pub fn load_corridor_ways(repo_root: &std::path::Path, centre_surveyed: [f64; 2]) -> Result<Vec<Vec<[f64; 2]>>> {
    use std::io::Read;
    #[derive(serde::Deserialize)]
    struct Way {
        #[serde(default)]
        railway: String,
        #[serde(default)]
        points: Vec<f64>,
    }
    #[derive(serde::Deserialize)]
    struct Rails {
        #[serde(rename = "railWays", default)]
        rail_ways: Vec<Way>,
    }
    let bytes = std::fs::read(repo_root.join("public/data/sf/sf-city.json.gz"))?;
    let mut s = String::new();
    flate2::read::GzDecoder::new(&bytes[..]).read_to_string(&mut s)?;
    let r: Rails = serde_json::from_str(&s)?;
    let k = eo_data::groundscale::scale();
    let mut out = Vec::new();
    for w in r.rail_ways {
        if !(w.railway == "tram" || w.railway == "light_rail") || w.points.len() < 4 {
            continue;
        }
        let pts: Vec<[f64; 2]> = w.points.chunks_exact(2).map(|c| [c[0], c[1]]).collect();
        if !pts.iter().any(|p| (p[0] - centre_surveyed[0]).hypot(p[1] - centre_surveyed[1]) <= CORRIDOR_M) {
            continue;
        }
        out.push(pts.into_iter().map(|p| [p[0] * k, p[1] * k]).collect());
    }
    Ok(out)
}

/// A polyline resampled to pieces of at most `step`, with a unit normal per point (left of travel).
fn resample(line: &[[f64; 2]], step: f64) -> Vec<([f64; 2], [f64; 2])> {
    let mut pts: Vec<[f64; 2]> = Vec::new();
    for w in line.windows(2) {
        let (a, b) = (w[0], w[1]);
        let l = (b[0] - a[0]).hypot(b[1] - a[1]);
        if l < 1e-6 {
            continue;
        }
        let n = (l / step).ceil().max(1.0) as usize;
        for i in 0..n {
            let t = i as f64 / n as f64;
            pts.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
        }
    }
    if let Some(l) = line.last() {
        pts.push(*l);
    }
    let m = pts.len();
    (0..m)
        .filter_map(|i| {
            let (p, q) = (pts[i.saturating_sub(1)], pts[(i + 1).min(m - 1)]);
            let (dx, dz) = (q[0] - p[0], q[1] - p[1]);
            let l = dx.hypot(dz);
            (l > 1e-9).then(|| (pts[i], [-dz / l, dx / l]))
        })
        .collect()
}

pub struct Tracks {
    /// The running heads (polished steel).
    pub heads: Frozen,
    /// The flangeway grooves.
    pub grooves: Frozen,
    pub tracks: usize,
    pub metres: f64,
}

/// Both rails of every way: a running head and its groove on the gauge side,
/// strips over the paving `surface_at` (x, z) gives, `lift` over it.
pub fn build(ways: &[Vec<[f64; 2]>], surface_at: &dyn Fn(f64, f64) -> f64, lift: f64) -> Tracks {
    let mut heads = MeshArrays::new("tram:heads");
    let mut grooves = MeshArrays::new("tram:grooves");
    let mut metres = 0.0;
    let half = GAUGE_M / 2.0;
    // lateral strips (from, to) off the centreline: each rail's head outside
    // the gauge face, its groove inside it.
    let head_strips = [(-half - HEAD_M, -half), (half, half + HEAD_M)];
    let groove_strips = [(-half, -half + GROOVE_M), (half - GROOVE_M, half)];
    for line in ways {
        let pts = resample(line, PIECE_M);
        for w in line.windows(2) {
            metres += (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
        }
        for pair in pts.windows(2) {
            let ((p0, n0), (p1, n1)) = (pair[0], pair[1]);
            let at = |p: [f64; 2], n: [f64; 2], o: f64| {
                let (x, z) = (p[0] + n[0] * o, p[1] + n[1] * o);
                [x, surface_at(x, z) + lift, z]
            };
            for (strips, m, col) in [(&head_strips, &mut heads, HEAD_LINEAR), (&groove_strips, &mut grooves, GROOVE_LINEAR)] {
                for (o0, o1) in strips.iter() {
                    let q = [at(p0, n0, *o0), at(p1, n1, *o0), at(p1, n1, *o1), at(p0, n0, *o1)];
                    // facing up whichever way the line runs
                    let e1 = [q[1][0] - q[0][0], q[1][2] - q[0][2]];
                    let e2 = [q[3][0] - q[0][0], q[3][2] - q[0][2]];
                    let up = e1[1] * e2[0] - e1[0] * e2[1] > 0.0;
                    if up { m.quad_n(q[0], q[1], q[2], q[3], col, [0.0, 1.0, 0.0]) } else { m.quad_n(q[3], q[2], q[1], q[0], col, [0.0, 1.0, 0.0]) }
                }
            }
        }
    }
    Tracks { heads: heads.freeze(), grooves: grooves.freeze(), tracks: ways.len(), metres }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_straight_track_has_both_rails_at_gauge_and_faces_up() {
        let t = build(&[vec![[0.0, 0.0], [10.0, 0.0]]], &|_, _| 1.0, 0.02);
        assert_eq!(t.tracks, 1);
        assert!((t.metres - 10.0).abs() < 1e-9);
        // 4 pieces of 2.5 m, two heads each, two triangles a quad.
        assert_eq!(t.heads.triangles(), 4 * 2 * 2);
        let zs: Vec<f32> = t.heads.positions.chunks_exact(3).map(|p| p[2]).collect();
        let (lo, hi) = (zs.iter().cloned().fold(f32::MAX, f32::min), zs.iter().cloned().fold(f32::MIN, f32::max));
        assert!(((hi - lo) as f64 - (GAUGE_M + 2.0 * HEAD_M)).abs() < 1e-5);
        assert!(t.heads.normals.chunks_exact(3).all(|n| n[1] == 1.0));
        assert!(t.heads.positions.chunks_exact(3).all(|p| (p[1] - 1.02).abs() < 1e-6));
        // the winding faces up
        for tri in t.heads.indices.chunks_exact(3) {
            let p = |i: u32| [t.heads.positions[3 * i as usize] as f64, t.heads.positions[3 * i as usize + 2] as f64];
            let (a, b, c) = (p(tri[0]), p(tri[1]), p(tri[2]));
            let cross_y = (b[1] - a[1]) * (c[0] - a[0]) - (b[0] - a[0]) * (c[1] - a[1]);
            assert!(cross_y > 0.0, "a rail triangle faces down");
        }
    }
}
