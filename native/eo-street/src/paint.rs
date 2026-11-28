//! Lane paint plans, ported from src/world/streets/street-surface-v2.js:
//! `planGenericSegmentMarkings` (the authored-lane branch and the geometric
//! fallback), `planParkingTerm`, `centreLineWarranted`, `brokenLinePattern`,
//! `planCrossingStripes`. Not ported: `applyObservedMarkings` (the reference-
//! photographed blocks of observed/marking-styles.js) and metered-kerb stall
//! ticks (`meteredKerbSides`): stall lines are never painted here.

use crate::city::Segment;
use crate::options::StreetOptions;
use crate::plan::street_class_rank;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Paint {
    White,
    Yellow,
    Green,
    None,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub role: &'static str,
    pub u: f64,
    pub width: f64,
    pub dashed: bool,
    pub paint: Paint,
    /// For a divider: +1 increasing, -1 decreasing traffic beside it.
    pub direction: i8,
    /// For an edge line: the kerb side it is laid against (+1 / -1), 0 otherwise.
    pub side: i8,
}

/// The longitudinal road paint for a complete street plan.
pub(crate) fn plan_markings(city: &crate::StreetCity, plan: &crate::plan::Plan, o: &StreetOptions) -> Vec<Vec<Line>> {
    let multi: HashSet<u64> = o.multi_lane_two_way.iter().copied().collect();
    plan.entries.iter().map(|e| {
        if e.structure_leg.is_some() {
            vec![]
        } else {
            segment_markings(&city.segments[e.road], &e.highway, e.half * 2.0, e.lanes, o, &multi)
        }
    }).collect()
}

fn line(role: &'static str, u: f64, width: f64, dashed: bool, paint: Paint) -> Line {
    Line { role, u, width, dashed, paint, direction: 0, side: 0 }
}

/// `scaledLineWidth`.
fn scaled(o: &StreetOptions, base: f64, k: f64) -> f64 {
    o.line_width_local[1].min(o.line_width_local[0].max(base * k))
}

pub struct ParkingTerm {
    pub width: f64,
    pub moving_lanes: i32,
    pub moving_half: f64,
}

/// `planParkingTerm`.
pub fn parking_term(seg: &Segment, highway: &str, width: f64, lanes: i32, o: &StreetOptions) -> Option<ParkingTerm> {
    if !o.parking_lanes || o.unmarked_highways.contains(highway) {
        return None;
    }
    let half = width / 2.0;
    if !(half >= o.min_marked_half_width) || seg.is_oneway() {
        return None;
    }
    if let Some(layout) = seg.lane_layout.as_ref().filter(|l| !l.is_empty()) {
        let by_role = layout.iter().filter(|l| l.role == "parking").map(|l| l.width).fold(0.0f64, f64::max);
        let by_band = seg.parking[0].min(seg.parking[1]);
        let p = by_role.max(by_band);
        if !(p >= o.parking_min_width) || !(p < half) {
            return None;
        }
        let moving = layout.iter().filter(|l| l.motor && l.role != "parking").count() as i32;
        return Some(ParkingTerm { width: p, moving_lanes: moving.max(1), moving_half: half - p });
    }
    let rank = street_class_rank(highway) as f64;
    if rank > o.parking_max_class_rank {
        return None;
    }
    let lanes = lanes.max(1);
    if lanes >= 4 && width / lanes as f64 <= o.parking_max_lane_width + 1e-9 {
        let p = width / lanes as f64;
        return Some(ParkingTerm { width: p, moving_lanes: lanes - 2, moving_half: half - p });
    }
    if lanes <= 2 {
        let leftover = (width - lanes as f64 * o.parking_moving_lane_min) / 2.0;
        if leftover >= o.parking_min_width {
            let p = o.parking_max_width.min(leftover);
            return Some(ParkingTerm { width: p, moving_lanes: lanes, moving_half: half - p });
        }
    }
    None
}

