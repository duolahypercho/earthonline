//! Tests on small synthetic inputs: a street world built from a handful of
//! segments over a made-up elevation grid, the real standards table.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use eo_data::standards::CaStandards;
use eo_data::{DataPaths, ElevationGrid, ElevationMeta};
use eo_street::city::Segment;
use eo_street::{BuildingIndex, Layer, StreetCity, StreetOptions, StreetWorld, Terrain};

use crate::landmark::LandmarkSpec;
use crate::{ChainKind, EndKind, Hold, LandmarkDeck, RoadTags, Status, StructureOptions, StructureWorld, freeway_clearance, solve};

const CELL: f64 = 8.0;
const ORIGIN: f64 = -600.0;
const CELLS: usize = 150;

pub(crate) fn grid(ground: &dyn Fn(f64, f64) -> f64) -> ElevationGrid {
    let mut lidar = vec![0.0f32; CELLS * CELLS];
    for j in 0..CELLS {
        for i in 0..CELLS {
            let (x, z) = (ORIGIN + (i as f64 + 0.5) * CELL, ORIGIN + (j as f64 + 0.5) * CELL);
            lidar[j * CELLS + i] = ground(x, z) as f32;
        }
    }
    let nodata = -32768;
    ElevationGrid {
        meta: ElevationMeta { version: "synthetic".into(), origin_x: ORIGIN, origin_z: ORIGIN, cell_size: CELL, width: CELLS, height: CELLS, nodata },
        lidar,
        topobathy: vec![nodata as i16; CELLS * CELLS],
        height_scale: 1.0,
    }
}

/// One street-plan segment of way `way`, piece `k`.
pub(crate) fn seg(way: &str, k: usize, highway: &str, width: f64, walk: f64, points: &[[f64; 2]], separated: bool) -> Segment {
    Segment {
        key: Some(format!("sf-street-{way}:{k}")),
        id: format!("sf-seg-{way}-{k}"),
        street_name: format!("Way {way}"),
        street_id: Some(format!("sf-street-{way}")),
        highway: highway.into(),
        width,
        lanes: Some(2.0),
        oneway: "both".into(),
        sidewalk_w: Some(walk),
        sidewalk_left: Some(walk),
        sidewalk_right: Some(walk),
        points: points.to_vec(),
        separated,
        lane_layout: None,
        parking: [0.0; 2],
        cycle: [0.0; 2],
        cycle_kerbside: [false; 2],
        cycle_buffer: [0.0; 2],
        shoulder: [0.0; 2],
        median: 0.0,
        street_type: None,
        maxspeed_kmh: None,
        maxspeed: String::new(),
        right_of_way: None,
        pedestrian_eligible: true,
    }
}

/// A straight way from `a` to `b` cut into pieces of about 40 m (the city's
/// segments are one piece per pair of points).
pub(crate) fn way(out: &mut Vec<Segment>, id: &str, highway: &str, width: f64, walk: f64, a: [f64; 2], b: [f64; 2], separated: bool) {
    let len = (b[0] - a[0]).hypot(b[1] - a[1]);
    let n = ((len / 40.0).ceil() as usize).max(1);
    for k in 0..n {
        let p = |t: f64| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        out.push(seg(id, k, highway, width, walk, &[p(k as f64 / n as f64), p((k + 1) as f64 / n as f64)], separated));
    }
}

pub(crate) struct Case {
    pub streets: StreetWorld,
    pub roads: HashMap<String, RoadTags>,
}

pub(crate) fn case(segments: Vec<Segment>, roads: &[(&str, i32, bool, bool)], ground: &dyn Fn(f64, f64) -> f64) -> Case {
    let o = StreetOptions::load().expect("street options");
    let city = StreetCity { segments, intersections: vec![], crossings: vec![], crossing_nodes: vec![], continental: vec![] };
    let lift = o.open_ground_lift;
    let streets = StreetWorld::build(o, city, Terrain { grid: Arc::new(grid(ground)), carpet_lift: lift }, BuildingIndex::new(vec![]));
    let roads = roads.iter().map(|&(id, layer, bridge, tunnel)| (id.to_string(), RoadTags { layer, bridge, tunnel, name: format!("Way {id}") })).collect();
    Case { streets, roads }
}

pub(crate) fn standards() -> CaStandards {
    CaStandards::load(&DataPaths::discover().expect("repository")).expect("standards table")
}

pub(crate) fn solve(c: &Case, landmarks: Vec<LandmarkSpec>, options: StructureOptions) -> StructureWorld {
    solve::build(&c.streets, &c.roads, &standards(), landmarks, options).expect("build")
}

pub(crate) fn flat(_: f64, _: f64) -> f64 {
    0.0
}

/// A motorway on a deck from (0, -200) to (0, 200) over a residential
/// street along z = 0, at grade beyond both ends.
pub(crate) fn deck_over_street() -> Case {
    let mut s = vec![];
    way(&mut s, "100", "motorway", 14.0, 0.0, [0.0, -400.0], [0.0, -200.0], false);
    way(&mut s, "101", "motorway", 14.0, 0.0, [0.0, -200.0], [0.0, 200.0], true);
    way(&mut s, "102", "motorway", 14.0, 0.0, [0.0, 200.0], [0.0, 400.0], false);
    way(&mut s, "200", "residential", 10.0, 3.0, [-300.0, 0.0], [300.0, 0.0], false);
    case(s, &[("100", 0, false, false), ("101", 1, true, false), ("102", 0, false, false), ("200", 0, false, false)], &flat)
}

pub(crate) fn built<'a>(w: &'a StructureWorld, way: &str) -> &'a crate::Chain {
    w.chains().iter().find(|c| c.ways().contains(&way)).unwrap_or_else(|| panic!("no chain carries way {way}"))
}

