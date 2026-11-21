//! The clearance envelope (stage A6): what space a built deck takes, so the
//! props (trees, lamps, signal masts and arms, signs, wires, furniture,
//! parked cars) keep out of every deck, girder, bent and barrier.
//!
//! Over a disc of plan radius `r` at (x, z), every solid of a drawn deck
//! chain (`ChainKind::Deck`, built, not a landmark's) the disc reaches is an
//! `Obstruction`: the vertical stretch it fills there. A point is placed in
//! the sweep's own coordinates (the station and the offset across at which
//! the swept section passes through it: between two stations the section
//! line turns from one station's to the next's, fanned round a mitred
//! vertex), so the envelope follows the meshes as drawn.
//!
//! * The deck: its whole section, roadway and both barriers (`half +
//!   concrete-barrier-type-60m-base-width` either side), from the soffit
//!   (the deck top less the superstructure depth, taken under the whole
//!   width: the overhang and the battered webs are inside that box) up to
//!   the barrier tops (`concrete-barrier-type-60m-height` over the deck top,
//!   taken across the whole width). Where the superstructure sits on its
//!   approach fill (`Station::fill`: retaining walls to the ground, which
//!   may stand higher than the deck in a cutting), at the abutment wall
//!   where it leaves the fill and at a paved end's end wall, it fills the
//!   space down to the ground (`lo` is minus infinity) and up to the ground
//!   beside it where that is higher.
//! * A bent's cap: along the chain over the cap's width, across from its
//!   outermost column (plus the cap's own reach past it) or the deck edge,
//!   whichever is further, and for a bent shared with a twin deck across
//!   the twin too; from under the soffit by the cap's depth (a T beam's, or
//!   a capital block's, whichever is deeper) up to the barrier tops.
//! * A column: its own disc, from its footing (`mesh::EMBED_M` under the
//!   ground) to the soffit.
//!
//! Heights are taken over the disc's stretch of stations. Every solid is
//! padded by `ENVELOPE_PAD_M` across and `ENVELOPE_PAD_Y_M` up and down
//! over the drawn concrete. A prop's own extent
//! and its margin are the caller's (eo-props). The downspouts (an 8 in pipe
//! standing 3 cm off its column) are not in the envelope; a caller's margin
//! round the column keeps clear of them.

use std::collections::HashMap;

use crate::index::cell_of;
use crate::sweep::{Draw, Station, Sweep};
use crate::{Chain, ChainKind, EndKind, StructureWorld};

/// The envelope reaches this far past the drawn concrete: the profile
/// against the drawn deck top (a paved end's crown blend and a merge's warp
/// are a few centimetres), and the caps and capitals bounded by boxes
/// rather than traced. INFERRED, structure-only.
pub const ENVELOPE_PAD_M: f64 = 0.1;

/// Heights are padded further: the swept quads are drawn as two triangles
/// each, which stand off the section the envelope interpolates by up to
/// about this much where the section twists (a merge's warp, a fanned
/// vertex, a crown blend). INFERRED, structure-only.
pub const ENVELOPE_PAD_Y_M: f64 = 0.25;

/// A paved end's end wall, and the abutment wall where the superstructure
/// leaves its fill, close the section down to the ground over this length
/// either side. INFERRED, structure-only (the wall's thickness and the step
/// to the stub, bounded).
const END_WALL_M: f64 = 0.6;

/// A cap reaches this far past its outermost column (`mesh::bent`: 0.3 to
/// 0.4 m, bounded). INFERRED, structure-only.
const CAP_PAST_COLUMN_M: f64 = 0.4;

/// The widest column radius (the columns' search reach).
const COLUMN_R_MAX_M: f64 = 1.2;

/// Which part of a structure fills the space.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Part {
    /// The deck section: girder, slab, barriers.
    Deck,
    /// The superstructure on its approach fill, an abutment wall or a paved
    /// end's end wall: solid down to the ground.
    Fill,
    /// A bent's cap beam or capital blocks.
    Cap,
    /// A bent's column.
    Column,
}

/// What a structure fills over a disc: the vertical stretch [`lo`, `hi`]
/// (game frame; `lo` is minus infinity for `Part::Fill`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Obstruction {
    pub chain: u32,
    pub part: Part,
    pub lo: f64,
    pub hi: f64,
}

