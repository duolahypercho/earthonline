//! The sea's exposure (water iteration 2, water-1): how much of the open
//! Pacific's swell and of the Gate's wind each stretch of water gets, baked
//! once on a coarse grid from the land around it.
//!
//! What it stands in for: a wave model over the bathymetry (refraction,
//! diffraction, shoaling) and a wind model over the terrain. Neither exists
//! here, so the exposure is geometric (a stand-in, not a simulation):
//!
//! - **Swell share** (R): the share of the swell's directions (its cos-2s
//!   spread about the travel direction, `waves::SWELL_DIR`, `SWELL_SPREAD`)
//!   along which the open sea reaches the point without crossing land within
//!   `RAY_M`, then blurred over `DIFFRACT_M` (waves bend round a headland
//!   over about the square root of their wavelength times the distance:
//!   300 m swell 0.5-1 km behind the headland, INFERRED). It takes the place
//!   of the old x ramp across the Gate (`wg_ocean`) for each place's waves
//!   and the body water.
//! - **Wind ratio** (G): the wind over the water against the shore station's
//!   (`climate::strait_wind`, NOAA 9414290; 1 at the station's open water,
//!   which lies in a lee of its own): the lee of high ground upwind
//!   (an upwind horizon steeper than `LEE_TAN`, a hill's wake reaching some
//!   10-60 hill heights downwind; Kaimal and Finnigan 1994, "Atmospheric
//!   Boundary Layer Flows", ch. 5, INFERRED thresholds) and the Gate's
//!   channelling (the westerly squeezed between the headlands: the
//!   across-wind width of open water against the width at the station, to the
//!   one half power, held to `GAIN_RANGE`, INFERRED).
//! - **Fetch factor** (B): fetch-limited growth, Hs proportional to the
//!   square root of the fetch (the law `waves::WindSea::hs` uses), against the
//!   bay's `waves::BAY_FETCH_M`, from the effective fetch upwind of the point
//!   (the Shore Protection Manual's: radials every 6 degrees to 42 degrees
//!   either side of the wind, F = sum F_i cos^2 a_i / sum cos a_i; US Army
//!   Coastal Engineering Research Center 1984, vol. 1, eq. 3-35), then blurred
//!   over `FETCH_SPREAD_M` (the wind sea runs on into the lee of a point,
//!   INFERRED). The lee's upwind horizon is weighted over the same radials.
//!
//! The land is the city's elevation grid (USGS 3DEP lidar and NOAA CUDEM
//! topobathy, `public/data/sf/sf-elevation.bin.gz`, which reaches the Marin
//! shore of the Gate) standing over the sea, and the shoreline rings of the
//! shore field. Past the grid: open sea to the west, land (Marin) to the
//! north, the shore field (else land) elsewhere.

use crate::field::ShoreField;
use crate::shoreline::SEA_LEVEL_Y;
use anyhow::{Context, Result, bail};
use std::sync::OnceLock;

