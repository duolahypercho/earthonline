// Appended to the agents' colour shader (agents.rs) only with `--people-capsules`,
// in people_contact.wgsl's place: the same pipeline (`agent-contact`) and entry
// points, drawn after the agents' opaque list in the city pass.

// ---- the ground round a person darkened by their body (`--people-capsules`;
// agents.rs `people_capsules`) ----
// A person is thirteen capsules riding their skeleton (eo-agents
// `people::add_capsule_anchors`: each fitted to the vertices its joints carry
// in the bind pose, its axis their principal axis, its radius their RMS
// distance from it): the shoes heel to toe, the shins, the thighs, the hips,
// the chest, the head, the upper arms, the forearms with the hands, posed
// each frame by the joint matrices the body is skinned with. On the ground
// round them (a quad within `PK_REACH_M` of the eye, at the feet's height)
// they take away light two ways:
//
// * the sky (and the rest of the ambient light): the share of the
//   cosine-weighted hemisphere over a ground point a capsule covers, in closed
//   form. A cylinder of radius r is, seen from afar, a strip 2 r wide; along
//   its axis s(t) = a + t u the strip's element covers 2 r sin(psi) dt / rho^2
//   of solid angle at cosine rho_y / rho, which integrates exactly (tau the
//   distance along the axis from the foot of the perpendicular, c the
//   perpendicular's length, h its height):
//     O = (r / pi) [ h (tau / (c (tau^2 + c^2)) + atan(tau / c) / c^2) - u_y c / (tau^2 + c^2) ]
//   between the ends, the axis lengthened by pi r / 4 at each (the caps: a
//   capsule's projected area across its axis is 2 r L + pi r^2) and c held at
//   r or more (inside the capsule as at its surface). A standing body hides
//   about 15 % of the sky 0.3 m from its axis, 9 % at 0.5 m, 4 % at 1 m, and
//   nearly all of it under a shoe: a tight dark line where the soles meet the
//   paving and a soft halo half a metre round, in shade as in the sun. The
//   capsules' visibilities multiply.
// * each street lamp and headlamp of the point's light cluster (the frame's
//   own lights, their shape and reach as sky_light.wgsl lights the ground):
//   the share of the lamp's emitting disc (its source radius, as the
//   cinematic look sizes it: `SKY_CINE_SRC_STREET`, `SKY_CINE_SRC_HEADLAMP`,
//   a shop window half its glazed side) each capsule covers seen from the
//   point, two discs overlapping in the plane across the ray where the
//   capsule's axis passes nearest it (the lamp's cone radius there src t / d):
//   a person's own shadow from the lamps at night, sharp at the shoes and
//   softening toward the head's end. A lamp whose map already holds the
//   people (a headlamp's, `--headlight-shadows`) is left to it, and a lamp's
//   own map (the static scene) still shades the point first.
//
// The frame there is multiplied by 1 - (light lost) / (light that reached it),
// both as the ground's diffuse irradiance: the ambient (sky, hemisphere,
// environment), the sun's or moon's key at its shadow and clouds, and each
// lamp at its map's visibility (the maps read at the ground point lifted as
// the quad is: at the feet's height a paving a centimetre or two higher would
// read as the point's own shadow); under the fog it found. Each fragment's
// ground point is where the view ray through it meets the ground's plane, not
// the quad's own point over it (`pk_factor`).
const PK_PARTS: u32 = 13u;
// the joint posing each capsule (eo-agents people `CAPSULE_JOINTS`): the left
// and right shoes on the feet, the calves, the thighs, the pelvis, Spine1 (the
// chest), the head, the upper arms, the forearms.
const PK_JOINTS: array<u32, 13> = array<u32, 13>(49u, 53u, 48u, 52u, 47u, 51u, 0u, 2u, 5u, 10u, 29u, 11u, 30u);
// the quad's half side round the feet: by day the sky's share (4 % 1 m from a
// standing body, 2.7 % at 1.2 m; faded out from `PK_SKY_FADE_M` so the quad
// leaves no edge), with the lamps lit as far as their shadows are drawn,
// faded out over the last `PK_LAMP_FADE_M`.
const PK_SKY_REACH_M: f32 = 1.3;
const PK_SKY_FADE_M: f32 = 0.8;
const PK_LAMP_REACH_M: f32 = 3.2;
const PK_LAMP_FADE_M: f32 = 0.8;
const PK_REACH_M: f32 = 90.0;
const PK_LIFT_M: f32 = 0.03;
// The capsules' ends packed to the vertex stage's flat outputs: x and z in
// 11 bits over +-PK_BOX_M round the feet (1.3 mm), y in 10 bits from
// PK_BOX_FLOOR under them to PK_BOX_H over that (2.3 mm), the radius in 8
// bits of PK_R_STEP (2 mm, to 0.51 m).
const PK_BOX_M: f32 = 1.3;
const PK_BOX_FLOOR: f32 = -0.2;
const PK_BOX_H: f32 = 2.4;
const PK_R_STEP: f32 = 0.002;
// a body's bounding sphere over the feet (centre height, radius): a lamp
// whose ray from the point passes wider of it than its cone is not tested.
const PK_BODY_C: f32 = 0.95;
const PK_BODY_R: f32 = 1.05;

