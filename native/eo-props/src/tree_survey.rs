//! The game's DataSF street tree list (`props-js.json.gz` `trees`, exported
//! 2026-10-20) joined to the current list's records (eo-data `tree_sizes`,
//! `public/data/sf/sf-tree-sizes.json.gz`): each list tree takes the record
//! nearest its point within `JOIN_M` (the game's points are decimetres on the
//! ground-plan scale; the join is in the city data's metres), and from it its
//! full species (`--tree-species`) and its size from the recorded trunk
//! diameter (`--tree-sizes`, the Urban Tree Database allometry). A list tree
//! with no record within `JOIN_M`, no DBH, or a palm keeps what it had.
//!
//! Sizes stay in real metres, as the list's crowns and heights always were:
//! the ground-plan scale (`eo_data::groundscale`) moves a tree's point, never
//! its model (props stay real size).

use crate::PropsWorld;
use eo_data::tree_sizes::TreeSizes;

/// The farthest a list tree's point may be from its record (city data metres).
pub const JOIN_M: f64 = 1.0;

pub struct TreeJoin {
    pub table: TreeSizes,
    /// Per list tree: the record joined (index into `table.trees`).
    pub record: Vec<Option<u32>>,
    /// Per list tree: (crown diameter, height) in metres from its DBH, where sized.
    pub size: Vec<Option<[f32; 2]>>,
    pub joined: usize,
    pub sized: usize,
}

impl TreeJoin {
    pub fn new(w: &PropsWorld, table: TreeSizes) -> Self {
        let t = &w.js.trees;
        let n = t.data.len() / t.stride;
        let gs = eo_data::groundscale::scale();
        let mut record = Vec::with_capacity(n);
        let mut size = Vec::with_capacity(n);
        let (mut joined, mut sized) = (0, 0);
        for i in 0..n {
            let r = crate::trees::record(w, i as u32);
            let k = table.nearest(r.x / gs, r.z / gs, JOIN_M);
            let s = k.and_then(|k| table.size(k)).map(|(c, h)| [c as f32, h as f32]);
            joined += k.is_some() as usize;
            sized += s.is_some() as usize;
            record.push(k.map(|k| k as u32));
            size.push(s);
        }
        TreeJoin { table, record, size, joined, sized }
    }

    /// The full species of list tree `i` ("Ulmus parvifolia :: Chinese Elm"), where joined.
    pub fn species(&self, i: u32) -> Option<&str> {
        let k = (*self.record.get(i as usize)?)?;
        self.table.species.get(self.table.trees[k as usize].species as usize).map(|s| s.as_str())
    }
}