pub(crate) fn steps_within_grade(c: &crate::Chain) {
    for i in 0..c.stations.len() - 1 {
        let g = (c.heights[i + 1] - c.heights[i]).abs() / (c.stations[i + 1] - c.stations[i]);
        assert!(g <= c.grade_limits[i] + 1e-3, "chain {} step {i}: grade {g} over its limit {}", c.id, c.grade_limits[i]);
    }
}

fn profile_hash(w: &StructureWorld) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for c in w.chains() {
        c.id.hash(&mut h);
        c.level.hash(&mut h);
        format!("{:?}{:?}", c.kind, c.status).hash(&mut h);
        for p in &c.pieces {
            p.key.hash(&mut h);
            p.reversed.hash(&mut h);
        }
        for v in c.stations.iter().chain(&c.heights).chain(&c.bents).chain(&c.depths).chain(&c.grade_limits) {
            v.to_bits().hash(&mut h);
        }
        for e in &c.ends {
            format!("{:?}", e.kind).hash(&mut h);
            e.fixed.map(f64::to_bits).hash(&mut h);
        }
    }
    h.finish()
}

#[test]
fn a_deck_over_a_street_meets_clearance_plus_depth() {
    let c = deck_over_street();
    let w = solve(&c, vec![], StructureOptions::default());
    let d = built(&w, "101");
    assert_eq!(d.status, Status::Built, "{:?}", d.status);
    assert_eq!((d.kind, d.level), (ChainKind::Deck, 1));
    assert_eq!((d.ends[0].kind, d.ends[1].kind), (EndKind::Paved, EndKind::Paved));
    let ca = standards();
    let clearance = ca.metric("structure-vertical-clearance-local").unwrap();
    let ratio = ca.metric_key("superstructure-depth-min-span-ratio", "cipBoxContinuous").unwrap();
    assert!(!freeway_clearance("residential"));
    // the street's datum (flat ground), measured independently.
    let datum = c.streets.ground(0.0, 0.0) + c.streets.o.road_lift;
    let street_half = c.streets.plan.entries.iter().find(|e| e.highway == "residential").unwrap().half;
    let deck_half = d.half_at(0.0);
    let mut checked = 0;
    for (i, &s) in d.stations.iter().enumerate() {
        let z = d.frame(s).z;
        // the deck's cross-section (perpendicular to z) is over the carriageway.
        if z.abs() > street_half {
            continue;
        }
        let (a, b) = d.span_at(s);
        let depth = (b - a) * ratio;
        assert!((d.depth_at(s) - depth).abs() < 1e-9);
        assert!(d.heights[i] - depth - datum >= clearance - 1e-6, "station {s}: {} - {depth} - {datum} < {clearance}", d.heights[i]);
        checked += 1;
    }
    assert!(checked >= 2, "stations over the street: {checked}");
    let x = d.crossings.iter().find(|x| x.over.contains("Way 200")).expect("the street is a crossing");
    assert!(x.achieved >= x.clearance - 1e-6 && (x.clearance - clearance).abs() < 1e-12);
    // the deck is the lowest within its grade: it touches the bound over the street.
    assert!(x.achieved < clearance + 0.05, "achieved {}", x.achieved);
    assert!(deck_half > 0.0 && w.census().clearance_short == 0);
    // bents: no column zone on the drawn paving, spans near the target.
    assert!(!d.bents.is_empty());
    for (k, (&s, zones)) in d.bents.iter().zip(&d.bent_columns).enumerate() {
        assert!(!zones.is_empty(), "bent {k} has no column room");
        let f = d.frame(s);
        for z in zones {
            let u = (z[0] + z[1]) / 2.0;
            let p = [f.x + f.nx * u, f.z + f.nz * u];
            let layer = c.streets.paved_layer_at(p[0], p[1]);
            assert!(!matches!(layer, Some(Layer::Carriageway | Layer::CurbTop | Layer::Sidewalk | Layer::Ramp)), "bent {k} column at {p:?} on {layer:?}");
        }
    }
    let mut edges = vec![0.0];
    edges.extend(&d.bents);
    edges.push(d.length());
    assert!(edges.windows(2).all(|e| e[1] - e[0] <= crate::MAX_SPAN_M * 1.5 + 1e-9), "{edges:?}");
}

#[test]
fn grade_never_exceeds_its_limit_and_ends_meet_the_datum() {
    let c = deck_over_street();
    let w = solve(&c, vec![], StructureOptions::default());
    let ca = standards();
    let rolling = ca.metric_key("freeway-profile-grade-max", "rolling").unwrap();
    let mut ends = 0;
    for ch in w.chains().iter().filter(|c| c.is_built()) {
        steps_within_grade(ch);
        // flat ground: a motorway takes the table's rolling grade.
        assert!(ch.grade_limits.iter().all(|g| (g - rolling).abs() < 1e-12));
        for (e, h) in ch.ends.iter().zip([ch.heights[0], *ch.heights.last().unwrap()]) {
            if e.kind == EndKind::Paved {
                let datum = c.streets.ground(e.at[0], e.at[1]) + c.streets.o.road_lift;
                assert!((h - datum).abs() < 0.01, "end at {:?}: {h} vs datum {datum}", e.at);
                ends += 1;
            }
        }
    }
    assert_eq!(ends, 2);
    let census = w.census();
    assert_eq!(census.grade_over, 0);
    assert!(census.steepest.as_ref().is_some_and(|s| s.value <= rolling + 1e-3));
}

