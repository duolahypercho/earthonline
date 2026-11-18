//! Sun shadow cascades as the game sets them up: three's `CSMShadowNode`
//! (three/addons/csm/CSMShadowNode.js, r180) with the renderer's settings
//! (src/citygen/renderer.js: 3 cascades, 3072 px, `maxFar` 160 m, 'practical'
//! split, `fade` on, `lightMargin` = max caster height + 40 m, cascade near
//! 0.5 m and far `lightMargin + 2 * halfExtent + 2`) and the bias pair from
//! `recommendShadowBias` (src/render/shadow-casters.js).
//!
//! Stable: each box is sized by its frustum slice alone (independent of the
//! camera's rotation) and its centre is snapped to whole texels in light
//! space, so a still sun gives the same texel grid however the camera moves.

use crate::math::*;

pub const CASCADES: usize = 3;
pub const MAP_SIZE: u32 = 3072;
pub const MAX_FAR_M: f64 = 160.0;
/// `SHADOW_FIT_DEFAULTS.maxCasterHeight` + `SUN_SHADOW_LIGHT_MARGIN_PAD`.
pub const LIGHT_MARGIN_M: f64 = 260.0 + 40.0;
pub const CASCADE_NEAR_M: f64 = 0.5;

/// three's `practicalSplit(cascades, near, far, 0.5)`: fractions of the
/// near->far corner interpolation at which each cascade ends.
pub fn practical_breaks(cascades: usize, near: f64, far: f64) -> Vec<f64> {
    let mut out = vec![];
    for i in 1..cascades {
        let log = (near * (far / near).powf(i as f64 / cascades as f64)) / far;
        let uni = (near + (far - near) * i as f64 / cascades as f64) / far;
        out.push(mix(uni, log, 0.5));
    }
    out.push(1.0);
    out
}

/// `slopeDepthErrorTexels`.
fn slope_depth_error_texels(phi_deg: f64) -> f64 {
    let phi = phi_deg * DEG;
    let c = phi.cos();
    if !(c > 1e-6) {
        return f64::INFINITY;
    }
    0.5 * phi.sin() / c
}

fn round_to(v: f64, digits: i32) -> f64 {
    let k = 10f64.powi(digits);
    (v * k).round() / k
}

/// `recommendShadowBias`: (normalBias metres, depth pull-back metres, bias in
/// depth units for `depth_range`).
pub fn recommend_bias(texel_world: f64, depth_range: f64, sun_altitude_deg: f64) -> (f64, f64, f64) {
    const MIN_THICKNESS_TEXELS: f64 = 1.5;
    const MIN_DEPTH_BIAS_TEXELS: f64 = 0.25;
    const MAX_PROTECTED_INCIDENCE_DEG: f64 = 75.0;
    const MAX_RECEIVER_SLOPE_DEG: f64 = 55.0;
    let facade = ((52.0 * DEG).cos() * (50.0 * DEG).cos()).acos() / DEG;
    let alt = clamp(sun_altitude_deg, 1.0, 89.0);
    let asked = MAX_RECEIVER_SLOPE_DEG.max(facade).max(90.0 - alt);
    let protect = asked.min(MAX_PROTECTED_INCIDENCE_DEG);
    let cos = (protect * DEG).cos();
    let required = slope_depth_error_texels(protect);
    let wanted = ((required - MIN_DEPTH_BIAS_TEXELS) / cos).max(0.0);
    let affordable = ((MIN_THICKNESS_TEXELS - required) / (1.0 - cos)).max(0.0);
    let normal_texels = round_to(wanted.min(affordable), 6);
    let depth_texels = round_to(MIN_DEPTH_BIAS_TEXELS.max(required - wanted.min(affordable) * cos), 6);
    let normal = round_to(normal_texels * texel_world, 4);
    let pullback = depth_texels * texel_world;
    let bias = -round_to(pullback / depth_range, 7);
    (normal, pullback, bias)
}

