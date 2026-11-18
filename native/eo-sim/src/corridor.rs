//! Corridor flow: the through traffic of the long roads that run out of the
//! local simulation's window (the Golden Gate Bridge and its approaches, the
//! state routes through the Presidio, Marina Boulevard), and the walkers on
//! the bridge's walk, stepped on the world's one clock (`World::step`).
//!
//! The local simulation (`region`) moves 42 cars about a focus inside its
//! 240 m window; nothing it does reaches the far deck of a bridge seen from a
//! headland. This layer is separate from that pool and much simpler:
//!
//!   * each corridor way is a 3-D centreline in travel order with parallel
//!     lanes at fixed offsets (the caller lays them on the drawn deck or road);
//!   * vehicles arrive at the way's start at the way's hourly flow (seeded
//!     exponential gaps, so a run replays), follow their leader by the
//!     intelligent driver model (Treiber, Hennecke and Helbing 2000, Phys. Rev.
//!     E 62, 1805: `a [1 - (v/v0)^4 - (s*/s)^2]`, `s* = s0 + v T + v dv / 2
//!     sqrt(a b)`) with the driving model's own acceleration, comfortable
//!     deceleration and bumper gap (`consts::ACCEL`, `FOLLOW_DECEL`,
//!     `MIN_BUMPER_GAP`) and the time gap the standards table gives, change
//!     lane when the next lane lets them go faster and its follower keeps a
//!     safe gap (the incentive and safety tests of MOBIL, Kesting, Treiber and
//!     Helbing 2007, Transp. Res. Rec. 1999, 86-94), and leave at its end;
//!   * at a change of hour (or at the start) every lane is filled at the
//!     hour's equilibrium spacing, so a still taken a second after a jump has
//!     the road's traffic on it and not a front advancing from the start.
//!
//! Output: one `CorridorInstance` per vehicle or walker, in world metres
//! with its height (the caller's centreline carries it), heading and pitch.
//! Ways marked `handover` (streets the local simulation also drives) leave
//! out the vehicles inside the local window round the focus, so the two
//! layers never draw two cars on one lane; ways the local simulation does
//! not drive (the caller removes them from its network) draw everywhere.

use crate::consts::{ACCEL, DRIVER_PACE_MIN, DRIVER_PACE_SPAN, FOLLOW_DECEL, LOCAL_RECYCLE_RADIUS, MIN_BUMPER_GAP, PED_CRUISE_MAX, PED_CRUISE_MIN, PRESENTED_LENGTH, TRAFFIC_KIND_MIX};
use crate::output::{CorridorInstance, FLAG_BRAKING, KIND_WALKER_BASE};
use crate::pedestrians::cadence_for_speed;
use crate::rng::Mulberry32;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// What travels a way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Travel {
    Vehicles,
    Walkers,
}

/// One directed way: its centreline in travel order, its lanes, its speed and its flow.
#[derive(Clone, Debug)]
pub struct WaySpec {
    pub name: String,
    /// Centreline (x, y, z), world metres, in travel order.
    pub points: Vec<[f64; 3]>,
    /// Lane centres, metres to the right of the centreline (facing travel).
    pub lanes: Vec<f64>,
    /// Desired speed before the driver's pace: the posted limit (survey), or
    /// a walker's cruise (the crowd's draw).
    pub speed_mps: f64,
    /// Arrivals per hour, hour 0..23 (local time): the hour's centre is h + 0.5.
    pub flow_vph: [f64; 24],
    /// Vehicle kinds (`TRAFFIC_KIND_MIX` order: sedan, taxi, truck, bus).
    pub mix: [f64; 4],
    pub travel: Travel,
    /// Per point: the stretch from it to the next draws nothing (a tunnel).
    pub hidden: Vec<bool>,
    /// The local simulation also drives this way: inside its window the local cars are drawn instead.
    pub handover: bool,
}

/// The car-following parameters.
#[derive(Clone, Copy, Debug)]
pub struct Params {
    /// IDM desired time gap, s (the standards table's following time gap).
    pub time_gap_s: f64,
    pub accel: f64,
    pub decel: f64,
    pub min_gap_m: f64,
    /// MOBIL's incentive threshold and safe braking, m/s2.
    pub change_threshold: f64,
    pub safe_decel: f64,
    /// Seconds a lane change takes across one lane.
    pub change_s: f64,
}

