//! The South of Market masonry loft / warehouse
//! (src/world/buildings/archetypes/soma-warehouse.js): a mechanical port of
//! `somaWarehouseSpec` and `addSomaWarehouseDetail`. Nothing is left out
//! (`SOMA_WAREHOUSE_LOD_DROP_ORDER` is documentation and is not used by the planner).

use crate::classify::Classified;
use crate::elevation::{PartArgs, Plan};
use crate::frame::Frame;
use crate::js::*;
use crate::rng::Rng;

pub const SOMA_WAREHOUSE_VERSION: &str = "sf-archetype-soma-warehouse-v1";

const GROUND_STOREY_M: f64 = 4.4;
const UPPER_STOREY_M: f64 = 3.7;
const PARAPET_M: f64 = 0.9;
const COPING_M: f64 = 0.14;
const BAY_TARGET_MIN: f64 = 5.8;
const BAY_TARGET_MAX: f64 = 7.0;
const END_PIER_M: f64 = 0.95;
const ROLLER_MIN_BAY_M: f64 = 3.3;
const WALL_FINISHES: [(&str, f64); 4] = [
    ("brick-running-bond", 0.46),
    ("brick-common-bond", 0.34),
    ("concrete-precast", 0.15),
    ("corrugated-metal", 0.05),
];

fn end_pier_for(length: f64) -> f64 {
    END_PIER_M.min(0.05f64.max(length * 0.12))
}

fn pick_wall(random: &mut Rng) -> &'static str {
    let mut roll = random.next();
    for (name, weight) in WALL_FINISHES {
        roll -= weight;
        if roll <= 0.0 {
            return name;
        }
    }
    WALL_FINISHES[0].0
}

/// `storeyStack(height)`: (height, use) per storey.
fn storey_stack(height: f64) -> Vec<(f64, &'static str)> {
    let usable = GROUND_STOREY_M.max(height - PARAPET_M - COPING_M);
    let remainder = usable - GROUND_STOREY_M;
    let mut upper_count = 0f64.max(js_round(remainder / UPPER_STOREY_M));
    while upper_count > 0.0 && remainder / upper_count < 2.4 {
        upper_count -= 1.0;
    }
    if upper_count == 0.0 {
        return vec![(usable, "ground")];
    }
    let upper = remainder / upper_count;
    let mut storeys = vec![(GROUND_STOREY_M, "ground")];
    for _ in 0..upper_count as usize {
        storeys.push((upper, "loft"));
    }
    storeys
}

fn bay_count_for(length: f64, target_width: f64) -> f64 {
    let available = 0f64.max(length - 2.0 * end_pier_for(length));
    1f64.max(js_round(available / target_width))
}

/// `groundPlan(bayCount, bayWidth, hasRetailGround, random)`: (types, entryBay, rollerCount).
fn ground_plan(bay_count: usize, bay_width: f64, has_retail_ground: bool, random: &mut Rng) -> (Vec<&'static str>, usize, usize) {
    let mut types = vec![if has_retail_ground { "storefront" } else { "punched" }; bay_count];
    let entry_bay = (bay_count - 1).min((random.next() * bay_count as f64).floor() as usize);
    types[entry_bay] = "entrance";
    let can_roll = bay_width >= ROLLER_MIN_BAY_M;
    let roller_target = if !can_roll || bay_count < 2 {
        0
    } else if has_retail_ground {
        if random.next() < 0.34 { 1 } else { 0 }
    } else if bay_count >= 4 && random.next() < 0.55 {
        2
    } else {
        1
    };
    let mut placed = 0;
    let from_end = random.next() < 0.5;
    let mut step = 0;
    while step < bay_count && placed < roller_target {
        let i = if from_end { bay_count - 1 - step } else { step };
        step += 1;
        if types[i] == "entrance" {
            continue;
        }
        types[i] = "roller";
        placed += 1;
    }
    (types, entry_bay, placed)
}

