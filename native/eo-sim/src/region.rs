//! A region: one simulation island around one focus (a player, a camera, a
//! server-side interest point) - its fleet, its crowd, the population rules
//! that decide who exists near the focus, and the level of detail of the
//! agents far from it.
//!
//! Regions share nothing mutable: each reads the network and the clock and
//! owns its agents, its random sequences and its scratch buffers. That is
//! what lets a `World` step regions on separate threads, and what lets the
//! same core run server-side with one region per player or per interest cell
//! (see README "Simulation core").
//!
//! POPULATION, as the JS game decides it:
//!
//!   cars     A pool of `VEHICLE_POOL` (42, `STREET_POPULATION.vehiclePool`;
//!            the High vehicle preset's `trafficMax`). The local-life recycler
//!            (`updateLocalLife` / `recycleCarsNearFocus`) re-targets the pool
//!            around the focus every second or every 70 m of focus travel:
//!            `round(32 * (0.55 + 0.45 * footfall(hour)))` cars inside 120 m,
//!            donors taken only from 240 m out and off camera, placed down the
//!            block but never in shot closer than 34 m. Cars beyond 120 m
//!            prefer the turn that brings them back (the homing bias).
//!   people   The People setting's resident disc (`crowdPopulationBubble`;
//!            the default `medium`: 40 bodies within 55 m, kept to 71 m,
//!            never appearing closer than 18 m or inside the 60-degree view
//!            cone closer than 38 m), refilled on the life materializer's
//!            0.5 s scan; a focus that jumps further than the disc is a cut and
//!            the street is rebuilt from scratch. `People::Pool` instead
//!            spawns a fixed ambient pool across the whole network, as
//!            `spawnAmbientCrowd` does, for comparison runs.
//!
//! LEVEL OF DETAIL. A walker farther than 60 m from the focus
//! (`SIM_TIER_CONTRACT.farRadiusM`, back inside 55 m) and a car beyond the
//! drawn traffic window (240 m) are stepped every second fixed step with the
//! time they skipped (alternating by index, so half the far set steps each
//! step) - the JS `farBridge` cadence, which the JS keeps off only because of
//! the life layer's street-wide encounter caps, which this crowd does not have.

use crate::consts::*;
use crate::geom::{self, js_clamp};
use crate::grid::Grid;
use crate::network::{FootwayId, LaneId, Network, TileKey};
use crate::output::*;
use crate::pedestrians::{Crowd, LegKind, WalkCtx, Walker};
use crate::rng::Mulberry32;
use crate::vehicles::{Band, Fleet, StepCtx};

#[derive(Clone, Copy, Debug)]
pub enum People {
    /// The life materializer's resident disc for a People setting.
    Bubble(CrowdBubble),
    /// A fixed ambient pool across the network (`spawnAmbientCrowd`).
    Pool(usize),
    None,
}

#[derive(Clone, Copy, Debug)]
pub struct RegionConfig {
    pub seed_int: u32,
    pub vehicle_pool: usize,
    pub people: People,
    /// Run the car recycler around the focus (the JS needs a camera for it).
    pub local_life: bool,
    /// Hour of day at time 0.
    pub hour: f64,
    pub lod: bool,
    /// Spawn the fleet within this distance of the focus (None: the whole resident network).
    pub spawn_radius: Option<f64>,
    /// Horizontal half field of view of the camera the visibility test stands in for.
    pub view_half_angle: f64,
    /// The drawn traffic window (`VEHICLE_QUALITY[level].trafficDistance`).
    pub draw_horizon: f64,
    /// The crowd by place and hour (eo-app `--crowd`, `CrowdDensity`) in
    /// place of `people`'s fixed resident disc; None (the default) as before.
    pub crowd: Option<CrowdDensity>,
}

/// The game camera of the JS harnesses: 52 degrees vertical at 16:9.
pub fn game_camera_half_angle() -> f64 {
    ((26.0f64).to_radians().tan() * 16.0 / 9.0).atan()
}

impl RegionConfig {
    /// The shipped defaults: High vehicles (42 moving, 240 m window), People `medium` (`DEFAULT_PEOPLE_LEVEL`).
    pub fn game_default(seed_int: u32) -> Self {
        RegionConfig {
            seed_int,
            vehicle_pool: VEHICLE_POOL,
            people: People::Bubble(CrowdBubble::for_level(DEFAULT_PEOPLE_LEVEL)),
            local_life: true,
            hour: 12.0,
            lod: true,
            spawn_radius: Some(LOCAL_RECYCLE_RADIUS),
            view_half_angle: game_camera_half_angle(),
            draw_horizon: traffic_window(Level::High).1,
            crowd: None,
        }
    }

    /// The configuration of the JS oracle run: TrafficSim with no camera (no
    /// recycler), the whole window, a fixed ambient pool, no level of detail.
    pub fn js_pool(seed_int: u32, walkers: usize) -> Self {
        RegionConfig {
            seed_int,
            vehicle_pool: VEHICLE_POOL,
            people: People::Pool(walkers),
            local_life: false,
            hour: 12.0,
            lod: false,
            spawn_radius: None,
            view_half_angle: game_camera_half_angle(),
            draw_horizon: traffic_window(Level::High).1,
            crowd: None,
        }
    }
}

/// What the crowd's disc holds (`Region::crowd_census`, `--crowd`).
#[derive(Clone, Copy, Debug, Default)]
pub struct CrowdCensus {
    pub footways: usize,
    pub length_m: f64,
    pub shops: f64,
    pub night: f64,
    pub storeys: f64,
    pub class_weight: f64,
    pub people: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct Focus {
    pub x: f64,
    pub z: f64,
    /// View yaw, `atan2(dx, dz)`; NaN when unknown.
    pub yaw: f64,
}

#[derive(Clone, Debug, Default)]
pub struct RegionStats {
    pub car_recycles: u64,
    pub focus_updates: u64,
    pub focus_cuts: u64,
    /// cars a shared region added to its fleet because it had no donor.
    pub cars_grown: u64,
}

/// the rectangle a shared region populates (a server's region cell):
/// recycled cars, new walkers and grown cars are put down only inside it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Claim {
    pub min: [f64; 2],
    pub max: [f64; 2],
}

impl Claim {
    pub fn contains(&self, x: f64, z: f64) -> bool {
        x >= self.min[0] && x < self.max[0] && z >= self.min[1] && z < self.max[1]
    }

