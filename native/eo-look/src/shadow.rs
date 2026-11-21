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

// ---------------------------------------------------------------------------
// Blending between neighbouring cascades (`--cascade-blend`).
// ---------------------------------------------------------------------------

/// The share of a cascade's slice (view depth) over which it hands over to
/// the next one before their split. A rendering choice, not a measurement:
/// wide enough that the coarser cascade's softer penumbra takes over without
/// a visible line, narrow enough that most of the slice keeps the sharper
/// cascade.
pub const CASCADE_BLEND_BAND: f64 = 0.2;

/// The cascade at view depth `vd` and the weight of the next one there
/// (`splits` the three cascades' far edges; the first slice is taken from 0):
/// 0 before the band, rising (smoothstep) to 1 at the split. The last cascade
/// hands over to nothing (its own far fade, `look.rs`). Mirrored in
/// `look_light.wgsl` `lk_shadow`.
pub fn cascade_blend_weight(vd: f64, splits: [f64; CASCADES]) -> (usize, f64) {
    let c = if vd > splits[1] {
        2
    } else if vd > splits[0] {
        1
    } else {
        0
    };
    if c + 1 >= CASCADES {
        return (c, 0.0);
    }
    let start = if c == 0 { 0.0 } else { splits[c - 1] };
    let end = splits[c];
    let band = (end - start) * CASCADE_BLEND_BAND;
    (c, smoothstep(end - band, end, vd))
}

// ---------------------------------------------------------------------------
// The far sun shadow rings (eo-render `far_shadow.rs`, `--far-shadows`): a
// cached orthographic depth map a ring, round the camera, past the cascades.
// ---------------------------------------------------------------------------

/// Texels a side of a ring's map (the high tier's 4096).
pub const FAR_MAP_SIZE: u32 = 4096;
/// Ring radii, metres on the ground: ring 1 about 2 km across (high), ring 2
/// out to the camera's far plane (hyper; `eo-render` camera `GAME_FAR_M`).
pub const FAR_RING_RADIUS_M: [f64; 2] = [1000.0, 4200.0];
/// Ring 1's radius grows with the eye's height over the scene's lowest
/// ground (`far_ring_radius`): 2 km across at street level, doubled each time
/// the eye's height doubles past `FAR_RING_GROW_AT_M` (so about 6.7 times the
/// height), up to the far plane; a view from a tower, a hill or the air has
/// the far city it sees in the map at texels it can resolve. A radius within
/// `FAR_RING_SNAP_SHARE` of the far plane takes the far plane. Steps by a
/// `FAR_RING_HYSTERESIS` share either side, so an eye hovering at a step does
/// not redraw the map back and forth.
pub const FAR_RING_GROW_AT_M: f64 = 150.0;
pub const FAR_RING_SNAP_SHARE: f64 = 0.9;
pub const FAR_RING_HYSTERESIS: f64 = 0.1;
/// The lowest sun the rings are fitted for: below it the casters' reach
/// toward the sun is held at this altitude's (the shadows are then kilometres
/// long and the key light all but gone, eo-sky's key envelope).
pub const FAR_MIN_ALTITUDE_DEG: f64 = 2.0;
/// The farthest a caster may stand toward the sun from what it shadows,
/// metres along the light (the depth range is spent on it; 16-bit depth over
/// 8 km plus the ring is still under 0.2 m a step).
pub const FAR_CASTER_REACH_M: f64 = 8000.0;
/// Slack round the receivers' box along the light, metres.
pub const FAR_DEPTH_PAD_M: f64 = 4.0;
/// Extra texels round the ring in light space (the snapped centre moves by up to one).
pub const FAR_EDGE_PAD_TEXELS: f64 = 2.0;

/// Ring `ring`'s radius for an eye `eye_height` metres over the scene's
/// lowest ground under a camera whose far plane is `far_plane` metres, given
/// the radius its map has now (None: none yet). Ring 2 reaches the far plane.
pub fn far_ring_radius(ring: usize, eye_height: f64, far_plane: f64, current: Option<f64>) -> f64 {
    let base = FAR_RING_RADIUS_M[0];
    let far = far_plane.max(base);
    if ring > 0 {
        return far;
    }
    let radius = |n: u32| {
        let r = base * 2f64.powi(n as i32);
        if r >= FAR_RING_SNAP_SHARE * far { far } else { r }
    };
    let top = (0..32).find(|&n| radius(n) == far).unwrap_or(31);
    let grow_at = |n: u32| FAR_RING_GROW_AT_M * 2f64.powi(n as i32);
    let mut n = match current.and_then(|c| (0..=top).find(|&n| (radius(n) - c).abs() < 1e-6)) {
        Some(n) => n,
        None => (0..top).filter(|&k| eye_height >= grow_at(k)).count() as u32,
    };
    while n < top && eye_height >= grow_at(n) * (1.0 + FAR_RING_HYSTERESIS) {
        n += 1;
    }
    while n > 0 && eye_height < grow_at(n - 1) * (1.0 - FAR_RING_HYSTERESIS) {
        n -= 1;
    }
    radius(n)
}