/// `somaWarehouseSpec(classified, frame)`.
pub fn spec(c: &Classified, frame: &Frame) -> J {
    let mut random = Rng::from_num(c.seed as f64);
    let storeys = storey_stack(c.height);
    let wall = pick_wall(&mut random);
    let bay_target = BAY_TARGET_MIN + random.next() * (BAY_TARGET_MAX - BAY_TARGET_MIN);

    let front_edge = frame.edge_for_role("front");
    let mut front_length = front_edge.map(|e| e.length).unwrap_or(0.0);
    if front_length == 0.0 || front_length.is_nan() {
        front_length = c.frontage;
    }
    if front_length == 0.0 || front_length.is_nan() {
        front_length = 18.0;
    }
    let front_end_pier = end_pier_for(front_length);
    let front_bays = bay_count_for(front_length, bay_target);
    let front_bay_width = 0.05f64.max(front_length - 2.0 * front_end_pier) / front_bays;
    let (types, entry_bay, roller_count) = ground_plan(front_bays as usize, front_bay_width, c.has_retail_ground, &mut random);
    let upper_height = if storeys.len() > 1 { storeys[1].0 } else { storeys[0].0 };
    let sash_clear = 0.6f64.max(upper_height - 0.8 - 0.55);

    let back_edges = frame.edges_for_role("back");
    let longest_back = back_edges.iter().copied().fold(None, |best: Option<&crate::frame::Edge>, e| match best {
        Some(b) if !(e.length > b.length) => Some(b),
        _ => Some(e),
    });
    let alley = longest_back.filter(|e| e.length >= 9.0);

    let wants_fire_escape = storeys.len() >= 3 && random.next() < 0.62;
    let fire_escape_at = front_length * (0.3 + random.next() * 0.4);
    let masonry = wall == "brick-running-bond" || wall == "brick-common-bond";

    let bays: Vec<J> = types.iter().map(|t| json!({ "width": front_bay_width, "storeys": { "0": t } })).collect();
    let transom_at = json!([sash_clear / 3.0, (sash_clear * 2.0) / 3.0]);
    let detail_on = crate::typology::detail_enabled();
    let mut front = json!({
        "bays": bays,
        "endPier": front_end_pier,
        "window": "punched",
        "reveal": if detail_on { 0.26 } else { 0.16 },
        "windowSpec": {
            "sillHeight": 0.8, "headClear": 0.55, "widthRatio": 0.70, "frameWidth": 0.06, "frameDepth": 0.09,
            "mullions": 2, "transomAt": transom_at,
        },
        "sill": { "height": 0.12, "projection": if detail_on { 0.11 } else { 0.07 }, "overhang": 0.11, "profile": "sloped" },
        "pilasters": { "width": 0.7, "projection": if detail_on { 0.18 } else { 0.1 }, "ends": true },
        "cornice": { "height": if detail_on { 0.62 } else { 0.55 }, "projection": if detail_on { 0.60 } else { 0.34 }, "profile": "stepped" },
        "base": { "height": 0.65, "projection": if detail_on { 0.08 } else { 0.05 } },
        "storefrontSpec": { "stallRiser": 0.45, "pier": 0.2, "mullionSpacing": 1.8, "transom": 0.45 },
        "entranceSpec": { "width": 1.9, "height": 2.7, "recess": 0.45, "leaf": "double-glass" },
        "rollerSpec": { "width": 4.6, "height": 4.0, "inset": 0.14, "hood": 0.4, "slat": 0.45 },
        "downpipes": "corners",
    });
    if c.has_retail_ground {
        set(&mut front, "signband", json!({ "height": 0.6, "projection": 0.09, "offset": 0.12 }));
    }
    if wants_fire_escape {
        let fs: Vec<usize> = (1..storeys.len()).collect();
        set(&mut front, "fireEscape", json!({ "storeys": fs, "at": fire_escape_at, "width": 2.6, "projection": 1.1 }));
    }

    let secondary = json!({
        "bayWidth": bay_target * 1.05,
        "endPier": END_PIER_M,
        "window": "punched",
        "ground": "punched",
        "reveal": if detail_on { 0.24 } else { 0.16 },
        "windowSpec": {
            "sillHeight": 0.8, "headClear": 0.55, "widthRatio": 0.66, "frameWidth": 0.06, "frameDepth": 0.09,
            "mullions": 2, "transomAt": transom_at,
        },
        "sill": { "height": 0.12, "projection": if detail_on { 0.10 } else { 0.07 }, "overhang": 0.1, "profile": "sloped" },
        "pilasters": { "width": 0.6, "projection": if detail_on { 0.14 } else { 0.08 }, "ends": false },
        "base": { "height": 0.65, "projection": 0.05 },
        "cornice": { "height": if detail_on { 0.56 } else { 0.5 }, "projection": if detail_on { 0.48 } else { 0.3 }, "profile": "stepped" },
    });

    let mut elevations = json!({
        "front": front,
        "front-secondary": secondary,
        "left": "blind",
        "right": "blind",
        "back": "blind",
        "default": "blind",
    });
    if let Some(alley) = alley {
        let rear = json!({
            "bayWidth": bay_target * 2.6,
            "endPier": END_PIER_M,
            "window": "punched",
            "ground": "blind",
            "windowSpec": { "sillHeight": 0.95, "headClear": 0.7, "widthRatio": 0.5, "frameWidth": 0.06, "frameDepth": 0.08, "mullions": 0, "transoms": 0 },
            "sill": false,
        });
        let mut m = Map::new();
        m.insert(alley.index.to_string(), rear);
        set(&mut elevations, "byEdge", J::Object(m));
    }

    let storeys_json: Vec<J> = storeys.iter().map(|(h, u)| json!({ "height": h, "use": u })).collect();
    json!({
        "id": format!("sf-archetype-{}", c.seed),
        "seed": c.seed,
        "height": jn(c.height),
        "storeys": storeys_json,
        "wallThickness": 0.4,
        "parapet": { "height": PARAPET_M, "thickness": 0.34 },
        "coping": { "height": COPING_M, "overhang": 0.08 },
        "materials": {
            "wall": wall,
            "parapet": wall,
            "base": "cast-stone",
            "trim": "cast-stone-trim",
            "coping": "cast-stone",
            "spandrel": "concrete-precast",
            "glass": "glass-clear",
            "frame": "painted-steel",
            "roller": "painted-steel-light",
            "metal": "painted-steel-mid",
            "storefront": "anodized-aluminium",
            "roof": "asphalt-roof",
        },
        "elevations": elevations,
        "roof": { "slab": 0.32 },
        "archetype": {
            "kind": "soma-warehouse",
            "version": SOMA_WAREHOUSE_VERSION,
            "wall": wall,
            "masonry": masonry,
            "bayTarget": bay_target,
            "frontBays": front_bays,
            "frontBayWidth": front_bay_width,
            "frontEndPier": front_end_pier,
            "groundTypes": types,
            "rollerCount": roller_count,
            "entryBay": entry_bay,
            "fireEscape": wants_fire_escape,
            "alleyEdge": alley.map(|a| json!(a.index)).unwrap_or(J::Null),
        },
    })
}

