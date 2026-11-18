//! The street front of a San Francisco row house, read off its footprint
//! (src/world/buildings/archetypes/row-house-front.js): the front band of the
//! footprint flattened onto one street wall, with the canted bays and the
//! entry notch it had reported in the rebuilt front edge's own s.

use crate::frame::{Frame, FrameOptions, P2, footprint_frame, point_in_polygon};
use crate::js::*;

/// Depth of the front band flattened onto the street wall (m).
pub const FRONT_BAND_M: f64 = 4.5;
/// Deepest canted bay drawn in front of the wall (m).
pub const BAY_DEPTH_M: f64 = 0.8;
const BAY_MIN_FACE_M: f64 = 1.8;
const BAY_MIN_FACE_IN_ROW_M: f64 = 1.2;
const BAY_MAX_FACE_SHARE: f64 = 0.75;
const NOTCH_MIN_DEPTH_M: f64 = 1.2;
const PARALLEL_COS: f64 = 0.95;
const MAX_AREA_GROWTH: f64 = 0.3;
const AREA_NOISE: f64 = 0.03;

fn area(points: &[P2]) -> f64 {
    let n = points.len();
    let mut a = 0.0;
    for i in 0..n {
        let p = points[i];
        let q = points[(i + 1) % n];
        a += p.x * q.z - q.x * p.z;
    }
    a / 2.0
}

fn sign(v: f64) -> f64 {
    if v > 0.0 {
        1.0
    } else if v < 0.0 {
        -1.0
    } else {
        v
    }
}

fn segments_cross(a: P2, b: P2, c: P2, d: P2) -> bool {
    let o = |p: P2, q: P2, r: P2| sign((q.x - p.x) * (r.z - p.z) - (q.z - p.z) * (r.x - p.x));
    o(a, b, c) * o(a, b, d) < 0.0 && o(c, d, a) * o(c, d, b) < 0.0
}

fn is_simple(points: &[P2]) -> bool {
    let n = points.len();
    for i in 0..n {
        for j in (i + 2)..n {
            if i == 0 && j == n - 1 {
                continue;
            }
            if segments_cross(points[i], points[(i + 1) % n], points[j], points[(j + 1) % n]) {
                return false;
            }
        }
    }
    true
}

/// Drop repeated and collinear vertices.
fn simplify(points: &[P2]) -> Vec<P2> {
    let mut out = points.to_vec();
    let mut changed = true;
    while changed && out.len() > 3 {
        changed = false;
        let mut i: i64 = 0;
        while (i as usize) < out.len() && out.len() > 3 {
            let n = out.len() as i64;
            let a = out[((i - 1 + n) % n) as usize];
            let b = out[i as usize];
            let c = out[((i + 1) % n) as usize];
            let cross = (b.x - a.x) * (c.z - b.z) - (b.z - a.z) * (c.x - b.x);
            if (b.x - a.x).hypot(b.z - a.z) < 0.05 || cross.abs() < 1e-3 {
                out.remove(i as usize);
                changed = true;
                i -= 1;
            }
            i += 1;
        }
    }
    out
}

struct Run {
    a0: f64,
    a1: f64,
    d: f64,
    len: f64,
}

