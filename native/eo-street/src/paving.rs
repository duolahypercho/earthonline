//! Resolve overlapping street surfaces and shoulders, including along a tile
//! boundary. The comparison uses the triangles that the neighbouring tile draws.
use crate::{Layer, StreetWorld, TileMesh, road_openings::RoadOpenings};
use std::{collections::HashMap, sync::Arc};

type Triangle = [[f32; 3]; 3];
type Key = (i32, i32);

#[derive(Clone, Copy)]
struct Bounds {
    low: [f32; 3],
    high: [f32; 3],
}

impl Bounds {
    fn of(t: &Triangle) -> Self {
        Self {
            low: std::array::from_fn(|k| t[0][k].min(t[1][k]).min(t[2][k])),
            high: std::array::from_fn(|k| t[0][k].max(t[1][k]).max(t[2][k])),
        }
    }
    fn cells(&self) -> impl Iterator<Item = Key> {
        let key = |v: f32| (v / 8.).floor() as i32;
        (key(self.low[0])..=key(self.high[0]))
            .flat_map(move |x| (key(self.low[2])..=key(self.high[2])).map(move |z| (x, z)))
    }
    fn meets(&self, other: &Self) -> bool {
        [0, 2]
            .iter()
            .all(|&k| self.low[k] < other.high[k] && self.high[k] > other.low[k])
    }
}

// At coincident surfaces the through roadway continues across the pad,
// followed by the crossing ramp, kerb top, and sidewalk. Raised curbs and
// roads on different levels fall outside the paint clearance and stay intact.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Surface {
    Road,
    Junction,
    Ramp,
    Curb,
    Walk,
    Bank,
}

impl Surface {
    fn of(mesh: &TileMesh, layer: Layer, first: u32) -> Option<Self> {
        match layer {
            Layer::Carriageway
                if mesh.colors[first as usize] & 0xffffff == crate::mesh::pad_rgb() =>
            {
                Some(Self::Junction)
            }
            Layer::Carriageway => Some(Self::Road),
            Layer::CurbTop => Some(Self::Curb),
            Layer::Ramp => Some(Self::Ramp),
            Layer::Sidewalk => Some(Self::Walk),
            Layer::Verge => Some(Self::Bank),
            _ => None,
        }
    }
}

struct Face {
    triangle: Triangle,
    bounds: Bounds,
    slope: [f64; 2],
    kind: Surface,
    ordinal: usize,
}

#[derive(Default)]
struct Cuts {
    exposed: RoadOpenings,
    cover: RoadOpenings,
}

struct Target {
    face: usize,
    ordinal: usize,
    kind: Surface,
    triangle: Triangle,
    bounds: Bounds,
}

fn touches_interior(a: &Triangle, b: &Triangle) -> bool {
    let separated = |a: &Triangle, b: &Triangle| {
        (0..3).any(|i| {
            let p = a[i].map(f64::from);
            let q = a[(i + 1) % 3].map(f64::from);
            let side = |v: [f32; 3]| {
                (q[0] - p[0]) * (f64::from(v[2]) - p[2]) - (q[2] - p[2]) * (f64::from(v[0]) - p[0])
            };
            let inward = side(a[(i + 2) % 3]).signum();
            b.iter().all(|&v| side(v) * inward <= 1e-9)
        })
    };
    !separated(a, b) && !separated(b, a)
}

#[derive(Default)]
pub(crate) struct PavingTile {
    faces: Vec<Face>,
    cells: HashMap<Key, Vec<usize>>,
}

