//! The authored kit's materials: the registry (exported from the JS by
//! the earlier browser version's exporter into `data/kit-material-specs.json`)
//! and a Rust port of the game's procedural map bake
//! (`src/world/buildings/authored/kit/material-bake.js`,
//! `bakeAuthoredSpecMaps`). The port runs the same operations in the same
//! order on f64, stores the same f32 intermediates, and is tested against
//! SHA-256 digests of the JS bake's bytes.
//!
//! Every GLB material named after a kit material (`<kit name>` or
//! `<kit name>@rrggbb`) is drawn with that kit material's baked albedo,
//! normal and ORM maps; that is what the JS runtime does (`dressScene` in
//! blender/runtime.js replaces those materials with the kit's).

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

pub const MAPS_VERSION: &str = "authored-kit-materials-v1";
/// The seed the Blender runtime's library uses (`createAuthoredMaterialLibrary({ seed: 'authored-slice-v1' })`).
pub const RUNTIME_SEED: &str = "authored-slice-v1";
/// Texels per side at the browser's 'high' quality.
pub const HIGH_RES: usize = 512;

const SPECS_JSON: &str = include_str!("../data/kit-material-specs.json");

#[derive(Clone, Debug, Deserialize)]
pub struct Physical {
    #[serde(default)]
    pub transparent: bool,
    #[serde(default = "one")]
    pub opacity: f64,
    /// three's clearcoat layer (`physical.clearcoat`, 0..1).
    #[serde(default)]
    pub clearcoat: f64,
}
fn one() -> f64 {
    1.0
}

#[derive(Clone, Debug, Deserialize)]
pub struct KitSpec {
    pub kind: String,
    #[serde(rename = "metresPerTile")]
    pub metres_per_tile: f64,
    #[serde(default)]
    pub height: f64,
    #[serde(rename = "worldTileM", default)]
    pub world_tile_m: Option<f64>,
    #[serde(rename = "normalStrength", default)]
    pub normal_strength: Option<f64>,
    #[serde(rename = "roughFlatten", default)]
    pub rough_flatten: Option<f64>,
    pub rough: f64,
    pub metal: f64,
    #[serde(rename = "baseTint", default)]
    pub base_tint: Option<String>,
    #[serde(rename = "mapsFrom", default)]
    pub maps_from: Option<String>,
    #[serde(default)]
    pub borrow: Option<String>,
    #[serde(rename = "atlasId", default)]
    pub atlas_id: Option<String>,
    /// the atlas cell's linear mean the JS divides its sample by.
    #[serde(rename = "atlasMeanLinear", default)]
    pub atlas_mean_linear: Option<[f64; 3]>,
    #[serde(default)]
    pub physical: Option<Physical>,
    #[serde(default)]
    pub p: Value,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ReferenceBake {
    pub name: String,
    pub seed: String,
    pub res: usize,
    pub albedo: String,
    pub normal: String,
    pub orm: String,
    pub sums: [u64; 3],
}

#[derive(Clone, Debug, Deserialize)]
struct SourceFile {
    path: String,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
struct SpecFile {
    schema: String,
    source: HashMap<String, SourceFile>,
    #[serde(rename = "mapsVersion")]
    maps_version: String,
    #[serde(rename = "referenceBakes")]
    reference_bakes: Vec<ReferenceBake>,
    specs: HashMap<String, KitSpec>,
}

pub struct KitLibrary {
    pub specs: HashMap<String, KitSpec>,
    pub reference_bakes: Vec<ReferenceBake>,
    /// JS source files this table was exported from, with their digests then.
    pub sources: Vec<(String, String)>,
}

impl KitLibrary {
    pub fn load() -> Result<Self> {
        let f: SpecFile = serde_json::from_str(SPECS_JSON).context("kit-material-specs.json")?;
        if f.schema != "eo-native-kit-material-specs-v1" || f.maps_version != MAPS_VERSION {
            bail!("kit-material-specs.json: unexpected schema {} / maps {}", f.schema, f.maps_version);
        }
        Ok(Self {
            specs: f.specs,
            reference_bakes: f.reference_bakes,
            sources: f.source.into_values().map(|s| (s.path, s.sha256)).collect(),
        })
    }


    /// `<kit name>` of a GLB material name `<kit name>@rrggbb`, if it is a kit material.
    pub fn kit_name<'a>(&self, material_name: &'a str) -> Option<&'a str> {
        let base = material_name.split('@').next().unwrap_or_default();
        self.specs.contains_key(base).then_some(base)
    }

