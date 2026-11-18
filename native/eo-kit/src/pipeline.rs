//! The factory: one source record to a planned kit building and its rung
//! meshes (src/world/buildings/archetypes/create-archetype-building.js and the
//! loaded-record shaping of src/citygen/sf-data.js).

use crate::archetypes;
use crate::classify::{self, ClassifyInput, Classified, RawRecord, TenancyUnit};
use crate::elevation::{Fenestration, Part, Plan, plan_elevation, resolve_elevation, resolve_elevation_raw};
use crate::frame::{Frame, FrameOptions, P2, footprint_frame};
use crate::js::*;
use crate::lod::{FENESTRATED_WALL_SLOT, reduce_plan_to_lod};
use crate::mesh::{Clamp, Sink, emit_part};
use crate::rng::{hash_seed, js_num};
use crate::street::{FrontChoice, MIN_STREET_RETURN_M, OcclusionIndex, StreetIndex, choose_archetype_front_edge, mark_lot_line_walls, promote_street_returns, wall_hidden_by_neighbours};
use std::collections::BTreeMap;

pub const ARCHETYPE_FRAME_ALLOWANCE_M: f64 = 0.9;
const LOT_LINE_BAY_M: f64 = 4.8;
const LOT_LINE_PROBE_M: f64 = 0.6;
const LOT_LINE_HEADROOM_M: f64 = 0.3;
const STREET_RETURN_OPENING_MIN_M: f64 = 5.0;
const STREET_RETURN_BAY_M: f64 = 4.2;
const STREET_RETURN_OPENING_ARCHETYPES: &[&str] =
    &["downtown-tower", "midcentury-stucco", "civic-concrete", "bay-window-apartment", "retail-strip", "soma-warehouse"];
/// `PAINTED_FENESTRATION.bayMetres` (kit/materials.js).
pub const PAINTED_FENESTRATION_BAY_M: f64 = 2.8;
pub const FENESTRATED_SUFFIX: &str = "+fenestrated";

/// The loaded record as sf-data.js publishes it and classify.js reads it.
#[derive(Clone, Debug)]
pub struct LoadedRecord {
    /// `sf-building-<key>`.
    pub id: String,
    /// The raw id as the file has it (`12345` or `ds12`).
    pub key: String,
    /// `flatToPoints(points)` (closing duplicate kept).
    pub polygon: Vec<P2>,
    pub height: f64,
    pub stories: f64,
    pub footprint_area: f64,
    pub usage: &'static str,
    pub program: &'static str,
    pub type_: &'static str,
    pub name: String,
    pub shop: String,
    pub amenity: String,
    pub year_built: f64,
    pub tenancy: Vec<TenancyUnit>,
}

/// The source record's fields (eo-data `Building` + tags), in JS terms.
pub struct SourceRecord<'a> {
    pub key: &'a str,
    pub points: &'a [f64],
    pub building: &'a str,
    pub amenity: &'a str,
    pub name: &'a str,
    pub levels: Option<f64>,
    pub height_raw: Option<f64>,
    pub area_raw: Option<f64>,
    /// The resolved height (eo-data `Building::height`, `max(3, resolveSourceHeight)`).
    pub height: f64,
}

pub fn flat_to_points(flat: &[f64]) -> Vec<P2> {
    let mut out = Vec::with_capacity(flat.len() / 2);
    let mut i = 0;
    while i + 1 < flat.len() {
        let (x, z) = (flat[i], flat[i + 1]);
        if x.is_finite() && z.is_finite() {
            out.push(P2::new(x, z));
        }
        i += 2;
    }
    out
}

pub fn polygon_area(p: &[P2]) -> f64 {
    let n = p.len();
    let mut a = 0.0;
    for i in 0..n {
        let (q, r) = (p[i], p[(i + 1) % n]);
        a += q.x * r.z - r.x * q.z;
    }
    (a / 2.0).abs()
}

/// `resolveSourceHeight(building, area)` (sf-data.js, with `defaultLevels`).
pub fn resolve_source_height(height: Option<f64>, levels: Option<f64>, area: f64) -> f64 {
    if let Some(h) = height.filter(|h| *h > 0.0) {
        return h;
    }
    let declared = levels.filter(|l| l.is_finite()).unwrap_or(0.0);
    let levels = if declared > 1.0 {
        declared
    } else if area > 2600.0 {
        8.0
    } else if area > 1200.0 {
        5.0
    } else if area > 450.0 {
        3.0
    } else {
        2.0
    };
    levels * 3.2
}

