// Ray-traced reflections (`--rt-reflect`, rt_reflect.rs): three compute stages
// at half the output's resolution on the city's opaque depth and its G-buffer
// (rt_composite.rs), composited in the same frame (rt_composite.wgsl).
// rt_scene.rs `module_source` prepends the ray-query extension, the look's
// common code, the street constants and the traced scene (rt_scene.wgsl,
// group 0).
//
//  * `rr_trace`: the surface's reflection lobe from the G-buffer (the vertex
//    normal, the roughness, what multiplies a reflected radiance), one ray a
//    texel drawn from the GGX distribution of visible normals (a mirror's is
//    the mirror direction), into the traced scene. A ray that hits takes the
//    hit's Lambert light (rt_scene.wgsl `rt_hit_light`); one that leaves the
//    scene takes the sky along it (look_common.wgsl `lk_sky_sample`, the
//    analytic sky the rig reflects) x the surface's environment class, as the
//    rig's reflection scales its sky. Surfaces rougher than `limits.z`, or
//    whose reflection the G-buffer weighs at nothing, are left to the rig.
//  * `rr_temporal`: the previous frame's history at the surface point,
//    reprojected with the previous camera and refused where its depth or
//    normal is another surface's, over at most a number of frames that grows
//    with the roughness (a mirror keeps none: its one ray is exact).
//  * `rr_atrous`: an edge-aware a-trous filter whose reach grows with the
//    roughness (a mirror is not filtered), stopped by the point's plane and
//    normal, the roughness and the luminance against the history's spread.

struct RtRefl {
    // xyz the eye, w the frame index
    cam: vec4<f32>,
    // xyz forward, w the ray's reach (m)
    fwd: vec4<f32>,
    prev_vp: mat4x4<f32>,
    // w 1: a history is usable
    prev_cam: vec4<f32>,
    prev_fwd: vec4<f32>,
    // x origin offset (m), y per metre of view depth, z the sky's share at a
    // hit, w the most frames a rough surface's history averages
    params: vec4<f32>,
    // x the fade's start (m), y its end, z the roughest surface traced, w a
    // rough sample's luminance clamp
    limits: vec4<f32>,
    // x volumetric clouds on (1/0), y physical sky LUT on (1/0), z sun beam (0..1), w spare
    sky_flags: vec4<f32>,
    // half-resolution size (xy), full-resolution size (zw)
    size: vec4<u32>,
};

@group(1) @binding(0) var<uniform> rr: RtRefl;
@group(1) @binding(1) var rr_depth: texture_depth_2d;
@group(1) @binding(2) var rr_gb_n: texture_2d<f32>;
@group(1) @binding(3) var rr_gb_c: texture_2d<f32>;
@group(1) @binding(4) var rr_gb_d: texture_2d<f32>;
@group(1) @binding(5) var rr_cloud_refl: texture_2d<f32>;
@group(1) @binding(6) var rr_atmo_view: texture_2d<f32>;
@group(1) @binding(7) var rr_sky_samp: sampler;

// The stage's own targets (group 2): trace 4 5; temporal 0 1 2 3 8, 4 5;
// a-trous 0 1 2, 4 6 7.
@group(2) @binding(0) var rr_in_a: texture_2d<f32>;
@group(2) @binding(1) var rr_in_b: texture_2d<f32>;
@group(2) @binding(2) var rr_in_c: texture_2d<f32>;
@group(2) @binding(3) var rr_in_d: texture_2d<f32>;
@group(2) @binding(4) var rr_out_a: texture_storage_2d<rgba16float, write>;
@group(2) @binding(5) var rr_out_b: texture_storage_2d<rgba32float, write>;
@group(2) @binding(6) var rr_out_c: texture_storage_2d<r32float, write>;
@group(2) @binding(7) var<uniform> rr_step: vec4<u32>;
@group(2) @binding(8) var rr_in_e: texture_2d<f32>;

const RR_NONE: f32 = -1.0;
// below this roughness a surface is a mirror: no history, no filter.
const RR_MIRROR: f32 = 0.06;
// the filter's luminance stop: standard deviations of the history.
const RR_LUMA_SIGMA: f32 = 3.0;

fn rr_pos(px: vec2<i32>) -> vec4<f32> {
    let dims = vec2<i32>(rr.size.zw);
    let p = clamp(px, vec2<i32>(0), dims - 1);
    let d = textureLoad(rr_depth, p, 0);
    let uv = (vec2<f32>(p) + 0.5) / vec2<f32>(dims);
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    let w = look.inv_view_proj * ndc;
    return vec4<f32>(w.xyz / w.w, d);
}