/// Bumped when the bake changes (part of the cache key).
pub const EXPOSURE_VERSION: &str = "water-exposure-v3";
/// The bake's cell (game metres) and extent (game x min, z min, x max, z max).
pub const CELL_M: f64 = 100.0;
pub const DOMAIN: [f64; 4] = [-40_000.0, -30_000.0, 20_000.0, 15_000.0];
/// How far a ray looks for land (m).
pub const RAY_M: f64 = 40_000.0;
/// The diffraction blur's standard deviation (m), INFERRED (see the module note).
pub const DIFFRACT_M: f64 = 450.0;
/// The swell's directions sampled either side of its travel (degrees, step).
pub const SWELL_FAN_DEG: f64 = 60.0;
pub const SWELL_FAN_STEP_DEG: f64 = 5.0;
/// The westerly through the Gate blows from this bearing (compass degrees):
/// the vector mean of NOAA 9414290's April-September afternoons (12-17 h
/// local standard time, 2022-2024: 255.9-259 degrees; `golden-gate-climate.json`).
pub const WIND_FROM_DEG: f64 = 258.0;
/// The upwind horizon (rise over run) at which the lee begins and is full,
/// and the share of the wind the full lee takes (INFERRED, see the module note).
pub const LEE_TAN: [f64; 2] = [0.015, 0.08];
pub const LEE_TAKE: f64 = 0.65;
/// How far upwind high ground shelters (m).
pub const LEE_REACH_M: f64 = 6_000.0;
/// The channelling gain's range and how far across the wind the width is measured (m each way).
pub const GAIN_RANGE: [f64; 2] = [0.85, 1.3];
pub const WIDTH_REACH_M: f64 = 8_000.0;
/// The shore station (NOAA 9414290, 37.8063 N, 122.4659 W) and the open
/// water 300 m north of it where its across-wind width is measured (game m
/// at the 1.25 ground plan: x = (lon + 122.4194) 87986.25 x 1.25,
/// z = (37.778 - lat) 110574 x 1.25).
pub const STATION_LONLAT: [f64; 2] = [-122.4659, 37.8063];
/// Fetch factor floor (the first metres off a lee shore keep a ripple).
pub const FETCH_FLOOR: f64 = 0.15;
/// The effective fetch's radials: every `FETCH_RADIAL_DEG` to `FETCH_FAN_DEG` either side of the wind.
pub const FETCH_RADIAL_DEG: f64 = 6.0;
pub const FETCH_FAN_DEG: f64 = 42.0;
/// The blur of the fetch factor and the wind ratio (m, INFERRED: see the module note).
pub const FETCH_SPREAD_M: f64 = 250.0;
/// Land: the ground standing this far over the still sea (m).
pub const LAND_OVER_SEA_M: f64 = 0.5;

/// The baked exposure: RGBA8 per cell (R swell share, G wind ratio / 2, B
/// fetch factor, A 255 over water, 0 over land), row 0 at the domain's z min.
#[derive(Clone, Debug)]
pub struct Exposure {
    pub origin: [f64; 2],
    pub cell_m: f64,
    pub width: usize,
    pub height: usize,
    pub texels: Vec<[u8; 4]>,
    pub bake_ms: f64,
    pub from_cache: bool,
}

/// What the land is: the ground's game y at a game point (None: no data there).
pub trait Ground: Sync {
    fn height(&self, x: f64, z: f64) -> Option<f64>;
}

impl<F: Fn(f64, f64) -> Option<f64> + Sync> Ground for F {
    fn height(&self, x: f64, z: f64) -> Option<f64> {
        self(x, z)
    }
}

