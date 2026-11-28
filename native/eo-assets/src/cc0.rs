//! the CC0 texture sets (Poly Haven, ambientCG) the renderer reads from a
//! folder Git does not track, verified, decoded and block-compressed once.
//!
//! * **Provenance**: `native/assets/cc0-manifest.json` (written by
//!   `native/tools/fetch-cc0.mjs`) lists every downloaded file with its source,
//!   asset page, authors as the site lists them, licence, resolution,
//!   download URL, size and SHA-256. Files are read from
//!   `<repo>/external/cc0` (git-ignored, filled by `scripts/setup-assets.py`),
//!   or from the folder `EO_CC0_ROOT` names instead. A file is decoded only
//!   after its SHA-256 equals the manifest's; nothing is tracked by Git.
//!   Without the downloads `available()` is false and the renderer keeps its
//!   own materials.
//! * **Cook**: each set becomes three square maps of `res` px with a mip
//!   chain down to 4 x 4: albedo (BC7, sRGB; decals carry their opacity in
//!   alpha), tangent normal (BC5, OpenGL +y up), and RHM (BC7: R roughness,
//!   G ambient occlusion, B height, A metalness). Mips are averaged in linear
//!   light (albedo), renormalised (normals) or as stored (the rest). The
//!   result is cached under `cache/cc0/` (key: cook version + the set's file
//!   digests; payload SHA-256 at the end, a damaged file is re-cooked); a
//!   cache hit reads no source file but still requires every source file to
//!   be present with the manifest's size.
//! * **Layers**: `MATERIAL_SETS` (2048 px) and `DECAL_SETS` (1024 px) fix the
//!   texture array layer of every set; eo-render maps the game's material
//!   classes onto them.

use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The repository's provenance manifest (small JSON, tracked).
pub const MANIFEST_JSON: &str = include_str!("../../assets/cc0-manifest.json");
/// Bump when the cooked layout or the encoder settings change.
pub const COOK_VERSION: &str = "eo-cc0-cook-v1";
pub const MATERIAL_RES: u32 = 2048;
pub const DECAL_RES: u32 = 1024;
/// Smallest mip kept (one BC block).
pub const MIN_MIP: u32 = 4;

#[derive(Clone, Debug)]
pub struct Cc0File {
    pub role: String,
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
    pub url: Option<String>,
    pub archive: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Cc0Asset {
    pub source: String,
    pub id: String,
    pub page: String,
    pub authors: Vec<String>,
    pub licence: String,
    /// Real-world size of one repeat (Poly Haven's `dimensions`), metres; empty when the site gives none.
    pub size_m: Vec<f64>,
    pub use_: String,
    pub files: Vec<Cc0File>,
}

impl Cc0Asset {
    pub fn file(&self, role: &str) -> Option<&Cc0File> {
        self.files.iter().find(|f| f.role == role)
    }
}

pub struct Cc0Manifest {
    pub assets: Vec<Cc0Asset>,
    pub downloaded_bytes: u64,
}

impl Cc0Manifest {
    pub fn get() -> &'static Cc0Manifest {
        static M: OnceLock<Cc0Manifest> = OnceLock::new();
        M.get_or_init(|| Self::parse(MANIFEST_JSON).expect("native/assets/cc0-manifest.json"))
    }

    pub fn parse(text: &str) -> Result<Self> {
        let v: serde_json::Value = serde_json::from_str(text)?;
        let s = |x: &serde_json::Value| x.as_str().unwrap_or_default().to_string();
        let mut assets = vec![];
        for a in v["assets"].as_array().ok_or_else(|| anyhow!("cc0 manifest: no assets"))? {
            let files = a["files"]
                .as_array()
                .ok_or_else(|| anyhow!("cc0 manifest: asset without files"))?
                .iter()
                .map(|f| Cc0File {
                    role: s(&f["role"]),
                    path: s(&f["path"]),
                    bytes: f["bytes"].as_u64().unwrap_or(0),
                    sha256: s(&f["sha256"]),
                    url: f["url"].as_str().map(|u| u.to_string()),
                    archive: f["archive"].as_str().map(|u| u.to_string()),
                })
                .collect();
            assets.push(Cc0Asset {
                source: s(&a["source"]),
                id: s(&a["id"]),
                page: s(&a["page"]),
                authors: a["authors"].as_array().map(|x| x.iter().map(s).collect()).unwrap_or_default(),
                licence: s(&a["licence"]),
                size_m: a["size_m"].as_array().map(|x| x.iter().filter_map(|v| v.as_f64()).collect()).unwrap_or_default(),
                use_: s(&a["use"]),
                files,
            });
        }
        Ok(Self { assets, downloaded_bytes: v["downloadedBytes"].as_u64().unwrap_or(0) })
    }

