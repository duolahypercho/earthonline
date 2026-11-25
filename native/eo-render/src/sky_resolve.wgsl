// the resolve (appended to look_common.wgsl). One full-screen pass from
// the HDR scene to the resolved HDR frame: the atmosphere volume composited by
// depth (a bilateral upsample of the half-resolution result), then temporal
// anti-aliasing: the history reprojected by the moving agents' own motion
// (the velocity target) or else by the camera's motion and the depth,
// 5-tap Catmull-Rom history, variance clipping in YCoCg against the 3 x 3
// neighbourhood, luminance-weighted blend. The result is the frame post
// tone-maps, the next frame's history, and the previous frame the wet
// ground's reflection reads.
//
// (`--cine-camera`, `SF_CAMERA` compiled in) its alpha carries the pixel's
// distance from the eye for the camera's depth of field: 2 + metres (the sky
// 65504; an alpha under 2 is a frame without it). Nothing else reads the
// alpha; compiled out it stays 1.
//
// (`--camera-resolve`, `SF_CAM_RESOLVE` compiled in; cine_post.rs
// `CAM_BIT_RESOLVE`) the frames accumulate in the unjittered image: the
// history is read where the camera's motion alone moved the pixel's point (not
// its jitter), the first frames after a cut are averaged evenly, and a still
// pixel's clipping box is `CAM_RESOLVE_GAMMA` deviations wide so coverage
// thinner than a pixel that the jitter moves (an edge, a wire, a frond's tip)
// accumulates. (A hole an alpha-tested canopy shows in every jitter phase is the
// material's, not the resolve's.)

/// (`--camera-resolve`) a still pixel's clipping box, in standard deviations of
/// its 3 x 3 neighbourhood (the model's: 1.25 still, 0.75 for a moving agent).
const CAM_RESOLVE_GAMMA: f32 = 2.0;

@group(0) @binding(0) var<uniform> look: Look;
@group(0) @binding(1) var<uniform> skyu: Sky;
@group(0) @binding(2) var rs_hdr: texture_2d<f32>;
@group(0) @binding(3) var rs_depth: texture_depth_2d;
@group(0) @binding(4) var rs_vol: texture_2d<f32>;
@group(0) @binding(5) var rs_volz: texture_2d<f32>;
@group(0) @binding(6) var rs_hist: texture_2d<f32>;
@group(0) @binding(7) var rs_vel: texture_2d<f32>;
@group(0) @binding(8) var rs_lin: sampler;
@group(0) @binding(9) var rs_trans: texture_2d<f32>; // the atmosphere's transmittance table
// (lighting, `--lit-fog`) the fog lit by the local lights (lit_fog.wgsl): the in-scatter from
// the eye to each froxel slice's far face, and [0] the frame it was integrated in.
@group(0) @binding(10) var rs_lit_fog_vol: texture_3d<f32>;
@group(0) @binding(11) var<storage, read> rs_lit_fog_state: array<u32>;

@vertex
fn resolve_vs(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(vi & 1u) * 4 - 1);
    let y = f32(i32(vi >> 1u) * 4 - 1);
    return vec4<f32>(x, y, 0.0, 1.0);
}

fn rs_ycocg(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(0.25 * c.r + 0.5 * c.g + 0.25 * c.b, 0.5 * c.r - 0.5 * c.b, -0.25 * c.r + 0.5 * c.g - 0.25 * c.b);
}

fn rs_rgb(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(c.x + c.y - c.z, c.x + c.z, c.x - c.y - c.z);
}

/// Range compression for the blend (Karis 2014): weights 1 / (1 + luma).
fn rs_tm(c: vec3<f32>) -> vec3<f32> {
    return c / (1.0 + lk_luma(c));
}

fn rs_itm(c: vec3<f32>) -> vec3<f32> {
    return c / max(1.0 - lk_luma(c), 1e-4);
}

