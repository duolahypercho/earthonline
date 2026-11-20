//! Where bents stand along a deck chain, and the columns each one stands on.
//!
//! A bent line is probed across the deck for room for a column footprint
//! (`BENT_FOOTPRINT_MARGIN_M`). Room is, in the order the rules prefer it:
//! off paving entirely (A1's rule); in the median of a street under the
//! deck; in a parking lane or the kerb-side furniture zone of a street's
//! footway (never a travel lane, a bike lane, a junction pad, a crosswalk or
//! the pedestrian throughway); and, failing all of those under the deck, a
//! straddle bent: columns on both sides of the street outside its travel
//! lanes, up to `OUTRIGGER_REACH_M` past the deck edge, under a cap that
//! spans it. Every column is also kept off any lower chain's roadway (and,
//! past the deck edge, off any other deck of its level).
//!
//! The lateral zones come from the street section the plan already carries:
//! the run's carriageway half width, the road segment's `parking` and
//! kerbside cycle bands, its `median` and lane layout, the drawn footways
//! (`walks`, `walk_yield_spans`), the run's junction trims and crossing kerb
//! ramps; the furniture zone is the footway less the SF frontage zone and
//! throughway (`sidewalk-frontage-zone-sf`, `sidewalk-throughway-zone-sf`)
//! beyond the kerb top (`StreetOptions::curb_top_width`).

use std::collections::HashMap;

use eo_street::StreetWorld;
use eo_street::plan::P2;

use crate::index::{Near, Paved};
use crate::{BENT_FOOTPRINT_MARGIN_M, BentRule, Chain, ColumnSite, MAX_SPAN_M, OUTRIGGER_REACH_M, StreetRef};

/// Bents slide along the chain in steps of this (a search resolution).
const BENT_STEP_M: f64 = 0.5;
/// A bent line is probed for column room this often across the deck (a
/// search resolution; a parking lane leaves under a metre for a column centre).
const BENT_PROBE_M: f64 = 0.25;

/// The kerb-side widths the rules read.
#[derive(Clone, Copy, Debug)]
pub(crate) struct KerbRule {
    /// `StreetOptions::curb_top_width`: the furniture zone starts past the kerb top.
    pub curb_top: f64,
    /// `sidewalk-frontage-zone-sf` (allStreets) and
    /// `sidewalk-throughway-zone-sf` (default): the footway left to the
    /// building frontage and the pedestrian throughway. None when the table
    /// has no verified value: then no furniture zone is used.
    pub frontage: Option<f64>,
    pub through: Option<f64>,
}

/// What a column centre's footprint stands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Site {
    Off,
    Median { entry: u32 },
    Parking { entry: u32, side: u8 },
    Furniture { entry: u32, side: u8 },
}

impl Site {
    pub fn rank(self) -> u8 {
        match self {
            Site::Off => 0,
            Site::Median { .. } => 1,
            Site::Parking { .. } | Site::Furniture { .. } => 2,
        }
    }

    pub fn column_site(self) -> ColumnSite {
        match self {
            Site::Off => ColumnSite::OffPaving,
            Site::Median { .. } => ColumnSite::Median,
            Site::Parking { .. } => ColumnSite::ParkingLane,
            Site::Furniture { .. } => ColumnSite::FurnitureZone,
        }
    }

    fn street(self) -> Option<(u32, u8)> {
        match self {
            Site::Off => None,
            Site::Median { entry } => Some((entry, 0)),
            Site::Parking { entry, side } | Site::Furniture { entry, side } => Some((entry, side)),
        }
    }
}

/// A stretch of a bent line (lateral offsets along the chain's frame normal)
/// where a column centre may stand, and on what.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Zone {
    pub lo: f64,
    pub hi: f64,
    pub site: Site,
    /// The largest column radius the zone holds (a barrier median's width;
    /// infinite elsewhere, where the footprint search radius bounds it).
    pub r_max: f64,
}

