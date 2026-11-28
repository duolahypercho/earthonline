//! A terrain vertex can be below the nearby street while its coarse triangle
//! still crosses that street on a hill. Lower only terrain triangles that
//! intersect the finished roadway, using the game's elevation grid.
use crate::StreetWorld;
use std::collections::HashMap;
type Point = [f64; 3];
type Key = (i32, i32);

struct Fit<'a> {
    w: &'a StreetWorld,
    cell: f32,
    origin: [f32; 2],
    vertices: HashMap<Key, Point>,
    caps: HashMap<Key, f32>,
}

impl Fit<'_> {
    fn point(&mut self, key: Key) -> Point {
        *self.vertices.entry(key).or_insert_with(|| {
            let x = self.origin[0] + (key.0 as f32 + 0.5) * self.cell;
            let z = self.origin[1] + (key.1 as f32 + 0.5) * self.cell;
            [
                f64::from(x),
                f64::from(self.w.base_carpet_vertex(x, z)),
                f64::from(z),
            ]
        })
    }
    fn road(&mut self, road: [Point; 3]) {
        let [a, b, c] = road;
        let [bx, by, bz] = std::array::from_fn(|i| b[i] - a[i]);
        let [cx, cy, cz] = std::array::from_fn(|i| c[i] - a[i]);
        let det = bx * cz - bz * cx;
        if det.abs() < 1e-9 {
            return;
        }
        let sx = (by * cz - bz * cy) / det;
        let sz = (bx * cy - by * cx) / det;
        let road_y = |p: Point| a[1] + sx * (p[0] - a[0]) + sz * (p[2] - a[2]);
        let cell = |p: f64, k: usize| {
            ((p - f64::from(self.origin[k])) / f64::from(self.cell) - 0.5).floor() as i32
        };
        let ext = |i: usize, k: usize| {
            cell(road.iter().map(|p| p[i]).fold(f64::INFINITY, f64::min), k)
                ..=cell(
                    road.iter().map(|p| p[i]).fold(f64::NEG_INFINITY, f64::max),
                    k,
                )
        };
        let rows = ext(2, 1);
        let columns = ext(0, 0);
        for x in columns {
            for z in rows.clone() {
                for keys in [
                    [(x, z), (x + 1, z), (x, z + 1)],
                    [(x + 1, z + 1), (x, z + 1), (x + 1, z)],
                ] {
                    let ground = keys.map(|key| self.point(key));
                    if ground.iter().all(|&p| p[1] <= road_y(p)) {
                        continue;
                    }
                    let mut poly = ground.to_vec();
                    for i in 0..3 {
                        let p = road[i];
                        let q = road[(i + 1) % 3];
                        let distance = |v: Point| {
                            ((q[0] - p[0]) * (v[2] - p[2]) - (q[2] - p[2]) * (v[0] - p[0]))
                                * det.signum()
                        };
                        let Some(mut previous) = poly.last().copied() else {
                            break;
                        };
                        let mut result = Vec::with_capacity(poly.len() + 1);
                        let mut u = distance(previous);
                        for v in poly {
                            let d = distance(v);
                            if d * u < 0.0 {
                                let t = u / (u - d);
                                result.push(std::array::from_fn(|j| {
                                    previous[j] + t * (v[j] - previous[j])
                                }));
                            }
                            if d >= 0.0 {
                                result.push(v);
                            }
                            previous = v;
                            u = d;
                        }
                        poly = result;
                    }
                    let excess = poly.iter().map(|&p| p[1] - road_y(p)).fold(0.0, f64::max);
                    if excess <= 0.0 {
                        continue;
                    }
                    // Both heights are affine on their common polygon. Its
                    // highest penetration is attained at one of those vertices.
                    for (key, p) in keys.into_iter().zip(ground) {
                        let cap = (p[1] - excess - self.w.o.marking_lift) as f32;
                        self.caps
                            .entry(key)
                            .and_modify(|c| *c = c.min(cap))
                            .or_insert(cap);
                    }
                }
            }
        }
    }
}

pub(crate) fn fit(w: &StreetWorld) -> HashMap<Key, f32> {
    let meta = &w.terrain.grid.meta;
    let mut fit = Fit {
        w,
        cell: meta.cell_size as f32,
        origin: [meta.origin_x as f32, meta.origin_z as f32],
        vertices: HashMap::new(),
        caps: HashMap::new(),
    };
    for (ei, g) in w.geoms.iter().enumerate().filter(|_| w.o.pavement_repairs) {
        let e = &w.plan.entries[ei];
        if w.city.segments[e.road].separated
            || !e.structure_ramps.is_empty()
            || e.structure_ends.iter().any(Option::is_some)
        {
            continue;
        }
        let offs = g.section_offsets(&w.o, e.half);
        for (i, &keep) in g.keep.iter().enumerate() {
            if !keep {
                continue;
            }
            let vertex = |i: usize, u: f64| {
                let [x, z] = g.stations[i].at(u);
                [x, g.pavement_y(&w.o, i, u, e.half), z].map(|v| f64::from(v as f32))
            };
            for u in offs.windows(2) {
                let [a, b, c, d] = [
                    vertex(i, u[0]),
                    vertex(i + 1, u[0]),
                    vertex(i + 1, u[1]),
                    vertex(i, u[1]),
                ];
                fit.road([a, b, c]);
                fit.road([a, c, d]);
            }
        }
    }
    if let Some(surface) = &w.surveyed_junction {
        for &face in &surface.pavement {
            fit.road(face.map(|p| p.map(|v| f64::from(v as f32))));
        }
    }
    fit.caps
}
