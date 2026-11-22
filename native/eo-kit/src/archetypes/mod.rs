//! The archetype modules (src/world/buildings/archetypes/<name>.js): each one
//! writes the planElevation spec for a classified record (`spec`) and adds its
//! own parts to the finished plan (`detail`).
//!
//! `plan<Name>` is exactly `detail(planElevation(spec(c, f), f, o), f, spec)`
//! in every JS module; the factory (pipeline.rs) runs the stages itself so the
//! lot-line and street-return passes can open walls between them.

use crate::classify::Classified;
use crate::elevation::Plan;
use crate::frame::Frame;
use crate::js::*;

pub mod bay_window_apartment;
pub mod civic_concrete;
pub mod cottage_garage;
pub mod downtown_tower;
pub mod edwardian_flats;
pub mod industrial_shed;
pub mod midcentury_stucco;
pub mod period_paint;
pub mod retail_strip;
pub mod soma_warehouse;
pub mod stucco_row;
pub mod victorian_row;

/// The spec for an archetype id, its window module drawn per building
/// (typology.rs; `EO_FACADE_TYPOLOGY=off`: as the port writes it).
pub fn spec_for(archetype: &str, c: &Classified, frame: &Frame) -> J {
    let mut spec = port_spec_for(archetype, c, frame);
    crate::typology::vary(c, &mut spec);
    spec
}

/// The spec for an archetype id as the port writes it.
pub fn port_spec_for(archetype: &str, c: &Classified, frame: &Frame) -> J {
    match archetype {
        crate::classify::BAY_WINDOW_APARTMENT => bay_window_apartment::spec(c, frame),
        crate::classify::MIDCENTURY_STUCCO => midcentury_stucco::spec(c, frame),
        crate::classify::VICTORIAN_ROW => victorian_row::spec(c, frame),
        crate::classify::RETAIL_STRIP => retail_strip::spec(c, frame),
        crate::classify::SOMA_WAREHOUSE => soma_warehouse::spec(c, frame),
        crate::classify::CIVIC_CONCRETE => civic_concrete::spec(c, frame),
        crate::classify::DOWNTOWN_TOWER => downtown_tower::spec(c, frame),
        crate::classify::INDUSTRIAL_SHED => industrial_shed::spec(c, frame),
        crate::classify::STUCCO_ROW => stucco_row::spec(c, frame),
        crate::classify::EDWARDIAN_FLATS => edwardian_flats::spec(c, frame),
        crate::classify::COTTAGE_GARAGE => cottage_garage::spec(c, frame),
        _ => J::Null,
    }
}

/// The archetype's own parts, added to the finished plan.
pub fn detail_for(archetype: &str, plan: &mut Plan, frame: &Frame, spec: &J) {
    match archetype {
        crate::classify::BAY_WINDOW_APARTMENT => bay_window_apartment::detail(plan, frame, spec),
        crate::classify::MIDCENTURY_STUCCO => midcentury_stucco::detail(plan, frame, spec),
        crate::classify::VICTORIAN_ROW => victorian_row::detail(plan, frame, spec),
        crate::classify::RETAIL_STRIP => retail_strip::detail(plan, frame, spec),
        crate::classify::SOMA_WAREHOUSE => soma_warehouse::detail(plan, frame, spec),
        crate::classify::CIVIC_CONCRETE => civic_concrete::detail(plan, frame, spec),
        crate::classify::DOWNTOWN_TOWER => downtown_tower::detail(plan, frame, spec),
        crate::classify::INDUSTRIAL_SHED => industrial_shed::detail(plan, frame, spec),
        crate::classify::STUCCO_ROW => stucco_row::detail(plan, frame, spec),
        crate::classify::EDWARDIAN_FLATS => edwardian_flats::detail(plan, frame, spec),
        crate::classify::COTTAGE_GARAGE => cottage_garage::detail(plan, frame, spec),
        _ => {}
    }
}

/// Placeholder spec used by a module that is not ported yet: the kit's
/// defaults (punched windows on the front, blind elsewhere) at the record's
/// height, in the module's own wall finish. Marked so tests can tell.
pub fn placeholder_spec(c: &Classified, wall: &str) -> J {
    json!({
        "id": format!("sf-archetype-{}", c.seed),
        "seed": c.seed,
        "height": c.height,
        "storeys": { "count": crate::classify::resolve_storey_count(c.height).max(1.0) },
        "materials": { "wall": wall },
        "elevations": { "front": { "window": "punched", "ground": if c.has_retail_ground { "storefront" } else { "punched" } }, "default": "blind" },
        "archetype": { "kind": c.archetype, "placeholder": true },
    })
}
