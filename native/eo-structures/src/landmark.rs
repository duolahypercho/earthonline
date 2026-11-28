//! The two landmark bridges' decks (eo-landmarks), as the solve sees them:
//! the ways each carries, the stretch its own deck draws (in its own
//! chainage), and its deck height there (`deck_top`, the Bay Bridge's lower
//! deck `lower_top`). Ways a landmark carries are not built as structures;
//! they become landmark chains whose profile is the landmark's own, so a
//! structure that meets them takes the landmark's height at the meeting.

use std::collections::HashMap;
use std::rc::Rc;

use eo_data::ElevationGrid;
use eo_data::sim_source::TrafficDeckLane;
use eo_landmarks::{LandmarksJs, bay_bridge, golden_gate, suspension};
use eo_street::plan::P2;

use crate::{LandmarkDeck, RoadTags};

/// The Golden Gate deck's table step (chainage metres).
const GG_TABLE_STEP_M: f64 = 0.25;

/// One landmark deck.
pub(crate) struct LandmarkSpec {
    pub deck: LandmarkDeck,
    /// OSM way ids it carries (`carried_ways`).
    pub ways: Vec<String>,
    /// Centrelines whose corridor (`corridor_half` either side) it also
    /// carries, whatever their way id (the Golden Gate's `carriageways`).
    pub corridor: Vec<Vec<P2>>,
    pub corridor_half: f64,
    /// The chainage stretch its deck is drawn over.
    pub c0: f64,
    pub c1: f64,
    pub chainage: Box<dyn Fn(f64, f64) -> f64>,
    /// Deck top at a chainage, game frame.
    pub top: Box<dyn Fn(f64) -> f64>,
}

/// The northbound survey way ends past the nominal Golden Gate deck line.
/// Its remaining short chain still participates in routing and the profile
/// solve. The Marin apron supplies its asphalt and wheel surface; building a
/// generic deck here also installs two concrete parapets inside the highway.
pub(crate) fn marin_apron_chains(js: &LandmarksJs, chains: &[crate::Chain]) -> Vec<u32> {
    if !golden_gate::approach_finish() || !js.registry.contains_key("golden-gate-bridge") {
        return Vec::new();
    }
    let gate = golden_gate::data(js);
    let site = golden_gate::Site::new(&gate);
    let tip = gate.way_ends["north"];
    let seam = site.chainage(tip[0],tip[1]);
    let end = gate.carriageways.iter().flat_map(|w| w.chunks_exact(2))
        .map(|p| site.chainage(p[0],p[1])).fold(seam,f64::max);
    let carried: Vec<_> = gate.carried_ways.iter().map(ToString::to_string).collect();
    chains.iter().filter(|chain| {
        chain.kind == crate::ChainKind::Deck && !chain.pieces.is_empty()
            && chain.pieces.iter().all(|piece| carried.contains(&piece.way))
            && chain.points.iter().all(|p| (seam-1e-5..=end+1e-5).contains(&site.chainage(p[0],p[1])))
    }).map(|chain| chain.id).collect()
}

/// Use the same frame, lane spacing and panel stations as the Golden Gate's
/// center stripes. The two mapped centrelines identify each carriageway and
/// its route endpoints, which are staggered at the abutments.
pub(crate) fn golden_gate_traffic_lanes(
    js: &LandmarksJs,
    streets: &eo_street::StreetWorld,
    ca: &suspension::DeckStandards,
    roads: &HashMap<&str, &eo_data::Road>,
    world: &crate::StructureWorld,
) -> HashMap<String, Vec<TrafficDeckLane>> {
    let mut lanes = HashMap::new();
    if !js.registry.contains_key("golden-gate-bridge") || js.golden_gate.carriageways.len() < 2 {
        return lanes;
    }
    let g = golden_gate::data(js);
    let site = golden_gate::Site::new(&g);
    let sample = |x, z| streets.terrain.grid.sample(x, z);
    let ground = eo_landmarks::set::ground_of(js, "golden-gate-bridge", &sample);
    let p = golden_gate::plan(js, &site, &|x, z| ground.at(x, z));
    let num = |key| eo_landmarks::data::num(&g.dimensions, key);
    let count = num("lanesPerDirection") as u32;
    let width = ca.median_lane_width(num("roadwayM"), f64::from(count));
    let panels = suspension::stations(p.c_south, p.c_north, num("panelM"));
    let chainage = |[x, z]: P2| site.chainage(x, z);
    for chain in world.chains().iter().filter(|c| c.kind == crate::ChainKind::Landmark && c.landmark == Some(LandmarkDeck::GoldenGate)) {
        for way in chain.ways() {
            let Some(road) = roads.get(way) else { continue };
            if road.lanes != count || road.points64.len() < 2 || lanes.contains_key(way) {
                continue;
            }
            let start = chainage(road.points64[0]);
            let end = chainage(*road.points64.last().unwrap());
            let [lo, hi] = [start.min(end), start.max(end)];
            if lo == hi {
                continue;
            }
            let middle = road.points64[road.points64.len() / 2];
            let centre = site.deck_point(chainage(middle), 0.0);
            let across = (middle[0] - centre[0]) * site.across[0] + (middle[1] - centre[1]) * site.across[1];
            let stations = std::iter::once(lo)
                .chain(panels.iter().copied().filter(|&c| c > lo && c < hi))
                .chain(std::iter::once(hi));
            let tracks = (0..count).map(|k| {
                let offset = across.signum() * (ca.median_edge() + (f64::from(k) + 0.5) * width);
                let mut points: Vec<_> = stations.clone().map(|c| {
                    let [x, z] = site.deck_point(c, offset);
                    let y = world.deck_top(chain.id, x, z).unwrap_or_else(|| p.deck_top(c));
                    [x, y, z]
                }).collect();
                if start > end {
                    points.reverse();
                }
                TrafficDeckLane { width, points }
            }).collect();
            lanes.insert(way.to_string(), tracks);
        }
    }
    lanes
}

