// Moving things in the city pass: appended to draw.wgsl + look_common.wgsl +
// look_light.wgsl + agents_common.wgsl (agents.rs). Group 0 has draw.wgsl's
// layout but the agent library's own buffers (frame, one object entry for the
// look's material class, the agent materials, the civilian atlases); group 1
// is the look's; group 2 the agent data. Lighting, shadows and fog are the
// city's own `look_light`.

const AG_M_PAINT: u32 = 256u;
const AG_M_TAIL: u32 = 512u;
const AG_M_BRAKE: u32 = 1024u;
const AG_M_INDICATOR: u32 = 2048u;
const AG_M_HEAD: u32 = 4096u;
const AG_M_DRL: u32 = 8192u;
const AG_M_REVERSE: u32 = 16384u;
const AG_M_ATLAS: u32 = 32768u;
// eo_agents AGENT_IN_BORE, BORE_DEPTH_SHIFT, BORE_DEPTH_QUANT (agents.rs tests them):
// a car in a tunnel's bore and how far in from its portal.
const AG_IN_BORE: u32 = 262144u;
const AG_BORE_DEPTH_SHIFT: u32 = 20u;
const AG_BORE_DEPTH_QUANT: f32 = 4.0;

struct AgOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) local: vec3<f32>,
    @location(4) @interpolate(flat) material: u32,
    @location(5) @interpolate(flat) agent: u32,
};

@vertex
fn agent_vs(
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) joints: vec4<u32>,
    @location(4) weights: vec4<f32>,
    @location(5) tag: u32,
    @builtin(instance_index) inst: u32,
) -> AgOut {
    let pose = ag_pose(ag_visible[inst], pos, normal.xyz, joints, weights, tag);
    var out: AgOut;
    out.clip = frame.view_proj * vec4<f32>(pose.world, 1.0);
    out.world = pose.world;
    out.normal = pose.normal;
    out.uv = uv;
    out.local = pose.local;
    out.material = tag & 0xffffu;
    out.agent = pose.agent;
    return out;
}

fn ag_unpack_srgb(c: u32) -> vec3<f32> {
    let s = vec3<f32>(f32(c & 255u), f32((c >> 8u) & 255u), f32((c >> 16u) & 255u)) / 255.0;
    return select(pow((s + 0.055) / 1.055, vec3<f32>(2.4)), s / 12.92, s <= vec3<f32>(0.04045));
}

// The lamp channels (`heroLampState` with `HERO_LAMP`): intensity for the
// agent's state between the day and night values.
fn ag_lamp(flags: u32, mflags: u32, local_x: f32) -> f32 {
    let night = ag.lamp_misc.x;
    let tail = mix(ag.lamp_tail.x, ag.lamp_tail.y, night);
    let brake = mix(ag.lamp_tail.z, ag.lamp_tail.w, night);
    let head = mix(ag.lamp_head.x, ag.lamp_head.y, night);
    let ind = mix(ag.lamp_head.z, ag.lamp_head.w, night);
    let braking = (flags & AG_FLAG_BRAKING) != 0u;
    var i = 0.0;
    if ((mflags & AG_M_TAIL) != 0u) {
        // No separate brake material on most bodies: the tail lens lights for the brake.
        i = max(tail, select(0.0, brake, braking));
    }
    if ((mflags & AG_M_BRAKE) != 0u && braking) {
        i = max(i, brake);
    }
    if ((mflags & AG_M_HEAD) != 0u) {
        i = max(i, head);
    }
    if ((mflags & AG_M_DRL) != 0u) {
        i = max(i, max(head, ag.lamp_misc.y));
    }
    if ((mflags & AG_M_INDICATOR) != 0u && (flags & AG_BLINK_ON) != 0u) {
        // +x is the body's left flank.
        let left = local_x > 0.0;
        if ((left && (flags & AG_FLAG_TURN_LEFT) != 0u) || (!left && (flags & AG_FLAG_TURN_RIGHT) != 0u)) {
            i = max(i, ind);
        }
    }
    if ((mflags & AG_M_REVERSE) != 0u) {
        i = max(i, ag.lamp_misc.w);
    }
    return i;
}

