//! Archetype `cottage-garage` (src/world/buildings/archetypes/cottage-garage.js):
//! the cottage, the postwar minimal-traditional house and the accessory garage,
//! shed or back cottage. Spec (with its detail-level search), the LOD1 detail
//! (gable roof, porch, step) and the close-range (LOD0) detail, which uses the
//! stucco-row close helpers.
//!
//! Not ported: nothing that changes parts. `plan.lodFarFromM` is kept in
//! `plan.extra` like the other plan fields the JS hangs on the plan.

use super::stucco_row::{
    CEdge, CLOSE_KIND as CK, Curtain, Cx, DoorLeaf, PEDESTRIAN_HEADROOM_M, bead_section, close_context, close_grid, close_part_triangles, close_ring,
    door_leaf_detail, downspout, fit_close_budget, garage_door_detail, opening_lights, shingle_courses,
};
use crate::classify::Classified;
use crate::elevation::{OpeningRecord, Part, PartArgs, Plan, Shape, plan_elevation, trim_profile};
use crate::frame::{Basis, Edge, Frame, P2};
use crate::js::*;
use crate::lod::triangles_for_part;
use crate::rng::Rng;

pub const COTTAGE_GARAGE_VERSION: &str = "sf-archetype-cottage-garage-v3";
const COTTAGE: &str = "cottage";
const MINIMAL_TRADITIONAL: &str = "minimal-traditional";
const ACCESSORY: &str = "accessory";

fn triangle_budget(form: &str) -> f64 {
    match form {
        ACCESSORY => 300.0,
        _ => 600.0,
    }
}

pub const ACCESSORY_FAR_RUNG_FROM_M: f64 = 65.0;
const DETAIL_LEVELS: [i64; 4] = [3, 2, 1, 0];
const POSTWAR_FROM_YEAR: f64 = 1925.0;
const MIN_WALL_M: f64 = 2.3;
const ROOF_SLAB_M: f64 = 0.16;
const MAX_RISE_M: f64 = 3.2;
const MIN_GABLE_RISE_M: f64 = 0.55;
const MIN_ROOF_COVER: f64 = 0.4;
const MIN_ROOF_SIDE_M: f64 = 2.4;
const MAX_PITCHED_ACCESSORY_M2: f64 = 150.0;
const ROLLER_MAX_SETBACK_M: f64 = 30.0;
const FLANK_OVERHANG_M: f64 = 0.12;

fn pitch_range(form: &str) -> (f64, f64) {
    match form {
        COTTAGE => (0.72, 0.9),
        MINIMAL_TRADITIONAL => (0.32, 0.42),
        _ => (0.3, 0.5),
    }
}

fn overhang_of(form: &str) -> (f64, f64) {
    match form {
        COTTAGE => (0.35, 0.22),
        MINIMAL_TRADITIONAL => (0.4, 0.18),
        _ => (0.18, 0.1),
    }
}

const COTTAGE_WALLS: [(&str, f64); 3] = [("painted-clapboard", 0.55), ("wood-siding", 0.3), ("painted-shingle", 0.15)];
const POSTWAR_WALLS: [(&str, f64); 3] = [("painted-stucco", 0.45), ("float-finish-stucco", 0.35), ("scanned-painted-plaster", 0.2)];
const GARAGE_WALLS: [(&str, f64); 3] = [("wood-siding", 0.4), ("painted-stucco", 0.35), ("painted-clapboard", 0.25)];
const SHED_WALLS: [(&str, f64); 3] = [("wood-siding", 0.45), ("corrugated-metal", 0.3), ("painted-clapboard", 0.25)];
const STUDIO_WALLS: [(&str, f64); 3] = [("painted-clapboard", 0.45), ("painted-stucco", 0.35), ("wood-siding", 0.2)];

