//! The harbours' floating docks: the OSM `man_made=pier` ways mapped as
//! lines (`pierLines` of public/data/sf/sf-map-citywide.json.gz, ODbL, the
//! earlier map build), which only the 2D map read. Hyde Street Harbor's main
//! walks and finger slips, the walk out from the quay, the rowing clubs'
//! piers off Aquatic Park beach and the Fisherman's Wharf lagoon's docks are
//! all such lines; the piers the waterfront already decks (`waterfront::Pier`,
//! the `piers` rings) are left to it.
//!
//! Each line over open water becomes a float on the still water: a deck
//! strip at `FREEBOARD_M`, timber walers along its sides, the float's body
//! down past the waterline, and guide piles beside it.
//!
//! Sources: main walkway width, California Department of Boating and
//! Waterways, *Layout and Design Guidelines for Marina Berthing Facilities*
//! (2005), Table C-2 (6 ft for a main walkway under 300 ft), and its 700 ft
//! practical longest main walkway (a longer mapped line is left out). Finger width:
//! the same guidelines' finger table runs 3 to 8 ft by berth length; the
//! narrow end is used (the harbour's berths are 30 to 45 ft). INFERRED (no
//! survey): the freeboard, the float's draft, the waler, the guide piles'
//! pitch, section and height.

use crate::geometry::{Frozen, MeshArrays, Rgb, srgb_hex};
use crate::waterfront::Pier;
use anyhow::Result;

/// Main walkway width (DBW 2005 Table C-2, under 300 ft: 6 ft). Metres.
pub const MAIN_WALK_W_M: f64 = 1.83;
/// Finger float width (DBW 2005, the finger table's narrow end: 3 ft). Metres.
pub const FINGER_W_M: f64 = 0.91;
/// A mapped line this long or longer is a main walk; a shorter one a finger. Metres.
pub const MAIN_MIN_M: f64 = 20.0;
/// A main walk's practical longest (DBW 2005: 700 ft): a longer mapped line
/// is a fixed pier, a wharf or a breakwater, not a float. Metres.
pub const MAIN_MAX_M: f64 = 213.0;
/// The deck over the still water. Metres. INFERRED.
pub const FREEBOARD_M: f64 = 0.45;
/// The float's body under the still water. Metres. INFERRED.
pub const FLOAT_DRAFT_M: f64 = 0.35;
/// The timber waler along the deck's edge: its depth. Metres. INFERRED.
pub const WALER_M: f64 = 0.22;
/// A finger's deck stands this far under the main walk's, where they meet. Metres.
pub const FINGER_DROP_M: f64 = 0.02;
/// Guide piles: pitch along a main walk, radius, top over the deck, foot under the sea. Metres. INFERRED.
pub const PILE_PITCH_M: f64 = 18.0;
pub const PILE_R_M: f64 = 0.2;
pub const PILE_TOP_M: f64 = 3.2;
pub const PILE_FOOT_M: f64 = 3.0;
/// A line is a float when at least this share of it lies over open water.
pub const WATER_SHARE: f64 = 0.6;
/// A line is the waterfront's pier (already decked) when this share of it
/// lies inside or within `ON_PIER_M` of a pier ring.
pub const ON_PIER_SHARE: f64 = 0.3;
pub const ON_PIER_M: f64 = 4.0;

/// Weathered tones (sRGB): the deck's worn concrete, the timber walers, the
/// float body at the waterline, the creosoted piles. INFERRED (the decks' light
/// grey checked by eye against the USDA NAIP 0.6 m aerial, public domain; matte and aged).
pub const DECK_SRGB: u32 = 0x868683;
pub const WALER_SRGB: u32 = 0x54483b;
pub const FLOAT_SRGB: u32 = 0x343833;
pub const PILE_SRGB: u32 = 0x3b3631;

/// One float to build: its centreline, width and kind.
#[derive(Clone, Debug)]
pub struct Dock {
    pub line: Vec<[f64; 2]>,
    pub width: f64,
    pub main: bool,
}

/// The meshes and counts of the built docks.
#[derive(Default)]
pub struct DockMeshes {
    pub mesh: Frozen,
    pub docks: usize,
    pub piles: usize,
    pub length_m: f64,
}

