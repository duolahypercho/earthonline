//! The Bay Bridge west span (San Francisco to Yerba Buena Island): a port of
//! src/world/landmarks/bay-bridge-west.js and bay-bridge-west-site.js from the
//! `main` branch. Two suspension bridges end to end through the Center
//! Anchorage, double deck. Positions and dimensions come from the export;
//! the INFERRED literals of the generator's function bodies stay here.
//!
//! PRESENTATION ONLY, as in the JS: it meets the elevated plan's decks where
//! they stop at the San Francisco shore and on the island, at the plan's own
//! heights; colliders are the four tower piers and the Center Anchorage.

use crate::data::{BayBridgeWest, LandmarksJs, hex, num, pair};
use crate::geometry::{ALL, GroundGrid, MeshArrays, NO_BOTTOM, NO_TOP, Rgb, srgb_hex};
use crate::suspension::{self, CableSpec, DeckColours, DeckSpec, DeckStandards, Frame, Run, SuspenderSpec, TrussSpec, cable_height_at};
use crate::{Built, Collider};
use serde_json::json;

const NEAR_FLOOR_ABOVE_SEA_M: f64 = 1.0;
/// INFERRED leg tiers: [fraction of the height from pier top, across, along].
const TIERS: [[f64; 3]; 3] = [[0.40, 6.0, 8.0], [0.75, 5.0, 6.5], [1.00, 4.0, 5.0]];
/// INFERRED: rope spacing within a suspender pair.
const PAIR_GAP_M: f64 = 0.4;
/// `smin` rounding at the deck's grade corners, metres.
pub const CORNER_ROUNDING_M: f64 = 12.0;

/// Dimension keys that are heights or member sizes of the structure: times
/// `golden_gate::structure_scale` (the plan widths are data.rs's; the deck
/// ends keep their clearances, where the decks meet the elevated plan's).
const STRUCTURE_KEYS: &[&str] = &["clearanceM", "trussDepthM", "panelM", "pierTopAboveWaterM", "saddleBelowTopM", "cableLowAboveDeckM", "suspenderSpacingM", "portalAboveDeckM"];
const ANCHORAGE_KEYS: &[&str] = &["plinthM", "corniceM", "corniceProudM", "pilasterM", "pilasterProudM", "housingHalfAlongM", "housingTaperM", "housingWallM", "deckClearanceM", "collarM"];

/// The west span's paint (`BridgeFeatures::paint`): a finish coat of grey
/// aluminium over two coats of red lead (San Francisco Chronicle, "What it was
/// like to be a Bay Bridge painter in 1948": the three coats the crews laid);
/// the span opened as "the silver bridge" (Bay Crossings, the 75th
/// anniversary). INFERRED: the weathered finish at a luminance factor 0.30
/// (steel), the truss a shade under it (0.26: the grime of the decks inside
/// it) and the shadowed members' 0.13; the export's tones were 0.11 / 0.08.
pub const STEEL_SRGB: u32 = 0x8F9598;
pub const TRUSS_SRGB: u32 = 0x878D90;
pub const STEEL_DARK_SRGB: u32 = 0x63686B;

/// The west span's site data with the structure's heights and member sizes
/// times the structure scale (see `golden_gate::data`).
pub fn data(js: &LandmarksJs) -> BayBridgeWest {
    let mut b = js.bay_bridge_west.clone();
    let h = crate::golden_gate::structure_scale();
    if h != 1.0 {
        let scale = |v: &mut serde_json::Value, k: &str| {
            if let Some(x) = v[k].as_f64() {
                v[k] = json!(x * h);
            }
        };
        for k in STRUCTURE_KEYS {
            scale(&mut b.dimensions, k);
        }
        for k in ANCHORAGE_KEYS {
            scale(&mut b.dimensions["anchorage"], k);
        }
        let [along, across] = pair(&b.dimensions, "pierPlanM");
        b.dimensions["pierPlanM"] = json!([along * h, across]);
        b.tower_top_above_water_m *= h;
        b.tower_plan_m[1] *= h; // along (across is the plan's)
        b.center_anchorage.height_m *= h;
        scale(&mut b.center_anchorage.plan_m, "along");
        b.deck_step_m *= h;
    }
    b
}

