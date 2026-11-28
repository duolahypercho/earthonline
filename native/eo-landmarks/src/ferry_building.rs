//! The San Francisco Ferry Building (1896-1898, A. Page Brown) on the city
//! record it stands for (sf-building-558731934, OSM way 558731934), which
//! the native build drew as a plain box: no Blender is installed for the
//! hand-built pipeline and the earlier browser version's code-built module
//! was never ported (the record is in eo-kit's exclusions, so the kit does
//! not plan it either).
//!
//! Every measured value is read from `data/ferry-building.json`, which names
//! its source: the record's footprint (the sf-city record, OSM, ODbL), the
//! tower's stages from the OSM `building:part` ways (shaft to 43.5 m, the
//! belfry to 50.5 m, the lantern stages, the drum and dome to 70 m, the
//! flagstaff to 83.1 m), the 245 ft tower and the four 22 ft clock dials
//! (Wikipedia; NoeHill's landmark page for the hands), the 16.1 m parapet
//! (OSM part 24460886), the 22 nave arches per frontage (Wikipedia), and the
//! Colusa sandstone's colour (California Division of Mines Bulletin 191).
//! The `inferred` block is every proportion no published drawing gave
//! (storey subdivision, opening sizes, cornice profile, the roof monitor),
//! chosen to fit those measurements.
//!
//! The ground plan: the record's footprint is the scaled plan's (the kit's
//! rule: footprints follow `groundscale`, so the collision footprint and the
//! drawn walls agree), while every element (storeys, arches, windows,
//! cornices, the tower, the dials) stays real size; the frontage carries as
//! many bays of the surveyed bay pitch (the record's length / 22) as fit, so
//! at scale 1 it is the cited 22 per frontage. The tower is moved whole: its
//! surveyed centre on the scaled plan, real size.
//!
//! Construction: the walls are planes with their openings cut (round-arched
//! or flat), each opening with its reveal through the wall, a recessed pane
//! (glazing, a shop front, or a dark void for the belfry), an archivolt band
//! and a keystone; the ground floor of the long fronts is an open arcade with
//! a gallery behind it (ceiling, shop fronts on its back wall, a paved floor);
//! cornices, string courses, pilasters, paired columns, sills and copings are
//! boxes and lathes proud of the wall. Faces turned down or set back take a
//! baked sky occlusion in their vertex colour (the kit finish multiplies it).

use crate::geometry::{Frozen, MeshArrays, Rgb, V3};
use serde::Deserialize;
use std::f64::consts::PI;
use std::sync::OnceLock;

pub const FERRY_BUILDING_JSON: &str = include_str!("../data/ferry-building.json");

