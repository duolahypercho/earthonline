//! The San Francisco midcentury stucco block
//! (src/world/buildings/archetypes/midcentury-stucco.js): a mechanical port of
//! `midcenturyStuccoSpec` and `addMidcenturyStuccoDetail`. Nothing is left out.

use crate::classify::Classified;
use crate::elevation::{trim_profile_d, PartArgs, Plan};
use crate::frame::Frame;
use crate::js::*;
use crate::rng::Rng;

pub const MIDCENTURY_STUCCO_VERSION: &str = "sf-archetype-midcentury-stucco-v2";

const GROUND_STOREY_M: f64 = 3.4;
const UPPER_STOREY_M: f64 = 3.0;
const MIN_STOREY_M: f64 = 2.35;
const PARAPET_M: f64 = 0.45;
const COPING_M: f64 = 0.08;
const BAY_MODULE_M: f64 = 5.5;
const ENTRY_BAY_M: f64 = 2.3;
const GARAGE_BAY_M: f64 = 3.2;
const MIN_GLAZED_EDGE_M: f64 = 9.0;
const WEST_FLATS_DISTRICTS: [&str; 13] = [
    "Sunset/Parkside", "Inner Sunset", "Outer Richmond", "Inner Richmond", "Marina", "Seacliff",
    "West of Twin Peaks", "Lakeshore", "Excelsior", "Outer Mission", "Oceanview/Merced/Ingleside", "Visitacion Valley", "Portola",
];
const WEST_FLATS_VARIANTS: [&str; 3] = ["apartment", "richmond-special", "flats"];
const GARAGE_ROW_BAY_M: f64 = 3.0;
const WALL_FINISHES: [&str; 4] = ["painted-stucco", "float-finish-stucco", "scanned-painted-plaster", "painted-stucco"];

fn window_wall() -> J {
    if crate::typology::detail_enabled() {
        json!({ "spandrelHeight": 0.84, "headClear": 0.34, "mullionSpacing": 1.85, "frameWidth": 0.11, "frameDepth": 0.16 })
    } else {
        json!({ "spandrelHeight": 1.05, "headClear": 0.45, "mullionSpacing": 3.2, "frameWidth": 0.2, "frameDepth": 0.11 })
    }
}

/// `storeyStack(height)`: storey heights (the parapet is the constant).
fn storey_stack(height: f64) -> Vec<f64> {
    let usable = MIN_STOREY_M.max(height - PARAPET_M - COPING_M);
    if usable < GROUND_STOREY_M + MIN_STOREY_M {
        return vec![usable];
    }
    let mut upper_count = 1f64.max(js_round((usable - GROUND_STOREY_M) / UPPER_STOREY_M));
    while upper_count > 1.0 && (usable - GROUND_STOREY_M) / upper_count < MIN_STOREY_M {
        upper_count -= 1.0;
    }
    let upper_height = (usable - GROUND_STOREY_M) / upper_count;
    let mut storeys = vec![GROUND_STOREY_M];
    for _ in 0..upper_count as usize {
        storeys.push(upper_height);
    }
    storeys
}

struct GroundBays {
    widths: Vec<f64>,
    entry_bay: i64,
    garage_bay: i64,
    garage_bays: Option<Vec<i64>>,
}

/// `groundBays(length, wantEntry, wantGarage)`.
fn ground_bays(length: f64, want_entry: bool, want_garage: bool) -> GroundBays {
    let fixed = (if want_entry { ENTRY_BAY_M } else { 0.0 }) + (if want_garage { GARAGE_BAY_M } else { 0.0 });
    let rest = (length - fixed).max(0.0);
    let plain = if rest > 1.2 { 1f64.max(js_round(rest / BAY_MODULE_M)) as usize } else { 0 };
    let mut widths = vec![];
    let mut entry_bay = -1;
    let mut garage_bay = -1;
    if want_entry {
        entry_bay = widths.len() as i64;
        widths.push(ENTRY_BAY_M);
    }
    for _ in 0..plain {
        widths.push(rest / plain as f64);
    }
    if want_garage {
        garage_bay = widths.len() as i64;
        widths.push(GARAGE_BAY_M);
    }
    if widths.is_empty() {
        widths.push(length);
    }
    let mut total = widths.iter().fold(0.0, |a, b| a + b);
    if total == 0.0 || total.is_nan() {
        total = 1.0;
    }
    GroundBays { widths: widths.iter().map(|w| (length * w) / total).collect(), entry_bay, garage_bay, garage_bays: None }
}

