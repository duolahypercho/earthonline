//! Plan polygons onto a texel grid.

/// A texel grid on the surveyed plan (x east, z south, metres): texel (i, j)
/// covers x in origin_x + i*texel .. +texel and z in origin_z + j*texel .. +texel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanGrid {
    pub origin_x: f64,
    pub origin_z: f64,
    pub texel: f64,
    pub width: usize,
    pub height: usize,
}

impl PlanGrid {
    /// The plan point at texel (i, j)'s centre.
    pub fn centre(&self, i: usize, j: usize) -> (f64, f64) {
        (self.origin_x + (i as f64 + 0.5) * self.texel, self.origin_z + (j as f64 + 0.5) * self.texel)
    }
    /// The texel holding plan point (x, z), if on the grid.
    pub fn texel_of(&self, x: f64, z: f64) -> Option<(usize, usize)> {
        let (fi, fj) = ((x - self.origin_x) / self.texel, (z - self.origin_z) / self.texel);
        (fi >= 0.0 && fj >= 0.0 && (fi as usize) < self.width && (fj as usize) < self.height).then(|| (fi as usize, fj as usize))
    }
}

/// Call `set(i, j)` for every texel whose centre lies inside `rings` (even-odd
/// over all of them: an outer ring and its holes). Each ring is a closed
/// polygon without its closing duplicate.
pub fn fill_rings(g: &PlanGrid, rings: &[&[[f64; 2]]], mut set: impl FnMut(usize, usize)) {
    let (mut z0, mut z1) = (f64::MAX, f64::MIN);
    for r in rings {
        for p in r.iter() {
            z0 = z0.min(p[1]);
            z1 = z1.max(p[1]);
        }
    }
    if z0 > z1 {
        return;
    }
    // Texel rows whose centre z lies in [z0, z1].
    let j0 = ((z0 - g.origin_z) / g.texel - 0.5).ceil().max(0.0) as usize;
    let j1 = (((z1 - g.origin_z) / g.texel - 0.5).floor()).min(g.height as f64 - 1.0);
    if j1 < 0.0 {
        return;
    }
    let mut xs: Vec<f64> = vec![];
    for j in j0..=j1 as usize {
        let z = g.origin_z + (j as f64 + 0.5) * g.texel;
        xs.clear();
        for r in rings {
            let n = r.len();
            for k in 0..n {
                let (a, b) = (r[k], r[(k + 1) % n]);
                if (a[1] > z) != (b[1] > z) {
                    xs.push(a[0] + (z - a[1]) * (b[0] - a[0]) / (b[1] - a[1]));
                }
            }
        }
        xs.sort_by(f64::total_cmp);
        for span in xs.chunks_exact(2) {
            // Texel columns whose centre x lies in [span[0], span[1]).
            let i0 = ((span[0] - g.origin_x) / g.texel - 0.5).ceil().max(0.0);
            let i1 = ((span[1] - g.origin_x) / g.texel - 0.5).ceil().min(g.width as f64);
            let mut i = i0;
            while i < i1 {
                set(i as usize, j);
                i += 1.0;
            }
        }
    }
}

/// Call `set(i, j)` for every texel whose centre lies within `radius` of the
/// polyline `pts` (a texel may be set more than once).
pub fn fill_corridor(g: &PlanGrid, pts: &[[f64; 2]], radius: f64, mut set: impl FnMut(usize, usize)) {
    let segs: Vec<([f64; 2], [f64; 2])> = if pts.len() == 1 { vec![(pts[0], pts[0])] } else { pts.windows(2).map(|w| (w[0], w[1])).collect() };
    for (a, b) in segs {
        let (x0, x1) = (a[0].min(b[0]) - radius, a[0].max(b[0]) + radius);
        let (z0, z1) = (a[1].min(b[1]) - radius, a[1].max(b[1]) + radius);
        let i0 = ((x0 - g.origin_x) / g.texel - 0.5).ceil().max(0.0);
        let i1 = ((x1 - g.origin_x) / g.texel - 0.5).floor().min(g.width as f64 - 1.0);
        let j0 = ((z0 - g.origin_z) / g.texel - 0.5).ceil().max(0.0);
        let j1 = ((z1 - g.origin_z) / g.texel - 0.5).floor().min(g.height as f64 - 1.0);
        if i1 < i0 || j1 < j0 {
            continue;
        }
        let (ex, ez) = (b[0] - a[0], b[1] - a[1]);
        let l2 = ex * ex + ez * ez;
        for j in j0 as usize..=j1 as usize {
            for i in i0 as usize..=i1 as usize {
                let (x, z) = g.centre(i, j);
                let t = if l2 > 0.0 { (((x - a[0]) * ex + (z - a[1]) * ez) / l2).clamp(0.0, 1.0) } else { 0.0 };
                if (x - a[0] - t * ex).hypot(z - a[1] - t * ez) <= radius {
                    set(i, j);
                }
            }
        }
    }
}

