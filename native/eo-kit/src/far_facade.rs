//! a building's facade as the far city draws it on its footprint box
//! (past the kit's mid rung, 470 m, and wherever the far layer is not drawn):
//! the archetype's own window rhythm, storey heights, glazing, wall and frame
//! finishes and paint, read from the same spec the planner starts from, so the
//! box continues the rung it replaces instead of a palette of its own.
//!
//! One record in, one [`FarFacade`] out, with nothing but the record: the
//! classification (`classify_loaded`), the archetype's spec on the record's
//! own frame (`archetypes::spec_for`, front edge 0: the street index is not
//! loaded when the far boxes are built) and the kit's material steps
//! (`pipeline::archetype_tints`, `wall_paint::apply`, the roof finish and the
//! measured roof colour). No plan, no mesh: about 30 microseconds a record.
//!
//! What is summarised, per building:
//!
//! * storeys: the ground storey and the median upper storey as the spec
//!   lists them; a downtown tower's shaft is one spec storey, drawn in the
//!   tower's own `floorHeight`; any other storey over 6 m in floors of about
//!   3.8 m (`TOWER_FLOOR_M`).
//! * the window rhythm of the front elevation (or the first glazed one): bay
//!   width, window width share (`widthRatio`, or the opening over the bay),
//!   sill and head (`sillHeight`, `head` / `headClear`), lights per window
//!   (`mullions` + 1), ribbon spandrels (`spandrelHeight`), and for a tower
//!   its family's pier spacing, pier width and spandrel band.
//! * the class the far shader draws: punched, a row of bay windows, ribbon,
//!   curtain wall, a masonry grid between piers, industrial sash, a shed's
//!   clerestory, or blind.
//! * the wall colour as the kit draws it (the finish's colour, or the neutral
//!   primer under a paint, times the paint), the frame finish, the glass kind,
//!   and the roof colour (the roof finish, toward the aerial measurement).
//! * lot-line walls: a row house on a narrow lot (short side under
//!   `ROW_LOT_MAX_M`, sides `ROW_ASPECT` times longer) has blind side walls,
//!   as the city's 25 ft lots have: the building code admits no openings in
//!   an exterior wall under 3 ft from the lot line (Table 705.8 of the model
//!   building code the California Building Code adopts, fire separation
//!   distance 0 to under 3 ft: openings not permitted).
//!
//! Switch: `EO_FAR_FACADE=off` (eo-app `--no-far-facade`): the far boxes as
//! before (one wall colour from the far ring's palette, the roof in the wall
//! colour, no windows).

use crate::archetypes;
use crate::classify::{self, Classified};
use crate::frame::{FrameOptions, footprint_frame};
use crate::js::*;
use crate::pipeline::{self, ARCHETYPE_FRAME_ALLOWANCE_M, LoadedRecord, SourceRecord, Tints};
use crate::rng::js_num;
use eo_data::roof_colours as rc;
use std::collections::BTreeMap;

/// Bump when what a far facade summarises changes.
pub const VERSION: u32 = 1;

/// `EO_FAR_FACADE=off` (or 0) turns the far facade off (eo-app `--no-far-facade`).
pub fn enabled() -> bool {
    !matches!(std::env::var("EO_FAR_FACADE").as_deref(), Ok("off") | Ok("0"))
}

/// What the far shader draws on a wall.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FacadeClass {
    /// Punched windows in a painted or masonry wall.
    Punched = 0,
    /// Punched windows with a canted bay window every second bay (the timber-era rows).
    RowBays = 1,
    /// A glass band per floor over a spandrel, piers at the bay.
    Ribbon = 2,
    /// Glass to the floor edge with a spandrel band and mullions (a curtain wall).
    Curtain = 3,
    /// Two-light windows between masonry piers (a prewar tower's grid).
    MasonryGrid = 4,
    /// Wide multi-light steel sash (a warehouse loft).
    Industrial = 5,
    /// Blind walls with a clerestory band in the top storey.
    Shed = 6,
    /// No windows.
    Blind = 7,
}