/// The room-finding queries over the street plan.
pub(crate) struct Sites<'a> {
    streets: &'a StreetWorld,
    paved: &'a Paved<'a>,
    kerb: KerbRule,
    /// Each run's crosswalk stretches (its crossing kerb ramps with their
    /// flares, either side), stations.
    crosswalks: Vec<Vec<[f64; 2]>>,
    /// Frontage depths asked so far: (run, side, metre of the run).
    fronts: std::cell::RefCell<HashMap<(u32, u8, i64), f64>>,
    /// Stage A6 (only with the street joins, `eo_street::joins`): a column
    /// may stand across two runs' kerbless parking lanes where they meet
    /// (`on_run`).
    pair_parking: bool,
}

impl<'a> Sites<'a> {
    pub fn new(streets: &'a StreetWorld, paved: &'a Paved<'a>, kerb: KerbRule) -> Self {
        let crosswalks = streets
            .plan
            .entries
            .iter()
            .map(|e| {
                e.kerb_ramps
                    .iter()
                    .flatten()
                    .map(|w| {
                        let lo = if w.flare_lo.is_finite() { w.flare_lo.min(w.lo) } else { w.lo };
                        let hi = if w.flare_hi.is_finite() { w.flare_hi.max(w.hi) } else { w.hi };
                        [lo, hi]
                    })
                    .collect()
            })
            .collect();
        Self { streets, paved, kerb, crosswalks, fronts: Default::default(), pair_parking: streets.o.structures.is_some() }
    }

    /// What a column of radius `r` centred at `p` would stand on, or None
    /// where it may not stand (a travel lane, a crosswalk, the throughway,
    /// across two of a street's zones). Off paving, a junction pad (the
    /// reach of the junction's widest approach) is kept clear as A1 did; a
    /// kerb-side site is checked against the run's own junction trims and
    /// crosswalks instead, which bound the junction more tightly along it.
    pub fn site(&self, p: P2, r: f64) -> Option<Site> {
        let mut found = Site::Off;
        for n in self.paved.runs_near(p, f64::INFINITY) {
            let e = &self.streets.plan.entries[n.entry as usize];
            let walk = if self.paved.walk_drawn(n.entry, n.side, n.s) { e.walks[n.side] } else { 0.0 };
            if n.dist - r > e.half + walk + self.frontage(&n, walk) {
                // clear of this run's paving (its carriageway, drawn footway
                // and the frontage paved past it) by r.
                continue;
            }
            let site = self.on_run(&n, walk, r)?;
            if site.rank() > found.rank() {
                found = site;
            }
        }
        if found == Site::Off && self.paved.on_pad(p, r) {
            return None;
        }
        self.drawn_as(p, r, found).then_some(found)
    }

    /// Stage A5's rule for a chain the A1b rules leave a span too long
    /// (`LONGEST_SPAN_M`): as `site`, but a freeway node's pad disc is not
    /// kept clear (the street as drawn decides there: a shallow ramp's
    /// trim makes the disc hundreds of metres wide over open ground), and a
    /// column of radius `r_least` may stand in a divided freeway's barrier
    /// median (`barrier_median`). (site, the largest column radius it holds).
    pub fn site_relaxed(&self, p: P2, r: f64, r_least: f64) -> Option<(Site, f64)> {
        let strict = (|| {
            let mut found = Site::Off;
            for n in self.paved.runs_near(p, f64::INFINITY) {
                let e = &self.streets.plan.entries[n.entry as usize];
                let walk = if self.paved.walk_drawn(n.entry, n.side, n.s) { e.walks[n.side] } else { 0.0 };
                if n.dist - r > e.half + walk + self.frontage(&n, walk) {
                    continue;
                }
                let site = self.on_run(&n, walk, r)?;
                if site.rank() > found.rank() {
                    found = site;
                }
            }
            self.drawn_as(p, r, found).then_some(found)
        })();
        match strict {
            Some(s) => Some((s, f64::INFINITY)),
            None => self.barrier_median(p, r_least, r),
        }
    }

