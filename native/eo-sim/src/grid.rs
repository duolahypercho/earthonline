//! A uniform grid over a set of points, rebuilt every step by counting sort.
//!
//! The JS sim keeps two such indexes (`crossGrid`, 12 m, for the vehicle
//! forward corridor; the person hazard hash, 16 m, for yielding to people).
//! This one is bounded by the points it holds, not by the city: a region's
//! agents sit within a few hundred metres of its focus, so the grid is a few
//! thousand cells however large the world is. Its buffers are reused, so a
//! rebuild allocates nothing once they have grown to the region's size.

pub struct Grid {
    pub cell: f64,
    min: [f64; 2],
    nx: usize,
    nz: usize,
    start: Vec<u32>,
    items: Vec<u32>,
    cell_of: Vec<u32>,
}

/// Grid dimensions are capped; a sparser population gets larger cells instead.
const MAX_CELLS_PER_AXIS: usize = 256;

impl Grid {
    pub fn new(cell: f64) -> Self {
        Grid { cell, min: [0.0, 0.0], nx: 1, nz: 1, start: vec![0, 0], items: vec![], cell_of: vec![] }
    }

    /// Index every `i` with `keep(i)` at (xs[i], zs[i]).
    pub fn build(&mut self, xs: &[f64], zs: &[f64], base_cell: f64, keep: impl Fn(usize) -> bool) {
        let n = xs.len();
        let (mut x0, mut z0, mut x1, mut z1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        let mut count = 0usize;
        for i in 0..n {
            if !keep(i) {
                continue;
            }
            count += 1;
            x0 = x0.min(xs[i]);
            z0 = z0.min(zs[i]);
            x1 = x1.max(xs[i]);
            z1 = z1.max(zs[i]);
        }
        self.cell_of.clear();
        self.cell_of.resize(n, u32::MAX);
        if count == 0 {
            self.nx = 1;
            self.nz = 1;
            self.start.clear();
            self.start.resize(2, 0);
            self.items.clear();
            return;
        }
        let mut cell = base_cell;
        while ((x1 - x0) / cell) as usize + 1 > MAX_CELLS_PER_AXIS || ((z1 - z0) / cell) as usize + 1 > MAX_CELLS_PER_AXIS {
            cell *= 2.0;
        }
        self.cell = cell;
        self.min = [x0, z0];
        self.nx = ((x1 - x0) / cell) as usize + 1;
        self.nz = ((z1 - z0) / cell) as usize + 1;
        let cells = self.nx * self.nz;
        self.start.clear();
        self.start.resize(cells + 1, 0);
        for i in 0..n {
            if !keep(i) {
                continue;
            }
            let cx = ((xs[i] - x0) / cell) as usize;
            let cz = ((zs[i] - z0) / cell) as usize;
            let c = (cz * self.nx + cx) as u32;
            self.cell_of[i] = c;
            self.start[c as usize + 1] += 1;
        }
        for c in 0..cells {
            self.start[c + 1] += self.start[c];
        }
        self.items.clear();
        self.items.resize(count, 0);
        // Fill using a running cursor stored temporarily in `start` (then restored).
        for i in 0..n {
            let c = self.cell_of[i];
            if c == u32::MAX {
                continue;
            }
            let at = self.start[c as usize] as usize;
            self.items[at] = i as u32;
            self.start[c as usize] += 1;
        }
        for c in (1..=cells).rev() {
            self.start[c] = self.start[c - 1];
        }
        self.start[0] = 0;
    }

    /// Visit every indexed item in cells overlapping the box.
    #[inline]
    pub fn query(&self, x0: f64, z0: f64, x1: f64, z1: f64, mut f: impl FnMut(u32)) {
        if self.items.is_empty() {
            return;
        }
        let to_cell = |v: f64, m: f64, n: usize| -> Option<usize> {
            let c = ((v - m) / self.cell).floor();
            if c < 0.0 {
                Some(0)
            } else if c as usize >= n {
                Some(n - 1)
            } else {
                Some(c as usize)
            }
        };
        if x1 < self.min[0] || z1 < self.min[1] {
            return;
        }
        if x0 > self.min[0] + self.nx as f64 * self.cell || z0 > self.min[1] + self.nz as f64 * self.cell {
            return;
        }
        let (Some(cx0), Some(cx1), Some(cz0), Some(cz1)) = (
            to_cell(x0, self.min[0], self.nx),
            to_cell(x1, self.min[0], self.nx),
            to_cell(z0, self.min[1], self.nz),
            to_cell(z1, self.min[1], self.nz),
        ) else {
            return;
        };
        for cz in cz0..=cz1 {
            for cx in cx0..=cx1 {
                let c = cz * self.nx + cx;
                for k in self.start[c]..self.start[c + 1] {
                    f(self.items[k as usize]);
                }
            }
        }
    }
}
