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

/// The clustered city lights at a surface.
fn sky_local_lights(world: vec3<f32>, n: vec3<f32>, v: vec3<f32>, diffuse: vec3<f32>, f0: vec3<f32>, rough: f32) -> vec3<f32> {
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
            shape = vert * side * near * near;
        } else if (li.kind == 3u) {
            // A props street light: the JS lamp practical's spot (three.js
            // SpotLight: smoothstep(cos angle, cos (angle (1 - penumbra)))).
            shape = smoothstep(li.cos_outer, li.cos_inner, ca);
        }
        let nl = max(dot(n, l), 0.0);
        let e = li.intensity * shape * w * w / d2 * nl;
        if (e > 0.0) {
            out = out + li.color * e * (diffuse / LK_PI + lk_ggx(n, v, l, f0, rough));
        }
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
    let w = skyu.wet.x;
    if (!sky_on(skyu.flags.x, SKY_WET) || w < 0.02) {
        return;
    }
    let darken = select(0.66, 0.58, asphalt);
    let wet_rough = select(0.32, 0.24, asphalt);
    var pud = 0.0;
    if (n_geom.y > 0.95) {
        pud = sky_puddle(world.xz, select(-1.0, kerb, asphalt)) * w;
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
