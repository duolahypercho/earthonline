// Ray-traced one-bounce diffuse light (`--rt-gi`, rt_gi.rs): three compute
// stages at half the output's resolution on the city's opaque depth and its
// G-buffer (written by the city pass, `fs_rtg` in rt_gi_use.wgsl), then the
// deferred composite (rt_gi_composite.wgsl) in the same frame. rt_scene.rs
// `module_source` prepends the ray-query extension, the look's common code,
// the street constants and the traced scene (rt_scene.wgsl, group 0).
//
//  * `rt_trace`: the surface point from the depth and its normal from the
//    G-buffer (the vertex normal the city shaded with), one cosine-distributed
//    ray into the traced scene. A ray that leaves it takes the rig's sky in
//    its direction (the hemisphere light's radiance); a ray that hits takes
//    the hit's Lambert light (rt_scene.wgsl `rt_hit_light`, the rig's sky at
//    the hit x RT_HIT_SKY). Writes the irradiance without the rig's ambient
//    and environment terms (rgb) and whether the ray left the scene (a: the
//    environment's visible share, the composite scales the environment term by
//    it), and the point's normal and view depth.
//  * `rt_temporal`: the previous frame's history at the point, reprojected
//    with the previous camera and refused where its depth or normal is another
//    surface's, blended with this frame's sample over up to RT_MAX_FRAMES
//    frames (a running mean). It also keeps the running moments of a guide
//    value (`rt_guide`: the irradiance the sample stands for, the
//    environment's share at the painted-facade class) for the filter's
//    luminance stop.
//  * `rt_atrous`: an edge-aware a-trous filter (5 x 5 B3 spline at a step of
//    1, 2, 4 texels), its weights stopped by the point's plane and normal and
//    by the guide value against the history's standard deviation (a
//    variance-guided stop: coplanar ground in and out of a deck's shade stays
//    apart); the last pass writes the result and its view depth for the
//    composite.

struct RtTrace {
    // xyz the eye, w the frame index (exact in a float to 2^24)
    cam: vec4<f32>,
    // xyz forward, w ray reach (m)
    fwd: vec4<f32>,
    // the previous frame's camera (world -> clip), its eye (w 1: a history is
    // usable) and its forward
    prev_vp: mat4x4<f32>,
    prev_cam: vec4<f32>,
    prev_fwd: vec4<f32>,
    // x origin offset (m), y origin offset per metre of view depth, z the sky's
    // share at a hit, w the most frames the history averages
    params: vec4<f32>,
    // x the distance the result starts fading out at, y where it is gone, z the
    // filter's normal power, w its plane distance scale (m)
    fade: vec4<f32>,
    // half-resolution size (xy), full-resolution size (zw)
    size: vec4<u32>,
};


// The frame's inputs (group 1): the trace uniform, the city's opaque depth,
// the G-buffer's normal (rgb n x 0.5 + 0.5, a 1 where the city shaded a texel).
@group(1) @binding(0) var<uniform> rt: RtTrace;
@group(1) @binding(1) var rt_depth: texture_depth_2d;
@group(1) @binding(2) var rt_gb_n: texture_2d<f32>;

// The stage's own targets (group 2; rt_gi.rs makes one layout a stage with
// only the bindings its entry point reads): trace 4 5; temporal 0 1 2 3 8, 4 5;
// a-trous 0 1 2, 4 6 7.
@group(2) @binding(0) var rt_in_a: texture_2d<f32>;
@group(2) @binding(1) var rt_in_b: texture_2d<f32>;
@group(2) @binding(2) var rt_in_c: texture_2d<f32>;
@group(2) @binding(3) var rt_in_d: texture_2d<f32>;
@group(2) @binding(4) var rt_out_a: texture_storage_2d<rgba16float, write>;
@group(2) @binding(5) var rt_out_b: texture_storage_2d<rgba32float, write>;
@group(2) @binding(6) var rt_out_c: texture_storage_2d<r32float, write>;
@group(2) @binding(7) var<uniform> rt_step: vec4<u32>;
@group(2) @binding(8) var rt_in_e: texture_2d<f32>;

