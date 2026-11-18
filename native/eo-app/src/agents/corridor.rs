//! The corridor layer's roads (eo-sim `corridor`), laid on what the renderer
//! draws: US-101 over the Golden Gate Bridge and down the Presidio Parkway,
//! CA-1 through the Presidio (Veterans Boulevard), Marina Boulevard, and the
//! walkers on the bridge's east walk.
//!
//! On the bridge the lanes are the deck's own: the centreline is the deck's
//! alignment (`Site::deck_point`), the height `Plan::deck_top` (so a change to
//! the deck's profile carries the traffic with it), and the lane centres the
//! layout `suspension::road_deck` paints (half the median barrier and 0.3 m
//! each side of it, then the lanes per direction sharing the rest of the
//! half roadway). Off the deck the centreline is the surveyed way's (the
//! extract's road lines, chained by name), the lane width the street model's
//! (`NativeSource::chains`, the scaled ground plan's), the height the drawn
//! street surface under it, followed along the run so a viaduct stays on its
//! deck and a road under it stays on the ground.
//!
//! `--no-corridor-traffic` leaves the whole layer out (and the local
//! simulation keeps every road, as before); `--no-deck-walkers` leaves out the
//! walkers alone.

use eo_landmarks::LandmarksJs;
use eo_landmarks::data::num;
use eo_landmarks::golden_gate::{self, Site};
use eo_sim::corridor::{Travel, WaySpec, hourly_profile, mix_with_heavy};
use eo_sim::native::{Chain, NativeSource};

/// The switches: (`--no-corridor-traffic` absent, `--no-deck-walkers` absent).
pub fn switches() -> (bool, bool) {
    let args: Vec<String> = std::env::args().collect();
    let has = |s: &str| args.iter().any(|a| a == s);
    let traffic = !has("--no-corridor-traffic");
    (traffic, traffic && !has("--no-deck-walkers"))
}

// ---------------------------------------------------------------- counts

/// US-101 at the Golden Gate Bridge (SF R9.561 / MRN 0.000): AADT 93,000 and
/// peak hour 7,500, both directions. Caltrans Traffic Census, "Traffic
/// Volumes AADT" layer (caltrans-gis.dot.ca.gov/arcgis/rest/services/
/// CHhighway/Traffic_AADT/FeatureServer/0, rows "GOLDEN GATE BRIDGE", read
/// 2026-11-18).
pub const US101_GGB_AADT: f64 = 93_000.0;
pub const US101_GGB_PEAK_HOUR: f64 = 7_500.0;
/// The same station's truck count (Caltrans "Truck Volumes AADT" layer, MRN
/// 0.000 "GOLDEN GATE BRIDGE", read 2026-11-18): of 92,000 vehicles, 899 on
/// two axles and 89 + 25 + 211 on three or more.
pub const US101_GGB_TRUCK_2AXLE: f64 = 899.0 / 92_000.0;
pub const US101_GGB_TRUCK_3AXLE_PLUS: f64 = (89.0 + 25.0 + 211.0) / 92_000.0;
/// The peak direction's share of the peak hour: the moveable median gives the
/// commute direction four of the six lanes (four southbound into San
/// Francisco in the morning, four northbound in the evening; goldengate.org,
/// "Moveable Median Barrier", read 2026-11-18). INFERRED: the demand split
/// the lane split was set for.
pub const PEAK_DIRECTION_SHARE: f64 = 4.0 / 6.0;
/// CA-1 at its junction with US-101 (SF 7.077): AADT 62,000, peak hour 5,900
/// (Caltrans "Traffic Volumes AADT", "JCT. RTE. 101", read 2026-11-18).
pub const CA1_AADT: f64 = 62_000.0;
pub const CA1_PEAK_HOUR: f64 = 5_900.0;
/// Marina Boulevard at Buchanan Street, Wednesday 13 Feb 2008: westbound
/// 9,652 a day (AM peak hour 1,179, PM 612), eastbound 9,774 (AM 477, PM
/// 1,178). SFMTA, "Traffic Count Data 1993-2015" (sfmta.com, 2016 report).
pub const MARINA_WB: [f64; 3] = [9_652.0, 1_179.0, 612.0];
pub const MARINA_EB: [f64; 3] = [9_774.0, 477.0, 1_178.0];
/// Pedestrian crossings on the bridge's sidewalks, September 2026: 199,462
/// (goldengate.org, "Golden Gate Bridge Monthly Bike & Pedestrian Sidewalk
/// Volume", FY2026/2027, estimated). Pedestrians use the east sidewalk only,
/// 5:00 am to 9:00 pm in daylight time (goldengate.org, "Bikes &
/// Pedestrians"). INFERRED: spread evenly over the open hours and the two
/// directions (no hourly count is published).
pub const PEDESTRIANS_SEPTEMBER: f64 = 199_462.0;
pub const EAST_WALK_OPEN_H: [usize; 2] = [5, 21];