/// Shape a source record as `loadSfData` does (no civic sidecar, 1900 default year).
pub fn loaded_record(src: &SourceRecord, tenancy: Vec<TenancyUnit>) -> LoadedRecord {
    let mut polygon = flat_to_points(src.points);
    if polygon.len() < 3 && !polygon.is_empty() {
        polygon.push(polygon[0]);
    }
    let area = polygon_area(&polygon);
    // height, program and archetype read the unscaled area (a scaled
    // footprint keeps its building type and height and gains bays instead).
    let gs = eo_data::groundscale::scale();
    let area = if gs != 1.0 { area / (gs * gs) } else { area };
    // `Math.max(3, resolveSourceHeight(building, area))` at f64, on the loaded polygon's area.
    let _ = src.height;
    let height = resolve_source_height(src.height_raw, src.levels, area).max(3.0);
    let raw = RawRecord {
        id_text: src.key,
        building: src.building,
        amenity: src.amenity,
        name: src.name,
        levels: src.levels,
        height: src.height_raw,
        area: src.area_raw,
        points: &polygon,
    };
    let program = classify::infer_program(&raw, area);
    let type_ = classify::infer_type(&raw, program);
    let usage = classify::infer_usage(type_, program);
    LoadedRecord {
        id: format!("sf-building-{}", src.key),
        key: src.key.to_string(),
        polygon,
        height,
        stories: crate::js::js_round(height / 3.2).max(1.0),
        footprint_area: area,
        usage,
        program,
        type_,
        name: src.name.to_string(),
        shop: String::new(),
        amenity: src.amenity.to_string(),
        year_built: 1900.0,
        tenancy,
    }
}

/// `classifyLoadedBuilding(building)`.
pub fn classify_loaded(rec: &LoadedRecord) -> Option<Classified> {
    if rec.polygon.len() < 3 {
        return None;
    }
    let digits = rec.id.trim_start_matches(|c: char| !c.is_ascii_digit());
    let numeric = digits.parse::<f64>().ok().filter(|v| v.is_finite()).unwrap_or(0.0);
    let input = ClassifyInput {
        id_text: js_num(numeric),
        name: rec.name.clone(),
        building: if !rec.usage.is_empty() { rec.usage.to_string() } else if !rec.type_.is_empty() { rec.type_.to_string() } else { "yes".into() },
        amenity: rec.amenity.clone(),
        shop: rec.shop.clone(),
        levels: rec.stories,
        height: rec.height,
        area: rec.footprint_area,
        points: rec.polygon.clone(),
        trade_source: !rec.shop.is_empty() || !rec.amenity.is_empty(),
        year_built: rec.year_built,
        tenancy: rec.tenancy.clone(),
    };
    let mut c = classify::classify(&input);
    c.record_id = rec.id.clone();
    crate::typology::reclassify(&mut c);
    Some(c)
}

/// The world the planner reads: streets and every footprint.
pub struct KitWorld {
    pub streets: StreetIndex,
    pub occlusion: OcclusionIndex,
}

/// One planned building.
pub struct Planned {
    pub classified: Classified,
    pub frame: Frame,
    pub front: FrontChoice,
    pub street_returns: Vec<usize>,
    /// The spec the archetype module wrote (before the factory's passes).
    pub spec: J,
    pub plan: Plan,
}

/// `resolveArchetypeFrame` + `markLotLineWalls`.
pub fn resolve_frame(rec: &LoadedRecord, c: &Classified, world: &KitWorld, self_id: u32) -> Option<(Frame, FrontChoice, Vec<usize>)> {
    let opts = FrameOptions { allowance: Some(ARCHETYPE_FRAME_ALLOWANCE_M), ..Default::default() };
    let probe = footprint_frame(&rec.polygon, 0, &opts)?;
    let front = choose_archetype_front_edge(&probe, Some(&world.streets), Some(&world.occlusion), self_id);
    let mut frame = if front.index == probe.front_edge_index { probe } else { footprint_frame(&rec.polygon, front.index, &opts)? };
    if c.archetype == classify::STUCCO_ROW && front.source == "street" {
        if let Some((f, bays, notch)) = crate::row_house::row_house_front(&frame, ARCHETYPE_FRAME_ALLOWANCE_M) {
            frame = f;
            frame.row_house = Some(crate::frame::RowHouseInfo { bays, notch });
        }
    }
    frame.front_source = front.source;
    frame.front_distance = front.distance;
    frame.front_occluded = front.occluded;
    let shop_front = c.archetype == classify::RETAIL_STRIP && c.has_retail_ground;
    let returns = promote_street_returns(&mut frame, &world.streets, Some(&world.occlusion), self_id, shop_front);
    Some((frame, front, returns))
}

