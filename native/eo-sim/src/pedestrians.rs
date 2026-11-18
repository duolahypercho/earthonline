//! Walkers on the footway network: a port of TrafficSim's ambient walker
//! (src/citygen/traffic.js `updatePedestrian`, `updateKerbWait`,
//! `considerCrossing`, `continueAlongFootway`, `routeAtPathEnd`,
//! `adoptFootwayPath`, `beginFootwayLink`, `advancePedestrianActivity`,
//! `ambulatoryActivityFor`, `pausedFacing`, `crossingIsClearOfTraffic`,
//! `signalCrossingMayStart`).
//!
//! Every choice a walker makes - which corner to take, whether to cross, how
//! long to pause and doing what - is `keyedRandom` of the walker's own key and
//! its own leg counters, exactly as in JS, so a walker's schedule is the same
//! whatever order or thread steps it. Walkers step independently of each
//! other (the JS ambient pool has no walker-walker interaction; the crowd
//! separation lives in the Rapier bridge, which is not ported), so the crowd
//! step parallelises trivially.
//!
//! A walker on a footway reads its polyline from the network; a corner or
//! crossing leg (two or three points) is held inline, so a walker owns no heap
//! memory.

use crate::consts::*;
use crate::geom::{self, P2, js_clamp, normalize_angle};
use crate::network::{FootwayId, JunctionId, NONE, Network};
use crate::rng::{walker_draw, walker_draw_salt};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LegKind {
    Corner,
    Cross,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Facing {
    Keep,
    Across,
    Inward,
    Partner,
}

#[derive(Clone, Copy, Debug)]
pub struct Leg {
    pub pts: [P2; 3],
    pub n: u8,
    pub cum: [f64; 3],
    pub total: f64,
    pub kind: LegKind,
    pub next: FootwayId,
    pub next_end: u8,
    pub arc: f64,
    pub dir: i8,
}

#[derive(Clone, Copy, Debug)]
pub struct KerbWait {
    pub signal: JunctionId,
    pub link: [P2; 2],
    pub walk: Option<[P2; 3]>,
    pub next: FootwayId,
    pub next_end: u8,
    pub arc: f64,
    pub dir: i8,
    pub lateral: f64,
    pub length: f64,
    pub elapsed: f64,
}

#[derive(Clone, Debug)]
pub struct Walker {
    /// `ambient-<key>`: the walker's identity for every keyed draw.
    pub key: u32,
    pub footway: FootwayId,
    pub leg: Option<Leg>,
    pub s: f64,
    pub dir: i8,
    pub speed: f64,
    pub lateral: f64,
    pub lateral_hold: Option<f64>,
    pub road_side: i8,
    pub group: bool,
    pub activity: Activity,
    pub ambulatory: Option<Activity>,
    ambulatory_hour: i32,
    ambulatory_draw: Option<Activity>,
    pub activity_timer: f64,
    pub activity_leg: u32,
    pub facing: Facing,
    pub kerb: Option<KerbWait>,
    pub cross_node: u32,
    pub cross_leg: u32,
    pub route_leg: u32,
    pub ground_speed: f64,
    pub yaw: f64,
    pub gait: f64,
    pub gait_freq: f64,
    pub far: bool,
    pub lod_debt: f64,
}

#[derive(Clone, Debug, Default)]
pub struct CrowdStats {
    pub crossings_started: u64,
    pub crossings_completed: u64,
    pub kerb_waits: u64,
    pub kerb_waits_abandoned: u64,
    pub continuations: u64,
    pub reversals: u64,
    pub declined_off_band: u64,
    /// Signalised crossings started while the crossing's indication was not WALK (must stay 0).
    pub starts_outside_walk: u64,
    pub spawned: u64,
    pub retired: u64,
}

#[derive(Default)]
pub struct Crowd {
    pub walkers: Vec<Walker>,
    pub x: Vec<f64>,
    pub z: Vec<f64>,
    pub stats: CrowdStats,
    pub next_key: u32,
}

/// What a walker's step reads from the world.
pub struct WalkCtx<'a> {
    pub net: &'a Network,
    pub time: f64,
    pub hour: f64,
    /// The fleet's poses, for the traffic-gap test at an unsignalised kerb.
    pub car_x: &'a [f64],
    pub car_z: &'a [f64],
    pub car_yaw: &'a [f64],
    pub car_speed: &'a [f64],
}