/// The volume at a full-resolution pixel: four half-resolution texels,
/// bilinear x depth agreement.
fn rs_volume(uv: vec2<f32>, dist: f32) -> vec4<f32> {
    let vd = vec2<i32>(textureDimensions(rs_vol));
    let hv = uv * vec2<f32>(vd) - 0.5;
    let b = vec2<i32>(floor(hv));
    let f = fract(hv);
    var acc = vec4<f32>(0.0);
    var wsum = 0.0;
    var agree = 0.0; // the best depth agreement among the four
    for (var k = 0; k < 4; k = k + 1) {
        let o = vec2<i32>(k & 1, k >> 1);
        let p = clamp(b + o, vec2<i32>(0), vd - 1);
        let bw = select(1.0 - f.x, f.x, o.x == 1) * select(1.0 - f.y, f.y, o.y == 1);
        let z = textureLoad(rs_volz, p, 0).r;
        var dw = 0.001;
        if (dist < 0.0) {
            dw = select(0.001, 1.0, z < 0.0);
        } else if (z >= 0.0) {
            dw = exp(-abs(z - dist) / (0.04 * dist + 0.5)) + 0.001;
            // past 3 km both ways (the far land, the sea toward its horizon) a
            // texel stands for the pixel whatever its depth: near the horizon the sea's
            // distance changes several-fold from one half-resolution row to the next, no
            // texel agreed, and every other row took no volume (a dashed dark line along
            // the sea horizon).
            // and only within a factor of 2 of the pixel's own depth: a downtown tower
            // 3-5 km off took the volume of the land 10+ km behind it (its haze, its clouds) and
            // its top dissolved into the sky.
            if (dist > 3000.0 && z > 3000.0 && z < 2.0 * dist && dist < 2.0 * z) {
                dw = max(dw, 0.5);
            }
        }
        let w = bw * dw;
        acc = acc + textureLoad(rs_vol, p, 0) * w;
        wsum = wsum + w;
        agree = max(agree, dw);
    }
    // a full-resolution pixel of near, thin geometry (a bridge tower's
    // lattice, a tower's top) whose four half-resolution texels all saw the sky
    // or something far behind it took their clouds: puffs drawn over the
    // lattice. With no texel at its depth it takes no volume.
    if (dist >= 0.0 && agree < 0.05) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    return acc / max(wsum, 1e-6);
}

/// (lighting, `--lit-fog`) the lamps' and headlamps' light the fog scatters
/// toward the eye in front of a pixel at view distance `dist` (< 0: the sky,
/// the whole volume) along `ray`: the integrated froxel volume read at the
/// pixel's view depth (its texel k holds the light from the eye to slice k's
/// far face; log slices over the clusters' near .. far). Added on top of the
/// volume's result: the volume already carries this fog's extinction of the
/// scene. Zero without the feature or when the volume was not made this frame
/// (by day, no lights).
fn rs_lit_fog(uv: vec2<f32>, dist: f32, ray: vec3<f32>) -> vec3<f32> {
    if (!lk_lx_on(LX_BIT_LIT_FOG) || rs_lit_fog_state[0] != skyu.flags.y) {
        return vec3<f32>(0.0);
    }
    var s = 1.0;
    if (dist >= 0.0) {
        let vz = max(dist * dot(ray, skyu.cam_fwd.xyz), skyu.cluster.x);
        s = clamp(log(vz / skyu.cluster.x) / log(skyu.cluster.y / skyu.cluster.x), 0.0, 1.0);
    }
    // across the screen a cubic B-spline over the froxel columns (four trilinear taps, Sigg and
    // Hadwiger 2005): bilinear over 12 px columns drew a lamp's halo as a soft square.
    let dims = vec3<f32>(textureDimensions(rs_lit_fog_vol));
    let w = s - 0.5 / dims.z;
    let q = uv * dims.xy - 0.5;
    let i = floor(q);
    let f = q - i;
    let f2 = f * f;
    let f3 = f2 * f;
    let w0 = (1.0 - 3.0 * f + 3.0 * f2 - f3) / 6.0;
    let w1 = (4.0 - 6.0 * f2 + 3.0 * f3) / 6.0;
    let w2 = (1.0 + 3.0 * f + 3.0 * f2 - 3.0 * f3) / 6.0;
    let w3 = f3 / 6.0;
    let g0 = w0 + w1;
    let g1 = w2 + w3;
    let t0 = (i - 0.5 + w1 / g0) / dims.xy;
    let t1 = (i + 1.5 + w3 / g1) / dims.xy;
    var l = textureSampleLevel(rs_lit_fog_vol, rs_lin, vec3<f32>(t0.x, t0.y, w), 0.0).rgb * (g0.x * g0.y);
    l = l + textureSampleLevel(rs_lit_fog_vol, rs_lin, vec3<f32>(t1.x, t0.y, w), 0.0).rgb * (g1.x * g0.y);
    l = l + textureSampleLevel(rs_lit_fog_vol, rs_lin, vec3<f32>(t0.x, t1.y, w), 0.0).rgb * (g0.x * g1.y);
    l = l + textureSampleLevel(rs_lit_fog_vol, rs_lin, vec3<f32>(t1.x, t1.y, w), 0.0).rgb * (g1.x * g1.y);
    return select(max(l, vec3<f32>(0.0)), vec3<f32>(0.0), any(l != l));
}