struct PkOut {
    @builtin(position) clip: vec4<f32>,
    // xyz the quad's point, w the ground's height under the person
    @location(0) world: vec4<f32>,
    // x, z the feet's middle, y the packing box's floor, w the quad's half side
    @location(1) @interpolate(flat) body: vec4<f32>,
    @location(2) @interpolate(flat) c0: vec4<u32>,
    @location(3) @interpolate(flat) c1: vec4<u32>,
    @location(4) @interpolate(flat) c2: vec4<u32>,
    @location(5) @interpolate(flat) c3: vec4<u32>,
    @location(6) @interpolate(flat) c4: vec4<u32>,
    @location(7) @interpolate(flat) c5: vec4<u32>,
    @location(8) @interpolate(flat) c6: vec4<u32>,
    @location(9) @interpolate(flat) c7: vec4<u32>,
};

// Joint `j`'s skinning rows as ag_pose blends them for agent `a` (the walk and
// standing frames by ground speed).
fn pk_rows(a: AgAgent, m: AgModel, j: u32) -> mat3x4<f32> {
    let wn = ag.clip_frames.x;
    let wf = fract(a.anim) * f32(wn);
    let wf0 = u32(floor(wf)) % wn;
    let wf1 = (wf0 + 1u) % wn;
    let c = min(a.clip, 3u);
    let cn = ag.clip_frames[c];
    let cb = ag.clip_base[c];
    let cf = fract(a.clip_phase) * f32(cn);
    let cf0 = cb + u32(floor(cf)) % cn;
    let cf1 = cb + (u32(floor(cf)) + 1u) % cn;
    let ww = smoothstep(0.15, 0.60, a.speed);
    let wb = ag.clip_base.x;
    return ag_joint_rows(m.skeleton, m.joints, j, wb + wf0, wb + wf1, fract(wf), cf0, cf1, fract(cf), ww, false);
}

fn pk_posed(a: AgAgent, r: mat3x4<f32>, p: vec3<f32>) -> vec3<f32> {
    let p4 = vec4<f32>(p, 1.0);
    return a.pos + ag_rotate(a.rot, vec3<f32>(dot(r[0], p4), dot(r[1], p4), dot(r[2], p4)) * a.scale);
}

fn pk_pack(p: vec3<f32>, o: vec3<f32>) -> u32 {
    let q = clamp((p - o) / vec3<f32>(2.0 * PK_BOX_M, PK_BOX_H, 2.0 * PK_BOX_M), vec3<f32>(0.0), vec3<f32>(1.0));
    return u32(round(q.x * 2047.0)) | (u32(round(q.y * 1023.0)) << 11u) | (u32(round(q.z * 2047.0)) << 21u);
}

fn pk_unpack(w: u32, o: vec3<f32>) -> vec3<f32> {
    let q = vec3<f32>(f32(w & 2047u) / 2047.0, f32((w >> 11u) & 1023u) / 1023.0, f32(w >> 21u) / 2047.0);
    return o + q * vec3<f32>(2.0 * PK_BOX_M, PK_BOX_H, 2.0 * PK_BOX_M);
}

