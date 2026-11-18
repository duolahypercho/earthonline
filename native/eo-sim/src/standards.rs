//! The California street standards the simulation obeys, read from the cited
//! source the JS table is generated from
//! (`scripts/world-data/sources/california-street-standards.json`), the same
//! file `eo_data::CaStandards` reads. Every signal interval, crossing walking
//! speed, prima facie limit, lane, sidewalk and crosswalk dimension the
//! simulation uses comes from an entry id here - the `caMetric()` rule of
//! CLAUDE.md. Nothing in this crate writes a street number as a literal; the
//! few geometric constants of the signal plan that are not standards
//! (`GREEN_*`, `PROGRESSION_SPEED_MPS`, ...) are the JS plan's own and are
//! named in `SignalTiming`.

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use std::collections::HashMap;

use eo_data::DataPaths;

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

/// `caMetric(id, key?)` over the cited source.
pub struct Standards {
    metric: HashMap<String, serde_json::Value>,
}

impl Standards {
    pub fn load(paths: &DataPaths) -> Result<Self> {
        let p = paths.repo_root.join("scripts/world-data/sources/california-street-standards.json");
        let bytes = std::fs::read(&p).with_context(|| format!("reading {}", p.display()))?;
        Self::parse(&bytes)
    }

    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let src: Source = serde_json::from_slice(bytes)?;
        let metric = src
            .entries
            .into_iter()
            .filter_map(|e| e.metric_value.filter(|v| !v.is_null()).map(|v| (e.id, v)))
            .collect();
        Ok(Self { metric })
    }

    /// `caMetric(id)`.
    pub fn metric(&self, id: &str) -> Result<f64> {
        self.metric
            .get(id)
            .and_then(|v| v.as_f64())
            .ok_or_else(|| anyhow!("California street standard \"{id}\" has no verified scalar metric value"))
    }

    /// `caMetric(id, key)`.
    pub fn metric_key(&self, id: &str, key: &str) -> Result<f64> {
        self.metric
            .get(id)
            .and_then(|v| v.get(key))
            .and_then(|v| v.as_f64())
            .ok_or_else(|| anyhow!("California street standard \"{id}\" [{key}] has no verified metric value"))
    }

    /// The keys of an object-valued entry, in file order.
    pub fn metric_table(&self, id: &str) -> Result<Vec<(String, f64)>> {
        let obj = self
            .metric
            .get(id)
            .and_then(|v| v.as_object())
            .ok_or_else(|| anyhow!("California street standard \"{id}\" is not a table"))?;
        obj.iter()
            .map(|(k, v)| v.as_f64().map(|f| (k.clone(), f)).ok_or_else(|| anyhow!("\"{id}\" [{k}] is not a number")))
            .collect()
    }
}

/// `FEET_TO_M` in src/world/streets/california-street-standards.js (a unit, not a standard).
pub const FEET_TO_M: f64 = 0.3048;
const MPH_TO_MPS: f64 = (5280.0 * FEET_TO_M) / 3600.0;

/// `SIGNAL_TIMING` in src/simulation/traffic/signal-control-plan.js, with
/// every standards-derived field read through `caMetric`.
#[derive(Clone, Debug)]
pub struct SignalTiming {
    pub yellow_min_s: f64,
    pub yellow_max_s: f64,
    pub all_red_min_s: f64,
    pub all_red_max_s: f64,
    pub ped_walk_s: f64,
    pub ped_speed_mps: f64,
    pub ped_total_speed_mps: f64,
    pub ped_total_start_behind_m: f64,
    pub ped_buffer_s: f64,
    /// Table 4D-102(CA): ascending (mph, open "+" row, seconds).
    pub yellow_by_posted: Vec<(f64, bool, f64)>,
    /// CVC 22352 prima facie business/residence speed, m/s.
    pub prima_facie_mps: f64,
    // The plan's own geometry/model numbers (not standards; signal-control-plan.js).
    pub green_min_s: f64,
    pub green_base_s: f64,
    pub green_per_lane_s: f64,
    pub green_max_s: f64,
    pub nominal_vehicle_len_m: f64,
    pub progression_speed_mps: f64,
    pub yellow_perception_s: f64,
    pub yellow_decel: f64,
    pub max_phases: usize,
    pub ped_fallback_crossing_m: f64,
    pub axis_tolerance_rad: f64,
}

