//! The one-dimensional profile solve along a chain's stations.
//!
//! Everything here works in grade coordinates: `tau[i]` is the sum of
//! `grade limit x station step` up to station `i`, so "rises or falls no
//! faster than the grade limit" is "1-Lipschitz in tau". A deck's profile is
//! the lowest such function on or above its lower bound that meets its fixed
//! end heights (`lowest_above` plus the end cones), with its crests rounded
//! and its sags eased by short vertical curves (`deck`); a tunnel's is the
//! mirror image (`tunnel`). Pure functions, no state.

/// End heights a profile must meet; None is a free end.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Ends {
    pub a: Option<f64>,
    pub b: Option<f64>,
}

/// Slack for floating point in the checks (metres).
const EPS: f64 = 1e-9;

/// The lowest 1-Lipschitz function (in `tau`) on or above `lo` (two sweeps
/// of the distance transform; `lo` may hold minus infinity where nothing
/// bounds it).
pub(crate) fn lowest_above(tau: &[f64], lo: &[f64]) -> Vec<f64> {
    let n = lo.len();
    let mut y = lo.to_vec();
    for i in 1..n {
        y[i] = y[i].max(y[i - 1] - (tau[i] - tau[i - 1]));
    }
    for i in (0..n.saturating_sub(1)).rev() {
        y[i] = y[i].max(y[i + 1] - (tau[i + 1] - tau[i]));
    }
    y
}

/// The highest 1-Lipschitz function (in `tau`) on or below `hi`.
pub(crate) fn highest_below(tau: &[f64], hi: &[f64]) -> Vec<f64> {
    let neg: Vec<f64> = hi.iter().map(|v| -v).collect();
    lowest_above(tau, &neg).into_iter().map(|v| -v).collect()
}

/// `y = max(y, the cones falling away from each fixed end)`: the least a
/// 1-Lipschitz profile through the end heights can be.
pub(crate) fn lower_cones(y: &mut [f64], tau: &[f64], ends: Ends) {
    let n = y.len();
    let t_end = tau[n - 1];
    for i in 0..n {
        if let Some(h) = ends.a {
            y[i] = y[i].max(h - tau[i]);
        }
        if let Some(h) = ends.b {
            y[i] = y[i].max(h - (t_end - tau[i]));
        }
    }
}

/// `y = min(y, the cones rising away from each fixed end)`: the most a
/// 1-Lipschitz profile through the end heights can be.
pub(crate) fn upper_cones(y: &mut [f64], tau: &[f64], ends: Ends) {
    let n = y.len();
    let t_end = tau[n - 1];
    for i in 0..n {
        if let Some(h) = ends.a {
            y[i] = y[i].min(h + tau[i]);
        }
        if let Some(h) = ends.b {
            y[i] = y[i].min(h + (t_end - tau[i]));
        }
    }
}

/// Crests rounded: the sup-convolution of `y` with the kernel `c(d) = d^2 /
/// (2 kappa)` for `d < kappa` and `d - kappa / 2` beyond. Each term is
/// 1-Lipschitz, so the result is; it is never below `y` (d = 0), and a crest
/// whose grade turns from +1 to -1 becomes a parabola `2 kappa` long in tau.
pub(crate) fn round_crests(y: &[f64], tau: &[f64], kappa: f64) -> Vec<f64> {
    let n = y.len();
    if n < 3 || kappa <= 0.0 {
        return y.to_vec();
    }
    // prefix max of y + tau (the far left terms), suffix max of y - tau (far right).
    let mut pm = vec![f64::NEG_INFINITY; n];
    let mut sm = vec![f64::NEG_INFINITY; n];
    for i in 0..n {
        let v = y[i] + tau[i];
        pm[i] = if i == 0 { v } else { pm[i - 1].max(v) };
    }
    for i in (0..n).rev() {
        let v = y[i] - tau[i];
        sm[i] = if i + 1 == n { v } else { sm[i + 1].max(v) };
    }
    let mut out = y.to_vec();
    for i in 0..n {
        let mut best = y[i];
        let mut j = i;
        loop {
            if j == 0 {
                break;
            }
            j -= 1;
            let d = tau[i] - tau[j];
            if d >= kappa {
                best = best.max(pm[j] - tau[i] + kappa / 2.0);
                break;
            }
            best = best.max(y[j] - d * d / (2.0 * kappa));
        }
        let mut j = i;
        while j + 1 < n {
            j += 1;
            let d = tau[j] - tau[i];
            if d >= kappa {
                best = best.max(sm[j] + tau[i] + kappa / 2.0);
                break;
            }
            best = best.max(y[j] - d * d / (2.0 * kappa));
        }
        out[i] = best;
    }
    out
}