impl PavingTile {
    fn build(mesh: &TileMesh) -> Self {
        let mut tile = Self::default();
        let mut ordinal = 0;
        for (face, &layer) in mesh.indices.chunks_exact(3).zip(&mesh.layers) {
            let Some(kind) = Surface::of(mesh, layer, face[0]) else {
                continue;
            };
            if kind == Surface::Bank {
                continue;
            }
            ordinal += 1;
            let tri = [face[0], face[1], face[2]].map(|i| mesh.positions[i as usize]);
            let [a, b, c] = tri.map(|v| v.map(f64::from));
            let u = std::array::from_fn::<_, 3, _>(|i| b[i] - a[i]);
            let v = std::array::from_fn::<_, 3, _>(|i| c[i] - a[i]);
            let det = u[0] * v[2] - u[2] * v[0];
            if det.abs() < 1e-9 {
                continue;
            }
            let slope = [
                (u[1] * v[2] - u[2] * v[1]) / det,
                (u[0] * v[1] - u[1] * v[0]) / det,
            ];
            let bounds = Bounds::of(&tri);
            for k in bounds.cells() {
                tile.cells.entry(k).or_default().push(tile.faces.len());
            }
            tile.faces.push(Face {
                triangle: tri,
                bounds,
                slope,
                kind,
                ordinal,
            });
        }
        tile
    }
    fn add_cuts(
        &self,
        source: Key,
        owner: Key,
        targets: &[Target],
        cuts: &mut HashMap<usize, Cuts>,
        o: &crate::StreetOptions,
    ) {
        let mut visited = vec![usize::MAX; self.faces.len()];
        for t in targets {
            for k in t.bounds.cells() {
                let Some(faces) = self.cells.get(&k) else {
                    continue;
                };
                for &i in faces {
                    if visited[i] == t.face {
                        continue;
                    }
                    visited[i] = t.face;
                    let f = &self.faces[i];
                    if (f.kind, source, f.ordinal) >= (t.kind, owner, t.ordinal)
                        || !t.bounds.meets(&f.bounds)
                    {
                        continue;
                    }
                    if t.kind == Surface::Bank
                        && matches!(f.kind, Surface::Road | Surface::Junction)
                        && f.bounds.high[1] - t.bounds.low[1] > o.marking_lift as f32
                        && f.bounds.low[1] - t.bounds.high[1] < o.curb_face_height as f32
                        && touches_interior(&t.triangle, &f.triangle)
                    {
                        cuts.entry(t.face).or_default().cover.add_surface(
                            f.triangle,
                            o.curb_face_height,
                            -o.marking_lift,
                        );
                    }
                    // A shoulder just beneath the asphalt still competes with
                    // the road at the tire contact. Give the pavement the
                    // same clearance underneath as at a paved junction.
                    let below = o.marking_lift;
                    let above = if t.kind == Surface::Bank {
                        o.verge_max_drop
                    } else {
                        o.junction_paint_lift
                    };
                    if f64::from(t.bounds.high[1] - f.bounds.low[1]) < -below
                        || f64::from(t.bounds.low[1] - f.bounds.high[1]) > above
                    {
                        continue;
                    }
                    let a = f.triangle[0].map(f64::from);
                    let difference = t.triangle.map(|p| {
                        f64::from(p[1])
                            - a[1]
                            - f.slope[0] * (f64::from(p[0]) - a[0])
                            - f.slope[1] * (f64::from(p[2]) - a[2])
                    });
                    if difference.iter().all(|d| *d <= -below + 1e-6)
                        || difference.iter().all(|d| *d >= above - 1e-6)
                        || (t.kind != Surface::Bank && !touches_interior(&t.triangle, &f.triangle))
                    {
                        continue;
                    }
                    // When the bank descends to this street, the whole slope
                    // above the paved footprint must end at its edge. Cutting
                    // off only the toe leaves an overhang above the lane.
                    // A bank wholly on another level was excluded above.
                    let above = if t.kind == Surface::Bank {
                        above.max(f64::from(t.bounds.high[1]) - f64::from(f.bounds.low[1]))
                    } else {
                        above
                    };
                    cuts.entry(t.face).or_default().exposed.add_paving_face(
                        f.triangle,
                        below,
                        above,
                        t.kind == Surface::Bank,
                    );
                }
            }
        }
    }
}

