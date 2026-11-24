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
// lets it through. The street's lamps (each light of the pixel's cluster, as
// the city's shading gathers them) are each taken where the car's bodies (the
// lower and upper body and the nearest tyre) stand between the point and the
// lamp: the ray to the lamp marched through the bodies' distance field, its
// clearance over the lamp's penumbra there (the lamp a sphere of its
// luminaire's size), so the shadow is hard at the contact and softens away
// from it; where the lamp's own shadow map already darkens the point the
// lesser of the two is kept. A lamp at or in the body (the car's own) is
// never taken. The pixel's colour is multiplied by what is left.

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
    var lo = vec3<f32>(-s.lower.x - AG_CT_REACH, -0.4, s.lower.z - AG_CT_REACH);
    var hi = vec3<f32>(s.lower.x + AG_CT_REACH, s.upper.x, s.lower.w + AG_CT_REACH);
    if (SF_CAR_WET && ag_ct_wet_on()) {
        // (`--car-wet`) on a wet street the box reaches toward the eye: the
        // road between the car and the eye mirrors the lights behind the car
        let e = ag_rotate(vec4<f32>(-a.rot.xyz, a.rot.w), frame.cam_pos.xyz - a.pos) / max(a.scale, 1e-3);
        let de = length(e.xz);
        let reach = clamp(de - AG_CT_WET_NEAR, 0.0, AG_CT_WET_REACH);
        if (reach > 0.0) {
            let q = e.xz / de * reach;
            let rise = reach * AG_CT_WET_GRADE;
            lo = min(lo, vec3<f32>(q.x - AG_CT_REACH, -0.4 - rise, q.y - AG_CT_REACH));
            hi = max(hi, vec3<f32>(q.x + AG_CT_REACH, s.upper.x + rise, q.y + AG_CT_REACH));
        }
    }
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

// A box's signed distance (m) at q.
fn ag_ct_sdbox(q: vec3<f32>, lo: vec3<f32>, hi: vec3<f32>) -> f32 {
    let d = abs(q - 0.5 * (lo + hi)) - 0.5 * (hi - lo);
    return length(max(d, vec3<f32>(0.0))) + min(max(d.x, max(d.y, d.z)), 0.0);
}

// Within this of the body a light is the car's own lamp (m).
const AG_CT_OWN_LAMP_M: f32 = 0.35;
// The ray's samples through the bodies' bounds, and its least penumbra (m).
const AG_CT_RAY_STEPS: u32 = 6u;
const AG_CT_PEN_MIN: f32 = 0.015;

// The share of a lamp at `lp` (the car's frame, `src` m across in radius) the
// car's bodies leave to the point `p`: the ray's least clearance over the
// lamp's penumbra along its stretch through the bodies' bounds.
fn ag_ct_lamp_vis(p: vec3<f32>, lp: vec3<f32>, src: f32, b: array<vec3<f32>, 6>) -> f32 {
    if (min(min(ag_ct_sdbox(lp, b[0], b[1]), ag_ct_sdbox(lp, b[2], b[3])), ag_ct_sdbox(lp, b[4], b[5])) < AG_CT_OWN_LAMP_M) {
        return 1.0;
    }
    let dv = lp - p;
    let dist = length(dv);
    let dir = dv / max(dist, 1e-4);
    let pen = src / max(dist, 1e-3);
    let g = vec3<f32>(0.05 + pen * 6.0);
    let blo = min(min(b[0], b[2]), b[4]) - g;
    let bhi = max(max(b[1], b[3]), b[5]) + g;
    let dd = select(dir, vec3<f32>(1e-6), abs(dir) < vec3<f32>(1e-6));
    let ta = (blo - p) / dd;
    let tb = (bhi - p) / dd;
    let t0 = max(max(max(min(ta.x, tb.x), min(ta.y, tb.y)), min(ta.z, tb.z)), 0.0);
    let t1 = min(min(min(max(ta.x, tb.x), max(ta.y, tb.y)), max(ta.z, tb.z)), dist);
    if (t1 <= t0) {
        return 1.0;
    }
    var m = 1e9;
    for (var i = 0u; i < AG_CT_RAY_STEPS; i = i + 1u) {
        let t = mix(t0, t1, (f32(i) + 0.5) / f32(AG_CT_RAY_STEPS));
        let q = p + dir * t;
        let sd = min(min(ag_ct_sdbox(q, b[0], b[1]), ag_ct_sdbox(q, b[2], b[3])), ag_ct_sdbox(q, b[4], b[5]));
        m = min(m, sd / max(pen * t, AG_CT_PEN_MIN));
    }
    return smoothstep(-1.0, 1.0, m);
}

