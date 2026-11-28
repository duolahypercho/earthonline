//! the waterfront as built places. The quay wall's edges, the
//! piers as decks over the water, the promenade at the top of the wall, and
//! the press the terrain takes behind them.
//!
//! What the JS has, and where this goes past it:
//!
//! - **The shore edges.** The JS walls every shoreline ring edge inside its
//!   window that is not near a beach (renderer.js `skipBeachEdge`); the port
//!   (shore.rs `city_walls`) also dropped a ring edge whose ground 4 m out on
//!   its seaward side reads dry (the county line). That probe reads the
//!   elevation grid's LiDAR, which the sampler takes whenever one of a
//!   sample's four cells is LiDAR ground (eo-data `ElevationGrid::sample`):
//!   4 m out from a quay is still the quay's own cells, so most of the
//!   Embarcadero lost its wall. Here an edge is a county line only when every
//!   probe out to `COUNTY_LINE_PROBES_M` reads dry.
//! - **Piers.** The JS builds one pier deck, the Ferry Plaza's (OSM way
//!   661723975, hero-waterfront-structures.js, "no piles"), and none of the
//!   other piers, whose land the shoreline rings carry (the DataSF shoreline
//!   draws round them: 90-100 % of each big pier's area is ring land). The
//!   pier outlines are the game's own data: `piers` of
//!   public/data/sf/sf-map-citywide.json.gz (OSM `man_made=pier|breakwater`,
//!   ODbL, the earlier map build), which only the 2D map reads.
//!   Each becomes a flat deck at its surveyed level (the median LiDAR ground
//!   inside it plus the road lift), with a fascia, a dark under-deck face and
//!   a pile row along every edge where it meets the water: on the shoreline
//!   ring's own edges inside the pier (so the deck edge stands exactly on the
//!   waterline the water pass draws to) and on its OSM edges that stand over
//!   open water. INFERRED (no JS or data source): the fascia depth, the pile
//!   pitch and section, the under-deck face, the deck strip's width.
//! - **The promenade.** The JS presses its carpet below the sea within 12 m
//!   of the water and lays its streets and paths over it
//!   (ground-coverage-contract.js); the native terrain stands on the surveyed
//!   ground, whose 8 m cells slope from the quay down to the seabed across the
//!   waterline, in front of the wall's face. Here the terrain behind a walled
//!   edge is pressed (`press_quay`) so that slope is under the water, and a
//!   paved strip at the wall's crest covers the press (`CAP_W_M`, the quay
//!   apron; INFERRED width). The wall's landward face, band and block courses
//!   are under that strip, so the near tier does not build them (shore.rs
//!   `build_seawall_seaward`).

use crate::geometry::{Frozen, MeshArrays, Rgb, ring_centroid, signed_area};
use crate::shore::BeachRule;
use anyhow::Result;
use std::collections::HashMap;

/// A ring edge is a county line (dry land on its "sea" side) only when the
/// ground reads more than `SHORE_DRY_ABOVE_SEA_M` over the sea at every one of
/// these distances out from it.
pub const COUNTY_LINE_PROBES_M: [f64; 5] = [4.0, 12.0, 24.0, 40.0, 64.0];
/// The longest piece of a ring edge the wall is built on, metres.
pub const EDGE_PIECE_M: f64 = 8.0;
/// The wall's crest stands on the highest ground within this of it (the
/// quay's own top, not the bank the elevation grid blends down to the sea
/// along the ring line), metres.
pub const CREST_REACH_M: f64 = 8.0;

/// The quay crest's ground at (x, z): the highest of the ground there and on
/// a ring of `CREST_REACH_M` and half of it around it.
pub fn crest_ground(height_at: &dyn Fn(f64, f64) -> Option<f64>, x: f64, z: f64) -> f64 {
    let mut best = height_at(x, z).unwrap_or(0.0);
    for r in [CREST_REACH_M * 0.5, CREST_REACH_M] {
        for k in 0..8 {
            let a = k as f64 * std::f64::consts::FRAC_PI_4;
            if let Some(h) = height_at(x + a.cos() * r, z + a.sin() * r) {
                best = best.max(h);
            }
        }
    }
    best
}

/// Pier outlines smaller than this are left out (moorings, floats). m^2.
pub const PIER_MIN_AREA_M2: f64 = 150.0;
/// A pier further than this from every ring edge is not in the bay the city
/// draws (Marin, the East Bay). Metres.
pub const PIER_MAX_SHORE_M: f64 = 400.0;
/// A ring edge whose middle is within this of a pier outline is that pier's edge. Metres.
pub const PIER_DILATE_M: f64 = 6.0;
/// The paved strip at the wall's crest (the quay apron) and a pier's deck strip, landward of the waterline. Metres.
pub const CAP_W_M: f64 = 8.0;
/// The coping's landward width: where the apron starts behind the wall's face. Metres.
pub const COPING_LAND_M: f64 = 0.42;
/// (`--quay-fill`, `LandmarkSet::add_quay_fill`) how far past the apron the
/// fill looks for the street's walk, the spacing of its stations along a wall
/// edge, and how much lower than the paving either side the ground between
/// must stand to be filled (the sunken strip; ground at the paving's level, or
/// over it, is left as it is). Metres. INFERRED (the Embarcadero promenade
/// runs from the seawall to the roadway's walk, Herb Caen Way, with no strip
/// of open ground between).
pub const FILL_REACH_M: f64 = 24.0;
pub const FILL_STATION_M: f64 = 2.0;
pub const FILL_SUNKEN_M: f64 = 0.15;
/// The highest step between the apron and the walk the fill closes with a
/// terrace and a step face (a seat wall's height and a little); a greater
/// difference is left as it is. Metres. INFERRED.
pub const FILL_STEP_MAX_M: f64 = 1.0;
/// The apron kept clear at the coping where a fill's ramp starts out on it. Metres. INFERRED.
pub const FILL_EDGE_CLEAR_M: f64 = 1.5;
/// The press behind a wall: at most `sea + QUAY_PRESS_FOOT_M + QUAY_PRESS_SLOPE x d`, d metres inland
/// of the waterline (the slope keeps the terrain's 8 m cells under the sea at the wall's foot).
pub const QUAY_PRESS_FOOT_M: f64 = -0.6;
pub const QUAY_PRESS_SLOPE: f64 = 0.45;
/// A pier deck's fascia (edge beam) depth, the set-back of the dark under-deck
/// face, and the pile row's pitch and section. Metres. INFERRED.
pub const PIER_FASCIA_M: f64 = 0.9;
pub const PIER_VOID_SETBACK_M: f64 = 1.0;
pub const PIER_PILE_PITCH_M: f64 = 4.2;
pub const PIER_PILE_SECTION_M: f64 = 0.46;
/// The deck stands this far under the road surface (ground + road lift), so
/// a street on a pier draws over it. Metres.
pub const PIER_UNDER_ROAD_M: f64 = 0.03;
/// A pier with no LiDAR ground inside (a float, a pier built over open water)
/// stands this far over the sea. Metres. INFERRED.
pub const PIER_FLOAT_FREEBOARD_M: f64 = 1.0;

/// The under-deck face's linear tone: the shadowed void under a pier deck,
/// over the water (INFERRED: a dark weathered concrete in deep shade).
pub const UNDER_DECK_LINEAR: Rgb = [0.018, 0.020, 0.019];
/// The kit finishes the Ferry Plaza pier's sheds are drawn in (kit-material-specs.json):
/// a painted steel panel (dielectric; the kit's bare corrugated metal, metalness 1,
/// mirrors the sky and drew the sheds blue at noon) and a membrane roof.
pub const SHED_WALL_KIT: &str = "painted-steel-light";
pub const SHED_ROOF_KIT: &str = "membrane-roof";

/// `createFerryCard04PierSheds` on the pier's deck at `deck_y`: the JS's
/// anchors and box sizes (shore.rs `ferry_shed_anchors`), as walls and roofs
/// (white vertex colour: the kit finish carries the tone).
pub fn ferry_sheds(outline: &[[f64; 2]], spec: &serde_json::Value, deck_y: f64) -> Option<(Frozen, Frozen)> {
    let n = |k: &str| crate::data::num(spec, k);
    let (w, d, h) = (n("widthM"), n("depthM"), n("heightM"));
    let picked = crate::shore::ferry_shed_anchors(outline, spec);
    if picked.is_empty() {
        return None;
    }
    let mut walls = MeshArrays::new("ferry-pier-sheds");
    let mut roofs = MeshArrays::new("ferry-pier-shed-roofs");
    for (p, yaw) in &picked {
        // three's rotation.y turns local +x toward -z: the box's local x is its width.
        walls.box_(p[0], p[1], deck_y - 0.05, deck_y + h, w / 2.0, d / 2.0, -yaw, [1.0; 3], crate::geometry::Faces { top: false, bottom: false });
        let mut top = MeshArrays::new("roof");
        top.box_(p[0], p[1], deck_y + h - 0.01, deck_y + h, w / 2.0, d / 2.0, -yaw, [1.0; 3], crate::geometry::NO_BOTTOM);
        // Its last quad is the top face (hexahedron: four sides, then the top).
        let v0 = top.positions.len() - 12;
        let q: Vec<[f64; 3]> = (0..4).map(|k| [top.positions[v0 + 3 * k], top.positions[v0 + 3 * k + 1], top.positions[v0 + 3 * k + 2]]).collect();
        roofs.quad_n(q[0], q[1], q[2], q[3], [1.0; 3], [0.0, 1.0, 0.0]);
    }
    Some((walls.freeze(), roofs.freeze()))
}

/// The Ferry Plaza's gate sheds drawn as built pieces (`features::pier_sheds`;
/// off: the JS's boxes, `ferry_sheds`): at the JS's anchors and plan sizes, an
/// open-air maritime steel-and-glass ferry terminal boarding canopy, INFERRED
/// (no survey of the gates): slender charcoal-slate steel columns on weathered
/// sandstone plinths with 45-degree structural knee-braces, recessed smoked
/// passenger windscreens below an open sunlit clerestory gap and louvered
/// transit frieze, cantilevered steel outrigger rafters supporting a low-pitched
/// dark standing-seam zinc-slate canopy roof, and a raised central glazed
/// skylight monitor echoing the Ferry Building's nave.
pub const GATE_BAY_M: f64 = 4.0;
pub const GATE_EAVE_M: f64 = 4.15;
pub const GATE_GLAZING_TOP_M: f64 = 2.50;
pub const GATE_RIDGE_RISE_M: f64 = 1.05;
pub const GATE_OVERHANG_M: f64 = 2.05;
pub const GATE_COLUMN_M: f64 = 0.22;
/// Recessed smoked architectural windscreen glass (warm-neutral dark charcoal, linear; avoids sky-blue flash).
pub const GATE_GLASS_LINEAR: Rgb = [0.016, 0.017, 0.018];

/// The gate sheds' meshes: frames and panels (in `SHED_WALL_KIT`), roofs (`SHED_ROOF_KIT`), glazing (vertex colour).
pub struct GateSheds {
    pub frames: Frozen,
    pub roofs: Frozen,
    pub glass: Frozen,
}

