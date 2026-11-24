// the city shader's side of the sky slice (appended to the city shader
// by look.rs city_shader_source; group 1 is the look's): cloud shadows on the
// key, the clustered city lights (street lamps, headlights), wet ground with
// puddles and ripples, and the screen-space reflection of the previous frame
// for wet ground. Its bindings and the cloud shadow are in sky_light_core.wgsl
// (appended before this; the water shader takes that part too).

/// Contact shadows (post-chain.js contact shadow: 0.85 m toward the key,
/// bias 0.022 m, thickness 0.5 m, intensity 0.72, faded 45-80 m), marched
/// here through the previous frame's half-resolution depth (the occlusion
/// pass's, with its camera), 8 steps: the small shadows under kerbs, sills,
/// feet and wheels the cascades are too coarse for.
fn sky_contact_shadow(world: vec3<f32>, ng: vec3<f32>) -> f32 {
    if (!sky_on(skyu.flags.x, SKY_CONTACT) || look.ao_params.x < 0.5 || frame.copy_step.z > 0.5 || look.key_dir.w <= 0.0) {
        return 1.0;
    }
    let dist = length(world - skyu.eye.xyz);
    let fade = 1.0 - smoothstep(45.0, 80.0, dist);
    if (fade <= 0.0) {
        return 1.0;
    }
    let l = look.key_dir.xyz;
    let dims = vec2<i32>(textureDimensions(lk_ao_z));
    let bias = max(0.022, dist * 0.0015);
    let start = world + ng * bias;
    var hits = 0.0;
    for (var i = 1; i <= 8; i = i + 1) {
        let p = start + l * (0.85 * f32(i) / 8.0);
        let clip = look.ao_vp * vec4<f32>(p, 1.0);
        if (clip.w <= 1e-3) {
            break;
        }
        let uv = vec2<f32>(clip.x / clip.w * 0.5 + 0.5, 0.5 - clip.y / clip.w * 0.5);
        if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) {
            break;
        }
        let z = textureLoad(lk_ao_z, clamp(vec2<i32>(uv * vec2<f32>(dims)), vec2<i32>(0), dims - 1), 0).r;
        let zr = dot(p - look.ao_cam.xyz, look.ao_fwd.xyz);
        let dz = zr - z;
        if (z > 0.0 && dz > bias && dz < 0.5) {
            hits = hits + 1.0;
        }
    }
    let occ = clamp(hits / (0.45 * 8.0), 0.0, 1.0);
    return 1.0 - occ * 0.72 * fade * smoothstep(0.0, 0.25, dot(ng, l));
}

// ---- (`--look cinematic`, stage L2; eo-sky `cine_lamps`, its values kept
// equal by eo-render sky.rs `the_cinematic_lamp_constants_are_eo_skys`) ----
// A street luminaire's pool: an LED cobra head (IES type II/III) long along
// its street (the axis its `dir` leans toward, sky_lights.wgsl
// `lt_cine_lamp`), short across, a soft roll-off; the low beam's cut-off
// stepped up on the kerb side and its hot spot; the local lights' specular
// widened by the source's size and capped. Compiled out without the look.
const SKY_CINE_COBRA_GAIN: f32 = 2.2;
const SKY_CINE_COBRA_ALONG_COS: vec2<f32> = vec2<f32>(0.4694716, 0.1391731);
const SKY_CINE_COBRA_ACROSS_COS: vec2<f32> = vec2<f32>(0.7431448, 0.3420201);
const SKY_CINE_COBRA_ROUND: f32 = 0.5;
const SKY_CINE_BEAM_CUT: vec2<f32> = vec2<f32>(0.4, 0.0);
const SKY_CINE_BEAM_STEP: f32 = 2.0;
const SKY_CINE_BEAM_EDGE: f32 = 0.35;
const SKY_CINE_BEAM_GLARE: f32 = 0.031;
const SKY_CINE_BEAM_HOT: vec2<f32> = vec2<f32>(1.5, 7.0);
const SKY_CINE_BEAM_SPREAD: f32 = 0.18;
const SKY_CINE_SRC_STREET: f32 = 0.3;
const SKY_CINE_SRC_HEADLAMP: f32 = 0.09;
const SKY_CINE_SPEC_CAP: f32 = 16.0;

/// The cobra head's intensity over its nadir's toward `-l` (eo-sky `cobra_shape`).
fn sky_cine_cobra(l: vec3<f32>, dir: vec3<f32>) -> f32 {
    let c = l.y;
    let ax = dir.xz;
    var along2 = SKY_CINE_COBRA_ROUND;
    if (dot(ax, ax) > 1e-6) {
        let hd = -l.xz;
        let k = dot(hd, ax);
        along2 = k * k / max(dot(hd, hd) * dot(ax, ax), 1e-12);
    }
    let lo = mix(SKY_CINE_COBRA_ACROSS_COS.x, SKY_CINE_COBRA_ALONG_COS.x, along2);
    let hi = mix(SKY_CINE_COBRA_ACROSS_COS.y, SKY_CINE_COBRA_ALONG_COS.y, along2);
    return (1.0 + SKY_CINE_COBRA_GAIN * max(1.0 - c * c, 0.0) * along2) * smoothstep(hi, lo, c);
}

/// The low beam's vertical and horizontal shape (eo-sky `low_beam_shape`):
/// `dep` deg below the horizontal, `h` deg right of the axis.
fn sky_cine_low_beam(dep: f32, h: f32) -> f32 {
    let cut = mix(SKY_CINE_BEAM_CUT.x, SKY_CINE_BEAM_CUT.y, smoothstep(0.0, SKY_CINE_BEAM_STEP, h));
    let under = smoothstep(cut, cut + SKY_CINE_BEAM_EDGE, dep);
    let x = (h - SKY_CINE_BEAM_HOT.x) / SKY_CINE_BEAM_HOT.y;
    let hot = SKY_CINE_BEAM_SPREAD + (1.0 - SKY_CINE_BEAM_SPREAD) * exp(-x * x);
    let fall = select(1.0, (2.5 / max(dep, 1e-3)) * (2.5 / max(dep, 1e-3)), dep > 2.5);
    return mix(SKY_CINE_BEAM_GLARE, hot * fall, under);
}

// ---- (`--weather damp` and `--look cinematic`, stage L3; eo-sky `cine_wet`,
// its values kept equal by eo-render tests/cine_wet.rs) ----
// The damp street's drying patches (`--weather damp`, any look). Under the
// look: wet ground darker and glossier (Lekner & Dorf: the albedo about
// halves), and the lights' reflections on it as a water film's: an
// anisotropic GGX lobe (F0 0.02) long along the ground toward the eye and
// narrow across it, so lamps, headlamps and shop windows draw long streaks
// down the road. Each streak light (the frame's brightest toward the eye,
// sky_streaks.wgsl, its list after the clusters' in `sky_clusters`) is
// reflected without the clusters' reach, with its luminaire's lens as the
// floor of its emission (what the eye sees of it), its reflected radiance
// never over the film's Fresnel times the lens's luminance (a mirror's
// bound); the clusters keep the light's diffuse. Compiled into the street's
// pipelines (the full variant) only with SF_CINEMATIC.
const SKY_DAMP_DRY_FLOOR: f32 = 0.3;
const SKY_CINE_WET_DARKEN: vec2<f32> = vec2<f32>(0.45, 0.55);
const SKY_CINE_WET_ROUGH: vec2<f32> = vec2<f32>(0.17, 0.26);
const SKY_CINE_STREAK_F0: f32 = 0.02;
const SKY_CINE_STREAK_ALONG: f32 = 0.14;
const SKY_CINE_STREAK_ACROSS: f32 = 0.02;
const SKY_CINE_PUDDLE_ALPHA: f32 = 0.006;
const SKY_CINE_STREAK_LENS: f32 = 0.35;
const SKY_CINE_STREAK_HEAD_LENS: f32 = 0.06;
const SKY_STREAK_CAP: u32 = 32u;
/// The street's pipelines (the full shadow variant: the city, agents, wires,
/// CC0): the props and the far layer compile none of the look's wet code.
const SKY_CINE_FULL: bool = SF_CINEMATIC && LK_SHADOW_CASCADES && LK_SHADOW_FAR;

/// (stage L3) The wet film's share at the street fragment being shaded (set
/// by `sky_wet_street` under the look; 0 elsewhere: no streaks) and its puddle's.
var<private> sky_streak_w: f32;
var<private> sky_streak_pud: f32;
/// (stage L3) Car paint's clearcoat (set by agents_draw.wgsl under the look:
/// its share, its roughness; 0 elsewhere) and the street's lights in it (the
/// clusters' loop gathers them for look_light's coat).
var<private> sky_coat: f32;
var<private> sky_coat_rough: f32;
var<private> sky_coat_lights: vec3<f32>;

/// (`--weather damp`) How much of the film is left at a point of the street
/// an hour after rain: drying patches by noise, the crown first, the gutter
/// and the kerb side last (`kerb` m from the kerb, < 0 none).
fn sky_damp_left(p: vec2<f32>, kerb: f32) -> f32 {
    let field = 0.55 * st_noise(p / 3.1 + 11.3) + 0.45 * st_noise(p / 0.9 + 7.3) + 0.35 * sky_drain(kerb);
    return mix(SKY_DAMP_DRY_FLOOR, 1.0, smoothstep(0.3, 0.62, field));
}

