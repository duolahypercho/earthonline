//! the street size. The city's ground plan is drawn `scale()`
//! times wider about the map origin (0, 0): street centrelines and every
//! horizontal width of the street plan, block and building footprints, the
//! terrain grid's horizontal spacing (heights stay real and line up with the
//! scaled streets), surveyed furniture, the shore and the bridges' spans.
//! Heights, the kerb height, paint widths, crosswalks, corner radii and every
//! model (people, cars, props, doors, windows, storeys) stay real size.
//!
//! **The default is the approved deviation, not a literal:** with no override
//! the scale is read through `standards::ground_plan_scale()` (the
//! `ground-plan-*` entries of `CALIFORNIA_STREET_DEVIATIONS`, approved by the
//! user in chat on 2026-11-15: 1.25). `deviation()` refuses an entry that is
//! not approved, and then nothing starts.
//!
//! * `EO_GROUND_SCALE=s` (eo-app / eo-server `--ground-scale s`): an explicit
//!   scale for A/B; `1.0` is today's (surveyed, standard) world.
//! * `EO_LOW_CAMERA=0` (eo-app `--default-camera`): the player's camera as
//!   before; the lower third-person camera is the default.
//! * `EO_PEOPLE_SCALE=p`, `EO_VEHICLE_SCALE=v` (`--people-scale`,
//!   `--vehicle-scale`): drawn people and vehicles scaled (visual only, a
//!   trial switch; 1.0 by default).
//!
//! The hand-built (Blender) buildings and the landmark buildings are moved
//! whole, real size, not scaled; the kit buildings beside them close the
//! leftover strip of their scaled lots (`GroundPlan`, below).
//!
//! The JS-oracle tests compare the port with the 1:1 browser game and pin
//! the scale to 1.0 (`pin`) before anything reads it.

use anyhow::{Context, Result, anyhow, bail};
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

/// The range an explicit scale may take.
pub const SCALE_RANGE: (f64, f64) = (0.5, 3.0);

/// Bump when the ground-plan transform (what `GroundPlan` does to a record)
/// changes: it is part of every cache key of a scaled world (`cache_tag`).
pub const PLAN_VERSION: &str = "ground-plan-3"; // + the seam infill records

static SCALE: OnceLock<(f64, &'static str)> = OnceLock::new();

/// Parse an explicit ground scale (`--ground-scale`, `EO_GROUND_SCALE`).
pub fn parse_scale(v: &str) -> Result<f64> {
    let s: f64 = v.trim().parse().with_context(|| format!("ground scale \"{v}\" is not a number"))?;
    if !(s.is_finite() && (SCALE_RANGE.0..=SCALE_RANGE.1).contains(&s)) {
        bail!("ground scale {s} is outside {}..{}", SCALE_RANGE.0, SCALE_RANGE.1);
    }
    Ok(s)
}

/// Resolve the scale once for the process: a pinned value, else
/// `EO_GROUND_SCALE`, else the approved ground-plan deviation. An error when
/// the override does not parse or the deviation is not approved.
pub fn resolve() -> Result<f64> {
    if let Some((s, _)) = SCALE.get() {
        return Ok(*s);
    }
    let (s, from) = match std::env::var("EO_GROUND_SCALE") {
        Ok(v) if !v.trim().is_empty() => (parse_scale(&v).context("EO_GROUND_SCALE")?, "EO_GROUND_SCALE / --ground-scale"),
        _ => (crate::standards::ground_plan_scale().context("the ground-plan deviation (CALIFORNIA_STREET_DEVIATIONS ground-plan-*)")?, "the approved ground-plan deviation"),
    };
    Ok(SCALE.get_or_init(|| (s, from)).0)
}

/// The ground-plan scale `s` (1.25 by default, read through the approved deviation).
pub fn scale() -> f64 {
    resolve().unwrap_or_else(|e| panic!("groundscale: {e:#}"))
}

/// Where the scale came from (for the start-up line).
pub fn source() -> &'static str {
    let _ = scale();
    SCALE.get().map(|x| x.1).unwrap_or("")
}

/// Pin the scale before anything reads it. The JS-oracle tests pin 1.0 (the
/// JS game is 1:1); panics when the process already uses another value.
pub fn pin(s: f64) {
    let (v, _) = *SCALE.get_or_init(|| (s, "pinned"));
    assert!(v == s, "groundscale: pinned {s}, but this process already uses {v}");
}

/// True when the ground plan is scaled (any scale but 1).
pub fn active() -> bool {
    scale() != 1.0
}

/// A surveyed (1:1) world point `[x, z]` on the scaled ground plan.
pub fn plan(p: [f64; 2]) -> [f64; 2] {
    let s = scale();
    [p[0] * s, p[1] * s]
}

/// `plan` for f32 points.
pub fn plan32(p: [f32; 2]) -> [f32; 2] {
    let s = scale();
    [(p[0] as f64 * s) as f32, (p[1] as f64 * s) as f32]
}

/// What a cache key of anything built on the ground plan adds: empty at
/// scale 1 with heights 1 (the surveyed world's keys unchanged), else the
/// scale and `PLAN_VERSION`, and (hills) the height scale and
/// `HEIGHT_VERSION` whenever heights are not the surveyed ones (plan 1.25
/// with heights 1.0 keeps the keys it had before heights followed the plan).
pub fn cache_tag() -> String {
    let (s, h) = (scale(), height_scale());
    let mut t = String::new();
    if s != 1.0 {
        t = format!("groundscale {s} {PLAN_VERSION}{}", if pull_enabled() { "" } else { " no-pull" });
    }
    if h != 1.0 {
        if !t.is_empty() {
            t.push(' ');
        }
        t.push_str(&format!("heights {h} {HEIGHT_VERSION}"));
    }
    t
}

