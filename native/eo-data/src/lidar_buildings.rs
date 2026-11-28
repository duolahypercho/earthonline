//! Per-building 3D roof, setback tier, and rooftop penthouse descriptors
//! extracted from USGS 3DEP LiDAR point-cloud surveys under `external/lidar/`.
//!
//! Each `<lidar_root>/<area>/` folder (from the offline LiDAR survey, e.g.
//! `external/lidar/liberty-hill/`) may provide:
//! - `roofs.json`: fitted roof classification (`flat`, `gable`, `shed`, `complex`)
//!   with axis/pitch/eave/top measurements on the surveyed 1:1 plan;
//! - `roof-cells.json`: 0.5 m x 0.5 m per-building LiDAR surface heights above
//!   the building's ground base, relative to the footprint centroid.
//!
//! This module fuses both files into compact [`BuildingMassing`] descriptors
//! (pitched gable/hip/shed parameters, multi-tier podium+tower setbacks, and up
//! to two rooftop penthouses/mechanical bulkheads), cached deterministically in
//! `<cache_dir>/lidar-massing/<key>.eolb` with a trailing SHA-256 integrity
//! digest. When `external/lidar/` is absent or unreadable, installation returns
//! `None` cleanly with zero runtime overhead.
//!
//! Horizontal offsets (`dx`, `dz`, `hx`, `hz`, `ridge_offset_m`) are stored on
//! the surveyed 1:1 plan and scaled by `groundscale::scale()` when consumed;
//! vertical heights (`eave_h`, `ridge_h`, `y0`, `y1`) stay real 1:1 metres.

use anyhow::{Result, bail};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::f64::consts::PI;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Cache file magic and binary schema version.
pub const CACHE_MAGIC: &[u8; 4] = b"EOLB";
pub const CACHE_VERSION: u32 = 1;
/// Embedded source text so any extraction change keys into a fresh cache file.
const SOURCE: &str = include_str!("lidar_buildings.rs");

/// Primary roof silhouette form detected for a building.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum RoofForm {
    #[default]
    Flat = 0,
    Gable = 1,
    Hip = 2,
    Shed = 3,
    Stepped = 4,
}

impl RoofForm {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Gable,
            2 => Self::Hip,
            3 => Self::Shed,
            4 => Self::Stepped,
            _ => Self::Flat,
        }
    }

    pub fn is_pitched(self) -> bool {
        matches!(self, Self::Gable | Self::Hip | Self::Shed)
    }
}

/// Kind of secondary 3D volume sitting on or stepping up from a building roof.
pub const BOX_NONE: u8 = 0;
/// Upper setback storey/shaft tier (carries wall/window finish).
pub const BOX_SETBACK: u8 = 1;
/// Rooftop stair/elevator penthouse or bulkhead enclosure.
pub const BOX_PENTHOUSE: u8 = 2;
/// Rooftop HVAC / mechanical unit / cooling tower enclosure.
pub const BOX_MECHANICAL: u8 = 3;

/// One oriented 3D massing box (setback tier, penthouse, or mechanical enclosure)
/// relative to the building footprint centroid on the surveyed 1:1 plan.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MassingBox {
    /// Offset from footprint centroid, surveyed 1:1 metres (x east, z south).
    pub dx: f32,
    pub dz: f32,
    /// Half-extents along the oriented `(u, v)` axes, surveyed 1:1 metres.
    pub hx: f32,
    pub hz: f32,
    /// Bottom height above building base, real metres.
    pub y0: f32,
    /// Top height above building base, real metres.
    pub y1: f32,
    /// Orientation of the `u` axis in radians (`atan2(dz, dx)`).
    pub angle_rad: f32,
    /// `BOX_*` kind.
    pub kind: u8,
}

impl MassingBox {
    pub fn is_valid(&self) -> bool {
        self.kind != BOX_NONE && self.hx > 0.4 && self.hz > 0.4 && self.y1 > self.y0 + 0.4
    }

    /// World-space 2D footprint corners (4 points, CCW in x-east / z-south)
    /// around the drawn building centroid `(cx, cz)`, applying `plan_scale`
    /// (`groundscale::scale()`) to horizontal dimensions while keeping heights 1:1.
    pub fn world_quad(&self, cx: f64, cz: f64, plan_scale: f64) -> [[f64; 2]; 4] {
        let (s, c) = (self.angle_rad as f64).sin_cos();
        let ox = cx + self.dx as f64 * plan_scale;
        let oz = cz + self.dz as f64 * plan_scale;
        let hx = self.hx as f64 * plan_scale;
        let hz = self.hz as f64 * plan_scale;
        let ux = c * hx;
        let uz = s * hx;
        let vx = -s * hz;
        let vz = c * hz;
        [
            [ox - ux - vx, oz - uz - vz],
            [ox + ux - vx, oz + uz - vz],
            [ox + ux + vx, oz + uz + vz],
            [ox - ux + vx, oz - uz + vz],
        ]
    }
}