/// (stage L3) The water film's anisotropic GGX (Heitz 2014; height-correlated
/// Smith), `at` along `t`, `ab` across: the BRDF (1/sr) times n.l.
fn sky_cine_film(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, t: vec3<f32>, at: f32, ab: f32) -> f32 {
    let nl = dot(n, l);
    let nv = max(dot(n, v), 1e-4);
    if (nl <= 0.0) {
        return 0.0;
    }
    let b = cross(n, t);
    let h = normalize(l + v);
    let th = dot(t, h) / at;
    let bh = dot(b, h) / ab;
    let nh = dot(n, h);
    let dd = th * th + bh * bh + nh * nh;
    let d = 1.0 / (LK_PI * at * ab * dd * dd);
    let gv = nl * length(vec3<f32>(at * dot(t, v), ab * dot(b, v), nv));
    let gl = nv * length(vec3<f32>(at * dot(t, l), ab * dot(b, l), nl));
    let vis = 0.5 / max(gv + gl, 1e-6);
    let f = SKY_CINE_STREAK_F0 + (1.0 - SKY_CINE_STREAK_F0) * pow(1.0 - max(dot(v, h), 0.0), 5.0);
    return f * d * vis * nl;
}

/// (stage L3) The streak lights' reflections in the wet film at a street
/// point (radiance): every one, whatever its distance (no reach), its
/// emission toward the point the larger of its pool's and its lens's.
fn sky_cine_streaks(world: vec3<f32>, n: vec3<f32>, v: vec3<f32>) -> vec3<f32> {
    let g = skyu.lights.yzw;
    let base = g.x * g.y * g.z * (1u + SKY_CLUSTER_CAP);
    let count = min(sky_clusters[base], SKY_STREAK_CAP);
    // the tangent along the ground toward the eye (the streak's length)
    var t = v - n * dot(v, n);
    if (dot(t, t) < 1e-8) {
        t = cross(n, vec3<f32>(1.0, 0.0, 0.0));
    }
    t = normalize(t);
    // (`--wet-surfaces`) a puddle's own lobe, and in rain its rings' (`sky_w2_lobe`; 0 without)
    let a_pud = select(SKY_CINE_PUDDLE_ALPHA, max(SKY_CINE_PUDDLE_ALPHA, sky_w2_pud_alpha), SKY_W2_FULL);
    let a_along = mix(SKY_CINE_STREAK_ALONG, a_pud, sky_streak_pud);
    let a_across = mix(SKY_CINE_STREAK_ACROSS, a_pud, sky_streak_pud);
    let ring_w = select(0.0, sky_w2_ring * sky_streak_pud, SKY_W2_FULL);
    let r_along = mix(SKY_CINE_STREAK_ALONG, SKY_W2_RING_LOBE.y, sky_streak_pud);
    let r_across = mix(SKY_CINE_STREAK_ACROSS, SKY_W2_RING_LOBE.y, sky_streak_pud);
    let lobes = select(1, 2, SKY_W2_FULL && ring_w > 0.0);
    var out = vec3<f32>(0.0);
    for (var k = 0u; k < count; k = k + 1u) {
        let li = sky_lights[sky_clusters[base + 1u + k]];
        let d = li.pos - world;
        let d2 = max(dot(d, d), 0.04);
        let dist = sqrt(d2);
        let l = d / dist;
        if (dot(n, l) <= 0.0) {
            continue;
        }
        let ca = dot(-l, li.dir);
        var shape = 1.0;
        var src = SKY_CINE_SRC_STREET;
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
            src = SKY_CINE_SRC_HEADLAMP;
        } else if (li.kind >= 4u) {
            shape = smoothstep(li.cos_outer, li.cos_inner, ca);
            src = 0.5 * sqrt(max(li.size, 0.0));
        }
        // the source's size widens the lobe both ways (as the clusters' specular)
        let wide = src / (2.0 * dist);
        // (`--wet-surfaces`, in rain) the rings' lobe over their share (one
        // call site: the lit pipelines stand at Apple's compiler's limit)
        var f = 0.0;
        for (var lb = 0; lb < lobes; lb = lb + 1) {
            let second = lb == 1;
            let wl = select(select(1.0, 1.0 - ring_w, lobes == 2), ring_w, second);
            f = f + wl * sky_cine_film(n, v, l, t, min(select(a_along, r_along, second) + wide, 1.0), min(select(a_across, r_across, second) + wide, 1.0));
        }
        // a mirror's bound: no more than the film's Fresnel (<= 1) times the source's luminance
        let e = li.intensity * shape / d2;
        out = out + li.color * e * min(f, d2 / (LK_PI * src * src));
    }
    return out * sky_streak_w;
}

/// The clustered city lights at a surface.
fn sky_local_lights(world: vec3<f32>, n: vec3<f32>, v: vec3<f32>, diffuse: vec3<f32>, f0: vec3<f32>, rough: f32) -> vec3<f32> {
    return sky_local_lights_ao(world, n, v, diffuse, f0, rough, 1.0);
}

/// `sky_local_lights` with the surface's occlusion `ao` (lighting,
/// `--light-shadows`, `--headlight-shadows`; light_shadow.rs): while they are
/// on, a light's record carries in its spare word its shadow map's tile + 1
/// (or -1: no map), and each light is then multiplied by `ao` and its shadow
/// (light_shadow_use.wgsl `ls_light_visibility`). Off, every record's word
/// is 0 and nothing here changes.
fn sky_local_lights_ao(world: vec3<f32>, n: vec3<f32>, v: vec3<f32>, diffuse: vec3<f32>, f0: vec3<f32>, rough: f32, ao: f32) -> vec3<f32> {
    if (!sky_on(skyu.flags.x, SKY_LIGHTS) || skyu.lights.x == 0u || frame.copy_step.z > 0.5) {
        return vec3<f32>(0.0);
    }
    let rel = world - skyu.eye.xyz;
    let vz = dot(rel, skyu.cam_fwd.xyz);
    if (vz < skyu.cluster.x || vz > skyu.cluster.y) {
        return vec3<f32>(0.0);
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
    var out = vec3<f32>(0.0);
    let cine = SF_CINEMATIC && sky_on(skyu.flags.x, SKY_CINEMATIC);
    for (var k = 0u; k < count; k = k + 1u) {
        let li = sky_lights[sky_clusters[base + k]];
        let d = li.pos - world;
        let d2 = max(dot(d, d), 0.04);
        let dist = sqrt(d2);
        let l = d / dist;
        let w = clamp(1.0 - pow(dist / li.range, 4.0), 0.0, 1.0);
        let ca = dot(-l, li.dir);
        var shape = 1.0;
        if (li.kind == 0u) {
            // A full-cutoff road luminaire: batwing toward its 60 deg peak, nothing past 75 deg.
            shape = (1.0 + 1.5 * (1.0 - ca * ca)) * smoothstep(li.cos_outer, li.cos_inner, ca);
            if (cine) {
                shape = sky_cine_cobra(l, li.dir); // (`--look cinematic`) the cobra head's pool
            }
        } else if (li.kind == 1u) {
            // A low beam as UN ECE R112 shapes one: the hot spot just under
            // the cut-off (0.6-2.5 deg down), the foreground falling as
            // (2.5 deg / depression)^2 (an even ~10-30 lux carpet a few metres
            // out, not a hot spot), above the cut-off the glare zone's
            // ~625 cd of the 20,000 cd peak (B50L: 0.031); sideways the
            // cone (0.42 rad, penumbra 0.55); and the JS near-field limit
            // (VEHICLE_LIGHTS: x (d / 6.5)^2 inside 6.5 m, d >= 0.4 m).
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
                // (`--look cinematic`) the cut-off stepped up on the kerb side
                // (right of the heading: (-z, x)) and the hot spot.
                let hr = vec2<f32>(-ha.y, ha.x);
                vert = sky_cine_low_beam(dep, atan2(dot(hl, hr), dot(hl, ha)) * 57.29578);
            }
            shape = vert * side * near * near;
        } else if (li.kind >= 3u) {
            // A props street light: the JS lamp practical's spot (three.js
            // SpotLight: smoothstep(cos angle, cos (angle (1 - penumbra))));
            // a shop window's light (kind 4, `--shop-light`) its window's lobe the same way.
            shape = smoothstep(li.cos_outer, li.cos_inner, ca);
        }
        let nl = max(dot(n, l), 0.0);
        var e = li.intensity * shape * w * w / d2 * nl;
        if (LS_SHADER && li.pad != 0.0 && e > 0.0) {
            e = e * ao;
            if (li.pad >= 1.0) {
                e = e * ls_light_visibility(u32(li.pad) - 1u, world, n);
            }
        }
        if (e > 0.0) {
            var re = rough;
            if (cine) {
                // (`--look cinematic`) the source's size widens the lobe (a shop window: half its glazed side).
                let src = select(select(SKY_CINE_SRC_STREET, SKY_CINE_SRC_HEADLAMP, li.kind == 1u), 0.5 * sqrt(max(li.size, 0.0)), li.kind >= 4u);
                re = sqrt(min(rough * rough + src / (2.0 * dist), 1.0));
            }
            var spec = lk_ggx(n, v, l, f0, re);
            if (cine) {
                spec = min(spec, vec3<f32>(SKY_CINE_SPEC_CAP));
            }
            if (SKY_CINE_FULL && sky_coat > 0.0) {
                // (stage L3) the paint's clearcoat (look_light), its lobe widened by the
                // source as above, bounded as a mirror (the lens's luminance x Fresnel)
                // rather than by the spec cap: a sharp coat shows the lamp
                let rc = sqrt(min(sky_coat_rough * sky_coat_rough + (re * re - rough * rough), 1.0));
                let bound = d2 / (LK_PI * SKY_CINE_SRC_STREET * SKY_CINE_SRC_STREET * max(nl, 1e-3));
                sky_coat_lights = sky_coat_lights + li.color * e * min(lk_ggx(n, v, l, vec3<f32>(0.04), rc), vec3<f32>(bound));
            }
            if (SKY_CINE_FULL && sky_streak_w > 0.0) {
                // (stage L3) the wet film reflects the streak lights below; here the
                // dry share's lobe only
                spec = spec * (1.0 - sky_streak_w);
            }
            out = out + li.color * e * (diffuse / LK_PI + spec);
        }
    }
    if (SKY_CINE_FULL && cine && sky_streak_w > 0.0) {
        out = out + sky_cine_streaks(world, n, v); // (stage L3) the lights' streaks on the wet street
    }
    return out;
}

