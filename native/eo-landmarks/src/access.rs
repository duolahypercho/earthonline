//! Enterable landmarks: the walkable decks at each landmark's best view,
//! their floors, railings, glazing, parapets and lamps, the lifts that take
//! a player up and down, and the building prisms the decks open in the
//! collision world. No GPU and no physics: `eo-render` draws the decks'
//! `structures()` with the rest of the set, and `eo-player`'s collision world
//! takes the same floors (`walk`), barriers (`barrier`), lifts and prism cuts
//! (eo-net `game::landmark_decks` converts them), so the client, the server
//! and the drawing stand on one geometry. `--no-landmark-access` turns all of
//! it off (`set::Options::access`), and the towers are built as before.
//!
//! Every deck is placed on the landmark's own generated model (towers.rs,
//! golden_gate.rs, bay_bridge.rs): the same site record, frame, ground grid
//! and plan shift, so the deck stands where the drawn landmark is.
//!
//! | Deck | Where | Real-world values | Source |
//! | --- | --- | --- | --- |
//! | Coit Tower gallery | the belvedere's arcade, a ring between the outer wall and the stair shaft | tower 210 ft (64 m); the observation deck 32 ft (9.8 m) below the top, an arcade with skylights above; three nested cylinders of unpainted reinforced concrete, the elevator in the inner one, the stair in the middle one | Wikipedia "Coit Tower" (citing the NRHP nomination and SF Recreation and Parks). The model's arcade is the LiDAR one of towers.rs; the floor is set so the drawn sills stand a guard height (below) over it |
//! | Salesforce Tower observation level | the 61st (top) floor, the "Ohana Floor", glazed all round under the crown | 1,070 ft (326 m), 61 floors; ceilings over 13 ft (3.96 m) and 10 ft (3.05 m) of continuous clear glass | salesforcetower.com/about; Salesforce blog "A look inside Salesforce Tower" (2018: the 61st floor's 360-degree views) |
//! | Transamerica Pyramid observation room | the 48th (top) floor under the 212 ft (64.6 m) spire, windows on all four sides, the elevator and stair wings on its east and west faces | 853 ft (260 m), 48 floors; the 48th floor about 2,025 to 2,531 sq ft (188 to 235 m2) with views in every direction | Wikipedia "Transamerica Pyramid"; Axios San Francisco, 18 June 2024 (the 48th floor and the spire's stair) |
//! | Golden Gate Bridge east sidewalk | along the whole span, round the towers on their walkways | sidewalk 10 ft (3.05 m); outer railing 4 ft (1.22 m) | Golden Gate Bridge Highway and Transportation District, "Design and construction stats" (goldengate.org); the deck's values are the generator's (golden_gate.rs) |
//! | Golden Gate south tower top | both legs' tops and a catwalk between them, by the leg's service elevator | tower 746 ft (227.4 m) above the water, 500 ft (152 m) above the roadway | goldengate.org "Design and construction stats" |
//! | Bay Bridge W2 tower top | the tower nearest San Francisco: both legs' tops and the top strut between them | tower top 160 m above the water (the export's `towerTopAboveWaterM`) | the generator's data (bay_bridge.rs) |
//! | Sutro Tower upper platform | the platform level where the three legs end and the masts begin | 977 ft (297.8 m); the platform (level 6) 751 ft (228.9 m) above the base; a two-person elevator inside one leg | Wikipedia "Sutro Tower"; sutrotower.com FAQ |
//! | City Hall lantern gallery | a balcony round the lantern over the dome | dome 307 ft 6 in (93.7 m) | the generator's data (towers.rs `city_hall`: `domeTopM`, `lanternRadiusM`, `lanternTopM`) |
//!
//! Guards: California Building Code 2022, section 1015.3: guards at least
//! 42 in (1,067 mm) high where the public stands; OSHA 29 CFR 1910.29(b):
//! the guardrail of a working platform (the tower tops, Sutro's platform),
//! top rail 42 in +- 3 in, a midrail halfway, a toeboard at least 3.5 in
//! (0.089 m, 1910.29(k)(1)). Walls, cores, finishes and lamp intensities that
//! no source gives are marked INFERRED.
//!
//! Not built: the Ferry Building's clock-tower balcony (the base build draws
//! the Ferry Building as a 15 m block with no tower to stand on) and Twin
//! Peaks (its summit is open ground the collision heightfield already walks).

use crate::LandmarkLight;
use crate::data::{LandmarksJs, Material, num};
use crate::geometry::{ALL, GroundGrid, MeshArrays, Rgb, V3, pairs, ring_centroid, scale_ring, shade, srgb_hex};
use crate::set::{Structure, ground_of, ground_scale_shift, plain_material};
use serde_json::Value;
use std::f64::consts::{PI, TAU};

/// A collision triangle, world metres.
pub type Tri = [V3; 3];

/// CBC 2022 section 1015.3: a guard's height over the walking surface, metres (42 in).
pub const GUARD_M: f64 = 1.067;
/// OSHA 29 CFR 1910.29(k)(1)(ii): a toeboard's least height, metres (3.5 in).
pub const TOEBOARD_M: f64 = 0.089;
/// The collision guard over every railing, parapet and open arch, metres
/// over the floor: more than a jump lifts the feet (2.7 m/s: 0.37 m) plus
/// the controller's 0.3 m autostep, so a railing stops the walker.
pub const GUARD_COLLIDE_M: f64 = 1.6;
/// A lift stop's reach: the feet within this horizontal distance of the stop.
pub const STOP_REACH_M: f64 = 1.8;
/// ... and within this height of it.
pub const STOP_RISE_M: f64 = 2.0;
/// The eye over the floor for a deck's view, metres (a standing adult's eye, INFERRED).
pub const EYE_M: f64 = 1.62;
/// Guardrail posts at most this far apart (OSHA 1910.29(b)(5) asks the rails
/// to hold 200 lb at any point; 8 ft is the usual post pitch, INFERRED).
const POST_PITCH_M: f64 = 2.4;

/// Where a lift leaves a player: the feet and the compass heading faced
/// (degrees: 0 north (-z), 90 east (+x)).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stop {
    pub feet: V3,
    pub heading_deg: f64,
}

/// A lift between a ground entrance and a deck.
#[derive(Clone, Debug, PartialEq)]
pub struct Lift {
    pub id: String,
    /// What the prompt names ("Coit Tower gallery").
    pub name: String,
    pub bottom: Stop,
    pub top: Stop,
}

/// Building prisms of the collision world whose footprint centroid lies within
/// `radius_m` of `centre` are opened between `y0` and `y1` (the deck's own
/// floor, glazing and core stand there instead).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cut {
    pub centre: [f64; 2],
    pub radius_m: f64,
    pub y0: f64,
    pub y1: f64,
}

/// One landmark's walkable deck.
#[derive(Clone, Debug)]
pub struct Deck {
    pub id: String,
    pub name: String,
    /// The registered landmark it belongs to.
    pub landmark: String,
    /// Walkable floor triangles (the collision world's STREET layer).
    pub walk: Vec<Tri>,
    /// Railings, glazing, parapets, cores (its WORLD layer).
    pub barrier: Vec<Tri>,
    pub lifts: Vec<Lift>,
    pub cuts: Vec<Cut>,
    pub structures: Vec<Structure>,
    pub lights: Vec<LandmarkLight>,
    /// The deck's best view: feet at the railing and the heading looked along.
    pub view: Stop,
    /// The view's pitch, degrees (up positive).
    pub view_pitch_deg: f64,
    /// A camera that shows the deck with a person standing at `view` (eye, look-at).
    pub deck_camera: (V3, V3),
    /// The deck floor's height (the view's floor).
    pub floor_y: f64,
}

/// Every deck.
#[derive(Clone, Debug, Default)]
pub struct AccessSet {
    pub decks: Vec<Deck>,
}

impl AccessSet {
    /// Every deck on the ground `height_at` (the drawn terrain's sampler, as
    /// `LandmarkSet::build` takes it). Deterministic: the same ground gives
    /// the same decks bit for bit (a server and its clients build it apart).
    pub fn build(height_at: &dyn Fn(f64, f64) -> Option<f64>) -> Self {
        let js = LandmarksJs::get();
        let decks = vec![coit(js, height_at), salesforce(js, height_at), transamerica(js, height_at), golden_gate_sidewalk(js, height_at), golden_gate_tower(js, height_at), bay_bridge_tower(js, height_at), sutro(js, height_at), city_hall(js, height_at)];
        Self { decks }
    }

    /// The decks' drawings, as the set's city structures.
    pub fn structures(&self) -> Vec<Structure> {
        self.decks.iter().flat_map(|d| d.structures.iter().cloned()).collect()
    }

    pub fn lights(&self) -> Vec<LandmarkLight> {
        self.decks.iter().flat_map(|d| d.lights.iter().copied()).collect()
    }

    pub fn lifts(&self) -> impl Iterator<Item = &Lift> {
        self.decks.iter().flat_map(|d| d.lifts.iter())
    }

    pub fn deck(&self, id: &str) -> Option<&Deck> {
        self.decks.iter().find(|d| d.id == id)
    }
}

/// Compass heading (degrees) of a plan direction (x east, z south).
pub fn heading_of(dx: f64, dz: f64) -> f64 {
    dx.atan2(-dz).to_degrees().rem_euclid(360.0)
}

/// The plan direction (x, z) of a compass heading.
pub fn dir_of(heading_deg: f64) -> [f64; 2] {
    let h = heading_deg.to_radians();
    [h.sin(), -h.cos()]
}

// ---------------------------------------------------------------------------
// The builder: draws and collision together.
// ---------------------------------------------------------------------------

struct Part {
    key: &'static str,
    material: Material,
    cast: bool,
    mesh: MeshArrays,
}

struct B {
    id: &'static str,
    parts: Vec<Part>,
    walk: Vec<Tri>,
    barrier: Vec<Tri>,
    lights: Vec<LandmarkLight>,
}

