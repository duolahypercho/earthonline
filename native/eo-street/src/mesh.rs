//! Street surface geometry per 128 m tile, from the plan: a port of the
//! construction `emitSegment`, `emitJunction` and `emitCurbRing` lay in
//! src/world/streets/street-surface-v2.js, on the same cross-section
//! functions (crossSectionY, curbTopY, footwayRiseAt, sidewalkSurfaceY,
//! gradedBankReach) and the same plan (trims, corner arcs and chords, ramp
//! windows, yield spans):
//!
//!   carriageway  cambered, gutter pan to the invert, one datum per centreline
//!                station (terrain + roadLift), mitred at bends
//!   kerb         a real face (invert -> kerb top), kerb top with its fall
//!   footway      from the back of the kerb, on the footway cross slope
//!   kerb ramps   the kerb top runs down to the gutter invert across a ramp
//!                window (flat part + flared shoulders), the ramp climbing
//!                `rampRun` back into the walk; on the ribbon at every painted
//!                crossing and on corner returns (arc pair, or the chord's)
//!   frontage     paving level with the walk to a facade within reach, or to
//!                the surveyed property line (a simplified `frontageDepth`)
//!   verge bank   from the back of the paving to the ground carpet
//!   junction pad a fan from the node over the approaches' mouth sections and
//!                the corner kerb lines; corner returns carry the kerb, ramps
//!                and footway round the fillet or the chord
//!   paint        lane lines (dashed per CA MUTCD broken-line pattern), centre
//!                lines, edge lines, cycle lanes, continental crosswalk bars,
//!                limit lines, lifted `markingLift` / `junctionPaintLift`
//!
//! Every primitive belongs to the tile holding its anchor (a span's centreline
//! midpoint, a node's position), so tiles never draw the same triangle twice
//! and need no clipping. Colours are placeholder tones for this slice (the
//! JS tints a scanned texture; no texture is drawn here), not a look.

use crate::plan::{self, Approach, CurbStation, Entry, Frame, Node, RampWin};
use crate::{Layer, StreetWorld};
use std::f64::consts::PI;

// The vertex colours are the JS palette's albedo tints
// (STREET_SURFACE_V2_PALETTES.sf, data/street-material.json); the renderer
// multiplies them by the scanned set's base fold and variance as the JS
// ground materials do.
fn pal(key: &str) -> [u8; 3] {
    crate::material::StreetMaterial::get().tint(key)
}
#[allow(non_snake_case)]
fn GUTTER() -> [u8; 3] {
    pal("gutterPan")
}
#[allow(non_snake_case)]
fn PAD() -> [u8; 3] {
    pal("junction")
}
#[allow(non_snake_case)]
fn CURB_FACE() -> [u8; 3] {
    pal("curbFace")
}
#[allow(non_snake_case)]
fn CURB_TOP() -> [u8; 3] {
    pal("curbTop")
}
#[allow(non_snake_case)]
fn WALK() -> [u8; 3] {
    pal("sidewalk")
}
#[allow(non_snake_case)]
fn RAMP() -> [u8; 3] {
    pal("ramp")
}
#[allow(non_snake_case)]
fn VERGE() -> [u8; 3] {
    pal("verge")
}

/// how far a retaining face reaches below the lower paving it closes
/// onto, metres (retaining-edge.js `RETAINING_WALL_TUCK_M`: a buried footing,
/// so the face and the paving never part; below the paving it is not seen).
const RETAINING_TUCK_M: f64 = 1.0;

/// A stub's datum this far over the ground's own datum is on an approach
/// embankment (`joins::StructureEnd::rise`): a side with no footway gets a
/// bank. A drawing tolerance, not a street dimension.
const RAISED_M: f64 = 0.02;

/// A stub's end cap (`emit_stub_cap`) stands this far inside the stub, so it
/// never shares a plane with the structure's own end face at the same end.
/// A drawing offset, not a street dimension.
const CAP_SET_BACK_M: f64 = 0.02;

/// The kerb depression of side `k` at station `i` (0 when none recorded).
fn mesh_dep(g: &EntryGeom, k: usize, i: usize) -> f64 {
    g.dep[k].get(i).copied().unwrap_or(0.0)
}
#[allow(non_snake_case)]
fn WHITE() -> [u8; 3] {
    pal("markingWhite")
}
#[allow(non_snake_case)]
fn CROSSWALK() -> [u8; 3] {
    pal("crosswalk")
}
#[allow(non_snake_case)]
fn YELLOW() -> [u8; 3] {
    pal("markingYellow")
}
#[allow(non_snake_case)]
fn GREEN() -> [u8; 3] {
    pal("markingBikeGreen")
}

/// The gutter pan's colour as packed RGB (the renderer draws it as the pan).
pub fn gutter_rgb() -> u32 {
    let g = GUTTER();
    (g[0] as u32) | ((g[1] as u32) << 8) | ((g[2] as u32) << 16)
}
/// The junction pad's colour as packed RGB.
pub fn pad_rgb() -> u32 {
    let g = PAD();
    (g[0] as u32) | ((g[1] as u32) << 8) | ((g[2] as u32) << 16)
}

pub fn rgba(c: [u8; 3]) -> u32 {
    (c[0] as u32) | ((c[1] as u32) << 8) | ((c[2] as u32) << 16) | (255u32 << 24)
}

fn tone(c: [u8; 3], k: f64) -> [u8; 3] {
    [(c[0] as f64 * k).clamp(0.0, 255.0) as u8, (c[1] as f64 * k).clamp(0.0, 255.0) as u8, (c[2] as f64 * k).clamp(0.0, 255.0) as u8]
}

/// Geometry of one tile, ready for the GPU (positions in world metres).
#[derive(Clone, Debug, Default)]
pub struct TileMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub colors: Vec<u32>,
    pub indices: Vec<u32>,
    /// One layer per triangle.
    pub layers: Vec<Layer>,
    /// Per vertex: distance to the kerb line on the carriageway (m), and
    /// (cc0) on a run's footway the distance across the walk from the kerb
    /// line (the renderer's sidewalk scoring grid), on a kerb face the height
    /// above the gutter flow line, on a marking its across-the-line coordinate
    /// (0..1), else -1.
    pub kerb: Vec<f32>,
}

