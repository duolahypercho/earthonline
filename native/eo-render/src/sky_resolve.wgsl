// the resolve (appended to look_common.wgsl). One full-screen pass from
// the HDR scene to the resolved HDR frame: the atmosphere volume composited by
// depth (a bilateral upsample of the half-resolution result), then temporal
// anti-aliasing: the history reprojected by the moving agents' own motion
// (the velocity target) or else by the camera's motion and the depth,
// 5-tap Catmull-Rom history, variance clipping in YCoCg against the 3 x 3
// neighbourhood, luminance-weighted blend. The result is the frame post
// tone-maps, the next frame's history, and the previous frame the wet
// ground's reflection reads.

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
        if (ang < r + px) {
            let vd = vec2<i32>(textureDimensions(rs_volz));
            let q = clamp(vec2<i32>(uv * vec2<f32>(vd)), vec2<i32>(0), vd - 1);
            // x the weather's beam (a disc showed through the rain deck)
            // at least 0.35 through a thin low bank on a clear day (the sun setting into the
            // offshore bank at 19.1 h drew no disc at all: 1.15 x the sky round it)
            // The share of the disc that reaches the eye, at least 0.35 under anything short of a
            // continuous deck (a low bank or a broken cumulus field still shows a reddened disc).
            let td = max(clamp(-textureLoad(rs_volz, q, 0).r - 1.0, 0.0, 1.0), 0.35 * (1.0 - smoothstep(0.75, 0.9, skyu.cloud0.z))) * smoothstep(0.15, 0.6, skyu.cloud1.w);
            let x = clamp(ang / r, 0.0, 1.0);
            let limb = 1.0 - 0.6 * (1.0 - sqrt(max(1.0 - x * x, 0.0)));
            // white-hot over a few degrees of altitude (the look's tinted disc tone-mapped
            // to the same cream as the glare round it), x 3 the look's radiance
            // its colour is the sun's own transmittance to the eye x 230 (about 3 x the
            // look's disc at noon), so only the air dims and reddens it: the look's disc fades
            // out between +0.3 and -0.6 deg and there was no sun on the sea at sunset. Kept
            // until its upper limb has set (the centre's ray grazing the horizon).
            let tr = at_sun(rs_trans, rs_lin, skyu.eye.y, max(sin(el_s), 0.0005)) * 230.0;
            let dc = mix(tr, vec3<f32>(lk_luma(tr)), 0.75 * smoothstep(0.0, 0.08, look.sun.y)) * smoothstep(-0.0075, -0.002, el_s);
            // at least 40 x the sky round it, its hue kept (at sunset the air's
            // transmittance alone left a disc no brighter than its glow: a smudge, not a sun).
            let dl = lk_luma(dc);
            // At least 80 x the sky round it (a disc on the horizon is 10-100 x the sky beside it,
            // so it clips with a hard limb), and toward white in its middle: the clipped core of a
            // low sun is yellow-white or orange-white, its colour held at the limb.
            var dc2 = dc * (max(dl, 80.0 * lk_luma(c)) / max(dl, 1e-6));
            let pk = max(max(dc2.r, dc2.g), dc2.b);
            dc2 = mix(dc2, vec3<f32>(pk), 0.55 * (1.0 - smoothstep(0.3, 0.9, x)));
            c = c + select(dc2, vec3<f32>(0.0), dl <= 0.0) * limb * smoothstep(r + px, r - px, ang) * td;
        }
    }
    if (any(c != c)) {
        c = vec3<f32>(0.0);
    }
    if (skyu.taa.x < 0.5 || skyu.taa.y < 0.5) {
        return vec4<f32>(c, 1.0);
    }
    // Where this pixel was last frame.
    let v = textureLoad(rs_vel, p, 0).xy;
    var prev_uv: vec2<f32>;
    var moving = 0.0;
    if (v.x > 1000.0) {
        let pc = skyu.prev_vp * vec4<f32>(skyu.eye.xyz + rel, 1.0);
        prev_uv = vec2<f32>(pc.x / pc.w * 0.5 + 0.5, 0.5 - pc.y / pc.w * 0.5);
    } else {
        prev_uv = uv - v;
        moving = 1.0;
    }
    if (any(prev_uv < vec2<f32>(0.0)) || any(prev_uv > vec2<f32>(1.0))) {
        return vec4<f32>(c, 1.0);
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
        let s = rs_ycocg(rs_tm(rs_scene(p + o, dims, vn)));
        m1 = m1 + s;
        m2 = m2 + s * s;
    }
    m1 = m1 / 9.0;
    let sigma = sqrt(max(m2 / 9.0 - m1 * m1, vec3<f32>(0.0)));
    let gamma = mix(1.25, 0.75, moving);
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
    let speed = length((uv - prev_uv) * vec2<f32>(dims));
    let a = mix(skyu.taa.z, 0.35, clamp(speed / 40.0, 0.0, 1.0) * 0.5 + moving * 0.15);
    let outc = rs_itm(rs_rgb(mix(h, cur, a)));
    return vec4<f32>(max(outc, vec3<f32>(0.0)), 1.0);
}