impl LandmarkSpec {
    /// Every point of `points` lies in its corridor.
    pub fn in_corridor(&self, points: &[P2]) -> bool {
        !self.corridor.is_empty()
            && points.iter().all(|&p| {
                self.corridor.iter().any(|line| line.windows(2).any(|w| crate::index::project(p, w[0], w[1]).1 <= self.corridor_half))
            })
    }
}

/// The decks of the Golden Gate and the Bay Bridge west span, on the ground
/// the landmark generators build on (`set::ground_of` over the elevation
/// grid's sampler, as eo-app hands `LandmarkSet::build_with`). A landmark the
/// registry does not list is left out.
pub(crate) fn specs(js: &LandmarksJs, grid: &ElevationGrid, roads: &HashMap<String, RoadTags>) -> Vec<LandmarkSpec> {
    let mut out = vec![];
    let sample = |x: f64, z: f64| grid.sample(x, z);
    if js.registry.contains_key("golden-gate-bridge") && js.golden_gate.carriageways.len() >= 2 {
        let g = &js.golden_gate;
        let ground = eo_landmarks::set::ground_of(js, "golden-gate-bridge", &sample);
        let site = Rc::new(golden_gate::Site::new(g));
        let p = golden_gate::plan(js, &site, &|x, z| ground.at(x, z));
        let (c0, c1) = (p.c_south.min(p.c_north), p.c_south.max(p.c_north));
        // `Plan` borrows the site, so its deck is read here into a table every
        // `GG_TABLE_STEP_M` (linear between: within 5 mm of `deck_top` at its
        // grade breaks, exact elsewhere to well under a millimetre).
        let n = (((c1 - c0) / GG_TABLE_STEP_M).ceil() as usize).max(1);
        let table: Vec<f64> = (0..=n).map(|i| p.deck_top(c0 + (c1 - c0) * i as f64 / n as f64)).collect();
        let s1 = site.clone();
        let half = eo_landmarks::data::num_opt(&g.dimensions, "widthM").unwrap_or(0.0) / 2.0;
        out.push(LandmarkSpec {
            deck: LandmarkDeck::GoldenGate,
            ways: g.carried_ways.iter().map(|w| w.to_string()).collect(),
            corridor: g.carriageways.iter().map(|c| c.chunks_exact(2).map(|q| [q[0], q[1]]).collect()).collect(),
            corridor_half: half,
            c0,
            c1,
            chainage: Box::new(move |x, z| s1.chainage(x, z)),
            top: Box::new(move |c| {
                let f = ((c - c0) / (c1 - c0) * n as f64).clamp(0.0, n as f64);
                let i = (f.floor() as usize).min(n.saturating_sub(1));
                let t = f - i as f64;
                table[i] + (table[(i + 1).min(n)] - table[i]) * t
            }),
        });
    }
    if js.registry.contains_key("bay-bridge-west") && !js.bay_bridge_west.carried_ways.is_empty() {
        let b = &js.bay_bridge_west;
        let ground = eo_landmarks::set::ground_of(js, "bay-bridge-west", &sample);
        let plan = Rc::new(bay_bridge::plan(js, &|x, z| ground.at(x, z)));
        // the upper deck carries the way on the higher OSM layer (westbound,
        // layer 2); the lower deck (`lower_top`, `deck_step_m` below) the other.
        let mut ways: Vec<(i32, usize, String)> = b.carried_ways.iter().enumerate().map(|(i, w)| (roads.get(&w.to_string()).map_or(0, |r| r.layer), i, w.to_string())).collect();
        ways.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        for (k, (_, _, way)) in ways.into_iter().enumerate() {
            let f = bay_bridge::frame(b);
            let p = plan.clone();
            let upper = k == 0;
            out.push(LandmarkSpec {
                deck: if upper { LandmarkDeck::BayBridgeUpper } else { LandmarkDeck::BayBridgeLower },
                ways: vec![way],
                corridor: vec![],
                corridor_half: 0.0,
                c0: plan.c_a.min(plan.c_b),
                c1: plan.c_a.max(plan.c_b),
                chainage: Box::new(move |x, z| f.chainage(x, z)),
                top: Box::new(move |c| if upper { p.deck_top(c) } else { p.lower_top(c) }),
            });
        }
    }
    out
}
