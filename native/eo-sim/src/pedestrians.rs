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
//! whatever order or thread steps it. In crowd mode, a walker following a
//! stranger on the same footway chooses its pace from their previous-step
//! positions. Activity and kerb timers keep running while it waits.
//!
//! A walker on a footway reads its polyline from the network; a corner or
//! crossing leg (two or three points) is held inline, so a walker owns no heap
//! memory.

use crate::consts::*;
use crate::geom::{self, P2, js_clamp, normalize_angle};
use crate::grid::Grid;
use crate::network::{FootwayId, JunctionId, NONE, Network};
use crate::rng::{walker_draw, walker_draw_salt};
use crate::vehicles::{Fleet, TrafficObservation, TrafficObstacle};
use std::collections::HashMap;

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
    /// Origin footway arc and retreat direction, so a walker deadlocked
    /// against a stopped vehicle mid-leg can step back onto its kerb cleanly.
    origin_arc: f64,
    origin_dir: i8,
    /// True once reversed mid-leg to step back onto the origin kerb (not counted as a completed crossing).
    retreating: bool,
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

/// (`--people-groups`) A member of a social group that keeps together: the
/// group's anchor is its leader's place on the walk (the leader keeps the
/// schedule; the others are stepped as copies of it), and each member stands
/// at its own place about that anchor.
#[derive(Clone, Copy, Debug)]
pub struct Party {
    /// The leader's key (this walker's own when it leads).
    pub leader: u32,
    /// Its place in the group (0 the leader) and the group's size.
    pub slot: u8,
    pub size: u8,
    /// Where it stands about the anchor now, in the anchor's frame (across
    /// to the right, along ahead), metres; NaN until first placed.
    pub at: [f64; 2],
    /// The heading the group's frame keeps (the leader's while walking,
    /// held while the group stands), and this member's own drawn heading.
    pub frame: f64,
    pub yaw: f64,
}

