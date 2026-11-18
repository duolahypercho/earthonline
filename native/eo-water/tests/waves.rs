//! the sea state's spectrum and its whitecaps, on the CPU.
//!
//! - `spectrum_carries_each_place_s_sea`: the discrete spectrum (eo-water's
//!   twin of the GPU's) gives the open Pacific and the bay the significant
//!   height and peak period their sea states ask for, the swell travels from
//!   the west-north-west, and almost no energy runs against the wind.
//! - `crests_sharpen`: with the plan's choppiness the surface gathers toward
//!   the crests (the Jacobian is below 1 there), as a real sea's crests do.
//! - `whitecap_coverage_follows_the_wind`: the GPU's foam step run on the CPU
//!   (the cascades transformed with eo-water's Stockham schedule): the foam
//!   the Jacobian thresholds leave covers about the fraction the plan aims for
//!   (within a factor of two), and more in a stronger wind.

use eo_water::spectrum::{N, Spectrum, stockham_inverse};
use eo_water::waves::{self, CASCADES, Plan};

const TAU: f64 = std::f64::consts::TAU;

fn spectrum_for(state: &waves::SeaState) -> (Plan, Spectrum) {
    let plan = Plan::new(state);
    let sp = waves::build(&plan, &waves::gaussians());
    (plan, sp)
}

/// Variance per log-omega bin over the cascades (weighted by `amp`), and the
/// energy-weighted mean travel direction of modes in an omega range.
fn omega_bins(sp: &Spectrum, amp: &[f64; CASCADES]) -> Vec<(f64, f64)> {
    let bins = 120;
    let (lo, hi) = (0.2f64.ln(), 30f64.ln());
    let mut e = vec![0.0; bins];
    for (i, t) in sp.h0.iter().enumerate() {
        let w = sp.wave[i][3] as f64;
        if w <= 0.2 || w >= 30.0 {
            continue;
        }
        let a = amp[i / (N * N)];
        let v = a * a * t.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / 2.0;
        let b = (((w.ln() - lo) / (hi - lo)) * bins as f64) as usize;
        e[b.min(bins - 1)] += v;
    }
    (0..bins)
        .map(|b| {
            let w0 = (lo + (hi - lo) * b as f64 / bins as f64).exp();
            let w1 = (lo + (hi - lo) * (b + 1) as f64 / bins as f64).exp();
            ((w0 * w1).sqrt(), e[b] / (w1 - w0))
        })
        .collect()
}

/// Peak period (s) of the variance density, smoothed over five bins.
fn peak_period(bins: &[(f64, f64)], wmin: f64, wmax: f64) -> f64 {
    let mut best = (0.0, 0.0);
    for i in 2..bins.len() - 2 {
        let w = bins[i].0;
        if w < wmin || w > wmax {
            continue;
        }
        let s: f64 = bins[i - 2..=i + 2].iter().map(|b| b.1).sum();
        if s > best.1 {
            best = (w, s);
        }
    }
    TAU / best.0
}

