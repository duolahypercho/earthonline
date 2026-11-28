//! Lane geometry: polylines, the junction movement classes and the turning
//! plans a car drives through a node. Ports of `buildCornerFillet`,
//! `straightJoinPlan`, `buildSmoothThroughMerge`, `buildJunctionPlan` and
//! `collinearRunAt` (src/citygen/traffic.js) and `classifyMovement`,
//! `movementsAllowedByLaneRole`, `movementRouteWeight`, `highwayPriority`
//! (src/simulation/traffic/junction-movement-contract.js). Pure functions of
//! two lane polylines; computed once per lane link, never per car per step.

use crate::consts::*;

pub type P2 = [f64; 2];

#[inline]
pub fn hypot(x: f64, z: f64) -> f64 {
    x.hypot(z)
}

/// Cumulative arc lengths of a polyline.
pub fn cumulative(points: &[P2]) -> Vec<f64> {
    let mut cum = Vec::with_capacity(points.len());
    cum.push(0.0);
    for i in 1..points.len() {
        let prev = cum[i - 1];
        cum.push(prev + hypot(points[i][0] - points[i - 1][0], points[i][1] - points[i - 1][1]));
    }
    cum
}

/// Position and unit segment tangent at arc `s` on a polyline (`pathPositionAtArc`), plus the segment index.
#[inline]
pub fn at_arc(points: &[P2], cum: &[f64], s: f64) -> (f64, f64, f64, f64, usize) {
    let n = points.len();
    if n < 2 {
        let p = points.first().copied().unwrap_or([0.0, 0.0]);
        return (p[0], p[1], 0.0, 1.0, 0);
    }
    // `pathPositionAtArc`: advance while `arc > cum[index + 1]`, so an arc exactly
    // on a vertex stays on the segment that ends there (its heading, not the next one's).
    let i = cum[1..n - 1].partition_point(|&c| c < s);
    let a = points[i];
    let b = points[i + 1];
    let len = cum[i + 1] - cum[i];
    let dx = b[0] - a[0];
    let dz = b[1] - a[1];
    let t = if len > 1e-9 { ((s - cum[i]) / len).clamp(0.0, 1.0) } else { 0.0 };
    let inv = if len > 1e-9 { 1.0 / len } else { 0.0 };
    (a[0] + dx * t, a[1] + dz * t, dx * inv, dz * inv, i)
}

/// Distance from a point to a polyline.
pub fn dist_to_polyline(points: &[P2], x: f64, z: f64) -> f64 {
    let mut best = f64::INFINITY;
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let dx = b[0] - a[0];
        let dz = b[1] - a[1];
        let len2 = (dx * dx + dz * dz).max(1e-9);
        let t = (((x - a[0]) * dx + (z - a[1]) * dz) / len2).clamp(0.0, 1.0);
        best = best.min(hypot(a[0] + dx * t - x, a[1] + dz * t - z));
    }
    best
}

/// The length of a polyline inside the disc of radius `r` round `c` (each
/// segment clipped to the circle).
pub fn length_in_disc(points: &[P2], c: P2, r: f64) -> f64 {
    let mut total = 0.0;
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let d = [b[0] - a[0], b[1] - a[1]];
        let f = [a[0] - c[0], a[1] - c[1]];
        let aa = d[0] * d[0] + d[1] * d[1];
        if aa < 1e-12 {
            continue;
        }
        let bb = 2.0 * (f[0] * d[0] + f[1] * d[1]);
        let cc = f[0] * f[0] + f[1] * f[1] - r * r;
        let disc = bb * bb - 4.0 * aa * cc;
        if disc <= 0.0 {
            continue;
        }
        let s = disc.sqrt();
        let t0 = ((-bb - s) / (2.0 * aa)).max(0.0);
        let t1 = ((-bb + s) / (2.0 * aa)).min(1.0);
        if t1 > t0 {
            total += (t1 - t0) * aa.sqrt();
        }
    }
    total
}

/// `wrapPi`.
#[inline]
pub fn wrap_pi(mut a: f64) -> f64 {
    let tau = std::f64::consts::PI * 2.0;
    while a > std::f64::consts::PI {
        a -= tau;
    }
    while a < -std::f64::consts::PI {
        a += tau;
    }
    a
}

/// `normalizeAngle` / `shortestAngle`.
#[inline]
pub fn normalize_angle(a: f64) -> f64 {
    a.sin().atan2(a.cos())
}

