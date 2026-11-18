//! Per-junction signal control: phase plan, legal indication sequence and the
//! pedestrian windows. A port of the timing half of `buildSignalControlPlan`
//! and of the pure evaluators (`axisAspectAt`, `axisRedRemaining`,
//! `axisCrossedByBearing`, `crossingPedestrianRemaining`,
//! `pedestrianCrossingLengthM`, `crossingPedestrianIndication`) in
//! src/simulation/traffic/signal-control-plan.js.
//!
//! The plan is built from geometric inputs - which approaches form which axis,
//! each approach's stop offset and drawn half-width, the crossing length each
//! crossed axis is sized on, the posted and recorded approach speeds - and
//! every interval follows from those and the CA MUTCD values in
//! `SignalTiming`. Evaluation is a pure function of the simulation clock, so
//! any thread may ask any junction for its aspect without shared state.

use crate::geom::{P2, class_rank, design_speed, js_clamp};
use crate::standards::SignalTiming;

pub const RED: u8 = 0;
pub const YELLOW: u8 = 1;
pub const GREEN: u8 = 2;

/// One approach of an axis, as the plan's timing reads it.
#[derive(Clone, Debug)]
pub struct MemberInput {
    /// Index of the lane in the network build (resolved to a `LaneId` later).
    pub lane: u32,
    pub highway: String,
    /// `postedSpeedMps(segment)`: the contract's speed, else the CVC prima facie limit.
    pub posted_mps: f64,
    /// `contractSpeedMps(segment)`: the contract's speed or none.
    pub recorded_mps: Option<f64>,
    pub stop_offset_m: f64,
    /// The drawn carriageway half-width of this approach, if known.
    pub carriageway_half_m: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct AxisInput {
    pub bearing: f64,
    pub members: Vec<MemberInput>,
}

#[derive(Clone, Debug)]
pub struct JunctionInput {
    pub signal_ids: Vec<String>,
    pub centre: P2,
    /// The authored `signal.phaseOffset` nudge.
    pub phase_offset: f64,
    /// No lane arrives: the junction controls nothing.
    pub orphan: bool,
    /// Axes in plan order (after the MAX_PHASES ranking, kept axes first).
    pub axes: Vec<AxisInput>,
    /// Per axis: the longest carriageway a crosswalk across it spans (0 = not known: fallback).
    pub crossing_m: Vec<f64>,
    /// The tile that owns the controller (None: the tile of its centre).
    pub tile: Option<(i32, i32)>,
}

#[derive(Clone, Debug, Default)]
pub struct PedWindow {
    pub crossing_m: f64,
    pub walk_s: f64,
    pub clearance_s: f64,
    pub buffer_s: f64,
    pub span_s: f64,
    pub required_total_s: f64,
    pub release_axis: i32,
    pub walk_start_s: f64,
    pub clearance_end_s: f64,
    pub buffer_end_s: f64,
}

#[derive(Clone, Debug)]
pub struct SignalAxis {
    pub bearing: f64,
    pub suppressed: bool,
    pub class_rank: i32,
    pub v85: f64,
    pub posted_mps: f64,
    pub yellow_table_s: f64,
    pub green_base_s: f64,
    pub green_s: f64,
    pub yellow_s: f64,
    pub all_red_s: f64,
    pub red_clearance_distance_m: f64,
    pub red_clearance_speed_mps: f64,
    pub ped: Option<PedWindow>,
}

#[derive(Clone, Debug)]
pub struct SignalJunction {
    pub signal_ids: Vec<String>,
    pub centre: P2,
    pub orphan: bool,
    pub degenerate: bool,
    pub single_axis: bool,
    pub phase_count: usize,
    pub primary_axis: usize,
    pub cycle_s: f64,
    pub offset_s: f64,
    pub intervals: Vec<f64>,
    pub pedestrian_interval_s: f64,
    pub axes: Vec<SignalAxis>,
}

fn positive_modulo(v: f64, span: f64) -> f64 {
    if !(span > 0.0) {
        return 0.0;
    }
    let r = v % span;
    if r < 0.0 { r + span } else { r }
}

fn fold_axis(b: f64) -> f64 {
    positive_modulo(b, std::f64::consts::PI)
}

fn folded_gap(a: f64, b: f64) -> f64 {
    let mut d = (a - b).abs() % std::f64::consts::PI;
    if d > std::f64::consts::PI / 2.0 {
        d = std::f64::consts::PI - d;
    }
    d
}

fn crossed_axis_index(bearings: &[f64], bearing: f64) -> usize {
    let target = fold_axis(bearing + std::f64::consts::FRAC_PI_2);
    let mut best = 0;
    let mut best_gap = f64::INFINITY;
    for (i, &b) in bearings.iter().enumerate() {
        let gap = folded_gap(b, target);
        if gap < best_gap {
            best_gap = gap;
            best = i;
        }
    }
    best
}

/// `clusterAxes`: approach bearings into street axes (each bearing with its member index).
pub fn cluster_axes(approaches: &[(f64, String)], tolerance: f64) -> Vec<(f64, Vec<usize>)> {
    let mut sorted: Vec<(f64, &str, usize)> =
        approaches.iter().enumerate().map(|(i, (b, id))| (fold_axis(*b), id.as_str(), i)).collect();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(b.1)));
    let mut clusters: Vec<(Vec<usize>, Vec<f64>, f64, f64)> = vec![];
    for (folded, _, i) in sorted {
        if let Some(open) = clusters.last_mut() {
            if folded - open.2 <= tolerance {
                open.0.push(i);
                open.1.push(folded);
                open.3 = folded;
                continue;
            }
        }
        clusters.push((vec![i], vec![folded], folded, folded));
    }
    if clusters.len() > 1 {
        let last_min = clusters.last().unwrap().2;
        let first_max = clusters[0].3;
        if first_max - (last_min - std::f64::consts::PI) <= tolerance {
            let last = clusters.pop().unwrap();
            clusters[0].0.extend(last.0);
            clusters[0].1.extend(last.1);
            clusters[0].2 = last_min - std::f64::consts::PI;
        }
    }
    let mut axes: Vec<(f64, Vec<usize>, String)> = clusters
        .into_iter()
        .map(|(members, folded, _, _)| {
            let (mut sx, mut sz) = (0.0, 0.0);
            for f in &folded {
                sx += (f * 2.0).cos();
                sz += (f * 2.0).sin();
            }
            let first = approaches[members[0]].1.clone();
            (positive_modulo(sz.atan2(sx) / 2.0, std::f64::consts::PI), members, first)
        })
        .collect();
    axes.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.2.cmp(&b.2)));
    axes.into_iter().map(|(b, m, _)| (b, m)).collect()
}