// A light's emission toward the point over its axis's (as sky_light.wgsl
// `sky_local_lights_ao` shapes it) and its source's radius (m).
fn ag_ct_lamp_shape(li: SkyLight, l: vec3<f32>, dist: f32, cine: bool) -> vec2<f32> {
    let ca = dot(-l, li.dir);
    if (li.kind == 0u) {
        if (cine) {
            return vec2<f32>(sky_cine_cobra(l, li.dir), SKY_CINE_SRC_STREET);
        }
        return vec2<f32>((1.0 + 1.5 * (1.0 - ca * ca)) * smoothstep(li.cos_outer, li.cos_inner, ca), SKY_CINE_SRC_STREET);
    }
    if (li.kind == 1u) {
        let dep = asin(clamp(l.y, -1.0, 1.0)) * 57.29578;
        var vert = 1.0;
        if (dep < 0.6) {
            vert = 0.031;
        } else if (dep < 1.3) {
            vert = mix(0.031, 1.0, (dep - 0.6) / 0.7);
        } else if (dep > 2.5) {
            vert = (2.5 / dep) * (2.5 / dep);
        }
        let hl = normalize(vec2<f32>(-l.x, -l.z) + vec2<f32>(1e-6, 0.0));
        let ha = normalize(vec2<f32>(li.dir.x, li.dir.z) + vec2<f32>(1e-6, 0.0));
        let side = smoothstep(li.cos_outer, li.cos_inner, dot(hl, ha));
        let near = clamp(max(dist, 0.4) / 6.5, 0.0, 1.0);
        if (cine) {
            let hr = vec2<f32>(-ha.y, ha.x);
            vert = sky_cine_low_beam(dep, atan2(dot(hl, hr), dot(hl, ha)) * 57.29578);
        }
        return vec2<f32>(vert * side * near * near, SKY_CINE_SRC_HEADLAMP);
    }
    if (li.kind >= 3u) {
        return vec2<f32>(smoothstep(li.cos_outer, li.cos_inner, ca), select(SKY_CINE_SRC_STREET, 0.5 * sqrt(max(li.size, 0.0)), li.kind >= 4u));
    }
    return vec2<f32>(1.0, SKY_CINE_SRC_STREET);
}

struct AgCtLamps {
    // what the street's lamps gave the point (white, their maps' shadows in), what the car leaves of it
    lit: vec3<f32>,
    left: vec3<f32>,
};