@vertex
fn agent_contact_vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> PkOut {
    var out: PkOut;
    out.clip = vec4<f32>(2.0, 2.0, 2.0, 1.0); // nothing drawn
    if (ii >= ag.counts.x) {
        return out;
    }
    let a = ag_agents[ii];
    let m = ag_models[a.model];
    if (m.model_class != AG_PERSON || m.pad.y < PK_PARTS || distance(a.pos, frame.cam_pos.xyz) > PK_REACH_M) {
        return out;
    }
    var ends: array<vec3<f32>, 26>;
    var w: array<u32, 32>;
    var c = vec2<f32>(0.0);
    var joints = PK_JOINTS;
    for (var k = 0u; k < PK_PARTS; k = k + 1u) {
        let ea = ag_bones[m.pad.x + 2u * k];
        let eb = ag_bones[m.pad.x + 2u * k + 1u];
        let r = pk_rows(a, m, joints[k]);
        ends[2u * k] = pk_posed(a, r, ea.xyz);
        ends[2u * k + 1u] = pk_posed(a, r, eb.xyz);
        let rq = min(u32(round(ea.w * a.scale / PK_R_STEP)), 255u);
        w[26u + k / 4u] = w[26u + k / 4u] | (rq << (8u * (k % 4u)));
        if (k < 2u) {
            c = c + (ends[2u * k].xz + ends[2u * k + 1u].xz) * 0.25;
        }
    }
    let floor_y = a.pos.y + PK_BOX_FLOOR;
    let o = vec3<f32>(c.x - PK_BOX_M, floor_y, c.y - PK_BOX_M);
    for (var k = 0u; k < 2u * PK_PARTS; k = k + 1u) {
        w[k] = pk_pack(ends[k], o);
    }
    let lamps = sky_on(skyu.flags.x, SKY_LIGHTS) && skyu.lights.x > 0u;
    let half = select(PK_SKY_REACH_M, PK_LAMP_REACH_M, lamps);
    var corner = array<vec2<f32>, 6>(vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0));
    let xz = c + corner[vi % 6u] * half;
    let p = vec3<f32>(xz.x, a.pos.y + PK_LIFT_M, xz.y);
    out.clip = frame.view_proj * vec4<f32>(p, 1.0);
    out.world = vec4<f32>(p, a.pos.y);
    out.body = vec4<f32>(c.x, floor_y, c.y, half);
    out.c0 = vec4<u32>(w[0], w[1], w[2], w[3]);
    out.c1 = vec4<u32>(w[4], w[5], w[6], w[7]);
    out.c2 = vec4<u32>(w[8], w[9], w[10], w[11]);
    out.c3 = vec4<u32>(w[12], w[13], w[14], w[15]);
    out.c4 = vec4<u32>(w[16], w[17], w[18], w[19]);
    out.c5 = vec4<u32>(w[20], w[21], w[22], w[23]);
    out.c6 = vec4<u32>(w[24], w[25], w[26], w[27]);
    out.c7 = vec4<u32>(w[28], w[29], w[30], w[31]);
    return out;
}

// the capsules of the fragment's person: a's xyz and the radius, b
var<private> pk_a: array<vec4<f32>, 13>;
var<private> pk_b: array<vec3<f32>, 13>;

fn pk_decode(in: PkOut) {
    let o = vec3<f32>(in.body.x - PK_BOX_M, in.body.y, in.body.z - PK_BOX_M);
    let w = array<u32, 32>(
        in.c0.x, in.c0.y, in.c0.z, in.c0.w, in.c1.x, in.c1.y, in.c1.z, in.c1.w,
        in.c2.x, in.c2.y, in.c2.z, in.c2.w, in.c3.x, in.c3.y, in.c3.z, in.c3.w,
        in.c4.x, in.c4.y, in.c4.z, in.c4.w, in.c5.x, in.c5.y, in.c5.z, in.c5.w,
        in.c6.x, in.c6.y, in.c6.z, in.c6.w, in.c7.x, in.c7.y, in.c7.z, in.c7.w,
    );
    for (var k = 0u; k < PK_PARTS; k = k + 1u) {
        let r = f32((w[26u + k / 4u] >> (8u * (k % 4u))) & 255u) * PK_R_STEP;
        pk_a[k] = vec4<f32>(pk_unpack(w[2u * k], o), r);
        pk_b[k] = pk_unpack(w[2u * k + 1u], o);
    }
}

