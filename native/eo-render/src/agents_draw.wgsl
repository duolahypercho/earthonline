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
const AG_BORE_DEPTH_SHIFT: u32 = 19u;
const AG_BORE_DEPTH_QUANT: f32 = 8.0;

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

// ---- the people's materials (`--people-shading`; agents.rs `SF_PEOPLE_SHADING`) ----
// The Rocketbox atlases carry colour, a normal map and the roughness the cook
// sets per material class (eo-agents rocketbox.rs `ROUGH_*`: skin 0.5 on the
// head and 0.55 on the body, hair 0.45, cloth 0.85, gear 0.6, glasses 0.15;
// agents.rs tests that each lands in its band). The ORM atlas is BC1, 6 bits
// of green, so a texel's class is read back from the band its roughness falls
// in; a texel between classes (a block or mip that mixes them) reads as none
// and keeps the city's shading.
const PP_NONE: u32 = 0u;
const PP_SKIN: u32 = 1u;
const PP_CLOTH: u32 = 2u;
const PP_HAIR: u32 = 3u;
const PP_HAIR_BAND: vec2<f32> = vec2<f32>(0.42, 0.47);
const PP_SKIN_BAND: vec2<f32> = vec2<f32>(0.47, 0.58);
const PP_CLOTH_BAND: vec2<f32> = vec2<f32>(0.75, 0.95);

fn pp_class(rough: f32) -> u32 {
    if (rough >= PP_SKIN_BAND.x && rough < PP_SKIN_BAND.y) {
        return PP_SKIN;
    }
    if (rough >= PP_CLOTH_BAND.x && rough < PP_CLOTH_BAND.y) {
        return PP_CLOTH;
    }
    if (rough >= PP_HAIR_BAND.x && rough < PP_HAIR_BAND.y) {
        return PP_HAIR;
    }
    return PP_NONE;
}

// Skin: light enters and leaves the skin apart, so the lit side's light
// spreads across the shadow terminator, red furthest. A diffusion profile of
// mean free path ld on a feature of radius R blurs the key's Lambert term
// over an angle sigma = ld / R: near the terminator the clamped cosine is the
// ramp max(d, 0) in d = asin(n.l), which a Gaussian of sigma turns into
// d Phi(d / sigma) + sigma phi(d / sigma), so the term adds that minus the
// ramp (a soft, red-weighted band either side of the terminator, nothing far
// from it, the energy it adds of order sigma^2). Mean free paths
// ld = 1 / sqrt(3 sa (sa + ss')) from Jensen et al. 2001 (skin1: sa 0.032 /
// 0.17 / 0.48 per mm, ss' 0.74 / 0.88 / 1.01 per mm): 3.67 / 1.37 / 0.68 mm;
// R 15 mm, between a nose's or a lip's and a cheek's. The skin's specular
// keeps the city's lobe at the cook's roughness and adds a second, sharper
// one for its oily film at the skin's F0 (n 1.4: 0.028), a two-lobe skin
// specular.
const PP_SKIN_MFP_MM: vec3<f32> = vec3<f32>(3.67, 1.37, 0.68);
const PP_SKIN_FEATURE_MM: f32 = 15.0;
const PP_SKIN_F0: f32 = 0.028;
const PP_SKIN_SHARP_ROUGH: f32 = 0.3;
const PP_SKIN_SHARP_WEIGHT: f32 = 0.15;
// Cloth: the fibres' grazing sheen, the Charlie sheen (Estevez and Kulla
// 2017) with the Neubelt-Pettineo visibility, from the key, its colour the
// fabric's own at half strength (sqrt(albedo), the cloth model's default
// sheen colour, x 0.5: everyday cotton, denim and wool, not velvet) and a
// broad roughness.
const PP_CLOTH_SHEEN: f32 = 0.5;
const PP_CLOTH_SHEEN_ROUGH: f32 = 0.7;
// Hair: Kajiya-Kay strand highlights (1989) on two shifted lobes (Scheuermann
// 2004: a white primary shifted toward the tip, a broader secondary in the
// hair's colour toward the root, the light that passes through a strand), the
// strands taken to run down the head (gravity on the surface: the cards carry
// no strand direction).
const PP_HAIR_SHIFT: vec2<f32> = vec2<f32>(0.08, -0.08);
const PP_HAIR_EXP: vec2<f32> = vec2<f32>(80.0, 20.0);
const PP_HAIR_SPEC: vec2<f32> = vec2<f32>(0.10, 0.25);

// The Gaussian-blurred terminator's addition to the key's Lambert term, per
// channel (sigma in radians): d Phi(d / sigma) + sigma phi(d / sigma) - max(d, 0),
// Phi by its tanh form (within 1e-3).
fn pp_terminator(nl: f32, sigma: vec3<f32>) -> vec3<f32> {
    let d = asin(clamp(nl, -1.0, 1.0));
    let z = vec3<f32>(d) / sigma;
    let cdf = 0.5 * (vec3<f32>(1.0) + tanh(0.7978846 * (z + 0.044715 * z * z * z)));
    let pdf = exp(-0.5 * z * z) * 0.3989423;
    return max(vec3<f32>(d) * cdf + sigma * pdf - vec3<f32>(max(d, 0.0)), vec3<f32>(0.0));
}