/// `collinearRunAt(points, direction)`.
pub fn collinear_run_at(points: &[P2], direction: i32) -> f64 {
    let n = points.len() as isize;
    let mut run = 0.0;
    let mut u: Option<(f64, f64)> = None;
    let (from, step) = if direction > 0 { (n - 1, -1isize) } else { (0, 1isize) };
    let mut i = from;
    while i + step >= 0 && i + step < n {
        let a = points[i as usize];
        let o = points[(i + step) as usize];
        let dx = a[0] - o[0];
        let dz = a[1] - o[1];
        let len = hypot(dx, dz);
        if len > 1e-6 {
            let (nx, nz) = (dx / len, dz / len);
            match u {
                None => u = Some((nx, nz)),
                Some((ux, uz)) => {
                    if nx * ux + nz * uz < 0.9998 {
                        break;
                    }
                }
            }
            run += len;
        }
        i += step;
    }
    run
}

// ---------------------------------------------------------------- movements

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Movement {
    Left,
    Through,
    Right,
    UTurn,
}

impl Movement {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "left" => Movement::Left,
            "through" => Movement::Through,
            "right" => Movement::Right,
            "uturn" => Movement::UTurn,
            _ => return None,
        })
    }
}

/// Painted lane role (`laneRole`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum LaneRole {
    Through,
    LeftTurn,
    RightTurn,
    ThroughOnly,
    ThroughLeft,
    ThroughRight,
}

impl LaneRole {
    pub fn parse(s: &str) -> Self {
        match s {
            "left-turn" | "center-turn" => LaneRole::LeftTurn,
            "right-turn" => LaneRole::RightTurn,
            "through-only" => LaneRole::ThroughOnly,
            "through-left" => LaneRole::ThroughLeft,
            "through-right" => LaneRole::ThroughRight,
            _ => LaneRole::Through,
        }
    }

    /// `movementsAllowedByLaneRole`.
    pub fn allows(self, m: Movement) -> bool {
        use Movement::*;
        match self {
            LaneRole::LeftTurn => matches!(m, Left | UTurn),
            LaneRole::RightTurn => m == Right,
            LaneRole::ThroughOnly => m == Through,
            LaneRole::ThroughLeft => matches!(m, Through | Left),
            LaneRole::ThroughRight => matches!(m, Through | Right),
            LaneRole::Through => true,
        }
    }

    /// `movementRouteWeight(role, movement)`.
    pub fn route_weight(self, m: Movement) -> f64 {
        use Movement::*;
        match self {
            LaneRole::LeftTurn => match m {
                Left => 4.0,
                UTurn => 0.8,
                _ => 0.0,
            },
            LaneRole::RightTurn => {
                if m == Right { 4.0 } else { 0.0 }
            }
            LaneRole::ThroughOnly => {
                if m == Through { 4.0 } else { 0.0 }
            }
            LaneRole::ThroughLeft => match m {
                Through => 4.0,
                Left => 1.6,
                _ => 0.0,
            },
            LaneRole::ThroughRight => match m {
                Through => 4.0,
                Right => 1.6,
                _ => 0.0,
            },
            LaneRole::Through => match m {
                Through => 4.0,
                Left | Right => 1.6,
                UTurn => 0.25,
            },
        }
    }
}

/// `travelRay(edge, atEnd)`: the lane's own end chord.
fn travel_ray(points: &[P2], at_end: bool) -> Option<P2> {
    let n = points.len();
    if n < 2 {
        return None;
    }
    Some(if at_end {
        [points[n - 1][0] - points[n - 2][0], points[n - 1][1] - points[n - 2][1]]
    } else {
        [points[1][0] - points[0][0], points[1][1] - points[0][1]]
    })
}

/// `classifyMovement(edge, next)`.
pub fn classify_movement(inp: &[P2], out: &[P2]) -> Movement {
    let (Some(i), Some(o)) = (travel_ray(inp, true), travel_ray(out, false)) else {
        return Movement::Through;
    };
    let il = hypot(i[0], i[1]);
    let ol = hypot(o[0], o[1]);
    let il = if il == 0.0 { 1.0 } else { il };
    let ol = if ol == 0.0 { 1.0 } else { ol };
    let dot = ((i[0] * o[0] + i[1] * o[1]) / (il * ol)).clamp(-1.0, 1.0);
    let sin = (i[1] * o[0] - i[0] * o[1]) / (il * ol);
    if dot < -0.35 && sin.abs() < 0.55 {
        return Movement::UTurn;
    }
    if sin.abs() < 0.35 {
        return Movement::Through;
    }
    if sin > 0.0 { Movement::Left } else { Movement::Right }
}

