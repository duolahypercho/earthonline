//! California street standards, read from the same cited source the browser
//! game's table is generated from: `scripts/world-data/sources/california-street-standards.json`.
//! There are no street dimensions written as literals in this crate; every
//! width comes from an entry id here (the `caMetric()` rule of CLAUDE.md).

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

use crate::{DataPaths, Road};

#[derive(Deserialize)]
struct Entry {
    id: String,
    #[serde(rename = "metricValue", default)]
    metric_value: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct Source {
    entries: Vec<Entry>,
}

pub struct CaStandards {
    metric: HashMap<String, serde_json::Value>,
    /// Every id the table lists, verified or not: what a deviation may name.
    known: HashSet<String>,
}

/// Lane of an SF city street with no survey: SF Better Streets' 10 ft
/// (`SF_CITY_LANE_MIN_M` / `SURVEY_TRAVEL_LANE_M` in src/citygen/sf-data.js).
pub const CITY_LANE_ID: &str = "travel-lane-clearance-at-bulbout-auto-sf";
/// HDM 301.1 State route lane, 12 ft.
pub const STATE_LANE_ID: &str = "travel-lane-width-state-highway-min";
/// HDM 301.1 urban conventional State route posted <= 40 mph, 11 ft.
pub const STATE_URBAN_LANE_ID: &str = "travel-lane-width-state-highway-urban-low-speed-min";

impl CaStandards {
    pub fn load(paths: &DataPaths) -> Result<Self> {
        let p = paths.repo_root.join("scripts/world-data/sources/california-street-standards.json");
        let bytes = std::fs::read(&p).with_context(|| format!("reading {}", p.display()))?;
        let src: Source = serde_json::from_slice(&bytes)?;
        let known = src.entries.iter().map(|e| e.id.clone()).collect();
        let metric = src
            .entries
            .into_iter()
            .filter_map(|e| e.metric_value.filter(|v| !v.is_null()).map(|v| (e.id, v)))
            .collect();
        Ok(Self { metric, known })
    }

    /// `caMetric(id)` for a scalar entry. Errors when the source has no verified number.
    pub fn metric(&self, id: &str) -> Result<f64> {
        self.metric
            .get(id)
            .and_then(|v| v.as_f64())
            .ok_or_else(|| anyhow!("California street standard \"{id}\" has no verified scalar metric value"))
    }

    /// `caMetric(id, key)` for an entry whose metric is an object: the number under `key`.
    pub fn metric_key(&self, id: &str, key: &str) -> Result<f64> {
        self.metric
            .get(id)
            .and_then(|v| v.get(key))
            .and_then(|v| v.as_f64())
            .ok_or_else(|| anyhow!("California street standard \"{id}\" [{key}] has no verified scalar metric value"))
    }

    /// `caMetric(id, key)` for a `[lo, hi]` range under `key`.
    pub fn metric_range(&self, id: &str, key: &str) -> Result<[f64; 2]> {
        let v = self
            .metric
            .get(id)
            .and_then(|v| v.get(key))
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow!("California street standard \"{id}\" [{key}] is not a range"))?;
        match (v.first().and_then(|x| x.as_f64()), v.get(1).and_then(|x| x.as_f64())) {
            (Some(a), Some(b)) => Ok([a, b]),
            _ => Err(anyhow!("California street standard \"{id}\" [{key}] is not a numeric range")),
        }
    }

    /// `ceilToCm` in sf-data.js: a width drawn to the centimetre, never under its standard.
    pub fn ceil_to_cm(m: f64) -> f64 {
        (m * 100.0 - 1e-6).ceil() / 100.0
    }

    /// Class-model lane width of one way (`classLaneWidthOf` in sf-data.js):
    /// the HDM lane on a State route, else the 10 ft city lane.
    pub fn class_lane_width(&self, road: &Road) -> Result<f64> {
        let state = self.state_route_lane_min(road)?;
        Ok(if state > 0.0 { Self::ceil_to_cm(state) } else { Self::ceil_to_cm(self.metric(CITY_LANE_ID)?) })
    }

    /// `sfStateRouteLaneMin` in sf-data.js.
    pub fn state_route_lane_min(&self, road: &Road) -> Result<f64> {
        let hw = road.highway.trim().to_ascii_lowercase();
        if !is_sf_state_route(&hw, &road.name) {
            return Ok(0.0);
        }
        if hw == "motorway" || hw == "motorway_link" {
            return self.metric(STATE_LANE_ID);
        }
        let posted = if road.maxspeed.is_empty() { 0 } else { parse_speed_limit_kmh(&road.maxspeed) };
        if posted > parse_speed_limit_kmh("40 mph") {
            self.metric(STATE_LANE_ID)
        } else {
            self.metric(STATE_URBAN_LANE_ID)
        }
    }
}

