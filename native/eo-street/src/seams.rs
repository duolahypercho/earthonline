//! Road seams (`--no-road-seams`, `EO_ROAD_SEAMS=0`): two carriageways
//! alongside each other meet at one surface.
//!
//! Each run's datum is the survey under its own centreline (plus the road
//! lift), so two runs side by side on a cross slope stood at their own
//! heights: where their carriageways overlap (a slip road beside the freeway
//! it leaves or joins, a divided road's two carriageways mapped closer than
//! their widths) both were drawn, one up to 0.6 m over the other, and where
//! they only touched, a step stood between the lanes.
//!
//! With seams on, a run whose carriageway overlaps a busier run's
//! (`plan::priority_order`) alongside it (within `SEAM_ALONG_COS` of its
//! heading) draws only the part outside it (`EntryGeom::lo`/`hi`, the drawn
//! lateral interval per station; its kerb, footway and bank on that side
//! not at all), and its datum there is moved so its surface meets the busier
//! one's along the seam. A run whose carriageway edge comes within
//! `SEAM_GAP_M` of a busier one's edge is matched the same way, unclipped.
//! The move fades out along the run over `SEAM_BLEND_M` (at a junction
//! mouth the pad follows it: `mesh::approach_datum`). Runs are seamed
//! busiest first, so a run meets the busier run's final surface.
//!
//! And two runs meeting end to end with no junction end on their bend's
//! bisector (`mitre_joints`), as a run's own vertices are drawn.

use crate::mesh::EntryGeom;
use crate::{StreetWorld, plan};

/// (`--no-road-seams`, `EO_ROAD_SEAMS=0`) every run drawn whole at its own datum, as before.
pub fn road_seams() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("EO_ROAD_SEAMS").map(|v| v.trim() != "0" && v.trim() != "off").unwrap_or(true))
}

/// A busier run counts as alongside within 35 degrees of this run's heading.
const SEAM_ALONG_COS: f64 = 0.82;
/// Lateral samples across a carriageway for its covered edges.
const SEAM_SAMPLES: usize = 17;
/// The clipped edge stands this far inside the busier carriageway (no crack between them), game metres.
const SEAM_OVERLAP_M: f64 = 0.02;
/// Carriageway edges this close (game metres) are matched though they do not overlap.
const SEAM_GAP_M: f64 = 0.75;
/// The datum's move fades out over this far along the run, game metres (0.6 m over it is a 2.5% grade change).
const SEAM_BLEND_M: f64 = 24.0;
/// Runs meeting end to end or at a junction are not seamed within this far
/// past their joint's reach (the pad's, or both halves), game metres.
const SEAM_JOINT_M: f64 = 3.0;
/// A run is moved at most this far to meet a busier one (game metres); two
/// runs further apart are at two levels, the lesser clipped and a face between.
const SEAM_MATCH_MAX_M: f64 = 1.0;
/// A walk narrowed to a busier one is brought to its level where the two are
/// at most this far apart (game metres; 0.5 m across a 3 m walk is a 1 in 6
/// cross fall); further, its outer edge keeps its own level and its bank meets the other walk.
const WALK_MATCH_MAX_M: f64 = 0.5;
/// What is left of a clipped carriageway narrower than this is not drawn, game metres.
const SEAM_MIN_WIDTH_M: f64 = 0.3;

/// A point on a busier run's drawn carriageway: the run, its station, its lateral offset.
#[derive(Clone, Copy)]
struct Hit {
    owner: usize,
    s: f64,
    u: f64,
}

impl StreetWorld {
    /// The runs in the seams' order (`plan::priority_order`), a slip merging
    /// alongside a through road (`plan::Approach::merge_trim`) after every
    /// other: at a merge the through road's surface goes on and the slip meets it.
    fn seam_order(&self) -> (Vec<usize>, Vec<usize>) {
        let (order, _) = plan::priority_order(&self.plan.entries, &self.city);
        let merging = |ei: usize| {
            let e = &self.plan.entries[ei];
            [e.approach_start, e.approach_end].into_iter().flatten().any(|(n, a)| self.plan.nodes[n].approaches[a].merge_trim.is_some())
        };
        let (mut first, last): (Vec<usize>, Vec<usize>) = order.into_iter().partition(|&ei| !merging(ei));
        first.extend(last);
        let mut rank = vec![usize::MAX; self.plan.entries.len()];
        for (i, &e) in first.iter().enumerate() {
            rank[e] = i;
        }
        (first, rank)
    }