/// The finishes. Each is a landmark registry material where the deck is part
/// of that structure (the same roughness, metalness and surface kind as the
/// drawn landmark), else a plain finish.
fn finish(key: &str) -> (Material, bool) {
    let js = LandmarksJs::get();
    let reg = |id: &str, k: &str| js.registry.get(id).and_then(|r| r.materials.get(k)).cloned();
    let m = match key {
        // Coit: the tower's own concrete (unpainted reinforced concrete).
        "coit-concrete" => reg("coit-tower", "concrete"),
        // Golden Gate: the bridge's International Orange painted steel.
        "gg-orange" => reg("golden-gate-bridge", "orange"),
        "gg-deck" => reg("golden-gate-bridge", "concrete"),
        "bb-steel" => reg("bay-bridge-west", "steel"),
        "sutro-steel" => reg("sutro-tower", "steel"),
        "ch-stone" => reg("city-hall", "stone").map(|m| Material { surface: None, surface_options: None, ..m }),
        "ta-precast" => reg("transamerica-pyramid", "precast"),
        "ta-metal" => reg("transamerica-pyramid", "metal"),
        "sf-metal" => reg("salesforce-tower", "crown"),
        _ => None,
    };
    let m = m.unwrap_or_else(|| match key {
        // A steel grating walking surface (galvanised, INFERRED).
        "grating" => Material { surface: Some("painted-steel".into()), ..plain_material(0.72, 0.35, "painted-metal") },
        // Interior floors: polished stone / terrazzo (INFERRED).
        "floor" => plain_material(0.42, 0.0, "facade-masonry"),
        // Interior walls, cores and ceilings: painted plaster (INFERRED).
        "interior" => plain_material(0.9, 0.0, "facade-masonry"),
        // Stainless lift doors (INFERRED).
        "doors" => plain_material(0.3, 0.9, "facade-metal"),
        _ => plain_material(0.85, 0.0, "facade-masonry"),
    });
    (m, true)
}

impl B {
    fn new(id: &'static str) -> Self {
        Self { id, parts: vec![], walk: vec![], barrier: vec![], lights: vec![] }
    }

    fn m(&mut self, key: &'static str) -> &mut MeshArrays {
        if let Some(i) = self.parts.iter().position(|p| p.key == key) {
            return &mut self.parts[i].mesh;
        }
        let (material, cast) = finish(key);
        self.parts.push(Part { key, material, cast, mesh: MeshArrays::new(&format!("access:{}:{key}", self.id)) });
        &mut self.parts.last_mut().unwrap().mesh
    }

    fn walk_quad(&mut self, a: V3, b: V3, c: V3, d: V3) {
        self.walk.push([a, b, c]);
        self.walk.push([a, c, d]);
    }

    /// A flat walkable polygon (any winding, simple).
    fn walk_ring(&mut self, ring: &[[f64; 2]], y: f64) {
        for t in triangulate(ring) {
            self.walk.push(t.map(|p| [p[0], y, p[1]]));
        }
    }

    /// A walkable annulus about (cx, cz).
    fn walk_annulus(&mut self, cx: f64, cz: f64, r0: f64, r1: f64, y: f64, segs: usize) {
        for k in 0..segs {
            let (a, b) = (k as f64 / segs as f64 * TAU, (k + 1) as f64 / segs as f64 * TAU);
            let p = |a: f64, r: f64| [cx + a.cos() * r, y, cz - a.sin() * r];
            self.walk_quad(p(a, r0), p(a, r1), p(b, r1), p(b, r0));
        }
    }

    /// A vertical collision wall from a to b between y0 and y1.
    fn wall(&mut self, a: [f64; 2], b: [f64; 2], y0: f64, y1: f64) {
        self.barrier.push([[a[0], y0, a[1]], [b[0], y0, b[1]], [b[0], y1, b[1]]]);
        self.barrier.push([[a[0], y0, a[1]], [b[0], y1, b[1]], [a[0], y1, a[1]]]);
    }

    /// A collision wall over a sloping run: `lo` below and `hi` above each end's walking height.
    fn wall_over(&mut self, a: V3, b: V3, lo: f64, hi: f64) {
        let (a0, a1, b0, b1) = ([a[0], a[1] - lo, a[2]], [a[0], a[1] + hi, a[2]], [b[0], b[1] - lo, b[2]], [b[0], b[1] + hi, b[2]]);
        self.barrier.push([a0, b0, b1]);
        self.barrier.push([a0, b1, a1]);
    }

    fn wall_ring(&mut self, ring: &[[f64; 2]], y0: f64, y1: f64) {
        for i in 0..ring.len() {
            self.wall(ring[i], ring[(i + 1) % ring.len()], y0, y1);
        }
    }

    fn wall_cylinder(&mut self, cx: f64, cz: f64, r: f64, y0: f64, y1: f64, segs: usize) {
        let ring: Vec<[f64; 2]> = (0..segs).map(|k| k as f64 / segs as f64 * TAU).map(|a| [cx + a.cos() * r, cz - a.sin() * r]).collect();
        self.wall_ring(&ring, y0, y1);
    }

    /// A guardrail (OSHA 1910.29: top rail at the guard height, a midrail
    /// halfway, a toeboard; posts at most `POST_PITCH_M` apart) from a to b
    /// on the walking surface, drawn in `key`, with its collision guard.
    fn guardrail(&mut self, key: &'static str, a: V3, b: V3, colour: Rgb, toe: bool) {
        let m = self.m(key);
        rail_mesh(m, a, b, GUARD_M, colour, toe);
        self.wall_over(a, b, 0.2, GUARD_COLLIDE_M);
    }

    /// A guardrail along a polyline (closed when `closed`).
    fn guardrail_ring(&mut self, key: &'static str, ring: &[V3], colour: Rgb, toe: bool, closed: bool) {
        let n = ring.len();
        let segs = if closed { n } else { n - 1 };
        for i in 0..segs {
            self.guardrail(key, ring[i], ring[(i + 1) % n], colour, toe);
        }
    }

    /// A warm-white lamp (3,000 K) hanging at `pos`, pointing down: `candela`
    /// on its axis, a cone of `angle` half-angle (INFERRED interior downlights).
    fn lamp(&mut self, pos: V3, candela: f64, range: f64, angle: f64) {
        let base = [pos[0], pos[1] - 3.0, pos[2]];
        self.lights.push(crate::lights::light(pos, base, crate::lights::unit_luminance(crate::lights::WARM_WHITE_SRGB), candela, range, angle, 0.5));
    }

    /// The deck, moved by `shift` (a landmark building's plan shift).
    #[allow(clippy::too_many_arguments)]
    fn finish(self, name: &str, landmark: &str, lifts: Vec<Lift>, cuts: Vec<Cut>, view: Stop, view_pitch_deg: f64, deck_camera: (V3, V3), floor_y: f64, shift: [f64; 2]) -> Deck {
        let mv = |p: V3| [p[0] + shift[0], p[1], p[2] + shift[1]];
        let mut structures = Vec::new();
        for p in self.parts {
            let mut mesh = p.mesh.freeze();
            for v in mesh.positions.chunks_exact_mut(3) {
                v[0] += shift[0] as f32;
                v[2] += shift[1] as f32;
            }
            if mesh.is_empty() {
                continue;
            }
            structures.push(Structure { name: format!("access:{}:{}", self.id, p.key), cast: p.cast, footway: false, kit: None, material: p.material, mesh });
        }
        let stop = |s: Stop| Stop { feet: mv(s.feet), ..s };
        Deck {
            id: self.id.to_string(),
            name: name.to_string(),
            landmark: landmark.to_string(),
            walk: self.walk.iter().map(|t| t.map(mv)).collect(),
            barrier: self.barrier.iter().map(|t| t.map(mv)).collect(),
            lifts: lifts.into_iter().map(|l| Lift { bottom: stop(l.bottom), top: stop(l.top), ..l }).collect(),
            cuts: cuts.into_iter().map(|c| Cut { centre: [c.centre[0] + shift[0], c.centre[1] + shift[1]], ..c }).collect(),
            structures,
            lights: self
                .lights
                .into_iter()
                .map(|mut l| {
                    l.pos = mv(l.pos);
                    l.base = mv(l.base);
                    l
                })
                .collect(),
            view: stop(view),
            view_pitch_deg,
            deck_camera: (mv(deck_camera.0), mv(deck_camera.1)),
            floor_y,
        }
    }
}

/// Earcut of a simple ring (plan), as plan triangles.
fn triangulate(ring: &[[f64; 2]]) -> Vec<[[f64; 2]; 3]> {
    let mut ear = earcut::Earcut::<f64>::new();
    let mut tri: Vec<u32> = vec![];
    ear.earcut(ring.iter().copied(), &[] as &[u32], &mut tri);
    tri.chunks_exact(3).map(|t| [ring[t[0] as usize], ring[t[1] as usize], ring[t[2] as usize]]).collect()
}

/// Earcut of a ring with one hole, as plan triangles.
fn triangulate_with_hole(outer: &[[f64; 2]], hole: &[[f64; 2]]) -> Vec<[[f64; 2]; 3]> {
    let pts: Vec<[f64; 2]> = outer.iter().chain(hole.iter()).copied().collect();
    let mut ear = earcut::Earcut::<f64>::new();
    let mut tri: Vec<u32> = vec![];
    ear.earcut(pts.iter().copied(), &[outer.len() as u32], &mut tri);
    tri.chunks_exact(3).map(|t| [pts[t[0] as usize], pts[t[1] as usize], pts[t[2] as usize]]).collect()
}

/// A flat slab over a plan ring: its top at `top`, `thick` deep, fanned top and bottom by earcut.
fn slab(m: &mut MeshArrays, ring: &[[f64; 2]], top: f64, thick: f64, top_colour: Rgb, edge_colour: Rgb) {
    let n = ring.len();
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        m.quad([a[0], top - thick, a[1]], [b[0], top - thick, b[1]], [b[0], top, b[1]], [a[0], top, a[1]], edge_colour);
    }
    for t in triangulate(ring) {
        m.tri([t[0][0], top, t[0][1]], [t[1][0], top, t[1][1]], [t[2][0], top, t[2][1]], top_colour);
        m.tri([t[2][0], top - thick, t[2][1]], [t[1][0], top - thick, t[1][1]], [t[0][0], top - thick, t[0][1]], edge_colour);
    }
}

/// A guardrail's drawing from a to b: posts, top rail, midrail, toeboard.
fn rail_mesh(m: &mut MeshArrays, a: V3, b: V3, height: f64, colour: Rgb, toe: bool) {
    let len = (b[0] - a[0]).hypot(b[2] - a[2]);
    if !(len > 1e-3) {
        return;
    }
    let up = |p: V3, h: f64| [p[0], p[1] + h, p[2]];
    m.beam(up(a, height), up(b, height), 0.05, 0.05, colour);
    m.beam(up(a, height * 0.5), up(b, height * 0.5), 0.035, 0.035, colour);
    if toe {
        m.quad(a, b, up(b, TOEBOARD_M + 0.01), up(a, TOEBOARD_M + 0.01), colour);
    }
    let posts = (len / POST_PITCH_M).ceil().max(1.0) as usize;
    for k in 0..=posts {
        let t = k as f64 / posts as f64;
        let p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
        m.beam(p, up(p, height), 0.045, 0.045, colour);
    }
}