#[test]
fn spectrum_carries_each_place_s_sea() {
    let st = waves::sea_state(16.0, [0.0, 0.0], 0.0);
    let (plan, sp) = spectrum_for(&st);
    // Significant heights of the discrete field, each place's amplitudes applied.
    let pac = waves::field_hs(&sp.h0, &plan.amp_pac);
    let want_pac = (st.swell.hs.powi(2) + st.pacific.hs().powi(2)).sqrt();
    let bay = waves::field_hs(&sp.h0, &plan.amp_bay);
    eprintln!("waves: clear afternoon Hs Pacific {pac:.2} m (want {want_pac:.2}), bay {bay:.2} m (want {:.2}); amp Pacific {:?} bay {:?}", st.bay.hs(), plan.amp_pac, plan.amp_bay);
    assert!((pac / want_pac - 1.0).abs() < 0.15, "Pacific Hs {pac} want {want_pac}");
    assert!((bay / st.bay.hs() - 1.0).abs() < 0.15, "bay Hs {bay} want {}", st.bay.hs());
    // Peak periods: the swell's on the Pacific, the bay's wind sea in the bay.
    let tp_pac = peak_period(&omega_bins(&sp, &plan.amp_pac), 0.2, 3.0);
    let tp_bay = peak_period(&omega_bins(&sp, &plan.amp_bay), 0.2, 30.0);
    eprintln!("waves: peak period Pacific {tp_pac:.1} s (swell {:.1}), bay {tp_bay:.1} s (want {:.1})", st.swell.tp, st.bay.tp());
    assert!((tp_pac / st.swell.tp - 1.0).abs() < 0.15, "Pacific peak {tp_pac} s");
    assert!((tp_bay / st.bay.tp() - 1.0).abs() < 0.15, "bay peak {tp_bay} s");
    // The swell travels toward the east-south-east (from the west-north-west), and the
    // energy running against its direction is small.
    let (mut along, mut against, mut cx, mut cz) = (0.0, 0.0, 0.0, 0.0);
    for i in 0..N * N {
        let w = sp.wave[i];
        if (w[3] as f64) > 0.6 || w[2] < 1e-6 {
            continue;
        }
        let h = sp.h0[i];
        let (kx, kz) = (w[0] as f64 / w[2] as f64, w[1] as f64 / w[2] as f64);
        let b = (h[2] as f64).powi(2) + (h[3] as f64).powi(2); // travels along +k
        let a = (h[0] as f64).powi(2) + (h[1] as f64).powi(2); // along -k
        cx += (b - a) * kx;
        cz += (b - a) * kz;
        let d = kx * waves::SWELL_DIR.cos() + kz * waves::SWELL_DIR.sin();
        if d > 0.0 { along += b; against += a } else { along += a; against += b }
    }
    let dir = cz.atan2(cx);
    eprintln!("waves: swell travels toward {:.1} deg (want {:.1}); against it {:.3} of its energy", dir.to_degrees(), waves::SWELL_DIR.to_degrees(), against / along);
    assert!((dir - waves::SWELL_DIR).abs() < 0.1, "swell direction {dir}");
    assert!(against / along < 0.05, "a standing swell: {against} against {along}");
    // A storm: the Pacific 3-6 m, the bay about 1 m.
    let s = waves::sea_state(15.0, [11.5, 4.2], 4.0);
    let (sp_plan, sp_s) = spectrum_for(&s);
    let pac_s = waves::field_hs(&sp_s.h0, &sp_plan.amp_pac);
    let bay_s = waves::field_hs(&sp_s.h0, &sp_plan.amp_bay);
    eprintln!("waves: storm Hs Pacific {pac_s:.2} m, bay {bay_s:.2} m");
    assert!((3.0..6.0).contains(&pac_s) && (0.8..1.4).contains(&bay_s));
    // A calm morning: the bay under 5 cm.
    let m = waves::sea_state(7.5, [0.0, 0.0], 0.0);
    let (mp, ms) = spectrum_for(&m);
    let bay_m = waves::field_hs(&ms.h0, &mp.amp_bay);
    eprintln!("waves: calm morning bay Hs {bay_m:.3} m");
    assert!(bay_m < 0.06);
}

/// The 2D inverse transform of one cascade's field coefficients (the GPU's
/// layout: texel (m, n) holds k = ((n - 128), (m - 128)) 2 pi / L), real part,
/// at the texels (row z, column x).
fn transform(coef: &[(f64, f64)]) -> Vec<f64> {
    let mut re = vec![0.0; N * N];
    let mut im = vec![0.0; N * N];
    for m in 0..N {
        let mut r: Vec<f64> = (0..N).map(|n| coef[m * N + n].0).collect();
        let mut i: Vec<f64> = (0..N).map(|n| coef[m * N + n].1).collect();
        stockham_inverse(&mut r, &mut i);
        for x in 0..N {
            re[m * N + x] = r[x];
            im[m * N + x] = i[x];
        }
    }
    let mut out = vec![0.0; N * N];
    for x in 0..N {
        let mut r: Vec<f64> = (0..N).map(|m| re[m * N + x]).collect();
        let mut i: Vec<f64> = (0..N).map(|m| im[m * N + x]).collect();
        stockham_inverse(&mut r, &mut i);
        for z in 0..N {
            let sign = if (x + z) % 2 == 0 { 1.0 } else { -1.0 };
            out[z * N + x] = r[z] * sign;
        }
    }
    out
}

/// The strains (dDx/dx, dDz/dz, dDx/dz) of cascade c at time t, times the choppiness.
fn strains(sp: &Spectrum, c: usize, t: f64, chop: f64) -> [Vec<f64>; 3] {
    let mut f = [vec![(0.0, 0.0); N * N], vec![(0.0, 0.0); N * N], vec![(0.0, 0.0); N * N]];
    for j in 0..N * N {
        let i = c * N * N + j;
        let w = sp.wave[i];
        let k = (w[2] as f64).max(1e-9);
        let (kx, kz) = (w[0] as f64, w[1] as f64);
        let (re, im) = sp.h_at(i, t);
        for (q, g) in [kx * kx / k, kz * kz / k, kx * kz / k].iter().enumerate() {
            f[q][j] = (re * g, im * g);
        }
    }
    f.map(|v| transform(&v).iter().map(|x| x * chop).collect())
}

