//! Traffic signal poles, mast arms and heads, and pedestrian signal heads, at
//! every signalised junction of the drawn street plan. A port of
//! `planTrafficSignalHeads` (traffic-signal-heads.js: far-side pole search on
//! the continuing leg's kerb, the straight-ahead half-plane search, the ramp
//! waiver, the corner-pole preference off throughways, split legs, the
//! near-side head on an existing pole or a new pedestal, the near-side arm,
//! the corner mast merge) and of `planPedestrianCrossings` /
//! `planPedestrianHeads` (traffic-pedestrian-heads.js), with the pole standing
//! rules of `createPoleStandingRules` against this crate's roadway index.
//! Dimensions are the JS's `SIGNAL_HARDWARE`, `NEAR_SIDE`, `CORNER_MERGE` and
//! `PED_HEAD` (exported), the 40 ft minimum view distance `caMetric
//! ('signal-face-longitudinal-position', 'min')`.
//!
//! THE LANES. The JS solves against the traffic graph's lane edges and the
//! signal plan's control records (`createTrafficGraphContract`,
//! `buildSignalControlPlan`), which the native build does not have. Here a
//! leg is one approach of the drawn junction that traffic arrives by; its
//! lanes are the street contract's arriving lanes (`laneLayout`: direction,
//! lateral offset, width) on the drawn ribbon, else the arriving half of the
//! carriageway split into the contract's lane count; each lane's stop point is
//! on the approach's limit line (`paintReach`). That is the difference that
//! moves heads relative to the JS (see the oracle test).

use crate::roadway::seg_distance;
use crate::{Live, Model, Prop, PropsWorld, station_at};
use eo_street::plan::{Approach, Node};

type P2 = [f64; 2];
const DEG: f64 = std::f64::consts::PI / 180.0;
const NEAR_RADIUS_M: f64 = 48.0;
const POLE_COLLAR_BASE_RADIUS_M: f64 = 0.26;

#[derive(Clone, Copy)]
struct Lane {
    stop: P2,
    along: f64,
    lat: f64,
    half_width: f64,
}

#[derive(Clone, Debug)]
pub struct Pole {
    pub id: String,
    pub leg: i32,
    pub kind_mast: bool,
    pub role: u8, // 0 far, 1 near, 2 ped
    pub x: f64,
    pub z: f64,
    pub base_y: f64,
    pub height: f64,
    pub kerb: f64,
    pub reserved: bool,
}

#[derive(Clone, Debug)]
pub struct Arm {
    pub pole: String,
    pub x: f64,
    pub z: f64,
    pub y: f64,
    pub length: f64,
    pub yaw: f64,
    pub near_arm: bool,
}

#[derive(Clone, Debug)]
pub struct Head {
    pub pole: String,
    pub mount: u8, // 0 pole, 1 mast, 2 near
    pub near_arm: bool,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub yaw: f64,
    pub bearing: f64,
}

#[derive(Clone, Debug)]
pub struct PedHead {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub yaw: f64,
    pub bearing: f64,
    pub length: f64,
}

#[derive(Default, Debug)]
pub struct NodePlan {
    pub poles: Vec<Pole>,
    pub arms: Vec<Arm>,
    pub heads: Vec<Head>,
    pub ped_heads: Vec<PedHead>,
    pub unplaced_legs: usize,
}

struct H {
    kerb_setback: f64,
    max_kerb_setback: f64,
    lane_margin: f64,
    mast_pole_height: f64,
    mast_arm_height: f64,
    mast_head_centre: f64,
    mast_max_reach: f64,
    mast_heads_max: usize,
    near_mast_head_centre: f64,
    pedestal_pole_height: f64,
    pedestal_head_centre: f64,
    pole_head_lateral: f64,
    head_half_height: f64,
    max_view_angle: f64,
    max_view_distance: f64,
    min_view_distance: f64,
    near_max_ahead: f64,
    near_max_kerb_offset: f64,
    near_max_view_angle: f64,
    near_max_arm_reach: f64,
    merge_radius: f64,
    merge_crossing_radius: f64,
    merge_max_across: f64,
    merge_min_arm: f64,
    merge_min_arm_sep_deg: f64,
    merge_band: f64,
}

impl H {
    fn new(w: &PropsWorld) -> Self {
        let s = |k: &str| w.js.num("signalHardware", k);
        let n = |k: &str| w.js.num("nearSide", k);
        let m = |k: &str| w.js.num("cornerMerge", k);
        H {
            kerb_setback: s("kerbSetbackM"),
            max_kerb_setback: s("maxKerbSetbackM"),
            lane_margin: s("laneMarginM"),
            mast_pole_height: s("mastPoleHeightM"),
            mast_arm_height: s("mastArmHeightM"),
            mast_head_centre: s("mastHeadCentreM"),
            mast_max_reach: s("mastMaxReachM"),
            mast_heads_max: s("mastHeadsMax") as usize,
            near_mast_head_centre: s("nearMastHeadCentreM"),
            pedestal_pole_height: s("pedestalPoleHeightM"),
            pedestal_head_centre: s("pedestalHeadCentreM"),
            pole_head_lateral: s("poleHeadLateralM"),
            head_half_height: s("headHalfHeightM"),
            max_view_angle: s("maxViewAngleDeg"),
            max_view_distance: s("maxViewDistanceM"),
            // caMetric('signal-face-longitudinal-position', 'min'), read here from the standards table.
            min_view_distance: w.std.signal_view_min,
            near_max_ahead: n("maxAheadM"),
            near_max_kerb_offset: n("maxKerbOffsetM"),
            near_max_view_angle: n("maxViewAngleDeg"),
            near_max_arm_reach: n("maxArmReachM"),
            merge_radius: m("radiusM"),
            merge_crossing_radius: m("crossingRadiusM"),
            merge_max_across: m("maxAcrossM"),
            merge_min_arm: m("minArmM"),
            merge_min_arm_sep_deg: m("minArmSeparationDeg"),
            merge_band: m("bandM"),
        }
    }
}

fn dist_to_polyline(x: f64, z: f64, pts: &[P2]) -> f64 {
    let mut best = f64::INFINITY;
    for i in 0..pts.len().saturating_sub(1) {
        best = best.min(seg_distance(x, z, pts[i], pts[i + 1]));
    }
    best
}

fn polyline_near(pts: &[P2], cx: f64, cz: f64, r: f64) -> bool {
    pts.iter().any(|p| (p[0] - cx).abs() <= r && (p[1] - cz).abs() <= r) || dist_to_polyline(cx, cz, pts) <= r
}