// The closed form's antiderivative's first part, tau / (c (tau^2 + c^2)) + atan(tau / c) / c^2.
fn pk_prim(t: f32, c: f32) -> f32 {
    return t / (c * (t * t + c * c)) + atan(t / c) / (c * c);
}

// The share of the cosine-weighted sky over ground point g (normal up) the
// capsule (a, b, r) hides (see above).
fn pk_sky(g: vec3<f32>, a: vec3<f32>, b: vec3<f32>, r: f32) -> f32 {
    if (r <= 0.0) {
        return 0.0;
    }
    let ab = b - a;
    let len = length(ab);
    var u = vec3<f32>(0.0, 1.0, 0.0);
    if (len > 1e-4) {
        u = ab / len;
    }
    let ext = 0.7853982 * r;
    let p0 = a - u * ext - g;
    let t0 = dot(p0, u);
    let t1 = t0 + len + 2.0 * ext;
    let perp = p0 - u * t0;
    let c = max(length(perp), r);
    let h = perp.y;
    let f1 = h * pk_prim(t1, c) - u.y * c / (t1 * t1 + c * c);
    let f0 = h * pk_prim(t0, c) - u.y * c / (t0 * t0 + c * c);
    return clamp(r / LK_PI * (f1 - f0), 0.0, 1.0);
}

// The share of a lamp's disc (radius `src`, `dist` away along unit `l` from
// g) the capsule (a, b, r) hides from g: the discs of the capsule and of the
// lamp's cone overlapping in the plane across the ray where the capsule's
// axis passes nearest it.
fn pk_lamp_cover(g: vec3<f32>, l: vec3<f32>, dist: f32, src: f32, a: vec3<f32>, b: vec3<f32>, r: f32) -> f32 {
    if (r <= 0.0) {
        return 0.0;
    }
    let d1 = b - a;
    let r0 = a - g;
    let aa = dot(d1, d1);
    let bb = dot(d1, l);
    let den = aa - bb * bb;
    var u = 0.0;
    if (den > 1e-8) {
        u = clamp((bb * dot(l, r0) - dot(d1, r0)) / den, 0.0, 1.0);
    }
    var q = a + d1 * u;
    if (aa > 1e-8) {
        let s = dot(q - g, l);
        u = clamp(dot(g + l * s - a, d1) / aa, 0.0, 1.0);
        q = a + d1 * u;
    }
    let t = dot(q - g, l);
    if (t <= 0.0 || t >= dist) {
        return 0.0;
    }
    let perp = length(q - g - l * t);
    let rl = max(src * t / dist, 1e-4);
    let most = min(1.0, (r * r) / (rl * rl));
    return most * (1.0 - smoothstep(abs(r - rl), r + rl, perp));
}

// A frame light's irradiance on the ground at g (normal up, luma, before its
// map's visibility; agents_draw.wgsl `pl_shape`, as sky_light.wgsl shapes it),
// the unit vector toward it and its distance.
struct PkLamp {
    e: f32,
    l: vec3<f32>,
    dist: f32,
};

fn pk_lamp(li: SkyLight, g: vec3<f32>, cine: bool) -> PkLamp {
    let d = li.pos - g;
    let dist = max(length(d), 1e-3);
    let l = d / dist;
    var out = PkLamp(0.0, l, dist);
    if (l.y > 0.0) {
        out.e = pl_shape(li, l, dist, cine) * l.y * lk_luma(li.color);
    }
    return out;
}