/// Lit windows on the far boxes at night (the far ring and every building
/// drawn as its footprint box): a grid of windows (3.0 m bays, 3.4 m storeys,
/// the pane 60 % x 55 % of the cell) lit by the room drive's occupancy
/// (night-interiors.js `nightPracticalProfile`), warm 2650 K or cool 4900 K by
/// its cool share (renderer.js emissive windows); where a cell is under two
/// pixels the pattern gives way to its mean so it cannot shimmer. Radiance:
/// a lit pane at ~25 lux-equivalent / pi of the lamps' scale, about 10 x a lit
/// road's.
fn sky_far_windows(world: vec3<f32>, n: vec3<f32>, object: u32, dpx: vec3<f32>, dpy: vec3<f32>) -> vec3<f32> {
    let dusk = look.rooms_env.x;
    if (!sky_on(skyu.flags.x, SKY_LIGHTS) || dusk <= 0.0 || abs(n.y) > 0.3 || frame.copy_step.z > 0.5) {
        return vec3<f32>(0.0);
    }
    let t = normalize(vec2<f32>(-n.z, n.x) + vec2<f32>(1e-5, 0.0));
    let u = dot(world.xz, t) / 3.0;
    let v = world.y / 3.4;
    let cell = floor(vec2<f32>(u, v));
    let f = fract(vec2<f32>(u, v));
    let pane = step(0.2, f.x) * step(f.x, 0.8) * step(0.25, f.y) * step(f.y, 0.8);
    let h = st_hash(cell * vec2<f32>(1.37, 2.11) + f32(object % 4096u) * 0.731);
    let occ = look.rooms_env.y;
    let lit = step(h, occ);
    let cool = step(1.0 - look.rooms_env.w, st_hash(cell + 17.3));
    let col = select(vec3<f32>(1.0, 0.62, 0.34), vec3<f32>(0.86, 0.93, 1.0), cool > 0.5);
    let px = max(length(dpx), length(dpy)) / 3.0;
    let mean = occ * 0.33;
    let k = mix(pane * lit, mean, smoothstep(0.25, 0.6, px));
    let level = skyu.bloom_w2.z * 25.0 / LK_PI * dusk * (0.6 + 0.8 * st_hash(cell + 5.1));
    return col * k * level;
}

// ---- wet ground (wet-surface-reflection.js puddleMaskNode, environment-ibl.js
// classed-material grade, street-surface-v2-render.js film and roughness) ----

fn sky_drain(k: f32) -> f32 {
    if (k < 0.0) {
        return 0.0;
    }
    if (k <= 1.3) {
        return 1.0;
    }
    if (k <= 3.0) {
        return mix(1.0, 0.5, (k - 1.3) / 1.7);
    }
    return max(0.0, mix(0.5, 0.0, (k - 3.0) / 1.2));
}

/// Puddle coverage 0..1 at a horizontal point (kerb distance in m, < 0 none).
fn sky_puddle(p: vec2<f32>, kerb: f32) -> f32 {
    let fill = skyu.wet.y;
    if (fill <= 0.0) {
        return 0.0;
    }
    let field = 0.62 * (0.3 * st_noise(p / 0.7) + 0.7 * st_noise(p / 2.4 + 31.7)) + 0.58 * sky_drain(kerb);
    let thr = 1.2 + (0.885 - 1.2) * pow(fill, 0.35);
    return smoothstep(thr - 0.05, thr + 0.05, field);
}

/// Rain rings on standing water: one drop per 0.45 m cell at a random phase,
/// a ring widening at 0.3 m/s, gone in 0.55 s. Returns a tangent-plane slope.
fn sky_ripple(p: vec2<f32>) -> vec2<f32> {
    let rate = skyu.wet.z;
    if (rate <= 0.0) {
        return vec2<f32>(0.0);
    }
    let cell = floor(p / 0.45);
    let h = st_hash(cell * 1.7 + 3.1);
    let c = (cell + 0.25 + 0.5 * vec2<f32>(h, st_hash(cell + 9.2))) * 0.45;
    let speed = 0.6 + 1.2 * clamp(rate / 10.0, 0.0, 1.0);
    let age = fract(skyu.wet.w * speed / 0.55 + h);
    let d = p - c;
    let r = length(d);
    let front = age * 0.17;
    let x = (r - front) * 70.0;
    let amp = (1.0 - age) * (1.0 - age) * exp(-x * x * 0.12) * 0.35;
    return select(vec2<f32>(0.0), d / max(r, 1e-4) * cos(x) * amp, r < 0.2);
}

/// Wet the street surface (the carriageway and the walk), before lighting.
fn sky_wet_street(s: ptr<function, Surface>, world: vec3<f32>, n_geom: vec3<f32>, kerb: f32, asphalt: bool) {
    var w = skyu.wet.x;
    if (!sky_on(skyu.flags.x, SKY_WET) || w < 0.02) {
        return;
    }
    if (sky_w2_on()) {
        sky_w2_street(s, world, n_geom, kerb, asphalt); // (`--wet-surfaces`)
        return;
    }
    if (sky_on(skyu.flags.x, SKY_DAMP)) {
        w = w * sky_damp_left(world.xz, select(-1.0, kerb, asphalt)); // (`--weather damp`) drying patches
    }
    var darken = select(0.66, 0.58, asphalt);
    var wet_rough = select(0.32, 0.24, asphalt);
    let cine = SKY_CINE_FULL && sky_on(skyu.flags.x, SKY_CINEMATIC);
    if (cine) {
        // (`--look cinematic`, stage L3) the film darker and glossier
        darken = select(SKY_CINE_WET_DARKEN.y, SKY_CINE_WET_DARKEN.x, asphalt);
        wet_rough = select(SKY_CINE_WET_ROUGH.y, SKY_CINE_WET_ROUGH.x, asphalt);
    }
    var pud = 0.0;
    if (n_geom.y > 0.95) {
        pud = sky_puddle(world.xz, select(-1.0, kerb, asphalt)) * w;
    }
    if (cine && n_geom.y > 0.7) {
        sky_streak_w = w;
        sky_streak_pud = pud;
    }
    (*s).albedo = (*s).albedo * mix(1.0, darken, w) * (1.0 - 0.2 * pud);
    // The film's grazing sheet (street-surface-v2-render.js: the film floor
    // mix(0.48, 0.05, grazeShare), grazeShare = smoothstep(0.26, 0.05, n.v)):
    // seen low along the street the film is a mirror.
    let v = normalize(skyu.eye.xyz - world);
    let graze = smoothstep(0.26, 0.05, abs(dot(n_geom, v))) * select(0.0, 1.0, n_geom.y > 0.95);
    let film = mix(wet_rough, 0.06, graze * 0.92);
    (*s).rough = mix(mix((*s).rough, film, w), 0.05, pud);
    // Relief filled by the film (reliefFill 0.75 / 0.70), flat in a puddle.
    var n = normalize(mix((*s).n, n_geom, clamp(w * select(0.70, 0.75, asphalt) + pud, 0.0, 1.0)));
    if (pud > 0.01) {
        let sl = sky_ripple(world.xz) * pud;
        n = normalize(n + vec3<f32>(sl.x, 0.0, sl.y));
    }
    (*s).n = n;
}

/// `sky_wet_generic` at `world` (look_light): with `--wet-surfaces` the
/// switch's porosity, puddles and wall runs (`sky_w2_generic`).
fn sky_wet_generic_at(s: ptr<function, Surface>, ng: vec3<f32>, world: vec3<f32>) {
    if (sky_w2_on()) {
        if (sky_on(skyu.flags.x, SKY_WET) && skyu.wet.x >= 0.02) {
            sky_w2_generic(s, ng, world);
        }
        return;
    }
    sky_wet_generic(s, ng);
}

/// Any other surface in the rain: roofs and ground soak (masonry darken 0.78,
/// wet roughness 0.54), walls take a third of it.
fn sky_wet_generic(s: ptr<function, Surface>, ng: vec3<f32>) {
    let w = skyu.wet.x;
    if (!sky_on(skyu.flags.x, SKY_WET) || w < 0.02) {
        return;
    }
    let wt = w * (0.33 + 0.67 * smoothstep(0.5, 0.9, ng.y));
    (*s).albedo = (*s).albedo * mix(1.0, 0.78, wt);
    (*s).rough = mix((*s).rough, min((*s).rough, 0.54), wt);
}

