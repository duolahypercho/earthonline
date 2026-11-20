//! A deck's cross-section: the roadway and its paint lines from the street
//! segment's own section (lane layout, widths, shoulders), the Type 60
//! barrier on each edge, and the box girder under it (deck overhang, sloped
//! exterior webs, flat soffit).
//!
//! Lateral offsets `u` run along the chain's frame normal (+n, the left of
//! increasing station); heights are relative to the deck top at the
//! centreline.

use anyhow::{Context, Result};
use eo_data::standards::CaStandards;
use eo_street::city::Segment;

use crate::{Piece, RoadClass};

/// The traffic face of a Caltrans single-slope barrier leans back this far
/// from vertical (Type 60 and Type 732: 9.1 degrees; reference sheet 1.4,
/// Caltrans DRISI crash-test reports, Y). The back of a bridge barrier is
/// vertical (Type 732), flush with the deck's fascia.
pub(crate) const BARRIER_FACE_DEG: f64 = 9.1;

/// The deck slab's overhang is at least 8 in (0.203 m) thick at its edge and
/// 12 in (0.305 m) at the exterior web of a cast-in-place box (Caltrans
/// Bridge Design Memo 9.4 section 9.4.4; reference sheet 1.3, Y).
pub(crate) const EDGE_THICK_M: f64 = 0.203;
pub(crate) const WEB_TOP_THICK_M: f64 = 0.305;

/// Deck overhang, from the fascia to the top of the exterior web. INFERRED,
/// structure-only: the reference sheet found no rule (the Presidio
/// Viaduct's 14 ft overhang rides on steel fins); 6 ft is within the
/// girder spacings of BDM 9.4's slab table (9.5-15 ft) halved.
pub(crate) const OVERHANG_M: f64 = 1.83;

/// The exterior webs slope in by this much per metre of height (Caltrans
/// box girders have sloping exterior girders, Bridge Design Details 8.2,
/// sheet 1.3; the batter is not given there). INFERRED, structure-only.
pub(crate) const WEB_BATTER: f64 = 0.35;

/// A drip groove under the overhang, this far in from the fascia, this wide
/// and deep. INFERRED, structure-only (a cast drip line; sheet 5.10 "drip line").
pub(crate) const DRIP_IN_M: f64 = 0.075;
pub(crate) const DRIP_W_M: f64 = 0.025;
pub(crate) const DRIP_D_M: f64 = 0.02;

/// Where two pieces of a chain differ in width, the deck tapers between them
/// over this length. INFERRED, structure-only (a lane add or drop taper).
pub(crate) const WIDTH_TAPER_M: f64 = 30.0;

/// The table's values the meshes read.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Dims {
    /// `concrete-barrier-type-60m-height` and `-base-width`; the top width
    /// follows from the 9.1-degree traffic face and a vertical back.
    pub barrier_h: f64,
    pub barrier_base: f64,
    pub barrier_top: f64,
    /// `line-width-state-highway`; `broken-line-pattern-ge45mph` line and gap.
    pub line_w: f64,
    pub dash: f64,
    pub gap: f64,
    /// Fallback section (a segment without a lane layout):
    /// `travel-lane-width-state-highway-min`, `ramp-shoulder-width` (left,
    /// right), `shoulder-width-conventional-highway` (leftCurbedMedianUrbanLe45, right).
    pub lane_state: f64,
    pub ramp_shoulder: [f64; 2],
    pub conv_shoulder: [f64; 2],
}

impl Dims {
    pub fn read(ca: &CaStandards) -> Result<Self> {
        let barrier_h = ca.metric("concrete-barrier-type-60m-height").context("eo-structures: barrier height")?;
        let barrier_base = ca.metric("concrete-barrier-type-60m-base-width").context("eo-structures: barrier base width")?;
        let barrier_top = (barrier_base - barrier_h * BARRIER_FACE_DEG.to_radians().tan()).max(0.05);
        Ok(Self {
            barrier_h,
            barrier_base,
            barrier_top,
            line_w: ca.metric("line-width-state-highway").context("eo-structures: line width")?,
            dash: ca.metric_key("broken-line-pattern-ge45mph", "line").context("eo-structures: broken line")?,
            gap: ca.metric_key("broken-line-pattern-ge45mph", "gap").context("eo-structures: broken line gap")?,
            lane_state: ca.metric("travel-lane-width-state-highway-min").context("eo-structures: state lane width")?,
            ramp_shoulder: [
                ca.metric_key("ramp-shoulder-width", "left").context("eo-structures: ramp shoulder")?,
                ca.metric_key("ramp-shoulder-width", "right").context("eo-structures: ramp shoulder")?,
            ],
            conv_shoulder: [
                ca.metric_key("shoulder-width-conventional-highway", "leftCurbedMedianUrbanLe45").context("eo-structures: shoulder")?,
                ca.metric_key("shoulder-width-conventional-highway", "right").context("eo-structures: shoulder")?,
            ],
        })
    }
}