    pub fn asset(&self, id: &str) -> Option<&Cc0Asset> {
        self.assets.iter().find(|a| a.id == id)
    }
}

/// The sets' folder this run: the one `EO_CC0_ROOT` names, else
/// `<repo>/external/cc0`; None when neither is known (the sets are then not used).
pub fn root() -> Option<PathBuf> {
    eo_data::external::CC0.root()
}

/// Where cooked sets are cached: `EO_CC0_CACHE`, else `cc0/` under the
/// native cache folder given (none: no cache).
pub fn cache_root(native_cache: Option<&Path>) -> Option<PathBuf> {
    if let Ok(p) = std::env::var("EO_CC0_CACHE") {
        return Some(PathBuf::from(p));
    }
    native_cache.map(|c| c.join("cc0"))
}

/// What one set is drawn as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetKind {
    Material,
    Decal,
}

/// How a set's repeat size is found: the site's measured size, or (sets the
/// site gives no size for) the dominant period of its height map along v
/// equal to `metres` (a clapboard's exposure, a shingle course).
#[derive(Clone, Copy, Debug)]
pub enum Scale {
    Site,
    PeriodV(f32),
    Metres([f32; 2]),
}

#[derive(Clone, Copy, Debug)]
pub struct SetDef {
    pub id: &'static str,
    pub kind: SetKind,
    pub scale: Scale,
    /// Roughness when the set has no roughness map.
    pub rough_default: f32,
}

const fn m(id: &'static str) -> SetDef {
    SetDef { id, kind: SetKind::Material, scale: Scale::Site, rough_default: 0.8 }
}
const fn mp(id: &'static str, period: f32, rough: f32) -> SetDef {
    SetDef { id, kind: SetKind::Material, scale: Scale::PeriodV(period), rough_default: rough }
}
const fn d(id: &'static str, size: [f32; 2]) -> SetDef {
    SetDef { id, kind: SetKind::Decal, scale: Scale::Metres(size), rough_default: 0.7 }
}

/// Texture array layer order of the material sets (eo-render names them CC0_L_*).
/// The kit's clapboard board is 0.15 m and its shingle course 0.14 m
/// (kit-material-specs.json `painted-clapboard` / `cedar-shingle`).
pub const MATERIAL_SETS: [SetDef; 34] = [
    m("white_stucco"),
    mp("WoodSiding009", 0.15, 0.62),
    mp("Planks010", 0.15, 0.75),
    mp("WoodSiding011", 0.14, 0.78),
    m("red_bricks_04"),
    m("brick_wall_006"),
    m("yellow_bricks"),
    m("white_sandstone_blocks_02"),
    m("concrete_wall_008"),
    m("wood_textured_concrete"),
    m("corrugated_iron_02"),
    m("box_profile_metal_sheet"),
    m("rectangular_facade_tiles"),
    m("concrete_block_wall"),
    m("painted_metal_shutter"),
    m("tarred_gravel"),
    m("clay_roof_tiles_02"),
    m("worn_asphalt"),
    m("road_damaged"),
    m("clean_asphalt"),
    m("dirty_concrete"),
    m("concrete_floor_worn_001"),
    m("herringbone_pavement"),
    m("patterned_concrete_pavers"),
    m("cobblestone_floor_08"),
    m("leafy_grass"),
    m("withered_grass"),
    m("park_dirt"),
    m("gravel_floor_03"),
    m("forest_floor"),
    m("coast_sand_04"),
    m("damp_beach_sand"),
    // ambientCG gives no size; a lawn scan of about 1.6 m (blade length against the frame).
    SetDef { id: "Grass004", kind: SetKind::Material, scale: Scale::Metres([1.6, 1.6]), rough_default: 0.9 },
    // Poly Haven granite block wall (fetched & verified in cc0-manifest.json; granite towers, plinths, shore armour).
    m("granite_wall"),
];

