//! Through streets and ramps have mapped carriageways through the gaps between
//! junctions and into the turning apron. A nearby corner return can project
//! over it, including the vertical back of the sidewalk. Cut the drawn
//! corner at the carriageway boundary so its floor and solid faces agree.
use crate::plan::{Node, frame_at};
use crate::{StreetWorld, TileMesh};

type Point = [f64; 2];
type Quad = [Point; 4];

fn side(a: Point, b: Point, p: [f32; 3]) -> f64 {
    (b[0] - a[0]) * (f64::from(p[2]) - a[1]) - (b[1] - a[1]) * (f64::from(p[0]) - a[0])
}

fn intersection(mesh: &mut TileMesh, a: u32, b: u32, t: f64) -> u32 {
    let (a, b) = (a as usize, b as usize);
    let mix = |a: f32, b: f32| (f64::from(a) + t * (f64::from(b) - f64::from(a))) as f32;
    let id = mesh.positions.len() as u32;
    mesh.positions.push(std::array::from_fn(|i| {
        mix(mesh.positions[a][i], mesh.positions[b][i])
    }));
    mesh.normals.push(std::array::from_fn(|i| {
        mix(mesh.normals[a][i], mesh.normals[b][i])
    }));
    let (ca, cb) = (mesh.colors[a].to_le_bytes(), mesh.colors[b].to_le_bytes());
    mesh.colors
        .push(u32::from_le_bytes(std::array::from_fn(|i| {
            mix(ca[i].into(), cb[i].into()).round() as u8
        })));
    mesh.kerb.push(mix(mesh.kerb[a], mesh.kerb[b]));
    id
}

/// Keep the portions outside the four half planes. Splitting in plan also
/// preserves vertical faces, which have zero area in a top-down projection.
fn subtract(mesh: &mut TileMesh, mut poly: Vec<u32>, quad: &Quad, output: &mut Vec<Vec<u32>>) {
    // If any edge separates the polygon from the road, keep the original
    // indices. A kerb coincident with the boundary belongs at the road edge.
    for i in 0..4 {
        if poly
            .iter()
            .all(|&v| side(quad[i], quad[(i + 1) % 4], mesh.positions[v as usize]) <= 0.0)
        {
            output.push(poly);
            return;
        }
    }
    for i in 0..4 {
        let distance =
            |mesh: &TileMesh, v| side(quad[i], quad[(i + 1) % 4], mesh.positions[v as usize]);
        if poly.iter().all(|&v| distance(mesh, v) >= 0.0) {
            continue;
        }
        let mut inside = Vec::new();
        let mut outside = Vec::new();
        let mut a = *poly.last().unwrap();
        let mut da = distance(mesh, a);
        for b in poly {
            let db = distance(mesh, b);
            if (da < 0.0 && db > 0.0) || (da > 0.0 && db < 0.0) {
                let v = intersection(mesh, a, b, da / (da - db));
                inside.push(v);
                outside.push(v);
            }
            if db >= 0.0 {
                inside.push(b);
            }
            if db <= 0.0 {
                outside.push(b);
            }
            a = b;
            da = db;
        }
        if outside.len() >= 3 {
            output.push(outside);
        }
        if inside.len() < 3 {
            return;
        }
        poly = inside;
    }
}

impl StreetWorld {
    /// Opposite directions of a divided arterial are separate mapped ways.
    /// A short connector between them can own a corner at *both* sides.
    pub(super) fn divided_name(&self, entry: usize) -> Option<&str> {
        let e = &self.plan.entries[entry];
        let road = &self.city.segments[e.road];
        (e.oneway
            && !road.separated
            && !road.street_name.is_empty()
            && matches!(e.highway.as_str(), "primary" | "secondary" | "trunk")
            && e.structure_leg.is_none())
        .then_some(road.street_name.as_str())
    }

