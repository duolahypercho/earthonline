//! The build: chains from the street plan, their ends, grade limits, bents
//! and crossings, then one profile per chain, lowest level first, iterated
//! until every shared end has one height; chains that cannot meet a fixed
//! end are held and the rest solved again without them.

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::{Context, Result};
use eo_data::standards::CaStandards;
use eo_street::StreetWorld;
use eo_street::plan::{P2, arc_table};

use crate::bents::{self, KerbRule, Room, Sites, TwinLine};
use crate::chains::{self, RawPiece};
use crate::index::{Grid, Paved, project, seg_distance};
use crate::landmark::LandmarkSpec;
use crate::profile::{self, Ends};
use crate::route::{self, Elem};
use crate::{
    BENT_FOOTPRINT_MARGIN_M, BORE_CLEAR_M, Bent, BentRule, BentTwin, COLUMN_DC_OVER_DS, COLUMN_MIN_MAINLINE_M, COLUMN_MIN_OTHER_M, CapKind, Chain, ChainEnd, ChainKind, Column, Crossing, DECK_QUERY_MARGIN_M, EndKind, Hold, LONGEST_SPAN_M, MAX_SPAN_M, MIN_COVER_M, Piece, RoadClass, RoadTags, STATION_MAX_M, Status, StructureOptions,
    StructureWorld, VERTICAL_CURVE_M, freeway_clearance,
};

/// A fixed end the profile misses by more than this is held (metres).
const END_TOL_M: f64 = 0.005;
/// A height that moved less than this between solve rounds has settled.
const SETTLED_M: f64 = 1e-7;
/// Solve rounds before a still-moving system is called infeasible.
const ROUNDS: usize = 64;
/// Outer passes (each holds the chains the last one could not meet).
const PASSES: usize = 32;

/// Over open water a deck's soffit stays at least this far above mean sea
/// level (`eo_data::SEA_LEVEL_Y`), so the superstructure is never drawn in
/// the water: mean higher high water above mean sea level at the San
/// Francisco tide station (NOAA 9414290 datums, MHHW 1.78 m and MSL 0.97 m
/// above MLLW). INFERRED, structure-only: the table has no navigational
/// clearance, so nothing more is asked.
pub(crate) const WATER_FREEBOARD_M: f64 = 0.81;

/// A deck over open water (its ground or seabed under mean sea level) for
/// longer than this is held for want of a navigation clearance
/// (`Hold::NoSourcedClearance`): the standards table has none, and with
/// nothing crossed the profile would lie on the water floor (a low causeway
/// where the real bridge stands high over a shipping channel, such as the
/// Bay Bridge east span past Yerba Buena Island). INFERRED, structure-only:
/// longer than the creek channels the city's own bridges cross (Mission and
/// Islais Creeks: the longest open water under a deck in the data is 136 m,
/// I-280 over China Basin), far shorter than a bay crossing (over 3 km).
/// Landmark decks keep their own profile and are not held.
pub(crate) const OPEN_WATER_HOLD_M: f64 = 200.0;

/// The crossed street is asked its clearance from a little over its
/// analytic surface (datum and crown): the drawn street joins its stations
/// with straight lines over a bilinear ground and can stand this much
/// higher between them. INFERRED, structure-only (a solve tolerance).
const SURFACE_MARGIN_M: f64 = 0.03;

/// A paved end takes the drawn street's height when it is within this of
/// the street datum; further off (a junction's pad fanned from the mean of
/// its approaches on a hillside) the datum is kept, and stage A6, which
/// joins structure legs into junctions, meets it. INFERRED (a solve tolerance).
const END_DRAWN_M: f64 = 0.5;

/// A paved end's street surface is read this far past the end (on the
/// stub, not the deck), and taken where it is within `END_SURFACE_M` of the
/// street crown over the end's height. INFERRED (sampling tolerances).
const END_SAMPLE_M: f64 = 0.05;
const END_SURFACE_M: f64 = 0.5;

/// A column's footprint keeps this far from another deck's edge (its
/// barrier's outer face). INFERRED, structure-only.
const COLUMN_DECK_GAP_M: f64 = 0.3;

/// Two columns of a bent are kept at least this far apart, face to face.
/// INFERRED, structure-only (a lane's width between columns).
const COLUMN_GAP_M: f64 = 3.0;

/// The superstructure sits on its approach fill (no bent drawn, its side
/// walls run down to the ground, an abutment wall closes the first span)
/// where its soffit is within this of the ground: lower than about head
/// height, an approach is retained fill, not an open span. INFERRED,
/// structure-only (never over a crossing: the clearance asked is higher).
pub(crate) const FILL_CLEAR_M: f64 = 2.0;

/// A T cap is this deep below the soffit for each metre of column diameter
/// (and never shallower than the superstructure). INFERRED, structure-only:
/// the "deep rectangular" T caps of the Selby St ramp (sheet 5.10).
const TBEAM_DEPTH_PER_DC: f64 = 0.9;

/// An expansion joint every this many spans (frame ends) and a downspout
/// down every this-many-th bent. INFERRED, structure-only: Caltrans MTD 7-10
/// did not extract (sheet 1.5); about 160 m frames at the city's mean span.
pub(crate) const JOINT_SPANS: usize = 5;
pub(crate) const DOWNSPOUT_BENTS: usize = 3;

/// The table's values the solve reads.
pub(crate) struct Limits {
    clear_freeway: Option<f64>,
    clear_local: Option<f64>,
    rolling: f64,
    mountainous: f64,
    connector: f64,
    ramp: f64,
    depth_ratio: f64,
    /// `concrete-barrier-type-60m-base-width`: the barrier on each deck edge.
    pub barrier_base: f64,
    frontage: Option<f64>,
    through: Option<f64>,
}

impl Limits {
    pub(crate) fn read(ca: &CaStandards) -> Result<Self> {
        let grade = "freeway-profile-grade-max";
        Ok(Self {
            clear_freeway: ca.metric("structure-vertical-clearance-freeway").ok(),
            clear_local: ca.metric("structure-vertical-clearance-local").ok(),
            rolling: ca.metric_key(grade, "rolling").context("eo-structures: freeway grade (rolling)")?,
            mountainous: ca.metric_key(grade, "mountainous").context("eo-structures: freeway grade (mountainous)")?,
            connector: ca.metric("connector-profile-grade-max").context("eo-structures: connector grade")?,
            ramp: ca.metric("ramp-profile-grade-max").context("eo-structures: ramp grade")?,
            depth_ratio: ca.metric_key("superstructure-depth-min-span-ratio", "cipBoxContinuous").context("eo-structures: superstructure depth ratio")?,
            barrier_base: ca.metric("concrete-barrier-type-60m-base-width").context("eo-structures: barrier base width")?,
            frontage: ca.metric_key("sidewalk-frontage-zone-sf", "allStreets").ok(),
            through: ca.metric_key("sidewalk-throughway-zone-sf", "default").ok(),
        })
    }

    fn clearance(&self, freeway: bool) -> Option<f64> {
        if freeway { self.clear_freeway } else { self.clear_local }
    }
}

/// Where a station's crossed surface comes from.
#[derive(Clone, Copy, Debug)]
enum Under {
    /// A paved street: its drawn surface there (the highest under the deck).
    Street(f64),
    /// A lower chain: its deck between these stations of it (the highest).
    Chain(f64, f64),
}

/// The most a lower chain's drawn deck stands over its profile between
/// its stations `a` and `b`: the street surface its top blends to toward a
/// paved end (the meshes' `CROWN_BLEND_M`).
fn crown_over(d: &Chain, a: f64, b: f64) -> f64 {
    let blend = crate::sweep::CROWN_BLEND_M;
    let (a, b) = (a.min(b), a.max(b));
    let len = d.length();
    let top = |e: usize| d.ends[e].surface.iter().map(|p| p[1]).fold(0.0, f64::max);
    let mut m: f64 = 0.0;
    if d.ends[0].kind == EndKind::Paved && a < blend {
        m = m.max(top(0));
    }
    if d.ends[1].kind == EndKind::Paved && b > len - blend {
        m = m.max(top(1));
    }
    m
}

/// The highest of a profile between stations `a` and `b` (linear between stations).
fn interp_max(stations: &[f64], heights: &[f64], a: f64, b: f64) -> f64 {
    let (a, b) = (a.min(b), a.max(b));
    let mut m = interp(stations, heights, a).max(interp(stations, heights, b));
    for (s, h) in stations.iter().zip(heights) {
        if *s > a && *s < b {
            m = m.max(*h);
        }
    }
    m
}

/// Points along a deck's cross-section from `l` to `r`, about a metre apart.
fn section_samples(l: P2, r: P2) -> Vec<P2> {
    let n = (((r[0] - l[0]).hypot(r[1] - l[1])).ceil() as usize).max(2);
    (0..=n).map(|k| [l[0] + (r[0] - l[0]) * k as f64 / n as f64, l[1] + (r[1] - l[1]) * k as f64 / n as f64]).collect()
}

/// One thing a deck passes over, as stations of the deck.
struct CrossSpec {
    over: String,
    chain: Option<u32>,
    freeway: bool,
    /// Stations strictly over it.
    first: usize,
    last: usize,
    /// The stations the bound is applied at (one more each side, so the
    /// profile between stations clears it too) and what is under each.
    at: Vec<(usize, Under)>,
}

/// What a node fixes.
#[derive(Clone, Copy, Debug, PartialEq)]
enum NodeKind {
    Paved(f64),
    Landmark(f64),
    Shared,
    Free,
}

/// The per-chain data the solve reuses every round.
struct Work {
    ground: Vec<f64>,
    /// The bound before any crossing: the street datum on the ground; over
    /// open water the datum on the water and, for a freeway class, the
    /// soffit `WATER_FREEBOARD_M` above it.
    floor: Vec<f64>,
    tau: Vec<f64>,
    kappa: f64,
    cross: Vec<CrossSpec>,
}

fn interp(stations: &[f64], heights: &[f64], s: f64) -> f64 {
    let n = stations.len();
    if n == 0 || heights.len() != n {
        return f64::NEG_INFINITY;
    }
    if n == 1 {
        return heights[0];
    }
    let i = match stations.binary_search_by(|t| t.total_cmp(&s)) {
        Ok(i) => i.min(n - 2),
        Err(0) => 0,
        Err(i) => (i - 1).min(n - 2),
    };
    let t = ((s - stations[i]) / (stations[i + 1] - stations[i])).clamp(0.0, 1.0);
    heights[i] + (heights[i + 1] - heights[i]) * t
}