impl SignalJunction {
    /// The timing half of `buildSignalControlPlan` for one junction.
    pub fn build(input: &JunctionInput, t: &SignalTiming) -> SignalJunction {
        let mut j = SignalJunction {
            signal_ids: input.signal_ids.clone(),
            centre: input.centre,
            orphan: input.orphan || input.axes.is_empty(),
            degenerate: false,
            single_axis: false,
            phase_count: 0,
            primary_axis: 0,
            cycle_s: 0.0,
            offset_s: 0.0,
            intervals: vec![],
            pedestrian_interval_s: f64::NAN,
            axes: vec![],
        };
        if j.orphan {
            j.degenerate = true;
            return j;
        }
        let n_axes = input.axes.len();
        j.phase_count = n_axes.min(t.max_phases);
        j.single_axis = j.phase_count == 1;
        j.degenerate = j.phase_count < 1;
        let widest_stop = input
            .axes
            .iter()
            .flat_map(|a| a.members.iter())
            .map(|m| m.stop_offset_m.max(0.0))
            .fold(0.0f64, f64::max);
        for (index, axis) in input.axes.iter().enumerate() {
            let v85 = {
                let m = axis.members.iter().map(|x| design_speed(&x.highway)).fold(0.0f64, f64::max);
                if m > 0.0 { m } else { design_speed("") }
            };
            let suppressed = index >= j.phase_count;
            let live = !suppressed && !j.degenerate;
            let (mut posted, mut yellow_table, mut yellow) = (0.0f64, 0.0f64, 0.0f64);
            if live {
                for m in &axis.members {
                    posted = posted.max(m.posted_mps);
                    yellow_table = yellow_table.max(t.yellow_minimum_for_posted(m.posted_mps));
                    yellow = yellow.max(t.yellow_change_interval(m.posted_mps, v85));
                }
            }
            let green_base = if live {
                js_clamp(
                    t.green_base_s + t.green_per_lane_s * axis.members.len() as f64,
                    t.green_min_s,
                    t.green_max_s,
                )
            } else {
                0.0
            };
            let near_stop = axis.members.iter().map(|m| m.stop_offset_m.max(0.0)).fold(0.0f64, f64::max);
            let mut conflict_half: Option<f64> = None;
            for (oi, other) in input.axes.iter().enumerate() {
                if oi == index {
                    continue;
                }
                for m in &other.members {
                    if let Some(h) = m.carriageway_half_m {
                        conflict_half = Some(conflict_half.map_or(h, |c| c.max(h)));
                    }
                }
            }
            let clearance_w = match conflict_half {
                Some(h) => near_stop + h,
                None => near_stop + widest_stop,
            };
            let mut clearance_v = f64::INFINITY;
            if live {
                for m in &axis.members {
                    clearance_v = clearance_v.min(m.recorded_mps.unwrap_or_else(|| design_speed(&m.highway)));
                }
            }
            if !clearance_v.is_finite() {
                clearance_v = v85;
            }
            let all_red = if live {
                js_clamp((clearance_w + t.nominal_vehicle_len_m) / clearance_v, t.all_red_min_s, t.all_red_max_s)
            } else {
                0.0
            };
            let class_rank = axis.members.iter().map(|m| class_rank(&m.highway)).max().unwrap_or(0);
            j.axes.push(SignalAxis {
                bearing: axis.bearing,
                suppressed,
                class_rank,
                v85,
                posted_mps: posted,
                yellow_table_s: yellow_table,
                green_base_s: green_base,
                green_s: green_base,
                yellow_s: yellow,
                all_red_s: all_red,
                red_clearance_distance_m: if live { clearance_w } else { 0.0 },
                red_clearance_speed_mps: if live { clearance_v } else { 0.0 },
                ped: None,
            });
            if !suppressed && class_rank > j.axes[j.primary_axis].class_rank {
                j.primary_axis = index;
            }
        }

        // Pedestrian phases (CA MUTCD 4I.06).
        if !j.degenerate {
            let bearings: Vec<f64> = j.axes.iter().map(|a| a.bearing).collect();
            for crossed in 0..n_axes {
                if j.single_axis && crossed > 0 {
                    break;
                }
                let sized = input.crossing_m.get(crossed).copied().unwrap_or(0.0);
                let length = if sized > 0.0 { sized } else { t.ped_fallback_crossing_m };
                let (walk, clearance, span, required) = t.pedestrian_timing(length);
                let release_axis = if j.single_axis {
                    -1
                } else {
                    // `releaseAxisIndex`
                    let target = fold_axis(bearings[crossed] + std::f64::consts::FRAC_PI_2);
                    let mut best = -1i32;
                    let mut best_gap = f64::INFINITY;
                    for i in 0..j.axes.len().min(j.phase_count) {
                        if i == crossed || j.axes[i].suppressed {
                            continue;
                        }
                        let gap = folded_gap(j.axes[i].bearing, target);
                        if gap < best_gap {
                            best_gap = gap;
                            best = i as i32;
                        }
                    }
                    best
                };
                j.axes[crossed].ped = Some(PedWindow {
                    crossing_m: length,
                    walk_s: walk,
                    clearance_s: clearance,
                    buffer_s: t.ped_buffer_s,
                    span_s: span,
                    required_total_s: required,
                    release_axis,
                    walk_start_s: f64::NAN,
                    clearance_end_s: f64::NAN,
                    buffer_end_s: f64::NAN,
                });
                if release_axis >= 0 {
                    let r = &mut j.axes[release_axis as usize];
                    if span > r.green_s {
                        r.green_s = span;
                    }
                }
            }
        }

        let mut intervals = vec![0.0; j.phase_count * 3];
        let mut cursor = 0.0;
        for (index, axis) in j.axes.iter().enumerate() {
            if axis.suppressed || j.degenerate {
                continue;
            }
            intervals[index * 3] = cursor;
            intervals[index * 3 + 1] = cursor + axis.green_s;
            intervals[index * 3 + 2] = cursor + axis.green_s + axis.yellow_s;
            cursor += axis.green_s + axis.yellow_s + axis.all_red_s;
        }
        if j.single_axis {
            let ped = j.axes[0].ped.as_mut().expect("single-axis junction has a pedestrian interval");
            ped.walk_start_s = cursor;
            j.pedestrian_interval_s = ped.span_s + ped.buffer_s;
            cursor += j.pedestrian_interval_s;
        } else if !j.degenerate {
            for axis in j.axes.iter_mut() {
                if let Some(ped) = axis.ped.as_mut() {
                    if ped.release_axis < 0 {
                        axis.ped = None;
                    } else {
                        ped.walk_start_s = intervals[ped.release_axis as usize * 3];
                    }
                }
            }
        }
        for axis in j.axes.iter_mut() {
            if let Some(ped) = axis.ped.as_mut() {
                ped.clearance_end_s = ped.walk_start_s + ped.span_s;
                ped.buffer_end_s = ped.clearance_end_s + ped.buffer_s;
            }
        }
        j.cycle_s = if j.degenerate { 0.0 } else { cursor };
        j.intervals = intervals;

        if !j.degenerate {
            let dominant = &j.axes[j.primary_axis];
            let progression = (input.centre[0] * dominant.bearing.cos() + input.centre[1] * dominant.bearing.sin())
                / t.progression_speed_mps;
            j.offset_s = positive_modulo(progression + input.phase_offset, j.cycle_s);
            let mut epoch = positive_modulo(-j.offset_s, j.cycle_s);
            for k in 0..j.phase_count {
                let green_end = j.intervals[k * 3 + 1];
                let phase_end = if k + 1 < j.phase_count { j.intervals[(k + 1) * 3] } else { j.cycle_s };
                if epoch >= green_end && epoch < phase_end {
                    epoch = if k + 1 < j.phase_count { j.intervals[(k + 1) * 3] } else { 0.0 };
                    break;
                }
            }
            j.offset_s = positive_modulo(-epoch, j.cycle_s);
        }
        j
    }