/// `cadenceForSpeed(speed)` at the reference height, steps/s (gait-activity-contract.js).
pub fn cadence_for_speed(speed: f64) -> f64 {
    // GAIT_CADENCE: intercept 1.02, slope 0.62, clamp [0.70, 3.20]; reference leg = 1.75 * 0.526.
    js_clamp(1.02 + 0.62 * speed.max(0.0), 0.70, 3.20)
}

impl Walker {
    #[allow(clippy::too_many_arguments)]
    pub fn new(key: u32, footway: FootwayId, s: f64, dir: i8, speed: f64, lateral: f64, road_side: i8, group: bool, gait: f64) -> Self {
        Walker {
            key,
            footway,
            leg: None,
            s,
            dir,
            speed,
            lateral,
            lateral_hold: None,
            road_side,
            group,
            activity: Activity::Walk,
            ambulatory: None,
            ambulatory_hour: -1,
            ambulatory_draw: None,
            activity_timer: f64::NAN,
            activity_leg: 0,
            facing: Facing::Keep,
            kerb: None,
            cross_node: NONE,
            cross_leg: 0,
            route_leg: 0,
            ground_speed: 0.0,
            yaw: f64::NAN,
            gait,
            gait_freq: std::f64::consts::PI * cadence_for_speed(speed),
            far: false,
            lod_debt: 0.0,
        }
    }

    #[inline]
    fn total(&self, net: &Network) -> f64 {
        match &self.leg {
            Some(l) => l.total,
            None => net.footways[self.footway as usize].length,
        }
    }

    /// Position and segment direction at arc `s` of the current path.
    #[inline]
    fn at(&self, net: &Network, s: f64) -> (f64, f64, f64, f64) {
        match &self.leg {
            Some(l) => {
                let (x, z, dx, dz, _) = geom::at_arc(&l.pts[..l.n as usize], &l.cum[..l.n as usize], s);
                (x, z, dx, dz)
            }
            None => {
                let f = &net.footways[self.footway as usize];
                let (x, z, dx, dz, _) = geom::at_arc(&f.pts, &f.cum, s);
                (x, z, dx, dz)
            }
        }
    }

    fn keeps_walking(&self) -> bool {
        self.activity == Activity::Walk || (self.ambulatory.is_some() && Some(self.activity) == self.ambulatory)
    }

    /// `ambulatoryActivityFor`.
    fn ambulatory_for(&mut self, hour: f64) -> Option<Activity> {
        let bucket = if hour.is_finite() { (((hour % 24.0) + 24.0) % 24.0).floor() as i32 } else { 12 };
        if self.ambulatory_hour == bucket {
            return self.ambulatory_draw;
        }
        let mut roll = walker_draw(self.key, "ambulatory-", bucket as u32);
        let mut drawn = None;
        for &(activity, weight, needs_company) in AMBULATORY_TABLE.iter() {
            if needs_company && !self.group {
                continue;
            }
            roll -= weight;
            if roll <= 0.0 {
                drawn = Some(activity);
                break;
            }
        }
        self.ambulatory_hour = bucket;
        self.ambulatory_draw = drawn;
        drawn
    }

