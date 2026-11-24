//! (`--lamp-bounce`) A street lamp's pool lights what stands round it: the
//! street under a lamp reflects part of the lamp's light back up, so the
//! walls, the trim, the kerbs, the car sides and the undersides near a pool
//! take a warm bounce that falls away from it, and the gaps between the
//! poles do not. Pure functions; the renderer's light gather (eo-render
//! sky_lights.wgsl `lt_bounce`) adds one light for each street lamp's pool.
//!
//! The model: one bounce of instant radiosity (Keller 1997, "Instant
//! radiosity", SIGGRAPH: a virtual point light where a light's paths first
//! land). The lamp's flux onto the street is its nadir intensity over the
//! surfaces' own distribution (the batwing the surfaces shade with, to the
//! full cut-off: `flux_over_nadir`); the street returns `GROUND_REFLECTANCE`
//! of it as a Lambertian emitter, whose intensity is rho Phi / pi along the
//! vertical and cos theta of that off it. The pool is a disc, not a point:
//! the emitter stands `depth_over_height` mounting heights under the pool's
//! centre (the radius holding half the flux), which makes the irradiance at
//! the pool's level exactly the disc's on its axis (E = M R^2 / (R^2 + z^2)
//! at z = 0; Modest, "Radiative Heat Transfer", the disc-to-point view
//! factor) and the far field the point's. The ground itself takes none (it
//! faces away from a source under it); a wall takes it by its cosine.
//!
//! Unshadowed: the bounce is a broad, low source and the surfaces' own
//! occlusion multiplies it (light_shadow.rs `ls_pads`: no map, -1).

use crate::lights::{STREET_CUTOFF_DEG, STREET_PEAK_DEG};

/// The diffuse reflectance of the street under a lamp: a dry road surface
/// of the CIE's R3 class has an average luminance coefficient q0 of 0.07
/// cd m^-2 lx^-1 (CIE 144:2001, "Road surface and road marking reflection
/// characteristics", Table 2), a diffuse-equivalent reflectance of
/// pi q0 = 0.22 when new; worn dark asphalt falls toward 0.1 and a
/// concrete sidewalk stands at 0.25-0.35. INFERRED: 0.15 for an aged San
/// Francisco street with its sidewalks (the owner's weathered city).
pub const GROUND_REFLECTANCE: f64 = 0.15;

/// The emitter's reach in pool radii (its light falls under ~1 % of the
/// pool's own level by then: the far field's 1 / r^2 x the cosine).
pub const RANGE_OVER_RADIUS: f64 = 4.0;

/// The surfaces' street luminaire distribution (sky_light.wgsl kind 0):
/// a batwing 1 + 1.5 sin^2 theta from the nadir, cut off by a smoothstep
/// from `STREET_PEAK_DEG` to `STREET_CUTOFF_DEG`. 1 at the nadir.
pub fn luminaire_shape(theta_rad: f64) -> f64 {
    let c = theta_rad.cos();
    let (lo, hi) = (STREET_CUTOFF_DEG.to_radians().cos(), STREET_PEAK_DEG.to_radians().cos());
    let t = ((c - lo) / (hi - lo)).clamp(0.0, 1.0);
    let s = theta_rad.sin();
    (1.0 + 1.5 * s * s) * t * t * (3.0 - 2.0 * t)
}

/// The flux (lm) a lamp of nadir intensity 1 cd sends below its cut-off:
/// 2 pi times the integral of `luminaire_shape` x sin theta (midpoint rule).
/// Returns (the flux, the angle from the nadir holding half of it, rad).
fn flux_and_half_angle() -> (f64, f64) {
    let n = 20_000;
    let top = STREET_CUTOFF_DEG.to_radians();
    let d = top / n as f64;
    let mut cum = Vec::with_capacity(n);
    let mut acc = 0.0;
    for k in 0..n {
        let th = (k as f64 + 0.5) * d;
        acc += 2.0 * std::f64::consts::PI * luminaire_shape(th) * th.sin() * d;
        cum.push(acc);
    }
    let half = cum.iter().position(|&c| c >= 0.5 * acc).unwrap_or(n - 1);
    (acc, (half as f64 + 0.5) * d)
}

/// The lamp's flux onto the street over its nadir intensity (lm / cd, sr).
pub fn flux_over_nadir() -> f64 {
    flux_and_half_angle().0
}

/// The pool's radius holding half the flux, in mounting heights: tan of
/// the angle from the nadir that splits the flux.
pub fn radius_over_height() -> f64 {
    flux_and_half_angle().1.tan()
}

/// How far under the pool's centre the emitter stands, in mounting heights
/// (the pool's half-flux radius: the disc's irradiance on its axis at the
/// pool's level).
pub fn depth_over_height() -> f64 {
    radius_over_height()
}

