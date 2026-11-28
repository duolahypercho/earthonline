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

use std::collections::HashMap;

use crate::consts::*;
use crate::corridor::{Params, idm_accel};
use crate::geom::{self, Movement, P2, Plan, PlanKind, js_clamp};
use crate::grid::Grid;
use crate::network::{ControlKind, Lane, LaneId, MovementPose, NONE, Network, NodeId};
use crate::rng::{Mulberry32, driver_pace};
use crate::signals::{GREEN, RED, YELLOW};

pub const NO_LINK: u16 = u16::MAX;
pub const KIND_NAMES: [&str; 4] = ["sedan", "taxi", "truck", "bus"];
const LOCAL_CORNER_SPEED_MPS: f64 = 4.0;

mod stops;

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
    pub stop_arrival: Option<f64>,
    pub terminal_timer: f64,
    pub gridlock_stall: f64,
    pub cross_wait: f64,
    pub person_hold: f64,
    pub travelled: f64,
    pub braking: bool,
    pub turn_side: i8,
    horn_until: f64,
    horn_ready_at: f64,
    pass_signal_at: Option<f64>,
    /// Level of detail: stepped every `SIM_TIER_FAR_EVERY` steps with the summed time.
    pub far: bool,
    pub lod_debt: f64,
}

impl Car {
    #[inline]
    pub fn cornering(&self) -> bool {
        self.corner_from != NONE
    }

    pub fn passing(&self) -> bool {
        self.cornering() && self.corner_link == NO_LINK && self.corner_replan.is_some()
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

/// An externally driven vehicle occupying a lane or junction.
#[derive(Clone, Copy, Debug)]
pub struct TrafficObstacle {
    pub pos: [f64; 2],
    pub yaw: f64,
    pub length: f64,
    pub width: f64,
    pub level: i8,
}

/// A chassis supplied by the authoritative collision world.
#[derive(Clone, Copy, Debug)]
pub struct TrafficObservation {
    pub fleet_id: Option<u32>,
    pub obstacle: TrafficObstacle,
    pub surface_y: f64,
}

#[derive(Clone, Copy, Debug, Default)]
struct ClaimSlot {
    node: NodeId,
    n: u8,
    car: [u32; 6],
    movement: [u32; 6],
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
struct MovementFootprint {
    node: NodeId,
    movement: usize,
    length: u64,
    width: u64,
}

impl MovementFootprint {
    fn new(node: NodeId, movement: usize, car: &Car) -> Self {
        Self { node, movement, length: car.len.to_bits(), width: car.wid.to_bits() }
    }

    fn dimensions(self) -> (f64, f64) {
        (f64::from_bits(self.length), f64::from_bits(self.width))
    }
}

/// The fleet of one region.
#[derive(Default)]
pub struct Fleet {
    pub cars: Vec<Car>,
    pub x: Vec<f64>,
    pub z: Vec<f64>,
    pub yaw: Vec<f64>,
    pub spd: Vec<f64>,
    /// The structure level of each car's lane (`Lane::level`; the lane it
    /// is turning onto while cornering): cars on different levels never
    /// block or see each other, and only cars at grade meet people.
    pub level: Vec<i8>,
    /// Height of the occupied road surface, NaN for legacy lane builds.
    pub surface_y: Vec<f64>,
    leader_gap: Vec<f64>,
    leader_len: Vec<f64>,
    leader_speed: Vec<f64>,
    cross_gap: Vec<f64>,
    person_gap: Vec<f64>,
    crossing_hold: Vec<f64>,
    // Per-step route advance allowed by the position of a contacted chassis.
    physical_advance: Vec<f64>,
    pedestrian_advance: Vec<f64>,
    pass_clear: Vec<bool>,
    pass_bounds: Option<[[f64; 2]; 2]>,
    order: Vec<(u64, u32)>,
    grid: Option<Grid>,
    claims: Vec<ClaimSlot>,
    /// Last station on the holder's path that still conflicts with the proposal.
    sweep_conflicts: HashMap<[MovementFootprint; 2], Option<f64>>,
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
    pub hazard_horns: u64,
    pub passes_started: u64,
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
    /// Is the replacement's full chassis clear of people at its landing pose?
    pub landing_clear: &'a dyn Fn(TrafficObstacle) -> bool,
}

fn presented(kind: u8) -> (f64, f64) {
    (PRESENTED_LENGTH[kind as usize] as f64, PRESENTED_WIDTH[kind as usize] as f64)
}

fn model_dimensions(net: &Network, seed: u32, id: u32, kind: u8) -> (f64, f64) {
    net.driving_model(seed, id, kind).and_then(|model| model.driving_dimensions())
        .unwrap_or_else(|| presented(kind))
}

#[derive(Clone, Copy)]
pub(crate) struct Landing {
    car: usize,
    kind: u8,
    len: f64,
    wid: f64,
}

impl Landing {
    /// Keep the whole vehicle on the receiving lane.
    pub(crate) fn arc(self, lane: &Lane, preferred: f64) -> Option<f64> {
        (lane.length >= self.len).then(|| preferred.clamp(self.len / 2.0, lane.length - self.len / 2.0))
    }

    pub(crate) fn body(self, lane: &Lane, arc: f64) -> TrafficObstacle {
        let (x, z, yaw) = lane.vehicle_pose(arc, self.len);
        TrafficObstacle { pos: [x, z], yaw, length: self.len, width: self.wid, level: lane.level }
    }

    fn hold(self, lane: &Lane) -> f64 {
        let hold = lane.hold[self.kind as usize];
        match lane.control {
            ControlKind::MappedStop { stop_offset_m, .. } => hold.max(stop_offset_m + self.len / 2.0),
            _ => hold,
        }
    }

    pub(crate) fn unfit(self, net: &Network, lane: LaneId, arc: f64, speed: f64) -> bool {
        if arc < self.len / 2.0 || !net.any_reachable_for(lane, arc, self.len, self.wid) {
            return true;
        }
        let lane = &net.lanes[lane as usize];
        let brake = speed * speed / (2.0 * APPROACH_COMFORT_DECEL) + 1.0;
        lane.length - arc < self.hold(lane) + TURN_APPROACH_M / 2.0 + brake
    }
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

/// When the path curves, the nose, flanks and tail follow different arcs.
/// Use the same chassis heading and corner splice as the simulation step.
fn curved_person_motion<'a>(
    net: &'a Network, car: &'a Car, reach: f64,
) -> Option<impl Fn(f64) -> (f64, f64, f64) + 'a> {
    let lane = &net.lanes[car.lane as usize];
    let corner = if car.cornering() {
        car.corner_plan(net).map(|plan| {
            let from = &net.lanes[car.corner_from as usize];
            (-car.corner_s, MovementPose::new(from, lane, plan, car.len))
        })
    } else if car.claim_node == lane.end_node {
        lane.links.get(car.next as usize).filter(|link| link.drivable).and_then(|link| {
            let plan = link.plan.as_ref()?;
            let to = &net.lanes[link.to as usize];
            let start = lane.length - plan.entry_setback - car.arc;
            // A straight seam can still lead directly into a curved lane.
            (lane.level == to.level && (0.0..reach).contains(&start)
                && (plan.kind != PlanKind::Line || to.bend_within(plan.exit_arc, reach - start, car.len)))
                .then(|| (start, MovementPose::new(lane, to, plan, car.len)))
        })
    } else {
        None
    };
    (corner.is_some() || (!car.cornering() && lane.bend_within(car.arc, reach, car.len)))
        .then(move || move |distance: f64| {
            if let Some((start, motion)) = &corner && distance >= *start {
                motion.at(distance - start)
            } else {
                lane.vehicle_pose((car.arc + distance).clamp(0.0, lane.length), car.len)
            }
        })
}

fn person_sweep_gap(
    point: P2, radius: f64, car: &Car, reach: f64, pose: &impl Fn(f64) -> (f64, f64, f64),
) -> f64 {
    let count = ((reach / JUNCTION_SWEEP_STEP_M).ceil() as usize).max(1);
    let step = reach / count as f64;
    let limit = car.len.hypot(car.wid) / 2.0 + radius;
    let clearance = |advance| {
        let (x, z, yaw) = pose(advance);
        let dx = x - point[0];
        let dz = z - point[1];
        if dx * dx + dz * dz > limit * limit {
            return f64::INFINITY;
        }
        geom::obb_signed_clearance(point[0], point[1], (x, z, yaw, car.len, car.wid))
    };
    let start = clearance(0.0);
    if start <= radius {
        // A pedestrian can reach the chassis from the side after sensing.
        // Let the car take one step away if the sampled clearance increases.
        let escape = reach.min((car.speed + ACCEL * FIXED_STEP_S) * FIXED_STEP_S);
        let middle = clearance(escape / 2.0);
        if middle > start && clearance(escape) >= middle {
            return escape;
        }
        return 0.0;
    }
    for i in 1..=count {
        let (mut clear, mut occupied) = ((i - 1) as f64 * step, i as f64 * step);
        if clearance(occupied) <= radius {
            // Keep the car on the clear side. Refine to let it advance up to
            // the actual clearance instead of stopping a full sample early.
            for _ in 0..6 {
                let middle = clear.midpoint(occupied);
                if clearance(middle) <= radius { occupied = middle; } else { clear = middle; }
            }
            return clear;
        }
    }
    f64::INFINITY
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

/// The gap to a body near a bend. Use the same vehicle
/// pose as movement, since the ray along the current heading can point into
/// the other lane even when the driving path curves away from it.
fn bend_gap(lane: &Lane, car: &Car, other_size: [f64; 2], other_poses: &[(f64, f64, f64)], look: f64) -> Option<f64> {
    let look = look.min(lane.length - car.arc).max(0.0);
    let [length, width] = other_size;
    let reach = (car.len.hypot(car.wid) + length.hypot(width)) / 2.0;
    let reach_squared = reach * reach;
    let count = ((look / JUNCTION_SWEEP_STEP_M).ceil() as usize).max(1);
    for step in 0..=count {
        let advance = look * step as f64 / count as f64;
        let (x, z, yaw) = lane.vehicle_pose(car.arc + advance, car.len);
        if other_poses.iter().any(|&(ox, oz, heading)| {
            let (dx, dz) = (x - ox, z - oz);
            dx * dx + dz * dz <= reach_squared && geom::obb_penetration(
                (x, z, yaw, car.len, car.wid), (ox, oz, heading, length, width),
            ) > 0.0
        }) {
            return Some(look * step.saturating_sub(1) as f64 / count as f64);
        }
    }
    None
}

/// Keep behind the portion of an adjacent vehicle's path it can occupy
/// while its body swings through the bend.
fn adjacent_bend_gap(
    lane: &Lane, car: &Car, other_lane: &Lane, other: &Car, body: (f64, f64, f64), look: f64, other_look: f64,
) -> Option<f64> {
    const MAX_SAMPLES: usize = (CROSS_LOOK_MAX / JUNCTION_SWEEP_STEP_M) as usize + 2;
    let travel = other_look.min(other_lane.length - other.arc).max(0.0);
    let count = (travel / JUNCTION_SWEEP_STEP_M).ceil() as usize;
    let mut poses = [(0.0, 0.0, 0.0); MAX_SAMPLES];
    let poses = &mut poses[..=count];
    poses[0] = body;
    for (step, pose) in poses.iter_mut().enumerate().skip(1) {
        let arc = other.arc + travel * step as f64 / count as f64;
        *pose = other_lane.vehicle_pose(arc, other.len);
    }
    bend_gap(lane, car, [other.len, other.wid], poses, look)
}

struct CornerSurface<'a> {
    through_exit: Option<&'a Lane>,
    plan: &'a Plan,
    ends: [Option<f64>; 2],
    pad: Option<&'a crate::network::JunctionSurface>,
    same_level: bool,
}

impl<'a> CornerSurface<'a> {
    fn new(from: &'a Lane, to: &'a Lane, plan: &'a Plan) -> Self {
        let pad = from.junction_to(to);
        Self {
            through_exit: pad.filter(|_| geom::classify_movement(&from.pts, &to.pts) == Movement::Through).map(|_| to),
            plan,
            ends: [from.surface_at(from.length - plan.entry_setback), to.surface_at(plan.exit_arc)],
            pad,
            same_level: from.level == to.level,
        }
    }

    fn fallback(&self, fraction: f64) -> Option<f64> {
        match self.ends {
            [Some(a), Some(b)] => Some(a + (b - a) * fraction),
            [a, b] if self.same_level => a.or(b),
            _ => None,
        }
    }

    fn at(&self, position: P2, fraction: f64) -> Option<f64> {
        let fallback = self.fallback(fraction);
        let Some(pad) = self.pad else { return fallback };
        if let Some(y) = pad.height_near(position, fallback.unwrap_or_default()) {
            return Some(y);
        }
        let length = self.plan.arc_len;
        let station = fraction * length;
        if let Some(exit) = self.through_exit {
            // The straight continuation beyond the pad follows its receiving road.
            let arc = self.plan.exit_arc - (length - station);
            if arc >= 0.0 {
                return exit.surface_at(arc).or(fallback);
            }
        }
        let seams = self.plan.junction_grade_seams.get_or_init(|| {
            pad.movement_seams(self.plan, |arc| self.fallback(arc / length).unwrap_or_default())
        });
        let next = seams.partition_point(|knot| knot[0] <= station);
        let start = next.checked_sub(1).map(|i| seams[i])
            .or_else(|| self.fallback(0.0).map(|y| [0.0, y]));
        let end = seams.get(next).copied()
            .or_else(|| self.fallback(1.0).map(|y| [length, y]));
        match (start, end) {
            (Some([a, ya]), Some([b, yb])) if a < b => Some(ya + (yb - ya) * ((station - a) / (b - a)).clamp(0.0, 1.0)),
            _ => fallback,
        }
    }

    fn along(&self, from: &Lane, exit: &Lane, plan: &Plan, position: P2, distance: f64) -> (i8, f64) {
        let (level, height) = if distance <= plan.arc_len {
            let fraction = distance / plan.arc_len.max(f64::EPSILON);
            (from.level, self.at(position, fraction))
        } else {
            (exit.level, exit.surface_at(plan.exit_arc + distance - plan.arc_len))
        };
        (level, height.unwrap_or(f64::NAN))
    }
}

/// Last overlapping station on the held path, including the receiving lane.
/// Both movements are compared at their own road heights.
fn movement_conflict_end<'a>(net: &'a Network, bodies: [MovementFootprint; 2], replans: [Option<&'a Plan>; 2]) -> Option<f64> {
    let movement = |body: MovementFootprint, replan: Option<&'a Plan>| {
        let (lane, link) = net.nodes[body.node as usize].movements[body.movement];
        let from = &net.lanes[lane as usize];
        let link = &from.links[link as usize];
        let plan = replan.or(link.plan.as_ref())?;
        Some((from, link.to, plan))
    };
    let (from_a, to_a, plan_a) = movement(bodies[0], replans[0])?;
    let (from_b, to_b, plan_b) = movement(bodies[1], replans[1])?;
    let exit_a = &net.lanes[to_a as usize];
    let exit_b = &net.lanes[to_b as usize];
    let same_node = bodies[0].node == bodies[1].node;
    if !same_node {
        // A chord between entries bounds the space both sweeps can reach.
        let (x, z, _) = plan_a.pose_at(0.0);
        let (u, v, _) = plan_b.pose_at(0.0);
        let reach = |plan: &Plan, body: MovementFootprint| {
            let (len, wid) = body.dimensions();
            plan.arc_len + len / 2.0 + CLAIM_RELEASE_MARGIN_M + JUNCTION_SWEEP_STEP_M + len.hypot(wid) / 2.0
        };
        if (x - u).hypot(z - v) > reach(plan_a, bodies[0]) + reach(plan_b, bodies[1]) {
            return None;
        }
    }
    let (samples_a, bounds_a) = net.junction_samples(from_a, to_a, plan_a, bodies[0].dimensions(), 0.0, false);
    let (samples_b, bounds_b) = net.junction_samples(from_b, to_b, plan_b, bodies[1].dimensions(), 0.0, same_node);
    if bounds_a[0] > bounds_b[2] || bounds_b[0] > bounds_a[2] || bounds_a[1] > bounds_b[3] || bounds_b[1] > bounds_a[3] {
        return None;
    }
    let surface_a = CornerSurface::new(from_a, exit_a, plan_a);
    let surface_b = CornerSurface::new(from_b, exit_b, plan_b);
    let swept_a: Vec<_> = samples_a.iter().map(|a| {
        let y = surface_a.along(from_a, exit_a, plan_a, [a.body.0, a.body.1], a.distance);
        (a.body, y, a.body.3.hypot(a.body.4) / 2.0)
    }).collect();
    samples_b.iter().rev().find(|b| {
        let yb = surface_b.along(from_b, exit_b, plan_b, [b.body.0, b.body.1], b.distance);
        let radius = b.body.3.hypot(b.body.4) / 2.0;
        swept_a.iter().any(|&(a, ya, r)| {
            let (dx, dz) = (a.0 - b.body.0, a.1 - b.body.1);
            dx * dx + dz * dz < (radius + r).powi(2)
                && net.shares_surface(ya, yb) && geom::obb_penetration(a, b.body) > 0.0
        })
    }).map(|sample| sample.distance + JUNCTION_SWEEP_STEP_M)
}

impl Fleet {
    pub(crate) fn surface(&self, i: usize) -> (i8, f64) {
        (self.level[i], self.surface_y[i])
    }