/// One paint line along a piece.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Line {
    /// Centre offset and width.
    pub u: f64,
    pub w: f64,
    /// Broken (`broken-line-pattern-ge45mph`), else solid.
    pub dashed: bool,
    pub yellow: bool,
    /// An edge line of the travelled way (the far level keeps these only).
    pub edge: bool,
}

/// A piece's roadway: half width and paint lines, in the chain's frame.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Section {
    pub half: f64,
    pub lines: Vec<Line>,
    /// Built from the fallback rule (no lane layout).
    pub fallback: bool,
}

/// The section of piece `p` (segment `seg`) in its chain's frame. `class_lane`
/// is the class-model lane width of its way (`CaStandards::class_lane_width`),
/// `scale` the ground scale the street contract's widths carry.
pub(crate) fn section(seg: &Segment, p: &Piece, class: RoadClass, dims: &Dims, class_lane: f64, scale: f64) -> Section {
    let flip = if p.reversed { -1.0 } else { 1.0 };
    // (direction along increasing station, centre, width) of each motor lane.
    let mut lanes: Vec<(i8, f64, f64)> = seg
        .lane_layout
        .as_ref()
        .map(|l| l.iter().filter(|x| x.motor && x.width > 0.1).map(|x| ((x.dir as f64 * flip) as i8, -x.lateral * flip, x.width)).collect())
        .unwrap_or_default();
    // a layout of one shared lane both ways (service roads, garage ramps):
    // the segment's own section, unmarked.
    if lanes.is_empty() && seg.lane_layout.as_ref().is_some_and(|l| !l.is_empty() && l.iter().all(|x| x.role == "shared-bidirectional")) {
        return Section { half: p.half, lines: vec![], fallback: false };
    }
    let mut fallback = false;
    if lanes.is_empty() {
        fallback = true;
        let n = seg.lanes.unwrap_or(1.0).round().max(1.0) as usize;
        let freeway = matches!(class, RoadClass::Motorway | RoadClass::MotorwayLink | RoadClass::Trunk);
        let w = if freeway { dims.lane_state * scale } else { class_lane * scale };
        let [sl, sr] = if p.highway.ends_with("_link") { dims.ramp_shoulder } else { dims.conv_shoulder };
        let dir = if seg.is_oneway() { if seg.oneway == "decreasing" { -1 } else { 1 } } else { 0 };
        let dir = (dir as f64 * flip) as i8;
        // the shoulders by the direction of travel: its left one on +n when it runs +s.
        let (s_plus, s_minus) = if dir < 0 { (sr * scale, sl * scale) } else { (sl * scale, sr * scale) };
        let travel = n as f64 * w;
        // the lanes centred between the shoulders, inside the piece's own half width.
        let mid = ((p.half - s_plus) + (-p.half + s_minus)) / 2.0;
        let right = mid - travel / 2.0;
        let left = right + travel;
        for k in 0..n {
            let u = right + w * (k as f64 + 0.5);
            // a two-way fallback: the left half one way, the right half the other.
            let d = if dir != 0 { dir } else if u > (left + right) / 2.0 { -1 } else { 1 };
            lanes.push((d, u, w));
        }
    }
    lanes.sort_by(|a, b| a.1.total_cmp(&b.1));
    let left_edge = lanes.iter().map(|l| l.1 + l.2 / 2.0).fold(f64::MIN, f64::max).min(p.half);
    let right_edge = lanes.iter().map(|l| l.1 - l.2 / 2.0).fold(f64::MAX, f64::min).max(-p.half);
    let lw = dims.line_w;
    let mut lines = vec![];
    let one_way = lanes.iter().all(|l| l.0 == lanes[0].0) && lanes[0].0 != 0;
    // CA MUTCD line colours: yellow on the left edge of a one-way roadway
    // (left of travel), white on the right edge and between lanes of one
    // direction; a double yellow between opposite directions.
    let left_of_travel_is_plus = lanes[0].0 > 0;
    lines.push(Line { u: left_edge - lw / 2.0, w: lw, dashed: false, yellow: one_way && left_of_travel_is_plus, edge: true });
    lines.push(Line { u: right_edge + lw / 2.0, w: lw, dashed: false, yellow: one_way && !left_of_travel_is_plus, edge: true });
    for w in lanes.windows(2) {
        let b = ((w[0].1 + w[0].2 / 2.0) + (w[1].1 - w[1].2 / 2.0)) / 2.0;
        if w[0].0 == w[1].0 {
            lines.push(Line { u: b, w: lw, dashed: true, yellow: false, edge: false });
        } else {
            for sgn in [-1.0, 1.0] {
                lines.push(Line { u: b + sgn * lw, w: lw, dashed: false, yellow: true, edge: false });
            }
        }
    }
    Section { half: p.half, lines, fallback }
}