#[test]
fn crests_sharpen() {
    // One mode travelling along +x: the height and the strain at its crest.
    let st = waves::sea_state(16.0, [0.0, 0.0], 0.0);
    let plan = Plan::new(&st);
    let mut sp = waves::build(&plan, &waves::gaussians());
    for h in sp.h0.iter_mut() {
        *h = [0.0; 4];
    }
    let c = 1;
    let (n, m) = (128 + 10, 128);
    sp.h0[c * N * N + m * N + n] = [0.0, 0.0, 0.5, 0.0];
    let s = strains(&sp, c, 0.0, plan.chop);
    let mut h = vec![(0.0, 0.0); N * N];
    let (re, im) = sp.h_at(c * N * N + m * N + n, 0.0);
    h[m * N + n] = (re, im);
    let mut hc = vec![(0.0, 0.0); N * N];
    hc.copy_from_slice(&h);
    let height = transform(&hc);
    let crest = (0..N).max_by(|a, b| height[*a].partial_cmp(&height[*b]).unwrap()).unwrap();
    let j_crest = 1.0 + s[0][crest];
    eprintln!("waves: one mode, crest at x texel {crest}: height {:.3}, Jacobian {:.3}", height[crest], j_crest);
    assert!(j_crest < 1.0, "the crest stretches ({j_crest}): the choppiness sign rounds the crests");
}

#[test]
fn whitecap_coverage_follows_the_wind() {
    let mut covers = vec![];
    for (name, state) in [("bay 15 kt", waves::sea_state(13.0, [0.0, 0.0], 0.0)), ("bay afternoon", waves::sea_state(16.0, [0.0, 0.0], 0.0)), ("storm", waves::sea_state(15.0, [11.5, 4.2], 4.0))] {
        let (plan, sp) = spectrum_for(&state);
        let place = 1; // the bay
        let amp = plan.amp_bay;
        let tau = plan.foam_tau[place];
        let dt = 0.25;
        let mut union_free = 1.0;
        let mut parts = vec![];
        for c in 0..waves::WHITECAP_CASCADES {
            let thr = plan.thr[place][c];
            if thr < -10.0 {
                parts.push((c, 0.0, 0.0, 0.0, 0.0));
                continue;
            }
            let soft = plan.soft[place][c];
            let mut foam = vec![0.0f64; N * N];
            let (mut cover, mut samples, mut below, mut sd) = (0.0, 0, 0.0, 0.0);
            let steps = 64;
            for step in 0..steps {
                let t = 20.0 - 16.0 + step as f64 * dt;
                let s = strains(&sp, c, t, plan.chop);
                let a = amp[c];
                let mut b = 0usize;
                let mut var = 0.0;
                // last step's foam carried downwind and spread along the wind, as water_fft.wgsl does
                let dir = if c == 0 { plan.state.pacific.dir } else { plan.systems[2].dir };
                let (wx, wz) = (dir.cos(), dir.sin());
                let u = if c == 0 { plan.state.pacific.u10 } else { plan.state.bay.u10.max(0.7 * plan.state.pacific.u10) };
                let carry = waves::FOAM_DRIFT * u * dt / (waves::SIZES[c] / N as f64);
                let kappa = (waves::FOAM_SPREAD * dt).clamp(0.0, 0.45);
                let at = |x: f64, y: f64| -> f64 {
                    let (fx, fy) = (x.floor(), y.floor());
                    let (tx, ty) = (x - fx, y - fy);
                    let w = |i: f64| (i as i64).rem_euclid(N as i64) as usize;
                    let (x0, y0, x1, y1) = (w(fx), w(fy), w(fx + 1.0), w(fy + 1.0));
                    let top = foam[y0 * N + x0] * (1.0 - tx) + foam[y0 * N + x1] * tx;
                    let bot = foam[y1 * N + x0] * (1.0 - tx) + foam[y1 * N + x1] * tx;
                    top * (1.0 - ty) + bot * ty
                };
                let carried: Vec<f64> = (0..N * N)
                    .map(|j| {
                        let (x, y) = ((j % N) as f64 - wx * carry, (j / N) as f64 - wz * carry);
                        let centre = at(x, y);
                        if kappa > 0.0 { centre + (0.5 * (at(x + wx, y + wz) + at(x - wx, y - wz)) - centre) * kappa } else { centre }
                    })
                    .collect();
                for j in 0..N * N {
                    let jac = (1.0 + a * s[0][j]) * (1.0 + a * s[1][j]) - a * a * s[2][j] * s[2][j];
                    var += (jac - 1.0).powi(2);
                    if jac < thr {
                        b += 1;
                    }
                    let inject = ((thr - jac) / soft).clamp(0.0, 1.0);
                    // the windrows water_fft.wgsl fades the foam between
                    let dir = if c == 0 { plan.state.pacific.dir } else { plan.systems[2].dir };
                    let (wx, wz) = (dir.cos(), dir.sin());
                    let cyc = [64.0, 40.0, 20.0, 8.0][c];
                    let (ax, az) = ((-wz * cyc).round(), (wx * cyc).round());
                    let (bx, bz) = ((wx * cyc * 0.21).round(), (wz * cyc * 0.21).round());
                    let (n, m) = ((j % N) as f64, (j / N) as f64);
                    let q = (n * ax + m * az) / N as f64;
                    let r = (n * bx + m * bz) / N as f64;
                    let row = 0.5 + 0.5 * (TAU * q + 1.3 * (TAU * r).sin()).sin();
                    let kept = carried[j] * (-dt / tau * (1.8 + (0.45 - 1.8) * row * row)).exp();
                    foam[j] = (kept + (1.0 - kept) * inject * (waves::FOAM_RATE * dt).min(1.0)).clamp(0.0, 1.0);
                }
                if step >= steps - 16 {
                    cover += foam.iter().filter(|f| **f > 0.3).count() as f64 / (N * N) as f64;
                    below += b as f64 / (N * N) as f64;
                    sd += (var / (N * N) as f64).sqrt();
                    samples += 1;
                }
            }
            let (cover, below, sd) = (cover / samples as f64, below / samples as f64, sd / samples as f64);
            let sigma = plan.chop.abs() * amp[c] * plan.mss_gpu[c].sqrt();
            parts.push((c, cover, below, sd, sigma));
            union_free *= 1.0 - cover;
        }
        let cover = 1.0 - union_free;
        eprintln!("waves: {name}: U10 {:.1} m/s, foam cover {:.4} (aim {:.4}); per cascade (c, cover, J<T, sd(J-1), sigma) {parts:.4?}", state.bay.u10, cover, plan.cover[1]);
        covers.push((cover, plan.cover[1]));
    }
    for (got, aim) in &covers {
        assert!(*got > aim * 0.5 && *got < aim * 2.0, "foam cover {got} against {aim}");
    }
    assert!(covers[2].0 > covers[1].0 && covers[1].0 > covers[0].0, "foam must grow with the wind {covers:?}");
}