// The GGX lobe's sampling (alpha = roughness^2, as look_light.wgsl `lk_ggx`):
// a microfacet normal from the distribution of visible normals (Heitz 2018)
// round n seen from v, for u in [0, 1)^2.
fn rr_vndf(v: vec3<f32>, n: vec3<f32>, a: f32, u: vec2<f32>) -> vec3<f32> {
    let tb = rt_basis(n);
    let ve = vec3<f32>(dot(v, tb[0]), dot(v, tb[1]), dot(v, n));
    let vh = normalize(vec3<f32>(a * ve.x, a * ve.y, ve.z));
    let lensq = vh.x * vh.x + vh.y * vh.y;
    let t1v = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(-vh.y, vh.x, 0.0) * inverseSqrt(max(lensq, 1e-12)), lensq > 1e-12);
    let t2v = cross(vh, t1v);
    let r = sqrt(u.x);
    let phi = 6.2831853 * u.y;
    let t1 = r * cos(phi);
    let s = 0.5 * (1.0 + vh.z);
    let t2 = (1.0 - s) * sqrt(max(1.0 - t1 * t1, 0.0)) + s * r * sin(phi);
    let nh = t1 * t1v + t2 * t2v + sqrt(max(0.0, 1.0 - t1 * t1 - t2 * t2)) * vh;
    let ne = normalize(vec3<f32>(a * nh.x, a * nh.y, max(nh.z, 1e-6)));
    return normalize(tb[0] * ne.x + tb[1] * ne.y + n * ne.z);
}

// Seamless hemi-octahedral UV matching sky_clouds.wgsl::vc_hemi_oct_decode.
fn rr_hemi_oct(d: vec3<f32>) -> vec2<f32> {
    let du = normalize(vec3<f32>(d.x, max(d.y, 0.004), d.z));
    let p = du.xz / max(abs(du.x) + abs(du.y) + abs(du.z), 1e-5);
    return vec2<f32>(p.x + p.y, p.x - p.y) * 0.5 + 0.5;
}

// Physical sky LUT + live volumetric cloud reflection dome + horizon/Bay fallback
// for rays that escape the traced BVH scene.
fn rr_miss_sky(l: vec3<f32>, rough: f32, env_i: f32) -> vec3<f32> {
    let up = normalize(vec3<f32>(l.x, max(l.y, 0.008), l.z) + vec3<f32>(1e-5, 0.0, 0.0));
    let phys = clamp(look.hemi_sky.w, 0.0, 1.0) * rr.sky_flags.y;
    var sky = lk_sky_sample(up) * env_i;
    if (phys > 0.001) {
        // Scale the normalized physical sky-view LUT to match `look.sh` upper-hemisphere radiance
        // (matching `look_light.wgsl::lk_phys_sky` without needing a separate Sky uniform buffer).
        let sun_d = look.sun.xyz;
        let raw_dir = textureSampleLevel(rr_atmo_view, rr_sky_samp, at_view_uv(up, sun_d), 0.0).rgb;
        let raw_zen = textureSampleLevel(rr_atmo_view, rr_sky_samp, at_view_uv(vec3<f32>(0.0, 1.0, 0.0), sun_d), 0.0).rgb;
        let sh_hz = normalize(vec2<f32>(sun_d.x, sun_d.z) + vec2<f32>(1e-5, 0.0));
        let raw_s = textureSampleLevel(rr_atmo_view, rr_sky_samp, at_view_uv(vec3<f32>(sh_hz.x * 0.707, 0.707, sh_hz.y * 0.707), sun_d), 0.0).rgb;
        let raw_o = textureSampleLevel(rr_atmo_view, rr_sky_samp, at_view_uv(vec3<f32>(-sh_hz.x * 0.707, 0.707, -sh_hz.y * 0.707), sun_d), 0.0).rgb;
        let raw_mean = max(lk_luma(raw_zen * 0.36 + (raw_s + raw_o) * 0.32), 1e-6);
        let sh_up = lk_luma(rt_sh(vec3<f32>(0.0, 1.0, 0.0)) / LK_PI);
        let phys_sky = at_spectral(raw_dir * (sh_up / raw_mean), 1.0) * env_i;
        sky = mix(sky, phys_sky, phys);
    }
    if (rr.sky_flags.x > 0.5 && look.sun.y > -0.06) {
        let mip = clamp((rough - 0.035) * 6.5, 0.0, 3.5);
        let cl_up = normalize(vec3<f32>(up.x, max(up.y, 0.04) + 0.035 * (1.0 - smoothstep(0.0, 0.28, up.y)), up.z));
        let cr = textureSampleLevel(rr_cloud_refl, rr_sky_samp, rr_hemi_oct(cl_up), mip);
        let comp = sky * (1.0 - clamp(cr.a, 0.0, 1.0)) + max(cr.rgb, vec3<f32>(0.0)) * env_i;
        let base_l = lk_luma(sky);
        let comp_l = lk_luma(comp);
        sky = comp * min(1.0, (base_l * 7.5 + 1e-4) / max(comp_l, 1e-6));
    }
    if (look.sun.y > -0.02) {
        let cs = max(dot(up, look.sun.xyz), 0.0);
        let beam = rr.sky_flags.z * smoothstep(-0.02, 0.08, look.sun.y);
        let corona = (pow(cs, 16.0) * 0.28 + pow(cs, 84.0) * 0.90) * beam * (1.0 - clamp(rough * 2.2, 0.0, 0.85));
        sky = sky + look.key.xyz * (corona * 0.10);
    }
    let night_w = 1.0 - smoothstep(-0.08, 0.02, look.sun.y);
    if (night_w > 0.01) {
        // Preserve deep midnight-indigo night sky reflections in RT glass/wet streets
        // (avoiding flat grey/brown sky fallback when night sky LUT attenuates).
        let inv_ex = 1.0 / max(look.key.w, 0.22);
        let indigo_n = vec3<f32>(0.010, 0.023, 0.068) * (0.42 + 0.58 * smoothstep(0.0, 0.65, up.y)) * env_i * inv_ex;
        sky = mix(sky, max(sky * vec3<f32>(0.85, 0.94, 1.18), indigo_n), night_w * 0.85);
    }
    if (l.y >= 0.03) {
        return sky;
    }
    let below = (rt_sh(l) / LK_PI) * env_i * mix(vec3<f32>(0.62, 0.68, 0.76), vec3<f32>(0.42, 0.56, 0.88), night_w);
    return mix(below, sky, smoothstep(-0.04, 0.03, l.y));
}