/// A simple polygon moved `d` inward (each edge offset, neighbours intersected).
pub fn inset_ring(ring: &[[f64; 2]], d: f64) -> Vec<[f64; 2]> {
    let n = ring.len();
    let area: f64 = (0..n).map(|i| ring[i][0] * ring[(i + 1) % n][1] - ring[(i + 1) % n][0] * ring[i][1]).sum::<f64>() / 2.0;
    // Inward normal of edge (a -> b): rotate the edge by +-90 degrees by the winding.
    let sgn = if area > 0.0 { 1.0 } else { -1.0 };
    let line = |i: usize| {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        let (ex, ez) = (b[0] - a[0], b[1] - a[1]);
        let l = ex.hypot(ez).max(1e-12);
        let nrm = [-ez / l * sgn, ex / l * sgn];
        ([a[0] + nrm[0] * d, a[1] + nrm[1] * d], [ex / l, ez / l])
    };
    (0..n)
        .map(|i| {
            let (p0, d0) = line((i + n - 1) % n);
            let (p1, d1) = line(i);
            let den = d0[0] * d1[1] - d0[1] * d1[0];
            if den.abs() < 1e-9 {
                return p1;
            }
            let t = ((p1[0] - p0[0]) * d1[1] - (p1[1] - p0[1]) * d1[0]) / den;
            [p0[0] + d0[0] * t, p0[1] + d0[1] * t]
        })
        .collect()
}

/// Distance from `c` along `dir` to a ring's boundary (the nearest crossing ahead).
fn ray_to_ring(c: [f64; 2], dir: [f64; 2], ring: &[[f64; 2]]) -> Option<f64> {
    let n = ring.len();
    let mut best: Option<f64> = None;
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        let e = [b[0] - a[0], b[1] - a[1]];
        let den = dir[0] * e[1] - dir[1] * e[0];
        if den.abs() < 1e-12 {
            continue;
        }
        let w = [a[0] - c[0], a[1] - c[1]];
        let t = (w[0] * e[1] - w[1] * e[0]) / den;
        let u = (w[0] * dir[1] - w[1] * dir[0]) / den;
        if t > 0.0 && (0.0..=1.0).contains(&u) && best.is_none_or(|b| t < b) {
            best = Some(t);
        }
    }
    best
}

fn v2(v: &Value) -> [f64; 2] {
    [v[0].as_f64().unwrap_or(0.0), v[1].as_f64().unwrap_or(0.0)]
}

fn flat(v: &Value) -> Vec<f64> {
    v.as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect()).unwrap_or_default()
}

fn hexc(v: &Value, k: &str) -> Rgb {
    srgb_hex(num(v, k) as u32)
}

/// A landmark building's ground as `LandmarkSet::build` samples it (the site's
/// grid on the ground of its scaled site) and the plan shift it is moved by.
fn site_ground(js: &LandmarksJs, id: &str, height_at: &dyn Fn(f64, f64) -> Option<f64>) -> (GroundGrid, [f64; 2]) {
    let d = ground_scale_shift(js, id);
    let shifted = |x: f64, z: f64| height_at(x + d[0], z + d[1]);
    let g = if d == [0.0, 0.0] { ground_of(js, id, height_at) } else { ground_of(js, id, &shifted) };
    (g, d)
}

fn base_y(ring: &[[f64; 2]], g: &GroundGrid) -> f64 {
    ring.iter().map(|p| g.at(p[0], p[1])).fold(f64::INFINITY, f64::min)
}

/// The point outside a ring's edge whose outward normal is nearest `heading`:
/// `out` metres off the edge's midpoint, on the ground.
fn door_outside(ring: &[[f64; 2]], heading: f64, out: f64) -> [f64; 2] {
    let c = ring_centroid(ring);
    let want = dir_of(heading);
    let n = ring.len();
    let mut best = (f64::MIN, [c[0], c[1]]);
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        let m = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        let (ex, ez) = (b[0] - a[0], b[1] - a[1]);
        let l = ex.hypot(ez);
        if l < 1.0 {
            continue;
        }
        let mut nrm = [ez / l, -ex / l];
        if nrm[0] * (m[0] - c[0]) + nrm[1] * (m[1] - c[1]) < 0.0 {
            nrm = [-nrm[0], -nrm[1]];
        }
        let score = nrm[0] * want[0] + nrm[1] * want[1];
        if score > best.0 {
            best = (score, [m[0] + nrm[0] * out, m[1] + nrm[1] * out]);
        }
    }
    best.1
}

// ---------------------------------------------------------------------------
// Coit Tower: the gallery in the belvedere's arcade.
// ---------------------------------------------------------------------------

/// The belvedere's wall, outer face to inner face (INFERRED: the outer of the
/// three nested concrete cylinders).
const COIT_WALL_M: f64 = 0.45;
/// The middle cylinder (the stair shaft round the elevator shaft), radius (INFERRED).
const COIT_CORE_R_M: f64 = 2.5;
/// The gallery floor slab, metres.
const SLAB_M: f64 = 0.3;

fn coit(js: &LandmarksJs, height_at: &dyn Fn(f64, f64) -> Option<f64>) -> Deck {
    let id = "coit-tower";
    let site = &js.buildings[id];
    let d = &site["dimensions"];
    let (g, shift) = site_ground(js, id, height_at);
    let ring = pairs(&flat(&site["record"]["points"]));
    let [cx, cz] = v2(&site["centre"]);
    // towers.rs `coit`: the belvedere's band and its pointed arches, as the tower draws them.
    let y0 = ring.iter().map(|p| g.at(p[0], p[1])).fold(f64::NEG_INFINITY, f64::max);
    let r0 = num(d, "shaftRadiusM");
    let r1 = r0 - num(d, "taperDiameterM") / 2.0;
    let bel0 = y0 + num(d, "belvedereFromM");
    let bel1 = y0 + num(d, "belvedereToM");
    let arches = num(d, "arches") as usize;
    let concrete = hexc(d, "concreteSrgb");
    let outer_c = shade(concrete, 0.82);
    let inner_c = shade(concrete, 0.9);
    let reveal_c = shade(concrete, 0.78);
    let (sill, spring, crown) = (bel0 + 0.9, bel1 - 1.8, bel1 - 0.6);
    // The floor a guard height under the drawn sills (CBC 1015.3).
    let floor = sill - GUARD_M;
    let r_in = r1 - COIT_WALL_M;
    let r_core = COIT_CORE_R_M;
    let mut b = B::new("coit-gallery");
    let p = |a: f64, r: f64, y: f64| -> V3 { [cx + a.cos() * r, y, cz - a.sin() * r] };
    {
        let m = b.m("coit-concrete");
        for k in 0..arches {
            let n = arches as f64;
            let a0 = (k as f64 + 0.18) / n * TAU;
            let a1 = (k as f64 + 0.82) / n * TAU;
            let am = (a0 + a1) / 2.0;
            let a2 = (k as f64 + 1.18) / n * TAU;
            let ap = (a1 + a2) / 2.0;
            for (r, lo, c) in [(r1, bel0, outer_c), (r_in, floor - SLAB_M, inner_c)] {
                // the pier to the next arch, in two facets.
                for (u, w) in [(a1, ap), (ap, a2)] {
                    m.quad(p(u, r, lo), p(w, r, lo), p(w, r, bel1), p(u, r, bel1), c);
                }
                // under the sill and over the arch.
                for (u, w) in [(a0, am), (am, a1)] {
                    m.quad(p(u, r, lo), p(w, r, lo), p(w, r, sill), p(u, r, sill), c);
                }
                m.quad(p(a0, r, spring), p(am, r, crown), p(am, r, bel1), p(a0, r, bel1), c);
                m.quad(p(am, r, crown), p(a1, r, spring), p(a1, r, bel1), p(am, r, bel1), c);
            }
            // the reveals: jambs, the sill's top, the pointed soffit.
            for a in [a0, a1] {
                m.quad(p(a, r1, sill), p(a, r_in, sill), p(a, r_in, spring), p(a, r1, spring), reveal_c);
            }
            for (u, w) in [(a0, am), (am, a1)] {
                m.quad(p(u, r1, sill), p(w, r1, sill), p(w, r_in, sill), p(u, r_in, sill), outer_c);
            }
            m.quad(p(a0, r1, spring), p(am, r1, crown), p(am, r_in, crown), p(a0, r_in, spring), reveal_c);
            m.quad(p(am, r1, crown), p(a1, r1, spring), p(a1, r_in, spring), p(am, r_in, crown), reveal_c);
        }
        // The gallery's floor, its ceiling under the skylit cornice, and the stair shaft.
        m.lathe(cx, cz, &[[r_in, floor], [r_core, floor]], 48, shade(concrete, 0.62), None, None);
        m.lathe(cx, cz, &[[r_core, floor - SLAB_M], [r_in, floor - SLAB_M]], 48, inner_c, None, None);
        m.lathe(cx, cz, &[[r_in, bel1], [r_core, bel1]], 48, shade(concrete, 0.95), None, None);
        m.lathe(cx, cz, &[[r_core, floor], [r_core, bel1]], 32, shade(concrete, 0.86), None, None);
    }
    // The lift door in the stair shaft, facing west (the entrance side).
    {
        let m = b.m("doors");
        let half = 0.45 / r_core;
        m.quad(p(PI - half, r_core + 0.02, floor), p(PI + half, r_core + 0.02, floor), p(PI + half, r_core + 0.02, floor + 2.1), p(PI - half, r_core + 0.02, floor + 2.1), srgb_hex(0x6b5a3e));
    }
    for k in 0..6 {
        let a = (k as f64 + 0.5) / 6.0 * TAU;
        b.lamp(p(a, (r_core + r_in) / 2.0, bel1 - 0.1), 600.0, 9.0, PI / 3.0);
    }
    b.walk_annulus(cx, cz, r_core, r_in, floor, 48);
    // The arcade is open: an invisible guard stands in the arches' plane over the sills.
    b.wall_cylinder(cx, cz, r_in, floor - 0.2, bel1, 48);
    b.wall_cylinder(cx, cz, r_core, floor - 0.2, bel1, 24);
    let view_h = 160.0;
    let vd = dir_of(view_h);
    // In the arch nearest the heading (lathe angles: x = cos a, z = -sin a), looking along the heading.
    let a_h = (-vd[1]).atan2(vd[0]).rem_euclid(TAU);
    let a_arch = (0..arches).map(|k| (k as f64 + 0.5) / arches as f64 * TAU).min_by(|p, q| {
        let dp = (p - a_h).sin().atan2((p - a_h).cos()).abs();
        let dq = (q - a_h).sin().atan2((q - a_h).cos()).abs();
        dp.total_cmp(&dq)
    }).unwrap_or(a_h);
    let view = Stop { feet: p(a_arch, r_in - 0.75, floor), heading_deg: view_h };
    // The deck shot: from across the ring behind the person, looking out past them.
    let side = dir_of(view_h + 75.0);
    let cam_eye = [cx + side[0] * (r_core + 1.0) - vd[0] * 0.6, floor + EYE_M + 0.1, cz + side[1] * (r_core + 1.0) - vd[1] * 0.6];
    let cam_at = [view.feet[0] + vd[0] * 3.0, floor + 1.2, view.feet[2] + vd[1] * 3.0];
    // The entrance: the base's west side (the parking loop and the Columbus statue face it).
    let door = door_outside(&ring, 270.0, 1.4);
    let lifts = vec![Lift {
        id: "coit-tower".into(),
        name: "Coit Tower gallery".into(),
        bottom: Stop { feet: [door[0], g.at(door[0], door[1]), door[1]], heading_deg: 270.0 },
        top: Stop { feet: [cx - (r_core + 0.8), floor, cz], heading_deg: view_h },
    }];
    let cuts = vec![Cut { centre: [cx, cz], radius_m: 12.0, y0: floor - 0.8, y1: bel1 + 0.5 }];
    b.finish("Coit Tower gallery", id, lifts, cuts, view, -5.0, (cam_eye, cam_at), floor, shift)
}

