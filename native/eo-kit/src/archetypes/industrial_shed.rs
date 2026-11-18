//! Port of src/world/buildings/archetypes/industrial-shed.js (the light-industrial
//! shed: one or two layout storeys, a blank street wall with a dock of roller
//! doors and a personnel door, a clerestory ribbon; the post-pass adds the dock
//! lintels, the painted name band and the seeded roof monitor).
//!
//! Not ported: INDUSTRIAL_SHED_LOD_DROP_ORDER (documentation table, no runtime use).

use crate::classify::Classified;
use crate::elevation::{trim_profile_d, PartArgs, Plan};
use crate::frame::Frame;
use crate::js::*;
use crate::rng::Rng;

pub const INDUSTRIAL_SHED_VERSION: &str = "sf-archetype-industrial-shed-v1";

const PARAPET_M: f64 = 0.55;
const COPING_M: f64 = 0.06;
const CLERESTORY_STOREY_M: f64 = 2.25;
const DOOR_STOREY_MIN_M: f64 = 3.0;
const MIN_WALL_M: f64 = 2.25;
const DOOR_BAY_M: f64 = 5.4;
const ENTRY_BAY_M: f64 = 1.9;
const MIN_DOOR_FRONT_M: f64 = 8.0;
const MIN_PROMOTED_RETURN_M: f64 = 14.0;
const WALL_THICKNESS_M: f64 = 0.2;

fn clerestory() -> J {
    json!({ "spandrelHeight": 0.42, "headClear": 0.33, "mullionSpacing": 6.0, "frameWidth": 0.12, "frameDepth": 0.09 })
}

/// (name, weight, metal)
const WALL_FINISHES: [(&str, f64, bool); 4] = [
    ("corrugated-metal", 0.40, true),
    ("concrete-precast", 0.31, false),
    ("standing-seam-metal", 0.19, true),
    ("painted-block", 0.10, false),
];
const BAND_PAINTS: [&str; 3] = ["painted-white", "signal-red", "painted-steel-mid"];

fn pick_wall(random: &mut Rng) -> (&'static str, f64, bool) {
    let mut roll = random.next();
    for entry in WALL_FINISHES {
        roll -= entry.1;
        if roll <= 0.0 {
            return entry;
        }
    }
    WALL_FINISHES[0]
}

fn end_pier_for(length: f64) -> f64 {
    0.9f64.min(0.05f64.max(length * 0.1))
}

fn storey_stack(height: f64) -> (Vec<(f64, &'static str)>, f64, bool) {
    let mut parapet = PARAPET_M;
    if height - parapet - COPING_M < MIN_WALL_M {
        parapet = 0.08f64.max(height - COPING_M - MIN_WALL_M);
    }
    let usable = MIN_WALL_M.max(height - parapet - COPING_M);
    if usable < DOOR_STOREY_MIN_M + CLERESTORY_STOREY_M {
        return (vec![(usable, "shed")], parapet, false);
    }
    (vec![(usable - CLERESTORY_STOREY_M, "shed"), (CLERESTORY_STOREY_M, "clerestory")], parapet, true)
}

fn door_count_for(length: f64) -> u32 {
    if length < MIN_DOOR_FRONT_M {
        0
    } else if length < 16.0 {
        1
    } else if length < 30.0 {
        2
    } else {
        3
    }
}

fn front_bays(length: f64, want_doors: u32, want_entry: bool, retail: bool) -> (Vec<f64>, Vec<&'static str>) {
    let available = 0.4f64.max(length - 2.0 * end_pier_for(length));
    let mut doors = want_doors;
    while doors > 0 && doors as f64 * DOOR_BAY_M + (if want_entry { ENTRY_BAY_M } else { 0.0 }) > available - 1.0 {
        doors -= 1;
    }
    let entry = want_entry && ENTRY_BAY_M <= available - 0.6;
    let fixed = doors as f64 * DOOR_BAY_M + if entry { ENTRY_BAY_M } else { 0.0 };
    let mut widths = vec![];
    let mut types = vec![];
    let mut push = |width: f64, t: &'static str| {
        if width > 1e-3 {
            widths.push(width);
            types.push(t);
        }
    };
    if fixed <= 0.0 {
        push(available, "blind");
        return (widths, types);
    }
    let slack = 0f64.max(available - fixed);
    push(slack * if retail { 0.35 } else { 0.55 }, "blind");
    if entry {
        push(ENTRY_BAY_M, "entrance");
    }
    for _ in 0..doors {
        push(DOOR_BAY_M, "roller");
    }
    push(slack * if retail { 0.65 } else { 0.45 }, "blind");
    let sum: f64 = widths.iter().fold(0.0, |a, b| a + b);
    let total = if sum == 0.0 || sum.is_nan() { 1.0 } else { sum };
    (widths.iter().map(|w| (available * w) / total).collect(), types)
}