impl SignalTiming {
    pub fn from_standards(s: &Standards) -> Result<Self> {
        let mut yellow_by_posted: Vec<(f64, bool, f64)> = s
            .metric_table("yellow-change-min-by-posted-speed")?
            .into_iter()
            .map(|(k, v)| (parse_leading_f64(&k), k.trim().ends_with('+'), v))
            .collect();
        yellow_by_posted.sort_by(|a, b| a.0.total_cmp(&b.0));
        Ok(Self {
            yellow_min_s: s.metric_key("yellow-change-range", "min")?,
            yellow_max_s: s.metric_key("yellow-change-range", "max")?,
            all_red_min_s: s.metric_key("red-clearance", "typicalMin")?,
            all_red_max_s: s.metric_key("red-clearance", "max")?,
            ped_walk_s: s.metric_key("ped-walk-interval", "min")?,
            ped_speed_mps: s.metric_key("ped-clearance-walking-speed", "default")?,
            ped_total_speed_mps: s.metric_key("ped-total-crossing-walking-speed", "speed")?,
            ped_total_start_behind_m: s.metric_key("ped-total-crossing-walking-speed", "startBehindCurb")?,
            ped_buffer_s: s.metric("ped-buffer-interval")?,
            yellow_by_posted,
            prima_facie_mps: s.metric("speed-prima-facie-business-residence")?,
            green_min_s: 6.0,
            green_base_s: 14.0,
            green_per_lane_s: 2.0,
            green_max_s: 26.0,
            nominal_vehicle_len_m: 5.5,
            progression_speed_mps: 11.0,
            yellow_perception_s: 1.0,
            yellow_decel: 3.05,
            max_phases: 4,
            ped_fallback_crossing_m: 10.0,
            axis_tolerance_rad: (25.0 * std::f64::consts::PI) / 180.0,
        })
    }

    /// `yellowMinimumForPostedSpeedS`.
    pub fn yellow_minimum_for_posted(&self, posted_mps: f64) -> f64 {
        let to_kmh = |mps: f64| (mps * 3600.0) / 1000.0;
        let kmh = js_round(to_kmh(posted_mps));
        for &(mph, open, s) in &self.yellow_by_posted {
            if open || js_round(to_kmh(mph * MPH_TO_MPS)) >= kmh {
                return s;
            }
        }
        self.yellow_by_posted.last().map(|r| r.2).unwrap_or(self.yellow_min_s)
    }

    /// `yellowChangeIntervalS`.
    pub fn yellow_change_interval(&self, posted_mps: f64, design_mps: f64) -> f64 {
        let ite = self.yellow_perception_s + design_mps / (2.0 * self.yellow_decel);
        self.yellow_minimum_for_posted(posted_mps).max(ite).clamp(self.yellow_min_s, self.yellow_max_s)
    }

    /// `pedestrianTimingFor(crossingM)`: (walk, clearance, span, required total).
    pub fn pedestrian_timing(&self, crossing_m: f64) -> (f64, f64, f64, f64) {
        let clearance = crossing_m / self.ped_speed_mps;
        let required = (crossing_m + self.ped_total_start_behind_m) / self.ped_total_speed_mps;
        let walk = self.ped_walk_s.max(required - clearance);
        (walk, clearance, walk + clearance, required)
    }
}

/// `Math.round`: halves round toward +infinity.
pub fn js_round(v: f64) -> f64 {
    (v + 0.5).floor()
}

fn parse_leading_f64(s: &str) -> f64 {
    let t = s.trim();
    let end = t.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(t.len());
    t[..end].parse().unwrap_or(f64::NAN)
}

/// Street dimensions the native network builder needs, every one a `caMetric`.
#[derive(Clone, Debug)]
pub struct StreetDims {
    /// 10 ft SF city lane (`travel-lane-clearance-at-bulbout-auto-sf`).
    pub city_lane_m: f64,
    /// 12 ft State route lane.
    pub state_lane_m: f64,
    /// 11 ft urban low-speed State route lane.
    pub state_urban_lane_m: f64,
    /// Sidewalk minimums by SF street type.
    pub sidewalk_commercial_m: f64,
    pub sidewalk_residential_m: f64,
    pub sidewalk_alley_m: f64,
    pub sidewalk_industrial_m: f64,
    /// Where the walking line sits: frontage zone, then the throughway's middle.
    pub frontage_zone_m: f64,
    pub throughway_m: f64,
    /// Crosswalk width and the limit line's setback from it.
    pub crosswalk_m: f64,
    pub limit_line_setback_m: f64,
    pub limit_line_width_m: f64,
}

impl StreetDims {
    pub fn from_standards(s: &Standards) -> Result<Self> {
        Ok(Self {
            city_lane_m: s.metric("travel-lane-clearance-at-bulbout-auto-sf")?,
            state_lane_m: s.metric("travel-lane-width-state-highway-min")?,
            state_urban_lane_m: s.metric("travel-lane-width-state-highway-urban-low-speed-min")?,
            sidewalk_commercial_m: s.metric_key("sidewalk-width-sf-commercial-throughway", "minimum")?,
            sidewalk_residential_m: s.metric_key("sidewalk-width-sf-neighborhood-residential", "minimum")?,
            sidewalk_alley_m: s.metric_key("sidewalk-width-sf-alley", "minimum")?,
            sidewalk_industrial_m: s.metric_key("sidewalk-width-sf-industrial", "minimum")?,
            frontage_zone_m: s.metric_key("sidewalk-frontage-zone-sf", "allStreets")?,
            throughway_m: s.metric_key("sidewalk-throughway-zone-sf", "default")?,
            crosswalk_m: s.metric_key("crosswalk-min-width", "general")?,
            limit_line_setback_m: s.metric_key("limit-line-setback-from-crosswalk", "minFromCrosswalk")?,
            limit_line_width_m: s.metric_key("limit-line-width", "max")?,
        })
    }

    /// `ceilToCm` in sf-data.js.
    pub fn ceil_to_cm(m: f64) -> f64 {
        (m * 100.0 - 1e-6).ceil() / 100.0
    }
}