/// `isSfStateRoute` in src/world/streets/california-street-standards.js.
pub fn is_sf_state_route(highway: &str, name: &str) -> bool {
    matches!(highway, "motorway" | "motorway_link" | "trunk" | "trunk_link") || {
        let n = name.trim().to_ascii_lowercase();
        highway == "primary" && (n == "sloat boulevard" || n == "skyline boulevard")
    }
}

/// `parseSpeedLimitKmh` in src/citygen/street-geometry.js.
pub fn parse_speed_limit_kmh(raw: &str) -> i64 {
    let v = raw.trim();
    if v.is_empty() {
        return 0;
    }
    let b = v.as_bytes();
    if b.len() >= 3 && b[0].is_ascii_uppercase() && b[1].is_ascii_uppercase() && b[2] == b':' {
        return 0;
    }
    let num_end = v.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(v.len());
    let Ok(speed) = v[..num_end].parse::<f64>() else { return 0 };
    let unit = v[num_end..].trim().to_ascii_lowercase();
    match unit.as_str() {
        "mph" => (speed * 1.609).round() as i64,
        "knots" => (speed * 1.852).round() as i64,
        "" | "km/h" | "kph" => speed.round() as i64,
        _ => 0,
    }
}

/// OSM highway classes this slice draws as carriageway ribbons. Footways,
/// paths, cycleways, pedestrian ways and steps are not drawn.
pub fn is_vehicle_way(highway: &str) -> bool {
    matches!(
        highway,
        "motorway"
            | "trunk"
            | "primary"
            | "secondary"
            | "tertiary"
            | "unclassified"
            | "residential"
            | "service"
            | "living_street"
            | "motorway_link"
            | "trunk_link"
            | "primary_link"
            | "secondary_link"
            | "tertiary_link"
            | "residential_link"
    )
}

// ---------------------------------------------------------------------------
// Game deviations (`CALIFORNIA_STREET_DEVIATIONS` / `caDeviation` in
// src/world/streets/california-street-standards.js, under the rules of
// Docs/CALIFORNIA-STREET-STANDARDS.md): a value that departs from the
// standard and from the survey is used only once the user has approved it in
// chat. An entry names the standard it departs from, the game value, the
// reason and the status; an approved entry also says who approved it and
// when. `deviation()` refuses anything else.
// ---------------------------------------------------------------------------

/// Where an entry stands. Only `Approved` can be read through `deviation()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviationStatus {
    Pending,
    Approved,
    Rejected,
}

/// One recorded departure from a California street standard.
#[derive(Clone, Copy, Debug)]
pub struct Deviation {
    pub id: &'static str,
    /// The standards-table id the game value departs from (JS `standard`).
    pub standard: &'static str,
    /// Other standards-table ids of the same family the same value departs from.
    pub also: &'static [&'static str],
    /// The game value (JS `gameValue`).
    pub game_value: f64,
    /// What the game value is.
    pub unit: &'static str,
    pub reason: &'static str,
    pub status: DeviationStatus,
    /// Who approved it (JS `approvedBy`): a role, never a person's name or address.
    pub approved_by: Option<&'static str>,
    /// When (JS `approvedOn`, YYYY-MM-DD).
    pub approved_on: Option<&'static str>,
}

/// The reason every ground-plan entry carries.
const GROUND_PLAN_REASON: &str = "Street size, the street-size trial's variant (d) as chosen by the user: the city's ground plan is \
drawn 1.25 times wider about the map origin, together with the lower player camera. Every horizontal width of the street plan \
(carriageway and lanes, parking, cycle lanes and buffers, shoulders, medians, footways, right of way) and the survey's horizontal \
distances (blocks, building footprints, terrain spacing, surveyed furniture, the bridges' spans) are 1.25 times the surveyed or \
standard value; the kerb height, paint widths, crosswalk and limit-line dimensions, corner radii, every height and every model \
stay real size. Asked in chat: Whichever width you pick departs from California street standards, so only you can approve it.";