impl StreetWorld {
    fn paving_tile(&self, key: Key, existing: Option<&TileMesh>) -> Arc<PavingTile> {
        if let Some(tile) = self.paving_cache.lock().unwrap().get(&key) {
            return tile.clone();
        }
        // This path emits only hard paving and never calls back into the bank
        // pass. A tile can enter either cache first, including on another thread.
        let tile = Arc::new(match existing {
            Some(mesh) => PavingTile::build(mesh),
            None => PavingTile::build(&self.draw_tile(key, false)),
        });
        let mut cache = self.paving_cache.lock().unwrap();
        if cache.len() >= 512 {
            cache.clear();
        }
        cache.entry(key).or_insert(tile).clone()
    }
    pub(crate) fn finish_paving(&self, owner: Key, mesh: &mut TileMesh) {
        let mut ordinal = 0;
        let mut targets = vec![];
        for (face, (ids, &layer)) in mesh.indices.chunks_exact(3).zip(&mesh.layers).enumerate() {
            let Some(kind) = Surface::of(mesh, layer, ids[0]) else {
                continue;
            };
            if kind != Surface::Bank {
                ordinal += 1;
            }
            if kind == Surface::Road {
                continue;
            }
            let triangle = [ids[0], ids[1], ids[2]].map(|i| mesh.positions[i as usize]);
            targets.push(Target {
                face,
                ordinal,
                kind,
                triangle,
                bounds: Bounds::of(&triangle),
            });
        }
        if targets.is_empty() {
            return;
        }
        let mut cuts = HashMap::new();
        let mut owners = Vec::new();
        for target in &targets {
            let b = target.bounds;
            let a = crate::tile_key(b.low[0].into(), b.low[2].into());
            let c = crate::tile_key(b.high[0].into(), b.high[2].into());
            for x in a.0..=c.0 {
                for z in a.1..=c.1 {
                    if let Some(keys) = self.paving_owners.get(&(x, z)) {
                        owners.extend_from_slice(keys);
                    }
                }
            }
        }
        owners.sort_unstable();
        owners.dedup();
        for k in owners {
            self.paving_tile(k, (k == owner).then_some(&*mesh))
                .add_cuts(k, owner, &targets, &mut cuts, &self.o);
        }
        if !cuts.is_empty() {
            mesh.clip_faces_with(0, |i, _, tri| {
                let cuts = cuts.get(&i)?;
                if cuts.exposed.is_empty() {
                    return None;
                }
                cuts.exposed.clip_covered(
                    tri,
                    |_| true,
                    (!cuts.cover.is_empty()).then_some(&cuts.cover),
                )
            });
        }
    }
    /// Register all tiles that a span's finished paving can reach. Derived
    /// from the same surveyed width and frontage limit as the street builder.
    pub(crate) fn index_paving_owners(&self) -> HashMap<Key, Vec<Key>> {
        let mut map: HashMap<Key, Vec<Key>> = HashMap::new();
        for (&owner, refs) in &self.tiles {
            let mut register = |points: &[[f64; 2]]| {
                let lo = std::array::from_fn::<_, 2, _>(|k| {
                    points.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min)
                });
                let hi = std::array::from_fn::<_, 2, _>(|k| {
                    points
                        .iter()
                        .map(|p| p[k])
                        .fold(f64::NEG_INFINITY, f64::max)
                });
                let a = crate::tile_key(lo[0], lo[1]);
                let b = crate::tile_key(hi[0], hi[1]);
                for x in a.0..=b.0 {
                    for z in a.1..=b.1 {
                        let ids = map.entry((x, z)).or_default();
                        if ids.last() != Some(&owner) {
                            ids.push(owner);
                        }
                    }
                }
            };
            for &(ei, i) in &refs.spans {
                let e = &self.plan.entries[ei as usize];
                let g = &self.geoms[ei as usize];
                let width = e.half
                    + e.walks[0]
                        .max(e.walks[1])
                        .max(self.o.ramp_run + self.o.curb_top_width);
                let extent = (width + self.o.frontage_reach).max(
                    self.city.segments[e.road].right_of_way.unwrap_or(0.) * 0.5
                        + self.o.frontage_past_line,
                );
                let a = g.stations[i as usize];
                let b = g.stations[i as usize + 1];
                register(&[a.at(-extent), a.at(extent), b.at(-extent), b.at(extent)]);
            }
            for &n in &refs.nodes {
                register(&self.corner_bounds[n as usize]);
            }
        }
        map
    }
}
