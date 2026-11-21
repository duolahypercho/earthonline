// the fog lit by the local lights (lighting, `--lit-fog`; lit_fog.rs), appended
// to look_common.wgsl (which carries sky_common.wgsl), eo-sky lit_fog::wgsl's
// constants and lit_fog.rs's. A froxel volume aligned with the light clusters'
// view frustum (the clusters' screen tiles and their log-depth range,
// `skyu.cluster` near .. far, unjittered camera), two passes:
//
// 1. `lf_inject`, one invocation a froxel: the fog's extinction at a point
//    jittered through the froxel's depth (the volume's own fog: the marine
//    layer, the weather's veil, a continuous low deck, the haze, rain), the
//    light the clustered lamps and headlamps send toward the eye from it (each
//    light read the way `sky_local_lights` reads it: its cluster's list, its
//    shape, its range window; `lf_light_visibility` is where a light's shadow
//    multiplies it), blended with the last frame's froxel at the same world
//    point (a low history weight, lower still as the eye moves and the view of
//    the point turns; a history far from the new value is held to within
//    LF_CLAMP of it, so a moving headlamp's beam does not smear).
//    Out: rgb the radiance a unit-albedo droplet there sends to the eye, a the
//    extinction (1/m).
// 2. `lf_integrate`, one invocation a froxel column: front to back, the
//    in-scatter and transmittance from the eye to each slice's far face.
//
// The resolve (sky_resolve.wgsl `rs_lit_fog`) reads (2) at each pixel's depth
// and adds it to the scene: the volume (`vol_main`) already carries this fog's
// extinction, the sun and the sky's light in it.

struct LfLight {
    pos: vec3<f32>,
    intensity: f32,
    color: vec3<f32>,
    range: f32,
    dir: vec3<f32>,
    cos_outer: f32,
    cos_inner: f32,
    kind: u32,
    size: f32,
    pad: f32,
};

@group(0) @binding(0) var<uniform> look: Look;
@group(0) @binding(1) var<uniform> skyu: Sky;
@group(0) @binding(2) var<storage, read> lf_lights: array<LfLight>;
@group(0) @binding(3) var<storage, read> lf_clusters: array<u32>;
// the cloud sheet (the marine layer's top and body), the cloud noise (its banks), the deck's base noise
@group(0) @binding(4) var lf_sheet: texture_2d<f32>;
@group(0) @binding(5) var lf_noise: texture_3d<f32>;
@group(0) @binding(6) var lf_deck: texture_3d<f32>;
@group(0) @binding(7) var lf_rep: sampler;
// the last frame's froxels (the other parity), a clamped linear sampler
@group(0) @binding(8) var lf_hist: texture_3d<f32>;
@group(0) @binding(9) var lf_lin: sampler;
@group(0) @binding(10) var lf_out: texture_storage_3d<rgba16float, write>;
// [0] the frame (`skyu.flags.y`) the volume was last integrated in
@group(0) @binding(11) var<storage, read_write> lf_state: array<u32>;
// the integration's input (this frame's froxels) and output
@group(0) @binding(12) var lf_src: texture_3d<f32>;
@group(0) @binding(13) var lf_integ: texture_storage_3d<rgba16float, write>;

/// The view depth (m) of slice coordinate `s` (0 .. slices): log-spaced from
/// the clusters' near to their far, as the clusters' slices are.
fn lf_depth(s: f32, slices: f32) -> f32 {
    return skyu.cluster.x * pow(skyu.cluster.y / skyu.cluster.x, s / slices);
}

/// The view ray through a froxel column's centre (camera basis, z = 1).
fn lf_view(ij: vec2<u32>, dims: vec3<u32>) -> vec3<f32> {
    let uv = (vec2<f32>(ij) + 0.5) / vec2<f32>(dims.xy);
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    return vec3<f32>(ndc.x * skyu.cluster.z * skyu.cluster.w, ndc.y * skyu.cluster.z, 1.0);
}

fn lf_world(v: vec3<f32>, z: f32) -> vec3<f32> {
    return skyu.eye.xyz + (skyu.cam_right.xyz * v.x + skyu.cam_up.xyz * v.y + skyu.cam_fwd.xyz) * z;
}