fn is_blind_elevation(entry: &J) -> bool {
    match entry {
        J::Null => true,
        J::String(s) => s == "blind",
        other => {
            if other.s("window").unwrap_or("punched") != "blind" {
                return false;
            }
            if let Some(g) = other.at("ground") {
                if g.as_str() != Some("blind") {
                    return false;
                }
            }
            if truthy(other.at("storeys")) || other.arr("bays").is_some() {
                return false;
            }
            true
        }
    }
}

fn front_source_elevation(spec: &J, frame: &Frame) -> J {
    frame.edge_for_role("front").map(|e| resolve_elevation_raw(spec, e)).filter(|v| v.is_object()).unwrap_or(json!({}))
}

fn is_ribbon(src: &J) -> bool {
    src.s("window") == Some("ribbon") || src.arr("storeys").map(|a| a.iter().any(|v| v.as_str() == Some("ribbon"))).unwrap_or(false)
}

fn lot_line_elevation(spec: &J, frame: &Frame) -> J {
    let src = front_source_elevation(spec, frame);
    let ribbon = is_ribbon(&src);
    let lot_bay = num_or(spec.get_nn("archetype").at("lotLineBay"), LOT_LINE_BAY_M).max(num_or(src.at("bayWidth"), 0.0));
    let mut elev = json!({
        "window": if ribbon { "ribbon" } else { "punched" },
        "ground": "blind",
        "bayWidth": lot_bay,
        "endPier": 0.9,
        "cornice": false,
        "downpipes": false,
    });
    if let Some(r) = src.at("reveal") {
        set(&mut elev, "reveal", r.clone());
    }
    if ribbon {
        if truthy(src.at("ribbonSpec")) {
            set(&mut elev, "ribbonSpec", src["ribbonSpec"].clone());
        }
    } else {
        let mut ws = spread(src.get_nn("windowSpec").filter(|v| truthy(Some(v))), None);
        set(&mut ws, "mullions", json!(0));
        set(&mut ws, "transoms", json!(0));
        set(&mut elev, "windowSpec", ws);
        set(&mut elev, "sill", json!({ "height": 0.06, "projection": 0.04, "overhang": 0.04, "profile": "flat" }));
    }
    elev
}

fn storey_bottoms(spec: &J) -> Option<Vec<f64>> {
    let list = spec.arr("storeys")?;
    if list.len() < 2 {
        return None;
    }
    let mut y = num_or(spec.at("floorLevel"), 0.0).max(0.0);
    Some(
        list.iter()
            .map(|entry| {
                let y0 = y;
                let h = if entry.is_number() { to_number(Some(entry)) } else { to_number(entry.at("height")) };
                y += num_or(Some(&jn(h)), 3.2).max(2.2);
                y0
            })
            .collect(),
    )
}

fn by_edge_mut<'a>(spec: &'a mut J) -> &'a mut J {
    let elev = obj_mut(spec, "elevations");
    obj_mut(elev, "byEdge")
}

