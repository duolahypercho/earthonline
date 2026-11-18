//! The San Francisco retail strip (src/world/buildings/archetypes/retail-strip.js):
//! a mechanical port of `retailStripSpec` and `addRetailStripDetail`.
//! `checkSignText` (kit/signage.js) is ported here as `check_sign_text` with the
//! shipped digest denylist; the earlier version's optional per-machine extra
//! digests (`extraDigests`) are not read.

use crate::classify::Classified;
use crate::elevation::{PartArgs, Plan};
use crate::frame::Frame;
use crate::js::*;
use crate::rng::{hash_seed, Rng};
use crate::shop_ground::{add_with, BULKHEAD_ACCENTS, TRADE_WORDS};
use std::collections::VecDeque;

pub const RETAIL_STRIP_VERSION: &str = "sf-archetype-retail-strip-v2";

const GROUND_TARGET_M: f64 = 3.9;
const MIN_GROUND_M: f64 = 2.6;
const UPPER_STOREY_M: f64 = 3.0;
const TWO_STOREY_M: f64 = 5.0;
const COPING_M: f64 = 0.08;
const PARAPET_MIN: f64 = 0.72;
const PARAPET_MAX: f64 = 1.18;
const SIGNBAND_MIN: f64 = 0.66;
const SIGNBAND_MAX: f64 = 0.98;
const HEAD_CLEAR_M: f64 = 0.40;
const STALL_RISER_MIN: f64 = 0.36;
const STALL_RISER_MAX: f64 = 0.58;
const END_PIER_M: f64 = 0.30;
const TENANCY_TARGET_MIN: f64 = 6.4;
const TENANCY_TARGET_MAX: f64 = 9.6;
const MAX_TENANCIES: f64 = 8.0;
const ENTRY_BAY_MIN: f64 = 1.95;
const ENTRY_BAY_MAX: f64 = 2.45;

const WALL_FINISHES: [&str; 7] = [
    "painted-stucco", "float-finish-stucco", "painted-stucco", "brick-running-bond",
    "painted-block", "scanned-painted-plaster", "float-finish-stucco",
];
const REVIVAL_FROM: f64 = 1910.0;
const REVIVAL_TO: f64 = 1945.0;
const REVIVAL_DISTRICTS: [&str; 16] = [
    "Marina", "Pacific Heights", "Presidio Heights", "Seacliff",
    "Sunset/Parkside", "Inner Sunset", "Outer Richmond", "Inner Richmond", "West of Twin Peaks",
    "Excelsior", "Outer Mission", "Oceanview/Merced/Ingleside", "Lakeshore", "Twin Peaks", "Glen Park", "Portola",
];
const REVIVAL_SHARE: f64 = 0.7;
const REVIVAL_PENT_PROJECTION: f64 = 0.42;
const REVIVAL_PENT_HEIGHT: f64 = 0.32;
const FASCIA_FINISHES: [&str; 4] = ["painted-steel", "signal-red", "painted-white", "painted-steel-mid"];
const STOREFRONT_FRAMES: [&str; 4] = ["anodized-aluminium", "painted-steel", "anodized-aluminium", "painted-steel-mid"];
const GLASS_SLOTS: [&str; 5] = ["glass", "glassWarm", "glassCool", "glass", "glassWarm"];
const SHADES: [&str; 5] = ["awning", "awning", "canopy", "none", "none"];

