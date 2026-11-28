//! Restore the street plan's missing junctions on the simplified road ways.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use eo_data::sim_source::{SimRoad, TrafficJunction};

use super::{SECTION_MATCH_M, qkey, seg_nearest};
use crate::geom::{self, P2};
use crate::network::{JunctionSurface, NODE_TOLERANCE_M, NodeKey, node_key};

/// The native graph drops shorter segments as degenerate.
pub(super) const MIN_SEGMENT_M: f64 = 0.5;

#[derive(Default)]
pub(super) struct DrawnJunctions {
    pads: Vec<Pad>,
    by_way: HashMap<String, Vec<Arc<JunctionSurface>>>,
    aligned: HashMap<String, AlignedRoad>,
    split_nodes: Option<HashSet<(i64, i64)>>,
}

struct Pad {
    ways: Vec<String>,
    surface: Arc<JunctionSurface>,
}

/// Parallel arrays. Each junction has its own node position and keeps the
/// projected point on this road's centreline as geometry.
pub(super) struct AlignedRoad {
    pub points: Vec<P2>,
    pub nodes: Vec<P2>,
}

#[derive(Clone, Copy)]
struct Site {
    arc: f64,
    point: P2,
    surveyed_node: bool,
}

#[derive(Default)]
struct Vertices(HashMap<NodeKey, Vec<(usize, P2)>>);

impl Vertices {
    fn near(&self, p: P2) -> impl Iterator<Item = (usize, P2)> + '_ {
        let (x, z) = node_key(p);
        (-1..=1).flat_map(move |dx| {
            (-1..=1).flat_map(move |dz| self.0.get(&(x + dx, z + dz)).into_iter().flatten())
        }).copied().filter(move |&(_, q)| distance(p, q) <= NODE_TOLERANCE_M)
    }
}

fn distance(a: P2, b: P2) -> f64 {
    geom::hypot(a[0] - b[0], a[1] - b[1])
}

fn site(index: usize, points: &[P2], cum: &[f64], pad: &JunctionSurface, vertices: &Vertices) -> Option<Site> {
    let [x, z] = pad.position;
    let (edge, (_, projected)) = points.windows(2).enumerate()
        .map(|(i, w)| (i, seg_nearest(w[0], w[1], x, z)))
        .min_by(|a, b| a.1.0.total_cmp(&b.1.0))?;
    let on_pad = |p| distance(p, pad.position) <= SECTION_MATCH_M && pad.height_near(p, 0.0).is_some();
    let shared_node = |point| vertices.near(point)
        .any(|(owner, p)| owner != index && qkey(p) == qkey(point));
    if !on_pad(projected) {
        // The named way can end early in the paved fan after map simplification.
        // Accept its endpoint on the actual fan. Keep the tighter centre check
        // for a through way, where projecting to the wrong arm would split it.
        let last = points.len() - 1;
        let i = [0, last].into_iter().find(|&i| {
            (i == edge || i == edge + 1)
                && distance(points[i], projected) <= MIN_SEGMENT_M
                && pad.height_near(points[i], 0.0).is_some()
        })?;
        let point = points[i];
        return Some(Site { arc: cum[i], point, surveyed_node: shared_node(point) });
    }
    let arc = cum[edge] + distance(points[edge], projected);
    // Preserve a shared mapped node when a neighbouring vertex already
    // supplies one within this pad. Otherwise avoid producing a stub.
    (edge..=edge + 1).filter_map(|i| {
        let point = points[i];
        let surveyed_node = on_pad(point) && shared_node(point);
        (surveyed_node || (cum[i] - arc).abs() <= MIN_SEGMENT_M)
            .then_some(Site { arc: cum[i], point, surveyed_node })
    }).min_by(|a, b| b.surveyed_node.cmp(&a.surveyed_node)
        .then((a.arc - arc).abs().total_cmp(&(b.arc - arc).abs())))
        .or(Some(Site { arc, point: projected, surveyed_node: false }))
}

impl DrawnJunctions {
    pub(super) fn new(input: Vec<TrafficJunction>) -> Self {
        let mut junctions = Self::default();
        for mut pad in input {
            let Some(surface) = JunctionSurface::from_drawn(&pad).map(Arc::new) else { continue };
            pad.ways.sort_unstable();
            pad.ways.dedup();
            for way in &pad.ways {
                junctions.by_way.entry(way.clone()).or_default().push(Arc::clone(&surface));
            }
            if pad.ways.len() >= 2 {
                junctions.pads.push(Pad { ways: pad.ways, surface });
            }
        }
        junctions
    }

    pub(super) fn has_pads(&self) -> bool {
        !self.pads.is_empty()
    }

    pub(super) fn pads(&self, way: &str) -> Option<&[Arc<JunctionSurface>]> {
        self.by_way.get(way).map(Vec::as_slice)
    }

    pub(super) fn share_pad_at(&self, way: &str, other: &str, nodes: [P2; 2]) -> bool {
        let (Some(own_pads), Some(other_pads)) = (self.pads(way), self.pads(other)) else { return false };
        own_pads.iter().any(|pad| {
            nodes.into_iter().any(|node| distance(node, pad.position) <= SECTION_MATCH_M)
                && other_pads.iter().any(|candidate| Arc::ptr_eq(candidate, pad))
        })
    }

    pub(super) fn road(&self, way: &str) -> Option<&AlignedRoad> {
        self.aligned.get(way)
    }

    pub(super) fn split_nodes(&self) -> Option<&HashSet<(i64, i64)>> {
        self.split_nodes.as_ref()
    }