// ---------------------------------------------------------------------------
// Salesforce Tower: the top floor's observation level.
// ---------------------------------------------------------------------------

/// The curtain wall's mullion pitch (5 ft units, INFERRED) and section, metres.
const SF_MULLION_PITCH_M: f64 = 1.524;
/// The central core's half width at the top floor (INFERRED: about a quarter of the plate).
const SF_CORE_HALF_M: f64 = 9.0;

fn salesforce(js: &LandmarksJs, height_at: &dyn Fn(f64, f64) -> Option<f64>) -> Deck {
    let id = "salesforce-tower";
    let site = &js.buildings[id];
    let d = &site["dimensions"];
    let (g, shift) = site_ground(js, id, height_at);
    let ring = pairs(&flat(&site["record"]["points"]));
    let [cx, cz] = ring_centroid(&ring);
    let base = base_y(&ring, &g);
    let (h_total, top_floor, floors) = (num(d, "heightM"), num(d, "topFloorM"), num(d, "floors"));
    let (k_top_floor, k_top) = (num(d, "scaleAtTopFloor"), num(d, "scaleAtTop"));
    let scale_at = |y: f64| {
        if y <= top_floor { 1.0 + (k_top_floor - 1.0) * (y / top_floor) } else { k_top_floor + (k_top - k_top_floor) * ((y - top_floor) / (h_total - top_floor)) }
    };
    let at = |y: f64| scale_ring(&ring, cx, cz, scale_at(y));
    // towers.rs `salesforce`: the top floor's vision glass from y_lo to y_hi, the spandrel above.
    let storey = top_floor / floors;
    let y_lo = (floors - 1.0) * storey;
    let y_hi = y_lo + storey - 0.6;
    let (floor, ceil) = (base + y_lo, base + y_hi);
    let metal = hexc(d, "spandrelSrgb");
    let mut b = B::new("salesforce-ohana");
    let r_mean = ring.iter().map(|p| (p[0] - cx).hypot(p[1] - cz)).sum::<f64>() / ring.len() as f64;
    let glass_lo = at(y_lo);
    let glass_hi = at(y_hi);
    let plate = scale_ring(&ring, cx, cz, scale_at(y_lo) * (1.0 - 0.12 / r_mean));
    // The core, square to the plate's longest edge.
    let (mut yaw, mut longest) = (0.0, 0.0);
    for i in 0..ring.len() {
        let (a, q) = (ring[i], ring[(i + 1) % ring.len()]);
        let l = (q[0] - a[0]).hypot(q[1] - a[1]);
        if l > longest {
            longest = l;
            yaw = (q[1] - a[1]).atan2(q[0] - a[0]);
        }
    }
    let (cy, sy) = (yaw.cos(), yaw.sin());
    let core_pt = |a: f64, c: f64| [cx + a * cy - c * sy, cz + a * sy + c * cy];
    let h = SF_CORE_HALF_M;
    let core: Vec<[f64; 2]> = [[-h, -h], [h, -h], [h, h], [-h, h]].iter().map(|q| core_pt(q[0], q[1])).collect();
    let view_h = 300.0;
    let mut mullions: Vec<([f64; 2], [f64; 2])> = vec![];
    {
        let floor_c = srgb_hex(0xb8b3a9);
        slab(b.m("floor"), &plate, floor + 0.02, 0.35, floor_c, shade(metal, 0.55));
        // The ceiling (its underside is what is seen) and the core's walls.
        let ceil_ring = scale_ring(&ring, cx, cz, scale_at(y_hi) * (1.0 - 0.12 / r_mean));
        slab(b.m("interior"), &ceil_ring, ceil + 0.3, 0.3, srgb_hex(0xe4e2dc), srgb_hex(0xd8d6d0));
        b.m("interior").prism(&core, floor, ceil, srgb_hex(0xd8d6d0), false, false);
        // The curtain wall's mullions, a sill and a head on the glass line.
        let m = b.m("sf-metal");
        let n = ring.len();
        mullions.clear();
        for i in 0..n {
            let (a0, a1, b0, b1) = (glass_lo[i], glass_lo[(i + 1) % n], glass_hi[i], glass_hi[(i + 1) % n]);
            let len = (a1[0] - a0[0]).hypot(a1[1] - a0[1]);
            let cnt = (len / SF_MULLION_PITCH_M).ceil().max(1.0) as usize;
            for k in 0..cnt {
                let t = k as f64 / cnt as f64;
                let lo = [a0[0] + (a1[0] - a0[0]) * t, floor, a0[1] + (a1[1] - a0[1]) * t];
                let hi = [b0[0] + (b1[0] - b0[0]) * t, ceil, b0[1] + (b1[1] - b0[1]) * t];
                m.beam(lo, hi, 0.07, 0.07, metal);
                let l = len.max(1e-9);
                mullions.push(([lo[0], lo[2]], [(a1[0] - a0[0]) / l, (a1[1] - a0[1]) / l]));
            }
            m.beam([a0[0], floor + 0.05, a0[1]], [a1[0], floor + 0.05, a1[1]], 0.12, 0.1, metal);
            m.beam([b0[0], ceil - 0.05, b0[1]], [b1[0], ceil - 0.05, b1[1]], 0.12, 0.1, metal);
        }
        // Lift doors on the core face toward the view.
        let want = dir_of(view_h);
        let faces: [[f64; 2]; 4] = [[1.0, 0.0], [-1.0, 0.0], [0.0, 1.0], [0.0, -1.0]];
        let best = faces.iter().copied().max_by(|p, q| {
            let dp = (p[0] * cy - p[1] * sy) * want[0] + (p[0] * sy + p[1] * cy) * want[1];
            let dq = (q[0] * cy - q[1] * sy) * want[0] + (q[0] * sy + q[1] * cy) * want[1];
            dp.total_cmp(&dq)
        });
        let f = best.unwrap();
        let dm = b.m("doors");
        for k in [-3.0, 0.0, 3.0] {
            let tang = [-f[1], f[0]];
            let c0 = core_pt(f[0] * (h + 0.02) + tang[0] * (k - 0.6), f[1] * (h + 0.02) + tang[1] * (k - 0.6));
            let c1 = core_pt(f[0] * (h + 0.02) + tang[0] * (k + 0.6), f[1] * (h + 0.02) + tang[1] * (k + 0.6));
            dm.quad([c0[0], floor, c0[1]], [c1[0], floor, c1[1]], [c1[0], floor + 2.4, c1[1]], [c0[0], floor + 2.4, c0[1]], srgb_hex(0x9a9c9e));
        }
    }
    for k in 0..12 {
        let a = (k as f64 + 0.5) / 12.0 * TAU;
        let r = (h * 1.2 + r_mean * scale_at(y_lo)) / 2.0;
        b.lamp([cx + a.cos() * r, ceil - 0.05, cz - a.sin() * r], 900.0, 9.0, PI / 3.0);
    }
    b.walk_ring(&plate, floor + 0.02);
    // The glazing stops the walker; the core too.
    let glass_line = scale_ring(&ring, cx, cz, scale_at(y_lo + 1.0) * (1.0 - 0.2 / r_mean));
    b.wall_ring(&glass_line, floor - 0.2, ceil);
    b.wall_ring(&core, floor - 0.2, ceil);
    // The view: the glazing toward the heading, 0.6 m back from it.
    let vd = dir_of(view_h);
    let reach = ray_to_ring([cx, cz], vd, &glass_line).unwrap_or(r_mean * 0.7) - 0.6;
    let mut vf = [cx + vd[0] * reach, cz + vd[1] * reach];
    // Mid-bay: the nearest mullion off the line of sight by half a pitch.
    let cross = [vf[0] + vd[0] * 0.6, vf[1] + vd[1] * 0.6];
    if let Some((mp, t)) = mullions.iter().min_by(|p, q| ((p.0[0] - cross[0]).hypot(p.0[1] - cross[1])).total_cmp(&(q.0[0] - cross[0]).hypot(q.0[1] - cross[1]))) {
        let off = (cross[0] - mp[0]) * t[0] + (cross[1] - mp[1]) * t[1];
        let shift_by = (SF_MULLION_PITCH_M / 2.0 - off.abs()) * if off >= 0.0 { 1.0 } else { -1.0 };
        vf = [vf[0] + t[0] * shift_by, vf[1] + t[1] * shift_by];
    }
    let view = Stop { feet: [vf[0], floor + 0.02, vf[1]], heading_deg: view_h };
    let side = dir_of(view_h - 70.0);
    let cam_eye = [view.feet[0] - vd[0] * 5.5 + side[0] * 3.0, floor + EYE_M + 0.2, view.feet[2] - vd[1] * 5.5 + side[1] * 3.0];
    let cam_at = [view.feet[0] + vd[0] * 2.0, floor + 1.25, view.feet[2] + vd[1] * 2.0];
    // The lobby on Mission Street (the tower's north-west face).
    let door = door_outside(&ring, 315.0, 1.6);
    let top_at = [cx + vd[0] * (h * 1.42 + 1.0), cz + vd[1] * (h * 1.42 + 1.0)];
    let lifts = vec![Lift {
        id: "salesforce-tower".into(),
        name: "Salesforce Tower observation level".into(),
        bottom: Stop { feet: [door[0], g.at(door[0], door[1]), door[1]], heading_deg: 315.0 },
        top: Stop { feet: [top_at[0], floor + 0.02, top_at[1]], heading_deg: view_h },
    }];
    let cuts = vec![Cut { centre: [cx, cz], radius_m: 45.0, y0: floor - 0.8, y1: ceil + 0.5 }];
    b.finish("Salesforce Tower observation level", id, lifts, cuts, view, -10.0, (cam_eye, cam_at), floor + 0.02, shift)
}

