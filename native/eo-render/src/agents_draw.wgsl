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

// (`--car-finish`, agents.rs `AgentPipes::make_for`) Which agents a colour
// pipeline draws: 0 every one (one pipeline, as before), 1 the cars, 2 the
// people; each compiled without the other's code.
override AG_PIPE: u32 = 0u;

fn ag_pipe_cars() -> bool {
    return AG_PIPE != 2u;
}

fn ag_pipe_people() -> bool {
    return AG_PIPE != 1u;
}

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
    // (`--car-materials`) a car's normal with its refinement (`ag_car_normal`; a person's spare byte is 0).
    var nrm = normal.xyz;
    if (SF_CAR_MATERIALS && normal.w != 0.0 && ag.misc.y != 0u) {
        nrm = ag_car_normal(normal);
    }
    let pose = ag_pose(ag_visible[inst], pos, nrm, joints, weights, tag);
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

// (`--car-lamps`; SF_CAR_LAMPS compiles it out) The fragment's car is parked
// (`ag_car_parked`): every lamp of it is off (set by `ag_surface`).
var<private> ag_lamps_off: bool;

// Whether the cars' lamps are worked as a driver works them (`--car-lamps`,
// agents.rs `AgParams::misc.z` bit 0).
fn ag_car_lamps_on() -> bool {
    return SF_CAR_LAMPS && SF_CAR_MATERIALS && (ag.misc.z & 1u) != 0u;
}

// Where the lamps stand between their day and night values. (`--car-lamps`)
// A lamp is lit or not: its luminance is the same at dusk as at midnight (the
// camera's exposure makes the difference), so the lamps are at their night
// values whenever the vehicle lamps are on (eo-sky `night::vehicle_lamps`:
// from dusk, and in rain or fog; the sky's `night.w`), at the day's otherwise;
// without it the sun's altitude fades them (night below -6 deg, day above 6).
fn ag_lamp_night() -> f32 {
    if (ag_car_lamps_on()) {
        return select(0.0, 1.0, skyu.night.w > 0.0);
    }
    return ag.lamp_misc.x;
}