/// The longest stretch of a chain (station steps whose two ends' ground is
/// under mean sea level, `eo_data::SEA_LEVEL_Y`) over open water, metres.
pub(crate) fn open_water_run(stations: &[f64], ground: &[f64]) -> f64 {
    let sea = eo_data::SEA_LEVEL_Y;
    let (mut longest, mut run) = (0.0f64, 0.0f64);
    for (s, g) in stations.windows(2).zip(ground.windows(2)) {
        if g[0] < sea && g[1] < sea {
            run += s[1] - s[0];
            longest = longest.max(run);
        } else {
            run = 0.0;
        }
    }
    longest
}

/// Runs of consecutive stations: (first, last).
fn runs(mut idx: Vec<usize>) -> Vec<(usize, usize)> {
    idx.sort_unstable();
    idx.dedup();
    let mut out: Vec<(usize, usize)> = vec![];
    for i in idx {
        match out.last_mut() {
            Some(r) if r.1 + 1 == i => r.1 = i,
            _ => out.push((i, i)),
        }
    }
    out
}

/// The chains whose roadway passes within `extra` of p (centreline within
/// their half width plus `extra`) and that `keep` accepts: (chain, its
/// nearest station, distance), by chain id.
fn near(chains: &[Chain], index: &Grid<(u32, u32)>, p: P2, extra: f64, keep: &dyn Fn(&Chain) -> bool) -> Vec<(u32, f64, f64)> {
    let mut found: Vec<(u32, f64, f64)> = vec![];
    for &(ci, si) in index.at(p[0], p[1]) {
        let d_chain = &chains[ci as usize];
        if !keep(d_chain) {
            continue;
        }
        let i = si as usize;
        let (t, d, _) = project(p, d_chain.points[i], d_chain.points[i + 1]);
        let s = d_chain.cum[i] + t * (d_chain.cum[i + 1] - d_chain.cum[i]);
        if d > d_chain.half_at(s) + extra {
            continue;
        }
        match found.iter_mut().find(|f| f.0 == ci) {
            Some(f) if d < f.2 => *f = (ci, s, d),
            Some(_) => {}
            None => found.push((ci, s, d)),
        }
    }
    found.sort_by_key(|f| f.0);
    found
}

/// The chains `keep` accepts whose roadway the deck cross-section `l r`
/// (centre `p`) reaches over: (chain, its station nearest the section), by id.
fn under_chains(chains: &[Chain], index: &Grid<(u32, u32)>, p: P2, l: P2, r: P2, keep: &dyn Fn(&Chain) -> bool) -> Vec<(u32, f64)> {
    let mut found: Vec<(u32, f64, f64)> = vec![];
    for &(ci, si) in index.at(p[0], p[1]) {
        let d_chain = &chains[ci as usize];
        if !keep(d_chain) {
            continue;
        }
        let i = si as usize;
        let (a, b) = (d_chain.points[i], d_chain.points[i + 1]);
        let (d, q) = seg_distance(l, r, a, b);
        let (t, _, _) = project(q, a, b);
        let s = d_chain.cum[i] + t * (d_chain.cum[i + 1] - d_chain.cum[i]);
        if d > d_chain.half_at(s) {
            continue;
        }
        match found.iter_mut().find(|f| f.0 == ci) {
            Some(f) if d < f.2 => *f = (ci, s, d),
            Some(_) => {}
            None => found.push((ci, s, d)),
        }
    }
    found.sort_by_key(|f| f.0);
    found.into_iter().map(|f| (f.0, f.1)).collect()
}

