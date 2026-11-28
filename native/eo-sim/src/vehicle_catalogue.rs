//! Stable vehicle selection shared by the game and headless traffic replays.

use crate::output::{KIND_BUS, KIND_TAXI, KIND_TRUCK};
use crate::rng::Key;
mod profiles;
pub use profiles::{Bounds, ChassisGeometry, ScaleReport, Vehicle};

/// Catalogue seed shared by the local and networked game feeds.
pub const RENDERED_SEED: u32 = 1;

pub fn kind_name(kind: u16) -> &'static str {
    match kind {
        KIND_TAXI => "taxi",
        KIND_TRUCK => "truck",
        KIND_BUS => "bus",
        _ => "sedan",
    }
}

pub fn pick(seed: u32, entity: u32, kind: u16, pool: &[String]) -> Option<&str> {
    if pool.is_empty() {
        return None;
    }
    let seed = if seed == 0 { 1 } else { seed };
    let draw = Key::new()
        .int(u64::from(seed))
        .str(":")
        .int(u64::from(entity))
        .sep()
        .str("vehicle-catalogue:")
        .str(kind_name(kind))
        .draw();
    Some(&pool[(draw * pool.len() as f64) as usize])
}

/// Select the same weighted entry on every feed. A partial asset pack first
/// falls back to its loaded entries, then to the sedan with the chosen paint.
pub fn resolve<T>(
    seed: u32,
    entity: u32,
    kind: u16,
    pool: &[String],
    lookup: impl Fn(&str) -> Option<T>,
) -> Option<(&str, T)> {
    let body = pick(seed, entity, kind, pool).unwrap_or("sedan");
    if let Some(model) = lookup(body) {
        return Some((body, model));
    }
    let pool: Vec<_> = pool
        .iter()
        .filter_map(|id| lookup(id).map(|v| (id.as_str(), v)))
        .collect();
    if pool.is_empty() {
        return lookup("sedan").map(|v| (body, v));
    }
    let index = entity as usize % pool.len();
    pool.into_iter().nth(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_draw_keeps_the_game_catalogue_keys_and_weights() {
        let pool = ["sedan", "sedan", "wagon", "minivan"].map(str::to_owned);
        for (kind, name) in ["sedan", "taxi", "truck", "bus"].into_iter().enumerate() {
            for seed in [0u32, 1, 7, u32::MAX] {
                for entity in [0u32, 1, 10, 99_956, u32::MAX] {
                    let draw = crate::rng::keyed_random(
                        &format!("{}:{entity}", seed.max(1)),
                        &format!("vehicle-catalogue:{name}"),
                    );
                    assert_eq!(
                        pick(seed, entity, kind as u16, &pool),
                        Some(pool[(draw * pool.len() as f64) as usize].as_str())
                    );
                }
            }
        }
    }

    #[test]
    fn partial_asset_pool_falls_back_to_the_same_loaded_model() {
        let pool = ["missing", "wagon", "wagon", "minivan"].map(str::to_owned);
        let entity = (0..100)
            .find(|&id| pick(7, id, 0, &pool) == Some("missing"))
            .unwrap();
        let loaded = ["wagon", "wagon", "minivan"];
        let expected = loaded[entity as usize % loaded.len()];
        assert_eq!(
            resolve(7, entity, 0, &pool, |name| match name {
                "wagon" => Some(1),
                "minivan" => Some(2),
                _ => None,
            }),
            Some((expected, if expected == "wagon" { 1 } else { 2 }))
        );
        assert_eq!(
            resolve(7, entity, 0, &pool, |name| (name == "sedan").then_some(0)),
            Some(("missing", 0))
        );
    }
}
