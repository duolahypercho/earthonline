//! Port of src/world/buildings/archetypes/edwardian-flats.js: SF stacked
//! flats 1906-1924 (spec, porches with one door per flat, full-height bays,
//! cornice supports, window heads, wall-band merge, shop ground, LOD0 budget).
//! `plan.archetype` and `plan.tints` go into `plan.extra`.
//! Not ported: `plan.diagnostics` (partsByKind/partsBySlot/lod0Triangles/
//! budgetShed) and `plan.containment` (checkPlanContainment) - diagnostics
//! only, nothing downstream reads them.

use crate::classify::Classified;
use crate::elevation::{trim_profile_d, PartArgs, Plan, Shape};
use crate::frame::{Basis, Edge, Frame, P2};
use crate::js::*;
use crate::rng::Rng;

pub const EDWARDIAN_FLATS_VERSION: &str = "sf-archetype-edwardian-flats-v2";
const KIND: &str = "edwardian-flats";
pub const EDWARDIAN_FLATS_LOD0_BUDGET: f64 = 3500.0;
const ROOF_PLANT_RESERVE_TRI: f64 = 36.0;

const STACK_M: f64 = 7.4;
const MAX_STACKS: f64 = 4.0;
const END_PIER_M: f64 = 0.3;
const FIRE_ESCAPE_SHARE: f64 = 0.4;
const FIRE_ESCAPE_MIN_SLOT_M: f64 = 1.4;
const GARAGE_STOREY_M: f64 = 2.75;
const FLAT_GROUND_M: f64 = 3.1;
const SHOP_GROUND_M: f64 = 3.75;
const UPPER_STOREY_M: f64 = 3.05;
const FRIEZE_M: f64 = 0.65;
const STOREY_MIN_M: f64 = 2.4;
const PARAPET_MIN: f64 = 0.7;
const PARAPET_MAX: f64 = 1.0;
const COPING_M: f64 = 0.12;
const CORNICE_M: f64 = 0.36;
const CORNICE_PROJECTION_M: f64 = 0.28;
const BAY_PROJECTION_MIN: f64 = 0.6;
const BAY_PROJECTION_MAX: f64 = 0.84;
const BAY_MAX_W: f64 = 4.6;
const BAY_MIN_W: f64 = 1.6;
const PLAIN_BAY_MIN_M: f64 = 1.7;
const BAY_CORNICE_M: f64 = 0.2;
const SIDEWALK_HEADROOM_M: f64 = 2.29;
const AT_GRADE_ALLOWANCE_M: f64 = 0.45;
const UPPER_BAY_PROJECTION_M: f64 = 0.91;
const UPPER_BAY_CORNICE_M: f64 = 0.1;
const BAY_ALLOWANCE_MARGIN_M: f64 = 0.03;
const SASH_FRAME_M: f64 = 0.07;
const SASH_PROUD_M: f64 = 0.045;
const SASH_LIGHT_M: f64 = 1.25;
const SILL_M: f64 = 0.75;
const HEAD_CLEAR_M: f64 = 0.45;
const DOOR_W_MIN: f64 = 0.82;
const DOOR_W_MAX: f64 = 0.92;
const DOOR_GAP_M: f64 = 0.16;
const MAX_DOORS_PER_PORCH: f64 = 3.0;
const PORCH_DEPTH_MIN: f64 = 1.0;
const PORCH_DEPTH_MAX: f64 = 1.45;
const KIT_ENTRANCE_PIERS_M: f64 = 0.4;
const GARAGE_SHARE: f64 = 0.82;
const MIN_FRONTAGE_FOR_GARAGE_M: f64 = 6.0;
const GARAGE_COLUMN_MIN_M: f64 = 2.9;
const BRACKET_PITCH_M: f64 = 1.15;
const MODILLION_PITCH_M: f64 = 0.45;

struct Palette {
    name: &'static str,
    wall: &'static str,
    trim: &'static str,
    weight: f64,
}
const PALETTES: [Palette; 5] = [
    Palette { name: "painted-siding", wall: "wood-siding", trim: "painted-white", weight: 4.0 },
    Palette { name: "white-clapboard", wall: "painted-clapboard", trim: "cast-stone-trim", weight: 2.0 },
    Palette { name: "cream-stucco", wall: "painted-stucco", trim: "painted-white", weight: 2.0 },
    Palette { name: "buff-stucco", wall: "float-finish-stucco", trim: "cast-stone-trim", weight: 1.0 },
    Palette { name: "shingled", wall: "painted-shingle", trim: "cast-stone-trim", weight: 1.0 },
];
const DOOR_FINISHES: [&str; 4] = ["painted-steel", "painted-steel", "signal-red", "painted-steel-mid"];
const GARAGE_FINISHES: [&str; 3] = ["sectional-garage-door", "painted-steel-light", "painted-steel-mid"];
const BAY_SHAPES: [&str; 10] = ["canted", "canted", "canted", "canted", "canted", "square", "square", "square", "curved", "curved"];

fn pick<'a>(random: &mut Rng, list: &[&'a str]) -> &'a str {
    list[(list.len() - 1).min((random.next() * list.len() as f64).floor() as usize)]
}

fn pick_palette(random: &mut Rng) -> &'static Palette {
    let total: f64 = PALETTES.iter().fold(0.0, |a, b| a + b.weight);
    let mut r = random.next() * total;
    for e in PALETTES.iter() {
        r -= e.weight;
        if r < 0.0 {
            return e;
        }
    }
    &PALETTES[PALETTES.len() - 1]
}

fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    lo.max(hi.min(v))
}