/// Decal layers: the size one decal (or one repeat of a tiling mask) covers on
/// the ground or wall (metres, u x v). These are the sizes of the scanned
/// pieces (a cast-iron cover with its frame, a gum scatter, a stain field),
/// not street geometry: where a placement needs a street dimension it reads
/// the California standards table (cc0_decals.rs).
pub const DECAL_SETS: [SetDef; 14] = [
    d("AsphaltDamage001", [3.2, 3.2]),
    d("ManholeCover001", [0.9, 0.9]),
    d("ManholeCover003", [0.9, 0.9]),
    d("ChewingGum001", [1.2, 1.2]),
    d("Leaking001", [1.6, 0.8]),
    d("Leaking003", [1.6, 0.8]),
    // cc0 v2: street and wall wear.
    d("ManholeCover006", [0.9, 0.9]),
    d("ChewingGum002", [1.2, 1.2]),
    // ambientCG gives 180 x 45 cm (excavator tracks in sand): a tread mask, compressed across a tyre.
    d("TireTracks001", [1.8, 0.45]),
    // Tiling masks: stain blotches (oil), mottled dirt (inside slabs), rising grime, worn paint.
    d("SurfaceImperfections002", [2.5, 2.5]),
    d("SurfaceImperfections008", [1.5, 1.5]),
    // (1024 x 256 px: a 4:1 strip.)
    d("Leaking008", [3.2, 0.8]),
    d("Grate002", [0.9, 0.9]),
    d("RoadLines004", [0.6, 0.6]),
];

/// One cooked set: BC blocks per mip (mip 0 = `res`), and the measures the
/// shaders normalise by.
#[derive(Clone, Debug)]
pub struct CookedSet {
    pub id: String,
    pub kind: SetKind,
    pub res: u32,
    /// BC7 sRGB albedo (+ opacity), BC5 normal, BC7 RHM; [map][mip].
    pub maps: [Vec<Vec<u8>>; 3],
    /// Linear mean albedo (opacity-weighted for decals).
    pub mean_rgb: [f32; 3],
    pub mean_rough: f32,
    pub mean_ao: f32,
    pub mean_height: f32,
    /// One repeat (or one decal), metres, u x v.
    pub size_m: [f32; 2],
    pub from_cache: bool,
    pub ms: f64,
}

impl CookedSet {
    pub fn bytes(&self) -> u64 {
        self.maps.iter().flatten().map(|v| v.len() as u64).sum()
    }
}

/// Mip count from `res` down to `MIN_MIP`.
pub fn mip_count(res: u32) -> u32 {
    res.ilog2() - MIN_MIP.ilog2() + 1
}

/// Every source file of every set present with the manifest's size (no hash: that is checked when cooking).
pub fn available(root: &Path) -> Result<()> {
    if !root.is_dir() {
        bail!("{} is not there", root.display());
    }
    let man = Cc0Manifest::get();
    for def in MATERIAL_SETS.iter().chain(DECAL_SETS.iter()) {
        let a = man.asset(def.id).ok_or_else(|| anyhow!("{} is not in the manifest", def.id))?;
        for f in a.files.iter().filter(|f| f.role != "archive") {
            let p = root.join(&f.path);
            let len = std::fs::metadata(&p).with_context(|| format!("{} missing", p.display()))?.len();
            if len != f.bytes {
                bail!("{} is {len} B, the manifest records {}", p.display(), f.bytes);
            }
        }
    }
    Ok(())
}