pub(crate) fn build(streets: &StreetWorld, roads: &HashMap<String, RoadTags>, standards: &CaStandards, landmarks: Vec<LandmarkSpec>, options: StructureOptions) -> Result<StructureWorld> {
    let o = &streets.o;
    let (lift, snap) = (o.road_lift, o.node_snap);
    let limits = Limits::read(standards)?;
    let (pieces, gathered) = chains::gather(streets, roads, &landmarks);
    let max_half = pieces.iter().map(|p| p.half).fold(0.0, f64::max);
    let reach = max_half + BENT_FOOTPRINT_MARGIN_M + snap + 1.0;
    let paved = Paved::new(&streets.plan, reach, o.min_sidewalk_width, o.max_step);
    let nodes = chains::cluster(&pieces, snap);
    // a separated end meets the street where it touches a paved centreline
    // (a run's end or its middle); where separated pieces continue through a
    // node, only a paved run that ends there joins it (a street running
    // through passes under or over).
    let paved_node: Vec<bool> = nodes.at.iter().zip(&nodes.ends).map(|(&p, ends)| if ends.len() == 1 { paved.touches(p, snap) } else { paved.run_ends_at(p, snap) }).collect();
    let walked = chains::walk(&pieces, &nodes, &paved_node);

    // ---- chains, stations, ground ------------------------------------------------
    // stage A6 (`eo_street::joins`, only when the street was built with the
    // structures' joins): a chain end that is a leg of a junction starts at
    // the junction's mouth for it (the leg's trim back from the node), so the
    // deck meets the junction's pad there instead of lying over it.
    let legs = leg_trims(streets);
    let mut out: Vec<Chain> = vec![];
    for (id, w) in walked.iter().enumerate() {
        let (points, spans) = chains::centreline(&pieces, w);
        let ends_seg = [pieces[w.pieces[0].0 as usize].segment, pieces[w.pieces.last().unwrap().0 as usize].segment];
        let cut = [0, 1].map(|k| leg_cut(&legs, nodes.at[w.nodes[k] as usize], ends_seg[k], snap));
        let (points, spans) = cut_centreline(points, spans, cut);
        let cum = arc_table(&points);
        let len = cum.last().copied().unwrap_or(0.0);
        let first: &RawPiece = &pieces[w.pieces[0].0 as usize];
        let mut list: Vec<Piece> = w
            .pieces
            .iter()
            .zip(&spans)
            .map(|(&(pi, rev), &(s0, s1))| {
                let p = &pieces[pi as usize];
                Piece { segment: p.segment, key: p.key.clone(), way: p.way.clone(), name: p.name.clone(), highway: p.highway.clone(), half: p.half, s0, s1, reversed: rev }
            })
            .collect();
        if let Some(l) = list.last_mut() {
            l.s1 = len;
        }
        let n = ((len / STATION_MAX_M).ceil() as usize).max(1);
        let stations: Vec<f64> = (0..=n).map(|i| len * i as f64 / n as f64).collect();
        let end = |k: usize| ChainEnd { kind: EndKind::Free, node: w.nodes[k], at: nodes.at[w.nodes[k] as usize], fixed: None, surface: vec![] };
        out.push(Chain {
            id: id as u32,
            level: first.level,
            class: first.class,
            kind: first.kind,
            landmark: chains::deck_of(&landmarks, first.landmark),
            points,
            cum,
            pieces: list,
            ends: [end(0), end(1)],
            stations,
            heights: vec![],
            grade_limits: vec![],
            bents: vec![],
            bent_columns: vec![],
            supports: vec![],
            depths: vec![],
            crossings: vec![],
            status: Status::Built,
            merged_with: vec![],
            end_wants: [None, None],
        });
    }
    let landmark_of: Vec<Option<usize>> = walked.iter().map(|w| pieces[w.pieces[0].0 as usize].landmark).collect();
    let station_xz = |c: &Chain| -> Vec<P2> {
        c.stations
            .iter()
            .map(|&s| {
                let f = c.frame(s);
                [f.x, f.z]
            })
            .collect()
    };
    let xz: Vec<Vec<P2>> = out.iter().map(station_xz).collect();
    let ground: Vec<Vec<f64>> = xz.iter().map(|ps| ps.iter().map(|p| streets.ground(p[0], p[1])).collect()).collect();

    // landmark chains: the landmark's own deck.
    for c in out.iter_mut() {
        if let Some(li) = landmark_of[c.id as usize] {
            let l = &landmarks[li];
            c.heights = xz[c.id as usize].iter().map(|p| (l.top)((l.chainage)(p[0], p[1]))).collect();
        }
    }

    // ---- nodes and ends -------------------------------------------------------------
    let mut at_node: Vec<Vec<(u32, usize)>> = vec![vec![]; nodes.at.len()];
    for c in &out {
        at_node[c.ends[0].node as usize].push((c.id, 0));
        at_node[c.ends[1].node as usize].push((c.id, 1));
    }
    let node_kind: Vec<NodeKind> = (0..nodes.at.len())
        .map(|n| {
            let ends = &at_node[n];
            if let Some(&(ci, e)) = ends.iter().find(|(ci, _)| out[*ci as usize].kind == ChainKind::Landmark) {
                let h = &out[ci as usize].heights;
                return NodeKind::Landmark(if e == 0 { h[0] } else { h[h.len() - 1] });
            }
            if paved_node[n] {
                let p = nodes.at[n];
                return NodeKind::Paved(streets.ground(p[0], p[1]) + lift);
            }
            if ends.len() >= 2 { NodeKind::Shared } else { NodeKind::Free }
        })
        .collect();
    // a paved end meets the street as drawn at the chain's own end point (a
    // stub's ribbon, or a junction's pad), less the crown the deck's own top
    // has there (the meshes blend the street's crown in at a paved end);
    // where nothing is drawn there, the street datum.
    // (stage A6: a stub raised on an approach embankment, `eo_street::joins`,
    // is met at its raised datum.)
    let rises: Vec<(P2, f64)> = o.structures.iter().flat_map(|j| j.ends.iter().flat_map(|e| [e.rise, e.cut].into_iter().flatten().map(move |r| (e.at, r.to)))).collect();
    let end_height = |tip: P2, half: f64| -> f64 {
        let datum = streets.ground(tip[0], tip[1]) + lift;
        let crown = o.cross_slope * half;
        let raised_to = |y: f64| rises.iter().any(|(at, to)| (at[0] - tip[0]).hypot(at[1] - tip[1]) <= snap && (y - crown - to).abs() <= END_DRAWN_M);
        match (streets.paved_layer_at(tip[0], tip[1]), streets.paved_at(tip[0], tip[1])) {
            (Some(eo_street::Layer::Carriageway), Some(y)) if (y - crown - datum).abs() <= END_DRAWN_M || raised_to(y) => y - crown,
            _ => datum,
        }
    };
    // the street's surface just past a paved end, across the deck at the
    // deck top's points (`sweep.rs`), over the end's height: what the deck
    // top blends to there; the street's own crown where nothing sensible is
    // drawn.
    let end_surface = |c: &Chain, e: usize, fixed: f64| -> Vec<[f64; 2]> {
        let s_end = if e == 0 { 0.0 } else { c.length() };
        let f = c.frame(s_end);
        let out_dir = if e == 0 { [-f.tx, -f.tz] } else { [f.tx, f.tz] };
        let half = if e == 0 { c.pieces[0].half } else { c.pieces.last().unwrap().half };
        crate::sweep::deck_offsets(half, o.gutter_width)
            .into_iter()
            .map(|u| {
                let q = [f.x + f.nx * u + out_dir[0] * END_SAMPLE_M, f.z + f.nz * u + out_dir[1] * END_SAMPLE_M];
                let crown = o.cross_section_y(0.0, u, half);
                let drawn = match (streets.paved_layer_at(q[0], q[1]), streets.paved_at(q[0], q[1])) {
                    (Some(eo_street::Layer::Carriageway), Some(y)) if (y - fixed - crown).abs() <= END_SURFACE_M => y - fixed,
                    _ => crown,
                };
                [u, drawn]
            })
            .collect()
    };
    for ci in 0..out.len() {
        let tips = [out[ci].points[0], *out[ci].points.last().unwrap()];
        let halves = [out[ci].pieces[0].half, out[ci].pieces.last().unwrap().half];
        for e in 0..2 {
            let (tip, half) = (tips[e], halves[e]);
            // a paved end meets the street at the chain's own end point.
            let (kind, fixed) = match node_kind[out[ci].ends[e].node as usize] {
                NodeKind::Paved(_) => (EndKind::Paved, Some(end_height(tip, half))),
                NodeKind::Landmark(h) => (EndKind::Landmark, Some(h)),
                NodeKind::Shared => (EndKind::Shared, None),
                NodeKind::Free => (EndKind::Free, None),
            };
            let surface = if kind == EndKind::Paved { end_surface(&out[ci], e, fixed.unwrap()) } else { vec![] };
            let end = &mut out[ci].ends[e];
            end.at = tip;
            (end.kind, end.fixed, end.surface) = (kind, fixed, surface);
        }
    }

    // ---- the chain index (every chain's centreline) -----------------------------------
    let mut index: Grid<(u32, u32)> = Grid::default();
    for c in &out {
        let pad = c.max_half() + reach;
        for i in 0..c.points.len().saturating_sub(1) {
            index.insert_seg(c.points[i], c.points[i + 1], pad, (c.id, i as u32));
        }
    }
    // ---- grade limits ---------------------------------------------------------------
    // A motorway_link is a freeway-to-freeway connector when, from each end
    // through other links (never through a paved street), it reaches a
    // motorway or trunk chain.
    let reaches_freeway = |start: u32, node: u32| -> bool {
        let mut seen: HashSet<u32> = HashSet::new();
        let mut queue = vec![node];
        while let Some(n) = queue.pop() {
            if !seen.insert(n) || matches!(node_kind[n as usize], NodeKind::Paved(_)) {
                continue;
            }
            for &(ci, e) in &at_node[n as usize] {
                if ci == start {
                    continue;
                }
                let c = &out[ci as usize];
                match c.class {
                    RoadClass::Motorway | RoadClass::Trunk => return true,
                    RoadClass::MotorwayLink => queue.push(c.ends[1 - e].node),
                    RoadClass::Other => {}
                }
            }
        }
        false
    };
    let connector: Vec<bool> = out.iter().map(|c| c.class == RoadClass::MotorwayLink && reaches_freeway(c.id, c.ends[0].node) && reaches_freeway(c.id, c.ends[1].node)).collect();
    for c in out.iter_mut() {
        if c.kind == ChainKind::Landmark {
            continue;
        }
        let g = &ground[c.id as usize];
        let len = c.length();
        let ground_s = |s: f64| interp(&c.stations, g, s.clamp(0.0, len));
        // the natural ground's grade over one target span about s.
        let ground_grade = |s: f64| {
            let (a, b) = ((s - MAX_SPAN_M / 2.0).max(0.0), (s + MAX_SPAN_M / 2.0).min(len));
            if b - a < 1e-6 { 0.0 } else { (ground_s(b) - ground_s(a)).abs() / (b - a) }
        };
        let limits_of = |s: f64| -> f64 {
            let hw = c.piece_at(s).highway.as_str();
            match hw {
                // Table 204.3 by terrain: rolling, or where the ground along the
                // chain is steeper than rolling, as steep as the ground up to
                // mountainous. This picks among the table's own keys.
                "motorway" | "trunk" => limits.rolling.max(limits.mountainous.min(ground_grade(s))),
                "motorway_link" if connector[c.id as usize] => limits.connector,
                h if h.ends_with("_link") => limits.ramp,
                _ => limits.connector,
            }
        };
        c.grade_limits = c.stations.windows(2).map(|w| limits_of((w[0] + w[1]) / 2.0)).collect();
    }

    // ---- bents and depths -------------------------------------------------------------
    // Room for a column (bents.rs): off paving; else a street's median; else
    // its parking lane or kerb-side furniture zone; else a straddle bent.
    // Always off the roadway of any chain below this one (and, past the deck
    // edge, of any other deck of its level), by the column footprint.
    let kerb = KerbRule { curb_top: o.curb_top_width, frontage: limits.frontage, through: limits.through };
    let sites = Sites::new(streets, &paved, kerb);
    let rooms: Vec<Vec<(f64, Room)>> = out
        .iter()
        .map(|c| {
            if c.kind != ChainKind::Deck {
                return vec![];
            }
            // (another deck's edge is its barrier's outer face: the column's
            // footprint keeps a gap from it.)
            let reach = BENT_FOOTPRINT_MARGIN_M + limits.barrier_base + COLUMN_DECK_GAP_M;
            let free = |p: P2, outside: bool| -> bool {
                let under = |d: &Chain| d.id != c.id && d.kind != ChainKind::Tunnel && (d.level < c.level || (outside && d.level <= c.level));
                near(&out, &index, p, reach, &under).is_empty()
            };
            bents::place(&sites, c, &free)
        })
        .collect();
    let (rooms, relaxed_bent_chains) = second_tier(&out, rooms, &index, &sites, &limits);
    let mut rooms_of: Vec<Vec<Room>> = vec![];
    for (c, list) in out.iter_mut().zip(rooms) {
        if c.kind != ChainKind::Deck {
            rooms_of.push(vec![]);
            continue;
        }
        let b: Vec<f64> = list.iter().map(|x| x.0).collect();
        let mut edges = vec![0.0];
        edges.extend(&b);
        edges.push(c.length());
        c.depths = edges.windows(2).map(|w| (w[1] - w[0]) * limits.depth_ratio).collect();
        c.bents = b;
        c.bent_columns = list.iter().map(|(_, rm)| rm.zones.iter().map(|z| [z.lo, z.hi]).collect()).collect();
        rooms_of.push(list.into_iter().map(|x| x.1).collect());
    }

    // ---- crossings ------------------------------------------------------------------
    // A deck clears the paved streets and the lower chains it passes over.
    // At-grade railways are not among them yet: `eo_data::City` does not load
    // the city file's `railWays`, so no railway clearance is asked.
    let mut work: Vec<Work> = out
        .iter()
        .map(|c| {
            let mut tau = vec![0.0; c.stations.len()];
            for i in 1..tau.len() {
                tau[i] = tau[i - 1] + c.grade_limits.get(i - 1).copied().unwrap_or(0.0) * (c.stations[i] - c.stations[i - 1]);
            }
            let gmin = c.grade_limits.iter().copied().fold(f64::INFINITY, f64::min);
            let kappa = if gmin.is_finite() { gmin * VERTICAL_CURVE_M / 2.0 } else { 0.0 };
            let g = &ground[c.id as usize];
            let sea = eo_data::SEA_LEVEL_Y;
            let last = c.stations.len() - 1;
            let floor = c
                .stations
                .iter()
                .zip(g)
                .enumerate()
                .map(|(i, (&s, &y))| match c.kind {
                    // (an end is the street datum, wherever it is.)
                    _ if i == 0 || i == last => y + lift,
                    // a freeway's superstructure stays out of the water; a city
                    // street's low (movable) bridge only keeps its deck above it.
                    ChainKind::Deck if y < sea && c.class != RoadClass::Other => (sea + lift).max(sea + WATER_FREEBOARD_M + c.depth_at(s)),
                    ChainKind::Deck if y < sea => sea + lift,
                    _ => y + lift,
                })
                .collect::<Vec<f64>>();
            // a paved end sits at the street as drawn there, which may be a
            // little under the datum the floor asks: next to it the floor asks
            // no more than the end's own grade reaches.
            let mut floor = floor;
            for (e, end) in c.ends.iter().enumerate() {
                if let (EndKind::Paved, Some(h)) = (end.kind, end.fixed) {
                    let te = if e == 0 { tau[0] } else { tau[last] };
                    for (f, t) in floor.iter_mut().zip(&tau) {
                        *f = f.min(h.max(*f - END_DRAWN_M) + (t - te).abs());
                    }
                }
            }
            Work { ground: g.clone(), floor, tau, kappa, cross: vec![] }
        })
        .collect();
    let mut static_holds: BTreeMap<u32, Hold> = BTreeMap::new();
    // a span longer than any the city records: the bents have no room.
    for c in &out {
        let mut edges = vec![0.0];
        edges.extend(&c.bents);
        edges.push(c.length());
        let longest = edges.windows(2).map(|w| w[1] - w[0]).fold(0.0, f64::max);
        if c.kind == ChainKind::Deck && longest > LONGEST_SPAN_M {
            static_holds.insert(c.id, Hold::NoBentRoom { span: longest.round() as u32 });
        }
    }
    // the routes (OSM way ids and names) of every paved run and every chain,
    // for the same-route overlap rule (route.rs).
    let names_of = |list: &mut Vec<String>| {
        list.retain(|n| !n.is_empty());
        list.sort();
        list.dedup();
    };
    let entry_route: Vec<(Vec<String>, Vec<String>)> = streets
        .plan
        .entries
        .iter()
        .map(|e| {
            let mut ways: Vec<String> = e.pieces.iter().filter_map(|&i| streets.city.segments[i].street_id.as_deref().map(|s| crate::way_key(s).to_string())).collect();
            let mut names: Vec<String> = e.pieces.iter().map(|&i| streets.city.segments[i].street_name.clone()).collect();
            names_of(&mut ways);
            names_of(&mut names);
            (ways, names)
        })
        .collect();
    let chain_route: Vec<(Vec<String>, Vec<String>)> = out
        .iter()
        .map(|c| {
            let mut ways: Vec<String> = c.ways().into_iter().map(str::to_string).collect();
            let mut names: Vec<String> = c.pieces.iter().map(|p| p.name.clone()).collect();
            names_of(&mut ways);
            names_of(&mut names);
            (ways, names)
        })
        .collect();
    let shares = |a: &(Vec<String>, Vec<String>), b: &(Vec<String>, Vec<String>)| a.0.iter().any(|w| b.0.contains(w)) || a.1.iter().any(|n| b.1.contains(n));
    // A paved run (`Ok(entry)`) or lower chain (`Err(chain)`) under stations
    // a..=b of `c` is the same road as `c` when it carries one of its ways or
    // its name, runs along it there, and joins it through pieces of that
    // route within `route::REACH_M`.
    let same_route = |c: &Chain, under: Result<u32, u32>, a: usize, b: usize| -> bool {
        let mine = &chain_route[c.id as usize];
        let theirs = match under {
            Ok(e) => &entry_route[e as usize],
            Err(d) => &chain_route[d as usize],
        };
        if !shares(mine, theirs) {
            return false;
        }
        let mid = c.stations[(a + b) / 2];
        let f = c.frame(mid);
        let other = match under {
            Ok(e) => Elem::new(streets.plan.entries[e as usize].points.clone()),
            Err(d) => Elem::new(out[d as usize].points.clone()),
        };
        let (at, _) = other.project([f.x, f.z]);
        if !route::along([f.tx, f.tz], other.dir(at)) {
            return false;
        }
        let mut elems = vec![Elem::new(c.points.clone()), other];
        for (j, r) in chain_route.iter().enumerate() {
            if j as u32 != c.id && under != Err(j as u32) && shares(mine, r) {
                elems.push(Elem::new(out[j].points.clone()));
            }
        }
        for (j, r) in entry_route.iter().enumerate() {
            if under != Ok(j as u32) && shares(mine, r) && streets.plan.entries[j].structure_leg.is_none() {
                elems.push(Elem::new(streets.plan.entries[j].points.clone()));
            }
        }
        route::connected(&elems, [c.stations[a], c.stations[b]], at, snap)
    };
    let mut same_route_ignored = 0usize;
    // each junction pad's highest drawn point (eo-street's `emit_node`: a fan
    // from an apex at the approaches' mean datum plus the crown, to each
    // approach mouth's cross-section at its own datum).
    let node_top: Vec<f64> = streets
        .plan
        .nodes
        .iter()
        .map(|n| {
            let ends: Vec<(f64, f64)> = n
                .approaches
                .iter()
                .map(|a| {
                    let f = eo_street::plan::approach_end_frame(&streets.plan.entries, a);
                    (streets.ground(f.x, f.z) + lift, a.half)
                })
                .collect();
            if ends.is_empty() {
                return f64::NEG_INFINITY;
            }
            let mean = ends.iter().map(|e| e.0).sum::<f64>() / ends.len() as f64;
            let mean_half = ends.iter().map(|e| e.1).sum::<f64>() / ends.len() as f64;
            ends.iter().map(|e| e.0 + o.cross_slope * e.1).fold(mean + o.cross_slope * mean_half, f64::max)
        })
        .collect();
    for c in &out {
        if c.kind != ChainKind::Deck {
            continue;
        }
        let pts = &xz[c.id as usize];
        let last = pts.len() - 1;
        // (station, crossed datum or station of the lower chain, signed offset of
        // this centreline from the crossed one, the crossed half width).
        let mut streets_at: BTreeMap<u32, Vec<(usize, f64, f64, f64)>> = BTreeMap::new();
        let mut chains_at: BTreeMap<u32, Vec<(usize, f64, f64, f64)>> = BTreeMap::new();
        let mut chain_span: HashMap<(u32, usize), (f64, f64)> = HashMap::new();
        let lower = |d: &Chain| d.id != c.id && d.level < c.level && d.kind != ChainKind::Tunnel;
        for (i, &p) in pts.iter().enumerate() {
            // the deck's cross-section at this station: over whatever's
            // roadway it reaches (the crossed way's width plus this deck's
            // half width, measured across this deck).
            let f = c.frame(c.stations[i]);
            let h = c.half_at(c.stations[i]);
            let (l, r) = ([f.x + f.nx * h, f.z + f.nz * h], [f.x - f.nx * h, f.z - f.nz * h]);
            let samples = section_samples(l, r);
            // the highest drawn street surface under the section: a run's
            // ribbon (its datum and crown, `StreetOptions::cross_section_y`)
            // where one covers a sample, else a junction's pad (its apex or
            // highest approach mouth).
            let mut section_top = f64::NEG_INFINITY;
            for q in &samples {
                let mut on_ribbon = false;
                for n in paved.runs_near(*q, f64::INFINITY) {
                    let e = &streets.plan.entries[n.entry as usize];
                    if n.dist > e.half {
                        continue;
                    }
                    if n.s >= e.s0() && n.s <= e.s1() {
                        on_ribbon = true;
                    }
                    let f = e.frame(n.s, false);
                    section_top = section_top.max(streets.ground(f.x, f.z) + lift + o.cross_section_y(0.0, n.dist, e.half));
                }
                if !on_ribbon {
                    for nd in paved.pads_at(*q) {
                        section_top = section_top.max(node_top[nd as usize]);
                    }
                }
            }
            for u in paved.under_section(p, l, r) {
                let e = &streets.plan.entries[u.entry as usize];
                let half = e.half;
                let top = (streets.ground(u.at[0], u.at[1]) + lift + o.cross_slope * half).max(section_top);
                streets_at.entry(u.entry).or_default().push((i, top + SURFACE_MARGIN_M, u.side, half));
            }
            for (di, sd) in under_chains(&out, &index, p, l, r, &lower) {
                let d = &out[di as usize];
                let g = d.frame(sd);
                let side = (p[0] - g.x) * g.nx + (p[1] - g.z) * g.nz;
                // the stretch of the lower deck under the section.
                let (mut lo, mut hi) = (sd, sd);
                for q in &samples {
                    for &(ci, si) in index.at(q[0], q[1]) {
                        if ci != di {
                            continue;
                        }
                        let k = si as usize;
                        let (t, dist, _) = project(*q, d.points[k], d.points[k + 1]);
                        let s2 = d.cum[k] + t * (d.cum[k + 1] - d.cum[k]);
                        if dist <= d.half_at(s2) {
                            lo = lo.min(s2);
                            hi = hi.max(s2);
                        }
                    }
                }
                chains_at.entry(di).or_default().push((i, sd, side, d.half_at(sd)));
                chain_span.insert((di, i), (lo, hi));
            }
        }
        // a run passes over what it overlaps only if the centrelines cross
        // in it or this centreline is over the crossed roadway somewhere in
        // it; footprints that only graze side by side are not a crossing.
        let passes_over = |list: &[(usize, f64, f64, f64)], a: usize, b: usize| -> bool {
            let run: Vec<&(usize, f64, f64, f64)> = list.iter().filter(|x| x.0 >= a && x.0 <= b).collect();
            run.iter().any(|x| x.2.abs() <= x.3) || run.windows(2).any(|w| w[0].2 * w[1].2 <= 0.0)
        };
        let mut specs = vec![];
        let shared_with = |d: &Chain, e: usize| d.ends.iter().any(|x| x.node == c.ends[e].node);
        for (entry, list) in streets_at {
            let e = &streets.plan.entries[entry as usize];
            let seg = &streets.city.segments[e.road];
            let name = if seg.street_name.is_empty() { e.id.clone() } else { format!("{} ({})", seg.street_name, e.id) };
            let freeway = freeway_clearance(&e.highway);
            for (a, b) in runs(list.iter().map(|x| x.0).collect()) {
                // the street a paved end lands on is met at grade, not crossed.
                if (a == 0 && c.ends[0].kind == EndKind::Paved) || (b == last && c.ends[1].kind == EndKind::Paved) || !passes_over(&list, a, b) {
                    continue;
                }
                if same_route(c, Ok(entry), a, b) {
                    same_route_ignored += 1;
                    continue;
                }
                // (the highest of this station's section and its neighbours', so
                // the steps between stations clear too.)
                let y_of = |i: usize| list.iter().filter(|x| x.0 + 1 >= i && x.0 <= i + 1).map(|x| x.1).fold(f64::NEG_INFINITY, f64::max);
                let mut at: Vec<(usize, Under)> = (a..=b).map(|i| (i, Under::Street(y_of(i)))).collect();
                if a > 0 {
                    at.push((a - 1, Under::Street(y_of(a))));
                }
                if b < last {
                    at.push((b + 1, Under::Street(y_of(b))));
                }
                specs.push(CrossSpec { over: name.clone(), chain: None, freeway, first: a, last: b, at });
            }
        }
        for (di, list) in chains_at {
            let d = &out[di as usize];
            let name = format!("{} (chain {}, level {})", if d.name().is_empty() { "unnamed" } else { d.name() }, d.id, d.level);
            for (a, b) in runs(list.iter().map(|x| x.0).collect()) {
                // a chain this one meets at its end is joined there, not crossed.
                if (a == 0 && shared_with(d, 0)) || (b == last && shared_with(d, 1)) || !passes_over(&list, a, b) {
                    continue;
                }
                if same_route(c, Err(di), a, b) {
                    same_route_ignored += 1;
                    continue;
                }
                let s_of = |i: usize| list.iter().find(|x| x.0 == i).map(|x| x.1).unwrap();
                // the lower deck under this station's section and its neighbours'.
                let span_of = |i: usize| {
                    let mut sp = (f64::INFINITY, f64::NEG_INFINITY);
                    for j in i.saturating_sub(1)..=i + 1 {
                        if let Some(&(lo, hi)) = chain_span.get(&(di, j)) {
                            sp = (sp.0.min(lo), sp.1.max(hi));
                        }
                    }
                    Under::Chain(sp.0, sp.1)
                };
                let freeway = freeway_clearance(&d.piece_at(s_of(a)).highway);
                let mut at: Vec<(usize, Under)> = (a..=b).map(|i| (i, span_of(i))).collect();
                if a > 0 {
                    at.push((a - 1, span_of(a)));
                }
                if b < last {
                    at.push((b + 1, span_of(b)));
                }
                specs.push(CrossSpec { over: name.clone(), chain: Some(di), freeway, first: a, last: b, at });
            }
        }
        if specs.iter().any(|s| limits.clearance(s.freeway).is_none()) {
            static_holds.insert(c.id, Hold::NoSourcedClearance);
        }
        // over open water for longer than a creek crossing: a navigation
        // clearance would set the deck there, and the table has none.
        if open_water_run(&c.stations, &ground[c.id as usize]) > OPEN_WATER_HOLD_M {
            static_holds.insert(c.id, Hold::NoSourcedClearance);
        }
        work[c.id as usize].cross = specs;
    }
    // same-level decks whose centrelines cross with no node between them:
    // which is on top is not known.
    let mut unknown = 0usize;
    for c in &out {
        if c.kind != ChainKind::Deck {
            continue;
        }
        let mut with: Option<u32> = None;
        for i in 0..c.points.len() - 1 {
            for &(di, si) in index.at(c.points[i][0], c.points[i][1]).iter().chain(index.at(c.points[i + 1][0], c.points[i + 1][1])) {
                let d = &out[di as usize];
                if di == c.id || d.level != c.level || !matches!(d.kind, ChainKind::Deck | ChainKind::Landmark) {
                    continue;
                }
                let j = si as usize;
                if crate::index::crossing(c.points[i], c.points[i + 1], d.points[j], d.points[j + 1]).is_some() && with.is_none_or(|w| di < w) {
                    with = Some(di);
                }
            }
        }
        if let Some(di) = with {
            unknown += 1;
            let d = &out[di as usize];
            let what = format!("{} (chain {}, level {})", if d.name().is_empty() { "unnamed" } else { d.name() }, d.id, d.level);
            static_holds.entry(c.id).or_insert(Hold::UnknownCrossing { with: what });
        }
    }

    // ---- the solve ----------------------------------------------------------------------
    let order: Vec<u32> = {
        let mut v: Vec<u32> = out.iter().filter(|c| c.kind != ChainKind::Landmark).map(|c| c.id).collect();
        v.sort_by_key(|&id| (out[id as usize].level, id));
        v
    };
    // the passes: each holds the chains the last could not meet, and solves again.
    let run = |ties: &[Tie], start: &BTreeMap<u32, Hold>, start_heights: &HashMap<u32, Vec<f64>>, start_wants: &BTreeMap<(u32, usize), f64>| -> (BTreeMap<u32, Hold>, HashMap<u32, Vec<f64>>, BTreeMap<(u32, usize), f64>, Solved, usize) {
        let mut held: BTreeMap<u32, Hold> = start.clone();
        let mut held_heights: HashMap<u32, Vec<f64>> = start_heights.clone();
        let mut held_wants: BTreeMap<(u32, usize), f64> = start_wants.clone();
        let mut solved = Solved::default();
        let mut passes = 0;
        for _ in 0..PASSES {
            passes += 1;
            solved = solve_pass(&out, &work, &order, &held, &node_kind, &limits, ties);
            let fresh: Vec<(u32, Hold)> = solved.violations.iter().filter(|(id, _)| !held.contains_key(id)).map(|(id, h)| (*id, h.clone())).collect();
            if fresh.is_empty() {
                break;
            }
            for (id, h) in fresh {
                if let Some(y) = solved.heights.get(&id) {
                    held_heights.insert(id, y.clone());
                }
                for e in 0..2 {
                    if let Some(&v) = solved.wants.get(&(id, e)) {
                        held_wants.insert((id, e), v);
                    }
                }
                held.insert(id, h);
            }
        }
        (held, held_heights, held_wants, solved, passes)
    };
    let (mut held, mut held_heights, mut held_wants, mut solved, mut passes) = run(&[], &static_holds, &HashMap::new(), &BTreeMap::new());
    // same-level decks whose roadways overlap (a merge's gore, twin
    // carriageways side by side) are solved again to meet there (stage A5),
    // unless the free solve put one far enough over the other to pass over
    // it (a genuine over/under pair near a shared node).
    let candidates = merge_ties(&out, limits.barrier_base);
    let ties: Vec<Tie> = candidates
        .into_iter()
        .filter(|t| {
            let (Some(ym), Some(yj)) = (solved.heights.get(&t.minor), solved.heights.get(&t.major)) else { return true };
            if held.contains_key(&t.minor) || held.contains_key(&t.major) {
                return true;
            }
            let (m, j) = (&out[t.minor as usize], &out[t.major as usize]);
            t.at.iter().all(|&(i, sj)| {
                let (a, b) = (ym[i], interp(&j.stations, yj, sj));
                let (upper, s_up) = if a > b { (m, m.stations[i]) } else { (j, sj) };
                let lower = if a > b { j } else { m };
                let clear = limits.clearance(freeway_clearance(&lower.piece_at(if a > b { sj } else { m.stations[i] }).highway)).unwrap_or(f64::INFINITY);
                (a - b).abs() < clear + upper.depth_at(s_up)
            })
        })
        .collect();
    let merged: Vec<(u32, u32)> = ties.iter().map(|t| (t.minor, t.major)).collect();
    if !ties.is_empty() {
        // (what the free solve held stays held, for its own reason: the ties only add bounds.)
        let more;
        (held, held_heights, held_wants, solved, more) = run(&ties, &held, &held_heights, &held_wants);
        passes += more;
    }
    for (minor, major) in &merged {
        if !held.contains_key(minor) && !held.contains_key(major) {
            out[*minor as usize].merged_with.push(*major);
            out[*major as usize].merged_with.push(*minor);
        }
    }
    for c in out.iter_mut() {
        c.merged_with.sort_unstable();
        c.merged_with.dedup();
    }

    // ---- results ------------------------------------------------------------------------
    for c in out.iter_mut() {
        if c.kind == ChainKind::Landmark {
            continue;
        }
        if let Some(h) = held.get(&c.id) {
            c.status = Status::Held(h.clone());
            c.heights = held_heights.remove(&c.id).unwrap_or_default();
            if matches!(h, Hold::EndTooLow { .. }) {
                c.end_wants = [held_wants.get(&(c.id, 0)).copied(), held_wants.get(&(c.id, 1)).copied()];
            }
        } else {
            c.heights = solved.heights.remove(&c.id).unwrap_or_default();
            if c.kind == ChainKind::Tunnel && !options.tunnels {
                c.status = Status::TunnelOff;
            }
        }
        for e in c.ends.iter_mut() {
            if e.kind == EndKind::Shared {
                e.fixed = solved.node_h.get(e.node as usize).copied().filter(|h| h.is_finite());
            }
        }
    }
    // what each deck achieved over what it crosses.
    let mut crossings: Vec<Vec<Crossing>> = vec![];
    for c in &out {
        let w = &work[c.id as usize];
        let list = w
            .cross
            .iter()
            .map(|sp| {
                let clearance = limits.clearance(sp.freeway).unwrap_or(f64::NAN);
                let mut achieved = f64::INFINITY;
                if c.heights.len() == c.stations.len() {
                    for &(i, under) in &sp.at {
                        if i < sp.first || i > sp.last {
                            continue;
                        }
                        let y_under = match under {
                            Under::Street(y) => y,
                            Under::Chain(lo, hi) => {
                                let d = &out[sp.chain.unwrap() as usize];
                                interp_max(&d.stations, &d.heights, lo, hi) + crown_over(d, lo, hi)
                            }
                        };
                        achieved = achieved.min(c.heights[i] - c.depth_at(c.stations[i]) - y_under);
                    }
                }
                Crossing { over: sp.over.clone(), chain: sp.chain, freeway: sp.freeway, s0: c.stations[sp.first], s1: c.stations[sp.last], clearance, achieved }
            })
            .collect();
        crossings.push(list);
    }
    for (c, list) in out.iter_mut().zip(crossings) {
        c.crossings = list;
    }

    // ---- supports: the columns and cap of every built deck's bents -------------------------
    let all_supports: Vec<Option<Vec<Bent>>> = out
        .iter()
        .map(|c| {
            if !(c.is_built() && c.kind == ChainKind::Deck) || c.heights.len() != c.stations.len() {
                return None;
            }
            // the stretches over something crossed (the stations the bound was applied at).
            let over: Vec<[f64; 2]> = work[c.id as usize]
                .cross
                .iter()
                .map(|sp| {
                    let lo = sp.at.iter().map(|x| x.0).min().unwrap_or(sp.first);
                    let hi = sp.at.iter().map(|x| x.0).max().unwrap_or(sp.last);
                    [c.stations[lo], c.stations[hi]]
                })
                .collect();
            Some(supports(c, &out, &rooms_of[c.id as usize], &over, &sites, streets, &limits))
        })
        .collect();
    for (c, s) in out.iter_mut().zip(all_supports) {
        if let Some(s) = s {
            c.supports = s;
        }
    }
    // a shared bent's twin: the other chain's bent on the same line (or none,
    // when that chain was not built: then each keeps what stands under it).
    let mut pairs: Vec<(u32, usize, Option<u32>)> = vec![];
    for c in &out {
        for (k, b) in c.supports.iter().enumerate() {
            let (Some(t), Some(sd)) = (b.twin, rooms_of[c.id as usize][k].twin.map(|x| x.s)) else { continue };
            let d = &out[t.chain as usize];
            let j = d.supports.iter().position(|e| e.twin.is_some_and(|x| x.chain == c.id && x.carried != t.carried) && (e.s - sd).abs() < 1e-6);
            pairs.push((c.id, k, j.map(|j| j as u32)));
        }
    }
    for (ci, k, j) in pairs {
        let b = &mut out[ci as usize].supports[k];
        match j {
            Some(j) => b.twin.as_mut().unwrap().bent = j,
            None => b.twin = None,
        }
    }

    // ---- indexes for the queries -----------------------------------------------------------
    let mut decks: HashMap<(i8, i32, i32), Vec<(u32, u32)>> = HashMap::new();
    let mut levels: HashMap<String, i8> = HashMap::new();
    let mut deck_levels: Vec<i8> = vec![];
    for c in &out {
        if !c.is_built() {
            continue;
        }
        deck_levels.push(c.level);
        for p in &c.pieces {
            levels.entry(p.way.clone()).or_insert(c.level);
        }
        let pad = c.max_half() + DECK_QUERY_MARGIN_M;
        let mut g: Grid<(u32, u32)> = Grid::default();
        for i in 0..c.points.len().saturating_sub(1) {
            g.insert_seg(c.points[i], c.points[i + 1], pad, (c.id, i as u32));
        }
        for (k, v) in g.into_map() {
            decks.entry((c.level, k.0, k.1)).or_default().extend(v);
        }
    }
    deck_levels.sort_unstable();
    deck_levels.dedup();
    let mut census = crate::census::Census::of(&out, &gathered, unknown, passes, solved.rounds);
    census.same_route_ignored = same_route_ignored;
    census.relaxed_bent_chains = relaxed_bent_chains;
    for c in &out {
        if c.is_built() && c.kind == ChainKind::Deck && c.heights.len() == c.stations.len() {
            let above = c.heights.iter().zip(&ground[c.id as usize]).map(|(h, g)| h - g).fold(f64::NEG_INFINITY, f64::max);
            census.note_height(above, c);
        }
    }
    Ok(StructureWorld { options, chains: out, decks, deck_levels, levels, census, barrier_base: limits.barrier_base, ..Default::default() })
}

