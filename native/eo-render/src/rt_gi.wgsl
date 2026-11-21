// Ray-traced one-bounce diffuse light (`--rt-gi`, rt_gi.rs): three compute
// stages at half the output's resolution, after the city pass, on its depth.
// rt_gi.rs prepends `enable wgpu_ray_query;`, look_common_source() (the
// `Look` struct and its helpers) and the street constants (ST_*).
//
//  * `rt_trace`: a surface point and normal from the depth (as surfaces_ao.wgsl
//    finds them), one cosine-distributed ray into the acceleration structure
//    (static city geometry only: kit buildings, hand-built buildings, terrain,
//    the street's flat strips, structures and landmarks; no agents, props or
//    water). A ray that leaves the scene takes the rig's sky in its direction
//    (the hemisphere light's radiance, sky above the horizon and ground lobe
//    below); a ray that hits takes the hit's Lambert radiance: its albedo
//    (the material table, the kit palette and paint, the kit layer's mean, the
//    street's class) x (the key, through a shadow ray toward it, + the rim +
//    the rig's sky at the hit's normal x RT_HIT_SKY). Writes the irradiance
//    without the rig's ambient and environment terms (rgb) and whether the ray
//    left the scene (a: the environment's visible share, look_light scales its
//    environment term by it), and the point's normal and view depth.
//  * `rt_temporal`: the previous frame's history at the point, reprojected
//    with the previous camera and refused where its depth or normal is another
//    surface's, blended with this frame's sample over up to RT_MAX_FRAMES
//    frames (a running mean).
//    It also keeps the running moments of a guide value (`rt_guide`: the
//    irradiance the sample stands for, the environment's share at the
//    painted-facade class) for the filter's luminance stop.
//  * `rt_atrous`: an edge-aware a-trous filter (5 x 5 B3 spline at a step of
//    1, 2, 4 texels), its weights stopped by the point's plane and normal and
//    by the guide value against the history's standard deviation (a
//    variance-guided stop: coplanar ground in and out of a deck's shade stays
//    apart); the last pass writes the result and its view depth for the next
//    frame's city pass (rt_gi_use.wgsl reads them, reprojected as `ao_fetch`
//    reads the AO).

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

struct RtGeo {
    base_vertex: u32,
    first_index: u32,
    material: u32,
    kind: u32,
};

struct RtMat {
    base: vec4<f32>,
    emissive: vec4<f32>,
    params: vec4<f32>,
    layer: i32,
    flags: u32,
    rough_flatten: f32,
    thin_absorption: f32,
};

@group(0) @binding(0) var<uniform> look: Look;
@group(0) @binding(1) var<uniform> rt: RtTrace;
@group(0) @binding(2) var rt_acc: acceleration_structure;
// the shared arena: 20-byte vertices (position, packed normal, colour), 32-bit indices.
@group(0) @binding(3) var<storage, read> rt_verts: array<u32>;
@group(0) @binding(4) var<storage, read> rt_idx: array<u32>;
// one entry per acceleration-structure geometry: instance custom data + geometry index.
@group(0) @binding(5) var<storage, read> rt_geos: array<RtGeo>;
@group(0) @binding(6) var<storage, read> rt_mats: array<RtMat>;
// each kit map layer's mean albedo (linear rgb), 0.5 grey until the bake lands.
@group(0) @binding(7) var<storage, read> rt_layer_mean: array<vec4<f32>>;
@group(0) @binding(8) var rt_depth: texture_depth_2d;

// The stage's own targets (group 1; rt_gi.rs makes one layout a stage with
// only the bindings its entry point reads): trace 4 5; temporal 0 1 2 3 8, 4 5;
// a-trous 0 1 2, 4 6 7.
@group(1) @binding(0) var rt_in_a: texture_2d<f32>;
@group(1) @binding(1) var rt_in_b: texture_2d<f32>;
@group(1) @binding(2) var rt_in_c: texture_2d<f32>;
@group(1) @binding(3) var rt_in_d: texture_2d<f32>;
@group(1) @binding(4) var rt_out_a: texture_storage_2d<rgba16float, write>;
@group(1) @binding(5) var rt_out_b: texture_storage_2d<rgba32float, write>;
@group(1) @binding(6) var rt_out_c: texture_storage_2d<r32float, write>;
@group(1) @binding(7) var<uniform> rt_step: vec4<u32>;
@group(1) @binding(8) var rt_in_e: texture_2d<f32>;

