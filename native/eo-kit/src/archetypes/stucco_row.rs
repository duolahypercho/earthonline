//! Archetype `stucco-row` (src/world/buildings/archetypes/stucco-row.js): the
//! San Francisco tract and interwar stucco row house, its spec, its detail pass
//! and its close-range (LOD0) detail. The close-detail helpers cottage-garage
//! uses (`close_context`, `close_ring`, `close_grid`, ...) are public here.
//!
//! Not ported: `refreshWorld` (Rust parts carry no world box), the plan's
//! `diagnostics` and `checkPlanContainment` (reports only), and the entry
//! aperture's `y0`/`height`/`world` updates (the Rust aperture record has no such
//! fields; its `stoop` is recorded).

use crate::classify::Classified;
use crate::elevation::{BayRecord, OpeningRecord, Part, PartArgs, Plan, Shape, arc_points, trim_profile, trim_profile_d};
use crate::frame::{Basis, Edge, Frame, P2, point_in_polygon};
use crate::js::*;
use crate::rng::Rng;
use std::collections::HashSet;
use std::f64::consts::PI;

pub const STUCCO_ROW_VERSION: &str = "sf-archetype-stucco-row-v8";

const TRACT_DISTRICTS: [&str; 12] = [
    "Sunset/Parkside",
    "Inner Sunset",
    "West of Twin Peaks",
    "Excelsior",
    "Oceanview/Merced/Ingleside",
    "Visitacion Valley",
    "Portola",
    "Outer Mission",
    "Bayview Hunters Point",
    "Lakeshore",
    "Seacliff",
    "Twin Peaks",
];
const RICHMOND_DISTRICTS: [&str; 2] = ["Inner Richmond", "Outer Richmond"];
const ROLL_YEAR_PLACEHOLDER: f64 = 1900.0;

const GARAGE_STOREY_MIN: f64 = 2.7;
const GARAGE_STOREY_MAX: f64 = 2.85;
const GARAGE_STOREY_LOW_M: f64 = 2.4;
const UPPER_STOREY_M: f64 = 2.95;
const MIN_STOREY_M: f64 = 2.3;
const FRONT_END_PIER_M: f64 = 0.3;
const ENTRY_BAY_M: f64 = 1.75;
const GARAGE_BAY_M: f64 = 3.3;
const WINDOW_BAY_M: f64 = 4.2;
const GARAGE_DOOR_WIDTH: f64 = 2.55;
const GARAGE_DOOR_HEIGHT: f64 = 2.15;
const RISER_M: f64 = 0.18;
const DOOR_CLEAR_M: f64 = 2.0;
const PORCH_RECESS_M: f64 = 0.9;
const WALL_THICKNESS_M: f64 = 0.28;
const PICTURE_MAX_M: f64 = 3.0;
const MODERNE_BAY_DEPTH_M: f64 = 0.4;
const ORNAMENT_STOREYS: usize = 2;
const CORNER_RETURN_MIN_M: f64 = 4.0;
const RETURN_BAY_M: f64 = 2.9;
const LOT_LINE_BAY_M: f64 = 3.0;
const STEPPED_MIN: f64 = 1.8;
const STEPPED_MAX: f64 = 6.0;
const STEPPED_SETBACK_MIN: f64 = 0.4;
const STEPPED_SETBACK_MAX: f64 = 5.0;
const STEPPED_PORCH_RECESS_M: f64 = 0.32;
const TREAD_M: f64 = 0.28;
const STAIR_MAX_RISER: f64 = 0.195;
const STAIR_TREAD: f64 = 0.26;
const STAIR_MIN_TREAD: f64 = 0.24;
const STAIR_LANDING: f64 = 0.95;
const STAIR_MIN_LANDING: f64 = 0.6;
const STAIR_MAX_WIDTH: f64 = 1.3;
const STAIR_DOOR: f64 = 2.05;
pub const STUCCO_ROW_TRIANGLE_BUDGET: f64 = 2200.0;
const ROOF_PLANT_RESERVE_TRI: f64 = 100.0;

fn paint(name: &str) -> u32 {
    match name {
        "white" => 0xffffff,
        "cream" => 0xfff3dc,
        "butter" => 0xfff0c2,
        "peach" => 0xffe0c8,
        "blush" => 0xf9dad8,
        "salmon" => 0xf8d6c6,
        "pink" => 0xf7d8dc,
        "rose" => 0xf2d0d6,
        "mint" => 0xdaf0dc,
        "sage" => 0xdfe8cf,
        "powder" => 0xd9e7f5,
        "sky" => 0xcfe0f2,
        "lilac" => 0xe6dcf0,
        "sand" => 0xeedfc4,
        "stone" => 0xe0dbd1,
        "dove" => 0xd6d8da,
        _ => 0xffffff,
    }
}

fn district_palette(group: &str) -> &'static [&'static str] {
    match group {
        "marina" => &["white", "white", "cream", "sand", "butter", "peach", "stone"],
        "tract" => &["white", "white", "white", "cream", "cream", "dove", "stone", "pink", "salmon", "peach", "mint", "powder"],
        "richmond" => &["white", "dove", "stone", "sand", "powder", "sage", "cream"],
        _ => &["white", "cream", "sand", "sage", "powder", "peach", "stone", "butter"],
    }
}

fn style_palette_bias(style: &str) -> Option<&'static [&'static str]> {
    match style {
        "tract-moderne" => Some(&["white", "cream", "powder", "mint"]),
        "tract-postwar" => Some(&["dove", "sage", "sand", "sky"]),
        _ => None,
    }
}

/// `STYLE[style]` (undefined: null).
fn style_base(style: &str) -> J {
    let dr = if crate::typology::detail_enabled() { 0.055 } else { 0.0 };
    match style {
        "tract-storybook" => json!({
            "wall": "scanned-painted-plaster", "frame": "painted-white", "parapet": 0.6, "coping": "cast-stone-trim",
            "reveal": 0.16 + dr, "sill": { "height": 0.08, "projection": 0.05, "overhang": 0.06, "profile": "sloped" },
            "archedPicture": true, "archedPorch": true, "archedGarage": true, "gable": true, "pent": false,
            "entryHood": true, "pictureHood": true, "pictureMullions": 2, "smallTransoms": 1, "smallMullions": 0,
        }),
        "interwar" => json!({
            "wall": "scanned-painted-plaster", "frame": "painted-white", "parapet": 0.65, "coping": "cast-stone-trim",
            "reveal": 0.18 + dr, "sill": { "height": 0.08, "projection": 0.05, "overhang": 0.06, "profile": "sloped" },
            "archedPicture": true, "archedPorch": true, "archedGarage": true, "gable": false, "pent": true,
            "entryHood": true, "pictureHood": false, "pictureMullions": 2, "smallTransoms": 1, "smallMullions": 0,
        }),
        "tract-moderne" => json!({
            "wall": "painted-stucco", "frame": "painted-white", "parapet": 0.5, "coping": "cast-stone-trim",
            "reveal": 0.12 + dr, "sill": { "height": 0.05, "projection": 0.03, "overhang": 0.03, "profile": "flat" },
            "archedPicture": false, "archedPorch": false, "archedGarage": false, "gable": false, "pent": false,
            "moderneBay": true, "bands": true, "glassBlock": true, "pictureMullions": 2, "smallTransoms": 2, "smallMullions": 0,
        }),
        "tract-postwar" => json!({
            "wall": "scanned-painted-plaster", "frame": "anodized-aluminium", "parapet": 0.4, "coping": "anodized-aluminium",
            "reveal": 0.12 + dr, "sill": false,
            "archedPicture": false, "archedPorch": false, "archedGarage": false, "gable": false, "pent": false,
            "eyebrow": true, "veneer": true, "pictureMullions": 2, "smallTransoms": 0, "smallMullions": 1,
        }),
        "marina-mediterranean" => json!({
            "wall": "scanned-painted-plaster", "frame": "painted-white", "parapet": 0.7, "coping": "cast-stone-trim",
            "reveal": 0.2 + dr, "sill": { "height": 0.1, "projection": 0.06, "overhang": 0.08, "profile": "sloped" },
            "archedPicture": false, "archedTop": true, "archedPorch": true, "archedGarage": true, "gable": false, "pent": true,
            "entryHood": false, "eyebrows": true, "balconette": true, "pictureMullions": 2, "smallTransoms": 1, "smallMullions": 0,
        }),
        "richmond-row" => json!({
            "wall": "painted-stucco", "frame": "painted-white", "parapet": 0.6, "coping": "cast-stone-trim",
            "reveal": 0.12 + dr, "sill": false,
            "archedPicture": false, "archedPorch": false, "archedGarage": false, "gable": false, "pent": false,
            "band": true, "pictureMullions": 1, "smallTransoms": 1, "smallMullions": 0,
        }),
        _ => J::Null,
    }
}

// ---------------------------------------------------------------------------
// Evidence and style.
// ---------------------------------------------------------------------------

struct Evidence {
    year: Option<f64>,
    district: Option<String>,
    pitched: Option<bool>,
    peak: Option<f64>,
}

impl Evidence {
    fn to_json(&self) -> J {
        json!({ "year": self.year, "district": self.district, "pitched": self.pitched, "peak": self.peak })
    }
}

fn evidence_of(c: &Classified) -> Evidence {
    let civic = c.civic.as_ref();
    let civic_year = civic.and_then(|v| v.year_built).unwrap_or(f64::NAN);
    let placeholder = civic.and_then(|v| v.year_exact) == Some(false);
    let mut year = None;
    if civic_year.is_finite() && civic_year > 1800.0 && !placeholder {
        year = Some(civic_year);
    } else {
        let y = c.year_built;
        if y.is_finite() && y > 1800.0 && y != ROLL_YEAR_PLACEHOLDER {
            year = Some(y);
        }
    }
    let district = civic.and_then(|v| v.district.clone());
    let pitched = civic.and_then(|v| v.pitched_roof);
    let peak = civic.and_then(|v| v.roof_peak_m).filter(|p| p.is_finite() && *p > 0.0);
    Evidence { year, district, pitched, peak }
}

/// `stuccoRowFrontTreatment(style, roll)`.
pub fn stucco_row_front_treatment(style: &str, roll: f64) -> J {
    if crate::typology::detail_enabled() {
        if style == "tract-storybook" {
            return if roll < 0.55 {
                json!({ "gable": true, "pent": false })
            } else if roll < 0.85 {
                json!({ "gable": false, "pent": true })
            } else {
                json!({ "gable": false, "pent": false })
            };
        }
        if style == "interwar" {
            return if roll < 0.28 {
                json!({ "gable": true, "pent": false })
            } else {
                json!({ "pent": roll < 0.72 })
            };
        }
        if style == "marina-mediterranean" && roll < 0.28 {
            return json!({ "gable": true, "pent": true });
        }
    }
    if style == "tract-storybook" {
        return if roll < 0.4 {
            json!({ "gable": true, "pent": false })
        } else if roll < 0.75 {
            json!({ "gable": false, "pent": true })
        } else {
            json!({ "gable": false, "pent": false })
        };
    }
    if style == "interwar" {
        return json!({ "pent": roll < 0.6 });
    }
    json!({})
}

fn style_of(meta: &J) -> J {
    let treatment = if truthy(meta.at("treatment")) { meta.at("treatment") } else { None };
    spread(Some(&style_base(meta.s("style").unwrap_or(""))), treatment)
}

fn variant_of(c: &Classified) -> Option<&str> {
    (!c.variant.is_empty()).then_some(c.variant.as_str())
}

fn stucco_row_style(c: &Classified, ev: &Evidence, roll: f64) -> &'static str {
    let district = ev.district.as_deref();
    if variant_of(c) == Some("marina") || district == Some("Marina") {
        return "marina-mediterranean";
    }
    if district.map(|d| RICHMOND_DISTRICTS.contains(&d)).unwrap_or(false) {
        return "richmond-row";
    }
    if let Some(year) = ev.year {
        if year < 1939.0 {
            let d_truthy = district.map(|d| !d.is_empty()).unwrap_or(false);
            return if d_truthy && !TRACT_DISTRICTS.contains(&district.unwrap()) { "interwar" } else { "tract-storybook" };
        }
        if year < 1950.0 {
            return "tract-moderne";
        }
        return "tract-postwar";
    }
    if variant_of(c) == Some("tract") {
        return if roll < 0.45 { "tract-moderne" } else { "tract-postwar" };
    }
    if roll < 0.6 { "tract-storybook" } else { "interwar" }
}

fn pitched_roof_finishes(style: &str) -> &'static [(&'static str, f64)] {
    match style {
        "tract-storybook" | "interwar" => &[("composition-shingle", 4.0), ("cedar-shingle", 1.0), ("clay-barrel-tile", 4.0)],
        "richmond-row" => &[("composition-shingle", 5.0), ("cedar-shingle", 2.0), ("clay-barrel-tile", 1.0)],
        "marina-mediterranean" => &[("clay-barrel-tile", 3.0), ("composition-shingle", 1.0)],
        "tract-moderne" => &[("composition-shingle", 1.0), ("cedar-shingle", 1.0)],
        "tract-postwar" => &[("composition-shingle", 4.0), ("cedar-shingle", 1.0)],
        _ => &[("composition-shingle", 4.0), ("cedar-shingle", 1.0), ("clay-barrel-tile", 4.0)],
    }
}

fn pitched_roof_finish(style: &str, seed: u32) -> &'static str {
    let list = pitched_roof_finishes(style);
    let total = list.iter().fold(0.0, |t, (_, w)| t + w);
    let mut at = Rng::new(&format!("{seed}|pitched-roof")).next() * total;
    for (finish, w) in list {
        at -= w;
        if at < 0.0 {
            return finish;
        }
    }
    list[list.len() - 1].0
}

fn palette_for(style: &str, district: Option<&str>, roll_a: f64, roll_b: f64) -> u32 {
    let group = match district {
        Some("Marina") => "marina",
        Some(d) if RICHMOND_DISTRICTS.contains(&d) => "richmond",
        Some(d) if TRACT_DISTRICTS.contains(&d) => "tract",
        None => "tract",
        _ => "other",
    };
    let bias = style_palette_bias(style);
    let names = match bias {
        Some(b) if roll_a < 0.34 => b,
        _ => district_palette(group),
    };
    paint(names[((roll_b * names.len() as f64).floor() as usize) % names.len()])
}

// ---------------------------------------------------------------------------
// Massing.
// ---------------------------------------------------------------------------

fn storey_stack(height: f64, reserve: f64, garage: f64) -> Vec<f64> {
    let usable = (height - reserve).max(2.3);
    let mut garage = garage;
    if usable < garage + MIN_STOREY_M {
        if usable < GARAGE_STOREY_LOW_M + MIN_STOREY_M {
            return vec![usable];
        }
        garage = GARAGE_STOREY_LOW_M.max(usable - MIN_STOREY_M);
    }
    let mut upper_count = js_round((usable - garage) / UPPER_STOREY_M).max(1.0);
    while upper_count > 1.0 && (usable - garage) / upper_count < 2.55 {
        upper_count -= 1.0;
    }
    let upper = (usable - garage) / upper_count;
    let mut out = vec![garage];
    for _ in 0..upper_count as usize {
        out.push(upper);
    }
    out
}

fn front_bays(available: f64, garage: bool, centre_entry: bool, no_entry: bool) -> (Vec<&'static str>, Vec<f64>) {
    let mut kinds: Vec<&'static str> = vec![];
    let mut widths: Vec<f64> = vec![];
    let count_for = |rest: f64| if rest >= 1.3 { js_round(rest / WINDOW_BAY_M).max(1.0) as usize } else { 0 };
    if no_entry {
        if garage && available >= GARAGE_DOOR_WIDTH + 0.4 {
            let rest = available - GARAGE_BAY_M;
            let count = count_for(rest);
            kinds.push("garage");
            widths.push(GARAGE_BAY_M + if count > 0 { 0.0 } else { rest.max(0.0) });
            for _ in 0..count {
                kinds.push("window");
                widths.push(rest / count as f64);
            }
        } else {
            kinds.push("window");
            widths.push(available);
        }
    } else if available < ENTRY_BAY_M + 0.2 {
        kinds.push("entry");
        widths.push(available);
    } else if !garage || available < ENTRY_BAY_M + GARAGE_BAY_M - 0.4 {
        kinds.push("entry");
        widths.push(ENTRY_BAY_M);
        let rest = available - ENTRY_BAY_M;
        let count = count_for(rest);
        for _ in 0..count {
            kinds.push("window");
            widths.push(rest / count as f64);
        }
        if count == 0 {
            widths[0] = available;
        }
    } else {
        let rest = available - ENTRY_BAY_M - GARAGE_BAY_M;
        let count = count_for(rest);
        let garage_w = GARAGE_BAY_M + if count > 0 { 0.0 } else { rest };
        if centre_entry && count > 0 {
            kinds.push("garage");
            widths.push(garage_w);
            kinds.push("entry");
            widths.push(ENTRY_BAY_M);
        } else {
            kinds.push("entry");
            widths.push(ENTRY_BAY_M);
            kinds.push("garage");
            widths.push(garage_w);
        }
        for _ in 0..count {
            kinds.push("window");
            widths.push(rest / count as f64);
        }
    }
    let total = widths.iter().fold(0.0, |a, b| a + b);
    let total = if total == 0.0 || total.is_nan() { 1.0 } else { total };
    (kinds, widths.iter().map(|w| (available * w) / total).collect())
}

/// The set-back entry run of a stepped front: (edge index, setback).
fn find_stepped_entry(frame: &Frame, front: Option<&Edge>) -> Option<(usize, f64)> {
    let front = front?;
    let front_z = frame.to_plan(front.start.x, front.start.z).z;
    let mut best: Option<(usize, f64)> = None;
    for edge in &frame.edges {
        if edge.role != "front-secondary" || edge.street_return {
            continue;
        }
        if edge.length < STEPPED_MIN || edge.length > STEPPED_MAX {
            continue;
        }
        let setback = front_z - frame.to_plan(edge.midpoint.x, edge.midpoint.z).z;
        if setback < STEPPED_SETBACK_MIN || setback > STEPPED_SETBACK_MAX {
            continue;
        }
        if best.map(|(i, _)| edge.length > frame.edges[i].length).unwrap_or(true) {
            best = Some((edge.index, setback));
        }
    }
    best
}

/// World point on an edge basis.
fn on_edge(basis: &Basis, s: f64, w: f64) -> P2 {
    P2::new(basis.origin[0] + basis.u[0] * s + basis.n[0] * w, basis.origin[1] + basis.u[1] * s + basis.n[1] * w)
}

