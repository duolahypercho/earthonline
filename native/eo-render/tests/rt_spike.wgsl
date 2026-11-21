// Stage R0: inline ray queries against a city-block acceleration structure
// (tests/rt_spike.rs). The same `trace` runs from compute (cs_*) and from a
// fragment shader (fs_*), so the two stages can be timed side by side.
//
// Syntax per naga 30 (front/wgsl/lower/mod.rs `rayQueryInitialize` ..
// `rayQueryGetCommittedIntersection`; front/type_gen.rs `RayDesc`,
// `RayIntersection`): the `wgpu_ray_query` enable extension is required.
enable wgpu_ray_query;

struct Cam {
    origin: vec3<f32>,
    tan_half: f32,
    right: vec3<f32>,
    aspect: f32,
    up: vec3<f32>,
    frame: u32,
    fwd: vec3<f32>,
    // hemisphere rays per pixel in the bounce passes
    rays: u32,
    size: vec2<u32>,
    tmax_bounce: f32,
    // the ray flags of the bounce rays (RAY_FLAG_* bits)
    bounce_flags: u32,
};

// one entry per BLAS: where its vertices (20-byte kit vertices, 5 u32) and
// its indices (local to its first vertex) start in the shared buffers.
struct Geo {
    vbase: u32,
    ibase: u32,
    pad0: u32,
    pad1: u32,
};

@group(0) @binding(0) var acc: acceleration_structure;
@group(0) @binding(1) var<uniform> cam: Cam;
@group(0) @binding(2) var<storage, read> verts: array<u32>;
@group(0) @binding(3) var<storage, read> idx: array<u32>;
@group(0) @binding(4) var<storage, read> geos: array<Geo>;

// cs_primary writes (t, normal); the bounce passes read it.
@group(1) @binding(0) var out_primary: texture_storage_2d<rgba32float, write>;
@group(1) @binding(1) var in_primary: texture_2d<f32>;
@group(1) @binding(2) var out_bounce: texture_storage_2d<rgba32float, write>;

struct Hit {
    t: f32,
    n: vec3<f32>,
};

fn vpos(i: u32) -> vec3<f32> {
    let b = i * 5u;
    return vec3<f32>(bitcast<f32>(verts[b]), bitcast<f32>(verts[b + 1u]), bitcast<f32>(verts[b + 2u]));
}

// Closest hit (or any hit with RAY_FLAG_TERMINATE_ON_FIRST_HIT); t < 0 on a
// miss. The geometric normal comes from the hit triangle's three vertices,
// fetched through the instance's custom data (no vertex-return feature on
// Metal), turned to world space and faced toward the ray.
fn trace(o: vec3<f32>, d: vec3<f32>, tmax: f32, flags: u32) -> Hit {
    var rq: ray_query;
    rayQueryInitialize(&rq, acc, RayDesc(flags, 0xFFu, 0.0, tmax, o, d));
    while (rayQueryProceed(&rq)) {}
    let h = rayQueryGetCommittedIntersection(&rq);
    var r: Hit;
    r.t = -1.0;
    r.n = vec3<f32>(0.0);
    if (h.kind == RAY_QUERY_INTERSECTION_TRIANGLE) {
        r.t = h.t;
        let g = geos[h.instance_custom_data];
        let at = g.ibase + 3u * h.primitive_index;
        let p0 = vpos(g.vbase + idx[at]);
        let p1 = vpos(g.vbase + idx[at + 1u]);
        let p2 = vpos(g.vbase + idx[at + 2u]);
        var n = cross(p1 - p0, p2 - p0);
        n = (h.object_to_world * vec4<f32>(n, 0.0)).xyz;
        n = n * inverseSqrt(max(dot(n, n), 1e-30));
        if (dot(n, d) > 0.0) {
            n = -n;
        }
        r.n = n;
    }
    return r;
}

fn primary_dir(px: vec2<f32>) -> vec3<f32> {
    let uv = (px + vec2<f32>(0.5)) / vec2<f32>(cam.size);
    let sx = (2.0 * uv.x - 1.0) * cam.tan_half * cam.aspect;
    let sy = (1.0 - 2.0 * uv.y) * cam.tan_half;
    return normalize(cam.fwd + sx * cam.right + sy * cam.up);
}

fn pcg(v: u32) -> u32 {
    let s = v * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}

fn rnd(s: ptr<function, u32>) -> f32 {
    *s = pcg(*s);
    return f32(*s) * (1.0 / 4294967296.0);
}

// cosine-weighted direction around n.
fn hemi(n: vec3<f32>, s: ptr<function, u32>) -> vec3<f32> {
    let u1 = rnd(s);
    let u2 = rnd(s);
    let r = sqrt(u1);
    let phi = 6.2831853 * u2;
    let a = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(n.x) > 0.9);
    let t = normalize(cross(a, n));
    let b = cross(n, t);
    return normalize(t * (r * cos(phi)) + b * (r * sin(phi)) + n * sqrt(max(0.0, 1.0 - u1)));
}

// For the bounce passes: the fraction of hemisphere rays that hit within
// tmax (an ambient-occlusion-like value), the mean hit distance and the mean
// up component of the hit normals (so the normal fetch a lighting pass needs
// is part of the measured cost).
fn bounce(p: vec2<u32>, prim: vec4<f32>) -> vec4<f32> {
    if (prim.x < 0.0) {
        return vec4<f32>(-1.0, 0.0, 0.0, 0.0);
    }
    let d = primary_dir(vec2<f32>(p));
    let n = prim.yzw;
    let o = cam.origin + d * prim.x + n * max(0.01, prim.x * 2e-4);
    var seed = pcg(p.x + p.y * 7919u + cam.frame * 104729u);
    var hits = 0.0;
    var dist = 0.0;
    var up = 0.0;
    for (var k = 0u; k < cam.rays; k++) {
        let h = trace(o, hemi(n, &seed), cam.tmax_bounce, cam.bounce_flags);
        if (h.t >= 0.0) {
            hits += 1.0;
            dist += h.t;
            up += h.n.y;
        }
    }
    return vec4<f32>(hits / f32(cam.rays), dist / max(hits, 1.0), up / max(hits, 1.0), 1.0);
}

@compute @workgroup_size(8, 8, 1)
fn cs_primary(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= cam.size.x || id.y >= cam.size.y) {
        return;
    }
    let h = trace(cam.origin, primary_dir(vec2<f32>(id.xy)), 1e5, RAY_FLAG_NONE);
    textureStore(out_primary, id.xy, vec4<f32>(h.t, h.n));
}

@compute @workgroup_size(8, 8, 1)
fn cs_bounce(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= cam.size.x || id.y >= cam.size.y) {
        return;
    }
    textureStore(out_bounce, id.xy, bounce(id.xy, textureLoad(in_primary, id.xy, 0)));
}

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

@fragment
fn fs_primary(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<u32>(pos.xy);
    let h = trace(cam.origin, primary_dir(vec2<f32>(p)), 1e5, RAY_FLAG_NONE);
    return vec4<f32>(h.t, h.n);
}

@fragment
fn fs_bounce(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<u32>(pos.xy);
    return bounce(p, textureLoad(in_primary, p, 0));
}