fn open_lot_line_walls(spec: &mut J, frame: &Frame, occ: &OcclusionIndex, self_id: u32) -> Vec<usize> {
    let mut opened = vec![];
    let Some(bottoms) = storey_bottoms(spec) else { return opened };
    let mut base: Option<J> = None;
    for edge in &frame.edges {
        if edge.lot_line.is_none() {
            continue;
        }
        if !is_blind_elevation(&resolve_elevation_raw(spec, edge)) {
            continue;
        }
        if base.is_none() {
            base = Some(lot_line_elevation(spec, frame));
        }
        let b = base.as_ref().unwrap();
        let end_pier = b.num("endPier");
        let bay_width = b.num("bayWidth");
        let available = (edge.length - 2.0 * end_pier).max(0.0);
        let count = ((available / bay_width).floor() as i64).max(1);
        let width = available / count as f64;
        let mut bays = vec![];
        let mut any_open = false;
        for i in 0..count {
            let s0 = end_pier + i as f64 * width;
            let mut top = 0.0f64;
            for f in [0.2, 0.5, 0.8] {
                let s = s0 + width * f;
                if let Some(c) = occ.cover_top(edge.start.x + edge.u.x * s + edge.n.x * LOT_LINE_PROBE_M, edge.start.z + edge.u.z * s + edge.n.z * LOT_LINE_PROBE_M, self_id) {
                    top = top.max(c);
                }
            }
            let mut storeys = Map::new();
            for (s, y0) in bottoms.iter().enumerate() {
                if s == 0 || !(*y0 >= top + LOT_LINE_HEADROOM_M) {
                    storeys.insert(s.to_string(), json!("blind"));
                } else {
                    any_open = true;
                }
            }
            bays.push(json!({ "width": width, "storeys": J::Object(storeys) }));
        }
        if !any_open {
            continue;
        }
        let mut entry = b.clone();
        set(&mut entry, "bays", J::Array(bays));
        set(by_edge_mut(spec), &edge.index.to_string(), entry);
        opened.push(edge.index);
    }
    opened
}

fn street_return_elevation(spec: &J, frame: &Frame) -> J {
    let src = front_source_elevation(spec, frame);
    let ribbon = is_ribbon(&src);
    let mut elev = json!({
        "window": if ribbon { "ribbon" } else { "punched" },
        "ground": "punched",
        "bayWidth": STREET_RETURN_BAY_M.max(num_or(src.at("bayWidth"), 0.0)),
        "endPier": 0.8,
        "cornice": false,
        "downpipes": false,
    });
    if let Some(r) = src.at("reveal") {
        set(&mut elev, "reveal", r.clone());
    }
    if ribbon {
        if truthy(src.at("ribbonSpec")) {
            set(&mut elev, "ribbonSpec", src["ribbonSpec"].clone());
        }
    } else {
        let mut ws = spread(src.get_nn("windowSpec").filter(|v| truthy(Some(v))), None);
        let m = num_or(ws.at("mullions"), 0.0).min(1.0);
        set(&mut ws, "mullions", jn(m));
        set(&mut ws, "transoms", json!(0));
        if let Some(o) = ws.as_object_mut() {
            o.shift_remove("transomAt");
        }
        set(&mut elev, "windowSpec", ws);
        let sill = if src.at("sill").map(|v| v.is_object()).unwrap_or(false) {
            src["sill"].clone()
        } else {
            json!({ "height": 0.07, "projection": 0.05, "overhang": 0.05, "profile": "sloped" })
        };
        set(&mut elev, "sill", sill);
    }
    elev
}

fn open_street_return_walls(spec: &mut J, frame: &Frame, archetype: &str, occ: &OcclusionIndex, self_id: u32, height: f64) -> Vec<usize> {
    let mut opened = vec![];
    if !STREET_RETURN_OPENING_ARCHETYPES.contains(&archetype) {
        return opened;
    }
    let mut base: Option<J> = None;
    for edge in &frame.edges {
        if edge.role != "front-secondary" || edge.party || edge.lot_line.is_some() {
            continue;
        }
        if !(edge.length >= STREET_RETURN_OPENING_MIN_M) {
            continue;
        }
        if !is_blind_elevation(&resolve_elevation_raw(spec, edge)) {
            continue;
        }
        if wall_hidden_by_neighbours(edge, Some(occ), self_id, Some(height)) {
            continue;
        }
        if base.is_none() {
            base = Some(street_return_elevation(spec, frame));
        }
        let entry = base.clone().unwrap();
        set(by_edge_mut(spec), &edge.index.to_string(), entry);
        opened.push(edge.index);
    }
    opened
}

/// `isOpeningPart(part)`.
pub fn is_opening_part(part: &Part) -> bool {
    let k = part.kind.as_str();
    const DRESS: [&str; 13] = ["frame", "sill", "hood", "guide", "bar", "threshold", "recess", "canopy", "slat", "rod", "reveal", "", ""];
    if DRESS.iter().filter(|d| !d.is_empty()).any(|d| k.contains(d)) {
        return false;
    }
    ["glass", "door-leaf", "roller-door", "window", "storefront", "pane"].iter().any(|d| k.contains(d))
}

