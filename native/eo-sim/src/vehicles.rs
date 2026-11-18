//! Moving traffic: a port of the driving model of `TrafficSim`
//! (src/citygen/traffic.js) - `advanceAiCar`, `updateCarSpacing`,
//! `updateCrossConflictGaps`, `chooseNextEdge`, `spawnCar`,
//! `separateQueuedCars`, `respawnCar`, `relieveGridlock`, the junction claims -
//! restructured for data:
//!
//!   * The pose every other system reads (x, z, yaw, speed) is held in flat
//!     arrays (`Fleet::x` ...). The planner state of each car is a compact
//!     plain struct. Nothing in the step allocates once the scratch buffers
//!     have grown to the fleet's size (the one exception is a corner opened
//!     late, which stores its re-planned arc).
//!   * A step is SENSE then ACT. Sense reads the fleet as the previous step
//!     left it and writes each car's gaps (leader, cross corridor, person,
//!     occupied crossing); every car's sense is independent. Act runs the cars
//!     in id order, because the one shared decision - who may enter a
//!     junction box - must be resolved in a fixed order to replay.
//!
//! The speed law is the JS one term for term: the target speed is the minimum
//! of every limit (class limit x driver pace, turn cap, signal / stop bar,
//! junction admission, dead end, leader at the settle gap, cross corridor,
//! box-clear hold at the bar, person, occupied crossing), the speed moves
//! toward it at `ACCEL` / `DECEL`, and the advance is then clamped so a body
//! can never tunnel through a leader, a crossing car or a person in one step.
//!
//! What it does NOT do (the JS does): the static-blocker bypass (no parked
//! bodies or wrecks exist here), ride dispatch and resident rides, combat
//! damage and impact holds, the population budget, destinations, the
//! occlusion-tested visibility (a cone test stands in), parked cars (the JS
//! places them in the render pass, `planParkedVehicles`, not in TrafficSim),
//! and the hold-overrun and claim-path refinements of the approach. See README
//! "Simulation core".

use crate::consts::*;
use crate::geom::{self, Plan, PlanKind, js_clamp};
use crate::grid::Grid;
use crate::network::{ControlKind, LaneId, NONE, Network, NodeId};
use crate::rng::{Mulberry32, driver_pace};
use crate::signals::{GREEN, RED, YELLOW};

pub const NO_LINK: u16 = u16::MAX;
pub const KIND_NAMES: [&str; 4] = ["sedan", "taxi", "truck", "bus"];

#[derive(Clone, Debug)]
pub struct Car {
    pub id: u32,
    pub kind: u8,
    pub len: f64,
    pub wid: f64,
    pub pace: f64,
    pub lane: LaneId,
    /// Arc of the car's centre along `lane`. While cornering, the arc on the
    /// outgoing lane the car's centre will reach (`exit_arc - (arc_len - s)`).
    pub arc: f64,
    pub speed: f64,
    pub max_speed: f64,
    pub next: u16,
    pub corner_from: LaneId,
    pub corner_link: u16,
    pub corner_s: f64,
    /// A corner opened late (`cornerFilletAt`): the re-planned arc that starts
    /// where the car stood. Allocated only on that rare event.
    pub corner_replan: Option<Box<Plan>>,
    pub claim_node: NodeId,
    pub claim_move: u32,
    /// The exit arc of the claimed movement on the lane it lands on.
    pub claim_exit: f64,
    pub claim_stall: f64,
    pub claim_yield: f64,
    pub junction_wait: f64,
    pub stop_released: bool,
    pub stop_hold: f64,
    pub terminal_timer: f64,
    pub gridlock_stall: f64,
    pub cross_wait: f64,
    pub person_hold: f64,
    pub travelled: f64,
    pub braking: bool,
    pub turn_side: i8,
    /// Level of detail: stepped every `SIM_TIER_FAR_EVERY` steps with the summed time.
    pub far: bool,
    pub lod_debt: f64,
}

impl Car {
    #[inline]
    pub fn cornering(&self) -> bool {
        self.corner_from != NONE
    }

    /// The plan of the corner being driven.
    #[inline]
    pub fn corner_plan<'a>(&'a self, net: &'a Network) -> Option<&'a Plan> {
        if let Some(p) = &self.corner_replan {
            return Some(p);
        }
        net.lanes[self.corner_from as usize].links.get(self.corner_link as usize).and_then(|l| l.plan.as_ref())
    }
}

/// An occupied crosswalk: a walker's crossing leg.
#[derive(Clone, Copy, Debug, Default)]
pub struct Band {
    pub a: [f64; 2],
    pub b: [f64; 2],
}

#[derive(Clone, Copy, Debug, Default)]
struct ClaimSlot {
    node: NodeId,
    n: u8,
    car: [u32; 6],
    movement: [u32; 6],
}

/// The fleet of one region.
#[derive(Default)]
pub struct Fleet {
    pub cars: Vec<Car>,
    pub x: Vec<f64>,
    pub z: Vec<f64>,
    pub yaw: Vec<f64>,
    pub spd: Vec<f64>,
    leader_gap: Vec<f64>,
    leader_len: Vec<f64>,
    cross_gap: Vec<f64>,
    person_gap: Vec<f64>,
    crossing_hold: Vec<f64>,
    order: Vec<(u64, u32)>,
    grid: Option<Grid>,
    claims: Vec<ClaimSlot>,
    gridlock_cooldown: f64,
    pub stats: FleetStats,
    weights: Vec<(u16, f64)>,
    respawn_scratch: Vec<LaneId>,
    /// the id the next spawned car takes (0, 1, 2... as the JS numbers
    /// its entities; a server starts each region at its own base so ids stay
    /// unique when cars are handed between regions).
    pub next_id: u32,
}

/// a car leaving one region's fleet for another's (`Fleet::take` / `Fleet::adopt`).
#[derive(Clone, Debug)]
pub struct CarHandoff {
    pub car: Car,
    /// x, z, yaw, speed as the last step left them.
    pub pose: [f64; 4],
}

#[derive(Clone, Debug, Default)]
pub struct FleetStats {
    pub respawns: u64,
    pub terminus_respawns: u64,
    pub gridlock_respawns: u64,
    pub stop_sign_stops: u64,
    pub claims_granted: u64,
    pub claims_expired: u64,
    pub late_corners: u64,
    /// Junction entries from a signalised lane showing red by a car not already past its bar (must stay 0).
    pub red_entries: u64,
    /// Entries on red by a car committed past the bar when the light changed (legal: it entered on yellow).
    pub committed_on_red: u64,
}

/// What a car's step reads from the world besides the fleet.
pub struct StepCtx<'a> {
    pub net: &'a Network,
    pub time: f64,
    pub seed_int: u32,
    /// The focus the homing bias steers toward (`localLifeFocus`), if any; a
    /// respawn then lands within `LOCAL_RECYCLE_RADIUS` of it (the JS window
    /// is the whole network, so there it lands anywhere).
    pub homing_focus: Option<[f64; 2]>,
    /// Is a world point currently on camera (recycling and gridlock relief leave those alone)?
    pub visible: &'a dyn Fn(f64, f64) -> bool,
}

fn presented(kind: u8) -> (f64, f64) {
    (PRESENTED_LENGTH[kind as usize] as f64, PRESENTED_WIDTH[kind as usize] as f64)
}

/// `pickTrafficKind(roll)`.
pub fn pick_kind(roll: f64) -> u8 {
    let mut c = 0.0;
    for (i, share) in TRAFFIC_KIND_MIX.iter().enumerate() {
        c += share;
        if roll < c {
            return i as u8;
        }
    }
    (TRAFFIC_KIND_MIX.len() - 1) as u8
}