// The street's lamps at a point facing `n` (the pixel's cluster, as the city's
// shading gathers them), and what of them the car leaves (`p` the point in the
// car's frame, `qi` the frame's inverse rotation, `b` its bodies' boxes).
fn ag_ct_lamps(world: vec3<f32>, n: vec3<f32>, p: vec3<f32>, a: AgAgent, b: array<vec3<f32>, 6>) -> AgCtLamps {
    var r = AgCtLamps(vec3<f32>(0.0), vec3<f32>(0.0));
    if (!sky_on(skyu.flags.x, SKY_LIGHTS) || skyu.lights.x == 0u) {
        return r;
    }
    let rel = world - skyu.eye.xyz;
    let vz = dot(rel, skyu.cam_fwd.xyz);
    if (vz < skyu.cluster.x || vz > skyu.cluster.y) {
        return r;
    }
    let vx = dot(rel, skyu.cam_right.xyz);
    let vy = dot(rel, skyu.cam_up.xyz);
    let ndc = vec2<f32>(vx / (vz * skyu.cluster.z * skyu.cluster.w), vy / (vz * skyu.cluster.z));
    let g = skyu.lights.yzw;
    let tx = u32(clamp((ndc.x * 0.5 + 0.5) * f32(g.x), 0.0, f32(g.x) - 1.0));
    let ty = u32(clamp((0.5 - ndc.y * 0.5) * f32(g.y), 0.0, f32(g.y) - 1.0));
    let tz = u32(clamp(log(vz / skyu.cluster.x) / log(skyu.cluster.y / skyu.cluster.x) * f32(g.z), 0.0, f32(g.z) - 1.0));
    let c = (tz * g.y + ty) * g.x + tx;
    let count = min(sky_clusters[c], SKY_CLUSTER_CAP);
    let base = g.x * g.y * g.z + c * SKY_CLUSTER_CAP;
    let cine = SF_CINEMATIC && sky_on(skyu.flags.x, SKY_CINEMATIC);
    let qi = vec4<f32>(-a.rot.xyz, a.rot.w);
    let inv = 1.0 / max(a.scale, 1e-3);
    for (var k = 0u; k < count; k = k + 1u) {
        let li = sky_lights[sky_clusters[base + k]];
        let d = li.pos - world;
        let d2 = max(dot(d, d), 0.04);
        let dist = sqrt(d2);
        let l = d / dist;
        let w = clamp(1.0 - pow(dist / li.range, 4.0), 0.0, 1.0);
        let nl = max(dot(n, l), 0.0);
        if (nl <= 0.0 || w <= 0.0) {
            continue;
        }
        let sh = ag_ct_lamp_shape(li, l, dist, cine);
        let e = li.color * (li.intensity * sh.x * w * w / d2 * nl);
        var map = 1.0;
        if (LS_SHADER && li.pad >= 1.0) {
            map = ls_light_visibility(u32(li.pad) - 1u, world, n);
        }
        let car = ag_ct_lamp_vis(p, ag_rotate(qi, li.pos - a.pos) * inv, sh.y * inv, b);
        r.lit = r.lit + e * map;
        r.left = r.left + e * min(map, car);
    }
    return r;
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
    // (`--car-wet`) the wet street's mirror of the lights the car stands in front of
    var wet = 1.0;
    if (SF_CAR_WET && ag_ct_wet_on()) {
        wet = ag_ct_wet(world, n, p, a, array<vec3<f32>, 6>(lower_lo, lower_hi, upper_lo, upper_hi, lower_lo, lower_hi));
    }
    let wet_out = vec4<f32>(vec3<f32>(wet), 1.0);
    // how far outside the body's footprint (0 under it).
    let out_x = max(abs(p.x) - s.lower.x, 0.0);
    let out_z = max(max(s.lower.z - p.z, p.z - s.lower.w), 0.0);
    let away = length(vec2<f32>(out_x, out_z));
    let fade = 1.0 - smoothstep(AG_CT_REACH * 0.6, AG_CT_REACH, away);
    if (fade <= 0.0) {
        return wet_out;
    }
    let body = 1.0 - (1.0 - ag_ct_box(p, lower_lo, lower_hi)) * (1.0 - ag_ct_box(p, upper_lo, upper_hi));
    var tyres = 0.0;
    let wr = max(m.wheel_radius, 0.2);
    // the nearest tyre's box (the lamps' shadows take it with the bodies)
    var near_lo = vec3<f32>(0.0, -1.0, 0.0);
    var near_hi = vec3<f32>(0.0, -0.9, 0.0);
    var near_d = 1e9;
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
        let dw = distance(p.xz, w.xz);
        if (dw < near_d) {
            near_d = dw;
            near_lo = tlo;
            near_hi = thi;
        }
    }
    let occ = (1.0 - (1.0 - body) * (1.0 - min(tyres, 1.0))) * up * fade;
    let vis = 1.0 - occ;
    // What lit the point (white, as look_light lights it): the key through its
    // shadow and the street's lamps stand (under the body the lamps are the
    // car's to block); the sky, the ambient, the rim and the bounce the car takes.
    let key = look.key.xyz * max(dot(n, look.key_dir.xyz), 0.0) * lk_shadow(world, n) * sky_cloud_transmittance(world);
    let lamps = ag_ct_lamps(world, n, p, a, array<vec3<f32>, 6>(lower_lo, lower_hi, upper_lo, upper_hi, near_lo, near_hi));
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
    let stand = key;
    // the lamps' shadows of the car fade out with the reach as the occlusion does (the maps carry them on)
    let lamps_left = mix(lamps.lit, lamps.left, up * fade);
    let taken = irr * ao + rim + bounce + lamps.lit;
    let left = irr * min(ao, vis) + (rim + bounce) * vis + lamps_left;
    if (occ < 0.004 && all(lamps_left >= lamps.lit * 0.996)) {
        return wet_out;
    }
    let k = (stand + left) / max(stand + taken, vec3<f32>(1e-6));
    return vec4<f32>(clamp(k, vec3<f32>(0.0), vec3<f32>(1.0)) * wet, 1.0);
}

