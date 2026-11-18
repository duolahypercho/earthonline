//! The Golden Gate Bridge: a port of src/world/landmarks/golden-gate-bridge.js
//! (generator) and golden-gate-bridge-site.js (alignment and the derived
//! frame) from the `main` branch. Positions and dimensions come from the
//! export (`data::GoldenGate`); the INFERRED structural literals of the JS
//! generator's function bodies are kept here with their JS names.
//!
//! PRESENTATION ONLY, as in the JS: the deck has no collider and nothing
//! routes onto it; the colliders are the parts a body on the ground or the
//! water can meet.

use crate::Built;
use crate::Collider;
use crate::data::{GoldenGate, LandmarksJs, hex, num, pair};
use crate::geometry::{ALL, GroundGrid, MeshArrays, NO_BOTTOM, NO_TOP, Rgb, shade, srgb_hex};
use crate::suspension::{self, CableSpec, DeckColours, DeckSpec, DeckStandards, Frame, Run, SuspenderSpec, TrussSpec, stations};
use serde_json::json;

mod pylons;
pub use pylons::published_dimensions;

/// Near geometry stays this far above the sea; below it is the far batch's skirt.
const NEAR_FLOOR_ABOVE_SEA_M: f64 = 1.0;
/// Width of each arch pylon column across the bridge (INFERRED).
const ARCH_PYLON_COLUMN_M: f64 = 2.4;
/// Depth of the south approach span's plate girders (INFERRED).
const APPROACH_GIRDER_DEPTH_M: f64 = 1.8;
/// `goldenGatePlan`: the structure's start is searched from the road end up
/// to this chainage (south) and down to L + this (north), in 2 m steps.
const STRUCTURE_SEARCH_SOUTH_C: f64 = -460.0;
const STRUCTURE_SEARCH_NORTH_PAST_L: f64 = 360.0;
const STRUCTURE_SEARCH_STEP_M: f64 = 2.0;
/// `ORANGE_DARK = ORANGE * 0.62`.
const DARK: f64 = 0.62;
/// INFERRED: rope spacing within a suspender pair.
const PAIR_GAP_M: f64 = 0.45;

/// Dimension keys that are heights or member sizes of the structure (not
/// plan widths, which data.rs puts on the ground plan, nor the real-size
/// cable, rope, railing, curb and barrier sizes): times `structure_scale`.
const STRUCTURE_KEYS: &[&str] = &["towerAboveWaterM", "towerAboveRoadwayM", "roadwayAtTowerM", "clearanceM", "trussDepthM", "panelM", "saddleBelowTopM", "cableLowAboveDeckM", "pylonAboveDeckM", "suspenderSpacingM", "bentSpacingMaxM"];

/// International Orange as an sRGB tone (`BridgeFeatures::paint`). The
/// Golden Gate Bridge Highway and Transportation District's paint formula is
/// given as print values, C 0 % M 69 % Y 100 % K 6 %, the paint supplied by
/// Sherwin-Williams to that formula, the nearest stock colour SW 6328
/// Fireweed (goldengate.org, "Color & Art Deco Styling"; Fireweed's light
/// reflectance value 7). The export's 0xF04A00 is the naive separation of the
/// print values (luminance factor 0.23, no blue at all): a print ink, not a
/// reflectance. #C0362C (sRGB 192, 54, 44; luminance factor 0.14, between
/// Fireweed's 0.07 and the naive 0.23) is the bridge's widely published
/// screen equivalent.
pub const INTERNATIONAL_ORANGE_SRGB: u32 = 0xC0362C;

/// The structure's own scale on the drawn world: the drawn terrain's height
/// scale with `BridgeFeatures::scale` (the plan is the ground plan's already),
/// else 1 (every height real, as the export).
pub fn structure_scale() -> f64 {
    if suspension::features().scale { eo_data::groundscale::height_scale() } else { 1.0 }
}

/// The site data the generator builds from: the export's, with the
/// structure's heights and member sizes times `structure_scale` (the leg
/// stages and struts, the piers' tops, the supports' sizes along the span).
/// With the plan 1.25 x and heights real the towers stood 1 / 1.25 as tall
/// as their span and the hills about them draw them (a 1,600 m span under
/// 227 m towers); scaled, every proportion of the published drawings holds.
pub fn data(js: &LandmarksJs) -> GoldenGate {
    let mut g = js.golden_gate.clone();
    let h = structure_scale();
    if h != 1.0 {
        for k in STRUCTURE_KEYS {
            if let Some(v) = g.dimensions[*k].as_f64() {
                g.dimensions[*k] = json!(v * h);
            }
        }
        let [cw, cd] = pair(&g.dimensions, "bentColumnM");
        g.dimensions["bentColumnM"] = json!([cw * h, cd * h]);
        for st in &mut g.leg_stages {
            st.iter_mut().for_each(|v| *v *= h);
        }
        for st in &mut g.struts {
            st.iter_mut().for_each(|v| *v *= h);
        }
        for sup in g.supports.values_mut() {
            sup.extent[1] *= h; // along the span (across is the deck's, on the plan)
            if let Some(t) = sup.top_m.as_mut() {
                *t *= h;
            }
        }
    }
    g
}

/// The site's derived frame (golden-gate-bridge-site.js).
pub struct Site {
    pub south_tower: [f64; 2],
    pub north_tower: [f64; 2],
    pub main_span_measured: f64,
    pub axis: [f64; 2],
    pub across: [f64; 2],
    pub half_plane: f64,
    profiles: [Vec<[f64; 2]>; 2],
}

impl Site {
    pub fn new(g: &GoldenGate) -> Self {
        let mid = |a: [f64; 2], b: [f64; 2]| [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        let (s, n) = (&g.tower_legs["south"], &g.tower_legs["north"]);
        let south_tower = mid(s.east, s.west);
        let north_tower = mid(n.east, n.west);
        let (dx, dz) = (north_tower[0] - south_tower[0], north_tower[1] - south_tower[1]);
        let l = dx.hypot(dz);
        let axis = [dx / l, dz / l];
        let across = [-axis[1], axis[0]];
        let half_plane = (s.east[0] - s.west[0]).hypot(s.east[1] - s.west[1]) / 2.0;
        let mut site = Self { south_tower, north_tower, main_span_measured: l, axis, across, half_plane, profiles: [vec![], vec![]] };
        let p0 = site.lateral_profile(&g.carriageways[0]);
        let p1 = site.lateral_profile(&g.carriageways[1]);
        site.profiles = [p0, p1];
        site
    }
    /// `ggbChainage`.
    pub fn chainage(&self, x: f64, z: f64) -> f64 {
        (x - self.south_tower[0]) * self.axis[0] + (z - self.south_tower[1]) * self.axis[1]
    }
    /// The site's `ggbPoint` (on the tower line).
    pub fn axis_point(&self, c: f64, s: f64) -> [f64; 2] {
        [self.south_tower[0] + self.axis[0] * c + self.across[0] * s, self.south_tower[1] + self.axis[1] * c + self.across[1] * s]
    }
    fn lateral_profile(&self, points: &[f64]) -> Vec<[f64; 2]> {
        let mut rows: Vec<[f64; 2]> = points
            .chunks_exact(2)
            .map(|p| {
                let c = self.chainage(p[0], p[1]);
                let [ax, az] = self.axis_point(c, 0.0);
                [c, (p[0] - ax) * self.across[0] + (p[1] - az) * self.across[1]]
            })
            .collect();
        rows.sort_by(|p, q| p[0].total_cmp(&q[0]));
        rows
    }
    /// `ggbCentreOffset`: the mean of the two carriageways' lateral offsets.
    pub fn centre_offset(&self, c: f64) -> f64 {
        let interp = |rows: &[[f64; 2]], c: f64| {
            if c <= rows[0][0] {
                return rows[0][1];
            }
            for i in 1..rows.len() {
                if c <= rows[i][0] {
                    let t = (c - rows[i - 1][0]) / (rows[i][0] - rows[i - 1][0]);
                    return rows[i - 1][1] + (rows[i][1] - rows[i - 1][1]) * t;
                }
            }
            rows[rows.len() - 1][1]
        };
        (interp(&self.profiles[0], c) + interp(&self.profiles[1], c)) / 2.0
    }
    /// The generator's `ggbPoint`: on the deck's own alignment.
    pub fn deck_point(&self, c: f64, s: f64) -> [f64; 2] {
        self.axis_point(c, s + self.centre_offset(c))
    }
}

/// `goldenGatePlan`.
pub struct Plan<'a> {
    pub l: f64,
    pub c_south: f64,
    pub c_north: f64,
    pub c_start: f64,
    pub c_end: f64,
    pub y_south: f64,
    pub y_north: f64,
    pub y_tower: f64,
    pub y_mid: f64,
    pub arch_south_c: f64,
    pub arch_north_c: f64,
    pub anchor_north_c: f64,
    pub site: &'a Site,
}

impl Plan<'_> {
    pub fn deck_top(&self, c: f64) -> f64 {
        if c <= 0.0 {
            return self.y_south + (self.y_tower - self.y_south) * ((c - self.c_south) / (0.0 - self.c_south)).max(0.0).min(1.0);
        }
        if c <= self.l {
            let t = (c - self.l / 2.0) / (self.l / 2.0);
            return self.y_tower + (self.y_mid - self.y_tower) * (1.0 - t * t);
        }
        self.y_tower + (self.y_north - self.y_tower) * ((c - self.l) / (self.c_north - self.l)).max(0.0).min(1.0)
    }
}