pub fn kit_spec_is_glass(name: &str) -> Option<bool> {
    crate::kit_material_kinds().get(name).map(|k| k == "glass")
}

/// `baseAuthoredMaterialName`.
pub fn base_material_name(name: &str) -> &str {
    name.strip_suffix(FENESTRATED_SUFFIX).unwrap_or(name)
}

/// `fenestratedMaterialName`.
pub fn fenestrated_material_name(name: &str) -> Option<String> {
    let base = base_material_name(name);
    match kit_spec_is_glass(base) {
        Some(false) => Some(format!("{base}{FENESTRATED_SUFFIX}")),
        _ => None,
    }
}

/// `fenestrateBlindWalls(plan, frame, { occlusion, selfId, height })`.
pub fn fenestrate_blind_walls(plan: &mut Plan, frame: &Frame, occ: &OcclusionIndex, self_id: u32, height: f64) -> Vec<usize> {
    let mut painted = vec![];
    let mut openings: BTreeMap<usize, usize> = BTreeMap::new();
    for p in &plan.parts {
        if let Some(e) = p.edge {
            if is_opening_part(p) {
                *openings.entry(e).or_default() += 1;
            }
        }
    }
    let storeys = plan.storeys.clone();
    let base = plan.base_y;
    let upper = storeys.iter().find(|s| s.index == 1).cloned();
    let storey_at = |y: f64| -> f64 {
        let h = y - base;
        for (k, s) in storeys.iter().enumerate() {
            if h <= s.y1 + 1e-6 || k == storeys.len() - 1 {
                return s.index as f64 + (h - s.y0) / s.height.max(1e-6);
            }
        }
        0.0
    };
    let spans_upper = |p: &Part| {
        upper.as_ref().map(|u| p.storey.is_none() && p.y0 - base <= u.y0 + 1e-6 && p.y1 - base >= u.y0 + 1f64.min(u.height)).unwrap_or(false)
    };
    let mut by_edge: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (i, p) in plan.parts.iter().enumerate() {
        if p.kind != "wall" || p.edge.is_none() || p.fenestration.is_some() {
            continue;
        }
        if !(p.storey.map(|s| s >= 1).unwrap_or(false)) && !spans_upper(p) {
            continue;
        }
        if !p.is_box() {
            continue;
        }
        by_edge.entry(p.edge.unwrap()).or_default().push(i);
    }
    for edge in &frame.edges {
        if !(edge.length >= MIN_STREET_RETURN_M) {
            continue;
        }
        if openings.get(&edge.index).copied().unwrap_or(0) > 0 {
            continue;
        }
        let Some(bands) = by_edge.get(&edge.index) else { continue };
        if bands.is_empty() {
            continue;
        }
        if wall_hidden_by_neighbours(edge, Some(occ), self_id, Some(height)) {
            continue;
        }
        let bays = crate::js::js_round(edge.length / PAINTED_FENESTRATION_BAY_M).max(1.0);
        let mut any = false;
        for &i in bands {
            let slot = format!("fenestrated:{}", plan.parts[i].slot);
            if !plan.materials.contains_key(&slot) {
                let src = plan.materials.get(&plan.parts[i].slot).or(plan.materials.get("wall")).cloned().unwrap_or_default();
                let Some(name) = fenestrated_material_name(&src) else { continue };
                plan.materials.insert(slot.clone(), name);
            }
            let p = &mut plan.parts[i];
            let v = match p.storey {
                Some(s) if s >= 1 => [s as f64, s as f64 + 1.0],
                _ => [storey_at(p.y0), storey_at(p.y1)],
            };
            p.fenestration = Some(Fenestration { u: [(p.s0 / edge.length) * bays, (p.s1 / edge.length) * bays], v, storeys: None });
            p.slot = slot;
            any = true;
        }
        if any {
            painted.push(edge.index);
        }
    }
    painted
}