// ---- (`--car-wet`; SF_CAR_WET compiles it out) The wet street's mirror of
// the lights behind a car. The wet film reflects the frame's brightest lights
// in long streaks down the road toward the eye (sky_light.wgsl
// `sky_cine_streaks`), each from the points whose mirrored ray finds the
// light; where the car stands between such a point and its light, the
// point mirrors the car instead (a dark body, its own lamps kept), so the
// streak is cut and the car's mirror image stands in the road. Each streak
// light's reflection at the point is reckoned as the street reckons it (the
// film's base lobe; its puddles' sharper lobes are left out) with the share
// of the light's sphere the car's bodies leave (`ag_ct_lamp_vis`, as for
// the lamps' shadows), and the pixel is multiplied by what is left of the
// point's radiance: the film's mirror of the lights over it plus the street's
// own diffuse light (its wet albedo under the sky, the ambient and the
// clusters' lamps, unshadowed).

// The box reaches toward the eye along the road this far (m), stopping this
// short of the eye's foot, its height grown by this grade along it (the road
// rising or falling).
const AG_CT_WET_REACH: f32 = 30.0;
const AG_CT_WET_NEAR: f32 = 2.5;
const AG_CT_WET_GRADE: f32 = 0.15;
// The road's plane in the car's frame: within this of it at the car (a kerb's top is 0.15 m up).
const AG_CT_WET_ROAD_M: f32 = 0.35;
// Wet asphalt's albedo (dry ~0.1 darkened by the film, sky_light.wgsl SKY_CINE_WET_DARKEN.x).
const AG_CT_WET_ALBEDO: f32 = 0.045;
// The road beside a streak: this far across it either way (m; a streak's core is a few centimetres to a few decimetres across).
const AG_CT_WET_SIDE_M: f32 = 0.6;
// A streak light's reflection under this share of the point's radiance is not tested against the car.
const AG_CT_WET_MIN: f32 = 0.003;

// Whether the cars' wet terms are on (`--car-wet`, agents.rs `AgParams::misc.z` bit 1) and the street is wet under the look.
fn ag_ct_wet_on() -> bool {
    return SF_CAR_WET && SKY_CINE_FULL && (ag.misc.z & 2u) != 0u && sky_on(skyu.flags.x, SKY_WET) && sky_on(skyu.flags.x, SKY_CINEMATIC)
        && sky_on(skyu.flags.x, SKY_LIGHTS) && skyu.lights.x != 0u && skyu.wet.x >= 0.02;
}

