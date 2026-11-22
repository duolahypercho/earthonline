//! The sky an enclosed deck sees through its openings (access.rs): a baked
//! per-vertex sky visibility the city shader spends on the indirect light of
//! the deck's interior surfaces (never on the sun, whose cascades already
//! cast the window patches). The openings are sky-light portals: each is a
//! planar quad, cut into patches; a patch the occluders (the deck's cores)
//! do not hide from a vertex adds its exact point-to-polygon form factor
//! (Lambert's polygon formula; Cohen and Wallace, "Radiosity and Realistic
//! Image Synthesis", 1993, eq. 4.18), clipped to the vertex's hemisphere.
//! What the room's own walls hand round is the BRE split-flux
//! internally-reflected component (Hopkinson, Petherbridge and Longmore,
//! "Daylighting", 1966; CIBSE LG10 "Daylighting and window design", 1999):
//! IRC = 0.85 W / (A (1 - R)) (C R_fw + 5 R_cw) per cent, C = 39 under an
//! unobstructed sky. Electric light is the share of the daytime sky's
//! diffuse illuminance an office's maintained illuminance is (EN 12464-1:
//! 2021 table 34, 500 lx for writing and reading; a clear sky's diffuse
//! horizontal illuminance at a high sun is about 20 klx: CIE S 011/E:2003
//! standard clear sky, Darula and Kittler), so it reads at noon.
//!
//! Deterministic: plain f64 arithmetic in a fixed order.

use crate::geometry::V3;

/// A sky-light portal: an opening's planar quad (any winding).
pub type Portal = [V3; 4];

/// An occluder: a vertical prism over a convex plan ring, from y0 to y1.
#[derive(Clone, Debug)]
pub struct Prism {
    pub ring: Vec<[f64; 2]>,
    pub y0: f64,
    pub y1: f64,
}

impl Prism {
    /// A prism approximating a vertical cylinder.
    pub fn cylinder(cx: f64, cz: f64, r: f64, y0: f64, y1: f64, segs: usize) -> Self {
        let ring = (0..segs).map(|k| k as f64 / segs as f64 * std::f64::consts::TAU).map(|a| [cx + a.cos() * r, cz - a.sin() * r]).collect();
        Self { ring, y0, y1 }
    }

    /// Whether the segment p -> q passes through the prism (Cyrus-Beck clip).
    pub fn hits(&self, p: V3, q: V3) -> bool {
        let d = [q[0] - p[0], q[1] - p[1], q[2] - p[2]];
        let (mut t0, mut t1) = (1e-4, 1.0 - 1e-4);
        // the y slab: y0 <= y(t) <= y1.
        if !clip(&mut t0, &mut t1, p[1] - self.y0, d[1]) || !clip(&mut t0, &mut t1, self.y1 - p[1], -d[1]) {
            return false;
        }
        let n = self.ring.len();
        let area: f64 = (0..n).map(|i| self.ring[i][0] * self.ring[(i + 1) % n][1] - self.ring[(i + 1) % n][0] * self.ring[i][1]).sum();
        let sgn = if area > 0.0 { 1.0 } else { -1.0 };
        for i in 0..n {
            let (a, b) = (self.ring[i], self.ring[(i + 1) % n]);
            // inward normal of edge a -> b; inside where nrm . (x - a) >= 0.
            let (ex, ez) = (b[0] - a[0], b[1] - a[1]);
            let nrm = [-ez * sgn, ex * sgn];
            if !clip(&mut t0, &mut t1, nrm[0] * (p[0] - a[0]) + nrm[1] * (p[2] - a[1]), nrm[0] * d[0] + nrm[1] * d[2]) {
                return false;
            }
        }
        t0 < t1
    }
}

/// Liang-Barsky step: keep the t in [t0, t1] where a + b t >= 0.
fn clip(t0: &mut f64, t1: &mut f64, a: f64, b: f64) -> bool {
    if b.abs() < 1e-15 {
        return a >= 0.0;
    }
    let t = -a / b;
    if b > 0.0 {
        *t0 = t0.max(t);
    } else {
        *t1 = t1.min(t);
    }
    *t0 <= *t1
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn len(a: V3) -> f64 {
    dot(a, a).sqrt()
}

/// The form factor from a differential area at `x` with unit normal `n` to
/// the planar polygon `poly`, clipped to x's hemisphere (Lambert).
pub fn form_factor(x: V3, n: V3, poly: &[V3]) -> f64 {
    // Sutherland-Hodgman against the tangent plane (keep n . (p - x) > eps).
    let eps = 1e-6;
    let mut clipped: Vec<V3> = Vec::with_capacity(poly.len() + 2);
    let m = poly.len();
    for i in 0..m {
        let (a, b) = (poly[i], poly[(i + 1) % m]);
        let (da, db) = (dot(n, sub(a, x)) - eps, dot(n, sub(b, x)) - eps);
        if da >= 0.0 {
            clipped.push(a);
        }
        if (da >= 0.0) != (db >= 0.0) {
            let t = da / (da - db);
            clipped.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]);
        }
    }
    if clipped.len() < 3 {
        return 0.0;
    }
    let k = clipped.len();
    let mut sum = 0.0;
    for i in 0..k {
        let (a, b) = (sub(clipped[i], x), sub(clipped[(i + 1) % k], x));
        let (la, lb) = (len(a), len(b));
        if la < 1e-9 || lb < 1e-9 {
            continue;
        }
        let c = cross(a, b);
        let lc = len(c);
        if lc < 1e-12 {
            continue;
        }
        let gamma = (dot(a, b) / (la * lb)).clamp(-1.0, 1.0).acos();
        sum += gamma * dot(n, c) / lc;
    }
    (sum / std::f64::consts::TAU).abs().min(1.0)
}