/// The box girder at one station: offsets of its outline and its depth.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Girder {
    /// The fascia (outer face of the barrier and the slab edge).
    pub fascia: f64,
    /// The top of the exterior web (where the overhang meets it).
    pub web_top: f64,
    /// The soffit's half width (the bottom of the sloped webs).
    pub soffit: f64,
    /// Depth from the deck top to the soffit.
    pub depth: f64,
}

/// The girder under a roadway `half` wide with barriers `barrier` wide, `depth` deep.
pub(crate) fn girder(half: f64, barrier: f64, depth: f64) -> Girder {
    let fascia = half + barrier;
    let depth = depth.max(WEB_TOP_THICK_M + 0.15);
    let web_top = (fascia - OVERHANG_M).max(0.55 * fascia);
    let soffit = (web_top - (depth - WEB_TOP_THICK_M) * WEB_BATTER).max(0.35 * fascia);
    Girder { fascia, web_top, soffit, depth }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lanes_and_lines_follow_the_layout() {
        let ca = CaStandards::load(&eo_data::DataPaths::discover().unwrap()).unwrap();
        let dims = Dims::read(&ca).unwrap();
        assert!((dims.barrier_top - (dims.barrier_base - dims.barrier_h * 9.1f64.to_radians().tan())).abs() < 1e-12);
        let mut seg = Segment {
            key: None,
            id: "s".into(),
            street_name: String::new(),
            street_id: None,
            highway: "motorway".into(),
            width: 11.0,
            lanes: Some(3.0),
            oneway: "increasing".into(),
            sidewalk_w: None,
            sidewalk_left: None,
            sidewalk_right: None,
            points: vec![[0.0, 0.0], [10.0, 0.0]],
            separated: true,
            lane_layout: Some(
                [-3.66, 0.0, 3.66].iter().map(|&l| eo_street::city::Lane { dir: 1, role: "through".into(), width: 3.66, lateral: l, motor: true }).collect(),
            ),
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
            pedestrian_eligible: false,
        };
        let piece = Piece { segment: 0, key: "k".into(), way: "1".into(), name: String::new(), highway: "motorway".into(), half: 5.49, s0: 0.0, s1: 10.0, reversed: false };
        let s = section(&seg, &piece, RoadClass::Motorway, &dims, 3.05, 1.0);
        assert!(!s.fallback);
        let edges: Vec<&Line> = s.lines.iter().filter(|l| l.edge).collect();
        assert_eq!(edges.len(), 2);
        // one-way, travelling +s: the left edge (+n) yellow, the right white.
        assert!(edges.iter().any(|l| l.u > 0.0 && l.yellow) && edges.iter().any(|l| l.u < 0.0 && !l.yellow));
        assert_eq!(s.lines.iter().filter(|l| l.dashed).count(), 2);
        // reversed along the chain: mirrored, the yellow edge now on -n.
        let rev = section(&seg, &Piece { reversed: true, ..piece.clone() }, RoadClass::Motorway, &dims, 3.05, 1.0);
        assert!(rev.lines.iter().filter(|l| l.edge).any(|l| l.u < 0.0 && l.yellow));
        seg.lane_layout = None;
        let fb = section(&seg, &piece, RoadClass::Motorway, &dims, 3.05, 1.25);
        assert!(fb.fallback && fb.lines.iter().filter(|l| l.dashed).count() == 2);
        let g = girder(5.49, dims.barrier_base, 1.3);
        assert!(g.fascia > g.web_top && g.web_top > g.soffit && g.soffit > 0.0);
    }
}