/// One far ring's fit: world -> the map's clip space, x and y in -1..1 over
/// the ring's box, z 0 at the sunward plane and 1 at the far one.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FarRing {
    /// The affine rows of world -> clip (x, y, z; w is 1): clip = row . (p, 1).
    pub rows: [[f64; 4]; 3],
    /// The same as a column-major 4 x 4 (the depth pass's view-projection).
    pub view_proj: [[f32; 4]; 4],
    /// Six planes (n, d), inside where n.p + d >= 0, for the GPU cull.
    pub planes: [[f32; 4]; 6],
    /// The ring's centre (the eye it was fitted round) and radius on the ground.
    pub eye: [f64; 3],
    pub radius: f64,
    /// Toward the sun when fitted.
    pub to_sun: [f64; 3],
    /// Half extents of the box in light space (x across the sun, y up the
    /// light's frame), metres; the texels a side `map_size`.
    pub half: [f64; 2],
    pub texel: [f64; 2],
    pub map_size: u32,
    /// Light-space depth covered, metres (the receivers and the casters' reach).
    pub depth_range: f64,
    /// Receiver normal offset (metres) and depth pull-back (metres along the
    /// light; folded into `rows[2]`), from `recommend_bias` at the coarser texel.
    pub normal_bias: f64,
    pub depth_bias_m: f64,
}

/// Fit ring `radius` round `eye` for a key toward `to_sun`, covering every
/// receiver on the ground disc between heights `y_range` (lowest ground,
/// highest roof or tower top of the scene) and every caster toward the sun
/// that can shadow one. The box is a rectangle in light space (across the
/// sun: the disc's diameter; up the light's frame: the disc foreshortened by
/// the sun's altitude plus the heights), its centre snapped to whole texels,
/// so a still sun gives the same texel grid wherever the camera stands.
pub fn fit_far_ring(eye: [f64; 3], radius: f64, to_sun: [f64; 3], map_size: u32, y_range: (f64, f64)) -> FarRing {
    let [lx, ly, lz] = light_basis(to_sun);
    let (y_lo, y_hi) = (y_range.0.min(y_range.1), y_range.0.max(y_range.1));
    let height = (y_hi - y_lo).max(1.0);
    let n = map_size.max(2) as f64;
    // the receivers: a cylinder of `radius` round the eye from y_lo to y_hi.
    let c = [eye[0], 0.5 * (y_lo + y_hi), eye[2]];
    let h_len = |v: [f64; 3]| (v[0] * v[0] + v[2] * v[2]).sqrt();
    let extent = |l: [f64; 3]| radius * h_len(l) + 0.5 * height * l[1].abs();
    // lx is level (cross(up, z)): its extent is the radius alone.
    let (ex, ey, ez) = (extent(lx), extent(ly), extent(lz));
    let pad = |e: f64| e * n / (n - 2.0 * FAR_EDGE_PAD_TEXELS);
    let half = [pad(ex).max(1.0), pad(ey).max(1.0)];
    let texel = [2.0 * half[0] / n, 2.0 * half[1] / n];
    let cx = (dot(c, lx) / texel[0]).floor() * texel[0];
    let cy = (dot(c, ly) / texel[1]).floor() * texel[1];
    // along the light: from the far side of the receivers to their sunward
    // side plus the reach of a caster as tall as the scene (H / sin(altitude)).
    let alt = (to_sun[1].clamp(-1.0, 1.0).asin() / DEG).max(FAR_MIN_ALTITUDE_DEG);
    let reach = (height / (alt * DEG).sin()).min(FAR_CASTER_REACH_M);
    let zc = dot(c, lz);
    let z_near = zc + ez + reach + FAR_DEPTH_PAD_M;
    let z_far = zc - ez - FAR_DEPTH_PAD_M;
    let dr = z_near - z_far;
    let texel_max = texel[0].max(texel[1]);
    let (nb, pull, _) = recommend_bias(texel_max, dr, alt);
    // clip = rows . (p, 1); the pull-back toward the sun is folded into z.
    let rows = [
        [lx[0] / half[0], lx[1] / half[0], lx[2] / half[0], -cx / half[0]],
        [ly[0] / half[1], ly[1] / half[1], ly[2] / half[1], -cy / half[1]],
        [-lz[0] / dr, -lz[1] / dr, -lz[2] / dr, (z_near - pull) / dr],
    ];
    // the depth pass draws the casters without the pull-back.
    let cast = [rows[0], rows[1], [rows[2][0], rows[2][1], rows[2][2], z_near / dr], [0.0, 0.0, 0.0, 1.0]];
    let mut vp = [[0f32; 4]; 4];
    for col in 0..4 {
        for row in 0..4 {
            vp[col][row] = cast[row][col] as f32;
        }
    }
    FarRing {
        rows,
        view_proj: vp,
        planes: box_planes(&cast, [-1.0, 1.0], [-1.0, 1.0]),
        eye,
        radius,
        to_sun,
        half,
        texel,
        map_size,
        depth_range: dr,
        normal_bias: nb,
        depth_bias_m: pull,
    }
}

