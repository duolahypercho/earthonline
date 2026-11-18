//! The citywide signed shore field: `bakeCitywideShoreField` (and its
//! `chamfer`, `beachDistanceGrid`) of src/render/materials/ocean-open-sea.js,
//! ported line for line, plus what the native renderer needs on top of it: a
//! max-reduction pyramid for culling grid patches that lie wholly over land,
//! and a disk cache.
//!
//! R = signed shore distance (metres, + over water, - over land, capped at
//! `FIELD_CAP_M`), G = 1 where the nearest shore is a mapped beach. Stored as
//! the JS stores it: half floats by three's (truncating) `toHalfFloat`.

use crate::shoreline::Oracle;
use anyhow::{Context, Result, bail};
use std::sync::atomic::{AtomicBool, Ordering};

/// `FIELD_CAP_M`.
pub const FIELD_CAP_M: f64 = 400.0;
/// `FAR_FIELD_CELL_M`: the citywide field's texel.
pub const FAR_FIELD_CELL_M: f64 = 16.0;
/// `FAR_SHORE_REACH_M`: the open sea draws this far landward of the zero line.
pub const FAR_SHORE_REACH_M: f64 = 4.0;
/// `bakeCitywideShoreField` default `maxTexels`.
pub const MAX_TEXELS: usize = 2048;
/// Bumped when the bake or the cache layout changes.
pub const FIELD_VERSION: &str = "water-field-v1-citywide-chamfer";

/// (water r2) Whether ring land a pier's width across that the surveyed
/// ground puts under the sea is pressed to the seabed instead of lifted 10 cm
/// over it. The DataSF shoreline rings draw the piers that stand on piles
/// (Torpedo Wharf off Crissy Field, the Municipal Pier, Pier 7) as land, but
/// a bare-earth survey sees the bay floor under their decks, so the lift laid
/// a flat sand slab at the waterline where the pier stands; the water now
/// draws over it (eo-render FEAT_SHORE). Off with `--no-water-shore`.
pub static PIERS_OVER_WATER: AtomicBool = AtomicBool::new(true);
/// The deepest a point lies inside a pier's footprint (m): half the widest
/// pier taken as one (about 30 m).
pub const PIER_MAX_HALF_M: f64 = 15.0;
/// How far past the far side of a pier the probe looks for the sea (m).
pub const PIER_PROBE_M: f64 = 10.0;

// the beach foreshore. The shoreline rings put a beach's waterline on the
// surveyed high-water line, but the surveyed sand only meets `SEA_LEVEL_Y`
// (the still sea the water draws at) 30-40 m further out: at Ocean Beach the
// ground on the ring line stands 0.75 m above the sea and falls about 1:50,
// so from the ring the sea was a strip far across dry sand. On a beach the
// sand is laid on the surf model's 1:55 foreshore through the sea level at the
// ring (never raised: the lower of the two), down to `SEABED_Y`, and blended
// back to the surveyed ground between 45 and 90 m inland (the berm), so the
// promenade and seawall behind stay where they are.
/// Foreshore slope (`water_surf.wgsl` SF_SLOPE, the surf model's profile).
pub const BEACH_FORESHORE_SLOPE: f64 = 1.0 / 55.0;
/// Inland distances (m) over which the foreshore hands back to the surveyed ground.
pub const BEACH_BLEND_M: [f64; 2] = [45.0, 90.0];

