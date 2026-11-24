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

// ---------------------------------------------------------------- social groups (`--people-groups`)
// Without the switch a group's members are walkers of their own that set out
// together (0.38 m apart across the walk, 0.9 m along it, the JS spawn) and
// part at the first pause: each keeps its own schedule. With it a group keeps
// together as people out together do (Moussaid, Perozo, Garnier, Helbing and
// Theraulaz, "The walking behaviour of pedestrian social groups and its
// impact on crowd dynamics", PLoS ONE 5(4) e10047, 2010: a pair walks side by
// side; three walk in a V, the middle one a little behind, so each can see
// and hear the others): the first member keeps the schedule and the others
// walk, wait, cross and stop with it. Stopped to talk they stand on a circle
// about a shared middle, each facing it (Kendon's F-formation: Conducting
// Interaction, 1990), at a personal distance (Hall, The Hidden Dimension,
// 1966: 0.45-1.2 m); waiting at a kerb or a window they stand side by side
// along it. These are model choices within those observations, not measured
// constants of San Francisco's walkers.

/// Between neighbours' centres walking abreast (a body's breadth, about
/// 0.45 m, and a hand's room between shoulders), metres.
pub const GROUP_ABREAST_M: f64 = 0.75;
/// How far the middle one of three walks behind the others (the V), metres.
pub const GROUP_V_BACK_M: f64 = 0.35;
/// Between neighbours' centres on a talking group's circle, metres.
pub const GROUP_TALK_GAP_M: f64 = 1.0;
/// How fast a member steps to its new place when the group's formation
/// changes (stopping, setting off, turning to talk), m/s: a side step.
pub const GROUP_STEP_ASIDE_MPS: f64 = 0.6;
/// (`--people-stagger`) How far a walking group's line may turn from square
/// across the walk, either way, radians (each group its own keyed turn, held
/// while it walks): companions walk side by side but rarely exactly level,
/// one a shoulder ahead (Moussaid et al. 2010 give the abreast line as the
/// mean at low density, the members' places spread about it). At 25 degrees
/// a pair 0.75 m apart walks up to 0.32 m out of level, enough that two
/// companions seen from the side show two shoulder lines, not one. A model
/// choice within those observations, not a measured constant.
pub const GROUP_STAGGER_RAD: f64 = 25.0 * std::f64::consts::PI / 180.0;

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

// ---------------------------------------------------------------- crowd density (`--crowd`)
// The crowd by place and hour (eo-app `--crowd`; `RegionConfig::crowd`).
// Without it the resident disc above (the JS People setting: a fixed body
// count in 55 m, whatever the street and the hour) is used, unchanged.
//
// Each footway holds people at a linear density (people per metre of its
// walking line) from the steady-state relation density = flow / speed: a
// sidewalk passed by Q people an hour at the mode walking speed
// (`PED_CRUISE_MODE`) holds Q / (3600 x speed) people a metre. The flow is
// the sum of what generates walking along it:
//
//   street     the street's class (`sidewalk_class_weight`) x `CROWD_FLOW_CLASS_PPH`;
//   floors     the storeys of the buildings fronting it (the mean over its
//              length, 0 where nothing fronts it) x `CROWD_FLOW_STOREY_PPH` x
//              the street's class: the homes and jobs above the street, so a
//              block of towers fills its walks and a block of two-storey
//              houses does not, and a service lane behind the towers (where
//              their doors are not) little;
//   shops      its share of walk-in frontage (shops, cafes, services with a
//              street door) x `CROWD_FLOW_SHOPS_PPH`;
//   nightlife  its share of bar, pub, restaurant, club and theatre frontage x
//              `CROWD_FLOW_NIGHT_PPH` (twice the shops': at its peak a
//              nightlife front gathers people at its doors as well as passing);
//   place      a walk that is a destination itself (a plaza's or a
//              promenade's paths, a pedestrian street: `FootActivity::place`
//              1, a park's footpaths a third) x `CROWD_FLOW_PLACE_PPH`.
//
// The first three follow the day's footfall (`hour_footfall`), nightlife its
// own curve (`night_footfall`). These are crowd-model parameters, set so a
// downtown block of towers and shops at its peak holds 10-15 people every
// 100 m of each sidewalk (500-700 an hour), a neighbourhood commercial street
// with bars at 21 h about half that, a residential street at noon 1-3 and a
// service lane behind houses next to nothing: typical ranges for San
// Francisco's sidewalks, not counts at any one place. The fronts' uses come
// from the city file's building tags (OSM), which name only some of the
// shops and bars a street has, so a commercial street reads lower than it is.
pub const CROWD_FLOW_CLASS_PPH: f64 = 60.0;
pub const CROWD_FLOW_STOREY_PPH: f64 = 25.0;
pub const CROWD_FLOW_SHOPS_PPH: f64 = 450.0;
pub const CROWD_FLOW_NIGHT_PPH: f64 = 900.0;
pub const CROWD_FLOW_PLACE_PPH: f64 = 450.0;
/// `FootActivity::place` of a pedestrian street or plaza way (OSM
/// `highway=pedestrian`) and of any other footpath (`footway`, `path`).
pub const PLACE_PEDESTRIAN: f32 = 1.0;
pub const PLACE_FOOTPATH: f32 = 1.0 / 3.0;
/// The resident disc (`--crowd`): people live within this of the focus (the
/// renderer's instanced band reaches 145 m), are retired past it plus the
/// margin, never appear closer than the pop-in guard off shot or inside the
/// view cone closer than the in-shot guard (a refill pops in only where a
/// body is a few pixels tall), and never number more than the body budget
/// (the renderer's crowd tiers draw at most 448).
pub const CROWD_RADIUS_M: f64 = 150.0;
pub const CROWD_KEEP_MARGIN_M: f64 = 16.0;
pub const CROWD_POP_IN_GUARD_M: f64 = OFF_SHOT_REFILL_FLOOR_M;
pub const CROWD_IN_SHOT_GUARD_M: f64 = 90.0;
pub const CROWD_MAX_BODIES: usize = 400;

