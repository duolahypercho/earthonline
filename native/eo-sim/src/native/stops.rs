//! Register sourced stop nodes to their signed lane approaches.

use std::collections::{HashMap, HashSet};

use eo_data::sim_source::{TrafficStop, TrafficStopControl};

use super::{JUNCTION_NODE_MAX_M, LaneMeta, NativeSource, SECTION_MATCH_COS, SECTION_MATCH_M, Seg, qkey, seg_nearest};
use crate::geom::{P2, cumulative};
use crate::network::{ControlKind, NetworkBuild};

#[derive(Default)]
pub(super) struct StopIndex {
    by_way: HashMap<String, Vec<Approach>>,
}

#[derive(Clone, Copy)]
struct Approach {
    id: u64,
    position: P2,
    toward: Option<P2>,
    all_way: bool,
}

impl StopIndex {
    pub(super) fn new(stops: Vec<TrafficStop>) -> Self {
        let mut by_way: HashMap<String, Vec<Approach>> = HashMap::new();
        for TrafficStop { id, position, control } in stops {
            match control {
                TrafficStopControl::Junction { junction } => {
                    for way in junction {
                        by_way.entry(way).or_default().push(Approach { id, position, toward: None, all_way: true });
                    }
                }
                TrafficStopControl::Approach { way, toward, all_way } => {
                    by_way.entry(way).or_default().push(Approach { id, position, toward: Some(toward), all_way });
                }
            }
        }
        Self { by_way }
    }

    pub(super) fn surveyed_control(&self, way: &str, id: u64, points: &[P2], bar_width: f64) -> Option<ControlKind> {
        let stop = self.by_way.get(way)?.iter().find(|s| s.id == id)?;
        let cum = cumulative(points);
        let length = *cum.last()?;
        let (distance, at) = project(points, &cum, stop.position, stop.toward?, false)?;
        let offset = length - at;
        (distance <= SECTION_MATCH_M && at > 0.0 && offset > 0.0).then_some(ControlKind::MappedStop {
            id, all_way: stop.all_way, stop_offset_m: offset, bar_depth_m: bar_width,
        })
    }
}

/// Project onto a part of the way travelled in the surveyed direction.
fn project(points: &[P2], cum: &[f64], position: P2, toward: P2, reverse: bool) -> Option<(f64, f64)> {
    points.windows(2).zip(cum.windows(2)).filter_map(|(w, s)| {
        let length = s[1] - s[0];
        let dot = (w[1][0] - w[0][0]) * toward[0] + (w[1][1] - w[0][1]) * toward[1];
        if length <= 0.0 || (if reverse { -dot } else { dot }) < length * SECTION_MATCH_COS {
            return None;
        }
        let (distance, at) = seg_nearest(w[0], w[1], position[0], position[1]);
        let station = s[0] + (at[0] - w[0][0]).hypot(at[1] - w[0][1]);
        Some((distance, station))
    }).min_by(|a, b| a.0.total_cmp(&b.0))
}

impl NativeSource {
    pub(super) fn apply_stops(
        &self,
        segs: &[Seg],
        lanes: &[LaneMeta],
        node_half: &HashMap<(i64, i64), Vec<(usize, f64)>>,
        signals: &HashSet<(i64, i64)>,
        build: &mut NetworkBuild,
    ) {
        if self.stops.by_way.is_empty() {
            return;
        }
        let mut signed = HashMap::<u64, ((f64, f64), usize, bool, Approach)>::new();
        let mut by_approach = HashMap::<(usize, bool), Approach>::new();
        for (si, seg) in segs.iter().enumerate() {
            let road = &self.roads[seg.road as usize];
            let Some(stops) = self.stops.by_way.get(&road.id) else { continue };
            if self.level_of(road) != 0 || road.bridge || road.tunnel || road.layer != 0 {
                continue;
            }
            let cum = cumulative(&seg.pts);
            let length = cum[cum.len() - 1];
            for (reverse, count, end) in [(false, seg.forward, seg.nodes[1]), (true, seg.backward, seg.nodes[0])] {
                let node = qkey(end);
                if count == 0 || signals.contains(&node) || node_half.get(&node).is_none_or(|legs| {
                    legs.iter().filter(|(leg, _)| {
                        let r = &self.roads[segs[*leg].road as usize];
                        self.level_of(r) == 0 && !r.bridge && !r.tunnel && r.layer == 0
                    }).count() < 3
                }) {
                    continue;
                }
                for &stop in stops {
                    let Some(toward) = stop.toward else {
                        if (stop.position[0] - end[0]).hypot(stop.position[1] - end[1]) <= SECTION_MATCH_M {
                            by_approach.entry((si, reverse)).or_insert(stop);
                        }
                        continue;
                    };
                    let Some((dist, at)) = project(&seg.pts, &cum, stop.position, toward, reverse) else { continue };
                    let remaining = if reverse { at } else { length - at };
                    if dist > SECTION_MATCH_M || remaining <= 0.0 || remaining > JUNCTION_NODE_MAX_M {
                        continue;
                    }
                    let rank = (dist, remaining);
                    let entry = signed.entry(stop.id).or_insert((rank, si, reverse, stop));
                    if rank < entry.0 {
                        *entry = (rank, si, reverse, stop);
                    }
                }
            }
        }
        // Multiple surveyed tags on one approach use its first stop line.
        let mut surveyed: Vec<_> = signed.into_values().collect();
        surveyed.sort_unstable_by(|a, b| b.0.1.total_cmp(&a.0.1).then(a.3.id.cmp(&b.3.id)));
        let mut approaches = HashSet::new();
        for (_, seg, reverse, mut stop) in surveyed {
            let key = (seg, reverse);
            if approaches.insert(key) {
                stop.all_way |= by_approach.get(&key).is_some_and(|junction| junction.all_way);
                by_approach.insert(key, stop);
            }
        }
        for (lane, meta) in build.lanes.iter_mut().zip(lanes) {
            let Some(stop) = by_approach.get(&(meta.seg, meta.reverse)) else { continue };
            if lane.control != ControlKind::None {
                continue;
            }
            let cum = cumulative(&lane.points);
            let length = cum[cum.len() - 1];
            let offset = if let Some(toward) = stop.toward {
                let Some((_, at)) = project(&lane.points, &cum, stop.position, toward, false) else { continue };
                length - at
            } else {
                let cross_half = node_half[&qkey(meta.end)].iter()
                    .filter(|(si, _)| {
                        let other = &self.roads[segs[*si].road as usize];
                        self.level_of(other) == 0 && !other.bridge && !other.tunnel && other.layer == 0
                            && other.name != lane.street
                    })
                    .map(|(_, half)| *half)
                    .fold(0.0f64, f64::max);
                cross_half + self.dims.crosswalk_m + self.dims.limit_line_setback_m
            };
            if offset > 0.0 && offset < length {
                lane.control = ControlKind::MappedStop {
                    id: stop.id, all_way: stop.all_way, stop_offset_m: offset, bar_depth_m: self.dims.limit_line_width_m,
                };
            }
        }
    }
}