fn smooth(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The station's game position at a ground-plan scale.
pub fn station_xz(scale: f64) -> [f64; 2] {
    let [lon, lat] = STATION_LONLAT;
    [(lon + 122.4194) * 87_986.247_476_406_54 * scale, (37.778 - lat) * 110_574.0 * scale]
}

struct Land {
    w: usize,
    h: usize,
    /// height over the sea (m), 0 over water
    over: Vec<f32>,
}

impl Land {
    fn at(&self, i: i64, j: i64) -> Option<f32> {
        if i < 0 || j < 0 || i >= self.w as i64 || j >= self.h as i64 {
            return None;
        }
        Some(self.over[j as usize * self.w + i as usize])
    }
}

/// A ray from cell (i, j) along (dx, dz) (unit, cells): the distance (m) to the
/// first land, or None if it leaves the domain or passes `RAY_M` over water;
/// and the steepest upwind horizon (rise over run) of the land within `lee_m`.
fn march(land: &Land, i: usize, j: usize, dx: f64, dz: f64, max_m: f64, lee_m: f64) -> (Option<f64>, f64) {
    let steps = (max_m / CELL_M) as usize;
    let (x0, z0) = (i as f64 + 0.5, j as f64 + 0.5);
    let mut first = None;
    let mut horizon = 0.0f64;
    for k in 1..=steps {
        let d = k as f64;
        let (x, z) = (x0 + dx * d, z0 + dz * d);
        let Some(o) = land.at(x.floor() as i64, z.floor() as i64) else {
            break;
        };
        if o > 0.0 {
            let m = d * CELL_M;
            if first.is_none() {
                first = Some(m);
            }
            if m <= lee_m {
                horizon = horizon.max(o as f64 / m);
            }
            if m > lee_m {
                break;
            }
        } else if first.is_some() && d * CELL_M > lee_m {
            break;
        }
    }
    (first, horizon)
}

/// Bake the exposure over `DOMAIN` from the shore field and the ground.
pub fn bake(field: &ShoreField, ground: &dyn Ground, grid_box: Option<[f64; 4]>, scale: f64) -> Exposure {
    let t0 = std::time::Instant::now();
    let w = ((DOMAIN[2] - DOMAIN[0]) / CELL_M).round() as usize;
    let h = ((DOMAIN[3] - DOMAIN[1]) / CELL_M).round() as usize;
    let fx0 = field.origin;
    let fx1 = [field.origin[0] + field.size[0], field.origin[1] + field.size[1]];
    let threads = std::thread::available_parallelism().map(|p| p.get()).unwrap_or(4).clamp(1, 16);
    let rows_per = h.div_ceil(threads);
    // the land mask (height over the sea) at the cell centres
    let mut over = vec![0f32; w * h];
    std::thread::scope(|s| {
        for (k, chunk) in over.chunks_mut(rows_per * w).enumerate() {
            s.spawn(move || {
                for (jj, row) in chunk.chunks_mut(w).enumerate() {
                    let j = k * rows_per + jj;
                    let z = DOMAIN[1] + (j as f64 + 0.5) * CELL_M;
                    for (i, o) in row.iter_mut().enumerate() {
                        let x = DOMAIN[0] + (i as f64 + 0.5) * CELL_M;
                        let in_grid = grid_box.is_some_and(|b| x >= b[0] && x <= b[2] && z >= b[1] && z <= b[3]);
                        let in_field = x >= fx0[0] && x <= fx1[0] && z >= fx0[1] && z <= fx1[1];
                        let ring_land = in_field && field.signed_exact(x, z) < 0.0;
                        let y = if in_grid { ground.height(x, z) } else { None };
                        *o = match (y, grid_box) {
                            (Some(y), _) => {
                                let d = (y - SEA_LEVEL_Y) as f32;
                                if d > LAND_OVER_SEA_M as f32 || ring_land { d.max(1.0) } else { 0.0 }
                            }
                            (None, Some(b)) if x < b[0] => 0.0, // the open Pacific
                            (None, Some(b)) if z < b[1] => 60.0, // Marin (INFERRED: past the grid, land)
                            _ if in_field => {
                                if ring_land { 10.0 } else { 0.0 }
                            }
                            _ => 30.0,
                        };
                    }
                }
            });
        }
    });
    let land = Land { w, h, over };
    // the swell's directions (from where it comes) and their weights
    let swell_to = crate::waves::SWELL_DIR;
    let s_exp = crate::waves::SWELL_SPREAD[0];
    let mut fan = Vec::new();
    let n_fan = (SWELL_FAN_DEG / SWELL_FAN_STEP_DEG).round() as i64;
    for k in -n_fan..=n_fan {
        let off = (k as f64 * SWELL_FAN_STEP_DEG).to_radians();
        let wgt = (off * 0.5).cos().powf(2.0 * s_exp);
        let a = swell_to + off + std::f64::consts::PI; // toward where it comes from
        fan.push((a.cos(), a.sin(), wgt));
    }
    let fan_sum: f64 = fan.iter().map(|f| f.2).sum();
    // upwind (compass bearing to x east, z south) and across it
    let b = WIND_FROM_DEG.to_radians();
    let up = (b.sin(), -b.cos());
    let across = (-up.1, up.0);
    let width_at = |i: usize, j: usize| -> f64 {
        let a = march(&land, i, j, across.0, across.1, WIDTH_REACH_M, 0.0).0.unwrap_or(WIDTH_REACH_M);
        let c = march(&land, i, j, -across.0, -across.1, WIDTH_REACH_M, 0.0).0.unwrap_or(WIDTH_REACH_M);
        a + c
    };
    let st = station_xz(scale);
    let si = (((st[0] - DOMAIN[0]) / CELL_M).floor() as usize).min(w - 1);
    let sj = ((((st[1] - 300.0 * scale) - DOMAIN[1]) / CELL_M).floor() as usize).min(h - 1);
    let w_station = width_at(si, sj).max(CELL_M);
    let mut raw = vec![[0f32; 4]; w * h];
    {
        let land = &land;
        let fan = &fan;
        let width_at = &width_at;
        std::thread::scope(|s| {
            for (k, chunk) in raw.chunks_mut(rows_per * w).enumerate() {
                s.spawn(move || {
                    for (jj, row) in chunk.chunks_mut(w).enumerate() {
                        let j = k * rows_per + jj;
                        for (i, out) in row.iter_mut().enumerate() {
                            let mut open = 0.0;
                            for &(dx, dz, wgt) in fan {
                                if march(land, i, j, dx, dz, RAY_M, 0.0).0.is_none() {
                                    open += wgt;
                                }
                            }
                            // the effective fetch and the lee over the radials either side of the wind
                            let (mut f_num, mut f_den, mut h_num) = (0.0, 0.0, 0.0);
                            let n_rad = (FETCH_FAN_DEG / FETCH_RADIAL_DEG).round() as i64;
                            for k in -n_rad..=n_rad {
                                let a = (k as f64 * FETCH_RADIAL_DEG).to_radians();
                                let (ca, sa) = (a.cos(), a.sin());
                                let d = (up.0 * ca - up.1 * sa, up.0 * sa + up.1 * ca);
                                let (f, hz) = march(land, i, j, d.0, d.1, RAY_M, LEE_REACH_M);
                                f_num += f.unwrap_or(RAY_M) * ca * ca;
                                f_den += ca;
                                h_num += hz * ca;
                            }
                            let fetch = f_num / f_den;
                            let horizon = h_num / f_den;
                            let lee = 1.0 - LEE_TAKE * smooth(LEE_TAN[0], LEE_TAN[1], horizon);
                            let gain = (w_station / width_at(i, j).max(CELL_M)).sqrt().clamp(GAIN_RANGE[0], GAIN_RANGE[1]);
                            let fetch_k = (fetch / crate::waves::BAY_FETCH_M).sqrt().clamp(FETCH_FLOOR, 1.0);
                            *out = [(open / fan_sum) as f32, (lee * gain) as f32, fetch_k as f32, if land.over[j * w + i] > 0.0 { 0.0 } else { 1.0 }];
                        }
                    }
                });
            }
        });
    }
    // the diffraction blur of the swell share, and the wind sea's spread into the lee
    // (separable Gaussians, clamped at the edges)
    blur(&mut raw, w, h, 0, DIFFRACT_M / CELL_M);
    blur(&mut raw, w, h, 1, FETCH_SPREAD_M / CELL_M);
    blur(&mut raw, w, h, 2, FETCH_SPREAD_M / CELL_M);
    // the ratio is to the station's wind, so it is 1 at the station: its
    // anemometer stands on the Crissy Field shore, itself in the lee the field
    // finds under the Presidio's bluffs for the westerly, so the open strait's
    // wind is the station's over that lee (the field read about 0.6 there before)
    let g_station = raw[sj * w + si][1].max(0.2);
    for v in raw.iter_mut() {
        v[1] /= g_station;
    }
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let texels = raw.iter().map(|v| [q(v[0]), q(v[1] * 0.5), q(v[2]), q(v[3])]).collect();
    Exposure { origin: [DOMAIN[0], DOMAIN[1]], cell_m: CELL_M, width: w, height: h, texels, bake_ms: t0.elapsed().as_secs_f64() * 1e3, from_cache: false }
}

/// A separable Gaussian blur (standard deviation `sigma` cells) of channel `ch`.
fn blur(raw: &mut [[f32; 4]], w: usize, h: usize, ch: usize, sigma: f64) {
    let r = (sigma * 3.0).ceil() as i64;
    let kern: Vec<f32> = (-r..=r).map(|d| (-(d as f64 * d as f64) / (2.0 * sigma * sigma)).exp() as f32).collect();
    let ks: f32 = kern.iter().sum();
    let mut tmp = vec![0f32; w * h];
    for j in 0..h {
        for i in 0..w {
            let mut a = 0.0;
            for (t, kv) in kern.iter().enumerate() {
                let ii = (i as i64 + t as i64 - r).clamp(0, w as i64 - 1) as usize;
                a += raw[j * w + ii][ch] * kv;
            }
            tmp[j * w + i] = a / ks;
        }
    }
    for j in 0..h {
        for i in 0..w {
            let mut a = 0.0;
            for (t, kv) in kern.iter().enumerate() {
                let jj = (j as i64 + t as i64 - r).clamp(0, h as i64 - 1) as usize;
                a += tmp[jj * w + i] * kv;
            }
            raw[j * w + i][ch] = a / ks;
        }
    }
}

impl Exposure {
    /// Bilinear sample (R, G x 2, B, A as 0..1) at a game point, cell centres
    /// at origin + (i + 0.5) cell, clamped at the edges (the GPU samples it so).
    pub fn at(&self, x: f64, z: f64) -> [f64; 4] {
        let fx = ((x - self.origin[0]) / self.cell_m - 0.5).clamp(0.0, (self.width - 1) as f64);
        let fz = ((z - self.origin[1]) / self.cell_m - 0.5).clamp(0.0, (self.height - 1) as f64);
        let (i0, j0) = (fx.floor() as usize, fz.floor() as usize);
        let (i1, j1) = ((i0 + 1).min(self.width - 1), (j0 + 1).min(self.height - 1));
        let (tx, tz) = (fx - i0 as f64, fz - j0 as f64);
        let mut o = [0.0; 4];
        for (c, oc) in o.iter_mut().enumerate() {
            let g = |i: usize, j: usize| self.texels[j * self.width + i][c] as f64 / 255.0;
            let a = g(i0, j0) * (1.0 - tx) + g(i1, j0) * tx;
            let b = g(i0, j1) * (1.0 - tx) + g(i1, j1) * tx;
            *oc = a * (1.0 - tz) + b * tz;
        }
        o[1] *= 2.0;
        o
    }

    /// The cache file: magic, version, frame, texels; SHA-256 last.
    pub fn to_bytes(&self) -> Vec<u8> {
        use sha2::{Digest, Sha256};
        let mut b = b"EOWX".to_vec();
        let ver = EXPOSURE_VERSION.as_bytes();
        b.extend_from_slice(&(ver.len() as u32).to_le_bytes());
        b.extend_from_slice(ver);
        for v in [self.origin[0], self.origin[1], self.cell_m] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&(self.width as u32).to_le_bytes());
        b.extend_from_slice(&(self.height as u32).to_le_bytes());
        for t in &self.texels {
            b.extend_from_slice(t);
        }
        let d = Sha256::digest(&b);
        b.extend_from_slice(d.as_slice());
        b
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self> {
        use sha2::{Digest, Sha256};
        if b.len() < 40 || &b[..4] != b"EOWX" {
            bail!("not a water exposure file");
        }
        let (body, sum) = b.split_at(b.len() - 32);
        if Sha256::digest(body).as_slice() != sum {
            bail!("water exposure file damaged (digest)");
        }
        let mut at = 4;
        let take = |at: &mut usize, n: usize| -> Result<&[u8]> {
            let s = body.get(*at..*at + n).context("water exposure file truncated")?;
            *at += n;
            Ok(s)
        };
        let vl = u32::from_le_bytes(take(&mut at, 4)?.try_into()?) as usize;
        if take(&mut at, vl)? != EXPOSURE_VERSION.as_bytes() {
            bail!("water exposure file of another version");
        }
        let mut f = [0f64; 3];
        for v in &mut f {
            *v = f64::from_le_bytes(take(&mut at, 8)?.try_into()?);
        }
        let width = u32::from_le_bytes(take(&mut at, 4)?.try_into()?) as usize;
        let height = u32::from_le_bytes(take(&mut at, 4)?.try_into()?) as usize;
        let raw = take(&mut at, width * height * 4)?;
        let texels = raw.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
        Ok(Self { origin: [f[0], f[1]], cell_m: f[2], width, height, texels, bake_ms: 0.0, from_cache: true })
    }
}

static INSTALLED: OnceLock<Exposure> = OnceLock::new();

/// The exposure this process baked (or read), once `crate::load` has run;
/// None before, in tests that build their own field, or when the bake failed.
pub fn installed() -> Option<&'static Exposure> {
    INSTALLED.get()
}