/// `HIGHWAY_PRIORITY` / `highwayPriority`.
pub fn highway_priority(highway: &str) -> i32 {
    match highway {
        "motorway" => 6,
        "trunk" => 5,
        "primary" => 4,
        "secondary" => 3,
        "tertiary" => 2,
        "unclassified" | "residential" => 1,
        "living_street" | "service" => 0,
        _ => 1,
    }
}

/// `classRankOf` in signal-control-plan.js.
pub fn class_rank(highway: &str) -> i32 {
    match highway {
        "motorway" | "trunk" => 5,
        "primary" => 4,
        "secondary" => 3,
        "tertiary" => 2,
        _ => 1,
    }
}

/// `designSpeed` in signal-control-plan.js (mirrors `CLASS_SPEED_MS` in traffic.js).
pub fn design_speed(highway: &str) -> f64 {
    match highway {
        "primary" | "trunk" => 12.0,
        "secondary" => 10.5,
        "tertiary" => 9.0,
        _ => 7.2,
    }
}

/// `parsePostedSpeedMps` in realistic-movement-speeds.js.
pub fn parse_posted_speed_mps(tag: &str) -> Option<f64> {
    let t = tag.trim();
    if t.is_empty() {
        return None;
    }
    let lower = t.to_ascii_lowercase();
    let num_end = lower.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(lower.len());
    if num_end == 0 {
        return None;
    }
    let value: f64 = lower[..num_end].parse().ok()?;
    let rest = lower[num_end..].trim_start();
    let speed = if rest == "mph" {
        value * METRES_PER_SECOND_PER_MPH
    } else if rest.is_empty() || rest == "km/h" || rest == "kmh" || rest == "kph" {
        value * (1.0 / 3.6)
    } else {
        return None;
    };
    (speed.is_finite() && speed > 0.0).then_some(speed)
}

pub(crate) fn freeway(highway: &str) -> bool {
    matches!(highway, "motorway" | "motorway_link")
}

/// `autonomousTrafficSpeedMps({ edge })`: the speed target of a lane.
pub fn autonomous_speed(highway: &str, maxspeed: &str, maxspeed_kmh: Option<f64>, maxspeed_source: &str) -> f64 {
    let mut speed = parse_posted_speed_mps(maxspeed);
    if speed.is_none() && maxspeed_source != "zone-default" {
        if let Some(kmh) = maxspeed_kmh.filter(|k| k.is_finite() && *k > 0.0) {
            speed = Some(kmh * (1.0 / 3.6));
        }
    }
    if let Some(posted) = speed {
        return if freeway(highway) { posted } else { posted.min(AUTONOMOUS_TRAFFIC_CEILING_MPS) };
    }
    match highway {
        "motorway" | "trunk" | "primary" => AUTONOMOUS_TRAFFIC_CEILING_MPS,
        "secondary" => 10.5,
        "tertiary" => 9.0,
        "unclassified" | "residential" => 7.2,
        "living_street" | "service" => 15.0 * METRES_PER_SECOND_PER_MPH,
        _ => 25.0 * METRES_PER_SECOND_PER_MPH,
    }
}

// ---------------------------------------------------------------- plans

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PlanKind {
    Line,
    Arc,
    Merge,
}

/// One junction movement's driven path (`buildJunctionPlan`'s result).
#[derive(Clone, Debug)]
pub struct Plan {
    pub kind: PlanKind,
    pub turn_side: i8,
    pub theta: f64,
    pub radius: f64,
    pub arc_len: f64,
    pub tangent: f64,
    pub along_in: f64,
    pub entry_setback: f64,
    pub exit_arc: f64,
    pub speed_cap: f64,
    pub lateral_step: f64,
    /// Line: [x, z, dx, dz, yaw_in, dyaw]; Arc: [cx, cz, rx, rz, sign, yaw_in]; Merge: p0..p3.
    g: [f64; 8],
    /// Merge only: cumulative Gauss-Legendre arc length at the 65 knots.
    lengths: Option<Box<[f64; MERGE_SAMPLES + 1]>>,
    /// Stations where this path meets its drawn junction surface, with heights.
    pub(crate) junction_grade_seams: std::sync::OnceLock<Box<[[f64; 2]]>>,
}

fn straight_join(p_in: P2, p_out: P2, yaw_in: f64, yaw_out: f64) -> Plan {
    let dx = p_out[0] - p_in[0];
    let dz = p_out[1] - p_in[1];
    let arc_len = hypot(dx, dz).max(0.5);
    let dyaw = wrap_pi(yaw_out - yaw_in);
    let lateral = (dx * -yaw_in.cos() + dz * yaw_in.sin()).abs();
    Plan {
        kind: PlanKind::Line,
        turn_side: 0,
        theta: dyaw,
        radius: f64::INFINITY,
        arc_len,
        tangent: f64::NAN,
        along_in: f64::NAN,
        entry_setback: 0.0,
        exit_arc: 0.0,
        speed_cap: f64::INFINITY,
        lateral_step: lateral,
        g: [p_in[0], p_in[1], dx, dz, yaw_in, dyaw, 0.0, 0.0],
        lengths: None,
        junction_grade_seams: Default::default(),
    }
}