fn pick_weighted(roll: f64, table: &[(&'static str, f64)]) -> &'static str {
    let mut rest = roll;
    for (name, weight) in table {
        rest -= weight;
        if rest <= 0.0 {
            return name;
        }
    }
    table[0].0
}

fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    hi.min(lo.max(v))
}

/// `cottageGarageForm(classified)`: (form, accessory kind).
pub fn cottage_garage_form(c: &Classified) -> (&'static str, Option<&'static str>) {
    let variant = c.variant.as_str();
    if variant == MINIMAL_TRADITIONAL {
        return (MINIMAL_TRADITIONAL, None);
    }
    if variant == "accessory-cottage" {
        return (ACCESSORY, Some("studio"));
    }
    if variant.starts_with("accessory") {
        return (ACCESSORY, Some("garage"));
    }
    if variant == "cottage" {
        let year = if c.year_built.is_nan() { 0.0 } else { c.year_built };
        return if year >= POSTWAR_FROM_YEAR { (MINIMAL_TRADITIONAL, None) } else { (COTTAGE, None) };
    }
    let h = if c.height.is_nan() { 0.0 } else { c.height };
    if h < 4.0 { (ACCESSORY, Some("garage")) } else { (COTTAGE, None) }
}

/// `frontServesWay(frame)`.
pub fn front_serves_way(frame: &Frame) -> bool {
    let Some(front) = frame.edge_for_role("front") else { return false };
    if front.party {
        return false;
    }
    if frame.front_source != "street" && frame.front_source != "service" {
        return false;
    }
    if frame.front_occluded == Some(true) {
        return false;
    }
    let distance = frame.front_distance.unwrap_or(f64::NAN);
    !(distance > ROLLER_MAX_SETBACK_M)
}

// ---------------------------------------------------------------------------
// Roof rectangle
// ---------------------------------------------------------------------------

const SIDE_SAMPLES: [f64; 7] = [0.002, 0.17, 0.33, 0.5, 0.67, 0.83, 0.998];
const INSIDE_TOL_M: f64 = 0.05;

/// Plan rectangle; sides indexed x0, x1, z0, z1.
#[derive(Clone, Copy, Debug)]
struct Rect {
    v: [f64; 4],
}

impl Rect {
    fn x0(&self) -> f64 {
        self.v[0]
    }
    fn x1(&self) -> f64 {
        self.v[1]
    }
    fn z0(&self) -> f64 {
        self.v[2]
    }
    fn z1(&self) -> f64 {
        self.v[3]
    }
}

fn side_inside(frame: &Frame, r: &Rect, side: usize) -> bool {
    for t in SIDE_SAMPLES {
        let (px, pz) = match side {
            0 => (r.x0(), r.z0() + (r.z1() - r.z0()) * t),
            1 => (r.x1(), r.z0() + (r.z1() - r.z0()) * t),
            2 => (r.x0() + (r.x1() - r.x0()) * t, r.z0()),
            _ => (r.x0() + (r.x1() - r.x0()) * t, r.z1()),
        };
        let w = frame.to_world(px, pz);
        if frame.signed_distance(w.x, w.z) > INSIDE_TOL_M {
            return false;
        }
    }
    true
}

/// (area, i0, i1, j0, j1)
fn largest_cell_rect(cells: &[u8], nx: usize, nz: usize) -> Option<(i64, i64, i64, i64, i64)> {
    let mut h = vec![0i64; nx];
    let mut best: Option<(i64, i64, i64, i64, i64)> = None;
    for j in 0..nz {
        for i in 0..nx {
            h[i] = if cells[j * nx + i] != 0 { h[i] + 1 } else { 0 };
        }
        let mut stack: Vec<usize> = vec![];
        for i in 0..=nx {
            let cur = if i < nx { h[i] } else { 0 };
            while let Some(&top) = stack.last() {
                if h[top] < cur {
                    break;
                }
                stack.pop();
                let height = h[top];
                if height == 0 {
                    continue;
                }
                let left = stack.last().map(|s| s + 1).unwrap_or(0) as i64;
                let area = height * (i as i64 - left);
                if best.map(|b| area > b.0).unwrap_or(true) {
                    best = Some((area, left, i as i64 - 1, j as i64 - height + 1, j as i64));
                }
            }
            stack.push(i);
        }
    }
    best
}

/// `inscribedRoofRect(frame)` as JSON { x0, x1, z0, z1, width, depth, cover }.
fn inscribed_roof_rect(frame: &Frame) -> Option<J> {
    let b = &frame.plan_bounds;
    let w = frame.width;
    let d = frame.depth;
    if !(w >= MIN_ROOF_SIDE_M && d >= MIN_ROOF_SIDE_M) {
        return None;
    }
    let g = clamp(w.max(d) / 26.0, 0.3, 1.0);
    let nx = 1f64.max(js_round(w / g)) as usize;
    let nz = 1f64.max(js_round(d / g)) as usize;
    let gx = w / nx as f64;
    let gz = d / nz as f64;
    let mut cells = vec![0u8; nx * nz];
    for j in 0..nz {
        for i in 0..nx {
            let p = frame.to_world(b.min_x + (i as f64 + 0.5) * gx, b.min_z + (j as f64 + 0.5) * gz);
            cells[j * nx + i] = if frame.signed_distance(p.x, p.z) < 0.0 { 1 } else { 0 };
        }
    }
    let (_, i0, i1, j0, j1) = largest_cell_rect(&cells, nx, nz)?;
    let mut r = Rect { v: [b.min_x + i0 as f64 * gx, b.min_x + (i1 + 1) as f64 * gx, b.min_z + j0 as f64 * gz, b.min_z + (j1 + 1) as f64 * gz] };
    let dir = [-1.0, 1.0, -1.0, 1.0];
    let limit = [b.min_x, b.max_x, b.min_z, b.max_z];
    let step = gx.min(gz) / 8.0;
    for side in 0..4 {
        let mut k = 0;
        while k < 10 && !side_inside(frame, &r, side) {
            r.v[side] -= dir[side] * step;
            k += 1;
        }
    }
    for side in 0..4 {
        for _ in 0..14 {
            let next = r.v[side] + dir[side] * step;
            let bounded = if dir[side] < 0.0 { limit[side].max(next) } else { limit[side].min(next) };
            if (bounded - r.v[side]).abs() < 1e-6 {
                break;
            }
            let mut trial = r;
            trial.v[side] = bounded;
            if !side_inside(frame, &trial, side) {
                break;
            }
            r.v[side] = bounded;
        }
    }
    let width = r.x1() - r.x0();
    let depth = r.z1() - r.z0();
    if !(width >= MIN_ROOF_SIDE_M && depth >= MIN_ROOF_SIDE_M) {
        return None;
    }
    if !(width * depth >= MIN_ROOF_COVER * frame.area) {
        return None;
    }
    Some(json!({ "x0": r.x0(), "x1": r.x1(), "z0": r.z0(), "z1": r.z1(), "width": width, "depth": depth, "cover": (width * depth) / frame.area }))
}

/// `sideOverhang(frame, r, side, full)`.
fn side_overhang(frame: &Frame, r: &J, side: &str, full: f64) -> f64 {
    let n_plan = match side {
        "x0" => [-1.0, 0.0],
        "x1" => [1.0, 0.0],
        "z0" => [0.0, -1.0],
        _ => [0.0, 1.0],
    };
    let at = r.num(side);
    let is_x = side.starts_with('x');
    let lo = if is_x { r.num("z0") } else { r.num("x0") };
    let hi = if is_x { r.num("z1") } else { r.num("x1") };
    let mut result = full;
    for edge in &frame.edges {
        let enx = edge.n.x * frame.right.x + edge.n.z * frame.right.z;
        let enz = edge.n.x * frame.front.x + edge.n.z * frame.front.z;
        if enx * n_plan[0] + enz * n_plan[1] < 0.94 {
            continue;
        }
        let a = frame.to_plan(edge.start.x, edge.start.z);
        let c = frame.to_plan(edge.end.x, edge.end.z);
        let coord = if is_x { (a.x + c.x) / 2.0 } else { (a.z + c.z) / 2.0 };
        if (coord - at).abs() > 0.3 {
            continue;
        }
        let e0 = if is_x { a.z.min(c.z) } else { a.x.min(c.x) };
        let e1 = if is_x { a.z.max(c.z) } else { a.x.max(c.x) };
        if hi.min(e1) - lo.max(e0) < 0.4 {
            continue;
        }
        if edge.party {
            return 0.02;
        }
        if (edge.role == "left" || edge.role == "right") && edge.lot_line.is_none() {
            result = result.min(FLANK_OVERHANG_M);
        }
    }
    result
}

/// (eave, rise, top) or None.
fn gable_stack(median: f64, half_span: f64, pitch: f64, allow_raise: bool) -> Option<(f64, f64, f64)> {
    let mut rise = MAX_RISE_M.min(pitch * half_span);
    let mut eave = median - (rise + ROOF_SLAB_M) / 2.0;
    if eave < MIN_WALL_M {
        let fitted = 2.0 * (median - MIN_WALL_M) - ROOF_SLAB_M;
        if !allow_raise && fitted < MIN_GABLE_RISE_M {
            return None;
        }
        rise = MIN_GABLE_RISE_M.max(rise.min(fitted));
        eave = MIN_WALL_M;
    }
    if rise < MIN_GABLE_RISE_M {
        return None;
    }
    Some((eave, rise, eave + rise + ROOF_SLAB_M))
}

// ---------------------------------------------------------------------------
// Analysis
// ---------------------------------------------------------------------------

struct Analysis<'a> {
    c: &'a Classified,
    frame: &'a Frame,
    form: &'static str,
    kind: Option<&'static str>,
    front: Option<&'a Edge>,
    front_length: f64,
    median: f64,
    flip: f64,
    wall: &'static str,
    /// `{ form: 'gable', rect, axis, halfSpan, pitch, eave, rise, top, overhang }` or `{ form: 'flat', eave, rise, top }`.
    roof: J,
    serves: bool,
    height_delta_m: f64,
    retail: bool,
}

fn analyse<'a>(c: &'a Classified, frame: &'a Frame) -> Analysis<'a> {
    let mut random = Rng::from_num(c.seed as f64);
    let (form, kind) = cottage_garage_form(c);
    let h = if c.height == 0.0 || c.height.is_nan() { 4.0 } else { c.height };
    let median = 1f64.max(h);
    let front = frame.edge_for_role("front").or_else(|| {
        let first = frame.edges.first()?;
        Some(frame.edges.iter().fold(first, |best, e| if e.length > best.length { e } else { best }))
    });
    let front_length = match front.map(|f| f.length) {
        Some(l) if l != 0.0 && !l.is_nan() => l,
        _ => 6.0,
    };
    let r_wall = random.next();
    let r_pitch = random.next();
    let r_flip = random.next();
    let r_pitched_accessory = random.next();
    let table: &[(&'static str, f64)] = if form == COTTAGE {
        &COTTAGE_WALLS
    } else if form == MINIMAL_TRADITIONAL {
        &POSTWAR_WALLS
    } else if kind == Some("studio") {
        &STUDIO_WALLS
    } else if frame.area < 14.0 {
        &SHED_WALLS
    } else {
        &GARAGE_WALLS
    };
    let wall = pick_weighted(r_wall, table);
    let rect = inscribed_roof_rect(frame);
    let mut roof: Option<J> = None;
    if let Some(rect) = &rect {
        let (rw, rd) = (rect.num("width"), rect.num("depth"));
        let axis = if form == COTTAGE {
            if rd >= 0.8 * rw { "z" } else { "x" }
        } else if form == MINIMAL_TRADITIONAL {
            if rw >= 0.7 * rd { "x" } else { "z" }
        } else if rw >= rd {
            "x"
        } else {
            "z"
        };
        let half_span = (if axis == "x" { rd } else { rw }) / 2.0;
        let (p0, p1) = pitch_range(form);
        let pitch = p0 + (p1 - p0) * r_pitch;
        let want_gable = form != ACCESSORY || (r_pitched_accessory < 0.58 && frame.area < MAX_PITCHED_ACCESSORY_M2);
        let stack = if want_gable { gable_stack(median, half_span, pitch, form != ACCESSORY) } else { None };
        if let Some((eave, rise, top)) = stack {
            let (oh_eave, oh_rake) = overhang_of(form);
            let (eave_sides, rake_sides) = if axis == "x" { (["z0", "z1"], ["x0", "x1"]) } else { (["x0", "x1"], ["z0", "z1"]) };
            let mut overhang = Map::new();
            for s in eave_sides {
                overhang.insert(s.into(), jn(side_overhang(frame, rect, s, oh_eave)));
            }
            for s in rake_sides {
                overhang.insert(s.into(), jn(side_overhang(frame, rect, s, oh_rake)));
            }
            roof = Some(json!({
                "form": "gable", "rect": rect, "axis": axis, "halfSpan": half_span, "pitch": pitch,
                "eave": eave, "rise": rise, "top": top, "overhang": J::Object(overhang),
            }));
        }
    }
    let roof = roof.unwrap_or_else(|| {
        let wall_h = MIN_WALL_M.max(median);
        json!({ "form": "flat", "eave": wall_h, "rise": 0, "top": wall_h })
    });
    let mean_roof = if roof.s("form") == Some("gable") { roof.num("eave") + (roof.num("rise") + ROOF_SLAB_M) / 2.0 } else { roof.num("top") };
    Analysis {
        c,
        frame,
        form,
        kind,
        front,
        front_length,
        median,
        flip: r_flip,
        wall,
        roof,
        serves: front_serves_way(frame),
        height_delta_m: mean_roof - median,
        retail: c.has_retail_ground,
    }
}

// ---------------------------------------------------------------------------
// Spec
// ---------------------------------------------------------------------------

fn blind() -> J {
    json!({ "window": "blind", "parapet": false })
}

fn normalise_bays(available: f64, bays: Vec<J>) -> Vec<J> {
    let total = bays.iter().fold(0.0, |s, b| s + b.num("width"));
    let total = if total == 0.0 || total.is_nan() { 1.0 } else { total };
    bays.into_iter()
        .map(|mut b| {
            let w = b.num("width");
            set(&mut b, "width", jn((available * w) / total));
            b
        })
        .collect()
}

fn front_party(a: &Analysis) -> bool {
    a.front.map(|f| f.party).unwrap_or(false)
}

fn cottage_front(a: &Analysis, level: i64) -> J {
    if front_party(a) {
        return blind();
    }
    let l = a.front_length;
    let end_pier = 0.45f64.min(l * 0.08);
    let avail = 0.5f64.max(l - 2.0 * end_pier);
    let head = 2.25f64.min(a.roof.num("eave") - 0.28);
    let sill_height = 0.85f64.min(0.5f64.max(head - 1.3));
    let window = |width: f64| json!({ "width": width, "storeys": { "0": "punched" }, "openingWidth": 1.05f64.min(0.6f64.max(width - 0.9)) });
    let door = |width: f64| json!({ "width": width, "storeys": { "0": "entrance" } });
    let bays = if avail >= 5.6 {
        vec![window((avail - 1.6) / 2.0), door(1.6), window((avail - 1.6) / 2.0)]
    } else if avail >= 3.4 {
        if a.flip < 0.5 { vec![door(1.6), window(avail - 1.6)] } else { vec![window(avail - 1.6), door(1.6)] }
    } else {
        vec![door(avail)]
    };
    json!({
        "window": "blind",
        "parapet": false,
        "endPier": end_pier,
        "bays": normalise_bays(avail, bays),
        "reveal": 0.1,
        "windowSpec": { "sillHeight": sill_height, "head": head, "frameWidth": 0.08, "frameDepth": 0.08, "mullions": 0, "transoms": if level >= 3 { 1 } else { 0 } },
        "sill": if level >= 2 { json!({ "height": 0.06, "projection": 0.06, "overhang": 0.06, "profile": "flat" }) } else { json!(false) },
        "entranceSpec": { "width": 0.95, "height": 2.05, "recess": 0.18, "transom": 0, "leaf": "single-solid", "frameWidth": 0.08 },
        "storefrontSpec": { "headClear": 0.28, "pier": 0.8 },
    })
}

fn postwar_front(a: &Analysis, level: i64) -> J {
    if front_party(a) {
        return blind();
    }
    let l = a.front_length;
    let end_pier = 0.35f64.min(l * 0.06);
    let avail = 0.5f64.max(l - 2.0 * end_pier);
    let head = 2.2f64.min(a.roof.num("eave") - 0.28);
    let sill_height = 0.6f64.min(0.45f64.max(head - 1.5));
    let garage = level >= 1 && !a.retail && avail >= 8.4;
    let picture = |width: f64| {
        json!({ "width": width, "storeys": { "0": "punched" }, "openingWidth": 3.0f64.min(0.8f64.max(width - 0.8)), "mullions": if level >= 2 { 2 } else { 0 } })
    };
    let door = json!({ "width": 1.4, "storeys": { "0": "entrance" } });
    let mut bays = if garage {
        vec![json!({ "width": 3.1, "storeys": { "0": "roller" } }), door.clone(), picture(avail - 4.5)]
    } else if avail >= 3.6 {
        vec![door.clone(), picture(avail - 1.4)]
    } else {
        let mut d = door.clone();
        set(&mut d, "width", jn(avail));
        vec![d]
    };
    if a.flip < 0.5 {
        bays.reverse();
    }
    json!({
        "window": "blind",
        "parapet": false,
        "endPier": end_pier,
        "bays": normalise_bays(avail, bays),
        "reveal": 0.12,
        "windowSpec": { "sillHeight": sill_height, "head": head, "frameWidth": 0.06, "frameDepth": 0.07, "mullions": 0 },
        "sill": if level >= 3 { json!({ "height": 0.05, "projection": 0.04, "overhang": 0.04, "profile": "flat" }) } else { json!(false) },
        "entranceSpec": { "width": 0.95, "height": 2.05, "recess": 0.18, "transom": 0, "leaf": "single-solid", "frameWidth": 0.07 },
        "storefrontSpec": { "headClear": 0.28, "pier": 0.8 },
        "rollerSpec": { "width": 2.7, "height": 2.1, "inset": 0.1, "hood": 0, "slat": if level >= 3 { 0.53 } else { 1e9 } },
    })
}

fn accessory_front(a: &Analysis, level: i64) -> J {
    if front_party(a) {
        return blind();
    }
    let l = a.front_length;
    let end_pier = 0.3f64.min(l * 0.08);
    let avail = 0.5f64.max(l - 2.0 * end_pier);
    let eave = a.roof.num("eave");
    let base = json!({ "window": "blind", "parapet": false, "endPier": end_pier, "reveal": 0.08 });
    if a.kind == Some("garage") && a.serves && avail >= 2.2 {
        let mut out = base;
        set(&mut out, "bays", json!([{ "width": avail, "storeys": { "0": "roller" } }]));
        set(&mut out, "rollerSpec", json!({ "width": 2.7f64.min(avail - 0.4), "height": 2.1, "inset": 0.08, "hood": 0, "slat": if level >= 2 { 0.6 } else { 1e9 } }));
        return out;
    }
    if a.kind == Some("studio") && level >= 1 {
        let upper = eave >= 4.8;
        let head = if upper { eave - 0.35 } else { 2.2f64.min(eave - 0.28) };
        let sill_height = if upper { 2.9f64.max(head - 1.3) } else { 0.9f64.min(0.5f64.max(head - 1.2)) };
        let door = json!({ "width": 1.4, "storeys": { "0": "entrance" } });
        let mut bays = if level >= 2 && avail >= 3.4 {
            vec![door, json!({ "width": avail - 1.4, "storeys": { "0": "punched" }, "openingWidth": 1.1f64.min(avail - 2.3) })]
        } else {
            let mut d = door;
            set(&mut d, "width", jn(avail));
            vec![d]
        };
        if a.flip < 0.5 {
            bays.reverse();
        }
        let mut out = base;
        set(&mut out, "bays", J::Array(normalise_bays(avail, bays)));
        set(&mut out, "windowSpec", json!({ "sillHeight": sill_height, "head": head, "frameWidth": 0.06, "frameDepth": 0.07, "mullions": 0 }));
        set(&mut out, "sill", json!(false));
        set(&mut out, "entranceSpec", json!({ "width": 0.9, "height": 2.0, "recess": 0.14, "transom": 0, "leaf": "single-solid", "frameWidth": 0.06 }));
        set(&mut out, "storefrontSpec", json!({ "headClear": 0.25, "pier": 0.8 }));
        return out;
    }
    blind()
}

fn street_return_elevations(a: &Analysis, level: i64) -> J {
    let mut by_edge = Map::new();
    if a.form != COTTAGE || level < 3 {
        return J::Object(by_edge);
    }
    let head = 2.25f64.min(a.roof.num("eave") - 0.28);
    for edge in &a.frame.edges {
        if edge.role != "front-secondary" || edge.party || !(edge.length >= 5.0) {
            continue;
        }
        by_edge.insert(
            edge.index.to_string(),
            json!({
                "window": "blind",
                "parapet": false,
                "endPier": 0.6,
                "bays": [{ "width": edge.length - 1.2, "storeys": { "0": "punched" }, "openingWidth": 1.0 }],
                "reveal": 0.1,
                "windowSpec": { "sillHeight": 0.85f64.min(0.5f64.max(head - 1.3)), "head": head, "frameWidth": 0.08, "frameDepth": 0.08, "mullions": 0, "transoms": 1 },
                "sill": { "height": 0.06, "projection": 0.06, "overhang": 0.06, "profile": "flat" },
            }),
        );
    }
    J::Object(by_edge)
}

fn build_spec(a: &Analysis, level: i64) -> J {
    let form = a.form;
    let roof = &a.roof;
    let front = if form == COTTAGE {
        cottage_front(a, level)
    } else if form == MINIMAL_TRADITIONAL {
        postwar_front(a, level)
    } else {
        accessory_front(a, level)
    };
    let is_gable = roof.s("form") == Some("gable");
    let gable = is_gable && (form != ACCESSORY || level >= 1);
    let rise = if gable { roof.num("rise") + ROOF_SLAB_M } else { 0.0 };
    let eave = if gable {
        roof.num("eave")
    } else if is_gable {
        MIN_WALL_M.max(a.median)
    } else {
        roof.num("eave")
    };
    let corrugated = a.wall == "corrugated-metal";
    let trim = if corrugated { "painted-steel-mid" } else { "painted-white" };
    let seed = a.c.seed;
    let roof_meta = if gable {
        json!({
            "form": "gable", "rect": roof.at("rect"), "axis": roof.at("axis"), "halfSpan": roof.at("halfSpan"), "rise": roof.at("rise"),
            "eave": roof.at("eave"), "overhang": roof.at("overhang"), "pitch": roof.at("pitch"),
        })
    } else {
        json!({ "form": "flat", "eave": eave })
    };
    let variant = (!a.c.variant.is_empty()).then_some(a.c.variant.as_str());
    json!({
        "id": format!("sf-archetype-{seed}"),
        "seed": seed,
        "height": eave + rise,
        "storeys": [{ "height": eave, "use": if form == ACCESSORY { "accessory" } else { "house" } }],
        "wallThickness": 0.2,
        "parapet": if gable { json!({ "height": rise, "thickness": 0.1 }) } else { json!(false) },
        "coping": false,
        "materials": {
            "wall": a.wall, "parapet": a.wall, "spandrel": a.wall, "base": "concrete-precast", "trim": trim, "coping": trim,
            "frame": if form == MINIMAL_TRADITIONAL { "anodized-aluminium" } else { "painted-white" },
            "glass": "glass-clear", "door": "painted-steel-mid", "doorGlass": "glass-door", "recess": a.wall,
            "roller": if corrugated { "painted-steel-light" } else { "sectional-garage-door" },
            "metal": "painted-steel-mid", "canopy": "painted-white",
            "roof": if corrugated { "corrugated-metal" } else { "asphalt-roof" },
            "pitchedRoof": if corrugated { "corrugated-metal" } else { "composition-shingle" },
        },
        "elevations": {
            "front": front,
            "front-secondary": blind(), "left": blind(), "right": blind(), "back": blind(), "default": blind(),
            "byEdge": street_return_elevations(a, level),
        },
        "roof": { "slab": 0.2, "sideFaces": false },
        "archetype": {
            "kind": "cottage-garage",
            "version": COTTAGE_GARAGE_VERSION,
            "form": form,
            "accessoryKind": a.kind,
            "variant": variant,
            "level": level,
            "retail": a.retail,
            "wall": a.wall,
            "roof": roof_meta,
            "heightDeltaM": if gable { a.height_delta_m } else { eave - a.median },
            "serves": a.serves,
            "porch": form == COTTAGE && !front_party(a),
            "stoop": form == MINIMAL_TRADITIONAL && !front_party(a),
            "budget": triangle_budget(form),
        },
    })
}

/// `cottageGarageSpec(classified, frame)`.
pub fn spec(c: &Classified, frame: &Frame) -> J {
    let a = analyse(c, frame);
    let budget = triangle_budget(a.form);
    let mut spec = J::Null;
    for level in DETAIL_LEVELS {
        spec = build_spec(&a, level);
        let mut probe = plan_elevation(&spec, frame, 0.0);
        add_base_detail(&mut probe, frame, &spec);
        let triangles = probe.parts.iter().map(triangles_for_part).sum::<usize>();
        set(obj_mut(&mut spec, "archetype"), "estimatedTriangles", json!(triangles));
        if triangles as f64 <= budget {
            break;
        }
    }
    spec
}

// ---------------------------------------------------------------------------
// Detail
// ---------------------------------------------------------------------------

/// The gable's frame (`gableGeometry`).
struct Gable {
    along_x: bool,
    o: P2,
    u: P2,
    n: P2,
    hw: f64,
    k: f64,
    e: f64,
    t: f64,
    basis: Basis,
    half_len: f64,
    eave_minus: f64,
    eave_plus: f64,
    rake_minus: f64,
    rake_plus: f64,
    ridge: f64,
    y_a: f64,
    y_b: f64,
}

fn gable_geometry(frame: &Frame, roof: &J, base_y: f64) -> Gable {
    let r = roof.get_nn("rect").cloned().unwrap_or(J::Null);
    let o = frame.to_world((r.num("x0") + r.num("x1")) / 2.0, (r.num("z0") + r.num("z1")) / 2.0);
    let rr = frame.right;
    let ff = frame.front;
    let along_x = roof.s("axis") == Some("x");
    let u = if along_x { rr } else { ff };
    let n = if along_x { ff } else { P2::new(-rr.x, -rr.z) };
    let oh = roof.get_nn("overhang").cloned().unwrap_or(J::Null);
    let hw = roof.num("halfSpan");
    let k = roof.num("rise") / hw;
    let e = roof.num("eave") + base_y;
    let eave_minus = if along_x { oh.num("z0") } else { oh.num("x1") };
    let eave_plus = if along_x { oh.num("z1") } else { oh.num("x0") };
    Gable {
        along_x,
        o,
        u,
        n,
        hw,
        k,
        e,
        t: ROOF_SLAB_M,
        basis: Basis { origin: [o.x, o.z], u: [u.x, u.z], n: [n.x, n.z] },
        half_len: (if along_x { r.num("width") } else { r.num("depth") }) / 2.0,
        eave_minus,
        eave_plus,
        rake_minus: if along_x { oh.num("x0") } else { oh.num("z0") },
        rake_plus: if along_x { oh.num("x1") } else { oh.num("z1") },
        ridge: e + roof.num("rise"),
        y_a: e - eave_minus * k,
        y_b: e - eave_plus * k,
    }
}

fn part_meta(kind: &J, part: &str) -> J {
    json!({ "archetype": kind, "part": part })
}

/// The bent roof slab, the gable-end infill and the street bargeboard.
fn add_gable_roof(plan: &mut Plan, frame: &Frame, meta: &J, base_y: f64) {
    let g = gable_geometry(frame, meta.get_nn("roof").unwrap_or(&J::Null), base_y);
    let kind = meta.at("kind").cloned().unwrap_or(J::Null);
    let t = g.t;
    plan.add_profile(
        &g.basis,
        PartArgs::profile(
            "roof-gable",
            "pitchedRoof",
            -g.half_len - g.rake_minus,
            g.half_len + g.rake_plus,
            vec![[-g.hw - g.eave_minus, g.y_a], [0.0, g.ridge], [g.hw + g.eave_plus, g.y_b], [g.hw + g.eave_plus, g.y_b + t], [0.0, g.ridge + t], [-g.hw - g.eave_minus, g.y_a + t]],
        )
        .meta(part_meta(&kind, "roof-slope")),
    );
    plan.add_profile(
        &g.basis,
        PartArgs::profile("gable-infill", "wall", -g.half_len, g.half_len, vec![[-g.hw, g.e - 0.01], [g.hw, g.e - 0.01], [0.0, g.ridge - 0.02]]).meta(part_meta(&kind, "gable-infill")),
    );
    if meta.s("form") == Some(COTTAGE) && !g.along_x && meta.num("level") >= 2.0 {
        let s_end = g.half_len + g.rake_plus;
        let plate_basis = Basis { origin: [g.o.x, g.o.z], u: [-g.n.x, -g.n.z], n: [g.u.x, g.u.z] };
        let d = 0.14;
        let (hw, em, ep, ya, yb, ridge) = (g.hw, g.eave_minus, g.eave_plus, g.y_a, g.y_b, g.ridge);
        plan.add_plate(
            &plate_basis,
            PartArgs {
                kind: "bargeboard".into(),
                slot: "trim".into(),
                w0: s_end + 0.002,
                w1: s_end + 0.03,
                profile: vec![[hw + em, ya + t], [0.0, ridge + t], [-(hw + ep), yb + t], [-(hw + ep), yb - d], [0.0, ridge - d], [hw + em, ya - d]],
                meta: Some(part_meta(&kind, "bargeboard")),
                ..Default::default()
            },
        );
    }
}

/// The cottage porch: deck, two posts and a roof over the door.
fn add_porch(plan: &mut Plan, front: &Edge, door: &OpeningRecord, meta: &J, base_y: f64, roof_top: f64) -> bool {
    let basis = front.basis;
    let kind = meta.at("kind").cloned().unwrap_or(J::Null);
    let centre = (door.s0 + door.s1) / 2.0;
    let width = clamp(door.s1 - door.s0 + 1.3, 2.0, 2.8);
    let s0 = 0.05f64.max(centre - width / 2.0);
    let s1 = (front.length - 0.05).min(centre + width / 2.0);
    if !(s1 - s0 >= 1.6) {
        return false;
    }
    let depth = 1.25;
    let deck_top = 0.18;
    let door_top = door.y1 - base_y;
    let eave = meta.get_nn("roof").num("eave");
    let y0 = (door_top + 0.12).max((door_top + 0.3).min(eave - 0.2));
    let thickness = 0.14;
    if !(y0 + thickness + base_y <= roof_top + 0.02) {
        return false;
    }
    let edge = front.index;
    let level = meta.num("level");
    if level >= 1.0 {
        plan.add_box(&basis, PartArgs::new("canopy", "base", s0, s1, base_y, base_y + deck_top, -0.02, depth).edge(edge).meta(part_meta(&kind, "porch-deck")));
        for s in [s0 + 0.08, s1 - 0.2] {
            plan.add_box(&basis, PartArgs::new("canopy-rod", "trim", s, s + 0.12, base_y + deck_top, base_y + y0, depth - 0.16, depth - 0.04).edge(edge).meta(part_meta(&kind, "porch-post")));
        }
    }
    if level >= 3.0 {
        let (ox, oz) = (basis.origin[0], basis.origin[1]);
        let (ux, uz) = (basis.u[0], basis.u[1]);
        let (nx, nz) = (basis.n[0], basis.n[1]);
        let mid = (s0 + s1) / 2.0;
        let half = (s1 - s0) / 2.0 + 0.1;
        let rise = 0.5;
        let porch_basis = Basis { origin: [ox + ux * mid, oz + uz * mid], u: [nx, nz], n: [-ux, -uz] };
        let yb = base_y + y0;
        plan.add_profile(
            &porch_basis,
            PartArgs::profile(
                "canopy",
                "roof",
                -0.02,
                depth + 0.15,
                vec![[-half, yb], [0.0, yb + rise], [half, yb], [half, yb + thickness], [0.0, yb + rise + thickness], [-half, yb + thickness]],
            )
            .edge(edge)
            .meta(part_meta(&kind, "porch-roof")),
        );
    } else {
        plan.add_box(&basis, PartArgs::new("canopy", "roof", s0 - 0.1, s1 + 0.1, base_y + y0, base_y + y0 + thickness, -0.02, depth + 0.15).edge(edge).meta(part_meta(&kind, "porch-roof")));
    }
    true
}

/// The postwar entry: a concrete step and, from level 2, a flat hood.
fn add_stoop(plan: &mut Plan, front: &Edge, door: &OpeningRecord, meta: &J, base_y: f64) {
    let basis = front.basis;
    let kind = meta.at("kind").cloned().unwrap_or(J::Null);
    let s0 = 0.05f64.max(door.s0 - 0.3);
    let s1 = (front.length - 0.05).min(door.s1 + 0.3);
    plan.add_box(&basis, PartArgs::new("canopy", "base", s0, s1, base_y, base_y + 0.15, -0.02, 0.9).edge(front.index).meta(part_meta(&kind, "entry-step")));
    if meta.num("level") >= 2.0 {
        let y0 = door.y1 + 0.12;
        if y0 + 0.1 <= base_y + meta.get_nn("roof").num("eave") + 0.02 {
            plan.add_box(&basis, PartArgs::new("canopy", "canopy", s0 - 0.1, s1 + 0.1, y0, y0 + 0.1, -0.02, 0.75).edge(front.index).meta(part_meta(&kind, "entry-hood")));
        }
    }
}

fn front_openings(plan: &Plan, front: Option<&Edge>) -> Vec<OpeningRecord> {
    front.and_then(|f| plan.edges.iter().find(|e| e.index == f.index)).map(|e| e.openings.clone()).unwrap_or_default()
}

/// `addCottageGarageBaseDetail`.
fn add_base_detail(plan: &mut Plan, frame: &Frame, spec: &J) {
    let Some(meta) = spec.get_nn("archetype").filter(|m| truthy(Some(m))).cloned() else { return };
    let base_y = if plan.base_y == 0.0 || plan.base_y.is_nan() { 0.0 } else { plan.base_y };
    let roof_top = plan.envelope.roof_top + base_y;
    let gable = meta.get_nn("roof").s("form") == Some("gable");
    if gable {
        add_gable_roof(plan, frame, &meta, base_y);
    }
    let front = frame.edge_for_role("front");
    let openings = front_openings(plan, front);
    let door = openings.iter().find(|o| o.type_ == "entrance").cloned();
    let roller = openings.iter().find(|o| o.type_ == "roller").cloned();
    let mut porch = false;
    if let (Some(f), Some(d)) = (front, &door) {
        if meta.truthy("porch") {
            porch = add_porch(plan, f, d, &meta, base_y, roof_top);
        }
        if meta.truthy("stoop") && meta.num("level") >= 1.0 {
            add_stoop(plan, f, d, &meta, base_y);
        }
    }
    let garage = match (front, &roller) {
        (Some(f), Some(r)) => {
            let centre = (r.s0 + r.s1) / 2.0;
            let b = f.basis;
            json!({ "edge": f.index, "width": r.s1 - r.s0, "world": { "x": b.origin[0] + b.u[0] * centre, "z": b.origin[1] + b.u[1] * centre }, "outward": [b.n[0], b.n[1]] })
        }
        _ => J::Null,
    };
    let roof = meta.get_nn("roof").cloned().unwrap_or(J::Null);
    plan.extra.insert(
        "archetype".into(),
        json!({
            "kind": meta.at("kind"),
            "version": meta.at("version"),
            "form": meta.at("form"),
            "accessoryKind": meta.at("accessoryKind"),
            "variant": meta.at("variant"),
            "level": meta.at("level"),
            "retail": meta.at("retail"),
            "roof": roof.at("form"),
            "porch": porch,
            "garage": garage,
            "heightDeltaM": meta.at("heightDeltaM"),
            "roofPlant": !gable && meta.s("form") != Some(ACCESSORY),
            "farMassTop": base_y + plan.envelope.roof_top - if gable { (roof.num("rise") + ROOF_SLAB_M) / 2.0 } else { 0.0 },
            "budget": meta.at("budget"),
        }),
    );
    if meta.s("form") == Some(ACCESSORY) {
        plan.extra.insert("lodFarFromM".into(), json!(ACCESSORY_FAR_RUNG_FROM_M));
    }
}

/// `addCottageGarageDetail(plan, frame, spec)`.
pub fn detail(plan: &mut Plan, frame: &Frame, spec: &J) {
    add_base_detail(plan, frame, spec);
    let close = add_close_detail(plan, frame, spec);
    if let (Some(c), Some(a)) = (close, plan.extra.get_mut("archetype")) {
        set(a, "close", c);
    }
}

// ---------------------------------------------------------------------------
// Close-range detail (LOD0 only)
// ---------------------------------------------------------------------------

fn close_budget(form: &str) -> f64 {
    match form {
        ACCESSORY => 300.0,
        _ => 3900.0,
    }
}
const CLOSE_DROP_ORDER: [&str; 10] = ["services", "muntins", "brackets", "beads", "roof-courses", "sashes", "trim", "roof-edge", "garage", "door"];
const COTTAGE_DOORS: [&str; 3] = ["lite-top", "six-panel", "three-horizontal"];
const POSTWAR_DOORS: [&str; 3] = ["three-horizontal", "flush", "lite-top"];

/// The gable end wall's basis at the ridge's +u (sign 1) or -u (sign -1) end.
fn gable_end_basis(g: &Gable, sign: f64) -> Basis {
    let origin = [g.o.x + g.u.x * g.half_len * sign, g.o.z + g.u.z * g.half_len * sign];
    if sign > 0.0 {
        Basis { origin, u: [-g.n.x, -g.n.z], n: [g.u.x, g.u.z] }
    } else {
        Basis { origin, u: [g.n.x, g.n.z], n: [-g.u.x, -g.u.z] }
    }
}

fn slack_beyond(frame: &Frame, value: f64, axis: &str, dir: f64) -> f64 {
    let b = &frame.plan_bounds;
    let a = if frame.allowance.is_finite() { frame.allowance } else { 0.45 };
    let limit = if axis == "x" {
        if dir > 0.0 { b.max_x } else { b.min_x }
    } else if dir > 0.0 {
        b.max_z
    } else {
        b.min_z
    };
    if dir > 0.0 { limit + a - value } else { value - (limit - a) }
}

struct Slots {
    budget: f64,
    trim: String,
    frame: String,
    metal: String,
    door: String,
    base: String,
    roller: String,
    glass: String,
    pipe: String,
}

struct Eave {
    sign: f64,
    w: f64,
    y: f64,
    overhang: f64,
    slack: f64,
}

#[allow(clippy::too_many_arguments)]
fn add_gable_close_detail(cx: &mut Cx, plan: &mut Plan, frame: &Frame, meta: &J, base_y: f64, spout: f64, slots: &Slots) {
    let g = gable_geometry(frame, meta.get_nn("roof").unwrap_or(&J::Null), base_y);
    let r_edge = CEdge::basis(g.basis, false);
    let s0 = -g.half_len - g.rake_minus;
    let s1 = g.half_len + g.rake_plus;
    let t = g.t;
    let shingle = plan.material("pitchedRoof") != Some("corrugated-metal");
    let accessory = meta.s("form") == Some(ACCESSORY);
    let roof = meta.get_nn("roof").cloned().unwrap_or(J::Null);
    let r = roof.get_nn("rect").cloned().unwrap_or(J::Null);
    let side_minus = if g.along_x { ("z", -1.0, r.num("z0") - g.eave_minus) } else { ("x", 1.0, r.num("x1") + g.eave_minus) };
    let side_plus = if g.along_x { ("z", 1.0, r.num("z1") + g.eave_plus) } else { ("x", -1.0, r.num("x0") - g.eave_plus) };
    let eaves = [
        Eave { sign: -1.0, w: -g.hw - g.eave_minus, y: g.y_a, overhang: g.eave_minus, slack: slack_beyond(frame, side_minus.2, side_minus.0, side_minus.1) },
        Eave { sign: 1.0, w: g.hw + g.eave_plus, y: g.y_b, overhang: g.eave_plus, slack: slack_beyond(frame, side_plus.2, side_plus.0, side_plus.1) },
    ];
    let level = meta.num("level");
    if !accessory {
        cx.group = "roof-edge".into();
        let floor = base_y + PEDESTRIAN_HEADROOM_M;
        for e in &eaves {
            if e.overhang < 0.1 || e.y - 0.13 < floor {
                continue;
            }
            let w0 = if e.sign < 0.0 { e.w - 0.025 } else { e.w };
            let w1 = if e.sign < 0.0 { e.w } else { e.w + 0.025 };
            cx.profile(plan, r_edge, CK::TRIM, &slots.trim, "fascia", s0, s1, vec![[w0, e.y - 0.13], [w1, e.y - 0.13], [w1, e.y + t + 0.012], [w0, e.y + t + 0.012]], None);
        }
        cx.profile(
            plan,
            r_edge,
            CK::COURSE,
            "pitchedRoof",
            "ridge-cap",
            s0,
            s1,
            vec![[-0.13, g.ridge + t - 0.13 * g.k - 0.003], [0.13, g.ridge + t - 0.13 * g.k - 0.003], [0.0, g.ridge + t + 0.035]],
            Some(&json!({ "roof": true })),
        );
        cx.group = "trim".into();
        for sign in [1.0f64, -1.0] {
            let s_end = if sign > 0.0 { s1 } else { -s0 };
            let plate_basis = if sign > 0.0 {
                Basis { origin: [g.o.x, g.o.z], u: [-g.n.x, -g.n.z], n: [g.u.x, g.u.z] }
            } else {
                Basis { origin: [g.o.x, g.o.z], u: [g.n.x, g.n.z], n: [-g.u.x, -g.u.z] }
            };
            let p = CEdge::basis(plate_basis, false);
            let sw = |w: f64| if sign > 0.0 { -w } else { w };
            let top = |w: f64| g.ridge + t - w.abs() * g.k;
            let w_m = -g.hw - g.eave_minus;
            let w_p = g.hw + g.eave_plus;
            let street = meta.s("form") == Some(COTTAGE) && !g.along_x && sign > 0.0 && level >= 2.0;
            let d = if street { 0.14 } else { 0.18 };
            if top(w_m).min(top(w_p)) - d - 0.09 < floor {
                continue;
            }
            if !street {
                cx.plate(
                    plan,
                    p,
                    CK::TRIM,
                    &slots.trim,
                    "rake-board",
                    vec![
                        [sw(w_m), top(w_m) + 0.01],
                        [sw(0.0), top(0.0) + 0.01],
                        [sw(w_p), top(w_p) + 0.01],
                        [sw(w_p), top(w_p) - d],
                        [sw(0.0), top(0.0) - d],
                        [sw(w_m), top(w_m) - d],
                    ],
                    s_end + 0.002,
                    s_end + 0.03,
                    None,
                );
            }
            if meta.s("form") == Some(COTTAGE) && sign > 0.0 && !g.along_x {
                cx.group = "brackets".into();
                for (wa, wb) in [(w_m + 0.1, -0.05), (0.05, w_p - 0.1)] {
                    let len = (wb - wa).abs();
                    let n = 2f64.max(9f64.min(js_round(len / 0.32))) as i64;
                    let y_top = |w: f64| top(w) - d;
                    let mut pts = vec![[sw(wa), y_top(wa)], [sw(wb), y_top(wb)]];
                    let mut i = n;
                    while i >= 0 {
                        let w = wa + ((wb - wa) * i as f64) / n as f64;
                        pts.push([sw(w), y_top(w) - if i % 2 != 0 { 0.075 } else { 0.035 }]);
                        i -= 1;
                    }
                    cx.plate(plan, p, CK::TRIM, &slots.trim, "bargeboard-drop", pts, s_end + 0.004, s_end + 0.026, None);
                }
                cx.group = "trim".into();
            }
        }
        if shingle {
            cx.group = "roof-courses".into();
            let used = plan.parts.iter().fold(0.0, |s, p| s + close_part_triangles(p));
            let spare = slots.budget - used - 900.0;
            let slope_len = (g.hw + g.eave_minus.max(g.eave_plus)).hypot(g.hw * g.k);
            let want = js_round(slope_len / 0.2);
            let n = 0f64.max(want.min((spare / 24.0).floor()));
            if n >= 4.0 {
                let exposure = slope_len / n;
                for e in &eaves {
                    let low_y = e.y + t;
                    let f = if low_y >= floor + 0.01 { 0.0 } else { (floor + 0.01 - low_y) / 1e-6f64.max(g.ridge + t - low_y) };
                    if f >= 0.9 {
                        continue;
                    }
                    shingle_courses(cx, plan, r_edge, "pitchedRoof", "shingle-course", s0 + 0.01, s1 - 0.01, [e.w * (1.0 - f), low_y + (g.ridge + t - low_y) * f], [0.0, g.ridge + t], exposure);
                }
            }
        }
    }
    cx.group = "roof-edge".into();
    for e in &eaves {
        let room = 0.13f64.min(e.slack - 0.03);
        if e.overhang < 0.1 || room < 0.07 || e.y - 0.16 < base_y + PEDESTRIAN_HEADROOM_M {
            continue;
        }
        let y_e = e.y - 0.06;
        let wi = e.w;
        let wo = e.w + e.sign * room;
        let a = wi.min(wo);
        let b = wi.max(wo);
        cx.profile(
            plan,
            r_edge,
            CK::TRIM,
            &slots.metal,
            "gutter",
            s0 + 0.02,
            s1 - 0.02,
            vec![[a, y_e + 0.1], [a + 0.01, y_e + 0.02], [a + 0.035, y_e - 0.015], [b - 0.035, y_e - 0.015], [b - 0.01, y_e + 0.02], [b, y_e + 0.1], [b - 0.012, y_e + 0.1], [a + 0.012, y_e + 0.1]],
            None,
        );
        if accessory {
            continue;
        }
        let end = (if spout < 0.5 { 1.0 } else { -1.0 }) * e.sign;
        let s = end * (g.half_len - 0.12);
        let wall_w = e.sign * g.hw;
        let px = g.o.x + g.u.x * s + g.n.x * wall_w;
        let pz = g.o.z + g.u.z * s + g.n.z * wall_w;
        if frame.signed_distance(px, pz).abs() > 0.06 {
            continue;
        }
        let basis = if e.sign > 0.0 {
            Basis { origin: [px, pz], u: [g.u.x, g.u.z], n: [g.n.x, g.n.z] }
        } else {
            Basis { origin: [px, pz], u: [-g.u.x, -g.u.z], n: [-g.n.x, -g.n.z] }
        };
        let reach_out = 0.1f64.max(0.2f64.min(e.slack + (wall_w - e.w).abs() - 0.04));
        downspout(cx, plan, CEdge::basis(basis, true), &slots.pipe, 0.0, y_e - 0.02, base_y, reach_out, Some([(e.w - wall_w).abs() + room / 2.0, y_e - 0.02]));
    }
    if !accessory {
        cx.group = "services".into();
        let sign = if g.along_x { if g.rake_plus >= g.rake_minus { 1.0 } else { -1.0 } } else { 1.0 };
        let b = CEdge::basis(gable_end_basis(&g, sign), true);
        let rise = roof.num("rise");
        let vy = g.e + rise * 0.45;
        let vh = 0.42f64.min(rise * 0.42);
        if vh > 0.2 {
            cx.plate(
                plan,
                b,
                CK::FIXTURE,
                &slots.trim,
                "gable-vent",
                vec![[-0.22, vy - vh / 2.0], [0.22, vy - vh / 2.0], [0.22, vy + vh / 2.0 - 0.08], [0.0, vy + vh / 2.0 + 0.06], [-0.22, vy + vh / 2.0 - 0.08]],
                0.0,
                0.03,
                None,
            );
            let mut y = vy - vh / 2.0 + 0.06;
            while y < vy + vh / 2.0 - 0.1 {
                cx.bx(plan, b, CK::COURSE, &slots.metal, "vent-louvre", [-0.17, 0.17, y, y + 0.025, 0.01, 0.04], None);
                y += 0.07;
            }
        }
    }
}

/// Edges a passer-by sees.
fn visible_edges(frame: &Frame) -> Vec<&Edge> {
    frame.edges.iter().filter(|e| !e.party && (e.role == "front" || e.role == "front-secondary" || e.lot_line.is_some() || e.street_return)).collect()
}

fn profile_of(p: &Part) -> Vec<[f64; 2]> {
    match &p.shape {
        Shape::Profile(v) => v.clone(),
        _ => vec![],
    }
}

/// `addCottageGarageCloseDetail(plan, frame, spec)`.
fn add_close_detail(plan: &mut Plan, frame: &Frame, spec: &J) -> Option<J> {
    let meta = spec.get_nn("archetype").filter(|m| truthy(Some(m))).cloned()?;
    let base_y = if plan.base_y == 0.0 || plan.base_y.is_nan() { 0.0 } else { plan.base_y };
    let seed = match spec.at("seed") {
        Some(J::Number(n)) => crate::rng::js_num(n.as_f64().unwrap_or(0.0)),
        Some(J::String(s)) => s.clone(),
        _ => "undefined".into(),
    };
    let mut random = Rng::new(&format!("{seed}|cottage-garage|close"));
    let r_door = random.next();
    let r_muntins = random.next();
    let r_spout = random.next();
    let r_meter = random.next();
    let r_dryer = random.next();
    let r_lamp = random.next();
    let r_flip = random.next();
    let r_mailbox = random.next();
    let mut cx = close_context(plan, meta.s("kind").unwrap_or(""));
    let form = meta.s("form").unwrap_or("").to_string();
    let budget = close_budget(&form);
    let accessory = form == ACCESSORY;
    let trim = cx.slot(plan, &["trim", "canopy", "frame"]);
    let metal = cx.slot(plan, &["metal", "frame", "trim"]);
    let slots = Slots {
        budget,
        frame: cx.slot(plan, &["frame", "trim"]),
        door: cx.slot(plan, &["door", "frame"]),
        base: cx.slot(plan, &["base", "trim"]),
        roller: cx.slot(plan, &["roller", "door"]),
        glass: cx.slot(plan, &["glass", "doorGlass"]),
        pipe: if form == COTTAGE { trim.clone() } else { metal.clone() },
        trim,
        metal,
    };
    let wall_mat = plan.material("wall").unwrap_or("").to_string();
    let stucco = wall_mat.contains("stucco") || wall_mat.contains("plaster");
    let front = frame.edge_for_role("front");
    let openings = front_openings(plan, front);
    let roller = openings.iter().find(|o| o.type_ == "roller").cloned();
    let door = openings.iter().find(|o| o.type_ == "entrance").cloned();
    let fit = |want: f64, edge: Option<&Edge>| {
        let a = edge.map(|e| e.allowance).filter(|a| a.is_finite()).unwrap_or(0.45);
        0.03f64.max(want.min(a - 0.04))
    };
    let fitf = |want: f64| fit(want, front);
    let gable = meta.get_nn("roof").s("form") == Some("gable") && plan.parts.iter().any(|p| p.kind == "roof-gable");

    if accessory {
        cx.group = "garage".into();
        if let (Some(f), Some(r)) = (front, &roller) {
            let cw = 0.09;
            cx.bx(plan, f, CK::TRIM, &slots.trim, "door-frame", [r.s0 - cw, r.s0, r.y0, r.y1 + cw, -0.01, fitf(0.03)], None);
            cx.bx(plan, f, CK::TRIM, &slots.trim, "door-frame", [r.s1, r.s1 + cw, r.y0, r.y1 + cw, -0.01, fitf(0.03)], None);
            cx.bx(plan, f, CK::TRIM, &slots.trim, "door-frame", [r.s0 - cw, r.s1 + cw, r.y1, r.y1 + cw, -0.01, fitf(0.03)], None);
        }
        if gable {
            add_gable_close_detail(&mut cx, plan, frame, &meta, base_y, r_spout, &slots);
        } else if let Some(f) = front.filter(|f| !f.party && plan.envelope.wall_top - 0.12 >= PEDESTRIAN_HEADROOM_M) {
            cx.group = "roof-edge".into();
            let y = base_y + plan.envelope.wall_top;
            let p = fitf(0.12);
            cx.profile(
                plan,
                f,
                CK::TRIM,
                &slots.metal,
                "gutter",
                0.02,
                f.length - 0.02,
                vec![[-0.01, y], [-0.005, y - 0.08], [0.03, y - 0.11], [p - 0.03, y - 0.11], [p, y - 0.06], [p, y + 0.005], [p - 0.012, y + 0.005], [0.005, y]],
                None,
            );
        }
        let drops = fit_close_budget(plan, budget, &["roof-edge", "garage"]);
        return Some(json!({ "drops": drops }));
    }

    // 1. The gable.
    if gable {
        add_gable_close_detail(&mut cx, plan, frame, &meta, base_y, r_spout, &slots);
    }

    // 2. Walls a passer-by sees.
    let eave = base_y + plan.envelope.wall_top;
    for edge in visible_edges(frame) {
        cx.group = "beads".into();
        for (at, dir, convex) in [(0.0, 1.0, edge.convex_start), (edge.length, -1.0, edge.convex_end)] {
            if !convex {
                continue;
            }
            if stucco {
                if eave - base_y - PEDESTRIAN_HEADROOM_M > 0.3 {
                    cx.column(plan, edge, CK::TRIM, "wall", "corner-bead", bead_section(at, dir), base_y + PEDESTRIAN_HEADROOM_M, eave, None);
                }
            } else {
                cx.bx(
                    plan,
                    edge,
                    CK::TRIM,
                    &slots.trim,
                    "corner-board",
                    [if dir > 0.0 { at } else { at - 0.11 }, if dir > 0.0 { at + 0.11 } else { at }, base_y + 0.3, eave - 0.01, -0.005, fit(0.024, Some(edge))],
                    None,
                );
            }
        }
        cx.group = "trim".into();
        let ops = front_openings(plan, Some(edge));
        let mut cuts: Vec<[f64; 2]> = ops.iter().filter(|o| o.type_ == "roller" || o.type_ == "entrance").map(|o| [o.s0 - 0.02, o.s1 + 0.02]).collect();
        cuts.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap_or(std::cmp::Ordering::Equal));
        cuts.push([edge.length, f64::INFINITY]);
        let mut s = 0.0f64;
        for [a, b] in cuts {
            if a - s > 0.15 {
                cx.profile(plan, edge, CK::TRIM, &slots.base, "foundation", s, a, trim_profile("sloped", fit(if stucco { 0.025 } else { 0.04 }, Some(edge)), 0.3, base_y - 0.05, -0.01), None);
            }
            s = s.max(b);
        }
    }

    // 3. Windows.
    let cottage = form == COTTAGE;
    let sash_lites = if r_muntins < 0.45 { [2.0, 1.0] } else if r_muntins < 0.8 { [3.0, 2.0] } else { [1.0, 1.0] };
    for edge in visible_edges(frame) {
        let ops = front_openings(plan, Some(edge));
        for op in &ops {
            if op.type_ != "punched" {
                continue;
            }
            let Some(lights) = opening_lights(plan, edge.index, op) else { continue };
            let w_at = lights.w_at;
            let cols = &lights.cols;
            let rows = &lights.rows;
            let sash_w = [w_at + 0.002, w_at + 0.03];
            let bar_w = [w_at + 0.001, w_at + 0.02];
            cx.group = "sashes".into();
            if !lights.framed {
                close_ring(&cx, plan, edge, CK::TRIM, &slots.frame, "frame-restored", op.s0, op.s1, op.y0, op.y1, 0.06, w_at - 0.035, w_at + 0.035);
            }
            if cottage {
                let (a, b) = (cols[0][0], cols[cols.len() - 1][1]);
                let c = rows[0][0];
                let d = rows[rows.len() - 1][1];
                let mid = if rows.len() > 1 { (rows[0][1] + rows[1][0]) / 2.0 } else { c + (d - c) * 0.52 };
                close_ring(&cx, plan, edge, CK::BAR, &slots.frame, "sash-upper", a, b, mid - 0.02, d, 0.045, sash_w[0], sash_w[1]);
                close_ring(&cx, plan, edge, CK::BAR, &slots.frame, "sash-lower", a, b, c, mid + 0.02, 0.05, sash_w[0] + 0.025, sash_w[1] + 0.025);
                cx.group = "muntins".into();
                close_grid(&cx, plan, edge, &slots.frame, "muntin", a + 0.045, b - 0.045, mid + 0.02, d - 0.045, sash_lites[0], sash_lites[1], 0.024, bar_w[0], bar_w[1]);
                close_grid(
                    &cx,
                    plan,
                    edge,
                    &slots.frame,
                    "muntin",
                    a + 0.05,
                    b - 0.05,
                    c + 0.06,
                    mid - 0.02,
                    sash_lites[0],
                    if sash_lites[1] == 2.0 { 2.0 } else { 1.0 },
                    0.024,
                    sash_w[0] + 0.025,
                    sash_w[1] + 0.02,
                );
            } else {
                let wide = cols.len() >= 3;
                for (ci, [a, b]) in cols.iter().enumerate() {
                    let (a, b) = (*a, *b);
                    for [c, d] in rows.iter() {
                        let (c, d) = (*c, *d);
                        if wide && ci == cols.len() / 2 {
                            continue;
                        }
                        close_ring(&cx, plan, edge, CK::BAR, &slots.frame, "slider-sash", a, b, c, d, 0.035, sash_w[0], sash_w[1]);
                        if !wide && cols.len() == 1 && b - a > 0.7 {
                            let m = (a + b) / 2.0;
                            cx.bx(plan, edge, CK::BAR, &slots.frame, "meeting-stile", [m - 0.03, m + 0.03, c, d, sash_w[0], sash_w[1] + 0.02], None);
                        }
                    }
                }
            }
            cx.group = "trim".into();
            let kit_sill = plan.parts.iter().find(|p| p.kind == "sill" && p.edge == Some(edge.index) && p.y1 <= op.y0 + 0.02 && p.s0 <= op.s0 && p.s1 >= op.s1).cloned();
            let p = fit(if cottage { 0.06 } else { 0.045 }, Some(edge));
            if let Some(ks) = kit_sill {
                let outer = profile_of(&ks).iter().fold(f64::NEG_INFINITY, |m, q| m.max(q[0]));
                cx.bx(plan, edge, CK::TRIM, &ks.slot, "sill-drip", [ks.s0 + 0.01, ks.s1 - 0.01, ks.y0 - 0.014, ks.y0 + 0.002, outer - 0.022, outer - 0.004], None);
            } else {
                let y = op.y0;
                let ext = if cottage { 0.12 } else { 0.04 };
                cx.profile(
                    plan,
                    edge,
                    CK::TRIM,
                    if cottage { &slots.trim } else { &slots.frame },
                    "drip-sill",
                    op.s0 - ext,
                    op.s1 + ext,
                    vec![[-0.08, y], [p, y - 0.018], [p, y - 0.055], [p - 0.018, y - 0.055], [p - 0.018, y - 0.04], [-0.08, y - 0.04]],
                    None,
                );
            }
            if cottage {
                let cw = 0.1;
                cx.bx(plan, edge, CK::TRIM, &slots.trim, "casing", [op.s0 - cw, op.s0, op.y0 - 0.04, op.y1 + 0.02, -0.005, 0.024], None);
                cx.bx(plan, edge, CK::TRIM, &slots.trim, "casing", [op.s1, op.s1 + cw, op.y0 - 0.04, op.y1 + 0.02, -0.005, 0.024], None);
                let f55 = fit(0.055, Some(edge));
                cx.profile(
                    plan,
                    edge,
                    CK::TRIM,
                    &slots.trim,
                    "head-casing",
                    op.s0 - cw - 0.02,
                    op.s1 + cw + 0.02,
                    vec![[-0.005, op.y1], [0.024, op.y1], [0.024, op.y1 + 0.14], [f55, op.y1 + 0.15], [f55, op.y1 + 0.19], [-0.005, op.y1 + 0.19]],
                    None,
                );
                cx.bx(plan, edge, CK::TRIM, &slots.trim, "apron", [op.s0 - 0.02, op.s1 + 0.02, op.y0 - 0.16, op.y0 - 0.04, -0.005, 0.02], None);
            } else if stucco {
                cx.group = "beads".into();
                cx.column(plan, edge, CK::TRIM, "wall", "reveal-bead", bead_section(op.s0, -1.0), op.y0, op.y1, None);
                cx.column(plan, edge, CK::TRIM, "wall", "reveal-bead", bead_section(op.s1, 1.0), op.y0, op.y1, None);
                cx.profile(
                    plan,
                    edge,
                    CK::TRIM,
                    "wall",
                    "reveal-bead",
                    op.s0,
                    op.s1,
                    vec![[-0.045, op.y1], [-0.022, op.y1 - 0.014], [0.006, op.y1 - 0.006], [0.014, op.y1 + 0.022], [0.0, op.y1 + 0.045]],
                    None,
                );
            }
        }
    }

    // 4. The door.
    cx.group = "door".into();
    if let (Some(f), Some(door)) = (front, &door) {
        let leaf_parts: Vec<Part> = plan.parts.iter().filter(|p| p.kind == "door-leaf" && p.meta.s("aperture") == Some(door.id.as_str())).cloned().collect();
        if !leaf_parts.is_empty() {
            let s0 = leaf_parts.iter().fold(f64::INFINITY, |m, q| m.min(q.s0));
            let s1 = leaf_parts.iter().fold(f64::NEG_INFINITY, |m, q| m.max(q.s1));
            let y0 = leaf_parts.iter().fold(f64::INFINITY, |m, q| m.min(q.y0));
            let y1 = leaf_parts.iter().fold(f64::NEG_INFINITY, |m, q| m.max(q.y1));
            let face = leaf_parts.iter().fold(f64::NEG_INFINITY, |m, q| m.max(q.w1));
            let list = if cottage { &COTTAGE_DOORS } else { &POSTWAR_DOORS };
            let style = list[((r_door * 3.0).floor() as usize) % 3];
            door_leaf_detail(
                &cx,
                plan,
                f,
                style,
                &DoorLeaf {
                    s0,
                    s1,
                    y0,
                    y1,
                    face,
                    door_slot: &slots.door,
                    metal_slot: &slots.metal,
                    frame_slot: &slots.frame,
                    glass_slot: &slots.glass,
                    hinge_left: r_flip < 0.5,
                },
            );
        }
        let cw = if cottage { 0.11 } else { 0.07 };
        cx.bx(plan, f, CK::TRIM, &slots.trim, "door-casing", [door.s0 - cw, door.s0, door.y0, door.y1 + cw, -0.005, 0.026], None);
        cx.bx(plan, f, CK::TRIM, &slots.trim, "door-casing", [door.s1, door.s1 + cw, door.y0, door.y1 + cw, -0.005, 0.026], None);
        cx.bx(plan, f, CK::TRIM, &slots.trim, "door-casing", [door.s0 - cw - 0.02, door.s1 + cw + 0.02, door.y1 + cw, door.y1 + cw + 0.05, -0.005, fitf(0.05)], None);
        cx.group = "services".into();
        let side = if r_lamp < 0.5 { -1.0 } else { 1.0 };
        let lamp_s = if side < 0.0 { door.s0 - cw - 0.16 } else { door.s1 + cw + 0.16 };
        let num_s = if side < 0.0 { door.s1 + cw + 0.16 } else { door.s0 - cw - 0.16 };
        let hr = base_y + PEDESTRIAN_HEADROOM_M;
        let ly = if door.y1 - 0.35 < hr + 0.17 && hr + 0.32 < eave - 0.05 { hr + 0.17 } else { door.y1 - 0.35 };
        if lamp_s > 0.15 && lamp_s < f.length - 0.15 {
            cx.bx(plan, f, CK::FIXTURE, &slots.metal, "lamp-plate", [lamp_s - 0.06, lamp_s + 0.06, ly - 0.1, ly + 0.12, -0.005, 0.02], None);
            cx.bx(plan, f, CK::FIXTURE, &slots.frame, "lamp", [lamp_s - 0.065, lamp_s + 0.065, ly - 0.17, ly + 0.07, fitf(0.05), fitf(0.18)], None);
            cx.bx(plan, f, CK::FIXTURE, &slots.metal, "lamp-cap", [lamp_s - 0.08, lamp_s + 0.08, ly + 0.07, ly + 0.1, fitf(0.04), fitf(0.19)], None);
        }
        if num_s > 0.2 && num_s < f.length - 0.2 {
            let ny = door.y1 - 0.1;
            cx.bx(plan, f, CK::FIXTURE, &slots.frame, "number-plate", [num_s - 0.11, num_s + 0.11, ny - 0.07, ny + 0.07, -0.005, 0.012], None);
            for d in 0..4 {
                let ds = num_s - 0.09 + 0.045 * d as f64;
                cx.bx(plan, f, CK::FIXTURE, &slots.metal, "number-digit", [ds, ds + 0.03, ny - 0.045, ny + 0.045, 0.012, 0.02], None);
            }
            if r_mailbox < 0.7 {
                cx.bx(plan, f, CK::FIXTURE, &slots.metal, "mailbox", [num_s - 0.13, num_s + 0.13, base_y + 1.1, base_y + 1.42, -0.005, fitf(0.09)], None);
                cx.bx(plan, f, CK::FIXTURE, &slots.metal, "mailbox-lid", [num_s - 0.145, num_s + 0.145, base_y + 1.42, base_y + 1.45, -0.005, fitf(0.11)], None);
            }
        }
    }

    // 5. Porch posts, beam, deck nosing and hood brackets.
    cx.group = "brackets".into();
    let fitting = json!({ "fitting": true });
    let posts: Vec<Part> = plan.parts.iter().filter(|p| p.kind == "canopy-rod" && p.meta.s("part") == Some("porch-post")).cloned().collect();
    let beam = plan.parts.iter().find(|p| p.kind == "canopy" && p.meta.s("part") == Some("porch-roof")).cloned();
    for post in &posts {
        let Some(edge) = post.edge.and_then(|i| frame.edges.get(i)) else { continue };
        let top = post.y1;
        cx.bx(plan, edge, CK::TRIM, &slots.trim, "post-cap", [post.s0 - 0.025, post.s1 + 0.025, top - 0.08, top, post.w0 - 0.025, post.w1 + 0.025], Some(&fitting));
        cx.bx(plan, edge, CK::TRIM, &slots.trim, "post-base", [post.s0 - 0.025, post.s1 + 0.025, post.y0, post.y0 + 0.12, post.w0 - 0.025, post.w1 + 0.025], Some(&fitting));
        for dir in [-1.0, 1.0] {
            let sa = if dir < 0.0 { post.s0 } else { post.s1 };
            let reach = 0.32;
            cx.plate(
                plan,
                edge,
                CK::BRACKET,
                &slots.trim,
                "post-bracket",
                vec![[sa, top - 0.08], [sa + dir * reach, top - 0.08], [sa, top - 0.08 - reach * 1.1]],
                post.w0 + 0.035,
                post.w1 - 0.035,
                Some(&fitting),
            );
        }
    }
    if let Some(beam) = &beam {
        if !posts.is_empty() {
            if let Some(edge) = beam.edge.and_then(|i| frame.edges.get(i)) {
                let y = posts.iter().fold(f64::INFINITY, |m, p| m.min(p.y1));
                let w0 = posts.iter().fold(f64::INFINITY, |m, p| m.min(p.w0));
                let w1 = posts.iter().fold(f64::NEG_INFINITY, |m, p| m.max(p.w1));
                let s0 = posts.iter().fold(f64::INFINITY, |m, p| m.min(p.s0)) - 0.05;
                let s1 = posts.iter().fold(f64::NEG_INFINITY, |m, p| m.max(p.s1)) + 0.05;
                cx.bx(plan, edge, CK::TRIM, &slots.trim, "porch-beam", [s0, s1, y - 0.2, y.min(beam.y0 + 0.001), w0, w1], Some(&fitting));
            }
        }
    }
    let deck = plan.parts.iter().find(|p| p.kind == "canopy" && matches!(p.meta.s("part"), Some("porch-deck") | Some("entry-step"))).cloned();
    if let Some(deck) = &deck {
        if let Some(edge) = deck.edge.and_then(|i| frame.edges.get(i)) {
            let reach = (deck.w1 + 0.28).min(1.7);
            cx.bx(plan, edge, CK::TRIM, &slots.base, "nosing", [deck.s0, deck.s1, deck.y1 - 0.035, deck.y1, deck.w1 - 0.01, (deck.w1 + 0.025).min(1.75)], Some(&fitting));
            if deck.y1 - deck.y0 > 0.16 && reach - deck.w1 > 0.2 {
                let sm = (deck.s0 + deck.s1) / 2.0;
                cx.bx(plan, edge, CK::TRIM, &slots.base, "porch-step", [sm - 0.6, sm + 0.6, deck.y0, deck.y0 + (deck.y1 - deck.y0) / 2.0, deck.w1 - 0.02, reach], Some(&fitting));
            }
        }
    }
    let hood = plan.parts.iter().find(|p| p.kind == "canopy" && p.meta.s("part") == Some("entry-hood")).cloned();
    if let Some(hood) = &hood {
        if let Some(edge) = hood.edge.and_then(|i| frame.edges.get(i)) {
            for s in [hood.s0 + 0.06, hood.s1 - 0.12] {
                cx.profile(
                    plan,
                    edge,
                    CK::BRACKET,
                    &slots.trim,
                    "hood-bracket",
                    s,
                    s + 0.06,
                    vec![[-0.01, hood.y0], [(hood.w1 - 0.05).min(0.4), hood.y0], [-0.01, hood.y0 - 0.3]],
                    Some(&fitting),
                );
            }
        }
    }

    // 6. Services.
    cx.group = "services".into();
    if let Some(f) = front.filter(|f| !f.party) {
        let mut ground: Vec<&OpeningRecord> = openings.iter().filter(|o| o.storey == 0).collect();
        ground.sort_by(|a, b| a.s0.partial_cmp(&b.s0).unwrap_or(std::cmp::Ordering::Equal));
        let mut gaps: Vec<[f64; 2]> = vec![];
        let mut cursor = 0.3;
        for o in &ground {
            let pad = if o.type_ == "punched" { 0.15 } else { 0.22 };
            if o.s0 - pad - cursor >= 0.5 {
                gaps.push([cursor, o.s0 - pad]);
            }
            cursor = f64::max(cursor, o.s1 + pad);
        }
        if f.length - 0.3 - cursor >= 0.5 {
            gaps.push([cursor, f.length - 0.3]);
        }
        if !gaps.is_empty() {
            let [a, b] = gaps[((r_meter * gaps.len() as f64).floor() as usize) % gaps.len()];
            let c = if r_meter < 0.5 { a + 0.2 } else { b - 0.2 };
            let m = slots.metal.as_str();
            cx.bx(plan, f, CK::FIXTURE, m, "meter-box", [c - 0.14, c + 0.14, base_y + 1.2, base_y + 1.66, -0.005, fitf(0.11)], None);
            cx.bx(plan, f, CK::FIXTURE, &slots.frame, "meter-dome", [c - 0.075, c + 0.075, base_y + 1.35, base_y + 1.53, fitf(0.1), fitf(0.17)], None);
            cx.bx(plan, f, CK::FIXTURE, m, "conduit", [c - 0.022, c + 0.022, base_y + 0.02, base_y + 1.2, -0.005, fitf(0.045)], None);
            if b - a >= 0.75 {
                let gx = if r_meter < 0.5 { c + 0.32 } else { c - 0.32 };
                cx.bx(plan, f, CK::FIXTURE, m, "gas-riser", [gx - 0.02, gx + 0.02, base_y, base_y + 0.82, 0.02, fitf(0.06)], None);
                cx.bx(plan, f, CK::FIXTURE, m, "gas-meter", [gx - 0.12, gx + 0.12, base_y + 0.36, base_y + 0.74, 0.03, fitf(0.24)], None);
                cx.bx(plan, f, CK::FIXTURE, m, "gas-manifold", [gx - 0.14, gx + 0.14, base_y + 0.74, base_y + 0.8, 0.04, fitf(0.09)], None);
            }
            if r_dryer < 0.6 {
                let ds = (a + b) / 2.0 + if r_meter < 0.5 { 0.15 } else { -0.15 };
                cx.bx(plan, f, CK::FIXTURE, m, "dryer-vent", [ds - 0.08, ds + 0.08, base_y + 0.42, base_y + 0.58, -0.005, fitf(0.09)], None);
            }
        }
    }

    // 7. Garage door joinery.
    cx.group = "garage".into();
    if let (Some(f), Some(roller)) = (front, &roller) {
        if let Some(curtain) = plan.parts.iter().find(|p| p.kind == "roller-door" && p.meta.s("opening") == Some(roller.id.as_str())).cloned() {
            let mut joints: Vec<f64> = plan
                .parts
                .iter()
                .filter(|p| p.kind == "roller-slat" && p.edge == Some(f.index) && p.s0 >= curtain.s0 - 1e-3 && p.s1 <= curtain.s1 + 1e-3)
                .map(|p| (p.y0 + p.y1) / 2.0)
                .collect();
            joints.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let mut cuts = vec![curtain.y0 + 0.08];
            if !joints.is_empty() {
                cuts.extend(joints);
            } else {
                let mut y = curtain.y0 + 0.53;
                while y < curtain.y1 - 0.2 {
                    cuts.push(y);
                    y += 0.53;
                }
            }
            cuts.push(curtain.y1);
            garage_door_detail(
                &cx,
                plan,
                f,
                if r_door < 0.5 { "short-panel" } else { "long-panel" },
                &Curtain { s0: curtain.s0, s1: curtain.s1 },
                &cuts,
                curtain.w1 + 0.03,
                &slots.roller,
                &slots.metal,
                &slots.glass,
                r_mailbox < 0.25,
            );
            let m = slots.metal.as_str();
            cx.bx(plan, f, CK::TRIM, m, "weatherstrip", [curtain.s0, curtain.s0 + 0.03, curtain.y0, curtain.y1, curtain.w1, curtain.w1 + 0.045], None);
            cx.bx(plan, f, CK::TRIM, m, "weatherstrip", [curtain.s1 - 0.03, curtain.s1, curtain.y0, curtain.y1, curtain.w1, curtain.w1 + 0.045], None);
            cx.bx(plan, f, CK::TRIM, m, "weatherstrip", [curtain.s0, curtain.s1, curtain.y1 - 0.03, curtain.y1, curtain.w1, curtain.w1 + 0.045], None);
            let cw = 0.09;
            cx.bx(plan, f, CK::TRIM, &slots.trim, "garage-casing", [roller.s0 - cw, roller.s0, roller.y0, roller.y1 + cw, -0.005, fitf(0.026)], None);
            cx.bx(plan, f, CK::TRIM, &slots.trim, "garage-casing", [roller.s1, roller.s1 + cw, roller.y0, roller.y1 + cw, -0.005, fitf(0.026)], None);
            cx.bx(plan, f, CK::TRIM, &slots.trim, "garage-casing", [roller.s0 - cw, roller.s1 + cw, roller.y1, roller.y1 + cw, -0.005, fitf(0.026)], None);
        }
    }

    let drops = fit_close_budget(plan, budget, &CLOSE_DROP_ORDER);
    Some(json!({ "drops": drops }))
}