/// The emitter's intensity along the vertical over the lamp's nadir
/// intensity: rho x the flux over pi (a Lambertian emitter's I(0) = Phi / pi).
pub fn intensity_over_nadir() -> f64 {
    GROUND_REFLECTANCE * flux_over_nadir() / std::f64::consts::PI
}

/// The bounce's irradiance (lux, for a lamp of nadir intensity `i_nadir`
/// cd mounted `h` m up) at a point `x` m from the pole's foot along the
/// ground and `y` m up, on a surface whose unit normal is `n` (y up, x away
/// from the pole): the emitter's I(0) cos theta_s cos theta_r / r^2.
pub fn irradiance(i_nadir: f64, h: f64, x: f64, y: f64, n: [f64; 2]) -> f64 {
    let d = depth_over_height() * h;
    // from the receiver toward the emitter
    let to = [-x, -(y + d)];
    let r2 = to[0] * to[0] + to[1] * to[1];
    let r = r2.sqrt();
    let cos_s = (y + d) / r;
    let cos_r = (n[0] * to[0] + n[1] * to[1]) / r;
    if cos_s <= 0.0 || cos_r <= 0.0 {
        return 0.0;
    }
    intensity_over_nadir() * i_nadir * cos_s * cos_r / r2
}

/// The constants the light gather is built with (sky_lights.wgsl `lt_bounce`).
pub fn wgsl() -> String {
    format!(
        "\n// generated by eo-sky lamp_bounce::wgsl\nconst LT_BOUNCE_I_OVER_NADIR: f32 = {:.6};\nconst LT_BOUNCE_DEPTH_MH: f32 = {:.6};\nconst LT_BOUNCE_RANGE_MH: f32 = {:.6};\n",
        intensity_over_nadir(),
        depth_over_height(),
        RANGE_OVER_RADIUS * radius_over_height()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flux_is_the_batwings_and_half_of_it_lands_inside_the_peak() {
        // a uniform 1 cd over the cap to 75 deg would send 2 pi (1 - cos 75) = 4.66 lm; the batwing more.
        let f = flux_over_nadir();
        let cap = 2.0 * std::f64::consts::PI * (1.0 - STREET_CUTOFF_DEG.to_radians().cos());
        assert!(f > cap && f < 2.5 * cap, "flux {f} lm/cd (cap {cap})");
        // half the flux inside the peak's 60 deg: the pool's radius under 1.7 heights, over 1.
        let r = radius_over_height();
        assert!(r > 1.0 && r < STREET_PEAK_DEG.to_radians().tan(), "R/h {r}");
        // the emitter: rho Phi / pi of the nadir
        assert!((intensity_over_nadir() - GROUND_REFLECTANCE * f / std::f64::consts::PI).abs() < 1e-12);
        assert!(intensity_over_nadir() > 0.2 && intensity_over_nadir() < 0.6, "{}", intensity_over_nadir());
    }

    #[test]
    fn a_wall_by_the_pool_takes_about_a_lux_falling_away_and_the_ground_none() {
        // an 8.84 m arterial head at the RP-8 peak's 30 lux nadir.
        let h = crate::lights::ARTERIAL_HEAD_M;
        let i = crate::photometry::LAMP_POOL_PEAK_LUX * h * h;
        let wall = |x: f64, y: f64| irradiance(i, h, x, y, [-1.0, 0.0]);
        // a facade 6 m from the pole (the sidewalk's back), 1 m up: about a lux
        let e = wall(6.0, 1.0);
        assert!(e > 0.5 && e < 3.0, "wall at 6 m: {e} lux");
        // less higher up and further out (the bounce stays at the wall's foot)
        assert!(wall(6.0, 8.0) < e && wall(20.0, 1.0) < 0.5 * e, "{} {}", wall(6.0, 8.0), wall(20.0, 1.0));
        // the ground faces away from it; a wall facing away takes none
        assert_eq!(irradiance(i, h, 6.0, 0.0, [0.0, 1.0]), 0.0);
        assert_eq!(irradiance(i, h, 6.0, 1.0, [1.0, 0.0]), 0.0);
        // an underside over the pool (an awning, a car's sill) takes it face on
        assert!(irradiance(i, h, 2.0, 3.0, [0.0, -1.0]) > e);
        // just over the pool's centre, facing down: the street's exitance there (rho x the nadir's
        // lux, the whole hemisphere below lit) within 1.5 x
        let m = GROUND_REFLECTANCE * crate::photometry::LAMP_POOL_PEAK_LUX;
        let up = irradiance(i, h, 0.0, 0.0, [0.0, -1.0]);
        assert!(up > m / 1.5 && up < 1.5 * m, "on the axis {up}, exitance {m}");
    }

    #[test]
    fn the_shader_constants_are_the_models() {
        let s = wgsl();
        assert!(s.contains(&format!("const LT_BOUNCE_I_OVER_NADIR: f32 = {:.6};", intensity_over_nadir())), "{s}");
        assert!(s.contains("const LT_BOUNCE_DEPTH_MH: f32 = "), "{s}");
    }
}