/// The citywide map's pier lines (`pierLines`: `len` vertices per feature,
/// `xy` feature-delta encoded, times `meta.quantum`), in world metres at the
/// scaled ground plan.
pub fn load_pier_lines(repo_root: &std::path::Path) -> Result<Vec<Vec<[f64; 2]>>> {
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
        #[serde(rename = "pierLines")]
        pier_lines: Layer,
    }
    let bytes = std::fs::read(repo_root.join("public/data/sf/sf-map-citywide.json.gz"))?;
    let mut s = String::new();
    flate2::read::GzDecoder::new(&bytes[..]).read_to_string(&mut s)?;
    let m: Map = serde_json::from_str(&s)?;
    let q = m.meta.quantum * eo_data::groundscale::scale();
    let l = &m.pier_lines;
    let mut out = Vec::with_capacity(l.len.len());
    let mut k = 0usize;
    for &n in &l.len {
        if n == 0 || k + 2 * n > l.xy.len() {
            break;
        }
        let (mut x, mut z) = (l.xy[k], l.xy[k + 1]);
        let mut line = vec![[x as f64 * q, z as f64 * q]];
        for v in 1..n {
            x += l.xy[k + 2 * v];
            z += l.xy[k + 2 * v + 1];
            line.push([x as f64 * q, z as f64 * q]);
        }
        k += 2 * n;
        if line.len() >= 2 {
            out.push(line);
        }
    }
    Ok(out)
}

/// The distance from p to an open polyline.
fn line_distance(line: &[[f64; 2]], p: [f64; 2]) -> f64 {
    line.windows(2)
        .map(|w| {
            let (a, b) = (w[0], w[1]);
            let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
            let l2 = dx * dx + dz * dz;
            let t = if l2 > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / l2).clamp(0.0, 1.0) } else { 0.0 };
            (p[0] - a[0] - dx * t).hypot(p[1] - a[1] - dz * t)
        })
        .fold(f64::INFINITY, f64::min)
}

/// A polyline's bounding box: [min x, min z, max x, max z].
fn bounds(line: &[[f64; 2]]) -> [f64; 4] {
    line.iter().fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])])
}

/// The distance from p to a bounding box (0 inside): never more than the distance to the line in it.
fn box_distance(b: [f64; 4], p: [f64; 2]) -> f64 {
    let dx = (b[0] - p[0]).max(p[0] - b[2]).max(0.0);
    let dz = (b[1] - p[1]).max(p[1] - b[3]).max(0.0);
    dx.hypot(dz)
}

fn length(line: &[[f64; 2]]) -> f64 {
    line.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum()
}

/// Points every `step` metres along a line (both ends included).
fn samples(line: &[[f64; 2]], step: f64) -> Vec<[f64; 2]> {
    let mut out = vec![line[0]];
    for w in line.windows(2) {
        let l = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
        let n = (l / step).ceil().max(1.0) as usize;
        for k in 1..=n {
            let t = k as f64 / n as f64;
            out.push([w[0][0] + (w[1][0] - w[0][0]) * t, w[0][1] + (w[1][1] - w[0][1]) * t]);
        }
    }
    out
}

/// The floats to build: every pier line mostly over open water and not on
/// a decked pier, cut to its runs over the water (a run must be over a metre),
/// a main walk or a finger by its mapped length.
pub fn plan_docks(lines: &[Vec<[f64; 2]>], oracle: &eo_water::shoreline::Oracle, piers: &[Pier]) -> Vec<Dock> {
    let on_pier = |x: f64, z: f64| piers.iter().any(|p| p.near(x, z, ON_PIER_M));
    let mut out = Vec::new();
    for line in lines {
        let len = length(line);
        if !(2.0..=MAIN_MAX_M).contains(&len) {
            continue;
        }
        let pts = samples(line, 1.0);
        let n = pts.len() as f64;
        let water = pts.iter().filter(|p| oracle.is_water(p[0], p[1])).count() as f64;
        let pier = pts.iter().filter(|p| on_pier(p[0], p[1])).count() as f64;
        if water / n < WATER_SHARE || pier / n > ON_PIER_SHARE {
            continue;
        }
        let main = len >= MAIN_MIN_M;
        let width = if main { MAIN_WALK_W_M } else { FINGER_W_M };
        // Runs of consecutive samples over open water, off any pier.
        let mut run: Vec<[f64; 2]> = Vec::new();
        let mut flush = |run: &mut Vec<[f64; 2]>| {
            if run.len() >= 2 && length(run) > 1.0 {
                out.push(Dock { line: simplify(run), width, main });
            }
            run.clear();
        };
        for p in &pts {
            if oracle.is_water(p[0], p[1]) && !on_pier(p[0], p[1]) {
                run.push(*p);
            } else {
                flush(&mut run);
            }
        }
        flush(&mut run);
    }
    // A finger starts at the main walk it hangs off: its first point the end
    // nearer a main walk (its outer end keeps the guide pile).
    let mains: Vec<Vec<[f64; 2]>> = out.iter().filter(|d| d.main).map(|d| d.line.clone()).collect();
    let to_main = |p: [f64; 2]| mains.iter().map(|l| line_distance(l, p)).fold(f64::INFINITY, f64::min);
    for d in out.iter_mut().filter(|d| !d.main) {
        if to_main(d.line[d.line.len() - 1]) < to_main(d.line[0]) {
            d.line.reverse();
        }
    }
    out
}