pub fn spec(c: &Classified, frame: &Frame) -> J {
    let mut random = Rng::from_num(c.seed as f64);
    let (storeys, parapet, has_clerestory) = storey_stack(c.height);
    let cladding = pick_wall(&mut random);

    let mut front_length = frame.edge_for_role("front").map(|e| e.length).unwrap_or(0.0);
    if front_length == 0.0 || front_length.is_nan() {
        front_length = frame.edges.iter().fold(0.0, |l: f64, e| l.max(e.length));
    }
    if front_length == 0.0 || front_length.is_nan() {
        front_length = 12.0;
    }

    let retail = c.has_retail_ground;
    let (widths, types) = front_bays(front_length, door_count_for(front_length), true, retail);
    let flip = random.next() < 0.5;
    let (order, ground_types) = if flip {
        (widths.iter().rev().copied().collect::<Vec<_>>(), types.iter().rev().copied().collect::<Vec<_>>())
    } else {
        (widths.clone(), types.clone())
    };
    let bays: Vec<J> = order.iter().enumerate().map(|(i, w)| json!({ "width": w, "storeys": { "0": ground_types[i] } })).collect();

    let mut by_edge = Map::new();
    let mut promoted: Option<usize> = None;
    if has_clerestory {
        let returns: Vec<_> = frame.edges_for_role("front-secondary").into_iter().filter(|e| e.length >= MIN_PROMOTED_RETURN_M).collect();
        if let Some(first) = returns.first() {
            let best = returns.iter().fold(*first, |best, e| if e.length > best.length { e } else { best });
            promoted = Some(best.index);
            by_edge.insert(
                best.index.to_string(),
                json!({
                    "window": "blind",
                    "ground": "blind",
                    "bays": { "count": 1 },
                    "endPier": end_pier_for(best.length),
                    "storeys": { "1": "ribbon" },
                    "ribbonSpec": clerestory(),
                }),
            );
        }
    }

    let coping = if cladding.2 { "painted-steel-mid" } else { "cast-stone" };
    let signage = BAND_PAINTS[(random.next() * BAND_PAINTS.len() as f64).floor() as usize];

    let mut front = json!({
        "window": "blind",
        "bays": bays,
        "endPier": end_pier_for(front_length),
        "ribbonSpec": clerestory(),
        "rollerSpec": { "width": 5.0, "height": 4.2, "inset": 0.1, "hood": 0.35, "slat": 1.0 },
        "entranceSpec": {
            "width": 1.6,
            "height": 2.3,
            "recess": WALL_THICKNESS_M - 0.02,
            "transom": 0,
            "leaf": if retail { "single-glass" } else { "single-solid" },
            "frameWidth": 0.07,
        },
        "storefrontSpec": { "headClear": 0.25, "pier": 0.6 },
    });
    if has_clerestory {
        set(&mut front, "storeys", json!({ "1": "ribbon" }));
    }
    let monitor = random.next() < 0.34;

    let storeys_j: Vec<J> = storeys.iter().map(|(h, u)| json!({ "height": h, "use": u })).collect();
    json!({
        "id": format!("sf-archetype-{}", c.seed),
        "seed": c.seed,
        "height": c.height,
        "storeys": storeys_j,
        "wallThickness": WALL_THICKNESS_M,
        "parapet": { "height": parapet, "thickness": 0.18 },
        "coping": { "height": COPING_M, "overhang": 0.04 },
        "materials": {
            "wall": cladding.0,
            "parapet": cladding.0,
            "spandrel": cladding.0,
            "coping": coping,
            "trim": "painted-steel-mid",
            "metal": "painted-steel-mid",
            "glass": "glass-clear",
            "frame": "painted-steel-light",
            "roller": "painted-steel-light",
            "door": "painted-steel",
            "doorGlass": "glass-door",
            "recess": "painted-steel-mid",
            "base": "concrete-precast",
            "roof": "asphalt-roof",
            "signage": signage,
        },
        "elevations": {
            "front": front,
            "front-secondary": "blind",
            "left": "blind",
            "right": "blind",
            "back": "blind",
            "default": "blind",
            "byEdge": by_edge,
        },
        "roof": { "slab": 0.25 },
        "archetype": {
            "kind": "industrial-shed",
            "version": INDUSTRIAL_SHED_VERSION,
            "wall": cladding.0,
            "metalClad": cladding.2,
            "retail": retail,
            "clerestory": has_clerestory,
            "groundTypes": ground_types,
            "promotedEdge": promoted,
            "monitor": monitor,
        },
    })
}

