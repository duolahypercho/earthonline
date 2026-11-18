//! Port of src/world/buildings/archetypes/downtown-tower.js (the downtown frame
//! tower: lobby, optional podium band, one ribbon storey for the whole shaft,
//! crown screen on the parapet; the post-pass draws the floor bands, piers,
//! crown fins, podium cap, roof penthouse and the prewar arcade arches).
//!
//! `archInfill` is imported from stucco-row.js in the JS; stucco_row.rs is owned
//! by another port, so it is ported here as a PRIVATE copy (`arch_infill`).
//! Not ported: DOWNTOWN_TOWER_LOD_DROP_ORDER (documentation table, no runtime use).

use std::f64::consts::PI;

use crate::classify::Classified;
use crate::elevation::{arc_points, trim_profile_d, PartArgs, Plan};
use crate::frame::{Edge, Frame, P2};
use crate::js::*;
use crate::rng::Rng;

pub const DOWNTOWN_TOWER_VERSION: &str = "sf-archetype-downtown-tower-v3";

const TYPICAL_FLOOR_MIN: f64 = 3.45;
const TYPICAL_FLOOR_MAX: f64 = 4.05;
const LOBBY_MIN: f64 = 4.6;
const LOBBY_MAX: f64 = 7.2;
const CROWN_MIN: f64 = 2.6;
const CROWN_MAX: f64 = 5.4;
const COPING_M: f64 = 0.22;
const MIN_STOREY_M: f64 = 3.2;
const PODIUM_MIN_SHAFT_M: f64 = 45.0;
const MIN_GLAZED_EDGE_M: f64 = 6.0;
const MIN_FALLBACK_EDGE_M: f64 = 3.5;
const MIN_CAP_EDGE_M: f64 = 2.5;
const MAX_GLAZED_EDGES: usize = 4;
const ENTRY_EDGE_MIN_M: f64 = 10.0;
const ENTRY_BAY_M: f64 = 5.4;
const GROUND_MODULE_M: f64 = 7.5;
const ARCADE_MODULE_M: f64 = 4.2;
const ARCADE_MAX_BAYS: f64 = 7.0;
const ARCADE_RISE_M: f64 = 0.95;
const SERVICE_BAY_M: f64 = 6.2;
const SERVICE_EDGE_MIN_M: f64 = 18.0;
const SERVICE_MIN_AREA_M2: f64 = 900.0;

struct Family {
    wall: &'static str,
    spandrel: &'static str,
    glass: &'static str,
    reveal: f64,
    spandrel_band: f64,
    pier_width: f64,
    pier_projection: f64,
    pier_spacing: f64,
    band_projection: f64,
}

fn family(key: &str) -> Family {
    match key {
        "curtain-wall" => Family {
            wall: "anodized-aluminium",
            spandrel: "anodized-aluminium",
            glass: "glass-film",
            reveal: 0.11,
            spandrel_band: 1.05,
            pier_width: 0.34,
            pier_projection: 0.13,
            pier_spacing: 4.4,
            band_projection: 0.07,
        },
        "prewar-masonry" => Family {
            wall: "limestone-ashlar",
            spandrel: "cast-stone",
            glass: "glass-clear",
            reveal: 0.3,
            spandrel_band: 1.35,
            pier_width: 1.05,
            pier_projection: 0.1,
            pier_spacing: 3.1,
            band_projection: 0.06,
        },
        _ => Family {
            wall: "concrete-precast",
            spandrel: "concrete-precast",
            glass: "glass-clear",
            reveal: 0.26,
            spandrel_band: 1.55,
            pier_width: 0.92,
            pier_projection: 0.2,
            pier_spacing: 5.4,
            band_projection: 0.12,
        },
    }
}

const CROWN_FINISHES: [&str; 4] = ["perforated-metal", "concrete-precast", "painted-steel-mid", "perforated-metal"];
const BASE_FINISHES: [&str; 4] = ["limestone-ashlar", "cast-stone", "concrete-precast", "limestone-ashlar"];

struct Stack {
    storeys: Vec<f64>,
    crown: f64,
    shaft_index: i64,
    podium_index: i64,
    floors: f64,
    floor_height: f64,
}

