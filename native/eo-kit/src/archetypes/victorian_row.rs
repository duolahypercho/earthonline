//! Port of src/world/buildings/archetypes/victorian-row.js: the SF Victorian
//! row house (spec, projecting bay, stoop, bracketed cornice, window heads,
//! period paint). `plan.tints` goes into `plan.extra`.

use crate::classify::Classified;
use crate::elevation::{trim_profile_d, PartArgs, Plan};
use crate::frame::{Basis, Frame, P2};
use crate::js::*;
use crate::rng::Rng;

pub const VICTORIAN_ROW_VERSION: &str = "sf-archetype-victorian-row-v1";
const KIND: &str = "victorian-row";

const UPPER_STOREY_M: f64 = 3.45;
const GROUND_BONUS_M: f64 = 0.3;
const STOREY_FLOOR_M: f64 = 2.35;
const PARAPET_M: f64 = 0.82;
const COPING_M: f64 = 0.14;
const CORNICE_M: f64 = 0.52;
const STOOP_RISE_MIN: f64 = 0.85;
const STOOP_RISE_MAX: f64 = 1.5;
const BAY_PROJECTION_MIN: f64 = 0.56;
const BAY_PROJECTION_MAX: f64 = 0.84;
const ORIEL_MAX_M: f64 = 4.4;
const LEFTOVER_MIN_M: f64 = 1.7;
const ENTRY_W_MIN: f64 = 1.7;
const ENTRY_W_MAX: f64 = 2.55;
const END_PIER_M: f64 = 0.28;
const SIDEWALK_HEADROOM_M: f64 = 2.29;
const AT_GRADE_ALLOWANCE_M: f64 = 0.45;
const BRACKET_PITCH_M: f64 = 1.15;
const DENTIL_PITCH_M: f64 = 0.34;

const WALL_FINISHES: [&str; 6] = ["wood-siding", "wood-siding", "wood-siding", "wood-siding", "painted-stucco", "wood-siding"];
const TRIM_FINISHES: [&str; 5] = ["painted-white", "painted-white", "cast-stone-trim", "painted-white", "signal-red"];
const BAY_SHAPES: [&str; 5] = ["square", "square", "canted", "canted", "round"];

fn pick<'a>(random: &mut Rng, list: &[&'a str]) -> &'a str {
    list[(list.len() - 1).min((random.next() * list.len() as f64).floor() as usize)]
}

fn storey_stack(height: f64, want_floor: f64) -> (Vec<f64>, f64, f64) {
    let roof_above = PARAPET_M + COPING_M;
    let floor_level = 0f64.max(want_floor.min(height - roof_above - STOREY_FLOOR_M));
    let usable = STOREY_FLOOR_M.max(height - roof_above - floor_level);
    let mut count = 1f64.max(js_round(usable / UPPER_STOREY_M));
    while count > 1.0 && (usable - GROUND_BONUS_M) / count < STOREY_FLOOR_M {
        count -= 1.0;
    }
    let upper = (usable - GROUND_BONUS_M) / count;
    let mut storeys = vec![upper + GROUND_BONUS_M];
    for _ in 1..count as usize {
        storeys.push(upper);
    }
    (storeys, floor_level, PARAPET_M)
}