    /// The material whose maps a kit material draws (`mapsFrom` chain).
    pub fn map_source<'a>(&'a self, name: &'a str) -> &'a str {
        let mut n = name;
        while let Some(next) = self.specs.get(n).and_then(|s| s.maps_from.as_deref()) {
            n = next;
        }
        n
    }
}

/// `#rrggbb` (sRGB) to linear, as THREE.Color does under colour management.
pub fn hex_to_linear(hex: &str) -> Option<[f64; 3]> {
    let n = u32::from_str_radix(hex.trim_start_matches('#'), 16).ok()?;
    Some([
        srgb_to_linear(((n >> 16) & 255) as f64 / 255.0),
        srgb_to_linear(((n >> 8) & 255) as f64 / 255.0),
        srgb_to_linear((n & 255) as f64 / 255.0),
    ])
}

// ---------------------------------------------------------------------------
// Port of material-bake.js. Integer hashing is done in u32 with wrapping ops,
// which is exactly JS's ToInt32/ToUint32 arithmetic for these inputs.
// ---------------------------------------------------------------------------

const UINT32: f64 = 4294967296.0;

/// `hashSeed` in kit/frame.js (FNV-1a over UTF-16 code units, then an avalanche).
pub fn hash_seed(text: &str) -> u32 {
    let mut h: u32 = 0x811c9dc5;
    for c in text.encode_utf16() {
        h = (h ^ c as u32).wrapping_mul(0x01000193);
    }
    h ^= h >> 16;
    h = h.wrapping_mul(0x85ebca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2ae35);
    h ^= h >> 16;
    h
}

#[inline]
fn hash_int(x: u32) -> u32 {
    let mut h = x;
    h = (h ^ (h >> 16)).wrapping_mul(0x7feb352d);
    h = (h ^ (h >> 15)).wrapping_mul(0x846ca68b);
    h ^ (h >> 16)
}

/// JS `x | 0` of an integer-valued number (wraps mod 2^32).
#[inline]
fn i32w(x: f64) -> u32 {
    (x as i64) as u32
}

#[inline]
fn hash2(ix: i64, iy: i64, seed: u32) -> f64 {
    let mut h = hash_int((ix + 0x9e3779b9) as u32);
    h = hash_int((h ^ iy as u32).wrapping_add(0x85ebca6b));
    h = hash_int((h ^ seed).wrapping_add(0xc2b2ae35));
    h as f64 / UINT32
}

#[inline]
fn fade(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}
#[inline]
fn mix(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}
#[inline]
fn clamp01(v: f64) -> f64 {
    if v < 0.0 {
        0.0
    } else if v > 1.0 {
        1.0
    } else {
        v
    }
}
#[inline]
fn imod(a: i64, m: i64) -> i64 {
    ((a % m) + m) % m
}
#[inline]
fn smooth(a: f64, b: f64, x: f64) -> f64 {
    let t = clamp01((x - a) / (b - a));
    t * t * (3.0 - 2.0 * t)
}

fn vnoise(u: f64, v: f64, px: i64, py: i64, seed: u32) -> f64 {
    let fx = u * px as f64;
    let fy = v * py as f64;
    let ixf = fx.floor();
    let iyf = fy.floor();
    let tx = fade(fx - ixf);
    let ty = fade(fy - iyf);
    let x0 = imod(ixf as i64, px);
    let x1 = if x0 + 1 == px { 0 } else { x0 + 1 };
    let y0 = imod(iyf as i64, py);
    let y1 = if y0 + 1 == py { 0 } else { y0 + 1 };
    mix(mix(hash2(x0, y0, seed), hash2(x1, y0, seed), tx), mix(hash2(x0, y1, seed), hash2(x1, y1, seed), tx), ty)
}

fn fbm(u: f64, v: f64, px: i64, py: i64, seed: u32, octaves: u32) -> f64 {
    let (mut sum, mut amp, mut norm) = (0.0, 1.0, 0.0);
    let mut ax = px.max(1);
    let mut ay = py.max(1);
    for i in 0..octaves {
        sum += amp * vnoise(u, v, ax, ay, seed.wrapping_add(i.wrapping_mul(0x9e37)));
        norm += amp;
        amp *= 0.5;
        ax *= 2;
        ay *= 2;
        if ax > 2048 || ay > 2048 {
            break;
        }
    }
    sum / norm
}

