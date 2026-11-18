//! The block overlays' kerb windows (src/world/streets/observed/
//! folsom-4th-5th.js: `observedOverlayFor`, `observedAxisPoint`,
//! `observedSegmentSide`, `observedStationOnSegment`, `observedSegmentWindows`)
//! and street-furniture.js's readers of them (`observedWindows`,
//! `observedAbsentWindows`, `observedAbsentAt`, `worldPointOnSegmentSide`).

use super::data::{Overlay, OverlayEntry};
use crate::station_at;
use eo_street::plan::{Entry, Frame};

pub struct AxisPoint {
    pub x: f64,
    pub z: f64,
    pub tx: f64,
    pub tz: f64,
    pub nx: f64,
    pub nz: f64,
}

/// `observedAxisPoint(overlay, s, u)`.
pub fn axis_point(o: &Overlay, s: f64, u: f64) -> AxisPoint {
    let (f, t) = (o.axis.from, o.axis.to);
    let l = (t.x - f.x).hypot(t.z - f.z);
    let l = if l == 0.0 { 1.0 } else { l };
    let (tx, tz) = ((t.x - f.x) / l, (t.z - f.z) / l);
    AxisPoint { x: f.x + tx * s + tz * u, z: f.z + tz * s - tx * u, tx, tz, nx: tz, nz: -tx }
}

/// `observedSegmentSide(overlay, frame, sideName)`.
pub fn segment_side(o: &Overlay, frame: &Frame, side_name: Option<&str>) -> f64 {
    let p = axis_point(o, 0.0, 0.0);
    let plus_is_north = frame.nx * p.nx + frame.nz * p.nz >= 0.0;
    if (side_name == Some("N")) == plus_is_north { 1.0 } else { -1.0 }
}

/// `observedStationOnSegment(segment, overlay, s)`.
pub fn station_on_segment(e: &Entry, o: &Overlay, s: f64) -> Option<f64> {
    let p = axis_point(o, s, 0.0);
    if e.points.len() < 2 {
        return None;
    }
    let mut best: Option<(f64, f64)> = None;
    for i in 0..e.points.len() - 1 {
        let (a, b) = (e.points[i], e.points[i + 1]);
        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
        let l2 = dx * dx + dz * dz;
        if !(l2 > 1e-9) {
            continue;
        }
        let t = ((p.x - a[0]) * dx + (p.z - a[1]) * dz) / l2;
        if t < -1e-6 || t > 1.0 + 1e-6 {
            continue;
        }
        let tc = t.clamp(0.0, 1.0);
        let d = (p.x - (a[0] + dx * tc)).hypot(p.z - (a[1] + dz * tc));
        if best.is_some_and(|b| d >= b.0) {
            continue;
        }
        best = Some((d, e.cum[i] + l2.sqrt() * tc));
    }
    best.filter(|b| b.0 < 12.0).map(|b| b.1)
}

pub struct Window<'a> {
    pub side: f64,
    pub lo: f64,
    pub hi: f64,
    pub entry: &'a OverlayEntry,
}

/// `observedOverlayFor(segment)`: by the street contract's `streetId`.
pub fn overlay_for<'a>(overlays: &'a [Overlay], street_id: Option<&str>) -> Option<&'a Overlay> {
    let id = street_id.filter(|s| !s.is_empty())?;
    overlays.iter().find(|o| o.street_ids.iter().any(|s| s == id))
}

/// `observedWindows(segment, side, key)` (street-furniture.js).
pub fn windows<'a>(o: Option<&'a Overlay>, e: &Entry, side: f64, key: &str) -> Vec<Window<'a>> {
    let Some(o) = o else { return vec![] };
    let list = o.list(key);
    if list.is_empty() {
        return vec![];
    }
    let frame = station_at(e, e.length / 2.0, false);
    let mut out = vec![];
    for entry in list {
        if segment_side(o, &frame, entry.side.as_deref()) != side {
            continue;
        }
        let (Some(s0), Some(s1)) = (entry.s0.or(entry.s), entry.s1.or(entry.s)) else { continue };
        let (Some(a), Some(b)) = (station_on_segment(e, o, s0), station_on_segment(e, o, s1)) else { continue };
        out.push(Window { side, lo: a.min(b), hi: a.max(b), entry });
    }
    out
}