/// Smooth minimum of two heights (k metres of rounding where they cross).
fn smin(a: f64, b: f64, k: f64) -> f64 {
    if !suspension::features().scale {
        return (a + b - ((a - b).powi(2) + k * k).sqrt()) / 2.0;
    }
    // Round the crest locally so the straight approach still reaches its measured end.
    let overlap = (k - (a - b).abs()).max(0.0);
    a.min(b) - overlap * overlap / (4.0 * k)
}

pub(crate) const UPPER_TRAFFIC_RAIL_M: f64 = 0.9;

pub struct Plan {
    pub c_a: f64,
    pub c_b: f64,
    pub y_a: f64,
    pub y_b: f64,
    pub crest: f64,
    pub grade: f64,
    pub step: f64,
    pub towers: Vec<(String, [f64; 2], f64)>,
    pub anchor_c: f64,
}

impl Plan {
    pub fn deck_top(&self, c: f64) -> f64 {
        let g = self.grade;
        smin(smin(self.crest, self.y_a + g * (c - self.c_a), CORNER_ROUNDING_M), self.y_b + g * (self.c_b - c), CORNER_ROUNDING_M)
    }
    pub fn lower_top(&self, c: f64) -> f64 {
        self.deck_top(c) - self.step
    }
}

/// the span's frame (W2 to W6), for the lights placed on it.
pub fn frame(b: &BayBridgeWest) -> Frame<'static> {
    frame_of(b)
}

/// the main cables' runs (the six spans' catenaries), for the lights placed on them.
pub fn main_cable_runs(js: &LandmarksJs, p: &Plan) -> Vec<Run> {
    cable_runs(js, p).runs
}

fn frame_of(b: &BayBridgeWest) -> Frame<'static> {
    let (o, e) = (b.towers[0].centre, b.towers[3].centre);
    let (dx, dz) = (e[0] - o[0], e[1] - o[1]);
    let len = dx.hypot(dz);
    Frame::new(o, [dx / len, dz / len])
}

/// `bayBridgeWestPlan`.
pub fn plan(js: &LandmarksJs, ground: &dyn Fn(f64, f64) -> f64) -> Plan {
    let b = &data(js);
    let d = &b.dimensions;
    let f = frame_of(b);
    let (sf, isl) = (&b.deck_ends["sf"], &b.deck_ends["island"]);
    let c_a = f.chainage(sf.x, sf.z);
    let c_b = f.chainage(isl.x, isl.z);
    let (y_a, y_b) = if suspension::features().scale {
        (eo_data::bridge_survey::game_y(eo_data::bridge_survey::BAY_SF_NAVD88_M), eo_data::bridge_survey::game_y(eo_data::bridge_survey::BAY_ISLAND_NAVD88_M))
    } else {
        (ground(sf.x, sf.z) + js.default_road_lift_m + sf.clearance, ground(isl.x, isl.z) + js.default_road_lift_m + isl.clearance)
    };
    let crest = js.sea_level_y + num(d, "clearanceM") + num(d, "trussDepthM");
    let towers = b.towers.iter().map(|t| (t.reference.clone(), t.centre, f.chainage(t.centre[0], t.centre[1]))).collect();
    let a = b.center_anchorage.centre;
    Plan { c_a, c_b, y_a, y_b, crest, grade: num(d, "approachGrade"), step: b.deck_step_m, towers, anchor_c: f.chainage(a[0], a[1]) }
}

struct Cables {
    runs: Vec<Run>,
    anchor_top: f64,
    portal_a: f64,
    portal_b: f64,
}

fn cable_runs(js: &LandmarksJs, p: &Plan) -> Cables {
    let b = &data(js);
    let d = &b.dimensions;
    let (w2, w3, w5, w6) = (p.towers[0].2, p.towers[1].2, p.towers[2].2, p.towers[3].2);
    let saddle = js.sea_level_y + b.tower_top_above_water_m - num(d, "saddleBelowTopM");
    let low_a = p.deck_top((w2 + w3) / 2.0) + num(d, "cableLowAboveDeckM");
    let low_b = p.deck_top((w5 + w6) / 2.0) + num(d, "cableLowAboveDeckM");
    let (sag_a, sag_b) = (saddle - low_a, saddle - low_b);
    let (span_a, span_b) = (w3 - w2, w6 - w5);
    let portal = num(d, "portalAboveDeckM");
    let anchor_top = (js.sea_level_y + b.center_anchorage.height_m).max(p.deck_top(p.anchor_c)) + portal;
    let portal_a = p.y_a + portal;
    let portal_b = p.y_b + portal;
    let ss = suspension::side_span_sag;
    Cables {
        runs: vec![
            Run { c0: p.c_a, c1: w2, y0: portal_a, y1: saddle, sag: ss(sag_a, span_a, w2 - p.c_a) },
            Run { c0: w2, c1: w3, y0: saddle, y1: saddle, sag: sag_a },
            Run { c0: w3, c1: p.anchor_c, y0: saddle, y1: anchor_top, sag: ss(sag_a, span_a, p.anchor_c - w3) },
            Run { c0: p.anchor_c, c1: w5, y0: anchor_top, y1: saddle, sag: ss(sag_b, span_b, w5 - p.anchor_c) },
            Run { c0: w5, c1: w6, y0: saddle, y1: saddle, sag: sag_b },
            Run { c0: w6, c1: p.c_b, y0: saddle, y1: portal_b, sag: ss(sag_b, span_b, p.c_b - w6) },
        ],
        anchor_top,
        portal_a,
        portal_b,
    }
}