/// Bake (or read from `cache`) the exposure for the field and the city's
/// elevation grid, and install it for the renderer.
pub fn load(repo_root: &std::path::Path, cache: Option<&std::path::Path>, field: &ShoreField, field_key: &str) -> Result<&'static Exposure> {
    let grid_path = repo_root.join("public/data/sf/sf-elevation.bin.gz");
    let grid_sha = crate::file_sha256(&grid_path)?;
    let scale = crate::ground_scale();
    let hs = eo_data::groundscale::height_scale();
    let key = {
        use sha2::{Digest, Sha256};
        let k = format!("{EXPOSURE_VERSION}|{field_key}|{grid_sha}|{scale}|{hs}");
        Sha256::digest(k.as_bytes()).iter().take(12).map(|x| format!("{x:02x}")).collect::<String>()
    };
    let path = cache.map(|c| c.join("water").join(format!("exposure-{key}.eowx")));
    if let Some(p) = &path
        && let Ok(b) = std::fs::read(p)
    {
        match Exposure::from_bytes(&b) {
            Ok(e) => return Ok(INSTALLED.get_or_init(|| e)),
            Err(e) => eprintln!("water: cached exposure refused ({e:#}); baking again"),
        }
    }
    let mut grid = eo_data::ElevationGrid::decode(&eo_data::read_maybe_gzip(&grid_path)?)?;
    grid.meta.origin_x *= scale;
    grid.meta.origin_z *= scale;
    grid.meta.cell_size *= scale;
    grid.height_scale = hs;
    let gbox = [grid.meta.origin_x, grid.meta.origin_z, grid.max_x(), grid.max_z()];
    let ground = |x: f64, z: f64| grid.sample(x, z);
    let e = bake(field, &ground, Some(gbox), scale);
    if let Some(p) = &path {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = p.with_extension("tmp");
        if std::fs::write(&tmp, e.to_bytes()).is_ok() {
            let _ = std::fs::rename(&tmp, p);
        }
    }
    Ok(INSTALLED.get_or_init(|| e))
}