    /// `advancePedestrianActivity`: returns whether the walker moves this step.
    fn advance_activity(&mut self, net: &Network, hour: f64, dt: f64) -> bool {
        if self.activity_timer.is_nan() {
            self.activity_leg = 0;
            self.activity_timer = 4.0 + walker_draw_salt(self.key, "seed") * 24.0;
            self.ambulatory = self.ambulatory_for(hour);
            self.activity = self.ambulatory.unwrap_or(Activity::Walk);
            self.facing = Facing::Keep;
        }
        self.activity_timer -= dt;
        if self.activity_timer > 0.0 {
            if self.activity == Activity::Walk && self.ambulatory.is_some() {
                self.activity = self.ambulatory.unwrap();
                self.facing = Facing::Keep;
            }
            return self.keeps_walking();
        }
        self.activity_leg += 1;
        let leg = self.activity_leg;
        if !self.keeps_walking() {
            let [lo, hi] = WALK_LEG_SECONDS;
            self.activity_timer = lo + walker_draw(self.key, "walk-", leg) * (hi - lo);
            self.ambulatory = self.ambulatory_for(hour);
            self.activity = self.ambulatory.unwrap_or(Activity::Walk);
            self.facing = Facing::Keep;
            return true;
        }
        self.ambulatory = None;
        let total = self.total(net);
        let near_corner = self.s.min(total - self.s) <= 6.0;
        let mut weight_total = 0.0;
        for e in PAUSE_TABLE.iter() {
            if !e.4 || near_corner {
                weight_total += e.1;
            }
        }
        let mut roll = walker_draw(self.key, "act-", leg) * weight_total;
        let mut chosen = PAUSE_TABLE[PAUSE_TABLE.len() - 1];
        for e in PAUSE_TABLE.iter() {
            if e.4 && !near_corner {
                continue;
            }
            roll -= e.1;
            if roll <= 0.0 {
                chosen = *e;
                break;
            }
        }
        // `pool[pool.length - 1]` is the last ADMITTED entry; Carry is never corner-only, so it is the same.
        self.activity = chosen.0;
        self.facing = match chosen.0 {
            Activity::Wait => Facing::Across,
            Activity::Talk => Facing::Partner,
            Activity::Browse => Facing::Inward,
            _ => Facing::Keep,
        };
        self.activity_timer = chosen.2 + walker_draw(self.key, "dur-", leg) * (chosen.3 - chosen.2);
        false
    }
}

impl Crowd {
    pub fn push(&mut self, w: Walker) {
        self.walkers.push(w);
        self.x.push(0.0);
        self.z.push(0.0);
    }

    pub fn remove(&mut self, i: usize) {
        self.walkers.swap_remove(i);
        self.x.swap_remove(i);
        self.z.swap_remove(i);
        self.stats.retired += 1;
    }

    /// hand walker `i` off to another region (not counted as retired):
    /// its state and position; the last walker takes its index.
    pub fn take(&mut self, i: usize) -> (Walker, f64, f64) {
        let w = self.walkers.swap_remove(i);
        (w, self.x.swap_remove(i), self.z.swap_remove(i))
    }

    /// take in a walker handed off by another region on the same
    /// network, keeping its key (so its keyed draws carry on) and its leg.
    /// Keys stay unique only if the regions number from disjoint bases
    /// (`Region::with_id_base`).
    pub fn adopt(&mut self, w: Walker, x: f64, z: f64) {
        self.walkers.push(w);
        self.x.push(x);
        self.z.push(z);
    }

    /// The occupied crosswalks (walkers on a crossing leg), for the cars.
    pub fn bands(&self, out: &mut Vec<crate::vehicles::Band>) {
        out.clear();
        for w in &self.walkers {
            if let Some(l) = &w.leg {
                if l.kind == LegKind::Cross {
                    out.push(crate::vehicles::Band { a: l.pts[l.n as usize - 2], b: l.pts[l.n as usize - 1] });
                }
            }
        }
    }

    /// One fixed step of every walker (`updatePedestrians`).
    pub fn step(&mut self, ctx: &WalkCtx, dt: f64, parity: u64) {
        let mut stats = std::mem::take(&mut self.stats);
        for i in 0..self.walkers.len() {
            let w = &mut self.walkers[i];
            let mut step_dt = dt;
            if w.far {
                w.lod_debt += dt;
                if (i as u64 + parity) % SIM_TIER_FAR_EVERY as u64 != 0 {
                    continue;
                }
                step_dt = w.lod_debt;
                w.lod_debt = 0.0;
            } else {
                step_dt += w.lod_debt;
                w.lod_debt = 0.0;
            }
            if !ctx.net.footways[w.footway as usize].alive {
                continue;
            }
            step_walker(ctx, w, step_dt, &mut stats);
            let (x, z) = position(ctx.net, w);
            self.x[i] = x;
            self.z[i] = z;
        }
        self.stats = stats;
    }

    /// Recompute positions without stepping (after spawning).
    pub fn refresh_positions(&mut self, net: &Network) {
        for i in 0..self.walkers.len() {
            let (x, z) = position(net, &self.walkers[i]);
            self.x[i] = x;
            self.z[i] = z;
        }
    }
}

