//! The named downtown San Francisco skyline towers drawn as authored
//! landmarks in place of the generic kit facade, and the authored facade
//! styles the landmark buildings' surface kind 7 draws.
//!
//! Every value comes from `data/skyline-buildings.json`, each entry with its
//! published source (heights, floors, materials, colours) or marked INFERRED.
//! A tower's plan is its own footprint record in public/data/sf/sf-city.json
//! (`detailBuildings`, by `record`): the generator builds multi-tier setbacks
//! (clipped, chamfered, cylindrical, corner-notched, bay-faceted, helical
//! twisted, H-plan or wedge profiles), recessed mechanical floors, exterior
//! structural ribs, sunshades and diagonal X-braces, multi-tower podium
//! complexes (`subTowers`), and distinctive architectural crowns (Art Deco
//! stepped wedding-cakes, cylindrical crowns, sloped/shed crowns, cantilevered
//! canopies/sails, illuminated louvered mechanical screens, octagonal spires,
//! hipped chateau roofs, twin 45-degree spires and masts) preserved in both
//! `Role::Near` and `Role::Far` meshes.
//!
//! Like the registry's landmark buildings each tower is built at its real site
//! and size on the ground of its scaled site, then moved whole
//! (`set::ground_scale_shift`). The kit and far-boxes draw none of these
//! records while the skyline is on (`set::claimed_records_with`);
//! `eo-app --no-skyline` draws them as before.

use crate::LandmarkLight;
use crate::data::Material;
use crate::geometry::{MeshArrays, Rgb, V3, orient_ring, ring_centroid, srgb_hex};
use crate::set::{Draw, Landmark, Role};
use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
use std::f64::consts::PI;
use std::sync::OnceLock;

pub const SKYLINE_JSON: &str = include_str!("../data/skyline-buildings.json");

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FacadeStyle {
    pub module_m: f64,
    pub storey_m: f64,
    pub glaze_x: f64,
    pub glaze_y: f64,
    pub glass_srgb: String,
    pub glass_roughness: f64,
    pub pattern: String,
    pub night_lit: f64,
    pub led: f64,
    /// How far the frame stands proud of the glass (m).
    #[serde(default)]
    pub fin_depth_m: f64,
    /// v3: the glazing's visible reflectance at normal incidence (0: the
    /// previous coated-mirror tint), the room seen through it by day, each
    /// pane's set-out tilt (degrees), the night lighting's bay (modules) and
    /// a lit window's luminance (cd/m2).
    #[serde(default)]
    pub reflectance: f64,
    #[serde(default)]
    pub body_srgb: Option<String>,
    #[serde(default)]
    pub jitter_deg: f64,
    #[serde(default)]
    pub bay_modules: f64,
    #[serde(default)]
    pub window_cd: f64,
    /// v4: the spandrel band's tone (between one floor's vision glass and
    /// the next, under the glass columns) as a share of the frame's (1: the
    /// frame's own), so a curtain wall's floor lines read apart from its fins.
    #[serde(default = "one")]
    pub spandrel_tone: f64,
    /// v4: 1 when the frame's depth shades the glass and the frame's own
    /// reveals from the sun (the fins', sunshades' and reveals' shadow lines).
    #[serde(default)]
    pub reveal_shadow: f64,
    /// v4: the share of each frame pier that is polished (its centre), its
    /// roughness, and the rest's (honed or flamed) tone over the frame's and roughness.
    #[serde(default)]
    pub polish_share: f64,
    #[serde(default)]
    pub polish_roughness: f64,
    #[serde(default = "one")]
    pub honed_tone: f64,
    #[serde(default)]
    pub honed_roughness: f64,
    /// Phase 3: per-facade night window Kelvin color-temperature bias in [-1.0 .. +1.0]
    /// (-1 = cool 5800-6500K tech/glass, 0 = mixed corporate, +1 = warm 2400-2800K residential/hotel).
    #[serde(default)]
    pub night_temp: f64,
    /// Phase 3: special architectural night accent color mode for `pattern = "screen"`
    /// (0 = warm-ivory, 1 = 2500K golden-amber uplight/arcade, 2 = 4800K champagne-white outline strip,
    ///  3 = ruby-red marine beacon, 4 = emerald-green marine beacon, 5 = sapphire-cyan crown wash).
    #[serde(default)]
    pub night_accent: f64,
}

fn one() -> f64 {
    1.0
}

fn default_shape() -> String {
    "plan".into()
}

fn default_chamfer() -> f64 {
    0.18
}

fn default_notch() -> f64 {
    0.16
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Tier {
    /// Top of the tier, a fraction of `heightM`.
    pub to: f64,
    /// The share of the plan's length (along its principal axis) the tier keeps.
    #[serde(default = "one")]
    pub along_frac: f64,
    /// The share of the plan's width (across its principal axis) the tier keeps.
    #[serde(default = "one")]
    pub across_frac: f64,
    /// Centre shift along the principal axis, as a fraction of the plan's half-length.
    #[serde(default)]
    pub along_shift: f64,
    /// Centre shift across the principal axis, as a fraction of the plan's half-width.
    #[serde(default)]
    pub across_shift: f64,
    /// Uniform scale of this tier's ring about its centroid (classic setback).
    #[serde(default = "one")]
    pub scale: f64,
    /// Optional architectural profile shape: "plan" (default), "rect", "chamfer",
    /// "octagon", "cylinder", "circle", "notched", "bay-faceted", "h-plan", "wedge".
    #[serde(default = "default_shape")]
    pub shape: String,
    /// Corner chamfer share for "chamfer" (default 0.18).
    #[serde(default = "default_chamfer")]
    pub chamfer: f64,
    /// Corner notch share for "notched" (default 0.16).
    #[serde(default = "default_notch")]
    pub notch: f64,
    /// Top-to-bottom scale taper of this tier (1.0: straight vertical).
    #[serde(default = "one")]
    pub taper: f64,
    /// Helical rotation from bottom to top of this tier, degrees (e.g. MIRA).
    #[serde(default)]
    pub twist_deg: f64,
    /// Extra plan yaw rotation of this tier relative to the principal axis, degrees.
    #[serde(default)]
    pub yaw_deg: f64,
    /// Optional per-tier facade style override (e.g. stone podium vs glass shaft).
    #[serde(default)]
    pub facade: Option<String>,
    /// Optional per-tier frame/stone sRGB override.
    #[serde(default)]
    pub stone_srgb: Option<String>,
    /// Optional per-tier stone roughness override.
    #[serde(default)]
    pub stone_roughness: Option<f64>,
    /// True to render this tier as a recessed dark louvered mechanical band.
    #[serde(default)]
    pub recess_band: bool,
}

/// Expressed exterior vertical fins, corner columns or precast piers.
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Ribs {
    /// Horizontal pitch between ribs along each face, metres (ignored if `cornersOnly`).
    #[serde(default)]
    pub spacing_m: f64,
    /// Width of each rib parallel to the facade (m).
    #[serde(default = "default_rib_w")]
    pub width_m: f64,
    /// Projection proud of the facade plane (m).
    #[serde(default = "default_rib_d")]
    pub depth_m: f64,
    /// Only place bold piers/blade fins at plan corners (lower poly, strong silhouette).
    #[serde(default)]
    pub corners_only: bool,
    /// Start height fraction (default 0.0).
    #[serde(default)]
    pub from_frac: f64,
    /// End height fraction (default 1.0).
    #[serde(default = "one")]
    pub to_frac: f64,
    /// Optional override colour (defaults to tower `stoneSrgb`).
    #[serde(default)]
    pub srgb: Option<String>,
    /// Render in metallic finish (`metal` / `far-metal`) instead of masonry/precast (`roof`).
    #[serde(default)]
    pub metal: bool,
    /// Preserve bold corner fins in `Role::Far` mesh.
    #[serde(default)]
    pub to_far: bool,
    /// Apply vertical ribs across every non-podium tier overlapping `fromFrac..toFrac`
    /// (e.g. Embarcadero Center's stepped slabs and Art Deco setbacks).
    #[serde(default)]
    pub all_tiers: bool,
}

fn default_rib_w() -> f64 {
    0.85
}
fn default_rib_d() -> f64 {
    0.45
}

/// Expressed horizontal slab bands, sunshades or residential balcony decks.
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Sunshades {
    /// Emit a band every N storeys (default 1.0).
    #[serde(default = "one")]
    pub every_floors: f64,
    /// Projection proud of the facade (m).
    #[serde(default = "default_shade_d")]
    pub depth_m: f64,
    /// Vertical thickness of each band (m).
    #[serde(default = "default_shade_t")]
    pub thick_m: f64,
    /// Start height fraction (default 0.08).
    #[serde(default = "default_shade_from")]
    pub from_frac: f64,
    /// End height fraction (default 0.98).
    #[serde(default = "default_shade_to")]
    pub to_frac: f64,
    /// Optional colour override.
    #[serde(default)]
    pub srgb: Option<String>,
    /// Render in metallic finish.
    #[serde(default)]
    pub metal: bool,
    /// Preserve bold periodic eyebrow bands in `Role::Far` mesh.
    #[serde(default)]
    pub to_far: bool,
}

fn default_shade_d() -> f64 {
    0.45
}
fn default_shade_t() -> f64 {
    0.26
}
fn default_shade_from() -> f64 {
    0.08
}
fn default_shade_to() -> f64 {
    0.98
}

/// A secondary tower standing on the same footprint complex (e.g. One Rincon
/// Hill's North Tower / The Harrison, One Market Plaza's Steuart Tower,
/// LUMINA's Tower B, Hilton Union Square's Tower 2, Rincon Center's second tower).
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SubTower {
    /// Centre shift along the footprint's principal axis (fraction of half-length).
    #[serde(default)]
    pub along_shift: f64,
    /// Centre shift across the footprint's principal axis (fraction of half-width).
    #[serde(default)]
    pub across_shift: f64,
    /// Share of footprint length this sub-tower spans.
    pub along_frac: f64,
    /// Share of footprint width this sub-tower spans.
    pub across_frac: f64,
    /// Base height of the sub-tower above ground (m, e.g. podium roof).
    #[serde(default)]
    pub from_m: f64,
    /// Roof height of the sub-tower above ground (m).
    pub height_m: f64,
    /// Profile shape ("rect", "chamfer", "cylinder", "circle", "notched", "plan").
    #[serde(default = "default_shape")]
    pub shape: String,
    #[serde(default = "default_chamfer")]
    pub chamfer: f64,
    #[serde(default = "default_notch")]
    pub notch: f64,
    #[serde(default)]
    pub yaw_deg: f64,
    #[serde(default)]
    pub facade: Option<String>,
    #[serde(default)]
    pub stone_srgb: Option<String>,
    #[serde(default)]
    pub stone_roughness: Option<f64>,
    #[serde(default)]
    pub tiers: Vec<Tier>,
    #[serde(default)]
    pub crown: Value,
    #[serde(default)]
    pub mech_floors: Vec<f64>,
    #[serde(default)]
    pub ribs: Option<Ribs>,
    #[serde(default)]
    pub sunshades: Option<Sunshades>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub record: String,
    /// Additional overlapping `detailBuildings` record IDs superseded by this
    /// authored complex (so overlapping OSM ways never z-fight through it).
    #[serde(default)]
    pub extra_records: Vec<String>,
    pub height_m: f64,
    pub floors: f64,
    pub facade: String,
    pub stone_srgb: String,
    pub stone_roughness: f64,
    pub roof_srgb: String,
    pub tiers: Vec<Tier>,
    pub crown: Value,
    #[serde(default)]
    pub braces: Option<Value>,
    /// Recessed horizontal mechanical / refuge floor fractions (e.g. `[0.48, 0.94]`).
    #[serde(default)]
    pub mech_floors: Vec<f64>,
    /// Expressed exterior vertical fins or corner piers.
    #[serde(default)]
    pub ribs: Option<Ribs>,
    /// Expressed horizontal slab bands / sunshades / balconies.
    #[serde(default)]
    pub sunshades: Option<Sunshades>,
    /// Secondary towers on the same block footprint.
    #[serde(default)]
    pub sub_towers: Vec<SubTower>,
    /// Whether to add subtle rooftop mechanical equipment (cooling towers, BMU
    /// track/davit, small rooftop vents) on flat/screened roofs (default true).
    #[serde(default = "default_true")]
    pub rooftop_detail: bool,
    /// Optional rusticated stone base plinth height above ground (m) with bevelled water-table cap.
    #[serde(default)]
    pub plinth_m: f64,
    /// Optional projecting classical stone belt course height fraction (`0.1..0.9`).
    #[serde(default)]
    pub belt_frac: f64,
    /// Emit 3D classical window sills, lintel hoods, keystones & pilaster capitals in Near.
    #[serde(default)]
    pub window_trim: bool,
    /// Optional projecting architectural crown cornice overhang depth (m) with shadowed soffit.
    #[serde(default)]
    pub cornice_m: f64,
    /// Optional iconic night architectural outline lighting ("embarcadero", "crown-cyan", "crown-warm").
    #[serde(default)]
    pub accent_edges: Option<String>,
    pub aviation: bool,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Skyline {
    pub version: String,
    pub facades: BTreeMap<String, FacadeStyle>,
    pub floodlight: Value,
    pub buildings: Vec<Entry>,
    /// Authored historic waterfront / Embarcadero pier bulkheads & low-rise landmarks
    /// (`heightM <= 50 m`, kept separate from `buildings` so tower-only tables & tests stay intact).
    #[serde(default)]
    pub waterfront: Vec<Entry>,
    /// v3: the crown's light sculpture (`night`, cd/m2).
    #[serde(default)]
    pub night: Option<Value>,
}

/// Each facade style's and building's `pass` object (`v3`, `v4`) laid over
/// its own keys (that pass's values; the earlier ones stay for the switches
/// that restore them), and the pass's night (`v3Night`, `v4Night`).
fn overlay(v: &mut Value, pass: &str) {
    let lay = |o: &mut Value| {
        if let Some(Value::Object(over)) = o.get(pass).cloned() {
            for (k, x) in over {
                if k != "source" {
                    o[k.as_str()] = x;
                }
            }
        }
    };
    if let Some(Value::Object(f)) = v.get_mut("facades") {
        for (_, style) in f.iter_mut() {
            lay(style);
        }
    }
    if let Some(Value::Array(b)) = v.get_mut("buildings") {
        for e in b.iter_mut() {
            lay(e);
        }
    }
    if let Some(Value::Array(w)) = v.get_mut("waterfront") {
        for e in w.iter_mut() {
            lay(e);
        }
    }
    if let Some(n) = v.get(format!("{pass}Night").as_str()).cloned() {
        v["night"] = n;
    }
}

/// The parsed table (compiled in) at a pass: 2 (as written), 3 (`v3` over
/// it) or 4 (`v4` over that).
pub fn data_at(level: u8) -> &'static Skyline {
    static D: [OnceLock<Skyline>; 3] = [OnceLock::new(), OnceLock::new(), OnceLock::new()];
    let level = level.clamp(2, 4);
    D[(level - 2) as usize].get_or_init(|| {
        let mut v: Value = serde_json::from_str(SKYLINE_JSON).expect("eo-landmarks: data/skyline-buildings.json does not parse");
        if level >= 3 {
            overlay(&mut v, "v3");
        }
        if level >= 4 {
            overlay(&mut v, "v4");
        }
        serde_json::from_value::<Skyline>(v).expect("eo-landmarks: data/skyline-buildings.json does not parse")
    })
}

/// The parsed table with the third pass's values when `v3` (no fourth).
pub fn data_for(v3: bool) -> &'static Skyline {
    data_at(if v3 { 3 } else { 2 })
}

/// The table as the switches set it (`crate::towers_level`).
pub fn data() -> &'static Skyline {
    data_at(crate::towers_level())
}

/// The index of a facade style in the generated WGSL table (`LM_FACADES`, in the table's key order).
pub fn facade_index(name: &str) -> Option<usize> {
    data().facades.keys().position(|k| k == name)
}

/// `#rrggbb` to a linear triple.
pub fn hex_linear(s: &str) -> Rgb {
    srgb_hex(u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap_or(0x808080))
}

/// The albedo and metalness that give a pane of visible reflectance `r` at
/// normal incidence (F0) over a room of diffuse albedo `d` in the city
/// shader's metalness model (diffuse = a (1 - m), F0 = 0.04 (1 - m) + a m):
/// x = 1 - m solves 0.04 x^2 - (r + d) x + d = 0.
pub fn pane_albedo_metal(r: f64, d: f64) -> (f64, f64) {
    let (r, d) = (r.clamp(0.04, 0.9), d.clamp(1e-4, 0.5));
    let b = r + d;
    let x = ((b - (b * b - 0.16 * d).max(0.0).sqrt()) / 0.08).clamp(0.02, 1.0);
    (d / x, 1.0 - x)
}

/// Rows of the WGSL table a style (`LM_FACADE_ROWS`).
pub const FACADE_ROWS: usize = 6;

/// Each facade style as six rows of the WGSL table.
pub fn facade_rows_at(level: u8) -> Vec<[f32; 4 * FACADE_ROWS]> {
    data_at(level)
        .facades
        .values()
        .map(|f| {
            let mut g = hex_linear(&f.glass_srgb);
            let mut metal = 0.85;
            if f.reflectance > 0.0 {
                let body = f.body_srgb.as_deref().map(hex_linear).unwrap_or([0.025; 3]);
                let lum = |c: Rgb| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
                let (a, m) = pane_albedo_metal(f.reflectance, lum(body));
                let l = lum(g).max(1e-4);
                g = g.map(|c| (c / l * a).min(1.0));
                metal = m;
            }
            let pattern = if f.pattern == "screen" { 1.0 } else { 0.0 };
            [
                f.module_m,
                f.storey_m,
                f.glaze_x,
                f.glaze_y,
                g[0],
                g[1],
                g[2],
                f.glass_roughness,
                pattern,
                f.night_lit,
                f.led,
                f.fin_depth_m,
                metal,
                f.jitter_deg.to_radians(),
                f.bay_modules,
                f.window_cd,
                f.spandrel_tone,
                f.reveal_shadow,
                f.polish_share,
                f.polish_roughness,
                f.honed_tone,
                f.honed_roughness,
                f.night_temp,
                f.night_accent,
            ]
            .map(|x| x as f32)
        })
        .collect()
}

/// The first four rows of each style (`facade_rows_at`), at the second pass or the third.
pub fn facade_rows_for(v3: bool) -> Vec<[f32; 16]> {
    facade_rows_at(if v3 { 3 } else { 2 }).iter().map(|r| std::array::from_fn(|k| r[k])).collect()
}

/// `facade_rows_for` as the switches set it (the first four rows).
pub fn facade_rows() -> Vec<[f32; 16]> {
    facade_rows_at(crate::towers_level()).iter().map(|r| std::array::from_fn(|k| r[k])).collect()
}

/// v3: the crown sculpture's colour wash and its figures' peak, cd/m2 (`night`).
pub fn led_luminance() -> [f64; 2] {
    let n = data().night.clone().unwrap_or(Value::Null);
    [n["ledWashCd"].as_f64().unwrap_or(0.0), n["ledFigureCd"].as_f64().unwrap_or(0.0)]
}

/// The floodlights' colour: `floodlight.colourK` as an sRGB tone at unit luminance.
pub fn flood_colour_linear() -> [f64; 3] {
    let c = srgb_hex(0xffc489);
    let l = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    [c[0] / l, c[1] / l, c[2] / l]
}

/// The authored-facade material (surface kind 7) of a facade style, with an
/// optional floodlight (`flood`: illuminance in lux at the foot, the lit
/// band's foot and head in world y).
pub fn facade_material(facade: &str, roughness: f64, metalness: f64, flood: Option<[f64; 3]>) -> Material {
    let mut o = json!({ "facade": facade_index(facade).unwrap_or(0) });
    if let Some(f) = flood {
        o["flood"] = json!(f);
    }
    Material { roughness, metalness, env_class: "facade-masonry".into(), surface: Some("authored".into()), draw: None, wire: false, surface_options: Some(o) }
}

/// `facade_material` with an explicit storey origin (`originY`, world y).
fn facade_material_at(facade: &str, roughness: f64, metalness: f64, origin_y: f64) -> Material {
    let mut m = facade_material(facade, roughness, metalness, None);
    if let Some(o) = m.surface_options.as_mut() {
        o["originY"] = json!(origin_y);
    }
    m
}

/// A plain material (no surface kind), optionally floodlit.
pub fn plain_material(roughness: f64, metalness: f64, env: &str, flood: Option<[f64; 3]>) -> Material {
    Material { roughness, metalness, env_class: env.into(), surface: None, draw: None, wire: false, surface_options: flood.map(|f| json!({ "flood": f })) }
}

/// Weathered landmark roof, cornice & masonry-trim material (`surface: "precast"` with
/// `bayM: 0, storeyM: 0`, signalling `landmarks_surface.wgsl` kind 4u to apply stone/brick
/// micro-grain, rain-drip darkening, and corrugated clay-tile ribbing on warm pitched roofs).
fn roof_trim_material() -> Material {
    Material {
        roughness: 0.9,
        metalness: 0.0,
        env_class: "facade-masonry".into(),
        surface: Some("precast".into()),
        draw: None,
        wire: false,
        surface_options: Some(json!({ "bayM": 0.0, "storeyM": 0.0 })),
    }
}

/// The skyline entries' footprint records (`detailBuildings` of sf-city.json, by id), unscaled.
pub fn load_records(repo_root: &std::path::Path) -> Result<BTreeMap<String, Vec<f64>>> {
    #[derive(Deserialize)]
    struct Rec {
        id: Value,
        #[serde(default)]
        points: Vec<f64>,
    }
    #[derive(Deserialize)]
    struct CityFile {
        #[serde(rename = "detailBuildings")]
        detail_buildings: Vec<Rec>,
    }
    let path = repo_root.join("public/data/sf/sf-city.json.gz");
    let f = std::fs::File::open(&path).with_context(|| format!("eo-landmarks skyline: {}", path.display()))?;
    let city: CityFile = serde_json::from_reader(std::io::BufReader::new(flate2::read::GzDecoder::new(f)))?;
    let d = data();
    let want: HashSet<&str> = d.buildings.iter().chain(d.waterfront.iter()).map(|b| b.record.as_str()).collect();
    let mut out = BTreeMap::new();
    for r in city.detail_buildings {
        let id = match &r.id {
            Value::Number(n) => n.to_string(),
            Value::String(s) => s.clone(),
            _ => continue,
        };
        if want.contains(id.as_str()) {
            out.insert(id, r.points);
        }
    }
    // Also check supplemental DataSF footprints (`ds*` records in sf-footprints.json.gz) if any authored
    // waterfront/skyline entry targets a footprint that exists only in the supplement file.
    if out.len() < want.len() {
        let fp_path = repo_root.join("public/data/sf/sf-footprints.json.gz");
        if let Ok(fp_file) = std::fs::File::open(&fp_path) {
            if let Ok(fp_city) = serde_json::from_reader::<_, CityFile>(std::io::BufReader::new(flate2::read::GzDecoder::new(fp_file))) {
                for r in fp_city.detail_buildings {
                    let id = match &r.id {
                        Value::Number(n) => n.to_string(),
                        Value::String(s) => s.clone(),
                        _ => continue,
                    };
                    if want.contains(id.as_str()) {
                        out.entry(id).or_insert(r.points);
                    }
                }
            }
        }
    }
    Ok(out)
}

/// The records the skyline draws or supersedes (their kit groups and far boxes draw nothing while it is on).
pub fn claims() -> Vec<String> {
    let d = data();
    let mut out = Vec::new();
    for b in d.buildings.iter().chain(d.waterfront.iter()) {
        out.push(b.record.clone());
        for extra in &b.extra_records {
            out.push(extra.clone());
        }
    }
    out
}

// ---- 2D plan geometry helpers ----

