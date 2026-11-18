//! The open-sea grid: `buildGridGeometry` of src/render/materials/
//! ocean-open-sea.js (one graded square grid around the camera, coarser with
//! distance: axis position `R (L t + (1 - L) t |t|)`, moved in snap steps so
//! the lattice does not crawl over the waves), cut into square patches so the
//! GPU can cull them one by one (frustum, land, occlusion) and draw the rest
//! with one multi-draw.
//!
//! What differs from the JS, and why: the JS grid is 2,400 m in radius at 448
//! segments; the game camera's far plane is 4,200 m, so past 2,400 m the JS
//! open sea simply ends. Here the radius is chosen from the camera's height
//! and far plane (at least `MIN_RADIUS_M`, which covers the game's far plane;
//! more from high up or for a camera that sees further, so the bay is whole
//! from the air) and the grid has 512
//! segments, so the vertex spacing at street level is 2.1 m near the camera
//! (JS 1.3 m) and 32 m at the rim (JS 20 m). The FFT bands a vertex cannot
//! carry are left out of its displacement exactly as the JS leaves them out
//! (`cascadeBandWeights` at the vertex spacing).

/// Segments per side (a multiple of `PATCH_QUADS`). 1024, so the
/// inner zone carries the breaking surf (0.8 m apart within 250 m).
pub const SEGMENTS: u32 = 1024;
/// Quads per patch side.
pub const PATCH_QUADS: u32 = 16;
/// Patches per side.
pub const PATCHES: u32 = SEGMENTS / PATCH_QUADS;
/// `GRID_LINEAR`: linear share of the axis mapping.
pub const LINEAR: f64 = 0.12;
/// Radius at street level: past the game's 4,200 m far plane.
pub const MIN_RADIUS_M: f64 = 4400.0;
/// `SNAP_M` at the JS radius; the snap scales with the radius.
pub const JS_RADIUS_M: f64 = 2400.0;
pub const JS_SNAP_M: f64 = 32.0;
/// Vertical reach of the displaced surface (the patch box's half height).
pub const DISPLACEMENT_BOX_M: f64 = 8.0;

// a two-zone axis instead of the JS's `R (L t + (1 - L) t |t|)`:
// the inner `INNER_SHARE` of each half axis spaced evenly over
// `INNER_RADIUS_SHARE` of the radius (0.81 m at street level, enough for a
// breaking wave's face), then spacing growing linearly to the rim (40 m), the
// slope continuous at the joint. `LINEAR` is kept for reference only.
/// Share of each half axis's grid lines in the even inner zone.
pub const INNER_SHARE: f64 = 0.6;
/// The inner zone's half width as a share of the radius (250 m at 4,400 m).
pub const INNER_RADIUS_SHARE: f64 = 250.0 / 4400.0;

fn zones(r: f64) -> (f64, f64, f64) {
    let rin = r * INNER_RADIUS_SHARE;
    let s0 = rin / INNER_SHARE;
    let o = 1.0 - INNER_SHARE;
    (rin, s0, (r - rin - s0 * o) / (o * o))
}

/// Axis position of grid line `i` (0..=SEGMENTS) at radius `r`.
pub fn axis(i: u32, r: f64) -> f64 {
    let t = (i as f64 / SEGMENTS as f64) * 2.0 - 1.0;
    let (rin, s0, k) = zones(r);
    let u = t.abs();
    let c = if u <= INNER_SHARE { s0 * u } else { rin + s0 * (u - INNER_SHARE) + k * (u - INNER_SHARE).powi(2) };
    c * t.signum()
}

/// Vertex spacing at local coordinate `c` (the inverse of the axis map's slope).
pub fn spacing_at(c: f64, r: f64) -> f64 {
    let (rin, s0, k) = zones(r);
    let a = c.abs();
    let slope = if a <= rin { s0 } else { s0 + 2.0 * k * ((-s0 + (s0 * s0 + 4.0 * k * (a - rin)).sqrt()) / (2.0 * k)) };
    slope * (2.0 / SEGMENTS as f64)
}

#[cfg(test)]
mod sea_tests {
    use super::*;

    #[test]
    fn two_zone_axis_is_monotone_and_reaches_the_rim() {
        let r = MIN_RADIUS_M;
        assert!((axis(SEGMENTS, r) - r).abs() < 1e-6 && (axis(0, r) + r).abs() < 1e-6);
        assert!(axis(SEGMENTS / 2, r).abs() < 1e-9);
        for i in 0..SEGMENTS {
            let (a, b) = (axis(i, r), axis(i + 1, r));
            assert!(b > a);
            // the spacing function matches the lattice within 1 %
            let s = spacing_at((a + b) * 0.5, r);
            assert!(((b - a) - s).abs() <= 0.01 * s + 1e-6, "line {i}: step {} against {s}", b - a);
        }
        assert!(spacing_at(100.0, r) < 0.85);
    }
}

/// The radius for a camera `height` metres above the sea whose far plane is
/// `far` metres out: one of a few fixed sizes (a continuous radius would make
/// the lattice crawl), with hysteresis. It covers the far plane (so the sea
/// does not end inside the view when a camera sees past the game's 4,200 m)
/// and, from high up, 2.6 times the height.
pub fn radius_for(height: f64, far: f64, current: f64) -> f64 {
    // from high up the sea runs on past the far plane into the haze (the
    // water pass draws past it): ten times the height reaches the haze from
    // the air (1.6 km up: 17.6 km).
    let want = (height * 10.0).max(far).max(MIN_RADIUS_M);
    let mut r = MIN_RADIUS_M;
    while r < want {
        r *= 2.0;
    }
    // Keep a larger current radius until the need falls well below it.
    if current > r && want > current * 0.35 {
        return current;
    }
    r
}

/// The snap step at a radius: the JS's 32 m at 2,400 m, scaled, a power of two.
pub fn snap_for(r: f64) -> f64 {
    let s = JS_SNAP_M * r / JS_RADIUS_M;
    2f64.powf(s.log2().round())
}

/// The grid's offset for a camera at (x, z).
pub fn offset_for(x: f64, z: f64, snap: f64) -> [f64; 2] {
    [(x / snap).round() * snap, (z / snap).round() * snap]
}

/// The index list of one patch: (PATCH_QUADS + 1)^2 vertices numbered row by
/// row, two triangles a quad, counter-clockwise seen from above (+Y), as the
/// JS grid winds them.
pub fn patch_indices() -> Vec<u32> {
    let n = PATCH_QUADS;
    let side = n + 1;
    let mut out = Vec::with_capacity((n * n * 6) as usize);
    for j in 0..n {
        for i in 0..n {
            let a = j * side + i;
            let b = a + 1;
            let c = a + side;
            let d = c + 1;
            out.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    out
}
