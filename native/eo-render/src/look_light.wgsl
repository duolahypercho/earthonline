// City shading under the JS game's light rig (appended to draw.wgsl by
// eo-render/src/look.rs). three's physical model, as the game's
// MeshStandardMaterial gets it (three/src/nodes/functions/PhysicalLightingModel.js):
//
//  * direct: the key (the sun, cascaded shadow) and the rim, irradiance
//    E = radiance x N.L, diffuse E x albedo / pi, GGX specular
//    (Smith height-correlated visibility, Schlick Fresnel);
//  * indirect diffuse: hemisphere light (mix(ground, sky, 0.5 n.y + 0.5)) +
//    ambient + the environment's irradiance (SH9 of the analytic sky, standing
//    in for the PMREM'd probe) x scene.environmentIntensity x the material
//    class's envMapIntensity, all x albedo / pi, x the material's AO;
//  * indirect specular: the analytic sky along the reflection (blurred toward
//    the SH irradiance with roughness) x the same intensities, with the
//    analytic split-sum DFG fit;
//  * three's range fog: mix(colour, fog colour, smoothstep(near, far, view depth)).

// envMapIntensity classes a surface may name (Surface.env).
// 0 (a Surface that never set it, e.g. an agent's) and below: none.
const ENV_ASPHALT: i32 = 1;
const ENV_SIDEWALK: i32 = 2;
const ENV_PAINTED_METAL: i32 = 3;
// A film of water over sand (the swash's wet band, landmarks_surface.wgsl): it reflects the environment strongly.
const ENV_WET_FILM: i32 = 4;
// The water pass accounts for air-interface reflection on submerged terrain.
const ENV_SUBMERGED: i32 = 6;
// GLAZING_PROBE_ROUGHNESS_FLOOR (src/render/materials/glazing-environment.js:
// a 4-texel footprint on the 128 px probe).
const GLAZING_ROUGHNESS_FLOOR: f32 = 0.152;

@group(1) @binding(0) var<uniform> look: Look;
@group(1) @binding(1) var lk_shadow_map: texture_depth_2d_array;
@group(1) @binding(2) var lk_shadow_cmp: sampler_comparison;

/// (`--night-photometry`; eo-sky `night_photometry`) the practicals' gain on
/// the emission carried in the rig's own units (lit rooms, signs, the lenses
/// of lamps, signals and vehicles): the look uniform's `fog.z`, 1 off.
fn lk_practical_gain() -> f32 {
    return select(1.0, look.fog.z, SF_NIGHT_PHOTOMETRY && look.fog.z > 0.0);
}

fn lk_sh(n: vec3<f32>) -> vec3<f32> {
    let x = n.x;
    let y = n.y;
    let z = n.z;
    var e = look.sh[0].xyz * 0.282095;
    e = e + look.sh[1].xyz * (0.488603 * y) + look.sh[2].xyz * (0.488603 * z) + look.sh[3].xyz * (0.488603 * x);
    e = e + look.sh[4].xyz * (1.092548 * x * y) + look.sh[5].xyz * (1.092548 * y * z) + look.sh[6].xyz * (0.315392 * (3.0 * z * z - 1.0));
    e = e + look.sh[7].xyz * (1.092548 * x * z) + look.sh[8].xyz * (0.546274 * (x * x - y * y));
    return max(e, vec3<f32>(0.0));
}

// Sun visibility from cascade `c` at a world point (normal offset, depth
// pull-back, four bilinear compares at half-texel offsets: a 3 x 3 texel
// tent), or -1 where the point lies outside the cascade's box and `bounded`
// asks (the probe pass sees round the camera; the blend's next cascade).
fn lk_cascade(c: i32, world: vec3<f32>, ng: vec3<f32>, bounded: bool) -> f32 {
    let b = look.cascade_bias[c];
    let p = world + ng * b.x;
    let clip = look.cascade_vp[c] * vec4<f32>(p, 1.0);
    let uv = vec2<f32>(clip.x * 0.5 + 0.5, 0.5 - clip.y * 0.5);
    let z = clip.z + b.y;
    if (bounded && (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || z < 0.0 || z > 1.0)) {
        return -1.0;
    }
    let texel = 1.0 / f32(textureDimensions(lk_shadow_map).x);
    let h = 0.5 * texel;
    return 0.25 * (textureSampleCompareLevel(lk_shadow_map, lk_shadow_cmp, uv + vec2<f32>(-h, -h), c, z)
        + textureSampleCompareLevel(lk_shadow_map, lk_shadow_cmp, uv + vec2<f32>(h, -h), c, z)
        + textureSampleCompareLevel(lk_shadow_map, lk_shadow_cmp, uv + vec2<f32>(-h, h), c, z)
        + textureSampleCompareLevel(lk_shadow_map, lk_shadow_cmp, uv + vec2<f32>(h, h), c, z));
}

