//! The render meshes of the structures, one 128 m tile and one level of
//! detail at a time (A3 draws them), and the collision triangles drawn from
//! the near level (A4 collides with them).
//!
//! Levels:
//! * 0, near: deck surface with the street's crown blended in at paved ends,
//!   lane and edge paint, expansion joints, Type 60 barriers, fascia with a
//!   drip groove, deck overhang, sloped exterior webs and flat soffit;
//!   approach fill walls and the abutment where the superstructure leaves
//!   its fill; bents (T caps, capital blocks, outrigger caps, round columns,
//!   downspouts); end faces.
//! * 1, far: every other station; deck, barrier boxes, the girder as a box,
//!   edge lines only, square columns, box caps and capitals.
//! * 2, coarse: every fourth station; the deck as a slab down to the soffit.
//!
//! Vertex data (`StructureMesh`): position, normal, colour, class, uv.
//! * `colors`: RGBA8 (`eo_street::mesh::rgba` packing). RGB is the tint
//!   (the street's carriageway tint on `DeckSurface`, the street palette's
//!   marking white or yellow on `Paint`, the concrete tint on `Concrete`).
//!   Alpha is wear on `Concrete`, `Joint` and `Metal` (0 clean .. 255 most
//!   stained: column tops and bases, cap faces under deck joints, the
//!   fascia below drains and its streak, the soffit near joints) and 255 on
//!   `DeckSurface` and `Paint`, except the registered East Span paving tag.
//! * `uv`: surface coordinates in metres for formwork and wear. `uv[0]` runs
//!   along the boards: the chain's station on everything swept along it
//!   (deck, barrier, fascia, overhang, webs, soffit, caps, paint), the
//!   height above the base on a column. `uv[1]` runs across them: the
//!   lateral offset from the centreline on near-horizontal faces (deck,
//!   overhang, soffit, cap undersides), the height relative to the deck top
//!   on near-vertical faces swept along (barrier, fascia, webs, cap sides),
//!   the distance around a column; on faces across the chain (cap ends,
//!   end faces, abutment walls) `uv = (lateral offset, height relative to
//!   the deck top)`.
//! Triangles wind counter-clockwise seen from the side their normal points to.

use std::f64::consts::TAU;

use eo_street::material::StreetMaterial;

use crate::section::{Dims, DRIP_D_M, DRIP_IN_M, DRIP_W_M, EDGE_THICK_M, WEB_TOP_THICK_M, WIDTH_TAPER_M, girder};
use crate::sweep::{Draw, Station, Sweep};
use crate::{Bent, CapKind, Chain, EndKind, StructureClass, StructureMesh};

pub(crate) type V3 = [f64; 3];

/// Paint stands this far above the deck (no fighting with it). INFERRED.
pub(crate) const PAINT_LIFT_M: f64 = 0.0025;
/// An expansion joint strip: this wide along the deck, this far above it.
/// INFERRED, structure-only (a strip seal's steel armour).
pub(crate) const JOINT_W_M: f64 = 0.10;
const JOINT_LIFT_M: f64 = 0.003;
/// Walls and columns run this far into the ground. INFERRED.
pub(crate) const EMBED_M: f64 = 0.5;
/// A capital block under the soffit is this deep per metre of column
/// diameter. INFERRED, structure-only (sheet 5.10's "square capital block").
pub(crate) const CAPITAL_DEPTH_PER_DC: f64 = 0.45;
/// A downspout: 8 in pipe (BDM 2.5 asks at least 6 in, sheet 1.5), standing
/// off its column by this much. INFERRED.
const DOWNSPOUT_R_M: f64 = 0.1016;
const DOWNSPOUT_GAP_M: f64 = 0.03;
/// Sides of a round column and of a pipe at the near level.
const COLUMN_SIDES: usize = 16;
const PIPE_SIDES: usize = 8;

const JOINT_RGB: [u8; 3] = [64, 64, 62];
const METAL_RGB: [u8; 3] = [92, 96, 92];

pub(crate) fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub(crate) fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub(crate) fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub(crate) fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
pub(crate) fn norm(a: V3) -> V3 {
    let l = dot(a, a).sqrt();
    if l < 1e-12 { [0.0, 1.0, 0.0] } else { [a[0] / l, a[1] / l, a[2] / l] }
}
pub(crate) fn scale(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}

/// A mesh being written.
#[derive(Default)]
pub(crate) struct Builder {
    pub m: StructureMesh,
    /// The era of the structure being written (`crate::Era`, stage A9):
    /// every vertex `v` writes carries it.
    pub era: u8,
}

impl Builder {
    pub(crate) fn v(&mut self, p: V3, n: V3, rgb: [u8; 3], wear: f64, uv: [f64; 2], class: StructureClass) -> u32 {
        let alpha = match class {
            StructureClass::DeckSurface | StructureClass::Paint => 255u32,
            _ => (wear.clamp(0.0, 1.0) * 255.0).round() as u32,
        };
        let m = &mut self.m;
        m.positions.push([p[0] as f32, p[1] as f32, p[2] as f32]);
        m.normals.push([n[0] as f32, n[1] as f32, n[2] as f32]);
        m.colors.push((rgb[0] as u32) | ((rgb[1] as u32) << 8) | ((rgb[2] as u32) << 16) | (alpha << 24));
        m.classes.push(class as u8);
        m.uv.push([uv[0] as f32, uv[1] as f32]);
        m.eras.push(self.era);
        (m.positions.len() - 1) as u32
    }

    fn pos(&self, i: u32) -> V3 {
        let p = self.m.positions[i as usize];
        [p[0] as f64, p[1] as f64, p[2] as f64]
    }

    /// A triangle, wound counter-clockwise seen from `want`'s side.
    pub(crate) fn tri(&mut self, i: [u32; 3], want: V3) {
        let (a, b, c) = (self.pos(i[0]), self.pos(i[1]), self.pos(i[2]));
        let n = cross(sub(b, a), sub(c, a));
        if dot(n, n) < 1e-14 {
            return;
        }
        if dot(n, want) >= 0.0 {
            self.m.indices.extend_from_slice(&i);
        } else {
            self.m.indices.extend_from_slice(&[i[0], i[2], i[1]]);
        }
    }

