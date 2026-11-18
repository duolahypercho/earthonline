//! the shore-aligned surf strip (see native/README.md "Surf strip").
//!
//! Per mapped beach, a polyline along the still waterline (the zero line of
//! the citywide shore field, marching squares on its signed channel), run on
//! along the shore past the beach polygon's ends, resampled every
//! `SAMPLE_M` metres and smoothed, with offshore normals. The renderer lays a
//! grid over (s along the polyline, n offshore) and moves its rows with the
//! waves, so the rows crowd at the crests.
//!
//! Per strip, tables along n (1 m) for its wave climate:
//!
//! - the bathymetry (Ocean Beach: the 1:55 foreshore the drawn sand lies on,
//!   an inner bar at about 105 m and an outer bar at about 235 m; a bay beach:
//!   one slope, no bars);
//! - the travel time from the outer edge (linear dispersion at the climate's
//!   period; the bore's run-up speed on the sand) and its inverse;
//! - per incoming height level, the wave height as the wave shoals (energy
//!   flux conserved), breaks (H >= 0.78 h, McCowan), decays toward the stable
//!   bore height (Dally, Dean and Dean 1985; a saturated surf zone: K 0.1, stable 0.5 h), reforms
//!   when it falls under 0.35 h in a trough (a bore keeps breaking across the runnel to the sand), and breaks again; with the
//!   metres travelled since the break began and the breaker type from the
//!   bed slope at the break (Iribarren number: spilling under 0.5, plunging
//!   0.5-3.3).
//!
//! `surf_cover` is the one hand-over rule both the sea and the strip read:
//! 1 inside the strip (the sea stands down), 0 past its outer ring.

use crate::field::{Beach, ShoreField};

/// Polyline spacing (m).
pub const SAMPLE_M: f64 = 4.0;
/// How far the strip runs on along the shore past a beach polygon (m).
pub const EXTEND_M: f64 = 600.0;
/// Landward reach of the strip (m, negative: onto the sand).
pub const N_LAND_M: f64 = -80.0;
/// Where the hand-over fades out on the sand (signed distance, m): well inside the strip's reach.
pub const LAND_FADE_M: [f64; 2] = [-42.0, -32.0];
/// Table step along n (m).
pub const TABLE_STEP_M: f64 = 1.0;
/// Incoming wave height levels in the break table.
pub const LEVELS: usize = 16;
/// Travel-time table step (s).
pub const TAU_STEP_S: f64 = 0.05;
/// The hand-over ring on the field's signed distance (outer, inner), Pacific
/// and bay. The field is capped at 400 m, so the Pacific ring ends inside it.
pub const RING_PACIFIC_M: [f64; 2] = [395.0, 330.0];
pub const RING_BAY_M: [f64; 2] = [105.0, 80.0];
/// The open-Pacific share over which the hand-over comes in: the strip is
/// built for the beaches the Pacific swell reaches (Ocean Beach, Baker and
/// China beaches); the bay's beaches keep the sea's analytic surf for now.
pub const PACIFIC_SHARE: [f64; 2] = [0.45, 0.75];
/// How far past the hand-over ring the strip reaches (m): its normals are
/// smoothed, so its n and the field's distance part by tens of metres.
pub const RING_MARGIN_M: f64 = 60.0;
/// The beach weight over which the cover comes in (the field's beach channel).
pub const RING_BEACH: [f64; 2] = [0.85, 0.98];
/// The swell train the Pacific strip's waves continue (surf r5: the sea's lead swell
/// mode, eo-water `waves` `Plan::lead_amp`: 315 m (2597.55 / sqrt(68)) from
/// (0.9701, 0.2425), h = a sin(k.x - w t), no meander; it was `w_swell`'s 210 m train).
pub const LOCK_LAMBDA_M: f64 = 315.0;
pub const LOCK_DIR: [f64; 2] = [0.9701, 0.2425];

const G: f64 = 9.81;
const GAMMA_BREAK: f64 = 0.78;
const GAMMA_STABLE: f64 = 0.5;
const GAMMA_REFORM: f64 = 0.35;
const DALLY_K: f64 = 0.1;
/// The bore's speed up the sand (m/s): the rows keep moving onto the beach.
const LAND_SPEED: f64 = 3.0;

