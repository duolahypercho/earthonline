//! The archetype LOD ladder (src/world/buildings/archetypes/lod.js).

use crate::elevation::{Fenestration, Part, Plan, Shape};
use crate::frame::{Frame, P2};
use crate::js::*;

/// Plan slot holding the painted-fenestration variant of the wall finish.
pub const FENESTRATED_WALL_SLOT: &str = "fenestrated:wall";
/// Outer edge of each rung, metres (`LOD_DISTANCE_M`).
pub const LOD_DISTANCE_M: [f64; 3] = [65.0, 170.0, 470.0];

const LOD1_DROP: &[&str] = &[
    "mullion", "transom", "dentil", "jamb", "corbel", "bay-sash", "fire-escape-rail", "fire-escape-ladder", "slat", "blade-bracket", "gate-picket",
    "roof-solar-rack", "shop-detail", "sign-lamp",
];
/// In high-detail mode (`typology::detail_enabled()`), retain window mullions,
/// transoms, casings (`jamb`), bay sashes, corbels, and fire-escape rails across
/// LOD1 (65-170 m) so street & hillside buildings keep crisp sculpted depth.
const LOD1_DETAIL_DROP: &[&str] = &[
    "dentil", "fire-escape-ladder", "slat", "blade-bracket", "gate-picket",
    "roof-solar-rack", "shop-detail", "sign-lamp", "stoop-tread", "roller-hardware",
];
const LOD2_EXTRA_DROP: &[&str] = &[
    "frame", "sill", "cornice-bracket", "window-head", "window-surround", "door-frame", "recess", "door-leaf", "stall-riser", "threshold", "signband", "blade-sign",
    "sign-text", "canopy-rod", "fire-escape", "gate-rail", "gate-post", "roof-vent", "roof-plant-detail", "roof-curb", "roof-solar-step",
];
/// In high-detail mode (`typology::detail_enabled()`), keep primary silhouette and
/// depth-reading architectural relief intact through LOD2 (170-470 m): cornices,
/// brackets, sills, window heads/surrounds, recessed jambs, bay caps/posts,
/// canopies, awnings, rooftop water tanks, penthouses, HVAC yards, chimneys, and
/// parapet rails stay while fine sub-decimeter trim is dropped.
const LOD2_DETAIL_EXTRA_DROP: &[&str] = &[
    "frame", "mullion", "transom", "bay-sash", "door-leaf", "threshold", "signband", "blade-sign", "sign-text", "canopy-rod",
    "fire-escape-rail", "gate-rail", "gate-post", "roof-vent", "roof-plant-detail", "roof-solar-step",
    "pier-bevel", "bmu-track", "roof-deck-rail", "stoop-rail", "stoop-tread", "roller-hardware", "porch-lantern",
];

pub fn lod1_drops(kind: &str) -> bool {
    if crate::typology::detail_enabled() {
        LOD1_DETAIL_DROP.contains(&kind)
    } else {
        LOD1_DROP.contains(&kind)
    }
}
pub fn lod2_drops(kind: &str) -> bool {
    if crate::typology::detail_enabled() {
        LOD1_DETAIL_DROP.contains(&kind) || LOD2_DETAIL_EXTRA_DROP.contains(&kind)
    } else {
        LOD1_DROP.contains(&kind) || LOD2_EXTRA_DROP.contains(&kind)
    }
}

/// `trianglesForPart`: boxes 12; swept and extruded 4N-4.
pub fn triangles_for_part(part: &Part) -> usize {
    match &part.shape {
        Shape::Prism(p) => (4 * p.len()).saturating_sub(4).max(4),
        Shape::Profile(p) => (4 * p.len()).saturating_sub(4).max(4),
        _ => 12,
    }
}

/// `reducePlanToLod(plan, rung)` for rungs 0-3.
pub fn reduce_plan_to_lod(plan: &Plan, frame: &Frame, rung: u32) -> Vec<Part> {
    match rung {
        0 => plan.parts.clone(),
        1 => collapse_wall_bands(plan.parts.iter().filter(|p| !lod1_drops(&p.kind)).cloned().collect()),
        2 => collapse_wall_bands(plan.parts.iter().filter(|p| !lod2_drops(&p.kind)).cloned().collect()),
        _ => envelope_parts(plan, frame),
    }
}