/// `buildCornerFillet(edge, next, tangentCap)`.
pub fn corner_fillet(inp: &[P2], in_len: f64, out: &[P2], out_len: f64, tangent_cap: f64) -> Option<Plan> {
    if inp.len() < 2 || out.len() < 2 {
        return None;
    }
    let p_in = inp[inp.len() - 1];
    let p_in_prev = inp[inp.len() - 2];
    let p_out = out[0];
    let p_out_next = out[1];
    let in_span = hypot(p_in[0] - p_in_prev[0], p_in[1] - p_in_prev[1]);
    let out_span = hypot(p_out_next[0] - p_out[0], p_out_next[1] - p_out[1]);
    if !(in_span > 1e-6) || !(out_span > 1e-6) {
        return None;
    }
    let d_in = [(p_in[0] - p_in_prev[0]) / in_span, (p_in[1] - p_in_prev[1]) / in_span];
    let d_out = [(p_out_next[0] - p_out[0]) / out_span, (p_out_next[1] - p_out[1]) / out_span];
    let yaw_in = d_in[0].atan2(d_in[1]);
    let yaw_out = d_out[0].atan2(d_out[1]);
    let theta = wrap_pi(yaw_out - yaw_in);
    let straight = straight_join(p_in, p_out, yaw_in, yaw_out);
    let sweep = theta.abs();
    if sweep < TURN_MIN_ANGLE || sweep > TURN_MAX_ANGLE {
        return Some(straight);
    }
    let den = d_in[0] * d_out[1] - d_in[1] * d_out[0];
    if den.abs() < 1e-6 {
        return Some(straight);
    }
    let half_tan = (theta / 2.0).tan().abs();
    if !(half_tan > 1e-6) {
        return Some(straight);
    }
    let wx = p_out[0] - p_in[0];
    let wz = p_out[1] - p_in[1];
    let along_in = (wx * d_out[1] - wz * d_out[0]) / den;
    let vx = p_in[0] + along_in * d_in[0];
    let vz = p_in[1] + along_in * d_in[1];
    let along_out = (vx - p_out[0]) * d_out[0] + (vz - p_out[1]) * d_out[1];
    let room_in = (collinear_run_at(inp, 1) - TURN_TANGENT_EPS).min(TURN_TANGENT_FRACTION * in_len);
    let room_out = (collinear_run_at(out, -1) - TURN_TANGENT_EPS).min(TURN_TANGENT_FRACTION * out_len);
    let tangent_min = TURN_TANGENT_EPS.max(along_in + TURN_TANGENT_EPS).max(TURN_TANGENT_EPS - along_out);
    let tangent_max = (along_in + room_in).min(room_out - along_out).min(along_in + TURN_SETBACK_MAX);
    if !(tangent_max >= tangent_min) {
        return Some(straight);
    }
    let wanted = TURN_RADIUS_WANT.clamp(TURN_RADIUS_MIN, TURN_RADIUS_MAX) * half_tan;
    let tangent = js_clamp(wanted.min(tangent_cap), tangent_min, tangent_max);
    let radius = tangent / half_tan;
    if !(radius >= TURN_RADIUS_FLOOR) {
        return Some(straight);
    }
    let sign = if theta >= 0.0 { 1.0 } else { -1.0 };
    let ax = vx - tangent * d_in[0];
    let az = vz - tangent * d_in[1];
    let cx = ax + sign * radius * d_in[1];
    let cz = az - sign * radius * d_in[0];
    Some(Plan {
        kind: PlanKind::Arc,
        turn_side: sign as i8,
        theta,
        radius,
        arc_len: radius * sweep,
        tangent,
        along_in,
        entry_setback: tangent - along_in,
        exit_arc: tangent + along_out,
        speed_cap: (TURN_LATERAL_ACCEL * radius).sqrt(),
        lateral_step: f64::NAN,
        g: [cx, cz, ax - cx, az - cz, sign, yaw_in, 0.0, 0.0],
        lengths: None,
        junction_grade_seams: Default::default(),
    })
}

/// JS `clamp(value, min, max)` = `Math.min(max, Math.max(min, value))`.
#[inline]
pub fn js_clamp(v: f64, lo: f64, hi: f64) -> f64 {
    hi.min(lo.max(v))
}

