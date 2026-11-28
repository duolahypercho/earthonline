//! Port of src/world/buildings/archetypes/bay-window-apartment.js: the SF
//! bay-window apartment block (spec, oriel pass, Chinatown front, crowning
//! cornice). `plan.archetype` and `plan.tints` go into `plan.extra`.

use crate::classify::Classified;
use crate::elevation::{trim_profile_d, Plan, PartArgs};
use crate::frame::{Basis, Edge, Frame, P2};
use crate::js::*;
use crate::rng::{hash_seed, js_num, Rng};

pub const BAY_WINDOW_APARTMENT_VERSION: &str = "sf-archetype-bay-window-apartment-v1";
pub const BAY_WINDOW_CORNICE_VERSION: &str = "sf-archetype-bay-window-cornice-v1";
const KIND: &str = "bay-window-apartment";

const GROUND_STOREY_M: f64 = 3.6;
const UPPER_STOREY_M: f64 = 3.05;
const BAY_PROJECTION_MIN: f64 = 0.62;
const BAY_PROJECTION_MAX: f64 = 0.88;
const BAY_WIDTH_RATIO: f64 = 0.72;
const COPING_M: f64 = 0.12;
const DEFAULT_BAY_ALLOWANCE_M: f64 = 0.45;
const BAY_TRIM_PROUD_M: f64 = 0.08;
const BAY_SASH_FRAME_M: f64 = 0.07;
const BAY_SASH_PROUD_M: f64 = 0.045;

const WALL_FINISHES: [&str; 4] = ["painted-stucco", "float-finish-stucco", "wood-siding", "painted-stucco"];
const CHINATOWN_PAINT: [u32; 7] = [0xb0453c, 0x4f8a6b, 0xd9b24a, 0x3f7f86, 0xf2e6cf, 0xb0453c, 0x4f8a6b];
const BALCONY_DEPTH: f64 = 0.85;
const BALCONY_RAIL: f64 = 1.0;
const BALCONY_SHARE: f64 = 0.64;
const EAVE_PROJECTION: f64 = 0.55;
const EAVE_HEIGHT: f64 = 0.42;
const CORNICE_MIN_EDGE_M: f64 = 3.5;
const MODILLION_PITCH_M: f64 = 0.72;

fn storey_stack(height: f64) -> (Vec<f64>, f64) {
    let parapet = 0.7;
    let usable = GROUND_STOREY_M.max(height - parapet - COPING_M);
    let upper_count = 0f64.max(js_round((usable - GROUND_STOREY_M) / UPPER_STOREY_M));
    let upper_height = if upper_count > 0.0 { (usable - GROUND_STOREY_M) / upper_count } else { 0.0 };
    let mut storeys = vec![GROUND_STOREY_M];
    for _ in 0..upper_count as usize {
        storeys.push(upper_height);
    }
    (storeys, parapet)
}

