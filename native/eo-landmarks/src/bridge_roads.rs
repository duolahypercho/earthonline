//! Contact geometry from the panels and traffic barriers of the drawn suspension bridges.
use crate::data::num;
use crate::geometry::{MeshArrays, NO_BOTTOM, Rgb, V3};
use crate::suspension::{DeckStandards, Frame, stations};
use crate::{LandmarksJs, bay_bridge, golden_gate};

/// A drivable level of a landmark bridge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LandmarkDeck {
    GoldenGate,
    BayBridgeUpper,
    BayBridgeLower,
}

impl LandmarkDeck {
    pub fn name(self) -> &'static str {
        match self {
            LandmarkDeck::GoldenGate => "Golden Gate Bridge deck",
            LandmarkDeck::BayBridgeUpper => "Bay Bridge upper deck",
            LandmarkDeck::BayBridgeLower => "Bay Bridge lower deck",
        }
    }
}

pub type Triangle = [V3; 3];

pub struct RoadSurface {
    pub deck: LandmarkDeck,
    pub road: Vec<Triangle>,
    /// First triangles in `road` that carry traffic. Access housings are also
    /// solid horizontal surfaces, but do not participate in lane-height queries.
    pub carriageway_faces: usize,
    pub barrier: Vec<Triangle>,
}

pub(crate) fn asphalt(frame: &Frame, a: f64, b: f64, ta: f64, tb: f64, half: f64) -> [V3; 4] {
    [
        frame.P(a, -half, ta),
        frame.P(a, half, ta),
        frame.P(b, half, tb),
        frame.P(b, -half, tb),
    ]
}

pub(crate) fn median(
    mesh: &mut MeshArrays,
    frame: &Frame,
    c: [f64; 2],
    top: [f64; 2],
    ends: [bool; 2],
    ca: &DeckStandards,
    colour: Rgb,
) {
    // Each end follows its own road station and grade on the curved approaches.
    let ring = |height: f64, width: f64| {
        asphalt(
            frame,
            c[0],
            c[1],
            top[0] + height * ca.k_height,
            top[1] + height * ca.k_height,
            width / 2.,
        )
    };
    median_profile(mesh, ends, ca, colour, ring);
}

pub(crate) fn median_profile(
    mesh: &mut MeshArrays,
    ends: [bool; 2],
    ca: &DeckStandards,
    colour: Rgb,
    ring: impl Fn(f64, f64) -> [V3; 4],
) {
    let body = ca.k_width;
    if !crate::golden_gate::approach_finish() {
        mesh.hexahedron(ring(0., body), ring(1., body), colour, NO_BOTTOM);
        return;
    }
    // The District's Finding of Effect §2.3, plate 1. Body, foot and total
    // height are dimensioned; intermediate profile proportions follow the drawing.
    let neck = 0.56 * body;
    let profile = [
        (0., ca.foot_width),
        (0.025, ca.foot_width),
        (0.065, body),
        (0.58, body),
        (0.69, neck),
        (0.83, neck),
        (0.85, body),
        (1., body * 0.91),
    ];
    let mut low = ring(profile[0].0, profile[0].1);
    for &(height, width) in &profile[1..] {
        let high = ring(height, width);
        for (a, b) in [(1, 2), (3, 0)] {
            mesh.quad(low[a], low[b], high[b], high[a], colour);
        }
        for (cap, a, b) in [(ends[0], 0, 1), (ends[1], 2, 3)] {
            if cap {
                mesh.quad(low[a], low[b], high[b], high[a], colour);
            }
        }
        low = high;
    }
    let [a, b, c, d] = low;
    mesh.quad(a, b, c, d, colour);
}

pub(crate) fn traffic_rail(
    mesh: &mut MeshArrays,
    frame: &Frame,
    c: [f64; 2],
    top: [f64; 2],
    road_edge: f64,
    height: f64,
    post: bool,
    colour: Rgb,
) {
    let [a, b] = c;
    let [ta, tb] = top;
    let offset = road_edge + road_edge.signum() * 0.15;
    mesh.beam(
        frame.P(a, offset, ta + height),
        frame.P(b, offset, tb + height),
        0.25,
        0.25,
        colour,
    );
    if post {
        mesh.beam(
            frame.P(b, offset, tb),
            frame.P(b, offset, tb + height),
            0.2,
            0.2,
            colour,
        );
    }
}

struct Span<'a> {
    frame: Frame<'a>,
    ends: [f64; 2],
    panel: f64,
    half: f64,
}

fn mesh_faces(barriers: &MeshArrays) -> Vec<Triangle> {
    barriers
        .indices
        .chunks_exact(3)
        .map(|indices| {
            [indices[0], indices[1], indices[2]].map(|i| {
                let k = 3 * i as usize;
                [
                    barriers.positions[k],
                    barriers.positions[k + 1],
                    barriers.positions[k + 2],
                ]
            })
        })
        .collect()
}