    /// Where two runs meet end to end with no junction (a way split where
    /// its lanes or name change, a long curve drawn as several runs), each
    /// ended square to its own heading: a wedge of ground showed on the
    /// outside of the bend and the two overlapped on the inside. Both now end
    /// on the bend's bisector (its mitre, as a run's own vertices are drawn).
    pub(crate) fn mitre_joints(&mut self) {
        let key = |p: [f64; 2]| ((p[0] * 20.0).round() as i64, (p[1] * 20.0).round() as i64);
        let mut ends: std::collections::HashMap<(i64, i64), Vec<(usize, bool)>> = Default::default();
        for (ei, e) in self.plan.entries.iter().enumerate() {
            if self.geoms[ei].stations.len() < 2 || e.points.len() < 2 {
                continue;
            }
            ends.entry(key(e.points[0])).or_default().push((ei, true));
            ends.entry(key(e.points[e.points.len() - 1])).or_default().push((ei, false));
        }
        // the heading through the joint, in the direction of travel from `e`'s side.
        let dir = |e: &plan::Entry, at_start: bool| -> [f64; 2] {
            let n = e.points.len();
            let (a, b) = if at_start { (e.points[0], e.points[1]) } else { (e.points[n - 2], e.points[n - 1]) };
            let l = (b[0] - a[0]).hypot(b[1] - a[1]).max(1e-9);
            [(b[0] - a[0]) / l, (b[1] - a[1]) / l]
        };
        let mut set: Vec<(usize, usize, [f64; 2], f64)> = vec![];
        self.joints = vec![[None, None]; self.plan.entries.len()];
        for list in ends.values() {
            if list.len() != 2 {
                continue;
            }
            for (me, other) in [(list[0], list[1]), (list[1], list[0])] {
                let (ei, at_start) = me;
                let (oi, o_start) = other;
                let (e, o) = (&self.plan.entries[ei], &self.plan.entries[oi]);
                let at_node = if at_start { e.approach_start.is_some() } else { e.approach_end.is_some() };
                let o_node = if o_start { o.approach_start.is_some() } else { o.approach_end.is_some() };
                let g = &self.geoms[ei];
                let k = if at_start { 0 } else { g.stations.len() - 1 };
                let untrimmed = if at_start { g.stations[k].s < 1e-6 } else { (g.stations[k].s - e.length).abs() < 1e-6 };
                let go = &self.geoms[oi];
                let ko = if o_start { 0 } else { go.stations.len() - 1 };
                let o_untrimmed = if o_start { go.stations[ko].s < 1e-6 } else { (go.stations[ko].s - o.length).abs() < 1e-6 };
                if ei == oi || at_node || o_node || !untrimmed || !o_untrimmed {
                    continue;
                }
                self.joints[ei][if at_start { 0 } else { 1 }] = Some((oi, o_start));
                // both headings in `e`'s direction of travel.
                let d = dir(e, at_start);
                let od = dir(o, o_start);
                let od = if at_start == o_start { [-od[0], -od[1]] } else { od };
                let (n1, n2) = ([-d[1], d[0]], [-od[1], od[0]]);
                let (mut ax, mut az) = (n1[0] + n2[0], n1[1] + n2[1]);
                let al = ax.hypot(az);
                if al < 1e-6 {
                    continue;
                }
                ax /= al;
                az /= al;
                let c = ax * n1[0] + az * n1[1];
                if c > 0.25 && c < 1.0 - 1e-9 {
                    set.push((ei, k, [ax, az], 1.0 / c));
                }
            }
        }
        for (ei, k, n, miter) in set {
            let f = &mut self.geoms[ei].stations[k];
            f.nx = n[0];
            f.nz = n[1];
            f.miter = miter;
        }
    }