fn cubic(g: &[f64; 8], t: f64) -> (f64, f64, f64, f64, f64, f64) {
    let (p0, p1, p2, p3) = ([g[0], g[1]], [g[2], g[3]], [g[4], g[5]], [g[6], g[7]]);
    let u = 1.0 - t;
    let x = u * u * u * p0[0] + 3.0 * u * u * t * p1[0] + 3.0 * u * t * t * p2[0] + t * t * t * p3[0];
    let z = u * u * u * p0[1] + 3.0 * u * u * t * p1[1] + 3.0 * u * t * t * p2[1] + t * t * t * p3[1];
    let dx = 3.0 * u * u * (p1[0] - p0[0]) + 6.0 * u * t * (p2[0] - p1[0]) + 3.0 * t * t * (p3[0] - p2[0]);
    let dz = 3.0 * u * u * (p1[1] - p0[1]) + 6.0 * u * t * (p2[1] - p1[1]) + 3.0 * t * t * (p3[1] - p2[1]);
    let ddx = 6.0 * u * (p2[0] - 2.0 * p1[0] + p0[0]) + 6.0 * t * (p3[0] - 2.0 * p2[0] + p1[0]);
    let ddz = 6.0 * u * (p2[1] - 2.0 * p1[1] + p0[1]) + 6.0 * t * (p3[1] - 2.0 * p2[1] + p1[1]);
    (x, z, dx, dz, ddx, ddz)
}

fn cubic_speed(g: &[f64; 8], t: f64) -> f64 {
    let c = cubic(g, t);
    hypot(c.2, c.3)
}

fn cubic_arc(g: &[f64; 8], a: f64, b: f64) -> f64 {
    let half = (b - a) / 2.0;
    let mid = (a + b) / 2.0;
    half * (0.3478548451374538
        * (cubic_speed(g, mid - half * 0.8611363115940526) + cubic_speed(g, mid + half * 0.8611363115940526))
        + 0.6521451548625461
            * (cubic_speed(g, mid - half * 0.3399810435848563) + cubic_speed(g, mid + half * 0.3399810435848563)))
}

/// `buildSmoothThroughMerge(edge, next, straight, tangentCap)`.
fn smooth_through_merge(inp: &[P2], in_len: f64, out: &[P2], out_len: f64, straight: &Plan, tangent_cap: f64) -> Option<Plan> {
    let in_end = inp[inp.len() - 1];
    let in_prev = inp[inp.len() - 2];
    let out_start = out[0];
    let out_next = out[1];
    let il = hypot(in_end[0] - in_prev[0], in_end[1] - in_prev[1]);
    let ol = hypot(out_next[0] - out_start[0], out_next[1] - out_start[1]);
    if !(il > 0.0) || !(ol > 0.0) {
        return None;
    }
    let d_in = [(in_end[0] - in_prev[0]) / il, (in_end[1] - in_prev[1]) / il];
    let d_out = [(out_next[0] - out_start[0]) / ol, (out_next[1] - out_start[1]) / ol];
    let room_in = (collinear_run_at(inp, 1) - TURN_TANGENT_EPS)
        .min(TURN_TANGENT_FRACTION * in_len)
        .min(TURN_SETBACK_MAX)
        .min(tangent_cap);
    let room_out = (collinear_run_at(out, -1) - TURN_TANGENT_EPS).min(TURN_TANGENT_FRACTION * out_len);
    let entry_setback = 8.0f64.min(room_in);
    let exit_arc = 8.0f64.min(room_out);
    if !(entry_setback >= 1.5 && exit_arc >= 2.0) {
        return None;
    }
    let p0 = [in_end[0] - d_in[0] * entry_setback, in_end[1] - d_in[1] * entry_setback];
    let p3 = [out_start[0] + d_out[0] * exit_arc, out_start[1] + d_out[1] * exit_arc];
    let mut plan = straight.clone();
    plan.entry_setback = entry_setback;
    plan.exit_arc = exit_arc;
    merge_curve(p0, p3, d_in, d_out, plan)
}

/// A mid-block transition between parallel lanes, tangent at both ends.
pub(crate) fn passing_plan(from: P2, to: P2, heading: P2, entry_setback: f64, exit_arc: f64) -> Option<Plan> {
    let template = Plan {
        kind: PlanKind::Merge,
        turn_side: 1,
        theta: 0.0,
        radius: 0.0,
        arc_len: 0.0,
        tangent: f64::NAN,
        along_in: 0.0,
        entry_setback,
        exit_arc,
        speed_cap: 0.0,
        lateral_step: (to[0] - from[0]) * heading[1] - (to[1] - from[1]) * heading[0],
        g: [0.0; 8],
        lengths: None,
        junction_grade_seams: Default::default(),
    };
    merge_curve(from, to, heading, heading, template)
}