// ---------------------------------------------------------------------------
// the ground heights follow the plan ("keep the hills steep",
// approved by the user in chat on 2026-11-16).
// ---------------------------------------------------------------------------

/// Bump when the height transform (`lift`) changes: part of every cache key
/// of a world whose heights are scaled (`cache_tag`).
pub const HEIGHT_VERSION: &str = "ground-heights-1";

static HEIGHT: OnceLock<(f64, &'static str)> = OnceLock::new();

/// Resolve the height scale once for the process: a pinned value, else
/// `EO_HEIGHT_SCALE` (`--height-scale h`), else the plan scale when the
/// approved `ground-plan-terrain-heights` deviation says heights follow the
/// plan (so `--ground-scale 1.0` is the surveyed world, heights 1.0, and the
/// default 1.25 plan has heights 1.25: every grade the surveyed one).
pub fn resolve_height() -> Result<f64> {
    if let Some((h, _)) = HEIGHT.get() {
        return Ok(*h);
    }
    let (h, from) = match std::env::var("EO_HEIGHT_SCALE") {
        Ok(v) if !v.trim().is_empty() => (parse_scale(&v).context("EO_HEIGHT_SCALE")?, "EO_HEIGHT_SCALE / --height-scale"),
        _ => {
            if crate::standards::terrain_heights_follow_plan().context("the terrain-heights deviation (CALIFORNIA_STREET_DEVIATIONS ground-plan-terrain-heights)")? {
                (resolve()?, "the ground plan (approved terrain-heights deviation)")
            } else {
                (1.0, "surveyed")
            }
        }
    };
    Ok(HEIGHT.get_or_init(|| (h, from)).0)
}

/// The ground-height scale `h` about mean sea level (the plan scale by default).
pub fn height_scale() -> f64 {
    resolve_height().unwrap_or_else(|e| panic!("groundscale: {e:#}"))
}

/// Where the height scale came from (for the start-up line).
pub fn height_source() -> &'static str {
    let _ = height_scale();
    HEIGHT.get().map(|x| x.1).unwrap_or("")
}

/// Pin the height scale before anything reads it (the JS-oracle tests pin
/// 1.0 with the plan); panics when the process already uses another value.
pub fn pin_height(h: f64) {
    let (v, _) = *HEIGHT.get_or_init(|| (h, "pinned"));
    assert!(v == h, "groundscale: heights pinned {h}, but this process already uses {v}");
}

/// A surveyed ground height (game y) on this process's heights: the height
/// above mean sea level times `h`; sea level and everything at or below it
/// (the shoreline, the bay floor) stay where they are, so the shore does not
/// move in plan and the water keeps its depth.
pub fn lift(y: f64) -> f64 {
    lift_by(y, height_scale())
}

/// `lift` at an explicit height scale.
pub fn lift_by(y: f64, h: f64) -> f64 {
    let d = y - crate::SEA_LEVEL_Y;
    if h == 1.0 || d <= 0.0 { y } else { crate::SEA_LEVEL_Y + d * h }
}

/// Evidence switch, on by default: `EO_GS_PULL=0` leaves the neighbours of
/// the hand-built buildings plainly scaled (the strip beside each stays
/// open), for the before/after pictures of the fix; never for play.
pub fn pull_enabled() -> bool {
    static S: OnceLock<bool> = OnceLock::new();
    *S.get_or_init(|| std::env::var("EO_GS_PULL").map_or(true, |v| v.trim() != "0"))
}

fn env_f64(name: &str, lo: f64, hi: f64) -> f64 {
    match std::env::var(name).ok().and_then(|v| v.trim().parse::<f64>().ok()) {
        Some(v) if v.is_finite() && (lo..=hi).contains(&v) => v,
        Some(v) => {
            eprintln!("groundscale: {name}={v} outside {lo}..{hi}; ignored (1.0)");
            1.0
        }
        None => 1.0,
    }
}

/// Drawn people scale (visual only; 1.0 unless `EO_PEOPLE_SCALE` is set).
pub fn people() -> f64 {
    static S: OnceLock<f64> = OnceLock::new();
    *S.get_or_init(|| env_f64("EO_PEOPLE_SCALE", 0.2, 3.0))
}

/// Drawn vehicle scale (visual only; 1.0 unless `EO_VEHICLE_SCALE` is set).
pub fn vehicles() -> f64 {
    static S: OnceLock<f64> = OnceLock::new();
    *S.get_or_init(|| env_f64("EO_VEHICLE_SCALE", 0.2, 3.0))
}

static LOW_CAMERA: OnceLock<bool> = OnceLock::new();

/// The lower third-person camera: the default; `EO_LOW_CAMERA=0`
/// (`--default-camera`) gives the camera as it was.
pub fn low_camera() -> bool {
    *LOW_CAMERA.get_or_init(|| match std::env::var("EO_LOW_CAMERA") {
        Ok(v) => !(v.trim().is_empty() || v.trim() == "0" || v.trim().eq_ignore_ascii_case("false")),
        Err(_) => true,
    })
}

/// Pin the camera choice before anything reads it (the JS-oracle tests pin
/// the camera as the JS has it, `false`); panics on a different value in use.
pub fn pin_low_camera(on: bool) {
    let v = *LOW_CAMERA.get_or_init(|| on);
    assert!(v == on, "groundscale: camera pinned {on}, but this process already uses {v}");
}