/// The fog's extinction (1/m) at a world point: the parts of the volume
/// (sky_volume.wgsl `vol_main`) near the ground, read the way it reads them.
fn lf_density(p: vec3<f32>) -> f32 {
    let bits = skyu.flags.x;
    var s = 0.0;
    // (B) the marine layer: its rolling top, its banks and thinner gaps.
    if (sky_on(bits, SKY_FOG) && (skyu.cloud3.x > 0.0 || skyu.cloud3.w > 0.0)) {
        let sh = textureSampleLevel(lf_sheet, lf_rep, (p.xz + skyu.cloud4.xy) / SKY_MARINE_TILE, 0.0);
        var top = skyu.cloud3.y + skyu.cloud3.z * (sh.b * 2.0 - 1.0);
        let off = smoothstep(-13000.0, -16000.0, p.x);
        if (skyu.cloud3.x <= 0.0) {
            top = top * 0.55;
        }
        let sig0 = max(skyu.cloud3.x, 0.35 * skyu.cloud3.w * off);
        let fq = textureSampleLevel(lf_noise, lf_rep, (p + vec3<f32>(skyu.cloud4.x, 0.0, skyu.cloud4.y)) / 600.0, 1.0);
        s = s + sig0 * smoothstep(top, top - mix(70.0, 180.0, off), p.y) * (0.55 + 0.9 * sh.a) * (0.35 + 1.3 * fq.r);
    }
    // the weather's veil (`vol_aerial`: the range-fog law, 95 % at `atmo3.w`) under the low deck's top.
    if (skyu.atmo2.x > 0.5 && skyu.atmo3.x > 0.5 && p.y <= skyu.cloud0.y) {
        s = s + 3.0 / max(skyu.atmo3.w, 1.0);
    }
    // (C) a continuous low deck the street stands in (fog's stratus from 60 m): sky_clouds.wgsl
    // `cl_shape`'s deck part, its base lowered and raised by the base noise, its rolls at their mean.
    let st = clamp((skyu.cloud0.z - 0.75) / 0.2, 0.0, 1.0);
    if (sky_on(bits, SKY_CLOUDS) && st > 0.0) {
        let h = (p.y - skyu.cloud0.x) / max(skyu.cloud0.y - skyu.cloud0.x, 1.0);
        if (h > 0.0 && h < 1.0) {
            let wxz = p.xz + skyu.cloud2.xy;
            let n = textureSampleLevel(lf_deck, lf_rep, vec3<f32>(wxz.x, p.y * mix(1.0, 0.6, st), wxz.y) / (LF_CL_BASE_TILE * (1.0 + st)), 0.0);
            let rainy = smoothstep(2.0, 8.0, skyu.wet.z);
            let base_lo = mix(0.06, 0.12, rainy) * smoothstep(0.35, 0.85, n.b);
            let crown = 0.75 + 0.25 * smoothstep(0.3, 0.8, 0.6 * n.g + 0.4 * n.b);
            let prof = smoothstep(base_lo, base_lo + 0.1, h) * (1.0 - smoothstep(0.7, 1.0, h / crown));
            let shape = mix(n.r, 0.625 * n.g + 0.25 * n.b + 0.125 * n.a, 0.3);
            let d = prof * (mix(0.62, 0.4, rainy) + mix(0.38, 0.6, rainy) * smoothstep(0.2, 0.8, shape));
            s = s + skyu.cloud1.x * skyu.cloud0.w * LF_CL_DENSITY * d * st;
        }
    }
    // the haze the sun's shafts are scattered in (thinning with height).
    if (sky_on(bits, SKY_SHAFTS)) {
        s = s + skyu.cloud4.z * exp(-max(p.y, 0.0) / max(skyu.cloud4.w, 1.0));
    }
    // (E) rain under a raining deck (Marshall-Palmer, its cells at their mean).
    let rate = skyu.wet.z;
    if (sky_on(bits, SKY_RAIN) && rate > 0.5 && skyu.cloud0.z > 0.75 && p.y < skyu.cloud0.x) {
        s = s + 3.912 / mix(6000.0, 1800.0, clamp(rate / 20.0, 0.0, 1.0)) * LF_RAIN_MEAN;
    }
    return s;
}

