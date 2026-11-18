//! The game rules both ends share: the collision world over the city, how a
//! player is spawned, and the one authoritative step of a player's
//! controller. Server and client run exactly this code, so the same inputs
//! give the same body on both (tested in `tests/determinism.rs`).

use crate::wire::{Check, NetInput, check_pos};
use anyhow::Result;
use eo_player::world::{self as pworld, BuildingSet, CollisionWorld, ElevationGround, GroundProvider};
use eo_player::{Mode, Player};
use glam::DVec3;
use std::sync::Arc;

/// The city as the player's physics sees it (built as eo-app's `--play` builds it).
pub struct GameWorld {
    pub city: Arc<eo_data::City>,
    pub elevation: Arc<eo_data::ElevationGrid>,
    pub ground: Arc<dyn GroundProvider>,
    pub buildings: Arc<BuildingSet>,
    pub streets: bool,
}

impl GameWorld {
    /// `streets`: stand on the drawn street surface (eo-street), as `--play`
    /// does by default; else on the elevation grid.
    pub fn load(paths: &eo_data::DataPaths, streets: bool) -> Result<Self> {
        let city = Arc::new(eo_data::City::load(paths)?);
        let elevation = Arc::new(eo_data::ElevationGrid::load(paths)?);
        let lift = eo_assets::placement::PlacementConstants::load()?.construction_lift_m;
        let buildings = Arc::new(BuildingSet::from_city(&city, lift));
        let ground: Arc<dyn GroundProvider> = if streets { Arc::new(eo_player::street_ground::StreetGround::build(&city, elevation.clone())?) } else { Arc::new(ElevationGround(elevation.clone())) };
        Ok(Self { city, elevation, ground, buildings, streets })
    }

    /// From parts a host already has (eo-app's loaded world).
    pub fn from_parts(city: Arc<eo_data::City>, elevation: Arc<eo_data::ElevationGrid>, ground: Arc<dyn GroundProvider>, buildings: Arc<BuildingSet>, streets: bool) -> Self {
        Self { city, elevation, ground, buildings, streets }
    }

    /// A collision world with its own tile cookers (a client), streaming in
    /// lockstep (`CollisionWorld::lockstep`): its colliders at each step follow
    /// the body's path, not the cookers' speed.
    pub fn collision_world(&self, threads: usize) -> CollisionWorld {
        let mut cw = CollisionWorld::new(self.ground.clone(), self.buildings.clone(), threads);
        cw.lockstep = true;
        cw
    }
}

/// One authoritative fixed step: the tiles the step can reach made resident
/// (`CollisionWorld::ensure_step_tiles`, waiting for a late cook rather than
/// stepping over a missing tile), the input's camera yaw, then the step.
/// The server, a predicting client and a client replaying after a correction
/// all step through here, so the same state and inputs give the same body
/// whatever the machine's load (the defect `determinism.rs` caught: a replay
/// faster than real time outran the tile cookers and walked or drove over
/// ground that was not there).
#[inline]
pub fn step_player(player: &mut Player, cw: &mut CollisionWorld, input: &NetInput) {
    cw.ensure_step_tiles(player.position(cw));
    player.yaw = input.yaw();
    player.fixed_step(cw, &input.step_input());
}

/// The position a check compares: feet on foot, the car's datum driving.
pub fn check_of(seq: u32, player: &Player, cw: &CollisionWorld) -> Check {
    Check { seq, pos_mm: check_pos(player.position(cw)), driving: matches!(player.mode, Mode::Driving(_)) }
}

/// Where a car is put beside a newly spawned player (4 m to its left, as `--play`).
pub fn car_spawn(x: f64, z: f64, yaw: f64, driving: bool) -> [f64; 3] {
    if driving { [x, z, yaw] } else { [x - yaw.cos() * 4.0, z + yaw.sin() * 4.0, yaw] }
}

/// Spawn a player standing at (x, z) facing `yaw` with a car beside it, or
/// sitting in it (`driving`). Loads the collision tiles around it first.
pub fn spawn_player(cw: &mut CollisionWorld, x: f64, z: f64, yaw: f64, driving: bool) -> (Player, Vec<[f32; 3]>) {
    let mut feet = DVec3::new(x, cw.ground.ground_y(x, z), z);
    cw.load_blocking(feet, pworld::LOAD_RADIUS_M);
    feet.y = cw.surface_y(x, z);
    let mut player = Player::new(feet, yaw);
    let c = car_spawn(x, z, yaw, driving);
    player.spawn_vehicle(cw, c[0], c[1], c[2]);
    if driving {
        // Settle the car on its springs, then get in.
        for _ in 0..30 {
            player.fixed_step(cw, &eo_player::StepInput::default());
        }
        player.try_enter(cw);
    }
    (player, vec![[c[0] as f32, c[1] as f32, c[2] as f32]])
}

/// A client's copy of a player the server spawned: the same cars, then the server's state.
pub fn rebuild_player(cw: &mut CollisionWorld, state: &eo_player::PlayerState, cars: &[[f32; 3]]) -> Player {
    let p = state.character.position;
    let feet = DVec3::new(p[0], p[1] - eo_player::tuning::capsule::HALF_HEIGHT_M - eo_player::tuning::capsule::RADIUS_M, p[2]);
    cw.load_blocking(feet, pworld::LOAD_RADIUS_M);
    let mut player = Player::new(feet, state.yaw);
    for c in cars {
        player.spawn_vehicle(cw, c[0] as f64, c[1] as f64, c[2] as f64);
    }
    player.set_state(cw, state);
    player
}