struct Colours {
    steel: Rgb,
    truss: Rgb,
    steel_dark: Rgb,
    asphalt: Rgb,
    concrete: Rgb,
    concrete_light: Rgb,
    white: Rgb,
    yellow: Rgb,
}

/// A plan rectangle in the bridge frame as [x, z] corners (`frameRect`).
fn frame_rect(f: &Frame, c: f64, s: f64, half_along: f64, half_across: f64) -> Vec<[f64; 2]> {
    [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]].iter().map(|[a, b]| f.point(c + a * half_along, s + b * half_across)).collect()
}

/// `first_top_by_deck` (landmark access, access.rs): the first tower's (W2's)
/// top is the deck's, which draws its saddle housings; the `members` pass leaves its own out there.
#[allow(clippy::too_many_arguments)]
fn towers(js: &LandmarksJs, p: &Plan, f: &Frame, k: &Colours, h: f64, ground: &dyn Fn(f64, f64) -> f64, steel: &mut MeshArrays, concrete: &mut MeshArrays, skirt: &mut MeshArrays, far: &mut MeshArrays, first_top_by_deck: bool) {
    let b = &data(js);
    let m = crate::golden_gate::structure_scale();
    let detail = crate::suspension::features().detail;
    let d = &b.dimensions;
    let w = js.sea_level_y;
    let top = w + b.tower_top_above_water_m;
    let pier_top = w + num(d, "pierTopAboveWaterM");
    let floor = w + NEAR_FLOOR_ABOVE_SEA_M;
    let truss_depth = num(d, "trussDepthM");
    for (ti, (_, [cx, cz], tc)) in p.towers.iter().enumerate() {
        let (cx, cz, tc) = (*cx, *cz, *tc);
        let bed = ground(cx, cz).min(w - 2.0);
        let [along, across] = pair(d, "pierPlanM");
        concrete.box_(cx, cz, floor, pier_top, along / 2.0, across / 2.0, f.yaw, k.concrete, NO_BOTTOM);
        skirt.box_(cx, cz, bed, floor, along / 2.0, across / 2.0, f.yaw, k.concrete, NO_TOP);
        far.box_(cx, cz, bed, pier_top, along / 2.0, across / 2.0, f.yaw, k.concrete, ALL);
        for s in [-h, h] {
            let mut y0 = pier_top;
            for [to, wd, dp] in TIERS {
                let (wd, dp) = (wd * m, dp * m);
                let y1 = pier_top + (top - pier_top) * to;
                let [x, z] = f.point(tc, s);
                let faces = crate::geometry::Faces { top: true, bottom: y0 == pier_top };
                if detail {
                    crate::suspension::notched_box(steel, x, z, y0, y1, dp / 2.0, wd / 2.0, f.yaw, k.steel, faces);
                } else {
                    steel.box_(x, z, y0, y1, dp / 2.0, wd / 2.0, f.yaw, k.steel, faces);
                }
                far.box_(x, z, y0, y1, dp / 2.0, wd / 2.0, f.yaw, k.steel, ALL);
                y0 = y1;
            }
        }
        let dy = p.deck_top(tc);
        let under = dy - truss_depth - 2.0;
        // `BridgeFeatures::members`: the diagonals as laced built-up members (two chords and their lacing in the tower's plane).
        let members = crate::suspension::features().members;
        let brace = |steel: &mut MeshArrays, a: crate::geometry::V3, b: crate::geometry::V3, wd: f64, dp: f64, colour: Rgb| {
            if members {
                let ax = [f.axis[0], 0.0, f.axis[1]];
                let dir = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                let l = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt().max(1e-9);
                let dn = [dir[0] / l, dir[1] / l, dir[2] / l];
                let ip = [ax[1] * dn[2] - ax[2] * dn[1], ax[2] * dn[0] - ax[0] * dn[2], ax[0] * dn[1] - ax[1] * dn[0]];
                let il = (ip[0] * ip[0] + ip[1] * ip[1] + ip[2] * ip[2]).sqrt().max(1e-9);
                let ip = [ip[0] / il, ip[1] / il, ip[2] / il];
                crate::suspension::laced(steel, a, b, ip, dp * LACED_DEPTH_K, LACED_CHORD_M * m, LACED_BAR_M * m, LACED_PITCH_M * m, colour);
                // the chords' flanges through the tower (its width along the span).
                let _ = wd;
            } else {
                steel.beam(a, b, wd, dp, colour);
            }
        };
        brace(steel, f.P(tc, -h, pier_top + 4.0 * m), f.P(tc, h, under - 3.0 * m), 1.8 * m, 2.2 * m, k.steel_dark);
        brace(steel, f.P(tc, h, pier_top + 4.0 * m), f.P(tc, -h, under - 3.0 * m), 1.8 * m, 2.2 * m, k.steel_dark);
        steel.beam(f.P(tc, -h, under), f.P(tc, h, under), 4.0 * m, 4.0 * m, k.steel_dark);
        let (x_base, x_top) = (dy + 6.0 * m, top - 7.0 * m);
        for pn in 0..3 {
            let ya = x_base + (x_top - x_base) * (pn as f64 / 3.0);
            let yb = x_base + (x_top - x_base) * ((pn + 1) as f64 / 3.0);
            brace(steel, f.P(tc, -h, ya), f.P(tc, h, yb), 1.6 * m, 2.0 * m, k.steel);
            brace(steel, f.P(tc, h, ya), f.P(tc, -h, yb), 1.6 * m, 2.0 * m, k.steel);
            steel.beam(f.P(tc, -h, ya), f.P(tc, h, ya), 2.6 * m, 2.4 * m, k.steel);
            far.beam(f.P(tc, -h, ya), f.P(tc, h, yb), 2.0 * m, 2.0 * m, k.steel);
            far.beam(f.P(tc, h, ya), f.P(tc, -h, yb), 2.0 * m, 2.0 * m, k.steel);
        }
        steel.beam(f.P(tc, -h, top - 3.5 * m), f.P(tc, h, top - 3.5 * m), 4.0 * m, 7.0 * m, k.steel);
        far.beam(f.P(tc, -h, top - 3.5 * m), f.P(tc, h, top - 3.5 * m), 4.0 * m, 7.0 * m, k.steel);
        if members && !(ti == 0 && first_top_by_deck) {
            // the cable saddles' housings over the legs' tops (the saddle `saddleBelowTopM` under the top).
            let [_, wd, dp] = TIERS[TIERS.len() - 1];
            for s in [-h, h] {
                crate::suspension::saddle_housing(steel, f, tc, s, top, dp * m / 2.0 * 0.95, wd * m / 2.0 * 0.9, 0.6 * m, k.steel);
            }
        }
    }
}