fn tower_stack(height: f64, random: &mut Rng) -> Stack {
    let crown = CROWN_MIN + random.next() * (CROWN_MAX - CROWN_MIN);
    let wall = MIN_STOREY_M.max(height - crown - COPING_M);
    let typical = TYPICAL_FLOOR_MIN + random.next() * (TYPICAL_FLOOR_MAX - TYPICAL_FLOOR_MIN);
    if wall < MIN_STOREY_M * 2.0 {
        return Stack { storeys: vec![wall], crown, shaft_index: 0, podium_index: -1, floors: 1.0, floor_height: wall };
    }
    let mut lobby = LOBBY_MAX.min(LOBBY_MIN.max(wall * 0.09));
    let podium_floors = if wall - lobby > PODIUM_MIN_SHAFT_M + 4.0 * typical && random.next() < 0.42 {
        2.0 + (random.next() * 3.0).floor()
    } else {
        0.0
    };
    let mut podium = podium_floors * typical;
    let mut shaft = wall - lobby - podium;
    if shaft < PODIUM_MIN_SHAFT_M {
        podium = 0.0;
        shaft = wall - lobby;
    }
    if shaft < MIN_STOREY_M {
        lobby = MIN_STOREY_M.max(wall - MIN_STOREY_M);
        shaft = wall - lobby;
    }
    let mut storeys = vec![lobby];
    let mut podium_index = -1;
    if podium > 0.0 {
        podium_index = storeys.len() as i64;
        storeys.push(podium);
    }
    let shaft_index = storeys.len() as i64;
    storeys.push(shaft);
    let floors = 1f64.max(js_round(shaft / typical));
    Stack { storeys, crown, shaft_index, podium_index, floors, floor_height: shaft / floors }
}

/// The glazed elevations (edge indices), longest first, capped at four.
fn glazed_edges(frame: &Frame) -> Vec<usize> {
    let mut ordered: Vec<&Edge> = frame.edges.iter().filter(|e| !e.party && e.length >= MIN_FALLBACK_EDGE_M).collect();
    ordered.sort_by(|a, b| {
        let d = b.length - a.length;
        if d != 0.0 && !d.is_nan() {
            d.partial_cmp(&0.0).unwrap()
        } else {
            a.index.cmp(&b.index)
        }
    });
    let real: Vec<&Edge> = ordered.iter().copied().filter(|e| e.length >= MIN_GLAZED_EDGE_M).collect();
    let pick: Vec<&Edge> = if !real.is_empty() { real } else { ordered.into_iter().take(1).collect() };
    pick.into_iter().take(MAX_GLAZED_EDGES).map(|e| e.index).collect()
}

fn ground_bays(available: f64, kind: &str, ground_type: &str, arcade: bool) -> (Vec<f64>, Vec<String>, i64) {
    let fixed = if kind == "entry" {
        ENTRY_BAY_M
    } else if kind == "service" {
        SERVICE_BAY_M
    } else {
        0.0
    };
    let mut widths: Vec<f64> = vec![];
    let mut kinds: Vec<String> = vec![];
    let mut special_bay: i64 = -1;
    if fixed <= 0.0 || available < fixed + 1.2 {
        widths.push(available);
        kinds.push(ground_type.into());
    } else {
        let rest = available - fixed;
        let two_sided = rest >= GROUND_MODULE_M;
        if two_sided {
            widths.push(rest * 0.5);
            kinds.push(ground_type.into());
        }
        special_bay = widths.len() as i64;
        widths.push(fixed);
        kinds.push(if kind == "entry" { "entrance" } else { "roller" }.into());
        widths.push(if two_sided { rest * 0.5 } else { rest });
        kinds.push(ground_type.into());
    }
    let sum: f64 = widths.iter().fold(0.0, |a, b| a + b);
    let total = if sum == 0.0 || sum.is_nan() { 1.0 } else { sum };
    let scaled: Vec<f64> = widths.iter().map(|w| (available * w) / total).collect();
    if !arcade {
        return (scaled, kinds, special_bay);
    }
    let mut out_w = vec![];
    let mut out_k = vec![];
    let mut out_special = -1;
    for (i, w) in scaled.iter().enumerate() {
        if i as i64 == special_bay {
            out_special = out_w.len() as i64;
        }
        let n = if kinds[i] == ground_type { 1f64.max(ARCADE_MAX_BAYS.min(js_round(w / ARCADE_MODULE_M))) } else { 1.0 };
        let n = n as usize;
        for _ in 0..n {
            out_w.push(w / n as f64);
            out_k.push(kinds[i].clone());
        }
    }
    (out_w, out_k, out_special)
}

