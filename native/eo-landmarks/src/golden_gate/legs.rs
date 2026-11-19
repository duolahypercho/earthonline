//! The tower legs' cellular section (`BridgeFeatures::leg_cells`).
//!
//! Each leg is built of riveted cells 3.5 ft square (42 in web plates and
//! 8 x 8 in angles), 15 x 9 cells at the base and 7 x 3 at the top, the
//! section shrinking "by successive omissions of outside cells"; the cells'
//! re-entrant corners and the stopped-off cells read on the faces as
//! Morrow's "simple vertical fluting ... of intersecting plane facets", the
//! towers' only ornament (Strauss 1937; the District calls it "vertical
//! fluting on the towers": goldengate.org, "Art Deco on a Grand Scale").
//!
//! Drawn: every face of a leg stage divided into whole cells (the stage's
//! width over the cell, rounded); a pilaster one cell wide at each end of
//! the face, recessed panels between them no wider than `PANEL_MAX_CELLS`,
//! split by one-cell pilasters, and each corner stepped in by half a cell
//! both ways (the re-entrant corner). INFERRED: the panels' widths and
//! depth and the corner step (no drawing of the faces was found; the cell is
//! the published module). The outline stays inside the stage's box, so the
//! silhouette, the casings' set-back and the floods' tables are unchanged.
//! The same rule is mirrored in eo-render landmarks_surface.wgsl
//! (`lm_leg_face`) for the panels' own sun shadow and sky occlusion past the
//! shadow maps' reach.

use super::{MeshArrays, Rgb};
use crate::geometry::Faces;

/// The cell (Strauss 1937), feet; drawn times the structure scale.
pub const CELL_FT: f64 = 3.5;
/// INFERRED: a panel's recess and the corner's step, in cells; the widest
/// panel between pilasters, in cells.
pub const RECESS_CELLS: f64 = 0.25;
pub const NOTCH_CELLS: f64 = 0.5;
pub const PANEL_MAX_CELLS: f64 = 3.0;

/// The drawn cell, metres (the structure scale `h`).
pub fn cell_m(h: f64) -> f64 {
    CELL_FT * 0.3048 * h
}

/// A face `half` either side of its centre, as (from, to, recessed) runs
/// from -half to +half: end pilasters one cell wide, panels between them of
/// at most `PANEL_MAX_CELLS` cells split by one-cell pilasters. The cell is
/// the face's width over its whole number of cells (at least 3).
pub fn face_runs(half: f64, cell: f64) -> Vec<(f64, f64, bool)> {
    let w = 2.0 * half;
    let n = (w / cell).round().max(3.0);
    let m = w / n;
    let panels = ((n - 1.0) / (PANEL_MAX_CELLS + 1.0)).ceil().max(1.0);
    let wp = (n - (panels + 1.0)) / panels;
    let mut out = Vec::new();
    let mut x = -half;
    for i in 0..panels as usize {
        out.push((x, x + m, false));
        x += m;
        out.push((x, x + wp * m, true));
        x += wp * m;
        let _ = i;
    }
    out.push((x, half, false));
    out
}

/// The stage's outline in its own frame (c along the span, s across it),
/// counter-clockwise from the (+c, -s) corner: the faces at c = +-ha and
/// s = +-hc each as `face_runs`, recessed runs `recess` in, every corner
/// stepped `notch` in both ways.
pub fn cell_outline(ha: f64, hc: f64, cell: f64) -> Vec<[f64; 2]> {
    let (recess, notch) = (RECESS_CELLS * cell, NOTCH_CELLS * cell);
    let mut pts: Vec<[f64; 2]> = Vec::new();
    // each face: its outward normal (nc, ns), its tangent (tc, ts) running counter-clockwise, its half width and its offset.
    let faces = [([1.0, 0.0], [0.0, 1.0], hc, ha), ([0.0, 1.0], [-1.0, 0.0], ha, hc), ([-1.0, 0.0], [0.0, -1.0], hc, ha), ([0.0, -1.0], [1.0, 0.0], ha, hc)];
    for (nrm, tan, half, off) in faces {
        let at = |t: f64, depth: f64| [nrm[0] * (off - depth) + tan[0] * t, nrm[1] * (off - depth) + tan[1] * t];
        // the corner step at the face's start: in from the previous face's plane.
        pts.push(at(-half + notch, notch));
        pts.push(at(-half + notch, 0.0));
        for (a, b, rec) in face_runs(half, cell) {
            let (a, b) = (a.max(-half + notch), b.min(half - notch));
            if b - a < 1e-6 {
                continue;
            }
            if rec {
                pts.push(at(a, 0.0));
                pts.push(at(a, recess));
                pts.push(at(b, recess));
                pts.push(at(b, 0.0));
            }
        }
        pts.push(at(half - notch, 0.0));
        pts.push(at(half - notch, notch));
    }
    // drop repeated points (a run ending on the corner step).
    let mut out: Vec<[f64; 2]> = Vec::with_capacity(pts.len());
    for p in pts {
        if out.last().is_none_or(|q: &[f64; 2]| (q[0] - p[0]).hypot(q[1] - p[1]) > 1e-6) {
            out.push(p);
        }
    }
    if out.len() > 1 && (out[0][0] - out[out.len() - 1][0]).hypot(out[0][1] - out[out.len() - 1][1]) < 1e-6 {
        out.pop();
    }
    out
}

