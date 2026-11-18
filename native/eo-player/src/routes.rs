//! Scripted routes over the real city, for the offscreen bench and the tests:
//! a walk along a whole street (many blocks, its steepest stretch included)
//! the steepest street, and a drive. Built from `sf-city.json` roads and the
//! elevation grid.

use glam::DVec3;
use std::collections::BTreeSet;

use crate::player::{Mode, Player, StepInput};
use crate::world::CollisionWorld;

/// A street's centreline, ordered along its main direction and resampled
/// every `step_m` (the street's ways merged; a two-carriageway boulevard is
/// averaged across).
pub fn street_polyline(city: &eo_data::City, name: &str, step_m: f64) -> Vec<[f64; 2]> {
    let pts: Vec<[f64; 2]> = city.roads.iter().filter(|r| r.name == name && eo_data::standards::is_vehicle_way(&r.highway) && !r.tunnel).flat_map(|r| r.points64.iter().copied()).collect();
    if pts.len() < 2 {
        return vec![];
    }
    let n = pts.len() as f64;
    let (mx, mz) = (pts.iter().map(|p| p[0]).sum::<f64>() / n, pts.iter().map(|p| p[1]).sum::<f64>() / n);
    let (mut sxx, mut sxz, mut szz) = (0.0, 0.0, 0.0);
    for p in &pts {
        let (dx, dz) = (p[0] - mx, p[1] - mz);
        sxx += dx * dx;
        sxz += dx * dz;
        szz += dz * dz;
    }
    let angle = 0.5 * (2.0 * sxz).atan2(sxx - szz);
    let (ux, uz) = (angle.cos(), angle.sin());
    let along = |p: &[f64; 2]| (p[0] - mx) * ux + (p[1] - mz) * uz;
    let across = |p: &[f64; 2]| -(p[0] - mx) * uz + (p[1] - mz) * ux;
    // Densify each way's segments so buckets are filled.
    let mut dense = vec![];
    for r in city.roads.iter().filter(|r| r.name == name && eo_data::standards::is_vehicle_way(&r.highway) && !r.tunnel) {
        for w in r.points64.windows(2) {
            let l = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
            let k = (l / 2.0).ceil().max(1.0) as usize;
            for i in 0..k {
                let t = i as f64 / k as f64;
                dense.push([w[0][0] + (w[1][0] - w[0][0]) * t, w[0][1] + (w[1][1] - w[0][1]) * t]);
            }
        }
    }
    let (lo, hi) = dense.iter().map(along).fold((f64::MAX, f64::MIN), |a, s| (a.0.min(s), a.1.max(s)));
    let mut out = vec![];
    let mut s = lo;
    while s <= hi {
        let near: Vec<f64> = dense.iter().filter(|p| (along(p) - s).abs() <= step_m * 0.75).map(across).collect();
        if !near.is_empty() {
            let c = near.iter().sum::<f64>() / near.len() as f64;
            out.push([mx + ux * s - uz * c, mz + uz * s + ux * c]);
        }
        s += step_m;
    }
    out
}

pub fn polyline_length(p: &[[f64; 2]]) -> f64 {
    p.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum()
}

/// Steepest grade over any `window_m` stretch of the polyline (rise / run).
pub fn steepest_grade(p: &[[f64; 2]], elevation: &eo_data::ElevationGrid, window_m: f64) -> (f64, usize) {
    let mut acc = vec![0.0];
    for w in p.windows(2) {
        acc.push(acc.last().unwrap() + (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]));
    }
    let h: Vec<f64> = p.iter().map(|q| elevation.ground(q[0], q[1])).collect();
    let mut best = (0.0, 0);
    let mut j = 0;
    for i in 0..p.len() {
        while j < p.len() && acc[j] - acc[i] < window_m {
            j += 1;
        }
        if j >= p.len() {
            break;
        }
        let g = (h[j] - h[i]).abs() / (acc[j] - acc[i]);
        if g > best.0 {
            best = (g, i);
        }
    }
    best
}

