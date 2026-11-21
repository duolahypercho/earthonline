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
//!
//! `structures` holds the grade-separated roads (eo-structures) when the
//! world has them (`--structures`). When it is there and not empty:
//!
//! * `surface_tile` adds the tile's structure triangles
//!   (`StructureWorld::surface_tile_shared`): deck tops (built decks, the
//!   open roads below other ways, the landmark bridges' decks) to STREET,
//!   their concrete (barriers, fascia, girders, caps, columns, fill,
//!   abutment and end walls) to WORLD; also in a tile with no street mesh
//!   (a deck over the water, a freeway-only tile);
//! * `multi_level` is true, so the collision world's level queries pick the
//!   surface nearest below a body (a deck, or the street under it).
//!
//! * with `--tunnels` (built tunnels), `terrain_cut` answers where a
//!   tunnel's open stretches cut through the ground
//!   (`StructureWorld::terrain_cut_at`): the collision heightfield leaves
//!   those cells out and the floor there is the cut's floor; the bores'
//!   roadway is STREET, their walls, vault and portals WORLD (in
//!   `surface_tile`, with the decks').
//!
//! `build_joined` builds the street with the structures' joins (stage A6,
//! `eo_structures::build_joined`: junction mouths for the structure legs,
//! capped and raised stubs); a host and its server must build the same way.
//!
//! Without it (or with an empty world) every tile and query is as before.

use std::sync::Arc;

use eo_street::{BuildingIndex, Layer, StreetCity, StreetOptions, StreetWorld, Terrain};
use eo_structures::{StructureOptions, StructureWorld};

use crate::world::{GroundProvider, SurfaceMesh, TileKey};

pub struct StreetGround {
    /// The street surface the renderer draws.
    pub world: Arc<StreetWorld>,
    /// The grade-separated roads over and under it, when built (`--structures`).
    pub structures: Option<Arc<StructureWorld>>,
}

impl StreetGround {
    /// The ground over a street world a host already has, with its
    /// grade-separated roads when it has them.
    pub fn new(world: Arc<StreetWorld>, structures: Option<Arc<StructureWorld>>) -> Self {
        Self { world, structures }
    }

    /// Build the street world as eo-app/src/streets.rs does (for tests and tools), without structures.
    pub fn build(city: &eo_data::City, elevation: Arc<eo_data::ElevationGrid>) -> anyhow::Result<Self> {
        Self::build_with(city, elevation, None)
    }

    /// Build the street world as eo-app/src/streets.rs does, and with
    /// `structures` the grade-separated roads over it as eo-app/src/structures.rs does.
    pub fn build_with(city: &eo_data::City, elevation: Arc<eo_data::ElevationGrid>, structures: Option<StructureOptions>) -> anyhow::Result<Self> {
        let o = StreetOptions::load()?;
        let sc = StreetCity::load()?;
        let plan = eo_street::plan::build_plan(&sc, &o);
        let rings = city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
        let lift = o.open_ground_lift;
        let world = Arc::new(StreetWorld::from_plan(o, sc, plan, Terrain { grid: elevation, carpet_lift: lift }, BuildingIndex::new(rings)));
        let structures = match structures {
            Some(opts) => {
                let ca = eo_data::CaStandards::load(&eo_data::DataPaths::discover()?)?;
                Some(Arc::new(StructureWorld::build(&world, city, &ca, eo_landmarks::LandmarksJs::get(), opts)?))
            }
            None => None,
        };
        Ok(Self::new(world, structures))
    }

    /// `build_with`, the street built again with the structures' joins
    /// (stage A6, `eo_structures::build_joined`) when structures are asked
    /// for: the street world and the grade-separated roads a client and a
    /// server must both hold (each builds them the same way).
    pub fn build_joined(city: &eo_data::City, elevation: Arc<eo_data::ElevationGrid>, structures: Option<StructureOptions>) -> anyhow::Result<Self> {
        let Some(opts) = structures else { return Self::build_with(city, elevation, None) };
        let first = Self::build_with(city, elevation, None)?;
        let ca = eo_data::CaStandards::load(&eo_data::DataPaths::discover()?)?;
        let joined = eo_structures::build_joined(&first.world, city, &ca, eo_landmarks::LandmarksJs::get(), opts)?;
        let world = match joined.streets {
            Some(s) => Arc::new(s),
            None => first.world,
        };
        Ok(Self::new(world, Some(Arc::new(joined.structures))))
    }

    /// The grade-separated roads, when there are any (`--structures` and a
    /// built deck).
    pub fn decks(&self) -> Option<&StructureWorld> {
        self.structures.as_deref().filter(|s| !s.is_empty())
    }
}

impl GroundProvider for StreetGround {
    fn ground_y(&self, x: f64, z: f64) -> f64 {
        self.world.carpet_y(x, z)
    }

    fn multi_level(&self) -> bool {
        self.decks().is_some()
    }

    fn terrain_cut(&self, x: f64, z: f64) -> Option<f64> {
        self.decks()?.terrain_cut_at(x, z).map(|c| c.floor)
    }

    fn surface_tile(&self, key: TileKey) -> Option<SurfaceMesh> {
        let m = self.world.cached_mesh((key.x, key.z));
        // (before the street's own early return: a tile with only a deck,
        // over the water or freeway-only, still collides.)
        let deck = self.decks().and_then(|w| w.surface_tile_shared((key.x, key.z))).filter(|d| !d.street.is_empty() || !d.barrier.is_empty());
        if m.indices.is_empty() && deck.is_none() {
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
        if let Some(d) = deck {
            s.street.extend_from_slice(&d.street);
            s.barrier.extend_from_slice(&d.barrier);
        }
        Some(s)
    }
}