/// `collapseWallBands`: merge touching per-storey wall bands of one edge.
pub fn collapse_wall_bands(parts: Vec<Part>) -> Vec<Part> {
    const EPS: f64 = 1e-3;
    let mut out: Vec<Part> = vec![];
    // Groups in first-seen order (a JS Map iterates in insertion order).
    let mut keys: Vec<String> = vec![];
    let mut groups: std::collections::HashMap<String, Vec<Part>> = Default::default();
    for part in parts {
        if part.kind != "wall" || !part.is_box() || part.edge.is_none() {
            out.push(part);
            continue;
        }
        let key = format!(
            "{}|{}|{}|{}|{}|{}",
            part.edge.unwrap(),
            part.slot,
            to_fixed(part.s0, 3),
            to_fixed(part.s1, 3),
            crate::rng::js_num(part.w0),
            crate::rng::js_num(part.w1)
        );
        if !groups.contains_key(&key) {
            keys.push(key.clone());
        }
        groups.entry(key).or_default().push(part);
    }
    for key in keys {
        let mut group = groups.remove(&key).unwrap();
        group.sort_by(|a, b| a.y0.partial_cmp(&b.y0).unwrap_or(std::cmp::Ordering::Equal));
        let mut run: Option<usize> = None;
        for part in group {
            if let Some(r) = run {
                if part.y0 <= out[r].y1 + EPS {
                    out[r].y1 = out[r].y1.max(part.y1);
                    if let (Some(rf), Some(pf)) = (&out[r].fenestration, &part.fenestration) {
                        let v = [rf.v[0], pf.v[1]];
                        out[r].fenestration = Some(Fenestration { v, ..rf.clone() });
                    }
                    continue;
                }
            }
            out.push(part);
            run = Some(out.len() - 1);
        }
    }
    out
}

/// `envelopeParts(plan)`: the footprint extruded to the roof (the far rung).
pub fn envelope_parts(plan: &Plan, frame: &Frame) -> Vec<Part> {
    let points: Vec<P2> = match &frame.source_polygon {
        Some(p) if p.len() >= 3 => p.clone(),
        _ => frame.edges.iter().map(|e| e.start).collect(),
    };
    let base_y = plan.base_y;
    let storeys = plan.storeys.len();
    let fenestrated = plan.materials.contains_key(FENESTRATED_WALL_SLOT) && storeys >= 1;
    let template = Part {
        id: "lod3:envelope".into(),
        kind: "wall".into(),
        slot: if fenestrated { FENESTRATED_WALL_SLOT.into() } else { "wall".into() },
        shape: Shape::Prism(points.clone()),
        basis: Default::default(),
        s0: 0.0,
        s1: 0.0,
        y0: base_y,
        y1: base_y + plan.envelope.roof_top,
        w0: 0.0,
        w1: 0.0,
        edge: None,
        storey: None,
        bay: None,
        fitting: false,
        cast_shadow: true,
        receive_shadow: true,
        meta: J::Null,
        side_faces: true,
        top_cap: true,
        bottom_cap: false,
        fenestration: fenestrated.then_some(Fenestration { u: [0.0, 0.0], v: [0.0, 0.0], storeys: Some(storeys as f64) }),
    };
    let detail = crate::typology::detail_enabled();
    let silhouette: Vec<Part> = plan
        .parts
        .iter()
        .filter(|p| {
            p.kind == "bay"
                || (detail
                    && matches!(
                        p.kind.as_str(),
                        "oriel" | "roof-penthouse" | "roof-water-tank" | "roof-screen" | "roof-gable" | "gable" | "cooling-tower" | "roof-deck" | "roof-solar" | "roof-skylight" | "chimney" | "cornice"
                    ))
        })
        .cloned()
        .collect();
    let roof_slab = plan.parts.iter().find(|p| p.kind == "roof-slab" && plan.materials.contains_key(&p.slot));
    let Some(roof_slab) = roof_slab else {
        let mut out = vec![template];
        out.extend(silhouette);
        return out;
    };
    let mut envelope = template.clone();
    envelope.top_cap = false;
    let roof = Part {
        id: "lod3:roof".into(),
        kind: "roof-slab".into(),
        slot: roof_slab.slot.clone(),
        y0: template.y1 - 0.01,
        y1: template.y1,
        side_faces: false,
        top_cap: true,
        bottom_cap: false,
        fenestration: None,
        ..template.clone()
    };
    let mut out = vec![envelope, roof];
    if let Some(b) = plan.parts.iter().find(|p| p.kind == "roof-bulkhead" && p.is_box()) {
        let mut bulk = b.clone();
        bulk.id = "lod3:bulkhead".into();
        bulk.slot = roof_slab.slot.clone();
        bulk.cast_shadow = true;
        out.push(bulk);
    }
    out.extend(silhouette);
    out
}