/// Drop the collinear samples a run was cut from (keep the line's corners).
fn simplify(run: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut out = vec![run[0]];
    for i in 1..run.len() - 1 {
        let (a, b, c) = (out[out.len() - 1], run[i], run[i + 1]);
        let cross = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]);
        if cross.abs() > 1e-3 {
            out.push(b);
        }
    }
    out.push(run[run.len() - 1]);
    out
}

/// A quad wound to face `want`.
fn face(m: &mut MeshArrays, p: [f64; 3], q: [f64; 3], r: [f64; 3], s: [f64; 3], col: Rgb, want: [f64; 3]) {
    let e1 = [q[0] - p[0], q[1] - p[1], q[2] - p[2]];
    let e2 = [s[0] - p[0], s[1] - p[1], s[2] - p[2]];
    let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
    if n[0] * want[0] + n[1] * want[1] + n[2] * want[2] >= 0.0 { m.quad(p, q, r, s, col) } else { m.quad(s, r, q, p, col) }
}

/// The left offset (unit normal times the mitred half width) at each vertex.
fn offsets(line: &[[f64; 2]], half: f64) -> Vec<[f64; 2]> {
    let dir = |a: [f64; 2], b: [f64; 2]| {
        let l = (b[0] - a[0]).hypot(b[1] - a[1]).max(1e-9);
        [(b[0] - a[0]) / l, (b[1] - a[1]) / l]
    };
    let left = |d: [f64; 2]| [-d[1], d[0]];
    let n = line.len();
    (0..n)
        .map(|i| {
            let d0 = if i > 0 { dir(line[i - 1], line[i]) } else { dir(line[0], line[1]) };
            let d1 = if i + 1 < n { dir(line[i], line[i + 1]) } else { d0 };
            let (l0, l1) = (left(d0), left(d1));
            let m = [l0[0] + l1[0], l0[1] + l1[1]];
            let ml = m[0].hypot(m[1]);
            if ml < 1e-6 {
                return [l0[0] * half, l0[1] * half];
            }
            let m = [m[0] / ml, m[1] / ml];
            let k = half / (m[0] * l0[0] + m[1] * l0[1]).max(0.35);
            [m[0] * k, m[1] * k]
        })
        .collect()
}

fn deck_y(d: &Dock, sea: f64) -> f64 {
    sea + FREEBOARD_M - if d.main { 0.0 } else { FINGER_DROP_M }
}

fn deck_quad(d: &Dock, off: &[[f64;2]], top: f64, i: usize) -> [[f64;3];4] {
    let at = |j: usize, side: f64| [d.line[j][0]+off[j][0]*side, top, d.line[j][1]+off[j][1]*side];
    let [a,b,c,e] = [at(i,1.0),at(i+1,1.0),at(i+1,-1.0),at(i,-1.0)];
    if (b[2]-a[2])*(e[0]-a[0]) >= (b[0]-a[0])*(e[2]-a[2]) { [a,b,c,e] } else { [e,c,b,a] }
}