fn smooth(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The sea's open-Pacific share at world x (water_common.wgsl `wg_ocean`).
pub fn ocean_share(x: f64) -> f64 {
    smooth(crate::optics::GOLDEN_GATE_X[0], crate::optics::GOLDEN_GATE_X[1] - 2200.0, x)
}

/// The hand-over: 1 where the strip draws alone, 0 where the sea draws alone
/// (`water_surf.wgsl` `surf_cover`, the same numbers).
pub fn surf_cover(x: f64, signed: f64, beach: f64) -> f64 {
    let o = ocean_share(x);
    let outer = RING_BAY_M[0] + (RING_PACIFIC_M[0] - RING_BAY_M[0]) * o;
    let inner = RING_BAY_M[1] + (RING_PACIFIC_M[1] - RING_BAY_M[1]) * o;
    smooth(RING_BEACH[0], RING_BEACH[1], beach) * smooth(outer, inner, signed) * smooth(LAND_FADE_M[0], LAND_FADE_M[1], signed) * smooth(PACIFIC_SHARE[0], PACIFIC_SHARE[1], o)
}

/// A strip's wave climate.
#[derive(Clone, Copy, Debug)]
pub struct Climate {
    /// Open Pacific (Ocean Beach) or the bay.
    pub pacific: bool,
    /// Nominal incoming wave height (m) on a clear day.
    pub h0: f64,
    /// The break table's largest incoming height (m).
    pub hmax: f64,
    /// Wave period (s).
    pub period: f64,
    /// Outer edge of the strip (m offshore).
    pub n_max: f64,
}

impl Climate {
    pub fn pacific() -> Self {
        let k = std::f64::consts::TAU / LOCK_LAMBDA_M;
        Self { pacific: true, h0: 2.8, hmax: 8.0, period: std::f64::consts::TAU / (G * k).sqrt(), n_max: RING_PACIFIC_M[0] + RING_MARGIN_M }
    }
    pub fn bay() -> Self {
        Self { pacific: false, h0: 0.25, hmax: 0.9, period: 3.6, n_max: 120.0 }
    }
}

/// The tables along n of one strip.
#[derive(Clone, Debug)]
pub struct Tables {
    /// n of entry 0 (m) and the step.
    pub n0: f64,
    pub step: f64,
    /// Per n: (depth m, travel time s, phase speed m/s, bed slope).
    pub along: Vec<[f32; 4]>,
    /// n at travel time i * TAU_STEP_S.
    pub n_of_tau: Vec<f32>,
    /// Per n, LEVELS entries: (H m, metres since the break began or -1, break intensity 0..1, plunging 0..1).
    pub waves: Vec<[f32; 4]>,
}

#[derive(Clone, Debug)]
pub struct Strip {
    /// Waterline samples (world x, z), every `SAMPLE_M` from s = 0.
    pub points: Vec<[f64; 2]>,
    /// Unit offshore normals per sample.
    pub normals: Vec<[f64; 2]>,
    /// 1 along the beach, 0 at the run's ends (the surf's share).
    pub beach_w: Vec<f32>,
    pub climate: Climate,
    pub tables: Tables,
}

impl Strip {
    pub fn length(&self) -> f64 {
        (self.points.len().max(1) - 1) as f64 * SAMPLE_M
    }
    /// World point at (s, n).
    pub fn at(&self, s: f64, n: f64) -> [f64; 2] {
        let f = (s / SAMPLE_M).clamp(0.0, (self.points.len() - 1) as f64);
        let i = (f.floor() as usize).min(self.points.len().saturating_sub(2));
        let t = f - i as f64;
        let j = (i + 1).min(self.points.len() - 1);
        let p = [self.points[i][0] * (1.0 - t) + self.points[j][0] * t, self.points[i][1] * (1.0 - t) + self.points[j][1] * t];
        let mut nr = [self.normals[i][0] * (1.0 - t) + self.normals[j][0] * t, self.normals[i][1] * (1.0 - t) + self.normals[j][1] * t];
        let l = (nr[0] * nr[0] + nr[1] * nr[1]).sqrt().max(1e-9);
        nr = [nr[0] / l, nr[1] / l];
        [p[0] + nr[0] * n, p[1] + nr[1] * n]
    }
    /// (s, n) of a world point: the nearest sample's frame (brute force).
    pub fn project(&self, x: f64, z: f64) -> (f64, f64) {
        let mut best = (f64::INFINITY, 0usize);
        for (i, p) in self.points.iter().enumerate() {
            let d = (p[0] - x).powi(2) + (p[1] - z).powi(2);
            if d < best.0 {
                best = (d, i);
            }
        }
        let i = best.1;
        let p = self.points[i];
        let nr = self.normals[i];
        let mut sn = (i as f64 * SAMPLE_M, (x - p[0]) * nr[0] + (z - p[1]) * nr[1]);
        // the normals are smoothed, so the frame through a far point is not the
        // nearest sample's: walk (s, n) until at(s, n) lands on the point
        for _ in 0..24 {
            let q = self.at(sn.0, sn.1);
            let (dx, dz) = (x - q[0], z - q[1]);
            if dx * dx + dz * dz < 1e-6 {
                break;
            }
            let e = 0.5;
            let qa = self.at(sn.0 + e, sn.1);
            let qb = self.at(sn.0, sn.1 + e);
            // the 2 x 2 Jacobian of at(s, n), solved for the step
            let (j00, j10, j01, j11) = ((qa[0] - q[0]) / e, (qa[1] - q[1]) / e, (qb[0] - q[0]) / e, (qb[1] - q[1]) / e);
            let det = j00 * j11 - j01 * j10;
            if det.abs() < 1e-9 {
                break;
            }
            let ds = (dx * j11 - dz * j01) / det;
            let dn = (j00 * dz - j10 * dx) / det;
            sn = (sn.0 + ds.clamp(-200.0, 200.0), sn.1 + dn.clamp(-200.0, 200.0));
        }
        sn
    }
    /// Bounding box (min x, min z, max x, max z) of the strip's reach.
    pub fn bounds(&self) -> [f64; 4] {
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for (p, nr) in self.points.iter().zip(&self.normals) {
            for n in [N_LAND_M, self.climate.n_max] {
                let q = [p[0] + nr[0] * n, p[1] + nr[1] * n];
                b = [b[0].min(q[0]), b[1].min(q[1]), b[2].max(q[0]), b[3].max(q[1])];
            }
        }
        b
    }
}

/// Depth (m below the still sea; negative on the sand) at offshore distance n.
pub fn depth(c: &Climate, n: f64) -> f64 {
    if c.pacific {
        // the drawn foreshore's 1:55 (eo-water field BEACH_FORESHORE_SLOPE) plus two bars
        let base = 0.12 + n / 55.0;
        if n <= 0.0 {
            return base;
        }
        let bars = 0.75 * (-((n - 105.0) / 22.0).powi(2)).exp() + 1.1 * (-((n - 235.0) / 35.0).powi(2)).exp();
        // the shorebreak: a steeper beach face and a runnel behind the inner bar, so the
        // waves that reform there break again close to the sand (bores of 0.5-1.5 m); the
        // drawn sand keeps its 1:55, the strip holds its troughs above it
        let runnel = 0.9 * smooth(0.0, 50.0, n) * (1.0 - smooth(70.0, 130.0, n));
        (base - bars + runnel).max(0.15)
    } else {
        let base = 0.12 + n / 40.0;
        if n <= 0.0 { base } else { base.max(0.12) }
    }
}

/// The strip's along-shore phase offset at s (waves; `surf.wgsl` `sp_delta`, the same
/// formula): the sea's lead swell mode (315 m) continued with a third of its
/// obliquity, plus sections (scaled up in a storm).
pub fn phase_delta(st: &Strip, s: f64, storm: f64) -> f64 {
    let mut d = (0.09 * (s * 0.0273 + 1.1).sin() + 0.05 * (s * 0.0698 + 2.3).sin() + 0.03 * (s * 0.17 + 0.4).sin()) * (1.0 + 1.5 * storm);
    if st.climate.pacific {
        let dir = LOCK_DIR;
        let k = std::f64::consts::TAU / LOCK_LAMBDA_M;
        let ph = |q: [f64; 2]| (q[0] * dir[0] + q[1] * dir[1]) * k;
        let (a, a0) = (ph(st.at(s, st.climate.n_max)), ph(st.at(0.0, st.climate.n_max)));
        d += 0.25 - (a0 + 0.3 * (a - a0)) / std::f64::consts::TAU;
    }
    d
}

/// A water clock near `near` (s) at which the swash at the world point (x, z)
/// is at `phase` of its cycle (0 the bore reaching the waterline, 0.75 the
/// sheet drained back to it with the wet glaze behind): for a card's clock.
pub fn swash_clock(strips: &[Strip], x: f64, z: f64, phase: f64, near: f64, storm: f64) -> Option<f64> {
    let st = strips.iter().min_by(|a, b| {
        let da = a.project(x, z);
        let db = b.project(x, z);
        let ea = if da.0 < 0.0 || da.0 > a.length() { 1e9 } else { da.1.abs() };
        let eb = if db.0 < 0.0 || db.0 > b.length() { 1e9 } else { db.1.abs() };
        ea.partial_cmp(&eb).unwrap()
    })?;
    let (s, _) = st.project(x, z);
    let t = &st.tables;
    let i0 = ((0.0 - t.n0) / t.step).round() as usize;
    let tau0 = t.along[i0.min(t.along.len() - 1)][1] as f64;
    let period = st.climate.period;
    let d = phase_delta(st, s, storm);
    // t = tau0 + (m + phase - d) T, the m that lands nearest `near`
    let m = ((near - tau0) / period - phase + d).round();
    Some(tau0 + (m + phase - d) * period)
}

/// Linear dispersion: wavenumber at depth h for angular frequency w.
fn wavenumber(w: f64, h: f64) -> f64 {
    let mut k = w * w / G;
    let h = h.max(0.02);
    // start from the shallow-water guess where it is the better one
    k = k.max(w / (G * h).sqrt());
    for _ in 0..40 {
        let t = (k * h).tanh();
        let f = G * k * t - w * w;
        let df = G * t + G * k * h * (1.0 - t * t);
        let nk = k - f / df;
        if (nk - k).abs() < 1e-10 {
            k = nk;
            break;
        }
        k = nk.max(1e-6);
    }
    k
}

fn speeds(w: f64, h: f64) -> (f64, f64) {
    if h <= 0.02 {
        return (LAND_SPEED, LAND_SPEED);
    }
    let k = wavenumber(w, h);
    let c = w / k;
    let kh = k * h;
    let n = 0.5 * (1.0 + if kh < 20.0 { 2.0 * kh / (2.0 * kh).sinh() } else { 0.0 });
    (c, c * n)
}

pub fn build_tables(c: &Climate) -> Tables {
    let n0 = N_LAND_M;
    let count = ((c.n_max - n0) / TABLE_STEP_M).round() as usize + 1;
    let w = std::f64::consts::TAU / c.period;
    let mut along = vec![[0f32; 4]; count];
    let ns: Vec<f64> = (0..count).map(|i| n0 + i as f64 * TABLE_STEP_M).collect();
    // travel time from the outer edge, shoreward
    let mut tau = vec![0f64; count];
    for i in (0..count - 1).rev() {
        let h = depth(c, ns[i] + TABLE_STEP_M * 0.5);
        let (cp, _) = speeds(w, h);
        // on the sand the bore runs up at the land speed (never slower than it)
        let v = if h <= 0.02 { LAND_SPEED } else { cp.max(LAND_SPEED.min(cp)) };
        tau[i] = tau[i + 1] + TABLE_STEP_M / v;
    }
    for i in 0..count {
        let h = depth(c, ns[i]);
        let (cp, _) = speeds(w, h.max(0.02));
        let slope = (depth(c, ns[i] + 2.0) - depth(c, ns[i] - 2.0)) / 4.0;
        along[i] = [h as f32, tau[i] as f32, cp as f32, slope as f32];
    }
    // inverse: n at travel time
    let tau_max = tau[0];
    let tcount = (tau_max / TAU_STEP_S).ceil() as usize + 2;
    let mut n_of_tau = vec![n0 as f32; tcount];
    let mut j = count - 1;
    for (k, out) in n_of_tau.iter_mut().enumerate() {
        let target = k as f64 * TAU_STEP_S;
        while j > 0 && tau[j - 1] <= target {
            j -= 1;
        }
        if j == 0 {
            *out = n0 as f32;
            continue;
        }
        // tau[j] <= target < tau[j-1]
        let (ta, tb) = (tau[j], tau[j - 1]);
        let f = if tb > ta { (target - ta) / (tb - ta) } else { 0.0 };
        *out = (ns[j] + (ns[j - 1] - ns[j]) * f) as f32;
    }
    // the break table
    let mut waves = vec![[0f32; 4]; count * LEVELS];
    let l0 = G * c.period * c.period / std::f64::consts::TAU;
    for lv in 0..LEVELS {
        let h_in = c.hmax * lv as f64 / (LEVELS - 1) as f64;
        let h_ref = depth(c, c.n_max);
        let (_, cg_ref) = speeds(w, h_ref);
        let mut flux = h_in * h_in * cg_ref;
        let mut breaking = false;
        let mut onset = 0.0;
        let mut plunge = 0.0;
        let mut hh;
        for i in (0..count).rev() {
            let n = ns[i];
            let h = depth(c, n);
            if h <= 0.03 || h_in <= 0.0 {
                waves[i * LEVELS + lv] = [0.0, -1.0, 0.0, 0.0];
                continue;
            }
            let (_, cg) = speeds(w, h);
            if !breaking {
                hh = (flux / cg).sqrt();
                if hh >= GAMMA_BREAK * h {
                    breaking = true;
                    onset = n;
                    let beta = along[i][3].max(1e-4) as f64;
                    let xi = beta / (hh / l0).sqrt();
                    plunge = smooth(0.25, 0.7, xi);
                    hh = GAMMA_BREAK * h;
                    flux = hh * hh * cg;
                }
            } else {
                let fs = (GAMMA_STABLE * h).powi(2) * cg;
                flux -= DALLY_K / h * (flux - fs) * TABLE_STEP_M;
                flux = flux.max(0.0);
                hh = (flux / cg).sqrt().min(GAMMA_BREAK * h);
                flux = hh * hh * cg;
                if hh < GAMMA_REFORM * h && n > 20.0 {
                    breaking = false;
                }
            }
            let intensity = if breaking { ((hh / h - GAMMA_STABLE) / (GAMMA_BREAK - GAMMA_STABLE)).clamp(0.0, 1.0) } else { 0.0 };
            waves[i * LEVELS + lv] = [hh as f32, if breaking { (onset - n) as f32 } else { -1.0 }, intensity as f32, plunge as f32];
        }
    }
    Tables { n0, step: TABLE_STEP_M, along, n_of_tau, waves }
}

/// Marching squares on the field's signed channel: chains of the zero line.
fn zero_chains(field: &ShoreField) -> Vec<Vec<[f64; 2]>> {
    use std::collections::HashMap;
    let (w, h) = (field.width, field.height);
    let step = [field.size[0] / (w - 1) as f64, field.size[1] / (h - 1) as f64];
    let at = |c: usize, r: usize| field.signed(r * w + c) as f64;
    let pos = |c: usize, r: usize| [field.origin[0] + step[0] * c as f64, field.origin[1] + step[1] * r as f64];
    // edge ids: horizontal (c,r)-(c+1,r) = 2*(r*w+c), vertical (c,r)-(c,r+1) = 2*(r*w+c)+1
    let cross = |a: (usize, usize), b: (usize, usize)| -> [f64; 2] {
        let (va, vb) = (at(a.0, a.1), at(b.0, b.1));
        let t = if (va - vb).abs() > 1e-12 { va / (va - vb) } else { 0.5 };
        let (pa, pb) = (pos(a.0, a.1), pos(b.0, b.1));
        [pa[0] + (pb[0] - pa[0]) * t, pa[1] + (pb[1] - pa[1]) * t]
    };
    let mut segs: Vec<(u64, u64)> = vec![];
    let mut pts: HashMap<u64, [f64; 2]> = HashMap::new();
    for r in 0..h - 1 {
        for c in 0..w - 1 {
            let v = [at(c, r) > 0.0, at(c + 1, r) > 0.0, at(c + 1, r + 1) > 0.0, at(c, r + 1) > 0.0];
            if v[0] == v[1] && v[1] == v[2] && v[2] == v[3] {
                continue;
            }
            let bottom = 2 * (r * w + c) as u64;
            let top = 2 * ((r + 1) * w + c) as u64;
            let left = 2 * (r * w + c) as u64 + 1;
            let right = 2 * (r * w + c + 1) as u64 + 1;
            let mut e = vec![];
            if v[0] != v[1] {
                pts.entry(bottom).or_insert_with(|| cross((c, r), (c + 1, r)));
                e.push(bottom);
            }
            if v[1] != v[2] {
                pts.entry(right).or_insert_with(|| cross((c + 1, r), (c + 1, r + 1)));
                e.push(right);
            }
            if v[3] != v[2] {
                pts.entry(top).or_insert_with(|| cross((c, r + 1), (c + 1, r + 1)));
                e.push(top);
            }
            if v[0] != v[3] {
                pts.entry(left).or_insert_with(|| cross((c, r), (c, r + 1)));
                e.push(left);
            }
            if e.len() == 2 {
                segs.push((e[0], e[1]));
            } else if e.len() == 4 {
                let centre = (at(c, r) + at(c + 1, r) + at(c + 1, r + 1) + at(c, r + 1)) * 0.25 > 0.0;
                if centre == v[0] {
                    segs.push((bottom, right));
                    segs.push((left, top));
                } else {
                    segs.push((bottom, left));
                    segs.push((right, top));
                }
            }
        }
    }
    let mut by_edge: HashMap<u64, Vec<usize>> = HashMap::new();
    for (i, s) in segs.iter().enumerate() {
        by_edge.entry(s.0).or_default().push(i);
        by_edge.entry(s.1).or_default().push(i);
    }
    let mut used = vec![false; segs.len()];
    let mut chains = vec![];
    for start in 0..segs.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        let mut fwd = vec![segs[start].0, segs[start].1];
        // forward from .1, backward from .0
        for dir in 0..2 {
            loop {
                let end = if dir == 0 { *fwd.last().unwrap() } else { fwd[0] };
                let next = by_edge.get(&end).and_then(|l| l.iter().copied().find(|&k| !used[k]));
                let Some(k) = next else { break };
                used[k] = true;
                let other = if segs[k].0 == end { segs[k].1 } else { segs[k].0 };
                if dir == 0 {
                    fwd.push(other);
                } else {
                    fwd.insert(0, other);
                }
            }
        }
        chains.push(fwd.iter().map(|e| pts[e]).collect::<Vec<_>>());
    }
    chains
}