pub fn ferry_gate_sheds(outline: &[[f64; 2]], spec: &serde_json::Value, deck_y: f64) -> Option<GateSheds> {
    let n = |k: &str| crate::data::num(spec, k);
    let (w, d) = (n("widthM"), n("depthM"));
    let picked = crate::shore::ferry_shed_anchors(outline, spec);
    if picked.is_empty() {
        return None;
    }
    let mut frames = MeshArrays::new("ferry-gate-sheds:frames");
    let mut roofs = MeshArrays::new("ferry-gate-sheds:roofs");
    let mut glass = MeshArrays::new("ferry-gate-sheds:glass");
    // Vertex tints multiply `painted-steel-light` (frames) and `membrane-roof` (roofs), so
    // realistic weathered maritime steel, sandstone plinths and dark zinc-slate stay well below 1.0.
    let steel_dark: Rgb = [0.19, 0.22, 0.25];
    let steel_mid: Rgb = [0.33, 0.34, 0.33];
    let bronze_trim: Rgb = [0.36, 0.34, 0.30];
    let plinth_col: Rgb = [0.44, 0.41, 0.36];
    let roof_deck: Rgb = [0.24, 0.27, 0.30];
    let roof_rib: Rgb = [0.17, 0.19, 0.22];
    let soffit_col: Rgb = [0.22, 0.18, 0.14];
    for (p, yaw) in &picked {
        // the box's frame: local x (its width) turned by -yaw as `ferry_sheds` turns it.
        let (c, s) = ((-yaw).cos(), (-yaw).sin());
        let at = |lx: f64, lz: f64, y: f64| [p[0] + lx * c - lz * s, deck_y + y, p[1] + lx * s + lz * c];
        let (hx, hz) = (w / 2.0, d / 2.0);
        let face = |m: &mut MeshArrays, q: [[f64; 3]; 4], want: [f64; 3], col: Rgb| {
            let e1 = [q[1][0] - q[0][0], q[1][1] - q[0][1], q[1][2] - q[0][2]];
            let e2 = [q[3][0] - q[0][0], q[3][1] - q[0][1], q[3][2] - q[0][2]];
            let nn = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
            if nn[0] * want[0] + nn[1] * want[1] + nn[2] * want[2] >= 0.0 { m.quad(q[0], q[1], q[2], q[3], col) } else { m.quad(q[3], q[2], q[1], q[0], col) }
        };
        let dir = |lx: f64, lz: f64| [lx * c - lz * s, 0.0, lx * s + lz * c];
        // an axis box in the shed's frame
        let lbox = |m: &mut MeshArrays, x: [f64; 2], z: [f64; 2], y: [f64; 2], col: Rgb| {
            let v = |i: usize, j: usize, k: usize| at(x[i], z[k], y[j]);
            face(m, [v(0, 0, 1), v(1, 0, 1), v(1, 1, 1), v(0, 1, 1)], dir(0.0, 1.0), col);
            face(m, [v(0, 0, 0), v(1, 0, 0), v(1, 1, 0), v(0, 1, 0)], dir(0.0, -1.0), col);
            face(m, [v(1, 0, 0), v(1, 0, 1), v(1, 1, 1), v(1, 1, 0)], dir(1.0, 0.0), col);
            face(m, [v(0, 0, 0), v(0, 0, 1), v(0, 1, 1), v(0, 1, 0)], dir(-1.0, 0.0), col);
            face(m, [v(0, 1, 0), v(1, 1, 0), v(1, 1, 1), v(0, 1, 1)], [0.0, 1.0, 0.0], col);
            face(m, [v(0, 0, 0), v(1, 0, 0), v(1, 0, 1), v(0, 0, 1)], [0.0, -1.0, 0.0], scale_rgb(col, 0.6));
        };
        // Slanted structural member (rafter or knee-brace) in local shed coords.
        let lbeam = |m: &mut MeshArrays, a: [f64; 3], b: [f64; 3], bw: f64, bh: f64, col: Rgb| {
            m.beam(at(a[0], a[2], a[1]), at(b[0], b[2], b[1]), bw, bh, col);
        };

        let cw = GATE_COLUMN_M / 2.0;
        let bays_x = (w / GATE_BAY_M).round().max(2.0) as usize;
        let ox = hx + GATE_OVERHANG_M * 0.90;
        let oz = hz + GATE_OVERHANG_M;
        let eave = GATE_EAVE_M;
        let ridge = eave + GATE_RIDGE_RISE_M;
        let slope_y = |abs_z: f64| ridge - (abs_z / oz) * GATE_RIDGE_RISE_M;
        let header_bot = eave - 0.30;
        let plinth_top = 0.46;
        let glass_top = GATE_GLAZING_TOP_M;

        // 1. Structural steel columns along both long facades, seated on sandstone plinths,
        //    carrying transverse outrigger rafters and 45-degree diagonal knee-braces.
        for k in 0..=bays_x {
            let lx = -hx + w * k as f64 / bays_x as f64;
            for side in [-1.0, 1.0] {
                let lz = side * hz;
                let col_top = slope_y(hz) - 0.14;
                // Weathered sandstone/concrete base plinth + dark steel anchor flange + slender steel column.
                lbox(&mut frames, [lx - 0.19, lx + 0.19], [lz - 0.19, lz + 0.19], [-0.08, plinth_top], plinth_col);
                lbox(&mut frames, [lx - 0.14, lx + 0.14], [lz - 0.14, lz + 0.14], [plinth_top, plinth_top + 0.08], steel_dark);
                lbox(&mut frames, [lx - cw, lx + cw], [lz - cw, lz + cw], [plinth_top, col_top], steel_dark);
                // Diagonal structural knee-brace out into cantilevered eave beam + inward into roof rafter.
                lbeam(&mut frames, [lx, 3.05, lz], [lx, slope_y(hz + 1.05) - 0.18, side * (hz + 1.05)], 0.08, 0.10, steel_dark);
                lbeam(&mut frames, [lx, 3.05, lz], [lx, slope_y(hz - 1.15) - 0.18, side * (hz - 1.15)], 0.08, 0.10, steel_dark);
                // Longitudinal knee-braces along the eaves portal beam.
                if k < bays_x {
                    lbeam(&mut frames, [lx + cw, 3.15, lz], [lx + 0.95, header_bot, lz], 0.075, 0.09, steel_dark);
                }
                if k > 0 {
                    lbeam(&mut frames, [lx - cw, 3.15, lz], [lx - 0.95, header_bot, lz], 0.075, 0.09, steel_dark);
                }
            }
            // Exposed transverse steel roof rafter spanning full cantilevered width (-oz..+oz) on both slopes,
            // plus a slender horizontal tension tie-rod spanning between opposing column tops.
            let y_eave_rafter = eave - 0.14;
            let y_ridge_rafter = ridge - 0.16;
            lbeam(&mut frames, [lx, y_eave_rafter, -oz + 0.15], [lx, y_ridge_rafter, 0.0], 0.12, 0.18, steel_dark);
            lbeam(&mut frames, [lx, y_eave_rafter, oz - 0.15], [lx, y_ridge_rafter, 0.0], 0.12, 0.18, steel_dark);
            lbeam(&mut frames, [lx, header_bot + 0.10, -hz], [lx, header_bot + 0.10, hz], 0.06, 0.06, steel_mid);
        }

        // 2. Longitudinal eaves portal beams, cantilevered overhang purlins, and slim louvered frieze
        //    (open sunlit clerestory gap between glass_top = 2.50 m and frieze = 3.38 m).
        for side in [-1.0, 1.0] {
            let lz = side * hz;
            // Primary eaves header beam + lower cantilevered purlin catching the overhang rafters.
            lbox(&mut frames, [-ox + 0.1, ox - 0.1], [lz - 0.10, lz + 0.10], [header_bot, eave + 0.04], steel_dark);
            let pz = side * (hz + GATE_OVERHANG_M * 0.58);
            let py = slope_y(pz.abs()) - 0.16;
            lbox(&mut frames, [-ox + 0.1, ox - 0.1], [pz - 0.07, pz + 0.07], [py - 0.08, py + 0.06], steel_dark);
            // Two slender horizontal bronze-pewter louver slats below the eaves header (open transit valence).
            for ly in [header_bot - 0.22, header_bot - 0.42] {
                lbox(&mut frames, [-hx + cw, hx - cw], [lz - 0.05, lz + 0.05], [ly - 0.035, ly + 0.035], bronze_trim);
            }
            // End-portal transverse header beams across the open 9 m ends (no solid barn walls!).
            let ex = side * hx;
            lbox(&mut frames, [ex - 0.10, ex + 0.10], [-hz, hz], [header_bot, header_bot + 0.24], steel_dark);
            // Open-louvered steel sun-screen in the upper gable triangle on each end (3 crisp horizontal louvers).
            for (ly, half_span) in [(header_bot + 0.44, hz * 0.82), (header_bot + 0.68, hz * 0.56), (header_bot + 0.92, hz * 0.30)] {
                if ly < ridge - 0.25 {
                    lbox(&mut frames, [ex - 0.05, ex + 0.05], [-half_span, half_span], [ly - 0.04, ly + 0.04], bronze_trim);
                }
            }
        }

        // 3. Recessed passenger windscreens along the long facades: leave a wide open central boarding
        //    entry (`|x| < 1.5 m`) and open 9 m ends so the shelter reads as an airy transit canopy.
        let bay_w = w / bays_x as f64;
        for side in [-1.0, 1.0] {
            let lz_curb = side * hz;
            let lz_glass = side * (hz - 0.22);
            for k in 0..bays_x {
                let x0 = -hx + bay_w * k as f64 + cw;
                let x1 = -hx + bay_w * (k + 1) as f64 - cw;
                // Trim bays touching x = 0 to open a 3.0 m central boarding portal.
                let segs: [(f64, f64); 2] = if x0 < 1.5 && x1 > -1.5 {
                    [(x0, (-1.5_f64).min(x1)), ((1.5_f64).max(x0), x1)]
                } else {
                    [(x0, x1), (0.0, 0.0)]
                };
                for (sa, sb) in segs {
                    if sb - sa < 0.6 {
                        continue;
                    }
                    // Low weathered sandstone kick-curb + dark-steel bottom/mid/top windscreen transoms.
                    lbox(&mut frames, [sa, sb], [lz_curb - 0.24, lz_curb + 0.08], [-0.04, plinth_top], plinth_col);
                    lbox(&mut frames, [sa, sb], [lz_glass - 0.045, lz_glass + 0.045], [plinth_top, plinth_top + 0.07], steel_dark);
                    lbox(&mut frames, [sa, sb], [lz_glass - 0.04, lz_glass + 0.04], [1.38, 1.44], steel_mid);
                    lbox(&mut frames, [sa, sb], [lz_glass - 0.055, lz_glass + 0.055], [glass_top - 0.06, glass_top + 0.04], steel_dark);
                    // Crisp intermediate mullions (~1.2 m rhythm) framing the recessed smoked glass panes.
                    let panes = ((sb - sa) / 1.25).round().max(1.0) as usize;
                    for m in 1..panes {
                        let mx = sa + (sb - sa) * m as f64 / panes as f64;
                        lbox(&mut frames, [mx - 0.032, mx + 0.032], [lz_glass - 0.04, lz_glass + 0.04], [plinth_top, glass_top], steel_dark);
                    }
                    // Two-sided recessed neutral-smoked architectural windscreen glazing.
                    let gq = [at(sa, lz_glass, plinth_top + 0.05), at(sb, lz_glass, plinth_top + 0.05), at(sb, lz_glass, glass_top - 0.04), at(sa, lz_glass, glass_top - 0.04)];
                    face(&mut glass, gq, dir(0.0, side), GATE_GLASS_LINEAR);
                    face(&mut glass, gq, dir(0.0, -side), scale_rgb(GATE_GLASS_LINEAR, 0.8));
                }
            }
        }

        // 4. Interior central passenger wind-baffle spine & waiting benches (gives real architectural depth).
        let spine_hx = hx - 2.6;
        if spine_hx > 1.0 {
            lbox(&mut frames, [-spine_hx, spine_hx], [-0.14, 0.14], [0.0, 2.25], steel_dark);
            lbox(&mut frames, [-spine_hx - 0.06, spine_hx + 0.06], [-0.20, 0.20], [2.22, 2.34], bronze_trim);
            // Warm bench seats flanking the central spine.
            lbox(&mut frames, [-spine_hx + 0.4, spine_hx - 0.4], [-0.62, -0.14], [0.42, 0.50], plinth_col);
            lbox(&mut frames, [-spine_hx + 0.4, spine_hx - 0.4], [0.14, 0.62], [0.42, 0.50], plinth_col);
        }

        // 5. Low-pitched cantilevered dark standing-seam zinc-slate canopy roof, shadowed warm soffit,
        //    slim dark-steel eave fascia, and raised 3D standing-seam ribs.
        let fascia_h = 0.22;
        let seam_count = ((2.0 * ox) / 1.45).round().max(6.0) as usize;
        for side in [-1.0, 1.0] {
            let e_out = eave;
            let r_out = ridge;
            let q_top = [at(-ox, side * oz, e_out), at(ox, side * oz, e_out), at(ox, 0.0, r_out), at(-ox, 0.0, r_out)];
            face(&mut roofs, q_top, [dir(0.0, side)[0] * 0.25, 1.0, dir(0.0, side)[2] * 0.25], roof_deck);
            // Shadowed warm-bronze canopy soffit underside.
            let q_bot = [at(-ox, side * oz, e_out - fascia_h), at(ox, side * oz, e_out - fascia_h), at(ox, 0.0, r_out - fascia_h), at(-ox, 0.0, r_out - fascia_h)];
            face(&mut frames, q_bot, [0.0, -1.0, 0.0], soffit_col);
            // Slim dark-steel outer eave fascia + subtle overhanging drip lip.
            face(&mut frames, [at(-ox, side * oz, e_out - fascia_h), at(ox, side * oz, e_out - fascia_h), at(ox, side * oz, e_out + 0.04), at(-ox, side * oz, e_out + 0.04)], dir(0.0, side), steel_dark);
            // Raised standing-seam roof ribs running eave-to-ridge.
            for k in 0..=seam_count {
                let sx = -ox + 2.0 * ox * k as f64 / seam_count as f64;
                lbeam(&mut roofs, [sx, e_out + 0.03, side * (oz - 0.06)], [sx, r_out + 0.03, side * 0.08], 0.055, 0.065, roof_rib);
            }
            // Slim slanted gable end verge fascia (matches roof thickness only — no chunky solid box!).
            let ex = side * ox;
            for rside in [-1.0, 1.0] {
                face(
                    &mut frames,
                    [at(ex, rside * oz, e_out - fascia_h), at(ex, 0.0, r_out - fascia_h), at(ex, 0.0, r_out + 0.04), at(ex, rside * oz, e_out + 0.04)],
                    dir(side, 0.0),
                    steel_dark,
                );
            }
        }

        // 6. Raised central glazed clerestory skylight monitor along the ridge (echoes Ferry Building nave).
        let mon_hx = hx - 0.9;
        let mon_hz = 1.05;
        let mon_base = slope_y(mon_hz) - 0.04;
        let mon_eave = ridge + 0.56;
        let mon_ridge = mon_eave + 0.28;
        let mon_ox = mon_hx + 0.48;
        let mon_oz = mon_hz + 0.45;
        if mon_hx > 1.5 {
            // Monitor base curb + dark-steel posts at bay lines + slim smoked clerestory glazing.
            lbox(&mut frames, [-mon_hx, mon_hx], [-mon_hz, mon_hz], [mon_base, ridge + 0.08], steel_dark);
            for side in [-1.0, 1.0] {
                let gz = side * (mon_hz - 0.05);
                face(&mut glass, [at(-mon_hx, gz, ridge + 0.08), at(mon_hx, gz, ridge + 0.08), at(mon_hx, gz, mon_eave - 0.08), at(-mon_hx, gz, mon_eave - 0.08)], dir(0.0, side), GATE_GLASS_LINEAR);
                lbox(&mut frames, [-mon_hx, mon_hx], [side * mon_hz - 0.05, side * mon_hz + 0.05], [mon_eave - 0.12, mon_eave], steel_dark);
            }
            for k in 0..=bays_x {
                let px = (-hx + bay_w * k as f64).clamp(-mon_hx, mon_hx);
                for side in [-1.0, 1.0] {
                    lbox(&mut frames, [px - 0.055, px + 0.055], [side * mon_hz - 0.055, side * mon_hz + 0.055], [ridge + 0.04, mon_eave], steel_dark);
                }
            }
            // Monitor end panels & overhanging standing-seam cap roof.
            for side in [-1.0, 1.0] {
                let ex = side * mon_hx;
                face(&mut frames, [at(ex, -mon_hz, ridge + 0.06), at(ex, mon_hz, ridge + 0.06), at(ex, mon_hz, mon_eave), at(ex, -mon_hz, mon_eave)], dir(side, 0.0), steel_dark);
                face(&mut roofs, [at(-mon_ox, side * mon_oz, mon_eave), at(mon_ox, side * mon_oz, mon_eave), at(mon_ox, 0.0, mon_ridge), at(-mon_ox, 0.0, mon_ridge)], [dir(0.0, side)[0] * 0.25, 1.0, dir(0.0, side)[2] * 0.25], roof_rib);
                face(&mut frames, [at(-mon_ox, side * mon_oz, mon_eave - 0.12), at(mon_ox, side * mon_oz, mon_eave - 0.12), at(mon_ox, side * mon_oz, mon_eave + 0.02), at(-mon_ox, side * mon_oz, mon_eave + 0.02)], dir(0.0, side), steel_dark);
                let mex = side * mon_ox;
                for rside in [-1.0, 1.0] {
                    face(&mut frames, [at(mex, rside * mon_oz, mon_eave - 0.12), at(mex, 0.0, mon_ridge - 0.12), at(mex, 0.0, mon_ridge + 0.02), at(mex, rside * mon_oz, mon_eave + 0.02)], dir(side, 0.0), steel_dark);
                }
            }
        }
    }
    Some(GateSheds { frames: frames.freeze(), roofs: roofs.freeze(), glass: glass.freeze() })
}