fn read_verified(root: &Path, f: &Cc0File) -> Result<Vec<u8>> {
    let p = root.join(&f.path);
    let bytes = std::fs::read(&p).with_context(|| format!("reading {}", p.display()))?;
    let got: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    if got != f.sha256 {
        bail!("{} has SHA-256 {got}, the manifest records {}", p.display(), f.sha256);
    }
    Ok(bytes)
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}
fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// A decoded image as float channels.
struct Img {
    w: usize,
    h: usize,
    c: usize,
    px: Vec<f32>,
}

impl Img {
    fn decode(bytes: &[u8], channels: usize) -> Result<Self> {
        let (w, h, rgb) = crate::jpeg::decode_rgb(bytes)?;
        let px = match channels {
            1 => rgb.chunks(3).map(|p| p[0] as f32 / 255.0).collect(),
            _ => rgb.iter().map(|&b| b as f32 / 255.0).collect(),
        };
        Ok(Self { w, h, c: channels.min(3), px })
    }

    /// Box / bilinear resample to `res` x `res` (area average when shrinking).
    fn resample(&self, res: usize) -> Img {
        if self.w == res && self.h == res {
            return Img { w: res, h: res, c: self.c, px: self.px.clone() };
        }
        let mut out = vec![0f32; res * res * self.c];
        let (fx, fy) = (self.w as f32 / res as f32, self.h as f32 / res as f32);
        for y in 0..res {
            for x in 0..res {
                let (x0, x1) = (x as f32 * fx, (x + 1) as f32 * fx);
                let (y0, y1) = (y as f32 * fy, (y + 1) as f32 * fy);
                if fx <= 1.0 && fy <= 1.0 {
                    // Upsampling: bilinear at the texel centre (wrapping: the sets tile).
                    let sx = (x as f32 + 0.5) * fx - 0.5;
                    let sy = (y as f32 + 0.5) * fy - 0.5;
                    let (ix, iy) = (sx.floor(), sy.floor());
                    let (tx, ty) = (sx - ix, sy - iy);
                    let wrap = |v: i64, n: usize| v.rem_euclid(n as i64) as usize;
                    for c in 0..self.c {
                        let g = |xx: i64, yy: i64| self.px[(wrap(yy, self.h) * self.w + wrap(xx, self.w)) * self.c + c];
                        let (ix, iy) = (ix as i64, iy as i64);
                        let a = g(ix, iy) * (1.0 - tx) + g(ix + 1, iy) * tx;
                        let b = g(ix, iy + 1) * (1.0 - tx) + g(ix + 1, iy + 1) * tx;
                        out[(y * res + x) * self.c + c] = a * (1.0 - ty) + b * ty;
                    }
                } else {
                    let (sx0, sx1) = (x0.floor() as usize, (x1.ceil() as usize).min(self.w));
                    let (sy0, sy1) = (y0.floor() as usize, (y1.ceil() as usize).min(self.h));
                    let mut acc = [0f32; 3];
                    let mut n = 0f32;
                    for sy in sy0..sy1.max(sy0 + 1) {
                        for sx in sx0..sx1.max(sx0 + 1) {
                            for c in 0..self.c {
                                acc[c] += self.px[(sy.min(self.h - 1) * self.w + sx.min(self.w - 1)) * self.c + c];
                            }
                            n += 1.0;
                        }
                    }
                    for c in 0..self.c {
                        out[(y * res + x) * self.c + c] = acc[c] / n;
                    }
                }
            }
        }
        Img { w: res, h: res, c: self.c, px: out }
    }
}