/// `planArchetypeBuilding(building, { baseY, streets, neighbours })`.
pub fn plan_building(rec: &LoadedRecord, world: &KitWorld, self_id: u32, base_y: f64) -> Option<Planned> {
    let c = classify_loaded(rec)?;
    let (mut frame, front, street_returns) = resolve_frame(rec, &c, world, self_id)?;
    mark_lot_line_walls(&mut frame, Some(&world.occlusion), self_id, c.height);
    // planArchetypeWithLotLines
    let spec0 = archetypes::spec_for(c.archetype, &c, &frame);
    if spec0.is_null() {
        return None;
    }
    let mut spec = spec0.clone();
    let opened = open_lot_line_walls(&mut spec, &frame, &world.occlusion, self_id);
    let returns = open_street_return_walls(&mut spec, &frame, c.archetype, &world.occlusion, self_id, c.height);
    let roof = spread(Some(&json!({ "sideFaces": false })), spec.get_nn("roof"));
    set(&mut spec, "roof", roof);
    crate::shop_ground::prepare_spec(&mut spec, &frame, &c, Some(&world.occlusion), self_id);
    let mut plan = plan_elevation(&spec, &frame, base_y);
    archetypes::detail_for(c.archetype, &mut plan, &frame, &spec);
    crate::shop_ground::add(&mut plan, &frame, &spec);
    plan.extra.insert("lotLineEdges".into(), json!(opened));
    plan.extra.insert("streetReturnEdges".into(), json!(returns));
    plan.id = rec.id.clone();
    crate::roof_plant::add(&mut plan, &frame, &c);
    let fen = fenestrate_blind_walls(&mut plan, &frame, &world.occlusion, self_id, c.height);
    plan.extra.insert("fenestratedEdges".into(), json!(fen));
    if let Some(name) = plan.material("wall").and_then(fenestrated_material_name) {
        plan.materials.insert(FENESTRATED_WALL_SLOT.into(), name);
    }
    Some(Planned { classified: c, frame, front, street_returns, spec: spec0, plan })
}

// ---------------------------------------------------------------------------
// Materials as createArchetypeBuilding draws them.
// ---------------------------------------------------------------------------

const HOLLOW_SHELL_GLASS: [(&str, &str); 4] =
    [("glass-clear", "glass-shell"), ("glass-door", "glass-shell"), ("glass-low-iron", "glass-shell"), ("glass-film", "glass-shell")];
const PAINTED_WALL_FINISHES: [&str; 6] = ["wood-siding", "painted-stucco", "float-finish-stucco", "scanned-painted-plaster", "painted-clapboard", "painted-shingle"];
const PAINT_TINTS: [u32; 16] = [
    0xffffff, 0xffffff, 0xffffff, 0xe6e6e4, 0xe6e6e4, 0xdfe7f0, 0xd6e1ee, 0xe4ecd8, 0xfff2cf, 0xf6e2dc, 0xf2e4cc, 0xe2dcd2, 0xbfc6cf, 0xc3ccb4, 0xd8c6a4, 0xc9c4bc,
];
const PAINTED_TIMBER_WALL_SLOTS: [&str; 5] = ["wall", "parapet", "spandrel", "recess", "bulkhead"];

/// Material name -> sRGB paint multiplier.
pub type Tints = BTreeMap<String, u32>;

fn tint_of(v: &J) -> Option<u32> {
    match v {
        J::Number(n) => n.as_f64().map(|f| f as u32),
        J::String(s) => u32::from_str_radix(s.trim_start_matches('#'), 16).ok(),
        _ => None,
    }
}

/// `archetypeTintsFor(plan, seed)`.
pub fn archetype_tints_for(plan: &Plan, seed: u32) -> Option<Tints> {
    archetype_tints(plan.extra.get("tints"), &plan.materials, seed)
}

/// `archetypeTintsFor` on the plan's own tints and a material table.
pub fn archetype_tints(own: Option<&J>, materials: &BTreeMap<String, String>, seed: u32) -> Option<Tints> {
    if let Some(J::Object(own)) = own {
        let mut out = Tints::new();
        for (name, t) in own {
            if !PAINTED_WALL_FINISHES.contains(&name.as_str()) {
                continue;
            }
            let Some(t) = tint_of(t) else { continue };
            if t == 0xffffff {
                continue;
            }
            out.insert(name.clone(), t);
        }
        return (!out.is_empty()).then_some(out);
    }
    let wall = materials.get("wall").map(|s| s.as_str())?;
    if !PAINTED_WALL_FINISHES.contains(&wall) {
        return None;
    }
    let t = PAINT_TINTS[(hash_seed(&format!("{seed}|archetype-paint")) as usize) % PAINT_TINTS.len()];
    (t != 0xffffff).then(|| Tints::from([(wall.to_string(), t)]))
}