/// The roads the corridor layer drives alone (the local simulation leaves them).
pub const EXCLUSIVE: [&str; 4] = ["Golden Gate Bridge", "Presidio Parkway", "CA 1", "Veterans Boulevard"];

/// Hourly flows (south or west first, north or east second) from a
/// two-direction count and its peak hour, the peak direction south in the
/// morning and north in the evening.
fn commute_pair(aadt: f64, peak_hour: f64) -> ([f64; 24], [f64; 24]) {
    let (hi, lo) = (peak_hour * PEAK_DIRECTION_SHARE, peak_hour * (1.0 - PEAK_DIRECTION_SHARE));
    (hourly_profile(aadt / 2.0, hi, lo), hourly_profile(aadt / 2.0, lo, hi))
}

fn walker_flow() -> [f64; 24] {
    let open = (EAST_WALK_OPEN_H[1] - EAST_WALK_OPEN_H[0]) as f64;
    let per_hour_each_way = PEDESTRIANS_SEPTEMBER / 30.0 / open / 2.0;
    let mut q = [0.0; 24];
    for (h, v) in q.iter_mut().enumerate() {
        if (EAST_WALK_OPEN_H[0]..EAST_WALK_OPEN_H[1]).contains(&h) {
            *v = per_hour_each_way;
        }
    }
    q
}

// ---------------------------------------------------------------- the deck

/// The deck's frame and the layout `road_deck` paints on it.
pub struct Deck<'a> {
    pub plan: golden_gate::Plan<'a>,
    /// Lane centres from the deck axis (each direction, outward from the median).
    pub lanes: Vec<f64>,
    pub half_width: f64,
    pub curb: f64,
    pub curb_m: f64,
    /// The towers' walkway detour: (zone half length, leg half length along, leg outer face, walk outer edge).
    pub tower_walk: [f64; 4],
}

impl<'a> Deck<'a> {
    pub fn new(js: &LandmarksJs, site: &'a Site, ground: &dyn Fn(f64, f64) -> f64, k_width: f64) -> Self {
        let g = golden_gate::data(js);
        let d = &g.dimensions;
        let plan = golden_gate::plan(js, site, ground);
        // suspension::road_deck: median_half = k_width / 2 + 0.3, per = lanes / 2, lane_w = (curb - median_half) / per.
        let curb = num(d, "roadwayM") / 2.0;
        let per = num(d, "lanesPerDirection");
        let median_half = k_width / 2.0 + 0.3;
        let lane_w = (curb - median_half) / per;
        let lanes = (0..per as usize).map(|k| median_half + (k as f64 + 0.5) * lane_w).collect();
        // golden_gate.rs tower_walkways: the leg stage at the roadway, the zone half length.
        let water = js.sea_level_y;
        let stage = g.leg_stages.iter().find(|s| plan.y_tower - water >= s[0] && plan.y_tower - water < s[1]).copied().unwrap_or(g.leg_stages[2]);
        let zone = g.leg_stages.iter().map(|s| s[3]).fold(f64::MIN, f64::max) / 2.0 + 1.0;
        let leg_outer = site.half_plane + stage[2] / 2.0;
        let tower_walk = [zone, stage[3] / 2.0, leg_outer, leg_outer + num(d, "sidewalkM")];
        Deck { plan, lanes, half_width: num(d, "widthM") / 2.0, curb, curb_m: num(d, "curbM"), tower_walk }
    }

    /// On the deck between its way ends, within its width.
    fn on_deck(&self, x: f64, z: f64) -> Option<f64> {
        let site = self.plan.site;
        let c = site.chainage(x, z);
        if c < self.plan.c_south || c > self.plan.c_north {
            return None;
        }
        let [ax, az] = site.deck_point(c, 0.0);
        let s = (x - ax) * site.across[0] + (z - az) * site.across[1];
        (s.abs() <= self.half_width + 2.0).then_some(c)
    }