/// The arriving lanes of one approach (see the module comment).
fn approach_lanes(w: &PropsWorld, app: &Approach) -> Vec<(P2, f64)> {
    let e = &w.street.plan.entries[app.entry];
    let seg = w.segment_of(e);
    let reach = app.paint_reach.max(app.trim);
    let s = if app.at_start { reach } else { e.length - reach };
    let f = station_at(e, s, false);
    // Arriving traffic runs toward the node: decreasing arc length at the start, increasing at the end.
    let want: i8 = if app.at_start { -1 } else { 1 };
    let mut out = vec![];
    if let Some(ll) = &seg.lane_layout {
        for l in ll.iter().filter(|l| l.motor && l.dir == want && !matches!(l.role.as_str(), "parking" | "bike" | "cycle" | "bus-stop")) {
            // +lateralOffset is toward the left of increasing travel, i.e. -n.
            out.push(([f.x - f.nx * l.lateral, f.z - f.nz * l.lateral], l.width.max(2.4)));
        }
        if !out.is_empty() || ll.iter().any(|l| l.motor) {
            return out;
        }
    }
    if app.oneway && !app.flows_toward {
        return out;
    }
    let total = seg.lanes.unwrap_or(if app.oneway { 1.0 } else { 2.0 }).max(1.0).round() as usize;
    let arriving = if app.oneway { total } else { total.div_ceil(2) };
    let width = if app.oneway { 2.0 * e.half / total as f64 } else { e.half / arriving as f64 };
    // Driver's right of the travel direction d is (-d.z, d.x); d = -u.
    let d = [-app.u[0], -app.u[1]];
    let right = [-d[1], d[0]];
    for k in 0..arriving {
        let off = if app.oneway { -e.half + width * (k as f64 + 0.5) } else { width * (k as f64 + 0.5) };
        let (rx, rz) = if app.oneway { (f.nx, f.nz) } else { (right[0], right[1]) };
        out.push(([f.x + rx * off, f.z + rz * off], width));
    }
    out
}

/// Lane centrelines near the node (both directions), for the lane margin rule.
fn node_lane_lines(w: &PropsWorld, n: &Node) -> Vec<(Vec<P2>, f64)> {
    let mut out = vec![];
    for app in &n.approaches {
        let e = &w.street.plan.entries[app.entry];
        let seg = w.segment_of(e);
        let lanes: Vec<(f64, f64)> = match &seg.lane_layout {
            Some(ll) if ll.iter().any(|l| l.motor) => ll.iter().filter(|l| l.motor).map(|l| (-l.lateral, l.width)).collect(),
            _ => {
                let total = seg.lanes.unwrap_or(2.0).max(1.0).round() as usize;
                let wd = 2.0 * e.half / total as f64;
                (0..total).map(|k| (-e.half + wd * (k as f64 + 0.5), wd)).collect()
            }
        };
        let s0 = if app.at_start { 0.0 } else { (e.length - 60.0).max(0.0) };
        let s1 = if app.at_start { 60.0f64.min(e.length) } else { e.length };
        for (u, wd) in lanes {
            let mut pts = vec![];
            let mut s = s0;
            while s <= s1 + 1e-6 {
                let f = station_at(e, s, false);
                pts.push([f.x + f.nx * u, f.z + f.nz * u]);
                s += 3.0;
            }
            if pts.len() > 1 {
                out.push((pts, wd));
            }
        }
    }
    out
}

struct Rules<'a> {
    w: &'a PropsWorld,
    segs: Vec<(&'a [P2], f64)>,
    lanes: &'a [(Vec<P2>, f64)],
    ramp_strict: bool,
    h: &'a H,
}

impl Rules<'_> {
    fn carriageway_clearance(&self, x: f64, z: f64) -> f64 {
        self.segs.iter().map(|(p, half)| dist_to_polyline(x, z, p) - half).fold(f64::INFINITY, f64::min)
    }
    fn lane_shortfall(&self, x: f64, z: f64) -> f64 {
        self.lanes.iter().map(|(p, wd)| wd / 2.0 + self.h.lane_margin - dist_to_polyline(x, z, p)).fold(f64::NEG_INFINITY, f64::max)
    }
    fn standable(&self, p: P2, poles: &[Pole]) -> Option<f64> {
        let kerb = self.carriageway_clearance(p[0], p[1]);
        if !(kerb >= self.h.kerb_setback && kerb <= self.h.max_kerb_setback) {
            return None;
        }
        if self.w.roadway.hit(p[0], p[1], POLE_COLLAR_BASE_RADIUS_M).is_some() {
            return None;
        }
        if self.ramp_strict && self.w.roadway.ramp_hit(p[0], p[1], POLE_COLLAR_BASE_RADIUS_M) {
            return None;
        }
        if self.lane_shortfall(p[0], p[1]) > 0.0 {
            return None;
        }
        if poles.iter().any(|q| (q.x - p[0]).hypot(q.z - p[1]) < 1.4) {
            return None;
        }
        Some(kerb)
    }
}

#[derive(Clone)]
struct Layout {
    toward: f64,
    heads: Vec<(u8, f64, f64)>, // (mount, lat, height)
    reach: f64,
}

#[derive(Clone)]
struct Cand {
    along: f64,
    lat: f64,
    p: P2,
    kerb: f64,
    layout: Layout,
    worst: f64,
    mast: bool,
    cost: f64,
}

fn circular_mean(a: &[f64]) -> f64 {
    let (sx, sz) = a.iter().fold((0.0, 0.0), |(x, z), v| (x + v.cos(), z + v.sin()));
    sz.atan2(sx)
}

/// Plan one signalised junction and append its props.
pub fn plan_node(w: &PropsWorld, ni: usize, out: &mut Vec<Prop>) {
    let plan = plan_node_parts(w, ni);
    let c = w.street.plan.nodes[ni].position;
    for p in &plan.poles {
        out.push(Prop { model: Model::SignalPole, pos: [p.x, p.base_y, p.z], yaw: 0.0, scale: [1.0, p.height as f32, 1.0], variant: p.kind_mast as u16, live: Live::None });
    }
    for a in &plan.arms {
        out.push(Prop { model: Model::MastArm, pos: [a.x, a.y, a.z], yaw: a.yaw, scale: [a.length as f32, 1.0, 1.0], variant: 0, live: Live::None });
    }
    for h in &plan.heads {
        out.push(Prop { model: Model::SignalHead, pos: [h.x, h.y, h.z], yaw: h.yaw, scale: [1.0; 3], variant: h.mount as u16, live: Live::Vehicle { node: c, bearing: h.bearing } });
    }
    for h in &plan.ped_heads {
        out.push(Prop { model: Model::PedHead, pos: [h.x, h.y, h.z], yaw: h.yaw, scale: [1.0; 3], variant: 0, live: Live::Pedestrian { node: c, bearing: h.bearing, length_m: h.length } });
    }
}

