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
//!   junction pad triangulates the approaches' mouth sections and the corner
//!                kerb lines; corner returns carry the kerb, ramps
//!                and footway round the fillet or the chord
//!   paint        lane lines (dashed per CA MUTCD broken-line pattern), centre
//!                lines, edge lines, cycle lanes, continental crosswalk bars,
//!                limit lines, lifted `markingLift` / `junctionPaintLift`
//!
//! Every primitive belongs to the tile holding its anchor (a span's centreline
//! midpoint, a node's position), so tiles never draw the same triangle twice
//! and need no clipping. Colours are placeholder tones for this slice (the
//! JS tints a scanned texture; no texture is drawn here), not a look.

mod marking;
mod apron;
pub(crate) mod junction;
pub(crate) mod surveyed_junction;
pub(crate) use apron::Apron;

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
pub(crate) fn CURB_FACE() -> [u8; 3] {
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
/// (`--bikeways`) A bike lane's raised island stops this short of the run's
/// marked ends (m), its nose clear of the crosswalk. INFERRED.
const ISLAND_END_M: f64 = 1.5;

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

/// A corner return whose walk lies this far inside its own junction's pad
/// draws no walk (`ring_span`), game metres: its approach was trimmed short,
/// and the walk would lie across the cross street's roadway.
const CORNER_IN_PAD_M: f64 = 1.5;

/// A junction's pad follows the survey filtered over this, game metres (`pad_ground`).
const PAD_SMOOTH_M: f64 = 4.0;

/// (`wall_coping`) a retaining wall rising more than this over the walk is
/// capped (`bank`), game metres.
const COPING_MIN_RISE_M: f64 = 0.35;
/// A capped wall stands this far over the ground it holds, game metres.
const COPING_UPSTAND_M: f64 = 0.10;
/// A wall's coping runs this far back over that ground, game metres (a
/// cast-in-place wall's 12 in top; INFERRED, no SF standard recorded).
const COPING_WIDTH_M: f64 = 0.30;

/// (`--no-wall-coping`, `EO_WALL_COPING=0`) a retaining wall behind a walk is
/// a bare face up to the ground, as before; on (the default) a wall over
/// `COPING_MIN_RISE_M` stands `COPING_UPSTAND_M` proud, capped by a coping.
pub fn wall_coping() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("EO_WALL_COPING").map(|v| v.trim() != "0" && v.trim() != "off").unwrap_or(true))
}

/// A bank whose run another paving cuts shorter than this at both ends is
/// drawn as a face (`bank`), game metres.
const BANK_SLIVER_M: f64 = 0.1;

/// A junction pad's fan is subdivided this far apart along its spokes when
/// it follows the ground (`junction_ground`), game metres: the 8 m terrain
/// grid's half on the 1.25 plan's metres (an INFERRED sampling of a smooth surface).
pub const JUNCTION_GROUND_STEP_M: f64 = 4.0;