/// `crossLookahead(speed)`.
#[inline]
fn cross_lookahead(speed: f64) -> f64 {
    js_clamp(speed * CROSS_LOOKAHEAD_S, CROSS_LOOK_MIN, CROSS_LOOK_MAX)
}

/// `crossCorridorGap(car, other, look)`: how far car A can drive straight on before touching B.
#[inline]
#[allow(clippy::too_many_arguments)]
pub fn cross_corridor_gap(ax: f64, az: f64, ayaw: f64, alen: f64, awid: f64, bx: f64, bz: f64, byaw: f64, blen: f64, bwid: f64, look: f64) -> Option<f64> {
    let (asin, acos) = ayaw.sin_cos();
    let (bsin, bcos) = byaw.sin_cos();
    let (afx, afz, alx, alz) = (asin, acos, acos, -asin);
    let (bfx, bfz, blx, blz) = (bsin, bcos, bcos, -bsin);
    let dx = bx - ax;
    let dz = bz - az;
    let axes = [(afx, afz), (alx, alz), (bfx, bfz), (blx, blz)];
    let mut enter = 0.0f64;
    let mut exit = look;
    for (nx, nz) in axes {
        let d = dx * nx + dz * nz;
        let ea = ((afx * nx + afz * nz).abs() * alen + (alx * nx + alz * nz).abs() * awid) / 2.0;
        let eb = ((bfx * nx + bfz * nz).abs() * blen + (blx * nx + blz * nz).abs() * bwid) / 2.0;
        let reach = ea + eb;
        let rate = afx * nx + afz * nz;
        if rate.abs() < 1e-9 {
            if d.abs() > reach {
                return None;
            }
            continue;
        }
        let t0 = (d - reach) / rate;
        let t1 = (d + reach) / rate;
        enter = enter.max(t0.min(t1));
        exit = exit.min(t0.max(t1));
        if enter > exit {
            return None;
        }
    }
    if enter > look { None } else { Some(enter.max(0.0)) }
}

impl Fleet {
    pub fn len(&self) -> usize {
        self.cars.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cars.is_empty()
    }

    fn push_pose_slots(&mut self) {
        self.x.push(0.0);
        self.z.push(0.0);
        self.yaw.push(0.0);
        self.spd.push(0.0);
        self.leader_gap.push(f64::INFINITY);
        self.leader_len.push(4.0);
        self.cross_gap.push(f64::INFINITY);
        self.person_gap.push(f64::INFINITY);
        self.crossing_hold.push(f64::INFINITY);
    }

    /// `spawnCar(edge, color, random, entityId)`: the JS draw sequence, so a
    /// fleet spawned on the JS lane order from the JS seed picks the JS kinds and lanes.
    pub fn spawn(&mut self, net: &Network, lanes: &[LaneId], rng: &mut Mulberry32, seed_int: u32) -> bool {
        if lanes.is_empty() {
            return false;
        }
        let id = self.next_id; // was `self.cars.len()`, the same number while no car is handed off
        let first = lanes[(rng.next_f64() * lanes.len() as f64) as usize];
        let _paint = rng.next_f64();
        let kind = pick_kind(rng.next_f64());
        let (len, wid) = presented(kind);
        let mut placement = None;
        for attempt in 0..10 {
            let cand = if attempt == 0 { first } else { lanes[(rng.next_f64() * lanes.len() as f64) as usize] };
            let lane = &net.lanes[cand as usize];
            let segs = (lane.pts.len().saturating_sub(1)).max(1);
            let path_index = (rng.next_f64() * segs as f64) as usize;
            let seg_len = {
                let l = lane.cum.get(path_index + 1).copied().unwrap_or(lane.length) - lane.cum[path_index];
                if l == 0.0 { 1.0 } else { l }
            };
            let arc = lane.cum[path_index] + rng.next_f64() * seg_len;
            let bar = matches!(lane.control, ControlKind::Signal { .. } | ControlKind::Stop { enforced: true, .. });
            if bar && arc > (lane.length - net.stop_offset(cand) - len / 2.0).max(0.0) - 0.25 {
                continue;
            }
            let clear = !self.cars.iter().any(|o| o.lane == cand && (o.arc - arc).abs() < 7.0);
            if clear {
                placement = Some((cand, arc));
                break;
            }
        }
        let Some((lane, arc)) = placement else { return false };
        let speed = 1.5 + rng.next_f64() * 2.5;
        let pace = driver_pace(id, seed_int, DRIVER_PACE_MIN, DRIVER_PACE_SPAN);
        self.cars.push(Car {
            id,
            kind,
            len,
            wid,
            pace,
            lane,
            arc,
            speed,
            max_speed: net.lanes[lane as usize].max_speed * pace,
            next: NO_LINK,
            corner_from: NONE,
            corner_link: NO_LINK,
            corner_s: 0.0,
            corner_replan: None,
            claim_node: NONE,
            claim_move: 0,
            claim_exit: 0.0,
            claim_stall: 0.0,
            claim_yield: 0.0,
            junction_wait: 0.0,
            stop_released: false,
            stop_hold: 0.0,
            terminal_timer: 0.0,
            gridlock_stall: 0.0,
            cross_wait: 0.0,
            person_hold: 0.0,
            travelled: 0.0,
            braking: false,
            turn_side: 0,
            far: false,
            lod_debt: 0.0,
        });
        self.next_id += 1; // net
        self.push_pose_slots();
        let i = self.cars.len() - 1;
        self.write_pose(net, i);
        true
    }

    /// hand car `i` off (to another region): it leaves this fleet with its
    /// state and pose; its junction claim is released, and the last car takes
    /// its index (claims renumbered). Draws no random numbers.
    pub fn take(&mut self, i: usize) -> CarHandoff {
        self.release_claim(i);
        let last = self.cars.len() - 1;
        if i != last {
            for s in &mut self.claims {
                for k in 0..s.n as usize {
                    if s.car[k] == last as u32 {
                        s.car[k] = i as u32;
                    }
                }
            }
        }
        let car = self.cars.swap_remove(i);
        let pose = [self.x.swap_remove(i), self.z.swap_remove(i), self.yaw.swap_remove(i), self.spd.swap_remove(i)];
        self.leader_gap.swap_remove(i);
        self.leader_len.swap_remove(i);
        self.cross_gap.swap_remove(i);
        self.person_gap.swap_remove(i);
        self.crossing_hold.swap_remove(i);
        CarHandoff { car, pose }
    }

    /// take in a car handed off by another region on the same network,
    /// keeping its id, kind, pace, lane, arc, speed and corner. It holds no
    /// junction claim here (it re-claims on its next decision). Returns its
    /// index. Ids stay unique only if the regions number from disjoint bases
    /// (`Region::with_id_base`).
    pub fn adopt(&mut self, net: &Network, h: CarHandoff) -> usize {
        let mut car = h.car;
        car.claim_node = NONE;
        car.claim_stall = 0.0;
        self.cars.push(car);
        self.push_pose_slots();
        let i = self.cars.len() - 1;
        self.write_pose(net, i);
        i
    }

