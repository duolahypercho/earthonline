//! Fog lit by the city's own lights (the renderer's `--lit-fog` froxel
//! volume): how fog droplets scatter a lamp's light toward the eye, as pure
//! functions with their sources, and the constants the GPU pass is built
//! from (`wgsl`).
//!
//! Sources (the plan's reference sheet, section 4):
//! - Extinction from visibility: Koschmieder, sigma = 3.912 / V (WMO-No. 8,
//!   CIMO Guide 2018, Vol. I Ch. 9, eqs 9.4-9.6).
//! - Fog droplets scatter strongly forward: Henyey-Greenstein g ~ 0.9 for
//!   fog (~0.75 haze, ~0.95 rain). A lamp's glow depends only on the optical
//!   thickness T = sigma R to it and on g, and fog glows are narrower than
//!   haze glows (Narasimhan and Nayar, "Shedding Light on the Weather", CVPR
//!   2003, Figs 5 and 7, eq. 9). Their glow is a series over the orders of
//!   scattering whose k-th term spreads as g^k; for Henyey-Greenstein lobes
//!   that is exact: k scatterings of HG(g) in a row are HG(g^k).
//! - Worked example: at V = 200 m a lamp 30 m away has T ~ 0.6 (a crisp
//!   source in a tight halo), at 100 m T ~ 2 (a wide glow), at 400 m T ~ 8
//!   (the glow alone).
//!
//! The pass is single scattering at each point of the fog, with the light's
//! earlier and later scatterings folded in the way those sources describe:
//! the light reaching a point from a lamp keeps travelling forward when a
//! droplet scatters it (it is spread in angle, not removed), so it is
//! attenuated over its path by the similarity-reduced extinction
//! sigma (1 - g) and leaves the point in the lobe of its expected order of
//! scattering over the whole path lamp -> point -> eye (`glow_phase`). A near
//! lamp in thin fog keeps the droplets' tight forward halo; a far one, or
//! thick fog, a wide glow.

/// Henyey-Greenstein asymmetry of fog droplets (Narasimhan and Nayar 2003,
/// Figs 5 and 7: fog ~ 0.9).
pub const DROPLET_G: f64 = 0.9;

/// Koschmieder's constant (2 % contrast threshold): sigma = 3.912 / V
/// (WMO-No. 8, 2018, Vol. I Ch. 9, eqs 9.4-9.6).
pub const KOSCHMIEDER: f64 = 3.912;

/// Single-scattering albedo of fog droplets at visible wavelengths (~1.0:
/// water absorbs almost nothing there; reference sheet section 4.2, recalled).
pub const DROPLET_ALBEDO: f64 = 1.0;

/// A street luminaire's light in the fog, by the vertical angle from its
/// nadir: San Francisco's cobra heads have Type II/III optics on poles about
/// three mounting heights apart (SFPUC Streetlight Standards Rev. 4, 2023,
/// p. 23 section 6), so their candela peaks well out along the street, and a
/// full-cutoff luminaire sends nothing at or above the horizontal (IES
/// cutoff classification). The batwing the surfaces use (1 + 1.5 sin^2 of the
/// angle) up to `LUMINAIRE_FULL_DEG`, then falling to zero at 90 deg as the
/// cosine's ratio to its value there to the power `LUMINAIRE_TAIL_POWER`.
/// INFERRED: the peak angle (72 deg, inside a medium throw's 66-75 deg) and
/// the tail's power (about 0.43 x the nadir at 80 deg) are not measured
/// values. The surfaces keep their pool's 75 deg cut-off (the next pole
/// lights the ground beyond it); the fog takes the tail, which is what a
/// lamp seen down the street sends toward the eye: its glow in fog.
pub const LUMINAIRE_FULL_DEG: f64 = 72.0;
pub const LUMINAIRE_TAIL_POWER: f64 = 3.0;

/// The street luminaire's relative intensity (1 at the nadir) at a direction
/// whose cosine from the nadir is `cos_nadir`.
pub fn luminaire_shape(cos_nadir: f64) -> f64 {
    let c = cos_nadir.clamp(-1.0, 1.0);
    if c <= 0.0 {
        return 0.0;
    }
    let tail = (c / LUMINAIRE_FULL_DEG.to_radians().cos()).min(1.0).powf(LUMINAIRE_TAIL_POWER);
    (1.0 + 1.5 * (1.0 - c * c)) * tail
}