    /// Seam every drawn run against the busier runs beside it (busiest first).
    pub(crate) fn apply_seams(&mut self) {
        let (order, rank) = self.seam_order();
        // the stations each run meets a busier one at (held in the joints' pass)
        let mut pinned: Vec<Vec<bool>> = vec![vec![]; self.plan.entries.len()];
        // (the runs with a busier one beside them at all, found on every thread:
        // seaming a busier run only shrinks what it covers, so a run with
        // nothing beside it now has nothing after)
        let cand = self.par_map(|ei| self.seam_candidate(ei, &rank));
        for &ei in order.iter().filter(|&&ei| cand[ei]) {
            if let Some((g, pins)) = self.seam_geom(ei, &rank) {
                self.geoms[ei] = g;
                pinned[ei] = pins;
            }
        }
        // two runs meeting end to end meet at one datum: each moved runs its
        // own move, so a joint between a moved run and the next stood a step.
        // Both ends go to their mean (or to the held one), the difference
        // faded back along each over `SEAM_BLEND_M`.
        for ei in 0..self.plan.entries.len() {
            for k in 0..2 {
                let Some((oi, o_start)) = self.joints[ei][k] else { continue };
                if oi < ei {
                    continue;
                }
                let ko = if o_start { 0 } else { 1 };
                let last = |g: &EntryGeom, k: usize| if k == 0 { 0 } else { g.stations.len() - 1 };
                let (ia, ib) = (last(&self.geoms[ei], k), last(&self.geoms[oi], ko));
                let (da, db) = (self.geoms[ei].datums[ia], self.geoms[oi].datums[ib]);
                if (da - db).abs() < 1e-4 {
                    continue;
                }
                // (a stub raised or lowered to a structure, `joins`, is never moved: its end meets the deck)
                let stub = |r: usize| {
                    let e = &self.plan.entries[r];
                    e.structure_ends.iter().any(Option::is_some) || !e.structure_ramps.is_empty()
                };
                let held = |r: usize, i: usize| stub(r) || pinned[r].get(i).copied().unwrap_or(false);
                let mid = match (held(ei, ia), held(oi, ib)) {
                    (true, true) => continue,
                    (true, false) => da,
                    (false, true) => db,
                    (false, false) => (da + db) / 2.0,
                };
                for (r, end, i) in [(ei, k, ia), (oi, ko, ib)] {
                    let diff = mid - self.geoms[r].datums[i];
                    if diff.abs() < 1e-6 || stub(r) {
                        continue;
                    }
                    let g = &mut self.geoms[r];
                    let s_end = g.stations[i].s;
                    for j in 0..g.stations.len() {
                        if pinned[r].get(j).copied().unwrap_or(false) {
                            continue;
                        }
                        let dist = if end == 0 { g.stations[j].s - s_end } else { s_end - g.stations[j].s };
                        let w = (1.0 - dist.abs() / SEAM_BLEND_M).max(0.0);
                        g.datums[j] += diff * w;
                    }
                    g.seamed = true;
                }
            }
        }
    }

    /// `f(ei)` for every run, on every thread.
    fn par_map(&self, f: impl Fn(usize) -> bool + Sync) -> Vec<bool> {
        let n = self.plan.entries.len();
        let threads = std::thread::available_parallelism().map(|t| t.get()).unwrap_or(4).min(16);
        let chunk = n.div_ceil(threads).max(1);
        let ids: Vec<usize> = (0..n).collect();
        std::thread::scope(|sc| {
            let f = &f;
            let hs: Vec<_> = ids.chunks(chunk).map(|c| sc.spawn(move || c.iter().map(|&ei| f(ei)).collect::<Vec<bool>>())).collect();
            hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
        })
    }

    /// Run `ei` has a busier run beside it anywhere (`seam_geom`'s quick test at every station).
    fn seam_candidate(&self, ei: usize, rank: &[usize]) -> bool {
        let e = &self.plan.entries[ei];
        let g = &self.geoms[ei];
        if g.stations.len() < 2 {
            return false;
        }
        let half = e.half;
        g.stations.iter().any(|f| {
            !e.in_yield_span(f.s)
                && [half + SEAM_GAP_M, -half - SEAM_GAP_M, half, -half, 0.0].iter().any(|&u| {
                    let p = f.at(u);
                    self.seam_hit(p[0], p[1], ei, rank[ei], rank, f.tx, f.tz).is_some()
                })
        })
    }