// ---------------------------------------------------------------------------
// Stage 1: trace. Writes rr_out_a (the reflected radiance, a the hit
// distance) and rr_out_b (normal, view depth; RR_NONE where nothing traced).
// ---------------------------------------------------------------------------

@compute @workgroup_size(8, 8)
fn rr_trace(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= rr.size.x || gid.y >= rr.size.y) {
        return;
    }
    let hp = vec2<i32>(gid.xy);
    let px = min(hp * 2, vec2<i32>(rr.size.zw) - 1);
    let gn = textureLoad(rr_gb_n, px, 0);
    let gc = textureLoad(rr_gb_c, px, 0);
    let gd = textureLoad(rr_gb_d, px, 0);
    let c = rr_pos(px);
    let vd = dot(c.xyz, rr.fwd.xyz);
    let rough = gc.a;
    if (gn.a < 0.5 || c.w <= 0.0 || vd >= rr.limits.y || rough > rr.limits.z || max(gd.r, max(gd.g, gd.b)) < 1e-3) {
        textureStore(rr_out_a, hp, vec4<f32>(0.0));
        textureStore(rr_out_b, hp, vec4<f32>(0.0, 1.0, 0.0, RR_NONE));
        return;
    }
    let v = normalize(-c.xyz);
    var n = normalize(gn.xyz * 2.0 - 1.0);
    if (dot(n, v) < 0.0) {
        n = -n; // a two-sided face seen from behind
    }
    var l = reflect(-v, n);
    if (rough >= RR_MIRROR) {
        let h = rr_vndf(v, n, rough * rough, rt_rand2(gid.xy, u32(rr.cam.w), 0x5bd1e995u));
        let lh = reflect(-v, h);
        if (dot(lh, n) > 1e-3) {
            l = lh;
        }
    }
    let world = c.xyz + rr.cam.xyz;
    let o = world + n * (rr.params.x + rr.params.y * vd);
    let hit = rt_hit(o, l, 0.0, rr.fwd.w);
    var rad: vec3<f32>;
    var t = rr.fwd.w;
    if (hit.hit) {
        rad = rt_hit_light(hit, o, l, rr.params.z, 2.0 * rr.fwd.w);
        t = hit.t;
    } else {
        rad = rr_miss_sky(l, rough, gd.a);
    }
    // Exposure-aware firefly suppression (tighter on rough stochastic lobes, generous safety ceiling on mirrors).
    let exp_cap = min(rr.limits.w, 8.0 / max(look.key.w, 0.15));
    let cap = select(exp_cap * 3.0, exp_cap, rough >= RR_MIRROR);
    let y = lk_luma(rad);
    if (y > cap) {
        rad = rad * (cap / max(y, 1e-6));
    }
    textureStore(rr_out_a, hp, vec4<f32>(rad, t));
    textureStore(rr_out_b, hp, vec4<f32>(n, vd));
}