/// `fitStairPorch`: { risers, riser, tread, depth, rise, width } or None.
fn fit_stair_porch(frame: &Frame, edge: Option<&Edge>, bay_s: [f64; 2], rise: f64) -> Option<J> {
    let risers = (rise / STAIR_MAX_RISER).ceil();
    let riser = rise / risers;
    let bay_width = bay_s[1] - bay_s[0];
    let width = STAIR_MAX_WIDTH.min(bay_width - 0.45);
    let edge = edge?;
    if !(width >= 0.9) {
        return None;
    }
    for (tread, landing) in [(STAIR_TREAD, STAIR_LANDING), (STAIR_MIN_TREAD, STAIR_MIN_LANDING)] {
        let depth = (risers - 1.0) * tread + landing;
        let mut ok = true;
        for s in [bay_s[0] + 0.05, (bay_s[0] + bay_s[1]) / 2.0, bay_s[1] - 0.05] {
            for w in [-depth - 0.2, -depth / 2.0] {
                let p = on_edge(&edge.basis, s, w);
                if !point_in_polygon(p.x, p.z, &frame.polygon) {
                    ok = false;
                }
            }
        }
        if ok {
            return Some(json!({ "risers": risers, "riser": riser, "tread": tread, "depth": depth, "rise": rise, "width": width }));
        }
    }
    None
}

fn curved_bay_share(style: &str) -> Option<f64> {
    match style {
        "tract-moderne" => Some(1.0),
        "tract-storybook" | "interwar" => Some(0.35),
        "richmond-row" => Some(0.25),
        "marina-mediterranean" => Some(0.2),
        "tract-postwar" => Some(0.0),
        _ => None,
    }
}
const GATE_SHARE: f64 = 0.55;
const GATE_BAR_PITCH_M: f64 = 0.12;
const GATE_BAR_M: f64 = 0.024;
const PORCH_ARCH_RISE_M: f64 = 0.55;
const BAY_HEADROOM_M: f64 = 2.35;

// ---------------------------------------------------------------------------
// Spec.
// ---------------------------------------------------------------------------

/// `stuccoRowSpec(classified, frame)`.
pub fn spec(c: &Classified, frame: &Frame) -> J {
    let mut random = Rng::new(&format!("{}|stucco-row", c.seed));
    let r_style = random.next();
    let r_paint_a = random.next();
    let r_paint_b = random.next();
    let r_flip = random.next();
    let r_centre = random.next();
    let r_garage = random.next();
    let r_garage_door = random.next();
    let r_door = random.next();
    let r_wall_alt = random.next();
    let r_front = random.next();
    let mut more = Rng::new(&format!("{}|stucco-row-front", c.seed));
    let m_curved = more.next();
    let m_gate = more.next();
    let evidence = evidence_of(c);
    let style = stucco_row_style(c, &evidence, r_style);
    let treatment = stucco_row_front_treatment(style, r_front);
    let st = spread(Some(&style_base(style)), Some(&treatment));
    let retail = c.has_retail_ground;
    let height = if c.height == 0.0 || c.height.is_nan() { 6.5 } else { c.height };

    let pitched = evidence.pitched == Some(true);
    let excess = evidence.peak.map(|p| p - height);
    let parapet_h = 0.15f64.max(st.num("parapet").min(height - 0.06 - 2.3));
    let reserve = if pitched { 0.45 } else { parapet_h + 0.06 };
    let garage_storey = GARAGE_STOREY_MIN + r_garage * (GARAGE_STOREY_MAX - GARAGE_STOREY_MIN);
    let storeys = storey_stack(height, reserve, garage_storey);
    let living = storeys.len() as i64 - 1;

    let wall = if style == "tract-storybook" && r_wall_alt < 0.3 { "painted-stucco".to_string() } else { st.s("wall").unwrap_or("").to_string() };
    let front = frame.edge_for_role("front");
    let max_len = frame.edges.iter().fold(0.0f64, |m, e| m.max(e.length));
    let front_length = match front.map(|e| e.length) {
        Some(l) if l != 0.0 && !l.is_nan() => l,
        _ => {
            if max_len != 0.0 {
                max_len
            } else {
                8.0
            }
        }
    };
    let available = 0.5f64.max(front_length - 2.0 * FRONT_END_PIER_M);
    let want_garage = !retail && living >= 1;
    let stepped = if retail { None } else { find_stepped_entry(frame, front) };
    let centre_entry =
        stepped.is_none() && (if style == "marina-mediterranean" { r_centre < 0.45 } else { r_centre < 0.2 }) && available >= 8.2;
    let (lk, lw) = front_bays(available, want_garage, centre_entry, stepped.is_some());
    let notch = frame.row_house.as_ref().map(|r| r.notch.clone()).filter(|n| truthy(Some(n)));
    let notch_mid = notch.as_ref().map(|n| (n.num("s0") + n.num("s1")) / 2.0);
    let flip = !centre_entry
        && match notch_mid {
            Some(m) if lk[0] == "entry" => m > front_length / 2.0,
            _ => r_flip < 0.5,
        };
    let kinds: Vec<&str> = if flip { lk.iter().rev().copied().collect() } else { lk.clone() };
    let widths: Vec<f64> = if flip { lw.iter().rev().copied().collect() } else { lw.clone() };
    let entry_bay = kinds.iter().position(|k| *k == "entry").map(|i| i as i64).unwrap_or(-1);
    let garage_bay = kinds.iter().position(|k| *k == "garage").map(|i| i as i64).unwrap_or(-1);
    let mut picture_bay = garage_bay;
    if picture_bay < 0 {
        let mut best: i64 = -1;
        for (i, k) in kinds.iter().enumerate() {
            if *k == "window" && (best < 0 || widths[i] > widths[best as usize]) {
                best = i as i64;
            }
        }
        picture_bay = best;
    }

    let measured: Vec<J> = if !retail && living >= 1 {
        frame.row_house.as_ref().and_then(|r| r.bays.as_array().cloned()).unwrap_or_default()
    } else {
        vec![]
    };
    let canted_bays: Vec<J> = measured
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let (s0, s1, depth) = (b.num("s0"), b.num("s1"), b.num("depth"));
            let b0 = (s0 - depth).max(if i > 0 { (measured[i - 1].num("s1") + s0) / 2.0 } else { 0.05 });
            let b1 = (s1 + depth).min(if i + 1 < measured.len() { (s1 + measured[i + 1].num("s0")) / 2.0 } else { front_length - 0.05 });
            let mut o = b.clone();
            set(&mut o, "b0", jn(b0));
            set(&mut o, "b1", jn(b1));
            o
        })
        .collect();
    let bay_starts: Vec<f64> = (0..widths.len()).map(|i| FRONT_END_PIER_M + widths[..i].iter().fold(0.0, |a, b| a + b)).collect();
    let under_bay = |i: usize| {
        canted_bays
            .iter()
            .any(|cb| (bay_starts[i] + widths[i]).min(cb.num("b1")) - bay_starts[i].max(cb.num("b0")) > 0.5 * widths[i])
    };

    let ground_storefront = retail;
    let glass_block = st.truthy("glassBlock");
    let make_bay = |kind: &str, width: f64, is_picture: bool, covered: bool| -> J {
        let mut per = Map::new();
        let g = if kind == "entry" {
            json!("entrance")
        } else if kind == "garage" {
            json!("roller")
        } else if ground_storefront {
            json!("storefront")
        } else if width >= 1.4 {
            json!({ "window": "punched", "openingWidth": 1.2f64.min(width - 0.8), "sillHeight": 1.0, "head": 2.1, "mullions": 0, "transoms": 0 })
        } else {
            json!("blind")
        };
        per.insert("0".into(), g);
        for s in 1..storeys.len() {
            let head = (storeys[s] - 0.32).min(2.3);
            let ornate = s <= ORNAMENT_STOREYS;
            let v = if covered {
                json!("blind")
            } else if is_picture {
                if st.truthy("moderneBay") && s == 1 {
                    json!("blind")
                } else {
                    json!({
                        "window": "punched",
                        "openingWidth": PICTURE_MAX_M.min(width - 0.7),
                        "sillHeight": if style == "marina-mediterranean" && s == 1 { 0.35 } else { 0.75 },
                        "head": head,
                        "mullions": if ornate { st.at("pictureMullions").cloned().unwrap_or(J::Null) } else { json!(1) },
                        "transoms": if style == "richmond-row" { 1 } else { 0 },
                    })
                }
            } else if kind == "entry" {
                json!({
                    "window": "punched", "openingWidth": 0.85f64.min(width - 0.8), "sillHeight": 1.0, "head": head,
                    "mullions": if glass_block { 1 } else { 0 },
                    "transoms": if glass_block { json!(3) } else { st.at("smallTransoms").cloned().unwrap_or(J::Null) },
                })
            } else {
                json!({
                    "window": "punched", "openingWidth": 1.4f64.min(width - 0.8), "sillHeight": 0.9, "head": head,
                    "mullions": st.at("smallMullions").cloned().unwrap_or(J::Null),
                    "transoms": if s == 1 { st.at("smallTransoms").cloned().unwrap_or(J::Null) } else { json!(0) },
                })
            };
            per.insert(s.to_string(), v);
        }
        let mut bay = json!({ "width": width, "storeys": J::Object(per) });
        if kind == "entry" && glass_block {
            set(&mut bay, "glassSlot", json!("glassBlock"));
        }
        bay
    };
    let bays: Vec<J> = kinds.iter().enumerate().map(|(i, k)| make_bay(k, widths[i], i as i64 == picture_bay, under_bay(i))).collect();

    let risers = if storeys[0] >= 2.6 { 2 } else { 1 };
    let mut stair: Option<J> = None;
    if stepped.is_none() && !retail && entry_bay >= 0 && garage_bay >= 0 && living >= 1 && storeys[0] >= GARAGE_STOREY_LOW_M {
        let eb = entry_bay as usize;
        let start = FRONT_END_PIER_M + widths[..eb].iter().fold(0.0, |a, b| a + b);
        stair = fit_stair_porch(frame, front, [start, start + widths[eb]], storeys[0]);
    }
    let porch_head = if stair.is_some() { storeys[0] - 0.12 } else { (storeys[0] - 0.12).min(risers as f64 * RISER_M + DOOR_CLEAR_M + 0.08) };
    let front_elevation = json!({
        "bays": bays,
        "endPier": FRONT_END_PIER_M,
        "window": "punched",
        "reveal": st.at("reveal").cloned().unwrap_or(J::Null),
        "pierMin": 0.6,
        "sill": st.at("sill").cloned().unwrap_or(J::Null),
        "windowSpec": { "frameWidth": 0.065, "frameDepth": 0.08, "sillHeight": 0.9, "headClear": 0.35 },
        "entranceSpec": {
            "width": stair.as_ref().map(|s| s.num("width")).unwrap_or(1.1),
            "height": porch_head,
            "recess": stair.as_ref().map(|s| s.num("depth")).unwrap_or(PORCH_RECESS_M),
            "transom": 0,
            "leaf": if style == "tract-postwar" || retail { "single-glass" } else { "single-solid" },
            "frameWidth": 0.07,
        },
        "rollerSpec": { "width": GARAGE_DOOR_WIDTH, "height": GARAGE_DOOR_HEIGHT, "inset": 0.1, "hood": 0, "slat": 0.52 },
        "storefrontSpec": {
            "stallRiser": 0.45, "headClear": 0.1f64.max(storeys[0] - porch_head), "pier": 0.15,
            "mullionSpacing": 1.8, "transom": 0, "frameWidth": 0.08, "frameDepth": 0.1,
        },
        "parapet": !(pitched || st.truthy("pent")),
        "cornice": false,
    });

    let blind = json!({ "window": "blind", "parapet": false });
    let mut by_edge = Map::new();
    let mut entry_edge = front.map(|f| f.index).unwrap_or(0);
    let mut entry_bay_index = entry_bay;
    let mut stoop_outside = 0.0;
    let mut porch_recess = stair.as_ref().map(|s| s.num("depth")).unwrap_or(PORCH_RECESS_M);
    if let Some((si, setback)) = stepped {
        let sedge = &frame.edges[si];
        let front = front.unwrap();
        let run = sedge.length - 0.4;
        let entry_kinds: Vec<&str> = if run >= ENTRY_BAY_M + 1.6 { vec!["entry", "window"] } else { vec!["entry"] };
        let entry_widths: Vec<f64> = if entry_kinds.len() > 1 { vec![ENTRY_BAY_M, run - ENTRY_BAY_M] } else { vec![run] };
        let front_at_end = sedge.next_index == front.index || frame.edges.get(sedge.next_index).map(|e| e.next_index == front.index).unwrap_or(false);
        let order_k: Vec<&str> = if front_at_end { entry_kinds.iter().rev().copied().collect() } else { entry_kinds.clone() };
        let order_w: Vec<f64> = if front_at_end { entry_widths.iter().rev().copied().collect() } else { entry_widths.clone() };
        entry_edge = sedge.index;
        entry_bay_index = order_k.iter().position(|k| *k == "entry").map(|i| i as i64).unwrap_or(-1);
        porch_recess = STEPPED_PORCH_RECESS_M;
        stoop_outside = 0.3f64.max((setback - 0.15).min(0.9));
        let mut e = front_elevation.clone();
        set(&mut e, "bays", J::Array(order_k.iter().enumerate().map(|(i, k)| make_bay(k, order_w[i], false, false)).collect()));
        set(&mut e, "endPier", json!(0.2));
        let mut es = front_elevation["entranceSpec"].clone();
        set(&mut es, "recess", jn(porch_recess));
        set(&mut e, "entranceSpec", es);
        by_edge.insert(entry_edge.to_string(), e);
    }
    let mut jogs: Vec<usize> = vec![];
    for index in [front.map(|f| f.index), stepped.map(|s| s.0)] {
        let Some(edge) = index.and_then(|i| frame.edges.get(i)) else { continue };
        for j in [edge.prev_index, edge.next_index] {
            if frame.edges.get(j).map(|e| e.length < 5.0).unwrap_or(false) && !jogs.contains(&j) {
                jogs.push(j);
            }
        }
    }
    for j in jogs {
        let edge = &frame.edges[j];
        if edge.role == "front" || by_edge.contains_key(&j.to_string()) || edge.role == "front-secondary" {
            continue;
        }
        by_edge.insert(j.to_string(), json!({ "window": "blind", "parapet": !pitched }));
    }
    for edge in &frame.edges {
        if edge.role != "front-secondary" || by_edge.contains_key(&edge.index.to_string()) {
            continue;
        }
        if edge.length < CORNER_RETURN_MIN_M {
            by_edge.insert(edge.index.to_string(), json!({ "window": "blind", "parapet": !pitched && edge.length >= 1.5 }));
            continue;
        }
        by_edge.insert(
            edge.index.to_string(),
            json!({
                "window": "punched", "ground": "blind", "bayWidth": RETURN_BAY_M, "endPier": 0.8,
                "reveal": st.at("reveal").cloned().unwrap_or(J::Null), "sill": false,
                "parapet": !pitched, "cornice": false,
                "windowSpec": { "width": 1.35, "sillHeight": 0.9, "headClear": 0.4, "frameWidth": 0.065, "frameDepth": 0.08, "mullions": 0, "transoms": 0 },
            }),
        );
    }

    let pitched_rise = if pitched { 2.4f64.min(0.9f64.max(excess.map(|e| e + reserve * 0.5).unwrap_or(1.4))) } else { 0.0 };
    let gable_extra = if st.truthy("gable") && !pitched { excess.map(|e| 1.3f64.min(e)).unwrap_or(0.8) } else { 0.0 };

    let treatment_out = if canted_bays.is_empty() {
        treatment.clone()
    } else {
        let mut t = treatment.clone();
        set(&mut t, "moderneBay", json!(false));
        t
    };
    let curved = m_curved < curved_bay_share(style).unwrap_or(0.0);
    json!({
        "id": format!("sf-archetype-{}", c.seed),
        "seed": c.seed,
        "height": height,
        "storeys": storeys.iter().map(|h| json!({ "height": h })).collect::<Vec<_>>(),
        "wallThickness": WALL_THICKNESS_M,
        "parapet": { "height": if pitched { reserve } else { parapet_h }, "thickness": 0.22 },
        "coping": if pitched { json!(false) } else { json!({ "height": 0.06, "overhang": 0.04, "profile": "flat" }) },
        "roof": { "slab": 0.25, "sideFaces": false },
        "materials": {
            "wall": wall, "parapet": wall, "spandrel": wall, "recess": wall, "bulkhead": wall,
            "base": "concrete-precast", "trim": "cast-stone-trim",
            "coping": st.at("coping").cloned().unwrap_or(J::Null),
            "glass": "glass-clear", "doorGlass": "glass-door",
            "frame": st.at("frame").cloned().unwrap_or(J::Null),
            "storefront": "anodized-aluminium",
            "door": if r_door < 0.6 { "wood-siding" } else { "painted-steel" },
            "roller": if r_garage_door < 0.55 { "sectional-garage-door" } else { "wood-siding" },
            "metal": "painted-steel", "canopy": "concrete-precast", "tile": "clay-barrel-tile", "glassBlock": "glass-sky",
            "iron": "painted-steel", "veneer": "brick-buff", "roof": "asphalt-roof",
            "pitchedRoof": pitched_roof_finish(style, c.seed),
            "plant": "painted-steel",
        },
        "elevations": {
            "front": front_elevation,
            "left": blind, "right": blind, "back": blind, "front-secondary": blind, "default": blind,
            "byEdge": J::Object(by_edge),
        },
        "archetype": {
            "kind": "stucco-row",
            "version": STUCCO_ROW_VERSION,
            "lotLineBay": LOT_LINE_BAY_M,
            "curvedBays": curved,
            "gate": st.truthy("archedPorch") && !retail && m_gate < GATE_SHARE,
            "style": style,
            "treatment": treatment_out,
            "stair": stair,
            "cantedBays": canted_bays,
            "variant": variant_of(c),
            "evidence": evidence.to_json(),
            "retail": retail,
            "wall": wall,
            "paint": palette_for(style, evidence.district.as_deref(), r_paint_a, r_paint_b),
            "kinds": kinds,
            "entryEdge": entry_edge,
            "entryBay": entry_bay_index,
            "porchRecess": porch_recess,
            "stoopOutside": stoop_outside,
            "garageBay": garage_bay,
            "pictureBay": picture_bay,
            "living": living,
            "risers": risers,
            "pitched": pitched,
            "pitchedRise": pitched_rise,
            "gableExtra": gable_extra,
        },
    })
}

// ---------------------------------------------------------------------------
// Detail pass.
// ---------------------------------------------------------------------------

fn plate_args(kind: &str, slot: &str, edge: Option<usize>, points: Vec<[f64; 2]>, w0: f64, w1: f64, meta: Option<J>) -> PartArgs {
    PartArgs { kind: kind.into(), slot: slot.into(), edge, profile: points, w0, w1, meta, ..Default::default() }
}

fn prism_args(kind: &str, slot: &str, points: Vec<P2>, y0: f64, y1: f64, meta: J, cast: Option<bool>) -> PartArgs {
    PartArgs { kind: kind.into(), slot: slot.into(), points, y0, y1, meta: Some(meta), cast_shadow: cast, ..Default::default() }
}

