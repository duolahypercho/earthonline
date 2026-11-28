//! The ground the streets stand in (`--street-ground`, on by default;
//! `--no-street-ground` restores the ground as before).
//!
//! The drawn terrain is a grid of 8 m cells (10 m on the 1.25 plan). Before
//! this module, the carpet was held under every street, out to a whole cell
//! past its paved corridor, at the lowest centreline of any street that
//! reached the vertex (`StreetWorld::terrain_cap`). A cell is too coarse to
//! step between two streets at different levels, so wherever two such
//! streets came within a cell of each other the lower one dug the ground
//! out from under the higher one. Hillside blocks, frontage roads and
//! interchange ramps were left floating over pits with cliffs at their edges,
//! and every junction on a slope was dug to its lowest approach. A citywide
//! scan found 93 % of the paving floating more than 3 m over the drawn ground
//! there, and 4,455 of 10,521 junctions over a hole deeper than 1 m.
//!
//! Here the street world draws its own ground near the streets, on a 2 m
//! lattice per street tile (`ground_overlay`), with the near street packs:
//!
//! * away from the streets, the surveyed ground (the elevation grid, the
//!   carpet's lift, the shore's press), not dug; but eased down to the 8 m
//!   carpet in a building's footprint and over `HOLD_FADE_M` round it (a
//!   building stands on the street's floor; the bare-earth survey is
//!   interpolated through its footprint, up the slope behind it);
//! * under a street's paved corridor (its carriageway and the walk on that
//!   side), held under that street (`StreetWorld::street_ground`) at its own
//!   centreline level, as the old cap did, but not past the corridor: just
//!   past it the street's banks and retaining faces meet the surveyed ground
//!   (the retaining edge's wall stands where it rises behind the walk, the
//!   bank falls to it where it drops);
//! * not in a tunnel's terrain cut (`set_ground_cut`): left open there, as
//!   the terrain and the collision heightfield leave it;
//! * at a junction, its approaches' levels blended by distance
//!   (`PAD_SAFETY_M` under them), not the lowest of them;
//! * on the lattice (`overlay_grid`), not drawn under the paving, and the
//!   vertices next to it snapped onto the paving's edge, so the ground meets
//!   the top of a retaining wall, the foot of a bank, or the walk at grade
//!   along the edge itself: no 2 m triangle reaches up through a walk, and
//!   none is held down into a ditch behind one.
//!
//! The lattice is drawn only where it stands above the 8 m carpet (where the
//! old cap dug), so elsewhere the terrain tiles draw the ground exactly as
//! before; under the overlay the carpet is lower and hidden. The banks and
//! retaining faces at the back of the walk meet this ground
//! (`StreetWorld::edge_ground`), and the overlay is walkable street geometry,
//! so the player stands on what is drawn.

use crate::{Layer, StreetWorld, TileMesh};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

/// The overlay lattice's step, game metres.
pub const FINE_STEP_M: f64 = 2.0;
/// A junction's ground is held this far under its approaches' blended level
/// (the pad is a fan between them, not their blend).
pub const PAD_SAFETY_M: f64 = 0.25;
/// The overlay draws a lattice triangle where a corner stands this far over
/// the 8 m carpet (elsewhere the terrain tiles draw the same ground).
pub const SHOW_M: f64 = 0.03;
/// The drawn lattice stands at least this far under any street paving
/// at a vertex or a step from it.
pub const UNDER_PAVING_M: f64 = 0.05;
/// Round a building the street ground eases from the 8 m carpet (at the
/// footprint) to its own height over this distance, game metres (three
/// lattice steps: a smooth bank, not a lattice-sized saw along a footprint
/// edge oblique to the lattice).
pub const HOLD_FADE_M: f64 = 6.0;
/// The street ground is not drawn where the ground or the carpet stands
/// less than this over the sea (the water and the shore's own passes draw it there), game metres.
pub const SHORE_KEEP_M: f64 = 0.1;
/// The lattice steps `HOLD_FADE_M` spans.
const HOLD_STEPS: i64 = 3;
/// Paving standing this far over the street ground is a raised stub or
/// ramp, with the ground drawn under it (`overlay_grid`), game metres.
pub const CLEAR_M: f64 = 2.5;
/// Halvings that find where the paving's edge crosses a lattice edge (6: to
/// 3 cm on a 2 m edge).
const SNAP_ITERATIONS: usize = 6;
/// A vertex snapped onto the paving's edge stands at the ground this far
/// past it: where the street's retaining wall reads its top (`mesh`'s rise).
const SNAP_PROBE_M: f64 = 0.3;
/// A snapped vertex with no wall at the edge stands this far under the
/// paving's edge, game metres.
const EDGE_TUCK_M: f64 = 0.02;
/// The least plan area a drawn lattice triangle keeps (twice it, square metres).
const MIN_TRI_AREA2: f64 = 0.02;
/// Where a triangle on a vertex under the paving is sampled for a gap the
/// paving leaves open (barycentric: the centroid, the edges' midpoints, and
/// halfway from the centroid to each corner).
const GAP_SAMPLES: [[f64; 3]; 7] = [
    [1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0],
    [0.5, 0.5, 0.0],
    [0.0, 0.5, 0.5],
    [0.5, 0.0, 0.5],
    [2.0 / 3.0, 1.0 / 6.0, 1.0 / 6.0],
    [1.0 / 6.0, 2.0 / 3.0, 1.0 / 6.0],
    [1.0 / 6.0, 1.0 / 6.0, 2.0 / 3.0],
];
/// The most a gap's triangle lowers its vertices under the paving, game metres
/// (more: it is left open). A triangle lowered further is steep where it shows
/// (a pit seen through the gap, a saw along a curved edge).
const GAP_MAX_LOWER_M: f64 = 0.15;
/// Two pavings this far apart in height (game metres) with open ground
/// between two lattice vertices under them: a sliver whose vertices move onto
/// its edges (a smaller step is a gap the gap triangles close).
const SLIVER_STEP_M: f64 = 0.12;
/// The most a sliver's triangle (every vertex under the paving: a gap narrower
/// than the lattice) lowers its vertices, game metres: past it, the two
/// pavings are at two levels and their banks or walls meet.
const GAP_MAX_LOWER_SLIVER_M: f64 = 0.6;
/// A gap's triangle has at least this many of its `GAP_SAMPLES` open (fewer: a
/// sliver past a curved edge, not a gap).
const GAP_MIN_OPEN: usize = 2;
/// A street cut into the hill: the ground behind its walk's back edge rises
/// at most this (rise over run: 1V:1.5H, the usual steepest graded soil cut
/// slope; INFERRED, no SF standard for a street's cut slope is recorded).
const CUT_SLOPE: f64 = 1.0 / 1.5;
/// The cut slope's run, game metres (within the corridor index's reach past
/// a walk, `TERRAIN_CAP_MARGIN_M` on the ground plan): ground rising more than
/// the slope gains over it stands on a wall at the slope's foot.
const CUT_BAND_M: f64 = 9.0;

/// A lone lattice vertex under paving is sampled this far round it: under a
/// sliver (fewer than two of four samples paved) it is open ground.
const SLIVER_PROBE_M: f64 = 0.5;
/// Lattice rings past a tile an island is looked for in (`street_islands`):
/// an island up to `ISLAND_MAX_STEPS` across touching the tile lies wholly
/// inside, so every tile it touches finds it the same.
const ISLAND_WINDOW: i64 = 13;
/// An island's extent at most, lattice steps (24 m).
const ISLAND_MAX_STEPS: i64 = 12;
/// An island's paving round it is the road's (the carriageway) at least this share.
const ISLAND_ROAD_SHARE: f64 = 0.75;
/// Open ground the paving encloses with less road round it (a gore between
/// two walks, a strip between a walk and a slip lane) up to this many lattice
/// steps across is filled flush with the paving round it.
const FLUSH_ISLAND_MAX_STEPS: i64 = 6;

/// A junction holds the ground under its pad out to this far past its
/// outline (the hull of its approach mouths; the corner returns' kerbs bow
/// out past it), game metres.
const NODE_CAP_MARGIN_M: f64 = 1.0;

/// An island's outline is traced on a grid this fine, game metres.
const ISLAND_TRACE_M: f64 = 0.5;
/// An island's traced outline is thinned to within this, game metres.
const ISLAND_THIN_M: f64 = 0.06;