pub fn walkable(docks: &[Dock], sea: f64) -> Vec<[[f32;3];3]> {
    let mut out = Vec::new();
    for d in docks {
        let off = offsets(&d.line, d.width / 2.0);
        for i in 0..d.line.len().saturating_sub(1) {
            if (d.line[i][0]-d.line[i+1][0]).hypot(d.line[i][1]-d.line[i+1][1]) < 1e-6 { continue; }
            let [a,b,c,e] = deck_quad(d, &off, deck_y(d,sea), i).map(|v| v.map(|f| f as f32));
            out.extend([[a,b,c],[a,c,e]]);
        }
    }
    out
}

/// The floats' meshes: deck, walers and float body per run, the guide piles.
pub fn build_meshes(docks: &[Dock], sea: f64) -> DockMeshes {
    let mut m = MeshArrays::new("waterfront:floating-docks");
    let (deck_c, waler_c, float_c, pile_c) = (srgb_hex(DECK_SRGB), srgb_hex(WALER_SRGB), srgb_hex(FLOAT_SRGB), srgb_hex(PILE_SRGB));
    let mut piles = 0usize;
    let mut total = 0.0;
    for d in docks {
        let top = deck_y(d, sea);
        let (waler_y, foot) = (top - WALER_M, sea - FLOAT_DRAFT_M);
        let off = offsets(&d.line, d.width / 2.0);
        let at = |i: usize, side: f64, y: f64| [d.line[i][0] + off[i][0] * side, y, d.line[i][1] + off[i][1] * side];
        for i in 0..d.line.len() - 1 {
            let (a, b) = (d.line[i], d.line[i + 1]);
            let l = (b[0] - a[0]).hypot(b[1] - a[1]);
            if l < 1e-6 {
                continue;
            }
            total += l;
            // The deck, facing up.
            let [a_top, b_top, c_top, d_top] = deck_quad(d, &off, top, i);
            m.quad(a_top,b_top,c_top,d_top,deck_c);
            // Each side: the waler, then the float body down past the waterline, facing out, plus mooring cleats on main walks.
            let cleats = if d.main && l >= 8.0 { (l / 9.0).floor() as usize } else { 0 };
            let cleat_c = srgb_hex(0x2a2e33);
            let dir = [(b[0] - a[0]) / l, (b[1] - a[1]) / l];
            let yaw = dir[1].atan2(dir[0]);
            for side in [1.0, -1.0] {
                let n = [-(b[1] - a[1]) / l * side, 0.0, (b[0] - a[0]) / l * side];
                face(&mut m, at(i, side, top), at(i + 1, side, top), at(i + 1, side, waler_y), at(i, side, waler_y), waler_c, n);
                face(&mut m, at(i, side, waler_y), at(i + 1, side, waler_y), at(i + 1, side, foot), at(i, side, foot), float_c, n);
                for k in 0..cleats {
                    let t = (k as f64 + 0.5) / cleats as f64;
                    let cx = a[0] + (b[0] - a[0]) * t + (off[i][0] + (off[i + 1][0] - off[i][0]) * t) * side * 0.86;
                    let cz = a[1] + (b[1] - a[1]) * t + (off[i][1] + (off[i + 1][1] - off[i][1]) * t) * side * 0.86;
                    m.box_(cx, cz, top, top + 0.09, 0.16, 0.045, yaw, cleat_c, crate::geometry::NO_BOTTOM);
                }
            }
        }
        // The two ends, facing out along the line.
        for (i, j) in [(0usize, 1usize), (d.line.len() - 1, d.line.len() - 2)] {
            let (p, q) = (d.line[i], d.line[j]);
            let l = (p[0] - q[0]).hypot(p[1] - q[1]).max(1e-9);
            let n = [(p[0] - q[0]) / l, 0.0, (p[1] - q[1]) / l];
            face(&mut m, at(i, 1.0, top), at(i, -1.0, top), at(i, -1.0, waler_y), at(i, 1.0, waler_y), waler_c, n);
            face(&mut m, at(i, 1.0, waler_y), at(i, -1.0, waler_y), at(i, -1.0, foot), at(i, 1.0, foot), float_c, n);
        }
        // Guide piles: along a main walk every PILE_PITCH_M, sides alternating;
        // one at a finger's outer end.
        let pile = |m: &mut MeshArrays, c: [f64; 2]| {
            m.lathe(c[0], c[1], &[[PILE_R_M, sea - PILE_FOOT_M], [PILE_R_M, top + PILE_TOP_M], [0.0, top + PILE_TOP_M]], 8, pile_c, None, None);
        };
        let len = length(&d.line);
        let pts = samples(&d.line, 0.5);
        let at_dist = |s: f64| -> ([f64; 2], [f64; 2]) {
            let k = ((s / len) * (pts.len() - 1) as f64).round().clamp(0.0, (pts.len() - 1) as f64) as usize;
            let (a, b) = if k + 1 < pts.len() { (pts[k], pts[k + 1]) } else { (pts[k - 1], pts[k]) };
            let l = (b[0] - a[0]).hypot(b[1] - a[1]).max(1e-9);
            (pts[k], [-(b[1] - a[1]) / l, (b[0] - a[0]) / l])
        };
        let clear = d.width / 2.0 + PILE_R_M + 0.05;
        if d.main {
            let count = (len / PILE_PITCH_M).floor() as usize;
            for k in 0..count {
                let (p, n) = at_dist((k as f64 + 0.5) * len / count.max(1) as f64);
                let side = if k % 2 == 0 { 1.0 } else { -1.0 };
                pile(&mut m, [p[0] + n[0] * clear * side, p[1] + n[1] * clear * side]);
                piles += 1;
            }
        } else {
            let (p, n) = at_dist(len - 0.6);
            pile(&mut m, [p[0] + n[0] * clear, p[1] + n[1] * clear]);
            piles += 1;
        }
    }
    DockMeshes { mesh: m.freeze(), docks: docks.len(), piles, length_m: total }
}