/// The dominant period (in rows) of a height map's row means: the first
/// strong peak of the autocorrelation (clapboard exposure, shingle course).
fn dominant_period_rows(h: &Img) -> Option<f32> {
    let rows: Vec<f32> = (0..h.h).map(|y| (0..h.w).map(|x| h.px[(y * h.w + x) * h.c]).sum::<f32>() / h.w as f32).collect();
    let mean = rows.iter().sum::<f32>() / rows.len() as f32;
    let d: Vec<f32> = rows.iter().map(|r| r - mean).collect();
    let n = d.len();
    let var: f32 = d.iter().map(|v| v * v).sum::<f32>();
    if var <= 1e-9 {
        return None;
    }
    let ac = |lag: usize| (0..n).map(|i| d[i] * d[(i + lag) % n]).sum::<f32>() / var;
    let lags: Vec<(usize, f32)> = (n / 64..n / 2).map(|l| (l, ac(l))).collect();
    let best = lags.iter().map(|x| x.1).fold(f32::MIN, f32::max);
    if best < 0.2 {
        return None;
    }
    for k in 1..lags.len() - 1 {
        let (l, v) = lags[k];
        if v >= lags[k - 1].1 && v >= lags[k + 1].1 && v > 0.6 * best {
            // Parabolic refinement of the peak.
            let (a, b, c) = (lags[k - 1].1, v, lags[k + 1].1);
            let den = a - 2.0 * b + c;
            let off = if den.abs() > 1e-9 { 0.5 * (a - c) / den } else { 0.0 };
            return Some(l as f32 + off.clamp(-0.5, 0.5));
        }
    }
    None
}

fn mips_rgba(base: Vec<[f32; 4]>, res: usize, renorm_normal: bool) -> Vec<Vec<[f32; 4]>> {
    let mut chain = vec![base];
    let mut r = res;
    while r > MIN_MIP as usize {
        let h = r / 2;
        let prev = chain.last().unwrap();
        let mut next = vec![[0f32; 4]; h * h];
        for y in 0..h {
            for x in 0..h {
                let mut s = [0f32; 4];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let p = prev[(2 * y + dy) * r + 2 * x + dx];
                    for c in 0..4 {
                        s[c] += p[c];
                    }
                }
                let mut v = s.map(|a| a / 4.0);
                if renorm_normal {
                    let (nx, ny) = (v[0] * 2.0 - 1.0, v[1] * 2.0 - 1.0);
                    let nz = (1.0 - nx * nx - ny * ny).max(0.0).sqrt();
                    let len = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-6);
                    v[0] = nx / len * 0.5 + 0.5;
                    v[1] = ny / len * 0.5 + 0.5;
                }
                next[y * h + x] = v;
            }
        }
        chain.push(next);
        r = h;
    }
    chain
}

fn to_bytes4(level: &[[f32; 4]], srgb_rgb: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(level.len() * 4);
    for p in level {
        for c in 0..4 {
            let v = if srgb_rgb && c < 3 { linear_to_srgb(p[c].clamp(0.0, 1.0)) } else { p[c] };
            out.push((v.clamp(0.0, 1.0) * 255.0).round() as u8);
        }
    }
    out
}

fn bc7(level: &[u8], r: u32, alpha: bool) -> Vec<u8> {
    let s = intel_tex_2::RgbaSurface { data: level, width: r, height: r, stride: r * 4 };
    let settings = if alpha { intel_tex_2::bc7::alpha_fast_settings() } else { intel_tex_2::bc7::opaque_fast_settings() };
    intel_tex_2::bc7::compress_blocks(&settings, &s)
}

fn bc5(level: &[[f32; 4]], r: u32) -> Vec<u8> {
    let rg: Vec<u8> = level.iter().flat_map(|p| [(p[0].clamp(0.0, 1.0) * 255.0).round() as u8, (p[1].clamp(0.0, 1.0) * 255.0).round() as u8]).collect();
    let s = intel_tex_2::RgSurface { data: &rg, width: r, height: r, stride: r * 2 };
    intel_tex_2::bc5::compress_blocks(&s)
}

fn set_res(def: &SetDef) -> u32 {
    if def.kind == SetKind::Decal { DECAL_RES } else { MATERIAL_RES }
}

/// Cache key of one set: cook version, resolution, the set's definition and its files' digests.
fn cache_key(def: &SetDef, a: &Cc0Asset) -> String {
    let mut h = Sha256::new();
    h.update(COOK_VERSION.as_bytes());
    h.update(set_res(def).to_le_bytes());
    h.update(format!("{def:?}").as_bytes());
    for f in &a.files {
        h.update(f.role.as_bytes());
        h.update(f.sha256.as_bytes());
    }
    h.finalize().iter().take(10).map(|b| format!("{b:02x}")).collect()
}