/// Compact per-building 3D massing descriptor extracted from LiDAR.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BuildingMassing {
    pub form: RoofForm,
    /// Main wall/eave height above building base (real metres).
    pub eave_h: f32,
    /// Roof ridge / highest roof plateau height above building base (real metres).
    pub ridge_h: f32,
    /// Principal ridge or slope direction angle (radians, x-east z-south).
    pub axis_rad: f32,
    /// Lateral offset of a gable ridge from centroid (surveyed 1:1 metres).
    pub ridge_offset_m: f32,
    /// Confidence in `[0.0, 1.0]` (LiDAR measured typically `0.65..1.0`).
    pub confidence: f32,
    /// Up to 2 upper setback tiers / rooftop penthouses / mechanical boxes.
    pub boxes: [MassingBox; 2],
    pub box_count: u8,
}

impl BuildingMassing {
    pub fn boxes(&self) -> &[MassingBox] {
        let n = (self.box_count as usize).min(2);
        &self.boxes[..n]
    }

    /// Maximum height above building base across roof ridge and all rooftop boxes.
    pub fn max_h(&self) -> f32 {
        let mut h = self.eave_h.max(self.ridge_h);
        for b in self.boxes() {
            if b.is_valid() {
                h = h.max(b.y1);
            }
        }
        h
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LidarMassingEntry {
    pub hash: u32,
    pub id: String,
    pub massing: BuildingMassing,
}

/// Loaded citywide/area LiDAR building massing table, indexed for O(log N) lookup.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LidarMassingTable {
    entries: Vec<LidarMassingEntry>,
    pub key: String,
    pub areas: Vec<String>,
}

impl LidarMassingTable {
    pub fn from_entries(mut entries: Vec<LidarMassingEntry>, key: String, areas: Vec<String>) -> Self {
        entries.sort_by(|a, b| a.hash.cmp(&b.hash).then_with(|| a.id.cmp(&b.id)));
        entries.dedup_by(|a, b| a.id == b.id);
        Self { entries, key, areas }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Lookup by raw record key (`"258922197"` or `"ds1234"`) or prefixed `"sf-building-..."`.
    pub fn get(&self, record_id: &str) -> Option<BuildingMassing> {
        let key = record_id.strip_prefix("sf-building-").unwrap_or(record_id);
        let h = crate::roof_colours::key_hash(key);
        let mut idx = self.entries.partition_point(|e| e.hash < h);
        while let Some(e) = self.entries.get(idx) {
            if e.hash != h {
                break;
            }
            if e.id == key {
                return Some(e.massing);
            }
            idx += 1;
        }
        None
    }

    pub fn iter(&self) -> impl Iterator<Item = &LidarMassingEntry> {
        self.entries.iter()
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64 + self.entries.len() * 100);
        out.extend_from_slice(CACHE_MAGIC);
        out.extend_from_slice(&CACHE_VERSION.to_le_bytes());
        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        let kb = self.key.as_bytes();
        out.extend_from_slice(&(kb.len() as u16).to_le_bytes());
        out.extend_from_slice(kb);
        out.extend_from_slice(&(self.areas.len() as u16).to_le_bytes());
        for a in &self.areas {
            let ab = a.as_bytes();
            out.extend_from_slice(&(ab.len() as u16).to_le_bytes());
            out.extend_from_slice(ab);
        }
        for e in &self.entries {
            out.extend_from_slice(&e.hash.to_le_bytes());
            let ib = e.id.as_bytes();
            out.extend_from_slice(&(ib.len() as u16).to_le_bytes());
            out.extend_from_slice(ib);
            let m = &e.massing;
            out.push(m.form as u8);
            out.push(m.box_count.min(2));
            for v in [m.eave_h, m.ridge_h, m.axis_rad, m.ridge_offset_m, m.confidence] {
                out.extend_from_slice(&v.to_le_bytes());
            }
            for b in &m.boxes {
                out.push(b.kind);
                for v in [b.dx, b.dz, b.hx, b.hz, b.y0, b.y1, b.angle_rad] {
                    out.extend_from_slice(&v.to_le_bytes());
                }
            }
        }
        let digest = sha256(&out);
        out.extend_from_slice(&digest);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 16 + 32 || &bytes[0..4] != CACHE_MAGIC {
            bail!("lidar massing: not an EOLB cache file");
        }
        let (body, digest) = bytes.split_at(bytes.len() - 32);
        if sha256(body) != digest {
            bail!("lidar massing: payload SHA-256 mismatch");
        }
        let mut at = 4usize;
        let take = |at: &mut usize, n: usize| -> Result<&[u8]> {
            let s = body.get(*at..*at + n).ok_or_else(|| anyhow::anyhow!("lidar massing: truncated"))?;
            *at += n;
            Ok(s)
        };
        let rd_u32 = |at: &mut usize| -> Result<u32> { Ok(u32::from_le_bytes(take(at, 4)?.try_into().unwrap())) };
        let rd_u16 = |at: &mut usize| -> Result<u16> { Ok(u16::from_le_bytes(take(at, 2)?.try_into().unwrap())) };
        let rd_f32 = |at: &mut usize| -> Result<f32> { Ok(f32::from_le_bytes(take(at, 4)?.try_into().unwrap())) };

        let ver = rd_u32(&mut at)?;
        if ver != CACHE_VERSION {
            bail!("lidar massing: cache version {ver} != {CACHE_VERSION}");
        }
        let count = rd_u32(&mut at)? as usize;
        let key_len = rd_u16(&mut at)? as usize;
        let key = std::str::from_utf8(take(&mut at, key_len)?)?.to_string();
        let area_count = rd_u16(&mut at)? as usize;
        let mut areas = Vec::with_capacity(area_count);
        for _ in 0..area_count {
            let al = rd_u16(&mut at)? as usize;
            areas.push(std::str::from_utf8(take(&mut at, al)?)?.to_string());
        }
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let hash = rd_u32(&mut at)?;
            let id_len = rd_u16(&mut at)? as usize;
            let id = std::str::from_utf8(take(&mut at, id_len)?)?.to_string();
            let header = take(&mut at, 2)?;
            let form = RoofForm::from_u8(header[0]);
            let box_count = header[1].min(2);
            let eave_h = rd_f32(&mut at)?;
            let ridge_h = rd_f32(&mut at)?;
            let axis_rad = rd_f32(&mut at)?;
            let ridge_offset_m = rd_f32(&mut at)?;
            let confidence = rd_f32(&mut at)?;
            let mut boxes = [MassingBox::default(); 2];
            for b in &mut boxes {
                b.kind = take(&mut at, 1)?[0];
                b.dx = rd_f32(&mut at)?;
                b.dz = rd_f32(&mut at)?;
                b.hx = rd_f32(&mut at)?;
                b.hz = rd_f32(&mut at)?;
                b.y0 = rd_f32(&mut at)?;
                b.y1 = rd_f32(&mut at)?;
                b.angle_rad = rd_f32(&mut at)?;
            }
            entries.push(LidarMassingEntry {
                hash,
                id,
                massing: BuildingMassing { form, eave_h, ridge_h, axis_rad, ridge_offset_m, confidence, boxes, box_count },
            });
        }
        if at != body.len() {
            bail!("lidar massing: trailing bytes");
        }
        Ok(Self { entries, key, areas })
    }
}