/// (`--crowd`) At a nightlife front, the share of walk legs (x its nightlife
/// share x `night_footfall`) that end standing at the doors, and how long
/// they stand (s): the crowd at a bar's door in the evening.
pub const DOOR_GATHER_SHARE: f64 = 0.6;
pub const DOOR_GATHER_SECONDS: [f64; 2] = [30.0, 150.0];

/// The evening economy's footfall (0..1; bars, restaurants, clubs, theatres):
/// a lunch peak, the dinner rise, the late evening at its height, falling
/// after midnight to the 02:00 last call (California Business and Professions
/// Code section 25631: no sale of alcohol between 2 a.m. and 6 a.m.).
pub fn night_footfall(hour: f64) -> f64 {
    const TABLE: [f64; 24] = [
        0.70, 0.50, 0.25, 0.08, 0.04, 0.03, 0.04, 0.10, 0.15, 0.18, 0.22, 0.40, 0.60, 0.55, 0.35, 0.30, 0.35, 0.50,
        0.75, 0.92, 1.00, 1.00, 0.95, 0.85,
    ];
    let h = if hour.is_finite() { ((hour % 24.0) + 24.0) % 24.0 } else { 12.0 };
    let i = h.floor() as usize;
    let f = h - i as f64;
    TABLE[i % 24] * (1.0 - f) + TABLE[(i + 1) % 24] * f
}

/// What fronts a footway (`--crowd`; `Footway::activity`): the shares of its
/// length fronted by walk-in shops and by the evening economy, the mean
/// storeys of whatever fronts it (0 where nothing does), and how much the walk
/// is a place of its own (a plaza's or a park's path, `PLACE_*`; 0 beside a
/// street). All zero without a frontage survey and paths
/// (`NativeSource::with_frontage`, `with_paths`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FootActivity {
    pub shops: f32,
    pub night: f32,
    pub storeys: f32,
    pub place: f32,
    /// Where along the walk a walk-in shop fronts it: bit i for the i-th of
    /// `SHOP_FRONT_SLOTS` equal stretches of its walking line, each the use
    /// of the front found at its middle (0 without a frontage survey). Read
    /// by `--people-windows` (`shop_front_at`).
    pub shop_fronts: u64,
}

/// The stretches `FootActivity::shop_fronts` divides a walking line into.
pub const SHOP_FRONT_SLOTS: usize = 64;

impl FootActivity {
    /// Whether a walk-in shop fronts the walk at arc `s` of its `length`.
    pub fn shop_front_at(&self, s: f64, length: f64) -> bool {
        if !(length > 0.0) || self.shop_fronts == 0 {
            return false;
        }
        let k = ((s / length).clamp(0.0, 1.0) * SHOP_FRONT_SLOTS as f64) as usize;
        self.shop_fronts >> k.min(SHOP_FRONT_SLOTS - 1) & 1 == 1
    }
}