fn scale_rgb(c: Rgb, k: f64) -> Rgb {
    [c[0] * k, c[1] * k, c[2] * k]
}

impl std::fmt::Debug for ShorePress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ShorePress({} edges, {} piers)", self.edges.len(), self.piers.len())
    }
}

/// One pier: its outline (the citywide map's ring), its deck level, its box.
#[derive(Clone, Debug)]
pub struct Pier {
    pub ring: Vec<[f64; 2]>,
    pub deck_y: f64,
    pub area_m2: f64,
    /// Share of its plan samples whose ground is LiDAR deck (over the sea).
    pub lidar_share: f64,
    pub min: [f64; 2],
    pub max: [f64; 2],
}

/// The National Park Service documents a continuous concrete parapet on Municipal
/// Pier. Follow the city polygon around the breakwater, leaving its entry clear.
/// The rail sections and post spacing are visual approximations.
pub fn municipal_parapet(piers: &[Pier]) -> Option<crate::geometry::Frozen> {
    municipal_parapet_with_feet(piers, &mut PileFeet::default())
}

pub(crate) fn municipal_parapet_with_feet(piers: &[Pier], feet: &mut PileFeet) -> Option<crate::geometry::Frozen> {
    let (_, pier) = municipal_pier(piers)?;
    let mut m = MeshArrays::new("municipal-pier-parapet");
    let top = pier.deck_y + 1.10;
    let winding = signed_area(&pier.ring).signum();
    let rail = crate::geometry::srgb_hex(0xb8b6a7);
    let base = crate::geometry::srgb_hex(0x919286);
    for (a, b) in pier.ring.iter().zip(pier.ring.iter().cycle().skip(1)) {
        let len = (a[0] - b[0]).hypot(a[1] - b[1]);
        if len < 0.1 || a[1].min(b[1]) > pier.max[1] - 4.0 { continue; }
        let line = |y| ([a[0], y, a[1]], [b[0], y, b[1]]);
        for (y, h, w, color) in [(top, 0.19, 0.30, rail), (pier.deck_y + 0.19, 0.35, 0.26, base)] {
            let (start, end) = line(y);
            m.beam(start, end, w, h, color);
        }
        // The interior row carries a longitudinal beam even along the stretches
        // classified as filled land in the shoreline mask. The outer fascia and
        // its short piles come from the ordinary waterfront builder.
        let nx = -(b[1] - a[1]) * winding / len;
        let nz = (b[0] - a[0]) * winding / len;
        let girder = |p: &[f64; 2]| [p[0] + 1.65 * nx, pier.deck_y - 0.48, p[1] + 1.65 * nz];
        m.beam(girder(a), girder(b), 0.85, 0.89, base);
        let bays = (len / PIER_PILE_PITCH_M).ceil() as usize;
        for i in 0..bays {
            let t = (i as f64 + 0.5) / bays as f64;
            let x = a[0] + t * (b[0] - a[0]) + 1.65 * nx;
            let z = a[1] + t * (b[1] - a[1]) + 1.65 * nz;
            let start = m.positions.len();
            m.box_(x, z, eo_water::SEA_LEVEL_Y - 2.0, pier.deck_y - 0.70, 0.38, 0.38, 0.0, base, crate::geometry::NO_BOTTOM);
            feet.boxes.push(start..m.positions.len());
        }
        let spans = (len / 2.4).ceil() as usize;
        for i in 0..spans {
            let t = i as f64 / spans as f64;
            let x = a[0] + t * (b[0] - a[0]);
            let z = a[1] + t * (b[1] - a[1]);
            m.box_(x, z, pier.deck_y + 0.25, top, 0.13, 0.13, 0.0, rail, crate::geometry::NO_BOTTOM);
        }
    }
    Some(m.freeze())
}

fn municipal_pier(piers: &[Pier]) -> Option<(usize, &Pier)> {
    let tip = eo_data::groundscale::plan([-431.5, -3610.0]);
    piers.iter().enumerate().find(|(_, p)| p.contains(tip[0] - 6.0, tip[1]) || (p.ring.len() > 20 && p.near(tip[0], tip[1], 5.0)))
}

/// The city's shoreline simplifies the pier curve into longer chords. Their
/// midpoints can lie more than the ordinary 6 game metres from the surveyed
/// pier. Use one survey cell of tolerance here so the pier is not walled as land.
pub fn align_municipal_edges(edges: &mut [ShoreEdge], census: &mut EdgeCensus, piers: &[Pier]) {
    let Some((index, pier)) = municipal_pier(piers) else { return; };
    let reach = 8.0 * eo_data::groundscale::scale();
    for e in edges.iter_mut().filter(|e| e.pier.is_none()) {
        let x = (e.a[0] + e.b[0]) * 0.5;
        let z = (e.a[1] + e.b[1]) * 0.5;
        if z < pier.max[1] - reach && pier.near(x, z, reach) {
            let length = (e.a[0] - e.b[0]).hypot(e.a[1] - e.b[1]);
            e.pier = Some(index);
            census.wall -= 1;
            census.wall_m -= length;
            census.pier += 1;
            census.pier_m += length;
        }
    }
}

impl Pier {
    pub fn contains(&self, x: f64, z: f64) -> bool {
        if x < self.min[0] || x > self.max[0] || z < self.min[1] || z > self.max[1] {
            return false;
        }
        point_in_ring(&self.ring, x, z)
    }
    /// Inside, or within `d` of its outline.
    pub fn near(&self, x: f64, z: f64, d: f64) -> bool {
        if x < self.min[0] - d || x > self.max[0] + d || z < self.min[1] - d || z > self.max[1] + d {
            return false;
        }
        self.contains(x, z) || ring_distance(&self.ring, x, z) < d
    }
}