/// kit/signage.js `SIGN_DENYLIST_DIGESTS`.
const SIGN_DENYLIST_DIGESTS: [u32; 243] = [
    768418, 3498942, 15639518, 34484083, 62366780, 68159807, 97189229, 108775112, 110778522, 120287048, 147163597, 183895099,
    192013946, 209044288, 212475776, 228712814, 232372193, 248737696, 248903295, 252403531, 253006652, 268143074, 299662067, 344614518,
    357606765, 422868715, 434560502, 439379688, 440652853, 500421696, 518153670, 549522431, 556121862, 604996244, 612012049, 625763385,
    656882518, 677453583, 679506516, 717802614, 720018959, 720645044, 726483662, 785865631, 787384196, 788269066, 794501977, 816349845,
    829892241, 848834650, 859980292, 906789362, 922495178, 942355954, 943361643, 957769600, 959357363, 966881310, 972071773, 985761528,
    989068619, 990422336, 1006584145, 1021609616, 1060247414, 1062138189, 1068407769, 1071833916, 1097887312, 1110289794, 1152736536, 1202073361,
    1209286198, 1250510879, 1253322018, 1269806420, 1272761525, 1326634411, 1339048652, 1347650592, 1354432531, 1377245229, 1382249916, 1386436845,
    1386927198, 1395262093, 1430912854, 1455910212, 1477453414, 1480151085, 1500531605, 1510004807, 1521812279, 1546070497, 1607891328, 1608886399,
    1633416709, 1645715523, 1650746889, 1668940789, 1679378555, 1698110531, 1702040454, 1716545717, 1717663153, 1719620384, 1740642476, 1797863409,
    1859556515, 1866041447, 1903992814, 1975413752, 1998597252, 2000053149, 2035183131, 2046283717, 2081705117, 2087865587, 2090206839, 2107985256,
    2131311227, 2135559081, 2150946214, 2179565191, 2184956371, 2214926114, 2237361058, 2238017689, 2245142003, 2263208990, 2268324328, 2309052736,
    2313353578, 2349317581, 2361200706, 2369064499, 2388542312, 2439174649, 2457734258, 2470412037, 2492196944, 2495776680, 2510971569, 2511170941,
    2520803653, 2524830885, 2532441587, 2568894802, 2580265200, 2656202268, 2679930945, 2682701165, 2775300852, 2788099943, 2795388055, 2798124894,
    2799447562, 2829265170, 2835120838, 2850087183, 2868526702, 2876755531, 2876911791, 2884372083, 2894583849, 2919567946, 2975368727, 2982748961,
    2993696001, 3012714941, 3017725047, 3019928626, 3028276742, 3049356131, 3074123037, 3082238232, 3155182174, 3156870618, 3191661535, 3193581898,
    3196521666, 3205842463, 3225196479, 3233284392, 3258838922, 3287276830, 3314142817, 3353180806, 3375796893, 3409238208, 3412428690, 3445015971,
    3450974470, 3456659880, 3459693020, 3489318092, 3503775159, 3557301939, 3607990670, 3614550613, 3615413050, 3624101773, 3693306370, 3710037220,
    3711716140, 3715645667, 3772269493, 3798733233, 3799936374, 3808085306, 3823726112, 3825369471, 3827043864, 3834529175, 3835613527, 3847900768,
    3873215680, 3873392669, 3883036277, 3883440389, 3888625101, 3890603922, 3922175739, 3937206568, 3940608427, 3944112511, 3955438145, 3978427474,
    3980898643, 4000545559, 4003508827, 4019848226, 4031195738, 4093687938, 4100559537, 4131746517, 4155109292, 4192578183, 4195546319, 4199607396,
    4201425791, 4204836446, 4247287712,
];

/// kit/signage.js `normaliseSignText`.
fn normalise_sign_text(text: &str) -> String {
    text.to_lowercase().chars().filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit()).collect()
}

/// kit/signage.js `checkSignText(text).ok` (no machine-local extra digests).
pub fn check_sign_text(text: &str) -> bool {
    if text.trim().is_empty() {
        return false;
    }
    if text.contains(['\u{2122}', '\u{00ae}', '\u{00a9}']) {
        return false;
    }
    if text.encode_utf16().count() > 64 {
        return false;
    }
    let hit = |candidate: &str| SIGN_DENYLIST_DIGESTS.contains(&hash_seed(&format!("sign|{}", normalise_sign_text(candidate))));
    if hit(text) {
        return false;
    }
    let tokens: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || matches!(c, '-' | '_' | '/' | '.' | ',' | '&' | '+'))
        .filter(|t| !t.is_empty())
        .collect();
    for i in 0..tokens.len() {
        let mut span = 1;
        while span <= 3 && i + span <= tokens.len() {
            if hit(&tokens[i..i + span].join(" ")) {
                return false;
            }
            span += 1;
        }
    }
    true
}

