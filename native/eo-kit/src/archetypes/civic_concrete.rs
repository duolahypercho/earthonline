//! Port of src/world/buildings/archetypes/civic-concrete.js (the institutional
//! building: classical or postwar family, an odd symmetric bay grid on the
//! street elevation with a widened centre entrance bay, one promoted return,
//! and the entrance order / canopy drawn in the post-pass).
//!
//! Not ported: CIVIC_CONCRETE_LOD_DROP_ORDER (documentation table). FEATURES is
//! all-true at LOD0 and is folded in as constants.
//! The JS reads `aperture.y0 + aperture.height` (the door head); the kit's Rust
//! aperture record carries only id/edge/storey/bay/s0/s1, so the door head is
//! recovered from the opening record (y0) and the entrance's first door leaf
//! (y1 = doorTop - 0.01, lifted), equal to the JS value up to float rounding.

use crate::classify::Classified;
use crate::elevation::{trim_profile_d, PartArgs, Plan};
use crate::frame::{Edge, Frame};
use crate::js::*;
use crate::rng::Rng;

pub const CIVIC_CONCRETE_VERSION: &str = "sf-archetype-civic-concrete-v1";

const CLASSICAL_GROUND_M: f64 = 4.3;
const CLASSICAL_UPPER_M: f64 = 4.0;
const PIANO_FACTOR: f64 = 1.16;
const MODERN_GROUND_M: f64 = 4.0;
const MODERN_UPPER_M: f64 = 3.5;
const MIN_STOREY_M: f64 = 2.5;
const MIN_STOREY_FLOOR_M: f64 = 2.25;
const PODIUM_M: f64 = 0.46;
const CLASSICAL_PARAPET_M: f64 = 1.05;
const CLASSICAL_COPING_M: f64 = 0.16;
const MODERN_PARAPET_M: f64 = 0.72;
const MODERN_COPING_M: f64 = 0.1;
const CLASSICAL_BAY_M: (f64, f64) = (4.5, 5.4);
const MODERN_BAY_M: (f64, f64) = (5.9, 7.0);
const MIN_BAY_M: f64 = 2.5;
const MIN_ENTRY_BAY_M: f64 = 1.7;
const CLASSICAL_ENTRY_FACTOR: f64 = 1.42;
const MODERN_ENTRY_FACTOR: f64 = 1.25;
const END_PIER_M: f64 = 0.85;
const MIN_RETURN_EDGE_M: f64 = 20.0;
const MAX_RETURNS: usize = 1;
const RETURN_MAX_BAYS: f64 = 7.0;
const WALL_THICKNESS_M: f64 = 0.46;

const CLASSICAL_WALLS: [(&str, f64); 4] =
    [("limestone-ashlar", 0.44), ("cast-stone", 0.31), ("brick-common-bond", 0.15), ("brick-running-bond", 0.10)];
const MODERN_WALLS: [(&str, f64); 3] = [("concrete-precast", 0.74), ("cast-stone", 0.16), ("limestone-ashlar", 0.10)];

