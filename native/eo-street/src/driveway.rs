//! A service drive can end on a through street's sidewalk without a shared
//! junction node. Fit the last drawn cross-section to that paving across its
//! whole width. The two wheel tracks can be at different heights on a hill.
use crate::{EntryGeom, StreetOptions, StreetWorld, SurfaceTile, TileMesh, tile_key};

#[derive(Clone, Debug)]
pub(crate) struct Apron {
    mouth: f64,
    approach: f64,
    // Signed offsets from the centreline, and height differences at the mouth.
    section: Vec<(f64, f64)>,
}

impl Apron {
    fn weight(&self, s: f64) -> f64 {
        let t = ((s - self.approach) / (self.mouth - self.approach)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    fn correction(&self, s: f64, u: f64) -> f64 {
        let weight = self.weight(s);
        if weight == 0.0 {
            return 0.0;
        }
        let i = self
            .section
            .partition_point(|v| v.0 < u)
            .clamp(1, self.section.len() - 1);
        let [(a, da), (b, db)] = [self.section[i - 1], self.section[i]];
        weight * (da + (db - da) * ((u - a) / (b - a)).clamp(0.0, 1.0))
    }
}

/// Include each boundary of a receiving triangle that the mouth crosses.
/// On a hillside the two sides of a driveway can meet different planes.
fn profile_offsets(mesh: &TileMesh, mouth: crate::plan::Frame, mut offsets: Vec<f64>) -> Vec<f64> {
    let left = offsets[0];
    let right = *offsets.last().unwrap();
    let n = [mouth.nx * mouth.miter, mouth.nz * mouth.miter];
    let cross = |a: [f64; 2], b: [f64; 2]| a[0] * b[1] - a[1] * b[0];
    for (indices, layer) in mesh.indices.chunks_exact(3).zip(&mesh.layers) {
        if !layer.walkable() {
            continue;
        }
        for i in 0..3 {
            let a = mesh.positions[indices[i] as usize].map(f64::from);
            let b = mesh.positions[indices[(i + 1) % 3] as usize].map(f64::from);
            let edge = [b[0] - a[0], b[2] - a[2]];
            let den = cross(n, edge);
            if den.abs() <= f64::EPSILON {
                continue;
            }
            let delta = [a[0] - mouth.x, a[2] - mouth.z];
            let t = cross(delta, n) / den;
            let u = cross(delta, edge) / den;
            if (0.0..=1.0).contains(&t) && left < u && u < right {
                offsets.push(u);
            }
        }
    }
    offsets.sort_by(f64::total_cmp);
    offsets.dedup_by(|a, b| (*a - *b).abs() < 1e-7);
    offsets
}

#[derive(Clone, Debug)]
pub(crate) struct Driveways {
    aprons: Vec<Apron>,
    offsets: Vec<f64>,
}

impl Driveways {
    fn new(aprons: Vec<Apron>) -> Self {
        let mut offsets: Vec<_> = aprons
            .iter()
            .flat_map(|a| a.section.iter().map(|p| p.0))
            .collect();
        offsets.sort_by(f64::total_cmp);
        offsets.dedup_by(|a, b| (*a - *b).abs() < 1e-7);
        Self { aprons, offsets }
    }
}

impl EntryGeom {
    pub(crate) fn section_offsets(
        &self,
        o: &StreetOptions,
        half: f64,
    ) -> std::borrow::Cow<'_, [f64]> {
        match &self.driveways {
            Some(d) => std::borrow::Cow::Borrowed(&d.offsets),
            None => std::borrow::Cow::Owned(o.section_offsets(half)),
        }
    }

    pub(crate) fn crosses_apron(&self, i: usize, j: usize) -> bool {
        self.driveways.as_ref().is_some_and(|d| {
            d.aprons.iter().any(|a| {
                self.stations[i].s < a.approach.max(a.mouth)
                    && self.stations[j].s > a.approach.min(a.mouth)
            })
        })
    }

    pub(crate) fn pavement_y(&self, o: &StreetOptions, i: usize, u: f64, half: f64) -> f64 {
        let y = o.cross_section_y(self.datums[i], u, half);
        let Some(aprons) = &self.driveways else {
            return y;
        };
        y + aprons
            .aprons
            .iter()
            .map(|a| a.correction(self.stations[i].s, u))
            .sum::<f64>()
    }
}

impl StreetWorld {
    pub(crate) fn fit_driveways(&mut self) {
        self.driveway_openings.clear();
        for (ei, e) in self.plan.entries.iter().enumerate() {
            if e.driveway_owners == [None, None] {
                continue;
            }
            let mut aprons = Vec::new();
            for (end, owner) in e.driveway_owners.iter().enumerate() {
                if let Some(apron) = owner.and_then(|owner| self.driveway_apron(ei, end, owner)) {
                    aprons.push(apron);
                }
            }
            // The two ends of a short drive may both need a transition.
            // Do not impose contradictory grades on the same piece of road.
            self.geoms[ei].driveways = match aprons.as_slice() {
                [] => None,
                [start, end] if start.approach > end.approach => None,
                _ => Some(Box::new(Driveways::new(aprons))),
            };
        }
        for (ei, e) in self.plan.entries.iter().enumerate() {
            let g = &self.geoms[ei];
            let Some(fitted) = &g.driveways else {
                continue;
            };
            for (end, owner) in e.driveway_owners.iter().enumerate() {
                let Some(owner) = *owner else { continue };
                let road = &self.plan.entries[owner];
                if road.highway == "service" || self.city.segments[road.road].separated {
                    continue;
                }
                let mut spans = 0..g.keep.len();
                let terminal = if end == 0 {
                    spans.find(|&i| g.keep[i])
                } else {
                    spans.rfind(|&i| g.keep[i])
                };
                let Some(i) = terminal else { continue };
                let mouth = g.stations[i + end].s;
                if !fitted.aprons.iter().any(|a| a.mouth == mouth) {
                    continue;
                }
                let reach = road.half + road.walks[0].max(road.walks[1]) + e.half;
                let open = self.driveway_openings.entry(owner).or_default();
                let section = |j: usize| {
                    [-e.half, e.half].map(|u| {
                        let [x, z] = g.stations[j].at(u);
                        [x, g.pavement_y(&self.o, j, u, e.half), z]
                    })
                };
                for j in 0..g.keep.len() {
                    if g.keep[j]
                        && g.stations[j].s <= mouth + reach
                        && g.stations[j + 1].s >= mouth - reach
                    {
                        open.add(
                            ei as u32,
                            section(j),
                            section(j + 1),
                            self.o.curb_face_height,
                            self.o.driveway.clearance,
                        );
                    }
                }
            }
        }
    }