/// The game's recorded deviations. `ground-plan-*` record the one ground-plan
/// factor once per family of widths it changes, each against the standard
/// that governs that family's width where no survey exists (the JS registry
/// names one standard an entry; `also` lists the family's other table ids).
pub const CALIFORNIA_STREET_DEVIATIONS: &[Deviation] = &[
    Deviation {
        id: "ground-plan-travel-lanes",
        standard: CITY_LANE_ID,
        also: &[STATE_LANE_ID, STATE_URBAN_LANE_ID, "left-turn-lane-width-state-highway"],
        game_value: 1.25,
        unit: "times the surveyed or standard travel-lane width",
        reason: GROUND_PLAN_REASON,
        status: DeviationStatus::Approved,
        approved_by: Some("user (chat)"),
        approved_on: Some("2026-11-15"),
    },
    Deviation {
        id: "ground-plan-parking-lanes",
        standard: "parking-space-marking-layout",
        also: &["parking-stall-desirable-size", "parking-shoulder-width-state-highway"],
        game_value: 1.25,
        unit: "times the surveyed or standard parking-lane width",
        reason: GROUND_PLAN_REASON,
        status: DeviationStatus::Approved,
        approved_by: Some("user (chat)"),
        approved_on: Some("2026-11-15"),
    },
    Deviation {
        id: "ground-plan-bike-lanes",
        standard: "bike-lane-class-ii-width-min",
        also: &["bike-lane-class-ii-width-adjacent-parking", "bike-lane-class-ii-width-over-40mph", "bike-lane-class-iv-one-way-width"],
        game_value: 1.25,
        unit: "times the surveyed or standard bike-lane width",
        reason: GROUND_PLAN_REASON,
        status: DeviationStatus::Approved,
        approved_by: Some("user (chat)"),
        approved_on: Some("2026-11-15"),
    },
    Deviation {
        id: "ground-plan-buffers",
        standard: "bike-lane-buffer",
        also: &["bike-lane-class-iv-buffer", "shoulder-width-conventional-highway"],
        game_value: 1.25,
        unit: "times the surveyed or standard buffer and shoulder width",
        reason: GROUND_PLAN_REASON,
        status: DeviationStatus::Approved,
        approved_by: Some("user (chat)"),
        approved_on: Some("2026-11-15"),
    },
    Deviation {
        id: "ground-plan-medians",
        // The table's only median width is the two-way left-turn lane; a raised
        // median's width is surveyed only (the street contract's lane layout).
        standard: "twltl-width-state-highway",
        also: &[],
        game_value: 1.25,
        unit: "times the surveyed or standard median width",
        reason: GROUND_PLAN_REASON,
        status: DeviationStatus::Approved,
        approved_by: Some("user (chat)"),
        approved_on: Some("2026-11-15"),
    },
    Deviation {
        id: "ground-plan-sidewalks",
        standard: "sidewalk-width-sf-commercial-throughway",
        also: &[
            "sidewalk-width-sf-neighborhood-commercial",
            "sidewalk-width-sf-downtown-residential",
            "sidewalk-width-sf-residential-throughway",
            "sidewalk-width-sf-neighborhood-residential",
            "sidewalk-width-sf-industrial",
            "sidewalk-width-sf-mixed-use",
            "sidewalk-width-sf-parkway",
            "sidewalk-width-sf-park-edge",
            "sidewalk-width-sf-multi-way-boulevard",
            "sidewalk-width-sf-alley",
            "sidewalk-throughway-zone-sf",
            "sidewalk-frontage-zone-sf",
            "sidewalk-furnishing-zone-sf",
            "sidewalk-width-state-highway",
        ],
        game_value: 1.25,
        unit: "times the surveyed or standard sidewalk width",
        reason: GROUND_PLAN_REASON,
        status: DeviationStatus::Approved,
        approved_by: Some("user (chat)"),
        approved_on: Some("2026-11-15"),
    },
    // "keep the hills steep" (2026-11-16).
    Deviation {
        id: TERRAIN_HEIGHTS_DEVIATION,
        // The profile grades are what the decision keeps: with the plan 1.25 x
        // wider and the heights real, every grade was 1 / 1.25 of the surveyed
        // one; the heights above sea level follow the plan so each drawn grade
        // is the surveyed grade again (the table's grade limits are state
        // highway only; a city street's grade is surveyed only).
        standard: "ramp-profile-grade-max",
        also: &["connector-profile-grade-max", "freeway-profile-grade-max"],
        game_value: 1.25,
        unit: "times the surveyed terrain height above mean sea level (the ground-plan factor), so every drawn grade is the surveyed grade",
        reason: TERRAIN_HEIGHTS_REASON,
        status: DeviationStatus::Approved,
        approved_by: Some("user (chat)"),
        approved_on: Some("2026-11-16"),
    },
];

/// the entry that makes the terrain heights follow the ground plan.
pub const TERRAIN_HEIGHTS_DEVIATION: &str = "ground-plan-terrain-heights";