/// Stage A6: every structure leg in a junction (`eo_street::plan::Entry::structure_leg`):
/// (its structure end point, the structure's end segment, the leg's trim at
/// its junction). Empty for a street built without the joins.
fn leg_trims(streets: &StreetWorld) -> Vec<(P2, usize, f64)> {
    let Some(joins) = streets.o.structures.as_ref() else { return vec![] };
    let plan = &streets.plan;
    plan.entries
        .iter()
        .filter_map(|e| {
            let end = joins.ends.get(e.structure_leg?)?;
            let start = eo_street::plan::leg_at_start(e, &streets.o);
            let (n, a) = if start { e.approach_start? } else { e.approach_end? };
            Some((end.at, end.segment, plan.nodes[n].approaches[a].trim))
        })
        .collect()
}

/// The trim of the leg (if any) of the chain end at `p` whose end piece is
/// street segment `segment`.
fn leg_cut(legs: &[(P2, usize, f64)], p: P2, segment: usize, snap: f64) -> f64 {
    legs.iter().filter(|(at, sg, _)| *sg == segment && (at[0] - p[0]).hypot(at[1] - p[1]) <= snap).map(|l| l.2).fold(0.0, f64::max)
}

/// A centreline (and its pieces' stretches) with `cut[0]` metres taken off
/// its start and `cut[1]` off its end; unchanged when nothing is cut or the
/// cuts would leave less than `LEG_MIN_LEFT_M`.
fn cut_centreline(points: Vec<P2>, spans: Vec<(f64, f64)>, cut: [f64; 2]) -> (Vec<P2>, Vec<(f64, f64)>) {
    let cum = arc_table(&points);
    let len = cum.last().copied().unwrap_or(0.0);
    let (c0, c1) = (cut[0].max(0.0), cut[1].max(0.0));
    if (c0 <= 0.0 && c1 <= 0.0) || len - c0 - c1 < LEG_MIN_LEFT_M {
        return (points, spans);
    }
    let (a, b) = (c0, len - c1);
    let at = |s: f64| {
        let f = eo_street::plan::frame_at(&points, &cum, s, false);
        [f.x, f.z]
    };
    let mut out = vec![at(a)];
    for (p, &c) in points.iter().zip(&cum) {
        if c > a + 1e-6 && c < b - 1e-6 {
            out.push(*p);
        }
    }
    out.push(at(b));
    let spans = spans.into_iter().map(|(s0, s1)| ((s0 - a).clamp(0.0, b - a), (s1 - a).clamp(0.0, b - a))).collect();
    (out, spans)
}