fn point_in_poly(p: [f64; 2], poly: &[[f64; 2]]) -> bool {
    let mut inside = false;
    let n = poly.len();
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (poly[j], poly[i]);
        if (b[1] > p[1]) != (a[1] > p[1]) && p[0] < (a[0] - b[0]) * (p[1] - b[1]) / (a[1] - b[1]) + b[0] {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn seg_dist(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let l2 = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if l2 > 0.0 { (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / l2).clamp(0.0, 1.0) } else { 0.0 };
    let q = [a[0] + ab[0] * t, a[1] + ab[1] * t];
    ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt()
}

/// Distance from a point to the nearest beach polygon (0 inside one).
pub fn beach_distance(p: [f64; 2], beaches: &[Beach]) -> f64 {
    let mut best = f64::INFINITY;
    for b in beaches {
        let poly = &b.polygon;
        if poly.len() < 3 {
            continue;
        }
        // a polygon whose box lies further than the best so far cannot be nearer
        let (mut x0, mut z0, mut x1, mut z1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        for q in poly {
            x0 = x0.min(q[0]);
            z0 = z0.min(q[1]);
            x1 = x1.max(q[0]);
            z1 = z1.max(q[1]);
        }
        let bx = (x0 - p[0]).max(p[0] - x1).max(0.0);
        let bz = (z0 - p[1]).max(p[1] - z1).max(0.0);
        if bx.hypot(bz) > best.min(EXTEND_M + 1.0) {
            continue;
        }
        if point_in_poly(p, poly) {
            return 0.0;
        }
        for i in 0..poly.len() {
            best = best.min(seg_dist(p, poly[i], poly[(i + 1) % poly.len()]));
        }
    }
    best
}

fn gauss_smooth(v: &[[f64; 2]], sigma_samples: f64) -> Vec<[f64; 2]> {
    let r = (sigma_samples * 3.0).ceil() as i64;
    let n = v.len() as i64;
    (0..n)
        .map(|i| {
            let mut acc = [0.0, 0.0];
            let mut wsum = 0.0;
            for k in -r..=r {
                let j = (i + k).clamp(0, n - 1);
                let w = (-(k as f64 * k as f64) / (2.0 * sigma_samples * sigma_samples)).exp();
                acc[0] += v[j as usize][0] * w;
                acc[1] += v[j as usize][1] * w;
                wsum += w;
            }
            [acc[0] / wsum, acc[1] / wsum]
        })
        .collect()
}

fn resample(chain: &[[f64; 2]], step: f64) -> Vec<[f64; 2]> {
    let mut out = vec![chain[0]];
    let mut carry = 0.0;
    for w in chain.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        let mut t = step - carry;
        while t <= len {
            let f = t / len;
            out.push([a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f]);
            t += step;
        }
        carry = len - (t - step);
    }
    out
}

/// Whether the offset curves at any of `ns` run backwards somewhere.
fn folds(p: &[[f64; 2]], tang: &[[f64; 2]], ns: &[f64]) -> bool {
    let nrm = |t: [f64; 2]| {
        let l = (t[0] * t[0] + t[1] * t[1]).sqrt().max(1e-9);
        [t[1] / l, -t[0] / l]
    };
    ns.iter().any(|&n| {
        (0..p.len() - 1).any(|i| {
            let (na, nb) = (nrm(tang[i]), nrm(tang[i + 1]));
            let a = [p[i][0] + na[0] * n, p[i][1] + na[1] * n];
            let b = [p[i + 1][0] + nb[0] * n, p[i + 1][1] + nb[1] * n];
            let t = [p[i + 1][0] - p[i][0], p[i + 1][1] - p[i][1]];
            (b[0] - a[0]) * t[0] + (b[1] - a[1]) * t[1] <= 0.0
        })
    })
}

/// The strips of every mapped beach.
pub fn build_strips(field: &ShoreField, beaches: &[Beach]) -> Vec<Strip> {
    if beaches.is_empty() {
        return vec![];
    }
    let mut strips = vec![];
    for chain in zero_chains(field) {
        if chain.len() < 4 {
            continue;
        }
        let pts = resample(&chain, SAMPLE_M);
        if pts.len() < 8 {
            continue;
        }
        let d: Vec<f64> = pts.iter().map(|p| beach_distance(*p, beaches)).collect();
        // maximal runs within EXTEND_M of a beach that touch one
        let mut i = 0;
        while i < pts.len() {
            if d[i] > EXTEND_M {
                i += 1;
                continue;
            }
            let start = i;
            while i < pts.len() && d[i] <= EXTEND_M {
                i += 1;
            }
            let run = start..i;
            let touches = d[run.clone()].iter().any(|&x| x <= 30.0);
            if !touches || run.len() < 20 {
                continue;
            }
            let raw: Vec<[f64; 2]> = pts[run.clone()].to_vec();
            // the open-Pacific share reaching this strip: its climate, and how far
            // out the sea hands over to it (the ring follows the share point by point)
            let share = raw.iter().map(|p| ocean_share(p[0])).fold(0.0, f64::max);
            if share <= PACIFIC_SHARE[0] {
                continue;
            }
            let mut climate = Climate::pacific();
            climate.n_max = RING_BAY_M[0] + (RING_PACIFIC_M[0] - RING_BAY_M[0]) * share + RING_MARGIN_M;
            let p = gauss_smooth(&raw, 10.0 / SAMPLE_M);
            let mut tang: Vec<[f64; 2]> = (0..p.len())
                .map(|k| {
                    let a = p[k.saturating_sub(1)];
                    let b = p[(k + 1).min(p.len() - 1)];
                    let t = [b[0] - a[0], b[1] - a[1]];
                    let l = (t[0] * t[0] + t[1] * t[1]).sqrt().max(1e-9);
                    [t[0] / l, t[1] / l]
                })
                .collect();
            // the normals bend slowly: the offsets out to n_max (and onto the sand) never fold
            let raw_tang = tang.clone();
            let mut sigma = (climate.n_max * 0.35).max(40.0);
            for _ in 0..12 {
                tang = gauss_smooth(&raw_tang, sigma / SAMPLE_M);
                if !folds(&p, &tang, &[N_LAND_M, climate.n_max]) {
                    break;
                }
                sigma *= 1.5;
            }
            let mut normals: Vec<[f64; 2]> = tang
                .iter()
                .map(|t| {
                    let l = (t[0] * t[0] + t[1] * t[1]).sqrt().max(1e-9);
                    [t[1] / l, -t[0] / l]
                })
                .collect();
            // offshore: toward the larger signed distance
            let mut votes = 0.0;
            for (q, nr) in p.iter().zip(&normals) {
                votes += field.signed_at(q[0] + nr[0] * 30.0, q[1] + nr[1] * 30.0) - field.signed_at(q[0] - nr[0] * 30.0, q[1] - nr[1] * 30.0);
            }
            if votes < 0.0 {
                for nr in &mut normals {
                    *nr = [-nr[0], -nr[1]];
                }
            }
            let dd = &d[run];
            let beach_w: Vec<f32> = dd.iter().map(|&x| (1.0 - smooth(30.0, EXTEND_M - 60.0, x)) as f32).collect();
            let tables = build_tables(&climate);
            strips.push(Strip { points: p, normals, beach_w, climate, tables });
        }
    }
    strips
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn travel_time_runs_shoreward_and_inverts() {
        for c in [Climate::pacific(), Climate::bay()] {
            let t = build_tables(&c);
            for w in t.along.windows(2) {
                assert!(w[0][1] > w[1][1], "travel time grows shoreward");
            }
            // the inverse returns the n the forward table gives
            for (i, a) in t.along.iter().enumerate().step_by(7) {
                let k = a[1] as f64 / TAU_STEP_S;
                let j = (k.floor() as usize).min(t.n_of_tau.len() - 2);
                let f = k - j as f64;
                let n = t.n_of_tau[j] as f64 * (1.0 - f) + t.n_of_tau[j + 1] as f64 * f;
                let want = t.n0 + i as f64 * t.step;
                assert!((n - want).abs() < 0.6, "n(tau(n)) {n} vs {want}");
            }
        }
    }

    #[test]
    fn waves_break_on_the_bars_and_shore() {
        let c = Climate::pacific();
        let t = build_tables(&c);
        let idx = |n: f64| ((n - t.n0) / t.step).round() as usize;
        // a 2 m incoming wave: unbroken at 400 m, breaking by the inner bar, H never over 0.78 h
        let lv = ((2.0 / c.hmax) * (LEVELS - 1) as f64).round() as usize;
        assert!(t.waves[idx(400.0) * LEVELS + lv][1] < 0.0);
        assert!(t.waves[idx(100.0) * LEVELS + lv][1] >= 0.0);
        for (i, a) in t.along.iter().enumerate() {
            let w = t.waves[i * LEVELS + lv];
            assert!(w[0] as f64 <= GAMMA_BREAK * a[0].max(0.0) as f64 + 1e-3);
        }
        // the largest level breaks on the outer bar
        let big = t.waves[idx(230.0) * LEVELS + LEVELS - 1];
        assert!(big[1] >= 0.0, "a 5 m wave breaks by the outer bar: {big:?}");
    }

    #[test]
    fn cover_hands_over_inside_the_field_cap() {
        let x = -12000.0;
        assert!((surf_cover(x, 100.0, 1.0) - 1.0).abs() < 1e-9);
        assert_eq!(surf_cover(x, 399.0, 1.0), 0.0);
        assert_eq!(surf_cover(x, 100.0, 0.5), 0.0);
        let mut prev = 1.0;
        for k in 0..100 {
            let c = surf_cover(x, 300.0 + k as f64, 1.0);
            assert!(c <= prev + 1e-12, "monotone across the ring");
            prev = c;
        }
        assert!(RING_PACIFIC_M[0] < crate::field::FIELD_CAP_M);
        assert!(RING_PACIFIC_M[0] + 20.0 <= Climate::pacific().n_max);
        assert_eq!(surf_cover(-4500.0, 50.0, 1.0), 0.0, "the bay keeps the sea's surf");
        assert!(RING_BAY_M[0] + 10.0 <= Climate::bay().n_max);
    }
}