// a ray in a texel of sky or past the fade: none.
const RT_NONE: f32 = -1.0;
// the filter's guide stop: standard deviations of the history, and a floor (a
// share of the centre's value) so a converged texel still averages its peers.
const RT_LUMA_SIGMA: f32 = 4.0;
const RT_LUMA_FLOOR: f32 = 0.1;

// The surface point from the depth (surfaces_ao.wgsl `ao_pos`), camera-relative.
fn rt_pos(px: vec2<i32>) -> vec4<f32> {
    let dims = vec2<i32>(rt.size.zw);
    let p = clamp(px, vec2<i32>(0), dims - 1);
    let d = textureLoad(rt_depth, p, 0);
    let uv = (vec2<f32>(p) + 0.5) / vec2<f32>(dims);
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    let w = look.inv_view_proj * ndc;
    return vec4<f32>(w.xyz / w.w, d);
}

// The filter's guide: the irradiance a texel's (irradiance, open share) stands
// for at normal n, the environment's share at the painted-facade class.
fn rt_guide(v: vec4<f32>, n: vec3<f32>) -> f32 {
    return lk_luma(v.rgb) + v.a * lk_luma(rt_sh(n) * (look.ambient.w * look.env_class.x));
}

// ---------------------------------------------------------------------------
// Stage 1: trace. Writes rt_out_a (irradiance without ambient and
// environment, the environment's open share) and rt_out_b (normal, view depth).
// ---------------------------------------------------------------------------

@compute @workgroup_size(8, 8)
fn rt_trace(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= rt.size.x || gid.y >= rt.size.y) {
        return;
    }
    let hp = vec2<i32>(gid.xy);
    let px = min(hp * 2, vec2<i32>(rt.size.zw) - 1);
    let gn = textureLoad(rt_gb_n, px, 0);
    let c = rt_pos(px);
    let vd = dot(c.xyz, rt.fwd.xyz);
    if (gn.a < 0.5 || c.w <= 0.0 || vd >= rt.fade.y) {
        // sky, a surface the city did not shade (props, agents: drawn after), or past the fade.
        textureStore(rt_out_a, hp, vec4<f32>(0.0));
        textureStore(rt_out_b, hp, vec4<f32>(0.0, 1.0, 0.0, RT_NONE));
        return;
    }
    let n = normalize(gn.xyz * 2.0 - 1.0);
    let world = c.xyz + rt.cam.xyz;
    let o = world + n * (rt.params.x + rt.params.y * vd);
    let d = rt_cosine(n, rt_rand2(gid.xy, u32(rt.cam.w), 0u));
    let h = rt_hit(o, d, 0.0, rt.fwd.w);
    var l = rt_hemi_radiance(d);
    var open = 1.0;
    if (h.hit) {
        l = rt_hit_light(h, o, d, rt.params.z, 2.0 * rt.fwd.w);
        open = 0.0;
    }
    // one cosine-distributed sample: the irradiance estimate is pi x its radiance,
    // soft-clamped relative to exposure to eliminate single-ray bounce fireflies.
    var irr = l * LK_PI;
    let gi_cap = 14.0 / max(look.key.w, 0.1);
    let gi_lum = lk_luma(irr);
    if (gi_lum > gi_cap) {
        irr = irr * (gi_cap / max(gi_lum, 1e-5));
    }
    textureStore(rt_out_a, hp, vec4<f32>(irr, open));
    textureStore(rt_out_b, hp, vec4<f32>(n, vd));
}

// ---------------------------------------------------------------------------
// Stage 2: temporal. In: a this frame's sample, b this frame's normal and
// depth, c the previous frame's, d the previous history, e its moments
// (frame count, the guide's mean and mean square). Out: a the history, b the
// moments.
// ---------------------------------------------------------------------------