/// `postedSpeedKmh` + `brokenLinePattern`: (mark, gap).
pub fn broken_line_pattern(seg: &Segment, highway: &str, o: &StreetOptions) -> (f64, f64) {
    let mut kmh = seg.maxspeed_kmh.unwrap_or(0.0);
    if !(kmh > 0.0) {
        let tag = seg.maxspeed.trim().to_ascii_lowercase();
        let end = tag.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(tag.len());
        if let Ok(v) = tag[..end].parse::<f64>() {
            if v > 0.0 {
                kmh = if tag.contains("mph") { v * 1.609344 } else { v };
            }
        }
    }
    let fast = if kmh > 0.0 { kmh / 1.609344 >= 45.0 - 0.5 } else { matches!(highway, "motorway" | "trunk") };
    if fast { (o.dash_mark_fast, o.dash_gap_fast) } else { (o.dash_mark, o.dash_gap) }
}

/// The OSM way id of `streetId: 'sf-street-<id>'`.
fn way_id(seg: &Segment) -> Option<u64> {
    seg.street_id.as_deref().and_then(|s| s.strip_prefix("sf-street-")).and_then(|s| s.parse().ok())
}

/// `centreLineWarranted`: Some(true) shall (double yellow), Some(false) may (broken), None none.
fn centre_warranted(seg: &Segment, rank: i32, through: i32, travelled: f64, o: &StreetOptions, multi: &HashSet<u64>) -> Option<bool> {
    if through as f64 >= o.centre_shall_lanes {
        return Some(true);
    }
    if way_id(seg).map(|w| multi.contains(&w)).unwrap_or(false) {
        return Some(true);
    }
    if rank >= 4 && travelled >= o.centre_traveled_way_min - 1e-6 {
        return Some(true);
    }
    let bike = seg.cycle[0] > 0.0 || seg.cycle[1] > 0.0;
    if bike && travelled >= o.centre_may_traveled_way_min - 1e-6 {
        return Some(false);
    }
    None
}

fn push_direction_boundary(lines: &mut Vec<Line>, u: f64, k: f64, o: &StreetOptions) {
    let stroke = scaled(o, o.centre_line_width, k);
    let pitch = o.centre_line_gap - o.centre_line_width + stroke;
    for sign in [-1.0, 1.0] {
        lines.push(line("centre", u + sign * pitch / 2.0, stroke, false, Paint::Yellow));
    }
}

fn push_centre_turn_boundary(lines: &mut Vec<Line>, centre: f64, lane_w: f64, k: f64, o: &StreetOptions) {
    let outer = lane_w / 2.0;
    let stroke = scaled(o, o.centre_line_width, k);
    let inner = (outer / 2.0).max(outer - (o.twltl_solid_to_broken + stroke));
    for sign in [-1.0, 1.0] {
        lines.push(line("centre-turn-boundary", centre + sign * outer, stroke, false, Paint::Yellow));
        lines.push(line("centre-turn-boundary", centre + sign * inner, stroke, true, Paint::Yellow));
    }
}

