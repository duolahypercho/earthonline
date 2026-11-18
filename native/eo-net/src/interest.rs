//! Interest management: which entities a client is told about.
//!
//! Every replicated entity is bucketed by its 64 m cell, keyed by
//! `CellKey { zone, x, z }`: the zone is the local frame (one per city, or
//! per Earth tile later), the cell indices are integers in that frame, so
//! nothing here is sized to one city. A client's set is the cells its
//! radii touch, filtered by distance per class (cars to the drawn traffic
//! window, walkers to the far crowd tier, players farther), with hysteresis
//! so an entity on the edge does not flicker in and out.

use crate::snapshot::{CLASS_CAR, CLASS_PLAYER, CLASS_PLAYER_CAR, CLASS_WALKER, QState, class_of};
use std::collections::HashMap;

pub const CELL_M: f64 = 64.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CellKey {
    pub zone: u16,
    pub x: i32,
    pub z: i32,
}

impl CellKey {
    pub fn of(zone: u16, x: f64, z: f64) -> Self {
        CellKey { zone, x: (x / CELL_M).floor() as i32, z: (z / CELL_M).floor() as i32 }
    }
}

/// Interest radii per class, metres (and the hysteresis added before an entity is dropped).
#[derive(Clone, Copy, Debug)]
pub struct Radii {
    pub car: f64,
    pub walker: f64,
    pub player: f64,
    pub player_car: f64,
    pub hysteresis: f64,
}

impl Default for Radii {
    fn default() -> Self {
        // Cars: the drawn traffic window (240 m, `trafficDistance` High) plus a margin;
        // walkers: the far crowd tier (240 m) would be 160 walkers a client in a
        // busy street; the mid tier's 145 m is where they are more than dots.
        Radii { car: 250.0, walker: 150.0, player: 600.0, player_car: 400.0, hysteresis: 15.0 }
    }
}

impl Radii {
    pub fn of(&self, id: u32) -> f64 {
        match class_of(id) {
            CLASS_CAR => self.car,
            CLASS_WALKER => self.walker,
            CLASS_PLAYER => self.player,
            CLASS_PLAYER_CAR => self.player_car,
            _ => 0.0,
        }
    }
    pub fn max(&self) -> f64 {
        self.car.max(self.walker).max(self.player).max(self.player_car) + self.hysteresis
    }
}

/// Every entity this tick, bucketed by cell.
#[derive(Default)]
pub struct Grid {
    pub entities: Vec<(u32, QState)>,
    cells: HashMap<CellKey, Vec<u32>>,
    pub zone: u16,
}

impl Grid {
    pub fn rebuild(&mut self, zone: u16, entities: Vec<(u32, QState)>) {
        for v in self.cells.values_mut() {
            v.clear();
        }
        self.zone = zone;
        self.entities = entities;
        for (i, (_, s)) in self.entities.iter().enumerate() {
            let p = s.pos();
            self.cells.entry(CellKey::of(zone, p[0], p[2])).or_default().push(i as u32);
        }
        self.cells.retain(|_, v| !v.is_empty());
    }

    /// The entities within their class radius of (x, z) (or within radius +
    /// hysteresis if `held` says the client already has them), sorted by id,
    /// with a priority weight (nearer first).
    pub fn query(&self, x: f64, z: f64, radii: &Radii, held: impl Fn(u32) -> bool, out: &mut Vec<crate::snapshot::Desired>, must: Option<u32>) {
        out.clear();
        let reach = radii.max();
        let lo = CellKey::of(self.zone, x - reach, z - reach);
        let hi = CellKey::of(self.zone, x + reach, z + reach);
        for cx in lo.x..=hi.x {
            for cz in lo.z..=hi.z {
                // Skip cells wholly beyond reach.
                let (x0, z0) = (cx as f64 * CELL_M, cz as f64 * CELL_M);
                let dx = (x0 - x).max(0.0).max(x - (x0 + CELL_M));
                let dz = (z0 - z).max(0.0).max(z - (z0 + CELL_M));
                if dx * dx + dz * dz > reach * reach {
                    continue;
                }
                let Some(list) = self.cells.get(&CellKey { zone: self.zone, x: cx, z: cz }) else { continue };
                for &i in list {
                    let (id, s) = self.entities[i as usize];
                    let p = s.pos();
                    let d = (p[0] - x).hypot(p[2] - z);
                    let r = radii.of(id) + if held(id) { radii.hysteresis } else { 0.0 };
                    if d > r && Some(id) != must {
                        continue;
                    }
                    let class = class_of(id);
                    let weight = if class == CLASS_PLAYER || class == CLASS_PLAYER_CAR && d < 60.0 {
                        1000.0 + (1.0 / (1.0 + d / 50.0)) as f32
                    } else {
                        let speed = s.speed_mps().abs();
                        ((1.0 + speed.min(20.0) / 4.0) * (1.0 / (1.0 + d / 25.0))) as f32 + 0.05
                    };
                    out.push(crate::snapshot::Desired { id, state: s, weight });
                }
            }
        }
        out.sort_unstable_by_key(|d| d.id);
        out.dedup_by_key(|d| d.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{net_id, q_pos};

    #[test]
    fn radii_and_hysteresis() {
        let mut g = Grid::default();
        let e = |c, k, x: f64| (net_id(c, k), QState { x: q_pos(x), ..Default::default() });
        g.rebuild(0, vec![e(CLASS_CAR, 1, 100.0), e(CLASS_CAR, 2, 255.0), e(CLASS_WALKER, 3, 140.0), e(CLASS_WALKER, 4, 200.0), e(CLASS_PLAYER, 5, 590.0), e(CLASS_PLAYER, 6, 700.0)]);
        let r = Radii::default();
        let mut out = vec![];
        g.query(0.0, 0.0, &r, |_| false, &mut out, None);
        assert_eq!(out.iter().map(|d| d.id).collect::<Vec<_>>(), vec![net_id(CLASS_CAR, 1), net_id(CLASS_WALKER, 3), net_id(CLASS_PLAYER, 5)]);
        g.query(0.0, 0.0, &r, |_| true, &mut out, None);
        assert!(out.iter().any(|d| d.id == net_id(CLASS_CAR, 2)), "held entity kept inside the hysteresis");
    }
}