    pub(crate) fn quad(&mut self, i: [u32; 4], want: V3) {
        self.tri([i[0], i[1], i[2]], want);
        self.tri([i[0], i[2], i[3]], want);
    }
}

/// The normal of a face whose cross-section direction at `st` has the 2-D
/// (lateral, up) normal `n2`, square to the deck's grade.
pub(crate) fn normal3(st: &Station, n2: [f64; 2]) -> V3 {
    let n = [n2[0] * st.nx, n2[1], n2[0] * st.nz];
    let t = norm([st.tx, st.slope, st.tz]);
    norm(sub(n, scale(t, dot(n, t))))
}

/// One point of a cross-section: lateral offset, height (world), wear.
#[derive(Clone, Copy, Debug)]
pub(crate) struct P {
    pub u: f64,
    pub y: f64,
    pub wear: f64,
}

pub(crate) fn p(u: f64, y: f64, wear: f64) -> P {
    P { u, y, wear }
}

/// Sweep a cross-section polyline along stations `idx` of `sw`: one flat
/// quad strip per segment of it. Between two stations at one place (where
/// the superstructure leaves its fill) the change of section is a wall
/// across the chain, facing the open side.
pub(crate) fn strip(b: &mut Builder, sw: &Sweep, idx: &[usize], class: StructureClass, rgb: [u8; 3], prof: &dyn Fn(&Station) -> Vec<P>) {
    if idx.len() < 2 {
        return;
    }
    let profs: Vec<Vec<P>> = idx.iter().map(|&k| prof(&sw.st[k])).collect();
    let np = profs.iter().map(|x| x.len()).min().unwrap_or(0);
    for j in 0..np.saturating_sub(1) {
        let mut prev: Option<(u32, u32, V3)> = None;
        for (q, &k) in idx.iter().enumerate() {
            let st = &sw.st[k];
            let (a, c) = (profs[q][j], profs[q][j + 1]);
            let (du, dy) = (c.u - a.u, c.y - a.y);
            let n3 = normal3(st, [-dy, du]);
            let flat = du.abs() >= dy.abs();
            let uv = |x: &P| if flat { [st.s, x.u] } else { [st.s, x.y - st.y] };
            let ia = b.v(st.at(a.u, a.y), n3, rgb, a.wear, uv(&a), class);
            let ic = b.v(st.at(c.u, c.y), n3, rgb, c.wear, uv(&c), class);
            if let Some((pa, pc, pn)) = prev {
                let ps = &sw.st[idx[q - 1]];
                if (st.s - ps.s).abs() < 1e-6 {
                    // a wall across the chain between two sections at one station.
                    let (a0, c0) = (profs[q - 1][j], profs[q - 1][j + 1]);
                    let open_ahead = !st.fill;
                    let t = norm([st.tx, 0.0, st.tz]);
                    let want = if open_ahead { t } else { scale(t, -1.0) };
                    let wv = |b: &mut Builder, x: P| b.v(st.at(x.u, x.y), want, sw.tint, x.wear.max(0.45), [x.u, x.y - st.y], StructureClass::Concrete);
                    let w = [wv(b, a0), wv(b, c0), wv(b, c), wv(b, a)];
                    b.quad(w, want);
                } else {
                    b.quad([pa, pc, ic, ia], add(pn, n3));
                }
            }
            prev = Some((ia, ic, n3));
        }
    }
}

/// The concrete outline of a section at one station, from the +n barrier's
/// toe round under the superstructure to the -n barrier's toe (the deck top
/// between them is the deck surface). Fill stations keep the same points,
/// the underside dropped to the ground.
fn outline(sw: &Sweep, st: &Station, dims: &Dims, lod: u8) -> Vec<P> {
    let h = st.half;
    let bw = dims.barrier_base;
    let hb_full = dims.barrier_h;
    let lean = bw - dims.barrier_top;
    let g = girder(h, bw, st.depth.max(1e-3));
    let te = EDGE_THICK_M.min(g.depth * 0.5);
    // At paved ends and merges the actual asphalt can dip below the profile
    // datum. Keep the underside beneath the lowest drawn part of the section.
    let pavement_min = if st.crown == [0.0; 2] && !st.meet.iter().any(|v| v.is_finite()) {
        st.y
    } else {
        deck_top(sw, st, lod, sw.cross[1], bw).iter().map(|q| q.y).fold(st.y, f64::min)
    };
    let soffit_y = (st.y - st.depth).min(pavement_min - te);
    let below = st.ground_min - EMBED_M;
    let wear_under = 0.3 + 0.35 * st.joint;
    let mut half_side: Vec<P> = vec![];
    for (side, sgn) in [(0usize, 1.0f64), (1, -1.0)] {
        // (an edge a merged major covers sinks under its surface.)
        let covered = if side == 0 { st.clip[1] < h - 1e-6 } else { st.clip[0] > -h + 1e-6 };
        let ye = (if lod == 2 { st.y } else { sw.top(st, sw.clamp_u(st, sgn * h)) }) - if covered { GORE_SINK_M } else { 0.0 };
        let hb = if st.barrier[side] { hb_full } else { 0.0 };
        let off = if hb_full > 0.0 { lean * hb / hb_full } else { 0.0 };
        let ground_side = st.ground[if side == 0 { 2 } else { 0 }] - EMBED_M;
        let streak = 0.4 * st.streak[side];
        let mut pts: Vec<P> = match lod {
            0 | 1 => {
                let inner = if lod == 0 { h + off } else { h };
                vec![p(h, ye, 0.45), p(inner, ye + hb, 0.18), p(h + bw, ye + hb, 0.18)]
            }
            _ => vec![p(h + bw, ye, 0.2)],
        };
        if st.fill {
            let n_under = match lod {
                0 => 6,
                1 => 2,
                _ => 0,
            };
            pts.push(p(h + bw, ground_side, 0.55));
            let us: Vec<f64> = match lod {
                0 => vec![h + bw - DRIP_IN_M, h + bw - DRIP_IN_M, h + bw - DRIP_IN_M - DRIP_W_M, h + bw - DRIP_IN_M - DRIP_W_M, g.web_top, g.soffit],
                1 => vec![(g.web_top + g.soffit) / 2.0, (g.web_top + g.soffit) / 2.0],
                _ => vec![],
            };
            for &u in us.iter().take(n_under) {
                pts.push(p(u, below, 0.5));
            }
        } else {
            match lod {
                0 => {
                    let yb = ye - te;
                    pts.push(p(h + bw, yb, 0.45 + streak));
                    pts.push(p(h + bw - DRIP_IN_M, yb, 0.5 + streak));
                    pts.push(p(h + bw - DRIP_IN_M, yb + DRIP_D_M, 0.5));
                    pts.push(p(h + bw - DRIP_IN_M - DRIP_W_M, yb + DRIP_D_M, 0.5));
                    pts.push(p(h + bw - DRIP_IN_M - DRIP_W_M, yb, 0.45));
                    pts.push(p(g.web_top, (st.y - WEB_TOP_THICK_M).min(pavement_min - te), wear_under));
                    pts.push(p(g.soffit, soffit_y, wear_under + 0.05));
                }
                1 => {
                    let um = (g.web_top + g.soffit) / 2.0;
                    pts.push(p(h + bw, ye - te, 0.45));
                    pts.push(p(um, ye - te, wear_under));
                    pts.push(p(um, soffit_y, wear_under));
                }
                _ => {
                    pts.push(p(h + bw, soffit_y, 0.4));
                }
            }
        }
        for q in pts.iter_mut() {
            q.u *= sgn;
        }
        if side == 0 {
            half_side = pts;
        } else {
            pts.reverse();
            half_side.extend(pts);
        }
    }
    half_side
}