struct Stack {
    storeys: Vec<(f64, &'static str)>,
    parapet: f64,
    coping: f64,
}

/// `storeyStack(height, random)`.
fn storey_stack(height: f64, random: &mut Rng) -> Stack {
    let mut parapet = PARAPET_MIN + random.next() * (PARAPET_MAX - PARAPET_MIN);
    let mut coping = COPING_M;
    let mut usable = height - parapet - coping;
    if usable < MIN_GROUND_M {
        parapet = 0.2f64.max(height - coping - MIN_GROUND_M);
        usable = height - parapet - coping;
    }
    if usable < 2.3 {
        parapet = 0.0;
        coping = 0.0;
        usable = height;
    }
    if usable >= TWO_STOREY_M {
        let upper_count = 1f64.max(js_round((usable - GROUND_TARGET_M) / UPPER_STOREY_M));
        let mut ground = GROUND_TARGET_M;
        let mut upper = (usable - ground) / upper_count;
        if upper < 2.45 {
            ground = MIN_GROUND_M.max(usable - 2.45 * upper_count);
            upper = (usable - ground) / upper_count;
        }
        if upper >= 2.25 {
            let mut storeys = vec![(ground, "retail")];
            for _ in 0..upper_count as usize {
                storeys.push((upper, "upper"));
            }
            return Stack { storeys, parapet, coping };
        }
    }
    Stack { storeys: vec![(usable, "retail")], parapet, coping }
}

/// `pick(pool, random)`.
fn pick<'a>(pool: &[&'a str], random: &mut Rng) -> &'a str {
    let i = ((random.next() * pool.len() as f64).floor() as usize).min(pool.len() - 1);
    pool[i]
}

/// `tradeCopy(random, avoid)`.
fn trade_copy(random: &mut Rng, avoid: Option<&str>) -> String {
    let mut word = pick(&TRADE_WORDS, random);
    if Some(word) == avoid {
        word = pick(&TRADE_WORDS, random);
    }
    let mut text = word.to_string();
    if random.next() < 0.18 {
        let second = pick(&TRADE_WORDS, random);
        if second != word {
            text = format!("{word}|{second}");
        }
    }
    if check_sign_text(&text) { text } else { word.to_string() }
}

/// `{ 0: ground, ...upper }`.
fn bay_storeys(ground: &str, upper: &J) -> J {
    let mut m = Map::new();
    m.insert("0".into(), J::String(ground.into()));
    if let Some(o) = upper.as_object() {
        for (k, v) in o {
            m.insert(k.clone(), v.clone());
        }
    }
    J::Object(m)
}

/// `tenancyBays(width, retail, upper, glassSlot, random)`: (pattern, bays).
fn tenancy_bays(width: f64, retail: bool, upper: &J, glass_slot: &str, random: &mut Rng) -> (&'static str, Vec<J>) {
    let entry = (ENTRY_BAY_MIN + random.next() * (ENTRY_BAY_MAX - ENTRY_BAY_MIN)).min(1.6f64.max(width * 0.34));
    let shop = |w: f64| json!({ "width": w, "kind": "storefront", "glassSlot": glass_slot, "storeys": bay_storeys("storefront", upper) });
    let door = |w: f64, random: &mut Rng| {
        let recess = 0.34 + random.next() * 0.5;
        let leaf = if random.next() < 0.72 { "double-glass" } else { "single-glass" };
        json!({
            "width": w, "kind": "entrance", "glassSlot": glass_slot,
            "storeys": bay_storeys("entrance", upper),
            "entrance": { "recess": recess, "leaf": leaf },
        })
    };
    let punched = |w: f64| json!({ "width": w, "kind": "punched", "storeys": bay_storeys("punched", upper) });

    if !retail {
        if width < 3.2 {
            return ("blank-base", vec![punched(width)]);
        }
        let lobby = 2.2f64.min(width * 0.4);
        let rest = width - lobby;
        let bays = if random.next() < 0.5 {
            let d = door(lobby, random);
            vec![d, punched(rest)]
        } else {
            let p = punched(rest);
            vec![p, door(lobby, random)]
        };
        return ("lobby-base", bays);
    }
    if width < 3.4 {
        return ("kiosk", vec![shop(width)]);
    }
    let roll = random.next();
    if width >= 8.2 && roll < 0.34 {
        let side = (width - entry) / 2.0;
        let a = shop(side);
        let d = door(entry, random);
        return ("entry-centre", vec![a, d, shop(side)]);
    }
    if roll < 0.62 {
        let d = door(entry, random);
        return ("entry-left", vec![d, shop(width - entry)]);
    }
    let a = shop(width - entry);
    ("entry-right", vec![a, door(entry, random)])
}