const MERGE_HEADING_LIMIT_RAD: f64 = 0.4;

pub(crate) fn passing_distance(lateral: f64) -> f64 {
    // Give the cubic room to turn back to parallel within the merge heading cap.
    2.0 * lateral / MERGE_HEADING_LIMIT_RAD.tan()
}

fn merge_curve(p0: P2, p3: P2, d_in: P2, d_out: P2, mut plan: Plan) -> Option<Plan> {
    let chord = hypot(p3[0] - p0[0], p3[1] - p0[1]);
    if !(chord >= 8.0) {
        return None;
    }
    let control = chord / 3.0;
    let p1 = [p0[0] + d_in[0] * control, p0[1] + d_in[1] * control];
    let p2 = [p3[0] - d_out[0] * control, p3[1] - d_out[1] * control];
    let g = [p0[0], p0[1], p1[0], p1[1], p2[0], p2[1], p3[0], p3[1]];
    let mut lengths = Box::new([0.0f64; MERGE_SAMPLES + 1]);
    let mut max_curv = 0.0f64;
    for index in 0..=MERGE_SAMPLES {
        let t = index as f64 / MERGE_SAMPLES as f64;
        let point = cubic(&g, t);
        if index > 0 {
            lengths[index] = lengths[index - 1] + cubic_arc(&g, (index - 1) as f64 / MERGE_SAMPLES as f64, t);
        }
        let speed = hypot(point.2, point.3);
        if !(speed > 1e-6) || (point.2 * d_in[0] + point.3 * d_in[1]) / speed < MERGE_HEADING_LIMIT_RAD.cos() {
            return None;
        }
        max_curv = max_curv.max((point.2 * point.5 - point.3 * point.4).abs() / (speed * speed * speed));
    }
    if !(max_curv > 0.0 && max_curv <= 1.0 / TURN_RADIUS_MIN) {
        return None;
    }
    let radius = 1.0 / max_curv;
    plan.kind = PlanKind::Merge;
    plan.radius = radius;
    plan.arc_len = lengths[MERGE_SAMPLES];
    plan.tangent = f64::NAN;
    plan.along_in = 0.0;
    plan.speed_cap = (TURN_LATERAL_ACCEL * radius).sqrt();
    plan.g = g;
    plan.lengths = Some(lengths);
    Some(plan)
}

/// What `buildJunctionPlan` needs to know about the lane's other exits.
pub struct SiblingExit<'a> {
    pub points: &'a [P2],
    pub length: f64,
    pub allows_through: bool,
}

/// `buildJunctionPlan(edge, next, tangentCap)`.
#[allow(clippy::too_many_arguments)]
pub fn junction_plan(
    inp: &[P2],
    in_len: f64,
    out: &[P2],
    out_len: f64,
    siblings: &[SiblingExit],
    next_index: usize,
    painted_through_mapped: bool,
    merge_max_lateral: f64,
    tangent_cap: f64,
) -> Option<Plan> {
    let ordinary = corner_fillet(inp, in_len, out, out_len, tangent_cap)?;
    if ordinary.kind != PlanKind::Line
        || classify_movement(inp, out) != Movement::Through
        || ordinary.theta.abs() > TURN_MIN_ANGLE
    {
        return Some(ordinary);
    }
    if ordinary.lateral_step > 0.025 && ordinary.lateral_step <= THROUGH_MOVE_MAX_LATERAL_M {
        return Some(smooth_through_merge(inp, in_len, out, out_len, &ordinary, tangent_cap).unwrap_or(ordinary));
    }
    if ordinary.lateral_step <= THROUGH_MOVE_MAX_LATERAL_M {
        return Some(ordinary);
    }
    let mut nearest: Option<usize> = None;
    let mut nearest_lateral = f64::INFINITY;
    for (k, sib) in siblings.iter().enumerate() {
        if classify_movement(inp, sib.points) != Movement::Through || !sib.allows_through {
            continue;
        }
        let Some(cand) = corner_fillet(inp, in_len, sib.points, sib.length, f64::INFINITY) else { continue };
        if cand.kind != PlanKind::Line || cand.theta.abs() > TURN_MIN_ANGLE {
            continue;
        }
        if cand.lateral_step < nearest_lateral {
            nearest = Some(k);
            nearest_lateral = cand.lateral_step;
        }
    }
    if nearest != Some(next_index) || nearest_lateral > merge_max_lateral {
        return Some(ordinary);
    }
    if !painted_through_mapped && !(out_len >= TURN_SETBACK_MAX) {
        return Some(ordinary);
    }
    Some(smooth_through_merge(inp, in_len, out, out_len, &ordinary, tangent_cap).unwrap_or(ordinary))
}