#[test]
fn a_level_two_chain_clears_a_level_one_chain() {
    let mut s = vec![];
    // level 1: east-west, at grade beyond its ends.
    way(&mut s, "300", "motorway", 14.0, 0.0, [-400.0, 0.0], [-200.0, 0.0], false);
    way(&mut s, "301", "motorway", 14.0, 0.0, [-200.0, 0.0], [200.0, 0.0], true);
    way(&mut s, "302", "motorway", 14.0, 0.0, [200.0, 0.0], [400.0, 0.0], false);
    // level 2: north-south over it.
    way(&mut s, "400", "motorway", 14.0, 0.0, [0.0, -450.0], [0.0, -250.0], false);
    way(&mut s, "401", "motorway", 14.0, 0.0, [0.0, -250.0], [0.0, 250.0], true);
    way(&mut s, "402", "motorway", 14.0, 0.0, [0.0, 250.0], [0.0, 450.0], false);
    let roads = [("300", 0, false, false), ("301", 1, true, false), ("302", 0, false, false), ("400", 0, false, false), ("401", 2, true, false), ("402", 0, false, false)];
    let c = case(s, &roads, &flat);
    let w = solve(&c, vec![], StructureOptions::default());
    let low = built(&w, "301");
    let high = built(&w, "401");
    assert!(low.is_built() && high.is_built(), "{:?} {:?}", low.status, high.status);
    assert_eq!((low.level, high.level), (1, 2));
    let fw = standards().metric("structure-vertical-clearance-freeway").unwrap();
    let x = high.crossings.iter().find(|x| x.chain == Some(low.id)).expect("level 2 crosses level 1");
    assert!(x.freeway && (x.clearance - fw).abs() < 1e-12);
    assert!(x.achieved >= fw - 1e-6, "achieved {}", x.achieved);
    // and directly: over the crossing point.
    let (s_high, _) = high.project(0.0, 0.0);
    let (s_low, _) = low.project(0.0, 0.0);
    assert!(high.y_at(s_high) - high.depth_at(s_high) - low.y_at(s_low) >= fw - 1e-6);
    // the queries see both levels there, the upper one higher.
    let l1 = w.deck_at(0.0, 0.0, 1).expect("level 1 deck");
    let l2 = w.deck_at(0.0, 0.0, 2).expect("level 2 deck");
    assert!(l2.y > l1.y + fw);
    assert_eq!(w.decks_at(0.0, 0.0).len(), 2);
    assert!(w.deck_at(0.0, 0.0, 3).is_none() && w.deck_at(100.0, 100.0, 1).is_none());
    assert_eq!(w.way_level("401"), 2);
    assert_eq!(w.way_level("sf-street-301"), 1);
    assert_eq!(w.way_level("300"), 0);
    // the slope a lane feels depends on its heading.
    let p = w.deck_at(0.0, -150.0, 2).unwrap();
    assert!((p.slope_toward(p.dir) - p.slope).abs() < 1e-12 && (p.slope_toward([-p.dir[0], -p.dir[1]]) + p.slope).abs() < 1e-12);
    for ch in w.chains().iter().filter(|c| c.is_built()) {
        steps_within_grade(ch);
    }
}

#[test]
fn a_tunnel_stays_under_its_bound() {
    // a hill, 40 m high in the middle; the tunnel runs east-west through it.
    let hill = |x: f64, _z: f64| 40.0 * (-(x * x) / (2.0 * 110.0 * 110.0)).exp();
    let mut s = vec![];
    way(&mut s, "500", "primary", 12.0, 0.0, [-500.0, 0.0], [-320.0, 0.0], false);
    way(&mut s, "501", "primary", 12.0, 0.0, [-320.0, 0.0], [320.0, 0.0], true);
    way(&mut s, "502", "primary", 12.0, 0.0, [320.0, 0.0], [500.0, 0.0], false);
    let c = case(s, &[("500", 0, false, false), ("501", -1, false, true), ("502", 0, false, false)], &hill);
    let off = solve(&c, vec![], StructureOptions::default());
    assert_eq!(built(&off, "501").status, Status::TunnelOff);
    assert_eq!(off.way_level("501"), 0);
    let w = solve(&c, vec![], StructureOptions { tunnels: true });
    let t = built(&w, "501");
    assert_eq!((t.kind, t.level, t.status.clone()), (ChainKind::Tunnel, -1, Status::Built));
    steps_within_grade(t);
    let mut tau = vec![0.0; t.stations.len()];
    for i in 1..tau.len() {
        tau[i] = tau[i - 1] + t.grade_limits[i - 1] * (t.stations[i] - t.stations[i - 1]);
    }
    let (h0, h1) = (t.ends[0].fixed.unwrap(), t.ends[1].fixed.unwrap());
    assert!((t.heights[0] - h0).abs() < 0.01 && (t.heights[t.heights.len() - 1] - h1).abs() < 0.01);
    let mut under = 0;
    for (i, &s) in t.stations.iter().enumerate() {
        let f = t.frame(s);
        let bound = c.streets.ground(f.x, f.z) - crate::MIN_COVER_M - crate::BORE_CLEAR_M;
        // past the portal zones (where the profile falling from a portal at
        // its grade limit is still above the bound) it stays under the bound.
        let portal = (h0 - tau[i]).max(h1 - (tau[tau.len() - 1] - tau[i]));
        assert!(t.heights[i] <= bound.max(portal) + 1e-6, "station {s}: {} over {bound} / {portal}", t.heights[i]);
        if portal < bound {
            assert!(t.heights[i] <= bound + 1e-6);
            under += 1;
        }
    }
    assert!(under > 10, "stations under the hill with full cover: {under}");
    assert_eq!(w.way_level("501"), -1);
    assert!(w.deck_at(0.0, 0.0, -1).is_some_and(|d| d.y < hill(0.0, 0.0) - crate::BORE_CLEAR_M));
}

