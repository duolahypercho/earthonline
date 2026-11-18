//! Tuning values ported from the JS game. Every number here is read from the
//! JS source named beside it; `tests/js_oracle.rs` compares each one with the
//! value `tools/export-player-oracle.mjs` extracts from that source, so a
//! change on either side fails the test instead of drifting.

/// `PHYSICS_FIXED_STEP_SECONDS` (src/simulation/physics/core/constants.js).
pub const FIXED_STEP_S: f64 = 1.0 / 60.0;
/// `STANDARD_GRAVITY_MPS2` (constants.js).
pub const GRAVITY_MPS2: f64 = 9.80665;
/// `PHYSICS_SOLVER_ITERATIONS` (constants.js).
pub const SOLVER_ITERATIONS: usize = 4;

/// The player's capsule and character-controller configuration:
/// `createPhysicsPlayerController` defaults (src/player/physics-player-controller.js)
/// and `createCharacterController` (src/simulation/physics/core/physics-runtime.js).
pub mod capsule {
    pub const RADIUS_M: f64 = 0.35;
    pub const HALF_HEIGHT_M: f64 = 0.55;
    pub const CONTROLLER_OFFSET_M: f64 = 0.01;
    pub const MAX_STEP_HEIGHT_M: f64 = 0.3;
    pub const MIN_STEP_WIDTH_M: f64 = 0.2;
    pub const MAX_SLOPE_DEGREES: f64 = 45.0;
    pub const MIN_SLIDE_DEGREES: f64 = 50.0;
    pub const SNAP_TO_GROUND_M: f64 = 0.15;
    pub const NORMAL_NUDGE_M: f64 = 0.001;
    /// Capsule centre above the feet (`PLAYER_CAPSULE_CENTER_FROM_FEET_M` in main.js).
    pub const CENTER_FROM_FEET_M: f64 = RADIUS_M + HALF_HEIGHT_M;
}

/// Kerb step-up, perch and ground-contact constants (physics-player-controller.js).
pub mod kerb {
    pub const GROUND_CONTACT_TOLERANCE_M: f64 = 0.02;
    pub const GROUND_CONTACT_PROBE_REACH_M: f64 = 1.0;
    pub const PERCHED_FALL_PRESS_MPS: f64 = 1.5;
    pub const PERCH_STEP_LIFT_M: f64 = 0.17;
    pub const PERCH_LIFT_AFTER_STEPS: u32 = 3;
    pub const PERCH_LIFT_COOLDOWN_STEPS: u32 = 60;
    pub const STEP_MAX_RISE_M: f64 = 0.24;
    pub const STEP_MIN_RISE_M: f64 = 0.03;
    pub const STEP_CLEARANCE_M: f64 = 0.03;
    pub const STEP_AFTER_BLOCKED_STEPS: u32 = 1;
    pub const STEP_REFUSED_SHARE: f64 = 0.5;
    pub const STEP_HOLD_STEPS: u32 = 18;
}

/// Locomotion speeds (src/citygen/main.js).
pub mod speed {
    pub const WALK_MPS: f64 = 1.6;
    pub const RUN_MPS: f64 = 5.6;
    pub const SPRINT_MPS: f64 = 7.4;
    pub const SPRINT_BUILD_S: f64 = 0.9;
    pub const SPRINT_RAMP_S: f64 = 0.6;
    pub const SPRINT_MIN_FORWARD: f64 = 0.7;
    pub const STANDING_TURN_RATE_RAD_PER_S: f64 = 7.5;
    /// `state.playerPhysics?.jump(2.7)`: "a roughly 0.37 m standing jump".
    pub const JUMP_MPS: f64 = 2.7;
    /// Crouched or aiming walk speed (`stance.crouched || aiming ? 1.25`).
    pub const CROUCH_AIM_MPS: f64 = 1.25;
}

/// World collider construction (src/simulation/physics/world/world-physics.js).
pub mod world {
    /// `buildingFootprintPrismMesh` `verticalSkirtM`: the prism reaches this far
    /// under the lowest ground sample of its footprint.
    pub const BUILDING_SKIRT_M: f64 = 0.25;
    /// `terrainHeightfieldPhysicsDescriptor` `sampleSpacingM`.
    pub const HEIGHTFIELD_SPACING_M: f64 = 4.0;
    /// Terrain material friction (`source-sampled-heightfield`).
    pub const TERRAIN_FRICTION: f32 = 0.85;
    /// Building prism material friction (`source-footprint-prism`).
    pub const BUILDING_FRICTION: f32 = 0.7;
}

/// `DEFAULT_INTERACT_RANGES.vehicle` (src/player/interaction-resolver.js).
pub const VEHICLE_INTERACT_RANGE_M: f64 = 2.5;
/// The door station stands this far outside the body's flank
/// (`vehicleEntryLayout`: `side * (width / 2 + 0.55)`, src/player/vehicle-entry-layout.js).
pub const DOOR_STATION_OUTSET_M: f64 = 0.55;