impl TileMesh {
    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }
    pub fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
        for p in &self.positions {
            for k in 0..3 {
                mn[k] = mn[k].min(p[k]);
                mx[k] = mx[k].max(p[k]);
            }
        }
        (mn, mx)
    }

    fn vert(&mut self, p: [f64; 3], n: [f64; 3], c: [u8; 3]) -> u32 {
        self.positions.push([p[0] as f32, p[1] as f32, p[2] as f32]);
        self.normals.push([n[0] as f32, n[1] as f32, n[2] as f32]);
        self.colors.push(rgba(c));
        self.kerb.push(-1.0);
        (self.positions.len() - 1) as u32
    }

    /// `quad`, recording each vertex's distance to the kerb line.
    fn quad_kerb(&mut self, layer: Layer, p: [[f64; 3]; 4], c: [[u8; 3]; 4], kerb: [f32; 4]) {
        self.quad_kerb_face(layer, p, c, None, kerb);
    }

    /// `quad` facing `face_ref`, recording a per-vertex coordinate in `kerb`
    /// (a kerb face's height above the gutter flow line).
    fn quad_kerb_face(&mut self, layer: Layer, p: [[f64; 3]; 4], c: [[u8; 3]; 4], face_ref: Option<[f64; 2]>, kerb: [f32; 4]) {
        let first = self.positions.len();
        self.quad(layer, p, c, face_ref);
        if self.positions.len() == first + 4 {
            self.kerb[first..first + 4].copy_from_slice(&kerb);
        }
    }

    /// A planar-ish quad a b c d (in order round it), one flat normal facing up
    /// (or, for a vertical face, toward `face_ref`). Degenerate quads are dropped.
    fn quad(&mut self, layer: Layer, p: [[f64; 3]; 4], c: [[u8; 3]; 4], face_ref: Option<[f64; 2]>) {
        let n = quad_normal(&p, face_ref);
        let Some(n) = n else { return };
        let base = self.positions.len() as u32;
        for k in 0..4 {
            self.vert(p[k], n, c[k]);
        }
        let area1 = tri_area(&p[0], &p[1], &p[2]);
        let area2 = tri_area(&p[0], &p[2], &p[3]);
        if area1 > 1e-9 {
            self.indices.extend_from_slice(&[base, base + 1, base + 2]);
            self.layers.push(layer);
        }
        if area2 > 1e-9 {
            self.indices.extend_from_slice(&[base, base + 2, base + 3]);
            self.layers.push(layer);
        }
    }

    fn tri(&mut self, layer: Layer, p: [[f64; 3]; 3], c: [[u8; 3]; 3]) {
        if tri_area(&p[0], &p[1], &p[2]) <= 1e-9 {
            return;
        }
        let e1 = sub(p[1], p[0]);
        let e2 = sub(p[2], p[0]);
        let mut n = cross(e1, e2);
        if n[1] < 0.0 {
            n = [-n[0], -n[1], -n[2]];
        }
        let n = norm(n);
        let base = self.positions.len() as u32;
        for k in 0..3 {
            self.vert(p[k], n, c[k]);
        }
        self.indices.extend_from_slice(&[base, base + 1, base + 2]);
        self.layers.push(layer);
    }
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm(v: [f64; 3]) -> [f64; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l < 1e-12 { [0.0, 1.0, 0.0] } else { [v[0] / l, v[1] / l, v[2] / l] }
}
fn tri_area(a: &[f64; 3], b: &[f64; 3], c: &[f64; 3]) -> f64 {
    let n = cross(sub(*b, *a), sub(*c, *a));
    0.5 * (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt()
}
fn quad_normal(p: &[[f64; 3]; 4], face_ref: Option<[f64; 2]>) -> Option<[f64; 3]> {
    // Newell normal.
    let mut n = [0.0; 3];
    for i in 0..4 {
        let (a, b) = (p[i], p[(i + 1) % 4]);
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if l < 1e-10 {
        return None;
    }
    let mut n = [n[0] / l, n[1] / l, n[2] / l];
    match face_ref {
        Some(r) => {
            if n[0] * r[0] + n[2] * r[1] < 0.0 {
                n = [-n[0], -n[1], -n[2]];
            }
        }
        None => {
            if n[1] < 0.0 {
                n = [-n[0], -n[1], -n[2]];
            }
        }
    }
    Some(n)
}

#[inline]
fn p3(xz: [f64; 2], y: f64) -> [f64; 3] {
    [xz[0], y, xz[1]]
}

/// Per-entry construction resolved once: stations, datums, kept spans, ramps.
#[derive(Clone, Debug, Default)]
pub struct EntryGeom {
    pub stations: Vec<Frame>,
    pub datums: Vec<f64>,
    pub keep: Vec<bool>,
    /// Kerb depression (0 full kerb .. 1 ramp lip) per station, [left, right].
    pub dep: [Vec<f64>; 2],
}

/// `depAt` of `emitSegment` / the ring: depression across a window list at parameter `v`.
pub fn dep_at(wins: &[RampWin], v: f64) -> f64 {
    let mut best: f64 = 0.0;
    for w in wins {
        if v > w.lo - 1e-9 && v < w.hi + 1e-9 {
            return 1.0;
        }
        if v >= w.flare_lo && v < w.lo && w.lo - w.flare_lo > 1e-9 {
            best = best.max((v - w.flare_lo) / (w.lo - w.flare_lo));
        } else if v > w.hi && v <= w.flare_hi && w.flare_hi - w.hi > 1e-9 {
            best = best.max((w.flare_hi - v) / (w.flare_hi - w.hi));
        }
    }
    best.clamp(0.0, 1.0)
}

impl StreetWorld {
    /// a run's carriageway tint (class hex x segment jitter x the
    /// street's age bleach, street-surface-age.js) and its footway tint (the
    /// per-street pour jitter), as `emitSegment` writes them.
    fn run_tints(&self, ei: usize) -> ([u8; 3], [u8; 3]) {
        let e = &self.plan.entries[ei];
        let seg = &self.city.segments[e.road];
        let m = crate::material::StreetMaterial::get();
        (m.carriageway_tint(&seg.id, seg.street_id.as_deref(), &e.highway), m.footway_tint(&seg.id, seg.street_id.as_deref()))
    }

    /// Kerb top at a station with depression `dep`: the full kerb at 0, the
    /// gutter invert (+ lip and lift) at 1.
    fn kerb_y(&self, datum: f64, dep: f64) -> f64 {
        let o = &self.o;
        let full = o.curb_top_y(datum);
        let lip = datum - o.gutter_depth + o.ramp_lip_height + o.ramp_lift;
        full + (lip - full) * dep
    }

    /// How far back from the kerb line the ramp (or the kerb top) reaches.
    fn ramp_reach(&self, dep: f64) -> f64 {
        self.o.curb_top_width + dep * self.o.ramp_run
    }

    /// The bank foot at plan point `foot` (`bankFootY` on open ground): the
    /// carpet the terrain mesh draws there, less `vergeUndercut`, so the bank
    /// runs into the ground instead of meeting it coplanar. Where the drop is
    /// over `vergeMaxDrop` the JS stands a retaining wall (not ported); the bank
    /// is carried down to the ground instead, steeper, so nothing hangs.
    fn bank_foot_y(&self, foot: [f64; 2], edge_y: f64, own: Option<usize>) -> f64 {
        let mut y = self.carpet_y(foot[0], foot[1]) - self.o.verge_undercut;
        // bankFootY: a foot landing on another street's paving stops just under
        // that street's walk level (its 0.03 m tuck), never above its own edge.
        if let Some(walk) = self.walk_level_beside(foot[0], foot[1], own) {
            y = y.max(edge_y.min(walk - 0.03));
        }
        // a bank never climbs over another street's paving (walk or
        // frontage). Where the ground is above that paving (an uphill bank
        // landing on it, the tan wedge at building corners), the foot is
        // tucked under the paving instead. The JS rejects these intersections
        // in its own admission pass (`bankFootY`'s note; not ported).
        if let Some(pave) = self.paving_level_beside(foot[0], foot[1], own) {
            y = y.min(pave - 0.03);
        }
        y
    }

    /// One bank strip between two outer rail points, outward along `out`.
    #[allow(clippy::too_many_arguments)]
    fn bank(&self, m: &mut TileMesh, own: Option<usize>, a: [f64; 2], ay: f64, a_out: [f64; 2], b: [f64; 2], by: f64, b_out: [f64; 2]) {
        let foot = |p: [f64; 2], y: f64, out: [f64; 2]| {
            let carpet = self.carpet_y(p[0], p[1]);
            let drop = y - (carpet - self.o.verge_undercut);
            let mut reach = self.o.graded_bank_reach(drop);
            // a bank stops where another street's paving starts
            // below it (two streets meeting at different levels): it falls
            // there as a steep face, tucked under that paving, instead of
            // sloping out over it (the wedge over the paving). The JS rejects
            // such a bank in its admission pass and stands a retaining wall
            // where the drop is large; neither is ported.
            let step = 0.25;
            let mut s = step * 0.5;
            while s < reach {
                let q = [p[0] + out[0] * s, p[1] + out[1] * s];
                if let Some(level) = self.paving_level_beside(q[0], q[1], own) {
                    if level < y - 0.05 {
                        reach = (s - step * 0.5).max(0.05);
                        let fp = [p[0] + out[0] * reach, p[1] + out[1] * reach];
                        return (fp, level - 0.03);
                    }
                }
                s += step;
            }
            let fp = [p[0] + out[0] * reach, p[1] + out[1] * reach];
            (fp, self.bank_foot_y(fp, y, own))
        };
        // where this bank's run meets another street's paving standing
        // more than one kerb face (`RETAINING_EDGE_MIN_RISE_M`, the SF
        // 6 in curb) below the walk edge, the JS closes the step with a
        // vertical face in its `retaining` layer (drawn in the kerb face's
        // concrete, `layers.retaining.extra` = the curb-face kind): "a walk
        // further off is a step a face closes, as a retaining edge separates
        // two walks that far apart" (street-surface-v2.js, the plaza and
        // retaining-edge rules). The face stands on the walk's own outer
        // vertices, from the walk edge down under the lower paving, so no
        // sloped bank face (the tan wedge) is drawn over that paving.
        let lower = |p: [f64; 2], y: f64, out: [f64; 2]| -> Option<f64> {
            let step = 0.25;
            let mut s = step * 0.5;
            let reach = self.o.graded_bank_reach(y - (self.carpet_y(p[0], p[1]) - self.o.verge_undercut)).max(step);
            while s < reach {
                let q = [p[0] + out[0] * s, p[1] + out[1] * s];
                if let Some(level) = self.paving_level_beside(q[0], q[1], own) {
                    if level < y - self.o.curb_face_height {
                        return Some(level);
                    }
                }
                s += step;
            }
            None
        };
        // THE RETAINING EDGE (retaining-edge.js `retainingRiseAt`, the
        // street verge's rule): open ground standing more than one kerb face
        // (`RETAINING_EDGE_MIN_RISE_M`) above the back of the walk is held by
        // a wall standing on the footway's own outer vertices, its top at that
        // ground, its foot buried `RETAINING_WALL_TUCK_M` under the walk edge;
        // no bank is drawn on a span that touches it. (Before: the bank graded
        // up into that ground as a steep tan face, the wedge against building
        // bases on the uphill side of a street.)
        let rise = |p: [f64; 2], y: f64, out: [f64; 2]| -> Option<f64> {
            let q = [p[0] + out[0] * 0.3, p[1] + out[1] * 0.3];
            let g = self.carpet_y(q[0], q[1]);
            (g - y > self.o.curb_face_height).then_some(g)
        };
        let (ra, rb) = (rise(a, ay, a_out), rise(b, by, b_out));
        if ra.is_some() || rb.is_some() {
            let (ta, tb) = (ra.unwrap_or(ay).max(ay), rb.unwrap_or(by).max(by));
            let inward = [-(a_out[0] + b_out[0]), -(a_out[1] + b_out[1])];
            m.quad(Layer::CurbFace, [p3(a, ay - RETAINING_TUCK_M), p3(b, by - RETAINING_TUCK_M), p3(b, tb), p3(a, ta)], [CURB_FACE(); 4], Some(inward));
            return;
        }
        // a bank whose run would reach a building (a walk standing over
        // the lower ground beside a building's base, the wedge in
        // sidewalk_walk frame 195) is closed the same way, a face on the walk's
        // outer vertices down under the ground, so no sloped verge face lies
        // against the facade.
        let into_building = |p: [f64; 2], y: f64, out: [f64; 2]| -> bool {
            let drop = y - (self.carpet_y(p[0], p[1]) - self.o.verge_undercut);
            if drop <= self.o.curb_face_height {
                return false;
            }
            let reach = self.o.graded_bank_reach(drop);
            let mut s = 0.25;
            while s <= reach + 1e-6 {
                if self.buildings.contains(p[0] + out[0] * s, p[1] + out[1] * s) {
                    return true;
                }
                s += 0.25;
            }
            false
        };
        let (mut la, mut lb) = (lower(a, ay, a_out), lower(b, by, b_out));
        if into_building(a, ay, a_out) || into_building(b, by, b_out) {
            la = Some(la.unwrap_or_else(|| self.carpet_y(a[0], a[1]) - self.o.verge_undercut));
            lb = Some(lb.unwrap_or_else(|| self.carpet_y(b[0], b[1]) - self.o.verge_undercut));
        }
        if la.is_some() || lb.is_some() {
            let tuck = |l: Option<f64>, p: [f64; 2], y: f64| l.unwrap_or_else(|| self.bank_foot_y(p, y, own)).min(y) - RETAINING_TUCK_M;
            let (ba, bb) = (tuck(la, a, ay), tuck(lb, b, by));
            let face = [a_out[0] + b_out[0], a_out[1] + b_out[1]];
            m.quad(Layer::CurbFace, [p3(a, ay), p3(b, by), p3(b, bb), p3(a, ba)], [CURB_FACE(); 4], Some(face));
            return;
        }
        let (fa, fay) = foot(a, ay, a_out);
        let (fb, fby) = foot(b, by, b_out);
        let edge = tone(VERGE(), 1.15);
        m.quad(Layer::Verge, [p3(a, ay), p3(b, by), p3(fb, fby), p3(fa, fay)], [edge, edge, VERGE(), VERGE()], None);
    }

    /// Frontage depth at a frame and side (a simplified `frontageDepth`): pave to
    /// a facade met within reach (or within `frontagePastLine` past the property
    /// line), else to the surveyed property line; another street's carriageway
    /// stops the march unpaved.
    pub fn frontage_depth(&self, ei: usize, f: &Frame, side: f64, walk: f64) -> f64 {
        let o = &self.o;
        let e = &self.plan.entries[ei];
        let base = e.half + walk;
        let row = self.city.segments[e.road].right_of_way.filter(|r| *r > 0.0);
        let to_line = row.map(|r| (r / 2.0 - base).max(0.0));
        let limit = to_line.unwrap_or(o.frontage_reach);
        let far = match to_line {
            Some(l) => (l + o.frontage_past_line).max(o.frontage_reach),
            None => o.frontage_reach,
        };
        let step = o.frontage_step.max(0.05);
        let mut t = step;
        while t <= far + 1e-6 {
            let u = side * (base + t);
            let x = f.x + f.nx * u * f.miter;
            let z = f.z + f.nz * u * f.miter;
            if self.buildings.contains(x, z) {
                return if t <= limit + o.frontage_past_line + 1e-6 || to_line.is_none() { t } else { to_line.unwrap_or(0.0) };
            }
            if self.on_other_carriageway(x, z, ei) {
                return if t <= limit { 0.0 } else { to_line.unwrap_or(0.0) };
            }
            t += step;
        }
        to_line.unwrap_or(0.0)
    }

    // -----------------------------------------------------------------------
    // ribbons
    // -----------------------------------------------------------------------

    pub(crate) fn emit_span(&self, m: &mut TileMesh, ei: usize, i: usize) {
        let o = &self.o;
        let e: &Entry = &self.plan.entries[ei];
        let g = &self.geoms[ei];
        let (a, b) = (g.stations[i], g.stations[i + 1]);
        let (da, db) = (g.datums[i], g.datums[i + 1]);
        let half = e.half;
        // The JS palette's tint for this run's highway class (the per-block
        // tone the JS varies in its shader's macro field, not per run).
        let (asphalt, walk_tint) = self.run_tints(ei);
        // Carriageway and gutter pans.
        let offs = o.section_offsets(half);
        let gutter_start = (half - o.gutter_width).max(0.0);
        let gutter = GUTTER();
        let col = |u: f64| if u.abs() <= gutter_start + 1e-9 { asphalt } else { gutter };
        for k in 0..offs.len() - 1 {
            let (u0, u1) = (offs[k], offs[k + 1]);
            // Distance to the kerb line per vertex (the shader's cross fall and silt).
            let (k0, k1) = ((half - u0.abs()).max(0.0) as f32, (half - u1.abs()).max(0.0) as f32);
            m.quad_kerb(
                Layer::Carriageway,
                [
                    p3(a.at(u0), o.cross_section_y(da, u0, half)),
                    p3(b.at(u0), o.cross_section_y(db, u0, half)),
                    p3(b.at(u1), o.cross_section_y(db, u1, half)),
                    p3(a.at(u1), o.cross_section_y(da, u1, half)),
                ],
                [col(u0), col(u0), col(u1), col(u1)],
                [k0, k0, k1, k1],
            );
        }
        // Kerb, footway, frontage and bank per side.
        let mid_s = (a.s + b.s) / 2.0;
        // a stub raised on an approach embankment (`joins::StructureEnd::rise`):
        // a side with no footway gets its bank from the carriageway's edge.
        let raised = (e.structure_ends.iter().any(Option::is_some) || !e.structure_ramps.is_empty()) && (self.datum_raise(ei, i) > RAISED_M || self.datum_raise(ei, i + 1) > RAISED_M);
        for (k, side) in [(0usize, 1.0f64), (1, -1.0)] {
            let walk = e.walks[k];
            if !(walk >= o.min_sidewalk_width) {
                if raised {
                    let (out_a, out_b) = ([side * a.nx, side * a.nz], [side * b.nx, side * b.nz]);
                    self.bank(m, Some(ei), a.at(side * half), da - o.gutter_depth, out_a, b.at(side * half), db - o.gutter_depth, out_b);
                }
                continue;
            }
            if e.walk_yield_spans[k].iter().any(|sp| mid_s >= sp[0] && mid_s <= sp[1]) {
                continue;
            }
            let (pa, pb) = (g.dep[k][i], g.dep[k][i + 1]);
            let (ia, ib) = (da - o.gutter_depth, db - o.gutter_depth);
            let (ka, kb) = (self.kerb_y(da, pa), self.kerb_y(db, pb));
            let face = [side * a.nx * -1.0, side * a.nz * -1.0];
            // each vertex's height above the gutter flow line (the renderer's kerb-face grime).
            m.quad_kerb_face(
                Layer::CurbFace,
                [p3(a.at(side * half), ia), p3(b.at(side * half), ib), p3(b.at(side * half), kb), p3(a.at(side * half), ka)],
                [CURB_FACE(); 4],
                Some(face),
                [0.0, 0.0, (kb - ib).max(0.0) as f32, (ka - ia).max(0.0) as f32],
            );
            let (ra, rb) = (self.ramp_reach(pa), self.ramp_reach(pb));
            let (wya, wyb) = (o.sidewalk_surface_y(da, half + ra, half), o.sidewalk_surface_y(db, half + rb, half));
            let (ramp_c, top_c) = (RAMP(), CURB_TOP());
            let top_col = |d: f64| if d > 0.0 { ramp_c } else { top_c };
            m.quad(
                if pa > 0.0 || pb > 0.0 { Layer::Ramp } else { Layer::CurbTop },
                [p3(a.at(side * half), ka), p3(b.at(side * half), kb), p3(b.at(side * (half + rb)), wyb), p3(a.at(side * (half + ra)), wya)],
                [top_col(pa), top_col(pb), top_col(pb), top_col(pa)],
                None,
            );
            let (eya, eyb) = (o.sidewalk_surface_y(da, half + walk, half), o.sidewalk_surface_y(db, half + walk, half));
            if walk > ra.max(rb) + 0.02 {
                let wc = walk_tint;
                // each vertex's distance across the walk from the kerb line.
                m.quad_kerb(
                    Layer::Sidewalk,
                    [p3(a.at(side * (half + ra)), wya), p3(b.at(side * (half + rb)), wyb), p3(b.at(side * (half + walk)), eyb), p3(a.at(side * (half + walk)), eya)],
                    [wc; 4],
                    [ra as f32, rb as f32, walk as f32, walk as f32],
                );
            }
            // Frontage: level with the walk edge, FRONTAGE_LIFT_M proud.
            let (fa, fb) = (self.frontage_depth(ei, &a, side, walk), self.frontage_depth(ei, &b, side, walk));
            let step = o.frontage_step;
            let (mut ox_a, mut ox_b, mut oy_a, mut oy_b) = (half + walk, half + walk, eya, eyb);
            if fa > step || fb > step {
                let (ya, yb) = (eya + o.frontage_lift, eyb + o.frontage_lift);
                m.quad(
                    Layer::Sidewalk,
                    [
                        p3(a.at(side * (half + walk)), ya),
                        p3(b.at(side * (half + walk)), yb),
                        p3(b.at(side * (half + walk + fb)), yb),
                        p3(a.at(side * (half + walk + fa)), ya),
                    ],
                    [walk_tint; 4],
                    None,
                );
                ox_a += fa;
                ox_b += fb;
                oy_a = ya;
                oy_b = yb;
            }
            // Verge bank from the outer edge down (or up) to the carpet.
            let out_a = [side * a.nx, side * a.nz];
            let out_b = [side * b.nx, side * b.nz];
            if self.by_leg_junction(e, mid_s) {
                // (by a junction with a structure's leg: a face, as its corners.)
                self.ring_edge(m, a.at(side * ox_a), oy_a, out_a, b.at(side * ox_b), oy_b, out_b, true);
            } else {
                self.bank(m, Some(ei), a.at(side * ox_a), oy_a, out_a, b.at(side * ox_b), oy_b, out_b);
            }
        }
        // Lane paint along this span.
        self.emit_span_paint(m, ei, i);
        // a stub's end at a structure (`joins`): its open section capped.
        if e.structure_ends[0].is_some() && i == 0 && e.approach_start.is_none() {
            self.emit_stub_cap(m, ei, 0);
        }
        if e.structure_ends[1].is_some() && i + 2 == g.stations.len() && e.approach_end.is_none() {
            self.emit_stub_cap(m, ei, 1);
        }
    }

    /// Station `s` of run `e` lies within its junction trim and a bank's
    /// reach of an end at a junction with a structure's leg (`joins`): its
    /// banks there would lie over the junction's pad.
    fn by_leg_junction(&self, e: &Entry, s: f64) -> bool {
        if self.o.structures.is_none() {
            return false;
        }
        [(e.approach_start, s), (e.approach_end, e.length - s)].iter().any(|&(app, d)| {
            app.is_some_and(|(n, a)| {
                let node = &self.plan.nodes[n];
                node.approaches.iter().any(|x| self.plan.entries[x.entry].structure_leg.is_some()) && d < node.approaches[a].trim + self.o.verge_reach + self.o.max_step
            })
        })
    }

    /// The cap over a stub's open end where it meets a structure
    /// (`joins::StructureEnd`): a vertical face in the kerb face's concrete
    /// over the stub's section beyond what the structure's own end face
    /// closes (its roadway and barriers, `StructureEnd::closed`), from the
    /// drawn surface (kerb top, ramp, footway, frontage, the bank to its
    /// foot) down under the ground carpet, `CAP_SET_BACK_M` inside the stub.
    fn emit_stub_cap(&self, m: &mut TileMesh, ei: usize, end: usize) {
        let o = &self.o;
        let e = &self.plan.entries[ei];
        let g = &self.geoms[ei];
        let Some(joins) = o.structures.as_ref() else { return };
        let Some(se) = e.structure_ends[end].and_then(|j| joins.ends.get(j)) else { return };
        let n = g.stations.len();
        if n < 2 {
            return;
        }
        let (i, j) = if end == 0 { (0, 1) } else { (n - 1, n - 2) };
        let (f0, f1) = (g.stations[i], g.stations[j]);
        let step = (f1.s - f0.s).abs().max(1e-6);
        let t = (CAP_SET_BACK_M / step).min(0.5);
        let lerp = |p: f64, q: f64| p + (q - p) * t;
        let f = plan::Frame { s: lerp(f0.s, f1.s), x: lerp(f0.x, f1.x), z: lerp(f0.z, f1.z), nx: f0.nx, nz: f0.nz, miter: f0.miter, tx: f0.tx, tz: f0.tz };
        let d = lerp(g.datums[i], g.datums[j]);
        let out = if end == 0 { [-f0.tx, -f0.tz] } else { [f0.tx, f0.tz] };
        let half = e.half;
        let raised = self.datum_raise(ei, i) > RAISED_M;
        for (k, side) in [(0usize, 1.0f64), (1, -1.0)] {
            let walk = e.walks[k];
            let drawn = walk >= o.min_sidewalk_width && !e.walk_yield_spans[k].iter().any(|sp| f.s >= sp[0] && f.s <= sp[1]);
            if !drawn && !raised {
                continue;
            }
            // (offset, top) across the section, outward from the carriageway.
            let mut prof: Vec<(f64, f64)> = vec![(0.0, o.cross_section_y(d, 0.0, half)), (half, d - o.gutter_depth)];
            let mut outer = (half, d - o.gutter_depth);
            if drawn {
                let dep = mesh_dep(g, k, i);
                let ra = self.ramp_reach(dep);
                prof.push((half, self.kerb_y(d, dep)));
                prof.push((half + ra, o.sidewalk_surface_y(d, half + ra, half)));
                let ey = o.sidewalk_surface_y(d, half + walk, half);
                prof.push((half + walk, ey));
                outer = (half + walk, ey);
                let fd = self.frontage_depth(ei, &f, side, walk);
                if fd > o.frontage_step {
                    prof.push((half + walk, ey + o.frontage_lift));
                    prof.push((half + walk + fd, ey + o.frontage_lift));
                    outer = (half + walk + fd, ey + o.frontage_lift);
                }
            }
            // the bank to its foot (`bank`'s graded reach).
            let edge = f.at(side * outer.0);
            let drop = outer.1 - (self.carpet_y(edge[0], edge[1]) - o.verge_undercut);
            let reach = o.graded_bank_reach(drop);
            let foot = f.at(side * (outer.0 + reach));
            prof.push((outer.0 + reach, self.bank_foot_y(foot, outer.1, Some(ei))));
            // clipped to what the structure's own end face leaves open.
            let lo = se.closed.max(0.0);
            let mut pts: Vec<(f64, f64)> = vec![];
            for w in prof.windows(2) {
                let ((u0, y0), (u1, y1)) = (w[0], w[1]);
                if u1 <= lo {
                    continue;
                }
                if u0 < lo && u1 > u0 {
                    let t = (lo - u0) / (u1 - u0);
                    pts.push((lo, y0 + (y1 - y0) * t));
                } else if pts.is_empty() || pts.last() != Some(&(u0, y0)) {
                    pts.push((u0, y0));
                }
                pts.push((u1, y1));
            }
            for w in pts.windows(2) {
                let ((u0, y0), (u1, y1)) = (w[0], w[1]);
                if (u1 - u0).abs() < 1e-4 {
                    continue;
                }
                let (p0, p1) = (f.at(side * u0), f.at(side * u1));
                let base = |p: [f64; 2], top: f64| (self.carpet_y(p[0], p[1]) - o.verge_undercut).min(top) - RETAINING_TUCK_M;
                m.quad(Layer::CurbFace, [p3(p0, y0), p3(p1, y1), p3(p1, base(p1, y1)), p3(p0, base(p0, y0))], [CURB_FACE(); 4], Some(out));
            }
        }
    }

    fn emit_span_paint(&self, m: &mut TileMesh, ei: usize, i: usize) {
        let o = &self.o;
        let e = &self.plan.entries[ei];
        let g = &self.geoms[ei];
        let lines = &self.lines[ei];
        if lines.is_empty() {
            return;
        }
        let (a, b) = (g.stations[i], g.stations[i + 1]);
        let lo = a.s.max(e.marking_trim_start).max(e.s0());
        let hi = b.s.min(e.length - e.marking_trim_end).min(e.s1());
        if !(hi - lo > 0.05) {
            return;
        }
        let (da, db) = (g.datums[i], g.datums[i + 1]);
        let span = (b.s - a.s).max(1e-9);
        let at = |s: f64, u: f64| -> [f64; 3] {
            let t = ((s - a.s) / span).clamp(0.0, 1.0);
            let (pa, pb) = (a.at(u), b.at(u));
            let xz = [pa[0] + (pb[0] - pa[0]) * t, pa[1] + (pb[1] - pa[1]) * t];
            let d = da + (db - da) * t;
            [xz[0], o.cross_section_y(d, u, e.half) + o.marking_lift, xz[1]]
        };
        let (mark, gap) = crate::paint::broken_line_pattern(&self.city.segments[e.road], &e.highway, o);
        for l in lines {
            let c = match l.paint {
                crate::paint::Paint::White => WHITE(),
                crate::paint::Paint::Yellow => YELLOW(),
                crate::paint::Paint::Green => GREEN(),
                crate::paint::Paint::None => continue,
            };
            if !(l.width > 0.0) {
                continue;
            }
            let (u0, u1) = (l.u - l.width / 2.0, l.u + l.width / 2.0);
            let mut pieces: Vec<[f64; 2]> = vec![];
            if l.dashed {
                let (dm, dg) = if l.role == "centre-turn-boundary" { (o.twltl_dash_mark, o.twltl_dash_gap) } else { (mark, gap) };
                let period = dm + dg;
                // Dashes are phased from the run's start, so neighbouring spans agree.
                let mut k = ((lo - e.marking_trim_start) / period).floor();
                loop {
                    let d0 = e.marking_trim_start + k * period;
                    if d0 > hi {
                        break;
                    }
                    let (s0, s1) = (d0.max(lo), (d0 + dm).min(hi));
                    if s1 > s0 + 0.02 {
                        pieces.push([s0, s1]);
                    }
                    k += 1.0;
                }
            } else {
                pieces.push([lo, hi]);
            }
            for [s0, s1] in pieces {
                if e.in_yield_span((s0 + s1) / 2.0) {
                    continue;
                }
                // each vertex's across-the-line coordinate, 0..1 (the renderer's worn-paint mask follows the line).
                m.quad_kerb(Layer::Marking, [at(s0, u0), at(s1, u0), at(s1, u1), at(s0, u1)], [c; 4], [0.0, 0.0, 1.0, 1.0]);
            }
        }
    }

    // -----------------------------------------------------------------------
    // junctions
    // -----------------------------------------------------------------------

    /// The datum of approach `app`'s trimmed end frame (`junctionDatums`).
    fn approach_datum(&self, app: &Approach) -> (Frame, f64) {
        let f = crate::plan::approach_end_frame(&self.plan.entries, app);
        (f, self.ground(f.x, f.z) + self.o.road_lift)
    }

    pub(crate) fn emit_node(&self, m: &mut TileMesh, ni: usize) {
        let o = &self.o;
        let node: &Node = &self.plan.nodes[ni];
        let n = node.approaches.len();
        if n == 0 {
            return;
        }
        let ends: Vec<(Frame, f64)> = node.approaches.iter().map(|a| self.approach_datum(a)).collect();
        // --- the pad: one ring round the node, fanned from its centre. -------
        let mut ring: Vec<([f64; 2], f64)> = vec![];
        for (i, app) in node.approaches.iter().enumerate() {
            let (f, d) = ends[i];
            let s = if app.at_start { 1.0 } else { -1.0 };
            for lat in o.section_offsets(app.half) {
                ring.push((f.at(s * lat), o.cross_section_y(d, lat, app.half)));
            }
            // The corner path from this approach to the next (interior stations: the kerb line).
            let path = &node.paths[i];
            let datums = self.path_datums(&path.stations, ends[path.a].1, ends[path.b].1);
            let last = path.stations.len().saturating_sub(1);
            for (k, st) in path.stations.iter().enumerate() {
                if k == 0 || k == last {
                    continue;
                }
                ring.push(([st.x, st.z], datums[k] - o.gutter_depth));
            }
        }
        let mean_d = ends.iter().map(|e| e.1).sum::<f64>() / n as f64;
        let mean_half = node.approaches.iter().map(|a| a.half).sum::<f64>() / n as f64;
        let apex = p3(node.position, mean_d + o.cross_slope * mean_half);
        let pad = PAD();
        for k in 0..ring.len() {
            let (p, py) = ring[k];
            let (q, qy) = ring[(k + 1) % ring.len()];
            if (p[0] - q[0]).abs() < 1e-9 && (p[1] - q[1]).abs() < 1e-9 {
                continue;
            }
            m.tri(Layer::Carriageway, [apex, p3(q, qy), p3(p, py)], [pad, pad, pad]);
        }
        // --- corner returns: kerb, ramps, footway, bank. ----------------------
        for path in &node.paths {
            let datums = self.path_datums(&path.stations, ends[path.a].1, ends[path.b].1);
            let st = &path.stations;
            let reach_full = o.curb_top_width + o.ramp_run;
            // (a junction with a structure's leg, `joins`: no bank on its
            // corners: the structure's walls and fill stand round it.)
            let leg = node.approaches.iter().any(|a| self.plan.entries[a.entry].structure_leg.is_some());
            let deps: Vec<f64> = st
                .iter()
                .map(|s| match s.ang {
                    Some(ang) if !path.ramps.is_empty() && s.walk > reach_full + 0.05 => dep_at(&path.ramps, ang),
                    _ => 0.0,
                })
                .collect();
            for k in 0..st.len().saturating_sub(1) {
                let (sa, sb) = (&st[k], &st[k + 1]);
                if (sa.x - sb.x).abs() < 1e-7 && (sa.z - sb.z).abs() < 1e-7 {
                    continue;
                }
                self.ring_span(m, sa, sb, datums[k], datums[k + 1], deps[k], deps[k + 1], leg);
            }
        }
        // --- junction paint: crosswalk bars and limit lines. ------------------
        // (none on a structure's leg: its mouth is the structure's own deck.)
        if node.earns_paint {
            for (i, app) in node.approaches.iter().enumerate() {
                if self.plan.entries[app.entry].structure_leg.is_some() {
                    continue;
                }
                self.emit_approach_paint(m, node, app, ends[i].1);
            }
        }
    }

    /// One datum per corner station, blended by chord length from A's to B's.
    fn path_datums(&self, st: &[CurbStation], da: f64, db: f64) -> Vec<f64> {
        let mut cum = vec![0.0];
        for k in 1..st.len() {
            cum.push(cum[k - 1] + ((st[k].x - st[k - 1].x).powi(2) + (st[k].z - st[k - 1].z).powi(2)).sqrt());
        }
        let total = cum.last().copied().unwrap_or(0.0).max(1e-9);
        cum.iter().map(|c| da + (db - da) * c / total).collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn ring_span(&self, m: &mut TileMesh, sa: &CurbStation, sb: &CurbStation, da: f64, db: f64, pa: f64, pb: f64, leg: bool) {
        let o = &self.o;
        let off = |s: &CurbStation, d: f64| [s.x + s.out[0] * d * s.scale, s.z + s.out[1] * d * s.scale];
        let (ia, ib) = (da - o.gutter_depth, db - o.gutter_depth);
        let (ka, kb) = (self.kerb_y(da, pa), self.kerb_y(db, pb));
        let face = [-(sa.out[0] + sb.out[0]), -(sa.out[1] + sb.out[1])];
        // each vertex's height above the gutter flow line (the renderer's kerb-face grime).
        m.quad_kerb_face(Layer::CurbFace, [p3([sa.x, sa.z], ia), p3([sb.x, sb.z], ib), p3([sb.x, sb.z], kb), p3([sa.x, sa.z], ka)], [CURB_FACE(); 4], Some(face), [0.0, 0.0, (kb - ib).max(0.0) as f32, (ka - ia).max(0.0) as f32]);
        let (wa, wb) = (sa.walk, sb.walk);
        if !(wa > 0.0 || wb > 0.0) {
            // A side with no footway: the bank starts at the kerb top.
            self.ring_edge(m, [sa.x, sa.z], ka, sa.out, [sb.x, sb.z], kb, sb.out, leg);
            return;
        }
        let (ra, rb) = (self.ramp_reach(pa).min(wa.max(o.curb_top_width)), self.ramp_reach(pb).min(wb.max(o.curb_top_width)));
        let walk_y = |d: f64, dist: f64| o.curb_top_y(d) + o.footway_rise_at(dist);
        let (ramp_c, top_c) = (RAMP(), CURB_TOP());
        let top_col = |p: f64| if p > 0.0 { ramp_c } else { top_c };
        m.quad(
            if pa > 0.0 || pb > 0.0 { Layer::Ramp } else { Layer::CurbTop },
            [p3([sa.x, sa.z], ka), p3([sb.x, sb.z], kb), p3(off(sb, rb), walk_y(db, rb)), p3(off(sa, ra), walk_y(da, ra))],
            [top_col(pa), top_col(pb), top_col(pb), top_col(pa)],
            None,
        );
        if wa > ra + 0.02 || wb > rb + 0.02 {
            let (wa2, wb2) = (wa.max(ra), wb.max(rb));
            m.quad(
                Layer::Sidewalk,
                [p3(off(sa, ra), walk_y(da, ra)), p3(off(sb, rb), walk_y(db, rb)), p3(off(sb, wb2), walk_y(db, wb2)), p3(off(sa, wa2), walk_y(da, wa2))],
                [WALK(); 4],
                None,
            );
        }
        let (wa2, wb2) = (wa.max(ra), wb.max(rb));
        self.ring_edge(m, off(sa, wa2), walk_y(da, wa2), sa.out, off(sb, wb2), walk_y(db, wb2), sb.out, leg);
    }

    /// The outer edge of a corner return: its verge bank; or, at a junction
    /// with a structure's leg (`joins`), a face in the kerb face's concrete
    /// down under the ground carpet (the structure's own walls and fill stand
    /// round it, where a sloped bank would lie over paving).
    #[allow(clippy::too_many_arguments)]
    fn ring_edge(&self, m: &mut TileMesh, a: [f64; 2], ay: f64, a_out: [f64; 2], b: [f64; 2], by: f64, b_out: [f64; 2], leg: bool) {
        if !leg {
            self.bank(m, None, a, ay, a_out, b, by, b_out);
            return;
        }
        let foot = |p: [f64; 2], y: f64| (self.carpet_y(p[0], p[1]) - self.o.verge_undercut).min(y) - RETAINING_TUCK_M;
        let face = [a_out[0] + b_out[0], a_out[1] + b_out[1]];
        m.quad(Layer::CurbFace, [p3(a, ay), p3(b, by), p3(b, foot(b, by)), p3(a, foot(a, ay))], [CURB_FACE(); 4], Some(face));
    }

    fn emit_approach_paint(&self, m: &mut TileMesh, node: &Node, app: &Approach, _d_end: f64) {
        let o = &self.o;
        let e = &self.plan.entries[app.entry];
        let g = &self.geoms[app.entry];
        let available = app.run_length - if app.at_start { e.trim_end } else { e.trim_start };
        let band0 = app.trim + o.crosswalk_clearance;
        let band1 = band0 + node.band_depth;
        let lift = o.junction_paint_lift;
        let sgn = if app.at_start { 1.0 } else { -1.0 };
        // A point `d` from the node along the approach, `lat` across it
        // (+perpCCW(u)), on the cambered carriageway.
        let at = |d: f64, lat: f64| -> [f64; 3] {
            let s = if app.at_start { d } else { e.length - d };
            let s = s.clamp(0.0, e.length);
            let f = e.frame(s, false);
            let datum = self.datum_at_station(g, s).unwrap_or_else(|| self.ground(f.x, f.z) + o.road_lift);
            let xz = f.at(sgn * lat);
            [xz[0], o.cross_section_y(datum, lat, app.half) + lift, xz[1]]
        };
        if app.crossing_painted && band1 + 0.4 <= available {
            let stripes = crate::paint::crossing_stripes(app.half, o);
            for v in &stripes.stripes {
                let dm = (band0 + band1) / 2.0;
                for [d0, d1] in [[band0, dm], [dm, band1]] {
                    m.quad_kerb(Layer::Marking, [at(d0, v[0]), at(d1, v[0]), at(d1, v[1]), at(d0, v[1])], [CROSSWALK(); 4], [0.0, 0.0, 1.0, 1.0]);
                }
            }
        }
        // Limit line on the approach traffic arrives by.
        if app.flows_toward && band1 + o.stop_bar_clearance + o.stop_bar_depth + 0.2 <= available {
            let d0 = band1 + o.stop_bar_clearance;
            let d1 = d0 + o.stop_bar_depth;
            let inset = o.stop_bar_edge_inset;
            let (lo, hi) = if app.oneway {
                (-app.half + inset, app.half - inset)
            } else {
                // Right-hand traffic arriving along -u keeps to -perpCCW(u):
                // from the kerb to the direction boundary.
                let boundary = self.lines[app.entry]
                    .iter()
                    .find(|l| l.role == "centre" || l.role == "unpainted-centre" || l.role == "centre-turn-boundary")
                    .map(|l| l.u * sgn)
                    .unwrap_or(0.0);
                (-app.half + inset, boundary.min(app.half - inset))
            };
            if hi - lo > 0.5 {
                m.quad_kerb(Layer::Marking, [at(d0, lo), at(d1, lo), at(d1, hi), at(d0, hi)], [WHITE(); 4], [0.0, 1.0, 1.0, 0.0]);
            }
        }
    }

    /// The datum the ribbon draws at arc length `s` (linear between its stations).
    pub fn datum_at_station(&self, g: &EntryGeom, s: f64) -> Option<f64> {
        let st = &g.stations;
        if st.len() < 2 || s < st[0].s - 1e-6 || s > st[st.len() - 1].s + 1e-6 {
            return None;
        }
        let k = st.partition_point(|f| f.s <= s).clamp(1, st.len() - 1);
        let (a, b) = (st[k - 1], st[k]);
        let t = ((s - a.s) / (b.s - a.s).max(1e-9)).clamp(0.0, 1.0);
        Some(g.datums[k - 1] + (g.datums[k] - g.datums[k - 1]) * t)
    }

    // -----------------------------------------------------------------------
    // far level
    // -----------------------------------------------------------------------

    /// The far level of a span: the carriageway and both footways as flat
    /// strips at the drawn datums, no kerb face, no paint.
    pub(crate) fn emit_far_span(&self, m: &mut TileMesh, ei: usize, i: usize, j: usize) {
        let o = &self.o;
        let e = &self.plan.entries[ei];
        let g = &self.geoms[ei];
        let (a, b) = (g.stations[i], g.stations[j]);
        let (da, db) = (g.datums[i], g.datums[j]);
        let h = e.half;
        m.quad(
            Layer::Carriageway,
            [p3(a.at(-h), da), p3(b.at(-h), db), p3(b.at(h), db), p3(a.at(h), da)],
            [self.run_tints(ei).0; 4],
            None,
        );
        for (k, side) in [(0usize, 1.0f64), (1, -1.0)] {
            let w = e.walks[k];
            if !(w >= o.min_sidewalk_width) {
                continue;
            }
            let (ya, yb) = (o.sidewalk_surface_y(da, h + w, h), o.sidewalk_surface_y(db, h + w, h));
            m.quad(
                Layer::Sidewalk,
                [p3(a.at(side * h), o.curb_top_y(da)), p3(b.at(side * h), o.curb_top_y(db)), p3(b.at(side * (h + w)), yb), p3(a.at(side * (h + w)), ya)],
                [self.run_tints(ei).1; 4],
                None,
            );
        }
    }

    /// The coarse level of a run's spans `i..j` (every 4th station): the
    /// carriageway and its footways as one flat strip.
    pub(crate) fn emit_coarse_span(&self, m: &mut TileMesh, ei: usize, i: usize, j: usize) {
        let e = &self.plan.entries[ei];
        let g = &self.geoms[ei];
        let (a, b) = (g.stations[i], g.stations[j]);
        let (da, db) = (g.datums[i], g.datums[j]);
        let wl = if e.walks[0] >= self.o.min_sidewalk_width { e.walks[0] } else { 0.0 };
        let wr = if e.walks[1] >= self.o.min_sidewalk_width { e.walks[1] } else { 0.0 };
        let (l, r) = (e.half + wl, -(e.half + wr));
        m.quad(Layer::Carriageway, [p3(a.at(r), da), p3(b.at(r), db), p3(b.at(l), db), p3(a.at(l), da)], [self.run_tints(ei).0; 4], None);
    }

    /// The far level of a node: its pad over the mouths' kerb corners.
    pub(crate) fn emit_far_node(&self, m: &mut TileMesh, ni: usize) {
        let o = &self.o;
        let node = &self.plan.nodes[ni];
        let mut ring: Vec<([f64; 2], f64)> = vec![];
        let mut sum = 0.0;
        for app in &node.approaches {
            let (f, d) = self.approach_datum(app);
            let s = if app.at_start { 1.0 } else { -1.0 };
            ring.push((f.at(-s * app.half), d));
            ring.push((f.at(s * app.half), d));
            sum += d;
        }
        if ring.len() < 3 {
            return;
        }
        let apex = p3(node.position, sum / node.approaches.len() as f64);
        for k in 0..ring.len() {
            let (p, py) = ring[k];
            let (q, qy) = ring[(k + 1) % ring.len()];
            m.tri(Layer::Carriageway, [apex, p3(q, qy), p3(p, py)], [PAD(); 3]);
        }
        let _ = (o, PI);
    }
}
