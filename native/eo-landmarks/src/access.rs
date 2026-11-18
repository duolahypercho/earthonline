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
//! | Coit Tower gallery | the belvedere's arcade, a ring between the outer wall and the stair shaft, its arches glazed with acrylic panes but the view's (visitors' accounts: small Plexiglass windows, a few left open) | tower 210 ft (64 m); the observation deck 32 ft (9.8 m) below the top, an arcade with skylights above; three nested cylinders of unpainted reinforced concrete, the elevator in the inner one, the stair in the middle one | Wikipedia "Coit Tower" (citing the NRHP nomination and SF Recreation and Parks). The model's arcade is the LiDAR one of towers.rs; the floor is set so the drawn sills stand a guard height (below) over it |
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
//! The enclosed decks (Coit, Salesforce, Transamerica) are rooms: their
//! interior surfaces carry the sky they see through their openings
//! (access_sky.rs), and each deck's `Interior` lets the camera meter open up
//! inside it (eo-render interior_meter.rs).
//!
//! Not built: the Ferry Building's clock-tower balcony (the base build draws
//! the Ferry Building as a 15 m block with no tower to stand on) and Twin
//! Peaks (its summit is open ground the collision heightfield already walks).

use crate::LandmarkLight;
use crate::access_sky::{Portal, Prism, Room};
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
    /// Where a visitor's eye reaches the opening (Coit's arch: its outer
    /// face), for a view with no part of the landmark in frame; None: the view's own eye.
    pub vantage: Option<V3>,
    /// An enclosed deck's room for the camera meter (eo-render
    /// interior_meter.rs); None for an open-air deck.
    pub interior: Option<Interior>,
}

/// An enclosed deck's room as the camera meter sees it: the plan ring and
/// the height range an eye inside stands in, the openings (sky-light
/// portals) a view leaves the room by, and the interior surfaces' mean baked
/// sky visibility (access_sky.rs), the share of the open ground's light the
/// room's surfaces get.
#[derive(Clone, Debug, PartialEq)]
pub struct Interior {
    pub ring: Vec<[f64; 2]>,
    pub y0: f64,
    pub y1: f64,
    pub portals: Vec<Portal>,
    pub sky: f64,
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
    /// An interior surface: its vertices take the room's baked sky visibility (access_sky.rs).
    bake: bool,
}

struct B {
    id: &'static str,
    parts: Vec<Part>,
    walk: Vec<Tri>,
    barrier: Vec<Tri>,
    lights: Vec<LandmarkLight>,
    /// The enclosed deck's openings and cores (None: an open-air deck).
    room: Option<Room>,
    vantage: Option<V3>,
    /// The room's plan ring and height range (with `room`: the deck's `Interior`).
    bounds: Option<(Vec<[f64; 2]>, f64, f64)>,
}

/// An interior surface is cut to triangles no longer than this a side, so
/// its baked sky visibility (a vertex attribute) can fall off across a room, metres.
const BAKE_EDGE_M: f64 = 1.1;

/// The observation floors' insulating glass units: visible transmittance
/// (INFERRED: a commercial low-E double unit, 0.6 to 0.7; Guardian SunGuard
/// SuperNeutral 68 on clear: 0.68).
const GLAZING_T: f64 = 0.68;
/// The BRE split-flux formula's own glazing transmittance (clean clear
/// single glazing, 0.85: Hopkinson et al.; CIBSE LG10), divided out where an
/// opening's real transmittance differs.
const IRC_GLAZING_T: f64 = 0.85;
/// Coit's arches are glazed with acrylic panes, a few of them left open
/// (visitors' accounts: "small Plexiglass windows" that are hard to
/// photograph through; most closed, a few open): clear acrylic transmits
/// 92 % (Plexiglas G data sheet, luminous transmittance), weathered and
/// scratched, 0.85 (INFERRED).
const COIT_PANE_T: f64 = 0.85;

/// The glazing's normal-incidence opacity: a clear insulating unit's
/// visible-light reflectance plus absorption (about 0.08 + 0.06 for clear
/// low-iron glass; Pilkington Optiwhite / Guardian ClimaGuard data sheets
/// give 0.08 to 0.15 reflectance), the rest transmitted.
const GLASS_OPACITY: f64 = 0.14;

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
        // the rails and the housing in the paint alone (the scanned steel's
        // grain reads as speckle on thin members); the cap plates with it.
        "gg-orange" => reg("golden-gate-bridge", "orange").map(|m| Material { surface: None, ..m }),
        "gg-plate" => reg("golden-gate-bridge", "orange"),
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
        // Ground and sealed concrete or terrazzo (INFERRED: the observation
        // floors' finish), glossy enough to carry the window wall's
        // reflection (a honed floor's roughness about 0.25 to 0.3), its pour
        // mottle in the vertex colour (`mottled`), not the formed concrete's scan.
        "slab" => plain_material(0.28, 0.0, "facade-masonry"),
        // Brushed aluminium mullions and frames (anodised, INFERRED).
        "mullion" => plain_material(0.38, 0.85, "facade-metal"),
        // Lit diffuser panels and signs (INFERRED).
        "fixtures" => plain_material(0.6, 0.0, "facade-masonry"),
        // A glazed pane (eo-render landmarks.rs DRAW_GLASS_PANE): smooth float glass.
        "glass" => Material { draw: Some("glass-pane".into()), surface_options: Some(serde_json::json!({ "opacity": GLASS_OPACITY })), ..plain_material(0.06, 0.0, "facade-glass") },
        _ => plain_material(0.85, 0.0, "facade-masonry"),
    });
    (m, key != "glass")
}

impl B {
    fn new(id: &'static str) -> Self {
        Self { id, parts: vec![], walk: vec![], barrier: vec![], lights: vec![], room: None, vantage: None, bounds: None }
    }

    fn part(&mut self, key: &'static str, bake: bool) -> &mut MeshArrays {
        if let Some(i) = self.parts.iter().position(|p| p.key == key && p.bake == bake) {
            return &mut self.parts[i].mesh;
        }
        let (material, cast) = finish(key);
        let name = if bake { format!("access:{}:{key}:interior", self.id) } else { format!("access:{}:{key}", self.id) };
        self.parts.push(Part { key, material, cast, mesh: MeshArrays::new(&name), bake });
        &mut self.parts.last_mut().unwrap().mesh
    }

    /// An outside (open-sky) surface in finish `key`.
    fn m(&mut self, key: &'static str) -> &mut MeshArrays {
        self.part(key, false)
    }