/// The reason the terrain-heights entry carries.
const TERRAIN_HEIGHTS_REASON: &str = "Keep the hills steep, as chosen by the user: with the ground plan 1.25 times wider and the \
heights real, every slope was 1 / 1.25 of the surveyed one (the steepest street drawn at 28.4 % instead of 35.5 %). The terrain \
height above mean sea level is multiplied by the ground-plan factor, so every grade equals the surveyed grade; sea level, the \
shoreline and the bay floor stay; buildings, people, vehicles, props, the kerb height and every model stay real size and stand \
on the raised ground. Asked in chat: keep the hills as steep as the real ones by raising the ground heights 1.25 times too?";

/// true when the approved terrain-heights entry makes the heights
/// follow the ground plan (its factor must equal the ground-plan factor).
pub fn terrain_heights_follow_plan() -> Result<bool> {
    terrain_heights_follow_plan_in(CALIFORNIA_STREET_DEVIATIONS)
}

/// `terrain_heights_follow_plan` over `registry`: false when the entry is
/// absent; an error when it is present but not approved, or its factor is
/// not the ground-plan factor.
pub fn terrain_heights_follow_plan_in(registry: &[Deviation]) -> Result<bool> {
    if !registry.iter().any(|d| d.id == TERRAIN_HEIGHTS_DEVIATION) {
        return Ok(false);
    }
    let v = deviation_in(registry, TERRAIN_HEIGHTS_DEVIATION)?;
    let p = ground_plan_scale_in(registry)?;
    if v != p {
        return Err(anyhow!("deviation \"{TERRAIN_HEIGHTS_DEVIATION}\" is {v}, the ground-plan factor {p}"));
    }
    Ok(true)
}

/// The entries the ground-plan factor is read through (one per width family).
pub const GROUND_PLAN_DEVIATIONS: &[&str] = &["ground-plan-travel-lanes", "ground-plan-parking-lanes", "ground-plan-bike-lanes", "ground-plan-buffers", "ground-plan-medians", "ground-plan-sidewalks"];

/// `caDeviation(id)` over `registry`: the game value of an approved entry; an
/// error for an unknown id, an entry that is not approved, or one approved
/// without `approved_by` / `approved_on`.
pub fn deviation_in(registry: &[Deviation], id: &str) -> Result<f64> {
    let d = registry.iter().find(|d| d.id == id).ok_or_else(|| anyhow!("no recorded deviation \"{id}\""))?;
    if d.status != DeviationStatus::Approved {
        return Err(anyhow!("deviation \"{id}\" is {:?}, not approved by the user", d.status));
    }
    if d.approved_by.is_none_or(|s| s.trim().is_empty()) || d.approved_on.is_none_or(|s| s.trim().is_empty()) {
        return Err(anyhow!("deviation \"{id}\" is approved without approvedBy/approvedOn"));
    }
    Ok(d.game_value)
}

/// `caDeviation(id)` over the game's registry.
pub fn deviation(id: &str) -> Result<f64> {
    deviation_in(CALIFORNIA_STREET_DEVIATIONS, id)
}

/// The ground-plan factor read through every `GROUND_PLAN_DEVIATIONS` entry of
/// `registry`: each must be approved, and all must carry the same value.
pub fn ground_plan_scale_in(registry: &[Deviation]) -> Result<f64> {
    let mut value: Option<f64> = None;
    for id in GROUND_PLAN_DEVIATIONS {
        let v = deviation_in(registry, id)?;
        if !(v.is_finite() && v > 0.0) {
            return Err(anyhow!("deviation \"{id}\" has no usable factor ({v})"));
        }
        match value {
            None => value = Some(v),
            Some(w) if w == v => {}
            Some(w) => return Err(anyhow!("ground-plan deviations disagree: \"{id}\" is {v}, the others {w}")),
        }
    }
    value.ok_or_else(|| anyhow!("no ground-plan deviation recorded"))
}

/// The approved ground-plan factor: the default of `groundscale::scale()`.
pub fn ground_plan_scale() -> Result<f64> {
    ground_plan_scale_in(CALIFORNIA_STREET_DEVIATIONS)
}