// ---------------------------------------------------------------------------
// Discovery, extraction from external/lidar/<area>/, and deterministic cache
// ---------------------------------------------------------------------------

/// Resolve the LiDAR root directory: `EO_LIDAR_DIR` if set and non-empty,
/// else `<repo_root>/external/lidar` when default external folders are enabled.
pub fn lidar_root(repo_root: &Path) -> Option<PathBuf> {
    if std::env::var("EO_LIDAR_MASSING").is_ok_and(|v| v == "off" || v == "0") {
        return None;
    }
    if let Some(v) = std::env::var_os("EO_LIDAR_DIR").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(v));
    }
    if crate::external::default_enabled() {
        let p = repo_root.join(crate::external::DIR).join("lidar");
        if p.is_dir() {
            return Some(p);
        }
    }
    None
}

pub fn cache_path(cache_dir: &Path, key: &str) -> PathBuf {
    cache_dir.join("lidar-massing").join(format!("{key}.eolb"))
}

/// List sorted area directories containing at least one of `roofs.json` or `roof-cells.json`.
fn scan_areas(root: &Path) -> Vec<(String, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(root) else { return vec![] };
    let mut areas: Vec<(String, PathBuf)> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir() && (p.join("roofs.json").is_file() || p.join("roof-cells.json").is_file()))
        .map(|p| {
            let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            (name, p)
        })
        .collect();
    areas.sort_by(|a, b| a.0.cmp(&b.0));
    areas
}