#[test]
fn chains_meeting_at_a_shared_node_meet_at_one_height() {
    let mut s = vec![];
    // a mainline in two chains through node (0, 0) over a street near it,
    // and a ramp leaving the node north.
    way(&mut s, "600", "motorway", 14.0, 0.0, [-500.0, 0.0], [-300.0, 0.0], false);
    way(&mut s, "601", "motorway", 14.0, 0.0, [-300.0, 0.0], [0.0, 0.0], true);
    way(&mut s, "602", "motorway", 14.0, 0.0, [0.0, 0.0], [300.0, 0.0], true);
    way(&mut s, "603", "motorway", 14.0, 0.0, [300.0, 0.0], [500.0, 0.0], false);
    way(&mut s, "604", "motorway_link", 8.0, 0.0, [0.0, 0.0], [120.0, 250.0], true);
    way(&mut s, "605", "residential", 8.0, 0.0, [120.0, 250.0], [120.0, 450.0], false);
    way(&mut s, "700", "residential", 10.0, 2.0, [-40.0, -300.0], [-40.0, 300.0], false);
    let roads = [("600", 0, false, false), ("601", 1, true, false), ("602", 1, true, false), ("603", 0, false, false), ("604", 1, true, false), ("605", 0, false, false), ("700", 0, false, false)];
    let c = case(s, &roads, &flat);
    let w = solve(&c, vec![], StructureOptions::default());
    let a = built(&w, "601");
    let b = built(&w, "602");
    let r = built(&w, "604");
    assert!(a.is_built() && b.is_built() && r.is_built(), "{:?} {:?} {:?}", a.status, b.status, r.status);
    let node = a.ends[1].node;
    let at_node = |ch: &crate::Chain| -> f64 {
        let e = ch.ends.iter().position(|e| e.node == node).expect("meets the node");
        assert_eq!(ch.ends[e].kind, EndKind::Shared);
        if e == 0 { ch.heights[0] } else { *ch.heights.last().unwrap() }
    };
    let (ya, yb, yr) = (at_node(a), at_node(b), at_node(r));
    assert!((ya - yb).abs() < 0.01 && (ya - yr).abs() < 0.01, "{ya} {yb} {yr}");
    // the street near the node lifts the node above the datum.
    assert!(ya > c.streets.o.road_lift + 1.0, "node height {ya}");
    // the ramp is a ramp (it ends on a street), the mainline a freeway.
    let ca = standards();
    assert!(r.grade_limits.iter().all(|g| (g - ca.metric("ramp-profile-grade-max").unwrap()).abs() < 1e-12));
    for ch in [a, b, r] {
        steps_within_grade(ch);
    }
}

#[test]
fn an_infeasible_chain_is_held_with_its_reason() {
    let mut s = vec![];
    // 60 m of deck between two at-grade ends cannot rise over the street between.
    way(&mut s, "800", "motorway", 14.0, 0.0, [-300.0, 0.0], [-30.0, 0.0], false);
    way(&mut s, "801", "motorway", 14.0, 0.0, [-30.0, 0.0], [30.0, 0.0], true);
    way(&mut s, "802", "motorway", 14.0, 0.0, [30.0, 0.0], [300.0, 0.0], false);
    // (its vertices are off the deck's: a street sharing a vertex with the
    // deck would be a junction at grade.)
    way(&mut s, "900", "residential", 10.0, 0.0, [0.0, -210.0], [0.0, 190.0], false);
    let roads = [("800", 0, false, false), ("801", 1, true, false), ("802", 0, false, false), ("900", 0, false, false)];
    let c = case(s, &roads, &flat);
    let w = solve(&c, vec![], StructureOptions::default());
    let d = built(&w, "801");
    match d.hold() {
        Some(Hold::EndTooLow { over }) => assert!(over.contains("Way 900"), "{over}"),
        other => panic!("held for clearance over the street, not {other:?}"),
    }
    assert!(!d.is_built() && w.held().any(|h| h.id == d.id));
    assert_eq!(w.census().held_by_reason.get("end too low for clearance"), Some(&1));
    // not built: no deck there, the way stays at grade for the simulation.
    assert!(w.deck_at(0.0, 0.0, 1).is_none());
    assert_eq!(w.way_level("801"), 0);
    assert!(w.is_empty());
}

#[test]
fn a_landmark_deck_end_fixes_the_chain_meeting_it() {
    let mut s = vec![];
    way(&mut s, "1000", "motorway", 14.0, 0.0, [-400.0, 0.0], [-250.0, 0.0], false);
    // one way, half of it on the landmark deck (chainage = x, deck from x = 0).
    way(&mut s, "1001", "motorway", 14.0, 0.0, [-250.0, 0.0], [300.0, 0.0], true);
    let c = case(s, &[("1000", 0, false, false), ("1001", 1, true, false)], &flat);
    let spec = LandmarkSpec {
        deck: LandmarkDeck::BayBridgeUpper,
        ways: vec!["1001".into()],
        corridor: vec![],
        corridor_half: 0.0,
        c0: 0.0,
        c1: 1000.0,
        chainage: Box::new(|x, _| x),
        top: Box::new(|c| 6.0 + 0.01 * c),
    };
    let w = solve(&c, vec![spec], StructureOptions::default());
    let lm = w.chains().iter().find(|c| c.kind == ChainKind::Landmark).expect("a landmark chain");
    assert_eq!(lm.landmark, Some(LandmarkDeck::BayBridgeUpper));
    assert!(lm.is_built() && lm.stations.iter().zip(&lm.heights).all(|(&s, &y)| (y - (6.0 + 0.01 * lm.frame(s).x)).abs() < 1e-9));
    let ap = w.chains().iter().find(|c| c.kind == ChainKind::Deck && c.ways().contains(&"1001")).expect("the approach");
    assert!(ap.is_built(), "{:?}", ap.status);
    let e = ap.ends.iter().position(|e| e.kind == EndKind::Landmark).expect("a landmark end");
    let y = if e == 0 { ap.heights[0] } else { *ap.heights.last().unwrap() };
    assert!((y - 6.0).abs() < 0.01, "{y}");
    steps_within_grade(ap);
    assert_eq!(w.way_level("1001"), 1);
    assert!(w.census().landmark == 1 && w.census().landmark_segments > 0);
}