fn pp_charlie(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, alpha: f32) -> f32 {
    let h = normalize(l + v);
    let nh = clamp(dot(n, h), 0.0, 1.0);
    let nl = max(dot(n, l), 0.0);
    let nv = max(dot(n, v), 1e-4);
    let sin2 = max(1.0 - nh * nh, 0.0);
    let inv = 1.0 / alpha;
    let d = (2.0 + inv) * pow(sin2, 0.5 * inv) / (2.0 * LK_PI);
    let vis = 1.0 / max(4.0 * (nl + nv - nl * nv), 1e-4);
    return d * vis;
}

fn pp_strand(t: vec3<f32>, n: vec3<f32>, h: vec3<f32>, shift: f32, e: f32) -> f32 {
    let ts = normalize(t + n * shift);
    let th = dot(ts, h);
    let s = sqrt(max(1.0 - th * th, 0.0));
    return smoothstep(-1.0, 0.0, th) * pow(s, e);
}

// The light the city's rig leaves out for a person's skin, cloth and hair,
// from the key (the sun or the moon), added after `look_light` under the fog
// it laid (look_light.wgsl `lk_fog_out`), at the key's visibility it found
// (`lk_key_vis`: shadow, clouds, contact; 0 where the key does not reach; for
// skin looked up past the terminator as far as its band reaches,
// `lk_vis_wrap`, set before `look_light`: one shadow lookup, the city's own;
// a second does not fit the agents' pipeline on Apple's compiler). The
// band reaches past the terminator about three red sigmas (0.73 rad: n.l
// down to -0.67).
const PP_SKIN_WRAP_REACH: f32 = 0.67;

fn pp_person(g: AgSurf) -> vec3<f32> {
    let cls = pp_class(g.s.rough);
    let n = g.s.n;
    let l = look.key_dir.xyz;
    let nl = dot(n, l);
    if (cls == PP_NONE || lk_key_vis <= 0.0 || (nl <= 0.0 && cls != PP_SKIN)) {
        return vec3<f32>(0.0);
    }
    let v = g.v;
    let key = look.key.xyz * lk_key_vis;
    var out = vec3<f32>(0.0);
    if (cls == PP_SKIN) {
        out = key * pp_terminator(nl, PP_SKIN_MFP_MM / PP_SKIN_FEATURE_MM) * g.s.albedo / LK_PI;
        out = out + key * max(nl, 0.0) * lk_ggx(n, v, l, vec3<f32>(PP_SKIN_F0), PP_SKIN_SHARP_ROUGH) * PP_SKIN_SHARP_WEIGHT;
    } else if (cls == PP_CLOTH) {
        out = key * nl * pp_charlie(n, v, l, PP_CLOTH_SHEEN_ROUGH) * PP_CLOTH_SHEEN * sqrt(max(g.s.albedo, vec3<f32>(0.0)));
    } else {
        let down = vec3<f32>(0.0, -1.0, 0.0);
        let t0 = down - n * dot(n, down);
        if (dot(t0, t0) > 1e-6) {
            let t = normalize(t0);
            let h = normalize(l + v);
            let s1 = pp_strand(t, n, h, PP_HAIR_SHIFT.x, PP_HAIR_EXP.x) * PP_HAIR_SPEC.x;
            let s2 = pp_strand(t, n, h, PP_HAIR_SHIFT.y, PP_HAIR_EXP.y) * PP_HAIR_SPEC.y;
            out = key * smoothstep(0.0, 0.25, nl) * (vec3<f32>(s1) + s2 * g.s.albedo);
        }
    }
    return out * (1.0 - lk_fog_out);
}

// ---- the ground's light on a person (`--people-bounce`; agents.rs `SF_PEOPLE_BOUNCE`) ----
// The rig lights the lower half of every surface's world with its ground
// lobe (look.hemi_ground, and with the ground bounce the sunlit open ground's
// exitance, ambient.wgsl `amb_bounce`): one colour for every street at every
// hour, nothing from the street lamps. A person stands a metre or two over the
// very ground the camera sees round their feet, so that ground's radiance L
// is measured there instead: the previous frame (sky_prev, the scene's
// radiance before exposure) read at a ring of points on the ground round the
// feet (`PB_RING_M`, `PB_SAMPLES` of them), each kept only where the previous
// frame's depth (lk_ao_z) shows that ground and not a body or car in front of
// it, and the rig's ground term is replaced by a uniform lower half-space of
// that radiance: irradiance pi L (1 - n.y) / 2 (the view factor of an infinite
// plane below), weighted by the share of the ring found. A lamp's pool lights
// a person from below at night, a sunlit street warms their underside by
// day, a shaded alley's dark paving gives them little.
const PB_RING_M: f32 = 0.9;
const PB_SAMPLES: u32 = 6u;
// how far the depth found may stand from the ring point's (m, and per m of distance)
const PB_DEPTH_TOL: vec2<f32> = vec2<f32>(0.2, 0.02);