fn rs_scene(p: vec2<i32>, dims: vec2<i32>, vol: vec4<f32>) -> vec3<f32> {
    let c = textureLoad(rs_hdr, clamp(p, vec2<i32>(0), dims - 1), 0).rgb;
    return max(c, vec3<f32>(0.0)) * vol.a + vol.rgb;
}

/// 5-tap Catmull-Rom history sample (Jimenez 2016).
fn rs_history(uv: vec2<f32>) -> vec3<f32> {
    let size = vec2<f32>(textureDimensions(rs_hist));
    let pos = uv * size;
    let c = floor(pos - 0.5) + 0.5;
    let f = pos - c;
    let w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
    let w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
    let w2 = f * (0.5 + f * (2.0 - 1.5 * f));
    let w3 = f * f * (-0.5 + 0.5 * f);
    let w12 = w1 + w2;
    let o12 = w2 / w12;
    let t0 = (c - 1.0) / size;
    let t3 = (c + 2.0) / size;
    let t12 = (c + o12) / size;
    var r = vec3<f32>(0.0);
    r = r + textureSampleLevel(rs_hist, rs_lin, vec2<f32>(t12.x, t0.y), 0.0).rgb * (w12.x * w0.y);
    r = r + textureSampleLevel(rs_hist, rs_lin, vec2<f32>(t0.x, t12.y), 0.0).rgb * (w0.x * w12.y);
    r = r + textureSampleLevel(rs_hist, rs_lin, vec2<f32>(t12.x, t12.y), 0.0).rgb * (w12.x * w12.y);
    r = r + textureSampleLevel(rs_hist, rs_lin, vec2<f32>(t3.x, t12.y), 0.0).rgb * (w3.x * w12.y);
    r = r + textureSampleLevel(rs_hist, rs_lin, vec2<f32>(t12.x, t3.y), 0.0).rgb * (w12.x * w3.y);
    let wsum = w12.x * w0.y + w0.x * w12.y + w12.x * w12.y + w3.x * w12.y + w12.x * w3.y;
    return max(r / wsum, vec3<f32>(0.0));
}

/// The resolved pixel and (`SF_CAMERA`) its distance for the camera's depth of field.
fn rs_out(c: vec3<f32>, dist: f32) -> vec4<f32> {
    if (SF_CAMERA) {
        return vec4<f32>(c, select(65504.0, min(2.0 + dist, 65000.0), dist >= 0.0));
    }
    return vec4<f32>(c, 1.0);
}