/// Henyey-Greenstein (eo-sky lit_fog::hg).
fn lf_hg(c: f32, g: f32) -> f32 {
    let x = max(1.0 + g * g - 2.0 * g * c, 1e-5);
    return (1.0 - g * g) / (12.566371 * x * sqrt(x));
}

/// The lobe after `tau` of fog on the light's whole path (eo-sky
/// lit_fog::glow_phase): the droplets' HG(g) unscattered, HG(g^2) once, HG(g^3) more.
fn lf_phase(c: f32, tau: f32) -> f32 {
    let e = exp(-tau);
    let g2 = LF_G * LF_G;
    return e * (lf_hg(c, LF_G) + tau * lf_hg(c, g2)) + max(1.0 - e * (1.0 + tau), 0.0) * lf_hg(c, g2 * LF_G);
}

/// (lighting, `--light-shadows`, `--headlight-shadows`; light_shadow.rs) A
/// light's visibility from a point of the fog, 0 (shadowed) .. 1: its record's
/// spare word carries its shadow map's tile + 1 (0 with the options off, -1
/// for a light without a map: visible). `index` is the light's frame-list slot.
fn lf_light_visibility(index: u32, li: LfLight, p: vec3<f32>) -> f32 {
    if (li.pad >= 1.0) {
        return ls_light_visibility(u32(li.pad) - 1u, p, vec3<f32>(0.0));
    }
    return 1.0;
}

/// One light's radiance toward the eye from a fog point `p` (a unit-albedo
/// droplet), `to_eye` the unit direction to the eye, `t_eye` the distance to
/// it, `sigma` the fog's extinction there, `rmin` the nearest a light is held
/// to (the froxel's own size: it stands for its whole cell).
fn lf_light(index: u32, li: LfLight, p: vec3<f32>, to_eye: vec3<f32>, t_eye: f32, sigma: f32, rmin: f32) -> vec3<f32> {
    // where the light leaves its luminaire: a props street light's (kind 3) source stands `size` m
    // over its head (the JS practical's lift, which widens the pool), its intensity carrying the
    // pool gain (1 + lift)^2 (sky_fx `fx_luminaire`): the fog glows round the head, not over it.
    var src = li.pos;
    var inten = li.intensity;
    if (li.kind == 3u && li.size > 0.0) {
        src = li.pos - vec3<f32>(0.0, li.size, 0.0);
        let g = 1.0 + LF_JS_SOURCE_LIFT_MH;
        inten = li.intensity / (g * g);
    }
    let d = p - src;
    let dist = length(d);
    if (dist >= li.range || inten <= 0.0) {
        return vec3<f32>(0.0);
    }
    let lt = d / max(dist, 1e-4); // the light's direction of travel
    let w = clamp(1.0 - pow(dist / li.range, 4.0), 0.0, 1.0);
    var shape = 1.0;
    if (li.kind == 0u || li.kind == 3u) {
        // a street luminaire (the fallback's, a props street light's head): the cobra head's
        // batwing out along the street and its tail to the horizontal (eo-sky
        // lit_fog::luminaire_shape), the light a lamp seen down the street sends toward the eye.
        // (the surfaces keep their pools' shapes: the fallback's 75 deg cut-off, the props
        // practical's spot cone)
        let ca = dot(lt, select(li.dir, vec3<f32>(0.0, -1.0, 0.0), li.kind == 3u));
        let tail = pow(clamp(ca / LF_LUMINAIRE_COS_FULL, 0.0, 1.0), LF_LUMINAIRE_TAIL_POWER);
        shape = (1.0 + 1.5 * (1.0 - ca * ca)) * tail;
    } else if (li.kind == 1u) {
        // a low beam (sky_light.wgsl): the hot spot just under the cut-off, the foreground
        // falling as (2.5 deg / depression)^2, the glare zone over the cut-off; sideways the cone.
        // (the surfaces' near-field limit is not taken: the beam lights the fog from the lens out.)
        let dep = asin(clamp(-lt.y, -1.0, 1.0)) * 57.29578;
        var vert = 1.0;
        if (dep < 0.6) {
            vert = 0.031;
        } else if (dep < 1.3) {
            vert = mix(0.031, 1.0, (dep - 0.6) / 0.7);
        } else if (dep > 2.5) {
            vert = (2.5 / dep) * (2.5 / dep);
        }
        let hl = normalize(vec2<f32>(lt.x, lt.z) + vec2<f32>(1e-6, 0.0));
        let ha = normalize(vec2<f32>(li.dir.x, li.dir.z) + vec2<f32>(1e-6, 0.0));
        shape = vert * smoothstep(li.cos_outer, li.cos_inner, dot(hl, ha));
    }
    if (shape <= 0.0) {
        return vec3<f32>(0.0);
    }
    let tau_l = sigma * dist;
    let e = inten * shape * w * w / max(dist * dist, rmin * rmin) * exp(-tau_l * (1.0 - LF_G)) * lf_light_visibility(index, li, p);
    return li.color * (e * lf_phase(dot(lt, to_eye), tau_l + sigma * t_eye));
}

