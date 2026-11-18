//! Port of src/world/buildings/archetypes/period-paint.js: the seeded body
//! paint for the timber-era fronts (Victorian rows, Edwardian flats), as the
//! `plan.tints` object `{ [wallFinish]: 0xRRGGBB }`, or None for white.

use crate::js::*;
use crate::rng::Rng;

/// Painted timber and stucco bodies (tint, weight).
const BODY: [(u32, f64); 10] = [
    (0xffffff, 3.0), // white
    (0xf1e7d2, 2.0), // cream
    (0xc8cacc, 2.0), // pale grey
    (0x5c6066, 2.0), // charcoal
    (0x5d6b80, 2.0), // slate navy
    (0xa2ad90, 2.0), // sage
    (0x858b66, 1.0), // olive
    (0x7c9894, 1.0), // teal
    (0x8a6763, 1.0), // oxblood, weathered
    (0xc4b38a, 1.0), // ochre
];
/// Shingle fronts: stained cedar first, then a painted shingle.
const SHINGLE: [(u32, f64); 3] = [(0x8a6a4e, 3.0), (0x5c6066, 1.0), (0xffffff, 1.0)];

fn pick_weighted(roll: f64, list: &[(u32, f64)]) -> u32 {
    let total: f64 = list.iter().fold(0.0, |t, e| t + e.1);
    let mut at = roll * total;
    for e in list {
        at -= e.1;
        if at < 0.0 {
            return e.0;
        }
    }
    list[list.len() - 1].0
}

/// `periodPaintTints(seed, wall)`: `{ [wall]: tint }` or None for white.
/// With the facade typology on, a body draws from `typology::PERIOD_BODY`
/// (the pastels as well), and a house of the Postcard Row (`postcard`, its
/// place in the row) takes its own (`typology::POSTCARD_BODIES`).
pub fn period_paint_tints(seed: &str, wall: Option<&str>) -> Option<J> {
    period_paint_tints_at(seed, wall, None)
}

/// `period_paint_tints` for a house of the Postcard Row at `postcard`.
pub fn period_paint_tints_at(seed: &str, wall: Option<&str>, postcard: Option<usize>) -> Option<J> {
    let wall = wall.filter(|w| !w.is_empty())?;
    let roll = Rng::new(&format!("{seed}|period-paint")).next();
    let typology = crate::typology::enabled();
    let body: &[(u32, f64)] = if typology { &crate::typology::PERIOD_BODY } else { &BODY };
    let mut tint = pick_weighted(roll, if wall == "painted-shingle" { &SHINGLE } else { body });
    if let (true, Some(i)) = (typology, postcard) {
        tint = crate::typology::POSTCARD_BODIES[i % crate::typology::POSTCARD_BODIES.len()];
    }
    if tint == 0xffffff {
        return None;
    }
    let mut m = Map::new();
    m.insert(wall.to_string(), json!(tint));
    Some(J::Object(m))
}