/// A signal pole the clearance dropped or cut down (`plan_node_clear`):
/// where it stood, and the height above which nothing it held still stands
/// (minus infinity when dropped, the pedestal's top when cut down).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PoleChange {
    pub x: f64,
    pub z: f64,
    pub top_before: f64,
    pub top_after: f64,
}

/// One signalised junction's props kept clear of the grade-separated roads.
#[derive(Clone, Debug, Default)]
pub struct ClearedNode {
    pub props: Vec<Prop>,
    pub changes: Vec<PoleChange>,
    pub stats: crate::clearance::ClearStats,
}

/// `plan_node` kept clear of the grade-separated roads `s` (`--structures`,
/// stage A6): each pole's assembly (its arms, the heads on it, the
/// pedestrian heads mounted on it) is kept whole when every part is clear;
/// a mast that is not is cut down to a pedestal (`pedestalPoleHeightM`)
/// with only its pole-mounted heads, at the pedestal's head height
/// (`pedestalHeadCentreM`), when that is clear; else the assembly is
/// dropped. A pedestrian head on no pole stands or goes alone. The props
/// come in `plan_node`'s order.
pub fn plan_node_clear(w: &PropsWorld, s: &eo_structures::StructureWorld, ni: usize) -> ClearedNode {
    use crate::clearance::{Extent, all_clear, extents};
    let h = H::new(w);
    let d = w.clear_dims();
    let mut np = plan_node_parts(w, ni);
    let mut out = ClearedNode::default();
    // each pedestrian head's pole: the nearest within reach of its mount.
    let ped_pole: Vec<Option<usize>> = np
        .ped_heads
        .iter()
        .map(|p| np.poles.iter().enumerate().map(|(i, q)| (i, (q.x - p.x).hypot(q.z - p.z))).filter(|x| x.1 <= 0.9).min_by(|a, b| a.1.total_cmp(&b.1)).map(|x| x.0))
        .collect();
    let c = w.street.plan.nodes[ni].position;
    let pole_prop = |p: &Pole| Prop { model: Model::SignalPole, pos: [p.x, p.base_y, p.z], yaw: 0.0, scale: [1.0, p.height as f32, 1.0], variant: p.kind_mast as u16, live: Live::None };
    let arm_prop = |a: &Arm| Prop { model: Model::MastArm, pos: [a.x, a.y, a.z], yaw: a.yaw, scale: [a.length as f32, 1.0, 1.0], variant: 0, live: Live::None };
    let head_prop = |hd: &Head| Prop { model: Model::SignalHead, pos: [hd.x, hd.y, hd.z], yaw: hd.yaw, scale: [1.0; 3], variant: hd.mount as u16, live: Live::Vehicle { node: c, bearing: hd.bearing } };
    let ped_prop = |hd: &PedHead| Prop { model: Model::PedHead, pos: [hd.x, hd.y, hd.z], yaw: hd.yaw, scale: [1.0; 3], variant: 0, live: Live::Pedestrian { node: c, bearing: hd.bearing, length_m: hd.length } };
    let clear = |props: &[Prop]| {
        let mut ex: Vec<Extent> = vec![];
        for p in props {
            extents(d, p, &mut ex);
        }
        all_clear(s, &ex, None)
    };
    let (mut keep_pole, mut keep_arm, mut keep_head, mut keep_ped) = (vec![true; np.poles.len()], vec![true; np.arms.len()], vec![true; np.heads.len()], vec![true; np.ped_heads.len()]);
    for pi in 0..np.poles.len() {
        let id = np.poles[pi].id.clone();
        let arms: Vec<usize> = (0..np.arms.len()).filter(|&k| np.arms[k].pole == id).collect();
        let heads: Vec<usize> = (0..np.heads.len()).filter(|&k| np.heads[k].pole == id).collect();
        let peds: Vec<usize> = (0..np.ped_heads.len()).filter(|&k| ped_pole[k] == Some(pi)).collect();
        let mut parts = vec![pole_prop(&np.poles[pi])];
        parts.extend(arms.iter().map(|&k| arm_prop(&np.arms[k])));
        parts.extend(heads.iter().map(|&k| head_prop(&np.heads[k])));
        parts.extend(peds.iter().map(|&k| ped_prop(&np.ped_heads[k])));
        if clear(&parts) {
            continue;
        }
        let before = np.poles[pi].base_y + np.poles[pi].height;
        let (px, pz) = (np.poles[pi].x, np.poles[pi].z);
        // a mast cut down to a pedestal, its pole-mounted heads at the pedestal's height.
        if np.poles[pi].kind_mast {
            let mut pole = np.poles[pi].clone();
            pole.height = h.pedestal_pole_height;
            pole.kind_mast = false;
            let on_pole: Vec<usize> = heads.iter().copied().filter(|&k| np.heads[k].mount == 0).collect();
            let mut cut = vec![pole_prop(&pole)];
            for &k in &on_pole {
                let mut hd = np.heads[k].clone();
                hd.y = pole.base_y + h.pedestal_head_centre;
                cut.push(head_prop(&hd));
            }
            cut.extend(peds.iter().map(|&k| ped_prop(&np.ped_heads[k])));
            if clear(&cut) {
                for &k in &on_pole {
                    np.heads[k].y = pole.base_y + h.pedestal_head_centre;
                }
                for &k in heads.iter().filter(|&&k| np.heads[k].mount != 0) {
                    keep_head[k] = false;
                }
                for &k in &arms {
                    keep_arm[k] = false;
                }
                out.changes.push(PoleChange { x: px, z: pz, top_before: before, top_after: pole.base_y + pole.height });
                np.poles[pi] = pole;
                out.stats.pedestals += 1;
                continue;
            }
        }
        keep_pole[pi] = false;
        arms.iter().for_each(|&k| keep_arm[k] = false);
        heads.iter().for_each(|&k| keep_head[k] = false);
        peds.iter().for_each(|&k| keep_ped[k] = false);
        out.changes.push(PoleChange { x: px, z: pz, top_before: before, top_after: f64::NEG_INFINITY });
    }
    // a pedestrian head on no pole of this plan.
    for k in 0..np.ped_heads.len() {
        if ped_pole[k].is_none() && keep_ped[k] && !clear(&[ped_prop(&np.ped_heads[k])]) {
            keep_ped[k] = false;
        }
    }
    let mut count = |name: &str, kept: &[bool]| {
        let n = kept.iter().filter(|k| !**k).count();
        if n > 0 {
            *out.stats.dropped.entry(name.to_string()).or_default() += n;
        }
    };
    count("SignalPole", &keep_pole);
    count("MastArm", &keep_arm);
    count("SignalHead", &keep_head);
    count("PedHead", &keep_ped);
    out.props.extend(np.poles.iter().zip(&keep_pole).filter(|x| *x.1).map(|x| pole_prop(x.0)));
    out.props.extend(np.arms.iter().zip(&keep_arm).filter(|x| *x.1).map(|x| arm_prop(x.0)));
    out.props.extend(np.heads.iter().zip(&keep_head).filter(|x| *x.1).map(|x| head_prop(x.0)));
    out.props.extend(np.ped_heads.iter().zip(&keep_ped).filter(|x| *x.1).map(|x| ped_prop(x.0)));
    out
}