pub fn plan<'a>(js: &LandmarksJs, site: &'a Site, ground: &dyn Fn(f64, f64) -> f64) -> Plan<'a> {
    let g = &data(js);
    let d = &g.dimensions;
    let water = js.sea_level_y;
    let l = site.main_span_measured;
    let (ws, wn) = (g.way_ends["south"], g.way_ends["north"]);
    let c_south = site.chainage(ws[0], ws[1]);
    let c_north = site.chainage(wn[0], wn[1]);
    let road_lift = js.default_road_lift_m;
    let y_south = ground(ws[0], ws[1]) + road_lift;
    let y_north = ground(wn[0], wn[1]) + road_lift;
    let y_tower = water + num(d, "roadwayAtTowerM");
    let y_mid = water + num(d, "clearanceM") + num(d, "trussDepthM");
    let sup = |k: &str| g.supports[k].centre;
    let mut p = Plan {
        l,
        c_south,
        c_north,
        c_start: c_south,
        c_end: c_north,
        y_south,
        y_north,
        y_tower,
        y_mid,
        arch_south_c: site.chainage(sup("archSouth")[0], sup("archSouth")[1]),
        arch_north_c: site.chainage(sup("archNorth")[0], sup("archNorth")[1]),
        anchor_north_c: site.chainage(sup("northAnchor")[0], sup("northAnchor")[1]),
        site,
    };
    let hp = site.half_plane;
    let min_ground = |c: f64| {
        let a = site.deck_point(c, 0.0);
        let b = site.deck_point(c, hp);
        let e = site.deck_point(c, -hp);
        ground(a[0], a[1]).max(ground(b[0], b[1])).max(ground(e[0], e[1]))
    };
    let (truss, clearance) = (num(d, "trussDepthM"), num(d, "vehicleClearanceM"));
    let clears = |p: &Plan, c: f64| p.deck_top(c) - truss - min_ground(c) >= clearance;
    let mut c_start = c_south;
    // the search bounds are chainages, on the scaled plan as the span is.
    let gs = eo_data::groundscale::scale();
    while c_start < STRUCTURE_SEARCH_SOUTH_C * gs && !clears(&p, c_start) {
        c_start += STRUCTURE_SEARCH_STEP_M;
    }
    let mut c_end = c_north;
    while c_end > l + STRUCTURE_SEARCH_NORTH_PAST_L * gs && !clears(&p, c_end) {
        c_end -= STRUCTURE_SEARCH_STEP_M;
    }
    p.c_start = c_start;
    p.c_end = c_end;
    p
}

struct Colours {
    orange: Rgb,
    orange_dark: Rgb,
    asphalt: Rgb,
    sidewalk: Rgb,
    concrete: Rgb,
    barrier: Rgb,
    white: Rgb,
    yellow: Rgb,
}

/// Everything the part builders share.
struct Ctx<'a> {
    g: &'a GoldenGate,
    d: &'a serde_json::Value,
    site: &'a Site,
    plan: &'a Plan<'a>,
    frame: &'a Frame<'a>,
    /// On the tower line (no curve offset): where the legs stand.
    tower_frame: &'a Frame<'a>,
    ground: &'a dyn Fn(f64, f64) -> f64,
    k: Colours,
    yaw: f64,
    water: f64,
    tower_zone_half: f64,
    /// `structure_scale` (member sizes the export leaves in code are scaled with it).
    h: f64,
    /// `BridgeFeatures::detail`, `::wires`.
    detail: bool,
    wires: bool,
    /// `BridgeFeatures::members`.
    members: bool,
    /// The fourth pass's `BridgeFeatures` (`haunch`, `bracing`, `pylons`, `arch2`, `fender2`, `collar`).
    haunch: bool,
    bracing: bool,
    pylons: bool,
    arch2: bool,
    fender2: bool,
    collar: bool,
}

/// A portal strut's casing (`BridgeFeatures::haunch`): which exported strut,
/// its bottom and top (world heights), its thickness along the span, and
/// whether a haunch stands over it (another casing above).
struct PortalCasing {
    index: usize,
    y0: f64,
    y1: f64,
    along: f64,
    top_haunch: bool,
}

/// INFERRED (`BridgeFeatures::haunch`; no drawing of the casings is
/// published, and the export's strut heights are the bracing's): the
/// casing's faces stand half a cell (the legs' 3.5 ft cell, Strauss 1937)
/// inside the legs' faces along the span; a casing grows top and bottom to
/// stand 0.45 of the opening under it (estimated from the Battery Spencer
/// photograph against the exported 0.33-0.36: the owner must accept it);
/// the haunch brackets reach 0.15 of the opening's width out from each leg
/// and as far along it, in five steps; the lip is 0.6 m high and 0.3 m proud.
/// Each is times the structure scale where it is a length.
const CASING_SETBACK_M: f64 = 3.5 * 0.3048 / 2.0;
const PORTAL_DEPTH_RATIO: f64 = 0.45;
const HAUNCH_RATIO: f64 = 0.15;
const HAUNCH_STEPS: usize = 5;
const CASING_LIP_M: [f64; 2] = [0.6, 0.3];

/// `BridgeFeatures::bracing`: the bracing members' width along the span,
/// "approximately the width of three cells, or 10 1/2 ft" (Strauss 1937).
/// INFERRED: strut 5's top this far under the stiffening truss's bottom
/// chord; a diagonal's depth in the tower's plane; the lacing's chord, bar
/// and pitch (chord, bar, pitch). Each is times the structure scale.
const BRACING_WIDTH_M: f64 = 10.5 * 0.3048;
const BRACING_DECK_GAP_M: f64 = 0.5;
const BRACING_DIAGONAL_M: f64 = 2.4;
const BRACING_LACING_M: [f64; 3] = [0.5, 0.3, 1.4];