struct TenancyPlan {
    end_pier: f64,
    bays: Vec<J>,
    tenancies: Vec<J>,
}

/// `planTenancies(edgeLength, retail, upper, random, signTexts)`.
fn plan_tenancies(edge_length: f64, retail: bool, upper: &J, random: &mut Rng, sign_texts: &mut VecDeque<String>) -> TenancyPlan {
    let end_pier = END_PIER_M.min(edge_length * 0.07);
    let available = 0.8f64.max(edge_length - 2.0 * end_pier);
    let target = TENANCY_TARGET_MIN + random.next() * (TENANCY_TARGET_MAX - TENANCY_TARGET_MIN);
    let count = 1f64.max(MAX_TENANCIES.min(js_round(available / target))) as usize;

    let mut weights = vec![];
    let mut weight_sum = 0.0;
    for _ in 0..count {
        let w = 0.74 + random.next() * 0.52;
        weights.push(w);
        weight_sum += w;
    }

    let mut bays: Vec<J> = vec![];
    let mut tenancies: Vec<J> = vec![];
    let mut previous_word: Option<String> = None;
    for (i, weight) in weights.iter().enumerate() {
        let width = (available * weight) / weight_sum * (1.0 - 1e-9);
        let glass_slot = pick(&GLASS_SLOTS, random);
        let (pattern, unit_bays) = tenancy_bays(width, retail, upper, glass_slot, random);
        let bay_from = bays.len();
        let glazed = unit_bays.iter().any(|b| matches!(b.s("kind"), Some("storefront") | Some("entrance")));
        bays.extend(unit_bays);
        let generic = if retail && glazed { Some(trade_copy(random, previous_word.as_deref())) } else { None };
        let recorded = if retail && glazed && !sign_texts.is_empty() { sign_texts.pop_front() } else { None };
        let text = match recorded {
            Some(r) if !r.is_empty() && check_sign_text(&r) => Some(r),
            _ => generic,
        };
        if let Some(t) = text.as_ref().filter(|t| !t.is_empty()) {
            previous_word = Some(t.split('|').next().unwrap_or("").to_string());
        }
        let accent = pick(&BULKHEAD_ACCENTS, random);
        let two_line = text.as_ref().map(|t| !t.is_empty() && t.contains('|')).unwrap_or(false);
        let sign_height = (SIGNBAND_MIN + random.next() * (SIGNBAND_MAX - SIGNBAND_MIN)) * if two_line { 1.22 } else { 1.0 };
        let shade = if glazed { pick(&SHADES, random) } else { "none" };
        let awning_projection = 1.12 + random.next() * 0.46;
        let awning_plain = random.next() < 0.42;
        let blade = glazed && random.next() < 0.16;
        tenancies.push(json!({
            "index": i,
            "pattern": pattern,
            "bayFrom": bay_from,
            "bayTo": bays.len(),
            "width": width,
            "glazed": glazed,
            "glassSlot": glass_slot,
            "accent": accent,
            "text": text,
            "signHeight": sign_height,
            "shade": shade,
            "awningProjection": awning_projection,
            "awningPlain": awning_plain,
            "blade": blade,
            "bladeText": J::Null,
        }));
    }
    for tenancy in tenancies.iter_mut() {
        if tenancy.truthy("blade") {
            set(tenancy, "bladeText", J::String(trade_copy(random, None)));
        }
    }
    TenancyPlan { end_pier, bays, tenancies }
}