pub fn plan_node_parts(w: &PropsWorld, ni: usize) -> NodePlan {
    let h = H::new(w);
    let plan = &w.street.plan;
    let node = &plan.nodes[ni];
    let c = node.position;
    let mut np = NodePlan::default();
    let segs: Vec<(&[P2], f64)> = plan
        .entries
        .iter()
        .filter(|e| e.points.len() > 1 && polyline_near(&e.points, c[0], c[1], NEAR_RADIUS_M + e.half))
        .map(|e| (e.points.as_slice(), e.half))
        .collect();
    let lane_lines = node_lane_lines(w, node);
    let mut rules = Rules { w, segs, lanes: &lane_lines, ramp_strict: true, h: &h };
    let ground = |x: f64, z: f64| w.ground(x, z);

    // Legs in approach order (the JS orders by key: signal|segment|axis|control).
    for (li, app) in node.approaches.iter().enumerate() {
        let lane_pts = approach_lanes(w, app);
        if lane_pts.is_empty() {
            continue;
        }
        let bearing = (-app.u[1]).atan2(-app.u[0]);
        let bearing = circular_mean(&[bearing]);
        let d = [bearing.cos(), bearing.sin()];
        let n = [-d[1], d[0]];
        let mut all: Vec<Lane> = lane_pts
            .iter()
            .map(|(p, wd)| Lane { stop: *p, along: (p[0] - c[0]) * d[0] + (p[1] - c[1]) * d[1], lat: (p[0] - c[0]) * n[0] + (p[1] - c[1]) * n[1], half_width: wd / 2.0 })
            .collect();
        all.sort_by(|a, b| a.lat.total_cmp(&b.lat));
        let leg_mean_lat = all.iter().map(|l| l.lat).sum::<f64>() / all.len() as f64;
        let leg_side = if leg_mean_lat < -0.25 { -1.0 } else { 1.0 };
        let to_world = |along: f64, lat: f64| [c[0] + d[0] * along + n[0] * lat, c[1] + d[1] * along + n[1] * lat];
        // The far leg: the approach that continues this one.
        let mut far: Option<&Approach> = None;
        let mut far_dot = -1.0;
        for a in &node.approaches {
            let dot = a.u[0] * d[0] + a.u[1] * d[1];
            if dot > far_dot {
                far_dot = dot;
                far = Some(a);
            }
        }
        let seg = w.segment_of(&plan.entries[app.entry]);
        let prefers_corner = seg.street_type.as_deref().is_some_and(|t| !t.is_empty() && t != "throughway");

        let solve_group = |lanes: &[Lane], group_side: Option<f64>, pole_only: bool, rules: &mut Rules, poles: &Vec<Pole>| -> Option<Cand> {
            let mean_lat = lanes.iter().map(|l| l.lat).sum::<f64>() / lanes.len() as f64;
            let side = group_side.unwrap_or(leg_side);
            let mast = !pole_only && lanes.len() >= 2;
            let head_layout = |pole_lat: f64| -> Layout {
                let toward = if mean_lat - pole_lat >= 0.0 { 1.0 } else { -1.0 };
                let mut heads = vec![(0u8, pole_lat + toward * h.pole_head_lateral, if mast { h.pedestal_head_centre + 0.15 } else { h.pedestal_head_centre })];
                if !mast {
                    return Layout { toward, heads, reach: 0.0 };
                }
                let chosen: Vec<Lane> = if lanes.len() > h.mast_heads_max {
                    (0..h.mast_heads_max).map(|k| lanes[((k * (lanes.len() - 1)) as f64 / (h.mast_heads_max - 1) as f64).round() as usize]).collect()
                } else {
                    lanes.to_vec()
                };
                let mut reach = 0.0f64;
                let mut lats: Vec<f64> = vec![];
                for l in &chosen {
                    let off = ((l.lat - pole_lat) * toward).clamp(0.9, h.mast_max_reach);
                    if lats.iter().any(|v| (v - off).abs() < 0.8) {
                        continue;
                    }
                    lats.push(off);
                    reach = reach.max(off);
                }
                for off in lats {
                    heads.push((1u8, pole_lat + toward * off, h.mast_head_centre));
                }
                Layout { toward, heads, reach }
            };
            let view = |pole_along: f64, layout: &Layout| -> Option<f64> {
                let mut worst = 0.0f64;
                let mut worst_far = 0.0f64;
                let mut nearest = f64::INFINITY;
                for l in lanes {
                    let mut best = f64::INFINITY;
                    let mut best_along = 0.0;
                    for hd in &layout.heads {
                        let along = pole_along - l.along;
                        if along < h.min_view_distance {
                            continue;
                        }
                        let ang = (hd.1 - l.lat).abs().atan2(along) / DEG;
                        if ang < best {
                            best = ang;
                            best_along = along;
                        }
                    }
                    if !best.is_finite() {
                        return None;
                    }
                    worst = worst.max(best);
                    worst_far = worst_far.max(best_along);
                    nearest = nearest.min(best_along);
                }
                (worst <= h.max_view_angle && worst_far <= h.max_view_distance && nearest >= h.min_view_distance).then_some(worst)
            };
            let evaluate = |along: f64, lat: f64, pref: f64, rules: &Rules| -> Option<Cand> {
                let p = to_world(along, lat);
                let kerb = rules.standable(p, poles)?;
                let layout = head_layout(lat);
                let worst = view(along, &layout)?;
                let crowded = if poles.iter().any(|o| o.role != 2 && (o.x - p[0]).hypot(o.z - p[1]) < h.merge_radius) { 6.0 } else { 0.0 };
                Some(Cand { along, lat, p, kerb, layout, worst, mast, cost: worst + pref + 0.4 * (kerb - h.kerb_setback) + crowded })
            };
            let mut best: Option<Cand> = None;
            let consider = |c: Option<Cand>, best: &mut Option<Cand>| {
                if let Some(c) = c {
                    if best.as_ref().is_none_or(|b| c.cost < b.cost) {
                        *best = Some(c);
                    }
                }
            };
            let search_far = |project: bool, rules: &Rules, best: &mut Option<Cand>| {
                let Some(far) = far else { return };
                if !(far_dot >= (35.0 * DEG).cos()) {
                    return;
                }
                let reach = far.paint_reach.max(far.trim);
                let fe = &plan.entries[far.entry];
                for extra in [1.2, 2.4, 3.6, 5.0, 6.5, 8.0] {
                    let afn = reach + extra;
                    let probe = [c[0] + far.u[0] * afn, c[1] + far.u[1] * afn];
                    let mut centre_lat = (probe[0] - c[0]) * n[0] + (probe[1] - c[1]) * n[1];
                    if project && fe.points.len() > 1 {
                        let mut bd = f64::INFINITY;
                        let mut foot = probe;
                        for i in 0..fe.points.len() - 1 {
                            let (a, b) = (fe.points[i], fe.points[i + 1]);
                            let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
                            let l2 = (dx * dx + dz * dz).max(1e-9);
                            let t = (((probe[0] - a[0]) * dx + (probe[1] - a[1]) * dz) / l2).clamp(0.0, 1.0);
                            let (fx, fz) = (a[0] + dx * t, a[1] + dz * t);
                            let dd = (probe[0] - fx).hypot(probe[1] - fz);
                            if dd < bd {
                                bd = dd;
                                foot = [fx, fz];
                            }
                        }
                        centre_lat = (foot[0] - c[0]) * n[0] + (foot[1] - c[1]) * n[1];
                    } else if project {
                        centre_lat = 0.0;
                    }
                    let along_d = afn * far_dot;
                    for setback in [0.75, 1.0, 1.35, 1.7, 2.1] {
                        for s in [side, -side] {
                            consider(evaluate(along_d, centre_lat + s * (far.half + setback), if s == side { 0.0 } else { 6.0 }, rules), best);
                        }
                    }
                }
            };
            let search_ahead = |from: f64, rules: &Rules, best: &mut Option<Cand>| {
                let mut along = from;
                while along <= from + h.max_view_distance + 1e-9 {
                    let mut lat = -24.0;
                    while lat <= 24.0 + 1e-9 {
                        let pref = 2.0 + 0.15 * (along - from - 12.0).abs() + if (lat - mean_lat).signum() == side { 0.0 } else { 2.0 };
                        consider(evaluate(along, lat, pref, rules), best);
                        lat += 0.35;
                    }
                    along += 0.5;
                }
            };
            let stop_along = lanes.iter().map(|l| l.along + h.min_view_distance).fold(f64::NEG_INFINITY, f64::max);
            let far_along = stop_along.max(2.0);
            let pass = |rules: &Rules, best: &mut Option<Cand>| {
                search_far(rules.ramp_strict, rules, best);
                if best.is_none() {
                    search_ahead(far_along, rules, best);
                }
                if best.is_none() && stop_along < far_along {
                    search_ahead(stop_along, rules, best);
                }
            };
            pass(rules, &mut best);
            if best.is_none() && rules.ramp_strict {
                rules.ramp_strict = false;
                pass(rules, &mut best);
                rules.ramp_strict = true;
            }
            best
        };
        let solve_leg = |pole_only: bool, rules: &mut Rules, poles: &mut Vec<Pole>| -> Option<Vec<Cand>> {
            if let Some(whole) = solve_group(&all, None, pole_only, rules, poles) {
                return Some(vec![whole]);
            }
            if all.len() < 2 {
                return None;
            }
            let cut = all.len().div_ceil(2);
            let halves = [all[..cut].to_vec(), all[cut..].to_vec()];
            let mut solved = vec![];
            for (k, half) in halves.iter().enumerate() {
                let Some(s) = solve_group(half, Some(if k == 0 { -1.0 } else { 1.0 }), pole_only, rules, poles) else { break };
                poles.push(Pole { id: String::new(), leg: -1, kind_mast: false, role: 0, x: s.p[0], z: s.p[1], base_y: 0.0, height: 0.0, kerb: 0.0, reserved: true });
                solved.push(s);
            }
            poles.retain(|p| !p.reserved);
            (solved.len() == 2).then_some(solved)
        };
        let groups = (if prefers_corner { solve_leg(true, &mut rules, &mut np.poles) } else { None }).or_else(|| solve_leg(false, &mut rules, &mut np.poles));
        let Some(groups) = groups else {
            np.unplaced_legs += 1;
            continue;
        };
        let facing = [-d[0], -d[1]];
        let yaw = facing[0].atan2(facing[1]);
        let ng = groups.len();
        for (g, best) in groups.into_iter().enumerate() {
            let pole_id = if ng > 1 { format!("{li}:{g}") } else { format!("{li}") };
            let base_y = ground(best.p[0], best.p[1]);
            np.poles.push(Pole {
                id: pole_id.clone(),
                leg: li as i32,
                kind_mast: best.mast,
                role: 0,
                x: best.p[0],
                z: best.p[1],
                base_y,
                height: if best.mast { h.mast_pole_height } else { h.pedestal_pole_height },
                kerb: best.kerb,
                reserved: false,
            });
            if best.mast && best.layout.reach > 0.0 {
                let dir = [n[0] * best.layout.toward, n[1] * best.layout.toward];
                np.arms.push(Arm { pole: pole_id.clone(), x: best.p[0], z: best.p[1], y: base_y + h.mast_arm_height, length: best.layout.reach + 0.35, yaw: (-dir[1]).atan2(dir[0]), near_arm: false });
            }
            for hd in &best.layout.heads {
                let p = to_world(best.along, hd.1);
                np.heads.push(Head { pole: pole_id.clone(), mount: hd.0, near_arm: false, x: p[0], y: base_y + hd.2, z: p[1], yaw, bearing });
            }
            let _ = best.worst;
            let _ = best.lat;
        }

        // NEAR-SIDE HEAD.
        let mut near_best: Option<(P2, f64, Lane, Option<usize>, f64, f64)> = None; // (p, cost, kerbLane, existing pole, mount height, kerb)
        let heads_on = |np: &NodePlan, id: &str| np.heads.iter().filter(|x| x.pole == id).map(|x| x.y).collect::<Vec<f64>>();
        for kerb_side in [leg_side, -leg_side] {
            let kl = if kerb_side > 0.0 { *all.last().unwrap() } else { all[0] };
            for (pi, pole) in np.poles.iter().enumerate() {
                if pole.reserved || pole.role == 2 {
                    continue;
                }
                let (vx, vz) = (pole.x - kl.stop[0], pole.z - kl.stop[1]);
                let ahead = vx * d[0] + vz * d[1];
                let off = (vx * n[0] + vz * n[1]).abs();
                if !(0.5..=h.near_max_ahead).contains(&ahead) {
                    continue;
                }
                let angle = off.atan2(ahead) / DEG;
                if angle > h.near_max_view_angle {
                    continue;
                }
                let taken: Vec<f64> = heads_on(&np, &pole.id).iter().map(|y| y - pole.base_y).collect();
                let Some(mh) = [h.pedestal_head_centre, h.near_mast_head_centre].into_iter().find(|y| y + h.head_half_height + 0.3 < pole.height && taken.iter().all(|t| (t - y).abs() >= 1.2)) else { continue };
                let dist = ahead.hypot(off);
                let cost = angle + 0.6 * dist - 6.0 + if kerb_side == leg_side { 0.0 } else { 8.0 };
                if near_best.as_ref().is_none_or(|b| cost < b.1) {
                    near_best = Some(([pole.x, pole.z], cost, kl, Some(pi), mh, pole.kerb));
                }
            }
        }
        for strict in [true, false] {
            if !strict && near_best.is_some() {
                break;
            }
            for kerb_side in [leg_side, -leg_side] {
                let kl = if kerb_side > 0.0 { *all.last().unwrap() } else { all[0] };
                let mut ahead = 0.5;
                while ahead <= h.near_max_ahead + 1e-9 {
                    let mut off = kl.half_width + h.lane_margin;
                    while off <= kl.half_width + h.near_max_kerb_offset + 1e-9 {
                        let angle = off.atan2(ahead) / DEG;
                        if angle <= h.near_max_view_angle {
                            let p = to_world(kl.along + ahead, kl.lat + kerb_side * off);
                            let clutter = np.poles.iter().any(|o| o.role != 2 && !o.reserved && (o.x - p[0]).hypot(o.z - p[1]) < h.merge_radius);
                            if !(strict && clutter) {
                                if let Some(kerb) = rules.standable(p, &np.poles) {
                                    let dist = ahead.hypot(off);
                                    let cost = angle + 0.6 * dist + 0.4 * (kerb - h.kerb_setback) + if kerb_side == leg_side { 0.0 } else { 8.0 };
                                    if near_best.as_ref().is_none_or(|b| cost < b.1) {
                                        near_best = Some((p, cost, kl, None, h.pedestal_head_centre, kerb));
                                    }
                                }
                            }
                        }
                        off += 0.25;
                    }
                    ahead += 0.5;
                }
            }
        }
        // NEAR-SIDE ARM where no footway point lies in the cone.
        let mut near_arm: Option<(P2, P2, Lane, Option<usize>, f64, f64)> = None; // (p, hp, lane, existing, cost, kerb)
        if near_best.is_none() {
            let tan_cone = ((h.near_max_view_angle - 2.0) * DEG).tan();
            for kerb_side in [leg_side, -leg_side] {
                let kl = if kerb_side > 0.0 { *all.last().unwrap() } else { all[0] };
                let mut consider = |p: P2, existing: Option<usize>, kerb: f64, np: &NodePlan| {
                    let (vx, vz) = (p[0] - kl.stop[0], p[1] - kl.stop[1]);
                    let ahead = vx * d[0] + vz * d[1];
                    let off = (vx * n[0] + vz * n[1]) * kerb_side;
                    if !(1.5..=h.near_max_ahead).contains(&ahead) || !(off > 0.0) {
                        return;
                    }
                    let head_off = (off - 0.9).min(ahead * tan_cone);
                    if !(head_off >= 0.0) {
                        return;
                    }
                    let hp = to_world(kl.along + ahead, kl.lat + kerb_side * head_off);
                    let reach = (hp[0] - p[0]).hypot(hp[1] - p[1]);
                    if !(0.9..=h.near_max_arm_reach).contains(&reach) {
                        return;
                    }
                    if let Some(ei) = existing {
                        let yaw_new = (-(hp[1] - p[1])).atan2(hp[0] - p[0]);
                        let id = &np.poles[ei].id;
                        if np.arms.iter().any(|a| &a.pole == id && (a.yaw - yaw_new).sin().atan2((a.yaw - yaw_new).cos()).abs() < 60.0 * DEG) {
                            return;
                        }
                    }
                    let cost = 1.5 * reach + 0.3 * ahead + if kerb_side == leg_side { 0.0 } else { 8.0 } - if existing.is_some() { 3.0 } else { 0.0 };
                    if near_arm.as_ref().is_none_or(|b| cost < b.4) {
                        near_arm = Some((p, hp, kl, existing, cost, kerb));
                    }
                };
                for (pi, pole) in np.poles.iter().enumerate() {
                    if pole.reserved || pole.role == 2 || !pole.kind_mast {
                        continue;
                    }
                    consider([pole.x, pole.z], Some(pi), pole.kerb, &np);
                }
                let mut ahead = 1.5;
                while ahead <= h.near_max_ahead + 1e-9 {
                    let mut off = kl.half_width + h.lane_margin;
                    while off <= kl.half_width + h.near_max_arm_reach + 2.0 + 1e-9 {
                        let p = to_world(kl.along + ahead, kl.lat + kerb_side * off);
                        if let Some(kerb) = rules.standable(p, &np.poles) {
                            consider(p, None, kerb, &np);
                        }
                        off += 0.25;
                    }
                    ahead += 0.5;
                }
            }
        }
        if let Some((p, hp, kl, existing, _, kerb)) = near_arm {
            let (pole_id, base_y) = match existing {
                Some(ei) => (np.poles[ei].id.clone(), np.poles[ei].base_y),
                None => {
                    let id = format!("{li}:near");
                    let base_y = ground(p[0], p[1]);
                    np.poles.push(Pole { id: id.clone(), leg: li as i32, kind_mast: true, role: 1, x: p[0], z: p[1], base_y, height: h.mast_pole_height, kerb, reserved: false });
                    (id, base_y)
                }
            };
            let arm_y = base_y + h.mast_arm_height;
            np.arms.push(Arm { pole: pole_id.clone(), x: p[0], z: p[1], y: arm_y, length: (hp[0] - p[0]).hypot(hp[1] - p[1]) + 0.3, yaw: (-(hp[1] - p[1])).atan2(hp[0] - p[0]), near_arm: true });
            let aim = to_world(kl.along - 12.0, leg_mean_lat);
            let (fx, fz) = (aim[0] - hp[0], aim[1] - hp[1]);
            np.heads.push(Head { pole: pole_id, mount: 1, near_arm: true, x: hp[0], y: arm_y - (h.mast_arm_height - h.mast_head_centre), z: hp[1], yaw: fx.atan2(fz), bearing });
        } else if let Some((p, _, kl, existing, mh, kerb)) = near_best {
            let (pole_id, base_y) = match existing {
                Some(ei) => (np.poles[ei].id.clone(), np.poles[ei].base_y),
                None => {
                    let id = format!("{li}:near");
                    let base_y = ground(p[0], p[1]);
                    np.poles.push(Pole { id: id.clone(), leg: li as i32, kind_mast: false, role: 1, x: p[0], z: p[1], base_y, height: h.pedestal_pole_height, kerb, reserved: false });
                    (id, base_y)
                }
            };
            let aim = to_world(kl.along - 12.0, leg_mean_lat);
            let (fx, fz) = (aim[0] - p[0], aim[1] - p[1]);
            let fl = fx.hypot(fz).max(1e-9);
            let f = [fx / fl, fz / fl];
            np.heads.push(Head { pole: pole_id, mount: 2, near_arm: false, x: p[0] + f[0] * 0.2, y: base_y + mh, z: p[1] + f[1] * 0.2, yaw: f[0].atan2(f[1]), bearing });
        }
    }
    merge_corner_masts(&mut np, &h);
    plan_ped(w, node, &mut np, &mut rules);
    np
}