/// The drawn ground position: path position, lane offset and crosswalk keep-right.
fn position(net: &Network, w: &Walker) -> (f64, f64) {
    let (mut x, mut z, dx, dz) = w.at(net, w.s);
    let mut keep_side = 0.0;
    if let Some(l) = &w.leg {
        if l.kind == LegKind::Cross && l.total > 2.0 * CROSS_KEEP_RAMP_M {
            let ramp = (w.s / CROSS_KEEP_RAMP_M).min((l.total - w.s) / CROSS_KEEP_RAMP_M).min(1.0);
            if ramp > 0.0 {
                keep_side = CROSS_KEEP_SIDE_M * ramp * if w.dir > 0 { 1.0 } else { -1.0 };
            }
        }
    }
    if w.lateral != 0.0 || keep_side != 0.0 {
        let lat = (w.lateral + keep_side).clamp(-0.85, 0.85);
        // `at` returns the unit direction; the JS offsets along (-dz, dx) of the segment.
        x += -dz * lat;
        z += dx * lat;
    }
    (x, z)
}

fn foot_end_arc(net: &Network, f: FootwayId, end: usize) -> f64 {
    net.footways[f as usize].ends[end].kerb_trim
}

/// `signalCrossingMayStart`: Some(may start) at a signal that serves this crossing, None to take a gap.
fn signal_may_start(ctx: &WalkCtx, junction: JunctionId, link: &[P2; 2], own_seconds: f64) -> Option<bool> {
    let j = &ctx.net.junctions[junction as usize];
    let bearing = (link[1][1] - link[0][1]).atan2(link[1][0] - link[0][0]);
    let axis = j.crossing_axis(bearing)?;
    let remaining = j.ped_remaining(axis, ctx.time);
    if !(remaining > 0.0) {
        return Some(false);
    }
    let t = &ctx.net.timing;
    let link_length = geom::hypot(link[1][0] - link[0][0], link[1][1] - link[0][1]);
    let crosswalk = j.ped_crossing_length(axis, link_length);
    let clearance = crosswalk / t.ped_speed_mps;
    let own_full = own_seconds.max(0.0);
    let own = if link_length > crosswalk && link_length > 0.0 { own_full * (crosswalk / link_length) } else { own_full };
    Some(remaining >= clearance.max(own) + t.ped_buffer_s)
}

/// `crossingIsClearOfTraffic(link, crossingSeconds)`.
fn clear_of_traffic(ctx: &WalkCtx, link: &[P2; 2], crossing_seconds: f64) -> bool {
    let mx = (link[0][0] + link[1][0]) / 2.0;
    let mz = (link[0][1] + link[1][1]) / 2.0;
    let exposure = crossing_seconds.max(0.0) + ctx.net.timing.ped_buffer_s;
    let half_span = geom::hypot(link[1][0] - link[0][0], link[1][1] - link[0][1]) / 2.0;
    for k in 0..ctx.car_x.len() {
        let speed = ctx.car_speed[k];
        if speed <= 1.0 {
            continue;
        }
        let dx = mx - ctx.car_x[k];
        let dz = mz - ctx.car_z[k];
        let range = geom::hypot(dx, dz);
        if range <= MIN_BUMPER_GAP * 2.0 {
            return false;
        }
        let (fx, fz) = (ctx.car_yaw[k].sin(), ctx.car_yaw[k].cos());
        let closing = (dx * fx + dz * fz) / (if range == 0.0 { 1.0 } else { range }) * speed;
        if closing <= 0.5 {
            continue;
        }
        let miss = (dx * fz - dz * fx).abs();
        if miss > half_span + CROSSING_MISS_MARGIN_M {
            continue;
        }
        if range / closing <= exposure {
            return false;
        }
        if range <= CROSSING_GAP_M {
            return false;
        }
    }
    true
}