/// Compute a deterministic 16-hex-char cache key over our extractor source and
/// every input file's bytes under `lidar_root`.
pub fn inputs_key(areas: &[(String, PathBuf)]) -> String {
    let mut buf = Vec::with_capacity(2048);
    buf.extend_from_slice(b"eo-lidar-buildings");
    buf.extend_from_slice(&CACHE_VERSION.to_le_bytes());
    buf.extend_from_slice(&sha256(SOURCE.as_bytes()));
    for (name, dir) in areas {
        buf.extend_from_slice(name.as_bytes());
        buf.push(0);
        for fname in ["manifest.json", "roofs.json", "roof-cells.json"] {
            let p = dir.join(fname);
            if let Ok(bytes) = std::fs::read(&p) {
                buf.extend_from_slice(fname.as_bytes());
                buf.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
                buf.extend_from_slice(&sha256(&bytes));
            }
        }
    }
    let d = sha256(&buf);
    d.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

#[derive(Deserialize)]
struct RoofsFile {
    #[serde(default)]
    roofs: Vec<RawRoof>,
}

#[derive(Deserialize)]
struct RawRoof {
    id: String,
    #[serde(default)]
    form: String,
    #[serde(default)]
    axis_deg: Option<f64>,
    #[serde(default)]
    pitch_deg: Option<f64>,
    #[serde(default)]
    ridge_offset_m: Option<f64>,
    #[serde(default)]
    eave_m: Option<f64>,
    #[serde(default)]
    down_deg: Option<f64>,
    #[serde(default)]
    top_m: Option<f64>,
    #[serde(default)]
    footprint_height_m: Option<f64>,
    #[serde(default)]
    rms_m: Option<f64>,
}

#[derive(Deserialize)]
struct RawRoofCells {
    id: String,
    #[serde(default)]
    cells: Vec<[f32; 3]>,
}

/// Extract building massing from a single `<area>/` folder's JSON files.
pub fn extract_area(area_dir: &Path) -> Result<Vec<LidarMassingEntry>> {
    let mut roofs_by_id: BTreeMap<String, RawRoof> = BTreeMap::new();
    let roofs_path = area_dir.join("roofs.json");
    if roofs_path.is_file() {
        let parsed: RoofsFile = serde_json::from_slice(&std::fs::read(&roofs_path)?)?;
        for r in parsed.roofs {
            roofs_by_id.insert(r.id.clone(), r);
        }
    }
    let mut cells_by_id: BTreeMap<String, RawRoofCells> = BTreeMap::new();
    let cells_path = area_dir.join("roof-cells.json");
    if cells_path.is_file() {
        let parsed: Vec<RawRoofCells> = serde_json::from_slice(&std::fs::read(&cells_path)?)?;
        for c in parsed {
            cells_by_id.insert(c.id.clone(), c);
        }
    }
    let mut ids: Vec<String> = roofs_by_id.keys().chain(cells_by_id.keys()).cloned().collect();
    ids.sort();
    ids.dedup();

    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let raw_roof = roofs_by_id.get(&id);
        let raw_cells = cells_by_id.get(&id);
        if let Some(massing) = synthesize_massing(raw_roof, raw_cells) {
            out.push(LidarMassingEntry { hash: crate::roof_colours::key_hash(&id), id, massing });
        }
    }
    Ok(out)
}