const RT_F_GLASS: u32 = 8u;
const RT_F_VERTEX_COLOUR: u32 = 1u;
// glazing reflects, it scatters little (a clean pane's diffuse share).
const RT_GLASS_ALBEDO: f32 = 0.04;
// the street classes (draw.wgsl SC_*): concrete from the kerb face on; the gutter pan's silt.
const RT_SC_GUTTER: u32 = 1u;
const RT_SC_KERB_FACE: u32 = 3u;
// a ray in a texel of sky or past the fade: none.
const RT_NONE: f32 = -1.0;
// the filter's guide stop: standard deviations of the history, and a floor (a
// share of the centre's value) so a converged texel still averages its peers.
const RT_LUMA_SIGMA: f32 = 4.0;
const RT_LUMA_FLOOR: f32 = 0.1;

// ---------------------------------------------------------------------------
// The surface point from the depth (surfaces_ao.wgsl `ao_pos`), camera-relative.
// ---------------------------------------------------------------------------

fn rt_pos(px: vec2<i32>) -> vec4<f32> {
    let dims = vec2<i32>(rt.size.zw);
    let p = clamp(px, vec2<i32>(0), dims - 1);
    let d = textureLoad(rt_depth, p, 0);
    let uv = (vec2<f32>(p) + 0.5) / vec2<f32>(dims);
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    let w = look.inv_view_proj * ndc;
    return vec4<f32>(w.xyz / w.w, d);
}

// The geometric normal from the depth neighbours, the smaller step on each
// axis, facing the eye (`ao_main`).
fn rt_normal(px: vec2<i32>, c: vec3<f32>) -> vec3<f32> {
    let r = rt_pos(px + vec2<i32>(1, 0)).xyz - c;
    let l = c - rt_pos(px - vec2<i32>(1, 0)).xyz;
    let d = rt_pos(px + vec2<i32>(0, 1)).xyz - c;
    let u = c - rt_pos(px - vec2<i32>(0, 1)).xyz;
    // the shorter non-zero step on each axis (at the frame's edge the clamped
    // neighbour is the texel itself: a zero step, which would make the normal NaN).
    let dx = select(l, r, (dot(r, r) < dot(l, l) && dot(r, r) > 0.0) || dot(l, l) == 0.0);
    let dy = select(u, d, (dot(d, d) < dot(u, u) && dot(d, d) > 0.0) || dot(u, u) == 0.0);
    let x = cross(dy, dx);
    let len2 = dot(x, x);
    if (len2 < 1e-24) {
        return -c * inverseSqrt(max(dot(c, c), 1e-12)); // no tangent: face the eye
    }
    var n = x * inverseSqrt(len2);
    if (dot(n, -c) < 0.0) {
        n = -n;
    }
    return n;
}

// ---------------------------------------------------------------------------
// Sampling.
// ---------------------------------------------------------------------------

fn rt_pcg(v: u32) -> u32 {
    let s = v * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}

// Two numbers in [0, 1): a per-texel offset stepped each frame along the R2
// sequence, so a texel's samples spread over its hemisphere in few frames.
fn rt_rand2(p: vec2<u32>, frame: u32) -> vec2<f32> {
    let h = rt_pcg(p.x + rt_pcg(p.y + 0x9e3779b9u));
    let o = vec2<f32>(f32(h & 0xffffu), f32(h >> 16u)) * (1.0 / 65536.0);
    let f = f32(frame % 4096u);
    return fract(o + f * vec2<f32>(0.7548776662, 0.5698402910));
}

fn rt_cosine(n: vec3<f32>, u: vec2<f32>) -> vec3<f32> {
    let r = sqrt(u.x);
    let phi = 6.2831853 * u.y;
    let a = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(n.x) > 0.9);
    let t = normalize(cross(a, n));
    let b = cross(n, t);
    return normalize(t * (r * cos(phi)) + b * (r * sin(phi)) + n * sqrt(max(0.0, 1.0 - u.x)));
}