fn begin_leg(net: &Network, w: &mut Walker, pts: &[P2], next: FootwayId, next_end: u8, kind: LegKind, arc: f64, dir: i8) {
    let n = pts.len().min(3);
    let mut leg_pts = [[0.0; 2]; 3];
    leg_pts[..n].copy_from_slice(&pts[..n]);
    let mut cum = [0.0; 3];
    for i in 1..n {
        cum[i] = cum[i - 1] + geom::hypot(leg_pts[i][0] - leg_pts[i - 1][0], leg_pts[i][1] - leg_pts[i - 1][1]);
    }
    let total = cum[n - 1];
    if !(total > 0.05) {
        adopt(net, w, next, next_end, arc, dir);
        return;
    }
    w.leg = Some(Leg { pts: leg_pts, n: n as u8, cum, total, kind, next, next_end, arc, dir });
    w.s = 0.01;
    w.dir = 1;
    w.activity = Activity::Walk;
    w.facing = Facing::Keep;
}

/// `adoptFootwayPath(pedestrian, index, end, arc, dir)`.
fn adopt(net: &Network, w: &mut Walker, index: FootwayId, end: u8, arc: f64, dir: i8) -> bool {
    let f = &net.footways[index as usize];
    if !f.alive {
        return false;
    }
    let total = if f.length > 0.0 { f.length } else { 0.01 };
    let previous_side = w.road_side;
    w.footway = index;
    w.road_side = f.road_side;
    w.leg = None;
    if let Some(h) = w.lateral_hold.take() {
        w.lateral = h;
    } else if w.road_side != previous_side && w.lateral != 0.0 {
        w.lateral = -w.lateral;
    }
    if !arc.is_nan() {
        w.s = js_clamp(arc, 0.01, (total - 0.01).max(0.01));
        if dir != 0 {
            w.dir = dir;
        }
        return true;
    }
    if end == 0 {
        w.dir = 1;
        w.s = total.min(f.ends[0].kerb_trim + 0.01);
    } else {
        w.dir = -1;
        w.s = (total - f.ends[1].kerb_trim - 0.01).max(0.0);
    }
    true
}

/// `considerCrossing(pedestrian, end)`.
fn consider_crossing(net: &Network, w: &mut Walker, end: usize, stats: &mut CrowdStats) {
    let f = &net.footways[w.footway as usize];
    if !f.routable || f.cross == NONE {
        return;
    }
    let node_id = f.ends[end].node;
    if w.cross_node == node_id {
        return;
    }
    w.cross_node = node_id;
    if node_id == NONE {
        return;
    }
    let node = &net.foot_nodes[node_id as usize];
    let junction = node.ends.len() > 2;
    let signal = node.signal;
    if signal == NONE && !junction {
        return;
    }
    w.cross_leg += 1;
    let share = if signal != NONE { CROSSING_SHARE } else { UNSIGNALLED_CROSSING_SHARE };
    if walker_draw(w.key, "cross-", w.cross_leg) >= share {
        return;
    }
    let other = &net.footways[f.cross as usize];
    if !other.alive {
        return;
    }
    let band_arc = f.ends[end].crossing_arc;
    let band_s = if end == 1 { f.length - band_arc } else { band_arc };
    let off = (w.s - band_s).abs();
    if off > CROSSING_BAND_REACH_M {
        stats.declined_off_band += 1;
        return;
    }
    let station = if off > CROSSING_BAND_SNAP_M { js_clamp(band_s, 0.0, f.length) } else { w.s };
    let (hx, hz, _, _, _) = geom::at_arc(&f.pts, &f.cum, station);
    let (tx, tz, _, _, _) = geom::at_arc(&other.pts, &other.cum, js_clamp(station, 0.0, other.length));
    let lateral = w.lateral;
    let (nx, nz) = (f.normal[0], f.normal[1]);
    let far_lateral = -lateral;
    let from = [hx + nx * lateral, hz + nz * lateral];
    let to = [tx + nx * far_lateral, tz + nz * far_lateral];
    let length = geom::hypot(to[0] - from[0], to[1] - from[1]);
    if !(length > 1.0) {
        return;
    }
    let walk = if station != w.s {
        let (sx, sz, _, _, _) = geom::at_arc(&f.pts, &f.cum, w.s);
        Some([[sx + nx * lateral, sz + nz * lateral], from, to])
    } else {
        None
    };
    w.kerb = Some(KerbWait {
        signal,
        link: [from, to],
        walk,
        next: f.cross,
        next_end: end as u8,
        arc: station,
        dir: w.dir,
        lateral: far_lateral,
        length,
        elapsed: 0.0,
    });
    stats.kerb_waits += 1;
}