/// The screen-space reflection on glossy ground: a march of the mirrored ray
/// through the previous frame's depth (the occlusion pass's half-resolution
/// view distance, with the camera it was made with), the hit read from the
/// previous resolved frame (wet-surface-reflection.js WET_REFLECTION_MARCH:
/// 12 steps from 0.12 m, first 0.22 m, x1.7, scaled by range / 30 m, 6
/// refinements, no ray above an elevation sine of 0.5). rgb radiance, a weight.
fn sky_ssr(world: vec3<f32>, n: vec3<f32>, v: vec3<f32>, rough: f32) -> vec4<f32> {
    if (sky_sr_on()) {
        return sky_sr_ssr(world, n, v, rough); // (`--screen-reflect`)
    }
    if (SKY_CINE_FULL && sky_on(skyu.flags.x, SKY_CINEMATIC)) {
        return sky_ssr_cine(world, n, v, rough); // (`--look cinematic`, stage L3)
    }
    if (!sky_on(skyu.flags.x, SKY_SSR) || skyu.flags.z == 0u || look.ao_params.x < 0.5 || rough > 0.36 || frame.copy_step.z > 0.5) {
        return vec4<f32>(0.0);
    }
    let r = reflect(-v, n);
    if (r.y > 0.5 || n.y < 0.6) {
        return vec4<f32>(0.0);
    }
    let dims = vec2<i32>(textureDimensions(lk_ao_z));
    let range = length(world - skyu.eye.xyz);
    var step = 0.22 * max(range / 30.0, 1.0);
    var t = 0.12;
    var t_prev = 0.0;
    var hit = -1.0;
    for (var i = 0; i < 12; i = i + 1) {
        let p = world + r * t;
        let clip = look.ao_vp * vec4<f32>(p, 1.0);
        if (clip.w <= 1e-3) {
            break;
        }
        let uv = vec2<f32>(clip.x / clip.w * 0.5 + 0.5, 0.5 - clip.y / clip.w * 0.5);
        if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) {
            break;
        }
        let z = textureLoad(lk_ao_z, clamp(vec2<i32>(uv * vec2<f32>(dims)), vec2<i32>(0), dims - 1), 0).r;
        let zr = dot(p - look.ao_cam.xyz, look.ao_fwd.xyz);
        if (z > 0.0 && zr > z + 0.05 && zr - z < 0.6 + 0.04 * z) {
            hit = t;
            break;
        }
        t_prev = t;
        t = t + step;
        step = step * 1.7;
    }
    if (hit < 0.0) {
        return vec4<f32>(0.0);
    }
    var lo = t_prev;
    var hi = hit;
    for (var i = 0; i < 6; i = i + 1) {
        let mid = 0.5 * (lo + hi);
        let p = world + r * mid;
        let clip = look.ao_vp * vec4<f32>(p, 1.0);
        let uv = vec2<f32>(clip.x / clip.w * 0.5 + 0.5, 0.5 - clip.y / clip.w * 0.5);
        let z = textureLoad(lk_ao_z, clamp(vec2<i32>(uv * vec2<f32>(dims)), vec2<i32>(0), dims - 1), 0).r;
        let zr = dot(p - look.ao_cam.xyz, look.ao_fwd.xyz);
        if (z > 0.0 && zr > z) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    let p = world + r * hi;
    let clip = look.ao_vp * vec4<f32>(p, 1.0);
    let uv = vec2<f32>(clip.x / clip.w * 0.5 + 0.5, 0.5 - clip.y / clip.w * 0.5);
    var c = textureSampleLevel(sky_prev, lk_ao_samp, uv, 0.0).rgb;
    // The JS ceiling on a hit's radiance (6 x the sky + 3, in exposure terms).
    let cap = 8.0 / max(look.key.w, 0.1);
    c = c * min(1.0, cap / max(lk_luma(c), 1e-4));
    let edge = smoothstep(0.0, 0.05, min(min(uv.x, 1.0 - uv.x), min(uv.y, 1.0 - uv.y)));
    let w = edge * (1.0 - smoothstep(0.1, 0.36, rough)) * (1.0 - smoothstep(0.3, 0.5, r.y));
    return vec4<f32>(c, w);
}

// ---- (`--look cinematic`, stage L3) the reflection on glossy ground, longer
// and softer: up to roughness 0.5 (damp asphalt) fading from 0.3, a march of
// 24 steps from 0.15 m x 1.32 (about 370 m at 30 m, scaled by range /
// 30 m) that takes the first step behind the depth as its hit and refines
// it (a long step can no longer jump a pole's thickness test), the hit's
// colour read as 7 taps along the frame's vertical, as long as the film's
// lobe (so signs, signals, windows and lamps streak down the street like the
// lights' analytic streaks), and its brightness cap raised from the JS's 8
// to 24 x the exposure (neon and lit signs keep their colour). Compiled into
// the street's pipelines only with SF_CINEMATIC (`SKY_CINE_FULL`). ----
const SKY_CINE_SSR_ROUGH: vec2<f32> = vec2<f32>(0.3, 0.5);
const SKY_CINE_SSR_STEPS: i32 = 24;
const SKY_CINE_SSR_STEP: vec2<f32> = vec2<f32>(0.15, 1.32);
const SKY_CINE_SSR_CAP: f32 = 24.0;

fn sky_ssr_cine_uv(p: vec3<f32>) -> vec3<f32> {
    let clip = look.ao_vp * vec4<f32>(p, 1.0);
    if (clip.w <= 1e-3) {
        return vec3<f32>(-1.0);
    }
    return vec3<f32>(clip.x / clip.w * 0.5 + 0.5, 0.5 - clip.y / clip.w * 0.5, 1.0);
}