/// Decode, resample, build the maps and their mips, block-compress.
pub fn cook(root: &Path, def: &SetDef) -> Result<CookedSet> {
    let t = std::time::Instant::now();
    let man = Cc0Manifest::get();
    let a = man.asset(def.id).ok_or_else(|| anyhow!("{} is not in the manifest", def.id))?;
    let res = set_res(def) as usize;
    let load = |role: &str, ch: usize| -> Result<Option<Img>> {
        match a.file(role) {
            Some(f) => Ok(Some(Img::decode(&read_verified(root, f)?, ch).with_context(|| format!("{}: decoding {role}", a.id))?)),
            None => Ok(None),
        }
    };
    let albedo = load("albedo", 3)?.ok_or_else(|| anyhow!("{}: no albedo", a.id))?;
    let normal = load("normal", 3)?.ok_or_else(|| anyhow!("{}: no normal", a.id))?;
    let height = load("height", 1)?;
    // Poly Haven packs AO, roughness, metalness in `arm`; ambientCG ships them apart.
    let arm = load("arm", 3)?;
    let rough = load("roughness", 1)?;
    let ao = load("ao", 1)?;
    let metal = load("metalness", 1)?;
    let opacity = load("opacity", 1)?;
    let (src_w, src_h) = (albedo.w as f32, albedo.h as f32);
    let size_m = match def.scale {
        Scale::Metres(s) => s,
        Scale::Site => {
            let s = &a.size_m;
            if s.len() >= 2 && s[0] > 0.0 && s[1] > 0.0 { [s[0] as f32, s[1] as f32] } else { [2.0, 2.0 * src_h / src_w] }
        }
        Scale::PeriodV(metres) => {
            let hm = height.as_ref().ok_or_else(|| anyhow!("{}: a period scale needs a height map", a.id))?;
            let p = dominant_period_rows(hm).ok_or_else(|| anyhow!("{}: no dominant period in the height map", a.id))?;
            let courses = hm.h as f32 / p;
            let v = courses * metres;
            [v * src_w / src_h, v]
        }
    };
    let albedo = albedo.resample(res);
    let normal = normal.resample(res);
    let height = height.map(|i| i.resample(res));
    let arm = arm.map(|i| i.resample(res));
    let rough = rough.map(|i| i.resample(res));
    let ao = ao.map(|i| i.resample(res));
    let metal = metal.map(|i| i.resample(res));
    let opacity = opacity.map(|i| i.resample(res));
    let n = res * res;
    let lut: Vec<f32> = (0..256).map(|i| srgb_to_linear(i as f32 / 255.0)).collect();
    let lin = |v: f32| lut[(v * 255.0).round().clamp(0.0, 255.0) as usize];
    let mut alb = vec![[0f32; 4]; n];
    let mut nrm = vec![[0f32; 4]; n];
    let mut rhm = vec![[0f32; 4]; n];
    let (mut sum, mut wsum) = ([0f64; 3], 0f64);
    let (mut sr, mut sa, mut sh) = (0f64, 0f64, 0f64);
    for i in 0..n {
        let op = opacity.as_ref().map(|o| o.px[i]).unwrap_or(1.0);
        let c = [lin(albedo.px[i * 3]), lin(albedo.px[i * 3 + 1]), lin(albedo.px[i * 3 + 2])];
        alb[i] = [c[0], c[1], c[2], op];
        for k in 0..3 {
            sum[k] += (c[k] * op) as f64;
        }
        wsum += op as f64;
        nrm[i] = [normal.px[i * 3], normal.px[i * 3 + 1], 0.0, 1.0];
        let r = match (&arm, &rough) {
            (Some(m), _) => m.px[i * 3 + 1],
            (None, Some(r)) => r.px[i],
            _ => def.rough_default,
        };
        let o = match (&arm, &ao) {
            (Some(m), _) => m.px[i * 3],
            (None, Some(a)) => a.px[i],
            _ => 1.0,
        };
        let me = match (&arm, &metal) {
            (Some(m), _) => m.px[i * 3 + 2],
            (None, Some(m)) => m.px[i],
            _ => 0.0,
        };
        let h = height.as_ref().map(|h| h.px[i]).unwrap_or(0.5);
        rhm[i] = [r, o, h, me];
        sr += (r * op) as f64;
        sa += (o * op) as f64;
        sh += (h * op) as f64;
    }
    let w = wsum.max(1e-6);
    let mean_rgb = [(sum[0] / w) as f32, (sum[1] / w) as f32, (sum[2] / w) as f32];
    let decal = def.kind == SetKind::Decal;
    let ca = mips_rgba(alb, res, false);
    let cn = mips_rgba(nrm, res, true);
    let cr = mips_rgba(rhm, res, false);
    let mut maps: [Vec<Vec<u8>>; 3] = [vec![], vec![], vec![]];
    for (k, level) in ca.iter().enumerate() {
        let r = (res >> k) as u32;
        maps[0].push(bc7(&to_bytes4(level, true), r, decal));
    }
    for (k, level) in cn.iter().enumerate() {
        maps[1].push(bc5(level, (res >> k) as u32));
    }
    for (k, level) in cr.iter().enumerate() {
        maps[2].push(bc7(&to_bytes4(level, false), (res >> k) as u32, true));
    }
    Ok(CookedSet {
        id: a.id.clone(),
        kind: def.kind,
        res: res as u32,
        maps,
        mean_rgb,
        mean_rough: (sr / w) as f32,
        mean_ao: (sa / w) as f32,
        mean_height: (sh / w) as f32,
        size_m,
        from_cache: false,
        ms: t.elapsed().as_secs_f64() * 1e3,
    })
}