    /// A column centred at `p` in the median of a divided freeway: between
    /// two one-way controlled-access runs carrying opposite directions side
    /// by side (within `route::ALONG_DEG`), off both carriageways (where the
    /// real road's Type 60 median barrier stands and wraps the column), off
    /// every other run's paving and off the drawn surface. Returns
    /// the site and the largest radius the median holds there (at least
    /// `r`), or None.
    fn barrier_median(&self, p: P2, r: f64, wide: f64) -> Option<(Site, f64)> {
        let near = self.paved.runs_near(p, f64::INFINITY);
        // the runs flanking it: within A1b's footprint (`wide`) of their paving.
        let mut sides: Vec<(&crate::index::Near, [f64; 2], f64)> = vec![];
        for n in &near {
            let e = &self.streets.plan.entries[n.entry as usize];
            let walk = if self.paved.walk_drawn(n.entry, n.side, n.s) { e.walks[n.side] } else { 0.0 };
            let reach = e.half + walk + self.frontage(n, walk);
            if n.dist - wide > reach {
                continue;
            }
            // only a freeway carriageway may come this close, and not over the column.
            if !crate::index::freeway_class(&e.highway) || !e.oneway || n.dist - r < e.half - 1e-9 || reach > e.half + 1e-9 {
                return None;
            }
            let f = e.frame(n.s, false);
            sides.push((n, [f.tx, f.tz], n.dist - e.half));
        }
        // two runs, one either side of the column, carrying opposite directions.
        let dir_of = |n: &crate::index::Near| -> f64 {
            let e = &self.streets.plan.entries[n.entry as usize];
            let seg = &self.streets.city.segments[e.road];
            let d = seg.lane_layout.as_ref().and_then(|l| l.iter().find(|x| x.motor && x.dir != 0)).map(|x| x.dir as f64);
            d.unwrap_or(if seg.oneway == "decreasing" { -1.0 } else { 1.0 })
        };
        let cos = crate::route::ALONG_DEG.to_radians().cos();
        let mut room = None;
        for (i, a) in sides.iter().enumerate() {
            for b in sides.iter().skip(i + 1) {
                let dot = a.1[0] * b.1[0] + a.1[1] * b.1[1];
                if dot.abs() < cos {
                    continue;
                }
                // travel directions opposite, and the column between the two centrelines.
                let opposite = dir_of(a.0) * dir_of(b.0) * dot < 0.0;
                let between = if dot > 0.0 { a.0.side != b.0.side } else { a.0.side == b.0.side };
                if opposite && between {
                    room = Some(a.2.min(b.2));
                }
            }
        }
        let r_max = room?;
        if r_max < r || !self.drawn_as(p, r, Site::Off) {
            return None;
        }
        let entry = sides.iter().map(|s| s.0.entry).min()?;
        Some((Site::Median { entry }, r_max))
    }

    /// The street as drawn agrees with a site at `p`: every sample of the
    /// footprint (its centre and eight points round its rim) is off the
    /// paving (or on a verge bank) for `Off`, on the carriageway for a
    /// parking lane or a median, on the footway for a furniture zone
    /// (`StreetWorld::paved_layer_at`; paint counts as what it is painted on).
    fn drawn_as(&self, p: P2, r: f64, site: Site) -> bool {
        use eo_street::Layer;
        (0..=8).all(|k| {
            let (x, z) = if k == 8 {
                (p[0], p[1])
            } else {
                let a = k as f64 * std::f64::consts::TAU / 8.0;
                (p[0] + r * a.cos(), p[1] + r * a.sin())
            };
            let layer = self.streets.paved_layer_at(x, z);
            match site {
                Site::Off => matches!(layer, None | Some(Layer::Verge)),
                Site::Parking { .. } | Site::Median { .. } => matches!(layer, Some(Layer::Carriageway | Layer::Marking)),
                Site::Furniture { .. } => matches!(layer, Some(Layer::Sidewalk | Layer::CurbTop | Layer::Marking)),
            }
        })
    }