/// INFERRED (`BridgeFeatures::members`; no section is published): a laced
/// diagonal's depth over the solid plank's, its chords' and lacing bars'
/// sections, the lacing's pitch; the decks' slab (with its haunch) under
/// the roadway, the stringers' count and section, the floor beams' depth
/// and width at each panel point.
const LACED_DEPTH_K: f64 = 1.5;
const LACED_CHORD_M: f64 = 0.55;
const LACED_BAR_M: f64 = 0.22;
const LACED_PITCH_M: f64 = 1.8;
const DECK_SLAB_M: f64 = 0.45;
const STRINGERS: usize = 6;
const STRINGER_M: [f64; 2] = [0.35, 0.9];
const FLOOR_BEAM_M: [f64; 2] = [0.5, 2.0];

/// The decks' structure (`BridgeFeatures::members`): each deck's floor
/// system closed at its edges (the edge stringer's web, `DECK_SLAB_M` plus a
/// floor beam's depth), its stringers under the slab and a floor beam
/// across the truss at each panel point (the lower deck's sky showed
/// through the truss where only its plate was drawn), and the lamp
/// standards on the upper deck at the luminaires lights.rs places there.
fn deck_members(p: &Plan, f: &Frame, k: &Colours, h: f64, panel: f64, roadway: f64, steel: &mut MeshArrays) {
    let half = h - 0.6;
    let cs = crate::suspension::stations(p.c_a, p.c_b, panel);
    for upper in [true, false] {
        let y = |c: f64| if upper { p.deck_top(c) } else { p.lower_top(c) };
        for i in 0..cs.len() - 1 {
            let (a, b) = (cs[i], cs[i + 1]);
            let (ya, yb) = (y(a), y(b));
            for sgn in [-1.0, 1.0] {
                // the floor system's outer face: the edge stringer's web from the slab's top down
                // to the floor beams' bottom flange, in the members' paint (the slab's edge sits
                // behind its top flange). A concrete fascia the slab's depth alone drew as a
                // light stepped strip with the sky under it.
                let e = sgn * half;
                let (p0, p1) = if sgn > 0.0 { (b, a) } else { (a, b) };
                let (y0, y1) = if sgn > 0.0 { (yb, ya) } else { (ya, yb) };
                let dd = DECK_SLAB_M + FLOOR_BEAM_M[1];
                steel.quad(f.P(p0, e, y0 - dd), f.P(p1, e, y1 - dd), f.P(p1, e, y1), f.P(p0, e, y0), k.steel);
            }
            for j in 0..STRINGERS {
                let s = -half + (j as f64 + 0.5) / STRINGERS as f64 * 2.0 * half;
                let d = DECK_SLAB_M + STRINGER_M[1] / 2.0;
                steel.beam(f.P(a, s, ya - d), f.P(b, s, yb - d), STRINGER_M[0], STRINGER_M[1], k.steel);
            }
        }
        for &c in &cs {
            let d = DECK_SLAB_M + FLOOR_BEAM_M[1] / 2.0;
            steel.beam(f.P(c, -h, y(c) - d), f.P(c, h, y(c) - d), FLOOR_BEAM_M[0], FLOOR_BEAM_M[1], k.steel);
        }
    }
    let bhalf = roadway / 2.0;
    let head = crate::lights::DECK_HEAD_M;
    let mut c = p.c_a + crate::lights::DECK_PITCH_M / 2.0;
    while c < p.c_b {
        for sgn in [-1.0, 1.0] {
            crate::golden_gate::lamp_standard(steel, f, c, sgn * (bhalf + 0.3), -sgn, p.deck_top(c) + 0.2, head, k.steel);
        }
        c += crate::lights::DECK_PITCH_M;
    }
}

