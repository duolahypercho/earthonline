// (`--car-contact`; appended to the agents' draw module only with the switch
// on, its pass made and encoded only then: agents.rs `Contact`) The cars'
// occlusion of the street round them, after the city pass, on its depth.
//
// Each car near the eye draws the back faces of a box round its lower body,
// grown by the reach; every pixel of the street (or a kerb's top, a bonnet)
// under it is taken back to the car's frame from the depth and gets the share
// of its cosine-weighted sky the car's body covers there, exactly, as the
// form factor of the boxes the body is made of (the lower body from its
// underside to the beltline, the upper body, the four tyres; eo-agents
// `vehicles::contact_shape`): Lambert's polygon formula over the faces the
// point sees (Hill and Heitz's fitted edge integral), the bodies combined as
// independent occluders. Under the chassis that is nearly the whole sky; at
// a tyre's foot most of it; a metre beside the sill a fifth. Only the light
// the car takes away is taken: the sky, the ambient light, the bounce and the
// rig's rim (what the frame's screen-space occlusion did not already take:
// the lesser visibility of the two), never the key light where its shadow map
// lets it through, nor the street's lamps beside the car (under the body the
// car blocks those too). The pixel's colour is multiplied by what is left.

@group(3) @binding(0) var ag_ct_depth: texture_depth_2d;

struct AgCtOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(flat) agent: u32,
};

// How far past the body the occlusion is reckoned (m; the lower body's form
// factor is ~5 % there), and how far from the eye a car draws it (past it the
// contact is a pixel or two).
const AG_CT_REACH: f32 = 1.6;
const AG_CT_MAX_DIST: f32 = 120.0;
// A tyre's box along the road: the share of its radius a block of the tyre's
// cross-section (its contact patch, then its sides rising away) is reckoned as.
const AG_CT_TYRE_LEN: f32 = 0.65;

struct AgCtShape {
    // half width, underside height, tail end, nose end
    lower: vec4<f32>,
    // beltline, half width, tail end, nose end
    upper: vec4<f32>,
    // top, the tyres' half width
    top: f32,
    tyre: f32,
};

// The model's occluder boxes (eo-agents `vehicles::contact_words`), or a
// stand-in from its wheels when the library carries none.
fn ag_ct_shape(m: AgModel) -> AgCtShape {
    var s: AgCtShape;
    if (any(m.pad != vec4<u32>(0u))) {
        let a = unpack2x16snorm(m.pad.x) * 32.767;
        let b = unpack2x16snorm(m.pad.y) * 32.767;
        let c = unpack2x16snorm(m.pad.z) * 32.767;
        let d = unpack2x16snorm(m.pad.w) * 32.767;
        s.lower = vec4<f32>(a.x, a.y, b.x, b.y);
        s.upper = vec4<f32>(c.x, c.y, d.x, d.y);
        s.top = m.wheel[0].w;
        s.tyre = m.wheel[1].w;
        return s;
    }
    let r = max(m.wheel_radius, 0.2);
    let hw = abs(m.wheel[0].x) + 0.15;
    let z0 = min(m.wheel[2].z, m.wheel[0].z) - 2.6 * r;
    let z1 = max(m.wheel[2].z, m.wheel[0].z) + 2.6 * r;
    let top = 2.0 * m.center.y;
    s.lower = vec4<f32>(hw, 0.55 * r, z0, z1);
    s.upper = vec4<f32>(0.6 * top, hw * 0.88, mix(z0, z1, 0.25), mix(z0, z1, 0.7));
    s.top = top;
    s.tyre = 0.11;
    return s;
}

@vertex
fn ag_contact_vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) inst: u32) -> AgCtOut {
    var out: AgCtOut;
    out.agent = inst;
    out.clip = vec4<f32>(0.0, 0.0, 0.0, 1.0); // every corner the same: nothing rasterised
    if (inst >= ag.counts.x) {
        return out;
    }
    let a = ag_agents[inst];
    if (a.model >= ag.counts.w) {
        return out;
    }
    let m = ag_models[a.model];
    if (m.model_class == AG_PERSON || m.wheel_radius <= 0.05 || distance(a.pos, frame.cam_pos.xyz) > AG_CT_MAX_DIST) {
        return out;
    }
    let s = ag_ct_shape(m);
    // 36 corners: the 12 triangles of the box, outward winding (the pass draws the back faces).
    let face = vi / 6u;
    var tri = array<u32, 6>(0u, 1u, 2u, 0u, 2u, 3u);
    let k = tri[vi % 6u];
    var quads = array<vec4<u32>, 6>(
        vec4<u32>(0u, 4u, 6u, 2u), // -x
        vec4<u32>(1u, 3u, 7u, 5u), // +x
        vec4<u32>(0u, 1u, 5u, 4u), // -y
        vec4<u32>(2u, 6u, 7u, 3u), // +y
        vec4<u32>(0u, 2u, 3u, 1u), // -z
        vec4<u32>(4u, 5u, 7u, 6u), // +z
    );
    let c = quads[face][k];
    let lo = vec3<f32>(-s.lower.x - AG_CT_REACH, -0.4, s.lower.z - AG_CT_REACH);
    let hi = vec3<f32>(s.lower.x + AG_CT_REACH, s.upper.x, s.lower.w + AG_CT_REACH);
    let p = select(lo, hi, vec3<bool>((c & 1u) != 0u, (c & 2u) != 0u, (c & 4u) != 0u));
    out.clip = frame.view_proj * vec4<f32>(a.pos + ag_rotate(a.rot, p * a.scale), 1.0);
    return out;
}