    /// How far the street paves its frontage past the footway of run `n` on
    /// its side there (`StreetWorld::frontage_depth`: to a facade within
    /// reach, else to the surveyed property line), kept per metre of the run.
    fn frontage(&self, n: &Near, walk: f64) -> f64 {
        if walk <= 0.0 {
            return 0.0;
        }
        let key = (n.entry, n.side as u8, n.s.floor() as i64);
        if let Some(&v) = self.fronts.borrow().get(&key) {
            return v;
        }
        let e = &self.streets.plan.entries[n.entry as usize];
        let f = e.frame(n.s.floor() + 0.5, false);
        let side = if n.side == 0 { 1.0 } else { -1.0 };
        let v = self.streets.frontage_depth(n.entry as usize, &f, side, walk);
        self.fronts.borrow_mut().insert(key, v);
        v
    }

    /// The footprint `[dist - r, dist + r]` across run `n` lies wholly in one
    /// of its kerb-side zones (or its median), away from its junction mouths
    /// and crosswalks.
    fn on_run(&self, n: &Near, walk: f64, r: f64) -> Option<Site> {
        let e = &self.streets.plan.entries[n.entry as usize];
        if n.s < e.s0() + r || n.s > e.s1() - r || e.in_yield_span(n.s) {
            return None;
        }
        if self.crosswalks[n.entry as usize].iter().any(|w| n.s >= w[0] - r && n.s <= w[1] + r) {
            return None;
        }
        let seg = &self.streets.city.segments[e.road];
        let k = n.side;
        let (lo, hi) = (n.dist - r, n.dist + r);
        let side = k as u8;
        // the parking lane: at the kerb, or inside a kerbside cycle lane.
        let park = seg.parking[k].max(0.0);
        if park > 0.0 {
            let kc = if seg.cycle_kerbside[k] { seg.cycle[k].max(0.0) + seg.cycle_buffer[k].max(0.0) } else { 0.0 };
            if lo >= e.half - kc - park && hi <= e.half - kc {
                return Some(Site::Parking { entry: n.entry, side });
            }
            // stage A6: where two runs' carriageways overlap with no kerb
            // between them (a one-way pair drawn each with a parking lane on
            // its inner side, as 13th Street under the Central Freeway), the
            // part of the footprint on this run lies in its parking lane and
            // the rest runs on past its kerbless edge, onto the other run's
            // carriageway: that run answers for it, and the drawn street must
            // be carriageway under every sample (`drawn_as`).
            if self.pair_parking && kc <= 0.0 && walk <= 0.0 && lo >= e.half - park && lo < e.half && hi > e.half {
                return Some(Site::Parking { entry: n.entry, side });
            }
        }
        // the furniture zone: past the kerb top, short of the throughway.
        if let (Some(f), Some(t)) = (self.kerb.frontage, self.kerb.through) {
            if walk > 0.0 && lo >= e.half + self.kerb.curb_top && hi <= e.half + walk - f - t {
                return Some(Site::Furniture { entry: n.entry, side });
            }
        }
        // the median, centred on the boundary between the two directions.
        if seg.median > 0.0 {
            if let Some(b) = direction_boundary(seg) {
                let v = if k == 0 { n.dist } else { -n.dist };
                if v - r >= b - seg.median / 2.0 && v + r <= b + seg.median / 2.0 {
                    return Some(Site::Median { entry: n.entry });
                }
            }
        }
        None
    }

    /// The street a site stands in.
    pub fn street_ref(&self, site: Site) -> Option<StreetRef> {
        let (entry, side) = site.street()?;
        let e = &self.streets.plan.entries[entry as usize];
        let seg = &self.streets.city.segments[e.road];
        Some(StreetRef { entry, id: e.id.clone(), name: seg.street_name.clone(), side })
    }

    /// The zones across a bent line at station `s` of `c`, from `u0` to `u1`
    /// (lateral offsets), for a column of radius `r`; `free(p, outside)`
    /// says a point is off every chain the column may not stand under
    /// (`outside`: past the deck edge).
    pub fn zones(&self, c: &Chain, s: f64, u0: f64, u1: f64, r: f64, half: f64, free: &dyn Fn(P2, bool) -> bool) -> Vec<Zone> {
        self.zones_by(c, s, u0, u1, &|p, u| if free(p, u.abs() > half) { self.site(p, r).map(|st| (st, f64::INFINITY)) } else { None })
    }