    pub(crate) fn index_divided_carriageways(
        &self,
    ) -> std::collections::HashMap<String, crate::road_openings::RoadOpenings> {
        let mut roads = std::collections::HashMap::new();
        for (ei, e) in self.plan.entries.iter().enumerate() {
            let Some(name) = self.divided_name(ei) else {
                continue;
            };
            let g = &self.geoms[ei];
            let cuts = roads
                .entry(name.to_owned())
                .or_insert_with(crate::road_openings::RoadOpenings::default);
            let has_structure = !e.structure_ramps.is_empty()
                || !e.landings.is_empty()
                || e.structure_ends.iter().any(Option::is_some);
            for (i, &keep) in g.keep.iter().enumerate() {
                if !keep {
                    continue;
                }
                if has_structure
                    && [i, i + 1]
                        .iter()
                        .any(|&j| self.datum_raise(ei, j).abs() > self.o.curb_face_height)
                {
                    continue;
                }
                let edge = |j: usize| {
                    [-e.half, e.half].map(|u| {
                        let [x, z] = g.stations[j].at(u);
                        [x, g.pavement_y(&self.o, j, u, e.half), z]
                    })
                };
                // Limit the cut to the bank that can grade to this road.
                // Separate decks and surfaces on other storeys retain their floors.
                cuts.add(
                    ei as u32,
                    edge(i),
                    edge(i + 1),
                    self.o.verge_max_drop,
                    self.o.verge_max_drop,
                );
            }
        }
        for node in &self.plan.nodes {
            let mut pad = None;
            for arm in &node.approaches {
                let Some(name) = self.divided_name(arm.entry) else {
                    continue;
                };
                let e = &self.plan.entries[arm.entry];
                let Some(&mouth) = (if arm.at_start {
                    self.geoms[arm.entry].stations.first()
                } else {
                    self.geoms[arm.entry].stations.last()
                }) else {
                    continue;
                };
                let (start, end) = if arm.at_start {
                    (0., mouth.s)
                } else {
                    (mouth.s, e.length)
                };
                let frames =
                    crate::plan::build_stations(&e.points, &e.cum, start, end, self.o.max_step);
                let depth = self.o.verge_max_drop;
                let mut approach = crate::road_openings::RoadOpenings::default();
                for pair in frames.windows(2) {
                    let edge = |f: crate::plan::Frame| {
                        [-e.half, e.half].map(|u| {
                            let [x, z] = f.at(u);
                            [x, self.ground(x, z) + self.o.road_lift, z]
                        })
                    };
                    approach.add(arm.entry as u32, edge(pair[0]), edge(pair[1]), depth, depth);
                }
                let (centre, rim, triangles) = pad.get_or_insert_with(|| self.traffic_junction_pad(node));
                let cuts = roads.get_mut(name).expect("indexed arterial");
                let vertex = |i| if i == 0 { *centre } else { rim[i - 1] };
                for face in crate::mesh::junction::faces(rim.len(), triangles) {
                    cuts.add_pad_approach(arm.entry as u32, face.map(vertex), &approach, depth);
                }
            }
        }
        roads
    }