    pub(super) fn align_roads(&mut self, roads: &[SimRoad], levels: &HashMap<String, i8>) {
        self.aligned.clear();
        self.split_nodes = None;
        if self.pads.is_empty() {
            return;
        }
        let mut vertices = Vertices::default();
        let mut owners = HashMap::new();
        let mut split_nodes = HashSet::new();
        let mut eligible = HashMap::new();
        for (i, road) in roads.iter().enumerate() {
            for &p in &road.points {
                vertices.0.entry(node_key(p)).or_default().push((i, p));
                let key = qkey(p);
                if owners.insert(key, i).is_some_and(|last| last != i) {
                    split_nodes.insert(key);
                }
            }
            if self.by_way.contains_key(&road.id) && !road.bridge && !road.tunnel && road.layer == 0
                && levels.get(&road.id).copied().unwrap_or(0) == 0 && !geom::freeway(&road.highway)
            {
                eligible.insert(road.id.as_str(), (i, geom::cumulative(&road.points)));
            }
        }
        let mut edits = vec![Vec::<(Site, P2)>::new(); roads.len()];
        for pad in &self.pads {
            let sites: Vec<_> = pad.ways.iter().filter_map(|way| {
                let (i, cum) = eligible.get(way.as_str())?;
                site(*i, &roads[*i].points, cum, &pad.surface, &vertices).map(|site| (*i, site))
            }).collect();
            if sites.len() < 2 {
                continue;
            }
            let mut surveyed = sites.iter().filter(|(_, p)| p.surveyed_node).map(|(_, p)| p.point);
            let anchor = surveyed.next().unwrap_or(pad.surface.position);
            // The fan can cover several already-mapped junctions on a divided
            // street. Keep those nodes and their existing connections.
            if surveyed.any(|p| qkey(p) != qkey(anchor))
                || vertices.near(anchor).any(|(i, _)| !sites.iter().any(|(road, _)| *road == i))
                || sites.iter().any(|(i, site)| edits[*i].iter().any(|(other, _)| {
                    (other.arc - site.arc).abs() <= MIN_SEGMENT_M
                }))
            {
                continue;
            }
            split_nodes.insert(qkey(anchor));
            for (i, site) in sites {
                edits[i].push((site, anchor));
            }
        }
        // One way has the same splits and lane keys in every tile delivery,
        // even when a connecting way ends outside a delivery's tile ring.
        self.split_nodes = Some(split_nodes);

        for (road, mut edits) in roads.iter().zip(edits) {
            if edits.is_empty() {
                continue;
            }
            let (_, cum) = &eligible[road.id.as_str()];
            edits.sort_unstable_by(|(a, _), (b, _)| a.arc.total_cmp(&b.arc));
            let mut edits = edits.into_iter().peekable();
            let capacity = road.points.len() + edits.len();
            let mut aligned = AlignedRoad { points: Vec::with_capacity(capacity), nodes: Vec::with_capacity(capacity) };
            let mut changed = false;
            for (&point, &arc) in road.points.iter().zip(cum) {
                while let Some((site, node)) = edits.next_if(|(site, _)| site.arc < arc) {
                    aligned.points.push(site.point);
                    aligned.nodes.push(node);
                    changed = true;
                }
                let node = edits.next_if(|(site, _)| site.arc == arc).map_or(point, |(_, node)| node);
                changed |= point != node;
                aligned.points.push(point);
                aligned.nodes.push(node);
            }
            if changed {
                self.aligned.insert(road.id.clone(), aligned);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drawn_pad_connects_walk_clearance_at_the_near_approach() {
        let square = |x: f32, a: &str, b: &str| TrafficJunction {
            ways: vec![a.into(), b.into()],
            center: [x, 0., 0.],
            rim: vec![[x - 4., 0., -4.], [x + 4., 0., -4.], [x + 4., 0., 4.], [x - 4., 0., 4.]],
            triangles: vec![],
        };
        let junctions = DrawnJunctions::new(vec![square(0., "walk", "lane"), square(50., "walk", "other")]);
        let here = [[0., 0.], [10., 0.]];
        assert!(junctions.share_pad_at("walk", "lane", here));
        assert!(!junctions.share_pad_at("walk", "other", here));
        assert!(!junctions.share_pad_at("walk", "lane", [[20., 0.], [30., 0.]]));
    }

    #[test]
    fn a_side_street_ending_inside_the_drawn_junction_joins_the_through_road() {
        let road = |id: &str, points: Vec<P2>| SimRoad {
            id: id.into(),
            name: id.into(),
            highway: "residential".into(),
            oneway: eo_data::sim_source::Oneway::Both,
            lanes: Some(2),
            maxspeed: String::new(),
            sidewalk: "both".into(),
            bridge: false,
            tunnel: false,
            layer: 0,
            points,
        };
        let mut junctions = DrawnJunctions::new(vec![TrafficJunction {
            ways: vec!["side".into(), "through".into(), "parallel".into(), "short".into()],
            center: [0.0, 0.0, 0.0],
            rim: vec![[-6., 0., -6.], [6., 0., -6.], [6., 0., 6.], [-6., 0., 6.]],
            triangles: vec![],
        }]);
        let mouth = [-2.0, -4.0];
        let roads = [
            road("side", vec![[-2.0, -20.0], mouth]),
            road("through", vec![[-20.0, 0.0], [20.0, 0.0]]),
            road("parallel", vec![[-20.0, 4.0], [20.0, 4.0]]),
            road("short", vec![[2.0, -20.0], [2.0, -8.0]]),
        ];
        junctions.align_roads(&roads, &HashMap::new());
        let side = junctions.road("side").unwrap();
        let through = junctions.road("through").unwrap();
        assert_eq!(side.points[1], mouth);
        assert_eq!(side.nodes[1], through.nodes[1]);
        assert_eq!(through.points[1], [0.0, 0.0]);
        assert!(junctions.road("parallel").is_none());
        assert!(junctions.road("short").is_none());
    }
}