/// `planGenericSegmentMarkings` for an entry's roadway.
pub fn segment_markings(seg: &Segment, highway: &str, width: f64, lanes: i32, o: &StreetOptions, multi: &HashSet<u64>) -> Vec<Line> {
    let half = width / 2.0;
    let mut lines = vec![];
    if !(half >= o.min_marked_half_width) || o.unmarked_highways.contains(highway) {
        return lines;
    }
    let lanes = lanes.max(1);
    let oneway = seg.is_oneway();
    let parking = parking_term(seg, highway, width, lanes, o);
    let rank = street_class_rank(highway);
    let k = if rank >= 6 { 1.3 } else if rank >= 5 { 1.15 } else { 1.0 };
    let authored: Vec<(i8, String, f64, f64)> = seg
        .lane_layout
        .as_ref()
        .map(|l| l.iter().filter(|x| x.motor || x.role == "center-turn").map(|x| (x.dir, x.role.clone(), x.width, -x.lateral)).collect())
        .unwrap_or_default();
    if !authored.is_empty() {
        let mut moving: Vec<&(i8, String, f64, f64)> = authored.iter().filter(|l| l.1 == "through").collect();
        moving.sort_by(|a, b| a.3.partial_cmp(&b.3).unwrap());
        if moving.is_empty() {
            return lines;
        }
        let left_edge = moving.iter().map(|l| l.3 + l.2 / 2.0).fold(f64::MIN, f64::max);
        let right_edge = moving.iter().map(|l| l.3 - l.2 / 2.0).fold(f64::MAX, f64::min);
        let edge_w = scaled(o, o.edge_line_width, k);
        let quiet_alley = seg.street_type.as_deref() == Some("alley") && moving.len() == 1;
        let edge_warranted = !quiet_alley && (rank as f64 >= o.edge_line_min_class_rank || (oneway && moving.len() >= 3));
        if edge_warranted || seg.cycle[0] > 0.0 {
            let mut l = line("edge", left_edge.min(half) - edge_w / 2.0, edge_w, false, Paint::White);
            l.side = 1;
            lines.push(l);
        }
        if edge_warranted || seg.cycle[1] > 0.0 {
            let mut l = line("edge", right_edge.max(-half) + edge_w / 2.0, edge_w, false, Paint::White);
            l.side = -1;
            lines.push(l);
        }
        for i in 1..moving.len() {
            let (p, c) = (moving[i - 1], moving[i]);
            if p.0 != c.0 {
                continue;
            }
            let mut l = line("divider", (p.3 + c.3) / 2.0, scaled(o, o.lane_line_width, k), true, Paint::White);
            l.direction = c.0;
            lines.push(l);
        }
        let centre_turn = authored.iter().find(|l| l.1 == "center-turn");
        let median = seg.median.max(0.0);
        let mut boundary = 0.0;
        let mut meet = false;
        for i in 1..moving.len() {
            if moving[i - 1].0 == moving[i].0 {
                continue;
            }
            boundary = (moving[i - 1].3 + moving[i].3) / 2.0;
            meet = true;
            break;
        }
        if let Some(ct) = centre_turn {
            push_centre_turn_boundary(&mut lines, ct.3, ct.2, k, o);
        } else if !oneway && meet && median > 0.0 {
            for sign in [-1.0, 1.0] {
                lines.push(line("median-edge", boundary + sign * median / 2.0, scaled(o, o.centre_line_width, k), false, Paint::Yellow));
            }
        } else if !oneway && meet {
            match centre_warranted(seg, rank, moving.len() as i32, left_edge - right_edge, o, multi) {
                Some(false) => lines.push(line("centre", boundary, scaled(o, o.centre_line_width, k), true, Paint::Yellow)),
                Some(true) => push_direction_boundary(&mut lines, boundary, k, o),
                None => lines.push(line("unpainted-centre", boundary, 0.0, false, Paint::None)),
            }
        }
        let band_inset = edge_w;
        let mut drop_edge: Vec<i8> = vec![];
        for (sign, kk, travel_edge) in [(1.0f64, 0usize, left_edge), (-1.0, 1, right_edge)] {
            let parking_w = seg.parking[kk].max(0.0);
            let cycle = seg.cycle[kk].max(0.0);
            let buffer = seg.cycle_buffer[kk].max(0.0);
            let kerbside = seg.cycle_kerbside[kk];
            let shoulder = seg.shoulder[kk].max(0.0);
            let at = |d: f64| travel_edge + sign * (shoulder + d);
            if shoulder > 0.0 && !(parking_w > 0.0) && (rank as f64 >= o.edge_line_min_class_rank || (oneway && lanes >= 3)) {
                let edge_u = travel_edge - sign * (edge_w / 2.0);
                if !lines.iter().any(|l| l.role == "edge" && (l.u - edge_u).abs() < edge_w) {
                    lines.push(line("edge", edge_u, edge_w, false, Paint::White));
                }
            }
            let fits = |d: f64| sign * at(d) <= half + 0.02;
            if kerbside && cycle >= 1.2 && fits(parking_w + cycle) {
                let buf = buffer.min(cycle - 1.2);
                if buf > 0.2 {
                    let b0 = at(parking_w);
                    lines.push(line("cycle-buffer-edge", b0 + sign * (band_inset / 2.0), band_inset, false, Paint::White));
                    lines.push(line("cycle-buffer", b0 + sign * (buf / 2.0), buf, false, Paint::None));
                }
                let g0 = at(parking_w + buf.max(0.0));
                let g1 = sign * (half - o.gutter_width.min(0.3));
                let green = (g1 - g0).abs();
                if green >= 1.0 {
                    lines.push(line("cycle-lane", (g0 + g1) / 2.0, (green - band_inset * 2.0).max(0.5), false, Paint::Green));
                }
                continue;
            }
            if cycle >= 1.2 && fits(cycle) {
                lines.push(line("cycle-lane", at(cycle / 2.0), (cycle - band_inset * 2.0).max(0.5), false, Paint::White));
            }
            if parking_w > 0.0 && fits(cycle + parking_w) && parking_w >= o.parking_min_width && !(cycle > 0.0) {
                drop_edge.push(sign as i8);
            }
        }
        if !drop_edge.is_empty() {
            lines.retain(|l| !(l.role == "edge" && drop_edge.contains(&l.side)));
        }
        return lines;
    }
    let edge_u = (0.35f64).max(half - o.gutter_width - 0.1);
    let paints_edge = parking.is_none() && (rank as f64 >= o.edge_line_min_class_rank || (oneway && lanes >= 3));
    if paints_edge {
        let w = scaled(o, o.edge_line_width, k);
        lines.push(line("edge", edge_u, w, false, Paint::White));
        lines.push(line("edge", -edge_u, w, false, Paint::White));
    }
    if oneway {
        let dir = if seg.oneway == "decreasing" { -1 } else { 1 };
        for j in 1..lanes {
            let mut l = line("divider", -half + width * j as f64 / lanes as f64, scaled(o, o.lane_line_width, k), true, Paint::White);
            l.direction = dir;
            lines.push(l);
        }
        return lines;
    }
    if let Some(p) = &parking {
        for sign in [1.0, -1.0] {
            lines.push(line("parking", sign * p.moving_half, scaled(o, o.parking_line_width, k), false, Paint::None));
        }
    }
    let moving = parking.as_ref().map(|p| p.moving_lanes).unwrap_or(lanes);
    let moving_half = parking.as_ref().map(|p| p.moving_half).unwrap_or(half);
    if moving < 2 {
        return lines;
    }
    let centre_turn = moving >= 3 && moving % 2 == 1;
    let through = moving - if centre_turn { 1 } else { 0 };
    let lane_w = moving_half * 2.0 / moving as f64;
    if centre_turn {
        push_centre_turn_boundary(&mut lines, 0.0, lane_w, k, o);
    } else {
        match centre_warranted(seg, rank, through, moving_half * 2.0, o, multi) {
            Some(false) => lines.push(line("centre", 0.0, scaled(o, o.centre_line_width, k), true, Paint::Yellow)),
            Some(true) => push_direction_boundary(&mut lines, 0.0, k, o),
            None => lines.push(line("unpainted-centre", 0.0, 0.0, false, Paint::None)),
        }
    }
    let per_side = through as f64 / 2.0;
    let first = if centre_turn { lane_w / 2.0 } else { 0.0 };
    let mut j = 1.0;
    while j < per_side {
        let u = first + j * lane_w;
        let mut a = line("divider", u, scaled(o, o.lane_line_width, k), true, Paint::White);
        a.direction = 1;
        lines.push(a);
        let mut b = line("divider", -u, scaled(o, o.lane_line_width, k), true, Paint::White);
        b.direction = -1;
        lines.push(b);
        j += 1.0;
    }
    lines
}

pub struct Stripes {
    pub pitch: f64,
    pub width: f64,
    /// [v0, v1] across the approach, centred on its axis.
    pub stripes: Vec<[f64; 2]>,
}

/// `planCrossingStripes`.
pub fn crossing_stripes(half: f64, o: &StreetOptions) -> Stripes {
    let inset = o.crosswalk_edge_inset;
    let width = o.crossing_stripe_width;
    let pitch = (width + 0.15).max(o.crossing_stripe_pitch);
    let usable = (half * 2.0 - inset * 2.0).max(0.0);
    let count = (o.crossing_min_stripes as i64).max(((usable + (pitch - width)) / pitch).floor() as i64);
    let span = count as f64 * pitch - (pitch - width);
    let mut stripes = vec![];
    for i in 0..count {
        let v = -span / 2.0 + width / 2.0 + i as f64 * pitch;
        if v.abs() + width / 2.0 > half - inset + 1e-6 {
            continue;
        }
        stripes.push([v - width / 2.0, v + width / 2.0]);
    }
    Stripes { pitch, width, stripes }
}
