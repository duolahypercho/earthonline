//! The shop ground floor (src/world/buildings/archetypes/shop-ground.js,
//! `sf-archetype-shop-ground-v2-identity`) and the shop identity it letters
//! (src/world/buildings/shop-identity.js).
//!
//! A mechanical port: same streams (`|shop-ground-spec`, `|shop-ground`), same
//! roll order, same parts. Not ported, and why:
//! - Everything driven by a footway grade (`gradeAlongEdge`, `groundLean`,
//!   `alongEdgeSamples`, `planEntryStoop`, `planSunkEntry`, `planSunkWay`,
//!   `planRaisedDrop`, `entryRiserCount`, `openSideStreetDoor`): the factory
//!   passes `grade: null` in this port, so no door is raised, stepped or sunk,
//!   no sill climbs, and no side-street door is opened (the JS returns null
//!   there without a draw). Those paths are the only readers of `caMetric`.
//! - Sign lettering (the fascia style is drawn from the stream, as in the JS, but
//!   only its index is kept) and `plan.containment` (a diagnostic).

use crate::classify::Classified;
use crate::elevation::{BayRecord, OpeningRecord, PartArgs, Plan, elevation_defaults, layout_bays, resolve_elevation_raw, trim_profile, window_type_for};
use crate::frame::{Edge, FITTING_PROJECTION_ALLOWANCE, Frame};
use crate::js::*;
use crate::rng::{Rng, hash_seed, js_num};
use crate::street::OcclusionIndex;
use std::collections::BTreeMap;

pub const SHOP_GROUND_VERSION: &str = "sf-archetype-shop-ground-v2-identity";
pub const SHOP_GLASS_SLOT: &str = "shopGlass";
pub const SHOP_GLASS_MATERIAL: &str = "glass-shop";
/// (family, slot, material) in the JS object's order.
pub const SHOP_GLASS_FAMILIES: [(&str, &str, &str); 5] = [
    ("goods", SHOP_GLASS_SLOT, SHOP_GLASS_MATERIAL),
    ("food", "shopGlassFood", "glass-shop-food"),
    ("garments", "shopGlassGarments", "glass-shop-garments"),
    ("studio", "shopGlassStudio", "glass-shop-studio"),
    ("flowers", "shopGlassFlowers", "glass-shop-flowers"),
];
pub const SHOP_GLASS_SLOTS: [&str; 5] = [SHOP_GLASS_SLOT, "shopGlassFood", "shopGlassGarments", "shopGlassStudio", "shopGlassFlowers"];
pub const SHOP_DETAIL_KIND: &str = "shop-detail";
pub const SHOP_LAMP_KIND: &str = "sign-lamp";
pub const SHOP_GOOSENECK_SHARE: f64 = 0.55;
pub const SHOP_AWNING_CLEARANCE_M: f64 = 2.44;

/// `SHOP_GROUND_M`: joinery sizes, metres.
mod m {
    pub const FASCIA_ZONE: (f64, f64) = (0.46, 0.78);
    pub const TALL_STOREY: f64 = 3.1;
    pub const STALL_RISER: (f64, f64) = (0.48, 0.62);
    pub const TRANSOM: (f64, f64) = (0.34, 0.46);
    pub const TRANSOM_MIN_GLAZING: f64 = 2.05;
    pub const SETBACK: f64 = 0.18;
    pub const REVEAL: f64 = 0.24;
    pub const FRAME_WIDTH: f64 = 0.09;
    pub const FRAME_DEPTH: f64 = 0.12;
    pub const MULLION_SPACING: (f64, f64) = (1.15, 1.9);
    pub const PIER_PROJECTION: (f64, f64) = (0.09, 0.14);
    pub const FASCIA_PROJECTION: f64 = 0.075;
    pub const PIER_WIDTH: (f64, f64) = (0.3, 0.44);
    pub const SKIRT: f64 = 0.1;
    pub const KICK: f64 = 0.15;
    pub const DOOR_CLEAR_PROBES: [f64; 2] = [1.0, 2.5];
    pub const CORNICE_HEIGHT: (f64, f64) = (0.2, 0.3);
    pub const CORNICE_PROJECTION: (f64, f64) = (0.14, 0.22);
    pub const DOOR_RECESS: (f64, f64) = (0.56, 0.9);
    pub const TENANCY: (f64, f64) = (6.2, 9.4);
}

pub const BULKHEAD_ACCENTS: [&str; 6] = ["glazed-tile", "cast-stone", "terracotta", "painted-steel-mid", "wood-siding", "signal-red"];

pub const TRADE_WORDS: [&str; 48] = [
    "MARKET", "GROCERY", "DELI", "CAFE", "COFFEE", "BAKERY", "DINER", "RESTAURANT", "PIZZA", "NOODLES", "TACOS", "SUSHI", "BURGERS", "LAUNDRY", "CLEANERS", "TAILOR",
    "SHOES", "HARDWARE", "PAINT", "PRINTING", "COPIES", "PHARMACY", "DENTAL", "OPTICAL", "BOOKS", "RECORDS", "MUSIC", "BIKES", "FLOWERS", "FLORIST", "LIQUOR", "WINE",
    "BARBER", "SALON", "NAILS", "FURNITURE", "ANTIQUES", "LIGHTING", "RUGS", "TILE", "SUPPLY", "CAMERA", "PHOTO", "GYM", "YOGA", "STUDIO", "GALLERY", "CLINIC",
];

const SHOP_FAR_FINISH: [(&str, &str); 6] = [
    ("glazed-tile", "painted-white"),
    ("terracotta", "cast-stone-trim"),
    ("wood-siding", "painted-steel-mid"),
    ("signal-red", "painted-steel-mid"),
    ("canvas-plain", "awning-canvas"),
    ("painted-steel", "painted-steel-mid"),
];

/// `/^(tenancy\d+|shopPier|awningPlain|metal)$/`.
fn shop_own_slot(slot: &str) -> bool {
    if matches!(slot, "shopPier" | "awningPlain" | "metal") {
        return true;
    }
    slot.strip_prefix("tenancy").map(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit())).unwrap_or(false)
}

/// `shopFarMaterials(materials)`.
pub fn far_materials(m: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut out = m.clone();
    for (slot, v) in out.iter_mut() {
        if !shop_own_slot(slot) {
            continue;
        }
        if let Some((_, to)) = SHOP_FAR_FINISH.iter().find(|(from, _)| *from == v.as_str()) {
            *v = to.to_string();
        }
    }
    out
}

const PIER_FINISHES_PREWAR: [&str; 4] = ["cast-stone-trim", "cast-stone-trim", "terracotta", "painted-steel-mid"];
const PIER_FINISHES_POSTWAR: [&str; 4] = ["cast-stone-trim", "painted-steel-mid", "painted-white", "glazed-tile"];
/// Number of fascia styles per era (the styles themselves are sign texture data).
const FASCIA_STYLE_COUNT: f64 = 5.0;

fn lerp(range: (f64, f64), t: f64) -> f64 {
    range.0 + (range.1 - range.0) * t
}

fn pick<T: Copy>(pool: &[T], random: &mut Rng) -> T {
    let n = pool.len();
    pool[(n - 1).min((random.next() * n as f64).floor() as usize)]
}

/// `eraOf(classified)`: true for prewar.
fn prewar(year_built: f64, storeys: f64) -> bool {
    let year = if year_built.is_nan() { 0.0 } else { year_built };
    if year > 0.0 && year < 1946.0 {
        true
    } else if year >= 1946.0 {
        false
    } else {
        !(storeys <= 2.0)
    }
}

/// `elevationFor(spec, edge)`: the entry as an object, or None.
fn elevation_for(spec: &J, edge: &Edge) -> Option<J> {
    let e = resolve_elevation_raw(spec, edge);
    e.is_object().then_some(e)
}

/// `{ ...(v || {}), 0: value }` for a bay's `storeys` (an array spreads by index).
fn storeys_with_ground(v: Option<&J>, value: &str) -> J {
    let mut out = Map::new();
    match v {
        Some(J::Object(o)) => out = o.clone(),
        Some(J::Array(a)) => {
            for (i, x) in a.iter().enumerate() {
                out.insert(i.to_string(), x.clone());
            }
        }
        _ => {}
    }
    out.insert("0".into(), J::String(value.into()));
    J::Object(out)
}

/// `settleDoor` with no footway grade: the entrance without stoop/sink/raise/drop.
fn settle_door(entrance: Option<&J>) -> J {
    let mut plain = entrance.and_then(|e| e.as_object()).cloned().unwrap_or_default();
    for k in ["stoop", "sink", "raise", "drop"] {
        plain.shift_remove(k);
    }
    J::Object(plain)
}

/// Per-edge state kept for a last-resort door.
struct LastResort {
    edge_index: usize,
    blocked: Vec<bool>,
    types: Vec<String>,
    door_y: f64,
    storey_height: f64,
    head_clear: f64,
}

/// `open(index, lastResort)` on the spec's own bay entry.
fn open_door(spec_bays: &mut [J], types: &[String], blocked: &[bool], index: usize, door_y: f64, storey_height: f64, head_clear: f64, random: &mut Rng) -> bool {
    let wants = &mut spec_bays[index];
    let width = wants.num("width");
    if types[index] != "storefront" || width < 1.4 || !(door_y >= 2.0) {
        return false;
    }
    if blocked[index] {
        return false;
    }
    let storeys = storeys_with_ground(wants.at("storeys"), "entrance");
    set(wants, "storeys", storeys);
    let w = (1.1 + random.next() * 0.8).min(width - 0.4);
    let recess = lerp(m::DOOR_RECESS, random.next());
    let transom = 0f64.max(0.42f64.min(storey_height - head_clear - door_y));
    let leaf = if random.next() < 0.34 { "double-glass" } else { "single-glass" };
    let entrance = json!({ "width": w, "height": door_y, "recess": recess, "transom": transom, "leaf": leaf, "frameWidth": 0.08 });
    set(wants, "entrance", settle_door(Some(&entrance)));
    true
}