/// `archInfill(plan, edge, { s0, s1, yTop, rise, w0, w1, meta, segments })`.
#[allow(clippy::too_many_arguments)]
pub fn arch_infill(plan: &mut Plan, edge: &Edge, s0: f64, s1: f64, y_top: f64, rise: f64, w0: f64, w1: f64, meta: J, segments: usize) -> Option<usize> {
    let half = (s1 - s0) / 2.0;
    if !(half > 0.2) {
        return None;
    }
    let h = 0.12f64.max(rise.min(half));
    let apex = y_top - 0.03;
    let r = (half * half + h * h) / (2.0 * h);
    let cy = apex - r;
    let spring = apex - h;
    let theta0 = (1f64.min(half / r)).acos();
    let mut points = vec![[s0, spring], [s0, y_top], [s1, y_top], [s1, spring]];
    let arc = arc_points((s0 + s1) / 2.0, cy, r, theta0, PI - theta0, segments);
    for p in arc.iter().take(arc.len() - 1).skip(1) {
        points.push(*p);
    }
    plan.add_plate(&edge.basis, plate_args("arch-infill", "wall", Some(edge.index), points, w0, w1, Some(meta)))
}

/// A shallow window bay with rounded front corners, as a world polygon.
fn rounded_bay(basis: &Basis, s0: f64, s1: f64, depth: f64, radius: f64, segments: usize) -> Vec<P2> {
    let mut pts = vec![on_edge(basis, s0, -0.02), on_edge(basis, s0, depth - radius)];
    for i in 1..segments {
        let a = PI - (PI / 2.0) * (i as f64 / segments as f64);
        pts.push(on_edge(basis, s0 + radius + radius * a.cos(), depth - radius + radius * a.sin()));
    }
    pts.push(on_edge(basis, s0 + radius, depth));
    pts.push(on_edge(basis, s1 - radius, depth));
    for i in 1..segments {
        let a = PI / 2.0 - (PI / 2.0) * (i as f64 / segments as f64);
        pts.push(on_edge(basis, s1 - radius + radius * a.cos(), depth - radius + radius * a.sin()));
    }
    pts.push(on_edge(basis, s1, depth - radius));
    pts.push(on_edge(basis, s1, -0.02));
    pts
}

fn key_num(v: f64) -> String {
    crate::rng::js_num(v + 0.0)
}

/// `mergeWallBands(plan)`.
fn merge_wall_bands(plan: &mut Plan) {
    let mut keys: Vec<String> = vec![];
    let mut groups: Vec<Vec<Part>> = vec![];
    let mut keep: Vec<Part> = vec![];
    for part in std::mem::take(&mut plan.parts) {
        if part.kind != "wall" || !part.is_box() || part.edge.is_none() {
            keep.push(part);
            continue;
        }
        let key = format!(
            "{}|{}|{}|{}|{}|{}",
            part.edge.unwrap(),
            part.slot,
            to_fixed(part.s0 + 0.0, 4),
            to_fixed(part.s1 + 0.0, 4),
            key_num(part.w0),
            key_num(part.w1)
        );
        match keys.iter().position(|k| *k == key) {
            Some(i) => groups[i].push(part),
            None => {
                keys.push(key);
                groups.push(vec![part]);
            }
        }
    }
    plan.parts = keep;
    for mut group in groups {
        group.sort_by(|a, b| a.y0.partial_cmp(&b.y0).unwrap_or(std::cmp::Ordering::Equal));
        // (first, y0, y1, count)
        let mut run: Option<(Part, f64, f64, usize)> = None;
        let flush = |plan: &mut Plan, run: Option<(Part, f64, f64, usize)>| {
            let Some((first, y0, y1, n)) = run else { return };
            if n == 1 {
                plan.parts.push(first);
                return;
            }
            plan.add_box(&first.basis, PartArgs::new("wall", &first.slot, first.s0, first.s1, y0, y1, first.w0, first.w1).edge_opt(first.edge));
        };
        for part in group {
            if let Some(r) = run.as_mut() {
                if part.y0 <= r.2 + 1e-3 {
                    r.2 = r.2.max(part.y1);
                    r.3 += 1;
                    continue;
                }
            }
            flush(plan, run.take());
            let (y0, y1) = (part.y0, part.y1);
            run = Some((part, y0, y1, 1));
        }
        flush(plan, run.take());
    }
}

/// Triangles one part draws (box 12; profile, plate and prism 4N-4; capless prism 2N-4).
pub fn close_part_triangles(part: &Part) -> f64 {
    match &part.shape {
        Shape::Box => 12.0,
        Shape::Prism(p) => {
            let n = p.len() as f64;
            if !part.side_faces { 2.0 * (n - 2.0) } else { 4.0 * n - 4.0 }
        }
        Shape::Plate(p) | Shape::Profile(p) => (4.0 * p.len() as f64 - 4.0).max(4.0),
    }
}

/// `stuccoRowPartTriangles`.
pub fn stucco_row_part_triangles(part: &Part) -> f64 {
    match &part.shape {
        Shape::Box => 12.0,
        Shape::Prism(p) => {
            let n = p.len() as f64;
            if !part.side_faces { 2.0 * (n - 2.0) } else { (4.0 * n - 4.0).max(4.0) }
        }
        Shape::Plate(p) | Shape::Profile(p) => (4.0 * p.len() as f64 - 4.0).max(4.0),
    }
}

fn total_triangles(plan: &Plan, f: fn(&Part) -> f64) -> f64 {
    plan.parts.iter().fold(0.0, |s, p| s + f(p))
}

/// `fitBudget(plan, frontIndex, budget)`.
fn fit_budget(plan: &mut Plan, front_index: usize, budget: f64) -> Vec<String> {
    let mut current = total_triangles(plan, stucco_row_part_triangles);
    let mut taken = vec![];
    if current <= budget {
        return taken;
    }
    let base_y = plan.base_y;
    let storey2 = plan.storeys.get(2).map(|s| s.y0).unwrap_or(f64::INFINITY);
    let on_front = |p: &Part| p.edge == Some(front_index);
    let bars = |p: &Part| p.kind == "transom" || p.kind == "mullion";
    let storey = |p: &Part| p.storey.unwrap_or(0);
    type Drop<'a> = Box<dyn Fn(&Part) -> bool + 'a>;
    let steps: Vec<(&str, Drop)> = vec![
        ("secondary-bars", Box::new(|p| bars(p) && !on_front(p))),
        ("secondary-sills", Box::new(|p| p.kind == "sill" && !on_front(p))),
        ("upper-bars", Box::new(|p| bars(p) && on_front(p) && storey(p) >= 2)),
        ("upper-eyebrows", Box::new(|p| p.kind == "eyebrow" && p.meta.s("part") == Some("window-eyebrow") && p.y0 > base_y + storey2)),
        ("balconette-bars", Box::new(|p| p.kind == "slat")),
        ("front-sills", Box::new(|p| p.kind == "sill" && on_front(p))),
        ("secondary-frames", Box::new(|p| p.kind == "frame" && !on_front(p))),
        ("front-bars", Box::new(|p| bars(p))),
        ("gate-pickets", Box::new(|p| p.kind == "gate-picket")),
        ("upper-frames", Box::new(|p| p.kind == "frame" && storey(p) >= 2)),
        ("frames", Box::new(|p| p.kind == "frame" && storey(p) >= 1)),
    ];
    for (name, drop) in steps {
        if current <= budget {
            break;
        }
        let before = plan.parts.len();
        plan.parts.retain(|p| p.meta.truthy("shopJoinery") || !drop(p));
        if plan.parts.len() != before {
            taken.push(name.to_string());
            current = total_triangles(plan, stucco_row_part_triangles);
        }
    }
    taken
}

fn record_for(plan: &Plan, index: usize) -> (Vec<BayRecord>, Vec<OpeningRecord>) {
    plan.edges.iter().find(|e| e.index == index).map(|e| (e.bays.clone(), e.openings.clone())).unwrap_or_default()
}

/// The short stoop.
#[allow(clippy::too_many_arguments)]
fn add_stoop(plan: &mut Plan, entry_edge: &Edge, entry: &OpeningRecord, entry_bay: i64, risers: f64, stoop_rise: f64, recess: f64, outside: f64, entry_allowance: f64, tag: &dyn Fn(&str) -> J) {
    for part in plan.parts.iter_mut() {
        if part.edge != Some(entry_edge.index) || part.storey != Some(0) || part.bay != Some(entry_bay) {
            continue;
        }
        if part.kind == "door-leaf" {
            if part.y1 - part.y0 < 0.5 {
                part.y1 += stoop_rise;
            }
            part.y0 += stoop_rise;
        } else if part.kind == "threshold" {
            part.y0 = entry.y0 + stoop_rise - 0.06;
            part.y1 = entry.y0 + stoop_rise + 0.02;
            part.w0 = -recess - 0.05;
            part.w1 = -recess + 0.3f64.min(recess);
        }
    }
    if let Some(ap) = plan.apertures.iter_mut().find(|a| a.s("id") == Some(entry.id.as_str())) {
        set(ap, "stoop", json!({ "risers": risers, "riser": RISER_M }));
    }
    let mut k = 1.0;
    while k <= risers {
        let reach = if outside > 0.0 {
            outside.min((risers - k + 1.0) * TREAD_M)
        } else if k == 1.0 {
            TREAD_M.min(entry_allowance - 0.04)
        } else {
            -0.02
        };
        let first = k == 1.0;
        plan.add_box(
            &entry_edge.basis,
            PartArgs::new(
                "stoop-step",
                "base",
                entry.s0 - if first { 0.04 } else { 0.0 },
                entry.s1 + if first { 0.04 } else { 0.0 },
                entry.y0 + (k - 1.0) * RISER_M - if first { 0.02 } else { 0.0 },
                entry.y0 + k * RISER_M,
                -recess,
                reach,
            )
            .edge(entry_edge.index)
            .storey(0)
            .bay(entry_bay)
            .meta(tag("stoop")),
        );
        k += 1.0;
    }
}

/// The stair porch.
fn add_stair_porch(plan: &mut Plan, entry_edge: &Edge, entry: &OpeningRecord, entry_bay: i64, stair: &J, tag: &dyn Fn(&str) -> J) {
    let risers = stair.num("risers");
    let riser = stair.num("riser");
    let tread = stair.num("tread");
    let depth = stair.num("depth");
    let rise = stair.num("rise");
    let basis = entry_edge.basis;
    let ei = entry_edge.index;
    let mine = |p: &Part| p.edge == Some(ei) && p.storey == Some(0) && p.bay == Some(entry_bay);
    plan.parts.retain(|p| !(mine(p) && ["door-leaf", "door-frame", "transom-glass", "threshold"].contains(&p.kind.as_str())));
    let lintel = 1.0f64.min(depth / 3.0);
    for p in plan.parts.iter_mut() {
        if mine(p) && p.kind == "recess" && p.y0 >= entry.y1 - 1e-3 {
            p.w0 = -lintel;
        }
    }
    let base = entry.y0;
    let top = base + rise;
    let head = top + STAIR_DOOR + 0.15;
    let mut k = 1.0;
    while k <= risers {
        plan.add_box(
            &basis,
            PartArgs::new("stoop-step", "base", entry.s0, entry.s1, base + (k - 1.0) * riser - if k == 1.0 { 0.02 } else { 0.0 }, base + k * riser, -depth, -(k - 1.0) * tread)
                .edge(ei)
                .storey(0)
                .bay(entry_bay)
                .meta(tag("stair")),
        );
        k += 1.0;
    }
    let cheek = 0.12;
    let rake = vec![[-lintel, entry.y1], [-depth - 0.1, head], [-depth - 0.1, head + 0.12], [-lintel, entry.y1 + 0.12]];
    plan.add_profile(&basis, PartArgs::profile("recess", "recess", entry.s0 - cheek, entry.s1 + cheek, rake).edge(ei).meta(tag("stair-soffit")));
    let wall = vec![[-lintel, entry.y1], [-depth - 0.1, entry.y1], [-depth - 0.1, head + 0.12], [-lintel, entry.y1 + 0.12]];
    plan.add_profile(&basis, PartArgs::profile("recess", "recess", entry.s0 - cheek, entry.s0, wall.clone()).edge(ei).meta(tag("stair-wall")));
    plan.add_profile(&basis, PartArgs::profile("recess", "recess", entry.s1, entry.s1 + cheek, wall).edge(ei).meta(tag("stair-wall")));
    plan.add_box(
        &basis,
        PartArgs::new("recess", "recess", entry.s0, entry.s1, top + STAIR_DOOR, head + 0.12, -depth - 0.1, -depth + 0.02).edge(ei).storey(0).bay(entry_bay).meta(tag("stair-head")),
    );
    let fw = 0.07;
    let w_leaf = -depth;
    for (s0, s1, y0, y1) in [
        (entry.s0, entry.s0 + fw, top, top + STAIR_DOOR),
        (entry.s1 - fw, entry.s1, top, top + STAIR_DOOR),
        (entry.s0, entry.s1, top + STAIR_DOOR - fw, top + STAIR_DOOR),
    ] {
        plan.add_box(&basis, PartArgs::new("door-frame", "door", s0, s1, y0, y1, w_leaf - 0.05, w_leaf + 0.05).edge(ei).storey(0).bay(entry_bay));
    }
    plan.add_box(
        &basis,
        PartArgs::new("door-leaf", "door", entry.s0 + fw + 0.01, entry.s1 - fw - 0.01, top + 0.02, top + STAIR_DOOR - fw - 0.01, w_leaf - 0.02, w_leaf + 0.02)
            .edge(ei)
            .storey(0)
            .bay(entry_bay)
            .meta(json!({ "aperture": entry.id, "leaf": 0, "hinge": { "s": entry.s0 + fw, "w": w_leaf }, "glazed": false })),
    );
    if let Some(ap) = plan.apertures.iter_mut().find(|a| a.s("id") == Some(entry.id.as_str())) {
        set(ap, "stoop", json!({ "risers": risers, "riser": riser }));
    }
}

fn allowance_of(e: &Edge) -> f64 {
    if e.allowance.is_finite() { e.allowance } else { 0.45 }
}

fn frame_allowance(frame: &Frame) -> f64 {
    if frame.allowance.is_finite() { frame.allowance } else { 0.45 }
}

/// The canted bay over the garage.
#[allow(clippy::too_many_arguments)]
fn add_canted_bay(plan: &mut Plan, frame: &Frame, front: &Edge, bay: &J, lift: f64, storeys: &[crate::elevation::Storey], wall_top: f64, tag: &dyn Fn(&str) -> J) {
    let (s0, s1) = (bay.num("s0"), bay.num("s1"));
    let depth = bay.num("depth").min(frame_allowance(frame) - 0.06).min(allowance_of(front) - 0.06);
    if !(depth >= 0.2) {
        return;
    }
    let l = front.length;
    let b0 = 0.05f64.max(bay.num("b0")).max(s0 - depth);
    let b1 = (l - 0.05).min(bay.num("b1")).min(s1 + depth);
    let f0 = (b0 + 0.05).max(s0);
    let f1 = (b1 - 0.05).min(s1);
    if !(f1 - f0 > 0.8) {
        return;
    }
    let y_base = (lift + BAY_HEADROOM_M).max(lift + storeys[1].y0 - 0.15);
    let fb = front.basis;
    let outline = |grow: f64| {
        vec![on_edge(&fb, b0 - grow, -0.05), on_edge(&fb, f0 - grow * 0.4, depth + grow), on_edge(&fb, f1 + grow * 0.4, depth + grow), on_edge(&fb, b1 + grow, -0.05)]
    };
    plan.add_prism(prism_args("bay", "wall", outline(0.0), y_base, wall_top, tag("canted-bay"), None));
    plan.add_prism(prism_args("bay-sill", "trim", outline(0.025), y_base - 0.06, y_base + 0.04, tag("canted-bay-sill"), None));
    let corners = [on_edge(&fb, b0, 0.0), on_edge(&fb, f0, depth), on_edge(&fb, f1, depth), on_edge(&fb, b1, 0.0)];
    for f in 0..3 {
        let a = corners[f];
        let b = corners[f + 1];
        let len = (b.x - a.x).hypot(b.z - a.z);
        let u = P2::new((b.x - a.x) / len, (b.z - a.z) / len);
        let basis = Basis { origin: [a.x, a.z], u: [u.x, u.z], n: [-u.z, u.x] };
        let margin = if f == 1 { 0.16 } else { 0.12 };
        let w0 = margin;
        let w1 = len - margin;
        if w1 - w0 < 0.45 {
            continue;
        }
        for (s, st) in storeys.iter().enumerate().skip(1) {
            let y_sill = (y_base + 0.25).max(lift + st.y0 + 0.75);
            let y_head = lift + st.y1 - 0.32;
            if !(y_head - y_sill > 0.8) {
                continue;
            }
            let m = tag("canted-bay-window");
            let si = s as i64;
            plan.add_box(&basis, PartArgs::new("glass", "glass", w0, w1, y_sill, y_head, 0.002, 0.012).edge(front.index).storey(si).meta(m.clone()).shadow(false));
            let fw = 0.07;
            for (p0, p1, q0, q1) in [(w0, w0 + fw, y_sill, y_head), (w1 - fw, w1, y_sill, y_head), (w0, w1, y_head - fw, y_head), (w0, w1, y_sill, y_sill + fw)] {
                plan.add_box(&basis, PartArgs::new("frame", "frame", p0, p1, q0, q1, 0.0, 0.05).edge(front.index).storey(si).meta(m.clone()));
            }
            plan.add_box(&basis, PartArgs::new("sill", "trim", w0 - 0.04, w1 + 0.04, y_sill - 0.06, y_sill, 0.0, 0.06).edge(front.index).storey(si).meta(m.clone()));
            if f == 1 {
                for t in [1.0 / 3.0, 2.0 / 3.0] {
                    let mm = w0 + (w1 - w0) * t;
                    plan.add_box(&basis, PartArgs::new("mullion", "frame", mm - 0.035, mm + 0.035, y_sill, y_head, 0.0, 0.045).edge(front.index).storey(si).meta(m.clone()));
                }
            }
        }
    }
}