    /// Diagnostic flags from the most recent sensing pass, before cars and
    /// walkers moved. This is an observation, not a braking-reason decision.
    pub fn pedestrian_observation(&self, i: usize) -> (bool, bool) {
        (self.person_gap[i].is_finite(), self.crossing_hold[i].is_finite())
    }

    pub fn horn_active(&self, i: usize, time: f64) -> bool {
        time < self.cars[i].horn_until
    }

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
        self.level.push(0);
        self.surface_y.push(f64::NAN);
        self.leader_gap.push(f64::INFINITY);
        self.leader_len.push(4.0);
        self.leader_speed.push(0.0);
        self.cross_gap.push(f64::INFINITY);
        self.person_gap.push(f64::INFINITY);
        self.crossing_hold.push(f64::INFINITY);
        self.pass_clear.push(false);
    }

    /// `spawnCar(edge, color, random, entityId)`: the JS draw sequence, so a
    /// fleet spawned on the JS lane order from the JS seed picks the JS kinds and lanes.
    pub fn spawn(&mut self, net: &Network, lanes: &[LaneId], rng: &mut Mulberry32, seed_int: u32) -> bool {
        self.spawn_near(net, lanes, rng, seed_int, None)
    }

    pub(crate) fn spawn_near(&mut self, net: &Network, lanes: &[LaneId], rng: &mut Mulberry32, seed_int: u32, near: Option<([f64; 2], f64)>) -> bool {
        if lanes.is_empty() {
            return false;
        }
        let id = self.next_id; // was `self.cars.len()`, the same number while no car is handed off
        let first = lanes[(rng.next_f64() * lanes.len() as f64) as usize];
        let _paint = rng.next_f64();
        let kind = pick_kind(rng.next_f64());
        let (len, wid) = model_dimensions(net, seed_int, id, kind);
        let mut placement = None;
        for attempt in 0..10 {
            let cand = if attempt == 0 { first } else { lanes[(rng.next_f64() * lanes.len() as f64) as usize] };
            let lane = &net.lanes[cand as usize];
            if !lane.fits_vehicle(len, wid) {
                continue;
            }
            let segs = (lane.pts.len().saturating_sub(1)).max(1);
            let path_index = (rng.next_f64() * segs as f64) as usize;
            let seg_len = {
                let l = lane.cum.get(path_index + 1).copied().unwrap_or(lane.length) - lane.cum[path_index];
                if l == 0.0 { 1.0 } else { l }
            };
            let arc = lane.cum[path_index] + rng.next_f64() * seg_len;
            if let Some(([x, z], radius)) = near.filter(|(_, r)| lane.length > *r) {
                let (px, pz, _, _, _) = geom::at_arc(&lane.pts, &lane.cum, arc);
                if (px - x).hypot(pz - z) > radius {
                    continue;
                }
            }
            let bar = matches!(lane.control,
                ControlKind::Signal { .. } | ControlKind::Stop { enforced: true, .. } | ControlKind::MappedStop { .. });
            if bar && arc > (lane.length - net.stop_offset(cand) - len / 2.0).max(0.0) - 0.25 {
                continue;
            }
            let clear = if near.is_some() {
                // Start a live car wholly on the lane, behind the junction
                // hold, with a reachable exit and body clearance.
                arc >= len / 2.0
                    && lane.length - arc >= lane.hold[kind as usize].max(len / 2.0)
                    && net.any_reachable_for(cand, arc, len, wid)
                    && !self.slot_occupied(net, cand, arc, (len, wid), None)
            } else {
                !self.cars.iter().any(|o| o.lane == cand && (o.arc - arc).abs() < 7.0)
            };
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
            stop_arrival: None,
            terminal_timer: 0.0,
            gridlock_stall: 0.0,
            cross_wait: 0.0,
            person_hold: 0.0,
            travelled: 0.0,
            braking: false,
            turn_side: 0,
            horn_until: 0.0,
            horn_ready_at: 0.0,
            pass_signal_at: None,
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
        self.level.swap_remove(i);
        self.surface_y.swap_remove(i);
        self.leader_gap.swap_remove(i);
        self.leader_len.swap_remove(i);
        self.leader_speed.swap_remove(i);
        self.cross_gap.swap_remove(i);
        self.person_gap.swap_remove(i);
        self.crossing_hold.swap_remove(i);
        self.pass_clear.swap_remove(i);
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
        self.landing_blocked(net, lane, arc, self.current_landing(me))
    }

    pub(crate) fn landing_blocked(&self, net: &Network, lane: LaneId, arc: f64, landing: Landing) -> bool {
        let l = &net.lanes[lane as usize];
        if !l.fits_vehicle(landing.len, landing.wid) {
            return true;
        }
        let has_exits = l.links.iter().any(|k| k.drivable);
        if has_exits && !net.any_reachable_for(lane, arc, landing.len, landing.wid) {
            return true;
        }
        if has_exits && arc > l.length - landing.hold(l) {
            return true;
        }
        self.slot_occupied(net, lane, arc, (landing.len, landing.wid), Some(landing.car))
    }

    fn slot_occupied(&self, net: &Network, lane_id: LaneId, arc: f64, (len, wid): (f64, f64), ignore: Option<usize>) -> bool {
        let lane = &net.lanes[lane_id as usize];
        let (x, z, yaw) = lane.vehicle_pose(arc, len);
        let surface = (lane.level, lane.surface_at(arc).unwrap_or(f64::NAN));
        const INSERT_MARGIN: f64 = 0.30; // QUEUE_SEPARATION.insertMargin
        for (j, o) in self.cars.iter().enumerate() {
            if ignore == Some(j) {
                continue;
            }
            let shares_surface = net.shares_surface(surface, self.surface(j));
            if o.lane == lane_id {
                let headway = (len + o.len) / 2.0 + MIN_BUMPER_GAP + FOLLOW_SETTLE_MARGIN;
                let blocks_slot = if !o.cornering() {
                    shares_surface && (o.arc - arc).abs() < headway.max(7.0)
                } else if !o.passing() {
                    // An incoming turn still owns space on the receiving lane.
                    // Checking its present box alone can insert a new car in its sweep.
                    arc < o.corner_plan(net).map_or(0.0, |plan| plan.exit_arc) + headway
                } else {
                    false
                };
                if blocks_slot {
                    return true;
                }
            }
            if !shares_surface {
                continue;
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

    fn current_landing(&self, car: usize) -> Landing {
        let c = &self.cars[car];
        Landing { car, kind: c.kind, len: c.len, wid: c.wid }
    }

    /// The body of the incoming model must fit before the old ID is retired.
    pub(crate) fn prepare_landing(&self, net: &Network, car: usize, replacement_seed: Option<u32>) -> Landing {
        let mut landing = self.current_landing(car);
        if let Some(seed) = replacement_seed.filter(|_| net.model_footprints.is_some()) {
            (landing.len, landing.wid) = model_dimensions(net, seed, self.next_id, landing.kind);
        }
        landing
    }

    #[cfg(test)]
    pub(crate) fn landing_arc(&self, lane: &Lane, preferred: f64, car: usize) -> Option<f64> {
        self.current_landing(car).arc(lane, preferred)
    }

    #[cfg(test)]
    pub(crate) fn landing_body(&self, lane: &Lane, arc: f64, car: usize) -> TrafficObstacle {
        self.current_landing(car).body(lane, arc)
    }

    /// `landingUnfit(car, edge, arc, speed)`: fit behind the junction hold with
    /// braking room and leave the whole body on a lane with a reachable exit.
    pub fn landing_unfit(&self, net: &Network, lane: LaneId, arc: f64, me: usize, speed: f64) -> bool {
        self.current_landing(me).unfit(net, lane, arc, speed)
    }

    /// `repairQueueInsert`: the nearest clear arc in 2.75 m strides, ahead first.
    pub fn repair_insert(&self, net: &Network, lane: LaneId, arc: f64, me: usize) -> Option<f64> {
        self.repair_landing(net, lane, arc, self.current_landing(me))
    }

    fn repair_landing(&self, net: &Network, lane: LaneId, arc: f64, landing: Landing) -> Option<f64> {
        let l = &net.lanes[lane as usize];
        if !l.fits_vehicle(landing.len, landing.wid) {
            return None;
        }
        if !self.landing_blocked(net, lane, arc, landing) {
            return Some(arc);
        }
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
                if !self.landing_blocked(net, lane, c, landing) {
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
        let lane = &net.lanes[car.lane as usize];
        let (x, z, yaw) = if car.cornering() {
            match car.corner_plan(net) {
                Some(p) => MovementPose::new(&net.lanes[car.corner_from as usize], lane, p, car.len).at(car.corner_s),
                None => {
                    let (x, z, dx, dz, _) = geom::at_arc(&lane.pts, &lane.cum, 0.0);
                    (x, z, dx.atan2(dz))
                }
            }
        } else {
            lane.vehicle_pose(car.arc.clamp(0.0, lane.length), car.len)
        };
        self.x[i] = x;
        self.z[i] = z;
        self.yaw[i] = yaw;
        self.spd[i] = car.speed;
        self.level[i] = lane.level;
        let surface = if car.cornering() {
            car.corner_plan(net).and_then(|plan| {
                let surface = CornerSurface::new(&net.lanes[car.corner_from as usize], lane, plan);
                let fraction = (car.corner_s / plan.arc_len.max(f64::EPSILON)).clamp(0.0, 1.0);
                surface.at([x, z], fraction)
            })
        } else {
            lane.surface_at(car.arc)
        };
        self.surface_y[i] = surface.unwrap_or(f64::NAN);
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

    fn last_on_lane(&self, lane: LaneId) -> Option<usize> {
        let end = self.order.partition_point(|e| (e.0 >> 32) <= lane as u64);
        let &(key, i) = self.order.get(end.checked_sub(1)?)?;
        (key >> 32 == lane as u64).then_some(i as usize)
    }

    fn note_leader(&mut self, follower: usize, leader: usize, gap: f64) {
        if gap < self.leader_gap[follower] {
            self.leader_gap[follower] = gap;
            self.leader_len[follower] = self.cars[leader].len;
            self.leader_speed[follower] = self.cars[leader].speed;
        }
    }

    /// Both lanes remain occupied until the body has moved clear of its source lane.
    fn sense_passing_bodies(&mut self, net: &Network) {
        for i in 0..self.cars.len() {
            let c = &self.cars[i];
            if !c.passing() {
                continue;
            }
            let source = c.corner_from;
            let lane = &net.lanes[source as usize];
            let (x, z, dx, dz, _) = geom::at_arc(&lane.pts, &lane.cum, 0.0);
            let rx = self.x[i] - x;
            let rz = self.z[i] - z;
            let arc = rx * dx + rz * dz;
            let lateral = (rx * dz - rz * dx).abs();
            let wid = c.wid;
            let mut k = self.order.partition_point(|e| (e.0 >> 32) < source as u64);
            while let Some(&(_, j)) = self.order.get(k).filter(|e| (e.0 >> 32) == source as u64) {
                let j = j as usize;
                let other = &self.cars[j];
                if lateral < (wid + other.wid) / 2.0 + PERSON_SIDE_MARGIN_M {
                    let lead = other.arc - arc;
                    if lead >= 0.0 {
                        self.note_leader(i, j, lead);
                    } else {
                        self.note_leader(j, i, -lead);
                    }
                }
                k += 1;
            }
        }
    }

    // ------------------------------------------------------------ sense

    /// Every gap a car's speed law reads, from the fleet as the last step left it.
    pub fn sense(&mut self, net: &Network, walkers_x: &[f64], walkers_z: &[f64], walker_grid: &Grid, bands: &[Band]) {
        let n = self.cars.len();
        self.physical_advance.clear();
        self.pedestrian_advance.clear();
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
                    self.leader_speed[i] = self.cars[j].speed;
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
            let follow_look = net.following_time_gap_s.filter(|_| geom::freeway(&lane.highway))
                .map_or(40.0, |gap| (c.speed * gap).max(40.0));
            if remaining > follow_look {
                continue;
            }
            if let Some(j) = self.first_on_lane(link.to, i) {
                let (setback, arc_len, exit) =
                    link.plan.as_ref().map_or((0.0, 0.0, 0.0), |p| (p.entry_setback.clamp(0.0, remaining), p.arc_len, p.exit_arc));
                self.leader_gap[i] = remaining - setback + arc_len + (self.cars[j].arc - exit);
                self.leader_len[i] = self.cars[j].len;
                self.leader_speed[i] = self.cars[j].speed;
            }
        }
        self.sense_passing_bodies(net);
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
            let lane = &net.lanes[c.lane as usize];
            let on_bend = !c.cornering() && lane.bend_within(c.arc, look, c.len);
            let (x0, z0, x1, z1) = if on_bend {
                let radius = look + (c.len.hypot(c.wid)
                    + f64::from(PRESENTED_LENGTH[3]).hypot(f64::from(PRESENTED_WIDTH[3]))) / 2.0;
                (x - radius, z - radius, x + radius, z + radius)
            } else {
                (x.min(x + fx * look) - 8.0, z.min(z + fz * look) - 8.0,
                    x.max(x + fx * look) + 8.0, z.max(z + fz * look) + 8.0)
            };
            let mut best = f64::INFINITY;
            let cars = &self.cars;
            let (xs, zs, yaws) = (&self.x, &self.z, &self.yaw);
            grid.query(x0, z0, x1, z1, |j| {
                let j = j as usize;
                if j == i || !net.shares_surface(self.surface(i), self.surface(j)) {
                    return;
                }
                let o = &cars[j];
                let body = (xs[j], zs[j], yaws[j]);
                if o.lane == c.lane {
                    // A loop can bring a car back toward another that is
                    // well behind it along the lane. Ordinary close followers
                    // still yield to their leader.
                    let separation = o.arc - c.arc;
                    let follower_gap = (c.len + o.len) / 2.0 + MIN_BUMPER_GAP;
                    if !c.cornering() && !o.cornering()
                        && (separation > 0.0 || separation < -follower_gap)
                        && lane.bend_within(c.arc.min(o.arc), separation.abs(), c.len.max(o.len))
                        && let Some(gap) = bend_gap(lane, c, [o.len, o.wid], &[body], look)
                    {
                        best = best.min(gap);
                    }
                    return;
                }
                let other_lane = &net.lanes[o.lane as usize];
                let other_look = cross_lookahead(o.speed);
                // Nearby vehicles on either side can sweep across the
                // divider as their chassis turn through the same bend.
                if !c.cornering() && !o.cornering() && lane.segment == other_lane.segment
                    && (on_bend || other_lane.bend_within(o.arc, other_look, o.len))
                {
                    // Give the front vehicle room to clear the bend. A
                    // vehicle in the next lane can close on its rear corner
                    // even though the current footprints are still clear.
                    let gap = if lane.end_node == other_lane.end_node
                        && (other_lane.length - o.arc, o.id) < (lane.length - c.arc, c.id)
                    {
                        adjacent_bend_gap(lane, c, other_lane, o, body, look, other_look)
                    } else {
                        bend_gap(lane, c, [o.len, o.wid], &[body], look)
                    };
                    if let Some(gap) = gap {
                        best = best.min(gap);
                    }
                    return;
                }
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
            let mut best = f64::INFINITY;
            // (people walk at grade: a car on a deck or in a tunnel meets none.)
            let at_grade = self.level[i] == 0;
            if at_grade && !walkers_x.is_empty() {
                let (x0, x1) = (x.min(x + fx * reach) - half, x.max(x + fx * reach) + half);
                let (z0, z1) = (z.min(z + fz * reach) - half, z.max(z + fz * reach) + half);
                walker_grid.query(x0, z0, x1, z1, |w| {
                    let w = w as usize;
                    let dx = walkers_x[w] - x;
                    let dz = walkers_z[w] - z;
                    let fwd = dx * fx + dz * fz;
                    let lat = (dx * fz - dz * fx).abs();
                    if fwd > 0.0 && fwd < reach && lat < half {
                        best = best.min(fwd - c.len / 2.0 - PERSON_CAPSULE_R_M);
                    }
                });
                // The pose check already covers the front and rear of the
                // body. Scan the stopping distance and room for one sample.
                let sweep = (c.speed * c.speed / (2.0 * PERSON_DECEL)
                    + PERSON_EMERGENCY_M + JUNCTION_SWEEP_STEP_M).min(reach);
                if let Some(pose) = curved_person_motion(net, c, sweep) {
                    let radius = sweep + c.len.hypot(c.wid) / 2.0 + PERSON_CAPSULE_R_M;
                    let mut gap = f64::INFINITY;
                    walker_grid.query(x - radius, z - radius, x + radius, z + radius, |w| {
                        let w = w as usize;
                        let point = [walkers_x[w], walkers_z[w]];
                        if (point[0] - x).powi(2) + (point[1] - z).powi(2) <= radius * radius {
                            gap = gap.min(person_sweep_gap(point, PERSON_CAPSULE_R_M, c, sweep.min(gap), &pose));
                        }
                    });
                    if gap.is_finite() {
                        // Off-axis contact already includes the person's capsule and
                        // the last clear chassis pose. Use that bound for movement.
                        self.pedestrian_advance.resize(n, f64::INFINITY);
                        self.pedestrian_advance[i] = gap;
                        best = best.min(gap + PERSON_STOP_MARGIN_M);
                    }
                }
            }
            self.person_gap[i] = if best.is_finite() { best.max(0.0) } else { f64::INFINITY };
            // Occupied crosswalks the car's path crosses within the yield lookahead
            // (`occupiedCrossingHold`): stop with the nose short of the band.
            let mut hold = f64::INFINITY;
            if !bands.is_empty() && !c.cornering() && at_grade {
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
            self.pass_clear[i] = false;
            let Some(pass) = net.lanes[c.lane as usize].passing else { continue };
            if c.cornering() || c.speed <= STALL_SPEED_MS || !self.leader_gap[i].is_finite() || hold.is_finite() {
                continue;
            }
            let Some(gap) = net.following_time_gap_s else { continue };
            let p = Params::with_time_gap(gap);
            let (a, b) = (&net.lanes[c.lane as usize], &net.lanes[pass.to as usize]);
            let forward = c.max_speed * (p.change_s + p.time_gap_s) + c.len;
            let rear = c.max_speed * p.time_gap_s + c.len;
            let lateral = (a.pts[0][0] - b.pts[0][0]).hypot(a.pts[0][1] - b.pts[0][1]);
            let half = c.wid / 2.0 + PERSON_SIDE_MARGIN_M + PERSON_CAPSULE_R_M;
            let corners = [
                [x - fx * rear - fz * half, z - fz * rear + fx * half],
                [x - fx * rear + fz * (lateral + half), z - fz * rear - fx * (lateral + half)],
                [x + fx * forward - fz * half, z + fz * forward + fx * half],
                [x + fx * forward + fz * (lateral + half), z + fz * forward - fx * (lateral + half)],
            ];
            let lo = [corners.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min), corners.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min)];
            let hi = [corners.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max), corners.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max)];
            if self.pass_bounds.is_some_and(|[a, b]| lo[0] < a[0] || lo[1] < a[1] || hi[0] >= b[0] || hi[1] >= b[1]) {
                continue;
            }
            let mut safe = true;
            walker_grid.query(lo[0], lo[1], hi[0], hi[1], |w| {
                let rx = walkers_x[w as usize] - x;
                let rz = walkers_z[w as usize] - z;
                let fwd = rx * fx + rz * fz;
                let left = rx * fz - rz * fx;
                if (-rear..=forward).contains(&fwd) && (-half..=lateral + half).contains(&left) {
                    safe = false;
                }
            });
            self.pass_clear[i] = safe;
        }
    }

    pub(crate) fn set_pass_bounds(&mut self, bounds: Option<[[f64; 2]; 2]>) {
        self.pass_bounds = bounds;
    }

    pub fn sense_obstacle(&mut self, net: &Network, obstacle: TrafficObstacle) {
        self.sense_external(net, obstacle, false, None);
    }

    pub fn sense_person(&mut self, net: &Network, obstacle: TrafficObstacle) {
        self.sense_external(net, obstacle, true, None);
    }

    pub(crate) fn sense_traffic_observation(&mut self, net: &Network, observed: TrafficObservation) {
        if let Some(id) = observed.fleet_id && let Some(i) = self.cars.iter().position(|car| car.id == id) {
            let (sin, cos) = self.yaw[i].sin_cos();
            let [x, z] = observed.obstacle.pos;
            let ahead = (x - self.x[i]) * sin + (z - self.z[i]) * cos;
            // Keep the route target within half a body length in front of
            // the chassis, where the driver can steer back toward the lane.
            self.physical_advance.resize(self.cars.len(), f64::INFINITY);
            self.physical_advance[i] = self.physical_advance[i].min((ahead + self.cars[i].len / 2.0).max(0.0));
            self.pass_clear[i] = false;
        }
        self.sense_external(net, observed.obstacle, false, Some(observed));
    }

    fn sense_external(&mut self, net: &Network, obstacle: TrafficObstacle, person: bool, observed: Option<TrafficObservation>) {
        let [x, z] = obstacle.pos;
        for (i, c) in self.cars.iter().enumerate() {
            let same_surface = observed.map_or(self.level[i] == obstacle.level, |observation| {
                observation.fleet_id != Some(c.id) && net.shares_surface(self.surface(i), (obstacle.level, observation.surface_y))
            });
            if !same_surface {
                continue;
            }
            let distance = (self.x[i] - x).hypot(self.z[i] - z);
            if self.pass_clear[i] {
                // External traffic and the player are outside the lane order.
                let reach = c.max_speed * Params::with_time_gap(0.0).change_s + c.len + obstacle.length;
                if distance <= reach {
                    self.pass_clear[i] = false;
                }
            }
            let look = c.speed * c.speed / (2.0 * FOLLOW_DECEL) + cross_lookahead(c.speed);
            let reach = look + (c.len.hypot(c.wid) + obstacle.length.hypot(obstacle.width)) / 2.0;
            if distance > reach {
                continue;
            }
            if person && let Some(pose) = curved_person_motion(net, c, look) {
                let swept = person_sweep_gap([x, z], obstacle.length.max(obstacle.width) / 2.0, c, look, &pose);
                if swept.is_finite() {
                    self.pedestrian_advance.resize(self.cars.len(), f64::INFINITY);
                    self.pedestrian_advance[i] = self.pedestrian_advance[i].min(swept);
                    self.person_gap[i] = self.person_gap[i].min(swept + PERSON_STOP_MARGIN_M);
                }
            }
            let mut gap = cross_corridor_gap(self.x[i], self.z[i], self.yaw[i], c.len, c.wid, x, z, obstacle.yaw, obstacle.length, obstacle.width, look);
            let lane = &net.lanes[c.lane as usize];
            // The rear quarter can swing into a chassis beside the tangent.
            // Test the same footprint poses the car will drive.
            let sweep = gap.unwrap_or(look);
            if !person && sweep > 0.0 && !c.cornering()
                && lane.bend_within(c.arc, sweep, c.len)
                && let Some(swept) = bend_gap(lane, c, [obstacle.length, obstacle.width], &[(x, z, obstacle.yaw)], sweep)
            {
                gap = Some(swept);
            }
            if let Some(gap) = gap {
                if person {
                    self.person_gap[i] = self.person_gap[i].min(gap);
                } else {
                    self.cross_gap[i] = self.cross_gap[i].min(gap);
                }
            }
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

    fn swept_claims_clear(&mut self, net: &Network, node: NodeId, m: usize, i: usize) -> bool {
        let c = &self.cars[i];
        let proposed = MovementFootprint::new(node, m, c);
        let (_, link) = net.nodes[node as usize].movements[m];
        let approach = &net.lanes[c.lane as usize];
        let proposal = &approach.links[link as usize];
        let exit = &net.lanes[proposal.to as usize];
        let mut late = None;
        for slot in &self.claims {
            for k in 0..slot.n as usize {
                let j = slot.car[k] as usize;
                if i == j { continue; }
                let other = &self.cars[j];
                let holder = MovementFootprint::new(slot.node, slot.movement[k] as usize, other);
                let (from, ix) = net.nodes[slot.node as usize].movements[holder.movement];
                let holder_lane = &net.lanes[from as usize];
                let link = &holder_lane.links[ix as usize];
                // A short connecting lane can carry both junction sweeps.
                if node != slot.node && approach.start_node != slot.node && exit.end_node != slot.node
                    && holder_lane.start_node != node && net.lanes[link.to as usize].end_node != node
                {
                    continue;
                }
                let proposed_late = late.get_or_insert_with(|| proposal.plan.as_ref()
                    .filter(|p| approach.length - c.arc < p.entry_setback - 1e-6)
                    .and_then(|_| net.plan_from_arc(c.lane, c.next as usize, c.arc)));
                let bodies = [proposed, holder];
                let replans = [proposed_late.as_ref(), other.corner_replan.as_deref()];
                let end = if replans.iter().any(Option::is_some) {
                    movement_conflict_end(net, bodies, replans)
                } else {
                    *self.sweep_conflicts.entry(bodies).or_insert_with(|| movement_conflict_end(net, bodies, [None, None]))
                };
                let Some(end) = end else { continue };
                let progress = if other.cornering() && other.corner_from == from {
                    other.corner_s
                } else if other.lane == link.to {
                    link.plan.as_ref().map_or(0.0, |p| p.arc_len + other.arc - p.exit_arc)
                } else {
                    0.0
                };
                if progress <= end { return false; }
            }
        }
        true
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
        self.record_stops(ctx.net, ctx.time);
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
            self.warn_of_hazard(i, ctx.time);
            self.step_car(ctx, i, step_dt, rng);
            let c = &mut self.cars[i];
            c.gridlock_stall = if c.speed <= STALL_SPEED_MS { c.gridlock_stall + step_dt } else { 0.0 };
            c.cross_wait = if self.cross_gap[i].is_finite() && c.speed <= 0.5 { c.cross_wait + step_dt } else { 0.0 };
            self.write_pose(ctx.net, i);
        }
        self.relieve_gridlock(ctx, dt, rng);
    }

    /// The horn accompanies braking for an immediate collision hazard.
    /// A stopped queue and a person with ample yielding distance stay quiet.
    fn warn_of_hazard(&mut self, i: usize, time: f64) {
        let c = &mut self.cars[i];
        if c.speed <= STALL_SPEED_MS || time < c.horn_ready_at {
            return;
        }
        let closing = c.speed - self.leader_speed[i];
        let leader_clearance = self.leader_gap[i] - (c.len + self.leader_len[i]) / 2.0 - MIN_BUMPER_GAP;
        let car_hazard = (closing > STALL_SPEED_MS && leader_clearance <= closing * closing / (2.0 * FOLLOW_DECEL))
            || self.cross_gap[i] <= c.speed * c.speed / (2.0 * FOLLOW_DECEL);
        let person_hazard = self.person_gap[i] - PERSON_STOP_MARGIN_M <= c.speed * c.speed / (2.0 * DECEL);
        if car_hazard || person_hazard {
            // Hold the sound trigger across the render and snapshot cadences.
            c.horn_until = time + 0.25;
            c.horn_ready_at = time + 10.0;
            self.stats.hazard_horns += 1;
        }
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
        let live = ctx.homing_focus.is_some();
        if live && (ctx.visible)(self.x[i], self.z[i]) {
            return false;
        }
        let net = ctx.net;
        let landing = self.prepare_landing(net, i, live.then_some(ctx.seed_int));
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
            if alive == 0 {
                break;
            }
            let k = (rng.next_f64() * alive as f64) as usize;
            let cand = (if pool > 0 { scratch[k] as usize } else { k }) as LaneId;
            let lane = &net.lanes[cand as usize];
            if !lane.alive || !lane.fits_vehicle(landing.len, landing.wid) {
                continue;
            }
            let segs = lane.pts.len().saturating_sub(1).max(1);
            let path_index = (rng.next_f64() * segs as f64) as usize;
            let seg_len = {
                let l = lane.cum.get(path_index + 1).copied().unwrap_or(lane.length) - lane.cum[path_index];
                if l == 0.0 { 1.0 } else { l }
            };
            let arc = lane.cum[path_index] + rng.next_f64() * seg_len;
            let Some(arc) = (if live { landing.arc(lane, arc) } else { Some(arc) }) else { continue };
            let clear = !self.cars.iter().enumerate().any(|(j, o)| j != i && o.lane == cand && (o.arc - arc).abs() < 7.0);
            if !clear {
                continue;
            }
            let repaired = self.repair_landing(net, cand, arc, landing);
            let speed = 1.0 + rng.next_f64() * 2.0;
            if let Some(a) = repaired {
                if !landing.unfit(net, cand, a, speed) {
                    if !(ctx.landing_clear)(landing.body(lane, a)) {
                        continue;
                    }
                    if live {
                        let (x, z, _, _, _) = geom::at_arc(&lane.pts, &lane.cum, a);
                        if (ctx.visible)(x, z) {
                            continue;
                        }
                        self.replace(net, i, cand, a, speed, ctx.seed_int);
                    } else {
                        self.place(net, i, cand, a, speed);
                    }
                    self.stats.respawns += 1;
                    placed = true;
                }
            }
            break;
        }
        self.respawn_scratch = scratch;
        placed
    }

    /// End one trip and start a different vehicle in a checked off-camera slot.
    pub fn replace(&mut self, net: &Network, i: usize, lane: LaneId, arc: f64, speed: f64, seed: u32) {
        let landing = self.prepare_landing(net, i, Some(seed));
        let c = &mut self.cars[i];
        c.id = self.next_id;
        (c.len, c.wid) = (landing.len, landing.wid);
        self.next_id += 1;
        c.pace = driver_pace(c.id, seed, DRIVER_PACE_MIN, DRIVER_PACE_SPAN);
        c.travelled = 0.0;
        c.cross_wait = 0.0;
        c.person_hold = 0.0;
        c.claim_yield = 0.0;
        c.braking = false;
        c.lod_debt = 0.0;
        self.place(net, i, lane, arc, speed);
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
        c.stop_arrival = None;
        c.terminal_timer = 0.0;
        c.gridlock_stall = 0.0;
        c.junction_wait = 0.0;
        c.claim_stall = 0.0;
        c.turn_side = 0;
        c.horn_until = 0.0;
        c.horn_ready_at = 0.0;
        c.pass_signal_at = None;
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
            if !net.lanes[link.to as usize].fits_vehicle(c.len, c.wid) || !net.link_reachable(c.lane, k, c.arc) {
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

    fn pass_window(&self, net: &Network, i: usize) -> Option<(LaneId, f64, Params)> {
        let c = &self.cars[i];
        if !self.pass_clear[i] || c.next != NO_LINK || c.claim_node != NONE
            || self.cross_gap[i].is_finite() || self.person_gap[i].is_finite()
        {
            return None;
        }
        let lane = &net.lanes[c.lane as usize];
        let pass = lane.passing?;
        let to = &net.lanes[pass.to as usize];
        if !to.alive || !to.fits_vehicle(c.len, c.wid) || to.width < c.wid || lane.width < c.wid
            || !to.links.iter().any(|l| l.legal && l.drivable && l.movement == Movement::Through)
        {
            return None;
        }
        let p = Params::with_time_gap(net.following_time_gap_s?);
        let lateral = (lane.pts[0][0] - to.pts[0][0]).hypot(lane.pts[0][1] - to.pts[0][1]);
        let distance = (c.speed * p.change_s).max(geom::passing_distance(lateral));
        let duration = distance / c.speed;
        let end = c.arc + distance;
        let buffer = c.len / 2.0 + MIN_BUMPER_GAP;
        let junction = to.hold[c.kind as usize].max(TURN_SIGNAL_DIST);
        let braking = c.speed * c.speed / (2.0 * APPROACH_COMFORT_DECEL);
        if c.arc - buffer < pass.span[0] || end + buffer > pass.span[1]
            || to.length - end < junction + braking + buffer
        {
            return None;
        }
        let lead_speed = self.leader_speed[i];
        let gap = self.leader_gap[i] - (c.len + self.leader_len[i]) / 2.0;
        let closure = (c.speed - lead_speed).max(0.0) * duration;
        if lead_speed <= STALL_SPEED_MS || lead_speed >= c.max_speed
            || gap < MIN_BUMPER_GAP + closure
            || gap > FOLLOW_SETTLE_GAP + c.speed * p.time_gap_s + closure
        {
            return None;
        }
        let desired = c.max_speed.min(to.max_speed * c.pace);
        let current = idm_accel(&p, c.speed, c.max_speed, Some((gap, c.speed - lead_speed)));
        let mut ahead = None;
        for (j, other) in self.cars.iter().enumerate() {
            if j == i || other.lane != pass.to && !(other.passing() && other.corner_from == pass.to) {
                continue;
            }
            let delta = other.arc - c.arc;
            let half = (c.len + other.len) / 2.0;
            if delta >= 0.0 {
                let gap = delta - half;
                if gap < p.min_gap_m + c.speed * p.time_gap_s + (desired - other.speed).max(0.0) * duration {
                    return None;
                }
                if ahead.is_none_or(|(g, _)| gap < g) {
                    ahead = Some((gap, c.speed - other.speed));
                }
            } else {
                let gap = -delta - half;
                let closing = other.speed - c.speed;
                if gap < p.min_gap_m + other.speed * p.time_gap_s + closing.max(0.0) * duration
                    || idm_accel(&p, other.speed, other.max_speed, Some((gap, closing))) < -p.safe_decel
                {
                    return None;
                }
            }
        }
        (idm_accel(&p, c.speed, desired, ahead) - current > p.change_threshold).then_some((pass.to, end, p))
    }

    fn try_pass(&mut self, ctx: &StepCtx, i: usize) -> bool {
        let Some((target, end, p)) = self.pass_window(ctx.net, i) else {
            if self.cars[i].pass_signal_at.take().is_some() {
                self.cars[i].turn_side = 0;
            }
            return false;
        };
        let c = &mut self.cars[i];
        let since = *c.pass_signal_at.get_or_insert(ctx.time);
        c.turn_side = 1;
        if ctx.time - since < p.change_s / 3.0 {
            return false;
        }
        let source = &ctx.net.lanes[c.lane as usize];
        let dest = &ctx.net.lanes[target as usize];
        let (x, z, dx, dz, _) = geom::at_arc(&source.pts, &source.cum, c.arc);
        let (tx, tz, _, _, _) = geom::at_arc(&dest.pts, &dest.cum, end);
        let Some(plan) = geom::passing_plan([x, z], [tx, tz], [dx, dz], source.length - c.arc, end) else {
            c.pass_signal_at = None;
            c.turn_side = 0;
            return false;
        };
        c.corner_from = c.lane;
        c.corner_link = NO_LINK;
        c.corner_s = 0.0;
        c.arc = end - plan.arc_len;
        c.lane = target;
        c.corner_replan = Some(Box::new(plan));
        c.max_speed = c.max_speed.min(dest.max_speed * c.pace);
        c.pass_signal_at = None;
        self.stats.passes_started += 1;
        true
    }

    fn step_car(&mut self, ctx: &StepCtx, i: usize, dt: f64, rng: &mut Mulberry32) {
        let net = ctx.net;
        if self.cars[i].cornering() {
            self.step_corner(ctx, i, dt);
            return;
        }
        if self.try_pass(ctx, i) {
            self.step_corner(ctx, i, dt);
            return;
        }
        let lane_id = self.cars[i].lane;
        let lane = &net.lanes[lane_id as usize];
        let remaining0 = lane.length - self.cars[i].arc;
        let kind = self.cars[i].kind as usize;
        let freeway = geom::freeway(&lane.highway);

        // Choose the onward lane early so braking and the turn cap know the manoeuvre.
        let mut choice_distance = (TURN_SIGNAL_DIST + 2.0).max(lane.hold[kind] + TURN_APPROACH_M);
        if freeway {
            let speed = self.cars[i].speed;
            choice_distance += speed * speed / (2.0 * APPROACH_COMFORT_DECEL);
        }
        if !lane.terminus && self.cars[i].next == NO_LINK && remaining0 < choice_distance {
            let k = self.choose_next(ctx, i, rng);
            self.cars[i].next = k;
            self.cars[i].turn_side =
                if k != NO_LINK { lane.links[k as usize].plan.as_ref().map_or(0, |p| p.turn_side) } else { 0 };
        }
        let turn_blocked = self.check_right_turn_approach(net, i, dt);
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

        if net.bend_braking {
            target = lane.bend_speed_target(c.arc, len, target);
            if let Some(link) = link {
                let exit = &net.lanes[link.to as usize];
                let exit_cruise = exit.max_speed * c.pace;
                let exit_arc = plan.map_or(0.0, |p| p.exit_arc);
                let exit_target = exit.bend_speed_target(exit_arc, len, exit_cruise);
                if exit_target < exit_cruise {
                    let approach = (remaining - entry_setback).max(0.0) + plan.map_or(0.0, |p| p.arc_len);
                    target = target.min((exit_target * exit_target + 2.0 * APPROACH_COMFORT_DECEL * approach).sqrt());
                }
            }
        }

        // Slow to the receiving freeway lane or curve before entering it.
        // Read the same cap as the corner step, with a stopping-distance envelope.
        if let Some(link) = link.filter(|l| freeway && geom::freeway(&net.lanes[l.to as usize].highway)) {
            let exit_speed = net.lanes[link.to as usize].max_speed * c.pace;
            let entry_speed = plan.map_or(exit_speed, |p| exit_speed.min(p.speed_cap.max(1.2)));
            let approach = (remaining - entry_setback).max(0.0);
            target = target.min((entry_speed * entry_speed + 2.0 * APPROACH_COMFORT_DECEL * approach).sqrt());
        } else if link.is_some() && remaining < TURN_APPROACH_M + entry_setback + speed * speed / (2.0 * DECEL) {
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
            ControlKind::MappedStop { stop_offset_m, bar_depth_m, .. } => (None, Some(stop_offset_m + nose), bar_depth_m),
            _ => (None, None, 0.5),
        };
        let stop_at = bar_hold.unwrap_or(LEGACY_STOP_LINE + nose);
        let mapped = stops::stop_station(net, c);
        let mapped_hold = mapped.is_some() && c.stop_arrival.is_none();
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
        let hold_at_line = mapped_hold || ((sig == Some(RED) || sig == Some(YELLOW) || stop_armed) && remaining <= SIGNAL_LOOKAHEAD);
        let node_hold = len / 2.0 + 1.8;
        // `junctionHoldFor(edge, car)`: clear of every movement's swept path through the node.
        let geometric_hold = if link.is_some() { lane.hold[kind] } else { 0.0 };
        let approach_hold = if link.is_some() {
            mapped.map_or_else(|| geometric_hold.min(bar_hold.unwrap_or(f64::INFINITY)), |station| lane.length - station)
        } else { 0.0 };

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
        if let (Some(link), Some(m)) = (link, movement) && !mapped_hold {
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
                    && !turn_blocked
                    && (if mapped.is_some() {
                        self.mapped_stop_clear(ctx, i, m)
                    } else {
                        link.movement != Movement::Left || !self.opposing_traffic(ctx, i, m)
                    })
                    && self.box_compatible(net, end_node, m, i);
                let unowned_stop = approach_hold.max(entry_setback);
                if ready
                    && remaining
                        <= unowned_stop + (speed * speed / (2.0 * APPROACH_COMFORT_DECEL)).max(2.0) + speed * dt + 0.1
                    && (link.movement == Movement::Right || self.turn_path_clear(net, i, next as usize))
                    && self.swept_claims_clear(net, end_node, m, i)
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
        if let Some(station) = mapped.filter(|_| mapped_hold) {
            target = target.min((2.0 * DECEL * (station - self.cars[i].arc).max(0.0)).sqrt());
        } else if hold_at_line {
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
                if let Some(gap) = net.following_time_gap_s.filter(|_| freeway) {
                    target = target.min(room / gap);
                }
            }
        }
        // Cross corridor.
        let cross_gap = self.cross_gap[i].min(self.reserved_turn_gap(net, i));
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
        c.braking = c.speed < speed || target <= 0.02;

        if terminus && c.speed == 0.0 {
            c.terminal_timer += dt;
            if c.terminal_timer > TERMINUS_RESPAWN_S {
                c.terminal_timer = 0.0;
                self.stats.terminus_respawns += 1;
                self.respawn(ctx, i, rng);
                return;
            }
        } else {
            c.terminal_timer = 0.0;
        }

        let c = &mut self.cars[i];
        let mut advance = (c.speed * dt).min(self.pedestrian_advance.get(i).copied().unwrap_or(f64::INFINITY));
        if advance < c.speed * dt {
            c.speed = advance / dt;
            c.braking = true;
        }
        advance = advance.min(self.physical_advance.get(i).copied().unwrap_or(f64::INFINITY));
        if let Some(station) = mapped.filter(|_| mapped_hold) {
            let remaining_to_stop = (station - c.arc).max(0.0);
            if advance >= remaining_to_stop {
                advance = remaining_to_stop;
                c.speed = 0.0;
                c.braking = true;
            }
        }
        if link.is_some() && !owned && remaining - advance < approach_hold {
            let to_hold = (remaining - approach_hold).max(0.0);
            advance = advance.min(to_hold);
            c.speed = c.speed.min(if dt > 0.0 { advance / dt } else { 0.0 });
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
                    if self.turn_path_clear(net, i, next as usize) && self.begin_corner(net, i, link.to, lead)
                    {
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

    /// Check the approaching driver's right turn before claiming space.
    /// If the original turn is blocked, try the curb-side link to the same segment.
    /// Returns true when the driver must wait.
    fn check_right_turn_approach(&mut self, net: &Network, i: usize, dt: f64) -> bool {
        let c = &self.cars[i];
        let current = c.next as usize;
        let lane = &net.lanes[c.lane as usize];
        let Some(link) = lane.links.get(current).filter(|l| l.movement == Movement::Right) else { return false };
        let Some(plan) = &link.plan else { return false };
        let remaining = lane.length - c.arc;
        let hold = match lane.control {
            ControlKind::Signal { stop_offset_m, .. } => lane.hold[c.kind as usize].min(stop_offset_m + c.len / 2.0),
            _ => lane.hold[c.kind as usize],
        };
        let claim_range = hold.max(plan.entry_setback) + (c.speed * c.speed / (2.0 * APPROACH_COMFORT_DECEL)).max(2.0) + c.speed * dt + 0.1;
        if c.claim_node == lane.end_node || remaining > claim_range
            || self.leader_gap[i] < remaining || self.turn_path_clear(net, i, current)
        {
            return false;
        }
        let target = &net.lanes[link.to as usize];
        let [x0, z0] = target.pts[0];
        let [x1, z1] = target.pts[1];
        let lateral = |to: LaneId| {
            let [x, z] = net.lanes[to as usize].pts[0];
            (x - x0) * (z1 - z0) - (z - z0) * (x1 - x0)
        };
        let alternative = lane.links.iter().enumerate().filter(|(_, other)| {
            let out = &net.lanes[other.to as usize];
            other.movement == link.movement && out.segment == target.segment
                && out.fits_vehicle(c.len, c.wid) && lateral(other.to) > 0.0
        }).max_by(|(_, a), (_, b)| lateral(a.to).total_cmp(&lateral(b.to)));
        if let Some((index, other)) = alternative {
            if link.legal && other.legal && other.drivable
                && (target.terminus || !net.lanes[other.to as usize].terminus)
                && self.first_on_lane(other.to, i).is_none()
                && net.movement_index(lane.end_node, c.lane, index as u16)
                    .is_some_and(|m| self.box_compatible(net, lane.end_node, m, i))
                && self.turn_path_clear(net, i, index)
            {
                self.cars[i].next = index as u16;
                return false;
            }
        }
        true
    }

    /// Test the planned sweep against nearby bodies, including how far a
    /// moving body may travel under service braking during a new reservation.
    fn turn_path_clear(&self, net: &Network, i: usize, ix: usize) -> bool {
        let c = &self.cars[i];
        let lane = &net.lanes[c.lane as usize];
        let Some(p) = lane.links[ix].plan.as_ref() else { return true };
        let remaining = lane.length - c.arc;
        let late;
        let plan = if remaining < p.entry_setback - 1e-6 {
            late = net.plan_from_arc(c.lane, ix, c.arc);
            let Some(plan) = &late else { return false };
            plan
        } else {
            p
        };
        let Some(grid) = &self.grid else { return true };
        let exit = &net.lanes[lane.links[ix].to as usize];
        let surface = CornerSurface::new(lane, exit, plan);
        let (len, wid) = (c.len, c.wid);
        let motion = MovementPose::new(lane, exit, plan, len);
        let (x0, z0, _) = motion.at(0.0);
        let release = (exit.length - plan.exit_arc).clamp(0.0, len / 2.0 + CLAIM_RELEASE_MARGIN_M);
        let total = plan.arc_len + release;
        let query_radius = total + len * 0.5 + f64::from(PRESENTED_LENGTH[3]) + CROSS_LOOK_MIN;
        let count = ((total / JUNCTION_SWEEP_STEP_M).ceil() as usize).max(1);
        let mut clear = true;
        grid.query(x0 - query_radius, z0 - query_radius, x0 + query_radius, z0 + query_radius, |j| {
            let j = j as usize;
            if !clear || j == i || self.cars[j].lane == c.lane
                || (!self.surface_y[j].is_finite() && self.level[j] != lane.level && self.level[j] != exit.level)
            {
                return;
            }
            let o = &self.cars[j];
            let (ol, ow) = (o.len, o.wid);
            let (ox, oz, oy) = (self.x[j], self.z[j], self.yaw[j]);
            let braking = o.speed * o.speed / (2.0 * APPROACH_COMFORT_DECEL);
            let radius = len.hypot(wid) * 0.5 + ol.hypot(ow) * 0.5 + braking;
            if geom::hypot(ox - x0, oz - z0) > radius + total {
                return;
            }
            for s in 0..=count {
                let d = total * s as f64 / count as f64;
                let (x, z, yaw) = motion.at(d);
                if net.shares_surface(surface.along(lane, exit, plan, [x, z], d), self.surface(j))
                    && geom::hypot(x - ox, z - oz) <= radius
                    && cross_corridor_gap(ox, oz, oy, ol, ow, x, z, yaw, len, wid, braking).is_some()
                {
                    clear = false;
                    return;
                }
            }
        });
        clear
    }

    /// Limit an approach to the start of an adjacent vehicle's reserved sweep.
    /// As the holder advances, the space behind its tail becomes available again.
    fn reserved_turn_gap(&self, net: &Network, i: usize) -> f64 {
        let c = &self.cars[i];
        let lane = &net.lanes[c.lane as usize];
        let remaining = lane.length - c.arc;
        let look = cross_lookahead(c.speed).min(remaining);
        if remaining > lane.hold[c.kind as usize] + look {
            return f64::INFINITY;
        }
        let Some(s) = self.slot(lane.end_node) else { return f64::INFINITY };
        let slot = &self.claims[s];
        let node = &net.nodes[lane.end_node as usize];
        let mut gap = f64::INFINITY;
        for k in 0..slot.n as usize {
            let j = slot.car[k] as usize;
            let o = &self.cars[j];
            let (from, ix) = node.movements[slot.movement[k] as usize];
            let approach = &net.lanes[from as usize];
            if from == c.lane || lane.level != approach.level || lane.segment != approach.segment
                || approach.links[ix as usize].movement != Movement::Right
                || (!o.cornering() && o.lane != from)
            {
                continue;
            }
            let progress = if o.cornering() { o.corner_s } else { 0.0 };
            let plan = if o.cornering() { o.corner_plan(net) } else { approach.links[ix as usize].plan.as_ref() };
            let Some(plan) = plan else { continue };
            let (len, wid) = (o.len, o.wid);
            let exit = &net.lanes[approach.links[ix as usize].to as usize];
            let motion = MovementPose::new(approach, exit, plan, len);
            let tail = plan.arc_len - progress;
            let n = ((tail / JUNCTION_SWEEP_STEP_M).ceil() as usize).max(1);
            for j in 0..=n {
                let (x, z, yaw) = motion.at(progress + tail * j as f64 / n as f64);
                if let Some(hit) = cross_corridor_gap(
                    self.x[i], self.z[i], self.yaw[i], c.len, c.wid, x, z, yaw, len, wid, look,
                ) {
                    gap = gap.min(hit);
                }
            }
        }
        gap
    }

    /// Opposing traffic close enough to conflict gets the approach before a
    /// permissive left turn. Cars held at red and cars behind a leader wait.
    fn opposing_traffic(&self, ctx: &StepCtx, i: usize, movement: usize) -> bool {
        let net = ctx.net;
        let lane = &net.lanes[self.cars[i].lane as usize];
        let ends = |lane: &crate::network::Lane| {
            let [a, b] = [lane.pts[lane.pts.len() - 2], lane.pts[lane.pts.len() - 1]];
            [b[0] - a[0], b[1] - a[1]]
        };
        let [dx, dz] = ends(lane);
        net.nodes[lane.end_node as usize].incoming.iter().any(|&other| {
            if other == self.cars[i].lane {
                return false;
            }
            let approach = &net.lanes[other as usize];
            let [ox, oz] = ends(approach);
            if approach.level != lane.level || dx * ox + dz * oz >= -0.5 * geom::hypot(dx, dz) * geom::hypot(ox, oz) {
                return false;
            }
            if let ControlKind::Signal { junction, axis, .. } = approach.control {
                if net.junctions[junction as usize].aspect_at(axis as usize, ctx.time) != GREEN {
                    return false;
                }
            }
            let Some(j) = self.last_on_lane(other) else { return false };
            let c = &self.cars[j];
            if c.lane != other || c.cornering()
                || (matches!(approach.control, ControlKind::Stop { enforced: true, .. } | ControlKind::MappedStop { .. }) && !c.stop_released)
            {
                return false;
            }
            let distance = approach.length - c.arc;
            if distance > approach.hold[c.kind as usize] + TURN_APPROACH_M + c.speed * c.speed / (2.0 * APPROACH_COMFORT_DECEL) {
                return false;
            }
            approach.links.iter().enumerate().any(|(k, l)| {
                (c.next == NO_LINK || c.next as usize == k)
                    && l.legal && l.drivable && matches!(l.movement, Movement::Through | Movement::Right)
                    && net.movement_index(lane.end_node, other, k as u16)
                        .is_some_and(|m| net.movements_conflict(lane.end_node, movement, m))
            })
        })
    }

    /// `beginCorner`: commit to the movement. A corner reached past its
    /// memoised entry is re-planned from where the car stands
    /// (`cornerFilletAt`); one that cannot be re-planned is refused.
    fn begin_corner(&mut self, net: &Network, i: usize, to: LaneId, lead: f64) -> bool {
        let c = &self.cars[i];
        if !net.lanes[to as usize].fits_vehicle(c.len, c.wid) {
            return false;
        }
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
        c.corner_s = if arc_len == 0.0 { lead } else { lead.min(arc_len) };
        c.lane = to;
        c.arc = exit - (arc_len - c.corner_s);
        c.next = NO_LINK;
        c.max_speed = net.lanes[to as usize].max_speed * c.pace;
        c.stop_released = false;
        c.stop_hold = 0.0;
        c.stop_arrival = None;
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
        let c = &self.cars[i];
        let net = ctx.net;
        let freeway = geom::freeway(&net.lanes[c.corner_from as usize].highway)
            && geom::freeway(&net.lanes[c.lane as usize].highway);
        let passing = c.passing();
        let mut ceiling = if freeway || passing { c.max_speed } else { LOCAL_CORNER_SPEED_MPS };
        if let Some(gap) = net.following_time_gap_s.filter(|_| freeway) {
            let clearance = self.leader_gap[i] - (c.len + self.leader_len[i]) / 2.0;
            ceiling = ceiling.min((clearance - FOLLOW_SETTLE_GAP).max(0.0) / gap);
        }
        let (arc_len, mut cap, exit) = c.corner_plan(net).map_or((0.5, ceiling, 0.0), |p| {
            (p.arc_len, ceiling.min(p.speed_cap.max(1.2)), p.exit_arc)
        });
        if net.bend_braking {
            let exit_target = net.lanes[c.lane as usize].bend_speed_target(exit, c.len, cap);
            if exit_target < cap {
                let approach = (arc_len - c.corner_s).max(0.0);
                cap = cap.min((exit_target * exit_target + 2.0 * APPROACH_COMFORT_DECEL * approach).sqrt());
            }
        }
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
        let pedestrian_advance = self.pedestrian_advance.get(i).copied().unwrap_or(f64::INFINITY);
        if pedestrian_advance < speed * dt {
            speed = pedestrian_advance / dt;
        }
        let mut advance = (speed * dt).min(self.physical_advance.get(i).copied().unwrap_or(f64::INFINITY));
        if passing {
            advance = advance.min((lg - (c.len + self.leader_len[i]) / 2.0 - MIN_BUMPER_GAP).max(0.0));
        }
        if pg.is_finite() {
            let room = (pg - PERSON_EMERGENCY_M).max(0.0);
            advance = advance.min(room);
        }
        let c = &mut self.cars[i];
        c.braking = speed < 0.05 || speed < c.speed;
        c.speed = speed;
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
            if c.claim_node != net.lanes[c.lane as usize].end_node && c.arc - c.len / 2.0 > c.claim_exit + CLAIM_RELEASE_MARGIN_M {
                self.release_claim(i);
            }
        }
    }

    /// The walking step uses the positions after vehicles move or respawn.
    /// Reuse the conflict grid now that vehicle sensing is done for this step.
    pub(crate) fn index_walker_obstacles(&mut self) -> f64 {
        let grid = self.grid.get_or_insert_with(|| Grid::new(CROSS_CELL_M));
        grid.build(&self.x, &self.z, CROSS_CELL_M, |i| self.level[i] == 0);
        self.cars.iter().zip(&self.level).filter(|(_, level)| **level == 0)
            .map(|(car, _)| car.len.hypot(car.wid) / 2.0)
            .fold(0.0, f64::max)
    }

    pub(crate) fn walker_obstacle_grid(&self) -> Option<&Grid> {
        self.grid.as_ref()
    }

    /// The resident network changed (tiles streamed in or out): node movement
    /// tables and link indices were rebuilt, so every claim and every chosen
    /// exit is dropped and re-decided; a car mid-corner keeps its movement if
    /// the link still exists and is set down on its exit lane if not.
    pub fn network_changed(&mut self, net: &Network) {
        self.claims.clear();
        self.sweep_conflicts.clear();
        for i in 0..self.cars.len() {
            let c = &mut self.cars[i];
            c.claim_node = NONE;
            c.next = NO_LINK;
            if c.cornering() {
                let from = &net.lanes[c.corner_from as usize];
                if c.passing() && from.alive && net.lanes[c.lane as usize].alive {
                    self.write_pose(net, i);
                    continue;
                }
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

#[cfg(test)]
mod behavior_tests;

#[cfg(test)]
mod right_turn_tests {
    use super::*;

    #[test]
    fn adjacent_buses_clear_folsom_at_fourth() {
        let paths = eo_data::DataPaths::discover().unwrap();
        let source = crate::native::load(&paths).unwrap();
        let [x, z] = eo_data::groundscale::plan([1539.0, -366.0]);
        let tiles = source.resident_tiles(x, z, 600.0);
        let mut net = Network::new(source.timing.clone());
        net.add(source.build(&tiles));
        let through = net.lane_by_key("n419633751:0-forward-1").unwrap();
        let turning = net.lane_by_key("n419633751:0-forward-2").unwrap();
        let original = net.lane_by_key("n397145255:0-forward-4").unwrap();
        let curb = net.lane_by_key("n397145255:0-forward-1").unwrap();
        let straight = net.lane_by_key("n255168047:0-forward-1").unwrap();

        let mut fleet = Fleet::default();
        let mut rng = Mulberry32::new(78);
        let grid = Grid::new(PERSON_CELL_M);
        // Both buses start at their recorded positions, with the through bus
        // stopped and the turning bus moving toward the original tangent.
        for (i, (id, lane, exit, arc, speed)) in [
            (54, through, straight, 51.37119448527577, 0.0),
            (72, turning, original, 58.26730223179189, 4.098780306383839),
        ].into_iter().enumerate() {
            assert!(fleet.spawn(&net, &[lane], &mut rng, 1));
            let c = &mut fleet.cars[i];
            c.id = id;
            c.kind = 3;
            (c.len, c.wid) = presented(c.kind);
            c.pace = driver_pace(id, 1, DRIVER_PACE_MIN, DRIVER_PACE_SPAN);
            fleet.place(&net, i, lane, arc, speed);
            fleet.cars[i].next = net.lanes[lane as usize].links.iter().position(|link| link.to == exit).unwrap() as u16;
        }

        let mut completed = [false; 2];
        for step in 0..3600 {
            let ctx = StepCtx {
                net: &net,
                time: 80.2 + step as f64 * FIXED_STEP_S,
                seed_int: 1,
                homing_focus: None,
                visible: &|_, _| true,
                landing_clear: &|_| true,
            };
            fleet.sense(&net, &[], &[], &grid, &[]);
            fleet.act(&ctx, FIXED_STEP_S, &mut rng, step);
            fleet.settle_claims(&net);
            let pose = |i: usize| (fleet.x[i], fleet.z[i], fleet.yaw[i], fleet.cars[i].len, fleet.cars[i].wid);
            assert_eq!(geom::obb_penetration(pose(0), pose(1)), 0.0, "bus overlap at {:.2} s", ctx.time);
            for (i, exit) in [straight, curb].into_iter().enumerate() {
                completed[i] |= fleet.cars[i].lane == exit && !fleet.cars[i].cornering();
            }
            if completed == [true; 2] {
                break;
            }
        }
        assert_eq!(completed, [true; 2], "the through bus and right-turning bus must both clear the junction");
        assert_eq!(fleet.stats.red_entries, 0);
    }
}

#[cfg(test)]
mod freeway_tests {
    use super::*;
    use crate::geom::LaneRole;
    use crate::network::{LaneBuild, NetworkBuild};
    use crate::standards::{SignalTiming, Standards};

    fn speed_zone_network() -> Network {
        let standards = Standards::load(&eo_data::DataPaths::discover().unwrap()).unwrap();
        let mut net = Network::new(SignalTiming::from_standards(&standards).unwrap());
        let lane = |key: &str, highway: &str, start: f64, end: f64, posted: &str| LaneBuild {
            key: key.into(),
            segment: key.into(),
            street: key.into(),
            highway: highway.into(),
            role: LaneRole::Through,
            painted_through_mapped: false,
            width: 3.6,
            points: vec![[start, 0.0], [end, 0.0]],
            start_node: [start, 0.0],
            end_node: [end, 0.0],
            max_speed: geom::autonomous_speed(highway, posted, None, ""),
            control: ControlKind::None,
            outgoing: None,
            legal: None,
            tile: None,
            level: 0,
        };
        net.add(NetworkBuild {
            lanes: vec![
                lane("main", "motorway", -240.0, 0.0, "45 mph"),
                lane("exit", "motorway_link", 0.0, 240.0, "35 mph"),
            ],
            following_time_gap_s: Some(standards.metric("following-time-gap").unwrap()),
            ..Default::default()
        });
        net
    }

    #[test]
    fn car_brakes_before_a_bend_without_a_lane_transition() {
        let standards = Standards::load(&eo_data::DataPaths::discover().unwrap()).unwrap();
        let mut net = Network::new(SignalTiming::from_standards(&standards).unwrap());
        let cruise = geom::autonomous_speed("service", "15 mph", None, "");
        let bend = 40.0;
        net.add(NetworkBuild {
            lanes: vec![LaneBuild {
                key: "bent-service-road".into(), segment: "service-road".into(), street: "service-road".into(),
                highway: "service".into(), role: LaneRole::Through, painted_through_mapped: false,
                width: 3.8, points: vec![[0.0, 0.0], [0.0, bend], [100.0, bend]],
                start_node: [0.0, 0.0], end_node: [100.0, bend], max_speed: cruise,
                control: ControlKind::None, outgoing: None, legal: None, tile: None, level: 0,
            }],
            continuous_lane_heading: true,
            bend_braking: true,
            ..Default::default()
        });
        let lane = net.lane_by_key("bent-service-road").unwrap();
        let mut fleet = Fleet::default();
        let mut rng = Mulberry32::new(7);
        assert!(fleet.spawn(&net, &[lane], &mut rng, 7));
        fleet.place(&net, 0, lane, 25.0, cruise);
        fleet.cars[0].max_speed = cruise;
        let half = fleet.cars[0].len / 2.0;
        let grid = Grid::new(PERSON_CELL_M);
        let mut braked_before_body_reached_bend = false;
        let mut crossed_apex = false;
        for step in 0..480 {
            let ctx = StepCtx {
                net: &net, time: step as f64 * FIXED_STEP_S, seed_int: 7, homing_focus: None,
                visible: &|_, _| true, landing_clear: &|_| true,
            };
            fleet.sense(&net, &[], &[], &grid, &[]);
            let previous_speed = fleet.cars[0].speed;
            let previous_arc = fleet.cars[0].arc;
            fleet.act(&ctx, FIXED_STEP_S, &mut rng, step);
            let car = &fleet.cars[0];
            assert_eq!(car.lane, lane);
            assert!((previous_speed - car.speed) <= DECEL * FIXED_STEP_S + 1e-6);
            braked_before_body_reached_bend |= car.arc + half < bend && car.braking;
            if previous_arc < bend && car.arc >= bend {
                crossed_apex = true;
                // The step read the radius at its previous position, before
                // moving across the apex.
                let offset = previous_arc - bend;
                let radius = (half + offset * offset / half).max(TURN_RADIUS_MIN);
                let expected = (TURN_LATERAL_ACCEL * radius).sqrt();
                assert!((car.speed - expected).abs() < 1e-6, "{} m/s at the bend", car.speed);
            }
        }
        assert!(braked_before_body_reached_bend && crossed_apex);
        assert!(fleet.cars[0].arc > bend + half && (fleet.cars[0].speed - cruise).abs() < 1e-6);
    }

    #[test]
    fn freeway_transition_brakes_for_the_receiving_lane() {
        let net = speed_zone_network();
        let main = net.lane_by_key("main").unwrap();
        let exit = net.lane_by_key("exit").unwrap();
        assert_eq!(net.lanes[main as usize].max_speed, 45.0 * METRES_PER_SECOND_PER_MPH);
        assert_eq!(net.lanes[exit as usize].max_speed, 35.0 * METRES_PER_SECOND_PER_MPH);
        assert_eq!(geom::autonomous_speed("primary", "35 mph", None, ""), AUTONOMOUS_TRAFFIC_CEILING_MPS);
        assert_eq!(geom::autonomous_speed("motorway", "", None, ""), AUTONOMOUS_TRAFFIC_CEILING_MPS);

        let mut fleet = Fleet::default();
        let mut rng = Mulberry32::new(7);
        assert!(fleet.spawn(&net, &[main], &mut rng, 7));
        let main_speed = net.lanes[main as usize].max_speed * fleet.cars[0].pace;
        let exit_speed = net.lanes[exit as usize].max_speed * fleet.cars[0].pace;
        fleet.place(&net, 0, main, 130.0, main_speed);
        let grid = Grid::new(PERSON_CELL_M);
        let mut slowed_before_entry = false;
        for step in 0..480 {
            let ctx = StepCtx {
                net: &net,
                time: step as f64 * FIXED_STEP_S,
                seed_int: 7,
                homing_focus: None,
                visible: &|_, _| true,
                landing_clear: &|_| true,
            };
            fleet.sense(&net, &[], &[], &grid, &[]);
            let previous = fleet.cars[0].speed;
            fleet.act(&ctx, FIXED_STEP_S, &mut rng, step);
            let car = &fleet.cars[0];
            assert!(previous - car.speed <= DECEL * FIXED_STEP_S + 1e-6);
            slowed_before_entry |= car.lane == main && car.speed < main_speed - 0.5;
            assert!(car.lane != exit || (car.speed - exit_speed).abs() <= 0.1);
        }
        assert!(slowed_before_entry);
        assert_eq!(fleet.cars[0].lane, exit);
    }

    #[test]
    fn freeway_follower_brakes_for_the_sourced_time_gap() {
        let net = speed_zone_network();
        let lane = net.lane_by_key("main").unwrap();
        let gap = net.following_time_gap_s.unwrap();
        let mut fleet = Fleet::default();
        let mut rng = Mulberry32::new(7);
        assert!(fleet.spawn(&net, &[lane], &mut rng, 7));
        fleet.place(&net, 0, lane, 85.0, 14.0);
        fleet.cars[0].max_speed = 14.0;
        assert!(fleet.spawn(&net, &[lane], &mut rng, 7));
        fleet.place(&net, 1, lane, 55.0, 14.0);
        let grid = Grid::new(PERSON_CELL_M);
        for step in 0..240 {
            let ctx = StepCtx {
                net: &net,
                time: step as f64 * FIXED_STEP_S,
                seed_int: 7,
                homing_focus: None,
                visible: &|_, _| true,
                landing_clear: &|_| true,
            };
            fleet.sense(&net, &[], &[], &grid, &[]);
            fleet.act(&ctx, FIXED_STEP_S, &mut rng, step);
            assert!(fleet.cars.iter().all(|c| c.lane == lane && !c.cornering()));
        }
        let (leader, follower) = (&fleet.cars[0], &fleet.cars[1]);
        let clearance = leader.arc - follower.arc - (leader.len + follower.len) / 2.0;
        assert!(follower.speed < leader.speed);
        assert!(clearance >= follower.speed * gap, "{clearance:.1} m at {:.1} m/s", follower.speed);
    }
}

#[cfg(test)]
mod level_tests {
    //! Two lanes crossing at one point (x = 0, z = 0) with no node between
    //! them, as a deck crosses a street: on different structure levels the
    //! cars on them never block or see each other; on one level they do.

    use super::*;
    use crate::geom::{LaneRole, P2};
    use crate::network::{LaneBuild, NetworkBuild, TileKey};
    use crate::standards::{SignalTiming, Standards};

    fn crossing_at(upper: i8, heights: Option<[f64; 2]>) -> Network {
        let standards = Standards::load(&eo_data::DataPaths::discover().unwrap()).unwrap();
        let mut net = Network::new(SignalTiming::from_standards(&standards).unwrap());
        let lane = |key: &str, pts: Vec<P2>, level: i8| LaneBuild {
            key: key.into(),
            segment: key.into(),
            street: key.into(),
            highway: "primary".into(),
            role: LaneRole::Through,
            painted_through_mapped: false,
            width: 3.5,
            start_node: pts[0],
            end_node: *pts.last().unwrap(),
            points: pts,
            max_speed: 13.0,
            control: ControlKind::None,
            outgoing: None,
            legal: None,
            tile: Some(TileKey { x: 0, z: 0 }),
            level,
        };
        net.add(NetworkBuild {
            tiles: vec![TileKey { x: 0, z: 0 }],
            lanes: vec![lane("street", vec![[-200.0, 0.0], [200.0, 0.0]], 0), lane("deck", vec![[0.0, -200.0], [0.0, 200.0]], upper)],
            lane_surfaces: heights.map_or_else(Vec::new, |[low, high]| {
                vec![(0, vec![[0.0, low], [400.0, low]]), (1, vec![[0.0, high], [400.0, high]])]
            }),
            surface_clearance_m: Some(standards.metric("structure-vertical-clearance-local").unwrap()),
            ..Default::default()
        });
        net
    }

    fn crossing(upper: i8) -> Network {
        crossing_at(upper, None)
    }

    /// A car on the street 6 m short of the crossing at 10 m/s, one on the
    /// deck standing at it: each one's cross-corridor gap, after a sense.
    fn gaps(net: &Network) -> (Fleet, [f64; 2]) {
        let (street, deck) = (net.lane_by_key("street").unwrap(), net.lane_by_key("deck").unwrap());
        let mut fleet = Fleet::default();
        let mut rng = Mulberry32::new(7);
        assert!(fleet.spawn(net, &[street], &mut rng, 7) && fleet.spawn(net, &[deck], &mut rng, 7));
        fleet.place(net, 0, street, 194.0, 10.0);
        fleet.place(net, 1, deck, 200.0, 0.0);
        let grid = crate::grid::Grid::new(PERSON_CELL_M);
        fleet.sense(net, &[], &[], &grid, &[]);
        let g = [fleet.cross_gap[0], fleet.cross_gap[1]];
        (fleet, g)
    }

    #[test]
    fn crossing_surfaces_at_different_heights_can_share_a_level_label() {
        let net = crossing_at(0, Some([11.4, 19.0]));
        let (fleet, gap) = gaps(&net);
        assert_eq!(fleet.level, vec![0, 0]);
        assert_eq!(fleet.surface_y, vec![11.4, 19.0]);
        assert!(gap.iter().all(|g| g.is_infinite()), "{gap:?}");
        assert!(!fleet.insert_blocked(&net, 0, 200.0, 0));

        let net = crossing_at(1, Some([11.4, 11.45]));
        let (fleet, gap) = gaps(&net);
        assert!(gap.iter().any(|g| g.is_finite()), "{gap:?}");
        assert!(fleet.insert_blocked(&net, 0, 200.0, 0));
    }

    #[test]
    fn cars_on_two_levels_never_block_or_see_each_other() {
        // one level: the street car sees the deck car in its corridor, and
        // may not be set down where it stands.
        let flat = crossing(0);
        let (fleet, g) = gaps(&flat);
        assert_eq!(fleet.level, vec![0, 0]);
        assert!(g[0].is_finite() || g[1].is_finite(), "{g:?}");
        let street = flat.lane_by_key("street").unwrap();
        assert!(fleet.insert_blocked(&flat, street, 200.0, 0));
        // two levels: neither sees the other, and the street car may stand under the deck car.
        let two = crossing(1);
        let (fleet, g) = gaps(&two);
        assert_eq!(fleet.level, vec![0, 1]);
        assert!(g[0].is_infinite() && g[1].is_infinite(), "{g:?}");
        let street = two.lane_by_key("street").unwrap();
        assert!(!fleet.insert_blocked(&two, street, 200.0, 0));
        // the instance flags carry the level; nothing else changes with it.
        assert_eq!(crate::output::level_of_flags(crate::output::level_flags(fleet.level[1])), 1);
        // a deck car meets no walker on the street under it.
        let mut fleet = fleet;
        let mut grid = crate::grid::Grid::new(PERSON_CELL_M);
        let (wx, wz) = (vec![0.0, 6.0], vec![-1.0, 0.0]);
        grid.build(&wx, &wz, PERSON_CELL_M, |_| true);
        let deck = two.lane_by_key("deck").unwrap();
        fleet.place(&two, 1, deck, 194.0, 10.0);
        fleet.sense(&two, &wx, &wz, &grid, &[]);
        assert!(fleet.person_gap[1].is_infinite(), "deck car's person gap {}", fleet.person_gap[1]);
        assert!(fleet.person_gap[0].is_finite(), "street car's person gap {}", fleet.person_gap[0]);
    }

    #[test]
    fn traffic_stops_for_the_player_vehicle_on_its_own_level() {
        let net = crossing(1);
        let (mut fleet, _) = gaps(&net);
        fleet.take(1);
        let street = fleet.cars[0].lane;
        fleet.place(&net, 0, street, 170.0, 10.0);
        let mut rng = Mulberry32::new(7);
        let grid = Grid::new(PERSON_CELL_M);
        let ctx = StepCtx { net: &net, time: 0.0, seed_int: 7, homing_focus: None, visible: &|_, _| true, landing_clear: &|_| true };
        let mut obstacle = TrafficObstacle { pos: [0.0, 0.0], yaw: 0.0, length: 5.0, width: 2.0, level: 1 };
        fleet.sense_obstacle(&net, obstacle);
        assert!(fleet.cross_gap[0].is_infinite());
        obstacle.level = 0;
        for step in 0..300 {
            fleet.sense(&net, &[], &[], &grid, &[]);
            fleet.sense_obstacle(&net, obstacle);
            fleet.act(&ctx, FIXED_STEP_S, &mut rng, step);
        }
        assert!(fleet.x[0] > -15.0 && fleet.x[0] + fleet.cars[0].len / 2.0 < -obstacle.width / 2.0);
        assert!(fleet.spd[0] <= STALL_SPEED_MS);
        for step in 300..480 {
            fleet.sense(&net, &[], &[], &grid, &[]);
            fleet.act(&ctx, FIXED_STEP_S, &mut rng, step);
        }
        assert!(fleet.x[0] > 0.0, "car should proceed after the obstruction clears");
    }

    #[test]
    fn traffic_brakes_for_a_chassis_around_a_lane_bend() {
        let mut net = Network::new(crossing(0).timing);
        let mut points: Vec<_> = (0..=20).map(|i| {
            let angle = f64::from(i) / 20.0 * std::f64::consts::FRAC_PI_2;
            [7.5 * (1.0 - angle.cos()), 7.5 * angle.sin()]
        }).collect();
        points.push([30.0, 7.5]);
        let tile = TileKey { x: 0, z: 0 };
        net.add(NetworkBuild {
            tiles: vec![tile],
            continuous_lane_heading: true,
            lanes: vec![LaneBuild {
                key: "bend".into(),
                segment: "bend".into(),
                street: "bend".into(),
                highway: "residential".into(),
                role: LaneRole::Through,
                painted_through_mapped: false,
                width: 3.5,
                start_node: points[0],
                end_node: *points.last().unwrap(),
                points,
                max_speed: 8.0,
                control: ControlKind::None,
                outgoing: None,
                legal: None,
                tile: Some(tile),
                level: 0,
            }],
            ..Default::default()
        });
        let lane = net.lane_by_key("bend").unwrap();
        let van = TrafficObstacle {
            pos: [7.5, 7.5],
            yaw: std::f64::consts::FRAC_PI_2,
            length: 5.54,
            width: 2.55,
            level: 0,
        };
        let mut fleet = Fleet::default();
        let mut rng = Mulberry32::new(1);
        assert!(fleet.spawn(&net, &[lane], &mut rng, 7));
        fleet.place(&net, 0, lane, 0.5, 6.0);
        let car = &fleet.cars[0];
        assert!(cross_corridor_gap(
            fleet.x[0], fleet.z[0], fleet.yaw[0], car.len, car.wid,
            van.pos[0], van.pos[1], van.yaw, van.length, van.width, 20.0,
        ).is_none(), "the van starts outside the straight sensing corridor");
        let grid = Grid::new(PERSON_CELL_M);
        let ctx = StepCtx {
            net: &net,
            time: 0.0,
            seed_int: 7,
            homing_focus: None,
            visible: &|_, _| true,
            landing_clear: &|_| true,
        };
        let mut saw_around_bend = false;
        for step in 0..480 {
            fleet.sense(&net, &[], &[], &grid, &[]);
            if step < 300 {
                fleet.sense_obstacle(&net, van);
                let car = &fleet.cars[0];
                saw_around_bend |= fleet.cross_gap[0].is_finite() && cross_corridor_gap(
                    fleet.x[0], fleet.z[0], fleet.yaw[0], car.len, car.wid,
                    van.pos[0], van.pos[1], van.yaw, van.length, van.width, 20.0,
                ).is_none();
            }
            fleet.act(&ctx, FIXED_STEP_S, &mut rng, step);
            if step < 300 {
                let car = &fleet.cars[0];
                assert_eq!(geom::obb_penetration(
                    (fleet.x[0], fleet.z[0], fleet.yaw[0], car.len, car.wid),
                    (van.pos[0], van.pos[1], van.yaw, van.length, van.width),
                ), 0.0, "body overlap on step {step}");
            }
        }
        assert!(saw_around_bend, "sense the van before it enters the straight corridor");
        assert!(fleet.x[0] > van.pos[0], "car proceeds after the bend clears");
    }

    #[test]
    fn a_displaced_chassis_limits_its_route_progress_without_blocking_itself() {
        let net = crossing(1);
        let (mut fleet, _) = gaps(&net);
        fleet.take(1);
        let street = fleet.cars[0].lane;
        let start = 170.0;
        fleet.place(&net, 0, street, start, 10.0);
        let c = &fleet.cars[0];
        let mut observed = TrafficObservation {
            fleet_id: Some(c.id), surface_y: 0.0,
            obstacle: TrafficObstacle {
                pos: [fleet.x[0] - c.len, fleet.z[0]], yaw: fleet.yaw[0], length: c.len, width: c.wid, level: 0,
            },
        };
        let grid = Grid::new(PERSON_CELL_M);
        let mut rng = Mulberry32::new(7);
        let ctx = StepCtx { net: &net, time: 0.0, seed_int: 7, homing_focus: None, visible: &|_, _| true, landing_clear: &|_| true };
        fleet.sense(&net, &[], &[], &grid, &[]);
        fleet.sense_traffic_observation(&net, observed);
        fleet.act(&ctx, FIXED_STEP_S, &mut rng, 1);
        assert_eq!(fleet.cars[0].arc, start);
        assert!(fleet.spd[0] > 9.0, "the physical driver keeps a speed target to recover");

        observed.obstacle.pos = [fleet.x[0], fleet.z[0]];
        fleet.sense(&net, &[], &[], &grid, &[]);
        fleet.sense_traffic_observation(&net, observed);
        assert!(fleet.cross_gap[0].is_infinite());
        fleet.act(&ctx, FIXED_STEP_S, &mut rng, 2);
        assert!(fleet.cars[0].arc > start);
    }
}