    /// `queueInsertBlocked(edge, arc, kind, dims, ignore)`: may car `me` be set
    /// down at `arc` on `lane`? Not past every reachable exit, not inside the
    /// junction hold, not inside a queue headway on the lane, and its presented
    /// body (plus the insert margin) clear of every nearby body on any lane.
    pub fn insert_blocked(&self, net: &Network, lane: LaneId, arc: f64, me: usize) -> bool {
        let l = &net.lanes[lane as usize];
        let has_exits = l.links.iter().any(|k| k.drivable);
        if has_exits && !net.any_reachable(lane, arc) {
            return true;
        }
        let kind = self.cars[me].kind as usize;
        if has_exits && arc > l.length - l.hold[kind] {
            return true;
        }
        let (len, wid) = (self.cars[me].len, self.cars[me].wid);
        let (x, z, dx, dz, _) = geom::at_arc(&l.pts, &l.cum, arc);
        let yaw = dx.atan2(dz);
        const INSERT_MARGIN: f64 = 0.30; // QUEUE_SEPARATION.insertMargin
        for (j, o) in self.cars.iter().enumerate() {
            if j == me {
                continue;
            }
            if o.lane == lane && !o.cornering() {
                let headway = (len + o.len) / 2.0 + MIN_BUMPER_GAP + FOLLOW_SETTLE_MARGIN;
                if (o.arc - arc).abs() < headway.max(7.0) {
                    return true;
                }
            }
            let reach = (geom::hypot(len, wid) + geom::hypot(o.len, o.wid)) / 2.0 + INSERT_MARGIN;
            if geom::hypot(self.x[j] - x, self.z[j] - z) > reach {
                continue;
            }
            if geom::obb_penetration((x, z, yaw, len + INSERT_MARGIN, wid + INSERT_MARGIN), (self.x[j], self.z[j], self.yaw[j], o.len, o.wid)) > 0.0 {
                return true;
            }
        }
        false
    }

    /// `landingUnfit(car, edge, arc, speed)`: never where no exit is reachable,
    /// nor inside the junction hold plus half an approach and the braking distance.
    pub fn landing_unfit(&self, net: &Network, lane: LaneId, arc: f64, me: usize, speed: f64) -> bool {
        let l = &net.lanes[lane as usize];
        if !net.any_reachable(lane, arc) {
            return true;
        }
        let brake = speed * speed / (2.0 * APPROACH_COMFORT_DECEL) + 1.0;
        l.length - arc < l.hold[self.cars[me].kind as usize] + TURN_APPROACH_M / 2.0 + brake
    }

    /// `repairQueueInsert`: the nearest clear arc in 2.75 m strides, ahead first.
    pub fn repair_insert(&self, net: &Network, lane: LaneId, arc: f64, me: usize) -> Option<f64> {
        if !self.insert_blocked(net, lane, arc, me) {
            return Some(arc);
        }
        let l = &net.lanes[lane as usize];
        let max_arc = l.length - 2.0;
        if !(max_arc > 2.0) {
            return None;
        }
        const REPAIR_STEP: f64 = 2.75; // QUEUE_SEPARATION.repairStep
        let strides = (l.length / REPAIR_STEP).ceil() as i32;
        for k in 1..=strides {
            for dir in [1.0, -1.0] {
                let c = arc + dir * k as f64 * REPAIR_STEP;
                if c < 2.0 || c > max_arc {
                    continue;
                }
                if !self.insert_blocked(net, lane, c, me) {
                    return Some(c);
                }
            }
        }
        None
    }

    /// `separateQueuedCars`: after the seeded spawn, move every car that breaks
    /// the insert constraint to the nearest clear arc on its lane, else onto the
    /// next lane (in network order) with room. Draws no random numbers.
    pub fn separate(&mut self, net: &Network) {
        let lanes = net.lanes.len();
        for i in 0..self.cars.len() {
            let (lane, arc) = (self.cars[i].lane, self.cars[i].arc);
            if !self.insert_blocked(net, lane, arc, i) {
                continue;
            }
            if let Some(a) = self.repair_insert(net, lane, arc, i) {
                self.cars[i].arc = a;
                self.write_pose(net, i);
                continue;
            }
            for e in 1..=lanes {
                let cand = ((lane as usize + e) % lanes) as LaneId;
                let l = &net.lanes[cand as usize];
                if !l.alive || !(l.length > 8.0) {
                    continue;
                }
                let middle = js_clamp(l.length / 2.0, 2.0, (l.length - 2.0).max(2.0));
                if let Some(a) = self.repair_insert(net, cand, middle, i) {
                    let speed = self.cars[i].speed;
                    self.place(net, i, cand, a, speed);
                    break;
                }
            }
        }
    }

    /// Pose from the route cursor (lane arc, or the plan while cornering).
    pub fn write_pose(&mut self, net: &Network, i: usize) {
        let car = &self.cars[i];
        let (x, z, yaw) = if car.cornering() {
            match car.corner_plan(net) {
                Some(p) => p.pose_at(car.corner_s),
                None => {
                    let lane = &net.lanes[car.lane as usize];
                    let (x, z, dx, dz, _) = geom::at_arc(&lane.pts, &lane.cum, 0.0);
                    (x, z, dx.atan2(dz))
                }
            }
        } else {
            let lane = &net.lanes[car.lane as usize];
            let (x, z, dx, dz, _) = geom::at_arc(&lane.pts, &lane.cum, car.arc.clamp(0.0, lane.length));
            (x, z, dx.atan2(dz))
        };
        self.x[i] = x;
        self.z[i] = z;
        self.yaw[i] = yaw;
        self.spd[i] = car.speed;
    }

    fn sort_key(lane: LaneId, arc: f64) -> u64 {
        ((lane as u64) << 32) | (((arc + 2000.0) * 256.0).clamp(0.0, u32::MAX as f64) as u64)
    }

    /// The first (lowest-arc) car on a lane, from the sorted order.
    fn first_on_lane(&self, lane: LaneId, skip: usize) -> Option<usize> {
        let lo = self.order.partition_point(|e| (e.0 >> 32) < lane as u64);
        let mut k = lo;
        while k < self.order.len() && (self.order[k].0 >> 32) == lane as u64 {
            let i = self.order[k].1 as usize;
            if i != skip {
                return Some(i);
            }
            k += 1;
        }
        None
    }

    // ------------------------------------------------------------ sense