/// A closed outline thinned (Douglas-Peucker, split at its two farthest points).
fn thin(p: &[[f64; 2]], tol: f64) -> Vec<[f64; 2]> {
    fn dp(p: &[[f64; 2]], tol: f64, out: &mut Vec<[f64; 2]>) {
        let (a, b) = (p[0], p[p.len() - 1]);
        let (ex, ez) = (b[0] - a[0], b[1] - a[1]);
        let l = (ex * ex + ez * ez).sqrt().max(1e-12);
        let (mut far, mut at) = (0.0, 0);
        for (k, q) in p.iter().enumerate().take(p.len() - 1).skip(1) {
            let d = ((q[0] - a[0]) * ez - (q[1] - a[1]) * ex).abs() / l;
            if d > far {
                far = d;
                at = k;
            }
        }
        if far > tol {
            dp(&p[..=at], tol, out);
            dp(&p[at..], tol, out);
        } else {
            out.push(a);
        }
    }
    if p.len() < 4 {
        return p.to_vec();
    }
    let far = (1..p.len()).max_by(|&a, &b| {
        let d = |k: usize| (p[k][0] - p[0][0]).powi(2) + (p[k][1] - p[0][1]).powi(2);
        d(a).total_cmp(&d(b))
    }).unwrap();
    let mut out = vec![];
    dp(&p[..=far], tol, &mut out);
    let mut back: Vec<[f64; 2]> = p[far..].to_vec();
    back.push(p[0]);
    dp(&back, tol, &mut out);
    out
}

/// A simple counter-clockwise polygon's triangles (ear clipping).
fn triangulate(p: &[[f64; 2]]) -> Vec<[[f64; 2]; 3]> {
    let cross = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let mut idx: Vec<usize> = (0..p.len()).collect();
    let mut out = vec![];
    let mut guard = 0;
    while idx.len() > 3 && guard < 10_000 {
        guard += 1;
        let n = idx.len();
        let mut clipped = false;
        for k in 0..n {
            let (a, b, c) = (p[idx[(k + n - 1) % n]], p[idx[k]], p[idx[(k + 1) % n]]);
            if cross(a, b, c) <= 1e-12 {
                continue;
            }
            let ear = idx.iter().all(|&q| {
                let r = p[q];
                r == a || r == b || r == c || !(cross(a, b, r) >= 0.0 && cross(b, c, r) >= 0.0 && cross(c, a, r) >= 0.0)
            });
            if ear {
                out.push([a, b, c]);
                idx.remove(k);
                clipped = true;
                break;
            }
        }
        if !clipped {
            break;
        }
    }
    if idx.len() == 3 {
        out.push([p[idx[0]], p[idx[1]], p[idx[2]]]);
    }
    out
}

/// The height of triangle `t` (x, y, z) over (x, z), None outside it.
fn tri_height(t: &[[f64; 3]; 3], x: f64, z: f64) -> Option<f64> {
    let [p0, p1, p2] = *t;
    let det = (p1[0] - p0[0]) * (p2[2] - p0[2]) - (p2[0] - p0[0]) * (p1[2] - p0[2]);
    if det.abs() < 1e-12 {
        return None;
    }
    let l1 = ((x - p0[0]) * (p2[2] - p0[2]) - (p2[0] - p0[0]) * (z - p0[2])) / det;
    let l2 = ((p1[0] - p0[0]) * (z - p0[2]) - (x - p0[0]) * (p1[2] - p0[2])) / det;
    (l1 >= -1e-9 && l2 >= -1e-9 && l1 + l2 <= 1.0 + 1e-9).then(|| p0[1] + (p1[1] - p0[1]) * l1 + (p2[1] - p0[1]) * l2)
}

/// Whether (x, z) lies in the counter-clockwise convex `hull` grown by `margin`.
fn inside_convex(hull: &[[f64; 2]], x: f64, z: f64, margin: f64) -> bool {
    (0..hull.len()).all(|i| {
        let (a, b) = (hull[i], hull[(i + 1) % hull.len()]);
        let (ex, ez) = (b[0] - a[0], b[1] - a[1]);
        let l = (ex * ex + ez * ez).sqrt().max(1e-9);
        (ex * (z - a[1]) - ez * (x - a[0])) / l >= -margin
    })
}

/// The lattice neighbours a vertex shares a triangle with (the cells' diagonal runs (0,0)-(1,1)).
const STAR: [(i64, i64); 6] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, -1)];

/// One street tile's street ground on its lattice (`StreetWorld::overlay_grid`).
pub struct OverlayGrid {
    /// The lattice index of the tile's first vertex.
    pub i0: i64,
    pub j0: i64,
    /// Vertices a side.
    pub side: usize,
    /// Vertex positions (x, y, z), row by row: the lattice point, or where
    /// it is snapped onto the paving's edge.
    pub pos: Vec<[f64; 3]>,
    /// Shading normals.
    pub nrm: Vec<[f32; 3]>,
    /// Per cell: whether its triangles are drawn ([(0,0),(0,1),(1,1)], [(0,0),(1,1),(1,0)]).
    pub drawn: Vec<[bool; 2]>,
    /// Per vertex: an island's edge vertex, the paving's level at it (its kerb
    /// face's foot); NaN elsewhere (`street_islands`).
    pub kerb: Vec<f32>,
    /// The islands traced from the paving's edge (`StreetWorld::island_outline`)
    /// this tile draws: their tops' triangles and their kerb faces (each with
    /// its outward direction).
    pub island_tris: Vec<[[f64; 3]; 3]>,
    pub island_faces: Vec<([[f64; 3]; 4], [f64; 2])>,
}

/// 0: not told (read `EO_STREET_GROUND`); 1 off; 2 on.
static ON: AtomicU8 = AtomicU8::new(0);

/// eo-app, before the street world is built: whether the street ground is drawn.
pub fn set_street_ground(on: bool) {
    ON.store(if on { 2 } else { 1 }, Ordering::Relaxed);
}

/// Whether the street ground is drawn (`--street-ground`; default on).
pub fn street_ground_on() -> bool {
    match ON.load(Ordering::Relaxed) {
        0 => {
            let on = std::env::var("EO_STREET_GROUND").map(|v| v.trim() != "0" && v.trim() != "off").unwrap_or(true);
            ON.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
        v => v == 2,
    }
}


/// (`--no-street-islands`, `EO_STREET_ISLANDS=0`) a small piece of open ground
/// a road's paving encloses (a traffic circle's island, a pork chop between a
/// slip lane and the road) is drawn at the ground, flush with the road or
/// under it, as before; on (the default) it is a kerbed island: raised one
/// kerb face over the paving round it and closed by a kerb face.
pub fn street_islands() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("EO_STREET_ISLANDS").map(|v| v.trim() != "0" && v.trim() != "off").unwrap_or(true))
}

/// (`--no-cut-slopes`, `EO_CUT_SLOPES=0`) the ground behind a walk rises
/// straight to the survey (a retaining wall the whole height, as before); on
/// (the default) it rises from the walk's back edge at most `CUT_SLOPE`, a
/// graded cut slope, the wall only the rest over `CUT_BAND_M`.
pub fn cut_slopes() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("EO_CUT_SLOPES").map(|v| v.trim() != "0" && v.trim() != "off").unwrap_or(true))
}