/// The deck surface across a station (only the range a merged major deck
/// leaves to it, `Station::clip`, warped to meet the major there).
pub(crate) fn deck_top(sw: &Sweep, st: &Station, lod: u8, gutter: f64, barrier: f64) -> Vec<P> {
    let h = st.half;
    let clipped = [st.clip[0] > -h + 1e-6, st.clip[1] < h - 1e-6];
    let us: Vec<f64> = match lod {
        0 => crate::sweep::deck_offsets(h, gutter).map(|u| sw.clamp_u(st, u)).collect(),
        1 => vec![st.clip[0], st.clip[1]],
        _ => vec![if clipped[0] { st.clip[0] } else { -(h + barrier) }, if clipped[1] { st.clip[1] } else { h + barrier }],
    };
    us.into_iter().map(|u| p(u, if lod == 2 && !clipped[0] && !clipped[1] { st.y } else { sw.top(st, u) }, 0.0)).collect()
}

/// How far a minor deck's slab edge sinks under a merged major's surface
/// where the major covers it (stage A5's gore: hidden, no fighting with
/// the major's deck). INFERRED, a drawing offset.
const GORE_SINK_M: f64 = 0.05;

/// The drawn deck surface's height at offset `u` of a station at a level
/// (its section's points joined by straight lines).
fn deck_y(sw: &Sweep, st: &Station, lod: u8, gutter: f64, barrier: f64, u: f64) -> f64 {
    let pts = deck_top(sw, st, lod, gutter, barrier);
    let n = pts.len();
    for j in 0..n - 1 {
        if u <= pts[j + 1].u || j + 2 == n {
            let t = ((u - pts[j].u) / (pts[j + 1].u - pts[j].u).max(1e-9)).clamp(0.0, 1.0);
            return pts[j].y + (pts[j + 1].y - pts[j].y) * t;
        }
    }
    st.y
}

/// The deck surface as drawn at a level along stations `idx`: its height
/// over the plan point at station `s`, offset `u` (on the very triangles
/// `run` draws, so what lies on it never dips into it), and where the
/// diagonal of the deck quad under offset `u` crosses it between stations
/// (a point paint must have to follow the two triangles).
struct DeckGrid<'a> {
    sw: &'a Sweep,
    idx: &'a [usize],
    lod: u8,
    gutter: f64,
    barrier: f64,
}

impl DeckGrid<'_> {
    /// The step of `idx` holding `s`: (position, its two stations).
    fn step(&self, s: f64) -> usize {
        let n = self.idx.len();
        let mut q = 0;
        while q + 2 < n && self.sw.st[self.idx[q + 1]].s <= s {
            q += 1;
        }
        q
    }

    fn y(&self, s: f64, u: f64) -> f64 {
        let st = self.sw.lerp(s);
        let q = self.step(s);
        let (a, b) = (&self.sw.st[self.idx[q]], &self.sw.st[self.idx[q + 1]]);
        if (s - a.s).abs() < 1e-9 || (b.s - a.s).abs() < 1e-9 {
            return deck_y(self.sw, a, self.lod, self.gutter, self.barrier, u);
        }
        if (s - b.s).abs() < 1e-9 {
            return deck_y(self.sw, b, self.lod, self.gutter, self.barrier, u);
        }
        let (pa, pb) = (deck_top(self.sw, a, self.lod, self.gutter, self.barrier), deck_top(self.sw, b, self.lod, self.gutter, self.barrier));
        let p = st.at(u, 0.0);
        let n = pa.len().min(pb.len());
        for j in 0..n - 1 {
            // the quad [A, C, C1, A1] is drawn as (A, C, C1) and (A, C1, A1).
            let (ca, cc, cc1, ca1) = (a.at(pa[j].u, pa[j].y), a.at(pa[j + 1].u, pa[j + 1].y), b.at(pb[j + 1].u, pb[j + 1].y), b.at(pb[j].u, pb[j].y));
            for t in [[ca, cc, cc1], [ca, cc1, ca1]] {
                if let Some(y) = plan_height(&t, p[0], p[2]) {
                    return y;
                }
            }
        }
        deck_y(self.sw, &st, self.lod, self.gutter, self.barrier, u)
    }

    /// Stations between `s0` and `s1` (one step) where offset `u` crosses a
    /// deck quad's diagonal.
    fn diagonals(&self, s0: f64, s1: f64, u: f64) -> Vec<f64> {
        let q = self.step((s0 + s1) / 2.0);
        let (a, b) = (&self.sw.st[self.idx[q]], &self.sw.st[self.idx[q + 1]]);
        if b.s - a.s < 1e-9 {
            return vec![];
        }
        let pa = deck_top(self.sw, a, self.lod, self.gutter, self.barrier);
        let mut out = vec![];
        for w in pa.windows(2) {
            if u > w[0].u && u < w[1].u {
                let t = (u - w[0].u) / (w[1].u - w[0].u);
                let s = a.s + (b.s - a.s) * t;
                if s > s0 + 1e-6 && s < s1 - 1e-6 {
                    out.push(s);
                }
            }
        }
        out
    }
}

