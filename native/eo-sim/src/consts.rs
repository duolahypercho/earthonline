//! The simulation's model constants, each one the JS game's own number, cited
//! by file. These are DRIVING-MODEL and CROWD-MODEL parameters (accelerations,
//! look-aheads, schedule tables), not street standards: every street rule
//! (signal intervals, crossing walking speeds, lane widths, the prima facie
//! limit) is read from the California standards table in `standards.rs`.
//! `tests/js_exact.rs` holds every number here that the oracle exports to its
//! JS value.

// ---------------------------------------------------------------- driving
// src/citygen/traffic.js, "Driving model constants".
pub const ACCEL: f64 = 2.6;
pub const DECEL: f64 = 5.2;
pub const FOLLOW_DECEL: f64 = 4.2;
pub const MIN_BUMPER_GAP: f64 = 1.8;
pub const FOLLOW_SETTLE_MARGIN: f64 = 0.6;
pub const FOLLOW_SETTLE_GAP: f64 = MIN_BUMPER_GAP + FOLLOW_SETTLE_MARGIN;
pub const FOLLOW_CREEP_MPS: f64 = 0.4;
/// The hold a car keeps short of a lane with no control record (`LEGACY_STOP_LINE`).
pub const LEGACY_STOP_LINE: f64 = 4.6;
pub const SIGNAL_LOOKAHEAD: f64 = 42.0;
pub const BOX_CLEAR_LOOKAHEAD_M: f64 = 16.0;
pub const TURN_SIGNAL_DIST: f64 = 16.0;
pub const TURN_APPROACH_M: f64 = 14.0;
pub const APPROACH_COMFORT_DECEL: f64 = 0.8 * DECEL;
pub const STALL_SPEED_MS: f64 = 0.05;
pub const CLAIM_STALL_S: f64 = 4.0;
pub const CLAIM_YIELD_S: f64 = 2.0;
pub const JUNCTION_STALL_S: f64 = 4.0;
pub const JUNCTION_CREEP_MS: f64 = 0.9;
pub const GRIDLOCK_RELIEF_S: f64 = 75.0;
pub const GRIDLOCK_RELIEF_GAP_S: f64 = 8.0;
pub const TERMINUS_RESPAWN_S: f64 = 4.0;

// Junction fillets (`buildCornerFillet`).
pub const TURN_LATERAL_ACCEL: f64 = 2.8;
pub const TURN_RADIUS_WANT: f64 = 6.0;
pub const TURN_RADIUS_MIN: f64 = 4.2;
pub const TURN_RADIUS_FLOOR: f64 = 0.75;
pub const TURN_RADIUS_MAX: f64 = 14.0;
pub const TURN_MIN_ANGLE: f64 = 0.10;
pub const TURN_MAX_ANGLE: f64 = 2.80;
pub const TURN_TANGENT_FRACTION: f64 = 0.9;
pub const TURN_TANGENT_EPS: f64 = 0.30;
pub const TURN_SETBACK_MAX: f64 = 15.0;
pub const THROUGH_MOVE_MAX_LATERAL_M: f64 = 0.25;
pub const MERGE_SAMPLES: usize = 64;

// Cross-edge corridor (`CROSS_*`).
pub const CROSS_CELL_M: f64 = 12.0;
pub const CROSS_LOOKAHEAD_S: f64 = 1.6;
pub const CROSS_LOOK_MIN: f64 = 7.0;
pub const CROSS_LOOK_MAX: f64 = 26.0;
pub const CROSS_LATERAL_MARGIN: f64 = 0.35;
pub const CROSS_STALL_S: f64 = 4.0;
pub const CROSS_CREEP_GAP: f64 = 0.35;
pub const CROSS_CREEP_SPEED: f64 = 1.2;

// Yielding to people (`PERSON_*`).
pub const PERSON_DECEL: f64 = 3.0;
pub const PERSON_STOP_MARGIN_M: f64 = 1.6;
pub const PERSON_SIDE_MARGIN_M: f64 = 0.45;
pub const PERSON_CAPSULE_R_M: f64 = 0.30;
pub const PERSON_SCAN_MIN_M: f64 = 6.0;
pub const PERSON_SCAN_MAX_M: f64 = 34.0;
pub const PERSON_CELL_M: f64 = 16.0;
pub const PERSON_HOLD_MAX_S: f64 = 8.0;
pub const PERSON_CREEP_MS: f64 = 0.9;
pub const PERSON_EMERGENCY_M: f64 = 0.55;

// Street-class limits (`CLASS_SPEED_MS` / `designSpeed` in signal-control-plan.js).
pub const DRIVER_PACE_MIN: f64 = 0.85;
pub const DRIVER_PACE_SPAN: f64 = 0.30;