/// `rowHouseFront(frame, { allowance })`: the regularised frame, its bays and notch.
pub fn row_house_front(frame: &Frame, allowance: f64) -> Option<(Frame, J, J)> {
    let front = frame.edge_for_role("front")?;
    let polygon = &frame.polygon;
    if polygon.len() < 4 {
        return None;
    }
    let o = front.start;
    let u = front.u;
    let n = front.n;
    let a_of = |p: P2| (p.x - o.x) * u.x + (p.z - o.z) * u.z;
    let d_of = |p: P2| (p.x - o.x) * n.x + (p.z - o.z) * n.z;
    let ds: Vec<f64> = polygon.iter().map(|p| d_of(*p)).collect();
    let d_max = ds.iter().fold(f64::NEG_INFINITY, |m, d| m.max(*d));
    let in_band: Vec<bool> = ds.iter().map(|d| *d >= d_max - FRONT_BAND_M).collect();
    let band_as: Vec<f64> = polygon.iter().enumerate().filter(|(i, _)| in_band[*i]).map(|(_, p)| a_of(*p)).collect();
    let lot_width = band_as.iter().fold(f64::NEG_INFINITY, |m, v| m.max(*v)) - band_as.iter().fold(f64::INFINITY, |m, v| m.min(*v));
    if !(lot_width > 2.0) {
        return None;
    }

    let mut along: Vec<Run> = vec![];
    let np = polygon.len();
    for i in 0..np {
        let j = (i + 1) % np;
        if !in_band[i] || !in_band[j] {
            continue;
        }
        let p = polygon[i];
        let q = polygon[j];
        let len = (q.x - p.x).hypot(q.z - p.z);
        if len < 0.3 {
            continue;
        }
        let cos = (((q.x - p.x) * u.x + (q.z - p.z) * u.z) / len).abs();
        if cos < PARALLEL_COS {
            continue;
        }
        along.push(Run { a0: a_of(p).min(a_of(q)), a1: a_of(p).max(a_of(q)), d: (ds[i] + ds[j]) / 2.0, len });
    }
    let mut front0: Vec<&Run> = along.iter().filter(|e| e.d >= d_max - 0.15).collect();
    front0.sort_by(|a, b| a.a0.partial_cmp(&b.a0).unwrap_or(std::cmp::Ordering::Equal));
    let min_face = if front0.len() > 1 { BAY_MIN_FACE_IN_ROW_M } else { BAY_MIN_FACE_M };
    let faces: Vec<&Run> = front0.iter().copied().filter(|e| e.len >= min_face).collect();
    let behind: Vec<&Run> = along.iter().filter(|e| e.d < d_max - 0.3).collect();
    let mut bays: Vec<(f64, f64, f64)> = vec![];
    let mut wall = d_max;
    let face_total = faces.iter().fold(0.0, |t, e| t + e.len);
    if !faces.is_empty() && faces.len() == front0.len() && face_total <= BAY_MAX_FACE_SHARE * lot_width {
        let deepest = polygon.iter().enumerate().filter(|(i, _)| in_band[*i]).map(|(_, p)| d_of(*p)).fold(f64::INFINITY, f64::min);
        let depth = BAY_DEPTH_M.min(d_max - deepest);
        if depth >= 0.3 {
            wall = d_max - depth;
            bays = faces.iter().map(|e| (e.a0, e.a1, depth)).collect();
        }
    }
    let mut notch_runs: Vec<&Run> = behind.iter().copied().filter(|e| e.d <= wall - NOTCH_MIN_DEPTH_M).collect();
    notch_runs.sort_by(|a, b| a.d.partial_cmp(&b.d).unwrap_or(std::cmp::Ordering::Equal));
    let notch = notch_runs.first().map(|r| (r.a0, r.a1));

    let moved: Vec<P2> = polygon
        .iter()
        .enumerate()
        .map(|(i, p)| if in_band[i] { P2::new(p.x + n.x * (wall - ds[i]), p.z + n.z * (wall - ds[i])) } else { *p })
        .collect();
    let flat = simplify(&moved);
    if flat.len() < 3 || !is_simple(&flat) {
        return None;
    }
    let span = |pts: &[P2]| {
        let a: Vec<f64> = pts.iter().map(|p| a_of(*p)).collect();
        a.iter().fold(f64::NEG_INFINITY, |m, v| m.max(*v)) - a.iter().fold(f64::INFINITY, |m, v| m.min(*v))
    };
    if span(&flat) < span(polygon) - 0.1 {
        return None;
    }
    let before = area(polygon).abs();
    let after = area(&flat).abs();
    let bay_area = bays.iter().fold(0.0, |t, (a0, a1, d)| t + (a1 - a0 + d) * d);
    if !(after >= before * (1.0 - AREA_NOISE) - bay_area && after <= before * (1.0 + MAX_AREA_GROWTH)) {
        return None;
    }

    let opts = FrameOptions { allowance: Some(allowance), ..Default::default() };
    let probe = footprint_frame(&flat, 0, &opts)?;
    let mut index: i64 = -1;
    let mut best = 0.0;
    for e in &probe.edges {
        let dm = d_of(e.midpoint);
        let cos = (e.u.x * u.x + e.u.z * u.z).abs();
        if (dm - wall).abs() < 0.05 && cos > PARALLEL_COS && e.length > best && e.n.x * n.x + e.n.z * n.z > 0.0 {
            best = e.length;
            index = e.index as i64;
        }
    }
    if index < 0 || best < 0.6 * lot_width {
        return None;
    }
    let mut rebuilt = footprint_frame(&flat, index as usize, &opts)?;
    let edge = rebuilt.edges[index as usize].clone();
    let same_dir = edge.u.x * u.x + edge.u.z * u.z > 0.0;
    let start_a = a_of(edge.start);
    let to_s = |a: f64| if same_dir { a - start_a } else { start_a - a };
    let range = |a0: f64, a1: f64| {
        let s0 = to_s(a0);
        let s1 = to_s(a1);
        (s0.min(s1), s0.max(s1))
    };
    let notch_s = notch.map(|(a0, a1)| range(a0, a1));
    let notch_ok = match notch_s {
        Some((s0, s1)) => {
            let mid = (s0 + s1) / 2.0;
            mid > 0.0
                && mid < edge.length
                && !point_in_polygon(edge.start.x + edge.u.x * mid - edge.n.x * 0.3, edge.start.z + edge.u.z * mid - edge.n.z * 0.3, polygon)
        }
        None => false,
    };
    rebuilt.source_polygon = Some(match &frame.source_polygon {
        Some(p) => p.clone(),
        None => frame.edges.iter().map(|e| P2::new(e.start.x, e.start.z)).collect(),
    });
    let mut bay_list: Vec<(f64, f64, f64)> = bays.iter().map(|(a0, a1, d)| {
        let (s0, s1) = range(*a0, *a1);
        (s0, s1, *d)
    }).collect();
    bay_list.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let bays_j = J::Array(bay_list.iter().map(|(s0, s1, d)| json!({ "s0": s0, "s1": s1, "depth": d })).collect());
    let notch_j = match (notch_ok, notch_s) {
        (true, Some((s0, s1))) => json!({ "s0": s0, "s1": s1 }),
        _ => J::Null,
    };
    Some((rebuilt, bays_j, notch_j))
}