fn ring_of(points: &[f64]) -> Vec<[f64; 2]> {
    let mut r: Vec<[f64; 2]> = points.chunks_exact(2).map(|p| [p[0], p[1]]).collect();
    if r.len() > 2 {
        let (a, b) = (r[0], r[r.len() - 1]);
        if (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-4 {
            r.pop();
        }
    }
    orient_ring(&r)
}

/// The plan's principal frame: unit `a`, perpendicular `b = [-a.1, a.0]`, and extents along both.
struct PlanFrame {
    c: [f64; 2],
    a: [f64; 2],
    b: [f64; 2],
    u_lo: f64,
    u_hi: f64,
    v_lo: f64,
    v_hi: f64,
}

impl PlanFrame {
    fn from_ring(ring: &[[f64; 2]]) -> Self {
        let c = ring_centroid(ring);
        let (mut sxx, mut sxz, mut szz) = (0.0, 0.0, 0.0);
        for p in ring {
            let (x, z) = (p[0] - c[0], p[1] - c[1]);
            sxx += x * x;
            sxz += x * z;
            szz += z * z;
        }
        let ang = 0.5 * (2.0 * sxz).atan2(sxx - szz);
        let a = [ang.cos(), ang.sin()];
        let b = [-a[1], a[0]];
        let (mut u_lo, mut u_hi, mut v_lo, mut v_hi) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for p in ring {
            let (dx, dz) = (p[0] - c[0], p[1] - c[1]);
            let u = dx * a[0] + dz * a[1];
            let v = dx * b[0] + dz * b[1];
            u_lo = u_lo.min(u);
            u_hi = u_hi.max(u);
            v_lo = v_lo.min(v);
            v_hi = v_hi.max(v);
        }
        Self { c, a, b, u_lo, u_hi, v_lo, v_hi }
    }

    fn u_mid(&self) -> f64 {
        0.5 * (self.u_lo + self.u_hi)
    }
    fn v_mid(&self) -> f64 {
        0.5 * (self.v_lo + self.v_hi)
    }
    fn half_u(&self) -> f64 {
        0.5 * (self.u_hi - self.u_lo)
    }
    fn half_v(&self) -> f64 {
        0.5 * (self.v_hi - self.v_lo)
    }
    fn yaw(&self) -> f64 {
        self.a[1].atan2(self.a[0])
    }
    fn at(&self, u: f64, v: f64) -> [f64; 2] {
        [self.c[0] + self.a[0] * u + self.b[0] * v, self.c[1] + self.a[1] * u + self.b[1] * v]
    }
}

/// The ring clipped to lo <= dot(p - c, a) <= hi (Sutherland-Hodgman, two half planes).
fn clip_slab(ring: &[[f64; 2]], c: [f64; 2], a: [f64; 2], lo: f64, hi: f64) -> Vec<[f64; 2]> {
    let s = |p: &[f64; 2]| (p[0] - c[0]) * a[0] + (p[1] - c[1]) * a[1];
    let clip = |pts: Vec<[f64; 2]>, keep: &dyn Fn(f64) -> bool, edge: f64| -> Vec<[f64; 2]> {
        let mut out = Vec::new();
        let n = pts.len();
        for i in 0..n {
            let (p, q) = (pts[i], pts[(i + 1) % n]);
            let (sp, sq) = (s(&p), s(&q));
            if keep(sp) {
                out.push(p);
            }
            if keep(sp) != keep(sq) {
                let denom = sq - sp;
                if denom.abs() > 1e-12 {
                    let t = (edge - sp) / denom;
                    out.push([p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t]);
                }
            }
        }
        out
    };
    let r = clip(ring.to_vec(), &|v| v >= lo, lo);
    if r.len() < 3 {
        return r;
    }
    clip(r, &|v| v <= hi, hi)
}

fn scale_about(ring: &[[f64; 2]], c: [f64; 2], k: f64) -> Vec<[f64; 2]> {
    ring.iter().map(|p| [c[0] + (p[0] - c[0]) * k, c[1] + (p[1] - c[1]) * k]).collect()
}

/// Fixed-metre outward normal offset of an `orient_ring`-oriented polygon (works cleanly on
/// both convex and concave rings and narrow long slabs without multiplicative corner ears).
fn offset_ring_out(ring: &[[f64; 2]], d: f64) -> Vec<[f64; 2]> {
    let pts = orient_ring(ring);
    let n = pts.len();
    if n < 3 || d.abs() < 1e-6 {
        return pts;
    }
    let edge_n = |a: [f64; 2], b: [f64; 2]| -> [f64; 2] {
        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
        let l = dx.hypot(dz);
        if l < 1e-9 { [0.0, 0.0] } else { [-dz / l, dx / l] }
    };
    (0..n)
        .map(|i| {
            let prev = pts[(i + n - 1) % n];
            let cur = pts[i];
            let next = pts[(i + 1) % n];
            let n0 = edge_n(prev, cur);
            let n1 = edge_n(cur, next);
            let bx = n0[0] + n1[0];
            let bz = n0[1] + n1[1];
            let bl2 = bx * bx + bz * bz;
            if bl2 < 1e-4 {
                [cur[0] + n0[0] * d, cur[1] + n0[1] * d]
            } else {
                // Exact miter scale `2 / bl2` gives perpendicular distance `d` from both adjacent edges;
                // clamped at 1.5 so acute corners never spike.
                let k = (2.0 / bl2).min(1.5) * d;
                [cur[0] + bx * k, cur[1] + bz * k]
            }
        })
        .collect()
}

/// Anisotropic scale of `ring` in a `PlanFrame` (`ku` along principal axis `a`, `kv` across `b`)
/// around the frame's midpoint, preserving exact vertex order for longitudinal hipped/gabled roofs.
fn scale_aniso(ring: &[[f64; 2]], f: &PlanFrame, ku: f64, kv: f64) -> Vec<[f64; 2]> {
    let um = f.u_mid();
    let vm = f.v_mid();
    ring.iter()
        .map(|p| {
            let (dx, dz) = (p[0] - f.c[0], p[1] - f.c[1]);
            let u = dx * f.a[0] + dz * f.a[1];
            let v = dx * f.b[0] + dz * f.b[1];
            f.at(um + (u - um) * ku, vm + (v - vm) * kv)
        })
        .collect()
}

fn rotate_about(ring: &[[f64; 2]], c: [f64; 2], rad: f64) -> Vec<[f64; 2]> {
    if rad.abs() < 1e-9 {
        return ring.to_vec();
    }
    let (co, si) = (rad.cos(), rad.sin());
    ring.iter()
        .map(|p| {
            let (dx, dz) = (p[0] - c[0], p[1] - c[1]);
            [c[0] + dx * co - dz * si, c[1] + dx * si + dz * co]
        })
        .collect()
}

/// Transform local (u, v) points about `centre` by `yaw` into world [x, z] and orient outward.
fn local_ring(centre: [f64; 2], yaw: f64, pts: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let (co, si) = (yaw.cos(), yaw.sin());
    let r: Vec<[f64; 2]> = pts.iter().map(|[u, v]| [centre[0] + u * co - v * si, centre[1] + u * si + v * co]).collect();
    orient_ring(&r)
}

/// A square of half side h about c, turned by `yaw` (radians from +x toward +z).
fn square(c: [f64; 2], h: f64, yaw: f64) -> Vec<[f64; 2]> {
    local_ring(c, yaw, &[[-h, -h], [h, -h], [h, h], [-h, h]])
}

/// A rectangle of half extents (hu, hv) about c, turned by `yaw`.
fn rect_ring(c: [f64; 2], hu: f64, hv: f64, yaw: f64) -> Vec<[f64; 2]> {
    local_ring(c, yaw, &[[-hu, -hv], [hu, -hv], [hu, hv], [-hu, hv]])
}

/// An octagonal / chamfered rectangle cutting `ch` of min(hu, hv) off each corner.
fn chamfer_ring(c: [f64; 2], hu: f64, hv: f64, ch: f64, yaw: f64) -> Vec<[f64; 2]> {
    let d = (hu.min(hv) * ch.clamp(0.04, 0.45)).max(0.5);
    let cu = (hu - d).max(0.2);
    let cv = (hv - d).max(0.2);
    local_ring(c, yaw, &[[-cu, -hv], [cu, -hv], [hu, -cv], [hu, cv], [cu, hv], [-cu, hv], [-hu, cv], [-hu, -cv]])
}

/// A corner-notched (cruciform) rectangle cutting `nt` off each corner.
fn notched_ring(c: [f64; 2], hu: f64, hv: f64, nt: f64, yaw: f64) -> Vec<[f64; 2]> {
    let nu = (hu * nt.clamp(0.05, 0.35)).max(0.8);
    let nv = (hv * nt.clamp(0.05, 0.35)).max(0.8);
    let (iu, iv) = (hu - nu, hv - nv);
    local_ring(
        c,
        yaw,
        &[
            [-iu, -hv],
            [iu, -hv],
            [iu, -iv],
            [hu, -iv],
            [hu, iv],
            [iu, iv],
            [iu, hv],
            [-iu, hv],
            [-iu, iv],
            [-hu, iv],
            [-hu, -iv],
            [-iu, -iv],
        ],
    )
}

/// An H-shaped historic courtyard plan (open light-court notches on the two long sides).
fn h_plan_ring(c: [f64; 2], hu: f64, hv: f64, yaw: f64) -> Vec<[f64; 2]> {
    let (cw, cd) = (hu * 0.38, hv * 0.42);
    local_ring(
        c,
        yaw,
        &[
            [-hu, -hv],
            [-cw, -hv],
            [-cw, -hv + cd],
            [cw, -hv + cd],
            [cw, -hv],
            [hu, -hv],
            [hu, hv],
            [cw, hv],
            [cw, hv - cd],
            [-cw, hv - cd],
            [-cw, hv],
            [-hu, hv],
        ],
    )
}

/// A bay-faceted / sawtooth perimeter ring (architectural projecting bays on each side).
fn bay_faceted_ring(c: [f64; 2], hu: f64, hv: f64, yaw: f64) -> Vec<[f64; 2]> {
    let mut pts = Vec::new();
    let depth = (hu.min(hv) * 0.055).clamp(0.6, 1.5);
    let nu = ((2.0 * hu) / 7.5).round().clamp(2.0, 6.0) as usize;
    let nv = ((2.0 * hv) / 7.5).round().clamp(2.0, 6.0) as usize;
    let push_edge = |pts: &mut Vec<[f64; 2]>, a: [f64; 2], b: [f64; 2], n_bays: usize, out: [f64; 2]| {
        for k in 0..n_bays {
            let t0 = k as f64 / n_bays as f64;
            let t1 = (k as f64 + 0.28) / n_bays as f64;
            let t2 = (k as f64 + 0.72) / n_bays as f64;
            let lerp = |t: f64, d: f64| [a[0] + (b[0] - a[0]) * t + out[0] * d, a[1] + (b[1] - a[1]) * t + out[1] * d];
            pts.push(lerp(t0, 0.0));
            pts.push(lerp(t1, depth));
            pts.push(lerp(t2, depth));
        }
    };
    push_edge(&mut pts, [-hu, -hv], [hu, -hv], nu, [0.0, -1.0]);
    push_edge(&mut pts, [hu, -hv], [hu, hv], nv, [1.0, 0.0]);
    push_edge(&mut pts, [hu, hv], [-hu, hv], nu, [0.0, 1.0]);
    push_edge(&mut pts, [-hu, hv], [-hu, -hv], nv, [-1.0, 0.0]);
    local_ring(c, yaw, &pts)
}

/// A flatiron / curved-prow wedge ring (broad base narrowing toward a chamfered prow at +u).
fn wedge_ring(c: [f64; 2], hu: f64, hv: f64, yaw: f64) -> Vec<[f64; 2]> {
    local_ring(c, yaw, &[[-hu, -hv], [hu * 0.45, -hv * 0.85], [hu, -hv * 0.32], [hu, hv * 0.32], [hu * 0.45, hv * 0.85], [-hu, hv]])
}

/// An N-sided elliptical or circular ring.
fn ellipse_ring(c: [f64; 2], ru: f64, rv: f64, segs: usize, yaw: f64) -> Vec<[f64; 2]> {
    let pts: Vec<[f64; 2]> = (0..segs)
        .map(|k| {
            let th = (k as f64 / segs as f64) * 2.0 * PI;
            [ru * th.cos(), rv * th.sin()]
        })
        .collect();
    local_ring(c, yaw, &pts)
}

/// Build the 2D ring for a tier (near and far resolutions) on `frame` and `base_ring`.
fn tier_rings(base_ring: &[[f64; 2]], frame: &PlanFrame, t: &Tier) -> (Vec<[f64; 2]>, Vec<[f64; 2]>) {
    let u_center = frame.u_mid() + frame.half_u() * t.along_shift;
    let v_center = frame.v_mid() + frame.half_v() * t.across_shift;
    let hu = (frame.half_u() * t.along_frac).max(2.5);
    let hv = (frame.half_v() * t.across_frac).max(2.5);
    let yaw = frame.yaw() + t.yaw_deg.to_radians();
    let centre = frame.at(u_center, v_center);

    let make = |segs: usize, simplified: bool| -> Vec<[f64; 2]> {
        let mut r = match t.shape.as_str() {
            "rect" => rect_ring(centre, hu * t.scale, hv * t.scale, yaw),
            "chamfer" => chamfer_ring(centre, hu * t.scale, hv * t.scale, t.chamfer, yaw),
            "octagon" => chamfer_ring(centre, hu * t.scale, hv * t.scale, 0.293, yaw),
            "cylinder" => ellipse_ring(centre, hu * t.scale, hv * t.scale, segs, yaw),
            "circle" => {
                let rad = hu.min(hv) * t.scale;
                ellipse_ring(centre, rad, rad, segs, yaw)
            }
            "notched" => notched_ring(centre, hu * t.scale, hv * t.scale, t.notch, yaw),
            "h-plan" => h_plan_ring(centre, hu * t.scale, hv * t.scale, yaw),
            "bay-faceted" => {
                if simplified {
                    chamfer_ring(centre, hu * t.scale, hv * t.scale, 0.1, yaw)
                } else {
                    bay_faceted_ring(centre, hu * t.scale, hv * t.scale, yaw)
                }
            }
            "wedge" => wedge_ring(centre, hu * t.scale, hv * t.scale, yaw),
            _ => {
                // "plan": clip the real footprint ring along both principal axes.
                let mut p = base_ring.to_vec();
                if t.along_frac < 0.999 || t.along_shift.abs() > 1e-4 {
                    p = clip_slab(&p, frame.c, frame.a, u_center - hu, u_center + hu);
                }
                if p.len() >= 3 && (t.across_frac < 0.999 || t.across_shift.abs() > 1e-4) {
                    p = clip_slab(&p, frame.c, frame.b, v_center - hv, v_center + hv);
                }
                if p.len() < 3 {
                    p = rect_ring(centre, hu, hv, frame.yaw());
                }
                let pc = ring_centroid(&p);
                if (t.scale - 1.0).abs() > 1e-4 {
                    p = scale_about(&p, pc, t.scale);
                }
                if t.yaw_deg.abs() > 1e-4 {
                    p = rotate_about(&p, pc, t.yaw_deg.to_radians());
                }
                orient_ring(&p)
            }
        };
        if r.len() < 3 {
            r = rect_ring(centre, hu * t.scale, hv * t.scale, yaw);
        }
        orient_ring(&r)
    };
    (make(24, false), make(14, true))
}

/// Walls of a ring between y0 and y1 (no caps).
fn walls(m: &mut MeshArrays, ring: &[[f64; 2]], y0: f64, y1: f64, colour: Rgb) {
    m.prism(ring, y0, y1, colour, false, false);
}

/// A flat cap over a (possibly concave) ring at y, facing up (earcut).
pub fn cap(m: &mut MeshArrays, ring: &[[f64; 2]], y: f64, colour: Rgb) {
    let pts = orient_ring(ring);
    if pts.len() < 3 {
        return;
    }
    let mut ear = earcut::Earcut::<f64>::new();
    let mut tri: Vec<u32> = Vec::new();
    ear.earcut(pts.iter().copied(), &[] as &[u32], &mut tri);
    for t in tri.chunks_exact(3) {
        let (a, b, c) = (pts[t[0] as usize], pts[t[1] as usize], pts[t[2] as usize]);
        let (p, q, r) = ([a[0], y, a[1]], [b[0], y, b[1]], [c[0], y, c[1]]);
        // Face up whatever the triangulation's winding.
        let ny = (q[2] - p[2]) * (r[0] - p[0]) - (q[0] - p[0]) * (r[2] - p[2]);
        if ny >= 0.0 {
            m.tri(p, q, r, colour);
        } else {
            m.tri(p, r, q, colour);
        }
    }
}

/// A downward-facing soffit cap over a ring at y.
fn soffit_cap(m: &mut MeshArrays, ring: &[[f64; 2]], y: f64, colour: Rgb) {
    let pts = orient_ring(ring);
    if pts.len() < 3 {
        return;
    }
    let mut ear = earcut::Earcut::<f64>::new();
    let mut tri: Vec<u32> = Vec::new();
    ear.earcut(pts.iter().copied(), &[] as &[u32], &mut tri);
    for t in tri.chunks_exact(3) {
        let (a, b, c) = (pts[t[0] as usize], pts[t[1] as usize], pts[t[2] as usize]);
        let (p, q, r) = ([a[0], y, a[1]], [b[0], y, b[1]], [c[0], y, c[1]]);
        let ny = (q[2] - p[2]) * (r[0] - p[0]) - (q[0] - p[0]) * (r[2] - p[2]);
        if ny <= 0.0 {
            m.tri(p, q, r, colour);
        } else {
            m.tri(p, r, q, colour);
        }
    }
}

/// A pyramid over a ring, from y0 to an apex at y1 over the ring's centroid.
fn pyramid(m: &mut MeshArrays, ring: &[[f64; 2]], y0: f64, y1: f64, colour: Rgb) {
    let pts = orient_ring(ring);
    let c = ring_centroid(&pts);
    let apex = [c[0], y1, c[1]];
    let n = pts.len();
    for i in 0..n {
        let (p, q) = (pts[i], pts[(i + 1) % n]);
        m.tri([p[0], y0, p[1]], [q[0], y0, q[1]], apex, colour);
    }
}

/// A sloped-top prism where each ring vertex i has top height `y1_fn(ring[i])`.
fn sloped_prism(m: &mut MeshArrays, ring: &[[f64; 2]], y0: f64, y1_fn: &dyn Fn([f64; 2]) -> f64, wall_c: Rgb, cap_c: Option<Rgb>) {
    let pts = orient_ring(ring);
    let n = pts.len();
    for i in 0..n {
        let (p, q) = (pts[i], pts[(i + 1) % n]);
        let (yp, yq) = (y1_fn(p), y1_fn(q));
        m.quad([p[0], y0, p[1]], [q[0], y0, q[1]], [q[0], yq, q[1]], [p[0], yp, p[1]], wall_c);
    }
    if let Some(cc) = cap_c {
        let mut ear = earcut::Earcut::<f64>::new();
        let mut tri: Vec<u32> = Vec::new();
        ear.earcut(pts.iter().copied(), &[] as &[u32], &mut tri);
        for t in tri.chunks_exact(3) {
            let (a, b, c) = (pts[t[0] as usize], pts[t[1] as usize], pts[t[2] as usize]);
            let (p, q, r) = ([a[0], y1_fn(a), a[1]], [b[0], y1_fn(b), b[1]], [c[0], y1_fn(c), c[1]]);
            let ny = (q[2] - p[2]) * (r[0] - p[0]) - (q[0] - p[0]) * (r[2] - p[2]);
            if ny >= 0.0 {
                m.tri(p, q, r, cc);
            } else {
                m.tri(p, r, q, cc);
            }
        }
    }
}

/// v4: the skyline towers' metal parts: architectural aluminium cladding
/// carries a metallic coating (a fluoropolymer with metal flake: part mirror,
/// part diffuse), drawn at metalness 0.6 and roughness 0.35 (INFERRED).
pub const METAL4_ROUGHNESS: f64 = 0.35;
pub const METAL4_METALNESS: f64 = 0.6;

/// Mesh buckets for a single skyline landmark (near and far).
struct TowerMeshes {
    id: String,
    /// Near facade draws keyed by `(facade_style_name, roughness_millies)` so
    /// podiums, shafts and illuminated crown screens each keep their own style.
    facades_near: BTreeMap<(String, i32), MeshArrays>,
    /// Far facade draws keyed the same way (using exact `stone` vertex colour,
    /// never pre-darkened, so near/far at 1500 m matches seamlessly!).
    facades_far: BTreeMap<(String, i32), MeshArrays>,
    roof_near: MeshArrays,
    roof_far: MeshArrays,
    metal_near: MeshArrays,
    metal_far: MeshArrays,
    /// Dedicated glossy reflective glass panes for compact Beaux-Arts hero windows (Near).
    glass_near: MeshArrays,
}

impl TowerMeshes {
    fn new(id: &str) -> Self {
        Self {
            id: id.into(),
            facades_near: BTreeMap::new(),
            facades_far: BTreeMap::new(),
            roof_near: MeshArrays::new(&format!("{id}:roof")),
            roof_far: MeshArrays::new(&format!("{id}:far-roof")),
            metal_near: MeshArrays::new(&format!("{id}:metal")),
            metal_far: MeshArrays::new(&format!("{id}:far-metal")),
            glass_near: MeshArrays::new(&format!("{id}:glass")),
        }
    }

    fn key(style: &str, rough: f64) -> (String, i32) {
        (style.to_string(), (rough * 1000.0).round() as i32)
    }

    fn facade_near(&mut self, style: &str, rough: f64) -> &mut MeshArrays {
        let k = Self::key(style, rough);
        let id = self.id.clone();
        self.facades_near.entry(k).or_insert_with(|| MeshArrays::new(&format!("{id}:facade:{style}")))
    }

    fn facade_far(&mut self, style: &str, rough: f64) -> &mut MeshArrays {
        let k = Self::key(style, rough);
        let id = self.id.clone();
        self.facades_far.entry(k).or_insert_with(|| MeshArrays::new(&format!("{id}:far:{style}")))
    }
}

/// Emit a vertical shaft span (splitting cleanly around any recessed mechanical
/// floor heights inside `y0..y1`, or lofting/twisting when requested).
#[allow(clippy::too_many_arguments)]
fn emit_shaft_span(
    ms: &mut TowerMeshes,
    near_ring: &[[f64; 2]],
    far_ring: &[[f64; 2]],
    y0: f64,
    y1: f64,
    style: &str,
    rough: f64,
    stone: Rgb,
    roof_c: Rgb,
    taper: f64,
    twist_deg: f64,
    mech_ys: &[f64],
    recess_band: bool,
) {
    if y1 <= y0 + 0.05 {
        return;
    }
    let dark_louver: Rgb = stone.map(|v| (v * 0.16).clamp(0.010, 0.095));
    let lip_c: Rgb = stone.map(|v| (v * 1.04).min(0.88));
    let soffit_c: Rgb = stone.map(|v| v * 0.44);
    if recess_band {
        let nc = ring_centroid(near_ring);
        let fc = ring_centroid(far_ring);
        let nr = scale_about(near_ring, nc, 0.905);
        let fr = scale_about(far_ring, fc, 0.905);
        walls(&mut ms.roof_near, &nr, y0, y1, dark_louver);
        walls(&mut ms.roof_far, &fr, y0, y1, dark_louver);
        // Projecting slab cornice lips framing the deeply recessed mechanical floor (Near & Far, fixed-metre offset).
        let lip_n = offset_ring_out(near_ring, 0.24);
        let lip_f = offset_ring_out(far_ring, 0.28);
        walls(&mut ms.roof_near, &lip_n, y0 - 0.18, y0 + 0.24, lip_c);
        walls(&mut ms.roof_near, &lip_n, y1 - 0.24, y1 + 0.18, lip_c);
        cap(&mut ms.roof_near, &lip_n, y0 + 0.24, roof_c);
        soffit_cap(&mut ms.roof_near, &lip_n, y1 - 0.24, soffit_c);
        walls(&mut ms.roof_far, &lip_f, y0 - 0.22, y0 + 0.28, lip_c);
        walls(&mut ms.roof_far, &lip_f, y1 - 0.28, y1 + 0.22, lip_c);
        cap(&mut ms.roof_far, &lip_f, y0 + 0.28, roof_c);
        return;
    }

    // Twisted / helical tier (e.g., MIRA's rotating white bay-window shaft).
    if twist_deg.abs() > 0.5 {
        let nc = ring_centroid(near_ring);
        let fc = ring_centroid(far_ring);
        let rad_total = twist_deg.to_radians();
        let emit_helical = |dst: &mut MeshArrays, r: &[[f64; 2]], c: [f64; 2], bands: usize| {
            for b in 0..bands {
                let t0 = b as f64 / bands as f64;
                let t1 = (b + 1) as f64 / bands as f64;
                let k0 = 1.0 + (taper - 1.0) * t0;
                let k1 = 1.0 + (taper - 1.0) * t1;
                let r0 = rotate_about(&scale_about(r, c, k0), c, rad_total * t0);
                let r1 = rotate_about(&scale_about(r, c, k1), c, rad_total * t1);
                dst.loft(&r0, y0 + (y1 - y0) * t0, &r1, y0 + (y1 - y0) * t1, stone, false);
            }
        };
        emit_helical(ms.facade_near(style, rough), near_ring, nc, 10);
        emit_helical(ms.facade_far(style, rough), far_ring, fc, 5);
        // Sculpted white corner bay ribs tracing the twist in both Near and Far!
        let n_near = near_ring.len();
        let rib_c = stone.map(|v| (v * 1.04).min(0.88));
        let bands = 10usize;
        for b in 0..bands {
            let t0 = b as f64 / bands as f64;
            let t1 = (b + 1) as f64 / bands as f64;
            let k0 = 1.0 + (taper - 1.0) * t0;
            let k1 = 1.0 + (taper - 1.0) * t1;
            let r0 = rotate_about(&scale_about(near_ring, nc, k0 * 1.018), nc, rad_total * t0);
            let r1 = rotate_about(&scale_about(near_ring, nc, k1 * 1.018), nc, rad_total * t1);
            let ya = y0 + (y1 - y0) * t0;
            let yb = y0 + (y1 - y0) * t1;
            for i in 0..n_near {
                let p0 = [r0[i][0], ya, r0[i][1]];
                let p1 = [r1[i][0], yb, r1[i][1]];
                ms.roof_near.beam(p0, p1, 0.78, 0.78, rib_c);
                if b % 2 == 0 {
                    let p0n = [r0[(i + 1) % n_near][0], ya, r0[(i + 1) % n_near][1]];
                    ms.roof_near.beam(p0, p0n, 0.48, 0.48, rib_c);
                }
            }
        }
        // Simplified 5-band white twisting ribs in Far so MIRA's sculpted bays read at 1.5 km.
        let n_far = far_ring.len();
        let f_step = (n_far / 8).max(1);
        for b in 0..5usize {
            let t0 = b as f64 / 5.0;
            let t1 = (b + 1) as f64 / 5.0;
            let k0 = 1.0 + (taper - 1.0) * t0;
            let k1 = 1.0 + (taper - 1.0) * t1;
            let r0 = rotate_about(&scale_about(far_ring, fc, k0 * 1.022), fc, rad_total * t0);
            let r1 = rotate_about(&scale_about(far_ring, fc, k1 * 1.022), fc, rad_total * t1);
            let ya = y0 + (y1 - y0) * t0;
            let yb = y0 + (y1 - y0) * t1;
            for i in (0..n_far).step_by(f_step) {
                ms.roof_far.beam([r0[i][0], ya, r0[i][1]], [r1[i][0], yb, r1[i][1]], 0.95, 0.95, rib_c);
            }
        }
        return;
    }

    // Tapered lofted tier.
    if (taper - 1.0).abs() > 1e-3 {
        let nc = ring_centroid(near_ring);
        let fc = ring_centroid(far_ring);
        ms.facade_near(style, rough).loft(near_ring, y0, &scale_about(near_ring, nc, taper), y1, stone, false);
        ms.facade_far(style, rough).loft(far_ring, y0, &scale_about(far_ring, fc, taper), y1, stone, false);
        return;
    }

    // Straight vertical prism, splitting cleanly around any mechanical floor bands inside [y0+6, y1-6].
    let mech_h = 4.6;
    let mut cuts: Vec<f64> = mech_ys.iter().copied().filter(|&ym| ym - mech_h * 0.5 > y0 + 5.0 && ym + mech_h * 0.5 < y1 - 5.0).collect();
    cuts.sort_by(f64::total_cmp);
    if cuts.is_empty() {
        walls(ms.facade_near(style, rough), near_ring, y0, y1, stone);
        walls(ms.facade_far(style, rough), far_ring, y0, y1, stone);
        return;
    }
    let nc = ring_centroid(near_ring);
    let fc = ring_centroid(far_ring);
    let inset_near = scale_about(near_ring, nc, 0.905);
    let inset_far = scale_about(far_ring, fc, 0.905);
    let blade_near = scale_about(near_ring, nc, 0.945);
    let lip_near = offset_ring_out(near_ring, 0.24);
    let lip_far = offset_ring_out(far_ring, 0.28);
    let louver_blade_c = stone.map(|v| (v * 0.34).clamp(0.04, 0.22));
    let pier_near = scale_about(near_ring, nc, 0.975);
    let pier_far = scale_about(far_ring, fc, 0.975);
    let mut cur = y0;
    for ym in cuts {
        let ma = ym - mech_h * 0.5;
        let mb = ym + mech_h * 0.5;
        if ma > cur + 0.2 {
            walls(ms.facade_near(style, rough), near_ring, cur, ma, stone);
            walls(ms.facade_far(style, rough), far_ring, cur, ma, stone);
        }
        // Deep dark recessed louvered mechanical core (Near & Far!).
        walls(&mut ms.roof_near, &inset_near, ma, mb, dark_louver);
        walls(&mut ms.roof_far, &inset_far, ma, mb, dark_louver);
        // Projecting upper & lower concrete/stone slab eyebrow cornices framing the mechanical notch!
        walls(&mut ms.roof_near, &lip_near, ma - 0.22, ma + 0.22, lip_c);
        walls(&mut ms.roof_near, &lip_near, mb - 0.22, mb + 0.22, lip_c);
        cap(&mut ms.roof_near, &lip_near, ma + 0.22, stone.map(|v| v * 0.62));
        soffit_cap(&mut ms.roof_near, &lip_near, mb - 0.22, soffit_c);
        walls(&mut ms.roof_far, &lip_far, ma - 0.26, ma + 0.26, lip_c);
        walls(&mut ms.roof_far, &lip_far, mb - 0.26, mb + 0.26, lip_c);
        // Two 3D horizontal dark-metal intake louver fin blades inside the recessed mechanical band (Near).
        for bf in [0.34, 0.68] {
            let by = ma + mech_h * bf;
            walls(&mut ms.metal_near, &blade_near, by - 0.18, by + 0.18, louver_blade_c);
        }
        // Structural corner transfer columns spanning across the recessed mechanical tier (Near & Far).
        let n_step = (pier_near.len() / 8).max(1);
        for i in (0..pier_near.len()).step_by(n_step) {
            let p = pier_near[i];
            let yaw = (p[1] - nc[1]).atan2(p[0] - nc[0]);
            ms.roof_near.box_(p[0], p[1], ma, mb, 0.62, 0.62, yaw, lip_c, crate::geometry::Faces { top: false, bottom: false });
        }
        let f_step = (pier_far.len() / 4).max(1);
        for i in (0..pier_far.len()).step_by(f_step) {
            let p = pier_far[i];
            let yaw = (p[1] - fc[1]).atan2(p[0] - fc[0]);
            ms.roof_far.box_(p[0], p[1], ma, mb, 0.72, 0.72, yaw, lip_c, crate::geometry::Faces { top: false, bottom: false });
        }
        cur = mb;
    }
    if y1 > cur + 0.2 {
        walls(ms.facade_near(style, rough), near_ring, cur, y1, stone);
        walls(ms.facade_far(style, rough), far_ring, cur, y1, stone);
    }
}

/// Emit expressed vertical structural ribs / corner piers along `ring` between `y0` and `y1`.
fn emit_ribs(ms: &mut TowerMeshes, ring: &[[f64; 2]], y0: f64, y1: f64, r: &Ribs, default_stone: Rgb) {
    if y1 <= y0 + 1.0 || ring.len() < 3 {
        return;
    }
    let col = r.srgb.as_deref().map(hex_linear).unwrap_or(default_stone);
    let n = ring.len();
    let c = ring_centroid(ring);
    let push_col = |near: &mut MeshArrays, far: &mut MeshArrays, px: f64, pz: f64, yaw: f64, w: f64, d: f64, to_far: bool| {
        near.box_(px, pz, y0, y1, w * 0.5, d * 0.5, yaw, col, crate::geometry::ALL);
        if to_far {
            far.box_(px, pz, y0, y1, (w * 0.55).max(0.55), (d * 0.55).max(0.45), yaw, col, crate::geometry::NO_BOTTOM);
        }
    };
    let (near, far) = if r.metal { (&mut ms.metal_near, &mut ms.metal_far) } else { (&mut ms.roof_near, &mut ms.roof_far) };
    if r.corners_only || r.spacing_m <= 0.1 {
        for i in 0..n {
            let p = ring[i];
            let (dx, dz) = (p[0] - c[0], p[1] - c[1]);
            let l = dx.hypot(dz).max(1e-6);
            let out = [dx / l, dz / l];
            let px = p[0] + out[0] * (r.depth_m * 0.4);
            let pz = p[1] + out[1] * (r.depth_m * 0.4);
            push_col(near, far, px, pz, out[1].atan2(out[0]), r.depth_m, r.width_m, r.to_far);
        }
        return;
    }
    for i in 0..n {
        let (p, q) = (ring[i], ring[(i + 1) % n]);
        let (dx, dz) = (q[0] - p[0], q[1] - p[1]);
        let len = dx.hypot(dz);
        if len < 4.0 {
            continue;
        }
        let (nx, nz) = (-dz / len, dx / len);
        let yaw = dz.atan2(dx);
        let count = (len / r.spacing_m).round().clamp(1.0, 14.0) as usize;
        let far_stride = (count / 3).max(2);
        for k in 0..=count {
            let t = k as f64 / count as f64;
            let px = p[0] + dx * t + nx * (r.depth_m * 0.42);
            let pz = p[1] + dz * t + nz * (r.depth_m * 0.42);
            let is_corner = k == 0 || k == count;
            let keep_far = r.to_far && (is_corner || (len >= 22.0 && k % far_stride == 0));
            push_col(near, far, px, pz, yaw, r.width_m, r.depth_m, keep_far);
        }
    }
}

/// Emit horizontal slab bands / sunshades / balconies around `ring` in Near (and periodic major bands in Far when `sh.to_far`).
fn emit_sunshades(ms: &mut TowerMeshes, ring: &[[f64; 2]], far_ring: &[[f64; 2]], y0: f64, y1: f64, storey_m: f64, sh: &Sunshades, default_stone: Rgb) {
    if y1 <= y0 + storey_m || ring.len() < 3 {
        return;
    }
    let col = sh.srgb.as_deref().map(hex_linear).unwrap_or(default_stone);
    let pitch = (storey_m * sh.every_floors.max(1.0)).max(3.2);
    let offset_out = |r: &[[f64; 2]], depth: f64| -> Vec<[f64; 2]> {
        let c = ring_centroid(r);
        let n = r.len();
        (0..n)
            .map(|i| {
                let (p, prev, next) = (r[i], r[(i + n - 1) % n], r[(i + 1) % n]);
                let mut nn = [-(next[1] - prev[1]), next[0] - prev[0]];
                let l = nn[0].hypot(nn[1]).max(1e-6);
                nn = [nn[0] / l, nn[1] / l];
                if (p[0] - c[0]) * nn[0] + (p[1] - c[1]) * nn[1] < 0.0 {
                    nn = [-nn[0], -nn[1]];
                }
                [p[0] + nn[0] * depth, p[1] + nn[1] * depth]
            })
            .collect()
    };
    let n = ring.len();
    let outer = offset_out(ring, sh.depth_m);
    let (dst_near, dst_far) = if sh.metal { (&mut ms.metal_near, &mut ms.metal_far) } else { (&mut ms.roof_near, &mut ms.roof_far) };
    let mut y = y0 + pitch;
    while y < y1 - 1.5 {
        let (ya, yb) = (y - sh.thick_m * 0.5, y + sh.thick_m * 0.5);
        for i in 0..n {
            let j = (i + 1) % n;
            let (ia, ib, oa, ob) = (ring[i], ring[j], outer[i], outer[j]);
            // Outer rim, top ledge, underside soffit (keep lean: 3 quads per edge).
            dst_near.quad([oa[0], ya, oa[1]], [ob[0], ya, ob[1]], [ob[0], yb, ob[1]], [oa[0], yb, oa[1]], col);
            dst_near.quad([ia[0], yb, ia[1]], [oa[0], yb, oa[1]], [ob[0], yb, ob[1]], [ib[0], yb, ib[1]], col.map(|v| v * 0.96));
            dst_near.quad([oa[0], ya, oa[1]], [ia[0], ya, ia[1]], [ib[0], ya, ib[1]], [ob[0], ya, ob[1]], col.map(|v| v * 0.65));
        }
        y += pitch;
    }
    if sh.to_far && far_ring.len() >= 3 {
        let nf = far_ring.len();
        let f_outer = offset_out(far_ring, (sh.depth_m * 1.15).max(0.55));
        let f_pitch = (pitch * 2.0).max(11.0);
        let f_thick = (sh.thick_m * 1.45).max(0.42);
        let mut fy = y0 + f_pitch;
        while fy < y1 - 2.0 {
            let (ya, yb) = (fy - f_thick * 0.5, fy + f_thick * 0.5);
            for i in 0..nf {
                let j = (i + 1) % nf;
                let (ia, ib, oa, ob) = (far_ring[i], far_ring[j], f_outer[i], f_outer[j]);
                dst_far.quad([oa[0], ya, oa[1]], [ob[0], ya, ob[1]], [ob[0], yb, ob[1]], [oa[0], yb, oa[1]], col);
                dst_far.quad([ia[0], yb, ia[1]], [oa[0], yb, oa[1]], [ob[0], yb, ob[1]], [ib[0], yb, ib[1]], col.map(|v| v * 0.96));
            }
            fy += f_pitch;
        }
    }
}

/// Emit rusticated stone plinth, projecting horizontal belt course, and 3D classical
/// Beaux-Arts window surrounds (sills, cornice hoods, proud keystones & pilaster capitals)
/// so historic Embarcadero & waterfront landmarks read as sculpted masonry rather than flat CAD boxes.
#[allow(clippy::too_many_arguments)]
fn emit_beaux_arts_relief(
    ms: &mut TowerMeshes,
    ring: &[[f64; 2]],
    far_ring: &[[f64; 2]],
    base: f64,
    h: f64,
    style: &FacadeStyle,
    stone: Rgb,
    plinth_m: f64,
    belt_frac: f64,
    window_trim: bool,
) {
    if ring.len() < 3 {
        return;
    }
    let c = ring_centroid(ring);
    let fc = ring_centroid(far_ring);
    let plinth_c = [stone[0] * 0.74, stone[1] * 0.75, stone[2] * 0.77];
    let trim_c = stone.map(|v| (v * 1.06).min(0.86));
    let trim_dark = stone.map(|v| v * 0.62);

    // 1. Rusticated stone water-table plinth + projecting bevelled stone water-table ledge.
    if plinth_m > 0.25 {
        let y_pl = (base + plinth_m).min(base + h * 0.45);
        let pn = scale_about(ring, c, 1.012);
        let pf = scale_about(far_ring, fc, 1.014);
        walls(&mut ms.roof_near, &pn, base - 0.5, y_pl, plinth_c);
        walls(&mut ms.roof_far, &pf, base - 0.5, y_pl, plinth_c);
        let ledge_n = scale_about(ring, c, 1.024);
        let ledge_f = scale_about(far_ring, fc, 1.024);
        walls(&mut ms.roof_near, &ledge_n, y_pl - 0.14, y_pl + 0.14, trim_c);
        cap(&mut ms.roof_near, &ledge_n, y_pl + 0.14, trim_c);
        soffit_cap(&mut ms.roof_near, &ledge_n, y_pl - 0.14, trim_dark);
        walls(&mut ms.roof_far, &ledge_f, y_pl - 0.16, y_pl + 0.16, trim_c);
    }

    // 2. Projecting horizontal classical stone belt course / stringcourse.
    if belt_frac > 0.08 && belt_frac < 0.92 {
        let yb = base + h * belt_frac;
        let bn = scale_about(ring, c, 1.020);
        let bf = scale_about(far_ring, fc, 1.020);
        walls(&mut ms.roof_near, &bn, yb - 0.22, yb + 0.22, trim_c);
        cap(&mut ms.roof_near, &bn, yb + 0.22, trim_c);
        soffit_cap(&mut ms.roof_near, &bn, yb - 0.22, trim_dark);
        walls(&mut ms.roof_far, &bf, yb - 0.26, yb + 0.26, trim_c);
    }

    // 3. 3D classical window sills, stepped lintel hoods, arched keystones & pilaster capitals (Near).
    if window_trim {
        let mod_m = style.module_m.max(2.2);
        let st_m = style.storey_m.max(3.2);
        let n_floors = ((h / st_m).floor() as usize).clamp(1, 2);
        let keystone_fl = n_floors - 1;
        let n = ring.len();
        for i in 0..n {
            let (p, q) = (ring[i], ring[(i + 1) % n]);
            let (dx, dz) = (q[0] - p[0], q[1] - p[1]);
            let len = dx.hypot(dz);
            if len < 7.0 {
                continue;
            }
            let (nx, nz) = (-dz / len, dx / len);
            let yaw = dz.atan2(dx);
            if len > 48.0 {
                // Long 100-240 m pier shed side walls: continuous projecting stone sill & lintel mouldings + pilaster capitals.
                let mx = (p[0] + q[0]) * 0.5 + nx * 0.14;
                let mz = (p[1] + q[1]) * 0.5 + nz * 0.14;
                for fl in 0..n_floors {
                    let fy0 = base + fl as f64 * st_m;
                    let sill_y = fy0 + st_m * (0.5 - 0.5 * style.glaze_y);
                    let head_y = fy0 + st_m * (0.5 + 0.5 * style.glaze_y);
                    if head_y > base + h - 0.7 {
                        continue;
                    }
                    ms.roof_near.box_(mx, mz, sill_y - 0.16, sill_y + 0.04, len * 0.49, 0.16, yaw, trim_c, crate::geometry::ALL);
                    ms.roof_near.box_(mx, mz, head_y - 0.02, head_y + 0.20, len * 0.49, 0.18, yaw, trim_c, crate::geometry::ALL);
                }
                let cap_count = ((len / 18.0).round() as usize).clamp(2, 8);
                for k in 0..=cap_count {
                    let t = k as f64 / cap_count as f64;
                    let cx = p[0] + dx * t + nx * 0.22;
                    let cz = p[1] + dz * t + nz * 0.22;
                    ms.roof_near.box_(cx, cz, base + h - 1.35, base + h - 0.72, 0.55, 0.28, yaw, trim_c, crate::geometry::NO_BOTTOM);
                }
                continue;
            }
            // Compact hero Beaux-Arts facades (Agriculture Bldg, Audiffred, Hills Bros, Embarcadero pier frontages):
            // sculpted individual window sills, dark glossy sky-reflecting recessed glass panes, projecting lintel hoods & keystones!
            let bays = ((len / mod_m).floor() as usize).clamp(1, 6);
            let win_w = (mod_m * style.glaze_x).clamp(1.1, 2.3);
            let (tx, tz) = (dx / len, dz / len);
            for b in 0..bays {
                let s = (b as f64 + 0.5) * len / bays as f64;
                let px = p[0] + dx * (s / len);
                let pz = p[1] + dz * (s / len);
                for fl in 0..n_floors {
                    let fy0 = base + fl as f64 * st_m;
                    let sill_y = fy0 + st_m * (0.5 - 0.5 * style.glaze_y);
                    let head_y = fy0 + st_m * (0.5 + 0.5 * style.glaze_y);
                    if head_y > base + h - 0.7 {
                        continue;
                    }
                    // Dark glossy sky-reflecting glass pane aligned cleanly inside the 3D window sill & hood!
                    let gx = px + nx * 0.045;
                    let gz = pz + nz * 0.045;
                    let hw = win_w * 0.48;
                    let ph = (((px * 0.37 + pz * 0.53 + fl as f64 * 1.9).sin() * 43758.5453).fract()).abs();
                    let tilt_u = (ph - 0.5) * 0.06;
                    let tilt_y = ((ph * 3.7).fract() - 0.5) * 0.05 + 0.025;
                    let pnx = nx + tx * tilt_u;
                    let pnz = nz + tz * tilt_u;
                    let pnl = (pnx * pnx + tilt_y * tilt_y + pnz * pnz).sqrt().max(1e-6);
                    let pn = [pnx / pnl, tilt_y / pnl, pnz / pnl];
                    let gcol = [0.055 + 0.025 * ph, 0.075 + 0.032 * ph, 0.098 + 0.038 * ph];
                    let (a, bb, cc, dd) = (
                        [gx - tx * hw, sill_y + 0.04, gz - tz * hw],
                        [gx + tx * hw, sill_y + 0.04, gz + tz * hw],
                        [gx + tx * hw, head_y - 0.02, gz + tz * hw],
                        [gx - tx * hw, head_y - 0.02, gz - tz * hw],
                    );
                    let cr = (bb[1] - a[1]) * (dd[2] - a[2]) - (bb[2] - a[2]) * (dd[1] - a[1]);
                    let crz = (bb[0] - a[0]) * (dd[1] - a[1]) - (bb[1] - a[1]) * (dd[0] - a[0]);
                    if cr * nx + crz * nz >= 0.0 {
                        ms.glass_near.quad_n(a, bb, cc, dd, gcol, pn);
                    } else {
                        ms.glass_near.quad_n(dd, cc, bb, a, gcol, pn);
                    }
                    let sx = px + nx * 0.14;
                    let sz = pz + nz * 0.14;
                    ms.roof_near.box_(sx, sz, sill_y - 0.18, sill_y + 0.04, win_w * 0.56, 0.16, yaw, trim_c, crate::geometry::ALL);
                    ms.roof_near.box_(sx, sz, head_y - 0.02, head_y + 0.22, win_w * 0.58, 0.18, yaw, trim_c, crate::geometry::NO_BOTTOM);
                    if fl == keystone_fl {
                        let kx = px + nx * 0.21;
                        let kz = pz + nz * 0.21;
                        ms.roof_near.box_(kx, kz, head_y - 0.10, head_y + 0.40, 0.26, 0.22, yaw, trim_c, crate::geometry::NO_BOTTOM);
                    }
                }
            }
            if len >= 16.0 {
                let cap_count = ((len / (mod_m * 3.0)).round() as usize).clamp(2, 4);
                for k in 0..=cap_count {
                    let t = k as f64 / cap_count as f64;
                    let cx = p[0] + dx * t + nx * 0.22;
                    let cz = p[1] + dz * t + nz * 0.22;
                    ms.roof_near.box_(cx, cz, base + h - 1.35, base + h - 0.72, 0.55, 0.28, yaw, trim_c, crate::geometry::NO_BOTTOM);
                }
            }
        }
    }
}

/// Emit realistic rooftop mechanical assemblies (louvered cooling-tower yards,
/// articulated BMU window-washing cranes, elevator overruns, and comm masts on
/// the tallest SF icons) with key micro-silhouettes preserved in `Role::Far`.
fn emit_rooftop_plant(ms: &mut TowerMeshes, top_plan: &[[f64; 2]], top_far: &[[f64; 2]], top_y: f64, height_m: f64, yaw: f64, stone: Rgb, roof_c: Rgb, has_spire: bool) {
    let pc = ring_centroid(top_plan);
    let fpc = ring_centroid(top_far);
    let mut max_r = 0.0f64;
    for p in top_plan {
        max_r = max_r.max((p[0] - pc[0]).hypot(p[1] - pc[1]));
    }
    if max_r < 9.0 {
        return;
    }
    let s = (max_r * 0.15).clamp(1.6, 4.6);
    let (co, si) = (yaw.cos(), yaw.sin());
    let at = |c0: [f64; 2], u: f64, v: f64| [c0[0] + u * co - v * si, c0[1] + u * si + v * co];
    let unit_c = stone.map(|v| (v * 0.72).clamp(0.08, 0.52));
    let louver_c = roof_c.map(|v| (v * 0.42).clamp(0.03, 0.22));
    let bmu_c = [0.46, 0.48, 0.50];
    let mast_c = [0.60, 0.63, 0.66];
    let is_tall = height_m >= 120.0;

    // 1. Louvered cooling-tower yard + chiller fan shrouds (larger stepped assemblies on top-30 tall towers).
    let ct_h = if is_tall { 3.8 } else { 2.6 };
    let p1 = at(pc, -s * 0.78, s * 0.52);
    ms.roof_near.box_(p1[0], p1[1], top_y, top_y + ct_h, s * 0.62, s * 0.44, yaw, unit_c, crate::geometry::NO_BOTTOM);
    // Dark recessed louver intake belt around cooling-tower upper band.
    ms.roof_near.box_(p1[0], p1[1], top_y + ct_h * 0.35, top_y + ct_h * 0.80, s * 0.635, s * 0.455, yaw, louver_c, crate::geometry::Faces { top: false, bottom: false });
    // Twin fan shrouds on top of the cooling tower.
    for side in [-0.28, 0.28] {
        let f = at(p1, s * side, 0.0);
        ms.metal_near.box_(f[0], f[1], top_y + ct_h, top_y + ct_h + 0.75, s * 0.18, s * 0.18, yaw, louver_c, crate::geometry::NO_BOTTOM);
    }
    let p2 = at(pc, s * 0.76, -s * 0.46);
    let h2 = if is_tall { 2.9 } else { 2.1 };
    ms.roof_near.box_(p2[0], p2[1], top_y, top_y + h2, s * 0.52, s * 0.40, yaw, unit_c, crate::geometry::NO_BOTTOM);
    ms.roof_near.box_(p2[0], p2[1], top_y + h2 * 0.4, top_y + h2 * 0.82, s * 0.535, s * 0.415, yaw, louver_c, crate::geometry::Faces { top: false, bottom: false });

    // 2. Articulated BMU window-washing crane (turntable pedestal + counterweight + angled jib boom).
    let p3 = at(pc, s * 0.25, s * 0.92);
    let bmu_h = if is_tall { 3.4 } else { 2.5 };
    let jib_len = if is_tall { 5.8 } else { 3.4 };
    let jib_tip = [p3[0] + co * jib_len, top_y + bmu_h + 2.2, p3[1] + si * jib_len];
    let cw_tip = [p3[0] - co * 1.9, top_y + bmu_h - 0.3, p3[1] - si * 1.9];
    ms.metal_near.box_(p3[0], p3[1], top_y, top_y + bmu_h, 0.62, 0.62, yaw, bmu_c, crate::geometry::NO_BOTTOM);
    ms.metal_near.beam([p3[0], top_y + bmu_h - 0.1, p3[1]], jib_tip, 0.34, 0.30, bmu_c);
    ms.metal_near.beam([p3[0], top_y + bmu_h - 0.1, p3[1]], cw_tip, 0.32, 0.28, bmu_c);
    ms.metal_near.box_(cw_tip[0], cw_tip[2], cw_tip[1] - 0.5, cw_tip[1] + 0.4, 0.55, 0.45, yaw, louver_c, crate::geometry::ALL);

    // 3. Preserve key crown micro-silhouettes in Role::Far on the top-30 tallest skyline towers (>= 135 m)!
    if height_m >= 135.0 {
        let fp1 = at(fpc, -s * 0.75, s * 0.50);
        ms.roof_far.box_(fp1[0], fp1[1], top_y, top_y + ct_h, s * 0.60, s * 0.42, yaw, unit_c, crate::geometry::NO_BOTTOM);
        let fp3 = at(fpc, s * 0.25, s * 0.92);
        ms.metal_far.box_(fp3[0], fp3[1], top_y, top_y + bmu_h, 0.7, 0.7, yaw, bmu_c, crate::geometry::NO_BOTTOM);
        ms.metal_far.beam([fp3[0], top_y + bmu_h - 0.1, fp3[1]], [fp3[0] + co * jib_len, top_y + bmu_h + 2.2, fp3[1] + si * jib_len], 0.42, 0.38, bmu_c);
    }

    // 4. Slender rooftop communication / lightning masts on tall corporate towers (>= 150 m) without spires.
    if height_m >= 150.0 && !has_spire {
        let m1 = at(pc, -s * 0.35, -s * 0.72);
        let m2 = at(pc, s * 0.55, s * 0.28);
        let mh1 = 7.5 + (height_m - 150.0).clamp(0.0, 90.0) * 0.08;
        let mh2 = mh1 * 0.68;
        ms.metal_near.box_(m1[0], m1[1], top_y, top_y + mh1, 0.24, 0.24, yaw, mast_c, crate::geometry::ALL);
        ms.metal_near.box_(m2[0], m2[1], top_y, top_y + mh2, 0.20, 0.20, yaw, mast_c, crate::geometry::ALL);
        ms.metal_far.box_(m1[0], m1[1], top_y, top_y + mh1, 0.36, 0.36, yaw, mast_c, crate::geometry::ALL);
    }
}

/// Emit an architectural crown over `(top_plan, top_far)` at `top_y`, returning `tip_y`.
#[allow(clippy::too_many_arguments)]
fn emit_crown(
    ms: &mut TowerMeshes,
    cr: &Value,
    top_plan: &[[f64; 2]],
    top_far: &[[f64; 2]],
    frame: &PlanFrame,
    base: f64,
    h: f64,
    top_y: f64,
    facade_style: &str,
    stone_roughness: f64,
    stone: Rgb,
    roof_c: Rgb,
    rooftop_detail: bool,
) -> f64 {
    let mut tip_y = top_y;
    let num = |k: &str| cr[k].as_f64().unwrap_or(0.0);
    let pc = ring_centroid(top_plan);
    let fpc = ring_centroid(top_far);
    let yaw = frame.yaw();
    let kind = cr["kind"].as_str().unwrap_or("flat");

    match kind {
        "flat" => {
            let p = num("parapetM");
            let is_tower = h >= 50.0;
            if p > 0.0 {
                walls(&mut ms.roof_near, top_plan, top_y, top_y + p, stone.map(|v| v * 0.92));
                walls(&mut ms.roof_near, &scale_about(top_plan, pc, 0.985), top_y, top_y + p, roof_c);
                walls(&mut ms.roof_far, top_far, top_y, top_y + p, stone.map(|v| v * 0.92));
                if is_tower {
                    // Tight fixed-metre overhanging crown parapet cornice + shadowed soffit + metallic coping trim
                    // (kills razor-flat box tops without protruding past long thin slab ends!).
                    let corn_c = stone.map(|v| (v * 1.06).min(0.90));
                    let cope_c = stone.map(|v| (v * 1.12).min(0.94));
                    let cn = offset_ring_out(top_plan, 0.26);
                    let cf = offset_ring_out(top_far, 0.30);
                    let cy0 = (top_y + p - 0.58).max(top_y);
                    walls(&mut ms.roof_near, &cn, cy0, top_y + p, corn_c);
                    cap(&mut ms.roof_near, &cn, top_y + p, corn_c.map(|v| v * 0.92));
                    soffit_cap(&mut ms.roof_near, &cn, cy0, stone.map(|v| v * 0.45));
                    walls(&mut ms.roof_far, &cf, cy0 - 0.12, top_y + p + 0.12, corn_c);
                    walls(&mut ms.metal_near, &offset_ring_out(top_plan, 0.28), top_y + p, top_y + p + 0.26, cope_c);
                    walls(&mut ms.metal_far, &offset_ring_out(top_far, 0.32), top_y + p, top_y + p + 0.32, cope_c);
                }
            }
            let (pf, ph) = (num("penthouseFrac"), num("penthouseM"));
            let mast_m = num("mastM");
            if pf > 0.0 && ph > 0.0 {
                let pr = scale_about(top_plan, pc, pf);
                let fpr = scale_about(top_far, fpc, pf);
                let pen_c = stone.map(|v| v * 0.82);
                let louv_c = stone.map(|v| (v * 0.24).clamp(0.02, 0.16));
                ms.roof_near.prism(&pr, top_y, top_y + ph, pen_c, false, false);
                cap(&mut ms.roof_near, &pr, top_y + ph, roof_c);
                // Recessed louvered collar around the penthouse upper half (Near & Far!).
                let collar = scale_about(&pr, pc, 1.015);
                walls(&mut ms.roof_near, &collar, top_y + ph * 0.35, top_y + ph * 0.78, louv_c);
                ms.roof_far.prism(&fpr, top_y, top_y + ph, pen_c, false, false);
                if is_tower {
                    let fcollar = scale_about(&fpr, fpc, 1.018);
                    walls(&mut ms.roof_far, &fcollar, top_y + ph * 0.35, top_y + ph * 0.78, louv_c);
                    // Crown penthouse cornice cap rim in Near.
                    let pcap = offset_ring_out(&pr, 0.22);
                    walls(&mut ms.roof_near, &pcap, top_y + ph - 0.45, top_y + ph + 0.12, stone.map(|v| v * 0.96));
                }
                cap(&mut ms.roof_far, &fpr, top_y + ph, roof_c);
                if rooftop_detail {
                    emit_rooftop_plant(ms, &pr, &fpr, top_y + ph, h, yaw, stone, roof_c, mast_m > 0.0);
                }
                tip_y = top_y + ph;
            } else if rooftop_detail {
                emit_rooftop_plant(ms, top_plan, top_far, top_y, h, yaw, stone, roof_c, mast_m > 0.0);
            }
            if mast_m > 0.0 {
                let mc = hex_linear(cr["spireSrgb"].as_str().unwrap_or("#d4d8dc"));
                ms.metal_near.box_(pc[0], pc[1], tip_y, tip_y + mast_m, 0.35, 0.35, yaw, mc, crate::geometry::ALL);
                ms.metal_far.box_(pc[0], pc[1], tip_y, tip_y + mast_m, 0.45, 0.45, yaw, mc, crate::geometry::ALL);
                tip_y += mast_m;
            }
            tip_y = tip_y.max(top_y + p);
        }
        "twin-spires" => {
            // 345 California: two square towers turned 45 degrees at the plan's ends, each with a metallic pyramidal spire.
            let side = num("towerFrac") * (frame.u_hi - frame.u_lo);
            let hh = side / 2.0;
            let off = frame.half_u() - hh * std::f64::consts::SQRT_2;
            let tyaw = yaw + std::f64::consts::FRAC_PI_4;
            let roof_y = base + h;
            let spire_top = base + num("spireTopM");
            let spire_c = hex_linear(cr["spireSrgb"].as_str().unwrap_or("#b9bec2"));
            let u_mid = frame.u_mid();
            for sgn in [-1.0, 1.0] {
                let tc = [frame.c[0] + frame.a[0] * (u_mid + sgn * off), frame.c[1] + frame.a[1] * (u_mid + sgn * off)];
                let sq = square(tc, hh, tyaw);
                walls(ms.facade_near(facade_style, stone_roughness), &sq, top_y, roof_y, stone);
                walls(ms.facade_far(facade_style, stone_roughness), &sq, top_y, roof_y, stone);
                ms.roof_near.prism(&square(tc, hh + 0.4, tyaw), roof_y - 1.2, roof_y, stone.map(|v| v * 0.9), false, false);
                cap(&mut ms.roof_near, &square(tc, hh + 0.4, tyaw), roof_y, roof_c);
                cap(&mut ms.roof_far, &square(tc, hh + 0.4, tyaw), roof_y, roof_c);
                // Gleaming aluminium spires in metal AND far-metal!
                pyramid(&mut ms.metal_near, &square(tc, hh * 0.86, tyaw), roof_y, spire_top, spire_c);
                pyramid(&mut ms.metal_far, &square(tc, hh * 0.86, tyaw), roof_y, spire_top, spire_c);
            }
            tip_y = spire_top;
        }
        "spire" => {
            let p = num("parapetM");
            if p > 0.0 {
                walls(ms.facade_near(facade_style, stone_roughness), top_plan, top_y, top_y + p, stone);
                walls(ms.facade_far(facade_style, stone_roughness), top_far, top_y, top_y + p, stone);
            }
            cap(&mut ms.roof_near, &scale_about(top_plan, pc, 0.97), top_y + 0.3, roof_c);
            cap(&mut ms.roof_far, &scale_about(top_far, fpc, 0.97), top_y + 0.3, roof_c);
            let spire_top = base + num("spireTopM");
            let spire_c = hex_linear(cr["spireSrgb"].as_str().unwrap_or("#c0c4c7"));
            ms.metal_near.box_(pc[0], pc[1], top_y, spire_top, 0.55, 0.55, yaw, spire_c, crate::geometry::ALL);
            ms.metal_far.box_(fpc[0], fpc[1], top_y, spire_top, 0.65, 0.65, yaw, spire_c, crate::geometry::ALL);
            tip_y = spire_top;
        }
        "screen-spire" => {
            let screen_top = base + num("screenTopM");
            walls(ms.facade_near(facade_style, stone_roughness), top_plan, top_y, screen_top, stone);
            walls(ms.facade_far(facade_style, stone_roughness), top_far, top_y, screen_top, stone);
            let spire_top = base + num("spireTopM");
            let spire_c = hex_linear(cr["spireSrgb"].as_str().unwrap_or("#c8ccce"));
            pyramid(&mut ms.metal_near, &square(pc, 1.4, yaw), screen_top - 6.0, spire_top, spire_c);
            pyramid(&mut ms.metal_far, &square(fpc, 1.4, yaw), screen_top - 6.0, spire_top, spire_c);
            tip_y = spire_top;
        }
        "screen-taper" => {
            let screen_top = base + num("screenTopM");
            let k = cr["screenTopScale"].as_f64().unwrap_or(1.0).clamp(0.3, 1.0);
            let top_ring = scale_about(top_plan, pc, k);
            let far_top_ring = scale_about(top_far, fpc, k);
            ms.facade_near(facade_style, stone_roughness).loft(top_plan, top_y, &top_ring, screen_top, stone, false);
            ms.facade_far(facade_style, stone_roughness).loft(top_far, top_y, &far_top_ring, screen_top, stone, false);
            ms.roof_near.loft(&scale_about(top_plan, pc, 0.985), top_y, &scale_about(top_plan, pc, k * 0.985), screen_top, stone.map(|v| v * 0.6), false);
            cap(&mut ms.roof_near, &scale_about(top_plan, pc, 0.97), top_y + 0.3, roof_c);
            cap(&mut ms.roof_far, &scale_about(top_far, fpc, 0.97), top_y + 0.3, roof_c);
            let spire_top = base + num("spireTopM");
            let spire_c = hex_linear(cr["spireSrgb"].as_str().unwrap_or("#c8ccce"));
            let half = cr["spireBaseHalfM"].as_f64().unwrap_or(1.0);
            pyramid(&mut ms.metal_near, &square(pc, half, yaw), top_y, spire_top, spire_c);
            pyramid(&mut ms.metal_far, &square(fpc, half, yaw), top_y, spire_top, spire_c);
            tip_y = spire_top;
        }
        "screen-box" => {
            // Open architectural crown screen (illuminated glass or louvered) over a recessed dark mechanical core.
            let screen_h = num("screenM").max(4.0);
            let k = cr["screenScale"].as_f64().unwrap_or(1.0).clamp(0.4, 1.02);
            let screen_style = cr["screenFacade"].as_str().unwrap_or(facade_style);
            let screen_c = cr["screenSrgb"].as_str().map(hex_linear).unwrap_or(stone);
            let sr = scale_about(top_plan, pc, k);
            let fsr = scale_about(top_far, fpc, k);
            let screen_top = top_y + screen_h;
            walls(ms.facade_near(screen_style, 0.25), &sr, top_y, screen_top, screen_c);
            walls(ms.facade_far(screen_style, 0.25), &fsr, top_y, screen_top, screen_c);
            // Inner dark screen wall + mechanical core.
            let inner = scale_about(&sr, pc, 0.96);
            walls(&mut ms.roof_near, &inner, top_y, screen_top - 0.3, stone.map(|v| v * 0.28));
            let core = scale_about(&sr, pc, 0.65);
            let fcore = scale_about(&fsr, fpc, 0.65);
            ms.roof_near.prism(&core, top_y, screen_top - 1.5, roof_c.map(|v| v * 0.7), false, false);
            cap(&mut ms.roof_near, &core, screen_top - 1.5, roof_c);
            // Tight fixed-metre overhanging metallic coping cornice around the open screen + lower transition band (Near & Far!).
            let rim = offset_ring_out(&sr, 0.24);
            let frim = offset_ring_out(&fsr, 0.28);
            let rim_c = hex_linear(cr["spireSrgb"].as_str().unwrap_or("#d4d8dc"));
            walls(&mut ms.metal_near, &rim, screen_top - 0.85, screen_top, rim_c);
            cap(&mut ms.metal_near, &rim, screen_top, rim_c.map(|v| v * 0.92));
            soffit_cap(&mut ms.metal_near, &rim, screen_top - 0.85, rim_c.map(|v| v * 0.55));
            walls(&mut ms.metal_near, &offset_ring_out(&sr, 0.16), top_y - 0.22, top_y + 0.32, rim_c);
            walls(&mut ms.metal_far, &frim, screen_top - 0.95, screen_top, rim_c);
            walls(&mut ms.metal_far, &offset_ring_out(&fsr, 0.20), top_y - 0.26, top_y + 0.36, rim_c);
            // Sculpted vertical architectural frame pilasters articulating the open crown screen (Near & Far).
            let ns = sr.len();
            let s_step = (ns / 8).max(1);
            for i in (0..ns).step_by(s_step) {
                let p = sr[i];
                let pyaw = (p[1] - pc[1]).atan2(p[0] - pc[0]);
                ms.metal_near.box_(p[0], p[1], top_y, screen_top, 0.52, 0.52, pyaw, rim_c, crate::geometry::NO_BOTTOM);
            }
            let nfs = fsr.len();
            let fs_step = (nfs / 4).max(1);
            for i in (0..nfs).step_by(fs_step) {
                let p = fsr[i];
                let pyaw = (p[1] - fpc[1]).atan2(p[0] - fpc[0]);
                ms.metal_far.box_(p[0], p[1], top_y, screen_top, 0.65, 0.65, pyaw, rim_c, crate::geometry::NO_BOTTOM);
            }
            tip_y = screen_top;
            let spire_top = if num("spireTopM") > 0.0 { base + num("spireTopM") } else { tip_y + num("mastM") };
            if rooftop_detail && h >= 110.0 {
                emit_rooftop_plant(ms, &core, &fcore, screen_top - 1.5, h, yaw, stone, roof_c, spire_top > tip_y + 0.5);
            }
            if spire_top > tip_y + 0.5 {
                let half = cr["spireBaseHalfM"].as_f64().unwrap_or(0.55);
                pyramid(&mut ms.metal_near, &square(pc, half, yaw), top_y, spire_top, rim_c);
                pyramid(&mut ms.metal_far, &square(fpc, half.max(0.65), yaw), top_y, spire_top, rim_c);
                tip_y = spire_top;
            }
        }
        "sloped" | "shed" => {
            // Sloping / diagonal bevel crown (e.g. One Rincon Hill, 555 Mission, 120 Kearny, 535 Mission, McKesson).
            let low_m = num("lowM").max(1.0);
            let high_m = num("highM").max(low_m + 3.0);
            let dir = match cr["slopeDir"].as_str().unwrap_or("along") {
                "-along" => [-frame.a[0], -frame.a[1]],
                "across" => frame.b,
                "-across" => [-frame.b[0], -frame.b[1]],
                _ => frame.a,
            };
            let mut dmin = f64::MAX;
            let mut dmax = f64::MIN;
            for p in top_plan {
                let s = (p[0] - pc[0]) * dir[0] + (p[1] - pc[1]) * dir[1];
                dmin = dmin.min(s);
                dmax = dmax.max(s);
            }
            let span = (dmax - dmin).max(1.0);
            let y_at = |p: [f64; 2]| {
                let s = (p[0] - pc[0]) * dir[0] + (p[1] - pc[1]) * dir[1];
                let t = ((s - dmin) / span).clamp(0.0, 1.0);
                top_y + low_m + (high_m - low_m) * t
            };
            let screen_style = cr["screenFacade"].as_str().unwrap_or(facade_style);
            let crown_c = cr["screenSrgb"].as_str().map(hex_linear).unwrap_or(stone);
            let k = cr["crownScale"].as_f64().unwrap_or(1.0).clamp(0.5, 1.0);
            let sr = scale_about(top_plan, pc, k);
            let fsr = scale_about(top_far, fpc, k);
            sloped_prism(ms.facade_near(screen_style, stone_roughness), &sr, top_y, &y_at, crown_c, None);
            sloped_prism(ms.facade_far(screen_style, stone_roughness), &fsr, top_y, &y_at, crown_c, None);
            // Sloped roof cap + metallic cornice edge.
            sloped_prism(&mut ms.roof_near, &scale_about(&sr, pc, 0.99), top_y + low_m * 0.5, &y_at, roof_c, Some(roof_c));
            sloped_prism(&mut ms.roof_far, &scale_about(&fsr, fpc, 0.99), top_y + low_m * 0.5, &y_at, roof_c, Some(roof_c));
            let rim_c = hex_linear(cr["spireSrgb"].as_str().unwrap_or("#d4d8dc"));
            let y_rim0 = |p: [f64; 2]| y_at(p) - 0.7;
            // Crisp metallic edge along the sloped parapet top.
            for (dst, r) in [(&mut ms.metal_near, &sr), (&mut ms.metal_far, &fsr)] {
                let pts = orient_ring(r);
                for i in 0..pts.len() {
                    let (p, q) = (pts[i], pts[(i + 1) % pts.len()]);
                    dst.quad([p[0], y_rim0(p), p[1]], [q[0], y_rim0(q), q[1]], [q[0], y_at(q), q[1]], [p[0], y_at(p), p[1]], rim_c);
                }
            }
            tip_y = top_y + high_m;
            let mast_m = num("mastM");
            if mast_m > 0.0 {
                // Mast at the high crest of the sloped crown!
                let crest = [pc[0] + dir[0] * (dmax * 0.55), pc[1] + dir[1] * (dmax * 0.55)];
                let cy = y_at(crest);
                ms.metal_near.box_(crest[0], crest[1], cy - 2.0, cy + mast_m, 0.45, 0.45, yaw, rim_c, crate::geometry::ALL);
                ms.metal_far.box_(crest[0], crest[1], cy - 2.0, cy + mast_m, 0.6, 0.6, yaw, rim_c, crate::geometry::ALL);
                tip_y = tip_y.max(cy + mast_m);
            }
        }
        "sail" | "canopy" => {
            // Recessed crown mechanical level + cantilevered architectural blade canopy / sail.
            let recess_m = num("recessM").max(4.0);
            let over = cr["overhangScale"].as_f64().unwrap_or(1.08).clamp(1.01, 1.25);
            let thick = cr["canopyThickM"].as_f64().unwrap_or(1.4).max(0.5);
            let slope_m = num("slopeM");
            let canopy_c = hex_linear(cr["spireSrgb"].as_str().unwrap_or("#dfe4e8"));
            let rec_near = scale_about(top_plan, pc, 0.78);
            let rec_far = scale_about(top_far, fpc, 0.78);
            let dark_c = stone.map(|v| v * 0.35);
            walls(&mut ms.roof_near, &rec_near, top_y, top_y + recess_m, dark_c);
            walls(&mut ms.roof_far, &rec_far, top_y, top_y + recess_m, dark_c);
            cap(&mut ms.roof_near, &rec_near, top_y + recess_m, roof_c);
            let can_near = scale_about(top_plan, pc, over);
            let can_far = scale_about(top_far, fpc, over);
            let y_can = top_y + recess_m;
            if slope_m.abs() > 0.2 {
                let dir = frame.a;
                let (mut dmin, mut dmax) = (f64::MAX, f64::MIN);
                for p in &can_near {
                    let s = (p[0] - pc[0]) * dir[0] + (p[1] - pc[1]) * dir[1];
                    dmin = dmin.min(s);
                    dmax = dmax.max(s);
                }
                let span = (dmax - dmin).max(1.0);
                let y_top = |p: [f64; 2]| {
                    let t = (((p[0] - pc[0]) * dir[0] + (p[1] - pc[1]) * dir[1] - dmin) / span).clamp(0.0, 1.0);
                    y_can + thick + slope_m * t
                };
                sloped_prism(&mut ms.metal_near, &can_near, y_can, &y_top, canopy_c, Some(canopy_c));
                sloped_prism(&mut ms.metal_far, &can_far, y_can, &y_top, canopy_c, Some(canopy_c));
                soffit_cap(&mut ms.metal_near, &can_near, y_can, canopy_c.map(|v| v * 0.55));
                tip_y = y_can + thick + slope_m.max(0.0);
            } else {
                walls(&mut ms.metal_near, &can_near, y_can, y_can + thick, canopy_c);
                cap(&mut ms.metal_near, &can_near, y_can + thick, canopy_c);
                soffit_cap(&mut ms.metal_near, &can_near, y_can, canopy_c.map(|v| v * 0.55));
                walls(&mut ms.metal_far, &can_far, y_can, y_can + thick, canopy_c);
                cap(&mut ms.metal_far, &can_far, y_can + thick, canopy_c);
                tip_y = y_can + thick;
            }
        }
        "stepped" | "art-deco" => {
            // Multi-tier Art Deco / Neo-Gothic wedding-cake crown (Russ, 140 New Montgomery, Shell, 450 Sutter, 505 Montgomery).
            let crown_h = num("crownM").max(8.0);
            let steps = cr["steps"].as_u64().unwrap_or(3).clamp(2, 5) as usize;
            let cornice_c = stone.map(|v| (v * 1.06).min(0.95));
            let mut cur_y = top_y;
            let mut cur_scale = 0.84;
            let mut last_ring = top_plan.to_vec();
            let mut last_far = top_far.to_vec();
            for s in 0..steps {
                let sh = crown_h * (if s + 1 == steps { 0.42 } else { 0.58 / (steps - 1) as f64 });
                let nr = scale_about(top_plan, pc, cur_scale);
                let fr = scale_about(top_far, fpc, cur_scale);
                walls(ms.facade_near(facade_style, stone_roughness), &nr, cur_y, cur_y + sh, stone);
                walls(ms.facade_far(facade_style, stone_roughness), &fr, cur_y, cur_y + sh, stone);
                // Stepped terracotta/stone parapet coping + corner pinnacles.
                let cope = scale_about(&nr, pc, 1.025);
                walls(&mut ms.roof_near, &cope, cur_y + sh - 1.1, cur_y + sh + 0.5, cornice_c);
                cap(&mut ms.roof_near, &nr, cur_y + sh, roof_c);
                cap(&mut ms.roof_far, &fr, cur_y + sh, roof_c);
                if s < 2 {
                    // Art Deco corner piers / pinnacles standing proud above each setback!
                    for pt in nr.iter().step_by((nr.len() / 4).max(1)).take(4) {
                        ms.roof_near.box_(pt[0], pt[1], cur_y, cur_y + sh + 1.8, 0.9, 0.9, yaw, cornice_c, crate::geometry::NO_BOTTOM);
                        ms.roof_far.box_(pt[0], pt[1], cur_y + sh - 1.0, cur_y + sh + 1.8, 1.1, 1.1, yaw, cornice_c, crate::geometry::NO_BOTTOM);
                    }
                }
                last_ring = nr;
                last_far = fr;
                cur_y += sh;
                cur_scale *= 0.68;
            }
            tip_y = cur_y;
            // Optional pyramidal lantern / finial / flagpole.
            let lantern_m = num("lanternM");
            if lantern_m > 0.0 {
                let lc = hex_linear(cr["spireSrgb"].as_str().unwrap_or("#d4cdb8"));
                pyramid(&mut ms.roof_near, &scale_about(&last_ring, pc, 0.72), cur_y, cur_y + lantern_m, lc);
                pyramid(&mut ms.roof_far, &scale_about(&last_far, fpc, 0.72), cur_y, cur_y + lantern_m, lc);
                tip_y = cur_y + lantern_m;
            }
            let spire_top = if num("spireTopM") > 0.0 { base + num("spireTopM") } else { tip_y + num("mastM") };
            if spire_top > tip_y + 0.5 {
                let sc = hex_linear(cr["spireSrgb"].as_str().unwrap_or("#d8dde0"));
                ms.metal_near.box_(pc[0], pc[1], cur_y, spire_top, 0.32, 0.32, yaw, sc, crate::geometry::ALL);
                ms.metal_far.box_(fpc[0], fpc[1], cur_y, spire_top, 0.45, 0.45, yaw, sc, crate::geometry::ALL);
                tip_y = spire_top;
            }
        }
        "cylinder-crown" | "drum-step" => {
            // Stepped circular/cylindrical crown (101 California, 388 Market, Infinity).
            let crown_h = num("crownM").max(6.0);
            let rad_near = top_plan.iter().map(|p| (p[0] - pc[0]).hypot(p[1] - pc[1])).fold(0.0, f64::max) * 0.76;
            let r1_near = ellipse_ring(pc, rad_near, rad_near, 24, yaw);
            let r1_far = ellipse_ring(fpc, rad_near, rad_near, 14, yaw);
            let y_mid = top_y + crown_h * 0.58;
            walls(ms.facade_near(facade_style, stone_roughness), &r1_near, top_y, y_mid, stone);
            walls(ms.facade_far(facade_style, stone_roughness), &r1_far, top_y, y_mid, stone);
            cap(&mut ms.roof_near, &r1_near, y_mid, roof_c);
            cap(&mut ms.roof_far, &r1_far, y_mid, roof_c);
            // Upper mechanical drum + metal band.
            let r2_near = ellipse_ring(pc, rad_near * 0.62, rad_near * 0.62, 20, yaw);
            let r2_far = ellipse_ring(fpc, rad_near * 0.62, rad_near * 0.62, 12, yaw);
            let drum_c = stone.map(|v| v * 0.78);
            walls(&mut ms.roof_near, &r2_near, y_mid, top_y + crown_h, drum_c);
            cap(&mut ms.roof_near, &r2_near, top_y + crown_h, roof_c);
            walls(&mut ms.roof_far, &r2_far, y_mid, top_y + crown_h, drum_c);
            cap(&mut ms.roof_far, &r2_far, top_y + crown_h, roof_c);
            let rim_c = hex_linear(cr["spireSrgb"].as_str().unwrap_or("#cdd4d8"));
            let r1_rim = ellipse_ring(pc, rad_near * 1.024, rad_near * 1.024, 24, yaw);
            let r1_frim = ellipse_ring(fpc, rad_near * 1.026, rad_near * 1.026, 14, yaw);
            walls(&mut ms.metal_near, &r1_rim, y_mid - 0.95, y_mid + 0.35, rim_c);
            walls(&mut ms.metal_far, &r1_frim, y_mid - 1.0, y_mid + 0.4, rim_c);
            let r2_rim = ellipse_ring(pc, rad_near * 0.645, rad_near * 0.645, 20, yaw);
            walls(&mut ms.metal_near, &r2_rim, top_y + crown_h - 0.65, top_y + crown_h + 0.25, rim_c);
            for k in 0..8 {
                let a = (k as f64 / 8.0) * 2.0 * PI + yaw;
                let px = pc[0] + a.cos() * (rad_near * 0.63);
                let pz = pc[1] + a.sin() * (rad_near * 0.63);
                ms.metal_near.box_(px, pz, y_mid, top_y + crown_h, 0.45, 0.35, a, rim_c, crate::geometry::NO_BOTTOM);
            }
            tip_y = top_y + crown_h;
        }
        "octagon-spire" => {
            // Stepped octagonal glass/metal lantern crown + spire (100 First Plaza, 505 Montgomery).
            let crown_h = num("crownM").max(8.0);
            let r_max = top_plan.iter().map(|p| (p[0] - pc[0]).hypot(p[1] - pc[1])).fold(0.0, f64::max);
            let oct_r = r_max * cr["crownFrac"].as_f64().unwrap_or(0.62);
            let oct1 = chamfer_ring(pc, oct_r, oct_r, 0.293, yaw);
            let oct2 = chamfer_ring(pc, oct_r * 0.66, oct_r * 0.66, 0.293, yaw);
            let y_mid = top_y + crown_h * 0.55;
            let spire_c = hex_linear(cr["spireSrgb"].as_str().unwrap_or("#d8dde0"));
            walls(ms.facade_near(facade_style, stone_roughness), &oct1, top_y, y_mid, stone);
            walls(ms.facade_far(facade_style, stone_roughness), &oct1, top_y, y_mid, stone);
            cap(&mut ms.roof_near, &oct1, y_mid, roof_c);
            cap(&mut ms.roof_far, &oct1, y_mid, roof_c);
            ms.metal_near.loft(&oct1, y_mid, &oct2, top_y + crown_h, spire_c, false);
            ms.metal_far.loft(&oct1, y_mid, &oct2, top_y + crown_h, spire_c, false);
            cap(&mut ms.metal_near, &oct2, top_y + crown_h, spire_c);
            cap(&mut ms.metal_far, &oct2, top_y + crown_h, spire_c);
            let spire_top = if num("spireTopM") > 0.0 { base + num("spireTopM") } else { top_y + crown_h + 12.0 };
            if spire_top > top_y + crown_h {
                pyramid(&mut ms.metal_near, &chamfer_ring(pc, oct_r * 0.22, oct_r * 0.22, 0.293, yaw), top_y + crown_h, spire_top, spire_c);
                pyramid(&mut ms.metal_far, &chamfer_ring(fpc, oct_r * 0.24, oct_r * 0.24, 0.293, yaw), top_y + crown_h, spire_top, spire_c);
            }
            tip_y = spire_top.max(top_y + crown_h);
        }
        "hipped" | "chateau" => {
            // Steep mansard / French Chateau copper or slate hipped roof (Hunter-Dulin, Mark Hopkins, 580 California).
            let roof_h = num("roofM").max(6.0);
            let ridge_k = cr["ridgeScale"].as_f64().unwrap_or(0.28).clamp(0.05, 0.65);
            let chateau_c = hex_linear(cr["roofSrgb"].as_str().unwrap_or("#59786b"));
            // Projecting classical cornice below the mansard.
            let corn_near = scale_about(top_plan, pc, 1.035);
            let corn_far = scale_about(top_far, fpc, 1.035);
            walls(&mut ms.roof_near, &corn_near, top_y - 1.2, top_y + 0.4, stone.map(|v| (v * 1.05).min(0.95)));
            cap(&mut ms.roof_near, &corn_near, top_y + 0.4, stone.map(|v| v * 0.85));
            walls(&mut ms.roof_far, &corn_far, top_y - 1.2, top_y + 0.4, stone.map(|v| (v * 1.05).min(0.95)));
            // Sloped copper/slate mansard loft to crest ridge.
            let ridge_near = scale_about(top_plan, pc, ridge_k);
            let ridge_far = scale_about(top_far, fpc, ridge_k);
            ms.roof_near.loft(top_plan, top_y, &ridge_near, top_y + roof_h, chateau_c, false);
            cap(&mut ms.roof_near, &ridge_near, top_y + roof_h, chateau_c.map(|v| v * 0.85));
            ms.roof_far.loft(top_far, top_y, &ridge_far, top_y + roof_h, chateau_c, false);
            cap(&mut ms.roof_far, &ridge_far, top_y + roof_h, chateau_c.map(|v| v * 0.85));
            // Decorative cresting ridge + finial.
            let crest_c = hex_linear(cr["spireSrgb"].as_str().unwrap_or("#c8d0cc"));
            walls(&mut ms.metal_near, &ridge_near, top_y + roof_h, top_y + roof_h + 1.2, crest_c);
            tip_y = top_y + roof_h + 1.2;
            let mast_m = num("mastM");
            if mast_m > 0.0 {
                ms.metal_near.box_(pc[0], pc[1], top_y + roof_h, tip_y + mast_m, 0.28, 0.28, yaw, crest_c, crate::geometry::ALL);
                ms.metal_far.box_(fpc[0], fpc[1], top_y + roof_h, tip_y + mast_m, 0.4, 0.4, yaw, crest_c, crate::geometry::ALL);
                tip_y += mast_m;
            }
        }
        "pier-shed" | "gabled" => {
            // Historic San Francisco Embarcadero Beaux-Arts / Mission Revival pier bulkhead & shed roof:
            // 3-step classical dentil entablature with shadowed overhang soffit, sculpted terracotta or
            // standing-seam hipped roof with raised hip & crest ridge cap tiles, optional Beaux-Arts
            // entry pediment, marine roof ventilator cupolas, raised clerestory monitor & Embarcadero headhouse.
            let roof_h = num("roofM").max(2.6);
            let ridge_u = cr["ridgeU"].as_f64().unwrap_or(0.84).clamp(0.15, 0.96);
            let ridge_v = cr["ridgeV"].as_f64().unwrap_or(0.10).clamp(0.04, 0.50);
            let shed_roof_c = hex_linear(cr["roofSrgb"].as_str().unwrap_or("#7c493a"));
            let ridge_tile_c = shed_roof_c.map(|v| (v * 0.82).max(0.02));
            let cornice_c = stone.map(|v| (v * 1.05).min(0.86));
            let frieze_c = stone.map(|v| v * 0.78);
            let soffit_c = stone.map(|v| v * 0.52);
            let pf_near = PlanFrame::from_ring(top_plan);
            let pf_far = PlanFrame::from_ring(top_far);

            // 1. 3-step projecting classical dentil entablature (architrave + shadowed frieze + dentil blocks + overhanging cornice).
            let arch_near = scale_about(top_plan, pc, 1.012);
            let corn_near = scale_about(top_plan, pc, 1.036);
            let corn_far = scale_about(top_far, fpc, 1.032);
            walls(&mut ms.roof_near, &arch_near, top_y - 0.95, top_y - 0.60, cornice_c);
            walls(&mut ms.roof_near, top_plan, top_y - 0.60, top_y - 0.12, frieze_c);
            walls(&mut ms.roof_near, &corn_near, top_y - 0.12, top_y + 0.45, cornice_c);
            cap(&mut ms.roof_near, &corn_near, top_y + 0.45, cornice_c.map(|v| v * 0.88));
            soffit_cap(&mut ms.roof_near, &corn_near, top_y - 0.12, soffit_c);
            walls(&mut ms.roof_far, &corn_far, top_y - 0.65, top_y + 0.45, cornice_c);
            // Rhythmic 3D classical modillion/dentil brackets nestled directly under the cornice soffit (Near).
            let n_ring = top_plan.len();
            for i in 0..n_ring {
                let (p, q) = (top_plan[i], top_plan[(i + 1) % n_ring]);
                let (dx, dz) = (q[0] - p[0], q[1] - p[1]);
                let len = dx.hypot(dz);
                if len < 9.0 {
                    continue;
                }
                let (nx, nz) = (-dz / len, dx / len);
                let dyaw = dz.atan2(dx);
                let (pitch, bw, max_n) = if len <= 55.0 { (3.6, 0.30, 10) } else { (10.5, 0.38, 8) };
                let dentils = ((len / pitch).floor() as usize).clamp(2, max_n);
                for k in 0..dentils {
                    let t = (k as f64 + 0.5) / dentils as f64;
                    let bx = p[0] + dx * t + nx * 0.24;
                    let bz = p[1] + dz * t + nz * 0.24;
                    ms.roof_near.box_(bx, bz, top_y - 0.44, top_y - 0.10, bw, 0.26, dyaw, cornice_c, crate::geometry::NO_BOTTOM);
                }
            }

            // 2. Longitudinal pitched hipped/gabled shed roof (preserved cleanly in Near and Far).
            let ridge_near = scale_aniso(top_plan, &pf_near, ridge_u, ridge_v);
            let ridge_far = scale_aniso(top_far, &pf_far, ridge_u, ridge_v);
            let y_eave = top_y + 0.35;
            let y_ridge = y_eave + roof_h;
            ms.roof_near.loft(&corn_near, y_eave, &ridge_near, y_ridge, shed_roof_c, false);
            cap(&mut ms.roof_near, &ridge_near, y_ridge, ridge_tile_c);
            ms.roof_far.loft(&corn_far, y_eave, &ridge_far, y_ridge, shed_roof_c, false);
            cap(&mut ms.roof_far, &ridge_far, y_ridge, ridge_tile_c);
            tip_y = y_ridge;

            // 3. Sculpted 3D terracotta/standing-seam hip & crest ridge cap tiles (breaks up smooth CAD roof facets!).
            for (dst, eave_r, rid_r, thick) in [
                (&mut ms.roof_near, &corn_near, &ridge_near, 0.30),
                (&mut ms.roof_far, &corn_far, &ridge_far, 0.42),
            ] {
                let m = eave_r.len().min(rid_r.len());
                for i in 0..m {
                    let a = [eave_r[i][0], y_eave + 0.08, eave_r[i][1]];
                    let b = [rid_r[i][0], y_ridge + 0.10, rid_r[i][1]];
                    dst.beam(a, b, thick, thick * 0.72, ridge_tile_c);
                    let bn = [rid_r[(i + 1) % m][0], y_ridge + 0.12, rid_r[(i + 1) % m][1]];
                    if (b[0] - bn[0]).hypot(b[2] - bn[2]) > 0.5 {
                        dst.beam(b, bn, thick * 1.15, thick * 0.85, ridge_tile_c);
                    }
                }
            }

            // 4. Optional classical Beaux-Arts triangular entry pediment / dormer projecting over the two main facades.
            if cr["pediment"].as_bool().unwrap_or(false) {
                for sgn in [-1.0, 1.0] {
                    let ped_u = (pf_near.half_u() * 0.24).clamp(4.5, 9.5);
                    let ped_v = 1.6;
                    let ped_c = pf_near.at(pf_near.u_mid(), pf_near.v_mid() + sgn * (pf_near.half_v() * 0.94));
                    let pring = rect_ring(ped_c, ped_u, ped_v, pf_near.yaw());
                    let y0 = top_y - 0.1;
                    let y1 = y0 + (roof_h * 0.62).clamp(1.8, 3.2);
                    // Pediment recessed tympanum + classical raking cornice cap!
                    walls(&mut ms.roof_near, &pring, y0, y0 + 0.9, frieze_c);
                    let ppf = PlanFrame::from_ring(&pring);
                    let pridge = scale_aniso(&pring, &ppf, 0.14, 0.92);
                    ms.roof_near.loft(&scale_about(&pring, ped_c, 1.04), y0 + 0.85, &pridge, y1, cornice_c, false);
                    cap(&mut ms.roof_near, &pridge, y1, ridge_tile_c);
                    ms.roof_far.loft(&scale_about(&pring, ped_c, 1.04), y0 + 0.85, &pridge, y1, cornice_c, false);
                }
            }

            // 5. Optional continuous glazed clerestory roof monitor along the ridge (Pier 1, 1.5/3, 9, 15, 17, 26, 28).
            let mon_h = num("monitorM");
            if mon_h > 0.3 {
                let mon_u = (pf_near.half_u() * cr["monitorU"].as_f64().unwrap_or(0.70)).max(4.0);
                let mon_v = (pf_near.half_v() * cr["monitorV"].as_f64().unwrap_or(0.24)).max(2.2);
                let mon_c = pf_near.at(pf_near.u_mid(), pf_near.v_mid());
                let mon_ring = rect_ring(mon_c, mon_u, mon_v, pf_near.yaw());
                let mon_y0 = y_eave + roof_h * 0.58;
                let mon_y1 = y_ridge + mon_h;
                let mon_style = cr["monitorFacade"].as_str().unwrap_or(facade_style);
                let mon_tone = stone.map(|v| v * 0.92);
                walls(ms.facade_near(mon_style, stone_roughness), &mon_ring, mon_y0, mon_y1, mon_tone);
                walls(ms.facade_far(mon_style, stone_roughness), &mon_ring, mon_y0, mon_y1, mon_tone);
                let mon_pf = PlanFrame::from_ring(&mon_ring);
                let mon_ridge = scale_aniso(&mon_ring, &mon_pf, 0.90, 0.10);
                let mon_cap_y = mon_y1 + (mon_h * 0.55).clamp(0.8, 1.8);
                let mon_roof_c = cr["monitorRoofSrgb"].as_str().map(hex_linear).unwrap_or_else(|| shed_roof_c.map(|v| v * 0.92));
                ms.roof_near.loft(&mon_ring, mon_y1, &mon_ridge, mon_cap_y, mon_roof_c, false);
                cap(&mut ms.roof_near, &mon_ridge, mon_cap_y, mon_roof_c);
                ms.roof_far.loft(&mon_ring, mon_y1, &mon_ridge, mon_cap_y, mon_roof_c, false);
                cap(&mut ms.roof_far, &mon_ridge, mon_cap_y, mon_roof_c);
                tip_y = tip_y.max(mon_cap_y);
            }

            // 6. Optional weathered copper/galvanized marine roof ventilator cupolas & ridge skylights.
            let n_vents = cr["vents"].as_u64().unwrap_or(0).clamp(0, 6) as usize;
            if n_vents > 0 {
                let vent_y = tip_y - 0.2;
                let vent_c = hex_linear("#5c6d66"); // weathered copper-verdigris / marine zinc
                let vent_dark = [0.14, 0.16, 0.17];
                for k in 0..n_vents {
                    let frac = if n_vents == 1 { 0.0 } else { -0.56 + 1.12 * (k as f64 / (n_vents - 1) as f64) };
                    let vc = pf_near.at(pf_near.u_mid() + pf_near.half_u() * frac * ridge_u, pf_near.v_mid());
                    ms.roof_near.box_(vc[0], vc[1], vent_y - 0.5, vent_y + 0.65, 0.95, 0.95, pf_near.yaw(), vent_dark, crate::geometry::NO_BOTTOM);
                    ms.metal_near.box_(vc[0], vc[1], vent_y + 0.65, vent_y + 1.15, 1.15, 1.15, pf_near.yaw(), vent_c, crate::geometry::ALL);
                    pyramid(&mut ms.metal_near, &square(vc, 1.25, pf_near.yaw()), vent_y + 1.15, vent_y + 1.85, vent_c);
                    if k == 0 || k + 1 == n_vents {
                        pyramid(&mut ms.metal_far, &square(vc, 1.35, pf_near.yaw()), vent_y + 0.5, vent_y + 1.85, vent_c);
                    }
                }
                tip_y = tip_y.max(vent_y + 1.85);
            }

            // 7. Optional Beaux-Arts Embarcadero front bulkhead pavilion on the landward end of long piers.
            if let Some(end) = cr["bulkheadEnd"].as_str() {
                let sgn = if end == "-along" { -1.0 } else { 1.0 };
                let head_frac = cr["bulkheadFrac"].as_f64().unwrap_or(0.11).clamp(0.06, 0.25);
                let head_h = cr["bulkheadM"].as_f64().unwrap_or(3.8).max(1.5);
                let head_roof_h = cr["bulkheadRoofM"].as_f64().unwrap_or(2.6).max(1.0);
                let head_u = (pf_near.half_u() * head_frac).max(6.0);
                let head_v = pf_near.half_v() * 0.96;
                let u_pos = pf_near.u_mid() + sgn * (pf_near.half_u() - head_u).max(0.0);
                let hc = pf_near.at(u_pos, pf_near.v_mid());
                let hring = rect_ring(hc, head_u, head_v, pf_near.yaw());
                let hy1 = top_y + head_h;
                let head_c = cr["bulkheadSrgb"].as_str().map(hex_linear).unwrap_or(cornice_c);
                let head_tile = hex_linear(cr["bulkheadRoofSrgb"].as_str().unwrap_or("#7c493a"));
                let head_ridge_c = head_tile.map(|v| (v * 0.82).max(0.02));
                walls(ms.facade_near(facade_style, stone_roughness), &hring, top_y - 0.5, hy1, head_c);
                walls(ms.facade_far(facade_style, stone_roughness), &hring, top_y - 0.5, hy1, head_c);
                let hcorn = scale_about(&hring, hc, 1.036);
                walls(&mut ms.roof_near, &hcorn, hy1 - 0.7, hy1 + 0.45, head_c);
                soffit_cap(&mut ms.roof_near, &hcorn, hy1 - 0.7, soffit_c);
                walls(&mut ms.roof_far, &hcorn, hy1 - 0.7, hy1 + 0.45, head_c);
                let hpf = PlanFrame::from_ring(&hcorn);
                let hridge = scale_aniso(&hcorn, &hpf, 0.28, 0.76);
                ms.roof_near.loft(&hcorn, hy1 + 0.35, &hridge, hy1 + 0.35 + head_roof_h, head_tile, false);
                cap(&mut ms.roof_near, &hridge, hy1 + 0.35 + head_roof_h, head_ridge_c);
                ms.roof_far.loft(&hcorn, hy1 + 0.35, &hridge, hy1 + 0.35 + head_roof_h, head_tile, false);
                cap(&mut ms.roof_far, &hridge, hy1 + 0.35 + head_roof_h, head_ridge_c);
                // Hip & crest ridge beams on bulkhead terracotta pavilion roof.
                for i in 0..hcorn.len().min(hridge.len()) {
                    ms.roof_near.beam([hcorn[i][0], hy1 + 0.42, hcorn[i][1]], [hridge[i][0], hy1 + 0.45 + head_roof_h, hridge[i][1]], 0.28, 0.22, head_ridge_c);
                }
                tip_y = tip_y.max(hy1 + 0.35 + head_roof_h);
            }
        }
        _ => {}
    }
    tip_y
}

/// What one skyline tower adds to the set.
pub struct BuiltTower {
    pub landmark: Landmark,
    pub lights: Vec<LandmarkLight>,
}

/// Emit a deeply recessed 3D ground-floor colonnade / lobby arcade in Near for tall skyline
/// towers (`h >= 50 m`), restoring authentic street-level depth along downtown canyons
/// (e.g. California St) in both `--play` and default `medium` while keeping `Role::Far` untouched.
fn emit_ground_arcade(ms: &mut TowerMeshes, near_ring: &[[f64; 2]], y0: f64, base: f64, y_lobby: f64, stone: Rgb) {
    let pts = orient_ring(near_ring);
    let n = pts.len();
    if n < 3 || y_lobby <= base + 3.0 {
        return;
    }
    let area2 = |r: &[[f64; 2]]| -> f64 {
        let mut s = 0.0;
        for i in 0..r.len() {
            let (a, b) = (r[i], r[(i + 1) % r.len()]);
            s += a[0] * b[1] - b[0] * a[1];
        }
        s
    };
    let orig_a = area2(&pts);
    let inset_cand = offset_ring_out(&pts, -2.1);
    let cand_a = area2(&inset_cand);
    let valid_inset = orig_a.abs() > 1.0
        && cand_a.signum() == orig_a.signum()
        && cand_a.abs() >= orig_a.abs() * 0.35
        && (0..inset_cand.len()).all(|i| {
            let (a, b) = (inset_cand[i], inset_cand[(i + 1) % inset_cand.len()]);
            (b[0] - a[0]).hypot(b[1] - a[1]) > 0.35
        });
    let lobby_ring = if valid_inset { inset_cand } else { scale_about(&pts, ring_centroid(&pts), 0.86) };

    // 1. Overhanging transfer lintel band + deeply shadowed arcade ceiling soffit.
    let lintel_c = stone.map(|v| (v * 0.96).clamp(0.06, 0.86));
    let soffit_c = stone.map(|v| (v * 0.30).clamp(0.02, 0.28));
    let lip = offset_ring_out(&pts, 0.16);
    walls(&mut ms.roof_near, &lip, y_lobby - 0.58, y_lobby + 0.24, lintel_c);
    soffit_cap(&mut ms.roof_near, &lip, y_lobby - 0.58, soffit_c);

    // 2. Deeply recessed dark architectural lobby storefront glazing + anodized transom, mullions & kickplate.
    let glass_c: Rgb = [0.052, 0.060, 0.068];
    let mull_c: Rgb = [0.11, 0.112, 0.116];
    let kick_y = (base + 0.46).min(y_lobby - 2.2);
    let trans_y = (y_lobby - 1.65).max(kick_y + 1.4);
    let mid_y = (kick_y + trans_y) * 0.5;
    walls(&mut ms.roof_near, &lobby_ring, y0, kick_y, stone.map(|v| v * 0.62));
    walls(&mut ms.glass_near, &lobby_ring, kick_y, y_lobby - 0.48, glass_c);
    let trans_ring = offset_ring_out(&lobby_ring, 0.07);
    walls(&mut ms.metal_near, &trans_ring, trans_y - 0.11, trans_y + 0.11, mull_c);
    walls(&mut ms.metal_near, &trans_ring, mid_y - 0.045, mid_y + 0.045, mull_c);
    // Vertical anodized storefront mullions along the recessed lobby glazing (lean budget: capped per edge).
    let ln = lobby_ring.len();
    for i in 0..ln {
        let (p, q) = (lobby_ring[i], lobby_ring[(i + 1) % ln]);
        let (dx, dz) = (q[0] - p[0], q[1] - p[1]);
        let len = dx.hypot(dz);
        if len < 3.0 {
            continue;
        }
        let (nx, nz) = (-dz / len, dx / len);
        let yaw = dz.atan2(dx);
        let bays = ((len / 2.8).round() as usize).clamp(2, 12);
        for k in 1..bays {
            let frac = k as f64 / bays as f64;
            let mx = p[0] + dx * frac + nx * 0.05;
            let mz = p[1] + dz * frac + nz * 0.05;
            ms.metal_near.box_(mx, mz, kick_y, y_lobby - 0.48, 0.06, 0.09, yaw, mull_c, crate::geometry::Faces { top: false, bottom: false });
        }
    }

    // 3. Sturdy 3D perimeter granite colonnade piers (spaced cleanly around the arcade perimeter,
    //    skipping tight consecutive polygon vertices on circular/chamfered plans).
    let pier_c = stone.map(|v| (v * 0.92).clamp(0.06, 0.84));
    let mut last_pier: Option<[f64; 2]> = None;
    let mut place_pier = |px: f64, pz: f64, yaw: f64, w: f64, d: f64| {
        if let Some(prev) = last_pier {
            if (px - prev[0]).hypot(pz - prev[1]) < 2.6 {
                return;
            }
        }
        last_pier = Some([px, pz]);
        ms.roof_near.box_(px, pz, y0, y_lobby - 0.50, w * 0.5, d * 0.5, yaw, pier_c, crate::geometry::Faces { top: false, bottom: false });
    };
    for i in 0..n {
        let (p, q) = (pts[i], pts[(i + 1) % n]);
        let (dx, dz) = (q[0] - p[0], q[1] - p[1]);
        let len = dx.hypot(dz);
        if len < 0.5 {
            continue;
        }
        let (nx, nz) = (-dz / len, dx / len);
        let yaw = dz.atan2(dx);
        // Corner pier pulled slightly inward so its outer face aligns flush with the shaft above.
        place_pier(p[0] - nx * 0.45, p[1] - nz * 0.45, yaw, 1.08, 0.96);
        if len > 7.2 {
            let spans = ((len / 6.2).round() as usize).clamp(2, 7);
            for k in 1..spans {
                let frac = k as f64 / spans as f64;
                let px = p[0] + dx * frac - nx * 0.42;
                let pz = p[1] + dz * frac - nz * 0.42;
                place_pier(px, pz, yaw, 0.88, 0.84);
            }
        }
    }
}

/// Build one entry on its footprint record (`points`, unscaled) and ground
/// sampler (`ground_at`, the drawn ground of its scaled site at real-site coordinates).
pub fn build_tower(e: &Entry, points: &[f64], ground_at: &dyn Fn(f64, f64) -> f64) -> Option<BuiltTower> {
    let t0 = std::time::Instant::now();
    let ring = ring_of(points);
    if ring.len() < 3 {
        return None;
    }
    let frame = PlanFrame::from_ring(&ring);
    let c = frame.c;
    let base = ring.iter().map(|p| ground_at(p[0], p[1])).fold(f64::INFINITY, f64::min);
    let base = if base.is_finite() { base } else { 0.0 };
    let h = e.height_m;
    let stone = hex_linear(&e.stone_srgb);
    let roof_c = hex_linear(&e.roof_srgb);
    let style = data().facades.get(&e.facade)?;
    let mut ms = TowerMeshes::new(&e.id);

    // Absolute heights of recessed mechanical floors.
    let mech_ys: Vec<f64> = e.mech_floors.iter().map(|&f| base + h * f).collect();

    // Primary tier stack. The foot stands 0.5 m into the ground (a sloping street never shows under it).
    let mut y_prev = base - 0.5;
    let mut plans: Vec<(Vec<[f64; 2]>, Vec<[f64; 2]>, f64, f64)> = Vec::new();
    for (t_idx, t) in e.tiers.iter().enumerate() {
        let (near_plan, far_plan) = tier_rings(&ring, &frame, t);
        if near_plan.len() < 3 {
            continue;
        }
        let y1 = base + h * t.to;
        let t_style = t.facade.as_deref().unwrap_or(&e.facade);
        let t_stone = t.stone_srgb.as_deref().map(hex_linear).unwrap_or(stone);
        let t_rough = t.stone_roughness.unwrap_or(e.stone_roughness);
        let has_arcade = t_idx == 0 && h >= 50.0 && !t.recess_band && t.twist_deg.abs() <= 0.5 && (t.taper - 1.0).abs() <= 1e-3 && (y1 - base) >= 14.0;
        if has_arcade {
            let y_lobby = base + 6.2_f64.min((y1 - base) * 0.42);
            emit_ground_arcade(&mut ms, &near_plan, y_prev, base, y_lobby, t_stone);
            // Keep Role::Far full-height (including mechanical bands) while Role::Near shaft rises above the recessed arcade.
            walls(ms.facade_far(t_style, t_rough), &far_plan, y_prev, y_lobby, t_stone);
            emit_shaft_span(&mut ms, &near_plan, &far_plan, y_lobby, y1, t_style, t_rough, t_stone, roof_c, t.taper, t.twist_deg, &mech_ys, false);
        } else {
            emit_shaft_span(&mut ms, &near_plan, &far_plan, y_prev, y1, t_style, t_rough, t_stone, roof_c, t.taper, t.twist_deg, &mech_ys, t.recess_band);
        }
        // Setback / top roof cap.
        let (cap_near, cap_far) = if (t.taper - 1.0).abs() > 1e-3 || t.twist_deg.abs() > 0.5 {
            let rad = t.twist_deg.to_radians();
            (
                rotate_about(&scale_about(&near_plan, ring_centroid(&near_plan), t.taper), ring_centroid(&near_plan), rad),
                rotate_about(&scale_about(&far_plan, ring_centroid(&far_plan), t.taper), ring_centroid(&far_plan), rad),
            )
        } else {
            (near_plan.clone(), far_plan.clone())
        };
        cap(&mut ms.roof_near, &cap_near, y1, roof_c);
        cap(&mut ms.roof_far, &cap_far, y1, roof_c);
        // Tight fixed-metre setback coping ledge on intermediate setback terraces of tall skyline towers!
        if h >= 50.0 && t.to < 0.995 && !t.recess_band {
            let ledge_c = t_stone.map(|v| (v * 1.06).min(0.90));
            let ln = offset_ring_out(&cap_near, 0.24);
            let lf = offset_ring_out(&cap_far, 0.28);
            walls(&mut ms.roof_near, &ln, y1 - 0.50, y1 + 0.32, ledge_c);
            cap(&mut ms.roof_near, &ln, y1 + 0.32, ledge_c.map(|v| v * 0.90));
            soffit_cap(&mut ms.roof_near, &ln, y1 - 0.50, t_stone.map(|v| v * 0.45));
            walls(&mut ms.roof_far, &lf, y1 - 0.58, y1 + 0.38, ledge_c);
        }
        plans.push((cap_near, cap_far, y_prev.max(base), y1));
        y_prev = y1;
    }
    let (top_plan, top_far, _, top_y) = plans.last().cloned()?;

    // Primary crown.
    let mut tip_y = emit_crown(&mut ms, &e.crown, &top_plan, &top_far, &frame, base, h, top_y, &e.facade, e.stone_roughness, stone, roof_c, e.rooftop_detail);

    // Optional projecting classical/Beaux-Arts overhanging crown cornice (`corniceM`, fixed-metre normal offset).
    if e.cornice_m > 0.15 {
        let off = e.cornice_m.clamp(0.25, 0.85);
        let corn_c = stone.map(|v| (v * 1.08).min(0.92));
        let cn = offset_ring_out(&top_plan, off);
        let cf = offset_ring_out(&top_far, (off * 1.1).min(0.95));
        walls(&mut ms.roof_near, &cn, top_y - 0.72, top_y + 0.44, corn_c);
        cap(&mut ms.roof_near, &cn, top_y + 0.44, corn_c.map(|v| v * 0.92));
        soffit_cap(&mut ms.roof_near, &cn, top_y - 0.72, stone.map(|v| v * 0.42));
        walls(&mut ms.roof_far, &cf, top_y - 0.80, top_y + 0.50, corn_c);
    }

    // Optional iconic SF night architectural edge / crown outline strips (Embarcadero 1-4, modern illuminated crowns).
    // Strictly anchored to real facade/parapet edges (`top_y` / `py1`, NEVER `tip_y`!) with tight fixed-metre offsets.
    if let Some(mode) = e.accent_edges.as_deref() {
        let (acc_style, acc_col) = match mode {
            "embarcadero" => ("sf-embarcadero-edge-glow", hex_linear("#d8d3c8")),
            "crown-cyan" => ("sf-crown-edge-cyan", hex_linear("#cddae0")),
            _ => ("sf-crown-edge-warm", hex_linear("#dcd8ce")),
        };
        if data().facades.contains_key(acc_style) {
            if mode == "embarcadero" {
                // Iconic Embarcadero Center glowing horizontal roofline perimeter on every stepped tier + tight vertical corner strips!
                for (pn, pf, py0, py1) in &plans {
                    let cn = ring_centroid(pn);
                    let strip_n = offset_ring_out(pn, 0.16);
                    let strip_f = offset_ring_out(pf, 0.24);
                    walls(ms.facade_near(acc_style, 0.42), &strip_n, py1 - 0.35, py1 + 0.45, acc_col);
                    walls(ms.facade_far(acc_style, 0.42), &strip_f, py1 - 0.45, py1 + 0.65, acc_col);
                    // Vertical glowing outline edge strips hugging the outer slab corners tightly (0.20 m proud).
                    if *py1 - *py0 > 18.0 {
                        let step_n = (pn.len() / 4).max(1);
                        for pt in pn.iter().step_by(step_n).take(4) {
                            let dx = pt[0] - cn[0];
                            let dz = pt[1] - cn[1];
                            let dl = dx.hypot(dz).max(1e-6);
                            let ox = pt[0] + dx / dl * 0.20;
                            let oz = pt[1] + dz / dl * 0.20;
                            let pyaw = dz.atan2(dx);
                            ms.facade_near(acc_style, 0.42).box_(ox, oz, *py0 + 14.0, *py1 + 0.35, 0.28, 0.28, pyaw, acc_col, crate::geometry::Faces { top: false, bottom: false });
                            ms.facade_far(acc_style, 0.42).box_(ox, oz, *py0 + 14.0, *py1 + 0.55, 0.46, 0.46, pyaw, acc_col, crate::geometry::Faces { top: false, bottom: false });
                        }
                    }
                }
            } else {
                // Sleek architectural glowing parapet perimeter light ribbon strictly hugging `top_plan` at `top_y` (never `tip_y`!).
                let rn = offset_ring_out(&top_plan, 0.14);
                let rf = offset_ring_out(&top_far, 0.20);
                walls(ms.facade_near(acc_style, 0.35), &rn, top_y - 0.85, top_y + 0.10, acc_col);
                walls(ms.facade_far(acc_style, 0.35), &rf, top_y - 1.05, top_y + 0.20, acc_col);
            }
        }
    }

    // Optional Beaux-Arts / classical relief (rusticated plinth, mid-story belt course, window sills/keystones/capitals).
    if e.plinth_m > 0.25 || (e.belt_frac > 0.08 && e.belt_frac < 0.92) || e.window_trim {
        if let Some((base_near, base_far, _, base_y1)) = plans.first() {
            emit_beaux_arts_relief(&mut ms, base_near, base_far, base, (base_y1 - base).max(3.0), style, stone, e.plinth_m, e.belt_frac, e.window_trim);
        }
    }

    // Choose the main shaft tier (tallest tier, usually above any low podium) for ribs, sunshades and X-braces.
    let shaft_idx = plans
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| (a.3 - a.2).total_cmp(&(b.3 - b.2)))
        .map(|(i, _)| i)
        .unwrap_or(0);
    let (ref shaft_ring, ref shaft_far_ring, shaft_y0, shaft_y1) = plans[shaft_idx];

    if let Some(r) = &e.ribs {
        let req_y0 = base + h * r.from_frac;
        let req_y1 = base + h * r.to_frac;
        if r.all_tiers {
            for (tr, _, ty0, ty1) in &plans {
                let ry0 = req_y0.max(*ty0);
                let ry1 = req_y1.min(*ty1);
                if ry1 > ry0 + 2.0 {
                    emit_ribs(&mut ms, tr, ry0, ry1, r, stone);
                }
            }
        } else {
            let ry0 = req_y0.max(shaft_y0);
            let ry1 = req_y1.min(shaft_y1);
            emit_ribs(&mut ms, shaft_ring, ry0, ry1, r, stone);
        }
    }
    if let Some(sh) = &e.sunshades {
        let sy0 = (base + h * sh.from_frac).max(shaft_y0);
        let sy1 = (base + h * sh.to_frac).min(shaft_y1);
        emit_sunshades(&mut ms, shaft_ring, shaft_far_ring, sy0, sy1, style.storey_m, sh, stone);
    }
    if let Some(b) = &e.braces {
        // Diagonal braces across every long face, one X per `storeysPerBay` storeys (preserved in both Near AND Far!).
        let span = b["storeysPerBay"].as_f64().unwrap_or(8.0) * style.storey_m;
        let (bw, bd) = (b["widthM"].as_f64().unwrap_or(0.9), b["depthM"].as_f64().unwrap_or(0.35));
        let brace_c = b["srgb"].as_str().or_else(|| e.crown["spireSrgb"].as_str()).map(hex_linear).unwrap_or_else(|| hex_linear("#c8ccce"));
        let br_ring = if b["onShaft"].as_bool().unwrap_or(false) { shaft_ring.as_slice() } else { ring.as_slice() };
        let br_y0 = if b["onShaft"].as_bool().unwrap_or(false) { shaft_y0 } else { base };
        let br_y1 = if b["onShaft"].as_bool().unwrap_or(false) { shaft_y1 } else { top_y };
        let n = br_ring.len();
        for i in 0..n {
            let (p, q) = (br_ring[i], br_ring[(i + 1) % n]);
            let (dx, dz) = (q[0] - p[0], q[1] - p[1]);
            let len = dx.hypot(dz);
            if len < 12.0 {
                continue;
            }
            let (nx, nz) = (-dz / len, dx / len);
            let o = 0.18 + if bd > 0.35 { bd / 2.0 - 0.175 } else { 0.0 };
            let at = |t: f64, y: f64| -> V3 { [p[0] + dx * t + nx * o, y, p[1] + dz * t + nz * o] };
            let bays = ((br_y1 - br_y0) / span).floor().max(1.0) as i64;
            for k in 0..bays {
                let (ya, yb) = (br_y0 + k as f64 * span, br_y0 + (k + 1) as f64 * span);
                ms.metal_near.beam_up(at(0.04, ya), at(0.96, yb), bw, bd, brace_c, Some([nx, 0.0, nz]));
                ms.metal_near.beam_up(at(0.96, ya), at(0.04, yb), bw, bd, brace_c, Some([nx, 0.0, nz]));
                // Preserve iconic exoskeleton X-braces in Far mesh so 181 Fremont & One Maritime read from the Bay Bridge!
                ms.metal_far.beam_up(at(0.04, ya), at(0.96, yb), bw.max(1.2), bd.max(0.55), brace_c, Some([nx, 0.0, nz]));
                ms.metal_far.beam_up(at(0.96, ya), at(0.04, yb), bw.max(1.2), bd.max(0.55), brace_c, Some([nx, 0.0, nz]));
            }
        }
    }

    // Secondary towers on the same complex footprint (One Rincon Hill Harrison, One Market Steuart, LUMINA B, etc.).
    for sub in &e.sub_towers {
        let sub_style = sub.facade.as_deref().unwrap_or(&e.facade);
        let sub_stone = sub.stone_srgb.as_deref().map(hex_linear).unwrap_or(stone);
        let sub_rough = sub.stone_roughness.unwrap_or(e.stone_roughness);
        let sub_storey = data().facades.get(sub_style).map(|s| s.storey_m).unwrap_or(style.storey_m);
        let sub_mechs: Vec<f64> = sub.mech_floors.iter().map(|&f| base + sub.height_m * f).collect();
        let mut sy_prev = base + sub.from_m - 0.5;
        let sub_tiers: Vec<Tier> = if sub.tiers.is_empty() {
            vec![Tier {
                to: 1.0,
                along_frac: sub.along_frac,
                across_frac: sub.across_frac,
                along_shift: sub.along_shift,
                across_shift: sub.across_shift,
                scale: 1.0,
                shape: sub.shape.clone(),
                chamfer: sub.chamfer,
                notch: sub.notch,
                taper: 1.0,
                twist_deg: 0.0,
                yaw_deg: sub.yaw_deg,
                facade: None,
                stone_srgb: None,
                stone_roughness: None,
                recess_band: false,
            }]
        } else {
            sub.tiers
                .iter()
                .map(|st| Tier {
                    along_frac: sub.along_frac * st.along_frac,
                    across_frac: sub.across_frac * st.across_frac,
                    along_shift: sub.along_shift + st.along_shift * sub.along_frac,
                    across_shift: sub.across_shift + st.across_shift * sub.across_frac,
                    yaw_deg: sub.yaw_deg + st.yaw_deg,
                    shape: if st.shape == "plan" { sub.shape.clone() } else { st.shape.clone() },
                    ..st.clone()
                })
                .collect()
        };
        let mut last_sub: Option<(Vec<[f64; 2]>, Vec<[f64; 2]>, f64)> = None;
        let mut first_sub_ring: Option<(Vec<[f64; 2]>, Vec<[f64; 2]>)> = None;
        for st in &sub_tiers {
            let (nr, fr) = tier_rings(&ring, &frame, st);
            if nr.len() < 3 {
                continue;
            }
            let sy1 = base + sub.height_m * st.to;
            let st_style = st.facade.as_deref().unwrap_or(sub_style);
            let st_stone = st.stone_srgb.as_deref().map(hex_linear).unwrap_or(sub_stone);
            let st_rough = st.stone_roughness.unwrap_or(sub_rough);
            emit_shaft_span(&mut ms, &nr, &fr, sy_prev, sy1, st_style, st_rough, st_stone, roof_c, st.taper, st.twist_deg, &sub_mechs, st.recess_band);
            cap(&mut ms.roof_near, &nr, sy1, roof_c);
            cap(&mut ms.roof_far, &fr, sy1, roof_c);
            if first_sub_ring.is_none() {
                first_sub_ring = Some((nr.clone(), fr.clone()));
            }
            last_sub = Some((nr, fr, sy1));
            sy_prev = sy1;
        }
        if let Some((sr, sfr, stop_y)) = last_sub {
            if let Some((ref base_sr, ref base_sfr)) = first_sub_ring {
                if let Some(r) = &sub.ribs {
                    let ry0 = (base + sub.height_m * r.from_frac).max(base + sub.from_m);
                    let ry1 = (base + sub.height_m * r.to_frac).min(stop_y);
                    emit_ribs(&mut ms, base_sr, ry0, ry1, r, sub_stone);
                }
                if let Some(sh) = &sub.sunshades {
                    let sy0 = (base + sub.height_m * sh.from_frac).max(base + sub.from_m);
                    let sy1 = (base + sub.height_m * sh.to_frac).min(stop_y);
                    emit_sunshades(&mut ms, base_sr, base_sfr, sy0, sy1, sub_storey, sh, sub_stone);
                }
            }
            if !sub.crown.is_null() {
                let sub_tip = emit_crown(&mut ms, &sub.crown, &sr, &sfr, &frame, base, sub.height_m, stop_y, sub_style, sub_rough, sub_stone, roof_c, e.rooftop_detail);
                tip_y = tip_y.max(sub_tip);
            }
        }
    }

    // Assemble Near and Far Draw records cleanly.
    let mut draws = Vec::new();
    let primary_key = TowerMeshes::key(&e.facade, e.stone_roughness);
    for ((st_name, r_milli), mesh) in ms.facades_near {
        if mesh.indices.is_empty() {
            continue;
        }
        let key = if (st_name.as_str(), r_milli) == (primary_key.0.as_str(), primary_key.1) { "facade".to_string() } else { format!("facade-{st_name}") };
        draws.push(Draw { key, role: Role::Near, material: facade_material_at(&st_name, r_milli as f64 / 1000.0, 0.0, base), mesh: mesh.freeze() });
    }
    if !ms.roof_near.indices.is_empty() {
        draws.push(Draw { key: "roof".into(), role: Role::Near, material: roof_trim_material(), mesh: ms.roof_near.freeze() });
    }
    let metal_mat = if crate::towers4_on() {
        plain_material(METAL4_ROUGHNESS, METAL4_METALNESS, "facade-metal", None)
    } else {
        plain_material(0.35, 0.7, "facade-metal", None)
    };
    if !ms.metal_near.indices.is_empty() {
        draws.push(Draw { key: "metal".into(), role: Role::Near, material: metal_mat.clone(), mesh: ms.metal_near.freeze() });
    }
    if !ms.glass_near.indices.is_empty() {
        draws.push(Draw { key: "glass".into(), role: Role::Near, material: plain_material(0.05, 0.34, "facade-glass", None), mesh: ms.glass_near.freeze() });
    }

    // Bounding box over Near draws.
    let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
    for d in &draws {
        for p in d.mesh.positions.chunks_exact(3) {
            for k in 0..3 {
                mn[k] = mn[k].min(p[k]);
                mx[k] = mx[k].max(p[k]);
            }
        }
    }

    // Far draws: primary facade ALWAYS keeps key "far" (checked by tests), secondary facades get "far-<style>",
    // plus separate "far-roof" and "far-metal" so spires/braces gleam and roofs never get fake windows!
    let mut facades_far = ms.facades_far;
    let mut has_far_key = false;
    if let Some(primary_mesh) = facades_far.remove(&primary_key) {
        if !primary_mesh.indices.is_empty() {
            has_far_key = true;
            draws.push(Draw {
                key: "far".to_string(),
                role: Role::Far,
                material: facade_material_at(&primary_key.0, primary_key.1 as f64 / 1000.0, 0.0, base),
                mesh: primary_mesh.freeze(),
            });
        }
    }
    for ((st_name, r_milli), mesh) in facades_far {
        if mesh.indices.is_empty() {
            continue;
        }
        let key = if !has_far_key {
            has_far_key = true;
            "far".to_string()
        } else {
            format!("far-{st_name}")
        };
        draws.push(Draw { key, role: Role::Far, material: facade_material_at(&st_name, r_milli as f64 / 1000.0, 0.0, base), mesh: mesh.freeze() });
    }
    if !ms.roof_far.indices.is_empty() {
        draws.push(Draw { key: "far-roof".into(), role: Role::Far, material: roof_trim_material(), mesh: ms.roof_far.freeze() });
    }
    if !ms.metal_far.indices.is_empty() {
        draws.push(Draw { key: "far-metal".into(), role: Role::Far, material: metal_mat, mesh: ms.metal_far.freeze() });
    }

    let mut lights = Vec::new();
    if e.aviation {
        let light_c = ring_centroid(&top_plan);
        lights.push(crate::lights::aviation(light_c, tip_y));
    }
    let landmark = Landmark {
        id: e.id.clone(),
        name: e.name.clone(),
        version: data().version.clone(),
        anchor: c,
        box_min: mn,
        box_max: mx,
        draws,
        colliders: vec![],
        stats: json!({ "baseY": base, "roofY": top_y, "tipY": tip_y, "record": e.record }),
        build_ms: t0.elapsed().as_secs_f64() * 1e3,
    };
    Some(BuiltTower { landmark, lights })
}