// Weathering (`--grime`, weathering.rs; reached only with the level on): a
// car's paint dusty on the lower body up to about the wheels' tops, grimy
// round the wheel arches, older cars' paint dull; each car its own (some
// clean, most a little dirty, a few filthy), keyed on its colour and model.
fn ag_weather(s: ptr<function, Surface>, local: vec3<f32>, a: AgAgent, fw: f32) {
    let m = ag_models[a.model];
    if (m.model_class == AG_PERSON || m.wheel_radius <= 0.05) {
        return;
    }
    let hs = vec2<f32>(f32(a.color & 0xfffu), f32(a.model) + f32((a.color >> 12u) & 0xfffu) * 0.37);
    let h = st_hash(hs + vec2<f32>(3.3, 1.7));
    let dirt = select(select(WX_CAR_SOME, WX_CAR_FILTHY, h > 1.0 - WX_CAR_FILTHY_SHARE), WX_CAR_CLEAN, h < WX_CAR_CLEAN_SHARE);
    let r = m.wheel_radius;
    let y = local.y - (m.wheel[0].y - r);
    let ragged = mix(0.5, st_noise(local.xz / 0.22 + local.y * 3.0 + h * 9.0), wx_keep(0.22, fw));
    let dust = (1.0 - smoothstep(r * 1.2, r * 2.5, y + (ragged - 0.5) * r * 0.9)) * dirt;
    let arch = 1.0 - smoothstep(r * 1.04, r * 1.8, min(length(local.yz - m.wheel[0].yz), length(local.yz - m.wheel[2].yz)) + (ragged - 0.5) * r * 0.5);
    let grime = arch * (0.35 + 0.65 * dirt);
    (*s).albedo = mix((*s).albedo, WX_CAR_DUST, clamp(dust * WX_CAR_DUST_COVER, 0.0, 1.0));
    (*s).albedo = mix((*s).albedo, WX_CAR_GRIME, clamp(grime * WX_CAR_GRIME_COVER, 0.0, 1.0));
    let old = smoothstep(0.45, 1.0, st_hash(hs + vec2<f32>(7.7, 2.1))) * select(0.0, 1.0, (*s).metal < 0.5 || (*s).rough > 0.2);
    (*s).rough = clamp(mix((*s).rough, 0.85, max(dust, grime) * 0.8) + WX_CAR_DULL * old, 0.03, 1.0);
}

// ---- another player's ground contact (eo-agents `rig::contact_instances`,
// material flag M_CONTACT; the glass pass only). The instance is a quad on
// the ground around one box of the body (a shoe, half a shin or thigh, a
// crouch's seat): `anim` half width, `steer` half length, `speed` top,
// `clip_phase` bottom (metres over the ground), `color` the weight (low
// byte) and the sun bit (bit 8). The patch darkens the ground under it
// (blended black: the ground times 1 - alpha) by the share of its light the
// box takes away: the sky the box hides (its exact view factor, mirrored by
// `rig::box_view_factor`) of the indirect and rim light, and for a low box
// the sun it blocks (a ray to the sun against the box), out of all the
// light this renderer puts there (the key with its cascades, clouds and
// screen-space contact ray, the rim, the sky and ambient with the
// screen-space occlusion). Physical in sun and in shade alike: a sole's
// edge loses the sun only on its shadow side.
const AG_M_CONTACT: u32 = 1048576u;
const AG_INV_2PI: f32 = 0.15915494;

// Howell's catalogue: a plane element to a rectangle in a plane at 90 degrees
// to it, h high and b wide, its bottom corner at the foot of the element's
// perpendicular, c away.
fn ag_vf_wall(c: f32, h: f32, b: f32) -> f32 {
    if (h <= 0.0 || b <= 0.0) {
        return 0.0;
    }
    let x = h / b;
    let y = max(c, 1e-4) / b;
    let s = sqrt(x * x + y * y);
    return AG_INV_2PI * (atan(1.0 / y) - y / s * atan(1.0 / s));
}

// Howell's catalogue: a plane element to a parallel rectangle a by b, c over
// it, the element's normal through one corner.
fn ag_vf_ceiling(c: f32, a: f32, b: f32) -> f32 {
    if (a <= 0.0 || b <= 0.0) {
        return 0.0;
    }
    let x = a / max(c, 1e-4);
    let y = b / max(c, 1e-4);
    let sx = sqrt(1.0 + x * x);
    let sy = sqrt(1.0 + y * y);
    return AG_INV_2PI * (x / sx * atan(y / sx) + y / sy * atan(x / sy));
}

// A wall face from h0 to h1, its lateral edge `e` from the foot (signed).
fn ag_vf_wall_band(c: f32, h0: f32, h1: f32, e: f32) -> f32 {
    let u = abs(e);
    var f = ag_vf_wall(c, h1, u);
    if (h0 > 0.002) {
        f = f - ag_vf_wall(c, h0, u);
    }
    return select(-f, f, e >= 0.0);
}