@fragment
fn resolve_fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let dims = vec2<i32>(textureDimensions(rs_hdr));
    let p = vec2<i32>(pos.xy);
    let uv = pos.xy / vec2<f32>(dims);
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let d = textureLoad(rs_depth, p, 0);
    var dist = -1.0;
    var rel: vec3<f32>;
    if (d > 0.0) {
        let w = look.inv_view_proj * vec4<f32>(ndc, d, 1.0);
        rel = w.xyz / w.w;
        dist = length(rel);
        // the far land past the far plane, as the volume pass reads it
        // (depth 1e-9 / view depth, DEPTH_K): without it the depth-aware
        // upsample found no volume texel at the far hills' distance and they
        // took no fog or aerial perspective.
        let fwd_cos = max(dot(normalize(rel), look.cam_fwd.xyz), 1e-3);
        let far_d = 1e-9 / d / fwd_cos;
        if (far_d > dist) {
            dist = far_d;
            rel = normalize(rel) * far_d;
        }
    } else {
        let w = look.inv_view_proj * vec4<f32>(ndc, 1.0, 1.0);
        rel = normalize(w.xyz / w.w) * 1e5;
    }
    // The sea's surface, not what lies under it (the water pass does not write
    // depth): the volume's distance there and the point the history is
    // reprojected from.
    let ray = normalize(rel);
    let sea_t = sky_air_end(skyu.eye.y, ray.y, 1e9);
    if (sea_t < 1e8 && (dist < 0.0 || sea_t < dist)) {
        dist = sea_t;
        rel = ray * sea_t;
    }
    var vol = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    if (skyu.flags.w == 1u) {
        vol = rs_volume(uv, dist);
    }
    var c = rs_scene(p, dims, vol);
    // the sun's disc (0.533 deg, limb-darkened, a one-pixel edge) at full
    // resolution, behind the share of it the volume lets through (its depth texel
    // stores -1 - that share over the sky): drawn in the volume at half resolution
    // and blended over frames it was a soft smudge, and at sunset it was gone.
    if (d <= 0.0 && sea_t >= 1e8 && skyu.atmo2.x > 0.5) {
        let r = 0.2667 * 0.0174533;
        let px = 0.035 * 0.0174533;
        // flattened by refraction near the horizon (the vertical radius 0.82 of
        // the horizontal at the horizon, round again by 3.5 deg up): the angle from the
        // centre with its vertical part stretched by 1 / that.
        let su = look.sun.xyz;
        // seen through the air's refraction (Bennett: ~0.57 deg at the horizon, 0.1 at
        // 5 deg): at 19.1 h the sun's true centre is under the horizon but the disc sits on it.
        let el_t = asin(clamp(su.y, -1.0, 1.0)) * 57.29578;
        let refr = 1.0 / tan((max(el_t, -1.5) + 7.31 / (max(el_t, -1.5) + 4.4)) * 0.0174533) / 60.0;
        let el_s = (el_t + refr) * 0.0174533;
        let dv = asin(clamp(ray.y, -1.0, 1.0)) - el_s;
        let dz = acos(clamp(dot(normalize(ray.xz + vec2<f32>(1e-7, 0.0)), normalize(su.xz + vec2<f32>(1e-7, 0.0))), -1.0, 1.0)) * cos(el_s);
        let fl = mix(0.82, 1.0, smoothstep(0.0, 0.06, su.y));
        let ang = sqrt(dz * dz + (dv / fl) * (dv / fl));
        let vol_clouds = sky_vol_clouds_on(skyu.cirrus);
        let beam_vis = smoothstep(0.15, 0.6, skyu.cloud1.w);
        if (ang < r + px || (vol_clouds && ang < 0.22 && el_s > -0.015 && beam_vis > 0.001)) {
            let vd = vec2<i32>(textureDimensions(rs_volz));
            let q = clamp(vec2<i32>(uv * vec2<f32>(vd)), vec2<i32>(0), vd - 1);
            // Smooth 2x2 bilinear sky transmittance when `--volumetric-clouds` is on so the sun disc
            // and corona feather cleanly around crisp cloud edges without half-res staircase edges.
            var raw_td = clamp(-textureLoad(rs_volz, q, 0).r - 1.0, 0.0, 1.0);
            if (vol_clouds) {
                let hv = uv * vec2<f32>(vd) - 0.5;
                let bi = vec2<i32>(floor(hv));
                let bf = fract(hv);
                var td_acc = 0.0;
                var td_w = 0.0;
                for (var k = 0; k < 4; k = k + 1) {
                    let o = vec2<i32>(k & 1, k >> 1);
                    let qp = clamp(bi + o, vec2<i32>(0), vd - 1);
                    let zv = textureLoad(rs_volz, qp, 0).r;
                    if (zv < 0.0) {
                        let bw = select(1.0 - bf.x, bf.x, o.x == 1) * select(1.0 - bf.y, bf.y, o.y == 1);
                        td_acc = td_acc + clamp(-zv - 1.0, 0.0, 1.0) * bw;
                        td_w = td_w + bw;
                    }
                }
                if (td_w > 1e-4) {
                    raw_td = td_acc / td_w;
                }
            }
            let floor_td = select(0.35 * (1.0 - smoothstep(0.75, 0.9, skyu.cloud0.z)), 0.0, vol_clouds);
            let td = max(raw_td, floor_td) * beam_vis;
            let tr = at_sun(rs_trans, rs_lin, skyu.eye.y, max(sin(el_s), 0.0005)) * 230.0;
            let dc = mix(tr, vec3<f32>(lk_luma(tr)), 0.75 * smoothstep(0.0, 0.08, look.sun.y)) * smoothstep(-0.0075, -0.002, el_s);
            let dl = lk_luma(dc);
            if (ang < r + px) {
                let x = clamp(ang / r, 0.0, 1.0);
                let limb = 1.0 - 0.6 * (1.0 - sqrt(max(1.0 - x * x, 0.0)));
                var dc2 = dc * (max(dl, 80.0 * lk_luma(c)) / max(dl, 1e-6));
                let pk = max(max(dc2.r, dc2.g), dc2.b);
                dc2 = mix(dc2, vec3<f32>(pk), 0.55 * (1.0 - smoothstep(0.3, 0.9, x)));
                c = c + select(dc2, vec3<f32>(0.0), dl <= 0.0) * limb * smoothstep(r + px, r - px, ang) * td;
            }
            if (vol_clouds && dl > 0.0) {
                // (`--volumetric-clouds`) Physical atmospheric Mie solar corona anchored strictly to
                // solar transmittance `dc` (never amplified by horizon luminance, preventing white-out bands).
                let su_h = max(length(su.xz), 1e-6);
                let sun_app = vec3<f32>(su.x / su_h * cos(el_s), sin(el_s), su.z / su_h * cos(el_s));
                var sun_td = raw_td;
                let sclip = skyu.vp * vec4<f32>(skyu.eye.xyz + sun_app * 1e5, 1.0);
                if (sclip.w > 1e-3) {
                    let suv = vec2<f32>(sclip.x / sclip.w * 0.5 + 0.5, 0.5 - sclip.y / sclip.w * 0.5);
                    if (all(suv >= vec2<f32>(0.0)) && all(suv <= vec2<f32>(1.0))) {
                        let sq = clamp(vec2<i32>(suv * vec2<f32>(vd)), vec2<i32>(0), vd - 1);
                        let sz = textureLoad(rs_volz, sq, 0).r;
                        sun_td = select(0.0, clamp(-sz - 1.0, 0.0, 1.0), sz < 0.0);
                    }
                }
                let corona_td = mix(raw_td * 0.40, max(sun_td, raw_td * 0.30), smoothstep(0.06, 0.006, ang)) * beam_vis;
                let rim_ang = max(ang - r * 0.85, 0.0);
                let inner = exp(-rim_ang / 0.0055) * 0.075;
                let mid = exp(-rim_ang / 0.026) * 0.0075;
                let outer = exp(-rim_ang / 0.085) * 0.0011 * smoothstep(0.22, 0.08, ang);
                let sun_col = mix(tr / max(lk_luma(tr), 1e-4), vec3<f32>(1.0), 0.35 * smoothstep(0.0, 0.18, su.y));
                c = c + sun_col * dl * (inner + mid + outer) * corona_td * smoothstep(-0.012, 0.004, el_s);
            }
        }
    }
    // Full-resolution forked sky lightning channel (`--volumetric-clouds` during active live
    // storm strikes `skyu.wind.w > 0.05`; zero cost outside active thunderstorm flashes).
    if (d <= 0.0 && sea_t >= 1e8 && sky_vol_clouds_on(skyu.cirrus) && skyu.wet.z >= 19.0 && skyu.wind.w > 0.05 && ray.y > 0.0) {
        let slot = floor(skyu.eye.w / 6.0);
        let h_az = fract(sin(slot * 12.9898 + 4.1414) * 43758.5453);
        let h_dist = fract(sin(slot * 78.233 + 1.917) * 43758.5453);
        let fwd_h = normalize(vec2<f32>(skyu.cam_fwd.x, skyu.cam_fwd.z) + vec2<f32>(1e-4, 0.0));
        let rgt_h = vec2<f32>(-fwd_h.y, fwd_h.x);
        let b_dist = mix(1900.0, 3800.0, h_dist);
        let b_base = skyu.eye.xz + fwd_h * b_dist + rgt_h * ((h_az - 0.5) * 1600.0);
        let rel_b = b_base - skyu.eye.xz;
        let b_dir = normalize(rel_b);
        let b_perp = vec2<f32>(-b_dir.y, b_dir.x);
        let r_hz = normalize(ray.xz + vec2<f32>(1e-6, 0.0));
        let along_cos = dot(r_hz, b_dir);
        if (along_cos > 0.96) {
            let b_top_y = skyu.cloud0.x + 80.0;
            let ray_y_at_b = skyu.eye.y + ray.y * length(rel_b) / max(along_cos, 0.1);
            if (ray_y_at_b > 0.0 && ray_y_at_b < b_top_y) {
                let u_y = clamp(ray_y_at_b / max(b_top_y, 1.0), 0.0, 1.0);
                // Multi-octave piecewise-kinked jagged lightning channel + upper secondary fork.
                let zig = sin(u_y * 19.0 + slot * 3.1) * 58.0
                    + sin(u_y * 47.0 - slot * 7.3) * 24.0
                    + sin(u_y * 113.0 + slot * 11.7) * 9.0;
                let lat_m = dot(r_hz, b_perp) * length(rel_b);
                let d_main = abs(lat_m - zig);
                // Secondary fork peeling off mid-channel downward.
                let fork_s = clamp((0.68 - u_y) * 1.8, 0.0, 1.0);
                let fork_x = zig + fork_s * 145.0 + sin(u_y * 61.0 + slot * 5.9) * 16.0;
                let d_fork = abs(lat_m - fork_x);
                let main_core = exp(-d_main * 0.34) * 1.4 + exp(-d_main * 0.055) * 0.32;
                let fork_core = (exp(-d_fork * 0.42) * 0.75 + exp(-d_fork * 0.08) * 0.14) * smoothstep(0.68, 0.56, u_y) * smoothstep(0.16, 0.34, u_y);
                let end_fade = smoothstep(0.0, 0.06, u_y) * smoothstep(1.0, 0.88, u_y);
                let bolt = (main_core + fork_core) * end_fade * skyu.wind.w;
                let bolt_col = mix(vec3<f32>(0.55, 0.78, 1.45), vec3<f32>(1.15, 1.22, 1.40), smoothstep(0.25, 0.9, bolt));
                c = c + bolt_col * bolt * 3.2;
            }
        }
    }
    // (lighting, `--lit-fog`) the lit fog in front of the pixel (and of the sun's disc); the
    // neighbourhood below takes it too, so the history is clipped to a box that holds it.
    let lit = rs_lit_fog(uv, dist, ray);
    c = c + lit;
    if (any(c != c)) {
        c = vec3<f32>(0.0);
    }
    if (skyu.taa.x < 0.5 || skyu.taa.y < 0.5) {
        return rs_out(c, dist);
    }
    // Where this pixel was last frame.
    let v = textureLoad(rs_vel, p, 0).xy;
    var prev_uv: vec2<f32>;
    var moving = 0.0;
    if (v.x > 1000.0) {
        let pc = skyu.prev_vp * vec4<f32>(skyu.eye.xyz + rel, 1.0);
        prev_uv = vec2<f32>(pc.x / pc.w * 0.5 + 0.5, 0.5 - pc.y / pc.w * 0.5);
        if (SF_CAM_RESOLVE) {
            // (`--camera-resolve`) the camera's motion of the point alone, without this frame's
            // jitter: `rel` is the point the jittered pixel shows, and `prev_vp` (unjittered)
            // puts it half a pixel or so off the pixel even when nothing moved; read there, the
            // history was resampled at a random sub-pixel offset every frame and blurred. The
            // agents' velocity below is unjittered at both ends already.
            let cc = skyu.vp * vec4<f32>(skyu.eye.xyz + rel, 1.0);
            let cur_uv = vec2<f32>(cc.x / cc.w * 0.5 + 0.5, 0.5 - cc.y / cc.w * 0.5);
            prev_uv = uv + (prev_uv - cur_uv);
        }
    } else {
        prev_uv = uv - v;
        moving = 1.0;
    }
    if (any(prev_uv < vec2<f32>(0.0)) || any(prev_uv > vec2<f32>(1.0))) {
        return rs_out(c, dist);
    }
    // Neighbourhood moments (YCoCg, range-compressed).
    var m1 = vec3<f32>(0.0);
    var m2 = vec3<f32>(0.0);
    for (var k = 0; k < 9; k = k + 1) {
        let o = vec2<i32>(k % 3 - 1, k / 3 - 1);
        // a neighbour on the other side of a sky / geometry edge takes its
        // own volume (with the atmosphere tables a sky pixel's colour is the
        // volume's, not the scene's dome): the clipping box made with the centre's
        // volume let the old dome's blue into the history round every tree and roof.
        var vn = vol;
        if (skyu.flags.w == 1u) {
            let dn = textureLoad(rs_depth, clamp(p + o, vec2<i32>(0), dims - 1), 0);
            if ((dn > 0.0) != (d > 0.0)) {
                vn = rs_volume((vec2<f32>(p + o) + 0.5) / vec2<f32>(dims), select(-1.0, dist, dn > 0.0));
            }
        }
        let s = rs_ycocg(rs_tm(rs_scene(p + o, dims, vn) + lit));
        m1 = m1 + s;
        m2 = m2 + s * s;
    }
    m1 = m1 / 9.0;
    let sigma = sqrt(max(m2 / 9.0 - m1 * m1, vec3<f32>(0.0)));
    // the pixel's motion across the screen (px; with `--camera-resolve` the jitter is not motion).
    let speed = length((uv - prev_uv) * vec2<f32>(dims));
    var gamma = mix(1.25, 0.75, moving);
    if (SF_CAM_RESOLVE) {
        // (`--camera-resolve`) a still pixel's box widened: an edge, a wire or a frond's tip
        // thinner than a pixel is in this frame's sample or not as the jitter moves, and a box
        // from this frame alone clipped the history back to whichever side the sample fell
        // on (a stair-stepped edge); past half a pixel of motion the box is the model's again.
        gamma = mix(mix(CAM_RESOLVE_GAMMA, 1.25, smoothstep(0.05, 0.5, speed)), 0.75, moving);
    }
    let lo = m1 - gamma * sigma;
    let hi = m1 + gamma * sigma;
    var h = rs_ycocg(rs_tm(rs_history(prev_uv)));
    // Clip toward the neighbourhood mean (variance clipping, Salvi 2016).
    let ctr = 0.5 * (lo + hi);
    let ext = max(0.5 * (hi - lo), vec3<f32>(1e-5));
    let off = h - ctr;
    let ts = abs(off / ext);
    let mx = max(ts.x, max(ts.y, ts.z));
    if (mx > 1.0) {
        h = ctr + off / mx;
    }
    let cur = rs_ycocg(rs_tm(c));
    // More of the new frame while the pixel moves fast across the screen.
    var a = mix(skyu.taa.z, 0.35, clamp(speed / 40.0, 0.0, 1.0) * 0.5 + moving * 0.15);
    if (SF_CAM_RESOLVE) {
        // (`--camera-resolve`) the first frames after a cut averaged evenly (frame n weighs
        // 1 / (n + 1)) until the steady weight takes over: the first, unresolved frame no
        // longer stands for a tenth of the picture twenty frames on.
        a = max(a, 1.0 / (skyu.taa.y + 1.0));
    }
    let outc = rs_itm(rs_rgb(mix(h, cur, a)));
    return rs_out(max(outc, vec3<f32>(0.0)), dist);
}