    /// The point (x, z) on the drawn carriageway of a run busier than `me`
    /// (rank under `mine`) heading within `SEAM_ALONG_COS` of (tx, tz).
    fn seam_hit(&self, x: f64, z: f64, me: usize, mine: usize, rank: &[usize], tx: f64, tz: f64) -> Option<Hit> {
        let k = ((x / 32.0).floor() as i32, (z / 32.0).floor() as i32);
        let list = self.road_cells.get(&k)?;
        for &id in list {
            let (o, a, b, half) = self.road_pieces[id as usize];
            let o = o as usize;
            if o == me || rank[o] >= mine {
                continue;
            }
            let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
            let len = dx.hypot(dz);
            if len < 1e-9 || ((dx * tx + dz * tz) / len).abs() < SEAM_ALONG_COS {
                continue;
            }
            let e = &self.plan.entries[o];
            let piece = crate::seg_index(e, a);
            let raw = ((x - a[0]) * dx + (z - a[1]) * dz) / (len * len);
            // past a piece's end only round an interior vertex or a joint with
            // the next run (the ribbons' mitre covers the outside of the bend
            // there), never past a run's free or junction end.
            let joint = |k: usize| self.joints.get(o).is_some_and(|j| j[k].is_some());
            if (raw < -1e-6 && piece == 0 && !joint(0)) || (raw > 1.0 + 1e-6 && piece + 2 >= e.points.len() && !joint(1)) {
                continue;
            }
            let t = raw.clamp(0.0, 1.0);
            let (cx, cz) = (a[0] + dx * t, a[1] + dz * t);
            if (x - cx).hypot(z - cz) > half - SEAM_OVERLAP_M {
                continue;
            }
            let g = &self.geoms[o];
            if g.stations.len() < 2 || self.seam_joint(me, o, x, z) {
                continue;
            }
            let s = e.cum[piece] + t * len;
            if s < g.stations[0].s || s > g.stations[g.stations.len() - 1].s || e.in_yield_span(s) {
                continue;
            }
            let f = e.frame(s, false);
            let u = ((x - f.x) * f.nx + (z - f.z) * f.nz) / f.miter.max(1e-6);
            // (the busier run itself seamed short of here: not drawn)
            let (lo, hi) = g.clip_at(s, e.half);
            if u < lo - 1e-3 || u > hi + 1e-3 {
                continue;
            }
            return Some(Hit { owner: o, s, u });
        }
        None
    }

