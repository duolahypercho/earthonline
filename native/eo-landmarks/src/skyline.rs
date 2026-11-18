//! The named downtown towers (555 California, the four Embarcadero Center
//! slabs, 345 California, Millennium Tower, 181 Fremont) drawn as authored
//! landmarks in place of the generic kit facade, and the authored facade
//! styles the landmark buildings' surface kind 7 draws.
//!
//! Every value comes from `data/skyline-buildings.json`, each entry with its
//! published source (heights, floors, materials, colours) or marked INFERRED.
//! A tower's plan is its own footprint record in public/data/sf/sf-city.json
//! (`detailBuildings`, by `record`): the generator extrudes it in the
//! entry's tiers (an Embarcadero Center slab steps down at its ends), caps it
//! with the entry's crown (a roof parapet and penthouse, 345 California's
//! twin towers turned 45 degrees with their spires, a mast, 181 Fremont's
//! screen and spire) and draws its walls in the entry's facade style. Like
//! the registry's landmark buildings it is built at its real site and size
//! on the ground of its scaled site, then moved whole (`set::ground_scale_shift`).
//! The kit draws none of these records while the skyline is on
//! (`set::claimed_records_with`); `eo-app --no-skyline` draws them as before.

use crate::LandmarkLight;
use crate::data::Material;
use crate::geometry::{MeshArrays, Rgb, V3, orient_ring, ring_centroid, srgb_hex};
use crate::set::{Draw, Landmark, Role};
use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
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
}