impl Params {
    /// The driving model's own accelerations and gap with the table's time gap.
    /// The lane change's incentive (0.2 m/s2), safe braking (4 m/s2) and duration
    /// (the MOBIL paper's defaults: threshold 0.1-0.2, b_safe 4) are INFERRED;
    /// the 3 s a change takes is INFERRED from a lateral move of one lane at
    /// about 1.2 m/s.
    pub fn with_time_gap(time_gap_s: f64) -> Self {
        Params { time_gap_s, accel: ACCEL, decel: FOLLOW_DECEL, min_gap_m: MIN_BUMPER_GAP, change_threshold: 0.2, safe_decel: 4.0, change_s: 3.0 }
    }
}

#[derive(Clone, Copy, Debug)]
struct Mover {
    id: u32,
    s: f64,
    v: f64,
    v0: f64,
    len: f64,
    lane: u16,
    /// Lateral offset now and where a change started from, and its progress 0..1 (1: settled).
    lat_from: f64,
    change_t: f64,
    kind: u16,
    travelled: f64,
    gait: f64,
    braking: bool,
    change_clock: f64,
}

struct Way {
    spec: WaySpec,
    cum: Vec<f64>,
    /// Mitred right normals per point (x, z).
    normal: Vec<[f64; 2]>,
    len: f64,
    movers: Vec<Mover>,
    next_arrival_s: f64,
}

/// The corridor layer.
pub struct Corridors {
    ways: Vec<Way>,
    params: Params,
    seed: u32,
    rng: Mulberry32,
    next_id: u32,
    /// The hour the lanes were last filled at (NaN: never).
    filled_hour: f64,
    /// The hour to run at, written by the caller from any thread (f64 bits).
    hour: Arc<AtomicU64>,
    pub out: Vec<CorridorInstance>,
    time: f64,
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

impl Way {
    fn new(spec: WaySpec) -> Option<Self> {
        if spec.points.len() < 2 || spec.lanes.is_empty() {
            return None;
        }
        let mut cum = vec![0.0];
        for w in spec.points.windows(2) {
            let d = (w[1][0] - w[0][0]).hypot(w[1][2] - w[0][2]);
            cum.push(cum.last().unwrap() + d);
        }
        let len = *cum.last().unwrap();
        if len < 1.0 {
            return None;
        }
        let n = spec.points.len();
        let seg_normal = |i: usize| {
            let (a, b) = (spec.points[i], spec.points[i + 1]);
            let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
            let l = dx.hypot(dz).max(1e-9);
            // right of travel: (-dz, dx) with x east and z south.
            [-dz / l, dx / l]
        };
        let mut normal = Vec::with_capacity(n);
        for i in 0..n {
            let nn = if i == 0 {
                seg_normal(0)
            } else if i == n - 1 {
                seg_normal(n - 2)
            } else {
                let (p, q) = (seg_normal(i - 1), seg_normal(i));
                let m = [p[0] + q[0], p[1] + q[1]];
                let l = m[0].hypot(m[1]);
                if l < 1e-6 {
                    q
                } else {
                    // mitre: the offset keeps its width through the bend (capped at 2x).
                    let cos_half = ((m[0] * q[0] + m[1] * q[1]) / l).max(0.5);
                    [m[0] / l / cos_half, m[1] / l / cos_half]
                }
            };
            normal.push(nn);
        }
        let mut hidden = spec.hidden.clone();
        hidden.resize(n, false);
        let spec = WaySpec { hidden, ..spec };
        Some(Way { spec, cum, normal, len, movers: vec![], next_arrival_s: 0.0 })
    }

    /// (segment, t) at arc `s`.
    fn locate(&self, s: f64) -> (usize, f64) {
        let s = s.clamp(0.0, self.len);
        let i = match self.cum.binary_search_by(|c| c.total_cmp(&s)) {
            Ok(i) => i.min(self.cum.len() - 2),
            Err(i) => i.saturating_sub(1).min(self.cum.len() - 2),
        };
        let span = (self.cum[i + 1] - self.cum[i]).max(1e-9);
        (i, ((s - self.cum[i]) / span).clamp(0.0, 1.0))
    }