    /// Narrow every walk lying on a busier run's walk alongside it to end at
    /// that walk's edge, its outer edge at that walk's level where the two
    /// are within `WALK_MATCH_MAX_M` (two parallel streets, a divided road's
    /// two carriageways, their walks over the strip between: the two stood
    /// at their own levels, one over the other, 0.2 m to a metre apart).
    pub(crate) fn seam_walks(&mut self) {
        let (order, rank) = self.seam_order();
        let min_walk = self.o.min_sidewalk_width;
        // (the walks with a busier one beside them at all, on every thread: as `apply_seams`)
        let cand = self.par_map(|ei| {
            let e = &self.plan.entries[ei];
            let g = &self.geoms[ei];
            [(0usize, 1.0f64), (1, -1.0)].iter().any(|&(k, side)| {
                let walk = e.walks[k];
                walk >= min_walk
                    && g.stations.iter().any(|f| (0..=8).any(|q| {
                        let p = f.at(side * (e.half + 0.3 + (walk - 0.3) * q as f64 / 8.0));
                        self.walk_hit(p[0], p[1], ei, rank[ei], &rank, f.tx, f.tz).is_some()
                    }))
            })
        });
        for &ei in order.iter().filter(|&&ei| cand[ei]) {
            let e = &self.plan.entries[ei];
            let g = &self.geoms[ei];
            let n = g.stations.len();
            if n < 2 {
                continue;
            }
            let half = e.half;
            let mut ww: [Vec<f64>; 2] = [vec![], vec![]];
            let mut wy: [Vec<f64>; 2] = [vec![], vec![]];
            for (k, side) in [(0usize, 1.0f64), (1, -1.0)] {
                let walk = e.walks[k];
                if !(walk >= min_walk) {
                    continue;
                }
                let (mut w, mut y) = (vec![walk; n], vec![f64::NAN; n]);
                let mut any = false;
                for (i, f) in g.stations.iter().enumerate() {
                    if e.in_yield_span(f.s) || e.walk_yield_spans[k].iter().any(|sp| f.s >= sp[0] && f.s <= sp[1]) {
                        continue;
                    }
                    let (lo, hi) = g.clip(i, half);
                    if (k == 0 && hi < half - 1e-3) || (k == 1 && lo > -half + 1e-3) {
                        continue;
                    }
                    let hit_at = |u: f64| {
                        let p = f.at(side * u);
                        self.walk_hit(p[0], p[1], ei, rank[ei], &rank, f.tx, f.tz)
                    };
                    let steps = 8;
                    let us: Vec<f64> = (0..=steps).map(|q| half + 0.3 + (walk - 0.3) * q as f64 / steps as f64).collect();
                    let Some(j) = us.iter().position(|&u| hit_at(u).is_some()) else { continue };
                    let (mut a, mut b) = (if j == 0 { half } else { us[j - 1] }, us[j]);
                    for _ in 0..8 {
                        let m = (a + b) / 2.0;
                        if hit_at(m).is_some() {
                            b = m;
                        } else {
                            a = m;
                        }
                    }
                    let uc = (a + b) / 2.0;
                    w[i] = (uc - half).max(0.0);
                    any = true;
                    if let Some(target) = hit_at(uc + 0.01).and_then(|(o, so, uo)| {
                        let q = &self.plan.entries[o];
                        self.datum_at_station(&self.geoms[o], so).map(|d| self.o.sidewalk_surface_y(d, uo.abs(), q.half))
                    }) {
                        let own = self.o.sidewalk_surface_y(g.datums[i], half + w[i], half);
                        if (target - own).abs() <= WALK_MATCH_MAX_M {
                            y[i] = target;
                        }
                    }
                }
                if any {
                    ww[k] = w;
                    wy[k] = y;
                }
            }
            if !ww[0].is_empty() || !ww[1].is_empty() {
                let g = &mut self.geoms[ei];
                g.walk_w = ww;
                g.walk_y = wy;
            }
        }
    }

    /// The point (x, z) on the drawn walk of a run busier than `me` (rank
    /// under `mine`) heading within `SEAM_ALONG_COS` of (tx, tz): the run,
    /// its station, its lateral offset.
    fn walk_hit(&self, x: f64, z: f64, me: usize, mine: usize, rank: &[usize], tx: f64, tz: f64) -> Option<(usize, f64, f64)> {
        let k = ((x / 32.0).floor() as i32, (z / 32.0).floor() as i32);
        let list = self.road_cells.get(&k)?;
        for &id in list {
            let (o, a, b, half) = self.road_pieces[id as usize];
            let o = o as usize;
            if o == me || rank[o] >= mine {
                continue;
            }
            let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
            let len = dx.hypot(dz);
            if len < 1e-9 || ((dx * tx + dz * tz) / len).abs() < SEAM_ALONG_COS {
                continue;
            }
            let Some(t) = self.piece_t(o, a, ((x - a[0]) * dx + (z - a[1]) * dz) / (len * len)) else { continue };
            let (cx, cz) = (a[0] + dx * t, a[1] + dz * t);
            let d = (x - cx).hypot(z - cz);
            let e = &self.plan.entries[o];
            let ko = if (x - cx) * -dz + (z - cz) * dx >= 0.0 { 0 } else { 1 };
            let walk = e.walks[ko];
            if !(walk >= self.o.min_sidewalk_width) || d <= half + 0.05 || d > half + walk {
                continue;
            }
            let g = &self.geoms[o];
            if g.stations.len() < 2 || self.seam_joint(me, o, x, z) {
                continue;
            }
            let s = e.cum[crate::seg_index(e, a)] + t * len;
            if s < g.stations[0].s || s > g.stations[g.stations.len() - 1].s || e.in_yield_span(s) || e.walk_yield_spans[ko].iter().any(|sp| s >= sp[0] && s <= sp[1]) {
                continue;
            }
            // (its own walk narrowed short of here, or its side clipped: not drawn)
            let kk = g.stations.partition_point(|f| f.s <= s).clamp(1, g.stations.len() - 1);
            if g.side_clipped(kk - 1, ko, e.half) || d > half + g.walk_at(ko, kk - 1, walk).min(g.walk_at(ko, kk, walk)) {
                continue;
            }
            return Some((o, s, d));
        }
        None
    }

