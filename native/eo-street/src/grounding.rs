//! Where a building's ground floor stands, and what closes the gap under it,
//! as the browser game decides it for a streamed building:
//!
//! * the floor (`createStreetFrontFloor`, src/world/buildings/street-front-grade.js):
//!   the mean of the surface DRAWN in front of the street face, read
//!   `WALK_PROBE_M` outside the wall at `FLOOR_PROBE_STATIONS` stations
//!   (`floorProbeT`), through `drawnSurfaceAt` (paving where the street pass
//!   draws a walk, past a bank to the first walking slope, else the ground
//!   carpet), held within `FRONT_CLAMP_M` of the centre ground on a deep steep
//!   footprint (`clampDeepFootprint`); without a street face, the lowest corner
//!   of the surface grade (`surfaceGradeOf`). The front edge is the archetype
//!   front the facades use (eo-kit `choose_archetype_front_edge`).
//! * the foundation (`planBuildingFoundation`, building-foundations.js): the
//!   floor stays where it is and a skirt wall is carried down from it to
//!   `min(floor - minimumSkirt, support - embed)`, `support` being the lowest
//!   ground under the outline.
//!
//! Not ported: the yard floor rule and the front walk rule (yard-floor-rule.js:
//! a row house that owns its front yard, or whose entry stands where the walk
//! is higher than the mean, has its floor raised to the walk at the entry) and
//! the door notches the skirt turns round.

use crate::StreetWorld;