impl Plan {
    /// `movementIsDrivable` for this plan.
    pub fn drivable(&self) -> bool {
        match self.kind {
            PlanKind::Arc | PlanKind::Merge => self.radius >= TURN_RADIUS_MIN,
            PlanKind::Line => self.theta.abs() <= TURN_MIN_ANGLE && self.lateral_step <= THROUGH_MOVE_MAX_LATERAL_M,
        }
    }

    /// `plan.poseAt(s)`: (x, z, yaw) at arc `s` along the movement.
    pub fn pose_at(&self, s: f64) -> (f64, f64, f64) {
        let g = &self.g;
        match self.kind {
            PlanKind::Line => {
                if self.arc_len == 0.0 {
                    return (g[0], g[1], g[4]);
                }
                let t = js_clamp(s / self.arc_len, 0.0, 1.0);
                (g[0] + g[2] * t, g[1] + g[3] * t, g[4] + g[5] * t)
            }
            PlanKind::Arc => {
                let (cx, cz, rx, rz, sign, yaw_in) = (g[0], g[1], g[2], g[3], g[4], g[5]);
                let phi = sign * js_clamp(s, 0.0, self.arc_len) / self.radius;
                let (sin, cos) = phi.sin_cos();
                (cx + rx * cos + rz * sin, cz - rx * sin + rz * cos, yaw_in + phi)
            }
            PlanKind::Merge => {
                let lengths = self.lengths.as_ref().expect("merge plan carries its length table");
                let distance = js_clamp(s, 0.0, self.arc_len);
                let (mut low, mut high) = (0usize, MERGE_SAMPLES);
                while low < high {
                    let middle = (low + high) >> 1;
                    if lengths[middle] < distance { low = middle + 1 } else { high = middle }
                }
                let index = low.max(1);
                let span = lengths[index] - lengths[index - 1];
                let from = (index - 1) as f64 / MERGE_SAMPLES as f64;
                let to = index as f64 / MERGE_SAMPLES as f64;
                let target = distance - lengths[index - 1];
                let mut t = from + (if span > 0.0 { target / span } else { 0.0 }) / MERGE_SAMPLES as f64;
                for _ in 0..3 {
                    let speed = cubic_speed(g, t);
                    if !(speed > 1e-9) {
                        break;
                    }
                    t = js_clamp(t - (cubic_arc(g, from, t) - target) / speed, from, to);
                }
                let p = cubic(g, t);
                (p.0, p.1, p.2.atan2(p.3))
            }
        }
    }
}

/// `junctionLanePoints(segment, lane)` in src/simulation/traffic/traffic-graph.js:
/// a lane polyline offset at every vertex (ends included, one-sided tangent),
/// a two-point centreline densified to three first, reversed for decreasing travel.
pub fn junction_lane_points(centre: &[P2], lateral_offset: f64, decreasing: bool) -> Vec<P2> {
    let mut source: Vec<P2> = centre.to_vec();
    if source.len() == 2 {
        source = vec![source[0], [(source[0][0] + source[1][0]) / 2.0, (source[0][1] + source[1][1]) / 2.0], source[1]];
    }
    let n = source.len();
    let mut pts: Vec<P2> = (0..n)
        .map(|i| {
            let prev = source[i.saturating_sub(1)];
            let next = source[(i + 1).min(n - 1)];
            let dx = next[0] - prev[0];
            let dz = next[1] - prev[1];
            let len = hypot(dx, dz);
            let len = if len == 0.0 { 1.0 } else { len };
            [source[i][0] + (dz / len) * lateral_offset, source[i][1] - (dx / len) * lateral_offset]
        })
        .collect();
    if decreasing {
        pts.reverse();
    }
    pts
}

/// Separating-axis overlap depth of two ground rectangles (`obbPenetration`): 0 when apart.
pub fn obb_penetration(a: (f64, f64, f64, f64, f64), b: (f64, f64, f64, f64, f64)) -> f64 {
    // (x, z, yaw, length, width)
    let extent = |p: (f64, f64, f64, f64, f64), ax: f64, az: f64| {
        let (s, c) = p.2.sin_cos();
        (p.4 / 2.0) * (ax * c - az * s).abs() + (p.3 / 2.0) * (ax * s + az * c).abs()
    };
    let mut min_overlap = f64::INFINITY;
    for pose in [a, b] {
        let (s, c) = pose.2.sin_cos();
        for (ax, az) in [(s, c), (c, -s)] {
            let d = ((b.0 - a.0) * ax + (b.1 - a.1) * az).abs();
            let overlap = extent(a, ax, az) + extent(b, ax, az) - d;
            if overlap <= 0.0 {
                return 0.0;
            }
            min_overlap = min_overlap.min(overlap);
        }
    }
    min_overlap
}