    /// `axisAspectAt(junction, axisIndex, seconds)`.
    #[inline]
    pub fn aspect_at(&self, axis: usize, seconds: f64) -> u8 {
        if self.orphan {
            return RED;
        }
        if self.degenerate {
            return GREEN;
        }
        if axis >= self.phase_count {
            return RED;
        }
        let u = positive_modulo(seconds - self.offset_s, self.cycle_s);
        let b = axis * 3;
        if u < self.intervals[b] {
            RED
        } else if u < self.intervals[b + 1] {
            GREEN
        } else if u < self.intervals[b + 2] {
            YELLOW
        } else {
            RED
        }
    }

    /// `axisRedRemaining`.
    pub fn red_remaining(&self, axis: usize, seconds: f64) -> f64 {
        if self.degenerate || self.aspect_at(axis, seconds) != RED {
            return 0.0;
        }
        if axis >= self.phase_count {
            return f64::INFINITY;
        }
        let u = positive_modulo(seconds - self.offset_s, self.cycle_s);
        let mut wait = self.intervals[axis * 3] - u;
        if wait <= 0.0 {
            wait += self.cycle_s;
        }
        wait
    }

    /// `axisCrossedByBearing`.
    pub fn axis_crossed_by_bearing(&self, bearing: f64) -> usize {
        if self.axes.is_empty() {
            return 0;
        }
        if !bearing.is_finite() {
            return self.primary_axis;
        }
        let bearings: Vec<f64> = self.axes.iter().map(|a| a.bearing).collect();
        crossed_axis_index(&bearings, bearing)
    }