/// The glass the far shader draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum GlassKind {
    /// Clear double glazing in a home: blinds and curtains behind it.
    Domestic = 0,
    /// Clear glass in an office or shop: a dark room by day.
    Office = 1,
    /// A tower's coated (low-e or reflective) glass.
    Coated = 2,
    /// A dark tinted or filmed pane (`glass-film`).
    Tinted = 3,
}

/// The frame and trim finishes the far shader knows (an index into its
/// palette, far_box.wgsl `FB_TRIM`; colours from kit-material-specs.json).
pub const TRIM_FINISHES: [&str; 8] = ["painted-white", "anodized-aluminium", "painted-steel-mid", "cast-stone", "painted-steel", "signal-red", "", "painted-steel-light"];
/// Palette slot of a frame finish the palette lacks: the wall's own tone, darker.
pub const TRIM_WALL_TONE: u8 = 6;

/// A building's facade as the far box draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct FarFacade {
    pub archetype: &'static str,
    pub class: FacadeClass,
    pub glass: GlassKind,
    /// The wall as the kit draws it, linear albedo.
    pub wall: [f32; 3],
    /// The roof cap, linear albedo (the finish, toward the measured colour).
    pub roof: [f32; 3],
    /// Frame and trim finish (`TRIM_FINISHES` index, or `TRIM_WALL_TONE`).
    pub trim: u8,
    /// Floor to floor: the ground storey and a typical upper storey, metres.
    pub ground_m: f32,
    pub storey_m: f32,
    /// The band over the top storey (parapet and coping, a tower's crown), metres.
    pub top_m: f32,
    /// The window module along a wall, metres.
    pub bay_m: f32,
    /// Window width over the bay, height over the storey, sill over the storey.
    pub window_w: f32,
    pub window_h: f32,
    pub sill: f32,
    /// Lights across one window (mullions + 1), 1-4.
    pub lights: u8,
    /// A shop front (or a lobby) in the ground storey.
    pub shop_ground: bool,
    /// A narrow row lot: walls along this direction (unit, x z) are lot-line walls.
    pub lot_axis: Option<[f64; 2]>,
}

/// A row lot is at most this wide (SF Planning: the historical increment of
/// the city's residential development is a 25-30 ft lot, Guidelines for
/// Ground Floor Residential Design; a standard lot is 25 ft by 100 ft,
/// 7.6 m by 30.5 m, SB 9 Objective Design Standards, which split a 50 ft lot
/// into two) and its sides at least this many times its front.
pub const ROW_LOT_MAX_M: f64 = 16.0;
pub const ROW_ASPECT: f64 = 1.5;
/// The floor height a multi-floor spec storey is drawn in (the CTBUH Tall
/// Building Height Calculator takes 3.9 m floor to floor for an office, 3.1 m
/// for a residence; the kit's towers plan 3.45-4.05 m; hlod_bake.rs `FLOOR_M`).
pub const TOWER_FLOOR_M: f64 = 3.8;
/// A pane of a curtain wall (the kit's `PANE_M` for the far layer, a 5 ft module).
pub const CURTAIN_PANE_M: f64 = 1.5;

const ROW_ARCHETYPES: [&str; 4] = [classify::VICTORIAN_ROW, classify::EDWARDIAN_FLATS, classify::STUCCO_ROW, classify::BAY_WINDOW_APARTMENT];

fn median(mut v: Vec<f64>) -> Option<f64> {
    v.retain(|x| x.is_finite());
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    Some(v[v.len() / 2])
}

fn hex_linear(h: u32) -> [f32; 3] {
    rc::rgb8_linear([(h >> 16) as u8, (h >> 8) as u8, h as u8])
}

fn mul(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2]]
}

/// A material as the kit draws it unweathered (linear): its finish colour,
/// or the primer under a paint, times its paint.
pub fn drawn_colour(name: &str, tints: &Tints) -> Option<[f32; 3]> {
    let base = crate::wall_paint::drawn_base(name).or_else(|| crate::roof_measured::material_colour(name))?;
    let t = tints.get(name).or_else(|| tints.get(pipeline::base_material_name(name))).copied().map(hex_linear).unwrap_or([1.0; 3]);
    Some(mul(base, t))
}