/// `garageRowBays(length)`.
fn garage_row_bays(length: f64) -> GroundBays {
    let doors = 1f64.max(((length - ENTRY_BAY_M) / GARAGE_ROW_BAY_M).floor()) as usize;
    let garage_width = (length - ENTRY_BAY_M) / doors as f64;
    let mut widths = vec![ENTRY_BAY_M];
    widths.extend(std::iter::repeat(garage_width).take(doors));
    let garage_bays: Vec<i64> = (1..widths.len() as i64).collect();
    GroundBays { widths, entry_bay: 0, garage_bay: garage_bays[0], garage_bays: Some(garage_bays) }
}

/// `windowWallElevation(ground)`.
fn window_wall_elevation(ground: &str) -> J {
    json!({
        "window": "ribbon",
        "ground": ground,
        "endPier": 0,
        "sill": false,
        "ribbonSpec": window_wall(),
        "windowSpec": { "widthRatio": 0.5, "sillHeight": 0.95, "headClear": 0.5, "frameWidth": 0.07, "frameDepth": 0.08, "mullions": 0 },
        "storefrontSpec": { "stallRiser": 0.45, "mullionSpacing": 2.6, "transom": 0, "pier": 0.2, "headClear": 0.6, "frameWidth": 0.09, "frameDepth": 0.12 },
    })
}