pub fn spec(c: &Classified, frame: &Frame) -> J {
    let mut random = Rng::from_num(c.seed as f64);
    let (storeys, parapet) = storey_stack(c.height);
    let drawn_wall = WALL_FINISHES[(random.next() * WALL_FINISHES.len() as f64).floor() as usize];
    let chinatown = c.template == "chinatown-mixed" && storeys.len() >= 2;
    let mut ct = Rng::new(&format!("{}|chinatown", c.seed));
    let (ct_wall, ct_paint, ct_eave) = (ct.next(), ct.next(), ct.next());
    let wall = if chinatown { if ct_wall < 0.7 { "painted-stucco" } else { "brick-running-bond" } } else { drawn_wall };
    let front = frame.edge_for_role("front");
    let front_length = {
        let l = front.map(|e| e.length).unwrap_or(0.0);
        if l != 0.0 && !l.is_nan() {
            l
        } else if c.frontage != 0.0 && !c.frontage.is_nan() {
            c.frontage
        } else {
            12.0
        }
    };
    let bay_count = 1f64.max(js_round(front_length / 4.2));
    let ground = if c.has_retail_ground { "storefront" } else { "punched" };
    let projection = BAY_PROJECTION_MIN + random.next() * (BAY_PROJECTION_MAX - BAY_PROJECTION_MIN);
    let canted = random.next() < 0.62;
    let entry_roll = random.next();
    let garage_row = c.variant == "flats" && !c.has_retail_ground && front.is_some() && bay_count >= 2.0;
    let mut by_edge = Map::new();
    if garage_row {
        let front = front.unwrap();
        let width = (front.length - 1.0) / bay_count;
        let entry = if entry_roll < 0.5 { 0 } else { bay_count as usize - 1 };
        let bays: Vec<J> = (0..bay_count as usize)
            .map(|i| json!({ "width": width, "storeys": { "0": if i == entry { "entrance" } else { "roller" } } }))
            .collect();
        by_edge.insert(
            front.index.to_string(),
            json!({
                "bays": bays,
                "ground": "punched",
                "window": "punched",
                "endPier": 0.5,
                "storeys": storeys.iter().map(|_| "blind").collect::<Vec<_>>(),
                "rollerSpec": { "width": 2.7f64.min(width - 0.8), "height": 2.3, "inset": 0.12, "hood": 0, "slat": 0.5 },
                "entranceSpec": { "width": 1.1, "height": 2.4, "recess": 0.6, "transom": 0.4, "leaf": "single-solid", "frameWidth": 0.07 },
            }),
        );
    }
    let front_storeys: Vec<&str> =
        (0..storeys.len()).map(|i| if i == 0 { ground } else if chinatown { "punched" } else { "blind" }).collect();
    let others = 1f64.max(bay_count - 1.0);
    let chinatown_meta = if chinatown {
        let i = ((ct_paint * CHINATOWN_PAINT.len() as f64).floor() as usize) % CHINATOWN_PAINT.len();
        json!({ "paint": CHINATOWN_PAINT[i] })
    } else {
        J::Null
    };
    // On SF residential hillsides (Russian Hill, Nob Hill, North Beach), a realistic share of
    // 2-4 storey apartment houses carry pitched gabled roofs (clay tile or composition shingle)
    // and wider 2-5 storey residential blocks carry terracotta/shingle mansard roof skirts
    // instead of 100% flat parapet boxes. Normalize footprint dimensions by groundscale so
    // real SF lot sizes pass cleanly at 1.25x plan scale.
    let gs = eo_data::groundscale::resolve().unwrap_or(1.0).max(0.5);
    let real_front = front_length / gs;
    let real_area = frame.area / (gs * gs);
    let mut prng = Rng::new(&format!("{}|bwa-pitched", c.seed));
    let p_roll = prng.next();
    let p_mat_roll = prng.next();
    let p_pitch = 0.34 + prng.next() * 0.24;
    let pitched_gable = crate::typology::detail_enabled()
        && !chinatown
        && storeys.len() >= 2
        && storeys.len() <= 4
        && front.is_some()
        && real_front >= 4.5
        && real_front <= 16.0
        && real_area <= 440.0
        && frame.polygon.len() <= 10
        && p_roll < 0.50;
    let mansard_skirt = crate::typology::detail_enabled()
        && !chinatown
        && !pitched_gable
        && storeys.len() >= 2
        && storeys.len() <= 5
        && front.is_some()
        && real_front >= 6.0
        && real_area <= 920.0
        && p_roll >= 0.50
        && p_roll < 0.78;
    let gable_rise = if pitched_gable { (0.5 * real_front * p_pitch).clamp(1.2, 3.6) } else { 0.0 };
    let pitched_mat = if p_mat_roll < 0.56 { "clay-barrel-tile" } else { "composition-shingle" };
    let mut spec = json!({
        "id": format!("sf-archetype-{}", c.seed),
        "seed": c.seed,
        "height": c.height,
        "storeys": storeys.iter().map(|h| json!({ "height": h })).collect::<Vec<_>>(),
        "parapet": { "height": parapet, "thickness": 0.3 },
        "coping": { "height": COPING_M, "overhang": 0.06 },
        "materials": {
            "wall": wall,
            "trim": "cast-stone-trim",
            "glass": "glass-clear",
            "frame": "painted-white",
            "roof": "asphalt-roof",
            "tile": if ct_eave < 0.6 { "clay-barrel-tile" } else { "glazed-tile" },
            "metal": "painted-steel",
        },
        "elevations": {
            "front": {
                "bays": { "count": bay_count },
                "ground": ground,
                "window": "punched",
                "endPier": 0.5,
                "storeys": front_storeys,
            },
            "front-secondary": { "bays": { "count": others }, "ground": "punched", "window": "punched", "endPier": 0.5 },
            "left": "blind",
            "right": "blind",
            "back": { "bays": { "count": others }, "window": "punched", "ground": "punched", "endPier": 0.5 },
            "default": "blind",
            "byEdge": J::Object(by_edge),
        },
        "archetype": {
            "kind": KIND,
            "version": BAY_WINDOW_APARTMENT_VERSION,
            "bayCount": bay_count,
            "projection": projection,
            "canted": canted,
            "garageRow": garage_row,
            "chinatown": chinatown_meta,
            "wall": wall,
            "pitched": pitched_gable,
            "mansard": mansard_skirt,
            "gableRise": gable_rise,
        },
    });
    if pitched_gable || mansard_skirt {
        set(obj_mut(&mut spec, "materials"), "pitchedRoof", json!(pitched_mat));
        let min_allow = if crate::typology::detail_enabled() { 4.2 } else { 1.8 };
        set(&mut spec, "roof", json!({ "slab": 0.3, "plantAllowance": (gable_rise.max(min_allow)) + 0.8 }));
    }
    spec
}