/// (widths, orielIndices, entryIndex)
fn front_bay_plan(front_length: f64, random: &mut Rng) -> (Vec<f64>, Vec<usize>, Option<usize>) {
    let available = 0f64.max(front_length - 2.0 * END_PIER_M);
    if available < 3.6 {
        return (vec![available], vec![0], Some(0));
    }
    let entry_w = ENTRY_W_MAX.min(ENTRY_W_MIN.max(available * 0.32));
    let rest = available - entry_w;
    let mut oriel_w = ORIEL_MAX_M.min(rest);
    let mut leftover = rest - oriel_w;
    if leftover < LEFTOVER_MIN_M {
        oriel_w += leftover;
        leftover = 0.0;
    }
    let plain_count = if leftover > 0.0 { 1f64.max(js_round(leftover / 3.2)) } else { 0.0 };
    let plain_w = if plain_count > 0.0 { leftover / plain_count } else { 0.0 };
    let entry_first = random.next() < 0.5;
    let mut widths = vec![];
    let mut entry_index = None;
    let mut oriels = vec![];
    if entry_first {
        entry_index = Some(widths.len());
        widths.push(entry_w);
    }
    oriels.push(widths.len());
    widths.push(oriel_w);
    for _ in 0..plain_count as usize {
        widths.push(plain_w);
    }
    if !entry_first {
        entry_index = Some(widths.len());
        widths.push(entry_w);
    }
    (widths, oriels, entry_index)
}

pub fn spec(c: &Classified, frame: &Frame) -> J {
    let mut random = Rng::from_num(c.seed as f64);
    let wall = pick(&mut random, &WALL_FINISHES);
    let trim = pick(&mut random, &TRIM_FINISHES);
    let bay_shape = pick(&mut random, &BAY_SHAPES);
    let projection = BAY_PROJECTION_MIN + random.next() * (BAY_PROJECTION_MAX - BAY_PROJECTION_MIN);
    let dentils = random.next() < 0.55;
    let stoop_wanted = STOOP_RISE_MIN + random.next() * (STOOP_RISE_MAX - STOOP_RISE_MIN);
    let retail = c.has_retail_ground;
    let (storeys, floor_level, parapet) = storey_stack(c.height, if retail { 0.0 } else { stoop_wanted });
    let storey_count = storeys.len();
    let front_length = {
        let l = frame.edge_for_role("front").map(|e| e.length).unwrap_or(0.0);
        if l != 0.0 && !l.is_nan() {
            l
        } else if c.frontage != 0.0 && !c.frontage.is_nan() {
            c.frontage
        } else {
            8.4
        }
    };
    let (widths, mut oriel_indices, entry_index) = front_bay_plan(front_length, &mut random);
    let mut bay_start: usize = if retail && storey_count > 1 { 1 } else { 0 };
    if let Some(ei) = entry_index {
        if oriel_indices.contains(&ei) {
            bay_start = bay_start.max(1);
        }
    }
    if bay_start as i64 > storey_count as i64 - 1 {
        oriel_indices.clear();
    }
    let ground = if retail { "storefront" } else { "entrance" };
    let bays: Vec<J> = widths
        .iter()
        .enumerate()
        .map(|(index, width)| {
            let mut per = Map::new();
            for s in 0..storey_count {
                let v = if oriel_indices.contains(&index) && s >= bay_start {
                    "blind"
                } else if s == 0 {
                    if Some(index) == entry_index {
                        ground
                    } else if retail {
                        "storefront"
                    } else {
                        "punched"
                    }
                } else {
                    "punched"
                };
                per.insert(s.to_string(), json!(v));
            }
            json!({ "width": width, "storeys": per, "mullions": 1 })
        })
        .collect();
    let sill_courses: Vec<J> = (1..storey_count)
        .map(|s| json!({ "atStorey": s, "height": 0.16, "projection": 0.09, "profile": "stepped", "slot": "trim" }))
        .collect();
    let ornate = |with_entry: bool| {
        let mut entrance = json!({ "width": 1.15, "height": 2.38, "recess": 0.36, "transom": 0.42, "leaf": "single-glass", "frameWidth": 0.08 });
        if !retail {
            set(&mut entrance, "canopy", json!({ "projection": 1.22, "thickness": 0.16, "overhang": 0.34, "rods": false, "slot": "trim" }));
        }
        let mut e = json!({
            "bayWidth": 3.3,
            "window": "punched",
            "endPier": END_PIER_M,
            "pierMin": 0.55,
            "align": "start",
            "reveal": 0.22,
            "windowSpec": { "sillHeight": 0.82, "headClear": 0.44, "widthRatio": 0.52, "frameWidth": 0.075, "frameDepth": 0.09, "mullions": 1, "transoms": 0 },
            "entranceSpec": entrance,
            "storefrontSpec": { "pier": if retail { 0.18 } else { 0.7 }, "stallRiser": 0.42, "headClear": 0.55, "mullionSpacing": 1.3, "transom": 0.42 },
            "sill": { "height": 0.11, "projection": 0.075, "overhang": 0.11, "profile": "sloped" },
            "surround": { "width": 0.16, "projection": 0.055, "slot": "trim" },
            "stringCourses": sill_courses.clone(),
            "cornice": { "height": CORNICE_M, "projection": 0.3, "profile": "ogee", "slot": "trim" },
            "cornerReturns": { "width": 0.32, "projection": 0.06, "slot": "trim" },
            "downpipes": false,
            "sillSlot": "trim",
        });
        if with_entry {
            set(&mut e, "bays", json!(bays.clone()));
        }
        e
    };
    json!({
        "id": format!("sf-archetype-{}", c.seed),
        "seed": c.seed,
        "height": c.height,
        "storeys": storeys.iter().map(|h| json!({ "height": h, "use": "residential" })).collect::<Vec<_>>(),
        "floorLevel": floor_level,
        "wallThickness": 0.3,
        "parapet": { "height": parapet, "thickness": 0.3 },
        "coping": { "height": COPING_M, "overhang": 0.07, "profile": "flat" },
        "materials": {
            "wall": wall,
            "trim": trim,
            "base": "cast-stone",
            "bulkhead": "cast-stone",
            "glass": "glass-clear",
            "doorGlass": "glass-clear",
            "frame": trim,
            "door": trim,
            "parapet": wall,
            "coping": "cast-stone",
            "storefront": "painted-steel-light",
            "recess": wall,
            "roof": "asphalt-roof",
        },
        "elevations": {
            "front": ornate(true),
            "front-secondary": ornate(false),
            "left": { "window": "blind", "cornice": false },
            "right": { "window": "blind", "cornice": false },
            "back": {
                "window": "punched",
                "ground": "punched",
                "bayWidth": 3.4,
                "endPier": 0.4,
                "windowSpec": { "sillHeight": 0.9, "headClear": 0.5, "widthRatio": 0.46, "mullions": 0 },
                "sill": { "height": 0.08, "projection": 0.05, "overhang": 0.06, "profile": "sloped" },
                "cornice": { "height": 0.24, "projection": 0.14, "profile": "flat", "slot": "trim" },
                "downpipes": "corners",
            },
            "default": { "window": "blind", "cornice": false },
        },
        "roof": { "slab": 0.3 },
        "archetype": {
            "kind": KIND,
            "version": VICTORIAN_ROW_VERSION,
            "bayShape": bay_shape,
            "projection": projection,
            "orielIndices": oriel_indices,
            "entryIndex": entry_index,
            "bayStartStorey": bay_start,
            "dentils": dentils,
            "corniceHeight": CORNICE_M,
            "stoopRise": floor_level,
            "retail": retail,
            "wall": wall,
            "trim": trim,
        },
    })
}