/// `prepareShopGroundSpec(spec, frame, classified, { grade: null, occlusion, selfId })`.
/// Returns the edges it took on. The JS keeps `classified` on `spec.shopGround`;
/// the port keeps there what `add` reads of it, as JSON.
pub fn prepare_spec(spec: &mut J, frame: &Frame, c: &Classified, occ: Option<&OcclusionIndex>, self_id: u32) {
    prepare_spec_edges(spec, frame, c, occ, self_id);
}

/// `prepareShopGroundSpec`, returning the edges it took on.
pub fn prepare_spec_edges(spec: &mut J, frame: &Frame, c: &Classified, occ: Option<&OcclusionIndex>, self_id: u32) -> Vec<usize> {
    if !c.has_retail_ground || !spec.truthy("elevations") || !spec.at("storeys").map(|v| v.is_array()).unwrap_or(false) {
        return vec![];
    }
    let storeys_len = spec.arr("storeys").map(|a| a.len()).unwrap_or(0);
    let ground = spec.arr("storeys").and_then(|a| a.first()).cloned();
    let storey_height = match &ground {
        Some(J::Number(n)) => n.as_f64().unwrap_or(0.0),
        other => num_or(other.as_ref().at("height"), 0.0),
    };
    if !(storey_height >= 2.3) {
        return vec![];
    }
    if spec.get_nn("archetype").truthy("arcade") {
        return vec![];
    }
    let defaults = elevation_defaults();
    let mut random = Rng::new(&format!("{}|shop-ground-spec", c.seed));
    let mut taken: Vec<usize> = vec![];
    let mut last_resorts: Vec<LastResort> = vec![];
    let mut clear_doors = 0usize;
    let meta = spec.get_nn("archetype").cloned().unwrap_or(json!({}));
    let retail_strip = meta.s("kind") == Some("retail-strip");

    // shopFrontages(spec, frame), taken before any edit.
    let fronts: Vec<(&Edge, J)> = frame
        .edges
        .iter()
        .filter(|e| !e.party && (e.role == "front" || e.role == "front-secondary"))
        .filter_map(|e| elevation_for(spec, e).map(|el| (e, el)))
        .collect();
    for (edge, elev) in fronts {
        let bays = layout_bays(edge.length, &elev, &defaults).bays;
        if bays.is_empty() || !bays.iter().all(|b| b.width >= 0.6) {
            continue;
        }
        let types: Vec<String> = bays.iter().map(|b| window_type_for(&elev, b, 0).filter(|t| !t.is_empty()).unwrap_or_else(|| "blind".into())).collect();
        if !types.iter().any(|t| t == "storefront") {
            continue;
        }
        let own_head = if retail_strip {
            None
        } else {
            Some(if storey_height >= m::TALL_STOREY { lerp(m::FASCIA_ZONE, 0.5 + random.next() * 0.5) } else { m::FASCIA_ZONE.0 })
        };
        let stall_riser = lerp(m::STALL_RISER, random.next());
        let mullion_spacing = lerp(m::MULLION_SPACING, random.next());
        let previous = elev.get_nn("storefrontSpec").filter(|v| truthy(Some(v))).cloned().unwrap_or(json!({}));
        let hc = num_or(previous.at("headClear"), 0.0).max(own_head.unwrap_or(0.0));
        let head_clear = if hc == 0.0 || hc.is_nan() { defaults["storefront"].num("headClear") } else { hc };
        let glazing = storey_height - head_clear - stall_riser;
        let mut sf = previous.as_object().cloned().unwrap_or_default();
        sf.insert("stallRiser".into(), jn(stall_riser));
        sf.insert("headClear".into(), jn(head_clear));
        let ms = if truthy(previous.at("mullionSpacing")) { previous.num("mullionSpacing").min(mullion_spacing) } else { mullion_spacing };
        sf.insert("mullionSpacing".into(), jn(ms));
        let transom = if glazing >= m::TRANSOM_MIN_GLAZING { lerp(m::TRANSOM, random.next()) } else { 0.0 };
        sf.insert("transom".into(), jn(transom));
        sf.insert("frameWidth".into(), jn(num_or(previous.at("frameWidth"), 0.0).max(m::FRAME_WIDTH)));
        sf.insert("frameDepth".into(), jn(num_or(previous.at("frameDepth"), 0.0).max(m::FRAME_DEPTH)));
        sf.insert("setback".into(), jn(m::SETBACK));
        sf.insert("capProjection".into(), jn(0.07));
        sf.insert("mullionCap".into(), J::Bool(true));

        let mut next = elev.as_object().cloned().unwrap_or_default();
        next.insert("storefrontSpec".into(), J::Object(sf));
        next.insert("reveal".into(), jn(num_or(elev.at("reveal"), 0.0).max(m::REVEAL)));
        next.shift_remove("signband");
        if storeys_len == 1 {
            next.insert("cornice".into(), J::Bool(false));
        }
        let mut specs: Vec<J> = bays
            .iter()
            .map(|b| {
                let mut o = b.spec.as_object().cloned().unwrap_or_default();
                o.insert("width".into(), jn(b.width));
                J::Object(o)
            })
            .collect();
        let door_y = 2.45f64.min(storey_height - head_clear - 0.1);
        // No footway grade: no sill climbs, every door meets its footway flush.
        let blocked: Vec<bool> = bays
            .iter()
            .map(|b| {
                let Some(occ) = occ else { return false };
                let mid = (b.s0 + b.s1) / 2.0;
                m::DOOR_CLEAR_PROBES.iter().any(|d| occ.occupied(edge.start.x + edge.u.x * mid + edge.n.x * d, edge.start.z + edge.u.z * mid + edge.n.z * d, self_id))
            })
            .collect();
        let door_clear = |n: usize| !blocked[n];
        // (`porch(i)` only changes how a door settles on a graded footway.)
        let mut run = 0.0;
        let mut tenancy_width = lerp(m::TENANCY, random.next());
        let has_door = retail_strip || types.iter().enumerate().any(|(i, t)| t == "entrance" && i > 0 && i < types.len() - 1);
        let mut can_open = false;
        if !has_door && !bays.is_empty() {
            can_open = true;
            let mut start = 0usize;
            for i in 0..bays.len() {
                run += bays[i].width;
                let last = i == bays.len() - 1;
                if run >= tenancy_width || last {
                    let at = ((start + i) / 2) as i64;
                    // Every entry is flush (no grade): nearest the middle, the later first.
                    let mut order: Vec<usize> = (start..=i).collect();
                    order.sort_by(|&a, &b| {
                        let d = (a as i64 - at).abs() - (b as i64 - at).abs();
                        let d = if d != 0 { d } else { b as i64 - a as i64 };
                        d.cmp(&0)
                    });
                    for n in order {
                        if open_door(&mut specs, &types, &blocked, n, door_y, storey_height, head_clear, &mut random) {
                            if door_clear(n) {
                                clear_doors += 1;
                            }
                            break;
                        }
                    }
                    start = i + 1;
                    run = 0.0;
                    tenancy_width = lerp(m::TENANCY, random.next());
                }
            }
        }
        // A door the archetype planned itself: one facing a neighbour moves to the nearest clear shop bay.
        let as_window = |wants: &mut J| -> Option<J> {
            let storeys = storeys_with_ground(wants.at("storeys"), "storefront");
            set(wants, "storeys", storeys);
            wants.as_object_mut().and_then(|o| o.shift_remove("entrance"))
        };
        for i in 0..specs.len() {
            if types[i] != "entrance" {
                continue;
            }
            let mut at = i;
            if !door_clear(i) {
                let free = |specs: &[J], n: usize| {
                    types[n] == "storefront"
                        && crate::elevation::index_any(specs[n].get_nn("storeys"), 0).and_then(|v| v.as_str()) != Some("entrance")
                        && specs[n].num("width") >= 1.4
                        && door_clear(n)
                };
                let mut cands: Vec<usize> = (0..bays.len()).filter(|&n| free(&specs, n)).collect();
                cands.sort_by_key(|&n| (n as i64 - i as i64).abs());
                let Some(&home) = cands.first() else {
                    // Blocked with no clear bay: it stays a window.
                    as_window(&mut specs[i]);
                    continue;
                };
                let moved = as_window(&mut specs[i]);
                let storeys = storeys_with_ground(specs[home].at("storeys"), "entrance");
                set(&mut specs[home], "storeys", storeys);
                if let Some(mv) = moved.filter(|v| truthy(Some(v))) {
                    set(&mut specs[home], "entrance", mv);
                }
                at = home;
            }
            clear_doors += 1;
            let settled = settle_door(specs[at].at("entrance").filter(|v| truthy(Some(v))));
            set(&mut specs[at], "entrance", settled);
        }
        if can_open {
            last_resorts.push(LastResort { edge_index: edge.index, blocked: blocked.clone(), types: types.clone(), door_y, storey_height, head_clear });
        }
        next.insert("bays".into(), J::Array(specs));
        if next.get("align").map(|v| v.is_null()).unwrap_or(true) {
            next.insert("align".into(), J::String("center".into()));
        }
        let elevations = obj_mut(spec, "elevations");
        let by_edge = obj_mut(elevations, "byEdge");
        set(by_edge, &edge.index.to_string(), J::Object(next));
        taken.push(edge.index);
    }
    // No grade: openSideStreetDoor finds no face (it returns null without a draw),
    // and no door is buried. A last resort opens a door where every one was blocked.
    if clear_doors == 0 {
        for lr in &last_resorts {
            let bays_j = spec.pointer_mut(&format!("/elevations/byEdge/{}/bays", lr.edge_index)).and_then(|v| v.as_array_mut());
            let Some(bays_j) = bays_j else { continue };
            let mut done = false;
            for n in 0..bays_j.len() {
                if open_door(bays_j, &lr.types, &lr.blocked, n, lr.door_y, lr.storey_height, lr.head_clear, &mut random) {
                    done = true;
                    break;
                }
            }
            if done {
                break;
            }
        }
    }
    if !taken.is_empty() {
        let mut mats = Map::new();
        mats.insert("awning".into(), json!("awning-canvas"));
        mats.insert("awningPlain".into(), json!("canvas-plain"));
        mats.insert("canopy".into(), json!("painted-steel-mid"));
        mats.insert("metal".into(), json!("painted-steel"));
        if let Some(o) = spec.obj("materials") {
            for (k, v) in o {
                mats.insert(k.clone(), v.clone());
            }
        }
        for (_, slot, material) in SHOP_GLASS_FAMILIES {
            mats.insert(slot.into(), json!(material));
        }
        for (i, a) in BULKHEAD_ACCENTS.iter().enumerate() {
            let k = format!("tenancy{i}");
            if !truthy(mats.get(&k)) {
                mats.insert(k, json!(a));
            }
        }
        set(spec, "materials", J::Object(mats));
        let units: Vec<J> = c
            .shop_units
            .iter()
            .map(|u| json!({ "edge": u.edge, "alongM": u.along_m, "widthM": u.width_m, "x": jn(u.x), "z": jn(u.z), "category": u.category, "subtype": u.subtype }))
            .collect();
        let year = if c.year_built.is_nan() { 0.0 } else { c.year_built };
        set(
            spec,
            "shopGround",
            json!({
                "version": SHOP_GROUND_VERSION,
                "edges": taken,
                "sideDoor": null,
                "grade": null,
                "classified": {
                    "seed": c.seed,
                    "signTexts": c.sign_texts,
                    "signTrade": c.sign_trade.clone().filter(|s| !s.is_empty()),
                    "recordId": if c.record_id.is_empty() { J::Null } else { J::String(c.record_id.clone()) },
                    "tradeTag": c.trade_tag,
                    "recordName": c.record_name,
                    "shopUnits": units,
                    "yearBuilt": jn(year),
                    "storeys": jn(if c.storeys.is_nan() { 0.0 } else { c.storeys }),
                },
            }),
        );
    }
    taken
}