fn pk_factor(in: PkOut) -> f32 {
    // The ground point the fragment shows: the view ray through the quad's
    // point met with the ground's plane under the person. The quad stands
    // `PK_LIFT_M` over that plane (to pass the depth test), so its own point
    // lies nearer the eye than the ground seen through it by the lift times
    // the distance over the eye's height (18 cm at 10 m for an eye 1.7 m up):
    // taken there, the line at the soles was drawn that far behind them.
    let up = vec3<f32>(0.0, 1.0, 0.0);
    let eye = frame.cam_pos.xyz;
    var g = vec3<f32>(in.world.x, in.world.w, in.world.z);
    let drop = eye.y - in.world.y;
    if (drop > 1e-3 && eye.y > in.world.w) {
        g = eye + (in.world.xyz - eye) * ((eye.y - in.world.w) / drop);
    }
    // the maps (lamps, the key) read at that point lifted as the quad is
    let lifted = g + up * PK_LIFT_M;
    let off = g.xz - in.body.xz;
    let reach = max(abs(off.x), abs(off.y));
    pk_decode(in);
    // the sky's share
    var sky_vis = 1.0;
    let rd = length(off);
    if (rd < PK_SKY_REACH_M) {
        for (var k = 0u; k < PK_PARTS; k = k + 1u) {
            sky_vis = sky_vis * (1.0 - pk_sky(g, pk_a[k].xyz, pk_b[k], pk_a[k].w));
        }
        sky_vis = 1.0 - (1.0 - sky_vis) * (1.0 - smoothstep(PK_SKY_FADE_M, PK_SKY_REACH_M, rd));
    }
    let env_i = look.ambient.w * look.env_class.w;
    let e_amb = lk_luma(look.hemi_sky.xyz + look.ambient.xyz + lk_sh(up) * env_i);
    var lost = e_amb * (1.0 - sky_vis);
    var e_lamps = 0.0;
    // the lamps: the point's light cluster (as sky_local_lights_ao finds it)
    let fade = 1.0 - smoothstep(in.body.w - PK_LAMP_FADE_M, in.body.w, reach);
    if (fade > 0.0 && sky_on(skyu.flags.x, SKY_LIGHTS) && skyu.lights.x != 0u && frame.copy_step.z < 0.5) {
        let cl = pl_cluster(g);
        if (cl.y > 0u) {
            let count = cl.y;
            let base = cl.x;
            let cine = SF_CINEMATIC && sky_on(skyu.flags.x, SKY_CINEMATIC);
            let m = vec3<f32>(in.body.x, in.world.w + PK_BODY_C, in.body.z);
            for (var i = 0u; i < count; i = i + 1u) {
                let li = sky_lights[sky_clusters[base + i]];
                let lp = pk_lamp(li, g, cine);
                if (lp.e <= 0.0) {
                    continue;
                }
                var e = lp.e;
                // a headlamp's own map holds the people already
                let mapped = LS_SHADER && li.pad >= 1.0;
                if (!(mapped && li.kind == 1u)) {
                    let src = select(select(SKY_CINE_SRC_STREET, SKY_CINE_SRC_HEADLAMP, li.kind == 1u), 0.5 * sqrt(max(li.size, 0.0)), li.kind >= 4u);
                    let tm = dot(m - g, lp.l);
                    let pm = length(m - g - lp.l * tm);
                    if (tm > -PK_BODY_R && tm < lp.dist + PK_BODY_R && pm < PK_BODY_R + src * max(tm, 0.0) / lp.dist) {
                        var vis = 1.0;
                        for (var k = 0u; k < PK_PARTS; k = k + 1u) {
                            vis = vis * (1.0 - pk_lamp_cover(g, lp.l, lp.dist, src, pk_a[k].xyz, pk_b[k], pk_a[k].w));
                        }
                        if (vis < 0.999) {
                            if (mapped) {
                                e = e * ls_light_visibility(u32(li.pad) - 1u, lifted, up);
                            }
                            lost = lost + e * (1.0 - vis) * fade;
                        }
                    }
                }
                e_lamps = e_lamps + e;
            }
        }
    }
    if (lost <= 0.0) {
        return 1.0;
    }
    var e_key = 0.0;
    if (look.key_dir.y > 0.0) {
        e_key = lk_luma(look.key.xyz) * look.key_dir.y * lk_shadow(lifted, up) * sky_cloud_transmittance(g);
    }
    let vd = dot(g - look.cam_pos.xyz, look.cam_fwd.xyz);
    let fog = lk_smooth(look.fog.x, look.fog.y, vd);
    return 1.0 - clamp(lost / max(e_amb + e_key + e_lamps, 1e-6), 0.0, 1.0) * (1.0 - fog);
}

@fragment
fn agent_contact_fs(in: PkOut) -> @location(0) vec4<f32> {
    return vec4<f32>(vec3<f32>(pk_factor(in)), 1.0);
}

@fragment
fn agent_contact_fs_ids(in: PkOut) -> AgIdsOut {
    var out: AgIdsOut;
    out.color = vec4<f32>(vec3<f32>(pk_factor(in)), 1.0);
    out.ids = vec2<u32>(0u, 0u);
    return out;
}