fn encode_cache(s: &CookedSet) -> Vec<u8> {
    let mut out = b"EOCC0001".to_vec();
    out.extend_from_slice(&s.res.to_le_bytes());
    for v in [s.mean_rgb[0], s.mean_rgb[1], s.mean_rgb[2], s.mean_rough, s.mean_ao, s.mean_height, s.size_m[0], s.size_m[1]] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for map in &s.maps {
        out.extend_from_slice(&(map.len() as u32).to_le_bytes());
        for lvl in map {
            out.extend_from_slice(&(lvl.len() as u32).to_le_bytes());
            out.extend_from_slice(lvl);
        }
    }
    let d = Sha256::digest(&out);
    out.extend_from_slice(&d);
    out
}

fn decode_cache(bytes: &[u8], def: &SetDef) -> Option<CookedSet> {
    if bytes.len() < 8 + 4 + 32 + 32 || &bytes[..8] != b"EOCC0001" {
        return None;
    }
    let (body, digest) = bytes.split_at(bytes.len() - 32);
    if Sha256::digest(body).as_slice() != digest {
        return None;
    }
    let mut at = 8;
    let u32_at = |at: &mut usize| -> Option<u32> {
        let v = u32::from_le_bytes(body.get(*at..*at + 4)?.try_into().ok()?);
        *at += 4;
        Some(v)
    };
    let res = u32_at(&mut at)?;
    if res != set_res(def) {
        return None;
    }
    let mut f = [0f32; 8];
    for v in f.iter_mut() {
        *v = f32::from_le_bytes(body.get(at..at + 4)?.try_into().ok()?);
        at += 4;
    }
    let mut maps: [Vec<Vec<u8>>; 3] = [vec![], vec![], vec![]];
    for map in maps.iter_mut() {
        let levels = u32_at(&mut at)?;
        if levels != mip_count(res) {
            return None;
        }
        for _ in 0..levels {
            let len = u32_at(&mut at)? as usize;
            map.push(body.get(at..at + len)?.to_vec());
            at += len;
        }
    }
    (at == body.len()).then(|| CookedSet {
        id: def.id.to_string(),
        kind: def.kind,
        res,
        maps,
        mean_rgb: [f[0], f[1], f[2]],
        mean_rough: f[3],
        mean_ao: f[4],
        mean_height: f[5],
        size_m: [f[6], f[7]],
        from_cache: true,
        ms: 0.0,
    })
}