impl Span<'_> {
    fn surface(
        &self,
        deck: LandmarkDeck,
        y: impl Fn(f64) -> f64,
        median_section: Option<&DeckStandards>,
        rail_height: Option<f64>,
        gap: impl Fn(f64, f64) -> bool,
    ) -> RoadSurface {
        let [start, end] = self.ends;
        let mut road = Vec::new();
        let mut barriers = MeshArrays::new("traffic barriers");
        for pair in stations(start, end, self.panel).windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let (ta, tb) = (y(a), y(b));
            let [p, q, r, s] = asphalt(&self.frame, a, b, ta, tb, self.half);
            road.extend([[p, q, r], [p, r, s]]);
            if let Some(ca) = median_section {
                median(
                    &mut barriers,
                    &self.frame,
                    [a, b],
                    [ta, tb],
                    [(a - start).abs() < 1e-8, (b - end).abs() < 1e-8],
                    ca,
                    [0.; 3],
                );
            }
        }
        if let Some(height) = rail_height {
            for pair in stations(start, end, self.panel / 2.).windows(2) {
                let (a, b) = (pair[0], pair[1]);
                for sign in [-1., 1.] {
                    traffic_rail(
                        &mut barriers,
                        &self.frame,
                        [a, b],
                        [y(a), y(b)],
                        sign * self.half,
                        height,
                        !gap(a, b),
                        [0.; 3],
                    );
                }
            }
        }
        let barrier = mesh_faces(&barriers);
        RoadSurface {
            deck,
            carriageway_faces: road.len(),
            road,
            barrier,
        }
    }
}

/// The asphalt has constant height across each station and linearly follows
/// the drawn panel between stations. Paint and the underside do not carry tires.
pub fn surfaces(
    js: &LandmarksJs,
    sample: &dyn Fn(f64, f64) -> Option<f64>,
    ca: &DeckStandards,
) -> Vec<RoadSurface> {
    let mut surfaces = Vec::new();
    if js.registry.contains_key("golden-gate-bridge") && js.golden_gate.carriageways.len() >= 2 {
        let g = golden_gate::data(js);
        let site = golden_gate::Site::new(&g);
        let ground = crate::set::ground_of(js, "golden-gate-bridge", sample);
        let p = golden_gate::plan(js, &site, &|x, z| ground.at(x, z));
        let offset = |c| site.centre_offset(c);
        let mut frame = Frame::new(site.axis_point(0., 0.), site.axis);
        frame.offset = Some(&offset);
        let span = Span {
            frame,
            ends: [p.c_south, p.c_north],
            panel: num(&g.dimensions, "panelM"),
            half: num(&g.dimensions, "roadwayM") / 2.,
        };
        let zone = golden_gate::tower_zone_half(&g);
        let mut gate = span.surface(
            LandmarkDeck::GoldenGate,
            |c| p.deck_top(c),
            Some(ca),
            Some(num(&g.dimensions, "innerBarrierM")),
            |a, b| [0., p.l].iter().any(|t| b > t - zone && a < t + zone),
        );
        if golden_gate::approach_finish() {
            let panels = golden_gate::abutment(&g, &site, &p, &|x, z| {
                sample(x, z).unwrap_or_else(|| ground.at(x, z))
            });
            let mut median = MeshArrays::new("Marin apron median");
            golden_gate::apron_median(
                &panels,
                num(&g.dimensions, "roadwayM"),
                ca,
                &mut median,
                [0.; 3],
            );
            gate.barrier.extend(mesh_faces(&median));
            for [a, b, c, d] in panels {
                gate.road.extend([[a, b, c], [a, c, d]]);
            }
        }
        gate.carriageway_faces = gate.road.len();
        if golden_gate::safety_net::enabled() {
            for panel in golden_gate::safety_net::panels(&p, &span.frame, num(&g.dimensions,"widthM"), num(&g.dimensions,"curbM")) {
                if panel.upright {
                    let faces = golden_gate::safety_net::housing_faces(&panel, &p, &span.frame, num(&g.dimensions,"widthM"), num(&g.dimensions,"curbM"));
                    for (index, [a,b,c,d]) in faces.into_iter().enumerate() {
                        let target = if index == 0 { &mut gate.road } else { &mut gate.barrier };
                        target.extend([[a,b,c], [a,c,d]]);
                    }
                }
                let [a,b,c,d] = panel.corners;
                if panel.side > 0. {
                    gate.barrier.extend([[a,b,c],[a,c,d]]);
                } else {
                    gate.barrier.extend([[a,c,b],[a,d,c]]);
                }
            }
        }
        surfaces.push(gate);
    }
    if js.registry.contains_key("bay-bridge-west") && !js.bay_bridge_west.carried_ways.is_empty() {
        let b = bay_bridge::data(js);
        let ground = crate::set::ground_of(js, "bay-bridge-west", sample);
        let p = bay_bridge::plan(js, &|x, z| ground.at(x, z));
        let span = Span {
            frame: bay_bridge::frame(&b),
            ends: [p.c_a, p.c_b],
            panel: num(&b.dimensions, "panelM"),
            half: num(&b.dimensions, "roadwayM") / 2.,
        };
        surfaces.push(span.surface(
            LandmarkDeck::BayBridgeUpper,
            |c| p.deck_top(c),
            None,
            Some(bay_bridge::UPPER_TRAFFIC_RAIL_M),
            |_, _| false,
        ));
        surfaces.push(span.surface(
            LandmarkDeck::BayBridgeLower,
            |c| p.lower_top(c),
            None,
            None,
            |_, _| false,
        ));
    }
    surfaces
}