struct StoreyStack {
    storeys: Vec<(f64, &'static str)>,
    parapet: f64,
    ground: &'static str,
    upper: f64,
}

fn storey_stack(height: f64, ground: &'static str, parapet_wanted: f64) -> StoreyStack {
    let mut parapet = parapet_wanted;
    if height - parapet - COPING_M < STOREY_MIN_M {
        parapet = 0.2f64.max(height - COPING_M - STOREY_MIN_M);
    }
    let avail = 2.2f64.max(height - parapet - COPING_M);
    let ground_wanted = match ground {
        "garage" => GARAGE_STOREY_M,
        "shop" => SHOP_GROUND_M,
        _ => FLAT_GROUND_M,
    };
    if avail < ground_wanted + STOREY_MIN_M + FRIEZE_M {
        return StoreyStack {
            storeys: vec![(avail, if ground == "shop" { "retail" } else { "residential" })],
            parapet,
            ground: if ground == "garage" { "flat" } else { ground },
            upper: avail,
        };
    }
    let span = avail - ground_wanted - FRIEZE_M;
    let mut count = 1f64.max(js_round(span / UPPER_STOREY_M));
    while count > 1.0 && span / count < STOREY_MIN_M {
        count -= 1.0;
    }
    let upper = span / count;
    let ground_use = match ground {
        "shop" => "retail",
        "garage" => "garage",
        _ => "residential",
    };
    let mut storeys = vec![(ground_wanted, ground_use)];
    let n = count as usize;
    for i in 0..n {
        storeys.push((upper + if i == n - 1 { FRIEZE_M } else { 0.0 }, "residential"));
    }
    StoreyStack { storeys, parapet, ground, upper }
}

#[derive(Clone)]
struct Slot {
    role: &'static str,
    width: f64,
    stack: usize,
}

#[derive(Clone)]
struct Stack {
    mode: &'static str,
    porch: Option<usize>,
    oriel: Option<usize>,
    plain: Vec<usize>,
    doors: f64,
    garage: bool,
}

fn front_layout(front_length: f64, units: f64, door_w: f64, garage: bool, garage_at_start: bool) -> (Vec<Slot>, Vec<Stack>) {
    let available = front_length - 2.0 * END_PIER_M;
    let mut slots: Vec<Slot> = vec![];
    let mut stacks: Vec<Stack> = vec![];
    if available < 1.2 {
        return (slots, stacks);
    }
    if available < 3.2 {
        stacks.push(Stack { mode: "shared", porch: None, oriel: Some(0), plain: vec![], doors: 1.0, garage: false });
        slots.push(Slot { role: "shared", width: available, stack: 0 });
        return (slots, stacks);
    }
    let stack_count = clamp(js_round(available / STACK_M), 1.0, MAX_STACKS);
    let stack_w = available / stack_count;
    let base = (units / stack_count).floor();
    let extra = units % stack_count;
    let porch_for = |n: f64| n * door_w + (n - 1.0) * DOOR_GAP_M + 0.24 + KIT_ENTRANCE_PIERS_M;
    for i in 0..stack_count as usize {
        let mut doors = clamp(base + if (i as f64) < extra { 1.0 } else { 0.0 }, 1.0, MAX_DOORS_PER_PORCH);
        while doors > 1.0 && porch_for(doors) > stack_w * 0.52 {
            doors -= 1.0;
        }
        let porch_w = porch_for(doors);
        let column_w = stack_w - porch_w;
        let mut stack = Stack { mode: "porch", porch: None, oriel: None, plain: vec![], doors, garage: false };
        if column_w < BAY_MIN_W || porch_w > stack_w * 0.6 {
            stack.mode = "shared";
            stack.doors = 1.0;
            stack.oriel = Some(slots.len());
            slots.push(Slot { role: "shared", width: stack_w, stack: i });
            stacks.push(stack);
            continue;
        }
        let mut oriel_w = BAY_MAX_W.min(column_w);
        let mut rest = column_w - oriel_w;
        if rest < PLAIN_BAY_MIN_M {
            oriel_w += rest;
            rest = 0.0;
        }
        let plain_count = if rest > 0.05 { 1f64.max(js_round(rest / 3.2)) } else { 0.0 };
        stack.garage = garage && oriel_w >= GARAGE_COLUMN_MIN_M;
        let porch_first = if i % 2 == 0 { !garage_at_start } else { garage_at_start };
        let order = if porch_first { ["porch", "oriel", "plain"] } else { ["plain", "oriel", "porch"] };
        for role in order {
            match role {
                "porch" => {
                    stack.porch = Some(slots.len());
                    slots.push(Slot { role: "porch", width: porch_w, stack: i });
                }
                "oriel" => {
                    stack.oriel = Some(slots.len());
                    slots.push(Slot { role: "oriel", width: oriel_w, stack: i });
                }
                _ => {
                    for _ in 0..plain_count as usize {
                        stack.plain.push(slots.len());
                        slots.push(Slot { role: "plain", width: rest / plain_count, stack: i });
                    }
                }
            }
        }
        stacks.push(stack);
    }
    let n = slots.len();
    let sum = slots[..n - 1].iter().fold(0.0, |a, s| a + s.width);
    slots[n - 1].width = available - sum;
    (slots, stacks)
}

fn opt_idx(v: Option<usize>) -> J {
    v.map(|x| json!(x)).unwrap_or(J::Null)
}

pub fn spec(c: &Classified, frame: &Frame) -> J {
    let mut random = Rng::from_num(c.seed as f64);
    // civicOf: `classified.units` / `.propertyClass` are never set flat on the record here.
    let civic_units = c.civic.as_ref().and_then(|v| v.units).filter(|u| u.is_finite() && *u > 0.0);
    let property_class = c.civic.as_ref().and_then(|v| v.property_class.clone());
    let palette = pick_palette(&mut random);
    let door_finish = pick(&mut random, &DOOR_FINISHES);
    let garage_finish = pick(&mut random, &GARAGE_FINISHES);
    let bay_shape = pick(&mut random, &BAY_SHAPES);
    let projection = BAY_PROJECTION_MIN + random.next() * (BAY_PROJECTION_MAX - BAY_PROJECTION_MIN);
    let parapet_wanted = PARAPET_MIN + random.next() * (PARAPET_MAX - PARAPET_MIN);
    let cornice_style = if random.next() < 0.55 { "bracketed" } else { "modillion" };
    let porch_style = if random.next() < 0.5 { "arched" } else { "columned" };
    let rusticated_draw = random.next() < 0.45;
    let corbelled = random.next() < 0.5;
    let door_w = DOOR_W_MIN + random.next() * (DOOR_W_MAX - DOOR_W_MIN);
    let porch_depth = PORCH_DEPTH_MIN + random.next() * (PORCH_DEPTH_MAX - PORCH_DEPTH_MIN);

    let front = frame.edge_for_role("front");
    let front_length = {
        let l = front.map(|e| e.length).unwrap_or(0.0);
        if l != 0.0 && !l.is_nan() {
            l
        } else if c.frontage != 0.0 && !c.frontage.is_nan() {
            c.frontage
        } else {
            7.6
        }
    };
    let pc = property_class.clone().unwrap_or_default();
    let retail = c.has_retail_ground || ["FS", "F2", "FS5"].contains(&pc.as_str());
    let mut garage_random = Rng::new(&format!("garage:{}", c.seed));
    let garage_draw = garage_random.next() <= GARAGE_SHARE;
    let garage_at_start = garage_random.next() < 0.5;
    let garage_wanted = !retail && garage_draw && front_length > MIN_FRONTAGE_FOR_GARAGE_M;

    let stack = storey_stack(c.height, if retail { "shop" } else if garage_wanted { "garage" } else { "flat" }, parapet_wanted);
    let storeys = &stack.storeys;
    let parapet = stack.parapet;
    let garage = stack.ground == "garage";
    let rusticated = rusticated_draw && !retail;
    let storey_count = storeys.len();
    let ground_h = storeys[0].0;
    let flats_per_stack = 1f64.max(storey_count as f64 - if garage || retail { 1.0 } else { 0.0 });
    let head = (stack.upper - HEAD_CLEAR_M).min(ground_h - 0.15);
    let stack_guess = 1f64.max(js_round(0.5f64.max(front_length - 2.0 * END_PIER_M) / STACK_M));

    let units = match civic_units {
        Some(u) => js_round(u),
        None => flats_per_stack * stack_guess,
    };
    let (slots, stacks) = front_layout(front_length, units, door_w, garage, garage_at_start);

    let variant = if retail {
        "shop"
    } else if garage {
        "garage"
    } else if corbelled && storey_count > 1 {
        "corbelled"
    } else {
        "grounded"
    };
    let shape = if variant == "grounded" && bay_shape == "curved" { "canted" } else { bay_shape };
    let base_start: usize = if variant == "grounded" { 0 } else { 1 };
    let mouth_h = 3.0f64.min(ground_h - 0.3);

    let bay_starts: Vec<Option<usize>> = stacks
        .iter()
        .map(|s| {
            let bs = if s.mode == "shared" { 1.max(base_start) } else if s.garage || retail { 1 } else { base_start };
            if bs as i64 > storey_count as i64 - 1 {
                None
            } else {
                Some(bs)
            }
        })
        .collect();

    let ground_for = |slot: &Slot, s: &Stack| -> J {
        if slot.role == "porch" || slot.role == "shared" {
            return json!("entrance");
        }
        if slot.role == "plain" {
            return json!(if retail { "storefront" } else { "punched" });
        }
        if s.garage {
            return json!("roller");
        }
        if retail {
            return json!("storefront");
        }
        json!({ "window": "punched", "openingWidth": 0.9f64.max(slot.width * 0.62), "head": head.min(ground_h - 0.75), "mullions": 2 })
    };

    let bays: Vec<J> = slots
        .iter()
        .map(|slot| {
            let s = &stacks[slot.stack];
            let bay_start = bay_starts[slot.stack];
            let is_oriel = slot.role == "oriel" || slot.role == "shared";
            let mut per = Map::new();
            for i in 0..storey_count {
                let v = if is_oriel && bay_start.is_some() && i >= bay_start.unwrap() {
                    json!("blind")
                } else if i == 0 {
                    ground_for(slot, s)
                } else {
                    json!("punched")
                };
                per.insert(i.to_string(), v);
            }
            let mut bay = json!({ "width": slot.width, "storeys": per });
            if slot.role == "porch" {
                set(
                    &mut bay,
                    "entrance",
                    json!({ "width": 99, "height": mouth_h, "recess": porch_depth, "transom": 0, "leaf": "single-solid", "frameWidth": 0.08, "canopy": false }),
                );
                set(&mut bay, "openingWidth", json!(0.6f64.max(0.95f64.min(slot.width - 0.55))));
            } else if slot.role == "shared" {
                set(
                    &mut bay,
                    "entrance",
                    json!({
                        "width": 0.9f64.max(1.1f64.min(slot.width - 0.5)), "height": 2.2f64.min(ground_h - 0.55), "recess": 0.55,
                        "transom": 0.35, "leaf": "single-glass", "frameWidth": 0.08, "canopy": false,
                    }),
                );
            } else if slot.role == "oriel" && s.garage {
                set(&mut bay, "roller", json!({ "width": 2.75f64.min(slot.width - 0.6) }));
            }
            bay
        })
        .collect();

    let fire_roll = Rng::new(&format!("{}|fire-escape", c.seed)).next();
    let unit_count = units;
    let mut fire_escape: Option<J> = None;
    if storey_count >= 3 && unit_count >= 3.0 && fire_roll < FIRE_ESCAPE_SHARE {
        let mut at = END_PIER_M;
        let mut best: Option<(f64, f64, f64)> = None; // centre, off, width
        for slot in &slots {
            let centre = at + slot.width / 2.0;
            if ["porch", "shared", "plain"].contains(&slot.role) && slot.width >= FIRE_ESCAPE_MIN_SLOT_M {
                let off = (centre - front_length / 2.0).abs();
                if best.map(|b| off < b.1).unwrap_or(true) {
                    best = Some((centre, off, slot.width));
                }
            }
            at += slot.width;
        }
        if let Some((centre, _, width)) = best {
            let list: Vec<usize> = (1..storey_count).collect();
            fire_escape = Some(json!({ "storeys": list, "at": centre, "width": 2.4f64.min(width - 0.1), "projection": 1.0 }));
        }
    }

    let pitch_for = |role: &str, base: f64, max_windows: f64| -> f64 {
        let runs: Vec<f64> = frame.edges.iter().filter(|e| e.role == role).map(|e| e.length).collect();
        for pitch in [base, 3.6, 4.5, 6.0, 8.0, 12.0, 24.0] {
            if pitch < base {
                continue;
            }
            let windows = runs.iter().fold(0.0, |a, len| a + 0f64.max(((len - 0.9) / pitch).floor())) * storey_count as f64;
            if windows <= max_windows {
                return pitch;
            }
        }
        24.0
    };
    let secondary_pitch = pitch_for("front-secondary", 3.0, 24.0);
    let back_pitch = pitch_for("back", 3.3, 12.0);

    let detail_on = crate::typology::detail_enabled();
    let cornice_h = if detail_on { 0.52 } else { CORNICE_M };
    let cornice_p = if detail_on { 0.56 } else { CORNICE_PROJECTION_M };
    let ornate = |with_bays: bool| -> J {
        let mut e = json!({
            "bayWidth": if with_bays { 3.0 } else { secondary_pitch },
            "align": "start",
            "endPier": if with_bays { END_PIER_M } else { 0.45 },
            "pierMin": 0.5,
            "reveal": if detail_on { 0.24 } else { 0.16 },
            "window": "punched",
            "ground": if retail { "storefront" } else { "punched" },
            "windowSpec": { "sillHeight": SILL_M, "head": head, "widthRatio": 0.48, "frameWidth": 0.07, "frameDepth": 0.08, "mullions": 0, "transoms": 1 },
            "storefrontSpec": { "stallRiser": 0.45, "headClear": 0.3, "pier": 0.15, "mullionSpacing": 1.25, "transom": 0.5, "frameWidth": 0.08, "frameDepth": 0.1 },
            "rollerSpec": { "width": 2.6, "height": 2.3f64.min(ground_h - 0.4), "inset": 0.14, "hood": 0.3, "slat": 0.5 },
            "sill": { "height": 0.09, "projection": 0.07, "overhang": 0.08, "profile": "sloped" },
            "surround": { "width": 0.12, "projection": 0.04, "slot": "trim" },
            "stringCourses": if (garage || retail) && storey_count > 1 {
                json!([{ "atStorey": 1, "height": if retail { 0.34 } else { 0.2 }, "projection": if retail { 0.16 } else { 0.08 }, "profile": if retail { "ogee" } else { "stepped" }, "slot": "trim" }])
            } else {
                json!([])
            },
            "cornice": { "height": cornice_h, "projection": cornice_p, "profile": "ogee", "slot": "trim" },
            "base": if rusticated || retail { json!(false) } else { json!({ "height": 0.42, "projection": 0.04, "slot": "base" }) },
            "downpipes": false,
        });
        if with_bays && !bays.is_empty() {
            set(&mut e, "bays", json!(bays.clone()));
        }
        if rusticated {
            set(&mut e, "groundWallSlot", json!("base"));
        }
        if with_bays {
            if let Some(fe) = &fire_escape {
                set(&mut e, "fireEscape", fe.clone());
            }
        }
        e
    };

    let stacks_meta: Vec<J> = stacks
        .iter()
        .zip(bay_starts.iter())
        .map(|(s, bs)| {
            json!({
                "mode": s.mode, "porch": opt_idx(s.porch), "oriel": opt_idx(s.oriel), "plain": s.plain,
                "doors": s.doors, "garage": s.garage, "bayStart": opt_idx(*bs),
            })
        })
        .collect();

    // Optional pitched clay-tile / composition-shingle gable roof on compact 2-4 storey hillside
    // Edwardian flats (scale-aware via groundscale; disabled when detail_enabled() is off).
    let gs = eo_data::groundscale::resolve().unwrap_or(1.0).max(0.5);
    let real_front = front.map(|e| e.length).unwrap_or(0.0) / gs;
    let real_area = frame.area / (gs * gs);
    let mut prng = Rng::new(&format!("{}|edw-pitched", c.seed));
    let p_roll = prng.next();
    let p_mat_roll = prng.next();
    let p_pitch = 0.32 + prng.next() * 0.20;
    let pitched_gable = crate::typology::detail_enabled()
        && !retail
        && storeys.len() >= 2
        && storeys.len() <= 4
        && front.is_some()
        && real_front >= 4.8
        && real_front <= 15.5
        && real_area <= 420.0
        && frame.polygon.len() <= 10
        && p_roll < 0.42;
    let gable_rise = if pitched_gable { (0.5 * real_front * p_pitch).clamp(1.2, 3.2) } else { 0.0 };
    let pitched_mat = if p_mat_roll < 0.48 { "clay-barrel-tile" } else { "composition-shingle" };

    let mut out_spec = json!({
        "id": format!("sf-archetype-{}", c.seed),
        "seed": c.seed,
        "height": c.height,
        "storeys": storeys.iter().map(|(h, u)| json!({ "height": h, "use": u })).collect::<Vec<_>>(),
        "wallThickness": 0.3,
        "parapet": { "height": parapet, "thickness": 0.3 },
        "coping": { "height": COPING_M, "overhang": 0.06, "profile": "flat" },
        "materials": {
            "wall": palette.wall,
            "trim": palette.trim,
            "base": "cast-stone",
            "bulkhead": "cast-stone",
            "glass": "glass-clear",
            "doorGlass": "glass-clear",
            "frame": "painted-white",
            "door": door_finish,
            "parapet": palette.wall,
            "coping": "cast-stone",
            "storefront": "painted-steel",
            "recess": palette.wall,
            "roller": garage_finish,
            "metal": "painted-steel",
            "roof": "asphalt-roof",
        },
        "elevations": {
            "front": ornate(true),
            "front-secondary": ornate(false),
            "left": { "window": "blind", "cornice": false },
            "right": { "window": "blind", "cornice": false },
            "back": {
                "window": "punched", "ground": "punched", "bayWidth": back_pitch, "endPier": 0.45,
                "windowSpec": { "sillHeight": 0.85, "head": head, "widthRatio": 0.42, "mullions": 0, "transoms": 0 },
                "sill": { "height": 0.06, "projection": 0.04, "overhang": 0.04, "profile": "flat" },
                "cornice": false, "downpipes": false,
            },
            "default": { "window": "blind", "cornice": false },
        },
        "roof": if pitched_gable { json!({ "slab": 0.3, "plantAllowance": gable_rise + 0.8 }) } else { json!({ "slab": 0.3 }) },
        "archetype": {
            "kind": KIND,
            "version": EDWARDIAN_FLATS_VERSION,
            "variant": variant,
            "palette": palette.name,
            "bayShape": shape,
            "projection": projection,
            "corniceStyle": cornice_style,
            "porchStyle": porch_style,
            "porchDepth": porch_depth,
            "doorW": door_w,
            "rusticated": rusticated,
            "garage": garage,
            "retail": retail,
            "fireEscape": fire_escape.is_some(),
            "units": civic_units.map(jn).unwrap_or(J::Null),
            "unitsSource": if civic_units.is_some() { "roll" } else { "storeys" },
            "head": head,
            "parapet": parapet,
            "corniceHeight": cornice_h,
            "stacks": stacks_meta,
            "pitched": pitched_gable,
            "gableRise": gable_rise,
        },
    });
    if pitched_gable {
        set(obj_mut(&mut out_spec, "materials"), "pitchedRoof", json!(pitched_mat));
    }
    out_spec
}

// ---------------------------------------------------------------------------
// Detail pass.
// ---------------------------------------------------------------------------

struct Face {
    length: f64,
    tangent: [f64; 2],
    basis: Basis,
}

fn face_basis(basis: &Basis, a: [f64; 2], b: [f64; 2]) -> Face {
    let [ox, oz] = basis.origin;
    let [ux, uz] = basis.u;
    let [nx, nz] = basis.n;
    let ds = b[0] - a[0];
    let dw = b[1] - a[1];
    let length = ds.hypot(dw);
    let ts = ds / length;
    let tw = dw / length;
    let os = -tw;
    let ow = ts;
    Face {
        length,
        tangent: [ts, tw],
        basis: Basis {
            origin: [ox + ux * a[0] + nx * a[1], oz + uz * a[0] + nz * a[1]],
            u: [ux * ts + nx * tw, uz * ts + nz * tw],
            n: [ux * os + nx * ow, uz * os + nz * ow],
        },
    }
}

fn bay_outline(shape: &str, s0: f64, s1: f64, p: f64) -> Vec<[f64; 2]> {
    let width = s1 - s0;
    if shape == "curved" {
        let h = width / 2.0;
        let radius = (p * p + h * h) / (2.0 * p);
        let centre_w = p - radius;
        let half = (h / radius).min(1.0).asin();
        let segments = 6;
        let mut out: Vec<[f64; 2]> = (0..=segments)
            .map(|i| {
                let a = -half + (2.0 * half * i as f64) / segments as f64;
                [s0 + h + a.sin() * radius, centre_w + a.cos() * radius]
            })
            .collect();
        out[0] = [s0, 0.0];
        out[segments] = [s1, 0.0];
        return out;
    }
    if shape == "canted" {
        let cant = (width * 0.26).min(p);
        return vec![[s0, 0.0], [s0 + cant, p], [s1 - cant, p], [s1, 0.0]];
    }
    vec![[s0, 0.0], [s0, p], [s1, p], [s1, 0.0]]
}

fn to_world(basis: &Basis, s: f64, w: f64) -> P2 {
    let [ox, oz] = basis.origin;
    let [ux, uz] = basis.u;
    let [nx, nz] = basis.n;
    P2 { x: ox + ux * s + nx * w, z: oz + uz * s + nz * w }
}

#[allow(clippy::too_many_arguments)]
fn add_sash(plan: &mut Plan, fb: &Basis, edge: usize, a: f64, b: f64, y0: f64, y1: f64, w_face: f64, face: &str) {
    let f = SASH_FRAME_M;
    let w0 = w_face - 0.02;
    let w1 = w_face + SASH_PROUD_M;
    let meta = json!({ "archetype": KIND, "face": face });
    let c = |kind: &str, s0: f64, s1: f64, y0: f64, y1: f64, w0: f64, w1: f64| {
        PartArgs::new(kind, "frame", s0, s1, y0, y1, w0, w1).edge(edge).meta(meta.clone()).shadow(false)
    };
    plan.add_box(fb, c("bay-sash", a, a + f, y0, y1, w0, w1));
    plan.add_box(fb, c("bay-sash", b - f, b, y0, y1, w0, w1));
    plan.add_box(fb, c("bay-sash", a, b, y1 - f, y1, w0, w1));
    plan.add_box(fb, c("bay-sash", a, b, y0, y0 + f, w0, w1));
    let lights = 1f64.max(js_round((b - a) / SASH_LIGHT_M));
    for i in 1..lights as usize {
        let s = a + ((b - a) * i as f64) / lights;
        plan.add_box(fb, c("bay-sash", s - 0.05, s + 0.05, y0, y1, w0, w1));
    }
    let mid = (y0 + y1) / 2.0 + 0.08;
    plan.add_box(fb, c("mullion", a, b, mid - 0.03, mid + 0.03, w0 + 0.01, w1 - 0.01));
    if crate::typology::detail_enabled() {
        plan.add_box(fb, PartArgs::new("sill", "trim", a - 0.05, b + 0.05, y0 - 0.075, y0 + 0.01, w_face - 0.01, w1 + 0.045).edge(edge).meta(meta.clone()));
        plan.add_box(fb, PartArgs::new("window-head", "trim", a - 0.05, b + 0.05, y1 + 0.01, y1 + 0.11, w_face - 0.01, w1 + 0.045).edge(edge).meta(meta.clone()));
    }
    plan.add_box(
        fb,
        PartArgs::new("glass", "glass", a + f * 0.5, b - f * 0.5, y0 + f * 0.5, y1 - f * 0.5, w_face + 0.004, w_face + 0.014)
            .edge(edge)
            .meta(meta.clone())
            .shadow(false),
    );
}

#[allow(clippy::too_many_arguments)]
fn add_cornice_supports(plan: &mut Plan, basis: &Basis, edge: usize, a: f64, b: f64, cornice_bottom: f64, style: &str, depth: f64, where_: &str) {
    let length = b - a;
    if length < 0.3 || depth < 0.06 {
        return;
    }
    let meta = json!({ "archetype": KIND, "where": where_ });
    if style == "bracketed" {
        let count = 1f64.max(js_round(length / BRACKET_PITCH_M));
        let pitch = length / count;
        let cast = crate::typology::detail_enabled() && where_ == "front";
        for i in 0..=count as usize {
            let s = clamp(a + i as f64 * pitch, a + 0.09, b - 0.09);
            plan.add_box(
                basis,
                PartArgs::new("cornice-bracket", "trim", s - 0.075, s + 0.075, cornice_bottom - 0.4, cornice_bottom + 0.01, -0.01, depth)
                    .edge(edge)
                    .meta(meta.clone())
                    .shadow(cast),
            );
        }
        return;
    }
    let count = 1f64.max((length / MODILLION_PITCH_M).floor());
    let pitch = length / count;
    for i in 0..count as usize {
        let s = a + (i as f64 + 0.5) * pitch;
        plan.add_box(
            basis,
            PartArgs::new("dentil", "trim", s - 0.065, s + 0.065, cornice_bottom - 0.14, cornice_bottom, -0.01, depth * 0.85)
                .edge(edge)
                .meta(meta.clone())
                .shadow(false),
        );
    }
}

/// `edwardianPartTriangles(part)`: 12 per box, 4N-4 per profile, prism and plate.
pub fn edwardian_part_triangles(part: &crate::elevation::Part) -> f64 {
    match &part.shape {
        Shape::Prism(points) => {
            let n = if points.is_empty() { 4.0 } else { points.len() as f64 };
            if !part.side_faces {
                2.0 * (n - 2.0)
            } else {
                4f64.max(4.0 * n - 4.0)
            }
        }
        Shape::Profile(p) | Shape::Plate(p) => {
            let n = if p.is_empty() { 4.0 } else { p.len() as f64 };
            4f64.max(4.0 * n - 4.0)
        }
        Shape::Box => 12.0,
    }
}

fn plan_triangles(parts: &[crate::elevation::Part]) -> f64 {
    parts.iter().map(edwardian_part_triangles).sum()
}

const SHED_ORDER: [&str; 19] = [
    "rear-joinery",
    "secondary-supports",
    "bay-aprons",
    "surrounds",
    "secondary-heads",
    "secondary-joinery",
    "cheek-rails",
    "supports",
    "transoms",
    "cheek-sashes",
    "off-front-cornices",
    "rails",
    "garage-slats",
    "door-casings",
    "bay-sashes",
    "heads-and-courses",
    "front-frames",
    "front-sills",
    "bay-caps",
];

fn shed_test(name: &str, p: &crate::elevation::Part, role: Option<&str>) -> bool {
    let k = p.kind.as_str();
    let meta_is = |key: &str, v: &str| p.meta.s(key) == Some(v);
    match name {
        "rear-joinery" => {
            ["frame", "transom", "sill", "jamb", "window-head"].contains(&k) && p.edge.is_some() && !matches!(role, Some("front") | Some("front-secondary"))
        }
        "secondary-supports" => (k == "cornice-bracket" || k == "dentil") && meta_is("where", "secondary"),
        "bay-aprons" => k == "bay-sill",
        "surrounds" => k == "jamb",
        "secondary-heads" => k == "window-head" && meta_is("where", "secondary"),
        "secondary-joinery" => (k == "frame" || k == "transom" || k == "sill") && role == Some("front-secondary"),
        "cheek-rails" => k == "mullion" && meta_is("face", "cheek"),
        "supports" => k == "cornice-bracket" || k == "dentil" || k == "corbel",
        "transoms" => k == "transom",
        "cheek-sashes" => k == "bay-sash" && meta_is("face", "cheek"),
        "off-front-cornices" => k == "cornice" && p.edge.is_some() && role != Some("front"),
        "rails" => k == "mullion",
        "garage-slats" => k == "roller-slat",
        "door-casings" => k == "door-frame" && meta_is("part", "door"),
        "bay-sashes" => k == "bay-sash",
        "heads-and-courses" => k == "window-head" || k == "string-course" || k == "base",
        "front-frames" => k == "frame",
        "front-sills" => k == "sill",
        "bay-caps" => k == "bay-cap",
        _ => false,
    }
}

fn enforce_budget(plan: &mut Plan, frame: &Frame) -> (f64, Vec<&'static str>) {
    let mut shed = vec![];
    let role = |p: &crate::elevation::Part| -> Option<&'static str> { p.edge.and_then(|e| frame.edges.get(e)).map(|e| e.role) };
    let mut triangles = plan_triangles(&plan.parts);
    let budget = if crate::typology::detail_enabled() { 5200.0 } else { EDWARDIAN_FLATS_LOD0_BUDGET };
    let target = budget - ROOF_PLANT_RESERVE_TRI;
    for name in SHED_ORDER {
        if triangles <= target {
            break;
        }
        let before = plan.parts.len();
        plan.parts.retain(|p| p.meta.truthy("shopJoinery") || !shed_test(name, p, role(p)));
        if plan.parts.len() == before {
            continue;
        }
        shed.push(name);
        triangles = plan_triangles(&plan.parts);
    }
    (triangles, shed)
}

struct StackMeta {
    porch: Option<i64>,
    oriel: Option<i64>,
    doors: f64,
    bay_start: Option<usize>,
}

struct Meta {
    door_w: f64,
    porch_depth: f64,
    porch_style: String,
    bay_shape: String,
    projection: f64,
    parapet: f64,
    variant: String,
    cornice_style: String,
    head: f64,
}

fn add_porch(plan: &mut Plan, front: &Edge, rec: usize, stack: &StackMeta, meta: &Meta) {
    let porch = stack.porch.unwrap();
    let Some(op) = plan.edges[rec].openings.iter().find(|o| o.type_ == "entrance" && o.bay as i64 == porch && o.storey == 0).cloned() else {
        return;
    };
    let Some(ai) = plan.apertures.iter().position(|a| {
        a.f("edge") == Some(front.index as f64) && a.f("bay") == Some(porch as f64) && a.f("storey") == Some(0.0)
    }) else {
        return;
    };
    let aperture = plan.apertures[ai].clone();
    let basis = front.basis;
    let fi = front.index;
    plan.parts.retain(|p| {
        !(["door-frame", "door-leaf", "transom-glass"].contains(&p.kind.as_str()) && p.edge == Some(fi) && p.storey == Some(0) && p.bay == Some(porch))
    });
    plan.apertures.remove(ai);

    // The kit's aperture record carries `recess = max(0.05, entrance.recess)`;
    // the port's record leaves it out, so it is recomputed from the bay's spec.
    let recess = aperture.f("recess").unwrap_or(0.05f64.max(meta.porch_depth));
    let w_back = -recess;
    let y0 = op.y0;
    let y1 = op.y1;
    let doors = 1f64.max(stack.doors);
    let door_w = meta.door_w.min((op.s1 - op.s0 - 0.2 - (doors - 1.0) * DOOR_GAP_M) / doors);
    let door_h = 2.15f64.min(y1 - y0 - 0.35);
    let total = doors * door_w + (doors - 1.0) * DOOR_GAP_M;
    let start = (op.s0 + op.s1) / 2.0 - total / 2.0;
    let wall_meta = json!({ "archetype": KIND, "part": "porch-wall" });
    let wall = |s0: f64, s1: f64, y0: f64, y1: f64| PartArgs::new("recess", "recess", s0, s1, y0, y1, w_back - 0.12, w_back).edge(fi).meta(wall_meta.clone());
    let mut cursor = op.s0 - 0.12;
    let nd = doors as usize;
    for i in 0..nd {
        let d0 = start + i as f64 * (door_w + DOOR_GAP_M);
        plan.add_box(&basis, wall(cursor, d0, y0 - 0.05, y0 + door_h));
        cursor = d0 + door_w;
    }
    plan.add_box(&basis, wall(cursor, op.s1 + 0.12, y0 - 0.05, y0 + door_h));
    plan.add_box(&basis, wall(op.s0 - 0.12, op.s1 + 0.12, y0 + door_h, y1 + 0.12));

    let transom_h = 0.42f64.min(y1 - y0 - door_h - 0.2);
    let [ox, oz] = basis.origin;
    let ap_id = aperture.s("id").unwrap_or("").to_string();
    for i in 0..nd {
        let d0 = start + i as f64 * (door_w + DOOR_GAP_M);
        let d1 = d0 + door_w;
        let dm = json!({ "archetype": KIND, "part": "door", "door": i });
        let casing = |s0: f64, s1: f64, y0: f64, y1: f64, w0: f64, w1: f64| {
            PartArgs::new("door-frame", "trim", s0, s1, y0, y1, w0, w1).edge(fi).meta(dm.clone()).shadow(false)
        };
        let top = y0 + door_h + if transom_h > 0.15 { transom_h + 0.06 } else { 0.0 };
        plan.add_box(&basis, casing(d0 - 0.07, d0, y0, top, w_back - 0.02, w_back + 0.04));
        plan.add_box(&basis, casing(d1, d1 + 0.07, y0, top, w_back - 0.02, w_back + 0.04));
        plan.add_box(&basis, casing(d0 - 0.1, d1 + 0.1, top, top + 0.1, w_back - 0.02, w_back + 0.05));
        let mut leaf_meta = dm.clone();
        set(&mut leaf_meta, "part", json!("door-leaf"));
        plan.add_box(&basis, PartArgs::new("door-frame", "door", d0, d1, y0 + 0.01, y0 + door_h, w_back - 0.08, w_back - 0.04).edge(fi).meta(leaf_meta));
        plan.add_box(
            &basis,
            PartArgs::new("glass", "doorGlass", d0 + 0.13, d1 - 0.13, y0 + door_h * 0.55, y0 + door_h - 0.14, w_back - 0.04, w_back - 0.03)
                .edge(fi)
                .meta(dm.clone())
                .shadow(false),
        );
        if transom_h > 0.15 {
            plan.add_box(
                &basis,
                PartArgs::new("glass", "glass", d0, d1, y0 + door_h + 0.05, y0 + door_h + 0.05 + transom_h, w_back + 0.004, w_back + 0.012)
                    .edge(fi)
                    .meta(dm.clone())
                    .shadow(false),
            );
        }
        let sc = (d0 + d1) / 2.0;
        plan.apertures.push(json!({
            "id": format!("{}:door{}", ap_id, i),
            "edge": fi, "storey": 0, "bay": porch,
            "s0": d0, "s1": d1, "width": door_w, "height": door_h, "recess": recess, "leaves": 1, "y0": y0, "static": true,
            "world": {
                "threshold": [ox + basis.u[0] * sc, y0, oz + basis.u[1] * sc],
                "leaf": [ox + basis.u[0] * sc + basis.n[0] * w_back, y0, oz + basis.u[1] * sc + basis.n[1] * w_back],
                "outward": [basis.n[0], basis.n[1]],
                "along": [basis.u[0], basis.u[1]],
            },
        }));
    }

    let mouth = json!({ "archetype": KIND, "part": "porch-mouth" });
    if meta.porch_style == "arched" {
        let width = op.s1 - op.s0;
        let apex = y1 - 0.06;
        let spring = apex - 0.5f64.min(width * 0.22);
        let n = 8;
        let mut pts: Vec<[f64; 2]> = (0..=n)
            .map(|i| {
                let t = i as f64 / n as f64;
                [op.s0 + width * t, spring + (apex - spring) * (std::f64::consts::PI * t).sin()]
            })
            .collect();
        pts.push([op.s1, y1]);
        pts.push([op.s0, y1]);
        plan.add_plate(
            &basis,
            PartArgs { kind: "wall".into(), slot: "wall".into(), edge: Some(fi), profile: pts, w0: -0.3, w1: 0.0, meta: Some(json!({ "archetype": KIND, "part": "porch-arch" })), ..Default::default() },
        );
        let mid = op.s0 + width / 2.0;
        plan.add_box(&basis, PartArgs::new("window-head", "trim", mid - 0.13, mid + 0.13, apex - 0.14, y1 + 0.08, -0.02, 0.07).edge(fi).meta(mouth.clone()));
    } else {
        for (a, b) in [(op.s0 - 0.2, op.s0), (op.s1, op.s1 + 0.2)] {
            plan.add_box(&basis, PartArgs::new("pilaster", "trim", a, b, y0, y1, -0.01, 0.11).edge(fi).meta(mouth.clone()));
        }
        plan.add_box(&basis, PartArgs::new("window-head", "trim", op.s0 - 0.28, op.s1 + 0.28, y1, y1 + 0.26, -0.01, 0.14).edge(fi).meta(mouth.clone()));
    }
    if crate::typology::detail_enabled() && recess >= 0.65 && (op.s1 - op.s0) >= 0.85 {
        // Hanging wrought-iron vestibule porch lantern inside the arched/columned Edwardian porch mouth
        let sm = (op.s0 + op.s1) * 0.5;
        let top_ref = if meta.porch_style == "arched" {
            (y1 - 0.06) - 0.5f64.min((op.s1 - op.s0) * 0.22) + 0.18
        } else {
            y1 - 0.08
        };
        let ly1 = top_ref.min(y1 - 0.08);
        let ly0 = ly1 - 0.30;
        if ly0 > y0 + 1.55 {
            let lw0 = -0.44;
            let lw1 = -0.32;
            let lm = json!({ "archetype": KIND, "part": "porch-lantern" });
            plan.add_box(&basis, PartArgs::new("porch-lantern", "metal", sm - 0.02, sm + 0.02, ly1, y1, lw0 + 0.04, lw1 - 0.04).edge(fi).meta(lm.clone()));
            plan.add_box(&basis, PartArgs::new("porch-lantern", "metal", sm - 0.15, sm + 0.15, ly1, ly1 + 0.04, lw0, lw1).edge(fi).meta(lm.clone()));
            plan.add_box(&basis, PartArgs::new("porch-lantern", "metal", sm - 0.14, sm + 0.14, ly0 - 0.03, ly0 + 0.01, lw0, lw1).edge(fi).meta(lm.clone()));
            plan.add_box(&basis, PartArgs::new("porch-lantern", "metal", sm - 0.15, sm - 0.125, ly0, ly1, lw0, lw1).edge(fi).meta(lm.clone()));
            plan.add_box(&basis, PartArgs::new("porch-lantern", "metal", sm + 0.125, sm + 0.15, ly0, ly1, lw0, lw1).edge(fi).meta(lm.clone()));
            plan.add_box(
                &basis,
                PartArgs::new("glass", "glass", sm - 0.135, sm + 0.135, ly0, ly1, lw1 - 0.025, lw1 - 0.013)
                    .edge(fi)
                    .meta(json!({ "archetype": KIND, "porchLantern": true }))
                    .shadow(false),
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn add_bay(plan: &mut Plan, front: &Edge, rec: usize, stack: &StackMeta, meta: &Meta, base_y: f64, wall_top: f64, cornice_bottom: f64) -> Option<[f64; 2]> {
    let oriel = stack.oriel?;
    let bay_rec = if oriel >= 0 { plan.edges[rec].bays.get(oriel as usize).cloned() } else { None };
    let bay_rec = bay_rec?;
    let bay_start = stack.bay_start?;
    let inset = 0.14f64.min(bay_rec.width * 0.05);
    let s0 = bay_rec.s0 + inset;
    let s1 = bay_rec.s1 - inset;
    if s1 - s0 < 1.2 {
        return None;
    }
    let allow = front.allowance;
    if !(allow >= 0.2) {
        return None;
    }
    let bay_y0 = plan.storeys[bay_start].y0 + base_y;
    let bay_y1 = wall_top + meta.parapet;
    let low = bay_y0 - base_y < SIDEWALK_HEADROOM_M;
    let corn_p = if low { BAY_CORNICE_M.min(allow * 0.3) } else { UPPER_BAY_CORNICE_M.min(allow * 0.3) };
    let body_limit = if low { allow.min(AT_GRADE_ALLOWANCE_M) } else { allow };
    let curved_margin = if meta.bay_shape == "curved" { 0.1 } else { 0.0 };
    let want = if low { meta.projection } else { meta.projection.max(UPPER_BAY_PROJECTION_M) };
    let trim_margin = if low { 0.05 } else { BAY_ALLOWANCE_MARGIN_M };
    let p = clamp(want, 0.12, (body_limit - SASH_PROUD_M - 0.03 - curved_margin).min(allow - corn_p - trim_margin));
    let basis = front.basis;
    let edge = front.index;
    let outline = bay_outline(&meta.bay_shape, s0, s1, p);
    plan.add_prism(PartArgs {
        kind: "bay".into(),
        slot: "wall".into(),
        points: outline.iter().map(|[s, w]| to_world(&basis, *s, *w)).collect(),
        y0: bay_y0,
        y1: bay_y1,
        cast_shadow: Some(true),
        receive_shadow: Some(true),
        meta: Some(json!({ "archetype": KIND, "shape": meta.bay_shape, "edge": edge })),
        ..Default::default()
    });
    let mid = (s0 + s1) / 2.0;
    plan.add_prism(PartArgs {
        kind: "bay-cap".into(),
        slot: "coping".into(),
        points: outline
            .iter()
            .map(|[s, w]| to_world(&basis, s + if *s < mid { -0.04 } else { 0.04 }, if *w > 0.0 { w + 0.04 } else { 0.0 }))
            .collect(),
        y0: bay_y1,
        y1: bay_y1 + COPING_M,
        cast_shadow: Some(false),
        meta: Some(json!({ "archetype": KIND })),
        ..Default::default()
    });

    let mut faces: Vec<Face> = vec![];
    for i in 0..outline.len().saturating_sub(1) {
        let f = face_basis(&basis, outline[i], outline[i + 1]);
        if f.length > 0.05 {
            faces.push(f);
        }
    }
    for f in &faces {
        plan.add_profile(
            &f.basis,
            PartArgs::profile(
                "cornice",
                "trim",
                -0.02,
                f.length + 0.02,
                trim_profile_d(if meta.bay_shape == "curved" { "cove" } else { "ogee" }, corn_p, CORNICE_M, cornice_bottom),
            )
            .edge(edge)
            .meta(json!({ "archetype": KIND, "part": "bay-cornice" })),
        );
        add_cornice_supports(plan, &f.basis, edge, 0.02, f.length - 0.02, cornice_bottom, &meta.cornice_style, 0.18f64.min(corn_p * 0.9), "front");
        if bay_y0 - 0.18 - base_y >= SIDEWALK_HEADROOM_M {
            plan.add_profile(
                &f.basis,
                PartArgs::profile("bay-sill", "trim", -0.02, f.length + 0.02, trim_profile_d("stepped", 0.04f64.max(0.1f64.min(allow - p - 0.05)), 0.18, bay_y0 - 0.18))
                    .edge(edge)
                    .meta(json!({ "archetype": KIND })),
            );
        }
    }
    if meta.variant == "corbelled" && bay_start == 1 {
        let corbel_bottom = bay_y0 - 0.58;
        let depth = (p * 0.85).min(if corbel_bottom - base_y < SIDEWALK_HEADROOM_M { AT_GRADE_ALLOWANCE_M - 0.03 } else { p });
        for s in [s0 + 0.18, mid, s1 - 0.18] {
            plan.add_box(&basis, PartArgs::new("corbel", "trim", s - 0.09, s + 0.09, corbel_bottom, bay_y0 - 0.18, -0.01, depth).edge(edge).meta(json!({ "archetype": KIND })));
        }
    }

    for i in bay_start..plan.storeys.len() {
        let wy0 = plan.storeys[i].y0 + base_y + SILL_M;
        let wy1 = (plan.storeys[i].y0 + base_y + meta.head).min(cornice_bottom - 0.5);
        if wy1 - wy0 < 0.9 {
            continue;
        }
        if meta.bay_shape == "curved" {
            let mut k = 0;
            while k + 2 < outline.len() {
                let a = outline[k];
                let m = outline[k + 1];
                let f = face_basis(&basis, a, outline[k + 2]);
                let [ts, tw] = f.tangent;
                let sag = (m[0] - a[0]) * -tw + (m[1] - a[1]) * ts;
                let margin = 0.1f64.min(f.length * 0.1);
                if f.length - 2.0 * margin >= 0.45 {
                    add_sash(plan, &f.basis, edge, margin, f.length - margin, wy0, wy1, 0f64.max(sag) + 0.005, if k == 2 { "face" } else { "cheek" });
                }
                k += 2;
            }
            continue;
        }
        for (index, f) in faces.iter().enumerate() {
            let margin = 0.12f64.min(f.length * 0.1);
            if f.length - 2.0 * margin < 0.45 {
                continue;
            }
            add_sash(plan, &f.basis, edge, margin, f.length - margin, wy0, wy1, 0.0, if index == 1 { "face" } else { "cheek" });
        }
    }
    Some([s0 - 0.12, s1 + 0.12])
}

fn opt_i(v: Option<&J>) -> Option<i64> {
    v.and_then(|x| x.as_f64()).map(|x| x as i64)
}

pub fn detail(plan: &mut Plan, frame: &Frame, spec: &J) {
    let Some(m) = spec.get_nn("archetype") else { return };
    let meta = Meta {
        door_w: m.num("doorW"),
        porch_depth: m.num("porchDepth"),
        porch_style: m.s("porchStyle").unwrap_or("").into(),
        bay_shape: m.s("bayShape").unwrap_or("").into(),
        projection: m.num("projection"),
        parapet: m.num("parapet"),
        variant: m.s("variant").unwrap_or("").into(),
        cornice_style: m.s("corniceStyle").unwrap_or("").into(),
        head: m.num("head"),
    };
    let stacks: Vec<StackMeta> = m
        .arr("stacks")
        .map(|a| {
            a.iter()
                .map(|s| StackMeta {
                    porch: opt_i(s.get_nn("porch")),
                    oriel: opt_i(s.get_nn("oriel")),
                    doors: s.num("doors"),
                    bay_start: opt_i(s.get_nn("bayStart")).map(|v| v as usize),
                })
                .collect()
        })
        .unwrap_or_default();
    let front = frame.edge_for_role("front");
    let rec = front.and_then(|f| plan.edges.iter().position(|e| e.index == f.index));
    let base_y = if plan.base_y.is_finite() { plan.base_y } else { 0.0 };
    let wall_top = plan.envelope.wall_top + base_y;
    let cornice_bottom = wall_top - m.num("corniceHeight");

    if let (Some(front), Some(rec)) = (front, rec) {
        let mut covered: Vec<[f64; 2]> = vec![];
        for stack in &stacks {
            if stack.porch.is_some() {
                add_porch(plan, front, rec, stack, &meta);
            }
            if stack.oriel.is_some() {
                if let Some(span) = add_bay(plan, front, rec, stack, &meta, base_y, wall_top, cornice_bottom) {
                    covered.push(span);
                }
            }
        }
        let detail_on = crate::typology::detail_enabled();
        let depth = if detail_on { 0.44f64.min((front.allowance - 0.02).max(0.24)) } else { 0.24f64.min(front.allowance - 0.02) };
        let mut cursor = 0.12;
        covered.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap_or(std::cmp::Ordering::Equal));
        covered.push([front.length - 0.12, f64::INFINITY]);
        for [a, b] in covered {
            if a - cursor > 0.3 {
                add_cornice_supports(plan, &front.basis, front.index, cursor, a, cornice_bottom, &meta.cornice_style, depth, "front");
            }
            cursor = f64::max(cursor, b);
        }
        let ops: Vec<(f64, f64, f64)> = plan.edges[rec]
            .openings
            .iter()
            .filter(|op| op.type_ == "punched" && !(op.storey == 0 && meta.variant == "corbelled"))
            .map(|op| (op.s0, op.s1, op.y1))
            .collect();
        let head_p = if detail_on { 0.18f64.min((front.allowance - 0.02).max(0.11)) } else { 0.11 };
        let head_h = if detail_on { 0.19 } else { 0.15 };
        for (s0, s1, y1) in ops {
            plan.add_profile(
                &front.basis,
                PartArgs::profile("window-head", "trim", s0 - 0.2, s1 + 0.2, trim_profile_d("cove", head_p, head_h, y1 + 0.12))
                    .edge(front.index)
                    .shadow(detail_on)
                    .meta(json!({ "archetype": KIND, "where": "front" })),
            );
        }
    }

    for edge in frame.edges_for_role("front-secondary") {
        if edge.party || edge.length < 1.2 {
            continue;
        }
        let Some(r) = plan.edges.iter().position(|e| e.index == edge.index) else { continue };
        let detail_on = crate::typology::detail_enabled();
        let s_depth = if detail_on { 0.38f64.min((edge.allowance - 0.02).max(0.24)) } else { 0.24f64.min(edge.allowance - 0.02) };
        add_cornice_supports(plan, &edge.basis, edge.index, 0.12, edge.length - 0.12, cornice_bottom, &meta.cornice_style, s_depth, "secondary");
        let ops: Vec<(f64, f64, f64)> = plan.edges[r].openings.iter().filter(|op| op.type_ == "punched").map(|op| (op.s0, op.s1, op.y1)).collect();
        let head_p = if detail_on { 0.16f64.min((edge.allowance - 0.02).max(0.11)) } else { 0.11 };
        let head_h = if detail_on { 0.18 } else { 0.15 };
        for (s0, s1, y1) in ops {
            plan.add_profile(
                &edge.basis,
                PartArgs::profile("window-head", "trim", s0 - 0.2, s1 + 0.2, trim_profile_d("cove", head_p, head_h, y1 + 0.12))
                    .edge(edge.index)
                    .meta(json!({ "archetype": KIND, "where": "secondary" })),
            );
        }
    }

    let mut pitched_added = false;
    if crate::typology::detail_enabled() {
        // Optional pitched gable roof + masonry chimney on compact hillside Edwardian flats
        if m.truthy("pitched") {
            let rise = m.num("gableRise");
            if rise > 0.8 {
                if let Some(front) = front {
                    let y0 = plan.envelope.wall_top + base_y;
                    let basis = front.basis;
                    let l = front.length;
                    let [ox, oz] = basis.origin;
                    let [ux, uz] = basis.u;
                    let [nx, nz] = basis.n;
                    let max_d = frame.polygon.iter().map(|p| -((p.x - ox) * nx + (p.z - oz) * nz)).fold(0.0f64, f64::max);
                    let mut depth = 0.0;
                    for step in (0..=16).rev() {
                        let d = 2.8 + (max_d - 2.8).max(0.0) * (step as f64 / 16.0);
                        let mut ok = true;
                        for frac in [0.08, 0.50, 0.92] {
                            let s = l * frac;
                            let wx = ox + ux * s - nx * d;
                            let wz = oz + uz * s - nz * d;
                            if frame.signed_distance(wx, wz) > 0.05 {
                                ok = false;
                                break;
                            }
                        }
                        if ok {
                            depth = d;
                            break;
                        }
                    }
                    if depth > 2.6 {
                        plan.add_plate(
                            &basis,
                            PartArgs {
                                kind: "roof-gable".into(),
                                slot: "pitchedRoof".into(),
                                edge: Some(front.index),
                                profile: vec![[-0.14, y0 - 0.08], [l + 0.14, y0 - 0.08], [l * 0.5, y0 + rise]],
                                w0: -depth + 0.22,
                                w1: -0.06,
                                cast_shadow: Some(true),
                                receive_shadow: Some(true),
                                meta: Some(json!({ "archetype": KIND, "part": "gable-roof", "roof": true })),
                                ..Default::default()
                            },
                        );
                        plan.add_plate(
                            &basis,
                            PartArgs {
                                kind: "gable".into(),
                                slot: "wall".into(),
                                edge: Some(front.index),
                                profile: vec![[0.05, y0], [l - 0.05, y0], [l * 0.5, y0 + rise - 0.08]],
                                w0: -0.14,
                                w1: 0.06,
                                cast_shadow: Some(true),
                                receive_shadow: Some(true),
                                meta: Some(json!({ "archetype": KIND, "part": "gable-end", "roof": true })),
                                ..Default::default()
                            },
                        );
                        let ch_w = -depth * 0.45;
                        let ch_s = l * 0.26;
                        let wx = ox + ux * ch_s + nx * ch_w;
                        let wz = oz + uz * ch_s + nz * ch_w;
                        if frame.signed_distance(wx, wz) < -0.55 {
                            plan.add_box(
                                &basis,
                                PartArgs::new("chimney", "trim", ch_s - 0.34, ch_s + 0.34, y0, y0 + rise + 0.45, ch_w - 0.28, ch_w + 0.28)
                                    .edge(front.index)
                                    .shadow(true)
                                    .receive(true)
                                    .meta(json!({ "archetype": KIND, "part": "chimney", "roof": true })),
                            );
                        }
                        pitched_added = true;
                    }
                }
            }
        }
        // Cast-stone water-table plinth belt along street edges to ground walls cleanly onto hillside skirts
        for edge in frame.edges.iter() {
            if edge.party || !(edge.role == "front" || edge.role == "front-secondary") || edge.length < 2.0 {
                continue;
            }
            let Some(r) = plan.edges.iter().position(|e| e.index == edge.index) else { continue };
            let mut cuts: Vec<(f64, f64)> = plan.edges[r]
                .openings
                .iter()
                .filter(|o| o.storey == 0 && matches!(o.type_.as_str(), "entrance" | "roller" | "storefront"))
                .map(|o| (o.s0 - 0.06, o.s1 + 0.06))
                .collect();
            cuts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            cuts.push((edge.length, edge.length));
            let wp = 0.065f64.min(0.03f64.max(edge.allowance - 0.03));
            let mut cur = 0.0;
            for (a, b) in cuts {
                let end = a.min(edge.length);
                if end - cur > 0.3 {
                    plan.add_box(
                        &edge.basis,
                        PartArgs::new("water-table", "base", cur, end, base_y - 0.02, base_y + 0.22, -0.01, wp)
                            .edge(edge.index)
                            .meta(json!({ "archetype": KIND, "part": "water-table" })),
                    );
                }
                cur = cur.max(b);
            }
        }
    }

    let parts = std::mem::take(&mut plan.parts);
    plan.parts = crate::lod::collapse_wall_bands(parts);
    crate::shop_ground::add(plan, frame, spec);
    let (triangles, _shed) = enforce_budget(plan, frame);

    let mut garages: Vec<J> = vec![];
    if let (Some(front), Some(rec)) = (front, rec) {
        let [ox, oz] = front.basis.origin;
        let [ux, uz] = front.basis.u;
        let [nx, nz] = front.basis.n;
        for op in &plan.edges[rec].openings {
            if op.type_ != "roller" || op.storey != 0 {
                continue;
            }
            let centre = (op.s0 + op.s1) / 2.0;
            garages.push(json!({ "edge": front.index, "width": op.s1 - op.s0, "world": { "x": ox + ux * centre, "z": oz + uz * centre }, "outward": [nx, nz] }));
        }
    }
    let first = garages.first().cloned().unwrap_or(J::Null);
    plan.extra.insert(
        "archetype".into(),
        json!({
            "kind": KIND,
            "version": m.at("version").cloned().unwrap_or(J::Null),
            "variant": m.at("variant").cloned().unwrap_or(J::Null),
            "retail": m.at("retail").cloned().unwrap_or(J::Null),
            "bayShape": m.at("bayShape").cloned().unwrap_or(J::Null),
            "garage": first,
            "garages": garages,
            "pitched": pitched_added,
            "roofPlantTriangles": ROOF_PLANT_RESERVE_TRI.max(EDWARDIAN_FLATS_LOD0_BUDGET - triangles),
        }),
    );
    let tints = super::period_paint::period_paint_tints(&plan.seed, plan.material("wall")).unwrap_or_else(|| json!({}));
    plan.extra.insert("tints".into(), tints);
}