    /// The middle of the east walk at chainage c, detouring round the tower legs.
    fn walk_s(&self, c: f64) -> f64 {
        let [zone, along, leg_outer, walk_outer] = self.tower_walk;
        let open = (self.curb + self.half_width) / 2.0;
        let detour = (leg_outer + walk_outer) / 2.0;
        let mut s = open;
        for t in [0.0, self.plan.l] {
            let d = (c - t).abs();
            if d < along + 1.0 {
                s = detour;
            } else if d < zone {
                // across the zone's plate (curb .. walk outer) toward the detour.
                let k = ((zone - d) / (zone - along - 1.0).max(0.1)).clamp(0.0, 1.0);
                s = open + (detour - open) * k * k * (3.0 - 2.0 * k);
            }
        }
        s
    }
}

// ---------------------------------------------------------------- building the ways

/// Points of a polyline no farther apart than `step` metres.
fn resample(pts: &[[f64; 2]], flags: &[(bool, bool)], step: f64) -> (Vec<[f64; 2]>, Vec<(bool, bool)>) {
    let mut out = vec![pts[0]];
    let mut out_f = vec![flags[0]];
    for i in 1..pts.len() {
        let (a, b) = (pts[i - 1], pts[i]);
        let n = ((b[0] - a[0]).hypot(b[1] - a[1]) / step).ceil().max(1.0) as usize;
        for k in 1..=n {
            let t = k as f64 / n as f64;
            out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
            out_f.push(if k == n { flags[i] } else { flags[i - 1] });
        }
    }
    (out, out_f)
}

/// Shift a polyline `d` metres to the left of travel.
fn shift_left(pts: &[[f64; 2]], d: f64) -> Vec<[f64; 2]> {
    let n = pts.len();
    (0..n)
        .map(|i| {
            let (a, b) = (pts[i.saturating_sub(1)], pts[(i + 1).min(n - 1)]);
            let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
            let l = dx.hypot(dz).max(1e-9);
            // right of travel is (-dz, dx); left is (dz, -dx).
            [pts[i][0] + dz / l * d, pts[i][1] - dx / l * d]
        })
        .collect()
}

/// Heights along a run on the drawn street: the paved surface nearest the
/// last height (a viaduct's deck or the road under it), else the ground.
fn heights(pts: &[[f64; 2]], start: Option<f64>, paved: &dyn Fn(f64, f64) -> Vec<f64>, ground: &dyn Fn(f64, f64) -> f64) -> Vec<f64> {
    let mut last = start;
    pts.iter()
        .map(|p| {
            let hs = paved(p[0], p[1]);
            let y = match last {
                Some(l) => hs.iter().copied().min_by(|a, b| (a - l).abs().total_cmp(&(b - l).abs())).filter(|h| (h - l).abs() < 6.0),
                None => hs.iter().copied().fold(None, |m: Option<f64>, h| Some(m.map_or(h, |m| m.max(h)))),
            }
            .unwrap_or_else(|| ground(p[0], p[1]));
            last = Some(y);
            y
        })
        .collect()
}

/// Everything the ways are laid on.
pub struct Surfaces<'a> {
    pub deck: Option<&'a Deck<'a>>,
    /// `paved` reads a drawn street (else it is always empty and hides nothing).
    pub street: bool,
    pub paved: &'a dyn Fn(f64, f64) -> Vec<f64>,
    pub ground: &'a dyn Fn(f64, f64) -> f64,
}