/// Henyey-Greenstein phase function (per steradian, integrates to 1 over the
/// sphere). `cos` is the cosine of the angle between the light's direction of
/// travel and the direction it leaves in (1: straight on).
pub fn hg(cos: f64, g: f64) -> f64 {
    let g2 = g * g;
    (1.0 - g2) / (4.0 * std::f64::consts::PI * (1.0 + g2 - 2.0 * g * cos).max(1e-12).powf(1.5))
}

/// The lobe the light leaves a point in after an optical depth `tau` of fog
/// on its whole path (lamp to the point and on to the eye): with Poisson
/// odds of having been scattered 0, 1 or more times before, the droplets'
/// own lobe HG(g), twice HG(g^2), three or more HG(g^3). Integrates to 1.
pub fn glow_phase(cos: f64, tau: f64) -> f64 {
    let tau = tau.max(0.0);
    let e = (-tau).exp();
    let g = DROPLET_G;
    e * hg(cos, g) + e * tau * hg(cos, g * g) + (1.0 - e * (1.0 + tau)).max(0.0) * hg(cos, g * g * g)
}

/// The share of a lamp's light still travelling out from it after `tau` of
/// fog on its way: the similarity relation's reduced extinction
/// sigma (1 - g) (forward-scattered light stays in the beam).
pub fn spread_transmittance(tau: f64) -> f64 {
    (-tau.max(0.0) * (1.0 - DROPLET_G)).exp()
}

/// Extinction coefficient (1/m) of air with meteorological visibility `v_m`.
pub fn sigma_for_visibility(v_m: f64) -> f64 {
    KOSCHMIEDER / v_m.max(1e-6)
}

/// Meteorological visibility (m) of air with extinction `sigma` (1/m).
pub fn visibility_for_sigma(sigma: f64) -> f64 {
    KOSCHMIEDER / sigma.max(1e-12)
}