fn sky_ssr_cine(world: vec3<f32>, n: vec3<f32>, v: vec3<f32>, rough: f32) -> vec4<f32> {
    if (!sky_on(skyu.flags.x, SKY_SSR) || skyu.flags.z == 0u || look.ao_params.x < 0.5 || rough > SKY_CINE_SSR_ROUGH.y || frame.copy_step.z > 0.5) {
        return vec4<f32>(0.0);
    }
    let r = reflect(-v, n);
    // (a car's clearcoat: its flanks too, the street and the fronts across it)
    if (r.y > 0.6 || (n.y < 0.6 && sky_coat <= 0.0)) {
        return vec4<f32>(0.0);
    }
    let dims = vec2<i32>(textureDimensions(lk_ao_z));
    let range = length(world - skyu.eye.xyz);
    var step = SKY_CINE_SSR_STEP.x * max(range / 30.0, 1.0);
    var t = 0.1;
    var t_prev = 0.0;
    var hit = -1.0;
    for (var i = 0; i < SKY_CINE_SSR_STEPS; i = i + 1) {
        let p = world + r * t;
        let q = sky_ssr_cine_uv(p);
        if (q.z < 0.0 || any(q.xy < vec2<f32>(0.0)) || any(q.xy > vec2<f32>(1.0))) {
            break;
        }
        let z = textureLoad(lk_ao_z, clamp(vec2<i32>(q.xy * vec2<f32>(dims)), vec2<i32>(0), dims - 1), 0).r;
        let zr = dot(p - look.ao_cam.xyz, look.ao_fwd.xyz);
        if (z > 0.0 && zr > z + 0.03) {
            hit = t;
            break;
        }
        t_prev = t;
        t = t + step;
        step = step * SKY_CINE_SSR_STEP.y;
    }
    if (hit < 0.0) {
        return vec4<f32>(0.0);
    }
    var lo = t_prev;
    var hi = hit;
    for (var i = 0; i < 6; i = i + 1) {
        let mid = 0.5 * (lo + hi);
        let p = world + r * mid;
        let q = sky_ssr_cine_uv(p);
        let z = textureLoad(lk_ao_z, clamp(vec2<i32>(q.xy * vec2<f32>(dims)), vec2<i32>(0), dims - 1), 0).r;
        let zr = dot(p - look.ao_cam.xyz, look.ao_fwd.xyz);
        if (z > 0.0 && zr > z) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    let p = world + r * hi;
    let q = sky_ssr_cine_uv(p);
    // behind a thin thing (more than its thickness behind the depth there): no hit
    let z = textureLoad(lk_ao_z, clamp(vec2<i32>(q.xy * vec2<f32>(dims)), vec2<i32>(0), dims - 1), 0).r;
    let zr = dot(p - look.ao_cam.xyz, look.ao_fwd.xyz);
    let thick = 0.8 + 0.06 * z;
    let solid = 1.0 - smoothstep(thick, 2.0 * thick, zr - z);
    if (solid <= 0.0) {
        return vec4<f32>(0.0);
    }
    // the film's lobe along the frame's vertical (alpha, as a share of the frame's height)
    let along = mix(rough * rough, mix(SKY_CINE_STREAK_ALONG, SKY_CINE_PUDDLE_ALPHA, sky_streak_pud), sky_streak_w);
    let spread = along / max(skyu.cluster.z, 0.1);
    var c = vec3<f32>(0.0);
    var wsum = 0.0;
    for (var k = -3; k <= 3; k = k + 1) {
        let o = f32(k) / 3.0;
        let wk = exp(-1.5 * o * o);
        let uv = vec2<f32>(q.x, clamp(q.y + o * spread, 0.0, 1.0));
        c = c + textureSampleLevel(sky_prev, lk_ao_samp, uv, 0.0).rgb * wk;
        wsum = wsum + wk;
    }
    c = c / wsum;
    let cap = SKY_CINE_SSR_CAP / max(look.key.w, 0.1);
    c = c * min(1.0, cap / max(lk_luma(c), 1e-4));
    let edge = smoothstep(0.0, 0.05, min(min(q.x, 1.0 - q.x), min(q.y, 1.0 - q.y)));
    let w = edge * solid * (1.0 - smoothstep(SKY_CINE_SSR_ROUGH.x, SKY_CINE_SSR_ROUGH.y, rough)) * (1.0 - smoothstep(0.4, 0.6, r.y));
    return vec4<f32>(c, w);
}

// ---- (`--wet-surfaces`, the reflect track; sky.rs `SkyOptions::wet_surfaces`,
// `SKY_WET_SURFACES_BIT`; compiled out without SF_WET_SURFACES, and into the
// street's pipelines (the full shadow variant: the city, agents, wires, CC0)
// only) ----
// Every exposed surface takes the rain (Lagarde 2012, "Water drop 2a/2b";
// Lekner & Dorf 1988 for the darkening):
// * porosity: how much a surface darkens follows how porous it is, read from
//   its dry roughness (Lagarde: porosity = (roughness - 0.5) / 0.4, here from
//   0.45 over 0.45): open asphalt, concrete and render about halve, painted
//   metal and glazed faces keep their colour; the film fills the relief and
//   smooths every one;
// * puddles: water stands where the ground's hollow field (domain-warped
//   value-noise settlement hollows of about 4, 1.6 and 0.55 m, plus the
//   gutter's fall) passes the weather's water level (the puddle fill
//   `skyu.wet.y`, calibrated on the field's distribution: 3 % of an open
//   carriageway at fill 0.1, 11 % at 0.45, 16 % at 0.6, 30 % at 1; most of
//   the gutter pan). The shore is sharp (anti-aliased by the pixel's
//   footprint); a damp halo of saturated ground rings it; inside, still water:
//   roughness 0.05, the geometric normal, wind wavelets, rain rings;
// * the environment: a film or a puddle reflects its environment whole (the
//   rig's environment class, 0.66 for asphalt and 0.40 for walks, is a dry
//   surface's, look_light `sky_w2_env`);
// * walls: the rain's runs, vertical streaks of wet render below the edge of
//   the rain, a third as wet between them; soffits stay dry.
const SKY_WET_SURFACES: u32 = 8388608u;
const SKY_W2_FULL: bool = SF_WET_SURFACES && LK_SHADOW_CASCADES && LK_SHADOW_FAR;
/// The water level over the hollow field: thr(fill) = x - y fill^z.
const SKY_W2_LEVEL: vec3<f32> = vec3<f32>(0.795, 0.225, 0.7);
/// The gutter's fall in hollow units (x `sky_drain`), the walk's and open
/// ground's share of the carriageway's fill (better drained).
const SKY_W2_GUTTER: f32 = 0.22;
const SKY_W2_WALK_FILL: f32 = 0.5;
/// The damp halo's width in hollow units (about 8 % of a carriageway at fill 0.45).
const SKY_W2_HALO: f32 = 0.05;
/// Still water: its roughness, the darkening of the ground under it, the wind
/// wavelets' slope (at 1 m/s) and scale (m).
const SKY_W2_WATER_ROUGH: f32 = 0.05;
const SKY_W2_UNDER_WATER: f32 = 0.8;
const SKY_W2_WAVELET: vec2<f32> = vec2<f32>(0.012, 0.16);
/// Porosity from the dry roughness: (rough - x) / y.
const SKY_W2_POROSITY: vec2<f32> = vec2<f32>(0.45, 0.45);
/// Open ground and roofs (not the street): the darkening of a fully porous
/// one, the film's roughness.
const SKY_W2_GROUND: vec2<f32> = vec2<f32>(0.5, 0.24);
/// Walls: the runs' share of the wetness between them and in them, the
/// darkening of a porous wall fully wet, the film's roughness, the runs' width
/// and length scales (1/m).
const SKY_W2_WALL: vec4<f32> = vec4<f32>(0.3, 0.9, 0.62, 0.4);
const SKY_W2_RUNS: vec2<f32> = vec2<f32>(2.9, 0.16);
/// A puddle's surface is not a mirror. The wind's capillary wavelets finer
/// than the drawn ones tilt it: their mean square slope a m/s of wind, a
/// puddle's fetch of a metre or two taking a fortieth of the open sea's
/// (Cox & Munk 1954: 0.003 + 0.00512 W), capped at `x` m/s; the GGX alpha is
/// its root.
const SKY_W2_WAVELET_MSS: vec2<f32> = vec2<f32>(6.0, 0.000128);
/// Rain rings (Lagarde 2012; drop counts from Marshall-Palmer): about 66 drops
/// of ~2 mm a square metre a second for each mm/h, each ring living ~0.4 s,
/// its packet (two crests, ~4 cm wide) covering ~0.005 m2 on average, so
/// the share of the water under rings is 1 - exp(-x rate); under them the
/// water's slopes (rms ~0.08) make a second lobe of alpha `y`.
const SKY_W2_RING_LOBE: vec2<f32> = vec2<f32>(0.15, 0.08);
/// A ring drawn where the pixel can hold it: its wavelength (m), the speed
/// of its front (m/s), its life (s), its peak slope; the cells drops land in (m).
const SKY_W2_RING: vec4<f32> = vec4<f32>(0.022, 0.25, 0.42, 0.16);
const SKY_W2_RING_CELL: f32 = 0.25;

/// (`--wet-surfaces`) the share of the shaded surface that is water (the film
/// and puddles): look_light reads its environment whole there (`sky_w2_env`).
var<private> sky_w2_water: f32;
/// (`--wet-surfaces`) a puddle's lobe (set with the puddle; 0 elsewhere):
/// the still water's alpha with the wind's wavelets, and the share of the
/// water under rain rings (the second lobe's weight).
var<private> sky_w2_pud_alpha: f32;
var<private> sky_w2_ring: f32;

fn sky_w2_on() -> bool {
    return SKY_W2_FULL && sky_on(skyu.flags.x, SKY_WET_SURFACES);
}

/// (look_light) The environment's intensity at the surface: a dry surface's
/// class (`e`), the rig's whole environment on its water.
fn sky_w2_env(e: f32) -> f32 {
    if (!sky_w2_on()) {
        return e;
    }
    return mix(e, look.ambient.w, sky_w2_water);
}

/// The pixel's footprint on a surface at `world` (m, along the view's grazing stretch).
fn sky_w2_footprint(world: vec3<f32>, n: vec3<f32>) -> f32 {
    let d = world - skyu.eye.xyz;
    let dist = length(d);
    let nv = abs(dot(n, d)) / max(dist, 1e-3);
    return dist * 2.0 * skyu.cluster.z / max(skyu.res.y, 1.0) / max(nv, 0.08);
}

/// The ground's hollow field at a horizontal point (0 a crest .. 1 the deepest
/// hollow; mean 0.5, sd 0.133 without the gutter), the finest octave faded to
/// its mean where the pixel's footprint `fp` (m) cannot hold it.
fn sky_w2_hollow(p: vec2<f32>, kerb: f32, fp: f32) -> f32 {
    let warp = vec2<f32>(st_noise(p * 0.19 + 3.7), st_noise(p * 0.19 + 19.1)) - 0.5;
    let q = p + warp * 3.0;
    let fine = mix(st_noise(q * 1.8 + 23.1), 0.5, smoothstep(0.08, 0.3, fp));
    return 0.5 * st_noise(q * 0.26 + 41.3) + 0.32 * st_noise(q * 0.63 + 7.9) + 0.18 * fine + SKY_W2_GUTTER * sky_drain(kerb);
}

/// The puddle (x, 0..1) and its damp halo (y, 0..1, none inside the water) at a
/// flat point: `fill` the water level's fill, `fp` the pixel's footprint.
fn sky_w2_puddle(p: vec2<f32>, kerb: f32, fill: f32, fp: f32) -> vec2<f32> {
    if (fill <= 0.0) {
        return vec2<f32>(0.0);
    }
    let depth = sky_w2_hollow(p, kerb, fp) - (SKY_W2_LEVEL.x - SKY_W2_LEVEL.y * pow(fill, SKY_W2_LEVEL.z));
    // the shore: about a centimetre, widened to the footprint (the field's slope ~0.4 / m)
    let shore = max(0.004, 0.4 * fp);
    let pud = smoothstep(-0.5 * shore, 0.5 * shore, depth);
    let halo = smoothstep(-SKY_W2_HALO, 0.0, depth) * (1.0 - pud) * clamp(fill * 2.5, 0.0, 1.0);
    return vec2<f32>(pud, halo);
}

/// Rain rings where the pixel's footprint `fp` holds their wavelength: in
/// each of two offset grids of `SKY_W2_RING_CELL` cells a drop lands once a
/// life at a random phase (only in the share of cells the rate fills), and
/// sends out a capillary packet of about two crests whose slope falls as it
/// spreads and ages. Past the footprint the rings are the second lobe instead
/// (`sky_w2_lobe`). Returns a tangent-plane slope.
fn sky_w2_rings(p: vec2<f32>, fp: f32) -> vec2<f32> {
    let rate = skyu.wet.z;
    let lam = SKY_W2_RING.x;
    let fade = 1.0 - smoothstep(0.15 * lam, 0.35 * lam, fp);
    if (rate <= 0.0 || fade <= 0.0) {
        return vec2<f32>(0.0);
    }
    let cover = 1.0 - exp(-SKY_W2_RING_LOBE.x * rate);
    var sl = vec2<f32>(0.0);
    for (var layer = 0; layer < 2; layer = layer + 1) {
        let o = f32(layer) * vec2<f32>(0.47, 0.71);
        let cell = floor(p / SKY_W2_RING_CELL + o);
        let s = cell * 1.37 + vec2<f32>(3.1, 7.9) * f32(layer + 1);
        let h = st_hash(s);
        let age = fract(skyu.wet.w / SKY_W2_RING.z + h);
        let live = step(st_hash(s + vec2<f32>(5.3, 1.7) + floor(skyu.wet.w / SKY_W2_RING.z + h)), cover);
        let c = (cell - o + 0.3 + 0.4 * vec2<f32>(st_hash(s + 9.2), st_hash(s + 4.4))) * SKY_W2_RING_CELL;
        let d = p - c;
        let r = length(d);
        let x = (r - age * SKY_W2_RING.z * SKY_W2_RING.y) / lam;
        let amp = SKY_W2_RING.w * (1.0 - age) * (1.0 - age) * exp(-0.5 * x * x) / (1.0 + r / 0.03);
        sl = sl + d / max(r, 1e-4) * sin(6.2831853 * x) * amp * live;
    }
    return sl * fade;
}

/// A puddle's lobe (sets `sky_w2_pud_alpha`, `sky_w2_ring`; returns its
/// roughness): the still water's alpha widened by the wind's unresolved
/// wavelets (their mean square slope adds to alpha squared), and in rain the
/// share of the water under rings, which reflects through a second, rough lobe.
fn sky_w2_lobe() -> f32 {
    let ws = min(length(skyu.wind.xy), SKY_W2_WAVELET_MSS.x);
    let a0 = SKY_W2_WATER_ROUGH * SKY_W2_WATER_ROUGH;
    let a = sqrt(a0 * a0 + SKY_W2_WAVELET_MSS.y * ws);
    sky_w2_pud_alpha = a;
    sky_w2_ring = select(0.0, 1.0 - exp(-SKY_W2_RING_LOBE.x * skyu.wet.z), skyu.wet.z > 0.0);
    return sqrt(a);
}

/// Still water's normal: the wind's capillary wavelets (a finite-difference
/// slope of animated value noise, faded with the footprint) and the rain's rings.
fn sky_w2_water_normal(p: vec2<f32>, n_geom: vec3<f32>, fp: f32) -> vec3<f32> {
    var n = n_geom;
    let wind = skyu.wind.xy;
    let ws = length(wind);
    let fade = 1.0 - smoothstep(0.02, 0.12, fp);
    if (fade > 0.0) {
        let q = p / SKY_W2_WAVELET.y + wind * (skyu.wet.w * 0.35);
        let h0 = st_noise(q);
        let hx = st_noise(q + vec2<f32>(0.35, 0.0));
        let hz = st_noise(q + vec2<f32>(0.0, 0.35));
        let amp = SKY_W2_WAVELET.x * (0.4 + 0.6 * min(ws, 4.0)) * fade / 0.35;
        n = normalize(n + vec3<f32>(hx - h0, 0.0, hz - h0) * amp);
    }
    let sl = sky_w2_rings(p, fp);
    return normalize(n + vec3<f32>(sl.x, 0.0, sl.y));
}

/// The street surface wet (`sky_wet_street` with the switch on).
fn sky_w2_street(s: ptr<function, Surface>, world: vec3<f32>, n_geom: vec3<f32>, kerb: f32, asphalt: bool) {
    var w = skyu.wet.x;
    let k = select(-1.0, kerb, asphalt);
    if (sky_on(skyu.flags.x, SKY_DAMP)) {
        w = w * sky_damp_left(world.xz, k); // (`--weather damp`) drying patches
    }
    var darken = select(0.66, 0.58, asphalt);
    var wet_rough = select(0.32, 0.24, asphalt);
    let cine = SKY_CINE_FULL && sky_on(skyu.flags.x, SKY_CINEMATIC);
    if (cine) {
        darken = select(SKY_CINE_WET_DARKEN.y, SKY_CINE_WET_DARKEN.x, asphalt);
        wet_rough = select(SKY_CINE_WET_ROUGH.y, SKY_CINE_WET_ROUGH.x, asphalt);
    }
    let fp = sky_w2_footprint(world, n_geom);
    var ph = vec2<f32>(0.0);
    if (n_geom.y > 0.95) {
        ph = sky_w2_puddle(world.xz, k, skyu.wet.y * select(SKY_W2_WALK_FILL, 1.0, asphalt), fp);
    }
    let pud = ph.x;
    // the halo is saturated ground: as wet as the film can be
    let soak = max(w, ph.y);
    if (cine && n_geom.y > 0.7) {
        sky_streak_w = max(soak, pud);
        sky_streak_pud = pud;
    }
    sky_w2_water = max(soak, pud);
    (*s).albedo = (*s).albedo * mix(1.0, darken, soak) * mix(1.0, SKY_W2_UNDER_WATER, pud);
    let v = normalize(skyu.eye.xyz - world);
    let graze = smoothstep(0.26, 0.05, abs(dot(n_geom, v))) * select(0.0, 1.0, n_geom.y > 0.95);
    let film = mix(wet_rough, 0.06, graze * 0.92);
    (*s).rough = mix(mix((*s).rough, film, soak), min(film, SKY_W2_WATER_ROUGH * 2.0), ph.y * 0.5);
    var n = normalize(mix((*s).n, n_geom, clamp(soak * select(0.70, 0.75, asphalt), 0.0, 1.0)));
    if (pud > 0.01) {
        (*s).rough = mix((*s).rough, sky_w2_lobe(), pud);
        n = normalize(mix(n, sky_w2_water_normal(world.xz, n_geom, fp), pud));
    }
    (*s).n = n;
}

/// Any other surface wet (`sky_wet_generic` with the switch on): open ground
/// and roofs as a porous walk with their own puddles, walls in the rain's runs,
/// soffits dry.
fn sky_w2_generic(s: ptr<function, Surface>, ng: vec3<f32>, world: vec3<f32>) {
    var w = skyu.wet.x;
    let porosity = clamp(((*s).rough - SKY_W2_POROSITY.x) / SKY_W2_POROSITY.y, 0.0, 1.0) * (1.0 - (*s).metal);
    let up = smoothstep(0.55, 0.9, ng.y);
    if (up > 0.0) {
        if (sky_on(skyu.flags.x, SKY_DAMP)) {
            w = w * sky_damp_left(world.xz, -1.0);
        }
        let fp = sky_w2_footprint(world, ng);
        var ph = vec2<f32>(0.0);
        if (ng.y > 0.95) {
            ph = sky_w2_puddle(world.xz, -1.0, skyu.wet.y * SKY_W2_WALK_FILL, fp);
        }
        let soak = max(w * up, ph.y);
        let pud = ph.x;
        if (SKY_CINE_FULL && sky_on(skyu.flags.x, SKY_CINEMATIC) && ng.y > 0.7) {
            sky_streak_w = max(soak, pud);
            sky_streak_pud = pud;
        }
        sky_w2_water = max(soak, pud);
        (*s).albedo = (*s).albedo * mix(1.0, mix(1.0, SKY_W2_GROUND.x, porosity), soak) * mix(1.0, SKY_W2_UNDER_WATER, pud);
        (*s).rough = mix((*s).rough, min((*s).rough, SKY_W2_GROUND.y), soak);
        var n = normalize(mix((*s).n, ng, clamp(soak * 0.7, 0.0, 1.0)));
        if (pud > 0.01) {
            (*s).rough = mix((*s).rough, sky_w2_lobe(), pud);
            n = normalize(mix(n, sky_w2_water_normal(world.xz, ng, fp), pud));
        }
        (*s).n = n;
        return;
    }
    if (ng.y < -0.3) {
        return; // a soffit: the rain does not reach it
    }
    // a wall: the rain's runs down it
    let t = normalize(vec2<f32>(-ng.z, ng.x) + vec2<f32>(1e-5, 0.0));
    let run = st_noise(vec2<f32>(dot(world.xz, t) * SKY_W2_RUNS.x, world.y * SKY_W2_RUNS.y + 5.3));
    // (an hour after the rain, `--weather damp`, the walls have half dried)
    let ww = w * mix(SKY_W2_WALL.x, SKY_W2_WALL.y, smoothstep(0.5, 0.8, run)) * select(1.0, 0.5, sky_on(skyu.flags.x, SKY_DAMP));
    (*s).albedo = (*s).albedo * mix(1.0, mix(1.0, SKY_W2_WALL.z, porosity), ww);
    (*s).rough = mix((*s).rough, min((*s).rough, SKY_W2_WALL.w), ww);
    sky_w2_water = ww * 0.5;
}

// ---- (`--screen-reflect`, the reflect track; sky.rs `SkyOptions::screen_reflect`,
// `SKY_SCREEN_REFLECT_BIT`; compiled out without SF_SCREEN_REFLECT, and into the
// street's pipelines only) ----
// The screen-space reflection, second generation, on glossy ground, car paint's
// clearcoat and glass:
// * the trace (`sky_sr_trace`): the ray's own path across the previous frame's
//   depth (the occlusion pass's half-resolution view depth, with the camera it
//   was made with), stepped on the screen (McGuire & Mara 2014): the ray's 1/w
//   and its depth over w are linear on the screen, so its depth at each step
//   is exact and every stretch of its path is looked at, near and far alike (a
//   march in metres, its steps growing, skips more of the screen the further it
//   goes: a reflected tower broke into stepped blocks, puddles' far images
//   into stairs). At most `SKY_SR_STEPS` steps and never more than one a
//   texel, denser toward the start (contact), the first a texel and a half
//   out, all jittered per pixel and frame (interleaved gradient noise,
//   golden-ratio steps) so the history accumulates what a fixed march draws as
//   bands. A step hits where the depth there lies within the ray's depth over
//   the step (passed by a small bias) and no more than the sample's assumed
//   thickness in front of it; bisections on the screen place the hit;
// * the filter: the surface's lobe at the hit seen through the mirror. A
//   lobe of half-width alpha spreads the reflected ray by 2 alpha; at the hit,
//   L along the ray, that is 2 alpha L, seen from the eye over the mirrored
//   path D + L: an angle of 2 alpha L / (D + L). So the reflection is sharp
//   where the reflected thing stands on the surface (a pole's foot in a puddle)
//   and long for far lamps, windows and signs (the wet film's anisotropic lobe;
//   a puddle's own, `sky_w2_lobe`, and in rain its rings' second lobe). Taps
//   on the jittered ellipse, one every 3 pixels of its long half-axis (2 to
//   20), averaged as they are (weighting bright taps down took the tail lamps
//   and signals out of the wet road), the mean capped at 24 x the exposure;
// * the open sky: a ray the trace does not stop reads the previous frame along
//   its own direction, the point at infinity, where that pixel is open sky or
//   scenery past 1.5-3 km (no parallax under a pixel there): the clouds, the
//   low sun's glow and the city's night glow as the frame draws them, not the
//   probes' analytic dome (puddles, car roofs, glass toward the sky). A
//   direction off the frame (a near puddle's view of the sky overhead) reads
//   the frame's edge toward it where that is sky (on ground facing up: a
//   pane's or a car flank's ray off the frame runs low into the facades,
//   which only the probes know), scaled by the weather dome's
//   gradient between the two directions (by its overcast weight the CIE
//   overcast's (1 + 2 sin el) / 3, held within 0.5-2) and trusted less with
//   the angle between them (fully within 0.3 rad, not past 0.9): the probes
//   keep only what the frame cannot say, so a puddle at the frame's foot is
//   never darker than its sky. The sky is read with one tap (smooth on the
//   scale of any lobe here);
// * glass (look_light `sky_sr_glass`, `sky_sr_glass_mix`): panes trace too, in
//   any orientation, and their clearcoat reads the trace where it finds
//   something (the facade across the street, the street, the sky) over the
//   probe grid.
const SKY_SCREEN_REFLECT: u32 = 16777216u;
const SKY_SR_FULL: bool = SF_SCREEN_REFLECT && LK_SHADOW_CASCADES && LK_SHADOW_FAR;
/// The filter's taps: one every `SKY_SR_TAP_PX` pixels of the lobe's long
/// half-axis, at least x and at most y (a sharp lobe takes few, a long streak
/// many: its noise falls as the root of their count).
const SKY_SR_TAPS: vec2<i32> = vec2<i32>(2, 20);
const SKY_SR_TAP_PX: f32 = 3.0;
/// The trace: its most steps, the power that packs them toward the start,
/// the bisections that place a hit.
const SKY_SR_STEPS: i32 = 40;
const SKY_SR_SPACING: f32 = 1.6;
const SKY_SR_REFINE: i32 = 5;
/// The ray's reach (m) and the nearest view depth it is followed to (m).
const SKY_SR_REACH_M: f32 = 800.0;
const SKY_SR_NEAR_M: f32 = 0.25;
/// A depth sample's assumed thickness, x + y x its depth (m), and the margin a
/// ray must pass behind it by to count, x + y x its depth (m: the
/// half-resolution texel's own depth slope on ground seen low).
const SKY_SR_THICK: vec2<f32> = vec2<f32>(0.4, 0.04);
const SKY_SR_BIAS: vec2<f32> = vec2<f32>(0.03, 0.004);
/// Scenery past this view distance (m) counts as open sky for the direction
/// lookup (fully past the second).
const SKY_SR_FAR_M: vec2<f32> = vec2<f32>(1500.0, 3000.0);
/// A direction off the frame: the frame's edge read this far in (uv), trusted
/// fully within 0.3 rad of it (cosine x) and not at all past 0.9 rad (y), the
/// dome's gradient between them held within z..w.
const SKY_SR_EDGE_IN: f32 = 0.01;
/// Only a surface facing up past this normal y extends the frame's sky (its
/// ray off the frame climbs over the street; a pane's runs into the facades).
const SKY_SR_EXTEND_NY: f32 = 0.6;
const SKY_SR_OFF_FRAME: vec4<f32> = vec4<f32>(0.9553365, 0.6216100, 0.5, 2.0);
/// (look_light) the surface being shaded is glass (set before `sky_ssr`).
var<private> sky_sr_glass: bool;

fn sky_sr_on() -> bool {
    return SKY_SR_FULL && sky_on(skyu.flags.x, SKY_SCREEN_REFLECT);
}

/// (look_light) The glass clearcoat's environment `rad` (x `env_i` there) with
/// the trace's radiance `ssr` over it where it found something.
fn sky_sr_glass_mix(rad: vec3<f32>, ssr: vec4<f32>, env_i: f32) -> vec3<f32> {
    if (!sky_sr_on() || !sky_sr_glass) {
        return rad;
    }
    return mix(rad, ssr.rgb / max(env_i, 1e-3), ssr.a);
}

/// The previous frame round `uv`: `count` taps on the ellipse of half-axes
/// `s` (uv), jittered by `j`, `j2`, Gaussian-weighted.
fn sky_sr_taps(uv: vec2<f32>, s: vec2<f32>, count: i32, j: f32, j2: f32) -> vec3<f32> {
    var c = vec3<f32>(0.0);
    var wsum = 0.0;
    for (var k = 0; k < count; k = k + 1) {
        let o = (f32(k) + j) / f32(count) * 2.0 - 1.0;
        let ox = fract(j2 + f32(k) * 0.618034) * 2.0 - 1.0;
        let ut = clamp(uv + vec2<f32>(ox * s.x, o * s.y), vec2<f32>(0.0), vec2<f32>(1.0));
        let wk = exp(-2.0 * (o * o + ox * ox));
        c = c + textureSampleLevel(sky_prev, lk_ao_samp, ut, 0.0).rgb * wk;
        wsum = wsum + wk;
    }
    return c / max(wsum, 1e-6);
}

/// The taps an ellipse of half-axes `s` (uv) takes: one every
/// `SKY_SR_TAP_PX` pixels along its long axis, `SKY_SR_TAPS.x` to `.y`.
fn sky_sr_tap_count(s: vec2<f32>) -> i32 {
    let long = max(s.x * skyu.res.x, s.y * skyu.res.y);
    return clamp(i32(ceil(long / SKY_SR_TAP_PX)), SKY_SR_TAPS.x, SKY_SR_TAPS.y);
}

/// The lobe's reflection round `uv`: half-axes `s` (uv) of the surface's lobe,
/// `sr` of the rain rings' second lobe at weight `ring`; capped at 24 x the exposure.
fn sky_sr_lobe(uv: vec2<f32>, s: vec2<f32>, sr: vec2<f32>, ring: f32, j: f32, j2: f32) -> vec3<f32> {
    var c = vec3<f32>(0.0);
    let lobes = select(1, 2, ring > 0.0);
    for (var lb = 0; lb < lobes; lb = lb + 1) {
        let second = lb == 1;
        let e = select(s, sr, second);
        let wl = select(select(1.0, 1.0 - ring, lobes == 2), ring, second);
        c = c + wl * sky_sr_taps(uv, e, sky_sr_tap_count(e), select(j, j2, second), select(j2, j, second));
    }
    let cap = SKY_CINE_SSR_CAP / max(look.key.w, 0.1);
    return c * min(1.0, cap / max(lk_luma(c), 1e-4));
}

/// The frame's edge fade over `b` of it (the trace's hits: 0.05).
fn sky_sr_edge(q: vec2<f32>, b: f32) -> f32 {
    return smoothstep(0.0, b, min(min(q.x, 1.0 - q.x), min(q.y, 1.0 - q.y)));
}

/// The previous frame's view depth (m, <= 0 nothing) at texel `s`.
fn sky_sr_depth(s: vec2<f32>, top: vec2<i32>) -> f32 {
    return textureLoad(lk_ao_z, clamp(vec2<i32>(s), vec2<i32>(0), top), 0).r;
}

/// The trace (see above) from `p0` along `r`, jittered by `j`: the hit's uv,
/// its distance along the ray (m) and its weight (0: none; the thickness
/// test's fade).
fn sky_sr_trace(p0: vec3<f32>, r: vec3<f32>, j: f32) -> vec4<f32> {
    let dims = vec2<f32>(textureDimensions(lk_ao_z));
    let top = vec2<i32>(dims) - 1;
    let fwd = look.ao_fwd.xyz;
    let d0 = dot(p0 - look.ao_cam.xyz, fwd);
    let dr = dot(r, fwd);
    var reach = SKY_SR_REACH_M;
    if (dr < -1e-4) {
        reach = min(reach, (SKY_SR_NEAR_M - d0) / dr);
    }
    let c0 = look.ao_vp * vec4<f32>(p0, 1.0);
    let c1 = look.ao_vp * vec4<f32>(p0 + r * reach, 1.0);
    if (reach < 0.1 || c0.w <= 1e-3 || c1.w <= 1e-3) {
        return vec4<f32>(0.0);
    }
    // 1 / w and depth / w, linear on the screen
    let k0 = 1.0 / c0.w;
    let k1 = 1.0 / c1.w;
    let q0 = d0 * k0;
    let q1 = (d0 + dr * reach) * k1;
    let s0 = vec2<f32>(c0.x * k0 * 0.5 + 0.5, 0.5 - c0.y * k0 * 0.5) * dims;
    let s1 = vec2<f32>(c1.x * k1 * 0.5 + 0.5, 0.5 - c1.y * k1 * 0.5) * dims;
    let ds = s1 - s0;
    // the part of the path on the texture
    var fe = 1.0;
    if (s1.x < 0.0) {
        fe = min(fe, -s0.x / ds.x);
    }
    if (s1.x > dims.x) {
        fe = min(fe, (dims.x - s0.x) / ds.x);
    }
    if (s1.y < 0.0) {
        fe = min(fe, -s0.y / ds.y);
    }
    if (s1.y > dims.y) {
        fe = min(fe, (dims.y - s0.y) / ds.y);
    }
    let len = length(ds);
    let span = len * max(fe, 0.0);
    if (span < 2.0) {
        return vec4<f32>(0.0);
    }
    let n = min(f32(SKY_SR_STEPS), ceil(span));
    var f_prev = 0.0;
    var z_prev = d0;
    var f_hit = -1.0;
    for (var i = 0; i < SKY_SR_STEPS; i = i + 1) {
        if (f32(i) >= n) {
            break;
        }
        let f = (1.5 + (span - 1.5) * pow((f32(i) + j) / n, SKY_SR_SPACING)) / len;
        let k = mix(k0, k1, f);
        let zr = mix(q0, q1, f) / k;
        let z = sky_sr_depth(s0 + ds * f, top);
        if (z > 0.0 && max(zr, z_prev) > z + SKY_SR_BIAS.x + SKY_SR_BIAS.y * z && min(zr, z_prev) < z + SKY_SR_THICK.x + SKY_SR_THICK.y * z) {
            f_hit = f;
            break;
        }
        f_prev = f;
        z_prev = zr;
    }
    if (f_hit < 0.0) {
        return vec4<f32>(0.0);
    }
    var lo = f_prev;
    var hi = f_hit;
    for (var i = 0; i < SKY_SR_REFINE; i = i + 1) {
        let mid = 0.5 * (lo + hi);
        let z = sky_sr_depth(s0 + ds * mid, top);
        if (z > 0.0 && mix(q0, q1, mid) / mix(k0, k1, mid) > z) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    let k = mix(k0, k1, hi);
    let z = sky_sr_depth(s0 + ds * hi, top);
    let thick = SKY_SR_THICK.x + SKY_SR_THICK.y * z;
    let solid = 1.0 - smoothstep(thick, 2.0 * thick, mix(q0, q1, hi) / k - z);
    if (z <= 0.0 || solid <= 0.0) {
        return vec4<f32>(0.0);
    }
    // the distance along the ray, perspective-correct
    return vec4<f32>((s0 + ds * hi) / dims, reach * k1 * hi / k, solid);
}

/// The reflection with the switch on (see above): rgb radiance, a weight.
fn sky_sr_ssr(world: vec3<f32>, n: vec3<f32>, v: vec3<f32>, rough: f32) -> vec4<f32> {
    if (!sky_on(skyu.flags.x, SKY_SSR) || skyu.flags.z == 0u || look.ao_params.x < 0.5 || rough > SKY_CINE_SSR_ROUGH.y || frame.copy_step.z > 0.5) {
        return vec4<f32>(0.0);
    }
    let glass = sky_sr_glass;
    if (n.y < 0.6 && sky_coat <= 0.0 && !glass) {
        return vec4<f32>(0.0);
    }
    let r = reflect(-v, n);
    // this pixel and frame's jitter (interleaved gradient noise, golden-ratio steps a frame)
    let c0 = skyu.vp * vec4<f32>(world, 1.0);
    let px = (vec2<f32>(c0.x, -c0.y) / max(c0.w, 1e-4) * 0.5 + 0.5) * skyu.res.xy;
    let fi = f32(skyu.flags.y % 64u);
    let j = fract(sky_ign(px) + fi * 0.618034);
    let j2 = fract(sky_ign(px + vec2<f32>(37.0, 11.0)) + fi * 0.754878);
    // the lobe's half-widths (alpha): the wet film's anisotropic one where the
    // street is wet, a puddle's own (`--wet-surfaces`: the wind's wavelets),
    // and in rain the rings' second lobe
    let a_pud = max(SKY_CINE_PUDDLE_ALPHA, sky_w2_pud_alpha);
    let a_along = mix(rough * rough, mix(SKY_CINE_STREAK_ALONG, a_pud, sky_streak_pud), sky_streak_w);
    let a_across = mix(rough * rough, mix(SKY_CINE_STREAK_ACROSS, a_pud, sky_streak_pud), sky_streak_w);
    let tn = max(skyu.cluster.z, 0.1);
    let tw = max(skyu.cluster.z * skyu.cluster.w, 0.1);
    let ang = vec2<f32>(a_across / tw, a_along / tn);
    let ang_r = vec2<f32>(SKY_W2_RING_LOBE.y / tw, SKY_W2_RING_LOBE.y / tn);
    let ring = sky_w2_ring * sky_streak_pud;
    let fade = 1.0 - smoothstep(SKY_CINE_SSR_ROUGH.x, SKY_CINE_SSR_ROUGH.y, rough);
    let range = length(world - skyu.eye.xyz);
    var hit_c = vec3<f32>(0.0);
    var hit_w = 0.0;
    let h = sky_sr_trace(world, r, j);
    if (h.w > 0.0) {
        // the lobe at the hit seen through the mirror: 2 alpha L / (D + L) each way
        let contact = h.z / (range + h.z);
        hit_c = sky_sr_lobe(h.xy, ang * contact, ang_r * contact, ring, j, j2);
        hit_w = sky_sr_edge(h.xy, 0.05) * h.w;
    }
    // the open sky along the ray's own direction (the point at infinity); a
    // direction off the frame reads the frame's edge toward it (the sky as
    // the frame draws it, where the edge is sky), scaled by the dome's own
    // gradient between the two and trusted less the further it is
    var sky_c = vec3<f32>(0.0);
    var sky_w = 0.0;
    if (hit_w < 1.0) {
        let clip = look.ao_vp * vec4<f32>(r, 0.0);
        if (clip.w > 1e-4) {
            let q = vec2<f32>(clip.x / clip.w * 0.5 + 0.5, 0.5 - clip.y / clip.w * 0.5);
            let qc = clamp(q, vec2<f32>(SKY_SR_EDGE_IN), vec2<f32>(1.0 - SKY_SR_EDGE_IN));
            let dims = vec2<f32>(textureDimensions(lk_ao_z));
            let z = sky_sr_depth(qc * dims, vec2<i32>(dims) - 1);
            var open = select(smoothstep(SKY_SR_FAR_M.x, SKY_SR_FAR_M.y, z), 1.0, z <= 0.0);
            var gain = 1.0;
            if (n.y < SKY_SR_EXTEND_NY) {
                // a pane's or a flank's ray off the frame runs low along the
                // street, where the frame cannot say what blocks it: the probes
                open = open * select(sky_sr_edge(q, 0.15), 0.0, any(q != qc));
            } else if (open > 0.0 && any(q != qc)) {
                let tn = skyu.cluster.z;
                let rc = normalize(skyu.cam_fwd.xyz + skyu.cam_right.xyz * ((2.0 * qc.x - 1.0) * tn * skyu.cluster.w) + skyu.cam_up.xyz * ((1.0 - 2.0 * qc.y) * tn));
                open = open * smoothstep(SKY_SR_OFF_FRAME.y, SKY_SR_OFF_FRAME.x, dot(r, rc));
                // the weather's dome (lk_sky_upper): toward the CIE overcast
                // gradient by its overcast weight, toward flat by its isotropy
                let cie = (1.0 + 2.0 * max(r.y, 0.0)) / (1.0 + 2.0 * max(rc.y, 0.0));
                gain = clamp(mix(mix(1.0, cie, look.wx.x), 1.0, look.wx.y), SKY_SR_OFF_FRAME.z, SKY_SR_OFF_FRAME.w);
            }
            if (open > 0.0) {
                // one tap: the sky is smooth on the scale of the lobe
                let c = textureSampleLevel(sky_prev, lk_ao_samp, qc, 0.0).rgb * gain;
                sky_c = c * min(1.0, SKY_CINE_SSR_CAP / max(look.key.w, 0.1) / max(lk_luma(c), 1e-4));
                sky_w = open;
            }
        }
    }
    let w = hit_w + sky_w * (1.0 - hit_w);
    if (w <= 0.0) {
        return vec4<f32>(0.0);
    }
    let c = (hit_c * hit_w + sky_c * sky_w * (1.0 - hit_w)) / w;
    return vec4<f32>(c, w * fade);
}