    /// Every gap a car's speed law reads, from the fleet as the last step left it.
    pub fn sense(&mut self, net: &Network, walkers_x: &[f64], walkers_z: &[f64], walker_grid: &Grid, bands: &[Band]) {
        let n = self.cars.len();
        // Lane order (`updateCarSpacing`'s byEdge buckets, sorted by arc).
        self.order.clear();
        for (i, c) in self.cars.iter().enumerate() {
            self.order.push((Self::sort_key(c.lane, c.arc), i as u32));
        }
        self.order.sort_unstable();
        for k in 0..n {
            let i = self.order[k].1 as usize;
            let lane = self.order[k].0 >> 32;
            match self.order.get(k + 1) {
                Some(&(key, j)) if key >> 32 == lane => {
                    let j = j as usize;
                    self.leader_gap[i] = self.cars[j].arc - self.cars[i].arc;
                    self.leader_len[i] = self.cars[j].len;
                }
                _ => {
                    self.leader_gap[i] = f64::INFINITY;
                    self.leader_len[i] = 4.0;
                }
            }
        }
        // Across the junction: the car the leader term will meet on the chosen next lane.
        for i in 0..n {
            if self.leader_gap[i].is_finite() || self.cars[i].cornering() || self.cars[i].next == NO_LINK {
                continue;
            }
            let c = &self.cars[i];
            let lane = &net.lanes[c.lane as usize];
            let Some(link) = lane.links.get(c.next as usize) else { continue };
            let remaining = lane.length - c.arc;
            if remaining > 40.0 {
                continue;
            }
            if let Some(j) = self.first_on_lane(link.to, i) {
                let (setback, arc_len, exit) =
                    link.plan.as_ref().map_or((0.0, 0.0, 0.0), |p| (p.entry_setback.clamp(0.0, remaining), p.arc_len, p.exit_arc));
                self.leader_gap[i] = remaining - setback + arc_len + (self.cars[j].arc - exit);
                self.leader_len[i] = self.cars[j].len;
            }
        }
        // Cross corridor (`updateCrossConflictGaps`).
        let grid = self.grid.get_or_insert_with(|| Grid::new(CROSS_CELL_M));
        grid.build(&self.x, &self.z, CROSS_CELL_M, |_| true);
        let grid = self.grid.as_ref().unwrap();
        for i in 0..n {
            let c = &self.cars[i];
            let mut look = cross_lookahead(c.speed.max(0.0));
            if c.cornering() {
                let arc_len = c.corner_plan(net).map_or(0.5, |p| p.arc_len);
                look = look.min((arc_len - c.corner_s).max(1.0));
            } else {
                let lane = &net.lanes[c.lane as usize];
                let remaining = lane.length - c.arc;
                let setback = if c.next != NO_LINK {
                    lane.links.get(c.next as usize).and_then(|l| l.plan.as_ref()).map_or(0.0, |p| p.entry_setback)
                } else {
                    0.0
                };
                look = look.min((remaining - setback).max(1.0));
            }
            let (x, z, yaw) = (self.x[i], self.z[i], self.yaw[i]);
            let (fx, fz) = (yaw.sin(), yaw.cos());
            let (x0, x1) = (x.min(x + fx * look) - 8.0, x.max(x + fx * look) + 8.0);
            let (z0, z1) = (z.min(z + fz * look) - 8.0, z.max(z + fz * look) + 8.0);
            let mut best = f64::INFINITY;
            let cars = &self.cars;
            let (xs, zs, yaws) = (&self.x, &self.z, &self.yaw);
            grid.query(x0, z0, x1, z1, |j| {
                let j = j as usize;
                if j == i || cars[j].lane == c.lane {
                    return;
                }
                let o = &cars[j];
                let Some(gap) = cross_corridor_gap(x, z, yaw, c.len, c.wid, xs[j], zs[j], yaws[j], o.len, o.wid, look) else {
                    return;
                };
                let mutual = cross_corridor_gap(xs[j], zs[j], yaws[j], o.len, o.wid, x, z, yaw, c.len, c.wid, cross_lookahead(o.speed.max(0.0)))
                    .is_some();
                if mutual && !yields_to(c, o) {
                    return;
                }
                if gap < best {
                    best = gap;
                }
            });
            self.cross_gap[i] = best;
        }
        // People in the forward corridor (`personConflictGap`), and occupied crossings ahead.
        for i in 0..n {
            let c = &self.cars[i];
            let (x, z, yaw) = (self.x[i], self.z[i], self.yaw[i]);
            let (fx, fz) = (yaw.sin(), yaw.cos());
            let stopping = c.speed * c.speed / (2.0 * PERSON_DECEL) + c.len / 2.0 + PERSON_STOP_MARGIN_M;
            let reach = stopping.clamp(PERSON_SCAN_MIN_M, PERSON_SCAN_MAX_M);
            let half = c.wid / 2.0 + PERSON_SIDE_MARGIN_M + PERSON_CAPSULE_R_M;
            let (x0, x1) = (x.min(x + fx * reach) - half, x.max(x + fx * reach) + half);
            let (z0, z1) = (z.min(z + fz * reach) - half, z.max(z + fz * reach) + half);
            let mut best = f64::INFINITY;
            walker_grid.query(x0, z0, x1, z1, |w| {
                let w = w as usize;
                let dx = walkers_x[w] - x;
                let dz = walkers_z[w] - z;
                let fwd = dx * fx + dz * fz;
                let lat = (dx * fz - dz * fx).abs();
                if fwd > 0.0 && fwd < reach && lat < half {
                    let gap = fwd - c.len / 2.0 - PERSON_CAPSULE_R_M;
                    if gap < best {
                        best = gap;
                    }
                }
            });
            self.person_gap[i] = if best.is_finite() { best.max(0.0) } else { f64::INFINITY };
            // Occupied crosswalks the car's path crosses within the yield lookahead
            // (`occupiedCrossingHold`): stop with the nose short of the band.
            let mut hold = f64::INFINITY;
            if !bands.is_empty() && !c.cornering() {
                let lane = &net.lanes[c.lane as usize];
                let look = 22.0 + c.len / 2.0;
                let mut travelled = 0.0;
                let (sx, sz, _, _, seg) = geom::at_arc(&lane.pts, &lane.cum, c.arc.clamp(0.0, lane.length));
                let mut prev = [sx, sz];
                let mut k = seg + 1;
                let mut on_next = false;
                loop {
                    let p = if !on_next && k < lane.pts.len() {
                        lane.pts[k]
                    } else {
                        // Continue onto the chosen next lane's first stretch.
                        if on_next || c.next == NO_LINK {
                            break;
                        }
                        on_next = true;
                        match lane.links.get(c.next as usize) {
                            Some(l) => {
                                let o = &net.lanes[l.to as usize];
                                o.pts[o.pts.len().min(2) - 1]
                            }
                            None => break,
                        }
                    };
                    let seg_len = geom::hypot(p[0] - prev[0], p[1] - prev[1]);
                    for band in bands {
                        if geom::segments_cross(prev, p, band.a, band.b) {
                            let (ax, az) = (band.b[0] - band.a[0], band.b[1] - band.a[1]);
                            let den = (p[0] - prev[0]) * az - (p[1] - prev[1]) * ax;
                            let t = if den.abs() > 1e-9 {
                                ((band.a[0] - prev[0]) * az - (band.a[1] - prev[1]) * ax) / den
                            } else {
                                0.0
                            };
                            let d = travelled + t.clamp(0.0, 1.0) * seg_len;
                            hold = hold.min(d - 1.8 - c.len / 2.0);
                        }
                    }
                    travelled += seg_len;
                    prev = p;
                    k += 1;
                    if travelled > look {
                        break;
                    }
                }
            }
            self.crossing_hold[i] = hold;
        }
    }

    // ------------------------------------------------------------ claims

    fn slot(&self, node: NodeId) -> Option<usize> {
        self.claims.iter().position(|s| s.node == node && s.n > 0)
    }

    fn holds(&self, i: usize, node: NodeId) -> bool {
        self.cars[i].claim_node == node
    }

    fn release_claim(&mut self, i: usize) {
        let node = self.cars[i].claim_node;
        if node == NONE {
            return;
        }
        if let Some(s) = self.slot(node) {
            let slot = &mut self.claims[s];
            let mut k = 0;
            while k < slot.n as usize {
                if slot.car[k] == i as u32 {
                    slot.n -= 1;
                    slot.car[k] = slot.car[slot.n as usize];
                    slot.movement[k] = slot.movement[slot.n as usize];
                } else {
                    k += 1;
                }
            }
        }
        self.cars[i].claim_node = NONE;
    }

    /// May car `i` enter `node` on movement `m` now? Every holder's movement must be disjoint.
    fn box_compatible(&self, net: &Network, node: NodeId, m: usize, i: usize) -> bool {
        let Some(s) = self.slot(node) else { return true };
        let slot = &self.claims[s];
        if slot.n as usize >= slot.car.len() {
            return false;
        }
        (0..slot.n as usize).all(|k| slot.car[k] as usize == i || !net.movements_conflict(node, m, slot.movement[k] as usize))
    }