struct Cell {
    d1: f64,
    id: u32,
}

fn cellular(u: f64, v: f64, p: i64, seed: u32) -> Cell {
    let fx = u * p as f64;
    let fy = v * p as f64;
    let ix = fx.floor() as i64;
    let iy = fy.floor() as i64;
    let mut d1 = 1e9;
    let mut id = 0u32;
    for oy in -1..=1 {
        for ox in -1..=1 {
            let cx = ix + ox;
            let cy = iy + oy;
            let wx = imod(cx, p);
            let wy = imod(cy, p);
            let dx = cx as f64 + 0.15 + 0.7 * hash2(wx, wy, seed) - fx;
            let dy = cy as f64 + 0.15 + 0.7 * hash2(wx, wy, seed ^ 0x5bf03635) - fy;
            let d = (dx * dx + dy * dy).sqrt();
            if d < d1 {
                d1 = d;
                id = hash_int((wx as u32).wrapping_mul(73856093) ^ (wy as u32).wrapping_mul(19349663) ^ seed);
            }
        }
    }
    Cell { d1, id }
}

#[inline]
fn id_random(id: u32, salt: u32) -> f64 {
    hash_int(id ^ hash_int(salt.wrapping_add(0x27d4eb2f))) as f64 / UINT32
}

fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}
fn linear_to_srgb(c: f64) -> f64 {
    let v = clamp01(c);
    if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}
fn hex_lin(hex: &str) -> [f64; 3] {
    // parseInt(...replace('#',''), 16) on a 6-digit colour.
    hex_to_linear(hex).unwrap_or([0.0; 3])
}

#[derive(Clone, Copy)]
struct Out {
    r: f64,
    g: f64,
    b: f64,
    h: f64,
    rough: f64,
    metal: f64,
    ao: f64,
}

impl Out {
    fn set_rgb(&mut self, rgb: [f64; 3], k: f64) {
        self.r = rgb[0] * k;
        self.g = rgb[1] * k;
        self.b = rgb[2] * k;
    }
    fn mix_rgb(&mut self, rgb: [f64; 3], t: f64) {
        self.r = mix(self.r, rgb[0], t);
        self.g = mix(self.g, rgb[1], t);
        self.b = mix(self.b, rgb[2], t);
    }
}

struct Ctx {
    seed: u32,
    base: [f64; 3],
    mortar: [f64; 3],
    palette: Vec<[f64; 3]>,
}

struct P<'a>(&'a Value);
impl P<'_> {
    fn f(&self, k: &str) -> Option<f64> {
        self.0.get(k).and_then(|v| v.as_f64())
    }
    fn n(&self, k: &str) -> f64 {
        self.f(k).unwrap_or(f64::NAN)
    }
    fn s(&self, k: &str) -> Option<&str> {
        self.0.get(k).and_then(|v| v.as_str()).filter(|s| !s.is_empty())
    }
}

/// JS `Math.round` for the values the bake rounds (finite, non-negative:
/// there ties-toward-+inf and Rust's ties-away-from-zero agree).
#[inline]
fn js_round(x: f64) -> f64 {
    x.round()
}