/// One room's light: its portals, its occluders, and the share every
/// interior surface gets over its direct sky (inter-reflection and electric light).
#[derive(Clone, Debug, Default)]
pub struct Room {
    pub portals: Vec<Portal>,
    /// Each portal's visible transmittance (its glazing; 1 open); missing: 1.
    pub transmit: Vec<f64>,
    pub occluders: Vec<Prism>,
    pub fill: f64,
}

/// The BRE split-flux internally-reflected component (a fraction, not per
/// cent): `window_m2` the openings' area, `total_m2` every interior surface
/// (windows included), `r_mean` its area-weighted reflectance, `r_fw` the
/// floor's and the walls' below the windows' mid-height, `r_cw` the
/// ceiling's and the walls' above it; an unobstructed sky (C = 39).
pub fn irc(window_m2: f64, total_m2: f64, r_mean: f64, r_fw: f64, r_cw: f64) -> f64 {
    0.85 * window_m2 / (total_m2 * (1.0 - r_mean)) * (39.0 * r_fw + 5.0 * r_cw) / 100.0
}

/// The sun's share through a room's openings, handed round its walls, as a
/// fraction of the clear sky's diffuse horizontal illuminance: the beam
/// (direct normal illuminance 5 times the sky's diffuse horizontal one, a
/// CIE clear sky at a high sun: Darula and Kittler, CIE S 011/E:2003) on
/// every opening facing it, landing on the floor (`r_floor`) and spread by
/// the split-flux denominator A (1 - R). The sun of 13 h at the equinox in
/// San Francisco (37.8 N: elevation 52 degrees, azimuth 190 degrees, NOAA
/// solar calculator). `portals` with their outward normals.
pub fn irc_sun(portals: &[(Portal, V3)], total_m2: f64, r_mean: f64, r_floor: f64) -> f64 {
    let (el, az) = (52f64.to_radians(), 190f64.to_radians());
    // toward the sun: x east, y up, z south.
    let sun = [az.sin() * el.cos(), el.sin(), -az.cos() * el.cos()];
    let mut flux = 0.0;
    for (p, out) in portals {
        let c = cross(sub(p[1], p[0]), sub(p[3], p[0]));
        let area = len(c);
        let cos = dot(*out, sun) / len(*out).max(1e-9);
        if cos > 0.0 {
            flux += area * cos;
        }
    }
    5.0 * flux * r_floor / (total_m2 * (1.0 - r_mean))
}

/// EN 12464-1 maintained illuminance of an office (500 lx) over a clear
/// sky's diffuse horizontal illuminance at a high sun (20 klx).
pub const ELECTRIC_SHARE: f64 = 500.0 / 20_000.0;

/// The patches a portal is cut into: at most this long a side, metres.
const PATCH_M: f64 = 1.6;

impl Room {
    /// The sky visibility at x (normal n): the visible portals' form factor, plus the fill.
    pub fn visibility(&self, x: V3, n: V3, patches: &[(V3, [V3; 4], f64)]) -> f64 {
        let lift = [x[0] + n[0] * 0.02, x[1] + n[1] * 0.02, x[2] + n[2] * 0.02];
        let mut f = 0.0;
        for (c, q, t) in patches {
            if dot(n, sub(*c, x)) <= 0.0 && q.iter().all(|p| dot(n, sub(*p, x)) <= 0.0) {
                continue;
            }
            if self.occluders.iter().any(|o| o.hits(lift, *c)) {
                continue;
            }
            f += form_factor(x, n, q) * t;
        }
        (f + self.fill).clamp(0.0, 1.0)
    }