    fn driveway_apron(&self, ei: usize, end: usize, owner: usize) -> Option<Apron> {
        let e = &self.plan.entries[ei];
        let g = &self.geoms[ei];
        let receiver = &self.plan.entries[owner];
        if receiver.highway == "service" || self.city.segments[receiver.road].separated {
            return None;
        }
        let span = if end == 0 {
            g.keep.iter().position(|&kept| kept)?
        } else {
            g.keep.iter().rposition(|&kept| kept)?
        };
        let i = span + end;
        let mouth = g.stations[i];
        let boundary = if end == 0 {
            e.yield_spans.first()?[1]
        } else {
            e.yield_spans.last()?[0]
        };
        if (mouth.s - boundary).abs() > 1e-4 {
            return None;
        }

        // Use only the receiver's hard paving. Reading the finished tile
        // here would include the driveway itself and its earth shoulders.
        let key = tile_key(mouth.x, mouth.z);
        let mut mesh = TileMesh::default();
        for dx in -1..=1 {
            for dz in -1..=1 {
                let Some(refs) = self.tiles.get(&(key.0 + dx, key.1 + dz)) else {
                    continue;
                };
                for &(entry, j) in &refs.spans {
                    if entry as usize == owner {
                        self.emit_span(&mut mesh, owner, j as usize, false);
                    }
                }
            }
        }
        let paving = SurfaceTile::build(&[&mesh], key);
        let mut section = Vec::new();
        let mut mismatch = 0.0_f64;
        let mut offsets = profile_offsets(&mesh, mouth, self.o.section_offsets(e.half));
        let mut shoulders = Vec::new();
        let precision = 4.0 * f64::from(f32::EPSILON) * mouth.x.abs().max(mouth.z.abs()).max(1.0);
        for pair in offsets.windows(2) {
            let [a, b] = [pair[0], pair[1]];
            if b - a <= precision * 2.0 {
                continue;
            }
            // An intersected vertical face has two limiting elevations at
            // the same plan coordinate. Fit each adjoining plane from its
            // own side rather than spreading the lip across the traffic lane.
            for (edge, inside) in [(a, a + precision), (b, b - precision)] {
                let [x, z] = mouth.at(edge);
                let [px, pz] = mouth.at(inside);
                if (paving.height(x, z)? - paving.height(px, pz)?).abs() > self.o.marking_lift {
                    shoulders.push(inside);
                }
            }
        }
        offsets.extend(shoulders);
        offsets.sort_by(f64::total_cmp);
        for u in offsets {
            let [x, z] = mouth.at(u);
            let base = self.o.cross_section_y(g.datums[i], u, e.half);
            let adjustment = paving.height(x, z)? - base;
            mismatch = mismatch.max(adjustment.abs());
            section.push((u, adjustment));
        }
        if mismatch < self.o.marking_lift {
            return None;
        }

        // Smoothstep's largest derivative is 1.5. Fit the transition inside
        // this contiguous approach at the configured ramp running slope.
        let run = (1.5 * mismatch / self.o.ramp_running_slope_max).max(self.o.max_step);
        let far = if end == 0 {
            span + g.keep[span..].iter().take_while(|&&keep| keep).count()
        } else {
            i - g.keep[..i].iter().rev().take_while(|&&keep| keep).count()
        };
        let available = g.stations[far].s - mouth.s;
        if run > available.abs() {
            return None;
        }
        Some(Apron {
            mouth: mouth.s,
            approach: mouth.s + run.copysign(available),
            section,
        })
    }
}

#[cfg(test)]
mod tests;

#[derive(Clone, Debug)]
pub(crate) struct Dimensions {
    pub lip: f64,
    pub depth: f64,
    flare: f64,
    clearance: f64,
}

/// Splitting a kerb to form its opening must preserve the receiving street's
/// existing grade and footprint, including the miter at a bend.
pub(crate) fn insert_curb_stations(
    cuts: &[Vec<crate::plan::RampWin>; 2],
    stations: &mut Vec<crate::plan::Frame>,
    datums: &mut Vec<f64>,
) {
    let mut splits: Vec<_> = cuts
        .iter()
        .flatten()
        .flat_map(|w| [w.flare_lo, w.lo, w.hi, w.flare_hi])
        .collect();
    if splits.is_empty() {
        return;
    }
    splits.sort_by(f64::total_cmp);
    splits.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
    let mut frames = Vec::with_capacity(stations.len() + splits.len());
    let mut heights = Vec::with_capacity(frames.capacity());
    for (i, pair) in stations.windows(2).enumerate() {
        let (a, b) = (pair[0], pair[1]);
        frames.push(a);
        heights.push(datums[i]);
        for &s in &splits[splits.partition_point(|s| *s <= a.s + 1e-4)..] {
            if s >= b.s - 1e-4 {
                break;
            }
            let t = (s - a.s) / (b.s - a.s);
            frames.push(a.split(b, s));
            heights.push(datums[i] + t * (datums[i + 1] - datums[i]));
        }
    }
    frames.push(*stations.last().unwrap());
    heights.push(*datums.last().unwrap());
    *stations = frames;
    *datums = heights;
}

impl Dimensions {
    pub(crate) fn load(ca: &eo_data::CaStandards) -> anyhow::Result<Self> {
        Ok(Self {
            lip: ca.metric_key("driveway-apron-sf", "lip")?,
            depth: ca.metric_key("driveway-apron-sf", "apronDepthApprox")?,
            flare: ca.metric_key("driveway-apron-sf", "flareEachSide")?,
            clearance: ca.metric("structure-vertical-clearance-local")?,
        })
    }
}

/// Portion of a kerb segment lying inside the surveyed service drive.
fn cut_interval(a: [f64; 2], b: [f64; 2], quad: &[[f64; 2]; 4]) -> Option<[f64; 2]> {
    let mut t = [0.0_f64, 1.0_f64];
    for i in 0..4 {
        let p = quad[i];
        let q = quad[(i + 1) % 4];
        let side = |v: [f64; 2]| (q[0] - p[0]) * (v[1] - p[1]) - (q[1] - p[1]) * (v[0] - p[0]);
        let (u, v) = (side(a), side(b));
        if u < 0. && v < 0. {
            return None;
        }
        if u < 0. {
            t[0] = t[0].max(u / (u - v));
        }
        if v < 0. {
            t[1] = t[1].min(u / (u - v));
        }
    }
    (t[0] < t[1]).then_some(t)
}

pub(crate) fn plan_curb_cuts(
    entries: &mut [crate::plan::Entry],
    city: &crate::StreetCity,
    o: &StreetOptions,
) {
    use crate::plan::{RampWin, nearest_station};
    let mut cuts = Vec::new();
    let mut mouths = Vec::new();
    for (ei, drive) in entries.iter().enumerate() {
        for (end, owner) in drive.driveway_owners.iter().enumerate() {
            let Some(owner) = *owner else { continue };
            let road = &entries[owner];
            let seg = &city.segments[road.road];
            if road.highway == "service"
                || seg.separated
                || eo_data::standards::is_sf_state_route(&road.highway, &seg.street_name)
            {
                continue;
            }
            let interval = if end == 0 {
                drive.yield_spans[0]
            } else {
                *drive.yield_spans.last().unwrap()
            };
            if let Some(s) = landing_station(drive, road, end, interval, o) {
                mouths.push((ei, end, s));
            }
            let tip = if end == 0 {
                drive.points[0]
            } else {
                *drive.points.last().unwrap()
            };
            let (onto, distance) = nearest_station(road, tip);
            if distance >= road.half || onto < road.s0() || onto > road.s1() {
                continue;
            }
            let stations = crate::plan::build_stations(
                &drive.points,
                &drive.cum,
                interval[0],
                interval[1],
                o.max_step,
            );
            for side in 0..2 {
                if road.walks[side] < o.min_sidewalk_width {
                    continue;
                }
                let lateral = if side == 0 { road.half } else { -road.half };
                for road_span in road.cum.windows(2) {
                    let (lo, hi) = (road_span[0].max(road.s0()), road_span[1].min(road.s1()));
                    if hi <= lo {
                        continue;
                    }
                    let a = road.frame(lo, true).at(lateral);
                    let b = road.frame(hi, true).at(lateral);
                    for pair in stations.windows(2) {
                        let quad = [
                            pair[0].at(-drive.half),
                            pair[1].at(-drive.half),
                            pair[1].at(drive.half),
                            pair[0].at(drive.half),
                        ];
                        if let Some([from, to]) = cut_interval(a, b, &quad) {
                            cuts.push((owner, side, [lo + from * (hi - lo), lo + to * (hi - lo)]));
                        }
                    }
                }
            }
        }
    }
    for (ei, end, s) in mouths {
        let e = &mut entries[ei];
        let i = if end == 0 { 0 } else { e.yield_spans.len() - 1 };
        e.yield_spans[i][1 - end] = s;
    }
    cuts.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)).then(a.2[0].total_cmp(&b.2[0])));
    for (owner, side, [lo, hi]) in cuts {
        let road = &mut entries[owner];
        let (begin, end) = (road.s0(), road.s1());
        let wins = &mut road.driveway_cuts[side];
        if let Some(last) = wins.last_mut() {
            if lo <= last.hi + 1e-6 {
                last.hi = last.hi.max(hi);
                last.flare_hi = (last.hi + o.driveway.flare).min(end);
                continue;
            }
        }
        wins.push(RampWin {
            lo,
            hi,
            flare_lo: (lo - o.driveway.flare).max(begin),
            flare_hi: (hi + o.driveway.flare).min(end),
            node: usize::MAX,
            at_start: false,
        });
    }
}