fn ag_vf_side(c: f32, t: f32, half_w: f32, h0: f32, h1: f32) -> f32 {
    return ag_vf_wall_band(c, h0, h1, half_w - t) - ag_vf_wall_band(c, h0, h1, -half_w - t);
}

fn ag_vf_corner(c: f32, u: f32, v: f32) -> f32 {
    return sign(u) * sign(v) * ag_vf_ceiling(c, abs(u), abs(v));
}

fn ag_box_view_factor(px: f32, pz: f32, hx: f32, hz: f32, h0: f32, h1: f32) -> f32 {
    if (abs(px) <= hx && abs(pz) <= hz && h0 < 0.002) {
        // Under the sole: all of it, but a shoe is round at the toe and heel,
        // so the box's corners (outside a rounded footprint, corner radius
        // 0.9 of the half width) keep the value at its faces (about 0.45).
        let r = 0.9 * min(hx, hz);
        let e = vec2<f32>(abs(px) - hx + r, abs(pz) - hz + r);
        let sd = length(max(e, vec2<f32>(0.0))) + min(max(e.x, e.y), 0.0) - r;
        return mix(0.45, 1.0, 1.0 - smoothstep(-0.01, 0.0, sd));
    }
    var f = 0.0;
    if (h0 > 0.002) {
        f = f + ag_vf_corner(h0, hx - px, hz - pz) - ag_vf_corner(h0, -hx - px, hz - pz) - ag_vf_corner(h0, hx - px, -hz - pz) + ag_vf_corner(h0, -hx - px, -hz - pz);
    }
    if (px > hx) {
        f = f + ag_vf_side(px - hx, pz, hz, h0, h1);
    }
    if (px < -hx) {
        f = f + ag_vf_side(-hx - px, pz, hz, h0, h1);
    }
    if (pz > hz) {
        f = f + ag_vf_side(pz - hz, px, hx, h0, h1);
    }
    if (pz < -hz) {
        f = f + ag_vf_side(-hz - pz, px, hx, h0, h1);
    }
    return clamp(f, 0.0, 1.0);
}

// Whether the ray from p along d meets the box lo..hi (slabs).
fn ag_ray_box(p: vec3<f32>, d: vec3<f32>, lo: vec3<f32>, hi: vec3<f32>) -> bool {
    let dd = select(d, vec3<f32>(1e-6), abs(d) < vec3<f32>(1e-6));
    let inv = 1.0 / dd;
    let t0 = (lo - p) * inv;
    let t1 = (hi - p) * inv;
    let tn = min(t0, t1);
    let tf = max(t0, t1);
    let near = max(max(tn.x, tn.y), tn.z);
    let far = min(min(tf.x, tf.y), tf.z);
    return far >= max(near, 0.0);
}

fn ag_contact(in: AgOut, a: AgAgent) -> vec4<f32> {
    let inv = vec4<f32>(-a.rot.xyz, a.rot.w);
    let q = ag_rotate(inv, in.world - a.pos);
    let reach = max(a.scale, 1e-3);
    let window = 1.0 - smoothstep(0.75, 1.0, max(abs(q.x), abs(q.z)) / reach);
    let hx = a.anim;
    let hz = a.steer;
    let h1 = a.speed;
    let h0 = a.clip_phase;
    let f = ag_box_view_factor(q.x, q.z, hx, hz, h0, h1);
    let up = vec3<f32>(0.0, 1.0, 0.0);
    let lum = vec3<f32>(0.2126, 0.7152, 0.0722);
    let l = look.key_dir.xyz;
    var e_key = 0.0;
    var blocked = 0.0;
    if (l.y > 0.0) {
        e_key = dot(look.key.xyz, lum) * l.y * lk_shadow(in.world, up) * sky_cloud_transmittance(in.world) * sky_contact_shadow(in.world, up);
        if (((a.color >> 8u) & 1u) != 0u && ag_ray_box(vec3<f32>(q.x, 0.0, q.z), ag_rotate(inv, l), vec3<f32>(-hx, h0, -hz), vec3<f32>(hx, h1, hz))) {
            // Only the first few centimetres past the footprint: beyond them
            // the cascades and the screen-space ray draw the shadow (taking
            // it again there would cut a box's shape into theirs).
            let o = max(abs(q.xz) - vec2<f32>(hx, hz), vec2<f32>(0.0));
            blocked = 1.0 - smoothstep(0.01, 0.05, length(o));
        }
    }
    let e_rim = dot(look.rim.xyz, lum) * max(look.rim_dir.y, 0.0);
    let env_i = look.ambient.w * look.env_class.w;
    let e_ind = dot(look.hemi_sky.xyz + look.ambient.xyz + lk_sh(up) * env_i, lum) * ao_multi_bounce(ao_fetch(in.world).a);
    let total = e_key + e_rim + e_ind;
    let share = (f * (e_rim + e_ind) + blocked * e_key) / max(total, 1e-6);
    let weight = f32(a.color & 255u) / 255.0;
    return vec4<f32>(0.0, 0.0, 0.0, clamp(share * weight * window, 0.0, 0.98));
}