// ---------------------------------------------------------------------------
// Transamerica Pyramid: the observation room on the top floor.
// ---------------------------------------------------------------------------

/// The precast wall, outer face to inner face (INFERRED).
const TA_WALL_M: f64 = 0.35;

fn transamerica(js: &LandmarksJs, height_at: &dyn Fn(f64, f64) -> Option<f64>) -> Deck {
    let id = "transamerica-pyramid";
    let site = &js.buildings[id];
    let d = &site["dimensions"];
    let fr = &site["frame"];
    let (g, shift) = site_ground(js, id, height_at);
    let (cx, cz, u, v, fhalf) = (num(fr, "cx"), num(fr, "cz"), v2(&fr["u"]), v2(&fr["v"]), num(fr, "half"));
    let ring = pairs(&flat(&site["record"]["points"]));
    let base = base_y(&ring, &g);
    let precast_c = hexc(d, "precastSrgb");
    let metal_c = hexc(d, "metalSrgb");
    let h_total = num(d, "heightM");
    let spire_y = h_total - num(d, "spireM");
    let half = |y: f64| (fhalf * (1.0 - y / h_total)).max(0.0);
    let at = |a: f64, b2: f64, y: f64| -> V3 { [cx + u[0] * a + v[0] * b2, base + y, cz + u[1] * a + v[1] * b2] };
    let top0 = num(d, "baseStoreysM");
    let floor_m = num(d, "floorM");
    let band = num(d, "windowBandM");
    let pitch = num(d, "windowPitchM");
    // towers.rs `transamerica`: the levels, the last one under the spire.
    let mut levels = Vec::new();
    let mut y = top0;
    while y < spire_y - 0.01 {
        levels.push(y);
        y += floor_m;
    }
    levels.push(spire_y);
    let (ya, yb) = (levels[levels.len() - 2], levels[levels.len() - 1]);
    let yw0 = ya + (yb - ya - band) / 2.0;
    let yw1 = yw0 + band;
    // The wings: the two faces whose normals run nearest east-west (the same stable sort).
    let wing_w = num(d, "wingWidthM") / 2.0;
    let wing_y1 = spire_y + 12.0;
    let wing_in = half(wing_y1) - 2.0;
    let mut b = B::new("transamerica-48");
    // Face `side` (towers.rs order: 0 b = -h, 1 a = +h, 2 b = +h, 3 a = -h) at `t` along it, `dist` out.
    let fp = |side: usize, t: f64, dist: f64, y: f64| -> V3 {
        match side {
            0 => at(t, -dist, y),
            1 => at(dist, t, y),
            2 => at(-t, dist, y),
            _ => at(-dist, -t, y),
        }
    };
    // sides 0 and 2 are the +-v faces: the wings (their normals run nearest east-west).
    let winged = |side: usize| side == 0 || side == 2;
    {
        let m = b.m("ta-precast");
        let inner_c = shade(precast_c, 0.9);
        let outer_d = |y: f64| half(y);
        let inner_d = |y: f64| half(y) - TA_WALL_M;
        for side in 0..4 {
            for (dist_of, c) in [(&outer_d as &dyn Fn(f64) -> f64, precast_c), (&inner_d as &dyn Fn(f64) -> f64, inner_c)] {
                let q = |t0: f64, t1: f64, y0: f64, y1: f64| -> [V3; 4] { [fp(side, t0 * dist_of(y0), dist_of(y0), y0), fp(side, t1 * dist_of(y0), dist_of(y0), y0), fp(side, t1 * dist_of(y1), dist_of(y1), y1), fp(side, t0 * dist_of(y1), dist_of(y1), y1)] };
                // below and above the windows, the face's whole width.
                for (y0, y1) in [(ya - 0.35, yw0), (yw1, yb)] {
                    let [a1, a2, a3, a4] = q(-1.0, 1.0, y0, y1);
                    m.quad(a1, a2, a3, a4, c);
                }
                // the piers between the windows (as the generator spaces them).
                let h0 = half(yw0) + 0.06;
                let count = ((2.0 * h0 - 2.0) / pitch).floor().max(1.0) as i64;
                let mut edges = vec![-1.0];
                for k in 0..count {
                    let s0 = -1.0 + (2.0 * k as f64 + 0.15) / count as f64;
                    let s1 = -1.0 + (2.0 * k as f64 + 1.85) / count as f64;
                    let (t0, t1) = (s0 * (h0 - 1.0) / h0, s1 * (h0 - 1.0) / h0);
                    if winged(side) && (t0 * h0).abs().min((t1 * h0).abs()) < wing_w + 0.3 {
                        continue; // behind the wing: solid
                    }
                    edges.push(t0);
                    edges.push(t1);
                }
                edges.push(1.0);
                for pair in edges.chunks_exact(2) {
                    let [a1, a2, a3, a4] = q(pair[0], pair[1], yw0, yw1);
                    m.quad(a1, a2, a3, a4, c);
                }
            }
            // the window reveals: sill, head, jambs between the faces.
            let h0 = half(yw0) + 0.06;
            let count = ((2.0 * h0 - 2.0) / pitch).floor().max(1.0) as i64;
            for k in 0..count {
                let s0 = -1.0 + (2.0 * k as f64 + 0.15) / count as f64;
                let s1 = -1.0 + (2.0 * k as f64 + 1.85) / count as f64;
                let (t0, t1) = (s0 * (h0 - 1.0) / h0, s1 * (h0 - 1.0) / h0);
                if winged(side) && (t0 * h0).abs().min((t1 * h0).abs()) < wing_w + 0.3 {
                    continue;
                }
                let o = |t: f64, y: f64| fp(side, t * half(y), half(y), y);
                let i = |t: f64, y: f64| fp(side, t * (half(y) - TA_WALL_M), half(y) - TA_WALL_M, y);
                m.quad(o(t0, yw0), o(t1, yw0), i(t1, yw0), i(t0, yw0), shade(precast_c, 0.85));
                m.quad(o(t1, yw1), o(t0, yw1), i(t0, yw1), i(t1, yw1), shade(precast_c, 0.7));
                for t in [t0, t1] {
                    m.quad(o(t, yw0), i(t, yw0), i(t, yw1), o(t, yw1), shade(precast_c, 0.78));
                }
            }
        }
    }
    // Window frames: an aluminium surround in each opening (the tower's metal).
    {
        let m = b.m("ta-metal");
        for side in 0..4 {
            let h0 = half(yw0) + 0.06;
            let count = ((2.0 * h0 - 2.0) / pitch).floor().max(1.0) as i64;
            for k in 0..count {
                let s0 = -1.0 + (2.0 * k as f64 + 0.15) / count as f64;
                let s1 = -1.0 + (2.0 * k as f64 + 1.85) / count as f64;
                let (t0, t1) = (s0 * (h0 - 1.0) / h0, s1 * (h0 - 1.0) / h0);
                if winged(side) && (t0 * h0).abs().min((t1 * h0).abs()) < wing_w + 0.3 {
                    continue;
                }
                let f = |t: f64, y: f64| fp(side, t * (half(y) - 0.12), half(y) - 0.12, y);
                m.beam(f(t0, yw0 + 0.03), f(t1, yw0 + 0.03), 0.06, 0.06, metal_c);
                m.beam(f(t0, yw1 - 0.03), f(t1, yw1 - 0.03), 0.06, 0.06, metal_c);
                m.beam(f(t0, yw0), f(t0, yw1), 0.06, 0.06, metal_c);
                m.beam(f(t1, yw0), f(t1, yw1), 0.06, 0.06, metal_c);
                let tm = (t0 + t1) / 2.0;
                m.beam(f(tm, yw0), f(tm, yw1), 0.05, 0.05, metal_c);
            }
        }
    }
    let df = half(ya) - TA_WALL_M;
    let floor = base + ya + 0.05;
    let square = |dd: f64| -> Vec<[f64; 2]> {
        [[-dd, -dd], [dd, -dd], [dd, dd], [-dd, dd]].iter().map(|q| {
            let p = at(q[0], q[1], 0.0);
            [p[0], p[2]]
        }).collect()
    };
    slab(b.m("floor"), &square(df), floor, 0.4, srgb_hex(0x7d7a74), shade(precast_c, 0.8));
    let dc = half(yb) - TA_WALL_M;
    slab(b.m("interior"), &square(dc), base + yb, 0.25, srgb_hex(0xe2dfd8), srgb_hex(0xe2dfd8));
    for (a, c) in [(-3.0, -3.0), (3.0, -3.0), (3.0, 3.0), (-3.0, 3.0)] {
        let p = at(a, c, yb - 0.3);
        b.lamp(p, 700.0, 7.0, PI / 3.0);
    }
    // The walkable room: the square less the two wings' cores (|a| < wing, |b| > wing_in).
    let w = wing_w;
    let room: Vec<[f64; 2]> = [[-df, -df], [-w, -df], [-w, -wing_in], [w, -wing_in], [w, -df], [df, -df], [df, df], [w, df], [w, wing_in], [-w, wing_in], [-w, df], [-df, df]]
        .iter()
        .map(|q| {
            let p = at(q[0], q[1], 0.0);
            [p[0], p[2]]
        })
        .collect();
    b.walk_ring(&room, floor);
    let dw = half(ya + 2.0) - TA_WALL_M - 0.05;
    b.wall_ring(&square(dw), floor - 0.2, base + yb);
    for sgn in [-1.0, 1.0] {
        let q = |a: f64, c: f64| {
            let p = at(a, c, 0.0);
            [p[0], p[2]]
        };
        b.wall(q(-w, sgn * wing_in), q(w, sgn * wing_in), floor - 0.2, base + yb);
        b.wall(q(-w, sgn * wing_in), q(-w, sgn * dw), floor - 0.2, base + yb);
        b.wall(q(w, sgn * wing_in), q(w, sgn * dw), floor - 0.2, base + yb);
    }
    // The view: north, from the north face's middle window (side 3, a = -h: the -u face).
    let view_h = heading_of(-u[0], -u[1]);
    // The middle of a window on the north face (side 3: at(-dist, -t)), the generator's spacing.
    let h0 = half(yw0) + 0.06;
    let count = ((2.0 * h0 - 2.0) / pitch).floor().max(1.0) as i64;
    let k = count / 2;
    let s_mid = -1.0 + (2.0 * k as f64 + 1.0) / count as f64;
    // a quarter window off its middle (the window's own mullion stands there).
    let s_view = s_mid + 0.25 * 1.7 / count as f64;
    let b_win = -s_view * (h0 - 1.0) / h0 * (half(ya + 1.6) - TA_WALL_M);
    let vf = at(-(dw - 0.45), b_win, ya + 0.05);
    let view = Stop { feet: vf, heading_deg: view_h };
    let cam = at(1.2, 2.6, ya + 0.05 + EYE_M + 0.15);
    let cam_at = at(-(dw + 3.0), b_win, ya + 1.3);
    // The lobby on Montgomery Street: the west (+v) face.
    let door = at(0.0, half(0.0) + 1.5, 0.0);
    let top = at(0.0, -(wing_in - 0.8), ya + 0.05);
    let lifts = vec![Lift {
        id: "transamerica-pyramid".into(),
        name: "Transamerica Pyramid observation room".into(),
        bottom: Stop { feet: [door[0], g.at(door[0], door[2]), door[2]], heading_deg: heading_of(v[0], v[1]) },
        top: Stop { feet: top, heading_deg: view_h },
    }];
    let cuts = vec![Cut { centre: [cx, cz], radius_m: 40.0, y0: floor - 0.8, y1: base + yb + 0.3 }];
    b.finish("Transamerica Pyramid observation room", id, lifts, cuts, view, -6.0, (cam, cam_at), floor, shift)
}

