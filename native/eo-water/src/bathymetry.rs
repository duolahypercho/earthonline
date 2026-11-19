//! The measured sea bed under the Golden Gate and its beaches (water
//! iteration 3, surf and bathymetry), and the swell it shapes: the NOAA NCEI
//! CUDEM 1/9 arc-second topobathy (survey group B1, read through
//! `eo_farshore::survey::bathy` under `EO_SURVEY_ROOT`) resampled onto a
//! model grid, the swell's crests and energy carried over it, and the result
//! handed to the water as its extra field (`extra.rs`).
//!
//! Method, each step cited:
//!
//! - **Depth.** Still-water depth below mean sea level: NAVD88 height from B1,
//!   mean sea level 0.969 m over NAVD88 at NOAA 9414290 (`beaches.rs`, the
//!   same datum eo-data takes). The sea bed is not lifted by the height scale
//!   (eo-data `groundscale::lift` leaves everything at or below sea level), so
//!   a depth is the surveyed depth in game metres too.
//! - **Wave speed.** Linear dispersion at the swell's peak period
//!   (`waves::SWELL_TP`), the wavelength from Fenton and McKee's explicit
//!   approximation (1990, "On calculating the lengths of water waves",
//!   Coastal Engineering 14, 499): L = L0 tanh((2 pi h / L0)^(3/4))^(2/3);
//!   group speed c_g = n c with n = (1 + 2kh / sinh 2kh) / 2.
//! - **Refraction: the crests.** The swell's crests are the level sets of its
//!   arrival time T, which obeys the eikonal |grad T| = 1 / c(h) (wave ray
//!   theory, Munk and Arthur 1952, "Wave intensity along a refracted ray",
//!   NBS Circular 521; Snell's law along every ray). Solved by the fast
//!   marching method (Sethian 1996, "A fast marching level set method for
//!   monotonically advancing fronts", PNAS 93, 1591), first order, from a
//!   plane swell entering the model's open-sea edges along `waves::SWELL_DIR`.
//! - **Refraction and sheltering: the height.** The energy flux E c_g carried
//!   along the rays (the direction of grad T) without loss between them: the
//!   steady wave energy balance with no sources (as a spectral wave model's
//!   geographic propagation, e.g. Booij, Ris and Holthuijsen 1999, JGR
//!   104(C4), 7649), solved cell by cell in arrival order with first-order
//!   upwind fluxes. Its lateral numerical spreading stands in for diffraction
//!   behind the headlands (INFERRED as adequate at the model's cell). The
//!   ratio to the open sea's flux is the refraction coefficient K_r squared;
//!   shoaling (Green's law in shallow water, the shoaling coefficient from c_g
//!   in general) is left to the shader, which has the local depth.
//! - **Depth-limited breaking on the way.** Where the significant height the
//!   clear-day swell (`waves::SWELL_HS[0]`) reaches exceeds 0.78 of the depth
//!   (McCowan 1894's limit, the breaker index the surf model uses) on a shoal
//!   at least `SHOAL_BREAK_M` deep, the flux is held at that limit, so a shoal
//!   that breaks the swell shelters what is behind it. In the beaches' own
//!   surf zone (shallower) the flux is carried unbroken: K_r there is the
//!   offshore amplitude the surf model (water_surf.wgsl) breaks itself on the
//!   measured depth.
//! - **Diffraction, roughly.** The directions' energy blurred over
//!   `DIFFRACT_SIGMA_M` (INFERRED: a few cells, the scale of the streaks a
//!   first-order ray solution leaves; a swell spreads behind an obstacle over
//!   the square root of its wavelength times the distance, hundreds of metres).
//!
//! The field (`extra::ExtraField`, RGBA16F over `FIELD_DOMAIN` at `CELL_M`):
//! R depth below mean sea level (m, negative on land), G the refraction
//! coefficient K_r (1 on the open sea), B the swell's arrival time in periods
//! of `PERIOD_S` less the field's least (so f16 keeps 1/64 of a period), A the
//! distance to the nearest surveyed land standing above the sea (m, capped at
//! `LAND_DIST_CAP_M`): the fender of the bridge's south pier and the rocks the
//! survey resolves, for the white water against them.
//!
//! Cooked once into `EO_NATIVE_CACHE/water/bathy-<key>.eoxf`, keyed by the
//! survey files' manifest digests and this module's constants; without the
//! survey folder (or with `--no-bathymetry`) there is no field and the water
//! draws as before.