fn sample_field(spec: &KitSpec, u: f64, v: f64, ctx: &Ctx, out: &mut Out) {
    let s = ctx.seed;
    let tile = spec.metres_per_tile;
    let p = P(&spec.p);
    out.h = 0.0;
    out.rough = spec.rough;
    out.metal = spec.metal;
    out.ao = 1.0;
    match spec.kind.as_str() {
        "brick" => {
            let courses = p.n("courses");
            let row = (v * courses).floor();
            let header = p.s("bond") == Some("common") && (row as i64) % 6 == 5;
            let per_row = if header { p.n("perRow") * 2.0 } else { p.n("perRow") };
            let offset = if header { 0.25 } else { ((row as i64) % 2) as f64 * 0.5 };
            let fu = u * per_row + offset;
            let col = fu.floor();
            let lx = fu - col;
            let ly = v * courses - row;
            let mortar_m = p.n("mortarMetres");
            let mw = mortar_m / (tile / per_row);
            let mh = mortar_m / (tile / courses);
            let mortar = (1.0 - smooth(mw * 0.6, mw * 1.4, lx)).max(1.0 - smooth(mh * 0.6, mh * 1.4, ly));
            let id = hash_int(
                ((imod(col as i64, per_row as i64) + 1) as u32).wrapping_mul(0x51ed27)
                    ^ ((row as i64 + 1) as u32).wrapping_mul(0x9e3779b1)
                    ^ s,
            );
            let n = ctx.palette.len();
            let tone = ctx.palette[((id_random(id, 1) * n as f64).floor() as usize) % n];
            let jitter = 0.9 + id_random(id, 2) * 0.22;
            let grain = fbm(u, v, 48, 48, s ^ id, 2);
            let chip = smooth(0.9, 1.0, fbm(u * 3.0, v * 3.0, 24, 24, s ^ 0x77, 3)) * 0.5;
            out.set_rgb(tone, jitter * (0.92 + grain * 0.16));
            let soot = fbm(u, v, 3, 3, s ^ 0x31, 2);
            out.mix_rgb([0.02, 0.018, 0.016], soot * soot * 0.35);
            out.h = (grain - 0.5) * 0.15 - chip * 0.3;
            out.rough = spec.rough + (grain - 0.5) * 0.12;
            out.mix_rgb(ctx.mortar, mortar);
            out.h = mix(out.h, -1.0, mortar);
            out.rough = mix(out.rough, 0.93, mortar);
            out.ao = 1.0 - mortar * 0.3;
        }
        "stucco" => {
            let n = fbm(u, v, 24, 24, s, 4);
            let fine = fbm(u, v, 96, 96, s ^ 0x13, 2);
            out.set_rgb(ctx.base, 0.97 + (n - 0.5) * 0.06);
            out.h = (n - 0.5) * 0.7 + (fine - 0.5) * 0.3;
            out.rough = spec.rough + (fine - 0.5) * 0.08;
        }
        "ashlar" | "terracotta" => {
            let bw = p.f("blockW").unwrap_or_else(|| p.n("block"));
            let bh = p.f("blockH").unwrap_or_else(|| p.n("block"));
            let cols = js_round(tile / bw).max(1.0);
            let rows = js_round(tile / bh).max(1.0);
            let row = (v * rows).floor();
            let fu = u * cols + ((row as i64) % 2) as f64 * 0.5;
            let col = fu.floor();
            let lx = fu - col;
            let ly = v * rows - row;
            let joint_m = p.n("joint");
            let jw = joint_m / (tile / cols);
            let jh = joint_m / (tile / rows);
            let edge = if joint_m > 0.0 { lx.min(1.0 - lx).min(ly * (jw / jh)).min((1.0 - ly) * (jw / jh)) } else { 1.0 };
            let joint = if joint_m > 0.0 { 1.0 - smooth(jw * 0.5, jw * 1.5, edge) } else { 0.0 };
            let id = hash_int(
                ((imod(col as i64, cols as i64) + 1) as u32).wrapping_mul(0x2545f491)
                    ^ ((row as i64 + 1) as u32).wrapping_mul(0x9e3779b1)
                    ^ s,
            );
            let tone = 1.0 + (id_random(id, 1) - 0.5) * p.f("toneJitter").unwrap_or(0.06) * 2.0;
            if spec.kind == "ashlar" {
                let tooled = fbm(u * 1.0, v, 8, 64, s ^ id, 3);
                let grain = fbm(u, v, 40, 40, s ^ 0x55, 2);
                out.set_rgb(ctx.base, tone * (0.96 + (grain - 0.5) * 0.1));
                out.h = (tooled - 0.5) * p.f("grain").unwrap_or(0.4) + (grain - 0.5) * 0.2;
                out.rough = spec.rough + (grain - 0.5) * 0.1;
            } else {
                let dome = smooth(0.0, 0.18, edge);
                let mottle = fbm(u, v, 12, 12, s ^ id, 3);
                out.set_rgb(ctx.base, tone * (0.94 + mottle * 0.12));
                out.h = dome * 0.6 + (mottle - 0.5) * 0.1;
                out.rough = spec.rough + (mottle - 0.5) * 0.15;
            }
            out.mix_rgb(ctx.mortar, joint);
            out.h = mix(out.h, -1.0, joint);
            out.rough = mix(out.rough, 0.9, joint);
            out.ao = 1.0 - joint * 0.35;
        }
        "precast" => {
            let panels = js_round(tile / p.n("panel")).max(1.0);
            let lx = u * panels - (u * panels).floor();
            let ly = v * panels - (v * panels).floor();
            let jw = p.n("joint") / (tile / panels);
            let joint = 1.0 - smooth(jw * 0.5, jw * 1.5, lx.min(1.0 - lx).min(ly).min(1.0 - ly));
            let mut tie: f64 = 0.0;
            for tx in [0.25, 0.75] {
                for ty in [0.25, 0.75] {
                    tie = tie.max(1.0 - smooth(0.012, 0.02, (lx - tx).hypot(ly - ty)));
                }
            }
            let speckle = fbm(u, v, 128, 128, s, 2);
            let cloud = fbm(u, v, 5, 5, s ^ 0x71, 3);
            out.set_rgb(ctx.base, 0.94 + (cloud - 0.5) * 0.1 + (speckle - 0.5) * 0.05);
            out.h = (speckle - 0.5) * 0.12 - joint * 0.8 - tie * 0.7;
            out.rough = spec.rough + (speckle - 0.5) * 0.1;
            out.ao = 1.0 - joint * 0.3 - tie * 0.4;
        }
        "corrugated" => {
            let ridges = js_round(tile / p.n("pitch")).max(1.0);
            let wave = (u * ridges * std::f64::consts::PI * 2.0).sin();
            let spangle = cellular(u, v, 32, s).id;
            let streak = fbm(u, v, 2, 12, s ^ 0x22, 3);
            out.set_rgb(ctx.base, 0.92 + id_random(spangle, 3) * 0.12);
            let rust = smooth(0.62, 0.9, streak) * 0.5;
            out.mix_rgb([0.32, 0.12, 0.05], rust);
            out.h = wave * 0.5;
            out.rough = spec.rough + rust * 0.4 + (id_random(spangle, 5) - 0.5) * 0.08;
            out.metal = mix(spec.metal, 0.2, rust);
        }
        "seam" => {
            let panels = js_round(tile / p.n("panel")).max(1.0);
            let lx = u * panels - (u * panels).floor();
            let sw = p.n("seam") / (tile / panels);
            let seam = smooth(sw * 1.2, sw * 0.4, lx.min(1.0 - lx));
            let oilcan = fbm(u, v, 4, 6, s, 3);
            out.set_rgb(ctx.base, 0.94 + (oilcan - 0.5) * 0.06);
            out.h = seam * 1.0 + (oilcan - 0.5) * 0.12;
            out.rough = spec.rough + (oilcan - 0.5) * 0.08;
            out.ao = 1.0 - seam * 0.1;
        }
        "tile" => {
            let tiles = js_round(tile / p.n("tile")).max(1.0);
            let lx = u * tiles - (u * tiles).floor();
            let ly = v * tiles - (v * tiles).floor();
            let gw = p.n("grout") / (tile / tiles);
            let grout = 1.0 - smooth(gw * 0.5, gw * 1.5, lx.min(1.0 - lx).min(ly).min(1.0 - ly));
            // Plain (non-imul) products in JS, then ToInt32 at the XOR.
            let a = ((u * tiles).floor() * 0x2545f491u32 as f64) as i64 as u32;
            let b = ((v * tiles).floor() * 0x9e3779b1u32 as f64) as i64 as u32;
            let id = hash_int(a ^ b ^ s);
            out.set_rgb(ctx.base, 0.94 + id_random(id, 1) * 0.12);
            out.h = -grout * 1.0 + smooth(0.0, 0.08, lx.min(1.0 - lx).min(ly).min(1.0 - ly)) * 0.2;
            out.mix_rgb(ctx.mortar, grout);
            out.rough = mix(spec.rough, 0.85, grout);
            out.ao = 1.0 - grout * 0.3;
        }
        "siding" => {
            let boards = js_round(tile / p.n("board")).max(1.0);
            let row = (v * boards).floor();
            let ly = v * boards - row;
            let id = hash_int(((row as i64 + 1) as u32).wrapping_mul(0x9e3779b1) ^ s);
            let grain = fbm(u, v, 64, 6, s ^ id, 3);
            out.set_rgb(ctx.base, (0.96 + (id_random(id, 1) - 0.5) * 0.08) * (0.97 + (grain - 0.5) * 0.06));
            out.h = ly * 1.0 - smooth(0.04, 0.0, ly) * 0.4 + (grain - 0.5) * 0.05;
            out.rough = spec.rough + (grain - 0.5) * 0.1;
            out.ao = 1.0 - smooth(0.12, 0.0, ly) * 0.35;
        }
        "shingle" => {
            let rows = js_round(tile / p.n("course")).max(1.0);
            let row = (v * rows).floor();
            let ly = v * rows - row;
            let per = js_round(tile / p.n("width")).max(2.0);
            let shift = id_random(hash_int(((row as i64 + 7) as u32).wrapping_mul(0x85ebca6b) ^ s), 3);
            let x = u * per + shift * per;
            let inner = hash_int(i32w(x.floor() * 31.0 + row));
            let col = (x + (id_random(inner, 5) - 0.5) * 0.35).floor();
            let lx = x - x.floor();
            let id = hash_int(
                ((row as i64 + 1) as u32).wrapping_mul(0x9e3779b1) ^ ((col as i64 + 3) as u32).wrapping_mul(0xc2b2ae35) ^ s,
            );
            let grain = fbm(u, v * 0.25, 96, 12, s ^ id, 3);
            let joint = smooth(0.035, 0.0, lx.min(1.0 - lx));
            let vary = p.n("vary");
            out.set_rgb(
                ctx.base,
                (1.0 + (id_random(id, 1) - 0.5) * vary) * (0.96 + (grain - 0.5) * 0.1) * (1.0 - joint * 0.45),
            );
            out.h = ly * 0.8 - smooth(0.05, 0.0, ly) * 0.5 - joint * 0.3 + (grain - 0.5) * 0.08;
            out.rough = spec.rough + (grain - 0.5) * 0.12;
            out.ao = (1.0 - smooth(0.14, 0.0, ly) * 0.4) * (1.0 - joint * 0.3);
        }
        "gravel" => {
            let c = cellular(u, v, 40, s);
            let dome = smooth(0.7, 0.1, c.d1);
            let fine = fbm(u, v, 128, 128, s ^ 0x2b, 2);
            out.set_rgb(ctx.base, 0.85 + id_random(c.id, 2) * 0.3);
            out.h = dome * 0.9 + (fine - 0.5) * 0.1;
            out.rough = spec.rough - dome * 0.05;
            out.ao = 1.0 - (1.0 - dome) * 0.3;
        }
        "membrane" => {
            let drift = fbm(u, v, 2, 2, s ^ 0x13, 4);
            let mottle = fbm(u, v, 14, 14, s ^ 0x5d, 3);
            let grit = fbm(u, v, 192, 192, s ^ 0x91, 2);
            let pond = cellular(u, v, 3, s ^ 0x3c);
            let holds = if id_random(pond.id, 3) < 0.17 { 1.0 } else { 0.0 };
            let warp = (fbm(u, v, 12, 12, s ^ 0x3d, 3) - 0.5) * 0.3;
            let d = pond.d1 + warp;
            let radius = 0.22 + id_random(pond.id, 4) * 0.16;
            let ring = smooth(radius + 0.06, radius, d) * smooth(radius - 0.16, radius - 0.02, d) * holds;
            let pond_floor = smooth(radius, radius - 0.2, d) * holds;
            out.set_rgb(ctx.base, 1.0);
            let grime = smooth(0.34, 0.8, drift) * 0.62 + (mottle - 0.5) * 0.22 + (grit - 0.5) * 0.08;
            out.mix_rgb(ctx.mortar, clamp01(grime * 0.55 + ring * 0.2 + pond_floor * 0.08));
            out.h = (grit - 0.5) * 0.25 + (mottle - 0.5) * 0.4 - pond_floor * 0.3;
            out.rough = spec.rough + (grit - 0.5) * 0.06 - pond_floor * 0.1 + smooth(0.4, 0.8, drift) * 0.05;
            out.ao = 1.0 - ring * 0.1;
        }
        "canvas" => {
            let stripe_m = p.f("stripe").unwrap_or(0.0);
            let stripes = if stripe_m > 0.0 { js_round(tile / stripe_m).max(2.0) } else { 0.0 };
            let stripe = if stripes > 0.0 { ((u * stripes).floor() as i64) % 2 } else { 0 };
            let pi = std::f64::consts::PI;
            let weave = ((u * 512.0 * pi).sin() * (v * 512.0 * pi).sin()) * 0.5 + 0.5;
            let fade_v = fbm(u, v, 3, 3, s, 2);
            out.set_rgb(if stripe != 0 { ctx.mortar } else { ctx.base }, 0.94 + fade_v * 0.08);
            out.h = weave * 1.0;
            out.rough = spec.rough + (weave - 0.5) * 0.06;
        }
        "glass" => {
            let smudge = fbm(u, v, 3, 3, s, 3);
            out.set_rgb(ctx.base, 1.0);
            out.h = (smudge - 0.5) * 1.0;
            out.rough = spec.rough + smooth(0.55, 0.8, smudge) * p.f("smudgeRoughness").unwrap_or(0.12);
        }
        _ => {
            // 'flat' and anything else.
            let n = fbm(u, v, 64, 64, s, 2);
            let drip = fbm(u, v, 12, 2, s ^ 0x44, 2);
            out.set_rgb(ctx.base, 0.97 + (n - 0.5) * 0.04);
            out.h = (n - 0.5) * 1.0;
            out.rough = spec.rough + (n - 0.5) * 0.1 + smooth(0.6, 0.9, drip) * 0.08;
        }
    }
}