/// Scale a flat `[x0, z0, x1, z1, ...]` list in place.
pub fn scale_flat(v: &mut [f64]) {
    let s = scale();
    if s != 1.0 {
        for c in v.iter_mut() {
            *c *= s;
        }
    }
}

/// Scale `[x, z]` points in place.
pub fn scale_points(v: &mut [[f64; 2]]) {
    let s = scale();
    if s != 1.0 {
        for p in v.iter_mut() {
            p[0] *= s;
            p[1] *= s;
        }
    }
}

/// Scale `[x, z]` f32 points in place.
pub fn scale_points32(v: &mut [[f32; 2]]) {
    let s = scale();
    if s != 1.0 {
        for p in v.iter_mut() {
            p[0] = (p[0] as f64 * s) as f32;
            p[1] = (p[1] as f64 * s) as f32;
        }
    }
}

/// The JSON keys of a world position or an along-axis distance that `scale_json_keys` scales.
pub const POSITION_KEYS: &[&str] = &["x", "z", "s", "s0", "s1"];

/// Scale the numbers under `keys` in every object of `v` (recursively); the
/// observed-block overlays' entries (`x`, `z`, along-axis `s`, `s0`, `s1`).
pub fn scale_json_keys(v: &mut serde_json::Value, keys: &[&str]) {
    let s = scale();
    if s == 1.0 {
        return;
    }
    match v {
        serde_json::Value::Object(m) => {
            for (k, x) in m.iter_mut() {
                if keys.contains(&k.as_str()) {
                    if let Some(f) = x.as_f64() {
                        *x = serde_json::json!(f * s);
                        continue;
                    }
                }
                scale_json_keys(x, keys);
            }
        }
        serde_json::Value::Array(a) => {
            for x in a.iter_mut() {
                scale_json_keys(x, keys);
            }
        }
        _ => {}
    }
}