impl Ctx<'_> {
    fn in_tower_zone(&self, c0: f64, c1: f64) -> bool {
        [0.0, self.plan.l].iter().any(|&t| c1 > t - self.tower_zone_half && c0 < t + self.tower_zone_half)
    }
    #[allow(non_snake_case)]
    fn P(&self, c: f64, s: f64, y: f64) -> [f64; 3] {
        self.frame.P(c, s, y)
    }

    fn cable_runs(&self) -> Vec<Run> {
        let (d, p) = (self.d, self.plan);
        let saddle = self.water + num(d, "towerAboveWaterM") - num(d, "saddleBelowTopM");
        let low = p.y_mid + num(d, "cableLowAboveDeckM");
        let sag_main = saddle - low;
        if self.pylons {
            return self.cable_runs4(saddle, sag_main);
        }
        let south_top = p.deck_top(p.arch_north_c) + num(d, "pylonAboveDeckM");
        let north_top = p.deck_top(p.anchor_north_c) + num(d, "pylonAboveDeckM");
        vec![
            Run { c0: p.arch_north_c, c1: 0.0, y0: south_top, y1: saddle, sag: suspension::side_span_sag(sag_main, p.l, -p.arch_north_c) },
            Run { c0: 0.0, c1: p.l, y0: saddle, y1: saddle, sag: sag_main },
            Run { c0: p.l, c1: p.anchor_north_c, y0: saddle, y1: north_top, sag: suspension::side_span_sag(sag_main, p.l, p.anchor_north_c - p.l) },
        ]
    }

    fn towers(&self, orange: &mut MeshArrays, concrete: &mut MeshArrays, skirt: &mut MeshArrays, far: &mut MeshArrays) {
        let (g, d, w, yaw) = (self.g, self.d, self.water, self.yaw);
        let k = &self.k;
        let stages = &g.leg_stages;
        // The JS writes `h1 === 225` (the top stage reaches the published tower
        // top) and `h0 === 13` (the lowest stage sits on the pier): the last
        // stage's top and the first stage's foot of the exported table.
        let top_h1 = stages.iter().map(|s| s[1]).fold(f64::MIN, f64::max);
        let foot_h0 = stages[0][0];
        for (key, pier) in [("south", &g.supports["southPier"]), ("north", &g.supports["northPier"])] {
            let [cx, cz] = pier.centre;
            let half_across = pier.extent[0] / 2.0;
            let half_along = pier.extent[1] / 2.0;
            let top = w + pier.top_m.unwrap_or(0.0);
            let floor = w + NEAR_FLOOR_ABOVE_SEA_M;
            let bed = (self.ground)(cx, cz).min(w - 2.0);
            concrete.box_(cx, cz, floor, top, half_along, half_across, yaw, k.concrete, NO_BOTTOM);
            skirt.box_(cx, cz, bed, floor, half_along, half_across, yaw, k.concrete, NO_TOP);
            far.box_(cx, cz, bed, top, half_along, half_across, yaw, k.concrete, ALL);
            let legs = &g.tower_legs[key];
            for [lx, lz] in [legs.east, legs.west] {
                for &[h0, h1, across, along] in stages {
                    let y1 = if h1 == top_h1 { num(d, "towerAboveWaterM") } else { h1 };
                    let faces = crate::geometry::Faces { top: true, bottom: h0 == foot_h0 };
                    if self.detail {
                        suspension::notched_box(orange, lx, lz, w + h0, w + y1, along / 2.0, across / 2.0, yaw, k.orange, faces);
                    } else {
                        orange.box_(lx, lz, w + h0, w + y1, along / 2.0, across / 2.0, yaw, k.orange, faces);
                    }
                    far.box_(lx, lz, w + h0, w + y1, along / 2.0, across / 2.0, yaw, k.orange, ALL);
                }
            }
            let ([ex, ez], [wx, wz]) = (legs.east, legs.west);
            let c = self.site.chainage(ex, ez);
            let portals = self.portals();
            for (i, &[h0, h1, along]) in g.struts.iter().enumerate() {
                let (mx, mz) = ((ex + wx) / 2.0, (ez + wz) / 2.0);
                let span = (ex - wx).hypot(ez - wz);
                if let Some(p) = portals.iter().find(|p| p.index == i) {
                    self.portal(orange, far, c, mx, mz, span, p);
                    continue;
                }
                if self.bracing && w + h1 < self.plan.y_tower {
                    // (drawn laced by `lower_bracing`; the far item keeps the box)
                    far.box_(mx, mz, w + h0, w + h1, along / 2.0, span / 2.0, yaw, k.orange_dark, ALL);
                    continue;
                }
                orange.box_(mx, mz, w + h0, w + h1, along / 2.0, span / 2.0, yaw, k.orange_dark, ALL);
                far.box_(mx, mz, w + h0, w + h1, along / 2.0, span / 2.0, yaw, k.orange_dark, ALL);
                if self.detail && w + h0 > self.plan.y_tower {
                    self.portal_ribs(orange, c, w + h0, w + h1, along);
                }
            }
            if self.bracing {
                self.lower_bracing(orange, far, c);
                continue;
            }
            // INFERRED: X bracing between the two lowest struts (the JS's 26 and 50 m: their facing chords).
            let s = self.site.half_plane - 3.0 * self.h;
            let (lo, hi) = (g.struts[0][1], g.struts[1][0]);
            let xb = 1.4 * self.h;
            orange.beam(self.P(c, -s, w + lo), self.P(c, s, w + hi), xb, xb, k.orange_dark);
            orange.beam(self.P(c, s, w + lo), self.P(c, -s, w + hi), xb, xb, k.orange_dark);
        }
    }

    /// The towers' bracing below the roadway (`BridgeFeatures::bracing`):
    /// "two panels of diagonals" between the legs, separated by horizontal
    /// struts, with a strut immediately below the roadway where the
    /// suspended spans' lateral forces pass to the tower (Strauss 1937; the
    /// District's 2025 retrofit scope names "Tower Strut 5" and its "bottom
    /// lacing"), read as struts 5 (under the deck), 6 and 7 (the export's two
    /// lower struts) with an X panel between 5 and 6 and between 6 and 7. The
    /// members are latticed ("riveted from many small pieces": goldengate.org,
    /// "Seismic Retrofits and Historic Preservation"): each is two laced faces,
    /// one each side of its width along the span (the bracing members'
    /// "approximately the width of three cells, or 10 1/2 ft": Strauss 1937),
    /// open between them. INFERRED: strut 5's height (strut 6's) and its top
    /// at the stiffening truss's bottom chord, the members' depths in the
    /// tower's plane, their chords, lacing and pitch.
    fn lower_bracing(&self, orange: &mut MeshArrays, far: &mut MeshArrays, c: f64) {
        let (g, w, h, k) = (self.g, self.water, self.h, &self.k);
        let f = self.tower_frame;
        let hp = self.site.half_plane;
        let truss = num(self.d, "trussDepthM");
        let (s7, s6) = (g.struts[0], g.struts[1]);
        let s5_top = self.plan.y_tower - truss - BRACING_DECK_GAP_M * h;
        let s5 = [s5_top - w - (s6[1] - s6[0]), s5_top - w, s6[2]];
        let width = BRACING_WIDTH_M * h;
        let (chord, lace, pitch) = (BRACING_LACING_M[0] * h, BRACING_LACING_M[1] * h, BRACING_LACING_M[2] * h);
        let axis = [f.axis[0], 0.0, f.axis[1]];
        // one member: two flanges the member's full width along the span, `depth`
        // apart in the tower's plane, laced on both faces, battened at its ends.
        let mut member = |a: [f64; 3], b: [f64; 3], depth: f64| {
            let (dx, dy, dz) = (b[0] - a[0], b[1] - a[1], b[2] - a[2]);
            let l = (dx * dx + dy * dy + dz * dz).sqrt();
            let d = [dx / l, dy / l, dz / l];
            // in the tower's plane, perpendicular to the member: axis x d.
            let mut up = [axis[1] * d[2] - axis[2] * d[1], axis[2] * d[0] - axis[0] * d[2], axis[0] * d[1] - axis[1] * d[0]];
            let ul = (up[0] * up[0] + up[1] * up[1] + up[2] * up[2]).sqrt();
            up = [up[0] / ul, up[1] / ul, up[2] / ul];
            let at = |p: [f64; 3], o: f64, along: f64| [p[0] + up[0] * o + axis[0] * along, p[1] + up[1] * o, p[2] + up[2] * o + axis[2] * along];
            let half = (depth - chord) / 2.0;
            for o in [-half, half] {
                orange.beam_up(at(a, o, 0.0), at(b, o, 0.0), width, chord, k.orange, Some(up));
            }
            let n = ((l / pitch).round() as usize).max(1);
            let p = |t: f64, o: f64, along: f64| at([a[0] + dx * t, a[1] + dy * t, a[2] + dz * t], o, along);
            for face in [-1.0, 1.0] {
                let along = face * (width / 2.0 - lace / 2.0);
                for i in 0..n {
                    let (t0, t1) = (i as f64 / n as f64, (i + 1) as f64 / n as f64);
                    let (o0, o1) = if i % 2 == 0 { (-half, half) } else { (half, -half) };
                    orange.beam_up(p(t0, o0, along), p(t1, o1, along), lace, lace * 0.5, k.orange, Some(axis));
                }
            }
            for t in [0.0, 1.0] {
                orange.beam_up(p(t, -half, 0.0), p(t, half, 0.0), width, chord, k.orange, Some(d));
            }
        };
        let inner_at = |y: f64| hp - self.stage_at(y - w)[2] / 2.0;
        // the struts, between the legs' inner faces at their mid-height.
        for st in [s5, s6, s7] {
            let ym = w + (st[0] + st[1]) / 2.0;
            let s = inner_at(ym);
            member(f.P(c, -s, ym), f.P(c, s, ym), st[1] - st[0]);
            if st[1] == s5[1] {
                let [x, z] = f.point(c, 0.0);
                far.box_(x, z, w + st[0], w + st[1], width / 2.0, hp, self.yaw, k.orange, ALL);
            }
        }
        // the diagonals, corner to corner of each panel (the struts' facing faces at the legs).
        for (lo, hi) in [(s7[1], s6[0]), (s6[1], s5[0])] {
            let (ya, yb) = (w + lo, w + hi);
            let depth = BRACING_DIAGONAL_M * h;
            let (sa, sb) = (inner_at(ya) - depth / 2.0, inner_at(yb) - depth / 2.0);
            member(f.P(c, -sa, ya + depth / 2.0), f.P(c, sb, yb - depth / 2.0), depth);
            member(f.P(c, sa, ya + depth / 2.0), f.P(c, -sb, yb - depth / 2.0), depth);
        }
    }

    /// The leg stage (h0, h1, across, along; heights over the water) at a height over the water.
    fn stage_at(&self, y: f64) -> [f64; 4] {
        let st = &self.g.leg_stages;
        *st.iter().find(|s| y >= s[0] && y < s[1]).unwrap_or(if y < st[0][0] { &st[0] } else { &st[st.len() - 1] })
    }

    /// The portal struts above the roadway as their steel casings
    /// (`BridgeFeatures::haunch`, with `detail`): see `PortalCasing`.
    fn portals(&self) -> Vec<PortalCasing> {
        if !(self.haunch && self.detail) {
            return Vec::new();
        }
        let (g, w, h) = (self.g, self.water, self.h);
        let above: Vec<usize> = (0..g.struts.len()).filter(|&i| w + g.struts[i][0] > self.plan.y_tower).collect();
        let mut out = Vec::new();
        for (k, &i) in above.iter().enumerate() {
            let [h0, h1, _] = g.struts[i];
            // the opening under the strut: down to the next strut, or to the roadway.
            let below = if k > 0 { g.struts[above[k - 1]][1] } else { self.plan.y_tower - w };
            let (height, opening) = (h1 - h0, h0 - below);
            // grow the casing top and bottom so it stands PORTAL_DEPTH_RATIO of the opening under it.
            let grow = ((PORTAL_DEPTH_RATIO * opening - height) / (2.0 + 2.0 * PORTAL_DEPTH_RATIO)).max(0.0);
            let stage = self.stage_at((h0 + h1) / 2.0);
            out.push(PortalCasing {
                index: i,
                y0: w + h0 - grow,
                y1: w + h1 + grow,
                along: stage[3] - 2.0 * CASING_SETBACK_M * h,
                top_haunch: k + 1 < above.len(),
            });
        }
        out
    }

    /// One portal strut's casing (`BridgeFeatures::haunch`): the steel facia
    /// plates that case the bracing ("transverse cross strut steel façade
    /// covers": the District's 2025 retrofit scope; "facia plates" blending
    /// with the stepped lines of the legs: Strauss 1937) as a box between the
    /// legs nearly flush with their faces, a lip projecting along its top,
    /// the vertical ribbing on both faces, and the "angular, stepped brackets
    /// on the horizontal struts" (goldengate.org, "Art Deco on a Grand
    /// Scale") as stepped haunches in the openings' corners, under every
    /// casing and over every casing with another above it. The far item takes
    /// the casing and one block a haunch.
    #[allow(clippy::too_many_arguments)]
    fn portal(&self, orange: &mut MeshArrays, far: &mut MeshArrays, c: f64, mx: f64, mz: f64, span: f64, p: &PortalCasing) {
        let (k, h, yaw, w) = (&self.k, self.h, self.yaw, self.water);
        orange.box_(mx, mz, p.y0, p.y1, p.along / 2.0, span / 2.0, yaw, k.orange, ALL);
        far.box_(mx, mz, p.y0, p.y1, p.along / 2.0, span / 2.0, yaw, k.orange, ALL);
        let hp = self.site.half_plane;
        // the lip, between the legs' inner faces at the casing's top.
        let top_stage = self.stage_at(p.y1 - w - 0.01);
        let clear_top = hp - top_stage[2] / 2.0;
        let (lip_h, lip_out) = (CASING_LIP_M[0] * h, CASING_LIP_M[1] * h);
        orange.box_(mx, mz, p.y1 - lip_h, p.y1, p.along / 2.0 + lip_out, clear_top, yaw, k.orange, ALL);
        self.portal_ribs(orange, c, p.y0, p.y1 - lip_h, p.along);
        // the haunches: HAUNCH_STEPS steps, each 1 / HAUNCH_STEPS of the reach out from the leg and down (or up) the leg.
        let recess = CASING_SETBACK_M * h;
        let mut corner = |y_face: f64, dir: f64| {
            let stage = self.stage_at(y_face - w + dir * 0.5);
            let inner = hp - stage[2] / 2.0;
            let r = HAUNCH_RATIO * 2.0 * inner;
            let n = HAUNCH_STEPS;
            for sgn in [-1.0, 1.0] {
                for i in 0..n {
                    let reach = r * (n - i) as f64 / n as f64;
                    let (ya, yb) = (y_face + dir * r * i as f64 / n as f64, y_face + dir * r * (i + 1) as f64 / n as f64);
                    let [x, z] = self.site.axis_point(c, sgn * (inner - reach / 2.0));
                    orange.box_(x, z, ya.min(yb), ya.max(yb), p.along / 2.0 - recess, reach / 2.0, yaw, k.orange, ALL);
                }
                let [x, z] = self.site.axis_point(c, sgn * (inner - r / 4.0));
                let (ya, yb) = (y_face, y_face + dir * r * 0.75);
                far.box_(x, z, ya.min(yb), ya.max(yb), p.along / 2.0 - recess, r / 4.0, yaw, k.orange, ALL);
            }
        };
        corner(p.y0, -1.0);
        if p.top_haunch {
            corner(p.y1, 1.0);
        }
    }

    /// The "wide, vertical ribbing" on the horizontal tower bracing above the
    /// roadway (goldengate.org, "Color & Art Deco Styling": added to catch the
    /// sun): fins on both faces of a portal strut, between the legs' inner
    /// faces. INFERRED: fin width 1.2 m, 0.35 m proud, at a 3 m pitch (x the
    /// structure scale), the strut's full height less 0.4 m top and bottom.
    fn portal_ribs(&self, orange: &mut MeshArrays, c: f64, y0: f64, y1: f64, along: f64) {
        let g = self.g;
        let w = self.water;
        let h = self.h;
        let stage = g.leg_stages.iter().find(|st| y0 - w >= st[0] && y0 - w < st[1]).unwrap_or(&g.leg_stages[g.leg_stages.len() - 1]);
        let clear = self.site.half_plane - stage[2] / 2.0;
        let (fin_w, proud, pitch, lip) = (1.2 * h, 0.35 * h, 3.0 * h, 0.4 * h);
        let n = ((2.0 * clear - fin_w) / pitch).floor().max(1.0) as i64;
        let first = -(n as f64) * pitch / 2.0;
        for side in [-1.0, 1.0] {
            let cf = c + side * (along / 2.0 + proud / 2.0);
            for i in 0..=n {
                let [x, z] = self.site.axis_point(cf, first + i as f64 * pitch);
                orange.box_(x, z, y0 + lip, y1 - lip, proud / 2.0, fin_w / 2.0, self.yaw, self.k.orange_dark, NO_BOTTOM);
            }
        }
    }

    fn cables(&self, wires: &mut MeshArrays, far: &mut MeshArrays) -> usize {
        let runs = self.cable_runs();
        let d = self.d;
        suspension::main_cables(wires, far, self.frame, &runs, &CableSpec { half_plane: self.site.half_plane, diameter: num(d, "cableDiameterM"), colour: self.k.orange, far_beams: !self.wires });
        let p = self.plan;
        suspension::suspender_pairs(
            wires,
            self.frame,
            &runs,
            &|c| p.deck_top(c) - 0.5,
            &SuspenderSpec { half_plane: self.site.half_plane, spacing: num(d, "suspenderSpacingM"), diameter: num(d, "suspenderDiameterM"), pair_gap: PAIR_GAP_M, colour: self.k.orange },
        )
    }

    fn truss(&self, orange: &mut MeshArrays, far: &mut MeshArrays) {
        let (d, p) = (self.d, self.plan);
        suspension::stiffening_truss(
            orange,
            far,
            self.frame,
            &TrussSpec {
                deck_top: &|c| p.deck_top(c),
                c_start: p.c_start,
                c_end: p.c_end,
                half_plane: self.site.half_plane,
                depth: num(d, "trussDepthM"),
                panel: num(d, "panelM"),
                colour: self.k.orange,
                dark: self.k.orange_dark,
                gap: &|a, b| self.in_tower_zone(a, b),
                far_half_width: num(d, "widthM") / 2.0,
                far_above: num(d, "railingM"),
                member: self.h,
            },
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn approach_span(&self, orange: &mut MeshArrays, concrete: &mut MeshArrays, far: &mut MeshArrays, from: f64, to: f64, seat_at: f64) {
        let (d, p, k) = (self.d, self.plan, &self.k);
        let (a, b) = (from.min(to), from.max(to));
        if !(b - a > 1.0) {
            return;
        }
        let depth = APPROACH_GIRDER_DEPTH_M * self.h;
        let panel = num(d, "panelM");
        let hp = self.site.half_plane;
        for s in [-hp, hp] {
            let st = stations(a, b, panel / 2.0);
            for &c in &st[..st.len() - 1] {
                let c1 = b.min(c + panel / 2.0);
                orange.beam(self.P(c, s, p.deck_top(c) - 0.3 - depth / 2.0), self.P(c1, s, p.deck_top(c1) - 0.3 - depth / 2.0), 0.5, depth, k.orange);
            }
        }
        let st = stations(a, b, panel);
        if st.len() > 2 {
            for &c in &st[1..st.len() - 1] {
                orange.beam(self.P(c, -hp, p.deck_top(c) - 0.3 - depth / 2.0), self.P(c, hp, p.deck_top(c) - 0.3 - depth / 2.0), 0.4, depth * 0.8, k.orange_dark);
            }
        }
        let [x, z] = self.site.deck_point(seat_at, 0.0);
        let seat_top = p.deck_top(seat_at) - 0.3 - depth;
        let seat_bottom = (seat_top - 0.5).min((self.ground)(x, z) - 0.5);
        let width = num(d, "widthM");
        concrete.box_(x, z, seat_bottom, seat_top, 1.0, width / 2.0 + 0.5, self.yaw, k.concrete, NO_BOTTOM);
        let [mx, mz] = self.site.deck_point((a + b) / 2.0, 0.0);
        let mean = (p.deck_top(a) + p.deck_top(b)) / 2.0;
        far.box_(mx, mz, mean - 0.3 - depth, mean + num(d, "railingM"), (b - a) / 2.0, width / 2.0, self.yaw, k.orange, ALL);
    }

    fn deck(&self, deck: &mut MeshArrays, orange: &mut MeshArrays, ca: &DeckStandards) {
        let (d, p, k) = (self.d, self.plan, &self.k);
        suspension::road_deck(
            deck,
            orange,
            self.frame,
            ca,
            &DeckSpec {
                y: &|c| p.deck_top(c),
                c_start: p.c_south,
                c_end: p.c_north,
                half_width: num(d, "widthM") / 2.0,
                roadway_m: num(d, "roadwayM"),
                lanes: 2 * num(d, "lanesPerDirection") as u32,
                median: true,
                sidewalk_m: num(d, "sidewalkM"),
                curb_m: num(d, "curbM"),
                railings: true,
                railing_m: num(d, "railingM"),
                inner_barrier_m: num(d, "innerBarrierM"),
                panel: num(d, "panelM"),
                gap: &|a, b| self.in_tower_zone(a, b),
                colours: DeckColours { asphalt: k.asphalt, sidewalk: k.sidewalk, soffit: self.soffit(), barrier: k.barrier, white: k.white, yellow: k.yellow, rail: k.orange },
                markings: true,
            },
        );
    }

    /// The deck plate's underside: with `detail` the orthotropic steel deck
    /// that replaced the concrete deck and its stringers (747 sections over
    /// 401 nights, 1982-1986: goldengate.org, "Major Bridge Improvements"),
    /// painted as the rest of the steel (in shadow: 0.8 of it); else the
    /// export's asphalt shade.
    fn soffit(&self) -> Rgb {
        if self.detail { shade(self.k.orange, 0.8) } else { shade(self.k.asphalt, 0.6) }
    }

    fn tower_walkways(&self, deck: &mut MeshArrays, orange: &mut MeshArrays) {
        let (g, d, p, k, w) = (self.g, self.d, self.plan, &self.k, self.water);
        let stage = g.leg_stages.iter().find(|s| p.y_tower - w >= s[0] && p.y_tower - w < s[1]).unwrap_or(&g.leg_stages[2]);
        let (leg_half_across, leg_half_along) = (stage[2] / 2.0, stage[3] / 2.0);
        let curb = num(d, "roadwayM") / 2.0;
        let half = num(d, "widthM") / 2.0;
        let hp = self.site.half_plane;
        let leg_inner = hp - leg_half_across;
        let leg_outer = hp + leg_half_across;
        let walk_outer = leg_outer + num(d, "sidewalkM");
        let zone = self.tower_zone_half;
        let (curb_m, railing, inner_barrier) = (num(d, "curbM"), num(d, "railingM"), num(d, "innerBarrierM"));
        let top = |c: f64| p.deck_top(c) + curb_m;
        let soffit = self.soffit();
        let plate = |deck: &mut MeshArrays, c0: f64, c1: f64, s0: f64, s1: f64| {
            deck.quad(self.P(c0, s0, top(c0)), self.P(c0, s1, top(c0)), self.P(c1, s1, top(c1)), self.P(c1, s0, top(c1)), k.sidewalk);
            deck.quad(self.P(c1, s0, top(c1) - 0.5), self.P(c1, s1, top(c1) - 0.5), self.P(c0, s1, top(c0) - 0.5), self.P(c0, s0, top(c0) - 0.5), soffit);
        };
        for t in [0.0, p.l] {
            for sgn in [-1.0, 1.0] {
                let span = |a: f64, b: f64| if sgn > 0.0 { (a, b) } else { (-b, -a) };
                let (s0, s1) = span(curb, walk_outer);
                plate(deck, t - zone, t - leg_half_along, s0, s1);
                plate(deck, t + leg_half_along, t + zone, s0, s1);
                let (s0, s1) = span(leg_outer, walk_outer);
                plate(deck, t - leg_half_along, t + leg_half_along, s0, s1);
                let [mx, mz] = self.site.deck_point(t, sgn * (curb + leg_inner) / 2.0);
                deck.box_(mx, mz, p.deck_top(t), p.deck_top(t) + inner_barrier, leg_half_along, (leg_inner - curb) / 2.0 + 0.05, self.yaw, k.barrier, NO_BOTTOM);
                let rail = |orange: &mut MeshArrays, c0: f64, s0: f64, c1: f64, s1: f64| {
                    orange.beam(self.P(c0, s0, top(c0) + railing), self.P(c1, s1, top(c1) + railing), 0.12, 0.12, k.orange);
                    orange.beam(self.P(c0, s0, top(c0) + railing * 0.5), self.P(c1, s1, top(c1) + railing * 0.5), 0.08, 0.08, k.orange);
                    orange.beam(self.P(c1, s1, top(c1)), self.P(c1, s1, top(c1) + railing), 0.14, 0.14, k.orange);
                };
                let edge = sgn * (walk_outer - 0.15);
                let mut c = t - zone;
                while c < t + zone - 0.01 {
                    rail(orange, c, edge, (t + zone).min(c + zone / 2.0), edge);
                    c += zone / 2.0;
                }
                for end in [t - zone, t + zone] {
                    rail(orange, end, sgn * (half - 0.15), end, edge);
                }
            }
        }
    }

    /// `bentColumns`: (x, z, ground, top) of a two-column bent at chainage c.
    fn bent_columns(&self, c: f64) -> [[f64; 4]; 2] {
        let hp = self.site.half_plane;
        let top = self.plan.deck_top(c) - num(self.d, "trussDepthM");
        // (`pylons`) a bent over an anchorage housing stands on its roof.
        let roof = self.housing_under(c);
        [-hp, hp].map(|s| {
            let [x, z] = self.site.deck_point(c, s);
            [x, z, roof.map_or((self.ground)(x, z), |r| r.max((self.ground)(x, z))), top]
        })
    }

    fn bent_clear(&self, c: f64) -> bool {
        let [cw, cd] = pair(self.d, "bentColumnM");
        let reach = (cw / 2.0 + 0.6).hypot(cd / 2.0 + 0.6);
        let hp = self.site.half_plane;
        for s in [-hp, hp] {
            let [x, z] = self.site.deck_point(c, s);
            for way in &self.g.keep_clear {
                if polyline_distance(x, z, &way.points) < way.half + reach {
                    return false;
                }
            }
        }
        true
    }

    /// `bentStations`.
    fn bent_stations(&self) -> Vec<f64> {
        let (g, d, p) = (self.g, self.d, self.plan);
        let spacing = num(d, "bentSpacingMaxM");
        let place = |list: Vec<f64>| {
            let mut out = Vec::new();
            for c in list {
                let mut placed = None;
                let mut dd = 0.0;
                while dd <= spacing / 2.0 && placed.is_none() {
                    let cands: Vec<f64> = if dd != 0.0 { vec![c - dd, c + dd] } else { vec![c] };
                    for cc in cands {
                        if placed.is_none() && self.bent_clear(cc) {
                            placed = Some(cc);
                        }
                    }
                    dd += 0.5;
                }
                if let Some(c) = placed {
                    out.push(c);
                }
            }
            out
        };
        let mut south = vec![];
        let first_s = p.c_start + 4.0;
        let last_s = p.arch_south_c - g.supports["archSouth"].extent[1];
        if last_s > first_s {
            south = stations(first_s, last_s, spacing);
        }
        let mut north = vec![];
        let first_n = p.anchor_north_c + g.supports["northAnchor"].extent[1];
        let last_n = p.c_end - 4.0;
        if last_n > first_n {
            north = stations(first_n, last_n, spacing);
        }
        let mut out = place(south);
        out.extend(place(north));
        out
    }

    fn approaches(&self, orange: &mut MeshArrays, concrete: &mut MeshArrays, far: &mut MeshArrays) {
        let [cw, cd] = pair(self.d, "bentColumnM");
        let k = &self.k;
        for c in self.bent_stations() {
            let cols = self.bent_columns(c);
            for &[x, z, ground, top] in &cols {
                if top - ground < 1.0 {
                    continue;
                }
                orange.box_(x, z, ground + 1.2, top, cw / 2.0, cd / 2.0, self.yaw, k.orange, ALL);
                concrete.box_(x, z, ground - 1.0, ground + 1.2, cw / 2.0 + 0.6, cd / 2.0 + 0.6, self.yaw, k.concrete, NO_BOTTOM);
                far.box_(x, z, ground, top, cw / 2.0, cd / 2.0, self.yaw, k.orange, ALL);
            }
            let [a, b] = cols;
            let top = a[3].min(b[3]);
            orange.beam([a[0], top - 0.6, a[1]], [b[0], top - 0.6, b[1]], 1.0, 1.2, k.orange_dark);
            let low = a[2].max(b[2]) + 3.0;
            if top - low > 8.0 {
                orange.beam([a[0], low, a[1]], [b[0], top - 1.5, b[1]], 0.5, 0.5, k.orange_dark);
                orange.beam([b[0], low, b[1]], [a[0], top - 1.5, a[1]], 0.5, 0.5, k.orange_dark);
            }
        }
    }

    /// `pylonParts`: the arch pylons as twin columns under a cap, and the north anchorage.
    fn pylon_parts(&self) -> Vec<(String, [f64; 7], bool)> {
        let (g, d, p) = (self.g, self.d, self.plan);
        let mut parts = Vec::new();
        for key in ["archSouth", "archNorth"] {
            let support = &g.supports[key];
            let c = self.site.chainage(support.centre[0], support.centre[1]);
            let half_outer = support.extent[0] / 2.0;
            let inner = self.site.half_plane + 0.5;
            let width = (half_outer - inner).min(ARCH_PYLON_COLUMN_M * self.h);
            let top = p.deck_top(c) + num(d, "pylonAboveDeckM");
            for sgn in [-1.0, 1.0] {
                let [x, z] = self.site.deck_point(c, sgn * (inner + width / 2.0));
                let side = if sgn < 0.0 { "west" } else { "east" };
                parts.push((format!("{key}-{side}"), [x, z, (self.ground)(x, z) - 1.0, top, support.extent[1] / 2.0, width / 2.0, self.yaw], false));
            }
            let [x, z] = self.site.deck_point(c, 0.0);
            parts.push((format!("{key}-cap"), [x, z, top - 4.0, top, support.extent[1] / 2.0, inner + width, self.yaw], true));
        }
        let a = &g.supports["northAnchor"];
        let ca = self.site.chainage(a.centre[0], a.centre[1]);
        parts.push((
            "northAnchor".to_string(),
            [a.centre[0], a.centre[1], (self.ground)(a.centre[0], a.centre[1]) - 1.0, p.deck_top(ca) + num(d, "pylonAboveDeckM"), a.extent[1] / 2.0, a.extent[0] / 2.0, self.yaw],
            false,
        ));
        parts
    }

    fn arch_ribs(&self, orange: &mut MeshArrays, far: &mut MeshArrays) {
        if self.arch2 {
            self.arch4(orange, far);
            return;
        }
        let (g, d, p, k) = (self.g, self.d, self.plan, &self.k);
        let s_c = p.arch_south_c + g.supports["archSouth"].extent[1] / 2.0;
        let n_c = p.arch_north_c - g.supports["archNorth"].extent[1] / 2.0;
        let truss = num(d, "trussDepthM");
        let panel = num(d, "panelM");
        let hp = self.site.half_plane;
        for s in [-hp, hp] {
            let ps = self.site.deck_point(s_c, s);
            let pn = self.site.deck_point(n_c, s);
            let y_s = (self.ground)(ps[0], ps[1]) + 0.5;
            let y_n = (self.ground)(pn[0], pn[1]) + 0.5;
            let crown_at = |c: f64| p.deck_top(c) - truss - 1.2;
            let rib_y = |c: f64| {
                let t = (c - s_c) / (n_c - s_c);
                let chord = y_s + (y_n - y_s) * t;
                let crown = crown_at((s_c + n_c) / 2.0) - (y_s + y_n) / 2.0;
                chord + crown * 4.0 * t * (1.0 - t)
            };
            if self.members {
                self.arch_truss(orange, s, s_c, n_c, &rib_y);
                let fcs = stations(s_c, n_c, 16.0);
                for i in 0..fcs.len() - 1 {
                    far.beam(self.P(fcs[i], s, rib_y(fcs[i])), self.P(fcs[i + 1], s, rib_y(fcs[i + 1])), 2.0, 2.6, k.orange);
                }
                continue;
            }
            let cs = stations(s_c, n_c, 4.0);
            for i in 0..cs.len() - 1 {
                orange.beam(self.P(cs[i], s, rib_y(cs[i])), self.P(cs[i + 1], s, rib_y(cs[i + 1])), 1.6, 2.4, k.orange);
            }
            let fcs = stations(s_c, n_c, 16.0);
            for i in 0..fcs.len() - 1 {
                far.beam(self.P(fcs[i], s, rib_y(fcs[i])), self.P(fcs[i + 1], s, rib_y(fcs[i + 1])), 2.0, 2.6, k.orange);
            }
            for c in stations(s_c + panel, n_c - panel, panel) {
                let y_top = p.deck_top(c) - truss;
                if y_top - rib_y(c) > 1.0 {
                    orange.beam(self.P(c, s, rib_y(c)), self.P(c, s, y_top), 0.6, 0.6, k.orange);
                }
            }
        }
    }

    /// The Fort Point arch's rib as the open-web steel arch it is
    /// (`BridgeFeatures::members`; the arch spans 320 ft over the fort:
    /// goldengate.org, "Fort Point Arch"; the export's `archSpanM`): a
    /// two-hinged arch truss whose bottom chord is the rib's line and whose
    /// top chord stands `ARCH_TRUSS_DEPTH_M` over it at the crown, closing
    /// to the hinges at the springings; verticals at each half panel,
    /// alternating diagonals, and the spandrel posts from the top chord up
    /// to the deck truss. INFERRED: the depth and the members' sections.
    fn arch_truss(&self, orange: &mut MeshArrays, s: f64, s_c: f64, n_c: f64, rib_y: &dyn Fn(f64) -> f64) {
        let (d, p, k, h) = (self.d, self.plan, &self.k, self.h);
        let truss = num(d, "trussDepthM");
        let depth = ARCH_TRUSS_DEPTH_M * h;
        let top_at = |c: f64| {
            let t = ((c - s_c) / (n_c - s_c)).clamp(0.0, 1.0);
            let y = rib_y(c) + depth * (std::f64::consts::PI * t).sin().sqrt();
            y.min(p.deck_top(c) - truss - 0.3)
        };
        let panel = num(d, "panelM") / 2.0;
        let cs = stations(s_c, n_c, panel);
        let (chord_w, chord_d) = (1.2 * h, 1.4 * h);
        for i in 0..cs.len() - 1 {
            let (a, b) = (cs[i], cs[i + 1]);
            orange.beam(self.P(a, s, rib_y(a)), self.P(b, s, rib_y(b)), chord_w, chord_d, k.orange);
            orange.beam(self.P(a, s, top_at(a)), self.P(b, s, top_at(b)), chord_w, chord_d * 0.8, k.orange);
            if top_at(b) - rib_y(b) > 0.6 {
                orange.beam(self.P(b, s, rib_y(b)), self.P(b, s, top_at(b)), 0.5 * h, 0.6 * h, k.orange_dark);
            }
            let (ya, yb) = if i % 2 == 0 { (rib_y(a), top_at(b)) } else { (top_at(a), rib_y(b)) };
            if (top_at(a) - rib_y(a)).min(top_at(b) - rib_y(b)) > 0.3 {
                orange.beam(self.P(a, s, ya), self.P(b, s, yb), 0.4 * h, 0.5 * h, k.orange_dark);
            }
        }
        // lateral struts between the two ribs at every other panel point.
        let hp = self.site.half_plane;
        if s > 0.0 {
            for (i, &c) in cs.iter().enumerate() {
                if i % 2 == 0 && i > 0 && i + 1 < cs.len() {
                    orange.beam(self.P(c, -hp, top_at(c) - 0.5 * h), self.P(c, hp, top_at(c) - 0.5 * h), 0.6 * h, 0.8 * h, k.orange_dark);
                }
            }
        }
        for c in stations(s_c + num(d, "panelM"), n_c - num(d, "panelM"), num(d, "panelM")) {
            let y_top = p.deck_top(c) - truss;
            if y_top - top_at(c) > 1.0 {
                orange.beam(self.P(c, s, top_at(c)), self.P(c, s, y_top), 0.6, 0.6, k.orange);
            }
        }
    }

    /// The Fort Point arch as built (`BridgeFeatures::arch2`): a steel,
    /// two-hinged arch of four ribs, the inner pair 42 ft 6 in apart and the
    /// outer pair 102 ft 6 in apart (6 ft 3 in outside the cable backstays),
    /// spanning 319 ft 7 in between the bearings' centres with a rise of
    /// 113 ft, the roadway 185 ft over the springing (Strauss 1937; the plan
    /// widths and the span times the ground plan's scale, the heights times
    /// the structure's). Each rib an open-web arch truss (its bottom chord on
    /// the rib's parabola, its top chord ARCH_TRUSS_DEPTH_M over it at the
    /// crown, closing to the hinges), verticals and alternating diagonals at
    /// every half panel; spandrel bents (a post on each rib at every panel
    /// point, up to the stiffening truss's bottom chord, X-braced across
    /// between the ribs); laterals (X between neighbouring ribs' top chords,
    /// struts across all four at the bottom chords). The springing line is
    /// level, 185 ft under the roadway at the north bearing. INFERRED: the
    /// ribs' depth, the members' sections, the bracing's pattern.
    fn arch4(&self, orange: &mut MeshArrays, far: &mut MeshArrays) {
        let (d, p, k, h) = (self.d, self.plan, &self.k, self.h);
        let gs = eo_data::groundscale::scale();
        let ft = 0.3048;
        let mid = (p.arch_south_c + p.arch_north_c) / 2.0;
        let half_span = ARCH_SPAN_FT * ft * gs / 2.0;
        let (b0, b1) = (mid - half_span, mid + half_span);
        let truss = num(d, "trussDepthM");
        let spring = p.deck_top(b1) - ARCH_ROADWAY_OVER_SPRINGING_FT * ft * h;
        let rise = ARCH_RISE_FT * ft * h;
        let rib_y = |c: f64| {
            let t = ((c - b0) / (b1 - b0)).clamp(0.0, 1.0);
            spring + rise * 4.0 * t * (1.0 - t)
        };
        let depth = ARCH_TRUSS_DEPTH_M * h;
        let floor = |c: f64| p.deck_top(c) - truss;
        let top_at = |c: f64| {
            let t = ((c - b0) / (b1 - b0)).clamp(0.0, 1.0);
            (rib_y(c) + depth * (std::f64::consts::PI * t).sin().sqrt()).min(floor(c) - 0.3)
        };
        let ribs = ARCH_RIBS_FT.map(|f| f * ft * gs / 2.0);
        let ss = [-ribs[1], -ribs[0], ribs[0], ribs[1]];
        let panel = num(d, "panelM");
        let cs = stations(b0, b1, panel / 2.0);
        let (chord_w, chord_d) = (1.2 * h, 1.4 * h);
        for &s in &ss {
            for i in 0..cs.len() - 1 {
                let (a, b) = (cs[i], cs[i + 1]);
                orange.beam(self.P(a, s, rib_y(a)), self.P(b, s, rib_y(b)), chord_w, chord_d, k.orange);
                orange.beam(self.P(a, s, top_at(a)), self.P(b, s, top_at(b)), chord_w, chord_d * 0.8, k.orange);
                if top_at(b) - rib_y(b) > 0.6 {
                    orange.beam(self.P(b, s, rib_y(b)), self.P(b, s, top_at(b)), 0.5 * h, 0.6 * h, k.orange);
                }
                let (ya, yb) = if i % 2 == 0 { (rib_y(a), top_at(b)) } else { (top_at(a), rib_y(b)) };
                if (top_at(a) - rib_y(a)).min(top_at(b) - rib_y(b)) > 0.3 {
                    orange.beam(self.P(a, s, ya), self.P(b, s, yb), 0.4 * h, 0.5 * h, k.orange);
                }
            }
            let fcs = stations(b0, b1, 16.0);
            for i in 0..fcs.len() - 1 {
                far.beam(self.P(fcs[i], s, rib_y(fcs[i])), self.P(fcs[i + 1], s, rib_y(fcs[i + 1])), 2.0, 2.6, k.orange);
            }
        }
        // laterals: X between neighbouring ribs' top chords in every other panel, struts across at the bottom chords.
        for (i, w) in cs.windows(2).enumerate() {
            let (a, b) = (w[0], w[1]);
            if i % 2 == 0 && i > 0 && i + 2 < cs.len() {
                for j in 0..3 {
                    let (s0, s1) = (ss[j], ss[j + 1]);
                    orange.beam(self.P(a, s0, top_at(a) - 0.4 * h), self.P(b, s1, top_at(b) - 0.4 * h), 0.3 * h, 0.4 * h, k.orange);
                    orange.beam(self.P(a, s1, top_at(a) - 0.4 * h), self.P(b, s0, top_at(b) - 0.4 * h), 0.3 * h, 0.4 * h, k.orange);
                }
                orange.beam(self.P(a, ss[0], rib_y(a) + 0.4 * h), self.P(a, ss[3], rib_y(a) + 0.4 * h), 0.4 * h, 0.5 * h, k.orange);
            }
        }
        // spandrel bents at the panel points: a post on each rib, X-braced across between the ribs.
        for c in stations(b0 + panel, b1 - panel, panel) {
            let (y0, y1) = (top_at(c), floor(c));
            if y1 - y0 < 1.0 {
                continue;
            }
            for &s in &ss {
                orange.beam(self.P(c, s, y0), self.P(c, s, y1), 0.7 * h, 0.8 * h, k.orange);
            }
            if y1 - y0 > 3.0 {
                for j in 0..3 {
                    let (s0, s1) = (ss[j], ss[j + 1]);
                    orange.beam(self.P(c, s0, y0 + 0.5), self.P(c, s1, y1 - 0.5), 0.3 * h, 0.35 * h, k.orange);
                    orange.beam(self.P(c, s1, y0 + 0.5), self.P(c, s0, y1 - 0.5), 0.3 * h, 0.35 * h, k.orange);
                }
            }
            // the bent's cap under the deck truss.
            orange.beam(self.P(c, ss[0], y1 - 0.4 * h), self.P(c, ss[3], y1 - 0.4 * h), 0.6 * h, 0.8 * h, k.orange);
        }
    }

    /// The arch's bearings (`BridgeFeatures::arch2`): a concrete pedestal
    /// under each rib's hinge, from the ground up to the springing line.
    /// INFERRED: its plan, 3 m square at the structure's scale.
    fn arch_bearings(&self, concrete: &mut MeshArrays) {
        let p = self.plan;
        let (gs, h, ft) = (eo_data::groundscale::scale(), self.h, 0.3048);
        let mid = (p.arch_south_c + p.arch_north_c) / 2.0;
        let half_span = ARCH_SPAN_FT * ft * gs / 2.0;
        let spring = p.deck_top(mid + half_span) - ARCH_ROADWAY_OVER_SPRINGING_FT * ft * h;
        let ribs = ARCH_RIBS_FT.map(|f| f * ft * gs / 2.0);
        let half = 1.5 * h;
        for c in [mid - half_span, mid + half_span] {
            for s in [-ribs[1], -ribs[0], ribs[0], ribs[1]] {
                let [x, z] = self.frame.point(c, s);
                let ground = (self.ground)(x, z);
                if spring - ground > 0.1 {
                    concrete.box_(x, z, ground - 1.0, spring, half, half, self.yaw, self.k.concrete, NO_BOTTOM);
                }
            }
        }
    }

    /// The parts the second pass adds (`BridgeFeatures::members`): the
    /// cable saddles' housings, the cable bands and the ropes' sockets, the
    /// pylons' Art Deco pilasters, the south pier's fender and the lamp
    /// standards along the roadway.
    fn members(&self, orange: &mut MeshArrays, concrete: &mut MeshArrays, skirt: &mut MeshArrays, wires: &mut MeshArrays, far: &mut MeshArrays) {
        let (g, d, p, k, w, h) = (self.g, self.d, self.plan, &self.k, self.water, self.h);
        // Saddle housings over each leg's top (the saddles 2 m under it: the export's `saddleBelowTopM`).
        let top = w + num(d, "towerAboveWaterM");
        let last = g.leg_stages.iter().fold(g.leg_stages[0], |a, s| if s[1] > a[1] { *s } else { a });
        for tc in [0.0, p.l] {
            for sgn in [-1.0, 1.0] {
                suspension::saddle_housing(orange, self.tower_frame, tc, sgn * self.site.half_plane, top, last[3] / 2.0 * 0.92, last[2] / 2.0 * 0.95, SADDLE_PLINTH_M * h, k.orange);
            }
        }
        // Cable bands and sockets.
        let runs = self.cable_runs();
        suspension::cable_bands(
            wires,
            self.frame,
            &runs,
            &|c| p.deck_top(c) - 0.5,
            &suspension::BandSpec {
                half_plane: self.site.half_plane,
                spacing: num(d, "suspenderSpacingM"),
                pair_gap: PAIR_GAP_M,
                cable_d: num(d, "cableDiameterM"),
                band_len: CABLE_BAND_LEN_M,
                band_d: num(d, "cableDiameterM") + 2.0 * CABLE_BAND_WALL_M,
                socket_len: SOCKET_LEN_M,
                socket_d: SOCKET_D_M,
                colour: k.orange,
            },
        );
        // The pylons' pilasters: three proud strips on each column's two faces along the span and its outer face.
        for (name, [x, z, y0, y1, hx, hz, yaw], cap) in self.pylon_parts() {
            if cap || self.pylons {
                continue;
            }
            let (cy, sy) = (yaw.cos(), yaw.sin());
            let at = |lx: f64, lz: f64| [x + lx * cy - lz * sy, z + lx * sy + lz * cy];
            let (pw, pd) = (PILASTER_W_M * h, PILASTER_PROUD_M * h);
            let top = y1 - PILASTER_TOP_CLEAR_M * h;
            for along in [-1.0, 1.0] {
                for kk in [-0.55, 0.0, 0.55] {
                    let [px, pz] = at(along * (hx + pd / 2.0), kk * hz);
                    concrete.box_(px, pz, y0 + 1.0, top, pd / 2.0, pw / 2.0, yaw, k.concrete, NO_BOTTOM);
                }
            }
            if !name.starts_with("northAnchor") {
                // a stepped band under the cap.
                concrete.box_(x, z, top + 0.4 * h, top + 1.6 * h, hx + pd * 1.5, hz + pd * 1.5, yaw, k.concrete, ALL);
            }
        }
        // The south tower's fender: the oval concrete wall poured round the pier
        // (goldengate.org, "A Bathtub for the South Tower"), enclosing an area the
        // size of a football field (a District inspection presentation, 2015:
        // walls 40 ft thick at most), sand-filled, 27 ft thick at its base and
        // 10 ft at sea level (the District's 2025 statement on the NTSB's
        // report). INFERRED: the oval's 110 x 52 m outside (a field and its end
        // zones, 360 x 160 ft, with the wall) and its top FENDER_TOP_M over the
        // sea; the wall drawn at its sea-level thickness.
        let pier = &g.supports["southPier"];
        let [cx, cz] = pier.centre;
        let (ra, rb) = (FENDER_OUTER_M[0] / 2.0, FENDER_OUTER_M[1] / 2.0);
        let wall = FENDER_WALL_TOP_M;
        let n = 64;
        let ring = |r_across: f64, r_along: f64| -> Vec<[f64; 2]> {
            (0..n)
                .map(|i| {
                    let t = std::f64::consts::TAU * i as f64 / n as f64;
                    let (ca, sa) = (t.cos() * r_along, t.sin() * r_across);
                    [cx + self.site.axis[0] * ca + self.site.across[0] * sa, cz + self.site.axis[1] * ca + self.site.across[1] * sa]
                })
                .collect()
        };
        let outer = crate::geometry::orient_ring(&ring(ra, rb));
        let inner = crate::geometry::orient_ring(&ring(ra - wall, rb - wall));
        let (floor, top_y) = (w + NEAR_FLOOR_ABOVE_SEA_M, w + FENDER_TOP_M);
        let bed = (self.ground)(cx, cz).min(w - 2.0);
        for i in 0..n {
            let j = (i + 1) % n;
            let (o0, o1, i0, i1) = (outer[i], outer[j], inner[i], inner[j]);
            // outer face, inner face, top.
            concrete.quad([o0[0], floor, o0[1]], [o1[0], floor, o1[1]], [o1[0], top_y, o1[1]], [o0[0], top_y, o0[1]], k.concrete);
            concrete.quad([i1[0], floor, i1[1]], [i0[0], floor, i0[1]], [i0[0], top_y, i0[1]], [i1[0], top_y, i1[1]], k.concrete);
            concrete.quad([o0[0], top_y, o0[1]], [o1[0], top_y, o1[1]], [i1[0], top_y, i1[1]], [i0[0], top_y, i0[1]], k.concrete);
            skirt.quad([o0[0], bed, o0[1]], [o1[0], bed, o1[1]], [o1[0], floor, o1[1]], [o0[0], floor, o0[1]], k.concrete);
            skirt.quad([i1[0], bed, i1[1]], [i0[0], bed, i0[1]], [i0[0], floor, i0[1]], [i1[0], floor, i1[1]], k.concrete);
            far.quad([o0[0], bed, o0[1]], [o1[0], bed, o1[1]], [o1[0], top_y, o1[1]], [o0[0], top_y, o0[1]], k.concrete);
            far.quad([o0[0], top_y, o0[1]], [o1[0], top_y, o1[1]], [i1[0], top_y, i1[1]], [i0[0], top_y, i0[1]], k.concrete);
        }
        // Lamp standards at the deck luminaires (lights.rs `bridge_lights`: their pitch and head),
        // on the curb line, the arm reaching over the roadway; none where the curb passes a tower leg.
        let half = num(d, "roadwayM") / 2.0;
        let head = crate::lights::DECK_HEAD_M;
        let mut c = p.c_start + crate::lights::DECK_PITCH_M / 2.0;
        while c < p.c_end {
            if !self.in_tower_zone(c - 2.0, c + 2.0) {
                let y = p.deck_top(c) + num(d, "curbM");
                for sgn in [-1.0, 1.0] {
                    lamp_standard(orange, self.frame, c, sgn * (half + 0.25), -sgn, y, head, k.orange);
                }
            }
            c += crate::lights::DECK_PITCH_M;
        }
    }

    fn colliders(&self) -> Vec<Collider> {
        let (g, d, w) = (self.g, self.d, self.water);
        let mut out = Vec::new();
        for (key, pier) in [("south", &g.supports["southPier"]), ("north", &g.supports["northPier"])] {
            let [cx, cz] = pier.centre;
            out.push(Collider {
                part: format!("{key}-pier"),
                x: cx,
                z: cz,
                y0: (self.ground)(cx, cz).min(w - 2.0),
                y1: w + pier.top_m.unwrap_or(0.0),
                hx: pier.extent[1] / 2.0,
                hz: pier.extent[0] / 2.0,
                yaw: self.yaw,
            });
        }
        if self.pylons {
            out.extend(self.pylon_colliders());
        }
        for (part, [x, z, y0, y1, hx, hz, yaw], cap_only) in self.pylon_parts() {
            if cap_only || self.pylons {
                continue;
            }
            out.push(Collider { part, x, z, y0, y1: y1.min(y0 + 30.0), hx, hz, yaw });
        }
        let [cw, cd] = pair(d, "bentColumnM");
        for c in self.bent_stations() {
            for (k, &[x, z, ground, top]) in self.bent_columns(c).iter().enumerate() {
                if top - ground < 1.0 {
                    continue;
                }
                out.push(Collider { part: format!("bent-{}-{k}", js_round(c)), x, z, y0: ground - 1.0, y1: top.min(ground + 8.0), hx: cw / 2.0 + 0.6, hz: cd / 2.0 + 0.6, yaw: self.yaw });
            }
        }
        out
    }
}

/// INFERRED (`BridgeFeatures::members`; none is published): the Fort Point
/// arch truss's depth at its crown; a saddle housing's plinth over the leg's
/// top; a cable band's length and wall (the band 1.2 m long, 8 cm thick
/// round the 0.92 m cable); a rope socket's length and diameter; the
/// pylons' pilasters (width, how proud, how far under the top they stop);
/// the south fender's oval outside (across the bridge, along it) and its top
/// over the sea. The fender's wall: 10 ft at sea level (the District's 2025
/// statement on the NTSB's report).
const ARCH_TRUSS_DEPTH_M: f64 = 4.5;
/// The Fort Point arch (`BridgeFeatures::arch2`; Strauss 1937): the span
/// between the bearings' centres (319 ft 7 in), its rise (113 ft), the
/// roadway over the springing at the north end (185 ft), the ribs' spacings
/// (inner pair, outer pair), feet.
const ARCH_SPAN_FT: f64 = 319.0 + 7.0 / 12.0;
const ARCH_RISE_FT: f64 = 113.0;
const ARCH_ROADWAY_OVER_SPRINGING_FT: f64 = 185.0;
const ARCH_RIBS_FT: [f64; 2] = [42.5, 102.5];
const SADDLE_PLINTH_M: f64 = 0.8;
const CABLE_BAND_LEN_M: f64 = 1.2;
const CABLE_BAND_WALL_M: f64 = 0.08;
const SOCKET_LEN_M: f64 = 0.9;
const SOCKET_D_M: f64 = 0.2;
const PILASTER_W_M: f64 = 0.9;
const PILASTER_PROUD_M: f64 = 0.35;
const PILASTER_TOP_CLEAR_M: f64 = 5.5;
const FENDER_OUTER_M: [f64; 2] = [110.0, 52.0];
const FENDER_WALL_TOP_M: f64 = 10.0 * 0.3048;
const FENDER_TOP_M: f64 = 4.5;

/// A lamp standard on a bridge's curb line (`BridgeFeatures::members`): a
/// pole from its base `head` metres up, a short arm reaching `toward` (+-1
/// across) over the roadway and the luminaire's housing at its end, over the
/// deck luminaire lights.rs places at the curb. INFERRED: the sections.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lamp_standard(m: &mut MeshArrays, frame: &Frame, c: f64, s: f64, toward: f64, y: f64, head: f64, colour: Rgb) {
    let [x, z] = frame.point(c, s);
    m.box_(x, z, y, y + 0.9, 0.3, 0.3, frame.yaw, colour, NO_BOTTOM);
    m.beam(frame.P(c, s, y + 0.9), frame.P(c, s, y + head - 0.25), 0.24, 0.24, colour);
    m.beam(frame.P(c, s, y + head - 0.6), frame.P(c, s + toward * 0.7, y + head), 0.12, 0.16, colour);
    m.beam(frame.P(c, s + toward * 0.3, y + head - 0.1), frame.P(c, s + toward * 1.2, y + head - 0.1), 0.42, 0.26, colour);
}