@compute @workgroup_size(8, 8)
fn rt_temporal(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= rt.size.x || gid.y >= rt.size.y) {
        return;
    }
    let hp = vec2<i32>(gid.xy);
    let cur = textureLoad(rt_in_a, hp, 0);
    let g = textureLoad(rt_in_b, hp, 0);
    if (g.w < 0.0) {
        textureStore(rt_out_a, hp, vec4<f32>(0.0));
        textureStore(rt_out_b, hp, vec4<f32>(0.0));
        return;
    }
    var hist = vec4<f32>(0.0);
    var mom = vec4<f32>(0.0);
    if (rt.prev_cam.w > 0.5) {
        let c = rt_pos(hp * 2);
        let world = c.xyz + rt.cam.xyz;
        let clip = rt.prev_vp * vec4<f32>(world, 1.0);
        if (clip.w > 1e-4) {
            let uv = vec2<f32>(clip.x / clip.w * 0.5 + 0.5, 0.5 - clip.y / clip.w * 0.5);
            let dims = vec2<i32>(rt.size.xy);
            let f = uv * vec2<f32>(dims) - 0.5;
            let base = vec2<i32>(floor(f));
            let fr = f - floor(f);
            let expect = dot(world - rt.prev_cam.xyz, rt.prev_fwd.xyz);
            var wsum = 0.0;
            for (var k = 0; k < 4; k++) {
                let o = vec2<i32>(k & 1, k >> 1);
                let q = base + o;
                if (any(q < vec2<i32>(0)) || any(q >= dims)) {
                    continue;
                }
                let pg = textureLoad(rt_in_c, q, 0);
                if (pg.w < 0.0 || abs(pg.w - expect) > 0.03 * expect + 0.1 || dot(pg.xyz, g.xyz) < 0.8) {
                    continue;
                }
                let bw = select(1.0 - fr.x, fr.x, o.x == 1) * select(1.0 - fr.y, fr.y, o.y == 1);
                hist = hist + textureLoad(rt_in_d, q, 0) * bw;
                mom = mom + textureLoad(rt_in_e, q, 0) * bw;
                wsum = wsum + bw;
            }
            if (wsum > 0.05) {
                hist = hist / wsum;
                mom = mom / wsum;
                // 3x3 variance-guided neighborhood clamp to prevent disocclusion ghosting & firefly trails.
                var m1 = vec4<f32>(0.0);
                var m2 = vec4<f32>(0.0);
                var cnt = 0.0;
                for (var dy = -1; dy <= 1; dy++) {
                    for (var dx = -1; dx <= 1; dx++) {
                        let nq = clamp(hp + vec2<i32>(dx, dy), vec2<i32>(0), dims - 1);
                        let ng = textureLoad(rt_in_b, nq, 0);
                        if (ng.w >= 0.0 && abs(ng.w - g.w) < 0.06 * g.w + 0.25 && dot(ng.xyz, g.xyz) > 0.75) {
                            let s = textureLoad(rt_in_a, nq, 0);
                            m1 = m1 + s;
                            m2 = m2 + s * s;
                            cnt = cnt + 1.0;
                        }
                    }
                }
                if (cnt >= 3.0) {
                    let mean = m1 / cnt;
                    let sigma = sqrt(max(m2 / cnt - mean * mean, vec4<f32>(0.0)));
                    hist = clamp(hist, max(mean - sigma * 2.75, vec4<f32>(0.0)), mean + sigma * 2.75);
                }
            } else {
                hist = vec4<f32>(0.0);
                mom = vec4<f32>(0.0);
            }
        }
    }
    let n = min(mom.x + 1.0, rt.params.w);
    let gs = rt_guide(cur, g.xyz);
    textureStore(rt_out_a, hp, mix(hist, cur, 1.0 / n));
    textureStore(rt_out_b, hp, vec4<f32>(n, mix(mom.yz, vec2<f32>(gs, gs * gs), 1.0 / n), 0.0));
}

// ---------------------------------------------------------------------------
// Stage 3: a-trous. In: a the history (or the previous pass), b this frame's
// normal and depth, c this frame's moments. rt_step: x the step, y 1 on the
// last pass, z the pass. Out: a the filtered result; the last pass also
// writes c, the view depth (RT_NONE where there is no result).
// ---------------------------------------------------------------------------