/// The plan's own tints (`plan.extra.tints`) as the archetype's detail pass
/// writes them, from the spec: the period paint of the timber-era rows, a
/// stucco row's paint, a Chinatown front's paint, a revival shop's paint.
fn own_tints(c: &Classified, spec: &J) -> Option<J> {
    let mats = spec.at("materials");
    let wall = mats.s("wall");
    let meta = spec.at("archetype");
    match c.archetype {
        classify::VICTORIAN_ROW | classify::EDWARDIAN_FLATS => {
            let seed = match spec.get_nn("seed").or(spec.get_nn("id")) {
                Some(J::Number(n)) => js_num(n.as_f64().unwrap_or(0.0)),
                Some(J::String(s)) => s.clone(),
                _ => return Some(json!({})),
            };
            Some(archetypes::period_paint::period_paint_tints(&seed, wall).unwrap_or_else(|| json!({})))
        }
        classify::STUCCO_ROW => {
            let mut m = Map::new();
            m.insert(meta.s("wall").unwrap_or("").to_string(), meta.at("paint").cloned().unwrap_or(J::Null));
            Some(J::Object(m))
        }
        classify::BAY_WINDOW_APARTMENT => {
            let paint = meta.at("chinatown").f("paint").unwrap_or(f64::NAN);
            let w = meta.s("wall").unwrap_or("");
            if paint == 0xf2e6cf as f64 || w != "painted-stucco" {
                Some(json!({}))
            } else {
                let mut m = Map::new();
                m.insert(w.to_string(), jn(paint));
                Some(J::Object(m))
            }
        }
        classify::RETAIL_STRIP if meta.truthy("revival") => match meta.get_nn("revivalPaint").filter(|v| truthy(Some(v))) {
            Some(p) => {
                let mut m = Map::new();
                m.insert(meta.s("wall").unwrap_or("undefined").to_string(), p.clone());
                Some(J::Object(m))
            }
            None => Some(json!({})),
        },
        _ => None,
    }
}

/// The front elevation of a spec, or the first glazed one (a tower's are by edge).
fn front_elevation(spec: &J) -> J {
    let els = spec.at("elevations");
    if let Some(f) = els.at("front").filter(|v| v.is_object()) {
        return f.clone();
    }
    if let Some(by) = els.obj("byEdge") {
        let mut keys: Vec<(i64, &J)> = by.iter().filter(|(_, v)| v.is_object()).map(|(k, v)| (k.parse::<i64>().unwrap_or(i64::MAX), v)).collect();
        keys.sort_by_key(|k| k.0);
        if let Some((_, v)) = keys.first() {
            return (*v).clone();
        }
    }
    els.at("default").filter(|v| v.is_object()).cloned().unwrap_or(json!({}))
}

/// The storey list's heights (a spec lists them, or gives a count).
fn storey_heights(spec: &J, height: f64) -> Vec<f64> {
    match spec.at("storeys") {
        Some(J::Array(a)) => a.iter().filter_map(|s| s.f("height")).filter(|h| *h > 0.5).collect(),
        Some(s) => {
            let n = s.f("count").unwrap_or(1.0).max(1.0);
            vec![height / n; n as usize]
        }
        None => vec![height],
    }
}

/// The resolved record and its classification, as `KitCity` loads them (no tenancy).
pub fn classify_source(src: &SourceRecord) -> Option<(LoadedRecord, Classified)> {
    let rec = pipeline::loaded_record(src, vec![]);
    let c = pipeline::classify_loaded(&rec)?;
    Some((rec, c))
}

/// One record's far facade (None: no classification or no spec).
pub fn far_facade(src: &SourceRecord) -> Option<FarFacade> {
    let (rec, c) = classify_source(src)?;
    let opts = FrameOptions { allowance: Some(ARCHETYPE_FRAME_ALLOWANCE_M), ..Default::default() };
    let frame = footprint_frame(&rec.polygon, 0, &opts)?;
    let spec = archetypes::spec_for(c.archetype, &c, &frame);
    if spec.is_null() {
        return None;
    }
    Some(summarise(&rec, &c, &spec))
}