#[test]
fn building_twice_or_in_another_order_gives_the_same_profiles() {
    let make = |rev: bool| {
        let mut s = vec![];
        way(&mut s, "600", "motorway", 14.0, 0.0, [-500.0, 0.0], [-300.0, 0.0], false);
        way(&mut s, "601", "motorway", 14.0, 0.0, [-300.0, 0.0], [0.0, 0.0], true);
        way(&mut s, "602", "motorway", 14.0, 0.0, [0.0, 0.0], [300.0, 0.0], true);
        way(&mut s, "603", "motorway", 14.0, 0.0, [300.0, 0.0], [500.0, 0.0], false);
        way(&mut s, "604", "motorway_link", 8.0, 0.0, [0.0, 0.0], [120.0, 250.0], true);
        way(&mut s, "605", "residential", 8.0, 0.0, [120.0, 250.0], [120.0, 450.0], false);
        way(&mut s, "700", "residential", 10.0, 2.0, [-40.0, -300.0], [-40.0, 300.0], false);
        way(&mut s, "701", "motorway", 14.0, 0.0, [-200.0, -250.0], [200.0, 250.0], true);
        if rev {
            // another order: reversed, then every other one moved to the end.
            s.reverse();
            let (even, odd): (Vec<(usize, Segment)>, Vec<(usize, Segment)>) = s.into_iter().enumerate().partition(|(i, _)| i % 2 == 0);
            s = even.into_iter().chain(odd).map(|(_, x)| x).collect();
        }
        let roads = [("600", 0, false, false), ("601", 1, true, false), ("602", 1, true, false), ("603", 0, false, false), ("604", 1, true, false), ("605", 0, false, false), ("700", 0, false, false), ("701", 2, true, false)];
        case(s, &roads, &|x, z| 3.0 + 0.01 * x - 0.005 * z)
    };
    let a = make(false);
    let first = solve(&a, vec![], StructureOptions::default());
    let again = solve(&a, vec![], StructureOptions::default());
    let other = solve(&make(true), vec![], StructureOptions::default());
    assert!(first.chains().iter().filter(|c| c.is_built()).count() >= 4);
    assert_eq!(profile_hash(&first), profile_hash(&again));
    assert_eq!(profile_hash(&first), profile_hash(&other));
    assert_eq!(first.census(), other.census());
}

#[test]
fn a_deck_over_a_street_running_under_it_has_no_bent_room() {
    let mut s = vec![];
    way(&mut s, "1100", "motorway", 14.0, 0.0, [0.0, -400.0], [0.0, -200.0], false);
    way(&mut s, "1101", "motorway", 14.0, 0.0, [0.0, -200.0], [0.0, 200.0], true);
    way(&mut s, "1102", "motorway", 14.0, 0.0, [0.0, 200.0], [0.0, 400.0], false);
    // a street with its footways right under the deck for 300 m: no parking,
    // footways too narrow for a furniture zone, and paving wider than a
    // straddle bent's outrigger reaches (A2: those rules come after A1's).
    way(&mut s, "1200", "residential", 30.0, 3.0, [1.0, -150.0], [1.0, 150.0], false);
    let c = case(s, &[("1100", 0, false, false), ("1101", 1, true, false), ("1102", 0, false, false), ("1200", 0, false, false)], &flat);
    let w = solve(&c, vec![], StructureOptions::default());
    let d = built(&w, "1101");
    match d.hold() {
        Some(Hold::NoBentRoom { span }) => assert!(*span as f64 > crate::MAX_SPAN_M * 2.0 && *span as f64 > crate::LONGEST_SPAN_M, "{span}"),
        other => panic!("held for want of bent room, not {other:?}"),
    }
    // no bent stands where the street is.
    for (&b, zones) in d.bents.iter().zip(&d.bent_columns) {
        assert!(!zones.is_empty());
        assert!(d.frame(b).z.abs() > 150.0, "bent at {b}");
    }
    assert_eq!(w.census().held_by_reason.get("no bent room"), Some(&1));
}

/// A 14 m motorway deck (way 1301, x = 0, z -400..400, at grade beyond)
/// over a street (way 1400) running under it along x = 1 for 300 m, its
/// section set by `street`.
pub(crate) fn deck_along_street(street: &dyn Fn(&mut Segment)) -> Case {
    let mut s = vec![];
    way(&mut s, "1300", "motorway", 14.0, 0.0, [0.0, -600.0], [0.0, -400.0], false);
    way(&mut s, "1301", "motorway", 14.0, 0.0, [0.0, -400.0], [0.0, 400.0], true);
    way(&mut s, "1302", "motorway", 14.0, 0.0, [0.0, 400.0], [0.0, 600.0], false);
    let mut st = vec![];
    way(&mut st, "1400", "residential", 10.0, 3.0, [1.0, -150.0], [1.0, 150.0], false);
    for g in st.iter_mut() {
        street(g);
    }
    s.extend(st);
    case(s, &[("1300", 0, false, false), ("1301", 1, true, false), ("1302", 0, false, false), ("1400", 0, false, false)], &flat)
}

/// Every column of the chain's bents over the street (|z| < 140): what
/// rule placed it, its site, and its footprint's offsets from the street's
/// centreline (x = 1), with the paving layer under each footprint sample.
fn street_columns(c: &Case, d: &crate::Chain) -> Vec<(crate::BentRule, crate::ColumnSite, f64, f64, Vec<Option<Layer>>)> {
    let mut out = vec![];
    for b in &d.supports {
        let f = d.frame(b.s);
        if f.z.abs() > 140.0 {
            continue;
        }
        for col in &b.columns {
            let dist = (col.x - 1.0).abs();
            let mut layers = vec![];
            for k in 0..16 {
                let a = k as f64 * std::f64::consts::TAU / 16.0;
                let (x, z) = (col.x + col.radius * 0.98 * a.cos(), col.z + col.radius * 0.98 * a.sin());
                layers.push(c.streets.paved_layer_at(x, z));
            }
            layers.push(c.streets.paved_layer_at(col.x, col.z));
            out.push((b.rule, col.site, dist - col.radius, dist + col.radius, layers));
        }
    }
    out
}