    /// The zones across a bent line at station `s` of `c` from `u0` to `u1`,
    /// `site_of(point, offset)` saying what a column centre there stands on
    /// and the largest radius it holds.
    pub fn zones_by(&self, c: &Chain, s: f64, u0: f64, u1: f64, site_of: &dyn Fn(P2, f64) -> Option<(Site, f64)>) -> Vec<Zone> {
        let f = c.frame(s);
        let n = (((u1 - u0) / BENT_PROBE_M).ceil() as usize).max(1);
        let mut out: Vec<Zone> = vec![];
        let mut prev_u = u0;
        for k in 0..=n {
            let u = u0 + (u1 - u0) * k as f64 / n as f64;
            let p = [f.x + f.nx * u, f.z + f.nz * u];
            let site = site_of(p, u);
            match (site, out.last_mut()) {
                (Some((st, rm)), Some(z)) if z.site == st && (z.hi - prev_u).abs() < 1e-9 => {
                    z.hi = u;
                    z.r_max = z.r_max.min(rm);
                }
                (Some((st, rm)), _) => out.push(Zone { lo: u, hi: u, site: st, r_max: rm }),
                (None, _) => {}
            }
            prev_u = u;
        }
        out
    }
}

/// The lateral offset (+n positive, the segment's own frame) of the line
/// between a road's two directions of travel, from its lane layout.
fn direction_boundary(seg: &eo_street::city::Segment) -> Option<f64> {
    let layout = seg.lane_layout.as_ref()?;
    let mut moving: Vec<(i8, f64)> = layout.iter().filter(|l| l.motor).map(|l| (l.dir, -l.lateral)).collect();
    moving.sort_by(|a, b| a.1.total_cmp(&b.1));
    moving.windows(2).find(|w| w[0].0 != w[1].0 && w[0].0 != 0 && w[1].0 != 0).map(|w| (w[0].1 + w[1].1) / 2.0)
}

/// What one bent line offers: the best rule and its zones.
#[derive(Clone, Debug)]
pub(crate) struct Room {
    pub rule: BentRule,
    pub zones: Vec<Zone>,
    /// A bent shared with a parallel twin deck of the same route (stage
    /// A5): its line crosses the twin's centreline at this offset (the zones
    /// span both decks), and the twin's station there.
    pub twin: Option<TwinLine>,
}

/// Where a shared bent's line crosses the twin deck.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TwinLine {
    pub chain: u32,
    /// The twin's centreline along this bent's line (lateral offset).
    pub u: f64,
    /// The twin's station there.
    pub s: f64,
    /// The twin's half width to its fascia (roadway and barrier).
    pub half: f64,
    /// This chain's bent is carried by the twin's cap (no columns of its own).
    pub carried: bool,
}

/// The rule a set of zones gives, the zones of that rule kept.
fn by_rule(inner: Vec<Zone>) -> Option<(BentRule, Vec<Zone>)> {
    let best = inner.iter().map(|z| z.site.rank()).min()?;
    let rule = match best {
        0 => BentRule::OffPaving,
        1 => BentRule::Median,
        _ => BentRule::Kerbside,
    };
    Some((rule, inner.into_iter().filter(|z| z.site.rank() <= best).collect()))
}

/// The best room at station `s` (None: no column can stand there).
fn room(sites: &Sites, c: &Chain, s: f64, free: &dyn Fn(P2, bool) -> bool) -> Option<Room> {
    let r = BENT_FOOTPRINT_MARGIN_M;
    let half = c.half_at(s);
    let h = half + r;
    let inner = sites.zones(c, s, -h, h, r, h, free);
    if let Some((rule, zones)) = by_rule(inner) {
        return Some(Room { rule, zones, twin: None });
    }
    // a straddle: room on both sides of the deck, out to the outrigger's reach.
    let reach = h + OUTRIGGER_REACH_M;
    let outer = sites.zones(c, s, -reach, reach, r, h, free);
    let left = outer.iter().any(|z| z.hi < 0.0);
    let right = outer.iter().any(|z| z.lo > 0.0);
    (left && right).then_some(Room { rule: BentRule::Straddle, zones: outer, twin: None })
}