use crate::extra::ExtraField;
use crate::waves;
use anyhow::{Context, Result, anyhow, bail};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Bumped when the cook changes (part of the cache key).
pub const BATHY_VERSION: &str = "water-bathy-v5";
/// The wave model's grid (game x min, z min, x max, z max): from the open
/// Pacific west of Point Bonita and Lands End (lon -122.561) to Aquatic Park,
/// from the Marin shore (lat 37.845) to the Richmond district.
pub const MODEL_DOMAIN: [f64; 4] = [-15_600.0, -9_200.0, -2_800.0, -1_000.0];
/// The field handed to the water (a crop of the model): Lands End and Baker
/// Beach to Crissy Field and the Marin shore of the Gate.
pub const FIELD_DOMAIN: [f64; 4] = [-9_000.0, -8_200.0, -2_800.0, -1_400.0];
/// The model's cell (game metres; 4.8 m on the ground at the 1.25 plan, about
/// 1.4 of B1's 3.4 m cells).
pub const CELL_M: f64 = 6.0;
/// Mean sea level over NAVD88 at NOAA 9414290 (m; `beaches.rs`).
pub const MSL_OVER_NAVD88_M: f64 = 0.969;
/// The swell's period the crests are cooked at (s): the clear-day swell.
pub const PERIOD_S: f64 = waves::SWELL_TP[0];
/// The clear-day significant height the breaking on the way is cooked at (m).
pub const HS_COOK_M: f64 = waves::SWELL_HS[0];
/// McCowan's breaker index (height over depth).
pub const BREAKER_INDEX: f64 = 0.78;
/// Shoals shallower than this (m) are the beaches' surf zone, broken by the surf model, not here.
pub const SHOAL_BREAK_M: f64 = 4.0;
/// The energy's final blur (game m).
pub const DIFFRACT_SIGMA_M: f64 = 24.0;
/// Water shallower than this is the shore for the swell (m).
pub const DRY_M: f64 = 0.1;
/// The open-sea edges seed the swell west of these game x (the north edge
/// west of Point Bonita; the west edge everywhere it is sea).
pub const NORTH_SEED_MAX_X: f64 = -12_500.0;
/// The land distance's cap (m).
pub const LAND_DIST_CAP_M: f64 = 60.0;
/// The refraction coefficient's cap (a caustic's focus).
pub const KR_MAX: f64 = 3.0;
/// The B1 files the field is cooked from (their manifest digests key the cache).
pub const SOURCES: [&str; 2] = eo_farshore::survey::bathy::TILES;

const G: f64 = 9.81;

/// Wavelength (m) at depth `h` (m) for period `t` (s): Fenton and McKee 1990.
pub fn wavelength(h: f64, t: f64) -> f64 {
    let l0 = G * t * t / std::f64::consts::TAU;
    let x = (std::f64::consts::TAU * h.max(1e-3) / l0).powf(0.75);
    l0 * x.tanh().powf(2.0 / 3.0)
}

/// Phase speed and group speed (m/s) at depth `h` for period `t`.
pub fn speeds(h: f64, t: f64) -> (f64, f64) {
    let l = wavelength(h, t);
    let c = l / t;
    let kh2 = 2.0 * std::f64::consts::TAU / l * h.max(1e-3);
    let n = if kh2 > 40.0 { 0.5 } else { 0.5 * (1.0 + kh2 / kh2.sinh()) };
    (c, n * c)
}

/// The model's cooked grids (row-major, rows along +z).
pub struct Model {
    pub origin: [f64; 2],
    pub cell_m: f64,
    pub nx: usize,
    pub nz: usize,
    /// depth below mean sea level (m), negative on land, NaN without survey
    pub depth: Vec<f32>,
    /// arrival time (s), infinite where the swell never arrives
    pub time: Vec<f64>,
    /// the refraction coefficient K_r (0 where no swell)
    pub kr: Vec<f32>,
    /// distance to land (m), capped
    pub land_dist: Vec<f32>,
}

impl Model {
    fn idx(&self, i: usize, j: usize) -> usize {
        j * self.nx + i
    }

    /// Nearest-cell values at a game point (depth, time, K_r), None outside.
    pub fn at(&self, x: f64, z: f64) -> Option<(f32, f64, f32)> {
        let i = ((x - self.origin[0]) / self.cell_m).floor();
        let j = ((z - self.origin[1]) / self.cell_m).floor();
        if i < 0.0 || j < 0.0 || i >= self.nx as f64 || j >= self.nz as f64 {
            return None;
        }
        let k = self.idx(i as usize, j as usize);
        Some((self.depth[k], self.time[k], self.kr[k]))
    }
}

/// Depth below mean sea level at each cell centre of `domain` from `height`
/// (NAVD88 m at lon, lat; None where the survey has none), threaded by rows.
pub fn sample_depth(domain: [f64; 4], cell: f64, height: &(dyn Fn(f64, f64) -> Option<f32> + Sync)) -> (usize, usize, Vec<f32>) {
    let nx = ((domain[2] - domain[0]) / cell).round() as usize;
    let nz = ((domain[3] - domain[1]) / cell).round() as usize;
    let gs = crate::ground_scale();
    let frame = eo_data::Frame::sf();
    let mut depth = vec![f32::NAN; nx * nz];
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 8);
    let rows_per = nz.div_ceil(threads);
    std::thread::scope(|s| {
        for (t, chunk) in depth.chunks_mut(rows_per * nx).enumerate() {
            let frame = &frame;
            s.spawn(move || {
                for (r, row) in chunk.chunks_mut(nx).enumerate() {
                    let j = t * rows_per + r;
                    let z = domain[1] + (j as f64 + 0.5) * cell;
                    for (i, v) in row.iter_mut().enumerate() {
                        let x = domain[0] + (i as f64 + 0.5) * cell;
                        let (lat, lon) = frame.unproject(x / gs, z / gs);
                        if let Some(hn) = height(lon, lat) {
                            *v = (MSL_OVER_NAVD88_M - hn as f64) as f32;
                        }
                    }
                }
            });
        }
    });
    (nx, nz, depth)
}

/// An f64's order as a u64 (for non-negative finite values).
fn key(v: f64) -> u64 {
    v.to_bits()
}