/// A leg's cut leaves at least this much of a chain (a solve resolution).
const LEG_MIN_LEFT_M: f64 = 8.0;

/// How far apart (fascia to fascia) two parallel decks of one route may run
/// and still stand on shared bents, one cap spanning the gap between them
/// (stage A5; the Central Viaduct's three round columns under one cap,
/// reference sheet 1.2). INFERRED, structure-only: a straddle bent's
/// outrigger reach.
pub(crate) const TWIN_GAP_M: f64 = crate::OUTRIGGER_REACH_M;

/// The longest span bents at `bents` leave on `c`.
fn longest_span(c: &Chain, bents: &[f64]) -> f64 {
    let mut edges = vec![0.0];
    edges.extend(bents);
    edges.push(c.length());
    edges.windows(2).map(|w| w[1] - w[0]).fold(0.0, f64::max)
}

/// The two chains carry one route (a way or a name in common).
fn same_route(c: &Chain, d: &Chain) -> bool {
    c.pieces.iter().any(|p| d.pieces.iter().any(|q| p.way == q.way || (!p.name.is_empty() && p.name == q.name)))
}

/// Where the bent line of `c` at station `s` (along its frame normal)
/// crosses the centreline of `d`, when `d` runs alongside within
/// `route::ALONG_DEG` and its fascia is within `TWIN_GAP_M` of `c`'s.
fn twin_line(c: &Chain, s: f64, d: &Chain, barrier: f64) -> Option<TwinLine> {
    let f = c.frame(s);
    let (n, t) = ([f.nx, f.nz], [f.tx, f.tz]);
    let mut best: Option<(f64, f64, usize)> = None;
    for i in 0..d.points.len().saturating_sub(1) {
        let (a, b) = (d.points[i], d.points[i + 1]);
        let e = [b[0] - a[0], b[1] - a[1]];
        // f + n u = a + e v
        let den = n[0] * (-e[1]) - n[1] * (-e[0]);
        if den.abs() < 1e-12 {
            continue;
        }
        let r = [a[0] - f.x, a[1] - f.z];
        let u = (r[0] * (-e[1]) - r[1] * (-e[0])) / den;
        let v = (n[0] * r[1] - n[1] * r[0]) / den;
        if !(0.0..=1.0).contains(&v) {
            continue;
        }
        if best.is_none_or(|x| u.abs() < x.0.abs()) {
            best = Some((u, d.cum[i] + v * (d.cum[i + 1] - d.cum[i]), i));
        }
    }
    let (u, sd, _) = best?;
    if sd <= 0.0 || sd >= d.length() {
        return None;
    }
    let g = d.frame(sd);
    if (t[0] * g.tx + t[1] * g.tz).abs() < crate::route::ALONG_DEG.to_radians().cos() {
        return None;
    }
    let (hc, hd) = (c.half_at(s) + barrier, d.half_at(sd) + barrier);
    (u.abs() - hc - hd <= TWIN_GAP_M).then_some(TwinLine { chain: d.id, u, s: sd, half: hd, carried: false })
}