// ---------------------------------------------------------------------------
// Shop identity (shop-identity.js).
// ---------------------------------------------------------------------------

const SHOPFRONT_SHOP_TYPES: [&str; 7] = ["books", "cleaners", "cafe", "studio", "grocery", "bar", "bakery"];

fn sign_names(list: &str) -> &'static [&'static str; 6] {
    match list {
        "cleaners" => &["BRIGHTFOLD", "PRESS & FOLD", "CLEAN LINE", "LAUNDRY CO", "STEAM WORKS", "FINE PRESS"],
        "cafe" => &["BEAN & BAR", "MORNING CUP", "THE KETTLE", "CORNER CAFE", "ROAST HOUSE", "LITTLE CUP"],
        "studio" => &["FRAME & INK", "STUDIO NINE", "GRID WORKS", "THE ATELIER", "LINE STUDIO", "PAPER & LENS"],
        "grocery" => &["DAILY GREENS", "MARKET BOX", "THE PANTRY", "FRESH CORNER", "ORCHARD ROW", "GREEN GROCER"],
        "bar" => &["THE STILL", "BARREL ROOM", "COPPER TAP", "LATE HOURS", "THE SNUG", "AMBER & OAK"],
        "bakery" => &["CRUST & CRUMB", "MORNING LOAF", "THE OVEN", "HEARTH BAKERY", "FLOUR & FIRE", "RISE BAKERY"],
        "florist" => &["PETAL & STEM", "WILD BLOOM", "FERN & FIG", "THE STEM", "BLOOM ROOM", "MEADOW HOUSE"],
        _ => &["FOLIO & QUILL", "NORTHLIGHT", "HARBOUR BOOKS", "LANTERN PRESS", "PAGE TURNERS", "INK & SPINE"],
    }
}

/// `hashKey(text)`: FNV-1a over UTF-16 code units.
pub fn hash_key(text: &str) -> u32 {
    let mut h: u32 = 2166136261;
    for c in text.encode_utf16() {
        h ^= c as u32;
        h = h.wrapping_mul(16777619);
    }
    h
}

fn resolve_shop_type(seed_key: &str) -> &'static str {
    SHOPFRONT_SHOP_TYPES[(hash_key(&format!("{seed_key}:shop-type")) as usize) % SHOPFRONT_SHOP_TYPES.len()]
}

const TRADE_ARCHETYPE_GROUPS: [(&str, &[&str]); 7] = [
    ("cafe", &["cafe", "restaurant", "fast_food", "food_court", "ice_cream", "coffee", "juice_bar", "deli"]),
    ("bar", &["bar", "pub", "nightclub", "biergarten", "stripclub", "swingerclub", "karaoke_box", "alcohol", "wine"]),
    ("bakery", &["bakery", "pastry", "confectionery", "chocolate"]),
    (
        "grocery",
        &[
            "supermarket", "convenience", "greengrocer", "butcher", "seafood", "farm", "beverages", "marketplace", "kiosk", "general", "wholesale", "department_store", "mall", "health_food",
            "nutrition_supplements", "herbalist", "cannabis",
        ],
    ),
    (
        "books",
        &["books", "library", "stationery", "newsagent", "music", "video", "toys", "gift", "antiques", "collector", "art", "gallery", "artwork", "museum", "copyshop", "ticket"],
    ),
    (
        "cleaners",
        &[
            "pharmacy", "chemist", "doctors", "dentist", "clinic", "hospital", "veterinary", "optician", "medical_supply", "laundry", "dry_cleaning", "hairdresser", "hairdresser_supply", "beauty",
            "massage", "nail_salon", "tattoo", "childcare", "kindergarten", "nursing_home", "animal_boarding", "pet",
        ],
    ),
    (
        "studio",
        &[
            "bank", "bureau_de_change", "financial", "insurance", "estate_agent", "property_management", "lawyer", "tax_advisor", "architect", "engineer", "landscape_architect",
            "advertising_agency", "travel_agency", "post_office", "post_depot", "government", "townhall", "courthouse", "police", "fire_station", "prison", "diplomatic", "company",
            "coworking", "coworking_space", "incubator", "research", "research_institute", "studio", "photo", "craft", "pottery", "theatre", "cinema", "concert_hall", "arts_centre",
            "events_venue", "music_rehearsal_place", "music_school", "language_school", "school", "college", "university", "prep_school", "training", "educational_institution",
            "community_centre", "social_centre", "social_facility", "ngo", "association", "union", "funeral_directors", "mortuary", "clothes", "shoes", "jewelry", "erotic", "florist",
            "garden_centre", "interior_decoration", "interior_design", "furniture", "houseware", "kitchen", "carpet", "paint", "hardware", "doityourself", "electrical", "electronics",
            "telecommunication", "bicycle", "motorcycle", "outdoor", "sports", "fishing", "car", "car_parts", "car_repair", "car_rental", "car_wash", "tyres", "boat_rental",
            "bicycle_rental", "rental", "storage_rental", "security", "bail_bond_agent", "trade", "construction_company", "hotel", "motel", "hostel", "guest_house", "conference_centre",
        ],
    ),
];

fn trade_archetype(tag: &str) -> Option<&'static str> {
    TRADE_ARCHETYPE_GROUPS.iter().find(|(_, tags)| tags.contains(&tag)).map(|(t, _)| *t)
}

/// `SHOPFRONT_NAME_TRADES`: alternatives per trade; `_` is an optional `\s`.
const NAME_TRADES: [(&str, &[&str]); 7] = [
    ("bakery", &["bakery", "bakeries", "baking", "patisserie", "boulangerie", "donut", "doughnut"]),
    (
        "cafe",
        &[
            "cafe", "caffe", "caffè", "coffee", "espresso", "tea_house", "creamery", "gelato", "juice", "diner", "restaurant", "kitchen", "taqueria", "noodle", "pizzeria", "pizza", "sushi",
            "bistro", "deli", "delicatessen",
        ],
    ),
    ("bar", &["bar", "pub", "tavern", "saloon", "lounge", "cocktail", "brewery", "brewing", "taproom", "wine", "liquor", "spirits"]),
    ("grocery", &["market", "grocery", "grocer", "produce", "foods", "provisions", "butcher", "fish_market", "deli_mart", "mini_mart", "corner_store"]),
    ("books", &["book", "books", "bookstore", "bookshop", "library", "records", "comics", "stationery", "newsstand", "gallery", "museum"]),
    (
        "cleaners",
        &[
            "pharmacy", "drug", "drugs", "drugstore", "clinic", "dental", "dentist", "medical", "optometry", "optical", "salon", "barber", "barbers", "nails", "spa", "laundry",
            "laundromat", "cleaners", "dry_clean",
        ],
    ),
    (
        "studio",
        &[
            "bank", "credit_union", "savings", "insurance", "realty", "real_estate", "law", "attorney", "attorneys", "studio", "atelier", "design", "architect", "architects", "hardware",
            "supply", "cleaners_co", "hotel", "inn", "theatre", "theater", "cinema",
        ],
    ),
];

fn is_word(c: Option<&char>) -> bool {
    c.map(|c| c.is_ascii_alphanumeric() || *c == '_').unwrap_or(false)
}