// Hill and Heitz's fitted edge integral (the vector form factor of one edge:
// the angle it subtends along the normal of the plane through the two directions).
fn ag_ct_edge(a: vec3<f32>, b: vec3<f32>) -> vec3<f32> {
    let x = dot(a, b);
    let y = abs(x);
    let u = 0.8543985 + (0.4965155 + 0.0145206 * y) * y;
    let w = 3.4175940 + (4.1616724 + y) * y;
    let v = u / w;
    let t = select(0.5 * inverseSqrt(max(1.0 - x * x, 1e-7)) - v, v, x > 0.0);
    return cross(a, b) * t;
}

fn ag_ct_quad(c: ptr<function, array<vec3<f32>, 8>>, q: vec4<u32>) -> vec3<f32> {
    return ag_ct_edge((*c)[q.x], (*c)[q.y]) + ag_ct_edge((*c)[q.y], (*c)[q.z]) + ag_ct_edge((*c)[q.z], (*c)[q.w]) + ag_ct_edge((*c)[q.w], (*c)[q.x]);
}

// The share of the cosine-weighted sky over p (facing +y) a box covers: the
// form factor of the faces p sees (a convex box's silhouette), its part
// below p's horizon left out (the box cut at p's height).
fn ag_ct_box(p: vec3<f32>, lo_in: vec3<f32>, hi: vec3<f32>) -> f32 {
    let lo = vec3<f32>(lo_in.x, max(lo_in.y, p.y + 0.005), lo_in.z);
    if (hi.y <= lo.y) {
        return 0.0;
    }
    var c: array<vec3<f32>, 8>;
    for (var i = 0u; i < 8u; i = i + 1u) {
        c[i] = normalize(select(lo, hi, vec3<bool>((i & 1u) != 0u, (i & 2u) != 0u, (i & 4u) != 0u)) - p);
    }
    var s = vec3<f32>(0.0);
    if (p.y < lo.y) {
        s = s + ag_ct_quad(&c, vec4<u32>(0u, 1u, 5u, 4u));
    }
    if (p.x < lo.x) {
        s = s + ag_ct_quad(&c, vec4<u32>(0u, 4u, 6u, 2u));
    } else if (p.x > hi.x) {
        s = s + ag_ct_quad(&c, vec4<u32>(1u, 3u, 7u, 5u));
    }
    if (p.z < lo.z) {
        s = s + ag_ct_quad(&c, vec4<u32>(0u, 2u, 3u, 1u));
    } else if (p.z > hi.z) {
        s = s + ag_ct_quad(&c, vec4<u32>(4u, 5u, 7u, 6u));
    }
    return clamp(abs(s.y) / (2.0 * LK_PI), 0.0, 1.0);
}

fn ag_ct_inside(p: vec3<f32>, lo: vec3<f32>, hi: vec3<f32>) -> bool {
    return all(p > lo) && all(p < hi);
}

// The pixel's position (camera-relative world, as the occlusion pass's `ao_pos`).
fn ag_ct_pos(px: vec2<i32>, dims: vec2<i32>) -> vec3<f32> {
    let p = clamp(px, vec2<i32>(0), dims - 1);
    let d = textureLoad(ag_ct_depth, p, 0);
    let uv = (vec2<f32>(p) + 0.5) / vec2<f32>(dims);
    let w = look.inv_view_proj * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    return w.xyz / w.w;
}