/// The distinct named streets a route crosses (its intersections).
pub fn cross_streets(city: &eo_data::City, route: &[[f64; 2]], own: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let seg_hit = |a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]| {
        let cross = |o: [f64; 2], p: [f64; 2], q: [f64; 2]| (p[0] - o[0]) * (q[1] - o[1]) - (p[1] - o[1]) * (q[0] - o[0]);
        let (d1, d2, d3, d4) = (cross(c, d, a), cross(c, d, b), cross(a, b, c), cross(a, b, d));
        (d1 > 0.0) != (d2 > 0.0) && (d3 > 0.0) != (d4 > 0.0)
    };
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for p in route {
        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
    }
    for r in &city.roads {
        if r.name.is_empty() || r.name == own || !eo_data::standards::is_vehicle_way(&r.highway) {
            continue;
        }
        if !r.points64.iter().any(|p| p[0] >= lo[0] - 50.0 && p[0] <= hi[0] + 50.0 && p[1] >= lo[1] - 50.0 && p[1] <= hi[1] + 50.0) {
            continue;
        }
        'way: for w in r.points64.windows(2) {
            for s in route.windows(2) {
                if seg_hit(s[0], s[1], w[0], w[1]) {
                    out.insert(r.name.clone());
                    break 'way;
                }
            }
        }
    }
    out
}

/// Every named vehicle street with its length and steepest 30 m grade, steepest first.
pub fn steep_streets(city: &eo_data::City, elevation: &eo_data::ElevationGrid, min_len_m: f64) -> Vec<(String, f64, f64)> {
    let names: BTreeSet<&str> = city.roads.iter().filter(|r| !r.name.is_empty() && eo_data::standards::is_vehicle_way(&r.highway)).map(|r| r.name.as_str()).collect();
    let mut out = vec![];
    for n in names {
        let p = street_polyline(city, n, 10.0);
        let len = polyline_length(&p);
        if len < min_len_m {
            continue;
        }
        let (g, _) = steepest_grade(&p, elevation, 30.0);
        out.push((n.to_string(), len, g));
    }
    out.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());
    out
}

fn street_way(r: &eo_data::Road) -> bool {
    eo_data::standards::is_vehicle_way(&r.highway) && !r.bridge && !r.tunnel && !r.highway.starts_with("motorway") && !r.highway.starts_with("trunk")
}

/// Every surface street way, with its steepest grade over `window_m` measured
/// within the way (no merging across gaps): (road index, grade, at point).
pub fn steep_ways(city: &eo_data::City, elevation: &eo_data::ElevationGrid, window_m: f64) -> Vec<(usize, f64, [f64; 2])> {
    let mut out = vec![];
    for (i, r) in city.roads.iter().enumerate() {
        if !street_way(r) || r.points64.len() < 2 {
            continue;
        }
        let mut dense = vec![];
        for w in r.points64.windows(2) {
            let l = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
            let k = (l / 5.0).ceil().max(1.0) as usize;
            for j in 0..k {
                let t = j as f64 / k as f64;
                dense.push([w[0][0] + (w[1][0] - w[0][0]) * t, w[0][1] + (w[1][1] - w[0][1]) * t]);
            }
        }
        dense.push(*r.points64.last().unwrap());
        let (g, at) = steepest_grade(&dense, elevation, window_m);
        if g > 0.0 {
            out.push((i, g, dense[at]));
        }
    }
    out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    out
}

fn node_key(p: [f64; 2]) -> (i64, i64) {
    ((p[0] * 100.0).round() as i64, (p[1] * 100.0).round() as i64)
}