    /// The point `lat` metres right of the centreline at arc `s`, its heading and pitch.
    fn pose(&self, s: f64, lat: f64) -> ([f64; 3], f64, f64, bool) {
        let (i, t) = self.locate(s);
        let (a, b) = (self.spec.points[i], self.spec.points[i + 1]);
        let (na, nb) = (self.normal[i], self.normal[i + 1]);
        let n = [lerp(na[0], nb[0], t), lerp(na[1], nb[1], t)];
        let p = [lerp(a[0], b[0], t) + n[0] * lat, lerp(a[1], b[1], t), lerp(a[2], b[2], t) + n[1] * lat];
        let (dx, dy, dz) = (b[0] - a[0], b[1] - a[1], b[2] - a[2]);
        let heading = dx.atan2(dz);
        let pitch = dy.atan2(dx.hypot(dz).max(1e-6));
        (p, heading, pitch, self.spec.hidden[i])
    }

    fn flow_at(&self, hour: f64) -> f64 {
        // hourly values at the hours' centres, linear between them.
        let h = (hour - 0.5).rem_euclid(24.0);
        let (i, t) = (h.floor() as usize % 24, h - h.floor());
        lerp(self.spec.flow_vph[i], self.spec.flow_vph[(i + 1) % 24], t).max(0.0)
    }
}

/// The IDM acceleration of a follower at speed `v` (desired `v0`) behind a gap `gap` closing at `dv`.
pub fn idm_accel(p: &Params, v: f64, v0: f64, gap: Option<(f64, f64)>) -> f64 {
    let free = 1.0 - (v / v0.max(0.1)).powi(4);
    let inter = match gap {
        Some((g, dv)) => {
            let s_star = p.min_gap_m + (v * p.time_gap_s + v * dv / (2.0 * (p.accel * p.decel).sqrt())).max(0.0);
            (s_star / g.max(0.1)).powi(2)
        }
        None => 0.0,
    };
    p.accel * (free - inter)
}

/// The equilibrium bumper gap at speed `v` (IDM, free term ignored).
fn equilibrium_gap(p: &Params, v: f64) -> f64 {
    p.min_gap_m + v * p.time_gap_s
}

impl Corridors {
    pub fn new(specs: Vec<WaySpec>, params: Params, seed: u32) -> Self {
        let ways = specs.into_iter().filter_map(Way::new).collect();
        Corridors { ways, params, seed, rng: Mulberry32::new(seed.wrapping_add(0x5eed)), next_id: 1, filled_hour: f64::NAN, hour: Arc::new(AtomicU64::new(12f64.to_bits())), out: vec![], time: 0.0 }
    }

    /// The handle the caller writes the hour through (local time, hours).
    pub fn hour_handle(&self) -> Arc<AtomicU64> {
        self.hour.clone()
    }

    pub fn ways(&self) -> usize {
        self.ways.len()
    }

    pub fn movers(&self) -> usize {
        self.ways.iter().map(|w| w.movers.len()).sum()
    }

    fn hour(&self) -> f64 {
        f64::from_bits(self.hour.load(Ordering::Relaxed))
    }

    fn new_mover(&mut self, travel: Travel, mix: &[f64; 4], speed: f64, lane: u16, lat: f64, s: f64) -> Mover {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let (kind, len, v0) = match travel {
            Travel::Vehicles => {
                let total: f64 = mix.iter().sum::<f64>().max(1e-9);
                let mut d = self.rng.next_f64() * total;
                let mut kind = 0u16;
                for (k, m) in mix.iter().enumerate() {
                    if d < *m {
                        kind = k as u16;
                        break;
                    }
                    d -= m;
                }
                let pace = DRIVER_PACE_MIN + DRIVER_PACE_SPAN * self.rng.next_f64();
                (kind, PRESENTED_LENGTH[kind as usize] as f64, speed * pace)
            }
            Travel::Walkers => {
                // the crowd's cruise range (`PED_CRUISE_MIN`..`MAX`).
                let v = PED_CRUISE_MIN + (PED_CRUISE_MAX - PED_CRUISE_MIN) * self.rng.next_f64();
                (KIND_WALKER_BASE, 0.6, v)
            }
        };
        let gait = self.rng.next_f64() * std::f64::consts::TAU;
        Mover { id, s, v: v0, v0, len, lane, lat_from: lat, change_t: 1.0, kind, travelled: self.rng.next_f64() * 1000.0, gait, braking: false, change_clock: self.rng.next_f64() }
    }