    fn take_claim(&mut self, i: usize, node: NodeId, m: usize) {
        self.release_claim(i);
        let s = match self.slot(node) {
            Some(s) => s,
            None => match self.claims.iter().position(|s| s.n == 0) {
                Some(s) => {
                    self.claims[s].node = node;
                    s
                }
                None => {
                    self.claims.push(ClaimSlot { node, ..Default::default() });
                    self.claims.len() - 1
                }
            },
        };
        let slot = &mut self.claims[s];
        slot.car[slot.n as usize] = i as u32;
        slot.movement[slot.n as usize] = m as u32;
        slot.n += 1;
        self.cars[i].claim_node = node;
        self.cars[i].claim_move = m as u32;
        self.stats.claims_granted += 1;
    }

    /// Is a conflicting car holding the box this car is heading for? (`junctionAvailable` negated)
    fn box_busy(&self, net: &Network, node: NodeId, i: usize, m: Option<usize>) -> bool {
        let Some(s) = self.slot(node) else { return false };
        let slot = &self.claims[s];
        (0..slot.n as usize).any(|k| {
            slot.car[k] as usize != i && m.is_none_or(|m| net.movements_conflict(node, m, slot.movement[k] as usize))
        })
    }

    // ------------------------------------------------------------ act

    /// One fixed step of every car, in id order.
    pub fn act(&mut self, ctx: &StepCtx, dt: f64, rng: &mut Mulberry32, parity: u64) {
        let n = self.cars.len();
        for i in 0..n {
            let mut step_dt = dt;
            if self.cars[i].far {
                // LOD: a far car is advanced on alternate steps with the time it skipped.
                self.cars[i].lod_debt += dt;
                if (i as u64 + parity) % SIM_TIER_FAR_EVERY as u64 != 0 {
                    continue;
                }
                step_dt = self.cars[i].lod_debt;
                self.cars[i].lod_debt = 0.0;
            } else {
                step_dt += self.cars[i].lod_debt;
                self.cars[i].lod_debt = 0.0;
            }
            if !ctx.net.lanes[self.cars[i].lane as usize].alive
                || (self.cars[i].cornering() && !ctx.net.lanes[self.cars[i].corner_from as usize].alive)
            {
                self.cars[i].corner_from = NONE;
                self.cars[i].corner_replan = None;
                self.respawn(ctx, i, rng);
                continue;
            }
            self.step_car(ctx, i, step_dt, rng);
            let c = &mut self.cars[i];
            c.gridlock_stall = if c.speed <= STALL_SPEED_MS { c.gridlock_stall + step_dt } else { 0.0 };
            c.cross_wait = if self.cross_gap[i].is_finite() && c.speed <= 0.5 { c.cross_wait + step_dt } else { 0.0 };
            self.write_pose(ctx.net, i);
        }
        self.relieve_gridlock(ctx, dt, rng);
    }

    /// `relieveGridlock`: respawn the longest-stalled car the camera cannot see.
    fn relieve_gridlock(&mut self, ctx: &StepCtx, dt: f64, rng: &mut Mulberry32) {
        self.gridlock_cooldown = (self.gridlock_cooldown - dt).max(0.0);
        if self.gridlock_cooldown > 0.0 {
            return;
        }
        let mut stalled: Option<usize> = None;
        for i in 0..self.cars.len() {
            let c = &self.cars[i];
            if c.gridlock_stall < GRIDLOCK_RELIEF_S || (ctx.visible)(self.x[i], self.z[i]) {
                continue;
            }
            if stalled.is_none_or(|s| c.gridlock_stall > self.cars[s].gridlock_stall) {
                stalled = Some(i);
            }
        }
        if let Some(i) = stalled {
            self.gridlock_cooldown = GRIDLOCK_RELIEF_GAP_S;
            if self.respawn(ctx, i, rng) {
                self.cars[i].gridlock_stall = 0.0;
                self.stats.gridlock_respawns += 1;
            }
        }
    }

    /// `respawnCar`: a clear, fit slot on a random lane.
    pub fn respawn(&mut self, ctx: &StepCtx, i: usize, rng: &mut Mulberry32) -> bool {
        let net = ctx.net;
        let mut scratch = std::mem::take(&mut self.respawn_scratch);
        scratch.clear();
        if let Some(f) = ctx.homing_focus {
            for t in net.tiles_near(f[0], f[1], LOCAL_RECYCLE_RADIUS) {
                scratch.extend_from_slice(&t.lanes);
            }
            scratch.sort_unstable();
            scratch.dedup();
        }
        let pool = scratch.len();
        let alive = if pool > 0 { pool } else { net.lanes.len() };
        // `respawnCar`'s draw pattern: up to 12 slots; the first one clear of a
        // 7 m same-lane gap decides (repaired RNG-free, refused if unfit).
        let mut placed = false;
        for _ in 0..12 {
            let k = (rng.next_f64() * alive as f64) as usize;
            let cand = (if pool > 0 { scratch[k] as usize } else { k }) as LaneId;
            let lane = &net.lanes[cand as usize];
            if !lane.alive {
                continue;
            }
            let segs = lane.pts.len().saturating_sub(1).max(1);
            let path_index = (rng.next_f64() * segs as f64) as usize;
            let seg_len = {
                let l = lane.cum.get(path_index + 1).copied().unwrap_or(lane.length) - lane.cum[path_index];
                if l == 0.0 { 1.0 } else { l }
            };
            let arc = lane.cum[path_index] + rng.next_f64() * seg_len;
            let clear = !self.cars.iter().enumerate().any(|(j, o)| j != i && o.lane == cand && (o.arc - arc).abs() < 7.0);
            if !clear {
                continue;
            }
            let repaired = self.repair_insert(net, cand, arc, i);
            let speed = 1.0 + rng.next_f64() * 2.0;
            if let Some(a) = repaired {
                if !self.landing_unfit(net, cand, a, i, speed) {
                    self.place(net, i, cand, a, speed);
                    self.stats.respawns += 1;
                    placed = true;
                }
            }
            break;
        }
        self.respawn_scratch = scratch;
        placed
    }

    /// Put car `i` on `lane` at `arc` (respawn / recycle): an atomic relocation.
    pub fn place(&mut self, net: &Network, i: usize, lane: LaneId, arc: f64, speed: f64) {
        self.release_claim(i);
        let c = &mut self.cars[i];
        c.lane = lane;
        c.arc = arc;
        c.speed = speed;
        c.max_speed = net.lanes[lane as usize].max_speed * c.pace;
        c.next = NO_LINK;
        c.corner_from = NONE;
        c.corner_link = NO_LINK;
        c.corner_s = 0.0;
        c.corner_replan = None;
        c.stop_released = false;
        c.stop_hold = 0.0;
        c.terminal_timer = 0.0;
        c.gridlock_stall = 0.0;
        c.junction_wait = 0.0;
        c.claim_stall = 0.0;
        c.turn_side = 0;
        self.write_pose(net, i);
    }