/// The cull planes of the part of a box's clip space x in `xr`, y in `yr`
/// (z 0..1), from its rows (x, y, z, w = (0, 0, 0, 1)).
fn box_planes(r: &[[f64; 4]; 4], xr: [f64; 2], yr: [f64; 2]) -> [[f32; 4]; 6] {
    let lin = |a: [f64; 4], b: [f64; 4], s: f64| [a[0] + s * b[0], a[1] + s * b[1], a[2] + s * b[2], a[3] + s * b[3]];
    // x >= xr0 w, x <= xr1 w, y >= yr0 w, y <= yr1 w, z >= 0, z <= w.
    let raw = [lin(r[0], r[3], -xr[0]), lin(lin([0.0; 4], r[0], -1.0), r[3], xr[1]), lin(r[1], r[3], -yr[0]), lin(lin([0.0; 4], r[1], -1.0), r[3], yr[1]), r[2], lin(r[3], r[2], -1.0)];
    let mut planes = [[0f32; 4]; 6];
    for (k, p) in raw.iter().enumerate() {
        let l = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt().max(1e-12);
        planes[k] = [(p[0] / l) as f32, (p[1] / l) as f32, (p[2] / l) as f32, (p[3] / l) as f32];
    }
    planes
}

impl FarRing {
    /// Tile `k` of a `tiles` x `tiles` split of the map (row-major from the
    /// map's top-left texel): its scissor rectangle (x, y, w, h in texels) and
    /// its cull planes (the box cut to the tile).
    pub fn tile(&self, k: u32, tiles: u32) -> ([u32; 4], [[f32; 4]; 6]) {
        let tiles = tiles.max(1);
        let (tx, ty) = (k % tiles, (k / tiles) % tiles);
        let side = self.map_size / tiles;
        let rect = [tx * side, ty * side, if tx + 1 == tiles { self.map_size - tx * side } else { side }, if ty + 1 == tiles { self.map_size - ty * side } else { side }];
        // texel rows run down the map: v = 0.5 - 0.5 y, so y = 1 - 2 v.
        let u = |t: u32| -1.0 + 2.0 * t as f64 / self.map_size as f64;
        let y = |t: u32| 1.0 - 2.0 * t as f64 / self.map_size as f64;
        let cast = self.cast_rows();
        let planes = box_planes(&cast, [u(rect[0]), u(rect[0] + rect[2])], [y(rect[1] + rect[3]), y(rect[1])]);
        (rect, planes)
    }

    /// The rows the depth pass draws with (without the receivers' pull-back).
    pub fn cast_rows(&self) -> [[f64; 4]; 4] {
        let r = self.rows;
        let pull = self.depth_bias_m / self.depth_range;
        [r[0], r[1], [r[2][0], r[2][1], r[2][2], r[2][3] + pull], [0.0, 0.0, 0.0, 1.0]]
    }

    /// Clip coordinates of a world point (the receivers' rows, pull-back in).
    pub fn clip(&self, p: [f64; 3]) -> [f64; 3] {
        let d = |r: [f64; 4]| r[0] * p[0] + r[1] * p[1] + r[2] * p[2] + r[3];
        [d(self.rows[0]), d(self.rows[1]), d(self.rows[2])]
    }

    /// Degrees between this fit's sun and `to_sun`.
    pub fn sun_turn_deg(&self, to_sun: [f64; 3]) -> f64 {
        let (a, b) = (norm(self.to_sun), norm(to_sun));
        dot(a, b).clamp(-1.0, 1.0).acos() / DEG
    }

    /// Metres on the ground between this fit's eye and `eye`.
    pub fn moved_m(&self, eye: [f64; 3]) -> f64 {
        ((self.eye[0] - eye[0]).powi(2) + (self.eye[2] - eye[2]).powi(2)).sqrt()
    }
}