/// Every skyline tower, on the drawn ground `height_at` (moved to its scaled
/// site, as the registry's landmark buildings are).
pub fn build_all(repo_root: &std::path::Path, height_at: &(dyn Fn(f64, f64) -> Option<f64> + Sync)) -> Result<(Vec<Landmark>, Vec<LandmarkLight>)> {
    let recs = load_records(repo_root)?;
    let s = eo_data::groundscale::scale();
    let mut out = Vec::new();
    let mut lights = Vec::new();
    let d_all = data();
    for e in d_all.buildings.iter().chain(d_all.waterfront.iter()) {
        let Some(points) = recs.get(&e.record) else { continue };
        let ring = ring_of(points);
        if ring.len() < 3 {
            continue;
        }
        let c = ring_centroid(&ring);
        let d = [(s - 1.0) * c[0], (s - 1.0) * c[1]];
        let ground = |x: f64, z: f64| height_at(x + d[0], z + d[1]).unwrap_or(0.0);
        if let Some(mut b) = build_tower(e, points, &ground) {
            b.landmark.translate(d);
            for l in &mut b.lights {
                l.pos[0] += d[0];
                l.pos[2] += d[1];
                l.base[0] += d[0];
                l.base[2] += d[1];
            }
            lights.extend(b.lights);
            out.push(b.landmark);
        }
    }
    // Moored San Francisco Bay commuter ferries & harbour workboat along Ferry Plaza / Pier 14
    // (open-water berths clear of all pier rings; cleanly removed when `--no-skyline` is passed).
    let sea_y = crate::LandmarksJs::get().sea_level_y;
    for mut v in build_moored_bay_vessels(sea_y) {
        let d = [(s - 1.0) * v.anchor[0], (s - 1.0) * v.anchor[1]];
        v.translate(d);
        out.push(v);
    }
    // Night-only architectural illumination for the San Francisco Ferry Building's 4 iconic
    // 22-ft clock dials, Giralda belfry/lantern arches, nave skylight, and marketplace colonnade
    // (zero daytime change, zero Z-fighting, toggled cleanly by `--no-skyline` / `--no-ferry-building`).
    if crate::features::ferry_building() {
        if let Some(fb_night) = build_ferry_night_illumination(height_at) {
            out.push(fb_night);
        }
    }
    Ok((out, lights))
}