#[derive(Clone, Debug)]
pub struct Walker {
    /// Transient offset from the walking line while passing oncoming people.
    passing_offset: f64,
    /// Seconds spent blocked at zero speed by a vehicle while walking a `Leg`.
    leg_blocked: f64,
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
    /// (`--people-groups`; None as before) the social group it keeps with.
    pub party: Option<Party>,
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

/// The forward space and lateral passing target sensed before this step.
#[derive(Clone, Copy)]
pub struct WalkClearance {
    gap: f64,
    passing_offset: f64,
}

impl Default for WalkClearance {
    fn default() -> Self { Self { gap: f64::INFINITY, passing_offset: 0.0 } }
}

/// What a walker's step reads from the world.
pub struct WalkCtx<'a> {
    pub net: &'a Network,
    pub time: f64,
    pub hour: f64,
    pub fleet: &'a Fleet,
    pub(crate) obstacle_grid: Option<(&'a Grid, f64)>,
    pub vehicle: Option<TrafficObstacle>,
    pub traffic_observations: &'a [TrafficObservation],
    /// (`--people-groups`, `RegionConfig::groups`; false as before) social
    /// groups keep together, and window-shopping stops only at shop fronts.
    pub groups: bool,
    /// (`--people-windows`, `RegionConfig::windows`; false as before) a
    /// window-shopping stop only in front of a shop's window.
    pub windows: bool,
    /// (`--people-stagger`, `RegionConfig::stagger`; false as before) a
    /// walking group's line turned by its own keyed angle.
    pub stagger: bool,
    /// Clear distance to the pedestrian ahead at the start of the tick.
    /// Empty for the independent ambient pool.
    pub following_gaps: &'a [WalkClearance],
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
            passing_offset: 0.0,
            leg_blocked: 0.0,
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
            party: None,
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
    fn advance_activity(&mut self, net: &Network, hour: f64, dt: f64, groups: bool, windows: bool) -> bool {
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
        // (`--crowd`) at a nightlife front in its hours, a walk often ends at
        // the doors: a group stands talking, facing each other; one alone
        // stands or takes a call with their back to the front, looking out
        // at the street, as people outside a bar do (not at its door)
        // (zero without the frontage survey: no draw, the JS pause below).
        let night = net.footways[self.footway as usize].activity.night as f64 * night_footfall(hour);
        if night > 0.0 && walker_draw(self.key, "door-", leg) < night * DOOR_GATHER_SHARE {
            self.activity = if self.group {
                Activity::Talk
            } else if walker_draw(self.key, "door-call-", leg) < 0.5 {
                Activity::Phone
            } else {
                Activity::Stand
            };
            self.facing = if self.group { Facing::Partner } else { Facing::Across };
            let [lo, hi] = DOOR_GATHER_SECONDS;
            self.activity_timer = lo + walker_draw(self.key, "door-dur-", leg) * (hi - lo);
            return false;
        }
        let total = self.total(net);
        let near_corner = self.s.min(total - self.s) <= 6.0;
        // (`--people-groups`) a window is looked into where the walk has
        // windows: the browsing stop's weight is taken by the share of the
        // walk's frontage that is walk-in shops (none beside a blank wall, a
        // car park or houses; with the frontage survey, `--crowd`).
        let shops = net.footways[self.footway as usize].activity.shops as f64;
        // (`--people-windows`) and only in front of a shop's window: the front
        // at the walker's own stretch of the walk, not the walk's share (a
        // walk half shops, half houses had people looking into the houses'
        // fronts and blank walls, standing with their faces to them).
        let window = if windows { net.footways[self.footway as usize].activity.shop_front_at(self.s, total) as u8 as f64 } else { 1.0 };
        let weight = |e: &(Activity, f64, f64, f64, bool)| {
            if e.0 != Activity::Browse {
                e.1
            } else if windows {
                e.1 * window
            } else if groups {
                e.1 * shops.clamp(0.0, 1.0)
            } else {
                e.1
            }
        };
        let mut weight_total = 0.0;
        for e in PAUSE_TABLE.iter() {
            if !e.4 || near_corner {
                weight_total += weight(e);
            }
        }
        let mut roll = walker_draw(self.key, "act-", leg) * weight_total;
        let mut chosen = PAUSE_TABLE[PAUSE_TABLE.len() - 1];
        for e in PAUSE_TABLE.iter() {
            if e.4 && !near_corner {
                continue;
            }
            roll -= weight(e);
            if roll <= 0.0 && weight(e) > 0.0 {
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
    /// Sense neighbours before any walker moves. Use actual positions so
    /// strangers walking abreast and members of the same party can keep pace.
    pub(crate) fn following_gap(&self, i: usize, net: &Network, grid: &crate::grid::Grid, player: Option<crate::vehicles::TrafficObstacle>) -> WalkClearance {
        let w = &self.walkers[i];
        if w.leg.is_some() || !net.footways[w.footway as usize].alive {
            return WalkClearance::default();
        }
        let (_, _, tx, tz) = w.at(net, w.s);
        let (x, z) = (self.x[i], self.z[i]);
        let reach = WALKER_LEN_M + WALKER_SLOW_GAP_M + WALKER_WIDTH_M;
        let mut clearance = WalkClearance::default();
        let mut closest = (f64::INFINITY, u32::MAX);
        grid.query(x - reach, z - reach, x + reach, z + reach, |j| {
            let other = &self.walkers[j as usize];
            if other.key == w.key || other.footway != w.footway || other.leg.is_some()
                || w.party.zip(other.party).is_some_and(|(a, b)| a.leader == b.leader) {
                return;
            }
            let (dx, dz) = (self.x[j as usize] - x, self.z[j as usize] - z);
            let ahead = (dx * tx + dz * tz) * f64::from(w.dir);
            let abreast = (dx * tz - dz * tx).abs();
            if w.dir != other.dir {
                if ahead < -WALKER_LEN_M || ahead > reach { return; }
                // Keep this target through the encounter, even once the two
                // actual bodies have already moved sideways far enough to pass.
                let separation = WALKER_WIDTH_M + 0.10;
                if w.keeps_walking() && (w.lateral - other.lateral).abs() < separation
                    && (ahead.abs(), other.key) < closest {
                    closest = (ahead.abs(), other.key);
                    let side = f64::from(w.dir);
                    let target = if other.keeps_walking() {
                        let half = separation * 0.55;
                        let mid = (0.5 * (w.lateral + other.lateral))
                            .clamp(-WALKER_LATERAL_LIMIT_M + half, WALKER_LATERAL_LIMIT_M - half);
                        mid + side * half
                    } else {
                        let preferred = other.lateral + side * separation;
                        if preferred.abs() <= WALKER_LATERAL_LIMIT_M { preferred } else { other.lateral - side * separation }
                    };
                    clearance.passing_offset = target - w.lateral;
                }
                if ahead >= 0.0 && abreast < WALKER_WIDTH_M {
                    clearance.gap = clearance.gap.min(0.5 * (ahead - WALKER_LEN_M).max(0.0));
                }
            } else if (ahead > 0.0 || (ahead == 0.0 && other.key < w.key)) && abreast < WALKER_WIDTH_M {
                clearance.gap = clearance.gap.min((ahead - WALKER_LEN_M).max(0.0));
            }
        });
        if let Some(player) = player.filter(|p| p.level == 0) {
            let (dx, dz) = (player.pos[0] - x, player.pos[1] - z);
            let ahead = (dx*tx + dz*tz) * f64::from(w.dir);
            let lateral = dz*tx - dx*tz;
            let radius = 0.5 * (WALKER_WIDTH_M + player.width);
            if ahead >= -WALKER_LEN_M && ahead < reach && lateral.abs() < radius + 0.3 {
                let other_lane = w.lateral + w.passing_offset + lateral;
                let side = if lateral.abs() > 0.05 { -lateral.signum() } else { f64::from(w.dir) };
                let mut target = other_lane + side * (radius + 0.12);
                if target.abs() > WALKER_LATERAL_LIMIT_M { target = other_lane - side * (radius + 0.12); }
                clearance.passing_offset = target.clamp(-WALKER_LATERAL_LIMIT_M, WALKER_LATERAL_LIMIT_M) - w.lateral;
                if ahead >= 0.0 && lateral.abs() < radius {
                    clearance.gap = clearance.gap.min((ahead - (WALKER_LEN_M + player.length)*0.5).max(0.0));
                }
            }
        }
        clearance
    }

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

    /// A new vehicle must leave room for any pedestrian already at its landing pose.
    pub(crate) fn traffic_landing_clear(&self, body: TrafficObstacle, player: Option<TrafficObstacle>) -> bool {
        let [x, z] = body.pos;
        let chassis = (x, z, body.yaw, body.length, body.width);
        let reach = body.length.hypot(body.width) / 2.0 + PERSON_CAPSULE_R_M;
        let overlaps = |px: f64, pz: f64| {
            (px - x).abs() <= reach && (pz - z).abs() <= reach
                && geom::obb_clearance(px, pz, chassis) < PERSON_CAPSULE_R_M
        };
        if player.is_some_and(|p| p.level == body.level && overlaps(p.pos[0], p.pos[1])) {
            return false;
        }
        // Ambient walkers use the surface footway network.
        body.level != 0 || !self.x.iter().zip(&self.z).any(|(&px, &pz)| overlaps(px, pz))
    }

    /// One fixed step of every walker (`updatePedestrians`).
    pub fn step(&mut self, ctx: &WalkCtx, dt: f64, parity: u64) {
        let mut stats = std::mem::take(&mut self.stats);
        // (`--people-groups`) each group's leader by its key: a member whose
        // leader is here is not stepped on its own but as a copy of it, after it.
        let leaders = if ctx.groups { self.leader_index() } else { HashMap::new() };
        for i in 0..self.walkers.len() {
            if self.walkers[i].party.is_some_and(|p| p.slot > 0 && leaders.contains_key(&p.leader)) {
                continue;
            }
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
            let clearance = ctx.following_gaps.get(i).copied().unwrap_or_default();
            let sidestep = GROUP_STEP_ASIDE_MPS * step_dt;
            let offset = w.passing_offset;
            let shift = (clearance.passing_offset - offset).clamp(-sidestep, sidestep);
            if shift != 0.0 {
                let start = position_at(ctx.net, w, w.s);
                w.passing_offset += shift;
                let end = position_at(ctx.net, w, w.s);
                let clear = vehicle_hit_fraction(ctx, start, end, PERSON_CAPSULE_R_M, true);
                w.passing_offset = offset + shift * clear.clamp(0.0, 1.0);
            }
            step_walker(ctx, w, step_dt, clearance.gap, &mut stats);
            let (x, z) = position(ctx.net, w);
            self.x[i] = x;
            self.z[i] = z;
        }
        self.stats = stats;
        if ctx.groups {
            self.keep_parties(ctx.net, dt, &leaders, ctx.stagger);
        }
    }

    /// (`--people-groups`) Each group's leader (a walker leading a party of
    /// two or more) by its key.
    fn leader_index(&self) -> HashMap<u32, usize> {
        let mut m = HashMap::new();
        for (i, w) in self.walkers.iter().enumerate() {
            if let Some(p) = w.party {
                if p.slot == 0 && p.size > 1 {
                    m.insert(w.key, i);
                }
            }
        }
        m
    }

    /// (`--people-groups`) Keep each social group together after a step: a
    /// member takes its leader's state (where it is on its walk or leg, its
    /// activity and its schedule: they walk, wait, cross and stop together),
    /// keeping its own key, gait and place; a member whose leader is gone
    /// walks on alone from there. Then every member, the leader too, is drawn
    /// at its place about the anchor (the leader's point on the walk): abreast
    /// (three in a V) while the group walks, side by side along the kerb or
    /// the window while it waits or looks in, on a circle facing its middle
    /// while it stands talking; each place reached at a side step's pace and
    /// each heading at a walker's turning rate.
    fn keep_parties(&mut self, net: &Network, dt: f64, leaders: &HashMap<u32, usize>, stagger: bool) {
        let n = self.walkers.len();
        // The leaders' frames: their heading while they walk, held while they stand.
        for &li in leaders.values() {
            let w = &mut self.walkers[li];
            let moving = w.ground_speed > 0.0 || w.leg.is_some();
            if let Some(p) = w.party.as_mut() {
                if moving || p.frame.is_nan() {
                    p.frame = w.yaw;
                }
            }
        }
        // The members take their leader's state.
        for i in 0..n {
            let Some(p) = self.walkers[i].party else { continue };
            if p.slot == 0 {
                continue;
            }
            let Some(&li) = leaders.get(&p.leader) else {
                self.walkers[i].party = None;
                continue;
            };
            let lead = self.walkers[li].clone();
            let w = &mut self.walkers[i];
            let (key, gait, gait_freq, yaw) = (w.key, w.gait, w.gait_freq, w.yaw);
            *w = lead;
            w.key = key;
            w.party = Some(p);
            w.yaw = yaw;
            w.gait_freq = gait_freq;
            w.gait = gait + if w.ground_speed > 0.0 { gait_freq * dt * (w.ground_speed / w.speed) } else { 0.0 };
            w.lod_debt = 0.0;
        }
        // Every member at its place about the anchor.
        let turn = PEDESTRIAN_TURN_RATE * dt.max(0.0);
        for i in 0..n {
            let Some(mut p) = self.walkers[i].party else { continue };
            if p.size < 2 {
                continue;
            }
            let li = if p.slot == 0 { i } else { leaders[&p.leader] };
            let lead = &self.walkers[li];
            let frame = lead.party.map(|q| q.frame).filter(|f| f.is_finite()).unwrap_or(lead.yaw);
            let (ax, az) = position(net, lead);
            let moving = lead.ground_speed > 0.0 || lead.leg.is_some();
            let line = !moving && (lead.kerb.is_some() || matches!(lead.facing, Facing::Across | Facing::Inward));
            let (k, size) = (p.slot as f64, p.size as f64);
            let mid = k - (size - 1.0) / 2.0;
            let target = if moving {
                let back = if p.size == 3 && p.slot == 1 { -GROUP_V_BACK_M } else { 0.0 };
                if stagger {
                    // (`--people-stagger`) the abreast line turned by the group's
                    // own angle (keyed by its leader), the V's middle still back.
                    let turn = (2.0 * walker_draw_salt(p.leader, "stagger") - 1.0) * GROUP_STAGGER_RAD;
                    let (ts, tc) = turn.sin_cos();
                    [mid * GROUP_ABREAST_M * tc, mid * GROUP_ABREAST_M * ts + back]
                } else {
                    [mid * GROUP_ABREAST_M, back]
                }
            } else if line && lead.kerb.is_some() {
                // at the kerb the leader stands at the crossing's foot; the
                // others along the kerb behind it, none past it into the junction.
                [0.0, -k * GROUP_ABREAST_M]
            } else if line {
                [0.0, mid * GROUP_ABREAST_M]
            } else {
                let r = GROUP_TALK_GAP_M / (2.0 * (std::f64::consts::PI / size).sin());
                let a = std::f64::consts::PI + std::f64::consts::TAU * k / size;
                [r * a.cos(), r * a.sin()]
            };
            if p.at[0].is_nan() || p.at[1].is_nan() {
                p.at = target;
            } else {
                let (dx, dz) = (target[0] - p.at[0], target[1] - p.at[1]);
                let d = dx.hypot(dz);
                let reach = GROUP_STEP_ASIDE_MPS * dt.max(0.0);
                if d <= reach {
                    p.at = target;
                } else {
                    p.at = [p.at[0] + dx * reach / d, p.at[1] + dz * reach / d];
                }
            }
            // forward (sin, cos) of the frame's heading, across (cos, -sin).
            let (fs, fc) = frame.sin_cos();
            let (x, z) = (ax + fc * p.at[0] + fs * p.at[1], az - fs * p.at[0] + fc * p.at[1]);
            let want = if moving || line { lead.yaw } else { (ax - x).atan2(az - z) };
            let from = if p.yaw.is_nan() { lead.yaw } else { p.yaw };
            p.yaw = normalize_angle(from + js_clamp(normalize_angle(want - from), -turn, turn));
            let w = &mut self.walkers[i];
            w.yaw = p.yaw;
            w.party = Some(Party { frame, ..p });
            self.x[i] = x;
            self.z[i] = z;
        }
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
pub(crate) fn position_at(net: &Network, w: &Walker, arc: f64) -> P2 {
    let (mut x, mut z, dx, dz) = w.at(net, arc);
    let keep_side = w.leg.filter(|l| l.kind == LegKind::Cross)
        .map_or(0.0, |l| crossing_offset(arc, l.total) * f64::from(w.dir));
    if w.lateral != 0.0 || keep_side != 0.0 || w.passing_offset != 0.0 {
        let lat = (w.lateral + keep_side + w.passing_offset)
            .clamp(-WALKER_LATERAL_LIMIT_M, WALKER_LATERAL_LIMIT_M);
        // `at` returns the unit direction; the JS offsets along (-dz, dx) of the segment.
        x += -dz * lat;
        z += dx * lat;
    }
    [x, z]
}

fn crossing_offset(arc: f64, total: f64) -> f64 {
    if total <= 2.0 * CROSS_KEEP_RAMP_M { return 0.0; }
    let ramp = (arc / CROSS_KEEP_RAMP_M).min((total - arc) / CROSS_KEEP_RAMP_M).clamp(0.0, 1.0);
    CROSS_KEEP_SIDE_M * ramp
}

fn position(net: &Network, w: &Walker) -> (f64, f64) {
    let [x, z] = position_at(net, w, w.s);
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
    for (k, &speed) in ctx.fleet.spd.iter().enumerate() {
        if speed <= 1.0 || ctx.fleet.level[k] != 0 {
            continue;
        }
        let dx = mx - ctx.fleet.x[k];
        let dz = mz - ctx.fleet.z[k];
        let range = geom::hypot(dx, dz);
        if range <= MIN_BUMPER_GAP * 2.0 {
            return false;
        }
        let (fx, fz) = ctx.fleet.yaw[k].sin_cos();
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

fn vehicle_hit_fraction(ctx: &WalkCtx, start: P2, end: P2, radius: f64, allow_escape: bool) -> f64 {
    let [dx, dz] = [end[0] - start[0], end[1] - start[1]];
    let span = dx.hypot(dz);
    if span == 0.0 { return f64::INFINITY; }
    let centre = [start[0] + dx / 2.0, start[1] + dz / 2.0];
    let mut hit = f64::INFINITY;
    let mut check = |c: TrafficObstacle| {
        let reach = span / 2.0 + radius + c.length.hypot(c.width) / 2.0;
        if c.level == 0 && (c.pos[0] - centre[0]).powi(2) + (c.pos[1] - centre[1]).powi(2) <= reach * reach {
            let body = (c.pos[0], c.pos[1], c.yaw, c.length, c.width);
            if let Some(fraction) = geom::disc_sweep_hit(start, end, radius, body, allow_escape) {
                hit = hit.min(fraction);
            }
        }
    };
    let mut fleet_car = |i: usize| {
        let car = &ctx.fleet.cars[i];
        if ctx.fleet.level[i] == 0 && !ctx.traffic_observations.iter().any(|o| o.fleet_id == Some(car.id)) {
            check(TrafficObstacle {
                pos: [ctx.fleet.x[i], ctx.fleet.z[i]], yaw: ctx.fleet.yaw[i],
                length: car.len, width: car.wid, level: 0,
            });
        }
    };
    if let Some((grid, reach)) = ctx.obstacle_grid {
        let r = reach + radius;
        grid.query(start[0].min(end[0]) - r, start[1].min(end[1]) - r,
            start[0].max(end[0]) + r, start[1].max(end[1]) + r, |i| fleet_car(i as usize));
    } else {
        (0..ctx.fleet.cars.len()).for_each(fleet_car);
    }
    ctx.vehicle.into_iter().chain(ctx.traffic_observations.iter().map(|o| o.obstacle)).for_each(check);
    hit
}

fn crossing_route_clear(ctx: &WalkCtx, wait: &KerbWait) -> bool {
    let route = wait.walk.as_ref().map_or(wait.link.as_slice(), |points| points.as_slice());
    let length = |a: P2, b: P2| (b[0] - a[0]).hypot(b[1] - a[1]);
    let total = wait.length + wait.walk.map_or(0.0, |p| length(p[0], p[1]));
    let mut start_arc = 0.0;
    let mut previous = route[0];
    for edge in route.windows(2) {
        let [a, b] = [edge[0], edge[1]];
        let span = length(a, b);
        if span == 0.0 { continue; }
        let end_arc = start_arc + span;
        let (dx, dz) = ((b[0] - a[0]) / span, (b[1] - a[1]) / span);
        let at = |arc: f64| {
            let keep = crossing_offset(arc, total);
            let along = arc - start_arc;
            [a[0] + dx * along - dz * keep, a[1] + dz * along + dx * keep]
        };
        // Include the direction change at the kerb and the two points at
        // which the keep-side offset starts or stops changing.
        let bends = [CROSS_KEEP_RAMP_M, total - CROSS_KEEP_RAMP_M];
        for arc in std::iter::once(start_arc)
            .chain(bends.into_iter().filter(|&s| total > 2.0 * CROSS_KEEP_RAMP_M && start_arc < s && s < end_arc))
            .chain(std::iter::once(end_arc))
        {
            let next = at(arc);
            if vehicle_hit_fraction(ctx, previous, next, PERSON_CAPSULE_R_M, false).is_finite() {
                return false;
            }
            previous = next;
        }
        start_arc = end_arc;
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
    let origin_arc = w.s;
    let origin_dir = if w.dir == 0 { 1 } else { -w.dir };
    w.leg = Some(Leg { pts: leg_pts, n: n as u8, cum, total, kind, next, next_end, arc, dir, origin_arc, origin_dir, retreating: false });
    w.s = 0.01;
    w.dir = 1;
    w.leg_blocked = 0.0;
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
    w.leg_blocked = 0.0;
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
    let far = f.cross[end];
    if !f.routable || far == NONE {
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
    // (`--crowd`) no crosswalk mapped at this band: crossed only on a signal;
    // (`--people-crosswalks`) no zebra drawn: not crossed, signal or not.
    if f.ends[end].no_crossing && (signal == NONE || f.ends[end].zebra_only) {
        return;
    }
    w.cross_leg += 1;
    let share = if signal != NONE { CROSSING_SHARE } else { UNSIGNALLED_CROSSING_SHARE };
    if walker_draw(w.key, "cross-", w.cross_leg) >= share {
        return;
    }
    let other = &net.footways[far as usize];
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
    let mut station = if off > CROSSING_BAND_SNAP_M { js_clamp(band_s, 0.0, f.length) } else { w.s };
    let offset = f.original_arc_start.unwrap_or(0.0) - other.original_arc_start.unwrap_or(0.0);
    if f.original_arc_start.is_some() || other.original_arc_start.is_some() {
        let far_walk = other.ends[0].kerb_trim..=other.length - other.ends[1].kerb_trim;
        if !far_walk.contains(&(station + offset)) {
            let near_walk = f.ends[0].kerb_trim..=f.length - f.ends[1].kerb_trim;
            if !near_walk.contains(&band_s) || !far_walk.contains(&(band_s + offset)) {
                return;
            }
            station = band_s;
        }
    }
    let far_station = station + offset;
    let (hx, hz, _, _, _) = geom::at_arc(&f.pts, &f.cum, station);
    let (tx, tz, _, _, _) = geom::at_arc(&other.pts, &other.cum, js_clamp(far_station, 0.0, other.length));
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
        next: far,
        next_end: end as u8,
        arc: far_station,
        dir: w.dir,
        lateral: far_lateral,
        length,
        elapsed: 0.0,
    });
    stats.kerb_waits += 1;
}

/// Where a footway's walking line meets its end's kerb (its kerb trim in from the end).
fn kerb_point(net: &Network, fw: FootwayId, end: u8) -> P2 {
    let f = &net.footways[fw as usize];
    let arc = if end == 1 { f.length - f.ends[1].kerb_trim } else { f.ends[0].kerb_trim };
    let (x, z, _, _, _) = geom::at_arc(&f.pts, &f.cum, arc.clamp(0.0, f.length));
    [x, z]
}

/// Whether footway end `b` is on the same corner of
/// foot node `node` as end `a`: no street leg of the node (a footway's own
/// direction out of it) points between their kerb points, as seen from the
/// node. The JS joins every footway end at a junction's node, so a corner
/// leg could take a walker off one corner and across the road to another,
/// with no crosswalk or signal.
fn same_corner(net: &Network, node: &crate::network::FootNode, a: (FootwayId, u8), b: (FootwayId, u8)) -> bool {
    let c = node.pos;
    let angle = |p: P2| (p[1] - c[1]).atan2(p[0] - c[0]);
    let ta = angle(kerb_point(net, a.0, a.1));
    let arc = normalize_angle(angle(kerb_point(net, b.0, b.1)) - ta);
    for &(fw, e) in &node.ends {
        let f = &net.footways[fw as usize];
        if !f.alive || f.pts.len() < 2 {
            continue;
        }
        let n = f.pts.len();
        let (p, q) = if e == 0 { (f.pts[0], f.pts[1]) } else { (f.pts[n - 1], f.pts[n - 2]) };
        let x = normalize_angle((q[1] - p[1]).atan2(q[0] - p[0]) - ta);
        let inside = if arc >= 0.0 { x > 1e-3 && x < arc - 1e-3 } else { x < -1e-3 && x > arc + 1e-3 };
        if inside {
            return false;
        }
    }
    true
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PathEnd { Continued, Wait, Reverse }

/// `continueAlongFootway(pedestrian, node, fromIndex, fromEnd)`.
fn continue_along(ctx: &WalkCtx, w: &mut Walker, node_id: u32, from_end: usize, stats: &mut CrowdStats) -> PathEnd {
    let net = ctx.net;
    let node = &net.foot_nodes[node_id as usize];
    let from = w.footway;
    let from_way = &net.footways[from as usize];
    let from_segment = from_way.segment;
    let corner_only = net.contain_walk_corners || from_way.ends[from_end].zebra_only;
    let admitted = |fw: FootwayId, e: u8| {
        !corner_only || (fw != from_way.cross[from_end] && same_corner(net, node, (from, from_end as u8), (fw, e)))
    };
    let mut total_weight = 0.0;
    let mut count = 0;
    for &(fw, e) in &node.ends {
        if fw == from || !net.footways[fw as usize].alive || !admitted(fw, e) {
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
        return PathEnd::Reverse;
    }
    let next_leg = w.route_leg + 1;
    let mut roll = walker_draw(w.key, "route-", next_leg) * total_weight;
    let mut chosen = None;
    let mut last = None;
    for &(fw, e) in &node.ends {
        if fw == from || !net.footways[fw as usize].alive || !admitted(fw, e) {
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
    if vehicle_hit_fraction(ctx, pts[0], pts[1], PERSON_CAPSULE_R_M, false).is_finite() {
        return PathEnd::Wait;
    }
    w.route_leg = next_leg;
    w.lateral_hold = Some(lateral_to);
    w.lateral = 0.0;
    w.cross_node = NONE;
    stats.continuations += 1;
    begin_leg(net, w, &pts, to_fw, to_end, LegKind::Corner, f64::NAN, 0);
    PathEnd::Continued
}

/// `routeAtPathEnd(pedestrian, end)`.
fn route_at_end(ctx: &WalkCtx, w: &mut Walker, end: usize, stats: &mut CrowdStats) -> PathEnd {
    let net = ctx.net;
    if w.kerb.is_some() { return PathEnd::Wait; }
    if let Some(leg) = w.leg {
        if leg.kind == LegKind::Cross && !leg.retreating {
            stats.crossings_completed += 1;
        }
        return if adopt(net, w, leg.next, leg.next_end, leg.arc, leg.dir) { PathEnd::Continued } else { PathEnd::Reverse };
    }
    let f = &net.footways[w.footway as usize];
    if !f.routable {
        return PathEnd::Reverse;
    }
    let node = f.ends[end].node;
    if node == NONE {
        return PathEnd::Reverse;
    }
    let result = continue_along(ctx, w, node, end, stats);
    if result == PathEnd::Reverse {
        stats.reversals += 1;
    }
    result
}

#[cfg(test)]
mod corner_tests {
    use super::*;
    use crate::network::{FootEndBuild, FootNodeBuild, FootwayBuild, NetworkBuild};
    use crate::standards::{SignalTiming, Standards};

    fn walk_context<'a>(net: &'a Network, fleet: &'a Fleet, vehicle: Option<TrafficObstacle>) -> WalkCtx<'a> {
        WalkCtx {
            net, fleet, obstacle_grid: None, vehicle, time: 0.0, hour: 12.0,
            traffic_observations: &[], groups: false, windows: false, stagger: false, following_gaps: &[],
        }
    }

    #[test]
    fn a_corner_leg_stays_on_the_sidewalk_in_the_paved_preview() {
        let paths = eo_data::DataPaths::discover().unwrap();
        let timing = SignalTiming::from_standards(&Standards::load(&paths).unwrap()).unwrap();
        let make_way = |x, a, b, side, cross_index, keys: [&str; 2]| {
            let end = FootEndBuild {
                kerb_trim: 0.0,
                crossing_arc: 2.0,
                no_crossing: false,
                zebra_only: false,
            };
            FootwayBuild {
                points: vec![[x, a], [x, b]],
                highway: "residential".into(),
                segment: if a == 0.0 { "north" } else { "south" }.into(),
                road_side: side,
                node_keys: Some(keys.map(str::to_owned)),
                normal: [1.0, 0.0],
                cross_index: [cross_index; 2],
                original_arc_start: None,
                ends: [end.clone(), end],
                tile: None,
            }
        };
        for (strict, continuation, expected) in [
            (false, false, Some(1)),
            (true, false, None),
            (true, true, Some(2)),
        ] {
            let mut net = Network::new(timing.clone());
            let mut footways = vec![
                make_way(-4.0, -20.0, 0.0, -1, Some(1), ["south", "junction"]),
                make_way(4.0, -20.0, 0.0, 1, Some(0), ["south", "junction"]),
            ];
            if continuation {
                footways.push(make_way(-5.0, 0.0, 20.0, -1, None, ["junction", "north"]));
            }
            net.add(NetworkBuild {
                footways,
                foot_nodes: vec![FootNodeBuild { key: "junction".into(), pos: [0.0, 0.0], signal: None }],
                contain_walk_corners: strict,
                ..Default::default()
            });
            let mut walker = Walker::new(1, 0, 20.0, 1, 1.0, 0.0, -1, false, 0.0);
            let fleet = Fleet::default();
            let ctx = walk_context(&net, &fleet, None);
            if continuation {
                let car = TrafficObstacle { pos: [-4.5, 0.0], yaw: 0.0, length: 4.0, width: 2.0, level: 0 };
                let occupied = walk_context(&net, &fleet, Some(car));
                assert_eq!(route_at_end(&occupied, &mut walker, 1, &mut CrowdStats::default()), PathEnd::Wait);
                assert!(walker.leg.is_none());
                assert_eq!(walker.route_leg, 0);
            }
            assert_eq!(route_at_end(&ctx, &mut walker, 1, &mut CrowdStats::default()) == PathEnd::Continued, expected.is_some());
            assert_eq!(walker.leg.map(|leg| (leg.kind, leg.next)), expected.map(|path| (LegKind::Corner, path)));
        }
    }

    #[test]
    fn a_cut_sidewalk_can_use_a_crossing_band_that_both_kerbs_still_reach() {
        let paths = eo_data::DataPaths::discover().unwrap();
        let timing = SignalTiming::from_standards(&Standards::load(&paths).unwrap()).unwrap();
        let mut net = Network::new(timing);
        let walk = |x, from, trim, cross, side| FootwayBuild {
            points: vec![[x, from], [x, 20.0]], highway: "residential".into(), segment: "street".into(),
            road_side: side, normal: [1.0, 0.0], node_keys: Some(["node".into(), format!("end-{x}")]),
            cross_index: [cross, None], original_arc_start: Some(from), tile: None,
            ends: [
                FootEndBuild { kerb_trim: trim, crossing_arc: 5.0 - from, no_crossing: false, zebra_only: false },
                FootEndBuild { kerb_trim: 0.0, crossing_arc: 0.0, no_crossing: true, zebra_only: false },
            ],
        };
        net.add(NetworkBuild {
            footways: vec![walk(0.0, 0.0, 3.0, Some(1), 1), walk(8.0, 4.8, 0.0, Some(0), -1), walk(16.0, 0.0, 0.0, None, 1)],
            foot_nodes: vec![FootNodeBuild { key: "node".into(), pos: [4.0, 0.0], signal: None }],
            ..Default::default()
        });
        let key = (0..64).find(|&key| walker_draw(key, "cross-", 1) < UNSIGNALLED_CROSSING_SHARE).unwrap();
        let walker_at_kerb = || Walker::new(key, 0, 4.4, -1, 1.2, 0.0, 1, false, 0.0);
        let mut walker = walker_at_kerb();
        consider_crossing(&net, &mut walker, 0, &mut CrowdStats::default());
        let wait = walker.kerb.expect("the two sidewalks meet the crossing band");
        assert_eq!(wait.link, [[0.0, 5.0], [8.0, 5.0]]);
        assert!(wait.walk.is_some(), "first walk along the intact sidewalk to the band");

        net.footways[1].ends[0].kerb_trim = 0.4;
        let mut walker = walker_at_kerb();
        consider_crossing(&net, &mut walker, 0, &mut CrowdStats::default());
        assert!(walker.kerb.is_none(), "the far kerb no longer reaches the band");
    }

    #[test]
    fn a_walker_holds_at_an_occupied_crossing_and_stops_short_of_a_new_obstruction() {
        use crate::signals::{AxisInput, JunctionInput, MemberInput, SignalJunction};
        let paths = eo_data::DataPaths::discover().unwrap();
        let timing = SignalTiming::from_standards(&Standards::load(&paths).unwrap()).unwrap();
        let mut net = Network::new(timing);
        let footway = |x| FootwayBuild {
            points: vec![[x, -20.0], [x, 0.0]], highway: "residential".into(), segment: "street".into(),
            road_side: 1, normal: [1.0, 0.0], cross_index: [None; 2], original_arc_start: None,
            node_keys: Some([format!("start-{x}"), "crossing".into()]), tile: None,
            ends: std::array::from_fn(|_| FootEndBuild { kerb_trim: 0.0, crossing_arc: 2.0, no_crossing: false, zebra_only: false }),
        };
        net.add(NetworkBuild { footways: vec![footway(0.0), footway(8.0)], ..Default::default() });
        let signal = SignalJunction::build(&JunctionInput {
            signal_ids: vec!["signal".into()], centre: [4.0, 0.0], phase_offset: 0.0, orphan: false,
            axes: [0.0, std::f64::consts::FRAC_PI_2].into_iter().enumerate().map(|(lane, bearing)| AxisInput {
                bearing, members: vec![MemberInput { lane: lane as u32, highway: "residential".into(),
                    posted_mps: 11.0, recorded_mps: None, stop_offset_m: 6.0, carriageway_half_m: Some(4.0) }],
            }).collect(), crossing_m: vec![8.0; 2], tile: None,
        }, &net.timing);
        let axis = signal.crossing_axis(0.0).unwrap();
        let walk_time = signal.offset_s + signal.axes[axis].ped.as_ref().unwrap().walk_start_s + 0.001;
        net.junctions.push(signal);
        let fleet = Fleet::default();
        for (signal, time) in [(NONE, 0.0), (0, walk_time)] {
            let mut car = TrafficObstacle { pos: [4.0, 0.0], yaw: 0.0, length: 4.0, width: 2.0, level: 0 };
            let mut walker = Walker::new(1, 0, 20.0, 1, 1.2, 0.0, 1, false, 0.0);
            let link = [[0.0, 0.0], [8.0, 0.0]];
            walker.kerb = Some(KerbWait {
                signal, link, walk: None, next: 1, next_end: 1, arc: 20.0,
                dir: -1, lateral: 0.0, length: 8.0, elapsed: 0.0,
            });
            let mut ctx = walk_context(&net, &fleet, Some(car));
            ctx.time = time;
            if signal != NONE { assert_eq!(signal_may_start(&ctx, signal, &link, 8.0 / walker.speed), Some(true)); }
            let mut stats = CrowdStats::default();
            step_walker(&ctx, &mut walker, FIXED_STEP_S, f64::INFINITY, &mut stats);
            assert!(walker.kerb.is_some() && walker.leg.is_none());
            assert_eq!((walker.s, walker.dir, walker.ground_speed), (20.0, 1, 0.0));

            car.level = 1;
            ctx.vehicle = Some(car);
            step_walker(&ctx, &mut walker, FIXED_STEP_S, f64::INFINITY, &mut stats);
            assert_eq!(walker.leg.map(|leg| leg.kind), Some(LegKind::Cross));
            assert!(walker.kerb.is_none());
            assert_eq!(stats.crossings_started, 1);
            assert_eq!(stats.starts_outside_walk, 0);

            let before = walker.s;
            car.level = 0;
            car.yaw = std::f64::consts::FRAC_PI_2;
            car.pos[0] = position_at(&net, &walker, before)[0] + car.length / 2.0 + PERSON_CAPSULE_R_M + 0.005;
            ctx.vehicle = Some(car);
            step_walker(&ctx, &mut walker, FIXED_STEP_S, f64::INFINITY, &mut stats);
            assert!(walker.s > before && walker.s < before + 0.01, "the stride should stop short of the chassis");
        }
    }

    #[test]
    fn a_walker_on_a_sidewalk_stops_short_of_a_vehicle() {
        let paths = eo_data::DataPaths::discover().unwrap();
        let timing = SignalTiming::from_standards(&Standards::load(&paths).unwrap()).unwrap();
        let mut net = Network::new(timing);
        net.add(NetworkBuild {
            footways: vec![FootwayBuild {
                points: vec![[0.0, 0.0], [0.0, 20.0]],
                highway: "residential".into(), segment: "street".into(),
                road_side: 1, normal: [1.0, 0.0], cross_index: [None; 2],
                original_arc_start: None, node_keys: None, tile: None,
                ends: std::array::from_fn(|_| FootEndBuild {
                    kerb_trim: 0.0, crossing_arc: 0.0, no_crossing: true, zebra_only: false,
                }),
            }],
            ..Default::default()
        });
        let mut walker = Walker::new(1, 0, 8.0, 1, 1.2, 0.0, 1, false, 0.0);
        let [x, z] = position_at(&net, &walker, walker.s);
        let mut vehicle = TrafficObstacle {
            pos: [x, z + 2.0 + PERSON_CAPSULE_R_M + 0.005], yaw: 0.0,
            length: 4.0, width: 2.0, level: 0,
        };
        let fleet = Fleet::default();
        let mut ctx = walk_context(&net, &fleet, Some(vehicle));
        let mut stats = CrowdStats::default();
        step_walker(&ctx, &mut walker, FIXED_STEP_S, f64::INFINITY, &mut stats);
        assert!(walker.leg.is_none());
        assert!(walker.s > 8.0 && walker.s < 8.01);

        let before = walker.s;
        vehicle.level = 1;
        ctx.vehicle = Some(vehicle);
        step_walker(&ctx, &mut walker, FIXED_STEP_S, f64::INFINITY, &mut stats);
        assert!((walker.s - before - walker.speed * FIXED_STEP_S).abs() < 1e-9);
    }
}

/// `updateKerbWait`: true while the walker stands at the kerb.
fn update_kerb_wait(ctx: &WalkCtx, w: &mut Walker, dt: f64, stats: &mut CrowdStats) -> bool {
    let Some(mut wait) = w.kerb else { return false };
    wait.elapsed += dt.max(0.0);
    w.activity = Activity::Wait;
    w.facing = Facing::Across;
    let crossing_seconds = wait.length / w.speed.max(0.4);
    let walk = if wait.signal != NONE { signal_may_start(ctx, wait.signal, &wait.link, crossing_seconds) } else { None };
    let clear = walk.unwrap_or_else(|| clear_of_traffic(ctx, &wait.link, crossing_seconds))
        && crossing_route_clear(ctx, &wait);
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
fn step_walker(ctx: &WalkCtx, w: &mut Walker, dt: f64, gap: f64, stats: &mut CrowdStats) {
    let net = ctx.net;
    let waiting = update_kerb_wait(ctx, w, dt, stats);
    let on_leg = w.leg.is_some();
    let scheduled = if waiting || on_leg { false } else { w.advance_activity(net, ctx.hour, dt, ctx.groups, ctx.windows) };
    if on_leg {
        w.activity = Activity::Walk;
        w.facing = Facing::Keep;
    }
    let moving = on_leg || scheduled;
    let total = w.total(net);
    let routed = w.leg.is_none();
    let (walk_end0, walk_end1) = if routed {
        (foot_end_arc(net, w.footway, 0), total - foot_end_arc(net, w.footway, 1))
    } else {
        (0.0, total)
    };
    // Shorten the stride as the gap closes, and never consume more clear
    // distance than is available (including a far walker's batched steps).
    let stride = (gap / WALKER_SLOW_GAP_M).min(1.0);
    let end_gap = if w.dir > 0 { walk_end1 - w.s } else { w.s - walk_end0 };
    let mut pace = (w.speed * stride).min(gap.min(end_gap.max(0.0)) / dt.max(f64::EPSILON));
    if moving && pace > 0.0 && dt > 0.0 {
        let start = position_at(net, w, w.s);
        let next = w.s + f64::from(w.dir) * pace * dt;
        let end = position_at(net, w, next);
        let mut clear = vehicle_hit_fraction(ctx, start, end, PERSON_CAPSULE_R_M, true);
        if routed && clear < 1.0 {
            // A vehicle may occupy the kerb side while leaving room farther
            // from the road. Only step sideways when that line is clear.
            let lateral = w.lateral;
            let outward = -f64::from(w.road_side) * WALKER_LATERAL_LIMIT_M - w.passing_offset;
            w.lateral = outward;
            let outer_start = position_at(net, w, w.s);
            let outer_end = position_at(net, w, next);
            w.lateral = lateral;
            if vehicle_hit_fraction(ctx, outer_start, outer_end, PERSON_CAPSULE_R_M, false) >= 1.0 {
                let step = GROUP_STEP_ASIDE_MPS * dt;
                let shift = (outward - lateral).clamp(-step, step);
                w.lateral += shift;
                let shifted = position_at(net, w, w.s);
                let side = vehicle_hit_fraction(ctx, start, shifted, PERSON_CAPSULE_R_M, true);
                w.lateral = lateral + shift * side.clamp(0.0, 1.0);
                clear = vehicle_hit_fraction(ctx, position_at(net, w, w.s),
                    position_at(net, w, next), PERSON_CAPSULE_R_M, true);
            }
        }
        pace *= clear.clamp(0.0, 1.0);
        if !routed && pace <= 1e-6 && clear <= 1e-6 {
            w.leg_blocked += dt;
            // A walker frozen mid-leg against a stopped car (which is also
            // yielding to the walker) retreats smoothly back to its origin kerb
            // when the retreat direction is clear, breaking mutual deadlocks
            // without ever clipping a vehicle.
            if w.leg_blocked >= 5.0 {
                if let Some(mut leg) = w.leg.filter(|l| !l.retreating) {
                    let n = leg.n as usize;
                    let mut rev_pts = leg.pts;
                    rev_pts[..n].reverse();
                    let mut rev_cum = [0.0; 3];
                    for k in 1..n {
                        rev_cum[k] = rev_cum[k - 1] + geom::hypot(rev_pts[k][0] - rev_pts[k - 1][0], rev_pts[k][1] - rev_pts[k - 1][1]);
                    }
                    let rev_s = (leg.total - w.s).clamp(0.01, (leg.total - 0.01).max(0.01));
                    let rev_next = (rev_s + w.speed * dt).min(leg.total);
                    let (rx0, rz0, _, _, _) = geom::at_arc(&rev_pts[..n], &rev_cum[..n], rev_s);
                    let (rx1, rz1, _, _, _) = geom::at_arc(&rev_pts[..n], &rev_cum[..n], rev_next);
                    if vehicle_hit_fraction(ctx, [rx0, rz0], [rx1, rz1], PERSON_CAPSULE_R_M, true) > 0.0 {
                        leg.pts = rev_pts;
                        leg.cum = rev_cum;
                        leg.retreating = true;
                        leg.next = w.footway;
                        leg.arc = leg.origin_arc;
                        leg.dir = leg.origin_dir;
                        w.leg = Some(leg);
                        w.s = rev_s;
                        w.dir = 1;
                        w.leg_blocked = 0.0;
                    }
                }
            }
        } else {
            w.leg_blocked = 0.0;
        }
    } else if !routed {
        w.leg_blocked = 0.0;
    }
    w.ground_speed = if moving { pace } else { 0.0 };
    if moving {
        w.s += w.dir as f64 * pace * dt;
        if pace > 0.0 { w.gait += w.gait_freq * dt * (pace / w.speed); }
    }
    if moving && routed && w.kerb.is_none() {
        let toward = if w.dir > 0 { 1 } else { 0 };
        let to_end = if w.dir > 0 { total - w.s } else { w.s };
        if to_end <= net.footways[w.footway as usize].ends[toward].crossing_arc {
            consider_crossing(net, w, toward, stats);
        }
    }
    if w.s >= walk_end1 {
        w.s = walk_end1;
        if route_at_end(ctx, w, 1, stats) == PathEnd::Reverse {
            w.dir = -1;
        }
    } else if w.s <= walk_end0 {
        w.s = walk_end0;
        if route_at_end(ctx, w, 0, stats) == PathEnd::Reverse {
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
