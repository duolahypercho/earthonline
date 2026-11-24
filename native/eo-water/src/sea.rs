//! The sea surface as the drawn water carries it, for the physics (what
//! floats on it: eo-player's boats). Deterministic: the same sea state plan,
//! world time and place give the same surface on every machine (plain f64
//! arithmetic in a fixed order and `libm`'s trigonometry, not the
//! platform's), so the server's world and each client's float the same boats.
//!
//! The drawn sea is the sum of the GPU's four FFT cascades, each place's
//! share of each cascade (water_common.wgsl `wg_cascade_amp`, [`Places`]:
//! the exposure field's bay wind sea and swell share where the water has
//! one, else the bay's and the open Pacific's blended across the Golden
//! Gate) and the shore's (water.wgsl `w_wave_ramp`). Each cascade is the sum of its modes, the
//! CPU twin of the GPU's spectrum (`waves::build`, the same numbers;
//! eo-render tests/water.rs `fft_matches_the_cpu_sum` holds the GPU's
//! transform to that sum).
//!
//! [`SeaField::grid`] is that transform on the CPU for cascades 0 to 2 (waves
//! over 1.1 m; the ripples of cascade 3 move no hull): every mode of each,
//! on the GPU's 256 grids (`GRIDS`), the height, its two slopes and the two
//! horizontal displacements. A grid is the whole sea at one time (the cascades tile the
//! world), so any number of points sample it at the cost of a lookup;
//! [`SeaGrid::sample`] reads it bilinearly, as the drawn sea reads the GPU's.
//!
//! Heights are metres above the still sea (`SEA_LEVEL_Y`), slopes are
//! dh/dx and dh/dz, and `shift` is the drawn surface's horizontal
//! displacement (choppy waves gather toward their crests): a world point's
//! height is the field's where the displacement came from, to first order
//! h(p) - grad h . shift(p).

use crate::waves::{self, CASCADES, Plan, SIZES};

/// The cascades that move a hull (cascade 3, the ripples under 1.1 m, does not).
pub const HULL_CASCADES: usize = 3;
/// The CPU grids' sides per cascade: the GPU's (`spectrum::N`), so a bilinear read of
/// one is the drawn sea's read of the GPU's (a half grid would read a 7 m wave at two
/// texels a wave and miss a twentieth of a calm sea between its points).
pub const GRIDS: [usize; HULL_CASCADES] = [crate::spectrum::N; HULL_CASCADES];

/// The surface's motion at a point now.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sample {
    /// Height above the still sea (m).
    pub height: f64,
    /// dh/dx, dh/dz.
    pub slope: [f64; 2],
    /// The drawn surface's horizontal displacement here (m).
    pub shift: [f64; 2],
}

impl Sample {
    /// `a` to `b` by `f` (a sample between two grids' times).
    pub fn lerp(a: Sample, b: Sample, f: f64) -> Sample {
        let l = |x: f64, y: f64| x + (y - x) * f;
        Sample { height: l(a.height, b.height), slope: [l(a.slope[0], b.slope[0]), l(a.slope[1], b.slope[1])], shift: [l(a.shift[0], b.shift[0]), l(a.shift[1], b.shift[1])] }
    }
}

/// The shore's hold on the waves at a point: the shore field's signed
/// distance (m, positive over water) and beach weight (`ShoreField`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Shore {
    pub signed_m: f64,
    pub beach: f64,
}

impl Shore {
    pub fn of(field: &crate::field::ShoreField, x: f64, z: f64) -> Self {
        Self { signed_m: field.signed_at(x, z), beach: field.beach_at(x, z) }
    }
}