    /// Every portal cut into patches (centre, corners, the portal's transmittance).
    pub fn patches(&self) -> Vec<(V3, [V3; 4], f64)> {
        let mut out = Vec::new();
        for (k, p) in self.portals.iter().enumerate() {
            let tr = self.transmit.get(k).copied().unwrap_or(1.0);
            let (a, b, d) = (p[0], p[1], p[3]);
            let (u, v) = (sub(b, a), sub(d, a));
            let nu = (len(u) / PATCH_M).ceil().max(1.0) as usize;
            let nv = (len(v) / PATCH_M).ceil().max(1.0) as usize;
            // bilinear over the quad (it may be a trapezoid).
            let at = |s: f64, t: f64| -> V3 {
                let lo = [p[0][0] + (p[1][0] - p[0][0]) * s, p[0][1] + (p[1][1] - p[0][1]) * s, p[0][2] + (p[1][2] - p[0][2]) * s];
                let hi = [p[3][0] + (p[2][0] - p[3][0]) * s, p[3][1] + (p[2][1] - p[3][1]) * s, p[3][2] + (p[2][2] - p[3][2]) * s];
                [lo[0] + (hi[0] - lo[0]) * t, lo[1] + (hi[1] - lo[1]) * t, lo[2] + (hi[2] - lo[2]) * t]
            };
            for i in 0..nu {
                for j in 0..nv {
                    let (s0, s1, t0, t1) = (i as f64 / nu as f64, (i + 1) as f64 / nu as f64, j as f64 / nv as f64, (j + 1) as f64 / nv as f64);
                    let q = [at(s0, t0), at(s1, t0), at(s1, t1), at(s0, t1)];
                    out.push((at((s0 + s1) / 2.0, (t0 + t1) / 2.0), q, tr));
                }
            }
        }
        out
    }

    /// The visibility of every vertex of a mesh (positions and normals, f32
    /// triples), cached on the quantised position and normal (a mesh repeats
    /// a vertex for every face it closes).
    pub fn bake(&self, positions: &[f32], normals: &[f32]) -> Vec<f32> {
        let patches = self.patches();
        let mut cache: std::collections::HashMap<[i64; 6], f32> = std::collections::HashMap::new();
        positions
            .chunks_exact(3)
            .zip(normals.chunks_exact(3))
            .map(|(p, n)| {
                let key = [(p[0] * 200.0).round() as i64, (p[1] * 200.0).round() as i64, (p[2] * 200.0).round() as i64, (n[0] * 100.0).round() as i64, (n[1] * 100.0).round() as i64, (n[2] * 100.0).round() as i64];
                *cache.entry(key).or_insert_with(|| {
                    let nn = [n[0] as f64, n[1] as f64, n[2] as f64];
                    let l = len(nn);
                    let nn = if l > 1e-6 { [nn[0] / l, nn[1] / l, nn[2] / l] } else { [0.0, 1.0, 0.0] };
                    self.visibility([p[0] as f64, p[1] as f64, p[2] as f64], nn, &patches) as f32
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_unit_square_over_a_point_has_the_textbook_form_factor() {
        // A 1 m x 1 m square 1 m over a corner-aligned point: F = 0.1385 (Howell's catalogue C-11 for a = b = c = 1).
        let sq = [[0.0, 1.0, 0.0], [1.0, 1.0, 0.0], [1.0, 1.0, 1.0], [0.0, 1.0, 1.0]];
        let f = form_factor([0.0, 0.0, 0.0], [0.0, 1.0, 0.0], &sq);
        assert!((f - 0.1385).abs() < 1e-3, "{f}");
        // either winding.
        let mut r = sq;
        r.reverse();
        assert!((form_factor([0.0, 0.0, 0.0], [0.0, 1.0, 0.0], &r) - f).abs() < 1e-12);
        // a huge square overhead sees the whole hemisphere.
        let big = [[-1e4, 1.0, -1e4], [1e4, 1.0, -1e4], [1e4, 1.0, 1e4], [-1e4, 1.0, 1e4]];
        assert!((form_factor([0.0, 0.0, 0.0], [0.0, 1.0, 0.0], &big) - 1.0).abs() < 1e-3);
        // behind the surface: nothing.
        assert_eq!(form_factor([0.0, 2.0, 0.0], [0.0, 1.0, 0.0], &sq), 0.0);
    }

    #[test]
    fn a_prism_hides_what_is_behind_it() {
        let core = Prism::cylinder(0.0, 0.0, 1.0, 0.0, 3.0, 24);
        assert!(core.hits([-3.0, 1.0, 0.0], [3.0, 1.0, 0.0]));
        assert!(!core.hits([-3.0, 1.0, 2.0], [3.0, 1.0, 2.0]));
        assert!(!core.hits([-3.0, 4.0, 0.0], [3.0, 4.0, 0.0]));
        let mut room = Room { portals: vec![[[2.0, 0.0, -1.0], [2.0, 0.0, 1.0], [2.0, 2.0, 1.0], [2.0, 2.0, -1.0]]], transmit: vec![], occluders: vec![core], fill: 0.0 };
        let p = room.patches();
        let open = room.visibility([1.5, 1.0, 0.0], [1.0, 0.0, 0.0], &p);
        let hidden = room.visibility([-1.5, 1.0, 0.0], [1.0, 0.0, 0.0], &p);
        assert!(open > 0.3 && hidden < 0.02, "{open} {hidden}");
        // a glazed opening passes its transmittance's share.
        room.transmit = vec![0.7];
        let p = room.patches();
        let glazed = room.visibility([1.5, 1.0, 0.0], [1.0, 0.0, 0.0], &p);
        assert!((glazed - 0.7 * open).abs() < 1e-9, "{glazed} {open}");
    }
}