    fn trim_divided_road<'a>(
        &'a self,
        mesh: &mut TileMesh,
        first_face: usize,
        first_vertex: usize,
        owner: Option<usize>,
        names: impl Iterator<Item = &'a str>,
    ) {
        let mut names: Vec<_> = names.collect();
        if names.is_empty() || mesh.layers.len() == first_face {
            return;
        }
        names.sort_unstable();
        names.dedup();
        let points = &mesh.positions[first_vertex..];
        let lo = std::array::from_fn(|i| {
            points
                .iter()
                .map(|p| f64::from(p[i]))
                .fold(f64::INFINITY, f64::min)
        });
        let hi = std::array::from_fn(|i| {
            points
                .iter()
                .map(|p| f64::from(p[i]))
                .fold(f64::NEG_INFINITY, f64::max)
        });
        for name in names {
            let Some(cuts) = self.divided_carriageways.get(name) else {
                continue;
            };
            if cuts.intersects_other(lo, hi, owner) {
                mesh.clip_faces_with(first_face, |_, layer, triangle| {
                    // The earth grades down to the roadbed. A retaining wall
                    // can instead belong to the pavement below an overpass.
                    // Only open its upper face where it meets the carriageway.
                    let inset = if layer == crate::Layer::Verge { 0.0 } else {
                        self.o.verge_max_drop - self.o.curb_face_height
                    };
                    cuts.clip_from_base(triangle, |r| Some(r as usize) != owner, None, inset)
                });
            }
        }
    }

    pub(crate) fn trim_arterial_corner(
        &self,
        mesh: &mut TileMesh,
        node: &Node,
        first_face: usize,
        first_vertex: usize,
    ) {
        // On these short connectors the junction builder extends a sidewalk
        // return into a different mapped through lane. Retain the through
        // way's surveyed cross section. Other junctions have their own
        // pedestrian ramp surfaces, including on overlapping centre lines.
        // The ids refer to the shipped street survey and survive plan scaling.
        let through = node
            .approaches
            .iter()
            .find_map(|a| self.through_way(a.entry));
        if let Some(way) = through {
            self.trim_at_through_way(mesh, way, first_face, first_vertex);
        }
        if !self.divided_carriageways.is_empty() {
            self.trim_divided_road(
                mesh,
                first_face,
                first_vertex,
                None,
                node.approaches
                    .iter()
                    .filter_map(|a| self.divided_name(a.entry)),
            );
        }
    }

    pub(crate) fn trim_arterial_footway(
        &self,
        mesh: &mut TileMesh,
        entry: usize,
        first_face: usize,
        first_vertex: usize,
    ) {
        if let Some(way) = self.through_way(entry) {
            self.trim_at_through_way(mesh, way, first_face, first_vertex);
        }
        if self.divided_carriageways.is_empty() {
            return;
        }
        let e = &self.plan.entries[entry];
        if self.city.segments[e.road].separated {
            return;
        }
        let own = self.divided_name(entry);
        // Between the two carriageways, the cross street has an approach
        // on each side with the same arterial name.
        let connected =
            e.approach_start
                .zip(e.approach_end)
                .into_iter()
                .flat_map(|((a, _), (b, _))| {
                    self.plan.nodes[a]
                        .approaches
                        .iter()
                        .filter_map(|arm| self.divided_name(arm.entry))
                        .filter(move |name| {
                            self.plan.nodes[b].approaches.iter().any(|arm| {
                                arm.entry != entry && self.divided_name(arm.entry) == Some(*name)
                            })
                        })
                });
        self.trim_divided_road(
            mesh,
            first_face,
            first_vertex,
            Some(entry),
            own.into_iter().chain(connected),
        );
    }

    fn through_way(&self, entry: usize) -> Option<&'static str> {
        match self.city.segments[self.plan.entries[entry].road].street_id.as_deref()? {
            "sf-street-25337673" => Some("sf-street-119237657"), // Mission at the US 101 exit
            "sf-street-204965181" => Some("sf-street-87554961"), // Market at Octavia
            _ => None,
        }
    }

    fn trim_at_through_way(&self, mesh: &mut TileMesh, through: &str, first_face: usize, first_vertex: usize) {
        let vertices = &mesh.positions[first_vertex..];
        if vertices.is_empty() {
            return;
        }
        let mut lo = [f32::INFINITY; 2];
        let mut hi = [f32::NEG_INFINITY; 2];
        for p in vertices {
            for (i, axis) in [0, 2].into_iter().enumerate() {
                lo[i] = lo[i].min(p[axis]);
                hi[i] = hi[i].max(p[axis]);
            }
        }
        let cell = |x: f32| (x / 32.0).floor() as i32;
        let mut pieces = Vec::new();
        for z in cell(lo[1])..=cell(hi[1]) {
            for x in cell(lo[0])..=cell(hi[0]) {
                if let Some(ids) = self.road_cells.get(&(x, z)) {
                    for &id in ids {
                        let owner = self.road_pieces[id as usize].0 as usize;
                        let road = &self.city.segments[self.plan.entries[owner].road];
                        if !road.separated && road.street_id.as_deref() == Some(through) {
                            pieces.push(id);
                        }
                    }
                }
            }
        }
        pieces.sort_unstable();
        pieces.dedup();
        let mut quads = Vec::new();
        for id in pieces {
            let (owner, a, _, half) = self.road_pieces[id as usize];
            let e = &self.plan.entries[owner as usize];
            let j = super::seg_index(e, a);
            // A run can give up part of its carriageway to another street.
            // Only its surviving length can displace a corner return.
            let (mut start, end) = (e.cum[j], e.cum[j + 1]);
            let mut add = |start: f64, end: f64| {
                if end <= start {
                    return;
                }
                let from = frame_at(&e.points, &e.cum, start, true);
                let to = frame_at(&e.points, &e.cum, end, true);
                let q = [from.at(-half), to.at(-half), to.at(half), from.at(half)];
                if (0..2).all(|i| {
                    q.iter().any(|p| p[i] >= f64::from(lo[i]))
                        && q.iter().any(|p| p[i] <= f64::from(hi[i]))
                }) {
                    quads.push(q);
                }
            };
            for &[a, b] in &e.yield_spans {
                if a >= end {
                    break;
                }
                if b > start {
                    add(start, a);
                    start = b;
                }
            }
            add(start, end);
        }
        if quads.is_empty() {
            return;
        }

        let faces = mesh.indices.split_off(first_face * 3);
        let layers = mesh.layers.split_off(first_face);
        for (tri, layer) in faces.chunks_exact(3).zip(layers) {
            let mut pieces = vec![tri.to_vec()];
            for q in &quads {
                let mut next = Vec::new();
                for p in pieces {
                    subtract(mesh, p, q, &mut next);
                }
                pieces = next;
                if pieces.is_empty() {
                    break;
                }
            }
            for p in pieces {
                for i in 1..p.len() - 1 {
                    let indices = [p[0], p[i], p[i + 1]];
                    let [a, b, c] = indices.map(|i| mesh.positions[i as usize].map(f64::from));
                    if crate::mesh::tri_area(&a, &b, &c) <= 1e-9 {
                        continue;
                    }
                    mesh.indices.extend_from_slice(&indices);
                    mesh.layers.push(layer);
                }
            }
        }
    }
}