    /// Horizontal distance from (x, z) to the rectangle (0 inside).
    pub fn distance(&self, x: f64, z: f64) -> f64 {
        let dx = (self.min[0] - x).max(0.0).max(x - self.max[0]);
        let dz = (self.min[1] - z).max(0.0).max(z - self.max[1]);
        dx.hypot(dz)
    }

    /// The share of a disc inside the rectangle (8 x 8 samples).
    pub fn disc_share(&self, x: f64, z: f64, r: f64) -> f64 {
        if self.distance(x, z) >= r {
            return 0.0;
        }
        let (mut inside, mut all) = (0u32, 0u32);
        for i in 0..8 {
            for j in 0..8 {
                let (u, v) = ((i as f64 + 0.5) / 4.0 - 1.0, (j as f64 + 0.5) / 4.0 - 1.0);
                if u * u + v * v > 1.0 {
                    continue;
                }
                all += 1;
                inside += self.contains(x + u * r, z + v * r) as u32;
            }
        }
        inside as f64 / all.max(1) as f64
    }
}

/// the method form of `Region::visible` for one viewer.
fn visible_from(f: &Focus, horizon: f64, half: f64, x: f64, z: f64) -> bool {
    let (dx, dz) = (x - f.x, z - f.z);
    let d = geom::hypot(dx, dz);
    if d > horizon + 14.0 {
        return false;
    }
    if !f.yaw.is_finite() {
        return d < horizon;
    }
    if d < 1e-3 {
        return true;
    }
    (dx * f.yaw.sin() + dz * f.yaw.cos()) / d >= half.cos()
}

pub struct Region {
    pub id: u32,
    pub cfg: RegionConfig,
    pub focus: Focus,
    pub fleet: Fleet,
    pub crowd: Crowd,
    car_rng: Mulberry32,
    walker_rng: Mulberry32,
    local_timer: f64,
    local_focus: Option<[f64; 2]>,
    scan_timer: f64,
    last_focus: Option<[f64; 2]>,
    cold_seed: bool,
    walker_grid: Grid,
    bands: Vec<Band>,
    placements: Vec<(f64, LaneId, f64)>,
    foot_scratch: Vec<(FootwayId, f64)>,
    donors: Vec<(f64, usize)>,
    pub step_index: u64,
    pub out: AgentBuffer,
    pub stats: RegionStats,
    /// Some = a SHARED region (a server's region cell several players
    /// see): the recycler, the crowd disc, the level of detail and the
    /// visibility tests follow every viewer in the list, the population is put
    /// down only inside `claim`, and `focus` only sets the output origin and
    /// the homing bias. None (the default): the single-focus rules above, unchanged.
    pub viewers: Option<Vec<Focus>>,
    /// where a shared region may put agents down (None: anywhere).
    pub claim: Option<Claim>,
    shared_last: Vec<[f64; 2]>,
    view_scratch: Vec<Focus>,
}

impl Region {
    pub fn new(net: &Network, id: u32, cfg: RegionConfig, focus: Focus) -> Self {
        Self::with_id_base(net, id, cfg, focus, 0)
    }

    /// `new`, numbering cars and walkers from `base` (a server gives each
    /// region a disjoint base, so ids stay unique when agents are handed off).
    pub fn with_id_base(net: &Network, id: u32, cfg: RegionConfig, focus: Focus, base: u32) -> Self {
        // Region 0 replays the JS fleet seed (`mulberry32(seedInt + 77)`); others are decorrelated by id.
        let car_seed = cfg.seed_int.wrapping_add(77).wrapping_add(id.wrapping_mul(0x9e37_79b9));
        let mut r = Region {
            id,
            cfg,
            focus,
            fleet: Fleet::default(),
            crowd: Crowd::default(),
            car_rng: Mulberry32::new(car_seed),
            walker_rng: Mulberry32::new(car_seed ^ 0x5bd1_e995),
            local_timer: 0.0,
            local_focus: None,
            scan_timer: 0.0,
            last_focus: None,
            cold_seed: true,
            walker_grid: Grid::new(PERSON_CELL_M),
            bands: vec![],
            placements: vec![],
            foot_scratch: vec![],
            donors: vec![],
            step_index: 0,
            out: AgentBuffer::default(),
            stats: RegionStats::default(),
            viewers: None, // net
            claim: None,
            shared_last: vec![],
            view_scratch: vec![],
        };
        r.fleet.next_id = base; // net
        r.crowd.next_key = base;
        r.spawn_fleet(net);
        if let People::Pool(n) = cfg.people {
            r.spawn_pool(net, n);
        }
        r
    }

    fn spawn_fleet(&mut self, net: &Network) {
        let lanes: Vec<LaneId> = match self.cfg.spawn_radius {
            None => net.alive_lanes().collect(),
            Some(radius) => {
                let mut v: Vec<LaneId> = net
                    .tiles_near(self.focus.x, self.focus.z, radius)
                    .flat_map(|t| t.lanes.iter().copied())
                    .filter(|&l| {
                        let lane = &net.lanes[l as usize];
                        lane.alive
                            && geom::dist_to_polyline(&lane.pts, self.focus.x, self.focus.z) <= radius
                            && lane.length > 8.0
                    })
                    .collect();
                v.sort_unstable();
                v.dedup();
                v
            }
        };
        for _ in 0..self.cfg.vehicle_pool {
            if !self.fleet.spawn(net, &lanes, &mut self.car_rng, self.cfg.seed_int) {
                continue;
            }
        }
        self.fleet.separate(net);
    }

    /// One group of ambient walkers (`spawnAmbientCrowd`'s inner body).
    fn spawn_group(&mut self, net: &Network, footway: FootwayId, base_arc: Option<f64>) -> usize {
        let rng = &mut self.walker_rng;
        let f = &net.footways[footway as usize];
        let total = f.length;
        let as_group = rng.next_f64() < GROUP_SHARE;
        let members = if as_group {
            GROUP_SIZE[1].min(GROUP_SIZE[0] + (rng.next_f64() * (GROUP_SIZE[1] - GROUP_SIZE[0] + 1) as f64) as u32)
        } else {
            1
        };
        let dir: i8 = if rng.next_f64() < 0.5 { 1 } else { -1 };
        let base = base_arc.unwrap_or_else(|| rng.next_f64() * total);
        let _ = base_arc.is_some() || true;
        let speed = pedestrian_cruise_speed(rng.next_f64());
        let side = f.road_side as f64;
        for m in 0..members {
            let lateral = if members > 1 {
                ((m as f64 - (members - 1) as f64 / 2.0) * 0.38 + rng.next_f64() * 0.14) * side
            } else {
                (rng.next_f64() * 0.90 - 0.38) * side
            };
            let solo = if as_group { None } else { Some(rng.next_f64()) };
            let _cadence = rng.next_f64();
            let gait = rng.next_f64() * 6.28;
            let _bob = rng.next_f64();
            let walker_speed = match solo {
                Some(u) => solo_cruise_speed(speed, u),
                None => speed,
            };
            let s = js_clamp(base + (m as f64 - (members - 1) as f64 / 2.0) * 0.9, 0.0, total);
            let key = self.crowd.next_key;
            self.crowd.next_key += 1;
            self.crowd.push(Walker::new(key, footway, s, dir, walker_speed, lateral, f.road_side, as_group, gait));
            self.crowd.stats.spawned += 1;
        }
        members as usize
    }

