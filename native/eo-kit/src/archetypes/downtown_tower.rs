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
///
/// With the facade typology on (typology.rs), every face that is not on a lot
/// line is glazed (`typology::glaze_every_face`): a tower is windowed on all
/// its exposed sides; only a party wall stands blind. The port's cap of four
/// left a many-sided tower (a chamfered or stepped plan, 12-48 edges) with
/// twenty-storey blank stone faces between its four longest sides.
fn glazed_edges(frame: &Frame) -> Vec<usize> {
    if crate::typology::glaze_every_face() {
        let mut ordered: Vec<&Edge> = frame.edges.iter().filter(|e| !e.party && e.length >= MIN_FALLBACK_EDGE_M).collect();
        ordered.sort_by(|a, b| b.length.partial_cmp(&a.length).unwrap_or(std::cmp::Ordering::Equal).then(a.index.cmp(&b.index)));
        if !ordered.is_empty() {
            return ordered.into_iter().map(|e| e.index).collect();
        }
    }
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
    let port_family = if prewar { "prewar-masonry" } else { drawn_family };
    // The facade system per tower (typology.rs; off: the family's own module).
    let system = crate::typology::tower(c, port_family);
    let family_key = system.as_ref().map(|t| t.family).unwrap_or(port_family);
    let mut fam = family(family_key);
    if let Some(t) = &system {
        let detail = crate::typology::detail_enabled();
        let extra_reveal = if !detail {
            0.0
        } else if t.family == "curtain-wall" {
            0.06
        } else {
            0.09
        };
        fam = Family {
            wall: t.wall,
            spandrel: t.spandrel,
            glass: t.glass,
            reveal: (t.reveal + extra_reveal).min(0.42),
            spandrel_band: t.spandrel_band,
            pier_width: t.pier_width,
            pier_projection: t.pier_projection,
            pier_spacing: t.pier_spacing,
            band_projection: t.band_projection,
        };
    }
    // A granite-clad tower (typology.rs `granite-bay`) carries its stone to the
    // top: the crown band and the coping in the tower's own granite, not a
    // light cast stone (555 California's parapet and roof edge are the same
    // dark carnelian granite as its bays).
    let granite = system.as_ref().is_some_and(|t| t.system == "granite-bay");
    let crown_finish = if granite { fam.wall } else if prewar { "cast-stone" } else { drawn_crown };
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
            // A blind side on a lot line (typology.rs on) is a party wall in its own finish: exposed brick on a prewar tower, cast concrete on a later one.
            by_edge.insert(edge.index.to_string(), if system.is_some() && edge.party { json!({ "window": "blind", "wallSlot": "party" }) } else { json!("blind") });
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
    let mut spec = json!({
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
            "coping": if granite { fam.wall } else { "cast-stone" },
            "base": base_finish,
            "bulkhead": base_finish,
            "trim": system.as_ref().map(|t| t.trim).unwrap_or("cast-stone-trim"),
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
        "roof": if crate::typology::detail_enabled() {
            json!({ "slab": 0.35, "plantAllowance": 4.2, "deck": { "units": 2, "ducts": 1, "antennas": 1, "inset": 1.2 } })
        } else {
            json!({ "slab": 0.35, "deck": { "units": 2, "ducts": 1, "antennas": 1, "inset": 1.2 } })
        },
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
            "system": system.as_ref().map(|t| json!(t.system)).unwrap_or(J::Null),
            "stoneTints": system.as_ref().and_then(|t| t.stone_tint.map(|k| {
                let mut m = Map::new();
                for name in [t.wall, t.spandrel, t.trim] {
                    if !name.starts_with("glass") && !matches!(name, "anodized-aluminium" | "painted-steel-mid" | "perforated-metal" | "painted-steel-light") {
                        m.insert(name.to_string(), json!(k));
                    }
                }
                J::Object(m)
            })).unwrap_or(J::Null),
        },
    });
    // the party walls' finish (typology.rs on): exposed brick on a prewar tower, cast concrete on a later one.
    if system.is_some() {
        let party = if family_key == "prewar-masonry" { "brick-common-bond" } else { "concrete-precast" };
        set(obj_mut(&mut spec, "materials"), "party", json!(party));
    }
    spec
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
    let detail_on = crate::typology::detail_enabled();
    let system_name = meta.s("system").unwrap_or("");
    let family_name = meta.s("family").unwrap_or("");
    let is_curtain = family_name == "curtain-wall" || matches!(system_name, "curtain-flush" | "curtain-mullion");
    let is_stone_fin = matches!(system_name, "precast-fin" | "precast-punched" | "masonry-pier" | "granite-bay") || family_name == "prewar-masonry";
    let wants_sunshades = detail_on && matches!(system_name, "curtain-mullion" | "ribbon-band" | "precast-fin" | "precast-punched");
    let mech_floor = if detail_on && floors >= 12.0 { Some((floors * 0.56).round()) } else { None };

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
                // Expressed horizontal sunshade / brise-soleil eyebrow above vision glass
                if wants_sunshades && y0 > ribbon_bottom + 0.4 {
                    let shade_w = (band_w + if is_curtain { 0.24 } else { 0.16 }).min(allow);
                    if shade_w > band_w + 0.04 {
                        plan.add_box(
                            &basis,
                            PartArgs::new("sunshade", "frame", 0.08, (edge.length - 0.08).max(0.1), y0 - 0.06, y0 + 0.02, band_w - 0.02, shade_w)
                                .edge(index)
                                .shadow(true)
                                .receive(true)
                                .meta(json!({ "archetype": kind, "part": "sunshade", "floor": i })),
                        );
                    }
                }
            }
            // Recessed mid-shaft louvered mechanical / refuge floor breaking uniform window grids
            if mech_floor == Some(i) {
                let my0 = line + band * 0.42;
                let my1 = (line + floor_height - band * 0.58).min(ribbon_top);
                if my1 - my0 > 1.2 {
                    let blades = 4;
                    let step = (my1 - my0) / blades as f64;
                    let lw = (band_w * 0.72).max(0.05).min(allow);
                    for b in 0..blades {
                        let by0 = my0 + step * (b as f64 + 0.18);
                        let by1 = by0 + step * 0.52;
                        plan.add_box(
                            &basis,
                            PartArgs::new("mech-louver", "metal", 0.04, (edge.length - 0.04).max(0.1), by0, by1, -0.08, lw)
                                .edge(index)
                                .shadow(true)
                                .receive(true)
                                .meta(json!({ "archetype": kind, "part": "mech-louver", "floor": i })),
                        );
                    }
                }
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
            let fin_proj = if detail_on && is_curtain {
                (pier_w + if system_name == "curtain-mullion" { 0.18 } else { 0.10 }).min(allow)
            } else {
                pier_w
            };
            for (pi, &s) in positions.iter().enumerate() {
                let is_corner = pi == 0 || pi + 1 == positions.len();
                let pw_use = if detail_on && is_corner { (pier_w + 0.08).min(allow) } else { pier_w };
                plan.add_box(
                    &basis,
                    PartArgs::new("pier", "trim", s, s + width, shaft_y0, pier_top, -0.03, pw_use)
                        .edge(index)
                        .shadow(true)
                        .receive(true)
                        .meta(json!({ "archetype": kind, "part": "pier" })),
                );
                if detail_on && is_stone_fin && width >= 0.28 && pw_use >= 0.09 {
                    // Stepped chamfered pier returns catching crisp raking shadows
                    let bw = (pw_use * 0.48).max(0.045);
                    let shoulder = (width * 0.22).clamp(0.05, 0.12);
                    let bs0 = (s - shoulder).max(0.0);
                    let bs1 = (s + width + shoulder).min(edge.length);
                    plan.add_box(
                        &basis,
                        PartArgs::new("pier-bevel", "trim", bs0, bs1, shaft_y0, pier_top, -0.03, bw)
                            .edge(index)
                            .shadow(false)
                            .receive(true)
                            .meta(json!({ "archetype": kind, "part": "pier-bevel" })),
                    );
                } else if detail_on && is_curtain && fin_proj > pier_w + 0.03 && (pi % 2 == 0 || is_corner) {
                    // Expressed anodized aluminum vertical solar fin blade on curtain-wall towers
                    let mid = s + width * 0.5;
                    plan.add_box(
                        &basis,
                        PartArgs::new("fin", "frame", mid - 0.03, mid + 0.03, shaft_y0, pier_top, pier_w - 0.01, fin_proj)
                            .edge(index)
                            .shadow(true)
                            .receive(true)
                            .meta(json!({ "archetype": kind, "part": "fin" })),
                    );
                }
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
        // Sculpted overhanging crown eyebrow cornice along the top of tower shafts (kills flat boxy rooflines)
        if detail_on && edge.length >= 4.0 && crown_top > wall_top + 0.8 {
            let cw = 0.44f64.min(allow);
            let ext0 = if edge.convex_start { cw } else { 0.0 };
            let ext1 = if edge.convex_end { cw } else { 0.0 };
            plan.add_profile(
                &basis,
                PartArgs::profile("cornice", "trim", -ext0, edge.length + ext1, trim_profile_d("stepped", cw, 0.38, crown_top - 0.34))
                    .edge(index)
                    .shadow(true)
                    .receive(true)
                    .meta(json!({ "archetype": kind, "part": "crown-eyebrow" })),
            );
        }
    }

    // Articulated double-height lobby entry portal framing (recessed columns + sculpted lintel + cantilevered glass-and-steel marquee)
    if detail_on {
        let entry_idx = meta.num("entryEdge");
        if entry_idx >= 0.0 {
            let ei = entry_idx as usize;
            if let (Some(edge), Some(lobby)) = (frame.edges.get(ei), plan.storeys.first()) {
                let special = meta.get_nn("groundPlan").get_nn(&ei.to_string()).get_nn("specialBay").and_then(|v| v.as_f64()).unwrap_or(-1.0);
                let bay = plan
                    .edges
                    .iter()
                    .find(|e| e.index == ei)
                    .and_then(|r| if special >= 0.0 { r.bays.get(special as usize) } else { None })
                    .cloned();
                if let Some(bay) = bay {
                    let allow = 0.08f64.max(edge.allowance - 0.05);
                    let ly0 = lobby.y0 + base_y;
                    let ly1 = (lobby.y1 + base_y).min(roof_top - 0.4);
                    let col_w = 0.48f64.min(bay.width * 0.12);
                    let col_p = 0.24f64.min(allow);
                    let lintel_p = 0.34f64.min(allow);
                    let ps0 = (bay.s0 - 0.06).max(0.0);
                    let ps1 = (bay.s1 + 0.06).min(edge.length);
                    if ly1 - ly0 > 3.2 && ps1 - ps0 > 2.4 {
                        for (c0, c1) in [(ps0, ps0 + col_w), (ps1 - col_w, ps1)] {
                            plan.add_box(
                                &edge.basis,
                                PartArgs::new("column", "base", c0, c1, ly0, ly1 - 0.28, -0.03, col_p)
                                    .edge(ei)
                                    .shadow(true)
                                    .receive(true)
                                    .meta(json!({ "archetype": kind, "part": "lobby-portal-column" })),
                            );
                        }
                        plan.add_box(
                            &edge.basis,
                            PartArgs::new("entablature", "base", (ps0 - 0.12).max(0.0), (ps1 + 0.12).min(edge.length), ly1 - 0.36, ly1 + 0.06, -0.03, lintel_p)
                                .edge(ei)
                                .shadow(true)
                                .receive(true)
                                .meta(json!({ "archetype": kind, "part": "lobby-portal-lintel" })),
                        );
                        // Cantilevered steel-and-glass lobby marquee on prewar towers (modern towers already get entranceSpec.canopy)
                        if meta.truthy("arcade") {
                            let my = (ly0 + 3.45).min(ly1 - 0.55);
                            let mp = 1.45f64.min(crate::frame::FITTING_PROJECTION_ALLOWANCE);
                            if my > ly0 + 2.65 && mp >= 0.7 {
                                let ms0 = (ps0 + 0.08).max(0.1);
                                let ms1 = (ps1 - 0.08).min(edge.length - 0.1);
                                if ms1 - ms0 > 2.0 {
                                    // Perimeter bronze/steel fascia frame + tie-rods (avoids solid opaque slab)
                                    plan.add_box(&edge.basis, PartArgs::new("canopy", "canopy", ms0, ms1, my, my + 0.22, mp - 0.14, mp).edge(ei).shadow(true).meta(json!({ "archetype": kind, "part": "lobby-marquee" })));
                                    plan.add_box(&edge.basis, PartArgs::new("canopy", "canopy", ms0, ms0 + 0.14, my, my + 0.22, -0.02, mp).edge(ei).shadow(true).meta(json!({ "archetype": kind, "part": "lobby-marquee" })));
                                    plan.add_box(&edge.basis, PartArgs::new("canopy", "canopy", ms1 - 0.14, ms1, my, my + 0.22, -0.02, mp).edge(ei).shadow(true).meta(json!({ "archetype": kind, "part": "lobby-marquee" })));
                                    plan.add_box(&edge.basis, PartArgs::new("canopy", "canopy", ms0 + 0.12, ms1 - 0.12, my + 0.04, my + 0.14, -0.02, mp - 0.12).edge(ei).shadow(true).meta(json!({ "archetype": kind, "part": "lobby-marquee-deck" })));
                                    for rs in [ms0 + 0.22, ms1 - 0.22] {
                                        let rise = 0.85f64.min((ly1 - my - 0.35).max(0.35));
                                        plan.add_box(&edge.basis, PartArgs::new("canopy-rod", "metal", rs - 0.025, rs + 0.025, my + 0.20, my + 0.20 + rise, mp - 0.12, mp - 0.07).edge(ei).meta(json!({ "archetype": kind, "part": "lobby-marquee-rod" })));
                                        plan.add_box(&edge.basis, PartArgs::new("canopy-rod", "metal", rs - 0.025, rs + 0.025, my + 0.18 + rise, my + 0.23 + rise, -0.02, mp - 0.07).edge(ei).meta(json!({ "archetype": kind, "part": "lobby-marquee-rod" })));
                                    }
                                }
                            }
                        }
                    }
                }
            }
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

    // The roof penthouse & detailed rooftop mechanical yard (cooling towers, chillers,
    // ducts, elevator machine room cap/louvers, parapet railings, BMU track).
    let mut pent_rect: Option<[f64; 4]> = None;
    if meta.truthy("penthouse") {
        let rise = 4.2f64.min(2.2f64.max(crown * 0.9));
        let (cx, cz) = (frame.centroid.x, frame.centroid.z);
        let points: Vec<P2> = frame.polygon.iter().map(|p| P2::new(cx + (p.x - cx) * 0.52, cz + (p.z - cz) * 0.52)).collect();
        let py1 = (wall_top + crown + rise).min(roof_top + if detail_on { 3.4 } else { 2.6 });
        if detail_on {
            let (mut x0, mut x1, mut z0, mut z1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
            for p in &points {
                let q = frame.to_plan(p.x, p.z);
                x0 = x0.min(q.x);
                x1 = x1.max(q.x);
                z0 = z0.min(q.z);
                z1 = z1.max(q.z);
            }
            if x0.is_finite() {
                pent_rect = Some([x0, x1, z0, z1]);
            }
        }
        plan.add_prism(PartArgs {
            kind: "roof-penthouse".into(),
            slot: "plant".into(),
            points: points.clone(),
            y0: wall_top,
            y1: py1,
            cast_shadow: Some(true),
            receive_shadow: Some(true),
            meta: Some(json!({ "archetype": kind, "part": "penthouse", "roof": true })),
            ..Default::default()
        });
        if detail_on {
            // Penthouse coping cap + elevator machine room overrun step
            let cap_pts: Vec<P2> = frame.polygon.iter().map(|p| P2::new(cx + (p.x - cx) * 0.54, cz + (p.z - cz) * 0.54)).collect();
            plan.add_prism(PartArgs {
                kind: "roof-curb".into(),
                slot: "coping".into(),
                points: cap_pts,
                y0: py1 - 0.04,
                y1: (py1 + 0.16).min(roof_top + 3.8),
                cast_shadow: Some(true),
                receive_shadow: Some(true),
                meta: Some(json!({ "archetype": kind, "part": "penthouse-cap", "roof": true })),
                ..Default::default()
            });
            if frame.area >= 420.0 {
                let ov_pts: Vec<P2> = frame.polygon.iter().map(|p| P2::new(cx + (p.x - cx) * 0.24, cz + (p.z - cz) * 0.24)).collect();
                let oy1 = (py1 + 1.35).min(roof_top + 3.9);
                if oy1 > py1 + 0.4 {
                    plan.add_prism(PartArgs {
                        kind: "roof-penthouse".into(),
                        slot: "metal".into(),
                        points: ov_pts,
                        y0: py1 - 0.02,
                        y1: oy1,
                        cast_shadow: Some(true),
                        receive_shadow: Some(true),
                        meta: Some(json!({ "archetype": kind, "part": "elevator-overrun", "roof": true })),
                        ..Default::default()
                    });
                }
            }
        }
    }
    if detail_on && frame.area >= 180.0 {
        let deck_y = wall_top;
        let roof_ceil = roof_top + 3.8;
        let fb = frame.basis;
        let b = &frame.plan_bounds;
        let mut rrng = Rng::new(&format!("{}|tower-roof-yard", plan.seed));
        let mut placed: Vec<[f64; 4]> = pent_rect.into_iter().collect();
        let rect_ok = |x0: f64, x1: f64, z0: f64, z1: f64, placed: &[[f64; 4]]| -> bool {
            for (px, pz) in [(x0, z0), (x1, z0), (x1, z1), (x0, z1), ((x0 + x1) * 0.5, (z0 + z1) * 0.5)] {
                let w = frame.to_world(px, pz);
                if !(frame.signed_distance(w.x, w.z) < -1.35) {
                    return false;
                }
            }
            !placed.iter().any(|r| x0 < r[1] + 0.65 && x1 > r[0] - 0.65 && z0 < r[3] + 0.65 && z1 > r[2] - 0.65)
        };
        let find_slot = |rrng: &mut Rng, sx: f64, sz: f64, placed: &[[f64; 4]]| -> Option<[f64; 4]> {
            for _ in 0..20 {
                let (ax, az) = if rrng.next() < 0.5 { (sx, sz) } else { (sz, sx) };
                let span_x = ((b.max_x - b.min_x) - ax - 2.8).max(0.0);
                let span_z = ((b.max_z - b.min_z) - az - 2.8).max(0.0);
                let x0 = b.min_x + 1.4 + rrng.next() * span_x;
                let z0 = b.min_z + 1.4 + rrng.next() * span_z;
                let (x1, z1) = (x0 + ax, z0 + az);
                if rect_ok(x0, x1, z0, z1, placed) {
                    return Some([x0, x1, z0, z1]);
                }
            }
            None
        };
        // 1-3 louvered cooling towers with fan shroud collars
        let ct_count = if frame.area >= 900.0 { 3 } else if frame.area >= 420.0 { 2 } else { 1 };
        for _ in 0..ct_count {
            let sx = 2.6 + rrng.next() * 1.4;
            let sz = 1.9 + rrng.next() * 0.9;
            let h = (2.3 + rrng.next() * 0.9).min(roof_ceil - deck_y - 0.55);
            if h > 1.0 {
                if let Some(r) = find_slot(&mut rrng, sx, sz, &placed) {
                    let y1 = deck_y + h;
                    plan.add_box(
                        &fb,
                        PartArgs::new("cooling-tower", "plant", r[0], r[1], deck_y - 0.02, y1, r[2], r[3])
                            .shadow(true)
                            .receive(true)
                            .meta(json!({ "archetype": kind, "part": "cooling-tower", "roof": true })),
                    );
                    // Cooling tower intake louver collar + fan stack shroud
                    let cx = (r[0] + r[1]) * 0.5;
                    let cz = (r[2] + r[3]) * 0.5;
                    let fr = sx.min(sz) * 0.34;
                    plan.add_box(
                        &fb,
                        PartArgs::new("roof-curb", "metal", cx - fr, cx + fr, y1 - 0.02, (y1 + 0.48).min(roof_ceil), cz - fr, cz + fr)
                            .shadow(true)
                            .receive(true)
                            .meta(json!({ "archetype": kind, "part": "cooling-fan-stack", "roof": true })),
                    );
                    plan.add_box(
                        &fb,
                        PartArgs::new("mech-louver", "frame", r[0] - 0.05, r[1] + 0.05, deck_y + 0.25, deck_y + h * 0.55, r[2] - 0.05, r[3] + 0.05)
                            .shadow(false)
                            .receive(true)
                            .meta(json!({ "archetype": kind, "part": "cooling-louver", "roof": true })),
                    );
                    placed.push(r);
                }
            }
        }
        // Chiller skids & galvanized duct runs
        let ch_count = if frame.area >= 600.0 { 2 } else { 1 };
        for _ in 0..ch_count {
            let sx = 1.8 + rrng.next() * 1.4;
            let sz = 1.2 + rrng.next() * 0.7;
            let h = (1.1 + rrng.next() * 0.6).min(roof_ceil - deck_y - 0.2);
            if h > 0.5 {
                if let Some(r) = find_slot(&mut rrng, sx, sz, &placed) {
                    plan.add_box(
                        &fb,
                        PartArgs::new("roof-plant", "metal", r[0], r[1], deck_y - 0.02, deck_y + h, r[2], r[3])
                            .shadow(true)
                            .receive(true)
                            .meta(json!({ "archetype": kind, "part": "chiller-skid", "roof": true })),
                    );
                    placed.push(r);
                }
            }
        }
        if let Some(d) = find_slot(&mut rrng, 4.2, 0.72, &placed) {
            plan.add_box(
                &fb,
                PartArgs::new("roof-duct", "roller", d[0], d[1], deck_y + 0.15, (deck_y + 0.72).min(roof_ceil), d[2], d[3])
                    .shadow(true)
                    .receive(true)
                    .meta(json!({ "archetype": kind, "part": "duct-run", "roof": true })),
            );
            placed.push(d);
        }
        // Perimeter parapet safety railing & BMU maintenance track on tall towers
        let rail_y = (roof_top + 0.88).min(roof_ceil);
        for edge in &frame.edges {
            if edge.length < 5.5 {
                continue;
            }
            let mid_w = frame.to_world(
                frame.to_plan(edge.basis.origin[0] + edge.basis.u[0] * edge.length * 0.5 - edge.basis.n[0] * 0.35, edge.basis.origin[1] + edge.basis.u[1] * edge.length * 0.5 - edge.basis.n[1] * 0.35).x,
                frame.to_plan(edge.basis.origin[0] + edge.basis.u[0] * edge.length * 0.5 - edge.basis.n[0] * 0.35, edge.basis.origin[1] + edge.basis.u[1] * edge.length * 0.5 - edge.basis.n[1] * 0.35).z,
            );
            if frame.signed_distance(mid_w.x, mid_w.z) < -0.2 {
                plan.add_box(
                    &edge.basis,
                    PartArgs::new("parapet-rail", "metal", 0.5, edge.length - 0.5, rail_y - 0.05, rail_y, -0.34, -0.28)
                        .edge(edge.index)
                        .shadow(false)
                        .receive(false)
                        .meta(json!({ "archetype": kind, "part": "parapet-rail", "roof": true })),
                );
            }
        }
        if floors >= 10.0 {
            if let Some(long_e) = frame.edges.iter().filter(|e| e.length >= 10.0).max_by(|a, b| a.length.partial_cmp(&b.length).unwrap_or(std::cmp::Ordering::Equal)) {
                let s0 = 2.0;
                let s1 = long_e.length - 2.0;
                let mid = (s0 + s1) * 0.5;
                let check = [
                    long_e.basis.origin[0] + long_e.basis.u[0] * mid - long_e.basis.n[0] * 1.4,
                    long_e.basis.origin[1] + long_e.basis.u[1] * mid - long_e.basis.n[1] * 1.4,
                ];
                if frame.signed_distance(check[0], check[1]) < -0.9 {
                    plan.add_box(
                        &long_e.basis,
                        PartArgs::new("bmu-track", "metal", s0, s1, deck_y, deck_y + 0.14, -1.55, -1.45)
                            .edge(long_e.index)
                            .shadow(false)
                            .meta(json!({ "archetype": kind, "part": "bmu-rail", "roof": true })),
                    );
                    plan.add_box(
                        &long_e.basis,
                        PartArgs::new("roof-plant", "metal", mid - 0.75, mid + 0.75, deck_y + 0.12, (deck_y + 1.25).min(roof_ceil), -1.85, -1.05)
                            .edge(long_e.index)
                            .shadow(true)
                            .meta(json!({ "archetype": kind, "part": "bmu-cradle", "roof": true })),
                    );
                }
            }
        }
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
            "system": meta.at("system"),
        }),
    );
    // the stone's per-tower tone (typology.rs), drawn after the paint (pipeline.rs `drawn_materials`).
    if let Some(t) = meta.get_nn("stoneTints").filter(|v| v.is_object()) {
        plan.extra.insert("stoneTints".into(), t.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{footprint_frame, FrameOptions};

    #[test]
    fn a_many_sided_tower_is_glazed_on_every_face_off_a_lot_line() {
        if !crate::typology::glaze_every_face() {
            return;
        }
        // A chamfered 40 m square: four 30 m sides and four 7 m corners.
        let pts = [(-15.0, -20.0), (15.0, -20.0), (20.0, -15.0), (20.0, 15.0), (15.0, 20.0), (-15.0, 20.0), (-20.0, 15.0), (-20.0, -15.0)];
        let polygon: Vec<P2> = pts.iter().map(|&(x, z)| P2 { x, z }).collect();
        let mut frame = footprint_frame(&polygon, 0, &FrameOptions::default()).expect("frame");
        assert_eq!(frame.edges.len(), 8);
        assert_eq!(glazed_edges(&frame).len(), 8, "the port's cap of four left the corners blind");
        frame.edges[2].party = true;
        let g = glazed_edges(&frame);
        assert_eq!(g.len(), 7);
        assert!(!g.contains(&2), "a party wall stays blind");
    }
}
