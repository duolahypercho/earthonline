//! (`--weather damp` and `--look cinematic`, the cinematic look plan's
//! stage L3) The wet street's values and the CPU twin of its shading: the
//! damp street's drying patches, the look's darker and glossier film, the
//! lights' streaks on it (an anisotropic GGX lobe of a water film, long along
//! the ground toward the eye), the reflection's longer march, and car
//! paint's clearcoat. The shaders (eo-render sky_light.wgsl, sky_streaks.wgsl,
//! agents_draw.wgsl) carry the same numbers; eo-render tests/cine_wet.rs
//! keeps them equal.
//!
//! Sources: water's Fresnel F0 = ((1.333 - 1) / (1.333 + 1))^2 = 0.020; wet
//! rough materials about halve their albedo (Lekner & Dorf, "Why some things
//! are darker when wet", Appl. Opt. 27(7), 1988); a wet layer as a clear coat
//! over the darkened base (Jensen, Legakis & Dorsey, "Rendering of Wet
//! Materials", 1999); the anisotropic GGX and its height-correlated Smith
//! term (Heitz, "Understanding the Masking-Shadowing Function in
//! Microfacet-Based BRDFs", JCGT 3(2), 2014). The lobe's widths, the lens
//! floor and the damp street's patches are INFERRED (judged on the key views
//! against the reference sheet's wet-street descriptions), and named in
//! eo-look `CINEMATIC_DEVIATIONS` where they depart from the default.

/// (`--weather damp`) The least of the film left where a patch has dried.
pub const DAMP_DRY_FLOOR: f64 = 0.3;
/// (look) Wet albedo factor: asphalt, concrete (default 0.58, 0.66).
pub const WET_DARKEN: [f64; 2] = [0.45, 0.55];
/// (look) The wet film's roughness: asphalt, concrete (default 0.24, 0.32).
pub const WET_ROUGH: [f64; 2] = [0.17, 0.26];
/// Water's Fresnel reflectance at normal incidence.
pub const STREAK_F0: f64 = 0.02;
/// (look) The film's lobe (GGX alpha) along the ground toward the eye and across it.
pub const STREAK_ALPHA_ALONG: f64 = 0.14;
pub const STREAK_ALPHA_ACROSS: f64 = 0.02;
/// (look) A puddle's lobe: a near mirror (still water).
pub const PUDDLE_ALPHA: f64 = 0.006;
/// (look) A street luminaire's emission toward a streak point is never under
/// this share of its nadir intensity: the lens the eye sees glowing (the
/// sprite draws it at the full intensity), whose mirror image the film shows
/// even where the pool's cut-off sends no light.
pub const STREAK_LENS: f64 = 0.35;
/// (look) A headlamp's lens floor (ahead of the car only).
pub const STREAK_HEAD_LENS: f64 = 0.06;
/// (look) The reflection march: roughness it fades over, steps, first step (m) and growth, hit cap (x exposure).
pub const SSR_ROUGH: [f64; 2] = [0.3, 0.5];
pub const SSR_STEPS: u32 = 24;
pub const SSR_STEP: [f64; 2] = [0.15, 1.32];
pub const SSR_CAP: f64 = 24.0;
/// (look) Car paint's clearcoat roughness, clean and under full grime.
pub const COAT_ROUGH: f64 = 0.07;
pub const COAT_GRIME_ROUGH: f64 = 0.35;