// A fragment's surface, its material (the lamps' glow set), the view and the
// shading normal (`ag_shade`, `ag_shade_bore`).
struct AgSurf {
    s: Surface,
    m: Material,
    v: vec3<f32>,
    n: vec3<f32>,
};

// `bore`: whether the pipeline draws the agents in a tunnel's bore (eo_agents
// AGENT_IN_BORE, `--tunnels`) or every other: the fragments of the others are
// discarded (each agent is drawn by one of the two).
fn ag_surface(in: AgOut, bore: bool) -> AgSurf {
    var m = materials[in.material];
    let a = ag_agents[in.agent];
    let dpx = dpdx(in.world);
    let dpy = dpdy(in.world);
    // Derivatives first, in uniform control flow.
    let duvx = dpdx(in.uv);
    let duvy = dpdy(in.uv);
    // (SF_TUNNELS false: no bore, no agent in one, and the test compiled out; look.rs `ShaderFeatures`)
    if (SF_TUNNELS && ((a.flags & AG_IN_BORE) != 0u) != bore) {
        discard;
    }
    let v = normalize(frame.cam_pos.xyz - in.world);
    var n = normalize(in.normal);
    if (dot(in.normal, in.normal) < 1e-8) {
        n = normalize(cross(dpx, dpy));
    }
    if (dot(n, v) < 0.0) {
        n = -n;
    }
    var s: Surface;
    s.albedo = m.base.rgb;
    s.rough = m.params.x;
    s.metal = m.params.y;
    s.ao = 1.0;
    s.n = n;
    s.alpha = m.base.a;
    if ((m.flags & AG_M_PAINT) != 0u) {
        s.albedo = ag_unpack_srgb(a.color);
    }
    if ((m.flags & AG_M_ATLAS) != 0u) {
        let layer = m.layer;
        let alb = textureSampleGrad(kit_albedo, kit_sampler, in.uv, layer, duvx, duvy);
        let orm = textureSampleGrad(kit_orm, kit_sampler, in.uv, layer, duvx, duvy);
        // z rebuilt from xy (the BC5 normal maps carry only xy; for the
        // RGBA8 maps it is the stored z of a unit normal).
        let nxy = textureSampleGrad(kit_normal, kit_sampler, in.uv, layer, duvx, duvy).xy * 2.0 - 1.0;
        let nt = vec3<f32>(nxy, sqrt(max(1.0 - dot(nxy, nxy), 0.0)));
        if (alb.a < 0.5) {
            discard;
        }
        s.albedo = alb.rgb;
        s.ao = orm.r;
        s.rough = orm.g;
        s.metal = orm.b;
        s.alpha = 1.0;
        // Tangent frame from screen derivatives (the GLBs carry no tangents).
        let dp1 = dpx;
        let dp2 = dpy;
        let r1 = cross(dp2, n);
        let r2 = cross(n, dp1);
        let t = r1 * duvx.x + r2 * duvy.x;
        let b = r1 * duvx.y + r2 * duvy.y;
        let inv = inverseSqrt(max(max(dot(t, t), dot(b, b)), 1e-20));
        // glTF's v runs down the image: the derived bitangent is flipped (three's
        // GLTFLoader negates normalScale.y for meshes without tangents).
        let nm = normalize(t * inv * nt.x * m.params.z - b * inv * nt.y * m.params.z + n * max(nt.z, 0.05));
        if (all(nm == nm)) {
            s.n = nm;
        }
    }
    let lamp = m.flags & (AG_M_TAIL | AG_M_BRAKE | AG_M_INDICATOR | AG_M_HEAD | AG_M_DRL | AG_M_REVERSE);
    if (lamp == 0u && wx_level() != 0u) {
        ag_weather(&s, in.local, a, wx_fw(dpx, dpy)); // weathering (`--grime`): a vehicle's body, wheels and trim
    }
    if (lamp != 0u) {
        m.emissive = vec4<f32>(m.emissive.rgb * ag_lamp(a.flags, m.flags, in.local.x), 0.0);
    }
    s.rough = clamp(s.rough, 0.03, 1.0);
    return AgSurf(s, m, v, n);
}