    /// An interior surface in finish `key`: it takes the room's baked sky visibility.
    fn mi(&mut self, key: &'static str) -> &mut MeshArrays {
        self.part(key, true)
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

    /// A warm-white lamp (3,000 K) at `pos`: `candela`, radiating as a point
    /// (the gather's point kind: a pendant or a wall lamp, INFERRED; a street
    /// lamp's cone is the street lamps' own photometry, which these are not).
    fn lamp(&mut self, pos: V3, candela: f64, range: f64) {
        let base = [pos[0], pos[1] - 3.0, pos[2]];
        self.lights.push(crate::lights::light(pos, base, crate::lights::unit_luminance(crate::lights::WARM_WHITE_SRGB), candela, range, PI / 2.0, 0.0));
    }

    /// The deck, moved by `shift` (a landmark building's plan shift).
    #[allow(clippy::too_many_arguments)]
    fn finish(self, name: &str, landmark: &str, lifts: Vec<Lift>, cuts: Vec<Cut>, view: Stop, view_pitch_deg: f64, deck_camera: (V3, V3), floor_y: f64, shift: [f64; 2]) -> Deck {
        let mv = |p: V3| [p[0] + shift[0], p[1], p[2] + shift[1]];
        let mut structures = Vec::new();
        let room = self.room.as_ref();
        let (mut sky_sum, mut sky_n) = (0.0f64, 0usize);
        for p in self.parts {
            let mut mesh = if p.bake { subdivided(&p.mesh, BAKE_EDGE_M).freeze() } else { p.mesh.freeze() };
            if p.key == "slab" {
                mottled(&mut mesh);
            }
            if let (true, Some(room)) = (p.bake, room) {
                let sky = room.bake(&mesh.positions, &mesh.normals);
                sky_sum += sky.iter().map(|v| *v as f64).sum::<f64>();
                sky_n += sky.len();
                mesh.sky = Some(sky);
            }
            for v in mesh.positions.chunks_exact_mut(3) {
                v[0] += shift[0] as f32;
                v[2] += shift[1] as f32;
            }
            if mesh.is_empty() {
                continue;
            }
            structures.push(Structure { name: mesh.name.clone(), cast: p.cast, footway: false, kit: None, material: p.material, mesh });
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
            vantage: self.vantage.map(mv),
            interior: match (&self.room, self.bounds) {
                (Some(room), Some((ring, y0, y1))) if sky_n > 0 => Some(Interior {
                    ring: ring.iter().map(|q| [q[0] + shift[0], q[1] + shift[1]]).collect(),
                    y0,
                    y1,
                    portals: room.portals.iter().map(|q| q.map(mv)).collect(),
                    sky: sky_sum / sky_n as f64,
                }),
                _ => None,
            },
        }
    }
}

/// A polished floor's pour mottle: each vertex's colour times a tone of
/// 0.94 to 1.04 hashed from its 0.9 m plan cell (INFERRED: the aggregate and
/// trowel variation of a ground concrete floor), interpolated across the
/// subdivided slab; deterministic.
fn mottled(mesh: &mut crate::geometry::Frozen) {
    for (p, c) in mesh.positions.chunks_exact(3).zip(mesh.colors.chunks_exact_mut(3)) {
        let (i, k) = ((p[0] as f64 / 0.9).floor() as i64, (p[2] as f64 / 0.9).floor() as i64);
        let tone = lift_tone(i.wrapping_mul(73_856_093) ^ k.wrapping_mul(19_349_663), 11) as f32;
        for v in c.iter_mut() {
            *v *= tone;
        }
    }
}

/// A mesh with every triangle cut (longest side first) until no side is
/// longer than `max_edge`: positions, normals and colours interpolated.
fn subdivided(m: &MeshArrays, max_edge: f64) -> MeshArrays {
    let mut out = MeshArrays::new(&m.name);
    type Vx = ([f64; 3], [f64; 3], [f64; 3]);
    let get = |i: u32| -> Vx {
        let i = i as usize * 3;
        ([m.positions[i], m.positions[i + 1], m.positions[i + 2]], [m.normals[i], m.normals[i + 1], m.normals[i + 2]], [m.colors[i], m.colors[i + 1], m.colors[i + 2]])
    };
    let mid = |a: &Vx, b: &Vx| -> Vx {
        let h = |p: [f64; 3], q: [f64; 3]| [(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0, (p[2] + q[2]) / 2.0];
        let n = h(a.1, b.1);
        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-12);
        (h(a.0, b.0), [n[0] / l, n[1] / l, n[2] / l], h(a.2, b.2))
    };
    fn d2(a: [f64; 3], b: [f64; 3]) -> f64 {
        (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
    }
    let mut stack: Vec<[Vx; 3]> = Vec::new();
    for t in m.indices.chunks_exact(3) {
        stack.push([get(t[0]), get(t[1]), get(t[2])]);
        while let Some([a, b, c]) = stack.pop() {
            // longest-edge bisection (keeps a long thin face from shattering into slivers).
            let (eab, ebc, eca) = (d2(a.0, b.0), d2(b.0, c.0), d2(c.0, a.0));
            let e = eab.max(ebc).max(eca);
            if e > max_edge * max_edge {
                if e == eab {
                    let m = mid(&a, &b);
                    stack.push([a, m, c]);
                    stack.push([m, b, c]);
                } else if e == ebc {
                    let m = mid(&b, &c);
                    stack.push([a, b, m]);
                    stack.push([a, m, c]);
                } else {
                    let m = mid(&c, &a);
                    stack.push([a, b, m]);
                    stack.push([m, b, c]);
                }
                continue;
            }
            let base = out.vertex_count();
            for v in [a, b, c] {
                out.positions.extend_from_slice(&v.0);
                out.normals.extend_from_slice(&v.1);
                out.colors.extend_from_slice(&v.2);
                out.min_y = out.min_y.min(v.0[1]);
                out.max_y = out.max_y.max(v.0[1]);
            }
            out.indices.extend_from_slice(&[base, base + 1, base + 2]);
        }
    }
    out
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
/// The gallery's width, outer wall's inner face to the stair shaft (INFERRED:
/// visitors walk a ring round the middle cylinder, which holds the stair
/// they climb the last flights by: Wikipedia "Coit Tower", the three nested
/// cylinders; a 5 ft (1.52 m) corridor plus the guard's reach).
const COIT_GALLERY_M: f64 = 1.6;
/// The gallery floor slab, metres.
const SLAB_M: f64 = 0.3;
/// Board-formed concrete lifts (INFERRED: 2 ft boards, the tower's
/// 1933 cast-in-place shell), each a slightly different tone.
const COIT_LIFT_M: f64 = 0.61;
/// Segments of each round arch head.
const ARCH_SEGS: usize = 10;

/// A small deterministic tone step per formwork lift: 0.94 to 1.04.
fn lift_tone(k: i64, salt: u64) -> f64 {
    let mut h = (k as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ salt.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h ^= h >> 31;
    h = h.wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^= h >> 29;
    0.94 + (h % 1000) as f64 / 1000.0 * 0.1
}

/// The linear luminance of a linear colour (Rec. 709 weights): a surface's reflectance.
fn reflectance(c: Rgb) -> f64 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn coit(js: &LandmarksJs, height_at: &dyn Fn(f64, f64) -> Option<f64>) -> Deck {
    let id = "coit-tower";
    let site = &js.buildings[id];
    let d = &site["dimensions"];
    let (g, shift) = site_ground(js, id, height_at);
    let ring = pairs(&flat(&site["record"]["points"]));
    let [cx, cz] = v2(&site["centre"]);
    // towers.rs `coit`: the belvedere's band and its arches, as the tower draws them.
    let y0 = ring.iter().map(|p| g.at(p[0], p[1])).fold(f64::NEG_INFINITY, f64::max);
    let r0 = num(d, "shaftRadiusM");
    let r1 = r0 - num(d, "taperDiameterM") / 2.0;
    let bel0 = y0 + num(d, "belvedereFromM");
    let bel1 = y0 + num(d, "belvedereToM");
    let arches = num(d, "arches") as usize;
    let concrete = hexc(d, "concreteSrgb");
    let outer_c = shade(concrete, 0.82);
    // the gallery's own walls: unpainted cast concrete out of the weather's
    // bleaching, reflectance about 0.35 to 0.40 (fresh grey Portland-cement
    // concrete: LBNL Heat Island Group, "Cool Pavements"), not the
    // sun-bleached outer face's 0.6.
    let inner_c = shade(concrete, 0.6);
    let reveal_c = shade(concrete, 0.8);
    let (sill, crown) = (bel0 + 0.9, bel1 - 0.6);
    // The floor a guard height under the drawn sills (CBC 1015.3).
    let floor = sill - GUARD_M;
    let r_in = r1 - COIT_WALL_M;
    let r_core = r_in - COIT_GALLERY_M;
    let n = arches as f64;
    // Each arch spans 64 % of its bay (towers.rs); its head a semicircle on the opening's chord.
    let (o0, o1) = (0.18, 0.82);
    let half_w = r1 * ((o1 - o0) / n * PI).sin();
    let spring = crown - half_w;
    let mut b = B::new("coit-gallery");
    let p = |a: f64, r: f64, y: f64| -> V3 { [cx + a.cos() * r, y, cz - a.sin() * r] };
    // the head's angle and height at step j of ARCH_SEGS across arch k.
    let head = |k: usize, j: usize| -> (f64, f64) {
        let (a0, a1) = ((k as f64 + o0) / n * TAU, (k as f64 + o1) / n * TAU);
        let am = (a0 + a1) / 2.0;
        let th = PI * j as f64 / ARCH_SEGS as f64;
        let x = -half_w * th.cos();
        (am + (x / r1).clamp(-1.0, 1.0).asin(), spring + half_w * th.sin())
    };
    // The view's arch: the one nearest the heading (lathe angles: x = cos a, z = -sin a), left open.
    let view_h = 160.0;
    let vd = dir_of(view_h);
    let a_h = (-vd[1]).atan2(vd[0]).rem_euclid(TAU);
    let k_view = (0..arches).min_by(|p, q| {
        let ang = |k: &usize| (*k as f64 + 0.5) / n * TAU;
        let dp = (ang(p) - a_h).sin().atan2((ang(p) - a_h).cos()).abs();
        let dq = (ang(q) - a_h).sin().atan2((ang(q) - a_h).cos()).abs();
        dp.total_cmp(&dq)
    }).unwrap_or(0);
    let a_arch = (k_view as f64 + 0.5) / n * TAU;
    // A wall face at radius r from angle u to w, y from lo to hi, cut at
    // the formwork lifts (interior faces: each lift's tone, the lowest a grime band).
    let face = |m: &mut MeshArrays, r: f64, u: f64, w: f64, lo: f64, hi: f64, c: Rgb, lifts: bool| {
        if !lifts {
            m.quad(p(u, r, lo), p(w, r, lo), p(w, r, hi), p(u, r, hi), c);
            return;
        }
        let mut y = lo;
        while y < hi - 1e-6 {
            let k = ((y - floor) / COIT_LIFT_M).floor() as i64;
            let top = (floor + (k + 1) as f64 * COIT_LIFT_M).min(hi).max(y + 0.05).min(hi);
            let tone = if k <= 0 { 0.82 } else { lift_tone(k, (r * 100.0) as u64) };
            m.quad(p(u, r, y), p(w, r, y), p(w, r, top), p(u, r, top), shade(c, tone));
            y = top;
        }
    };
    for k in 0..arches {
        let a0 = (k as f64 + o0) / n * TAU;
        let a1 = (k as f64 + o1) / n * TAU;
        let am = (a0 + a1) / 2.0;
        let a2 = (k as f64 + 1.0 + o0) / n * TAU;
        let ap = (a1 + a2) / 2.0;
        // the outer face (open sky) and the inner face (the gallery's).
        for (r, lo, c, inside) in [(r1, bel0, outer_c, false), (r_in, floor - SLAB_M, inner_c, true)] {
            let m = if inside { b.mi("coit-concrete") } else { b.m("coit-concrete") };
            // the pier to the next arch, in two facets.
            for (u, w) in [(a1, ap), (ap, a2)] {
                face(m, r, u, w, lo, bel1, c, inside);
            }
            // under the sill.
            for (u, w) in [(a0, am), (am, a1)] {
                face(m, r, u, w, lo, sill, c, inside);
            }
            // over the round head: the spandrel from the intrados up to the band's top.
            for j in 0..ARCH_SEGS {
                let ((ua, ya), (ub, yb)) = (head(k, j), head(k, j + 1));
                m.quad(p(ua, r, ya), p(ub, r, yb), p(ub, r, bel1), p(ua, r, bel1), c);
            }
        }
        // the reveals: the jambs, the round soffit, the sill with its fall
        // and drip (INFERRED: a 25 mm fall outward, a 50 mm drip lip).
        let m = b.mi("coit-concrete");
        for a in [a0, a1] {
            m.quad(p(a, r1, sill), p(a, r_in, sill), p(a, r_in, spring), p(a, r1, spring), reveal_c);
        }
        for j in 0..ARCH_SEGS {
            let ((ua, ya), (ub, yb)) = (head(k, j), head(k, j + 1));
            m.quad(p(ua, r1, ya), p(ub, r1, yb), p(ub, r_in, yb), p(ua, r_in, ya), reveal_c);
        }
        for (u, w) in [(a0, am), (am, a1)] {
            m.quad(p(u, r1, sill - 0.025), p(w, r1, sill - 0.025), p(w, r_in, sill), p(u, r_in, sill), shade(concrete, 0.86));
        }
        let mo = b.m("coit-concrete");
        for (u, w) in [(a0, am), (am, a1)] {
            mo.quad(p(u, r1 + 0.05, sill - 0.085), p(w, r1 + 0.05, sill - 0.085), p(w, r1 + 0.05, sill - 0.025), p(u, r1 + 0.05, sill - 0.025), outer_c);
            mo.quad(p(u, r1, sill - 0.025), p(w, r1, sill - 0.025), p(w, r1 + 0.05, sill - 0.025), p(u, r1 + 0.05, sill - 0.025), outer_c);
            mo.quad(p(w, r1, sill - 0.085), p(u, r1, sill - 0.085), p(u, r1 + 0.05, sill - 0.085), p(w, r1 + 0.05, sill - 0.085), shade(outer_c, 0.7));
        }
    }
    {
        // The gallery's floor, its ceiling under the skylit cornice, and the stair shaft.
        let m = b.mi("coit-concrete");
        m.lathe(cx, cz, &[[r_in, floor], [r_core, floor]], 48, shade(concrete, 0.62), None, None);
        m.lathe(cx, cz, &[[r_core, floor - SLAB_M], [r_in, floor - SLAB_M]], 48, inner_c, None, None);
        m.lathe(cx, cz, &[[r_in, bel1], [r_core, bel1]], 48, shade(concrete, 0.95), None, None);
        let mut prof = vec![];
        let mut y = floor;
        while y < bel1 - 1e-6 {
            prof.push([r_core, y]);
            y = (y + COIT_LIFT_M).min(bel1);
        }
        prof.push([r_core, bel1]);
        let core_c = shade(concrete, 0.86);
        m.lathe(cx, cz, &prof, 40, core_c, None, Some(&|k: usize, _y: f64| shade(core_c, if k == 0 { 0.82 } else { lift_tone(k as i64, 7) })));
    }
    // The lift door in the stair shaft, facing west (the entrance side).
    {
        let m = b.mi("doors");
        let half = 0.45 / r_core;
        m.quad(p(PI - half, r_core + 0.02, floor), p(PI + half, r_core + 0.02, floor), p(PI + half, r_core + 0.02, floor + 2.1), p(PI - half, r_core + 0.02, floor + 2.1), srgb_hex(0x6b5a3e));
    }
    for k in 0..6 {
        let a = (k as f64 + 0.5) / 6.0 * TAU;
        b.lamp(p(a, (r_core + r_in) / 2.0, bel1 - 0.1), 600.0, 9.0);
    }
    // The gallery's sky: the arches' outer openings (the rectangle under the
    // spring, the head in strips), the stair shaft in the way.
    let mut portals: Vec<Portal> = vec![];
    let mut transmit: Vec<f64> = vec![];
    // the acrylic panes 0.1 m in from the outer face (INFERRED), every arch
    // but the view's; weathered acrylic's faint grey haze (INFERRED).
    let r_pane = r1 - 0.1;
    let pane_c = srgb_hex(0x3a3f3d);
    for k in 0..arches {
        let (a0, a1) = ((k as f64 + o0) / n * TAU, (k as f64 + o1) / n * TAU);
        let t = if k == k_view { 1.0 } else { COIT_PANE_T };
        let mut quads = vec![[p(a0, r1, sill), p(a1, r1, sill), p(a1, r1, spring), p(a0, r1, spring)]];
        for j in 0..ARCH_SEGS / 2 {
            let ((ua, ya), (ub, yb)) = (head(k, j), head(k, j + 1));
            let ((uc, yc), (ud, yd)) = (head(k, ARCH_SEGS - j - 1), head(k, ARCH_SEGS - j));
            quads.push([p(ua, r1, ya), p(ud, r1, yd), p(uc, r1, yc), p(ub, r1, yb)]);
        }
        if k != k_view {
            // the pane: the same outline at the pane's radius, in a thin frame.
            let at_pane = |q: V3| {
                let (dx, dz) = (q[0] - cx, q[2] - cz);
                let l = dx.hypot(dz).max(1e-9);
                [cx + dx / l * r_pane, q[1], cz + dz / l * r_pane]
            };
            for q in &quads {
                let g = b.m("glass");
                g.quad(at_pane(q[0]), at_pane(q[1]), at_pane(q[2]), at_pane(q[3]), pane_c);
            }
            // the frame: a 40 mm bronze-anodised bar round the pane and across at the spring (INFERRED).
            let fr = b.m("mullion");
            let frame_c = srgb_hex(0x4a4036);
            let up = |a: f64| Some([a.cos(), 0.0, -a.sin()]);
            fr.beam_up(p(a0, r_pane, sill + 0.02), p(a1, r_pane, sill + 0.02), 0.04, 0.05, frame_c, up((a0 + a1) / 2.0));
            fr.beam_up(p(a0, r_pane, spring), p(a1, r_pane, spring), 0.04, 0.05, frame_c, up((a0 + a1) / 2.0));
            for a in [a0, a1] {
                fr.beam_up(p(a, r_pane, sill), p(a, r_pane, spring), 0.04, 0.05, frame_c, up(a));
            }
            for j in 0..ARCH_SEGS {
                let ((ua, ya), (ub, yb)) = (head(k, j), head(k, j + 1));
                fr.beam_up(p(ua, r_pane, ya), p(ub, r_pane, yb), 0.04, 0.05, frame_c, up((ua + ub) / 2.0));
            }
        }
        for q in quads {
            portals.push(q);
            transmit.push(t);
        }
    }
    let t_mean = (COIT_PANE_T * (arches as f64 - 1.0) + 1.0) / arches as f64;
    let window = arches as f64 * (2.0 * half_w * (spring - sill) + PI / 2.0 * half_w * half_w);
    let h = bel1 - floor;
    let (a_floor, a_ceil) = (PI * (r_in * r_in - r_core * r_core), PI * (r_in * r_in - r_core * r_core));
    let (a_outer, a_core) = (TAU * r_in * h - window, TAU * r_core * h);
    let (rf, rw) = (reflectance(shade(concrete, 0.62)), reflectance(inner_c));
    let total = a_floor + a_ceil + a_outer + a_core + window;
    let r_mean = (a_floor * rf + a_ceil * rw + (a_outer + a_core) * rw + window * 0.1) / total;
    // the walls split at the openings' mid-height.
    let mid = ((sill + crown) / 2.0 - floor) / h;
    let r_fw = (a_floor * rf + (a_outer + a_core) * mid * rw) / (a_floor + (a_outer + a_core) * mid);
    let r_cw = (a_ceil * rw + (a_outer + a_core) * (1.0 - mid) * rw) / (a_ceil + (a_outer + a_core) * (1.0 - mid));
    let outward: Vec<(Portal, V3)> = portals.iter().map(|q| {
        let c = [(q[0][0] + q[2][0]) / 2.0 - cx, 0.0, (q[0][2] + q[2][2]) / 2.0 - cz];
        (*q, c)
    }).collect();
    let fill = crate::access_sky::irc(window, total, r_mean, r_fw, r_cw) * t_mean / IRC_GLAZING_T + t_mean * crate::access_sky::irc_sun(&outward, total, r_mean, rf);
    b.room = Some(Room { portals, transmit, occluders: vec![Prism::cylinder(cx, cz, r_core, floor - 0.5, bel1 + 0.5, 32)], fill });
    // the gallery for the camera meter: inside the arcade's outer face, floor to ceiling.
    b.bounds = Some(((0..48).map(|k| k as f64 / 48.0 * TAU).map(|a| [cx + a.cos() * (r1 + 0.1), cz - a.sin() * (r1 + 0.1)]).collect(), floor - 0.2, bel1));
    b.walk_annulus(cx, cz, r_core, r_in, floor, 48);
    // The arcade is open: an invisible guard stands in the arches' plane over the sills.
    b.wall_cylinder(cx, cz, r_in, floor - 0.2, bel1, 48);
    b.wall_cylinder(cx, cz, r_core, floor - 0.2, bel1, 24);
    // In the view's arch, looking along the heading.
    let view = Stop { feet: p(a_arch, r_in - 0.35, floor), heading_deg: view_h };
    b.vantage = Some(p(a_arch, r1 - 0.05, floor + EYE_M));
    // The deck shot: from along the ring, the person at the arch beyond.
    let ca = a_arch + 52f64.to_radians();
    let rc = (r_core + r_in) / 2.0 + 0.15;
    let cam_eye = p(ca, rc, floor + EYE_M + 0.05);
    let cam_at = [view.feet[0] + vd[0] * 1.5, floor + 1.25, view.feet[2] + vd[1] * 1.5];
    // The entrance: the base's west side (the parking loop and the Columbus statue face it).
    let door = door_outside(&ring, 270.0, 1.4);
    let lifts = vec![Lift {
        id: "coit-tower".into(),
        name: "Coit Tower gallery".into(),
        bottom: Stop { feet: [door[0], g.at(door[0], door[1]), door[1]], heading_deg: 270.0 },
        top: Stop { feet: [cx - (r_core + COIT_GALLERY_M / 2.0), floor, cz], heading_deg: view_h },
    }];
    let cuts = vec![Cut { centre: [cx, cz], radius_m: 12.0, y0: floor - 0.8, y1: bel1 + 0.5 }];
    b.finish("Coit Tower gallery", id, lifts, cuts, view, -5.0, (cam_eye, cam_at), floor, shift)
}

// ---------------------------------------------------------------------------
// Salesforce Tower: the top floor's observation level.
// ---------------------------------------------------------------------------

/// The curtain wall's mullion pitch (5 ft units, INFERRED), metres.
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
    let glass_tint = shade(hexc(d, "glassSrgb"), 0.35);
    let mut portals: Vec<Portal> = vec![];
    let mut outward: Vec<(Portal, V3)> = vec![];
    let mut window_m2 = 0.0;
    {
        let floor_c = srgb_hex(0xb8b3a9);
        slab(b.mi("slab"), &plate, floor + 0.02, 0.35, floor_c, shade(metal, 0.55));
        // The ceiling (its underside is what is seen) and the core's walls.
        let ceil_ring = scale_ring(&ring, cx, cz, scale_at(y_hi) * (1.0 - 0.12 / r_mean));
        slab(b.mi("interior"), &ceil_ring, ceil + 0.3, 0.3, srgb_hex(0xe4e2dc), srgb_hex(0xd8d6d0));
        b.mi("interior").prism(&core, floor, ceil, srgb_hex(0xd8d6d0), false, false);
        // Linear diffusers in the ceiling, round the core (INFERRED: 1.2 m by 0.15 m slots on a 3 m grid).
        for k in 0..16 {
            let a = (k as f64 + 0.5) / 16.0 * TAU;
            let r = h * 1.25 + 3.5;
            let (ca, sa) = (a.cos(), a.sin());
            let c = [cx + ca * r, cz - sa * r];
            let t = [-sa * 0.6, -ca * 0.6];
            let w = [ca * 0.075, -sa * 0.075];
            let y = ceil - 0.012;
            let q = |s0: f64, s1: f64| [c[0] + t[0] * s0 + w[0] * s1, y, c[1] + t[1] * s0 + w[1] * s1];
            b.mi("fixtures").quad(q(-1.0, -1.0), q(-1.0, 1.0), q(1.0, 1.0), q(1.0, -1.0), srgb_hex(0xf4f2ec));
        }
        // The curtain wall: aluminium mullions (INFERRED: a unitised wall's
        // 65 mm face, 150 mm deep) on the glass line, a sill and a head; the
        // panes between them, and the openings the room's sky comes through.
        let n = ring.len();
        mullions.clear();
        for i in 0..n {
            let (a0, a1, b0, b1) = (glass_lo[i], glass_lo[(i + 1) % n], glass_hi[i], glass_hi[(i + 1) % n]);
            let len = (a1[0] - a0[0]).hypot(a1[1] - a0[1]);
            let l = len.max(1e-9);
            // the facet's outward normal (away from the plate's centre).
            let mut out = [(a1[1] - a0[1]) / l, -(a1[0] - a0[0]) / l];
            if out[0] * ((a0[0] + a1[0]) / 2.0 - cx) + out[1] * ((a0[1] + a1[1]) / 2.0 - cz) < 0.0 {
                out = [-out[0], -out[1]];
            }
            let up = Some([out[0], 0.0, out[1]]);
            let cnt = (len / SF_MULLION_PITCH_M).ceil().max(1.0) as usize;
            let at = |t: f64, lo: bool| -> V3 {
                if lo { [a0[0] + (a1[0] - a0[0]) * t, floor, a0[1] + (a1[1] - a0[1]) * t] } else { [b0[0] + (b1[0] - b0[0]) * t, ceil, b0[1] + (b1[1] - b0[1]) * t] }
            };
            let m = b.m("mullion");
            for k in 0..cnt {
                let t = k as f64 / cnt as f64;
                let (lo, hi) = (at(t, true), at(t, false));
                m.beam_up([lo[0] - out[0] * 0.04, lo[1], lo[2] - out[1] * 0.04], [hi[0] - out[0] * 0.04, hi[1], hi[2] - out[1] * 0.04], 0.065, 0.15, metal, up);
                mullions.push(([lo[0], lo[2]], [(a1[0] - a0[0]) / l, (a1[1] - a0[1]) / l]));
            }
            m.beam_up([a0[0], floor + 0.05, a0[1]], [a1[0], floor + 0.05, a1[1]], 0.12, 0.1, metal, None);
            m.beam_up([b0[0], ceil - 0.05, b0[1]], [b1[0], ceil - 0.05, b1[1]], 0.12, 0.1, metal, None);
            // the panes, just outside the mullions' inner faces.
            let g = b.m("glass");
            for k in 0..cnt {
                let (t0, t1) = (k as f64 / cnt as f64, (k + 1) as f64 / cnt as f64);
                let o = |v: V3| [v[0] + out[0] * 0.02, v[1], v[2] + out[1] * 0.02];
                g.quad(o(at(t0, true)), o(at(t1, true)), o(at(t1, false)), o(at(t0, false)), glass_tint);
            }
            portals.push([[a0[0], floor + 0.1, a0[1]], [a1[0], floor + 0.1, a1[1]], [b1[0], ceil - 0.1, b1[1]], [b0[0], ceil - 0.1, b0[1]]]);
            outward.push((*portals.last().unwrap(), [out[0], 0.0, out[1]]));
            window_m2 += len * (ceil - floor - 0.2);
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
        let dm = b.mi("doors");
        for k in [-3.0, 0.0, 3.0] {
            let tang = [-f[1], f[0]];
            let c0 = core_pt(f[0] * (h + 0.02) + tang[0] * (k - 0.6), f[1] * (h + 0.02) + tang[1] * (k - 0.6));
            let c1 = core_pt(f[0] * (h + 0.02) + tang[0] * (k + 0.6), f[1] * (h + 0.02) + tang[1] * (k + 0.6));
            dm.quad([c0[0], floor, c0[1]], [c1[0], floor, c1[1]], [c1[0], floor + 2.4, c1[1]], [c0[0], floor + 2.4, c0[1]], srgb_hex(0x9a9c9e));
        }
    }
    {
        // The room's light: the curtain wall's sky past the core, the
        // inter-reflected share (BRE split flux) and the floor's electric light.
        let a_floor = crate::geometry::signed_area(&plate).abs();
        let perim: f64 = (0..plate.len()).map(|i| (plate[(i + 1) % plate.len()][0] - plate[i][0]).hypot(plate[(i + 1) % plate.len()][1] - plate[i][1])).sum();
        let hh = ceil - floor;
        let a_core = 8.0 * h * hh;
        let total = 2.0 * a_floor + perim * hh + a_core;
        let (rf, rc, rw) = (reflectance(srgb_hex(0xb8b3a9)), reflectance(srgb_hex(0xe4e2dc)), reflectance(srgb_hex(0xd8d6d0)));
        let r_mean = (a_floor * rf + a_floor * rc + a_core * rw + window_m2 * 0.1) / total;
        let r_fw = (a_floor * rf + a_core * 0.5 * rw) / (a_floor + a_core * 0.5);
        let r_cw = (a_floor * rc + a_core * 0.5 * rw) / (a_floor + a_core * 0.5);
        // the glass passes GLAZING_T of the sky and the beam.
        let fill = crate::access_sky::irc(window_m2, total, r_mean, r_fw, r_cw) * GLAZING_T / IRC_GLAZING_T + GLAZING_T * crate::access_sky::irc_sun(&outward, total, r_mean, rf) + crate::access_sky::ELECTRIC_SHARE;
        let transmit = vec![GLAZING_T; portals.len()];
        b.room = Some(Room { portals, transmit, occluders: vec![Prism { ring: core.clone(), y0: floor - 0.5, y1: ceil + 0.5 }], fill });
        // the floor for the camera meter: inside the curtain wall, floor to ceiling.
        b.bounds = Some((glass_lo.clone(), floor - 0.2, ceil + 0.3));
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

/// The precast wall, outer face to inner face (INFERRED: the facade's
/// deep-set windows; its quartz-aggregate panels stand proud of the glass).
const TA_WALL_M: f64 = 0.45;

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
    // The windows of a face as the generator spaces them: (t0, t1) along it.
    let windows = |side: usize| -> Vec<(f64, f64)> {
        let h0 = half(yw0) + 0.06;
        let count = ((2.0 * h0 - 2.0) / pitch).floor().max(1.0) as i64;
        (0..count)
            .map(|k| {
                let s0 = -1.0 + (2.0 * k as f64 + 0.15) / count as f64;
                let s1 = -1.0 + (2.0 * k as f64 + 1.85) / count as f64;
                (s0 * (h0 - 1.0) / h0, s1 * (h0 - 1.0) / h0)
            })
            .filter(|(t0, t1)| !(winged(side) && (t0 * h0).abs().min((t1 * h0).abs()) < wing_w + 0.3))
            .collect()
    };
    let glass_tint = shade(hexc(d, "glassSrgb"), 1.6);
    let mut portals: Vec<Portal> = vec![];
    let mut outward: Vec<(Portal, V3)> = vec![];
    let mut window_m2 = 0.0;
    let inner_c = srgb_hex(0xe6e3dc);
    {
        let outer_d = |y: f64| half(y);
        let inner_d = |y: f64| half(y) - TA_WALL_M;
        for side in 0..4 {
            let wins = windows(side);
            // the precast outside (open sky), the plastered inside (the room's).
            for (inside, c) in [(false, precast_c), (true, inner_c)] {
                let dist_of: &dyn Fn(f64) -> f64 = if inside { &inner_d } else { &outer_d };
                let q = |t0: f64, t1: f64, y0: f64, y1: f64| -> [V3; 4] { [fp(side, t0 * dist_of(y0), dist_of(y0), y0), fp(side, t1 * dist_of(y0), dist_of(y0), y0), fp(side, t1 * dist_of(y1), dist_of(y1), y1), fp(side, t0 * dist_of(y1), dist_of(y1), y1)] };
                let m = if inside { b.mi("interior") } else { b.m("ta-precast") };
                // below and above the windows, the face's whole width.
                for (y0, y1) in [(ya - 0.35, yw0), (yw1, yb)] {
                    let [a1, a2, a3, a4] = q(-1.0, 1.0, y0, y1);
                    m.quad(a1, a2, a3, a4, c);
                }
                // the piers between the windows.
                let mut edges = vec![-1.0];
                for (t0, t1) in &wins {
                    edges.push(*t0);
                    edges.push(*t1);
                }
                edges.push(1.0);
                for pair in edges.chunks_exact(2) {
                    let [a1, a2, a3, a4] = q(pair[0], pair[1], yw0, yw1);
                    m.quad(a1, a2, a3, a4, c);
                }
            }
            // the window reveals (sill, head, jambs) through the precast's
            // depth, the aluminium frame and its pane at the depth's middle.
            for &(t0, t1) in &wins {
                let o = |t: f64, y: f64| fp(side, t * half(y), half(y), y);
                let i = |t: f64, y: f64| fp(side, t * (half(y) - TA_WALL_M), half(y) - TA_WALL_M, y);
                let m = b.m("ta-precast");
                m.quad(o(t0, yw0), o(t1, yw0), i(t1, yw0), i(t0, yw0), shade(precast_c, 0.85));
                m.quad(o(t1, yw1), o(t0, yw1), i(t0, yw1), i(t1, yw1), shade(precast_c, 0.7));
                for t in [t0, t1] {
                    m.quad(o(t, yw0), i(t, yw0), i(t, yw1), o(t, yw1), shade(precast_c, 0.78));
                }
                let fd = |y: f64| half(y) - TA_WALL_M * 0.5;
                let f = |t: f64, y: f64| fp(side, t * fd(y), fd(y), y);
                let m = b.m("ta-metal");
                // the frame on the glass plane, its section turned to the face (INFERRED: 60 mm sightline).
                let nrm = { let a = fp(side, 0.0, 1.0, 0.0); let c = fp(side, 0.0, 0.0, 0.0); [a[0] - c[0], 0.0, a[2] - c[2]] };
                let up = Some(nrm);
                m.beam_up(f(t0, yw0 + 0.03), f(t1, yw0 + 0.03), 0.06, 0.09, metal_c, up);
                m.beam_up(f(t0, yw1 - 0.03), f(t1, yw1 - 0.03), 0.06, 0.09, metal_c, up);
                m.beam_up(f(t0, yw0), f(t0, yw1), 0.06, 0.09, metal_c, up);
                m.beam_up(f(t1, yw0), f(t1, yw1), 0.06, 0.09, metal_c, up);
                let tm = (t0 + t1) / 2.0;
                m.beam_up(f(tm, yw0), f(tm, yw1), 0.05, 0.08, metal_c, up);
                b.m("glass").quad(f(t0, yw0), f(t1, yw0), f(t1, yw1), f(t0, yw1), glass_tint);
                portals.push([o(t0, yw0 + 0.04), o(t1, yw0 + 0.04), o(t1, yw1 - 0.04), o(t0, yw1 - 0.04)]);
                outward.push((*portals.last().unwrap(), nrm));
                let w = (o(t1, yw0)[0] - o(t0, yw0)[0]).hypot(o(t1, yw0)[2] - o(t0, yw0)[2]);
                window_m2 += w * band;
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
    // a light terrazzo floor (INFERRED: the lobby finish carried up).
    let floor_c = srgb_hex(0xb3aea4);
    slab(b.mi("slab"), &square(df), floor, 0.4, floor_c, shade(precast_c, 0.8));
    let dc = half(yb) - TA_WALL_M;
    slab(b.mi("interior"), &square(dc), base + yb, 0.25, srgb_hex(0xe2dfd8), srgb_hex(0xe2dfd8));
    // The ceiling's troffers: their light is the room's electric share in
    // the baked fill (access_sky.rs ELECTRIC_SHARE), as the Salesforce
    // floor's is, not point lamps, whose highlight a polished floor shows
    // as a hot spot under a fixture that does not glow.
    for (a, c) in [(-3.0, -3.0), (3.0, -3.0), (3.0, 3.0), (-3.0, 3.0)] {
        // a 0.6 m square diffuser (INFERRED: a 2 ft lay-in troffer).
        let q = |da: f64, dc2: f64| at(a + da, c + dc2, yb - 0.012);
        b.mi("fixtures").quad(q(-0.3, -0.3), q(0.3, -0.3), q(0.3, 0.3), q(-0.3, 0.3), srgb_hex(0xf4f2ec));
    }
    // The wings' cores inside the room (the east one the lifts, the west
    // one the stair: Wikipedia "Transamerica Pyramid"), plastered, with
    // the lift doors and the stair door under its exit sign (CBC 1013.3:
    // internally lit, at the door; INFERRED green, the city's usual).
    let w = wing_w;
    for sgn in [-1.0, 1.0] {
        let wy0 = ya;
        // 3 cm proud of the generator's own wing walls (towers.rs), which they would z-fight.
        let (wo, wi2) = (w + 0.03, wing_in - 0.03);
        let corners = [at(-wo, sgn * wi2, wy0), at(wo, sgn * wi2, wy0), at(wo, sgn * df, wy0), at(-wo, sgn * df, wy0)];
        let up = |p: V3| [p[0], base + yb, p[2]];
        let m = b.mi("interior");
        m.quad(corners[0], corners[1], up(corners[1]), up(corners[0]), shade(inner_c, 0.97));
        m.quad(corners[1], corners[2], up(corners[2]), up(corners[1]), shade(inner_c, 0.95));
        m.quad(corners[3], corners[0], up(corners[0]), up(corners[3]), shade(inner_c, 0.95));
        // the core's face toward the room (b = sgn wing_in): doors 0.05 proud.
        let face = |aa: f64, yy: f64| at(aa, sgn * (wing_in - 0.06), yy);
        // which wing is east: its outward v points east.
        let east = sgn * v[0] > 0.0;
        if east {
            // two lift doors (INFERRED: 1.1 m by 2.15 m, ASME A17.1 car doors are 0.9 to 1.2 m).
            for c0 in [-1.7, 0.6] {
                b.mi("doors").quad(face(c0, ya + 0.05), face(c0 + 1.1, ya + 0.05), face(c0 + 1.1, ya + 2.2), face(c0, ya + 2.2), srgb_hex(0xa7a9ab));
            }
        } else {
            b.mi("doors").quad(face(-0.5, ya + 0.05), face(0.5, ya + 0.05), face(0.5, ya + 2.1), face(-0.5, ya + 2.1), srgb_hex(0x6d6a64));
            b.mi("fixtures").quad(face(-0.18, ya + 2.25), face(0.18, ya + 2.25), face(0.18, ya + 2.45), face(-0.18, ya + 2.45), srgb_hex(0x1d9b4a));
        }
    }
    {
        // The room's light: the windows' sky past the wing cores, the
        // inter-reflected share (BRE split flux) and the electric light.
        let side_m = 2.0 * df;
        let hh = yb - ya;
        let a_floor = side_m * side_m;
        let total = 2.0 * a_floor + 4.0 * side_m * hh;
        let (rf, rc, rw) = (reflectance(floor_c), reflectance(srgb_hex(0xe2dfd8)), reflectance(inner_c));
        let walls = 4.0 * side_m * hh - window_m2;
        let r_mean = (a_floor * rf + a_floor * rc + walls * rw + window_m2 * 0.1) / total;
        let r_fw = (a_floor * rf + walls * 0.5 * rw) / (a_floor + walls * 0.5);
        let r_cw = (a_floor * rc + walls * 0.5 * rw) / (a_floor + walls * 0.5);
        let fill = crate::access_sky::irc(window_m2, total, r_mean, r_fw, r_cw) * GLAZING_T / IRC_GLAZING_T + GLAZING_T * crate::access_sky::irc_sun(&outward, total, r_mean, rf) + crate::access_sky::ELECTRIC_SHARE;
        let core = |sgn: f64| -> Prism {
            let ring = [[-w, sgn * wing_in], [w, sgn * wing_in], [w, sgn * (df + 1.0)], [-w, sgn * (df + 1.0)]].iter().map(|q| {
                let p = at(q[0], q[1], 0.0);
                [p[0], p[2]]
            }).collect();
            Prism { ring, y0: floor - 0.5, y1: base + yb + 0.5 }
        };
        let transmit = vec![GLAZING_T; portals.len()];
        b.room = Some(Room { portals, transmit, occluders: vec![core(-1.0), core(1.0)], fill });
        // the room for the camera meter: inside the precast's outer face, floor to ceiling.
        let ring: Vec<[f64; 2]> = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]].iter().map(|q| {
            let p = at(q[0] * half(ya), q[1] * half(ya), 0.0);
            [p[0], p[2]]
        }).collect();
        b.bounds = Some((ring, floor - 0.2, base + yb));
    }
    // The walkable room: the square less the two wings' cores (|a| < wing, |b| > wing_in).
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
    // The east sidewalk's railing (the generator leaves it to the deck):
    // Irving Morrow's railing of "modest, uniform posts" spaced so the view
    // stays open (goldengate.org "Color and Art Deco styling"), 4 ft
    // (1.22 m) high, its pickets 4 in (102 mm) H-sections (Marin County
    // Bicycle Coalition on the 2000s west-railing retrofit, which thinned
    // the west side's to 1/4 in slats), on 12 in (0.305 m) centres
    // (INFERRED), between a capped top rail and a bottom rail, with a
    // heavier post at every half panel (the generator's 3.81 m).
    let s_rail = half_w - 0.15;
    let picket = |m: &mut MeshArrays, a: f64, c1: f64| {
        let (pa, pb) = (frame.P(a, s_rail, side(a)), frame.P(c1, s_rail, side(c1)));
        let up = |p: V3, h: f64| [p[0], p[1] + h, p[2]];
        let across = [site.across[0], 0.0, site.across[1]];
        m.beam_up(up(pa, railing - 0.035), up(pb, railing - 0.035), 0.16, 0.07, orange, Some([0.0, 1.0, 0.0]));
        m.beam_up(up(pa, 0.12), up(pb, 0.12), 0.1, 0.06, orange, Some([0.0, 1.0, 0.0]));
        let len = (c1 - a).abs();
        let n = (len / 0.305).round().max(1.0) as usize;
        for k in 0..n {
            let t = (k as f64 + 0.5) / n as f64;
            let p = [pa[0] + (pb[0] - pa[0]) * t, pa[1] + (pb[1] - pa[1]) * t, pa[2] + (pb[2] - pa[2]) * t];
            m.beam_up(up(p, 0.15), up(p, railing - 0.07), 0.102, 0.035, shade(orange, 0.97), Some(across));
        }
        m.beam_up(up(pa, 0.0), up(pa, railing + 0.04), 0.16, 0.16, orange, Some(across));
    };
    for w2 in rails.windows(2) {
        let (a, c1) = (w2[0], w2[1]);
        if gap(a, c1) {
            continue;
        }
        picket(b.m("gg-orange"), a, c1);
    }
    for (a, c1) in open_runs(&rails) {
        picket(b.m("gg-orange"), a, c1);
    }
    // The suspender ropes' cast sockets where they land on the truss's top
    // chord, just outside the railing (INFERRED: a 150 mm cast socket 0.6 m
    // long on each rope of a pair, a gusset to the chord), at the stations
    // suspension.rs `suspender_pairs` hangs them (golden_gate.rs cable runs).
    {
        let saddle = js.sea_level_y + num(d, "towerAboveWaterM") - num(d, "saddleBelowTopM");
        let low = plan.y_mid + num(d, "cableLowAboveDeckM");
        let sag_main = saddle - low;
        let south_top = plan.deck_top(plan.arch_north_c) + num(d, "pylonAboveDeckM");
        let north_top = plan.deck_top(plan.anchor_north_c) + num(d, "pylonAboveDeckM");
        let runs = [
            crate::suspension::Run { c0: plan.arch_north_c, c1: 0.0, y0: south_top, y1: saddle, sag: crate::suspension::side_span_sag(sag_main, plan.l, -plan.arch_north_c) },
            crate::suspension::Run { c0: 0.0, c1: plan.l, y0: saddle, y1: saddle, sag: sag_main },
            crate::suspension::Run { c0: plan.l, c1: plan.anchor_north_c, y0: saddle, y1: north_top, sag: crate::suspension::side_span_sag(sag_main, plan.l, plan.anchor_north_c - plan.l) },
        ];
        let spacing = num(d, "suspenderSpacingM");
        let m = b.m("gg-orange");
        for r in &runs {
            let n = ((r.c1 - r.c0) / spacing).floor() as i64;
            let offset = ((r.c1 - r.c0) - n as f64 * spacing) / 2.0;
            for k in 0..=n {
                let c = r.c0 + offset + k as f64 * spacing;
                if c - r.c0 < 6.0 || r.c1 - c < 6.0 {
                    continue;
                }
                let Some(top) = crate::suspension::cable_height_at(&[*r], c) else { continue };
                let bottom = plan.deck_top(c) - 0.5;
                if top - bottom < 0.5 {
                    continue;
                }
                for sgn in [-1.0, 1.0] {
                    let sp = sgn * hp;
                    for dd in [-0.225, 0.225] {
                        let q0 = frame.P(c + dd, sp, bottom);
                        m.lathe(q0[0], q0[2], &[[0.075, bottom], [0.075, side(c) + 0.45], [0.05, side(c) + 0.55], [0.035, side(c) + 0.6]], 10, shade(orange, 0.9), None, None);
                    }
                    m.beam_up(frame.P(c - 0.3, sp, bottom + 0.1), frame.P(c + 0.3, sp, bottom + 0.1), 0.03, 0.4, shade(orange, 0.8), Some([site.across[0], 0.0, site.across[1]]));
                }
            }
        }
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
        // the leg's cap: steel deck plate (INFERRED: 1.5 m plates, their
        // seams, an access hatch by the inner face), painted with the tower.
        slab(b.m("gg-plate"), &deck, top_y + 0.04, 0.04, shade(orange, 0.92), orange);
        for k in 1..((2.0 * la / 1.5).floor() as i64) {
            let a = -la + k as f64 * 1.5;
            b.m("gg-orange").beam_up(at(p, a, -lx + 0.02, top_y + 0.045), at(p, a, lx - 0.02, top_y + 0.045), 0.03, 0.01, shade(orange, 0.7), None);
        }
        let hs = inward * (lx - 0.9);
        let hatch: Vec<[f64; 2]> = [[1.2, hs - 0.45], [2.1, hs - 0.45], [2.1, hs + 0.45], [1.2, hs + 0.45]].iter().map(|q| {
            let v = at(p, q[0], q[1], 0.0);
            [v[0], v[2]]
        }).collect();
        slab(b.m("gg-orange"), &hatch, top_y + 0.1, 0.06, shade(orange, 0.85), shade(orange, 0.6));
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
    grating_strip(&mut b, ([cat[0][0], cat[0][1]], [cat[3][0], cat[3][1]]), ([cat[1][0], cat[1][1]], [cat[2][0], cat[2][1]]), y, orange, "gg-orange");
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
    // A saddle housing over each main cable's saddle (INFERRED: the cast
    // saddle's riveted cover, 5 m along the cable and 1.8 m across for its
    // 0.92 m cable, goldengate.org; walls 1 m, a round roof, rivet rows on
    // its seams at 120 mm, a band at each end where the cable passes).
    let (hl, hw, hwall) = (2.5, 0.9, 1.0);
    for p in [e, w] {
        let m = b.m("gg-orange");
        let c_h = shade(orange, 0.9);
        let q = |a: f64, s2: f64, yy: f64| at(p, a, s2, yy);
        // the walls.
        for sgn in [-1.0, 1.0] {
            m.quad(q(-hl, sgn * hw, top_y), q(hl, sgn * hw, top_y), q(hl, sgn * hw, top_y + hwall), q(-hl, sgn * hw, top_y + hwall), c_h);
        }
        // the round roof and its end walls.
        let segs = 12;
        for k in 0..segs {
            let (t0, t1) = (PI * k as f64 / segs as f64, PI * (k + 1) as f64 / segs as f64);
            let (s0, y0r, s1, y1r) = (hw * t0.cos(), top_y + hwall + hw * t0.sin(), hw * t1.cos(), top_y + hwall + hw * t1.sin());
            m.quad(q(-hl, s0, y0r), q(-hl, s1, y1r), q(hl, s1, y1r), q(hl, s0, y0r), shade(c_h, 0.96 + 0.04 * (t0 - PI / 2.0).cos()));
            for a in [-hl, hl] {
                m.tri(q(a, 0.0, top_y + hwall), q(a, s0, y0r), q(a, s1, y1r), shade(c_h, 0.85));
            }
        }
        for a in [-hl, hl] {
            m.quad(q(a, -hw, top_y), q(a, hw, top_y), q(a, hw, top_y + hwall), q(a, -hw, top_y + hwall), shade(c_h, 0.85));
        }
        // rivet heads (INFERRED: 22 mm rivets, 35 mm heads) on the roof's springing seams and the end seams.
        let n_r = (2.0 * hl / 0.12).floor() as usize;
        for k in 0..=n_r {
            let a = -hl + 2.0 * hl * k as f64 / n_r as f64;
            for sgn in [-1.0, 1.0] {
                let c0 = q(a, sgn * (hw + 0.012), top_y + hwall - 0.06);
                m.box_(c0[0], c0[2], c0[1] - 0.017, c0[1] + 0.017, 0.017, 0.017, 0.0, shade(c_h, 0.8), ALL);
            }
        }
        for a in [-hl + 0.05, hl - 0.05] {
            for k in 0..=12 {
                let t = PI * k as f64 / 12.0;
                let c0 = q(a, (hw + 0.012) * t.cos(), top_y + hwall + (hw + 0.012) * t.sin());
                m.box_(c0[0], c0[2], c0[1] - 0.017, c0[1] + 0.017, 0.017, 0.017, 0.0, shade(c_h, 0.8), ALL);
            }
        }
        let ring: Vec<[f64; 2]> = [[-hl - 0.05, -hw - 0.05], [hl + 0.05, -hw - 0.05], [hl + 0.05, hw + 0.05], [-hl - 0.05, hw + 0.05]].iter().map(|q2| {
            let v = at(p, q2[0], q2[1], 0.0);
            [v[0], v[2]]
        }).collect();
        b.wall_ring(&ring, top_y - 0.2, top_y + hwall + hw);
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
    // The legs' caps: steel deck plate with an access hatch each; the top
    // strut between them: open bar grating on its edge channels.
    for sgn in [-1.0, 1.0] {
        let cap: Vec<[f64; 2]> = [[-la, sgn * h - lx], [la, sgn * h - lx], [la, sgn * h + lx], [-la, sgn * h + lx]].iter().map(|q| f.point(tc + q[0], q[1])).collect();
        slab(b.m("bb-steel"), &cap, y, 0.04, shade(steel, 1.1), steel);
        let hs = sgn * (h + 1.2);
        let hatch: Vec<[f64; 2]> = [[0.6, hs - 0.35], [1.5, hs - 0.35], [1.5, hs + 0.35], [0.6, hs + 0.35]].iter().map(|q| f.point(tc + q[0], q[1])).collect();
        slab(b.m("bb-steel"), &hatch, y + 0.07, 0.06, shade(steel, 1.2), shade(steel, 0.7));
    }
    let gs = |c: f64, s2: f64| f.point(tc + c, s2);
    grating_strip(&mut b, (gs(-ba, -h + lx), gs(-ba, h - lx)), (gs(ba, -h + lx), gs(ba, h - lx)), y, steel, "bb-steel");
    // A cable saddle housing on each leg over its main cable (INFERRED:
    // 4 m long, 1.6 m across for the 0.72 m cable, a round roof), the cable
    // itself the generator's (bay_bridge.rs) under it.
    let axis_yaw = f.axis[1].atan2(f.axis[0]);
    for sgn in [-1.0, 1.0] {
        let c0 = f.point(tc, sgn * h);
        let m = b.m("bb-steel");
        m.box_(c0[0], c0[1], y, y + 0.9, 2.0, 0.8, axis_yaw, shade(steel, 1.05), ALL);
        for k in 0..10 {
            let (t0, t1) = (PI * k as f64 / 10.0, PI * (k + 1) as f64 / 10.0);
            let q = |c: f64, t: f64| {
                let pp = f.point(tc + c, sgn * h + 0.8 * t.cos());
                [pp[0], y + 0.9 + 0.8 * t.sin(), pp[1]]
            };
            m.quad(q(-2.0, t0), q(-2.0, t1), q(2.0, t1), q(2.0, t0), shade(steel, 1.0 + 0.08 * (t0 - PI / 2.0).cos()));
            for c in [-2.0, 2.0] {
                let cc = f.point(tc + c, sgn * h);
                m.tri([cc[0], y + 0.9, cc[1]], q(c, t0), q(c, t1), shade(steel, 0.95));
            }
        }
        let ring: Vec<[f64; 2]> = [[-2.05, sgn * h - 0.85], [2.05, sgn * h - 0.85], [2.05, sgn * h + 0.85], [-2.05, sgn * h + 0.85]].iter().map(|q| f.point(tc + q[0], q[1])).collect();
        b.wall_ring(&ring, y - 0.2, y + 1.7);
    }
    b.walk_ring(&outline, y);
    let rail: Vec<V3> = inset_ring(&outline, 0.15).iter().map(|p| [p[0], y, p[1]]).collect();
    b.guardrail_ring("bb-steel", &rail, shade(steel, 1.25), true, true);
    // An aviation obstruction beacon on a mast at each leg's outer corner
    // (FAA AC 70/7460-1M L-864 red, the bridge lights' own lamp there).
    for sgn in [-1.0, 1.0] {
        let mp = f.point(tc + la - 0.5, sgn * (h + lx - 0.5));
        let m = b.m("bb-steel");
        m.beam_up([mp[0], y, mp[1]], [mp[0], y + 2.4, mp[1]], 0.09, 0.09, shade(steel, 0.9), None);
        m.lathe(mp[0], mp[1], &[[0.0, y + 2.4], [0.16, y + 2.4], [0.16, y + 2.55], [0.0, y + 2.56]], 12, shade(steel, 0.8), None, None);
        b.m("fixtures").lathe(mp[0], mp[1], &[[0.0, y + 2.56], [0.13, y + 2.57], [0.12, y + 2.75], [0.0, y + 2.8]], 12, srgb_hex(0xb3261e), None, None);
    }
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

/// Welded bar grating (ANSI/NAAMM MBG 531-17, "19-W-4": bearing bars on
/// 1 3/16 in (30 mm) centres, 1 in (25 mm) deep, 3/16 in (4.8 mm) thick;
/// cross bars on 4 in (102 mm) centres), open between the bars, on two
/// edge channels (C8 x 11.5: 203 mm deep, AISC Manual table 1-5) with
/// stiffener plates every 1.2 m (INFERRED), between the outer and inner
/// rings of a walkway at height y.
fn open_grating(b: &mut B, outer: &[[f64; 2]], inner: &[[f64; 2]], y: f64, steel: Rgb) {
    let n = outer.len();
    for i in 0..n {
        grating_strip(b, (outer[i], outer[(i + 1) % n]), (inner[i], inner[(i + 1) % n]), y, steel, "sutro-steel");
    }
}

/// One straight run of `open_grating`: from the edge a -> q to the edge ai -> qi.
fn grating_strip(b: &mut B, (a, q): ([f64; 2], [f64; 2]), (ai, qi): ([f64; 2], [f64; 2]), y: f64, steel: Rgb, steel_key: &'static str) {
    let bar_c = srgb_hex(0x7d8082);
    let len = (q[0] - a[0]).hypot(q[1] - a[1]);
    let lin = (qi[0] - ai[0]).hypot(qi[1] - ai[1]);
    let m = b.m("grating");
    // bearing bars across the walk.
    let bars = (len.max(lin) / 0.03).floor() as usize;
    for k in 0..=bars {
        let t = k as f64 / bars.max(1) as f64;
        let o = [a[0] + (q[0] - a[0]) * t, a[1] + (q[1] - a[1]) * t];
        let ii = [ai[0] + (qi[0] - ai[0]) * t, ai[1] + (qi[1] - ai[1]) * t];
        m.beam_up([o[0], y - 0.0125, o[1]], [ii[0], y - 0.0125, ii[1]], 0.0048, 0.025, bar_c, Some([0.0, 1.0, 0.0]));
    }
    // cross bars along it, flush with the top.
    let width = {
        let mx = [(a[0] + q[0]) / 2.0, (a[1] + q[1]) / 2.0];
        let mi = [(ai[0] + qi[0]) / 2.0, (ai[1] + qi[1]) / 2.0];
        (mx[0] - mi[0]).hypot(mx[1] - mi[1])
    };
    let rows = (width / 0.102).floor() as usize;
    for r in 1..rows {
        let s = r as f64 / rows as f64;
        let p0 = [a[0] + (ai[0] - a[0]) * s, a[1] + (ai[1] - a[1]) * s];
        let p1 = [q[0] + (qi[0] - q[0]) * s, q[1] + (qi[1] - q[1]) * s];
        m.beam_up([p0[0], y - 0.004, p0[1]], [p1[0], y - 0.004, p1[1]], 0.006, 0.008, bar_c, Some([0.0, 1.0, 0.0]));
    }
    // the edge channels and their stiffeners.
    let m = b.m(steel_key);
    let (cx, cz) = ((a[0] + q[0] + ai[0] + qi[0]) / 4.0, (a[1] + q[1] + ai[1] + qi[1]) / 4.0);
    for (u, w) in [(a, q), (ai, qi)] {
        m.beam_up([u[0], y - 0.1, u[1]], [w[0], y - 0.1, w[1]], 0.06, 0.203, steel, None);
        let l = (w[0] - u[0]).hypot(w[1] - u[1]);
        let ribs = (l / 1.2).floor() as usize;
        for k in 1..ribs {
            let t = k as f64 / ribs as f64;
            let p = [u[0] + (w[0] - u[0]) * t, u[1] + (w[1] - u[1]) * t];
            // a plate on the channel's web, toward the walk's middle line.
            let mid = [cx + (q[0] - a[0]) * (t - 0.5), cz + (q[1] - a[1]) * (t - 0.5)];
            let dir = [mid[0] - p[0], mid[1] - p[1]];
            let dl = dir[0].hypot(dir[1]).max(1e-9);
            let q2 = [p[0] + dir[0] / dl * 0.09, p[1] + dir[1] / dl * 0.09];
            m.beam_up([q2[0], y - 0.2, q2[1]], [q2[0], y - 0.03, q2[1]], 0.01, 0.09, shade(steel, 0.9), Some([dir[0] / dl, 0.0, dir[1] / dl]));
        }
    }
}

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
    open_grating(&mut b, &hex, &inner, y, shade(white, 0.8));
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
    // towers.rs `city_hall`'s dome profile: radius max(lantern, rd cos(t pi/2)^0.85) at e_top + rise sin(t pi/2).
    let e_top = num(d, "drumTopM") + num(d, "entablatureM");
    let (rd, rise) = (num(d, "domeDiameterM") / 2.0, num(d, "domeRiseM"));
    let dome_y = |r: f64| {
        let c = (r / rd).clamp(0.0, 1.0).powf(1.0 / 0.85);
        y(e_top + rise * (1.0 - c * c).sqrt())
    };
    {
        let m = b.m("ch-stone");
        m.lathe(cx, cz, &[[r_out, floor], [r_in, floor]], 48, shade(stone, 0.92), None, None);
        // The gallery's entablature on the dome's crown (INFERRED: a
        // Beaux-Arts frieze, architrave, cyma and corona, after the drum's
        // own entablature, towers.rs): seated into the dome, its foot in
        // the junction's shade.
        let fy = floor;
        let foot = dome_y(r_out - 0.3) - 0.15;
        let prof = [
            [r_out - 0.3, foot],
            [r_out - 0.3, fy - 1.0],
            [r_out - 0.12, fy - 1.0],
            [r_out - 0.12, fy - 0.78],
            [r_out - 0.02, fy - 0.74],
            [r_out + 0.06, fy - 0.62],
            [r_out + 0.2, fy - 0.5],
            [r_out + 0.2, fy - 0.36],
            [r_out + 0.06, fy - 0.36],
            [r_out + 0.06, fy - 0.3],
            [r_out, fy - 0.3],
            [r_out, fy],
        ];
        let base_c = shade(stone, 0.86);
        m.lathe(cx, cz, &prof, 64, base_c, None, Some(&|k: usize, _y: f64| match k {
            0 => shade(base_c, 0.62),
            6 | 7 => shade(base_c, 0.78),
            _ => base_c,
        }));
        m.lathe(cx, cz, &[[r_in, fy - 0.4], [r_out - 0.3, fy - 0.4]], 48, shade(stone, 0.7), None, None);
        // The balustrade: plinth, turned vase balusters, the rail cap.
        let cap = |m: &mut MeshArrays, y0: f64, y1: f64, w: f64, c: Rgb| {
            m.lathe(cx, cz, &[[r_rail - w, y0], [r_rail + w, y0], [r_rail + w, y1], [r_rail - w, y1], [r_rail - w, y0]], 64, c, None, None);
        };
        cap(m, floor, floor + 0.16, 0.14, shade(stone, 0.85));
        cap(m, floor + GUARD_M - 0.12, floor + GUARD_M, 0.15, stone);
        // a double-bellied vase (INFERRED proportions after Vignola's
        // baluster: a square-ish foot, a belly at two fifths, a neck, a collar).
        let (b0, b1) = (floor + 0.16, floor + GUARD_M - 0.12);
        let hb = b1 - b0;
        let vase: Vec<[f64; 2]> = [[0.075, 0.0], [0.075, 0.06], [0.05, 0.1], [0.07, 0.2], [0.085, 0.36], [0.07, 0.52], [0.042, 0.68], [0.036, 0.8], [0.055, 0.86], [0.062, 0.92], [0.062, 1.0]].iter().map(|q| [q[0], b0 + q[1] * hb]).collect();
        let n = (TAU * r_rail / 0.3).round() as usize;
        let bal_c = shade(stone, 0.95);
        for k in 0..n {
            let a = k as f64 / n as f64 * TAU;
            m.lathe(cx + a.cos() * r_rail, cz - a.sin() * r_rail, &vase, 10, bal_c, None, None);
        }
    }
    for k in 0..4 {
        let a = k as f64 / 4.0 * TAU;
        b.lamp([cx + a.cos() * (r_in + 0.1), floor + 2.4, cz - a.sin() * (r_in + 0.1)], 400.0, 6.0);
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
