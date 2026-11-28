//! Vehicle-carrying movable leaves missing between fixed bridge approaches in
//! the original city extract. Both road loaders and the street plan use them.
use std::sync::OnceLock;

use serde::Deserialize;

use crate::{RawRoad, RecordId};

#[derive(Deserialize)]
pub struct Span {
    pub approach: String,
    #[serde(flatten)]
    pub road: RawRoad,
}

pub fn spans() -> &'static [Span] {
    static DATA: OnceLock<Vec<Span>> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!("../data/movable-road-spans.json"))
            .expect("versioned movable road spans")
    })
}

pub fn is_leaf(way: &str) -> bool {
    let Ok(id) = way.parse::<u64>() else {
        return false;
    };
    spans().iter().any(|s| s.road.id == RecordId::Num(id))
}

fn osm_id(id: &RecordId) -> Option<u64> {
    match id {
        RecordId::Num(value) => Some(*value),
        RecordId::Str(key) => key.parse().ok(),
    }
}

pub(crate) fn add_to(roads: &mut Vec<RawRoad>) {
    for span in spans() {
        let approach = span.approach.parse().expect("source approach ID");
        let leaf = osm_id(&span.road.id).expect("source movable span ID");
        let present = |id| roads.iter().any(|r| osm_id(&r.id) == Some(id));
        if present(approach) && !present(leaf) {
            roads.push(span.road.clone());
        }
    }
}
