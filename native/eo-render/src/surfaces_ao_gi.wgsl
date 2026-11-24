// (`--screen-gi`, SF_SCREEN_GI; eo-look `screen_gi`) the occlusion pass as a
// visibility-bitmask gather (Therrien, Levesque and Gilet 2023) out to
// GI_RADIUS_M, blended into a reprojected history. Appended to
// surfaces_ao.wgsl (its bindings, `ao_pos`, `ao_arc`, the sun bounce) only
// when the feature is compiled in; `ao_gi_main` writes the raw + history
// blend to binding 3 (the pass's scratch target) and the view depth to
// binding 4, and surfaces_ao_gi_filter.wgsl filters it into the texture the
// lit shader reads. Same outputs as `ao_main`: rgb the bounce (hidden shares
// x the radiance that hides them), a the visible share through AO_INTENSITY.

@group(0) @binding(6) var gi_hist: texture_2d<f32>;
@group(0) @binding(7) var gi_hist_z: texture_2d<f32>;
@group(0) @binding(8) var gi_samp: sampler;

// The cumulative cosine-weighted share of the slice from its side -1 edge
// (n - pi/2) to the angle h (eo-look `cumulative`; `lo` the side -1 arc,
// `inv_total` 1 / the slice's whole arc).
fn gi_cum(h: f32, n: f32, lo: f32, inv_total: f32) -> f32 {
    var c = lo + ao_arc(h, n);
    if (h <= 0.0) {
        c = lo - ao_arc(h, n);
    }
    return clamp(c * inv_total, 0.0, 1.0);
}

// The sectors between two cumulative coordinates (eo-look `sector_bits`).
fn gi_bits(u0: f32, u1: f32) -> u32 {
    let a = u32(clamp(round(min(u0, u1) * f32(GI_SECTORS)), 0.0, f32(GI_SECTORS)));
    let b = u32(clamp(round(max(u0, u1) * f32(GI_SECTORS)), 0.0, f32(GI_SECTORS)));
    if (b <= a) {
        return 0u;
    }
    let k = b - a;
    var m = 0xFFFFFFFFu;
    if (k < 32u) {
        m = (1u << k) - 1u;
    }
    return m << a;
}

fn gi_luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// A sample's radiance as a bounce: under the base pass's ceiling, and by day
// at most a white diffuse surface's under the whole rig (eo-look `diffuse_cap`:
// a glint is light sent one way, not to the point).
fn gi_radiance(sp: vec2<i32>, dims: vec2<i32>) -> vec3<f32> {
    var rad = min(textureLoad(ao_hdr_in, clamp(sp, vec2<i32>(0), dims - 1), 0).rgb, vec3<f32>(AO_RADIANCE_CAP));
    let fill = gi_luma(look.hemi_sky.rgb) + gi_luma(look.hemi_ground.rgb) + gi_luma(look.ambient.rgb) + gi_luma(look.rim.rgb);
    let day = lk_smooth(-0.05, 0.05, look.sun.y);
    let cap = mix(AO_RADIANCE_CAP, GI_MAX_DIFFUSE_ALBEDO * (gi_luma(look.key.rgb) + fill) / LK_PI, day);
    let l = gi_luma(rad);
    if (l > cap) {
        rad = rad * (cap / l);
    }
    return rad;
}

// The history at a camera-relative point: the previous result reprojected
// with the camera it was made with (look.ao_vp), refused where the depth it
// recorded is not this surface's (as `ao_fetch`). w < 0: none.
fn gi_history(world: vec3<f32>) -> vec4<f32> {
    let none = vec4<f32>(0.0, 0.0, 0.0, -1.0);
    if (look.ao_params.x < 0.5) {
        return none;
    }
    let clip = look.ao_vp * vec4<f32>(world, 1.0);
    if (clip.w <= 1e-4) {
        return none;
    }
    let uv = vec2<f32>(clip.x / clip.w * 0.5 + 0.5, 0.5 - clip.y / clip.w * 0.5);
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) {
        return none;
    }
    let dims = vec2<i32>(textureDimensions(gi_hist_z));
    let tp = clamp(vec2<i32>(uv * vec2<f32>(dims)), vec2<i32>(0), dims - 1);
    let z = textureLoad(gi_hist_z, tp, 0).r;
    let expect = dot(world - look.ao_cam.xyz, look.ao_fwd.xyz);
    if (z < 0.0 || abs(z - expect) > 0.04 * expect + 0.12) {
        return none;
    }
    return textureSampleLevel(gi_hist, gi_samp, uv, 0.0);
}