fn bay_plan_polygon(basis: &Basis, shape: &str, s0: f64, s1: f64, p: f64) -> Vec<P2> {
    let [ox, oz] = basis.origin;
    let [ux, uz] = basis.u;
    let [nx, nz] = basis.n;
    let at = |s: f64, w: f64| P2 { x: ox + ux * s + nx * w, z: oz + uz * s + nz * w };
    let width = s1 - s0;
    if shape == "round" {
        let h = width / 2.0;
        let radius = (p * p + h * h) / (2.0 * p);
        let centre_w = p - radius;
        let half = (h / radius).min(1.0).asin();
        let segments = 8;
        return (0..=segments)
            .map(|i| {
                let a = -half + (2.0 * half * i as f64) / segments as f64;
                at(s0 + h + a.sin() * radius, centre_w + a.cos() * radius)
            })
            .collect();
    }
    if shape == "canted" {
        let cant = 0.62f64.min(width * 0.24);
        return vec![at(s0, 0.0), at(s0 + cant, p), at(s1 - cant, p), at(s1, 0.0)];
    }
    vec![at(s0, 0.0), at(s0, p), at(s1, p), at(s1, 0.0)]
}

#[allow(clippy::too_many_arguments)]
fn bay_front_sash(plan: &mut Plan, basis: &Basis, edge: usize, s0: f64, s1: f64, y0: f64, y1: f64, w: f64, kind: &str) {
    let f = 0.075;
    let wf0 = w - 0.02;
    let wf1 = w + 0.045;
    let meta = json!({ "archetype": kind });
    let b = |s0: f64, s1: f64, y0: f64, y1: f64, w0: f64, w1: f64| {
        PartArgs::new("bay-sash", "frame", s0, s1, y0, y1, w0, w1).edge(edge).meta(meta.clone())
    };
    plan.add_box(basis, b(s0, s0 + f, y0, y1, wf0, wf1));
    plan.add_box(basis, b(s1 - f, s1, y0, y1, wf0, wf1));
    plan.add_box(basis, b(s0, s1, y1 - f, y1, wf0, wf1));
    plan.add_box(basis, b(s0, s1, y0, y0 + f, wf0, wf1));
    let mid = (s0 + s1) / 2.0;
    plan.add_box(basis, b(mid - 0.026, mid + 0.026, y0, y1, wf0 + 0.01, wf1 - 0.01));
    plan.add_box(
        basis,
        PartArgs::new("glass", "glass", s0 + f * 0.5, s1 - f * 0.5, y0 + f * 0.5, y1 - f * 0.5, w + 0.004, w + 0.014)
            .edge(edge)
            .meta(meta.clone())
            .shadow(false),
    );
}