// ---------------------------------------------------------------------------
// Golden Gate Bridge: the east sidewalk, and the south tower's top.
// ---------------------------------------------------------------------------

/// The ramp from the ground onto the sidewalk's south end, metres at least.
const GG_RAMP_M: f64 = 6.0;
/// The tower top's catwalk between the legs: width, metres (INFERRED).
const GG_CATWALK_M: f64 = 1.2;

struct GgFrame {
    site: crate::golden_gate::Site,
    water: f64,
}

fn gg_frame(js: &LandmarksJs) -> GgFrame {
    GgFrame { site: crate::golden_gate::Site::new(&js.golden_gate), water: js.sea_level_y }
}

fn golden_gate_sidewalk(js: &LandmarksJs, height_at: &dyn Fn(f64, f64) -> Option<f64>) -> Deck {
    let gg = &js.golden_gate;
    let d = &gg.dimensions;
    let ground = ground_of(js, "golden-gate-bridge", height_at);
    let f = gg_frame(js);
    let site = &f.site;
    let plan = crate::golden_gate::plan(js, site, &|x, z| ground.at(x, z));
    let offset = |c: f64| site.centre_offset(c);
    let mut frame = crate::suspension::Frame::new(site.axis_point(0.0, 0.0), site.axis);
    frame.offset = Some(&offset);
    let (curb, half_w, curb_m, panel, sidewalk) = (num(d, "roadwayM") / 2.0, num(d, "widthM") / 2.0, num(d, "curbM"), num(d, "panelM"), num(d, "sidewalkM"));
    let zone = gg.leg_stages.iter().map(|s| s[3]).fold(f64::MIN, f64::max) / 2.0 + 1.0;
    let stage = gg.leg_stages.iter().find(|s| plan.y_tower - f.water >= s[0] && plan.y_tower - f.water < s[1]).unwrap_or(&gg.leg_stages[2]);
    let (leg_half_across, leg_half_along) = (stage[2] / 2.0, stage[3] / 2.0);
    let hp = site.half_plane;
    let leg_outer = hp + leg_half_across;
    let walk_outer = leg_outer + sidewalk;
    let towers = [0.0, plan.l];
    let side = |c: f64| plan.deck_top(c) + curb_m;
    let pt = |c: f64, s: f64| frame.P(c, s, side(c));
    let mut b = B::new("golden-gate-east-sidewalk");
    // Every chainage the walk changes at, and the panels between.
    let mut cs: Vec<f64> = crate::suspension::stations(plan.c_south, plan.c_north, panel / 2.0);
    for t in towers {
        cs.extend([t - zone, t - leg_half_along, t + leg_half_along, t + zone]);
    }
    cs.retain(|c| *c >= plan.c_south && *c <= plan.c_north);
    cs.sort_by(f64::total_cmp);
    cs.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    let in_leg = |c: f64| towers.iter().any(|t| (c - t).abs() < leg_half_along);
    let in_zone = |c: f64| towers.iter().any(|t| (c - t).abs() < zone);
    for w in cs.windows(2) {
        let (a, c1) = (w[0], w[1]);
        let m = (a + c1) / 2.0;
        let (s0, s1) = if in_leg(m) { (leg_outer, walk_outer) } else if in_zone(m) { (curb, walk_outer) } else { (curb, half_w) };
        b.walk_quad(pt(a, s0), pt(a, s1), pt(c1, s1), pt(c1, s0));
        // the traffic barrier and the outer railing.
        b.wall_over(pt(a, curb + 0.15), pt(c1, curb + 0.15), 0.4, GUARD_COLLIDE_M);
        let outer = if in_zone(m) { walk_outer - 0.15 } else { half_w - 0.15 };
        b.wall_over(pt(a, outer), pt(c1, outer), 0.4, GUARD_COLLIDE_M);
    }
    for t in towers {
        for e in [t - zone, t + zone] {
            b.wall_over(pt(e, half_w - 0.15), pt(e, walk_outer - 0.15), 0.4, GUARD_COLLIDE_M);
        }
        // The leg itself.
        let lo = side(t) - 1.0;
        let corners = [pt(t - leg_half_along, hp - leg_half_across), pt(t + leg_half_along, hp - leg_half_across), pt(t + leg_half_along, leg_outer), pt(t - leg_half_along, leg_outer)];
        let ring: Vec<[f64; 2]> = corners.iter().map(|p| [p[0], p[2]]).collect();
        b.wall_ring(&ring, lo, lo + 5.0);
    }
    // The drawn deck leaves the sidewalk open where a panel overlaps a tower
    // zone short of the walkway's plates (road_deck cuts the whole panel):
    // plates and railing close it, in the sidewalk's own colours.
    let sw_c = srgb_hex(num(d, "sidewalkSrgb") as u32);
    let orange = srgb_hex(num(d, "orangeSrgb") as u32);
    let railing = num(d, "railingM");
    let gap = |a: f64, c1: f64| towers.iter().any(|t| c1 > t - zone && a < t + zone);
    let drawn = crate::suspension::stations(plan.c_south, plan.c_north, panel);
    let rails = crate::suspension::stations(plan.c_south, plan.c_north, panel / 2.0);
    let open_runs = |st: &[f64]| -> Vec<(f64, f64)> {
        let mut out = vec![];
        for t in towers {
            // the last drawn station before the zone and the first after it.
            let before = st.windows(2).filter(|w| !gap(w[0], w[1]) && w[1] <= t).map(|w| w[1]).fold(f64::MIN, f64::max);
            let after = st.windows(2).filter(|w| !gap(w[0], w[1]) && w[0] >= t).map(|w| w[0]).fold(f64::MAX, f64::min);
            if before > f64::MIN && t - zone - before > 0.01 {
                out.push((before, t - zone));
            }
            if after < f64::MAX && after - (t + zone) > 0.01 {
                out.push((t + zone, after));
            }
        }
        out
    };
    for (a, c1) in open_runs(&drawn) {
        let m = b.m("gg-deck");
        m.quad(pt(a, curb), pt(c1, curb), pt(c1, half_w), pt(a, half_w), sw_c);
        let ya = |c: f64| plan.deck_top(c);
        m.quad(frame.P(a, curb, ya(a)), frame.P(c1, curb, ya(c1)), frame.P(c1, curb, side(c1)), frame.P(a, curb, side(a)), sw_c);
    }
    for (a, c1) in open_runs(&rails) {
        let m = b.m("gg-orange");
        let s = half_w - 0.15;
        m.beam(frame.P(a, s, side(a) + railing), frame.P(c1, s, side(c1) + railing), 0.12, 0.12, orange);
        m.beam(frame.P(a, s, side(a) + railing * 0.5), frame.P(c1, s, side(c1) + railing * 0.5), 0.08, 0.08, orange);
        m.beam(frame.P(c1, s, side(c1)), frame.P(c1, s, side(c1) + railing), 0.14, 0.14, orange);
    }
    // The south end: a ramp down to the ground where the deck meets land.
    let c0 = plan.c_south;
    let gy = |s: f64| {
        let [x, z] = frame.point(c0, s);
        ground.at(x, z)
    };
    let rise = (side(c0) - gy(curb)).abs().max((side(c0) - gy(half_w)).abs());
    let ramp = GG_RAMP_M.max(rise * 3.0);
    let foot = |s: f64| {
        let [x, z] = frame.point(c0 - ramp, s);
        [x, ground.at(x, z) - 0.05, z]
    };
    b.walk_quad(foot(curb), foot(half_w), pt(c0, half_w), pt(c0, curb));
    {
        let conc = srgb_hex(num(d, "concreteSrgb") as u32);
        let m = b.m("gg-deck");
        m.quad(foot(curb), pt(c0, curb), pt(c0, half_w), foot(half_w), conc);
    }
    // The view: the east sidewalk over the main span's middle, looking east into the bay.
    // Short of the mid-span, where the main cable is high over the walk
    // (at mid-span it passes the railing at head height).
    let mid = plan.l * 0.12;
    let vf = pt(mid, half_w - 0.6);
    let view = Stop { feet: vf, heading_deg: heading_of(site.across[0], site.across[1]) + 40.0 };
    let cam = pt(mid - 9.0, curb + 1.0);
    let cam_eye = [cam[0], cam[1] + EYE_M + 0.6, cam[2]];
    let cam_at = [vf[0], vf[1] + 1.0, vf[2]];
    b.finish("Golden Gate Bridge east sidewalk", "golden-gate-bridge", vec![], vec![], view, -2.0, (cam_eye, cam_at), vf[1], [0.0, 0.0])
}