// The lamp channels (`heroLampState` with `HERO_LAMP`): intensity for the
// agent's state between the day and night values (`--car-lamps`: none on a
// parked car).
fn ag_lamp(flags: u32, mflags: u32, local_x: f32) -> f32 {
    if (SF_CAR_LAMPS && ag_lamps_off) {
        return 0.0;
    }
    let night = ag_lamp_night();
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
        // (`--car-lamps`) a moving car's running lamps by day at their own
        // luminance (agents.rs `DRL_CD`: a bright strip in sunlight), as before at night
        var drl = max(head, ag.lamp_misc.y);
        if (SF_CAR_LAMPS && ag_car_lamps_on()) {
            drl = mix(AG_CAR_DRL_DAY, drl, night);
        }
        i = max(i, drl);
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
const AG_M_CONTACT: u32 = 16777216u;
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
    if (ag_pipe_people() && (m.flags & AG_M_ATLAS) != 0u) {
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
        let nm0 = normalize(t * inv * nt.x * m.params.z - b * inv * nt.y * m.params.z + n * max(nt.z, 0.05));
        if (all(nm0 == nm0)) {
            // Keep normal-mapped skin/hair/cloth in the eye's hemisphere so silhouette
            // pixels never drop to nv -> 0 (which caused blinding white grazing Fresnel rims).
            let g_soft = smoothstep(0.04, 0.28, dot(n, v));
            var nm = normalize(mix(n, nm0, mix(0.25, 1.0, g_soft)));
            let snv = dot(nm, v);
            if (snv < 0.14) {
                nm = normalize(nm + v * (0.14 - snv));
            }
            s.n = nm;
        }
    }
    let lamp = m.flags & (AG_M_TAIL | AG_M_BRAKE | AG_M_INDICATOR | AG_M_HEAD | AG_M_DRL | AG_M_REVERSE);
    if (SF_CAR_LAMPS) {
        ag_lamps_off = ag_car_lamps_on() && ag_car_parked(a); // (`--car-lamps`) a parked car's lamps are dark
    }
    if (lamp == 0u && wx_level() != 0u && !ag_car_on() && ag_pipe_cars()) {
        ag_weather(&s, in.local, a, wx_fw(dpx, dpy)); // weathering (`--grime`): a vehicle's body, wheels and trim (`--car-materials`: ag_car_grime)
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
const PP_SKIN_SHARP_ROUGH: f32 = 0.34;
const PP_SKIN_SHARP_WEIGHT: f32 = 0.10;
// Cloth: the fibres' grazing sheen, the Charlie sheen (Estevez and Kulla
// 2017) with the Neubelt-Pettineo visibility, from the key, its colour the
// fabric's own at half strength (sqrt(albedo), the cloth model's default
// sheen colour, x 0.5: everyday cotton, denim and wool, not velvet) and a
// broad roughness.
const PP_CLOTH_SHEEN: f32 = 0.42;
const PP_CLOTH_SHEEN_ROUGH: f32 = 0.72;
// Hair: Kajiya-Kay strand highlights (1989) on two shifted lobes (Scheuermann
// 2004: a white primary shifted toward the tip, a broader secondary in the
// hair's colour toward the root, the light that passes through a strand), the
// strands taken to run down the head (gravity on the surface: the cards carry
// no strand direction).
const PP_HAIR_SHIFT: vec2<f32> = vec2<f32>(0.08, -0.08);
const PP_HAIR_EXP: vec2<f32> = vec2<f32>(80.0, 20.0);
const PP_HAIR_SPEC: vec2<f32> = vec2<f32>(0.032, 0.09);

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
    let nv = max(dot(n, v), 0.12);
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
    let key = look.key.xyz * lk_key_vis;
    return pp_lobes(g, cls, l, nl, key) * (1.0 - lk_fog_out);
}

// The skin's, cloth's and hair's own terms for one light: `key` its irradiance
// across a surface facing it (at its visibility), `l` toward it, nl = n.l.
fn pp_lobes(g: AgSurf, cls: u32, l: vec3<f32>, nl: f32, key: vec3<f32>) -> vec3<f32> {
    let n = g.s.n;
    let v = g.v;
    var out = vec3<f32>(0.0);
    if (cls == PP_SKIN) {
        out = key * pp_terminator(nl, PP_SKIN_MFP_MM / PP_SKIN_FEATURE_MM) * g.s.albedo / LK_PI;
        out = out + key * max(nl, 0.0) * lk_ggx(n, v, l, vec3<f32>(PP_SKIN_F0), PP_SKIN_SHARP_ROUGH) * PP_SKIN_SHARP_WEIGHT;
    } else if (cls == PP_CLOTH) {
        out = key * nl * pp_charlie(n, v, l, PP_CLOTH_SHEEN_ROUGH) * PP_CLOTH_SHEEN * sqrt(max(g.s.albedo, vec3<f32>(0.0)));
    } else {
        let down = vec3<f32>(0.0, -1.0, 0.0);
        let t0 = down - n * dot(n, down);
        let t_len = length(t0);
        if (t_len > 1e-3) {
            let t = t0 / t_len;
            let h = normalize(l + v);
            // Fade the vertical-strand highlight smoothly near the crown pole (where gravity tangent degenerates).
            let pole_fade = smoothstep(0.08, 0.42, t_len);
            let s1 = pp_strand(t, n, h, PP_HAIR_SHIFT.x, PP_HAIR_EXP.x) * PP_HAIR_SPEC.x;
            let s2 = pp_strand(t, n, h, PP_HAIR_SHIFT.y, PP_HAIR_EXP.y) * PP_HAIR_SPEC.y;
            out = key * smoothstep(0.0, 0.25, nl) * pole_fade * (vec3<f32>(s1) + s2 * g.s.albedo);
        }
    }
    return out;
}

// ---- the lamps on a person (`--people-lamps`; agents.rs `SF_PEOPLE_LAMPS`) ----
// The rig lights a person from each street lamp, headlamp and lit shop window
// with the city's two lobes (Lambert and the GGX at the texel's roughness,
// sky_light.wgsl `sky_local_lights_ao`); what `pp_person` adds for the key,
// the skin's blurred terminator and oily lobe, the cloth's sheen and the
// hair's strand highlights, it adds for each of those lights too: the
// frame's lights of the fragment's cluster, each at its irradiance there
// (shaped and reached as sky_light.wgsl shapes them, `pl_shape`) and, where
// the light keeps a shadow map (`--light-shadows`), at its visibility
// (looked up at any n.l, so the skin's band reaches past the terminator as
// the key's does). At night this is the light that outlines a person
// against the dark: the sheen of a jacket's shoulders and sleeves and the
// glint of hair toward a lamp behind them, the warm band on a cheek turned
// from a shop window. Under the fog look_light found.

// A frame light's irradiance factor at a point `dist` away along unit `l`
// toward it (x its colour and intensity, before n.l): its luminaire's shape
// toward the point and its reach, as `sky_local_lights_ao` computes them.
fn pl_shape(li: SkyLight, l: vec3<f32>, dist: f32, cine: bool) -> f32 {
    let d2 = max(dist * dist, 0.04);
    let w = clamp(1.0 - pow(dist / li.range, 4.0), 0.0, 1.0);
    let ca = dot(-l, li.dir);
    var shape = 1.0;
    if (li.kind == 0u) {
        shape = (1.0 + 1.5 * (1.0 - ca * ca)) * smoothstep(li.cos_outer, li.cos_inner, ca);
        if (cine) {
            shape = sky_cine_cobra(l, li.dir);
        }
    } else if (li.kind == 1u) {
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
        shape = vert * side * near * near;
    } else if (li.kind >= 3u) {
        shape = smoothstep(li.cos_outer, li.cos_inner, ca);
    }
    return li.intensity * shape * w * w / d2;
}

// The frame's light cluster at a world point: (first record, count), count 0 outside the clusters.
fn pl_cluster(world: vec3<f32>) -> vec2<u32> {
    let rel = world - skyu.eye.xyz;
    let vz = dot(rel, skyu.cam_fwd.xyz);
    if (vz < skyu.cluster.x || vz > skyu.cluster.y) {
        return vec2<u32>(0u, 0u);
    }
    let vx = dot(rel, skyu.cam_right.xyz);
    let vy = dot(rel, skyu.cam_up.xyz);
    let ndc = vec2<f32>(vx / (vz * skyu.cluster.z * skyu.cluster.w), vy / (vz * skyu.cluster.z));
    let gr = skyu.lights.yzw;
    let tx = u32(clamp((ndc.x * 0.5 + 0.5) * f32(gr.x), 0.0, f32(gr.x) - 1.0));
    let ty = u32(clamp((0.5 - ndc.y * 0.5) * f32(gr.y), 0.0, f32(gr.y) - 1.0));
    let tz = u32(clamp(log(vz / skyu.cluster.x) / log(skyu.cluster.y / skyu.cluster.x) * f32(gr.z), 0.0, f32(gr.z) - 1.0));
    let c = (tz * gr.y + ty) * gr.x + tx;
    return vec2<u32>(gr.x * gr.y * gr.z + c * SKY_CLUSTER_CAP, min(sky_clusters[c], SKY_CLUSTER_CAP));
}

fn pl_person(g: AgSurf, world: vec3<f32>) -> vec3<f32> {
    let cls = pp_class(g.s.rough);
    if (cls == PP_NONE || !sky_on(skyu.flags.x, SKY_LIGHTS) || skyu.lights.x == 0u || frame.copy_step.z > 0.5) {
        return vec3<f32>(0.0);
    }
    let cl = pl_cluster(world);
    let cine = SF_CINEMATIC && sky_on(skyu.flags.x, SKY_CINEMATIC);
    let n = g.s.n;
    let reach = select(0.0, -PP_SKIN_WRAP_REACH, cls == PP_SKIN);
    var out = vec3<f32>(0.0);
    for (var k = 0u; k < cl.y; k = k + 1u) {
        let li = sky_lights[sky_clusters[cl.x + k]];
        let d = li.pos - world;
        let dist = max(length(d), 1e-3);
        let l = d / dist;
        let nl = dot(n, l);
        if (nl <= reach || (nl <= 0.0 && cls != PP_SKIN)) {
            continue;
        }
        var e = pl_shape(li, l, dist, cine);
        if (e <= 0.0) {
            continue;
        }
        if (LS_SHADER && li.pad >= 1.0) {
            e = e * ls_light_visibility(u32(li.pad) - 1u, world, g.n);
        }
        out = out + pp_lobes(g, cls, l, nl, li.color * e);
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
        // Soft-compress specular HDR spikes from wet-pavement lamp reflections so
        // only diffuse ground bounce lights a pedestrian's chin and undersides.
        let tap = max(textureSampleLevel(sky_prev, lk_ao_samp, uv, 0.0).rgb, vec3<f32>(0.0));
        let tl = dot(tap, vec3<f32>(0.2126, 0.7152, 0.0722));
        let soft = tl / (1.0 + tl / 1.4);
        sum = sum + tap * (soft / max(tl, 1e-4));
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
// (`--soft-shadows`, look_light.wgsl `lk_sun_pre`) the sun's visibility at the
// fragment's geometric normal (as ag_surface takes it), looked up before the shading.
fn ag_sun_pre(in: AgOut) {
    let dn = cross(dpdx(in.world), dpdy(in.world));
    var n = in.normal;
    if (dot(n, n) < 1e-8) {
        n = dn;
    }
    n = normalize(n);
    if (dot(n, frame.cam_pos.xyz - in.world) < 0.0) {
        n = -n;
    }
    lk_sun_pre = lk_shadow(in.world, n);
}

fn ag_shade(in: AgOut) -> vec4<f32> {
    if (LK_SUN_PRE) {
        ag_sun_pre(in); // (`--soft-shadows`) the penumbra first, where little else is live (look_light.wgsl `lk_sun_pre`)
    }
    if (AMB_OL_PRE) {
        amb_ol_pre(); // (`--occluder-light`) what hides the sky's light first too (ambient.wgsl)
    }
    let cd = ag_car_derivs(in); // (`--car-materials`) in uniform control flow; nothing without it
    var g = ag_surface(in, false);
    // (SF_PLAYER_CONTACT false: no patch is drawn, and the test compiled out; look.rs `ShaderFeatures`)
    if (SF_PLAYER_CONTACT && (g.m.flags & AG_M_CONTACT) != 0u) {
        return ag_contact(in, ag_agents[in.agent]);
    }
    if (SF_LANDMARK_ACCESS) {
        // in a landmark deck's room, the room's sky (landmarks_deck.wgsl).
        g.s.ao = g.s.ao * deck_room_vis(in.world);
    }
    if (SF_CINEMATIC && ag_pipe_cars() && (g.m.flags & AG_M_PAINT) != 0u && sky_on(skyu.flags.x, SKY_CINEMATIC)) {
        let grime = clamp((g.s.rough - materials[in.material].params.x) / 0.4, 0.0, 1.0);
        sky_coat = 1.0;
        sky_coat_rough = mix(AG_CINE_COAT_ROUGH, AG_CINE_COAT_GRIME_ROUGH, grime);
    }
    // (`--car-materials`) the cars' own materials and lamps: the surface and
    // its coat set before the one look_light, the glass's reflection after.
    var car = AgCar(0u, false, vec3<f32>(0.0), 0u, 0.0, vec4<f32>(0.0));
    let car_on = ag_car_on();
    if (car_on) {
        car = ag_car_prepare(&g, in, cd);
    }
    // (`--people-shading`) a person's skin, cloth and hair; compiled out without it.
    let is_person = ag_pipe_people() && (g.m.flags & AG_M_ATLAS) != 0u && ag_models[ag_agents[in.agent].model].model_class == AG_PERSON; // (`AG_PIPE`)
    let person = SF_PEOPLE_SHADING && is_person;
    if (person && pp_class(g.s.rough) == PP_SKIN) {
        lk_vis_wrap = PP_SKIN_WRAP_REACH; // the key's visibility past the terminator, for the band's tail
    }
    var c = look_light(g.s, g.m, g.v, in.world, g.n, 0u);
    if (car_on) {
        c = ag_car_finish(c, car, in.world, g, in.local, cd.px); // (a person comes back as it went: `ag_car_prepare` leaves them out)
    }
    if (person) {
        c = vec4<f32>(c.rgb + pp_person(g), c.a);
    }
    // (`--people-lamps`) the lamps' skin, cloth and hair terms; compiled out without it.
    if (SF_PEOPLE_LAMPS && is_person) {
        c = vec4<f32>(c.rgb + pl_person(g, in.world), c.a);
    }
    // (`--people-bounce`) the ground round the feet in place of the rig's ground lobe; compiled out without it.
    if (SF_PEOPLE_BOUNCE && is_person) {
        c = vec4<f32>(max(c.rgb + pb_person(g, in.world, ag_agents[in.agent].pos) * (1.0 - lk_fog_out), vec3<f32>(0.0)), c.a);
    }
    return c;
}

// ---- (`--car-materials`; SF_CAR_MATERIALS, look.rs `ShaderFeatures`: compiled
// out without it, so the agents' pipelines are as before) The cars' own
// materials: a two-layer paint (the base, metallic where the body's paint is,
// under a clear lacquer: F0 0.04, GGX ~0.04 when new, rougher with age and
// grime) reflecting the environment at its physical strength (the painted
// metal class, ~1 x the environment, where a facade's paint takes 0.45);
// thin tinted glass (a dielectric's Fresnel reflection of the probes, the
// screen-space reflection and the street's lamps over the cabin seen through
// it, instead of a lit beige panel); a cabin in its own shade; lamps in
// saturated LED colours (red, amber) behind a clear or tinted cover, with
// the LED or reflector structure where a pixel resolves it, a lamp's
// reflector bowl lit with it and the rear plate under its lamp; a light bar
// lit only on a moving unit that is answering a call (never on a parked one);
// road grime in the spray band up the lower body (highest behind each wheel,
// in run-off streaks), round the arches and as a dust film on the roof;
// California plates (white sheeting, the red script, seven embossed navy
// characters, the frame, bolts, the year sticker and dirt); weathered tyres
// (rubber, road dust, raised sidewall lettering) and brake-dusted wheels;
// sun-faded black trim on older cars; the wheel arches and the underside in
// the body's own shade; and every glossy part's roughness widened by its
// normal's change across the pixel (specular anti-aliasing: no sparkle on a
// curved panel or a rim's edge). Roles from eo-agents `vehicles::role_of`. ----

// eo-agents vehicles.rs `M_ROLE_SHIFT` and the roles (agents.rs tests them).
const AG_ROLE_SHIFT: u32 = 20u;
const AG_ROLE_GLASS: u32 = 1u;
const AG_ROLE_GLASS_TINT: u32 = 2u;
const AG_ROLE_TYRE: u32 = 3u;
const AG_ROLE_PLATE: u32 = 4u;
const AG_ROLE_CHROME: u32 = 5u;
const AG_ROLE_LENS: u32 = 6u;
const AG_ROLE_INTERIOR: u32 = 7u;
const AG_ROLE_WHEEL: u32 = 8u;
const AG_ROLE_TRIM: u32 = 9u;
const AG_ROLE_REFLECTOR: u32 = 10u;
const AG_ROLE_EMERGENCY: u32 = 11u;
const AG_ROLE_BRAKE: u32 = 12u;
const AG_ROLE_LIVERY: u32 = 13u;
const AG_ROLE_BADGE: u32 = 14u;

// Specular anti-aliasing (Tokuyoshi and Kaplanyan 2019, as Filament filters
// a normal): the GGX alpha^2 widened by 2 x variance x the normal's squared
// change across the pixel, at most the clamp.
const AG_CAR_AA_VARIANCE: f32 = 0.15;
const AG_CAR_AA_CLAMP: f32 = 0.2;
// The lacquer's GGX roughness: a new car's (automotive clearcoat 0.03-0.05)
// and an old one's (swirls, wash marring, oxidation: ~0.12); grime takes it
// toward a dull film as the cinematic coat's grime does.
const AG_CAR_COAT_NEW: f32 = 0.04;
const AG_CAR_COAT_OLD: f32 = 0.12;
// Road grime over the lacquer: the share of it a dense film covers, and the
// lacquer's roughness between the film's particles.
const AG_CAR_FILM_COVER: f32 = 0.85;
const AG_CAR_COAT_FILMED: f32 = 0.16;
// A metallic base coat is flake in a binder, not bare metal: its metalness.
// Which cars are metallic goes by the paint (the instance colour; the
// body's own paint material says nothing of it): silvers and greys always,
// whites never (solid or pearl), dark and coloured paints half of them.
const AG_CAR_FLAKE_METAL: f32 = 0.55;
// The base coat's share of the paint's colour (the lacquer passes ~96 % each
// way, a white pigment returns ~0.85: a palette white of 0.89 is ~0.7 under
// the lacquer) and an old coat's chalking (a warm grey cast).
const AG_CAR_BASE_COAT: f32 = 0.8;
const AG_CAR_CHALK: vec3<f32> = vec3<f32>(0.96, 0.94, 0.89);
// A red or amber lamp's luminance (the brightest lamp, the brake at night)
// held under this many times the exposure's white: past it the camera's
// crosstalk (eo-look `tone::SENSOR_CROSSTALK`) whitens the core, as a real
// sensor does; held under it the core keeps its red and the bloom its hue
// (8 x: still bright enough that a wet street mirrors it, the crosstalk
// tinting the core only a little).
const AG_CAR_LAMP_WHITE: f32 = 8.0;
// Glass: the key light's lobe as wide as the sun's disc gives (0.27 deg:
// GGX ~0.06), the lacquer-like lobe that takes the screen-space reflection
// and the lamps' glints sharp; absorption of a clear laminated screen (about
// 75 % visible transmission) and of privacy glass (the material's own alpha).
const AG_CAR_GLASS_ROUGH: f32 = 0.06;
const AG_CAR_GLASS_COAT_ROUGH: f32 = 0.02;
const AG_CAR_GLASS_CLEAR: f32 = 0.25;
// A lamp cover: polycarbonate (F0 ~0.05 as glass), nearly clear; a red or
// amber cover passes little of what lies behind it untinted (it is seen in
// its own colour, `ag_car_lens`).
const AG_CAR_LENS_CLEAR: f32 = 0.08;
const AG_CAR_LENS_TINTED: f32 = 0.6;
// A lit lamp's cover: the share of the emitter's luminance its optic spreads
// across the cover (a red or amber cover; a clear headlamp cover over its
// projector or bowl); by day what a red or amber cover over its bowl sends
// back (the cover's transmittance in its colour twice, x the bowl's ~0.6).
const AG_CAR_LENS_FILL: f32 = 0.08;
// A red cover lower than this share of the body's mid-height is a bumper's
// passive reflector (the tail lamps sit at 0.55-0.75 of the body's height).
const AG_CAR_LAMP_LOWEST: f32 = 0.9;
const AG_CAR_REFLECTOR_BACK: f32 = 0.35;
const AG_CAR_LENS_FILL_CLEAR: f32 = 0.05;
const AG_CAR_LENS_RED_BACK: vec3<f32> = vec3<f32>(0.36, 0.006, 0.004);
const AG_CAR_LENS_AMBER_BACK: vec3<f32> = vec3<f32>(0.42, 0.12, 0.004);
// The cabin's share of the sky and the ambient light (roof, pillars, the
// body round it: a car interior sees about a third of the open sky).
const AG_CAR_CABIN_AO: f32 = 0.3;
// Saturated LED emission (red ~625 nm, amber ~590 nm in linear sRGB, clipped
// to the gamut): a lamp's core keeps its hue through the tone curve instead
// of whitening on the green the display colours carry.
const AG_CAR_LED_RED: vec3<f32> = vec3<f32>(1.0, 0.012, 0.004);
const AG_CAR_LED_AMBER: vec3<f32> = vec3<f32>(1.0, 0.32, 0.0);
// LEDs and reflector facets on a 12 mm pitch.
const AG_CAR_LED_PITCH: f32 = 0.012;
// (`--car-lamps`) A red or amber lamp's emitter (its light guide or LED row)
// held under this many times the exposure's white, where its cover keeps
// `AG_CAR_LAMP_WHITE`: a lamp's emitter is far brighter than its cover (a
// tail lamp's 8 cd from a few square centimetres of light guide is tens of
// times a lit street's white at night, a stop lamp's hundreds), so its core
// runs past the camera's red and the crosstalk (eo-look
// `tone::SENSOR_CROSSTALK`) turns it orange-white, as a photograph shows a
// lamp, while the cover round it stays a deep red.
const AG_CAR_LAMP_CORE_WHITE: f32 = 24.0;
// (`--car-lamps`) A lamp housing's pillow optics: its chrome facets (18 mm)
// each throw the emitter's light (at night) or the sky's (by day) through the
// cover as a spot (a Gaussian of 0.15 of a facet), the cover between them
// dimmer.
const AG_CAR_FACET_M: f32 = 0.018;
// Carbon-black rubber (albedo ~0.03), road dust on it, raised lettering faded grey.
const AG_CAR_RUBBER: vec3<f32> = vec3<f32>(0.028, 0.028, 0.03);
const AG_CAR_ROAD_DUST: vec3<f32> = vec3<f32>(0.11, 0.095, 0.08);
const AG_CAR_LETTERING: vec3<f32> = vec3<f32>(0.075, 0.075, 0.075);
const AG_CAR_BRAKE_DUST: vec3<f32> = vec3<f32>(0.05, 0.042, 0.034);
// California plate: retroreflective white sheeting, the red script, navy characters (a 5 x 7 face, `ag_car_font`).
const AG_CAR_PLATE_WHITE: vec3<f32> = vec3<f32>(0.62, 0.62, 0.6);
const AG_CAR_PLATE_RED: vec3<f32> = vec3<f32>(0.42, 0.02, 0.02);
const AG_CAR_PLATE_NAVY: vec3<f32> = vec3<f32>(0.012, 0.02, 0.09);
// The rear plate's lamp: the plate's luminance a share of the tail lamps' intensity.
const AG_CAR_PLATE_LAMP: f32 = 0.025;

struct AgCarD {
    // the normal's squared change across the pixel (specular anti-aliasing)
    dn: f32,
    // the pixel's footprint on the surface, m
    px: f32,
    // the texture coordinates' change across the pixel (the plates)
    duv: vec2<f32>,
};

// What `ag_car_finish` needs: the role, whether the surface is thin glass
// (its reflection over its opacity), its own glow (a lit lamp cover), and
// (`--car-reflections`) which layer mirrors the street's far lights: 0 none,
// 1 the lacquer or the glass (sky_coat), 2 bare metal (chrome, a reflector
// bowl, a wheel), and the lacquer's lobe for them (paint: widened further
// where the body's normals bend fast, so a lamp's mirror image does not
// break into glitter on a dimpled panel).
struct AgCar {
    role: u32,
    glass: bool,
    emit: vec3<f32>,
    far: u32,
    far_rough: f32,
    // (`--car-finish`, `ag_car_finish_*`) x the metallic flake's glint weight, y 1 on the
    // cabin (seen through the glass), z the glass's dust haze, w the rain's speckle on the far lights
    fx: vec4<f32>,
};

// Whether the cars' materials draw (agents.rs `AgParams::misc.y`).
fn ag_car_on() -> bool {
    return SF_CAR_MATERIALS && ag.misc.y != 0u && ag_pipe_cars();
}

// The derivatives, taken in uniform control flow before any discard.
fn ag_car_derivs(in: AgOut) -> AgCarD {
    var d = AgCarD(0.0, 0.0, vec2<f32>(0.0));
    if (!SF_CAR_MATERIALS) {
        return d;
    }
    let nx = dpdx(in.normal);
    let ny = dpdy(in.normal);
    let wx = dpdx(in.world);
    let wy = dpdy(in.world);
    d.dn = dot(nx, nx) + dot(ny, ny);
    d.px = sqrt(max(dot(wx, wx), dot(wy, wy)));
    d.duv = abs(dpdx(in.uv)) + abs(dpdy(in.uv));
    return d;
}

fn ag_car_aa(rough: f32, dn: f32) -> f32 {
    let a = rough * rough;
    let a2 = clamp(a * a + min(2.0 * AG_CAR_AA_VARIANCE * dn, AG_CAR_AA_CLAMP), 0.0, 1.0);
    return sqrt(sqrt(a2));
}

// A parked car (eo-app props: stopped, speed 0, wheels never turned; as sky_lights.wgsl `lights_agents`).
fn ag_car_parked(a: AgAgent) -> bool {
    return a.flags == 2u && a.speed == 0.0 && a.anim == 0.0;
}

// LEDs (or a reflector's facets) on a pitch, their mean where the pixel is wider.
fn ag_car_led(p: vec2<f32>, px: f32) -> f32 {
    let c = fract(p / AG_CAR_LED_PITCH) - 0.5;
    let led = 1.0 - smoothstep(0.2, 0.38, length(c));
    let keep = 1.0 - smoothstep(0.3, 0.7, px / AG_CAR_LED_PITCH);
    return mix(1.0, 0.25 + 2.4 * led, keep);
}

// (`--car-lamps`) The pillow optics' glints at a point of the cover (`p` in
// the body's frame across the cover, m): mean 1, a spot of up to five times
// the mean at each facet's centre over a third of it between, where a pixel
// spans a third of a facet or less; the mean where it spans a sixth or more.
fn ag_car_facets(p: vec2<f32>, px: f32) -> f32 {
    let c = fract(p / AG_CAR_FACET_M) - 0.5;
    // a Gaussian spot of sigma 0.15 has a mean of 2 pi 0.15^2 over the facet
    let spot = exp(-dot(c, c) / (2.0 * 0.15 * 0.15)) / (6.2831853 * 0.15 * 0.15);
    return mix(1.0, 0.35 + 0.65 * spot, 1.0 - smoothstep(AG_CAR_FACET_M * 0.16, AG_CAR_FACET_M * 0.33, px));
}

// A seven-segment code (bits a..g) for a tyre sidewall character: digits, then letters.
fn ag_car_glyph(k: u32) -> u32 {
    switch k {
        case 0u: { return 63u; }
        case 1u: { return 6u; }
        case 2u: { return 91u; }
        case 3u: { return 79u; }
        case 4u: { return 102u; }
        case 5u: { return 109u; }
        case 6u: { return 125u; }
        case 7u: { return 7u; }
        case 8u: { return 127u; }
        case 9u: { return 111u; }
        case 10u: { return 119u; }
        case 11u: { return 124u; }
        case 12u: { return 57u; }
        case 13u: { return 94u; }
        case 14u: { return 121u; }
        case 15u: { return 113u; }
        case 16u: { return 118u; }
        case 17u: { return 30u; }
        case 18u: { return 56u; }
        case 19u: { return 115u; }
        default: { return 62u; }
    }
}

// A plate character's 5 x 7 bitmap (rows top to bottom, five bits each,
// the leftmost the highest; rows 0-3 in x, 4-6 in y): the digits, then the
// letters a plate uses (no I, O or Q).
fn ag_car_font(k: u32) -> vec2<u32> {
    switch k {
        case 0u: { return vec2<u32>(708142u, 14905u); } // 0
        case 1u: { return vec2<u32>(135556u, 14468u); } // 1
        case 2u: { return vec2<u32>(67118u, 32004u); } // 2
        case 3u: { return vec2<u32>(69727u, 14881u); } // 3
        case 4u: { return vec2<u32>(600258u, 2143u); } // 4
        case 5u: { return vec2<u32>(64031u, 14881u); } // 5
        case 6u: { return vec2<u32>(999686u, 14897u); } // 6
        case 7u: { return vec2<u32>(133183u, 8456u); } // 7
        case 8u: { return vec2<u32>(476718u, 14897u); } // 8
        case 9u: { return vec2<u32>(509486u, 12353u); } // 9
        case 10u: { return vec2<u32>(1033774u, 17969u); } // A
        case 11u: { return vec2<u32>(1001022u, 31281u); } // B
        case 12u: { return vec2<u32>(541230u, 14896u); } // C
        case 13u: { return vec2<u32>(575068u, 29265u); } // D
        case 14u: { return vec2<u32>(999967u, 32272u); } // E
        case 15u: { return vec2<u32>(999967u, 16912u); } // F
        case 16u: { return vec2<u32>(770606u, 15921u); } // G
        case 17u: { return vec2<u32>(1033777u, 17969u); } // H
        case 18u: { return vec2<u32>(67655u, 12866u); } // J
        case 19u: { return vec2<u32>(807505u, 18004u); } // K
        case 20u: { return vec2<u32>(541200u, 32272u); } // L
        case 21u: { return vec2<u32>(710513u, 17969u); } // M
        case 22u: { return vec2<u32>(714289u, 17971u); } // N
        case 23u: { return vec2<u32>(1001022u, 16912u); } // P
        case 24u: { return vec2<u32>(1001022u, 18004u); } // R
        case 25u: { return vec2<u32>(475663u, 30753u); } // S
        case 26u: { return vec2<u32>(135327u, 4228u); } // T
        case 27u: { return vec2<u32>(575025u, 14897u); } // U
        case 28u: { return vec2<u32>(575025u, 4433u); } // V
        case 29u: { return vec2<u32>(706097u, 10933u); } // W
        case 30u: { return vec2<u32>(141873u, 17962u); } // X
        case 31u: { return vec2<u32>(141873u, 4228u); } // Y
        default: { return vec2<u32>(133183u, 32264u); } // Z
    }
}

fn ag_car_font_bit(b: vec2<u32>, c: i32, r: i32) -> f32 {
    if (c < 0 || c > 4 || r < 0 || r > 6) {
        return 0.0;
    }
    let w = select(b.y, b.x, r < 4);
    let row = (w >> (5u * u32(r % 4))) & 31u;
    return f32((row >> u32(4 - c)) & 1u);
}

// A character's ink at gp (0..1 across, 0..1 up its box): its bitmap read
// bilinearly and cut at half (rounded strokes a cell wide), the cut's width
// the pixel's footprint `f` in cells.
fn ag_car_font_ink(b: vec2<u32>, gp: vec2<f32>, f: vec2<f32>) -> f32 {
    let q = vec2<f32>(gp.x * 5.0, (1.0 - gp.y) * 7.0) - 0.5;
    let i = vec2<i32>(floor(q));
    let t = q - floor(q);
    let v = mix(mix(ag_car_font_bit(b, i.x, i.y), ag_car_font_bit(b, i.x + 1, i.y), t.x), mix(ag_car_font_bit(b, i.x, i.y + 1), ag_car_font_bit(b, i.x + 1, i.y + 1), t.x), t.y);
    let w = clamp(max(f.x * 5.0, f.y * 7.0) * 0.5, 0.05, 0.5);
    return smoothstep(0.5 - w, 0.5 + w, v);
}

// The cover of a box (centre c, half size h) at p, filtered over `f`.
fn ag_car_box(p: vec2<f32>, c: vec2<f32>, h: vec2<f32>, f: vec2<f32>) -> f32 {
    let q = (h - abs(p - c)) / max(f, vec2<f32>(1e-5));
    return clamp(min(q.x, q.y) + 0.5, 0.0, 1.0);
}

// A California plate at its own coordinates `uv` (0..1 across as read, 0..1
// up): rgb albedo, a its roughness. `h` the car's hash, `dirt` its grime.
fn ag_car_plate(uv: vec2<f32>, duv: vec2<f32>, h: f32, dirt: f32, rear: bool) -> vec4<f32> {
    var c = AG_CAR_PLATE_WHITE;
    var r = 0.45;
    // The characters: seven in a row (a digit, three letters, three digits), embossed.
    let x0 = 0.085;
    let cw = (1.0 - 2.0 * x0) / 7.0;
    let k = floor((uv.x - x0) / cw);
    var ink = 0.0;
    if (k >= 0.0 && k < 7.0 && uv.y > 0.2 && uv.y < 0.7) {
        let gp = vec2<f32>((fract((uv.x - x0) / cw) - 0.12) / 0.76, (uv.y - 0.22) / 0.44);
        let f = vec2<f32>(duv.x / (cw * 0.76), duv.y / 0.44);
        let hk = st_hash(vec2<f32>(h * 97.0 + k * 13.1, k + 5.3));
        let letter = k >= 1.0 && k <= 3.0;
        ink = ag_car_font_ink(ag_car_font(select(u32(hk * 9.99), 10u + u32(hk * 22.99), letter)), gp, f);
        // under a few pixels a character: its mean ink (no shimmer, the same tone).
        let keep = 1.0 - smoothstep(0.12, 0.35, max(f.x, f.y) * 0.5);
        ink = mix(0.3, ink, keep);
    }
    c = mix(c, AG_CAR_PLATE_NAVY, ink);
    // "California" in red script across the top (strokes, no letters).
    let sx = (uv.x - 0.3) / 0.4;
    if (sx > 0.0 && sx < 1.0 && uv.y > 0.76 && uv.y < 0.9) {
        let wave = 0.5 + 0.5 * sin(sx * 70.0 + 3.0 * sin(sx * 23.0 + h * 6.0));
        let keep = 1.0 - smoothstep(0.004, 0.012, duv.x);
        c = mix(c, AG_CAR_PLATE_RED, mix(0.45, smoothstep(0.45, 0.7, wave), keep) * smoothstep(0.76, 0.79, uv.y) * (1.0 - smoothstep(0.87, 0.9, uv.y)));
    }
    // The registration sticker on the rear plate's top right corner (the year's colour).
    if (rear && uv.x > 0.8 && uv.x < 0.93 && uv.y > 0.74 && uv.y < 0.92) {
        let yc = floor(h * 5.0);
        c = select(select(select(vec3<f32>(0.6, 0.25, 0.02), vec3<f32>(0.02, 0.18, 0.5), yc < 1.0), vec3<f32>(0.5, 0.42, 0.02), yc > 3.0), vec3<f32>(0.03, 0.3, 0.06), yc == 2.0);
        r = 0.35;
    }
    // Bolts near the top corners.
    let bl = length((vec2<f32>(abs(uv.x - 0.5), uv.y) - vec2<f32>(0.33, 0.86)) * vec2<f32>(2.0, 1.0));
    if (bl < 0.035) {
        c = vec3<f32>(0.22, 0.22, 0.21);
        r = 0.4;
    }
    // A frame round it (a dealer's plastic surround, deeper at the foot).
    if (uv.x < 0.035 || uv.x > 0.965 || uv.y < 0.1 || uv.y > 0.95) {
        c = vec3<f32>(0.02, 0.02, 0.022);
        r = 0.5;
    }
    // Road dirt from the foot and the edges, spotted.
    let edge = min(min(uv.x, 1.0 - uv.x) * 2.0, uv.y);
    let spots = st_noise(uv * vec2<f32>(40.0, 20.0) + h * 31.0);
    let film = clamp(dirt * (0.35 + 0.65 * (1.0 - smoothstep(0.0, 0.6, edge))) * (0.6 + 0.8 * spots), 0.0, 0.85);
    c = mix(c, AG_CAR_ROAD_DUST * 0.8, film);
    r = mix(r, 0.75, film);
    return vec4<f32>(c, r);
}

// The surface and material of a car's fragment by its role, and the lacquer
// (sky_coat) for look_light; what `ag_car_finish` needs after it.
fn ag_car_prepare(g: ptr<function, AgSurf>, in: AgOut, d: AgCarD) -> AgCar {
    var k = AgCar(0u, false, vec3<f32>(0.0), 0u, 0.0, vec4<f32>(0.0));
    let a = ag_agents[in.agent];
    let md = ag_models[a.model];
    if (md.model_class == AG_PERSON) {
        return k;
    }
    let base = materials[in.material];
    let role = (base.flags >> AG_ROLE_SHIFT) & 15u;
    k.role = role;
    let local = in.local;
    let parked = ag_car_parked(a);
    // the car's own numbers (as ag_weather keys them): age, grime.
    let hs = vec2<f32>(f32(a.color & 0xfffu), f32(a.model) + f32((a.color >> 12u) & 0xfffu) * 0.37);
    let h = st_hash(hs + vec2<f32>(11.7, 4.3));
    let age = smoothstep(0.25, 1.0, st_hash(hs + vec2<f32>(7.7, 2.1)));
    let hd = st_hash(hs + vec2<f32>(3.3, 1.7));
    let dirt = select(0.2, select(select(WX_CAR_SOME, WX_CAR_FILTHY, hd > 1.0 - WX_CAR_FILTHY_SHARE), WX_CAR_CLEAN, hd < WX_CAR_CLEAN_SHARE), wx_level() != 0u);
    // the normal in the body's frame.
    let nb = ag_rotate(vec4<f32>(-a.rot.xyz, a.rot.w), (*g).n);
    let lamp = base.flags & (AG_M_TAIL | AG_M_BRAKE | AG_M_INDICATOR | AG_M_HEAD | AG_M_DRL | AG_M_REVERSE);
    let wet = select(0.0, skyu.wet.x, sky_on(skyu.flags.x, SKY_WET));
    // The body's own shade: the wheel arches (inside the arch round each
    // wheel, inboard of the tyre's face) and the underside (facing down, over
    // the shaded road under the car).
    var piv = md.wheel[0].xyz;
    var wi = 0u;
    for (var i = 1u; i < 4u; i = i + 1u) {
        if (distance(local, md.wheel[i].xyz) < distance(local, piv)) {
            piv = md.wheel[i].xyz;
            wi = i;
        }
    }
    let r = max(md.wheel_radius, 0.2);
    let arch = length(local.yz - piv.yz) / r;
    var occ = 1.0;
    if (role != AG_ROLE_TYRE && role != AG_ROLE_WHEEL && role != AG_ROLE_BRAKE) {
        occ = mix(0.14, 1.0, smoothstep(1.0, 1.48, arch) + smoothstep(0.0, 0.14, abs(local.x) - abs(piv.x) + 0.1));
    } else {
        // Shade the upper half of wheels/tyres tucked inside the wheel well.
        occ = mix(1.0, 0.36, smoothstep(0.15, 0.85, (local.y - piv.y) / r));
    }
    occ = min(occ, mix(1.0, 0.16, smoothstep(0.05, 0.65, -nb.y)));
    (*g).s.ao = (*g).s.ao * clamp(occ, 0.0, 1.0);
    if (lamp != 0u) {
        // Lamps (lit as ag_lamp lights them): LED colours, the LEDs' structure.
        let i = max(max((*g).m.emissive.r, (*g).m.emissive.g), (*g).m.emissive.b);
        var col = base.emissive.rgb / max(max(max(base.emissive.r, base.emissive.g), base.emissive.b), 1e-4);
        if ((base.flags & (AG_M_TAIL | AG_M_BRAKE)) != 0u) {
            col = AG_CAR_LED_RED;
        } else if ((base.flags & AG_M_INDICATOR) != 0u) {
            col = AG_CAR_LED_AMBER;
        }
        var hold = 1.0;
        if ((base.flags & (AG_M_TAIL | AG_M_BRAKE | AG_M_INDICATOR)) != 0u) {
            // (`--car-lamps`: the emitter itself, a light guide or an LED row, at
            // its core's hold; the cover round it keeps the lamp's)
            let white = select(AG_CAR_LAMP_WHITE, AG_CAR_LAMP_CORE_WHITE, ag_car_lamps_on());
            hold = min(1.0, white / max(ag.lamp_tail.w * look.key.w, 1e-4));
        }
        let face = select(local.zy, local.xy, abs(nb.z) > 0.5);
        (*g).m.emissive = vec4<f32>(col * i * hold * ag_car_led(face, d.px), 0.0);
        return k;
    }
    // The base coat under the lacquer: the paint's colour as its pigment
    // gives it back through the lacquer both ways (the lacquer's own
    // reflection is the coat's), older paint chalked toward a warm grey.
    if ((base.flags & AG_M_PAINT) != 0u) {
        (*g).s.albedo = (*g).s.albedo * AG_CAR_BASE_COAT * mix(vec3<f32>(1.0), AG_CAR_CHALK, age);
    }
    // Road grime (`--grime`) on the body, its trim and liveries.
    let paint = (base.flags & AG_M_PAINT) != 0u || role == AG_ROLE_LIVERY;
    if (wx_level() != 0u && (paint || role == 0u || role == AG_ROLE_TRIM)) {
        if (ag_car_finish_on()) {
            ag_cf_grime(&(*g).s, local, nb, piv, r, arch, dirt, d.px, md); // (`--car-finish`) where the road and the weather put it
        } else {
            ag_car_grime(&(*g).s, local, nb, piv, r, arch, dirt, d.px);
        }
    }
    // (`--car-finish`) rain on the lacquer and the glass: one drop field (beads
    // on the paint, drops on the panes), its tilt for the branch below
    var drop = vec3<f32>(0.0);
    let glassy = role == AG_ROLE_GLASS || role == AG_ROLE_GLASS_TINT;
    let rain = select(0.0, wet, ag_car_finish_on() && skyu.wet.z > 0.05 && (paint || glassy));
    if (rain > 0.0) {
        drop = ag_cf_drop(local, nb, d.px, select(AG_CF_BEAD_M, AG_CF_DROP_M, glassy), select(AG_CF_BEAD_COVER, AG_CF_DROP_COVER, glassy) * rain);
        drop = ag_rotate(a.rot, drop);
    }
    if (paint) {
        // The base coat (flake in a binder) under the lacquer, at the environment's physical strength.
        if (wet > 0.02) {
            sky_wet_generic(&(*g).s, (*g).n);
        }
        let grime = clamp(((*g).s.rough - base.params.x) / 0.4, 0.0, 1.0);
        let pc = ag_unpack_srgb(a.color);
        let pl = dot(pc, vec3<f32>(0.2126, 0.7152, 0.0722));
        let grey = max(max(pc.r, pc.g), pc.b) - min(min(pc.r, pc.g), pc.b) < 0.25 * max(max(pc.r, pc.g), pc.b);
        var flake = select(0.0, 1.0, h < 0.5);
        if (grey && pl > 0.08 && pl < 0.6) {
            flake = 1.0;
        } else if (pl >= 0.6) {
            flake = 0.0;
        }
        if (role != AG_ROLE_LIVERY) {
            (*g).s.metal = AG_CAR_FLAKE_METAL * flake;
        }
        (*g).s.rough = ag_car_aa(max((*g).s.rough, 0.3), d.dn);
        // the base coat's diffuse at the facades' class (as the street round it
        // is lit), its reflections and the lacquer's at the painted-metal class.
        sky_coat_env = ST_ENV_PAINTED_METAL;
        // grime lies on the lacquer as a film of matte particles: where it
        // covers, the lacquer's reflection is gone (the film's own diffuse is
        // the base's); between the particles the lacquer stays a lacquer,
        // only a little hazier (not a glossy 0.35 lobe a lamp's highlight
        // would spread across the film).
        sky_coat = 1.0 - AG_CAR_FILM_COVER * grime;
        let coat = mix(mix(AG_CAR_COAT_NEW, AG_CAR_COAT_OLD, age), AG_CAR_COAT_FILMED, grime);
        sky_coat_rough = ag_car_aa(mix(coat, AG_CAR_GLASS_COAT_ROUGH, wet * smoothstep(0.3, 0.8, nb.y)), d.dn);
        k.far = 1u;
        k.far_rough = ag_car_aa(sky_coat_rough, AG_CAR_FAR_AA * d.dn);
        if (ag_car_finish_on()) {
            ag_cf_paint(g, &k, local, nb, h, age, rain, drop, select(flake, 0.0, role == AG_ROLE_LIVERY), d, role == AG_ROLE_LIVERY);
        }
        return k;
    }
    switch role {
        case 1u, 2u, 6u: {
            // Thin glass: a dielectric's reflection over what is behind it.
            var absorb = max(base.base.a, AG_CAR_GLASS_CLEAR);
            if (role == AG_ROLE_LENS) {
                let hi = max(max(base.base.r, base.base.g), base.base.b);
                let tinted = hi - min(min(base.base.r, base.base.g), base.base.b) > 0.5 * hi;
                absorb = select(AG_CAR_LENS_CLEAR, AG_CAR_LENS_TINTED, tinted);
                let red = base.base.r > 2.5 * base.base.g;
                let fill = ag_car_lens(tinted, red, local, md, a);
                if (tinted) {
                    // A red or amber cover: a coloured translucent layer lit as
                    // a surface is (by day the light its bowl sends back through
                    // it, its colour twice), what lies behind it showing through
                    // the rest, the lacquer-like gloss over it, its lamp's fill
                    // (the blend takes the layer at its cover).
                    // (a bumper's passive reflector: no bowl behind it, a deeper red)
                    let reflector = red && local.y < AG_CAR_LAMP_LOWEST * md.center.y;
                    (*g).s.albedo = select(AG_CAR_LENS_AMBER_BACK, AG_CAR_LENS_RED_BACK, red) * select(1.0, AG_CAR_REFLECTOR_BACK, reflector);
                    // (`--car-lamps`) the housing's pillow optics through the cover
                    // (`ag_car_facets`): its lamp's fill and, by day, its bowl's
                    // return broken into a grid of glints of the same mean
                    var facets = 1.0;
                    if (SF_CAR_LAMPS && ag_car_lamps_on() && !reflector) {
                        facets = ag_car_facets(select(local.zy, local.xy, abs(nb.z) > 0.5), d.px);
                        (*g).s.albedo = (*g).s.albedo * mix(1.0, facets, 0.5);
                    }
                    (*g).s.metal = 0.0;
                    (*g).s.rough = ag_car_aa(0.35, d.dn);
                    (*g).s.alpha = absorb;
                    (*g).m.flags = (*g).m.flags | F_GLASS;
                    (*g).m.emissive = vec4<f32>(fill * facets / absorb, 0.0);
                    sky_coat = 1.0;
                    sky_coat_rough = ag_car_aa(AG_CAR_GLASS_COAT_ROUGH + 0.04 * dirt * age, d.dn);
                    k.far = 1u;
                    k.far_rough = sky_coat_rough;
                    return k;
                }
                k.emit = fill;
            }
            (*g).s.albedo = vec3<f32>(0.0);
            (*g).s.metal = 0.0;
            (*g).s.rough = ag_car_aa(AG_CAR_GLASS_ROUGH + 0.05 * dirt * age, d.dn);
            (*g).m.flags = (*g).m.flags | F_GLASS | F_THIN_GLASS;
            (*g).m.thin_absorption = absorb;
            sky_coat = 1.0;
            sky_coat_rough = ag_car_aa(AG_CAR_GLASS_COAT_ROUGH + 0.04 * dirt * age, d.dn);
            k.glass = true;
            k.far = 1u;
            k.far_rough = sky_coat_rough;
            if (ag_car_finish_on() && role != AG_ROLE_LENS) {
                ag_cf_glass(g, &k, local, nb, md, age, dirt, rain, drop, d);
            }
        }
        case 3u: {
            // Rubber: the sidewall's raised lettering on the outer face, road
            // dust on the tread and the lower sidewall. The wheel's own frame:
            // the steer and the spin undone.
            var q = local - piv;
            if (wi < 2u) {
                let cs = cos(a.steer);
                let sn = sin(a.steer);
                q = vec3<f32>(q.x * cs - q.z * sn, q.y, q.x * sn + q.z * cs);
            }
            let rr = length(q.yz) / r;
            let tw = atan2(q.z, q.y) - a.anim;
            let th = tw - floor(tw / 6.2831853) * 6.2831853;
            let outer = q.x * sign(piv.x) > 0.0;
            let side = abs(nb.x) > 0.55;
            var c = AG_CAR_RUBBER;
            var ro = 0.72;
            if (side && outer && rr > 0.76 && rr < 0.9) {
                let cells = max(round(6.2831853 * r / 0.035), 8.0);
                let t = th / 6.2831853 * cells;
                let ci = floor(t);
                let blk = fract(ci / 46.0);
                let gp = vec2<f32>(fract(t), (rr - 0.78) / 0.1);
                let code = ag_car_glyph(u32(st_hash(vec2<f32>(ci, 3.1)) * 20.99));
                let f = vec2<f32>(d.px / (0.035), d.px / (0.1 * r));
                var ink = 0.0;
                if (blk < 0.32 || (blk > 0.5 && blk < 0.62)) {
                    let w = 0.12;
                    if ((code & 1u) != 0u) { ink = max(ink, ag_car_box(gp, vec2<f32>(0.5, 1.0 - w), vec2<f32>(0.32, w), f)); }
                    if ((code & 6u) != 0u) { ink = max(ink, ag_car_box(gp, vec2<f32>(0.8, 0.5), vec2<f32>(w, 0.42), f)); }
                    if ((code & 8u) != 0u) { ink = max(ink, ag_car_box(gp, vec2<f32>(0.5, w), vec2<f32>(0.32, w), f)); }
                    if ((code & 48u) != 0u) { ink = max(ink, ag_car_box(gp, vec2<f32>(0.2, 0.5), vec2<f32>(w, 0.42), f)); }
                    if ((code & 64u) != 0u) { ink = max(ink, ag_car_box(gp, vec2<f32>(0.5, 0.5), vec2<f32>(0.32, w), f)); }
                    ink = mix(0.25, ink, 1.0 - smoothstep(0.15, 0.4, max(f.x, f.y)));
                }
                c = mix(c, AG_CAR_LETTERING, ink * (0.5 + 0.5 * age));
                ro = mix(ro, 0.85, ink);
            }
            let low = 1.0 - smoothstep(-0.2, 0.6, q.y / r);
            let dust = clamp(select(0.25 + 0.55 * low, 0.65, !side) * (0.4 + 0.6 * dirt), 0.0, 1.0);
            (*g).s.albedo = mix(c, AG_CAR_ROAD_DUST, dust * 0.55);
            (*g).s.rough = mix(ro, 0.9, dust);
            (*g).s.metal = 0.0;
            if (ag_car_finish_on() && wet > 0.02) {
                // (`--car-finish`) wet rubber: the water darkens the dust and
                // smooths the tread's face to a sheen
                (*g).s.albedo = mix((*g).s.albedo, AG_CAR_RUBBER * 0.8, wet * 0.7);
                (*g).s.rough = mix((*g).s.rough, AG_CF_WET_RUBBER_ROUGH, wet);
            }
            (*g).s.ao = (*g).s.ao * mix(0.55, 1.0, smoothstep(0.55, 0.95, rr));
        }
        case 4u: {
            let rear = local.z < md.center.z;
            let p = ag_car_plate(in.uv, d.duv, h, dirt, rear);
            (*g).s.albedo = p.rgb;
            (*g).s.rough = p.a;
            (*g).s.metal = 0.0;
            if (rear && !parked) {
                // the plate lamp, lit with the tail lamps.
                let tail = mix(ag.lamp_tail.x, ag.lamp_tail.y, ag_lamp_night());
                (*g).m.emissive = vec4<f32>(p.rgb * tail * AG_CAR_PLATE_LAMP, 0.0);
            }
        }
        case 5u, 10u: {
            // Chrome and the lamps' reflector bowls: mirrors at the
            // environment's strength; a bowl glows with its lit lamp.
            (*g).s.env = ENV_PAINTED_METAL;
            (*g).s.rough = ag_car_aa((*g).s.rough, d.dn);
            k.far = 2u;
            if (role == AG_ROLE_REFLECTOR && !parked) {
                let front = local.z > md.center.z;
                // (`--car-lamps`: a front bowl glows with the headlamps alone, not the running lamps' strip)
                let i = ag_lamp(a.flags, select(AG_M_TAIL, select(AG_M_HEAD | AG_M_DRL, AG_M_HEAD, ag_car_lamps_on()), front), local.x);
                let col = select(AG_CAR_LED_RED, vec3<f32>(1.0, 0.97, 0.92), front);
                let ring = 0.6 + 0.4 * sin(length(local.xy - vec2<f32>(sign(local.x) * abs(piv.x) * 0.75, piv.y + 0.28)) / AG_CAR_LED_PITCH * 3.14159);
                (*g).m.emissive = vec4<f32>(col * i * 0.3 * mix(1.0, ring, 1.0 - smoothstep(0.004, 0.012, d.px)), 0.0);
            }
        }
        case 7u: {
            // The cabin in its own shade; the screen dark in a parked car.
            (*g).s.ao = (*g).s.ao * AG_CAR_CABIN_AO;
            (*g).m.emissive = (*g).m.emissive * select(0.35, 0.0, parked);
            if (ag_car_finish_on()) {
                // (`--car-finish`) matte dark plastics and cloth (no sun-lit
                // sheen of a metallic trim), seen through the glass (`ag_car_finish`)
                let screen = base.params.x < 0.1;
                (*g).s.albedo = min((*g).s.albedo, vec3<f32>(AG_CF_CABIN_ALBEDO_MAX));
                (*g).s.metal = 0.0;
                (*g).s.rough = select(max((*g).s.rough, AG_CF_CABIN_ROUGH), (*g).s.rough, screen);
                k.fx.y = 1.0;
            }
        }
        case 8u: {
            // Wheels: brake dust (dielectric, brown-black) toward the rim and on the front axle.
            let rr = length(local.yz - piv.yz) / r;
            let bd = clamp((0.2 + 0.5 * dirt + select(0.0, 0.2, wi < 2u)) * (0.5 + 0.5 * smoothstep(0.3, 0.75, rr)), 0.0, 0.9);
            (*g).s.albedo = mix((*g).s.albedo, AG_CAR_BRAKE_DUST, bd);
            (*g).s.metal = mix((*g).s.metal, 0.15, bd * 0.8);
            (*g).s.rough = ag_car_aa(mix((*g).s.rough, 0.6, bd), d.dn);
            (*g).s.env = ENV_PAINTED_METAL;
            k.far = 2u;
        }
        case 9u: {
            // Black plastics, sun-faded to a chalky grey on older cars.
            (*g).s.albedo = mix((*g).s.albedo, vec3<f32>(0.055, 0.055, 0.058), age * 0.8);
            (*g).s.rough = ag_car_aa(mix((*g).s.rough, 0.7, age), d.dn);
        }
        case 11u: {
            // A light bar lit only on a moving unit answering a call (a third of
            // them), its halves alternating; dark and clear-red or blue otherwise.
            let answering = !parked && a.speed > 0.5 && h < 0.33;
            let on = answering && ((fract(ag.lamp_misc.z * 1.5) < 0.5) == (local.x > 0.0));
            (*g).s.albedo = (*g).s.albedo * 0.4;
            (*g).m.emissive = vec4<f32>(select(vec3<f32>(0.0), base.emissive.rgb * 6.0, on), 0.0);
        }
        case 14u: {
            // (`--car-panels`) a badge: a pressed chrome emblem (`ag_car_badge`)
            if (SF_CAR_PANELS) {
                ag_car_badge(g, in.uv, d, nb, a);
                k.far = 2u;
            } else {
                (*g).s.rough = ag_car_aa((*g).s.rough, d.dn);
            }
        }
        case 12u: {
            // (`--car-finish`) a caliper (the painted part of the brake) as street
            // cars carry them: most bare cast iron, some black, a few red, the
            // body's own colour on the rest
            let bh = max(max(base.base.r, base.base.g), base.base.b);
            if (ag_car_finish_on() && bh - min(min(base.base.r, base.base.g), base.base.b) > 0.3 * bh) {
                let hc = st_hash(hs + vec2<f32>(5.1, 8.3));
                let finish = select(select(select((*g).s.albedo, AG_CF_CALIPER_RED, hc < AG_CF_CALIPER_SHARE.z), AG_CF_CALIPER_BLACK, hc < AG_CF_CALIPER_SHARE.y), AG_CF_CALIPER_IRON, hc < AG_CF_CALIPER_SHARE.x);
                (*g).s.albedo = mix(finish, AG_CAR_BRAKE_DUST, 0.3 + 0.4 * dirt);
                (*g).s.metal = select(0.0, 0.4, hc < AG_CF_CALIPER_SHARE.x);
                (*g).s.rough = max((*g).s.rough, 0.55);
            }
            // Discs and calipers: rust on an old or parked car's discs.
            let rust = age * select(0.3, 0.7, parked);
            (*g).s.albedo = mix((*g).s.albedo, vec3<f32>(0.1, 0.045, 0.02), rust);
            (*g).s.rough = ag_car_aa(mix((*g).s.rough, 0.85, rust), d.dn);
            (*g).s.metal = (*g).s.metal * (1.0 - rust);
        }
        default: {
            (*g).s.rough = ag_car_aa((*g).s.rough, d.dn);
        }
    }
    return k;
}

// A car's normal from its refined packing (eo-agents `vehicles::pack_normal_fine`,
// `unpack_normal_fine`; a spare byte of 0, a person's, is never read here):
// the two smaller 8-bit components (by magnitude, the lowest axis on a tie)
// corrected by fourteenths of a step from the spare byte's code, the largest
// from unit length with its 8-bit sign.
fn ag_car_normal(p: vec4<f32>) -> vec3<f32> {
    let q = round(p.xyz * 127.0);
    let b = i32(round(p.w * 127.0));
    let c = b + select(111, 112, b < 0);
    let e1 = f32(c % 15 - 7) / 14.0;
    let e2 = f32(c / 15 - 7) / 14.0;
    let a = abs(q);
    if (a.x >= a.y && a.x >= a.z) {
        let y = (q.y + e1) / 127.0;
        let z = (q.z + e2) / 127.0;
        return vec3<f32>(sign(q.x) * sqrt(max(1.0 - y * y - z * z, 0.0)), y, z);
    }
    if (a.y >= a.z) {
        let x = (q.x + e1) / 127.0;
        let z = (q.z + e2) / 127.0;
        return vec3<f32>(x, sign(q.y) * sqrt(max(1.0 - x * x - z * z, 0.0)), z);
    }
    let x = (q.x + e1) / 127.0;
    let y = (q.y + e2) / 127.0;
    return vec3<f32>(x, y, sign(q.z) * sqrt(max(1.0 - x * x - y * y, 0.0)));
}

// ---- (`--car-panels`; SF_CAR_PANELS, agents.rs `car_wgsl`: compiled out
// without it) A badge (eo-agents `ROLE_BADGE`: a `Badge_*` chrome emblem, its
// vertices' uv its own disc coordinates, 0..1 across and up): a pressed chrome
// emblem, flat across its face with a bevelled rim turning to the side, a
// raised polished ring and a polished bar on a satin field inside it, the
// patterns' mean where a pixel does not resolve them. The mesh is a thin lens
// of a few vertices whose interpolated normals sweep across the whole face, so
// it shaded as a chrome ball mirroring the whole sky round it (a glowing white
// sphere in the low sun); its face now mirrors what stands before it. ----
// The face is flat to this share of the radius; the rim's tilt at the edge
// (radians, 60 degrees); the polished and the satin chrome's roughness.
const AG_CAR_BADGE_FLAT: f32 = 0.86;
const AG_CAR_BADGE_BEVEL: f32 = 1.05;
const AG_CAR_BADGE_POLISH: f32 = 0.06;
const AG_CAR_BADGE_SATIN: f32 = 0.3;
fn ag_car_badge(g: ptr<function, AgSurf>, uv: vec2<f32>, d: AgCarD, nb: vec3<f32>, a: AgAgent) {
    let q = uv * 2.0 - 1.0;
    let r = length(q);
    // the pixel's footprint across the disc (q spans 2 where uv spans 1)
    let f = max(2.0 * max(d.duv.x, d.duv.y), 1e-4);
    // the face's axis (the body's nose or tail) and the way out from its
    // centre (the lens's own normal leans outward from it)
    let axis = vec3<f32>(0.0, 0.0, select(-1.0, 1.0, nb.z >= 0.0));
    let out = normalize(vec3<f32>(nb.x, nb.y, 0.0) + vec3<f32>(0.0, 1e-4, 0.0));
    let bevel = smoothstep(AG_CAR_BADGE_FLAT, 1.0, r);
    let tilt = AG_CAR_BADGE_BEVEL * bevel;
    let n = ag_rotate(a.rot, normalize(axis * cos(tilt) + out * sin(tilt)));
    (*g).s.n = n;
    (*g).n = n;
    // the ring and the bar polished, the field satin, the rim polished
    let ring = smoothstep(0.66 - f, 0.66 + f, r) * (1.0 - smoothstep(0.82 - f, 0.82 + f, r));
    let bar = ag_car_box(q, vec2<f32>(0.0), vec2<f32>(0.42, 0.1), vec2<f32>(f));
    let polish = clamp(max(max(ring, bar), smoothstep(0.84, 0.88, r)), 0.0, 1.0);
    let p = mix(0.45, polish, 1.0 - smoothstep(0.15, 0.4, f));
    // specular anti-aliasing from the bevel's own turn across the pixel (the face does not turn)
    let turn = AG_CAR_BADGE_BEVEL / (1.0 - AG_CAR_BADGE_FLAT) * f;
    (*g).s.rough = ag_car_aa(mix(AG_CAR_BADGE_SATIN, AG_CAR_BADGE_POLISH, p), turn * turn * smoothstep(AG_CAR_BADGE_FLAT - f, AG_CAR_BADGE_FLAT, r));
    (*g).s.metal = 1.0;
    (*g).s.env = ENV_PAINTED_METAL;
}

// A lit lamp's cover: its lamp fills it (the light the lamp's optic and bowl
// spread across it, its colour round the brighter LEDs or bulb, instead of
// lit edges round a dark cover). A red or amber cover is its tail, brake or
// indicator lamp's, a clear one the headlamp's in front, the reversing
// lamp's behind.
fn ag_car_lens(tinted: bool, red: bool, local: vec3<f32>, md: AgModel, a: AgAgent) -> vec3<f32> {
    var lf = AG_M_REVERSE;
    var col = vec3<f32>(1.0, 0.97, 0.92);
    var hold = 1.0;
    if (tinted) {
        lf = select(AG_M_INDICATOR, AG_M_TAIL, red);
        col = select(AG_CAR_LED_AMBER, AG_CAR_LED_RED, red);
        hold = min(1.0, AG_CAR_LAMP_WHITE / max(ag.lamp_tail.w * look.key.w, 1e-4));
        if (local.y < AG_CAR_LAMP_LOWEST * md.center.y) {
            return vec3<f32>(0.0); // a red reflector in the bumper: no lamp behind it
        }
    } else if (local.z > md.center.z) {
        if (ag_car_parked(a)) {
            return vec3<f32>(0.0); // a parked car's headlamps are off
        }
        // (`--car-lamps`: a clear cover is filled by its headlamps, not the running lamps' strip under it)
        lf = select(AG_M_HEAD | AG_M_DRL, AG_M_HEAD, ag_car_lamps_on());
    }
    return col * ag_lamp(a.flags, lf, local.x) * hold * select(AG_CAR_LENS_FILL_CLEAR, AG_CAR_LENS_FILL, tinted);
}

// After look_light: thin glass's reflection (with the lamps' glints the
// coat's gather found, which the thin-glass path leaves out) over its
// opacity, so the blend gives the reflection plus the cabin seen through it
// (look_light's alpha is 1 - the transmittance, (1 - F)(1 - absorption)).
// look_light's range fog (f toward F) stays where it was: what the blend
// adds behind the glass is already fogged, so out = (c - f F + (1 - f) x the
// glints) / alpha + f F gives mix(reflection + T x behind, F, f).
// (`--car-reflections`) the far lights' reflection `far` joins the glints
// (fogged as look_light fogged the rest); a tinted cover's (blended at its
// alpha) over its alpha, so the blend keeps it whole.
fn ag_car_finish(c_in: vec4<f32>, k: AgCar, world: vec3<f32>, g: AgSurf, local: vec3<f32>, px: f32) -> vec4<f32> {
    var c = c_in;
    if (SF_CAR_FINISH && ag_car_finish_on()) {
        c = ag_cf_light(c, k, world, local, g, px); // (`--car-finish`) the cabin's colour, the glass's haze, the flakes' glints
    }
    var far = vec3<f32>(0.0);
    if (SF_CAR_REFLECTIONS && k.far != 0u) {
        let far_occ = g.s.ao * smoothstep(-0.28, 0.08, g.s.n.y);
        if (k.far == 1u) {
            far = sky_coat * far_occ * ag_car_far_lights(world, g.n, g.v, vec3<f32>(0.04), k.far_rough);
        } else {
            far = far_occ * ag_car_far_lights(world, g.s.n, g.v, mix(vec3<f32>(0.04), g.s.albedo, g.s.metal), g.s.rough);
        }
        if (SF_CAR_FINISH && k.fx.w > 0.0 && px < AG_CF_GLINT_PX) {
            // (`--car-finish`) drops too small for their lens: the lamps they mirror
            // break into a speckle (a fifth of the pixels' drops face the lamp; the
            // mean kept), only where a pixel resolves a glint (`ag_cf_resolved`);
            // further out a frame shows the mean (the lobe `ag_cf_paint` widened)
            let u = ag_cf_cell_hash(local, px, 11.0);
            far = far * mix(1.0, select(0.25, 4.0, u < 0.2), k.fx.w * ag_cf_resolved(px));
        }
    }
    if (k.glass) {
        let a = max(c.a, 0.02);
        let f = ag_car_fog(world, g.m);
        let ff = look.fog_color.xyz * f;
        return vec4<f32>((c.rgb - ff + (1.0 - f) * (sky_coat_lights + k.emit + far)) / a + ff, a);
    }
    if (SF_CAR_REFLECTIONS && k.far != 0u) {
        let over = select(1.0, 1.0 / max(c.a, 0.02), (g.m.flags & F_GLASS) != 0u);
        return vec4<f32>(c.rgb + far * (1.0 - ag_car_fog(world, g.m)) * over, c.a);
    }
    return c;
}

// look_light's range fog at a car's point (its share toward the fog colour).
fn ag_car_fog(world: vec3<f32>, m: Material) -> f32 {
    var vd = dot(world - look.cam_pos.xyz, look.cam_fwd.xyz);
    if (skyu.flags.x != 0u) {
        let u = max(abs(world.y - look.cam_pos.y) - 20.0, 0.0) / 400.0;
        if (u > 1e-2) {
            vd = vd * (1.0 - exp(-u)) / u;
        }
    }
    return lk_smooth(look.fog.x, look.fog.y, lm_fog_depth(m, vd));
}

// ---- (`--car-reflections`; SF_CAR_REFLECTIONS, look.rs `ShaderFeatures`:
// compiled out without it) The street's brightest lights mirrored in a car
// at any distance. A smooth layer (the lacquer at GGX ~0.04-0.12, glass at
// 0.02, chrome) shows a street lamp, a headlamp or a lit shop window as a
// sharp highlight wherever its reflection finds one, however far the lamp
// is: a mirror's reflection keeps its source's luminance (it shrinks with
// distance, it does not dim), so the clusters' reach, which windows a
// light's irradiance, has no place in it. The lights are the frame's
// brightest toward the eye (the streak list, sky_streaks.wgsl, the wet
// street's), each a sphere of its luminaire's size: the GGX lobe widened by
// the source's angle (as the clusters' specular), the reflected radiance
// never over the lens's luminance (a mirror's bound), the lens's emission
// toward the point at least the share the eye sees of it (as the streaks).
// What the clusters' loop already gave inside a light's reach (its window
// squared) is left out, so nothing counts twice; a lamp with a shadow map
// is hidden where its map says the point does not see it (read 15 cm off
// the body, `AG_CAR_FAR_MAP_OFFSET`).
const AG_CAR_FAR_MAP_OFFSET: f32 = 0.15;
// A light's reflection under this share of the exposure's white skips its map.
const AG_CAR_FAR_MAP_MIN: f32 = 0.002;
// The specular anti-aliasing's variance for the far lights' lobe on paint, x AG_CAR_AA_VARIANCE.
const AG_CAR_FAR_AA: f32 = 4.0;
// A light whose half vector lies where the widened GGX lobe is under 1e-5 of
// its peak leaves the layer nothing a pixel shows: D falls as
// a2^2 / (1 - nh^2 (1 - a2))^2, so past 1 - nh^2 (1 - a2) > sqrt(1e5) a2 its
// shape, map and BRDF are not evaluated (most of the 32 lights, on most of a
// car: the term's cost was the loop over them all).
const AG_CAR_FAR_TAIL: f32 = 316.0;
fn ag_car_far_lights(world: vec3<f32>, n: vec3<f32>, v: vec3<f32>, f0: vec3<f32>, rough: f32) -> vec3<f32> {
    if (!SF_CAR_REFLECTIONS || !SKY_CINE_FULL || !sky_on(skyu.flags.x, SKY_LIGHTS) || !sky_on(skyu.flags.x, SKY_CINEMATIC) || skyu.lights.x == 0u || frame.copy_step.z > 0.5) {
        return vec3<f32>(0.0);
    }
    let g = skyu.lights.yzw;
    let base = g.x * g.y * g.z * (1u + SKY_CLUSTER_CAP);
    let count = min(sky_clusters[base], SKY_STREAK_CAP);
    var out = vec3<f32>(0.0);
    for (var k = 0u; k < count; k = k + 1u) {
        let li = sky_lights[sky_clusters[base + 1u + k]];
        let d = li.pos - world;
        let d2 = max(dot(d, d), 0.04);
        let dist = sqrt(d2);
        let l = d / dist;
        let nl = dot(n, l);
        let w = clamp(1.0 - pow(dist / max(li.range, 1e-3), 4.0), 0.0, 1.0);
        let rest = 1.0 - w * w;
        if (nl <= 0.0 || rest <= 0.0) {
            continue;
        }
        var src = SKY_CINE_SRC_STREET;
        if (li.kind == 1u) {
            src = SKY_CINE_SRC_HEADLAMP;
        } else if (li.kind >= 4u) {
            src = 0.5 * sqrt(max(li.size, 0.0));
        }
        src = max(src, 0.02);
        let rc2 = min(rough * rough + src / (2.0 * dist), 1.0);
        let a2 = rc2 * rc2;
        let nh = dot(n, normalize(l + v));
        if (1.0 - nh * nh * (1.0 - a2) > AG_CAR_FAR_TAIL * a2) {
            continue; // past the lobe's tail
        }
        let ca = dot(-l, li.dir);
        var shape = 1.0;
        if (li.kind == 0u) {
            shape = max(sky_cine_cobra(l, li.dir), SKY_CINE_STREAK_LENS);
        } else if (li.kind == 3u) {
            shape = max(smoothstep(li.cos_outer, li.cos_inner, ca), SKY_CINE_STREAK_LENS);
        } else if (li.kind == 1u) {
            let dep = asin(clamp(l.y, -1.0, 1.0)) * 57.29578;
            let hl = normalize(vec2<f32>(-l.x, -l.z) + vec2<f32>(1e-6, 0.0));
            let ha = normalize(vec2<f32>(li.dir.x, li.dir.z) + vec2<f32>(1e-6, 0.0));
            let hr = vec2<f32>(-ha.y, ha.x);
            let side = smoothstep(li.cos_outer, li.cos_inner, dot(hl, ha));
            shape = max(sky_cine_low_beam(dep, atan2(dot(hl, hr), dot(hl, ha)) * 57.29578) * side, SKY_CINE_STREAK_HEAD_LENS * smoothstep(-0.2, 0.2, dot(hl, ha)));
        } else if (li.kind >= 4u) {
            shape = smoothstep(li.cos_outer, li.cos_inner, ca);
            if (SF_CAR_LAMPS && li.kind == SF_REAR_LAMP_KIND) {
                shape = sf_rear_lamp_shape(-l, li.dir); // (`--car-lamps`) a car's rear lamp's field
            }
        }
        let e = li.intensity * shape * rest / d2 * nl;
        if (e <= 0.0) {
            continue;
        }
        let rc = sqrt(rc2);
        var bound = d2 / (LK_PI * src * src * nl);
        if (SF_CAR_LAMPS && li.kind == SF_REAR_LAMP_KIND) {
            // (`--car-lamps`) a rear lamp's mirror image no brighter than the lamp
            // is drawn (its luminance held under AG_CAR_LAMP_WHITE x the
            // exposure's white, `ag_car_prepare`): a chrome badge or a bumper
            // shows a lamp behind the eye as the lamp itself looks, not as a
            // spark over it
            bound = min(bound, AG_CAR_LAMP_WHITE / max(look.key.w * e, 1e-12));
        }
        var c = li.color * e * min(lk_ggx(n, v, l, f0, rc), vec3<f32>(bound));
        // the lamp's map only where its highlight shows (most lights leave a
        // sharp lacquer nothing: four compares a light were the term's cost),
        // looked up off the body: the car is in the map, and its own depth
        // there (a 10 cm texel) would speckle a sharp highlight
        if (LS_SHADER && li.pad >= 1.0 && lk_luma(c) * look.key.w > AG_CAR_FAR_MAP_MIN) {
            c = c * ls_light_visibility(u32(li.pad) - 1u, world + n * AG_CAR_FAR_MAP_OFFSET, n);
        }
        out = out + c;
    }
    return out;
}

// Road grime on a car (`--grime`; in ag_weather's place under the cars'
// materials, at its per-car levels and colours), where the road and the
// weather put it: the tyres' spray as a road film (asphalt fines, tyre and
// brake dust: a dark grey-brown, darker than the airborne dust) densest at
// the sill and fading up the lower body to about the wheels' tops, highest
// just behind each wheel, its top rising and falling slowly along the body
// and broken into run-off drips (3.5 cm across, only where a pixel resolves
// them; the mean otherwise); the grime round each arch; on a dusty car a
// fine film of dust on the roof, bonnet and boot (the surfaces facing up).
// The paint between stays as it is: no blotches, no noise across a panel.
const AG_CAR_ROAD_FILM: vec3<f32> = vec3<f32>(0.12, 0.11, 0.095);
fn ag_car_grime(s: ptr<function, Surface>, local: vec3<f32>, nb: vec3<f32>, piv: vec3<f32>, r: f32, arch: f32, dirt: f32, px: f32) {
    let y = local.y - (piv.y - r);
    let wave = st_noise(vec2<f32>(local.z / 0.45 + local.x * 3.1, dirt * 13.0)) - 0.5;
    let drip = mix(0.5, st_noise(vec2<f32>(local.z / 0.035 + local.x * 7.3, y / 0.5) + dirt * 17.0), wx_keep(0.035, px)) - 0.5;
    let spray = 1.0 - smoothstep(0.0, r * 2.2, abs(local.z - (piv.z - r * 1.3)));
    let top = r * (1.35 + 0.35 * wave + 0.7 * spray);
    let band = (1.0 - smoothstep(top * 0.3, top * (1.0 + 0.3 * drip), y)) * dirt;
    let ring = (1.0 - smoothstep(1.02, 1.5, arch)) * (0.35 + 0.65 * dirt);
    let film = dirt * 0.35 * smoothstep(0.55, 0.9, nb.y);
    (*s).albedo = mix((*s).albedo, AG_CAR_ROAD_FILM, clamp(band * WX_CAR_DUST_COVER, 0.0, 1.0));
    (*s).albedo = mix((*s).albedo, WX_CAR_DUST, clamp(film * 0.45, 0.0, 1.0));
    (*s).albedo = mix((*s).albedo, WX_CAR_GRIME, clamp(ring * WX_CAR_GRIME_COVER, 0.0, 1.0));
    // the film's haze on the lacquer follows the band without its drips: a
    // roughness changing from one pixel to the next sparkles under a lamp.
    let haze = (1.0 - smoothstep(top * 0.3, top, y)) * dirt;
    (*s).rough = clamp(mix((*s).rough, 0.85, max(haze, ring) * 0.8 + film * 0.5), 0.03, 1.0);
}

// ---- (`--car-finish`; SF_CAR_FINISH, look.rs `ShaderFeatures`: compiled out
// without it, `ag.misc.z` bit 2 at run time) The cars' finish as the street
// wears it. Paint: the base coat's metallic flakes glint one by one in the sun
// where a pixel resolves them (a 2 mm pixel holds a few hundred flakes, of
// which only those turned within the sun's quarter-degree of the half vector
// send it to the eye: a count of a mean of a few, a field of sparks over the
// highlight's smooth mean); further out the camera's reconstruction filter and
// its history average a pixel's few glints with its neighbours', so a frame
// shows their mean, the flake lobe, and nothing crawls; the lacquer's orange
// peel (a 4 mm ripple of half a degree, under a pixel at the cards'
// distances) as a slightly wider lobe; road grime where the road and the
// weather put it (`ag_cf_grime`: the tyres' spray, the tail's wake, run-off
// streaks from the glass's foot, settled dust); on old cars the lacquer failed in patches on the
// surfaces facing the sun (chalky, matte, no reflection). Glass: automotive
// float glass is green-grey (iron oxide; ~75 % visible transmission), so the
// cabin behind it, lit through it and seen through it, is dim and green-grey
// rather than the sky's blue; a dust film on the outer face (heaviest on the
// rear glass, light where the wipers sweep) scatters the light round it as a
// haze; the cabin's trim is matte. Rain: beads on the paint (a waxed
// lacquer's 90 degree contact angle) and drops on the glass, running into
// trails on the vertical panes, as lenses of their own normals where a pixel
// resolves them; where it does not, the lamps they mirror break into a speckle
// and the lacquer's lobe widens; wet tyres take a sheen. ----

// The finish's level at run time (agents.rs `AgParams::misc.z` bit 2).
fn ag_car_finish_on() -> bool {
    return SF_CAR_FINISH && SF_CAR_MATERIALS && (ag.misc.z & 4u) != 0u;
}

// Float glass's visible transmittance by colour (soda-lime with ~0.1 % Fe2O3,
// a 4-5 mm pane: ~75 % visible, green); the cabin as the eye sees it: lit
// through the glass (its colour once) and seen through it (its colour again
// over its luminance: the blend takes the scalar transmittance).
const AG_CF_GLASS_T: vec3<f32> = vec3<f32>(0.70, 0.80, 0.74);
const AG_CF_CABIN: vec3<f32> = vec3<f32>(0.633, 0.827, 0.707);
// Cabin plastics and cloth: dark and matte (the screen keeps its gloss).
const AG_CF_CABIN_ALBEDO_MAX: f32 = 0.12;
const AG_CF_CABIN_ROUGH: f32 = 0.6;
// The road film: dried asphalt fines, tyre and brake dust (a grey-brown
// lighter than the wet film), its cover at full dirt (densest in each
// wheel's wake, thinner between: its deposit is uneven, `AG_CF_FILM_PATCH_M`),
// and how high it reaches in wheel radii; the tail's wake (the surfaces facing
// back below the rear glass) at this share of the film's cover; the settled
// dust's cover on the surfaces facing up.
const AG_CF_ROAD_FILM: vec3<f32> = vec3<f32>(0.16, 0.145, 0.12);
const AG_CF_FILM_COVER: f32 = 0.85;
const AG_CF_FILM_REACH: f32 = 1.9;
const AG_CF_FILM_PATCH_M: f32 = 0.3;
const AG_CF_WAKE: f32 = 0.7;
const AG_CF_DUST_COVER: f32 = 0.45;
// Run-off: rain off the glass's foot carries the dust down the doors and the
// tail in streaks: one may start in each run of this length along the
// beltline (m), a streak this wide (m), reaching down at most this share of
// the beltline's height, at this share of the film's cover.
const AG_CF_RUN_M: f32 = 0.07;
const AG_CF_RUN_W: f32 = 0.012;
const AG_CF_RUN_REACH: f32 = 0.6;
const AG_CF_RUN_COVER: f32 = 0.55;
// A flake's glint is drawn one by one only where a pixel's footprint is
// under this (m: ~2.5 m from the 1080-line camera); half of it and under in full.
const AG_CF_GLINT_PX: f32 = 0.002;
// Failed lacquer: the share of old cars showing it, the patches' size (m),
// the chalked base under it.
const AG_CF_FAIL_SHARE: f32 = 0.35;
const AG_CF_FAIL_PATCH_M: f32 = 0.22;
const AG_CF_CHALK: vec3<f32> = vec3<f32>(0.62, 0.6, 0.57);
// Metallic flake: flakes a square metre of the base coat (~30 um flakes at
// a third of its area) times a quarter of the sun's solid angle (6.8e-5 sr):
// the expected count of flakes glinting a square metre of footprint per unit
// of the flakes' normal distribution; that distribution a Beckmann (Gaussian)
// one of slope 0.2 (flakes lie within ~11 degrees of the panel: no long tail,
// so the sparks keep to the highlight).
const AG_CF_FLAKE_RATE: f32 = 5100.0;
const AG_CF_FLAKE_SLOPE: f32 = 0.2;
// Orange peel (a ripple of ~4 mm and half a degree: under a pixel at the
// cards' distances): the lacquer's lobe widened by it.
const AG_CF_PEEL_ROUGH: f32 = 0.015;
// The glass's dust film: its scattering albedo at full dirt, its phase
// function's asymmetry (fine dust scatters forward).
const AG_CF_HAZE: f32 = 0.05;
const AG_CF_HAZE_G: f32 = 0.5;
// Rain: beads on the lacquer and drops on the glass (cells, m), the share of
// cells holding one, a drop's radius in cells; wet rubber's roughness.
const AG_CF_BEAD_M: f32 = 0.007;
const AG_CF_DROP_M: f32 = 0.011;
const AG_CF_BEAD_COVER: f32 = 0.65;
const AG_CF_DROP_COVER: f32 = 0.5;
const AG_CF_WET_RUBBER_ROUGH: f32 = 0.32;
// Calipers: the shares of the fleet with bare cast iron, black, red (cumulative;
// the rest keep the body's own painted caliper) and their colours (iron a dull
// grey-brown under brake dust).
const AG_CF_CALIPER_SHARE: vec3<f32> = vec3<f32>(0.65, 0.85, 0.93);
const AG_CF_CALIPER_IRON: vec3<f32> = vec3<f32>(0.11, 0.1, 0.09);
const AG_CF_CALIPER_BLACK: vec3<f32> = vec3<f32>(0.02, 0.02, 0.021);
const AG_CF_CALIPER_RED: vec3<f32> = vec3<f32>(0.3, 0.02, 0.02);

// How much of a glint a pixel of footprint `px` (m) shows one by one (`AG_CF_GLINT_PX`): 1 where it
// resolves the flakes' sparks, 0 where the camera's filter and history show their mean.
fn ag_cf_resolved(px: f32) -> f32 {
    return 1.0 - smoothstep(AG_CF_GLINT_PX * 0.5, AG_CF_GLINT_PX, px);
}

// A pattern of `scale` (m) drawn where it spans three pixels and more, its
// mean where it spans under a pixel and a half (stricter than `wx_keep`: a
// pattern of two or three pixels reads as grain on a car's smooth paint).
fn ag_cf_keep(scale: f32, px: f32) -> f32 {
    return 1.0 - smoothstep(scale * 0.16, scale * 0.33, px);
}

// (`--car-finish`, in `ag_car_grime`'s place) Road grime where the road and the
// weather put it on a car a few weeks from a wash (`dirt` its level, `md` its
// body): the tyres' spray as a grey-brown film densest at the sill, up the
// lower body to about the wheels' tops, highest in each wheel's wake (the
// rear of its arch and the panel behind it) and uneven along the body, its
// upper edge run down in drips; the tail's wake (a hatch, a tailgate, a van's
// doors: the surfaces facing back below the rear glass) dusted by the
// low-pressure air behind the car; run-off streaks from the glass's foot down
// the doors and the tail (each a narrow streak of the film fading downward,
// drawn where a pixel resolves it, its mean cover where not); the dust
// settled on the roof, bonnet and boot, uneven over half a metre; the grime
// round each arch. Every pattern is under the pixel's resolve (`ag_cf_keep`):
// no grain across a panel.
fn ag_cf_grime(s: ptr<function, Surface>, local: vec3<f32>, nb: vec3<f32>, piv: vec3<f32>, r: f32, arch: f32, dirt: f32, px: f32, md: AgModel) {
    let y = local.y - (piv.y - r);
    // the beltline: the glass's foot (the occluder boxes' `belt`, eo-agents `contact_words`), else above the body's middle
    var belt = md.center.y * 1.25;
    if (md.pad.z != 0u) {
        belt = unpack2x16snorm(md.pad.z).x * 32.767;
    }
    let wave = st_noise(vec2<f32>(local.z / 0.45 + local.x * 3.1, dirt * 13.0)) - 0.5;
    let drip = mix(0.5, st_noise(vec2<f32>(local.z / 0.035 + local.x * 7.3, y / 0.5) + dirt * 17.0), ag_cf_keep(0.035, px)) - 0.5;
    let spray = 1.0 - smoothstep(0.0, r * 2.2, abs(local.z - (piv.z - r * 1.3)));
    let lay = st_noise(vec2<f32>(local.z, local.y + local.x * 0.7) / AG_CF_FILM_PATCH_M + vec2<f32>(dirt * 5.3, 1.7));
    let top = r * AG_CF_FILM_REACH * (1.0 + 0.25 * wave + 0.45 * spray);
    // the spray: dense to about half its height, thinning above
    let band = (1.0 - smoothstep(top * 0.45, top * (1.1 + 0.3 * drip), y)) * min(dirt * 1.6, 1.0) * mix(0.65, 1.0, spray) * (0.75 + 0.5 * lay);
    // the wake on the surfaces facing back, below the rear glass, heavier toward the bumper
    let back = smoothstep(0.35, 0.75, -nb.z) * (1.0 - smoothstep(belt * 0.9, belt * 1.02, local.y));
    let wake = back * AG_CF_WAKE * dirt * (0.7 + 0.3 * (1.0 - smoothstep(0.0, belt, local.y))) * (0.8 + 0.4 * lay);
    // run-off streaks on the panels standing up (the side along z, the tail along x)
    var run = 0.0;
    let upright = 1.0 - smoothstep(0.35, 0.6, abs(nb.y));
    if (upright > 0.0 && local.y < belt) {
        let side = abs(nb.x) > abs(nb.z);
        let along = select(local.x, local.z, side);
        let ci = floor(along / AG_CF_RUN_M);
        let salt = select(2.3, 7.9, side) + select(0.0, 3.1, select(nb.z, nb.x, side) < 0.0);
        let hr = st_hash(vec2<f32>(ci + floor(dirt * 61.0), salt));
        let hx = st_hash(vec2<f32>(ci * 1.7 + salt, 4.4));
        let reach = belt * AG_CF_RUN_REACH * (0.3 + 0.7 * hr);
        // down from the beltline, fading toward its foot
        let fall = smoothstep(belt - reach, belt - reach * 0.4, local.y) * (1.0 - smoothstep(belt - 0.015, belt, local.y));
        let w = AG_CF_RUN_W * (0.6 + 0.8 * hx);
        let dx = abs((fract(along / AG_CF_RUN_M) - 0.2 - 0.6 * hx) * AG_CF_RUN_M);
        let line = 1.0 - smoothstep(w * 0.5, w * 0.5 + px, dx);
        let starts = select(0.0, 1.0, hr > 0.5);
        // the mean cover of the streaks where a pixel does not resolve them
        let mean = 0.5 * (w / AG_CF_RUN_M) * 0.7;
        run = mix(mean, line * starts, ag_cf_keep(w * 3.0, px)) * fall * upright * dirt * AG_CF_RUN_COVER;
    }
    let ring = (1.0 - smoothstep(1.02, 1.5, arch)) * (0.35 + 0.65 * dirt);
    // the dust settled on the surfaces facing up, uneven over half a metre
    let lie = st_noise(local.xz / 0.5 + vec2<f32>(dirt * 7.0, 3.1));
    let film = dirt * AG_CF_DUST_COVER * smoothstep(0.5, 0.85, nb.y) * (0.6 + 0.8 * lie);
    let cover = clamp(max(max(band, wake), run) * AG_CF_FILM_COVER, 0.0, 1.0);
    (*s).albedo = mix((*s).albedo, AG_CF_ROAD_FILM, cover);
    (*s).albedo = mix((*s).albedo, WX_CAR_DUST, clamp(film, 0.0, 1.0));
    (*s).albedo = mix((*s).albedo, WX_CAR_GRIME, clamp(ring * WX_CAR_GRIME_COVER, 0.0, 1.0));
    // the film's haze on the lacquer follows the cover without its drips and streaks
    // (a roughness changing from one pixel to the next sparkles under a lamp)
    let haze = max((1.0 - smoothstep(top * 0.3, top, y)) * dirt * mix(0.65, 1.0, spray), wake);
    (*s).rough = clamp(mix((*s).rough, 0.85, max(haze, ring) * 0.8 + film * 0.5), 0.03, 1.0);
}

// A drop's lens at a point of the body (its frame `local`, normal `nb`):
// the tilt it gives the normal, in the body's frame (0 off a drop or where a
// pixel `px` does not resolve the cells). Cells on the plane the normal faces
// most, stretched down the vertical panes into running trails.
fn ag_cf_drop(local: vec3<f32>, nb: vec3<f32>, px: f32, cell: f32, cover: f32) -> vec3<f32> {
    let keep = ag_cf_keep(cell, px);
    if (keep <= 0.0) {
        return vec3<f32>(0.0);
    }
    let an = abs(nb);
    var uv = local.xz;
    var tu = vec3<f32>(1.0, 0.0, 0.0);
    var tv = vec3<f32>(0.0, 0.0, 1.0);
    if (an.x > an.y && an.x >= an.z) {
        uv = local.zy;
        tu = vec3<f32>(0.0, 0.0, 1.0);
        tv = vec3<f32>(0.0, 1.0, 0.0);
    } else if (an.z > an.y) {
        uv = local.xy;
        tu = vec3<f32>(1.0, 0.0, 0.0);
        tv = vec3<f32>(0.0, 1.0, 0.0);
    }
    let vert = 1.0 - smoothstep(0.5, 0.8, an.y);
    let q = uv / cell * vec2<f32>(1.0, mix(1.0, 0.35, vert));
    let ci = floor(q);
    if (st_hash(ci + vec2<f32>(7.1, 3.3)) > cover) {
        return vec3<f32>(0.0);
    }
    let c = (vec2<f32>(st_hash(ci + vec2<f32>(1.3, 9.2)), st_hash(ci + vec2<f32>(2.9, 4.4))) - 0.5) * 0.36;
    let rr = 0.18 + 0.2 * st_hash(ci + vec2<f32>(5.7, 0.6));
    let o = (fract(q) - 0.5 - c) * vec2<f32>(1.0, mix(1.0, 2.2, vert));
    let t = length(o) / rr;
    if (t >= 1.0) {
        return vec3<f32>(0.0);
    }
    // a spherical cap: the slope rises to the contact angle at its rim
    let sn = t * 0.85;
    let tilt = (o / max(length(o), 1e-5)) * (sn / sqrt(1.0 - sn * sn)) * keep;
    return tu * tilt.x + tv * tilt.y;
}

// The paint's finish (after the paint's own terms): the failed lacquer on old
// cars, orange peel's lobe, rain beads (`rain` the wet share while it rains,
// `drop` a bead's tilt of the normal where a pixel resolves it); `flake` the
// base coat's metallic share for the sun's glints (`ag_car_finish`).
fn ag_cf_paint(g: ptr<function, AgSurf>, k: ptr<function, AgCar>, local: vec3<f32>, nb: vec3<f32>, h: f32, age: f32, rain: f32, drop: vec3<f32>, flake: f32, d: AgCarD, livery: bool) {
    // the lacquer failed in patches on the surfaces facing the sun (old cars only)
    let fail_car = select(0.0, 1.0, st_hash(vec2<f32>(h * 41.0, 2.7)) < AG_CF_FAIL_SHARE) * smoothstep(0.55, 0.9, age);
    if (fail_car > 0.0 && !livery) {
        let blot = st_noise(local.xz / AG_CF_FAIL_PATCH_M + vec2<f32>(h * 13.0, 1.9));
        let failed = smoothstep(0.35, 0.8, nb.y) * fail_car * smoothstep(0.45, 0.6, blot);
        (*g).s.albedo = mix((*g).s.albedo, (*g).s.albedo * 0.55 + AG_CF_CHALK * dot((*g).s.albedo, vec3<f32>(0.3333)) * 0.6 + vec3<f32>(0.03), failed);
        (*g).s.rough = mix((*g).s.rough, 0.7, failed);
        (*g).s.metal = (*g).s.metal * (1.0 - failed);
        sky_coat = sky_coat * (1.0 - 0.9 * failed);
    }
    // orange peel (a 4 mm ripple: under a pixel at the cards' distances) as the lacquer's wider lobe
    sky_coat_rough = sky_coat_rough + AG_CF_PEEL_ROUGH;
    if (rain > 0.0) {
        // beads: their lenses where a pixel resolves them; a wider lobe and the lamps' speckle where not
        let unres = 1.0 - ag_cf_keep(AG_CF_BEAD_M, d.px);
        sky_coat_rough = sky_coat_rough + 0.05 * unres * rain;
        (*k).far_rough = mix((*k).far_rough, max((*k).far_rough, 0.12), unres * rain);
        (*k).fx.w = unres * rain;
        (*g).n = normalize((*g).n + drop - (*g).n * dot((*g).n, drop));
    }
    (*k).fx.x = flake;
}

// The glass's finish: its dust film (`k.fx.z`, the haze `ag_car_finish` lights)
// and, in rain, its drops (`drop` as `ag_cf_paint`'s); `md` the body.
fn ag_cf_glass(g: ptr<function, AgSurf>, k: ptr<function, AgCar>, local: vec3<f32>, nb: vec3<f32>, md: AgModel, age: f32, dirt: f32, rain: f32, drop: vec3<f32>, d: AgCarD) {
    // the film: heaviest on the rear glass (the wake's spray), lightest on a
    // windscreen the wipers keep, thicker toward each pane's foot, dried spots in it
    let rear = smoothstep(0.2, 0.6, -nb.z);
    let screen = smoothstep(0.3, 0.7, nb.z);
    let foot = 1.0 - smoothstep(md.center.y, md.center.y + 0.5 * md.radius, local.y);
    let spots = mix(0.5, st_noise(local.xy / 0.012 + local.zz * 37.0), ag_cf_keep(0.012, d.px));
    var haze = AG_CF_HAZE * dirt * (0.35 + 0.65 * age) * (1.0 + 1.5 * rear - 0.6 * screen) * (0.6 + 0.6 * foot) * (0.75 + 0.5 * spots);
    // the glass's outer face reflects the street as its base does: by the sky
    // the ambient volume sees along it (look_light's coat takes the occlusion
    // only with a class set; the glass's own, so its strength is unchanged)
    sky_coat_env = look.env_class.y;
    if (rain > 0.0) {
        let unres = 1.0 - ag_cf_keep(AG_CF_DROP_M, d.px);
        (*k).far_rough = mix((*k).far_rough, 0.15, unres * rain);
        (*k).fx.w = unres * rain;
        (*g).n = normalize((*g).n + drop - (*g).n * dot((*g).n, drop));
        haze = haze * (1.0 - 0.6 * rain); // the rain washes the film off
    }
    (*k).fx.z = haze;
}

// A count of mean `lam` from a uniform `u`: under a mean of one, one or none
// (a Poisson count's first two terms, its mean kept); over it, the mean and
// its spread (sqrt lam) as a uniform of unit variance.
fn ag_cf_count(lam: f32, u: f32) -> f32 {
    return select(select(0.0, 1.0, u < lam), max(lam + sqrt(lam) * (u - 0.5) * 3.4641, 0.0), lam >= 1.0);
}

// A cell of the body no wider than the pixel (its footprint snapped down to an
// octave, so a car's sparks hold as it nears or recedes within it).
fn ag_cf_cell(px: f32) -> f32 {
    return exp2(floor(log2(max(px, 1e-4))));
}

// A uniform hash of a point of the body in its `ag_cf_cell`.
fn ag_cf_cell_hash(local: vec3<f32>, px: f32, salt: f32) -> f32 {
    let cell = ag_cf_cell(px);
    let ci = floor(local / cell);
    return st_hash(vec2<f32>(ci.x + ci.y * 37.13 + salt, ci.z - ci.y * 17.31 + salt * 0.37));
}

// After look_light, the finish's own light: the cabin's colour seen through the
// glass, the glass's haze, the flakes' glints (the sun's share of the base
// coat's lobe redrawn as its count of glinting flakes over their mean).
fn ag_cf_light(c: vec4<f32>, k: AgCar, world: vec3<f32>, local: vec3<f32>, g: AgSurf, px: f32) -> vec4<f32> {
    var o = c;
    if (k.fx.y > 0.0) {
        let ff = look.fog_color.xyz * ag_car_fog(world, g.m);
        o = vec4<f32>((o.rgb - ff) * AG_CF_CABIN + ff, o.a);
    }
    let l = look.key_dir.xyz;
    if (k.fx.z > 0.0) {
        // the dust film: single scattering of the sky round it and the sun (its visibility as look_light found it)
        let hemi = mix(look.hemi_ground.xyz, look.hemi_sky.xyz, g.n.y * 0.5 + 0.5) + look.ambient.xyz;
        let ct = dot(-l, g.v);
        let gg = AG_CF_HAZE_G;
        let hb = max(1.0 + gg * gg - 2.0 * gg * ct, 1e-4);
        let hg = (1.0 - gg * gg) / (hb * sqrt(hb));
        let sun = look.key.xyz * lk_key_vis * max(dot(g.n, l), 0.0) * hg;
        let veil = k.fx.z * (hemi * g.s.ao + sun) / LK_PI * (1.0 - ag_car_fog(world, g.m));
        o = vec4<f32>(o.rgb + veil, o.a); // (the glass's blend keeps it whole: `ag_car_finish` divides by the alpha)
    }
    if (k.fx.x > 0.0 && lk_key_vis > 0.0) {
        let nl = dot(g.s.n, l);
        if (nl > 0.0) {
            let hv = normalize(l + g.v);
            let nh = max(dot(g.s.n, hv), 1e-3);
            // the flakes' normals (Beckmann, AG_CF_FLAKE_SLOPE) about the half vector
            let c2 = nh * nh;
            let m2 = AG_CF_FLAKE_SLOPE * AG_CF_FLAKE_SLOPE;
            let dgx = exp((c2 - 1.0) / (c2 * m2)) / (LK_PI * m2 * c2 * c2);
            let cell = ag_cf_cell(px);
            let lam = AG_CF_FLAKE_RATE * cell * cell * dgx;
            let resolved = ag_cf_resolved(px);
            if (lam > 1e-3 && resolved > 0.0) {
                // the flakes' lobe from the sun (that D, Kelemen's visibility, the base coat's F0)
                let f0 = mix(vec3<f32>(0.04), g.s.albedo, g.s.metal);
                let lh = max(dot(l, hv), 0.05);
                let mean = look.key.xyz * (lk_key_vis * nl * dgx / (4.0 * lh * lh) * k.fx.x) * f0;
                let n = ag_cf_count(lam, ag_cf_cell_hash(local, px, 3.0));
                let fcc = 0.04 + 0.96 * pow(1.0 - max(dot(g.n, g.v), 0.0), 5.0);
                let under = (1.0 - sky_coat * fcc) * (1.0 - ag_car_fog(world, g.m));
                o = vec4<f32>(max(o.rgb + mean * (n / lam - 1.0) * under * resolved, vec3<f32>(0.0)), o.a);
            }
        }
    }
    return o;
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
