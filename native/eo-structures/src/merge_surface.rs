//! Close the part of a merge between consecutive cross-sections. A minor
//! deck is trimmed across each station; at an oblique end the major can end
//! inside the strip between stations. Clip that leftover strip against the
//! actual major pavement before adding it to the shared render/collision mesh.
use crate::index::cell_of;
use crate::mesh::{Builder, V3};
use crate::sweep::{Station, Sweep};
use crate::tiles::Refs;
use crate::{Chain, StructureClass};
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Patch {
    chain: u32,
    points: [V3; 3],
    color: [u8; 3],
    era: u8,
}

impl Patch {
    pub fn height_on(&self, chain: u32, x: f64, z: f64) -> Option<f64> {
        if self.chain != chain {
            return None;
        }
        crate::mesh::plan_height(&self.points, x, z)
    }

    pub fn draw(&self, b: &mut Builder) {
        b.era = self.era;
        let up = [0.0, 1.0, 0.0];
        let v = self.points.map(|p| {
            b.v(
                p,
                up,
                self.color,
                0.0,
                [p[0], p[2]],
                StructureClass::DeckSurface,
            )
        });
        b.tri(v, up);
    }
}

type Face = [V3; 3];

fn side(a: V3, b: V3, p: V3) -> f64 {
    (b[0] - a[0]) * (p[2] - a[2]) - (b[2] - a[2]) * (p[0] - a[0])
}

fn bounds(p: &[V3], axis: usize) -> (f64, f64) {
    p.iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
            (lo.min(p[axis]), hi.max(p[axis]))
        })
}

fn cells(p: &[V3]) -> impl Iterator<Item = (i32, i32)> {
    let (x0, x1) = bounds(p, 0);
    let (z0, z1) = bounds(p, 2);
    (cell_of(x0)..=cell_of(x1)).flat_map(move |x| (cell_of(z0)..=cell_of(z1)).map(move |z| (x, z)))
}

/// Partition a convex piece along each edge of the covering road triangle.
/// Vertices on an intersection inherit the subject road's interpolated height.
fn subtract(mut poly: Vec<V3>, face: &Face, out: &mut Vec<Vec<V3>>) {
    let orientation = side(face[0], face[1], face[2]).signum();
    for e in 0..3 {
        let (a, b) = (face[e], face[(e + 1) % 3]);
        let dist = |p| orientation * side(a, b, p);
        if poly.iter().all(|&p| dist(p) <= 0.0) {
            out.push(poly);
            return;
        }
        if poly.iter().all(|&p| dist(p) >= 0.0) {
            continue;
        }
        let (mut inside, mut outside) = (vec![], vec![]);
        let mut prev = *poly.last().unwrap();
        for &p in &poly {
            let (d0, d1) = (dist(prev), dist(p));
            if d0 * d1 < 0.0 {
                let t = d0 / (d0 - d1);
                let cross = std::array::from_fn(|axis| prev[axis] + t * (p[axis] - prev[axis]));
                inside.push(cross);
                outside.push(cross);
            }
            if d1 >= 0.0 {
                inside.push(p);
            }
            if d1 <= 0.0 {
                outside.push(p);
            }
            prev = p;
        }
        if outside.len() >= 3 {
            out.push(outside);
        }
        if inside.len() < 3 {
            return;
        }
        poly = inside;
    }
}