    /// Fill every lane at the hour's equilibrium spacing (seeded by the hour, so a still replays).
    fn fill(&mut self, hour: f64) {
        self.rng = Mulberry32::new(self.seed.wrapping_add((hour * 60.0).round() as u32).wrapping_add(0x5eed));
        let p = self.params;
        for wi in 0..self.ways.len() {
            let (flow, lanes, len, speed, travel, mix) = {
                let w = &self.ways[wi];
                (w.flow_at(hour), w.spec.lanes.clone(), w.len, w.spec.speed_mps, w.spec.travel, w.spec.mix)
            };
            let mut movers = vec![];
            let per_lane = flow / lanes.len() as f64; // vehicles per hour per lane
            if per_lane > 0.0 {
                for (k, &lat) in lanes.iter().enumerate() {
                    // mean headway per lane, s; at least the following gap at the desired speed.
                    let mean_h = 3600.0 / per_lane;
                    let mut s = self.rng.next_f64() * mean_h * speed;
                    while s < len {
                        let m = self.new_mover(travel, &mix, speed, k as u16, lat, s);
                        let min_space = m.len + equilibrium_gap(&p, m.v0) * if travel == Travel::Walkers { 0.3 } else { 1.0 };
                        movers.push(m);
                        let h = -(1.0 - self.rng.next_f64()).ln() * mean_h;
                        s += (h * m.v0).max(min_space);
                    }
                }
            }
            let w = &mut self.ways[wi];
            w.movers = movers;
            w.next_arrival_s = self.time;
        }
        self.filled_hour = hour;
    }