/// `upperTreatment(storeyCount, random)`.
fn upper_treatment(storey_count: usize, random: &mut Rng) -> J {
    if storey_count < 2 {
        return json!({});
    }
    let sill = 0.72 + random.next() * 0.26;
    let mullions = if random.next() < 0.45 { 1 } else { 0 };
    let mut out = Map::new();
    for i in 1..storey_count {
        out.insert(i.to_string(), json!({ "window": "punched", "sillHeight": sill, "mullions": mullions }));
    }
    J::Object(out)
}

/// `retailStripSpec(classified, frame)`.
pub fn spec(c: &Classified, frame: &Frame) -> J {
    let mut random = Rng::from_num(c.seed as f64);
    let Stack { storeys, parapet, coping } = storey_stack(c.height, &mut random);
    let drawn_wall = pick(&WALL_FINISHES, &mut random);
    let mut revival_random = Rng::new(&format!("{}|revival", c.seed));
    let year = if c.year_exact == Some(true) { Some(c.year_built) } else { None };
    let district = c.civic.as_ref().and_then(|v| v.district.as_deref());
    let revival = parapet > 0.3
        && year.map(|y| y >= REVIVAL_FROM && y <= REVIVAL_TO).unwrap_or(false)
        && district.map(|d| REVIVAL_DISTRICTS.contains(&d)).unwrap_or(false)
        && revival_random.next() < REVIVAL_SHARE;
    let wall = if revival {
        if revival_random.next() < 0.6 { "painted-stucco" } else { "float-finish-stucco" }
    } else {
        drawn_wall
    };
    let revival_paint = if revival && revival_random.next() < 0.34 { json!(0xf6eedd) } else { J::Null };
    let fascia = pick(&FASCIA_FINISHES, &mut random);
    let joinery = pick(&STOREFRONT_FRAMES, &mut random);
    let stall_riser = STALL_RISER_MIN + random.next() * (STALL_RISER_MAX - STALL_RISER_MIN);
    let retail = c.has_retail_ground;

    let front = frame.edge_for_role("front");
    let mut front_length = front.map(|e| e.length).unwrap_or(0.0);
    if front_length == 0.0 || front_length.is_nan() {
        front_length = c.frontage;
    }
    if front_length == 0.0 || front_length.is_nan() {
        front_length = 12.0;
    }
    let upper = upper_treatment(storeys.len(), &mut random);
    let mut sign_texts: VecDeque<String> = c.sign_texts.iter().cloned().collect();
    if let Some(t) = c.sign_trade.as_ref().filter(|t| !t.is_empty()) {
        sign_texts.push_back(t.clone());
    }
    let mut front_plan = plan_tenancies(front_length, retail, &upper, &mut random, &mut sign_texts);

    let mut secondary: Option<&crate::frame::Edge> = None;
    for edge in frame.edges_for_role("front-secondary") {
        if secondary.map(|b| edge.length > b.length).unwrap_or(true) {
            secondary = Some(edge);
        }
    }
    let mut secondary_plan = match secondary {
        Some(s) if s.length >= 4.0 => {
            let up = upper_treatment(storeys.len(), &mut random);
            Some(plan_tenancies(s.length, retail, &up, &mut random, &mut sign_texts))
        }
        _ => None,
    };

    let ground_height = storeys[0].0;
    let head = ground_height - HEAD_CLEAR_M;
    let entrance_height = 2.05f64.max(2.78f64.min(head - 0.45));
    for plan in [Some(&mut front_plan), secondary_plan.as_mut()].into_iter().flatten() {
        for bay in plan.bays.iter_mut() {
            if bay.truthy("entrance") {
                set(obj_mut(bay, "entrance"), "height", json!(entrance_height));
            }
        }
    }

    let storefront_spec = json!({
        "stallRiser": stall_riser,
        "headClear": HEAD_CLEAR_M,
        "pier": 0.12,
        "mullionSpacing": 1.55 + random.next() * 0.5,
        "transom": if head - stall_riser > 2.1 { 0.42 } else { 0.0 },
        "frameWidth": 0.08,
        "frameDepth": 0.11,
        "setback": 0.16,
        "mullionCap": true,
    });

    let mut back_len = frame.edge_for_role("back").map(|e| e.length).unwrap_or(0.0);
    if back_len == 0.0 || back_len.is_nan() {
        back_len = front_length;
    }
    let back_bays = 1f64.max(js_round(back_len / 6.5));
    let back = if storeys.len() > 1 {
        json!({ "bays": { "count": back_bays }, "ground": "blind", "window": "punched", "endPier": 0.6, "windowSpec": { "sillHeight": 1.1, "widthRatio": 0.42 } })
    } else {
        J::String("blind".into())
    };

    let mut front_elevation = json!({
        "bays": front_plan.bays,
        "endPier": front_plan.end_pier,
        "align": "center",
        "reveal": 0.2,
        "storefrontSpec": storefront_spec,
        "entranceSpec": { "width": 1.85, "transom": 0.42, "frameWidth": 0.08 },
        "rollerSpec": { "inset": 0.14, "hood": 0.4, "slat": 0.5 },
        "windowSpec": { "headClear": 0.45, "widthRatio": 0.58, "frameWidth": 0.07, "frameDepth": 0.08 },
    });
    if random.next() < 0.55 && !revival {
        set(&mut front_elevation, "cornice", json!({ "height": 0.28, "projection": 0.18, "profile": "stepped" }));
    }

    let mut by_edge = Map::new();
    if let (Some(sp), Some(s)) = (&secondary_plan, secondary) {
        let mut e = front_elevation.clone();
        set(&mut e, "bays", J::Array(sp.bays.clone()));
        set(&mut e, "endPier", json!(sp.end_pier));
        by_edge.insert(s.index.to_string(), e);
    }

    let tenancy_count = front_plan.tenancies.len() + secondary_plan.as_ref().map(|p| p.tenancies.len()).unwrap_or(0);
    let mut frontages = vec![];
    if let Some(f) = front {
        frontages.push(json!({ "edgeIndex": f.index, "primary": true, "tenancies": front_plan.tenancies }));
    }
    if let (Some(sp), Some(s)) = (secondary_plan.take(), secondary) {
        frontages.push(json!({ "edgeIndex": s.index, "primary": false, "tenancies": sp.tenancies }));
    }
    let parapet_fascia = parapet > 0.55 && random.next() < 0.58;

    let storeys_json: Vec<J> = storeys.iter().map(|(h, u)| json!({ "height": h, "use": u })).collect();
    json!({
        "id": format!("sf-archetype-{}", c.seed),
        "seed": c.seed,
        "height": jn(c.height),
        "storeys": storeys_json,
        "parapet": if parapet > 0.0 { json!({ "height": parapet, "thickness": 0.28 }) } else { json!(false) },
        "coping": if parapet > 0.0 { json!({ "height": coping, "overhang": 0.07 }) } else { json!(false) },
        "materials": {
            "wall": wall,
            "parapet": wall,
            "recess": wall,
            "trim": "cast-stone-trim",
            "base": "cast-stone",
            "bulkhead": "cast-stone",
            "storefront": joinery,
            "frame": "painted-white",
            "door": joinery,
            "glass": "glass-clear",
            "doorGlass": "glass-door",
            "glassWarm": "glass-door",
            "glassCool": "glass-low-iron",
            "signage": "signage",
            "fascia": fascia,
            "canopy": "painted-steel",
            "awning": "awning-canvas",
            "awningPlain": "canvas-plain",
            "metal": "painted-steel",
            "roller": "painted-steel-light",
            "roof": "asphalt-roof",
            "coping": "cast-stone",
            "tile": "clay-barrel-tile",
            "tenancy0": BULKHEAD_ACCENTS[0],
            "tenancy1": BULKHEAD_ACCENTS[1],
            "tenancy2": BULKHEAD_ACCENTS[2],
            "tenancy3": BULKHEAD_ACCENTS[3],
            "tenancy4": BULKHEAD_ACCENTS[4],
            "tenancy5": BULKHEAD_ACCENTS[5],
        },
        "elevations": {
            "front": front_elevation,
            "front-secondary": { "bays": { "count": 2 }, "ground": "punched", "window": "punched", "endPier": 0.6, "windowSpec": { "widthRatio": 0.5 } },
            "left": "blind",
            "right": "blind",
            "back": back,
            "default": "blind",
            "byEdge": J::Object(by_edge),
        },
        "archetype": {
            "kind": "retail-strip",
            "version": RETAIL_STRIP_VERSION,
            "retail": retail,
            "revival": revival,
            "revivalPaint": revival_paint,
            "wall": wall,
            "fascia": fascia,
            "stallRiser": stall_riser,
            "head": head,
            "tenancyCount": tenancy_count,
            "frontages": frontages,
            "parapetFascia": parapet_fascia,
        },
    })
}