/// Height of triangle `t` over plan point (x, z), if it is inside it.
pub(crate) fn plan_height(t: &[V3; 3], x: f64, z: f64) -> Option<f64> {
    let (a, b, c) = (t[0], t[1], t[2]);
    let det = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
    if det.abs() < 1e-12 {
        return None;
    }
    let l1 = ((b[2] - c[2]) * (x - c[0]) + (c[0] - b[0]) * (z - c[2])) / det;
    let l2 = ((c[2] - a[2]) * (x - c[0]) + (a[0] - c[0]) * (z - c[2])) / det;
    let l3 = 1.0 - l1 - l2;
    let e = -1e-7;
    (l1 >= e && l2 >= e && l3 >= e).then(|| l1 * a[1] + l2 * b[1] + l3 * c[1])
}

/// Height on the near mesh's triangles. The projected arc selects the
/// neighborhood, then the world point selects the strip and its diagonal.
pub(crate) fn near_deck_height(sw: &Sweep, s: f64, x: f64, z: f64) -> Option<f64> {
    let rows = &sw.levels[0];
    let last_span = rows.len().checked_sub(2)?;
    let span = rows.partition_point(|&row| sw.st[row].s <= s).saturating_sub(1).min(last_span);
    let candidates = [Some(span), span.checked_sub(1), (span < last_span).then_some(span + 1)];
    let row = |index| {
        let st: &Station = &sw.st[index];
        crate::sweep::deck_offsets(st.half, sw.cross[1]).map(move |u| {
            let u = sw.clamp_u(st, u);
            st.at(u, sw.top(st, u))
        })
    };
    for q in candidates.into_iter().flatten() {
        let (a, b) = (rows[q], rows[q + 1]);
        if (sw.st[a].s - sw.st[b].s).abs() < 1e-6 {
            continue;
        }
        let mut columns = row(a).zip(row(b));
        let (mut a0, mut b0) = columns.next()?;
        for (a1, b1) in columns {
            if let Some(y) = plan_height(&[a0, a1, b1], x, z)
                .or_else(|| plan_height(&[a0, b1, b0], x, z))
            {
                return Some(y);
            }
            (a0, b0) = (a1, b1);
        }
    }
    None
}

/// Paint along a run (stations `idx`), each piece's lines over its own
/// stretch (clear of width tapers): broken lines as dashes on the table's
/// pattern from the chain's start, solid lines whole. Every vertex lies on
/// the deck as drawn, `PAINT_LIFT_M` above it.
pub(crate) fn paint(b: &mut Builder, sw: &Sweep, c: &Chain, idx: &[usize], dims: &Dims, lod: u8) {
    let edges_only = lod > 0;
    let grid = DeckGrid { sw, idx, lod, gutter: sw.cross[1], barrier: dims.barrier_base };
    let (s_lo, s_hi) = (sw.st[idx[0]].s, sw.st[*idx.last().unwrap()].s);
    let mat = StreetMaterial::get();
    let (white, yellow) = (mat.tint("markingWhite"), mat.tint("markingYellow"));
    let cycle = dims.dash + dims.gap;
    let inner: Vec<f64> = idx.iter().map(|&k| sw.st[k].s).collect();
    for (pi, piece) in c.pieces.iter().enumerate() {
        let sec = &sw.sections[pi];
        let mut a = piece.s0;
        let mut z = piece.s1;
        if pi > 0 && (c.pieces[pi - 1].half - piece.half).abs() > 1e-6 {
            a += WIDTH_TAPER_M / 2.0;
        }
        if pi + 1 < c.pieces.len() && (c.pieces[pi + 1].half - piece.half).abs() > 1e-6 {
            z -= WIDTH_TAPER_M / 2.0;
        }
        let (a, z) = (a.max(s_lo), z.min(s_hi));
        if z - a < 1e-3 {
            continue;
        }
        for line in sec.lines.iter().filter(|l| l.edge || !edges_only) {
            let mut spans: Vec<(f64, f64)> = vec![];
            if line.dashed {
                let mut k = (a / cycle).floor();
                loop {
                    let d0 = k * cycle;
                    if d0 > z {
                        break;
                    }
                    let (x0, x1) = (d0.max(a), (d0 + dims.dash).min(z));
                    if x1 - x0 > 0.05 {
                        spans.push((x0, x1));
                    }
                    k += 1.0;
                }
            } else {
                spans.push((a, z));
            }
            let rgb = if line.yellow { yellow } else { white };
            let (ul, ur) = (line.u - line.w / 2.0, line.u + line.w / 2.0);
            for (x0, x1) in spans {
                let mut ss: Vec<f64> = vec![x0];
                ss.extend(inner.iter().copied().filter(|&s| s > x0 + 1e-6 && s < x1 - 1e-6));
                ss.push(x1);
                let mut all: Vec<f64> = vec![];
                for w in ss.windows(2) {
                    all.push(w[0]);
                    for u in [ul, line.u, ur] {
                        all.extend(grid.diagonals(w[0], w[1], u));
                    }
                }
                all.push(x1);
                all.sort_by(|p, q| p.total_cmp(q));
                all.dedup_by(|p, q| (*p - *q).abs() < 1e-6);
                let mut prev: Option<(u32, u32)> = None;
                for s in all {
                    let st = sw.lerp(s);
                    // (not over a merged major's surface: the major paints its own.)
                    if ul < st.clip[0] - 1e-6 || ur > st.clip[1] + 1e-6 {
                        prev = None;
                        continue;
                    }
                    let n = normal3(&st, [0.0, 1.0]);
                    let il = b.v(st.at(ul, grid.y(s, ul) + PAINT_LIFT_M), n, rgb, 1.0, [s, 0.0], StructureClass::Paint);
                    let ir = b.v(st.at(ur, grid.y(s, ur) + PAINT_LIFT_M), n, rgb, 1.0, [s, 1.0], StructureClass::Paint);
                    if let Some((pl, pr)) = prev {
                        b.quad([pl, il, ir, pr], n);
                    }
                    prev = Some((il, ir));
                }
            }
        }
    }
}