    /// One fixed step. `focus`: the local simulation's focus (x, z), for the hand-over.
    pub fn step(&mut self, dt: f64, focus: Option<[f64; 2]>) {
        self.time += dt;
        let hour = self.hour();
        if !(self.filled_hour - hour).abs().is_finite() || (self.filled_hour - hour).abs() > 0.05 {
            self.fill(hour);
        }
        let p = self.params;
        for wi in 0..self.ways.len() {
            // arrivals at the start, exponential gaps at the hour's flow.
            let flow = self.ways[wi].flow_at(hour);
            if flow > 0.0 {
                while self.ways[wi].next_arrival_s <= self.time {
                    let (lanes, speed, travel, mix) = {
                        let w = &self.ways[wi];
                        (w.spec.lanes.clone(), w.spec.speed_mps, w.spec.travel, w.spec.mix)
                    };
                    let k = ((self.rng.next_f64() * lanes.len() as f64) as usize).min(lanes.len() - 1);
                    let m = self.new_mover(travel, &mix, speed, k as u16, lanes[k], 0.0);
                    let w = &mut self.ways[wi];
                    // the lane's last mover must have cleared the entry.
                    let last = w.movers.iter().filter(|o| o.lane == k as u16).map(|o| o.s - o.len).fold(f64::INFINITY, f64::min);
                    let need = m.len + if travel == Travel::Walkers { 0.5 } else { equilibrium_gap(&p, m.v0) * 0.5 };
                    if last >= need {
                        let mut m = m;
                        if let Some(lead) = w.movers.iter().filter(|o| o.lane == k as u16).min_by(|a, b| a.s.total_cmp(&b.s)) {
                            m.v = m.v.min(lead.v);
                        }
                        w.movers.push(m);
                    }
                    let gap = -(1.0 - self.rng.next_f64()).ln() * 3600.0 / flow;
                    self.ways[wi].next_arrival_s += gap.max(0.05);
                }
            } else {
                self.ways[wi].next_arrival_s = self.time;
            }
            let w = &mut self.ways[wi];
            let nl = w.spec.lanes.len();
            // leaders: per lane, sorted by arc.
            w.movers.sort_by(|a, b| a.lane.cmp(&b.lane).then(b.s.total_cmp(&a.s)));
            let n = w.movers.len();
            let mut acc = vec![0.0; n];
            for i in 0..n {
                let m = w.movers[i];
                let lead = if i > 0 && w.movers[i - 1].lane == m.lane { Some(&w.movers[i - 1]) } else { None };
                let gap = lead.map(|l| (l.s - l.len * 0.5 - (m.s + m.len * 0.5), m.v - l.v));
                acc[i] = if w.spec.travel == Travel::Walkers {
                    // walkers keep their cruise and slow behind a slower one.
                    match gap {
                        Some((g, _)) if g < 1.5 => -1.0,
                        _ => (m.v0 - m.v).clamp(-1.0, 1.0),
                    }
                } else {
                    idm_accel(&p, m.v, m.v0, gap)
                };
            }
            // lane changes (vehicles), each mover deciding about once a second.
            if w.spec.travel == Travel::Vehicles && nl > 1 {
                for i in 0..n {
                    let m = w.movers[i];
                    let clock = m.change_clock + dt;
                    w.movers[i].change_clock = clock.fract();
                    if clock < 1.0 || m.change_t < 1.0 {
                        continue;
                    }
                    let mut best: Option<(u16, f64)> = None;
                    for dl in [-1i32, 1] {
                        let to = m.lane as i32 + dl;
                        if to < 0 || to >= nl as i32 {
                            continue;
                        }
                        let to = to as u16;
                        let ahead = w.movers.iter().filter(|o| o.lane == to && o.s > m.s).min_by(|a, b| a.s.total_cmp(&b.s));
                        let behind = w.movers.iter().filter(|o| o.lane == to && o.s <= m.s).max_by(|a, b| a.s.total_cmp(&b.s));
                        let gap_a = ahead.map(|l| (l.s - l.len * 0.5 - (m.s + m.len * 0.5), m.v - l.v));
                        if gap_a.is_some_and(|(g, _)| g < p.min_gap_m) {
                            continue;
                        }
                        if let Some(f) = behind {
                            let g = m.s - m.len * 0.5 - (f.s + f.len * 0.5);
                            if g < p.min_gap_m || idm_accel(&p, f.v, f.v0, Some((g, f.v - m.v))) < -p.safe_decel {
                                continue;
                            }
                        }
                        let gain = idm_accel(&p, m.v, m.v0, gap_a) - acc[i];
                        // keep right: a small bias toward the outer lane (CVC 21650 / 21654, slower traffic right).
                        let bias = if (dl > 0) == (w.spec.lanes[to as usize] > w.spec.lanes[m.lane as usize]) { 0.05 } else { 0.0 };
                        if gain + bias > p.change_threshold && best.is_none_or(|(_, g)| gain > g) {
                            best = Some((to, gain));
                        }
                    }
                    if let Some((to, _)) = best {
                        let lat_now = lerp(m.lat_from, w.spec.lanes[m.lane as usize], m.change_t);
                        let mm = &mut w.movers[i];
                        mm.lat_from = lat_now;
                        mm.lane = to;
                        mm.change_t = 0.0;
                    }
                }
            }
            for i in 0..n {
                let m = &mut w.movers[i];
                let a = acc[i].max(-2.0 * p.decel);
                let v = (m.v + a * dt).max(0.0);
                let ds = (m.v + v) * 0.5 * dt;
                m.braking = a < -0.5;
                m.v = v;
                m.s += ds;
                m.travelled += ds;
                if m.kind >= KIND_WALKER_BASE {
                    m.gait += std::f64::consts::PI * cadence_for_speed(m.v) * dt;
                }
                if m.change_t < 1.0 {
                    m.change_t = (m.change_t + dt / p.change_s).min(1.0);
                }
            }
            let len = w.len;
            w.movers.retain(|m| m.s < len);
        }
        self.write_out(focus);
    }

