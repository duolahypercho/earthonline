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

// ---- A low beam's stray light high over its cut-off (`--no-headlamp-stray`:
// the glare zone's value at every height, as before; sky_reflect.rs
// `sky_light_source`). FMVSS 108 Table XIX (every US lower-beam type): at
// most 125 cd from 10 deg up to the zenith; compliant lamps measure 37 cd (an
// NHTSA compliance test) to 104 cd (a retail LB2V lamp's report), so their
// geometric mean, 62 cd, of the 20,000 cd peak (eo-sky photometry
// `LOW_BEAM_PEAK_CD`). Up to 4 deg the glare zone's value stands (ECE R112
// zone III, 625 cd; FMVSS 108's 4U points ask at least 64 cd), falling
// between. A car at a stop line lit the signal head over it at 620 cd.
const SKY_HEAD_STRAY_ON: bool = true;
/// x..y deg over the horizontal the fall's ends; z the stray share of the peak.
const SKY_HEAD_STRAY: vec3<f32> = vec3<f32>(4.0, 10.0, 0.0031);

/// The low beam's share of its peak over the cut-off: `glare` near it, the
/// stray share high over it (`dep` deg below the horizontal).
fn sky_head_glare(glare: f32, dep: f32) -> f32 {
    if (!SKY_HEAD_STRAY_ON) {
        return glare;
    }
    return mix(glare, SKY_HEAD_STRAY.z, smoothstep(SKY_HEAD_STRAY.x, SKY_HEAD_STRAY.y, -dep));
}