fn golden_gate_tower(js: &LandmarksJs, height_at: &dyn Fn(f64, f64) -> Option<f64>) -> Deck {
    let gg = &js.golden_gate;
    let d = &gg.dimensions;
    let ground = ground_of(js, "golden-gate-bridge", height_at);
    let f = gg_frame(js);
    let site = &f.site;
    let plan = crate::golden_gate::plan(js, site, &|x, z| ground.at(x, z));
    let offset = |c: f64| site.centre_offset(c);
    let mut frame = crate::suspension::Frame::new(site.axis_point(0.0, 0.0), site.axis);
    frame.offset = Some(&offset);
    let (axis, across) = (site.axis, site.across);
    let top_y = f.water + num(d, "towerAboveWaterM");
    let orange = srgb_hex(num(d, "orangeSrgb") as u32);
    let top_stage = gg.leg_stages.last().unwrap();
    let (lx, la) = (top_stage[2] / 2.0, top_stage[3] / 2.0);
    let legs = &gg.tower_legs["south"];
    let (e, w) = (legs.east, legs.west);
    let at = |p: [f64; 2], a: f64, s: f64, y: f64| -> V3 { [p[0] + axis[0] * a + across[0] * s, y, p[1] + axis[1] * a + across[1] * s] };
    let mut b = B::new("golden-gate-south-tower");
    let inset = 0.15;
    // Each leg top: a grating deck flush with the cap, a guardrail round it
    // (open where the catwalk meets the inner face).
    let cw = GG_CATWALK_M / 2.0;
    for (p, inward) in [(e, -1.0), (w, 1.0)] {
        let deck: Vec<[f64; 2]> = [[-la, -lx], [la, -lx], [la, lx], [-la, lx]].iter().map(|q| {
            let v = at(p, q[0], q[1], 0.0);
            [v[0], v[2]]
        }).collect();
        slab(b.m("grating"), &deck, top_y + 0.04, 0.04, srgb_hex(0x8d8f8f), orange);
        b.walk_ring(&deck, top_y + 0.04);
        let (a0, a1, s0, s1) = (-la + inset, la - inset, -lx + inset, lx - inset);
        let y = top_y + 0.04;
        let si = if inward < 0.0 { s0 } else { s1 }; // the inner face's side
        let so = -si;
        let run = [at(p, -cw, si, y), at(p, a0, si, y), at(p, a0, so, y), at(p, a1, so, y), at(p, a1, si, y), at(p, cw, si, y)];
        b.guardrail_ring("gg-orange", &run, orange, true, false);
    }
    // The catwalk between the legs' inner faces, on posts down to the top strut.
    let y = top_y + 0.04;
    let ei = at(e, 0.0, -lx, y);
    let wi = at(w, 0.0, lx, y);
    let span_dir = [wi[0] - ei[0], wi[2] - ei[2]];
    let cat: Vec<[f64; 2]> = [at(e, -cw, -lx, 0.0), at(e, cw, -lx, 0.0), at(w, cw, lx, 0.0), at(w, -cw, lx, 0.0)].iter().map(|v| [v[0], v[2]]).collect();
    slab(b.m("grating"), &cat, y, 0.12, srgb_hex(0x8d8f8f), orange);
    b.walk_ring(&cat, y);
    for s in [-cw + 0.05, cw - 0.05] {
        b.guardrail("gg-orange", at(e, s, -lx, y), at(w, s, lx, y), orange, true);
    }
    let strut_top = f.water + gg.struts.last().map(|s| s[1]).unwrap_or(223.0);
    for t in [1.0 / 3.0, 2.0 / 3.0] {
        let px = ei[0] + span_dir[0] * t;
        let pz = ei[2] + span_dir[1] * t;
        b.m("gg-orange").beam([px, strut_top, pz], [px, y - 0.12, pz], 0.3, 0.3, shade(orange, 0.62));
    }
    // A saddle housing over each main cable's saddle (INFERRED: a hood 2 m long).
    for p in [e, w] {
        b.m("gg-orange").box_(p[0], p[1], top_y, top_y + 1.1, 1.0, 0.6, axis[1].atan2(axis[0]), shade(orange, 0.85), ALL);
        let ring: Vec<[f64; 2]> = [[-1.05, -0.65], [1.05, -0.65], [1.05, 0.65], [-1.05, 0.65]].iter().map(|q| {
            let v = at(p, q[0], q[1], 0.0);
            [v[0], v[2]]
        }).collect();
        b.wall_ring(&ring, top_y - 0.2, top_y + 1.1);
    }
    let view_h = 120.0;
    // On the east leg, its south-east corner (the axis runs north).
    let vf = at(e, -la + 0.65, lx - 0.6, y);
    let view = Stop { feet: vf, heading_deg: view_h };
    let cam_eye = at(e, 4.5, -9.0, y + 4.0);
    let cam_at = [vf[0], vf[1] + 1.0, vf[2]];
    // The service elevator: from the east sidewalk's tower walkway, at the leg's outer face.
    let stage = gg.leg_stages.iter().find(|s| plan.y_tower - f.water >= s[0] && plan.y_tower - f.water < s[1]).unwrap_or(&gg.leg_stages[2]);
    let leg_outer = site.half_plane + stage[2] / 2.0;
    let walk_outer = leg_outer + num(d, "sidewalkM");
    let bottom = frame.P(0.0, (leg_outer + walk_outer - 0.15) / 2.0, plan.deck_top(0.0) + num(d, "curbM"));
    let lifts = vec![Lift {
        id: "golden-gate-south-tower".into(),
        name: "Golden Gate south tower top".into(),
        bottom: Stop { feet: bottom, heading_deg: heading_of(across[0], across[1]) },
        top: Stop { feet: at(e, -2.2, 0.0, y), heading_deg: view_h },
    }];
    b.finish("Golden Gate Bridge south tower top", "golden-gate-bridge", lifts, vec![], view, -10.0, (cam_eye, cam_at), y, [0.0, 0.0])
}

// ---------------------------------------------------------------------------
// Bay Bridge west span: the W2 tower's top.
// ---------------------------------------------------------------------------

fn bay_bridge_tower(js: &LandmarksJs, height_at: &dyn Fn(f64, f64) -> Option<f64>) -> Deck {
    let bb = &js.bay_bridge_west;
    let d = &bb.dimensions;
    let ground = ground_of(js, "bay-bridge-west", height_at);
    let plan = crate::bay_bridge::plan(js, &|x, z| ground.at(x, z));
    let f = crate::bay_bridge::frame(bb);
    let h = num(d, "halfPlaneM");
    let water = js.sea_level_y;
    let top = water + bb.tower_top_above_water_m;
    let tc = plan.towers[0].2;
    let steel = srgb_hex(num(d, "steelSrgb") as u32);
    let mut b = B::new("bay-bridge-w2");
    // bay_bridge.rs `towers`: the top tier's legs (5 m along, 4 m across) at
    // +-h and the top strut (4 m along, 7 m deep) between them, flush on top.
    let (la, lx, ba) = (2.5, 2.0, 2.0);
    let outline: Vec<[f64; 2]> = [[-la, -h - lx], [la, -h - lx], [la, -h + lx], [ba, -h + lx], [ba, h - lx], [la, h - lx], [la, h + lx], [-la, h + lx], [-la, h - lx], [-ba, h - lx], [-ba, -h + lx], [-la, -h + lx]]
        .iter()
        .map(|q| f.point(tc + q[0], q[1]))
        .collect();
    let y = top + 0.04;
    slab(b.m("grating"), &outline, y, 0.04, srgb_hex(0x8a8c8c), steel);
    b.walk_ring(&outline, y);
    let rail: Vec<V3> = inset_ring(&outline, 0.15).iter().map(|p| [p[0], y, p[1]]).collect();
    b.guardrail_ring("bb-steel", &rail, shade(steel, 1.25), true, true);
    // An aviation beacon mast on each leg (the bridge lights put the lamp there).
    let view_h = 264.0;
    let vf2 = f.point(tc - ba + 0.55, 0.0);
    let vf = [vf2[0], y, vf2[1]];
    let view = Stop { feet: vf, heading_deg: view_h };
    let ce = f.point(tc + 9.0, h + 6.0);
    let cam_eye = [ce[0], y + 7.0, ce[1]];
    let cam_at = [vf[0], vf[1] + 1.0, vf[2]];
    // The maintenance entrance: the first dry ground under the span toward San Francisco.
    let mut c_land = tc - 40.0;
    while c_land > tc - 900.0 {
        let p = f.point(c_land, 0.0);
        if ground.at(p[0], p[1]) > water + 1.5 {
            break;
        }
        c_land -= 4.0;
    }
    let bp = f.point(c_land - 6.0, 0.0);
    let topf = f.point(tc, h - lx - 1.2);
    let lifts = vec![Lift {
        id: "bay-bridge-w2".into(),
        name: "Bay Bridge W2 tower top".into(),
        bottom: Stop { feet: [bp[0], ground.at(bp[0], bp[1]), bp[1]], heading_deg: heading_of(f.axis[0], f.axis[1]) },
        top: Stop { feet: [topf[0], y, topf[1]], heading_deg: view_h },
    }];
    b.finish("Bay Bridge W2 tower top", "bay-bridge-west", lifts, vec![], view, -8.0, (cam_eye, cam_at), y, [0.0, 0.0])
}

// ---------------------------------------------------------------------------
// Sutro Tower: the upper platform.
// ---------------------------------------------------------------------------

/// The platform's corners are cut back from each leg point for the mast (INFERRED).
const SUTRO_CORNER_CUT_M: f64 = 2.4;
/// The catwalk's width round the platform (INFERRED).
const SUTRO_CATWALK_M: f64 = 1.8;