    fn write_out(&mut self, focus: Option<[f64; 2]>) {
        self.out.clear();
        for w in &self.ways {
            for m in &w.movers {
                let to = w.spec.lanes[m.lane as usize];
                // smoothstep across the lane change.
                let t = m.change_t * m.change_t * (3.0 - 2.0 * m.change_t);
                let lat = lerp(m.lat_from, to, t);
                let (p, heading, pitch, hidden) = w.pose(m.s, lat);
                if hidden {
                    continue;
                }
                if w.spec.handover {
                    if let Some(f) = focus {
                        if (p[0] - f[0]).hypot(p[2] - f[1]) < LOCAL_RECYCLE_RADIUS {
                            continue;
                        }
                    }
                }
                // turning into the change shows as a small yaw.
                let yaw_change = if m.change_t < 1.0 && m.v > 1.0 { ((to - m.lat_from) / (m.v * self.params.change_s)).atan() * 1.5 * (std::f64::consts::PI * m.change_t).sin() } else { 0.0 };
                let walker = m.kind >= KIND_WALKER_BASE;
                self.out.push(
                    CorridorInstance {
                        pos: [p[0] as f32, p[1] as f32, p[2] as f32],
                        heading: (heading - yaw_change) as f32,
                        pitch: pitch as f32,
                        speed: m.v as f32,
                        anim: if walker { (m.gait % std::f64::consts::TAU) as f32 } else { (m.travelled % 10_000.0) as f32 },
                        kind: m.kind,
                        flags: if m.braking { FLAG_BRAKING } else { 0 },
                        id: m.id,
                    },
                );
            }
        }
    }
}

/// The vehicle mix of a corridor: heavy vehicles at the counted share (the
/// count's 2-axle trucks drawn as buses, 3 or more axles as trucks; INFERRED:
/// the count station's 2-axle class on this bridge is mostly the transit
/// district's coaches), the rest as the driving model's mix of cars and taxis.
pub fn mix_with_heavy(two_axle_share: f64, multi_axle_share: f64) -> [f64; 4] {
    let cars = (1.0 - two_axle_share - multi_axle_share).max(0.0);
    let (sedan, taxi) = (TRAFFIC_KIND_MIX[0], TRAFFIC_KIND_MIX[1]);
    let light = sedan + taxi;
    [cars * sedan / light, cars * taxi / light, multi_axle_share, two_axle_share]
}

/// An hourly flow profile for one direction from a count's 24-hour volume and
/// its AM and PM peak-hour volumes.
///
/// Shape (INFERRED, an urban weekday commute profile, not a count): the AM
/// peak at 07-09 h, the PM peak at 16-18 h, the night (22-05 h) at a tenth of
/// the larger peak, linear shoulders 05-07 h and 18-22 h, and a midday plateau
/// (09-16 h) whose level is solved so the day sums to the 24-hour volume
/// (clamped between the night level and the smaller peak).
pub fn hourly_profile(day_volume: f64, am_peak: f64, pm_peak: f64) -> [f64; 24] {
    let night = 0.1 * am_peak.max(pm_peak);
    let mut q = [0.0f64; 24];
    let build = |plateau: f64, q: &mut [f64; 24]| {
        for (h, v) in q.iter_mut().enumerate() {
            *v = match h {
                0..=4 | 22..=23 => night,
                5 => lerp(night, am_peak, 1.0 / 3.0),
                6 => lerp(night, am_peak, 2.0 / 3.0),
                7 | 8 => am_peak,
                9..=15 => plateau,
                16 | 17 => pm_peak,
                18 => lerp(pm_peak, night, 0.25),
                19 => lerp(pm_peak, night, 0.5),
                20 => lerp(pm_peak, night, 0.75),
                _ => night,
            };
        }
    };
    build(0.0, &mut q);
    let rest: f64 = q.iter().sum();
    let plateau = ((day_volume - rest) / 7.0).clamp(night, am_peak.min(pm_peak).max(night));
    build(plateau, &mut q);
    q
}

#[cfg(test)]
mod tests {
    use super::*;

    fn straight(len: f64, lanes: usize, flow: f64) -> WaySpec {
        WaySpec {
            name: "test".into(),
            points: vec![[0.0, 10.0, 0.0], [0.0, 10.0, -len]],
            lanes: (0..lanes).map(|k| 1.8 + 3.6 * k as f64).collect(),
            speed_mps: 20.0,
            flow_vph: [flow; 24],
            mix: [0.9, 0.02, 0.06, 0.02],
            travel: Travel::Vehicles,
            hidden: vec![],
            handover: false,
        }
    }