/// The low beam's vertical and horizontal shape (eo-sky `low_beam_shape`):
/// `dep` deg below the horizontal, `h` deg right of the axis.
fn sky_cine_low_beam(dep: f32, h: f32) -> f32 {
    let cut = mix(SKY_CINE_BEAM_CUT.x, SKY_CINE_BEAM_CUT.y, smoothstep(0.0, SKY_CINE_BEAM_STEP, h));
    let under = smoothstep(cut, cut + SKY_CINE_BEAM_EDGE, dep);
    let x = (h - SKY_CINE_BEAM_HOT.x) / SKY_CINE_BEAM_HOT.y;
    let hot = SKY_CINE_BEAM_SPREAD + (1.0 - SKY_CINE_BEAM_SPREAD) * exp(-x * x);
    let fall = select(1.0, (2.5 / max(dep, 1e-3)) * (2.5 / max(dep, 1e-3)), dep > 2.5);
    return mix(sky_head_glare(SKY_CINE_BEAM_GLARE, dep), hot * fall, under);
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
    // (`--lamp-streaks`) the cars' rear lamps appended to the list (`sky_ls_lamps`; none without)
    let count = min(sky_clusters[base], SKY_STREAK_CAP + sky_ls_lamps());
    // the tangent along the ground toward the eye (the streak's length)
    var t = v - n * dot(v, n);
    if (dot(t, t) < 1e-8) {
        t = cross(n, vec3<f32>(1.0, 0.0, 0.0));
    }
    t = normalize(t);
    // (`--wet-surfaces`) a puddle's own lobe, and in rain its rings' (`sky_w2_lobe`; 0 without)
    let a_pud = select(SKY_CINE_PUDDLE_ALPHA, max(SKY_CINE_PUDDLE_ALPHA, sky_w2_pud_alpha), SKY_W2_FULL);
    // (`--wet-relief`) the film's lobes as smooth as the surface under it (`sky_wr_film_k`; 1 without)
    let a_k = select(1.0, sky_wr_alpha_k, SKY_WR_FULL);
    let a_along = mix(SKY_CINE_STREAK_ALONG * a_k, a_pud, sky_streak_pud);
    let a_across = mix(SKY_CINE_STREAK_ACROSS * a_k, a_pud, sky_streak_pud);
    let ring_w = select(0.0, sky_w2_ring * sky_streak_pud, SKY_W2_FULL);
    let r_along = mix(SKY_CINE_STREAK_ALONG, SKY_W2_RING_LOBE.y, sky_streak_pud);
    let r_across = mix(SKY_CINE_STREAK_ACROSS, SKY_W2_RING_LOBE.y, sky_streak_pud);
    let lobes = select(1, 2, SKY_W2_FULL && ring_w > 0.0);
    // Subtle asphalt micro-aggregate slope breakup on non-puddle wet film so
    // night taillight and streetlamp streaks shimmer organically over coarse road grain.
    var ns = n;
    let film_grain = 1.0 - sky_streak_pud;
    if (film_grain > 0.02 && n.y > 0.7) {
        let dist_s = length(world - skyu.eye.xyz);
        let fp_s = dist_s * 2.0 * skyu.cluster.z / max(skyu.res.y, 1.0) / max(abs(dot(n, v)), 0.08);
        let gk = (1.0 - smoothstep(0.015, 0.14, fp_s)) * film_grain;
        if (gk > 0.01) {
            let p1 = world.xz * 7.5;
            let p2 = world.xz * 19.0 + vec2<f32>(4.3, 11.7);
            let gx = (st_noise(p1 + vec2<f32>(0.25, 0.0)) - st_noise(p1)) * 0.65
                + (st_noise(p2 + vec2<f32>(0.25, 0.0)) - st_noise(p2)) * 0.35;
            let gz = (st_noise(p1 + vec2<f32>(0.0, 0.25)) - st_noise(p1)) * 0.65
                + (st_noise(p2 + vec2<f32>(0.0, 0.25)) - st_noise(p2)) * 0.35;
            ns = normalize(n + vec3<f32>(gx, 0.0, gz) * (0.14 * gk));
        }
    }
    var out = vec3<f32>(0.0);
    for (var k = 0u; k < count; k = k + 1u) {
        let li = sky_lights[sky_clusters[base + 1u + k]];
        let d = li.pos - world;
        let d2 = max(dot(d, d), 0.04);
        let dist = sqrt(d2);
        let l = d / dist;
        if (dot(ns, l) <= 0.0) {
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
            if (SF_CAR_LAMPS && li.kind == SF_REAR_LAMP_KIND) {
                shape = sf_rear_lamp_shape(-l, li.dir); // (`--car-lamps`) a car's rear lamp's field (agents.rs)
            }
            if ((SF_SIGNAL_SPILL || SF_REAR_LAMPS) && (li.kind == NT_SIGNAL_KIND || li.kind == NT_REAR_KIND)) {
                // (`--signal-spill`, `--rear-lamps`) its lobe toward the film
                shape = nt_lobe(-l, li.dir, select(NT_REAR_V, NT_SIGNAL_V, li.kind == NT_SIGNAL_KIND), select(NT_REAR_H, NT_SIGNAL_H, li.kind == NT_SIGNAL_KIND));
            }
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
            f = f + wl * sky_cine_film(ns, v, l, t, min(select(a_along, r_along, second) + wide, 1.0), min(select(a_across, r_across, second) + wide, 1.0));
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

// sf:LEAF_SHADE+
// (`--leaf-shade`, set by props_draw.wgsl on a palm's or a street tree's
// pixels; w 0 everywhere else) the crown the point stands in: its centre (xyz)
// and radius (w, m), and its leaf area's extinction (x per metre, uniform in
// the crown; y m, its radial part's C in sigma(r) = C / r^2: the fronds of a
// palm all leave the heart, so their leaf area per volume falls as the square
// of the distance from it).
var<private> sky_leaf: vec4<f32> = vec4<f32>(0.0);
var<private> sky_leaf_k: vec2<f32> = vec2<f32>(0.0);
// true in the props' module alone (look.rs `props_city_source`): every other
// pipeline's lights' loop compiles as before, its registers untouched.
const SKY_LEAF_ON: bool = false;

/// (`--leaf-shade`) The share of a light at `lp` that reaches `p` through the
/// crown's leaves (Beer-Lambert over the segment's optical depth inside the
/// crown: the uniform part's length, and the radial part's C / b (atan(t1 / b)
/// - atan(t0 / b)), b the segment line's distance from the centre, at least a
/// tenth of the radius: the heart and the trunk's top).
fn sky_leaf_t(p: vec3<f32>, lp: vec3<f32>) -> f32 {
    let d = lp - p;
    let len = max(length(d), 1e-4);
    let w = d / len;
    let oc = p - sky_leaf.xyz;
    let b = dot(oc, w);
    let disc = b * b - (dot(oc, oc) - sky_leaf.w * sky_leaf.w);
    if (disc <= 0.0) {
        return 1.0;
    }
    let r = sqrt(disc);
    let t0 = max(-b - r, 0.0);
    let t1 = min(-b + r, len);
    if (t1 <= t0) {
        return 1.0;
    }
    var tau = sky_leaf_k.x * (t1 - t0);
    if (sky_leaf_k.y > 0.0) {
        // atan(u1) - atan(u0) in one atan (u1 > u0: the difference lies in (0, pi))
        let bl = max(sqrt(max(dot(oc, oc) - b * b, 0.0)), 0.1 * sky_leaf.w);
        let u1 = (t1 + b) / bl;
        let u0 = (t0 + b) / bl;
        let q = 1.0 + u1 * u0;
        tau = tau + sky_leaf_k.y / bl * (atan((u1 - u0) / q) + select(0.0, 3.14159265, q < 0.0));
    }
    return exp(-tau);
}
// sf:end
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
                vert = sky_head_glare(0.031, dep); // high over the cut-off, the stray light
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
            if (SF_CAR_LAMPS && li.kind == SF_REAR_LAMP_KIND) {
                shape = sf_rear_lamp_shape(-l, li.dir); // (`--car-lamps`) a car's rear lamp's field (agents.rs)
            }
            if ((SF_SIGNAL_SPILL || SF_REAR_LAMPS) && (li.kind == NT_SIGNAL_KIND || li.kind == NT_REAR_KIND)) {
                // (`--signal-spill`, `--rear-lamps`) a signal lens's or rear lamp's lobe (eo-sky
                // lights `SIGNAL_LOBE`, `REAR_LOBE`); a signal's beam lights nothing nearer
                // it than NT_SIGNAL_CLEAR_M (what stood there would hide the lens).
                let sig = li.kind == NT_SIGNAL_KIND;
                shape = nt_lobe(-l, li.dir, select(NT_REAR_V, NT_SIGNAL_V, sig), select(NT_REAR_H, NT_SIGNAL_H, sig)) * select(1.0, smoothstep(0.75 * NT_SIGNAL_CLEAR_M, NT_SIGNAL_CLEAR_M, dist), sig);
            }
        }
        let nl = max(dot(n, l), 0.0);
        var e = li.intensity * shape * w * w / d2 * nl;
        if (LS_SHADER && li.pad != 0.0 && e > 0.0) {
            e = e * ao;
            if (li.pad >= 1.0) {
                e = e * ls_light_visibility(u32(li.pad) - 1u, world, n);
            }
        }
// sf:LEAF_SHADE+
        if (SKY_LEAF_ON && sky_leaf.w > 0.0 && e > 0.0 && li.pad < 1.0) {
            e = e * sky_leaf_t(world, li.pos); // (`--leaf-shade`) a light with no map of its own, through the crown's leaves
        }
// sf:end
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

// sf:WEATHER_FX_FLOOR+
/// (`--weather-fx`) Raindrop impact splash & droplet sparkle intensity for the
/// current floor fragment, written by `sky_wet_street` / `sky_wet_generic_at` and
/// consumed + cleared by `look_light`.
var<private> sky_rain_spark: f32 = 0.0;

/// (`--weather-fx`) Standing water pooling across gutters, carriageways,
/// sidewalks and horizontal floors as rainwater accumulates (`skyu.wet.y` +
/// current rain rate). Returns (puddle_mask, water_depth, soaked_rim).
fn sky_puddle_fx(p: vec2<f32>, kerb: f32, asphalt: bool) -> vec3<f32> {
    let fill = clamp(skyu.wet.y + smoothstep(2.5, 20.0, skyu.wet.z) * 0.14, 0.0, 1.0);
    if (fill <= 0.005) {
        return vec3<f32>(0.0);
    }
    let n_lg = st_noise(p / 4.2 + vec2<f32>(7.3, 19.1));
    let n_md = st_noise(p / 1.55 + vec2<f32>(31.7, 4.9));
    let n_sm = st_noise(p / 0.52 + vec2<f32>(11.4, 27.3));
    let topo = 0.50 * n_lg + 0.35 * n_md + 0.15 * n_sm;
    let gutter = select(0.0, sky_drain(kerb) * 0.38, asphalt && kerb >= 0.0);
    let basin = topo + gutter;
    let thr = mix(0.92, 0.45, pow(fill, 0.52)) + select(0.05, 0.02, asphalt);
    let pud = smoothstep(thr - 0.035, thr + 0.035, basin);
    let depth = clamp((basin - thr) / 0.28, 0.0, 1.0) * pud;
    let rim = smoothstep(thr - 0.14, thr - 0.01, basin) * (1.0 - pud);
    return vec3<f32>(pud, depth, rim);
}

/// (`--weather-fx`) Subtle wind-driven capillary ripples drifting downwind
/// across deeper standing puddles, faded out on grazing far views.
fn sky_puddle_wind(p: vec2<f32>, t: f32, fp_g: f32) -> vec2<f32> {
    let ws = length(skyu.wind.xy);
    let vis = 1.0 - smoothstep(0.04, 0.14, fp_g);
    if (ws < 1.5 || vis <= 0.001) {
        return vec2<f32>(0.0);
    }
    let wd = skyu.wind.xy / ws;
    let perp = vec2<f32>(-wd.y, wd.x);
    let along = dot(p, wd);
    let across = dot(p, perp);
    let gain = smoothstep(1.5, 11.0, ws) * 0.024 * vis;
    let p1 = along * 28.0 - t * (5.5 + ws * 0.9) + sin(across * 6.5) * 1.4;
    let p2 = along * 46.0 - t * (8.5 + ws * 1.3) - sin(across * 11.0) * 1.1;
    let dw = (cos(p1) * 0.65 + cos(p2) * 0.35) * gain;
    return wd * dw + perp * (sin(p1 * 0.7 + across * 8.0) * gain * 0.25);
}

/// (`--weather-fx`) Raindrops impacting standing puddles and wet floors:
/// three staggered scales of expanding capillary ripple rings on standing water,
/// plus delicate impact splash crowns and parallaxed rebounding droplet sparkles
/// on both puddles and wet pavement. Uses the grazing ground footprint (`fp_g`)
/// so distant low-angle roadways stay mirror-smooth with zero ripple aliasing.
/// Returns (slope_x, slope_z, sparkle).
fn sky_rain_floor_fx(world: vec3<f32>, pud: f32, w: f32, v: vec3<f32>) -> vec3<f32> {
    let rate = skyu.wet.z;
    if (rate <= 0.1 || !sky_on(skyu.flags.x, SKY_RAIN)) {
        return vec3<f32>(0.0);
    }
    let dist = length(world - skyu.eye.xyz);
    let fp = max(dist * 2.0 * skyu.cluster.z * skyu.res.w, 1e-4);
    let fp_g = fp / max(abs(v.y), 0.14);
    if (fp_g > 0.24) {
        return vec3<f32>(0.0);
    }
    let rain = smoothstep(0.15, 7.0, rate);
    let heavy_r = smoothstep(5.0, 20.0, rate);
    let t = skyu.wet.w;
    let p = world.xz;
    let view_par = v.xz / max(abs(v.y), 0.26);
    // Natural rain intensity spectrum + passing spatial shower bands across city blocks.
    let shower = 0.84 + 0.22 * sin(p.x * 0.031 + p.y * 0.023 - t * 0.45) * cos(p.x * 0.019 - p.y * 0.037 - t * 0.31);
    let density = clamp((0.18 + 0.50 * rain + 0.26 * heavy_r) * shower, 0.08, 0.96);
    let vigor = mix(0.75, 1.35, heavy_r);
    var g = vec2<f32>(0.0);
    var spark = 0.0;
    for (var k = 0u; k < 3u; k = k + 1u) {
        let sc = select(select(2.2, 3.6, k == 1u), 5.6, k == 2u);
        let freq = select(select(1.12, 1.58, k == 1u), 2.25, k == 2u) * mix(0.85, 1.25, heavy_r);
        let ring_vis = 1.0 - smoothstep(0.028 / (sc / 2.2), 0.095 / (sc / 2.2), fp_g);
        let drop_vis = 1.0 - smoothstep(0.055, 0.20, fp_g);
        if (ring_vis <= 0.001 && drop_vis <= 0.001) {
            continue;
        }
        let q = p * sc + vec2<f32>(f32(k) * 0.43, f32(k) * 0.67);
        let cell = floor(q);
        let h = st_hash(cell * 1.7 + vec2<f32>(f32(k) * 19.3 + 3.1, f32(k) * 7.7 + 1.9));
        let h2 = fract(h * 17.31);
        let h3 = fract(h * 43.7);
        if (fract(h * 89.3) > density) {
            continue;
        }
        let centre = cell + vec2<f32>(0.30 + 0.40 * h, 0.30 + 0.40 * h2);
        let phase = fract(t * freq + h * 7.3 + f32(k) * 0.37);
        let d_m = (q - centre) / sc;
        let r_m = length(d_m);
        // Keep every expanding capillary ring strictly within its 0.30/sc cell margin with a smooth
        // circular boundary mask so rings never clip into artificial square tiles.
        let cell_r = 0.28 / sc;
        let cell_fade = 1.0 - smoothstep(cell_r * 0.62, cell_r, r_m);
        let onset = smoothstep(0.0, 0.08, phase);
        let size = (0.70 + 0.50 * h2) * mix(0.88, 1.12, heavy_r);
        let travel = min(0.115 * size, cell_r * 0.85);
        // Gentle, organic capillary ripples on standing water (and subtle dimple on wet film).
        if (ring_vis > 0.001 && cell_fade > 0.001) {
            if (pud > 0.02) {
                let lam = 0.016 * (0.85 + 0.30 * h2);
                let x = (r_m - phase * travel) / lam;
                let env = exp(-x * x * 0.45) * onset * (1.0 - phase) * (1.0 - phase) * cell_fade * ring_vis * pud;
                g = g + (d_m / max(r_m, 1e-4)) * (sin(6.2831853 * x) * env * 0.18 * vigor);
            } else if (phase < 0.14) {
                let x = (r_m - phase * min(0.045 * size, cell_r * 0.6)) / 0.012;
                let env = exp(-x * x * 0.55) * onset * (1.0 - phase / 0.14) * cell_fade * ring_vis * w * 0.05 * vigor;
                g = g + (d_m / max(r_m, 1e-4)) * (sin(6.2831853 * x) * env);
            }
        }
        // Understated splash glint on standing puddles (avoid bright painted white rings on asphalt).
        let crown_dur = mix(0.06, 0.085, pud);
        let jet_end = mix(0.11, 0.16, pud);
        if (drop_vis > 0.001 && phase < jet_end && pud > 0.05) {
            let px_m = max(fp * 0.7, 0.0035);
            var sp = 0.0;
            if (phase < crown_dur) {
                let ct = phase / crown_dur;
                let cr_r = (0.003 + ct * 0.016) * size;
                let ang = atan2(d_m.y, d_m.x);
                let asym = 0.65 + 0.35 * sin(ang * 3.0 + h * 23.0) * cos(ang * 5.0 - h2 * 17.0);
                let rw = max(px_m * 0.95, 0.0032);
                let dr = (r_m - cr_r) / rw;
                sp = sp + exp(-dr * dr) * sin(ct * 3.14159) * asym * 0.18 * pud;
            }
            if (phase > 0.015 && phase < jet_end) {
                let jt = (phase - 0.015) / (jet_end - 0.015);
                let h_jet_m = 0.030 * size * sin(jt * 3.14159);
                let p_jet = d_m + view_par * h_jet_m;
                let bw = max(px_m * 0.85, 0.0038);
                sp = sp + exp(-dot(p_jet, p_jet) / (bw * bw)) * sin(jt * 3.14159) * 0.36 * pud;
            }
            let sub_att = clamp(0.008 / max(px_m, 0.008), 0.25, 1.0);
            spark = spark + sp * drop_vis * sub_att * vigor * (0.55 + 0.45 * h3);
        }
    }
    return vec3<f32>(g.x, g.y, spark * rain * max(w, pud));
}
// sf:end

/// Wet the street surface (the carriageway and the walk), before lighting.
fn sky_wet_street(s: ptr<function, Surface>, world: vec3<f32>, n_geom: vec3<f32>, kerb: f32, asphalt: bool) {
// sf:WEATHER_FX_FLOOR+
    sky_rain_spark = 0.0;
// sf:end
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
// sf:WEATHER_FX_FLOOR+
    let fx = sky_weather_fx_on(skyu.cirrus);
    var pud = 0.0;
    var pud_depth = 0.0;
    var pud_rim = 0.0;
    if (n_geom.y > 0.95) {
        if (fx) {
            let pf = sky_puddle_fx(world.xz, kerb, asphalt);
            pud = pf.x * w;
            pud_depth = pf.y * w;
            pud_rim = pf.z * w;
        } else {
            pud = sky_puddle(world.xz, select(-1.0, kerb, asphalt)) * w;
        }
        if (SF_STREET_PBR) {
            pud = max(pud, sp_pond); // (`--street-pbr`, street_pbr.wgsl) the water the street's relief holds
        }
    }
    if (cine && n_geom.y > 0.7) {
        sky_streak_w = w;
        sky_streak_pud = pud;
    }
    (*s).albedo = (*s).albedo * mix(1.0, darken, w) * (1.0 - 0.16 * pud_rim) * (1.0 - 0.2 * pud - 0.14 * pud_depth);
    // The film's grazing sheet (street-surface-v2-render.js: the film floor
    // mix(0.48, 0.05, grazeShare), grazeShare = smoothstep(0.26, 0.05, n.v)):
    // seen low along the street the film is a mirror.
    let v = normalize(skyu.eye.xyz - world);
    let graze = smoothstep(0.26, 0.05, abs(dot(n_geom, v))) * select(0.0, 1.0, n_geom.y > 0.95);
    let film = mix(wet_rough, 0.06, graze * 0.92);
    let pud_rough = select(0.05, mix(0.045, 0.018, pud_depth), fx);
    (*s).rough = mix(mix((*s).rough, film, w), pud_rough, pud);
    // Relief filled by the film (reliefFill 0.75 / 0.70), flat in a puddle.
    var n = normalize(mix((*s).n, n_geom, clamp(w * select(0.70, 0.75, asphalt) + pud, 0.0, 1.0)));
    if (fx && n_geom.y > 0.92) {
        if (pud_depth > 0.05) {
            let fp_g = max(length(world - skyu.eye.xyz) * 2.0 * skyu.cluster.z * skyu.res.w, 1e-4) / max(abs(v.y), 0.14);
            let wr = sky_puddle_wind(world.xz, skyu.wet.w, fp_g) * pud_depth;
            n = normalize(n + vec3<f32>(wr.x, 0.0, wr.y));
        }
        let imp = sky_rain_floor_fx(world, pud, w, v);
        n = normalize(n + vec3<f32>(imp.x, 0.0, imp.y));
        sky_rain_spark = imp.z;
    } else if (pud > 0.01) {
        let sl = sky_ripple(world.xz) * pud;
        n = normalize(n + vec3<f32>(sl.x, 0.0, sl.y));
    }
    (*s).n = n;
// sf:end
// sf:WEATHER_FX_FLOOR-
    var pud = 0.0;
    if (n_geom.y > 0.95) {
        pud = sky_puddle(world.xz, select(-1.0, kerb, asphalt)) * w;
        if (SF_STREET_PBR) {
            pud = max(pud, sp_pond); // (`--street-pbr`, street_pbr.wgsl) the water the street's relief holds
        }
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
// sf:end
}

/// `sky_wet_generic` at `world` (look_light): with `--wet-surfaces` the
/// switch's porosity, puddles and wall runs (`sky_w2_generic`).
fn sky_wet_generic_at(s: ptr<function, Surface>, ng: vec3<f32>, world: vec3<f32>, allow_puddle: bool) {
    if (sky_w2_on()) {
        if (sky_on(skyu.flags.x, SKY_WET) && skyu.wet.x >= 0.02) {
            sky_w2_generic(s, ng, world, allow_puddle);
        }
        return;
    }
    sky_wet_generic(s, ng);
// sf:WEATHER_FX_FLOOR+
    sky_rain_spark = 0.0;
    let w = skyu.wet.x;
    if (allow_puddle && sky_on(skyu.flags.x, SKY_WET) && w >= 0.02 && sky_weather_fx_on(skyu.cirrus) && ng.y > 0.97) {
        let pf = sky_puddle_fx(world.xz, -1.0, false);
        // Keep standing mirror puddles off vegetated grass slopes; pool on flat plazas, decks and roofs.
        let grass = smoothstep(0.015, 0.055, (*s).albedo.g - max((*s).albedo.r, (*s).albedo.b));
        let hard = 1.0 - 0.85 * grass;
        let pud = pf.x * w * hard;
        let depth = pf.y * w * hard;
        let rim = pf.z * w * hard;
        let v = normalize(skyu.eye.xyz - world);
        (*s).albedo = (*s).albedo * (1.0 - 0.14 * rim) * (1.0 - 0.2 * pud - 0.12 * depth);
        (*s).rough = mix((*s).rough, mix(0.08, 0.04, depth), pud);
        var n = normalize(mix((*s).n, ng, clamp(pud * 0.95, 0.0, 1.0)));
        if (depth > 0.05) {
            let fp_g = max(length(world - skyu.eye.xyz) * 2.0 * skyu.cluster.z * skyu.res.w, 1e-4) / max(abs(v.y), 0.14);
            let wr = sky_puddle_wind(world.xz, skyu.wet.w, fp_g) * depth;
            n = normalize(n + vec3<f32>(wr.x, 0.0, wr.y));
        }
        let imp = sky_rain_floor_fx(world, pud, w, v);
        n = normalize(n + vec3<f32>(imp.x, 0.0, imp.y));
        (*s).n = n;
        sky_rain_spark = imp.z;
    }
// sf:end
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
/// The damp halo's width in hollow units (capillary soak ring around every
/// puddle), and the further darkening of its soaked ground (saturated, no film:
/// Lekner & Dorf's full darkening past the wet film's).
const SKY_W2_HALO: f32 = 0.14;
const SKY_W2_HALO_DARKEN: f32 = 0.64;
/// The aggregate's relief in the hollow field (its 12 and 5 cm octaves'
/// weight): a shore's lobes and fingers.
const SKY_W2_GRIT: f32 = 0.085;
/// Still water: its roughness, the darkening of the ground under it, the wind
/// wavelets' slope (at 1 m/s) and scale (m).
const SKY_W2_WATER_ROUGH: f32 = 0.05;
const SKY_W2_UNDER_WATER: f32 = 0.68;
const SKY_W2_WAVELET: vec2<f32> = vec2<f32>(0.011, 0.15);
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
/// its root. The wind is the weather's, at 10 m; a puddle lies in the
/// street canyon's lowest centimetres, where the mean wind is a tenth to a
/// third of it (urban canopy measurements): `SKY_W2_CANYON_WIND` of it.
const SKY_W2_WAVELET_MSS: vec2<f32> = vec2<f32>(6.0, 0.000128);
const SKY_W2_CANYON_WIND: f32 = 0.2;
/// Rain rings (Lagarde 2012; drop counts from Marshall-Palmer): about 66 drops
/// of ~2 mm a square metre a second for each mm/h, each ring living ~0.4 s,
/// its packet (two crests, ~4 cm wide) covering ~0.005 m2 on average, so
/// the share of the water under rings is 1 - exp(-x rate); under them the
/// water's slopes make a second lobe of alpha `y`: the drawn packet's
/// (`SKY_W2_RING`: peak 0.16, falling as (1 - age)^2 and with radius over
/// 3 cm, a sine across it) has a root mean square slope of about 0.04 over
/// its life and area.
const SKY_W2_RING_LOBE: vec2<f32> = vec2<f32>(0.15, 0.04);
/// A ring drawn where the pixel can hold it: its wavelength (m), the speed
/// of its front (m/s), its life (s), its peak slope; the cells drops land in (m).
const SKY_W2_RING: vec4<f32> = vec4<f32>(0.024, 0.22, 0.42, 0.055);
const SKY_W2_RING_CELL: f32 = 0.42;

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
    // the aggregate's own relief (about 12 and 5 cm), which breaks a shore
    // into lobes and fingers instead of a smooth ellipse
    let grit = mix(0.6 * st_noise(q * 8.3 + 5.1) + 0.4 * st_noise(q * 19.7 + 2.3), 0.5, smoothstep(0.02, 0.08, fp));
    return 0.5 * st_noise(q * 0.26 + 41.3) + 0.32 * st_noise(q * 0.63 + 7.9) + 0.18 * fine + SKY_W2_GRIT * (grit - 0.5) + SKY_W2_GUTTER * sky_drain(kerb);
}

/// The puddle (x, 0..1) and its damp halo (y, 0..1, none inside the water) at a
/// flat point: `fill` the water level's fill, `fp` the pixel's footprint.
fn sky_w2_puddle(p: vec2<f32>, kerb: f32, fill: f32, fp: f32) -> vec2<f32> {
    if (fill <= 0.0) {
        return vec2<f32>(0.0);
    }
    let depth = sky_w2_hollow(p, kerb, fp) - (SKY_W2_LEVEL.x - SKY_W2_LEVEL.y * pow(fill, SKY_W2_LEVEL.z));
    // the shore: soft capillary meniscus widened to the pixel footprint
    let shore = max(0.018, 0.62 * fp);
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
    let fade = 1.0 - smoothstep(0.22 * lam, 0.85 * lam, fp);
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
        let c = (cell - o + 0.32 + 0.36 * vec2<f32>(st_hash(s + 9.2), st_hash(s + 4.4))) * SKY_W2_RING_CELL;
        let d = p - c;
        let r = length(d);
        let cell_r = 0.30 * SKY_W2_RING_CELL;
        let cell_fade = 1.0 - smoothstep(cell_r * 0.62, cell_r, r);
        let onset = smoothstep(0.0, 0.10, age);
        let x = (r - age * SKY_W2_RING.z * SKY_W2_RING.y) / lam;
        let amp = SKY_W2_RING.w * onset * (1.0 - age) * (1.0 - age) * cell_fade * exp(-0.5 * x * x) / (1.0 + r / 0.035);
        sl = sl + d / max(r, 1e-4) * sin(6.2831853 * x) * amp * live;
    }
    return sl * fade;
}

/// A puddle's lobe (sets `sky_w2_pud_alpha`, `sky_w2_ring`; returns its
/// roughness): the still water's alpha widened by the wind's unresolved
/// wavelets (their mean square slope adds to alpha squared), and in rain the
/// share of the water under rings, which reflects through a second, rough lobe
/// only where the pixel's footprint `fp` is too coarse to draw them
/// (`sky_w2_rings`: where they are drawn their slopes are the normal's, and a
/// lobe as well counted them twice: a broad grey glow in near puddles).
fn sky_w2_lobe(fp: f32) -> f32 {
    let ws = min(length(skyu.wind.xy), SKY_W2_WAVELET_MSS.x);
    let a0 = SKY_W2_WATER_ROUGH * SKY_W2_WATER_ROUGH;
    let a = sqrt(a0 * a0 + SKY_W2_WAVELET_MSS.y * ws * SKY_W2_CANYON_WIND);
    sky_w2_pud_alpha = a;
    let drawn = 1.0 - smoothstep(0.22 * SKY_W2_RING.x, 0.85 * SKY_W2_RING.x, fp);
    sky_w2_ring = select(0.0, (1.0 - exp(-SKY_W2_RING_LOBE.x * skyu.wet.z)) * (1.0 - drawn), skyu.wet.z > 0.0);
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
    // (`--wet-relief`) where the street pass measured its relief's water: the ponds it holds
    let relief = sky_wr_on();
    let held = relief && SF_STREET_PBR && sp_pond_water > -1.0;
    if (n_geom.y > 0.95) {
        if (held) {
            ph = sky_wr_pond(world.xz, fp);
        } else {
            ph = sky_w2_puddle(world.xz, k, skyu.wet.y * select(SKY_W2_WALK_FILL, 1.0, asphalt), fp);
        }
    }
    let pud = ph.x;
    // the halo is saturated ground: as wet as the film can be
    let soak = max(w, ph.y);
    if (cine && n_geom.y > 0.7) {
        sky_streak_w = max(soak, pud);
        sky_streak_pud = pud;
    }
    sky_w2_water = max(soak, pud);
    if (held) {
        // (`--wet-relief`) Lekner & Dorf: the film and the ground under a pond alike
        (*s).albedo = (*s).albedo * mix(vec3<f32>(1.0), sky_wr_darken((*s).albedo), max(soak, pud));
    } else {
        (*s).albedo = (*s).albedo * mix(1.0, darken, soak) * mix(1.0, SKY_W2_UNDER_WATER, pud) * mix(1.0, SKY_W2_HALO_DARKEN, ph.y);
    }
    let v = normalize(skyu.eye.xyz - world);
    let graze = smoothstep(0.26, 0.05, abs(dot(n_geom, v))) * select(0.0, 1.0, n_geom.y > 0.95);
    var film = mix(wet_rough, 0.06, graze * 0.92);
    if (relief && !held) {
        sky_wr_iso = 1.0; // (`--wet-relief`) a walk's film is isotropic for the trace
    }
    if (held) {
        // (`--wet-relief`) the film as smooth as the surface under it, at every
        // angle: GGX and Fresnel make it a sheet seen low (its roughness does
        // not fall with the view; the grazing drop toward 0.06 made the whole
        // road as sharp a mirror as a pond, and the ponds could not be told
        // from it)
        film = wet_rough * sky_wr_film_k((*s).rough, asphalt);
    }
    (*s).rough = mix(mix((*s).rough, film, soak), min(film, SKY_W2_WATER_ROUGH * 2.4), ph.y * 0.65);
    var n = normalize(mix((*s).n, n_geom, clamp(soak * select(0.70, 0.75, asphalt), 0.0, 1.0)));
    if (pud > 0.01) {
        (*s).rough = mix((*s).rough, sky_w2_lobe(fp), pud);
        n = normalize(mix(n, sky_w2_water_normal(world.xz, n_geom, fp), pud));
    }
    (*s).n = n;
}

/// Any other surface wet (`sky_wet_generic` with the switch on): open ground
/// and roofs as a porous walk with their own puddles, walls in the rain's runs,
/// soffits dry. Dynamic props and bodies (`!allow_puddle`) take soft dampness
/// without standing mirror puddles or building wall runs.
fn sky_w2_generic(s: ptr<function, Surface>, ng: vec3<f32>, world: vec3<f32>, allow_puddle: bool) {
    var w = skyu.wet.x;
    let porosity = clamp(((*s).rough - SKY_W2_POROSITY.x) / SKY_W2_POROSITY.y, 0.0, 1.0) * (1.0 - (*s).metal);
    if (!allow_puddle) {
        let wt = w * (0.28 + 0.52 * smoothstep(0.45, 0.9, ng.y));
        (*s).albedo = (*s).albedo * mix(1.0, mix(1.0, 0.72, porosity), wt);
        (*s).rough = mix((*s).rough, min((*s).rough, 0.42), wt);
        sky_w2_water = wt * 0.35;
        return;
    }
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
        (*s).albedo = (*s).albedo * mix(1.0, mix(1.0, SKY_W2_GROUND.x, porosity), soak) * mix(1.0, SKY_W2_UNDER_WATER, pud) * mix(1.0, SKY_W2_HALO_DARKEN, ph.y);
        (*s).rough = mix((*s).rough, min((*s).rough, SKY_W2_GROUND.y), soak);
        var n = normalize(mix((*s).n, ng, clamp(soak * 0.7, 0.0, 1.0)));
        if (pud > 0.01) {
            (*s).rough = mix((*s).rough, sky_w2_lobe(fp), pud);
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

// ---- (`--wet-relief`, the reflect track; sky.rs `SkyOptions::wet_relief`,
// `SKY_WET_RELIEF_BIT`; compiled out without SF_WET_RELIEF, and with it only
// over `--wet-surfaces` in the street's pipelines) ----
// The water stands where the street's own relief holds it, and the film is
// as smooth as the surface under it lets it be:
// Only where the street pass measured its relief (with `--street-pbr`: the
// carriageway and the gutter pan); walks and plazas keep `--wet-surfaces`'
// film and puddles (their noise hollows are the only ponds there, and a
// rougher film seen low made them dark holes in a bright sheet):
// * the carriageway and the gutter pan: the wheel
//   paths' ruts (laid out from the standards' lane, parking lane and design
//   vehicle), the settled birdbaths and the pan's flow line hold water deeper
//   than what has drained since the rain (street_pbr.wgsl `sp_wet_relief`,
//   its water depth `sp_pond_water`), over the pavement's profile inside
//   those basins (a chain of pools along a rut, lobed shores, not a strip with
//   a straight edge); the switch's value-noise hollows stay
//   off the road (on a crowned carriageway they drew blobs either side of the
//   centre line and, in a narrow street, a canal from kerb to kerb). The shore
//   is anti-aliased by the pixel's footprint over the relief's slope; a ring
//   of saturated ground half a millimetre of depth wide lies round each pond
//   where the water has just drawn back;
// * the film's roughness and the streaks' lobes follow the surface's own
//   (its scan, the polished wheel paths, the oil band, the sealant, the
//   patches): the film fills the texture's valleys and what stays rough is
//   the texture's tops, so a polished path is glossier wet than the open
//   binder beside it. The look's calibrated film (`SKY_CINE_WET_ROUGH`) is the
//   open texture's; each pixel takes it times its own roughness over the
//   street's mean (never more than the look's film),
//   the streak lobes' alphas times that squared, at every angle of view (no
//   drop toward a mirror seen low: GGX and Fresnel make the sheet; the drop
//   made the film as sharp as the ponds, which then could not be told from
//   it). Lamps and headlamps break into bands along the wheel paths and
//   glints on the sealant instead of one uniform bar per light, and a pond is
//   a sharp mirror in a rougher film;
// * the darkening is Lekner & Dorf's (1988): a rough surface under water
//   keeps (1 - r_e)(1 - r_i) / (1 - r_i A) of its albedo A (r_e water's
//   normal-incidence reflectance 0.0204, r_i the film's mean internal
//   reflectance 1 - (1 - 0.066) / 1.333^2 = 0.474): 0.53-0.55 for asphalt
//   (the street pass has already taken the dry aggregate's lighter tone,
//   `SP_WET_DARKEN`), 0.6 for concrete; the ground under a pond the same
//   (saturated), not a further fixed share. Wet asphalt kept a third of its
//   albedo before (x 0.45 over the street pass's x 0.7): a black sheet in
//   which the ponds read as stains.
const SKY_WET_RELIEF: u32 = 4194304u;
const SKY_WR_FULL: bool = SF_WET_RELIEF && SKY_W2_FULL;
/// Lekner & Dorf: (1 - r_e)(1 - r_i), r_i.
const SKY_WR_LEKNER: vec2<f32> = vec2<f32>(0.5149, 0.4744);
/// The saturated ring round a pond: its width in water depth (m; over a
/// rut's or a birdbath's sides, a few centimetres on the ground).
const SKY_WR_HALO_M: f32 = 0.0005;
/// The shore's width in depth (m) and per metre of the pixel's footprint
/// (the relief's slope, street_pbr.wgsl `sp_wet_relief`).
const SKY_WR_SHORE: vec2<f32> = vec2<f32>(3e-4, 0.015);
/// The film's roughness over the look's, as the surface's own over its mean:
/// held within x..y (never rougher than the look's film, the open texture's
/// own, so a rough patch's film stays inside the reflection's roughness
/// range, 0.3-0.5). The film of a few tenths of a millimetre levels the
/// texture under it, and what roughens it is as much its own (drops landing,
/// the drying film's edges) as the surface's: a smooth patch, plate or
/// sealant band makes it at most a third smoother (a floor of 0.45 made a
/// smooth plate a hard-edged mirror in a soft sheen, and lines of sealant
/// laser-thin streaks).
const SKY_WR_FILM_K: vec2<f32> = vec2<f32>(0.7, 1.0);
/// The pavement's profile under the water (m of height, m of wavelength; two
/// octaves, domain-warped): a rut's floor, a birdbath's and the gutter's flow
/// line are not level along the road. The paver's screed and the base's
/// settling leave a few millimetres of undulation over a metre or two (an
/// urban street's roughness, IRI ~3-5 m/km), the mix's segregation about a
/// millimetre over half a metre. Against a rut's side slope of ~2 % that moves
/// a shore by up to 15 cm and leaves stretches of rut dry: the water stands in
/// a chain of pools with lobed shores, not in a strip with a straight edge.
const SKY_WR_PROFILE: vec4<f32> = vec4<f32>(0.003, 1.7, 0.0012, 0.45);
/// The profile pools the water only inside a basin of the relief (a rut, a
/// birdbath, the flow line: relief deeper than x, fully from y m): on the
/// open carriageway the crown's cross fall drains an undulation instead.
const SKY_WR_BASIN: vec2<f32> = vec2<f32>(0.0005, 0.002);

/// (`--wet-relief`) the streak lobes' alphas over the look's at the fragment
/// (the film's roughness ratio squared; 1 elsewhere).
var<private> sky_wr_alpha_k: f32 = 1.0;
/// (`--wet-relief`) the share of the fragment's film the trace filters as an
/// isotropic surface (walks and plazas: 1; 0 elsewhere). A broom-finished
/// walk's film is as rough across the view as along it, so what it reflects
/// blurs sideways by alpha n.v as it stretches toward the eye by alpha: the
/// look's anisotropic streak lobe (a seventh as wide across) drew a pillar, a
/// post or a person on a wet walk as a hard-edged slab.
var<private> sky_wr_iso: f32 = 0.0;

fn sky_wr_on() -> bool {
    return SKY_WR_FULL && sky_on(skyu.flags.x, SKY_WET_RELIEF) && sky_w2_on();
}

/// The pavement's profile at a horizontal point (m, mean 0; `SKY_WR_PROFILE`).
fn sky_wr_profile(p: vec2<f32>) -> f32 {
    let warp = vec2<f32>(st_noise(p * 0.31 + 5.7), st_noise(p * 0.31 + 13.9)) - 0.5;
    let q = p + warp * 1.6;
    return (st_noise(q / SKY_WR_PROFILE.y + 2.9) - 0.5) * 2.0 * SKY_WR_PROFILE.x + (st_noise(q / SKY_WR_PROFILE.w + 8.3) - 0.5) * 2.0 * SKY_WR_PROFILE.z;
}

/// The pond (x) and its saturated ring (y) at `p` where the street pass
/// measured the relief's water (`sp_pond_water`, m over the drained level),
/// over the pavement's profile there; the shore over the footprint `fp` (m).
fn sky_wr_pond(p: vec2<f32>, fp: f32) -> vec2<f32> {
    // the relief's depth there (the street pass's water over what has drained)
    let dip = sp_pond_water + SP_POND_DRAIN_M * (1.0 - clamp(skyu.wet.y, 0.0, 1.0));
    let water = sp_pond_water + sky_wr_profile(p) * smoothstep(SKY_WR_BASIN.x, SKY_WR_BASIN.y, dip);
    let pud = smoothstep(0.0, SKY_WR_SHORE.x + fp * SKY_WR_SHORE.y, water);
    let ring = smoothstep(-SKY_WR_HALO_M, 0.0, water) * (1.0 - pud);
    return vec2<f32>(pud, ring);
}

/// Lekner & Dorf's wet albedo over the dry one, for albedo `a`.
fn sky_wr_darken(a: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(SKY_WR_LEKNER.x) / (vec3<f32>(1.0) - SKY_WR_LEKNER.y * clamp(a, vec3<f32>(0.0), vec3<f32>(1.0)));
}

/// The film's roughness over the look's at a surface of roughness `rough`
/// (asphalt or concrete), and the streak lobes' factor (`sky_wr_alpha_k`).
fn sky_wr_film_k(rough: f32, asphalt: bool) -> f32 {
    let mean = select(ST_CONCRETE_ROUGHNESS, select(ST_ASPHALT_ROUGHNESS, SP_ASPHALT_ROUGH, SF_STREET_PBR), asphalt);
    let k = clamp(rough / max(mean, 0.05), SKY_WR_FILM_K.x, SKY_WR_FILM_K.y);
    sky_wr_alpha_k = k * k;
    return k;
}

// ---- (`--screen-reflect`, the reflect track; sky.rs `SkyOptions::screen_reflect`,
// `SKY_SCREEN_REFLECT_BIT`; compiled out without SF_SCREEN_REFLECT, and into the
// street's pipelines only) ----
// The screen-space reflection, third generation, on glossy ground, car paint's
// clearcoat and glass:
// * the trace (`sky_sr_trace`): the ray's own path across the previous frame's
//   depth (the occlusion pass's half-resolution view depth, with the camera it
//   was made with), stepped on the screen (McGuire & Mara 2014): the ray's 1/w
//   and its depth over w are linear on the screen, so its depth at each step
//   is exact and every stretch of its path is looked at, near and far alike.
//   It walks the depth's hierarchy (Hi-Z: the reflection pyramid's alpha,
//   sky_reflect.rs, the nearest depth of each cell of 2^l half-resolution
//   texels): a cell the ray crosses wholly in front of its nearest depth is
//   passed in one step and the walk climbs a level; a cell it may pass behind
//   is looked into a level finer; a texel is tested. So every texel of the
//   path is looked at (the linear march's 40 steps, packed toward the start,
//   skipped up to 25 texels at a time at its far end: thin poles and kerbs
//   were stepped over) in at most `SKY_SR_STEPS` steps. The first texel is a
//   texel and a half out, jittered per pixel and frame (interleaved gradient
//   noise, golden-ratio steps). A texel hits where its depth lies within the
//   ray's depth across it (passed by a small bias) and no more than the
//   sample's assumed thickness in front of it, but a ray that comes in behind
//   a face at its silhouette only as deep behind it as the thing is wide (a
//   person, a pole is passed, not drawn out into a slab); bisections on the screen place
//   the hit against the depth read bilinearly where its four texels are one
//   surface (a reflected facade moves smoothly with the pixel instead of in
//   half-resolution blocks). A ray that stays on the frame to its full reach
//   and meets nothing has escaped: it sees the sky;
// * the filter (`sky_sr_cone`): the surface's lobe at the hit seen through
//   the mirror, read from the previous frame's mip chain (sky_reflect.rs). A
//   GGX lobe of alpha a turns the reflected ray by about 1.1 a (a Gaussian's
//   sigma) in the plane of incidence and by 1.1 a n.v across it (a sideways
//   tilt of the normal turns the ray by only its cosine: why every light on a
//   wet road streaks toward the eye, longer the lower it is seen); at the hit,
//   L along the ray, that is an angle of sigma L / (D + L) from the eye over
//   the mirrored path. So the reflection is sharp where the reflected thing
//   stands on the surface (a pole's foot in a puddle) and long for far lamps,
//   windows and signs. The level is the across width; taps along the streak
//   (at most 12) where it is longer, each wide enough to meet its neighbours.
//   No tap is jittered across a lobe: the noise in tail-lamp and window
//   reflections was the jittered taps, which the history's clamp kept;
// * the lobes: the wet film's anisotropic one where the street is wet, a
//   puddle's own (`sky_w2_lobe`: the wind's wavelets), and in rain the rings'
//   second lobe where the rings are too fine to draw;
// * the open sky: a ray the trace does not stop reads the previous frame along
//   its own direction, the point at infinity, where that pixel is open sky or
//   scenery past 1.5-3 km (read from five depth texels round it, so the
//   frame's skyline fades in over a few pixels instead of cutting): the clouds,
//   the low sun's glow and the city's night glow as the frame draws them,
//   through the same lobe, and where it is scenery far past the surface
//   (parallax under 0.05-0.25 rad: the eye's view along the ray is the
//   ray's own; the far towers in a puddle). An escaped ray whose direction
//   lands on a nearer building reads the sky just above the skyline there
//   instead (it passed over or beside what the camera sees at that pixel). A direction off the frame (a
//   near puddle's view of the sky overhead) reads the frame's edge toward it
//   where that is sky (ground facing up and escaped rays: a pane's ray off the
//   frame that has not escaped runs low into the facades, which only the
//   probes know), scaled by the weather dome's gradient between the two
//   directions (by daylight only: its overcast weight's CIE (1 + 2 sin el) / 3,
//   held within 0.5-2; the night's city glow on a deck is flat) and trusted
//   less with the angle between them (fully within 0.3 rad, not past 0.9):
//   the probes keep only what the frame cannot say;
// * glass (look_light `sky_sr_glass`, `sky_sr_glass_mix`): panes trace too, in
//   any orientation, and their clearcoat reads the trace where it finds
//   something (the facade across the street, the street, the sky) over the
//   probe grid.
const SKY_SCREEN_REFLECT: u32 = 16777216u;
const SKY_SR_FULL: bool = SF_SCREEN_REFLECT && LK_SHADOW_CASCADES && LK_SHADOW_FAR;
/// The cone: the reflected ray's spread (a Gaussian's sigma, rad) per unit of
/// GGX alpha (twice the half-width at half maximum of the normals, 0.644 a,
/// over 1.177); a mip level m's blur, about `y` 2^m pixels (13-tap means read
/// bilinearly); the most taps along a streak.
const SKY_SR_CONE: vec3<f32> = vec3<f32>(1.1, 0.8, 12.0);
/// The trace: its most steps, its coarsest Hi-Z level (cells of 2^x
/// half-resolution texels: the pyramid's level x + 1), the bisections that
/// place a hit.
const SKY_SR_STEPS: i32 = 48;
const SKY_SR_HIZ_TOP: i32 = 6;
const SKY_SR_REFINE: i32 = 5;
/// The cone's coarsest level (1/32 of the frame: the widest lobe it filters;
/// the pyramid goes on for the Hi-Z).
const SKY_SR_CONE_TOP: f32 = 5.0;
/// The Hi-Z's sky (sky_reflect.wgsl `RP_FAR_M`): no surface in the cell.
const SKY_SR_HIZ_FAR: f32 = 60000.0;
/// The ray's reach (m) and the nearest view depth it is followed to (m).
const SKY_SR_REACH_M: f32 = 800.0;
const SKY_SR_NEAR_M: f32 = 0.25;
/// A depth sample's assumed thickness, x + y x its depth (m), and the margin a
/// ray must pass behind it by to count, x + y x its depth (m: the
/// half-resolution texel's own depth slope on ground seen low).
const SKY_SR_THICK: vec2<f32> = vec2<f32>(0.4, 0.04);
const SKY_SR_BIAS: vec2<f32> = vec2<f32>(0.03, 0.004);
/// The first thing a ray passes behind holds it (the ray is inside it) where
/// the ray is no deeper behind its face than x times its width (a canopy, a
/// facade, a car: about as deep as they are wide); its width measured where
/// the depth stays within y of its own at taps this many half-resolution
/// texels out (`sky_sr_width`). Without it a ray from a puddle up into a
/// street tree's crown passed behind the leaves the camera sees and read the
/// sky beyond: a hole the colour of the sky in the tree's reflection.
const SKY_SR_WIDE: vec2<f32> = vec2<f32>(1.0, 0.35);
const SKY_SR_WIDE_TAPS: vec3<f32> = vec3<f32>(4.0, 12.0, 32.0);
/// Four depth texels are one surface for the bilinear read within this share
/// of their nearest.
const SKY_SR_LIN_TOL: f32 = 0.08;
/// Scenery past this view distance (m) counts as open sky for the direction
/// lookup (fully past the second).
const SKY_SR_FAR_M: vec2<f32> = vec2<f32>(1500.0, 3000.0);
/// Scenery along the ray's direction reads as the ray's own where the
/// parallax between the eye's view and the surface's (range / depth, rad) is
/// under x, not past y.
const SKY_SR_PARALLAX: vec2<f32> = vec2<f32>(0.05, 0.25);
/// The open test's taps round the direction (uv, at least): x; an escaped
/// ray's search for the skyline above a building: y steps of z (uv).
const SKY_SR_OPEN: vec3<f32> = vec3<f32>(0.006, 6.0, 0.05);
/// A direction off the frame: the frame's edge read this far in (uv), trusted
/// fully within 0.3 rad of it (cosine x) and not at all past 0.9 rad (y), the
/// dome's gradient between them held within z..w.
const SKY_SR_EDGE_IN: f32 = 0.01;
/// Only a surface facing up past this normal y extends the frame's sky (its
/// ray off the frame climbs over the street; a pane's runs into the facades)
/// unless its ray escaped; it is also the ground whose lobe streaks along the
/// frame's vertical.
const SKY_SR_EXTEND_NY: f32 = 0.6;
const SKY_SR_OFF_FRAME: vec4<f32> = vec4<f32>(0.9553365, 0.6216100, 0.5, 2.0);
/// A pane's or a flank's ray that neither hits nor escapes: the share of the
/// frame along its direction over the probes where that is not open sky.
const SKY_SR_FRAME_ENV: f32 = 0.6;
/// (look_light) the surface being shaded is glass (set before `sky_ssr`).
var<private> sky_sr_glass: bool;
/// The share of the reflection's weight the trace's hit carries (set by
/// `sky_sr_ssr`; the rest is the frame's sky along the ray).
var<private> sky_sr_hit: f32;

fn sky_sr_on() -> bool {
    return SKY_SR_FULL && sky_on(skyu.flags.x, SKY_SCREEN_REFLECT);
}

/// (look_light) The indirect specular: the environment `env` and the
/// reflection `ssr` (rgb, weight) through the split sum `fss`, under the
/// specular occlusion `so` (the occlusion pass's visibility as a cone, Lagarde
/// & de Rousiers 2014) only where the trace did not find what the ray meets:
/// a hit's radiance is what the occluder sends, and occluding it again took
/// most of a car's, a kerb's or a wall's reflection at its foot (where the
/// occlusion is deepest). Off, as before.
fn sky_sr_spec(env: vec3<f32>, ssr: vec4<f32>, fss: vec3<f32>, so: f32) -> vec3<f32> {
    if (!sky_sr_on()) {
        return mix(env, ssr.rgb, ssr.a) * fss * so;
    }
    return mix(env * so, ssr.rgb * mix(so, 1.0, sky_sr_hit), ssr.a) * fss;
}

/// (look_light) The glass clearcoat's environment `rad` (x `env_i` there) with
/// the trace's radiance `ssr` over it where it found something.
fn sky_sr_glass_mix(rad: vec3<f32>, ssr: vec4<f32>, env_i: f32) -> vec3<f32> {
    if (!sky_sr_on() || !sky_sr_glass) {
        return rad;
    }
    return mix(rad, ssr.rgb / max(env_i, 1e-3), ssr.a);
}

/// The previous frame round `uv` through a lobe of Gaussian sigma `sig`
/// pixels across (x, the frame's horizontal) and along (y, its vertical) the
/// plane of incidence: the history's mip chain read at the across width, with
/// taps along the streak where it is longer (their level wide enough to meet
/// each other; `j` slides them within a spacing), Gaussian-weighted; capped
/// at 24 x the exposure.
fn sky_sr_cone(uv: vec2<f32>, sig: vec2<f32>, j: f32) -> vec3<f32> {
    let top = min(f32(textureNumLevels(sky_prev) - 1u), SKY_SR_CONE_TOP);
    let sa = max(sig.x, 0.5);
    let sl = max(sig.y, 0.5);
    let n = i32(clamp(ceil(2.0 * sl / sa), 1.0, SKY_SR_CONE.z));
    let spacing = 4.0 * sl / f32(n);
    let blur = select(sa, max(sa, 0.5 * spacing), n > 1);
    let lod = clamp(log2(blur / SKY_SR_CONE.y), 0.0, top);
    let h = 0.5 * exp2(ceil(lod)) / skyu.res.xy;
    var c = vec3<f32>(0.0);
    var wsum = 0.0;
    for (var k = 0; k < n; k = k + 1) {
        let o = select(0.0, (f32(k) + j) * spacing - 2.0 * sl, n > 1);
        let wk = exp(-0.5 * (o / sl) * (o / sl));
        let ut = clamp(uv + vec2<f32>(0.0, o / skyu.res.y), h, vec2<f32>(1.0) - h);
        c = c + textureSampleLevel(sky_prev, sky_rep, ut, lod).rgb * wk;
        wsum = wsum + wk;
    }
    c = c / max(wsum, 1e-6);
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

/// The depth at `s` (texels) read bilinearly where the four texels round it
/// are one surface (within `SKY_SR_LIN_TOL` of their nearest), else the
/// nearest texel's.
fn sky_sr_depth_lin(s: vec2<f32>, top: vec2<i32>) -> f32 {
    let p = s - 0.5;
    let b = vec2<i32>(floor(p));
    let f = p - floor(p);
    let z00 = textureLoad(lk_ao_z, clamp(b, vec2<i32>(0), top), 0).r;
    let z10 = textureLoad(lk_ao_z, clamp(b + vec2<i32>(1, 0), vec2<i32>(0), top), 0).r;
    let z01 = textureLoad(lk_ao_z, clamp(b + vec2<i32>(0, 1), vec2<i32>(0), top), 0).r;
    let z11 = textureLoad(lk_ao_z, clamp(b + vec2<i32>(1, 1), vec2<i32>(0), top), 0).r;
    let lo = min(min(z00, z10), min(z01, z11));
    let hi = max(max(z00, z10), max(z01, z11));
    if (lo <= 0.0 || hi - lo > SKY_SR_LIN_TOL * lo) {
        return sky_sr_depth(s, top);
    }
    return mix(mix(z00, z10, f.x), mix(z01, z11, f.x), f.y);
}

/// The nearest view depth (m) in Hi-Z cell `c` of level `l` (0: the
/// half-resolution depth's own texel, `top` its last; sky and nothing: far).
fn sky_sr_hiz(c: vec2<i32>, l: i32, top: vec2<i32>) -> f32 {
    if (l == 0) {
        let z = textureLoad(lk_ao_z, clamp(c, vec2<i32>(0), top), 0).r;
        return select(SKY_SR_HIZ_FAR, z, z > 0.0);
    }
    let d = vec2<i32>(textureDimensions(sky_prev, l + 1)) - 1;
    return textureLoad(sky_prev, clamp(c, vec2<i32>(0), d), l + 1).a;
}

/// Open sky at the frame's `q` (uv): the share of five depth texels round it
/// (`rho` uv apart) that are sky or scenery past `SKY_SR_FAR_M`.
fn sky_sr_open(q: vec2<f32>, rho: f32) -> f32 {
    let dims = vec2<f32>(textureDimensions(lk_ao_z));
    let top = vec2<i32>(dims) - 1;
    var open = 0.0;
    for (var k = 0; k < 5; k = k + 1) {
        let a = f32(k) * 1.2566371;
        let o = select(vec2<f32>(cos(a), sin(a)) * rho, vec2<f32>(0.0), k == 4);
        let z = sky_sr_depth(clamp(q + o, vec2<f32>(0.0), vec2<f32>(1.0)) * dims, top);
        open = open + select(smoothstep(SKY_SR_FAR_M.x, SKY_SR_FAR_M.y, z), 1.0, z <= 0.0);
    }
    return open * 0.2;
}

/// The width (half-resolution texels) of what stands at texel `s` (view depth
/// `z`) along the ray's screen direction `dir` and across it: how far the
/// depth stays within `SKY_SR_WIDE.y` of `z` at taps `SKY_SR_WIDE_TAPS` out
/// (ahead; either side, summed), the smaller of the two.
fn sky_sr_width(s: vec2<f32>, dir: vec2<f32>, z: f32, top: vec2<i32>) -> f32 {
    let perp = vec2<f32>(-dir.y, dir.x);
    var along = 0.0;
    var left = 0.0;
    var right = 0.0;
    for (var m = 0; m < 3; m = m + 1) {
        let o = select(select(SKY_SR_WIDE_TAPS.z, SKY_SR_WIDE_TAPS.y, m == 1), SKY_SR_WIDE_TAPS.x, m == 0);
        let za = sky_sr_depth(s + dir * o, top);
        let zl = sky_sr_depth(s + perp * o, top);
        let zr = sky_sr_depth(s - perp * o, top);
        let tol = SKY_SR_WIDE.y * z;
        along = select(along, o, za > 0.0 && abs(za - z) < tol);
        left = select(left, o, zl > 0.0 && abs(zl - z) < tol);
        right = select(right, o, zr > 0.0 && abs(zr - z) < tol);
    }
    return min(along, left + right);
}

/// The trace (see above) from `p0` along `r`, jittered by `j`: the hit's uv,
/// its distance along the ray (m) and its weight (0: none; the thickness
/// test's fade). No hit: z -1 where the ray escaped (it stayed on the frame
/// to its full reach), else 0.
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
    // the whole reach on the frame: a ray that meets nothing and never
    // passes behind what the frame shows has escaped
    let whole = fe >= 1.0 && reach >= SKY_SR_REACH_M;
    let len = length(ds);
    let span = len * max(fe, 0.0);
    if (span < 2.0) {
        return vec4<f32>(0.0, 0.0, select(0.0, -1.0, whole), 0.0);
    }
    // the Hi-Z walk (see above): the levels the pyramid holds over this
    // depth (none where its level 1 is not the depth's size: a texel at a time)
    let hiz_ok = textureNumLevels(sky_prev) > 2u && all(textureDimensions(sky_prev, 1) == textureDimensions(lk_ao_z));
    let l_top = select(0, min(SKY_SR_HIZ_TOP, i32(textureNumLevels(sky_prev)) - 2), hiz_ok);
    let f_end = max(fe, 0.0);
    let inv = vec2<f32>(select(1e30, 1.0 / ds.x, abs(ds.x) > 1e-6), select(1e30, 1.0 / ds.y, abs(ds.y) > 1e-6));
    let toward = select(vec2<f32>(0.0), vec2<f32>(1.0), ds > vec2<f32>(0.0));
    let nudge = 1e-3 / len;
    var f = (1.5 + j) / len;
    var lvl = 0;
    var f_prev = 0.0;
    var f_hit = -1.0;
    var behind = false;
    // the width (m) of the last thing measured and its depth (0: none yet)
    var w_m = 0.0;
    var w_z = 0.0;
    for (var i = 0; i < SKY_SR_STEPS; i = i + 1) {
        if (f >= f_end) {
            break;
        }
        let cs = exp2(f32(lvl));
        let cell = floor((s0 + ds * f) / cs);
        // where the ray leaves the cell
        let tb = ((cell + toward) * cs - s0) * inv;
        let f_out = min(max(min(tb.x, tb.y), f), f_end);
        let za = mix(q0, q1, f) / mix(k0, k1, f);
        let zb = mix(q0, q1, f_out) / mix(k0, k1, f_out);
        let z = sky_sr_hiz(vec2<i32>(cell), lvl, top);
        if (max(za, zb) <= z + SKY_SR_BIAS.x + SKY_SR_BIAS.y * z) {
            // wholly in front of the cell's nearest: past it, a level up
            f = f_out + nudge;
            lvl = min(lvl + 1, l_top);
            continue;
        }
        if (lvl > 0) {
            lvl = lvl - 1;
            continue;
        }
        // a texel: a hit where the ray crosses its depth within its thickness
        let thick = SKY_SR_THICK.x + SKY_SR_THICK.y * z;
        let solid = z < SKY_SR_HIZ_FAR;
        // how far behind the texel's face the ray's nearer end lies, and what
        // a crossing in the texel allows (the ray's own depth across it and the bias)
        let gap = min(za, zb) - z;
        let cross = abs(zb - za) + SKY_SR_BIAS.x + SKY_SR_BIAS.y * z;
        let inside = solid && gap < thick;
        let past = solid && gap >= thick;
        // a ray that comes in behind a face (at its silhouette, not across
        // it) or passes wholly behind it is inside the thing only as deep as
        // it is wide (a canopy, a facade, a car body: about as deep as wide);
        // a pole, a sign or a person, narrow across the ray's path, it passes
        // (a fixed thickness held it a metre behind a person: his reflection
        // drawn out into a slab on a wet walk). The width: how far the depth
        // stays near this texel's along the ray's path and either side of it,
        // measured again only for another thing (one call site).
        if ((inside && gap > cross) || (past && !behind)) {
            if (w_z <= 0.0 || abs(z - w_z) > SKY_SR_WIDE.y * w_z) {
                w_m = sky_sr_width(s0 + ds * f, ds / len, z, top) * z * 2.0 * skyu.cluster.z / dims.y;
                w_z = z;
            }
        }
        if (inside && (gap <= cross || gap < SKY_SR_WIDE.x * w_m)) {
            f_prev = f;
            f_hit = f_out;
            break;
        }
        if (past && !behind && gap < SKY_SR_WIDE.x * w_m) {
            // the first thing the ray passes behind holds it
            let kf = mix(k0, k1, f);
            return vec4<f32>((s0 + ds * f) / dims, reach * k1 * f / kf, 1.0);
        }
        behind = behind || past;
        f = f_out + nudge;
        lvl = min(1, l_top);
    }
    let escaped = select(0.0, -1.0, whole && !behind && f >= f_end);
    if (f_hit < 0.0) {
        return vec4<f32>(0.0, 0.0, escaped, 0.0);
    }
    // bisections, then the ray's gap to the depth at both ends of the last
    // interval (one call site: the lit pipelines stand at Apple's compiler's
    // limit) for the secant's crossing
    var lo = f_prev;
    var hi = f_hit;
    var gl = 0.0;
    var gh = 0.0;
    for (var i = 0; i < SKY_SR_REFINE + 2; i = i + 1) {
        let mid = select(select(hi, lo, i == SKY_SR_REFINE), 0.5 * (lo + hi), i < SKY_SR_REFINE);
        let z = sky_sr_depth_lin(s0 + ds * mid, top);
        let g = select(-1.0, mix(q0, q1, mid) / mix(k0, k1, mid) - z, z > 0.0);
        if (i == SKY_SR_REFINE) {
            gl = g;
        } else if (i > SKY_SR_REFINE) {
            gh = g;
        } else if (g > 0.0) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    var fx = hi;
    if (gl < 0.0 && gh > 0.0) {
        fx = mix(lo, hi, clamp(-gl / (gh - gl), 0.0, 1.0));
    }
    let k = mix(k0, k1, fx);
    let z = sky_sr_depth(s0 + ds * fx, top);
    let thick = SKY_SR_THICK.x + SKY_SR_THICK.y * z;
    let solid = 1.0 - smoothstep(thick, 2.0 * thick, mix(q0, q1, fx) / k - z);
    if (z <= 0.0 || solid <= 0.0) {
        return vec4<f32>(0.0, 0.0, escaped, 0.0);
    }
    // the distance along the ray, perspective-correct
    return vec4<f32>((s0 + ds * fx) / dims, reach * k1 * fx / k, solid);
}

/// The view direction through the frame's `q` (uv).
fn sky_sr_dir(q: vec2<f32>) -> vec3<f32> {
    let tn = skyu.cluster.z;
    return normalize(skyu.cam_fwd.xyz + skyu.cam_right.xyz * ((2.0 * q.x - 1.0) * tn * skyu.cluster.w) + skyu.cam_up.xyz * ((1.0 - 2.0 * q.y) * tn));
}

/// The reflection with the switch on (see above): rgb radiance, a weight.
fn sky_sr_ssr(world: vec3<f32>, n: vec3<f32>, v: vec3<f32>, rough: f32) -> vec4<f32> {
    sky_sr_hit = 0.0;
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
    // the lobe's alphas: the wet film's anisotropic one where the street is
    // wet, a puddle's own (`--wet-surfaces`: the wind's wavelets), and in rain
    // the rings' second lobe; as the reflected ray's spread (sigma, rad)
    // across (x n.v on the ground) and along the plane of incidence
    let ground = n.y >= SKY_SR_EXTEND_NY;
    let a_pud = max(SKY_CINE_PUDDLE_ALPHA, sky_w2_pud_alpha);
    let a_k = select(1.0, sky_wr_alpha_k, SKY_WR_FULL); // (`--wet-relief`) the film's lobes by the surface under it
    let a_along = mix(rough * rough, mix(SKY_CINE_STREAK_ALONG * a_k, a_pud, sky_streak_pud), sky_streak_w);
    var a_across = mix(rough * rough, mix(SKY_CINE_STREAK_ACROSS * a_k, a_pud, sky_streak_pud), sky_streak_w);
    if (SKY_WR_FULL) {
        a_across = mix(a_across, a_along, sky_wr_iso * (1.0 - sky_streak_pud)); // (`--wet-relief`) a walk's film is isotropic
    }
    let cw = select(1.0, max(dot(n, v), 0.02), ground);
    // pixels a radian at the frame's centre
    let ppr = skyu.res.y / (2.0 * max(skyu.cluster.z, 0.1));
    let ang = SKY_SR_CONE.x * vec2<f32>(a_across * cw, a_along) * ppr;
    let ang_r = SKY_SR_CONE.x * SKY_W2_RING_LOBE.y * vec2<f32>(cw, 1.0) * ppr;
    let ring = sky_w2_ring * sky_streak_pud;
    let fade = 1.0 - smoothstep(SKY_CINE_SSR_ROUGH.x, SKY_CINE_SSR_ROUGH.y, rough);
    let range = length(world - skyu.eye.xyz);
    let h = sky_sr_trace(world, r, j);
    var hit_w = 0.0;
    var contact = 0.0;
    if (h.w > 0.0) {
        // the lobe at the hit seen through the mirror: sigma L / (D + L) each way
        contact = h.z / (range + h.z);
        hit_w = sky_sr_edge(h.xy, 0.05) * h.w;
    }
    // the open sky along the ray's own direction (the point at infinity)
    var q_sky = vec2<f32>(0.0);
    var sky_w = 0.0;
    var gain = 1.0;
    if (hit_w < 1.0) {
        let clip = look.ao_vp * vec4<f32>(r, 0.0);
        if (clip.w > 1e-4) {
            let escaped = h.z < 0.0;
            let q = vec2<f32>(clip.x / clip.w * 0.5 + 0.5, 0.5 - clip.y / clip.w * 0.5);
            var qc = clamp(q, vec2<f32>(SKY_SR_EDGE_IN), vec2<f32>(1.0 - SKY_SR_EDGE_IN));
            let off = any(q != qc);
            let rho = max(SKY_SR_OPEN.x, 2.0 * ang.x / skyu.res.y);
            // scenery there far past the surface (against the eye's distance
            // to it) is at infinity for this ray too: the eye's view along r
            // and the surface's differ by about range / depth radians, so
            // what the frame shows there is what the ray meets (the far
            // towers in a puddle, which a trace's steps can pass between)
            let zd = vec2<f32>(textureDimensions(lk_ao_z));
            let zq = sky_sr_depth(qc * zd, vec2<i32>(zd) - 1);
            let far = select(0.0, 1.0 - smoothstep(SKY_SR_PARALLAX.x, SKY_SR_PARALLAX.y, range / max(zq, 1e-3)), zq > 0.0);
            // open sky there (or far scenery); an escaped ray whose direction
            // lands on a near building passed what the camera sees there: the
            // sky over the skyline above it (one call site, as above)
            var open = 0.0;
            for (var k = 0; k <= i32(SKY_SR_OPEN.y); k = k + 1) {
                let qk = vec2<f32>(qc.x, max(qc.y - f32(k) * SKY_SR_OPEN.z, SKY_SR_EDGE_IN));
                let ok = max(sky_sr_open(qk, rho), select(0.0, far, k == 0));
                if (k == 0) {
                    open = ok;
                    if (!escaped || ok >= 0.5) {
                        break;
                    }
                    open = 0.0;
                } else if (ok >= 0.5) {
                    qc = qk;
                    open = 1.0;
                    break;
                }
            }
            let moved = off || any(qc != clamp(q, vec2<f32>(SKY_SR_EDGE_IN), vec2<f32>(1.0 - SKY_SR_EDGE_IN)));
            if (!ground && !escaped) {
                // a pane's or a flank's ray off the frame runs low along the
                // street, where the frame cannot say what blocks it: the
                // probes. On the frame, what the camera sees along the ray's
                // direction stands for what the ray meets past the frame's
                // data (the facades across, the sky over them, no parallax):
                // wholly where that is open sky, `SKY_SR_FRAME_ENV` of it
                // over the probes elsewhere (the probes' dome is not the
                // frame's sky: a cut from clouds to flat blue in the glass)
                open = select(mix(SKY_SR_FRAME_ENV, 1.0, open) * sky_sr_edge(q, 0.15), 0.0, off);
            } else if (open > 0.0 && moved) {
                let rc = sky_sr_dir(qc);
                open = open * smoothstep(SKY_SR_OFF_FRAME.y, SKY_SR_OFF_FRAME.x, dot(r, rc));
                // the weather's dome by daylight (lk_sky_upper): toward the
                // CIE overcast gradient by its overcast weight, toward flat by
                // its isotropy; the night's glow on the deck is flat
                let cie = (1.0 + 2.0 * max(r.y, 0.0)) / (1.0 + 2.0 * max(rc.y, 0.0));
                gain = mix(clamp(mix(mix(1.0, cie, look.wx.x), 1.0, look.wx.y), SKY_SR_OFF_FRAME.z, SKY_SR_OFF_FRAME.w), 1.0, clamp(skyu.night.z, 0.0, 1.0));
            }
            q_sky = qc;
            sky_w = open;
        }
    }
    let w = hit_w + sky_w * (1.0 - hit_w);
    if (w <= 0.0) {
        return vec4<f32>(0.0);
    }
    sky_sr_hit = hit_w / w;
    // the lookups through the lobes (one call site: the lit pipelines stand
    // at Apple's compiler's limit): the hit through the surface's lobe and
    // the rings', the sky through the surface's
    var hit_c = vec3<f32>(0.0);
    var sky_c = vec3<f32>(0.0);
    for (var k = 0; k < 3; k = k + 1) {
        var wk = 0.0;
        var uv = h.xy;
        var sg = ang * contact;
        if (k == 0) {
            wk = select(0.0, 1.0 - ring, hit_w > 0.0);
        } else if (k == 1) {
            wk = select(0.0, ring, hit_w > 0.0);
            sg = ang_r * contact;
        } else {
            wk = select(0.0, 1.0, sky_w > 0.0);
            uv = q_sky;
            sg = ang;
        }
        if (wk <= 0.0) {
            continue;
        }
        let c = sky_sr_cone(uv, sg, j2) * wk;
        if (k < 2) {
            hit_c = hit_c + c;
        } else {
            sky_c = c * gain;
        }
    }
    let c = (hit_c * hit_w + sky_c * sky_w * (1.0 - hit_w)) / w;
    return vec4<f32>(c, w * fade);
}

// ---- (`--lamp-streaks`, the reflect track; sky.rs `SkyOptions::lamp_streaks`;
// compiled out without SF_LAMP_STREAKS, and into the street's pipelines only;
// the gather's side is sky_lamp_streaks.wgsl) ----
// The cars' rear lamps streak on the wet street as the street lamps and
// headlamps do (`sky_cine_streaks`): the gather writes the rear lamps of the
// cars brightest at the eye as light records past the frame list's cap (a
// lens of the lamp's size, its intensity in the frame's light units, its
// lens's red at unit luminance, a cone round the car's rear axis) and appends
// their indices to the streak list, so the streak loop reflects them through
// the same film lobes with no other change to it (a loop that read them from
// a second place cost the city's opaque pass about a millisecond, compiled in
// and never run). With the switch off the gather appends none.
const SKY_LS_FULL: bool = SF_LAMP_STREAKS && SKY_CINE_FULL;
/// The rear lamps appended at most (two a car, sky_reflect.rs `LS_CARS`).
const SKY_LS_LAMPS: u32 = 12u;

/// (`--lamp-streaks`) The streak list's room past its own cap (0 off).
fn sky_ls_lamps() -> u32 {
    return select(0u, SKY_LS_LAMPS, SKY_LS_FULL);
}