/// A plan is rebuilt when the hour or the weather moves (off the render
/// thread outside captures): it must stay cheap.
#[test]
fn plan_build_is_cheap() {
    let states: Vec<waves::SeaState> = (0..24).map(|i| waves::sea_state(i as f64, [0.5 * i as f64, 0.1 * i as f64], (i % 5) as f64)).collect();
    let t = std::time::Instant::now();
    for st in &states {
        std::hint::black_box(Plan::new(st));
    }
    let ms = t.elapsed().as_secs_f64() * 1e3 / states.len() as f64;
    eprintln!("waves: a plan builds in {ms:.3} ms");
    assert!(ms < 2.0, "a plan takes {ms} ms");
}

/// (water r1) the whitecaps cover what the Monahan and O'Muircheartaigh fit
/// gives, the whole photographed cover, and no more: fewer crests break than
/// under the earlier reading (2.5 times the fit), every threshold lower.
#[test]
fn whitecaps_cover_the_fit_itself() {
    for (hour, wind, sev) in [(13.0, [0.0, 0.0], 0.0), (16.0, [0.0, 0.0], 0.0), (15.0, [11.5, 4.2], 4.0)] {
        let state = waves::sea_state(hour, wind, sev);
        let total = Plan::with_foam(&state, waves::FOAM_OVER_TOTAL);
        let earlier = Plan::new(&state);
        for place in 0..2 {
            let u = if place == 0 { state.pacific.u10 } else { state.bay.u10 };
            let fit = (waves::active_whitecaps(u) * if u > 4.5 { 1.0 } else { 0.0 }).min(0.45);
            if u > 4.5 {
                assert!((total.cover[place] - fit).abs() < 1e-12, "cover {} against the fit {fit} at {u} m/s", total.cover[place]);
            }
            assert!(total.cover[place] <= earlier.cover[place] + 1e-12);
            for c in 0..waves::WHITECAP_CASCADES {
                assert!(total.thr[place][c] <= earlier.thr[place][c] + 1e-12, "cascade {c} breaks more often under the fit than under the earlier reading");
            }
        }
    }
    // at the bay's 13 h breeze (about 6 m/s) the fit is a fifth of a percent
    let st = waves::sea_state(13.0, [0.0, 0.0], 0.0);
    let p = Plan::with_foam(&st, waves::FOAM_OVER_TOTAL);
    assert!(p.cover[1] > 0.001 && p.cover[1] < 0.003, "bay cover {}", p.cover[1]);
}