// A streak light's emission toward the point over its axis's and its source's radius (m), as `sky_cine_streaks` shapes them.
fn ag_ct_streak_shape(li: SkyLight, l: vec3<f32>) -> vec2<f32> {
    let ca = dot(-l, li.dir);
    if (li.kind == 0u) {
        return vec2<f32>(max(sky_cine_cobra(l, li.dir), SKY_CINE_STREAK_LENS), SKY_CINE_SRC_STREET);
    }
    if (li.kind == 3u) {
        return vec2<f32>(max(smoothstep(li.cos_outer, li.cos_inner, ca), SKY_CINE_STREAK_LENS), SKY_CINE_SRC_STREET);
    }
    if (li.kind == 1u) {
        let dep = asin(clamp(l.y, -1.0, 1.0)) * 57.29578;
        let hl = normalize(vec2<f32>(-l.x, -l.z) + vec2<f32>(1e-6, 0.0));
        let ha = normalize(vec2<f32>(li.dir.x, li.dir.z) + vec2<f32>(1e-6, 0.0));
        let hr = vec2<f32>(-ha.y, ha.x);
        let side = smoothstep(li.cos_outer, li.cos_inner, dot(hl, ha));
        return vec2<f32>(max(sky_cine_low_beam(dep, atan2(dot(hl, hr), dot(hl, ha)) * 57.29578) * side, SKY_CINE_STREAK_HEAD_LENS * smoothstep(-0.2, 0.2, dot(hl, ha))), SKY_CINE_SRC_HEADLAMP);
    }
    if (li.kind >= 4u) {
        return vec2<f32>(smoothstep(li.cos_outer, li.cos_inner, ca), 0.5 * sqrt(max(li.size, 0.0)));
    }
    return vec2<f32>(1.0, SKY_CINE_SRC_STREET);
}

// The share of the point's radiance (luminance) the car leaves it: 1 where it
// mirrors no streak light the car hides. `p` the point in the car's frame, `b`
// the car's boxes (lower, upper, a tyre).
fn ag_ct_wet(world: vec3<f32>, n: vec3<f32>, p: vec3<f32>, a: AgAgent, b: array<vec3<f32>, 6>) -> f32 {
    // the road (and a kerb's top) round the car: its plane, rising or falling a little away from it (not a bonnet)
    let off = length(max(abs(p.xz - 0.5 * (b[0].xz + b[1].xz)) - 0.5 * (b[1].xz - b[0].xz), vec2<f32>(0.0)));
    if (n.y < 0.7 || abs(p.y) > AG_CT_WET_ROAD_M + AG_CT_WET_GRADE * off) {
        return 1.0;
    }
    var w = skyu.wet.x;
    if (sky_on(skyu.flags.x, SKY_DAMP)) {
        w = w * sky_damp_left(world.xz, -1.0); // (`--weather damp`) the drying patches, as the street's
    }
    if (w < 0.02) {
        return 1.0;
    }
    let v = normalize(skyu.eye.xyz - world);
    var t = v - n * dot(v, n);
    if (dot(t, t) < 1e-8) {
        t = cross(n, vec3<f32>(1.0, 0.0, 0.0));
    }
    t = normalize(t);
    let g = skyu.lights.yzw;
    let base = g.x * g.y * g.z * (1u + SKY_CLUSTER_CAP);
    let count = min(sky_clusters[base], SKY_STREAK_CAP);
    let qi = vec4<f32>(-a.rot.xyz, a.rot.w);
    let inv = 1.0 / max(a.scale, 1e-3);
    var sum = 0.0;
    var hidden = 0.0;
    var lights = array<vec2<f32>, 32>();
    for (var k = 0u; k < count; k = k + 1u) {
        let li = sky_lights[sky_clusters[base + 1u + k]];
        let d = li.pos - world;
        let d2 = max(dot(d, d), 0.04);
        let dist = sqrt(d2);
        let l = d / dist;
        if (dot(n, l) <= 0.0) {
            continue;
        }
        let sh = ag_ct_streak_shape(li, l);
        let wide = sh.y / (2.0 * dist);
        let f = sky_cine_film(n, v, l, t, min(SKY_CINE_STREAK_ALONG + wide, 1.0), min(SKY_CINE_STREAK_ACROSS + wide, 1.0));
        let e = lk_luma(li.color) * li.intensity * sh.x / d2;
        let r = e * min(f, d2 / (LK_PI * sh.y * sh.y)) * w;
        sum = sum + r;
        lights[k] = vec2<f32>(r, sh.y);
    }
    if (sum <= 0.0) {
        return 1.0;
    }
    // the street's own light at the point (white, unshadowed): the sky, the ambient, the key and the clusters' lamps
    let env_i = look.ambient.w * look.env_class.z;
    let hemi = mix(look.hemi_ground.xyz, look.hemi_sky.xyz, n.y * 0.5 + 0.5);
    var irr = lk_luma(hemi + look.ambient.xyz + lk_sh(n) * env_i) + lk_luma(look.key.xyz) * max(dot(n, look.key_dir.xyz), 0.0);
    irr = irr + ag_ct_wet_lamps(world, n);
    var own = AG_CT_WET_ALBEDO / LK_PI * irr;
    // the road beside the streaks in the previous frame (its mirror of the sky,
    // the facades and the cars, which the film shows over its diffuse): the
    // lesser of the two points across the streak, where the history holds it
    if (sky_on(skyu.flags.x, SKY_SSR) && skyu.flags.z != 0u) {
        let across = normalize(cross(n, t)) * AG_CT_WET_SIDE_M;
        own = max(own, min(ag_ct_prev_luma(world + across), ag_ct_prev_luma(world - across)));
    }
    let total = own + sum;
    for (var k = 0u; k < count; k = k + 1u) {
        let r = lights[k].x;
        if (r <= AG_CT_WET_MIN * total) {
            continue;
        }
        let li = sky_lights[sky_clusters[base + 1u + k]];
        hidden = hidden + r * (1.0 - ag_ct_lamp_vis(p, ag_rotate(qi, li.pos - a.pos) * inv, lights[k].y * inv, b));
    }
    return clamp(1.0 - hidden / total, 0.0, 1.0);
}