// ---------------------------------------------------------------------------
// Stage 2: temporal. In: a this frame's sample, b this frame's normal and
// depth, c the previous frame's, d the previous history, e its moments
// (frames, luminance mean, mean square). Out: a the history, b the moments.
// ---------------------------------------------------------------------------

@compute @workgroup_size(8, 8)
fn rr_temporal(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= rr.size.x || gid.y >= rr.size.y) {
        return;
    }
    let hp = vec2<i32>(gid.xy);
    let cur = textureLoad(rr_in_a, hp, 0);
    let g = textureLoad(rr_in_b, hp, 0);
    if (g.w < 0.0) {
        textureStore(rr_out_a, hp, vec4<f32>(0.0));
        textureStore(rr_out_b, hp, vec4<f32>(0.0));
        return;
    }
    let px = min(hp * 2, vec2<i32>(rr.size.zw) - 1);
    let rough = textureLoad(rr_gb_c, px, 0).a;
    let most = mix(1.0, rr.params.w, smoothstep(RR_MIRROR, 0.35, rough));
    var hist = vec4<f32>(0.0);
    var mom = vec4<f32>(0.0);
    if (rr.prev_cam.w > 0.5 && most > 1.5) {
        let c = rr_pos(px);
        let world = c.xyz + rr.cam.xyz;
        let clip = rr.prev_vp * vec4<f32>(world, 1.0);
        if (clip.w > 1e-4) {
            let uv = vec2<f32>(clip.x / clip.w * 0.5 + 0.5, 0.5 - clip.y / clip.w * 0.5);
            let dims = vec2<i32>(rr.size.xy);
            let f = uv * vec2<f32>(dims) - 0.5;
            let base = vec2<i32>(floor(f));
            let fr = f - floor(f);
            let expect = dot(world - rr.prev_cam.xyz, rr.prev_fwd.xyz);
            var wsum = 0.0;
            for (var k = 0; k < 4; k++) {
                let o = vec2<i32>(k & 1, k >> 1);
                let q = base + o;
                if (any(q < vec2<i32>(0)) || any(q >= dims)) {
                    continue;
                }
                let pg = textureLoad(rr_in_c, q, 0);
                if (pg.w < 0.0 || abs(pg.w - expect) > 0.03 * expect + 0.1 || dot(pg.xyz, g.xyz) < 0.9) {
                    continue;
                }
                let bw = select(1.0 - fr.x, fr.x, o.x == 1) * select(1.0 - fr.y, fr.y, o.y == 1);
                hist = hist + textureLoad(rr_in_d, q, 0) * bw;
                mom = mom + textureLoad(rr_in_e, q, 0) * bw;
                wsum = wsum + bw;
            }
            if (wsum > 0.05) {
                hist = hist / wsum;
                mom = mom / wsum;
                // 3x3 variance-guided neighborhood clamp on rough reflections to kill disocclusion trails & fireflies.
                var m1 = vec3<f32>(0.0);
                var m2 = vec3<f32>(0.0);
                var cnt = 0.0;
                for (var dy = -1; dy <= 1; dy++) {
                    for (var dx = -1; dx <= 1; dx++) {
                        let nq = clamp(hp + vec2<i32>(dx, dy), vec2<i32>(0), dims - 1);
                        let ng = textureLoad(rr_in_b, nq, 0);
                        if (ng.w >= 0.0 && abs(ng.w - g.w) < 0.06 * g.w + 0.25 && dot(ng.xyz, g.xyz) > 0.85) {
                            let s = textureLoad(rr_in_a, nq, 0).rgb;
                            m1 = m1 + s;
                            m2 = m2 + s * s;
                            cnt = cnt + 1.0;
                        }
                    }
                }
                if (cnt >= 3.0) {
                    let mean = m1 / cnt;
                    let sigma = sqrt(max(m2 / cnt - mean * mean, vec3<f32>(0.0)));
                    hist = vec4<f32>(clamp(hist.rgb, max(mean - sigma * 2.25, vec3<f32>(0.0)), mean + sigma * 2.25), hist.a);
                }
            } else {
                hist = vec4<f32>(0.0);
                mom = vec4<f32>(0.0);
            }
        }
    }
    let n = min(mom.x + 1.0, most);
    let y = lk_luma(cur.rgb);
    textureStore(rr_out_a, hp, mix(hist, cur, 1.0 / n));
    textureStore(rr_out_b, hp, vec4<f32>(n, mix(mom.yz, vec2<f32>(y, y * y), 1.0 / n), 0.0));
}