/// A point on top of one of a built deck's barriers, where a
/// barrier-mounted luminaire stands (stage A6).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarrierPoint {
    pub chain: u32,
    pub s: f64,
    /// +1: the barrier on the deck's +n edge (left of increasing station), -1: on its -n edge.
    pub side: i8,
    /// The middle of the barrier's top (plan).
    pub x: f64,
    pub z: f64,
    /// The deck top at the roadway's edge beside it, and the barrier's top.
    pub deck_y: f64,
    pub top_y: f64,
    /// Unit plan direction from the barrier over the roadway.
    pub inward: [f64; 2],
}

#[derive(Clone, Copy, Debug)]
struct Col {
    chain: u32,
    x: f64,
    z: f64,
    r: f64,
    lo: f64,
    hi: f64,
}

/// The envelope's indexes (built with the meshes).
#[derive(Debug, Default)]
pub(crate) struct ClearIndex {
    /// 32 m cell -> (chain, centreline segment) of every drawn deck, filed
    /// under every cell its section (and its caps) reaches.
    cells: HashMap<(i32, i32), Vec<(u32, u32)>>,
    /// 32 m cell of a column's centre -> index into `columns`.
    col_cells: HashMap<(i32, i32), Vec<u32>>,
    columns: Vec<Col>,
    /// Per chain: how far across (section metres) its solids reach.
    reach: Vec<f64>,
    barrier_h: f64,
    barrier_base: f64,
}

fn drawn(sw: &Option<Sweep>) -> Option<&Sweep> {
    sw.as_ref().filter(|s| s.draw == Draw::Deck && s.st.len() >= 2)
}

/// The deck top's lowest and highest points across a station's roadway.
fn top_range(sw: &Sweep, st: &Station) -> (f64, f64) {
    let ys = [sw.top(st, -st.half), sw.top(st, 0.0), sw.top(st, st.half), st.y];
    (ys.iter().copied().fold(f64::INFINITY, f64::min), ys.iter().copied().fold(f64::NEG_INFINITY, f64::max))
}

/// The superstructure's depth as drawn (`section::girder` keeps at least
/// the web top and a little more).
fn drawn_depth(st: &Station) -> f64 {
    st.depth.max(crate::section::WEB_TOP_THICK_M + 0.15)
}

/// What the deck is over [s - span, s + span]: the lowest soffit, the
/// highest deck top, the highest ground beside it, and whether any of it
/// stands on fill.
struct About {
    soffit: f64,
    top: f64,
    ground: f64,
    fill: bool,
}

fn about(sw: &Sweep, s: f64, span: f64) -> About {
    let mut a = About { soffit: f64::INFINITY, top: f64::NEG_INFINITY, ground: f64::NEG_INFINITY, fill: false };
    let n = ((2.0 * span / 2.0).ceil() as usize).clamp(2, 16);
    for k in 0..=n {
        let st = sw.lerp(s - span + 2.0 * span * k as f64 / n as f64);
        let (lo, hi) = top_range(sw, &st);
        a.soffit = a.soffit.min(lo - drawn_depth(&st));
        a.top = a.top.max(hi);
        a.ground = a.ground.max(st.ground.iter().copied().fold(f64::NEG_INFINITY, f64::max));
        a.fill |= st.fill;
    }
    a
}