/// Does `alt` (lower case, `_` = optional whitespace) match at `i`? Returns the end positions.
fn match_alt(chars: &[char], i: usize, alt: &[char], out: &mut Vec<usize>) {
    if alt.is_empty() {
        out.push(i);
        return;
    }
    if alt[0] == '_' {
        match_alt(chars, i, &alt[1..], out);
        if i < chars.len() && chars[i].is_whitespace() {
            match_alt(chars, i + 1, &alt[1..], out);
        }
        return;
    }
    if i < chars.len() && chars[i].to_lowercase().eq(std::iter::once(alt[0])) {
        match_alt(chars, i + 1, &alt[1..], out);
    }
}

/// `/\b(alt|...)\b/i.test(name)`.
fn name_matches(name: &str, alts: &[&str]) -> bool {
    let chars: Vec<char> = name.chars().collect();
    let boundary = |p: usize| is_word(if p == 0 { None } else { chars.get(p - 1) }) != is_word(chars.get(p));
    for i in 0..=chars.len() {
        if !boundary(i) {
            continue;
        }
        for alt in alts {
            let a: Vec<char> = alt.chars().collect();
            let mut ends = vec![];
            match_alt(&chars, i, &a, &mut ends);
            if ends.iter().any(|&e| boundary(e)) {
                return true;
            }
        }
    }
    false
}

/// `resolveShopTrade({ amenity: tag, name }, seedKey)`: (shopType, source).
fn resolve_shop_trade(tag: &str, name: &str, seed_key: &str) -> (&'static str, &'static str) {
    if !tag.is_empty() {
        if let Some(t) = trade_archetype(tag) {
            return (t, "tag");
        }
    }
    let name = name.trim();
    if !name.is_empty() {
        for (t, alts) in NAME_TRADES {
            if name_matches(name, alts) {
                return (t, "name");
            }
        }
    }
    (resolve_shop_type(seed_key), "hash")
}

fn tag_sign_word(tag: &str) -> Option<&'static str> {
    const T: &[(&str, &str)] = &[
        ("cafe", "CAFE"), ("coffee", "COFFEE"), ("fast_food", "CAFE"), ("restaurant", "RESTAURANT"), ("food_court", "RESTAURANT"), ("ice_cream", "CAFE"), ("juice_bar", "CAFE"),
        ("deli", "DELI"), ("pizza", "PIZZA"), ("sushi", "SUSHI"), ("noodles", "NOODLES"), ("bar", "BAR"), ("pub", "TAVERN"), ("nightclub", "CLUB"), ("biergarten", "BAR"),
        ("alcohol", "LIQUOR"), ("wine", "WINE"), ("bakery", "BAKERY"), ("pastry", "BAKERY"), ("confectionery", "BAKERY"), ("supermarket", "GROCERY"), ("convenience", "MARKET"),
        ("greengrocer", "MARKET"), ("butcher", "MARKET"), ("general", "MARKET"), ("health_food", "GROCERY"), ("kiosk", "MARKET"), ("beverages", "MARKET"), ("books", "BOOKS"),
        ("stationery", "SUPPLY"), ("music", "RECORDS"), ("video", "VIDEO"), ("antiques", "ANTIQUES"), ("art", "GALLERY"), ("gallery", "GALLERY"), ("copyshop", "COPIES"),
        ("pharmacy", "PHARMACY"), ("chemist", "PHARMACY"), ("doctors", "CLINIC"), ("clinic", "CLINIC"), ("dentist", "DENTAL"), ("optician", "OPTICAL"), ("laundry", "LAUNDRY"),
        ("dry_cleaning", "CLEANERS"), ("hairdresser", "SALON"), ("beauty", "SALON"), ("nail_salon", "NAILS"), ("tattoo", "TATTOO"), ("barber", "BARBER"), ("clothes", "TAILOR"),
        ("tailor", "TAILOR"), ("shoes", "SHOES"), ("florist", "FLORIST"), ("garden_centre", "FLOWERS"), ("furniture", "FURNITURE"), ("interior_decoration", "FURNITURE"),
        ("carpet", "RUGS"), ("lighting", "LIGHTING"), ("paint", "PAINT"), ("hardware", "HARDWARE"), ("doityourself", "HARDWARE"), ("bicycle", "BIKES"), ("photo", "PHOTO"),
        ("camera", "CAMERA"), ("fitness_centre", "FITNESS"), ("gym", "GYM"), ("yoga", "YOGA"), ("studio", "STUDIO"), ("printing", "PRINTING"),
    ];
    if tag.is_empty() {
        return None;
    }
    T.iter().find(|(k, _)| *k == tag).map(|(_, v)| *v)
}

fn shop_type_word(t: &str) -> &'static str {
    match t {
        "books" => "BOOKS",
        "cleaners" => "CLEANERS",
        "cafe" => "CAFE",
        "studio" => "STUDIO",
        "grocery" => "GROCERY",
        "bar" => "BAR",
        _ => "BAKERY",
    }
}

fn sign_word_shop_type(w: &str) -> Option<&'static str> {
    Some(match w {
        "CAFE" | "COFFEE" | "RESTAURANT" | "DINER" | "DELI" | "PIZZA" | "SUSHI" | "NOODLES" | "TACOS" | "BURGERS" => "cafe",
        "BAR" | "TAVERN" | "LOUNGE" | "CLUB" | "LIQUOR" | "WINE" | "SPIRITS" => "bar",
        "BAKERY" => "bakery",
        "GROCERY" | "MARKET" => "grocery",
        "BOOKS" | "RECORDS" | "MUSIC" | "VIDEO" | "ANTIQUES" | "GALLERY" | "COPIES" | "SUPPLY" => "books",
        "CLEANERS" | "LAUNDRY" | "PHARMACY" | "CLINIC" | "DENTAL" | "OPTICAL" | "SALON" | "BARBER" | "NAILS" | "TATTOO" => "cleaners",
        "STUDIO" | "TAILOR" | "SHOES" | "FLORIST" | "FLOWERS" | "FURNITURE" | "RUGS" | "LIGHTING" | "PAINT" | "HARDWARE" | "BIKES" | "PHOTO" | "CAMERA" | "FITNESS" | "GYM" | "YOGA"
        | "PRINTING" => "studio",
        _ => return None,
    })
}

fn shop_room_family(shop_type: &str, word: &str) -> &'static str {
    let by_word = match word {
        "CLEANERS" | "LAUNDRY" | "TAILOR" | "SHOES" => Some("garments"),
        "SALON" | "BARBER" | "NAILS" | "TATTOO" | "CLINIC" | "DENTAL" | "GALLERY" | "STUDIO" | "PHOTO" | "CAMERA" | "FURNITURE" | "RUGS" | "LIGHTING" | "ANTIQUES" | "FITNESS"
        | "GYM" | "YOGA" => Some("studio"),
        "FLORIST" | "FLOWERS" => Some("flowers"),
        "PHARMACY" | "OPTICAL" | "HARDWARE" | "PAINT" | "BIKES" | "PRINTING" | "COPIES" => Some("goods"),
        _ => None,
    };
    by_word.unwrap_or(match shop_type {
        "books" | "grocery" => "goods",
        "cafe" | "bakery" | "bar" => "food",
        "cleaners" => "garments",
        "studio" => "studio",
        _ => "goods",
    })
}

fn name_list_for_word(w: &str) -> Option<&'static str> {
    Some(match w {
        "BOOKS" => "books",
        "CLEANERS" | "LAUNDRY" => "cleaners",
        "CAFE" | "COFFEE" => "cafe",
        "STUDIO" => "studio",
        "GROCERY" | "MARKET" => "grocery",
        "BAR" => "bar",
        "BAKERY" => "bakery",
        "FLORIST" | "FLOWERS" => "florist",
        _ => return None,
    })
}

fn name_says_trade(name: &str, word: &str) -> bool {
    let upper = name.to_uppercase();
    let tokens: Vec<&str> = upper.split(|c: char| !c.is_ascii_uppercase()).filter(|t| !t.is_empty()).collect();
    if tokens.contains(&word) {
        return true;
    }
    let kin: &[&str] = match word {
        "CLEANERS" => &["LAUNDRY", "FOLD"],
        "LAUNDRY" => &["CLEANERS"],
        "GROCERY" => &["GROCER", "MARKET"],
        "MARKET" => &["GROCER"],
        "CAFE" => &["CUP", "ROAST", "KETTLE", "BEAN"],
        "BAR" => &["TAP", "BARREL"],
        "BAKERY" => &["OVEN", "LOAF", "CRUST", "FLOUR"],
        _ => &[],
    };
    kin.iter().any(|k| tokens.contains(k))
}

fn shop_fascia_text(name: Option<&str>, word: &str) -> Option<String> {
    let Some(name) = name.filter(|n| !n.is_empty()) else {
        return (!word.is_empty()).then(|| word.to_string());
    };
    if word.is_empty() || name_says_trade(name, word) {
        return Some(name.to_string());
    }
    Some(format!("{name}|{word}"))
}

fn sign_trade_from_unit(key: &str) -> Option<&'static str> {
    Some(match key {
        "cafe" | "restaurant:limited-service" => "CAFE",
        "restaurant" => "RESTAURANT",
        "bar" => "BAR",
        "supermarket" => "GROCERY",
        "convenience" | "retail:shop" | "retail:misc-store" | "retail:general-merchandise" => "MARKET",
        "retail:specialty-food" | "retail:food-store" => "DELI",
        "retail:bakery" => "BAKERY",
        "retail:hardware-garden" => "HARDWARE",
        "retail:dry-cleaner" | "retail:laundromat" => "CLEANERS",
        "retail:salon-personal-care" | "retail:personal-service" => "BARBER",
        "retail:hobby-books-sport" => "BOOKS",
        "retail:fitness" => "STUDIO",
        "retail:apparel" => "TAILOR",
        _ => return None,
    })
}