pub fn detail(plan: &mut Plan, frame: &Frame, spec: &J) {
    let Some(meta) = spec.get_nn("archetype") else { return };
    let kind = meta.s("kind").unwrap_or("").to_string();
    let base_y = if plan.base_y.is_nan() { 0.0 } else { plan.base_y };
    let roof_top = plan.envelope.roof_top + base_y;
    let wall_top = plan.envelope.wall_top + base_y;
    let front = frame.edge_for_role("front");
    let record_idx = front.and_then(|f| plan.edges.iter().position(|e| e.index == f.index));

    let mut doors: Vec<J> = vec![];
    if let (Some(front), Some(ri)) = (front, record_idx) {
        let allowance = 0.12f64.min(front.allowance);
        let rollers: Vec<(f64, f64, f64)> =
            plan.edges[ri].openings.iter().filter(|o| o.type_ == "roller").map(|o| (o.s0, o.s1, o.y1)).collect();
        for &(s0, s1, op_y1) in &rollers {
            let y0 = op_y1;
            let y1 = (y0 + 0.18).min(roof_top - 0.02);
            if y1 - y0 < 0.06 {
                continue;
            }
            plan.add_profile(
                &front.basis,
                PartArgs::profile("dock-lintel", "metal", s0 - 0.12, s1 + 0.12, trim_profile_d("flat", allowance, y1 - y0, y0))
                    .edge(front.index)
                    .meta(json!({ "archetype": kind, "part": "dock-lintel" })),
            );
        }

        let types: Vec<String> =
            meta.arr("groundTypes").map(|a| a.iter().map(|v| v.as_str().unwrap_or("").to_string()).collect()).unwrap_or_default();
        let mut band: Option<(f64, f64, f64)> = None;
        for bay in &plan.edges[ri].bays {
            if types.get(bay.index).map(|s| s.as_str()) != Some("blind") {
                continue;
            }
            if band.map(|b| bay.width > b.2).unwrap_or(true) {
                band = Some((bay.s0, bay.s1, bay.width));
            }
        }
        if let (Some((b_s0, b_s1, b_w)), Some(ground)) = (band, plan.storeys.first()) {
            if b_w >= 3.0 {
                let height = 0.9f64.min(ground.height * 0.22);
                let y1 = (ground.y1 + base_y - 0.25).min(wall_top - 0.2).min(roof_top - 0.25);
                let y0 = y1 - height;
                if y0 > (ground.y0 + base_y) + 0.6 {
                    plan.add_box(
                        &front.basis,
                        PartArgs::new("signband", "signage", b_s0 + 0.35, b_s1 - 0.35, y0, y1, -0.01, 0.04f64.min(front.allowance))
                            .edge(front.index)
                            .meta(json!({ "archetype": kind, "part": "name-band" })),
                    );
                }
            }
        }
    }

    if meta.truthy("monitor") {
        add_roof_monitor(plan, frame, &kind, wall_top, roof_top);
    }

    if let (Some(front), Some(ri)) = (front, record_idx) {
        let [ox, oz] = front.basis.origin;
        let [ux, uz] = front.basis.u;
        let [nx, nz] = front.basis.n;
        for op in &plan.edges[ri].openings {
            if op.type_ != "roller" {
                continue;
            }
            let centre = (op.s0 + op.s1) / 2.0;
            doors.push(json!({
                "edge": front.index,
                "width": op.s1 - op.s0,
                "world": { "x": ox + ux * centre, "z": oz + uz * centre },
                "outward": [nx, nz],
            }));
        }
    }
    plan.extra.insert(
        "archetype".into(),
        json!({
            "kind": kind,
            "version": meta.at("version"),
            "retail": meta.at("retail"),
            "clerestory": meta.at("clerestory"),
            "garage": doors.first().cloned().unwrap_or(J::Null),
            "loadingDoors": doors,
        }),
    );
}

fn add_roof_monitor(plan: &mut Plan, frame: &Frame, kind: &str, wall_top: f64, roof_top: f64) {
    let along_x = frame.width >= frame.depth;
    let long_span = if along_x { frame.width } else { frame.depth };
    if !(long_span >= 12.0) || !(frame.width.min(frame.depth) >= 8.0) {
        return;
    }
    let half_length = (long_span * 0.3).min(26.0);
    let half_width = 1.3;
    let s = if along_x { half_length } else { half_width };
    let w = if along_x { half_width } else { half_length };
    for ds in [-1.0, 1.0] {
        for dw in [-1.0, 1.0] {
            let p = frame.to_world(ds * s, dw * w);
            if !(frame.signed_distance(p.x, p.z) < -0.9) {
                return;
            }
        }
    }
    let meta = json!({ "archetype": kind, "part": "roof-monitor", "roof": true });
    let basis = frame.basis;
    let curb_top = wall_top + 0.22;
    let glass_top = (wall_top + 0.98).min(roof_top + 2.4);
    if !(glass_top > curb_top + 0.2) {
        return;
    }
    let mut boxed = |kind: &str, slot: &str, y0: f64, y1: f64, grow: f64, cast: Option<bool>| {
        let mut a = PartArgs::new(kind, slot, -s - grow, s + grow, y0, y1, -w - grow, w + grow).meta(meta.clone());
        a.cast_shadow = cast;
        plan.add_box(&basis, a);
    };
    boxed("roof-monitor", "wall", wall_top - 0.05, curb_top, 0.0, None);
    boxed("roof-monitor-glass", "glass", curb_top, glass_top, -0.05, Some(false));
    boxed("roof-monitor-cap", "roof", glass_top, glass_top + 0.16, 0.1, None);
}