/// Scale every number of a JSON value (an `[x, z]` pair or a list of them).
pub fn scale_json_numbers(v: &mut serde_json::Value) {
    let s = scale();
    if s == 1.0 {
        return;
    }
    match v {
        serde_json::Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                *v = serde_json::json!(f * s);
            }
        }
        serde_json::Value::Array(a) => {
            for x in a.iter_mut() {
                scale_json_numbers(x);
            }
        }
        serde_json::Value::Object(m) => {
            for (_, x) in m.iter_mut() {
                scale_json_numbers(x);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// The ground plan of the building records.
// ---------------------------------------------------------------------------

/// Where a hand-built record is anchored when it is moved whole.
#[derive(Clone, Copy, Debug)]
pub enum Anchor {
    /// A Blender building: the midpoint of its record polygon's edge `i`
    /// (`i`, `i + 1`), its authored front.
    FrontEdge(usize),
    /// A record a landmark building claims: the landmark's registry anchor
    /// (the landmark moves by `(s - 1) * anchor`, eo-landmarks `set.rs`).
    Point([f64; 2]),
}

/// The landmark registry (eo-landmarks' export, read here for the claimed records and anchors).
const LANDMARKS_JS: &str = include_str!("../../eo-landmarks/data/landmarks-js.json");
/// The Blender building modules (`eo_assets::manifest::GENERATED_DIR`).
const BLENDER_GENERATED_DIR: &str = "native/assets/blender-buildings";

/// The records moved whole instead of scaled: every Blender-built building
/// (anchored at its authored front edge) and every record a landmark
/// building claims (anchored at the landmark's anchor).
pub fn hand_built_records(repo_root: &Path) -> Result<HashMap<String, Anchor>> {
    let mut m = HashMap::new();
    let dir = repo_root.join(BLENDER_GENERATED_DIR);
    let index = std::fs::read_to_string(dir.join("index.js")).with_context(|| format!("reading {}", dir.join("index.js").display()))?;
    for line in index.lines().map(str::trim).filter(|l| l.starts_with("import ")) {
        let Some(q) = line.find("from '") else { continue };
        let file = line[q + 6..].split('\'').next().unwrap_or_default().trim_start_matches("./");
        let p = dir.join(file);
        let text = std::fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?;
        let start = text.find("Object.freeze(").ok_or_else(|| anyhow!("{}: no Object.freeze(", p.display()))? + "Object.freeze(".len();
        let end = text.rfind(");").ok_or_else(|| anyhow!("{}: no closing );", p.display()))?;
        let v: serde_json::Value = serde_json::from_str(&text[start..end]).with_context(|| format!("decoding {}", p.display()))?;
        let id = v["id"].as_str().ok_or_else(|| anyhow!("{}: no id", p.display()))?;
        let edge = v["frame"]["edgeIndex"].as_u64().ok_or_else(|| anyhow!("{}: no frame.edgeIndex", p.display()))? as usize;
        m.insert(id.trim_start_matches("sf-building-").to_string(), Anchor::FrontEdge(edge));
    }
    if m.is_empty() {
        bail!("{} lists no Blender buildings", dir.join("index.js").display());
    }
    let js: serde_json::Value = serde_json::from_str(LANDMARKS_JS).context("eo-landmarks/data/landmarks-js.json")?;
    if let Some(reg) = js["registry"].as_object() {
        for r in reg.values() {
            let a = [r["anchor"]["x"].as_f64().unwrap_or(0.0), r["anchor"]["z"].as_f64().unwrap_or(0.0)];
            for c in r["claims"].as_array().into_iter().flatten().filter_map(|c| c.as_str()) {
                m.insert(c.trim_start_matches("sf-building-").to_string(), Anchor::Point(a));
            }
        }
    }
    Ok(m)
}

/// What `GroundPlan::build` found (printed at load, asserted by the tests).
#[derive(Clone, Debug, Default)]
pub struct PlanReport {
    /// Records moved whole (Blender buildings and landmark claims present in the city).
    pub moved: usize,
    /// Groups of touching Blender buildings moved by one shift (sizes > 1 only).
    pub clusters: Vec<usize>,
    /// Touching Blender buildings that could not share a shift (back to back):
    /// the seam between them opens.
    pub open_seams: usize,
    /// the seams between back-to-back pairs left apart at the end, and
    /// how many of them an infill record closes (`GroundPlan::seams`).
    pub seams_left_apart: usize,
    pub seams_filled: usize,
    /// the widest seam an infill closes, m.
    pub widest_seam_m: f64,
    /// Facade edges held on their scaled street line (the front edges and the
    /// other street-facing edges of the moved Blender buildings).
    pub street_edges: usize,
    /// The largest distance a held facade edge ends up off its scaled line, m
    /// (a building with street faces on opposite sides cannot keep both: it
    /// stands centred, real size, in its 1.25 x wider lot, each face set back
    /// (s - 1) / 2 x its width from its scaled line).
    pub worst_street_edge_m: f64,
    /// The largest distance an authored front edge ends up off its scaled line, m.
    pub worst_front_m: f64,
    /// The groups with a facade edge more than 0.5 m off its scaled line: (first record, members, metres off).
    pub off_line: Vec<(String, usize, f64)>,
    /// Vertices of other records pulled toward a moved building (weight > 0).
    pub pulled_vertices: usize,
    /// Records with at least one pulled vertex.
    pub pulled_records: usize,
    /// The largest move of a pulled vertex away from its plain scaled place, m.
    pub worst_pull_m: f64,
}

/// The ground-plan transform of the building records at scale `s`:
///
/// * a moved record (`hand_built_records`) is translated, real size, by the
///   least-squares shift that keeps its street-facing facade edges on their
///   scaled street lines (the authored front edge weighted `FRONT_WEIGHT`,
///   any other edge that sees a vehicle way within 30 m weighted 1). A single
///   building on one street lands with its front edge's midpoint on its
///   scaled place; a corner building keeps both street faces. Touching
///   Blender buildings move as one group (their party walls stay shut) when
///   one shift keeps every facade of the group within `MERGE_TOLERANCE_M`
///   of its line: a row along a street, a corner; buildings back to back
///   (fronts on opposite streets) cannot, and the seam between their rear
///   walls opens into the block. A building with street faces on opposite
///   sides keeps its front on its line and stands centred between the others.
/// * every other record is scaled, except that a vertex within `PULL_FULL_M`
///   of a moved record (or inside it) moves with that record, fading to the
///   plain scaled place at `PULL_FADE_M`. A neighbour's party wall therefore
///   meets the hand-built building's real-size wall: the leftover strip of the
///   hand-built building's scaled lot (12.5 % of its width each side and 25 %
///   of its depth behind at 1.25) is taken up by the kit buildings around it
///   instead of showing as a gap with blank side walls.
pub struct GroundPlan {
    pub s: f64,
    /// Translation of every moved record, world metres.
    pub shifts: HashMap<String, [f64; 2]>,
    /// The moved records' polygons (unscaled), their shift, for the pull.
    polys: Vec<(Vec<[f64; 2]>, [f64; 2], [f64; 4])>,
    grid: HashMap<(i64, i64), Vec<u32>>,
    pub report: PlanReport,
    /// an infill footprint for each seam that opens between two
    /// back-to-back hand-built buildings (their rear walls swept apart by the
    /// difference of their shifts): (the two records, the infill ring on the
    /// scaled plan). `City::load_at` adds each as a record (no hand-built
    /// building is stretched; the gap behind them is built up instead).
    pub seams: Vec<(String, String, Vec<[f64; 2]>)>,
}

/// a seam narrower than this is left open (a kit building needs room).
pub const SEAM_FILL_MIN_M: f64 = 1.5;

fn hull(mut p: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    p.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    p.dedup_by(|a, b| (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-6);
    if p.len() < 3 {
        return p;
    }
    let cross = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let mut lower: Vec<[f64; 2]> = vec![];
    for &q in &p {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], q) <= 0.0 {
            lower.pop();
        }
        lower.push(q);
    }
    let mut upper: Vec<[f64; 2]> = vec![];
    for &q in p.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], q) <= 0.0 {
            upper.pop();
        }
        upper.push(q);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// A vertex this close to a moved record (or inside it) moves with it.
pub const PULL_FULL_M: f64 = 1.0;
/// ... fading to the plain scaled place at this distance.
pub const PULL_FADE_M: f64 = 5.5;
/// Blender buildings this close to each other move as one, when they can (`MERGE_TOLERANCE_M`).
pub const CLUSTER_GAP_M: f64 = 0.6;
/// ... that is, when one shift keeps every facade of the group this close to its scaled street line.
pub const MERGE_TOLERANCE_M: f64 = 0.3;
/// The authored front edge outweighs another street face this many times
/// (per metre): the front holds its line, a side face decides only what the
/// front leaves free (the position along the street, at a corner).
pub const FRONT_WEIGHT: f64 = 50.0;
/// How far in front of a facade edge a vehicle way may run for the edge to be held on its street line.
pub const STREET_PROBE_M: f64 = 30.0;
const GRID_M: f64 = 32.0;