/// (`--no-junction-ground`, `EO_JUNCTION_GROUND=0`) a junction's pad is one
/// flat fan from an apex at its approaches' mean datum, and its corners blend
/// their datums in a straight line from mouth to mouth, as the JS drew them;
/// on (the default), both follow the ground between the mouths (an acute
/// fork's pad, cut back tens of metres, was one slab across its slope).
pub fn junction_ground() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("EO_JUNCTION_GROUND").map(|v| v.trim() != "0" && v.trim() != "off").unwrap_or(true))
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
    pub(crate) fn open_roadways(&mut self, openings: &crate::road_openings::RoadOpenings) {
        if self.indices.is_empty() || openings.is_empty() { return; }
        let (lo, hi) = self.bounds();
        if !openings.intersects_bounds(lo, hi) { return; }
        self.clip_faces(0, openings, |layer| !matches!(layer, Layer::Marking | Layer::Carriageway));
    }

    pub(crate) fn clip_faces(&mut self, first: usize, openings: &crate::road_openings::RoadOpenings, include: impl Fn(Layer) -> bool) {
        self.clip_faces_with(first, |_, layer, triangle| {
            if include(layer) { openings.clip(triangle) } else { None }
        });
    }

    pub(crate) fn clip_faces_with(&mut self, first: usize, mut clip: impl FnMut(usize, Layer, [[f32; 3]; 3]) -> Option<Vec<[crate::road_openings::Vertex; 3]>>) {
        let old = self.indices.split_off(first * 3);
        let layers = self.layers.split_off(first);
        for (t, (face, layer)) in old.chunks_exact(3).zip(layers).enumerate() {
            let ids = [face[0], face[1], face[2]].map(|i| i as usize);
            let source = ids.map(|i| self.positions[i]);
            let Some(pieces) = clip(first+t, layer, source) else {
                self.indices.extend_from_slice(face);
                self.layers.push(layer);
                continue;
            };
            for triangle in pieces {
                let positions = triangle.map(|v| v.mesh_position(source));
                let [a,b,c] = positions.map(|p| p.map(f64::from));
                if tri_area(&a,&b,&c) <= 1e-9 { continue; }
                if layer == Layer::Verge {
                    // A top face narrower than one position unit at this
                    // location collapses onto the pavement edge on the GPU.
                    let area2 = ((b[0]-a[0])*(c[2]-a[2])-(b[2]-a[2])*(c[0]-a[0])).abs();
                    let edge = [(&a,&b),(&b,&c),(&c,&a)].iter().map(|(u,v)| (u[0]-v[0]).hypot(u[2]-v[2])).fold(0.0,f64::max);
                    let ulp = positions.iter().flat_map(|p| [p[0],p[2]]).map(|q| f64::from(q.abs().next_up()-q.abs())).fold(0.0,f64::max);
                    if area2 <= edge * ulp { continue; }
                }
                let k = self.positions.len() as u32;
                for (v,p) in triangle.into_iter().zip(positions) {
                    self.positions.push(p);
                    self.normals.push(v.interpolate(ids.map(|i| self.normals[i])));
                    self.colors.push(v.color(ids.map(|i| self.colors[i])));
                    self.kerb.push(v.interpolate(ids.map(|i| [self.kerb[i]]))[0]);
                }
                self.indices.extend_from_slice(&[k, k+1, k+2]);
                self.layers.push(layer);
            }
        }
    }

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
    pub(crate) fn quad(&mut self, layer: Layer, p: [[f64; 3]; 4], c: [[u8; 3]; 4], face_ref: Option<[f64; 2]>) {
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

    /// `tri`, recording each vertex's across-the-marking coordinate (a
    /// marking's wear mask reads it; -1, `tri`'s, reads as worn away).
    fn tri_kerb(&mut self, layer: Layer, p: [[f64; 3]; 3], c: [[u8; 3]; 3], kerb: [f32; 3]) {
        let first = self.positions.len();
        self.tri(layer, p, c);
        if self.positions.len() == first + 3 {
            self.kerb[first..first + 3].copy_from_slice(&kerb);
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
pub(crate) fn tri_area(a: &[f64; 3], b: &[f64; 3], c: &[f64; 3]) -> f64 {
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
    pub(crate) shared_ends: [bool; 2],
    pub(crate) driveways: Option<Box<crate::driveway::Driveways>>,
    pub stations: Vec<Frame>,
    pub datums: Vec<f64>,
    pub keep: Vec<bool>,
    /// Kerb depression (0 full kerb .. 1 ramp lip) per station, [left, right].
    pub dep: [Vec<f64>; 2],
    /// (`seams`) the carriageway's drawn lateral interval per station, -half..half
    /// where it is drawn whole; empty where no station is clipped.
    pub lo: Vec<f64>,
    pub hi: Vec<f64>,
    /// (`seams`) the datums were moved to meet a busier run's surface.
    pub seamed: bool,
    /// (`seams`) a run clipped by a busier one at another level: the busier
    /// surface at each station's [lo, hi] clipped edge (NaN where not), a
    /// face drawn between the two along the seam; empty where none.
    pub face_lo: Vec<f64>,
    pub face_hi: Vec<f64>,
    /// (`seams`) each side's [left, right] drawn walk width per station
    /// (a busier run's walk alongside takes the rest; empty where whole),
    /// and its outer edge's height where it meets that walk (NaN where its own).
    pub walk_w: [Vec<f64>; 2],
    pub walk_y: [Vec<f64>; 2],
}

impl EntryGeom {
    /// The drawn lateral interval at station `i` of a run `half` wide.
    pub fn clip(&self, i: usize, half: f64) -> (f64, f64) {
        if self.lo.is_empty() { (-half, half) } else { (self.lo[i], self.hi[i]) }
    }

    /// The drawn lateral interval at arc length `s` (the narrower of its span's two stations).
    pub fn clip_at(&self, s: f64, half: f64) -> (f64, f64) {
        if self.lo.is_empty() || self.stations.len() < 2 {
            return (-half, half);
        }
        let k = self.stations.partition_point(|f| f.s <= s).clamp(1, self.stations.len() - 1);
        (self.lo[k - 1].max(self.lo[k]), self.hi[k - 1].min(self.hi[k]))
    }

    /// Side `k`'s walk width at station `i` of a run whose walk is `walk` there (`seams`).
    pub fn walk_at(&self, k: usize, i: usize, walk: f64) -> f64 {
        self.walk_w[k].get(i).map_or(walk, |&w| w.min(walk))
    }

    /// Side `k`'s walk was narrowed to a busier walk at station `i` (`seams`).
    pub fn walk_met(&self, k: usize, i: usize, walk: f64) -> bool {
        self.walk_w[k].get(i).is_some_and(|&w| w < walk - 1e-3)
    }

    /// Side `k` (0 the +normal side, 1 the other) of span `i` is clipped: the
    /// busier run's carriageway lies there, and no kerb, footway or bank is drawn.
    pub fn side_clipped(&self, i: usize, k: usize, half: f64) -> bool {
        if self.lo.is_empty() {
            return false;
        }
        let edge = |j: usize| if k == 0 { self.hi[j] < half - 1e-3 } else { self.lo[j] > -half + 1e-3 };
        edge(i) || edge(i + 1)
    }
}

impl EntryGeom {
    /// Clipping a neighbouring strip can insert vertices along the shared
    /// edge. At city-scale f32 coordinates those vertices may round to
    /// opposite sides of the edge. Overlap the two paved sections by one
    /// position unit on each side before converting their triangles to f32.
    fn paving_frame(&self, i: usize) -> Frame {
        let mut f = self.stations[i];
        let sign = if i == 0 && self.shared_ends[0] { -1.0 }
            else if i + 1 == self.stations.len() && self.shared_ends[1] { 1.0 }
            else { return f };
        let magnitude = f.x.abs().max(f.z.abs()) as f32;
        let shift = sign * f64::from(magnitude.next_up() - magnitude);
        f.x += shift * f.tx;
        f.z += shift * f.tz;
        f
    }
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
        let mut y = self.edge_ground(foot[0], foot[1]) - self.o.verge_undercut;
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
            let carpet = self.edge_ground(p[0], p[1]);
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
            let reach = self.o.graded_bank_reach(y - (self.edge_ground(p[0], p[1]) - self.o.verge_undercut)).max(step);
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
            let g = self.edge_ground(q[0], q[1]);
            (g - y > self.o.curb_face_height).then_some(g)
        };
        let (ra, rb) = (rise(a, ay, a_out), rise(b, by, b_out));
        if ra.is_some() || rb.is_some() {
            let (ta, tb) = (ra.unwrap_or(ay).max(ay), rb.unwrap_or(by).max(by));
            let inward = [-(a_out[0] + b_out[0]), -(a_out[1] + b_out[1])];
            // (`wall_coping`) a wall of some height stands a little proud of the
            // ground it holds, capped by a coping run back over that ground, and
            // closed behind down into it (not a bare face flush with the turf)
            let (a2, b2) = ([a[0] + a_out[0] * COPING_WIDTH_M, a[1] + a_out[1] * COPING_WIDTH_M], [b[0] + b_out[0] * COPING_WIDTH_M, b[1] + b_out[1] * COPING_WIDTH_M]);
            let cope = wall_coping()
                && (ta - ay).max(tb - by) > COPING_MIN_RISE_M
                && !self.buildings.contains(a2[0], a2[1])
                && !self.buildings.contains(b2[0], b2[1]);
            let (ca, cb) = if cope { (ta + COPING_UPSTAND_M, tb + COPING_UPSTAND_M) } else { (ta, tb) };
            m.quad(Layer::CurbFace, [p3(a, ay - RETAINING_TUCK_M), p3(b, by - RETAINING_TUCK_M), p3(b, cb), p3(a, ca)], [CURB_FACE(); 4], Some(inward));
            if cope {
                m.quad(Layer::CurbTop, [p3(a, ca), p3(b, cb), p3(b2, cb), p3(a2, ca)], [CURB_TOP(); 4], None);
                let back = |p: [f64; 2], top: f64| (self.edge_ground(p[0], p[1]) - self.o.verge_undercut).min(top) - RETAINING_TUCK_M;
                let out = [a_out[0] + b_out[0], a_out[1] + b_out[1]];
                m.quad(Layer::CurbFace, [p3(a2, ca), p3(b2, cb), p3(b2, back(b2, cb)), p3(a2, back(a2, ca))], [CURB_FACE(); 4], Some(out));
            }
            return;
        }
        // a bank whose run would reach a building (a walk standing over
        // the lower ground beside a building's base, the wedge in
        // sidewalk_walk frame 195) is closed the same way, a face on the walk's
        // outer vertices down under the ground, so no sloped verge face lies
        // against the facade.
        let into_building = |p: [f64; 2], y: f64, out: [f64; 2]| -> bool {
            let drop = y - (self.edge_ground(p[0], p[1]) - self.o.verge_undercut);
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
            la = Some(la.unwrap_or_else(|| self.edge_ground(a[0], a[1]) - self.o.verge_undercut));
            lb = Some(lb.unwrap_or_else(|| self.edge_ground(b[0], b[1]) - self.o.verge_undercut));
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
        // (`street_ground` on) a bank cut back to nothing at both ends by
        // another street's paving starting at the walk's edge is a face down
        // under that paving, not a sliver of verge lying over it.
        let span = |p: [f64; 2], f: [f64; 2]| (f[0] - p[0]).hypot(f[1] - p[1]);
        if crate::ground::street_ground_on() && span(a, fa) < BANK_SLIVER_M && span(b, fb) < BANK_SLIVER_M {
            let face = [a_out[0] + b_out[0], a_out[1] + b_out[1]];
            m.quad(Layer::CurbFace, [p3(a, ay), p3(b, by), p3(b, fby.min(by) - RETAINING_TUCK_M), p3(a, fay.min(ay) - RETAINING_TUCK_M)], [CURB_FACE(); 4], Some(face));
            return;
        }
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

    pub(crate) fn emit_span(&self, m: &mut TileMesh, ei: usize, i: usize, with_edges: bool) {
        let o = &self.o;
        let e: &Entry = &self.plan.entries[ei];
        let g = &self.geoms[ei];
        let (a, b) = (g.paving_frame(i), g.paving_frame(i + 1));
        let (da, db) = (g.datums[i], g.datums[i + 1]);
        let half = e.half;
        // (`--street-tapers`) the carriageway's drawn half at each station: `half`
        // without a taper; the kerb, gutter and crown follow it, the building line holds.
        let (mut ha, mut hb) = (self.half_at(ei, a.s), self.half_at(ei, b.s));
        // The JS palette's tint for this run's highway class (the per-block
        // tone the JS varies in its shader's macro field, not per run).
        let (asphalt, walk_tint) = self.run_tints(ei);
        // Carriageway and gutter pans.
        let gutter = GUTTER();
        let tapered = (ha - half).abs() > 1e-4 || (hb - half).abs() > 1e-4;
        if tapered && g.lo.is_empty() {
            let (offs_a, offs_b) = (g.section_offsets(o, ha), g.section_offsets(o, hb));
            let (offs_a, offs_b) = if offs_a.len() == offs_b.len() {
                (offs_a, offs_b)
            } else {
                ha = half;
                hb = half;
                (g.section_offsets(o, half), g.section_offsets(o, half))
            };
            let (gs_a, gs_b) = ((ha - o.gutter_width).max(0.0), (hb - o.gutter_width).max(0.0));
            let col = |u: f64, gs: f64| if u.abs() <= gs + 1e-9 { asphalt } else { gutter };
            let kd = |h: f64, u: f64| (h - u.abs()).max(0.0) as f32;
            for k in 0..offs_a.len() - 1 {
                let (u0a, u1a, u0b, u1b) = (offs_a[k], offs_a[k + 1], offs_b[k], offs_b[k + 1]);
                m.quad_kerb(
                    Layer::Carriageway,
                    [
                        p3(a.at(u0a), g.pavement_y(o, i, u0a, ha)),
                        p3(b.at(u0b), g.pavement_y(o, i + 1, u0b, hb)),
                        p3(b.at(u1b), g.pavement_y(o, i + 1, u1b, hb)),
                        p3(a.at(u1a), g.pavement_y(o, i, u1a, ha)),
                    ],
                    [col(u0a, gs_a), col(u0b, gs_b), col(u1b, gs_b), col(u1a, gs_a)],
                    [kd(ha, u0a), kd(hb, u0b), kd(hb, u1b), kd(ha, u1a)],
                );
            }
        } else {
            let offs = g.section_offsets(o, half);
            let (gs_a, gs_b) = ((ha - o.gutter_width).max(0.0), (hb - o.gutter_width).max(0.0));
            let col = |u: f64, gs: f64| if u.abs() <= gs + 1e-9 { asphalt } else { gutter };
            let kd = |h: f64, u: f64| (h - u.abs()).max(0.0) as f32;
            // (`seams`) the part of the section each station draws: a busier run's
            // carriageway alongside covers the rest. The band both stations draw
            // is laid in whole strips (each strip's offset the same at both ends:
            // the renderer takes the road's frame from the kerb distance's
            // gradient, and a strip clamped differently at its two ends turned
            // its patches askew triangle by triangle); the sliver between the two
            // stations' clips is fanned from the narrower one's edge.
            let ((cla, cha), (clb, chb)) = (g.clip(i, half), g.clip(i + 1, half));
            let (cla, cha) = (cla.clamp(-ha, ha), cha.clamp(-ha, ha));
            let (clb, chb) = (clb.clamp(-hb, hb), chb.clamp(-hb, hb));
            let (cl, ch) = (cla.max(clb), cha.min(chb));
            if ch - cl > 1e-4 {
                let mut us = vec![cl];
                us.extend(offs.iter().copied().filter(|&u| u > cl + 1e-4 && u < ch - 1e-4));
                us.push(ch);
                for w in us.windows(2) {
                    let (u0, u1) = (w[0], w[1]);
                    m.quad_kerb(
                        Layer::Carriageway,
                        [
                            p3(a.at(u0), g.pavement_y(o, i, u0, ha)),
                            p3(b.at(u0), g.pavement_y(o, i + 1, u0, hb)),
                            p3(b.at(u1), g.pavement_y(o, i + 1, u1, hb)),
                            p3(a.at(u1), g.pavement_y(o, i, u1, ha)),
                        ],
                        [col(u0, gs_a), col(u0, gs_b), col(u1, gs_b), col(u1, gs_a)],
                        [kd(ha, u0), kd(hb, u0), kd(hb, u1), kd(ha, u1)],
                    );
                }
            }
            if ch - cl <= 1e-4 {
                // no band both draw (a run going under a busier one): each strip clamped at both ends.
                for k in 0..offs.len() - 1 {
                    let (u0, u1) = (offs[k], offs[k + 1]);
                    let (a0, a1, b0, b1) = (u0.clamp(cla, cha), u1.clamp(cla, cha), u0.clamp(clb, chb), u1.clamp(clb, chb));
                    if a1 - a0 < 1e-4 && b1 - b0 < 1e-4 {
                        continue;
                    }
                    m.quad_kerb(
                        Layer::Carriageway,
                        [
                            p3(a.at(a0), g.pavement_y(o, i, a0, ha)),
                            p3(b.at(b0), g.pavement_y(o, i + 1, b0, hb)),
                            p3(b.at(b1), g.pavement_y(o, i + 1, b1, hb)),
                            p3(a.at(a1), g.pavement_y(o, i, a1, ha)),
                        ],
                        [col(a0, gs_a), col(b0, gs_b), col(b1, gs_b), col(a1, gs_a)],
                        [kd(ha, a0), kd(hb, b0), kd(hb, b1), kd(ha, a1)],
                    );
                }
            }
            // the slivers: (edge at a, edge at b, the band's edge)
            for (ea, eb, inner) in [(cla, clb, cl), (cha, chb, ch)] {
                if (ea - eb).abs() < 1e-4 || ch - cl <= 1e-4 {
                    continue;
                }
                let a_wide = (ea - inner).abs() > (eb - inner).abs();
                let (edge, iw, in_, hw, hn, gsw, gsn) = if a_wide {
                    (ea, i, i + 1, ha, hb, gs_a, gs_b)
                } else {
                    (eb, i + 1, i, hb, ha, gs_b, gs_a)
                };
                let (wide, narrow) = if a_wide { (&a, &b) } else { (&b, &a) };
                let (lo_u, hi_u) = (edge.min(inner), edge.max(inner));
                let mut us = vec![lo_u];
                us.extend(offs.iter().copied().filter(|&u| u > lo_u + 1e-4 && u < hi_u - 1e-4));
                us.push(hi_u);
                let apex = p3(narrow.at(inner), g.pavement_y(o, in_, inner, hn));
                for w in us.windows(2) {
                    let (u0, u1) = (w[0], w[1]);
                    let (p0, p1) = (p3(wide.at(u0), g.pavement_y(o, iw, u0, hw)), p3(wide.at(u1), g.pavement_y(o, iw, u1, hw)));
                    // the strip's winding: a at u0, b at u0, b at u1, a at u1
                    let (q, c, k) = if a_wide {
                        ([p0, apex, apex, p1], [col(u0, gsw), col(inner, gsn), col(inner, gsn), col(u1, gsw)], [kd(hw, u0), kd(hn, inner), kd(hn, inner), kd(hw, u1)])
                    } else {
                        ([apex, p0, p1, apex], [col(inner, gsn), col(u0, gsw), col(u1, gsw), col(inner, gsn)], [kd(hn, inner), kd(hw, u0), kd(hw, u1), kd(hn, inner)])
                    };
                    m.quad_kerb(Layer::Carriageway, q, c, k);
                }
            }
            // (`seams`) a busier run at another level along a clipped edge: a face between the two.
            if !g.face_lo.is_empty() {
                for (faces, side, ea, eb) in [(&g.face_hi, 1.0, cha, chb), (&g.face_lo, -1.0, cla, clb)] {
                    let (ya, yb) = (faces[i], faces[i + 1]);
                    if !(ya.is_finite() && yb.is_finite()) || (ea - side * ha).abs() < 1e-3 || (eb - side * hb).abs() < 1e-3 {
                        continue;
                    }
                    let (oa, ob) = (g.pavement_y(o, i, ea, ha), g.pavement_y(o, i + 1, eb, hb));
                    // facing the lower side
                    let up = (oa + ob) > (ya + yb);
                    let n = [side * (a.nx + b.nx) / 2.0, side * (a.nz + b.nz) / 2.0];
                    let face = if up { n } else { [-n[0], -n[1]] };
                    m.quad(Layer::CurbFace, [p3(a.at(ea), oa), p3(b.at(eb), ob), p3(b.at(eb), yb), p3(a.at(ea), ya)], [CURB_FACE(); 4], Some(face));
                }
            }
        }
        let first_footway = (m.layers.len(), m.positions.len());
        // Kerb, footway, frontage and bank per side.
        let mid_s = (a.s + b.s) / 2.0;
        // a stub raised on an approach embankment (`joins::StructureEnd::rise`):
        // a side with no footway gets its bank from the carriageway's edge.
        let raised = self.o.structures.is_some() && (self.datum_raise(ei, i) > RAISED_M || self.datum_raise(ei, i + 1) > RAISED_M);
        let retained = raised && self.surveyed_datum(e, [a.x, a.z], da).is_some();
        for (k, side) in [(0usize, 1.0f64), (1, -1.0)] {
            let walk = e.walks[k];
            if g.side_clipped(i, k, half) {
                continue;
            }
            if !(walk >= o.min_sidewalk_width) {
                if raised && with_edges {
                    let (out_a, out_b) = ([side * a.nx, side * a.nz], [side * b.nx, side * b.nz]);
                    // Retain the narrow approach at its road edge. A sloping
                    // earth bank here can project across the adjacent ramp.
                    let (left, right) = (a.at(side * ha), b.at(side * hb));
                    let (ya, yb) = (da - o.gutter_depth, db - o.gutter_depth);
                    if retained {
                        self.ring_edge(m, left, ya, out_a, right, yb, out_b, true);
                    } else {
                        self.bank(m, Some(ei), left, ya, out_a, right, yb, out_b);
                    }
                }
                continue;
            }
            if e.walk_yield_spans[k].iter().any(|sp| mid_s >= sp[0] && mid_s <= sp[1]) {
                continue;
            }
            let (pa, pb) = (g.dep[k][i], g.dep[k][i + 1]);
            let (ia, ib) = (da - o.gutter_depth, db - o.gutter_depth);
            let section = |s, datum, dep| {
                let driveway = dep_at(&e.driveway_cuts[k], s);
                let full = o.curb_top_y(datum);
                let lip = datum - o.gutter_depth + o.driveway.lip;
                let height = self.kerb_y(datum, dep).min(full + (lip - full) * driveway);
                let apron = o.curb_top_width + driveway * (o.driveway.depth.min(walk) - o.curb_top_width);
                (height, self.ramp_reach(dep).max(apron), dep.max(driveway))
            };
            let (ka, ra, pa) = section(a.s, da, pa);
            let (kb, rb, pb) = section(b.s, db, pb);
            let face = [side * a.nx * -1.0, side * a.nz * -1.0];
            // each vertex's height above the gutter flow line (the renderer's kerb-face grime).
            m.quad_kerb_face(
                Layer::CurbFace,
                [p3(a.at(side * ha), ia), p3(b.at(side * hb), ib), p3(b.at(side * hb), kb), p3(a.at(side * ha), ka)],
                [CURB_FACE(); 4],
                Some(face),
                [0.0, 0.0, (kb - ib).max(0.0) as f32, (ka - ia).max(0.0) as f32],
            );
            let (wya, wyb) = (o.sidewalk_surface_y(da, ha + ra, ha), o.sidewalk_surface_y(db, hb + rb, hb));
            let (ramp_c, top_c) = (RAMP(), CURB_TOP());
            let top_col = |d: f64| if d > 0.0 { ramp_c } else { top_c };
            m.quad(
                if pa > 0.0 || pb > 0.0 { Layer::Ramp } else { Layer::CurbTop },
                [p3(a.at(side * ha), ka), p3(b.at(side * hb), kb), p3(b.at(side * (hb + rb)), wyb), p3(a.at(side * (ha + ra)), wya)],
                [top_col(pa), top_col(pb), top_col(pb), top_col(pa)],
                None,
            );
            // (`seams`) a walk narrowed to a busier one alongside meets it at its edge;
            // (`--street-tapers`) the walk widens over a taper, its outer edge held at `half + walk`.
            let (wwa, wwb) = (g.walk_at(k, i, walk), g.walk_at(k, i + 1, walk));
            let met = g.walk_met(k, i, walk) || g.walk_met(k, i + 1, walk);
            let outer_y = |j: usize, d: f64, w: f64, h: f64| g.walk_y[k].get(j).copied().filter(|y| y.is_finite()).unwrap_or_else(|| o.sidewalk_surface_y(d, half + w, h));
            let (eya, eyb) = (outer_y(i, da, wwa, ha), outer_y(i + 1, db, wwb, hb));
            let (wa, wb) = (half + wwa - ha, half + wwb - hb);
            if wa > ra + 0.02 || wb > rb + 0.02 {
                let wc = walk_tint;
                let (wa2, wb2) = (wa.max(ra), wb.max(rb));
                // each vertex's distance across the walk from the kerb line.
                m.quad_kerb(
                    Layer::Sidewalk,
                    [p3(a.at(side * (ha + ra)), wya), p3(b.at(side * (hb + rb)), wyb), p3(b.at(side * (hb + wb2)), eyb), p3(a.at(side * (ha + wa2)), eya)],
                    [wc; 4],
                    [ra as f32, rb as f32, wb2 as f32, wa2 as f32],
                );
            }
            // a walk meeting a busier one at its level: no frontage, no bank (the other walk goes on).
            let level = met && [i, i + 1].iter().all(|&j| g.walk_y[k].get(j).is_some_and(|y| y.is_finite()) || !g.walk_met(k, j, walk));
            if level {
                continue;
            }
            // Frontage: level with the walk edge, FRONTAGE_LIFT_M proud.
            let (fa, fb) = if met { (0.0, 0.0) } else { (self.frontage_depth(ei, &a, side, walk), self.frontage_depth(ei, &b, side, walk)) };
            let step = o.frontage_step;
            let (mut ox_a, mut ox_b, mut oy_a, mut oy_b) = (ha + wa.max(ra), hb + wb.max(rb), eya, eyb);
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
            if !with_edges { continue; }
            // Verge bank from the outer edge down (or up) to the carpet.
            let out_a = [side * a.nx, side * a.nz];
            let out_b = [side * b.nx, side * b.nz];
            if retained || self.by_leg_junction(e, mid_s) {
                // (by a junction with a structure's leg: a face, as its corners.)
                self.ring_edge(m, a.at(side * ox_a), oy_a, out_a, b.at(side * ox_b), oy_b, out_b, true);
            } else {
                self.bank(m, Some(ei), a.at(side * ox_a), oy_a, out_a, b.at(side * ox_b), oy_b, out_b);
            }
        }
        // Up to the fitted seam the service drive owns the floor. Retaining
        // walls and the old sidewalk slab end at the sides of the entrance.
        if let Some(openings) = self.driveway_openings.get(&ei) {
            m.clip_faces(first_footway.0, openings, |_| true);
        }
        self.trim_arterial_footway(m, ei, first_footway.0, first_footway.1);
        // Lane paint along this span.
        if with_edges {
            self.emit_span_paint(m, ei, i);
            // (`--bikeways`) the kerbside bike lane as the city's network builds it.
            if self.bikeway(ei).is_some() {
                self.emit_bikeway(m, ei, i);
            }
            // (`--midblock-crossings`) a crossing between junctions whose centre lies in this span.
            if self.midblock.get(ei).is_some_and(|v| !v.is_empty()) {
                self.emit_midblock(m, ei, i);
            }
        }
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
        // An angled deck and a square-ended street leave a wedge outside
        // the bend. Its sides are the actual end frames of the two ribbons.
        if let Some(q) = se.line.get(1) {
            let (dx, dz) = (q[0] - se.at[0], q[1] - se.at[1]);
            let len = dx.hypot(dz);
            let direction = if end == 0 { 1.0 } else { -1.0 };
            let turn = (f0.tx * dz - f0.tz * dx) * direction;
            if len > 0.0 && turn.abs() > len * 1e-6 {
                let a = f0.at(-turn.signum() * direction * e.half);
                let b = [se.at[0] - turn.signum() * dz * se.half / len,
                         se.at[1] + turn.signum() * dx * se.half / len];
                let datum = g.datums[i];
                let color = self.run_tints(ei).0;
                m.tri(Layer::Carriageway, [
                    p3([f0.x, f0.z], o.cross_section_y(datum, 0.0, e.half)),
                    p3(a, o.cross_section_y(datum, e.half, e.half)),
                    p3(b, o.cross_section_y(datum, se.half, se.half)),
                ], [color; 3]);
            }
        }
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
            let drop = outer.1 - (self.edge_ground(edge[0], edge[1]) - o.verge_undercut);
            let reach = o.graded_bank_reach(drop);
            let foot = f.at(side * (outer.0 + reach));
            if !raised || self.surveyed_datum(e, [f.x, f.z], d).is_none() {
                prof.push((outer.0 + reach, self.bank_foot_y(foot, outer.1, Some(ei))));
            }
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
                let base = |p: [f64; 2], top: f64| (self.edge_ground(p[0], p[1]) - o.verge_undercut).min(top) - RETAINING_TUCK_M;
                m.quad(Layer::CurbFace, [p3(p0, y0), p3(p1, y1), p3(p1, base(p1, y1)), p3(p0, base(p0, y0))], [CURB_FACE(); 4], Some(out));
            }
        }
    }

    /// The bike lane's green: the palette's, or with `--bikeways` the green
    /// colored pavement of the standards (crate::bikeway).
    fn bike_green(&self) -> [u8; 3] {
        match &self.bike_dims {
            Some(d) if self.o.bikeways => crate::bikeway::albedo_bytes(d.green),
            _ => GREEN(),
        }
    }

    /// (`--bikeways`) A run's kerbside bike lane in span `i` (crate::bikeway):
    /// a two-way bikeway's broken yellow centre line, the buffer's diagonal
    /// markings, its tubular markers or its raised island, and the bicycle
    /// symbol with its arrow downstream after each intersection, per
    /// direction the lane carries. Each piece is drawn by the span its start
    /// lies in.
    fn emit_bikeway(&self, m: &mut TileMesh, ei: usize, i: usize) {
        let (Some(st), Some(d)) = (self.bikeway(ei).copied(), self.bike_dims) else { return };
        let o = &self.o;
        let e = &self.plan.entries[ei];
        let g = &self.geoms[ei];
        let (a, b) = (g.stations[i], g.stations[i + 1]);
        let (da, db) = (g.datums[i], g.datums[i + 1]);
        let span = (b.s - a.s).max(1e-9);
        let (m0, m1) = (e.marking_trim_start.max(e.s0()), (e.length - e.marking_trim_end).min(e.s1()));
        let (lo, hi) = (a.s.max(m0), b.s.min(m1));
        if !(hi - lo > 0.05) {
            return;
        }
        // a point on the drawn surface (its markings scaled with a taper as the paint is)
        let pt = |s: f64, u: f64, lift: f64| -> [f64; 3] {
            let h = self.half_at(ei, s);
            let u = if h == e.half { u } else { u * h / e.half.max(1e-6) };
            let t = (s - a.s) / span;
            let (pa, pb) = (a.at(u), b.at(u));
            let dat = da + (db - da) * t;
            [pa[0] + (pb[0] - pa[0]) * t, o.cross_section_y(dat, u, h) + lift, pa[1] + (pb[1] - pa[1]) * t]
        };
        let lines = &self.lines[ei];
        let white = WHITE();
        for lane in lines.iter().filter(|l| l.role == "cycle-lane") {
            let (uc, w) = (lane.u, lane.width);
            let buffer = lines.iter().filter(|l| l.role == "cycle-buffer").min_by(|x, y| (x.u - uc).abs().total_cmp(&(y.u - uc).abs())).map(|l| (l.u, l.width));
            // a. a two-way bikeway's broken yellow centre line (Section 9E.13), phased from the run's start
            if st.two_way {
                let period = d.centre_segment + d.centre_gap;
                let mut k = ((lo - m0) / period).floor();
                loop {
                    let s0 = m0 + k * period;
                    if s0 >= hi {
                        break;
                    }
                    let (p0, p1) = (s0.max(lo), (s0 + d.centre_segment).min(hi));
                    for [p0, p1] in self.cut_midblock(ei, p0, p1) {
                        if p1 > p0 + 0.02 {
                            let hw = d.line_width / 2.0;
                            m.quad_kerb(Layer::Marking, [pt(p0, uc - hw, o.marking_lift), pt(p1, uc - hw, o.marking_lift), pt(p1, uc + hw, o.marking_lift), pt(p0, uc + hw, o.marking_lift)], [YELLOW(); 4], [0.0, 0.0, 1.0, 1.0]);
                        }
                    }
                    k += 1.0;
                }
            }
            if buffer.is_none() && st.posts {
                // c'. no marked buffer: tubular markers along the lane's line on the traffic side
                let edge = lines.iter().filter(|l| l.role == "cycle-lane-edge").min_by(|x, y| x.u.abs().total_cmp(&y.u.abs())).map(|l| l.u);
                if let Some(ue) = edge {
                    let (mark, gap) = crate::paint::broken_line_pattern(&self.city.segments[e.road], &e.highway, o);
                    let step = ((mark + gap) / 2.0).max(1.0);
                    let mut k = ((lo - m0 - step / 2.0) / step).ceil().max(0.0);
                    loop {
                        let sp = m0 + step / 2.0 + k * step;
                        if sp >= hi || sp > m1 - step / 2.0 {
                            break;
                        }
                        if !self.midblock_gaps(ei).iter().any(|g| sp > g[0] && sp < g[1]) {
                            self.tubular_marker(m, pt(sp, ue, 0.0), d.post_width / 2.0, d.post_height);
                        }
                        k += 1.0;
                    }
                }
            }
            if let Some((ub, wb)) = buffer {
                let (u_in, u_out) = (ub - wb / 2.0, ub + wb / 2.0);
                if st.island {
                    // d. the raised island over the buffer, a kerb's height, short of the run's ends
                    // (at a plain joint the island runs on into the next run: no nose, no gap)
                    let (j0, j1) = (e.approach_start.is_some(), e.approach_end.is_some());
                    let (is0, is1) = (if j0 { m0 + ISLAND_END_M } else { m0 }, if j1 { m1 - ISLAND_END_M } else { m1 });
                    let gaps = self.midblock_gaps(ei);
                    for [s0, s1] in self.cut_midblock(ei, lo.max(is0), hi.min(is1)) {
                    // a nose where the island stops for a junction or a mid-block crossing
                    let (nose0, nose1) = ((j0 && s0 == is0) || gaps.iter().any(|g| g[1] == s0), (j1 && s1 == is1) || gaps.iter().any(|g| g[0] == s1));
                    if s1 > s0 + 0.02 {
                        let hgt = o.curb_face_height;
                        let top = |s: f64, u: f64| pt(s, u, hgt);
                        let base = |s: f64, u: f64| pt(s, u, 0.0);
                        m.quad(Layer::CurbTop, [top(s0, u_in), top(s1, u_in), top(s1, u_out), top(s0, u_out)], [CURB_TOP(); 4], None);
                        let nrm = [a.nx, a.nz];
                        for (u, out) in [(u_in, -1.0), (u_out, 1.0)] {
                            m.quad_kerb_face(Layer::CurbFace, [base(s0, u), base(s1, u), top(s1, u), top(s0, u)], [CURB_FACE(); 4], Some([nrm[0] * out, nrm[1] * out]), [0.0, 0.0, hgt as f32, hgt as f32]);
                        }
                        let tan = [a.tx, a.tz];
                        for (sx, out, here) in [(s0, -1.0, nose0), (s1, 1.0, nose1)] {
                            if here {
                                m.quad_kerb_face(Layer::CurbFace, [base(sx, u_in), base(sx, u_out), top(sx, u_out), top(sx, u_in)], [CURB_FACE(); 4], Some([tan[0] * out, tan[1] * out]), [0.0, 0.0, hgt as f32, hgt as f32]);
                            }
                        }
                    }
                    }
                } else if wb >= d.buffer_chevrons_from {
                    // b. diagonal buffer markings at 45 degrees (Section 9E.07 paras 15-16)
                    let lw = d.line_width * std::f64::consts::SQRT_2;
                    let mut k = ((lo - m0) / d.chevron_spacing).ceil();
                    loop {
                        let s0 = m0 + k * d.chevron_spacing;
                        if s0 >= hi {
                            break;
                        }
                        let gapped = self.midblock_gaps(ei).iter().any(|g| s0 + wb + lw > g[0] && s0 < g[1]);
                        if s0 + wb + lw <= m1 && !gapped {
                            m.quad_kerb(Layer::Marking, [pt(s0, u_in, o.marking_lift), pt(s0 + lw, u_in, o.marking_lift), pt(s0 + wb + lw, u_out, o.marking_lift), pt(s0 + wb, u_out, o.marking_lift)], [white; 4], [0.0, 0.0, 1.0, 1.0]);
                        }
                        k += 1.0;
                    }
                }
                if st.posts {
                    // c. tubular markers down the buffer's middle, N / 2 apart (Sections 3I.02, 3B.16)
                    let (mark, gap) = crate::paint::broken_line_pattern(&self.city.segments[e.road], &e.highway, o);
                    let step = ((mark + gap) / 2.0).max(1.0);
                    let mut k = ((lo - m0 - step / 2.0) / step).ceil().max(0.0);
                    loop {
                        let sp = m0 + step / 2.0 + k * step;
                        if sp >= hi || sp > m1 - step / 2.0 {
                            break;
                        }
                        if !self.midblock_gaps(ei).iter().any(|g| sp > g[0] && sp < g[1]) {
                            self.tubular_marker(m, pt(sp, ub, 0.0), d.post_width / 2.0, d.post_height);
                        }
                        k += 1.0;
                    }
                }
            }
            // e. the bicycle symbol, its arrow downstream, after each intersection per direction served
            let dirs: Vec<(f64, f64)> = if st.two_way {
                vec![(1.0, uc + w / 4.0), (-1.0, uc - w / 4.0)]
            } else if e.oneway {
                vec![(1.0, uc)]
            } else {
                vec![(if uc >= 0.0 { 1.0 } else { -1.0 }, uc)]
            };
            let need = 2.0 * d.symbol_gap + d.symbol_len + d.arrow_len;
            if m1 - m0 < need {
                continue;
            }
            for (dir, u0) in dirs {
                // after an intersection only (a plain joint between two runs is mid-block)
                if (dir > 0.0 && e.approach_start.is_none()) || (dir < 0.0 && e.approach_end.is_none()) {
                    continue;
                }
                let start = if dir > 0.0 { m0 + d.symbol_gap } else { m1 - d.symbol_gap };
                let arrow_at = start + dir * (d.symbol_len + d.symbol_gap);
                for (shape, at) in [(crate::bikeway::bicycle(d.symbol_len), start), (crate::bikeway::arrow(d.arrow_len), arrow_at)] {
                    if !(at >= lo && at < hi) || (dir < 0.0 && at == hi) {
                        continue;
                    }
                    for t in &shape {
                        let p = t.map(|q| pt(at + dir * q[0], u0 + dir * q[1], o.marking_lift * 1.5));
                        m.tri_kerb(Layer::Marking, p, [white; 3], [0.5; 3]);
                    }
                }
            }
        }
    }

    /// (`--midblock-crossings`) The crossings between junctions whose centre
    /// lies in span `i` of run `ei`: the continental crosswalk across the
    /// carriageway (`crossing_stripes`, as a junction's), and for a signal or
    /// a beacon the limit line of each direction's traffic arriving, set back
    /// from the crosswalk by `stopBarClearance` (right-hand traffic: the +s
    /// direction keeps to the +u side of the run's centre line).
    fn emit_midblock(&self, m: &mut TileMesh, ei: usize, i: usize) {
        let o = &self.o;
        let e = &self.plan.entries[ei];
        let g = &self.geoms[ei];
        let (a, b) = (g.stations[i].s, g.stations[i + 1].s);
        let at = |s: f64, u: f64, lift: f64| -> [f64; 3] {
            let h = self.half_at(ei, s);
            let f = e.frame(s, false);
            let datum = self.datum_at_station(g, s).unwrap_or_else(|| self.ground(f.x, f.z) + o.road_lift);
            let xz = f.at(u);
            [xz[0], o.cross_section_y(datum, u, h) + lift, xz[1]]
        };
        for c in self.midblock[ei].iter().filter(|c| c.s >= a && c.s < b) {
            let h = self.half_at(ei, c.s);
            let (b0, b1) = (c.s - c.depth / 2.0, c.s + c.depth / 2.0);
            let lift = o.junction_paint_lift;
            for v in &crate::paint::crossing_stripes(h, o).stripes {
                for [d0, d1] in [[b0, c.s], [c.s, b1]] {
                    m.quad_kerb(Layer::Marking, [at(d0, v[0], lift), at(d1, v[0], lift), at(d1, v[1], lift), at(d0, v[1], lift)], [CROSSWALK(); 4], [0.0, 0.0, 1.0, 1.0]);
                }
            }
            if c.kind > 2 {
                continue;
            }
            let boundary = self.lines[ei].iter().find(|l| l.role == "centre" || l.role == "unpainted-centre" || l.role == "centre-turn-boundary").map(|l| l.u).unwrap_or(0.0);
            let inset = o.stop_bar_edge_inset;
            let mut bars = vec![];
            if e.oneway {
                bars.push((b0 - o.stop_bar_clearance, -1.0, -h + inset, h - inset));
            } else {
                bars.push((b0 - o.stop_bar_clearance, -1.0, boundary.max(-h + inset), h - inset));
                bars.push((b1 + o.stop_bar_clearance, 1.0, -h + inset, boundary.min(h - inset)));
            }
            for (d_near, dir, lo, hi) in bars {
                if hi - lo < 0.5 {
                    continue;
                }
                let d_far = d_near + dir * o.stop_bar_depth;
                m.quad_kerb(Layer::Marking, [at(d_near, lo, lift), at(d_far, lo, lift), at(d_far, hi, lift), at(d_near, hi, lift)], [WHITE(); 4], [0.0, 1.0, 1.0, 0.0]);
            }
        }
    }

    /// (`--bikeways`) A tubular marker: an octagonal white post of radius
    /// `r`, `h` tall, standing at `base`.
    fn tubular_marker(&self, m: &mut TileMesh, base: [f64; 3], r: f64, h: f64) {
        let n = 8;
        let c = WHITE();
        let p = |k: usize, y: f64| {
            let t = k as f64 / n as f64 * std::f64::consts::TAU;
            [base[0] + r * t.cos(), base[1] + y, base[2] + r * t.sin()]
        };
        for k in 0..n {
            let t = (k as f64 + 0.5) / n as f64 * std::f64::consts::TAU;
            m.quad_kerb_face(Layer::Marking, [p(k, 0.0), p(k + 1, 0.0), p(k + 1, h), p(k, h)], [c; 4], Some([t.cos(), t.sin()]), [0.5; 4]);
            m.tri_kerb(Layer::Marking, [[base[0], base[1] + h, base[2]], p(k, h), p(k + 1, h)], [c; 3], [0.5; 3]);
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
        let (a, b) = (g.paving_frame(i), g.paving_frame(i + 1));
        let lo = a.s.max(e.marking_trim_start).max(e.s0());
        let hi = b.s.min(e.length - e.marking_trim_end).min(e.s1());
        if !(hi - lo > 0.05) {
            return;
        }
        let (da, db) = (g.datums[i], g.datums[i + 1]);
        let span = (b.s - a.s).max(1e-9);
        let at = |s: f64, u: f64| -> [f64; 3] {
            let t = ((s - a.s) / span).clamp(0.0, 1.0);
            // (`--street-tapers`) the markings eased in with the kerb: every line at its
            // share of the drawn half, so none is cut and the layout narrows as one
            let h = self.half_at(ei, s);
            let u = if h == e.half { u } else { u * h / e.half.max(1e-6) };
            let (pa, pb) = (a.at(u), b.at(u));
            let xz = [pa[0] + (pb[0] - pa[0]) * t, pa[1] + (pb[1] - pa[1]) * t];
            let d = da + (db - da) * t;
            [xz[0], o.cross_section_y(d, u, h) + o.marking_lift, xz[1]]
        };
        let (mark, gap) = crate::paint::broken_line_pattern(&self.city.segments[e.road], &e.highway, o);
        for l in lines {
            let c = match l.paint {
                crate::paint::Paint::White => WHITE(),
                crate::paint::Paint::Yellow => YELLOW(),
                crate::paint::Paint::Green => self.bike_green(),
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
            // (`seams`) a line on the part of the section a busier run covers is not drawn.
            let (cl, ch) = { let ((la, ha), (lb, hb)) = (g.clip(i, e.half), g.clip(i + 1, e.half)); (la.max(lb), ha.min(hb)) };
            if u0 < cl - 0.01 || u1 > ch + 0.01 {
                continue;
            }
            // (`--midblock-crossings`) no lane line across a mid-block crosswalk and its limit lines
            let pieces: Vec<[f64; 2]> = pieces.into_iter().flat_map(|p| self.cut_midblock(ei, p[0], p[1])).filter(|p| p[1] > p[0] + 0.02).collect();
            for [s0, s1] in pieces {
                if e.in_yield_span((s0 + s1) / 2.0) {
                    continue;
                }
                // each vertex's across-the-line coordinate, 0..1 (the renderer's worn-paint mask follows the line).
                if o.pavement_repairs {
                    marking::paint(m, g, o, e.half, [s0,s1], [u0,u1], marking::StripeWidth::AcrossRoad, c, o.marking_lift);
                } else {
                    m.quad_kerb(Layer::Marking, [at(s0, u0), at(s1, u0), at(s1, u1), at(s0, u1)], [c; 4], [0.0, 0.0, 1.0, 1.0]);
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // junctions
    // -----------------------------------------------------------------------

    /// The datum of approach `app`'s trimmed end frame (`junctionDatums`).
    fn approach_datum(&self, app: &Approach) -> (Frame, f64) {
        let f = crate::plan::approach_end_frame(&self.plan.entries, app);
        let terrain = self.ground(f.x, f.z) + self.o.road_lift;
        if let Some(d) = self.surveyed_datum(&self.plan.entries[app.entry], [f.x, f.z], terrain) {
            return (f, d);
        }
        // (`seams`) a run moved to meet a busier one's surface meets its pad there.
        if let Some(g) = self.geoms.get(app.entry).filter(|g| g.seamed) {
            if let Some(d) = self.datum_at_station(g, f.s) {
                return (f, d);
            }
        }
        (f, terrain)
    }

    fn junction_pad(&self, node: &Node, ends: &[(Frame, f64)]) -> ([f64; 3], Vec<[f64; 3]>, Vec<[usize; 3]>) {
        let o = &self.o;
        let n = node.approaches.len();
        let mut rim = Vec::new();
        for (i, app) in node.approaches.iter().enumerate() {
            let (f, d) = ends[i];
            let s = if app.at_start { 1.0 } else { -1.0 };
            for lat in o.section_offsets(app.half) {
                rim.push(p3(f.at(s * lat), o.cross_section_y(d, lat, app.half)));
            }
            // The corner path from this approach to the next (interior stations: the kerb line).
            let path = &node.paths[i];
            let datums = self.path_datums(&path.stations, ends[path.a].1, ends[path.b].1);
            let last = path.stations.len().saturating_sub(1);
            for (k, st) in path.stations.iter().enumerate() {
                if k == 0 || k == last {
                    continue;
                }
                rim.push(p3([st.x, st.z], datums[k] - o.gutter_depth));
            }
        }
        let mean_d = ends.iter().map(|e| e.1).sum::<f64>() / n as f64;
        let mean_half = node.approaches.iter().map(|a| a.half).sum::<f64>() / n as f64;
        let center = p3(node.position, mean_d + o.cross_slope * mean_half);
        let triangles = junction::surface(center, &mut rim, || {
            let mut mouths = Vec::new();
            for (app, (frame, _)) in node.approaches.iter().zip(ends) {
                mouths.push(frame.at(0.0));
                let source = &self.city.segments[self.plan.entries[app.entry].road];
                for lane in source.lane_layout.iter().flatten().filter(|lane| lane.motor) {
                    mouths.push(frame.at(-lane.lateral));
                }
            }
            mouths
        });
        (center, rim, triangles)
    }

    pub(crate) fn traffic_junction_pad(&self, node: &Node) -> ([f32; 3], Vec<[f32; 3]>, Vec<[usize; 3]>) {
        let ends: Vec<_> = node.approaches.iter().map(|a| self.approach_datum(a)).collect();
        let (center, rim, triangles) = self.junction_pad(node, &ends);
        // Match the near street mesh's stored vertex positions.
        let vertex = |p: [f64; 3]| p.map(|v| v as f32);
        (vertex(center), rim.into_iter().map(vertex).collect(), triangles)
    }

    pub(crate) fn emit_node(&self, m: &mut TileMesh, ni: usize, with_edges: bool) {
        let o = &self.o;
        let node: &Node = &self.plan.nodes[ni];
        let n = node.approaches.len();
        if n == 0 {
            return;
        }
        let ends: Vec<(Frame, f64)> = node.approaches.iter().map(|a| self.approach_datum(a)).collect();
        // (`seams`) a slip road merging alongside a through leg (`plan::Approach::merge_trim`)
        // is drawn by its own ribbon: no mouth in the pad, no corner either side of it.
        let merge: Vec<bool> = node.approaches.iter().map(|a| a.merge_trim.is_some() && crate::seams::road_seams()).collect();
        let pad = PAD();
        if junction_ground() {
            // --- the pad: one ring round the node, fanned from its centre. -------
            let ring = self.pad_ring(node, &ends, &merge);
            let mean_half = node.approaches.iter().map(|a| a.half).sum::<f64>() / n as f64;
            // the fan subdivided every JUNCTION_GROUND_STEP_M along each spoke,
            // each point on the ground (+ road lift) plus an offset blended from
            // the apex's (the ring's mean offset and the crown) to its ring
            // point's, so the mouths meet their ribbons exactly and a long pad
            // (an acute fork's, cut back tens of metres) follows its slope
            // instead of standing as one flat slab across it.
            let lift = o.road_lift;
            let off: Vec<f64> = ring.iter().map(|(p, y)| y - (self.pad_ground(p[0], p[1]) + lift)).collect();
            let apex_off = off.iter().sum::<f64>() / off.len().max(1) as f64 + o.cross_slope * mean_half;
            // the fan's apex: a point the whole ring is seen from (its kernel's
            // centroid); a ring seen whole from no point (concave round its
            // corners, an approach cut back far past the others: a fifth of the
            // city's pads) is ear-clipped instead, the fan from the node had
            // folded over itself, two pad surfaces one over the other.
            let a = match Self::ring_kernel(&ring) {
                Some(k) => k,
                None => {
                    if self.emit_pad_clipped(m, &ring, &off, lift, pad) {
                        return self.emit_node_rest(m, ni, &ends, &merge, with_edges);
                    }
                    node.position
                }
            };
            let reach = ring.iter().map(|(p, _)| ((p[0] - a[0]).powi(2) + (p[1] - a[1]).powi(2)).sqrt()).fold(0.0, f64::max);
            let levels = ((reach / JUNCTION_GROUND_STEP_M).ceil() as usize).max(1);
            let at = |k: usize, j: usize| -> [f64; 3] {
                let t = j as f64 / levels as f64;
                let r = ring[k].0;
                let x = [a[0] + (r[0] - a[0]) * t, a[1] + (r[1] - a[1]) * t];
                if j == levels {
                    return p3(r, ring[k].1);
                }
                p3(x, self.pad_ground(x[0], x[1]) + lift + apex_off + (off[k] - apex_off) * t)
            };
            let apex = p3(a, self.pad_ground(a[0], a[1]) + lift + apex_off);
            for k in 0..ring.len() {
                let q_k = (k + 1) % ring.len();
                let (p, q) = (ring[k].0, ring[q_k].0);
                if (p[0] - q[0]).abs() < 1e-9 && (p[1] - q[1]).abs() < 1e-9 {
                    continue;
                }
                m.tri(Layer::Carriageway, [apex, at(q_k, 1), at(k, 1)], [pad, pad, pad]);
                for j in 1..levels {
                    // (the fan's own winding: apex, q, p)
                    m.tri(Layer::Carriageway, [at(k, j), at(q_k, j), at(q_k, j + 1)], [pad, pad, pad]);
                    m.tri(Layer::Carriageway, [at(k, j), at(q_k, j + 1), at(k, j + 1)], [pad, pad, pad]);
                }
            }
        } else {
            let (apex, rim, triangles) = self.junction_pad(node, &ends);
            let vertex = |i| if i == 0 { apex } else { rim[i - 1] };
            for [a, b, c] in junction::faces(rim.len(), &triangles) {
                let [a, b, c] = [vertex(a), vertex(b), vertex(c)];
                if (b[0] - c[0]).abs() < 1e-9 && (b[2] - c[2]).abs() < 1e-9 {
                    continue;
                }
                m.tri(Layer::Carriageway, [a, c, b], [pad; 3]);
            }
        }
        self.emit_node_rest(m, ni, &ends, &merge, with_edges);
    }

    /// A node's corner returns and paint (after its pad).
    fn emit_node_rest(&self, m: &mut TileMesh, ni: usize, ends: &[(Frame, f64)], merge: &[bool], with_edges: bool) {
        let o = &self.o;
        let node: &Node = &self.plan.nodes[ni];
        let merge_path = |p: &crate::plan::CornerPath| merge.get(p.a).copied().unwrap_or(false) || merge.get(p.b).copied().unwrap_or(false);
        // --- corner returns: kerb, ramps, footway, bank. ----------------------
        let first_corner_face = m.layers.len();
        let first_corner_vertex = m.positions.len();
        let retained = node.approaches.iter().zip(ends).any(|(a, (f, datum))| {
            self.plan.entries[a.entry].structure_leg.is_some()
                || (datum - (self.ground(f.x, f.z) + o.road_lift)).abs() > RAISED_M
        });
        for path in node.paths.iter().filter(|p| !merge_path(p)) {
            let datums = self.path_datums(&path.stations, ends[path.a].1, ends[path.b].1);
            let st = &path.stations;
            let reach_full = o.curb_top_width + o.ramp_run;
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
                self.ring_span(m, ni, sa, sb, datums[k], datums[k + 1], deps[k], deps[k + 1], retained, with_edges);
            }
        }
        self.trim_arterial_corner(m, node, first_corner_face, first_corner_vertex);
        // --- junction paint: crosswalk bars and limit lines. ------------------
        // (none on a structure's leg: its mouth is the structure's own deck.)
        if with_edges && node.earns_paint {
            for (i, app) in node.approaches.iter().enumerate() {
                if self.plan.entries[app.entry].structure_leg.is_some() || merge[i] {
                    continue;
                }
                self.emit_approach_paint(m, node, app, ends[i].1);
            }
        }
    }

    /// A pad whose ring no fan covers once (`ring_kernel` None): the ring
    /// ear-clipped, each ear subdivided (the same count on every ear, so
    /// shared edges agree) so it follows the ground (+ road lift) with the
    /// ring's offsets from it blended across, its edges along the ring
    /// straight between their ring points (the mouths meet their ribbons
    /// exactly). False where the ring does not clip (not simple): the fan.
    fn emit_pad_clipped(&self, m: &mut TileMesh, ring: &[([f64; 2], f64)], off: &[f64], lift: f64, pad: [u8; 3]) -> bool {
        // (consecutive duplicate points dropped: a corner's first station on a mouth's corner)
        let mut keep: Vec<usize> = vec![];
        for k in 0..ring.len() {
            let p = ring[k].0;
            if keep.last().is_some_and(|&l| (ring[l].0[0] - p[0]).hypot(ring[l].0[1] - p[1]) < 1e-6) {
                continue;
            }
            keep.push(k);
        }
        while keep.len() > 1 && (ring[keep[0]].0[0] - ring[*keep.last().unwrap()].0[0]).hypot(ring[keep[0]].0[1] - ring[*keep.last().unwrap()].0[1]) < 1e-6 {
            keep.pop();
        }
        let n = keep.len();
        if n < 3 {
            return false;
        }
        let pts: Vec<[f64; 2]> = keep.iter().map(|&k| ring[k].0).collect();
        let area: f64 = (0..n).map(|k| { let (p, q) = (pts[k], pts[(k + 1) % n]); p[0] * q[1] - q[0] * p[1] }).sum();
        let ccw = area > 0.0;
        // ear clipping on the ring made counter-clockwise (indices into `pts`)
        let order: Vec<usize> = if ccw { (0..n).collect() } else { (0..n).rev().collect() };
        let cross = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
        let mut idx = order.clone();
        let mut ears: Vec<[usize; 3]> = vec![];
        let mut guard = 0;
        while idx.len() > 3 && guard < 4 * n * n {
            guard += 1;
            let len = idx.len();
            let mut clipped = false;
            for k in 0..len {
                let (ia, ib, ic) = (idx[(k + len - 1) % len], idx[k], idx[(k + 1) % len]);
                let (a, b, c) = (pts[ia], pts[ib], pts[ic]);
                if cross(a, b, c) <= 1e-9 {
                    continue;
                }
                let ear = idx.iter().all(|&q| q == ia || q == ib || q == ic || { let r = pts[q]; !(cross(a, b, r) >= -1e-9 && cross(b, c, r) >= -1e-9 && cross(c, a, r) >= -1e-9) });
                if ear {
                    ears.push([ia, ib, ic]);
                    idx.remove(k);
                    clipped = true;
                    break;
                }
            }
            if !clipped {
                return false;
            }
        }
        if idx.len() == 3 {
            ears.push([idx[0], idx[1], idx[2]]);
        }
        let mut longest = 0.0f64;
        for e in &ears {
            for q in 0..3 {
                let (a, b) = (pts[e[q]], pts[e[(q + 1) % 3]]);
                longest = longest.max((a[0] - b[0]).hypot(a[1] - b[1]));
            }
        }
        let div = ((longest / JUNCTION_GROUND_STEP_M).ceil() as usize).clamp(1, 12);
        let ring_edge = |i: usize, j: usize| (i + 1) % n == j || (j + 1) % n == i;
        let y_of = |k: usize| ring[keep[k]].1;
        let off_of = |k: usize| off[keep[k]];
        for e in &ears {
            // a point at barycentric (u, v, w) of the ear, (i, j) of `div`
            let at = |i: usize, j: usize| -> [f64; 3] {
                let (u, v) = (i as f64 / div as f64, j as f64 / div as f64);
                let w = 1.0 - u - v;
                let (pa, pb, pc) = (pts[e[0]], pts[e[1]], pts[e[2]]);
                let x = [pa[0] * w + pb[0] * u + pc[0] * v, pa[1] * w + pb[1] * u + pc[1] * v];
                // a corner: its ring height; along a ring edge: straight between its ends
                let bc = [(w, e[0]), (u, e[1]), (v, e[2])];
                if let Some(&(_, k)) = bc.iter().find(|(t, _)| (*t - 1.0).abs() < 1e-9) {
                    return p3(x, y_of(k));
                }
                for q in 0..3 {
                    let (t0, k0) = bc[q];
                    let (t1, k1) = bc[(q + 1) % 3];
                    let (t2, _) = bc[(q + 2) % 3];
                    if t2.abs() < 1e-9 && ring_edge(k0, k1) {
                        return p3(x, (y_of(k0) * t0 + y_of(k1) * t1) / (t0 + t1).max(1e-12));
                    }
                }
                let o = off_of(e[0]) * w + off_of(e[1]) * u + off_of(e[2]) * v;
                p3(x, self.pad_ground(x[0], x[1]) + lift + o)
            };
            for j in 0..div {
                for i in 0..div - j {
                    // (the fan's winding: opposite the ring's; the ears run counter-clockwise)
                    let t1 = [at(i, j), at(i + 1, j), at(i, j + 1)];
                    m.tri(Layer::Carriageway, if ccw { [t1[0], t1[2], t1[1]] } else { t1 }, [pad, pad, pad]);
                    if i + j + 1 < div {
                        let t2 = [at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)];
                        m.tri(Layer::Carriageway, if ccw { [t2[0], t2[2], t2[1]] } else { t2 }, [pad, pad, pad]);
                    }
                }
            }
        }
        true
    }

    /// A node's pad ring: each approach's mouth section (a merging slip's
    /// none), then the corner path to the next (its interior stations, the
    /// kerb line), with their heights.
    fn pad_ring(&self, node: &Node, ends: &[(Frame, f64)], merge: &[bool]) -> Vec<([f64; 2], f64)> {
        let o = &self.o;
        let mut ring: Vec<([f64; 2], f64)> = vec![];
        for (i, app) in node.approaches.iter().enumerate() {
            let (f, d) = ends[i];
            let s = if app.at_start { 1.0 } else { -1.0 };
            if !merge[i] {
                for lat in o.section_offsets(app.half) {
                    ring.push((f.at(s * lat), o.cross_section_y(d, lat, app.half)));
                }
            }
            let path = &node.paths[i];
            if merge.get(path.a).copied().unwrap_or(false) || merge.get(path.b).copied().unwrap_or(false) {
                continue;
            }
            let datums = self.path_datums(&path.stations, ends[path.a].1, ends[path.b].1);
            let last = path.stations.len().saturating_sub(1);
            for (k, st) in path.stations.iter().enumerate() {
                if k == 0 || k == last {
                    continue;
                }
                ring.push(([st.x, st.z], datums[k] - o.gutter_depth));
            }
        }
        ring
    }

    /// A point the whole ring can be seen from (the centroid of its kernel,
    /// the half-planes inside every edge), None where there is none (a ring
    /// not star-shaped: no fan covers it once).
    pub(crate) fn ring_kernel(ring: &[([f64; 2], f64)]) -> Option<[f64; 2]> {
        let n = ring.len();
        if n < 3 {
            return None;
        }
        let area: f64 = (0..n).map(|k| { let (p, q) = (ring[k].0, ring[(k + 1) % n].0); p[0] * q[1] - q[0] * p[1] }).sum();
        let ccw = area > 0.0;
        let (mut x0, mut x1, mut z0, mut z1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for (p, _) in ring {
            x0 = x0.min(p[0]);
            x1 = x1.max(p[0]);
            z0 = z0.min(p[1]);
            z1 = z1.max(p[1]);
        }
        let mut poly = vec![[x0 - 1.0, z0 - 1.0], [x1 + 1.0, z0 - 1.0], [x1 + 1.0, z1 + 1.0], [x0 - 1.0, z1 + 1.0]];
        if !ccw {
            poly.reverse();
        }
        for k in 0..n {
            let (p, q) = (ring[k].0, ring[(k + 1) % n].0);
            let (ex, ez) = (q[0] - p[0], q[1] - p[1]);
            if ex.abs() < 1e-9 && ez.abs() < 1e-9 {
                continue;
            }
            // inside: left of p->q on a counter-clockwise ring, right on a clockwise one (shrunk 1 cm)
            let side = |r: [f64; 2]| { let c = ex * (r[1] - p[1]) - ez * (r[0] - p[0]); (if ccw { c } else { -c }) / ex.hypot(ez) - 0.01 };
            let mut out = vec![];
            for i in 0..poly.len() {
                let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
                let (sa, sb) = (side(a), side(b));
                if sa >= 0.0 {
                    out.push(a);
                }
                if (sa >= 0.0) != (sb >= 0.0) {
                    let t = sa / (sa - sb);
                    out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
                }
            }
            poly = out;
            if poly.len() < 3 {
                return None;
            }
        }
        let a2: f64 = (0..poly.len()).map(|k| { let (p, q) = (poly[k], poly[(k + 1) % poly.len()]); p[0] * q[1] - q[0] * p[1] }).sum();
        if a2.abs() < 1e-4 {
            return None;
        }
        let (mut cx, mut cz) = (0.0, 0.0);
        for k in 0..poly.len() {
            let (p, q) = (poly[k], poly[(k + 1) % poly.len()]);
            let c = p[0] * q[1] - q[0] * p[1];
            cx += (p[0] + q[0]) * c;
            cz += (p[1] + q[1]) * c;
        }
        Some([cx / (3.0 * a2), cz / (3.0 * a2)])
    }

    /// (diagnostic) a node's pad fan: its triangles folded back over the
    /// others (the ring not star-shaped from its apex), and all of them.
    pub fn pad_folds(&self, ni: usize) -> (usize, usize) {
        let node: &Node = &self.plan.nodes[ni];
        if node.approaches.is_empty() {
            return (0, 0);
        }
        let ends: Vec<(Frame, f64)> = node.approaches.iter().map(|a| self.approach_datum(a)).collect();
        let merge: Vec<bool> = node.approaches.iter().map(|a| a.merge_trim.is_some() && crate::seams::road_seams()).collect();
        let ring = self.pad_ring(node, &ends, &merge);
        let a = node.position;
        let cross: Vec<f64> = (0..ring.len()).map(|k| { let (p, q) = (ring[k].0, ring[(k + 1) % ring.len()].0); (p[0] - a[0]) * (q[1] - a[1]) - (p[1] - a[1]) * (q[0] - a[0]) }).filter(|c| c.abs() > 1e-6).collect();
        let pos = cross.iter().filter(|&&c| c > 0.0).count();
        let neg = cross.len() - pos;
        let folded = pos.min(neg);
        // (with `junction_ground`: none where the ring has a kernel (the fan
        // from its centroid) or ear-clips; the fan from the node else)
        if folded > 0 && junction_ground() {
            let lift = self.o.road_lift;
            let off: Vec<f64> = ring.iter().map(|(p, y)| y - (self.pad_ground(p[0], p[1]) + lift)).collect();
            if Self::ring_kernel(&ring).is_some() || self.emit_pad_clipped(&mut TileMesh::default(), &ring, &off, lift, PAD()) {
                return (0, cross.len());
            }
        }
        (folded, cross.len())
    }

    /// One datum per corner station, blended by chord length from A's to B's
    /// (with `junction_ground`, the ground at the station plus the road lift
    /// and that blend's offset from it, so a long corner follows its slope).
    fn path_datums(&self, st: &[CurbStation], da: f64, db: f64) -> Vec<f64> {
        let mut cum = vec![0.0];
        for k in 1..st.len() {
            cum.push(cum[k - 1] + ((st[k].x - st[k - 1].x).powi(2) + (st[k].z - st[k - 1].z).powi(2)).sqrt());
        }
        let total = cum.last().copied().unwrap_or(0.0).max(1e-9);
        if junction_ground() && st.len() >= 2 {
            let g = |k: usize| self.pad_ground(st[k].x, st[k].z) + self.o.road_lift;
            let (oa, ob) = (da - g(0), db - g(st.len() - 1));
            return cum.iter().enumerate().map(|(k, c)| g(k) + oa + (ob - oa) * c / total).collect();
        }
        cum.iter().map(|c| da + (db - da) * c / total).collect()
    }

    /// The ground a junction's pad and corner returns follow (`junction_ground`):
    /// the survey filtered over `PAD_SMOOTH_M` (a 3 x 3 binomial), so the pad
    /// carries no crease of the survey's bilinear cells nor a valley's V, but a
    /// sag or crest drawn round as a road's vertical curve is.
    fn pad_ground(&self, x: f64, z: f64) -> f64 {
        let r = PAD_SMOOTH_M;
        let mut sum = 0.0;
        for (dz, wz) in [(-r, 1.0), (0.0, 2.0), (r, 1.0)] {
            for (dx, wx) in [(-r, 1.0), (0.0, 2.0), (r, 1.0)] {
                sum += wx * wz * self.ground(x + dx, z + dz);
            }
        }
        sum / 16.0
    }

    #[allow(clippy::too_many_arguments)]
    fn ring_span(&self, m: &mut TileMesh, ni: usize, sa: &CurbStation, sb: &CurbStation, da: f64, db: f64, pa: f64, pb: f64, leg: bool, with_edges: bool) {
        let o = &self.o;
        let off = |s: &CurbStation, d: f64| [s.x + s.out[0] * d * s.scale, s.z + s.out[1] * d * s.scale];
        // a corner return lying on a neighbouring junction's pad (a cluster of
        // junctions: a traffic circle, a divided road's crossing) is not drawn:
        // its kerb and walk would stand over or under that pad at their own datum.
        let mid = [(sa.x + sb.x) / 2.0, (sa.z + sb.z) / 2.0];
        if self.pads.on_other_pad(mid[0], mid[1], ni) {
            return;
        }
        let walk_on_pad = {
            let (wm, om) = ((sa.walk + sb.walk) / 2.0, [(sa.out[0] + sb.out[0]) / 2.0, (sa.out[1] + sb.out[1]) / 2.0]);
            let wp = [mid[0] + om[0] * wm * 0.6, mid[1] + om[1] * wm * 0.6];
            wm > 0.0 && (self.pads.on_other_pad(wp[0], wp[1], ni) || self.pads.deep_in_own_pad(wp[0], wp[1], ni, CORNER_IN_PAD_M))
        };
        let (ia, ib) = (da - o.gutter_depth, db - o.gutter_depth);
        let (ka, kb) = (self.kerb_y(da, pa), self.kerb_y(db, pb));
        let face = [-(sa.out[0] + sb.out[0]), -(sa.out[1] + sb.out[1])];
        // each vertex's height above the gutter flow line (the renderer's kerb-face grime).
        m.quad_kerb_face(Layer::CurbFace, [p3([sa.x, sa.z], ia), p3([sb.x, sb.z], ib), p3([sb.x, sb.z], kb), p3([sa.x, sa.z], ka)], [CURB_FACE(); 4], Some(face), [0.0, 0.0, (kb - ib).max(0.0) as f32, (ka - ia).max(0.0) as f32]);
        let (wa, wb) = if walk_on_pad { (0.0, 0.0) } else { (sa.walk, sb.walk) };
        if !(wa > 0.0 || wb > 0.0) {
            // A side with no footway: the bank starts at the kerb top.
            if with_edges { self.ring_edge(m, [sa.x, sa.z], ka, sa.out, [sb.x, sb.z], kb, sb.out, leg); }
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
        if with_edges { self.ring_edge(m, off(sa, wa2), walk_y(da, wa2), sa.out, off(sb, wb2), walk_y(db, wb2), sb.out, leg); }
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
        let foot = |p: [f64; 2], y: f64| (self.edge_ground(p[0], p[1]) - self.o.verge_undercut).min(y) - RETAINING_TUCK_M;
        let face = [a_out[0] + b_out[0], a_out[1] + b_out[1]];
        m.quad(Layer::CurbFace, [p3(a, ay), p3(b, by), p3(b, foot(b, by)), p3(a, foot(a, ay))], [CURB_FACE(); 4], Some(face));
    }

    /// Distances from the junction along this approach to the edges of its
    /// drawn crosswalk. Short arms can have a surveyed crossing and too
    /// little road between their two junctions to paint the band.
    pub fn painted_crossing_band(&self, node: &Node, app: &Approach) -> Option<[f64; 2]> {
        let e = &self.plan.entries[app.entry];
        let available = app.run_length - if app.at_start { e.trim_end } else { e.trim_start };
        let start = app.trim + self.o.crosswalk_clearance;
        let end = start + node.band_depth;
        (node.earns_paint && app.crossing_painted && end + 0.4 <= available).then_some([start, end])
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
        if let Some([band0, band1]) = self.painted_crossing_band(node, app) {
            let stripes = crate::paint::crossing_stripes(app.half, o);
            for v in &stripes.stripes {
                if o.pavement_repairs {
                    let station = |d| if app.at_start { d } else { e.length - d };
                    marking::paint(m, g, o, e.half, [station(band0), station(band1)], [sgn*v[0], sgn*v[1]],
                        marking::StripeWidth::AcrossRoad, CROSSWALK(), lift);
                    continue;
                }
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
                if o.pavement_repairs {
                    let station = |d| if app.at_start { d } else { e.length - d };
                    marking::paint(m, g, o, e.half, [station(d0), station(d1)], [sgn*lo, sgn*hi],
                        marking::StripeWidth::AlongRoad, WHITE(), lift);
                } else {
                    m.quad_kerb(Layer::Marking, [at(d0, lo), at(d1, lo), at(d1, hi), at(d0, hi)], [WHITE(); 4], [0.0, 1.0, 1.0, 0.0]);
                }
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
        if g.crosses_apron(i,j) {
            for span in i..j { self.emit_span(m, ei, span, false); }
            return;
        }
        let (a, b) = (g.paving_frame(i), g.paving_frame(j));
        let (da, db) = (g.datums[i], g.datums[j]);
        let h = e.half;
        // (`--street-tapers`) the drawn half at each end; the walk's outer edge held
        // (`seams`) clamped to the drawn part of the section at each end.
        let (ha, hb) = (self.half_at(ei, a.s), self.half_at(ei, b.s));
        let ((cla, cha), (clb, chb)) = (g.clip(i, h), g.clip(j, h));
        let (cla, cha) = (cla.clamp(-ha, ha), cha.clamp(-ha, ha));
        let (clb, chb) = (clb.clamp(-hb, hb), chb.clamp(-hb, hb));
        if cha - cla > 1e-4 || chb - clb > 1e-4 {
            m.quad(
                Layer::Carriageway,
                [p3(a.at(cla), da), p3(b.at(clb), db), p3(b.at(chb), db), p3(a.at(cha), da)],
                [self.run_tints(ei).0; 4],
                None,
            );
        }
        let first_footway = (m.layers.len(), m.positions.len());
        for (k, side) in [(0usize, 1.0f64), (1, -1.0)] {
            let w = e.walks[k];
            if !(w >= o.min_sidewalk_width) || g.side_clipped(i, k, h) || g.side_clipped(j.saturating_sub(1), k, h) {
                continue;
            }
            let (ya, yb) = (o.sidewalk_surface_y(da, h + w, ha), o.sidewalk_surface_y(db, h + w, hb));
            m.quad(
                Layer::Sidewalk,
                [p3(a.at(side * ha), o.curb_top_y(da)), p3(b.at(side * hb), o.curb_top_y(db)), p3(b.at(side * (h + w)), yb), p3(a.at(side * (h + w)), ya)],
                [self.run_tints(ei).1; 4],
                None,
            );
        }
        self.trim_arterial_footway(m, ei, first_footway.0, first_footway.1);
    }

    /// The coarse level of a run's spans `i..j` (every 4th station): the
    /// carriageway and its footways as one flat strip.
    pub(crate) fn emit_coarse_span(&self, m: &mut TileMesh, ei: usize, i: usize, j: usize) {
        let e = &self.plan.entries[ei];
        let g = &self.geoms[ei];
        if g.crosses_apron(i,j) {
            for span in i..j { self.emit_span(m, ei, span, false); }
            return;
        }
        let (a, b) = (g.paving_frame(i), g.paving_frame(j));
        let (da, db) = (g.datums[i], g.datums[j]);
        let wl = if e.walks[0] >= self.o.min_sidewalk_width { e.walks[0] } else { 0.0 };
        let wr = if e.walks[1] >= self.o.min_sidewalk_width { e.walks[1] } else { 0.0 };
        let (l, r) = (e.half + wl, -(e.half + wr));
        // (`seams`) a clipped side ends at its clip, no footway.
        let ((la, ha), (lb, hb)) = (g.clip(i, e.half), g.clip(j, e.half));
        let side = |c: f64, full: f64, edge: f64| if (c - edge).abs() > 1e-3 { c } else { full };
        let (ra, rb) = (side(la, r, -e.half), side(lb, r, -e.half));
        let (lfa, lfb) = (side(ha, l, e.half), side(hb, l, e.half));
        if lfa - ra < 1e-4 && lfb - rb < 1e-4 {
            return;
        }
        m.quad(Layer::Carriageway, [p3(a.at(ra), da), p3(b.at(rb), db), p3(b.at(lfb), db), p3(a.at(lfa), da)], [self.run_tints(ei).0; 4], None);
    }

    /// The far level of a node: its pad over the mouths' kerb corners.
    pub(crate) fn emit_far_node(&self, m: &mut TileMesh, ni: usize) {
        let o = &self.o;
        let node = &self.plan.nodes[ni];
        let mut ring: Vec<([f64; 2], f64)> = vec![];
        let mut sum = 0.0;
        for app in node.approaches.iter().filter(|a| !(a.merge_trim.is_some() && crate::seams::road_seams())) {
            let (f, d) = self.approach_datum(app);
            let s = if app.at_start { 1.0 } else { -1.0 };
            ring.push((f.at(-s * app.half), d));
            ring.push((f.at(s * app.half), d));
            sum += d;
        }
        if ring.len() < 3 {
            return;
        }
        let apex = p3(node.position, sum / (ring.len() / 2).max(1) as f64);
        for k in 0..ring.len() {
            let (p, py) = ring[k];
            let (q, qy) = ring[(k + 1) % ring.len()];
            m.tri(Layer::Carriageway, [apex, p3(q, qy), p3(p, py)], [PAD(); 3]);
        }
        let _ = (o, PI);
    }
}