/// Sags eased: `max(y, its moving average)` over a window of half-width `w`
/// (tau) that shrinks to stay inside the chain, so the ends keep their
/// heights. The average of a 1-Lipschitz function over such a window is
/// 1-Lipschitz, and so is the max.
pub(crate) fn smooth_sags(y: &[f64], tau: &[f64], w: f64) -> Vec<f64> {
    let n = y.len();
    if n < 3 || w <= 0.0 {
        return y.to_vec();
    }
    let mut cum = vec![0.0; n];
    for i in 1..n {
        cum[i] = cum[i - 1] + (tau[i] - tau[i - 1]) * (y[i] + y[i - 1]) / 2.0;
    }
    let integral = |x: f64| -> f64 {
        let k = match tau.binary_search_by(|t| t.total_cmp(&x)) {
            Ok(k) => return cum[k],
            Err(0) => return cum[0],
            Err(k) if k >= n => return cum[n - 1],
            Err(k) => k - 1,
        };
        let t = (x - tau[k]) / (tau[k + 1] - tau[k]);
        let yx = y[k] + (y[k + 1] - y[k]) * t;
        cum[k] + (x - tau[k]) * (y[k] + yx) / 2.0
    };
    (0..n)
        .map(|i| {
            let h = w.min(tau[i] - tau[0]).min(tau[n - 1] - tau[i]);
            if h <= 1e-12 { y[i] } else { y[i].max((integral(tau[i] + h) - integral(tau[i] - h)) / (2.0 * h)) }
        })
        .collect()
}

/// Every step within the grade limit (1 in tau).
pub(crate) fn lipschitz_ok(y: &[f64], tau: &[f64]) -> bool {
    y.windows(2).zip(tau.windows(2)).all(|(v, t)| (v[1] - v[0]).abs() <= (t[1] - t[0]) * (1.0 + 1e-9) + EPS)
}

fn ends_ok(y: &[f64], ends: Ends) -> bool {
    ends.a.is_none_or(|h| (y[0] - h).abs() <= 1e-6) && ends.b.is_none_or(|h| (y[y.len() - 1] - h).abs() <= 1e-6)
}

/// The sharp deck profile: the lowest 1-Lipschitz function on or above `lo`
/// that is at least each fixed end's falling cone. When the ends are
/// feasible (the caller checks) it meets them exactly.
pub(crate) fn deck_sharp(tau: &[f64], lo: &[f64], ends: Ends) -> Vec<f64> {
    let mut y = lowest_above(tau, lo);
    lower_cones(&mut y, tau, ends);
    y
}

/// The deck profile with vertical curves: the sharp profile, crests rounded
/// (kernel `kappa`), held under the fixed ends' rising cones so the ends stay
/// put, sags eased over the same length; then re-checked (never under `lo`,
/// within the grade, ends met), falling back to the less smoothed profile if
/// a check fails. `feasible` false (an end the sharp profile cannot meet)
/// returns the sharp profile untouched.
pub(crate) fn deck(tau: &[f64], lo: &[f64], ends: Ends, kappa: f64, feasible: bool) -> Vec<f64> {
    let sharp = deck_sharp(tau, lo, ends);
    if !feasible || kappa <= 0.0 || sharp.len() < 3 {
        return sharp;
    }
    let mut rounded = round_crests(&sharp, tau, kappa);
    upper_cones(&mut rounded, tau, ends);
    let eased = smooth_sags(&rounded, tau, kappa);
    let ok = |y: &[f64]| y.iter().zip(lo).all(|(v, l)| *v >= l - EPS) && lipschitz_ok(y, tau) && ends_ok(y, ends);
    for cand in [eased, rounded] {
        if ok(&cand) {
            return cand;
        }
    }
    sharp
}