/// Build night architectural illumination for the San Francisco Ferry Building:
/// - Four 22-foot warm-ivory clock dials & Giralda belfry/lantern arch backings lit at night
///   only via `lm_flood3` interior projectors (`dusk == 0` in daylight -> zero daytime change!).
/// - Recessed open ground arcade gallery, roof monitor clerestory, and bay-side `Role::Far`
///   upper arched window glow via `"sf-ferry-arcade-glow"` so the Ferry Building shines across
///   the Bay in `06a_night_skyline` while keeping `08_embarcadero_street_closeup` untouched.
pub fn build_ferry_night_illumination(height_at: &(dyn Fn(f64, f64) -> Option<f64> + Sync)) -> Option<Landmark> {
    let sp = crate::ferry_building::spec();
    let inf = &sp.inferred;
    let s = eo_data::groundscale::scale();
    let sc = |p: [f64; 2]| [p[0] * s, p[1] * s];
    let r = &sp.record;
    let (nw, nb, sb, sw) = (sc(r.north_west), sc(r.north_bay), sc(r.south_bay), sc(r.south_west));
    let len = (sw[0] - nw[0]).hypot(sw[1] - nw[1]);
    let u = [(sw[0] - nw[0]) / len, (sw[1] - nw[1]) / len];
    let mut n = [-u[1], u[0]];
    if (nw[0] - nb[0]) * n[0] + (nw[1] - nb[1]) * n[1] < 0.0 {
        n = [-n[0], -n[1]];
    }
    let at_frame = |lu: f64, lv: f64, y: f64| -> V3 { [nw[0] + u[0] * lu + n[0] * lv, y, nw[1] + u[1] * lu + n[1] * lv] };
    let local = |p: [f64; 2]| -> [f64; 2] {
        let d = [p[0] - nw[0], p[1] - nw[1]];
        [d[0] * u[0] + d[1] * u[1], d[0] * n[0] + d[1] * n[1]]
    };
    let depth = -local(nb)[1].min(local(sb)[1]);
    let length = local(sw)[0].max(local(sb)[0]);
    let pav: Vec<[f64; 2]> = r.pavilion.iter().map(|p| local(sc(*p))).collect();
    let (pu0, pu1, pd) = (pav[0][0].min(pav[1][0]), pav[2][0].max(pav[3][0]), pav[1][1].max(pav[2][1]));
    let mut lv: Vec<f64> = (0..=40)
        .map(|k| {
            let a = length * k as f64 / 40.0;
            let v = if a > pu0 && a < pu1 { pd + 1.5 } else { 1.5 };
            let p = at_frame(a, v, 0.0);
            height_at(p[0], p[2]).unwrap_or(2.8)
        })
        .collect();
    lv.sort_by(f64::total_cmp);
    let base = lv[lv.len() / 2];

    let tc_world = sc(sp.tower.centre_surveyed);
    let tc = local(tc_world);
    let st = &sp.tower.stages;
    let h0 = st[0].side_m * 0.5;
    let shaft_top = st[0].top_m;
    let dial_y = base + shaft_top - inf.dial_centre_below_shaft_top_m;
    let dial_r = sp.tower.clock.dial_diameter_m * 0.5 - 0.04;

    let mut clock_near = MeshArrays::new("ferry-night:clock");
    let mut clock_far = MeshArrays::new("ferry-night:far-clock");
    let mut arcade_near = MeshArrays::new("ferry-night:arcade");
    let mut arcade_far = MeshArrays::new("ferry-night:far-arcade");
    let mut uplight_far = MeshArrays::new("ferry-night:far-uplight");
    let mut beacon_post_near = MeshArrays::new("ferry-night:beacon-post");
    let mut beacon_red_near = MeshArrays::new("ferry-night:beacon-red");
    let mut beacon_red_far = MeshArrays::new("ferry-night:far-beacon-red");
    let mut beacon_green_near = MeshArrays::new("ferry-night:beacon-green");
    let mut beacon_green_far = MeshArrays::new("ferry-night:far-beacon-green");

    let dial_col = hex_linear("#ebe6da");
    let belfry_col = hex_linear("#26221d");
    let arcade_col = hex_linear("#26221d");
    let uplight_col = hex_linear("#c4b698");
    let post_col = hex_linear("#2a2f34");
    let red_col = hex_linear("#6e1f1c");
    let green_col = hex_linear("#1c5840");

    let push_q = |m: &mut MeshArrays, a: V3, b: V3, c: V3, d: V3, want: V3, col: Rgb| {
        let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let e2 = [d[0] - a[0], d[1] - a[1], d[2] - a[2]];
        let cr = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
        if cr[0] * want[0] + cr[1] * want[1] + cr[2] * want[2] >= 0.0 {
            m.quad(a, b, c, d, col);
        } else {
            m.quad(d, c, b, a, col);
        }
    };

    // 1. Four 22-foot illuminated clock dials (at d=+0.138, cleanly 1.8 cm proud of F::Dial 0.12 and 3.7 cm behind ticks/hands!).
    // Uses `sf-ferry-clock-glow`: cream-white by day (`dusk==0` -> identical to `painted-white`), luminous 16 cd/m2 warm ivory at night!
    let face_dirs: [([f64; 2], [f64; 2]); 4] = [
        ([1.0, 0.0], [0.0, 1.0]),   // plaza (+v)
        ([0.0, -1.0], [1.0, 0.0]),  // south (+u)
        ([-1.0, 0.0], [0.0, -1.0]), // bay (-v)
        ([0.0, 1.0], [-1.0, 0.0]),  // north (-u)
    ];
    for (td, nd) in face_dirs {
        let wn = [u[0] * nd[0] + n[0] * nd[1], 0.0, u[1] * nd[0] + n[1] * nd[1]];
        let face_pt = |off_t: f64, y: f64, off_n: f64| at_frame(tc[0] + td[0] * off_t + nd[0] * off_n, tc[1] + td[1] * off_t + nd[1] * off_n, y);
        let segs = 16usize;
        let ctr = face_pt(0.0, dial_y, h0 + 0.138);
        for dst in [&mut clock_near, &mut clock_far] {
            for k in 0..segs {
                let a0 = 2.0 * PI * k as f64 / segs as f64;
                let a1 = 2.0 * PI * (k + 1) as f64 / segs as f64;
                let p0 = face_pt(dial_r * a0.cos(), dial_y + dial_r * a0.sin(), h0 + 0.138);
                let p1 = face_pt(dial_r * a1.cos(), dial_y + dial_r * a1.sin(), h0 + 0.138);
                let e1 = [p0[0] - ctr[0], p0[1] - ctr[1], p0[2] - ctr[2]];
                let e2 = [p1[0] - ctr[0], p1[1] - ctr[1], p1[2] - ctr[2]];
                let cr = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
                if cr[0] * wn[0] + cr[1] * wn[1] + cr[2] * wn[2] >= 0.0 {
                    dst.tri(ctr, p0, p1, dial_col);
                } else {
                    dst.tri(ctr, p1, p0, dial_col);
                }
            }
        }
        // 2. Giralda clock tower belfry & lantern open-arch glowing interiors (`sf-ferry-arcade-glow` 12.5 cd/m2 warm amber!).
        // Near sits at d=-0.44 (inside -0.55 Back::Void, dark shadowed recess by day in `08`);
        // Far sits at d=-0.26 so every tower belfry & lantern arch shines unobstructed from 1.15 km across the Bay in `06a`!
        let mut sy = shaft_top;
        for stg in st.iter().skip(1) {
            let hs = stg.side_m * 0.5;
            let tall = stg.top_m - sy;
            let y0 = base + sy + tall * 0.16;
            let y1 = base + stg.top_m - 0.50;
            let hw = (stg.side_m * 0.42).max(0.85);
            push_q(&mut arcade_near, face_pt(-hw, y0, hs - 0.44), face_pt(hw, y0, hs - 0.44), face_pt(hw, y1, hs - 0.44), face_pt(-hw, y1, hs - 0.44), wn, belfry_col);
            push_q(&mut arcade_far, face_pt(-hw, y0, hs - 0.24), face_pt(hw, y0, hs - 0.24), face_pt(hw, y1, hs - 0.24), face_pt(-hw, y1, hs - 0.24), wn, belfry_col);
            // Warm Colusa-sandstone architectural uplight wash on Giralda tower upper cornices & belfry piers in Role::Far ONLY
            // (matches Colusa sandstone in daylight, glows warm golden-amber across SF Bay in 06a_night_skyline; 0% drawn in Near `08`!).
            push_q(&mut uplight_far, face_pt(-hs - 0.14, base + sy - 0.4, hs + 0.14), face_pt(hs + 0.14, base + sy - 0.4, hs + 0.14), face_pt(hs + 0.14, base + sy + 1.3, hs + 0.14), face_pt(-hs - 0.14, base + sy + 1.3, hs + 0.14), wn, uplight_col);
            sy = stg.top_m;
        }
        // Subtle warm uplight cornice bands on the main Giralda clock-tower shaft (Role::Far only, framing the ivory clock face!).
        for band_y in [base + shaft_top - 1.4, base + shaft_top - 12.0, base + shaft_top - 22.0] {
            push_q(&mut uplight_far, face_pt(-h0 - 0.14, band_y, h0 + 0.14), face_pt(h0 + 0.14, band_y, h0 + 0.14), face_pt(h0 + 0.14, band_y + 1.5, h0 + 0.14), face_pt(-h0 - 0.14, band_y + 1.5, h0 + 0.14), wn, uplight_col);
        }
    }

    // 3. Open ground arcade marketplace glow & Great Nave roof skylight glow.
    let bay_n = [n[0] * -1.0, 0.0, n[1] * -1.0];
    let plaza_n = [n[0], 0.0, n[1]];
    let (u0, u1) = (4.5, length - 4.5);
    // Near: deeply recessed inside 4.0 m open colonnade (`d = -4.0 m`, untouched daytime look in `08`).
    push_q(&mut arcade_near, at_frame(u0, -depth + 4.0, base + 0.6), at_frame(u1, -depth + 4.0, base + 0.6), at_frame(u1, -depth + 4.0, base + 6.5), at_frame(u0, -depth + 4.0, base + 6.5), bay_n, arcade_col);
    if pu0 > 8.0 {
        push_q(&mut arcade_near, at_frame(4.5, -4.0, base + 0.6), at_frame(pu0 - 2.0, -4.0, base + 0.6), at_frame(pu0 - 2.0, -4.0, base + 6.5), at_frame(4.5, -4.0, base + 6.5), plaza_n, arcade_col);
    }
    if length - pu1 > 8.0 {
        push_q(&mut arcade_near, at_frame(pu1 + 2.0, -4.0, base + 0.6), at_frame(length - 4.5, -4.0, base + 0.6), at_frame(length - 4.5, -4.0, base + 6.5), at_frame(pu1 + 2.0, -4.0, base + 6.5), plaza_n, arcade_col);
    }
    // Far (`06a_night_skyline` at 1.15 km, 0% drawn in `08` Near!): unoccluded warm marketplace arches & upper nave arches
    // cleanly set 22 cm inside the bay-side wall (`-depth + 0.22`) so stone arcade piers frame the warm golden light across all 22 bays!
    push_q(&mut arcade_far, at_frame(u0, -depth + 0.22, base + 0.8), at_frame(u1, -depth + 0.22, base + 0.8), at_frame(u1, -depth + 0.22, base + 6.4), at_frame(u0, -depth + 0.22, base + 6.4), bay_n, arcade_col);
    push_q(
        &mut arcade_far,
        at_frame(u0, -depth + 0.22, base + inf.upper_sill_m + 0.15),
        at_frame(u1, -depth + 0.22, base + inf.upper_sill_m + 0.15),
        at_frame(u1, -depth + 0.22, base + inf.upper_spring_m + inf.upper_arch_width_m * 0.40),
        at_frame(u0, -depth + 0.22, base + inf.upper_spring_m + inf.upper_arch_width_m * 0.40),
        bay_n,
        arcade_col,
    );
    // Great Nave roof monitor clerestory glow along both long faces (Near & Far).
    let inset = (depth - inf.roof_monitor_width_m) * 0.5 + 0.24;
    let (my0, my1) = (base + inf.roof_monitor_foot_m + 0.10, base + inf.roof_clerestory_top_m - 0.10);
    for dst in [&mut arcade_near, &mut arcade_far] {
        push_q(dst, at_frame(inset, -depth + inset, my0), at_frame(length - inset, -depth + inset, my0), at_frame(length - inset, -depth + inset, my1), at_frame(inset, -depth + inset, my1), bay_n, arcade_col);
        push_q(dst, at_frame(inset, -inset, my0), at_frame(length - inset, -inset, my0), at_frame(length - inset, -inset, my1), at_frame(inset, -inset, my1), plaza_n, arcade_col);
    }

    // 4. Four subtle maritime pierhead navigation beacon lanterns on Pier 14 & Ferry Plaza outer tips
    // (pure `pattern="screen"` Fresnel lanterns — 0 LandmarkLights so tower aviation beacon tests stay exact!).
    let deck_y = base.max(2.6);
    let beacons: [([f64; 2], bool); 4] = [
        (sc([2613.0, -1828.0]), true),  // Pier 14 bayward tip — ruby-red port beacon
        (sc([2518.0, -1828.5]), false), // Ferry Plaza south gate tip — emerald-green starboard beacon
        (sc([2444.5, -2014.0]), true),  // Ferry Plaza central pierhead — ruby-red port beacon
        (sc([2308.5, -2095.5]), false), // Ferry Plaza north gate tip — emerald-green starboard beacon
    ];
    for ([bx, bz], is_red) in beacons {
        beacon_post_near.box_(bx, bz, deck_y - 0.4, deck_y + 2.4, 0.14, 0.14, 0.0, post_col, crate::geometry::ALL);
        if is_red {
            beacon_red_near.box_(bx, bz, deck_y + 2.4, deck_y + 2.95, 0.22, 0.22, 0.0, red_col, crate::geometry::ALL);
            beacon_red_far.box_(bx, bz, deck_y + 2.2, deck_y + 3.4, 0.48, 0.48, 0.0, red_col, crate::geometry::ALL);
        } else {
            beacon_green_near.box_(bx, bz, deck_y + 2.4, deck_y + 2.95, 0.22, 0.22, 0.0, green_col, crate::geometry::ALL);
            beacon_green_far.box_(bx, bz, deck_y + 2.2, deck_y + 3.4, 0.48, 0.48, 0.0, green_col, crate::geometry::ALL);
        }
    }

    let clock_mat = facade_material_at("sf-ferry-clock-glow", 0.45, 0.0, base);
    let arcade_mat = facade_material_at("sf-ferry-arcade-glow", 0.55, 0.0, base);
    let uplight_mat = if data().facades.contains_key("sf-ferry-tower-uplight") {
        facade_material_at("sf-ferry-tower-uplight", 0.72, 0.0, base)
    } else {
        arcade_mat.clone()
    };
    let red_mat = if data().facades.contains_key("sf-beacon-red") {
        facade_material_at("sf-beacon-red", 0.35, 0.0, base)
    } else {
        clock_mat.clone()
    };
    let green_mat = if data().facades.contains_key("sf-beacon-green") {
        facade_material_at("sf-beacon-green", 0.35, 0.0, base)
    } else {
        clock_mat.clone()
    };

    let draws = vec![
        Draw { key: "clock-night".into(), role: Role::Near, material: clock_mat.clone(), mesh: clock_near.freeze() },
        Draw { key: "arcade-night".into(), role: Role::Near, material: arcade_mat.clone(), mesh: arcade_near.freeze() },
        Draw { key: "beacon-post".into(), role: Role::Near, material: roof_trim_material(), mesh: beacon_post_near.freeze() },
        Draw { key: "beacon-red".into(), role: Role::Near, material: red_mat.clone(), mesh: beacon_red_near.freeze() },
        Draw { key: "beacon-green".into(), role: Role::Near, material: green_mat.clone(), mesh: beacon_green_near.freeze() },
        Draw { key: "far-clock-night".into(), role: Role::Far, material: clock_mat, mesh: clock_far.freeze() },
        Draw { key: "far-arcade-night".into(), role: Role::Far, material: arcade_mat, mesh: arcade_far.freeze() },
        Draw { key: "far-uplight-night".into(), role: Role::Far, material: uplight_mat, mesh: uplight_far.freeze() },
        Draw { key: "far-beacon-red".into(), role: Role::Far, material: red_mat, mesh: beacon_red_far.freeze() },
        Draw { key: "far-beacon-green".into(), role: Role::Far, material: green_mat, mesh: beacon_green_far.freeze() },
    ];
    let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
    for d in &draws {
        for p in d.mesh.positions.chunks_exact(3) {
            for k in 0..3 {
                mn[k] = mn[k].min(p[k]);
                mx[k] = mx[k].max(p[k]);
            }
        }
    }
    Some(Landmark {
        id: "waterfront-ferry-night".into(),
        name: "Ferry Building Night Illumination".into(),
        version: data().version.clone(),
        anchor: tc_world,
        box_min: mn,
        box_max: mx,
        draws,
        colliders: vec![],
        stats: json!({ "baseY": base, "kind": "ferry-night" }),
        build_ms: 0.0,
    })
}

