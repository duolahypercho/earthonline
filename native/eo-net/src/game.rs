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

/// The state a player has on arriving by fast travel at (x, z) facing `yaw`:
/// standing on the walkable surface there, still, on foot, its cars parked
/// beside it in a row (4 m to its left, then 6 m apart behind), wheels on the
/// ground. The tiles there are made resident first (the stream asked for
/// them when the trip was accepted), and the broad phase is brought up to
/// date so the surface below can be found (no dynamics step is taken).
pub fn arrival_state(player: &Player, cw: &mut CollisionWorld, x: f64, z: f64, yaw: f64) -> eo_player::PlayerState {
    let here = DVec3::new(x, cw.ground.ground_y(x, z), z);
    cw.ensure_step_tiles(here);
    cw.physics.detect_collisions(&(), &());
    let feet = DVec3::new(x, cw.surface_y(x, z), z);
    let mut s = player.state(cw);
    let fresh = eo_player::character::Character::new(feet);
    s.character = eo_player::character::CharacterState { position: fresh.position.to_array(), grounded: true, crouched: false, ..fresh.state() };
    s.momentum = [0.0; 4];
    s.momentum_last = None;
    s.plant_yaw = None;
    s.yaw = yaw;
    s.facing_yaw = yaw;
    s.sprint_held_s = 0.0;
    s.sprint = 0.0;
    s.ground_speed = 0.0;
    s.driving = None;
    let spec = eo_player::vehicle::VehicleSpec::sedan();
    let rot = glam::DQuat::from_rotation_y(yaw);
    for (k, v) in s.vehicles.iter_mut().enumerate() {
        let c = car_spawn(x, z, yaw, false);
        let back = 6.0 * k as f64;
        let (cx, cz) = (c[0] - yaw.sin() * back, c[1] - yaw.cos() * back);
        let datum = DVec3::new(cx, cw.surface_y(cx, cz), cz);
        let centre = datum + rot * DVec3::new(0.0, spec.height / 2.0, 0.0);
        v.translation = [centre.x as f32, centre.y as f32, centre.z as f32];
        let q = glam::Quat::from_rotation_y(yaw as f32);
        v.rotation = [q.x, q.y, q.z, q.w];
        v.linvel = [0.0; 3];
        v.angvel = [0.0; 3];
        v.sleeping = false;
        v.time_since_can_sleep = 0.0;
        v.steer_normalized = 0.0;
        v.controls = [0.0; 3];
        v.handbrake = true;
    }
    s
}