// (`--soft-shadows`, SF_SOFT_SHADOWS; eo-look `penumbra`) the sun's visibility
// through its disc 0.5334 degrees across: percentage-closer soft shadows
// (Fernando 2005). The casters in a disc of PS_MAX_SPREAD_M round the point
// give the distance d up the beam to the edge that shades it: each caster's
// height over the receiver's plane at its own texel, the nearest past
// PS_MIN_CASTER_M (the map holds a caster's sun-facing surface, so the mean
// over a thick or layered caster overstates the distance to the edge, and
// differently either side of it, which bent the band). The comparison is
// then filtered over a disc of the penumbra's half width, d x tan(0.5334
// deg) / 2 (a uniform disc: a straight edge's soft band is the sun's disc
// past a chord). Each tap compares against the receiver's own plane (its
// geometric normal in the cascade's clip space). Only a point with casters
// round it but not all round (the band itself) runs the filter. -1: too
// narrow a penumbra, the cascade's 3 x 3 tent stands. Both discs are
// constant Vogel tables (PS_SEARCH_DISC, PS_FILTER_DISC) turned by `rot`
// (cos, sin): one rotation a pixel, not a sine and a cosine a tap.
fn lk_cascade_soft(c: i32, uv: vec2<f32>, z: f32, b: vec4<f32>, ng: vec3<f32>, rot: vec2<f32>) -> f32 {
    let dims = vec2<f32>(textureDimensions(lk_shadow_map).xy);
    let texel = 1.0 / dims.x;
    // the receiver's plane: dz per uv (rows r0, r1 and the normal solved for the tangent offset).
    let m = look.cascade_vp[c];
    let r0 = vec3<f32>(m[0].x, m[1].x, m[2].x);
    let r1 = vec3<f32>(m[0].y, m[1].y, m[2].y);
    let r2 = vec3<f32>(m[0].z, m[1].z, m[2].z);
    let c1 = cross(r1, ng);
    let c0 = cross(ng, r0);
    let det = dot(r0, c1);
    var g = vec2<f32>(0.0);
    if (abs(det) > 1e-12) {
        g = vec2<f32>(2.0 * dot(r2, c1), -2.0 * dot(r2, c0)) / det;
    }
    let gmax = PS_MAX_PLANE_SLOPE * b.z / (b.w * texel);
    g = clamp(g, vec2<f32>(-gmax), vec2<f32>(gmax));
    // the casters round the point.
    let base = uv * dims;
    let rs = clamp(PS_MAX_SPREAD_M / b.z, 1.0, PS_MAX_RADIUS_TEXELS);
    let ms = mat2x2<f32>(rot.x, rot.y, -rot.y, rot.x) * rs;
    var nb = 0.0;
    var dz = 1e9;
    for (var i = 0u; i < PS_SEARCH_TAPS; i = i + 1u) {
        let t = clamp(floor(base + ms * PS_SEARCH_DISC[i]), vec2<f32>(0.0), dims - 1.0);
        let d = textureLoad(lk_shadow_map, vec2<i32>(t), c, 0);
        let zp = z + dot(g, (t + 0.5 - base) * texel);
        if (d < zp) {
            nb = nb + 1.0;
            let h = (zp - d) * b.w;
            if (h > PS_MIN_CASTER_M) {
                dz = min(dz, h);
            }
        }
    }
    // no caster within the widest penumbra: lit; casters all round: the umbra.
    if (nb < 0.5) {
        return 1.0;
    }
    if (nb > f32(PS_SEARCH_TAPS) - 0.5) {
        return 0.0;
    }
    // only casters touching the receiver: the tent. Else the filter's radius
    // in texels for the edge dz metres up the beam (never past the search's,
    // so the band is the same either side).
    if (dz > 1e8) {
        return -1.0;
    }
    let rf = min(0.5 * dz * PS_TAN_DIAMETER / b.z, rs);
    if (rf <= PS_TENT_TEXELS) {
        return -1.0;
    }
    // the filter's disc turned a further 1.7 rad from the search's (cos 1.7, sin 1.7).
    let rq = vec2<f32>(rot.x * -0.12884449 - rot.y * 0.99166481, rot.y * -0.12884449 + rot.x * 0.99166481);
    let mf = mat2x2<f32>(rq.x, rq.y, -rq.y, rq.x) * (rf * texel);
    var s = 0.0;
    for (var i = 0u; i < PS_FILTER_TAPS; i = i + 1u) {
        let o = mf * PS_FILTER_DISC[i];
        s = s + textureSampleCompareLevel(lk_shadow_map, lk_shadow_cmp, uv + o, c, z + dot(g, o));
    }
    return s / f32(PS_FILTER_TAPS);
}