/// One chain as a way: on the deck the deck's lanes, else the road's.
fn way_of(chain: &Chain, s: &Surfaces, name: &str, flow: [f64; 24], mix: [f64; 4], handover: bool) -> WaySpec {
    let flags: Vec<(bool, bool)> = chain.tunnel.iter().zip(&chain.bridge).map(|(t, b)| (*t, *b)).collect();
    let (pts, flags) = resample(&chain.points, &flags, 8.0);
    let n = chain.lanes.clamp(1, 4) as usize;
    let deck_c: Vec<Option<f64>> = pts.iter().map(|p| s.deck.and_then(|d| d.on_deck(p[0], p[1]))).collect();
    let any_deck = deck_c.iter().any(|c| c.is_some());
    // Lanes: on a run that crosses the deck, the deck's (its count and width
    // on the whole run, the centreline on the median); else the road's,
    // spread about its line (a one-way carriageway's centre).
    let (lanes, centre_shift): (Vec<f64>, f64) = match (any_deck, s.deck) {
        (true, Some(d)) => (d.lanes.clone(), (d.lanes[0] + d.lanes[d.lanes.len() - 1]) / 2.0),
        _ => ((0..n).map(|k| (k as f64 - (n as f64 - 1.0) / 2.0) * chain.lane_w).collect(), 0.0),
    };
    // Off the deck the road line is the carriageway's centre: the centreline sits `centre_shift` left of it.
    let off = shift_left(&pts, centre_shift);
    let mut out_pts: Vec<[f64; 3]> = Vec::with_capacity(pts.len());
    let ys = heights(&off, None, s.paved, s.ground);
    for i in 0..pts.len() {
        match (deck_c[i], s.deck) {
            (Some(c), Some(d)) => {
                let [x, z] = d.plan.site.deck_point(c, 0.0);
                out_pts.push([x, d.plan.deck_top(c), z]);
            }
            _ => out_pts.push([off[i][0], ys[i], off[i][1]]),
        }
    }
    // The run's heights where it leaves the deck: follow on from the deck's height.
    if any_deck {
        let first_deck = deck_c.iter().position(|c| c.is_some()).unwrap();
        let last_deck = deck_c.iter().rposition(|c| c.is_some()).unwrap();
        if last_deck + 1 < pts.len() {
            let tail = heights(&off[last_deck + 1..], Some(out_pts[last_deck][1]), s.paved, s.ground);
            for (k, y) in tail.into_iter().enumerate() {
                out_pts[last_deck + 1 + k][1] = y;
            }
        }
        if first_deck > 0 {
            let rev: Vec<[f64; 2]> = off[..first_deck].iter().rev().copied().collect();
            let head = heights(&rev, Some(out_pts[first_deck][1]), s.paved, s.ground);
            for (k, y) in head.into_iter().enumerate() {
                out_pts[first_deck - 1 - k][1] = y;
            }
        }
    }
    // Nothing is drawn where the street draws no road (a tunnel, a covered
    // cut, a way the street leaves out): off the deck, a stretch whose road
    // line has no paving under it is hidden (with a street to read).
    let hidden = (0..pts.len())
        .map(|i| flags[i].0 || (s.street && deck_c[i].is_none() && (s.paved)(pts[i][0], pts[i][1]).is_empty()))
        .collect();
    WaySpec {
        name: name.to_string(),
        points: out_pts,
        lanes,
        speed_mps: chain.speed_mps.unwrap_or(eo_sim::consts::AUTONOMOUS_TRAFFIC_CEILING_MPS),
        flow_vph: flow,
        mix,
        travel: Travel::Vehicles,
        hidden,
        handover,
    }
}

/// Heading class of a run: true when it travels mostly south or west.
fn south_or_west(chain: &Chain) -> bool {
    let (a, b) = (chain.points[0], chain.points[chain.points.len() - 1]);
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    if dz.abs() >= dx.abs() { dz > 0.0 } else { dx < 0.0 }
}

/// Every corridor way (before the local network leaves the exclusive roads).
pub fn ways(source: &NativeSource, s: &Surfaces, walkers: bool) -> Vec<WaySpec> {
    let mut out = vec![];
    let heavy = mix_with_heavy(US101_GGB_TRUCK_2AXLE, US101_GGB_TRUCK_3AXLE_PLUS);
    let (gg_south, gg_north) = commute_pair(US101_GGB_AADT, US101_GGB_PEAK_HOUR);
    let gg: Vec<Chain> = source.chains(&["Golden Gate Bridge", "Presidio Parkway"], 400.0);
    for c in &gg {
        let sw = south_or_west(c);
        out.push(way_of(c, s, if sw { "US-101 southbound" } else { "US-101 northbound" }, if sw { gg_south } else { gg_north }, heavy, false));
    }
    // CA-1: INFERRED, the bridge's directional split and heavy share (it carries the bridge's traffic).
    let (ca_south, ca_north) = commute_pair(CA1_AADT, CA1_PEAK_HOUR);
    for c in &source.chains(&["CA 1", "Veterans Boulevard"], 300.0) {
        let sw = south_or_west(c);
        out.push(way_of(c, s, if sw { "CA-1 southbound" } else { "CA-1 northbound" }, if sw { ca_south } else { ca_north }, heavy, false));
    }
    let wb = hourly_profile(MARINA_WB[0], MARINA_WB[1], MARINA_WB[2]);
    let eb = hourly_profile(MARINA_EB[0], MARINA_EB[1], MARINA_EB[2]);
    for c in &source.chains(&["Marina Boulevard"], 300.0) {
        let w = south_or_west(c);
        out.push(way_of(c, s, if w { "Marina Boulevard westbound" } else { "Marina Boulevard eastbound" }, if w { wb } else { eb }, eo_sim::consts::TRAFFIC_KIND_MIX, true));
    }
    if walkers {
        if let Some(d) = s.deck {
            out.extend(walk_ways(d));
        }
    }
    out
}