/// Expansion joint strips across the deck whose station lies in [lo, hi)
/// of the run `idx`, on the deck as drawn.
fn joints(b: &mut Builder, sw: &Sweep, c: &Chain, idx: &[usize], last: bool) {
    let (lo, hi) = (sw.st[idx[0]].s, sw.st[*idx.last().unwrap()].s);
    let grid = DeckGrid { sw, idx, lod: 0, gutter: sw.cross[1], barrier: 0.0 };
    let len = c.length();
    let mut at: Vec<f64> = c.supports.iter().filter(|x| x.joint).map(|x| x.s).collect();
    for (e, end) in c.ends.iter().enumerate() {
        if end.kind != EndKind::Free {
            at.push(if e == 0 { JOINT_W_M / 2.0 } else { len - JOINT_W_M / 2.0 });
        }
    }
    for s in at {
        if !(s >= lo && (s < hi || (last && s <= hi))) {
            continue;
        }
        let (s0, s1) = ((s - JOINT_W_M / 2.0).max(lo), (s + JOINT_W_M / 2.0).min(hi));
        let (a, z) = (sw.lerp(s0), sw.lerp(s1));
        let n = normal3(&a, [0.0, 1.0]);
        let us: Vec<f64> = deck_top(sw, &a, 0, sw.cross[1], 0.0).iter().map(|q| q.u).collect();
        let mut prev: Option<(u32, u32)> = None;
        for u in us {
            let ia = b.v(a.at(u, grid.y(s0, u) + JOINT_LIFT_M), n, JOINT_RGB, 0.6, [s0, u], StructureClass::Joint);
            let iz = b.v(z.at(u, grid.y(s1, u) + JOINT_LIFT_M), n, JOINT_RGB, 0.6, [s1, u], StructureClass::Joint);
            if let Some((pa, pz)) = prev {
                b.quad([pa, ia, iz, pz], n);
            }
            prev = Some((ia, iz));
        }
    }
}

/// One stretch of a chain at one level: deck, paint, joints, concrete.
pub(crate) fn run(b: &mut Builder, sw: &Sweep, c: &Chain, idx: &[usize], lod: u8, dims: &Dims, last: bool) {
    let gutter = sw.cross[1];
    let bw = dims.barrier_base;
    // deck surface: one strip per piece's tint (pieces change at stations).
    let mut start = 0;
    while start + 1 < idx.len() {
        let pi = piece_index(c, (sw.st[idx[start]].s + sw.st[idx[start + 1]].s) / 2.0);
        let mut end = start + 1;
        while end + 1 < idx.len() && piece_index(c, (sw.st[idx[end]].s + sw.st[idx[end + 1]].s) / 2.0) == pi {
            end += 1;
        }
        let begin = b.m.colors.len();
        strip(b, sw, &idx[start..=end], StructureClass::DeckSurface, sw.asphalt[pi], &|st| deck_top(sw, st, lod, gutter, bw));
        if sw.east_pavement {
            for color in &mut b.m.colors[begin..] {
                *color = (*color & 0x00ff_ffff) | (u32::from(crate::EAST_SPAN_PAVING_ALPHA) << 24);
            }
        }
        start = end;
    }
    if sw.draw == Draw::Landmark {
        return;
    }
    if lod < 2 {
        paint(b, sw, c, idx, dims, lod);
    }
    if lod == 0 {
        joints(b, sw, c, idx, last);
    }
    strip(b, sw, idx, StructureClass::Concrete, sw.tint, &|st| outline(sw, st, dims, lod));
}

pub(crate) fn piece_index(c: &Chain, s: f64) -> usize {
    c.pieces.iter().position(|p| s <= p.s1 + 1e-9).unwrap_or(c.pieces.len() - 1)
}

/// A closed face across the chain at a station (an end), as a fan from a
/// point just under the deck top; `out` is the side it faces.
fn end_face(b: &mut Builder, sw: &Sweep, st: &Station, dims: &Dims, lod: u8, out: V3) {
    let mut ring = outline(sw, st, dims, lod);
    // the body without the barriers (their end faces follow): from the +n
    // deck edge round to the -n deck edge, then back along the deck top.
    let h = st.half;
    let bw = dims.barrier_base;
    let skip = if lod < 2 { 3 } else { 1 };
    let n = ring.len();
    let body: Vec<P> = if n > 2 * skip { ring.drain(skip..n - skip).collect() } else { ring };
    let mut poly = vec![p(h + bw, sw.top(st, h), 0.3)];
    poly.extend(body);
    poly.push(p(-(h + bw), sw.top(st, -h), 0.3));
    let deck = deck_top(sw, st, lod, sw.cross[1], bw);
    let lowest = deck.iter().map(|q| q.y).fold(st.y, f64::min);
    // Close the concrete against the actual pavement, including its crown
    // and gutters. A chord across the edges can stand above the driven lane.
    if st.crown != [0.0; 2] || st.meet.iter().any(|v| v.is_finite()) {
        poly.extend(deck.iter().filter(|q| q.u > -h && q.u < h).copied());
    }
    let centre = p(0.0, lowest - 0.1_f64.min(st.depth * 0.5).max(0.05), 0.3);
    let vc = b.v(st.at(centre.u, centre.y), out, sw.tint, 0.35, [0.0, centre.y - st.y], StructureClass::Concrete);
    let vs: Vec<u32> = poly.iter().map(|q| b.v(st.at(q.u, q.y), out, sw.tint, q.wear.max(0.35), [q.u, q.y - st.y], StructureClass::Concrete)).collect();
    for i in 0..vs.len() {
        let j = (i + 1) % vs.len();
        b.tri([vc, vs[i], vs[j]], out);
    }
    // the barrier ends.
    if lod < 2 {
        for (side, sgn) in [(0usize, 1.0f64), (1, -1.0)] {
            if !st.barrier[side] {
                continue;
            }
            let ye = sw.top(st, sgn * h);
            let lean = if lod == 0 { bw - dims.barrier_top } else { 0.0 };
            let pts = [p(h, ye, 0.4), p(h + lean, ye + dims.barrier_h, 0.2), p(h + bw, ye + dims.barrier_h, 0.2), p(h + bw, ye, 0.3)];
            let vs: Vec<u32> = pts.iter().map(|q| b.v(st.at(sgn * q.u, q.y), out, sw.tint, q.wear, [sgn * q.u, q.y - st.y], StructureClass::Concrete)).collect();
            b.quad([vs[0], vs[1], vs[2], vs[3]], out);
        }
    }
}