/// Place the seam on a single paved strip. A skew driveway can have its
/// centre over the walk while one edge is in the gutter or beyond the walk.
/// Keep the surveyed curb lip on the receiving street, clear of the seam.
fn landing_station(
    drive: &crate::plan::Entry,
    road: &crate::plan::Entry,
    end: usize,
    interval: [f64; 2],
    o: &StreetOptions,
) -> Option<f64> {
    let mouth = interval[1 - end];
    let (lo, hi) = (drive.s0(), drive.s1());
    let sign = if end == 0 { -1.0 } else { 1.0 };
    let supported = |s| {
        let f = drive.frame(s, true);
        let strip = |u| {
            let p = f.at(u);
            let (s, _) = crate::plan::nearest_station(road, p);
            if s <= road.s0() || s >= road.s1() || road.in_yield_span(s) {
                return None;
            }
            let r = road.frame(s, true);
            let lateral = (p[0] - r.x) * r.nx + (p[1] - r.z) * r.nz;
            let edge = lateral.abs() - road.half;
            // Leave room for the kerb face at the join; a short span on
            // either side then belongs to the same paving material.
            if edge < -o.curb_top_width {
                return Some(0);
            }
            let side = usize::from(lateral < 0.);
            if edge > o.curb_top_width
                && edge < road.walks[side] - o.curb_top_width
                && !road.walk_yield_spans[side]
                    .iter()
                    .any(|w| w[0] <= s && s <= w[1])
            {
                return Some(1 + side);
            }
            None
        };
        strip(0.).is_some_and(|k| strip(-drive.half) == Some(k) && strip(drive.half) == Some(k))
    };
    let stations = crate::plan::build_stations(&drive.points, &drive.cum, lo, hi, o.max_step);
    let snap = |s: f64| {
        stations
            .iter()
            .map(|f| f.s)
            .filter(|v| (*v - s).abs() < o.curb_top_width && *v > lo && *v < hi)
            .min_by(|a, b| (a - s).abs().total_cmp(&(b - s).abs()))
            .filter(|v| supported(*v))
            .unwrap_or(s)
    };
    if supported(mouth) {
        return Some(snap(mouth));
    }
    // Search inside the overlapping corridors, staying on this terminal
    // span. Prefer the closer side of the existing cut.
    let reach = road.half + road.walks[0].max(road.walks[1]) + drive.half;
    let steps = (reach / o.curb_top_width).ceil() as usize;
    for i in 1..=steps {
        let d = i as f64 * o.curb_top_width;
        for s in [mouth + sign * d, mouth - sign * d] {
            if s > lo
                && s < hi
                && !drive
                    .yield_spans
                    .iter()
                    .any(|w| *w != interval && w[0] <= s.max(mouth) && w[1] >= s.min(mouth))
                && supported(s)
            {
                return Some(snap(s));
            }
        }
    }
    None
}