#[test]
fn a_deck_over_a_street_with_parking_stands_in_its_parking_lanes() {
    // a 14 m carriageway with 3 m parking lanes either side, 3 m footways.
    let c = deck_along_street(&|g| {
        g.width = 14.0;
        g.parking = [3.0, 3.0];
    });
    let w = solve(&c, vec![], StructureOptions::default());
    let d = built(&w, "1301");
    assert!(d.is_built(), "{:?}", d.status);
    let cols = street_columns(&c, d);
    assert!(cols.len() >= 6, "columns over the street: {}", cols.len());
    for (rule, site, near, far, layers) in cols {
        assert_eq!((rule, site), (crate::BentRule::Kerbside, crate::ColumnSite::ParkingLane));
        // wholly in the parking lane: 4..7 m from the centreline, the travel lanes inside 4 m.
        assert!(near >= 4.0 - 1e-6 && far <= 7.0 + 1e-6, "column footprint {near}..{far}");
        assert!(layers.iter().all(|l| *l == Some(Layer::Carriageway)), "{layers:?}");
    }
    assert_eq!(w.census().bents_by_rule.get("parking lane or furniture zone").copied().unwrap_or(0) > 0, true);
    assert!(w.census().street_columns.keys().any(|k| k.contains("Way 1400")));
}

#[test]
fn a_deck_over_a_street_with_wide_footways_stands_in_its_furniture_zones() {
    // no parking; 7 m footways: a furniture zone past the kerb top, short of
    // the throughway and the frontage zone.
    let c = deck_along_street(&|g| {
        g.sidewalk_left = Some(7.0);
        g.sidewalk_right = Some(7.0);
        g.sidewalk_w = Some(7.0);
    });
    let w = solve(&c, vec![], StructureOptions::default());
    let d = built(&w, "1301");
    assert!(d.is_built(), "{:?}", d.status);
    let ca = standards();
    let frontage = ca.metric_key("sidewalk-frontage-zone-sf", "allStreets").unwrap();
    let through = ca.metric_key("sidewalk-throughway-zone-sf", "default").unwrap();
    let cols = street_columns(&c, d);
    assert!(!cols.is_empty());
    for (rule, site, near, far, layers) in cols {
        assert_eq!((rule, site), (crate::BentRule::Kerbside, crate::ColumnSite::FurnitureZone));
        assert!(near >= 5.0 + c.streets.o.curb_top_width - 1e-6 && far <= 5.0 + 7.0 - frontage - through + 1e-6, "column footprint {near}..{far}");
        assert!(layers.iter().all(|l| matches!(l, Some(Layer::Sidewalk))), "{layers:?}");
    }
}

#[test]
fn a_deck_over_a_street_with_no_kerbside_room_straddles_it() {
    // A1's case: no parking, 3 m footways (no furniture zone), the paving a
    // little wider than the deck: columns beyond the footways on both sides.
    let c = deck_along_street(&|_| {});
    let w = solve(&c, vec![], StructureOptions::default());
    let d = built(&w, "1301");
    assert!(d.is_built(), "{:?}", d.status);
    let mut straddles = 0;
    for b in &d.supports {
        if b.rule != crate::BentRule::Straddle {
            continue;
        }
        straddles += 1;
        assert_eq!(b.cap, crate::CapKind::Outrigger);
        let us: Vec<f64> = b.columns.iter().map(|c| c.u).collect();
        assert!(us.len() == 2 && us[0] < 0.0 && us[1] > 0.0, "{us:?}");
        for col in &b.columns {
            assert_eq!(col.site, crate::ColumnSite::OffPaving);
            // off the paving (8 m either side of x = 1) by its footprint.
            assert!((col.x - 1.0).abs() - col.radius >= 8.0 - 1e-6, "column at x {}", col.x);
            assert!(col.u.abs() <= d.max_half() + crate::BENT_FOOTPRINT_MARGIN_M + crate::OUTRIGGER_REACH_M + 1e-6);
        }
    }
    assert!(straddles >= 4, "straddle bents: {straddles}");
    let (_, _, _, _, layers): (_, _, _, _, Vec<Option<Layer>>) = street_columns(&c, d).into_iter().next().unwrap();
    assert!(layers.iter().all(|l| l.is_none() || *l == Some(Layer::Verge)), "{layers:?}");
}

/// A deck (way 1501, "Route 99", x = 0, z -200..200) whose route continues at
/// grade past its north end (1502 to z 260) and comes back under the deck as
/// a paved piece (1503, named `name`) running along it from z 175 to 50.
pub(crate) fn deck_over_own_route(name: &str) -> Case {
    let mut s = vec![];
    way(&mut s, "1500", "motorway", 14.0, 0.0, [0.0, -400.0], [0.0, -200.0], false);
    way(&mut s, "1501", "motorway", 14.0, 0.0, [0.0, -200.0], [0.0, 200.0], true);
    way(&mut s, "1502", "motorway", 14.0, 0.0, [0.0, 200.0], [0.0, 260.0], false);
    s.push(seg("1503", 0, "motorway_link", 8.0, 0.0, &[[0.0, 260.0], [16.0, 230.0], [16.0, 190.0], [4.0, 150.0], [4.0, 50.0]], false));
    for g in s.iter_mut() {
        g.street_name = if g.street_id.as_deref() == Some("sf-street-1503") { name.to_string() } else { "Route 99".to_string() };
    }
    case(s, &[("1500", 0, false, false), ("1501", 1, true, false), ("1502", 0, false, false), ("1503", 0, false, false)], &flat)
}