/// How much of the sea's waves the shore lets reach a point (water.wgsl
/// `w_wave_ramp`): none within 50 m of the shore, all past 220 m; at a
/// beach a quarter comes ashore.
pub fn shore_gain(s: Shore) -> f64 {
    let smooth = |e0: f64, e1: f64, x: f64| {
        let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    smooth(50.0, 220.0, s.signed_m) * (1.0 - 0.6 * s.beach) + 0.25 * s.beach * smooth(0.0, 60.0, s.signed_m)
}

/// Each place's amplitude of each cascade, as the drawn sea sets it
/// (water.wgsl `w_expo_set`, water_common.wgsl `wg_cascade_amp`): where the
/// exposure field is installed and on (`exposure`, `--no-water-exposure`)
/// the bay's wind sea as the plan tabulates it at the place's wind ratio and
/// fetch factor (`Plan::expo_amp`, bilinear between the nodes) and the
/// Pacific's share its swell share, that share taken over by the measured
/// bed's where it covers (`bathymetry`, `--no-bathymetry`); without the
/// field the plan's bay and Pacific blended by the ramp across the Gate.
#[derive(Clone, Debug)]
pub struct Places {
    amp_bay: [f64; CASCADES],
    amp_pac: [f64; CASCADES],
    expo_amp: [[[f64; CASCADES]; waves::EXPO_FETCH_NODES.len()]; waves::EXPO_WIND_NODES.len()],
    exposure: Option<&'static crate::exposure::Exposure>,
    bathymetry: Option<&'static crate::extra::ExtraField>,
}

impl Places {
    /// The plan's places with the fields the water draws with: those this
    /// process installed (`crate::load`) and the command line keeps on.
    pub fn drawn(plan: &Plan) -> Self {
        let exposure = crate::exposure::installed().filter(|_| crate::switches::is_on(crate::switches::EXPOSURE));
        let bathymetry = crate::extra::installed().filter(|f| crate::switches::is_on(crate::switches::BATHYMETRY) && f.width > 0 && f.height > 0 && f.texels.len() == f.width * f.height);
        Self::with(plan, exposure, bathymetry)
    }

    /// The plan's places over the given fields (None: as without them).
    pub fn with(plan: &Plan, exposure: Option<&'static crate::exposure::Exposure>, bathymetry: Option<&'static crate::extra::ExtraField>) -> Self {
        Self { amp_bay: plan.amp_bay, amp_pac: plan.amp_pac, expo_amp: plan.expo_amp, exposure, bathymetry }
    }

    /// Per cascade, the drawn sea's amplitude at a point (before the shore's gain).
    pub fn amp(&self, x: f64, z: f64) -> [f64; CASCADES] {
        let blend = |bay: [f64; CASCADES], o: f64| std::array::from_fn(|c| bay[c] + (self.amp_pac[c] - bay[c]) * o);
        let Some(e) = self.exposure else {
            return blend(self.amp_bay, crate::surf_strip::ocean_share(x));
        };
        // wg_ex: the swell share, the wind ratio (G x 2), the fetch factor
        let ex = e.at(x, z);
        let mut share = ex[0];
        if let Some((on, kr)) = self.bathymetry.and_then(|f| bed_share(f, x, z)) {
            share += (kr.clamp(0.0, 1.0) - share) * on;
        }
        let (g, gf) = node(ex[1], &waves::EXPO_WIND_NODES);
        let (b, bf) = node(ex[2], &waves::EXPO_FETCH_NODES);
        let xa = &self.expo_amp;
        let bay = std::array::from_fn(|c| {
            let lo = xa[g][b][c] + (xa[g][b + 1][c] - xa[g][b][c]) * bf;
            let hi = xa[g + 1][b][c] + (xa[g + 1][b + 1][c] - xa[g + 1][b][c]) * bf;
            lo + (hi - lo) * gf
        });
        blend(bay, share)
    }
}

/// Where `v` falls among eight ascending nodes: the lower node and the
/// fraction to the next (water_common.wgsl `wg_node`).
fn node(v: f64, n: &[f64; 8]) -> (usize, f64) {
    let x = v.clamp(n[0], n[7]);
    let i = (1..7).filter(|&k| x > n[k]).last().unwrap_or(0);
    (i, ((x - n[i]) / (n[i + 1] - n[i]).max(1e-6)).clamp(0.0, 1.0))
}

/// The measured bed's hold here and the swell share it carries (water_common.wgsl
/// `wb_set` and `wb_layer`: layer 0's G, read half a texel inside the layer, the
/// hold rising over 200 m from the field's edge); None outside the field.
fn bed_share(f: &crate::extra::ExtraField, x: f64, z: f64) -> Option<(f64, f64)> {
    let fr = f.frame();
    let (u, v) = ((x - fr[0]) * fr[2], (z - fr[1]) * fr[3]);
    if u <= 0.0 || v <= 0.0 || u >= 1.0 || v >= 1.0 {
        return None;
    }
    let ext = [1.0 / fr[2], 1.0 / fr[3]];
    let edge = (u.min(1.0 - u) * ext[0]).min(v.min(1.0 - v) * ext[1]);
    let t = (edge / 200.0).clamp(0.0, 1.0);
    let on = t * t * (3.0 - 2.0 * t);
    let (w, h) = (f.width, f.height);
    let hh = 0.5 / h as f64;
    let tv = (v * 0.5).clamp(hh, 0.5 - hh);
    // texel centres at (i + 0.5) / size, clamped at the edges, as the GPU's linear sampler
    let fx = (u * w as f64 - 0.5).clamp(0.0, (w - 1) as f64);
    let fy = (tv * h as f64 - 0.5).clamp(0.0, (h - 1) as f64);
    let (i0, j0) = (fx.floor() as usize, fy.floor() as usize);
    let (i1, j1) = ((i0 + 1).min(w - 1), (j0 + 1).min(h - 1));
    let (tx, ty) = (fx - i0 as f64, fy - j0 as f64);
    let g = |i: usize, j: usize| crate::bathymetry::half_to_f32(f.texels[j * w + i][1]) as f64;
    let kr = (g(i0, j0) * (1.0 - tx) + g(i1, j0) * tx) * (1.0 - ty) + (g(i0, j1) * (1.0 - tx) + g(i1, j1) * tx) * ty;
    Some((on, kr))
}

#[derive(Clone, Copy, Debug)]
struct Mode {
    /// the texel in its cascade's grid (row-major, centred: index side / 2 is k = 0).
    at: usize,
    kx: f64,
    kz: f64,
    k: f64,
    omega: f64,
    /// h0's +k draw: h(t) = (a + i b) e^{-i omega t}.
    a: f64,
    b: f64,
}

/// One plan's sea: the modes of cascades 0..3 laid on the CPU grid.
#[derive(Clone, Debug)]
pub struct SeaField {
    modes: [Vec<Mode>; HULL_CASCADES],
    places: std::sync::Arc<Places>,
    chop: f64,
    /// e^{+2 pi i j / side} per cascade
    twiddle: [Vec<[f64; 2]>; HULL_CASCADES],
}

/// The sea at one time: per cascade, the five fields on the grid.
#[derive(Clone, Debug)]
pub struct SeaGrid {
    pub t: f64,
    /// per cascade, per texel: height, dh/dx, dh/dz, dx, dz (displacements with the choppiness)
    cells: [Vec<[f64; 5]>; HULL_CASCADES],
    places: std::sync::Arc<Places>,
}

impl SeaField {
    /// From a plan: the CPU twin of its spectrum (`waves::build`), cascades
    /// 0..3, over the places the water draws (`Places::drawn`).
    pub fn new(plan: &Plan) -> Self {
        Self::from_spectrum(plan, &waves::build(plan, &waves::gaussians()), Places::drawn(plan))
    }

    pub fn from_spectrum(plan: &Plan, sp: &crate::spectrum::Spectrum, places: Places) -> Self {
        let n = crate::spectrum::N;
        let nh = n / 2;
        let mut modes: [Vec<Mode>; HULL_CASCADES] = Default::default();
        for (c, list) in modes.iter_mut().enumerate() {
            for m in 0..n {
                for k in 0..n {
                    let i = c * n * n + m * n + k;
                    let (h, w) = (sp.h0[i], sp.wave[i]);
                    if h == [0.0; 4] {
                        continue;
                    }
                    // every mode lies within its cascade's grid, the first draw is 0 (each mode runs along +k).
                    let (side, half) = (GRIDS[c], GRIDS[c] as i64 / 2);
                    let (dm, dk) = (m as i64 - nh as i64, k as i64 - nh as i64);
                    assert!(dm.abs() < half && dk.abs() < half, "sea: cascade {c} mode ({dk}, {dm}) past the CPU grid");
                    assert!(h[0] == 0.0 && h[1] == 0.0, "sea: a mode with a -k draw");
                    let at = (dm + half) as usize * side + (dk + half) as usize;
                    list.push(Mode { at, kx: w[0] as f64, kz: w[1] as f64, k: w[2] as f64, omega: w[3] as f64, a: h[2] as f64, b: h[3] as f64 });
                }
            }
        }
        let twiddle = GRIDS.map(|side| {
            (0..side)
                .map(|j| {
                    let (s, c) = libm::sincos(std::f64::consts::TAU * j as f64 / side as f64);
                    [c, s]
                })
                .collect()
        });
        Self { modes, places: std::sync::Arc::new(places), chop: plan.chop, twiddle }
    }

    /// Modes per cascade.
    pub fn modes(&self) -> [usize; HULL_CASCADES] {
        [self.modes[0].len(), self.modes[1].len(), self.modes[2].len()]
    }

    /// The sea at world time `t` (s, the water's clock).
    pub fn grid(&self, t: f64) -> SeaGrid {
        let mut cells: [Vec<[f64; 5]>; HULL_CASCADES] = Default::default();
        for (c, out) in cells.iter_mut().enumerate() {
            let side = GRIDS[c];
            let n2 = side * side;
            // The five fields' spectra: F_h = h, F_dhdx = i kx h, F_dhdz = i kz h,
            // F_dx = -i (kx / k) chop h, F_dz = -i (kz / k) chop h (water_fft.wgsl `fields_at`).
            let mut f: Vec<[[f64; 2]; 5]> = vec![[[0.0; 2]; 5]; n2];
            for md in &self.modes[c] {
                let (s, co) = libm::sincos(md.omega * t);
                let h = [md.a * co + md.b * s, -md.a * s + md.b * co];
                let i_h = [-h[1], h[0]];
                let (ux, uz) = (md.kx / md.k * self.chop, md.kz / md.k * self.chop);
                f[md.at] = [h, [md.kx * i_h[0], md.kx * i_h[1]], [md.kz * i_h[0], md.kz * i_h[1]], [-ux * i_h[0], -ux * i_h[1]], [-uz * i_h[0], -uz * i_h[1]]];
            }
            // Each real field is Re of its transform; packed two to a transform by their
            // Hermitian parts H(k) = (F(k) + conj F(-k)) / 2 (as water_fft.wgsl `packed`).
            let herm = |q: usize, at: usize| -> [f64; 2] {
                let (m, k) = (at / side, at % side);
                let mirror = ((side - m) % side) * side + (side - k) % side;
                let (p, r) = (f[at][q], f[mirror][q]);
                [(p[0] + r[0]) * 0.5, (p[1] - r[1]) * 0.5]
            };
            let pack = |q0: usize, q1: Option<usize>| -> Vec<[f64; 2]> {
                (0..n2)
                    .map(|at| {
                        let a = herm(q0, at);
                        let b = q1.map_or([0.0; 2], |q| herm(q, at));
                        // a + i b
                        [a[0] - b[1], a[1] + b[0]]
                    })
                    .collect()
            };
            let mut p0 = pack(0, Some(1));
            let mut p1 = pack(2, Some(3));
            let mut p2 = pack(4, None);
            for p in [&mut p0, &mut p1, &mut p2] {
                Self::inverse_2d(&self.twiddle[c], p);
            }
            *out = (0..n2)
                .map(|at| {
                    // the centred spectrum's shift: (-1)^(m + n)
                    let sgn = if ((at / side) + (at % side)) % 2 == 0 { 1.0 } else { -1.0 };
                    [p0[at][0] * sgn, p0[at][1] * sgn, p1[at][0] * sgn, p1[at][1] * sgn, p2[at][0] * sgn]
                })
                .collect();
        }
        SeaGrid { t, cells, places: self.places.clone() }
    }

    /// Inverse DFT along rows then columns (unnormalised, e^{+2 pi i}).
    fn inverse_2d(twiddle: &[[f64; 2]], d: &mut [[f64; 2]]) {
        let side = twiddle.len();
        let mut line = vec![[0.0; 2]; side];
        let mut scratch = vec![[0.0; 2]; side];
        for r in 0..side {
            line.copy_from_slice(&d[r * side..(r + 1) * side]);
            Self::inverse_1d(twiddle, &mut line, &mut scratch);
            d[r * side..(r + 1) * side].copy_from_slice(&line);
        }
        for col in 0..side {
            for r in 0..side {
                line[r] = d[r * side + col];
            }
            Self::inverse_1d(twiddle, &mut line, &mut scratch);
            for r in 0..side {
                d[r * side + col] = line[r];
            }
        }
    }

    /// Radix-2 Stockham in the order `spectrum::stockham_inverse` and the GPU use;
    /// the result in `a`.
    fn inverse_1d(twiddle: &[[f64; 2]], a: &mut [[f64; 2]], b: &mut [[f64; 2]]) {
        let side = twiddle.len();
        let stages = side.trailing_zeros() as usize;
        let mut ns = 1;
        for stage in 0..stages {
            let (src, dst) = if stage % 2 == 0 { (&*a, &mut *b) } else { (&*b, &mut *a) };
            Self::stage(twiddle, ns, src, dst);
            ns *= 2;
        }
        if stages % 2 == 1 {
            a.copy_from_slice(b);
        }
    }

    fn stage(twiddle: &[[f64; 2]], ns: usize, src: &[[f64; 2]], dst: &mut [[f64; 2]]) {
        let side = twiddle.len();
        let half = side / 2;
        for j in 0..half {
            let k = j % ns;
            let w = twiddle[k * (side / (2 * ns))];
            let (x, y) = (src[j], src[j + half]);
            let t = [y[0] * w[0] - y[1] * w[1], y[0] * w[1] + y[1] * w[0]];
            let d = (j / ns) * ns * 2 + k;
            dst[d] = [x[0] + t[0], x[1] + t[1]];
            dst[d + ns] = [x[0] - t[0], x[1] - t[1]];
        }
    }
}

impl SeaGrid {
    /// One cascade's five fields at a world point (bilinear, the tile repeating).
    fn read(&self, c: usize, x: f64, z: f64) -> [f64; 5] {
        let (s, side) = (SIZES[c], GRIDS[c]);
        let (u, v) = (x / s * side as f64, z / s * side as f64);
        let (fu, fv) = (u.floor(), v.floor());
        let (tu, tv) = (u - fu, v - fv);
        let wrap = |i: f64| (i as i64).rem_euclid(side as i64) as usize;
        let (i0, j0) = (wrap(fu), wrap(fv));
        let (i1, j1) = ((i0 + 1) % side, (j0 + 1) % side);
        let cell = &self.cells[c];
        let (a, b, cc, d) = (cell[j0 * side + i0], cell[j0 * side + i1], cell[j1 * side + i0], cell[j1 * side + i1]);
        let mut o = [0.0; 5];
        for q in 0..5 {
            o[q] = (a[q] * (1.0 - tu) + b[q] * tu) * (1.0 - tv) + (cc[q] * (1.0 - tu) + d[q] * tu) * tv;
        }
        o
    }

    /// The places this grid's sea is drawn over.
    pub fn places(&self) -> &Places {
        &self.places
    }

    /// The field at its own point (no displacement correction), with the place's and the shore's gains.
    pub fn sample_field_point(&self, x: f64, z: f64, shore: Shore) -> Sample {
        let gain = shore_gain(shore);
        let mut s = Sample::default();
        if gain <= 0.0 {
            return s;
        }
        let amp = self.places.amp(x, z);
        for c in 0..HULL_CASCADES {
            let g = amp[c] * gain;
            if g == 0.0 {
                continue;
            }
            let f = self.read(c, x, z);
            s.height += g * f[0];
            s.slope[0] += g * f[1];
            s.slope[1] += g * f[2];
            s.shift[0] += g * f[3];
            s.shift[1] += g * f[4];
        }
        s
    }

    /// The drawn surface at a world point: the field brought there by its
    /// displacement, to first order (h - grad h . shift).
    pub fn sample(&self, x: f64, z: f64, shore: Shore) -> Sample {
        let mut s = self.sample_field_point(x, z, shore);
        s.height -= s.slope[0] * s.shift[0] + s.slope[1] * s.shift[1];
        s
    }
}