impl CaStandards {
    /// The browser verifier's deviation check (A) over a native registry:
    /// unique ids; every entry names a standard the table has (its `also` ids
    /// too), carries a reason and a finite game value; an approved entry
    /// carries `approved_by` and `approved_on`.
    pub fn check_deviations(&self, registry: &[Deviation]) -> Result<()> {
        let mut seen = HashSet::new();
        for d in registry {
            if !seen.insert(d.id) {
                return Err(anyhow!("deviation \"{}\" is recorded twice", d.id));
            }
            for s in std::iter::once(&d.standard).chain(d.also.iter()) {
                if !self.known.contains(*s) {
                    return Err(anyhow!("deviation \"{}\" names unknown standard \"{s}\"", d.id));
                }
            }
            if d.reason.trim().is_empty() || !d.game_value.is_finite() {
                return Err(anyhow!("deviation \"{}\" needs standard, gameValue, reason, status", d.id));
            }
            if d.status == DeviationStatus::Approved && (d.approved_by.is_none_or(|s| s.trim().is_empty()) || d.approved_on.is_none_or(|s| s.trim().is_empty())) {
                return Err(anyhow!("deviation \"{}\" is approved without approvedBy/approvedOn", d.id));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod deviation_tests {
    use super::*;

    fn entry(id: &'static str, status: DeviationStatus, by: Option<&'static str>, on: Option<&'static str>, v: f64) -> Deviation {
        Deviation { id, standard: CITY_LANE_ID, also: &[], game_value: v, unit: "", reason: "test", status, approved_by: by, approved_on: on }
    }

    #[test]
    fn lookup_refuses_anything_not_approved() {
        let reg = [
            entry("a", DeviationStatus::Approved, Some("user (chat)"), Some("2026-11-15"), 1.25),
            entry("p", DeviationStatus::Pending, None, None, 1.25),
            entry("r", DeviationStatus::Rejected, None, None, 1.25),
            entry("nb", DeviationStatus::Approved, None, Some("2026-11-15"), 1.25),
            entry("no", DeviationStatus::Approved, Some("user (chat)"), None, 1.25),
        ];
        assert_eq!(deviation_in(&reg, "a").unwrap(), 1.25);
        for id in ["p", "r", "nb", "no", "missing"] {
            assert!(deviation_in(&reg, id).is_err(), "{id} must be refused");
        }
    }

    #[test]
    fn ground_plan_factor_needs_every_family_approved_and_equal() {
        assert_eq!(ground_plan_scale().unwrap(), 1.25);
        let mut reg: Vec<Deviation> = CALIFORNIA_STREET_DEVIATIONS.to_vec();
        reg[2].status = DeviationStatus::Pending;
        assert!(ground_plan_scale_in(&reg).is_err(), "one pending family refuses the factor");
        let mut reg: Vec<Deviation> = CALIFORNIA_STREET_DEVIATIONS.to_vec();
        reg[0].game_value = 1.2;
        assert!(ground_plan_scale_in(&reg).is_err(), "families that disagree refuse the factor");
        let reg: Vec<Deviation> = CALIFORNIA_STREET_DEVIATIONS.iter().copied().filter(|d| d.id != "ground-plan-medians").collect();
        assert!(ground_plan_scale_in(&reg).is_err(), "a missing family refuses the factor");
    }

    #[test]
    fn every_entry_is_well_formed_and_names_no_person() {
        for d in CALIFORNIA_STREET_DEVIATIONS {
            assert_eq!(d.status, DeviationStatus::Approved, "{}", d.id);
            assert_eq!(d.approved_by, Some("user (chat)"), "{}", d.id);
            // the six width families on 2026-11-15, the terrain heights on 2026-11-16.
            let on = if d.id == TERRAIN_HEIGHTS_DEVIATION { "2026-11-16" } else { "2026-11-15" };
            assert_eq!(d.approved_on, Some(on), "{}", d.id);
            assert!(!d.reason.contains('@'), "{}: no address in the record", d.id);
        }
        assert_eq!(CALIFORNIA_STREET_DEVIATIONS.len(), GROUND_PLAN_DEVIATIONS.len() + 1);
    }

    #[test]
    fn terrain_heights_follow_the_plan_only_when_approved_and_equal() {
        // hills
        assert!(terrain_heights_follow_plan().unwrap());
        let mut reg: Vec<Deviation> = CALIFORNIA_STREET_DEVIATIONS.to_vec();
        let i = reg.iter().position(|d| d.id == TERRAIN_HEIGHTS_DEVIATION).unwrap();
        reg[i].status = DeviationStatus::Pending;
        assert!(terrain_heights_follow_plan_in(&reg).is_err(), "a pending entry refuses");
        let mut reg: Vec<Deviation> = CALIFORNIA_STREET_DEVIATIONS.to_vec();
        reg[i].game_value = 1.2;
        assert!(terrain_heights_follow_plan_in(&reg).is_err(), "a factor other than the plan's refuses");
        let reg: Vec<Deviation> = CALIFORNIA_STREET_DEVIATIONS.iter().copied().filter(|d| d.id != TERRAIN_HEIGHTS_DEVIATION).collect();
        assert!(!terrain_heights_follow_plan_in(&reg).unwrap(), "no entry: heights stay surveyed");
    }
}