/// The texels of `mask` (row-major, `w` wide) whose 8 neighbours are in it
/// too: the texels fully inside a region. The border row and column are out.
pub fn erode(mask: &[bool], w: usize, h: usize) -> Vec<bool> {
    let mut out = vec![false; w * h];
    for j in 1..h.saturating_sub(1) {
        for i in 1..w.saturating_sub(1) {
            out[j * w + i] = (j - 1..=j + 1).all(|jj| mask[jj * w + i - 1..=jj * w + i + 1].iter().all(|&m| m));
        }
    }
    out
}

/// The rings of a flat `[x, z, x, z, ...]` polygon list, closing duplicate dropped.
pub fn ring_of_flat(flat: &[f64]) -> Vec<[f64; 2]> {
    let mut r: Vec<[f64; 2]> = flat.chunks_exact(2).map(|p| [p[0], p[1]]).collect();
    if r.len() > 1 && r.first() == r.last() {
        r.pop();
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texel_centre_round_trip() {
        let g = PlanGrid { origin_x: -7000.0, origin_z: -5000.0, texel: 2.0, width: 100, height: 50 };
        for (i, j) in [(0, 0), (99, 49), (37, 12)] {
            let (x, z) = g.centre(i, j);
            assert_eq!(g.texel_of(x, z), Some((i, j)));
        }
        assert_eq!(g.texel_of(-7000.1, -4999.0), None);
        assert_eq!(g.texel_of(-7000.0 + 200.0, -4999.0), None);
    }

    #[test]
    fn fills_centres_inside_and_skips_holes() {
        let g = PlanGrid { origin_x: 0.0, origin_z: 0.0, texel: 1.0, width: 10, height: 10 };
        let outer = [[1.0, 1.0], [9.0, 1.0], [9.0, 9.0], [1.0, 9.0]];
        let hole = [[4.0, 4.0], [6.0, 4.0], [6.0, 6.0], [4.0, 6.0]];
        let mut got = vec![false; 100];
        fill_rings(&g, &[&outer, &hole], |i, j| got[j * 10 + i] = true);
        for j in 0..10 {
            for i in 0..10 {
                let (x, z) = g.centre(i, j);
                let want = (1.0..9.0).contains(&x) && (1.0..9.0).contains(&z) && !((4.0..6.0).contains(&x) && (4.0..6.0).contains(&z));
                assert_eq!(got[j * 10 + i], want, "texel ({i}, {j})");
            }
        }
        assert_eq!(got.iter().filter(|v| **v).count(), 64 - 4);
        assert_eq!(ring_of_flat(&[0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 0.0]).len(), 3);
    }

    #[test]
    fn corridor_and_erosion() {
        let g = PlanGrid { origin_x: 0.0, origin_z: 0.0, texel: 1.0, width: 10, height: 10 };
        // A line along z = 5 from x = 2 to x = 7, 1.2 m each side: rows 4 and 5
        // (centres 4.5 and 5.5), columns whose centre is within reach of the ends.
        let mut got = vec![false; 100];
        fill_corridor(&g, &[[2.0, 5.0], [7.0, 5.0]], 1.2, |i, j| got[j * 10 + i] = true);
        for j in 0..10 {
            for i in 0..10 {
                let (x, z) = g.centre(i, j);
                let dx = if x < 2.0 { 2.0 - x } else if x > 7.0 { x - 7.0 } else { 0.0 };
                assert_eq!(got[j * 10 + i], dx.hypot(z - 5.0) <= 1.2, "texel ({i}, {j})");
            }
        }
        // A 5 x 4 block erodes to its 3 x 2 inside.
        let mut m = vec![false; 100];
        for j in 2..6 {
            for i in 1..6 {
                m[j * 10 + i] = true;
            }
        }
        let e = erode(&m, 10, 10);
        let inside: Vec<(usize, usize)> = (0..100).filter(|&k| e[k]).map(|k| (k % 10, k / 10)).collect();
        assert_eq!(inside, vec![(2, 3), (3, 3), (4, 3), (2, 4), (3, 4), (4, 4)]);
    }
}
