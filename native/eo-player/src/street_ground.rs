//! The drawn street surface (eo-street) as the collision ground: the player
//! stands on the same carriageway, gutter, kerb top, sidewalk and ramp the
//! renderer draws, and the kerb faces stop or step it, as the JS street
//! cross-section colliders do (`streetCrossSectionPhysicsDescriptor` in
//! src/simulation/physics/world/world-physics.js: kerb tops and walks walkable
//! on the STREET layer, kerb faces on WORLD).
//!
//! * `ground_y`: the drawn ground carpet (`StreetWorld::carpet_y`: terrain plus
//!   the open-ground lift, held under any street), sampled into the heightfield;
//! * `surface_tile`: the tile's near street mesh (`StreetWorld::cached_mesh`, the
//!   same 128 m grid as the collision tiles): walkable layers to STREET, kerb
//!   faces to WORLD; lane markings are skipped (paint lies on the carriageway).

use std::sync::Arc;

use eo_street::{BuildingIndex, Layer, StreetCity, StreetOptions, StreetWorld, Terrain};

use crate::world::{GroundProvider, SurfaceMesh, TileKey};

pub struct StreetGround(pub Arc<StreetWorld>);

impl StreetGround {
    /// Build the street world as eo-app/src/streets.rs does (for tests and tools).
    pub fn build(city: &eo_data::City, elevation: Arc<eo_data::ElevationGrid>) -> anyhow::Result<Self> {
        let o = StreetOptions::load()?;
        let sc = StreetCity::load()?;
        let plan = eo_street::plan::build_plan(&sc, &o);
        let rings = city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
        let lift = o.open_ground_lift;
        Ok(Self(Arc::new(StreetWorld::from_plan(o, sc, plan, Terrain { grid: elevation, carpet_lift: lift }, BuildingIndex::new(rings)))))
    }
}

impl GroundProvider for StreetGround {
    fn ground_y(&self, x: f64, z: f64) -> f64 {
        self.0.carpet_y(x, z)
    }

    fn surface_tile(&self, key: TileKey) -> Option<SurfaceMesh> {
        let m = self.0.cached_mesh((key.x, key.z));
        if m.indices.is_empty() {
            return None;
        }
        let mut s = SurfaceMesh::default();
        let p = |i: u32| {
            let v = m.positions[i as usize];
            [v[0] as f64, v[1] as f64, v[2] as f64]
        };
        for (t, layer) in m.indices.chunks_exact(3).zip(m.layers.iter()) {
            let tri = [p(t[0]), p(t[1]), p(t[2])];
            match layer {
                Layer::CurbFace => s.barrier.push(tri),
                Layer::Marking => {}
                l if l.walkable() => s.street.push(tri),
                _ => {}
            }
        }
        Some(s)
    }
}