/// Stage A5, for the deck chains whose A1b bents (`rooms`) leave a span
/// longer than `LONGEST_SPAN_M`: placed again by the relaxed rule
/// (`bents::room_relaxed`: freeway pads judged by the drawn street, barrier
/// medians), and where two such chains of one route run side by side
/// (`twin_line`), on shared bents: one line of columns across both decks
/// and the gap between them, placed along the lower id's chain (the
/// other's bents there are carried by its cap) - kept when every chain of
/// the pair then meets `LONGEST_SPAN_M`, else each is placed alone. A
/// chain keeps its A1b bents unless the new ones leave a shorter longest
/// span. Every other chain's bents are as A1b placed them.
fn second_tier(out: &[Chain], rooms: Vec<Vec<(f64, Room)>>, index: &Grid<(u32, u32)>, sites: &Sites, limits: &Limits) -> (Vec<Vec<(f64, Room)>>, usize) {
    let stations = |l: &[(f64, Room)]| -> Vec<f64> { l.iter().map(|x| x.0).collect() };
    let wanting: Vec<bool> = out.iter().zip(&rooms).map(|(c, r)| c.kind == ChainKind::Deck && longest_span(c, &stations(r)) > LONGEST_SPAN_M).collect();
    let mut rooms = rooms;
    let mut done = vec![false; out.len()];
    let mut relaxed = 0usize;
    let reach = BENT_FOOTPRINT_MARGIN_M + limits.barrier_base + COLUMN_DECK_GAP_M;
    let bw = limits.barrier_base;
    // the relaxed room of `c` alone (Tier 1's `free`: off lower chains, and past the edge off same-level decks).
    let alone = |c: &Chain, fixed: &[(f64, Room)]| -> Vec<(f64, Room)> {
        let r_least = column_diameter(c.class, 0.0) / 2.0;
        let free = |p: P2, outside: bool| -> bool {
            let under = |d: &Chain| d.id != c.id && d.kind != ChainKind::Tunnel && (d.level < c.level || (outside && d.level <= c.level));
            near(out, index, p, reach, &under).is_empty()
        };
        bents::place_by(c, &mut |s| bents::room_relaxed(sites, c, s, r_least, &free), fixed)
    };
    // shared bents along `m`, its twins' bents on the lines it shares with them carried by its caps.
    let shared = |m: &Chain, twins: &[&Chain]| -> Vec<(u32, Vec<(f64, Room)>)> {
        let r_least = column_diameter(m.class, 0.0) / 2.0;
        let mut room_of = |s: f64| -> Option<Room> {
            for d in twins {
                let Some(tl) = twin_line(m, s, d, bw) else { continue };
                let hm = m.half_at(s);
                let free = |p: P2, u: f64| -> bool {
                    let under_m = u.abs() <= hm + bw;
                    let under_d = (u - tl.u).abs() <= tl.half;
                    let under = |e: &Chain| e.id != m.id && e.id != d.id && e.kind != ChainKind::Tunnel && (e.level < m.level || (!under_m && !under_d && e.level <= m.level));
                    near(out, index, p, reach, &under).is_empty()
                };
                return bents::room_shared(sites, m, s, tl, r_least, &free);
            }
            let free = |p: P2, outside: bool| -> bool {
                let under = |d: &Chain| d.id != m.id && d.kind != ChainKind::Tunnel && (d.level < m.level || (outside && d.level <= m.level));
                near(out, index, p, reach, &under).is_empty()
            };
            bents::room_relaxed(sites, m, s, r_least, &free)
        };
        let master = bents::place_by(m, &mut room_of, &[]);
        let mut fixed: HashMap<u32, Vec<(f64, Room)>> = HashMap::new();
        for (s, rm) in &master {
            if let Some(tl) = rm.twin {
                let back = TwinLine { chain: m.id, u: tl.u, s: *s, half: m.half_at(*s) + bw, carried: true };
                fixed.entry(tl.chain).or_default().push((tl.s, Room { rule: rm.rule, zones: vec![], twin: Some(back) }));
            }
        }
        let mut lists: Vec<(u32, Vec<(f64, Room)>)> = vec![(m.id, master)];
        for d in twins {
            let Some(mut fx) = fixed.remove(&d.id) else { continue };
            fx.sort_by(|a, b| a.0.total_cmp(&b.0));
            // (two shared lines closer than a step: the first kept.)
            fx.dedup_by(|b, a| b.0 - a.0 < 1.0);
            lists.push((d.id, alone(d, &fx)));
        }
        lists
    };
    for c in out {
        if !wanting[c.id as usize] || done[c.id as usize] {
            continue;
        }
        let twins: Vec<&Chain> = out.iter().filter(|d| d.id != c.id && wanting[d.id as usize] && !done[d.id as usize] && d.kind == ChainKind::Deck && d.level == c.level && same_route(c, d)).collect();
        // each of the pair as the one whose line the bents follow, the lower id first.
        let mut group: Vec<&Chain> = vec![c];
        group.extend(twins.iter().copied());
        let mut accepted = false;
        for (k, m) in group.iter().enumerate() {
            if twins.is_empty() {
                break;
            }
            let others: Vec<&Chain> = group.iter().enumerate().filter(|(j, _)| *j != k).map(|(_, d)| *d).collect();
            let lists = shared(m, &others);
            if lists.len() > 1 && lists.iter().all(|(id, l)| longest_span(&out[*id as usize], &stations(l)) <= LONGEST_SPAN_M) {
                for (id, l) in lists {
                    done[id as usize] = true;
                    rooms[id as usize] = l;
                    relaxed += 1;
                }
                accepted = true;
                break;
            }
        }
        if accepted {
            continue;
        }
        // alone, by the relaxed rule.
        done[c.id as usize] = true;
        let list = alone(c, &[]);
        if longest_span(c, &stations(&list)) < longest_span(c, &stations(&rooms[c.id as usize])) {
            rooms[c.id as usize] = list;
            relaxed += 1;
        }
    }
    (rooms, relaxed)
}