/// The optical thickness to a lamp `r_m` away (Narasimhan and Nayar's T).
pub fn glow_thickness(sigma: f64, r_m: f64) -> f64 {
    sigma * r_m
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Radiance reaching `eye` along the unit ray `dir` from the first `dist`
/// metres of a uniform fog of extinction `sigma` (1/m, albedo
/// `DROPLET_ALBEDO`), lit by an isotropic point lamp of `intensity` (cd, or
/// the renderer's units per steradian) at `lamp`, by the model above: the
/// integral over the ray of sigma e^(-sigma t) I T_spread(sigma d) / d^2
/// glow_phase(cos, sigma (d + t)). Midpoint rule over `steps` samples
/// crowded round the lamp's closest approach (the reference the GPU pass is
/// checked against). Distances under `min_d` are held at it (a lamp head is
/// not a point).
pub fn point_glow(eye: [f64; 3], dir: [f64; 3], dist: f64, lamp: [f64; 3], intensity: f64, sigma: f64, min_d: f64, steps: usize) -> f64 {
    if sigma <= 0.0 || intensity <= 0.0 || dist <= 0.0 {
        return 0.0;
    }
    let steps = steps.max(16);
    // the closest approach along the ray: the samples crowd round it (t = c + b sinh(u)).
    let c = dot(sub(lamp, eye), dir).clamp(0.0, dist);
    let b = {
        let p = [eye[0] + dir[0] * c, eye[1] + dir[1] * c, eye[2] + dir[2] * c];
        let d = sub(lamp, p);
        dot(d, d).sqrt().max(min_d)
    };
    let (u0, u1) = (((0.0 - c) / b).asinh(), ((dist - c) / b).asinh());
    let du = (u1 - u0) / steps as f64;
    let mut acc = 0.0;
    for k in 0..steps {
        let u = u0 + (k as f64 + 0.5) * du;
        let t = c + b * u.sinh();
        let dt = b * u.cosh() * du;
        let p = [eye[0] + dir[0] * t, eye[1] + dir[1] * t, eye[2] + dir[2] * t];
        let lp = sub(p, lamp);
        let d = dot(lp, lp).sqrt();
        let d2 = (d * d).max(min_d * min_d);
        // light travelling from the lamp through p, leaving toward the eye (-dir).
        let cos = -dot(lp, dir) / d.max(1e-12);
        acc += sigma * DROPLET_ALBEDO * (-sigma * t).exp() * intensity * spread_transmittance(sigma * d) / d2 * glow_phase(cos, sigma * (d + t)) * dt;
    }
    acc
}

/// The constants the renderer's lit-fog pass is built from (WGSL).
pub fn wgsl() -> String {
    format!(
        "\n// generated by eo-sky lit_fog::wgsl\nconst LF_G: f32 = {:.6};\nconst LF_ALBEDO: f32 = {:.6};\nconst LF_LUMINAIRE_COS_FULL: f32 = {:.6};\nconst LF_LUMINAIRE_TAIL_POWER: f32 = {:.6};\n",
        DROPLET_G,
        DROPLET_ALBEDO,
        LUMINAIRE_FULL_DEG.to_radians().cos(),
        LUMINAIRE_TAIL_POWER
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (integral over the sphere, mean cosine) of a phase, by the cosine (midpoint rule).
    fn moments(p: impl Fn(f64) -> f64) -> (f64, f64) {
        let n = 200_000;
        let (mut e, mut m) = (0.0, 0.0);
        for k in 0..n {
            let c = -1.0 + (k as f64 + 0.5) * 2.0 / n as f64;
            let w = p(c) * 2.0 * std::f64::consts::PI * 2.0 / n as f64;
            e += w;
            m += w * c;
        }
        (e, m / e)
    }

    #[test]
    fn the_phases_are_normalised_and_forward() {
        for g in [0.0, 0.35, 0.75, 0.9] {
            let (e, m) = moments(|c| hg(c, g));
            assert!((e - 1.0).abs() < 1e-3, "HG({g}) integrates to {e}");
            assert!((m - g).abs() < 2e-3, "HG({g}) mean cosine {m}");
        }
        // straight on, a fog droplet sends ~2,400 x what it sends sideways (the halo's peak).
        assert!(hg(1.0, DROPLET_G) / hg(0.0, DROPLET_G) > 1000.0);
        for tau in [0.0, 0.3, 1.0, 3.0, 10.0] {
            let (e, m) = moments(|c| glow_phase(c, tau));
            assert!((e - 1.0).abs() < 2e-3, "the glow's lobe after tau {tau} integrates to {e}");
            // its mean cosine is the expected g^(k+1) over the orders: forward, falling with tau.
            let x = (-tau as f64).exp();
            let want = x * DROPLET_G + x * tau * DROPLET_G.powi(2) + (1.0 - x * (1.0 + tau)) * DROPLET_G.powi(3);
            assert!((m - want).abs() < 3e-3, "tau {tau}: mean cosine {m}, want {want}");
        }
        // thin fog: the droplets' own lobe; thick fog: broader (more light sideways, less straight on).
        assert!((glow_phase(1.0, 0.0) - hg(1.0, DROPLET_G)).abs() < 1e-9);
        assert!(glow_phase(0.0, 3.0) > 2.0 * glow_phase(0.0, 0.0));
        assert!(glow_phase(1.0, 3.0) < 0.5 * glow_phase(1.0, 0.0));
        // forward-scattered light stays in the beam: the reduced extinction.
        assert!((spread_transmittance(2.0) - (-0.2f64).exp()).abs() < 1e-12);
    }

    #[test]
    fn koschmieder_and_the_worked_example() {
        // the sheet's table: V = 50 / 100 / 200 / 400 m / 1 km -> 0.078 / 0.039 / 0.0196 / 0.0098 / 0.0039.
        for (v, s) in [(50.0, 0.078), (100.0, 0.039), (200.0, 0.0196), (400.0, 0.0098), (1000.0, 0.0039)] {
            assert!((sigma_for_visibility(v) - s).abs() < 0.0005, "V {v}: {}", sigma_for_visibility(v));
            assert!((visibility_for_sigma(sigma_for_visibility(v)) - v).abs() < 1e-9);
        }
        // V = 200 m: T 0.6 at 30 m, 2 at 100 m, 8 at 400 m.
        let s = sigma_for_visibility(200.0);
        for (r, t) in [(30.0, 0.59), (100.0, 1.96), (400.0, 7.82)] {
            assert!((glow_thickness(s, r) - t).abs() < 0.01, "T at {r} m: {}", glow_thickness(s, r));
        }
    }

    /// The glow of a lamp `r` m ahead seen `b` m off it, in fog of visibility `v`.
    fn glow_off(v: f64, r: f64, b: f64) -> f64 {
        let d = [b, 0.0, r];
        let l = dot(d, d).sqrt();
        point_glow([0.0; 3], [d[0] / l, 0.0, d[2] / l], 6.0 * r, [0.0, 0.0, r], 1000.0, sigma_for_visibility(v), 0.1, 8192)
    }

    #[test]
    fn a_lamp_glows_round_itself_and_fades_over_metres() {
        // a lamp 30 m ahead in V = 200 m fog, rays passing it at 0.25 .. 8 m.
        let v: Vec<f64> = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0].iter().map(|b| glow_off(200.0, 30.0, *b)).collect();
        assert!(v.windows(2).all(|w| w[1] < w[0]), "the glow does not fall away from the lamp: {v:?}");
        // fades over several metres: 2 m out still a few percent of its 0.25 m value, 8 m out under 5 %.
        assert!(v[3] > 0.02 * v[0] && v[5] < 0.05 * v[0], "{v:?}");
        // linear in the lamp; none without fog.
        let lamp = [0.0, 0.0, 30.0];
        let s = sigma_for_visibility(200.0);
        let one = point_glow([0.0; 3], [0.0, 0.0, 1.0], 200.0, lamp, 1.0, s, 0.1, 4096);
        let two = point_glow([0.0; 3], [0.0, 0.0, 1.0], 200.0, lamp, 2.0, s, 0.1, 4096);
        assert!((two / one - 2.0).abs() < 1e-9);
        assert_eq!(point_glow([0.0; 3], [0.0, 0.0, 1.0], 200.0, lamp, 1.0, 0.0, 0.1, 4096), 0.0);
        // a ray that stops short of the lamp sees less of it than one that passes it.
        let short = point_glow([0.0; 3], [0.0, 0.0, 1.0], 20.0, lamp, 1.0, s, 0.1, 4096);
        assert!(short < one && short > 0.0);
    }

    #[test]
    fn the_glow_widens_with_the_optical_thickness() {
        // Narasimhan and Nayar: the glow's shape follows T = sigma R. The glow 2 deg off a lamp
        // over the glow 0.2 deg off it, for T 0.15 (a near lamp in thin fog) .. 8 (glow only).
        let width = |v: f64, r: f64| {
            let a = |deg: f64| glow_off(v, r, r * (deg as f64).to_radians().tan());
            a(2.0) / a(0.2)
        };
        let ts = [(1000.0, 40.0), (200.0, 30.0), (200.0, 100.0), (200.0, 400.0)];
        let w: Vec<f64> = ts.iter().map(|(v, r)| width(*v, *r)).collect();
        assert!(w.windows(2).all(|p| p[1] > p[0]), "the glow does not widen with T: {w:?}");
    }

    #[test]
    fn a_street_luminaire_peaks_out_along_the_street_and_cuts_off_at_the_horizontal() {
        assert!((luminaire_shape(1.0) - 1.0).abs() < 1e-12, "1 at the nadir");
        let at = |deg: f64| luminaire_shape(deg.to_radians().cos());
        // the peak inside a medium throw's 66-75 deg, nothing at or above the horizontal.
        let peak = (0..=900).map(|k| k as f64 * 0.1).max_by(|a, b| at(*a).total_cmp(&at(*b))).unwrap();
        assert!((66.0..=75.0).contains(&peak), "peak at {peak} deg");
        assert!(at(90.0) < 1e-12, "{}", at(90.0));
        assert_eq!(at(120.0), 0.0);
        // falling through its tail: under half the nadir at 80 deg, a tenth by 86 deg.
        assert!(at(80.0) < 0.5 && at(80.0) > at(83.0) && at(86.0) < 0.1, "{} {} {}", at(80.0), at(83.0), at(86.0));
        // the batwing the surfaces use inside it.
        assert!((at(60.0) - (1.0 + 1.5 * 0.75)).abs() < 1e-9);
    }

    #[test]
    fn the_shader_constants_are_the_models() {
        let s = wgsl();
        assert!(s.contains("const LF_G: f32 = 0.900000;"), "{s}");
        assert!(s.contains("const LF_ALBEDO: f32 = 1.000000;"), "{s}");
    }
}
