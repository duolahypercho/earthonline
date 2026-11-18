//! The shoreline oracle: `getCitywideShorelineOracle` / `buildOracle` of
//! src/citygen/shoreline.js, ported line for line.
//!
//! The land is the `boundary` of sf-city.json.gz (DataSF "Shoreline and
//! Islands" rings, world metres). `is_land` is even-odd parity on horizontal
//! rows 2 m apart (cached lazily, as the JS caches them); `edge_distance` is
//! the exact point-to-segment distance to the nearest ring edge through the
//! JS's 24 m edge grid and its Chebyshev shell search.

use std::sync::OnceLock;

/// `SEA_LEVEL_Y`: the sea surface, world metres.
pub const SEA_LEVEL_Y: f64 = -0.55;
/// `SEABED_Y`: where the JS presses its ground carpet under open water.
pub const SEABED_Y: f64 = -2.6;
/// `WATER_APRON_M`: how far past the extent the oracle's region reaches.
pub const WATER_APRON_M: f64 = 1500.0;
/// `ROW_PITCH_M`: parity row pitch.
const ROW_PITCH_M: f64 = 2.0;
/// `CITYWIDE_RING_REACH_M`: rings kept by the citywide oracle.
const CITYWIDE_RING_REACH_M: f64 = 20000.0;
/// `EDGE_CELL_M`: the edge index's cell.
const EDGE_CELL_M: f64 = 24.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    pub min_x: f64,
    pub max_x: f64,
    pub min_z: f64,
    pub max_z: f64,
}

fn ring_bounds(ring: &[f64]) -> Region {
    let (mut min_x, mut max_x, mut min_z, mut max_z) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
    for p in ring.chunks_exact(2) {
        min_x = min_x.min(p[0]);
        max_x = max_x.max(p[0]);
        min_z = min_z.min(p[1]);
        max_z = max_z.max(p[1]);
    }
    Region { min_x, max_x, min_z, max_z }
}

/// The edges of one ring, closed with an explicit wrap whether or not the ring
/// repeats its first point (the JS iteration, in its order).
fn ring_edges(ring: &[f64], mut visit: impl FnMut(f64, f64, f64, f64)) {
    let n = ring.len();
    let (mut x0, mut z0) = (ring[n - 2], ring[n - 1]);
    let closed = (x0 - ring[0]).abs() < 1e-9 && (z0 - ring[1]).abs() < 1e-9;
    let mut start = 0;
    if closed {
        x0 = ring[0];
        z0 = ring[1];
        start = 2;
    }
    let mut i = start;
    while i + 1 < n {
        let (x1, z1) = (ring[i], ring[i + 1]);
        visit(x0, z0, x1, z1);
        x0 = x1;
        z0 = z1;
        i += 2;
    }
}

struct EdgeIndex {
    ax: Vec<f64>,
    az: Vec<f64>,
    bx: Vec<f64>,
    bz: Vec<f64>,
    cells: Vec<Vec<u32>>,
    cols: i64,
    rows: i64,
    near: Vec<i32>,
}

pub struct Oracle {
    pub region: Region,
    pub ring_count: usize,
    active: Vec<Vec<f64>>,
    row_count: usize,
    rows: Vec<OnceLock<Vec<f64>>>,
    edges: OnceLock<EdgeIndex>,
}

impl Oracle {
    /// `getCitywideShorelineOracle(city)`: the bounding box of every ring
    /// within 20 km of the origin, plus the apron. None without rings.
    pub fn citywide(boundary: &[Vec<f64>]) -> Option<Self> {
        let (mut min_x, mut max_x, mut min_z, mut max_z) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
        for ring in boundary {
            if ring.len() < 6 {
                continue;
            }
            let rb = ring_bounds(ring);
            if rb.min_x.abs().min(rb.max_x.abs()) > CITYWIDE_RING_REACH_M || rb.min_z.abs().min(rb.max_z.abs()) > CITYWIDE_RING_REACH_M {
                continue;
            }
            min_x = min_x.min(rb.min_x);
            max_x = max_x.max(rb.max_x);
            min_z = min_z.min(rb.min_z);
            max_z = max_z.max(rb.max_z);
        }
        if !min_x.is_finite() {
            return None;
        }
        Self::build(boundary, Region { min_x, max_x, min_z, max_z })
    }