pub fn point_in_ring(ring: &[[f64; 2]], x: f64, z: f64) -> bool {
    let mut inside = false;
    let mut j = ring.len() - 1;
    for i in 0..ring.len() {
        let (a, c) = (ring[i], ring[j]);
        if (a[1] > z) != (c[1] > z) && x < ((c[0] - a[0]) * (z - a[1])) / (c[1] - a[1]) + a[0] {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn seg_distance(x: f64, z: f64, a: [f64; 2], b: [f64; 2]) -> (f64, f64) {
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dz * dz;
    let t = if l2 > 0.0 { (((x - a[0]) * dx + (z - a[1]) * dz) / l2).clamp(0.0, 1.0) } else { 0.0 };
    ((x - a[0] - dx * t).hypot(z - a[1] - dz * t), t)
}

pub fn ring_distance(ring: &[[f64; 2]], x: f64, z: f64) -> f64 {
    let mut d = f64::INFINITY;
    for i in 0..ring.len() {
        d = d.min(seg_distance(x, z, ring[i], ring[(i + 1) % ring.len()]).0);
    }
    d
}

/// The citywide map's pier rings (`piers`: `len` vertices per feature, `xy`
/// feature-delta encoded, times `meta.quantum`), in world metres.
pub fn load_pier_rings(repo_root: &std::path::Path) -> Result<Vec<Vec<[f64; 2]>>> {
    use std::io::Read;
    #[derive(serde::Deserialize)]
    struct Meta {
        quantum: f64,
    }
    #[derive(serde::Deserialize)]
    struct Layer {
        len: Vec<usize>,
        xy: Vec<i64>,
    }
    #[derive(serde::Deserialize)]
    struct Map {
        meta: Meta,
        piers: Layer,
    }
    let bytes = std::fs::read(repo_root.join("public/data/sf/sf-map-citywide.json.gz"))?;
    let mut s = String::new();
    flate2::read::GzDecoder::new(&bytes[..]).read_to_string(&mut s)?;
    let m: Map = serde_json::from_str(&s)?;
    let q = m.meta.quantum * eo_data::groundscale::scale(); // the piers at the scaled ground plan
    let mut out = Vec::with_capacity(m.piers.len.len());
    let mut k = 0usize;
    for &n in &m.piers.len {
        if n == 0 || k + 2 * n > m.piers.xy.len() {
            break;
        }
        let (mut x, mut z) = (m.piers.xy[k], m.piers.xy[k + 1]);
        let mut r = vec![[x as f64 * q, z as f64 * q]];
        for v in 1..n {
            x += m.piers.xy[k + 2 * v];
            z += m.piers.xy[k + 2 * v + 1];
            r.push([x as f64 * q, z as f64 * q]);
        }
        k += 2 * n;
        if r.len() > 2 && (r[0][0] - r[r.len() - 1][0]).hypot(r[0][1] - r[r.len() - 1][1]) < 1e-6 {
            r.pop();
        }
        if r.len() >= 3 {
            out.push(r);
        }
    }
    Ok(out)
}

/// The citywide map's `pierLines` (OSM `man_made=pier` mapped as lines:
/// outlines drawn as open ways, a walkway as its two edges) on the Presidio
/// shore (`PIER_LINES_BOX_SURVEYED`), closed into rings: a way whose ends
/// meet within `PIER_LINE_CLOSE_M` is a ring; ways sharing an end are joined
/// first. Torpedo Wharf (an L of asphalt on piles, OSM relation 8346136's
/// outline) and the Coast Guard pier's walkway (its two edges) come out of
/// it; the `piers` layer has neither. Rings inside an existing pier ring are
/// dropped. World metres at the scaled ground plan.
pub fn load_pier_line_rings(repo_root: &std::path::Path, existing: &[Vec<[f64; 2]>]) -> Result<Vec<Vec<[f64; 2]>>> {
    use std::io::Read;
    #[derive(serde::Deserialize)]
    struct Meta {
        quantum: f64,
    }
    #[derive(serde::Deserialize)]
    struct Layer {
        len: Vec<usize>,
        xy: Vec<i64>,
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Map {
        meta: Meta,
        pier_lines: Layer,
    }
    let bytes = std::fs::read(repo_root.join("public/data/sf/sf-map-citywide.json.gz"))?;
    let mut s = String::new();
    flate2::read::GzDecoder::new(&bytes[..]).read_to_string(&mut s)?;
    let m: Map = serde_json::from_str(&s)?;
    let q = m.meta.quantum;
    let b = PIER_LINES_BOX_SURVEYED;
    let mut ways: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut k = 0usize;
    for &n in &m.pier_lines.len {
        if n == 0 || k + 2 * n > m.pier_lines.xy.len() {
            break;
        }
        let (mut x, mut z) = (m.pier_lines.xy[k], m.pier_lines.xy[k + 1]);
        let mut w = vec![[x as f64 * q, z as f64 * q]];
        for v in 1..n {
            x += m.pier_lines.xy[k + 2 * v];
            z += m.pier_lines.xy[k + 2 * v + 1];
            w.push([x as f64 * q, z as f64 * q]);
        }
        k += 2 * n;
        if w.iter().all(|p| p[0] >= b[0] && p[0] <= b[2] && p[1] >= b[1] && p[1] <= b[3]) && w.len() >= 2 {
            ways.push(w);
        }
    }
    let rings = close_pier_lines(ways);
    let gs = eo_data::groundscale::scale();
    Ok(rings
        .into_iter()
        .map(|r| r.into_iter().map(|p| [p[0] * gs, p[1] * gs]).collect::<Vec<_>>())
        .filter(|r| {
            let c = ring_centroid(r);
            !existing.iter().any(|e| e.len() >= 3 && point_in_ring(e, c[0], c[1]))
        })
        .collect())
}

/// Ways joined at shared ends (within `PIER_LINE_JOIN_M`) into chains, the
/// chains whose ends meet within `PIER_LINE_CLOSE_M` as rings of at least
/// `PIER_MIN_AREA_M2`.
pub fn close_pier_lines(mut ways: Vec<Vec<[f64; 2]>>) -> Vec<Vec<[f64; 2]>> {
    let d = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]);
    let mut out = Vec::new();
    while let Some(mut chain) = ways.pop() {
        loop {
            let (head, tail) = (chain[0], *chain.last().unwrap());
            if d(head, tail) < PIER_LINE_CLOSE_M && chain.len() >= 3 {
                break;
            }
            let Some((i, rev, front)) = ways.iter().enumerate().find_map(|(i, w)| {
                let (a, b) = (w[0], *w.last().unwrap());
                if d(tail, a) < PIER_LINE_JOIN_M {
                    Some((i, false, false))
                } else if d(tail, b) < PIER_LINE_JOIN_M {
                    Some((i, true, false))
                } else if d(head, b) < PIER_LINE_JOIN_M {
                    Some((i, false, true))
                } else if d(head, a) < PIER_LINE_JOIN_M {
                    Some((i, true, true))
                } else {
                    None
                }
            }) else {
                break;
            };
            let mut w = ways.swap_remove(i);
            if rev {
                w.reverse();
            }
            if front {
                w.pop();
                w.extend(chain);
                chain = w;
            } else {
                chain.extend(w.into_iter().skip(1));
            }
        }
        if chain.len() >= 3 && d(chain[0], *chain.last().unwrap()) < PIER_LINE_CLOSE_M {
            if d(chain[0], *chain.last().unwrap()) < 1e-6 {
                chain.pop();
            }
            if signed_area(&chain).abs() >= PIER_MIN_AREA_M2 {
                out.push(chain);
            }
        }
    }
    out
}

/// The Presidio shore the pier lines are built on (surveyed metres in the
/// city frame: west, north, east, south): Crissy Field's west end, from the
/// Warming Hut's wharf to the Coast Guard station.
pub const PIER_LINES_BOX_SURVEYED: [f64; 4] = [-4600.0, -3600.0, -4000.0, -3000.0];
/// Pier-line ends this close are one point; a chain whose ends are this close is closed. Metres.
pub const PIER_LINE_JOIN_M: f64 = 1.0;
pub const PIER_LINE_CLOSE_M: f64 = 8.0;
/// The quay railing's paint, sRGB: the grey steel of the guard rail on the
/// quay wall (set.rs, the Ferry Building corridor's railing, the value it
/// has had since its port) and of the pier-line decks' guard.
pub const RAIL_PAINT_SRGB: u32 = 0x6e7478;

/// The guard on a pier-line deck's water edges: 42 in (1.067 m) over the deck,
/// California Building Code section 1015.3 (guards); people are drawn at real
/// size (`groundscale::people`, 1.0), so this is not scaled. Posts at 8 ft
/// (2.44 m, INFERRED), a 4 in (0.1 m) post and rail section (INFERRED).
pub const PIER_GUARD_M: f64 = 1.067;
pub const PIER_GUARD_POST_PITCH_M: f64 = 2.44;
pub const PIER_GUARD_SECTION_M: f64 = 0.1;

/// The guard along a pier deck's outline edges over the water (`water_at`):
/// posts, a top rail and a mid rail, in `colour`.
pub fn pier_guard(m: &mut MeshArrays, p: &Pier, water_at: &dyn Fn(f64, f64) -> bool, colour: Rgb) {
    let inward = if signed_area(&p.ring) > 0.0 { 1.0 } else { -1.0 };
    for i in 0..p.ring.len() {
        let (a, b) = (p.ring[i], p.ring[(i + 1) % p.ring.len()]);
        let l = (b[0] - a[0]).hypot(b[1] - a[1]);
        if l < 1.0 {
            continue;
        }
        let n = [-(b[1] - a[1]) / l * inward, (b[0] - a[0]) / l * inward];
        let (mx, mz) = ((a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0);
        let n = if point_in_ring(&p.ring, mx + n[0], mz + n[1]) { n } else { [-n[0], -n[1]] };
        if !water_at(mx - n[0] * 2.0, mz - n[1] * 2.0) {
            continue;
        }
        let inset = PIER_GUARD_SECTION_M;
        let (a, b) = ([a[0] + n[0] * inset, a[1] + n[1] * inset], [b[0] + n[0] * inset, b[1] + n[1] * inset]);
        let y = p.deck_y;
        let posts = (l / PIER_GUARD_POST_PITCH_M).ceil().max(1.0) as usize;
        for k in 0..=posts {
            let t = k as f64 / posts as f64;
            let q = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            m.beam([q[0], y, q[1]], [q[0], y + PIER_GUARD_M, q[1]], PIER_GUARD_SECTION_M, PIER_GUARD_SECTION_M, colour);
        }
        for h in [PIER_GUARD_M, PIER_GUARD_M * 0.5] {
            m.beam([a[0], y + h, a[1]], [b[0], y + h, b[1]], PIER_GUARD_SECTION_M, PIER_GUARD_SECTION_M * 0.6, colour);
        }
    }
}

/// The piers the city draws: every ring of `PIER_MIN_AREA_M2` or more within
/// `PIER_MAX_SHORE_M` of a shoreline ring edge, its deck at the median LiDAR
/// ground inside it plus the road lift (less `PIER_UNDER_ROAD_M`), or
/// `PIER_FLOAT_FREEBOARD_M` over the sea where nothing inside reads LiDAR.
pub fn plan_piers(rings: &[Vec<[f64; 2]>], oracle: &eo_water::shoreline::Oracle, ground: &(dyn Fn(f64, f64) -> Option<f64> + Sync), sea: f64, road_lift: f64) -> Vec<Pier> {
    let mut out = Vec::new();
    for r in rings {
        let area = signed_area(r).abs();
        if area < PIER_MIN_AREA_M2 {
            continue;
        }
        let (mut mn, mut mx) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for p in r {
            for k in 0..2 {
                mn[k] = mn[k].min(p[k]);
                mx[k] = mx[k].max(p[k]);
            }
        }
        let c = ring_centroid(r);
        if oracle.edge_distance(c[0], c[1], PIER_MAX_SHORE_M + 1.0) > PIER_MAX_SHORE_M {
            continue;
        }
        // Plan samples on a grid of about 300 points.
        let step = ((mx[0] - mn[0]) * (mx[1] - mn[1]) / 300.0).sqrt().max(1.5);
        let mut hs: Vec<f64> = Vec::new();
        let mut n = 0usize;
        let mut z = mn[1] + step / 2.0;
        while z < mx[1] {
            let mut x = mn[0] + step / 2.0;
            while x < mx[0] {
                if point_in_ring(r, x, z) {
                    n += 1;
                    if let Some(g) = ground(x, z).filter(|g| *g > sea + 0.3) {
                        hs.push(g);
                    }
                }
                x += step;
            }
            z += step;
        }
        if n == 0 {
            continue;
        }
        let share = hs.len() as f64 / n as f64;
        let deck_y = if share >= 0.2 {
            hs.sort_by(|a, b| a.total_cmp(b));
            hs[hs.len() / 2] + road_lift - PIER_UNDER_ROAD_M
        } else {
            sea + PIER_FLOAT_FREEBOARD_M
        };
        out.push(Pier { ring: r.clone(), deck_y, area_m2: area, lidar_share: share, min: mn, max: mx });
    }
    out
}

/// One shoreline ring edge the city builds on, with its landward unit normal.
#[derive(Clone, Copy, Debug)]
pub struct ShoreEdge {
    pub a: [f64; 2],
    pub b: [f64; 2],
    pub land: [f64; 2],
    /// The pier it belongs to (`None`: a quay wall edge).
    pub pier: Option<usize>,
}

/// What the shore classification found, for the load trace.
#[derive(Clone, Debug, Default)]
pub struct EdgeCensus {
    pub visited: usize,
    pub beach: usize,
    pub county_line: usize,
    pub wall: usize,
    pub pier: usize,
    /// Edges the 4 m probe alone (the earlier rule) would have dropped as a county line.
    pub kept_by_far_probe: usize,
    pub wall_m: f64,
    pub pier_m: f64,
}

/// The landward unit normal of a ring edge, as `buildSeawallGeometry` picks
/// it (probes at 0.6, 1.4 and 2.6 m off the edge's middle; the side two of
/// them call water is the sea).
pub fn land_normal(oracle: &eo_water::shoreline::Oracle, a: [f64; 2], b: [f64; 2]) -> Option<[f64; 2]> {
    let length = (b[0] - a[0]).hypot(b[1] - a[1]);
    if !(length > 0.05) {
        return None;
    }
    let (ux, uz) = ((b[0] - a[0]) / length, (b[1] - a[1]) / length);
    let (nx, nz) = (-uz, ux);
    let (mx, mz) = ((a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0);
    let water_side = [0.6, 1.4, 2.6].iter().filter(|d| oracle.is_water(mx + nx * **d, mz + nz * **d)).count();
    Some(if water_side >= 2 { [-nx, -nz] } else { [nx, nz] })
}

/// Every shoreline ring edge, classified: a beach (no wall, the JS rule), a
/// county line (dry land on both sides, every probe out to 64 m), a pier's
/// edge (its middle within `PIER_DILATE_M` of a pier), else a quay wall edge.
pub fn classify_edges(
    oracle: &eo_water::shoreline::Oracle,
    ground: &(dyn Fn(f64, f64) -> Option<f64> + Sync),
    sea: f64,
    beaches: &BeachRule,
    piers: &[Pier],
) -> (Vec<ShoreEdge>, EdgeCensus) {
    let mut census = EdgeCensus::default();
    let mut raw: Vec<[f64; 4]> = Vec::new();
    oracle.for_each_edge(|ax, az, bx, bz| raw.push([ax, az, bx, bz]));
    // A pier lookup on a coarse grid of their boxes.
    const CELL: f64 = 64.0;
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, p) in piers.iter().enumerate() {
        let (c0, c1) = (((p.min[0] - PIER_DILATE_M) / CELL).floor() as i64, ((p.max[0] + PIER_DILATE_M) / CELL).floor() as i64);
        let (r0, r1) = (((p.min[1] - PIER_DILATE_M) / CELL).floor() as i64, ((p.max[1] + PIER_DILATE_M) / CELL).floor() as i64);
        for r in r0..=r1 {
            for c in c0..=c1 {
                grid.entry((c, r)).or_default().push(i);
            }
        }
    }
    let dry = sea + crate::shore::SHORE_DRY_ABOVE_SEA_M;
    let mut out = Vec::new();
    // The DataSF rings run straight for hundreds of metres along the
    // Embarcadero (one edge from Rincon Park to the Bay Bridge is 373 m):
    // the wall's crest follows the ground only at an edge's ends, so a long
    // edge is cut into pieces of at most `EDGE_PIECE_M`, each classified on
    // its own (its landward side from the whole edge).
    let mut pieces: Vec<([f64; 2], [f64; 2], [f64; 2])> = Vec::new();
    for e in raw {
        census.visited += 1;
        let (a, b) = ([e[0], e[1]], [e[2], e[3]]);
        let Some(land) = land_normal(oracle, a, b) else { continue };
        let n = ((b[0] - a[0]).hypot(b[1] - a[1]) / EDGE_PIECE_M).ceil().max(1.0) as usize;
        for k in 0..n {
            let (t0, t1) = (k as f64 / n as f64, (k + 1) as f64 / n as f64);
            let p = |t: f64| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            pieces.push((p(t0), p(t1), land));
        }
    }
    for (a, b, land) in pieces {
        let (mx, mz) = ((a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0);
        let sea_n = [-land[0], -land[1]];
        let probe = |d: f64| ground(mx + sea_n[0] * d, mz + sea_n[1] * d).is_some_and(|g| g > dry);
        if COUNTY_LINE_PROBES_M.iter().all(|d| probe(*d)) {
            census.county_line += 1;
            continue;
        }
        if beaches.skip(a[0], a[1], b[0], b[1]) {
            census.beach += 1;
            continue;
        }
        if probe(COUNTY_LINE_PROBES_M[0]) {
            census.kept_by_far_probe += 1;
        }
        let length = (b[0] - a[0]).hypot(b[1] - a[1]);
        let key = ((mx / CELL).floor() as i64, (mz / CELL).floor() as i64);
        let pier = grid.get(&key).and_then(|list| list.iter().copied().find(|&i| piers[i].near(mx, mz, PIER_DILATE_M)));
        if pier.is_some() {
            census.pier += 1;
            census.pier_m += length;
        } else {
            census.wall += 1;
            census.wall_m += length;
        }
        out.push(ShoreEdge { a, b, land, pier });
    }
    (out, census)
}

/// The press the terrain takes behind a quay wall, `d` metres inland of the waterline.
pub fn press_quay(sea: f64, d: f64) -> f64 {
    sea + QUAY_PRESS_FOOT_M + QUAY_PRESS_SLOPE * d.max(0.0)
}

/// Behind a pier's edge: under the sea to the under-deck face, then the quay's slope.
pub fn press_pier_edge(sea: f64, d: f64) -> f64 {
    sea - 1.2 + QUAY_PRESS_SLOPE * (d - PIER_VOID_SETBACK_M - 0.4).max(0.0)
}

/// The terrain press of the built waterfront: a grid of the walled and pier
/// edges (with their landward normals) and the piers, answering the most a
/// terrain vertex may stand at.
pub struct ShorePress {
    sea: f64,
    cell: f64,
    municipal: Option<Pier>,
    edges: Vec<ShoreEdge>,
    edge_cells: HashMap<(i64, i64), Vec<u32>>,
    piers: Vec<(Vec<[f64; 2]>, [f64; 2], [f64; 2], f64)>,
    pier_cells: HashMap<(i64, i64), Vec<u32>>,
}

/// Demolished or submerged non-pier boundary fingers (such as Pier 24 Annex beneath Bay Bridge
/// Tower W2, where 3DEP LiDAR measures open-bay seabed at -2.59 m while the older DataSF shoreline
/// ring still outlines the former pier wider than `eo-water`'s 15 m pier probe) get artificially
/// lifted to `sea + 0.10 m` within `LAND_LIFT_REACH_M` (24 m) by `eo-water::field::press_ground_rings`.
/// Pressing vertices that arrive at or below `sea + SUNKEN_LAND_CEILING_M` within
/// `SUNKEN_LAND_REACH_M` of a non-pier shore edge back to seabed depth (`sea - 2.0 m`) lets the
/// bay water pass draw cleanly over the submerged footprint without touching real dry quays.
pub const SUNKEN_LAND_REACH_M: f64 = 26.0;
pub const SUNKEN_LAND_CEILING_M: f64 = 0.15;

impl ShorePress {
    /// The walled and pier edges the press stands behind.
    pub fn edges(&self) -> &[ShoreEdge] {
        &self.edges
    }


    pub fn new(sea: f64, edges: &[ShoreEdge], piers: &[Pier]) -> Self {
        let cell = 32.0;
        let quay_reach = CAP_W_M + 2.0;
        let mut edge_cells: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
        for (i, e) in edges.iter().enumerate() {
            let reach = if e.pier.is_none() { SUNKEN_LAND_REACH_M } else { quay_reach };
            let (x0, x1) = (e.a[0].min(e.b[0]) - reach, e.a[0].max(e.b[0]) + reach);
            let (z0, z1) = (e.a[1].min(e.b[1]) - reach, e.a[1].max(e.b[1]) + reach);
            for r in (z0 / cell).floor() as i64..=(z1 / cell).floor() as i64 {
                for c in (x0 / cell).floor() as i64..=(x1 / cell).floor() as i64 {
                    edge_cells.entry((c, r)).or_default().push(i as u32);
                }
            }
        }
        let mut pier_cells: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
        let mut list = Vec::new();
        for (i, p) in piers.iter().enumerate() {
            list.push((p.ring.clone(), [p.min[0] - 2.0, p.min[1] - 2.0], [p.max[0] + 2.0, p.max[1] + 2.0], p.deck_y));
            for r in ((p.min[1] - 2.0) / cell).floor() as i64..=((p.max[1] + 2.0) / cell).floor() as i64 {
                for c in ((p.min[0] - 2.0) / cell).floor() as i64..=((p.max[0] + 2.0) / cell).floor() as i64 {
                    pier_cells.entry((c, r)).or_default().push(i as u32);
                }
            }
        }
        Self { sea, cell, municipal: None, edges: edges.to_vec(), edge_cells, piers: list, pier_cells }
    }

    /// A LiDAR return from the pier's deck is not a seabed sounding. Keep the
    /// existing bed wherever it is deeper, and keep the Black Point land abutment.
    pub fn with_municipal_pier(mut self, piers: &[Pier]) -> Self {
        self.municipal = municipal_pier(piers).map(|(_, p)| p.clone());
        self
    }

    pub fn under_municipal_pier(&self, x: f64, z: f64) -> bool {
        self.municipal.as_ref().is_some_and(|p| {
            // Cover every terrain triangle touching the pier (8 m survey grid).
            let reach = 8.0 * std::f64::consts::SQRT_2 * eo_data::groundscale::scale();
            z < p.max[1] - reach && p.near(x, z, reach)
        })
    }

    /// The terrain height at (x, z) given the carpet's `y`: under every pier
    /// deck, and pressed behind the walled and pier edges.
    pub fn press(&self, x: f64, z: f64, y: f64) -> f64 {
        let key = ((x / self.cell).floor() as i64, (z / self.cell).floor() as i64);
        let mut out = if self.under_municipal_pier(x, z) { y.min(self.sea - 1.2) } else { y };
        if let Some(list) = self.pier_cells.get(&key) {
            for &i in list {
                let (ring, mn, mx, deck) = &self.piers[i as usize];
                if x < mn[0] || x > mx[0] || z < mn[1] || z > mx[1] {
                    continue;
                }
                if point_in_ring(ring, x, z) || ring_distance(ring, x, z) < 2.0 {
                    out = out.min(deck - 0.3);
                }
            }
        }
        let sunken_shelf = y <= self.sea + SUNKEN_LAND_CEILING_M;
        if let Some(list) = self.edge_cells.get(&key) {
            for &i in list {
                let e = &self.edges[i as usize];
                // Only the land side: the water side is the seabed already.
                let side = (x - e.a[0]) * e.land[0] + (z - e.a[1]) * e.land[1];
                if side < -0.5 {
                    continue;
                }
                let (d, _) = seg_distance(x, z, e.a, e.b);
                if d <= CAP_W_M + 2.0 {
                    let limit = if e.pier.is_some() { press_pier_edge(self.sea, d) } else { press_quay(self.sea, d) };
                    out = out.min(limit);
                }
                if sunken_shelf && e.pier.is_none() && d <= SUNKEN_LAND_REACH_M {
                    out = out.min(self.sea - 2.0);
                }
            }
        }
        out
    }
}

/// Positions of the pile boxes in their owning mesh. The terrain is assembled
/// after the waterfront, when its precise ground triangles become available.
#[derive(Clone, Default, Debug)]
pub(crate) struct PileFeet {
    boxes: Vec<std::ops::Range<usize>>,
}

impl PileFeet {
    pub(crate) fn seat(&self, mesh: &mut crate::geometry::Frozen, bed: &mut impl FnMut(f32, f32) -> f32) {
        for span in &self.boxes {
            let vertices = &mut mesh.positions[span.clone()];
            let original = vertices[1]; // first lower corner of the box
            let mut foot = original;
            for p in vertices.chunks_exact(3).filter(|p| p[1] == original) {
                // Keep the visible ends under all four corners on a sloping bed.
                // This small overlap hides the open end of the modeled support.
                foot = foot.min(bed(p[0], p[2]) - 0.08);
            }
            for p in vertices.chunks_exact_mut(3).filter(|p| p[1] == original) {
                p[1] = foot;
            }
            mesh.min_y = mesh.min_y.min(f64::from(foot));
        }
    }
}

/// The meshes of the built waterfront.
#[derive(Default)]
pub struct WaterfrontMeshes {
    /// The quay apron and the pier decks (drawn in the street's footway paving).
    pub paving: Frozen,
    /// Pier fascias, piles, the apron's kerb skirt (cast concrete).
    pub concrete: Frozen,
    /// The dark under-deck faces.
    pub under_deck: Frozen,
    pub piles: usize,
    pub(crate) municipal_feet: PileFeet,
    pub decks: usize,
}

/// The quay apron (a strip at the wall's crest, `CAP_W_M` landward of the
/// coping) and its landward kerb skirt, for each wall edge; the pier fascia,
/// under-deck face, pile row and deck strip for each pier edge; each pier's
/// deck slab over its outline and a fascia on its outline edges over open water.
#[allow(clippy::too_many_arguments)]
pub fn build_meshes(
    edges: &[ShoreEdge],
    piers: &[Pier],
    oracle: &eo_water::shoreline::Oracle,
    crest_at: &dyn Fn(f64, f64) -> f64,
    sea: f64,
    coping_land: f64,
    concrete: Rgb,
    dark: Rgb,
    apron: Rgb,
    deck: Rgb,
) -> WaterfrontMeshes {
    build_meshes_with(edges, piers, oracle, crest_at, sea, coping_land, concrete, dark, apron, &|_| deck)
}

/// `build_meshes` with each pier's deck tone given by its index (`deck_of`).
#[allow(clippy::too_many_arguments)]
pub fn build_meshes_with(
    edges: &[ShoreEdge],
    piers: &[Pier],
    oracle: &eo_water::shoreline::Oracle,
    crest_at: &dyn Fn(f64, f64) -> f64,
    sea: f64,
    coping_land: f64,
    concrete: Rgb,
    dark: Rgb,
    apron: Rgb,
    deck_of: &dyn Fn(usize) -> Rgb,
) -> WaterfrontMeshes {
    let mut paving = MeshArrays::new("waterfront:paving");
    let mut conc = MeshArrays::new("waterfront:concrete");
    let mut under = MeshArrays::new("waterfront:under-deck");
    let mut piles = 0usize;
    let municipal = municipal_pier(piers).map(|(index, _)| index);
    let mut municipal_feet = PileFeet::default();
    // Joints: the strip's corner at each edge end, to fill the wedge between consecutive edges.
    let mut ends: HashMap<(i64, i64), Vec<([f64; 2], f64, f64, Rgb)>> = HashMap::new();
    let key = |p: [f64; 2]| ((p[0] * 100.0).round() as i64, (p[1] * 100.0).round() as i64);
    for e in edges {
        let (a, b, n) = (e.a, e.b, e.land);
        let at = |p: [f64; 2], o: f64, y: f64| [p[0] + n[0] * o, y, p[1] + n[1] * o];
        let (ya, yb, inner, tone) = match e.pier {
            None => (crest_at(a[0], a[1]), crest_at(b[0], b[1]), coping_land, apron),
            Some(i) => (piers[i].deck_y, piers[i].deck_y, 0.0, deck_of(i)),
        };
        // Skip submerged ghost quay edges (e.g. demolished Pier 24 Annex beneath Bay Bridge W2)
        // where surveyed LiDAR sits underwater: prevents concrete aprons ramping down into the bay.
        if e.pier.is_none() && ya.min(yb) < sea + SUNKEN_LAND_CEILING_M {
            continue;
        }
        // The strip: from the coping (or the waterline) to CAP_W_M inland, facing up.
        let w = CAP_W_M + coping_land;
        let up = |p: [f64; 3], q: [f64; 3], r: [f64; 3], s: [f64; 3], m: &mut MeshArrays| {
            // Wound to face up whichever way the edge runs.
            let cr = (q[0] - p[0]) * (s[2] - p[2]) - (q[2] - p[2]) * (s[0] - p[0]);
            if cr < 0.0 { m.quad(p, q, r, s, tone) } else { m.quad(s, r, q, p, tone) }
        };
        up(at(a, inner, ya), at(b, inner, yb), at(b, w, yb), at(a, w, ya), &mut paving);
        // Its landward kerb skirt, down past the press.
        conc.quad(at(a, w, ya), at(b, w, yb), at(b, w, yb - 1.4), at(a, w, ya - 1.4), concrete);
        ends.entry(key(a)).or_default().push(([n[0], n[1]], ya, inner, tone));
        ends.entry(key(b)).or_default().push(([n[0], n[1]], yb, inner, tone));
        if let Some(i) = e.pier {
            let deck = piers[i].deck_y;
            let is_municipal = municipal == Some(i);
            pier_fascia(&mut conc, &mut under, a, b, n, deck, sea, concrete, dark, &mut piles, is_municipal.then_some(&mut municipal_feet));
        }
    }
    // Joint fans: the strip's wedge between two edges meeting at a point.
    for (k, list) in &ends {
        if list.len() < 2 {
            continue;
        }
        let p = [k.0 as f64 / 100.0, k.1 as f64 / 100.0];
        let (n0, y0, i0, tone) = list[0];
        let (n1, y1, i1, _) = list[1];
        let y = y0.max(y1);
        let w = CAP_W_M + i0.max(i1);
        let a = [p[0] + n0[0] * i0, y, p[1] + n0[1] * i0];
        let b = [p[0] + n0[0] * w, y, p[1] + n0[1] * w];
        let c = [p[0] + n1[0] * w, y, p[1] + n1[1] * w];
        let cr = (b[0] - a[0]) * (c[2] - a[2]) - (b[2] - a[2]) * (c[0] - a[0]);
        if cr.abs() < 1e-6 {
            continue;
        }
        if cr < 0.0 { paving.tri(a, b, c, tone) } else { paving.tri(a, c, b, tone) }
    }
    // Pier slabs over their outlines, and fascias on outline edges over open water.
    let fb_c = eo_data::groundscale::plan(crate::ferry_building::spec().tower.centre_surveyed);
    let gs = eo_data::groundscale::scale();
    let fender_reach = 2200.0 * gs;
    let bollard_reach = 1200.0 * gs;
    let prom_reach = 560.0 * gs;
    let mut decks = 0usize;
    for (index, p) in piers.iter().enumerate() {
        let deck = deck_of(index);
        if !pier_top(p, |[a,b,c]| paving.tri(a, b, c, deck)) { continue; }
        decks += 1;
        let is_municipal = municipal == Some(index);
        let p_mid = [(p.min[0] + p.max[0]) * 0.5, (p.min[1] + p.max[1]) * 0.5];
        let fb_dist = (p_mid[0] - fb_c[0]).hypot(p_mid[1] - fb_c[1]);
        // Narrow public Embarcadero promenade piers (e.g. Pier 14, Gate floats) get maritime railings & lamp stanchions.
        let is_prom_pier = !is_municipal && fb_dist < prom_reach && p.area_m2 < 3_000.0 * gs * gs;
        let inward = if signed_area(&p.ring) > 0.0 { 1.0 } else { -1.0 };
        for i in 0..p.ring.len() {
            let (a, b) = (p.ring[i], p.ring[(i + 1) % p.ring.len()]);
            let l = (b[0] - a[0]).hypot(b[1] - a[1]);
            if l < 0.5 {
                continue;
            }
            let (mx, mz) = ((a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0);
            // Over open water: the middle, and a metre out from it, is water.
            let n = [-(b[1] - a[1]) / l * inward, (b[0] - a[0]) / l * inward];
            if !(oracle.is_water(mx, mz) && oracle.is_water(mx - n[0], mz - n[1])) {
                continue;
            }
            // The outline's interior side: a point a metre in must be inside.
            let n = if point_in_ring(&p.ring, mx + n[0], mz + n[1]) { n } else { [-n[0], -n[1]] };
            pier_fascia(&mut conc, &mut under, a, b, n, p.deck_y, sea, concrete, dark, &mut piles, is_municipal.then_some(&mut municipal_feet));
            let edge_fb = (mx - fb_c[0]).hypot(mz - fb_c[1]);
            if !is_municipal {
                pier_edge_hardware(&mut conc, a, b, n, p.deck_y, sea, concrete, edge_fb < fender_reach, edge_fb < bollard_reach, is_prom_pier);
            }
        }
    }
    WaterfrontMeshes { paving: paving.freeze(), concrete: conc.freeze(), under_deck: under.freeze(), piles, decks, municipal_feet }
}

/// The upward-facing pier slab shared by the waterfront mesh and the headless world.
pub fn pier_top(p: &Pier, mut emit: impl FnMut([[f64;3];3])) -> bool {
    let mut ear = earcut::Earcut::<f64>::new();
    let mut indices = Vec::<u32>::new();
    ear.earcut(p.ring.iter().copied(), &[] as &[u32], &mut indices);
    for t in indices.chunks_exact(3) {
        let v = |i: u32| [p.ring[i as usize][0], p.deck_y, p.ring[i as usize][1]];
        let (a,b,c) = (v(t[0]),v(t[1]),v(t[2]));
        let cr = (b[0]-a[0])*(c[2]-a[2]) - (b[2]-a[2])*(c[0]-a[0]);
        emit(if cr < 0.0 { [a,b,c] } else { [a,c,b] });
    }
    !indices.is_empty()
}

/// Pier and dock floors for clients and dedicated servers without a render scene.
pub fn harbour_floors(root: &std::path::Path, elevation: &eo_data::ElevationGrid) -> Result<Vec<[[f32;3];3]>> {
    let boundary = eo_water::load_boundary(root)?;
    let shore = eo_water::shoreline::Oracle::citywide(&boundary).ok_or_else(|| anyhow::anyhow!("missing city coastline"))?;
    let js = crate::LandmarksJs::get();
    let mut rings = load_pier_rings(root)?;
    let line_rings = load_pier_line_rings(root, &rings)?;
    rings.extend(line_rings);
    let piers = plan_piers(&rings, &shore, &|x,z| elevation.sample(x,z), js.sea_level_y, js.shore.road_lift);
    let mut triangles = Vec::new();
    for p in &piers { pier_top(p, |t| triangles.push(t.map(|v| v.map(|f| f as f32)))); }
    let lines = crate::floating_docks::load_pier_lines(root)?;
    let docks = crate::floating_docks::plan_docks(&lines, &shore, &piers);
    triangles.extend(crate::floating_docks::walkable(&docks, js.sea_level_y));
    Ok(triangles)
}

/// A pier deck's edge on the segment a-b (`n` into the deck): the fascia beam,
/// its soffit back to the under-deck face, that dark face down into the
/// water, and a pile row under the soffit.
#[allow(clippy::too_many_arguments)]
fn pier_fascia(conc: &mut MeshArrays, under: &mut MeshArrays, a: [f64; 2], b: [f64; 2], n: [f64; 2], deck: f64, sea: f64, concrete: Rgb, dark: Rgb, piles: &mut usize, mut feet: Option<&mut PileFeet>) {
    let l = (b[0] - a[0]).hypot(b[1] - a[1]);
    if l < 0.3 {
        return;
    }
    let at = |p: [f64; 2], o: f64, y: f64| [p[0] + n[0] * o, y, p[1] + n[1] * o];
    let soffit = deck - PIER_FASCIA_M;
    let foot = sea - 1.4;
    // Fascia: facing out of the deck (quad wound with the outward normal).
    let out_quad = |m: &mut MeshArrays, p: [f64; 3], q: [f64; 3], r: [f64; 3], s: [f64; 3], col: Rgb| {
        let e1 = [q[0] - p[0], q[1] - p[1], q[2] - p[2]];
        let e2 = [s[0] - p[0], s[1] - p[1], s[2] - p[2]];
        let nn = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
        if nn[0] * -n[0] + nn[2] * -n[1] >= 0.0 { m.quad(p, q, r, s, col) } else { m.quad(s, r, q, p, col) }
    };
    out_quad(conc, at(a, 0.0, deck), at(b, 0.0, deck), at(b, 0.0, soffit), at(a, 0.0, soffit), concrete);
    // Soffit (facing down) and the under-deck face (facing out), set back.
    under.quad(at(a, 0.0, soffit), at(b, 0.0, soffit), at(b, PIER_VOID_SETBACK_M, soffit), at(a, PIER_VOID_SETBACK_M, soffit), dark);
    out_quad(under, at(a, PIER_VOID_SETBACK_M, soffit), at(b, PIER_VOID_SETBACK_M, soffit), at(b, PIER_VOID_SETBACK_M, foot), at(a, PIER_VOID_SETBACK_M, foot), dark);
    // Piles under the soffit, half-way to the face.
    let count = (l / PIER_PILE_PITCH_M).floor().max(1.0) as usize;
    let h = PIER_PILE_SECTION_M / 2.0;
    let u = [(b[0] - a[0]) / l, (b[1] - a[1]) / l];
    for k in 0..count {
        let s = (k as f64 + 0.5) * l / count as f64;
        let c = [a[0] + u[0] * s + n[0] * (PIER_VOID_SETBACK_M * 0.5), a[1] + u[1] * s + n[1] * (PIER_VOID_SETBACK_M * 0.5)];
        let yaw = u[1].atan2(u[0]);
        let start = conc.positions.len();
        conc.box_(c[0], c[1], foot, soffit, h, h, yaw, concrete, crate::geometry::Faces { top: false, bottom: false });
        if let Some(row) = feet.as_deref_mut() {
            row.boxes.push(start..conc.positions.len());
        }
        *piles += 1;
    }
}

/// Maritime pier-edge hardware along a water-facing pier segment `a..b` (`n` inward):
/// weathered concrete bull-rail curb (all piers), creosoted timber fender pilings &
/// horizontal timber rub-rails (Embarcadero), cast-iron double-bitt mooring bollards,
/// and perimeter railings + historic maritime lamp stanchions on public promenade piers (Pier 14 / Ferry Plaza).
#[allow(clippy::too_many_arguments)]
fn pier_edge_hardware(
    conc: &mut MeshArrays,
    a: [f64; 2],
    b: [f64; 2],
    n: [f64; 2],
    deck: f64,
    sea: f64,
    concrete: Rgb,
    fenders: bool,
    bollards: bool,
    promenade: bool,
) {
    let l = (b[0] - a[0]).hypot(b[1] - a[1]);
    if l < 1.2 {
        return;
    }
    let u = [(b[0] - a[0]) / l, (b[1] - a[1]) / l];
    let yaw = u[1].atan2(u[0]);
    let mid = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
    let pt = |s: f64, inward: f64| [a[0] + u[0] * s + n[0] * inward, a[1] + u[1] * s + n[1] * inward];

    // Subtle deterministic weathered tone variation per edge so pier curbs never look uniform CAD grey.
    let h = ((mid[0] * 0.13 + mid[1] * 0.19).sin() * 43758.5453).fract().abs();
    let bull_c = [
        (concrete[0] * (1.14 + 0.10 * h)).min(0.62),
        (concrete[1] * (1.12 + 0.10 * h)).min(0.60),
        (concrete[2] * (1.06 + 0.08 * h)).min(0.55),
    ];

    // 1. Weathered concrete bull-rail / perimeter curb along the deck edge (breaks up flat black pier slabs).
    let bull_w = 0.36;
    let bull_h = 0.24;
    let bc = pt(l * 0.5, bull_w * 0.5);
    conc.box_(bc[0], bc[1], deck - 0.02, deck + bull_h, l * 0.5, bull_w * 0.5, yaw, bull_c, crate::geometry::NO_BOTTOM);

    // 2. Embarcadero creosoted timber fender pilings & horizontal timber waler rub-rails along the seaward face.
    if fenders && l >= 2.4 {
        let timber_top = crate::geometry::srgb_hex(0x4b3e32);
        let timber_wet = crate::geometry::srgb_hex(0x232922);
        let waler_c = crate::geometry::srgb_hex(0x524539);
        // Two horizontal timber rub-rails bolted across the seaward face of the pier fascia.
        for wy in [deck - 0.24, deck - 0.66] {
            let wc = pt(l * 0.5, -0.14);
            conc.box_(wc[0], wc[1], wy - 0.10, wy + 0.10, l * 0.5, 0.09, yaw, waler_c, crate::geometry::ALL);
        }
        // Creosoted timber fender pilings standing proud of the concrete fascia (wet algae band at waterline).
        let pitch = 7.4;
        let count = (l / pitch).floor().max(1.0) as usize;
        let wet_top = (sea + 0.72).min(deck - 0.35);
        for k in 0..count {
            let s = (k as f64 + 0.5) * l / count as f64;
            let fc = pt(s, -0.26);
            // Slight organic piling top height variation (typical historic SF timber fender clusters).
            let ph = 0.28 + 0.18 * (((s * 0.73 + mid[0] * 0.11).sin()).abs());
            conc.box_(fc[0], fc[1], sea - 0.85, wet_top, 0.18, 0.18, yaw, timber_wet, crate::geometry::Faces { top: false, bottom: false });
            conc.box_(fc[0], fc[1], wet_top, deck + ph, 0.18, 0.18, yaw, timber_top, crate::geometry::NO_BOTTOM);
        }
    }

    // 3. Heavy cast-iron mooring bollards seated on the bull-rail curb.
    if bollards && l >= 12.0 {
        let iron = crate::geometry::srgb_hex(0x2b3035);
        let iron_top = crate::geometry::srgb_hex(0x3a4046);
        let pitch = 24.0;
        let count = (l / pitch).floor().max(1.0) as usize;
        for k in 0..count {
            let s = (k as f64 + 0.5) * l / count as f64;
            let pc = pt(s, 0.22);
            let y0 = deck + bull_h;
            // Base plate + bollard barrel + overhanging mooring horn cap.
            conc.box_(pc[0], pc[1], y0, y0 + 0.07, 0.34, 0.20, yaw, iron, crate::geometry::NO_BOTTOM);
            conc.box_(pc[0], pc[1], y0 + 0.07, y0 + 0.52, 0.14, 0.14, yaw, iron, crate::geometry::NO_BOTTOM);
            conc.box_(pc[0], pc[1], y0 + 0.50, y0 + 0.62, 0.24, 0.17, yaw, iron_top, crate::geometry::NO_BOTTOM);
        }
    }

    // 4. Public promenade piers (Pier 14 & Ferry Plaza outer walkways): maritime safety railing + historic lamp stanchions.
    if promenade && l >= 4.0 {
        let rail_iron = crate::geometry::srgb_hex(0x2e3736);
        let lantern_glow = crate::geometry::srgb_hex(0xd8ccb4);
        let y_rail0 = deck + bull_h;
        let y_rail1 = y_rail0 + 1.02;
        // Top handrail + two intermediate maritime cable/pipe rails.
        let rc = pt(l * 0.5, 0.18);
        conc.box_(rc[0], rc[1], y_rail1 - 0.05, y_rail1 + 0.02, l * 0.5, 0.045, yaw, rail_iron, crate::geometry::ALL);
        for ry in [y_rail0 + 0.34, y_rail0 + 0.68] {
            conc.box_(rc[0], rc[1], ry - 0.022, ry + 0.022, l * 0.5, 0.028, yaw, rail_iron, crate::geometry::ALL);
        }
        // Railing posts every 3.4 m along the bull-rail.
        let posts = (l / 3.4).ceil().max(2.0) as usize;
        for k in 0..=posts {
            let s = (k as f64 / posts as f64) * l;
            let pc = pt(s, 0.18);
            conc.box_(pc[0], pc[1], y_rail0, y_rail1, 0.04, 0.04, yaw, rail_iron, crate::geometry::NO_BOTTOM);
        }
        // Historic SF Embarcadero fluted maritime lamp stanchions every ~26 m on long promenade piers (Pier 14).
        if l >= 18.0 {
            let lamps = (l / 26.0).floor().max(1.0) as usize;
            for k in 0..lamps {
                let s = (k as f64 + 0.5) * l / lamps as f64;
                let lc = pt(s, 0.24);
                let y0 = deck + bull_h;
                // Stepped cast-iron pedestal + fluted column + cross-arm + frosted maritime globe lantern.
                conc.box_(lc[0], lc[1], y0, y0 + 0.72, 0.19, 0.19, yaw, rail_iron, crate::geometry::NO_BOTTOM);
                conc.box_(lc[0], lc[1], y0 + 0.72, y0 + 3.85, 0.075, 0.075, yaw, rail_iron, crate::geometry::NO_BOTTOM);
                conc.box_(lc[0], lc[1], y0 + 3.45, y0 + 3.55, 0.34, 0.05, yaw, rail_iron, crate::geometry::ALL);
                conc.box_(lc[0], lc[1], y0 + 3.85, y0 + 4.30, 0.17, 0.17, yaw, lantern_glow, crate::geometry::ALL);
                conc.box_(lc[0], lc[1], y0 + 4.30, y0 + 4.44, 0.11, 0.11, yaw, rail_iron, crate::geometry::NO_BOTTOM);
            }
        }
    }
}

#[cfg(test)]
mod pile_tests {
    use super::*;

    #[test]
    fn only_the_pile_feet_follow_the_deepest_corner_of_the_bed() {
        let mut concrete = MeshArrays::new("concrete");
        concrete.box_(1.0, 0.0, -1.95, 1.2, 0.23, 0.23, 0.4, [0.4; 3], crate::geometry::NO_BOTTOM);
        let end = concrete.positions.len();
        concrete.box_(4.0, 0.0, -1.95, 1.2, 0.23, 0.23, 0.4, [0.4; 3], crate::geometry::NO_BOTTOM);
        let mut mesh = concrete.freeze();
        let reference = mesh.clone();
        let feet = PileFeet { boxes: vec![0..end] };
        let floor = |x: f32, z: f32| -3.0 - 0.6 * x + 0.3 * z;
        feet.seat(&mut mesh, &mut |x, z| floor(x, z));
        let mut bases = Vec::new();
        for (p, old) in mesh.positions[..end].chunks_exact(3).zip(reference.positions.chunks_exact(3)) {
            assert_eq!((p[0], p[2]), (old[0], old[2]));
            if old[1] < 0.0 {
                assert!(p[1] < floor(p[0], p[2]));
                bases.push(p[1]);
            } else {
                assert_eq!(p[1], old[1]);
            }
        }
        assert_eq!(bases.len(), 8);
        assert!(bases.iter().all(|base| *base == bases[0]));
        assert_eq!(mesh.min_y, f64::from(bases[0]));
        assert_eq!(&mesh.positions[end..], &reference.positions[end..]);
        assert_eq!(mesh.indices, reference.indices);
        assert_eq!(mesh.normals, reference.normals);
    }
}

/// The guard rail along the Embarcadero's quay wall: the promenade's edge
/// over the bay within `RAILING_CORRIDOR_M` of the Ferry Building, on the
/// wall's coping. A picket guard of painted steel (`--no-quay-railing`: none).
///
/// * Height: 42 in (1067 mm) to the top of the rail, the guard height of the
///   California Building Code 2022 section 1015.3.
/// * Openings: no 4 in (102 mm) sphere passes (CBC 1015.4): pickets of
///   `RAILING_PICKET_M` square at `RAILING_PICKET_PITCH_M` leave 99 mm.
/// * INFERRED (no survey of the rail): the post pitch, the sections, the
///   bottom rail's height, its place on the coping.
pub const RAILING_HEIGHT_M: f64 = 1.067;
pub const RAILING_PICKET_M: f64 = 0.016;
pub const RAILING_PICKET_PITCH_M: f64 = 0.115;
pub const RAILING_POST_PITCH_M: f64 = 1.83;
pub const RAILING_POST_M: f64 = 0.06;
pub const RAILING_TOP_RAIL_M: [f64; 2] = [0.064, 0.05];
pub const RAILING_BOTTOM_RAIL_Y_M: f64 = 0.1;
/// The rail's line, landward of the wall's sea face (half the coping), m.
pub const RAILING_SET_BACK_M: f64 = 0.21;
/// The corridor (surveyed metres from the Ferry Building's tower, before the plan scale).
pub const RAILING_CORRIDOR_M: f64 = 900.0;
/// The kit finish the rail is drawn in (kit-material-specs.json: a mid-grey painted steel).
pub const RAILING_KIT: &str = "painted-steel-mid";

/// The corridor quay's lift over the quay's own top (`crest_ground`) with the
/// guard's second pass: just over the drawn terrain carpet (eo-street's
/// `openGroundLiftM`, 0.11 m, plus a 2 cm margin) so the apron is the walk
/// and no carpet shows through it. The JS crest (its road lift, 0.45 m, and
/// 0.04 m over it) stood the apron 0.5 m over the promenade's carpet beside
/// it: a raised shelf with a trench behind its kerb skirt, and the guard read
/// half a metre too tall from the walk. Blended back to the JS crest over
/// `QUAY_LIFT_BLEND_M` at the corridor's edge.
pub const QUAY_APRON_LIFT_M: f64 = 0.13;
pub const QUAY_LIFT_BLEND_M: f64 = 60.0;

/// The quay's lift at `d` from the corridor's centre (corridor radius `reach`; `js_lift` outside it).
pub fn quay_lift(d: f64, reach: f64, js_lift: f64) -> f64 {
    let t = ((d - (reach - QUAY_LIFT_BLEND_M)) / QUAY_LIFT_BLEND_M).clamp(0.0, 1.0);
    let t = t * t * (3.0 - 2.0 * t);
    QUAY_APRON_LIFT_M + (js_lift - QUAY_APRON_LIFT_M) * t
}

/// The guard's second pass (`features::guard_detail`), INFERRED (no survey of
/// the rail): a kick plate along its foot on the coping (its bottom rail
/// seated on it), a base plate under each post, the posts' feet darkened
/// where the coping's grime and the sky's occlusion meet them, and the
/// pickets as wire members (eo-render's wire pass keeps a picket where the
/// view ray passes within its radius or half a few pixels of its axis, faded
/// by the share it truly covers: far pickets fade to their coverage instead
/// of aliasing into moire), each drawn inside `PICKET_ENVELOPE_M` of its axis.
pub const KICK_PLATE_M: [f64; 2] = [0.1, 0.01];
pub const BASE_PLATE_M: [f64; 2] = [0.15, 0.015];
pub const POST_FOOT_TONE: f64 = 0.62;
pub const POST_FOOT_M: f64 = 0.3;
pub const PICKET_ENVELOPE_M: f64 = 0.05;

/// The guard: its rails, posts and (first pass) pickets, and with the second
/// pass its pickets as a wire record (else empty), and the pieces built.
pub struct Railing {
    pub solid: Frozen,
    pub pickets: Frozen,
    pub pieces: usize,
}

/// The guard on every walled (non-pier) shore edge piece whose middle is
/// within `reach` of `centre` (drawn plan), standing on `crest_at`.
pub fn quay_railing(edges: &[ShoreEdge], crest_at: &dyn Fn(f64, f64) -> f64, centre: [f64; 2], reach: f64, colour: Rgb) -> (Frozen, usize) {
    let r = quay_railing_with(edges, crest_at, centre, reach, colour, false);
    (r.solid, r.pieces)
}

/// `quay_railing`, with the second pass when `detail`.
pub fn quay_railing_with(edges: &[ShoreEdge], crest_at: &dyn Fn(f64, f64) -> f64, centre: [f64; 2], reach: f64, colour: Rgb, detail: bool) -> Railing {
    let mut m = MeshArrays::new("waterfront:railing");
    let mut wires = MeshArrays::wire("waterfront:railing:pickets", PICKET_ENVELOPE_M);
    let mut pieces = 0usize;
    let face = |m: &mut MeshArrays, q: [[f64; 3]; 4], want: [f64; 3]| {
        let e1 = [q[1][0] - q[0][0], q[1][1] - q[0][1], q[1][2] - q[0][2]];
        let e2 = [q[3][0] - q[0][0], q[3][1] - q[0][1], q[3][2] - q[0][2]];
        let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
        if n[0] * want[0] + n[1] * want[1] + n[2] * want[2] >= 0.0 { m.quad(q[0], q[1], q[2], q[3], colour) } else { m.quad(q[3], q[2], q[1], q[0], colour) }
    };
    // A bar from a to b (plan points, heights ya..yb at its foot) of section w x h, its sides facing out.
    let bar = |m: &mut MeshArrays, a: [f64; 2], b: [f64; 2], ya: f64, yb: f64, w: f64, h: f64, ends: bool| {
        let l = (b[0] - a[0]).hypot(b[1] - a[1]);
        if l < 1e-6 {
            return;
        }
        let (t, n) = ([(b[0] - a[0]) / l, (b[1] - a[1]) / l], [-(b[1] - a[1]) / l, (b[0] - a[0]) / l]);
        let p = |s: [f64; 2], y: f64, o: f64, k: f64| [s[0] + n[0] * o, y + k, s[1] + n[1] * o];
        let (o0, o1) = (-w / 2.0, w / 2.0);
        face(m, [p(a, ya, o1, 0.0), p(b, yb, o1, 0.0), p(b, yb, o1, h), p(a, ya, o1, h)], [n[0], 0.0, n[1]]);
        face(m, [p(a, ya, o0, 0.0), p(b, yb, o0, 0.0), p(b, yb, o0, h), p(a, ya, o0, h)], [-n[0], 0.0, -n[1]]);
        face(m, [p(a, ya, o0, h), p(b, yb, o0, h), p(b, yb, o1, h), p(a, ya, o1, h)], [0.0, 1.0, 0.0]);
        face(m, [p(a, ya, o0, 0.0), p(b, yb, o0, 0.0), p(b, yb, o1, 0.0), p(a, ya, o1, 0.0)], [0.0, -1.0, 0.0]);
        if ends {
            face(m, [p(a, ya, o0, 0.0), p(a, ya, o1, 0.0), p(a, ya, o1, h), p(a, ya, o0, h)], [-t[0], 0.0, -t[1]]);
            face(m, [p(b, yb, o0, 0.0), p(b, yb, o1, 0.0), p(b, yb, o1, h), p(b, yb, o0, h)], [t[0], 0.0, t[1]]);
        }
    };
    // A vertical member of square section s at plan point c from y0 to y1, its four sides.
    let post = |m: &mut MeshArrays, c: [f64; 2], u: [f64; 2], y0: f64, y1: f64, s: f64| {
        let v = [-u[1], u[0]];
        let h = s / 2.0;
        let corner = |a: f64, b: f64, y: f64| [c[0] + u[0] * a + v[0] * b, y, c[1] + u[1] * a + v[1] * b];
        for (da, db, want) in [(1.0, 0.0, [u[0], 0.0, u[1]]), (-1.0, 0.0, [-u[0], 0.0, -u[1]]), (0.0, 1.0, [v[0], 0.0, v[1]]), (0.0, -1.0, [-v[0], 0.0, -v[1]])] {
            let (a0, b0, a1, b1) = if da != 0.0 { (da * h, -h, da * h, h) } else { (-h, db * h, h, db * h) };
            face(m, [corner(a0, b0, y0), corner(a1, b1, y0), corner(a1, b1, y1), corner(a0, b0, y1)], want);
        }
    };
    for e in edges.iter().filter(|e| e.pier.is_none()) {
        let mid = [(e.a[0] + e.b[0]) / 2.0, (e.a[1] + e.b[1]) / 2.0];
        if (mid[0] - centre[0]).hypot(mid[1] - centre[1]) > reach {
            continue;
        }
        let on = |p: [f64; 2]| [p[0] + e.land[0] * RAILING_SET_BACK_M, p[1] + e.land[1] * RAILING_SET_BACK_M];
        let (a, b) = (on(e.a), on(e.b));
        let l = (b[0] - a[0]).hypot(b[1] - a[1]);
        if l < 0.2 {
            continue;
        }
        let u = [(b[0] - a[0]) / l, (b[1] - a[1]) / l];
        let (ya, yb) = (crest_at(e.a[0], e.a[1]), crest_at(e.b[0], e.b[1]));
        if ya.min(yb) < eo_water::SEA_LEVEL_Y + SUNKEN_LAND_CEILING_M {
            continue;
        }
        pieces += 1;
        let at = |t: f64| ([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t], ya + (yb - ya) * t);
        let (tw, th) = (RAILING_TOP_RAIL_M[0], RAILING_TOP_RAIL_M[1]);
        let from = m.positions.len() / 3;
        bar(&mut m, a, b, ya + RAILING_HEIGHT_M - th, yb + RAILING_HEIGHT_M - th, tw, th, false);
        bar(&mut m, a, b, ya + RAILING_BOTTOM_RAIL_Y_M, yb + RAILING_BOTTOM_RAIL_Y_M, 0.04, 0.04, false);
        if detail {
            // the kick plate seated on the coping, the bottom rail on it
            bar(&mut m, a, b, ya, yb, KICK_PLATE_M[1], RAILING_BOTTOM_RAIL_Y_M, true);
        }
        let posts = (l / RAILING_POST_PITCH_M).ceil().max(1.0) as usize;
        for k in 0..posts {
            let (c, y) = at(k as f64 / posts as f64);
            post(&mut m, c, u, y - 0.05, y + RAILING_HEIGHT_M - th, RAILING_POST_M);
            if detail {
                let c1 = [c[0] + u[0] * BASE_PLATE_M[0] / 2.0, c[1] + u[1] * BASE_PLATE_M[0] / 2.0];
                let c0 = [c[0] - u[0] * BASE_PLATE_M[0] / 2.0, c[1] - u[1] * BASE_PLATE_M[0] / 2.0];
                bar(&mut m, c0, c1, y, y, BASE_PLATE_M[0], BASE_PLATE_M[1], true);
            }
        }
        if detail {
            // the feet darker: the coping's grime and the sky the plates and posts lose there
            let base = ya.min(yb);
            for v in from..m.positions.len() / 3 {
                let t = ((m.positions[3 * v + 1] - base) / POST_FOOT_M).clamp(0.0, 1.0);
                let k = POST_FOOT_TONE + (1.0 - POST_FOOT_TONE) * t * t * (3.0 - 2.0 * t);
                for ch in 0..3 {
                    m.colors[3 * v + ch] *= k;
                }
            }
        }
        let pickets = (l / RAILING_PICKET_PITCH_M).floor() as usize;
        for k in 0..pickets {
            let (c, y) = at((k as f64 + 0.5) / pickets as f64);
            if detail {
                wires.wire_segment([c[0], y + RAILING_BOTTOM_RAIL_Y_M + 0.04, c[1]], [c[0], y + RAILING_HEIGHT_M - th, c[1]], RAILING_PICKET_M, colour);
            } else {
                post(&mut m, c, u, y + RAILING_BOTTOM_RAIL_Y_M + 0.04, y + RAILING_HEIGHT_M - th, RAILING_PICKET_M);
            }
        }
    }
    Railing { solid: m.freeze(), pickets: wires.freeze(), pieces }
}

#[cfg(test)]
mod pier_line_tests {
    use super::*;

    #[test]
    fn pier_lines_close_into_rings() {
        // an outline drawn as one open way, and a walkway as its two edges sharing their ends
        let outline = vec![[0.0, 0.0], [0.0, -100.0], [60.0, -100.0], [60.0, -90.0], [6.0, -90.0], [6.0, -2.0]];
        let left = vec![[100.0, 0.0], [100.0, -60.0], [103.0, -60.0]];
        let right = vec![[103.0, -60.0], [103.0, 0.0], [100.5, 0.0]];
        let rings = close_pier_lines(vec![outline, left, right]);
        assert_eq!(rings.len(), 2, "{rings:?}");
        let mut areas: Vec<f64> = rings.iter().map(|r| signed_area(r).abs()).collect();
        areas.sort_by(|a, b| a.total_cmp(b));
        assert!((areas[0] - 180.0).abs() < 20.0, "{areas:?}");
        // a short open stub is no pier
        assert!(close_pier_lines(vec![vec![[0.0, 0.0], [3.0, 0.0]]]).is_empty());
        assert!((PIER_GUARD_M - 42.0 * 0.0254).abs() < 1e-3);
    }

    #[test]
    fn sunken_non_pier_shelf_is_pressed_to_seabed_while_dry_quay_stays() {
        let sea = -0.55;
        let edge = ShoreEdge { a: [0.0, 0.0], b: [60.0, 0.0], land: [0.0, 1.0], pier: None };
        let press = ShorePress::new(sea, &[edge], &[]);
        // Artificially lifted underwater ghost pier shelf (sea + 0.10 m) at 16 m inland -> pressed to seabed.
        assert!(press.press(30.0, 16.0, sea + 0.10) <= sea - 2.0 + 1e-9);
        // Real dry quay land (+2.0 m) at 16 m inland -> untouched.
        assert!((press.press(30.0, 16.0, 2.0) - 2.0).abs() < 1e-9);
        // Beyond 30 m inland -> untouched even at sea + 0.10 m.
        assert!((press.press(30.0, 30.0, sea + 0.10) - (sea + 0.10)).abs() < 1e-9);
    }

    #[test]
    fn ferry_gate_sheds_build_dark_steel_canopies_with_neutral_glass() {
        let js = crate::LandmarksJs::get();
        let fp = &js.shore.ferry_pier;
        let g = ferry_gate_sheds(&fp.outline, &fp.sheds, 2.0).expect("ferry gate sheds");
        assert!(g.frames.triangles() > 400 && g.frames.triangles() < 8_000, "frames={}", g.frames.triangles());
        assert!(g.roofs.triangles() > 100 && g.roofs.triangles() < 4_000, "roofs={}", g.roofs.triangles());
        assert!(g.glass.triangles() > 20 && g.glass.triangles() < 1_000, "glass={}", g.glass.triangles());
        // No blinding pure-white vertex tints on frames or roofs, and neutral dark smoked glazing.
        assert!(g.frames.colors.iter().all(|&c| c <= 0.50), "bright white frame tint found");
        assert!(g.roofs.colors.iter().all(|&c| c <= 0.40), "bright white roof tint found");
        assert!(g.glass.colors.iter().all(|&c| c <= 0.025), "bright blue glass tint found");
    }
}