/// A walk/drive route through connected street ways: from the middle of
/// `start` (a road index), extended forward and backward through the way
/// network (same name preferred, else the straightest continuation) until each
/// half is `half_len_m` long. Returns the polyline and the intersection nodes
/// it passes (nodes shared with a way of another street).
pub fn chain_route(city: &eo_data::City, start: usize, half_len_m: f64) -> (Vec<[f64; 2]>, Vec<[f64; 2]>) {
    let mut nodes: std::collections::HashMap<(i64, i64), Vec<(usize, usize)>> = std::collections::HashMap::new();
    for (i, r) in city.roads.iter().enumerate() {
        if !street_way(r) {
            continue;
        }
        for (j, p) in r.points64.iter().enumerate() {
            nodes.entry(node_key(*p)).or_default().push((i, j));
        }
    }
    let walk = |way: usize, from: usize, dir: i64| -> Vec<[f64; 2]> {
        let mut out: Vec<[f64; 2]> = vec![];
        let mut used: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
        let (mut w, mut j, mut d) = (way, from as i64, dir);
        let mut length = 0.0;
        out.push(city.roads[w].points64[j as usize]);
        while length < half_len_m {
            let pts = &city.roads[w].points64;
            let nj = j + d;
            if nj >= 0 && (nj as usize) < pts.len() && used.insert((w, j.min(nj) as usize)) {
                let a = pts[j as usize];
                let b = pts[nj as usize];
                length += (b[0] - a[0]).hypot(b[1] - a[1]);
                out.push(b);
                j = nj;
                // At a node shared with other ways, keep the straightest same-name continuation.
                let here = node_key(b);
                let heading = (b[0] - a[0]).atan2(b[1] - a[1]);
                let name = &city.roads[w].name;
                let mut best: Option<(f64, usize, i64, i64)> = None;
                for &(ow, oj) in nodes.get(&here).map(|v| v.as_slice()).unwrap_or(&[]) {
                    for od in [-1i64, 1] {
                        let nn = oj as i64 + od;
                        let opts = &city.roads[ow].points64;
                        if nn < 0 || nn as usize >= opts.len() || used.contains(&(ow, (oj as i64).min(nn) as usize)) {
                            continue;
                        }
                        let c = opts[nn as usize];
                        let h2 = (c[0] - b[0]).atan2(c[1] - b[1]);
                        let turn = (h2 - heading).sin().atan2((h2 - heading).cos()).abs();
                        if turn > 0.6 {
                            continue;
                        }
                        let score = turn + if &city.roads[ow].name == name { 0.0 } else { 0.3 };
                        if best.map(|x| score < x.0).unwrap_or(true) {
                            best = Some((score, ow, oj as i64, od));
                        }
                    }
                }
                if let Some((_, ow, oj, od)) = best {
                    w = ow;
                    j = oj;
                    d = od;
                }
            } else {
                break;
            }
        }
        out
    };
    let mid = city.roads[start].points64.len() / 2;
    let fwd = walk(start, mid, 1);
    let mut back = walk(start, mid, -1);
    back.reverse();
    back.pop();
    back.extend(fwd);
    // a route that ends where its street goes onto a bridge (a way the
    // chain does not follow) ends at the last vertex before that node: the
    // ground under a bridge node is the abutment or the cut it spans (Mount
    // Vernon Avenue over the freeway: a 44.6 degree bank at real heights),
    // which no street surface stands on.
    let bridge_nodes: std::collections::HashSet<(i64, i64)> = city.roads.iter().filter(|r| r.bridge && eo_data::standards::is_vehicle_way(&r.highway)).flat_map(|r| r.points64.iter().map(|p| node_key(*p))).collect();
    while back.len() > 2 && bridge_nodes.contains(&node_key(back[0])) {
        back.remove(0);
    }
    while back.len() > 2 && bridge_nodes.contains(&node_key(*back.last().unwrap())) {
        back.pop();
    }
    let crossings = intersections_on(city, &back);
    (back, crossings)
}