/// People a metre of a footway's walking line holds at `hour` (see above).
pub fn footway_density(class_weight: f64, a: FootActivity, hour: f64) -> f64 {
    footway_density_with(class_weight, a, hour, false)
}

/// `footway_density`, with (`evening`, eo-app `--people-evening`) the
/// walk-in shops' share of the flow following a neighbourhood commercial
/// street's day (`shop_footfall`) instead of the offices' (`hour_footfall`).
pub fn footway_density_with(class_weight: f64, a: FootActivity, hour: f64, evening: bool) -> f64 {
    let day = hour_footfall(hour);
    let shops = if evening { shop_footfall(hour) } else { day };
    let flow = ((CROWD_FLOW_CLASS_PPH + CROWD_FLOW_STOREY_PPH * a.storeys as f64) * class_weight + CROWD_FLOW_PLACE_PPH * a.place as f64) * day
        + CROWD_FLOW_SHOPS_PPH * a.shops as f64 * shops
        + CROWD_FLOW_NIGHT_PPH * a.night as f64 * night_footfall(hour);
    flow / (3600.0 * PED_CRUISE_MODE)
}

/// (`--people-evening`) A neighbourhood commercial street's footfall (0..1)
/// through the day. San Francisco Planning's public life study of 9th Avenue
/// and Irving and of Upper Market and Castro (2008: ten-minute counts in each
/// hour, times six, weekdays 8:00-23:00 and Saturdays 10:00-23:00, at three
/// blocks of 9th and Irving and one each of Castro and Market Streets) found
/// the evening's counts at about the day's: at 9th and Irving 200-850 people
/// an hour a block, the three blocks' sum between 20:00 and 23:00 at 0.8-1.0
/// of its 10:00-17:00 mean (one low hour at 18:00); Market Street's weekday
/// peak was 20:00-21:00, and Castro Street's counts rose again between 21:00
/// and 23:00. A street of shops, cafes and restaurants holds its walkers into
/// the night as an office district (`hour_footfall`, 0.32 at 22:00) does not.
/// Here: 1 from 9:00 to 17:00, 0.85 at 18:00, 0.8 from 19:00 to 22:00 (the
/// counts, smoothed and taken at their low side); past 23:00, where the
/// counts end, falling to `hour_footfall`'s small hours (INFERRED), and the
/// morning before 8:00 as `hour_footfall`.
pub fn shop_footfall(hour: f64) -> f64 {
    const TABLE: [f64; 24] = [
        0.20, 0.07, 0.05, 0.04, 0.05, 0.10, 0.24, 0.52, 0.85, 1.00, 1.00, 1.00, 1.00, 1.00, 1.00, 1.00, 1.00, 1.00,
        0.85, 0.80, 0.80, 0.80, 0.80, 0.45,
    ];
    let h = if hour.is_finite() { ((hour % 24.0) + 24.0) % 24.0 } else { 12.0 };
    let i = h.floor() as usize;
    let f = h - i as f64;
    TABLE[i % 24] * (1.0 - f) + TABLE[(i + 1) % 24] * f
}

/// The resident disc of the crowd by place and hour (`RegionConfig::crowd`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrowdDensity {
    pub radius_m: f64,
    pub keep_radius_m: f64,
    pub pop_in_guard_m: f64,
    pub in_shot_guard_m: f64,
    pub max_bodies: usize,
    /// Multiplies every footway's density (eo-app `--agent-density`).
    pub scale: f64,
    /// (eo-app `--people-evening`; false as before) the walk-in shops' flow
    /// by a commercial street's day (`shop_footfall`).
    pub evening: bool,
}

impl CrowdDensity {
    pub fn standard(scale: f64) -> Self {
        CrowdDensity {
            radius_m: CROWD_RADIUS_M,
            keep_radius_m: CROWD_RADIUS_M + CROWD_KEEP_MARGIN_M,
            pop_in_guard_m: CROWD_POP_IN_GUARD_M,
            in_shot_guard_m: CROWD_IN_SHOT_GUARD_M,
            max_bodies: CROWD_MAX_BODIES,
            scale,
            evening: false,
        }
    }
}