// The previous resolved frame's luminance at a world point (its level 1, a
// little wider than a pixel), 1e9 off the frame (the other side decides).
fn ag_ct_prev_luma(p: vec3<f32>) -> f32 {
    let q = sky_ssr_cine_uv(p);
    if (q.z < 0.0 || any(q.xy < vec2<f32>(0.0)) || any(q.xy > vec2<f32>(1.0))) {
        return 1e9;
    }
    return lk_luma(textureSampleLevel(sky_prev, sky_rep, q.xy, 1.0).rgb);
}

// The clusters' lamps' irradiance (luminance) at a point facing `n`, shaped as the city's shading shapes them, without their maps.
fn ag_ct_wet_lamps(world: vec3<f32>, n: vec3<f32>) -> f32 {
    let rel = world - skyu.eye.xyz;
    let vz = dot(rel, skyu.cam_fwd.xyz);
    if (vz < skyu.cluster.x || vz > skyu.cluster.y) {
        return 0.0;
    }
    let vx = dot(rel, skyu.cam_right.xyz);
    let vy = dot(rel, skyu.cam_up.xyz);
    let ndc = vec2<f32>(vx / (vz * skyu.cluster.z * skyu.cluster.w), vy / (vz * skyu.cluster.z));
    let g = skyu.lights.yzw;
    let tx = u32(clamp((ndc.x * 0.5 + 0.5) * f32(g.x), 0.0, f32(g.x) - 1.0));
    let ty = u32(clamp((0.5 - ndc.y * 0.5) * f32(g.y), 0.0, f32(g.y) - 1.0));
    let tz = u32(clamp(log(vz / skyu.cluster.x) / log(skyu.cluster.y / skyu.cluster.x) * f32(g.z), 0.0, f32(g.z) - 1.0));
    let c = (tz * g.y + ty) * g.x + tx;
    let count = min(sky_clusters[c], SKY_CLUSTER_CAP);
    let base = g.x * g.y * g.z + c * SKY_CLUSTER_CAP;
    var e = 0.0;
    for (var k = 0u; k < count; k = k + 1u) {
        let li = sky_lights[sky_clusters[base + k]];
        let d = li.pos - world;
        let d2 = max(dot(d, d), 0.04);
        let dist = sqrt(d2);
        let l = d / dist;
        let wr = clamp(1.0 - pow(dist / li.range, 4.0), 0.0, 1.0);
        let nl = max(dot(n, l), 0.0);
        if (nl <= 0.0 || wr <= 0.0) {
            continue;
        }
        e = e + lk_luma(li.color) * li.intensity * ag_ct_lamp_shape(li, l, dist, true).x * wr * wr / d2 * nl;
    }
    return e;
}