/// The measured bay drawn curved.
#[allow(clippy::too_many_arguments)]
fn add_curved_bay(plan: &mut Plan, frame: &Frame, front: &Edge, bay: &J, lift: f64, storeys: &[crate::elevation::Storey], wall_top: f64, tag: &dyn Fn(&str) -> J) {
    let depth = bay.num("depth").min(frame_allowance(frame) - 0.06).min(allowance_of(front) - 0.06);
    if !(depth >= 0.2) {
        return;
    }
    let l = front.length;
    let b0 = 0.05f64.max(bay.num("b0")).max(bay.num("s0") - depth * 0.5);
    let b1 = (l - 0.05).min(bay.num("b1")).min(bay.num("s1") + depth * 0.5);
    if !(b1 - b0 > 1.6) {
        return;
    }
    let radius = (depth - 0.02).min((b1 - b0) / 4.0).min(0.6);
    let fb = front.basis;
    let solid = rounded_bay(&fb, b0, b1, depth, radius, 4);
    let glass = rounded_bay(&fb, b0 + 0.04, b1 - 0.04, depth - 0.03, 0.05f64.max(radius - 0.03), 4);
    let y_base = (lift + BAY_HEADROOM_M).max(lift + storeys[1].y0 - 0.15);
    plan.add_prism(prism_args(
        "bay-sill",
        "trim",
        rounded_bay(&fb, b0 - 0.03, b1 + 0.03, depth + 0.025, radius + 0.025, 4),
        y_base - 0.06,
        y_base + 0.04,
        tag("curved-bay-sill"),
        None,
    ));
    let mut y = y_base;
    for (s, st) in storeys.iter().enumerate().skip(1) {
        let y_sill = (y_base + 0.25).max(lift + st.y0 + 0.75);
        let y_head = lift + st.y1 - 0.32;
        if !(y_head - y_sill > 0.8) {
            continue;
        }
        plan.add_prism(prism_args("bay", "wall", solid.clone(), y, y_sill, tag("curved-bay-apron"), None));
        plan.add_prism(prism_args("bay", "glass", glass.clone(), y_sill, y_head, tag("curved-bay-glass"), Some(false)));
        for t in [1.0 / 3.0, 2.0 / 3.0] {
            let sx = b0 + radius + (b1 - b0 - 2.0 * radius) * t;
            plan.add_box(&fb, PartArgs::new("mullion", "frame", sx - 0.03, sx + 0.03, y_sill, y_head, depth - 0.05, depth - 0.015).edge(front.index).storey(s as i64).meta(tag("curved-bay-bar")));
        }
        y = y_head;
    }
    if wall_top - y > 0.02 {
        plan.add_prism(prism_args("bay", "wall", solid, y, wall_top, tag("curved-bay-head"), None));
    }
}

/// An iron gate across the arched porch.
fn add_entry_gate(plan: &mut Plan, edge: &Edge, entry: &OpeningRecord, lift: f64, tag: &dyn Fn(&str) -> J) {
    let s0 = entry.s0 + 0.04;
    let s1 = entry.s1 - 0.04;
    let y0 = entry.y0.min(lift) + 0.04;
    let y1 = entry.y1 - PORCH_ARCH_RISE_M;
    if !(s1 - s0 > 0.6) || !(y1 - y0 > 1.4) {
        return;
    }
    let (w0, w1) = (0.005, 0.005 + GATE_BAR_M);
    let count = 2f64.max(js_round((s1 - s0) / GATE_BAR_PITCH_M)) as i64;
    for i in 1..count {
        let sx = s0 + ((s1 - s0) * i as f64) / count as f64;
        plan.add_box(&edge.basis, PartArgs::new("gate-picket", "iron", sx - GATE_BAR_M / 2.0, sx + GATE_BAR_M / 2.0, y0, y1, w0, w1).edge(edge.index).meta(tag("entry-gate")));
    }
    for yr in [y0, (y0 + y1) / 2.0, y1 - 0.05] {
        plan.add_box(&edge.basis, PartArgs::new("gate-rail", "iron", s0, s1, yr, yr + 0.05, w0, w1).edge(edge.index).meta(tag("entry-gate")));
    }
    for sx in [s0, s1 - 0.05] {
        plan.add_box(&edge.basis, PartArgs::new("gate-post", "iron", sx, sx + 0.05, y0, y1, w0, w1).edge(edge.index).meta(tag("entry-gate")));
    }
}

/// `addStuccoRowDetail(plan, frame, spec)`.
pub fn detail(plan: &mut Plan, frame: &Frame, spec: &J) {
    let Some(meta) = spec.get_nn("archetype").filter(|m| truthy(Some(m))).cloned() else { return };
    let Some(front) = frame.edge_for_role("front") else { return };
    let st = style_of(&meta);
    let (bays, openings) = record_for(plan, front.index);
    let lift = if plan.base_y.is_finite() { plan.base_y } else { 0.0 };
    let storeys = plan.storeys.clone();
    let wall_top = lift + plan.envelope.wall_top;
    let roof_top = lift + plan.envelope.roof_top;
    let allowance = allowance_of(front);
    let fit = |want: f64| 0.05f64.max(want.min(allowance - 0.04));
    let kind = meta.s("kind").unwrap_or("").to_string();
    let style = meta.s("style").unwrap_or("").to_string();
    let tag_fn = |part: &str| json!({ "archetype": kind, "style": style, "part": part });
    let tag: &dyn Fn(&str) -> J = &tag_fn;
    let opening_at = |storey: usize, bay: usize, type_: &str| openings.iter().find(|o| o.storey == storey && o.bay == bay && o.type_ == type_).cloned();
    let wt = plan.envelope.wall_thickness;

    // 1. The stoop.
    let entry_edge = meta.f("entryEdge").and_then(|i| frame.edges.get(i as usize)).unwrap_or(front);
    let entry_bay = meta.f("entryBay").unwrap_or(-1.0);
    let (_, entry_openings) = record_for(plan, entry_edge.index);
    let entry = if entry_bay >= 0.0 {
        entry_openings.iter().find(|o| o.storey == 0 && o.bay as f64 == entry_bay && o.type_ == "entrance").cloned()
    } else {
        None
    };
    let risers = meta.f("risers").unwrap_or(2.0);
    let stoop_rise = RISER_M * risers;
    let recess = meta.f("porchRecess").unwrap_or(PORCH_RECESS_M);
    let outside = meta.f("stoopOutside").unwrap_or(0.0);
    let entry_allowance = allowance_of(entry_edge);
    if let Some(entry) = &entry {
        let eb = entry_bay as i64;
        match meta.get_nn("stair").filter(|s| truthy(Some(s))) {
            Some(stair) => add_stair_porch(plan, entry_edge, entry, eb, stair, tag),
            None => add_stoop(plan, entry_edge, entry, eb, risers, stoop_rise, recess, outside, entry_allowance, tag),
        }
        if st.truthy("archedPorch") {
            arch_infill(plan, entry_edge, entry.s0, entry.s1, entry.y1, PORCH_ARCH_RISE_M, -wt, 0.005, tag("porch-arch"), 8);
        }
        if st.truthy("archedPorch") && meta.truthy("gate") {
            add_entry_gate(plan, entry_edge, entry, lift, tag);
        }
        if st.truthy("entryHood") {
            let profile = trim_profile_d("sloped", 0.05f64.max(0.32f64.min(entry_allowance - 0.04)), 0.24, entry.y1 + 0.12);
            plan.add_profile(&entry_edge.basis, PartArgs::profile("eyebrow", "tile", entry.s0 - 0.3, entry.s1 + 0.3, profile).edge(entry_edge.index).meta(tag("entry-hood")));
        }
        if st.truthy("eyebrow") {
            plan.add_box(
                &entry_edge.basis,
                PartArgs::new("canopy", "canopy", entry.s0 - 0.25, entry.s1 + 0.25, entry.y1 + 0.06, entry.y1 + 0.2, -0.02, 0.8)
                    .edge(entry_edge.index)
                    .shadow(true)
                    .meta(tag("entry-eyebrow")),
            );
        }
    }

    // 2. Garage head.
    let garage_bay = meta.f("garageBay").unwrap_or(-1.0);
    let garage = if garage_bay >= 0.0 { opening_at(0, garage_bay as usize, "roller") } else { None };
    if let Some(g) = &garage {
        if st.truthy("archedGarage") {
            arch_infill(plan, front, g.s0, g.s1, g.y1, 0.32, -wt, 0.005, tag("garage-arch"), 8);
        } else {
            let profile = trim_profile_d("flat", fit(0.05), 0.12, g.y1 + 0.02);
            plan.add_profile(&front.basis, PartArgs::profile("string-course", "trim", g.s0 - 0.08, g.s1 + 0.08, profile).edge(front.index).meta(tag("garage-head")));
        }
    }

    // 3. Postwar veneer.
    let retail = meta.truthy("retail");
    let entry_edge_meta = meta.f("entryEdge");
    if st.truthy("veneer") && !retail {
        for (i, bay) in bays.iter().enumerate() {
            if i as f64 == garage_bay || (i as f64 == entry_bay && entry_edge_meta == Some(front.index as f64)) {
                continue;
            }
            plan.add_box(
                &front.basis,
                PartArgs::new("veneer", "veneer", bay.s0, bay.s1, lift + storeys[0].y0, lift + storeys[0].y0 + 0.95, -0.01, 0.05)
                    .edge(front.index)
                    .storey(0)
                    .bay(i as i64)
                    .meta(tag("veneer")),
            );
        }
    }

    // 4. Upper storeys.
    let picture_bay = meta.f("pictureBay").unwrap_or(-1.0);
    let pitched = meta.truthy("pitched");
    let reveal = st.num("reveal");
    for s in 1..storeys.len() {
        let ornate = s <= ORNAMENT_STOREYS;
        let top = s == storeys.len() - 1;
        for b in 0..bays.len() {
            let Some(op) = opening_at(s, b, "punched") else { continue };
            let is_picture = b as f64 == picture_bay;
            let arched = ornate && ((is_picture && s == 1 && st.truthy("archedPicture")) || (st.truthy("archedTop") && top));
            if arched {
                arch_infill(plan, front, op.s0, op.s1, op.y1, if is_picture { 0.5 } else { 0.45 }, -(reveal - 0.03), 0.005, tag("window-arch"), 8);
            }
            let under_eave = pitched && op.y1 + 0.4 > wall_top - 0.12;
            if ornate && !under_eave && (st.truthy("eyebrows") || (st.truthy("pictureHood") && is_picture && s == 1)) {
                let profile = trim_profile_d("sloped", fit(0.3), 0.22, op.y1 + 0.06);
                plan.add_profile(&front.basis, PartArgs::profile("eyebrow", "tile", op.s0 - 0.18, op.s1 + 0.18, profile).edge(front.index).meta(tag("window-eyebrow")));
            }
            if st.truthy("balconette") && is_picture && s == 1 {
                let sa = op.s0 - 0.08;
                let sb = op.s1 + 0.08;
                let d = fit(0.26);
                let yb = op.y0 + 0.02;
                let yt = (op.y0 + 0.92).min(op.y1 - 0.2);
                let fb = front.basis;
                let add = |plan: &mut Plan, kind: &str, a: [f64; 6], m: J, cast: Option<bool>| {
                    let mut pa = PartArgs::new(kind, "iron", a[0], a[1], a[2], a[3], a[4], a[5]).edge(front.index).storey(s as i64).bay(b as i64).meta(m);
                    pa.cast_shadow = cast;
                    plan.add_box(&fb, pa);
                };
                add(plan, "balconette", [sa, sb, yb - 0.08, yb, -0.02, d], tag("balconette"), None);
                add(plan, "balconette", [sa, sb, yt - 0.04, yt, d - 0.04, d], tag("balconette"), None);
                for sx in [sa, sb - 0.04] {
                    add(plan, "balconette", [sx, sx + 0.04, yb, yt, d - 0.04, d], tag("balconette"), None);
                }
                let bars = 2f64.max(9f64.min(js_round((sb - sa) / 0.3) - 1.0)) as i64;
                for k in 1..=bars {
                    let sx = sa + ((sb - sa) * k as f64) / (bars + 1) as f64;
                    add(plan, "slat", [sx - 0.012, sx + 0.012, yb, yt - 0.04, d - 0.03, d - 0.006], tag("balconette-bar"), Some(false));
                }
            }
        }
    }

    // 5. Moderne bay.
    if st.truthy("moderneBay") && storeys.len() > 1 && picture_bay >= 0.0 && bays.get(picture_bay as usize).is_some() {
        let bay = &bays[picture_bay as usize];
        let width = (PICTURE_MAX_M + 0.4).min(bay.width - 0.3);
        let centre = (bay.s0 + bay.s1) / 2.0;
        let s0 = centre - width / 2.0;
        let s1 = centre + width / 2.0;
        let depth = 0.15f64.max(MODERNE_BAY_DEPTH_M.min(frame_allowance(frame) - 0.04).min(allowance - 0.04));
        let radius = 0.36f64.min(depth - 0.02);
        let storey = &storeys[1];
        let y0 = lift + storey.y0 + 0.1;
        let y_sill = lift + storey.y0 + 0.8;
        let y_head = lift + storey.y1 - 0.38;
        let y1 = lift + storey.y1 - 0.12;
        if y_head - y_sill > 0.6 && width > 2.0 * radius + 0.4 {
            let solid = rounded_bay(&front.basis, s0, s1, depth, radius, 3);
            let glass = rounded_bay(&front.basis, s0 + 0.03, s1 - 0.03, depth - 0.035, 0.05f64.max(radius - 0.035), 3);
            plan.add_prism(prism_args("bay", "wall", solid.clone(), y0, y_sill, tag("moderne-bay-apron"), None));
            plan.add_prism(prism_args("bay", "glass", glass, y_sill, y_head, tag("moderne-bay-glass"), Some(false)));
            plan.add_prism(prism_args("bay", "wall", solid, y_head, y1, tag("moderne-bay-head"), None));
            for t in [1.0 / 3.0, 2.0 / 3.0] {
                let sx = s0 + radius + (width - 2.0 * radius) * t;
                plan.add_box(
                    &front.basis,
                    PartArgs::new("mullion", "frame", sx - 0.03, sx + 0.03, y_sill, y_head, depth - 0.05, depth - 0.015)
                        .edge(front.index)
                        .storey(1)
                        .bay(picture_bay as i64)
                        .meta(tag("moderne-bay-bar")),
                );
            }
        }
    }

    // 5b. Measured bays.
    if storeys.len() > 1 {
        let canted = meta.arr("cantedBays").cloned().unwrap_or_default();
        for bay in &canted {
            if meta.truthy("curvedBays") {
                add_curved_bay(plan, frame, front, bay, lift, &storeys, wall_top, tag);
            } else {
                add_canted_bay(plan, frame, front, bay, lift, &storeys, wall_top, tag);
            }
        }
    }

    if st.truthy("bands") && storeys.len() > 1 {
        let storey = &storeys[1];
        let y_base = lift + storey.y1 - 0.62;
        let bay = if picture_bay >= 0.0 { bays.get(picture_bay as usize) } else { None };
        let runs: Vec<(f64, f64)> = match bay {
            Some(b) => vec![(0.05, b.s0), (b.s1, front.length - 0.05)],
            None => vec![(0.05, front.length - 0.05)],
        };
        for (a, b) in runs {
            if b - a < 0.6 {
                continue;
            }
            for k in 0..3 {
                let profile = trim_profile_d("flat", fit(0.04), 0.05, y_base + k as f64 * 0.13);
                plan.add_profile(&front.basis, PartArgs::profile("string-course", "wall", a, b, profile).edge(front.index).meta(tag("moderne-band")));
            }
        }
    }

    if st.truthy("band") {
        let profile = trim_profile_d("flat", fit(0.05), 0.14, wall_top - 0.3);
        plan.add_profile(&front.basis, PartArgs::profile("string-course", "wall", 0.0, front.length, profile).edge(front.index).meta(tag("parapet-band")));
    }

    // 6. Roof treatment.
    let l = front.length;
    let ov = fit(0.3);
    let front_z = frame.to_plan(front.start.x, front.start.z).z;
    let inward = 0.6f64.max(front_z - frame.plan_bounds.min_z - 0.3);
    if pitched {
        let rise = meta.num("pitchedRise");
        let run = (inward * 0.7).min(1.6f64.max(5.0f64.min(rise / (22.0 * PI / 180.0).tan()).min(inward * 0.45)));
        let back = 0.05f64.max((run * 0.6).min(inward - run));
        let mut m = tag("front-roof");
        set(&mut m, "roof", json!(true));
        plan.add_profile(
            &front.basis,
            PartArgs::profile("roof-slope", "pitchedRoof", 0.0, l, vec![[ov, wall_top - 0.12], [ov, wall_top + 0.04], [-run, wall_top + rise], [-run - back, wall_top], [0.0, wall_top - 0.12]])
                .edge(front.index)
                .meta(m),
        );
    } else if st.truthy("pent") {
        let rise = roof_top - 0.02 - wall_top;
        let run = inward.min(1.0f64.max(2.2f64.min(rise / (32.0 * PI / 180.0).tan())));
        plan.add_profile(
            &front.basis,
            PartArgs::profile("roof-slope", "tile", 0.0, l, vec![[ov, wall_top - 0.1], [ov, wall_top + 0.04], [-run, wall_top + rise], [-run, wall_top], [0.0, wall_top - 0.1]])
                .edge(front.index)
                .meta(tag("front-pent")),
        );
        if entry_edge.index != front.index {
            let e_z = frame.to_plan(entry_edge.start.x, entry_edge.start.z).z;
            let e_run = run.min(0.6f64.max(e_z - frame.plan_bounds.min_z - 0.3));
            plan.add_profile(
                &entry_edge.basis,
                PartArgs::profile(
                    "roof-slope",
                    "tile",
                    0.0,
                    entry_edge.length,
                    vec![[ov, wall_top - 0.1], [ov, wall_top + 0.04], [-e_run, wall_top + rise], [-e_run, wall_top], [0.0, wall_top - 0.1]],
                )
                .edge(entry_edge.index)
                .meta(tag("entry-pent")),
            );
        }
    }

    if st.truthy("gable") && meta.num("gableExtra") >= 0.35 && storeys.len() > 1 {
        let bay = if picture_bay >= 0.0 { bays.get(picture_bay as usize) } else { None };
        let centre = bay.map(|b| (b.s0 + b.s1) / 2.0).unwrap_or(l / 2.0);
        let half = (bay.map(|b| b.width).unwrap_or(l) / 2.0 + 0.5).min(centre - 0.15).min(l - centre - 0.15);
        if half > 1.0 {
            let apex = roof_top + meta.num("gableExtra");
            let shoulder = roof_top;
            let inner = half * 0.82;
            let mut gm = tag("false-gable");
            set(&mut gm, "roof", json!(true));
            plan.add_plate(
                &front.basis,
                plate_args(
                    "gable",
                    "wall",
                    Some(front.index),
                    vec![
                        [centre - half, wall_top],
                        [centre + half, wall_top],
                        [centre + half, shoulder],
                        [centre + inner, shoulder],
                        [centre, apex],
                        [centre - inner, shoulder],
                        [centre - half, shoulder],
                    ],
                    -0.22,
                    0.03,
                    Some(gm.clone()),
                ),
            );
            for sign in [-1.0, 1.0] {
                let a = [centre + sign * (inner + 0.06), shoulder - 0.02];
                let c = [centre, apex + 0.02];
                plan.add_plate(&front.basis, plate_args("gable-rake", "tile", Some(front.index), vec![a, c, [c[0], c[1] + 0.12], [a[0], a[1] + 0.12]], -0.24, 0.07, Some(gm.clone())));
            }
        }
    }

    if pitched || st.truthy("pent") {
        plan.parts.retain(|p| {
            !((p.kind == "parapet" || p.kind == "coping")
                && p.edge != Some(front.index)
                && p.edge.and_then(|e| frame.edges.get(e)).map(|e| e.role) != Some("front-secondary"))
        });
    }
    merge_wall_bands(plan);
    let budget_drops = fit_budget(plan, front.index, STUCCO_ROW_TRIANGLE_BUDGET - ROOF_PLANT_RESERVE_TRI);
    let close = add_stucco_row_close_detail(plan, frame, &meta, front, entry_edge, entry.as_ref(), garage.as_ref(), wall_top, roof_top, lift);

    let mut tints = Map::new();
    tints.insert(meta.s("wall").unwrap_or("").to_string(), meta.at("paint").cloned().unwrap_or(J::Null));
    plan.extra.insert("tints".into(), J::Object(tints));
    let (ox, oz) = (front.basis.origin[0], front.basis.origin[1]);
    let (ux, uz) = (front.basis.u[0], front.basis.u[1]);
    let (nx, nz) = (front.basis.n[0], front.basis.n[1]);
    let roof_plant_triangles = ROOF_PLANT_RESERVE_TRI.max(STUCCO_ROW_CLOSE_TRIANGLE_BUDGET - total_triangles(plan, close_part_triangles));
    let stair = meta.get_nn("stair").filter(|s| truthy(Some(s))).map(|s| json!({ "risers": s.at("risers"), "depth": s.at("depth") }));
    let garage_j = garage.as_ref().map(|g| {
        let centre = (g.s0 + g.s1) / 2.0;
        json!({ "edge": front.index, "width": g.s1 - g.s0, "world": { "x": ox + ux * centre, "z": oz + uz * centre }, "outward": [nx, nz] })
    });
    plan.extra.insert(
        "archetype".into(),
        json!({
            "kind": meta.at("kind"),
            "version": meta.at("version"),
            "style": meta.at("style"),
            "treatment": meta.at("treatment"),
            "stair": stair,
            "variant": meta.at("variant"),
            "retail": meta.at("retail"),
            "pitched": meta.at("pitched"),
            "bayCount": bays.len(),
            "paint": meta.at("paint"),
            "budgetDrops": budget_drops,
            "close": close,
            "roofPlantTriangles": roof_plant_triangles,
            "garage": garage_j,
        }),
    );
}

