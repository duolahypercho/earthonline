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
        let mut set = BuildingSet::from_city(&city, lift);
        if access {
            set = set.with_decks(landmark_decks(&elevation));
        }
        let buildings = Arc::new(set);
        let structures = structures.filter(|_| streets);
        let (ground, decks): (Arc<dyn GroundProvider>, Option<Arc<StructureWorld>>) = if streets {
            let sg = eo_player::street_ground::StreetGround::build_joined(&city, elevation.clone(), structures)?;
            let decks = sg.structures.clone();
            (Arc::new(sg), decks)
        } else {
            (Arc::new(ElevationGround(elevation.clone())), None)
        };
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

    /// A collision world with its own tile cookers (a client).
    pub fn collision_world(&self, threads: usize) -> CollisionWorld {
        CollisionWorld::new(self.ground.clone(), self.buildings.clone(), threads)
    }
}

/// The landmarks' walkable decks as the collision world takes them
/// (eo-landmarks access.rs on the drawn ground, the grid's `sample`, as the
/// renderer builds the landmarks): the same on a server and every client.
pub fn landmark_decks(elevation: &eo_data::ElevationGrid) -> eo_player::Decks {
    let set = eo_landmarks::access::AccessSet::build(&|x: f64, z: f64| elevation.sample(x, z));
    decks_of(&set)
}

/// An access set's collision, prism cuts and lifts.
pub fn decks_of(set: &eo_landmarks::access::AccessSet) -> eo_player::Decks {
    // compass heading to the player's yaw (forward = (sin yaw, 0, cos yaw)).
    let yaw = |h: f64| {
        let d = eo_landmarks::access::dir_of(h);
        d[0].atan2(d[1])
    };
    eo_player::Decks {
        parts: set.decks.iter().map(|d| eo_player::DeckPart { name: d.id.clone(), walk: d.walk.clone(), barrier: d.barrier.clone() }).collect(),
        cuts: set.decks.iter().flat_map(|d| d.cuts.iter()).map(|c| eo_player::PrismCut { centre: c.centre, radius_m: c.radius_m, y0: c.y0, y1: c.y1 }).collect(),
        lifts: set
            .lifts()
            .map(|l| eo_player::LiftDef { name: l.name.clone(), bottom: DVec3::from_array(l.bottom.feet), bottom_yaw: yaw(l.bottom.heading_deg), top: DVec3::from_array(l.top.feet), top_yaw: yaw(l.top.heading_deg) })
            .collect(),
    }
}

/// One authoritative fixed step: the input's camera yaw, then the step.
#[inline]
pub fn step_player(player: &mut Player, cw: &mut CollisionWorld, input: &NetInput) {
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