/// One baked map set: RGBA8 albedo (sRGB), normal (tangent space, +Z up) and
/// ORM (R occlusion, G roughness, B metalness), `res` x `res`, tiling.
#[derive(Clone)]
pub struct BakedMaps {
    pub res: usize,
    pub metres_per_tile: f64,
    pub albedo: Vec<u8>,
    pub normal: Vec<u8>,
    pub orm: Vec<u8>,
}

/// `bakeAuthoredSpecMaps(name, spec, seed, res)`. `name` seeds the field.
pub fn bake(name: &str, spec: &KitSpec, seed: &str, res: usize) -> BakedMaps {
    let p = P(&spec.p);
    let ctx = Ctx {
        seed: hash_seed(&format!("{MAPS_VERSION}|{seed}|{name}")),
        base: hex_lin(p.s("color").or(p.s("colorA")).unwrap_or("#888888")),
        mortar: hex_lin(p.s("mortar").or(p.s("groutColor")).or(p.s("colorB")).unwrap_or("#a09a90")),
        palette: match spec.p.get("colors").and_then(|c| c.as_array()) {
            Some(a) => a.iter().map(|c| hex_lin(c.as_str().unwrap_or("#888888"))).collect(),
            None => vec![hex_lin(p.s("color").unwrap_or("#888888"))],
        },
    };
    let n = res * res;
    let mut albedo = vec![0u8; n * 4];
    let mut height = vec![0f32; n];
    let mut rough = vec![0f32; n];
    let mut metal = vec![0f32; n];
    let mut ao = vec![0f32; n];
    let (mut min_h, mut max_h) = (f64::INFINITY, f64::NEG_INFINITY);
    let mut out = Out { r: 0.0, g: 0.0, b: 0.0, h: 0.0, rough: 0.5, metal: 0.0, ao: 1.0 };
    for y in 0..res {
        let v = (y as f64 + 0.5) / res as f64;
        for x in 0..res {
            let u = (x as f64 + 0.5) / res as f64;
            sample_field(spec, u, v, &ctx, &mut out);
            let i = y * res + x;
            height[i] = out.h as f32;
            rough[i] = out.rough as f32;
            metal[i] = out.metal as f32;
            ao[i] = out.ao as f32;
            if out.h < min_h {
                min_h = out.h;
            }
            if out.h > max_h {
                max_h = out.h;
            }
            let o = i * 4;
            albedo[o] = js_round(linear_to_srgb(out.r) * 255.0) as u8;
            albedo[o + 1] = js_round(linear_to_srgb(out.g) * 255.0) as u8;
            albedo[o + 2] = js_round(linear_to_srgb(out.b) * 255.0) as u8;
            albedo[o + 3] = 255;
        }
    }
    let span = max_h - min_h;
    let scale = if span > 1e-9 { spec.height / span } else { 0.0 };
    for h in height.iter_mut() {
        *h = ((*h as f64 - min_h) * scale) as f32;
    }
    let texel_metres = spec.metres_per_tile / res as f64;
    let mut normal = vec![0u8; n * 4];
    let mut orm = vec![0u8; n * 4];
    let r = res as i64;
    let hf = |i: usize| height[i] as f64;
    for y in 0..res {
        let ym = imod(y as i64 - 1, r) as usize * res;
        let y0 = y * res;
        let yp = imod(y as i64 + 1, r) as usize * res;
        for x in 0..res {
            let xm = imod(x as i64 - 1, r) as usize;
            let xp = imod(x as i64 + 1, r) as usize;
            let gx = ((hf(ym + xp) + 2.0 * hf(y0 + xp) + hf(yp + xp)) - (hf(ym + xm) + 2.0 * hf(y0 + xm) + hf(yp + xm)))
                / (8.0 * texel_metres);
            let gy = ((hf(yp + xm) + 2.0 * hf(yp + x) + hf(yp + xp)) - (hf(ym + xm) + 2.0 * hf(ym + x) + hf(ym + xp)))
                / (8.0 * texel_metres);
            let (mut dx, mut dy) = (-gx, -gy);
            let g = dx.hypot(dy);
            if g > 4.0 {
                dx *= 4.0 / g;
                dy *= 4.0 / g;
            }
            let inv = 1.0 / (dx * dx + dy * dy + 1.0).sqrt();
            let i = y0 + x;
            let o = i * 4;
            normal[o] = js_round((dx * inv * 0.5 + 0.5) * 255.0) as u8;
            normal[o + 1] = js_round((dy * inv * 0.5 + 0.5) * 255.0) as u8;
            normal[o + 2] = js_round((inv * 0.5 + 0.5) * 255.0) as u8;
            normal[o + 3] = 255;
            let mean_h = (hf(ym + x) + hf(yp + x) + hf(y0 + xm) + hf(y0 + xp)) / 4.0;
            let cavity = clamp01((mean_h - hf(i)) / (spec.height * 0.5).max(1e-7));
            orm[o] = js_round(clamp01(ao[i] as f64 * (1.0 - cavity * 0.5)) * 255.0) as u8;
            orm[o + 1] = js_round(clamp01(rough[i] as f64) * 255.0) as u8;
            orm[o + 2] = js_round(clamp01(metal[i] as f64) * 255.0) as u8;
            orm[o + 3] = 255;
        }
    }
    BakedMaps { res, metres_per_tile: spec.metres_per_tile, albedo, normal, orm }
}