/// Clearance from a point to a ground rectangle (`obbClearance`): 0 on or inside.
pub fn obb_clearance(x: f64, z: f64, b: (f64, f64, f64, f64, f64)) -> f64 {
    let [forward, lateral] = obb_face_distances(x, z, b);
    hypot(forward.max(0.0), lateral.max(0.0))
}

pub(crate) fn obb_signed_clearance(x: f64, z: f64, b: (f64, f64, f64, f64, f64)) -> f64 {
    let [forward, lateral] = obb_face_distances(x, z, b);
    hypot(forward.max(0.0), lateral.max(0.0)) + forward.max(lateral).min(0.0)
}

fn obb_face_distances(x: f64, z: f64, b: (f64, f64, f64, f64, f64)) -> P2 {
    let dx = x - b.0;
    let dz = z - b.1;
    let (s, c) = b.2.sin_cos();
    let forward = dx * s + dz * c;
    let lateral = dx * c - dz * s;
    [forward.abs() - b.3 / 2.0, lateral.abs() - b.4 / 2.0]
}

/// First fraction of a ground-disc sweep that touches a rotated vehicle box.
pub(crate) fn disc_sweep_hit(start: P2, end: P2, radius: f64, body: (f64, f64, f64, f64, f64), allow_escape: bool) -> Option<f64> {
    let (sin, cos) = body.2.sin_cos();
    let local = |[x, z]: P2| {
        let (dx, dz) = (x - body.0, z - body.1);
        [dx * sin + dz * cos, dx * cos - dz * sin]
    };
    let (from, to) = (local(start), local(end));
    let delta = [to[0] - from[0], to[1] - from[1]];
    let half = [body.3 / 2.0, body.4 / 2.0];
    let signed_clearance = |p: P2| {
        let (x, z) = (p[0].abs() - half[0], p[1].abs() - half[1]);
        x.max(0.0).hypot(z.max(0.0)) + x.max(z).min(0.0)
    };
    let clearance = signed_clearance(from);
    if clearance <= radius {
        return if allow_escape && signed_clearance(to) > clearance { None } else { Some(0.0) };
    }
    let slab = |bounds: P2| {
        let (mut enter, mut exit) = (0.0_f64, 1.0_f64);
        for axis in 0..2 {
            if delta[axis] == 0.0 {
                if from[axis].abs() > bounds[axis] { return None; }
                continue;
            }
            let a = (-bounds[axis] - from[axis]) / delta[axis];
            let b = (bounds[axis] - from[axis]) / delta[axis];
            enter = enter.max(a.min(b));
            exit = exit.min(a.max(b));
        }
        (enter <= exit).then_some(enter)
    };
    slab([half[0] + radius, half[1] + radius])?;
    // The box expanded by a disc is two strips and four circular corners.
    let strips = [slab([half[0] + radius, half[1]]), slab([half[0], half[1] + radius])];
    let speed_sq = delta[0] * delta[0] + delta[1] * delta[1];
    let corners = [-1.0, 1.0].into_iter().flat_map(|x| [-1.0, 1.0].map(|z| [x * half[0], z * half[1]]));
    let circles = corners.filter_map(|corner| {
        if speed_sq == 0.0 { return None; }
        let m = [from[0] - corner[0], from[1] - corner[1]];
        let towards = m[0] * delta[0] + m[1] * delta[1];
        let discriminant = towards * towards - speed_sq * (m[0] * m[0] + m[1] * m[1] - radius * radius);
        if towards >= 0.0 || discriminant < 0.0 { return None; }
        let t = (-towards - discriminant.sqrt()) / speed_sq;
        (0.0..=1.0).contains(&t).then_some(t)
    });
    strips.into_iter().flatten().chain(circles).min_by(f64::total_cmp)
}

/// Do segments ab and cd cross?
pub fn segments_cross(a: P2, b: P2, c: P2, d: P2) -> bool {
    let o = |p: P2, q: P2, r: P2| (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
    let d1 = o(c, d, a);
    let d2 = o(c, d, b);
    let d3 = o(a, b, c);
    let d4 = o(a, b, d);
    ((d1 > 0.0) != (d2 > 0.0)) && ((d3 > 0.0) != (d4 > 0.0))
}
