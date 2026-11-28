//! Paint is a thin overlay on the same two triangles the road builder emits
//! between adjacent stations and lateral section offsets.
use super::{EntryGeom, Layer, TileMesh, p3};
use crate::StreetOptions;

#[derive(Clone, Copy)]
pub(super) enum StripeWidth {
    AcrossRoad,
    AlongRoad,
}

#[derive(Clone, Copy)]
struct Point {
    x: f64,
    z: f64,
    edge: f64,
}

/// A rectangle in road coordinates. An approach can run toward decreasing
/// stations and offsets. The original directions also orient the wear mask.
pub(super) fn paint(
    mesh: &mut TileMesh,
    g: &EntryGeom,
    o: &StreetOptions,
    half: f64,
    stations: [f64; 2],
    across: [f64; 2],
    width: StripeWidth,
    color: [u8; 3],
    lift: f64,
) {
    let [s0, s1] = stations;
    let [u0, u1] = across;
    let lo = s0.min(s1);
    let hi = s0.max(s1);
    let start = g.stations.partition_point(|f| f.s <= lo).saturating_sub(1);
    let offsets = g.section_offsets(o, half);
    for i in start..g.keep.len() {
        let a = g.stations[i];
        let b = g.stations[i + 1];
        if a.s >= hi {
            break;
        }
        if !g.keep[i] || b.s <= lo || b.s <= a.s {
            continue;
        }
        let near = lo.max(a.s);
        let far = hi.min(b.s);
        for band in offsets.windows(2) {
            let left = u0.min(u1).max(band[0]);
            let right = u0.max(u1).min(band[1]);
            if left >= right {
                continue;
            }
            let corner = |s: f64, u: f64| {
                let t = (s - a.s) / (b.s - a.s);
                let p = a.at(u);
                let q = b.at(u);
                let edge = match width {
                    StripeWidth::AcrossRoad => (u - u0) / (u1 - u0),
                    StripeWidth::AlongRoad => (s - s0) / (s1 - s0),
                };
                Point {
                    x: p[0] + (q[0] - p[0]) * t,
                    z: p[1] + (q[1] - p[1]) * t,
                    edge,
                }
            };
            let vertex = |j: usize, u: f64| {
                // The real carriageway positions have already gone to f32.
                p3(g.stations[j].at(u), g.pavement_y(o, j, u, half))
                    .map(|c| f64::from(c as f32))
            };
            let road = [
                vertex(i, band[0]),
                vertex(i + 1, band[0]),
                vertex(i + 1, band[1]),
                vertex(i, band[1]),
            ];
            let stripe = [
                corner(near, left),
                corner(far, left),
                corner(far, right),
                corner(near, right),
            ];
            project(mesh, road, stripe, color, lift);
        }
    }
}

fn project(
    mesh: &mut TileMesh,
    road: [[f64; 3]; 4],
    stripe: [Point; 4],
    color: [u8; 3],
    lift: f64,
) {
    let [a, b, c, d] = road;
    let dx = c[0] - a[0];
    let dz = c[2] - a[2];
    let len2 = dx * dx + dz * dz;
    if len2 <= f64::EPSILON {
        return;
    }
    let side = |x: f64, z: f64| dx * (z - a[2]) - dz * (x - a[0]);
    let base = |x: f64, z: f64| a[1] + (c[1] - a[1]) * ((x - a[0]) * dx + (z - a[2]) * dz) / len2;
    let slopes = [b, d].map(|p| {
        let v = side(p[0], p[2]);
        (v, (p[1] - base(p[0], p[2])) / v)
    });
    let distances = stripe.map(|p| side(p.x, p.z));
    for (orientation, slope) in slopes {
        if !slope.is_finite() || orientation == 0. {
            continue;
        }
        let position = |p: Point| [p.x, base(p.x, p.z) + slope * side(p.x, p.z) + lift, p.z];
        if distances.iter().all(|v| v * orientation >= 0.)
            || ((slopes[0].1 - slopes[1].1).abs()
                * distances.iter().map(|v| v.abs()).fold(0., f64::max)
                < 1e-6)
        {
            mesh.quad_kerb(
                Layer::Marking,
                stripe.map(position),
                [color; 4],
                stripe.map(|p| p.edge as f32),
            );
            return;
        }
        if distances.iter().all(|v| v * orientation <= 0.) {
            continue;
        }
        // Clip the mark along the real pavement diagonal. Its two halves can
        // have different slopes on a bend or a banked approach.
        let mut points = Vec::with_capacity(5);
        for j in 0..4 {
            let prev = (j + 3) % 4;
            let p = stripe[prev];
            let q = stripe[j];
            if distances[prev] * distances[j] < 0. {
                let t = distances[prev] / (distances[prev] - distances[j]);
                points.push(Point {
                    x: p.x + t * (q.x - p.x),
                    z: p.z + t * (q.z - p.z),
                    edge: p.edge + t * (q.edge - p.edge),
                });
            }
            if distances[j] * orientation >= 0. {
                points.push(q)
            }
        }
        for j in 2..points.len() {
            let tri = [points[0], points[j - 1], points[j]];
            let first = mesh.positions.len();
            mesh.tri(Layer::Marking, tri.map(position), [color; 3]);
            if mesh.positions.len() > first {
                mesh.kerb[first..].copy_from_slice(&tri.map(|p| p.edge as f32));
            }
        }
    }
}
