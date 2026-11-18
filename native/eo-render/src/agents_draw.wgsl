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

fn ag_shade(in: AgOut) -> vec4<f32> {
    var m = materials[in.material];
    let a = ag_agents[in.agent];
    let dpx = dpdx(in.world);
    let dpy = dpdy(in.world);
    // Derivatives first, in uniform control flow.
    let duvx = dpdx(in.uv);
    let duvy = dpdy(in.uv);
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
    if (lamp != 0u) {
        m.emissive = vec4<f32>(m.emissive.rgb * ag_lamp(a.flags, m.flags, in.local.x), 0.0);
    }
    s.rough = clamp(s.rough, 0.03, 1.0);
    return look_light(s, m, v, in.world, n, 0u);
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