fn polygon(points: &[f64]) -> Vec<[f64; 2]> {
    let mut p: Vec<[f64; 2]> = points.chunks_exact(2).map(|c| [c[0], c[1]]).collect();
    if p.len() > 1 && p.first() == p.last() {
        p.pop();
    }
    p
}

fn bbox(p: &[[f64; 2]]) -> [f64; 4] {
    p.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, q| [b[0].min(q[0]), b[1].min(q[1]), b[2].max(q[0]), b[3].max(q[1])])
}

fn seg_dist(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dz * dz;
    let t = if l2 > 1e-12 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / l2).clamp(0.0, 1.0) } else { 0.0 };
    (a[0] + dx * t - p[0]).hypot(a[1] + dz * t - p[1])
}

fn inside(p: [f64; 2], poly: &[[f64; 2]]) -> bool {
    let mut c = false;
    let n = poly.len();
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
            c = !c;
        }
        j = i;
    }
    c
}

/// Distance from `p` to the polygon (0 inside).
fn poly_dist(p: [f64; 2], poly: &[[f64; 2]]) -> f64 {
    if poly.len() >= 3 && inside(p, poly) {
        return 0.0;
    }
    let n = poly.len();
    (0..n).map(|i| seg_dist(p, poly[i], poly[(i + 1) % n])).fold(f64::MAX, f64::min)
}

fn polys_touch(a: &[[f64; 2]], b: &[[f64; 2]], tol: f64) -> bool {
    a.iter().any(|&p| poly_dist(p, b) <= tol) || b.iter().any(|&p| poly_dist(p, a) <= tol)
}

fn cells(b: [f64; 4], pad: f64) -> impl Iterator<Item = (i64, i64)> {
    let (x0, z0, x1, z1) = (((b[0] - pad) / GRID_M).floor() as i64, ((b[1] - pad) / GRID_M).floor() as i64, ((b[2] + pad) / GRID_M).floor() as i64, ((b[3] + pad) / GRID_M).floor() as i64);
    (x0..=x1).flat_map(move |x| (z0..=z1).map(move |z| (x, z)))
}

/// One facade-line constraint: unit normal, the scaled line's required shift `(s - 1) * point`, weight.
struct Hold {
    n: [f64; 2],
    t: [f64; 2],
    w: f64,
    /// The authored front edge (else another street face).
    front: bool,
}