// ---------------------------------------------------------------------------
// The rig's indirect terms (look_light.wgsl), for a hit.
// ---------------------------------------------------------------------------

fn rt_sh(n: vec3<f32>) -> vec3<f32> {
    let x = n.x;
    let y = n.y;
    let z = n.z;
    var e = look.sh[0].xyz * 0.282095;
    e = e + look.sh[1].xyz * (0.488603 * y) + look.sh[2].xyz * (0.488603 * z) + look.sh[3].xyz * (0.488603 * x);
    e = e + look.sh[4].xyz * (1.092548 * x * y) + look.sh[5].xyz * (1.092548 * y * z) + look.sh[6].xyz * (0.315392 * (3.0 * z * z - 1.0));
    e = e + look.sh[7].xyz * (1.092548 * x * z) + look.sh[8].xyz * (0.546274 * (x * x - y * y));
    return max(e, vec3<f32>(0.0));
}

// The rig's open-sky irradiance at a normal (hemisphere light, ambient,
// environment at the painted-facade class), as `look_light` builds `irr`.
fn rt_rig_irradiance(n: vec3<f32>) -> vec3<f32> {
    let hemi = mix(look.hemi_ground.xyz, look.hemi_sky.xyz, n.y * 0.5 + 0.5);
    return hemi + look.ambient.xyz + rt_sh(n) * (look.ambient.w * look.env_class.x);
}

// The filter's guide: the irradiance a texel's (irradiance, open share) stands
// for at normal n, the environment's share at the painted-facade class.
fn rt_guide(v: vec4<f32>, n: vec3<f32>) -> f32 {
    return lk_luma(v.rgb) + v.a * lk_luma(rt_sh(n) * (look.ambient.w * look.env_class.x));
}