/// Arrival time by the fast marching method over the water cells, from seeds
/// (cell index, time); speed `c[k]` (m/s), 0 for land.
pub fn fast_march(nx: usize, nz: usize, cell: f64, c: &[f64], seeds: &[(usize, f64)]) -> (Vec<f64>, Vec<u32>) {
    let n = nx * nz;
    let mut t = vec![f64::INFINITY; n];
    let mut known = vec![false; n];
    let mut order = Vec::with_capacity(n);
    let mut heap = BinaryHeap::new();
    for &(k, t0) in seeds {
        if c[k] > 0.0 && t0 < t[k] {
            t[k] = t0;
            heap.push(Reverse((key(t0), k as u32)));
        }
    }
    let solve = |t: &[f64], known: &[bool], i: usize, j: usize, ck: f64| -> f64 {
        let k = j * nx + i;
        let pick = |a: Option<usize>, b: Option<usize>| -> f64 {
            let va = a.filter(|&q| known[q]).map_or(f64::INFINITY, |q| t[q]);
            let vb = b.filter(|&q| known[q]).map_or(f64::INFINITY, |q| t[q]);
            va.min(vb)
        };
        let tx = pick((i > 0).then(|| k - 1), (i + 1 < nx).then(|| k + 1));
        let tz = pick((j > 0).then(|| k - nx), (j + 1 < nz).then(|| k + nx));
        let h = cell / ck;
        let (a, b) = (tx.min(tz), tx.max(tz));
        if !b.is_finite() || b - a >= h {
            return a + h;
        }
        // ((T - a)^2 + (T - b)^2) = h^2
        let s = a + b;
        let disc = s * s - 2.0 * (a * a + b * b - h * h);
        0.5 * (s + disc.max(0.0).sqrt())
    };
    while let Some(Reverse((_, k32))) = heap.pop() {
        let k = k32 as usize;
        if known[k] {
            continue;
        }
        known[k] = true;
        order.push(k32);
        let (i, j) = (k % nx, k / nx);
        let nb = [(i > 0).then(|| (i - 1, j)), (i + 1 < nx).then(|| (i + 1, j)), (j > 0).then(|| (i, j - 1)), (j + 1 < nz).then(|| (i, j + 1))];
        for (ni, nj) in nb.into_iter().flatten() {
            let q = nj * nx + ni;
            if known[q] || c[q] <= 0.0 {
                continue;
            }
            let tn = solve(&t, &known, ni, nj, c[q]);
            if tn < t[q] {
                t[q] = tn;
                heap.push(Reverse((key(tn), q as u32)));
            }
        }
    }
    (t, order)
}

/// The energy flux E c_g carried along the rays (grad T) in arrival order,
/// each cell's outflow equal to its upwind inflow; seeds hold `f_seed`; the
/// flux at a cell is capped at `cap[k]` (depth-limited breaking).
pub fn transport(nx: usize, nz: usize, t: &[f64], order: &[u32], seed: &[bool], f_seed: &[f64], cap: &[f64]) -> Vec<f64> {
    let n = nx * nz;
    // the ray direction: grad T by differences over the finite neighbours
    let mut dir = vec![[0f64; 2]; n];
    for k in 0..n {
        if !t[k].is_finite() {
            continue;
        }
        let (i, j) = (k % nx, k / nx);
        let d = |a: Option<usize>, b: Option<usize>| -> f64 {
            let ta = a.map(|q| t[q]).filter(|v| v.is_finite());
            let tb = b.map(|q| t[q]).filter(|v| v.is_finite());
            match (ta, tb) {
                (Some(x), Some(y)) => 0.5 * (y - x),
                (Some(x), None) => t[k] - x,
                (None, Some(y)) => y - t[k],
                _ => 0.0,
            }
        };
        let gx = d((i > 0).then(|| k - 1), (i + 1 < nx).then(|| k + 1));
        let gz = d((j > 0).then(|| k - nx), (j + 1 < nz).then(|| k + nx));
        let l = (gx * gx + gz * gz).sqrt();
        if l > 0.0 {
            dir[k] = [gx / l, gz / l];
        }
    }
    let mut f = vec![0f64; n];
    let mut done = vec![false; n];
    for &k32 in order {
        let k = k32 as usize;
        let (i, j) = (k % nx, k / nx);
        let u = dir[k];
        // faces: (neighbour, unit normal from this cell to it)
        let faces = [((i > 0).then(|| k - 1), [-1.0, 0.0]), ((i + 1 < nx).then(|| k + 1), [1.0, 0.0]), ((j > 0).then(|| k - nx), [0.0, -1.0]), ((j + 1 < nz).then(|| k + nx), [0.0, 1.0])];
        let mut out = 0.0;
        let mut inflow = 0.0;
        for (q, nrm) in faces {
            let o = u[0] * nrm[0] + u[1] * nrm[1];
            if o > 0.0 {
                out += o;
            }
            if let Some(q) = q
                && done[q]
            {
                let uq = dir[q];
                // the neighbour's flow toward this cell (its normal is the opposite)
                let w = -(uq[0] * nrm[0] + uq[1] * nrm[1]);
                if w > 0.0 {
                    inflow += f[q] * w;
                }
            }
        }
        f[k] = if seed[k] { f_seed[k] } else if out > 1e-9 { inflow / out } else { 0.0 };
        f[k] = f[k].min(cap[k]);
        done[k] = true;
    }
    f
}