/// `midcenturyStuccoSpec(classified, frame)`.
pub fn spec(c: &Classified, frame: &Frame) -> J {
    let mut random = Rng::from_num(c.seed as f64);
    let storeys = storey_stack(c.height);
    let parapet = PARAPET_M;
    let wall = WALL_FINISHES[(random.next() * WALL_FINISHES.len() as f64).floor() as usize];

    let front_edge = frame.edge_for_role("front");
    let mut front_length = front_edge.map(|e| e.length).unwrap_or(0.0);
    if front_length == 0.0 || front_length.is_nan() {
        front_length = frame.edges.iter().fold(0.0, |longest: f64, e| longest.max(e.length));
    }
    if front_length == 0.0 || front_length.is_nan() {
        front_length = 12.0;
    }

    let retail = c.has_retail_ground;
    let ground_default = if retail { "storefront" } else { "punched" };

    let want_garage = !retail && front_length >= ENTRY_BAY_M + GARAGE_BAY_M + 1.2;
    let district = c.civic.as_ref().and_then(|v| v.district.as_deref());
    let west_flats = !retail
        && storeys.len() >= 3
        && WEST_FLATS_VARIANTS.contains(&c.variant.as_str())
        && district.map(|d| WEST_FLATS_DISTRICTS.contains(&d)).unwrap_or(false)
        && front_length >= ENTRY_BAY_M + GARAGE_ROW_BAY_M;
    let gb = if west_flats { garage_row_bays(front_length) } else { ground_bays(front_length, true, want_garage) };
    let flip = random.next() < 0.45;
    let order: Vec<f64> = if flip { gb.widths.iter().rev().cloned().collect() } else { gb.widths.clone() };
    let bay_count = order.len() as i64;
    let at = |i: i64| if flip { bay_count - 1 - i } else { i };
    let front_bays: Vec<J> = order
        .iter()
        .enumerate()
        .map(|(i, &width)| {
            let a = at(i as i64);
            let use_ = if a == gb.entry_bay {
                "entrance"
            } else if a == gb.garage_bay || gb.garage_bays.as_ref().map(|g| g.contains(&a)).unwrap_or(false) {
                "roller"
            } else {
                ground_default
            };
            json!({ "width": width, "storeys": [use_] })
        })
        .collect();
    let front_entry_bay = if flip { bay_count - 1 - gb.entry_bay } else { gb.entry_bay };
    let front_garage_bay = if gb.garage_bay < 0 { -1 } else if flip { bay_count - 1 - gb.garage_bay } else { gb.garage_bay };

    let min_sec_edge = if crate::typology::detail_enabled() { 5.5 } else { MIN_GLAZED_EDGE_M };
    let mut by_edge = Map::new();
    for edge in &frame.edges {
        if edge.role != "front-secondary" {
            continue;
        }
        let entry = if edge.length >= min_sec_edge {
            let mut e = window_wall_elevation(if ground_default == "storefront" { "storefront" } else { "punched" });
            set(&mut e, "bays", json!({ "count": 1f64.max(js_round(edge.length / BAY_MODULE_M)) }));
            set(&mut e, "endPier", json!(0));
            e
        } else {
            J::String("blind".into())
        };
        by_edge.insert(edge.index.to_string(), entry);
    }

    let storeys_json: Vec<J> = storeys.iter().map(|h| json!({ "height": h })).collect();
    let storey_uses: Vec<J> = (0..storeys.len())
        .map(|i| J::String(if i == 0 { ground_default } else if west_flats { "punched" } else { "ribbon" }.into()))
        .collect();

    let mut front = window_wall_elevation(ground_default);
    set(&mut front, "bays", J::Array(front_bays));
    set(&mut front, "storeys", J::Array(storey_uses));
    if west_flats {
        set(
            &mut front,
            "windowSpec",
            json!({ "widthRatio": 0.66, "sillHeight": 0.9, "headClear": 0.45, "frameWidth": 0.06, "frameDepth": 0.07, "mullions": 1 }),
        );
    }
    set(&mut front, "entranceSpec", json!({ "width": 1.2, "height": 2.4, "recess": 0.55, "transom": 0, "leaf": "single-glass", "frameWidth": 0.07 }));
    set(
        &mut front,
        "rollerSpec",
        if west_flats {
            json!({ "width": GARAGE_ROW_BAY_M - 0.55, "height": 2.3, "inset": 0.1, "hood": 0, "slat": 0.5 })
        } else {
            json!({ "width": 4.6, "height": 2.4, "inset": 0.1, "hood": 0.35, "slat": 0.55 })
        },
    );

    let eyebrow = random.next() < 0.74;
    json!({
        "id": format!("sf-archetype-{}", c.seed),
        "seed": c.seed,
        "height": jn(c.height),
        "storeys": storeys_json,
        "parapet": { "height": parapet, "thickness": 0.25 },
        "coping": { "height": COPING_M, "overhang": 0.05 },
        "materials": {
            "wall": wall,
            "parapet": wall,
            "spandrel": wall,
            "base": "concrete-precast",
            "trim": "cast-stone-trim",
            "coping": "cast-stone",
            "glass": "glass-clear",
            "frame": "anodized-aluminium",
            "storefront": "anodized-aluminium",
            "door": "anodized-aluminium",
            "doorGlass": "glass-door",
            "recess": "painted-stucco",
            "canopy": "concrete-precast",
            "roof": "asphalt-roof",
        },
        "elevations": {
            "front": front,
            "left": "blind",
            "right": "blind",
            "back": "blind",
            "front-secondary": "blind",
            "default": "blind",
            "byEdge": J::Object(by_edge),
        },
        "archetype": {
            "kind": "midcentury-stucco",
            "version": MIDCENTURY_STUCCO_VERSION,
            "bayCount": bay_count,
            "entryBay": front_entry_bay,
            "garageBay": front_garage_bay,
            "retail": retail,
            "westFlats": west_flats,
            "wall": wall,
            "eyebrow": eyebrow,
        },
    })
}