impl GroundPlan {
    /// Build the transform. `records`: every building record (key, unscaled
    /// flat points as the file lists them); `ways`: the unscaled polylines of
    /// the vehicle ways (the street a facade may face).
    pub fn build(s: f64, moved: &HashMap<String, Anchor>, records: &[(&str, &[f64])], ways: &[Vec<[f64; 2]>]) -> Self {
        let mut report = PlanReport::default();
        // The moved records present in the city.
        let present: Vec<(String, Vec<[f64; 2]>, Anchor)> = records
            .iter()
            .filter_map(|(k, pts)| moved.get(*k).map(|a| (k.to_string(), polygon(pts), *a)))
            .filter(|(_, p, _)| p.len() >= 3)
            .collect();
        report.moved = present.len();
        // Way segments on a grid, for the street-facing probe.
        let mut wgrid: HashMap<(i64, i64), Vec<([f64; 2], [f64; 2])>> = HashMap::new();
        for w in ways {
            for seg in w.windows(2) {
                let b = bbox(seg);
                for c in cells(b, 0.0) {
                    wgrid.entry(c).or_default().push((seg[0], seg[1]));
                }
            }
        }
        // Every record on a grid, for the street-facing probe's line of sight.
        let all: Vec<Vec<[f64; 2]>> = records.iter().map(|(_, p)| polygon(p)).collect();
        let all_boxes: Vec<[f64; 4]> = all.iter().map(|p| bbox(p)).collect();
        let mut rgrid: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
        for (i, b) in all_boxes.iter().enumerate() {
            if all[i].len() >= 3 {
                for c in cells(*b, 0.0) {
                    rgrid.entry(c).or_default().push(i as u32);
                }
            }
        }
        let covered = |p: [f64; 2]| -> bool {
            let cell = ((p[0] / GRID_M).floor() as i64, (p[1] / GRID_M).floor() as i64);
            rgrid.get(&cell).into_iter().flatten().any(|&i| {
                let b = all_boxes[i as usize];
                p[0] >= b[0] && p[0] <= b[2] && p[1] >= b[1] && p[1] <= b[3] && inside(p, &all[i as usize])
            })
        };
        // Street-facing edges of a Blender building and its front.
        // A back face is never held (it would pull the front off its line by
        // (s - 1) x the depth); a side face is, on a corner.
        let holds_of = |poly: &[[f64; 2]], front: usize| -> Vec<Hold> {
            let m = poly.len();
            let area2: f64 = (0..m).map(|i| poly[i][0] * poly[(i + 1) % m][1] - poly[(i + 1) % m][0] * poly[i][1]).sum();
            let orient = if area2 >= 0.0 { 1.0 } else { -1.0 };
            let mut out = vec![];
            let edge_normal = |i: usize| -> Option<[f64; 2]> {
                let (a, b) = (poly[i], poly[(i + 1) % m]);
                let l = (b[0] - a[0]).hypot(b[1] - a[1]);
                (l >= 1e-6).then(|| [(b[1] - a[1]) / l * orient, -(b[0] - a[0]) / l * orient])
            };
            let front_n = edge_normal(front % m).unwrap_or([0.0, 0.0]);
            for i in 0..m {
                let (a, b) = (poly[i], poly[(i + 1) % m]);
                let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
                let l = dx.hypot(dz);
                if l < 1e-6 {
                    continue;
                }
                let (ux, uz) = (dx / l, dz / l);
                // Outward normal: right of the edge for a counter-clockwise ring (x east, z south).
                let o = [uz * orient, -ux * orient];
                let c = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
                let is_front = i == front % m;
                let mut faces = false;
                if !is_front && l >= 3.0 && o[0] * front_n[0] + o[1] * front_n[1] > -0.5 {
                    let probe = [c[0] + o[0] * STREET_PROBE_M, c[1] + o[1] * STREET_PROBE_M];
                    let mut hit = f64::MAX;
                    for cell in cells(bbox(&[c, probe]), 0.0) {
                        for (p, q) in wgrid.get(&cell).into_iter().flatten() {
                            let (sx, sz) = (q[0] - p[0], q[1] - p[1]);
                            let sl = sx.hypot(sz);
                            if sl < 1e-6 || ((sx * ux + sz * uz) / sl).abs() < 0.85 {
                                continue;
                            }
                            // Where the normal ray from the edge's middle meets the way.
                            let den = o[0] * sz - o[1] * sx;
                            if den.abs() < 1e-9 {
                                continue;
                            }
                            let t = ((p[0] - c[0]) * sz - (p[1] - c[1]) * sx) / den;
                            let u = ((p[0] - c[0]) * o[1] - (p[1] - c[1]) * o[0]) / den;
                            if t > 0.5 && t <= STREET_PROBE_M && (-0.05..=1.05).contains(&u) {
                                hit = hit.min(t);
                            }
                        }
                    }
                    // A street face sees the way: no building (another record, or this
                    // one) stands on the ray from the wall to it (an inner party wall
                    // facing the street through its neighbour is not one).
                    if hit < f64::MAX {
                        let mut d = 1.0;
                        faces = true;
                        while d < hit - 1.0 {
                            if covered([c[0] + o[0] * d, c[1] + o[1] * d]) {
                                faces = false;
                                break;
                            }
                            d += 2.0;
                        }
                    }
                }
                if is_front || faces {
                    out.push(Hold { n: o, t: [(s - 1.0) * c[0], (s - 1.0) * c[1]], w: if is_front { FRONT_WEIGHT * l.max(1.0) } else { l }, front: is_front });
                }
            }
            out
        };
        // Each Blender building's holds; a landmark claim's fixed shift.
        let n = present.len();
        let boxes: Vec<[f64; 4]> = present.iter().map(|(_, p, _)| bbox(p)).collect();
        let mut holds: Vec<Vec<Hold>> = Vec::with_capacity(n);
        let mut fronts: Vec<Option<[f64; 2]>> = Vec::with_capacity(n);
        for (_, poly, a) in &present {
            match *a {
                Anchor::Point(_) => {
                    holds.push(vec![]);
                    fronts.push(None);
                }
                Anchor::FrontEdge(e) => {
                    let m = poly.len();
                    let (pa, pb) = (poly[e % m], poly[(e + 1) % m]);
                    fronts.push(Some([(s - 1.0) * (pa[0] + pb[0]) / 2.0, (s - 1.0) * (pa[1] + pb[1]) / 2.0]));
                    holds.push(holds_of(poly, e));
                }
            }
        }
        // Least squares over a group's holds: sum w (n.(T - t))^2 + eps |T - each
        // front t|^2 (the eps term only decides what the holds leave free: the
        // position along a single street). Returns the shift, its worst hold and
        // its worst front.
        let solve = |members: &[usize]| -> ([f64; 2], f64, f64) {
            let hs: Vec<&Hold> = members.iter().flat_map(|&i| holds[i].iter()).collect();
            let fs: Vec<[f64; 2]> = members.iter().filter_map(|&i| fronts[i]).collect();
            let wsum: f64 = hs.iter().map(|h| h.w).sum::<f64>().max(1e-9);
            let eps = 1e-5 * wsum / fs.len().max(1) as f64;
            let mut a = [[0.0f64; 2]; 2];
            let mut b = [0.0f64; 2];
            for h in &hs {
                let d = h.n[0] * h.t[0] + h.n[1] * h.t[1];
                for r in 0..2 {
                    for c in 0..2 {
                        a[r][c] += h.w * h.n[r] * h.n[c];
                    }
                    b[r] += h.w * h.n[r] * d;
                }
            }
            for f in &fs {
                a[0][0] += eps;
                a[1][1] += eps;
                b[0] += eps * f[0];
                b[1] += eps * f[1];
            }
            let det = a[0][0] * a[1][1] - a[0][1] * a[1][0];
            let t = if det.abs() < 1e-12 {
                let k = fs.len().max(1) as f64;
                fs.iter().fold([0.0, 0.0], |acc, f| [acc[0] + f[0] / k, acc[1] + f[1] / k])
            } else {
                [(b[0] * a[1][1] - b[1] * a[0][1]) / det, (a[0][0] * b[1] - a[1][0] * b[0]) / det]
            };
            let off = |h: &&Hold| (h.n[0] * (t[0] - h.t[0]) + h.n[1] * (t[1] - h.t[1])).abs();
            let worst = hs.iter().map(off).fold(0.0, f64::max);
            let worst_front = hs.iter().filter(|h| h.front).map(off).fold(0.0, f64::max);
            (t, worst, worst_front)
        };
        // Touching Blender buildings move as one group when one shift still holds
        // every facade of the group within MERGE_TOLERANCE_M of its scaled street
        // line (a row along one street, a corner); back-to-back buildings (their
        // fronts on opposite streets) cannot, and stay apart: the seam between
        // their rear walls opens into the block's interior.
        let mut group: Vec<usize> = (0..n).collect();
        let mut members: Vec<Vec<usize>> = (0..n).map(|i| vec![i]).collect();
        let mut apart: Vec<(usize, usize)> = vec![]; // hills
        for i in 0..n {
            for j in i + 1..n {
                if fronts[i].is_none() || fronts[j].is_none() || group[i] == group[j] {
                    continue;
                }
                let (a, b) = (boxes[i], boxes[j]);
                if a[0] > b[2] + CLUSTER_GAP_M || b[0] > a[2] + CLUSTER_GAP_M || a[1] > b[3] + CLUSTER_GAP_M || b[1] > a[3] + CLUSTER_GAP_M {
                    continue;
                }
                if !polys_touch(&present[i].1, &present[j].1, CLUSTER_GAP_M) {
                    continue;
                }
                let (gi, gj) = (group[i], group[j]);
                let union: Vec<usize> = members[gi].iter().chain(members[gj].iter()).copied().collect();
                if solve(&union).1 <= MERGE_TOLERANCE_M {
                    let moved: Vec<usize> = std::mem::take(&mut members[gj]);
                    for &k in &moved {
                        group[k] = gi;
                    }
                    members[gi].extend(moved);
                } else {
                    report.open_seams += 1;
                    apart.push((i, j));
                }
            }
        }
        let mut shifts = HashMap::new();
        let mut polys = vec![];
        for g in members.iter().filter(|g| !g.is_empty()) {
            let shift = match present[g[0]].2 {
                Anchor::Point(p) => [(s - 1.0) * p[0], (s - 1.0) * p[1]],
                Anchor::FrontEdge(_) => {
                    let (t, worst, worst_front) = solve(g);
                    report.worst_street_edge_m = report.worst_street_edge_m.max(worst);
                    report.worst_front_m = report.worst_front_m.max(worst_front);
                    if worst > 0.5 {
                        report.off_line.push((present[g[0]].0.clone(), g.len(), worst));
                    }
                    report.street_edges += g.iter().map(|&i| holds[i].len()).sum::<usize>();
                    t
                }
            };
            if g.len() > 1 {
                report.clusters.push(g.len());
            }
            for &i in g {
                let (k, poly, _) = &present[i];
                shifts.insert(k.clone(), shift);
                polys.push((poly.clone(), shift, bbox(poly)));
            }
        }
        report.clusters.sort_unstable_by(|a, b| b.cmp(a));
        report.off_line.sort_by(|a, b| b.2.total_cmp(&a.2));
        let mut grid: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
        for (i, (_, _, b)) in polys.iter().enumerate() {
            for c in cells(*b, PULL_FADE_M) {
                grid.entry(c).or_default().push(i as u32);
            }
        }
        // the seams that stay open, each closed by an infill footprint:
        // the touching stretch of the two rear walls swept from one building's
        // shift to the other's.
        let mut seams = vec![];
        for &(i, j) in &apart {
            if group[i] == group[j] {
                continue;
            }
            report.seams_left_apart += 1;
            let (ki, pi, _) = &present[i];
            let (kj, pj, _) = &present[j];
            let (ti, tj) = (shifts[ki], shifts[kj]);
            let touch: Vec<[f64; 2]> = pi.iter().filter(|&&p| poly_dist(p, pj) <= CLUSTER_GAP_M).chain(pj.iter().filter(|&&p| poly_dist(p, pi) <= CLUSTER_GAP_M)).copied().collect();
            let span = touch.iter().flat_map(|a| touch.iter().map(move |b| (a[0] - b[0]).hypot(a[1] - b[1]))).fold(0.0, f64::max);
            let width = (ti[0] - tj[0]).hypot(ti[1] - tj[1]);
            if touch.len() < 2 || span < 1.0 || width < SEAM_FILL_MIN_M {
                continue;
            }
            let ring = hull(touch.iter().flat_map(|p| [[p[0] + ti[0], p[1] + ti[1]], [p[0] + tj[0], p[1] + tj[1]]]).collect());
            if ring.len() < 3 {
                continue;
            }
            report.seams_filled += 1;
            report.widest_seam_m = report.widest_seam_m.max(width);
            seams.push((ki.clone(), kj.clone(), ring));
        }
        GroundPlan { s, shifts, polys, grid, report, seams }
    }