/// `addSomaWarehouseDetail(plan, frame, spec)`.
pub fn detail(plan: &mut Plan, frame: &Frame, spec: &J) {
    let Some(meta) = spec.get_nn("archetype") else { return };
    let Some(front) = frame.edge_for_role("front") else { return };
    let Some(edge_record) = plan.edges.iter().find(|e| e.index == front.index) else { return };
    let bays = edge_record.bays.clone();
    let base_y = if plan.base_y.is_nan() { 0.0 } else { plan.base_y };
    let wall_top = plan.envelope.wall_top + base_y;
    let ground = plan.storeys.first().cloned();
    let allowance = 0.34f64.min(front.allowance);
    let kind = meta.s("kind").unwrap_or("").to_string();

    if meta.truthy("masonry") && !bays.is_empty() {
        let cornice_height = 0.55;
        let y1 = wall_top - cornice_height;
        let end_pier = meta.get_nn("frontEndPier").map(|v| to_number(Some(v))).unwrap_or(END_PIER_M);
        let mut piers = vec![end_pier];
        for bay in bays.iter().skip(1) {
            piers.push(bay.s0);
        }
        piers.push(front.length - end_pier);
        for s in piers {
            plan.add_box(
                &front.basis,
                PartArgs::new("corbel", "wall", s - 0.34, s + 0.34, y1 - 0.30, y1 - 0.15, -0.02, 0.12f64.min(allowance))
                    .edge(front.index)
                    .meta(json!({ "archetype": kind, "course": 0 })),
            );
            plan.add_box(
                &front.basis,
                PartArgs::new("corbel", "wall", s - 0.44, s + 0.44, y1 - 0.15, y1, -0.02, 0.22f64.min(allowance))
                    .edge(front.index)
                    .meta(json!({ "archetype": kind, "course": 1 })),
            );
        }
    }

    let ground_types: Vec<J> = meta.arr("groundTypes").cloned().unwrap_or_default();
    for bay in &bays {
        if ground_types.get(bay.index).and_then(|v| v.as_str()) != Some("roller") {
            continue;
        }
        let centre = (bay.s0 + bay.s1) / 2.0;
        let width = 4.6f64.min(bay.width - 0.4);
        if !(width > 1.5) {
            continue;
        }
        // JS reads `ground.y0` here; planElevation always yields a ground storey.
        let Some(ground) = ground.as_ref() else { return };
        let s0 = centre - width / 2.0;
        let s1 = centre + width / 2.0;
        let head = (ground.y0 + 4.0).min(ground.y1 - 0.4) + base_y;
        plan.add_box(
            &front.basis,
            PartArgs::new("dock-lintel", "metal", s0 - 0.16, s1 + 0.16, head, head + 0.22, -0.02, 0.1f64.min(allowance))
                .edge(front.index)
                .meta(json!({ "archetype": kind })),
        );
        for s in [s0 - 0.14, s1 + 0.02] {
            plan.add_box(
                &front.basis,
                PartArgs::new(
                    "dock-bumper",
                    "metal",
                    s,
                    s + 0.12,
                    ground.y0 + base_y + 0.9,
                    ground.y0 + base_y + 1.32,
                    -0.01,
                    0.14f64.min(allowance),
                )
                .edge(front.index)
                .shadow(false)
                .meta(json!({ "archetype": kind })),
            );
        }
    }

    if crate::typology::detail_enabled() {
        let mut street_edges = vec![front];
        street_edges.extend(frame.edges_for_role("front-secondary"));
        for edge in street_edges {
            if edge.party || edge.length < 2.0 {
                continue;
            }
            let allow_e = 0.24f64.min(0.05f64.max(edge.allowance - 0.04));
            let (ops, mut cuts): (Vec<(f64, f64, f64)>, Vec<(f64, f64)>) = {
                let Some(rec) = plan.edges.iter().find(|e| e.index == edge.index) else { continue };
                let ops = if meta.truthy("masonry") {
                    rec.openings
                        .iter()
                        .filter(|o| o.type_ == "punched" && o.s1 - o.s0 >= 0.7)
                        .map(|o| (o.s0, o.s1, o.y1))
                        .collect()
                } else {
                    vec![]
                };
                let cuts = rec
                    .openings
                    .iter()
                    .filter(|o| o.storey == 0 && matches!(o.type_.as_str(), "roller" | "entrance" | "storefront"))
                    .map(|o| (o.s0 - 0.08, o.s1 + 0.08))
                    .collect();
                (ops, cuts)
            };
            // Cast-stone flat-arch window lintel headers & subtle keystones over loft windows
            for (s0, s1, y1) in ops {
                let y_h0 = y1 + base_y - 0.01;
                let y_h1 = (y_h0 + 0.22).min(wall_top - 0.56);
                if y_h1 - y_h0 > 0.1 {
                    plan.add_box(
                        &edge.basis,
                        PartArgs::new("window-head", "trim", s0 - 0.14, s1 + 0.14, y_h0, y_h1, -0.02, 0.08f64.min(allow_e))
                            .edge(edge.index)
                            .meta(json!({ "archetype": kind, "part": "loft-lintel" })),
                    );
                }
            }
            // Heavy cast-stone water-table plinth belt along wall base outside openings
            cuts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            cuts.push((edge.length, edge.length));
            let mut cur = 0.0;
            for (a, b) in cuts {
                let end = a.min(edge.length);
                if end - cur > 0.3 {
                    plan.add_box(
                        &edge.basis,
                        PartArgs::new("water-table", "base", cur, end, base_y - 0.02, base_y + 0.24, -0.02, 0.075f64.min(allow_e))
                            .edge(edge.index)
                            .meta(json!({ "archetype": kind, "part": "water-table" })),
                    );
                }
                cur = cur.max(b);
            }
        }
    }
}