    /// (x, z) lies where runs `me` and `o` meet end to end or at a junction
    /// both approach: there they overlap by construction (a bend's inner
    /// side, the mouths round a pad), and the pad and the corners join them.
    fn seam_joint(&self, me: usize, o: usize, x: f64, z: f64) -> bool {
        let (a, b) = (&self.plan.entries[me], &self.plan.entries[o]);
        for (na, ai) in [a.approach_start, a.approach_end].into_iter().flatten() {
            let Some((_, bi)) = [b.approach_start, b.approach_end].into_iter().flatten().find(|&(nb, _)| nb == na) else { continue };
            let n = &self.plan.nodes[na];
            // (a slip merging alongside the through road: seamed right up to the pad)
            if n.approaches[ai].merge_trim.is_some() || n.approaches[bi].merge_trim.is_some() {
                continue;
            }
            {
                // (the pad's reach: a merging slip's from the through leg's mouth)
                let r = n.approaches.iter().map(|q| q.merge_trim.unwrap_or(q.trim).min(q.trim) + q.half).fold(0.0, f64::max) + a.half + b.half + SEAM_JOINT_M;
                if (x - n.position[0]).hypot(z - n.position[1]) < r {
                    return true;
                }
            }
        }
        let ends = |e: &plan::Entry| [e.points[0], e.points[e.points.len() - 1]];
        for pa in ends(a) {
            for pb in ends(b) {
                if (pa[0] - pb[0]).hypot(pa[1] - pb[1]) < 0.5 && (x - pa[0]).hypot(z - pa[1]) < a.half + b.half + SEAM_JOINT_M {
                    return true;
                }
            }
        }
        false
    }

    /// The drawn surface of a busier run at `h`.
    fn seam_surface(&self, h: Hit) -> Option<f64> {
        let e = &self.plan.entries[h.owner];
        let d = self.datum_at_station(&self.geoms[h.owner], h.s)?;
        Some(self.o.cross_section_y(d, h.u, e.half))
    }