fn cross(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

/// The sweep's own coordinates of plan point `p` among stations from index
/// `k0` to `k1`: every (station, offset across in section metres, how far
/// past an end of the sweep) at which a station's section line, or one
/// turning between two stations, passes through it with the offset within
/// `reach`; a point before the first station or after the last is put on
/// the end's section line, with its distance past it.
fn locate(sw: &Sweep, k0: usize, k1: usize, p: [f64; 2], reach: f64, past: f64, out: &mut Vec<(f64, f64, f64)>) {
    let n = sw.st.len();
    let line = |st: &Station| ([st.x, st.z], [st.ox, st.oz]);
    let on = |c: [f64; 2], o: [f64; 2]| -> f64 { cross([p[0] - c[0], p[1] - c[1]], o) };
    for k in k0..k1.min(n - 1) {
        let (a, b) = (&sw.st[k], &sw.st[k + 1]);
        if b.s - a.s < 1e-6 {
            continue;
        }
        let at = |t: f64| -> ([f64; 2], [f64; 2]) {
            let l = |p: f64, q: f64| p + (q - p) * t;
            ([l(a.x, b.x), l(a.z, b.z)], [l(a.ox, b.ox), l(a.oz, b.oz)])
        };
        let g = |t: f64| {
            let (c, o) = at(t);
            on(c, o)
        };
        let (ga, gb) = (g(0.0), g(1.0));
        if ga.signum() == gb.signum() && ga != 0.0 && gb != 0.0 {
            continue;
        }
        let t = if ga == 0.0 {
            0.0
        } else if gb == 0.0 {
            1.0
        } else {
            let (mut lo, mut hi) = (0.0, 1.0);
            for _ in 0..40 {
                let m = (lo + hi) / 2.0;
                if g(m).signum() == ga.signum() {
                    lo = m;
                } else {
                    hi = m;
                }
            }
            (lo + hi) / 2.0
        };
        let (c, o) = at(t);
        let o2 = (o[0] * o[0] + o[1] * o[1]).max(1e-12);
        let u = ((p[0] - c[0]) * o[0] + (p[1] - c[1]) * o[1]) / o2;
        if u.abs() <= reach {
            out.push((a.s + (b.s - a.s) * t, u, 0.0));
        }
    }
    // past either end: on the end's section line.
    for (end, k, sgn) in [(k0 == 0, 0usize, -1.0f64), (k1 + 1 >= n, n - 1, 1.0)] {
        if !end {
            continue;
        }
        let st = &sw.st[k];
        let (c, o) = line(st);
        let along = sgn * ((p[0] - c[0]) * st.tx + (p[1] - c[1]) * st.tz);
        if along > 0.0 && along <= past {
            let o2 = (o[0] * o[0] + o[1] * o[1]).max(1e-12);
            let u = ((p[0] - c[0]) * o[0] + (p[1] - c[1]) * o[1]) / o2;
            if u.abs() <= reach {
                out.push((st.s, u, along));
            }
        }
    }
}

impl ClearIndex {
    pub fn build(chains: &[Chain], sweeps: &[Option<Sweep>], barrier_h: f64, barrier_base: f64) -> Self {
        let mut ix = ClearIndex { barrier_h, barrier_base, reach: vec![0.0; chains.len()], ..Default::default() };
        for (ci, c) in chains.iter().enumerate() {
            let Some(sw) = drawn(&sweeps[ci]) else { continue };
            if c.kind != ChainKind::Deck || c.landmark.is_some() {
                continue;
            }
            // how far across any of its solids reaches (section metres), and in the plan.
            let mut reach = sw.st.iter().map(|s| s.half).fold(c.max_half(), f64::max) + barrier_base;
            for b in &c.supports {
                let st = sw.lerp(b.s);
                let mut soffit = top_range(sw, &st).0 - drawn_depth(&st);
                if let Some(t) = b.twin.filter(|t| !t.carried)
                    && let Some(tc) = chains.get(t.chain as usize)
                {
                    reach = reach.max(t.u.abs() + tc.max_half() + barrier_base);
                    // (a shared bent's columns under the twin stand up to its soffit.)
                    if let (Some(tsw), Some(tb)) = (sweeps.get(t.chain as usize).and_then(drawn), tc.supports.get(t.bent as usize)) {
                        let ts = tsw.lerp(tb.s);
                        soffit = soffit.max(top_range(tsw, &ts).0 - drawn_depth(&ts));
                    }
                }
                for col in &b.columns {
                    reach = reach.max(col.u.abs() + col.radius + CAP_PAST_COLUMN_M);
                    ix.col_cells.entry((cell_of(col.x), cell_of(col.z))).or_default().push(ix.columns.len() as u32);
                    ix.columns.push(Col { chain: c.id, x: col.x, z: col.z, r: col.radius, lo: col.ground - crate::mesh::EMBED_M, hi: soffit });
                }
            }
            ix.reach[ci] = reach + ENVELOPE_PAD_M;
            let miter = sw.st.iter().map(|s| s.ox.hypot(s.oz)).fold(1.0, f64::max);
            let pad = (reach + ENVELOPE_PAD_M) * miter;
            for i in 0..c.points.len().saturating_sub(1) {
                let (a, b) = (c.points[i], c.points[i + 1]);
                let (x0, x1) = (a[0].min(b[0]) - pad, a[0].max(b[0]) + pad);
                let (z0, z1) = (a[1].min(b[1]) - pad, a[1].max(b[1]) + pad);
                for gz in cell_of(z0)..=cell_of(z1) {
                    for gx in cell_of(x0)..=cell_of(x1) {
                        ix.cells.entry((gx, gz)).or_default().push((ci as u32, i as u32));
                    }
                }
            }
        }
        ix
    }
}

impl StructureWorld {
    /// Every solid of a built deck (its deck and girder, barriers, approach
    /// fill, abutment and end walls, bent caps and columns) a disc of plan
    /// radius `radius` at (x, z) reaches, each with the vertical stretch it
    /// fills there (see the `clear` module). `f` returns false to stop
    /// early. Nothing for a world without decks.
    pub fn scan_obstructions(&self, x: f64, z: f64, radius: f64, f: &mut dyn FnMut(Obstruction) -> bool) {
        let ix = &self.clear;
        let r = radius.max(0.0);
        let pad = ENVELOPE_PAD_M;
        let pad_y = ENVELOPE_PAD_Y_M;
        // columns: each filed once, under its centre's cell.
        let cr = r + COLUMN_R_MAX_M + pad;
        for gz in cell_of(z - cr)..=cell_of(z + cr) {
            for gx in cell_of(x - cr)..=cell_of(x + cr) {
                let Some(list) = ix.col_cells.get(&(gx, gz)) else { continue };
                for &k in list {
                    let c = ix.columns[k as usize];
                    if (c.x - x).hypot(c.z - z) <= c.r + r + pad && !f(Obstruction { chain: c.chain, part: Part::Column, lo: c.lo, hi: c.hi + pad_y }) {
                        return;
                    }
                }
            }
        }
        // decks and caps: the chains filed where the disc is, over the
        // stations of their segments there.
        let mut cand: Vec<(u32, u32)> = vec![];
        for gz in cell_of(z - r)..=cell_of(z + r) {
            for gx in cell_of(x - r)..=cell_of(x + r) {
                if let Some(list) = ix.cells.get(&(gx, gz)) {
                    cand.extend_from_slice(list);
                }
            }
        }
        if cand.is_empty() {
            return;
        }
        cand.sort_unstable();
        cand.dedup();
        let mut hits: Vec<(f64, f64, f64)> = vec![];
        let mut i = 0;
        while i < cand.len() {
            let ci = cand[i].0;
            let c = &self.chains[ci as usize];
            // the stretch of stations of this chain's segments filed here.
            let (mut s0, mut s1) = (f64::INFINITY, f64::NEG_INFINITY);
            while i < cand.len() && cand[i].0 == ci {
                let si = cand[i].1 as usize;
                s0 = s0.min(c.cum[si]);
                s1 = s1.max(c.cum[si + 1]);
                i += 1;
            }
            let Some(sw) = self.tiles.sweeps.get(ci as usize).and_then(drawn) else { continue };
            let reach = ix.reach[ci as usize] + r;
            let k_of = |s: f64| sw.st.partition_point(|q| q.s < s);
            let k0 = k_of(s0 - reach - crate::STATION_MAX_M).saturating_sub(1);
            let k1 = (k_of(s1 + reach + crate::STATION_MAX_M) + 1).min(sw.st.len() - 1);
            hits.clear();
            locate(sw, k0, k1, [x, z], reach, r + pad, &mut hits);
            for &(s, u, past) in &hits {
                let span = (r + pad).max(past);
                let a = about(sw, s, span);
                let st = sw.lerp(s);
                let edge = st.half + ix.barrier_base;
                let hi = a.top + ix.barrier_h + pad_y;
                if u.abs() <= edge + r + pad {
                    let paved_end = (c.ends[0].kind == EndKind::Paved && s <= END_WALL_M + span) || (c.ends[1].kind == EndKind::Paved && s >= c.length() - END_WALL_M - span);
                    // (the abutment wall where the superstructure leaves its fill closes the section to the ground.)
                    let fill = a.fill || sw.lerp(s - END_WALL_M - span).fill || sw.lerp(s + END_WALL_M + span).fill;
                    let o = if fill || paved_end {
                        Obstruction { chain: c.id, part: Part::Fill, lo: f64::NEG_INFINITY, hi: hi.max(a.ground + pad_y) }
                    } else {
                        Obstruction { chain: c.id, part: Part::Deck, lo: a.soffit - pad_y, hi }
                    };
                    if !f(o) {
                        return;
                    }
                }
                // the caps of the bents there (swept along the stations over the cap's width).
                for bent in &c.supports {
                    if bent.columns.is_empty() || (bent.s - s).abs() > bent.cap_width / 2.0 + span {
                        continue;
                    }
                    let sb = sw.lerp(bent.s);
                    let eb = sb.half + ix.barrier_base;
                    let (mut lo_u, mut hi_u) = (-eb, eb);
                    let mut deepest = bent.cap_depth;
                    for col in &bent.columns {
                        lo_u = lo_u.min(col.u - col.radius - CAP_PAST_COLUMN_M);
                        hi_u = hi_u.max(col.u + col.radius + CAP_PAST_COLUMN_M);
                        deepest = deepest.max(crate::mesh::CAPITAL_DEPTH_PER_DC * 2.0 * col.radius);
                    }
                    let b = about(sw, bent.s, bent.cap_width / 2.0 + pad);
                    let mut cap_lo = b.soffit.min(a.soffit) - deepest;
                    let mut cap_hi = b.top.max(a.top) + ix.barrier_h;
                    if let Some(tw) = bent.twin.filter(|t| !t.carried)
                        && let Some(tc) = self.chains.get(tw.chain as usize)
                        && let Some(tsw) = self.tiles.sweeps.get(tw.chain as usize).and_then(drawn)
                        && let Some(tb) = tc.supports.get(tw.bent as usize)
                    {
                        let other = tsw.lerp(tb.s).half + ix.barrier_base;
                        lo_u = lo_u.min(tw.u - other);
                        hi_u = hi_u.max(tw.u + other);
                        let o = about(tsw, tb.s, bent.cap_width / 2.0 + pad);
                        cap_lo = cap_lo.min(o.soffit - deepest);
                        cap_hi = cap_hi.max(o.top + ix.barrier_h);
                    }
                    if u >= lo_u - r - pad && u <= hi_u + r + pad && !f(Obstruction { chain: c.id, part: Part::Cap, lo: cap_lo - pad_y, hi: cap_hi + pad_y }) {
                        return;
                    }
                }
            }
        }
    }

    /// Every solid a disc reaches (`scan_obstructions`), in scan order.
    pub fn obstructions(&self, x: f64, z: f64, radius: f64) -> Vec<Obstruction> {
        let mut out = vec![];
        self.scan_obstructions(x, z, radius, &mut |o| {
            out.push(o);
            true
        });
        out
    }

    /// Whether a vertical cylinder of plan radius `radius` at (x, z) from
    /// `y0` to `y1` meets any built deck, girder, barrier, fill, cap or
    /// column (the clearance envelope); `except` leaves one chain's solids
    /// out (what stands on that deck).
    pub fn blocked_except(&self, x: f64, z: f64, radius: f64, y0: f64, y1: f64, except: Option<u32>) -> bool {
        let mut hit = false;
        self.scan_obstructions(x, z, radius, &mut |o| {
            if Some(o.chain) != except && o.lo <= y1 && o.hi >= y0 {
                hit = true;
            }
            !hit
        });
        hit
    }

    /// `blocked_except` with nothing left out.
    pub fn blocked(&self, x: f64, z: f64, radius: f64, y0: f64, y1: f64) -> bool {
        self.blocked_except(x, z, radius, y0, y1, None)
    }

    /// The lowest underside over a disc above height `y` (a soffit, a cap's
    /// underside): how high something standing at `y` may reach. `y` itself
    /// (or less) when a solid already fills the space at `y` (a column, the
    /// fill); None with nothing over the disc.
    pub fn headroom(&self, x: f64, z: f64, radius: f64, y: f64) -> Option<f64> {
        let mut best: Option<f64> = None;
        self.scan_obstructions(x, z, radius, &mut |o| {
            if o.hi >= y {
                best = Some(best.map_or(o.lo, |b: f64| b.min(o.lo)));
            }
            true
        });
        best
    }

    /// The tops of a built deck's barriers at station `s` (one per edge
    /// where a barrier stands: none at a merge's or a gore's open edge),
    /// for what stands on them. Empty for a chain with no barriers drawn
    /// (held, a landmark's, a road below others), or `s` off the chain.
    pub fn barrier_at(&self, chain: u32, s: f64) -> Vec<BarrierPoint> {
        let Some(c) = self.chains.get(chain as usize) else { return vec![] };
        let Some(sw) = self.tiles.sweeps.get(chain as usize).and_then(drawn) else { return vec![] };
        if c.kind != ChainKind::Deck || c.landmark.is_some() || !(s >= 0.0 && s <= c.length()) {
            return vec![];
        }
        let st = sw.lerp(s);
        let ix = &self.clear;
        let mut out = vec![];
        for (k, sgn) in [(0usize, 1.0f64), (1, -1.0)] {
            if !st.barrier[k] {
                continue;
            }
            let p = st.at(sgn * (st.half + ix.barrier_base / 2.0), 0.0);
            let deck_y = sw.top(&st, sgn * st.half);
            out.push(BarrierPoint { chain, s, side: sgn as i8, x: p[0], z: p[2], deck_y, top_y: deck_y + ix.barrier_h, inward: [-sgn * st.nx, -sgn * st.nz] });
        }
        out
    }
}