#[test]
fn a_paved_piece_of_the_same_route_along_under_a_deck_is_not_crossed() {
    let same = deck_over_own_route("Route 99");
    let w = solve(&same, vec![], StructureOptions::default());
    let d = built(&w, "1501");
    assert!(d.is_built(), "{:?}", d.status);
    assert!(d.crossings.iter().all(|x| !x.over.contains("Route 99")), "{:?}", d.crossings);
    assert_eq!(w.census().same_route_ignored, 1);
    // nothing to clear: the deck stays on its datum.
    let datum = same.streets.o.road_lift;
    assert!(d.heights.iter().all(|&y| (y - datum).abs() < 0.01), "max {}", d.heights.iter().copied().fold(f64::MIN, f64::max));
    // another route there is crossed, and the deck rises over it.
    let other = deck_over_own_route("Other Street");
    let w2 = solve(&other, vec![], StructureOptions::default());
    let d2 = built(&w2, "1501");
    assert_eq!(w2.census().same_route_ignored, 0);
    assert!(d2.crossings.iter().any(|x| x.over.contains("Other Street")));
    assert!(!d2.is_built() || d2.heights.iter().copied().fold(f64::MIN, f64::max) > datum + 4.0);
}

/// A 600 m motorway deck (way 1601) over a channel 6 m deep, `channel_half`
/// either side of x = 0, between two shores at 3 m.
fn deck_over_channel(channel_half: f64) -> Case {
    let ground = move |x: f64, _z: f64| if x.abs() < channel_half { -6.0 } else { 3.0 };
    let mut s = vec![];
    way(&mut s, "1600", "motorway", 14.0, 0.0, [-500.0, 0.0], [-300.0, 0.0], false);
    way(&mut s, "1601", "motorway", 14.0, 0.0, [-300.0, 0.0], [300.0, 0.0], true);
    way(&mut s, "1602", "motorway", 14.0, 0.0, [300.0, 0.0], [500.0, 0.0], false);
    case(s, &[("1600", 0, false, false), ("1601", 1, true, false), ("1602", 0, false, false)], &ground)
}

#[test]
fn a_freeway_deck_over_open_water_keeps_its_soffit_above_the_water() {
    // (stage A4: the channel is a creek's, 180 m, under `OPEN_WATER_HOLD_M`;
    // the 500 m one this test had is now held, the next test.)
    let half = 90.0;
    assert!(2.0 * half < crate::solve::OPEN_WATER_HOLD_M);
    let c = deck_over_channel(half);
    let w = solve(&c, vec![], StructureOptions::default());
    let d = built(&w, "1601");
    assert!(d.is_built(), "{:?}", d.status);
    for (&s, &y) in d.stations.iter().zip(&d.heights) {
        if d.frame(s).x.abs() < half - 10.0 {
            assert!(y - d.depth_at(s) >= eo_data::SEA_LEVEL_Y + crate::solve::WATER_FREEBOARD_M - 1e-6, "station {s}: soffit {}", y - d.depth_at(s));
        }
    }
    // its bents stand on the channel bed.
    let cols: Vec<&crate::Column> = d.supports.iter().flat_map(|b| &b.columns).filter(|c| c.x.abs() < half - 10.0).collect();
    assert!(!cols.is_empty() && cols.iter().all(|c| (c.ground + 6.0).abs() < 1e-6));
    steps_within_grade(d);
}

#[test]
fn a_deck_over_more_open_water_than_a_creek_is_held_for_its_navigation_clearance() {
    // 500 m of open water: no sourced navigation clearance, so no low causeway.
    let half = 250.0;
    assert!(2.0 * half > crate::solve::OPEN_WATER_HOLD_M);
    let c = deck_over_channel(half);
    let w = solve(&c, vec![], StructureOptions::default());
    let d = built(&w, "1601");
    assert_eq!(d.hold(), Some(&Hold::NoSourcedClearance), "{:?}", d.status);
    assert!(w.deck_at(0.0, 0.0, 1).is_none());
    assert_eq!(w.way_level("1601"), 0);
    assert_eq!(w.census().held_by_reason.get("no sourced clearance"), Some(&1));
    assert!(w.surface_tile(eo_street::tile_key(0.0, 0.0)).is_none() && w.render_tile(eo_street::tile_key(0.0, 0.0), 0).is_none());
    // a landmark deck over the same water keeps its own profile.
    let spec = LandmarkSpec {
        deck: LandmarkDeck::BayBridgeUpper,
        ways: vec!["1601".into()],
        corridor: vec![],
        corridor_half: 0.0,
        c0: -300.0,
        c1: 300.0,
        chainage: Box::new(|x, _| x),
        top: Box::new(|_| 60.0),
    };
    let w = solve(&c, vec![spec], StructureOptions::default());
    let lm = w.chains().iter().find(|c| c.kind == ChainKind::Landmark).expect("a landmark chain");
    assert!(lm.is_built(), "{:?}", lm.status);
    assert!(w.deck_at(0.0, 0.0, 1).is_some_and(|p| (p.y - 60.0).abs() < 1e-9));
}

/// Two parallel motorway decks of one route ("Twin Freeway", ways 1700 at
/// x = -9.6 and 1710 at x = +9.6, z -400..400, at grade beyond), 4 m apart
/// fascia to fascia, over a street (way 1800) running along under the gap
/// for 300 m with a 3 m median between its two directions, and streets
/// beside them: neither deck has room for a column of its own over the
/// street (the median is out of its reach), both have it on shared bents.
pub(crate) fn twin_decks_over_a_median() -> Case {
    let mut s = vec![];
    for (id, x) in [(1700, -9.6), (1710, 9.6)] {
        way(&mut s, &(id + 1).to_string(), "motorway", 14.0, 0.0, [x, -600.0], [x, -400.0], false);
        way(&mut s, &id.to_string(), "motorway", 14.0, 0.0, [x, -400.0], [x, 400.0], true);
        way(&mut s, &(id + 2).to_string(), "motorway", 14.0, 0.0, [x, 400.0], [x, 600.0], false);
    }
    for g in s.iter_mut() {
        g.street_name = "Twin Freeway".into();
    }
    let mut st = vec![];
    way(&mut st, "1800", "residential", 28.0, 3.0, [0.0, -150.0], [0.0, 150.0], false);
    // (streets beside it, so no straddle bent finds room past either deck.)
    way(&mut st, "1801", "residential", 18.0, 3.0, [-27.0, -150.0], [-27.0, 150.0], false);
    way(&mut st, "1802", "residential", 18.0, 3.0, [27.0, -150.0], [27.0, 150.0], false);
    for g in st.iter_mut().filter(|g| g.street_id.as_deref() == Some("sf-street-1800")) {
        g.median = 3.0;
        g.lane_layout = Some(
            [(-1, 8.0), (-1, 4.0), (1, -4.0), (1, -8.0)].iter().map(|&(dir, lateral)| eo_street::city::Lane { dir, role: "through".into(), width: 4.0, lateral, motor: true }).collect(),
        );
    }
    s.extend(st);
    let mut roads = vec![("1800", 0, false, false), ("1801", 0, false, false), ("1802", 0, false, false)];
    let ids = ["1700", "1701", "1702", "1710", "1711", "1712"];
    for id in ids {
        roads.push((id, if id.ends_with('0') { 1 } else { 0 }, id.ends_with('0'), false));
    }
    case(s, &roads, &flat)
}