fn allowed_for(front: &Edge, frame: &Frame) -> f64 {
    if front.allowance.is_finite() {
        front.allowance
    } else if frame.allowance.is_finite() {
        frame.allowance
    } else {
        DEFAULT_BAY_ALLOWANCE_M
    }
}

fn add_chinatown_front(plan: &mut Plan, frame: &Frame, front: &Edge, meta: &J, lift: f64) {
    let allowed = allowed_for(front, frame);
    let l = front.length;
    let run = (l - 1.0).min(2.4f64.max(l * BALCONY_SHARE));
    let s0 = (l - run) / 2.0;
    let s1 = s0 + run;
    let depth = BALCONY_DEPTH.min(allowed - 0.06);
    let kind = meta.s("kind").unwrap_or(KIND).to_string();
    let tag = json!({ "archetype": kind, "part": "chinatown-balcony" });
    let basis = front.basis;
    let n = plan.storeys.len();
    if depth >= 0.3 && run >= 2.0 {
        for i in 1..n {
            let y = lift + plan.storeys[i].y0;
            let b = |k: &str, slot: &str, s0: f64, s1: f64, y0: f64, y1: f64, w0: f64, w1: f64| {
                PartArgs::new(k, slot, s0, s1, y0, y1, w0, w1).edge(front.index).storey(i as i64).meta(tag.clone())
            };
            plan.add_box(&basis, b("balcony", "trim", s0, s1, y - 0.14, y + 0.02, 0.0, depth));
            plan.add_box(&basis, b("balcony", "metal", s0, s1, y + 0.02, y + BALCONY_RAIL * 0.55, depth - 0.05, depth));
            for (a, bb) in [(s0, s0 + 0.05), (s1 - 0.05, s1)] {
                plan.add_box(&basis, b("balcony", "metal", a, bb, y + 0.02, y + BALCONY_RAIL * 0.55, 0.02, depth));
            }
            plan.add_box(&basis, b("balcony", "metal", s0 - 0.03, s1 + 0.03, y + BALCONY_RAIL - 0.06, y + BALCONY_RAIL, depth - 0.06, depth));
            let mut s = s0 + 0.6;
            while s < s1 - 0.3 {
                plan.add_box(
                    &basis,
                    b("balcony", "metal", s - 0.025, s + 0.025, y + BALCONY_RAIL * 0.55, y + BALCONY_RAIL - 0.06, depth - 0.05, depth - 0.01),
                );
                s += 0.6;
            }
        }
    }
    let roof_top = plan.envelope.roof_top + lift;
    // `plan.coping` is never set by planElevation: the fallback always applies.
    let coping_h = COPING_M;
    let eave = EAVE_PROJECTION.min(allowed - 0.05);
    let ey = roof_top - coping_h - EAVE_HEIGHT - 0.04;
    if eave > 0.2 && ey > lift + plan.storeys[n - 1].y0 + 1.2 {
        plan.add_profile(
            &basis,
            PartArgs::profile("cornice", "tile", 0.05, l - 0.05, vec![[-0.02, ey], [eave, ey], [eave, ey + 0.06], [-0.02, ey + EAVE_HEIGHT]])
                .edge(front.index)
                .meta(json!({ "archetype": kind, "part": "chinatown-eave" })),
        );
    }
    let paint = meta.at("chinatown").f("paint").unwrap_or(f64::NAN);
    let wall = meta.s("wall").unwrap_or("");
    let tints = if paint == 0xf2e6cf as f64 || wall != "painted-stucco" {
        json!({})
    } else {
        let mut m = Map::new();
        m.insert(wall.to_string(), jn(paint));
        J::Object(m)
    };
    plan.extra.insert("tints".into(), tints);
}

