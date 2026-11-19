// spray and spindrift off the breaking crests in a strong wind
// (water.rs draws it in the water pass, after the sea, only while the wind
// over the water is over about 9 m/s). No state and no buffers: each of the
// instanced quads is a slot that, once per lifetime, picks a point near the
// camera (ahead of it), keeps it only where the simulation's whitecap foam is
// fresh there, and throws a puff of droplets up from the crest and downwind,
// growing and thinning as it goes. Lit by the sky's light and the sun
// scattered forward through it, never brighter than the light it scatters.

struct SprayV {
    @builtin(position) clip: vec4<f32>,
    // xy quad corner -1..1, z age 0..1, w opacity
    @location(0) q: vec4<f32>,
    @location(1) world: vec3<f32>,
};

const WSP_LIFE_S: f32 = 1.7;
const WSP_RADIUS_M: f32 = 80.0;

fn wsp_hash(x: u32) -> u32 {
    var h = x * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    return (h >> 22u) ^ h;
}

fn wsp_unit(x: u32) -> f32 {
    return f32(wsp_hash(x) >> 8u) / 16777216.0;
}

// The place's fresh foam at a point: cascade 1 and 2's, at their finest level.
fn wsp_foam(p: vec2<f32>) -> f32 {
    let a = textureSampleLevel(w_deriv, w_repeat, w_casc_uv(p, 1u), 1u, 0.0);
    let b = textureSampleLevel(w_deriv, w_repeat, w_casc_uv(p, 2u), 2u, 0.0);
    // (iteration 2, exposure) the bay's foam as the surface draws it there (`wg_foam_k`)
    return max(mix(a.w * wg_foam_k(), a.z, wg_ocean(p.x)), mix(b.w * wg_foam_k(), b.z, wg_ocean(p.x)) * 0.8);
}

@vertex
fn vs_wspray(@builtin(vertex_index) vi: u32) -> SprayV {
    var o: SprayV;
    o.clip = vec4<f32>(0.0, 0.0, -1.0, 1.0);
    o.q = vec4<f32>(0.0);
    o.world = vec3<f32>(0.0);
    let slot = vi / 6u;
    let corner = vi % 6u;
    let cx = select(-1.0, 1.0, corner == 1u || corner == 4u || corner == 5u);
    let cy = select(-1.0, 1.0, corner == 2u || corner == 3u || corner == 5u);
    let t = wu.cam.w;
    let phase = wsp_unit(slot * 3u + 11u);
    let life = WSP_LIFE_S * (0.75 + 0.5 * wsp_unit(slot * 3u + 12u));
    let k = (t / life) + phase;
    let cycle = u32(floor(k));
    let age = fract(k);
    let seed = slot * 7919u + cycle * 104729u;
    // where it starts: a disc ahead of the camera
    let fw = normalize(vec2<f32>(wu.fwd.x, wu.fwd.z) + vec2<f32>(1e-4, 0.0));
    let r = WSP_RADIUS_M * sqrt(wsp_unit(seed + 1u));
    let a = 6.2831853 * wsp_unit(seed + 2u);
    let p0 = wu.cam.xz + fw * (WSP_RADIUS_M * 0.55) + r * vec2<f32>(cos(a), sin(a));
    w_expo_set(p0); // (iteration 2, exposure) this place's wind, waves and foam, as the surface's
    let u = wv_u10(p0.x);
    let amount = smoothstep(9.0, 17.0, u);
    if (amount <= 0.0) {
        return o;
    }
    // only on fresh foam (a crest breaking now), over open water (the surf strip
    // throws its own spray at the beaches)
    let f = w_field_at(p0);
    if (f.x < 40.0 || (f.y > 0.1 && f.x < 450.0)) {
        return o;
    }
    let fresh = smoothstep(0.4, 0.8, wsp_foam(p0));
    if (fresh <= 0.0) {
        return o;
    }
    // the crest's height there (cascades 0-1, each place's share)
    let amp = wg_cascade_amp(p0.x);
    let h = textureSampleLevel(w_disp, w_repeat, w_casc_uv(p0, 0u), 0u, 0.0).y * amp.x + textureSampleLevel(w_disp, w_repeat, w_casc_uv(p0, 1u), 1u, 0.0).y * amp.y;
    let wd = normalize(select(vec2<f32>(0.985, 0.174), skyu.wind.xy, skyu.flags.x != 0u && dot(skyu.wind.xy, skyu.wind.xy) > 1.0));
    let storm = wu.wv_misc.x;
    // thrown up off the crest and carried downwind (spindrift in a gale runs low and long)
    let rise = (0.25 + 1.6 * age * (1.0 - 0.45 * age)) * mix(1.0, 1.6, storm);
    let run = u * 0.22 * age * life;
    let centre = vec3<f32>(p0.x + wd.x * run, wu.grid.z + h + rise, p0.y + wd.y * run);
    let size = (0.6 + 2.2 * age) * mix(1.0, 1.5, storm);
    let to_cam = wu.cam.xyz - centre;
    let dist = length(to_cam);
    let fade = smoothstep(WSP_RADIUS_M * 1.25, WSP_RADIUS_M * 0.8, dist) * smoothstep(1.5, 4.0, dist);
    let alpha = (1.0 - age) * (1.0 - age) * smoothstep(0.0, 0.08, age) * amount * fresh * fade * 0.8;
    if (alpha <= 0.002) {
        return o;
    }
    // camera-facing, stretched a little along the wind
    let fwd3 = to_cam / max(dist, 1e-3);
    let right = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), fwd3) + vec3<f32>(1e-4, 0.0, 0.0));
    let up = cross(fwd3, right);
    let world = centre + right * (cx * size * 1.35) + up * (cy * size * 0.8);
    o.clip = wu.view_proj * vec4<f32>(world, 1.0);
    o.clip.z = max(o.clip.z, 1e-9);
    o.q = vec4<f32>(cx, cy, age, alpha);
    o.world = world;
    return o;
}

@fragment
fn fs_wspray(i: SprayV) -> @location(0) vec4<f32> {
    let d2 = dot(i.q.xy, i.q.xy);
    if (d2 >= 1.0 || i.q.w <= 0.0) {
        discard;
    }
    // a soft, torn puff (two scales of noise eat its edge as it ages)
    let n = mx_noise(vec3<f32>(i.world * 1.3)) * 0.6 + mx_noise(vec3<f32>(i.world * 3.7 + vec3<f32>(5.1, 2.3, 0.0))) * 0.4;
    let body = clamp((1.0 - d2) * (1.0 + 0.9 * n) - 0.25 * i.q.z, 0.0, 1.0);
    let a = body * i.q.w;
    if (a <= 0.003) {
        discard;
    }
    // lit by the sky dome (the horizon's light) and the sun scattered forward through the droplets
    // (under cloud the key is the rig's, not a sun: it fades with the overcast, as the sea's foam does)
    let v = normalize(i.world - wu.cam.xyz);
    let fwd_sc = pow(max(dot(v, normalize(look.sun.xyz)), 0.0), 6.0);
    let sky_light = max(look.horizon.xyz, look.ambient.xyz * 0.35);
    let sun = look.key.xyz * (0.10 + 0.9 * fwd_sc) * max(look.sun.y + 0.05, 0.0) * (1.0 - 0.9 * clamp(look.wx.x, 0.0, 1.0));
    let col = min(sky_light * 0.9 + sun, vec3<f32>(lk_luma(sky_light) * 2.5));
    return vec4<f32>(col * a, a);
}