/// `observedSegmentWindows(segment, key, frame)`.
pub fn segment_windows<'a>(o: &'a Overlay, e: &Entry, key: &str, frame: &Frame) -> Vec<Window<'a>> {
    let list = o.list(key);
    if list.is_empty() || e.points.len() < 2 || !frame.nx.is_finite() {
        return vec![];
    }
    let p = axis_point(o, 0.0, 0.0);
    let from = o.axis.from;
    let axis_s: Vec<f64> = e.points.iter().map(|q| (q[0] - from.x) * p.tx + (q[1] - from.z) * p.tz).collect();
    let s_min = axis_s.iter().copied().fold(f64::INFINITY, f64::min);
    let s_max = axis_s.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let to_station = |s: f64| -> Option<f64> {
        for i in 0..e.points.len() - 1 {
            let (a, b) = (axis_s[i], axis_s[i + 1]);
            if (s - a) * (s - b) <= 0.0 && (b - a).abs() > 1e-9 {
                return Some(e.cum[i] + (e.cum[i + 1] - e.cum[i]) * ((s - a) / (b - a)));
            }
        }
        None
    };
    let mut out = vec![];
    for entry in list {
        let (Some(e0), Some(e1)) = (entry.s0.or(entry.s), entry.s1.or(entry.s)) else { continue };
        let lo = e0.min(e1).max(s_min);
        let hi = e0.max(e1).min(s_max);
        if !(hi >= lo) {
            continue;
        }
        let (Some(a), Some(b)) = (to_station(lo), to_station(hi)) else { continue };
        out.push(Window { side: segment_side(o, frame, entry.side.as_deref()), lo: a.min(b), hi: a.max(b), entry });
    }
    out
}

/// `observedAbsentWindows(segment, side)`: None without an overlay or a window on this side.
pub fn absent_windows<'a>(o: Option<&'a Overlay>, e: &Entry, side: f64) -> Option<Vec<Window<'a>>> {
    let o = o?;
    let frame = station_at(e, e.length / 2.0, false);
    let list: Vec<Window> = segment_windows(o, e, "furnitureAbsent", &frame).into_iter().filter(|w| w.side == side).collect();
    (!list.is_empty()).then_some(list)
}

/// `observedAbsentAt(windows, kindId, station)`.
pub fn absent_at(windows: &[Window], kind: &str, station: f64) -> bool {
    windows.iter().any(|w| station >= w.lo && station <= w.hi && w.entry.kinds.as_ref().is_some_and(|k| k.iter().any(|x| x == kind)))
}

/// `worldPointOnSegmentSide(segment, x, z)`: (offset, station, side).
pub fn world_point_on_segment_side(e: &Entry, x: f64, z: f64) -> Option<(f64, f64, f64)> {
    if e.points.len() < 2 {
        return None;
    }
    let mut best: Option<(f64, f64)> = None;
    for i in 0..e.points.len() - 1 {
        let (a, b) = (e.points[i], e.points[i + 1]);
        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
        let l2 = dx * dx + dz * dz;
        if !(l2 > 1e-9) {
            continue;
        }
        let t = ((x - a[0]) * dx + (z - a[1]) * dz) / l2;
        if !(0.0..=1.0).contains(&t) {
            continue;
        }
        let d = (x - (a[0] + dx * t)).hypot(z - (a[1] + dz * t));
        if best.is_some_and(|b| d >= b.0) {
            continue;
        }
        best = Some((d, e.cum[i] + l2.sqrt() * t));
    }
    let (offset, station) = best?;
    let f = station_at(e, station, false);
    let side = if (x - f.x) * f.nx + (z - f.z) * f.nz >= 0.0 { 1.0 } else { -1.0 };
    Some((offset, station, side))
}