fn summarise(rec: &LoadedRecord, c: &Classified, spec: &J) -> FarFacade {
    let meta = spec.at("archetype");
    let el = front_elevation(spec);
    let ws = el.at("windowSpec");
    let window = el.s("window").unwrap_or("punched").to_string();

    // Storeys.
    let heights = storey_heights(spec, c.height);
    let ground = heights.first().copied().unwrap_or(3.2);
    let mut storey = median(heights.iter().skip(1).copied().collect()).unwrap_or(ground);
    let tower = c.archetype == classify::DOWNTOWN_TOWER;
    if tower {
        storey = meta.f("floorHeight").filter(|h| *h > 2.0).unwrap_or(storey);
    }
    if storey > 6.0 {
        storey /= (storey / TOWER_FLOOR_M).round().max(1.0);
    }
    let parapet = spec.at("parapet").f("height").unwrap_or(0.0).max(0.0);
    let coping = spec.at("coping").f("height").unwrap_or(0.0).max(0.0);
    let top = (parapet + coping).min(6.0);

    // Window rhythm.
    let bay_widths: Vec<f64> = el.arr("bays").map(|a| a.iter().filter_map(|b| b.f("width")).filter(|w| *w > 0.6).collect()).unwrap_or_default();
    let mut bay = el.f("bayWidth").or_else(|| median(bay_widths.clone())).unwrap_or(3.0);
    let opening: Vec<f64> = el
        .arr("bays")
        .map(|a| {
            a.iter()
                .filter_map(|b| b.f("openingWidth").or_else(|| b.at("storeys").and_then(|s| s.as_object()).and_then(|m| m.values().find_map(|v| v.f("openingWidth")))))
                .collect()
        })
        .unwrap_or_default();
    let mut ww = ws.f("widthRatio").or_else(|| median(opening).map(|o| o / bay.max(0.5))).unwrap_or(0.5);
    let sill = ws.f("sillHeight").unwrap_or(0.85);
    let head = ws.f("head").or_else(|| ws.f("headClear").map(|h| storey - h)).unwrap_or(storey - 0.5);
    let mut wh = ((head - sill) / storey).clamp(0.2, 0.9);
    let mut sill_share = (sill / storey).clamp(0.0, 0.6);
    let mut lights = (ws.f("mullions").unwrap_or(0.0).round() as i64 + 1).clamp(1, 4) as u8;

    let mats = spec.at("materials");
    let family = meta.s("family").unwrap_or("");
    let mut class = match window.as_str() {
        "ribbon" => FacadeClass::Ribbon,
        "blind" => FacadeClass::Blind,
        _ => FacadeClass::Punched,
    };
    let mut glass = GlassKind::Office;
    match c.archetype {
        classify::DOWNTOWN_TOWER => {
            let spacing = meta.f("pierSpacing").unwrap_or(4.4);
            let pier = meta.f("pierWidth").unwrap_or(0.6);
            let band = meta.f("spandrelBand").unwrap_or(1.3);
            wh = ((storey - band) / storey).clamp(0.3, 0.85);
            sill_share = (0.5 * band / storey).clamp(0.0, 0.4);
            match family {
                "curtain-wall" => {
                    class = FacadeClass::Curtain;
                    glass = GlassKind::Tinted;
                    bay = CURTAIN_PANE_M;
                    ww = (1.0 - pier / spacing * 0.5).clamp(0.85, 0.97);
                    lights = 1;
                }
                "prewar-masonry" => {
                    class = FacadeClass::MasonryGrid;
                    bay = spacing;
                    ww = ((spacing - pier) / spacing).clamp(0.3, 0.8);
                    lights = 2;
                }
                _ => {
                    // the punched grid: precast piers and spandrels, the family's clear glass.
                    class = FacadeClass::Ribbon;
                    bay = spacing;
                    ww = ((spacing - pier) / spacing).clamp(0.4, 0.9);
                    lights = 3;
                }
            }
        }
        classify::SOMA_WAREHOUSE => {
            class = FacadeClass::Industrial;
            lights = lights.max(3);
        }
        classify::INDUSTRIAL_SHED => {
            class = FacadeClass::Shed;
            wh = (2.0 / storey).clamp(0.05, 0.5);
        }
        classify::VICTORIAN_ROW | classify::EDWARDIAN_FLATS => {
            class = if class == FacadeClass::Blind { FacadeClass::Blind } else { FacadeClass::RowBays };
            glass = GlassKind::Domestic;
        }
        classify::BAY_WINDOW_APARTMENT => {
            if class == FacadeClass::Punched && meta.f("projection").unwrap_or(0.0) > 0.3 {
                class = FacadeClass::RowBays;
            }
            if let Some(n) = el.at("bays").f("count").filter(|n| *n >= 1.0) {
                let long = c.bbox_long_side_m.max(c.frontage);
                if long > 1.0 {
                    bay = (long / n).clamp(1.8, 6.0);
                }
            }
            glass = GlassKind::Domestic;
        }
        classify::STUCCO_ROW | classify::MIDCENTURY_STUCCO | classify::COTTAGE_GARAGE => glass = GlassKind::Domestic,
        _ => {}
    }
    if class == FacadeClass::Ribbon && !tower {
        if let Some(sp) = el.at("ribbonSpec").f("spandrelHeight") {
            wh = ((storey - sp) / storey).clamp(0.25, 0.85);
            sill_share = (0.5 * sp / storey).clamp(0.0, 0.45);
        }
        if let Some(m) = el.at("ribbonSpec").f("mullionSpacing").filter(|m| *m < 20.0) {
            bay = m;
        }
        ww = 0.9;
    }
    // a cottage's front lists its windows per bay (the elevation itself is
    // blind); a garage keeps its blind walls.
    if c.archetype == classify::COTTAGE_GARAGE {
        let garage = meta.s("accessoryKind") == Some("garage") || meta.s("form") == Some("garage");
        class = if garage { FacadeClass::Blind } else { FacadeClass::Punched };
    }
    let shop_ground = el.s("ground") == Some("storefront")
        || el.arr("storeys").and_then(|s| s.first()).and_then(|s| s.as_str()) == Some("storefront")
        || (tower && c.has_retail_ground);

    // Paint, as `drawn_materials` paints the rung.
    let mut materials: BTreeMap<String, String> = mats.obj_entries();
    let own = own_tints(c, spec);
    let paint = pipeline::archetype_tints(own.as_ref(), &materials, c.seed);
    let mut tints = if c.civic.is_some() { pipeline::painted_timber_as_clapboard(&mut materials, paint) } else { paint }.unwrap_or_default();
    if crate::wall_paint::enabled() {
        crate::wall_paint::apply(c, &materials, matches!(own, Some(J::Object(_))), &mut tints);
    }
    let wall_name = materials.get("wall").cloned().unwrap_or_else(|| "brick-running-bond".into());
    let wall = drawn_colour(&wall_name, &tints).unwrap_or([0.5; 3]);
    let frame_name = materials.get("frame").cloned().unwrap_or_default();
    let trim = TRIM_FINISHES
        .iter()
        .position(|t| !t.is_empty() && (frame_name == *t || (*t == "cast-stone" && frame_name == "cast-stone-trim")))
        .map(|i| i as u8)
        .unwrap_or(TRIM_WALL_TONE);
    // The glass finish names the pane: a film or dark tint, a reflective or coated pane, else clear.
    match mats.s("glass") {
        Some("glass-film") | Some("glass-dark") if glass != GlassKind::Domestic => glass = GlassKind::Tinted,
        Some("glass-reflective") | Some("glass-sky") | Some("glass-storefront") => glass = GlassKind::Coated,
        _ => {}
    }

    // The roof: its finish, toward the aerial measurement (as `roof_measured::apply`).
    let pitched = meta.s("roof") == Some("gable") || truthy(meta.at("pitched")) || meta.at("roof").s("form") == Some("gable");
    let mut roof_name = mats.s("roof").unwrap_or("asphalt-roof").to_string();
    let mut roof_tint: Option<u32> = None;
    if pitched {
        if let Some(p) = mats.s("pitchedRoof") {
            roof_name = p.to_string();
        }
    } else if let Some(id) = pipeline::roof_finish_id_for(c.archetype, c.seed) {
        let (m, t) = pipeline::roof_finish(id);
        roof_name = m.to_string();
        roof_tint = t;
    }
    let mut roof = crate::roof_measured::material_colour(&roof_name).unwrap_or([0.1; 3]);
    if let Some(t) = roof_tint {
        roof = mul(roof, hex_linear(t));
    }
    if let Some(m) = rc::table().and_then(|t| t.get(&rec.key)) {
        let w = crate::roof_measured::weight(m.confidence);
        if w > 0.0 {
            let mut target = rc::rgb8_linear(m.rgb);
            if crate::roof_measured::feature_on("contrast") {
                target = crate::roof_measured::separated(target, Some(wall));
            }
            roof = [0, 1, 2].map(|k| roof[k] + (target[k] - roof[k]) * w);
        }
    }

    // A narrow row lot's side walls are on its lot lines.
    let lot_axis = ROW_ARCHETYPES.contains(&c.archetype).then(|| row_lot_axis(&rec.polygon)).flatten();

    FarFacade {
        archetype: c.archetype,
        class,
        glass,
        wall,
        roof,
        trim,
        ground_m: ground.clamp(2.0, 8.3) as f32,
        storey_m: storey.clamp(2.0, 7.1) as f32,
        top_m: top as f32,
        bay_m: bay.clamp(0.8, 12.0) as f32,
        window_w: ww.clamp(0.05, 0.97) as f32,
        window_h: wh as f32,
        sill: sill_share as f32,
        lights,
        shop_ground,
        lot_axis,
    }
}