fn arms_cross(a: &Arm, b: &Arm) -> bool {
    let seg = |m: &Arm| ([m.x, m.z], [m.x + m.yaw.cos() * m.length, m.z - m.yaw.sin() * m.length]);
    let (p1, p2) = seg(a);
    let (q1, q2) = seg(b);
    let cr = |p: P2, q: P2, r: P2| (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
    cr(p1, p2, q1) * cr(p1, p2, q2) < 0.0 && cr(q1, q2, p1) * cr(q1, q2, p2) < 0.0
}

/// `mergeCornerMasts`.
fn merge_corner_masts(np: &mut NodePlan, h: &H) {
    let ang = |a: f64, b: f64| (a - b).sin().atan2((a - b).cos()).abs();
    let mut i = np.poles.len();
    while i > 0 {
        i -= 1;
        if i >= np.poles.len() || np.poles[i].role == 2 {
            continue;
        }
        let b = np.poles[i].clone();
        let b_arms: Vec<usize> = (0..np.arms.len()).filter(|&k| np.arms[k].pole == b.id).collect();
        let mut best: Option<(usize, f64, f64, f64, Vec<(usize, f64, P2)>, Vec<(usize, f64, f64, f64)>)> = None;
        for j in 0..np.poles.len() {
            let a = &np.poles[j];
            if j == i || a.role == 2 {
                continue;
            }
            if !b_arms.is_empty() && !a.kind_mast {
                continue;
            }
            if b_arms.is_empty() && !a.kind_mast && j > i {
                continue;
            }
            if a.leg == b.leg && !b_arms.is_empty() && np.arms.iter().any(|o| o.pole == a.id) {
                continue;
            }
            let (dx, dz) = (a.x - b.x, a.z - b.z);
            let dist = dx.hypot(dz);
            let crossing = dist <= h.merge_crossing_radius && b_arms.iter().any(|&x| np.arms.iter().any(|y| y.pole == a.id && arms_cross(&np.arms[x], y)));
            if dist > h.merge_radius && !crossing {
                continue;
            }
            let mut moves = vec![];
            let mut ok = true;
            for &k in &b_arms {
                let arm = &np.arms[k];
                let u = [arm.yaw.cos(), -arm.yaw.sin()];
                let along = dx * u[0] + dz * u[1];
                let across = -dx * u[1] + dz * u[0];
                if across.abs() > h.merge_max_across || arm.length - along < h.merge_min_arm {
                    ok = false;
                    break;
                }
                if np.arms.iter().any(|o| o.pole == a.id && ang(o.yaw, arm.yaw) < h.merge_min_arm_sep_deg * DEG) {
                    ok = false;
                    break;
                }
                moves.push((k, along, [-u[1] * across, u[0] * across]));
            }
            if !ok {
                continue;
            }
            let taken: Vec<(f64, f64, f64)> = np.heads.iter().filter(|x| x.pole == a.id && x.mount != 1).map(|x| (x.y - a.base_y, x.x, x.z)).collect();
            let mut bands: Vec<(usize, f64, f64, f64)> = vec![];
            for (hk, hd) in np.heads.iter().enumerate().filter(|(_, x)| x.pole == b.id && x.mount != 1) {
                let own = hd.y - b.base_y;
                let (hx, hz) = (hd.x + dx, hd.z + dz);
                let y = [own, h.pedestal_head_centre, h.pedestal_head_centre + 0.15, h.near_mast_head_centre].into_iter().find(|&cy| {
                    cy + h.head_half_height + 0.3 < a.height
                        && taken.iter().map(|t| (t.0, t.1, t.2)).chain(bands.iter().map(|bb| (bb.1, bb.2, bb.3))).all(|t| (t.0 - cy).abs() >= h.merge_band || (t.1 - hx).hypot(t.2 - hz) >= 0.55)
                });
                let Some(y) = y else {
                    ok = false;
                    break;
                };
                bands.push((hk, y, hx, hz));
            }
            if !ok {
                continue;
            }
            let rank = dist - if a.kind_mast { 10.0 } else { 0.0 };
            if best.as_ref().is_none_or(|bb| rank < bb.1) {
                best = Some((j, rank, dx, dz, moves, bands));
            }
        }
        let Some((j, _, dx, dz, moves, bands)) = best else { continue };
        let a = np.poles[j].clone();
        for (k, along, shift) in moves {
            let near = np.arms[k].near_arm;
            for hd in np.heads.iter_mut() {
                if hd.pole != b.id || hd.mount != 1 || hd.near_arm != near {
                    continue;
                }
                hd.x += shift[0];
                hd.z += shift[1];
                hd.y += a.base_y - b.base_y;
            }
            let arm = &mut np.arms[k];
            arm.pole = a.id.clone();
            arm.x = a.x;
            arm.z = a.z;
            arm.y += a.base_y - b.base_y;
            arm.length -= along;
        }
        for (hk, y, _, _) in bands {
            let hd = &mut np.heads[hk];
            hd.x += dx;
            hd.z += dz;
            hd.y = a.base_y + y;
        }
        for hd in np.heads.iter_mut() {
            if hd.pole == b.id {
                hd.pole = a.id.clone();
            }
        }
        np.poles.remove(i);
    }
}

/// `planPedestrianCrossings` + `planPedestrianHeads` for one junction.
fn plan_ped(w: &PropsWorld, node: &Node, np: &mut NodePlan, rules: &mut Rules) {
    if !node.earns_paint {
        return;
    }
    let ph = |k: &str| w.js.num("pedHead", k);
    let (centre_m, post_h, share_r, standoff) = (ph("centreM"), ph("postHeightM"), ph("shareRadiusM"), ph("standoffM"));
    let speed = w.js.num("pedSignal", "speedMps");
    let _ = speed;
    let clearance = w.street.o.crosswalk_clearance;
    let c = node.position;
    let band_depth = if node.band_depth > 0.0 { node.band_depth } else { 2.4 };
    for app in &node.approaches {
        let e = &w.street.plan.entries[app.entry];
        let available = app.run_length - if app.at_start { e.trim_end } else { e.trim_start };
        let band_start = app.trim + clearance;
        if !(band_start + band_depth + 0.4 <= available) {
            continue;
        }
        if !(app.width_ccw >= 1.0 && app.width_cw >= 1.0) {
            continue;
        }
        let along = band_start + band_depth / 2.0;
        let m = [-app.u[1], app.u[0]];
        let mid = [c[0] + app.u[0] * along, c[1] + app.u[1] * along];
        let a = [mid[0] + m[0] * app.half, mid[1] + m[1] * app.half];
        let b = [mid[0] - m[0] * app.half, mid[1] - m[1] * app.half];
        let length = 2.0 * app.half;
        if !(length > 0.5) {
            continue;
        }
        let bearing = (b[1] - a[1]).atan2(b[0] - a[0]);
        let mark = (np.poles.len(), np.ped_heads.len());
        let mut failed = false;
        for (end, other, sgn) in [(a, b, 1.0), (b, a, -1.0)] {
            let facing = [(other[0] - end[0]) / length, (other[1] - end[1]) / length];
            let outv = [m[0] * sgn, m[1] * sgn];
            let share_on = |radius: f64, np: &NodePlan| -> Option<(usize, P2, f64)> {
                let mut found: Option<(usize, P2, f64, f64)> = None;
                for (pi, pole) in np.poles.iter().enumerate() {
                    if pole.reserved {
                        continue;
                    }
                    let (vx, vz) = (pole.x - end[0], pole.z - end[1]);
                    let dist = vx.hypot(vz);
                    if dist > radius || vx * outv[0] + vz * outv[1] < 0.4 {
                        continue;
                    }
                    let y = pole.base_y + centre_m;
                    let side = [-facing[1], facing[0]];
                    for lateral in [0.0, 0.36, -0.36, 0.5, -0.5] {
                        let mount = [pole.x + facing[0] * standoff + side[0] * lateral, pole.z + facing[1] * standoff + side[1] * lateral];
                        let clash = np.heads.iter().filter(|x| x.pole == pole.id).map(|x| (x.y, x.x, x.z)).chain(np.ped_heads.iter().filter(|_| false).map(|x| (x.y, x.x, x.z))).any(|(hy, hx, hz)| (hy - y).abs() < 0.95 && (hx - mount[0]).hypot(hz - mount[1]) < 0.6)
                            || np.ped_heads.iter().any(|x| (x.y - y).abs() < 0.95 && (x.x - mount[0]).hypot(x.z - mount[1]) < 0.6);
                        if clash {
                            continue;
                        }
                        let cost = dist + lateral.abs();
                        if found.as_ref().is_none_or(|f| cost < f.3) {
                            found = Some((pi, mount, y, cost));
                        }
                        break;
                    }
                }
                found.map(|f| (f.0, f.1, f.2))
            };
            let mut best = share_on(share_r, np);
            if best.is_none() {
                let mut spot: Option<(P2, f64, f64)> = None;
                for setback in [0.8, 1.1, 1.4, 1.8, 2.2, 2.5, 3.0, 3.6, 4.2, 5.0] {
                    for shift in [0.0f64, 0.6, -0.6, 1.2, -1.2, 1.8, -1.8, 2.4, -2.4, 3.2, -3.2, 4.0, -4.0] {
                        let al = shift + (if shift == 0.0 { 1.0 } else { shift.signum() }) * if shift != 0.0 { band_depth / 2.0 } else { 0.0 };
                        let p = [end[0] + outv[0] * setback + app.u[0] * al, end[1] + outv[1] * setback + app.u[1] * al];
                        let Some(kerb) = rules.standable(p, &np.poles) else { continue };
                        let cost = 0.6 * (al + 1.5).abs() + 0.5 * setback + if setback > 2.5 { 4.0 } else { 0.0 };
                        if spot.as_ref().is_none_or(|s| cost < s.1) {
                            spot = Some((p, cost, kerb));
                        }
                    }
                }
                match spot {
                    Some((p, _, kerb)) => {
                        let base_y = w.ground(p[0], p[1]);
                        np.poles.push(Pole { id: format!("ped:{}:{}", app.entry, sgn), leg: -1, kind_mast: false, role: 2, x: p[0], z: p[1], base_y, height: post_h, kerb, reserved: false });
                        let pi = np.poles.len() - 1;
                        best = Some((pi, [p[0] + facing[0] * standoff, p[1] + facing[1] * standoff], base_y + centre_m));
                    }
                    None => {
                        best = share_on(9.0, np);
                        if best.is_none() {
                            failed = true;
                            break;
                        }
                    }
                }
            }
            let (_, mount, y) = best.unwrap();
            np.ped_heads.push(PedHead { x: mount[0], y, z: mount[1], yaw: facing[0].atan2(facing[1]), bearing, length });
        }
        if failed {
            np.poles.truncate(mark.0);
            np.ped_heads.truncate(mark.1);
        }
    }
}