/// `continueAlongFootway(pedestrian, node, fromIndex, fromEnd)`.
fn continue_along(net: &Network, w: &mut Walker, node_id: u32, from_end: usize, stats: &mut CrowdStats) -> bool {
    let node = &net.foot_nodes[node_id as usize];
    let from = w.footway;
    let from_segment = net.footways[from as usize].segment;
    let mut total_weight = 0.0;
    let mut count = 0;
    for &(fw, _) in &node.ends {
        if fw == from || !net.footways[fw as usize].alive {
            continue;
        }
        let p = &net.footways[fw as usize];
        let mut weight = p.weight;
        if p.segment == from_segment {
            weight *= 0.05;
        }
        total_weight += weight.max(0.01);
        count += 1;
    }
    if count == 0 {
        return false;
    }
    w.route_leg += 1;
    let mut roll = walker_draw(w.key, "route-", w.route_leg) * total_weight;
    let mut chosen = None;
    let mut last = None;
    for &(fw, e) in &node.ends {
        if fw == from || !net.footways[fw as usize].alive {
            continue;
        }
        let p = &net.footways[fw as usize];
        let mut weight = p.weight;
        if p.segment == from_segment {
            weight *= 0.05;
        }
        last = Some((fw, e));
        roll -= weight.max(0.01);
        if roll <= 0.0 && chosen.is_none() {
            chosen = Some((fw, e));
        }
    }
    let (to_fw, to_end) = chosen.or(last).unwrap();
    let f = &net.footways[from as usize];
    let from_arc = if from_end == 1 { f.length - foot_end_arc(net, from, 1) } else { foot_end_arc(net, from, 0) };
    let (fx, fz, _, _, _) = geom::at_arc(&f.pts, &f.cum, from_arc);
    let t = &net.footways[to_fw as usize];
    let to_arc = if to_end == 1 { t.length - foot_end_arc(net, to_fw, 1) } else { foot_end_arc(net, to_fw, 0) };
    let (tx, tz, _, _, _) = geom::at_arc(&t.pts, &t.cum, to_arc);
    let lateral = w.lateral;
    let lateral_to = if w.road_side == t.road_side { lateral } else { -lateral };
    let pts = [
        [fx + f.normal[0] * lateral, fz + f.normal[1] * lateral],
        [tx + t.normal[0] * lateral_to, tz + t.normal[1] * lateral_to],
    ];
    w.lateral_hold = Some(lateral_to);
    w.lateral = 0.0;
    w.cross_node = NONE;
    stats.continuations += 1;
    begin_leg(net, w, &pts, to_fw, to_end, LegKind::Corner, f64::NAN, 0);
    true
}

/// `routeAtPathEnd(pedestrian, end)`.
fn route_at_end(net: &Network, w: &mut Walker, end: usize, stats: &mut CrowdStats) -> bool {
    if let Some(leg) = w.leg {
        if leg.kind == LegKind::Cross {
            stats.crossings_completed += 1;
        }
        return adopt(net, w, leg.next, leg.next_end, leg.arc, leg.dir);
    }
    let f = &net.footways[w.footway as usize];
    if !f.routable {
        return false;
    }
    let node = f.ends[end].node;
    if node == NONE {
        return false;
    }
    if continue_along(net, w, node, end, stats) {
        return true;
    }
    stats.reversals += 1;
    false
}