/// `QUEUE_SEPARATION.presented` in traffic.js: longest/widest drawn body per kind.
pub const PRESENTED_LENGTH: [f32; 4] = [5.89, 4.92, 7.62, 12.50];
pub const PRESENTED_WIDTH: [f32; 4] = [2.03, 1.86, 2.44, 2.59];
/// `TRAFFIC_KIND_MIX`: sedan, taxi, truck, bus shares.
pub const TRAFFIC_KIND_MIX: [f64; 4] = [0.9, 0.02, 0.06, 0.02];

// ---------------------------------------------------------------- walking
// src/simulation/traffic/realistic-movement-speeds.js
pub const PED_CRUISE_MIN: f64 = 0.95;
pub const PED_CRUISE_MODE: f64 = 1.31;
pub const PED_CRUISE_MAX: f64 = 1.65;
pub const AUTONOMOUS_TRAFFIC_CEILING_MPS: f64 = 12.0;
pub const METRES_PER_SECOND_PER_MPH: f64 = 0.44704;

// src/citygen/traffic.js, ambient routing.
pub const PEDESTRIAN_TURN_RATE: f64 = 2.6;
pub const SIGNAL_NODE_RADIUS_M: f64 = 20.0;
pub const CROSSING_SETBACK_M: f64 = 4.0;
pub const CROSSING_SHARE: f64 = 0.6;
pub const UNSIGNALLED_CROSSING_SHARE: f64 = 0.3;
pub const CROSSING_GAP_M: f64 = 22.0;
pub const CROSSING_MISS_MARGIN_M: f64 = 2.0;
pub const KERB_WAIT_MAX_S: f64 = 40.0;
/// src/simulation/pedestrians/crosswalk-walking.js
pub const CROSSING_BAND_SNAP_M: f64 = 1.2;
pub const CROSSING_BAND_REACH_M: f64 = 4.0;
pub const CROSS_KEEP_SIDE_M: f64 = 0.45;
pub const CROSS_KEEP_RAMP_M: f64 = 1.5;
/// `WALK_LEG_SECONDS`.
pub const WALK_LEG_SECONDS: [f64; 2] = [14.0, 52.0];
/// `STREET_POPULATION.groupShare` / `groupSize`.
pub const GROUP_SHARE: f64 = 0.34;
pub const GROUP_SIZE: [u32; 2] = [2, 3];

/// What a walker is doing (`PEDESTRIAN_ACTIVITIES` + `AMBULATORY_ACTIVITIES`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Activity {
    Walk = 0,
    Wait = 1,
    Talk = 2,
    Phone = 3,
    Browse = 4,
    Stand = 5,
    Carry = 6,
}

/// `PEDESTRIAN_ACTIVITIES`: (activity, weight, seconds lo, seconds hi, at corner only).
pub const PAUSE_TABLE: [(Activity, f64, f64, f64, bool); 6] = [
    (Activity::Wait, 0.34, 6.0, 18.0, true),
    (Activity::Talk, 0.16, 8.0, 26.0, false),
    (Activity::Phone, 0.16, 5.0, 16.0, false),
    (Activity::Browse, 0.13, 4.0, 13.0, false),
    (Activity::Stand, 0.11, 3.0, 9.0, false),
    (Activity::Carry, 0.10, 4.0, 10.0, false),
];
/// `AMBULATORY_ACTIVITIES`: (activity, weight, needs company).
pub const AMBULATORY_TABLE: [(Activity, f64, bool); 3] =
    [(Activity::Phone, 0.17, false), (Activity::Carry, 0.13, false), (Activity::Talk, 0.20, true)];

/// `SIDEWALK_CLASS_WEIGHT` in traffic.js.
pub fn sidewalk_class_weight(highway: &str) -> f64 {
    match highway {
        "primary" => 1.0,
        "secondary" => 0.95,
        "tertiary" => 0.8,
        "residential" => 0.55,
        "unclassified" => 0.5,
        "living_street" => 0.55,
        "pedestrian" => 1.15,
        "service" => 0.2,
        _ => 0.5,
    }
}

/// `hourFootfall(hour)` in traffic.js.
pub fn hour_footfall(hour: f64) -> f64 {
    const TABLE: [f64; 24] = [
        0.10, 0.07, 0.05, 0.04, 0.05, 0.10, 0.24, 0.52, 0.82, 0.88, 0.92, 0.97, 1.00, 0.98, 0.93, 0.92, 0.95, 1.00,
        0.94, 0.78, 0.60, 0.46, 0.32, 0.18,
    ];
    let h = if hour.is_finite() { ((hour % 24.0) + 24.0) % 24.0 } else { 12.0 };
    let i = h.floor() as usize;
    let f = h - i as f64;
    TABLE[i % 24] * (1.0 - f) + TABLE[(i + 1) % 24] * f
}