// (`--look cinematic`, stage L3; eo-sky `cine_wet`, kept equal by eo-render
// tests/cine_wet.rs) car paint's clearcoat: a clear lacquer over the base
// coat (automotive clearcoat GGX roughness ~0.05-0.1), the grime of
// `--grime` (how much it roughened the paint) roughening the coat toward a
// dull film.
const AG_CINE_COAT_ROUGH: f32 = 0.07;
const AG_CINE_COAT_GRIME_ROUGH: f32 = 0.35;

// Every agent but those in a tunnel's bore: the city's light rig (look_light.wgsl).
fn ag_shade(in: AgOut) -> vec4<f32> {
    var g = ag_surface(in, false);
    // (SF_PLAYER_CONTACT false: no patch is drawn, and the test compiled out; look.rs `ShaderFeatures`)
    if (SF_PLAYER_CONTACT && (g.m.flags & AG_M_CONTACT) != 0u) {
        return ag_contact(in, ag_agents[in.agent]);
    }
    if (SF_LANDMARK_ACCESS) {
        // in a landmark deck's room, the room's sky (landmarks_deck.wgsl).
        g.s.ao = g.s.ao * deck_room_vis(in.world);
    }
    if (SF_CINEMATIC && (g.m.flags & AG_M_PAINT) != 0u && sky_on(skyu.flags.x, SKY_CINEMATIC)) {
        let grime = clamp((g.s.rough - materials[in.material].params.x) / 0.4, 0.0, 1.0);
        sky_coat = 1.0;
        sky_coat_rough = mix(AG_CINE_COAT_ROUGH, AG_CINE_COAT_GRIME_ROUGH, grime);
    }
    return look_light(g.s, g.m, g.v, in.world, g.n, 0u);
}

// A car (or the player) in a tunnel's bore (`--tunnels`; the feed's
// AGENT_IN_BORE), in pipelines of its own (agents.rs `bore`: agent-opaque
// stands at the edge of what Apple's compiler takes, and a second light loop
// beside look_light's did not compile): lit as the bore's own surfaces are
// (structures.wgsl `sx_bore_light_at`: the clustered lights, the lined bore's
// interreflection, the threshold zone and the portal's daylight by its depth
// in; no sun, sky, rim or environment), its lamps' own glow added.
fn ag_shade_bore(in: AgOut) -> vec4<f32> {
    let g = ag_surface(in, true);
    // No ground-contact patch in a bore: `ag_contact` weighs the city rig's
    // sun, rim and sky, none of which reach in here (the patch would draw
    // black). The body itself is lit by the bore as any agent in one.
    if (SF_PLAYER_CONTACT && (g.m.flags & AG_M_CONTACT) != 0u) {
        discard;
    }
    let d = f32(ag_agents[in.agent].flags >> AG_BORE_DEPTH_SHIFT) / AG_BORE_DEPTH_QUANT;
    let c = sx_bore_light_at(g.s, g.m, g.v, in.world, 0u, d);
    return vec4<f32>(c.rgb + g.m.emissive.rgb, c.a);
}

@fragment
fn agent_fs(in: AgOut) -> @location(0) vec4<f32> {
    return ag_shade(in);
}

struct AgIdsOut {
    @location(0) color: vec4<f32>,
    @location(1) ids: vec2<u32>,
};

@fragment
fn agent_fs_ids(in: AgOut) -> AgIdsOut {
    var out: AgIdsOut;
    out.color = ag_shade(in);
    out.ids = vec2<u32>(0xfffffffeu, in.material);
    return out;
}

@fragment
fn agent_bore_fs(in: AgOut) -> @location(0) vec4<f32> {
    return ag_shade_bore(in);
}

@fragment
fn agent_bore_fs_ids(in: AgOut) -> AgIdsOut {
    var out: AgIdsOut;
    out.color = ag_shade_bore(in);
    out.ids = vec2<u32>(0xfffffffeu, in.material);
    return out;
}