/// `room` by stage A5's relaxed rule (`Sites::site_relaxed`): freeway pads
/// judged by the drawn street, barrier medians (a column of `r_least`).
pub(crate) fn room_relaxed(sites: &Sites, c: &Chain, s: f64, r_least: f64, free: &dyn Fn(P2, bool) -> bool) -> Option<Room> {
    let r = BENT_FOOTPRINT_MARGIN_M;
    let half = c.half_at(s);
    let h = half + r;
    let site_of = |p: P2, u: f64| if free(p, u.abs() > h) { sites.site_relaxed(p, r, r_least) } else { None };
    if let Some((rule, zones)) = by_rule(sites.zones_by(c, s, -h, h, &site_of)) {
        return Some(Room { rule, zones, twin: None });
    }
    let reach = h + OUTRIGGER_REACH_M;
    let outer = sites.zones_by(c, s, -reach, reach, &site_of);
    let left = outer.iter().any(|z| z.hi < 0.0);
    let right = outer.iter().any(|z| z.lo > 0.0);
    (left && right).then_some(Room { rule: BentRule::Straddle, zones: outer, twin: None })
}

/// A bent line shared with a twin deck (`TwinLine`): room across both decks
/// and the gap between them (`free(point, offset)` says a point is clear of
/// every other chain a column may not stand under), by the relaxed rule.
pub(crate) fn room_shared(sites: &Sites, c: &Chain, s: f64, twin: TwinLine, r_least: f64, free: &dyn Fn(P2, f64) -> bool) -> Option<Room> {
    let r = BENT_FOOTPRINT_MARGIN_M;
    let h = c.half_at(s) + r;
    let (u0, u1) = if twin.u < 0.0 { (twin.u - twin.half - r, h) } else { (-h, twin.u + twin.half + r) };
    let site_of = |p: P2, u: f64| if free(p, u) { sites.site_relaxed(p, r, r_least) } else { None };
    if let Some((rule, zones)) = by_rule(sites.zones_by(c, s, u0, u1, &site_of)) {
        return Some(Room { rule, zones, twin: Some(twin) });
    }
    // a straddle over both decks: room either side of them, out to the outrigger's reach.
    let outer = sites.zones_by(c, s, u0 - OUTRIGGER_REACH_M, u1 + OUTRIGGER_REACH_M, &site_of);
    let left = outer.iter().any(|z| z.hi < u0 + r);
    let right = outer.iter().any(|z| z.lo > u1 - r);
    (left && right).then_some(Room { rule: BentRule::Straddle, zones: outer, twin: Some(twin) })
}

/// Bents along a deck chain: equal-division targets at `MAX_SPAN_M`, each
/// slid to the nearest station within half a span whose room is best by
/// the rules' order, else to the next station with any room (the span
/// lengthens). Returns the bent stations and their room.
pub(crate) fn place(sites: &Sites, c: &Chain, free: &dyn Fn(P2, bool) -> bool) -> Vec<(f64, Room)> {
    place_by(c, &mut |s| room(sites, c, s, free), &[])
}