pub fn spec(c: &Classified, frame: &Frame) -> J {
    let mut random = Rng::from_num(c.seed as f64);
    let stack = tower_stack(c.height, &mut random);
    let drawn_family = if random.next() < 0.62 { "curtain-wall" } else { "punched-grid" };
    let drawn_crown = CROWN_FINISHES[(random.next() * CROWN_FINISHES.len() as f64).floor() as usize];
    let drawn_base = BASE_FINISHES[(random.next() * BASE_FINISHES.len() as f64).floor() as usize];
    let prewar = (c.template == "downtown-tower" && c.variant == "prewar") || c.template == "art-deco-midrise";
    let family_key = if prewar { "prewar-masonry" } else { drawn_family };
    let fam = family(family_key);
    let crown_finish = if prewar { "cast-stone" } else { drawn_crown };
    let base_finish = if prewar { "limestone-ashlar" } else { drawn_base };
    let retail = c.has_retail_ground;

    let facades = glazed_edges(frame);
    let front_edge = frame.edge_for_role("front");
    let entry_edge: Option<usize> = match front_edge {
        Some(f) if facades.contains(&f.index) && f.length >= ENTRY_EDGE_MIN_M => Some(f.index),
        _ => facades.iter().copied().find(|&i| frame.edges[i].length >= ENTRY_EDGE_MIN_M).or(facades.first().copied()),
    };
    let service_edge: Option<usize> = if c.area >= SERVICE_MIN_AREA_M2 {
        facades.iter().copied().find(|&i| Some(i) != entry_edge && frame.edges[i].length >= SERVICE_EDGE_MIN_M)
    } else {
        None
    };

    let ground_type = "storefront";
    let storey_types: Vec<J> = (0..stack.storeys.len()).map(|i| json!(if i == 0 { ground_type } else { "ribbon" })).collect();
    let mech_band = 4.4f64.min(1.6f64.max(stack.floor_height * 1.05));
    let ribbon_spec = json!({
        "spandrelHeight": 2.2f64.min(fam.spandrel_band + 0.35),
        "headClear": mech_band,
        "mullionSpacing": 1e4,
        "frameWidth": if family_key == "curtain-wall" { 0.12 } else { 0.1 },
        "frameDepth": 0.12,
    });

    let mut by_edge = Map::new();
    let mut ground_plan = Map::new();
    for edge in &frame.edges {
        if !facades.contains(&edge.index) {
            by_edge.insert(edge.index.to_string(), json!("blind"));
            continue;
        }
        let end_pier = 1.1f64.min(0.1f64.max(edge.length * 0.05));
        let available = 0.6f64.max(edge.length - 2.0 * end_pier);
        let kind = if Some(edge.index) == entry_edge {
            "entry"
        } else if Some(edge.index) == service_edge {
            "service"
        } else {
            "plain"
        };
        let (widths, kinds, special_bay) = ground_bays(available, kind, ground_type, prewar);
        ground_plan.insert(edge.index.to_string(), json!({ "specialBay": special_bay, "kind": kind, "endPier": end_pier }));
        let storefront_spec = if prewar {
            json!({ "stallRiser": 0.7, "mullionSpacing": 2.0, "transom": 1.0, "pier": 0.5, "headClear": 0.9, "frameWidth": 0.1, "frameDepth": 0.14, "setback": 0.2 })
        } else if retail {
            json!({ "stallRiser": 0.52, "mullionSpacing": 3.0, "transom": 0.7, "pier": 0.22, "headClear": 1.0, "frameWidth": 0.1, "frameDepth": 0.14, "setback": 0.16 })
        } else {
            json!({ "stallRiser": 0.62, "mullionSpacing": 4.2, "transom": 0, "pier": 0.3, "headClear": 1.1, "frameWidth": 0.11, "frameDepth": 0.16, "setback": 0.2 })
        };
        let mut entrance_spec = json!({
            "width": 3.6, "height": 3.0, "recess": 1.0, "transom": 1.2, "leaf": "double-glass", "frameWidth": 0.09,
        });
        if !prewar {
            set(&mut entrance_spec, "canopy", json!({ "projection": 1.6, "thickness": 0.22, "overhang": 0.7, "rods": true, "slot": "canopy" }));
        }
        let bays: Vec<J> = widths.iter().zip(kinds.iter()).map(|(w, k)| json!({ "width": w, "storeys": [k] })).collect();
        by_edge.insert(
            edge.index.to_string(),
            json!({
                "window": "ribbon",
                "ground": ground_type,
                "endPier": end_pier,
                "reveal": fam.reveal,
                "sill": false,
                "bays": bays,
                "storeys": storey_types,
                "ribbonSpec": ribbon_spec,
                "storefrontSpec": storefront_spec,
                "entranceSpec": entrance_spec,
                "rollerSpec": { "width": 5.4, "height": 4.0, "inset": 0.14, "hood": 0.4, "slat": 0.6 },
            }),
        );
    }

    let storeys: Vec<J> = stack.storeys.iter().map(|h| json!({ "height": h })).collect();
    json!({
        "id": format!("sf-archetype-{}", c.seed),
        "seed": c.seed,
        "height": c.height,
        "storeys": storeys,
        "parapet": { "height": stack.crown, "thickness": 0.4 },
        "coping": { "height": COPING_M, "overhang": 0.09 },
        "wallThickness": 0.36,
        "materials": {
            "wall": fam.wall,
            "spandrel": fam.spandrel,
            "parapet": crown_finish,
            "coping": "cast-stone",
            "base": base_finish,
            "bulkhead": base_finish,
            "trim": "cast-stone-trim",
            "glass": fam.glass,
            "frame": "anodized-aluminium",
            "storefront": "anodized-aluminium",
            "door": "anodized-aluminium",
            "doorGlass": "glass-door",
            "recess": "limestone-ashlar",
            "canopy": "anodized-aluminium",
            "metal": "painted-steel-mid",
            "roller": "painted-steel-light",
            "plant": "painted-steel-mid",
            "roof": "asphalt-roof",
        },
        "elevations": { "default": "blind", "byEdge": by_edge },
        "roof": { "slab": 0.35, "deck": { "units": 2, "ducts": 1, "antennas": 1, "inset": 1.2 } },
        "archetype": {
            "kind": "downtown-tower",
            "version": DOWNTOWN_TOWER_VERSION,
            "family": family_key,
            "retail": retail,
            "crown": stack.crown,
            "shaftIndex": stack.shaft_index,
            "podiumIndex": stack.podium_index,
            "floors": stack.floors,
            "floorHeight": jn(stack.floor_height),
            "mechBand": mech_band,
            "spandrelBand": fam.spandrel_band,
            "bandProjection": fam.band_projection,
            "pierWidth": fam.pier_width,
            "pierProjection": fam.pier_projection,
            "pierSpacing": fam.pier_spacing,
            "facadeEdges": facades,
            "entryEdge": entry_edge.map(|i| i as i64).unwrap_or(-1),
            "serviceEdge": service_edge.map(|i| i as i64).unwrap_or(-1),
            "groundPlan": ground_plan,
            "arcade": prewar,
            "penthouse": frame.polygon.len() <= 12 && frame.area >= 220.0,
        },
    })
}