// ---------------------------------------------------------------------------
// Close-range detail (LOD0 only).
// ---------------------------------------------------------------------------

/// Close-detail part kinds, each one the generic ladder drops at LOD1.
#[allow(non_snake_case)]
pub mod CLOSE_KIND {
    pub const BAR: &str = "mullion";
    pub const RAIL: &str = "transom";
    pub const TRIM: &str = "jamb";
    pub const FIXTURE: &str = "dentil";
    pub const BRACKET: &str = "corbel";
    pub const COURSE: &str = "slat";
}
use CLOSE_KIND as CK;

pub const PEDESTRIAN_HEADROOM_M: f64 = 2.29;
const FLUSH_M: f64 = 0.006;

/// The edge a close part is drawn on: a real edge, or a pseudo-edge (basis only).
#[derive(Clone, Copy, Debug)]
pub struct CEdge {
    pub index: Option<usize>,
    pub basis: Basis,
    /// `edge.outward !== false`: below headroom the part is pulled flush.
    pub outward: bool,
}

impl From<&Edge> for CEdge {
    fn from(e: &Edge) -> Self {
        CEdge { index: Some(e.index), basis: e.basis, outward: true }
    }
}

impl CEdge {
    /// `basisEdge(basis, outward)` in cottage-garage.js.
    pub fn basis(basis: Basis, outward: bool) -> Self {
        CEdge { index: None, basis, outward }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Out {
    W,
    S,
    None,
}

/// A drawing context for one building's close-detail pass (`closeContext`).
pub struct Cx {
    pub group: String,
    archetype: String,
    floor: f64,
    used: HashSet<String>,
}

/// `closeContext(plan, archetype)`.
pub fn close_context(plan: &Plan, archetype: &str) -> Cx {
    let base = if plan.base_y.is_finite() { plan.base_y } else { 0.0 };
    let mut used = HashSet::new();
    for part in &plan.parts {
        let name = plan.materials.get(&part.slot).filter(|s| !s.is_empty()).or(plan.materials.get("wall"));
        if let Some(n) = name {
            used.insert(n.clone());
        }
    }
    Cx { group: "detail".into(), archetype: archetype.into(), floor: base + PEDESTRIAN_HEADROOM_M, used }
}

fn flag(extra: Option<&J>, key: &str) -> bool {
    extra.map(|e| e.truthy(key)).unwrap_or(false)
}

impl Cx {
    /// The first preferred slot whose material the plan already draws.
    pub fn slot(&self, plan: &Plan, prefs: &[&str]) -> String {
        for slot in prefs {
            if let Some(name) = plan.materials.get(*slot) {
                if !name.is_empty() && self.used.contains(name) {
                    return slot.to_string();
                }
            }
        }
        "wall".into()
    }

    pub fn meta(&self, part: &str, extra: Option<&J>) -> J {
        let base = json!({ "archetype": self.archetype, "part": part, "close": self.group, "lod": 0 });
        spread(Some(&base), extra)
    }

    fn keep(&self, plan: &mut Plan, idx: Option<usize>, extra: Option<&J>, outward: Out) -> Option<usize> {
        let i = idx?;
        let p = &mut plan.parts[i];
        let fitting = flag(extra, "fitting");
        if fitting {
            p.fitting = true;
            if let Some(m) = p.meta.as_object_mut() {
                m.shift_remove("fitting");
            }
        }
        if p.meta.truthy("stoop") {
            if let Some(m) = p.meta.as_object_mut() {
                m.shift_remove("stoop");
            }
        }
        if outward != Out::None && !fitting && !flag(extra, "stoop") && p.y0 < self.floor - 1e-6 {
            match outward {
                Out::W => {
                    if !matches!(p.shape, Shape::Prism(_)) && p.w1 > FLUSH_M {
                        let d = p.w1 - FLUSH_M;
                        p.w0 -= d;
                        p.w1 -= d;
                        if let Shape::Profile(pr) = &mut p.shape {
                            for q in pr.iter_mut() {
                                q[0] -= d;
                            }
                        }
                    }
                }
                Out::S => {
                    if p.s1 > FLUSH_M {
                        let d = p.s1 - FLUSH_M;
                        p.s0 -= d;
                        p.s1 -= d;
                        if let Shape::Plate(pts) = &mut p.shape {
                            for q in pts.iter_mut() {
                                q[0] -= d;
                            }
                        }
                    }
                }
                Out::None => {}
            }
        }
        Some(i)
    }

    fn out_of(e: &CEdge, o: Out) -> Out {
        if e.outward { o } else { Out::None }
    }

    /// `cx.box(edge, kind, slot, part, { s0, s1, y0, y1, w0, w1 }, extra)`.
    #[allow(clippy::too_many_arguments)]
    pub fn bx(&self, plan: &mut Plan, e: impl Into<CEdge>, kind: &str, slot: &str, part: &str, b: [f64; 6], extra: Option<&J>) -> Option<usize> {
        let e: CEdge = e.into();
        let idx = plan.add_box(&e.basis, PartArgs::new(kind, slot, b[0], b[1], b[2], b[3], b[4], b[5]).edge_opt(e.index).meta(self.meta(part, extra)));
        self.keep(plan, idx, extra, Self::out_of(&e, Out::W))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn profile(&self, plan: &mut Plan, e: impl Into<CEdge>, kind: &str, slot: &str, part: &str, s0: f64, s1: f64, profile: Vec<[f64; 2]>, extra: Option<&J>) -> Option<usize> {
        let e: CEdge = e.into();
        let idx = plan.add_profile(&e.basis, PartArgs::profile(kind, slot, s0, s1, profile).edge_opt(e.index).meta(self.meta(part, extra)));
        self.keep(plan, idx, extra, Self::out_of(&e, Out::W))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn plate(&self, plan: &mut Plan, e: impl Into<CEdge>, kind: &str, slot: &str, part: &str, points: Vec<[f64; 2]>, w0: f64, w1: f64, extra: Option<&J>) -> Option<usize> {
        let e: CEdge = e.into();
        let idx = plan.add_plate(&e.basis, plate_args(kind, slot, e.index, points, w0, w1, Some(self.meta(part, extra))));
        self.keep(plan, idx, extra, Self::out_of(&e, Out::W))
    }

    /// A bar in the vertical plane through the wall normal: `points` are [w, y].
    #[allow(clippy::too_many_arguments)]
    pub fn section(&self, plan: &mut Plan, e: impl Into<CEdge>, kind: &str, slot: &str, part: &str, points: Vec<[f64; 2]>, s0: f64, s1: f64, extra: Option<&J>) -> Option<usize> {
        let e: CEdge = e.into();
        let u = e.basis.n;
        let basis = Basis { origin: e.basis.origin, u, n: [-u[1], u[0]] };
        let idx = plan.add_plate(&basis, plate_args(kind, slot, e.index, points, -s1, -s0, Some(self.meta(part, extra))));
        self.keep(plan, idx, extra, Self::out_of(&e, Out::S))
    }

    /// A vertical prism over a section given as [s, w] pairs on an edge.
    #[allow(clippy::too_many_arguments)]
    pub fn column(&self, plan: &mut Plan, e: impl Into<CEdge>, kind: &str, slot: &str, part: &str, section_sw: Vec<[f64; 2]>, y0: f64, y1: f64, extra: Option<&J>) -> Option<usize> {
        let e: CEdge = e.into();
        let b = e.basis;
        let proud = section_sw.iter().fold(f64::NEG_INFINITY, |m, q| m.max(q[1]));
        let d = if y0 < self.floor - 1e-6 && !flag(extra, "fitting") && proud > FLUSH_M { proud - FLUSH_M } else { 0.0 };
        let points: Vec<P2> = section_sw
            .iter()
            .map(|[s, w]| P2::new(b.origin[0] + b.u[0] * s + b.n[0] * (w - d), b.origin[1] + b.u[1] * s + b.n[1] * (w - d)))
            .collect();
        let idx = plan.add_prism(prism_args(kind, slot, points, y0, y1, self.meta(part, extra), None));
        self.keep(plan, idx, extra, Out::None)
    }

    /// `cx.keep(addPrismPart(plan, { kind, slot, points, y0, y1, meta: cx.meta(part) }))`.
    #[allow(clippy::too_many_arguments)]
    pub fn prism(&self, plan: &mut Plan, kind: &str, slot: &str, part: &str, points: Vec<P2>, y0: f64, y1: f64) -> Option<usize> {
        let idx = plan.add_prism(prism_args(kind, slot, points, y0, y1, self.meta(part, None), None));
        self.keep(plan, idx, None, Out::W)
    }
}

/// `fitCloseBudget(plan, budget, dropOrder)`.
pub fn fit_close_budget(plan: &mut Plan, budget: f64, drop_order: &[&str]) -> Vec<String> {
    let mut current = total_triangles(plan, close_part_triangles);
    let mut taken = vec![];
    for group in drop_order {
        if current <= budget {
            break;
        }
        let before = plan.parts.len();
        plan.parts.retain(|p| p.meta.s("close") != Some(*group));
        if plan.parts.len() != before {
            taken.push(group.to_string());
            current = total_triangles(plan, close_part_triangles);
        }
    }
    taken
}

/// The glazed lights of one punched opening (`openingLights`).
pub struct Lights {
    pub w_at: f64,
    pub framed: bool,
    pub frame_width: f64,
    pub cols: Vec<[f64; 2]>,
    pub rows: Vec<[f64; 2]>,
}

/// `openingLights(plan, edgeIndex, op)`.
pub fn opening_lights(plan: &Plan, edge_index: usize, op: &OpeningRecord) -> Option<Lights> {
    let eps = 1e-3;
    let inside = |p: &Part| {
        p.edge == Some(edge_index) && p.is_box() && p.s0 >= op.s0 - eps && p.s1 <= op.s1 + eps && p.y0 >= op.y0 - eps && p.y1 <= op.y1 + eps
    };
    let glass = plan.parts.iter().find(|p| p.kind == "glass" && p.meta.s("opening") == Some(op.id.as_str()))?;
    let frames: Vec<&Part> = plan.parts.iter().filter(|p| p.kind == "frame" && inside(p)).collect();
    let fw = if frames.is_empty() { 0.0 } else { frames.iter().fold(f64::INFINITY, |m, p| m.min((p.s1 - p.s0).min(p.y1 - p.y0))) };
    let w_at = glass.w1;
    let height = op.y1 - op.y0;
    let mut mullions: Vec<[f64; 2]> = plan
        .parts
        .iter()
        .filter(|p| p.kind == "mullion" && inside(p) && p.y1 - p.y0 > 0.5 * height && !p.meta.truthy("close"))
        .map(|p| [p.s0, p.s1])
        .collect();
    mullions.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap_or(std::cmp::Ordering::Equal));
    let mut transoms: Vec<[f64; 2]> = plan
        .parts
        .iter()
        .filter(|p| p.kind == "transom" && inside(p) && p.s1 - p.s0 > 0.5 * (op.s1 - op.s0) && !p.meta.truthy("close"))
        .map(|p| [p.y0, p.y1])
        .collect();
    transoms.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap_or(std::cmp::Ordering::Equal));
    let inset = if fw != 0.0 && !fw.is_nan() { fw } else { 0.065 };
    let mut cols = vec![];
    let mut a = op.s0 + inset;
    for [m0, m1] in mullions {
        cols.push([a, m0]);
        a = m1;
    }
    cols.push([a, op.s1 - inset]);
    let mut rows = vec![];
    let mut b = op.y0 + inset;
    for [t0, t1] in transoms {
        rows.push([b, t0]);
        b = t1;
    }
    rows.push([b, op.y1 - inset]);
    Some(Lights { w_at, framed: !frames.is_empty(), frame_width: inset, cols, rows })
}

pub const STUCCO_ROW_CLOSE_TRIANGLE_BUDGET: f64 = 8800.0;
const STUCCO_CLOSE_DROP_ORDER: [&str; 9] = ["secondary-windows", "lattice", "upper-muntins", "corbels", "tile-steps", "beads", "muntins", "tile-ribs", "sashes"];

fn stucco_glazing(style: &str) -> &'static [&'static str] {
    match style {
        "tract-storybook" => &["leaded", "casement-grid"],
        "interwar" => &["casement-grid", "leaded"],
        "tract-moderne" => &["steel-horizontal"],
        "tract-postwar" => &["slider"],
        "marina-mediterranean" => &["casement-grid", "french-grid"],
        "richmond-row" => &["double-hung"],
        _ => &["casement-grid"],
    }
}

fn door_styles(style: &str) -> &'static [&'static str] {
    match style {
        "tract-storybook" => &["plank", "arched-panel", "six-panel"],
        "interwar" => &["six-panel", "plank", "lite-top"],
        "tract-moderne" => &["three-horizontal", "lite-top"],
        "tract-postwar" => &["three-horizontal", "flush"],
        "marina-mediterranean" => &["plank", "six-panel", "arched-panel"],
        "richmond-row" => &["six-panel", "lite-top", "three-horizontal"],
        _ => &["six-panel"],
    }
}

const GARAGE_STYLES: [&str; 4] = ["short-panel", "short-panel", "long-panel", "ribbed"];

fn pick<'a>(list: &[&'a str], roll: f64) -> &'a str {
    list[(list.len() - 1).min((roll * list.len() as f64).floor() as usize)]
}

/// `closeRing`: a rectangle of four boxes inside [s0,s1]x[y0,y1] on one plane.
#[allow(clippy::too_many_arguments)]
pub fn close_ring(cx: &Cx, plan: &mut Plan, e: impl Into<CEdge>, kind: &str, slot: &str, part: &str, s0: f64, s1: f64, y0: f64, y1: f64, width: f64, w0: f64, w1: f64) {
    let e: CEdge = e.into();
    if !(s1 - s0 > 2.0 * width + 0.02 && y1 - y0 > 2.0 * width + 0.02) {
        return;
    }
    cx.bx(plan, e, kind, slot, part, [s0, s0 + width, y0, y1, w0, w1], None);
    cx.bx(plan, e, kind, slot, part, [s1 - width, s1, y0, y1, w0, w1], None);
    cx.bx(plan, e, CK::RAIL, slot, part, [s0 + width, s1 - width, y1 - width, y1, w0, w1], None);
    cx.bx(plan, e, CK::RAIL, slot, part, [s0 + width, s1 - width, y0, y0 + width * 1.2, w0, w1], None);
}

/// `closeGrid`: even glazing bars across a light, `cols` x `rows` panes.
#[allow(clippy::too_many_arguments)]
pub fn close_grid(cx: &Cx, plan: &mut Plan, e: impl Into<CEdge>, slot: &str, part: &str, s0: f64, s1: f64, y0: f64, y1: f64, cols: f64, rows: f64, bar: f64, w0: f64, w1: f64) {
    let e: CEdge = e.into();
    let mut i = 1.0;
    while i < cols {
        let s = s0 + ((s1 - s0) * i) / cols;
        cx.bx(plan, e, CK::BAR, slot, part, [s - bar / 2.0, s + bar / 2.0, y0, y1, w0, w1], None);
        i += 1.0;
    }
    let mut j = 1.0;
    while j < rows {
        let y = y0 + ((y1 - y0) * j) / rows;
        cx.bx(plan, e, CK::RAIL, slot, part, [s0, s1, y - bar / 2.0, y + bar / 2.0, w0, w1], None);
        j += 1.0;
    }
}

/// Diamond leading across a light.
#[allow(clippy::too_many_arguments)]
fn leaded(cx: &Cx, plan: &mut Plan, e: CEdge, slot: &str, part: &str, s0: f64, s1: f64, y0: f64, y1: f64, w0: f64, w1: f64) {
    let pitch = 0.17;
    let k = 1.45;
    let half = 0.006;
    for dir in [1.0f64, -1.0] {
        let span = (y1 - y0) / k;
        let off = if dir < 0.0 { y1 - y0 } else { 0.0 };
        let mut c = s0 - span;
        while c <= s1 + span {
            let y_at = |s: f64| y0 + dir * k * (s - c) + off;
            let s_at = |y: f64| c + (y - y0 - off) / (dir * k);
            let mut hits: Vec<[f64; 2]> = vec![];
            for s in [s0, s1] {
                let y = y_at(s);
                if y >= y0 - 1e-6 && y <= y1 + 1e-6 {
                    hits.push([s, y]);
                }
            }
            for y in [y0, y1] {
                let s = s_at(y);
                if s > s0 + 1e-6 && s < s1 - 1e-6 {
                    hits.push([s, y]);
                }
            }
            if hits.len() >= 2 {
                hits.sort_by(|p, q| p[0].partial_cmp(&q[0]).unwrap_or(std::cmp::Ordering::Equal));
                let a = hits[0];
                let b = hits[hits.len() - 1];
                if (b[0] - a[0]).hypot(b[1] - a[1]) >= 0.05 {
                    cx.plate(plan, e, CK::BAR, slot, part, vec![[a[0] - half, a[1]], [a[0] + half, a[1]], [b[0] + half, b[1]], [b[0] - half, b[1]]], w0, w1, None);
                }
            }
            c += pitch;
        }
    }
}

/// Options of `tileRibs`.
pub struct RibOpts {
    pub pitch: f64,
    pub width: f64,
    pub segments: usize,
    pub roof: bool,
    pub over: f64,
    pub rise: f64,
}