fn gi_store(hp: vec2<i32>, raw: vec4<f32>, world: vec3<f32>) {
    let h = gi_history(world);
    var o = raw;
    if (h.w >= 0.0) {
        o = mix(h, raw, GI_HISTORY_WEIGHT);
    }
    textureStore(ao_out, hp, o);
}

@compute @workgroup_size(8, 8)
fn ao_gi_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let half_dims = vec2<i32>(textureDimensions(ao_out));
    let hp = vec2<i32>(gid.xy);
    if (hp.x >= half_dims.x || hp.y >= half_dims.y) {
        return;
    }
    let dims = vec2<i32>(textureDimensions(ao_depth_in));
    let px = hp * 2;
    let c = ao_pos(px, dims);
    let fwd = look.cam_fwd.xyz;
    if (c.w <= 0.0) {
        textureStore(ao_out, hp, vec4<f32>(0.0, 0.0, 0.0, 1.0));
        textureStore(ao_zout, hp, vec4<f32>(-1.0));
        return;
    }
    let P = c.xyz;
    let world = P + look.cam_pos.xyz;
    let vd = dot(P, fwd);
    textureStore(ao_zout, hp, vec4<f32>(vd));
    // Normal from the depth neighbours, the smaller step on each axis (at the
    // frame's edge the clamped neighbour is the point itself: the other one;
    // a NaN here would live on in the history and spread through it).
    let r = ao_pos(px + vec2<i32>(1, 0), dims).xyz - P;
    let l = P - ao_pos(px - vec2<i32>(1, 0), dims).xyz;
    let d = ao_pos(px + vec2<i32>(0, 1), dims).xyz - P;
    let u = P - ao_pos(px - vec2<i32>(0, 1), dims).xyz;
    let dx = select(l, r, (dot(r, r) < dot(l, l) && dot(r, r) > 1e-12) || dot(l, l) <= 1e-12);
    let dy = select(u, d, (dot(d, d) < dot(u, u) && dot(d, d) > 1e-12) || dot(u, u) <= 1e-12);
    let V = normalize(-P);
    let cn = cross(dy, dx);
    var N = V;
    if (dot(cn, cn) > 1e-20) {
        N = normalize(cn);
    }
    if (dot(N, V) < 0.0) {
        N = -N;
    }
    let right = normalize(cross(fwd, vec3<f32>(0.0, 1.0, 0.0)));
    let up = cross(right, fwd);
    let focal = 1.0 / look.ao_params.y;
    let reach = min(GI_RADIUS_M * focal / max(vd, 0.1), GI_MAX_REACH_SHARE * f32(dims.y));
    var sun_b = vec3<f32>(0.0);
    if (SF_SUN_BOUNCE) {
        let sn = fract(52.9829189 * fract(dot(vec2<f32>(hp) + look.post2.w * 5.588238, vec2<f32>(0.00583715, 0.06711056))));
        // the sun bounce leaves the gather's reach to it as it leaves the 4 m search its own
        sb_near_m = GI_RADIUS_M;
        sb_samples = SB_SAMPLES_WITH_HISTORY; // (the history's mean takes the rest)
        sun_b = ao_sun_bounce(world, N, sn * 2.0 * LK_PI, fract(sn * 11.7 + 0.61), reach >= 2.0);
    }
    if (reach < 2.0) {
        gi_store(hp, vec4<f32>(sun_b, 1.0), world);
        return;
    }
    let fi = look.post2.w;
    let noise = fract(52.9829189 * fract(dot(vec2<f32>(hp) + fi * 5.588238, vec2<f32>(0.06711056, 0.00583715))));
    let jitter = fract(noise * 7.13 + 0.37);
    let fade_rnd = fract(noise * 3.61 + 0.73);
    var vis = 0.0;
    var bounce = vec3<f32>(0.0);
    var norm = 0.0;
    for (var s = 0; s < i32(GI_SLICES); s = s + 1) {
        let phi = (f32(s) + noise) * (LK_PI / f32(GI_SLICES));
        let dir = vec2<f32>(cos(phi), sin(phi));
        let D = normalize(right * dir.x - up * dir.y);
        let ortho = D - V * dot(D, V);
        let axis = normalize(cross(ortho, V));
        let pn = N - axis * dot(N, axis);
        let plen = length(pn);
        if (plen < 1e-4) {
            continue;
        }
        let cos_n = clamp(dot(pn / plen, V), -1.0, 1.0);
        let n = sign(dot(ortho, pn)) * acos(cos_n);
        let lo = ao_arc(n - 0.5 * LK_PI, n);
        let total = lo + ao_arc(n + 0.5 * LK_PI, n);
        if (total < 1e-5) {
            continue;
        }
        let inv_total = 1.0 / total;
        var mask = 0u;
        var slice_b = vec3<f32>(0.0);
        for (var side = 0; side < 2; side = side + 1) {
            let sg = select(-1.0, 1.0, side == 1);
            for (var j = 0; j < i32(GI_STEPS); j = j + 1) {
                let t = (f32(j) + jitter) / f32(GI_STEPS);
                let off = dir * sg * (t * t * reach + 1.0);
                let sp = px + vec2<i32>(round(off));
                let S = ao_pos(sp, dims);
                if (S.w <= 0.0) {
                    continue;
                }
                let delta = S.xyz - P;
                let dist = length(delta);
                if (dist < 1e-3) {
                    continue;
                }
                // coplanar detail (paint, a kerb lip seen edge-on) is not an occluder.
                if (dot(delta, N) < 0.03) {
                    continue;
                }
                // the reach fades out (1 - d^2 / R^2 of the samples count, chosen per pixel and frame).
                if (1.0 - (dist * dist) / (GI_RADIUS_M * GI_RADIUS_M) <= fade_rnd) {
                    continue;
                }
                // the sample's front, and its back `thickness` further from the eye.
                let sd = length(S.xyz);
                let thick = max(GI_THICKNESS_M + GI_THICKNESS_PER_M * sd, GI_THICKNESS_SHARE * dist);
                let back = S.xyz * (1.0 + thick / max(sd, 1e-3)) - P;
                let hf = sg * acos(clamp(dot(delta / dist, V), -1.0, 1.0));
                let hb = sg * acos(clamp(dot(normalize(back), V), -1.0, 1.0));
                let lim0 = n - 0.5 * LK_PI;
                let lim1 = n + 0.5 * LK_PI;
                let bits = gi_bits(gi_cum(clamp(hf, lim0, lim1), n, lo, inv_total), gi_cum(clamp(hb, lim0, lim1), n, lo, inv_total));
                let fresh = bits & ~mask;
                if (fresh != 0u) {
                    slice_b = slice_b + (f32(countOneBits(fresh)) / f32(GI_SECTORS)) * gi_radiance(sp, dims);
                    mask = mask | bits;
                }
            }
        }
        let slice_vis = 1.0 - f32(countOneBits(mask)) / f32(GI_SECTORS);
        vis = vis + plen * total * slice_vis;
        bounce = bounce + plen * total * slice_b;
        norm = norm + plen * total;
    }
    if (norm > 1e-4) {
        vis = clamp(vis / norm, 0.0, 1.0);
        bounce = bounce / norm;
    } else {
        vis = 1.0;
    }
    vis = 1.0 - AO_INTENSITY * (1.0 - vis);
    gi_store(hp, vec4<f32>(bounce + sun_b, vis), world);
}