pub fn detail(plan: &mut Plan, frame: &Frame, spec: &J) {
    let Some(meta) = spec.get_nn("archetype") else { return };
    let Some(front) = frame.edge_for_role("front") else { return };
    let base_y = if plan.base_y.is_nan() { 0.0 } else { plan.base_y };
    let Some(rec) = plan.edges.iter().position(|e| e.index == front.index) else { return };
    let basis = front.basis;
    let kind = meta.s("kind").unwrap_or(KIND).to_string();
    let wall_top = plan.envelope.wall_top + base_y;
    let cornice_bottom = wall_top - meta.num("corniceHeight");
    let n = plan.storeys.len();
    let bay_shape = meta.s("bayShape").unwrap_or("").to_string();
    let oriel_indices: Vec<usize> =
        meta.arr("orielIndices").map(|a| a.iter().filter_map(|v| v.as_f64()).map(|v| v as usize).collect()).unwrap_or_default();

    // -- The projecting bay
    let start_storey = (meta.num("bayStartStorey") as i64).min(n as i64 - 1).max(0) as usize;
    let bay_y0 = plan.storeys[start_storey].y0 + base_y;
    let bay_y1 = cornice_bottom - 0.06;
    let allow = 0.2f64.max(if bay_y0 - base_y < SIDEWALK_HEADROOM_M { front.allowance.min(AT_GRADE_ALLOWANCE_M) } else { front.allowance });
    let p = meta.num("projection").min(allow - 0.12);
    let cap_p = (p + 0.1).min(allow - 0.01);
    let apron_p = (p + 0.08).min(allow - 0.01);
    for &index in &oriel_indices {
        let Some(bay) = plan.edges[rec].bays.get(index).cloned() else { continue };
        if bay.width < 1.1 || bay_y1 - bay_y0 < 1.6 {
            continue;
        }
        let inset = 0.16f64.min(bay.width * 0.06);
        let s0 = bay.s0 + inset;
        let s1 = bay.s1 - inset;
        plan.add_prism(PartArgs {
            kind: "oriel".into(),
            slot: "wall".into(),
            points: bay_plan_polygon(&basis, &bay_shape, s0, s1, p),
            y0: bay_y0,
            y1: bay_y1,
            cast_shadow: Some(true),
            receive_shadow: Some(true),
            meta: Some(json!({ "archetype": kind, "shape": bay_shape, "edge": front.index })),
            ..Default::default()
        });
        plan.add_profile(&basis, PartArgs::profile("oriel-cap", "trim", s0 - 0.07, s1 + 0.07, trim_profile_d("ogee", cap_p, 0.26, bay_y1)).edge(front.index));
        let apron_h = 0.2f64.min(0f64.max(plan.storeys[start_storey].y0 + 0.08));
        if apron_h > 0.1 {
            plan.add_profile(
                &basis,
                PartArgs::profile("oriel-apron", "trim", s0 - 0.07, s1 + 0.07, trim_profile_d("stepped", apron_p, apron_h, bay_y0 - apron_h)).edge(front.index),
            );
        }
        let glass_inset = if bay_shape == "canted" {
            0.62f64.min((s1 - s0) * 0.24) + 0.12
        } else if bay_shape == "round" {
            (s1 - s0) * 0.3
        } else {
            0.18
        };
        for i in start_storey..n {
            let (sy0, sy1) = (plan.storeys[i].y0, plan.storeys[i].y1);
            let wy0 = sy0 + base_y + 0.78;
            let wy1 = (sy1 + base_y - 0.4).min(bay_y1 - 0.25);
            if wy1 - wy0 < 0.9 {
                continue;
            }
            let gs0 = s0 + glass_inset;
            let gs1 = s1 - glass_inset;
            if gs1 - gs0 < 0.6 {
                continue;
            }
            bay_front_sash(plan, &basis, front.index, gs0, gs1, wy0, wy1, p, &kind);
            if bay_shape == "square" {
                for (c0, c1) in [(s0 - 0.014, s0 - 0.004), (s1 + 0.004, s1 + 0.014)] {
                    plan.add_box(
                        &basis,
                        PartArgs::new("glass", "glass", c0, c1, wy0, wy1, 0.16, p - 0.09)
                            .edge(front.index)
                            .shadow(false)
                            .meta(json!({ "archetype": kind, "face": "cheek" })),
                    );
                }
            }
        }
    }

    // -- The stoop
    let entry = plan.apertures.iter().find(|a| a.f("edge").map(|e| e as usize) == Some(front.index)).cloned();
    let stoop_rise = meta.num("stoopRise");
    if let Some(entry) = entry {
        if stoop_rise > 0.55 {
            let es0 = entry.num("s0");
            let es1 = entry.num("s1");
            // The kit's aperture y0 is `op.y0 + baseY`; the port's aperture
            // record leaves it out, so read it from the opening record.
            let id = entry.s("id").unwrap_or("");
            let op_y0 = plan.edges[rec].openings.iter().find(|o| o.id == id).map(|o| o.y0).unwrap_or(f64::NAN);
            let entry_y0 = entry.f("y0").unwrap_or(op_y0 + base_y);
            let rise = stoop_rise;
            let steps = 3f64.max(js_round(rise / 0.175));
            let riser = rise / steps;
            let going = 0.27;
            let run = 1.15f64.min(steps * going).min(0.2f64.max(front.allowance.min(AT_GRADE_ALLOWANCE_M)) - 0.05);
            let step_going = run / steps;
            let width = 1.3f64.max((es1 - es0) + 0.26);
            let mid = (es0 + es1) / 2.0;
            let ss0 = mid - width / 2.0;
            let ss1 = mid + width / 2.0;
            let mut stair = vec![[-0.05, base_y]];
            for i in 0..steps as usize {
                let fi = i as f64;
                stair.push([run - fi * step_going, base_y + fi * riser]);
                stair.push([run - fi * step_going, base_y + (fi + 1.0) * riser]);
            }
            stair.push([-0.05, base_y + rise]);
            plan.add_profile(&basis, PartArgs::profile("stoop", "base", ss0, ss1, stair).edge(front.index).meta(json!({ "archetype": kind })));
            let rail_top = base_y + rise + 0.92;
            let cheek = vec![[-0.05, base_y], [run, base_y], [run, base_y + 0.9], [run * 0.18, rail_top], [-0.05, rail_top]];
            for cs in [ss0, ss1 - 0.14] {
                plan.add_profile(&basis, PartArgs::profile("stoop-cheek", "trim", cs, cs + 0.14, cheek.clone()).edge(front.index).meta(json!({ "archetype": kind })));
            }
            let hood_top = entry_y0 + 2.38 + 0.42 + 0.15;
            for ps in [ss0 + 0.05, ss1 - 0.17] {
                plan.add_box(
                    &basis,
                    PartArgs::new("canopy-rod", "trim", ps, ps + 0.12, base_y + rise, hood_top, run - 0.24, run - 0.12)
                        .edge(front.index)
                        .meta(json!({ "archetype": kind, "part": "porch-post" })),
                );
            }
        }
    }

    // -- Bracketed cornice and decorated window heads
    let mut street_edges = vec![front];
    street_edges.extend(frame.edges_for_role("front-secondary"));
    let dentils = meta.truthy("dentils");
    for edge in street_edges {
        let Some(r) = plan.edges.iter().position(|e| e.index == edge.index) else { continue };
        if edge.party || edge.length < 1.2 {
            continue;
        }
        let eb = edge.basis;
        let bracket_depth = 0.26;
        let bracket_count = 2f64.max(js_round(edge.length / BRACKET_PITCH_M));
        let bracket_pitch = edge.length / bracket_count;
        for i in 0..=bracket_count as usize {
            let s = i as f64 * bracket_pitch;
            plan.add_box(
                &eb,
                PartArgs::new("cornice-bracket", "trim", s - 0.085, s + 0.085, cornice_bottom - 0.46, cornice_bottom + 0.02, -0.01, bracket_depth)
                    .edge(edge.index)
                    .meta(json!({ "archetype": kind })),
            );
            plan.add_box(
                &eb,
                PartArgs::new("cornice-bracket", "trim", s - 0.12, s + 0.12, cornice_bottom - 0.58, cornice_bottom - 0.44, -0.01, bracket_depth * 0.62)
                    .edge(edge.index)
                    .meta(json!({ "archetype": kind })),
            );
        }
        if dentils {
            let dentil_count = 2f64.max(js_round(edge.length / DENTIL_PITCH_M));
            let pitch = edge.length / dentil_count;
            for i in 0..dentil_count as usize {
                let s = (i as f64 + 0.5) * pitch;
                plan.add_box(
                    &eb,
                    PartArgs::new("dentil", "trim", s - 0.085, s + 0.085, cornice_bottom - 0.74, cornice_bottom - 0.6, -0.01, 0.13)
                        .edge(edge.index)
                        .shadow(false)
                        .meta(json!({ "archetype": kind })),
                );
            }
        }
        let ops: Vec<(f64, f64, f64)> = plan.edges[r]
            .openings
            .iter()
            .filter(|op| op.type_ == "punched" && !(edge.index == front.index && oriel_indices.contains(&op.bay)))
            .map(|op| (op.s0, op.s1, op.y1))
            .collect();
        for (s0, s1, y1) in ops {
            plan.add_profile(
                &eb,
                PartArgs::profile("window-head", "trim", s0 - 0.24, s1 + 0.24, trim_profile_d("cove", 0.19, 0.21, y1 + 0.16))
                    .edge(edge.index)
                    .meta(json!({ "archetype": kind })),
            );
        }
    }
    let tints = super::period_paint::period_paint_tints(&plan.seed, plan.material("wall")).unwrap_or_else(|| json!({}));
    plan.extra.insert("tints".into(), tints);
}