fn pick_weighted(random: &mut Rng, table: &[(&'static str, f64)]) -> &'static str {
    let mut roll = random.next();
    for (name, weight) in table {
        roll -= weight;
        if roll <= 0.0 {
            return *name;
        }
    }
    table[0].0
}

fn pick_family(c: &Classified, front_length: f64, random: &mut Rng) -> &'static str {
    let mut weight = 0.56;
    let storeys = 1f64.max(js_round(c.height / 3.3));
    if storeys <= 2.0 {
        weight += 0.14;
    }
    if storeys >= 6.0 {
        weight -= 0.38;
    } else if storeys >= 5.0 {
        weight -= 0.16;
    }
    if front_length > 70.0 {
        weight -= 0.14;
    }
    if c.area > 4500.0 {
        weight -= 0.10;
    }
    let p = 0.86f64.min(0.06f64.max(weight));
    if random.next() < p {
        "classical"
    } else {
        "modern"
    }
}

struct StoreyStack {
    storeys: Vec<(f64, &'static str)>,
    floor_level: f64,
    parapet: f64,
    coping: f64,
}

fn storey_stack(height: f64, family: &str, want_podium: f64) -> StoreyStack {
    let classical = family == "classical";
    let mut parapet = if classical { CLASSICAL_PARAPET_M } else { MODERN_PARAPET_M };
    let mut coping = if classical { CLASSICAL_COPING_M } else { MODERN_COPING_M };
    let ground_target = if classical { CLASSICAL_GROUND_M } else { MODERN_GROUND_M };
    let upper_target = if classical { CLASSICAL_UPPER_M } else { MODERN_UPPER_M };
    let mut roof_above = parapet + coping;
    if height - roof_above < MIN_STOREY_FLOOR_M {
        let shrink = 0.16f64.max(height - MIN_STOREY_FLOOR_M) / roof_above;
        parapet *= shrink;
        coping *= shrink;
        roof_above = parapet + coping;
    }
    let floor_level = 0f64.max(want_podium.min(height - roof_above - MIN_STOREY_M));
    let usable = MIN_STOREY_FLOOR_M.max(height - roof_above - floor_level);
    let ground = ground_target.min(MIN_STOREY_FLOOR_M.max(usable * 0.52));
    if usable < ground + MIN_STOREY_FLOOR_M {
        return StoreyStack { storeys: vec![(usable, "ground")], floor_level, parapet, coping };
    }
    let remainder = usable - ground;
    let mut upper_count = 1f64.max(js_round(remainder / upper_target));
    while upper_count > 1.0 && remainder / upper_count < MIN_STOREY_M {
        upper_count -= 1.0;
    }
    let upper = remainder / upper_count;
    let mut storeys = vec![(ground, "ground")];
    let piano = upper * PIANO_FACTOR;
    let rest = if upper_count > 1.0 { (remainder - piano) / (upper_count - 1.0) } else { 0.0 };
    let n = upper_count as usize;
    if classical && upper_count >= 2.0 && rest >= MIN_STOREY_M + 0.1 {
        storeys.push((piano, "piano"));
        for _ in 1..n {
            storeys.push((rest, "upper"));
        }
    } else {
        for i in 0..n {
            storeys.push((upper, if i == 0 && classical { "piano" } else { "upper" }));
        }
    }
    StoreyStack { storeys, floor_level, parapet, coping }
}

fn end_pier_for(length: f64) -> f64 {
    END_PIER_M.min(0.05f64.max(length * 0.1))
}

fn symmetric_bays(length: f64, target: f64, entry_factor: f64) -> (Vec<f64>, f64, usize) {
    let end_pier = end_pier_for(length);
    let available = 0.3f64.max(length - 2.0 * end_pier);
    let mut count = 1f64.max(js_round(available / target));
    if count % 2.0 == 0.0 {
        count = if available / (count + 1.0) >= MIN_BAY_M { count + 1.0 } else { 1f64.max(count - 1.0) };
    }
    let count = count as usize;
    let entry_bay = (count - 1) / 2;
    let mut weights = vec![1.0; count];
    if count >= 3 {
        weights[entry_bay] = entry_factor;
    }
    let total: f64 = weights.iter().fold(0.0, |a, b| a + b);
    let widths = weights.iter().map(|w| (available * w * (1.0 - 1e-4)) / total).collect();
    (widths, end_pier, entry_bay)
}

fn classical_storey_override(width: f64, storey_height: f64, is_piano: bool) -> J {
    let clear = 0.6f64.max(storey_height - (if is_piano { 0.95 } else { 1.05 }) - 0.55);
    json!({
        "window": "punched",
        "openingWidth": 0.6f64.max(width * (if is_piano { 0.56 } else { 0.5 })),
        "sillHeight": if is_piano { 0.95 } else { 1.05 },
        "head": storey_height - 0.55,
        "mullions": 1,
        "transomAt": if is_piano { json!([clear * 0.66]) } else { J::Null },
    })
}

pub fn spec(c: &Classified, frame: &Frame) -> J {
    let mut random = Rng::from_num(c.seed as f64);
    let front_edge = frame.edge_for_role("front");
    let mut front_length = front_edge.map(|e| e.length).unwrap_or(0.0);
    if front_length == 0.0 || front_length.is_nan() {
        front_length = frame.edges.iter().fold(0.0, |l: f64, e| l.max(e.length));
    }
    if front_length == 0.0 || front_length.is_nan() {
        front_length = 12.0;
    }

    let family = pick_family(c, front_length, &mut random);
    let classical = family == "classical";
    let wall = pick_weighted(&mut random, if classical { &CLASSICAL_WALLS[..] } else { &MODERN_WALLS[..] });
    let band = if classical { CLASSICAL_BAY_M } else { MODERN_BAY_M };
    let bay_target = band.0 + random.next() * (band.1 - band.0);
    let st = storey_stack(c.height, family, if classical { PODIUM_M } else { 0.0 });
    let storeys = &st.storeys;

    let (widths, end_pier, entry_bay) =
        symmetric_bays(front_length, bay_target, if classical { CLASSICAL_ENTRY_FACTOR } else { MODERN_ENTRY_FACTOR });
    let has_entrance = widths[entry_bay] >= MIN_ENTRY_BAY_M;
    let retail = c.has_retail_ground;
    let ground_default = if retail { "storefront" } else { "punched" };

    let pilaster_roll = random.next() < 0.62;
    let wants_pilasters = classical && storeys.len() >= 2 && pilaster_roll;
    let spandrel = if classical {
        wall
    } else if random.next() < 0.32 {
        "glazed-tile"
    } else {
        "concrete-precast"
    };
    let wants_pediment = classical && random.next() < 0.55;

    let ground = storeys[0].0;
    let door_height = 2.1f64.max(3.1f64.min(ground - 0.8));

    let front_bays: Vec<J> = widths
        .iter()
        .enumerate()
        .map(|(i, &width)| {
            let mut per = Map::new();
            per.insert("0".into(), json!(if i == entry_bay && has_entrance { "entrance" } else { ground_default }));
            if classical {
                for (s, (h, use_)) in storeys.iter().enumerate().skip(1) {
                    per.insert(s.to_string(), classical_storey_override(width, *h, *use_ == "piano"));
                }
            }
            json!({ "width": width, "storeys": per })
        })
        .collect();

    let front = if classical {
        let mut f = json!({
            "bays": front_bays,
            "endPier": end_pier,
            "pierMin": 0.7,
            "window": "punched",
            "reveal": 0.22,
            "windowSpec": {
                "sillHeight": 1.05, "headClear": 0.55, "widthRatio": 0.5,
                "frameWidth": 0.07, "frameDepth": 0.09, "mullions": 1, "transomAt": null,
            },
            "sill": { "height": 0.14, "projection": 0.09, "overhang": 0.11, "profile": "sloped" },
            "lintel": { "height": 0.22, "projection": 0.07, "overhang": 0.14, "slot": "trim" },
            "cornice": { "height": 0.68, "projection": 0.34, "profile": "ogee" },
            "entranceSpec": {
                "width": 3.2f64.min(1.3f64.max(widths[entry_bay] * 0.55)),
                "height": door_height, "recess": 0.62, "transom": 0.55,
                "leaf": "double-solid", "frameWidth": 0.1,
            },
            "storefrontSpec": { "stallRiser": 0.5, "pier": 0.24, "mullionSpacing": 1.9, "transom": 0.5 },
        });
        if wants_pilasters {
            set(&mut f, "pilasters", json!({ "width": 0.62, "projection": 0.11, "ends": true }));
        }
        if storeys.len() >= 2 {
            set(&mut f, "stringCourses", json!([{ "atStorey": 1, "height": 0.2, "projection": 0.1, "profile": "flat" }]));
        }
        f
    } else {
        let courses: Vec<J> = (1..storeys.len())
            .map(|i| json!({ "atStorey": i, "height": 0.26, "projection": 0.13, "profile": "flat" }))
            .collect();
        json!({
            "bays": front_bays,
            "endPier": end_pier,
            "pierMin": 0.7,
            "window": "ribbon",
            "reveal": 0.34,
            "ribbonSpec": { "spandrelHeight": 1.0, "headClear": 0.52, "mullionSpacing": 3.8 },
            "windowSpec": {
                "sillHeight": 0.95, "headClear": 0.5, "widthRatio": 0.56,
                "frameWidth": 0.09, "frameDepth": 0.1, "mullions": 0, "transomAt": null,
            },
            "sill": false,
            "stringCourses": courses,
            "entranceSpec": {
                "width": 3.4f64.min(1.3f64.max(widths[entry_bay] * 0.6)),
                "height": door_height, "recess": 0.78, "transom": 0.6,
                "leaf": "double-glass", "frameWidth": 0.08,
            },
            "storefrontSpec": { "stallRiser": 0.45, "pier": 0.2, "mullionSpacing": 2.8, "transom": 0 },
        })
    };

    let mut by_edge = Map::new();
    let mut return_edges: Vec<usize> = vec![];
    {
        let long = |edges: Vec<&Edge>| -> Vec<(usize, f64)> {
            let mut v: Vec<(usize, f64)> =
                edges.into_iter().filter(|e| e.length >= MIN_RETURN_EDGE_M && !e.party).map(|e| (e.index, e.length)).collect();
            v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            v
        };
        let street = long(frame.edges_for_role("front-secondary"));
        let candidates: Vec<(usize, f64)> =
            if !street.is_empty() { street } else { long(frame.edges_for_role("back")) }.into_iter().take(MAX_RETURNS).collect();
        for (index, length) in candidates {
            let pier = end_pier_for(length);
            let count = RETURN_MAX_BAYS.min(1f64.max(js_round((length - 2.0 * pier) / (bay_target * 1.9))));
            let entry = if classical {
                json!({
                    "bays": { "count": count }, "endPier": pier, "pierMin": 0.7,
                    "window": "punched", "ground": "punched",
                    "windowSpec": {
                        "sillHeight": 1.1, "headClear": 0.6, "widthRatio": 0.44,
                        "frameWidth": 0.07, "frameDepth": 0.09, "mullions": 0, "transomAt": null,
                    },
                    "sill": false,
                    "cornice": { "height": 0.5, "projection": 0.22, "profile": "stepped" },
                })
            } else {
                json!({
                    "bays": { "count": count }, "endPier": pier, "pierMin": 0.7,
                    "window": "ribbon", "ground": "punched",
                    "reveal": 0.3,
                    "ribbonSpec": { "spandrelHeight": 1.1, "headClear": 0.6, "mullionSpacing": 4.4 },
                    "windowSpec": {
                        "sillHeight": 1.0, "headClear": 0.55, "widthRatio": 0.48,
                        "frameWidth": 0.09, "frameDepth": 0.1, "mullions": 0, "transomAt": null,
                    },
                    "sill": false,
                })
            };
            by_edge.insert(index.to_string(), entry);
            if !return_edges.contains(&index) {
                return_edges.push(index);
            }
        }
    }
    return_edges.sort_unstable();

    let storeys_j: Vec<J> = storeys.iter().map(|(h, u)| json!({ "height": h, "use": u })).collect();
    json!({
        "id": format!("sf-archetype-{}", c.seed),
        "seed": c.seed,
        "height": c.height,
        "storeys": storeys_j,
        "floorLevel": st.floor_level,
        "wallThickness": WALL_THICKNESS_M,
        "parapet": { "height": st.parapet, "thickness": 0.34 },
        "coping": { "height": st.coping, "overhang": if classical { 0.11 } else { 0.07 } },
        "materials": {
            "wall": wall,
            "parapet": wall,
            "spandrel": spandrel,
            "base": if classical { "cast-stone" } else { "concrete-precast" },
            "bulkhead": if classical { "cast-stone" } else { "concrete-precast" },
            "trim": if classical { "cast-stone-trim" } else { "concrete-precast" },
            "coping": "cast-stone",
            "glass": "glass-clear",
            "frame": if classical { "painted-steel-mid" } else { "anodized-aluminium" },
            "storefront": "anodized-aluminium",
            "door": if classical { "painted-steel-mid" } else { "anodized-aluminium" },
            "doorGlass": "glass-door",
            "recess": if classical { "cast-stone" } else { "concrete-precast" },
            "canopy": "concrete-precast",
            "metal": "painted-steel-mid",
            "roof": "asphalt-roof",
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
        "roof": { "slab": 0.34 },
        "archetype": {
            "kind": "civic-concrete",
            "version": CIVIC_CONCRETE_VERSION,
            "family": family,
            "wall": wall,
            "spandrel": spandrel,
            "retail": retail,
            "bayCount": widths.len(),
            "entryBay": if has_entrance { entry_bay as i64 } else { -1 },
            "entryBayWidth": widths[entry_bay],
            "podium": st.floor_level,
            "pilasters": wants_pilasters,
            "pediment": wants_pediment,
            "doorHeight": door_height,
            "returnEdges": return_edges,
        },
    })
}

#[derive(Clone, Copy)]
struct BayRec {
    s0: f64,
    s1: f64,
    width: f64,
}

/// The JS aperture fields the post-pass reads: s0, s1, y0 (lifted) and the
/// head `y0 + height` (see the file comment).
#[derive(Clone, Copy)]
struct Aperture {
    s0: f64,
    s1: f64,
    y0: f64,
    top: f64,
}

fn find_aperture(plan: &Plan, edge: usize, bay: i64) -> Option<Aperture> {
    let ap = plan.apertures.iter().find(|a| a.f("edge") == Some(edge as f64) && a.f("bay") == Some(bay as f64))?;
    let id = ap.s("id").unwrap_or("");
    let s0 = ap.num("s0");
    let s1 = ap.num("s1");
    let rec = plan.edges.iter().find(|e| e.index == edge).and_then(|e| e.openings.iter().find(|o| o.id == id));
    let y0 = rec.map(|o| o.y0).unwrap_or(f64::NAN);
    let leaf = plan.parts.iter().find(|p| p.kind == "door-leaf" && p.meta.s("aperture") == Some(id));
    let top = match leaf {
        Some(p) => p.y1 + 0.01,
        None => rec.map(|o| o.y1).unwrap_or(f64::NAN),
    };
    Some(Aperture { s0, s1, y0, top })
}

pub fn detail(plan: &mut Plan, frame: &Frame, spec: &J) {
    let Some(meta) = spec.get_nn("archetype") else { return };
    let Some(front) = frame.edge_for_role("front") else { return };
    let Some(edge_record) = plan.edges.iter().find(|e| e.index == front.index) else { return };
    let basis = front.basis;
    let base_y = if plan.base_y.is_nan() { 0.0 } else { plan.base_y };
    let Some(ground) = plan.storeys.first() else { return };
    let kind = meta.s("kind").unwrap_or("").to_string();

    let reach = 0.06f64.max(0.32f64.min(front.allowance - 0.06));
    let y_cap = plan.envelope.roof_top + base_y - 0.05;
    let floor_y = ground.y0 + base_y;
    let ground_top = ground.y1 + base_y;
    let entry_bay = meta.num("entryBay");
    let bay: Option<BayRec> = if entry_bay >= 0.0 {
        edge_record.bays.get(entry_bay as usize).map(|b| BayRec { s0: b.s0, s1: b.s1, width: b.width })
    } else {
        None
    };
    let classical = meta.s("family") == Some("classical");
    let fi = front.index;
    let entry_bay_i = entry_bay as i64;
    let above = edge_record
        .openings
        .iter()
        .find(|o| o.storey == 1 && o.bay as i64 == entry_bay_i && o.type_ == "punched")
        .map(|o| (o.s0, o.s1, o.y0));

    // The steps up the podium.
    let podium = meta.num("podium");
    if let Some(bay) = bay {
        if podium > 0.12 {
            let rise = podium;
            let steps = 2f64.max(js_round(rise / 0.16));
            let riser = rise / steps;
            let run = 0.12f64.max(reach - 0.04);
            let going = run / steps;
            let width = (bay.width + 0.5).min(1.6f64.max((bay.s1 - bay.s0) * 0.92));
            let mid = (bay.s0 + bay.s1) / 2.0;
            let s0 = mid - width / 2.0;
            let s1 = mid + width / 2.0;
            let mut stair = vec![[-0.05, base_y]];
            let mut i = 0.0;
            while i < steps {
                stair.push([run - i * going, base_y + i * riser]);
                stair.push([run - i * going, base_y + (i + 1.0) * riser]);
                i += 1.0;
            }
            stair.push([-0.05, base_y + rise]);
            plan.add_profile(
                &basis,
                PartArgs::profile("entry-steps", "base", s0, s1, stair).edge(fi).meta(json!({ "archetype": kind, "part": "entry-steps" })),
            );
            let cheek_top = y_cap.min(base_y + rise + 0.52);
            if cheek_top > base_y + rise + 0.1 {
                let cheek = vec![[-0.05, base_y], [run, base_y], [run, base_y + rise * 0.5], [run * 0.2, cheek_top], [-0.05, cheek_top]];
                for cs in [s0 - 0.16, s1] {
                    plan.add_profile(
                        &basis,
                        PartArgs::profile("entry-cheek", "trim", cs, cs + 0.16, cheek.clone())
                            .edge(fi)
                            .meta(json!({ "archetype": kind, "part": "entry-cheek" })),
                    );
                }
            }
        }
    }

    let Some(bay) = bay else {
        record_handoff(plan, meta, front, None);
        return;
    };

    let aperture = find_aperture(plan, fi, entry_bay_i);
    let open_top = match aperture {
        Some(a) => a.top,
        None => floor_y + meta.num("doorHeight"),
    };
    let m = |part: &str| json!({ "archetype": kind, "part": part });

    if classical {
        let col_top = (y_cap - 0.42).min(open_top + 0.34);
        if col_top > floor_y + 1.4 {
            let col_width = 0.58f64.min(0.24f64.max(bay.width * 0.12));
            let shaft_reach = 0.04f64.max(reach - 0.06);
            let jambs = match aperture {
                Some(a) => [a.s0 - col_width - 0.08, a.s1 + 0.08],
                None => [bay.s0 + 0.12, bay.s1 - 0.12 - col_width],
            };
            for cs in jambs {
                plan.add_box(
                    &basis,
                    PartArgs::new("column-plinth", "base", cs - 0.06, cs + col_width + 0.06, floor_y, floor_y + 0.34, -0.02, reach)
                        .edge(fi)
                        .meta(m("column-plinth")),
                );
                plan.add_box(
                    &basis,
                    PartArgs::new("column", "trim", cs, cs + col_width, floor_y + 0.34, col_top - 0.18, -0.02, shaft_reach).edge(fi).meta(m("column")),
                );
                plan.add_box(
                    &basis,
                    PartArgs::new("column-capital", "trim", cs - 0.07, cs + col_width + 0.07, col_top - 0.18, col_top, -0.02, reach)
                        .edge(fi)
                        .meta(m("column-capital")),
                );
            }
            let arch_top = y_cap.min(col_top + 0.4);
            if arch_top > col_top + 0.12 {
                let es0 = 0.02f64.max(bay.s0 - 0.1);
                let es1 = (front.length - 0.02).min(bay.s1 + 0.1);
                plan.add_profile(
                    &basis,
                    PartArgs::profile("entablature", "trim", es0, es1, trim_profile_d("stepped", reach, arch_top - col_top, col_top))
                        .edge(fi)
                        .meta(m("entablature")),
                );
                plan.add_box(
                    &basis,
                    PartArgs::new(
                        "dedication-panel",
                        "trim",
                        es0 + 0.3,
                        es1 - 0.3,
                        (y_cap - 0.02).min(col_top - 0.62),
                        y_cap.min(col_top - 0.24),
                        -0.02,
                        0.03f64.max(reach - 0.14),
                    )
                    .edge(fi)
                    .meta(m("dedication-panel")),
                );
                let peak_wanted = (es1 - es0) * 0.17;
                if meta.truthy("pediment") && arch_top + peak_wanted <= y_cap {
                    plan.add_plate(
                        &basis,
                        PartArgs {
                            kind: "pediment".into(),
                            slot: "trim".into(),
                            edge: Some(fi),
                            profile: vec![[es0 - 0.12, arch_top], [es1 + 0.12, arch_top], [(es0 + es1) / 2.0, arch_top + peak_wanted]],
                            w0: -0.02,
                            w1: 0.03f64.max(reach - 0.1),
                            meta: Some(m("pediment")),
                            ..Default::default()
                        },
                    );
                }
            }
        }
        if let Some((a_s0, a_s1, a_y0)) = above {
            if a_y0 + 0.9 < y_cap {
                let sill_y = a_y0;
                plan.add_box(
                    &basis,
                    PartArgs::new("balconet-slab", "trim", a_s0 - 0.24, a_s1 + 0.24, sill_y - 0.16, sill_y, -0.02, reach)
                        .edge(fi)
                        .meta(m("balconet-slab")),
                );
                let rail_y = y_cap.min(sill_y + 0.88);
                if rail_y > sill_y + 0.4 {
                    for ps in [a_s0 - 0.18, a_s1 + 0.12] {
                        plan.add_box(
                            &basis,
                            PartArgs::new("balconet-rail", "metal", ps, ps + 0.06, sill_y, rail_y, 0f64.max(reach - 0.14), 0.03f64.max(reach - 0.08))
                                .edge(fi)
                                .shadow(false)
                                .meta(m("balconet-rail")),
                        );
                    }
                    plan.add_box(
                        &basis,
                        PartArgs::new(
                            "balconet-rail",
                            "metal",
                            a_s0 - 0.18,
                            a_s1 + 0.18,
                            rail_y - 0.06,
                            rail_y,
                            0f64.max(reach - 0.15),
                            0.03f64.max(reach - 0.07),
                        )
                        .edge(fi)
                        .shadow(false)
                        .meta(m("balconet-rail")),
                    );
                }
            }
        }
    } else {
        let canopy_y = (y_cap - 0.24).min((open_top + 0.18).max(floor_y + 2.7));
        if canopy_y > floor_y + 1.9 && canopy_y + 0.2 <= y_cap {
            let project = 1.25;
            let cs0 = 0.1f64.max(bay.s0 - 0.75);
            let cs1 = (front.length - 0.1).min(bay.s1 + 0.75);
            plan.add_box(
                &basis,
                PartArgs::new("canopy", "canopy", cs0, cs1, canopy_y, canopy_y + 0.2, -0.02, project)
                    .edge(fi)
                    .shadow(true)
                    .receive(true)
                    .meta(m("entry-canopy")),
            );
            let rod_top = y_cap.min(canopy_y + 0.95);
            if rod_top > canopy_y + 0.3 {
                for rs in [cs0 + 0.2, cs1 - 0.24] {
                    plan.add_box(
                        &basis,
                        PartArgs::new("canopy-rod", "metal", rs, rs + 0.05, canopy_y + 0.2, rod_top, project - 0.16, project - 0.11)
                            .edge(fi)
                            .shadow(false)
                            .meta(m("canopy-tie")),
                    );
                    plan.add_box(
                        &basis,
                        PartArgs::new("canopy-rod", "metal", rs, rs + 0.05, rod_top - 0.05, rod_top, 0.0, project - 0.11)
                            .edge(fi)
                            .shadow(false)
                            .meta(m("canopy-tie")),
                    );
                }
            }
            let blade_top = y_cap.min((ground_top - 0.1).min(canopy_y + 0.2));
            if blade_top > floor_y + 1.6 {
                for bs in [bay.s0 - 0.02, bay.s1 - 0.32] {
                    plan.add_box(
                        &basis,
                        PartArgs::new("entry-blade", "base", bs, bs + 0.34, floor_y, blade_top, -0.02, reach).edge(fi).meta(m("entry-blade")),
                    );
                }
            }
            let panel_y1 = y_cap.min(canopy_y - 0.12);
            if panel_y1 - 0.42 > open_top + 0.04 {
                plan.add_box(
                    &basis,
                    PartArgs::new("dedication-panel", "trim", bay.s0 + 0.4, bay.s1 - 0.4, panel_y1 - 0.42, panel_y1, -0.02, 0.03f64.max(reach - 0.16))
                        .edge(fi)
                        .meta(m("dedication-panel")),
                );
            }
        }
        if let Some(a) = aperture {
            let band = 0.22;
            let head_y = y_cap.min(a.top + band + 0.1);
            if head_y > a.y0 + 1.2 {
                let proud = 0.03f64.max(reach - 0.08);
                for (js0, js1) in [(a.s0 - band, a.s0), (a.s1, a.s1 + band)] {
                    plan.add_box(
                        &basis,
                        PartArgs::new("entry-architrave", "trim", js0, js1, floor_y, head_y, -0.02, proud).edge(fi).meta(m("entry-architrave")),
                    );
                }
                plan.add_box(
                    &basis,
                    PartArgs::new("entry-architrave", "trim", a.s0 - band, a.s1 + band, head_y - band, head_y, -0.02, proud)
                        .edge(fi)
                        .meta(m("entry-architrave")),
                );
            }
        }
    }

    record_handoff(plan, meta, front, Some(bay));
}

fn record_handoff(plan: &mut Plan, meta: &J, front: &Edge, bay: Option<BayRec>) {
    let [ox, oz] = front.basis.origin;
    let [ux, uz] = front.basis.u;
    let [nx, nz] = front.basis.n;
    let entrance = match bay {
        Some(b) => {
            let centre = (b.s0 + b.s1) / 2.0;
            json!({
                "edge": front.index,
                "width": b.width,
                "formal": true,
                "world": { "x": ox + ux * centre, "z": oz + uz * centre },
                "outward": [nx, nz],
            })
        }
        None => J::Null,
    };
    plan.extra.insert(
        "archetype".into(),
        json!({
            "kind": meta.at("kind"),
            "version": meta.at("version"),
            "family": meta.at("family"),
            "retail": meta.at("retail"),
            "bayCount": meta.at("bayCount"),
            "podium": meta.at("podium"),
            "entrance": entrance,
        }),
    );
}