// ---------------------------------------------------------------------------
// Stage 3: a-trous. In: a the history (or the previous pass), b this frame's
// normal and depth, c this frame's moments. rr_step: x the step, y 1 on the
// last pass, z the pass. Out: a the filtered result; the last pass also
// writes c, the view depth (RR_NONE where there is no result).
// ---------------------------------------------------------------------------

fn rr_ray_rel(hp: vec2<i32>, vd: f32) -> vec3<f32> {
    let dims = vec2<f32>(rr.size.zw);
    let uv = (vec2<f32>(hp * 2) + 0.5) / dims;
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 1.0, 1.0);
    let w = look.inv_view_proj * ndc;
    let dir = w.xyz / w.w;
    return dir * (vd / max(dot(dir, rr.fwd.xyz), 1e-6));
}

@compute @workgroup_size(8, 8)
fn rr_atrous(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= rr.size.x || gid.y >= rr.size.y) {
        return;
    }
    let hp = vec2<i32>(gid.xy);
    let last = rr_step.y == 1u;
    let g0 = textureLoad(rr_in_b, hp, 0);
    let c0 = textureLoad(rr_in_a, hp, 0);
    if (last) {
        textureStore(rr_out_c, hp, vec4<f32>(g0.w));
    }
    if (g0.w < 0.0) {
        textureStore(rr_out_a, hp, vec4<f32>(0.0));
        return;
    }
    let px = min(hp * 2, vec2<i32>(rr.size.zw) - 1);
    let r0 = textureLoad(rr_gb_c, px, 0).a;
    // the filter's reach: none on a mirror, the whole kernel from roughness 0.4.
    let reach = smoothstep(RR_MIRROR, 0.4, r0);
    let step = i32(rr_step.x);
    if (step == 0 || reach <= 0.0) {
        textureStore(rr_out_a, hp, c0);
        return;
    }
    let dims = vec2<i32>(rr.size.xy);
    let p0 = rr_ray_rel(hp, g0.w);
    let kern = array<f32, 5>(0.0625, 0.25, 0.375, 0.25, 0.0625);
    let tol = 0.25 * (1.0 + 0.02 * g0.w) * f32(step);
    let m0 = textureLoad(rr_in_c, hp, 0);
    let sd = sqrt(max(m0.z - m0.y * m0.y, 0.0)) / (1.0 + f32(rr_step.z));
    let y0 = lk_luma(c0.rgb);
    let lstop = RR_LUMA_SIGMA * sd + 0.2 * max(y0, 1e-3);
    var sum = vec4<f32>(0.0);
    var wsum = 0.0;
    for (var dy = -2; dy <= 2; dy++) {
        for (var dx = -2; dx <= 2; dx++) {
            let q = hp + vec2<i32>(dx, dy) * step;
            if (any(q < vec2<i32>(0)) || any(q >= dims)) {
                continue;
            }
            let gq = textureLoad(rr_in_b, q, 0);
            if (gq.w < 0.0) {
                continue;
            }
            let rq = textureLoad(rr_gb_c, min(q * 2, vec2<i32>(rr.size.zw) - 1), 0).a;
            let wn = pow(max(dot(g0.xyz, gq.xyz), 0.0), 64.0);
            let pq = rr_ray_rel(q, gq.w);
            let wp = exp(-abs(dot(pq - p0, g0.xyz)) / tol);
            let wr = exp(-abs(rq - r0) * 12.0);
            let cq = textureLoad(rr_in_a, q, 0);
            let wl = exp(-abs(lk_luma(cq.rgb) - y0) / lstop);
            let centre = dx == 0 && dy == 0;
            let k = select(kern[dx + 2] * kern[dy + 2] * reach, kern[2] * kern[2], centre);
            let w = k * wn * wp * wr * wl;
            sum = sum + cq * w;
            wsum = wsum + w;
        }
    }
    textureStore(rr_out_a, hp, select(c0, sum / wsum, wsum > 1e-6));
}