    /// Run `ei`'s geometry seamed against the busier runs, or None where nothing changes.
    fn seam_geom(&self, ei: usize, rank: &[usize]) -> Option<(EntryGeom, Vec<bool>)> {
        let e = &self.plan.entries[ei];
        let g = &self.geoms[ei];
        if g.stations.len() < 2 {
            return None;
        }
        // (a stub raised or lowered to a structure, `joins`, is clipped but never moved: its end meets the deck)
        let stub = e.structure_ends.iter().any(Option::is_some) || !e.structure_ramps.is_empty();
        let mine = rank[ei];
        let half = e.half;
        let n = g.stations.len();
        let (mut lo, mut hi) = (vec![-half; n], vec![half; n]);
        let mut target: Vec<Option<f64>> = vec![None; n];
        // the busier surface at each station's clipped edges (for the faces, `SEAM_MATCH_MAX_M`)
        let (mut face_lo, mut face_hi) = (vec![f64::NAN; n], vec![f64::NAN; n]);
        let mut clipped = false;
        let shape = |u: f64| self.o.cross_section_y(0.0, u, half);
        for (i, f) in g.stations.iter().enumerate() {
            if e.in_yield_span(f.s) {
                continue;
            }
            let (tx, tz) = (f.tx, f.tz);
            let hit_at = |u: f64| {
                let p = f.at(u);
                self.seam_hit(p[0], p[1], ei, mine, rank, tx, tz)
            };
            // quick reject: nothing busier alongside within reach of either edge.
            if hit_at(half + SEAM_GAP_M).is_none() && hit_at(-half - SEAM_GAP_M).is_none() && hit_at(half).is_none() && hit_at(-half).is_none() && hit_at(0.0).is_none() {
                continue;
            }
            let us: Vec<f64> = (0..SEAM_SAMPLES).map(|j| -half + 2.0 * half * j as f64 / (SEAM_SAMPLES - 1) as f64).collect();
            let cov: Vec<bool> = us.iter().map(|&u| hit_at(u).is_some()).collect();
            // the boundary between an uncovered lateral `a` and a covered `b` (bisection; either order).
            let boundary = |mut a: f64, mut b: f64| {
                for _ in 0..8 {
                    let m = (a + b) / 2.0;
                    if hit_at(m).is_some() {
                        b = m;
                    } else {
                        a = m;
                    }
                }
                (a + b) / 2.0
            };
            let mut sums = (0.0, 0);
            let mut meet = |u: f64, at: f64| -> f64 {
                // our surface at `u` onto the busier one's at the point lateral `at`
                if let Some(y) = hit_at(at).and_then(|h| self.seam_surface(h)) {
                    sums.0 += y - shape(u);
                    sums.1 += 1;
                    return y;
                }
                f64::NAN
            };
            let last = SEAM_SAMPLES - 1;
            if cov.iter().all(|&c| c) {
                // wholly on the busier run: not drawn here.
                lo[i] = 0.0;
                hi[i] = 0.0;
                clipped = true;
                continue;
            }
            if cov[last] {
                let j = (0..=last).rev().take_while(|&j| cov[j]).last().unwrap();
                let u = boundary(us[j - 1], us[j]);
                hi[i] = u;
                clipped = true;
                face_hi[i] = meet(u, u + 0.01);
            } else if hit_at(half + SEAM_GAP_M).is_some() {
                let at = boundary(half, half + SEAM_GAP_M);
                meet(half, at + 0.01);
            }
            if cov[0] {
                let j = (0..=last).take_while(|&j| cov[j]).last().unwrap();
                let u = boundary(us[j + 1], us[j]);
                lo[i] = u;
                clipped = true;
                face_lo[i] = meet(u, u - 0.01);
            } else if hit_at(-half - SEAM_GAP_M).is_some() {
                let at = boundary(-half, -half - SEAM_GAP_M);
                meet(-half, at - 0.01);
            }
            if hi[i] - lo[i] < SEAM_MIN_WIDTH_M {
                lo[i] = 0.0;
                hi[i] = 0.0;
            }
            if sums.1 > 0 {
                target[i] = Some(sums.0 / sums.1 as f64);
            }
        }
        if !clipped && target.iter().all(Option::is_none) {
            return None;
        }
        // two runs this far apart are at two levels (a split-level street, a
        // road under a retaining wall beside another): neither is moved; the
        // lesser is clipped and a face stands between the two along the seam.
        let reach = (0..n).filter_map(|i| target[i].map(|t| (t - g.datums[i]).abs())).fold(0.0, f64::max);
        if stub || reach > SEAM_MATCH_MAX_M {
            if !clipped {
                return None;
            }
            let mut out = g.clone();
            out.lo = lo;
            out.hi = hi;
            out.face_lo = face_lo;
            out.face_hi = face_hi;
            return Some((out, vec![false; n]));
        }
        // the datum's move: exact at the seamed stations, faded along the run between and past them.
        let delta: Vec<Option<f64>> = (0..n).map(|i| target[i].map(|t| t - g.datums[i])).collect();
        let mut out = g.clone();
        for i in 0..n {
            let s = g.stations[i].s;
            let d = match delta[i] {
                Some(d) => d,
                None => {
                    let left = (0..i).rev().find_map(|k| delta[k].map(|d| (g.stations[k].s, d)));
                    let right = (i + 1..n).find_map(|k| delta[k].map(|d| (g.stations[k].s, d)));
                    let fade = |dist: f64| (1.0 - dist / SEAM_BLEND_M).max(0.0);
                    match (left, right) {
                        (Some((sl, dl)), Some((sr, dr))) if sr - sl <= 2.0 * SEAM_BLEND_M => dl + (dr - dl) * (s - sl) / (sr - sl).max(1e-9),
                        _ => left.map_or(0.0, |(sl, dl)| dl * fade(s - sl)) + right.map_or(0.0, |(sr, dr)| dr * fade(sr - s)),
                    }
                }
            };
            out.datums[i] += d;
        }
        out.seamed = true;
        if clipped {
            out.lo = lo;
            out.hi = hi;
        }
        Some((out, target.iter().map(Option::is_some).collect()))
    }
}
