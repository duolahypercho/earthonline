//! per-building APPEARANCE (src/world/buildings/building-appearance.js,
//! `building-appearance-v1`): the age, upkeep and paint history that scale a
//! kit building's weathering, its sill rhythm, and the packed vec4 the JS tile
//! stream writes into every vertex of an aged finish (`archetypeAge`). The
//! renderer reads the packed value per building (eo-render facade pass).
//!
//! Inputs as `archetypeAppearanceFor` (create-archetype-building.js) passes
//! them: the loaded id (`sf-building-<key>`), the classified archetype, its
//! published civic record and year. Tested against the node-run JS
//! (the earlier browser version's exporter, eo-render test `facade`).

use crate::classify::Classified;
use crate::elevation::Plan;
use crate::rng::hash_seed;

pub const APPEARANCE_REFERENCE_YEAR: f64 = 2026.0;
const ROLL_YEAR_PLACEHOLDER: f64 = 1900.0;
pub const PLACEHOLDER_ERA_MIDPOINT: f64 = 1890.0;
const DEFAULT_ERA_YEAR: f64 = 1935.0;
pub const APPEARANCE_ORIGIN_CELL_M: f64 = 32.0;
/// `APPEARANCE_PITCH` (min, step) and `APPEARANCE_FIRST_SILL` (min, step).
pub const APPEARANCE_PITCH: (f64, f64) = (2.5, 0.125);
pub const APPEARANCE_FIRST_SILL: (f64, f64) = (0.6, 0.25);
const FRACTION_MAX: f64 = 0.999;

fn era_year(archetype: &str) -> f64 {
    match archetype {
        "victorian-row" => 1890.0,
        "bay-window-apartment" => 1912.0,
        "edwardian-flats" => 1914.0,
        "soma-warehouse" => 1922.0,
        "retail-strip" => 1928.0,
        "stucco-row" => 1938.0,
        "cottage-garage" => 1940.0,
        "midcentury-stucco" => 1958.0,
        "industrial-shed" => 1958.0,
        "civic-concrete" => 1962.0,
        "downtown-tower" => 1978.0,
        _ => DEFAULT_ERA_YEAR,
    }
}

fn use_upkeep(code: &str) -> f64 {
    match code {
        "COMO" | "COMH" => 0.12,
        "COMM" | "GOVT" => 0.1,
        "COMR" => 0.08,
        "SRES" => 0.03,
        "MRES" => -0.06,
        "MISC" => -0.08,
        "IND" => -0.18,
        _ => 0.0,
    }
}

const COASTAL_DISTRICTS: [&str; 4] = ["Sunset/Parkside", "Outer Richmond", "Lakeshore", "Seacliff"];

#[derive(Clone, Debug, PartialEq)]
pub struct Appearance {
    pub year_built: f64,
    pub year_source: &'static str,
    pub age_years: f64,
    pub maintenance: f64,
    pub coastal: bool,
    pub grime_seed: u32,
    pub wear: f64,
    pub paint: f64,
}

/// JS `Math.round` (half toward +infinity).
fn js_round(v: f64) -> f64 {
    (v + 0.5).floor()
}
fn round3(v: f64) -> f64 {
    js_round(v * 1000.0) / 1000.0
}
fn clamp01(v: f64) -> f64 {
    v.clamp(0.0, 1.0)
}
fn hash01(key: &str) -> f64 {
    hash_seed(key) as f64 / 4294967296.0
}

/// `resolveYear`.
fn resolve_year(archetype: &str, civic_year: Option<f64>, civic_exact: Option<bool>, year_built: Option<f64>) -> (f64, &'static str) {
    let era = era_year(archetype);
    let roll = civic_year.filter(|y| y.is_finite() && *y > 1800.0);
    let year = roll.or(year_built.filter(|y| *y > 1800.0));
    let Some(year) = year else { return (era, "archetype-era") };
    if year == ROLL_YEAR_PLACEHOLDER && civic_exact != Some(true) {
        return (PLACEHOLDER_ERA_MIDPOINT.min(era), "placeholder-era-midpoint");
    }
    (year.min(APPEARANCE_REFERENCE_YEAR), if civic_exact == Some(true) { "roll-exact" } else { "roll" })
}