/// Bake (or read from `cache_dir/kit/`) the maps of every source in `names`,
/// on worker threads. Returns maps in the order of `names`.
pub fn bake_all(lib: &KitLibrary, names: &[String], res: usize, cache_dir: Option<&Path>) -> Result<Vec<BakedMaps>> {
    let spec_of = |n: &str| lib.specs.get(n).ok_or_else(|| anyhow!("kit material {n} not in the registry"));
    for n in names {
        spec_of(n)?;
    }
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(names.len().max(1));
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results: Vec<std::sync::Mutex<Option<Result<BakedMaps>>>> = names.iter().map(|_| std::sync::Mutex::new(None)).collect();
    std::thread::scope(|sc| {
        for _ in 0..threads {
            sc.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if i >= names.len() {
                    break;
                }
                let name = &names[i];
                let r = (|| -> Result<BakedMaps> {
                    let spec = spec_of(name)?;
                    let key = crate::glb::sha256_hex(
                        format!("{MAPS_VERSION}|{RUNTIME_SEED}|{name}|{res}|{}", serde_json::to_string(&spec.p)?).as_bytes(),
                    );
                    let path = cache_dir.map(|d| d.join("kit").join(format!("{name}-{res}-{}.bin", &key[..16])));
                    if let Some(p) = &path {
                        if let Ok(b) = std::fs::read(p) {
                            if b.len() == res * res * 12 {
                                let n = res * res * 4;
                                return Ok(BakedMaps {
                                    res,
                                    metres_per_tile: spec.metres_per_tile,
                                    albedo: b[..n].to_vec(),
                                    normal: b[n..2 * n].to_vec(),
                                    orm: b[2 * n..].to_vec(),
                                });
                            }
                        }
                    }
                    let m = bake(name, spec, RUNTIME_SEED, res);
                    if let Some(p) = &path {
                        std::fs::create_dir_all(p.parent().unwrap())?;
                        let mut b = Vec::with_capacity(res * res * 12);
                        b.extend_from_slice(&m.albedo);
                        b.extend_from_slice(&m.normal);
                        b.extend_from_slice(&m.orm);
                        let tmp = p.with_extension(format!("tmp{}", std::process::id()));
                        std::fs::write(&tmp, &b)?;
                        let _ = std::fs::rename(&tmp, p);
                    }
                    Ok(m)
                })();
                *results[i].lock().unwrap() = Some(r);
            });
        }
    });
    results.into_iter().map(|m| m.into_inner().unwrap().unwrap()).collect()
}