/// `archInfill` from stucco-row.js (private copy, see the file comment).
#[allow(clippy::too_many_arguments)]
fn arch_infill(plan: &mut Plan, edge: &Edge, s0: f64, s1: f64, y_top: f64, rise: f64, w0: f64, w1: f64, meta: J) -> Option<usize> {
    let segments = 8;
    let half = (s1 - s0) / 2.0;
    if !(half > 0.2) {
        return None;
    }
    let h = 0.12f64.max(rise.min(half));
    let apex = y_top - 0.03;
    let r = (half * half + h * h) / (2.0 * h);
    let cy = apex - r;
    let spring = apex - h;
    let theta0 = (half / r).min(1.0).acos();
    let mut points = vec![[s0, spring], [s0, y_top], [s1, y_top], [s1, spring]];
    let arc = arc_points((s0 + s1) / 2.0, cy, r, theta0, PI - theta0, segments);
    for p in arc.iter().take(arc.len() - 1).skip(1) {
        points.push(*p);
    }
    plan.add_plate(
        &edge.basis,
        PartArgs { kind: "arch-infill".into(), slot: "wall".into(), edge: Some(edge.index), profile: points, w0, w1, meta: Some(meta), ..Default::default() },
    )
}

pub fn detail(plan: &mut Plan, frame: &Frame, spec: &J) {
    let Some(meta) = spec.get_nn("archetype") else { return };
    let base_y = if plan.base_y.is_nan() { 0.0 } else { plan.base_y };
    let shaft_index = meta.num("shaftIndex");
    let Some(shaft) = (shaft_index >= 0.0).then(|| plan.storeys.get(shaft_index as usize)).flatten() else { return };
    let kind = meta.s("kind").unwrap_or("").to_string();
    let shaft_y0 = shaft.y0 + base_y;
    let shaft_y1 = shaft.y1 + base_y;
    let wall_top = plan.envelope.wall_top + base_y;
    let roof_top = plan.envelope.roof_top + base_y;
    let mech_band = meta.num("mechBand");
    let spandrel_band = meta.num("spandrelBand");
    let ribbon_top = shaft_y1 - mech_band;
    let ribbon_bottom = shaft_y0 + 2.2f64.min(spandrel_band + 0.35);
    let floors = meta.num("floors");
    let floor_height = meta.num("floorHeight");
    let crown = meta.num("crown");
    let facade_edges: Vec<usize> = meta.arr("facadeEdges").map(|a| a.iter().filter_map(|v| v.as_u64()).map(|v| v as usize).collect()).unwrap_or_default();

    for &index in &facade_edges {
        let Some(edge) = frame.edges.get(index) else { continue };
        let basis = edge.basis;
        let allow = 0.08f64.max(edge.allowance - 0.05);
        let band_w = meta.num("bandProjection").min(allow);
        let pier_w = meta.num("pierProjection").min(allow);

        let band = spandrel_band;
        let mut i = 1.0;
        while i < floors {
            let line = shaft_y0 + floor_height * i;
            let y0 = line - band * 0.58;
            let y1 = line + band * 0.42;
            if !(y0 < ribbon_bottom - 0.05 || y1 > (ribbon_top + 0.05).min(roof_top)) {
                plan.add_box(
                    &basis,
                    PartArgs::new("floor-band", "spandrel", 0.0, edge.length, y0, y1, -0.04, band_w)
                        .edge(index)
                        .shadow(true)
                        .receive(true)
                        .meta(json!({ "archetype": kind, "part": "floor-band", "floor": i })),
                );
            }
            i += 1.0;
        }

        let width = meta.num("pierWidth");
        let pier_top = shaft_y1.min(roof_top);
        if pier_top - shaft_y0 > 1.0 {
            let pier_spacing = meta.num("pierSpacing");
            let span = 0f64.max(edge.length - width);
            let count = if span > pier_spacing { (span / pier_spacing).floor() } else { 0.0 };
            let mut positions = vec![0.0];
            let mut k = 1.0;
            while k <= count {
                positions.push((span * k) / (count + 1.0));
                k += 1.0;
            }
            positions.push(span);
            for s in positions {
                plan.add_box(
                    &basis,
                    PartArgs::new("pier", "trim", s, s + width, shaft_y0, pier_top, -0.03, pier_w)
                        .edge(index)
                        .shadow(true)
                        .receive(true)
                        .meta(json!({ "archetype": kind, "part": "pier" })),
                );
            }
        }

        let crown_top = roof_top - 0.24;
        for t in [0.34, 0.68] {
            let y0 = wall_top + crown * t;
            let y1 = y0 + 0.2;
            if y1 > crown_top {
                continue;
            }
            plan.add_box(
                &basis,
                PartArgs::new("crown-fin", "metal", 0.0, edge.length, y0, y1, -0.02, 0.14f64.min(allow))
                    .edge(index)
                    .shadow(true)
                    .receive(true)
                    .meta(json!({ "archetype": kind, "part": "crown-fin" })),
            );
        }
    }

    // The podium cap.
    let podium_index = meta.num("podiumIndex");
    let cap_index = if podium_index >= 0.0 { podium_index as usize } else { 0 };
    if let Some(cap_storey) = plan.storeys.get(cap_index) {
        if plan.storeys.len() > 1 {
            let cap_y = cap_storey.y1 + base_y;
            let height = 0.55;
            if cap_y + height * 0.5 < roof_top {
                for edge in &frame.edges {
                    let facade = facade_edges.contains(&edge.index);
                    if edge.length < MIN_CAP_EDGE_M && !facade {
                        continue;
                    }
                    let p = 0.34f64.min(0.06f64.max(edge.allowance - 0.06));
                    let section = if facade { "stepped" } else { "flat" };
                    plan.add_profile(
                        &edge.basis,
                        PartArgs::profile("podium-cap", "trim", 0.0, edge.length, trim_profile_d(section, p, height, cap_y - height * 0.5))
                            .edge(edge.index)
                            .meta(json!({ "archetype": kind, "part": "podium-cap" })),
                    );
                }
            }
        }
    }

    // The roof penthouse.
    if meta.truthy("penthouse") {
        let rise = 4.2f64.min(2.2f64.max(crown * 0.9));
        let (cx, cz) = (frame.centroid.x, frame.centroid.z);
        let points: Vec<P2> = frame.polygon.iter().map(|p| P2::new(cx + (p.x - cx) * 0.52, cz + (p.z - cz) * 0.52)).collect();
        plan.add_prism(PartArgs {
            kind: "roof-penthouse".into(),
            slot: "plant".into(),
            points,
            y0: wall_top,
            y1: (wall_top + crown + rise).min(roof_top + 2.6),
            cast_shadow: Some(true),
            receive_shadow: Some(true),
            meta: Some(json!({ "archetype": kind, "part": "penthouse", "roof": true })),
            ..Default::default()
        });
    }

    // Cross-system hand-off: the garage apron.
    let mut garage = J::Null;
    let service_edge = meta.num("serviceEdge");
    if service_edge >= 0.0 {
        let se = service_edge as usize;
        let edge = frame.edges.get(se);
        let special = meta.get_nn("groundPlan").get_nn(&se.to_string()).get_nn("specialBay").and_then(|v| v.as_f64()).unwrap_or(-1.0);
        let bay = plan
            .edges
            .iter()
            .find(|e| e.index == se)
            .and_then(|r| if special >= 0.0 { r.bays.get(special as usize) } else { None });
        if let (Some(edge), Some(bay)) = (edge, bay) {
            let [ox, oz] = edge.basis.origin;
            let [ux, uz] = edge.basis.u;
            let [nx, nz] = edge.basis.n;
            let centre = (bay.s0 + bay.s1) / 2.0;
            garage = json!({
                "edge": se,
                "width": 5.4f64.min(bay.width - 0.4),
                "world": { "x": ox + ux * centre, "z": oz + uz * centre },
                "outward": [nx, nz],
            });
        }
    }

    // The prewar arcade.
    let mut arches = 0;
    if meta.truthy("arcade") {
        let mut jobs: Vec<(usize, f64, f64, f64, bool)> = vec![];
        for record in &plan.edges {
            if frame.edges.get(record.index).is_none() || !facade_edges.contains(&record.index) {
                continue;
            }
            for op in &record.openings {
                if op.storey != 0 || (op.type_ != "storefront" && op.type_ != "entrance") {
                    continue;
                }
                if op.type_ == "storefront" && op.s1 - op.s0 < 1.6 {
                    continue;
                }
                jobs.push((record.index, op.s0, op.s1, op.y1, op.type_ == "entrance"));
            }
        }
        for (index, s0, s1, y_top, entrance) in jobs {
            let edge = &frame.edges[index];
            let part = arch_infill(
                plan,
                edge,
                s0,
                s1,
                y_top,
                ARCADE_RISE_M,
                -0.15,
                0.005,
                json!({ "archetype": kind, "part": if entrance { "portal-arch" } else { "arcade-arch" } }),
            );
            if part.is_some() {
                arches += 1;
            }
        }
    }
    plan.extra.insert(
        "archetype".into(),
        json!({
            "kind": kind,
            "version": meta.at("version"),
            "family": meta.at("family"),
            "arches": arches,
            "retail": meta.at("retail"),
            "floors": meta.at("floors"),
            "floorHeight": meta.at("floorHeight"),
            "facadeEdges": meta.at("facadeEdges"),
            "entryEdge": meta.at("entryEdge"),
            "garage": garage,
        }),
    );
}