#[cfg(test)]
mod tests {
    use super::*;

    // a field of open water everywhere (the rings far away), 100 m texels
    fn open_field() -> ShoreField {
        let (w, h) = (8usize, 8usize);
        let half = vec![crate::field::to_half_three(400.0); w * h * 2];
        ShoreField { origin: [-1.0e6, -1.0e6], size: [10.0, 10.0], width: w, height: h, half, range: [400.0, 400.0], beach_texels: 0, bake_ms: 0.0 }
    }

    // A wall of land running north-south at x = 0 with a 2 km gap (the Gate) at
    // z in [-1000, 1000], 200 m high: west of it the open sea, east of it a bay.
    fn gate(x: f64, z: f64) -> Option<f64> {
        if (0.0..400.0).contains(&x) && !(-1000.0..1000.0).contains(&z) { Some(SEA_LEVEL_Y + 200.0) } else { Some(SEA_LEVEL_Y - 10.0) }
    }

    #[test]
    fn the_bay_behind_a_gate_is_sheltered_and_the_gate_is_open() {
        let f = open_field();
        let e = bake(&f, &gate, Some([-40_000.0, -30_000.0, 20_000.0, 15_000.0]), 1.0);
        let sea = e.at(-5000.0, 0.0);
        let gate_mouth = e.at(200.0, 0.0);
        let bay_lee = e.at(4000.0, 6000.0);
        let bay_in_line = e.at(4000.0, 0.0);
        assert!(sea[0] > 0.95, "open sea swell {sea:?}");
        assert!(gate_mouth[0] > 0.5, "the gate's mouth {gate_mouth:?}");
        assert!(bay_lee[0] < 0.1, "the bay behind the wall {bay_lee:?}");
        assert!(bay_in_line[0] > bay_lee[0], "{bay_in_line:?} {bay_lee:?}");
        // the wind: the lee just behind the 200 m wall is calmer than the gate
        let lee = e.at(1200.0, 3000.0);
        assert!(lee[1] < gate_mouth[1], "lee {lee:?} gate {gate_mouth:?}");
        // fetch: just behind the wall the fetch is short
        assert!(lee[2] < 0.5 && sea[2] > 0.99, "fetch {lee:?} {sea:?}");
        // round trip
        let back = Exposure::from_bytes(&e.to_bytes()).unwrap();
        assert_eq!(back.texels, e.texels);
    }
}