fn percentile(sorted: &[f32], p: f32) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() - 1) as f32 * p.clamp(0.0, 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// Principal axis orientation (radians) and extents of a 2D point cloud.
fn pca_frame(pts: &[[f32; 3]]) -> (f64, [f64; 2], f64, f64) {
    let n = pts.len().max(1) as f64;
    let (mut mx, mut mz) = (0.0, 0.0);
    for p in pts {
        mx += p[0] as f64;
        mz += p[1] as f64;
    }
    mx /= n;
    mz /= n;
    let (mut sxx, mut sxz, mut szz) = (0.0, 0.0, 0.0);
    for p in pts {
        let (dx, dz) = (p[0] as f64 - mx, p[1] as f64 - mz);
        sxx += dx * dx;
        sxz += dx * dz;
        szz += dz * dz;
    }
    let angle = 0.5 * (2.0 * sxz).atan2(sxx - szz);
    let (s, c) = angle.sin_cos();
    let (mut u0, mut u1, mut v0, mut v1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for p in pts {
        let (dx, dz) = (p[0] as f64 - mx, p[1] as f64 - mz);
        let u = dx * c + dz * s;
        let v = -dx * s + dz * c;
        u0 = u0.min(u);
        u1 = u1.max(u);
        v0 = v0.min(v);
        v1 = v1.max(v);
    }
    (angle, [mx, mz], (u1 - u0).max(0.5) * 0.5, (v1 - v0).max(0.5) * 0.5)
}

/// Fit a 2-tier Otsu threshold to detect podium + setback tower/upper-storey splits.
/// Returns `Some((lower_h, upper_h, threshold))` when a clear step of >= 2.2 m exists.
fn detect_stepped_split(sorted_h: &[f32]) -> Option<(f32, f32, f32)> {
    if sorted_h.len() < 40 {
        return None;
    }
    let lo = percentile(sorted_h, 0.15);
    let hi = percentile(sorted_h, 0.92);
    if hi - lo < 2.2 {
        return None;
    }
    let n = sorted_h.len() as f64;
    let total_sum: f64 = sorted_h.iter().map(|&v| v as f64).sum();
    let mut best_var = -1.0f64;
    let mut best_idx = 0usize;
    let mut sum0 = 0.0f64;
    let min_i = (sorted_h.len() * 18) / 100;
    let max_i = (sorted_h.len() * 82) / 100;
    for (i, &h) in sorted_h.iter().enumerate() {
        sum0 += h as f64;
        if i < min_i || i > max_i {
            continue;
        }
        let w0 = (i + 1) as f64;
        let w1 = n - w0;
        let m0 = sum0 / w0;
        let m1 = (total_sum - sum0) / w1;
        let diff = m1 - m0;
        let between = w0 * w1 * diff * diff;
        if between > best_var {
            best_var = between;
            best_idx = i;
        }
    }
    if best_var <= 0.0 {
        return None;
    }
    let low_slice = &sorted_h[..=best_idx];
    let hi_slice = &sorted_h[best_idx + 1..];
    let low_h = percentile(low_slice, 0.5);
    let hi_h = percentile(hi_slice, 0.55);
    // Reject gradual slopes (gable/shed): a stepped massing has low within-class spread.
    let spread_low = percentile(low_slice, 0.8) - percentile(low_slice, 0.2);
    let spread_hi = percentile(hi_slice, 0.8) - percentile(hi_slice, 0.2);
    if hi_h - low_h >= 2.2 && spread_low < (hi_h - low_h) * 0.65 && spread_hi < (hi_h - low_h) * 0.75 {
        Some((low_h, hi_h, 0.5 * (low_h + hi_h)))
    } else {
        None
    }
}

/// Cluster protruding cells on the 0.5 m LiDAR grid into up to 2 oriented boxes
/// (penthouses, stair/elevator bulkheads, mechanical enclosures, or upper setback tiers).
fn extract_protrusion_boxes(cells: &[[f32; 3]], base_plateau: f32, min_rise: f32, max_share: f32, axis_rad: f64, kind_large: u8, kind_small: u8) -> Vec<MassingBox> {
    const CELL_M: f32 = 0.5;
    let total = cells.len().max(1);
    let mut grid: HashMap<(i32, i32), [f32; 3]> = HashMap::new();
    for &c in cells {
        if c[2] >= base_plateau + min_rise {
            let ix = (c[0] / CELL_M).round() as i32;
            let iz = (c[1] / CELL_M).round() as i32;
            grid.entry((ix, iz)).and_modify(|e| e[2] = e[2].max(c[2])).or_insert(c);
        }
    }
    if grid.len() < 8 {
        return vec![];
    }
    let mut visited: HashMap<(i32, i32), bool> = HashMap::with_capacity(grid.len());
    let mut clusters: Vec<Vec<[f32; 3]>> = vec![];
    let mut q = VecDeque::new();
    let mut keys: Vec<(i32, i32)> = grid.keys().copied().collect();
    keys.sort_unstable();
    for k in keys {
        if visited.contains_key(&k) {
            continue;
        }
        let mut comp = vec![];
        visited.insert(k, true);
        q.push_back(k);
        while let Some((ix, iz)) = q.pop_front() {
            if let Some(&pt) = grid.get(&(ix, iz)) {
                comp.push(pt);
            }
            for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                let nb = (ix + dx, iz + dz);
                if grid.contains_key(&nb) && !visited.contains_key(&nb) {
                    visited.insert(nb, true);
                    q.push_back(nb);
                }
            }
        }
        // Require at least 3 m^2 (12 cells at 0.5m) and at most max_share of footprint.
        if comp.len() >= 12 && (comp.len() as f32 / total as f32) <= max_share {
            clusters.push(comp);
        }
    }
    let (s, c) = axis_rad.sin_cos();
    let mut scored: Vec<(f32, MassingBox)> = vec![];
    for comp in clusters {
        let mut hs: Vec<f32> = comp.iter().map(|p| p[2]).collect();
        hs.sort_by(|a, b| a.total_cmp(b));
        let top_h = percentile(&hs, 0.85);
        let rise = top_h - base_plateau;
        if rise < min_rise {
            continue;
        }
        let (mut u0, mut u1, mut v0, mut v1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for p in &comp {
            let u = p[0] as f64 * c + p[1] as f64 * s;
            let v = -p[0] as f64 * s + p[1] as f64 * c;
            u0 = u0.min(u);
            u1 = u1.max(u);
            v0 = v0.min(v);
            v1 = v1.max(v);
        }
        let hx = ((u1 - u0) * 0.5 + 0.25).clamp(0.7, 40.0);
        let hz = ((v1 - v0) * 0.5 + 0.25).clamp(0.7, 40.0);
        let uc = 0.5 * (u0 + u1);
        let vc = 0.5 * (v0 + v1);
        let dx = uc * c - vc * s;
        let dz = uc * s + vc * c;
        let area = (4.0 * hx * hz) as f32;
        let kind = if area >= 14.0 && rise >= 2.2 { kind_large } else { kind_small };
        let b = MassingBox {
            dx: dx as f32,
            dz: dz as f32,
            hx: hx as f32,
            hz: hz as f32,
            y0: base_plateau,
            y1: top_h.min(base_plateau + 18.0),
            angle_rad: axis_rad as f32,
            kind,
        };
        scored.push((area * rise, b));
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored.into_iter().take(2).map(|(_, b)| b).collect()
}

/// Test whether a cell cloud fits a 4-sided hipped roof better than a flat cap.
fn fit_hip_or_gable(cells: &[[f32; 3]], axis_rad: f64, center: [f64; 2], hu: f64, hv: f64, eave_h: f32, ridge_h: f32) -> Option<RoofForm> {
    let rise = (ridge_h - eave_h) as f64;
    if rise < 1.25 || hu < 1.8 || hv < 1.8 || cells.len() < 30 {
        return None;
    }
    let (s, c) = axis_rad.sin_cos();
    let (mut err_flat, mut err_gable, mut err_hip) = (0.0f64, 0.0f64, 0.0f64);
    let mean_h = cells.iter().map(|p| p[2] as f64).sum::<f64>() / cells.len() as f64;
    for p in cells {
        let dx = p[0] as f64 - center[0];
        let dz = p[1] as f64 - center[1];
        let u = (dx * c + dz * s).abs() / hu.max(0.5);
        let v = (-dx * s + dz * c).abs() / hv.max(0.5);
        let h = p[2] as f64;
        // Gable ridge runs along the longer axis (u): height falls with |v|.
        let h_gable = eave_h as f64 + rise * (1.0 - v).clamp(0.0, 1.0);
        // Hip roof falls toward ALL four edges (pyramid/hip ridge).
        let h_hip = eave_h as f64 + rise * (1.0 - u.max(v)).clamp(0.0, 1.0);
        err_flat += (h - mean_h).powi(2);
        err_gable += (h - h_gable).powi(2);
        err_hip += (h - h_hip).powi(2);
    }
    let best_pitched = err_gable.min(err_hip);
    // Require pitched fit to reduce squared residual significantly over flat.
    if best_pitched < err_flat * 0.68 {
        if err_hip < err_gable * 0.92 {
            Some(RoofForm::Hip)
        } else {
            Some(RoofForm::Gable)
        }
    } else {
        None
    }
}

fn synthesize_massing(raw_roof: Option<&RawRoof>, raw_cells: Option<&RawRoofCells>) -> Option<BuildingMassing> {
    // Filter valid roof cells (exclude ground/alley spill below 2 m).
    let valid_cells: Vec<[f32; 3]> = raw_cells
        .map(|rc| rc.cells.iter().copied().filter(|c| c[2].is_finite() && c[2] > 2.0 && c[2] < 450.0).collect())
        .unwrap_or_default();

    let mut hs: Vec<f32> = valid_cells.iter().map(|c| c[2]).collect();
    hs.sort_by(|a, b| a.total_cmp(b));

    let (axis_pca, center_pca, hu_pca, hv_pca) = if valid_cells.len() >= 12 {
        pca_frame(&valid_cells)
    } else {
        (0.0, [0.0, 0.0], 4.0, 4.0)
    };

    let mut m = BuildingMassing {
        form: RoofForm::Flat,
        eave_h: 0.0,
        ridge_h: 0.0,
        axis_rad: axis_pca as f32,
        ridge_offset_m: 0.0,
        confidence: if !valid_cells.is_empty() { 0.9 } else { 0.75 },
        boxes: [MassingBox::default(); 2],
        box_count: 0,
    };

    // Baseline heights from LiDAR cells or roofs.json.
    let cell_p25 = percentile(&hs, 0.25);
    let cell_p55 = percentile(&hs, 0.55);
    let cell_p95 = percentile(&hs, 0.95);
    let json_top = raw_roof.and_then(|r| r.top_m).or_else(|| raw_roof.and_then(|r| r.footprint_height_m)).unwrap_or(0.0) as f32;

    if hs.is_empty() && json_top <= 1.0 {
        return None;
    }

    let form_str = raw_roof.map(|r| r.form.as_str()).unwrap_or("");
    match form_str {
        "gable" => {
            m.form = RoofForm::Gable;
            let pitch_rise = raw_roof.and_then(|r| r.pitch_deg).map(|d| (d.clamp(12.0, 60.0).to_radians().tan() * hv_pca.clamp(2.0, 7.0)) as f32).unwrap_or(2.2);
            let eave = raw_roof.and_then(|r| r.eave_m).map(|v| v as f32).unwrap_or_else(|| if !hs.is_empty() { cell_p25 } else { (json_top - pitch_rise).max(3.0) });
            let ridge = raw_roof.and_then(|r| r.top_m).map(|v| v as f32).unwrap_or_else(|| if !hs.is_empty() { cell_p95 } else { json_top.max(eave + pitch_rise) });
            m.eave_h = eave.max(2.5);
            m.ridge_h = ridge.max(m.eave_h + 0.8);
            if let Some(deg) = raw_roof.and_then(|r| r.axis_deg) {
                m.axis_rad = (deg * PI / 180.0) as f32;
            }
            m.ridge_offset_m = raw_roof.and_then(|r| r.ridge_offset_m).unwrap_or(0.0).clamp(-6.0, 6.0) as f32;
            if let Some(rms) = raw_roof.and_then(|r| r.rms_m) {
                m.confidence = (1.0 - (rms as f32 * 0.4)).clamp(0.5, 1.0);
            }
        }
        "shed" => {
            m.form = RoofForm::Shed;
            let pitch_rise = raw_roof.and_then(|r| r.pitch_deg).map(|d| (d.clamp(8.0, 50.0).to_radians().tan() * hv_pca.clamp(2.0, 8.0)) as f32).unwrap_or(1.8);
            let eave = if !hs.is_empty() { cell_p25 } else { (json_top - pitch_rise).max(3.0) };
            let ridge = raw_roof.and_then(|r| r.top_m).map(|v| v as f32).unwrap_or_else(|| if !hs.is_empty() { cell_p95 } else { json_top.max(eave + pitch_rise) });
            m.eave_h = eave.max(2.5);
            m.ridge_h = ridge.max(m.eave_h + 0.6);
            if let Some(deg) = raw_roof.and_then(|r| r.down_deg) {
                m.axis_rad = (deg * PI / 180.0) as f32;
            }
        }
        "complex" => {
            // Complex LiDAR roofs are either stepped multi-tier setbacks, hip/gable roofs, or flat roofs with large penthouses.
            let plateau = if !hs.is_empty() { cell_p55 } else { json_top };
            let peak = if !hs.is_empty() { cell_p95 } else { json_top };
            if let Some((low_h, hi_h, thresh)) = detect_stepped_split(&hs) {
                m.form = RoofForm::Stepped;
                m.eave_h = low_h.max(2.5);
                m.ridge_h = hi_h.max(m.eave_h + 2.0);
                let upper_boxes = extract_protrusion_boxes(&valid_cells, low_h, (thresh - low_h).max(1.8), 0.78, axis_pca, BOX_SETBACK, BOX_PENTHOUSE);
                for (i, b) in upper_boxes.into_iter().take(2).enumerate() {
                    m.boxes[i] = b;
                    m.box_count = (i + 1) as u8;
                }
            } else if let Some(pitched) = fit_hip_or_gable(&valid_cells, axis_pca, center_pca, hu_pca, hv_pca, cell_p25.max(2.5), peak) {
                m.form = pitched;
                m.eave_h = cell_p25.max(2.5);
                m.ridge_h = peak.max(m.eave_h + 1.2);
                m.axis_rad = axis_pca as f32;
            } else {
                m.form = RoofForm::Flat;
                m.eave_h = plateau.max(2.5);
                m.ridge_h = m.eave_h;
                let boxes = extract_protrusion_boxes(&valid_cells, m.eave_h, 1.5, 0.48, axis_pca, BOX_PENTHOUSE, BOX_MECHANICAL);
                for (i, b) in boxes.into_iter().take(2).enumerate() {
                    m.boxes[i] = b;
                    m.box_count = (i + 1) as u8;
                }
            }
        }
        _ => {
            // Flat (or cells-only record): preserve exact plateau height and extract any real rooftop bulkheads / mechanical enclosures.
            let plateau = if !hs.is_empty() { cell_p55 } else { json_top };
            m.form = RoofForm::Flat;
            m.eave_h = plateau.max(2.5);
            m.ridge_h = m.eave_h;
            if !valid_cells.is_empty() {
                let boxes = extract_protrusion_boxes(&valid_cells, m.eave_h, 1.5, 0.42, axis_pca, BOX_PENTHOUSE, BOX_MECHANICAL);
                for (i, b) in boxes.into_iter().take(2).enumerate() {
                    m.boxes[i] = b;
                    m.box_count = (i + 1) as u8;
                }
            }
        }
    }

    // Sanity clamp on heights.
    if !(m.eave_h.is_finite() && m.eave_h >= 2.0) {
        return None;
    }
    m.ridge_h = m.ridge_h.max(m.eave_h);
    Some(m)
}

/// Load the cached LiDAR massing table or cook it deterministically from `lidar_root`.
pub fn load_or_cook(lidar_root: &Path, cache_dir: Option<&Path>) -> Option<LidarMassingTable> {
    let areas = scan_areas(lidar_root);
    if areas.is_empty() {
        return None;
    }
    let key = inputs_key(&areas);
    if let Some(cdir) = cache_dir {
        let p = cache_path(cdir, &key);
        if let Ok(bytes) = std::fs::read(&p) {
            if let Ok(table) = LidarMassingTable::decode(&bytes) {
                if table.key == key {
                    return Some(table);
                }
            }
        }
    }
    let mut all = vec![];
    let mut area_names = vec![];
    for (name, dir) in &areas {
        match extract_area(dir) {
            Ok(entries) => {
                area_names.push(name.clone());
                all.extend(entries);
            }
            Err(e) => {
                eprintln!("lidar massing: skipping area {name}: {e:#}");
            }
        }
    }
    let table = LidarMassingTable::from_entries(all, key.clone(), area_names);
    if let Some(cdir) = cache_dir {
        let p = cache_path(cdir, &key);
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let tmp = p.with_extension(format!("tmp{}", std::process::id()));
        if std::fs::write(&tmp, table.encode()).is_ok() {
            let _ = std::fs::rename(&tmp, &p);
        }
    }
    Some(table)
}

static TABLE: OnceLock<Option<LidarMassingTable>> = OnceLock::new();

/// Install the process-wide LiDAR massing table (once per process; absent or
/// unreadable `external/lidar/` cleanly installs `None`).
pub fn install(repo_root: &Path, cache_dir: Option<&Path>) -> Option<&'static LidarMassingTable> {
    TABLE
        .get_or_init(|| {
            let root = lidar_root(repo_root)?;
            let t = load_or_cook(&root, cache_dir)?;
            eprintln!("lidar massing: {} buildings from {} ({:?}, key {})", t.len(), root.display(), t.areas, t.key);
            Some(t)
        })
        .as_ref()
}

/// Installed LiDAR massing table (`None` before `install`, or if absent).
pub fn table() -> Option<&'static LidarMassingTable> {
    TABLE.get().and_then(|t| t.as_ref())
}

/// Lookup one building's LiDAR massing from the installed table.
pub fn get(record_id: &str) -> Option<BuildingMassing> {
    table()?.get(record_id)
}

/// Cache tag for downstream bakes (empty when no LiDAR table is installed).
pub fn cache_tag() -> String {
    match table() {
        Some(t) if !t.is_empty() => format!("lidar-massing-v{CACHE_VERSION}-{}", t.key),
        _ => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Zero-dependency FIPS 180-4 SHA-256 (keeps eo-data free of new crate deps).
// ---------------------------------------------------------------------------

pub(crate) fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, delta) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(delta);
        }
    }
    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_fips_vector() {
        let got: String = sha256(b"abc").iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(got, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn table_round_trips_and_rejects_corruption() {
        let m = BuildingMassing {
            form: RoofForm::Gable,
            eave_h: 10.5,
            ridge_h: 14.2,
            axis_rad: 0.75,
            ridge_offset_m: -0.4,
            confidence: 0.92,
            boxes: [
                MassingBox { dx: 1.0, dz: -2.0, hx: 3.0, hz: 2.5, y0: 10.5, y1: 13.5, angle_rad: 0.5, kind: BOX_PENTHOUSE },
                MassingBox::default(),
            ],
            box_count: 1,
        };
        let t = LidarMassingTable::from_entries(
            vec![LidarMassingEntry { hash: crate::roof_colours::key_hash("258922197"), id: "258922197".into(), massing: m }],
            "0123456789abcdef".into(),
            vec!["liberty-hill".into()],
        );
        let bytes = t.encode();
        let dec = LidarMassingTable::decode(&bytes).unwrap();
        assert_eq!(dec, t);
        assert_eq!(dec.get("258922197"), Some(m));
        assert_eq!(dec.get("sf-building-258922197"), Some(m));
        assert_eq!(dec.get("missing"), None);

        let mut bad = bytes.clone();
        bad[20] ^= 0x55;
        assert!(LidarMassingTable::decode(&bad).is_err());
        assert!(LidarMassingTable::decode(&bytes[..bytes.len() - 4]).is_err());
    }

    #[test]
    fn extracts_real_liberty_hill_lidar_if_present() {
        let Ok(paths) = crate::DataPaths::discover() else { return };
        let Some(root) = lidar_root(&paths.repo_root) else { return };
        let tmp = std::env::temp_dir().join(format!("eo-lidar-massing-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let t1 = load_or_cook(&root, Some(&tmp)).expect("liberty-hill lidar cooks");
        assert!(t1.len() >= 190, "expected >=190 buildings, got {}", t1.len());
        let gables = t1.iter().filter(|e| e.massing.form == RoofForm::Gable).count();
        let stepped = t1.iter().filter(|e| e.massing.form == RoofForm::Stepped).count();
        let with_boxes = t1.iter().filter(|e| e.massing.box_count > 0).count();
        assert!(gables >= 25, "expected real gables in Liberty Hill, got {gables}");
        assert!(stepped + with_boxes >= 20, "expected real stepped tiers or penthouses, got stepped={stepped}, boxes={with_boxes}");
        // Reload from deterministic cache and verify exact equality.
        let t2 = load_or_cook(&root, Some(&tmp)).expect("cache reload");
        assert_eq!(t1, t2);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