/// The east walk's two directions, each keeping to its right.
fn walk_ways(d: &Deck) -> Vec<WaySpec> {
    let (c0, c1) = (d.plan.c_south, d.plan.c_north);
    let n = ((c1 - c0) / 4.0).ceil().max(2.0) as usize;
    let flow = walker_flow();
    [(true, 0.45), (false, -0.45)]
        .into_iter()
        .map(|(north, keep)| {
            let mut pts: Vec<[f64; 3]> = (0..=n)
                .map(|k| {
                    let c = c0 + (c1 - c0) * k as f64 / n as f64;
                    let [x, z] = d.plan.site.deck_point(c, d.walk_s(c) + keep);
                    [x, d.plan.deck_top(c) + d.curb_m, z]
                })
                .collect();
            if !north {
                pts.reverse();
            }
            WaySpec {
                name: if north { "east walk northbound".into() } else { "east walk southbound".into() },
                points: pts,
                lanes: vec![0.0],
                speed_mps: 0.0,
                flow_vph: flow,
                mix: [1.0, 0.0, 0.0, 0.0],
                travel: Travel::Walkers,
                hidden: vec![],
                handover: false,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bridge_flows_keep_the_count() {
        let (s, n) = commute_pair(US101_GGB_AADT, US101_GGB_PEAK_HOUR);
        let total: f64 = s.iter().chain(n.iter()).sum();
        assert!((total - US101_GGB_AADT).abs() < 2.0, "total {total}");
        // the evening peak runs north, at the peak hour's peak-direction share.
        assert!((n[17] - 5_000.0).abs() < 1e-6 && (s[17] - 2_500.0).abs() < 1e-6);
        assert!((s[8] - 5_000.0).abs() < 1e-6);
    }

    #[test]
    fn walkers_walk_only_while_the_walk_is_open() {
        let q = walker_flow();
        assert_eq!(q[4], 0.0);
        assert_eq!(q[21], 0.0);
        let day: f64 = q.iter().sum::<f64>() * 2.0;
        assert!((day - PEDESTRIANS_SEPTEMBER / 30.0).abs() < 1.0);
    }

    #[test]
    fn deck_lanes_lie_inside_the_scaled_roadway() {
        let js = LandmarksJs::get();
        let site = Site::new(&golden_gate::data(js));
        let ca = eo_data::CaStandards::load(&eo_data::DataPaths::discover().expect("repository")).expect("standards");
        let k_width = eo_landmarks::suspension::DeckStandards::load(&ca).expect("deck standards").k_width;
        let d = Deck::new(js, &site, &|_, _| 0.0, k_width);
        // the roadway on the 1.25 ground plan: 18.8976 m x 1.25 = 23.622 m curb to curb.
        let gs = eo_data::groundscale::scale();
        assert!((2.0 * d.curb - 18.8976 * gs).abs() < 1e-6, "roadway {}", 2.0 * d.curb);
        assert_eq!(d.lanes.len(), 3);
        assert!(d.lanes[0] > k_width / 2.0 && *d.lanes.last().unwrap() < d.curb);
        // the walk's middle lies on the walk (between the curb and the deck's edge) away from the towers.
        let s = d.walk_s(d.plan.l / 2.0);
        assert!(s > d.curb && s < d.half_width);
        // and outside the legs at a tower.
        assert!(d.walk_s(0.0) > d.tower_walk[2]);
    }

    /// The real ways on the drawn ground: every vehicle on the bridge rides
    /// the deck (none under it), the lanes keep their sides, and the 16:24
    /// weekday deck carries the counted flow's order of traffic.
    #[test]
    fn bridge_traffic_rides_the_deck_at_the_counted_flow() {
        let paths = eo_data::DataPaths::discover().expect("repository");
        let source = eo_sim::native::load(&paths).expect("sim source");
        let elevation = eo_data::ElevationGrid::load(&paths).expect("elevation");
        let js = LandmarksJs::get();
        let gg = eo_landmarks::set::ground_of(js, "golden-gate-bridge", &|x: f64, z: f64| elevation.sample(x, z));
        let site = Site::new(&golden_gate::data(js));
        let ca = eo_data::CaStandards::load(&paths).expect("standards");
        let k_width = eo_landmarks::suspension::DeckStandards::load(&ca).expect("deck").k_width;
        let deck = Deck::new(js, &site, &|x: f64, z: f64| gg.at(x, z), k_width);
        let paved = |_: f64, _: f64| vec![];
        let ground = |x: f64, z: f64| elevation.ground(x, z);
        let s = Surfaces { deck: Some(&deck), street: false, paved: &paved, ground: &ground };
        let ways = ways(&source, &s, true);
        let names: Vec<&str> = ways.iter().map(|w| w.name.as_str()).collect();
        for n in ["US-101 southbound", "US-101 northbound", "east walk northbound", "east walk southbound"] {
            assert!(names.contains(&n), "{n} missing from {names:?}");
        }
        let gap = eo_sim::Standards::load(&paths).expect("standards").metric("following-time-gap").expect("time gap");
        let mut c = eo_sim::corridor::Corridors::new(ways, eo_sim::corridor::Params::with_time_gap(gap), 1);
        c.hour_handle().store(16.4f64.to_bits(), std::sync::atomic::Ordering::Relaxed);
        for _ in 0..(60 * 30) {
            c.step(1.0 / 60.0, None);
        }
        let (mut north, mut south, mut walkers) = (0usize, 0usize, 0usize);
        let len_km = (deck.plan.c_north - deck.plan.c_south) / 1000.0;
        for o in &c.out {
            let (x, z) = (o.pos[0] as f64, o.pos[2] as f64);
            let cc = site.chainage(x, z);
            // (the last 10 m at each way end ramp to the road beyond it.)
            if cc < deck.plan.c_south + 10.0 || cc > deck.plan.c_north - 10.0 {
                continue;
            }
            let [ax, az] = site.deck_point(cc, 0.0);
            let lat = (x - ax) * site.across[0] + (z - az) * site.across[1];
            let top = deck.plan.deck_top(cc);
            if o.kind >= eo_sim::output::KIND_WALKER_BASE {
                walkers += 1;
                assert!((o.pos[1] as f64 - top - deck.curb_m).abs() < 0.2, "walker off the walk height");
                assert!(lat > deck.curb && lat < deck.tower_walk[3] + 0.5, "walker off the east walk: {lat}");
                continue;
            }
            assert!((o.pos[1] as f64 - top).abs() < 0.25, "vehicle at {} under/over the deck top {top}", o.pos[1]);
            assert!(lat.abs() < deck.curb && lat.abs() > k_width / 2.0, "vehicle off the roadway: {lat}");
            // right-hand traffic: northbound (heading toward the north tower) east of the median.
            let north_bound = (o.heading as f64).sin() * site.axis[0] + (o.heading as f64).cos() * site.axis[1] > 0.0;
            assert_eq!(north_bound, lat > 0.0, "a vehicle on the wrong side");
            if north_bound { north += 1 } else { south += 1 }
        }
        eprintln!("16:24 on the deck ({len_km:.2} km): {north} northbound, {south} southbound, {walkers} walkers");
        // 3 lanes a way at the 3 s gap hold at most about 3 x 1060 vph; at 20 m/s that is
        // about 50 a kilometre a direction; the off-peak direction about 40.
        assert!(north as f64 / len_km > 25.0 && north as f64 / len_km < 70.0, "north {north}");
        assert!(south as f64 / len_km > 20.0 && south as f64 / len_km < 60.0, "south {south}");
        assert!(walkers > 20, "walkers {walkers}");
    }
}