fn bay_face_basis(basis: &Basis, a: [f64; 2], b: [f64; 2]) -> (f64, Basis) {
    let [ox, oz] = basis.origin;
    let [ux, uz] = basis.u;
    let [nx, nz] = basis.n;
    let ds = b[0] - a[0];
    let dw = b[1] - a[1];
    let length = ds.hypot(dw).max(1e-6);
    let ts = ds / length;
    let tw = dw / length;
    let os = -tw;
    let ow = ts;
    (
        length,
        Basis {
            origin: [ox + ux * a[0] + nx * a[1], oz + uz * a[0] + nz * a[1]],
            u: [ux * ts + nx * tw, uz * ts + nz * tw],
            n: [ux * os + nx * ow, uz * os + nz * ow],
        },
    )
}

fn bay_world_pt(basis: &Basis, s: f64, w: f64) -> P2 {
    P2 { x: basis.origin[0] + basis.u[0] * s + basis.n[0] * w, z: basis.origin[1] + basis.u[1] * s + basis.n[1] * w }
}

fn add_bay_projections(plan: &mut Plan, frame: &Frame, spec: &J) {
    let Some(meta) = spec.get_nn("archetype") else { return };
    let Some(front) = frame.edge_for_role("front") else { return };
    if plan.storeys.len() < 2 {
        return;
    }
    let kind = meta.s("kind").unwrap_or(KIND).to_string();
    let rec = plan.edges.iter().position(|e| e.index == front.index);
    let bays: Vec<(f64, f64, f64)> =
        rec.map(|r| plan.edges[r].bays.iter().map(|b| (b.s0, b.s1, b.width)).collect()).unwrap_or_default();
    let lift = if plan.base_y.is_finite() { plan.base_y } else { 0.0 };
    if truthy(meta.at("chinatown")) {
        add_chinatown_front(plan, frame, front, meta, lift);
        return;
    }
    let n = plan.storeys.len();
    let y0 = lift + plan.storeys[1].y0;
    let y1 = lift + plan.storeys[n - 1].y1;
    let allowed = allowed_for(front, frame);
    let outermost = 0.14f64.max(allowed - 0.02);
    let projection = meta.num("projection").min(outermost - BAY_TRIM_PROUD_M);
    let canted = meta.at("canted").cloned().unwrap_or(J::Null);
    let is_canted = crate::typology::detail_enabled() && truthy(meta.at("canted")) && projection >= 0.28;
    let detail_on = crate::typology::detail_enabled();
    let basis = front.basis;
    let e = front.index;
    for (bs0, bs1, bw) in bays {
        let width = bw * BAY_WIDTH_RATIO;
        let centre = (bs0 + bs1) / 2.0;
        let s0 = centre - width / 2.0;
        let s1 = centre + width / 2.0;
        if is_canted && width >= 1.6 {
            // True 3-sided canted bay window prism with angled cheek sashes & molded cornice wrap
            let cant = (width * 0.24).min(projection * 0.92).min(0.62);
            let outline = [[s0, 0.0], [s0 + cant, projection], [s1 - cant, projection], [s1, 0.0]];
            plan.add_prism(PartArgs {
                kind: "bay".into(),
                slot: "wall".into(),
                points: outline.iter().map(|[s, w]| bay_world_pt(&basis, *s, *w)).collect(),
                y0,
                y1,
                cast_shadow: Some(true),
                receive_shadow: Some(true),
                meta: Some(json!({ "archetype": kind, "canted": true, "edge": e })),
                ..Default::default()
            });
            let faces: Vec<(f64, Basis)> = (0..3).map(|k| bay_face_basis(&basis, outline[k], outline[k + 1])).collect();
            for &(flen, fbasis) in &faces {
                plan.add_profile(&fbasis, PartArgs::profile("bay-cap", "trim", -0.04, flen + 0.04, trim_profile_d("ogee", 0.12, 0.24, y1)).edge(e));
                plan.add_profile(&fbasis, PartArgs::profile("bay-sill", "trim", -0.04, flen + 0.04, trim_profile_d("stepped", 0.10, 0.20, y0 - 0.20)).edge(e));
            }
            let f = BAY_SASH_FRAME_M;
            let m = json!({ "archetype": kind, "part": "bay-glazing" });
            for i in 1..n {
                let wy0 = lift + plan.storeys[i].y0 + 0.72;
                let wy1 = lift + plan.storeys[i].y1 - 0.34;
                if wy1 - wy0 < 0.9 {
                    continue;
                }
                for (fi, &(flen, fbasis)) in faces.iter().enumerate() {
                    let margin = (flen * 0.11).min(0.14);
                    let a = margin;
                    let b = flen - margin;
                    if b - a < 0.36 {
                        continue;
                    }
                    let fw0 = -0.02;
                    let fw1 = BAY_SASH_PROUD_M;
                    let sash = |x0: f64, x1: f64, yy0: f64, yy1: f64, w1: f64| {
                        PartArgs::new("bay-sash", "frame", x0, x1, yy0, yy1, fw0, w1).edge(e).meta(m.clone())
                    };
                    plan.add_box(&fbasis, sash(a, a + f, wy0, wy1, fw1));
                    plan.add_box(&fbasis, sash(b - f, b, wy0, wy1, fw1));
                    plan.add_box(&fbasis, sash(a, b, wy1 - f, wy1, fw1));
                    plan.add_box(&fbasis, sash(a, b, wy0, wy0 + f, fw1));
                    // Double-hung meeting rail + front mullions on center face
                    let rail = wy0 + (wy1 - wy0) * 0.54;
                    plan.add_box(&fbasis, sash(a, b, rail - 0.025, rail + 0.025, fw1 - 0.01));
                    if fi == 1 && b - a > 1.35 {
                        let mm = (a + b) * 0.5;
                        plan.add_box(&fbasis, PartArgs::new("bay-post", "frame", mm - 0.045, mm + 0.045, wy0, wy1, fw0, fw1 + 0.01).edge(e).meta(m.clone()));
                    }
                    // Projecting window sill & head trim on each canted bay face
                    plan.add_box(&fbasis, PartArgs::new("sill", "trim", a - 0.05, b + 0.05, wy0 - 0.08, wy0 + 0.01, -0.01, fw1 + 0.055).edge(e).meta(m.clone()));
                    plan.add_box(&fbasis, PartArgs::new("window-head", "trim", a - 0.06, b + 0.06, wy1 + 0.01, wy1 + 0.12, -0.01, fw1 + 0.055).edge(e).meta(m.clone()));
                    plan.add_box(
                        &fbasis,
                        PartArgs::new("glass", "glass", a + f * 0.5, b - f * 0.5, wy0 + f * 0.5, wy1 - f * 0.5, 0.004, 0.014)
                            .edge(e)
                            .meta(m.clone())
                            .shadow(false),
                    );
                }
            }
            continue;
        }
        plan.add_box(
            &basis,
            PartArgs::new("bay", "wall", s0, s1, y0, y1, 0.0, projection)
                .edge(e)
                .shadow(true)
                .receive(true)
                .meta(json!({ "archetype": kind, "canted": canted })),
        );
        plan.add_profile(&basis, PartArgs::profile("bay-cap", "trim", s0 - 0.06, s1 + 0.06, trim_profile_d("ogee", projection + 0.08, 0.22, y1)).edge(e));
        plan.add_profile(
            &basis,
            PartArgs::profile("bay-sill", "trim", s0 - 0.06, s1 + 0.06, trim_profile_d("stepped", projection + 0.06, 0.18, y0 - 0.18)).edge(e),
        );
        let face = projection;
        let f = BAY_SASH_FRAME_M;
        let fw0 = face - 0.02;
        let fw1 = face + BAY_SASH_PROUD_M;
        let gs0 = s0 + 0.2f64.min(width * 0.08);
        let gs1 = s1 - 0.2f64.min(width * 0.08);
        if gs1 - gs0 < 0.8 {
            continue;
        }
        let m = json!({ "archetype": kind, "part": "bay-glazing" });
        for i in 1..n {
            let wy0 = lift + plan.storeys[i].y0 + 0.72;
            let wy1 = lift + plan.storeys[i].y1 - 0.34;
            if wy1 - wy0 < 0.9 {
                continue;
            }
            let sash = |s0: f64, s1: f64, y0: f64, y1: f64, w1: f64| {
                PartArgs::new("bay-sash", "frame", s0, s1, y0, y1, fw0, w1).edge(e).meta(m.clone())
            };
            plan.add_box(&basis, sash(gs0, gs0 + f, wy0, wy1, fw1));
            plan.add_box(&basis, sash(gs1 - f, gs1, wy0, wy1, fw1));
            plan.add_box(&basis, sash(gs0, gs1, wy1 - f, wy1, fw1));
            plan.add_box(&basis, sash(gs0, gs1, wy0, wy0 + f, fw1));
            for t in [1.0 / 3.0, 2.0 / 3.0] {
                let mm = gs0 + (gs1 - gs0) * t;
                plan.add_box(&basis, sash(mm - 0.03, mm + 0.03, wy0, wy1, fw1 - 0.01));
            }
            if detail_on {
                plan.add_box(&basis, PartArgs::new("sill", "trim", gs0 - 0.06, gs1 + 0.06, wy0 - 0.08, wy0 + 0.01, face - 0.01, fw1 + 0.055).edge(e).meta(m.clone()));
                plan.add_box(&basis, PartArgs::new("window-head", "trim", gs0 - 0.07, gs1 + 0.07, wy1 + 0.01, wy1 + 0.12, face - 0.01, fw1 + 0.055).edge(e).meta(m.clone()));
            }
            plan.add_box(
                &basis,
                PartArgs::new("glass", "glass", gs0 + f * 0.5, gs1 - f * 0.5, wy0 + f * 0.5, wy1 - f * 0.5, face + 0.004, face + 0.014)
                    .edge(e)
                    .meta(m.clone())
                    .shadow(false),
            );
            if face - 0.24 > 0.2 {
                for (c0, c1) in [(s0 - 0.014, s0 - 0.004), (s1 + 0.004, s1 + 0.014)] {
                    plan.add_box(&basis, PartArgs::new("glass", "glass", c0, c1, wy0, wy1, 0.12, face - 0.12).edge(e).meta(m.clone()).shadow(false));
                }
            }
        }
    }
    // Projecting stone water-table plinth belt along street edges so building walls meet sloped foundations crisply
    if detail_on {
        for edge in frame.edges.iter() {
            if edge.party || !(edge.role == "front" || edge.role == "front-secondary") || edge.length < 2.0 {
                continue;
            }
            let r_ops = plan.edges.iter().find(|er| er.index == edge.index);
            let mut cuts: Vec<(f64, f64)> = r_ops
                .map(|er| {
                    er.openings
                        .iter()
                        .filter(|o| o.storey == 0 && matches!(o.type_.as_str(), "entrance" | "roller" | "storefront"))
                        .map(|o| (o.s0 - 0.06, o.s1 + 0.06))
                        .collect()
                })
                .unwrap_or_default();
            cuts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            cuts.push((edge.length, edge.length));
            let wp = 0.065f64.min(0.03f64.max(edge.allowance - 0.03));
            let mut cur = 0.0;
            for (a, b) in cuts {
                let end = a.min(edge.length);
                if end - cur > 0.25 {
                    plan.add_profile(
                        &edge.basis,
                        PartArgs::profile("water-table", "trim", cur, end, trim_profile_d("stepped", wp, 0.26, lift - 0.02))
                            .edge(edge.index)
                            .meta(json!({ "archetype": kind, "part": "water-table" })),
                    );
                }
                cur = cur.max(b);
            }
        }
    }
    if truthy(meta.at("garageRow")) {
        let [ox, oz] = basis.origin;
        let [ux, uz] = basis.u;
        let [nx, nz] = basis.n;
        let garages: Vec<J> = rec
            .map(|r| {
                plan.edges[r]
                    .openings
                    .iter()
                    .filter(|op| op.type_ == "roller" && op.storey == 0)
                    .map(|op| {
                        let centre = (op.s0 + op.s1) / 2.0;
                        json!({ "edge": e, "width": op.s1 - op.s0, "world": { "x": ox + ux * centre, "z": oz + uz * centre }, "outward": [nx, nz] })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let first = garages.first().cloned().unwrap_or(J::Null);
        plan.extra.insert(
            "archetype".into(),
            json!({ "kind": kind, "version": meta.at("version").cloned().unwrap_or(J::Null), "garage": first, "garages": garages }),
        );
    }
}

fn add_bay_window_cornice(plan: &mut Plan, frame: &Frame, spec: &J) {
    let Some(meta) = spec.get_nn("archetype") else { return };
    let kind = meta.s("kind").unwrap_or(KIND).to_string();
    let seed_text = match spec.at("seed") {
        Some(J::Number(n)) => js_num(n.as_f64().unwrap_or(f64::NAN)),
        Some(J::String(s)) => s.clone(),
        _ => "undefined".into(),
    };
    let mut random = Rng::from_num(hash_seed(&format!("{seed_text}|bay-window-cornice")) as f64);
    let detail_on = crate::typology::detail_enabled();
    let profile = if random.next() < 0.62 { "ogee" } else { "stepped" };
    let height = 0.46 + random.next() * 0.2 + if detail_on { 0.08 } else { 0.0 };
    let brackets = random.next() < if detail_on { 0.90 } else { 0.72 };
    let lift = if plan.base_y.is_finite() { plan.base_y } else { 0.0 };
    let wall_top = plan.envelope.wall_top + lift;
    let parapet_top = wall_top + spec.at("parapet").get_nn("height").map(|v| to_number(Some(v))).unwrap_or(0.7);
    let y0 = (wall_top + 0.12).min(parapet_top - height - 0.02);
    if !(y0 > wall_top - 0.4) {
        return;
    }
    let front = frame.edge_for_role("front");
    let oriel_spans: Vec<(f64, f64)> = plan
        .parts
        .iter()
        .filter(|p| p.kind == "bay" && front.is_some() && p.edge == Some(front.unwrap().index))
        .map(|p| (p.s0 - 0.12, p.s1 + 0.12))
        .collect();
    let cmeta = json!({ "archetype": kind, "part": "crowning-cornice", "version": BAY_WINDOW_CORNICE_VERSION });
    for edge in frame.edges.iter() {
        if edge.party || !(edge.length >= CORNICE_MIN_EDGE_M) || !(edge.role == "front" || edge.role == "front-secondary") {
            continue;
        }
        let allowed = if edge.allowance.is_finite() { edge.allowance } else { DEFAULT_BAY_ALLOWANCE_M };
        let a = frame.to_plan(edge.start.x, edge.start.z);
        let b = frame.to_plan(edge.start.x + edge.u.x, edge.start.z + edge.u.z);
        let skew = if edge.convex_start || edge.convex_end { (b.x - a.x).abs() + (b.z - a.z).abs() } else { 1.0 };
        let max_p: f64 = if detail_on { 0.72 } else { 0.62 };
        let p = 0.12f64.max(max_p.min((allowed - 0.02) / 1f64.max(skew)));
        if !(p <= allowed - 0.02) {
            continue;
        }
        let ext0 = if edge.convex_start { p } else { 0.0 };
        let ext1 = if edge.convex_end { p } else { 0.0 };
        let mut frieze = cmeta.clone();
        set(&mut frieze, "part", json!("frieze"));
        plan.add_box(
            &edge.basis,
            PartArgs::new("string-course", "trim", -(0.04f64.min(ext0)), edge.length + 0.04f64.min(ext1), wall_top - 0.34, y0, -0.01, if detail_on { 0.07f64.min(p) } else { 0.04f64.min(p) })
                .edge(edge.index)
                .meta(frieze),
        );
        plan.add_profile(
            &edge.basis,
            PartArgs::profile("cornice", "trim", -ext0, edge.length + ext1, trim_profile_d(profile, p, height, y0)).edge(edge.index).meta(cmeta.clone()),
        );
        if !brackets {
            continue;
        }
        let is_front = front.map(|f| f.index == edge.index).unwrap_or(false);
        let count = ((edge.length - 0.3) / MODILLION_PITCH_M).floor();
        let start = (edge.length - count * MODILLION_PITCH_M) / 2.0;
        let mut i = 0.0;
        while i <= count {
            let s = start + i * MODILLION_PITCH_M;
            i += 1.0;
            if s < 0.15 || s > edge.length - 0.15 {
                continue;
            }
            if is_front && oriel_spans.iter().any(|(a, b)| s > *a && s < *b) {
                continue;
            }
            let mut mm = cmeta.clone();
            set(&mut mm, "part", json!("modillion"));
            let mh = if detail_on { 0.34 } else { 0.26 };
            let md = p * if detail_on { 0.80 } else { 0.72 };
            plan.add_box(
                &edge.basis,
                PartArgs::new("cornice-bracket", "trim", s - 0.065, s + 0.065, y0 - mh, y0, -0.01, md)
                    .edge(edge.index)
                    .shadow(detail_on)
                    .meta(mm),
            );
        }
    }
}

pub fn detail(plan: &mut Plan, frame: &Frame, spec: &J) {
    add_bay_projections(plan, frame, spec);
    add_bay_window_cornice(plan, frame, spec);
    // Optional pitched tile/shingle gable roof (inscribed strictly inside the footprint) or perimeter
    // mansard parapet roof cap + masonry chimney on hillside residential apartments
    if let Some(meta) = spec.get_nn("archetype") {
        let lift = if plan.base_y.is_finite() { plan.base_y } else { 0.0 };
        let y0 = plan.envelope.wall_top + lift;
        if meta.truthy("pitched") {
            let rise = meta.num("gableRise");
            if rise > 0.8 {
                if let Some(front) = frame.edge_for_role("front") {
                    let basis = front.basis;
                    let l = front.length;
                    let [ox, oz] = basis.origin;
                    let [ux, uz] = basis.u;
                    let [nx, nz] = basis.n;
                    let max_d = frame.polygon.iter().map(|p| -((p.x - ox) * nx + (p.z - oz) * nz)).fold(0.0f64, f64::max);
                    // Step depth inward until rear corners stay inside non-rectangular/notched footprints (zero floating overhangs)
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
                    let inset = 0.22;
                    if depth > 2.6 {
                        plan.add_plate(
                            &basis,
                            PartArgs {
                                kind: "roof-gable".into(),
                                slot: "pitchedRoof".into(),
                                edge: Some(front.index),
                                profile: vec![[-0.14, y0 - 0.08], [l + 0.14, y0 - 0.08], [l * 0.5, y0 + rise]],
                                w0: -depth + inset,
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
                        let ch_w = -depth * 0.44;
                        let ch_s = l * 0.72;
                        let wx = ox + ux * ch_s + nx * ch_w;
                        let wz = oz + uz * ch_s + nz * ch_w;
                        if frame.signed_distance(wx, wz) < -0.55 {
                            let ch_top = y0 + rise + 0.45;
                            plan.add_box(
                                &basis,
                                PartArgs::new("chimney", "trim", ch_s - 0.34, ch_s + 0.34, y0, ch_top, ch_w - 0.28, ch_w + 0.28)
                                    .edge(front.index)
                                    .shadow(true)
                                    .receive(true)
                                    .meta(json!({ "archetype": KIND, "part": "chimney", "roof": true })),
                            );
                        }
                        if let Some(a) = plan.extra.get_mut("archetype").and_then(|v| v.as_object_mut()) {
                            a.insert("pitched".into(), json!(true));
                        } else {
                            plan.extra.insert("archetype".into(), json!({ "kind": KIND, "pitched": true }));
                        }
                    }
                }
            }
        } else if meta.truthy("mansard") {
            // Terracotta / shingle pitched mansard parapet skirt along street-facing rooflines + brick chimney
            let mh = 1.45;
            let md = 1.15;
            let prof = vec![[0.08, y0 - 0.04], [-md, y0 + mh], [-md - 0.16, y0 + mh], [-md - 0.16, y0 - 0.04]];
            for edge in &frame.edges {
                if edge.party || !(edge.role == "front" || edge.role == "front-secondary") || edge.length < 3.5 {
                    continue;
                }
                let [ox, oz] = edge.basis.origin;
                let [ux, uz] = edge.basis.u;
                let [nx, nz] = edge.basis.n;
                let mx = ox + ux * (edge.length * 0.5) - nx * (md + 0.2);
                let mz = oz + uz * (edge.length * 0.5) - nz * (md + 0.2);
                if frame.signed_distance(mx, mz) > -0.15 {
                    continue;
                }
                plan.add_profile(
                    &edge.basis,
                    PartArgs::profile("roof-gable", "pitchedRoof", 0.05, edge.length - 0.05, prof.clone())
                        .edge(edge.index)
                        .shadow(true)
                        .receive(true)
                        .meta(json!({ "archetype": KIND, "part": "mansard-skirt", "roof": true })),
                );
            }
            if let Some(front) = frame.edge_for_role("front") {
                let [ox, oz] = front.basis.origin;
                let [ux, uz] = front.basis.u;
                let [nx, nz] = front.basis.n;
                let ch_s = front.length * 0.24;
                let ch_w = -2.2;
                let wx = ox + ux * ch_s + nx * ch_w;
                let wz = oz + uz * ch_s + nz * ch_w;
                if frame.signed_distance(wx, wz) < -0.6 {
                    plan.add_box(
                        &front.basis,
                        PartArgs::new("chimney", "trim", ch_s - 0.36, ch_s + 0.36, y0, y0 + mh + 0.9, ch_w - 0.3, ch_w + 0.3)
                            .edge(front.index)
                            .shadow(true)
                            .receive(true)
                            .meta(json!({ "archetype": KIND, "part": "chimney", "roof": true })),
                    );
                }
            }
        }
    }
}