/// `buildingAppearance` with `archetypeAppearanceFor`'s inputs: `id` the
/// loaded id (`sf-building-<key>`), `c` the planned classification
/// (`planned.classified || classified`), `record_year` the loaded record's
/// `yearBuilt` (sf-data's 1900 default).
pub fn building_appearance(id: &str, c: &Classified, record_year: f64) -> Appearance {
    let seed_key = format!("appearance|{id}");
    let civic = c.civic.as_ref();
    // classified?.yearBuilt || building?.yearBuilt || building?.civic?.yearBuilt || null
    let year_in = if c.year_built != 0.0 && c.year_built.is_finite() {
        Some(c.year_built)
    } else if record_year != 0.0 && record_year.is_finite() {
        Some(record_year)
    } else {
        None
    };
    let (year, source) = resolve_year(c.archetype, civic.and_then(|v| v.year_built), civic.and_then(|v| v.year_exact), year_in);
    let age_years = (APPEARANCE_REFERENCE_YEAR - year).max(0.0);
    let use_code = civic.and_then(|v| v.use_code.as_deref());
    let coastal = civic.and_then(|v| v.district.as_deref()).map(|d| COASTAL_DISTRICTS.contains(&d)).unwrap_or(false);
    let mut mean = 0.55 + use_code.map(use_upkeep).unwrap_or(0.0);
    let recent = age_years < 20.0;
    if recent {
        mean = mean.max(0.9 - age_years * 0.01);
    }
    let spread = if recent { 0.2 } else { 0.6 };
    let maintenance = clamp01(mean + (hash01(&format!("{seed_key}|upkeep")) - 0.5) * spread) * 0.93 + 0.05;
    let lerp = |a: f64, b: f64, t: f64| a + (b - a) * t;
    let cycle = lerp(35.0, 4.0, maintenance) * (0.6 + 0.8 * hash01(&format!("{seed_key}|repaint")));
    let paint_years = age_years.min(cycle);
    let modern_paint = if year >= 1990.0 { 0.6 } else { 1.0 };
    let paint = clamp01((paint_years / 30.0) * modern_paint * if coastal { 1.3 } else { 1.0 });
    let eff_age = (age_years - 6.0).max(0.0);
    let wear = clamp01((1.0 - (-eff_age / 45.0).exp()) * (1.0 - 0.55 * maintenance) * if coastal { 1.1 } else { 1.0 });
    Appearance {
        year_built: year,
        year_source: source,
        age_years,
        maintenance: round3(maintenance),
        coastal,
        grime_seed: hash_seed(&format!("{seed_key}|grime")) % 64,
        wear: round3(wear),
        paint: round3(paint),
    }
}

/// `sillRhythm(plan)`: the lowest sill above the base and the median storey
/// pitch between sills.
pub fn sill_rhythm(plan: &Plan) -> (f64, f64) {
    let base_y = plan.base_y;
    let mut ys: Vec<f64> = vec![];
    for p in &plan.parts {
        if p.kind == "sill" && p.y0.is_finite() {
            let y = js_round((p.y0 - base_y) * 10.0) / 10.0;
            if !ys.contains(&y) {
                ys.push(y);
            }
        }
    }
    let mut sorted: Vec<f64> = ys.into_iter().filter(|y| *y > 0.2).collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let first = sorted.first().copied().unwrap_or(0.9);
    let mut gaps: Vec<f64> = sorted.windows(2).map(|w| w[1] - w[0]).filter(|g| *g > 2.2 && *g < 5.0).collect();
    gaps.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pitch = if gaps.is_empty() { 3.0 } else { gaps[gaps.len() >> 1] };
    (first, pitch)
}

/// `packAppearance`: x = grimeSeed + wear, y = coastal * 256 + pitchCode * 16
/// + firstCode + paint, z = baseY, w = the 32 m origin cell.
pub fn pack_appearance(a: &Appearance, base_y: f64, centre: [f64; 2], first_sill_m: f64, pitch_m: f64) -> [f32; 4] {
    let wear = clamp01(a.wear).min(FRACTION_MAX);
    let paint = clamp01(a.paint).min(FRACTION_MAX);
    let seed = (a.grime_seed as f64).floor().clamp(0.0, 63.0);
    let pitch_code = js_round((pitch_m - APPEARANCE_PITCH.0) / APPEARANCE_PITCH.1).clamp(0.0, 15.0);
    let first_code = js_round((first_sill_m - APPEARANCE_FIRST_SILL.0) / APPEARANCE_FIRST_SILL.1).clamp(0.0, 15.0);
    let ix = js_round(centre[0] / APPEARANCE_ORIGIN_CELL_M).clamp(-511.0, 511.0);
    let iz = js_round(centre[1] / APPEARANCE_ORIGIN_CELL_M).clamp(-511.0, 511.0);
    let coastal = if a.coastal { 1.0 } else { 0.0 };
    [
        (seed + wear) as f32,
        (coastal * 256.0 + pitch_code * 16.0 + first_code + paint) as f32,
        base_y as f32,
        ((ix + 512.0) * 1024.0 + (iz + 512.0)) as f32,
    ]
}

/// `polygonCentroid` (archetype-tile-stream.js): the mean of the points.
pub fn polygon_centroid(points: &[crate::frame::P2]) -> [f64; 2] {
    let n = points.len().max(1) as f64;
    let (x, z) = points.iter().fold((0.0, 0.0), |a, p| (a.0 + p.x, a.1 + p.z));
    [x / n, z / n]
}

/// The packed appearance of one planned building at its floor `base_y`.
pub fn planned_appearance(rec: &crate::pipeline::LoadedRecord, planned: &crate::pipeline::Planned, base_y: f64) -> [f32; 4] {
    let a = building_appearance(&rec.id, &planned.classified, rec.year_built);
    let (first, pitch) = sill_rhythm(&planned.plan);
    pack_appearance(&a, base_y, polygon_centroid(&rec.polygon), first, pitch)
}