/// `pedestrianCruiseSpeedMps(sample)`: the triangular walking-speed inverse CDF.
pub fn pedestrian_cruise_speed(sample: f64) -> f64 {
    let u = sample.clamp(0.0, 1.0 - f64::EPSILON);
    let width = PED_CRUISE_MAX - PED_CRUISE_MIN;
    let split = (PED_CRUISE_MODE - PED_CRUISE_MIN) / width;
    if u < split {
        PED_CRUISE_MIN + (u * width * (PED_CRUISE_MODE - PED_CRUISE_MIN)).sqrt()
    } else {
        PED_CRUISE_MAX - ((1.0 - u) * width * (PED_CRUISE_MAX - PED_CRUISE_MODE)).sqrt()
    }
}

/// `soloPedestrianCruiseSpeedMps(base, sample)`.
pub fn solo_cruise_speed(base: f64, sample: f64) -> f64 {
    let scale = 0.94 + sample.clamp(0.0, 1.0 - f64::EPSILON) * 0.12;
    (base * scale).clamp(PED_CRUISE_MIN, PED_CRUISE_MAX)
}

// ---------------------------------------------------------------- population
// src/citygen/traffic.js local life.
pub const LOCAL_LIFE_RADIUS: f64 = 120.0;
pub const LOCAL_RECYCLE_RADIUS: f64 = 240.0;
pub const LOCAL_FOCUS_SHIFT: f64 = 70.0;
pub const LOCAL_CAR_TARGET: f64 = 32.0;
pub const LOCAL_ONSCREEN_CAR_TARGET: usize = 6;
pub const LOCAL_STARVATION_RATIO: f64 = 0.6;
pub const POP_IN_GUARD_METRES: f64 = 34.0;
pub const HOMING_MAX_GAIN: f64 = 3.0;
/// `STREET_POPULATION.vehiclePool`: the moving fleet on a real-map city.
pub const VEHICLE_POOL: usize = 42;

/// A People setting's live resident disc: `crowdPopulationBubble(level)` in
/// src/render/performance-settings.js over its `CROWD_QUALITY` row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrowdBubble {
    pub max_bodies: usize,
    pub radius_m: f64,
    pub keep_radius_m: f64,
    pub pop_in_guard_m: f64,
    pub in_shot_guard_m: f64,
}

/// The off-shot refill floor (`OFF_SHOT_REFILL_FLOOR_M`) and the materializer's own floor.
const OFF_SHOT_REFILL_FLOOR_M: f64 = 18.0;
const MATERIALIZER_POP_IN_FLOOR_M: f64 = 30.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    High,
    Medium,
    Low,
}

impl CrowdBubble {
    pub fn for_level(level: Level) -> Self {
        // CROWD_QUALITY[level]: (maxBodies, instanced distance, far distance, refill floor)
        let (max_bodies, instanced, far, refill) = match level {
            Level::High => (116, 52.0, 88.0, Some(OFF_SHOT_REFILL_FLOOR_M)),
            Level::Medium => (40, 38.0, 55.0, Some(OFF_SHOT_REFILL_FLOOR_M)),
            Level::Low => (14, 30.0, 42.0, None),
        };
        CrowdBubble {
            max_bodies,
            radius_m: far,
            keep_radius_m: far + 16.0,
            pop_in_guard_m: f64::min(instanced, refill.unwrap_or(MATERIALIZER_POP_IN_FLOOR_M)),
            in_shot_guard_m: instanced,
        }
    }
}

/// `DEFAULT_PEOPLE_LEVEL` in performance-settings.js.
pub const DEFAULT_PEOPLE_LEVEL: Level = Level::Medium;
/// `LIFE_MATERIALIZER.inShotHalfAngleRad` / `scanIntervalS`.
pub const IN_SHOT_HALF_ANGLE_RAD: f64 = std::f64::consts::PI / 3.0;
pub const SCAN_INTERVAL_S: f64 = 0.5;

/// `VEHICLE_QUALITY[level].trafficMax / trafficDistance`: the drawn moving window.
pub fn traffic_window(level: Level) -> (usize, f64) {
    match level {
        Level::High => (42, 240.0),
        Level::Medium => (32, 160.0),
        Level::Low => (20, 110.0),
    }
}

/// `SIM_TIER_CONTRACT` in src/simulation/sim-tiers.js.
pub const SIM_TIER_NEAR_RADIUS_M: f64 = 45.0;
pub const SIM_TIER_FAR_RADIUS_M: f64 = 60.0;
pub const SIM_TIER_FAR_REENTER_M: f64 = 55.0;
pub const SIM_TIER_FAR_EVERY: u32 = 2;

/// `PHYSICS_FIXED_STEP_SECONDS` in src/simulation/physics/core/constants.js.
pub const FIXED_STEP_S: f64 = 1.0 / 60.0;