// (`--soft-shadows`) the cascades' visibility through the sun's disc while the
// penumbra runs (look.cascade_bias[c].w the cascade's depth range; never in
// the probe pass). In the cascade blend's band one cascade is looked up, not
// two: the next one where the pixel's noise falls under the blend's weight
// (the frame's temporal resolve averages the two), so every pixel runs the
// search and filter at most once (a shader the size Apple's compiler takes
// for the agents' pipelines with the cars' materials too, and half the work
// in the band).
fn lk_cascades_soft(c0: i32, world: vec3<f32>, ng: vec3<f32>, vd: f32) -> f32 {
    // per pixel and per frame noise (interleaved gradient noise on the pixel).
    let cp = frame.view_proj * vec4<f32>(world, 1.0);
    let sp = (cp.xy / max(cp.w, 1e-6) * vec2<f32>(0.5, -0.5) + 0.5) * vec2<f32>(textureDimensions(lk_ao)) * 2.0;
    let ign = fract(52.9829189 * fract(dot(floor(sp) + look.post2.w * 5.588238, vec2<f32>(0.06711056, 0.00583715))));
    var c = c0;
    if (LK_SHADOW_BLEND && c0 < 2 && lk_lx_on(LX_BIT_CASCADE_BLEND)) {
        let split = look.cascade_split;
        let end = select(split.x, split.y, c0 == 1);
        let start = select(0.0, split.x, c0 == 1);
        let t = lk_smooth(end - (end - start) * LK_CASCADE_BLEND_BAND, end, vd);
        // the noise turned half a period, so the cascade's pick and the disc's turn are not one pattern;
        // the next cascade only where its box holds the point (else the blend kept its own).
        if (fract(ign + 0.5) < t) {
            let b1 = look.cascade_bias[c0 + 1];
            let clip1 = look.cascade_vp[c0 + 1] * vec4<f32>(world + ng * b1.x, 1.0);
            let uv1 = vec2<f32>(clip1.x * 0.5 + 0.5, 0.5 - clip1.y * 0.5);
            let z1 = clip1.z + b1.y;
            if (all(uv1 >= vec2<f32>(0.0)) && all(uv1 <= vec2<f32>(1.0)) && z1 >= 0.0 && z1 <= 1.0) {
                c = c0 + 1;
            }
        }
    }
    let b = look.cascade_bias[c];
    let clip = look.cascade_vp[c] * vec4<f32>(world + ng * b.x, 1.0);
    let uv = vec2<f32>(clip.x * 0.5 + 0.5, 0.5 - clip.y * 0.5);
    let z = clip.z + b.y;
    let a = ign * 6.2831853;
    let soft = lk_cascade_soft(c, uv, z, b, ng, vec2<f32>(cos(a), sin(a)));
    if (soft >= 0.0) {
        return soft;
    }
    return lk_cascade(c, world, ng, false);
}