/// `unitSignWord(unit)`.
fn unit_sign_word(unit: &J) -> Option<&'static str> {
    let cat = unit.s("category").unwrap_or("undefined");
    let sub = unit.s("subtype").unwrap_or("undefined");
    sign_trade_from_unit(&format!("{cat}:{sub}")).or_else(|| sign_trade_from_unit(cat))
}

#[derive(Clone, Debug)]
struct Identity {
    shop_type: &'static str,
    word: String,
    text: Option<String>,
    family: &'static str,
    source: &'static str,
}

/// `resolveShopIdentity({ seedKey, tag, recordName, word, parody })`.
fn resolve_shop_identity(seed_key: &str, tag: &str, record_name: &str, word: Option<&str>, parody: Option<&str>) -> Identity {
    let clean_tag = tag.trim().to_lowercase();
    let mut shop_type: Option<&'static str> = None;
    let mut fascia: Option<String> = None;
    let mut source = "hash";
    if !clean_tag.is_empty() || !record_name.is_empty() {
        let (t, s) = resolve_shop_trade(&clean_tag, record_name, seed_key);
        if s != "hash" {
            shop_type = Some(t);
            fascia = tag_sign_word(&clean_tag).map(|w| w.to_string());
            source = s;
        }
    }
    if shop_type.is_none() {
        if let Some(w) = word.filter(|w| !w.is_empty()) {
            shop_type = sign_word_shop_type(w);
            if shop_type.is_some() {
                fascia = Some(w.to_string());
                source = "unit";
            }
        }
    }
    let shop_type = shop_type.unwrap_or_else(|| resolve_shop_type(seed_key));
    let fascia = fascia.unwrap_or_else(|| match word.filter(|w| !w.is_empty() && sign_word_shop_type(w) == Some(shop_type)) {
        Some(w) => w.to_string(),
        None => shop_type_word(shop_type).to_string(),
    });
    let name: Option<String> = match parody.filter(|p| !p.is_empty()) {
        Some(p) => Some(p.to_string()),
        None => name_list_for_word(&fascia).map(|list| {
            let names = sign_names(list);
            names[(hash_key(&format!("{seed_key}:sign-name")) as usize) % names.len()].to_string()
        }),
    };
    let text = shop_fascia_text(name.as_deref(), &fascia);
    let family = shop_room_family(shop_type, &fascia);
    Identity { shop_type, word: fascia, text, family, source }
}

const SIGN_DENYLIST_DIGESTS: &[u32] = &[
    768418, 3498942, 15639518, 34484083, 62366780, 68159807, 97189229, 108775112, 110778522, 120287048, 147163597, 183895099, 192013946, 209044288, 212475776, 228712814, 232372193,
    248737696, 248903295, 252403531, 253006652, 268143074, 299662067, 344614518, 357606765, 422868715, 434560502, 439379688, 440652853, 500421696, 518153670, 549522431, 556121862,
    604996244, 612012049, 625763385, 656882518, 677453583, 679506516, 717802614, 720018959, 720645044, 726483662, 785865631, 787384196, 788269066, 794501977, 816349845, 829892241,
    848834650, 859980292, 906789362, 922495178, 942355954, 943361643, 957769600, 959357363, 966881310, 972071773, 985761528, 989068619, 990422336, 1006584145, 1021609616, 1060247414,
    1062138189, 1068407769, 1071833916, 1097887312, 1110289794, 1152736536, 1202073361, 1209286198, 1250510879, 1253322018, 1269806420, 1272761525, 1326634411, 1339048652, 1347650592,
    1354432531, 1377245229, 1382249916, 1386436845, 1386927198, 1395262093, 1430912854, 1455910212, 1477453414, 1480151085, 1500531605, 1510004807, 1521812279, 1546070497, 1607891328,
    1608886399, 1633416709, 1645715523, 1650746889, 1668940789, 1679378555, 1698110531, 1702040454, 1716545717, 1717663153, 1719620384, 1740642476, 1797863409, 1859556515, 1866041447,
    1903992814, 1975413752, 1998597252, 2000053149, 2035183131, 2046283717, 2081705117, 2087865587, 2090206839, 2107985256, 2131311227, 2135559081, 2150946214, 2179565191, 2184956371,
    2214926114, 2237361058, 2238017689, 2245142003, 2263208990, 2268324328, 2309052736, 2313353578, 2349317581, 2361200706, 2369064499, 2388542312, 2439174649, 2457734258, 2470412037,
    2492196944, 2495776680, 2510971569, 2511170941, 2520803653, 2524830885, 2532441587, 2568894802, 2580265200, 2656202268, 2679930945, 2682701165, 2775300852, 2788099943, 2795388055,
    2798124894, 2799447562, 2829265170, 2835120838, 2850087183, 2868526702, 2876755531, 2876911791, 2884372083, 2894583849, 2919567946, 2975368727, 2982748961, 2993696001, 3012714941,
    3017725047, 3019928626, 3028276742, 3049356131, 3074123037, 3082238232, 3155182174, 3156870618, 3191661535, 3193581898, 3196521666, 3205842463, 3225196479, 3233284392, 3258838922,
    3287276830, 3314142817, 3353180806, 3375796893, 3409238208, 3412428690, 3445015971, 3450974470, 3456659880, 3459693020, 3489318092, 3503775159, 3557301939, 3607990670, 3614550613,
    3615413050, 3624101773, 3693306370, 3710037220, 3711716140, 3715645667, 3772269493, 3798733233, 3799936374, 3808085306, 3823726112, 3825369471, 3827043864, 3834529175, 3835613527,
    3847900768, 3873215680, 3873392669, 3883036277, 3883440389, 3888625101, 3890603922, 3922175739, 3937206568, 3940608427, 3944112511, 3955438145, 3978427474, 3980898643, 4000545559,
    4003508827, 4019848226, 4031195738, 4093687938, 4100559537, 4131746517, 4155109292, 4192578183, 4195546319, 4199607396, 4201425791, 4204836446, 4247287712,
];

fn sign_text_denied(text: &str) -> bool {
    let norm: String = text.to_lowercase().chars().filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit()).collect();
    SIGN_DENYLIST_DIGESTS.contains(&hash_seed(&format!("sign|{norm}")))
}

/// `checkSignText(text).ok` (kit/signage.js), without a machine-local list.
fn check_sign_text(raw: &str) -> bool {
    if raw.trim().is_empty() || raw.contains(['™', '®', '©']) || raw.encode_utf16().count() > 64 || sign_text_denied(raw) {
        return false;
    }
    let tokens: Vec<&str> = raw.split(|c: char| c.is_whitespace() || "-_/.,&+".contains(c)).filter(|t| !t.is_empty()).collect();
    for i in 0..tokens.len() {
        for span in 1..=3 {
            if i + span <= tokens.len() && sign_text_denied(&tokens[i..i + span].join(" ")) {
                return false;
            }
        }
    }
    true
}

// ---------------------------------------------------------------------------
// addShopGround.
// ---------------------------------------------------------------------------

struct Tenancy {
    s0: f64,
    s1: f64,
    openings: Vec<OpeningRecord>,
    measured: Option<J>,
    pending: bool,
    identity: Option<Identity>,
}

struct Run<'a> {
    index: usize,
    edge: &'a Edge,
    tenancies: Vec<Tenancy>,
}

/// `tenanciesFromOpenings(openings, random)`.
fn tenancies_from_openings(openings: &[OpeningRecord], random: &mut Rng) -> Vec<Tenancy> {
    let mut glazed: Vec<OpeningRecord> = openings.iter().filter(|op| op.storey == 0 && (op.type_ == "storefront" || op.type_ == "entrance")).cloned().collect();
    glazed.sort_by(|a, b| a.s0.partial_cmp(&b.s0).unwrap_or(std::cmp::Ordering::Equal));
    let mut out: Vec<Tenancy> = vec![];
    let mut target = lerp(m::TENANCY, random.next());
    for op in glazed {
        let break_run = match out.last() {
            None => true,
            Some(cur) => {
                let gap = op.s0 - cur.s1;
                gap > 1.4 || ((cur.s1 - cur.s0 >= target || cur.openings.len() >= 5) && gap > 0.2)
            }
        };
        if break_run {
            out.push(Tenancy { s0: op.s0, s1: op.s1, openings: vec![], measured: None, pending: false, identity: None });
            target = lerp(m::TENANCY, random.next());
        }
        let cur = out.last_mut().unwrap();
        cur.s0 = cur.s0.min(op.s0);
        cur.s1 = cur.s1.max(op.s1);
        cur.openings.push(op);
    }
    for t in out.iter_mut() {
        let glass: Vec<&OpeningRecord> = t.openings.iter().filter(|op| op.type_ == "storefront").collect();
        if glass.is_empty() {
            continue;
        }
        t.s0 = glass.iter().map(|o| o.s0).fold(f64::INFINITY, f64::min);
        t.s1 = glass.iter().map(|o| o.s1).fold(f64::NEG_INFINITY, f64::max);
    }
    out.retain(|t| t.openings.iter().any(|op| op.type_ == "storefront"));
    out
}