    /// `chooseNextEdge`: a weighted legal, drivable, reachable exit.
    fn choose_next(&mut self, ctx: &StepCtx, i: usize, rng: &mut Mulberry32) -> u16 {
        let net = ctx.net;
        let c = &self.cars[i];
        let lane = &net.lanes[c.lane as usize];
        self.weights.clear();
        let mut any_legal = false;
        for (k, link) in lane.links.iter().enumerate() {
            if !net.link_reachable(c.lane, k, c.arc) {
                continue;
            }
            if link.legal {
                any_legal = true;
            }
            self.weights.push((k as u16, 0.0));
        }
        if self.weights.is_empty() {
            return NO_LINK;
        }
        // A strict arrow with no legal exit holds at the line.
        if !any_legal && lane.role != geom::LaneRole::Through {
            return NO_LINK;
        }
        let a = lane.pts[lane.pts.len().saturating_sub(2)];
        let end = lane.pts[lane.pts.len() - 1];
        let homing = ctx.homing_focus.and_then(|f| {
            let d = geom::hypot(end[0] - f[0], end[1] - f[1]);
            (d > LOCAL_LIFE_RADIUS).then(|| {
                let ramp = js_clamp((d - LOCAL_LIFE_RADIUS) / (LOCAL_RECYCLE_RADIUS - LOCAL_LIFE_RADIUS).max(1.0), 0.0, 1.0);
                (ramp * HOMING_MAX_GAIN, (f[0] - end[0]) / d, (f[1] - end[1]) / d)
            })
        });
        let any_open = self.weights.iter().any(|&(k, _)| !net.lanes[lane.links[k as usize].to as usize].terminus);
        let mut total = 0.0;
        for w in self.weights.iter_mut() {
            let link = &lane.links[w.0 as usize];
            let to = &net.lanes[link.to as usize];
            let mut weight = if any_legal {
                if link.legal { lane.role.route_weight(link.movement) } else { 0.0 }
            } else {
                lane.role.route_weight(link.movement)
            };
            let ns = to.pts[0];
            if geom::hypot(ns[0] - a[0], ns[1] - a[1]) < 0.5 {
                weight *= 0.15;
            }
            if let Some((gain, fx, fz)) = homing {
                let o = to.pts[0];
                let p = to.pts[1.min(to.pts.len() - 1)];
                let (dx, dz) = (p[0] - o[0], p[1] - o[1]);
                let len = geom::hypot(dx, dz);
                let len = if len == 0.0 { 1.0 } else { len };
                let toward = (dx * fx + dz * fz) / len;
                if toward > 0.0 {
                    weight *= 1.0 + gain * toward;
                }
            }
            if any_open && to.terminus {
                weight = 0.0;
            }
            w.1 = weight;
            total += weight;
        }
        let mut pick = rng.next_f64() * total;
        for &(k, w) in &self.weights {
            pick -= w;
            if w > 0.0 && pick <= 0.0 {
                return k;
            }
        }
        self.weights.iter().rev().find(|w| w.1 > 0.0).map(|w| w.0).unwrap_or(self.weights[self.weights.len() - 1].0)
    }