// (`--soft-shadows`, eo-look `penumbra::resolves`) whether the widest penumbra
// (PS_MAX_SPREAD_M each way) is wider than the blur the tent already draws:
// the cascade's texel (look.cascade_bias[c].z metres) and PS_SUBPIXEL_PX of the
// frame's pixels at view depth vd (look.ao_params.y: 1 / the focal length in
// pixels). Where it is not, the tent draws the same edge without the search.
fn lk_soft_resolves(c: i32, vd: f32) -> bool {
    return PS_MAX_SPREAD_M > max(look.cascade_bias[c].z * PS_TENT_TEXELS, vd * look.ao_params.y * PS_SUBPIXEL_PX);
}

// Sun visibility 0..1 at a world point: the cascade by view depth, the last
// fading out over the shadow fade (cascade_split.w to .z). With
// `--cascade-blend` (LX_BIT_CASCADE_BLEND) a cascade hands over to the next
// over the last LK_CASCADE_BLEND_BAND of its slice (eo-look
// `cascade_blend_weight`) instead of switching at the split; with
// `--far-shadows` the far map (far_shadow.wgsl) takes over in the fade and
// past it. LK_SHADOW_* (look.rs `ShadowLevels`) say which parts a pipeline
// compiles.
fn lk_shadow(world: vec3<f32>, ng: vec3<f32>) -> f32 {
    var vd = dot(world - look.cam_pos.xyz, look.cam_fwd.xyz);
    let probe = frame.copy_step.z > 0.5;
    if (probe) {
        vd = distance(world, look.cam_pos.xyz); // probe pass
    }
    let split = look.cascade_split;
    if (look.key_dir.w <= 0.0) {
        return 1.0;
    }
    let far = LK_SHADOW_FAR && lk_far_on();
    if (vd >= split.z || !LK_SHADOW_CASCADES) {
        if (!far) {
            return 1.0;
        }
        return mix(1.0, lk_far_shadow(world, ng), look.key_dir.w);
    }
    var c = 0;
    if (vd > split.x) {
        c = 1;
    }
    if (vd > split.y) {
        c = 2;
    }
    var vis = 0.0;
    // (the agents' module, LK_SUN_PRE, keeps the penumbra's branch uniform: with the
    // resolve test in it Apple's compiler aborted their opaque pipeline again, r2i2)
    if (SF_SOFT_SHADOWS && LK_SHADOW_SOFT && look.cascade_bias[0].w > 0.0 && !probe && (LK_SUN_PRE || lk_soft_resolves(c, vd))) {
        vis = lk_cascades_soft(c, world, ng, vd); // (`--soft-shadows`) the penumbra, one cascade a pixel
    } else {
        vis = lk_cascade(c, world, ng, probe);
        if (vis < 0.0) {
            return 1.0; // the probe pass, outside the cascade's box: unshadowed
        }
        if (LK_SHADOW_BLEND && c < 2 && lk_lx_on(LX_BIT_CASCADE_BLEND)) {
            let end = select(split.x, split.y, c == 1);
            let start = select(0.0, split.x, c == 1);
            let t = lk_smooth(end - (end - start) * LK_CASCADE_BLEND_BAND, end, vd);
            if (t > 0.0) {
                let next = lk_cascade(c + 1, world, ng, true);
                if (next >= 0.0) {
                    vis = vis + (next - vis) * t;
                }
            }
        }
    }
    let fade = lk_smooth(split.w, split.z, vd);
    if (far && fade > 0.0) {
        return mix(1.0, mix(vis, lk_far_shadow(world, ng), fade), look.key_dir.w);
    }
    return mix(1.0, mix(vis, 1.0, fade), look.key_dir.w);
}

fn lk_ggx(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, f0: vec3<f32>, rough: f32) -> vec3<f32> {
    let h = normalize(l + v);
    let nl = max(dot(n, l), 0.0);
    let nv = max(dot(n, v), 1e-4);
    let nh = max(dot(n, h), 0.0);
    let vh = max(dot(v, h), 0.0);
    let a = rough * rough;
    let a2 = a * a;
    let dd = nh * nh * (a2 - 1.0) + 1.0;
    let d = a2 / (LK_PI * dd * dd);
    let gv = nl * sqrt(a2 + (1.0 - a2) * nv * nv);
    let gl = nv * sqrt(a2 + (1.0 - a2) * nl * nl);
    let vis = 0.5 / max(gv + gl, 1e-6);
    let f = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - vh, 5.0);
    return f * (vis * d);
}