/// Distance (m) to the nearest cell where `land[k]`, a two-pass chamfer
/// (3-4 weights; Borgefors 1986), capped.
pub fn land_distance(nx: usize, nz: usize, cell: f64, land: &[bool], cap: f64) -> Vec<f32> {
    let big = f64::MAX / 4.0;
    let mut d: Vec<f64> = land.iter().map(|&l| if l { 0.0 } else { big }).collect();
    let (a, b) = (cell, cell * 4.0 / 3.0);
    for j in 0..nz {
        for i in 0..nx {
            let k = j * nx + i;
            let mut v = d[k];
            if i > 0 {
                v = v.min(d[k - 1] + a);
            }
            if j > 0 {
                v = v.min(d[k - nx] + a);
                if i > 0 {
                    v = v.min(d[k - nx - 1] + b);
                }
                if i + 1 < nx {
                    v = v.min(d[k - nx + 1] + b);
                }
            }
            d[k] = v;
        }
    }
    for j in (0..nz).rev() {
        for i in (0..nx).rev() {
            let k = j * nx + i;
            let mut v = d[k];
            if i + 1 < nx {
                v = v.min(d[k + 1] + a);
            }
            if j + 1 < nz {
                v = v.min(d[k + nx] + a);
                if i + 1 < nx {
                    v = v.min(d[k + nx + 1] + b);
                }
                if i > 0 {
                    v = v.min(d[k + nx - 1] + b);
                }
            }
            d[k] = v;
        }
    }
    d.into_iter().map(|v| v.min(cap) as f32).collect()
}

/// A separable Gaussian blur (three box passes, Wells 1986) of a grid, `sigma` in cells.
pub fn blur(nx: usize, nz: usize, v: &[f64], sigma: f64) -> Vec<f64> {
    // three boxes of width w have variance 3 (w^2 - 1) / 12
    let w = ((4.0 * sigma * sigma + 1.0).sqrt().round() as usize).max(1) | 1;
    let r = w / 2;
    let mut a = v.to_vec();
    let mut b = vec![0f64; v.len()];
    for _ in 0..3 {
        for j in 0..nz {
            let row = &a[j * nx..(j + 1) * nx];
            let mut acc: f64 = (0..=r.min(nx - 1)).map(|i| row[i]).sum::<f64>() + row[0] * r as f64;
            for i in 0..nx {
                b[j * nx + i] = acc / w as f64;
                let add = row[(i + r + 1).min(nx - 1)];
                let sub = row[i.saturating_sub(r)];
                acc += add - sub;
            }
        }
        for i in 0..nx {
            let col = |j: usize| b[j * nx + i];
            let mut acc: f64 = (0..=r.min(nz - 1)).map(col).sum::<f64>() + col(0) * r as f64;
            for j in 0..nz {
                a[j * nx + i] = acc / w as f64;
                acc += col((j + r + 1).min(nz - 1)) - col(j.saturating_sub(r));
            }
        }
    }
    a
}

/// The bed the rays see is the survey smoothed over `RAY_BED_SIGMA_M`:
/// geometric optics holds only for depth changing slowly over a wavelength
/// (the swell's is 170 m on the Bar and 45-80 m in the last metres), and the
/// bed west of the bridge carries a field of sand waves 30-220 m long and up to
/// 10 m high (Barnard, Hanes, Rubin and Kvitek 2006, "Giant sand waves at the
/// mouth of San Francisco Bay", Eos 87(29), 285), which scatter a swell rather
/// than refract it and otherwise focus the rays into bands along its crests;
/// INFERRED as a quarter of the larger sand waves' length. The near-shore
/// depth stays at least half the survey's (`model`).
pub const RAY_BED_SIGMA_M: f64 = 50.0;
/// The swell's directions averaged (in units of its spread's standard
/// deviation, Gaussian weights): a real sea's spread of directions blurs the
/// caustics a single ray family draws. The spread of a cos-2s sea is about
/// sqrt(2 / (s + 1)) radians (Longuet-Higgins, Cartwright and Smith 1963).
pub const DIR_NODES: [f64; 5] = [-1.6, -0.8, 0.0, 0.8, 1.6];