/// The city record the Ferry Building is drawn in place of, without its `sf-building-` prefix.
pub const RECORD: &str = "558731934";

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub id: String,
    pub north_west: [f64; 2],
    pub north_bay: [f64; 2],
    pub south_bay: [f64; 2],
    pub south_west: [f64; 2],
    /// The entrance pavilion's front: (west line, out, out, west line), north to south.
    pub pavilion: [[f64; 2]; 4],
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Stage {
    pub way: i64,
    pub top_m: f64,
    pub side_m: f64,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Round {
    pub top_m: f64,
    pub diameter_m: f64,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Clock {
    pub dial_diameter_m: f64,
    pub minute_hand_m: f64,
    pub hour_hand_m: f64,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Tower {
    pub centre_surveyed: [f64; 2],
    pub stages: Vec<Stage>,
    pub drum: Round,
    pub dome: Round,
    pub finial_top_m: f64,
    pub flagstaff_top_m: f64,
    pub clock: Clock,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Body {
    pub parapet_top_m: f64,
    pub arches_per_frontage: f64,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Stone {
    pub kit: String,
    pub tint_linear: [f64; 3],
}

/// The INFERRED proportions (no published elevation drawing).
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Inferred {
    pub plinth_m: f64,
    pub wall_m: f64,
    pub gallery_depth_m: f64,
    pub arcade_arch_width_m: f64,
    pub arcade_spring_m: f64,
    pub gallery_ceiling_m: f64,
    pub string_course: [f64; 2],
    pub upper_arch_width_m: f64,
    pub upper_sill_m: f64,
    pub upper_spring_m: f64,
    pub upper_reveal_m: f64,
    pub pilaster_width_m: f64,
    pub frieze: [f64; 2],
    pub cornice: [f64; 2],
    pub cornice_projection_m: f64,
    pub pavilion_parapet_top_m: f64,
    pub pavilion_arch_width_m: f64,
    pub pavilion_arch_spring_m: f64,
    pub pavilion_arch_pitch_m: f64,
    pub pavilion_upper_arch_width_m: f64,
    pub pavilion_upper_sill_m: f64,
    pub pavilion_upper_spring_m: f64,
    pub pavilion_cornice: [f64; 2],
    pub column_diameter_m: f64,
    pub roof_monitor_width_m: f64,
    pub roof_eave_m: f64,
    pub roof_monitor_foot_m: f64,
    pub roof_clerestory_top_m: f64,
    pub roof_ridge_m: f64,
    pub dial_centre_below_shaft_top_m: f64,
    pub clock_hour: f64,
}

/// The second pass's proportions and finishes (`detail` in the spec; INFERRED
/// unless named): drawn while `features::ferry_building_detail` is on.
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Detail {
    /// The ashlar scan's tile and its courses (polyhaven-sandstone), and the course it is drawn at.
    pub scan_tile_m: f64,
    pub scan_courses_per_tile: f64,
    pub ashlar_course_m: f64,
    /// The scan's variation kept (its wall is rough-dressed; the front's sandstone is rubbed smooth).
    pub ashlar_contrast: f64,
    pub upper_reveal_m: f64,
    /// A reveal's tone where it meets the glass (its face edge is 1): the sky it sees falls off with depth.
    pub reveal_deep_tone: f64,
    pub rustication_proud_m: f64,
    pub rustication_joint_m: f64,
    /// The rusticated piers' tone at their foot (splash and grime), rising to 1 over the lowest courses.
    pub rustication_ground_tone: f64,
    pub keystone_proud_m: f64,
    pub modillion_pitch_m: f64,
    pub modillion_width_m: f64,
    pub modillion_height_m: f64,
    pub crown_mould_m: f64,
    pub plinth_m: f64,
    pub plinth_proud_m: f64,
    pub plinth_linear: [f64; 3],
    /// The plinth's tone where it meets the paving (the contact occlusion), rising to 1 at its top.
    pub plinth_contact_tone: f64,
    /// Rain-washed dirt under the cornice and the sills: the darkest tone, fading to 1.
    pub weather_tone: f64,
    pub streak_length_m: f64,
    pub balustrade_m: f64,
    pub baluster_pitch_m: f64,
    pub baluster_diameter_m: f64,
    pub urn_m: f64,
    /// The gallery shops' lit interiors seen through their glass: the spread of their tone.
    pub shop_tone_spread: [f64; 2],
}

impl Detail {
    /// The ashlar scan's repeat scale: its course drawn at `ashlar_course_m`.
    pub fn ashlar_scale(&self) -> f64 {
        self.ashlar_course_m / (self.scan_tile_m / self.scan_courses_per_tile)
    }
}

/// The third pass (`relief` in the spec; INFERRED unless named): the facade's
/// relief read in light at any distance. Drawn while
/// `features::ferry_building_relief` is on (and the second pass).
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Relief {
    /// The wall's tone right under a cornice's bed mould (the sky the corona
    /// hides from it), rising to 1 over `cornice_occlusion_m`: the shadow line
    /// a cornice draws on its frieze in any light, kept at any distance.
    pub cornice_occlusion_tone: f64,
    pub cornice_occlusion_m: f64,
    /// A reveal's tone at the glass (in place of the second pass's `revealDeepTone`).
    pub reveal_deep_tone: f64,
    /// The glazing's roughness (float glass, nearly a mirror) and its frame:
    /// the perimeter frame's width and how far frame and bars stand off the glass.
    pub glass_roughness: f64,
    pub frame_width_m: f64,
    pub frame_proud_m: f64,
    /// Each light of a window its own room tone (the spread of its multiplier),
    /// a share of them with a pale blind or shade drawn behind the glass and its tone (linear).
    pub light_tone_spread: [f64; 2],
    pub blind_share: f64,
    pub blind_linear: [f64; 3],
    /// The archivolts' voussoirs: each stone's tone within this spread of the band's.
    pub voussoir_tone_spread: f64,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Spec {
    pub schema: String,
    pub sources: std::collections::BTreeMap<String, String>,
    pub record: Record,
    pub tower: Tower,
    pub body: Body,
    pub stone: Stone,
    pub inferred: Inferred,
    pub detail: Detail,
    pub relief: Relief,
}

/// The stone finish with the detail pass: the kit finish's scan at the
/// ashlar's course and the dressed face's evenness (`<kit>*<scale>~<contrast>`:
/// eo-render draws the finish with its texture repeat that many times larger
/// and its scan's variation around its mean times the contrast).
pub fn detail_stone_kit() -> &'static str {
    static K: OnceLock<String> = OnceLock::new();
    K.get_or_init(|| format!("{}*{:.3}~{:.3}", spec().stone.kit, spec().detail.ashlar_scale(), spec().detail.ashlar_contrast))
}

/// The spec compiled into the crate.
pub fn spec() -> &'static Spec {
    static S: OnceLock<Spec> = OnceLock::new();
    S.get_or_init(|| serde_json::from_str(FERRY_BUILDING_JSON).expect("eo-landmarks: data/ferry-building.json does not parse"))
}

/// One drawn part: a mesh in one finish.
pub struct Part {
    pub key: &'static str,
    /// The kit finish it is drawn in (`None`: its vertex colour, matte).
    pub kit: Option<&'static str>,
    pub cast: bool,
    /// Laid in the street surface's footway paving (the gallery floor).
    pub footway: bool,
    /// A vertex-colour part's roughness and metalness (unused under a kit finish).
    pub rough: f64,
    pub metal: f64,
    pub mesh: Frozen,
}

/// What `LandmarkSet::add_ferry_building` built, for the load trace.
#[derive(Clone, Debug)]
pub struct Report {
    pub base_y: f64,
    pub length_m: f64,
    pub depth_m: f64,
    pub bays: [usize; 3],
    pub triangles: usize,
    pub ms: f64,
}

pub struct Built {
    pub parts: Vec<Part>,
    /// The floor level the building stands on (the footway datum along its plaza front).
    pub base_y: f64,
    /// Length along the Embarcadero and depth, on the drawn (scaled) plan, m.
    pub length_m: f64,
    pub depth_m: f64,
    /// Bays per long frontage (plaza side wings, bay side).
    pub bays: [usize; 3],
    pub triangles: usize,
}

// --- small vector helpers -------------------------------------------------

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn mul(c: Rgb, k: f64) -> Rgb {
    [c[0] * k, c[1] * k, c[2] * k]
}

/// A wall's face in world terms: how far a world point stands out of it.
#[derive(Clone, Copy)]
struct WRef {
    o: V3,
    n: V3,
}

impl WRef {
    /// How far `p` stands out of the face (the wall's `d`).
    fn d(&self, p: V3) -> f64 {
        (p[0] - self.o[0]) * self.n[0] + (p[2] - self.o[2]) * self.n[2]
    }
}

/// A stable hash of a plan point to [0, 1) (bay-to-bay variation).
fn h01(x: f64, z: f64, salt: i64) -> f64 {
    let (a, b) = ((x * 10.0).round() as i64, (z * 10.0).round() as i64);
    let h = (a.wrapping_mul(73_856_093) ^ b.wrapping_mul(19_349_663) ^ salt.wrapping_mul(83_492_791)).rem_euclid(10_007);
    h as f64 / 10_007.0
}

fn smooth(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The sky a face sees, baked into its vertex colour: a face turned down
/// sees the ground and the building's own soffits (INFERRED factors).
fn sky(c: Rgb, n: V3) -> Rgb {
    if n[1] < -0.5 { mul(c, 0.62) } else if n[1] > 0.5 { c } else { mul(c, 0.94) }
}

/// A quad a-b-c-d (planar, in order round its edge) facing `want`.
fn quad(m: &mut MeshArrays, a: V3, b: V3, c: V3, d: V3, col: Rgb, want: V3) {
    let n = cross(sub(b, a), sub(d, a));
    let col = sky(col, want);
    if dot(n, want) >= 0.0 { m.quad(a, b, c, d, col) } else { m.quad(d, c, b, a, col) }
}

fn tri(m: &mut MeshArrays, a: V3, b: V3, c: V3, col: Rgb, want: V3) {
    let n = cross(sub(b, a), sub(c, a));
    let col = sky(col, want);
    if dot(n, want) >= 0.0 { m.tri(a, b, c, col) } else { m.tri(a, c, b, col) }
}

/// The building's plan frame: `u` along the Embarcadero front (north to
/// south), `v` toward the plaza (out of the west front), y up from the floor.
#[derive(Clone, Copy)]
struct Frame {
    o: [f64; 2],
    u: [f64; 2],
    n: [f64; 2],
    base: f64,
}

impl Frame {
    fn at(&self, u: f64, v: f64, y: f64) -> V3 {
        [self.o[0] + self.u[0] * u + self.n[0] * v, self.base + y, self.o[1] + self.u[1] * u + self.n[1] * v]
    }
    /// A plan direction (du, dv) and a vertical component, in world axes.
    fn dir(&self, du: f64, dv: f64, dy: f64) -> V3 {
        [self.u[0] * du + self.n[0] * dv, dy, self.u[1] * du + self.n[1] * dv]
    }
    /// A world plan point in frame (u, v).
    fn local(&self, p: [f64; 2]) -> [f64; 2] {
        let d = [p[0] - self.o[0], p[1] - self.o[1]];
        [d[0] * self.u[0] + d[1] * self.u[1], d[0] * self.n[0] + d[1] * self.n[1]]
    }
}

/// A wall plane in the frame: from `a`, along unit `t`, facing unit `n` (both frame (u, v)).
#[derive(Clone, Copy)]
struct Wall {
    a: [f64; 2],
    t: [f64; 2],
    n: [f64; 2],
    len: f64,
}

impl Wall {
    fn new(a: [f64; 2], b: [f64; 2], n: [f64; 2]) -> Self {
        let len = (b[0] - a[0]).hypot(b[1] - a[1]);
        Self { a, t: [(b[0] - a[0]) / len, (b[1] - a[1]) / len], n, len }
    }
}

/// A wall opening: centre along the wall, width, sill and spring heights, round-arched or flat.
#[derive(Clone, Copy, Debug)]
struct Op {
    s: f64,
    w: f64,
    sill: f64,
    spring: f64,
    arch: bool,
}

impl Op {
    fn r(&self) -> f64 {
        self.w / 2.0
    }
    fn top(&self) -> f64 {
        if self.arch { self.spring + self.r() } else { self.spring }
    }
    fn l(&self) -> f64 {
        self.s - self.r()
    }
    fn rt(&self) -> f64 {
        self.s + self.r()
    }
}

/// Segments of a semicircular arch (even).
const ARC: usize = 14;

/// What lies behind an opening.
#[derive(Clone, Copy, PartialEq)]
enum Back {
    /// Glazing at the reveal's depth, with mullions.
    Glass,
    /// A shop front at the reveal's depth (door, transom).
    Shop,
    /// A dark recess (the belfry's voids).
    Void,
    /// Nothing: the opening goes through (an arcade into its gallery).
    Open,
}

/// The finishes.
#[derive(Clone, Copy)]
enum F {
    Stone,
    Trim,
    Plinth,
    Glass,
    Shop,
    Frame,
    Roof,
    Skylight,
    Dial,
    Dark,
    Floor,
}

/// key, kit finish (kit-material-specs.json), casts, footway, and a vertex-colour part's roughness and metalness.
type PartSpec = (&'static str, Option<&'static str>, bool, bool, f64, f64);
const PARTS: [PartSpec; 11] = [
    ("stone", None, true, false, 0.0, 0.0), // the spec's kit (limestone-ashlar), set below
    ("trim", Some("cast-stone-trim"), true, false, 0.0, 0.0),
    ("plinth", Some("cast-stone"), true, false, 0.0, 0.0),
    // the upper windows: float glass over a dark interior, a dielectric
    // (F0 4 %, n = 1.52) whose reflection is its Fresnel term, nearly smooth.
    ("glass", None, false, false, 0.08, 0.0),
    // the shop fronts in the galleries: the same glass over lit shops.
    ("shop", None, false, false, 0.08, 0.0),
    ("frame", Some("painted-steel"), true, false, 0.0, 0.0),
    // the low-slope roofs either side of the monitor (a dark built-up roof, INFERRED).
    ("roof", Some("asphalt-roof"), true, false, 0.0, 0.0),
    // the nave's skylight: glass over the lit nave, as the windows'.
    ("skylight", None, true, false, 0.08, 0.0),
    ("dial", Some("painted-white"), false, false, 0.0, 0.0),
    ("dark", None, false, false, 0.95, 0.0),
    ("floor", None, false, true, 0.9, 0.0),
];

struct B {
    f: Frame,
    m: Vec<MeshArrays>,
    stone: Rgb,
    /// The second pass (None: the first pass, `--no-ferry-building-detail`).
    dt: Option<Detail>,
    /// The third pass (None: the second pass's output, `--no-facade-relief`).
    rl: Option<Relief>,
}

impl B {
    fn mesh(&mut self, k: F) -> &mut MeshArrays {
        &mut self.m[k as usize]
    }
    /// The vertex count of a finish's mesh (a mark for `shade_from`).
    fn mark(&self, k: F) -> usize {
        self.m[k as usize].positions.len() / 3
    }
    /// Multiply the colour of every vertex of finish `k` pushed since `from` by `f(world position)`.
    fn shade_from(&mut self, k: F, from: usize, f: &dyn Fn(V3) -> f64) {
        let m = &mut self.m[k as usize];
        for v in from..m.positions.len() / 3 {
            let s = f([m.positions[3 * v], m.positions[3 * v + 1], m.positions[3 * v + 2]]);
            for c in 0..3 {
                m.colors[3 * v + c] *= s;
            }
        }
    }
    /// Wall `w`'s face in world terms, to read a world point's (s, y, d) back.
    fn wref(&self, w: &Wall) -> WRef {
        WRef { o: self.p(w, 0.0, 0.0, 0.0), n: self.wn(w, 1.0) }
    }
    /// The lit shop interior's tone multiplier of a bay at `s` on `w` (1 with the first pass).
    fn shop_tone(&self, w: &Wall, s: f64) -> f64 {
        match &self.dt {
            Some(d) => {
                let c = self.p(w, s, 0.0, 0.0);
                let [lo, hi] = d.shop_tone_spread;
                lo + (hi - lo) * h01(c[0], c[2], 7)
            }
            None => 1.0,
        }
    }
    fn tone(&self, k: F) -> Rgb {
        match k {
            F::Stone => self.stone,
            // the trim in the stone's tone over its smooth bake.
            F::Trim => [self.stone[0] * 1.02, self.stone[1] * 1.02, self.stone[2] * 1.02],
            // the granite-grey base course (INFERRED: darker than the ashlar above; the second pass's own tone).
            F::Plinth => self.dt.as_ref().map(|d| d.plinth_linear).unwrap_or([0.62, 0.62, 0.63]),
            F::Dark => [0.012, 0.012, 0.013],
            // the room behind the glass in daylight (INFERRED: a dim interior seen through it).
            F::Glass => [0.028, 0.031, 0.034],
            F::Shop => [0.05, 0.045, 0.037],
            F::Skylight => [0.06, 0.065, 0.07],
            // the gallery floor's paving tone (multiplies the street's concrete).
            F::Floor => [0.62, 0.6, 0.57],
            _ => [1.0; 3],
        }
    }
    /// A point on wall `w`: `s` along it, height `y`, `d` out of its face.
    fn p(&self, w: &Wall, s: f64, y: f64, d: f64) -> V3 {
        self.f.at(w.a[0] + w.t[0] * s + w.n[0] * d, w.a[1] + w.t[1] * s + w.n[1] * d, y)
    }
    fn wn(&self, w: &Wall, k: f64) -> V3 {
        self.f.dir(w.n[0] * k, w.n[1] * k, 0.0)
    }
    fn wt(&self, w: &Wall, k: f64) -> V3 {
        self.f.dir(w.t[0] * k, w.t[1] * k, 0.0)
    }

    /// A box on wall `w` (s0..s1 along, y0..y1, d0..d1 out of the face); `back`: draw its face toward the wall.
    #[allow(clippy::too_many_arguments)]
    fn wbox(&mut self, k: F, w: &Wall, s: [f64; 2], y: [f64; 2], d: [f64; 2], back: bool, bottom: bool, tone: f64) {
        let col = mul(self.tone(k), tone);
        let c = |b: &B, i: usize, j: usize, l: usize| b.p(w, s[i], y[j], d[l]);
        let pts: Vec<V3> = (0..8).map(|q| c(self, q & 1, (q >> 1) & 1, (q >> 2) & 1)).collect();
        let (nt, nn, up) = (self.wt(w, 1.0), self.wn(w, 1.0), [0.0, 1.0, 0.0]);
        let neg = |v: V3| [-v[0], -v[1], -v[2]];
        let m = &mut self.m[k as usize];
        // index bits: s (1), y (2), d (4)
        quad(m, pts[4], pts[5], pts[7], pts[6], col, nn); // front (d1)
        if back {
            quad(m, pts[0], pts[1], pts[3], pts[2], col, neg(nn));
        }
        quad(m, pts[1], pts[5], pts[7], pts[3], col, nt); // s1 end
        quad(m, pts[0], pts[4], pts[6], pts[2], col, neg(nt)); // s0 end
        quad(m, pts[2], pts[3], pts[7], pts[6], col, up); // top
        if bottom {
            quad(m, pts[0], pts[1], pts[5], pts[4], col, neg(up));
        }
    }

    /// The face of wall `w` at offset `d`, facing `sign` x its normal, over
    /// s0..s1 and y0..y1, with `ops` cut out (each inside y0..y1).
    #[allow(clippy::too_many_arguments)]
    fn panel(&mut self, k: F, w: &Wall, s: [f64; 2], y: [f64; 2], d: f64, sign: f64, ops: &[Op], tone: f64) {
        let col = mul(self.tone(k), tone);
        let want = self.wn(w, sign);
        let mut ops: Vec<Op> = ops.iter().copied().filter(|o| o.rt() > s[0] && o.l() < s[1]).collect();
        ops.sort_by(|a, b| a.s.total_cmp(&b.s));
        let mut cur = s[0];
        let q = |b: &B, s0: f64, s1: f64, y0: f64, y1: f64| [b.p(w, s0, y0, d), b.p(w, s1, y0, d), b.p(w, s1, y1, d), b.p(w, s0, y1, d)];
        let mut quads: Vec<[V3; 4]> = Vec::new();
        let mut tris: Vec<[V3; 3]> = Vec::new();
        for o in &ops {
            let (l, r) = (o.l().max(s[0]), o.rt().min(s[1]));
            if l > cur + 1e-6 {
                quads.push(q(self, cur, l, y[0], y[1]));
            }
            if o.sill > y[0] + 1e-6 {
                quads.push(q(self, l, r, y[0], o.sill));
            }
            if y[1] > o.top() + 1e-6 {
                quads.push(q(self, l, r, o.top(), y[1]));
            }
            if o.arch {
                // the spandrels: each half fanned from its upper corner.
                let pt = |b: &B, i: usize| {
                    let th = PI - PI * i as f64 / ARC as f64;
                    b.p(w, o.s + o.r() * th.cos(), o.spring + o.r() * th.sin(), d)
                };
                let (cl, cr) = (self.p(w, o.l(), o.top(), d), self.p(w, o.rt(), o.top(), d));
                for i in 0..ARC {
                    let corner = if i < ARC / 2 { cl } else { cr };
                    tris.push([corner, pt(self, i), pt(self, i + 1)]);
                }
            }
            cur = r;
        }
        if s[1] > cur + 1e-6 {
            quads.push(q(self, cur, s[1], y[0], y[1]));
        }
        let m = &mut self.m[k as usize];
        for [a, b, c, e] in quads {
            quad(m, a, b, c, e, col, want);
        }
        for [a, b, c] in tris {
            tri(m, a, b, c, col, want);
        }
    }

    /// `panel` in horizontal bands split at `splits`: openings stacked over
    /// one another (an arcade arch under an upper window) each go to the band
    /// holding their middle, so no band's openings share a column.
    #[allow(clippy::too_many_arguments)]
    fn panel_bands(&mut self, k: F, w: &Wall, s: [f64; 2], y: [f64; 2], splits: &[f64], d: f64, ops: &[Op], tone: f64) {
        let mut edges = vec![y[0]];
        edges.extend(splits.iter().copied().filter(|v| *v > y[0] && *v < y[1]));
        edges.push(y[1]);
        for b in edges.windows(2) {
            let mine: Vec<Op> = ops.iter().copied().filter(|o| {
                let m = (o.sill + o.top()) / 2.0;
                m >= b[0] && m < b[1]
            }).collect();
            debug_assert!(mine.iter().all(|o| o.sill >= b[0] - 1e-6 && o.top() <= b[1] + 1e-6), "an opening crosses a band edge");
            self.panel(k, w, s, [b[0], b[1]], d, 1.0, &mine, tone);
        }
    }

    /// An opening's reveal through the wall from d0 (face) to d1 (back): its
    /// jambs, the arch soffit or the lintel, and the sill when `sill`.
    #[allow(clippy::too_many_arguments)]
    fn reveal(&mut self, k: F, w: &Wall, o: &Op, d0: f64, d1: f64, sill: bool, tone: f64) {
        let col = mul(self.tone(k), tone);
        let (nt, up) = (self.wt(w, 1.0), [0.0, 1.0, 0.0]);
        let jl = [self.p(w, o.l(), o.sill, d0), self.p(w, o.l(), o.spring, d0), self.p(w, o.l(), o.spring, d1), self.p(w, o.l(), o.sill, d1)];
        let jr = [self.p(w, o.rt(), o.sill, d0), self.p(w, o.rt(), o.spring, d0), self.p(w, o.rt(), o.spring, d1), self.p(w, o.rt(), o.sill, d1)];
        let sl = [self.p(w, o.l(), o.sill, d0), self.p(w, o.rt(), o.sill, d0), self.p(w, o.rt(), o.sill, d1), self.p(w, o.l(), o.sill, d1)];
        let mut arc: Vec<(V3, V3, V3)> = Vec::new();
        if o.arch {
            for i in 0..ARC {
                let (a0, a1) = (PI - PI * i as f64 / ARC as f64, PI - PI * (i + 1) as f64 / ARC as f64);
                let at = |b: &B, th: f64, d: f64| b.p(w, o.s + o.r() * th.cos(), o.spring + o.r() * th.sin(), d);
                let mid = (a0 + a1) / 2.0;
                let inward = {
                    let t = self.wt(w, -mid.cos());
                    [t[0], -mid.sin(), t[2]]
                };
                arc.push((at(self, a0, d0), at(self, a1, d0), inward));
                arc.push((at(self, a1, d1), at(self, a0, d1), inward));
            }
        }
        let lintel = [self.p(w, o.l(), o.spring, d0), self.p(w, o.rt(), o.spring, d0), self.p(w, o.rt(), o.spring, d1), self.p(w, o.l(), o.spring, d1)];
        let m = &mut self.m[k as usize];
        quad(m, jl[0], jl[1], jl[2], jl[3], col, nt);
        quad(m, jr[0], jr[1], jr[2], jr[3], col, [-nt[0], -nt[1], -nt[2]]);
        if sill {
            quad(m, sl[0], sl[1], sl[2], sl[3], col, up);
        }
        if o.arch {
            for pair in arc.chunks_exact(2) {
                let ((a, b, n), (c, e, _)) = (pair[0], pair[1]);
                quad(m, a, b, c, e, mul(col, 0.92), n);
            }
        } else {
            quad(m, lintel[0], lintel[1], lintel[2], lintel[3], col, [0.0, -1.0, 0.0]);
        }
    }

    /// The glazing of a window with the third pass: one pane per light (the
    /// mullions' quarter columns and their rails; the fanlight's three sectors
    /// between its radial bars), each its own room tone or drawn linen blind,
    /// plus subtle per-pane hand-set glass tilt so sky and cloud reflections break
    /// up realistically across the arched lights instead of reading as a flat black sheet.
    fn glass_lights(&mut self, w: &Wall, o: &Op, d: f64, rl: &Relief) {
        let want = self.wn(w, 1.0);
        let wt = self.wt(w, 1.0);
        let base = self.tone(F::Glass);
        let c = self.p(w, o.s, o.spring, 0.0);
        let pick = |i: i64| -> Rgb {
            let h = h01(c[0], c[2], 101 + i);
            if h < rl.blind_share {
                rl.blind_linear
            } else {
                let t = h01(c[0], c[2], 211 + 3 * i);
                mul(base, rl.light_tone_spread[0] + (rl.light_tone_spread[1] - rl.light_tone_spread[0]) * t)
            }
        };
        let pane_nrm = |i: i64| -> V3 {
            let tx = (h01(c[0], c[2], 521 + 7 * i) - 0.5) * 0.04;
            let ty = (h01(c[0], c[2], 643 + 11 * i) - 0.5) * 0.04 + 0.015;
            let nx = want[0] + wt[0] * tx;
            let ny = want[1] + ty;
            let nz = want[2] + wt[2] * tx;
            let l = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-9);
            [nx / l, ny / l, nz / l]
        };
        let cols = [o.l(), o.s - o.w / 4.0, o.s, o.s + o.w / 4.0, o.rt()];
        let n = ((o.spring - o.sill) / 0.95).round().max(1.0) as usize;
        let mut quads: Vec<([V3; 4], Rgb, V3)> = Vec::new();
        let mut tris: Vec<([V3; 3], Rgb, V3)> = Vec::new();
        let mut i = 0;
        for r in 0..n {
            let (y0, y1) = (o.sill + (o.spring - o.sill) * r as f64 / n as f64, o.sill + (o.spring - o.sill) * (r + 1) as f64 / n as f64);
            for cc in cols.windows(2) {
                let q = [self.p(w, cc[0], y0, d), self.p(w, cc[1], y0, d), self.p(w, cc[1], y1, d), self.p(w, cc[0], y1, d)];
                quads.push((q, pick(i), pane_nrm(i)));
                i += 1;
            }
        }
        if o.arch {
            let ctr = self.p(w, o.s, o.spring, d);
            for k in 0..ARC {
                // the radial bars at 45 and 135 degrees split the fan in three.
                let sector = if 4 * k < ARC { 0 } else if 4 * k < 3 * ARC { 1 } else { 2 };
                let col = pick(100 + sector);
                let nrm = pane_nrm(100 + sector);
                let at = |b: &B, j: usize| {
                    let th = PI - PI * j as f64 / ARC as f64;
                    b.p(w, o.s + o.r() * th.cos(), o.spring + o.r() * th.sin(), d)
                };
                tris.push(([ctr, at(self, k), at(self, k + 1)], col, nrm));
            }
        }
        let m = self.mesh(F::Glass);
        for (q, col, nrm) in quads {
            let gn = cross(sub(q[1], q[0]), sub(q[3], q[0]));
            let col = sky(col, want);
            if dot(gn, want) >= 0.0 {
                m.quad_n(q[0], q[1], q[2], q[3], col, nrm);
            } else {
                m.quad_n(q[3], q[2], q[1], q[0], col, nrm);
            }
        }
        for (t, col, nrm) in tris {
            let gn = cross(sub(t[1], t[0]), sub(t[2], t[0]));
            let col = sky(col, want);
            let (a, b, c) = if dot(gn, want) >= 0.0 { (t[0], t[1], t[2]) } else { (t[0], t[2], t[1]) };
            let (ia, ib, ic) = (m.vertex(a, nrm, col), m.vertex(b, nrm, col), m.vertex(c, nrm, col));
            m.tri_idx(ia, ib, ic);
        }
    }

    /// The opening's own shape as a flat pane at `d` facing out (rectangle and half-disc fan).
    fn pane(&mut self, k: F, w: &Wall, o: &Op, d: f64, tone: f64) {
        if let (F::Glass, Some(rl)) = (k, self.rl.clone()) {
            if (tone - 1.0).abs() < 1e-9 && o.spring - o.sill > 0.5 && o.w > 1.2 {
                self.glass_lights(w, o, d, &rl);
                return;
            }
        }
        let col = mul(self.tone(k), tone);
        let want = self.wn(w, 1.0);
        let r = [self.p(w, o.l(), o.sill, d), self.p(w, o.rt(), o.sill, d), self.p(w, o.rt(), o.spring, d), self.p(w, o.l(), o.spring, d)];
        let c = self.p(w, o.s, o.spring, d);
        let pts: Vec<V3> = (0..=ARC).map(|i| {
            let th = PI - PI * i as f64 / ARC as f64;
            self.p(w, o.s + o.r() * th.cos(), o.spring + o.r() * th.sin(), d)
        }).collect();
        let m = &mut self.m[k as usize];
        quad(m, r[0], r[1], r[2], r[3], col, want);
        if o.arch {
            for i in 0..ARC {
                tri(m, c, pts[i], pts[i + 1], col, want);
            }
        }
    }

    /// The band round an arch (`band` wide, `proud` out of the face at `d`) and its keystone.
    fn archivolt(&mut self, w: &Wall, o: &Op, band: f64, proud: f64, d: f64) {
        if !o.arch {
            return;
        }
        let col = self.tone(F::Trim);
        let want = self.wn(w, 1.0);
        let (r0, r1) = (o.r(), o.r() + band);
        let mut faces: Vec<([V3; 4], V3)> = Vec::new();
        // with the third pass each voussoir (one per arc step) its own stone's tone.
        let vt: Vec<f64> = {
            let c = self.p(w, o.s, o.spring, 0.0);
            let spread = self.rl.as_ref().map(|r| r.voussoir_tone_spread).unwrap_or(0.0);
            (0..ARC).map(|i| 1.0 + spread * (h01(c[0], c[2], 300 + i as i64) - 0.5) * 2.0 - if i % 2 == 1 { spread * 0.5 } else { 0.0 }).collect()
        };
        for i in 0..ARC {
            let (a0, a1) = (PI - PI * i as f64 / ARC as f64, PI - PI * (i + 1) as f64 / ARC as f64);
            let at = |b: &B, th: f64, r: f64, dd: f64| b.p(w, o.s + r * th.cos(), o.spring + r * th.sin(), dd);
            faces.push(([at(self, a0, r0, d + proud), at(self, a1, r0, d + proud), at(self, a1, r1, d + proud), at(self, a0, r1, d + proud)], want));
            let mid = (a0 + a1) / 2.0;
            let out = {
                let t = self.wt(w, mid.cos());
                [t[0], mid.sin(), t[2]]
            };
            faces.push(([at(self, a0, r1, d), at(self, a1, r1, d), at(self, a1, r1, d + proud), at(self, a0, r1, d + proud)], out));
            faces.push(([at(self, a0, r0, d), at(self, a1, r0, d), at(self, a1, r0, d + proud), at(self, a0, r0, d + proud)], [-out[0], -out[1], -out[2]]));
        }
        let m = &mut self.m[F::Trim as usize];
        for (j, ([a, b, c, e], n)) in faces.into_iter().enumerate() {
            quad(m, a, b, c, e, mul(col, vt[j / 3]), n);
        }
        // the keystone, and the impost blocks the band springs from.
        let kw = (o.w * 0.11).clamp(0.3, 0.7);
        match self.dt.as_ref().map(|t| t.keystone_proud_m) {
            Some(kp) => {
                // a projecting keystone that casts its own shadow: its foot in the
                // band, its head over it, a capping slab a little wider on top.
                self.wbox(F::Trim, w, [o.s - kw / 2.0, o.s + kw / 2.0], [o.top() - band * 0.7, o.top() + band * 1.5], [d, d + kp], false, true, 1.0);
                self.wbox(F::Trim, w, [o.s - kw / 2.0 - 0.05, o.s + kw / 2.0 + 0.05], [o.top() + band * 1.5, o.top() + band * 1.5 + 0.1], [d, d + kp + 0.04], false, true, 1.0);
            }
            None => self.wbox(F::Trim, w, [o.s - kw / 2.0, o.s + kw / 2.0], [o.top() - band * 0.4, o.top() + band * 1.25], [d, d + proud + 0.06], false, true, 1.0),
        }
        for s in [o.l() - band, o.rt()] {
            self.wbox(F::Trim, w, [s, s + band], [o.spring - band * 0.7, o.spring], [d, d + proud], false, true, 1.0);
        }
    }

    /// Mullions and transoms of a glazed or shop opening at the pane's depth `d`.
    fn mullions(&mut self, w: &Wall, o: &Op, d: f64, shop: bool) {
        let proud = self.rl.as_ref().map(|r| r.frame_proud_m).unwrap_or(0.07);
        let fw = self.rl.as_ref().map(|r| r.frame_width_m).unwrap_or(0.1);
        let (bw, dd) = (0.08, [d, d + proud]);
        // divided lights (INFERRED: quarter-width lights about a metre tall); a shop front's door and side lights.
        let verticals = if shop { vec![o.s - o.w * 0.32, o.s, o.s + o.w * 0.32] } else { vec![o.s - o.w / 4.0, o.s, o.s + o.w / 4.0] };
        let arc_y = |x: f64| if o.arch { o.spring + (o.r() * o.r() - (x - o.s).powi(2)).max(0.0).sqrt() } else { o.spring };
        for x in verticals {
            let top = arc_y(x);
            self.wbox(F::Frame, w, [x - bw / 2.0, x + bw / 2.0], [o.sill, top - 0.02], dd, false, false, 1.0);
        }
        let rails: Vec<f64> = if shop {
            vec![o.sill + 2.6, o.spring]
        } else {
            let n = ((o.spring - o.sill) / 0.95).round().max(1.0) as usize;
            (1..=n).map(|k| o.sill + (o.spring - o.sill) * k as f64 / n as f64).collect()
        };
        for y in rails {
            if y > o.sill + 0.1 && y <= o.spring + 1e-6 {
                self.wbox(F::Frame, w, [o.l(), o.rt()], [y - bw / 2.0, y + bw / 2.0], dd, false, true, 1.0);
            }
        }
        // perimeter frame
        self.wbox(F::Frame, w, [o.l(), o.l() + fw], [o.sill, o.spring], dd, false, false, 1.0);
        self.wbox(F::Frame, w, [o.rt() - fw, o.rt()], [o.sill, o.spring], dd, false, false, 1.0);
        if self.rl.is_some() {
            // the sill rail of the frame, and the frame round the fanlight's curve.
            self.wbox(F::Frame, w, [o.l(), o.rt()], [o.sill, o.sill + fw], dd, false, true, 1.0);
            if o.arch {
                let col = self.tone(F::Frame);
                let up = self.wn(w, 1.0);
                for i in 0..ARC {
                    let at = |b: &B, j: usize, r: f64| {
                        let th = PI - PI * j as f64 / ARC as f64;
                        b.p(w, o.s + r * th.cos(), o.spring + r * th.sin(), d + proud / 2.0)
                    };
                    let (a, c) = (at(self, i, o.r() - fw / 2.0), at(self, i + 1, o.r() - fw / 2.0));
                    self.mesh(F::Frame).beam_up(a, c, fw, proud, col, Some(up));
                }
            }
        }
        if o.arch {
            // two radial bars in the fanlight
            for th in [PI / 4.0, 3.0 * PI / 4.0] {
                let a = self.p(w, o.s, o.spring, d + 0.035);
                let b = self.p(w, o.s + o.r() * th.cos(), o.spring + o.r() * th.sin(), d + 0.035);
                let up = self.wn(w, 1.0);
                let col = self.tone(F::Frame);
                self.mesh(F::Frame).beam_up(a, b, bw, 0.07, col, Some(up));
            }
        }
    }

    /// An opening complete: reveal, what is behind it, archivolt and sill trim.
    #[allow(clippy::too_many_arguments)]
    fn opening(&mut self, w: &Wall, o: &Op, depth: f64, back: Back, band: f64, sill_trim: bool, reveal_sill: bool) {
        let at = self.mark(F::Stone);
        self.reveal(F::Stone, w, o, 0.0, -depth, reveal_sill, 0.8);
        if let Some(deep) = self.rl.as_ref().map(|r| r.reveal_deep_tone).or(self.dt.as_ref().map(|d| d.reveal_deep_tone)) {
            // the sky a reveal sees falls off into the wall: darkest at the glass.
            let r = self.wref(w);
            self.shade_from(F::Stone, at, &move |p| 1.0 - (1.0 - deep) * smooth(-r.d(p) / depth));
        }
        match back {
            Back::Glass => {
                self.pane(F::Glass, w, o, -depth, 1.0);
                self.mullions(w, o, -depth, false);
            }
            Back::Shop => {
                let k = self.shop_tone(w, o.s);
                self.pane(F::Shop, w, o, -depth, k);
                self.mullions(w, o, -depth, true);
            }
            Back::Void => self.pane(F::Dark, w, o, -depth, 1.0),
            Back::Open => {}
        }
        if band > 0.0 {
            self.archivolt(w, o, band, 0.09, 0.0);
        }
        if sill_trim {
            self.wbox(F::Trim, w, [o.l() - 0.18, o.rt() + 0.18], [o.sill - 0.16, o.sill], [0.0, 0.16], false, true, 1.0);
        }
    }
}

/// The second pass's pieces (each a no-op without `dt`).
impl B {
    /// The base course: granite-grey, proud of the wall, darkest where it meets
    /// the paving (the contact occlusion) and with a weathered top course;
    /// over `spans` along `w` (the wall between its ground openings).
    fn plinth(&mut self, w: &Wall, spans: &[[f64; 2]], foot: f64, first: (f64, f64)) {
        let Some(d) = self.dt.clone() else {
            for s in spans {
                self.wbox(F::Plinth, w, *s, [foot, first.0], [0.0, first.1], false, false, 1.0);
            }
            return;
        };
        let (h, pr) = (d.plinth_m, d.plinth_proud_m);
        let at = self.mark(F::Plinth);
        for s in spans {
            // below the paving (its walls go down past the lowest ground) and the visible course
            self.wbox(F::Plinth, w, *s, [foot, 0.0], [0.0, pr], false, false, 1.0);
            self.wbox(F::Plinth, w, *s, [0.0, h - 0.1], [0.0, pr], false, false, 1.0);
            self.wbox(F::Plinth, w, [s[0] - 0.03, s[1] + 0.03], [h - 0.1, h], [0.0, pr + 0.04], false, true, 1.0);
        }
        let base = self.f.base;
        let c = d.plinth_contact_tone;
        self.shade_from(F::Plinth, at, &move |p| c + (1.0 - c) * smooth((p[1] - base) / (h * 0.8)));
    }

    /// Banded rustication on the ground storey's piers: courses of
    /// `ashlar_course_m` proud of the wall, a channel joint between each, from
    /// the plinth up to `top`, on `spans` along `w`; the lowest courses carry the street's splash.
    fn rustication(&mut self, w: &Wall, spans: &[[f64; 2]], top: f64) {
        let Some(d) = self.dt.clone() else { return };
        let y0 = d.plinth_m;
        let n = ((top - y0) / d.ashlar_course_m).round().max(1.0) as usize;
        let c = (top - y0) / n as f64;
        let at = self.mark(F::Stone);
        for s in spans {
            if s[1] - s[0] < 0.3 {
                continue;
            }
            for k in 0..n {
                let (a, b) = (y0 + c * k as f64 + d.rustication_joint_m / 2.0, y0 + c * (k + 1) as f64 - d.rustication_joint_m / 2.0);
                self.wbox(F::Stone, w, *s, [a, b.min(top)], [0.0, d.rustication_proud_m], false, true, 1.0);
            }
        }
        let base = self.f.base;
        let g = d.rustication_ground_tone;
        self.shade_from(F::Stone, at, &move |p| g + (1.0 - g) * smooth((p[1] - base - y0) / 1.6));
    }

    /// Modillions under a cornice's corona (`soffit`, projecting `proj`) from
    /// the bed mould (`bed` out) along `w` over `s`, and a crown moulding on
    /// the corona at `crown_y`.
    fn modillions(&mut self, w: &Wall, s: [f64; 2], soffit: f64, bed: f64, proj: f64) {
        let Some(d) = self.dt.clone() else { return };
        let n = ((s[1] - s[0]) / d.modillion_pitch_m).floor().max(1.0) as usize;
        let p = (s[1] - s[0]) / n as f64;
        for k in 0..n {
            let c = s[0] + p * (k as f64 + 0.5);
            let hw = d.modillion_width_m / 2.0;
            // the bracket and its scrolled foot (a smaller block under its front)
            self.wbox(F::Trim, w, [c - hw, c + hw], [soffit - d.modillion_height_m * 0.55, soffit], [bed, proj - 0.1], false, true, 1.0);
            self.wbox(F::Trim, w, [c - hw * 0.8, c + hw * 0.8], [soffit - d.modillion_height_m, soffit - d.modillion_height_m * 0.55], [bed, bed + 0.16], false, true, 1.0);
        }
    }

    /// The cyma on a corona over `s` at `y`: a lip that throws a second shadow line.
    fn crown(&mut self, w: &Wall, s: [f64; 2], y: f64, proj: f64) {
        let Some(d) = self.dt.clone() else { return };
        self.wbox(F::Trim, w, s, [y, y + d.crown_mould_m], [-0.3, proj + 0.06], false, true, 1.0);
    }

    /// Rain-washed dirt as stain sheets a few millimetres proud of a face, in
    /// the face's own finish (the scan carries on across them, only darker):
    /// a band on `w` over `s` from `y_top` (darkest, `weather_tone`) fading out
    /// down to `y_bot`, at `d` out of the wall.
    fn stain(&mut self, k: F, w: &Wall, s: [f64; 2], y_top: f64, y_bot: f64, d: f64, strength: f64) {
        let Some(dt) = self.dt.clone() else { return };
        if !(s[1] - s[0] > 0.02 && y_top - y_bot > 0.02) {
            return;
        }
        let dark = 1.0 - (1.0 - dt.weather_tone) * strength;
        let col = self.tone(k);
        let want = self.wn(w, 1.0);
        let d = d + 0.004;
        let q = [self.p(w, s[0], y_bot, d), self.p(w, s[1], y_bot, d), self.p(w, s[1], y_top, d), self.p(w, s[0], y_top, d)];
        let m = self.mesh(k);
        let v = m.positions.len() / 3;
        quad(m, q[0], q[1], q[2], q[3], col, want);
        // the two vertices at y_top darker than the two at y_bot
        for i in v..v + 4 {
            if (m.positions[3 * i + 1] - q[2][1]).abs() < 1e-6 {
                for c in 0..3 {
                    m.colors[3 * i + c] *= dark;
                }
            }
        }
    }

    /// The cornice's occlusion on the wall under it with the third pass (in place
    /// of the second pass's dirt sheet there, `stain`): darkest under the bed
    /// mould at `cornice_occlusion_tone`, rising to the dirt's own tone over
    /// `cornice_occlusion_m` and to 1 at `y_bot`. Without the third pass, `stain`.
    fn under_cornice(&mut self, w: &Wall, s: [f64; 2], y_top: f64, y_bot: f64) {
        let (Some(dt), Some(rl)) = (self.dt.clone(), self.rl.clone()) else {
            self.stain(F::Stone, w, s, y_top, y_bot, 0.0, 1.0);
            return;
        };
        if !(s[1] - s[0] > 0.02 && y_top - y_bot > 0.02) {
            return;
        }
        let col = self.tone(F::Stone);
        let want = self.wn(w, 1.0);
        let d = 0.004;
        let y_mid = (y_top - rl.cornice_occlusion_m).max(y_bot + 0.01);
        let rows = [(y_top, rl.cornice_occlusion_tone), (y_mid, dt.weather_tone + (1.0 - dt.weather_tone) * (y_top - y_mid) / (y_top - y_bot)), (y_bot, 1.0)];
        for r in rows.windows(2) {
            let ((ya, ka), (yb, kb)) = (r[0], r[1]);
            let q = [self.p(w, s[0], yb, d), self.p(w, s[1], yb, d), self.p(w, s[1], ya, d), self.p(w, s[0], ya, d)];
            let m = &mut self.m[F::Stone as usize];
            let v = m.positions.len() / 3;
            quad(m, q[0], q[1], q[2], q[3], col, want);
            for i in v..v + 4 {
                let k = if (m.positions[3 * i + 1] - ya).abs() < 1e-6 { ka } else { kb };
                for c in 0..3 {
                    m.colors[3 * i + c] *= k;
                }
            }
        }
    }

    /// Streaks under an opening's sill: two or three narrow runs of dirt from
    /// `y_top` down `len`, their widths and strengths drawn per opening.
    fn sill_streaks(&mut self, k: F, w: &Wall, o: &Op, y_top: f64, len: f64, d: f64) {
        if self.dt.is_none() {
            return;
        }
        let c = self.p(w, o.s, 0.0, 0.0);
        for (i, x) in [-0.3, 0.02, 0.31].iter().enumerate() {
            let h = h01(c[0], c[2], 11 + i as i64);
            if i == 1 && h < 0.4 {
                continue;
            }
            let width = 0.14 + 0.22 * h;
            let s = o.s + x * o.w + (h - 0.5) * 0.3;
            self.stain(k, w, [s - width / 2.0, s + width / 2.0], y_top, y_top - len * (0.55 + 0.45 * h), d, 0.6 + 0.4 * h);
        }
    }

    /// A balustrade round the square of half side `h` about the tower's plan
    /// centre `tc` (frame), standing at `y`: plinth and top rails, balusters,
    /// corner pedestals with urns.
    fn balustrade(&mut self, tc: [f64; 2], h: f64, y: f64, urns: bool) {
        let Some(d) = self.dt.clone() else { return };
        let ht = d.balustrade_m;
        let walls = [
            Wall::new([tc[0] - h, tc[1] + h], [tc[0] + h, tc[1] + h], [0.0, 1.0]),
            Wall::new([tc[0] + h, tc[1] + h], [tc[0] + h, tc[1] - h], [1.0, 0.0]),
            Wall::new([tc[0] + h, tc[1] - h], [tc[0] - h, tc[1] - h], [0.0, -1.0]),
            Wall::new([tc[0] - h, tc[1] - h], [tc[0] - h, tc[1] + h], [-1.0, 0.0]),
        ];
        let col = self.tone(F::Trim);
        let base = self.f.base;
        let r = d.baluster_diameter_m / 2.0;
        let ped = 0.5;
        for w in walls {
            self.wbox(F::Trim, &w, [0.0, w.len], [y, y + 0.16], [-0.17, 0.17], true, false, 1.0);
            self.wbox(F::Trim, &w, [0.0, w.len], [y + ht - 0.15, y + ht], [-0.19, 0.19], true, true, 1.0);
            let run = w.len - 2.0 * ped;
            let n = (run / d.baluster_pitch_m).floor().max(1.0) as usize;
            for k in 0..n {
                let s = ped + run * (k as f64 + 0.5) / n as f64;
                let c = self.p(&w, s, 0.0, 0.0);
                let (y0, y1) = (y + 0.16, y + ht - 0.15);
                let prof = [[r * 0.7, y0], [r * 0.75, y0 + 0.06], [r, y0 + (y1 - y0) * 0.3], [r * 0.55, y0 + (y1 - y0) * 0.72], [r * 0.6, y1 - 0.05], [r * 0.75, y1]];
                let prof: Vec<[f64; 2]> = prof.iter().map(|[a, yy]| [*a, base + yy]).collect();
                self.mesh(F::Trim).lathe(c[0], c[2], &prof, 6, col, None, None);
            }
            // the pedestal at the wall's start corner (each corner once)
            self.wbox(F::Trim, &w, [-ped / 2.0, ped / 2.0], [y, y + ht + 0.08], [-ped / 2.0, ped / 2.0], true, true, 1.0);
            if urns {
                let c = self.p(&w, 0.0, 0.0, 0.0);
                let u = d.urn_m;
                let y0 = y + ht + 0.08;
                let prof = [[0.16, y0], [0.12, y0 + 0.12], [0.2, y0 + u * 0.35], [0.24, y0 + u * 0.55], [0.13, y0 + u * 0.75], [0.07, y0 + u * 0.85], [0.1, y0 + u * 0.9], [0.0, y0 + u]];
                let prof: Vec<[f64; 2]> = prof.iter().map(|[a, yy]| [*a, base + yy]).collect();
                self.mesh(F::Trim).lathe(c[0], c[2], &prof, 8, col, None, None);
            }
        }
    }

    /// A moulded surround (architrave) round a flat opening, `bw` wide, `proud` out.
    fn surround(&mut self, w: &Wall, o: &Op, bw: f64, proud: f64) {
        if self.dt.is_none() {
            return;
        }
        self.wbox(F::Trim, w, [o.l() - bw, o.l()], [o.sill, o.top()], [0.0, proud], false, false, 1.0);
        self.wbox(F::Trim, w, [o.rt(), o.rt() + bw], [o.sill, o.top()], [0.0, proud], false, false, 1.0);
        self.wbox(F::Trim, w, [o.l() - bw, o.rt() + bw], [o.top(), o.top() + bw], [0.0, proud], false, true, 1.0);
    }
}

/// `n` bay centres evenly over a..b with at least `pier` of wall at each end.
fn bays(a: f64, b: f64, pitch: f64, pier: f64) -> Vec<f64> {
    let span = b - a - 2.0 * pier;
    if span < pitch * 0.5 {
        return vec![];
    }
    let n = (span / pitch).round().max(1.0) as usize;
    let p = span / n as f64;
    (0..n).map(|k| a + pier + p * (k as f64 + 0.5)).collect()
}

/// The forecourt round the building (Harry Bridges Plaza on its plaza side,
/// the walks past its ends): `FORECOURT_CELL_M` cells out to `FORECOURT_M`,
/// each laid where the street does not pave (`paved_at` None) and over dry
/// ground (`carpet_at` more than `FORECOURT_DRY_M` over the sea), a centimetre
/// under the footway datum so the street's own paving wins where they meet.
/// The paving's bays take a small tone spread (INFERRED, as the street's
/// plaza bays do).
pub const FORECOURT_M: f64 = 45.0;
pub const FORECOURT_CELL_M: f64 = 1.0;
pub const FORECOURT_DRY_M: f64 = 0.6;
const FORECOURT_BAY_M: f64 = 3.0;

/// The Ferry Building on the drawn plan, standing on `level_at` (the footway
/// datum: its plaza front's median level is the floor).
pub fn build(level_at: &dyn Fn(f64, f64) -> f64) -> Built {
    build_with(level_at, &|_, _| None, &|_, _| f64::NEG_INFINITY)
}

/// `build` with its forecourt (see `FORECOURT_M`); no forecourt when
/// `carpet_at` reads everything as under the sea.
pub fn build_with(level_at: &dyn Fn(f64, f64) -> f64, paved_at: &dyn Fn(f64, f64) -> Option<f64>, carpet_at: &dyn Fn(f64, f64) -> f64) -> Built {
    let sp = spec();
    let inf = &sp.inferred;
    let s = eo_data::groundscale::scale();
    let sc = |p: [f64; 2]| [p[0] * s, p[1] * s];
    let r = &sp.record;
    let (nw, nb, sb, sw) = (sc(r.north_west), sc(r.north_bay), sc(r.south_bay), sc(r.south_west));
    // The long axis along the plaza front, north to south; the plaza normal
    // out of it (away from the bay side).
    let len = (sw[0] - nw[0]).hypot(sw[1] - nw[1]);
    let u = [(sw[0] - nw[0]) / len, (sw[1] - nw[1]) / len];
    let mut n = [-u[1], u[0]];
    if (nw[0] - nb[0]) * n[0] + (nw[1] - nb[1]) * n[1] < 0.0 {
        n = [-n[0], -n[1]];
    }
    let mut f = Frame { o: nw, u, n, base: 0.0 };
    let depth = -f.local(nb)[1].min(f.local(sb)[1]);
    let length = f.local(sw)[0].max(f.local(sb)[0]);
    let pav: Vec<[f64; 2]> = r.pavilion.iter().map(|p| f.local(sc(*p))).collect();
    let (pu0, pu1, pd) = (pav[0][0].min(pav[1][0]), pav[2][0].max(pav[3][0]), pav[1][1].max(pav[2][1]));
    // The floor: the median footway level along the plaza front, 1.5 m out.
    let mut lv: Vec<f64> = (0..=40).map(|k| {
        let a = length * k as f64 / 40.0;
        let v = if a > pu0 && a < pu1 { pd + 1.5 } else { 1.5 };
        let p = f.at(a, v, 0.0);
        level_at(p[0], p[2])
    }).collect();
    lv.sort_by(|a, b| a.total_cmp(b));
    f.base = lv[lv.len() / 2];
    // The walls go down past the lowest level round the building.
    let foot = (lv[0] - f.base).min(0.0) - 1.2;

    let stone_kit: &'static str = match sp.stone.kit.as_str() {
        "limestone-ashlar" => "limestone-ashlar",
        "cast-stone" => "cast-stone",
        _ => "limestone-ashlar",
    };
    let dt = crate::features::ferry_building_detail().then(|| sp.detail.clone());
    let rl = (dt.is_some() && crate::features::ferry_building_relief()).then(|| sp.relief.clone());
    let mut b = B { f, m: PARTS.iter().map(|(k, ..)| MeshArrays::new(&format!("ferry-building:{k}"))).collect(), stone: sp.stone.tint_linear, dt: dt.clone(), rl: rl.clone() };
    // the upper windows' reveal (the second pass sets the glazing deeper)
    let upper_reveal = dt.as_ref().map(|d| d.upper_reveal_m).unwrap_or(inf.upper_reveal_m);

    let pitch = {
        // the surveyed bay pitch: the record's surveyed length over its 22 nave arches.
        let l = (r.south_west[0] - r.north_west[0]).hypot(r.south_west[1] - r.north_west[1]);
        l / sp.body.arches_per_frontage
    };
    let top = sp.body.parapet_top_m;
    let (c0, c1) = (inf.cornice[0], inf.cornice[1]);
    let wall_t = inf.wall_m;
    let gallery = inf.gallery_depth_m;
    let ceil = inf.gallery_ceiling_m;
    let corner_pier: f64 = 2.2; // INFERRED: the wing's end piers

    // ---- the main block's walls (frame (u, v); v = 0 the plaza front, -depth the bay front) ----
    let west_n = Wall::new([0.0, 0.0], [pu0, 0.0], [0.0, 1.0]);
    let west_s = Wall::new([pu1, 0.0], [length, 0.0], [0.0, 1.0]);
    let bay = Wall::new([length, -depth], [0.0, -depth], [0.0, -1.0]);
    let end_s = Wall::new([length, 0.0], [length, -depth], [1.0, 0.0]);
    let end_n = Wall::new([0.0, -depth], [0.0, 0.0], [-1.0, 0.0]);
    let mut counts = [0usize; 3];
    for (i, (w, with_gallery, ends)) in [(west_n, true, [true, false]), (west_s, true, [false, true]), (bay, true, [true, true]), (end_s, false, [true, true]), (end_n, false, [true, true])].into_iter().enumerate() {
        let pier0 = if ends[0] { corner_pier } else { 1.6 };
        let pier1 = if ends[1] { corner_pier } else { 1.6 };
        let centres = bays(0.0, w.len, pitch, pier0.max(pier1));
        if i < 3 {
            counts[i] = centres.len();
        }
        let arcade: Vec<Op> = centres.iter().map(|&c| Op { s: c, w: inf.arcade_arch_width_m, sill: 0.0, spring: inf.arcade_spring_m, arch: true }).collect();
        let upper: Vec<Op> = centres.iter().map(|&c| Op { s: c, w: inf.upper_arch_width_m, sill: inf.upper_sill_m, spring: inf.upper_spring_m, arch: true }).collect();
        // the overlap at a corner: long boxes run past the wall's ends by their projection.
        let ext = |p: f64| [-p, w.len + p];
        let mut ops = arcade.clone();
        ops.extend(upper.iter().copied());
        b.panel_bands(F::Stone, &w, [0.0, w.len], [foot, c1], &[inf.string_course[0]], 0.0, &ops, 1.0);
        for o in &arcade {
            let back = if with_gallery { Back::Open } else { Back::Shop };
            b.opening(&w, o, wall_t, back, 0.42, false, false);
        }
        for o in &upper {
            b.opening(&w, o, upper_reveal, Back::Glass, 0.3, true, true);
            // dirt washed off the sill, down to the string course and over its face
            b.sill_streaks(F::Stone, &w, o, o.sill - 0.16, o.sill - 0.16 - inf.string_course[1], 0.0);
            b.sill_streaks(F::Trim, &w, o, inf.string_course[1], inf.string_course[1] - inf.string_course[0], 0.25);
        }
        // the base course between the arches, and the rusticated piers over it
        let mut spans = Vec::new();
        let mut cur = 0.0;
        for o in &arcade {
            spans.push([cur, o.l()]);
            cur = o.rt();
        }
        spans.push([cur, w.len]);
        b.plinth(&w, &spans, foot, (inf.plinth_m, 0.12));
        b.rustication(&w, &spans, inf.arcade_spring_m);
        // string course, pilasters with their capitals, frieze band, cornice, parapet, coping
        let (sc0, sc1) = (inf.string_course[0], inf.string_course[1]);
        b.wbox(F::Trim, &w, ext(0.25), [sc0, sc1], [0.0, 0.25], false, true, 1.0);
        let pw = inf.pilaster_width_m;
        let mut edges: Vec<f64> = upper.windows(2).map(|p| (p[0].rt() + p[1].l()) / 2.0).collect();
        edges.push(pw / 2.0 + 0.15);
        edges.push(w.len - pw / 2.0 - 0.15);
        for e in edges {
            b.wbox(F::Trim, &w, [e - pw / 2.0, e + pw / 2.0], [sc1, inf.frieze[0] - 0.3], [0.0, 0.14], false, false, 1.0);
            b.wbox(F::Trim, &w, [e - pw / 2.0 - 0.1, e + pw / 2.0 + 0.1], [inf.frieze[0] - 0.3, inf.frieze[0]], [0.0, 0.22], false, true, 1.0);
            let k = h01(e, w.a[1] + w.len, 3);
            b.stain(F::Trim, &w, [e - pw / 2.0, e + pw / 2.0], inf.frieze[0] - 0.3, inf.frieze[0] - 0.3 - 1.2 - 1.6 * k, 0.14, 0.7 + 0.3 * k);
        }
        b.wbox(F::Trim, &w, ext(0.3), [inf.frieze[0], inf.frieze[0] + 0.18], [0.0, 0.18], false, true, 1.0);
        b.wbox(F::Trim, &w, ext(0.45), [c0, c0 + (c1 - c0) * 0.45], [0.0, 0.45], false, true, 1.0);
        let pr = inf.cornice_projection_m;
        b.wbox(F::Trim, &w, ext(pr), [c0 + (c1 - c0) * 0.45, c1], [0.0, pr], false, true, 1.0);
        // the second pass: modillions under the corona, its crown moulding, and the
        // dirt the cornice's drip washes down the frieze and the pilasters' heads.
        b.modillions(&w, [0.3, w.len - 0.3], c0 + (c1 - c0) * 0.45, 0.45, pr);
        b.crown(&w, ext(pr + 0.06), c1, pr);
        b.under_cornice(&w, [0.0, w.len], c0, upper.first().map(|o| o.top() + 0.05).unwrap_or(c0 - 1.0));
        b.wbox(F::Stone, &w, [0.0, w.len], [c1, top - 0.12], [-0.5, 0.0], true, false, 1.0);
        b.wbox(F::Trim, &w, ext(0.1), [top - 0.12, top], [-0.6, 0.1], true, true, 1.0);
        // the gallery behind the arcade
        if with_gallery && !arcade.is_empty() {
            let (g0, g1) = (arcade[0].l() - 0.8, arcade[arcade.len() - 1].rt() + 0.8);
            let back_d = -wall_t - gallery;
            // the front wall's inner face (seen through the other arches)
            b.panel(F::Stone, &w, [g0, g1], [0.0, ceil], -wall_t, -1.0, &arcade, 0.6);
            // the ceiling
            {
                let col = mul(b.tone(F::Stone), 0.55);
                let q = [b.p(&w, g0, ceil, -wall_t), b.p(&w, g1, ceil, -wall_t), b.p(&w, g1, ceil, back_d), b.p(&w, g0, ceil, back_d)];
                quad(b.mesh(F::Stone), q[0], q[1], q[2], q[3], col, [0.0, -1.0, 0.0]);
            }
            // the back wall's shop fronts, one per bay
            let shops: Vec<Op> = centres.iter().map(|&c| Op { s: c, w: inf.arcade_arch_width_m - 0.6, sill: 0.0, spring: inf.arcade_spring_m - 0.9, arch: true }).collect();
            let back_wall = Wall { a: [w.a[0] + w.n[0] * back_d, w.a[1] + w.n[1] * back_d], ..w };
            b.panel(F::Stone, &back_wall, [g0, g1], [0.0, ceil], 0.0, 1.0, &shops, 0.62);
            for o in &shops {
                b.reveal(F::Stone, &back_wall, o, 0.0, -0.25, false, 0.55);
                let k = b.shop_tone(&back_wall, o.s);
                b.pane(F::Shop, &back_wall, o, -0.25, k);
                b.mullions(&back_wall, o, -0.25, true);
            }
            // its end walls
            for (sx, sign) in [(g0, 1.0), (g1, -1.0)] {
                let q = [b.p(&w, sx, 0.0, -wall_t), b.p(&w, sx, 0.0, back_d), b.p(&w, sx, ceil, back_d), b.p(&w, sx, ceil, -wall_t)];
                let want = b.wt(&w, sign);
                let col = mul(b.tone(F::Stone), 0.6);
                quad(b.mesh(F::Stone), q[0], q[1], q[2], q[3], col, want);
            }
            // the paved floor, out to the face
            {
                let col = b.tone(F::Floor);
                let q = [b.p(&w, g0, 0.02, 0.0), b.p(&w, g1, 0.02, 0.0), b.p(&w, g1, 0.02, back_d), b.p(&w, g0, 0.02, back_d)];
                quad(b.mesh(F::Floor), q[0], q[1], q[2], q[3], col, [0.0, 1.0, 0.0]);
            }
        }
    }

    // ---- the entrance pavilion ----
    let ptop = inf.pavilion_parapet_top_m;
    let pback = -(14.0_f64.min(depth * 0.4)); // INFERRED: the tall block's depth behind the plaza front
    let pfront = Wall::new([pu0, pd], [pu1, pd], [0.0, 1.0]);
    let pside_n = Wall::new([pu0, pback], [pu0, pd], [-1.0, 0.0]);
    let pside_s = Wall::new([pu1, pd], [pu1, pback], [1.0, 0.0]);
    let prear = Wall::new([pu1, pback], [pu0, pback], [0.0, -1.0]);
    {
        let w = pfront;
        let mid = w.len / 2.0;
        let pp = inf.pavilion_arch_pitch_m;
        let mut centres: Vec<f64> = vec![mid - pp, mid, mid + pp];
        let inner = pp + inf.pavilion_arch_width_m / 2.0 + 1.6;
        let side = bays(0.0, mid - inner + 0.0, pitch, 2.4);
        for c in &side {
            centres.push(*c);
            centres.push(w.len - c);
        }
        centres.sort_by(|a, b| a.total_cmp(b));
        let main = |c: f64| (c - mid).abs() <= pp + 0.1;
        let ground: Vec<Op> = centres.iter().map(|&c| Op { s: c, w: inf.pavilion_arch_width_m, sill: 0.0, spring: inf.pavilion_arch_spring_m, arch: true }).collect();
        let upper: Vec<Op> = centres.iter().map(|&c| Op { s: c, w: inf.pavilion_upper_arch_width_m, sill: inf.pavilion_upper_sill_m, spring: inf.pavilion_upper_spring_m, arch: true }).collect();
        let (pc0, pc1) = (inf.pavilion_cornice[0], inf.pavilion_cornice[1]);
        let attic: Vec<Op> = centres.iter().map(|&c| Op { s: c, w: 1.6, sill: pc1 + 0.8, spring: pc1 + 2.3, arch: false }).collect();
        let mut ops = ground.clone();
        ops.extend(upper.iter().copied());
        ops.extend(attic.iter().copied());
        let scy = inf.pavilion_arch_spring_m + inf.pavilion_arch_width_m / 2.0 + 0.25;
        b.panel_bands(F::Stone, &w, [0.0, w.len], [foot, ptop - 0.3], &[scy, pc1], 0.0, &ops, 1.0);
        for o in &ground {
            // the entrances: a vestibule with glazed doors at its back.
            let d = if main(o.s) { wall_t + 2.4 } else { wall_t };
            b.opening(&w, o, d, Back::Shop, 0.5, false, false);
        }
        for o in &upper {
            b.opening(&w, o, upper_reveal, Back::Glass, 0.32, true, true);
            b.sill_streaks(F::Stone, &w, o, o.sill - 0.16, 1.2, 0.0);
        }
        for o in &attic {
            b.opening(&w, o, if b.dt.is_some() { 0.45 } else { 0.3 }, Back::Glass, 0.0, true, true);
            b.surround(&w, o, 0.18, 0.1);
        }
        let mut spans = Vec::new();
        let mut cur = 0.0;
        for o in &ground {
            spans.push([cur, o.l()]);
            cur = o.rt();
        }
        spans.push([cur, w.len]);
        b.plinth(&w, &spans, foot, (inf.plinth_m, 0.14));
        b.rustication(&w, &spans, inf.pavilion_arch_spring_m);
        // string course over the entrances, the entablature, cornice, attic cornice, parapet
        b.wbox(F::Trim, &w, [-0.3, w.len + 0.3], [scy, scy + 0.5], [0.0, 0.3], false, true, 1.0);
        b.wbox(F::Trim, &w, [-0.45, w.len + 0.45], [pc0, pc0 + 0.35], [0.0, 0.45], false, true, 1.0);
        b.wbox(F::Trim, &w, [-1.0, w.len + 1.0], [pc0 + 0.35, pc1], [0.0, 1.0], false, true, 1.0);
        b.modillions(&w, [0.4, w.len - 0.4], pc0 + 0.35, 0.45, 1.0);
        b.crown(&w, [-1.06, w.len + 1.06], pc1, 1.0);
        let heads = upper.iter().map(|o| o.top()).fold(scy + 0.5, f64::max);
        b.under_cornice(&w, [0.0, w.len], pc0, heads + 0.05);
        b.wbox(F::Trim, &w, [-0.5, w.len + 0.5], [ptop - 0.6, ptop - 0.3], [0.0, 0.5], false, true, 1.0);
        b.wbox(F::Stone, &w, [0.0, w.len], [ptop - 0.3, ptop], [-0.5, 0.0], true, false, 1.0);
        // the paired Corinthian columns either side of the main bays, on their pedestals
        let cd = inf.column_diameter_m;
        let col_tone = b.tone(F::Trim);
        let mut posts: Vec<f64> = vec![mid - 1.5 * pp, mid - 0.5 * pp, mid + 0.5 * pp, mid + 1.5 * pp];
        posts.retain(|p| *p > 1.0 && *p < w.len - 1.0);
        let y0 = scy + 0.5;
        for p in posts {
            for off in [-0.62, 0.62] {
                let cs = p + off;
                b.wbox(F::Trim, &w, [cs - cd * 0.7, cs + cd * 0.7], [y0, y0 + 0.9], [0.0, cd * 1.05], false, false, 1.0);
                let c = b.p(&w, cs, 0.0, cd * 0.45);
                let base = b.f.base;
                let prof = [[cd * 0.62, y0 + 0.9], [cd * 0.56, y0 + 1.15], [cd * 0.5, y0 + 1.25], [cd * 0.5, pc0 - 1.0], [cd * 0.45, pc0 - 0.9], [cd * 0.6, pc0 - 0.45], [cd * 0.72, pc0 - 0.2], [cd * 0.72, pc0]];
                let prof: Vec<[f64; 2]> = prof.iter().map(|[rr, y]| [*rr, base + y]).collect();
                b.mesh(F::Trim).lathe(c[0], c[2], &prof, 12, col_tone, None, None);
            }
        }
    }
    for w in [pside_n, pside_s] {
        // the projecting part's end of the wall (pside_n runs out to the plaza, pside_s back from it)
        let front_at_end = w.t[1] > 0.0;
        let mid = if front_at_end { w.len - pd / 2.0 } else { pd / 2.0 };
        let ext = |p: f64| if front_at_end { [0.0, w.len + p] } else { [-p, w.len] };
        let ops = [
            Op { s: mid, w: inf.arcade_arch_width_m * 0.8, sill: 0.0, spring: inf.pavilion_arch_spring_m, arch: true },
            Op { s: mid, w: inf.pavilion_upper_arch_width_m * 0.8, sill: inf.pavilion_upper_sill_m, spring: inf.pavilion_upper_spring_m, arch: true },
        ];
        let scy = inf.pavilion_arch_spring_m + inf.pavilion_arch_width_m / 2.0 + 0.25;
        b.panel_bands(F::Stone, &w, [0.0, w.len], [foot, ptop - 0.3], &[scy], 0.0, &ops, 1.0);
        b.opening(&w, &ops[0], wall_t, Back::Shop, 0.42, false, false);
        b.opening(&w, &ops[1], upper_reveal, Back::Glass, 0.3, true, true);
        let (pc0, pc1) = (inf.pavilion_cornice[0], inf.pavilion_cornice[1]);
        b.wbox(F::Trim, &w, ext(0.3), [scy, scy + 0.5], [0.0, 0.3], false, true, 1.0);
        b.wbox(F::Trim, &w, ext(0.45), [pc0, pc0 + 0.35], [0.0, 0.45], false, true, 1.0);
        b.wbox(F::Trim, &w, ext(1.0), [pc0 + 0.35, pc1], [0.0, 1.0], false, true, 1.0);
        b.wbox(F::Trim, &w, ext(0.5), [ptop - 0.6, ptop - 0.3], [0.0, 0.5], false, true, 1.0);
        b.wbox(F::Stone, &w, [0.0, w.len], [ptop - 0.3, ptop], [-0.5, 0.0], true, false, 1.0);
        let spans = [[0.0, ops[0].l()], [ops[0].rt(), w.len]];
        b.plinth(&w, &spans, foot, (inf.plinth_m, 0.14));
        b.rustication(&w, &spans, inf.pavilion_arch_spring_m);
    }
    {
        let w = prear;
        b.panel(F::Stone, &w, [0.0, w.len], [c1 - 1.0, ptop - 0.3], 0.0, 1.0, &[], 1.0);
        b.wbox(F::Stone, &w, [0.0, w.len], [ptop - 0.3, ptop], [-0.5, 0.0], true, false, 1.0);
        b.wbox(F::Trim, &w, [-0.5, w.len + 0.5], [ptop - 0.6, ptop - 0.3], [0.0, 0.5], false, true, 1.0);
    }
    // the pavilion's flat roof inside its parapet
    {
        let col = b.tone(F::Roof);
        let y = ptop - 0.5;
        let q = [b.f.at(pu0 + 0.4, pback + 0.4, y), b.f.at(pu1 - 0.4, pback + 0.4, y), b.f.at(pu1 - 0.4, pd - 0.4, y), b.f.at(pu0 + 0.4, pd - 0.4, y)];
        quad(b.mesh(F::Roof), q[0], q[1], q[2], q[3], col, [0.0, 1.0, 0.0]);
    }

    // ---- the roof: hipped from the parapets up to the nave's skylit monitor ----
    {
        let mw = inf.roof_monitor_width_m;
        let inset = (depth - mw) / 2.0;
        let (eave, mfoot, ctop, ridge) = (inf.roof_eave_m, inf.roof_monitor_foot_m, inf.roof_clerestory_top_m, inf.roof_ridge_m);
        let r0 = [[0.5, -0.5], [length - 0.5, -0.5], [length - 0.5, -depth + 0.5], [0.5, -depth + 0.5]];
        let r1 = [[inset, -inset], [length - inset, -inset], [length - inset, -depth + inset], [inset, -depth + inset]];
        let col = b.tone(F::Roof);
        let f = b.f;
        for i in 0..4 {
            let j = (i + 1) % 4;
            let (a, bb, c, d) = (f.at(r0[i][0], r0[i][1], eave), f.at(r0[j][0], r0[j][1], eave), f.at(r1[j][0], r1[j][1], mfoot), f.at(r1[i][0], r1[i][1], mfoot));
            let mid = [(r0[i][0] + r0[j][0]) / 2.0 - length / 2.0, (r0[i][1] + r0[j][1]) / 2.0 + depth / 2.0];
            let want = f.dir(mid[0], mid[1], 30.0);
            quad(b.mesh(F::Roof), a, bb, c, d, col, want);
        }
        // the clerestory: glazing between posts round the monitor
        let mlen = length - 2.0 * inset;
        let walls = [
            Wall::new(r1[0], r1[1], [0.0, 1.0]),
            Wall::new(r1[1], r1[2], [1.0, 0.0]),
            Wall::new(r1[2], r1[3], [0.0, -1.0]),
            Wall::new(r1[3], r1[0], [-1.0, 0.0]),
        ];
        for w in walls {
            let col = b.tone(F::Skylight);
            let q = [b.p(&w, 0.0, mfoot, 0.0), b.p(&w, w.len, mfoot, 0.0), b.p(&w, w.len, ctop, 0.0), b.p(&w, 0.0, ctop, 0.0)];
            let want = b.wn(&w, 1.0);
            quad(b.mesh(F::Skylight), q[0], q[1], q[2], q[3], col, want);
            let posts = (w.len / 3.0).floor().max(1.0) as usize;
            for k in 0..=posts {
                let ps = w.len * k as f64 / posts as f64;
                b.wbox(F::Frame, &w, [ps - 0.09, ps + 0.09], [mfoot, ctop], [0.0, 0.1], false, false, 1.0);
            }
            b.wbox(F::Frame, &w, [-0.3, w.len + 0.3], [mfoot - 0.1, mfoot + 0.35], [0.0, 0.2], false, false, 1.0);
            b.wbox(F::Frame, &w, [-0.3, w.len + 0.3], [ctop - 0.1, ctop + 0.15], [0.0, 0.35], false, true, 1.0);
        }
        // the skylight: a glazed ridge over the monitor, gable ends in the roof's metal, glazing bars
        let vc = -depth / 2.0;
        let (ea, eb) = (-inset, -depth + inset);
        let sky = b.tone(F::Skylight);
        for (v0, sign) in [(ea, 1.0), (eb, -1.0)] {
            let q = [f.at(inset, v0, ctop), f.at(length - inset, v0, ctop), f.at(length - inset, vc, ridge), f.at(inset, vc, ridge)];
            quad(b.mesh(F::Skylight), q[0], q[1], q[2], q[3], sky, f.dir(0.0, sign, 1.0));
        }
        for (u0, sign) in [(inset, -1.0), (length - inset, 1.0)] {
            let t = [f.at(u0, ea, ctop), f.at(u0, eb, ctop), f.at(u0, vc, ridge)];
            tri(b.mesh(F::Frame), t[0], t[1], t[2], col, f.dir(sign, 0.0, 0.0));
        }
        let bars = (mlen / 2.4).floor().max(1.0) as usize;
        let frame = b.tone(F::Frame);
        for k in 0..=bars {
            let uu = inset + mlen * k as f64 / bars as f64;
            for v0 in [ea, eb] {
                let (p0, p1) = (f.at(uu, v0, ctop + 0.05), f.at(uu, vc, ridge + 0.05));
                b.mesh(F::Frame).beam_up(p0, p1, 0.1, 0.12, frame, None);
            }
        }
        let (p0, p1) = (f.at(inset, vc, ridge + 0.08), f.at(length - inset, vc, ridge + 0.08));
        b.mesh(F::Frame).beam_up(p0, p1, 0.5, 0.25, col, None);
    }

    // ---- the clock tower (real size, its surveyed centre on the scaled plan) ----
    {
        let t = &sp.tower;
        let tc = b.f.local(sc(t.centre_surveyed));
        let st = &t.stages;
        // A square stage's four faces as walls of the frame, `h` the half side.
        let faces = |h: f64| -> [Wall; 4] {
            [
                Wall::new([tc[0] - h, tc[1] + h], [tc[0] + h, tc[1] + h], [0.0, 1.0]),
                Wall::new([tc[0] + h, tc[1] + h], [tc[0] + h, tc[1] - h], [1.0, 0.0]),
                Wall::new([tc[0] + h, tc[1] - h], [tc[0] - h, tc[1] - h], [0.0, -1.0]),
                Wall::new([tc[0] - h, tc[1] - h], [tc[0] - h, tc[1] + h], [-1.0, 0.0]),
            ]
        };
        let frame_w = Wall { a: [0.0, 0.0], t: [1.0, 0.0], n: [0.0, 1.0], len: 1.0 };
        // a frame-aligned box about the tower's centre
        let cbox = |b: &mut B, k: F, h: f64, y0: f64, y1: f64| {
            b.wbox(k, &frame_w, [tc[0] - h, tc[0] + h], [y0, y1], [tc[1] - h, tc[1] + h], true, true, 1.0);
        };
        // the shaft, from inside the pavilion roof to its cornice
        let h0 = st[0].side_m / 2.0;
        let shaft_y0 = c1 - 1.0;
        let shaft_top = st[0].top_m;
        let dial_y = shaft_top - inf.dial_centre_below_shaft_top_m;
        let dial_r = t.clock.dial_diameter_m / 2.0;
        let cp = 1.6; // INFERRED: the corner piers
        for w in faces(h0) {
            // two tiers of paired arched lights below the clock (INFERRED)
            let mut ops = Vec::new();
            for (sill, spring) in [(ptop + 3.0, ptop + 6.4), (ptop + 8.2, ptop + 10.8)] {
                for off in [-1.15, 1.15] {
                    ops.push(Op { s: w.len / 2.0 + off, w: 1.25, sill, spring, arch: true });
                }
            }
            b.panel_bands(F::Stone, &w, [0.0, w.len], [shaft_y0, shaft_top - 0.6], &[ptop + 7.6], 0.0, &ops, 1.0);
            for o in &ops {
                b.opening(&w, o, 0.4, Back::Glass, 0.16, true, true);
            }
            // the corner piers, proud of the face
            for s0 in [-0.25, w.len - cp + 0.25] {
                b.wbox(F::Stone, &w, [s0, s0 + cp], [shaft_y0, shaft_top - 0.6], [0.0, 0.25], false, false, 1.0);
            }
            // bands at the tiers, and the clock zone's string course
            b.wbox(F::Trim, &w, [-0.3, w.len + 0.3], [ptop + 1.6, ptop + 2.0], [0.0, 0.33], false, true, 1.0);
            b.wbox(F::Trim, &w, [-0.3, w.len + 0.3], [dial_y - dial_r - 1.3, dial_y - dial_r - 0.9], [0.0, 0.33], false, true, 1.0);
            // the clock: a stone ring, the white dial, hour marks, the hands (at the spec's hour)
            let cs = w.len / 2.0;
            let ring = |b: &mut B, r0: f64, r1: f64, d0: f64, d1: f64, k: F| {
                let col = b.tone(k);
                let want = b.wn(&w, 1.0);
                let segs = 32;
                for i in 0..segs {
                    let (a0, a1) = (2.0 * PI * i as f64 / segs as f64, 2.0 * PI * (i + 1) as f64 / segs as f64);
                    let at = |b: &B, a: f64, r: f64, d: f64| b.p(&w, cs + r * a.cos(), dial_y + r * a.sin(), d);
                    let q = [at(&*b, a0, r0, d1), at(&*b, a1, r0, d1), at(&*b, a1, r1, d1), at(&*b, a0, r1, d1)];
                    let out = {
                        let tt = b.wt(&w, ((a0 + a1) / 2.0).cos());
                        [tt[0], ((a0 + a1) / 2.0).sin(), tt[2]]
                    };
                    let e = [at(&*b, a0, r1, d0), at(&*b, a1, r1, d0), at(&*b, a1, r1, d1), at(&*b, a0, r1, d1)];
                    let m = b.mesh(k);
                    if r0 > 0.0 {
                        quad(m, q[0], q[1], q[2], q[3], col, want);
                    } else {
                        tri(m, q[0], q[2], q[3], col, want);
                    }
                    quad(m, e[0], e[1], e[2], e[3], col, out);
                }
            };
            ring(&mut b, dial_r, dial_r + 0.45, 0.0, 0.28, F::Trim);
            ring(&mut b, 0.0, dial_r, 0.0, 0.12, F::Dial);
            // seen from outside a face, `t` runs to the viewer's right (the frame keeps n = t turned a
            // quarter turn the same way on every wall), so clockwise from 12 is +t.
            for k in 0..12 {
                let a = 2.0 * PI * k as f64 / 12.0;
                let (sx, sy) = (a.sin(), a.cos());
                let (r0, r1) = (dial_r - 0.75, dial_r - 0.2);
                let (p0, p1) = (b.p(&w, cs + sx * r0, dial_y + sy * r0, 0.175), b.p(&w, cs + sx * r1, dial_y + sy * r1, 0.175));
                let up = b.wn(&w, 1.0);
                let col = b.tone(F::Frame);
                b.mesh(F::Frame).beam_up(p0, p1, if k % 3 == 0 { 0.32 } else { 0.18 }, 0.05, col, Some(up));
            }
            let hour = inf.clock_hour.rem_euclid(12.0);
            for (len_m, wdt, ang) in [(t.clock.hour_hand_m, 0.3, hour / 12.0 * 2.0 * PI), (t.clock.minute_hand_m.min(dial_r - 0.15), 0.2, (inf.clock_hour.fract()) * 2.0 * PI)] {
                let (sx, sy) = (ang.sin(), ang.cos());
                let (p0, p1) = (b.p(&w, cs - sx * 0.35, dial_y - sy * 0.35, 0.225), b.p(&w, cs + sx * len_m, dial_y + sy * len_m, 0.225));
                let up = b.wn(&w, 1.0);
                let col = b.tone(F::Frame);
                b.mesh(F::Frame).beam_up(p0, p1, wdt, 0.06, col, Some(up));
            }
        }
        // the shaft's cornice, and the balustrade round the belfry's foot on it
        cbox(&mut b, F::Trim, h0 + 0.35, shaft_top - 0.6, shaft_top - 0.25);
        cbox(&mut b, F::Trim, h0 + 0.7, shaft_top - 0.25, shaft_top);
        b.balustrade(tc, h0 + 0.42, shaft_top, true);
        // the upper stages: each with its openings (voids), corner piers and cornice
        let mut y = shaft_top;
        for (i, stg) in st.iter().enumerate().skip(1) {
            let h = stg.side_m / 2.0;
            let tall = stg.top_m - y;
            let (count, ow) = match i {
                1 => (3, 1.9),
                3 => (1, 2.3),
                _ => (1, (stg.side_m * 0.26).max(1.0)),
            };
            for w in faces(h) {
                let pitch = w.len / count as f64;
                let ops: Vec<Op> = (0..count).map(|k| {
                    let spring = y + tall * 0.62;
                    Op { s: pitch * (k as f64 + 0.5), w: ow, sill: y + tall * 0.16, spring: spring.min(stg.top_m - 0.55 - ow / 2.0), arch: true }
                }).collect();
                b.panel(F::Stone, &w, [0.0, w.len], [y, stg.top_m - 0.4], 0.0, 1.0, &ops, 1.0);
                for o in &ops {
                    b.opening(&w, o, 0.55, Back::Void, 0.14, i == 1, true);
                }
                let pier = (stg.side_m * 0.11).max(0.5);
                for s0 in [-0.15, w.len - pier + 0.15] {
                    b.wbox(F::Stone, &w, [s0, s0 + pier], [y, stg.top_m - 0.4], [0.0, 0.15], false, false, 1.0);
                }
                if b.dt.is_some() && i == 3 {
                    // the lantern's corners: an engaged column with base and capital at each
                    let c = b.p(&w, -0.05, 0.0, 0.2);
                    let base = b.f.base;
                    let (y0, y1) = (y, stg.top_m - 0.4);
                    let prof = [[0.36, y0], [0.36, y0 + 0.2], [0.27, y0 + 0.32], [0.25, y1 - 0.55], [0.3, y1 - 0.35], [0.38, y1 - 0.12], [0.38, y1]];
                    let prof: Vec<[f64; 2]> = prof.iter().map(|[r, yy]| [*r, base + yy]).collect();
                    let col = b.tone(F::Trim);
                    b.mesh(F::Trim).lathe(c[0], c[2], &prof, 10, col, None, None);
                }
            }
            cbox(&mut b, F::Trim, h + 0.25, stg.top_m - 0.4, stg.top_m - 0.15);
            cbox(&mut b, F::Trim, h + 0.45, stg.top_m - 0.15, stg.top_m);
            if b.dt.is_some() {
                // the second pass: a balustrade with corner urns at each setback up to the lantern
                if i + 1 < st.len() && i <= 2 {
                    b.balustrade(tc, h + 0.22, stg.top_m, true);
                }
            } else if i == 1 {
                // the belfry's balustrade line on its cornice
                cbox(&mut b, F::Trim, h + 0.35, stg.top_m, stg.top_m + 0.7);
            }
            y = stg.top_m;
        }
        // the drum, the dome, the finial and the flagstaff (lathes)
        let c = b.f.at(tc[0], tc[1], 0.0);
        let base = b.f.base;
        let (dr, dm) = (t.drum.diameter_m / 2.0, t.dome.diameter_m / 2.0);
        let prof: Vec<[f64; 2]> = [
            [dr, y],
            [dr, t.drum.top_m - 0.35],
            [dr + 0.2, t.drum.top_m - 0.2],
            [dr + 0.2, t.drum.top_m],
            [dm, t.drum.top_m],
            [dm, t.drum.top_m + 0.15],
            [dm * 0.92, t.drum.top_m + (t.dome.top_m - t.drum.top_m) * 0.45],
            [dm * 0.7, t.drum.top_m + (t.dome.top_m - t.drum.top_m) * 0.75],
            [dm * 0.35, t.dome.top_m - 0.08],
            [0.2, t.dome.top_m],
            [0.2, t.dome.top_m + 1.2],
            [0.42, t.dome.top_m + 1.6],
            [0.46, t.dome.top_m + 2.0],
            [0.42, t.dome.top_m + 2.4],
            [0.12, t.dome.top_m + 2.8],
            [0.08, t.finial_top_m],
            [0.0, t.finial_top_m],
        ]
        .iter()
        .map(|[r, yy]| [*r, base + yy])
        .collect();
        let trim = b.tone(F::Trim);
        b.mesh(F::Trim).lathe(c[0], c[2], &prof, 20, trim, None, None);
        let pole = [[0.09, base + t.finial_top_m], [0.05, base + t.flagstaff_top_m], [0.0, base + t.flagstaff_top_m]];
        let white = b.tone(F::Dial);
        b.mesh(F::Dial).lathe(c[0], c[2], &pole, 8, white, None, None);
    }

    // ---- the forecourt ----
    {
        let sea = eo_water::shoreline::SEA_LEVEL_Y;
        let c = FORECOURT_CELL_M;
        let base_tone = b.tone(F::Floor);
        let f = b.f;
        let (n_u, n_v) = (((length + 2.0 * FORECOURT_M) / c).ceil() as i64, ((depth + FORECOURT_M) / c).ceil() as i64);
        let mut quads: Vec<([V3; 4], Rgb)> = Vec::new();
        for j in 0..n_v {
            let v0 = FORECOURT_M - (j + 1) as f64 * c;
            for i in 0..n_u {
                let u0 = -FORECOURT_M + i as f64 * c;
                let (um, vm) = (u0 + c / 2.0, v0 + c / 2.0);
                // not inside the building (the main block or the pavilion), not beside its bay front
                let in_block = um > -0.2 && um < length + 0.2 && vm > -depth - 0.2 && vm < 0.2;
                let in_pavilion = um > pu0 - 0.2 && um < pu1 + 0.2 && vm >= 0.0 && vm < pd + 0.2;
                let bay_side = vm < -depth + 0.2 && um > 0.0 && um < length;
                if in_block || in_pavilion || bay_side {
                    continue;
                }
                let m = f.at(um, vm, 0.0);
                if paved_at(m[0], m[2]).is_some() || carpet_at(m[0], m[2]) < sea + FORECOURT_DRY_M {
                    continue;
                }
                let corner = |du: f64, dv: f64| {
                    let p = f.at(u0 + du, v0 + dv, 0.0);
                    [p[0], level_at(p[0], p[2]) - 0.01, p[2]]
                };
                let bay = ((um / FORECOURT_BAY_M).floor() as i64, (vm / FORECOURT_BAY_M).floor() as i64);
                let h = (bay.0.wrapping_mul(73_856_093) ^ bay.1.wrapping_mul(19_349_663)).rem_euclid(41) as f64;
                let k = 1.0 + (h - 20.0) / 20.0 * 0.05;
                quads.push(([corner(0.0, 0.0), corner(c, 0.0), corner(c, c), corner(0.0, c)], mul(base_tone, k)));
            }
        }
        let m = b.mesh(F::Floor);
        for (q, col) in quads {
            quad(m, q[0], q[1], q[2], q[3], col, [0.0, 1.0, 0.0]);
        }
    }

    let mut parts = Vec::new();
    let mut triangles = 0;
    for (i, (key, kit, cast, footway, rough, metal)) in PARTS.iter().enumerate() {
        let mesh = b.m[i].freeze();
        if mesh.is_empty() {
            continue;
        }
        triangles += mesh.triangles();
        let kit = if *key == "stone" { Some(if dt.is_some() { detail_stone_kit() } else { stone_kit }) } else { *kit };
        let rough = match (&rl, *key) {
            (Some(r), "glass") => r.glass_roughness,
            _ => *rough,
        };
        parts.push(Part { key, kit, cast: *cast, footway: *footway, rough, metal: *metal, mesh });
    }
    Built { parts, base_y: f.base, length_m: length, depth_m: depth, bays: counts, triangles }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spec_parses_and_cites_its_sources() {
        let s = spec();
        assert_eq!(s.record.id, format!("sf-building-{RECORD}"));
        for k in ["record", "osm-parts", "wikipedia", "noehill", "cdmg-191"] {
            assert!(s.sources.contains_key(k), "source {k}");
        }
        // 245 ft, the four 22 ft dials.
        assert!((s.tower.finial_top_m - 245.0 * 0.3048).abs() < 0.01);
        assert!((s.tower.clock.dial_diameter_m - 22.0 * 0.3048).abs() < 0.01);
        // the stages rise and narrow.
        for p in s.tower.stages.windows(2) {
            assert!(p[1].top_m > p[0].top_m && p[1].side_m < p[0].side_m);
        }
    }

    #[test]
    fn the_detail_pass_adds_to_the_first_and_names_its_scaled_ashlar() {
        let _g = PASSES.lock().unwrap_or_else(|e| e.into_inner());
        crate::features::set_ferry_building_detail(false);
        let a = build(&|_, _| 3.0);
        crate::features::set_ferry_building_detail(true);
        let b = build(&|_, _| 3.0);
        assert!(b.triangles > a.triangles && b.triangles < 150_000, "{} -> {} triangles", a.triangles, b.triangles);
        let kit = |x: &Built| x.parts.iter().find(|p| p.key == "stone").and_then(|p| p.kit).unwrap();
        assert_eq!(kit(&a), "limestone-ashlar");
        assert_eq!(kit(&b), detail_stone_kit());
        // 0.6 m courses drawn from the scan's eleven over its 2 m tile.
        assert!((spec().detail.ashlar_scale() - 3.3).abs() < 1e-9);
        // the second pass's pieces stay on the building: nothing below its foot or over the flagstaff.
        let (lo, hi) = b.parts.iter().flat_map(|p| p.mesh.positions.chunks_exact(3).map(|v| v[1] as f64)).fold((f64::MAX, f64::MIN), |(l, h), y| (l.min(y), h.max(y)));
        assert!(lo > 3.0 - 6.0 && hi <= 3.0 + spec().tower.flagstaff_top_m + 1e-3, "{lo} .. {hi}");
    }

    /// The passes' switches are process-wide: the tests that flip them take turns.
    static PASSES: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn the_relief_pass_splits_the_glazing_into_lights_within_the_budget() {
        let _g = PASSES.lock().unwrap_or_else(|e| e.into_inner());
        crate::features::set_ferry_building_detail(true);
        crate::features::set_ferry_building_relief(false);
        let a = build(&|_, _| 3.0);
        crate::features::set_ferry_building_relief(true);
        let b = build(&|_, _| 3.0);
        let part = |x: &Built, k: &str| x.parts.iter().find(|p| p.key == k).map(|p| (p.mesh.triangles(), p.rough)).unwrap();
        let (ga, gb) = (part(&a, "glass"), part(&b, "glass"));
        // a pane a light (the fanlights stay fans): more than twice the glazing's triangles; the glass a near-mirror.
        assert!(gb.0 > 2 * ga.0, "glass {} -> {} triangles", ga.0, gb.0);
        assert!((ga.1 - 0.08).abs() < 1e-9 && (gb.1 - spec().relief.glass_roughness).abs() < 1e-9);
        // the frames round the fanlights, and the budget.
        assert!(part(&b, "frame").0 > part(&a, "frame").0);
        assert!(b.triangles < 150_000, "{} triangles", b.triangles);
        // the cornices' occlusion darkens the wall under them past the second pass's dirt.
        let darkest = |x: &Built| x.parts.iter().find(|p| p.key == "stone").unwrap().mesh.colors.chunks_exact(3).map(|c| c[1] as f64).fold(f64::MAX, f64::min);
        assert!(darkest(&b) <= darkest(&a) + 1e-9);
    }

    #[test]
    fn bays_fill_the_frontage_evenly() {
        let c = bays(0.0, 100.0, 9.0, 2.0);
        assert_eq!(c.len(), 11);
        assert!((c[0] - (2.0 + 96.0 / 22.0)).abs() < 1e-9);
        assert!(bays(0.0, 3.0, 9.0, 2.0).is_empty());
    }
}