/// `cook` through the cache (`cache_root`: `EO_CC0_CACHE`, else `cache_dir/cc0/`).
pub fn load_cached(root: &Path, def: &SetDef, cache_dir: Option<&Path>) -> Result<CookedSet> {
    let t = std::time::Instant::now();
    let man = Cc0Manifest::get();
    let a = man.asset(def.id).ok_or_else(|| anyhow!("{} is not in the manifest", def.id))?;
    let file = cache_root(cache_dir).map(|c| c.join(format!("{}-{}.eocc", def.id, cache_key(def, a))));
    if let Some(f) = &file {
        if let Ok(bytes) = std::fs::read(f) {
            if let Some(mut s) = decode_cache(&bytes, def) {
                s.ms = t.elapsed().as_secs_f64() * 1e3;
                return Ok(s);
            }
        }
    }
    let s = cook(root, def)?;
    if let Some(f) = &file {
        let _ = std::fs::create_dir_all(f.parent().unwrap());
        let tmp = f.with_extension("tmp");
        if std::fs::write(&tmp, encode_cache(&s)).is_ok() {
            let _ = std::fs::rename(&tmp, f);
        }
    }
    Ok(s)
}

/// The set definitions in load order: (kind, layer, def).
pub fn all_sets() -> Vec<(SetKind, u32, SetDef)> {
    let mut v: Vec<(SetKind, u32, SetDef)> = MATERIAL_SETS.iter().enumerate().map(|(i, d)| (SetKind::Material, i as u32, *d)).collect();
    v.extend(DECAL_SETS.iter().enumerate().map(|(i, d)| (SetKind::Decal, i as u32, *d)));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sets' folder is `<repo>/external/cc0` unless EO_CC0_ROOT names another.
    #[test]
    fn root_defaults_to_external() {
        let repo = eo_data::DataPaths::discover().expect("the repository").repo_root;
        match std::env::var_os("EO_CC0_ROOT").filter(|v| !v.is_empty()) {
            Some(v) => assert_eq!(root(), Some(PathBuf::from(v))),
            None => assert_eq!(root(), Some(repo.join("external").join("cc0"))),
        }
    }

    #[test]
    fn manifest_names_every_set_with_cc0_licence_and_digests() {
        let man = Cc0Manifest::get();
        for (_, _, def) in all_sets() {
            let a = man.asset(def.id).unwrap_or_else(|| panic!("{} not in the manifest", def.id));
            assert_eq!(a.licence, "CC0-1.0", "{}", a.id);
            assert!(a.page.starts_with("https://polyhaven.com/a/") || a.page.starts_with("https://ambientcg.com/a/"), "{}", a.page);
            assert!(!a.authors.is_empty(), "{}", a.id);
            assert!(a.file("albedo").is_some() && a.file("normal").is_some(), "{}", a.id);
            for f in &a.files {
                assert_eq!(f.sha256.len(), 64, "{} {}", a.id, f.path);
                assert!(f.bytes > 0);
                assert!(!f.path.contains(".."), "{}", f.path);
            }
        }
        assert!(man.downloaded_bytes > 0 && man.downloaded_bytes < 3_000_000_000);
    }

    #[test]
    fn mip_chain_stops_at_one_block() {
        assert_eq!(mip_count(2048), 10);
        assert_eq!(mip_count(1024), 9);
    }

    #[test]
    fn period_of_a_striped_height_map() {
        let (w, h) = (64usize, 256usize);
        let px: Vec<f32> = (0..w * h).map(|i| ((i / w) as f32 * std::f32::consts::TAU / 32.0).sin() * 0.5 + 0.5).collect();
        let img = Img { w, h, c: 1, px };
        let p = dominant_period_rows(&img).unwrap();
        assert!((p - 32.0).abs() < 0.6, "{p}");
    }
}