fn rt_srgb(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

// The Lambert albedo at a hit, from its geometry's material and the hit
// triangle's first vertex colour (draw.wgsl `vs` and `shade`, reduced to
// their means: no scans, no weathering).
fn rt_albedo(g: RtGeo, colour: u32) -> vec3<f32> {
    let kind = g.kind & 255u;
    let c = unpack4x8unorm(colour);
    var mi = g.material;
    if (kind == 5u) {
        mi = mi + u32(round(c.a * 255.0)); // the kit palette index
    }
    let m = rt_mats[mi];
    if ((m.flags & RT_F_GLASS) != 0u) {
        return vec3<f32>(RT_GLASS_ALBEDO);
    }
    var a = m.base.rgb;
    if (kind == 5u) {
        a = a * pow(c.rgb, vec3<f32>(2.2)); // the paint
        if (m.layer >= 0) {
            a = a * rt_layer_mean[m.layer].rgb;
        }
    } else if (kind == 6u) {
        // the street surface: the class in the alpha byte, the tint exact sRGB.
        let cls = u32(round(c.a * 255.0));
        a = rt_srgb(c.rgb);
        if (cls >= RT_SC_KERB_FACE) {
            a = a * ST_CONCRETE_BASE;
        } else if (cls == RT_SC_GUTTER) {
            a = a * (1.0 - ST_GUTTER_POUR_SILT);
        } else {
            a = a * ST_ASPHALT_BASE;
        }
    } else if (kind == 7u || kind == 9u || kind == 10u || kind == 11u) {
        a = rt_srgb(c.rgb); // paint, landmarks, a structure's deck: exact sRGB vertex colour
    } else if (kind == 12u) {
        a = m.base.rgb; // a structure's concrete: its tint (the colour bytes carry the formwork)
    } else if ((m.flags & RT_F_VERTEX_COLOUR) != 0u) {
        a = pow(c.rgb, vec3<f32>(2.2));
    } else if (m.layer >= 0) {
        a = a * rt_layer_mean[m.layer].rgb;
    }
    return clamp(a, vec3<f32>(0.0), vec3<f32>(0.95));
}

fn rt_vpos(i: u32) -> vec3<f32> {
    let b = i * 5u;
    return vec3<f32>(bitcast<f32>(rt_verts[b]), bitcast<f32>(rt_verts[b + 1u]), bitcast<f32>(rt_verts[b + 2u]));
}

// The key's visibility from a point: any hit toward the sun within the reach.
fn rt_sun_visible(o: vec3<f32>, l: vec3<f32>) -> f32 {
    var rq: ray_query;
    rayQueryInitialize(&rq, rt_acc, RayDesc(RAY_FLAG_TERMINATE_ON_FIRST_HIT, 0xFFu, 0.02, 2.0 * rt.fwd.w, o, l));
    while (rayQueryProceed(&rq)) {}
    let h = rayQueryGetCommittedIntersection(&rq);
    return select(1.0, 0.0, h.kind == RAY_QUERY_INTERSECTION_TRIANGLE);
}

struct RtRay {
    // radiance along the ray without the rig's ambient and environment terms
    l: vec3<f32>,
    // 1 when the ray left the scene (its environment counts), else 0
    open: f32,
};

fn rt_cast(o: vec3<f32>, d: vec3<f32>, tmin: f32) -> RtRay {
    var rq: ray_query;
    rayQueryInitialize(&rq, rt_acc, RayDesc(RAY_FLAG_NONE, 0xFFu, tmin, rt.fwd.w, o, d));
    while (rayQueryProceed(&rq)) {}
    let h = rayQueryGetCommittedIntersection(&rq);
    var r: RtRay;
    if (h.kind != RAY_QUERY_INTERSECTION_TRIANGLE) {
        // the hemisphere light's radiance in this direction (sky above, ground lobe below).
        r.l = select(look.hemi_ground.xyz, look.hemi_sky.xyz, d.y >= 0.0) / LK_PI;
        r.open = 1.0;
        return r;
    }
    let g = rt_geos[h.instance_custom_data + h.geometry_index];
    let at = g.first_index + 3u * h.primitive_index;
    let i0 = g.base_vertex + rt_idx[at];
    let p0 = rt_vpos(i0);
    let p1 = rt_vpos(g.base_vertex + rt_idx[at + 1u]);
    let p2 = rt_vpos(g.base_vertex + rt_idx[at + 2u]);
    var n = cross(p1 - p0, p2 - p0);
    n = n * inverseSqrt(max(dot(n, n), 1e-30));
    if (dot(n, d) > 0.0) {
        n = -n;
    }
    let albedo = rt_albedo(g, rt_verts[i0 * 5u + 4u]);
    let p = o + d * h.t + n * 0.03;
    var e = rt_rig_irradiance(n) * rt.params.z;
    let l = look.key_dir.xyz;
    let nl = dot(n, l);
    if (nl > 0.0 && look.key_dir.w > 0.0) {
        e = e + look.key.xyz * (nl * look.key_dir.w * rt_sun_visible(p, l));
    }
    e = e + look.rim.xyz * max(dot(n, look.rim_dir.xyz), 0.0);
    r.l = albedo / LK_PI * e;
    r.open = 0.0;
    return r;
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
    let px = hp * 2;
    let c = rt_pos(px);
    if (c.w <= 0.0) {
        textureStore(rt_out_a, hp, vec4<f32>(0.0));
        textureStore(rt_out_b, hp, vec4<f32>(0.0, 1.0, 0.0, RT_NONE));
        return;
    }
    let vd = dot(c.xyz, rt.fwd.xyz);
    if (vd >= rt.fade.y) {
        textureStore(rt_out_a, hp, vec4<f32>(0.0));
        textureStore(rt_out_b, hp, vec4<f32>(0.0, 1.0, 0.0, RT_NONE));
        return;
    }
    let n = rt_normal(px, c.xyz);
    let world = c.xyz + rt.cam.xyz;
    let o = world + n * (rt.params.x + rt.params.y * vd);
    let d = rt_cosine(n, rt_rand2(gid.xy, u32(rt.cam.w)));
    let r = rt_cast(o, d, 0.0);
    // one cosine-distributed sample: the irradiance estimate is pi x its radiance.
    textureStore(rt_out_a, hp, vec4<f32>(r.l * LK_PI, r.open));
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
