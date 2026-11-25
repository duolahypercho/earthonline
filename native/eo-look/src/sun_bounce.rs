//! The sun bounce (eo-render `--sun-bounce`, surfaces_ao_sun.wgsl): the
//! sun's first bounce off the sunlit walls round a point, gathered from the
//! far sun shadow map as a reflective shadow map (Dachsbacher and
//! Stamminger, "Reflective Shadow Maps", I3D 2005).
//!
//! Each texel of an orthographic sun depth map is the first surface the sun
//! meets in a beam of the texel's cross-section `a` (square metres across
//! the beam). Whatever its orientation, that surface takes the key's
//! irradiance on a plane facing the sun, `E_n`, over `a`; a Lambertian
//! surface of albedo `rho` has the radiance `rho E_n cos_s / pi` (cos_s
//! between its normal and the sun) over its area `a / cos_s`, so a point P
//! with normal n at a distance d receives from it
//!
//! ```text
//!     dE = rho E_n a cos_x cos_p / (pi d^2)
//! ```
//!
//! (cos_x at the surface toward P, cos_p at P toward the surface).
//!
//! What the rig already stands for is left as it is. Its ground lobe stands
//! for the sunlit street below (and the ambient volume's ground bounce puts
//! the sunlit ground's own exitance in its place where it is baked), and with
//! the volume the buildings that hide the sky are lit at the ground lobe's
//! radiance (`hemi_ground / pi`, ambient.wgsl `AMB_OCCLUDER`). What no term
//! has is a sunlit wall: a facade across the street from a sunlit one, a
//! canyon's lower floors under its sunlit upper floors, a column beside a
//! sunlit wall. So the gather takes walls only (the ground and roofs, normals
//! within `GROUND_COS` of up, are the ground bounce's), and each sunlit wall
//! sample adds its radiance less the stand-in's in its direction:
//!
//! ```text
//!     dE = (rho E_n - hemi_ground / cos_s) a cos_x cos_p / (pi d^2),  at least 0
//! ```
//!
//! The colour is the key's (warm at a low sun) times the walls' albedo. The
//! map holds no visibility between P and the wall; the cosines and the
//! falloff keep what leaks through a building small (a wall behind one faces
//! P across a block at least).
//!
//! The gather takes `SAMPLES` texels on a disc of `RADIUS_M` round P's own
//! texel in light space, sample i at radius `RADIUS_M u^2` (u its stratum,
//! jittered), on a golden-angle spiral turned per pixel: dense near P where
//! the falloff is steep. Each stands for `area` (the inverse of the sample
//! density: 4 pi r^1.5 R^0.5 / N), so the sum is an unbiased estimate of the
//! integral over the disc; the distance is held to at least the sample's own
//! area's square root, so no sample stands for more than about a steradian.
//!
//! Near P the screen-space bounce (surfaces_ao.wgsl, `AO_RADIUS_M`) gathers
//! the surfaces it can see with their own colours and true visibility; a
//! sample nearer than that radius is weighted down by `d^2 / R_ss^2`, the
//! share the screen-space pass leaves (its own falloff is 1 - d^2 / R_ss^2).

use crate::math::*;

/// Samples a point gathers (24 at half resolution: the frame's temporal
/// resolve takes the rest of the noise).
pub const SAMPLES: u32 = 24;
/// Samples a point gathers a frame where the screen-space gather's history
/// (`--screen-gi`, `screen_gi::HISTORY_WEIGHT`) holds the result: its mean
/// over about 19 frames takes the rest, each frame's disc turned anew.
pub const SAMPLES_WITH_HISTORY: u32 = 12;
/// The gather's radius in light space, metres: a street's width and a
/// canyon's lower floors (the falloff leaves a surface at 32 m about a
/// tenth of one at 10 m).
pub const RADIUS_M: f64 = 32.0;
/// The screen-space bounce's radius (surfaces_ao.wgsl `AO_RADIUS_M`): the
/// near field it leaves to this gather by `d^2 / R^2`.
pub const SCREEN_SPACE_RADIUS_M: f64 = 4.0;
/// The least squared distance a sample is taken at (m^2).
pub const MIN_DIST2_M2: f64 = 0.25;
/// A surface is the ground or a roof (left to the ground bounce) when its
/// normal is within this of straight up (cos), else a wall.
pub const GROUND_COS: f64 = 0.7;
/// The walls' albedo (linear sRGB). INFERRED from the reference sheet:
/// weathered concrete, stucco and painted brick fronts 0.25-0.35 (LBNL
/// measurements, US EPA 2012), their reflectance rising toward the red;
/// a luminance of 0.28.
pub const WALL_ALBEDO: Rgb = [0.31, 0.28, 0.24];
/// A sample's surface is refused when its normal turns from the sun past
/// this (cos): a broken normal at a building's edge in the map.
pub const MIN_SUN_COS: f64 = 0.05;
/// The shader's normals come from the map's depth a metre either way along
/// its x and y: a step along the beam steeper than this many times the
/// baseline (a surface within 2.4 degrees of edge-on to the sun) is a
/// roof's edge dropping to the street, no surface, and is refused. The
/// ground under a 3-degree sun stands at 19 times its baseline.
pub const MAX_SLOPE: f64 = 24.0;
/// The golden angle (radians), the spiral's step.
pub const GOLDEN_ANGLE: f64 = 2.399_963_229_728_653;