/// `addMidcenturyStuccoDetail(plan, frame, spec)`.
pub fn detail(plan: &mut Plan, frame: &Frame, spec: &J) {
    let Some(meta) = spec.get_nn("archetype") else { return };
    let Some(front) = frame.edge_for_role("front") else { return };
    let edge_pos = plan.edges.iter().position(|e| e.index == front.index);
    let bays = edge_pos.map(|p| plan.edges[p].bays.clone()).unwrap_or_default();
    let Some(ground) = plan.storeys.first().cloned() else { return };
    let kind = meta.s("kind").unwrap_or("").to_string();

    let bay_at = |i: f64| -> Option<crate::elevation::BayRecord> {
        if i >= 0.0 && i.fract() == 0.0 { bays.get(i as usize).cloned() } else { None }
    };
    let entry = bay_at(meta.num("entryBay"));
    let head = (ground.y1 - 0.18).min(ground.y0 + 2.95);
    if let Some(entry) = &entry {
        if meta.truthy("eyebrow") {
            plan.add_box(
                &front.basis,
                PartArgs::new("canopy", "canopy", entry.s0 - 0.12, entry.s1 + 0.12, head, head + 0.16, -0.02, 0.85)
                    .edge(front.index)
                    .shadow(true)
                    .receive(true)
                    .meta(json!({ "archetype": kind, "part": "entry-eyebrow" })),
            );
        }
        for (s0, s1) in [(entry.s0 - 0.14, entry.s0), (entry.s1, entry.s1 + 0.14)] {
            plan.add_box(
                &front.basis,
                PartArgs::new("entry-jamb", "trim", s0, s1, ground.y0, head + 0.16, -0.01, 0.1)
                    .edge(front.index)
                    .meta(json!({ "archetype": kind, "part": "entry-jamb" })),
            );
        }
    }

    let garage_bay = meta.num("garageBay");
    let garage = if garage_bay >= 0.0 { bay_at(garage_bay) } else { None };
    if let Some(garage) = &garage {
        let lintel = (ground.y1 - 0.25).min(ground.y0 + 2.42);
        plan.add_profile(
            &front.basis,
            PartArgs::profile("garage-head", "trim", garage.s0 + 0.05, garage.s1 - 0.05, trim_profile_d("flat", 0.07, 0.14, lintel))
                .edge(front.index),
        );
    }

    if crate::typology::detail_enabled() {
        let base_y = if plan.base_y.is_finite() { plan.base_y } else { 0.0 };
        let wall_top = plan.envelope.wall_top + base_y;
        let is_retail = meta.truthy("retail");
        // Midcentury cantilevered roofline slab eyebrow, horizontal storey/window shadow ledges, and vertical bay divider fins
        for edge in frame.edges.iter() {
            if edge.party || !(edge.role == "front" || edge.role == "front-secondary") || edge.length < 3.0 {
                continue;
            }
            let allow = (edge.allowance - 0.02).max(0.08);
            let eave_p = 0.50f64.min(allow);
            let ext0 = if edge.convex_start { eave_p } else { 0.0 };
            let ext1 = if edge.convex_end { eave_p } else { 0.0 };
            plan.add_profile(
                &edge.basis,
                PartArgs::profile("cornice", "trim", -ext0, edge.length + ext1, trim_profile_d("stepped", eave_p, 0.28, wall_top - 0.24))
                    .edge(edge.index)
                    .shadow(true)
                    .receive(true)
                    .meta(json!({ "archetype": kind, "part": "midcentury-eave" })),
            );
            // Horizontal slab shadow ledges between upper storeys (skip storey 1 on retail fronts so
            // ground-floor storefront fascia sign boards & awnings stay 100% unobstructed in clutter.rs!)
            let ledge_p = 0.14f64.min(allow);
            let storey_info: Vec<(usize, f64, f64)> = plan.storeys.iter().enumerate().skip(1).map(|(i, st)| (i, st.y0 + base_y, st.y1 + base_y)).collect();
            for (si, sy0, sy1) in storey_info {
                let skip_fascia = is_retail && si == 1;
                if !skip_fascia && sy0 < wall_top - 0.6 {
                    plan.add_box(
                        &edge.basis,
                        PartArgs::new("string-course", "trim", -ledge_p * if edge.convex_start { 1.0 } else { 0.0 }, edge.length + ledge_p * if edge.convex_end { 1.0 } else { 0.0 }, sy0 - 0.08, sy0 + 0.07, -0.01, ledge_p)
                            .edge(edge.index)
                            .shadow(true)
                            .receive(true)
                            .meta(json!({ "archetype": kind, "part": "slab-ledge" })),
                    );
                }
                // Projecting midcentury window sill & upper drip eyebrow hood on residential blocks (`09` right-mid)
                if !is_retail && sy1 - sy0 >= 2.2 && sy1 < wall_top + 0.05 {
                    let sill_y = sy0 + 0.78;
                    let head_y = sy1 - 0.32;
                    if head_y > sill_y + 0.6 {
                        plan.add_box(
                            &edge.basis,
                            PartArgs::new("sill", "trim", 0.06, (edge.length - 0.06).max(0.1), sill_y - 0.08, sill_y + 0.02, -0.01, ledge_p)
                                .edge(edge.index)
                                .shadow(true)
                                .receive(true)
                                .meta(json!({ "archetype": kind, "part": "ribbon-sill" })),
                        );
                        plan.add_box(
                            &edge.basis,
                            PartArgs::new("window-head", "trim", 0.06, (edge.length - 0.06).max(0.1), head_y, head_y + 0.09, -0.01, (ledge_p * 1.15).min(allow))
                                .edge(edge.index)
                                .shadow(true)
                                .receive(true)
                                .meta(json!({ "archetype": kind, "part": "ribbon-head" })),
                        );
                    }
                }
            }
            // Crisp vertical bay-divider fin pilasters on upper street edges (above retail fascia band)
            let fin_s_opt: Option<Vec<f64>> = plan.edges.iter().find(|r| r.index == edge.index).map(|er| {
                let mut v: Vec<f64> = er.bays.iter().skip(1).map(|b| b.s0).collect();
                if edge.length >= 4.5 {
                    v.push(0.10);
                    v.push(edge.length - 0.10);
                }
                v
            });
            if let Some(fin_s) = fin_s_opt {
                let fin_p = 0.11f64.min(allow);
                let y0_fin = plan.storeys.get(1).map(|s| s.y0 + base_y + if is_retail { 0.45 } else { 0.0 }).unwrap_or(ground.y0 + base_y);
                if wall_top - 0.28 > y0_fin + 1.0 {
                    for s in fin_s {
                        plan.add_box(
                            &edge.basis,
                            PartArgs::new("pilaster", "trim", s - 0.075, s + 0.075, y0_fin, wall_top - 0.24, -0.01, fin_p)
                                .edge(edge.index)
                                .shadow(true)
                                .receive(true)
                                .meta(json!({ "archetype": kind, "part": "bay-fin" })),
                        );
                    }
                }
            }
        }
    }

    let [ox, oz] = front.basis.origin;
    let [ux, uz] = front.basis.u;
    let [nx, nz] = front.basis.n;
    let garages: Option<Vec<J>> = if meta.truthy("westFlats") {
        let ops = edge_pos.map(|p| plan.edges[p].openings.clone()).unwrap_or_default();
        Some(
            ops.iter()
                .filter(|op| op.type_ == "roller" && op.storey == 0)
                .map(|op| {
                    let c = (op.s0 + op.s1) / 2.0;
                    json!({ "edge": front.index, "width": jn(op.s1 - op.s0), "world": { "x": jn(ox + ux * c), "z": jn(oz + uz * c) }, "outward": [jn(nx), jn(nz)] })
                })
                .collect(),
        )
    } else {
        None
    };
    let garage_json = if let Some(g) = &garages {
        g.first().cloned().unwrap_or(J::Null)
    } else if let Some(garage) = &garage {
        let centre = (garage.s0 + garage.s1) / 2.0;
        json!({
            "edge": front.index,
            "width": jn(4.6f64.min(garage.width - 0.4)),
            "world": { "x": jn(ox + ux * centre), "z": jn(oz + uz * centre) },
            "outward": [jn(nx), jn(nz)],
        })
    } else {
        J::Null
    };
    let mut archetype = json!({
        "kind": meta.at("kind").cloned().unwrap_or(J::Null),
        "version": meta.at("version").cloned().unwrap_or(J::Null),
        "retail": meta.at("retail").cloned().unwrap_or(J::Null),
        "bayCount": meta.at("bayCount").cloned().unwrap_or(J::Null),
        "garage": garage_json,
    });
    if let Some(g) = garages {
        set(&mut archetype, "garages", J::Array(g));
    }
    plan.extra.insert("archetype".into(), archetype);
}