/// The face closing a chain's end (Free, Landmark and Paved ends).
pub(crate) fn end(b: &mut Builder, sw: &Sweep, e: usize, lod: u8, dims: &Dims) {
    if sw.ends[e] == EndKind::Shared || sw.draw == Draw::Landmark {
        return;
    }
    let st = if e == 0 { sw.st[0] } else { *sw.st.last().unwrap() };
    let t = norm([st.tx, 0.0, st.tz]);
    let out = if e == 0 { scale(t, -1.0) } else { t };
    end_face(b, sw, &st, dims, lod, out);
}

/// A prism along the chain from station `s0` to `s1`: a closed outline in
/// (lateral offset, height above `base(st)`) swept between the two, its ends
/// closed (the outline must be convex).
fn prism(b: &mut Builder, sw: &Sweep, s0: f64, s1: f64, rgb: [u8; 3], wear: &dyn Fn(usize) -> f64, outline: &[(f64, f64)], base: &dyn Fn(&Station) -> f64, class: StructureClass) {
    let (a, z) = (sw.lerp(s0), sw.lerp(s1));
    let n = outline.len();
    for (st0, st1) in [(a, z)] {
        for i in 0..n {
            let j = (i + 1) % n;
            let (p0, p1) = (outline[i], outline[j]);
            let (du, dy) = (p1.0 - p0.0, p1.1 - p0.1);
            let n3 = normal3(&st0, [-dy, du]);
            let flat = du.abs() >= dy.abs();
            let uv = |st: &Station, q: (f64, f64)| if flat { [st.s, q.0] } else { [st.s, base(st) + q.1 - st.y] };
            let v = [
                b.v(st0.at(p0.0, base(&st0) + p0.1), n3, rgb, wear(i), uv(&st0, p0), class),
                b.v(st0.at(p1.0, base(&st0) + p1.1), n3, rgb, wear(j), uv(&st0, p1), class),
                b.v(st1.at(p1.0, base(&st1) + p1.1), n3, rgb, wear(j), uv(&st1, p1), class),
                b.v(st1.at(p0.0, base(&st1) + p0.1), n3, rgb, wear(i), uv(&st1, p0), class),
            ];
            b.quad(v, n3);
        }
    }
    for (st, sgn) in [(a, -1.0), (z, 1.0)] {
        let t = scale(norm([st.tx, st.slope, st.tz]), sgn);
        let cu = outline.iter().map(|q| q.0).sum::<f64>() / n as f64;
        let cy = outline.iter().map(|q| q.1).sum::<f64>() / n as f64;
        let vc = b.v(st.at(cu, base(&st) + cy), t, rgb, wear(0), [cu, base(&st) + cy - st.y], class);
        let vs: Vec<u32> = (0..n).map(|i| b.v(st.at(outline[i].0, base(&st) + outline[i].1), t, rgb, wear(i), [outline[i].0, base(&st) + outline[i].1 - st.y], class)).collect();
        for i in 0..n {
            b.tri([vc, vs[i], vs[(i + 1) % n]], t);
        }
    }
}

/// A round column (or a square one at the far level, its faces square to
/// the chain's tangent `axis`) from `y0` to `y1`, with rings where its wear
/// changes (splashed base, clean shaft, stained top under the cap).
fn column(b: &mut Builder, centre: [f64; 2], r: f64, y0: f64, y1: f64, sides: usize, axis: [f64; 2], rgb: [u8; 3], class: StructureClass, wear: &dyn Fn(f64) -> f64) {
    if y1 - y0 < 0.05 {
        return;
    }
    let h = y1 - y0;
    let mut rings: Vec<f64> = vec![y0];
    for y in [y0 + EMBED_M + 0.8, y1 - 1.2] {
        if y > rings.last().unwrap() + 0.3 && y < y1 - 0.3 {
            rings.push(y);
        }
    }
    rings.push(y1);
    let square = sides <= 4;
    let (start, rr) = if square { (axis[1].atan2(axis[0]) + TAU / 8.0, r * std::f64::consts::SQRT_2) } else { (0.0, r) };
    let at = |a: f64, y: f64| [centre[0] + rr * a.cos(), y, centre[1] + rr * a.sin()];
    let ang = |k: usize| start + TAU * k as f64 / sides as f64;
    if square {
        for w in rings.windows(2) {
            for k in 0..sides {
                let (a0, a1) = (ang(k), ang(k + 1));
                let m = (a0 + a1) / 2.0;
                let n = [m.cos(), 0.0, m.sin()];
                let q = [
                    b.v(at(a0, w[0]), n, rgb, wear((w[0] - y0) / h), [w[0] - y0, 0.0], class),
                    b.v(at(a1, w[0]), n, rgb, wear((w[0] - y0) / h), [w[0] - y0, 2.0 * r], class),
                    b.v(at(a1, w[1]), n, rgb, wear((w[1] - y0) / h), [w[1] - y0, 2.0 * r], class),
                    b.v(at(a0, w[1]), n, rgb, wear((w[1] - y0) / h), [w[1] - y0, 0.0], class),
                ];
                b.quad(q, n);
            }
        }
        return;
    }
    let mut prev: Vec<u32> = vec![];
    for &y in &rings {
        let ring: Vec<u32> = (0..=sides)
            .map(|k| {
                let a = ang(k);
                b.v(at(a, y), [a.cos(), 0.0, a.sin()], rgb, wear((y - y0) / h), [y - y0, r * (a - start)], class)
            })
            .collect();
        if !prev.is_empty() {
            for k in 0..sides {
                let m = start + TAU * (k as f64 + 0.5) / sides as f64;
                b.quad([prev[k], prev[k + 1], ring[k + 1], ring[k]], [m.cos(), 0.0, m.sin()]);
            }
        }
        prev = ring;
    }
}