    /// `spawnAmbientCrowd`: `n` walkers on footways weighted by street class.
    fn spawn_pool(&mut self, net: &Network, n: usize) {
        let ids: Vec<FootwayId> = net.alive_footways().filter(|&f| net.footways[f as usize].length >= 4.0).collect();
        if ids.is_empty() {
            return;
        }
        let mut cumulative = Vec::with_capacity(ids.len());
        let mut running = 0.0;
        for &f in &ids {
            running += net.footways[f as usize].weight;
            cumulative.push(running);
        }
        let mut guard = 0;
        while self.crowd.walkers.len() < n && guard < n * 6 {
            guard += 1;
            let t = self.walker_rng.next_f64() * running;
            let k = cumulative.partition_point(|&c| c < t).min(ids.len() - 1);
            self.spawn_group(net, ids[k], None);
        }
        self.crowd.walkers.truncate(n);
        self.crowd.x.truncate(n);
        self.crowd.z.truncate(n);
        self.crowd.refresh_positions(net);
    }

    /// Would a point be on camera? A cone and the draw horizon (the JS adds an occlusion probe).
    pub fn visible(&self, x: f64, z: f64) -> bool {
        if let Some(vs) = &self.viewers {
            // a shared region: seen by any viewer.
            return vs.iter().any(|f| visible_from(f, self.cfg.draw_horizon, self.cfg.view_half_angle, x, z));
        }
        let f = &self.focus;
        let (dx, dz) = (x - f.x, z - f.z);
        let d = geom::hypot(dx, dz);
        if d > self.cfg.draw_horizon + 14.0 {
            return false;
        }
        if !f.yaw.is_finite() {
            return d < self.cfg.draw_horizon;
        }
        if d < 1e-3 {
            return true;
        }
        (dx * f.yaw.sin() + dz * f.yaw.cos()) / d >= self.cfg.view_half_angle.cos()
    }