#[test]
fn twin_decks_over_a_street_stand_on_shared_bents() {
    let c = twin_decks_over_a_median();
    let w = solve(&c, vec![], StructureOptions::default());
    let (a, b) = (built(&w, "1700"), built(&w, "1710"));
    assert!(a.is_built() && b.is_built(), "{:?} {:?}", a.status, b.status);
    assert!(w.census().shared_bents > 0 && w.census().relaxed_bent_chains == 2, "{:?}", (w.census().shared_bents, w.census().relaxed_bent_chains));
    let mut lines = 0;
    for (d, other) in [(a, b), (b, a)] {
        for bent in &d.supports {
            let f = d.frame(bent.s);
            if f.z.abs() > 140.0 {
                continue;
            }
            // over the street every bent is shared: one deck owns the columns, the other rests on its cap.
            let t = bent.twin.expect("a shared bent over the street");
            assert_eq!(t.chain, other.id);
            let o = &other.supports[t.bent as usize];
            let back = o.twin.expect("the other half of the line");
            assert!(back.chain == d.id && back.bent as usize == d.supports.iter().position(|x| x == bent).unwrap() && back.carried != t.carried);
            assert!((other.frame(o.s).z - f.z).abs() < 0.5, "one line: z {} vs {}", f.z, other.frame(o.s).z);
            if t.carried {
                assert!(bent.columns.is_empty());
                continue;
            }
            lines += 1;
            assert!((t.u.abs() - 19.2).abs() < 1e-6, "the twin's centreline along the line: {}", t.u);
            assert!(!bent.columns.is_empty());
            for col in &bent.columns {
                // in the street's median, between the two decks.
                assert_eq!(col.site, crate::ColumnSite::Median);
                assert!(col.x.abs() + col.radius <= 1.5 + 1e-6, "column at x {} r {}", col.x, col.radius);
                assert!(matches!(c.streets.paved_layer_at(col.x, col.z), Some(Layer::Carriageway | Layer::Marking)));
            }
        }
    }
    assert!(lines >= 6, "shared lines over the street: {lines}");
    for d in [a, b] {
        steps_within_grade(d);
    }
}

#[test]
fn a_column_may_stand_in_a_divided_freeways_barrier_median() {
    // two one-way motorway carriageways of opposite directions side by side
    // (9.15 m half widths, centres 20.1 m apart: a 1.8 m unpaved median), a
    // third beside the second carrying its direction, and a street.
    let mut s = vec![];
    way(&mut s, "1900", "motorway", 18.3, 0.0, [-300.0, -10.05], [300.0, -10.05], false);
    way(&mut s, "1901", "motorway", 18.3, 0.0, [300.0, 10.05], [-300.0, 10.05], false);
    way(&mut s, "1902", "motorway", 18.3, 0.0, [300.0, 30.15], [-300.0, 30.15], false);
    for g in s.iter_mut() {
        g.oneway = "increasing".into();
    }
    way(&mut s, "1903", "residential", 10.0, 3.0, [-300.0, -60.0], [300.0, -60.0], false);
    let roads = [("1900", 0, false, false), ("1901", 0, false, false), ("1902", 0, false, false), ("1903", 0, false, false)];
    let c = case(s, &roads, &flat);
    let o = &c.streets.o;
    let paved = crate::index::Paved::new(&c.streets.plan, 30.0, o.min_sidewalk_width, o.max_step);
    let ca = standards();
    let kerb = crate::bents::KerbRule { curb_top: o.curb_top_width, frontage: ca.metric_key("sidewalk-frontage-zone-sf", "allStreets").ok(), through: ca.metric_key("sidewalk-throughway-zone-sf", "default").ok() };
    let sites = crate::bents::Sites::new(&c.streets, &paved, kerb);
    let least = crate::solve::column_diameter(crate::RoadClass::Other, 0.0) / 2.0;
    let wide = crate::BENT_FOOTPRINT_MARGIN_M;
    // the median between the opposite directions: too narrow for A1b's footprint, wide enough for a least column.
    assert_eq!(sites.site([0.0, 0.0], wide), None);
    let (site, r_max) = sites.site_relaxed([0.0, 0.0], wide, least).expect("a barrier median column");
    assert!(matches!(site, crate::bents::Site::Median { .. }));
    assert!((r_max - 0.9).abs() < 0.05 && r_max >= least, "{r_max}");
    // never over a carriageway, nor between two carriageways of one direction.
    assert_eq!(sites.site_relaxed([0.0, 1.2], wide, least), None);
    assert_eq!(sites.site_relaxed([0.0, -5.0], wide, least), None);
    assert_eq!(sites.site_relaxed([0.0, 20.1], wide, least), None);
    // a street's verge is no barrier median (and the A1b rules still answer there).
    assert_eq!(sites.site_relaxed([0.0, -60.0], wide, least), None);
}
