//! The game rules both ends share: the collision world over the city, how a
//! player is spawned, and the one authoritative step of a player's
//! controller. Server and client run exactly this code, so the same inputs
//! give the same body on both (tested in `tests/determinism.rs`).

use crate::wire::{Check, NetInput, check_pos};
use anyhow::Result;
use eo_player::world::{self as pworld, BuildingSet, CollisionWorld, ElevationGround, GroundProvider};
use eo_player::{Mode, Player};
use eo_structures::{StructureOptions, StructureWorld};
use glam::DVec3;
use std::sync::Arc;

/// The city as the player's physics sees it (built as eo-app's `--play` builds it).
pub struct GameWorld {
    pub city: Arc<eo_data::City>,
    pub elevation: Arc<eo_data::ElevationGrid>,
    pub ground: Arc<dyn GroundProvider>,
    pub buildings: Arc<BuildingSet>,
    pub streets: bool,
    /// The grade-separated roads the ground was built with (`--structures`,
    /// `--tunnels`; only on the street surface): the handshake refuses a
    /// client whose own differ (`proto::structures_code`).
    pub structures: Option<StructureOptions>,
    /// The grade-separated roads themselves, as built for the ground (with
    /// `structures`; None without them, and from `from_parts` unless a host
    /// hands them over with `with_decks`): the server's simulation takes
    /// its lanes' levels from them (`StructureWorld::way_levels`).
    pub decks: Option<Arc<StructureWorld>>,
}

impl GameWorld {
    /// `streets`: stand on the drawn street surface (eo-street), as `--play`
    /// does by default; else on the elevation grid. The landmarks' decks and
    /// lifts are in the collision world (`load_access` to leave them out).
    pub fn load(paths: &eo_data::DataPaths, streets: bool) -> Result<Self> {
        Self::load_with(paths, streets, None)
    }

    /// As `load`, and with `structures` (on the street surface only) the
    /// grade-separated roads built over it, as eo-app's `--structures` builds
    /// them: the street built again with the structures' joins (stage A6,
    /// `StreetGround::build_joined`), so a dedicated server and the app stand
    /// on the same street and collide alike. Without `structures` the street
    /// is the one `load` builds. The landmarks' decks and lifts are in.
    pub fn load_with(paths: &eo_data::DataPaths, streets: bool, structures: Option<StructureOptions>) -> Result<Self> {
        Self::load_access(paths, streets, structures, true)
    }

    /// `load_with`; `access`: the landmark decks and lifts (`--no-landmark-access` leaves them out).
    pub fn load_access(paths: &eo_data::DataPaths, streets: bool, structures: Option<StructureOptions>, access: bool) -> Result<Self> {
        let city = Arc::new(eo_data::City::load(paths)?);
        let elevation = Arc::new(eo_data::ElevationGrid::load(paths)?);
        let lift = eo_assets::placement::PlacementConstants::load()?.construction_lift_m;
        let structures = structures.filter(|_| streets);
        let (ground, decks): (Arc<dyn GroundProvider>, Option<Arc<StructureWorld>>) = if streets {
            let sg = eo_player::street_ground::StreetGround::build_joined(&city, elevation.clone(), structures)?;
            let decks = sg.structures.clone();
            (Arc::new(sg), decks)
        } else {
            (Arc::new(ElevationGround(elevation.clone())), None)
        };
        let floors = city_decks(paths, &elevation, ground.as_ref(), access)?;
        let mut buildings = BuildingSet::from_city(&city, lift);
        buildings.open_tunnel_approaches(decks.as_deref());
        let buildings = Arc::new(buildings.with_decks(floors));
        Ok(Self { city, elevation, ground, buildings, streets, structures, decks })
    }

    /// From parts a host already has (eo-app's loaded world); `structures`
    /// as the ground was built (None without grade-separated roads). The
    /// built roads are not among the parts: `with_decks` adds them.
    pub fn from_parts(city: Arc<eo_data::City>, elevation: Arc<eo_data::ElevationGrid>, ground: Arc<dyn GroundProvider>, buildings: Arc<BuildingSet>, streets: bool, structures: Option<StructureOptions>) -> Self {
        Self { city, elevation, ground, buildings, streets, structures, decks: None }
    }