#[allow(clippy::too_many_arguments)]
fn center_anchorage(js: &LandmarksJs, p: &Plan, f: &Frame, k: &Colours, h: f64, cables: &Cables, ground: &dyn Fn(f64, f64) -> f64, concrete: &mut MeshArrays, steel: &mut MeshArrays, skirt: &mut MeshArrays, far: &mut MeshArrays) {
    let b = &data(js);
    let d = &b.dimensions;
    let a = &b.center_anchorage;
    let an = &d["anchorage"];
    let w = js.sea_level_y;
    let [cx, cz] = a.centre;
    let c = p.anchor_c;
    let half_along = num(&a.plan_m, "along") / 2.0;
    let half_across = num(&a.plan_m, "across") / 2.0;
    let bed = ground(cx, cz).min(w - 2.0);
    let floor = w + NEAR_FLOOR_ABOVE_SEA_M;
    let truss = p.deck_top(c) - num(d, "trussDepthM");
    let block_top = (w + a.height_m).min(truss - 0.5);
    let plinth_top = w + num(an, "plinthM");
    let inset = (block_top - plinth_top) * num(an, "batter");
    let (cornice, cornice_proud) = (num(an, "corniceM"), num(an, "corniceProudM"));
    concrete.prism(&frame_rect(f, c, 0.0, half_along, half_across), floor, plinth_top, k.concrete, true, false);
    let body0 = frame_rect(f, c, 0.0, half_along - 1.2, half_across - 1.2);
    let body1 = frame_rect(f, c, 0.0, half_along - 1.2 - inset, half_across - 1.2 - inset);
    concrete.loft(&body0, plinth_top, &body1, block_top - cornice, k.concrete, false);
    concrete.prism(&frame_rect(f, c, 0.0, half_along - 1.2 - inset + cornice_proud, half_across - 1.2 - inset + cornice_proud), block_top - cornice, block_top, k.concrete_light, true, false);
    let (pw, pd) = (num(an, "pilasterM") / 2.0, num(an, "pilasterProudM"));
    let pilaster = |concrete: &mut MeshArrays, along: f64, across: f64, normal_along: bool| {
        let at = |t: f64| {
            let ia = half_along - 1.2 - inset * t;
            let ic = half_across - 1.2 - inset * t;
            if normal_along {
                frame_rect(f, c + along.signum() * (ia + pd / 2.0), across, pd / 2.0, pw)
            } else {
                frame_rect(f, c + along, across.signum() * (ic + pd / 2.0), pw, pd / 2.0)
            }
        };
        concrete.loft(&at(0.0), plinth_top, &at(1.0), block_top - cornice, k.concrete_light, false);
    };
    let (n_along, n_across) = (num(an, "pilastersAlong") as usize, num(an, "pilastersAcross") as usize);
    for side in [-1.0, 1.0] {
        for kk in 0..n_along {
            pilaster(concrete, -half_along + 1.2 + ((kk as f64 + 0.5) / n_along as f64) * 2.0 * (half_along - 1.2), side, false);
        }
        for kk in 0..n_across {
            pilaster(concrete, side, -half_across + 1.2 + ((kk as f64 + 0.5) / n_across as f64) * 2.0 * (half_across - 1.2), true);
        }
    }
    let deck_half = h - 0.6 + 0.15;
    let wall_outer = deck_half + num(an, "housingWallM");
    let top = cables.anchor_top + 3.0;
    let lintel = p.deck_top(c) + num(an, "deckClearanceM");
    let (hha, taper, collar) = (num(an, "housingHalfAlongM"), num(an, "housingTaperM"), num(an, "collarM"));
    for sgn in [-1.0, 1.0] {
        let mid = sgn * (deck_half + wall_outer) / 2.0;
        let half = (wall_outer - deck_half) / 2.0;
        concrete.loft(&frame_rect(f, c, mid, hha, half), block_top, &frame_rect(f, c, mid, hha - taper, half), top, k.concrete, true);
        let [mx, mz] = f.point(c, mid);
        far.box_(mx, mz, block_top, top, hha, half, f.yaw, k.concrete, ALL);
        for end in [-1.0, 1.0] {
            let ce = c + end * (hha - taper * 0.5);
            let y = cable_height_at(&cables.runs, ce).unwrap_or(cables.anchor_top);
            let [x, z] = f.point(ce, sgn * h);
            steel.box_(x, z, y - collar / 2.0, y + collar / 2.0, 0.8, collar / 2.0, f.yaw, k.steel_dark, ALL);
        }
    }
    concrete.loft(&frame_rect(f, c, 0.0, hha - taper * 0.5, deck_half), lintel, &frame_rect(f, c, 0.0, hha - taper, deck_half), top, k.concrete, true);
    skirt.box_(cx, cz, bed, floor, half_along, half_across, f.yaw, k.concrete, NO_TOP);
    far.box_(cx, cz, bed, plinth_top, half_along, half_across, f.yaw, k.concrete, ALL);
    far.loft(&body0, plinth_top, &body1, block_top, k.concrete, true);
}