pub(crate) fn add(
    chains: &[Chain],
    sweeps: &[Option<Sweep>],
    tiles: &mut BTreeMap<(i32, i32), Refs>,
) {
    for lod in 0..3 {
        let mut masks: Vec<(u32, Face)> = vec![];
        let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for (c, sw) in chains.iter().zip(sweeps) {
            let Some(sw) = sw else { continue };
            if c.merged_with.is_empty() {
                continue;
            }
            for w in sw.levels[lod].windows(2) {
                let (a, b) = (&sw.st[w[0]], &sw.st[w[1]]);
                let edge = |s: &Station, sign: f64| s.at(sign * s.half, s.y);
                let ring = [edge(a, -1.0), edge(a, 1.0), edge(b, 1.0), edge(b, -1.0)];
                for tri in [[0, 1, 2], [0, 2, 3]] {
                    let face = tri.map(|i| ring[i]);
                    if side(face[0], face[1], face[2]).abs() <= 1e-12 {
                        continue;
                    }
                    let index = masks.len();
                    for k in cells(&face) {
                        grid.entry(k).or_default().push(index);
                    }
                    masks.push((c.id, face));
                }
            }
        }
        for (c, sw) in chains.iter().zip(sweeps) {
            let Some(sw) = sw else { continue };
            let majors: Vec<_> = c
                .merged_with
                .iter()
                .copied()
                .filter(|&id| crate::solve::is_major(&chains[id as usize], c))
                .collect();
            if majors.is_empty() {
                continue;
            }
            for w in sw.levels[lod].windows(2) {
                let (a, b) = (&sw.st[w[0]], &sw.st[w[1]]);
                let key = eo_street::tile_key((a.x + b.x) / 2.0, (a.z + b.z) / 2.0);
                let color = sw.asphalt[crate::mesh::piece_index(c, (a.s + b.s) / 2.0)];
                for (side_idx, sign) in [(0, -1.0), (1, 1.0)] {
                    let edge = |s: &Station| sign * s.half;
                    if a.clip[side_idx] == edge(a) && b.clip[side_idx] == edge(b) {
                        continue;
                    }
                    let point = |s: &Station, u: f64| s.at(u, sw.top(s, u));
                    let ring = [
                        point(a, edge(a)),
                        point(a, a.clip[side_idx]),
                        point(b, b.clip[side_idx]),
                        point(b, edge(b)),
                    ];
                    let mask_ids: BTreeSet<_> = cells(&ring)
                        .filter_map(|k| grid.get(&k))
                        .flatten()
                        .copied()
                        .filter(|&i| majors.contains(&masks[i].0))
                        .collect();
                    for idx in [[0, 1, 2], [0, 2, 3]] {
                        let mut regions = vec![idx.map(|i| ring[i]).to_vec()];
                        for &i in &mask_ids {
                            let mut pieces = vec![];
                            for region in regions {
                                subtract(region, &masks[i].1, &mut pieces);
                            }
                            regions = pieces;
                            if regions.is_empty() {
                                break;
                            }
                        }
                        for p in regions {
                            for j in 2..p.len() {
                                let points = [p[0], p[j - 1], p[j]];
                                if side(points[0], points[1], points[2]).abs() > 1e-8 {
                                    tiles.entry(key).or_default().merges[lod].push(Patch {
                                        chain: c.id,
                                        points,
                                        color,
                                        era: sw.era,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// At a landmark abutment the surveyed carriageway's last cross-section can
/// meet the bridge plates obliquely. Draw only the uncovered part of that
/// approach strip. Its near mesh supplies exactly the same faces to physics.
pub(crate) fn bridge_ends(
    road: &eo_landmarks::bridge_roads::RoadSurface,
    chains: &[Chain],
    sweeps: &[Option<Sweep>],
    tiles: &mut BTreeMap<(i32, i32), Refs>,
    clear: &mut crate::clear::ClearIndex,
) {
    for (chain, sweep) in chains.iter().zip(sweeps) {
        let Some(sw) = sweep else { continue };
        if chain.landmark != Some(road.deck) {
            continue;
        }
        let stations = &sw.levels[0];
        for end in 0..2 {
            let i = if end == 0 { 0 } else { stations.len() - 2 };
            // A one-panel chain has one end strip.
            if end == 1 && i == 0 {
                continue;
            }
            let section = |s: &Station| [-s.half, s.half].map(|u| s.at(u, sw.top(s, u)));
            let [a, b] = section(&sw.st[stations[i]]);
            let [d, c] = section(&sw.st[stations[i + 1]]);
            for face in [[a, b, c], [a, c, d]] {
                let nearby: Vec<_> = road
                    .road
                    .iter()
                    .filter(|mask| {
                        [0, 2].iter().all(|&axis| {
                            let (lo, hi) = bounds(&face, axis);
                            let (mlo, mhi) = bounds(&mask[..], axis);
                            hi >= mlo && mhi >= lo
                        })
                    })
                    .collect();
                let mut pieces = vec![face.to_vec()];
                for mask in &nearby {
                    let mut next = Vec::new();
                    for poly in pieces {
                        subtract(poly, mask, &mut next);
                    }
                    pieces = next;
                }
                for mut poly in pieces {
                    // Carry the precise bridge height through vertices on a
                    // clipped edge. The far boundary keeps its surveyed height.
                    for p in &mut poly {
                        'touch: for mask in &nearby {
                            for e in 0..3 {
                                let (a, b) = (mask[e], mask[(e + 1) % 3]);
                                let (t, gap, _) =
                                    crate::index::project([p[0], p[2]], [a[0], a[2]], [b[0], b[2]]);
                                if gap < 1e-6 {
                                    p[1] = a[1] + t * (b[1] - a[1]);
                                    break 'touch;
                                }
                            }
                        }
                    }
                    for k in 2..poly.len() {
                        let points = [poly[0], poly[k - 1], poly[k]];
                        if side(points[0], points[1], points[2]).abs() <= 1e-8 {
                            continue;
                        }
                        let key = eo_street::tile_key(
                            (points[0][0] + points[1][0] + points[2][0]) / 3.,
                            (points[0][2] + points[1][2] + points[2][2]) / 3.,
                        );
                        let patch = Patch {
                            chain: chain.id,
                            points,
                            color: sw.asphalt[if end == 0 { 0 } else { sw.asphalt.len() - 1 }],
                            era: sw.era,
                        };
                        clear.add_abutment(chain.id, points);
                        for lod in &mut tiles.entry(key).or_default().merges {
                            lod.push(patch.clone());
                        }
                    }
                }
            }
        }
    }
}