fn sutro(js: &LandmarksJs, height_at: &dyn Fn(f64, f64) -> Option<f64>) -> Deck {
    let id = "sutro-tower";
    let site = &js.buildings[id];
    let d = &site["dimensions"];
    let (g, shift) = site_ground(js, id, height_at);
    let c = [num(&site["centre"], "x"), num(&site["centre"], "z")];
    let dirs: Vec<[f64; 2]> = site["legDirections"].as_array().map(|a| a.iter().map(v2).collect()).unwrap_or_default();
    let sqrt3 = 3f64.sqrt();
    let (rb, rt) = (num(d, "baseSideM") / sqrt3, num(d, "topSideM") / sqrt3);
    let platform = num(d, "platformM");
    let white = hexc(d, "whiteSrgb");
    // towers.rs `sutro`: each leg's foot on the ground, the base their mean.
    let feet: Vec<([f64; 2], f64)> = dirs
        .iter()
        .map(|[ux, uz]| {
            let (x, z) = (c[0] + ux * rb, c[1] + uz * rb);
            let gr = g.at(x, z);
            ([x, z], if gr.is_nan() { 0.0 } else { gr })
        })
        .collect();
    let base_y = feet.iter().map(|f| f.1).sum::<f64>() / feet.len() as f64;
    // the platform ring frame's beams are 2.4 m deep about the platform: the deck lies on them.
    let y = base_y + platform + 1.2 + 0.05;
    let corner: Vec<[f64; 2]> = dirs.iter().map(|u| [c[0] + u[0] * rt, c[1] + u[1] * rt]).collect();
    let mut hex: Vec<[f64; 2]> = vec![];
    for k in 0..3 {
        let (p, q) = (corner[k], corner[(k + 1) % 3]);
        let l = (q[0] - p[0]).hypot(q[1] - p[1]);
        let e = [(q[0] - p[0]) / l, (q[1] - p[1]) / l];
        hex.push([p[0] + e[0] * SUTRO_CORNER_CUT_M, p[1] + e[1] * SUTRO_CORNER_CUT_M]);
        hex.push([q[0] - e[0] * SUTRO_CORNER_CUT_M, q[1] - e[1] * SUTRO_CORNER_CUT_M]);
    }
    let mut b = B::new("sutro-platform");
    // A grating catwalk round the platform on its ring frame, open in the
    // middle ("a catwalk and an open-air span": sutrotower.com), the masts
    // standing clear of its cut corners.
    let inner = inset_ring(&hex, SUTRO_CATWALK_M);
    {
        let m = b.m("grating");
        let (top_c, edge_c) = (srgb_hex(0x8f9192), shade(white, 0.8));
        for t in triangulate_with_hole(&hex, &inner) {
            m.tri([t[0][0], y, t[0][1]], [t[1][0], y, t[1][1]], [t[2][0], y, t[2][1]], top_c);
            m.tri([t[2][0], y - 0.08, t[2][1]], [t[1][0], y - 0.08, t[1][1]], [t[0][0], y - 0.08, t[0][1]], edge_c);
        }
        for ring in [&hex, &inner] {
            for i in 0..ring.len() {
                let (a, q) = (ring[i], ring[(i + 1) % ring.len()]);
                m.quad([a[0], y - 0.08, a[1]], [q[0], y - 0.08, q[1]], [q[0], y, q[1]], [a[0], y, a[1]], edge_c);
            }
        }
        // Bearing bars across the walk, every 0.3 m (INFERRED: a common grating pitch is 1 3/16 in, drawn coarser).
        let dark = srgb_hex(0x5f6163);
        for i in 0..hex.len() {
            let (a, q) = (hex[i], hex[(i + 1) % hex.len()]);
            let (ai, qi) = (inner[i], inner[(i + 1) % inner.len()]);
            let len = (q[0] - a[0]).hypot(q[1] - a[1]);
            let n = (len / 0.3).floor() as usize;
            for k in 1..n {
                let t = k as f64 / n as f64;
                let o = [a[0] + (q[0] - a[0]) * t, a[1] + (q[1] - a[1]) * t];
                let ii = [ai[0] + (qi[0] - ai[0]) * t, ai[1] + (qi[1] - ai[1]) * t];
                m.beam([o[0], y + 0.005, o[1]], [ii[0], y + 0.005, ii[1]], 0.03, 0.01, dark);
            }
        }
    }
    for t in triangulate_with_hole(&hex, &inner) {
        b.walk.push(t.map(|p| [p[0], y, p[1]]));
    }
    let rail: Vec<V3> = inset_ring(&hex, 0.3).iter().map(|p| [p[0], y, p[1]]).collect();
    b.guardrail_ring("sutro-steel", &rail, white, true, true);
    let rail_in: Vec<V3> = inset_ring(&hex, SUTRO_CATWALK_M - 0.25).iter().map(|p| [p[0], y, p[1]]).collect();
    b.guardrail_ring("sutro-steel", &rail_in, white, true, true);
    // The view: the east edge (legs 1 and 2) toward downtown.
    let m12 = [(corner[1][0] + corner[2][0]) / 2.0, (corner[1][1] + corner[2][1]) / 2.0];
    let inward = [c[0] - m12[0], c[1] - m12[1]];
    let il = inward[0].hypot(inward[1]);
    let vf = [m12[0] + inward[0] / il * 0.85, y, m12[1] + inward[1] / il * 0.85];
    let view = Stop { feet: vf, heading_deg: 60.0 };
    let tang = [-inward[1] / il, inward[0] / il];
    let cam_eye = [m12[0] - inward[0] / il * 10.0 + tang[0] * 4.0, y + 4.0, m12[1] - inward[1] / il * 10.0 + tang[1] * 4.0];
    let cam_at = [vf[0], vf[1] + 1.0, vf[2]];
    // The elevator in the west leg (leg 0): its door beside the leg's footing.
    let u0 = dirs[0];
    let foot = [feet[0].0[0] + u0[0] * 5.0, feet[0].0[1] + u0[1] * 5.0];
    // On the catwalk inside the corner's cut (the cut line stands cut x cos 30 deg from the corner).
    let at_cut = SUTRO_CORNER_CUT_M * (PI / 6.0).cos() + 0.9;
    let topf = [corner[0][0] - u0[0] * at_cut, y, corner[0][1] - u0[1] * at_cut];
    let lifts = vec![Lift {
        id: "sutro-tower".into(),
        name: "Sutro Tower upper platform".into(),
        bottom: Stop { feet: [foot[0], g.at(foot[0], foot[1]), foot[1]], heading_deg: heading_of(u0[0], u0[1]) },
        top: Stop { feet: topf, heading_deg: 60.0 },
    }];
    b.finish("Sutro Tower upper platform", id, lifts, vec![], view, -12.0, (cam_eye, cam_at), y, shift)
}

// ---------------------------------------------------------------------------
// City Hall: the lantern gallery over the dome.
// ---------------------------------------------------------------------------

/// The gallery's reach past the lantern's columns (INFERRED).
const CH_GALLERY_OUT_M: f64 = 1.6;

fn city_hall(js: &LandmarksJs, height_at: &dyn Fn(f64, f64) -> Option<f64>) -> Deck {
    let id = "city-hall";
    let site = &js.buildings[id];
    let d = &site["dimensions"];
    let fr = &site["frame"];
    let (g, shift) = site_ground(js, id, height_at);
    let (cx, cz, v, hv) = (num(fr, "cx"), num(fr, "cz"), v2(&fr["v"]), num(fr, "hv"));
    let ring = pairs(&flat(&site["record"]["points"]));
    let base = base_y(&ring, &g);
    let y = |h: f64| base + h;
    // towers.rs `city_hall`: the lantern stands on the dome's crown at l0.
    let l0 = num(d, "drumTopM") + num(d, "entablatureM") + num(d, "domeRiseM");
    let lr = num(d, "lanternRadiusM");
    let lantern_top = num(d, "lanternTopM");
    let stone = hexc(d, "stoneSrgb");
    let floor = y(l0) + 0.06;
    let (r_in, r_out) = (lr - 1.0, lr + CH_GALLERY_OUT_M);
    let r_rail = r_out - 0.18;
    let mut b = B::new("city-hall-lantern");
    {
        let m = b.m("ch-stone");
        m.lathe(cx, cz, &[[r_out, floor], [r_in, floor]], 48, shade(stone, 0.92), None, None);
        m.lathe(cx, cz, &[[r_in, floor - 0.4], [r_out, floor - 0.4], [r_out, floor]], 48, shade(stone, 0.8), None, None);
        // The balustrade: plinth, balusters every 0.32 m, the rail cap.
        let cap = |m: &mut MeshArrays, y0: f64, y1: f64, w: f64, c: Rgb| {
            m.lathe(cx, cz, &[[r_rail - w, y0], [r_rail + w, y0], [r_rail + w, y1], [r_rail - w, y1], [r_rail - w, y0]], 48, c, None, None);
        };
        cap(m, floor, floor + 0.16, 0.14, shade(stone, 0.85));
        cap(m, floor + GUARD_M - 0.12, floor + GUARD_M, 0.15, stone);
        let n = (TAU * r_rail / 0.32).round() as usize;
        for k in 0..n {
            let a = k as f64 / n as f64 * TAU;
            m.box_(cx + a.cos() * r_rail, cz - a.sin() * r_rail, floor + 0.16, floor + GUARD_M - 0.12, 0.06, 0.06, -a, shade(stone, 0.95), ALL);
        }
    }
    for k in 0..4 {
        let a = k as f64 / 4.0 * TAU;
        b.lamp([cx + a.cos() * (r_in + 0.1), floor + 2.4, cz - a.sin() * (r_in + 0.1)], 400.0, 6.0, PI / 2.5);
    }
    b.walk_annulus(cx, cz, r_in, r_out, floor, 48);
    b.wall_cylinder(cx, cz, r_rail, floor - 0.2, floor + GUARD_COLLIDE_M, 48);
    b.wall_cylinder(cx, cz, r_in, floor - 0.2, y(lantern_top), 24);
    // The lantern's eight columns (towers.rs: radius 0.35 at the lantern radius).
    for k in 0..8 {
        let a = (k as f64 + 0.5) / 8.0 * TAU;
        b.wall_cylinder(cx + a.cos() * lr, cz - a.sin() * lr, 0.4, floor - 0.2, y(lantern_top), 6);
    }
    let view_h = 75.0;
    let vd = dir_of(view_h);
    let vf = [cx + vd[0] * (r_rail - 0.5), floor, cz + vd[1] * (r_rail - 0.5)];
    let view = Stop { feet: vf, heading_deg: view_h };
    let cd = dir_of(view_h + 38.0);
    let cam_eye = [cx + cd[0] * 13.0, floor + 4.0, cz + cd[1] * 13.0];
    let cam_at = [vf[0], floor + 1.0, vf[2]];
    // The entrance: the east (Polk Street) portico, toward Civic Center Plaza.
    let e_sign = if v[0] < 0.0 { -1.0 } else { 1.0 };
    let off = hv + num(d, "porticoDepthM") + 3.0;
    let door = [cx + v[0] * off * e_sign, cz + v[1] * off * e_sign];
    let lifts = vec![Lift {
        id: "city-hall".into(),
        name: "City Hall lantern gallery".into(),
        bottom: Stop { feet: [door[0], g.at(door[0], door[1]), door[1]], heading_deg: 90.0 },
        top: Stop { feet: [cx + (r_in + 1.7), floor, cz], heading_deg: 90.0 },
    }];
    let cuts = vec![Cut { centre: [cx, cz], radius_m: 30.0, y0: floor - 0.8, y1: floor + 3.5 }];
    b.finish("City Hall lantern gallery", id, lifts, cuts, view, -8.0, (cam_eye, cam_at), floor, shift)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inset_moves_a_square_inward_either_winding() {
        let sq = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let r = inset_ring(&sq, 1.0);
        assert!((r[0][0] - 1.0).abs() < 1e-9 && (r[0][1] - 1.0).abs() < 1e-9, "{r:?}");
        let mut rev = sq.clone();
        rev.reverse();
        let r = inset_ring(&rev, 1.0);
        assert!(r.iter().all(|p| p[0] > 0.5 && p[0] < 9.5 && p[1] > 0.5 && p[1] < 9.5), "{r:?}");
    }

    #[test]
    fn headings_round_trip() {
        for h in [0.0, 45.0, 90.0, 160.0, 300.0] {
            let d = dir_of(h);
            assert!((heading_of(d[0], d[1]) - h).abs() < 1e-9);
        }
        assert_eq!(dir_of(90.0).map(|v| (v * 1e9).round() / 1e9), [1.0, 0.0]);
    }
}