    fn step_car(&mut self, ctx: &StepCtx, i: usize, dt: f64, rng: &mut Mulberry32) {
        let net = ctx.net;
        if self.cars[i].cornering() {
            self.step_corner(ctx, i, dt);
            return;
        }
        let lane_id = self.cars[i].lane;
        let lane = &net.lanes[lane_id as usize];
        let remaining0 = lane.length - self.cars[i].arc;
        let kind = self.cars[i].kind as usize;

        // Choose the onward lane early so braking and the turn cap know the manoeuvre.
        if !lane.terminus && self.cars[i].next == NO_LINK && remaining0 < (TURN_SIGNAL_DIST + 2.0).max(lane.hold[kind] + TURN_APPROACH_M) {
            let k = self.choose_next(ctx, i, rng);
            self.cars[i].next = k;
            self.cars[i].turn_side =
                if k != NO_LINK { lane.links[k as usize].plan.as_ref().map_or(0, |p| p.turn_side) } else { 0 };
        }
        let next = self.cars[i].next;
        let link = if next != NO_LINK { lane.links.get(next as usize) } else { None };
        let plan = link.and_then(|l| l.plan.as_ref());
        // A corner reached late re-plans from where the car stands, so its entry is here.
        let entry_setback = plan.map_or(0.0, |p| if remaining0 < p.entry_setback - 1e-6 { remaining0 } else { p.entry_setback });
        let terminus = lane.terminus || (next == NO_LINK && remaining0 < 2.0 + self.cars[i].len);

        let c = &self.cars[i];
        let (len, speed) = (c.len, c.speed);
        let remaining = remaining0;
        let mut target = c.max_speed;

        // Turn approach cap (the plan's sqrt(a_lat * r)).
        if link.is_some() && remaining < TURN_APPROACH_M + entry_setback + speed * speed / (2.0 * DECEL) {
            if let Some(p) = plan {
                if p.speed_cap.is_finite() {
                    target = target.min(p.speed_cap.max(2.2));
                }
            }
        }

        // Signal / stop control.
        let nose = len / 2.0;
        let (sig, bar_hold, bar_depth) = match lane.control {
            ControlKind::Signal { junction, axis, stop_offset_m, bar_depth_m } => {
                let a = net.junctions[junction as usize].aspect_at(axis as usize, ctx.time);
                (Some(a), Some(stop_offset_m + nose), bar_depth_m)
            }
            _ => (None, None, 0.5),
        };
        let stop_at = bar_hold.unwrap_or(LEGACY_STOP_LINE + nose);
        let stop_lookahead = stop_at + speed * speed / (2.0 * DECEL) + 2.0;
        let stop_armed = matches!(lane.control, ControlKind::Stop { enforced: true, .. })
            && !self.cars[i].stop_released
            && remaining <= stop_lookahead;
        if stop_armed && remaining <= stop_at + 0.6 && speed <= STALL_SPEED_MS {
            let hold_s = match lane.control {
                ControlKind::Stop { hold_s, .. } => hold_s,
                _ => 1.0,
            };
            self.cars[i].stop_hold += dt;
            if self.cars[i].stop_hold >= hold_s {
                self.cars[i].stop_released = true;
                self.stats.stop_sign_stops += 1;
            }
        }
        let hold_at_line = (sig == Some(RED) || sig == Some(YELLOW) || stop_armed) && remaining <= SIGNAL_LOOKAHEAD;
        let node_hold = len / 2.0 + 1.8;
        // `junctionHoldFor(edge, car)`: clear of every movement's swept path through the node.
        let geometric_hold = if link.is_some() { lane.hold[kind] } else { 0.0 };
        let approach_hold = if link.is_some() { geometric_hold.min(bar_hold.unwrap_or(f64::INFINITY)) } else { 0.0 };

        // Junction claims: a stalled holder lets go (`CLAIM_STALL_S`).
        let end_node = lane.end_node;
        self.cars[i].claim_yield = (self.cars[i].claim_yield - dt).max(0.0);
        if speed <= STALL_SPEED_MS && self.cars[i].claim_node != NONE {
            self.cars[i].claim_stall += dt;
            if self.cars[i].claim_stall >= CLAIM_STALL_S {
                self.release_claim(i);
                self.cars[i].claim_yield = CLAIM_YIELD_S;
                self.cars[i].claim_stall = 0.0;
                self.stats.claims_expired += 1;
            }
        } else {
            self.cars[i].claim_stall = 0.0;
        }
        let mut owned = self.holds(i, end_node);
        // The frontmost car on a lane goes first.
        if owned && self.leader_gap[i].is_finite() && self.leader_gap[i] < remaining {
            self.release_claim(i);
            owned = false;
        }
        let movement = link.and_then(|_| net.movement_index(end_node, lane_id, next));
        if let (Some(link), Some(m)) = (link, movement) {
            if remaining < approach_hold + TURN_APPROACH_M && !owned {
                let frontmost = !(self.leader_gap[i].is_finite() && self.leader_gap[i] < remaining);
                let exit_clear = match self.first_on_lane(link.to, i) {
                    None => true,
                    Some(j) => {
                        let exit = plan.map_or(0.0, |p| p.exit_arc);
                        let need = exit + len / 2.0 + self.cars[j].len / 2.0 + MIN_BUMPER_GAP + speed * speed / (2.0 * FOLLOW_DECEL);
                        self.cars[j].arc >= need || self.cars[j].arc >= net.lanes[link.to as usize].length - 0.5
                    }
                };
                let ready = !hold_at_line
                    && self.cars[i].claim_yield <= 0.0
                    && frontmost
                    && exit_clear
                    && self.box_compatible(net, end_node, m, i);
                let unowned_stop = approach_hold.max(entry_setback);
                if ready
                    && remaining
                        <= unowned_stop + (speed * speed / (2.0 * APPROACH_COMFORT_DECEL)).max(2.0) + speed * dt + 0.1
                {
                    self.take_claim(i, end_node, m);
                    owned = true;
                }
                if !owned {
                    target = target.min((2.0 * APPROACH_COMFORT_DECEL * (remaining - unowned_stop - 0.05).max(0.0)).sqrt());
                }
                if !ready {
                    let clear = (remaining - unowned_stop).max(0.0);
                    target = target.min(if clear <= 0.35 { 0.0 } else { (2.0 * DECEL * clear).sqrt() });
                }
                self.cars[i].junction_wait =
                    if !owned && !hold_at_line && speed <= STALL_SPEED_MS && remaining < unowned_stop + 3.0 {
                        self.cars[i].junction_wait + dt
                    } else {
                        0.0
                    };
            } else {
                self.cars[i].junction_wait = 0.0;
            }
        }

        // Signal hold at the painted bar.
        let mut committed = false;
        if hold_at_line {
            let stop_at = stop_at.max(entry_setback);
            let dist = remaining - stop_at;
            if dist >= 0.0 {
                let stop_speed = (2.0 * DECEL * dist.max(0.0)).sqrt();
                target = target.min(if dist < 1.4 { stop_speed.min(0.9) } else { stop_speed });
                if dist <= 0.35 {
                    target = 0.0;
                }
            } else {
                let overshoot = -dist;
                let braking = speed * speed / (2.0 * DECEL);
                if overshoot + braking <= bar_depth {
                    target = 0.0;
                } else {
                    committed = true;
                    if speed > 0.3 {
                        target = target.min(speed);
                    }
                }
            }
        }
        // Dead end.
        if terminus && remaining < (9.0f64).max(node_hold + 5.0) + speed * speed / (2.0 * DECEL) {
            target = target.min((2.0 * DECEL * (remaining - node_hold).max(0.0)).sqrt());
            if remaining <= node_hold + 0.3 {
                target = 0.0;
            }
        }
        // Leader, at the settle gap.
        let leader_gap = self.leader_gap[i];
        if leader_gap.is_finite() {
            let clearance = leader_gap - (len + self.leader_len[i]) / 2.0;
            if clearance <= FOLLOW_SETTLE_GAP {
                target = 0.0;
            } else {
                let room = clearance - FOLLOW_SETTLE_GAP;
                let creep = FOLLOW_CREEP_MPS.min(room / dt.max(1.0 / 120.0));
                target = target.min((2.0 * FOLLOW_DECEL * room).sqrt() + creep);
            }
        }
        // Cross corridor.
        let cross_gap = self.cross_gap[i];
        let creeping = self.cars[i].cross_wait >= CROSS_STALL_S;
        let cross_floor = if creeping { CROSS_CREEP_GAP } else { MIN_BUMPER_GAP };
        if cross_gap.is_finite() {
            if cross_gap <= cross_floor {
                target = 0.0;
            } else {
                target = target.min((2.0 * FOLLOW_DECEL * (cross_gap - cross_floor)).sqrt());
            }
            if creeping {
                target = target.min(CROSS_CREEP_SPEED);
            }
        }
        // Do not block the box: wait at the paint while it is busy.
        if let (Some(bar), Some(_)) = (bar_hold, link) {
            let busy = !owned && self.box_busy(net, end_node, i, movement);
            if remaining < bar + BOX_CLEAR_LOOKAHEAD_M && (busy || cross_gap.is_finite()) {
                let d = remaining - bar;
                let s = (2.0 * DECEL * d.max(0.0)).sqrt();
                target = target.min(if d < 1.4 { s.min(0.9) } else { s });
                if d <= 0.35 {
                    target = 0.0;
                }
            }
        }
        // People.
        let person_gap = self.person_gap[i];
        if person_gap.is_finite() {
            let clear = person_gap - PERSON_STOP_MARGIN_M;
            let mut pt = if clear <= 0.0 { 0.0 } else { (2.0 * PERSON_DECEL * clear).sqrt() };
            if self.cars[i].person_hold > PERSON_HOLD_MAX_S {
                pt = pt.max(PERSON_CREEP_MS);
            }
            target = target.min(pt);
            self.cars[i].person_hold = if speed <= 0.05 { self.cars[i].person_hold + dt } else { 0.0 };
        } else {
            self.cars[i].person_hold = 0.0;
        }
        let crossing_hold = self.crossing_hold[i];
        if crossing_hold.is_finite() && crossing_hold < 22.0 {
            target = target.min(if crossing_hold <= 0.3 { 0.0 } else { (2.0 * DECEL * crossing_hold).sqrt() });
        }

        // Rate limiter.
        let c = &mut self.cars[i];
        let rate = if target < c.speed { DECEL } else { ACCEL };
        c.speed += js_clamp(target - c.speed, -rate * dt, rate * dt);
        if target <= 0.02 && c.speed < 0.045 {
            c.speed = 0.0;
        }
        c.braking = target < c.speed - 0.08 || target <= 0.02;

        if terminus && c.speed == 0.0 {
            c.terminal_timer += dt;
            if c.terminal_timer > TERMINUS_RESPAWN_S {
                self.stats.terminus_respawns += 1;
                self.respawn(ctx, i, rng);
                return;
            }
        } else {
            c.terminal_timer = 0.0;
        }

        let c = &mut self.cars[i];
        let mut advance = c.speed * dt;
        if link.is_some() && !owned && remaining - advance < approach_hold {
            let to_hold = (remaining - approach_hold).max(0.0);
            c.speed = c.speed.min(if dt > 0.0 { to_hold / dt } else { 0.0 });
            advance = c.speed * dt;
        }
        if cross_gap.is_finite() {
            let max_adv = (cross_gap - cross_floor).max(0.0);
            if advance > max_adv {
                advance = max_adv;
                c.speed = if dt > 0.0 { advance / dt } else { 0.0 };
                c.braking = true;
            }
        }
        if person_gap.is_finite() {
            let max_adv = (person_gap - PERSON_EMERGENCY_M).max(0.0);
            if advance > max_adv {
                advance = max_adv;
                c.speed = c.speed.min(if dt > 0.0 { advance / dt } else { 0.0 });
            }
        }
        if leader_gap.is_finite() {
            let max_adv = (leader_gap - (len + self.leader_len[i]) / 2.0 - MIN_BUMPER_GAP).max(0.0);
            if advance > max_adv {
                advance = max_adv;
                c.speed = c.speed.min(if dt > 0.0 { advance / dt } else { 0.0 });
            }
        }
        // Begin the corner where the fillet is tangent to this lane.
        if let Some(link) = link {
            if remaining - advance <= entry_setback {
                if (!hold_at_line || committed) && owned {
                    let lead = (advance - (remaining - entry_setback).max(0.0)).max(0.0);
                    if self.begin_corner(net, i, link.to, lead) {
                        if sig == Some(RED) {
                            if committed {
                                self.stats.committed_on_red += 1;
                            } else {
                                self.stats.red_entries += 1;
                            }
                        }
                        return;
                    }
                }
                let c = &mut self.cars[i];
                advance = advance.min((remaining - entry_setback).max(0.0));
                c.speed = c.speed.min(if dt > 0.0 { advance / dt } else { 0.0 });
                c.braking = true;
            }
        }
        let c = &mut self.cars[i];
        c.arc += advance;
        c.travelled += advance;
        if c.arc >= lane.length {
            c.arc = lane.length;
            c.speed = c.speed.min(0.4);
        }
    }