/// `tileRibs`: a cover-tile row every `pitch` along s, on the slope low -> high.
#[allow(clippy::too_many_arguments)]
pub fn tile_ribs(cx: &Cx, plan: &mut Plan, e: impl Into<CEdge>, slot: &str, part: &str, s0: f64, s1: f64, low: [f64; 2], high: [f64; 2], o: &RibOpts) -> usize {
    let e: CEdge = e.into();
    let count = 1f64.max(((s1 - s0) / o.pitch).floor()) as usize;
    let first = s0 + ((s1 - s0) - (count as f64 - 1.0) * o.pitch) / 2.0;
    let [wh, yh] = high;
    let len = {
        let l = (low[0] - wh).hypot(low[1] - yh);
        if l == 0.0 || l.is_nan() { 1.0 } else { l }
    };
    let wl = low[0] + ((low[0] - wh) / len) * o.over;
    let yl = low[1] + ((low[1] - yh) / len) * o.over;
    let extra = if o.roof { Some(json!({ "roof": true })) } else { None };
    let segments = o.segments;
    for kk in 0..count {
        let sc = first + kk as f64 * o.pitch;
        for g in 0..segments {
            let f0 = g as f64 / segments as f64 - if g > 0 { 0.04 } else { 0.0 };
            let f1 = (g + 1) as f64 / segments as f64;
            let pa = [wl + (wh - wl) * f0, yl + (yh - yl) * f0];
            let pb = [wl + (wh - wl) * f1, yl + (yh - yl) * f1];
            let t_low = if segments > 1 { o.rise + 0.012 } else { o.rise };
            cx.section(
                plan,
                e,
                CK::COURSE,
                slot,
                part,
                vec![[pa[0], pa[1] - 0.004], [pb[0], pb[1] - 0.004], [pb[0], pb[1] + o.rise * 0.7], [pa[0], pa[1] + t_low]],
                sc - o.width / 2.0,
                sc + o.width / 2.0,
                extra.as_ref(),
            );
        }
        let r = o.width / 2.0;
        cx.plate(
            plan,
            e,
            CK::COURSE,
            slot,
            &format!("{part}-end"),
            vec![[sc - r, yl - 0.006], [sc + r, yl - 0.006], [sc + r * 0.62, yl + o.rise * 0.85], [sc, yl + o.rise + 0.008], [sc - r * 0.62, yl + o.rise * 0.85]],
            wl - 0.004,
            wl + 0.012,
            extra.as_ref(),
        );
    }
    count * segments
}

/// `shingleCourses`: one wedge profile per course across a slope.
#[allow(clippy::too_many_arguments)]
pub fn shingle_courses(cx: &Cx, plan: &mut Plan, e: impl Into<CEdge>, slot: &str, part: &str, s0: f64, s1: f64, low: [f64; 2], high: [f64; 2], exposure: f64) {
    let e: CEdge = e.into();
    let [wl, yl] = low;
    let [wh, yh] = high;
    let length = (wh - wl).hypot(yh - yl);
    let n = 2f64.max(js_round(length / exposure)) as usize;
    let extra = json!({ "roof": true });
    for g in 0..n {
        let f0 = g as f64 / n as f64;
        let f1 = (g + 1) as f64 / n as f64;
        let pa = [wl + (wh - wl) * f0, yl + (yh - yl) * f0];
        let pb = [wl + (wh - wl) * f1, yl + (yh - yl) * f1];
        cx.profile(plan, e, CK::COURSE, slot, part, s0, s1, vec![[pa[0], pa[1] - 0.003], [pb[0], pb[1] - 0.003], [pb[0], pb[1] + 0.006], [pa[0], pa[1] + 0.022]], Some(&extra));
    }
}

/// A rounded bead proud of a convex vertical corner at (s, 0).
pub fn bead_section(s: f64, dir: f64) -> Vec<[f64; 2]> {
    vec![[s + dir * 0.045, 0.0], [s + dir * 0.022, 0.014], [s - dir * 0.006, 0.006], [s - dir * 0.014, -0.022], [s, -0.045]]
}

fn plate_points(p: &Part) -> &[[f64; 2]] {
    match &p.shape {
        Shape::Plate(v) | Shape::Profile(v) => v,
        _ => &[],
    }
}

/// The raised ring round an arched head.
fn archivolt(cx: &Cx, plan: &mut Plan, e: CEdge, arch: &Part, slot: &str) {
    let pts = plate_points(arch);
    if pts.len() < 4 {
        return;
    }
    let s0 = pts[0][0];
    let s1 = pts[3][0];
    let spring = pts[0][1];
    let apex = pts[1][1] - 0.03;
    let half = (s1 - s0) / 2.0;
    let h = apex - spring;
    if !(half > 0.2 && h > 0.05) {
        return;
    }
    let r = (half * half + h * h) / (2.0 * h);
    let cy = apex - r;
    let t0 = (1f64.min(half / r)).acos();
    let band = 0.07;
    let mut outer = arc_points((s0 + s1) / 2.0, cy, r + band, t0, PI - t0, 8);
    let inner = arc_points((s0 + s1) / 2.0, cy, r, PI - t0, t0, 8);
    outer.extend(inner);
    cx.plate(plan, e, CK::TRIM, slot, "archivolt", outer, -0.01, 0.022, None);
    cx.bx(plan, e, CK::TRIM, slot, "impost", [s0 - 0.08, s0 + 0.01, spring - 0.06, spring, -0.01, 0.03], None);
    cx.bx(plan, e, CK::TRIM, slot, "impost", [s1 - 0.01, s1 + 0.08, spring - 0.06, spring, -0.01, 0.03], None);
}

/// Arguments of `doorLeafDetail`.
pub struct DoorLeaf<'a> {
    pub s0: f64,
    pub s1: f64,
    pub y0: f64,
    pub y1: f64,
    pub face: f64,
    pub door_slot: &'a str,
    pub metal_slot: &'a str,
    pub frame_slot: &'a str,
    pub glass_slot: &'a str,
    pub hinge_left: bool,
}

/// `doorLeafDetail`: panels, hardware and a mail slot on a door leaf.
pub fn door_leaf_detail(cx: &Cx, plan: &mut Plan, e: impl Into<CEdge>, style: &str, d: &DoorLeaf) {
    let e: CEdge = e.into();
    let (s0, s1, y0, y1, face) = (d.s0, d.s1, d.y0, d.y1, d.face);
    let w = s1 - s0;
    let h = y1 - y0;
    if !(w > 0.5 && h > 1.6) {
        return;
    }
    let raised = [face, face + 0.014];
    let panel = |plan: &mut Plan, a: f64, b: f64, c: f64, dd: f64| {
        cx.bx(plan, e, CK::COURSE, d.door_slot, "door-panel", [a, b, c, dd, raised[0], raised[1]], None);
    };
    let m = 0.11;
    if style == "six-panel" {
        let cw = (w - 3.0 * m) / 2.0;
        let rows_h: Vec<f64> = [0.36, 0.3, 0.34].iter().map(|f| f * (h - 4.0 * m)).collect();
        let mut y = y0 + m;
        for rh in rows_h {
            for c in 0..2 {
                let c = c as f64;
                panel(plan, s0 + m + c * (cw + m), s0 + m + c * (cw + m) + cw, y, y + rh);
            }
            y += rh + m;
        }
    } else if style == "three-horizontal" {
        let ph = (h - 4.0 * m) / 3.0;
        for r in 0..3 {
            let r = r as f64;
            panel(plan, s0 + m, s1 - m, y0 + m + r * (ph + m), y0 + m + r * (ph + m) + ph);
        }
    } else if style == "lite-top" {
        let lite_h = h * 0.3;
        cx.bx(plan, e, CK::COURSE, d.glass_slot, "door-lite", [s0 + m, s1 - m, y1 - m - lite_h, y1 - m, face - 0.004, face + 0.004], None);
        cx.bx(plan, e, CK::RAIL, d.frame_slot, "lite-bar", [s0 + m, s1 - m, y1 - m - lite_h / 2.0 - 0.012, y1 - m - lite_h / 2.0 + 0.012, face, face + 0.012], None);
        let ph = (h - lite_h - 3.0 * m) / 2.0;
        for r in 0..2 {
            let r = r as f64;
            panel(plan, s0 + m, s1 - m, y0 + m + r * (ph + m), y0 + m + r * (ph + m) + ph);
        }
    } else if style == "plank" || style == "arched-panel" {
        let n = 5;
        for k in 1..n {
            let s = s0 + (w * k as f64) / n as f64;
            cx.bx(plan, e, CK::BAR, d.door_slot, "plank-joint", [s - 0.006, s + 0.006, y0 + 0.02, y1 - 0.02, face - 0.004, face + 0.003], None);
        }
        if style == "arched-panel" {
            panel(plan, s0 + m, s1 - m, y0 + m, y0 + h * 0.45);
            panel(plan, s0 + m, s1 - m, y0 + h * 0.45 + m, y1 - m * 1.5);
        } else {
            cx.bx(plan, e, CK::RAIL, d.door_slot, "ledger", [s0 + 0.04, s1 - 0.04, y0 + 0.3, y0 + 0.4, face, face + 0.018], None);
            cx.bx(plan, e, CK::RAIL, d.door_slot, "ledger", [s0 + 0.04, s1 - 0.04, y1 - 0.42, y1 - 0.32, face, face + 0.018], None);
        }
        let gy = y0 + h * 0.72;
        let gs = (s0 + s1) / 2.0;
        cx.bx(plan, e, CK::FIXTURE, d.metal_slot, "grille-frame", [gs - 0.13, gs + 0.13, gy - 0.1, gy + 0.1, face, face + 0.012], None);
        for k in -1..=1 {
            let k = k as f64;
            cx.bx(plan, e, CK::BAR, d.metal_slot, "grille-bar", [gs + k * 0.07 - 0.008, gs + k * 0.07 + 0.008, gy - 0.1, gy + 0.1, face + 0.012, face + 0.03], None);
        }
    }
    let ls = if d.hinge_left { s1 - 0.08 } else { s0 + 0.08 };
    cx.bx(plan, e, CK::FIXTURE, d.metal_slot, "escutcheon", [ls - 0.025, ls + 0.025, y0 + 0.9, y0 + 1.12, face, face + 0.01], None);
    cx.bx(
        plan,
        e,
        CK::FIXTURE,
        d.metal_slot,
        "lever",
        [if d.hinge_left { ls - 0.13 } else { ls - 0.02 }, if d.hinge_left { ls + 0.02 } else { ls + 0.13 }, y0 + 1.0, y0 + 1.025, face + 0.03, face + 0.05],
        None,
    );
    cx.bx(plan, e, CK::FIXTURE, d.metal_slot, "lever-neck", [ls - 0.012, ls + 0.012, y0 + 0.995, y0 + 1.03, face + 0.01, face + 0.03], None);
    if style != "flush" || h > 1.8 {
        let ms = (s0 + s1) / 2.0;
        cx.bx(plan, e, CK::FIXTURE, d.metal_slot, "mail-slot", [ms - 0.15, ms + 0.15, y0 + 0.78, y0 + 0.85, face, face + 0.012], None);
    }
}

/// The closed roller curtain a garage door detail is drawn on.
pub struct Curtain {
    pub s0: f64,
    pub s1: f64,
}

/// `garageDoorDetail`: sectional garage door joinery on the closed curtain.
#[allow(clippy::too_many_arguments)]
pub fn garage_door_detail(cx: &Cx, plan: &mut Plan, e: impl Into<CEdge>, style: &str, door: &Curtain, cuts: &[f64], face: f64, roller_slot: &str, metal_slot: &str, glass_slot: &str, lites: bool) {
    let e: CEdge = e.into();
    let s0 = door.s0 + 0.04;
    let s1 = door.s1 - 0.04;
    let w = s1 - s0;
    let mut k = 0;
    while k + 1 < cuts.len() {
        let a = cuts[k] + 0.06;
        let b = cuts[k + 1] - 0.06;
        if !(b - a > 0.12) {
            k += 1;
            continue;
        }
        let top = k + 2 == cuts.len();
        if style == "ribbed" {
            for r in 1..4 {
                let y = a + ((b - a) * r as f64) / 4.0;
                cx.bx(plan, e, CK::COURSE, roller_slot, "door-rib", [s0, s1, y - 0.006, y + 0.006, face - 0.03, face - 0.018], None);
            }
            k += 1;
            continue;
        }
        let n = if style == "long-panel" { 2 } else { 4 };
        let gap = 0.07;
        let pw = (w - (n as f64 + 1.0) * gap) / n as f64;
        for i in 0..n {
            let ps0 = s0 + gap + i as f64 * (pw + gap);
            if top && lites {
                cx.bx(plan, e, CK::COURSE, glass_slot, "door-lite", [ps0, ps0 + pw, a, b, face - 0.03, face - 0.016], None);
            } else {
                cx.bx(plan, e, CK::COURSE, roller_slot, "door-panel", [ps0, ps0 + pw, a, b, face - 0.03, face - 0.016], None);
            }
        }
        k += 1;
    }
    let ms = (door.s0 + door.s1) / 2.0;
    let y = cuts[0] + 0.32;
    cx.bx(plan, e, CK::FIXTURE, metal_slot, "garage-handle", [ms - 0.07, ms + 0.07, y, y + 0.035, face - 0.016, face + 0.01], None);
}

/// `downspout`: a downspout down the wall from `yTop` to grade.
#[allow(clippy::too_many_arguments)]
pub fn downspout(cx: &Cx, plan: &mut Plan, e: impl Into<CEdge>, slot: &str, s: f64, y_top: f64, lift: f64, reach: f64, from: Option<[f64; 2]>) {
    let e: CEdge = e.into();
    let w0 = 0.03f64.min(reach - 0.08);
    let w1 = w0 + 0.075;
    let mut top = y_top;
    let foot = lift + PEDESTRIAN_HEADROOM_M;
    if let Some([fw, fy]) = from {
        let drop_to = foot.max(fy - 0.25f64.max((fw - (w0 + w1) / 2.0).abs() * 1.2));
        if !(fy - drop_to > 0.1) {
            return;
        }
        cx.section(plan, e, CK::TRIM, slot, "swan-neck", vec![[fw - 0.035, fy], [fw + 0.035, fy], [w1, drop_to], [w0, drop_to]], s - 0.035, s + 0.035, None);
        top = drop_to;
    }
    if !(top - foot > 0.3) {
        let yb = foot.max(top - 0.09);
        cx.bx(plan, e, CK::TRIM, slot, "downspout-elbow", [s - 0.04, s + 0.04, yb, yb + 0.09, -0.02, w1 + 0.004], None);
        return;
    }
    cx.bx(plan, e, CK::TRIM, slot, "downspout", [s - 0.037, s + 0.037, foot + 0.08, top, w0, w1], None);
    cx.bx(plan, e, CK::TRIM, slot, "downspout-elbow", [s - 0.04, s + 0.04, foot, foot + 0.09, -0.02, w1 + 0.004], None);
    let mut y = foot + 0.6;
    while y < top - 0.3 {
        cx.bx(plan, e, CK::FIXTURE, slot, "strap", [s - 0.05, s + 0.05, y, y + 0.03, -0.005, w1 + 0.008], None);
        y += 1.6;
    }
}

fn profile_points(p: &Part) -> Vec<[f64; 2]> {
    match &p.shape {
        Shape::Profile(v) => v.clone(),
        _ => vec![],
    }
}

/// The upper surface of a slope profile: (lo, hi).
fn slope_of(p: &Part) -> (Option<[f64; 2]>, Option<[f64; 2]>) {
    let pr = profile_points(p);
    if pr.is_empty() {
        return (None, None);
    }
    let mut hi = pr[0];
    for q in &pr {
        if q[1] > hi[1] + 1e-6 || ((q[1] - hi[1]).abs() < 1e-6 && q[0] < hi[0]) {
            hi = *q;
        }
    }
    let outer = pr.iter().fold(f64::NEG_INFINITY, |m, q| m.max(q[0]));
    let mut lo: Option<[f64; 2]> = None;
    for q in &pr {
        if (q[0] - outer).abs() < 1e-6 && lo.map(|l| q[1] > l[1]).unwrap_or(true) {
            lo = Some(*q);
        }
    }
    (lo, Some(hi))
}