/// `updateKerbWait`: true while the walker stands at the kerb.
fn update_kerb_wait(ctx: &WalkCtx, w: &mut Walker, dt: f64, stats: &mut CrowdStats) -> bool {
    let Some(mut wait) = w.kerb else { return false };
    wait.elapsed += dt.max(0.0);
    w.activity = Activity::Wait;
    w.facing = Facing::Across;
    let crossing_seconds = wait.length / w.speed.max(0.4);
    let walk = if wait.signal != NONE { signal_may_start(ctx, wait.signal, &wait.link, crossing_seconds) } else { None };
    let clear = walk.unwrap_or_else(|| clear_of_traffic(ctx, &wait.link, crossing_seconds));
    if clear {
        if walk.is_some() {
            // The indication a pedestrian head beside this kerb shows right now.
            let j = &ctx.net.junctions[wait.signal as usize];
            let bearing = (wait.link[1][1] - wait.link[0][1]).atan2(wait.link[1][0] - wait.link[0][0]);
            if let Some(axis) = j.crossing_axis(bearing) {
                let length = j.ped_crossing_length(axis, wait.length);
                if crate::signals::ped_indication(j, axis, ctx.time, length, &ctx.net.timing) != 3 {
                    stats.starts_outside_walk += 1;
                }
            }
        }
        w.kerb = None;
        w.activity = Activity::Walk;
        w.facing = Facing::Keep;
        w.lateral_hold = Some(wait.lateral);
        w.lateral = 0.0;
        match &wait.walk {
            Some(pts) => begin_leg(ctx.net, w, pts, wait.next, wait.next_end, LegKind::Cross, wait.arc, wait.dir),
            None => begin_leg(ctx.net, w, &wait.link, wait.next, wait.next_end, LegKind::Cross, wait.arc, wait.dir),
        }
        stats.crossings_started += 1;
        return false;
    }
    if wait.elapsed >= ctx.net.kerb_wait_max_s {
        w.kerb = None;
        w.activity = Activity::Walk;
        w.facing = Facing::Keep;
        stats.kerb_waits_abandoned += 1;
        return false;
    }
    w.kerb = Some(wait);
    true
}

/// `updatePedestrian` for an ambient walker.
fn step_walker(ctx: &WalkCtx, w: &mut Walker, dt: f64, stats: &mut CrowdStats) {
    let net = ctx.net;
    let waiting = update_kerb_wait(ctx, w, dt, stats);
    let on_leg = w.leg.is_some();
    let scheduled = if waiting || on_leg { false } else { w.advance_activity(net, ctx.hour, dt) };
    if on_leg {
        w.activity = Activity::Walk;
        w.facing = Facing::Keep;
    }
    let moving = on_leg || scheduled;
    let pace = w.speed;
    w.ground_speed = if moving { pace } else { 0.0 };
    if moving {
        w.s += w.dir as f64 * pace * dt;
        w.gait += w.gait_freq * dt;
    }
    let total = w.total(net);
    let routed = w.leg.is_none();
    let (walk_end1, walk_end0) = if routed {
        (total - foot_end_arc(net, w.footway, 1), foot_end_arc(net, w.footway, 0))
    } else {
        (total, 0.0)
    };
    if moving && routed && w.kerb.is_none() {
        let toward = if w.dir > 0 { 1 } else { 0 };
        let to_end = if w.dir > 0 { total - w.s } else { w.s };
        if to_end <= net.footways[w.footway as usize].ends[toward].crossing_arc {
            consider_crossing(net, w, toward, stats);
        }
    }
    if w.s >= walk_end1 {
        w.s = walk_end1;
        if !route_at_end(net, w, 1, stats) {
            w.dir = -1;
        }
    } else if w.s <= walk_end0 {
        w.s = walk_end0;
        if !route_at_end(net, w, 0, stats) {
            w.dir = 1;
        }
    }
    // Heading: a body turns at a walking rate.
    let (_, _, dx, dz) = w.at(net, w.s);
    let (fx, fz) = if w.dir > 0 { (dx, dz) } else { (-dx, -dz) };
    let travel_yaw = fx.atan2(fz);
    let target = if moving {
        travel_yaw
    } else {
        let across = (-dz).atan2(dx);
        match w.facing {
            Facing::Keep => travel_yaw,
            Facing::Across => {
                if w.road_side < 0 { across + std::f64::consts::PI } else { across }
            }
            Facing::Inward => {
                if w.road_side < 0 { across } else { across + std::f64::consts::PI }
            }
            Facing::Partner => {
                travel_yaw + if w.lateral >= 0.0 { -std::f64::consts::FRAC_PI_2 } else { std::f64::consts::FRAC_PI_2 }
            }
        }
    };
    if w.yaw.is_nan() {
        w.yaw = target;
    }
    let step = PEDESTRIAN_TURN_RATE * dt.max(0.0);
    let diff = normalize_angle(target - w.yaw);
    w.yaw = normalize_angle(w.yaw + js_clamp(diff, -step, step));
}