/// A tunnel's profile: the highest 1-Lipschitz function under `hi`, except
/// near each fixed portal, where it is held up by the cone falling from the
/// portal into the hill (the open cut and portal zone, where no cover is
/// asked), and never above the portals' rising cones; smoothed as the deck's
/// mirror image. Feasible when the portals are within the grade of each
/// other (the caller checks).
pub(crate) fn tunnel(tau: &[f64], hi: &[f64], ends: Ends, kappa: f64, feasible: bool) -> Vec<f64> {
    let mut y = highest_below(tau, hi);
    lower_cones(&mut y, tau, ends);
    upper_cones(&mut y, tau, ends);
    if !feasible || kappa <= 0.0 || y.len() < 3 {
        return y;
    }
    // the deck smoothing on the mirror image only lowers the profile.
    let neg: Vec<f64> = y.iter().map(|v| -v).collect();
    let mends = Ends { a: ends.a.map(|h| -h), b: ends.b.map(|h| -h) };
    let mut rounded = round_crests(&neg, tau, kappa);
    upper_cones(&mut rounded, tau, mends);
    let eased = smooth_sags(&rounded, tau, kappa);
    let ok = |m: &[f64]| m.iter().zip(&neg).all(|(v, l)| *v >= l - EPS) && lipschitz_ok(m, tau) && ends_ok(m, mends);
    for cand in [eased, rounded] {
        if ok(&cand) {
            return cand.into_iter().map(|v| -v).collect();
        }
    }
    y
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tau_of(n: usize, step: f64) -> Vec<f64> {
        (0..n).map(|i| i as f64 * step).collect()
    }

    #[test]
    fn lowest_above_is_the_cone_envelope() {
        let tau = tau_of(11, 1.0);
        let mut lo = vec![0.0; 11];
        lo[5] = 3.0;
        let y = lowest_above(&tau, &lo);
        assert_eq!(y, vec![0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 2.0, 1.0, 0.0, 0.0, 0.0]);
        assert!(lipschitz_ok(&y, &tau));
    }

    #[test]
    fn smoothing_keeps_bounds_grade_and_ends() {
        let tau = tau_of(101, 0.16);
        let mut lo: Vec<f64> = (0..101).map(|i| (i as f64 * 0.3).sin() * 0.2).collect();
        for v in lo.iter_mut().skip(40).take(10) {
            *v = 4.0;
        }
        let ends = Ends { a: Some(0.0), b: Some(1.0) };
        let y = deck(&tau, &lo, ends, 0.6, true);
        assert!(lipschitz_ok(&y, &tau));
        assert!(y.iter().zip(&lo).all(|(v, l)| *v >= l - 1e-9));
        assert!((y[0] - 0.0).abs() < 1e-9 && (y[100] - 1.0).abs() < 1e-9);
        let sharp = deck_sharp(&tau, &lo, ends);
        assert!(y.iter().zip(&sharp).all(|(v, s)| *v >= s - 1e-9));
        // the crest is rounded: the largest change of grade between steps is
        // smaller than the sharp profile's (2, from +1 to -1).
        let turn = |p: &[f64]| p.windows(3).map(|w| ((w[2] - w[1]) - (w[1] - w[0])).abs() / 0.16).fold(0.0, f64::max);
        assert!(turn(&y) < turn(&sharp) - 0.5, "{} vs {}", turn(&y), turn(&sharp));
    }

    #[test]
    fn tunnel_stays_under_its_bound_past_the_portal_zones() {
        let tau = tau_of(201, 0.24);
        let hi: Vec<f64> = (0..201).map(|i| 30.0 - ((i as f64 - 100.0) / 100.0).powi(2) * 40.0).collect();
        let ends = Ends { a: Some(0.0), b: Some(0.0) };
        let y = tunnel(&tau, &hi, ends, 0.6, true);
        assert!(lipschitz_ok(&y, &tau));
        assert!((y[0]).abs() < 1e-9 && (y[200]).abs() < 1e-9);
        let sharp = {
            let mut s = highest_below(&tau, &hi);
            lower_cones(&mut s, &tau, ends);
            upper_cones(&mut s, &tau, ends);
            s
        };
        assert!(y.iter().zip(&sharp).all(|(v, s)| *v <= s + 1e-9));
        assert!(y[100] <= hi[100] + 1e-9);
    }
}