/// `paintedTimberAsClapboard(materials, tints)`.
pub fn painted_timber_as_clapboard(materials: &mut BTreeMap<String, String>, tints: Option<Tints>) -> Option<Tints> {
    let Some(tint) = tints.as_ref().and_then(|t| t.get("wood-siding")).copied() else { return tints };
    let mut moved = false;
    for slot in PAINTED_TIMBER_WALL_SLOTS {
        if materials.get(slot).map(|s| s == "wood-siding").unwrap_or(false) {
            materials.insert(slot.into(), "painted-clapboard".into());
            moved = true;
        }
    }
    if !moved {
        return tints;
    }
    let timber = format!("wood-siding{FENESTRATED_SUFFIX}");
    for slot in PAINTED_TIMBER_WALL_SLOTS {
        let key = format!("fenestrated:{slot}");
        if materials.get(&key) == Some(&timber) {
            materials.insert(key, format!("painted-clapboard{FENESTRATED_SUFFIX}"));
        }
    }
    let mut out = tints.unwrap();
    out.remove("wood-siding");
    out.entry("painted-clapboard".into()).or_insert(tint);
    Some(out)
}

const ROOF_WEIGHTS_COMMERCIAL: [(&str, f64); 6] =
    [("membrane-white", 0.22), ("membrane-light-grey", 0.22), ("membrane-silver", 0.12), ("membrane-aged", 0.16), ("cap-sheet-grey", 0.1), ("tar-gravel", 0.18)];
const ROOF_WEIGHTS_RESIDENTIAL: [(&str, f64); 6] =
    [("membrane-white", 0.06), ("membrane-light-grey", 0.12), ("membrane-silver", 0.1), ("membrane-aged", 0.17), ("cap-sheet-grey", 0.2), ("tar-gravel", 0.35)];

/// A roof finish id's material and tint (`ROOF_FINISHES`).
pub fn roof_finish(id: &str) -> (&'static str, Option<u32>) {
    match id {
        "membrane-white" => ("membrane-roof", None),
        "membrane-light-grey" => ("membrane-roof", Some(0xdcdad5)),
        "membrane-silver" => ("membrane-roof", Some(0xc4c9cd)),
        "membrane-aged" => ("membrane-roof", Some(0xa9a7a1)),
        "cap-sheet-grey" => ("membrane-roof", Some(0x7d7a74)),
        _ => ("asphalt-roof", None),
    }
}

/// `roofFinishIdFor(archetype, seed)`.
pub fn roof_finish_id_for(archetype: &str, seed: u32) -> Option<&'static str> {
    if archetype.is_empty() || archetype == "cottage-garage" {
        return None;
    }
    let commercial = ["soma-warehouse", "retail-strip", "civic-concrete", "downtown-tower", "industrial-shed"].contains(&archetype);
    let table = if commercial { &ROOF_WEIGHTS_COMMERCIAL } else { &ROOF_WEIGHTS_RESIDENTIAL };
    let total: f64 = table.iter().map(|t| t.1).sum();
    let x = (hash_seed(&format!("{seed}|roof-finish")) % 100000) as f64 / 100000.0 * total;
    let mut acc = 0.0;
    for (id, w) in table.iter() {
        acc += w;
        if x < acc {
            return Some(id);
        }
    }
    Some(table[table.len() - 1].0)
}

/// The material slots, tints and roof finish a rung draws with.
#[derive(Clone, Debug, Default)]
pub struct DrawnMaterials {
    pub materials: BTreeMap<String, String>,
    pub tints: Tints,
    pub roof_finish: Option<&'static str>,
}