/// The clustered lights at a fog point (`sky_local_lights`' lookup).
fn lf_lights_at(p: vec3<f32>, vz: f32, ndc: vec2<f32>, to_eye: vec3<f32>, t_eye: f32, sigma: f32, rmin: f32) -> vec3<f32> {
    if (!sky_on(skyu.flags.x, SKY_LIGHTS) || skyu.lights.x == 0u || vz < skyu.cluster.x || vz > skyu.cluster.y) {
        return vec3<f32>(0.0);
    }
    let g = skyu.lights.yzw;
    let tx = u32(clamp((ndc.x * 0.5 + 0.5) * f32(g.x), 0.0, f32(g.x) - 1.0));
    let ty = u32(clamp((0.5 - ndc.y * 0.5) * f32(g.y), 0.0, f32(g.y) - 1.0));
    let tz = u32(clamp(log(vz / skyu.cluster.x) / log(skyu.cluster.y / skyu.cluster.x) * f32(g.z), 0.0, f32(g.z) - 1.0));
    let c = (tz * g.y + ty) * g.x + tx;
    let count = min(lf_clusters[c], LF_CLUSTER_CAP);
    let base = g.x * g.y * g.z + c * LF_CLUSTER_CAP;
    var out = vec3<f32>(0.0);
    for (var k = 0u; k < count; k = k + 1u) {
        let i = lf_clusters[base + k];
        out = out + lf_light(i, lf_lights[i], p, to_eye, t_eye, sigma, rmin);
    }
    return out;
}