/// Where other street ways meet the route at an angle (crossings and T
/// junctions), merged within 15 m: the intersections a walker crosses.
pub fn intersections_on(city: &eo_data::City, route: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for p in route {
        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
    }
    let mut found: Vec<([f64; 2], f64)> = vec![];
    for r in &city.roads {
        if !street_way(r) || !r.points64.iter().any(|p| p[0] >= lo[0] - 30.0 && p[0] <= hi[0] + 30.0 && p[1] >= lo[1] - 30.0 && p[1] <= hi[1] + 30.0) {
            continue;
        }
        for w in r.points64.windows(2) {
            let (wa, wb) = (w[0], w[1]);
            let wl = (wb[0] - wa[0]).hypot(wb[1] - wa[1]);
            if wl < 1e-6 {
                continue;
            }
            for (k, s) in route.windows(2).enumerate() {
                let (a, b) = (s[0], s[1]);
                let sl = (b[0] - a[0]).hypot(b[1] - a[1]);
                if sl < 1e-6 {
                    continue;
                }
                let cos = ((wb[0] - wa[0]) * (b[0] - a[0]) + (wb[1] - wa[1]) * (b[1] - a[1])) / (wl * sl);
                if cos.abs() > 0.85 {
                    continue; // along the route, not across it
                }
                // Segment intersection with a 1 m tolerance on the way's ends.
                let den = (b[0] - a[0]) * (wb[1] - wa[1]) - (b[1] - a[1]) * (wb[0] - wa[0]);
                if den.abs() < 1e-9 {
                    continue;
                }
                let t = ((wa[0] - a[0]) * (wb[1] - wa[1]) - (wa[1] - a[1]) * (wb[0] - wa[0])) / den;
                let u = ((wa[0] - a[0]) * (b[1] - a[1]) - (wa[1] - a[1]) * (b[0] - a[0])) / den;
                let tol_t = 1.0 / sl;
                let tol_u = 1.0 / wl;
                if t >= -tol_t && t <= 1.0 + tol_t && u >= -tol_u && u <= 1.0 + tol_u {
                    let p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
                    let along = k as f64 + t;
                    if !found.iter().any(|(q, _)| (q[0] - p[0]).hypot(q[1] - p[1]) < 15.0) {
                        found.push((p, along));
                    }
                }
            }
        }
    }
    found.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    found.into_iter().map(|f| f.0).collect()
}

pub struct Route {
    pub path: Vec<[f64; 2]>,
    pub intersections: Vec<[f64; 2]>,
    /// Steepest 20 m grade on the route.
    pub steepest_grade: f64,
    pub start_way: usize,
}

/// Grades above this over 20 m are survey/stair artefacts, not a street a
/// car or walker uses (SF's steepest drivable blocks are about 31.5 %).
pub const MAX_PLAUSIBLE_GRADE: f64 = 0.42;

/// The walk: chained from the steepest plausible named street way, about
/// 2.6 km long. Deterministic for the data.
pub fn walk_route(city: &eo_data::City, elevation: &eo_data::ElevationGrid) -> Route {
    route_from_steepest(city, elevation, 3000.0)
}

/// The steepest plausible city street in the data, chained through the way
/// network to at least 1 km (its intersections may be fewer than 21).
pub fn steepest_route(city: &eo_data::City, elevation: &eo_data::ElevationGrid) -> Route {
    let ways = steep_ways(city, elevation, 20.0);
    for (i, g, _) in &ways {
        let h = city.roads[*i].highway.as_str();
        if *g > MAX_PLAUSIBLE_GRADE || city.roads[*i].name.is_empty() || !matches!(h, "residential" | "tertiary" | "secondary" | "primary" | "unclassified" | "living_street") {
            continue;
        }
        let (path, intersections) = chain_route(city, *i, 1500.0);
        if polyline_length(&path) >= 1000.0 {
            return Route { steepest_grade: *g, path, intersections, start_way: *i };
        }
    }
    panic!("no steep street route");
}

fn route_from_steepest(city: &eo_data::City, elevation: &eo_data::ElevationGrid, half: f64) -> Route {
    let ways = steep_ways(city, elevation, 20.0);
    let city_street = |h: &str| matches!(h, "residential" | "tertiary" | "secondary" | "primary" | "unclassified" | "living_street");
    // The steepest plausible city street whose way network carries a route of
    // the wanted length through at least 21 intersections (20 blocks).
    let mut chosen = None;
    for (i, g, _) in &ways {
        if *g > MAX_PLAUSIBLE_GRADE || city.roads[*i].name.is_empty() || !city_street(&city.roads[*i].highway) {
            continue;
        }
        let (path, inter) = chain_route(city, *i, half);
        if polyline_length(&path) >= 2000.0 && inter.len() >= 21 {
            chosen = Some((*i, path, inter));
            break;
        }
    }
    let (start, path, intersections) = chosen.expect("a steep street route");
    let mut dense = vec![];
    for w in path.windows(2) {
        let l = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
        let k = (l / 5.0).ceil().max(1.0) as usize;
        for j in 0..k {
            let t = j as f64 / k as f64;
            dense.push([w[0][0] + (w[1][0] - w[0][0]) * t, w[0][1] + (w[1][1] - w[0][1]) * t]);
        }
    }
    let (g, _) = steepest_grade(&dense, elevation, 20.0);
    Route { path, intersections, steepest_grade: g, start_way: start }
}