    /// The same world holding the grade-separated roads its ground was built
    /// with (a host that built them itself, as eo-app's `--structures` does).
    pub fn with_decks(mut self, decks: Option<Arc<StructureWorld>>) -> Self {
        self.decks = decks;
        self
    }

    /// The level table of the simulation's lanes (`StructureWorld::way_levels`;
    /// empty without grade-separated roads, which leaves every lane at grade).
    pub fn way_levels(&self) -> std::collections::HashMap<String, i8> {
        self.decks.as_ref().map(|d| d.way_levels().clone()).unwrap_or_default()
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

/// The landmarks' walkable decks as the collision world takes them
/// (eo-landmarks access.rs on the drawn ground, the grid's `sample`, as the
/// renderer builds the landmarks): the same on a server and every client.
pub fn landmark_decks(elevation: &eo_data::ElevationGrid) -> eo_player::Decks {
    let set = eo_landmarks::access::AccessSet::build(&|x: f64, z: f64| elevation.sample(x, z));
    decks_of(&set)
}

/// The base city harbour surfaces participate on both network peers.
pub fn network_decks(paths: &eo_data::DataPaths, elevation: &eo_data::ElevationGrid) -> Result<eo_player::Decks> {
    let mut decks = landmark_decks(elevation);
    decks.parts.push(harbour_floor(paths, elevation)?);
    Ok(decks)
}

fn harbour_floor(paths: &eo_data::DataPaths, elevation: &eo_data::ElevationGrid) -> Result<eo_player::DeckPart> {
    let surfaces = eo_landmarks::waterfront::harbour_floors(&paths.repo_root, elevation)?;
    Ok(eo_player::DeckPart {
        name: "harbour piers and docks".into(),
        walk: surfaces.into_iter().map(|tri| tri.map(|p| p.map(f64::from))).collect(),
        ..Default::default()
    })
}

/// The base bridge draws the same sidewalk plates with its original railing.
/// Only the replacement rail, lifts and other access decks follow the switch.
pub fn base_bridge_walk(elevation: &eo_data::ElevationGrid) -> eo_player::DeckPart {
    let deck = eo_landmarks::access::golden_gate_sidewalk(eo_landmarks::LandmarksJs::get(),
        &|x, z| elevation.sample(x, z));
    eo_player::DeckPart { name: deck.id, walk: deck.walk, ..Default::default() }
}

/// Without the optional streamed structures, these drawn bridge roads belong
/// to the permanent collision world. A structured city installs the same faces per tile.
pub fn bridge_roads(paths: &eo_data::DataPaths, elevation: &eo_data::ElevationGrid) -> Result<Vec<eo_player::DeckPart>> {
    let ca = eo_landmarks::suspension::DeckStandards::load(&eo_data::CaStandards::load(paths)?)?;
    let roads = eo_landmarks::bridge_roads::surfaces(eo_landmarks::LandmarksJs::get(), &|x,z| elevation.sample(x,z), &ca);
    Ok(roads.into_iter().map(|r| eo_player::DeckPart { name: r.deck.name().into(), walk: r.road, barrier: r.barrier, ..Default::default() }).collect())
}

/// Shore walks are part of the base city. Tower lifts follow the access switch.
pub fn city_decks(paths: &eo_data::DataPaths, elevation: &eo_data::ElevationGrid, ground: &dyn GroundProvider, access: bool) -> Result<eo_player::Decks> {
    let mut decks = if access { network_decks(paths, elevation)? } else {
        eo_player::Decks { parts: vec![base_bridge_walk(elevation), harbour_floor(paths, elevation)?], ..Default::default() }
    };
    if !ground.has_landmark_roads() { decks.parts.extend(bridge_roads(paths, elevation)?); }
    let coast = eo_landmarks::promenade::collision(&|x,z| ground.ground_y(x,z));
    decks.parts.extend(coast.iter().map(eo_player::DeckPart::from));
    Ok(decks)
}

/// An access set's collision, prism cuts and lifts.
pub fn decks_of(set: &eo_landmarks::access::AccessSet) -> eo_player::Decks {
    // compass heading to the player's yaw (forward = (sin yaw, 0, cos yaw)).
    let yaw = |h: f64| {
        let d = eo_landmarks::access::dir_of(h);
        d[0].atan2(d[1])
    };
    eo_player::Decks {
        parts: set.decks.iter().map(|d| eo_player::DeckPart { name: d.id.clone(), walk: d.walk.clone(), barrier: d.barrier.clone(), ..Default::default() }).collect(),
        cuts: set.decks.iter().flat_map(|d| d.cuts.iter()).map(|c| eo_player::PrismCut { centre: c.centre, radius_m: c.radius_m, y0: c.y0, y1: c.y1 }).collect(),
        lifts: set
            .lifts()
            .map(|l| eo_player::LiftDef { name: l.name.clone(), bottom: DVec3::from_array(l.bottom.feet), bottom_yaw: yaw(l.bottom.heading_deg), top: DVec3::from_array(l.top.feet), top_yaw: yaw(l.top.heading_deg) })
            .collect(),
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

/// Step the controller with the road and air at this input's simulation time.
pub fn step_player_in_weather(player: &mut Player, cw: &mut CollisionWorld, input: &NetInput, seq: u32, weather: &mut crate::weather::RoadWeather) {
    player.set_weather(weather.at(seq));
    step_player(player, cw, input);
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
    spawn_configured_player(cw, x, z, yaw, driving, Default::default())
}

pub fn spawn_configured_player(cw: &mut CollisionWorld, x: f64, z: f64, yaw: f64, driving: bool, config: eo_player::vehicle::VehicleConfig) -> (Player, Vec<[f32; 3]>) {
    let mut feet = DVec3::new(x, cw.ground.ground_y(x, z), z);
    cw.load_blocking(feet, pworld::LOAD_RADIUS_M);
    feet.y = cw.arrival_y(x, z);
    let mut player = Player::new(feet, yaw);
    player.spawn_config = config;
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
    for (i, c) in cars.iter().enumerate() {
        player.spawn_config = state.vehicles.get(i).map(|v| v.config).unwrap_or_default();
        player.spawn_vehicle(cw, c[0] as f64, c[1] as f64, c[2] as f64);
    }
    player.set_state(cw, state);
    player
}

/// The state a player has on arriving by fast travel at (x, z) facing `yaw`:
/// standing on the walkable surface there, still, on foot and off any lift, its cars parked
/// beside it in a row (4 m to its left, then 6 m apart behind), wheels on the
/// ground. The tiles there are made resident first (the stream asked for
/// them when the trip was accepted), and the broad phase is brought up to
/// date so the surface below can be found (no dynamics step is taken).
pub fn arrival_state(player: &Player, cw: &mut CollisionWorld, x: f64, z: f64, yaw: f64) -> eo_player::PlayerState {
    let here = DVec3::new(x, cw.ground.ground_y(x, z), z);
    cw.ensure_step_tiles(here);
    cw.physics.detect_collisions(&(), &());
    let feet = DVec3::new(x, cw.arrival_y(x, z), z);
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
    // an arrival ends a landmark lift ride (its move would take the body back).
    s.lift = None;
    for (k, v) in s.vehicles.iter_mut().enumerate() {
        let spec = v.config.spec();
        let c = car_spawn(x, z, yaw, false);
        let back = 6.0 * k as f64;
        let (cx, cz) = (c[0] - yaw.sin() * back, c[1] - yaw.cos() * back);
        let datum = DVec3::new(cx, cw.surface_beside(feet, cx, cz), cz);
        let (centre, rot) = spec.ground_pose(cw, datum, yaw);
        v.translation = centre.as_vec3().to_array();
        v.rotation = rot.as_quat().to_array();
        v.linvel = [0.0; 3];
        v.angvel = [0.0; 3];
        v.sleeping = false;
        v.time_since_can_sleep = 0.0;
        v.transmission = Default::default();
        v.steer_normalized = 0.0;
        v.controls = [0.0; 3];
        v.handbrake = true;
    }
    s
}