    /// The record's flat points on the scaled plan: moved whole when it is a
    /// moved record, else scaled with the pull toward a nearby moved record.
    pub fn place(&mut self, key: &str, points: &mut [f64]) {
        let s = self.s;
        if let Some(d) = self.shifts.get(key) {
            for c in points.chunks_exact_mut(2) {
                c[0] += d[0];
                c[1] += d[1];
            }
            return;
        }
        let mut pulled = false;
        let pull = pull_enabled();
        for c in points.chunks_exact_mut(2) {
            let v = [c[0], c[1]];
            let cell = ((v[0] / GRID_M).floor() as i64, (v[1] / GRID_M).floor() as i64);
            let mut best: Option<(f64, [f64; 2])> = None;
            for &i in self.grid.get(&cell).into_iter().flatten() {
                let (poly, shift, b) = &self.polys[i as usize];
                if v[0] < b[0] - PULL_FADE_M || v[0] > b[2] + PULL_FADE_M || v[1] < b[1] - PULL_FADE_M || v[1] > b[3] + PULL_FADE_M {
                    continue;
                }
                let d = poly_dist(v, poly);
                if d < PULL_FADE_M && best.is_none_or(|x| d < x.0) {
                    best = Some((d, *shift));
                }
            }
            let mut out = [v[0] * s, v[1] * s];
            if let Some((d, t)) = best.filter(|_| pull) {
                let w = if d <= PULL_FULL_M {
                    1.0
                } else {
                    let x = 1.0 - (d - PULL_FULL_M) / (PULL_FADE_M - PULL_FULL_M);
                    x * x * (3.0 - 2.0 * x)
                };
                if w > 0.0 {
                    let pull = [w * (t[0] - (s - 1.0) * v[0]), w * (t[1] - (s - 1.0) * v[1])];
                    out = [out[0] + pull[0], out[1] + pull[1]];
                    self.report.pulled_vertices += 1;
                    self.report.worst_pull_m = self.report.worst_pull_m.max(pull[0].hypot(pull[1]));
                    pulled = true;
                }
            }
            c[0] = out[0];
            c[1] = out[1];
        }
        if pulled {
            self.report.pulled_records += 1;
        }
    }
}