// (radiance, share of the ring found) round the ground point `foot`.
fn pb_ground(foot: vec3<f32>) -> vec4<f32> {
    if (look.ao_params.x < 0.5 || frame.copy_step.z > 0.5) {
        return vec4<f32>(0.0);
    }
    let dims = vec2<i32>(textureDimensions(lk_ao_z));
    var sum = vec3<f32>(0.0);
    var found = 0.0;
    for (var k = 0u; k < PB_SAMPLES; k = k + 1u) {
        let a = (f32(k) + 0.5) * (2.0 * LK_PI / f32(PB_SAMPLES));
        let p = foot + vec3<f32>(cos(a) * PB_RING_M, 0.02, sin(a) * PB_RING_M);
        let clip = look.ao_vp * vec4<f32>(p, 1.0);
        if (clip.w <= 1e-3) {
            continue;
        }
        let uv = vec2<f32>(clip.x / clip.w * 0.5 + 0.5, 0.5 - clip.y / clip.w * 0.5);
        if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) {
            continue;
        }
        let z = textureLoad(lk_ao_z, clamp(vec2<i32>(uv * vec2<f32>(dims)), vec2<i32>(0), dims - 1), 0).r;
        let zr = dot(p - look.ao_cam.xyz, look.ao_fwd.xyz);
        if (z <= 0.0 || abs(zr - z) > PB_DEPTH_TOL.x + PB_DEPTH_TOL.y * zr) {
            continue;
        }
        sum = sum + textureSampleLevel(sky_prev, lk_ao_samp, uv, 0.0).rgb;
        found = found + 1.0;
    }
    if (found < 1.0) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(sum / found, found / f32(PB_SAMPLES));
}

// What the measured ground adds to a person's fragment (outgoing radiance,
// before the fog; may be negative where the ground is darker than the rig's).
fn pb_person(g: AgSurf, world: vec3<f32>, foot: vec3<f32>) -> vec3<f32> {
    let lg = pb_ground(foot);
    if (lg.w <= 0.0) {
        return vec3<f32>(0.0);
    }
    let n = g.s.n;
    let lower = 0.5 - 0.5 * n.y;
    var rig = look.hemi_ground.xyz * lower;
    if (AMB_SHADER_LEVELS > 0u && amb_on() && any(look.lx[0].yzw > vec3<f32>(0.0))) {
        rig = rig + amb_bounce(amb_ground(world, g.n), n, sky_cloud_transmittance(world));
    }
    let measured = LK_PI * lg.rgb * lower;
    let diffuse = g.s.albedo * (1.0 - g.s.metal);
    let f0 = mix(vec3<f32>(0.04), g.s.albedo, g.s.metal);
    let dfg = lk_dfg(max(dot(n, g.v), 1e-4), g.s.rough);
    let fss = f0 * dfg.x + vec3<f32>(dfg.y);
    var ao = g.s.ao;
    if (frame.copy_step.z < 0.5) {
        ao = ao * ao_multi_bounce(ao_fetch(world).a);
    }
    return (measured - rig) * lg.w * diffuse / LK_PI * (vec3<f32>(1.0) - fss) * ao;
}

// Every agent but those in a tunnel's bore: the city's light rig (look_light.wgsl).
fn ag_shade(in: AgOut) -> vec4<f32> {
    let g = ag_surface(in, false);
    if (SF_CINEMATIC && (g.m.flags & AG_M_PAINT) != 0u && sky_on(skyu.flags.x, SKY_CINEMATIC)) {
        let grime = clamp((g.s.rough - materials[in.material].params.x) / 0.4, 0.0, 1.0);
        sky_coat = 1.0;
        sky_coat_rough = mix(AG_CINE_COAT_ROUGH, AG_CINE_COAT_GRIME_ROUGH, grime);
    }
    // (`--people-shading`) a person's skin, cloth and hair; compiled out without it.
    let is_person = (g.m.flags & AG_M_ATLAS) != 0u && ag_models[ag_agents[in.agent].model].model_class == AG_PERSON;
    let person = SF_PEOPLE_SHADING && is_person;
    if (person && pp_class(g.s.rough) == PP_SKIN) {
        lk_vis_wrap = PP_SKIN_WRAP_REACH; // the key's visibility past the terminator, for the band's tail
    }
    var c = look_light(g.s, g.m, g.v, in.world, g.n, 0u);
    if (person) {
        c = vec4<f32>(c.rgb + pp_person(g), c.a);
    }
    // (`--people-bounce`) the ground round the feet in place of the rig's ground lobe; compiled out without it.
    if (SF_PEOPLE_BOUNCE && is_person) {
        c = vec4<f32>(max(c.rgb + pb_person(g, in.world, ag_agents[in.agent].pos) * (1.0 - lk_fog_out), vec3<f32>(0.0)), c.a);
    }
    return c;
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