/// Build sculpted San Francisco Bay vessels moored in verified open-water slips beside
/// Ferry Plaza (Gate B/C, North Gate) and Pier 14 so the Embarcadero waterfront reads as
/// an active working harbour at mid-range and street level (`lights` empty so tower beacon counts stay exact).
pub fn build_moored_bay_vessels(sea_y: f64) -> Vec<Landmark> {
    let mut out = Vec::new();
    let metal_mat = plain_material(0.34, 0.58, "facade-metal", None);
    let hull_mat = plain_material(0.56, 0.06, "facade-masonry", None);
    let glass_mat = plain_material(0.05, 0.35, "facade-glass", None);

    // Helper: assemble a sleek twin-hull SF Bay Catamaran Commuter Ferry with real glossy panoramic glass & mullions.
    let build_ferry = |id: &str, name: &str, c: [f64; 2], yaw_deg: f64, len: f64, beam: f64, stripe_hex: &str| -> Landmark {
        let yaw = yaw_deg.to_radians();
        let (co, si) = (yaw.cos(), yaw.sin());
        let at = |u: f64, v: f64| [c[0] + u * co - v * si, c[1] + u * si + v * co];
        let mut body_near = MeshArrays::new(&format!("{id}:hull"));
        let mut body_far = MeshArrays::new(&format!("{id}:far-hull"));
        let mut glass_near = MeshArrays::new(&format!("{id}:glass"));
        let mut glass_far = MeshArrays::new(&format!("{id}:far-glass"));
        let mut metal_near = MeshArrays::new(&format!("{id}:metal"));
        let mut metal_far = MeshArrays::new(&format!("{id}:far-metal"));

        let white = hex_linear("#d6d4ce");
        let deck_grey = hex_linear("#6e757b");
        let anti_foul = hex_linear("#3b2323");
        let stripe = hex_linear(stripe_hex);
        let win_glass = [0.055, 0.082, 0.112];
        let rail_c = hex_linear("#c4c9ce");
        let hl = len * 0.5;
        let hb = beam * 0.5;

        // Twin catamaran demihulls with flared pointed wave-piercing bows.
        for sgn in [-1.0, 1.0] {
            let v_off = sgn * (hb * 0.68);
            let hw = hb * 0.27;
            let bot_pts = [
                [-hl * 0.92, v_off - hw * 0.78],
                [hl * 0.54, v_off - hw * 0.78],
                [hl * 0.94, v_off],
                [hl * 0.54, v_off + hw * 0.78],
                [-hl * 0.92, v_off + hw * 0.78],
            ];
            let top_pts = [
                [-hl * 0.95, v_off - hw],
                [hl * 0.62, v_off - hw],
                [hl * 1.02, v_off],
                [hl * 0.62, v_off + hw],
                [-hl * 0.95, v_off + hw],
            ];
            let br = local_ring(c, yaw, &bot_pts);
            let tr = local_ring(c, yaw, &top_pts);
            body_near.loft(&br, sea_y - 0.85, &tr, sea_y + 0.38, anti_foul, false);
            body_near.prism(&tr, sea_y + 0.38, sea_y + 1.85, white, true, false);
            body_far.prism(&tr, sea_y - 0.5, sea_y + 1.85, white, true, false);
        }

        // Main bridging deck platform + painted SF Bay Ferry livery stripe.
        let deck_pts = [
            [-hl * 0.94, -hb],
            [hl * 0.68, -hb],
            [hl * 0.94, -hb * 0.48],
            [hl * 0.94, hb * 0.48],
            [hl * 0.68, hb],
            [-hl * 0.94, hb],
        ];
        let dr = local_ring(c, yaw, &deck_pts);
        body_near.prism(&dr, sea_y + 1.40, sea_y + 2.18, stripe, true, false);
        body_near.prism(&scale_about(&dr, c, 0.97), sea_y + 2.18, sea_y + 2.26, deck_grey, true, false);
        body_far.prism(&dr, sea_y + 1.45, sea_y + 2.22, stripe, true, false);

        // Main passenger cabin with glossy sky-reflecting panoramic windows + crisp white mullions.
        let cab1_pts = [
            [-hl * 0.70, -hb * 0.86],
            [hl * 0.44, -hb * 0.86],
            [hl * 0.68, -hb * 0.56],
            [hl * 0.68, hb * 0.56],
            [hl * 0.44, hb * 0.86],
            [-hl * 0.70, hb * 0.86],
        ];
        let c1r = local_ring(c, yaw, &cab1_pts);
        let c1_top = scale_about(&c1r, c, 0.96);
        body_near.prism(&c1r, sea_y + 2.25, sea_y + 2.95, white, false, false);
        glass_near.loft(&c1r, sea_y + 2.95, &c1_top, sea_y + 4.35, win_glass, false);
        glass_far.loft(&c1r, sea_y + 2.95, &c1_top, sea_y + 4.35, win_glass, false);
        body_near.prism(&scale_about(&c1_top, c, 1.03), sea_y + 4.35, sea_y + 4.78, white, true, false);
        body_far.prism(&c1_top, sea_y + 4.35, sea_y + 4.78, white, true, false);
        for sgn in [-1.0, 1.0] {
            for k in -4..=2 {
                let mp = at(k as f64 * (len * 0.085), sgn * hb * 0.855);
                body_near.box_(mp[0], mp[1], sea_y + 2.90, sea_y + 4.40, 0.14, 0.10, yaw, white, crate::geometry::NO_BOTTOM);
            }
        }

        // Upper passenger saloon & forward angled wheelhouse bridge.
        let cab2_pts = [
            [-hl * 0.46, -hb * 0.74],
            [hl * 0.30, -hb * 0.74],
            [hl * 0.50, -hb * 0.46],
            [hl * 0.50, hb * 0.46],
            [hl * 0.30, hb * 0.74],
            [-hl * 0.46, hb * 0.74],
        ];
        let c2r = local_ring(c, yaw, &cab2_pts);
        let c2_top = scale_about(&c2r, c, 0.94);
        body_near.prism(&c2r, sea_y + 4.78, sea_y + 5.30, white, false, false);
        glass_near.loft(&c2r, sea_y + 5.30, &c2_top, sea_y + 6.65, win_glass, false);
        glass_far.loft(&c2r, sea_y + 5.30, &c2_top, sea_y + 6.65, win_glass, false);
        body_near.prism(&scale_about(&c2_top, c, 1.04), sea_y + 6.65, sea_y + 7.05, white, true, false);
        body_far.prism(&c2_top, sea_y + 6.65, sea_y + 7.05, white, true, false);
        let br_pts = [
            [hl * 0.04, -hb * 0.54],
            [hl * 0.44, -hb * 0.52],
            [hl * 0.52, -hb * 0.34],
            [hl * 0.52, hb * 0.34],
            [hl * 0.44, hb * 0.52],
            [hl * 0.04, hb * 0.54],
        ];
        let br_bot = local_ring(c, yaw, &br_pts);
        let br_top = scale_about(&br_bot, at(hl * 0.26, 0.0), 0.90);
        body_near.prism(&br_bot, sea_y + 7.05, sea_y + 7.42, white, false, false);
        glass_near.loft(&br_bot, sea_y + 7.42, &br_top, sea_y + 8.32, win_glass, false);
        body_near.prism(&scale_about(&br_top, at(hl * 0.26, 0.0), 1.07), sea_y + 8.32, sea_y + 8.62, white, true, false);
        body_far.prism(&br_bot, sea_y + 7.05, sea_y + 8.60, white, true, false);

        // Metallic safety rails around decks + swept twin exhaust stacks + radar mast.
        for sgn in [-1.0, 1.0] {
            let r0 = at(-hl * 0.88, sgn * hb * 0.90);
            let r1 = at(hl * 0.62, sgn * hb * 0.90);
            metal_near.beam([r0[0], sea_y + 3.15, r0[1]], [r1[0], sea_y + 3.15, r1[1]], 0.07, 0.07, rail_c);
            let u0 = at(-hl * 0.68, sgn * hb * 0.80);
            let u1 = at(-hl * 0.46, sgn * hb * 0.80);
            metal_near.beam([u0[0], sea_y + 5.65, u0[1]], [u1[0], sea_y + 5.65, u1[1]], 0.07, 0.07, rail_c);
            let s_pos = at(-hl * 0.56, sgn * hb * 0.58);
            metal_near.box_(s_pos[0], s_pos[1], sea_y + 4.78, sea_y + 7.30, 0.38, 0.30, yaw, stripe, crate::geometry::NO_BOTTOM);
        }
        let mast_p = at(hl * 0.18, 0.0);
        metal_near.beam([mast_p[0], sea_y + 8.60, mast_p[1]], [mast_p[0] - co * 1.4, sea_y + 12.2, mast_p[1] - si * 1.4], 0.16, 0.16, rail_c);
        metal_near.box_(mast_p[0] - co * 1.0, mast_p[1] - si * 1.0, sea_y + 11.1, sea_y + 11.35, 0.14, 1.35, yaw, rail_c, crate::geometry::ALL);
        metal_far.beam([mast_p[0], sea_y + 8.60, mast_p[1]], [mast_p[0] - co * 1.4, sea_y + 12.2, mast_p[1] - si * 1.4], 0.26, 0.26, rail_c);

        let mut draws = vec![
            Draw { key: "hull".into(), role: Role::Near, material: hull_mat.clone(), mesh: body_near.freeze() },
            Draw { key: "glass".into(), role: Role::Near, material: glass_mat.clone(), mesh: glass_near.freeze() },
            Draw { key: "metal".into(), role: Role::Near, material: metal_mat.clone(), mesh: metal_near.freeze() },
            Draw { key: "far".into(), role: Role::Far, material: hull_mat.clone(), mesh: body_far.freeze() },
            Draw { key: "far-glass".into(), role: Role::Far, material: glass_mat.clone(), mesh: glass_far.freeze() },
            Draw { key: "far-metal".into(), role: Role::Far, material: metal_mat.clone(), mesh: metal_far.freeze() },
        ];
        let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
        for d in &draws {
            for p in d.mesh.positions.chunks_exact(3) {
                for k in 0..3 {
                    mn[k] = mn[k].min(p[k]);
                    mx[k] = mx[k].max(p[k]);
                }
            }
        }
        draws.retain(|d| !d.mesh.is_empty());
        Landmark {
            id: id.into(),
            name: name.into(),
            version: data().version.clone(),
            anchor: c,
            box_min: mn,
            box_max: mx,
            draws,
            colliders: vec![],
            stats: json!({ "seaY": sea_y, "kind": "bay-ferry" }),
            build_ms: 0.0,
        }
    };

    // Helper: sculpted marine-grade San Francisco Bay harbour tractor tug / pilot launch:
    // flared curved sheer boat hull, heavy wrap-around rubber fender + bow push-knees,
    // chamfered cream/navy deckhouse with portholes & lifebuoys, raked octagonal pilothouse
    // with real glossy sky-reflecting glass & corner mullions, tapered cylindrical funnel,
    // swept A-frame radar mast, safety railings, and aft towing H-bitt & winch.
    let build_tug = |id: &str, name: &str, c: [f64; 2], yaw_deg: f64| -> Landmark {
        let yaw = yaw_deg.to_radians();
        let (co, si) = (yaw.cos(), yaw.sin());
        let at = |u: f64, v: f64| [c[0] + u * co - v * si, c[1] + u * si + v * co];
        let mut body_near = MeshArrays::new(&format!("{id}:hull"));
        let mut body_far = MeshArrays::new(&format!("{id}:far-hull"));
        let mut glass_near = MeshArrays::new(&format!("{id}:glass"));
        let mut glass_far = MeshArrays::new(&format!("{id}:far-glass"));
        let mut metal_near = MeshArrays::new(&format!("{id}:metal"));
        let mut metal_far = MeshArrays::new(&format!("{id}:far-metal"));

        let hull_navy = hex_linear("#1e2630");
        let hull_red = hex_linear("#5b2520");
        let cabin_white = hex_linear("#cfc9bc");
        let cabin_trim = hex_linear("#263342");
        let deck_slate = hex_linear("#464c52");
        let fender_blk = hex_linear("#181a1d");
        let lifebuoy_org = hex_linear("#b64926");
        let funnel_red = hex_linear("#782922");
        let win_glass = [0.055, 0.085, 0.118];
        let mast_c = hex_linear("#c8c6be");
        let chrome_c = hex_linear("#9ea4aa");
        let (hl, hb) = (7.7, 2.65);

        // 1. Curved flared sheer boat hull (14-point smooth maritime hull stations, sweeping bow sheer!).
        let stations: [(f64, f64); 14] = [
            (-hl * 0.96, -hb * 0.56),
            (-hl * 0.80, -hb * 0.86),
            (-hl * 0.45, -hb * 0.98),
            (hl * 0.15, -hb * 1.00),
            (hl * 0.56, -hb * 0.86),
            (hl * 0.84, -hb * 0.52),
            (hl * 1.00, 0.0),
            (hl * 0.84, hb * 0.52),
            (hl * 0.56, hb * 0.86),
            (hl * 0.15, hb * 1.00),
            (-hl * 0.45, hb * 0.98),
            (-hl * 0.80, hb * 0.86),
            (-hl * 0.96, hb * 0.56),
            (-hl * 1.00, 0.0),
        ];
        let sheer_y = |u: f64| -> f64 {
            let t = ((u + hl) / (2.0 * hl)).clamp(0.0, 1.0);
            sea_y + 1.28 + 0.54 * (t - 0.34).max(0.0).powf(1.8) * 1.65 + 0.14 * (0.34 - t).max(0.0).powf(1.5)
        };
        let to_world_ring = |scale_u: f64, scale_v: f64| -> Vec<[f64; 2]> {
            orient_ring(&stations.iter().map(|&(u, v)| at(u * scale_u, v * scale_v)).collect::<Vec<_>>())
        };
        let keel_r = to_world_ring(0.84, 0.72);
        let wl_r = to_world_ring(0.94, 0.92);
        let gunwale_r = to_world_ring(1.00, 1.00);
        let inner_r = to_world_ring(0.93, 0.91);
        body_near.loft(&keel_r, sea_y - 0.65, &wl_r, sea_y + 0.28, hull_red, false);
        let n_st = gunwale_r.len();
        for i in 0..n_st {
            let j = (i + 1) % n_st;
            let (wa, wb) = (wl_r[i], wl_r[j]);
            let (ga, gb) = (gunwale_r[i], gunwale_r[j]);
            let ua = (ga[0] - c[0]) * co + (ga[1] - c[1]) * si;
            let ub = (gb[0] - c[0]) * co + (gb[1] - c[1]) * si;
            let (ya, yb) = (sheer_y(ua), sheer_y(ub));
            body_near.quad([wa[0], sea_y + 0.28, wa[1]], [wb[0], sea_y + 0.28, wb[1]], [gb[0], yb, gb[1]], [ga[0], ya, ga[1]], hull_navy);
            body_near.quad([ga[0], ya, ga[1]], [gb[0], yb, gb[1]], [gb[0], yb + 0.42, gb[1]], [ga[0], ya + 0.42, ga[1]], hull_navy);
            body_near.beam([ga[0], ya + 0.44, ga[1]], [gb[0], yb + 0.44, gb[1]], 0.12, 0.08, cabin_white);
            body_near.beam([ga[0], ya - 0.14, ga[1]], [gb[0], yb - 0.14, gb[1]], 0.24, 0.26, fender_blk);
            let (ia, ib) = (inner_r[i], inner_r[j]);
            body_near.quad([ib[0], sea_y + 1.24, ib[1]], [ia[0], sea_y + 1.24, ia[1]], [ga[0], ya + 0.40, ga[1]], [gb[0], yb + 0.40, gb[1]], deck_slate);
        }
        cap(&mut body_near, &inner_r, sea_y + 1.24, deck_slate);
        body_far.loft(&wl_r, sea_y - 0.3, &gunwale_r, sea_y + 1.50, hull_navy, true);

        // Heavy chamfered twin rubber bow push-knees + cylindrical side tyre fenders.
        for sgn in [-1.0, 1.0] {
            let pk = at(hl * 0.92, sgn * 0.56);
            body_near.box_(pk[0], pk[1], sea_y + 0.65, sea_y + 2.35, 0.42, 0.28, yaw, fender_blk, crate::geometry::ALL);
            for k in -2..=1 {
                let tp = at(k as f64 * 2.15 + 0.3, sgn * (hb * 1.02));
                body_near.box_(tp[0], tp[1], sea_y + 0.55, sea_y + 1.18, 0.34, 0.18, yaw, fender_blk, crate::geometry::ALL);
            }
        }

        // 2. Chamfered lower deckhouse with navy boot stripe, recessed dark portholes & orange lifebuoys.
        let dh_pts = [
            [-hl * 0.46, -hb * 0.64],
            [hl * 0.36, -hb * 0.64],
            [hl * 0.50, -hb * 0.44],
            [hl * 0.50, hb * 0.44],
            [hl * 0.36, hb * 0.64],
            [-hl * 0.46, hb * 0.64],
        ];
        let dhr = local_ring(c, yaw, &dh_pts);
        let dhr_top = scale_about(&dhr, at(0.2, 0.0), 0.96);
        body_near.prism(&dhr, sea_y + 1.24, sea_y + 1.62, cabin_trim, false, false);
        body_near.loft(&dhr, sea_y + 1.62, &dhr_top, sea_y + 3.05, cabin_white, false);
        body_near.prism(&scale_about(&dhr_top, at(0.2, 0.0), 1.04), sea_y + 3.05, sea_y + 3.22, deck_slate, true, false);
        body_far.prism(&dhr, sea_y + 1.30, sea_y + 3.20, cabin_white, true, false);
        for sgn in [-1.0, 1.0] {
            for pu in [-1.8, -0.3, 1.2] {
                let pp = at(pu, sgn * (hb * 0.64 + 0.02));
                glass_near.box_(pp[0], pp[1], sea_y + 2.18, sea_y + 2.58, 0.24, 0.06, yaw, win_glass, crate::geometry::ALL);
            }
            let lb = at(-2.6, sgn * (hb * 0.64 + 0.06));
            body_near.box_(lb[0], lb[1], sea_y + 2.05, sea_y + 2.65, 0.28, 0.07, yaw, lifebuoy_org, crate::geometry::ALL);
        }

        // 3. Raked octagonal pilothouse bridge with REAL glossy sky-reflecting wraparound windows + corner mullions + angled visor!
        let wh_c = at(hl * 0.14, 0.0);
        let wh_pts = [
            [-hl * 0.10, -hb * 0.50],
            [hl * 0.28, -hb * 0.50],
            [hl * 0.42, -hb * 0.30],
            [hl * 0.42, hb * 0.30],
            [hl * 0.28, hb * 0.50],
            [-hl * 0.10, hb * 0.50],
        ];
        let wh_base = local_ring(c, yaw, &wh_pts);
        let wh_sill = scale_about(&wh_base, wh_c, 0.98);
        let wh_head = scale_about(&wh_base, wh_c, 1.05);
        let wh_visor = scale_about(&wh_head, wh_c, 1.09);
        body_near.loft(&wh_base, sea_y + 3.22, &wh_sill, sea_y + 3.92, cabin_white, false);
        glass_near.loft(&wh_sill, sea_y + 3.92, &wh_head, sea_y + 4.86, win_glass, false);
        glass_far.loft(&wh_sill, sea_y + 3.92, &wh_head, sea_y + 4.86, win_glass, false);
        body_near.loft(&wh_head, sea_y + 4.86, &wh_visor, sea_y + 5.14, cabin_white, false);
        cap(&mut body_near, &wh_visor, sea_y + 5.14, deck_slate);
        body_far.prism(&wh_base, sea_y + 3.22, sea_y + 5.12, cabin_white, true, false);
        for pt in &wh_head {
            let dir = [pt[0] - wh_c[0], pt[1] - wh_c[1]];
            let dl = dir[0].hypot(dir[1]).max(1e-6);
            let mx = pt[0] + dir[0] / dl * 0.02;
            let mz = pt[1] + dir[1] / dl * 0.02;
            body_near.box_(mx, mz, sea_y + 3.90, sea_y + 4.90, 0.09, 0.09, yaw, cabin_white, crate::geometry::NO_BOTTOM);
        }

        // 4. Sculpted 12-sided tapered oval smokestack funnel (SF Bay red & navy livery + twin chrome exhaust pipes!).
        let fn_c = at(-hl * 0.28, 0.0);
        let fn_r0 = ellipse_ring(fn_c, 0.78, 0.54, 12, yaw);
        let fn_r1 = ellipse_ring(fn_c, 0.66, 0.45, 12, yaw);
        let fn_r2 = ellipse_ring(fn_c, 0.62, 0.42, 12, yaw);
        let fn_r3 = ellipse_ring(fn_c, 0.58, 0.39, 12, yaw);
        body_near.loft(&fn_r0, sea_y + 3.20, &fn_r1, sea_y + 5.15, funnel_red, false);
        body_near.loft(&fn_r1, sea_y + 5.15, &fn_r2, sea_y + 5.50, cabin_white, false);
        body_near.loft(&fn_r2, sea_y + 5.50, &fn_r3, sea_y + 6.05, hull_navy, false);
        cap(&mut body_near, &fn_r3, sea_y + 6.05, fender_blk);
        body_far.loft(&fn_r0, sea_y + 3.20, &fn_r3, sea_y + 6.05, funnel_red, true);
        for sgn in [-0.20, 0.20] {
            let ex = at(-hl * 0.28, sgn);
            metal_near.box_(ex[0], ex[1], sea_y + 6.00, sea_y + 6.55, 0.09, 0.09, yaw, chrome_c, crate::geometry::NO_BOTTOM);
        }

        // 5. Swept A-frame radar mast, radar scanner bar, bridge safety railings & aft towing winch/H-bitt!
        let m_top = at(hl * 0.02, 0.0);
        for sgn in [-1.0, 1.0] {
            let leg0 = at(hl * 0.14, sgn * 0.72);
            metal_near.beam([leg0[0], sea_y + 5.14, leg0[1]], [m_top[0], sea_y + 8.55, m_top[1]], 0.10, 0.10, mast_c);
            let ra = at(-hl * 0.44, sgn * hb * 0.62);
            let rb = at(hl * 0.12, sgn * hb * 0.62);
            metal_near.beam([ra[0], sea_y + 3.95, ra[1]], [rb[0], sea_y + 3.95, rb[1]], 0.05, 0.05, mast_c);
            metal_near.beam([ra[0], sea_y + 3.60, ra[1]], [rb[0], sea_y + 3.60, rb[1]], 0.04, 0.04, mast_c);
        }
        metal_near.beam([m_top[0], sea_y + 8.45, m_top[1]], [m_top[0], sea_y + 9.45, m_top[1]], 0.07, 0.07, mast_c);
        let yard_l = at(hl * 0.02, -1.15);
        let yard_r = at(hl * 0.02, 1.15);
        metal_near.beam([yard_l[0], sea_y + 7.75, yard_l[1]], [yard_r[0], sea_y + 7.75, yard_r[1]], 0.06, 0.06, mast_c);
        let rdr = at(hl * 0.14, 0.0);
        metal_near.box_(rdr[0], rdr[1], sea_y + 6.95, sea_y + 7.12, 0.12, 0.68, yaw, cabin_white, crate::geometry::ALL);
        metal_far.beam([wh_c[0], sea_y + 5.14, wh_c[1]], [m_top[0], sea_y + 9.20, m_top[1]], 0.18, 0.18, mast_c);
        let wb = at(-hl * 0.64, 0.0);
        metal_near.box_(wb[0], wb[1], sea_y + 1.24, sea_y + 2.05, 0.58, 0.85, yaw, hull_navy, crate::geometry::NO_BOTTOM);
        for sgn in [-0.45, 0.45] {
            let hb_p = at(-hl * 0.80, sgn);
            metal_near.box_(hb_p[0], hb_p[1], sea_y + 1.24, sea_y + 2.22, 0.14, 0.14, yaw, hull_navy, crate::geometry::NO_BOTTOM);
        }
        let hbl = at(-hl * 0.80, -0.58);
        let hbr = at(-hl * 0.80, 0.58);
        metal_near.beam([hbl[0], sea_y + 1.98, hbl[1]], [hbr[0], sea_y + 1.98, hbr[1]], 0.14, 0.14, hull_navy);

        let draws = vec![
            Draw { key: "hull".into(), role: Role::Near, material: hull_mat.clone(), mesh: body_near.freeze() },
            Draw { key: "glass".into(), role: Role::Near, material: glass_mat.clone(), mesh: glass_near.freeze() },
            Draw { key: "metal".into(), role: Role::Near, material: metal_mat.clone(), mesh: metal_near.freeze() },
            Draw { key: "far".into(), role: Role::Far, material: hull_mat.clone(), mesh: body_far.freeze() },
            Draw { key: "far-glass".into(), role: Role::Far, material: glass_mat.clone(), mesh: glass_far.freeze() },
            Draw { key: "far-metal".into(), role: Role::Far, material: metal_mat.clone(), mesh: metal_far.freeze() },
        ];
        let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
        for d in &draws {
            for p in d.mesh.positions.chunks_exact(3) {
                for k in 0..3 {
                    mn[k] = mn[k].min(p[k]);
                    mx[k] = mx[k].max(p[k]);
                }
            }
        }
        Landmark {
            id: id.into(),
            name: name.into(),
            version: data().version.clone(),
            anchor: c,
            box_min: mn,
            box_max: mx,
            draws,
            colliders: vec![],
            stats: json!({ "seaY": sea_y, "kind": "bay-tug" }),
            build_ms: 0.0,
        }
    };

    out.push(build_ferry("waterfront-vessel-ferry-south", "SF Bay Commuter Ferry (Ferry Plaza Gate C)", [2484.0, -1886.0], -36.0, 34.0, 10.2, "#295472"));
    out.push(build_ferry("waterfront-vessel-ferry-north", "SF Bay Commuter Ferry (Ferry Plaza North)", [2326.0, -2062.0], -36.4, 31.0, 9.6, "#365c6b"));
    out.push(build_tug("waterfront-vessel-tug-pier14", "SF Bay Harbour Tug (Pier 14 Slip)", [2476.0, -1808.0], -34.0));
    out
}