    /// `beginCorner`: commit to the movement. A corner reached past its
    /// memoised entry is re-planned from where the car stands
    /// (`cornerFilletAt`); one that cannot be re-planned is refused.
    fn begin_corner(&mut self, net: &Network, i: usize, to: LaneId, lead: f64) -> bool {
        let c = &self.cars[i];
        let from = c.lane;
        let link_index = c.next;
        let lane = &net.lanes[from as usize];
        let memo = lane.links[link_index as usize].plan.as_ref();
        let remaining = lane.length - c.arc;
        let replan = match memo {
            Some(p) if remaining < p.entry_setback - 1e-6 => match net.plan_from_arc(from, link_index as usize, c.arc) {
                Some(r) => Some(Box::new(r)),
                None => return false,
            },
            _ => None,
        };
        if replan.is_some() {
            self.stats.late_corners += 1;
        }
        let c = &mut self.cars[i];
        c.corner_replan = replan;
        let (arc_len, exit, kind) = c
            .corner_replan
            .as_deref()
            .or(memo)
            .map_or((0.5, 0.0, PlanKind::Line), |p| (p.arc_len, p.exit_arc, p.kind));
        c.claim_exit = exit;
        c.corner_from = from;
        c.corner_link = link_index;
        c.corner_s = lead.min(arc_len);
        c.lane = to;
        c.arc = exit - (arc_len - c.corner_s);
        c.next = NO_LINK;
        c.max_speed = net.lanes[to as usize].max_speed * c.pace;
        c.stop_released = false;
        c.stop_hold = 0.0;
        c.travelled += lead;
        if kind == PlanKind::Line && arc_len <= lead {
            self.finish_corner(net, i);
        }
        true
    }

    fn finish_corner(&mut self, net: &Network, i: usize) {
        let c = &self.cars[i];
        let (arc_len, exit) = c.corner_plan(net).map_or((0.5, 0.0), |p| (p.arc_len, p.exit_arc));
        let c = &mut self.cars[i];
        c.arc = (exit + (c.corner_s - arc_len).max(0.0)).min(net.lanes[c.lane as usize].length);
        c.corner_from = NONE;
        c.corner_link = NO_LINK;
        c.corner_s = 0.0;
        c.corner_replan = None;
        c.turn_side = 0;
    }

    fn step_corner(&mut self, ctx: &StepCtx, i: usize, dt: f64) {
        let net = ctx.net;
        let c = &self.cars[i];
        let (arc_len, cap, exit) =
            c.corner_plan(net).map_or((0.5, 4.0, 0.0), |p| (p.arc_len, p.speed_cap.min(4.0).max(1.2), p.exit_arc));
        let mut speed = c.speed;
        if speed < cap {
            speed = cap.min(speed + ACCEL * dt);
        } else if speed > cap {
            speed = cap.max(speed - DECEL * dt);
        }
        // A car on the arc brakes only to avoid contact (`CROSS_CREEP_GAP`).
        let cg = self.cross_gap[i];
        if cg.is_finite() {
            let safe = (cg - CROSS_CREEP_GAP).max(0.0);
            let s = (2.0 * FOLLOW_DECEL * safe).sqrt();
            if s < speed {
                speed = s.max(speed - DECEL * dt);
            }
            if safe <= 0.0 || speed * dt > safe {
                speed = 0.0;
            }
        }
        // Following through the arc.
        let lg = self.leader_gap[i];
        if lg.is_finite() {
            let room = lg - (c.len + self.leader_len[i]) / 2.0 - MIN_BUMPER_GAP;
            let s = (2.0 * FOLLOW_DECEL * room.max(0.0)).sqrt();
            if s < speed {
                speed = s.max(speed - DECEL * dt);
            }
            if room <= 0.0 {
                speed = 0.0;
            }
        }
        let pg = self.person_gap[i];
        if pg.is_finite() {
            let clear = pg - PERSON_STOP_MARGIN_M;
            let t = if clear <= 0.0 { 0.0 } else { (2.0 * PERSON_DECEL * clear).sqrt() };
            if t < speed {
                speed = t.max(speed - DECEL * dt);
            }
            if t <= 0.02 && speed < 0.06 {
                speed = 0.0;
            }
        }
        let mut advance = speed * dt;
        if pg.is_finite() {
            advance = advance.min((pg - PERSON_EMERGENCY_M).max(0.0));
        }
        let c = &mut self.cars[i];
        c.speed = speed;
        c.braking = speed < 0.05;
        c.corner_s += advance;
        c.travelled += advance;
        c.arc = exit - (arc_len - c.corner_s);
        if c.corner_s >= arc_len - 1e-9 {
            self.finish_corner(net, i);
        }
    }

    /// After every car has moved: a claim is held until the car has carried its
    /// whole presented body clear of the movement's exit.
    pub fn settle_claims(&mut self, net: &Network) {
        for i in 0..self.cars.len() {
            let c = &self.cars[i];
            if c.claim_node == NONE || c.cornering() {
                continue;
            }
            if c.claim_node != net.lanes[c.lane as usize].end_node && c.arc - c.len / 2.0 > c.claim_exit + 0.5 {
                self.release_claim(i);
            }
        }
    }

    /// The resident network changed (tiles streamed in or out): node movement
    /// tables and link indices were rebuilt, so every claim and every chosen
    /// exit is dropped and re-decided; a car mid-corner keeps its movement if
    /// the link still exists and is set down on its exit lane if not.
    pub fn network_changed(&mut self, net: &Network) {
        self.claims.clear();
        for i in 0..self.cars.len() {
            let c = &mut self.cars[i];
            c.claim_node = NONE;
            c.next = NO_LINK;
            if c.cornering() {
                let from = &net.lanes[c.corner_from as usize];
                match from.links.iter().position(|l| l.to == c.lane) {
                    Some(k) if from.alive && net.lanes[c.lane as usize].alive => c.corner_link = k as u16,
                    _ => {
                        c.corner_from = NONE;
                        c.corner_link = NO_LINK;
                        c.corner_replan = None;
                        c.arc = c.arc.max(0.0);
                    }
                }
            }
            self.write_pose(net, i);
        }
    }

    pub fn aspect_name(a: u8) -> &'static str {
        match a {
            RED => "red",
            YELLOW => "yellow",
            GREEN => "green",
            _ => "?",
        }
    }
}

/// `crossConflictYieldsTo(car, other)`.
fn yields_to(c: &Car, o: &Car) -> bool {
    if o.speed <= 0.5 {
        return true;
    }
    if c.speed <= 0.5 && o.speed > 0.5 {
        return false;
    }
    let mine = (c.claim_node != NONE) as i32;
    let theirs = (o.claim_node != NONE) as i32;
    if mine != theirs {
        return theirs > mine;
    }
    o.id < c.id
}