trait ObjEntries {
    fn obj_entries(&self) -> BTreeMap<String, String>;
}

impl ObjEntries for Option<&J> {
    fn obj_entries(&self) -> BTreeMap<String, String> {
        self.and_then(|v| v.as_object()).map(|m| m.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect()).unwrap_or_default()
    }
}

/// The long axis of a narrow lot (unit, x z), or None: the second moments of
/// the ring's edges (each edge weighted by its length) give the principal
/// direction; the lot's extent across it is its front.
pub fn row_lot_axis(polygon: &[crate::frame::P2]) -> Option<[f64; 2]> {
    let n = polygon.len();
    if n < 3 {
        return None;
    }
    let (mut sxx, mut sxz, mut szz) = (0.0, 0.0, 0.0);
    for i in 0..n {
        let (a, b) = (polygon[i], polygon[(i + 1) % n]);
        let (dx, dz) = (b.x - a.x, b.z - a.z);
        let l = dx.hypot(dz);
        if l < 1e-6 {
            continue;
        }
        sxx += dx * dx / l;
        sxz += dx * dz / l;
        szz += dz * dz / l;
    }
    let theta = 0.5 * (2.0 * sxz).atan2(sxx - szz);
    let axis = [theta.cos(), theta.sin()];
    let (mut a0, mut a1, mut c0, mut c1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for p in polygon {
        let a = p.x * axis[0] + p.z * axis[1];
        let c = -p.x * axis[1] + p.z * axis[0];
        a0 = a0.min(a);
        a1 = a1.max(a);
        c0 = c0.min(c);
        c1 = c1.max(c);
    }
    let (along, across) = (a1 - a0, c1 - c0);
    (across <= ROW_LOT_MAX_M && along >= ROW_ASPECT * across).then_some(axis)
}

/// Window bays per edge of `ring` (open, x z): the wall's length over the
/// facade's bay, whole; 0 for a blind wall (a lot-line side of a row lot, a
/// wall under one bay or 2 m, every wall of a blind facade).
pub fn edge_bays(ring: &[[f32; 2]], f: &FarFacade) -> Vec<u32> {
    let n = ring.len();
    let bay = f.bay_m.max(0.5) as f64;
    (0..n)
        .map(|i| {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            let (dx, dz) = ((b[0] - a[0]) as f64, (b[1] - a[1]) as f64);
            let len = dx.hypot(dz);
            if f.class == FacadeClass::Blind || len < 2.0 || len < 0.6 * bay {
                return 0;
            }
            if let Some(ax) = f.lot_axis {
                // Within 30 degrees of the lot's long axis: a side wall on the lot line.
                if ((dx * ax[0] + dz * ax[1]) / len).abs() > 0.866 {
                    return 0;
                }
            }
            (len / bay).round().max(1.0) as u32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(w: f64, d: f64) -> Vec<f64> {
        vec![0.0, 0.0, w, 0.0, w, d, 0.0, d, 0.0, 0.0]
    }

    fn src<'a>(key: &'a str, points: &'a [f64], building: &'a str, levels: Option<f64>, height: Option<f64>) -> SourceRecord<'a> {
        SourceRecord { key, points, building, amenity: "", name: "", levels, height_raw: height, area_raw: None, height: height.unwrap_or(10.0) }
    }

    #[test]
    fn a_tower_gets_floors_of_its_family_and_glazing() {
        let pts = rect(40.0, 30.0);
        let mut seen = std::collections::BTreeSet::new();
        for k in 0..40 {
            let key = format!("9{k:04}");
            let Some(f) = far_facade(&src(&key, &pts, "office", Some(30.0), Some(110.0))) else { continue };
            if f.archetype != classify::DOWNTOWN_TOWER {
                continue;
            }
            assert!((3.2..=4.2).contains(&f.storey_m), "floor {}", f.storey_m);
            assert!(matches!(f.class, FacadeClass::Curtain | FacadeClass::Ribbon | FacadeClass::MasonryGrid), "{:?}", f.class);
            assert!(f.window_h > 0.3 && f.window_h < 0.9);
            seen.insert(f.class as u8);
        }
        assert!(!seen.is_empty(), "no tower among the samples");
    }

    #[test]
    fn the_same_record_gives_the_same_facade() {
        let pts = rect(7.6, 30.0);
        let a = far_facade(&src("12345", &pts, "house", Some(3.0), Some(10.0)));
        let b = far_facade(&src("12345", &pts, "house", Some(3.0), Some(10.0)));
        assert_eq!(a, b);
        assert!(a.is_some());
    }

    #[test]
    fn a_row_lot_has_blind_sides_and_glazed_ends() {
        let pts = rect(7.6, 30.0);
        let poly = pipeline::flat_to_points(&pts);
        let ax = row_lot_axis(&poly).expect("a 25 ft lot is a row lot");
        assert!(ax[1].abs() > 0.99, "{ax:?}");
        let f = FarFacade {
            archetype: classify::VICTORIAN_ROW,
            class: FacadeClass::RowBays,
            glass: GlassKind::Domestic,
            wall: [0.5; 3],
            roof: [0.2; 3],
            trim: 0,
            ground_m: 3.2,
            storey_m: 3.2,
            top_m: 0.9,
            bay_m: 3.3,
            window_w: 0.5,
            window_h: 0.55,
            sill: 0.25,
            lights: 2,
            shop_ground: false,
            lot_axis: Some(ax),
        };
        let ring: Vec<[f32; 2]> = poly.iter().take(4).map(|p| [p.x as f32, p.z as f32]).collect();
        assert_eq!(edge_bays(&ring, &f), vec![2, 0, 2, 0]);
        // A square block is no row lot.
        assert!(row_lot_axis(&pipeline::flat_to_points(&rect(30.0, 32.0))).is_none());
    }

    #[test]
    fn residential_walls_are_painted_light_on_average() {
        // The kit's paint on the primer: SF's painted fronts read light (the
        // mean over many lots well above mid grey), not the far ring's dark palette.
        let pts = rect(7.6, 25.0);
        let (mut sum, mut n) = (0.0f64, 0);
        for k in 0..300 {
            let key = format!("7{k:05}");
            let Some(f) = far_facade(&src(&key, &pts, "house", Some(3.0), Some(10.0))) else { continue };
            sum += (0.2126 * f.wall[0] + 0.7152 * f.wall[1] + 0.0722 * f.wall[2]) as f64;
            n += 1;
        }
        assert!(n > 200);
        let mean = sum / n as f64;
        assert!(mean > 0.3 && mean < 0.85, "mean wall luminance {mean}");
    }
}