/// `place` with the room at a station from `room_of`, and with bents
/// already fixed at `fixed` (stations, ascending, with their room: a twin's
/// shared bents): the targets are placed between them and the ends.
pub(crate) fn place_by(c: &Chain, room_of: &mut dyn FnMut(f64) -> Option<Room>, fixed: &[(f64, Room)]) -> Vec<(f64, Room)> {
    let len = c.length();
    let span = MAX_SPAN_M;
    let mut cache: HashMap<i64, Option<Room>> = HashMap::new();
    let mut room_at = |s: f64| -> Option<Room> {
        let key = (s / BENT_STEP_M * 64.0).round() as i64;
        cache.entry(key).or_insert_with(|| room_of(s)).clone()
    };
    let mut out: Vec<(f64, Room)> = vec![];
    let mut ends: Vec<f64> = fixed.iter().map(|f| f.0).collect();
    ends.push(len);
    let mut start = 0.0;
    for (seg, &end) in ends.iter().enumerate() {
        let mut prev = start;
        loop {
            let remaining = end - prev;
            let k = (remaining / span).ceil();
            if k <= 1.0 {
                break;
            }
            let t = prev + remaining / k;
            let (lo, hi) = ((t - span / 2.0).max(prev + span / 2.0), (t + span / 2.0).min(end - span / 2.0));
            let steps = ((hi - lo).max(0.0) / BENT_STEP_M).ceil() as i64 + 1;
            let mut found: Option<(f64, Room)> = None;
            // the best rule anywhere in the window first, then the nearest to t
            // (the earlier one on a tie).
            'rules: for rule in BentRule::ALL {
                for j in 0..=steps {
                    for s in [t - j as f64 * BENT_STEP_M, t + j as f64 * BENT_STEP_M] {
                        if s < lo - 1e-9 || s > hi + 1e-9 {
                            continue;
                        }
                        if let Some(rm) = room_at(s).filter(|rm| rm.rule <= rule) {
                            found = Some((s, rm));
                            break 'rules;
                        }
                    }
                }
            }
            if found.is_none() {
                // none within half a span: the span lengthens to the next station with room.
                let mut s = hi + BENT_STEP_M;
                while s <= end - span / 2.0 {
                    if let Some(rm) = room_at(s) {
                        found = Some((s, rm));
                        break;
                    }
                    s += BENT_STEP_M;
                }
            }
            match found {
                Some((s, rm)) => {
                    prev = s;
                    out.push((s, rm));
                }
                None => break,
            }
        }
        if let Some(f) = fixed.get(seg) {
            out.push(f.clone());
        }
        start = end;
    }
    out
}

/// How many columns a deck `width` wide (barriers included) stands on.
pub(crate) fn column_count(width: f64) -> usize {
    if width <= crate::ONE_COLUMN_DECK_MAX_M {
        1
    } else if width <= crate::TWO_COLUMN_DECK_MAX_M {
        2
    } else {
        3
    }
}

/// The columns of one bent: `count` wanted, at their ideal places across a
/// deck `width` wide centred at offset `centre` (one centred; two at +-0.3
/// of the width; three at the centre and +-0.36), each moved to the nearest
/// point of the bent's room; a straddle bent stands on the innermost room
/// either side. Columns closer than their diameter plus `gap` are merged.
/// (lateral offset, site, the largest radius its zone holds), by offset.
pub(crate) fn column_places(room: &Room, count: usize, width: f64, centre: f64, r: f64, gap: f64) -> Vec<(f64, Site, f64)> {
    let zones = &room.zones;
    if zones.is_empty() {
        return vec![];
    }
    // (offset, site, distance moved, radius held)
    let nearest = |u: f64| -> (f64, Site, f64, f64) {
        let mut best = (0.0, zones[0].site, f64::INFINITY, zones[0].r_max);
        for z in zones {
            let v = u.clamp(z.lo, z.hi);
            let d = (v - u).abs();
            if d < best.2 {
                best = (v, z.site, d, z.r_max);
            }
        }
        best
    };
    let mut picks: Vec<(f64, Site, f64, f64)> = if room.rule == BentRule::Straddle {
        let l = zones.iter().filter(|z| z.hi < centre).max_by(|a, b| a.hi.total_cmp(&b.hi)).map(|z| (z.hi, z.site, 0.0, z.r_max));
        let rt = zones.iter().filter(|z| z.lo > centre).min_by(|a, b| a.lo.total_cmp(&b.lo)).map(|z| (z.lo, z.site, 0.0, z.r_max));
        l.into_iter().chain(rt).collect()
    } else {
        let ideals: Vec<f64> = match count {
            1 => vec![0.0],
            2 => vec![-0.3 * width, 0.3 * width],
            _ => vec![-0.36 * width, 0.0, 0.36 * width],
        };
        ideals.into_iter().map(|u| nearest(centre + u)).collect()
    };
    picks.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<(f64, Site, f64, f64)> = vec![];
    for p in picks {
        match out.last_mut() {
            Some(q) if p.0 - q.0 < 2.0 * r + gap => {
                if p.2 < q.2 {
                    *q = p;
                }
            }
            _ => out.push(p),
        }
    }
    out.into_iter().map(|(u, s, _, rm)| (u, s, rm)).collect()
}