// Analytic split-sum DFG (three's `DFGApprox`).
fn lk_dfg(nv: f32, rough: f32) -> vec2<f32> {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = rough * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * nv)) * r.x + r.y;
    return vec2<f32>(-1.04, 1.04) * a004 + r.zw;
}

// the room behind kit glazing at night (rooms.wgsl), set by the city
// shader before `look_light`: the pane's emission (kit/materials.js emissiveNode).
var<private> rm_emit: vec3<f32>;
// (`--people-shading`, agents_draw.wgsl `pp_person`) what `look_light` found:
// the key's visibility (shadow x clouds x contact; 0 where it did not look)
// and the fog it laid over the point; and how far past the terminator (n.l
// down to minus this) the caller wants the visibility looked up (skin: its
// wrap's tail; 0, everywhere else, as before). Read and set only by the people.
var<private> lk_key_vis: f32;
var<private> lk_fog_out: f32;
var<private> lk_vis_wrap: f32;

// (`--car-materials`) a car paint's reflection class (agents_draw.wgsl; 0 elsewhere: the surface's own).
var<private> sky_coat_env: f32;

// (`--soft-shadows`) the sun's visibility looked up before the shading
// (LK_SUN_PRE, look.rs `shadow_wgsl`: on only in the agents' module, whose
// fragment sets it first thing, agents_draw.wgsl `ag_shade`). Inside the key's
// branch below, with the cars' materials, the penumbra made Apple's compiler
// abort (an LLVM fatal error in its instruction emission, MTLCompilerService);
// looked up where little else is live it builds.
var<private> lk_sun_pre: f32 = 1.0;