fn rt_ray_rel(hp: vec2<i32>, vd: f32) -> vec3<f32> {
    // the camera-relative point at view depth vd along this texel's ray.
    let dims = vec2<f32>(rt.size.zw);
    let uv = (vec2<f32>(hp * 2) + 0.5) / dims;
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 1.0, 1.0);
    let w = look.inv_view_proj * ndc;
    let dir = w.xyz / w.w;
    return dir * (vd / max(dot(dir, rt.fwd.xyz), 1e-6));
}

@compute @workgroup_size(8, 8)
fn rt_atrous(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= rt.size.x || gid.y >= rt.size.y) {
        return;
    }
    let hp = vec2<i32>(gid.xy);
    let last = rt_step.y == 1u;
    let g0 = textureLoad(rt_in_b, hp, 0);
    let c0 = textureLoad(rt_in_a, hp, 0);
    if (g0.w < 0.0) {
        textureStore(rt_out_a, hp, vec4<f32>(0.0));
        if (last) {
            textureStore(rt_out_c, hp, vec4<f32>(RT_NONE));
        }
        return;
    }
    let step = i32(rt_step.x);
    if (step == 0) {
        textureStore(rt_out_a, hp, c0);
        if (last) {
            textureStore(rt_out_c, hp, vec4<f32>(g0.w));
        }
        return;
    }
    let dims = vec2<i32>(rt.size.xy);
    let p0 = rt_ray_rel(hp, g0.w);
    let kern = array<f32, 5>(0.0625, 0.25, 0.375, 0.25, 0.0625);
    // the plane tolerance grows with the depth and the step (a texel's footprint).
    let tol = rt.fade.w * (1.0 + 0.02 * g0.w) * f32(step);
    // the guide's stop: its history's standard deviation (3 x 3 mean of the
    // variance), shrinking as the passes average (as a filtered variance would);
    // a short history (few frames) stops on the value alone, loosely.
    var var3 = 0.0;
    var n0 = 0.0;
    for (var k = 0; k < 9; k++) {
        let q = clamp(hp + vec2<i32>(k % 3 - 1, k / 3 - 1), vec2<i32>(0), dims - 1);
        let m = textureLoad(rt_in_c, q, 0);
        var3 = var3 + max(m.z - m.y * m.y, 0.0);
        if (k == 4) {
            n0 = m.x;
        }
    }
    let i0 = rt_guide(c0, g0.xyz);
    let sd = sqrt(var3 / 9.0) / (1.0 + f32(rt_step.z));
    let young = 1.0 - lk_smooth(1.0, 6.0, n0);
    let lstop = RT_LUMA_SIGMA * sd + (RT_LUMA_FLOOR + young) * max(i0, 1e-3);
    var sum = vec4<f32>(0.0);
    var wsum = 0.0;
    for (var dy = -2; dy <= 2; dy++) {
        for (var dx = -2; dx <= 2; dx++) {
            let q = hp + vec2<i32>(dx, dy) * step;
            if (any(q < vec2<i32>(0)) || any(q >= dims)) {
                continue;
            }
            let gq = textureLoad(rt_in_b, q, 0);
            if (gq.w < 0.0) {
                continue;
            }
            let wn = pow(max(dot(g0.xyz, gq.xyz), 0.0), rt.fade.z);
            let pq = rt_ray_rel(q, gq.w);
            let dist = abs(dot(pq - p0, g0.xyz));
            let wp = exp(-dist / tol);
            let cq = textureLoad(rt_in_a, q, 0);
            let wl = exp(-abs(rt_guide(cq, gq.xyz) - i0) / lstop);
            let w = kern[dx + 2] * kern[dy + 2] * wn * wp * wl;
            sum = sum + cq * w;
            wsum = wsum + w;
        }
    }
    let r = select(c0, sum / wsum, wsum > 1e-6);
    textureStore(rt_out_a, hp, r);
    if (last) {
        textureStore(rt_out_c, hp, vec4<f32>(g0.w));
    }
}