/// One leg stage as its cellular section: the outline about (x, z) on the
/// leg's yaw, extruded from y0 to y1, its top (and, with `faces.bottom`, its
/// foot) closed by a fan from the centre.
#[allow(clippy::too_many_arguments)]
pub fn cell_stage(m: &mut MeshArrays, x: f64, z: f64, y0: f64, y1: f64, half_along: f64, half_across: f64, yaw: f64, cell: f64, colour: Rgb, faces: Faces) {
    let (cy, sy) = (yaw.cos(), yaw.sin());
    let local = cell_outline(half_along, half_across, cell);
    let ring: Vec<[f64; 2]> = local.iter().map(|&[c, s]| [x + c * cy - s * sy, z + c * sy + s * cy]).collect();
    let ring = crate::geometry::orient_ring(&ring);
    let n = ring.len();
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        m.quad([a[0], y0, a[1]], [b[0], y0, b[1]], [b[0], y1, b[1]], [a[0], y1, a[1]], colour);
        if faces.top {
            m.tri([a[0], y1, a[1]], [b[0], y1, b[1]], [x, y1, z], colour);
        }
        if faces.bottom {
            m.tri([b[0], y0, b[1]], [a[0], y0, a[1]], [x, y0, z], colour);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The faces fill their width; the top stage's 3-cell face is pilaster,
    /// panel, pilaster; the base's 15 cells take four panels.
    #[test]
    fn the_faces_are_whole_cells() {
        let cell = cell_m(1.25);
        assert!((cell - 1.3335).abs() < 1e-4, "{cell}");
        for cells in [3.0, 5.0, 7.0, 9.0, 11.0, 13.0, 15.0] {
            let half = cells * cell / 2.0;
            let runs = face_runs(half, cell);
            assert!((runs[0].0 + half).abs() < 1e-9 && (runs[runs.len() - 1].1 - half).abs() < 1e-9);
            for w in runs.windows(2) {
                assert!((w[0].1 - w[1].0).abs() < 1e-9);
            }
            let panels = runs.iter().filter(|r| r.2).count();
            let widest = runs.iter().filter(|r| r.2).map(|r| (r.1 - r.0) / cell).fold(0.0, f64::max);
            assert!(widest <= PANEL_MAX_CELLS + 1e-9 && widest >= 1.0 - 1e-9, "{cells}: {widest}");
            if cells == 3.0 {
                assert_eq!(panels, 1);
            }
            if cells == 15.0 {
                assert_eq!(panels, 4);
            }
        }
    }

    /// The outline stays inside the stage's box and reaches its faces.
    #[test]
    fn the_outline_stays_in_the_stage() {
        let cell = cell_m(1.25);
        let (ha, hc) = (7.5 * cell, 4.5 * cell);
        let o = cell_outline(ha, hc, cell);
        let (mut mc, mut ms) = (0.0f64, 0.0f64);
        for p in &o {
            assert!(p[0].abs() <= ha + 1e-9 && p[1].abs() <= hc + 1e-9);
            mc = mc.max(p[0].abs());
            ms = ms.max(p[1].abs());
        }
        assert!((mc - ha).abs() < 1e-9 && (ms - hc).abs() < 1e-9);
        assert!(o.len() > 40, "{}", o.len());
    }
}