    /// `buildOracle(city, extent)`.
    pub fn build(boundary: &[Vec<f64>], extent: Region) -> Option<Self> {
        let region = Region {
            min_x: extent.min_x - WATER_APRON_M,
            max_x: extent.max_x + WATER_APRON_M,
            min_z: extent.min_z - WATER_APRON_M,
            max_z: extent.max_z + WATER_APRON_M,
        };
        let active: Vec<Vec<f64>> = boundary
            .iter()
            .filter(|r| r.len() >= 6)
            .filter(|r| {
                let rb = ring_bounds(r);
                !(rb.max_x < region.min_x || rb.min_x > region.max_x || rb.max_z < region.min_z || rb.min_z > region.max_z)
            })
            .cloned()
            .collect();
        if active.is_empty() {
            return None;
        }
        let row_count = (((region.max_z - region.min_z) / ROW_PITCH_M).ceil() as usize).max(1);
        Some(Self { region, ring_count: active.len(), active, row_count, rows: (0..row_count).map(|_| OnceLock::new()).collect(), edges: OnceLock::new() })
    }

    /// `crossingsAtZ`: every land-edge x crossing of the line at `z`, sorted.
    pub fn crossings_at_z(&self, z: f64) -> Vec<f64> {
        let mut xs = vec![];
        for ring in &self.active {
            ring_edges(ring, |x0, z0, x1, z1| {
                if (z0 <= z) != (z1 <= z) {
                    xs.push(x0 + ((z - z0) / (z1 - z0)) * (x1 - x0));
                }
            });
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        xs
    }

    fn row_for(&self, z: f64) -> Option<&[f64]> {
        let index = ((z - self.region.min_z) / ROW_PITCH_M).floor();
        if index < 0.0 || index >= self.row_count as f64 {
            return None;
        }
        let i = index as usize;
        Some(self.rows[i].get_or_init(|| self.crossings_at_z(self.region.min_z + (i as f64 + 0.5) * ROW_PITCH_M)))
    }

    /// Lower bound: crossings strictly left of `x`.
    fn left_of(row: &[f64], x: f64) -> usize {
        row.partition_point(|v| *v < x)
    }

    pub fn is_land(&self, x: f64, z: f64) -> bool {
        let r = &self.region;
        if x < r.min_x || x > r.max_x || z < r.min_z || z > r.max_z {
            return true;
        }
        match self.row_for(z) {
            None => false,
            Some(row) if row.is_empty() => false,
            Some(row) => Self::left_of(row, x) & 1 == 1,
        }
    }

    pub fn is_water(&self, x: f64, z: f64) -> bool {
        !self.is_land(x, z)
    }

    fn edge_index(&self) -> &EdgeIndex {
        self.edges.get_or_init(|| {
            let r = self.region;
            let (mut ax, mut az, mut bx, mut bz) = (vec![], vec![], vec![], vec![]);
            for ring in &self.active {
                ring_edges(ring, |x0, z0, x1, z1| {
                    ax.push(x0);
                    az.push(z0);
                    bx.push(x1);
                    bz.push(z1);
                });
            }
            let cols = (((r.max_x - r.min_x) / EDGE_CELL_M).ceil() as i64).max(1);
            let rows = (((r.max_z - r.min_z) / EDGE_CELL_M).ceil() as i64).max(1);
            let mut cells: Vec<Vec<u32>> = vec![vec![]; (cols * rows) as usize];
            for e in 0..ax.len() {
                let c0 = ((ax[e].min(bx[e]) - r.min_x) / EDGE_CELL_M).floor() as i64;
                let c1 = ((ax[e].max(bx[e]) - r.min_x) / EDGE_CELL_M).floor() as i64;
                let r0 = ((az[e].min(bz[e]) - r.min_z) / EDGE_CELL_M).floor() as i64;
                let r1 = ((az[e].max(bz[e]) - r.min_z) / EDGE_CELL_M).floor() as i64;
                if c1 < 0 || c0 >= cols || r1 < 0 || r0 >= rows {
                    continue;
                }
                for rr in r0.max(0)..=r1.min(rows - 1) {
                    for cc in c0.max(0)..=c1.min(cols - 1) {
                        cells[(rr * cols + cc) as usize].push(e as u32);
                    }
                }
            }
            // Chebyshev distance (cells) to the nearest occupied cell: two-pass chamfer.
            let none = (cols + rows + 1) as i32;
            let mut near: Vec<i32> = cells.iter().map(|c| if c.is_empty() { none } else { 0 }).collect();
            let (cu, ru) = (cols as usize, rows as usize);
            for rr in 0..ru {
                for cc in 0..cu {
                    let k = rr * cu + cc;
                    let mut v = near[k];
                    if v == 0 {
                        continue;
                    }
                    if cc > 0 && near[k - 1] + 1 < v {
                        v = near[k - 1] + 1;
                    }
                    if rr > 0 {
                        let up = k - cu;
                        if near[up] + 1 < v {
                            v = near[up] + 1;
                        }
                        if cc > 0 && near[up - 1] + 1 < v {
                            v = near[up - 1] + 1;
                        }
                        if cc < cu - 1 && near[up + 1] + 1 < v {
                            v = near[up + 1] + 1;
                        }
                    }
                    near[k] = v;
                }
            }
            for rr in (0..ru).rev() {
                for cc in (0..cu).rev() {
                    let k = rr * cu + cc;
                    let mut v = near[k];
                    if v == 0 {
                        continue;
                    }
                    if cc < cu - 1 && near[k + 1] + 1 < v {
                        v = near[k + 1] + 1;
                    }
                    if rr < ru - 1 {
                        let down = k + cu;
                        if near[down] + 1 < v {
                            v = near[down] + 1;
                        }
                        if cc > 0 && near[down - 1] + 1 < v {
                            v = near[down - 1] + 1;
                        }
                        if cc < cu - 1 && near[down + 1] + 1 < v {
                            v = near[down + 1] + 1;
                        }
                    }
                    near[k] = v;
                }
            }
            EdgeIndex { ax, az, bx, bz, cells, cols, rows, near }
        })
    }

    /// `edgeDistance`: the exact distance to the nearest ring edge, capped.
    pub fn edge_distance(&self, x: f64, z: f64, cap: f64) -> f64 {
        let ix = self.edge_index();
        let r = self.region;
        let qc = ((x - r.min_x) / EDGE_CELL_M).floor() as i64;
        let qr = ((z - r.min_z) / EDGE_CELL_M).floor() as i64;
        let mut best = cap;
        let max_ring = (cap / EDGE_CELL_M).ceil() as i64 + 1;
        let mut first_ring = 0;
        if qc >= 0 && qc < ix.cols && qr >= 0 && qr < ix.rows {
            let empty = (ix.near[(qr * ix.cols + qc) as usize] as i64).min(max_ring + 1);
            for ring in 0..empty {
                if best <= ring as f64 * EDGE_CELL_M {
                    return best;
                }
            }
            first_ring = empty;
        }
        for ring in first_ring..=max_ring {
            let (r0, r1) = (qr - ring, qr + ring);
            for rr in r0.max(0)..=r1.min(ix.rows - 1) {
                let whole = rr == r0 || rr == r1;
                let step = if whole || ring == 0 { 1 } else { 2 * ring };
                let mut c = qc - ring;
                while c <= qc + ring {
                    if c >= 0 && c < ix.cols {
                        for &e in &ix.cells[(rr * ix.cols + c) as usize] {
                            let e = e as usize;
                            let dx = ix.bx[e] - ix.ax[e];
                            let dz = ix.bz[e] - ix.az[e];
                            let len2 = dx * dx + dz * dz;
                            let mut t = if len2 > 0.0 { ((x - ix.ax[e]) * dx + (z - ix.az[e]) * dz) / len2 } else { 0.0 };
                            t = t.clamp(0.0, 1.0);
                            let d = (x - (ix.ax[e] + dx * t)).hypot(z - (ix.az[e] + dz * t));
                            if d < best {
                                best = d;
                            }
                        }
                    }
                    c += step;
                }
            }
            if best <= ring as f64 * EDGE_CELL_M {
                break;
            }
        }
        best
    }

    /// Every ring edge (for geometry built along the shore).
    pub fn for_each_edge(&self, mut visit: impl FnMut(f64, f64, f64, f64)) {
        for ring in &self.active {
            ring_edges(ring, &mut visit);
        }
    }
}