#[derive(Clone, Copy, Debug)]
pub struct CascadeCamera {
    pub eye: [f64; 3],
    pub forward: [f64; 3],
    pub right: [f64; 3],
    pub up: [f64; 3],
    pub fov_y_deg: f64,
    pub aspect: f64,
    pub near: f64,
    pub far: f64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Cascade {
    /// World -> clip (x, y in -1..1, depth 0 at the light's near plane), column-major.
    pub view_proj: [[f32; 4]; 4],
    /// Six planes (n, d), inside where n.p + d >= 0, for the GPU cull.
    pub planes: [[f32; 4]; 6],
    /// View depth (metres along the camera's forward axis) where this cascade ends.
    pub split_far: f64,
    pub half_extent: f64,
    pub texel_world: f64,
    pub depth_range: f64,
    pub normal_bias: f64,
    /// Depth pull-back, metres along the light.
    pub depth_bias_m: f64,
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm(a: [f64; 3]) -> [f64; 3] {
    let l = dot(a, a).sqrt();
    if l > 0.0 { [a[0] / l, a[1] / l, a[2] / l] } else { a }
}
fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// Light basis of `Matrix4.lookAt(position, target, up)` for a light that
/// shines along `-to_sun`: (x, y, z) with z pointing toward the sun.
pub fn light_basis(to_sun: [f64; 3]) -> [[f64; 3]; 3] {
    let mut z = norm(to_sun);
    let up = [0.0, 1.0, 0.0];
    let mut x = cross(up, z);
    if dot(x, x) == 0.0 {
        z[2] += 0.0001;
        z = norm(z);
        x = cross(up, z);
    }
    let x = norm(x);
    let y = cross(z, x);
    [x, y, z]
}

/// Fit the cascades for a camera and a key direction (toward the sun).
pub fn fit_cascades(cam: &CascadeCamera, to_sun: [f64; 3], map_size: u32) -> [Cascade; CASCADES] {
    let far = cam.far.min(MAX_FAR_M);
    let breaks = practical_breaks(CASCADES, cam.near, far);
    // Corner points in camera space (x right, y up, z forward) at the near and
    // far planes (three's CSMFrustum: near at camera.near, far at maxFar).
    let th = (cam.fov_y_deg * DEG * 0.5).tan();
    let corners = |d: f64| -> [[f64; 3]; 4] {
        let (h, w) = (d * th, d * th * cam.aspect);
        [[w, h, d], [w, -h, d], [-w, -h, d], [-w, h, d]]
    };
    let nv = corners(cam.near);
    let fv = corners(far);
    let to_world = |p: [f64; 3]| -> [f64; 3] {
        [
            cam.eye[0] + cam.right[0] * p[0] + cam.up[0] * p[1] + cam.forward[0] * p[2],
            cam.eye[1] + cam.right[1] * p[0] + cam.up[1] * p[1] + cam.forward[1] * p[2],
            cam.eye[2] + cam.right[2] * p[0] + cam.up[2] * p[1] + cam.forward[2] * p[2],
        ]
    };
    let [lx, ly, lz] = light_basis(to_sun);
    let mut out = [Cascade::default(); CASCADES];
    for (i, c) in out.iter_mut().enumerate() {
        let b0 = if i == 0 { 0.0 } else { breaks[i - 1] };
        let b1 = breaks[i];
        let lerp = |a: [f64; 3], b: [f64; 3], t: f64| [mix(a[0], b[0], t), mix(a[1], b[1], t), mix(a[2], b[2], t)];
        let near_s: Vec<[f64; 3]> = (0..4).map(|j| lerp(nv[j], fv[j], b0)).collect();
        let far_s: Vec<[f64; 3]> = (0..4).map(|j| lerp(nv[j], fv[j], b1)).collect();
        // `_updateShadowBounds`: the slice's diagonal, plus the fade margin.
        let p1 = far_s[0];
        let mut width = if dist(p1, far_s[2]) > dist(p1, near_s[2]) { dist(p1, far_s[2]) } else { dist(p1, near_s[2]) };
        let fade_far = cam.far.max(MAX_FAR_M);
        let linear_depth = far_s[0][2] / (fade_far - cam.near);
        width += 0.25 * linear_depth * linear_depth * (fade_far - cam.near);
        let half = width / 2.0;
        let texel = width / map_size as f64;
        // `updateFrustums`: light-space box of the slice, centre pulled toward
        // the sun by the margin, snapped to texels.
        let mut mn = [f64::MAX; 3];
        let mut mx = [f64::MIN; 3];
        for p in near_s.iter().chain(far_s.iter()) {
            let w = to_world(*p);
            let ls = [dot(w, lx), dot(w, ly), dot(w, lz)];
            for k in 0..3 {
                mn[k] = mn[k].min(ls[k]);
                mx[k] = mx[k].max(ls[k]);
            }
        }
        let cx = ((mn[0] + mx[0]) / 2.0 / texel).floor() * texel;
        let cy = ((mn[1] + mx[1]) / 2.0 / texel).floor() * texel;
        let cz = mx[2] + LIGHT_MARGIN_M;
        let near = CASCADE_NEAR_M;
        let far_l = LIGHT_MARGIN_M + half * 2.0 + 2.0;
        // clip = M * world. x' = (p.lx - cx) / half, y' = (p.ly - cy) / half,
        // z' = (cz - p.lz - near) / (far - near).
        let dr = far_l - near;
        let rows: [[f64; 4]; 4] = [
            [lx[0] / half, lx[1] / half, lx[2] / half, -cx / half],
            [ly[0] / half, ly[1] / half, ly[2] / half, -cy / half],
            [-lz[0] / dr, -lz[1] / dr, -lz[2] / dr, (cz - near) / dr],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let mut vp = [[0f32; 4]; 4];
        for col in 0..4 {
            for row in 0..4 {
                vp[col][row] = rows[row][col] as f32;
            }
        }
        // Planes (Gribb-Hartmann, depth range 0..1).
        let r = |k: usize| rows[k];
        let add = |a: [f64; 4], b: [f64; 4], s: f64| [a[0] + s * b[0], a[1] + s * b[1], a[2] + s * b[2], a[3] + s * b[3]];
        let raw = [add(r(3), r(0), 1.0), add(r(3), r(0), -1.0), add(r(3), r(1), 1.0), add(r(3), r(1), -1.0), r(2), add(r(3), r(2), -1.0)];
        let mut planes = [[0f32; 4]; 6];
        for (k, p) in raw.iter().enumerate() {
            let l = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt().max(1e-12);
            planes[k] = [(p[0] / l) as f32, (p[1] / l) as f32, (p[2] / l) as f32, (p[3] / l) as f32];
        }
        let alt = to_sun[1].clamp(-1.0, 1.0).asin() / DEG;
        let (nb, pull, _) = recommend_bias(texel, dr, alt);
        *c = Cascade {
            view_proj: vp,
            planes,
            split_far: cam.near + b1 * (far - cam.near),
            half_extent: half,
            texel_world: texel,
            depth_range: dr,
            normal_bias: nb,
            depth_bias_m: pull,
        };
    }
    out
}