/// Run the model over a depth grid (the B1 sample or a test's).
pub fn model(domain: [f64; 4], cell: f64, nx: usize, nz: usize, depth: Vec<f32>) -> Model {
    let n = nx * nz;
    let wet = |k: usize| depth[k].is_finite() && depth[k] as f64 > DRY_M;
    let bed: Vec<f64> = (0..n).map(|k| if depth[k].is_finite() { (depth[k] as f64).max(0.0) } else { 0.0 }).collect();
    let bed = blur(nx, nz, &bed, RAY_BED_SIGMA_M / cell);
    let h_ray = |k: usize| bed[k].max(DRY_M).max(0.5 * depth[k] as f64);
    let c: Vec<f64> = (0..n).map(|k| if wet(k) { speeds(h_ray(k), PERIOD_S).0 } else { 0.0 }).collect();
    let cg: Vec<f64> = (0..n).map(|k| if wet(k) { speeds(h_ray(k), PERIOD_S).1 } else { 0.0 }).collect();
    let c0 = G * PERIOD_S / std::f64::consts::TAU; // deep water
    // the flux at which the clear-day swell breaks: Hs = 0.78 h, E ~ Hs^2, flux E c_g (the open sea's E is 1)
    let cap: Vec<f64> = (0..n)
        .map(|k| {
            if !wet(k) {
                return 0.0;
            }
            if (depth[k] as f64) < SHOAL_BREAK_M {
                return f64::INFINITY;
            }
            let h_rel = BREAKER_INDEX * depth[k] as f64 / HS_COOK_M;
            h_rel * h_rel * cg[k]
        })
        .collect();
    let spread = (2.0 / (waves::SWELL_SPREAD[0] + 1.0)).sqrt();
    let mut e_sum = vec![0f64; n];
    let mut w_sum = 0.0;
    let mut time = vec![];
    for node in DIR_NODES {
        let dir = waves::SWELL_DIR + node * spread;
        let (dx, dz) = (dir.cos(), dir.sin());
        let mut seeds = vec![];
        let mut seed = vec![false; n];
        let mut add = |i: usize, j: usize, seeds: &mut Vec<(usize, f64)>| {
            let k = j * nx + i;
            if wet(k) && !seed[k] {
                let (x, z) = (domain[0] + (i as f64 + 0.5) * cell, domain[1] + (j as f64 + 0.5) * cell);
                seeds.push((k, ((x - domain[0]) * dx + (z - domain[1]) * dz + 1.0e5) / c0));
                seed[k] = true;
            }
        };
        for j in 0..nz {
            add(0, j, &mut seeds);
        }
        for i in 0..nx {
            if domain[0] + (i as f64 + 0.5) * cell < NORTH_SEED_MAX_X {
                add(i, 0, &mut seeds);
            }
        }
        let (t, order) = fast_march(nx, nz, cell, &c, &seeds);
        let f = transport(nx, nz, &t, &order, &seed, &cg, &cap);
        let w = (-0.5 * node * node).exp();
        for k in 0..n {
            if cg[k] > 0.0 {
                e_sum[k] += w * f[k] / cg[k];
            }
        }
        w_sum += w;
        if node == 0.0 {
            time = t;
        }
    }
    // E relative to the open sea's (1), over the directions; then K_r = sqrt(E c_g / c_g0)
    // (the flux ratio), c_g0 the seeds' mean
    let cg0 = {
        let v: Vec<f64> = (0..nz).filter(|&j| wet(j * nx)).map(|j| cg[j * nx]).collect();
        if v.is_empty() { 1.0 } else { v.iter().sum::<f64>() / v.len() as f64 }
    };
    // the flux ratio blurred over the water (land holds no energy: a normalised blur, so the shore keeps its value)
    let flux: Vec<f64> = (0..n).map(|k| e_sum[k] / w_sum * cg[k] / cg0).collect();
    let wmask: Vec<f64> = (0..n).map(|k| if wet(k) { 1.0 } else { 0.0 }).collect();
    let fb = blur(nx, nz, &flux, DIFFRACT_SIGMA_M / cell);
    let wb = blur(nx, nz, &wmask, DIFFRACT_SIGMA_M / cell);
    let kr = (0..n).map(|k| if wet(k) { ((fb[k] / wb[k].max(1e-6)).max(0.0).sqrt().min(KR_MAX)) as f32 } else { 0.0 }).collect();
    let land: Vec<bool> = depth.iter().map(|&d| d.is_finite() && d < 0.0).collect();
    let land_dist = land_distance(nx, nz, cell, &land, LAND_DIST_CAP_M);
    Model { origin: [domain[0], domain[1]], cell_m: cell, nx, nz, depth, time, kr, land_dist }
}

/// Cells the arrival time and K_r are carried past the water's edge.
pub const FILL_PASSES: usize = 24;