/// The share of its own height the street ground keeps at a lattice vertex
/// `d` metres from the nearest building footprint (0 in it, 1 from `HOLD_FADE_M` on).
fn hold_weight(d: f64) -> f64 {
    let t = (d / HOLD_FADE_M).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The distance from lattice vertex (0, 0) to the nearest lattice point inside
/// a building within `HOLD_STEPS` (`inside(dx, dz)`), game metres; infinite if none.
fn building_distance(inside: impl Fn(i64, i64) -> bool) -> f64 {
    let mut best = f64::INFINITY;
    for dz in -HOLD_STEPS..=HOLD_STEPS {
        for dx in -HOLD_STEPS..=HOLD_STEPS {
            let d = ((dx * dx + dz * dz) as f64).sqrt() * FINE_STEP_M;
            if d < best && inside(dx, dz) {
                best = d;
            }
        }
    }
    best
}

impl StreetWorld {
    /// The ground the streets stand in at (x, z) (module docs): the surveyed
    /// ground, held under the paved corridors and junctions. Analytic (no tile mesh is read), so the banks can stand on it.
    pub fn street_ground(&self, x: f64, z: f64) -> f64 {
        let real = self.terrain.grid.ground(x, z) + self.terrain.carpet_lift;
        let y = match self.fine_cap(x, z) {
            Some(c) => real.min(c),
            None => real,
        };
        let y = match self.shore_press.get() {
            Some(p) => p(x, z, y),
            None => y,
        };
        match self.cut_floor(x, z) {
            Some(f) => y.min(f),
            None => y,
        }
    }

    /// Whether the shore's ground rule moves the ground at (x, z) (a beach on
    /// its foreshore, a seawall's quay): the 8 m carpet draws it there and the
    /// street ground is not drawn (it would stand over the water).
    pub fn shore_pressed(&self, x: f64, z: f64) -> bool {
        let y = self.terrain.grid.ground(x, z) + self.terrain.carpet_lift;
        // the shore and the water: the carpet and the water's own passes draw
        // them (the surf, the sand, a pier's deck over its piles), not the street ground.
        if y < eo_data::SEA_LEVEL_Y + SHORE_KEEP_M || self.ground_water.get().is_some_and(|w| w(x, z)) {
            return true;
        }
        let Some(p) = self.shore_press.get() else { return false };
        (p(x, z, y) - y).abs() > 0.01
    }

    /// Install where the water's press moves the drawn terrain (once; later
    /// calls are ignored): no street ground there. The street tiles drawn
    /// before it are dropped.
    pub fn set_ground_water(&self, water: crate::GroundWater) {
        if self.ground_water.set(water).is_ok() {
            self.drawn_cache.lock().unwrap().clear();
            self.overlay_cache.lock().unwrap().clear();
        }
    }

    /// Install the grade-separated roads' terrain cuts (once; later calls are
    /// ignored): the street ground leaves them open. The street tiles drawn
    /// before it are dropped.
    pub fn set_ground_cut(&self, cut: crate::GroundCut) {
        if self.ground_cut.set(cut).is_ok() {
            self.drawn_cache.lock().unwrap().clear();
            self.overlay_cache.lock().unwrap().clear();
        }
    }

    /// The floor of the terrain cut holding (x, z), if any.
    pub fn cut_floor(&self, x: f64, z: f64) -> Option<f64> {
        self.ground_cut.get().and_then(|c| c(x, z))
    }

    /// The ground the street's own banks and retaining faces meet at (x, z):
    /// `street_ground` with the street ground on, else the 8 m carpet.
    pub fn edge_ground(&self, x: f64, z: f64) -> f64 {
        if street_ground_on() { self.street_ground(x, z) } else { self.carpet_y(x, z) }
    }

    /// The highest the ground may stand at (x, z) under a paved corridor or a
    /// junction: under the corridor's own centreline
    /// level (as `terrain_cap`), under a junction's approaches blended by
    /// distance. None away from the streets.
    fn fine_cap(&self, x: f64, z: f64) -> Option<f64> {
        let k = ((x / 32.0).floor() as i32, (z / 32.0).floor() as i32);
        let mut cap: Option<f64> = None;
        let lift = self.terrain.carpet_lift - self.o.road_lift;
        let margin = 0.0;
        let cut = cut_slopes();
        let mut real: Option<f64> = None;
        if let Some(list) = self.road_cells.get(&k) {
            for &id in list {
                let (ei, a, b, _) = self.road_pieces[id as usize];
                let e = &self.plan.entries[ei as usize];
                let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
                let len2 = dx * dx + dz * dz;
                if len2 < 1e-12 {
                    continue;
                }
                let t = ((x - a[0]) * dx + (z - a[1]) * dz) / len2;
                // past a run's own end, its cut slope rounds the end (no crease
                // where the band would stop square across a corner's slope)
                // (`seams`) round an interior vertex or a joint with the next run, as the ribbon's mitre
                let past_joint = !(0.0..=1.0).contains(&t) && self.piece_t(ei as usize, a, t).is_some();
                let end_cap = cut && !past_joint && ((t < 0.0 && e.points.first() == Some(&a)) || (t > 1.0 && e.points.last() == Some(&b)));
                if !(0.0..=1.0).contains(&t) && !end_cap && !past_joint {
                    continue;
                }
                let t = t.clamp(0.0, 1.0);
                let (px, pz) = (a[0] + dx * t, a[1] + dz * t);
                // the walk on the point's own side (plan frames: walks[0] on
                // the normal (-dz, dx), walks[1] opposite), not the wider of the two:
                // a street with a wide walk on one side only does not hold the
                // open ground down on the other.
                let side = (x - px) * -dz + (z - pz) * dx;
                let walk = if side >= 0.0 { e.walks[0] } else { e.walks[1] };
                let reach = e.half + walk;
                let dist = ((x - px).powi(2) + (z - pz).powi(2)).sqrt();
                if dist > reach + margin || end_cap {
                    // past the walk: the cut slope from its back edge (`cut_slopes`)
                    if cut && dist <= reach + CUT_BAND_M {
                        let station = e.cum[crate::seg_index(e, a)] + t * len2.sqrt();
                        if let Some(d) = self.geoms.get(ei as usize).and_then(|g| self.datum_at_station(g, station)) {
                            let edge = if walk > 0.0 { self.o.sidewalk_surface_y(d, reach, e.half) } else { self.o.cross_section_y(d, e.half, e.half) };
                            let over = (real.get_or_insert_with(|| self.terrain.grid.ground(x, z) + self.terrain.carpet_lift).to_owned() - edge - CUT_SLOPE * CUT_BAND_M).max(0.0);
                            let y = edge + CUT_SLOPE * (dist - reach - SNAP_PROBE_M).max(0.0) + over;
                            cap = Some(cap.map_or(y, |c: f64| c.min(y)));
                        }
                    }
                    continue;
                }
                let mut y = self.ground(px, pz) + self.o.road_lift + lift;
                // a stub raised or lowered to a structure (`joins`), a run moved to meet
                // a busier one's surface (`seams`): its drawn datum.
                if e.structure_ends.iter().any(Option::is_some) || !e.structure_ramps.is_empty() || self.geoms.get(ei as usize).is_some_and(|g| g.seamed) {
                    if let Some(d) = self.geoms.get(ei as usize).and_then(|g| self.datum_at_station(g, e.cum[crate::seg_index(e, a)] + t * len2.sqrt())) {
                        y = y.min(d + lift);
                    }
                }
                cap = Some(cap.map_or(y, |c: f64| c.min(y)));
            }
        }
        if let Some(list) = self.node_cells.get(&k) {
            for &ni in list {
                let n = &self.plan.nodes[ni as usize];
                let hull = &self.node_hulls()[ni as usize];
                if hull.len() >= 3 {
                    // a junction's pad only (`NODE_CAP_MARGIN_M` past its outline), not the
                    // disc round it: that reached past the corner walks into the
                    // ground behind them, a trench behind every uphill corner.
                    if !inside_convex(hull, x, z, NODE_CAP_MARGIN_M) {
                        continue;
                    }
                } else {
                    let r = n.approaches.iter().map(|a| a.trim + a.half).fold(0.0, f64::max);
                    if ((x - n.position[0]).powi(2) + (z - n.position[1]).powi(2)).sqrt() > r + margin {
                        continue;
                    }
                }
                let (mut wsum, mut ysum) = (0.0, 0.0);
                for app in &n.approaches {
                    let f = crate::plan::approach_end_frame(&self.plan.entries, app);
                    let d2 = ((x - f.x).powi(2) + (z - f.z).powi(2)).max(0.25);
                    let w = 1.0 / d2;
                    wsum += w;
                    let d = self.geoms.get(app.entry).filter(|g| g.seamed).and_then(|g| self.datum_at_station(g, f.s)).unwrap_or(self.ground(f.x, f.z) + self.o.road_lift);
                    ysum += w * (d + lift);
                }
                if wsum > 0.0 {
                    let y = ysum / wsum - PAD_SAFETY_M;
                    cap = Some(cap.map_or(y, |c: f64| c.min(y)));
                }
            }
        }
        cap
    }



    /// Each junction's pad outline (`StreetWorld::node_hulls`).
    fn node_hulls(&self) -> &Vec<Vec<[f64; 2]>> {
        self.node_hulls.get_or_init(|| {
            self.plan
                .nodes
                .iter()
                .map(|n| {
                    if n.approaches.len() < 3 {
                        return vec![];
                    }
                    let mut pts = vec![];
                    for app in &n.approaches {
                        let f = crate::plan::approach_end_frame(&self.plan.entries, app);
                        pts.push(f.at(-app.half));
                        pts.push(f.at(app.half));
                    }
                    let h = crate::plan::convex_hull(pts);
                    if h.len() >= 3 { h } else { vec![] }
                })
                .collect()
        })
    }

    #[doc(hidden)]
    /// (diagnostic) what `fine_cap` reads at (x, z): (what, level).
    pub fn fine_cap_sources(&self, x: f64, z: f64) -> Vec<(String, f64)> {
        let mut out = vec![];
        let k = ((x / 32.0).floor() as i32, (z / 32.0).floor() as i32);
        let lift = self.terrain.carpet_lift - self.o.road_lift;
        if let Some(list) = self.road_cells.get(&k) {
            for &id in list {
                let (ei, a, b, _) = self.road_pieces[id as usize];
                let e = &self.plan.entries[ei as usize];
                let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
                let len2 = dx * dx + dz * dz;
                if len2 < 1e-12 { continue; }
                let t = ((x - a[0]) * dx + (z - a[1]) * dz) / len2;
                if !(0.0..=1.0).contains(&t) { continue; }
                let (px, pz) = (a[0] + dx * t, a[1] + dz * t);
                let side = (x - px) * -dz + (z - pz) * dx;
                let reach = e.half + if side >= 0.0 { e.walks[0] } else { e.walks[1] };
                let d = ((x - px).powi(2) + (z - pz).powi(2)).sqrt();
                if d > reach { continue; }
                let seg = &self.city.segments[e.road];
                out.push((format!("run {ei} {} {} d {:.1} reach {:.1} (half {:.1} walks {:?})", seg.id, e.highway, d, reach, e.half, e.walks), self.ground(px, pz) + self.o.road_lift + lift));
            }
        }
        if let Some(list) = self.node_cells.get(&k) {
            for &ni in list {
                let n = &self.plan.nodes[ni as usize];
                let r = n.approaches.iter().map(|a| a.trim + a.half).fold(0.0, f64::max);
                let d = ((x - n.position[0]).powi(2) + (z - n.position[1]).powi(2)).sqrt();
                let hull = &self.node_hulls()[ni as usize];
                if if hull.len() >= 3 { !inside_convex(hull, x, z, NODE_CAP_MARGIN_M) } else { d > r } { continue; }
                out.push((format!("node {ni} r {r:.1} d {d:.1} approaches {}", n.approaches.len()), f64::NAN));
            }
        }
        out
    }

    /// The lowest street paving drawn over (x, z) (any walkable street
    /// triangle of the paving, `cached_mesh`; not the street ground) that
    /// stands on the ground (less than `CLEAR_M` over the street ground; the
    /// ground is drawn under a raised stub or ramp), if any.
    fn low_paving(&self, x: f64, z: f64) -> Option<f64> {
        let p = self.paved_heights(x, z).into_iter().reduce(f64::min)?;
        (p - self.street_ground(x, z) < CLEAR_M).then_some(p)
    }

    /// `low_paving` with its layer.
    fn low_paving_layer(&self, x: f64, z: f64) -> Option<(f64, Layer)> {
        let (p, l) = self.paved_layers(x, z).into_iter().reduce(|a, b| if b.0 < a.0 { b } else { a })?;
        (p - self.street_ground(x, z) < CLEAR_M).then_some((p, l))
    }

    /// The street ground of one street tile on its 2 m lattice (cached; the
    /// drawing, the collision and `overlay_height` all read it), None where
    /// the tile draws none.
    ///
    /// A lattice cannot hold the edge of the paving where it runs obliquely
    /// to it: a 2 m triangle from under a walk to the open ground behind it
    /// either reaches up through the walk or, held under it, digs a ditch
    /// behind the walk's retaining wall (a row of pits that shade as dark
    /// spots along every street on a slope). So the lattice is made to meet
    /// the paving's edge: a vertex under the paving is not drawn (nor any
    /// triangle on it), and a vertex next to one (along a triangle's edge)
    /// is moved along that edge onto the paving's edge (the nearest such
    /// crossing). Its height there is the ground just past the edge, where
    /// the street's retaining wall reads its top (`SNAP_PROBE_M`), a little
    /// under that top where the wall stands; where no wall stands, the
    /// paving's own edge (the bank's foot, the walk at grade), just under it.
    /// Every vertex is computed from the lattice round it to a fixed reach
    /// (the tiles' margins), so two tiles agree on their edge. Away from
    /// the paving, `street_ground`, eased down to the 8 m carpet round a
    /// building (`HOLD_FADE_M`).
    pub fn overlay_grid(&self, key: (i32, i32)) -> Option<Arc<OverlayGrid>> {
        if !street_ground_on() || !self.tiles.contains_key(&key) {
            return None;
        }
        if let Some(g) = self.overlay_cache.lock().unwrap().get(&key) {
            return g.clone();
        }
        let g = self.build_overlay_grid(key).map(Arc::new);
        let mut c = self.overlay_cache.lock().unwrap();
        if c.len() > 2048 {
            c.clear();
        }
        c.insert(key, g.clone());
        g
    }

    fn build_overlay_grid(&self, key: (i32, i32)) -> Option<OverlayGrid> {
        let tm = crate::STREET_TILE_M;
        let n = (tm / FINE_STEP_M).round() as i64;
        let side = (n + 1) as usize;
        let (i0, j0) = ((key.0 as f64 * tm / FINE_STEP_M).round() as i64, (key.1 as f64 * tm / FINE_STEP_M).round() as i64);
        // margins (lattice rings past the tile): the vertices one ring (the
        // normals' triangles); the paving one more (a vertex's star); the
        // buildings HOLD_STEPS more than the vertices.
        const R: i64 = 1;
        let ge = side + 2 * R as usize;
        let pe_r = R + 1;
        let pe = side + 2 * pe_r as usize;
        let be_r = R + HOLD_STEPS;
        let be = side + 2 * be_r as usize;
        let at = |i: i64, j: i64| (i as f64 * FINE_STEP_M, j as f64 * FINE_STEP_M);
        // the paving under the lattice, out to the islands' window
        let islands_on = street_islands();
        let wr = if islands_on { R + ISLAND_WINDOW } else { pe_r };
        let we = side + 2 * wr as usize;
        let mut wmin = vec![f64::INFINITY; we * we];
        let mut wroad = vec![false; we * we];
        for j in 0..we {
            for i in 0..we {
                let (x, z) = at(i0 + i as i64 - wr, j0 + j as i64 - wr);
                if let Some((p, l)) = self.low_paving_layer(x, z) {
                    wmin[j * we + i] = p;
                    wroad[j * we + i] = l == Layer::Carriageway;
                }
            }
        }
        // a lone vertex on a sliver of paving (a kerb corner's tip, a pad's
        // point) is not under it: its six triangles left undrawn were a hole
        // four metres across round a few centimetres of paving
        let wi = we as i64;
        let lone: Vec<usize> = (0..we * we)
            .filter(|&k| {
                let (ki, kj) = ((k % we) as i64, (k / we) as i64);
                wmin[k].is_finite()
                    && STAR.iter().all(|&(di, dj)| {
                        let (a, b) = (ki + di, kj + dj);
                        a >= 0 && b >= 0 && a < wi && b < wi && !wmin[b as usize * we + a as usize].is_finite()
                    })
            })
            .collect();
        for k in lone {
            let (x, z) = at(i0 + (k % we) as i64 - wr, j0 + (k / we) as i64 - wr);
            let h = SLIVER_PROBE_M;
            let covered = [(h, 0.0), (-h, 0.0), (0.0, h), (0.0, -h)].iter().filter(|(dx, dz)| self.low_paving(x + dx, z + dz).is_some()).count();
            if covered < 2 {
                wmin[k] = f64::INFINITY;
                wroad[k] = false;
            }
        }
        let mut pmin = vec![f64::INFINITY; pe * pe];
        let wo = (wr - pe_r) as usize;
        for j in 0..pe {
            for i in 0..pe {
                pmin[j * pe + i] = wmin[(j + wo) * we + i + wo];
            }
        }
        // the islands: open ground the road's paving encloses, small (each a
        // plane through the paving round it, raised one kerb face)
        let mut island: Vec<Option<usize>> = vec![None; we * we];
        // (centre x, z, y, gradient x, z, the lift over that plane)
        let mut planes: Vec<[f64; 6]> = vec![];
        if islands_on {
            let mut seen = vec![false; we * we];
            for start in 0..we * we {
                if seen[start] || wmin[start].is_finite() {
                    continue;
                }
                let (mut members, mut queue) = (vec![start], vec![start]);
                seen[start] = true;
                let mut border = false;
                while let Some(k) = queue.pop() {
                    let (ki, kj) = ((k % we) as i64, (k / we) as i64);
                    for (di, dj) in STAR {
                        let (a, b) = (ki + di, kj + dj);
                        if a < 0 || b < 0 || a >= wi || b >= wi {
                            border = true;
                            continue;
                        }
                        let q = b as usize * we + a as usize;
                        if !seen[q] && !wmin[q].is_finite() {
                            seen[q] = true;
                            members.push(q);
                            queue.push(q);
                        }
                    }
                }
                // (the whole component is drained first, so no part of a large one is taken for an island)
                if members.len() > ((ISLAND_MAX_STEPS + 1) * (ISLAND_MAX_STEPS + 1)) as usize {
                    continue;
                }
                if border || members.len() < 3 {
                    continue;
                }
                let (mut i_lo, mut i_hi, mut j_lo, mut j_hi) = (i64::MAX, i64::MIN, i64::MAX, i64::MIN);
                for &k in &members {
                    let (ki, kj) = ((k % we) as i64, (k / we) as i64);
                    i_lo = i_lo.min(ki);
                    i_hi = i_hi.max(ki);
                    j_lo = j_lo.min(kj);
                    j_hi = j_hi.max(kj);
                }
                if i_hi - i_lo > ISLAND_MAX_STEPS || j_hi - j_lo > ISLAND_MAX_STEPS {
                    continue;
                }
                // the paving round it: the road's at least ISLAND_ROAD_SHARE; its plane
                let (mut edge, mut road) = (0usize, 0usize);
                let mut pts: Vec<[f64; 3]> = vec![];
                for &k in &members {
                    let (ki, kj) = ((k % we) as i64, (k / we) as i64);
                    let mut on_edge = false;
                    let mut by_road = false;
                    for (di, dj) in STAR {
                        let q = (kj + dj) as usize * we + (ki + di) as usize;
                        if wmin[q].is_finite() {
                            on_edge = true;
                            by_road |= wroad[q];
                            let (x, z) = at(i0 + ki + di - wr, j0 + kj + dj - wr);
                            pts.push([x, z, wmin[q]]);
                        }
                    }
                    edge += on_edge as usize;
                    road += by_road as usize;
                }
                if edge < 3 {
                    continue;
                }
                // the road round most of it: a kerbed island, one kerb face proud;
                // else (a gore between two walks at an acute corner, a strip between
                // a walk and a slip lane) a small one is filled flush with the paving
                // round it: held under the walks' level it was a pit between them.
                let kerbed = (road as f64) >= ISLAND_ROAD_SHARE * edge as f64;
                if !kerbed && (i_hi - i_lo > FLUSH_ISLAND_MAX_STEPS || j_hi - j_lo > FLUSH_ISLAND_MAX_STEPS) {
                    continue;
                }
                let nf = pts.len() as f64;
                let (mx, mz, my) = (pts.iter().map(|p| p[0]).sum::<f64>() / nf, pts.iter().map(|p| p[1]).sum::<f64>() / nf, pts.iter().map(|p| p[2]).sum::<f64>() / nf);
                let (mut sxx, mut sxz, mut szz, mut sxy, mut szy) = (0.0, 0.0, 0.0, 0.0, 0.0);
                for p in &pts {
                    let (dx, dz, dy) = (p[0] - mx, p[1] - mz, p[2] - my);
                    sxx += dx * dx;
                    sxz += dx * dz;
                    szz += dz * dz;
                    sxy += dx * dy;
                    szy += dz * dy;
                }
                let det = sxx * szz - sxz * sxz;
                let (gx, gz) = if det.abs() > 1e-6 { ((sxy * szz - szy * sxz) / det, (szy * sxx - sxy * sxz) / det) } else { (0.0, 0.0) };
                let id = planes.len();
                planes.push([mx, mz, my, gx, gz, if kerbed { self.o.curb_face_height } else { -EDGE_TUCK_M }]);
                for &k in &members {
                    island[k] = Some(id);
                }
            }
        }
        let island_y = |id: usize, x: f64, z: f64| {
            let p = planes[id];
            p[2] + p[3] * (x - p[0]) + p[4] * (z - p[1]) + p[5]
        };
        // each island traced from the paving's own edge where it can be (a
        // simple outline): its lattice triangles are not drawn, and the tile
        // holding the outline's first lattice member draws it
        let mut traced = vec![false; planes.len()];
        let (mut island_tris, mut island_faces) = (vec![], vec![]);
        for id in 0..planes.len() {
            let members: Vec<usize> = (0..we * we).filter(|&k| island[k] == Some(id)).collect();
            // (only an island on this tile's own lattice, its ring included, concerns it)
            let lo = (wr - R) as usize;
            if !members.iter().any(|&k| (lo..lo + ge).contains(&(k % we)) && (lo..lo + ge).contains(&(k / we))) {
                continue;
            }
            let pts: Vec<[f64; 2]> = members.iter().map(|&k| { let (x, z) = at(i0 + (k % we) as i64 - wr, j0 + (k / we) as i64 - wr); [x, z] }).collect();
            let Some(outline) = self.island_outline(&pts) else { continue };
            traced[id] = true;
            // the owner: the tile holding the island's lowest lattice index (the same in every tile)
            let first = members.iter().map(|&k| (j0 + (k / we) as i64 - wr, i0 + (k % we) as i64 - wr)).min().unwrap();
            let owner = crate::tile_key(first.1 as f64 * FINE_STEP_M, first.0 as f64 * FINE_STEP_M);
            if owner != key {
                continue;
            }
            let y = |p: [f64; 2]| island_y(id, p[0], p[1]);
            for t in triangulate(&outline) {
                island_tris.push(t.map(|p| [p[0], y(p), p[1]]));
            }
            let n = outline.len();
            for a in 0..n {
                let (p, q) = (outline[a], outline[(a + 1) % n]);
                // (counter-clockwise in x, z: outward is the edge turned clockwise)
                let (ex, ez) = (q[0] - p[0], q[1] - p[1]);
                let l = (ex * ex + ez * ez).sqrt().max(1e-9);
                let out = [ez / l, -ex / l];
                let foot = |r: [f64; 2]| {
                    let o = [r[0] + out[0] * 0.15, r[1] + out[1] * 0.15];
                    self.low_paving(o[0], o[1]).unwrap_or(y(r) - self.o.curb_face_height).min(y(r)) - UNDER_PAVING_M
                };
                island_faces.push(([[p[0], y(p), p[1]], [q[0], y(q), q[1]], [q[0], foot(q), q[1]], [p[0], foot(p), p[1]]], out));
            }
        }
        let mut bld = vec![false; be * be];
        for j in 0..be {
            for i in 0..be {
                let (x, z) = at(i0 + i as i64 - be_r, j0 + j as i64 - be_r);
                bld[j * be + i] = self.buildings.contains(x, z);
            }
        }
        let rise = self.o.curb_face_height;
        // each vertex: under the paving, snapped to its edge, or free
        let mut pos = vec![[0.0f64; 3]; ge * ge];
        let mut under = vec![false; ge * ge];
        // the vertices standing on the open survey itself (not held by a
        // street, a building or the paving's edge): shaded as the terrain is
        let mut natural = vec![false; ge * ge];
        // a free vertex snapped onto a paving's edge: that paving's height there (NaN elsewhere)
        let mut edge_ph = vec![f64::NAN; ge * ge];
        // an island's edge vertex: the paving's level at it
        let mut kerb_at = vec![f64::NAN; ge * ge];
        // a traced island's lattice vertex (its triangles are not drawn)
        let mut in_traced = vec![false; ge * ge];
        for j in 0..ge {
            for i in 0..ge {
                let k = j * ge + i;
                let isl = island[(j + (wr - R) as usize) * we + i + (wr - R) as usize];
                in_traced[k] = isl.is_some_and(|id| traced[id]);
                let (li, lj) = (i0 + i as i64 - R, j0 + j as i64 - R);
                let (x, z) = at(li, lj);
                let pk = |di: i64, dj: i64| ((j as i64 + dj + (pe_r - R)) as usize) * pe + (i as i64 + di + (pe_r - R)) as usize;
                if pmin[pk(0, 0)].is_finite() {
                    // a vertex under one paving whose neighbour along the star is
                    // under another at another level, with open ground between
                    // (a sliver narrower than the lattice between a junction's pad
                    // and a run, two runs of a cluster): moved onto its paving's edge
                    // facing the sliver, at that edge, so the sliver's triangles
                    // span it from edge to edge (held under both, they could not
                    // reach the step between them and the 8 m carpet showed, a pit).
                    let here = pmin[pk(0, 0)];
                    let mut best: Option<(f64, [f64; 3])> = None;
                    for (di, dj) in STAR {
                        let q = pmin[pk(di, dj)];
                        if !q.is_finite() || (q - here).abs() < SLIVER_STEP_M {
                            continue;
                        }
                        let (nx, nz) = at(li + di, lj + dj);
                        if !self.paved_heights((x + nx) / 2.0, (z + nz) / 2.0).is_empty() {
                            continue;
                        }
                        let (mut lo, mut hi, mut ph) = (0.0f64, 0.5f64, here);
                        for _ in 0..SNAP_ITERATIONS {
                            let t = 0.5 * (lo + hi);
                            match self.low_paving(x + (nx - x) * t, z + (nz - z) * t) {
                                Some(p) => {
                                    lo = t;
                                    ph = p;
                                }
                                None => hi = t,
                            }
                        }
                        if best.is_none_or(|(bt, _)| lo < bt) {
                            best = Some((lo, [x + (nx - x) * lo, ph - EDGE_TUCK_M, z + (nz - z) * lo]));
                        }
                    }
                    if let Some((_, p)) = best {
                        pos[k] = p;
                        continue;
                    }
                    under[k] = true;
                    pos[k] = [x, here - UNDER_PAVING_M, z];
                    continue;
                }
                let (bi, bj) = (i as i64 + HOLD_STEPS, j as i64 + HOLD_STEPS);
                let d = building_distance(|dx, dz| bld[((bj + dz) as usize) * be + (bi + dx) as usize]);
                let held = |px: f64, pz: f64, y: f64| -> f64 {
                    if d.is_finite() {
                        let c = self.carpet_y(px, pz);
                        if y > c {
                            return c + hold_weight(d) * (y - c);
                        }
                    }
                    y
                };
                // the nearest crossing of the paving's edge along the vertex's star:
                // (share of the edge, the paving's height just past it, the edge's direction)
                let mut best: Option<(f64, f64, [f64; 2])> = None;
                for (di, dj) in STAR {
                    let q = pmin[pk(di, dj)];
                    if !q.is_finite() {
                        continue;
                    }
                    let (nx, nz) = at(li + di, lj + dj);
                    let (mut lo, mut hi, mut ph) = (0.0f64, 1.0f64, q);
                    for _ in 0..SNAP_ITERATIONS {
                        let t = 0.5 * (lo + hi);
                        match self.low_paving(x + (nx - x) * t, z + (nz - z) * t) {
                            Some(p) => {
                                hi = t;
                                ph = p;
                            }
                            None => lo = t,
                        }
                    }
                    if best.is_none_or(|b| lo < b.0) {
                        best = Some((lo, ph, [nx - x, nz - z]));
                    }
                }
                pos[k] = match best {
                    None => {
                        let y = self.street_ground(x, z);
                        let real = self.terrain.grid.ground(x, z) + self.terrain.carpet_lift;
                        natural[k] = !d.is_finite() && (y - real).abs() < 0.01;
                        [x, held(x, z, y), z]
                    }
                    Some((t, ph, dir)) => {
                        edge_ph[k] = ph;
                        let len = (dir[0] * dir[0] + dir[1] * dir[1]).sqrt();
                        let (px, pz) = (x + dir[0] * t, z + dir[1] * t);
                        if isl.is_some_and(|id| !traced[id]) {
                            kerb_at[k] = ph;
                        }
                        let back = SNAP_PROBE_M.min(t * len) / len;
                        let (qx, qz) = (px - dir[0] * back, pz - dir[1] * back);
                        let g = held(qx, qz, self.street_ground(qx, qz));
                        let y = if g - ph > rise { g - UNDER_PAVING_M } else { g.min(ph) - EDGE_TUCK_M };
                        [px, y, pz]
                    }
                };
                if let Some(id) = isl {
                    pos[k][1] = island_y(id, pos[k][0], pos[k][2]);
                    natural[k] = false;
                }
            }
        }
        // a vertex under one paving next to a free vertex snapped onto another
        // paving's edge at another level, open ground between (a sliver narrower
        // than the lattice: its one free row snaps to its nearer edge and leaves
        // the other bare): moved onto its own paving's edge toward it, so the
        // sliver's triangles span it from edge to edge.
        let snapped_under: Vec<(usize, [f64; 3])> = (0..ge * ge)
            .filter(|&k| under[k])
            .filter_map(|k| {
                let (i, j) = ((k % ge) as i64, (k / ge) as i64);
                let (li, lj) = (i0 + i - R, j0 + j - R);
                let (x, z) = at(li, lj);
                let here = pos[k][1] + UNDER_PAVING_M;
                let mut best: Option<(f64, [f64; 3])> = None;
                for (di, dj) in STAR {
                    let (ni, nj) = (i + di, j + dj);
                    if ni < 0 || nj < 0 || ni >= ge as i64 || nj >= ge as i64 {
                        continue;
                    }
                    let n = nj as usize * ge + ni as usize;
                    if under[n] || !edge_ph[n].is_finite() || (edge_ph[n] - here).abs() < SLIVER_STEP_M {
                        continue;
                    }
                    let (nx, nz) = at(li + di, lj + dj);
                    if !self.paved_heights((x + nx) / 2.0, (z + nz) / 2.0).is_empty() {
                        continue;
                    }
                    let (mut lo, mut hi, mut ph) = (0.0f64, 0.5f64, here);
                    for _ in 0..SNAP_ITERATIONS {
                        let t = 0.5 * (lo + hi);
                        match self.low_paving(x + (nx - x) * t, z + (nz - z) * t) {
                            Some(p) => {
                                lo = t;
                                ph = p;
                            }
                            None => hi = t,
                        }
                    }
                    if best.is_none_or(|(bt, _)| lo < bt) {
                        best = Some((lo, [x + (nx - x) * lo, ph - EDGE_TUCK_M, z + (nz - z) * lo]));
                    }
                }
                best.map(|(_, p)| (k, p))
            })
            .collect();
        for (k, p) in snapped_under {
            under[k] = false;
            pos[k] = p;
        }
        let gcell = ge - 1;
        let tri_of = |i: usize, j: usize, t: usize| -> [(usize, usize); 3] {
            if t == 0 { [(i, j), (i, j + 1), (i + 1, j + 1)] } else { [(i, j), (i + 1, j + 1), (i + 1, j)] }
        };
        // a triangle on a vertex under the paving that the paving leaves partly
        // open: a gap narrower than the lattice between two streets, which a
        // vertex snapped to one edge does not close. It is drawn, its vertices
        // under the paving lowered so it stays under that paving (one pass, from
        // the snapped heights: a vertex on a tile's edge has all its triangles in
        // both tiles, so they agree).
        let mut gap = vec![[false; 2]; gcell * gcell];
        let mut lower = vec![0.0f64; ge * ge];
        for j in 0..gcell {
            for i in 0..gcell {
                for t in 0..2 {
                    let tri = tri_of(i, j, t);
                    let und = tri.map(|(a, b)| under[b * ge + a]);
                    if !und.iter().any(|&u| u) {
                        continue;
                    }
                    let ps = tri.map(|(a, b)| pos[b * ge + a]);
                    // every vertex under the paving: a gap narrower than the
                    // lattice between two pavings (a sliver between a junction's
                    // pad and a run, two runs of a cluster) left no triangle,
                    // and the 8 m carpet showed through it, a pit up to a metre
                    // deep. Drawn too where the paving leaves it open (first
                    // the cheap test: no paving at all at the samples).
                    if und.iter().all(|&u| u) {
                        let open = GAP_SAMPLES
                            .iter()
                            .filter(|bc| {
                                let at = |c: usize| bc[0] * ps[0][c] + bc[1] * ps[1][c] + bc[2] * ps[2][c];
                                self.paved_heights(at(0), at(2)).is_empty()
                            })
                            .count();
                        if open < GAP_MIN_OPEN {
                            continue;
                        }
                    }
                    let (mut open, mut need) = (0usize, 0.0f64);
                    for bc in GAP_SAMPLES {
                        let at = |c: usize| bc[0] * ps[0][c] + bc[1] * ps[1][c] + bc[2] * ps[2][c];
                        match self.low_paving(at(0), at(2)) {
                            None => open += 1,
                            Some(p) => {
                                let poke = at(1) - (p - EDGE_TUCK_M);
                                if poke > 0.0 {
                                    let share: f64 = (0..3).filter(|&q| und[q]).map(|q| bc[q]).sum();
                                    need = need.max(poke / share.max(1e-3));
                                }
                            }
                        }
                    }
                    // (a sliver between two pavings at two levels: lowered under the
                    // lower one it shows a step at the higher one's edge, not the carpet
                    // a metre down)
                    let most = if und.iter().all(|&u| u) { GAP_MAX_LOWER_SLIVER_M } else { GAP_MAX_LOWER_M };
                    if open < GAP_MIN_OPEN || need > most {
                        continue;
                    }
                    gap[j * gcell + i][t] = true;
                    for q in 0..3 {
                        if und[q] {
                            let k = tri[q].1 * ge + tri[q].0;
                            lower[k] = lower[k].max(need);
                        }
                    }
                }
            }
        }
        for (p, l) in pos.iter_mut().zip(&lower) {
            p[1] -= l;
        }
        // the triangles that may be drawn: none of their vertices under the
        // paving (or a gap's), not folded
        let mut drawable = vec![[false; 2]; gcell * gcell];
        let mut nacc = vec![[0.0f64; 3]; ge * ge];
        for j in 0..gcell {
            for i in 0..gcell {
                for t in 0..2 {
                    let tri = tri_of(i, j, t);
                    if tri.iter().any(|&(a, b)| in_traced[b * ge + a]) {
                        continue;
                    }
                    if !gap[j * gcell + i][t] && tri.iter().any(|&(a, b)| under[b * ge + a]) {
                        continue;
                    }
                    let [p0, p1, p2] = tri.map(|(a, b)| pos[b * ge + a]);
                    let (u, v) = ([p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]], [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]]);
                    // plan area (x, z), negative the lattice's own winding
                    if u[0] * v[2] - u[2] * v[0] > -MIN_TRI_AREA2 {
                        continue;
                    }
                    drawable[j * gcell + i][t] = true;
                    // the face normal (area weighted), up
                    let c = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
                    let s = if c[1] < 0.0 { -1.0 } else { 1.0 };
                    for &(a, b) in &tri {
                        let acc = &mut nacc[b * ge + a];
                        for q in 0..3 {
                            acc[q] += s * c[q];
                        }
                    }
                }
            }
        }
        // the tile's vertices: positions, normals, what is shown
        let off = R as usize;
        let mut out_pos = vec![[0.0f64; 3]; side * side];
        let mut nrm = vec![[0.0f32; 3]; side * side];
        let mut show = vec![false; side * side];
        let mut cut = vec![false; side * side];
        let mut any = false;
        for j in 0..side {
            for i in 0..side {
                let g = (j + off) * ge + i + off;
                let p = pos[g];
                let k = j * side + i;
                out_pos[k] = p;
                let a = nacc[g];
                let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
                let mut n = if l > 1e-12 { [a[0] / l, a[1] / l, a[2] / l] } else { [0.0, 1.0, 0.0] };
                // the open survey shaded as the terrain tiles shade it (central
                // differences over a survey cell, `smooth_terrain`), not by the
                // 2 m faces of its bilinear cells (their creases read as blocks);
                // blended to the faces' by the share of the vertex's star that is natural.
                let w = (std::iter::once((0i64, 0i64)).chain(STAR).filter(|&(di, dj)| natural[((j + off) as i64 + dj) as usize * ge + ((i + off) as i64 + di) as usize]).count()) as f64 / 7.0;
                if w > 0.0 {
                    let real = |x: f64, z: f64| self.terrain.grid.ground(x, z);
                    let c = self.terrain.grid.meta.cell_size;
                    let hx = (real(p[0] + c, p[2]) - real(p[0] - c, p[2])) / (2.0 * c);
                    let hz = (real(p[0], p[2] + c) - real(p[0], p[2] - c)) / (2.0 * c);
                    let sl = (hx * hx + 1.0 + hz * hz).sqrt();
                    let sm = [-hx / sl, 1.0 / sl, -hz / sl];
                    let b = [0, 1, 2].map(|q| w * sm[q] + (1.0 - w) * n[q]);
                    let bl = (b[0] * b[0] + b[1] * b[1] + b[2] * b[2]).sqrt().max(1e-12);
                    n = b.map(|v| v / bl);
                }
                nrm[k] = n.map(|v| v as f32);
                // (not on the shore's or the water's ground: the carpet and the water draw them)
                show[k] = p[1] - self.carpet_y(p[0], p[2]) > SHOW_M && !self.shore_pressed(p[0], p[2]);
                any |= show[k];
                cut[k] = self.cut_floor(p[0], p[2]).is_some();
            }
        }
        if !any && island_tris.is_empty() {
            return None;
        }
        let ncell = side - 1;
        let mut drawn = vec![[false; 2]; ncell * ncell];
        for j in 0..ncell {
            for i in 0..ncell {
                for t in 0..2 {
                    if !drawable[(j + off) * gcell + i + off][t] {
                        continue;
                    }
                    let tri = tri_of(i, j, t);
                    let shown = tri.iter().any(|&(a, b)| show[b * side + a]);
                    // a tunnel's cut is left open (as the terrain and the collision leave it)
                    let in_cut = tri.iter().any(|&(a, b)| cut[b * side + a]);
                    drawn[j * ncell + i][t] = shown && !in_cut;
                }
            }
        }
        let kerb: Vec<f32> = (0..side * side).map(|k| kerb_at[(k / side + off) * ge + k % side + off] as f32).collect();
        Some(OverlayGrid { i0, j0, side, pos: out_pos, nrm, drawn, kerb, island_tris, island_faces })
    }

    /// An island's outline traced from the paving's edge round its lattice
    /// members `pts` (counter-clockwise in x, z), or None where it is not one
    /// simple outline within reach (left to the lattice): the open ground
    /// round the members flooded on a `ISLAND_TRACE_M` grid, its edge by
    /// marching squares, each crossing found on the paving's own edge, then
    /// thinned (`ISLAND_THIN_M`).
    fn island_outline(&self, pts: &[[f64; 2]]) -> Option<Vec<[f64; 2]>> {
        let h = ISLAND_TRACE_M;
        let (mut x0, mut x1, mut z0, mut z1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for p in pts {
            x0 = x0.min(p[0]);
            x1 = x1.max(p[0]);
            z0 = z0.min(p[1]);
            z1 = z1.max(p[1]);
        }
        let pad = 2.0 * FINE_STEP_M;
        let (x0, z0) = (((x0 - pad) / h).floor() * h, ((z0 - pad) / h).floor() * h);
        let nx = ((x1 + pad - x0) / h).ceil() as usize + 1;
        let nz = ((z1 + pad - z0) / h).ceil() as usize + 1;
        let at = |i: usize, j: usize| [x0 + i as f64 * h, z0 + j as f64 * h];
        let open: Vec<bool> = (0..nx * nz).map(|k| self.low_paving(at(k % nx, k / nx)[0], at(k % nx, k / nx)[1]).is_none()).collect();
        let mut inside = vec![false; nx * nz];
        let mut queue: Vec<usize> = vec![];
        for p in pts {
            let (i, j) = (((p[0] - x0) / h).round() as usize, ((p[1] - z0) / h).round() as usize);
            let k = j * nx + i;
            if i < nx && j < nz && open[k] && !inside[k] {
                inside[k] = true;
                queue.push(k);
            }
        }
        while let Some(k) = queue.pop() {
            let (i, j) = (k % nx, k / nx);
            if i == 0 || j == 0 || i + 1 == nx || j + 1 == nz {
                return None; // not closed by the paving within reach
            }
            for q in [k - 1, k + 1, k - nx, k + nx] {
                if open[q] && !inside[q] {
                    inside[q] = true;
                    queue.push(q);
                }
            }
        }
        // the crossing on a grid edge between an inside sample and an outside one
        let cross = |a: [f64; 2], b: [f64; 2]| -> [f64; 2] {
            let (mut lo, mut hi) = (0.0, 1.0); // lo inside (open), hi paved side
            for _ in 0..5 {
                let t = 0.5 * (lo + hi);
                if self.low_paving(a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t).is_none() { lo = t } else { hi = t }
            }
            [a[0] + (b[0] - a[0]) * lo, a[1] + (b[1] - a[1]) * lo]
        };
        // marching squares: each cell's segments between its edges (0 bottom, 1 right, 2 top, 3 left)
        let edge_id = |i: usize, j: usize, e: usize| -> (usize, usize, u8) {
            match e {
                0 => (i, j, 0),     // (i,j)-(i+1,j)
                1 => (i + 1, j, 1), // (i+1,j)-(i+1,j+1)
                2 => (i, j + 1, 0), // (i,j+1)-(i+1,j+1)
                _ => (i, j, 1),     // (i,j)-(i,j+1)
            }
        };
        let mut adj: std::collections::HashMap<(usize, usize, u8), Vec<(usize, usize, u8)>> = std::collections::HashMap::new();
        for j in 0..nz - 1 {
            for i in 0..nx - 1 {
                let c = [inside[j * nx + i], inside[j * nx + i + 1], inside[(j + 1) * nx + i + 1], inside[(j + 1) * nx + i]];
                let case = c[0] as u8 | (c[1] as u8) << 1 | (c[2] as u8) << 2 | (c[3] as u8) << 3;
                let segs: &[(usize, usize)] = match case {
                    1 | 14 => &[(3, 0)],
                    2 | 13 => &[(0, 1)],
                    3 | 12 => &[(3, 1)],
                    4 | 11 => &[(1, 2)],
                    6 | 9 => &[(0, 2)],
                    7 | 8 => &[(3, 2)],
                    5 => &[(3, 0), (1, 2)],
                    10 => &[(0, 1), (2, 3)],
                    _ => &[],
                };
                for &(a, b) in segs {
                    let (ea, eb) = (edge_id(i, j, a), edge_id(i, j, b));
                    adj.entry(ea).or_default().push(eb);
                    adj.entry(eb).or_default().push(ea);
                }
            }
        }
        if adj.values().any(|v| v.len() != 2) {
            return None;
        }
        // one loop round all of it (more: holes or pieces, left to the lattice)
        let start = *adj.keys().min()?;
        let mut ring = vec![start];
        let (mut prev, mut cur) = (start, adj[&start][0]);
        while cur != start {
            ring.push(cur);
            let nb = &adj[&cur];
            let next = if nb[0] == prev { nb[1] } else { nb[0] };
            prev = cur;
            cur = next;
            if ring.len() > adj.len() {
                return None;
            }
        }
        if ring.len() != adj.len() || ring.len() < 3 {
            return None;
        }
        let mut poly: Vec<[f64; 2]> = ring
            .iter()
            .map(|&(i, j, d)| {
                let (a, b) = (at(i, j), if d == 0 { at(i + 1, j) } else { at(i, j + 1) });
                let (ka, kb) = (j * nx + i, if d == 0 { j * nx + i + 1 } else { (j + 1) * nx + i });
                if inside[ka] && !inside[kb] { cross(a, b) } else { cross(b, a) }
            })
            .collect();
        poly = thin(&poly, ISLAND_THIN_M);
        if poly.len() < 3 {
            return None;
        }
        let area: f64 = (0..poly.len()).map(|a| { let (p, q) = (poly[a], poly[(a + 1) % poly.len()]); p[0] * q[1] - q[0] * p[1] }).sum();
        if area < 0.0 {
            poly.reverse();
        }
        Some(poly)
    }

    /// The street ground as `ground_overlay` draws it at (x, z) (`overlay_grid`),
    /// None where the terrain tiles draw the ground (or the street ground is off).
    pub fn overlay_height(&self, x: f64, z: f64) -> Option<f64> {
        // a traced island's top (its owner tile, or a neighbour's)
        let k0 = crate::tile_key(x, z);
        for dj in -1..=1 {
            for di in -1..=1 {
                let Some(g) = self.overlay_grid((k0.0 + di, k0.1 + dj)) else { continue };
                for t in &g.island_tris {
                    if let Some(y) = tri_height(t, x, z) {
                        return Some(y);
                    }
                }
            }
        }
        let n = (crate::STREET_TILE_M / FINE_STEP_M).round() as i64;
        let (ci, cj) = ((x / FINE_STEP_M).floor() as i64, (z / FINE_STEP_M).floor() as i64);
        let mut last: Option<((i32, i32), Option<Arc<OverlayGrid>>)> = None;
        // a snapped vertex moves under a step, so a triangle covers its own cell's neighbours at most
        for dj in [0i64, -1, 1] {
            for di in [0i64, -1, 1] {
                let (a, b) = (ci + di, cj + dj);
                let key = (a.div_euclid(n) as i32, b.div_euclid(n) as i32);
                if last.as_ref().is_none_or(|(k, _)| *k != key) {
                    last = Some((key, self.overlay_grid(key)));
                }
                let Some(g) = last.as_ref().and_then(|(_, g)| g.as_ref()) else { continue };
                let (i, j) = ((a - g.i0) as usize, (b - g.j0) as usize);
                let ncell = g.side - 1;
                if i >= ncell || j >= ncell {
                    continue;
                }
                for t in 0..2 {
                    if !g.drawn[j * ncell + i][t] {
                        continue;
                    }
                    let tri = if t == 0 { [(i, j), (i, j + 1), (i + 1, j + 1)] } else { [(i, j), (i + 1, j + 1), (i + 1, j)] };
                    let [p0, p1, p2] = tri.map(|(a, b)| g.pos[b * g.side + a]);
                    let det = (p1[0] - p0[0]) * (p2[2] - p0[2]) - (p2[0] - p0[0]) * (p1[2] - p0[2]);
                    if det.abs() < 1e-12 {
                        continue;
                    }
                    let l1 = ((x - p0[0]) * (p2[2] - p0[2]) - (p2[0] - p0[0]) * (z - p0[2])) / det;
                    let l2 = ((p1[0] - p0[0]) * (z - p0[2]) - (x - p0[0]) * (p1[2] - p0[2])) / det;
                    let eps = -1e-9;
                    if l1 >= eps && l2 >= eps && l1 + l2 <= 1.0 - eps {
                        return Some(p0[1] + (p1[1] - p0[1]) * l1 + (p2[1] - p0[1]) * l2);
                    }
                }
            }
        }
        None
    }

    /// The street ground of one street tile as a mesh (`overlay_grid`'s drawn
    /// triangles). Empty with the street ground off. Layer `Ground`; colours
    /// 0 (the renderer gives the terrain's own).
    pub fn ground_overlay(&self, key: (i32, i32)) -> TileMesh {
        let mut m = TileMesh::default();
        let Some(g) = self.overlay_grid(key) else { return m };
        let side = g.side;
        let ncell = side - 1;
        let mut index = vec![u32::MAX; side * side];
        let mut vert = |m: &mut TileMesh, i: usize, j: usize| -> u32 {
            let k = j * side + i;
            if index[k] == u32::MAX {
                let p = g.pos[k];
                m.positions.push([p[0] as f32, p[1] as f32, p[2] as f32]);
                m.normals.push(g.nrm[k]);
                m.colors.push(0);
                m.kerb.push(-1.0);
                index[k] = (m.positions.len() - 1) as u32;
            }
            index[k]
        };
        for j in 0..ncell {
            for i in 0..ncell {
                let c = [(i, j), (i + 1, j), (i, j + 1), (i + 1, j + 1)];
                for (t, tri) in [[c[0], c[2], c[3]], [c[0], c[3], c[1]]].iter().enumerate() {
                    if !g.drawn[j * ncell + i][t] {
                        continue;
                    }
                    let v: Vec<u32> = tri.iter().map(|&(a, b)| vert(&mut m, a, b)).collect();
                    m.indices.extend_from_slice(&v);
                    m.layers.push(Layer::Ground);
                }
            }
        }
        // an island's kerb face: along each lattice edge between two of its edge
        // vertices with its triangle drawn on one side only, down to the paving
        let drawn_at = |ci: i64, cj: i64, t: usize| ci >= 0 && cj >= 0 && (ci as usize) < ncell && (cj as usize) < ncell && g.drawn[cj as usize * ncell + ci as usize][t];
        let tri_pts = |ci: i64, cj: i64, t: usize| -> [[f64; 3]; 3] {
            let (i, j) = (ci as usize, cj as usize);
            let c = if t == 0 { [(i, j), (i, j + 1), (i + 1, j + 1)] } else { [(i, j), (i + 1, j + 1), (i + 1, j)] };
            c.map(|(a, b)| g.pos[b * side + a])
        };
        for j in 0..side as i64 {
            for i in 0..side as i64 {
                // the edges from (i, j): along x, along z, the diagonal; the triangles either side
                for (di, dj, adj) in [(1i64, 0i64, [(i, j, 1usize), (i, j - 1, 0usize)]), (0, 1, [(i, j, 0), (i - 1, j, 1)]), (1, 1, [(i, j, 0), (i, j, 1)])] {
                    let (a, b) = (i + di, j + dj);
                    if a >= side as i64 || b >= side as i64 {
                        continue;
                    }
                    let (ku, kv) = (j as usize * side + i as usize, b as usize * side + a as usize);
                    let (fu, fv) = (g.kerb[ku], g.kerb[kv]);
                    if !fu.is_finite() || !fv.is_finite() {
                        continue;
                    }
                    let on: Vec<(i64, i64, usize)> = adj.iter().copied().filter(|&(ci, cj, t)| drawn_at(ci, cj, t)).collect();
                    if on.len() != 1 {
                        continue;
                    }
                    let (pu, pv) = (g.pos[ku], g.pos[kv]);
                    let tp = tri_pts(on[0].0, on[0].1, on[0].2);
                    let (cx, cz) = ((tp[0][0] + tp[1][0] + tp[2][0]) / 3.0, (tp[0][2] + tp[1][2] + tp[2][2]) / 3.0);
                    let out = [(pu[0] + pv[0]) / 2.0 - cx, (pu[2] + pv[2]) / 2.0 - cz];
                    let foot = |p: [f64; 3], f: f32| (f as f64).min(p[1]) - UNDER_PAVING_M;
                    m.quad(
                        Layer::CurbFace,
                        [[pu[0], pu[1], pu[2]], [pv[0], pv[1], pv[2]], [pv[0], foot(pv, fv), pv[2]], [pu[0], foot(pu, fu), pu[2]]],
                        [crate::mesh::CURB_FACE(); 4],
                        Some(out),
                    );
                }
            }
        }
        // the traced islands: their tops (the lattice's winding) and kerb faces
        for t in &g.island_tris {
            let (u, v) = ([t[1][0] - t[0][0], t[1][1] - t[0][1], t[1][2] - t[0][2]], [t[2][0] - t[0][0], t[2][1] - t[0][1], t[2][2] - t[0][2]]);
            let mut nr = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
            if nr[1] < 0.0 {
                nr = nr.map(|c| -c);
            }
            let l = (nr[0] * nr[0] + nr[1] * nr[1] + nr[2] * nr[2]).sqrt().max(1e-12);
            // wound as the lattice's own triangles: clockwise in x, z seen from above
            let cw = u[0] * v[2] - u[2] * v[0] < 0.0;
            let order = if cw { [0, 1, 2] } else { [0, 2, 1] };
            for &q in &order {
                m.positions.push([t[q][0] as f32, t[q][1] as f32, t[q][2] as f32]);
                m.normals.push([(nr[0] / l) as f32, (nr[1] / l) as f32, (nr[2] / l) as f32]);
                m.colors.push(0);
                m.kerb.push(-1.0);
            }
            let b = m.positions.len() as u32 - 3;
            m.indices.extend_from_slice(&[b, b + 1, b + 2]);
            m.layers.push(Layer::Ground);
        }
        for (q, out) in &g.island_faces {
            m.quad(Layer::CurbFace, *q, [crate::mesh::CURB_FACE(); 4], Some(*out));
        }
        m
    }

    /// The near street tile as drawn and collided with: the paving
    /// (`cached_mesh`) and, with the street ground on, its ground (`ground_overlay`).
    pub fn drawn_mesh(&self, key: (i32, i32)) -> std::sync::Arc<TileMesh> {
        let paving = self.cached_mesh(key);
        if !street_ground_on() {
            return paving;
        }
        if let Some(m) = self.drawn_cache.lock().unwrap().get(&key) {
            return m.clone();
        }
        let over = self.ground_overlay(key);
        let out = if over.indices.is_empty() {
            paving
        } else {
            let mut m = (*paving).clone();
            let base = m.positions.len() as u32;
            m.positions.extend_from_slice(&over.positions);
            m.normals.extend_from_slice(&over.normals);
            m.colors.extend_from_slice(&over.colors);
            m.kerb.extend_from_slice(&over.kerb);
            m.indices.extend(over.indices.iter().map(|i| i + base));
            m.layers.extend_from_slice(&over.layers);
            std::sync::Arc::new(m)
        };
        let mut c = self.drawn_cache.lock().unwrap();
        if c.len() > 2048 {
            c.clear();
        }
        c.insert(key, out.clone());
        out
    }
}
