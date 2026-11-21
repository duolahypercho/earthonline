// The traced scene (rt_scene.rs): group 0 of every tracing pipeline and the
// shading of a ray's hit, shared by the diffuse light (rt_gi.wgsl) and the
// reflections. rt_scene.rs `module_source` prepends `enable wgpu_ray_query;`,
// look_common_source() (the `Look` struct and its helpers) and the street
// constants (ST_*); a pass's own WGSL follows (its bindings from group 1 on).
//
// A hit is static city geometry only (kit buildings, hand-built buildings,
// terrain, the street's flat strips, structures and landmarks; no agents,
// props, trees or water). Its light is Lambert: its albedo (the material
// table, the kit palette and paint, the kit layer's mean, the street's class)
// x (the key through a shadow ray toward it + the rim + the rig's open-sky
// irradiance x a share the pass chooses), the hit's own sky visibility not
// traced (one bounce).

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
@group(0) @binding(1) var rt_acc: acceleration_structure;
// the shared arena: 20-byte vertices (position, packed normal, colour), 32-bit indices.
@group(0) @binding(2) var<storage, read> rt_verts: array<u32>;
@group(0) @binding(3) var<storage, read> rt_idx: array<u32>;
// one entry per acceleration-structure geometry: instance custom data + geometry index.
@group(0) @binding(4) var<storage, read> rt_geos: array<RtGeo>;
@group(0) @binding(5) var<storage, read> rt_mats: array<RtMat>;
// each kit map layer's mean albedo (linear rgb), 0.5 grey until the bake lands.
@group(0) @binding(6) var<storage, read> rt_layer_mean: array<vec4<f32>>;

const RT_F_GLASS: u32 = 8u;
const RT_F_VERTEX_COLOUR: u32 = 1u;
// glazing reflects, it scatters little (a clean pane's diffuse share).
const RT_GLASS_ALBEDO: f32 = 0.04;
// the street classes (draw.wgsl SC_*): concrete from the kerb face on; the gutter pan's silt.
const RT_SC_GUTTER: u32 = 1u;
const RT_SC_KERB_FACE: u32 = 3u;

// ---------------------------------------------------------------------------
// Sampling.
// ---------------------------------------------------------------------------

fn rt_pcg(v: u32) -> u32 {
    let s = v * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}

// Two numbers in [0, 1): a per-texel offset (salted per pass) stepped each
// frame along the R2 sequence, so a texel's samples spread over its domain
// in few frames.
fn rt_rand2(p: vec2<u32>, frame: u32, salt: u32) -> vec2<f32> {
    let h = rt_pcg(p.x + rt_pcg(p.y + 0x9e3779b9u + salt));
    let o = vec2<f32>(f32(h & 0xffffu), f32(h >> 16u)) * (1.0 / 65536.0);
    let f = f32(frame % 4096u);
    return fract(o + f * vec2<f32>(0.7548776662, 0.5698402910));
}

// An orthonormal frame round n (t, b).
fn rt_basis(n: vec3<f32>) -> mat2x3<f32> {
    let a = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(n.x) > 0.9);
    let t = normalize(cross(a, n));
    return mat2x3<f32>(t, cross(n, t));
}

fn rt_cosine(n: vec3<f32>, u: vec2<f32>) -> vec3<f32> {
    let r = sqrt(u.x);
    let phi = 6.2831853 * u.y;
    let tb = rt_basis(n);
    return normalize(tb[0] * (r * cos(phi)) + tb[1] * (r * sin(phi)) + n * sqrt(max(0.0, 1.0 - u.x)));
}

// ---------------------------------------------------------------------------
// The rig's terms (look_light.wgsl), for a hit.
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

// The hemisphere light's radiance along d (sky above the horizon, the ground lobe below).
fn rt_hemi_radiance(d: vec3<f32>) -> vec3<f32> {
    return select(look.hemi_ground.xyz, look.hemi_sky.xyz, d.y >= 0.0) / LK_PI;
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

// The key's visibility from a point: any hit toward the sun within `reach`.
fn rt_sun_visible(o: vec3<f32>, l: vec3<f32>, reach: f32) -> f32 {
    var rq: ray_query;
    rayQueryInitialize(&rq, rt_acc, RayDesc(RAY_FLAG_TERMINATE_ON_FIRST_HIT, 0xFFu, 0.02, reach, o, l));
    while (rayQueryProceed(&rq)) {}
    let h = rayQueryGetCommittedIntersection(&rq);
    return select(1.0, 0.0, h.kind == RAY_QUERY_INTERSECTION_TRIANGLE);
}

struct RtHit {
    // false: the ray left the scene within its reach
    hit: bool,
    t: f32,
    // the hit triangle's normal, facing the ray's origin
    n: vec3<f32>,
    albedo: vec3<f32>,
    // the hit's material flags (RT_F_GLASS: a pane)
    flags: u32,
};

// The nearest hit along d from o within [tmin, tmax].
fn rt_hit(o: vec3<f32>, d: vec3<f32>, tmin: f32, tmax: f32) -> RtHit {
    var rq: ray_query;
    rayQueryInitialize(&rq, rt_acc, RayDesc(RAY_FLAG_NONE, 0xFFu, tmin, tmax, o, d));
    while (rayQueryProceed(&rq)) {}
    let h = rayQueryGetCommittedIntersection(&rq);
    var r = RtHit(false, tmax, vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(0.0), 0u);
    if (h.kind != RAY_QUERY_INTERSECTION_TRIANGLE) {
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
    let colour = rt_verts[i0 * 5u + 4u];
    r.hit = true;
    r.t = h.t;
    r.n = n;
    r.albedo = rt_albedo(g, colour);
    var mi = g.material;
    if ((g.kind & 255u) == 5u) {
        mi = mi + u32(round(unpack4x8unorm(colour).a * 255.0));
    }
    r.flags = rt_mats[mi].flags;
    return r;
}

// The Lambert radiance a hit sends back along the ray: albedo / pi x (the key
// through a shadow ray within `reach`, the rim, `sky` x the rig's open-sky
// irradiance at its normal).
fn rt_hit_light(h: RtHit, o: vec3<f32>, d: vec3<f32>, sky: f32, reach: f32) -> vec3<f32> {
    let p = o + d * h.t + h.n * 0.03;
    var e = rt_rig_irradiance(h.n) * sky;
    let l = look.key_dir.xyz;
    let nl = dot(h.n, l);
    if (nl > 0.0 && look.key_dir.w > 0.0) {
        e = e + look.key.xyz * (nl * look.key_dir.w * rt_sun_visible(p, l, reach));
    }
    e = e + look.rim.xyz * max(dot(h.n, look.rim_dir.xyz), 0.0);
    return h.albedo / LK_PI * e;
}