static SHIFTS: OnceLock<HashMap<String, [f64; 2]>> = OnceLock::new();

/// Remember the moved records' shifts (`City::load`, first load wins; every
/// load of the same process computes the same map).
pub fn set_record_shifts(m: HashMap<String, [f64; 2]>) {
    let _ = SHIFTS.set(m);
}

/// How far a record was moved whole (a Blender building or a landmark claim),
/// once the city is loaded; None for a scaled record or before the load.
pub fn record_shift(key: &str) -> Option<[f64; 2]> {
    SHIFTS.get()?.get(key).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f64, z0: f64, x1: f64, z1: f64) -> Vec<f64> {
        vec![x0, z0, x1, z0, x1, z1, x0, z1, x0, z0]
    }

    #[test]
    fn a_neighbour_closes_the_strip_beside_a_moved_building() {
        // A row on a street along z = 0 (the way at z = -10): kit, hand-built, kit, each 10 m wide, 20 m deep.
        let s = 1.25;
        let recs = [rect(90.0, 0.0, 100.0, 20.0), rect(100.0, 0.0, 110.0, 20.0), rect(110.0, 0.0, 120.0, 20.0)];
        let keys = ["w", "b", "e"];
        let records: Vec<(&str, &[f64])> = keys.iter().zip(recs.iter()).map(|(k, r)| (*k, r.as_slice())).collect();
        let moved: HashMap<String, Anchor> = [("b".to_string(), Anchor::FrontEdge(0))].into_iter().collect();
        let ways = vec![vec![[0.0, -10.0], [300.0, -10.0]]];
        let mut gp = GroundPlan::build(s, &moved, &records, &ways);
        let mut placed: Vec<Vec<f64>> = recs.to_vec();
        for (k, p) in keys.iter().zip(placed.iter_mut()) {
            gp.place(k, p);
        }
        // The hand-built one: real size, its front edge's midpoint (105, 0) at its scaled place (131.25, 0).
        let b = &placed[1];
        assert!((b[0] - 126.25).abs() < 1e-9 && (b[2] - 136.25).abs() < 1e-9 && b[1].abs() < 1e-9, "{b:?}");
        // West neighbour's east wall meets its west wall; its far wall stays at the scaled place.
        let w = &placed[0];
        assert!((w[2] - 126.25).abs() < 1e-9 && (w[0] - 112.5).abs() < 1e-9, "{w:?}");
        // East neighbour's west wall meets its east wall.
        let e = &placed[2];
        assert!((e[0] - 136.25).abs() < 1e-9 && (e[2] - 150.0).abs() < 1e-9, "{e:?}");
        assert_eq!(gp.report.moved, 1);
        assert!(gp.report.worst_street_edge_m < 1e-9);
    }

    #[test]
    fn touching_hand_built_buildings_move_together_and_a_corner_keeps_both_faces() {
        let s = 1.25;
        // Two touching hand-built buildings at a corner: streets along z = 0 (way at z = -10) and x = 0 (way at x = -10).
        let a = rect(0.0, 0.0, 10.0, 20.0);
        let b = rect(10.0, 0.0, 20.0, 20.0);
        let records: Vec<(&str, &[f64])> = vec![("a", a.as_slice()), ("b", b.as_slice())];
        let moved: HashMap<String, Anchor> = [("a".to_string(), Anchor::FrontEdge(0)), ("b".to_string(), Anchor::FrontEdge(0))].into_iter().collect();
        let ways = vec![vec![[-50.0, -10.0], [300.0, -10.0]], vec![[-10.0, -50.0], [-10.0, 300.0]]];
        let gp = GroundPlan::build(s, &moved, &records, &ways);
        assert_eq!(gp.report.clusters, vec![2]);
        let (ta, tb) = (gp.shifts["a"], gp.shifts["b"]);
        assert_eq!(ta, tb, "one shift for the cluster");
        // The corner (0, 0) stays on both scaled street lines: the shift is (s - 1) * corner = 0 in both axes.
        assert!(ta[1].abs() < 0.05 && ta[0].abs() < 0.05, "{ta:?}");
    }
}