    /// `updateLocalLife` + `recycleCarsNearFocus`.
    fn local_life(&mut self, net: &Network, dt: f64, hour: f64) {
        if !self.cfg.local_life {
            return;
        }
        self.local_timer -= dt;
        let focus = [self.focus.x, self.focus.z];
        let moved = self.local_focus.is_none_or(|f| geom::hypot(focus[0] - f[0], focus[1] - f[1]) >= LOCAL_FOCUS_SHIFT);
        if !moved && self.local_timer > 0.0 {
            return;
        }
        self.local_timer = 1.0;
        self.local_focus = Some(focus);
        self.stats.focus_updates += 1;
        let first = self.stats.focus_updates == 1;
        let footfall = hour_footfall(hour);
        let car_target = crate::standards::js_round(LOCAL_CAR_TARGET * (0.55 + 0.45 * footfall)) as usize;
        let n = self.fleet.len();
        let mut local = 0;
        let mut on_screen = 0;
        for i in 0..n {
            let (x, z) = (self.fleet.x[i], self.fleet.z[i]);
            if geom::hypot(x - focus[0], z - focus[1]) <= LOCAL_LIFE_RADIUS {
                local += 1;
                if self.visible(x, z) {
                    on_screen += 1;
                }
            }
        }
        let starved = (local as f64) < car_target as f64 * LOCAL_STARVATION_RATIO;
        let allow_visible = first || starved || on_screen < LOCAL_ONSCREEN_CAR_TARGET;
        let needed = car_target.saturating_sub(local);
        if needed == 0 {
            return;
        }
        // Placements: lanes of the ordinary classes whose nearest point is inside 112 m.
        self.placements.clear();
        let max_d = LOCAL_LIFE_RADIUS - 8.0;
        for t in net.tiles_near(focus[0], focus[1], max_d) {
            for &l in &t.lanes {
                let lane = &net.lanes[l as usize];
                if !lane.alive
                    || !matches!(lane.highway.as_str(), "primary" | "secondary" | "tertiary" | "residential" | "unclassified" | "service")
                {
                    continue;
                }
                let (mut best_d, mut best_arc) = (f64::INFINITY, 0.0);
                for k in 0..lane.pts.len() - 1 {
                    let (a, b) = (lane.pts[k], lane.pts[k + 1]);
                    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
                    let len2 = (dx * dx + dz * dz).max(1e-9);
                    let t = (((focus[0] - a[0]) * dx + (focus[1] - a[1]) * dz) / len2).clamp(0.0, 1.0);
                    let d = geom::hypot(a[0] + dx * t - focus[0], a[1] + dz * t - focus[1]);
                    if d < best_d {
                        best_d = d;
                        best_arc = lane.cum[k] + t * (lane.cum[k + 1] - lane.cum[k]);
                    }
                }
                if best_d <= max_d {
                    self.placements.push((best_d, l, best_arc));
                }
            }
        }
        if self.placements.is_empty() {
            return;
        }
        self.placements.sort_by(|a, b| a.0.total_cmp(&b.0).then(net.lanes[a.1 as usize].key.cmp(&net.lanes[b.1 as usize].key)));
        self.donors.clear();
        for i in 0..n {
            let (x, z) = (self.fleet.x[i], self.fleet.z[i]);
            let d = geom::hypot(x - focus[0], z - focus[1]);
            if d >= LOCAL_RECYCLE_RADIUS && !self.visible(x, z) {
                self.donors.push((d, i));
            }
        }
        self.donors.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut placed = 0usize;
        for di in 0..self.donors.len() {
            if placed >= needed {
                break;
            }
            let car = self.donors[di].1;
            let passes: &[bool] = if allow_visible { &[true, false] } else { &[false] };
            let mut selected = None;
            'passes: for &prefer_visible in passes {
                for attempt in 0..self.placements.len() {
                    let (_, lane_id, arc0) = self.placements[(placed * 7 + attempt) % self.placements.len()];
                    let lane = &net.lanes[lane_id as usize];
                    let jitter = ((placed % 5) as f64 - 2.0) * 5.5;
                    let arc = js_clamp(arc0 + jitter, 2.0, (lane.length - 2.0).max(2.0));
                    let (x, z, _, _, _) = geom::at_arc(&lane.pts, &lane.cum, arc);
                    let visible_after = self.visible(x, z);
                    if visible_after && geom::hypot(x - focus[0], z - focus[1]) < POP_IN_GUARD_METRES {
                        continue;
                    }
                    if prefer_visible && !visible_after {
                        continue;
                    }
                    if !(allow_visible || !visible_after) {
                        continue;
                    }
                    if self.fleet.insert_blocked(net, lane_id, arc, car) || self.fleet.landing_unfit(net, lane_id, arc, car, 4.5) {
                        continue;
                    }
                    selected = Some((lane_id, arc));
                    break 'passes;
                }
            }
            let Some((lane, arc)) = selected else { continue };
            let speed = 2.0 + self.car_rng.next_f64() * 2.5;
            self.fleet.place(net, car, lane, arc, speed);
            self.stats.car_recycles += 1;
            placed += 1;
        }
    }

    /// The resident disc around the focus (life materializer scan/retire).
    fn bubble(&mut self, net: &Network, dt: f64, bubble: CrowdBubble) {
        let focus = [self.focus.x, self.focus.z];
        if let Some(last) = self.last_focus {
            if geom::hypot(focus[0] - last[0], focus[1] - last[1]) > bubble.radius_m {
                // A focus teleport is a cut: the street is rebuilt where the focus arrived.
                while !self.crowd.walkers.is_empty() {
                    self.crowd.remove(self.crowd.walkers.len() - 1);
                }
                self.cold_seed = true;
                self.scan_timer = f64::INFINITY;
                self.stats.focus_cuts += 1;
            }
        }
        self.last_focus = Some(focus);
        // Retire every step beyond the keep radius.
        let keep2 = bubble.keep_radius_m * bubble.keep_radius_m;
        let mut i = self.crowd.walkers.len();
        while i > 0 {
            i -= 1;
            let (dx, dz) = (self.crowd.x[i] - focus[0], self.crowd.z[i] - focus[1]);
            if dx * dx + dz * dz > keep2 {
                self.crowd.remove(i);
            }
        }
        self.scan_timer += dt;
        if self.scan_timer < SCAN_INTERVAL_S {
            return;
        }
        self.scan_timer = 0.0;
        let deficit = bubble.max_bodies.saturating_sub(self.crowd.walkers.len());
        if deficit == 0 {
            self.cold_seed = false;
            return;
        }
        // Candidate footways inside the disc.
        self.foot_scratch.clear();
        let mut running = 0.0;
        for t in net.tiles_near(focus[0], focus[1], bubble.radius_m) {
            for &f in &t.footways {
                let fw = &net.footways[f as usize];
                if !fw.alive || fw.length < 4.0 {
                    continue;
                }
                if geom::dist_to_polyline(&fw.pts, focus[0], focus[1]) > bubble.radius_m {
                    continue;
                }
                running += fw.weight;
                self.foot_scratch.push((f, running));
            }
        }
        if self.foot_scratch.is_empty() {
            return;
        }
        let cos_cone = IN_SHOT_HALF_ANGLE_RAD.cos();
        let mut spawned = 0;
        let mut attempts = 0;
        while spawned < deficit && attempts < deficit * 4 {
            attempts += 1;
            let t = self.walker_rng.next_f64() * running;
            let k = self.foot_scratch.partition_point(|e| e.1 < t).min(self.foot_scratch.len() - 1);
            let f = self.foot_scratch[k].0;
            let fw = &net.footways[f as usize];
            let arc = self.walker_rng.next_f64() * fw.length;
            let (x, z, _, _, _) = geom::at_arc(&fw.pts, &fw.cum, arc);
            let (dx, dz) = (x - focus[0], z - focus[1]);
            let d = geom::hypot(dx, dz);
            if d > bubble.radius_m {
                continue;
            }
            if !self.cold_seed {
                if d < bubble.pop_in_guard_m {
                    continue;
                }
                if self.focus.yaw.is_finite() && d < bubble.in_shot_guard_m && d > 1e-3 {
                    let c = (dx * self.focus.yaw.sin() + dz * self.focus.yaw.cos()) / d;
                    if c >= cos_cone {
                        continue;
                    }
                }
            }
            let before = self.crowd.walkers.len();
            let members = self.spawn_group(net, f, Some(arc));
            spawned += members;
            for w in before..self.crowd.walkers.len() {
                let (x, z) = {
                    let wk = &self.crowd.walkers[w];
                    let fw = &net.footways[wk.footway as usize];
                    let (x, z, _, _, _) = geom::at_arc(&fw.pts, &fw.cum, wk.s);
                    (x, z)
                };
                self.crowd.x[w] = x;
                self.crowd.z[w] = z;
            }
        }
        while self.crowd.walkers.len() > bubble.max_bodies {
            let last = self.crowd.walkers.len() - 1;
            self.crowd.remove(last);
            self.crowd.stats.spawned -= 1;
            self.crowd.stats.retired -= 1;
        }
        self.cold_seed = false;
    }

    /// What the disc holds (`--crowd`): footways and their length inside
    /// it, their length-weighted mean activity and class weight, and the
    /// people they hold at `hour` (every footway's density over its length
    /// inside the disc, before the body budget).
    pub fn crowd_census(&self, net: &Network, d: CrowdDensity, hour: f64) -> CrowdCensus {
        let focus = [self.focus.x, self.focus.z];
        let mut c = CrowdCensus::default();
        let mut seen = std::collections::HashSet::new();
        for t in net.tiles_near(focus[0], focus[1], d.radius_m) {
            for &f in &t.footways {
                let fw = &net.footways[f as usize];
                if !fw.alive || fw.length < 4.0 || !seen.insert(f) {
                    continue;
                }
                let inside = geom::length_in_disc(&fw.pts, focus, d.radius_m);
                if inside <= 0.0 {
                    continue;
                }
                c.footways += 1;
                c.length_m += inside;
                c.shops += fw.activity.shops as f64 * inside;
                c.night += fw.activity.night as f64 * inside;
                c.storeys += fw.activity.storeys as f64 * inside;
                c.class_weight += fw.weight * inside;
                c.people += footway_density(fw.weight, fw.activity, hour) * d.scale * inside;
            }
        }
        if c.length_m > 0.0 {
            for v in [&mut c.shops, &mut c.night, &mut c.storeys, &mut c.class_weight] {
                *v /= c.length_m;
            }
        }
        c
    }

    /// The resident disc by place and hour (`--crowd`; `bubble`'s rules with
    /// the body count each footway's density gives in place of a fixed one):
    /// a focus that jumps further than the disc is a cut and the street is
    /// rebuilt from scratch (every footway in the disc filled, in shot too);
    /// after that the disc is refilled on the materializer's scan, off shot
    /// beyond the pop-in guard or in shot beyond the in-shot guard, each
    /// footway in proportion to the people it holds inside the disc.
    fn density_bubble(&mut self, net: &Network, dt: f64, d: CrowdDensity, hour: f64) {
        let focus = [self.focus.x, self.focus.z];
        if let Some(last) = self.last_focus {
            if geom::hypot(focus[0] - last[0], focus[1] - last[1]) > d.radius_m {
                while !self.crowd.walkers.is_empty() {
                    self.crowd.remove(self.crowd.walkers.len() - 1);
                }
                self.cold_seed = true;
                self.scan_timer = f64::INFINITY;
                self.stats.focus_cuts += 1;
            }
        }
        self.last_focus = Some(focus);
        let keep2 = d.keep_radius_m * d.keep_radius_m;
        let mut i = self.crowd.walkers.len();
        while i > 0 {
            i -= 1;
            let (dx, dz) = (self.crowd.x[i] - focus[0], self.crowd.z[i] - focus[1]);
            if dx * dx + dz * dz > keep2 {
                self.crowd.remove(i);
            }
        }
        self.scan_timer += dt;
        if self.scan_timer < SCAN_INTERVAL_S {
            return;
        }
        self.scan_timer = 0.0;
        // Candidate footways inside the disc, each weighted by the people it holds there.
        self.foot_scratch.clear();
        let mut running = 0.0;
        for t in net.tiles_near(focus[0], focus[1], d.radius_m) {
            for &f in &t.footways {
                let fw = &net.footways[f as usize];
                if !fw.alive || fw.length < 4.0 {
                    continue;
                }
                let inside = geom::length_in_disc(&fw.pts, focus, d.radius_m);
                if inside <= 0.0 {
                    continue;
                }
                let people = footway_density(fw.weight, fw.activity, hour) * d.scale * inside;
                if people <= 0.0 {
                    continue;
                }
                running += people;
                self.foot_scratch.push((f, running));
            }
        }
        let target = (crate::standards::js_round(running) as usize).min(d.max_bodies);
        let deficit = target.saturating_sub(self.crowd.walkers.len());
        if deficit == 0 || self.foot_scratch.is_empty() {
            self.cold_seed = false;
            return;
        }
        let cos_cone = IN_SHOT_HALF_ANGLE_RAD.cos();
        let mut spawned = 0;
        let mut attempts = 0;
        while spawned < deficit && attempts < deficit * 8 {
            attempts += 1;
            let t = self.walker_rng.next_f64() * running;
            let k = self.foot_scratch.partition_point(|e| e.1 < t).min(self.foot_scratch.len() - 1);
            let f = self.foot_scratch[k].0;
            let fw = &net.footways[f as usize];
            let arc = self.walker_rng.next_f64() * fw.length;
            let (x, z, _, _, _) = geom::at_arc(&fw.pts, &fw.cum, arc);
            let (dx, dz) = (x - focus[0], z - focus[1]);
            let dist = geom::hypot(dx, dz);
            if dist > d.radius_m {
                continue;
            }
            if !self.cold_seed {
                if dist < d.pop_in_guard_m {
                    continue;
                }
                if self.focus.yaw.is_finite() && dist < d.in_shot_guard_m && dist > 1e-3 {
                    let c = (dx * self.focus.yaw.sin() + dz * self.focus.yaw.cos()) / dist;
                    if c >= cos_cone {
                        continue;
                    }
                }
            }
            let before = self.crowd.walkers.len();
            spawned += self.spawn_group(net, f, Some(arc));
            for w in before..self.crowd.walkers.len() {
                let (x, z) = {
                    let wk = &self.crowd.walkers[w];
                    let fw = &net.footways[wk.footway as usize];
                    let (x, z, _, _, _) = geom::at_arc(&fw.pts, &fw.cum, wk.s);
                    (x, z)
                };
                self.crowd.x[w] = x;
                self.crowd.z[w] = z;
            }
        }
        while self.crowd.walkers.len() > target {
            let last = self.crowd.walkers.len() - 1;
            self.crowd.remove(last);
            self.crowd.stats.spawned -= 1;
            self.crowd.stats.retired -= 1;
        }
        self.cold_seed = false;
    }

    /// seed the next refill with every pop-in guard on (a shared region
    /// opened beside a live one, whose viewers already see the street).
    pub fn warm(&mut self) {
        self.cold_seed = false;
        self.stats.focus_updates = self.stats.focus_updates.max(1);
    }

    /// the recycler of a shared region (`updateLocalLife` per viewer).
    /// Each viewer wants the JS car target within 120 m, scaled by the share
    /// of that disc inside the claim (so two regions meeting at a border
    /// together hold the single-player density); donors are cars 240 m from
    /// every viewer and seen by none; with no donor left the fleet grows (up
    /// to `vehicle_pool` per viewer) and the new car is placed by the same rules.
    fn shared_life(&mut self, net: &Network, dt: f64, hour: f64) {
        if !self.cfg.local_life {
            return;
        }
        let mut vs = std::mem::take(&mut self.view_scratch);
        vs.clear();
        vs.extend_from_slice(self.viewers.as_deref().unwrap_or(&[]));
        self.local_timer -= dt;
        let moved = vs.len() != self.shared_last.len() || vs.iter().zip(&self.shared_last).any(|(f, l)| geom::hypot(f.x - l[0], f.z - l[1]) >= LOCAL_FOCUS_SHIFT);
        if vs.is_empty() || (!moved && self.local_timer > 0.0) {
            self.view_scratch = vs;
            return;
        }
        self.local_timer = 1.0;
        self.shared_last.clear();
        self.shared_last.extend(vs.iter().map(|f| [f.x, f.z]));
        self.stats.focus_updates += 1;
        let first = self.stats.focus_updates == 1;
        let (horizon, half) = (self.cfg.draw_horizon, self.cfg.view_half_angle);
        let seen = |vs: &[Focus], x: f64, z: f64| vs.iter().any(|f| visible_from(f, horizon, half, x, z));
        let footfall = hour_footfall(hour);
        let car_target = crate::standards::js_round(LOCAL_CAR_TARGET * (0.55 + 0.45 * footfall)) as f64;
        let cap = self.cfg.vehicle_pool.max(VEHICLE_POOL) * vs.len();
        self.donors.clear();
        for i in 0..self.fleet.len() {
            let (x, z) = (self.fleet.x[i], self.fleet.z[i]);
            let d = vs.iter().map(|f| geom::hypot(x - f.x, z - f.z)).fold(f64::INFINITY, f64::min);
            if d >= LOCAL_RECYCLE_RADIUS && !seen(&vs, x, z) {
                self.donors.push((d, i));
            }
        }
        self.donors.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        let mut next_donor = 0;
        for f in vs.iter() {
            let focus = [f.x, f.z];
            let share = self.claim.map_or(1.0, |c| c.disc_share(f.x, f.z, LOCAL_LIFE_RADIUS));
            let target = (car_target * share).round() as usize;
            let (mut local, mut on_screen) = (0usize, 0usize);
            for i in 0..self.fleet.len() {
                let (x, z) = (self.fleet.x[i], self.fleet.z[i]);
                if geom::hypot(x - focus[0], z - focus[1]) <= LOCAL_LIFE_RADIUS {
                    local += 1;
                    on_screen += visible_from(f, horizon, half, x, z) as usize;
                }
            }
            let needed = target.saturating_sub(local);
            if needed == 0 {
                continue;
            }
            let starved = (local as f64) < target as f64 * LOCAL_STARVATION_RATIO;
            let allow_visible = first || starved || on_screen < LOCAL_ONSCREEN_CAR_TARGET;
            self.placements.clear();
            let max_d = LOCAL_LIFE_RADIUS - 8.0;
            for t in net.tiles_near(focus[0], focus[1], max_d) {
                for &l in &t.lanes {
                    let lane = &net.lanes[l as usize];
                    if !lane.alive
                        || !matches!(lane.highway.as_str(), "primary" | "secondary" | "tertiary" | "residential" | "unclassified" | "service")
                    {
                        continue;
                    }
                    let (mut best_d, mut best_arc) = (f64::INFINITY, 0.0);
                    for k in 0..lane.pts.len() - 1 {
                        let (a, b) = (lane.pts[k], lane.pts[k + 1]);
                        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
                        let len2 = (dx * dx + dz * dz).max(1e-9);
                        let t = (((focus[0] - a[0]) * dx + (focus[1] - a[1]) * dz) / len2).clamp(0.0, 1.0);
                        let d = geom::hypot(a[0] + dx * t - focus[0], a[1] + dz * t - focus[1]);
                        if d < best_d {
                            best_d = d;
                            best_arc = lane.cum[k] + t * (lane.cum[k + 1] - lane.cum[k]);
                        }
                    }
                    if best_d <= max_d {
                        self.placements.push((best_d, l, best_arc));
                    }
                }
            }
            if self.placements.is_empty() {
                continue;
            }
            self.placements.sort_by(|a, b| a.0.total_cmp(&b.0).then(net.lanes[a.1 as usize].key.cmp(&net.lanes[b.1 as usize].key)));
            self.placements.dedup_by_key(|p| p.1);
            let mut placed = 0usize;
            let mut tries = 0usize;
            while placed < needed && tries < needed * 2 {
                tries += 1;
                // A donor, else a new car while the fleet is under its cap.
                let (car, grown) = if next_donor < self.donors.len() {
                    next_donor += 1;
                    (self.donors[next_donor - 1].1, false)
                } else if self.fleet.len() < cap {
                    let lane = self.placements[(placed * 7) % self.placements.len()].1;
                    if !self.fleet.spawn(net, &[lane], &mut self.car_rng, self.cfg.seed_int) {
                        continue;
                    }
                    (self.fleet.len() - 1, true)
                } else {
                    break;
                };
                let passes: &[bool] = if allow_visible { &[true, false] } else { &[false] };
                let mut selected = None;
                'passes: for &prefer_visible in passes {
                    for attempt in 0..self.placements.len() {
                        let (_, lane_id, arc0) = self.placements[(placed * 7 + attempt) % self.placements.len()];
                        let lane = &net.lanes[lane_id as usize];
                        let jitter = ((placed % 5) as f64 - 2.0) * 5.5;
                        let arc = js_clamp(arc0 + jitter, 2.0, (lane.length - 2.0).max(2.0));
                        let (x, z, _, _, _) = geom::at_arc(&lane.pts, &lane.cum, arc);
                        if self.claim.is_some_and(|c| !c.contains(x, z)) {
                            continue;
                        }
                        let visible_after = seen(&vs, x, z);
                        if visible_after && vs.iter().any(|u| geom::hypot(x - u.x, z - u.z) < POP_IN_GUARD_METRES) {
                            continue;
                        }
                        if prefer_visible && !visible_after {
                            continue;
                        }
                        if !(allow_visible || !visible_after) {
                            continue;
                        }
                        if self.fleet.insert_blocked(net, lane_id, arc, car) || self.fleet.landing_unfit(net, lane_id, arc, car, 4.5) {
                            continue;
                        }
                        selected = Some((lane_id, arc));
                        break 'passes;
                    }
                }
                match selected {
                    Some((lane, arc)) => {
                        let speed = 2.0 + self.car_rng.next_f64() * 2.5;
                        self.fleet.place(net, car, lane, arc, speed);
                        if grown {
                            self.stats.cars_grown += 1;
                        } else {
                            self.stats.car_recycles += 1;
                        }
                        placed += 1;
                    }
                    None if grown => {
                        // Nowhere fit for it: it never existed.
                        let _ = self.fleet.take(car);
                        self.fleet.next_id -= 1;
                        break;
                    }
                    None => {}
                }
            }
        }
        self.view_scratch = vs;
    }

    /// the crowd disc of a shared region: a walker is kept while it is
    /// within the keep radius of any viewer; each viewer's disc is refilled to
    /// the People setting's bodies, scaled by the share of the disc inside the
    /// claim, with every viewer's pop-in guards (unless the region is cold).
    fn shared_bubble(&mut self, net: &Network, dt: f64, bubble: CrowdBubble) {
        let mut vs = std::mem::take(&mut self.view_scratch);
        vs.clear();
        vs.extend_from_slice(self.viewers.as_deref().unwrap_or(&[]));
        let keep2 = bubble.keep_radius_m * bubble.keep_radius_m;
        let mut i = self.crowd.walkers.len();
        while i > 0 {
            i -= 1;
            let (x, z) = (self.crowd.x[i], self.crowd.z[i]);
            if vs.iter().all(|f| (x - f.x) * (x - f.x) + (z - f.z) * (z - f.z) > keep2) {
                self.crowd.remove(i);
            }
        }
        self.scan_timer += dt;
        if self.scan_timer < SCAN_INTERVAL_S || vs.is_empty() {
            self.view_scratch = vs;
            return;
        }
        self.scan_timer = 0.0;
        let cos_cone = IN_SHOT_HALF_ANGLE_RAD.cos();
        let r2 = bubble.radius_m * bubble.radius_m;
        for f in vs.iter() {
            let focus = [f.x, f.z];
            let share = self.claim.map_or(1.0, |c| c.disc_share(f.x, f.z, bubble.radius_m));
            let want = (bubble.max_bodies as f64 * share).round() as usize;
            let count = (0..self.crowd.walkers.len()).filter(|&k| (self.crowd.x[k] - f.x).powi(2) + (self.crowd.z[k] - f.z).powi(2) <= r2).count();
            let deficit = want.saturating_sub(count);
            if deficit == 0 {
                continue;
            }
            self.foot_scratch.clear();
            let mut running = 0.0;
            for t in net.tiles_near(focus[0], focus[1], bubble.radius_m) {
                for &fw_id in &t.footways {
                    let fw = &net.footways[fw_id as usize];
                    if !fw.alive || fw.length < 4.0 || geom::dist_to_polyline(&fw.pts, focus[0], focus[1]) > bubble.radius_m {
                        continue;
                    }
                    running += fw.weight;
                    self.foot_scratch.push((fw_id, running));
                }
            }
            if self.foot_scratch.is_empty() {
                continue;
            }
            let mut spawned = 0;
            let mut attempts = 0;
            while spawned < deficit && attempts < deficit * 4 {
                attempts += 1;
                let t = self.walker_rng.next_f64() * running;
                let k = self.foot_scratch.partition_point(|e| e.1 < t).min(self.foot_scratch.len() - 1);
                let fw_id = self.foot_scratch[k].0;
                let fw = &net.footways[fw_id as usize];
                let arc = self.walker_rng.next_f64() * fw.length;
                let (x, z, _, _, _) = geom::at_arc(&fw.pts, &fw.cum, arc);
                if geom::hypot(x - focus[0], z - focus[1]) > bubble.radius_m || self.claim.is_some_and(|c| !c.contains(x, z)) {
                    continue;
                }
                if !self.cold_seed {
                    let guarded = vs.iter().any(|u| {
                        let (dx, dz) = (x - u.x, z - u.z);
                        let d = geom::hypot(dx, dz);
                        d < bubble.pop_in_guard_m || (u.yaw.is_finite() && d < bubble.in_shot_guard_m && d > 1e-3 && (dx * u.yaw.sin() + dz * u.yaw.cos()) / d >= cos_cone)
                    });
                    if guarded {
                        continue;
                    }
                }
                let before = self.crowd.walkers.len();
                spawned += self.spawn_group(net, fw_id, Some(arc));
                for w in before..self.crowd.walkers.len() {
                    let (x, z) = {
                        let wk = &self.crowd.walkers[w];
                        let fw = &net.footways[wk.footway as usize];
                        let (x, z, _, _, _) = geom::at_arc(&fw.pts, &fw.cum, wk.s);
                        (x, z)
                    };
                    self.crowd.x[w] = x;
                    self.crowd.z[w] = z;
                }
            }
        }
        self.cold_seed = false;
        self.view_scratch = vs;
    }

    /// Tiles streamed in or out: re-decide claims and exits, and retire walkers
    /// whose pavement (or the pavement their leg leads to) has left.
    pub fn network_changed(&mut self, net: &Network) {
        // A car on a lane that left is set down elsewhere now, before its slot can be reused.
        let invisible = |_: f64, _: f64| false;
        let homing_focus = self.cfg.local_life.then_some([self.focus.x, self.focus.z]);
        let ctx = StepCtx { net, time: 0.0, seed_int: self.cfg.seed_int, homing_focus, visible: &invisible };
        for i in 0..self.fleet.cars.len() {
            let c = &self.fleet.cars[i];
            let gone = !net.lanes[c.lane as usize].alive || (c.cornering() && !net.lanes[c.corner_from as usize].alive);
            if gone {
                self.fleet.cars[i].corner_from = crate::network::NONE;
                if !self.fleet.respawn(&ctx, i, &mut self.car_rng) {
                    // Nowhere to go yet: park it on any live lane's start; the recycler moves it on.
                    if let Some(l) = net.alive_lanes().next() {
                        self.fleet.place(net, i, l, 0.0, 0.0);
                    }
                }
            }
        }
        self.fleet.network_changed(net);
        let mut i = self.crowd.walkers.len();
        while i > 0 {
            i -= 1;
            let w = &self.crowd.walkers[i];
            let dead = !net.footways[w.footway as usize].alive
                || w.leg.is_some_and(|l| !net.footways[l.next as usize].alive)
                || w.kerb.is_some_and(|k| !net.footways[k.next as usize].alive);
            if dead {
                self.crowd.remove(i);
            }
        }
    }

    fn update_lod(&mut self) {
        if !self.cfg.lod {
            return;
        }
        if let Some(vs) = &self.viewers {
            // a shared region: the nearest viewer decides.
            let near = |x: f64, z: f64| vs.iter().map(|f| geom::hypot(x - f.x, z - f.z)).fold(f64::INFINITY, f64::min);
            for (i, w) in self.crowd.walkers.iter_mut().enumerate() {
                let d = near(self.crowd.x[i], self.crowd.z[i]);
                w.far = if w.far { d > SIM_TIER_FAR_REENTER_M } else { d > SIM_TIER_FAR_RADIUS_M };
            }
            let horizon = self.cfg.draw_horizon;
            for (i, c) in self.fleet.cars.iter_mut().enumerate() {
                let d = near(self.fleet.x[i], self.fleet.z[i]);
                c.far = if c.far { d > horizon - 10.0 } else { d > horizon };
            }
            return;
        }
        let f = [self.focus.x, self.focus.z];
        for (i, w) in self.crowd.walkers.iter_mut().enumerate() {
            let d = geom::hypot(self.crowd.x[i] - f[0], self.crowd.z[i] - f[1]);
            w.far = if w.far { d > SIM_TIER_FAR_REENTER_M } else { d > SIM_TIER_FAR_RADIUS_M };
        }
        let horizon = self.cfg.draw_horizon;
        for (i, c) in self.fleet.cars.iter_mut().enumerate() {
            let d = geom::hypot(self.fleet.x[i] - f[0], self.fleet.z[i] - f[1]);
            c.far = if c.far { d > horizon - 10.0 } else { d > horizon };
        }
    }

    /// One fixed step at absolute simulation time `time` (the shared clock, after advancing).
    pub fn step(&mut self, net: &Network, time: f64, dt: f64) {
        self.step_index += 1;
        let hour = self.cfg.hour + time / 3600.0;
        if self.viewers.is_some() {
            // a shared region.
            self.shared_life(net, dt, hour);
            if let People::Bubble(b) = self.cfg.people {
                self.shared_bubble(net, dt, b);
            }
        } else {
            self.local_life(net, dt, hour);
            if let Some(d) = self.cfg.crowd {
                self.density_bubble(net, dt, d, hour); // `--crowd`
            } else if let People::Bubble(b) = self.cfg.people {
                self.bubble(net, dt, b);
            }
        }
        self.update_lod();
        let parity = self.step_index;
        // SENSE: people index and occupied crossings, then every car's gaps.
        self.walker_grid.build(&self.crowd.x, &self.crowd.z, PERSON_CELL_M, |_| true);
        self.crowd.bands(&mut self.bands);
        self.fleet.sense(net, &self.crowd.x, &self.crowd.z, &self.walker_grid, &self.bands);
        // ACT: cars in id order, then walkers.
        if let Some(vs) = &self.viewers {
            // a shared region: seen by any viewer, homing on the first.
            let (horizon, half) = (self.cfg.draw_horizon, self.cfg.view_half_angle);
            let visible = |x: f64, z: f64| vs.iter().any(|f| visible_from(f, horizon, half, x, z));
            let homing_focus = if self.cfg.local_life { vs.first().map(|f| [f.x, f.z]) } else { None };
            let ctx = StepCtx { net, time, seed_int: self.cfg.seed_int, homing_focus, visible: &visible };
            self.fleet.act(&ctx, dt, &mut self.car_rng, parity);
        } else {
            let focus = self.local_focus.or(if self.cfg.local_life { Some([self.focus.x, self.focus.z]) } else { None });
            let fcopy = self.focus;
            let horizon = self.cfg.draw_horizon;
            let half = self.cfg.view_half_angle;
            let visible = move |x: f64, z: f64| -> bool {
                let (dx, dz) = (x - fcopy.x, z - fcopy.z);
                let d = geom::hypot(dx, dz);
                if d > horizon + 14.0 {
                    return false;
                }
                if !fcopy.yaw.is_finite() || d < 1e-3 {
                    return true;
                }
                (dx * fcopy.yaw.sin() + dz * fcopy.yaw.cos()) / d >= half.cos()
            };
            let ctx = StepCtx { net, time, seed_int: self.cfg.seed_int, homing_focus: focus, visible: &visible };
            self.fleet.act(&ctx, dt, &mut self.car_rng, parity);
        }
        self.fleet.settle_claims(net);
        {
            let wctx = WalkCtx {
                net,
                time,
                hour,
                car_x: &self.fleet.x,
                car_z: &self.fleet.z,
                car_yaw: &self.fleet.yaw,
                car_speed: &self.fleet.spd,
                car_level: &self.fleet.level,
            };
            self.crowd.step(&wctx, dt, parity);
        }
        self.write_output(net, time);
    }

    /// Fill `out` with this step's instance records.
    fn write_output(&mut self, net: &Network, time: f64) {
        let origin = TileKey::of(self.focus.x, self.focus.z).origin();
        let out = &mut self.out;
        out.origin = origin;
        out.time = time;
        out.step = self.step_index;
        out.vehicles.clear();
        for (i, c) in self.fleet.cars.iter().enumerate() {
            let mut flags = 0;
            if c.braking {
                flags |= FLAG_BRAKING;
            }
            if c.speed <= 0.02 {
                flags |= FLAG_STOPPED;
            }
            if c.turn_side > 0 {
                flags |= FLAG_TURN_LEFT;
            } else if c.turn_side < 0 {
                flags |= FLAG_TURN_RIGHT;
            }
            if c.cornering() {
                flags |= FLAG_CORNERING;
            }
            if c.far {
                flags |= FLAG_FAR;
            }
            // (its structure level, 0 at grade: no bits without a level table.)
            flags |= crate::output::level_flags(self.fleet.level[i]);
            out.vehicles.push(AgentInstance {
                pos: [(self.fleet.x[i] - origin[0]) as f32, 0.0, (self.fleet.z[i] - origin[1]) as f32],
                heading: self.fleet.yaw[i] as f32,
                speed: c.speed as f32,
                anim: (c.travelled % 10_000.0) as f32,
                kind: c.kind as u16,
                flags,
                id: c.id,
            });
        }
        out.walkers.clear();
        for (i, w) in self.crowd.walkers.iter().enumerate() {
            let mut flags = 0;
            if w.kerb.is_some() {
                flags |= FLAG_KERB_WAIT;
            }
            if w.leg.is_some_and(|l| l.kind == LegKind::Cross) {
                flags |= FLAG_CROSSING;
            }
            if w.ground_speed <= 0.0 {
                flags |= FLAG_STOPPED;
            }
            if w.far {
                flags |= FLAG_FAR;
            }
            out.walkers.push(AgentInstance {
                pos: [(self.crowd.x[i] - origin[0]) as f32, 0.0, (self.crowd.z[i] - origin[1]) as f32],
                heading: w.yaw as f32,
                speed: w.ground_speed as f32,
                anim: (w.gait % std::f64::consts::TAU) as f32,
                kind: KIND_WALKER_BASE + w.activity as u16,
                flags,
                id: w.key,
            });
        }
        let _ = net;
    }
}