/// Sample `i` of `n` (jitter `j` in 0..1, the spiral turned by `rot`
/// radians): its offset in light space (metres, along the map's x and y) and
/// the area it stands for (m^2).
pub fn sample(i: u32, n: u32, j: f64, rot: f64) -> ([f64; 2], f64) {
    let u = (i as f64 + j) / n as f64;
    let r = RADIUS_M * u * u;
    let a = i as f64 * GOLDEN_ANGLE + rot;
    let area = 4.0 * std::f64::consts::PI * r.powf(1.5) * RADIUS_M.sqrt() / n as f64;
    ([r * a.cos(), r * a.sin()], area)
}

/// One sample's irradiance at `p` (normal `n`) from the sunlit surface at
/// `x` (normal `nx`) standing for `area` under the key `key` (irradiance
/// facing the sun): the wall's radiance less the rig's stand-in for it
/// (`stand_in`, the ground lobe), rho E_n - stand_in / cos_s, at least 0,
/// x a cos_x cos_p / (pi d^2) x the near-field share. Nothing from the
/// ground and roofs (the ground bounce's) or a surface turned from the sun.
pub fn contribution(p: [f64; 3], n: [f64; 3], x: [f64; 3], nx: [f64; 3], to_sun: [f64; 3], area: f64, key: Rgb, stand_in: Rgb) -> Rgb {
    let d = [x[0] - p[0], x[1] - p[1], x[2] - p[2]];
    let d2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
    let cs = dot3(nx, to_sun);
    if d2 < 1e-8 || cs < MIN_SUN_COS || nx[1] > GROUND_COS {
        return [0.0; 3];
    }
    let dist = d2.sqrt();
    let dir = [d[0] / dist, d[1] / dist, d[2] / dist];
    let cp = dot3(n, dir);
    let cx = -dot3(nx, dir);
    if cp <= 0.0 || cx <= 0.0 {
        return [0.0; 3];
    }
    let near = (d2 / (SCREEN_SPACE_RADIUS_M * SCREEN_SPACE_RADIUS_M)).min(1.0);
    let k = cp * cx * area * near / (std::f64::consts::PI * d2.max(area).max(MIN_DIST2_M2));
    [0, 1, 2].map(|c| (WALL_ALBEDO[c] * key[c] - stand_in[c] / cs).max(0.0) * k)
}

/// The CPU reference of the shader's gather at `p` (normal `n`), its
/// irradiance under the key `key` with the stand-in `stand_in`: `surface(q)`
/// is the first surface the sun meets through the light-space point `q`
/// (metres along `basis[0]` and `basis[1]` from P's own), its position and
/// normal, or None.
#[allow(clippy::too_many_arguments)]
pub fn gather(p: [f64; 3], n: [f64; 3], to_sun: [f64; 3], basis: [[f64; 3]; 2], key: Rgb, stand_in: Rgb, j: f64, rot: f64, surface: impl Fn([f64; 3]) -> Option<([f64; 3], [f64; 3])>) -> Rgb {
    let mut e = [0.0; 3];
    for i in 0..SAMPLES {
        let (o, area) = sample(i, SAMPLES, j, rot);
        let q = [0, 1, 2].map(|c| p[c] + basis[0][c] * o[0] + basis[1][c] * o[1]);
        if let Some((x, nx)) = surface(q) {
            let c = contribution(p, n, x, nx, to_sun, area, key, stand_in);
            for k in 0..3 {
                e[k] += c[k];
            }
        }
    }
    e
}

fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shadow::light_basis;

    /// The sun at `alt` degrees from the azimuth `az` (radians from +x toward +z).
    fn to_sun(alt: f64, az: f64) -> [f64; 3] {
        let (sa, ca) = (alt * DEG).sin_cos();
        [ca * az.cos(), sa, ca * az.sin()]
    }

    type Surface = Box<dyn Fn([f64; 3]) -> Option<([f64; 3], [f64; 3])>>;

    /// The first hit of the sun's ray through `q` on the ground plane y = 0 (normal up).
    fn ground(s: [f64; 3]) -> Surface {
        Box::new(move |q: [f64; 3]| {
            let t = q[1] / s[1];
            Some(([q[0] - s[0] * t, 0.0, q[2] - s[2] * t], [0.0, 1.0, 0.0]))
        })
    }

    /// A facade at x = `at` facing -x, `h` m tall, as the sun's rays through q meet it.
    fn facade(s: [f64; 3], at: f64, h: f64) -> Surface {
        Box::new(move |q: [f64; 3]| {
            let t = (at - q[0]) / -s[0];
            let y = q[1] - s[1] * t;
            (y > 0.0 && y < h).then_some(([at, y, q[2] - s[2] * t], [-1.0, 0.0, 0.0]))
        })
    }

    /// The mean of the gather over many jitters and turns (the frame's temporal resolve).
    fn converged(p: [f64; 3], n: [f64; 3], s: [f64; 3], key: Rgb, stand_in: Rgb, surface: Surface) -> Rgb {
        let b = light_basis(s);
        let basis = [b[0], b[1]];
        let k = 4096;
        let mut e = [0.0; 3];
        for t in 0..k {
            let j = ((t as f64 + 0.5) * 0.754_877_666).fract();
            let rot = ((t as f64 + 0.5) * 0.569_840_29).fract() * TAU;
            let g = gather(p, n, s, basis, key, stand_in, j, rot, &surface);
            for c in 0..3 {
                e[c] += g[c] / k as f64;
            }
        }
        e
    }

    #[test]
    fn samples_cover_the_disc_once() {
        // the areas sum to the disc's on average over the jitter.
        let k = 2000;
        let mut sum = 0.0;
        for t in 0..k {
            let j = (t as f64 + 0.5) / k as f64;
            for i in 0..SAMPLES {
                sum += sample(i, SAMPLES, j, 0.0).1;
            }
        }
        let disc = std::f64::consts::PI * RADIUS_M * RADIUS_M;
        assert!((sum / k as f64 / disc - 1.0).abs() < 0.01, "{} of the disc", sum / k as f64 / disc);
        for i in 0..SAMPLES {
            let (o, _) = sample(i, SAMPLES, 0.999, 1.0);
            assert!((o[0] * o[0] + o[1] * o[1]).sqrt() <= RADIUS_M + 1e-9);
        }
    }

    #[test]
    fn a_sunlit_wall_across_a_street_lights_the_shaded_one_by_its_form_factor() {
        // two facades 20 m apart: x = 0 faces +x (in shade, the sun behind it),
        // x = 20 faces -x toward a low western sun, sunlit to its 40 m top. With
        // no stand-in the gather is the facade's exitance x the form factor of
        // the part the disc reaches (a point 10 m up, the facade from -10 to
        // +28 m round it, about 32 m along it either way): 0.552 by a 800 x 800
        // quadrature of the same integral.
        let s = to_sun(10.0, std::f64::consts::PI); // from -x
        let key = [1.0; 3];
        let e = converged([0.0, 10.0, 0.0], [1.0, 0.0, 0.0], s, key, [0.0; 3], facade(s, 20.0, 40.0));
        let m = WALL_ALBEDO[1] * -s[0]; // the facade's exitance (cos_s = the sun's horizontal share)
        println!("shaded facade from the sunlit one 20 m across: {e:?}, {:.3} of its exitance", e[1] / m);
        assert!((e[1] / m - 0.552).abs() < 0.02, "{}", e[1] / m);
        assert!(e[0] > e[1] && e[1] > e[2], "the walls' albedo is warm: {e:?}");
        // the stand-in the rig already gives those directions is taken off.
        let net = converged([0.0, 10.0, 0.0], [1.0, 0.0, 0.0], s, key, [0.1; 3], facade(s, 20.0, 40.0));
        for c in 0..3 {
            assert!(net[c] < e[c] && net[c] > 0.0, "{net:?} of {e:?}");
        }
    }

    #[test]
    fn the_ground_roofs_and_grazed_walls_bring_nothing() {
        let s = to_sun(40.0, 0.5);
        // the sunlit ground is the ground bounce's: nothing from it, facing down or sideways.
        for n in [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0]] {
            assert_eq!(converged([0.0, 6.0, 0.0], n, s, [1.0; 3], [0.0; 3], ground(s)), [0.0; 3]);
        }
        // a wall the sun grazes is no brighter than the stand-in for it.
        let g = to_sun(10.0, std::f64::consts::PI * 0.5 + 0.03); // nearly along the facade
        let e = converged([0.0, 10.0, 0.0], [1.0, 0.0, 0.0], g, [1.0; 3], [0.2; 3], facade(g, 20.0, 40.0));
        assert!(e.iter().all(|v| *v < 1e-3), "{e:?}");
    }
}