@fragment
fn ag_contact_fs(in: AgCtOut) -> @location(0) vec4<f32> {
    let none = vec4<f32>(1.0);
    let dims = vec2<i32>(textureDimensions(ag_ct_depth));
    let px = vec2<i32>(in.clip.xy);
    if (textureLoad(ag_ct_depth, clamp(px, vec2<i32>(0), dims - 1), 0) <= 0.0) {
        return none; // the sky
    }
    let P = ag_ct_pos(px, dims);
    // the normal from the depth's neighbours, the smaller step on each axis.
    let pr = ag_ct_pos(px + vec2<i32>(1, 0), dims) - P;
    let pl = P - ag_ct_pos(px - vec2<i32>(1, 0), dims);
    let pd = ag_ct_pos(px + vec2<i32>(0, 1), dims) - P;
    let pu = P - ag_ct_pos(px - vec2<i32>(0, 1), dims);
    var n = normalize(cross(select(pu, pd, dot(pd, pd) < dot(pu, pu)), select(pl, pr, dot(pr, pr) < dot(pl, pl))));
    if (dot(n, P) > 0.0) {
        n = -n;
    }
    let world = P + look.cam_pos.xyz;
    let a = ag_agents[in.agent];
    let m = ag_models[a.model];
    let s = ag_ct_shape(m);
    let qi = vec4<f32>(-a.rot.xyz, a.rot.w);
    let p = ag_rotate(qi, world - a.pos) / max(a.scale, 1e-3);
    // a surface facing up (the street, a kerb's top, a bonnet) under the car's sky.
    let up = smoothstep(0.55, 0.85, ag_rotate(qi, n).y);
    if (up <= 0.0) {
        return none;
    }
    let lower_lo = vec3<f32>(-s.lower.x, s.lower.y, s.lower.z);
    let lower_hi = vec3<f32>(s.lower.x, s.upper.x, s.lower.w);
    let upper_lo = vec3<f32>(-s.upper.y, s.upper.x, s.upper.z);
    let upper_hi = vec3<f32>(s.upper.y, s.top, s.upper.w);
    // the car's own surfaces are not under it.
    let grow = vec3<f32>(0.02);
    if (ag_ct_inside(p, lower_lo - grow, lower_hi + grow) || ag_ct_inside(p, upper_lo - grow, upper_hi + grow)) {
        return none;
    }
    // how far outside the body's footprint (0 under it).
    let out_x = max(abs(p.x) - s.lower.x, 0.0);
    let out_z = max(max(s.lower.z - p.z, p.z - s.lower.w), 0.0);
    let away = length(vec2<f32>(out_x, out_z));
    let fade = 1.0 - smoothstep(AG_CT_REACH * 0.6, AG_CT_REACH, away);
    if (fade <= 0.0) {
        return none;
    }
    let body = 1.0 - (1.0 - ag_ct_box(p, lower_lo, lower_hi)) * (1.0 - ag_ct_box(p, upper_lo, upper_hi));
    var tyres = 0.0;
    let wr = max(m.wheel_radius, 0.2);
    for (var i = 0u; i < 4u; i = i + 1u) {
        let w = m.wheel[i].xyz;
        if (abs(p.z - w.z) > 1.5 || abs(p.x - w.x) > 1.5) {
            continue;
        }
        let tlo = vec3<f32>(w.x - s.tyre, 0.0, w.z - AG_CT_TYRE_LEN * wr);
        let thi = vec3<f32>(w.x + s.tyre, w.y + wr, w.z + AG_CT_TYRE_LEN * wr);
        if (ag_ct_inside(p, tlo - grow, thi + grow)) {
            return none; // the tyre itself
        }
        tyres = tyres + ag_ct_box(p, tlo, thi);
    }
    let occ = (1.0 - (1.0 - body) * (1.0 - min(tyres, 1.0))) * up * fade;
    if (occ < 0.004) {
        return none;
    }
    let vis = 1.0 - occ;
    // What lit the point (white, as look_light lights it): the key through its
    // shadow and the street's lamps stand (under the body the lamps are the
    // car's to block); the sky, the ambient, the rim and the bounce the car takes.
    let v = normalize(-P);
    let under = 1.0 - smoothstep(0.0, 0.3, away);
    let key = look.key.xyz * max(dot(n, look.key_dir.xyz), 0.0) * lk_shadow(world, n) * sky_cloud_transmittance(world);
    let lamps = sky_local_lights_ao(world, n, v, vec3<f32>(LK_PI), vec3<f32>(0.0), 1.0, 1.0);
    let env_i = look.ambient.w * look.env_class.z;
    let hemi = mix(look.hemi_ground.xyz, look.hemi_sky.xyz, n.y * 0.5 + 0.5);
    var irr = hemi + look.ambient.xyz + lk_sh(n) * env_i;
    if (amb_on()) {
        irr = amb_irradiance(amb_sample(world, n), n, lk_sh(n) * env_i);
    }
    let ao4 = ao_fetch(world);
    let ao = ao_multi_bounce(ao4.a);
    let rim = look.rim.xyz * max(dot(n, look.rim_dir.xyz), 0.0);
    let bounce = ao4.rgb * LK_PI;
    let stand = key + lamps * (1.0 - under);
    let taken = irr * ao + rim + bounce + lamps * under;
    let left = irr * min(ao, vis) + (rim + bounce + lamps * under) * vis;
    let k = (stand + left) / max(stand + taken, vec3<f32>(1e-6));
    return vec4<f32>(clamp(k, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