/// `addStuccoRowCloseDetail`.
#[allow(clippy::too_many_arguments)]
fn add_stucco_row_close_detail(
    plan: &mut Plan,
    frame: &Frame,
    meta: &J,
    front: &Edge,
    entry_edge: &Edge,
    entry: Option<&OpeningRecord>,
    garage: Option<&OpeningRecord>,
    wall_top: f64,
    _roof_top: f64,
    lift: f64,
) -> J {
    let st = style_of(meta);
    let style = meta.s("style").unwrap_or("").to_string();
    let mut random = Rng::new(&format!("{}|stucco-row|close", plan.seed));
    let r_glazing = random.next();
    let r_upper = random.next();
    let r_door = random.next();
    let r_garage = random.next();
    let r_lites = random.next();
    let r_spout = random.next();
    let r_spout_colour = random.next();
    let r_meter = random.next();
    let r_dryer = random.next();
    let r_number = random.next();
    let r_lamp = random.next();
    let r_rail = random.next();
    let r_eyebrow_fin = random.next();
    let r_mailbox = random.next();
    let mut cx = close_context(plan, meta.s("kind").unwrap_or(""));
    let allowance = allowance_of(front);
    let fit = |want: f64, edge: &Edge| 0.03f64.max(want.min(allowance_of(edge) - 0.04));
    let fitf = |want: f64| fit(want, front);
    let glazing = pick(stucco_glazing(&style), r_glazing);
    let frame_slot = cx.slot(plan, &["frame"]);
    let metal_slot = cx.slot(plan, &["metal", "iron", "frame"]);
    let trim_slot = cx.slot(plan, &["trim", "coping", "frame"]);
    let base_slot = cx.slot(plan, &["base", "trim"]);
    let tile_slot = cx.slot(plan, &["tile", "pitchedRoof"]);
    let door_slot = cx.slot(plan, &["door", "frame"]);
    let roller_slot = cx.slot(plan, &["roller", "door"]);
    let pipe_slot = if r_spout_colour < 0.55 { "wall".to_string() } else { metal_slot.clone() };
    let mut edges: Vec<&Edge> = vec![front];
    if entry_edge.index != front.index {
        edges.push(entry_edge);
    }
    let fs = frame_slot.as_str();
    let ms = metal_slot.as_str();

    // 1. Windows.
    let arches: Vec<Part> = plan.parts.iter().filter(|p| p.kind == "arch-infill").cloned().collect();
    let arch_over = |edge: &Edge, s0: f64, s1: f64, y1: f64| -> Option<Part> {
        arches.iter().find(|p| p.edge == Some(edge.index) && p.s0 >= s0 - 0.05 && p.s1 <= s1 + 0.05 && p.y1 >= y1 - 0.06 && p.y0 < y1).cloned()
    };
    let secondary: Vec<&Edge> = frame.edges.iter().filter(|e| e.role == "front-secondary" && e.index != entry_edge.index && !e.party).collect();
    let mut all_edges = edges.clone();
    all_edges.extend(secondary.iter().copied());
    let reveal_meta = meta.f("reveal").unwrap_or_else(|| st.num("reveal"));
    for edge in all_edges {
        let Some(record) = plan.edges.iter().find(|e| e.index == edge.index) else { continue };
        let ops = record.openings.clone();
        let on_secondary = !edges.iter().any(|e| e.index == edge.index);
        for op in &ops {
            if op.type_ != "punched" {
                continue;
            }
            let Some(lights) = opening_lights(plan, edge.index, op) else { continue };
            let w_at = lights.w_at;
            let upper = op.storey >= 2;
            cx.group = if on_secondary { "secondary-windows" } else { "sashes" }.into();
            if !lights.framed {
                close_ring(&cx, plan, edge, CK::TRIM, fs, "frame-restored", op.s0, op.s1, op.y0, op.y1, 0.065, w_at - 0.04, w_at + 0.04);
            }
            let sash_w = [w_at + 0.002, w_at + 0.03];
            let bar_w = [w_at + 0.001, w_at + 0.02];
            let bar = if glazing == "steel-horizontal" { 0.018 } else { 0.024 };
            let cols = &lights.cols;
            let rows = &lights.rows;
            let wide = cols.len() > 1;
            let muntin_group = if on_secondary { "secondary-windows" } else if upper { "upper-muntins" } else { "muntins" };
            for (ci, [a, b]) in cols.iter().enumerate() {
                let (a, b) = (*a, *b);
                for [c, d] in rows.iter() {
                    let (c, d) = (*c, *d);
                    let light_w = b - a;
                    let light_h = d - c;
                    if !(light_w > 0.2 && light_h > 0.2) {
                        continue;
                    }
                    let fixed_centre = wide && cols.len() >= 3 && ci == cols.len() / 2 && light_w > 1.0;
                    cx.group = if on_secondary { "secondary-windows" } else { "sashes" }.into();
                    if glazing == "double-hung" && rows.len() == 1 {
                        let mid = c + light_h * 0.52;
                        close_ring(&cx, plan, edge, CK::BAR, fs, "sash-upper", a, b, mid - 0.02, d, 0.045, sash_w[0], sash_w[1]);
                        close_ring(&cx, plan, edge, CK::BAR, fs, "sash-lower", a, b, c, mid + 0.02, 0.05, sash_w[0] + 0.025, sash_w[1] + 0.025);
                        cx.group = muntin_group.into();
                        let lites = if r_upper < 0.4 { 1.0 } else if r_upper < 0.75 { 2.0 } else { 3.0 };
                        if light_w > 0.55 {
                            close_grid(&cx, plan, edge, fs, "muntin", a + 0.045, b - 0.045, mid, d - 0.045, lites, 1.0, bar, bar_w[0], bar_w[1]);
                        }
                        continue;
                    }
                    if !fixed_centre {
                        let part = if glazing == "slider" { "slider-sash" } else { "sash" };
                        close_ring(&cx, plan, edge, CK::BAR, fs, part, a, b, c, d, if glazing == "steel-horizontal" { 0.03 } else { 0.045 }, sash_w[0], sash_w[1]);
                    }
                    if fixed_centre && glazing != "leaded" {
                        continue;
                    }
                    let ia = a + 0.045;
                    let ib = b - 0.045;
                    let ic = c + 0.05;
                    let id = d - 0.045;
                    if !(ib - ia > 0.15 && id - ic > 0.15) {
                        continue;
                    }
                    cx.group = muntin_group.into();
                    if glazing == "leaded" {
                        if fixed_centre {
                            let band = 0.45f64.min(light_h * 0.3);
                            cx.bx(plan, edge, CK::RAIL, fs, "leaded-band", [a, b, d - band - 0.02, d - band + 0.02, bar_w[0], bar_w[1]], None);
                            cx.group = "lattice".into();
                            leaded(&cx, plan, edge.into(), fs, "came", a, b, d - band, d, bar_w[0], bar_w[1]);
                        } else {
                            cx.group = "lattice".into();
                            leaded(&cx, plan, edge.into(), fs, "came", ia, ib, ic, id, bar_w[0], bar_w[1]);
                        }
                    } else if glazing == "casement-grid" || glazing == "french-grid" {
                        let cols_n = 1f64.max(4f64.min(js_round((ib - ia) / if glazing == "french-grid" { 0.26 } else { 0.32 })));
                        let rows_n = 1f64.max(6f64.min(js_round((id - ic) / if glazing == "french-grid" { 0.3 } else { 0.36 })));
                        close_grid(&cx, plan, edge, fs, "muntin", ia, ib, ic, id, cols_n, rows_n, bar, bar_w[0], bar_w[1]);
                    } else if glazing == "steel-horizontal" {
                        let rows_n = 2f64.max(6f64.min(js_round((id - ic) / 0.3)));
                        close_grid(&cx, plan, edge, fs, "steel-bar", ia, ib, ic, id, if light_w > 0.9 { 2.0 } else { 1.0 }, rows_n, bar, bar_w[0], bar_w[1]);
                    } else if glazing == "slider" && !wide {
                        let m = (ia + ib) / 2.0;
                        cx.bx(plan, edge, CK::BAR, fs, "meeting-stile", [m - 0.03, m + 0.03, ic, id, sash_w[0], sash_w[1] + 0.02], None);
                    }
                }
            }
            cx.group = if on_secondary { "secondary-windows" } else { "beads" }.into();
            let kit_sill = plan
                .parts
                .iter()
                .find(|p| p.kind == "sill" && p.edge == Some(edge.index) && p.storey == Some(op.storey as i64) && p.bay == Some(op.bay as i64) && p.y1 <= op.y0 + 0.02)
                .cloned();
            if let Some(ks) = kit_sill {
                let p = profile_points(&ks).iter().fold(f64::NEG_INFINITY, |m, q| m.max(q[0]));
                cx.bx(plan, edge, CK::TRIM, &ks.slot, "sill-drip", [ks.s0 + 0.01, ks.s1 - 0.01, ks.y0 - 0.014, ks.y0 + 0.002, p - 0.022, p - 0.004], None);
            } else {
                let p = fit(0.05, edge);
                let y = op.y0;
                let back = -reveal_meta + 0.005;
                let slot = if style == "tract-postwar" || style == "richmond-row" { fs } else { trim_slot.as_str() };
                cx.profile(
                    plan,
                    edge,
                    CK::TRIM,
                    slot,
                    "drip-sill",
                    op.s0 - 0.05,
                    op.s1 + 0.05,
                    vec![[back, y], [p, y - 0.018], [p, y - 0.055], [p - 0.018, y - 0.055], [p - 0.018, y - 0.04], [back, y - 0.04]],
                    None,
                );
            }
            let arch = arch_over(edge, op.s0, op.s1, op.y1);
            let spring = arch.as_ref().map(|a| plate_points(a)[0][1]).unwrap_or(op.y1);
            if spring - op.y0 > 0.3 {
                cx.column(plan, edge, CK::TRIM, "wall", "reveal-bead", bead_section(op.s0, -1.0), op.y0, spring, None);
                cx.column(plan, edge, CK::TRIM, "wall", "reveal-bead", bead_section(op.s1, 1.0), op.y0, spring, None);
            }
            if arch.is_none() {
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
            cx.group = if on_secondary { "secondary-windows" } else { "beads" }.into();
            if let Some(a) = &arch {
                archivolt(&cx, plan, edge.into(), a, "wall");
            } else if style == "richmond-row" {
                let cw = 0.09;
                cx.bx(plan, edge, CK::TRIM, fs, "casing", [op.s0 - cw, op.s0, op.y0 - 0.04, op.y1 + cw, -0.01, 0.022], None);
                cx.bx(plan, edge, CK::TRIM, fs, "casing", [op.s1, op.s1 + cw, op.y0 - 0.04, op.y1 + cw, -0.01, 0.022], None);
                cx.profile(plan, edge, CK::TRIM, fs, "head-casing", op.s0 - cw - 0.03, op.s1 + cw + 0.03, trim_profile("stepped", fit(0.06, edge), 0.14, op.y1 + cw, -0.01), None);
            } else if style == "tract-postwar" {
                cx.bx(plan, edge, CK::TRIM, fs, "head-flashing", [op.s0 - 0.03, op.s1 + 0.03, op.y1, op.y1 + 0.03, -0.01, fit(0.03, edge)], None);
            } else if style == "tract-moderne" && r_eyebrow_fin < 0.6 {
                cx.bx(plan, edge, CK::TRIM, "wall", "head-fin", [op.s0 - 0.12, op.s1 + 0.12, op.y1 + 0.06, op.y1 + 0.11, -0.01, fit(0.18, edge)], None);
            } else if !plan.parts.iter().any(|p| p.kind == "eyebrow" && p.edge == Some(edge.index) && p.s0 <= op.s0 && p.s1 >= op.s1 && (p.y0 - op.y1).abs() < 0.2) {
                cx.profile(plan, edge, CK::TRIM, "wall", "drip-cap", op.s0 - 0.06, op.s1 + 0.06, trim_profile_d("sloped", fit(0.045, edge), 0.09, op.y1 + 0.02), None);
            }
        }
    }

    // Glass block.
    if st.truthy("glassBlock") {
        cx.group = "muntins".into();
        for edge in &edges {
            let (_, ops) = record_for(plan, edge.index);
            for op in &ops {
                let Some(pane) = plan.parts.iter().find(|p| p.kind == "glass" && p.meta.s("opening") == Some(op.id.as_str()) && p.slot == "glassBlock").cloned() else { continue };
                close_grid(
                    &cx,
                    plan,
                    *edge,
                    fs,
                    "block-joint",
                    pane.s0,
                    pane.s1,
                    pane.y0,
                    pane.y1,
                    2f64.max(js_round((pane.s1 - pane.s0) / 0.2)),
                    2f64.max(js_round((pane.y1 - pane.y0) / 0.2)),
                    0.016,
                    pane.w1,
                    pane.w1 + 0.012,
                );
            }
        }
    }

    // 2. The entry.
    cx.group = "door".into();
    if let Some(entry) = entry {
        let leaf_parts: Vec<Part> = plan.parts.iter().filter(|p| p.kind == "door-leaf" && p.meta.s("aperture") == Some(entry.id.as_str())).cloned().collect();
        if !leaf_parts.is_empty() {
            let s0 = leaf_parts.iter().fold(f64::INFINITY, |m, p| m.min(p.s0));
            let s1 = leaf_parts.iter().fold(f64::NEG_INFINITY, |m, p| m.max(p.s1));
            let y0 = leaf_parts.iter().fold(f64::INFINITY, |m, p| m.min(p.y0));
            let y1 = leaf_parts.iter().fold(f64::NEG_INFINITY, |m, p| m.max(p.y1));
            let face = leaf_parts.iter().fold(f64::NEG_INFINITY, |m, p| m.max(p.w1));
            let glazed = leaf_parts.iter().any(|p| p.meta.truthy("glazed"));
            let glass_slot = cx.slot(plan, &["glass", "doorGlass"]);
            let dstyle = if glazed { "flush" } else { pick(door_styles(&style), r_door) };
            door_leaf_detail(
                &cx,
                plan,
                entry_edge,
                dstyle,
                &DoorLeaf { s0, s1, y0, y1, face, door_slot: &door_slot, metal_slot: ms, frame_slot: fs, glass_slot: &glass_slot, hinge_left: true },
            );
            let fw = 0.07;
            let cw = [face + 0.03, face + 0.05];
            cx.bx(plan, entry_edge, CK::TRIM, &door_slot, "door-casing", [entry.s0, entry.s0 + fw + 0.03, y0 - 0.02, y1 + 0.1, cw[0], cw[1]], None);
            cx.bx(plan, entry_edge, CK::TRIM, &door_slot, "door-casing", [entry.s1 - fw - 0.03, entry.s1, y0 - 0.02, y1 + 0.1, cw[0], cw[1]], None);
            cx.bx(plan, entry_edge, CK::TRIM, &door_slot, "door-casing", [entry.s0, entry.s1, y1, y1 + 0.12, cw[0], cw[1]], None);
        }
        let steps: Vec<Part> = plan.parts.iter().filter(|p| p.kind == "stoop-step" && p.edge == Some(entry_edge.index)).cloned().collect();
        let stoop = json!({ "stoop": true });
        for step in &steps {
            cx.bx(
                plan,
                entry_edge,
                CK::TRIM,
                "base",
                "nosing",
                [step.s0, step.s1, step.y1 - 0.035, step.y1, step.w1 - 0.01, (step.w1 + 0.02).min(fit(step.w1 + 0.02, entry_edge))],
                Some(&stoop),
            );
        }
        let stoop_outside = meta.f("stoopOutside").unwrap_or(0.0);
        if steps.len() >= 2 || (!steps.is_empty() && stoop_outside > 0.0) {
            let top = steps.iter().fold(f64::NEG_INFINITY, |m, p| m.max(p.y1));
            let bottom = steps.iter().fold(f64::INFINITY, |m, p| m.min(p.y0));
            let reach = steps.iter().fold(f64::NEG_INFINITY, |m, p| m.max(p.w1));
            let back = -meta.f("porchRecess").unwrap_or(PORCH_RECESS_M) + 0.1;
            let side = if r_rail < 0.5 { entry.s0 + 0.05 } else { entry.s1 - 0.09 };
            let rail_top = 0.9;
            let lo = [(back + 0.2).max(reach - 0.06), bottom + 0.02 + RISER_M];
            let hi = [back + 0.05, top];
            cx.bx(plan, entry_edge, CK::BAR, ms, "rail-post", [side, side + 0.04, lo[1] - RISER_M, lo[1] + rail_top, lo[0] - 0.04, lo[0]], Some(&stoop));
            cx.bx(plan, entry_edge, CK::BAR, ms, "rail-post", [side, side + 0.04, hi[1], hi[1] + rail_top, hi[0], hi[0] + 0.04], Some(&stoop));
            cx.section(
                plan,
                entry_edge,
                CK::RAIL,
                ms,
                "handrail",
                vec![[lo[0], lo[1] + rail_top - 0.045], [hi[0], hi[1] + rail_top - 0.045], [hi[0], hi[1] + rail_top], [lo[0], lo[1] + rail_top]],
                side - 0.005,
                side + 0.045,
                Some(&stoop),
            );
            cx.section(
                plan,
                entry_edge,
                CK::RAIL,
                ms,
                "handrail",
                vec![[lo[0], lo[1] + 0.12], [hi[0], hi[1] + 0.12], [hi[0], hi[1] + 0.15], [lo[0], lo[1] + 0.15]],
                side + 0.005,
                side + 0.035,
                Some(&stoop),
            );
        }
        cx.group = "fixtures".into();
        let (ebays, _) = record_for(plan, entry_edge.index);
        let entry_bay = meta.f("entryBay").unwrap_or(-1.0);
        let bay = if entry_bay >= 0.0 { ebays.get(entry_bay as usize) } else { None };
        let left = bay.map(|b| entry.s0 - b.s0).unwrap_or(0.3);
        let right = bay.map(|b| b.s1 - entry.s1).unwrap_or(0.3);
        let lamp_left = if r_lamp < 0.5 { left >= 0.22 } else { right < 0.22 };
        let lamp_s = if lamp_left { entry.s0 - 0.2f64.min(left / 2.0) } else { entry.s1 + 0.2f64.min(right / 2.0) };
        let num_s = if lamp_left { entry.s1 + 0.2f64.min(right / 2.0) } else { entry.s0 - 0.2f64.min(left / 2.0) };
        let lamp_y = (entry.y1 - 0.45).max(lift + PEDESTRIAN_HEADROOM_M + 0.18);
        let ee = entry_edge;
        if (if lamp_left { left } else { right }) >= 0.16 {
            cx.bx(plan, ee, CK::FIXTURE, ms, "lamp-plate", [lamp_s - 0.06, lamp_s + 0.06, lamp_y - 0.1, lamp_y + 0.12, -0.005, 0.02], None);
            cx.bx(plan, ee, CK::FIXTURE, ms, "lamp-arm", [lamp_s - 0.012, lamp_s + 0.012, lamp_y - 0.01, lamp_y + 0.015, 0.02, fit(0.1, ee)], None);
            cx.bx(plan, ee, CK::FIXTURE, fs, "lamp", [lamp_s - 0.065, lamp_s + 0.065, lamp_y - 0.17, lamp_y + 0.07, fit(0.06, ee), fit(0.19, ee)], None);
            cx.bx(plan, ee, CK::FIXTURE, ms, "lamp-cap", [lamp_s - 0.08, lamp_s + 0.08, lamp_y + 0.07, lamp_y + 0.1, fit(0.05, ee), fit(0.2, ee)], None);
        }
        let other = if lamp_left { right } else { left };
        if other >= 0.14 {
            let digits = if r_number < 0.3 { 3 } else { 4 };
            let ny = entry.y1 - 0.2;
            let pw = 0.05 * digits as f64 + 0.04;
            if pw < other + 0.2 {
                cx.bx(plan, ee, CK::FIXTURE, fs, "number-plate", [num_s - pw / 2.0, num_s + pw / 2.0, ny - 0.07, ny + 0.07, -0.005, 0.012], None);
                for d in 0..digits {
                    let ds = num_s - pw / 2.0 + 0.02 + 0.05 * d as f64 + 0.008;
                    cx.bx(plan, ee, CK::FIXTURE, ms, "number-digit", [ds, ds + 0.034, ny - 0.045, ny + 0.045, 0.012, 0.02], None);
                }
            }
            if r_mailbox < 0.65 {
                let my = (entry.y0 + 1.05).max(ny - 0.75);
                cx.bx(plan, ee, CK::FIXTURE, ms, "mailbox", [num_s - 0.13, num_s + 0.13, my, my + 0.32, -0.005, fit(0.09, ee)], None);
                cx.bx(plan, ee, CK::FIXTURE, ms, "mailbox-lid", [num_s - 0.145, num_s + 0.145, my + 0.32, my + 0.35, -0.005, fit(0.11, ee)], None);
            }
        }
    }

    // 3. The garage.
    cx.group = "garage".into();
    if let Some(garage) = garage {
        if let Some(door) = plan.parts.iter().find(|p| p.kind == "roller-door" && p.meta.s("opening") == Some(garage.id.as_str())).cloned() {
            let gstyle = pick(&GARAGE_STYLES, r_garage);
            let face = door.w1 + 0.03;
            let mut joints: Vec<f64> = plan
                .parts
                .iter()
                .filter(|p| p.kind == "roller-slat" && p.edge == Some(front.index) && p.s0 >= door.s0 - 1e-3 && p.s1 <= door.s1 + 1e-3)
                .map(|p| (p.y0 + p.y1) / 2.0)
                .collect();
            joints.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let mut cuts = vec![door.y0 + 0.08];
            cuts.extend(joints);
            cuts.push(door.y1);
            let glass_slot = cx.slot(plan, &["glass", "doorGlass"]);
            garage_door_detail(&cx, plan, front, gstyle, &Curtain { s0: door.s0, s1: door.s1 }, &cuts, face, &roller_slot, ms, &glass_slot, r_lites < 0.3);
            let arch = arch_over(front, garage.s0, garage.s1, garage.y1);
            let top = arch.as_ref().map(|a| plate_points(a)[0][1]).unwrap_or(garage.y1);
            cx.bx(plan, front, CK::TRIM, ms, "weatherstrip", [door.s0, door.s0 + 0.03, door.y0, door.y1, door.w1, door.w1 + 0.045], None);
            cx.bx(plan, front, CK::TRIM, ms, "weatherstrip", [door.s1 - 0.03, door.s1, door.y0, door.y1, door.w1, door.w1 + 0.045], None);
            cx.bx(plan, front, CK::TRIM, ms, "weatherstrip", [door.s0, door.s1, door.y1 - 0.03, door.y1, door.w1, door.w1 + 0.045], None);
            let cw = 0.1;
            let head = if arch.is_some() { 0.0 } else { cw };
            cx.bx(plan, front, CK::TRIM, &trim_slot, "garage-casing", [garage.s0 - cw, garage.s0, garage.y0, top + head, -0.01, fitf(0.025)], None);
            cx.bx(plan, front, CK::TRIM, &trim_slot, "garage-casing", [garage.s1, garage.s1 + cw, garage.y0, top + head, -0.01, fitf(0.025)], None);
            match &arch {
                None => {
                    cx.bx(plan, front, CK::TRIM, &trim_slot, "garage-casing", [garage.s0 - cw, garage.s1 + cw, garage.y1, garage.y1 + cw, -0.01, fitf(0.025)], None);
                }
                Some(a) => archivolt(&cx, plan, front.into(), a, "wall"),
            }
        }
    }

    // 4. Roof edge.
    cx.group = "roof-edge".into();
    let copings: Vec<Part> = plan.parts.iter().filter(|p| p.kind == "coping" && matches!(p.shape, Shape::Profile(_))).cloned().collect();
    for c in &copings {
        let Some(edge) = c.edge.and_then(|i| frame.edges.get(i)) else { continue };
        let pr = profile_points(c);
        let w0 = pr.iter().fold(f64::INFINITY, |m, q| m.min(q[0]));
        let w1 = pr.iter().fold(f64::NEG_INFINITY, |m, q| m.max(q[0]));
        let y0 = pr.iter().fold(f64::INFINITY, |m, q| m.min(q[1]));
        let y1 = pr.iter().fold(f64::NEG_INFINITY, |m, q| m.max(q[1]));
        let out = (w1 + 0.025).min(fit(w1 + 0.025, edge));
        cx.profile(
            plan,
            edge,
            CK::TRIM,
            &c.slot,
            "coping-cap",
            c.s0 - 0.012,
            c.s1 + 0.012,
            vec![
                [out, y0 - 0.022],
                [out, y1 + 0.02],
                [w0 - 0.02, y1 + 0.006],
                [w0 - 0.02, y0 - 0.022],
                [w0 - 0.006, y0 - 0.022],
                [w0 - 0.006, y0 - 0.004],
                [out - 0.016, y0 - 0.004],
                [out - 0.016, y0 - 0.022],
            ],
            None,
        );
        if edge.index == front.index && st.s("coping") == Some("cast-stone-trim") {
            cx.profile(plan, edge, CK::TRIM, "wall", "bed-moulding", 0.0, edge.length, trim_profile_d("cove", fitf(0.05), 0.09, y0 - 0.16), None);
        }
    }
    let l = front.length;
    let slopes: Vec<Part> = plan
        .parts
        .iter()
        .filter(|p| (p.kind == "roof-slope" || p.kind == "eyebrow") && matches!(p.shape, Shape::Profile(_)) && p.edge.and_then(|e| frame.edges.get(e)).is_some())
        .cloned()
        .collect();
    let is_tile = |p: &Part| plan.materials.get(&p.slot).map(|s| s == "clay-barrel-tile").unwrap_or(false);
    let ribbed: Vec<Part> = slopes.iter().filter(|p| is_tile(p)).cloned().collect();
    let coursed: Vec<Part> = slopes.iter().filter(|p| !is_tile(p)).cloned().collect();
    let base_cost = total_triangles(plan, close_part_triangles);
    let columns = ribbed.iter().fold(0.0, |n, p| n + 1f64.max(((p.s1 - p.s0) / if p.kind == "roof-slope" { 0.22 } else { 0.17 }).floor()));
    let spare = STUCCO_ROW_CLOSE_TRIANGLE_BUDGET - ROOF_PLANT_RESERVE_TRI - base_cost - 1400.0;
    let segments = if columns != 0.0 { 1f64.max(4f64.min((spare / (columns * 12.0)).floor() - 1.0)) } else { 1.0 };
    for p in &ribbed {
        let edge = &frame.edges[p.edge.unwrap()];
        let (lo, hi) = slope_of(p);
        let (Some(lo), Some(hi)) = (lo, hi) else { continue };
        if !(lo[0] - hi[0] > 0.12) {
            continue;
        }
        let is_roof = p.kind == "roof-slope";
        cx.group = if segments > 1.0 && is_roof { "tile-steps" } else { "tile-ribs" }.into();
        let opts = if is_roof {
            RibOpts { segments: segments as usize, roof: true, pitch: 0.22, width: 0.12, over: 0f64.max(0.04f64.min(fit(lo[0] + 0.04, edge) - lo[0])), rise: 0.045 }
        } else {
            RibOpts { segments: 1, roof: p.meta.truthy("roof"), pitch: 0.17, width: 0.1, over: 0f64.max(0.03f64.min(fit(lo[0] + 0.03, edge) - lo[0])), rise: 0.032 }
        };
        tile_ribs(&cx, plan, edge, &p.slot, "cover-tile", p.s0 + 0.04, p.s1 - 0.04, lo, hi, &opts);
        cx.group = "tile-ribs".into();
        if is_roof {
            cx.profile(
                plan,
                edge,
                CK::COURSE,
                &p.slot,
                "ridge-tile",
                p.s0,
                p.s1,
                vec![[hi[0] - 0.08, hi[1] - 0.01], [hi[0] + 0.08, hi[1] - 0.01], [hi[0] + 0.06, hi[1] + 0.07], [hi[0], hi[1] + 0.1], [hi[0] - 0.06, hi[1] + 0.07]],
                Some(&json!({ "roof": true })),
            );
        }
    }
    for p in &coursed {
        let edge = &frame.edges[p.edge.unwrap()];
        let (lo, hi) = slope_of(p);
        let (Some(lo), Some(hi)) = (lo, hi) else { continue };
        if !(lo[0] - hi[0] > 0.12) || p.kind != "roof-slope" {
            continue;
        }
        cx.group = "tile-ribs".into();
        let exposure = if plan.materials.get(&p.slot).map(|s| s == "cedar-shingle").unwrap_or(false) { 0.16 } else { 0.2 };
        shingle_courses(&cx, plan, edge, &p.slot, "shingle-course", p.s0, p.s1, lo, hi, exposure);
        cx.profile(
            plan,
            edge,
            CK::COURSE,
            &p.slot,
            "ridge-cap",
            p.s0,
            p.s1,
            vec![[hi[0] - 0.12, hi[1] - 0.01], [hi[0] + 0.12, hi[1] - 0.01], [hi[0], hi[1] + 0.05]],
            Some(&json!({ "roof": true })),
        );
    }
    let eave = slopes.iter().find(|p| p.kind == "roof-slope" && p.edge == Some(front.index));
    let spout_s = if r_spout < 0.5 { 0.12 } else { l - 0.12 };
    let pent_or_pitched = meta.truthy("pitched") || st.truthy("pent");
    if let Some(eave) = eave {
        let (lo, _) = slope_of(eave);
        let lo = lo.unwrap_or([f64::NAN, f64::NAN]);
        let y_e = profile_points(eave).iter().fold(f64::INFINITY, |m, q| m.min(q[1]));
        let outer = (lo[0] + 0.12).min(allowance - 0.03);
        let inner = outer - 0.14;
        cx.group = "roof-edge".into();
        let y_g = y_e - 0.02;
        cx.profile(
            plan,
            front,
            CK::TRIM,
            &pipe_slot,
            "gutter",
            0.02,
            l - 0.02,
            vec![
                [inner, y_g],
                [inner + 0.01, y_g - 0.07],
                [inner + 0.035, y_g - 0.1],
                [outer - 0.035, y_g - 0.1],
                [outer - 0.005, y_g - 0.06],
                [outer, y_g + 0.01],
                [outer - 0.012, y_g + 0.01],
                [inner + 0.012, y_g],
            ],
            None,
        );
        if style != "tract-postwar" && style != "tract-moderne" {
            cx.group = "corbels".into();
            let n = 2f64.max(js_round(l / 0.62)) as usize;
            let d = 0.08f64.max((inner - 0.02).min(0.26));
            for k in 0..n {
                let s = 0.3 + ((l - 0.6) * k as f64) / 1f64.max(n as f64 - 1.0);
                cx.profile(plan, front, CK::BRACKET, "wall", "eave-corbel", s - 0.05, s + 0.05, vec![[-0.01, y_e], [d, y_e], [d, y_e - 0.07], [d * 0.45, y_e - 0.2], [-0.01, y_e - 0.24]], None);
            }
        }
        cx.group = "roof-edge".into();
        let mid = (inner + outer) / 2.0;
        downspout(&cx, plan, front, &pipe_slot, spout_s, y_e - 0.1, lift, fitf(0.13), Some([mid, y_e - 0.1]));
    } else if !pent_or_pitched && copings.iter().any(|c| c.edge == Some(front.index)) {
        cx.group = "roof-edge".into();
        cx.bx(plan, front, CK::FIXTURE, ms, "scupper", [spout_s - 0.1, spout_s + 0.1, wall_top + 0.02, wall_top + 0.12, -0.01, fitf(0.07)], None);
        cx.bx(plan, front, CK::FIXTURE, &pipe_slot, "conductor-head", [spout_s - 0.13, spout_s + 0.13, wall_top - 0.3, wall_top, 0.0, fitf(0.17)], None);
        downspout(&cx, plan, front, &pipe_slot, spout_s, wall_top - 0.3, lift, fitf(0.13), None);
    }

    // 5. Storybook false gable: a louvred vent and a finial.
    if let Some(gp) = plan.parts.iter().find(|p| p.kind == "gable").cloned() {
        cx.group = "fixtures".into();
        let pts = plate_points(&gp);
        let mut apex = pts[0];
        for q in pts.iter().skip(1) {
            if q[1] > apex[1] {
                apex = *q;
            }
        }
        let vy = (wall_top + apex[1]) / 2.0 + 0.1;
        let roof = json!({ "roof": true });
        if apex[1] - wall_top > 0.9 {
            cx.bx(plan, front, CK::FIXTURE, "wall", "gable-vent", [apex[0] - 0.16, apex[0] + 0.16, vy - 0.16, vy + 0.16, 0.02, 0.05], Some(&roof));
            for k in 0..4 {
                let y = vy - 0.12 + k as f64 * 0.075;
                cx.bx(plan, front, CK::COURSE, ms, "vent-louvre", [apex[0] - 0.12, apex[0] + 0.12, y, y + 0.025, 0.03, 0.06], Some(&roof));
            }
        }
        cx.bx(plan, front, CK::FIXTURE, &tile_slot, "finial", [apex[0] - 0.05, apex[0] + 0.05, apex[1], apex[1] + 0.22, -0.1, 0.06], Some(&roof));
    }
    let bay_head = plan.parts.iter().find(|p| p.kind == "bay" && p.meta.s("part") == Some("moderne-bay-head")).cloned();
    let bay_apron = plan.parts.iter().find(|p| p.kind == "bay" && p.meta.s("part") == Some("moderne-bay-apron")).cloned();
    if let (Some(bh), Some(ba)) = (bay_head, bay_apron) {
        cx.group = "beads".into();
        let grow = |points: &[P2], by: f64| -> Vec<P2> {
            let n = points.len() as f64;
            let c = points.iter().fold(P2::new(0.0, 0.0), |a, q| P2::new(a.x + q.x / n, a.z + q.z / n));
            points
                .iter()
                .map(|q| {
                    let p2 = frame.to_plan(q.x, q.z);
                    let front_z = frame.to_plan(front.start.x, front.start.z).z;
                    let out = if p2.z - front_z > 0.01 { by } else { 0.0 };
                    let dx = q.x - c.x;
                    let dz = q.z - c.z;
                    let len = {
                        let l = dx.hypot(dz);
                        if l == 0.0 || l.is_nan() { 1.0 } else { l }
                    };
                    P2::new(q.x + (dx / len) * out, q.z + (dz / len) * out)
                })
                .collect()
        };
        let head_pts = match &bh.shape {
            Shape::Prism(p) => p.clone(),
            _ => vec![],
        };
        let fin = grow(&head_pts, 0.08f64.min(0.02f64.max(frame_allowance(frame) - MODERNE_BAY_DEPTH_M - 0.06)));
        cx.prism(plan, CK::TRIM, "wall", "bay-fin", fin.clone(), bh.y1 - 0.05, bh.y1);
        cx.prism(plan, CK::TRIM, "wall", "bay-sill-fin", fin, ba.y1 - 0.04, ba.y1);
    }

    // 6. Services.
    cx.group = "fixtures".into();
    let (_, front_ops) = record_for(plan, front.index);
    let mut ground_ops: Vec<&OpeningRecord> = front_ops.iter().filter(|o| o.storey == 0).collect();
    ground_ops.sort_by(|a, b| a.s0.partial_cmp(&b.s0).unwrap_or(std::cmp::Ordering::Equal));
    let mut gaps: Vec<[f64; 2]> = vec![];
    let mut cursor = 0.25;
    for o in &ground_ops {
        if o.s0 - 0.14 - cursor >= 0.42 {
            gaps.push([cursor, o.s0 - 0.14]);
        }
        cursor = f64::max(cursor, o.s1 + 0.14 + 0.1);
    }
    if l - 0.25 - cursor >= 0.42 {
        gaps.push([cursor, l - 0.25]);
    }
    let usable: Vec<[f64; 2]> = gaps.iter().filter(|[a, b]| !(spout_s > a - 0.15 && spout_s < b + 0.15) || b - a > 0.9).copied().collect();
    if !usable.is_empty() && !meta.truthy("retail") {
        let ui = (r_meter * usable.len() as f64).floor() as usize;
        let [a, b] = usable[ui % usable.len()];
        let c = if (spout_s - a).abs() < (spout_s - b).abs() { (b - 0.2).min(a + 0.35) } else { (a + 0.2).max(b - 0.35) };
        cx.bx(plan, front, CK::FIXTURE, ms, "meter-box", [c - 0.14, c + 0.14, lift + 1.25, lift + 1.72, -0.005, fitf(0.11)], None);
        cx.bx(plan, front, CK::FIXTURE, fs, "meter-dome", [c - 0.075, c + 0.075, lift + 1.4, lift + 1.58, fitf(0.1), fitf(0.17)], None);
        cx.bx(plan, front, CK::FIXTURE, ms, "conduit", [c - 0.022, c + 0.022, lift + 0.02, lift + 1.25, -0.005, fitf(0.045)], None);
        if b - a >= 0.62 {
            let gs = if c < (a + b) / 2.0 { 1.0 } else { -1.0 };
            let gx = c + gs * 0.18;
            cx.bx(plan, front, CK::FIXTURE, ms, "gas-riser", [gx - 0.02, gx + 0.02, lift, lift + 0.82, 0.02, fitf(0.06)], None);
            cx.bx(plan, front, CK::FIXTURE, ms, "gas-meter", [gx + gs * 0.03 - 0.12, gx + gs * 0.03 + 0.12, lift + 0.36, lift + 0.74, 0.03, fitf(0.24)], None);
            cx.bx(plan, front, CK::FIXTURE, ms, "gas-manifold", [gx - 0.14, gx + 0.14, lift + 0.74, lift + 0.8, 0.04, fitf(0.09)], None);
            cx.bx(plan, front, CK::FIXTURE, ms, "gas-regulator", [gx - gs * 0.1 - 0.05, gx - gs * 0.1 + 0.05, lift + 0.62, lift + 0.7, 0.04, fitf(0.14)], None);
        }
        if r_dryer < 0.7 {
            let other = if usable.len() > 1 { Some(usable[(ui + 1) % usable.len()]) } else { None };
            let ds = match other {
                Some(o) => (o[0] + o[1]) / 2.0,
                None => {
                    if c < (a + b) / 2.0 {
                        b - 0.12
                    } else {
                        a + 0.12
                    }
                }
            };
            if other.is_some() || b - a >= 0.95 {
                cx.bx(plan, front, CK::FIXTURE, ms, "dryer-vent", [ds - 0.08, ds + 0.08, lift + 0.42, lift + 0.58, -0.005, fitf(0.09)], None);
                cx.bx(plan, front, CK::FIXTURE, ms, "dryer-flap", [ds - 0.07, ds + 0.07, lift + 0.43, lift + 0.48, fitf(0.09), fitf(0.1)], None);
            }
        }
    }
    // Water table.
    cx.group = "base".into();
    if !meta.truthy("retail") {
        for edge in &edges {
            let (_, ops) = record_for(plan, edge.index);
            let mut cuts_at: Vec<[f64; 2]> = ops
                .iter()
                .filter(|o| o.storey == 0 && (o.type_ == "roller" || o.type_ == "entrance" || o.type_ == "storefront"))
                .map(|o| {
                    let pad = if o.type_ == "roller" { 0.1 } else { 0.0 };
                    [o.s0 - pad, o.s1 + pad]
                })
                .collect();
            cuts_at.sort_by(|p, q| p[0].partial_cmp(&q[0]).unwrap_or(std::cmp::Ordering::Equal));
            cuts_at.push([edge.length, edge.length]);
            let mut s = 0.0f64;
            for [a0, b] in cuts_at {
                let a = a0.min(edge.length);
                if a - s > 0.12 {
                    cx.profile(plan, *edge, CK::TRIM, &base_slot, "water-table", s, a, trim_profile("sloped", fit(0.03, edge), 0.32, lift, -0.01), None);
                }
                s = s.max(b);
            }
        }
    }
    // Corner beads.
    cx.group = "beads".into();
    let front_copings: Vec<&Part> = copings.iter().filter(|c| c.edge == Some(front.index)).collect();
    let bead_top = if front_copings.is_empty() { wall_top } else { front_copings.iter().fold(f64::INFINITY, |m, c| m.min(c.y0)) };
    let bead_from = lift + PEDESTRIAN_HEADROOM_M;
    if front.convex_start && bead_top - bead_from > 0.3 {
        cx.column(plan, front, CK::TRIM, "wall", "corner-bead", bead_section(0.0, 1.0), bead_from, bead_top, None);
    }
    if front.convex_end && bead_top - bead_from > 0.3 {
        cx.column(plan, front, CK::TRIM, "wall", "corner-bead", bead_section(l, -1.0), bead_from, bead_top, None);
    }

    let drops = fit_close_budget(plan, STUCCO_ROW_CLOSE_TRIANGLE_BUDGET - ROOF_PLANT_RESERVE_TRI, &STUCCO_CLOSE_DROP_ORDER);
    json!({ "glazing": glazing, "tileSegments": segments, "drops": drops })
}