    /// `crossingPedestrianRemaining`.
    pub fn ped_remaining(&self, crossed: usize, seconds: f64) -> f64 {
        if self.degenerate || self.orphan || !(self.cycle_s > 0.0) {
            return 0.0;
        }
        let Some(ped) = self.axes.get(crossed).and_then(|a| a.ped.as_ref()) else { return 0.0 };
        let u = positive_modulo(seconds - self.offset_s, self.cycle_s);
        let since = positive_modulo(u - ped.walk_start_s, self.cycle_s);
        let window = ped.span_s + ped.buffer_s;
        if since < window { window - since } else { 0.0 }
    }

    /// `pedestrianCrossingLengthM`.
    pub fn ped_crossing_length(&self, crossed: usize, link_length: f64) -> f64 {
        let sized = self.axes.get(crossed).and_then(|a| a.ped.as_ref()).map(|p| p.crossing_m);
        if !(link_length > 0.0) {
            return sized.unwrap_or(0.0);
        }
        match sized {
            Some(s) => link_length.min(s),
            None => link_length,
        }
    }

    /// `signalCrossingAxis(signal, link)`: the crossed axis whose pedestrian
    /// interval serves a crossing of this bearing, or none (take a gap).
    pub fn crossing_axis(&self, bearing: f64) -> Option<usize> {
        if self.degenerate || self.orphan {
            return None;
        }
        if self.single_axis && bearing.is_finite() {
            let axis = self.axes[0].bearing;
            let folded = (((bearing - axis) % std::f64::consts::PI) + std::f64::consts::PI) % std::f64::consts::PI;
            let off = folded.min(std::f64::consts::PI - folded);
            if off < std::f64::consts::FRAC_PI_4 {
                return None;
            }
        }
        let axis = self.axis_crossed_by_bearing(bearing);
        self.axes.get(axis).and_then(|a| a.ped.as_ref()).map(|_| axis)
    }
}

/// `crossingPedestrianIndication`: 0 dont-walk, 1 buffer, 2 clearance, 3 walk.
pub fn ped_indication(j: &SignalJunction, crossed: usize, seconds: f64, length_m: f64, t: &SignalTiming) -> u8 {
    let remaining = j.ped_remaining(crossed, seconds);
    let buffer = t.ped_buffer_s;
    if remaining >= length_m / t.ped_speed_mps + buffer {
        3
    } else if remaining > buffer {
        2
    } else if remaining > 0.0 {
        1
    } else {
        0
    }
}