type V3 = [f64; 3];

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn norm(a: V3) -> V3 {
    let l = dot(a, a).sqrt().max(1e-12);
    [a[0] / l, a[1] / l, a[2] / l]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// The film's anisotropic GGX times n.l (sky_light.wgsl `sky_cine_film`):
/// `at` along the tangent `t`, `ab` across.
pub fn film(n: V3, v: V3, l: V3, t: V3, at: f64, ab: f64) -> f64 {
    let nl = dot(n, l);
    let nv = dot(n, v).max(1e-4);
    if nl <= 0.0 {
        return 0.0;
    }
    let b = cross(n, t);
    let h = norm([l[0] + v[0], l[1] + v[1], l[2] + v[2]]);
    let th = dot(t, h) / at;
    let bh = dot(b, h) / ab;
    let nh = dot(n, h);
    let dd = th * th + bh * bh + nh * nh;
    let d = 1.0 / (std::f64::consts::PI * at * ab * dd * dd);
    let len = |x: f64, y: f64, z: f64| (x * x + y * y + z * z).sqrt();
    let gv = nl * len(at * dot(t, v), ab * dot(b, v), nv);
    let gl = nv * len(at * dot(t, l), ab * dot(b, l), nl);
    let vis = 0.5 / (gv + gl).max(1e-6);
    let f = STREAK_F0 + (1.0 - STREAK_F0) * (1.0 - dot(v, h).max(0.0)).powi(5);
    f * d * vis * nl
}

/// One light's streak radiance at a street point (sky_light.wgsl
/// `sky_cine_streaks`, one light): intensity `i` (units at 1 m) with
/// emission `shape` toward the point, its source radius `src` (m), the
/// point's wet share `wet` and puddle share `pud`.
#[allow(clippy::too_many_arguments)]
pub fn streak(world: V3, n: V3, v: V3, light: V3, i: f64, shape: f64, src: f64, wet: f64, pud: f64) -> f64 {
    if wet <= 0.0 {
        return 0.0;
    }
    let d = [light[0] - world[0], light[1] - world[1], light[2] - world[2]];
    let d2 = dot(d, d).max(0.04);
    let dist = d2.sqrt();
    let l = [d[0] / dist, d[1] / dist, d[2] / dist];
    let vn = dot(v, n);
    let t = norm([v[0] - n[0] * vn, v[1] - n[1] * vn, v[2] - n[2] * vn]);
    let a_along = STREAK_ALPHA_ALONG + (PUDDLE_ALPHA - STREAK_ALPHA_ALONG) * pud;
    let a_across = STREAK_ALPHA_ACROSS + (PUDDLE_ALPHA - STREAK_ALPHA_ACROSS) * pud;
    let wide = src / (2.0 * dist);
    let f = film(n, v, l, t, (a_along + wide).min(1.0), (a_across + wide).min(1.0));
    i * shape / d2 * f.min(d2 / (std::f64::consts::PI * src * src)) * wet
}

/// (`--weather damp`) The film's share left at a point whose patch field
/// (noise and drainage, sky_light.wgsl `sky_damp_left`) is `field`.
pub fn damp_left(field: f64) -> f64 {
    let t = ((field - 0.3) / (0.62 - 0.3)).clamp(0.0, 1.0);
    DAMP_DRY_FLOOR + (1.0 - DAMP_DRY_FLOOR) * t * t * (3.0 - 2.0 * t)
}

/// The coat's roughness for a paint roughened by grime by `d_rough`
/// (agents_draw.wgsl `ag_shade`).
pub fn coat_rough(d_rough: f64) -> f64 {
    let g = (d_rough / 0.4).clamp(0.0, 1.0);
    COAT_ROUGH + (COAT_GRIME_ROUGH - COAT_ROUGH) * g
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    /// The hemisphere integral of the film's BRDF x n.l for a view `v`
    /// (the share of light from that direction sent back: the albedo).
    fn albedo(v: V3, at: f64, ab: f64) -> f64 {
        let n = [0.0, 1.0, 0.0];
        let vn = dot(v, n);
        let t = norm([v[0] - n[0] * vn, v[1] - n[1] * vn, v[2] - n[2] * vn]);
        let (nt, np) = (600, 600);
        let mut sum = 0.0;
        for i in 0..nt {
            // cos theta uniform: d omega = d(cos) d phi
            let c = (i as f64 + 0.5) / nt as f64;
            let s = (1.0 - c * c).sqrt();
            for j in 0..np {
                let p = (j as f64 + 0.5) / np as f64 * 2.0 * PI;
                let l = [s * p.cos(), c, s * p.sin()];
                sum += film(n, v, l, t, at, ab);
            }
        }
        sum * (1.0 / nt as f64) * (2.0 * PI / np as f64)
    }

    #[test]
    fn the_film_lobe_never_returns_more_light_than_it_receives() {
        for elev in [60.0f64, 20.0, 8.0, 3.0] {
            let e = elev.to_radians();
            let v = [e.cos(), e.sin(), 0.0];
            for (at, ab) in [(STREAK_ALPHA_ALONG, STREAK_ALPHA_ACROSS), (0.3, 0.3), (0.5, 0.05)] {
                let a = albedo(v, at, ab);
                assert!(a <= 1.0 + 0.02, "albedo {a} at elevation {elev} alpha {at}/{ab}");
                assert!(a > 0.0);
            }
        }
    }

    #[test]
    fn a_far_lamp_draws_a_long_streak_toward_the_eye_and_none_on_dry_ground() {
        // eye 2.5 m up looking down +x; a lamp 150 m ahead, 9 m up (the hero card's far lamps)
        let eye = [0.0, 2.5, 0.0];
        let lamp = [150.0, 9.0, 0.0];
        let n = [0.0, 1.0, 0.0];
        let at = |x: f64, z: f64, wet: f64| {
            let p = [x, 0.0, z];
            let v = norm([eye[0] - p[0], eye[1] - p[1], eye[2] - p[2]]);
            streak(p, n, v, lamp, 2400.0, STREAK_LENS, 0.3, wet, 0.0)
        };
        // the mirror point: 150 x 2.5 / 11.5 = 32.6 m
        let mirror = at(32.6, 0.0, 1.0);
        assert!(mirror > 1.0, "the streak's peak {mirror} reads over a lit road (~1)");
        // long along the street: still a tenth of the peak 12 m nearer the eye, ...
        assert!(at(20.0, 0.0, 1.0) > 0.1 * mirror, "{} vs {mirror}", at(20.0, 0.0, 1.0));
        // ... narrow across it: a few metres aside it is gone
        assert!(at(32.6, 3.0, 1.0) < 0.05 * mirror, "{} vs {mirror}", at(32.6, 3.0, 1.0));
        // dry ground: nothing
        assert_eq!(at(32.6, 0.0, 0.0), 0.0);
        // a mirror's bound: never more than the source's luminance x Fresnel (<= 1)
        let lens = 2400.0 * STREAK_LENS / (PI * 0.3 * 0.3);
        for x in [5.0, 10.0, 20.0, 32.6, 60.0, 120.0, 149.0] {
            assert!(at(x, 0.0, 1.0) <= lens, "{x}");
        }
        // a puddle is a sharper mirror: brighter at the mirror point, shorter
        let p = [32.6, 0.0, 0.0];
        let v = norm([eye[0] - p[0], eye[1] - p[1], eye[2] - p[2]]);
        let pud = streak(p, n, v, lamp, 2400.0, STREAK_LENS, 0.3, 1.0, 1.0);
        assert!(pud > mirror, "puddle {pud} vs film {mirror}");
    }

    #[test]
    fn damp_patches_and_coat_stay_in_range() {
        assert_eq!(damp_left(-1.0), DAMP_DRY_FLOOR);
        assert_eq!(damp_left(2.0), 1.0);
        assert!(damp_left(0.46) > DAMP_DRY_FLOOR && damp_left(0.46) < 1.0);
        assert_eq!(coat_rough(0.0), COAT_ROUGH);
        assert_eq!(coat_rough(1.0), COAT_GRIME_ROUGH);
        assert!((0.05..=0.1).contains(&COAT_ROUGH), "automotive clearcoat");
        assert!(WET_DARKEN[0] < 0.58 && WET_DARKEN[1] < 0.66, "darker than the default");
        assert!((STREAK_F0 - ((1.333f64 - 1.0) / (1.333 + 1.0)).powi(2)).abs() < 1e-3);
    }
}