@compute @workgroup_size(8, 8, 1)
fn lf_inject(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(lf_out);
    if (any(gid >= dims)) {
        return;
    }
    let slices = f32(dims.z);
    let v = lf_view(gid.xy, dims);
    // the sample's depth: through the slice by a per-froxel offset (interleaved gradient noise)
    // moved along the R1 sequence each frame; the history averages the slice.
    let jit = fract(sky_ign(vec2<f32>(gid.xy) + vec2<f32>(f32(gid.z) * 5.588238, f32(gid.z) * 3.1)) + f32(skyu.flags.y % 1024u) * 0.618034);
    let z0 = lf_depth(f32(gid.z), slices);
    let z1 = lf_depth(f32(gid.z) + 1.0, slices);
    let z = mix(z0, z1, jit);
    let p = lf_world(v, z);
    let vlen = length(v);
    let t_eye = z * vlen;
    let to_eye = -(skyu.cam_right.xyz * v.x + skyu.cam_up.xyz * v.y + skyu.cam_fwd.xyz) / vlen;
    let sigma = lf_density(p);
    // a froxel stands for its cell: no light nearer than half the cell's width or an eighth of its
    // depth (a jittered sample beside a lamp would otherwise flicker), nor than the lamp head's size.
    let rmin = max(LF_MIN_DIST_M, max(z * skyu.cluster.z / f32(dims.y), (z1 - z0) * 0.125));
    let ndc = vec2<f32>(v.x / (skyu.cluster.z * skyu.cluster.w), v.y / skyu.cluster.z);
    var cur = vec4<f32>(0.0, 0.0, 0.0, sigma);
    if (sigma > 0.0) {
        cur = vec4<f32>(lf_lights_at(p, z, ndc, to_eye, t_eye, sigma, rmin) * LF_ALBEDO, sigma);
    }
    if (any(cur != cur) || any(abs(cur) > vec4<f32>(6e4))) {
        cur = vec4<f32>(0.0, 0.0, 0.0, sigma);
    }
    // the last frame's froxel at this one's centre (not after a cut, nor when the last frame had none).
    var res = cur;
    if (skyu.taa.y > 0.5 && lf_state[0] + 1u == skyu.flags.y) {
        let pc = lf_world(v, mix(z0, z1, 0.5));
        let clip = skyu.prev_vp * vec4<f32>(pc, 1.0);
        if (clip.w > 1e-3) {
            let puv = vec2<f32>(clip.x / clip.w * 0.5 + 0.5, 0.5 - clip.y / clip.w * 0.5);
            let ps = log(clip.w / skyu.cluster.x) / log(skyu.cluster.y / skyu.cluster.x);
            if (all(puv >= vec2<f32>(0.0)) && all(puv <= vec2<f32>(1.0)) && ps >= 0.0 && ps <= 1.0) {
                let h = textureSampleLevel(lf_hist, lf_lin, vec3<f32>(puv, ps), 0.0);
                if (all(h == h)) {
                    // rejection: the history held within LF_CLAMP of this frame's light (a beam
                    // that has moved on leaves nothing behind; the jitter's spread still averages).
                    let lo = cur.rgb / LF_CLAMP;
                    let hi = cur.rgb * LF_CLAMP + vec3<f32>(LF_CLAMP_FLOOR);
                    let hc = vec4<f32>(clamp(h.rgb, lo, hi), h.a);
                    // the light a droplet sends to the eye depends on where the eye is (the forward
                    // lobe): as the eye moves, the history stands for a direction the eye has left, so
                    // it gives way with the turn of this point's direction to the eye since last frame.
                    let turn = acos(clamp(dot(normalize(skyu.eye.xyz - pc), normalize(skyu.prev_eye.xyz - pc)), -1.0, 1.0));
                    res = mix(hc, cur, max(LF_NEW_WEIGHT, turn / (turn + LF_TURN_HALF_RAD)));
                }
            }
        }
    }
    textureStore(lf_out, vec3<i32>(gid), res);
}

@compute @workgroup_size(8, 8, 1)
fn lf_integrate(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(lf_integ);
    if (gid.x >= dims.x || gid.y >= dims.y) {
        return;
    }
    let slices = f32(dims.z);
    let vlen = length(lf_view(gid.xy, dims));
    var l = vec3<f32>(0.0);
    var tr = 1.0;
    var z0 = lf_depth(0.0, slices);
    for (var k = 0u; k < dims.z; k = k + 1u) {
        let z1 = lf_depth(f32(k) + 1.0, slices);
        let ds = (z1 - z0) * vlen;
        let f = textureLoad(lf_src, vec3<i32>(vec2<i32>(gid.xy), i32(k)), 0);
        // the slice's own transmittance; the light it scatters in, attenuated by what lies in front
        // and through itself (Hillaire 2015's energy-conserving step: (1 - T) / sigma of the source).
        let od = max(f.a, 0.0) * ds;
        let t = exp(-od);
        l = l + tr * f.rgb * select(od, 1.0 - t, od > 1e-4);
        tr = tr * t;
        textureStore(lf_integ, vec3<i32>(vec2<i32>(gid.xy), i32(k)), vec4<f32>(l, tr));
        z0 = z1;
    }
    if (gid.x == 0u && gid.y == 0u) {
        lf_state[0] = skyu.flags.y;
    }
}