/// `assignShopIdentities(runs, classified, fallbackId)`.
fn assign_shop_identities(runs: &mut [Run], cl: &J, fallback_id: &str) {
    let id = cl.s("recordId").filter(|s| !s.is_empty()).map(|s| s.to_string()).unwrap_or_else(|| fallback_id.to_string());
    let mut names: Vec<String> = cl.arr("signTexts").map(|a| a.iter().map(|v| v.as_str().map(|s| s.to_string()).unwrap_or_else(|| v.to_string())).collect()).unwrap_or_default();
    names.retain(|n| check_sign_text(n));
    let mut names = names.into_iter();
    let units = cl.arr("shopUnits").cloned().unwrap_or_default();
    if runs.iter().all(|r| r.tenancies.is_empty()) {
        return;
    }
    // (run, tenancy, unit index, word)
    let mut placed: Vec<(usize, usize, usize, Option<&'static str>)> = vec![];
    for (unit_index, unit) in units.iter().enumerate() {
        let (Some(ux), Some(uz)) = (unit.f("x"), unit.f("z")) else { continue };
        let mut best: Option<(f64, usize, usize)> = None;
        for (ri, run) in runs.iter().enumerate() {
            let b = run.edge.basis;
            let dx = ux - b.origin[0];
            let dz = uz - b.origin[1];
            let along = dx * b.u[0] + dz * b.u[1];
            let off = (dx * b.n[0] + dz * b.n[1]).abs();
            if along < -0.6 || along > run.edge.length + 0.6 || off > 2.5 {
                continue;
            }
            for (ti, t) in run.tenancies.iter().enumerate() {
                if t.pending || t.identity.is_some() {
                    continue;
                }
                let gap = if along < t.s0 { t.s0 - along } else if along > t.s1 { along - t.s1 } else { 0.0 };
                let score = off + gap;
                if best.map(|b| score < b.0).unwrap_or(true) {
                    best = Some((score, ri, ti));
                }
            }
        }
        let Some((_, ri, ti)) = best else { continue };
        runs[ri].tenancies[ti].pending = true;
        placed.push((ri, ti, unit_index, unit_sign_word(unit)));
    }
    let tag = cl.s("tradeTag").unwrap_or("").to_string();
    let record_name = cl.s("recordName").unwrap_or("").to_string();
    let main = resolve_shop_identity(&format!("{id}:shopfront"), &tag, &record_name, None, None);
    let tagged = main.source == "tag" || main.source == "name";
    let front = runs.iter().position(|r| r.edge.role == "front").unwrap_or(0);
    let mut middle: Option<(usize, usize)> = None;
    let mut best_d = f64::INFINITY;
    for (ti, t) in runs[front].tenancies.iter().enumerate() {
        let d = ((t.s0 + t.s1) / 2.0 - runs[front].edge.length / 2.0).abs();
        if middle.is_none() || d < best_d {
            middle = Some((front, ti));
            best_d = d;
        }
    }
    let mut main_tenancy: Option<(usize, usize)> = None;
    let mut main_word: Option<&'static str> = None;
    if tagged {
        let same = placed.iter().find(|p| p.3.map(|w| resolve_shop_identity("x", "", "", Some(w), None).shop_type == main.shop_type).unwrap_or(false));
        main_tenancy = match same {
            Some(p) => {
                if p.0 != front {
                    Some((p.0, p.1))
                } else {
                    middle
                }
            }
            None => placed.first().map(|p| (p.0, p.1)).or(middle),
        };
    } else if let Some(p) = placed.first() {
        main_tenancy = Some((p.0, p.1));
        main_word = p.3;
    }
    if main_tenancy.is_none() && names.len() > 0 {
        main_tenancy = middle;
    }
    if let Some((ri, ti)) = main_tenancy {
        let parody = names.next();
        let ident = resolve_shop_identity(&format!("{id}:shopfront"), &tag, &record_name, main_word, parody.as_deref());
        let t = &mut runs[ri].tenancies[ti];
        t.identity = Some(ident);
        t.pending = false;
    }
    for p in &placed {
        if Some((p.0, p.1)) == main_tenancy {
            continue;
        }
        let parody = names.next();
        let ident = resolve_shop_identity(&format!("{id}:unit:{}", p.2), "", "", p.3, parody.as_deref());
        let t = &mut runs[p.0].tenancies[p.1];
        t.identity = Some(ident);
        t.pending = false;
    }
    for run in runs.iter_mut() {
        let mut k = 0;
        let mut previous: Option<&'static str> = None;
        let mut order: Vec<usize> = (0..run.tenancies.len()).collect();
        order.sort_by(|&a, &b| run.tenancies[a].s0.partial_cmp(&run.tenancies[b].s0).unwrap_or(std::cmp::Ordering::Equal));
        for ti in order {
            let t = &mut run.tenancies[ti];
            if t.identity.is_none() || t.pending {
                let key = format!("{id}:elev:{}:-1:{k}", run.edge.index);
                let mut ident = resolve_shop_identity(&key, "", "", None, None);
                if previous == Some(ident.shop_type) {
                    ident = resolve_shop_identity(&format!("{key}:next"), "", "", None, None);
                }
                t.identity = Some(ident);
                t.pending = false;
                k += 1;
            }
            previous = t.identity.as_ref().map(|i| i.shop_type);
        }
    }
}

struct ShadeArgs {
    floor_y: f64,
    band_bottom: f64,
    band_top: f64,
    shade: String,
    plain: bool,
    projection: f64,
}

/// `addShade(plan, edge, tenancy, opts)`.
fn add_shade(plan: &mut Plan, edge: &Edge, s0: f64, s1: f64, o: &ShadeArgs) -> bool {
    let basis = edge.basis;
    let clearance = o.floor_y + SHOP_AWNING_CLEARANCE_M;
    if o.shade == "awning" {
        let y_top = o.band_bottom - 0.01;
        let valance = 0.22;
        let fall = 0.56f64.min(y_top - valance - clearance);
        if !(fall >= 0.18) {
            return false;
        }
        let p = FITTING_PROJECTION_ALLOWANCE.min(o.projection);
        plan.add_box(
            &basis,
            PartArgs::new(SHOP_DETAIL_KIND, "metal", s0 + 0.02, s1 - 0.02, y_top - 0.04, y_top + 0.01, 0.0, 0.06).edge(edge.index).shadow(false).meta(json!({ "part": "awning-rail" })),
        );
        let slot = if o.plain { "awningPlain" } else { "awning" };
        let sag = 0.045f64.min(fall * 0.12);
        plan.add_profile(
            &basis,
            PartArgs::profile(
                "awning",
                slot,
                s0 + 0.03,
                s1 - 0.03,
                vec![[0.02, y_top], [p * 0.55, y_top - fall * 0.55 - sag], [p, y_top - fall], [p, y_top - fall - valance], [0.05, y_top - 0.06]],
            )
            .edge(edge.index)
            .shadow(true),
        );
        plan.add_box(
            &basis,
            PartArgs::new("canopy-rod", "metal", s0 + 0.03, s1 - 0.03, y_top - fall - 0.035, y_top - fall + 0.005, p - 0.036, p - 0.004)
                .edge(edge.index)
                .shadow(false)
                .meta(json!({ "part": "awning-bar" })),
        );
        for (a, b) in [(s0 + 0.03, s0 + 0.045), (s1 - 0.045, s1 - 0.03)] {
            plan.add_profile(
                &basis,
                PartArgs::profile("awning", slot, a, b, vec![[0.03, y_top - 0.03], [p - 0.005, y_top - fall - 0.005], [p - 0.005, y_top - fall - valance + 0.01]])
                    .edge(edge.index)
                    .shadow(false),
            );
        }
        return true;
    }
    if o.shade == "canopy" {
        let y_plate = o.band_bottom - 0.1;
        if !(y_plate >= clearance) {
            return false;
        }
        let p = 1.6f64.min(o.projection);
        plan.add_box(&basis, PartArgs::new("canopy", "canopy", s0 + 0.02, s1 - 0.02, y_plate, y_plate + 0.1, 0.0, p).edge(edge.index).shadow(true));
        let y_wall = (o.band_top - 0.04).max(y_plate + 0.3);
        let at: Vec<f64> = if s1 - s0 > 3.0 { vec![s0 + 0.6, s1 - 0.6] } else { vec![(s0 + s1) / 2.0] };
        for s in at {
            plan.add_profile(
                &basis,
                PartArgs::profile("canopy-rod", "metal", s - 0.018, s + 0.018, vec![[0.0, y_wall], [0.0, y_wall - 0.04], [p - 0.2, y_plate + 0.095], [p - 0.2, y_plate + 0.13]])
                    .edge(edge.index)
                    .shadow(false),
            );
        }
        return true;
    }
    false
}

/// `addBlade(plan, edge, at, { bandTop, roofCap, text, signs })`.
fn add_blade(plan: &mut Plan, edge: &Edge, at: f64, band_top: f64, roof_cap: f64, text: Option<&str>) {
    let basis = edge.basis;
    let blade_y = band_top + 0.12;
    let blade_h = 1.5f64.min(roof_cap - blade_y);
    let Some(text) = text.filter(|t| !t.is_empty()) else { return };
    if !(blade_h >= 0.5) {
        return;
    }
    let p = 0.95;
    let blade = plan.add_box(
        &basis,
        PartArgs::new("blade-sign", "signage", at - 0.05, at + 0.05, blade_y, blade_y + blade_h, 0.14, p).edge(edge.index).meta(json!({ "text": text })),
    );
    for y in [blade_y + blade_h - 0.05, blade_y] {
        plan.add_box(&basis, PartArgs::new("blade-bracket", "metal", at - 0.02, at + 0.02, y, y + 0.05, 0.0, p).edge(edge.index));
    }
    if let Some(k) = blade {
        let id = plan.parts[k].id.clone();
        plan.signs.push(json!({ "partId": id, "kind": "blade", "text": text, "edge": edge.index, "widthMetres": p - 0.14, "heightMetres": blade_h, "style": null, "face": "side" }));
    }
}

/// `addShopGround(plan, frame, spec)`.
pub fn add(plan: &mut Plan, frame: &Frame, spec: &J) {
    add_with(plan, frame, spec, None)
}

/// `addShopGround(plan, frame, spec, { frontages })`: `frontages` is retail-strip's
/// measured list (`[{ edgeIndex, primary, tenancies: [{ bayFrom, bayTo, glazed, ... }] }]`).
pub fn add_with(plan: &mut Plan, frame: &Frame, spec: &J, frontages: Option<&J>) {
    if plan.extra.get("shopGround").map(|v| truthy(Some(v))).unwrap_or(false) {
        return;
    }
    let Some(ground) = plan.storeys.first().cloned() else { return };
    let sg = spec.get_nn("shopGround");
    let edge_indices: Vec<usize> = sg.arr("edges").map(|a| a.iter().filter_map(|v| v.as_f64()).map(|v| v as usize).collect()).unwrap_or_default();
    if edge_indices.is_empty() {
        return;
    }
    let cl = sg.get_nn("classified").cloned().unwrap_or(J::Null);
    let lift = if plan.base_y.is_nan() { 0.0 } else { plan.base_y };
    let floor_y = ground.y0 + lift;
    let floor_line = ground.y1 + lift;
    let wall_top = plan.envelope.wall_top + lift;
    let roof_cap = plan.envelope.roof_top + lift - 0.04;
    let seed = match cl.get_nn("seed") {
        Some(J::Number(n)) => js_num(n.as_f64().unwrap_or(0.0)),
        Some(J::String(s)) => s.clone(),
        _ => plan.seed.clone(),
    };
    let mut random = Rng::new(&format!("{seed}|shop-ground"));
    let is_prewar = prewar(num_or(cl.at("yearBuilt"), 0.0), cl.num("storeys"));
    let pier = pick(if is_prewar { &PIER_FINISHES_PREWAR } else { &PIER_FINISHES_POSTWAR }, &mut random);
    plan.materials.insert("shopPier".into(), pier.into());
    for (_, slot, material) in SHOP_GLASS_FAMILIES {
        if plan.materials.get(slot).map(|s| s.is_empty()).unwrap_or(true) {
            plan.materials.insert(slot.into(), material.into());
        }
    }
    let mut shop_bays: Vec<J> = vec![];
    let mut tenancy_count = 0usize;

    // Pass 1: the tenancies along every shop frontage.
    let mut runs: Vec<Run> = vec![];
    for &index in &edge_indices {
        let Some(edge) = frame.edges.iter().find(|e| e.index == index) else { continue };
        let Some(record) = plan.edges.iter().find(|e| e.index == index) else { continue };
        let glazed: Vec<OpeningRecord> = record.openings.iter().filter(|op| op.storey == 0 && (op.type_ == "storefront" || op.type_ == "entrance")).cloned().collect();
        if !glazed.iter().any(|op| op.type_ == "storefront") {
            continue;
        }
        let measured = frontages.and_then(|f| f.as_array()).and_then(|a| a.iter().find(|f| f.f("edgeIndex") == Some(index as f64)));
        let mut tenancies = vec![];
        if let Some(measured) = measured {
            let bays: &Vec<BayRecord> = &record.bays;
            for t in measured.arr("tenancies").cloned().unwrap_or_default() {
                if !t.truthy("glazed") {
                    continue;
                }
                let from = t.num("bayFrom");
                let to = t.num("bayTo") - 1.0;
                let get = |i: f64| if i >= 0.0 && i.fract() == 0.0 { bays.get(i as usize) } else { None };
                let (Some(first), Some(last)) = (get(from), get(to)) else { continue };
                let openings = glazed.iter().filter(|op| op.s0 >= first.s0 - 1e-3 && op.s1 <= last.s1 + 1e-3).cloned().collect();
                tenancies.push(Tenancy { s0: first.s0, s1: last.s1, openings, measured: Some(t.clone()), pending: false, identity: None });
            }
        } else {
            tenancies = tenancies_from_openings(&glazed, &mut random);
        }
        if !tenancies.is_empty() {
            runs.push(Run { index, edge, tenancies });
        }
    }
    assign_shop_identities(&mut runs, &cl, &format!("sf-building-{seed}"));

    // Pass 2: build each frontage's assembly.
    for run in &runs {
        let index = run.index;
        let edge = run.edge;
        let basis = edge.basis;
        let pier_projection = edge.allowance.min(lerp(m::PIER_PROJECTION, random.next()));
        let pier_width = lerp(m::PIER_WIDTH, random.next());
        let cornice_h = lerp(m::CORNICE_HEIGHT, random.next());
        let cornice_p = edge.allowance.min(lerp(m::CORNICE_PROJECTION, random.next()));
        let profile = if is_prewar { if random.next() < 0.6 { "ogee" } else { "stepped" } } else { "stepped" };
        let has_cornice = plan.parts.iter().any(|p| {
            p.edge == Some(index)
                && (p.kind == "string-course" || (p.kind == "cornice" && p.meta.s("part") != Some("revival-pent")))
                && p.y1 > floor_line - 0.45
                && p.y0 < floor_line + 0.45
        });
        let mut pier_places: Vec<(f64, f64)> = vec![];
        let mut run_s0 = f64::INFINITY;
        let mut run_s1 = f64::NEG_INFINITY;
        let mut top_of_bands = f64::NEG_INFINITY;

        for (tenancy_index, tenancy) in run.tenancies.iter().enumerate() {
            let t = tenancy.measured.as_ref();
            let (s0, s1) = (tenancy.s0, tenancy.s1);
            run_s0 = run_s0.min(s0);
            run_s1 = run_s1.max(s1);
            let head = tenancy.openings.iter().map(|op| op.y1).fold(floor_y + 2.0, f64::max);
            let identity = tenancy.identity.as_ref();
            let text: Option<String> = identity.and_then(|i| i.text.clone()).filter(|s| !s.is_empty());
            let band_bottom = head + 0.02;
            let wanted = t.and_then(|t| t.get_nn("signHeight")).map(|v| to_number(Some(v))).unwrap_or(floor_line - 0.02 - band_bottom);
            let two_line = text.as_deref().map(|s| s.contains('|')).unwrap_or(false);
            let band_ceiling = (if has_cornice { floor_line - 0.02 } else { floor_line + if t.is_some() { 0.45 } else { 0.08 } }).min(wall_top + 0.45).min(roof_cap);
            let band_top = (band_bottom + wanted.max(if two_line { 0.72 } else { 0.42 })).min(band_ceiling);
            let band_builds = band_top - band_bottom >= 0.26;
            if band_builds {
                top_of_bands = top_of_bands.max(band_top);
                let board = plan.add_box(
                    &basis,
                    PartArgs::new("signband", "signage", s0, s1, band_bottom, band_top, -0.01, m::FASCIA_PROJECTION)
                        .edge(index)
                        .shadow(true)
                        .meta(json!({ "tenancy": tenancy_index, "text": text })),
                );
                let mut sign_at: Option<usize> = None;
                if let (Some(k), Some(tx)) = (board, &text) {
                    let style = (random.next() * FASCIA_STYLE_COUNT).floor();
                    let id = plan.parts[k].id.clone();
                    plan.signs.push(json!({
                        "partId": id, "kind": "fascia", "text": tx, "edge": index, "widthMetres": s1 - s0, "heightMetres": band_top - band_bottom,
                        "style": { "era": if is_prewar { "prewar" } else { "postwar" }, "index": style }, "roughness": 0.62, "face": "front", "lamps": 0,
                    }));
                    sign_at = Some(plan.signs.len() - 1);
                }
                if text.is_some() && random.next() < SHOP_GOOSENECK_SHARE {
                    let count = if s1 - s0 > 5.5 { 3 } else { 2 };
                    if let Some(k) = sign_at {
                        set(&mut plan.signs[k], "lamps", json!(count));
                    }
                    for i in 0..count {
                        let at = s0 + ((s1 - s0) * (i as f64 + 0.5)) / count as f64;
                        plan.add_box(
                            &basis,
                            PartArgs::new(SHOP_LAMP_KIND, "metal", at - 0.012, at + 0.012, band_top - 0.005, band_top + 0.025, m::FASCIA_PROJECTION - 0.01, 0.41)
                                .edge(index)
                                .shadow(false)
                                .meta(json!({ "part": "gooseneck-arm" })),
                        );
                        plan.add_box(
                            &basis,
                            PartArgs::new(SHOP_LAMP_KIND, "metal", at - 0.075, at + 0.075, band_top - 0.1, band_top - 0.01, 0.33, 0.47)
                                .edge(index)
                                .shadow(false)
                                .meta(json!({ "part": "gooseneck-shade" })),
                        );
                    }
                }
                plan.add_box(
                    &basis,
                    PartArgs::new(SHOP_DETAIL_KIND, "shopPier", s0, s1, band_bottom - 0.04, band_bottom + 0.02, -0.01, m::FASCIA_PROJECTION + 0.03)
                        .edge(index)
                        .shadow(false)
                        .meta(json!({ "part": "bed-mould" })),
                );
            }

            // Stall riser.
            let accent_index = match t {
                Some(t) => BULKHEAD_ACCENTS.iter().position(|a| Some(*a) == t.s("accent")).unwrap_or(0),
                None => (random.next() * BULKHEAD_ACCENTS.len() as f64).floor() as usize,
            };
            let accent = format!("tenancy{accent_index}");
            for op in &tenancy.openings {
                if op.type_ == "entrance" || !(op.y0 > floor_y + 0.2) {
                    continue;
                }
                let a = op.s0 - 0.01;
                let b = op.s1 + 0.01;
                let riser_top = op.y0 - 0.06;
                // No grade: the footway is level with the floor (`hi` = 0).
                let kick_top = (floor_y + m::KICK).min(riser_top - 0.08);
                plan.add_box(
                    &basis,
                    PartArgs::new(SHOP_DETAIL_KIND, "base", a, b, floor_y - m::SKIRT, kick_top, -0.01, 0.06).edge(index).meta(json!({ "part": "kick", "tenancy": tenancy_index })),
                );
                plan.add_box(
                    &basis,
                    PartArgs::new("stall-riser", &accent, a + 0.02, b - 0.02, kick_top, riser_top, -0.01, 0.036)
                        .edge(index)
                        .receive(true)
                        .meta(json!({ "part": "panel", "tenancy": tenancy_index })),
                );
                if riser_top - kick_top >= 0.2 && b - a > 0.8 {
                    plan.add_box(
                        &basis,
                        PartArgs::new(SHOP_DETAIL_KIND, &accent, a + 0.11, b - 0.11, kick_top + 0.07, riser_top - 0.07, 0.036, 0.052)
                            .edge(index)
                            .shadow(false)
                            .meta(json!({ "part": "riser-field", "tenancy": tenancy_index })),
                    );
                }
                let mut max_room_depth = 0.0;
                let mut depth = 1.0;
                while depth <= 3.01 {
                    let inside = [0.08, 0.5, 0.92].iter().all(|tt| {
                        let s = op.s0 + (op.s1 - op.s0) * tt;
                        frame.signed_distance(basis.origin[0] + basis.u[0] * s - basis.n[0] * depth, basis.origin[1] + basis.u[1] * s - basis.n[1] * depth) < -0.3
                    });
                    if !inside {
                        break;
                    }
                    max_room_depth = depth;
                    depth += 0.25;
                }
                let sfs = spec.pointer(&format!("/elevations/byEdge/{index}/storefrontSpec")).filter(|v| !v.is_null());
                shop_bays.push(json!({
                    "openingId": op.id, "maxRoomDepth": max_room_depth, "edge": index, "tenancy": tenancy_count,
                    "s0": op.s0, "s1": op.s1, "y0": op.y0, "y1": op.y1, "floorY": floor_y,
                    "shopType": identity.map(|i| i.shop_type),
                    "glassW": -sfs.f("setback").unwrap_or(m::SETBACK).min(m::REVEAL),
                    "frameWidth": sfs.f("frameWidth").unwrap_or(m::FRAME_WIDTH),
                    "text": text,
                    "basis": { "origin": basis.origin, "u": basis.u, "n": basis.n },
                }));
            }

            // Shade.
            if band_builds {
                let shade = match t {
                    Some(t) => t.s("shade").unwrap_or("").to_string(),
                    None => pick(&["awning", "none", "none", "canopy", "none", "awning", "none"], &mut random).to_string(),
                };
                let plain = match t {
                    Some(t) => t.truthy("awningPlain"),
                    None => random.next() < 0.25,
                };
                let projection = match t {
                    Some(t) => t.num("awningProjection"),
                    None => 1.05 + random.next() * 0.45,
                };
                add_shade(plan, edge, s0, s1, &ShadeArgs { floor_y, band_bottom, band_top, shade, plain, projection });
            }

            // Blade.
            let blade = match t {
                Some(t) => t.truthy("blade"),
                None => random.next() < 0.12,
            };
            if blade && band_builds {
                add_blade(plan, edge, (edge.length - 0.2).min(s1 + 0.02), band_top, roof_cap, identity.map(|i| i.word.as_str()));
            }

            for s in [s0, s1] {
                let key = js_round(s * 100.0) / 100.0;
                match pier_places.iter_mut().find(|p| p.0 == key) {
                    Some(p) => p.1 = s,
                    None => pier_places.push((key, s)),
                }
            }
            tenancy_count += 1;
        }

        if !top_of_bands.is_finite() {
            top_of_bands = floor_line - 0.02;
        }
        let pier_top = top_of_bands.min(wall_top - 0.02).min(roof_cap);

        // Piers.
        let mut places: Vec<f64> = pier_places.iter().map(|p| p.1).collect();
        places.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mut merged: Vec<f64> = vec![];
        for s in places {
            match merged.last_mut() {
                Some(last) if s - *last < pier_width * 1.4 => *last = (*last + s) / 2.0,
                _ => merged.push(s),
            }
        }
        for s in merged {
            let a = 0f64.max(s - pier_width / 2.0);
            let b = edge.length.min(s + pier_width / 2.0);
            if b - a < 0.12 {
                continue;
            }
            let pier_foot = floor_y - m::SKIRT;
            plan.add_box(&basis, PartArgs::new("pilaster", "shopPier", a, b, pier_foot, pier_top, -0.01, pier_projection).edge(index).meta(json!({ "part": "shop-pier" })));
            plan.add_box(
                &basis,
                PartArgs::new(SHOP_DETAIL_KIND, "base", a - 0.025, b + 0.025, pier_foot, (floor_y + 0.28).min(pier_top - 0.2), -0.01, pier_projection + 0.025)
                    .edge(index)
                    .meta(json!({ "part": "pier-base" })),
            );
            plan.add_box(
                &basis,
                PartArgs::new(SHOP_DETAIL_KIND, "shopPier", a - 0.03, b + 0.03, pier_top - 0.2, pier_top, -0.01, pier_projection + 0.05)
                    .edge(index)
                    .meta(json!({ "part": "pier-console" })),
            );
        }

        // Cornice.
        if !has_cornice && top_of_bands + cornice_h <= roof_cap {
            plan.add_profile(
                &basis,
                PartArgs::profile(
                    "cornice",
                    "trim",
                    0f64.max(run_s0 - pier_width / 2.0 - 0.06),
                    edge.length.min(run_s1 + pier_width / 2.0 + 0.06),
                    trim_profile(profile, cornice_p, cornice_h, top_of_bands - 0.01, -0.02),
                )
                .edge(index)
                .meta(json!({ "part": "shop-cornice" })),
            );
        }

        // Glass and doors.
        let mut shop_spans: Vec<(f64, f64, &str)> = vec![];
        for tenancy in &run.tenancies {
            let family = tenancy.identity.as_ref().map(|i| i.family).unwrap_or("goods");
            let slot = SHOP_GLASS_FAMILIES.iter().find(|f| f.0 == family).unwrap_or(&SHOP_GLASS_FAMILIES[0]).1;
            for op in &tenancy.openings {
                if op.type_ == "storefront" || (op.s0 >= tenancy.s0 - 0.05 && op.s1 <= tenancy.s1 + 0.05) {
                    shop_spans.push((op.s0 - 0.02, op.s1 + 0.02, slot));
                }
            }
        }
        let shop_slot = |s0: f64, s1: f64| -> Option<&str> {
            let mid = (s0 + s1) / 2.0;
            shop_spans.iter().find(|sp| mid >= sp.0 && mid <= sp.1).map(|sp| sp.2)
        };
        let in_ground = |p: &crate::elevation::Part| p.edge == Some(index) && p.storey == Some(0) && shop_slot(p.s0, p.s1).is_some();
        for k in 0..plan.parts.len() {
            if !in_ground(&plan.parts[k]) {
                continue;
            }
            let p = &plan.parts[k];
            if p.kind == "glass" || p.kind == "transom-glass" || (p.kind == "door-leaf" && p.slot == "doorGlass") {
                let slot = shop_slot(p.s0, p.s1).unwrap().to_string();
                plan.parts[k].slot = slot;
            }
        }
        let leaves: Vec<crate::elevation::Part> =
            plan.parts.iter().filter(|p| in_ground(p) && p.kind == "door-leaf" && SHOP_GLASS_SLOTS.contains(&p.slot.as_str())).cloned().collect();
        for leaf in leaves {
            let hinge = leaf.meta.get_nn("hinge").f("s").unwrap_or(leaf.s0);
            let lock_s = if (hinge - leaf.s0).abs() < (hinge - leaf.s1).abs() { leaf.s1 - 0.045 } else { leaf.s0 + 0.045 };
            let face = leaf.w1 + 0.006;
            let foot_y = leaf.y0;
            let meta = |part: &str| {
                let mut m = if leaf.meta.truthy("sunkEntry") {
                    json!({ "belowFloor": true, "sunkEntry": true, "floorY": leaf.meta.get_nn("floorY").cloned().unwrap_or(J::Null) })
                } else {
                    json!({})
                };
                set(&mut m, "part", json!(part));
                m
            };
            plan.add_box(
                &basis,
                PartArgs::new(SHOP_DETAIL_KIND, "metal", lock_s - 0.014, lock_s + 0.014, foot_y + 0.9, foot_y + 1.28, face + 0.03, face + 0.06)
                    .edge(index)
                    .storey(0)
                    .shadow(false)
                    .meta(meta("door-pull")),
            );
            plan.add_box(
                &basis,
                PartArgs::new(SHOP_DETAIL_KIND, "metal", leaf.s0 + 0.012, leaf.s1 - 0.012, foot_y + 0.02, foot_y + 0.22, face, face + 0.006)
                    .edge(index)
                    .storey(0)
                    .shadow(false)
                    .meta(meta("door-kick")),
            );
        }
    }

    let bays_len = shop_bays.len();
    plan.extra.insert("shopBays".into(), J::Array(shop_bays));
    plan.extra.insert(
        "shopGround".into(),
        json!({
            "version": SHOP_GROUND_VERSION,
            "tenancies": tenancy_count,
            "bays": bays_len,
            "edges": edge_indices,
            "sideDoor": sg.get_nn("sideDoor").cloned().unwrap_or(J::Null),
        }),
    );
}