fn end_portals(js: &LandmarksJs, p: &Plan, f: &Frame, k: &Colours, h: f64, cables: &Cables, steel: &mut MeshArrays) {
    let truss = num(&data(js).dimensions, "trussDepthM");
    let m = crate::golden_gate::structure_scale();
    for (c, y) in [(p.c_a, cables.portal_a), (p.c_b, cables.portal_b)] {
        for s in [-h, h] {
            steel.beam(f.P(c, s, p.deck_top(c) - truss), f.P(c, s, y + 1.0 * m), 2.2 * m, 2.2 * m, k.steel);
        }
        steel.beam(f.P(c, -h, y), f.P(c, h, y), 2.4 * m, 2.0 * m, k.steel);
    }
}

/// `buildBayBridgeWest`.
pub fn build(js: &LandmarksJs, ground: &GroundGrid, ca: &DeckStandards) -> Built {
    build_opened(js, ground, ca, false)
}

/// `build`; `open` (landmark access, access.rs): the W2 tower's top is the
/// deck's (its saddle housings with it).
pub fn build_opened(js: &LandmarksJs, ground: &GroundGrid, ca: &DeckStandards, open: bool) -> Built {
    let features = crate::suspension::features();
    let bd = data(js);
    let b = &bd;
    let m = crate::golden_gate::structure_scale();
    let d = &b.dimensions;
    let ground_at = |x: f64, z: f64| ground.at(x, z);
    let p = plan(js, &ground_at);
    let f = frame_of(b);
    let h = num(d, "halfPlaneM");
    crate::suspension::record_deck_line("bay-bridge-west", [p.c_a, p.c_b, p.y_a, p.y_b, p.crest, p.grade, p.step, h - 0.6]);
    let paint = |cited: u32, key: &str| srgb_hex(if features.paint { cited } else { hex(d, key) });
    let k = Colours {
        steel: paint(STEEL_SRGB, "steelSrgb"),
        // `finish`: the truss in the towers' coat (the same paint; the frame's shadows darken its interior).
        truss: paint(if features.finish { STEEL_SRGB } else { TRUSS_SRGB }, "trussSrgb"),
        steel_dark: paint(STEEL_DARK_SRGB, "steelDarkSrgb"),
        asphalt: srgb_hex(hex(d, "asphaltSrgb")),
        concrete: srgb_hex(hex(d, "concreteSrgb")),
        concrete_light: srgb_hex(hex(d, "concreteLightSrgb")),
        white: srgb_hex(hex(d, "whiteSrgb")),
        yellow: srgb_hex(hex(d, "yellowSrgb")),
    };
    let mut steel = MeshArrays::new("bay-bridge-west:steel");
    let mut deck = MeshArrays::new("bay-bridge-west:deck");
    let mut concrete = MeshArrays::new("bay-bridge-west:concrete");
    let mut wires = MeshArrays::wire("bay-bridge-west:cables", js.wire_drawn_radius_m);
    let mut far = MeshArrays::new("bay-bridge-west:far");
    let mut skirt = MeshArrays::new("bay-bridge-west:skirt");
    // `zones`: only the towers' legs interrupt the truss planes and walkways.
    let zone_half = b.tower_plan_m[1].max(8.0) / 2.0 + 1.5;
    let centres: Vec<f64> = p.towers.iter().map(|t| t.2).collect();
    let gap = |c0: f64, c1: f64| centres.iter().any(|&t| c1 > t - zone_half && c0 < t + zone_half);
    let cables = cable_runs(js, &p);

    towers(js, &p, &f, &k, h, &ground_at, &mut steel, &mut concrete, &mut skirt, &mut far, open);
    center_anchorage(js, &p, &f, &k, h, &cables, &ground_at, &mut concrete, &mut steel, &mut skirt, &mut far);
    suspension::main_cables(&mut wires, &mut far, &f, &cables.runs, &CableSpec { half_plane: h, diameter: num(d, "cableDiameterM"), colour: k.steel, far_beams: !features.wires });
    let suspenders = suspension::suspender_pairs(
        &mut wires,
        &f,
        &cables.runs,
        &|c| p.deck_top(c) - 0.5,
        &SuspenderSpec { half_plane: h, spacing: num(d, "suspenderSpacingM"), diameter: num(d, "suspenderDiameterM"), pair_gap: PAIR_GAP_M, colour: k.steel },
    );
    suspension::stiffening_truss(
        &mut steel,
        &mut far,
        &f,
        &TrussSpec {
            deck_top: &|c| p.deck_top(c),
            c_start: p.c_a,
            c_end: p.c_b,
            half_plane: h,
            depth: num(d, "trussDepthM"),
            panel: num(d, "panelM"),
            colour: k.truss,
            dark: k.steel_dark,
            gap: &gap,
            far_half_width: h + 0.5,
            end_plate_cover: 0.,
            floorbeam_span: None,
            far_above: 1.0,
            member: m,
        },
    );
    let colours = DeckColours { asphalt: k.asphalt, sidewalk: k.asphalt, soffit: k.steel_dark, barrier: k.concrete, white: k.white, yellow: k.yellow, rail: k.steel };
    let lanes = num(d, "lanesPerDeck") as u32;
    // Upper deck: westbound, five lanes. Lower deck: eastbound, five lanes, inside the truss.
    let upper_y = |c: f64| p.deck_top(c);
    let lower_y = |c: f64| p.lower_top(c);
    let decks: [(bool, &dyn Fn(f64) -> f64); 2] = [(true, &upper_y), (false, &lower_y)];
    for (upper, y) in decks {
        suspension::bridge_road_deck(
            &mut deck,
            &mut steel,
            &mut far,
            &f,
            ca,
            &DeckSpec {
                y,
                c_start: p.c_a,
                c_end: p.c_b,
                half_width: h - 0.6,
                roadway_m: num(d, "roadwayM"),
                lanes,
                median: false,
                sidewalk_m: 0.0,
                curb_m: 0.2,
                railings: upper,
                railing_m: 1.2,
                inner_barrier_m: if upper { UPPER_TRAFFIC_RAIL_M } else { 1.0 },
                panel: num(d, "panelM"),
                gap: &|_, _| false,
                colours,
                markings: true,
                outer_railing_elsewhere: None,
                outer_parapet: None,
            },
        );
    }
    end_portals(js, &p, &f, &k, h, &cables, &mut steel);
    if features.members {
        deck_members(&p, &f, &k, h, num(d, "panelM"), num(d, "roadwayM"), &mut steel);
        // the cable bands and the ropes' sockets (as the Golden Gate's: golden_gate.rs).
        // INFERRED: the bands 1.1 m long and 7 cm thick round the 0.73 m cable, the sockets 0.8 m by 0.18 m.
        crate::suspension::cable_bands(
            &mut wires,
            &f,
            &cables.runs,
            &|c| p.deck_top(c) - 0.5,
            &crate::suspension::BandSpec {
                half_plane: h,
                spacing: num(d, "suspenderSpacingM"),
                pair_gap: PAIR_GAP_M,
                cable_d: num(d, "cableDiameterM"),
                band_len: 1.1,
                band_d: num(d, "cableDiameterM") + 0.14,
                socket_len: 0.8,
                socket_d: 0.18,
                colour: k.steel,
            },
        );
    }
    let inward = f.inward();
    let (down, inw) = crate::suspension::occlusion(num(d, "steelShadeDown"), num(d, "steelShadeInward"));
    steel.shade_occlusion(Some(&inward), down, inw, 1.5);
    concrete.shade_occlusion(Some(&inward), crate::suspension::occlusion(0.62, 0.86).0, crate::suspension::occlusion(0.62, 0.86).1, 1.5);
    let (dd, di) = crate::suspension::occlusion(0.62, 0.74);
    deck.shade_occlusion(None, dd, di, 1.5);
    far.shade_occlusion(Some(&inward), down, inw, 1.5);

    let mut colliders = Vec::new();
    let [along, across] = pair(d, "pierPlanM");
    for (r, [cx, cz], _) in &p.towers {
        colliders.push(Collider { part: format!("{r}-pier"), x: *cx, z: *cz, y0: ground_at(*cx, *cz).min(js.sea_level_y - 2.0), y1: js.sea_level_y + num(d, "pierTopAboveWaterM"), hx: along / 2.0, hz: across / 2.0, yaw: f.yaw });
    }
    let a = &b.center_anchorage;
    let truss = p.deck_top(p.anchor_c) - num(d, "trussDepthM");
    colliders.push(Collider {
        part: "W4-anchorage".into(),
        x: a.centre[0],
        z: a.centre[1],
        y0: ground_at(a.centre[0], a.centre[1]).min(js.sea_level_y - 2.0),
        y1: (js.sea_level_y + a.height_m).min(truss - 0.5),
        hx: num(&a.plan_m, "along") / 2.0,
        hz: num(&a.plan_m, "across") / 2.0,
        yaw: f.yaw,
    });
    let r1 = |v: f64| (v * 10.0).round() / 10.0;
    let r2 = |v: f64| (v * 100.0).round() / 100.0;
    Built {
        id: b.id.clone(),
        version: b.version.clone(),
        groups: vec![("steel".into(), steel.freeze()), ("deck".into(), deck.freeze()), ("concrete".into(), concrete.freeze()), ("cables".into(), wires.freeze())],
        far: far.freeze(),
        skirt: skirt.freeze(),
        colliders,
        stats: json!({
            "deckEnds": { "sf": [r1(p.c_a), r2(p.y_a)], "island": [r1(p.c_b), r2(p.y_b)] },
            "crest": r2(p.crest),
            "deckTopAtTowers": p.towers.iter().map(|t| r1(p.deck_top(t.2))).collect::<Vec<_>>(),
            "mainSpansMeasured": [r1(p.towers[1].2 - p.towers[0].2), r1(p.towers[3].2 - p.towers[2].2)],
            "suspenderStationsPerPlane": suspenders,
        }),
    }
}