/// createArchetypeBuilding's material steps for one rung.
pub fn drawn_materials(planned: &Planned, rung: u32) -> DrawnMaterials {
    let plan = &planned.plan;
    let c = &planned.classified;
    let mut materials = plan.materials.clone();
    if rung >= 2 && plan.extra.get("shopGround").map(|v| truthy(Some(v))).unwrap_or(false) {
        materials = crate::shop_ground::far_materials(&materials);
    }
    for v in materials.values_mut() {
        if let Some((_, to)) = HOLLOW_SHELL_GLASS.iter().find(|(from, _)| *from == v.as_str()) {
            *v = to.to_string();
        }
    }
    let paint = archetype_tints(plan.extra.get("tints"), &materials, c.seed);
    let wall_tints = if c.civic.is_some() { painted_timber_as_clapboard(&mut materials, paint) } else { paint };
    // applyRoofFinish
    let meta = plan.extra.get("archetype");
    let skip = !materials.contains_key("roof")
        || meta.s("roof") == Some("gable")
        || truthy(meta.at("pitched"))
        || meta.at("roofPlant") == Some(&J::Bool(false));
    let mut tints = wall_tints.unwrap_or_default();
    // the wall paint per building on a neutral primer (wall_paint.rs; EO_WALL_PAINT=off: the port's tints).
    if crate::wall_paint::enabled() {
        crate::wall_paint::apply(c, &materials, matches!(plan.extra.get("tints"), Some(J::Object(_))), &mut tints);
    }
    // a stone or precast face's per-building tone (typology.rs: a tower's stone), not a paint.
    if let Some(J::Object(st)) = plan.extra.get("stoneTints") {
        for (name, t) in st {
            if let Some(t) = tint_of(t) {
                tints.insert(name.clone(), t);
                tints.insert(format!("{name}{FENESTRATED_SUFFIX}"), t);
            }
        }
    }
    let mut finish = None;
    if !skip {
        if let Some(id) = roof_finish_id_for(c.archetype, c.seed) {
            let (material, tint) = roof_finish(id);
            materials.insert("roof".into(), material.into());
            if let Some(t) = tint {
                tints.insert(material.into(), t);
            }
            finish = Some(id);
        }
    }
    // the roof colour measured from aerial imagery, when a table is installed (none: as before).
    if let Some(m) = eo_data::roof_colours::table().and_then(|t| t.get(plan.id.strip_prefix("sf-building-").unwrap_or(&plan.id))) {
        crate::roof_measured::apply(&mut materials, &mut tints, &m, !skip && finish.is_some());
    }
    DrawnMaterials { materials, tints, roof_finish: finish }
}

/// The parts a rung draws (rung 0-2; rung 3 is the renderer's footprint box).
pub fn rung_parts(planned: &Planned, rung: u32) -> Vec<Part> {
    let mut parts = reduce_plan_to_lod(&planned.plan, &planned.frame, rung);
    if rung <= 1 {
        parts.extend(planned.plan.stoop_parts.iter().cloned());
    }
    parts
}

/// One rung as triangles grouped by drawn material name, with the paint per material.
#[derive(Clone, Debug, Default)]
pub struct RungMesh {
    /// (material name as drawn, its paint, its triangles), in first-use order.
    pub groups: Vec<(String, Option<u32>, Sink)>,
    pub clamped_vertices: usize,
}

impl RungMesh {
    pub fn triangles(&self) -> usize {
        self.groups.iter().map(|g| g.2.triangles()).sum()
    }
    pub fn vertices(&self) -> usize {
        self.groups.iter().map(|g| g.2.positions.len()).sum()
    }
}

/// Emit a rung: every part through the build clamp into one sink per material.
pub fn build_rung(planned: &Planned, rung: u32) -> RungMesh {
    let drawn = drawn_materials(planned, rung);
    let parts = rung_parts(planned, rung);
    let mut clamp = Clamp::new(&planned.plan, &planned.frame);
    let mut out = RungMesh::default();
    let wall = drawn.materials.get("wall").cloned().unwrap_or_else(|| "brick-running-bond".into());
    for part in &parts {
        let name = drawn.materials.get(&part.slot).cloned().filter(|s| !s.is_empty()).unwrap_or_else(|| wall.clone());
        let k = match out.groups.iter().position(|g| g.0 == name) {
            Some(k) => k,
            None => {
                let tint = drawn.tints.get(&name).copied();
                out.groups.push((name.clone(), tint, Sink::default()));
                out.groups.len() - 1
            }
        };
        emit_part(&mut out.groups[k].2, part, &mut clamp);
    }
    out.clamped_vertices = clamp.clamped;
    out
}

/// Resolve the street-return elevation helper for tests.
pub fn elevation_for_edge(spec: &J, frame: &Frame, edge: usize) -> J {
    resolve_elevation(spec, &frame.edges[edge])
}