fn one() -> f64 {
    1.0
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Tier {
    /// Top of the tier, a fraction of `heightM`.
    pub to: f64,
    /// The central share of the plan's length (along its principal axis) the tier keeps.
    pub along_frac: f64,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub record: String,
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
    pub aviation: bool,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Skyline {
    pub version: String,
    pub facades: BTreeMap<String, FacadeStyle>,
    pub floodlight: Value,
    pub buildings: Vec<Entry>,
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

/// Each facade style as six rows of the WGSL table: (module, storey,
/// glaze x, glaze y), (glass linear rgb, glass roughness), (pattern code,
/// night lit share, led, frame depth), (v3: the pane's metalness, its tilt
/// in radians, the night bay in modules, a lit window's cd/m2), (v4: the
/// spandrel's tone, the reveal shadow 0/1, the polished share of a pier,
/// its roughness), (v4: the honed rest's tone and roughness, 0, 0). Pattern 0
/// a window grid, 1 an open screen. With a `reflectance` the glass row is
/// the pane's albedo (`pane_albedo_metal`, the tint's chroma at the
/// reflectance) and its metalness in the fourth; without one the tint as
/// before. The fifth and sixth rows are what the fourth pass reads (the
/// earlier passes' shaders never do).
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
                0.0,
                0.0,
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

/// The floodlights' colour: `floodlight.colourK` as an sRGB tone at unit
/// luminance. 3,500 K is #ffc489 in Mitchell Charity's blackbody colour
/// table (vendian.org, "What colour is a blackbody?", CIE 1931 2-degree, D65 white).
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

/// A plain material (no surface kind), optionally floodlit.
pub fn plain_material(roughness: f64, metalness: f64, env: &str, flood: Option<[f64; 3]>) -> Material {
    Material { roughness, metalness, env_class: env.into(), surface: None, draw: None, wire: false, surface_options: flood.map(|f| json!({ "flood": f })) }
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
    let want: Vec<&str> = data().buildings.iter().map(|b| b.record.as_str()).collect();
    let mut out = BTreeMap::new();
    for r in city.detail_buildings {
        let id = match &r.id {
            Value::Number(n) => n.to_string(),
            Value::String(s) => s.clone(),
            _ => continue,
        };
        if want.contains(&id.as_str()) {
            out.insert(id, r.points);
        }
    }
    Ok(out)
}

/// The records the skyline draws (their kit groups draw nothing while it is on).
pub fn claims() -> Vec<String> {
    data().buildings.iter().map(|b| b.record.clone()).collect()
}

fn ring_of(points: &[f64]) -> Vec<[f64; 2]> {
    let mut r: Vec<[f64; 2]> = points.chunks_exact(2).map(|p| [p[0], p[1]]).collect();
    if r.len() > 2 && r[0] == r[r.len() - 1] {
        r.pop();
    }
    orient_ring(&r)
}

/// The plan's principal axis (unit) and its extent along it, about the centroid.
fn principal_axis(ring: &[[f64; 2]], c: [f64; 2]) -> ([f64; 2], f64, f64) {
    let (mut sxx, mut sxz, mut szz) = (0.0, 0.0, 0.0);
    for p in ring {
        let (x, z) = (p[0] - c[0], p[1] - c[1]);
        sxx += x * x;
        sxz += x * z;
        szz += z * z;
    }
    let ang = 0.5 * (2.0 * sxz).atan2(sxx - szz);
    let a = [ang.cos(), ang.sin()];
    let (mut lo, mut hi) = (f64::MAX, f64::MIN);
    for p in ring {
        let s = (p[0] - c[0]) * a[0] + (p[1] - c[1]) * a[1];
        lo = lo.min(s);
        hi = hi.max(s);
    }
    (a, lo, hi)
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
                let t = (edge - sp) / (sq - sp);
                out.push([p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t]);
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

/// Walls of a ring between y0 and y1 (no caps).
fn walls(m: &mut MeshArrays, ring: &[[f64; 2]], y0: f64, y1: f64, colour: Rgb) {
    m.prism(ring, y0, y1, colour, false, false);
}

/// A flat cap over a (possibly concave) ring at y, facing up (earcut).
pub fn cap(m: &mut MeshArrays, ring: &[[f64; 2]], y: f64, colour: Rgb) {
    let pts = orient_ring(ring);
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

fn scale_about(ring: &[[f64; 2]], c: [f64; 2], k: f64) -> Vec<[f64; 2]> {
    ring.iter().map(|p| [c[0] + (p[0] - c[0]) * k, c[1] + (p[1] - c[1]) * k]).collect()
}

/// A square of half side h about c, turned by `yaw` (radians from +x toward +z).
fn square(c: [f64; 2], h: f64, yaw: f64) -> Vec<[f64; 2]> {
    let (co, si) = (yaw.cos(), yaw.sin());
    [[-h, -h], [h, -h], [h, h], [-h, h]].iter().map(|[x, z]| [c[0] + x * co - z * si, c[1] + x * si + z * co]).collect()
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

/// v4: the skyline towers' metal parts: architectural aluminium cladding
/// carries a metallic coating (a fluoropolymer with metal flake: part mirror,
/// part diffuse), drawn at metalness 0.6 and roughness 0.35 (INFERRED).
pub const METAL4_ROUGHNESS: f64 = 0.35;
pub const METAL4_METALNESS: f64 = 0.6;

/// What one skyline tower adds to the set.
pub struct BuiltTower {
    pub landmark: Landmark,
    pub lights: Vec<LandmarkLight>,
}

/// Build one entry on its footprint record (`points`, unscaled) and ground
/// sampler (`ground_at`, the drawn ground of its scaled site at real-site coordinates).
pub fn build_tower(e: &Entry, points: &[f64], ground_at: &dyn Fn(f64, f64) -> f64) -> Option<BuiltTower> {
    let t0 = std::time::Instant::now();
    let ring = ring_of(points);
    if ring.len() < 3 {
        return None;
    }
    let c = ring_centroid(&ring);
    let base = ring.iter().map(|p| ground_at(p[0], p[1])).fold(f64::INFINITY, f64::min);
    let base = if base.is_finite() { base } else { 0.0 };
    let h = e.height_m;
    let stone = hex_linear(&e.stone_srgb);
    let roof_c = hex_linear(&e.roof_srgb);
    let style = data().facades.get(&e.facade)?;
    let glass = hex_linear(&style.glass_srgb);
    // The far item's tone: the facade's glass and stone by their areas.
    let gshare = style.glaze_x * style.glaze_y;
    let far_c: Rgb = [0, 1, 2].map(|k| stone[k] * (1.0 - gshare) + glass[k] * 0.55 * gshare);
    let (a, lo, hi) = principal_axis(&ring, c);
    let mid = (lo + hi) / 2.0;
    let half_len = (hi - lo) / 2.0;
    let mut facade = MeshArrays::new(&format!("{}:facade", e.id));
    let mut roof = MeshArrays::new(&format!("{}:roof", e.id));
    let mut metal = MeshArrays::new(&format!("{}:metal", e.id));
    let mut far = MeshArrays::new(&format!("{}:far", e.id));
    // The foot stands 0.5 m into the ground (a sloping street never shows under it).
    let mut y_prev = base - 0.5;
    let mut plans: Vec<(Vec<[f64; 2]>, f64)> = Vec::new();
    for t in &e.tiers {
        let plan = if t.along_frac >= 0.999 { ring.clone() } else { clip_slab(&ring, c, a, mid - half_len * t.along_frac, mid + half_len * t.along_frac) };
        if plan.len() < 3 {
            continue;
        }
        let y1 = base + h * t.to;
        walls(&mut facade, &plan, y_prev, y1, stone);
        walls(&mut far, &plan, y_prev, y1, far_c);
        cap(&mut roof, &plan, y1, roof_c);
        cap(&mut far, &plan, y1, roof_c);
        plans.push((plan, y1));
        y_prev = y1;
    }
    let (top_plan, top_y) = plans.last().cloned()?;
    let mut tip_y = top_y;
    let cr = &e.crown;
    let num = |k: &str| cr[k].as_f64().unwrap_or(0.0);
    match cr["kind"].as_str().unwrap_or("flat") {
        "flat" => {
            // A parapet (the walls carried up in the roof's finish) and a penthouse.
            let p = num("parapetM");
            if p > 0.0 {
                walls(&mut roof, &top_plan, top_y, top_y + p, stone.map(|v| v * 0.92));
                walls(&mut roof, &scale_about(&top_plan, ring_centroid(&top_plan), 0.985), top_y, top_y + p, roof_c);
            }
            let (pf, ph) = (num("penthouseFrac"), num("penthouseM"));
            if pf > 0.0 && ph > 0.0 {
                let pc = ring_centroid(&top_plan);
                let pr = scale_about(&top_plan, pc, pf);
                roof.prism(&pr, top_y, top_y + ph, stone.map(|v| v * 0.85), false, false);
                cap(&mut roof, &pr, top_y + ph, roof_c);
                far.prism(&pr, top_y, top_y + ph, far_c, false, false);
                tip_y = top_y + ph;
            }
            tip_y = tip_y.max(top_y + p);
        }
        "twin-spires" => {
            // Two square towers turned 45 degrees to the plan, at its ends, up to
            // the roof, each with a pyramidal spire.
            let side = num("towerFrac") * (hi - lo);
            let hh = side / 2.0;
            let off = half_len - hh * std::f64::consts::SQRT_2;
            let yaw = a[1].atan2(a[0]) + std::f64::consts::FRAC_PI_4;
            let roof_y = base + h;
            let spire_top = base + num("spireTopM");
            let spire_c = hex_linear(cr["spireSrgb"].as_str().unwrap_or("#b9bec2"));
            for sgn in [-1.0, 1.0] {
                let tc = [c[0] + a[0] * (mid + sgn * off), c[1] + a[1] * (mid + sgn * off)];
                let sq = square(tc, hh, yaw);
                walls(&mut facade, &sq, top_y, roof_y, stone);
                walls(&mut far, &sq, top_y, roof_y, far_c);
                // A cornice band, then the spire from a slightly smaller base.
                roof.prism(&square(tc, hh + 0.4, yaw), roof_y - 1.2, roof_y, stone.map(|v| v * 0.9), false, false);
                cap(&mut roof, &square(tc, hh + 0.4, yaw), roof_y, roof_c);
                pyramid(&mut metal, &square(tc, hh * 0.86, yaw), roof_y, spire_top, spire_c);
                pyramid(&mut far, &square(tc, hh * 0.86, yaw), roof_y, spire_top, spire_c);
            }
            tip_y = spire_top;
        }
        "spire" => {
            let p = num("parapetM");
            walls(&mut facade, &top_plan, top_y, top_y + p, stone);
            cap(&mut roof, &scale_about(&top_plan, ring_centroid(&top_plan), 0.97), top_y + 0.3, roof_c);
            let spire_top = base + num("spireTopM");
            let spire_c = hex_linear(cr["spireSrgb"].as_str().unwrap_or("#c0c4c7"));
            let pc = ring_centroid(&top_plan);
            metal.box_(pc[0], pc[1], top_y, spire_top, 0.55, 0.55, 0.0, spire_c, crate::geometry::ALL);
            tip_y = spire_top;
        }
        "screen-spire" => {
            // The curtain wall carried up as the mechanical screen, open over the roof; a tapering spire.
            let screen_top = base + num("screenTopM");
            walls(&mut facade, &top_plan, top_y, screen_top, stone);
            walls(&mut far, &top_plan, top_y, screen_top, far_c);
            let spire_top = base + num("spireTopM");
            let spire_c = hex_linear(cr["spireSrgb"].as_str().unwrap_or("#c8ccce"));
            let pc = ring_centroid(&top_plan);
            pyramid(&mut metal, &square(pc, 1.4, a[1].atan2(a[0])), screen_top - 6.0, spire_top, spire_c);
            tip_y = spire_top;
        }
        "screen-taper" => {
            // v3: the curtain wall carried up past the roof as the
            // mechanical screen, its faces leaning in to `screenTopScale` of
            // the roof's plan at its top (the tower's sloped crown), open
            // over the roof; a slender spire from the roof inside it.
            let screen_top = base + num("screenTopM");
            let k = cr["screenTopScale"].as_f64().unwrap_or(1.0).clamp(0.3, 1.0);
            let pc = ring_centroid(&top_plan);
            let top_ring = scale_about(&top_plan, pc, k);
            facade.loft(&top_plan, top_y, &top_ring, screen_top, stone, false);
            far.loft(&top_plan, top_y, &top_ring, screen_top, far_c, false);
            // the screen's inner face (seen down into the crown) and the roof inside it.
            roof.loft(&scale_about(&top_plan, pc, 0.985), top_y, &scale_about(&top_plan, pc, k * 0.985), screen_top, stone.map(|v| v * 0.6), false);
            cap(&mut roof, &scale_about(&top_plan, pc, 0.97), top_y + 0.3, roof_c);
            let spire_top = base + num("spireTopM");
            let spire_c = hex_linear(cr["spireSrgb"].as_str().unwrap_or("#c8ccce"));
            let half = cr["spireBaseHalfM"].as_f64().unwrap_or(1.0);
            pyramid(&mut metal, &square(pc, half, a[1].atan2(a[0])), top_y, spire_top, spire_c);
            pyramid(&mut far, &square(pc, half, a[1].atan2(a[0])), top_y, spire_top, spire_c);
            tip_y = spire_top;
        }
        _ => {}
    }
    if let Some(b) = &e.braces {
        // Diagonal braces across every long face, one X per `storeysPerBay` storeys.
        let span = b["storeysPerBay"].as_f64().unwrap_or(8.0) * style.storey_m;
        let (bw, bd) = (b["widthM"].as_f64().unwrap_or(0.9), b["depthM"].as_f64().unwrap_or(0.35));
        let brace_c = hex_linear(e.crown["spireSrgb"].as_str().unwrap_or("#c8ccce"));
        let n = ring.len();
        for i in 0..n {
            let (p, q) = (ring[i], ring[(i + 1) % n]);
            let (dx, dz) = (q[0] - p[0], q[1] - p[1]);
            let len = dx.hypot(dz);
            if len < 12.0 {
                continue;
            }
            // Outward normal of an edge of the outward-wound ring (orient_ring: its side quads face (-dz, dx)).
            let (nx, nz) = (-dz / len, dx / len);
            let o = 0.18 + if bd > 0.35 { bd / 2.0 - 0.175 } else { 0.0 };
            let at = |t: f64, y: f64| -> V3 { [p[0] + dx * t + nx * o, y, p[1] + dz * t + nz * o] };
            let bays = ((top_y - base) / span).floor().max(1.0) as i64;
            for k in 0..bays {
                let (ya, yb) = (base + k as f64 * span, base + (k + 1) as f64 * span);
                metal.beam_up(at(0.04, ya), at(0.96, yb), bw, bd, brace_c, Some([nx, 0.0, nz]));
                metal.beam_up(at(0.96, ya), at(0.04, yb), bw, bd, brace_c, Some([nx, 0.0, nz]));
            }
        }
    }
    let fa = facade_material(&e.facade, e.stone_roughness, 0.0, None);
    let mut draws = vec![
        Draw { key: "facade".into(), role: Role::Near, material: fa.clone(), mesh: facade.freeze() },
        Draw { key: "roof".into(), role: Role::Near, material: plain_material(0.9, 0.0, "facade-masonry", None), mesh: roof.freeze() },
    ];
    if !metal.indices.is_empty() {
        // v4: the spires and braces clad in coated aluminium: a metallic coating (metalness METAL4_METALNESS: its flake mirrors, its binder scatters) of the entries' v4 `spireSrgb`, aluminium's F0 weathered.
        let mm = if crate::towers4_on() { plain_material(METAL4_ROUGHNESS, METAL4_METALNESS, "facade-metal", None) } else { plain_material(0.35, 0.7, "facade-metal", None) };
        draws.push(Draw { key: "metal".into(), role: Role::Near, material: mm, mesh: metal.freeze() });
    }
    let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
    for d in &draws {
        for p in d.mesh.positions.chunks_exact(3) {
            for k in 0..3 {
                mn[k] = mn[k].min(p[k]);
                mx[k] = mx[k].max(p[k]);
            }
        }
    }
    // The far item: the tiers' prisms in the facade's material (its window
    // grid box-filters to the mean tone at that range; its windows light at night).
    draws.push(Draw { key: "far".into(), role: Role::Far, material: fa, mesh: far.freeze() });
    let mut lights = Vec::new();
    if e.aviation {
        lights.push(crate::lights::aviation(c, tip_y));
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
    for e in &data().buildings {
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
    Ok((out, lights))
}