/// Grow two grids' known (finite) values into their unknown cells, each pass
/// taking the mean of the known 4-neighbours (the same cells for both).
pub fn fill_from_known(w: usize, h: usize, a: &mut [f64], b: &mut [f64], passes: usize) {
    for _ in 0..passes {
        let (pa, pb) = (a.to_vec(), b.to_vec());
        let mut changed = false;
        for j in 0..h {
            for i in 0..w {
                let k = j * w + i;
                if pa[k].is_finite() {
                    continue;
                }
                let (mut sa, mut sb, mut c) = (0.0, 0.0, 0.0);
                for (ok, q) in [(i > 0, k.wrapping_sub(1)), (i + 1 < w, k + 1), (j > 0, k.wrapping_sub(w)), (j + 1 < h, k + w)] {
                    if ok && pa[q].is_finite() {
                        sa += pa[q];
                        sb += pb[q];
                        c += 1.0;
                    }
                }
                if c > 0.0 {
                    a[k] = sa / c;
                    b[k] = sb / c;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
}

/// The field over `FIELD_DOMAIN` from the model (nearest cells; the domains share the cell grid).
pub fn field_of(m: &Model, domain: [f64; 4]) -> Result<ExtraField> {
    let i0 = ((domain[0] - m.origin[0]) / m.cell_m).round() as isize;
    let j0 = ((domain[1] - m.origin[1]) / m.cell_m).round() as isize;
    let w = ((domain[2] - domain[0]) / m.cell_m).round() as usize;
    let h = ((domain[3] - domain[1]) / m.cell_m).round() as usize;
    if i0 < 0 || j0 < 0 || i0 as usize + w > m.nx || j0 as usize + h > m.nz {
        bail!("bathymetry: the field's domain is not inside the model's");
    }
    let (i0, j0) = (i0 as usize, j0 as usize);
    let mut t_min = f64::INFINITY;
    for j in 0..h {
        for i in 0..w {
            let t = m.time[m.idx(i0 + i, j0 + j)];
            if t.is_finite() {
                t_min = t_min.min(t);
            }
        }
    }
    let base = if t_min.is_finite() { (t_min / PERIOD_S).floor() } else { 0.0 };
    // the arrival time and K_r carried onto the land and the unreached water from
    // the nearest reached water (`FILL_PASSES` cells), so the shader's bilinear
    // read at the waterline blends real values, not a step to zero
    let n = w * h;
    let mut tv = vec![f64::NAN; n];
    let mut kv = vec![f64::NAN; n];
    for j in 0..h {
        for i in 0..w {
            let k = m.idx(i0 + i, j0 + j);
            if m.time[k].is_finite() {
                tv[j * w + i] = m.time[k] / PERIOD_S - base;
                kv[j * w + i] = m.kr[k] as f64;
            }
        }
    }
    fill_from_known(w, h, &mut tv, &mut kv, FILL_PASSES);
    let mut texels = Vec::with_capacity(n);
    let mut t_max: f64 = 0.0;
    for j in 0..h {
        for i in 0..w {
            let k = m.idx(i0 + i, j0 + j);
            let d = if m.depth[k].is_finite() { (m.depth[k] as f64).clamp(-30.0, 400.0) } else { -30.0 };
            let q = j * w + i;
            let tp = if tv[q].is_finite() { tv[q] } else { 0.0 };
            let kr = if kv[q].is_finite() { kv[q] } else { 0.0 };
            t_max = t_max.max(tp);
            texels.push([crate::field::to_half_three(d), crate::field::to_half_three(kr), crate::field::to_half_three(tp), crate::field::to_half_three(m.land_dist[k] as f64)]);
        }
    }
    if t_max > 1000.0 {
        bail!("bathymetry: the arrival time spans {t_max:.0} periods, past what f16 carries");
    }
    Ok(ExtraField { origin: [domain[0], domain[1]], cell_m: m.cell_m, width: w, height: h, texels })
}

/// The cache key: this module's constants, the survey files' digests and the scales.
pub fn cache_key() -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut k = format!("{BATHY_VERSION}|{MODEL_DOMAIN:?}|{FIELD_DOMAIN:?}|{CELL_M}|{PERIOD_S}|{HS_COOK_M}|{SHOAL_BREAK_M}|{DIFFRACT_SIGMA_M}|{RAY_BED_SIGMA_M}|{DIR_NODES:?}|{}|{}|{}", waves::SWELL_DIR, waves::SWELL_SPREAD[0], crate::ground_scale());
    for rel in SOURCES {
        let e = eo_farshore::survey::entry(rel).ok_or_else(|| anyhow!("{rel}: not in the survey manifest"))?;
        k.push_str(&format!("|{}", e.sha256));
    }
    Ok(Sha256::digest(k.as_bytes()).iter().take(12).map(|x| format!("{x:02x}")).collect())
}

const MAGIC: &[u8; 8] = b"EOXF0001";

/// The field's bytes (the cache file).
pub fn to_bytes(f: &ExtraField) -> Vec<u8> {
    let mut b = Vec::with_capacity(48 + f.texels.len() * 8);
    b.extend_from_slice(MAGIC);
    for v in [f.origin[0], f.origin[1], f.cell_m] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&(f.width as u32).to_le_bytes());
    b.extend_from_slice(&(f.height as u32).to_le_bytes());
    for t in &f.texels {
        for c in t {
            b.extend_from_slice(&c.to_le_bytes());
        }
    }
    b
}

/// The field from its bytes.
pub fn from_bytes(b: &[u8]) -> Result<ExtraField> {
    if b.len() < 40 || &b[..8] != MAGIC {
        bail!("not a cooked extra field");
    }
    let f = |o: usize| f64::from_le_bytes(b[o..o + 8].try_into().unwrap());
    let u = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as usize;
    let (width, height) = (u(32), u(36));
    if b.len() != 40 + width * height * 8 {
        bail!("cooked extra field: {} bytes for {width} x {height}", b.len());
    }
    let texels = b[40..].chunks_exact(8).map(|c| [0, 1, 2, 3].map(|i| u16::from_le_bytes([c[2 * i], c[2 * i + 1]]))).collect();
    Ok(ExtraField { origin: [f(8), f(16)], cell_m: f(24), width, height, texels })
}

/// A half float's value (for tests and the log).
pub fn half_to_f32(h: u16) -> f32 {
    let s = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let e = ((h >> 10) & 0x1f) as i32;
    let m = (h & 0x3ff) as f32;
    s * match e {
        0 => m * 2f32.powi(-24),
        31 => f32::INFINITY,
        _ => (1.0 + m / 1024.0) * 2f32.powi(e - 15),
    }
}

/// What the cook did (for the start-up line).
pub struct Cooked {
    pub field: ExtraField,
    pub from_cache: bool,
    pub ms: f64,
}

/// The model over `MODEL_DOMAIN` from the B1 tiles under `root`.
pub fn cook_model(root: &eo_farshore::survey::Root) -> Result<Model> {
    let bathy = eo_farshore::survey::bathy::Bathy::open(root).context("B1 CUDEM tiles")?;
    let (nx, nz, depth) = sample_depth(MODEL_DOMAIN, CELL_M, &|lon, lat| bathy.height(lon, lat));
    Ok(model(MODEL_DOMAIN, CELL_M, nx, nz, depth))
}

/// Cook (or read from `cache`) the field from the survey folder `EO_SURVEY_ROOT` names.
pub fn load(cache: Option<&std::path::Path>) -> Result<Cooked> {
    let t0 = std::time::Instant::now();
    let root = eo_farshore::survey::Root::discover().ok_or_else(|| anyhow!("{} not set or not a folder", eo_farshore::survey::ROOT_ENV))?;
    let key = cache_key()?;
    let path = cache.map(|c| c.join("water").join(format!("bathy-{key}.eoxf")));
    if let Some(p) = &path
        && let Ok(b) = std::fs::read(p)
    {
        match from_bytes(&b) {
            Ok(field) => return Ok(Cooked { field, from_cache: true, ms: t0.elapsed().as_secs_f64() * 1e3 }),
            Err(e) => eprintln!("water: cached bathymetry refused ({e:#}); cooking again"),
        }
    }
    let m = cook_model(&root)?;
    let field = field_of(&m, FIELD_DOMAIN)?;
    if let Some(p) = &path {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = p.with_extension("tmp");
        if std::fs::write(&tmp, to_bytes(&field)).is_ok() {
            let _ = std::fs::rename(&tmp, p);
        }
    }
    Ok(Cooked { field, from_cache: false, ms: t0.elapsed().as_secs_f64() * 1e3 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispersion_matches_its_limits() {
        // deep water: L0 = g T^2 / 2 pi (314.8 m at 14.2 s), c_g = c / 2
        let (c, cg) = speeds(2000.0, 14.2);
        assert!((wavelength(2000.0, 14.2) - 314.8).abs() < 1.0);
        assert!((cg / c - 0.5).abs() < 1e-3);
        // shallow water: c = c_g = sqrt(g h) within 2 % at h = 1 m
        let (c, cg) = speeds(1.0, 14.2);
        assert!((c / (G * 1.0f64).sqrt() - 1.0).abs() < 0.02 && (cg / c - 1.0).abs() < 0.02, "{c} {cg}");
        // Fenton and McKee's stated accuracy (1.5 %) against the exact root at kh ~ 1
        let h = 30.0;
        let l = wavelength(h, 10.0);
        let k = std::f64::consts::TAU / l;
        let w = std::f64::consts::TAU / 10.0;
        assert!((w * w / (G * k * (k * h).tanh()) - 1.0).abs() < 0.03);
    }

    #[test]
    fn a_plane_wave_on_a_flat_bed_keeps_its_speed_and_height() {
        let (nx, nz, cell) = (80, 60, 5.0);
        let depth = vec![20.0f32; nx * nz];
        let m = model([0.0, 0.0, nx as f64 * cell, nz as f64 * cell], cell, nx, nz, depth);
        let c = speeds(20.0, PERIOD_S).0;
        // the crests travel along SWELL_DIR at c: T along x grows as cos(dir) / c
        let (_, ta, _) = m.at(52.5, 152.5).unwrap();
        let (_, tb, _) = m.at(252.5, 152.5).unwrap();
        let expect = 200.0 * waves::SWELL_DIR.cos() / c;
        assert!(((tb - ta) / expect - 1.0).abs() < 0.08, "{} {}", tb - ta, expect);
        // and the flux stays the open sea's away from the edges
        let (_, _, k) = m.at(252.5, 202.5).unwrap();
        assert!((k - 1.0).abs() < 0.12, "{k}");
    }

    #[test]
    fn a_shoal_bends_the_crests_and_an_island_shelters_its_lee() {
        // a bed shoaling toward +x (from 30 m to 2 m): the crests turn toward the
        // contours (Snell), so T grows faster along x near the shore than along z
        let (nx, nz, cell) = (200, 120, 5.0);
        let mut depth = vec![0f32; nx * nz];
        for j in 0..nz {
            for i in 0..nx {
                depth[j * nx + i] = (30.0 - 28.0 * i as f32 / nx as f32).max(2.0);
            }
        }
        // an island in the middle
        for j in 45..75 {
            for i in 60..75 {
                depth[j * nx + i] = -3.0;
            }
        }
        let m = model([0.0, 0.0, nx as f64 * cell, nz as f64 * cell], cell, nx, nz, depth);
        let d = |x: f64, z: f64| m.at(x, z).unwrap();
        // the island's lee (down the swell from it) is sheltered against the open water beside it
        let lee = d(85.0 * cell, 64.0 * cell).2;
        let open = d(90.0 * cell, 15.0 * cell).2;
        assert!(lee < 0.75 * open, "lee {lee} open {open}");
        // land is land
        assert!(d(65.0 * cell, 60.0 * cell).0 < 0.0);
        assert_eq!(m.land_dist[60 * nx + 65], 0.0);
        assert!((m.land_dist[60 * nx + 80] - 6.0 * cell as f32).abs() < 0.5 * cell as f32); // (the island's last column is 74)
        // refraction: near the shallow edge the ray turns toward +x (normal to the contours)
        let (t1, t2) = (d(190.0 * cell, 20.0 * cell).1, d(190.0 * cell, 30.0 * cell).1);
        let (t3, t4) = (d(30.0 * cell, 20.0 * cell).1, d(30.0 * cell, 30.0 * cell).1);
        let along_z_shallow = (t2 - t1) / (10.0 * cell);
        let along_z_deep = (t4 - t3) / (10.0 * cell);
        // |dT/dz| = sin(angle) / c is constant along a ray (Snell); in shallow water c is small but the ray is near normal
        assert!(along_z_shallow > 0.0 && along_z_deep > 0.0);
        assert!((along_z_shallow / along_z_deep - 1.0).abs() < 0.25, "Snell's invariant {along_z_shallow} {along_z_deep}");
        // the field crop round-trips through its bytes
        let f = field_of(&m, [100.0, 100.0, 600.0, 400.0]).unwrap();
        assert_eq!(from_bytes(&to_bytes(&f)).unwrap(), f);
        assert!((half_to_f32(f.texels[0][0]) - m.at(102.5, 102.5).unwrap().0).abs() < 0.05);
    }

    /// Straight parallel contours (a plane beach shoaling toward +x, the
    /// swell oblique to it): Snell's law and energy flux between rays give
    /// K_r = sqrt(cos a0 / cos a) analytically (Dean and Dalrymple 1991,
    /// "Water Wave Mechanics for Engineers and Scientists", ch. 4); the model
    /// must reproduce it away from the domain's edges.
    #[test]
    fn a_plane_beach_matches_snell_and_the_ray_spacing() {
        // the beach's shoreward normal 40 degrees off the swell's travel
        let phi = waves::SWELL_DIR - 40f64.to_radians();
        let (nrm_x, nrm_z) = (phi.cos(), phi.sin());
        let (nx, nz, cell) = (520usize, 500usize, 6.0);
        let span = 2000.0;
        let h = |s: f64| (40.0 - 38.0 * (s / span).clamp(0.0, 1.0)).max(0.5);
        let mut depth = vec![0f32; nx * nz];
        for j in 0..nz {
            for i in 0..nx {
                let (x, z) = ((i as f64 + 0.5) * cell, (j as f64 + 0.5) * cell);
                depth[j * nx + i] = h(x * nrm_x + z * nrm_z) as f32;
            }
        }
        let m = model([0.0, 0.0, nx as f64 * cell, nz as f64 * cell], cell, nx, nz, depth);
        let spread = (2.0 / (waves::SWELL_SPREAD[0] + 1.0)).sqrt();
        let (_, cg0) = speeds(40.0, PERIOD_S);
        let c0 = speeds(40.0, PERIOD_S).0;
        // points along the normal through the domain's middle
        for sdist in [400.0, 1000.0, 1500.0, 1850.0] {
            let (x, z) = (1200.0 + (sdist - 1200.0 * nrm_x - 1500.0 * nrm_z) * nrm_x, 1500.0 + (sdist - 1200.0 * nrm_x - 1500.0 * nrm_z) * nrm_z);
            let hi = h(sdist);
            let (c, cg) = speeds(hi, PERIOD_S);
            let (mut e, mut ws) = (0.0, 0.0);
            for node in DIR_NODES {
                let a0 = 40f64.to_radians() + node * spread;
                let a = (a0.sin() * c / c0).asin();
                let w = (-0.5 * node * node).exp();
                // E c_g cos a constant between rays: E = E0 cg0 cos a0 / (cg cos a)
                e += w * cg0 * a0.cos() / (cg * a.cos());
                ws += w;
            }
            let kr_expect = (e / ws * cg / cg0).sqrt();
            let (_, _, k) = m.at(x, z).unwrap();
            println!("depth {hi:.1} m: K_r {k:.3}, Snell {kr_expect:.3}");
            assert!((k as f64 / kr_expect - 1.0).abs() < 0.1, "depth {hi}: {k} against {kr_expect}");
        }
    }

    /// The field cooked from B1 (skips without `EO_SURVEY_ROOT`): the strait is
    /// deep, Baker Beach's surf zone shallow with swell reaching it, Crissy
    /// Field's water sheltered against Baker's, and the fender of the south pier
    /// stands as land in the open strait.
    #[test]
    fn the_survey_field_at_named_places() {
        let Some(_root) = eo_farshore::survey::Root::discover() else {
            eprintln!("bathymetry: skipped, {} not set", eo_farshore::survey::ROOT_ENV);
            return;
        };
        let t = std::time::Instant::now();
        let c = load(None).expect("cook");
        let f = &c.field;
        println!("cooked {} x {} in {:.0} ms", f.width, f.height, c.ms);
        let gs = crate::ground_scale();
        let fr = eo_data::Frame::sf();
        let at = |lat: f64, lon: f64| -> [f32; 4] {
            let x = (lon - fr.center_lon) * fr.m_per_deg_lon * gs;
            let z = -(lat - fr.center_lat) * fr.m_per_deg_lat * gs;
            let i = ((x - f.origin[0]) / f.cell_m) as usize;
            let j = ((z - f.origin[1]) / f.cell_m) as usize;
            f.texels[j * f.width + i].map(half_to_f32)
        };
        for (name, lat, lon) in [
            ("mid-strait", 37.8160, -122.4780),
            ("off Baker Beach 300 m", 37.7960, -122.4870),
            ("Baker Beach surf 60 m", 37.7955, -122.4842),
            ("off Crissy Field 150 m", 37.8065, -122.4640),
            ("off Fort Point 100 m", 37.8115, -122.4760),
            ("south pier", 37.81060, -122.47720),
        ] {
            println!("{name}: {:?}", at(lat, lon));
        }
        assert!(at(37.8160, -122.4780)[0] > 50.0);
        // (diagnostics) EO_BATHY_DUMP=<dir>: the model's depth, time, K_r and land distance as raw f32
        if let Ok(dir) = std::env::var("EO_BATHY_DUMP") {
            let m = cook_model(&_root).unwrap();
            let dir = std::path::Path::new(&dir);
            std::fs::create_dir_all(dir).unwrap();
            let raw = |v: Vec<f32>| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
            std::fs::write(dir.join("depth.f32"), raw(m.depth.clone())).unwrap();
            std::fs::write(dir.join("time.f32"), raw(m.time.iter().map(|&t| if t.is_finite() { t as f32 } else { -1.0 }).collect())).unwrap();
            std::fs::write(dir.join("kr.f32"), raw(m.kr.clone())).unwrap();
            std::fs::write(dir.join("land.f32"), raw(m.land_dist.clone())).unwrap();
            std::fs::write(dir.join("dims.txt"), format!("{} {} {:?} {}", m.nx, m.nz, m.origin, m.cell_m)).unwrap();
        }
        println!("total {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
    }
}