/// A berth a boat can lie in: its centre on the still water, the heading of
/// its bow (radians, from +x towards +z, as `atan2(dz, dx)`), and the room it
/// has (length along the berth, beam across it).
#[derive(Clone, Copy, Debug)]
pub struct Berth {
    pub centre: [f64; 2],
    pub heading: f64,
    pub length: f64,
    pub beam: f64,
    /// Alongside a finger (true), or side-tied along a main walk.
    pub finger: bool,
}

/// The gap a boat keeps off the float it lies against (fenders). Metres. INFERRED.
pub const FENDER_GAP_M: f64 = 0.35;
/// The widest berth a finger offers (a 50 ft boat's beam). Metres. INFERRED.
pub const BERTH_BEAM_MAX_M: f64 = 5.2;
/// The share of a finger's length a boat may run past its outer end. INFERRED.
pub const FINGER_OVERHANG: f64 = 0.25;
/// Side-tied boats along a main walk: the gap between them, and how far from
/// any finger a stretch of walk must be to take one. Metres. INFERRED.
pub const SIDE_TIE_GAP_M: f64 = 2.5;
pub const SIDE_TIE_CLEAR_M: f64 = 6.0;

/// The berths along the planned docks: one each side of every finger (the
/// room across reaching half-way to the next finger or float on that side),
/// and side ties along each main walk's stretches clear of fingers. A berth's
/// footprint must lie over open water (`water`) and clear of every float.
pub fn plan_berths(docks: &[Dock], water: &dyn Fn(f64, f64) -> bool) -> Vec<Berth> {
    let mut out = Vec::new();
    // Each float's bounding box: a float whose box is farther than a test's reach is skipped
    // (the box is never farther than the line, so the result is the same, sooner).
    let boxes: Vec<[f64; 4]> = docks.iter().map(|d| bounds(&d.line)).collect();
    // Everything a berth must stay off: every float's centreline and half width.
    let clear_of_floats = |c: [f64; 2], u: [f64; 2], len: f64, beam: f64, skip: Option<usize>| -> bool {
        let v = [-u[1], u[0]];
        let corners = [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5), (0.0, 0.0), (0.5, 0.0), (-0.5, 0.0)];
        corners.iter().all(|&(a, b)| {
            let p = [c[0] + u[0] * len * a + v[0] * beam * b, c[1] + u[1] * len * a + v[1] * beam * b];
            water(p[0], p[1])
                && docks.iter().enumerate().all(|(k, d)| {
                    let r = d.width / 2.0 + 0.1;
                    Some(k) == skip || box_distance(boxes[k], p) > r || line_distance(&d.line, p) > r
                })
        })
    };
    for (i, d) in docks.iter().enumerate() {
        let len = length(&d.line);
        if !d.main {
            // A finger: one berth on each side, the bow toward the walk (its first point).
            let (a, b) = (d.line[0], d.line[d.line.len() - 1]);
            let u = [(a[0] - b[0]) / len, (a[1] - b[1]) / len]; // outer end -> walk
            let v = [-u[1], u[0]];
            for side in [1.0, -1.0] {
                // Room across: half-way to the nearest other float on this side, along the finger's middle.
                let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
                let mut room = 12.0f64;
                for (k, o) in docks.iter().enumerate() {
                    // (the search reaches 12 m from the middle)
                    if k == i || box_distance(boxes[k], mid) > 12.0 + o.width / 2.0 {
                        continue;
                    }
                    for t in 1..=24 {
                        let s = t as f64 * 0.5;
                        let p = [mid[0] + v[0] * side * s, mid[1] + v[1] * side * s];
                        if line_distance(&o.line, p) <= o.width / 2.0 {
                            room = room.min(s);
                            break;
                        }
                    }
                }
                // The water between this finger and the next float is shared by
                // the two boats lying in it (a double slip): this one takes half.
                let beam = ((room - d.width) / 2.0 - FENDER_GAP_M - 0.15).min(BERTH_BEAM_MAX_M);
                if beam < 1.0 {
                    continue;
                }
                let blen = len * (1.0 + FINGER_OVERHANG);
                let off = d.width / 2.0 + FENDER_GAP_M + beam / 2.0;
                // The berth's middle: its stern past the outer end by the overhang.
                let s_mid = len - blen / 2.0 + 0.3;
                let c = [b[0] + u[0] * (len - s_mid) + v[0] * side * off, b[1] + u[1] * (len - s_mid) + v[1] * side * off];
                if clear_of_floats(c, u, blen * 0.9, beam, Some(i)) {
                    out.push(Berth { centre: c, heading: u[1].atan2(u[0]), length: blen, beam, finger: true });
                }
            }
        } else {
            // A main walk: side ties along stretches with no finger within SIDE_TIE_CLEAR_M.
            let pts = samples(&d.line, 0.5);
            // The fingers whose box comes within the clearance of this walk's box.
            let near_walk = |k: usize| {
                let (a, b) = (boxes[k], boxes[i]);
                ((a[0] - b[2]).max(b[0] - a[2]).max(0.0)).hypot((a[1] - b[3]).max(b[1] - a[3]).max(0.0)) <= SIDE_TIE_CLEAR_M
            };
            let fingers: Vec<usize> = (0..docks.len()).filter(|&k| !docks[k].main && near_walk(k)).collect();
            let free: Vec<bool> = pts.iter().map(|p| fingers.iter().all(|&k| box_distance(boxes[k], *p) > SIDE_TIE_CLEAR_M || line_distance(&docks[k].line, *p) > SIDE_TIE_CLEAR_M)).collect();
            for side in [1.0, -1.0] {
                let mut k = 0usize;
                while k + 1 < pts.len() {
                    if !free[k] {
                        k += 1;
                        continue;
                    }
                    // A run of free samples from k.
                    let mut e = k;
                    while e + 1 < pts.len() && free[e + 1] {
                        e += 1;
                    }
                    let run = (e - k) as f64 * 0.5;
                    let slot = 12.0 + SIDE_TIE_GAP_M;
                    let n = (run / slot).floor() as usize;
                    for j in 0..n {
                        let s0 = k + ((j as f64 + 0.5) * slot / 0.5) as usize;
                        let s1 = (s0 + 1).min(pts.len() - 1);
                        let (p, q) = (pts[s0.min(pts.len() - 2)], pts[s1]);
                        let l = (q[0] - p[0]).hypot(q[1] - p[1]).max(1e-9);
                        let u = [(q[0] - p[0]) / l * side, (q[1] - p[1]) / l * side];
                        let v = [-(q[1] - p[1]) / l * side, (q[0] - p[0]) / l * side];
                        let beam = 4.2;
                        let off = d.width / 2.0 + FENDER_GAP_M + beam / 2.0;
                        let c = [p[0] + v[0] * off, p[1] + v[1] * off];
                        if clear_of_floats(c, u, 12.0, beam, Some(i)) {
                            out.push(Berth { centre: c, heading: u[1].atan2(u[0]), length: 12.0, beam, finger: false });
                        }
                    }
                    k = e + 1;
                }
            }
        }
    }
    out
}