/// A cap beam is this much wider (along the chain) than its column: Caltrans
/// SDC section 7.4.2.1, Eq. 7.10, Bcap >= Dc + 2 ft (reference sheet 1.3).
pub(crate) const CAP_OVER_COLUMN_M: f64 = 0.6096;

/// The column diameter of a bent between spans of superstructure depth
/// `depth` on a chain of class `class`: `COLUMN_DC_OVER_DS` x the depth,
/// never under the class's least, never over the footprint the room was
/// cleared for.
pub(crate) fn column_diameter(class: RoadClass, depth: f64) -> f64 {
    let least = if matches!(class, RoadClass::Motorway | RoadClass::Trunk) { COLUMN_MIN_MAINLINE_M } else { COLUMN_MIN_OTHER_M };
    (COLUMN_DC_OVER_DS * depth).clamp(least, 2.0 * BENT_FOOTPRINT_MARGIN_M)
}

/// The bents of a built deck chain: columns stood in each bent's room (the
/// count by the deck's width), the cap by what is under it (`over`: station
/// stretches over something crossed, where nothing may hang below the
/// soffit), and nothing where the superstructure sits on its approach fill.
fn supports(c: &Chain, chains: &[Chain], rooms: &[Room], over: &[[f64; 2]], sites: &Sites, streets: &StreetWorld, limits: &Limits) -> Vec<Bent> {
    let mut out = vec![];
    for (k, (&s, room)) in c.bents.iter().zip(rooms).enumerate() {
        let joint = (k + 1) % JOINT_SPANS == 0;
        let (d0, d1) = (c.depths[k], c.depths[k + 1]);
        // a bent carried by its twin's cap: no columns of its own.
        if let Some(t) = room.twin.filter(|t| t.carried) {
            let twin = Some(BentTwin { chain: t.chain, bent: 0, u: t.u, carried: true });
            out.push(Bent { s, rule: room.rule, cap: CapKind::Integral, columns: vec![], cap_width: 0.0, cap_depth: 0.0, joint, downspout: false, twin });
            continue;
        }
        // a shared bent's twin: its depth there (the columns carry both).
        let twin = room.twin.and_then(|t| {
            let d = &chains[t.chain as usize];
            (d.heights.len() == d.stations.len() && d.is_built()).then_some((t, d))
        });
        // (either side of the twin's bent on this line.)
        let twin_depths = twin.map_or([0.0; 2], |(t, d)| [d.depth_at(t.s - 1e-6), d.depth_at(t.s + 1e-6)]);
        let twin_depth = twin_depths[0].max(twin_depths[1]);
        let dc = column_diameter(c.class, d0.max(d1).max(twin_depth));
        let mut r = dc / 2.0;
        let own = c.half_at(s) + limits.barrier_base;
        // the section the columns stand under: this deck, or both and the gap.
        let (lo, hi) = match twin {
            Some((t, _)) => ((-own).min(t.u - t.half), own.max(t.u + t.half)),
            None => (-own, own),
        };
        let width = hi - lo;
        let places = bents::column_places(room, bents::column_count(width), width, (lo + hi) / 2.0, r, COLUMN_GAP_M);
        // (a barrier median holds no wider a column than it is.)
        r = places.iter().map(|p| p.2).fold(r, f64::min);
        let dc = 2.0 * r;
        let f = c.frame(s);
        let mut columns: Vec<Column> = places
            .into_iter()
            .map(|(u, site, _)| {
                let (x, z) = (f.x + f.nx * u, f.z + f.nz * u);
                let ground = [(0.0, 0.0), (r, 0.0), (-r, 0.0), (0.0, r), (0.0, -r)].iter().map(|&(a, b)| streets.ground(x + f.nx * a + f.tx * b, z + f.nz * a + f.tz * b)).fold(f64::INFINITY, f64::min);
                Column { u, x, z, radius: r, site: site.column_site(), street: sites.street_ref(site), ground }
            })
            .collect();
        let cap_width = dc + CAP_OVER_COLUMN_M;
        let crossed = over.iter().any(|o| s + cap_width / 2.0 >= o[0] && s - cap_width / 2.0 <= o[1]);
        let mut cap = if room.rule == BentRule::Straddle {
            CapKind::Outrigger
        } else if columns.len() >= 2 || crossed || twin.is_some() {
            CapKind::Integral
        } else {
            CapKind::TBeam
        };
        let soffit = c.y_at(s) - d0.min(d1);
        let high_ground = columns.iter().map(|col| col.ground).fold(f64::NEG_INFINITY, f64::max);
        let mut cap_depth = if cap == CapKind::TBeam { d0.max(d1).max(TBEAM_DEPTH_PER_DC * dc) } else { 0.0 };
        if cap == CapKind::TBeam && soffit - cap_depth - high_ground < FILL_CLEAR_M + dc {
            // no room under it for a T cap and a column: the cap goes into the box.
            cap = CapKind::Integral;
            cap_depth = 0.0;
        }
        match twin {
            Some((t, d)) => {
                // each column under the deck nearer it, kept off that deck's fill.
                let twin_soffit = d.y_at(t.s) - twin_depths[0].min(twin_depths[1]);
                columns.retain(|col| {
                    let on_twin = (col.u - t.u).abs() - t.half < col.u.abs() - own;
                    let sf = if on_twin { twin_soffit } else { soffit };
                    sf - col.ground >= FILL_CLEAR_M + dc
                });
            }
            None => {
                if soffit - high_ground < FILL_CLEAR_M + dc {
                    // the superstructure sits on its approach fill here.
                    columns.clear();
                }
            }
        }
        let downspout = k % DOWNSPOUT_BENTS == 1 && !columns.is_empty();
        let twin = twin.map(|(t, _)| BentTwin { chain: t.chain, bent: 0, u: t.u, carried: false });
        out.push(Bent { s, rule: room.rule, cap, columns, cap_width, cap_depth, joint, downspout, twin });
    }
    out
}

/// A pinned station of a merge may stand this far off its major's profile
/// before the vertical curves are given up for the sharp profile there (a
/// solve tolerance).
const PIN_TOL_M: f64 = 0.002;

/// A deck tied to another of its level where their roadways overlap (stage
/// A5, `merge_ties`): at the minor's stations `at.0` its profile is pinned
/// to the major's at the major's station `at.1` on the same cross-section
/// line; the major is never under what the minor needs there.
#[derive(Clone, Debug)]
pub(crate) struct Tie {
    pub minor: u32,
    pub major: u32,
    pub at: Vec<(usize, f64)>,
}

/// Which of two overlapping decks conforms to the other: the lower road class
/// (motorway, trunk, link, other), then the narrower, the shorter, the
/// higher id is the minor.
/// `d` is the major of a merged pair with `c` (`Chain::merged_with`).
pub(crate) fn is_major(d: &Chain, c: &Chain) -> bool {
    major_rank(d) > major_rank(c)
}

fn major_rank(c: &Chain) -> (u8, i64, i64, i64) {
    let class = match c.class {
        RoadClass::Motorway => 3,
        RoadClass::Trunk => 2,
        RoadClass::MotorwayLink => 1,
        RoadClass::Other => 0,
    };
    (class, (c.max_half() * 1000.0).round() as i64, (c.length() * 1000.0).round() as i64, -(c.id as i64))
}

/// Every pair of same-level decks (not landmarks) where one's deck (its
/// roadway and barriers, across each station) reaches over the other's
/// roadway, and they share a node (a merge or a split, at any angle) or
/// run within `route::ALONG_DEG` of each other with one's centreline over
/// the other's roadway somewhere (twin decks side by side): the minor's
/// stations there and the major's station its centreline projects to.
pub(crate) fn merge_ties(chains: &[Chain], barrier: f64) -> Vec<Tie> {
    let decks: Vec<&Chain> = chains.iter().filter(|c| c.kind == ChainKind::Deck && c.landmark.is_none()).collect();
    let max_half = decks.iter().map(|c| c.max_half()).fold(0.0, f64::max);
    let pad = 2.0 * (max_half + barrier) + 1.0;
    let mut grid: Grid<(u32, u32)> = Grid::default();
    for c in &decks {
        for i in 0..c.points.len().saturating_sub(1) {
            grid.insert_seg(c.points[i], c.points[i + 1], pad, (c.id, i as u32));
        }
    }
    let cos = crate::route::ALONG_DEG.to_radians().cos();
    let mut out = vec![];
    for c in &decks {
        let mut per: BTreeMap<u32, BTreeMap<usize, (f64, f64)>> = BTreeMap::new();
        for (i, &s) in c.stations.iter().enumerate() {
            let f = c.frame(s);
            let h = c.half_at(s) + barrier;
            let (l, r) = ([f.x + f.nx * h, f.z + f.nz * h], [f.x - f.nx * h, f.z - f.nz * h]);
            for &(di, si) in grid.at(f.x, f.z) {
                let d = &chains[di as usize];
                if di == c.id || d.level != c.level || major_rank(c) >= major_rank(d) {
                    continue;
                }
                let k = si as usize;
                let (a, b) = (d.points[k], d.points[k + 1]);
                let (dist, q) = seg_distance(l, r, a, b);
                let (t, _, _) = project(q, a, b);
                if dist > d.half_at(d.cum[k] + t * (d.cum[k + 1] - d.cum[k])) {
                    continue;
                }
                // the line across: where the minor's centreline point projects on the major.
                let (t, _, _) = project([f.x, f.z], a, b);
                let sd = d.cum[k] + t * (d.cum[k + 1] - d.cum[k]);
                // (a merge or a split, sharing a node, at any angle; decks side by side only along each other.)
                let g = d.frame(sd);
                let shared = c.ends.iter().any(|e| d.ends.iter().any(|x| x.node == e.node));
                if !shared && (f.tx * g.tx + f.tz * g.tz).abs() < cos {
                    continue;
                }
                let e = per.entry(di).or_default().entry(i).or_insert((f64::INFINITY, sd));
                if dist < e.0 {
                    *e = (dist, sd);
                }
            }
        }
        for (di, at) in per {
            let d = &chains[di as usize];
            // a merge (the two share a node) or one's centreline over the other's
            // roadway somewhere; decks side by side whose edges only touch are not tied.
            let shared = c.ends.iter().any(|e| d.ends.iter().any(|x| x.node == e.node));
            let over = |a: &Chain, b: &Chain| {
                a.stations.iter().any(|&s| {
                    let f = a.frame(s);
                    let (sb, dist) = b.project(f.x, f.z);
                    sb > 1e-6 && sb < b.length() - 1e-6 && dist <= b.half_at(sb)
                })
            };
            if shared || over(c, d) || over(d, c) {
                out.push(Tie { minor: c.id, major: di, at: at.into_iter().map(|(i, (_, sd))| (i, sd)).collect() });
            }
        }
    }
    out
}