/// `Math.round` (halves toward +infinity).
fn js_round(v: f64) -> i64 {
    (v + 0.5).floor() as i64
}

/// Distance from (x, z) to a flat [x, z, ...] polyline.
fn polyline_distance(x: f64, z: f64, pts: &[f64]) -> f64 {
    let mut best = f64::INFINITY;
    let mut i = 0;
    while i + 3 < pts.len() {
        let (ax, az, dx, dz) = (pts[i], pts[i + 1], pts[i + 2] - pts[i], pts[i + 3] - pts[i + 1]);
        let l = dx * dx + dz * dz;
        let t = if l != 0.0 { (((x - ax) * dx + (z - az) * dz) / l).min(1.0).max(0.0) } else { 0.0 };
        best = best.min((x - ax - t * dx).hypot(z - az - t * dz));
        i += 2;
    }
    best
}

/// `buildGoldenGateBridge`.
pub fn build(js: &LandmarksJs, ground: &GroundGrid, ca: &DeckStandards) -> Built {
    let features = suspension::features();
    let gd = data(js);
    let g = &gd;
    let d = &g.dimensions;
    let site = Site::new(g);
    let ground_at = |x: f64, z: f64| ground.at(x, z);
    let plan = plan(js, &site, &ground_at);
    suspension::record_deck_line("golden-gate-bridge", [plan.c_south, plan.y_south, plan.l, plan.y_tower, plan.y_mid, plan.c_north, plan.y_north, num(d, "widthM") / 2.0]);
    let offset = |c: f64| site.centre_offset(c);
    let mut frame = Frame::new(site.axis_point(0.0, 0.0), site.axis);
    frame.offset = Some(&offset);
    let orange = srgb_hex(if features.paint { INTERNATIONAL_ORANGE_SRGB } else { hex(d, "orangeSrgb") });
    let k = Colours {
        orange,
        // `shade2`: every painted member in the one paint (the JS's 0.62 stood
        // for shade the renderer now draws: the struts read as dark grilles).
        orange_dark: if features.shade2 { orange } else { shade(orange, DARK) },
        asphalt: srgb_hex(hex(d, "asphaltSrgb")),
        sidewalk: srgb_hex(hex(d, "sidewalkSrgb")),
        concrete: srgb_hex(hex(d, "concreteSrgb")),
        barrier: srgb_hex(hex(d, "barrierSrgb")),
        white: srgb_hex(hex(d, "whiteSrgb")),
        yellow: srgb_hex(hex(d, "yellowSrgb")),
    };
    let tower_zone_half = g.leg_stages.iter().map(|s| s[3]).fold(f64::MIN, f64::max) / 2.0 + 1.0;
    let tower_frame = Frame::new(site.axis_point(0.0, 0.0), site.axis);
    let ctx = Ctx { g, d, site: &site, plan: &plan, frame: &frame, tower_frame: &tower_frame, ground: &ground_at, k, yaw: site.axis[1].atan2(site.axis[0]), water: js.sea_level_y, tower_zone_half, h: structure_scale(), detail: features.detail, wires: features.wires, members: features.members, haunch: features.haunch, bracing: features.bracing, pylons: features.pylons, arch2: features.arch2, fender2: features.fender2, collar: features.collar };
    let mut orange_m = MeshArrays::new("golden-gate-bridge:orange");
    let mut deck_m = MeshArrays::new("golden-gate-bridge:deck");
    let mut concrete_m = MeshArrays::new("golden-gate-bridge:concrete");
    let mut wires = MeshArrays::wire("golden-gate-bridge:cables", js.wire_drawn_radius_m);
    let mut far = MeshArrays::new("golden-gate-bridge:far");
    let mut skirt = MeshArrays::new("golden-gate-bridge:skirt");

    ctx.towers(&mut orange_m, &mut concrete_m, &mut skirt, &mut far);
    let suspenders = ctx.cables(&mut wires, &mut far);
    ctx.truss(&mut orange_m, &mut far);
    ctx.deck(&mut deck_m, &mut orange_m, ca);
    ctx.approach_span(&mut orange_m, &mut concrete_m, &mut far, plan.c_south, plan.c_start, plan.c_south);
    ctx.approach_span(&mut orange_m, &mut concrete_m, &mut far, plan.c_end, plan.c_north, plan.c_north);
    ctx.tower_walkways(&mut deck_m, &mut orange_m);
    ctx.approaches(&mut orange_m, &mut concrete_m, &mut far);
    ctx.arch_ribs(&mut orange_m, &mut far);
    if ctx.arch2 {
        ctx.arch_bearings(&mut concrete_m);
    }
    if ctx.members {
        ctx.members(&mut orange_m, &mut concrete_m, &mut skirt, &mut wires, &mut far);
    }
    if ctx.pylons {
        ctx.draw_pylons(&mut concrete_m, &mut far);
        ctx.tie_downs(&mut wires, &mut concrete_m, &ctx.cable_runs());
    } else {
        for (_, [x, z, y0, y1, hx, hz, yaw], _) in ctx.pylon_parts() {
            concrete_m.box_(x, z, y0, y1, hx, hz, yaw, ctx.k.concrete, NO_BOTTOM);
            far.box_(x, z, y0, y1, hx, hz, yaw, ctx.k.concrete, ALL);
        }
    }
    // Past the sun's shadow reach the structure's own occlusion is baked in.
    // With `shade2` nothing is baked: the bake darkened the albedo, so the
    // sun on an inward face lost it too, and a face turned down was darkened
    // twice (the environment's irradiance on it already holds no sky). The
    // renderer takes what the screen-space occlusion misses at a bridge's
    // distance, the sky the deck and the opposite leg hide from a face, off
    // the indirect light alone (eo-render landmarks_surface.wgsl `lm_bridge_sky_vis`).
    if !(features.shade2 && features.finish) {
        let inward = frame.inward();
        let (dn, inw) = suspension::occlusion(0.62, 0.74);
        orange_m.shade_occlusion(Some(&inward), dn, inw, 1.5);
        concrete_m.shade_occlusion(Some(&inward), dn, suspension::occlusion(0.62, 0.86).1, 1.5);
        deck_m.shade_occlusion(None, dn, inw, 1.5);
        far.shade_occlusion(Some(&inward), dn, inw, 1.5);
    }

    let r1 = |v: f64| (v * 10.0).round() / 10.0;
    let r2 = |v: f64| (v * 100.0).round() / 100.0;
    let bents = ctx.bent_stations().len();
    let colliders = ctx.colliders();
    Built {
        id: g.id.clone(),
        version: g.version.clone(),
        groups: vec![
            ("orange".into(), orange_m.freeze()),
            ("deck".into(), deck_m.freeze()),
            ("concrete".into(), concrete_m.freeze()),
            ("cables".into(), wires.freeze()),
        ],
        far: far.freeze(),
        skirt: skirt.freeze(),
        colliders,
        stats: json!({
            "mainSpanMeasuredM": r1(site.main_span_measured),
            "mainSpanPublishedM": r1(num(d, "mainSpanM")),
            "structureFromC": plan.c_start, "structureToC": plan.c_end, "roadEndC": plan.c_south, "wayNorthC": plan.c_north,
            "deckTopAtRoadEnd": r2(plan.y_south), "deckTopAtTowers": r2(plan.y_tower), "deckTopMid": r2(plan.y_mid),
            "deckTopAtStructureStart": r2(plan.deck_top(plan.c_start)),
            "suspenderPairsPerCable": suspenders,
            "bents": bents,
        }),
    }
}