    #[test]
    fn a_way_fills_at_its_flow_and_keeps_it() {
        let mut c = Corridors::new(vec![straight(3000.0, 3, 3000.0)], Params::with_time_gap(3.0), 1);
        c.step(1.0 / 60.0, None);
        let filled = c.movers();
        // 1000 vph per lane at about 20 m/s, each car at least its 3 s gap
        // behind the next (about 1060 vph a lane at most): 30-50 a lane in 3 km.
        assert!((80..=160).contains(&filled), "filled {filled}");
        for _ in 0..(60 * 120) {
            c.step(1.0 / 60.0, None);
        }
        let later = c.movers();
        assert!((later as f64 - filled as f64).abs() < 0.35 * filled as f64, "filled {filled}, later {later}");
        // nobody overlaps its leader.
        for w in &c.ways {
            for a in &w.movers {
                for b in &w.movers {
                    if a.id != b.id && a.lane == b.lane && a.change_t >= 1.0 && b.change_t >= 1.0 {
                        assert!((a.s - b.s).abs() > 0.5 * (a.len + b.len) - 0.5, "overlap {} {}", a.s, b.s);
                    }
                }
            }
        }
    }

    #[test]
    fn instances_ride_the_centreline_height_and_lanes() {
        let mut c = Corridors::new(vec![straight(1000.0, 2, 2000.0)], Params::with_time_gap(3.0), 7);
        c.step(1.0 / 60.0, None);
        assert!(!c.out.is_empty());
        for o in &c.out {
            assert!((o.pos[1] - 10.0).abs() < 1e-4);
            // facing north (-z): right of travel is +x.
            assert!(o.pos[0] > 1.0 && o.pos[0] < 6.0, "x {}", o.pos[0]);
            assert!((o.heading.abs() - std::f32::consts::PI).abs() < 0.05);
        }
    }

    #[test]
    fn the_same_seed_replays() {
        let run = || {
            let mut c = Corridors::new(vec![straight(2000.0, 3, 2500.0)], Params::with_time_gap(3.0), 3);
            for _ in 0..600 {
                c.step(1.0 / 60.0, None);
            }
            c.out.clone()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn handover_ways_leave_the_local_window_to_the_local_cars() {
        let mut spec = straight(2000.0, 1, 3000.0);
        spec.handover = true;
        let mut c = Corridors::new(vec![spec], Params::with_time_gap(3.0), 5);
        c.step(1.0 / 60.0, Some([0.0, -1000.0]));
        assert!(!c.out.is_empty());
        for o in &c.out {
            assert!((o.pos[0] as f64).hypot(o.pos[2] as f64 + 1000.0) >= LOCAL_RECYCLE_RADIUS);
        }
    }

    #[test]
    fn hidden_stretches_draw_nothing() {
        let mut spec = straight(1000.0, 1, 3000.0);
        spec.points = vec![[0.0, 0.0, 0.0], [0.0, 0.0, -500.0], [0.0, 0.0, -1000.0]];
        spec.hidden = vec![true, false, false];
        let mut c = Corridors::new(vec![spec], Params::with_time_gap(3.0), 5);
        c.step(1.0 / 60.0, None);
        assert!(!c.out.is_empty());
        assert!(c.out.iter().all(|o| o.pos[2] <= -500.0));
    }

    #[test]
    fn the_profile_keeps_the_day_and_the_peaks() {
        let q = hourly_profile(46_500.0, 3000.0, 3750.0);
        let sum: f64 = q.iter().sum();
        assert!((sum - 46_500.0).abs() < 1.0, "sum {sum}");
        assert_eq!(q[8], 3000.0);
        assert_eq!(q[17], 3750.0);
        assert!(q[3] < q[12] && q[12] <= 3000.0);
    }

    #[test]
    fn the_mix_keeps_the_heavy_shares() {
        let m = mix_with_heavy(0.01, 0.003);
        assert!((m.iter().sum::<f64>() - 1.0).abs() < 1e-12);
        assert_eq!(m[3], 0.01);
        assert_eq!(m[2], 0.003);
    }
}