fn look_light(s_in: Surface, m: Material, v: vec3<f32>, world: vec3<f32>, ng: vec3<f32>, object: u32) -> vec4<f32> {
    var s = s_in; // sky
    if ((m.flags & F_GLASS) == 0u && s.env <= 0) {
        sky_wet_generic_at(&s, ng, world); // roofs, ground, walls, bodies in the rain (the street wets itself)
    }
    let n = s.n;
    let rough = s.rough;
    let diffuse = s.albedo * (1.0 - s.metal);
    let f0 = mix(vec3<f32>(0.04), s.albedo, s.metal);
    let glass = (m.flags & F_GLASS) != 0u;
    // envMapIntensity class: glass, roads and the street surface (asphalt),
    // terrain (sidewalk), else painted facade.
    let kind = objects[object].kind & 255u;
    var cls = look.env_class.x;
    if (glass) {
        cls = look.env_class.y;
    } else if (s.env == ENV_ASPHALT) {
        cls = look.env_class.z;
    } else if (s.env == ENV_SIDEWALK) {
        cls = look.env_class.w;
    } else if (s.env == ENV_PAINTED_METAL) {
        cls = ST_ENV_PAINTED_METAL;
    } else if (s.env == ENV_SUBMERGED) {
        cls = 0.0;
    } else if (s.env == ENV_WET_FILM) {
        cls = 0.65 / max(look.ambient.w, 1e-3); // env_i 0.65 (at 1 the film reads as standing water)
    } else if (kind == 2u || kind == 6u || kind == 7u) {
        cls = look.env_class.z;
    } else if (kind == 1u) {
        cls = look.env_class.w;
    }
    let env_i = sky_w2_env(look.ambient.w * cls); // (`--wet-surfaces`) the water's environment whole
    // (`--car-materials`) a car's paint: its reflections (the base's and the
    // lacquer's) at their own class (agents_draw.wgsl sets `sky_coat_env`; 0
    // elsewhere), the diffuse at the surface's; wet (`--wet-surfaces`), the
    // water's environment whole as on any other surface.
    var env_s = env_i;
    if (SF_CAR_MATERIALS && sky_coat_env > 0.0) {
        env_s = sky_w2_env(look.ambient.w * sky_coat_env);
    }

    // Key light with its shadow.
    let l = look.key_dir.xyz;
    let nl = max(dot(n, l), 0.0);
    var key = vec3<f32>(0.0);
    if (nl > -lk_vis_wrap && dot(ng, l) > -0.05 - lk_vis_wrap) {
        var sun_vis = lk_sun_pre;
        if (!LK_SUN_PRE) {
            sun_vis = lk_shadow(world, ng);
        }
        let kv = vec3<f32>(sun_vis, sky_cloud_transmittance(world), sky_contact_shadow(world, ng));
        key = look.key.xyz * max(nl, 0.0) * kv.x * kv.y * kv.z; // cloud and contact shadows
        lk_key_vis = kv.x * kv.y * kv.z; // (`--people-shading`)
        if (SF_STREET_PBR) {
            key = key * sp_sun_vis; // (`--street-pbr`, street_pbr.wgsl) the paving's own relief over the key
        }
    }
    var direct_d = key * diffuse / LK_PI;
    var direct_s = key * lk_ggx(n, v, l, f0, rough);
    // Rim (no shadow).
    let rl = look.rim_dir.xyz;
    let rnl = max(dot(n, rl), 0.0);
    let rim = look.rim.xyz * rnl;
    direct_d = direct_d + rim * diffuse / LK_PI;
    direct_s = direct_s + rim * lk_ggx(n, v, rl, f0, rough) * select(1.0, 0.0, s.env == ENV_WET_FILM); // the rig's rim light draws no false sun in a water film

    // Indirect diffuse.
    let hemi = mix(look.hemi_ground.xyz, look.hemi_sky.xyz, n.y * 0.5 + 0.5);
    var env_e = lk_sh(n);
    // the probe grid (`--probe-grid`, probe_grid.wgsl): the two probes round
    // the point by distance and presence, in the single probe's place (never
    // inside the probe pass).
    let pg = pg_live() && frame.copy_step.z < 0.5;
    var pgs = PgSel(vec4<f32>(0.0), vec4<f32>(0.0), 0u, 0.0, 0.0);
    if (pg) {
        pgs = pg_select(world);
    }
    if (fa_probe_on() && (fa_run() & 16u) != 0u) {
        env_e = fa_probe_irradiance(n); // the bound scene probe's diffuse
    } else if (pg && pgs.cover > 0.0 && (fa_run() & 16u) != 0u) {
        env_e = mix(env_e, pg_irradiance(pgs, n), pgs.cover); // the grid's diffuse (`--probe-yield`)
    }
    var irr = hemi + look.ambient.xyz + env_e * env_i;
    // the ambient volume (`--ambient-volume`, ambient.wgsl): the sky terms
    // by the large-scale sky visibility round the normal.
    let amb = amb_on();
    var av = AMB_OPEN;
    if (amb) {
        if (amb_ol_on()) {
            av = amb_sample_ground(world, ng, n); // (`--occluder-light`) and the street's own sky under the point
        } else {
            av = amb_sample(world, ng);
        }
        irr = amb_irradiance(av, n, env_e * env_i);
        // the ground bounce (`--ground-bounce`, ambient.wgsl): light from the
        // sunlit open ground below (look.lx[0].yzw, zero while off or at night).
        if (AMB_SHADER_LEVELS > 0u) {
            if (any(look.lx[0].yzw > vec3<f32>(0.0))) {
                irr = irr + amb_bounce(amb_ground(world, ng), n, sky_cloud_transmittance(world));
            }
        }
    }
    let nv = max(dot(n, v), 1e-4);
    let dfg = lk_dfg(nv, rough);
    let fss = f0 * dfg.x + vec3<f32>(dfg.y);
    // the scene-scale occlusion and bounce (previous frame).
    var ao4 = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    if (frame.copy_step.z < 0.5) {
        ao4 = ao_fetch(world); // no screen-space AO inside the probe pass
    }
    let ao_vis = ao_multi_bounce(ao4.a);
    let ao = s.ao * ao_vis;
    // (`--ao-albedo`) the multi-bounce at the surface's own albedo on the indirect diffuse.
    var ao_d = vec3<f32>(ao);
    if (SF_AO_ALBEDO && ao_run(2u)) {
        ao_d = s.ao * ao_multi_bounce_albedo(ao4.a, diffuse);
    }
    let indirect_d = irr * diffuse / LK_PI * (vec3<f32>(1.0) - fss) * ao_d;

    // Indirect specular: the sky along the reflection, blurred with roughness.
    let r = reflect(-v, n);
    var rad = lk_sh(r) / LK_PI;
    let sharp = 1.0 - lk_smooth(0.15, 0.5, rough);
    if (fa_probe_on()) {
        // the scene probe is the specular environment (glass reads it
        // through the glazing floor and canyon parallax).
        if (glass) {
            rad = fa_glazing(r, world, rough);
        } else {
            rad = fa_probe_sample(r, fa_probe_lod(rough));
        }
    } else {
        if (sharp > 0.0) {
            rad = mix(rad, lk_sky_sample(r), sharp);
        }
        if (pg && pgs.cover > 0.0) {
            // the probe grid is the specular environment where its probes reach
            // (glass through the glazing floor and the canyon lift).
            rad = mix(rad, pg_reflect(pgs, r, world, rough, glass), pgs.cover);
        }
    }
    let spec_occ = clamp(pow(nv + ao, exp2(-16.0 * rough - 1.0)) - 1.0 + ao, 0.0, 1.0);
    // glossy (wet) ground reflects the previous frame where its ray finds it.
    // (`--look cinematic`, stage L3) on car paint the clearcoat's sharper reflection (below)
    let coat_on = SKY_CINE_FULL && sky_coat > 0.0;
    sky_sr_glass = glass; // (`--screen-reflect`) panes trace too
    let ssr = sky_ssr(world, select(n, ng, coat_on), v, select(rough, sky_coat_rough, coat_on));
    var rad_e = rad * env_s;
    if (amb) {
        // the ambient volume: the environment by the sky seen along the
        // reflection, what blocks it by its own radiance (ambient.wgsl).
        let aw = amb_reflection(av, r);
        rad_e = rad_e * aw.x + amb_occluder_light() * (env_s * aw.y / LK_PI); // (`--occluder-light`: the shaded walls)
    }
    if (SF_CANYON_GLASS && glass && cg_sky.w > 0.0) {
        rad_e = cg_apply(rad) * env_i; // (`--canyon-glass`, rooms.wgsl) the street canyon in kit glass, the volume's occlusion in it
    }
    let indirect_s = mix(rad_e, ssr.rgb, ssr.a) * fss * spec_occ;
    // The bounce is outgoing Lambert radiance (bounce x diffuse albedo), added
    // outside the occlusion multiply as the JS adds it on the emissive slot.
    let bounce = ao4.rgb * diffuse;
    if (look.ao_params.z > 0.5) {
        // EO_AO_DEBUG=1: visibility; =2: the bounce irradiance fraction.
        if (look.ao_params.z > 1.5) {
            return vec4<f32>(ao4.rgb, 1.0);
        }
        return vec4<f32>(vec3<f32>(ao4.a), 1.0);
    }

    var lit = direct_d + direct_s + indirect_d + indirect_s + bounce + m.emissive.rgb * lk_practical_gain() + rm_emit; // + the room (`--night-photometry`: the lenses' gain)
    lit = lit + sky_local_lights_ao(world, n, v, diffuse, f0, rough, ao); // street lamps, headlamps (`ao` and their shadows only with the light shadows on)
    var alpha = s.alpha;
    if (glass && m.emissive.w > 0.0) {
        // the glass's clearcoat layer (three's PhysicalLightingModel:
        // F0 0.04 on the geometry normal, the base attenuated by
        // 1 - clearcoat x F), its environment read never sharper than the
        // JS glazing probe floor (glazing-environment.js, roughness ~0.152).
        let cc = m.emissive.w;
        let ncc = ng;
        let nvc = max(dot(ncc, v), 1e-4);
        let fcc = 0.04 + 0.96 * pow(1.0 - nvc, 5.0);
        let ccr = max(GLAZING_ROUGHNESS_FLOOR, 0.08);
        let rc = reflect(-v, ncc);
        let sharp_c = 1.0 - lk_smooth(0.15, 0.5, ccr);
        var rad_c = mix(lk_sh(rc) / LK_PI, lk_sky_sample(rc), sharp_c);
        if (fa_probe_on()) {
            rad_c = fa_glazing(rc, world, ccr); // facade
        } else if (pg && pgs.cover > 0.0) {
            rad_c = mix(rad_c, pg_reflect(pgs, rc, world, ccr, true), pgs.cover); // the probe grid
        }
        if (SF_CANYON_GLASS && cg_sky.w > 0.0) {
            rad_c = cg_apply(rad_c); // (`--canyon-glass`, rooms.wgsl) the street canyon in kit glass
        }
        rad_c = sky_sr_glass_mix(rad_c, ssr, env_i); // (`--screen-reflect`) the trace over the probes
        let dfg_c = lk_dfg(nvc, ccr);
        let cc_ind = rad_c * env_i * (0.04 * dfg_c.x + dfg_c.y);
        var cc_dir = vec3<f32>(0.0);
        if (nl > 0.0) {
            cc_dir = key * lk_ggx(ncc, v, l, vec3<f32>(0.04), ccr) / max(nl, 1e-4) * max(dot(ncc, l), 0.0);
        }
        lit = lit * (1.0 - cc * fcc) + cc * (cc_ind + cc_dir);
    }
    if (coat_on) {
        // (`--look cinematic`, stage L3) car paint's clearcoat (set by
        // agents_draw.wgsl): F0 0.04 on the geometry normal, its own
        // roughness (grime roughens it), the base attenuated by 1 - coat x F;
        // the sky or the probe grid along its reflection (the street where the
        // screen-space reflection finds it), the key, and the street's lights
        // (gathered in the clusters' loop, `sky_coat_lights`).
        let ccr = sky_coat_rough;
        let nvc = max(dot(ng, v), 1e-4);
        let fcc = 0.04 + 0.96 * pow(1.0 - nvc, 5.0);
        let rc = reflect(-v, ng);
        var rad_c = mix(lk_sh(rc) / LK_PI, lk_sky_sample(rc), 1.0 - lk_smooth(0.15, 0.5, ccr));
        if (pg && pgs.cover > 0.0) {
            rad_c = mix(rad_c, pg_reflect(pgs, rc, world, ccr, false), pgs.cover);
        }
        var rad_ce = rad_c * env_s;
        if (SF_CAR_MATERIALS && amb && sky_coat_env > 0.0) {
            // (`--car-materials`) the lacquer's environment as the base's: by the
            // sky the ambient volume sees along it (a car under a deck reflects the deck)
            let awc = amb_reflection(av, rc);
            rad_ce = rad_ce * awc.x + amb_occluder_light() * (env_s * awc.y / LK_PI);
        }
        rad_c = mix(rad_ce, ssr.rgb, ssr.a); // the street it stands on, where the reflection finds it
        let dfg_c = lk_dfg(nvc, ccr);
        var cc_dir = vec3<f32>(0.0);
        if (nl > 0.0) {
            cc_dir = key * lk_ggx(ng, v, l, vec3<f32>(0.04), ccr) / max(nl, 1e-4) * max(dot(ng, l), 0.0);
        }
        lit = lit * (1.0 - sky_coat * fcc) + sky_coat * (rad_c * (0.04 * dfg_c.x + dfg_c.y) * ao + cc_dir + sky_coat_lights);
    }
    if (glass) {
        let fv = 0.04 + 0.96 * pow(1.0 - nv, 5.0);
        alpha = mix(alpha, 1.0, fv);
        if ((m.flags & F_THIN_GLASS) != 0u) {
            alpha = 1.0 - (1.0 - fv) * (1.0 - m.thin_absorption);
            lit = indirect_s + direct_s;
        }
    }
    // three's range fog on view depth.
    var vd = dot(world - look.cam_pos.xyz, look.cam_fwd.xyz);
    if (frame.copy_step.z > 0.5) {
        vd = distance(world, frame.cam_pos.xyz); // probe pass
    }
    // the JS post chain's aerial path density (post-chain.js aerial
    // stage, not ported before): the haze thins with height (scale height
    // 400 m, a 20 m dead band), so a ray climbing or falling |dh| metres
    // crosses vd (1 - e^-u) / u of it, u = (|dh| - 20) / 400.
    if (frame.copy_step.z < 0.5 && skyu.flags.x != 0u) {
        let u = max(abs(world.y - look.cam_pos.y) - 20.0, 0.0) / 400.0;
        if (u > 1e-2) {
            vd = vd * (1.0 - exp(-u)) / u;
        }
    }
    let fog = lk_smooth(look.fog.x, look.fog.y, lm_fog_depth(m, vd)); // the set's capped sight distance
    lk_fog_out = fog; // (`--people-shading`)
    lit = mix(lit, look.fog_color.xyz, fog);
    return vec4<f32>(lit, alpha);
}