/// `addRetailStripDetail(plan, frame, spec)`.
pub fn detail(plan: &mut Plan, frame: &Frame, spec: &J) {
    let Some(meta) = spec.get_nn("archetype") else { return };
    let Some(ground) = plan.storeys.first().cloned() else { return };
    let lift = if plan.base_y.is_nan() { 0.0 } else { plan.base_y };
    let floor_line = ground.y1 + lift;
    let wall_top = plan.envelope.wall_top + lift;
    let frontages = meta.at("frontages").cloned().unwrap_or(J::Array(vec![]));
    add_with(plan, frame, spec, Some(&frontages));
    let frontages = frontages.as_array().cloned().unwrap_or_default();
    let kind = meta.s("kind").unwrap_or("").to_string();

    for frontage in &frontages {
        let Some(edge) = frame.edges.iter().find(|e| Some(e.index as f64) == frontage.f("edgeIndex")) else { continue };
        let has_shop_tenancies = plan.extra.get("shopGround").map(|g| g.truthy("tenancies")).unwrap_or(false);
        if meta.truthy("parapetFascia") && frontage.truthy("primary") && !has_shop_tenancies {
            let pp = spec.at("parapet");
            if truthy(pp) {
                let ph = pp.num("height");
                if ph > 0.55 {
                    plan.add_box(
                        &edge.basis,
                        PartArgs::new("parapet-fascia", "fascia", 0.1, edge.length - 0.1, wall_top + 0.1, wall_top + ph - 0.08, 0.0, 0.05)
                            .edge(edge.index)
                            .meta(json!({ "archetype": kind })),
                    );
                }
            }
        }
    }
    if meta.truthy("revival") {
        let tints = match meta.get_nn("revivalPaint").filter(|v| truthy(Some(v))) {
            Some(paint) => {
                let mut m = Map::new();
                m.insert(meta.s("wall").unwrap_or("undefined").to_string(), paint.clone());
                J::Object(m)
            }
            None => json!({}),
        };
        plan.extra.insert("tints".into(), tints);
        let roof_top = plan.envelope.roof_top + lift;
        // `plan.coping` is never set by planElevation: the JS falls back to 0.08.
        let coping_h = 0.08;
        for frontage in &frontages {
            let Some(edge) = frame.edges.iter().find(|e| Some(e.index as f64) == frontage.f("edgeIndex")) else { continue };
            let allowance = if edge.allowance.is_finite() { edge.allowance } else { 0.45 };
            let projection = REVIVAL_PENT_PROJECTION.min(allowance - 0.05);
            let y0 = roof_top - coping_h - REVIVAL_PENT_HEIGHT - 0.04;
            if !(projection > 0.15) || !(y0 > floor_line) {
                continue;
            }
            plan.add_profile(
                &edge.basis,
                PartArgs::profile(
                    "cornice",
                    "tile",
                    0.05,
                    edge.length - 0.05,
                    vec![[-0.02, y0], [projection, y0], [projection, y0 + 0.05], [-0.02, y0 + REVIVAL_PENT_HEIGHT]],
                )
                .edge(edge.index)
                .meta(json!({ "archetype": kind, "part": "revival-pent" })),
            );
        }
    }
}