/// One pass of the solve.
#[derive(Default)]
struct Solved {
    heights: HashMap<u32, Vec<f64>>,
    node_h: Vec<f64>,
    violations: BTreeMap<u32, Hold>,
    /// For a violation held as end too low: the least height each fixed end
    /// would need (the binding bound's reach back to it).
    wants: BTreeMap<(u32, usize), f64>,
    rounds: usize,
}

/// Solve every chain not held, lowest level first, round after round until
/// no shared end or profile moves; then list the chains whose fixed end the
/// profile cannot meet.
fn solve_pass(chains: &[Chain], work: &[Work], order: &[u32], held: &BTreeMap<u32, Hold>, node_kind: &[NodeKind], limits: &Limits, ties: &[Tie]) -> Solved {
    let mut heights: HashMap<u32, Vec<f64>> = HashMap::new();
    // each chain's ties as the minor (pinned to its major) and as the major.
    let mut as_minor: HashMap<u32, Vec<&Tie>> = HashMap::new();
    let mut as_major: HashMap<u32, Vec<&Tie>> = HashMap::new();
    for t in ties {
        as_minor.entry(t.minor).or_default().push(t);
        as_major.entry(t.major).or_default().push(t);
    }
    // a minor's own lowest profile (its bounds and fixed ends, no pins), for its majors.
    let mut envs: HashMap<u32, Vec<f64>> = HashMap::new();
    let mut node_h: Vec<f64> = node_kind
        .iter()
        .map(|k| match k {
            NodeKind::Paved(h) | NodeKind::Landmark(h) => *h,
            _ => f64::NEG_INFINITY,
        })
        .collect();
    let active: Vec<u32> = order.iter().copied().filter(|id| !held.contains_key(id)).collect();
    let mut violations: BTreeMap<u32, Hold> = BTreeMap::new();
    let mut wants: BTreeMap<(u32, usize), f64> = BTreeMap::new();
    let mut moving: HashSet<u32> = HashSet::new();
    let mut rounds = 0;
    for _ in 0..ROUNDS {
        rounds += 1;
        moving.clear();
        violations.clear();
        wants.clear();
        for &id in &active {
            let c = &chains[id as usize];
            let w = &work[id as usize];
            let n = c.stations.len();
            let t_end = w.tau[n - 1];
            let fixed = |e: usize, node_h: &[f64]| -> Option<f64> {
                match node_kind[c.ends[e].node as usize] {
                    NodeKind::Paved(_) | NodeKind::Landmark(_) => c.ends[e].fixed,
                    NodeKind::Shared => Some(node_h[c.ends[e].node as usize]).filter(|h| h.is_finite()),
                    NodeKind::Free => None,
                }
            };
            let y = if c.kind == ChainKind::Tunnel {
                let hi: Vec<f64> = w.ground.iter().map(|g| g - MIN_COVER_M - BORE_CLEAR_M).collect();
                let ends = Ends { a: fixed(0, &node_h), b: fixed(1, &node_h) };
                let feasible = match (ends.a, ends.b) {
                    (Some(a), Some(b)) => (a - b).abs() <= t_end + END_TOL_M,
                    _ => true,
                };
                if !feasible {
                    violations.insert(id, Hold::Grade);
                }
                profile::tunnel(&w.tau, &hi, ends, w.kappa, feasible)
            } else {
                // the lower bound: on the ground (or above the water), and over each crossing.
                let mut lo: Vec<f64> = w.floor.clone();
                let mut src: Vec<Option<usize>> = vec![None; n];
                for (k, sp) in w.cross.iter().enumerate() {
                    let Some(clear) = limits.clearance(sp.freeway) else { continue };
                    for &(i, under) in &sp.at {
                        let y_under = match under {
                            Under::Street(y) => y,
                            Under::Chain(lo, hi) => {
                                let d = sp.chain.unwrap();
                                let dc = &chains[d as usize];
                                if dc.kind == ChainKind::Landmark {
                                    interp_max(&dc.stations, &dc.heights, lo, hi)
                                } else if held.contains_key(&d) {
                                    continue;
                                } else {
                                    match heights.get(&d) {
                                        Some(h) => interp_max(&dc.stations, h, lo, hi) + crown_over(dc, lo, hi),
                                        None => continue,
                                    }
                                }
                            }
                        };
                        let v = y_under + clear + c.depth_at(c.stations[i]);
                        if v > lo[i] {
                            lo[i] = v;
                            src[i] = Some(k);
                        }
                    }
                }
                // stage A5's merges: as a major, never under what a minor needs
                // where they overlap; as a minor, pinned to its major there.
                let own = lo.clone();
                for t in as_major.get(&id).into_iter().flatten() {
                    if held.contains_key(&t.minor) {
                        continue;
                    }
                    let Some(env) = envs.get(&t.minor) else { continue };
                    for &(i, sj) in &t.at {
                        let j = match c.stations.binary_search_by(|v| v.total_cmp(&sj)) {
                            Ok(j) => j.min(n - 1),
                            Err(j) => j.clamp(1, n - 1) - 1,
                        };
                        for k in [j, (j + 1).min(n - 1)] {
                            if env[i] > lo[k] {
                                lo[k] = env[i];
                                src[k] = None;
                            }
                        }
                    }
                }
                let mut pins: Vec<(usize, f64)> = vec![];
                for t in as_minor.get(&id).into_iter().flatten() {
                    if held.contains_key(&t.major) {
                        continue;
                    }
                    let Some(yj) = heights.get(&t.major) else { continue };
                    let dj = &chains[t.major as usize];
                    for &(i, sj) in &t.at {
                        let pin = interp(&dj.stations, yj, sj);
                        pins.push((i, pin));
                        if pin > lo[i] {
                            lo[i] = pin;
                            src[i] = None;
                        }
                    }
                }
                let z = profile::lowest_above(&w.tau, &lo);
                // a shared end rises to what this chain needs there.
                for e in 0..2 {
                    let node = c.ends[e].node as usize;
                    if node_kind[node] != NodeKind::Shared {
                        continue;
                    }
                    let z_e = if e == 0 { z[0] } else { z[n - 1] };
                    let other = fixed(1 - e, &node_h).map_or(f64::NEG_INFINITY, |h| h - t_end);
                    let r = z_e.max(other);
                    if r > node_h[node] + SETTLED_M {
                        node_h[node] = r;
                        moving.insert(id);
                    }
                }
                let ends = Ends { a: fixed(0, &node_h), b: fixed(1, &node_h) };
                // a fixed end (paved, landmark) the profile cannot come down to.
                let mut feasible = true;
                for e in 0..2 {
                    if !matches!(node_kind[c.ends[e].node as usize], NodeKind::Paved(_) | NodeKind::Landmark(_)) {
                        continue;
                    }
                    let h = fixed(e, &node_h).unwrap();
                    let te = if e == 0 { 0.0 } else { t_end };
                    // the binding bound at this end, and the other end's cone.
                    let (mut best, mut at) = (f64::NEG_INFINITY, None);
                    for i in 0..n {
                        let v = lo[i] - (w.tau[i] - te).abs();
                        if v > best {
                            best = v;
                            at = Some(i);
                        }
                    }
                    let other = fixed(1 - e, &node_h).map_or(f64::NEG_INFINITY, |o| o - t_end);
                    if best.max(other) > h + END_TOL_M {
                        feasible = false;
                        let why = if other >= best {
                            Hold::Grade
                        } else {
                            match at.and_then(|i| src[i]) {
                                Some(k) => {
                                    wants.insert((id, e), best);
                                    Hold::EndTooLow { over: w.cross[k].over.clone() }
                                }
                                None => Hold::Grade,
                            }
                        };
                        violations.entry(id).or_insert(why);
                    }
                }
                if as_major.contains_key(&id) || !pins.is_empty() {
                    let mut env = profile::lowest_above(&w.tau, &own);
                    profile::lower_cones(&mut env, &w.tau, ends);
                    envs.insert(id, env);
                }
                let y = profile::deck(&w.tau, &lo, ends, w.kappa, feasible);
                // (the vertical curves never lift a pinned station off its major:
                // held under each pin's rising cone, which leaves the curves
                // everywhere else; the sharp profile if even that misses one.)
                if pins.iter().any(|&(i, p)| (y[i] - p).abs() > PIN_TOL_M) {
                    let mut held = y;
                    for (j, v) in held.iter_mut().enumerate() {
                        for &(i, p) in &pins {
                            *v = v.min(p + (w.tau[j] - w.tau[i]).abs());
                        }
                    }
                    let ok = held.iter().zip(&lo).all(|(v, l)| *v >= l - 1e-9) && pins.iter().all(|&(i, p)| (held[i] - p).abs() <= PIN_TOL_M) && profile::lipschitz_ok(&held, &w.tau);
                    if ok { held } else { profile::deck_sharp(&w.tau, &lo, ends) }
                } else {
                    y
                }
            };
            let changed = heights.get(&id).is_none_or(|old| old.len() != y.len() || old.iter().zip(&y).any(|(a, b)| (a - b).abs() > SETTLED_M));
            if changed {
                moving.insert(id);
            }
            heights.insert(id, y);
        }
        if moving.is_empty() {
            break;
        }
    }
    // still moving after every round: the shared ends and the grade cannot settle.
    if !moving.is_empty() && rounds == ROUNDS {
        for id in moving {
            violations.entry(id).or_insert(Hold::Grade);
        }
    }
    Solved { heights, node_h, violations, wants, rounds }
}