/// Follows waypoints on foot: camera yaw toward the next one, stick forward, run.
pub struct WalkPilot {
    pub waypoints: Vec<[f64; 2]>,
    pub next: usize,
    pub sprint: bool,
    /// A waypoint counts as reached within this distance (2 m).
    pub reach: f64,
}

impl WalkPilot {
    pub fn new(waypoints: Vec<[f64; 2]>, sprint: bool) -> Self {
        Self { waypoints, next: 1, sprint, reach: 2.0 }
    }
    pub fn done(&self) -> bool {
        self.next >= self.waypoints.len()
    }
    /// Set the player's yaw and return this step's input.
    pub fn input(&mut self, player: &mut Player, world: &CollisionWorld) -> StepInput {
        let p = player.position(world);
        while self.next < self.waypoints.len() {
            let w = self.waypoints[self.next];
            if (w[0] - p.x).hypot(w[1] - p.z) < self.reach {
                self.next += 1;
            } else {
                break;
            }
        }
        if self.done() {
            return StepInput::default();
        }
        let w = self.waypoints[self.next];
        player.yaw = (w[0] - p.x).atan2(w[1] - p.z);
        StepInput { move_y: 1.0, sprint: self.sprint, ..Default::default() }
    }
}


/// The drive: the same route as the walk.
pub fn drive_route(city: &eo_data::City, elevation: &eo_data::ElevationGrid) -> Route {
    walk_route(city, elevation)
}

/// Drives a path: pure pursuit on steering (analog), throttle/brake to a target speed.
pub struct DrivePilot {
    pub path: Vec<[f64; 2]>,
    pub target_mps: f64,
    s: f64,
}

impl DrivePilot {
    pub fn new(path: Vec<[f64; 2]>, target_mps: f64) -> Self {
        Self { path, target_mps, s: 0.0 }
    }
    pub fn done(&self) -> bool {
        self.s >= polyline_length(&self.path) - 12.0
    }
    fn point_at(&self, s: f64) -> [f64; 2] {
        let mut left = s.max(0.0);
        for w in self.path.windows(2) {
            let l = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
            if left <= l {
                let t = left / l.max(1e-9);
                return [w[0][0] + (w[1][0] - w[0][0]) * t, w[0][1] + (w[1][1] - w[0][1]) * t];
            }
            left -= l;
        }
        *self.path.last().unwrap()
    }
    pub fn input(&mut self, player: &Player, world: &CollisionWorld) -> StepInput {
        let Mode::Driving(i) = player.mode else { return StepInput::default() };
        let v = &player.vehicles[i];
        let p = v.datum(world);
        // Progress: the nearest arc length ahead of the last one.
        let mut best = (f64::MAX, self.s);
        let mut s = self.s;
        while s < self.s + 30.0 {
            let q = self.point_at(s);
            let d = (q[0] - p.x).hypot(q[1] - p.z);
            if d < best.0 {
                best = (d, s);
            }
            s += 0.5;
        }
        self.s = best.1;
        let speed = v.forward_speed(world);
        let look = 6.0 + speed.abs() * 0.6;
        let t = self.point_at(self.s + look);
        let yaw = v.yaw(world);
        let want = (t[0] - p.x).atan2(t[1] - p.z);
        let err = (want - yaw).sin().atan2((want - yaw).cos());
        // Positive steer turns left = toward +yaw.
        let steer = (err * 2.0).clamp(-0.98, 0.98);
        let dv = self.target_mps - speed;
        StepInput { throttle: (dv * 0.4).clamp(0.0, 1.0), brake: (-dv * 0.3).clamp(0.0, 1.0), steer, analog_steer: true, ..Default::default() }
    }
}

/// Feet height above the bare ground (negative: below it).
pub fn clearance(world: &CollisionWorld, p: DVec3) -> f64 {
    p.y - world.ground.ground_y(p.x, p.z)
}