fn smooth(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The beach ground at signed shore distance `s` (positive seaward) for a
/// surveyed height `y`: the lower of the two near the sea, the surveyed
/// ground past `BEACH_BLEND_M[1]` inland.
pub fn beach_foreshore(s: f64, y: f64) -> f64 {
    let profile = (crate::shoreline::SEA_LEVEL_Y - s * BEACH_FORESHORE_SLOPE).max(crate::shoreline::SEABED_Y);
    let pressed = y.min(profile);
    let keep = smooth(-BEACH_BLEND_M[0], -BEACH_BLEND_M[1], s);
    pressed + (y - pressed) * keep
}

/// A beach polygon (sf-lots-parks `greens` of kind beach: `sfShoreBeaches`).
#[derive(Clone, Debug)]
pub struct Beach {
    pub polygon: Vec<[f64; 2]>,
}

#[derive(Clone, Debug)]
pub struct ShoreField {
    pub origin: [f64; 2],
    pub size: [f64; 2],
    pub width: usize,
    pub height: usize,
    /// Interleaved (signed distance, beach) as three's half floats.
    pub half: Vec<u16>,
    pub range: [f64; 2],
    pub beach_texels: usize,
    pub bake_ms: f64,
}

/// three's `DataUtils.toHalfFloat` (table form: truncation, not rounding).
pub fn to_half_three(val: f64) -> u16 {
    let v = val.clamp(-65504.0, 65504.0) as f32;
    let f = v.to_bits();
    let e = ((f >> 23) & 0x1ff) as usize;
    let (base, shift) = half_tables(e);
    (base + ((f & 0x007f_ffff) >> shift)) as u16
}

fn half_tables(i9: usize) -> (u32, u32) {
    let sign = if i9 & 0x100 != 0 { 0x8000 } else { 0 };
    let i = (i9 & 0xff) as i32;
    let e = i - 127;
    if e < -27 {
        (sign, 24)
    } else if e < -14 {
        ((0x0400 >> (-e - 14)) | sign, (-e - 1) as u32)
    } else if e <= 15 {
        ((((e + 15) as u32) << 10) | sign, 13)
    } else if e < 128 {
        (0x7c00 | sign, 24)
    } else {
        (0x7c00 | sign, 13)
    }
}

/// IEEE half -> f32 (exact).
pub fn half_to_f32(h: u16) -> f32 {
    let s = (h >> 15) as u32;
    let e = ((h >> 10) & 0x1f) as i32;
    let m = (h & 0x3ff) as u32;
    let v = if e == 0 {
        (m as f32) * 2f32.powi(-24)
    } else if e == 31 {
        if m == 0 { f32::INFINITY } else { f32::NAN }
    } else {
        f32::from_bits(((e + 112) as u32) << 23 | (m << 13))
    };
    if s == 1 { -v } else { v }
}

/// JS `Math.round` (half rounds toward +inf).
fn js_round(v: f64) -> f64 {
    (v + 0.5).floor()
}

/// `chamfer`: two-pass 8-neighbour chamfer in place (the JS stores into a
/// Float32Array, so every assignment rounds to f32 as here).
fn chamfer(dist: &mut [f32], width: usize, height: usize, step_x: f64, step_z: f64) {
    let diagonal = step_x.hypot(step_z);
    for row in 0..height {
        for col in 0..width {
            let i = row * width + col;
            let mut d = dist[i] as f64;
            if col > 0 {
                d = d.min(dist[i - 1] as f64 + step_x);
            }
            if row > 0 {
                d = d.min(dist[i - width] as f64 + step_z);
                if col > 0 {
                    d = d.min(dist[i - width - 1] as f64 + diagonal);
                }
                if col < width - 1 {
                    d = d.min(dist[i - width + 1] as f64 + diagonal);
                }
            }
            dist[i] = d as f32;
        }
    }
    for row in (0..height).rev() {
        for col in (0..width).rev() {
            let i = row * width + col;
            let mut d = dist[i] as f64;
            if col < width - 1 {
                d = d.min(dist[i + 1] as f64 + step_x);
            }
            if row < height - 1 {
                d = d.min(dist[i + width] as f64 + step_z);
                if col < width - 1 {
                    d = d.min(dist[i + width + 1] as f64 + diagonal);
                }
                if col > 0 {
                    d = d.min(dist[i + width - 1] as f64 + diagonal);
                }
            }
            dist[i] = d as f32;
        }
    }
}

/// `beachDistanceGrid`.
fn beach_distance_grid(beaches: &[Beach], min_x: f64, min_z: f64, step_x: f64, step_z: f64, width: usize, height: usize) -> Option<Vec<f32>> {
    let polys: Vec<&Beach> = beaches.iter().filter(|b| b.polygon.len() >= 3).collect();
    if polys.is_empty() {
        return None;
    }
    let mut dist = vec![f32::INFINITY; width * height];
    let col_of = |x: f64| js_round((x - min_x) / step_x) as i64;
    let row_of = |z: f64| js_round((z - min_z) / step_z) as i64;
    for b in polys {
        let poly = &b.polygon;
        let (mut x0, mut x1, mut z0, mut z1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
        for p in poly {
            x0 = x0.min(p[0]);
            x1 = x1.max(p[0]);
            z0 = z0.min(p[1]);
            z1 = z1.max(p[1]);
            let (c, r) = (col_of(p[0]), row_of(p[1]));
            if c >= 0 && (c as usize) < width && r >= 0 && (r as usize) < height {
                dist[r as usize * width + c as usize] = 0.0;
            }
        }
        let (ra, rb) = (row_of(z0).max(0), row_of(z1).min(height as i64 - 1));
        let (ca, cb) = (col_of(x0).max(0), col_of(x1).min(width as i64 - 1));
        for r in ra..=rb {
            let z = min_z + step_z * r as f64;
            for c in ca..=cb {
                let x = min_x + step_x * c as f64;
                let mut inside = false;
                let n = poly.len();
                let mut j = n - 1;
                for i in 0..n {
                    let (a, bb) = (poly[j], poly[i]);
                    if (bb[1] > z) != (a[1] > z) && x < ((a[0] - bb[0]) * (z - bb[1])) / (a[1] - bb[1]) + bb[0] {
                        inside = !inside;
                    }
                    j = i;
                }
                if inside {
                    dist[r as usize * width + c as usize] = 0.0;
                }
            }
        }
    }
    chamfer(&mut dist, width, height, step_x, step_z);
    Some(dist)
}

/// `bakeCitywideShoreField(oracle, { cellM, maxTexels, beaches })`.
pub fn bake_citywide(oracle: &Oracle, cell_m: f64, max_texels: usize, beaches: &[Beach]) -> ShoreField {
    let t = std::time::Instant::now();
    let r = oracle.region;
    let width = 2.max(max_texels.min(((r.max_x - r.min_x) / cell_m).ceil() as usize + 1));
    let height = 2.max(max_texels.min(((r.max_z - r.min_z) / cell_m).ceil() as usize + 1));
    let (span_x, span_z) = (r.max_x - r.min_x, r.max_z - r.min_z);
    let (step_x, step_z) = (span_x / (width - 1) as f64, span_z / (height - 1) as f64);
    let diagonal = step_x.hypot(step_z);
    let count = width * height;
    let threads = std::thread::available_parallelism().map(|p| p.get()).unwrap_or(4).clamp(1, 16);
    let rows_per = height.div_ceil(threads);
    // Parity per texel (rows in parallel: each touches its own parity row).
    let mut water = vec![0u8; count];
    std::thread::scope(|s| {
        for (k, chunk) in water.chunks_mut(rows_per * width).enumerate() {
            s.spawn(move || {
                for (j, row) in chunk.chunks_mut(width).enumerate() {
                    let z = r.min_z + step_z * (k * rows_per + j) as f64;
                    for (col, w) in row.iter_mut().enumerate() {
                        *w = oracle.is_water(r.min_x + step_x * col as f64, z) as u8;
                    }
                }
            });
        }
    });
    // Seeds: exact edge distance where a neighbour has the other parity.
    let seed_cap = diagonal * 1.5;
    let mut dist = vec![f32::INFINITY; count];
    {
        let water = &water;
        std::thread::scope(|s| {
            for (k, chunk) in dist.chunks_mut(rows_per * width).enumerate() {
                s.spawn(move || {
                    for (j, out) in chunk.chunks_mut(width).enumerate() {
                        let row = k * rows_per + j;
                        for col in 0..width {
                            let i = row * width + col;
                            let mut shore = false;
                            'n: for dr in -1i64..=1 {
                                let rr = row as i64 + dr;
                                if rr < 0 || rr >= height as i64 {
                                    continue;
                                }
                                for dc in -1i64..=1 {
                                    let c = col as i64 + dc;
                                    if c >= 0 && c < width as i64 && water[rr as usize * width + c as usize] != water[i] {
                                        shore = true;
                                        break 'n;
                                    }
                                }
                            }
                            if shore {
                                out[col] = oracle.edge_distance(r.min_x + step_x * col as f64, r.min_z + step_z * row as f64, seed_cap) as f32;
                            }
                        }
                    }
                });
            }
        });
    }
    chamfer(&mut dist, width, height, step_x, step_z);
    let beach_dist = beach_distance_grid(beaches, r.min_x, r.min_z, step_x, step_z, width, height);
    let mut half = vec![0u16; count * 2];
    let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
    let mut beach_texels = 0;
    for i in 0..count {
        let d = if dist[i].is_finite() { (dist[i] as f64).min(FIELD_CAP_M) } else { FIELD_CAP_M };
        let signed = if water[i] == 1 { d } else { -d };
        low = low.min(signed);
        high = high.max(signed);
        let mut beach = 0.0;
        if let Some(bd) = &beach_dist {
            if signed > -40.0 {
                let reach = signed.max(0.0) + 140.0;
                beach = ((reach - bd[i] as f64) / 60.0).clamp(0.0, 1.0);
                if beach > 0.0 {
                    beach_texels += 1;
                }
            }
        }
        half[i * 2] = to_half_three(signed);
        half[i * 2 + 1] = to_half_three(beach);
    }
    let round1 = |v: f64| (v * 10.0).round() / 10.0;
    ShoreField {
        origin: [r.min_x, r.min_z],
        size: [span_x, span_z],
        width,
        height,
        half,
        range: [round1(low), round1(high)],
        beach_texels,
        bake_ms: t.elapsed().as_secs_f64() * 1e3,
    }
}

impl ShoreField {
    pub fn signed(&self, i: usize) -> f32 {
        half_to_f32(self.half[i * 2])
    }
    pub fn beach(&self, i: usize) -> f32 {
        half_to_f32(self.half[i * 2 + 1])
    }

    /// Bilinear sample of the signed distance at a world point (clamped to
    /// the edge, as the water shader samples it).
    pub fn signed_at(&self, x: f64, z: f64) -> f64 {
        self.signed_exact(x, z)
    }

    /// Bilinear sample of the beach weight (channel G).
    pub fn beach_at(&self, x: f64, z: f64) -> f64 {
        let u = ((x - self.origin[0]) / self.size[0]).clamp(0.0, 1.0) * (self.width - 1) as f64;
        let v = ((z - self.origin[1]) / self.size[1]).clamp(0.0, 1.0) * (self.height - 1) as f64;
        let (c, r) = (u.floor() as usize, v.floor() as usize);
        let (tx, tz) = (u - c as f64, v - r as f64);
        let at = |c: usize, r: usize| self.beach(r.min(self.height - 1) * self.width + c.min(self.width - 1)) as f64;
        let a = at(c, r) * (1.0 - tx) + at(c + 1, r) * tx;
        let b = at(c, r + 1) * (1.0 - tx) + at(c + 1, r + 1) * tx;
        a * (1.0 - tz) + b * tz
    }

    /// The JS ground rule at the shore (ground-coverage-contract.js
    /// `canonicalGroundCoverageOptions`: where the shoreline says water, the
    /// ground carpet is pressed to `SEABED_Y`), for a ground height `y` at a
    /// world point, so the waterline follows the shoreline rings as the JS's
    /// does. Not where the nearest shore is a beach: there (sea) the sand is
    /// laid on the foreshore (`beach_foreshore`).
    /// What differs: the JS's near-shore land ramp (land within 36 m of
    /// the water pressed toward the sea) is not applied, because the streets
    /// and buildings here stand on the surveyed ground; instead ring land
    /// within `LAND_LIFT_REACH_M` of the water that the surveyed ground puts
    /// below the sea (the lidar / CUDEM blend along a seawall) is held
    /// `LAND_LIFT_M` above it, which is what the rings say it is.
    pub fn press_ground(&self, x: f64, z: f64, y: f64) -> f64 {
        self.press_beach(x, z, self.press_ground_rings(x, z, y))
    }

    /// the beach part of `press_ground` alone: at a beach the sand laid
    /// on the foreshore (`beach_foreshore`), weighted by the field's beach
    /// channel; elsewhere `y`. The one function the drawn carpet and the
    /// collision ground both read (eo-street `StreetWorld::set_shore_press`).
    /// Keyed to the sea level and the shore field, not to the ground's height.
    pub fn press_beach(&self, x: f64, z: f64, y: f64) -> f64 {
        let s = self.signed_exact(x, z);
        if s <= -BEACH_BLEND_M[1] {
            return y;
        }
        let b = self.beach_at(x, z);
        if b <= 0.0 {
            return y;
        }
        y + (beach_foreshore(s, y) - y) * smooth(0.25, 0.6, b)
    }

    /// `press_ground` without the beach (the rings' seabed and land lift).
    pub fn press_ground_rings(&self, x: f64, z: f64, y: f64) -> f64 {
        const LAND_LIFT_REACH_M: f64 = 24.0;
        const LAND_LIFT_M: f64 = 0.1;
        let s = self.signed_exact(x, z);
        if s > 0.0 && y > crate::shoreline::SEABED_Y && self.beach_at(x, z) < 0.5 {
            return crate::shoreline::SEABED_Y;
        }
        if s > 0.0 && s < PIER_PROBE_M && y > crate::shoreline::SEABED_Y && PIERS_OVER_WATER.load(Ordering::Relaxed) {
            // the water beside a pier where the nearest shore is otherwise a
            // beach (Torpedo Wharf off Crissy Field): its flanks in the survey
            // raster are the pier's deck smeared under the sea, not a foreshore
            let g = [self.signed_exact(x + 1.0, z) - self.signed_exact(x - 1.0, z), self.signed_exact(x, z + 1.0) - self.signed_exact(x, z - 1.0)];
            let gl = g[0].hypot(g[1]);
            if gl > 1e-6 {
                let (lx, lz) = (x - g[0] / gl * (s + 2.0), z - g[1] / gl * (s + 2.0));
                let ls = self.signed_exact(lx, lz);
                if ls <= 0.0 && self.is_pier(lx, lz, ls) {
                    return crate::shoreline::SEABED_Y;
                }
            }
        }
        let sea = crate::shoreline::SEA_LEVEL_Y;
        if s <= 0.0 && s > -LAND_LIFT_REACH_M && y < sea + LAND_LIFT_M && self.beach_at(x, z) < 0.5 {
            if PIERS_OVER_WATER.load(Ordering::Relaxed) && self.is_pier(x, z, s) {
                // a pier's footprint: the sea under its deck (see `PIERS_OVER_WATER`)
                return y.min(crate::shoreline::SEABED_Y);
            }
            return sea + LAND_LIFT_M;
        }
        y
    }

    /// Whether ring land at `(x, z)` (signed distance `s`, at most 0) is a
    /// pier's footprint: no deeper than `PIER_MAX_HALF_M` into the land, and
    /// the sea again on both sides of it along one of four directions within
    /// twice that depth plus `PIER_PROBE_M` (a strip under about 40 m wide).
    pub fn is_pier(&self, x: f64, z: f64, s: f64) -> bool {
        if -s > PIER_MAX_HALF_M {
            return false;
        }
        let d = 2.0 * -s + PIER_PROBE_M;
        const DIRS: [[f64; 2]; 4] = [[1.0, 0.0], [0.0, 1.0], [std::f64::consts::FRAC_1_SQRT_2, std::f64::consts::FRAC_1_SQRT_2], [std::f64::consts::FRAC_1_SQRT_2, -std::f64::consts::FRAC_1_SQRT_2]];
        DIRS.iter().any(|u| self.signed_exact(x + u[0] * d, z + u[1] * d) > 0.0 && self.signed_exact(x - u[0] * d, z - u[1] * d) > 0.0)
    }

    /// The signed distance at a world point, texel centres at origin + step i (as baked).
    pub fn signed_exact(&self, x: f64, z: f64) -> f64 {
        let u = ((x - self.origin[0]) / self.size[0]).clamp(0.0, 1.0) * (self.width - 1) as f64;
        let v = ((z - self.origin[1]) / self.size[1]).clamp(0.0, 1.0) * (self.height - 1) as f64;
        let (c, r) = (u.floor() as usize, v.floor() as usize);
        let (tx, tz) = (u - c as f64, v - r as f64);
        let at = |c: usize, r: usize| self.signed(r.min(self.height - 1) * self.width + c.min(self.width - 1)) as f64;
        let a = at(c, r) * (1.0 - tx) + at(c + 1, r) * tx;
        let b = at(c, r + 1) * (1.0 - tx) + at(c + 1, r + 1) * tx;
        a * (1.0 - tz) + b * tz
    }

    /// Max-reduction pyramid of the signed distance at a GPU mip chain's
    /// sizes (level L is max(1, size >> L)): texel c of level L holds the max
    /// of texels 2c and 2c + 1 of level L - 1, and the last texel of a row or
    /// column also the odd one left over, so texel (a >> L) of level L covers
    /// every level-0 texel a maps to. A grid patch whose footprint's max lies
    /// below the draw mask is wholly over land.
    pub fn max_pyramid(&self) -> Vec<(usize, usize, Vec<f32>)> {
        let mut levels = vec![(self.width, self.height, (0..self.width * self.height).map(|i| self.signed(i)).collect::<Vec<f32>>())];
        loop {
            let (w, h, prev) = levels.last().unwrap();
            let (w, h) = (*w, *h);
            if w == 1 && h == 1 {
                break;
            }
            let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
            let span = |i: usize, n: usize, old: usize| -> std::ops::Range<usize> {
                let lo = (2 * i).min(old - 1);
                let hi = if i == n - 1 { old } else { (2 * i + 2).min(old) };
                lo..hi.max(lo + 1)
            };
            let mut out = vec![f32::NEG_INFINITY; nw * nh];
            for r in 0..nh {
                for c in 0..nw {
                    let mut m = f32::NEG_INFINITY;
                    for rr in span(r, nh, h) {
                        for cc in span(c, nw, w) {
                            m = m.max(prev[rr * w + cc]);
                        }
                    }
                    out[r * nw + c] = m;
                }
            }
            levels.push((nw, nh, out));
        }
        levels
    }

    /// The cache file: magic, version, frame, then the halfs; SHA-256 last.
    pub fn to_bytes(&self) -> Vec<u8> {
        use sha2::{Digest, Sha256};
        let mut b = b"EOWF".to_vec();
        let ver = FIELD_VERSION.as_bytes();
        b.extend_from_slice(&(ver.len() as u32).to_le_bytes());
        b.extend_from_slice(ver);
        for v in [self.origin[0], self.origin[1], self.size[0], self.size[1], self.range[0], self.range[1]] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&(self.width as u32).to_le_bytes());
        b.extend_from_slice(&(self.height as u32).to_le_bytes());
        b.extend_from_slice(&(self.beach_texels as u32).to_le_bytes());
        for h in &self.half {
            b.extend_from_slice(&h.to_le_bytes());
        }
        let d = Sha256::digest(&b);
        b.extend_from_slice(d.as_slice());
        b
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self> {
        use sha2::{Digest, Sha256};
        if b.len() < 40 || &b[..4] != b"EOWF" {
            bail!("not a water field file");
        }
        let (body, sum) = b.split_at(b.len() - 32);
        if Sha256::digest(body).as_slice() != sum {
            bail!("water field file damaged (digest)");
        }
        let mut at = 4;
        let take = |at: &mut usize, n: usize| -> Result<&[u8]> {
            let s = body.get(*at..*at + n).context("water field file truncated")?;
            *at += n;
            Ok(s)
        };
        let vl = u32::from_le_bytes(take(&mut at, 4)?.try_into()?) as usize;
        if take(&mut at, vl)? != FIELD_VERSION.as_bytes() {
            bail!("water field file of another version");
        }
        let mut f = [0f64; 6];
        for v in &mut f {
            *v = f64::from_le_bytes(take(&mut at, 8)?.try_into()?);
        }
        let width = u32::from_le_bytes(take(&mut at, 4)?.try_into()?) as usize;
        let height = u32::from_le_bytes(take(&mut at, 4)?.try_into()?) as usize;
        let beach_texels = u32::from_le_bytes(take(&mut at, 4)?.try_into()?) as usize;
        let raw = take(&mut at, width * height * 4)?;
        let half = raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        Ok(Self { origin: [f[0], f[1]], size: [f[2], f[3]], width, height, half, range: [f[4], f[5]], beach_texels, bake_ms: 0.0 })
    }
}

#[cfg(test)]
mod sea_tests {
    use super::*;
    use crate::shoreline::{SEABED_Y, SEA_LEVEL_Y};

    // a field with a mainland (z < 0) and a 12 m wide pier running out along
    // +z from it, 1 m texels: the signed distance to the nearest of the two
    fn pier_field() -> ShoreField {
        let (w, h) = (161usize, 161usize);
        let mut half = vec![0u16; w * h * 2];
        for r in 0..h {
            for c in 0..w {
                let (x, z) = (c as f64 - 80.0, r as f64 - 40.0);
                // the mainland's edge at z = 0; the pier x in [-6, 6], z in [0, 100]
                let d_main = z;
                let (dx, dz) = (x.abs() - 6.0, (z - 50.0).abs() - 50.0);
                let d_pier = if dx <= 0.0 && dz <= 0.0 { dx.max(dz) } else { dx.max(0.0).hypot(dz.max(0.0)) };
                half[(r * w + c) * 2] = to_half_three(d_main.min(d_pier));
            }
        }
        ShoreField { origin: [-80.0, -40.0], size: [160.0, 160.0], width: w, height: h, half, range: [-40.0, 120.0], beach_texels: 0, bake_ms: 0.0 }
    }

    // a pier's footprint the survey puts under the sea is pressed to the
    // seabed (the water draws over it); the mainland's edge keeps the lift
    #[test]
    fn a_sunken_pier_footprint_is_sea_and_the_seawall_keeps_its_lift() {
        let f = pier_field();
        let y = SEA_LEVEL_Y - 0.3;
        // on the pier's axis, 60 m out: the sea's floor under the deck
        assert_eq!(f.press_ground_rings(0.0, 60.0, y), y.min(SEABED_Y));
        // beside it in the water, where the field's shore is the pier: the seabed
        assert_eq!(f.press_ground_rings(9.0, 60.0, -1.0), SEABED_Y);
        // the mainland 5 m in from its seawall, away from the pier: lifted over the sea as before
        assert!((f.press_ground_rings(-50.0, -5.0, y) - (SEA_LEVEL_Y + 0.1)).abs() < 1e-9);
        // ground standing above the sea is never touched
        assert_eq!(f.press_ground_rings(0.0, 60.0, 2.0), 2.0);
        // with the switch off, the earlier lift on the pier too
        PIERS_OVER_WATER.store(false, Ordering::Relaxed);
        let off = f.press_ground_rings(0.0, 60.0, y);
        PIERS_OVER_WATER.store(true, Ordering::Relaxed);
        assert!((off - (SEA_LEVEL_Y + 0.1)).abs() < 1e-9);
    }

    // the foreshore meets the sea at the ring line, never raises the
    // ground, reaches the seabed offshore and hands back to the surveyed ground inland.
    #[test]
    fn beach_foreshore_meets_the_sea_at_the_ring() {
        assert!((beach_foreshore(0.0, 0.2) - SEA_LEVEL_Y).abs() < 1e-12);
        assert_eq!(beach_foreshore(-10.0, -1.0), -1.0);
        assert_eq!(beach_foreshore(500.0, 0.2), SEABED_Y);
        assert_eq!(beach_foreshore(-BEACH_BLEND_M[1], 3.0), 3.0);
        let mut last = f64::MAX;
        for i in 0..200 {
            let s = -100.0 + i as f64;
            let y = beach_foreshore(s, 0.75 - s / 50.0);
            assert!(y <= last + 1e-9, "the foreshore rises seaward at {s}");
            last = y;
        }
    }
}
