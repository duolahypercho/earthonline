//! The run statistics the JS oracle records (the earlier browser version's exporter),
//! measured the same way on a Rust region, so the two runs can be compared.

use crate::geom;
use crate::network::Network;
use crate::pedestrians::LegKind;
use crate::region::Region;

pub const BODY_OVERLAP_MIN_M: f64 = 0.1;

#[derive(Clone, Debug, Default)]
pub struct Sample {
    pub t: f64,
    pub cars: usize,
    pub kinds: [usize; 4],
    pub moving: usize,
    pub mean_speed: f64,
    pub off_lane: usize,
    pub off_lane_max: f64,
    pub overlaps: usize,
    pub walkers: usize,
    pub walking: usize,
    pub waiting: usize,
    pub waiting_signal: usize,
    pub crossing: usize,
    pub paused: usize,
    pub mean_ground_speed: f64,
    pub on_road: usize,
    pub on_road_outside_crossing: usize,
    pub person_in_car: usize,
}

pub fn sample(net: &Network, r: &Region, t: f64) -> Sample {
    let f = &r.fleet;
    let mut s = Sample { t, cars: f.cars.len(), walkers: r.crowd.walkers.len(), ..Default::default() };
    let mut speed_sum = 0.0;
    for (i, c) in f.cars.iter().enumerate() {
        s.kinds[c.kind as usize] += 1;
        speed_sum += c.speed;
        if c.speed > 0.5 {
            s.moving += 1;
        }
        if !c.cornering() {
            let d = geom::dist_to_polyline(&net.lanes[c.lane as usize].pts, f.x[i], f.z[i]);
            s.off_lane_max = s.off_lane_max.max(d);
            if d > 0.5 {
                s.off_lane += 1;
            }
        }
    }
    s.mean_speed = speed_sum / f.cars.len().max(1) as f64;
    for a in 0..f.cars.len() {
        for b in (a + 1)..f.cars.len() {
            if !net.shares_surface(f.surface(a), f.surface(b))
                || geom::hypot(f.x[a] - f.x[b], f.z[a] - f.z[b]) > 14.0
            {
                continue;
            }
            let pa = (f.x[a], f.z[a], f.yaw[a], f.cars[a].len, f.cars[a].wid);
            let pb = (f.x[b], f.z[b], f.yaw[b], f.cars[b].len, f.cars[b].wid);
            if geom::obb_penetration(pa, pb) > BODY_OVERLAP_MIN_M {
                s.overlaps += 1;
            }
        }
    }
    let mut ground = 0.0;
    for (i, w) in r.crowd.walkers.iter().enumerate() {
        ground += w.ground_speed;
        if let Some(k) = &w.kerb {
            s.waiting += 1;
            if k.signal != crate::network::NONE {
                s.waiting_signal += 1;
            }
        } else if w.leg.is_some_and(|l| l.kind == LegKind::Cross) {
            s.crossing += 1;
        } else if w.ground_speed > 0.0 {
            s.walking += 1;
        } else {
            s.paused += 1;
        }
        let (x, z) = (r.crowd.x[i], r.crowd.z[i]);
        if net.on_carriageway(x, z, 0.3) {
            s.on_road += 1;
            if w.leg.is_none() {
                s.on_road_outside_crossing += 1;
            }
        }
        for (k, c) in f.cars.iter().enumerate() {
            if f.level[k] != 0 || geom::hypot(f.x[k] - x, f.z[k] - z) > 8.0 {
                continue;
            }
            if geom::obb_clearance(x, z, (f.x[k], f.z[k], f.yaw[k], c.len, c.wid)) <= 0.0 {
                s.person_in_car += 1;
            }
        }
    }
    s.mean_ground_speed = ground / r.crowd.walkers.len().max(1) as f64;
    s
}

/// Mean of a field over samples.
pub fn mean(samples: &[Sample], f: impl Fn(&Sample) -> f64) -> f64 {
    samples.iter().map(&f).sum::<f64>() / samples.len().max(1) as f64
}