/// A short pipe between two points (an elbow's leg).
fn pipe(b: &mut Builder, a: V3, z: V3, r: f64) {
    let d = norm(sub(z, a));
    let up = if d[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
    let e1 = norm(cross(d, up));
    let e2 = cross(d, e1);
    let len = dot(sub(z, a), d);
    let mut rings: [Vec<u32>; 2] = [vec![], vec![]];
    for (ri, base) in [a, z].into_iter().enumerate() {
        for k in 0..=PIPE_SIDES {
            let t = TAU * k as f64 / PIPE_SIDES as f64;
            let n = add(scale(e1, t.cos()), scale(e2, t.sin()));
            rings[ri].push(b.v(add(base, scale(n, r)), n, METAL_RGB, if ri == 0 { 0.5 } else { 0.35 }, [len * ri as f64, r * t], StructureClass::Metal));
        }
    }
    for k in 0..PIPE_SIDES {
        let t = TAU * (k as f64 + 0.5) / PIPE_SIDES as f64;
        let n = add(scale(e1, t.cos()), scale(e2, t.sin()));
        b.quad([rings[0][k], rings[0][k + 1], rings[1][k + 1], rings[1][k]], n);
    }
}

/// One bent: its cap (a T beam, capital blocks, or an outrigger cap) and its
/// columns, and a downspout down its +n column. A bent shared with a twin
/// deck (`Bent::twin`, `twin` = the other deck's sweep and its bent's
/// station) draws one cap under both decks, flush with each one's soffit
/// and meeting between them, and every column of the line once, each up to
/// the deck over it.
pub(crate) fn bent(b: &mut Builder, sw: &Sweep, c: &Chain, k: usize, lod: u8, dims: &Dims, twin: Option<(&Sweep, f64)>) {
    let bent: &Bent = &c.supports[k];
    if bent.columns.is_empty() || lod > 1 {
        return;
    }
    if let (Some(t), Some((tsw, ts))) = (bent.twin.filter(|t| !t.carried), twin) {
        shared_bent(b, sw, bent, k, lod, dims, t.u, tsw, ts);
        return;
    }
    let st = sw.lerp(bent.s);
    let g = girder(st.half, dims.barrier_base, st.depth.max(1e-3));
    let soffit = |q: &Station| q.y - q.depth;
    let soffit_y = soffit(&st);
    let r = bent.columns[0].radius;
    let dc = 2.0 * r;
    let jitter = 1.0 + ((k as f64 * 0.618_034).fract() - 0.5) * 0.06;
    let rgb = sw.tint.map(|v| (v as f64 * jitter).clamp(0.0, 255.0) as u8);
    let cap_wear = if bent.joint { 0.8 } else { 0.5 };
    let (umin, umax) = (bent.columns.iter().map(|x| x.u).fold(f64::INFINITY, f64::min), bent.columns.iter().map(|x| x.u).fold(f64::NEG_INFINITY, f64::max));
    let half_w = bent.cap_width / 2.0;
    let mut col_top = soffit_y;
    match bent.cap {
        CapKind::TBeam => {
            let cd = bent.cap_depth;
            let (lo, hi) = ((-g.soffit - 0.25).min(umin - r - 0.3), (g.soffit + 0.25).max(umax + r + 0.3));
            let (cl, cr) = ((umin - r - 0.4).max(lo + 0.05), (umax + r + 0.4).min(hi - 0.05));
            let tip = if lod == 0 { 0.55 * cd } else { cd };
            let o = [(lo, 0.05), (hi, 0.05), (hi, -tip), (cr, -cd), (cl, -cd), (lo, -tip)];
            let wear = |i: usize| if i < 2 { cap_wear - 0.1 } else { cap_wear };
            prism(b, sw, bent.s - half_w, bent.s + half_w, rgb, &wear, &o, &soffit, StructureClass::Concrete);
            col_top = soffit_y - cd;
        }
        CapKind::Outrigger => {
            let (lo, hi) = (umin - r - 0.3, umax + r + 0.3);
            let top = st.depth - EDGE_THICK_M.max(WEB_TOP_THICK_M);
            let o = [(lo, top.max(0.3)), (hi, top.max(0.3)), (hi, 0.0), (lo, 0.0)];
            let wear = |i: usize| if i < 2 { cap_wear - 0.1 } else { cap_wear };
            prism(b, sw, bent.s - half_w, bent.s + half_w, rgb, &wear, &o, &soffit, StructureClass::Concrete);
        }
        CapKind::Integral => {
            let side = (dc + crate::solve::CAP_OVER_COLUMN_M).min(2.0 * crate::BENT_FOOTPRINT_MARGIN_M);
            let ch = (CAPITAL_DEPTH_PER_DC * dc).min((soffit_y - bent.columns.iter().map(|x| x.ground).fold(f64::NEG_INFINITY, f64::max)) * 0.25);
            for col in &bent.columns {
                let o = [(col.u - side / 2.0, 0.05), (col.u + side / 2.0, 0.05), (col.u + side / 2.0, -ch), (col.u - side / 2.0, -ch)];
                let wear = |i: usize| if i < 2 { 0.55 } else { 0.6 };
                prism(b, sw, bent.s - side / 2.0, bent.s + side / 2.0, rgb, &wear, &o, &soffit, StructureClass::Concrete);
            }
            col_top = soffit_y - ch.max(0.0);
        }
    }
    let axis = [st.tx, st.tz];
    let sides = if lod == 0 { COLUMN_SIDES } else { 4 };
    for col in &bent.columns {
        let base = col.ground - EMBED_M;
        let top = col_top;
        let wear = |f: f64| -> f64 {
            // the splashed base, a clean shaft, a stained top under the cap.
            let base_w = 0.45 * (1.0 - (f * 6.0).min(1.0));
            let top_w = 0.6 * ((f - 0.75) / 0.25).clamp(0.0, 1.0);
            0.18 + base_w + top_w
        };
        column(b, [col.x, col.z], col.radius, base, top, sides, axis, rgb, StructureClass::Concrete, &wear);
    }
    if lod == 0 && bent.downspout {
        if let Some(col) = bent.columns.last() {
            let t = norm([st.tx, 0.0, st.tz]);
            let off = col.radius + DOWNSPOUT_GAP_M + DOWNSPOUT_R_M;
            let (x, z) = (col.x + t[0] * off, col.z + t[2] * off);
            let low = col.ground + 0.35;
            if col_top - low > 0.5 {
                pipe(b, [x, col_top + 0.02, z], [x, low, z], DOWNSPOUT_R_M);
                pipe(b, [x, low, z], [x + t[0] * 0.45, low, z + t[2] * 0.45], DOWNSPOUT_R_M);
            }
        }
    }
}

/// A bent shared by two parallel decks (`bent`): along the owner's line at
/// its station, a cap beam under the owner's soffit out to the middle of the
/// gap and on under the twin's soffit (its centreline `tu` along the line;
/// its sweep `tsw` at its own station `ts`), each part as deep as an
/// outrigger cap (flush with the soffit, girder-deep); the columns up to the
/// soffit of the deck nearer each.
#[allow(clippy::too_many_arguments)]
fn shared_bent(b: &mut Builder, sw: &Sweep, bent: &Bent, k: usize, lod: u8, dims: &Dims, tu: f64, tsw: &Sweep, ts: f64) {
    let st = sw.lerp(bent.s);
    let tw = tsw.lerp(ts);
    let bw = dims.barrier_base;
    let (own, other) = (st.half + bw, tw.half + bw);
    let soffits = [st.y - st.depth, tw.y - tw.depth];
    let depths = [st.depth, tw.depth];
    let r = bent.columns[0].radius;
    let jitter = 1.0 + ((k as f64 * 0.618_034).fract() - 0.5) * 0.06;
    let rgb = sw.tint.map(|v| (v as f64 * jitter).clamp(0.0, 255.0) as u8);
    let cap_wear = if bent.joint { 0.8 } else { 0.5 };
    let half_w = bent.cap_width / 2.0;
    let sgn = if tu >= 0.0 { 1.0 } else { -1.0 };
    // the owner's side and the twin's side of the line, meeting between the decks.
    let mid = sgn * ((own + (tu.abs() - other)) / 2.0);
    let (umin, umax) = (bent.columns.iter().map(|x| x.u).fold(f64::INFINITY, f64::min), bent.columns.iter().map(|x| x.u).fold(f64::NEG_INFINITY, f64::max));
    let far_own = -sgn * own.max(if sgn > 0.0 { -umin } else { umax } + r + 0.3);
    let far_twin = tu + sgn * other.max(if sgn > 0.0 { umax - tu } else { tu - umin } + r + 0.3);
    let parts = [(far_own.min(mid), far_own.max(mid), 0usize), (far_twin.min(mid), far_twin.max(mid), 1usize)];
    for (lo, hi, deck) in parts {
        if hi - lo < 1e-3 {
            continue;
        }
        let top = (depths[deck] - EDGE_THICK_M.max(WEB_TOP_THICK_M)).max(0.3);
        let o = [(lo, top), (hi, top), (hi, 0.0), (lo, 0.0)];
        let wear = |i: usize| if i < 2 { cap_wear - 0.1 } else { cap_wear };
        let base = soffits[deck];
        prism(b, sw, bent.s - half_w, bent.s + half_w, rgb, &wear, &o, &|_| base, StructureClass::Concrete);
    }
    let axis = [st.tx, st.tz];
    let sides = if lod == 0 { COLUMN_SIDES } else { 4 };
    for col in &bent.columns {
        let on_twin = (col.u - tu).abs() - other < col.u.abs() - own;
        let top = soffits[usize::from(on_twin)];
        let wear = |f: f64| -> f64 {
            let base_w = 0.45 * (1.0 - (f * 6.0).min(1.0));
            let top_w = 0.6 * ((f - 0.75) / 0.25).clamp(0.0, 1.0);
            0.18 + base_w + top_w
        };
        column(b, [col.x, col.z], col.radius, col.ground - EMBED_M, top, sides, axis, rgb, StructureClass::Concrete, &wear);
    }
    if lod == 0 && bent.downspout {
        if let Some(col) = bent.columns.last() {
            let on_twin = (col.u - tu).abs() - other < col.u.abs() - own;
            let col_top = soffits[usize::from(on_twin)];
            let t = norm([st.tx, 0.0, st.tz]);
            let off = col.radius + DOWNSPOUT_GAP_M + DOWNSPOUT_R_M;
            let (x, z) = (col.x + t[0] * off, col.z + t[2] * off);
            let low = col.ground + 0.35;
            if col_top - low > 0.5 {
                pipe(b, [x, col_top + 0.02, z], [x, low, z], DOWNSPOUT_R_M);
                pipe(b, [x, low, z], [x + t[0] * 0.45, low, z + t[2] * 0.45], DOWNSPOUT_R_M);
            }
        }
    }
}

/// The collision triangles of a render mesh: the deck surface walks, the
/// concrete blocks (paint, joints and pipes are left out).
pub(crate) fn collide(m: &StructureMesh, street: &mut Vec<[[f64; 3]; 3]>, barrier: &mut Vec<[[f64; 3]; 3]>) {
    let pos = |i: u32| -> [f64; 3] {
        let p = m.positions[i as usize];
        [p[0] as f64, p[1] as f64, p[2] as f64]
    };
    for t in m.indices.chunks_exact(3) {
        let tri = [pos(t[0]), pos(t[1]), pos(t[2])];
        match m.classes[t[0] as usize] {
            x if x == StructureClass::DeckSurface as u8 => street.push(tri),
            x if x == StructureClass::Concrete as u8 => barrier.push(tri),
            _ => {}
        }
    }
}