/// A footprint edge in world metres, `n` its outward normal.
#[derive(Clone, Copy, Debug)]
pub struct FrontEdge {
    pub start: [f64; 2],
    pub u: [f64; 2],
    pub n: [f64; 2],
    pub length: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct Floor {
    pub y: f64,
    /// Read off a street front (else the lowest corner).
    pub from_front: bool,
    /// `clampDeepFootprint` moved it.
    pub clamped: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Foundation {
    pub base_y: f64,
    pub bottom_y: f64,
    pub support_min: f64,
}

fn shoelace(p: &[[f64; 2]]) -> f64 {
    let mut a = 0.0;
    for i in 0..p.len() {
        let (x, y) = (p[i], p[(i + 1) % p.len()]);
        a += x[0] * y[1] - y[0] * x[1];
    }
    a / 2.0
}

fn outward(a: [f64; 2], b: [f64; 2], sign: f64) -> Option<[f64; 2]> {
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let l = (dx * dx + dz * dz).sqrt();
    if l <= 1e-9 {
        return None;
    }
    Some(if sign > 0.0 { [dz / l, -dx / l] } else { [-dz / l, dx / l] })
}

/// A ring without a closing duplicate.
pub fn open_ring(ring: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut r = ring.to_vec();
    if r.len() > 1 {
        let (f, l) = (r[0], r[r.len() - 1]);
        if (f[0] - l[0]).abs() < 1e-9 && (f[1] - l[1]).abs() < 1e-9 {
            r.pop();
        }
    }
    r
}

impl StreetWorld {
    /// `floorProbeT`.
    pub fn floor_probe_t(&self, i: usize) -> f64 {
        0.05 + 0.9 * (i as f64 / (self.o.floor_probe_stations - 1) as f64)
    }

    /// The front probe points and their drawn surface.
    pub fn front_probes(&self, e: &FrontEdge) -> Vec<([f64; 2], f64)> {
        let p = self.o.walk_probe_m;
        (0..self.o.floor_probe_stations)
            .map(|i| {
                let t = self.floor_probe_t(i);
                let x = e.start[0] + e.u[0] * e.length * t + e.n[0] * p;
                let z = e.start[1] + e.u[1] * e.length * t + e.n[1] * p;
                ([x, z], self.surface_at(x, z, Some(e.n)))
            })
            .collect()
    }

    /// `surfaceGradeOf`: the drawn surface just outside each vertex, along its bisector.
    pub fn surface_grade(&self, ring: &[[f64; 2]]) -> Vec<f64> {
        let r = open_ring(ring);
        let sign = if shoelace(&r) > 0.0 { 1.0 } else { -1.0 };
        let n = r.len();
        let p = self.o.walk_probe_m;
        (0..n)
            .map(|i| {
                let (prev, pt, next) = (r[(i + n - 1) % n], r[i], r[(i + 1) % n]);
                let n0 = outward(prev, pt, sign);
                let n1 = outward(pt, next, sign);
                let (mut nx, mut nz) = (n0.map(|v| v[0]).unwrap_or(0.0) + n1.map(|v| v[0]).unwrap_or(0.0), n0.map(|v| v[1]).unwrap_or(0.0) + n1.map(|v| v[1]).unwrap_or(0.0));
                let l = (nx * nx + nz * nz).sqrt();
                if l > 1e-6 {
                    nx /= l;
                    nz /= l;
                } else {
                    let f = n1.or(n0).unwrap_or([0.0, 0.0]);
                    nx = f[0];
                    nz = f[1];
                }
                self.surface_at(pt[0] + nx * p, pt[1] + nz * p, Some([nx, nz]))
            })
            .collect()
    }

    /// `clampDeepFootprint` (terrain space, as the JS reads it).
    fn clamp_deep(&self, ring: &[[f64; 2]], y: f64) -> (f64, bool) {
        let (mut mn, mut mx, mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2], f64::MAX, f64::MIN);
        for q in ring {
            mn = [mn[0].min(q[0]), mn[1].min(q[1])];
            mx = [mx[0].max(q[0]), mx[1].max(q[1])];
            let h = self.ground(q[0], q[1]);
            lo = lo.min(h);
            hi = hi.max(h);
        }
        let centre = self.ground((mn[0] + mx[0]) / 2.0, (mn[1] + mx[1]) / 2.0);
        if hi - lo > self.o.front_grade_max_relief_m {
            let c = y.max(centre - self.o.front_clamp_m).min(centre + self.o.front_clamp_m);
            return (c, (c - y).abs() > 1e-9);
        }
        (y, false)
    }

    /// The street-front floor of a building (`createStreetFrontFloor`).
    pub fn street_front_floor(&self, ring: &[[f64; 2]], front: Option<&FrontEdge>) -> Floor {
        let lowest = || {
            let g = self.surface_grade(ring);
            Floor { y: g.iter().copied().fold(f64::INFINITY, f64::min), from_front: false, clamped: false }
        };
        let Some(e) = front else { return lowest() };
        let probes = self.front_probes(e);
        let mean = probes.iter().map(|p| p.1).sum::<f64>() / probes.len() as f64;
        let (y, clamped) = self.clamp_deep(ring, mean);
        Floor { y, from_front: true, clamped }
    }

    /// Perimeter samples (every metre at most), with their outward normals.
    pub fn perimeter_samples(&self, ring: &[[f64; 2]]) -> Vec<([f64; 2], [f64; 2])> {
        let r = open_ring(ring);
        let sign = if shoelace(&r) > 0.0 { 1.0 } else { -1.0 };
        let mut out = vec![];
        for i in 0..r.len() {
            let (a, b) = (r[i], r[(i + 1) % r.len()]);
            let Some(n) = outward(a, b, sign) else { continue };
            let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
            let steps = (len.ceil() as usize).max(1);
            for s in 0..steps {
                let t = s as f64 / steps as f64;
                out.push(([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t], n));
            }
        }
        out
    }

    /// The lowest ground the walls stand over: the drawn surface on the outline
    /// and just outside it, and the terrain mesh's vertices round the footprint.
    pub fn support_min(&self, ring: &[[f64; 2]]) -> f64 {
        let mut lo = f64::INFINITY;
        let p = self.o.walk_probe_m;
        let r = open_ring(ring);
        let sign = if shoelace(&r) > 0.0 { 1.0 } else { -1.0 };
        for i in 0..r.len() {
            let (a, b) = (r[i], r[(i + 1) % r.len()]);
            let Some(n) = outward(a, b, sign) else { continue };
            let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
            let steps = ((len / 0.25).ceil() as usize).max(1);
            for s in 0..=steps {
                let t = s as f64 / steps as f64;
                let q = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
                // each probe distance and 3 cm either side of it, so a
                // steep bank face standing at the probe (two levels meeting) is
                // read at its foot whichever side of it a probe lands on.
                for d in [-0.03, 0.0, 0.03, 0.07, 0.1, 0.13, p - 0.03, p, p + 0.03] {
                    let (x, z) = (q[0] + n[0] * d, q[1] + n[1] * d);
                    lo = lo.min(self.surface_at(x, z, None));
                    // where a bank's steep face stands over lower
                    // paving, the skirt reaches the lower of the two.
                    if let Some(h) = self.paved_heights(x, z).into_iter().reduce(f64::min) {
                        lo = lo.min(h);
                    }
                }
                lo = lo.min(self.carpet_y(q[0], q[1]));
            }
        }
        lo
    }

    /// `planBuildingFoundation` with the archetype minimum skirt.
    pub fn foundation(&self, ring: &[[f64; 2]], base_y: f64, minimum_skirt: f64) -> Foundation {
        let support = self.support_min(ring);
        let bottom = (base_y - minimum_skirt.max(0.0)).min(support - self.o.foundation_embed_m.max(0.0));
        Foundation { base_y, bottom_y: bottom, support_min: support }
    }
}

/// Edge `i` of a record polygon (as the record lists it, a closing duplicate
/// kept: the authored modules number edges that way), with its outward normal.
pub fn record_edge(record: &[[f64; 2]], i: usize) -> Option<FrontEdge> {
    if record.len() < 3 {
        return None;
    }
    let open = open_ring(record);
    let sign = if shoelace(&open) > 0.0 { 1.0 } else { -1.0 };
    let (a, b) = (record[i % record.len()], record[(i + 1) % record.len()]);
    let n = outward(a, b, sign)?;
    let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
    Some(FrontEdge { start: a, u: [(b[0] - a[0]) / len, (b[1] - a[1]) / len], n, length: len })
}

impl StreetWorld {
    /// A Blender-built building's floor: the JS placement's (street-front
    /// grade over its authored front edge + construction lift), raised to the
    /// walk drawn in front of that edge where the walk is higher - the
    /// direction the JS's front walk rule moves a floor (`yardFloorGrades`:
    /// "raised to the walk along its front"; it never lowers one).
    pub fn authored_floor(&self, record: &[[f64; 2]], edge_index: usize, js_base: f64) -> (f64, Option<FrontEdge>) {
        let Some(e) = record_edge(record, edge_index) else { return (js_base, None) };
        let pr = self.front_probes(&e);
        let mean = pr.iter().map(|p| p.1).sum::<f64>() / pr.len() as f64;
        (js_base.max(mean), Some(e))
    }
}

/// What the measurement reads off one building.
#[derive(Clone, Debug)]
pub struct Grounding {
    pub floor_y: f64,
    pub bottom_y: f64,
    /// floor - mean drawn surface in front of the street face (None: no street face).
    pub front_gap: Option<f64>,
    /// Spread of the drawn surface along the front (max - min).
    pub front_spread: f64,
    /// floor - drawn surface outside each corner.
    pub corner_gaps: Vec<f64>,
    /// bottom of the building (skirt or base) - lowest drawn surface round the perimeter:
    /// > 0 is a visible gap under a wall.
    pub float_gap: f64,
    pub clamped: bool,
}

pub fn measure(w: &StreetWorld, ring: &[[f64; 2]], floor_y: f64, bottom_y: f64, front: Option<&FrontEdge>, clamped: bool) -> Grounding {
    let (front_gap, front_spread) = match front {
        Some(e) => {
            let pr = w.front_probes(e);
            let mean = pr.iter().map(|p| p.1).sum::<f64>() / pr.len() as f64;
            let (lo, hi) = pr.iter().fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p.1), a.1.max(p.1)));
            (Some(floor_y - mean), hi - lo)
        }
        None => (None, 0.0),
    };
    let corner_gaps = w.surface_grade(ring).into_iter().map(|s| floor_y - s).collect();
    // Independently of how the skirt was sized: every 0.25 m round the outline,
    // the drawn surface on the wall line and 0.1 m and the probe distance out.
    let r = open_ring(ring);
    let sign = if shoelace(&r) > 0.0 { 1.0 } else { -1.0 };
    let mut lo = f64::INFINITY;
    for i in 0..r.len() {
        let (a, b) = (r[i], r[(i + 1) % r.len()]);
        let Some(n) = outward(a, b, sign) else { continue };
        let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        let steps = ((len / 0.25).ceil() as usize).max(1);
        for s in 0..=steps {
            let t = s as f64 / steps as f64;
            let q = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            for d in [0.0, 0.1, w.o.walk_probe_m] {
                lo = lo.min(w.surface_at(q[0] + n[0] * d, q[1] + n[1] * d, None));
            }
        }
    }
    Grounding { floor_y, bottom_y, front_gap, front_spread, corner_gaps, float_gap: bottom_y - lo, clamped }
}
