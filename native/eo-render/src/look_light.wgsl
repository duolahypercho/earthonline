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
const ENV_LOCAL_PROBE: i32 = 5;
const ENV_INTERIOR_FLOOR: i32 = 6;
// GLAZING_PROBE_ROUGHNESS_FLOOR (src/render/materials/glazing-environment.js:
// a 4-texel footprint on the 128 px probe).
const GLAZING_ROUGHNESS_FLOOR: f32 = 0.152;

@group(1) @binding(0) var<uniform> look: Look;
@group(1) @binding(1) var lk_shadow_map: texture_depth_2d_array;
@group(1) @binding(2) var lk_shadow_cmp: sampler_comparison;

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

// Sun visibility 0..1 at a world point (cascade by view depth, normal offset,
// depth pull-back, 4-tap bilinear PCF, the last cascade fading out).
fn lk_shadow(world: vec3<f32>, ng: vec3<f32>) -> f32 {
    var vd = dot(world - look.cam_pos.xyz, look.cam_fwd.xyz);
    if (frame.copy_step.z > 0.5) {
        vd = distance(world, look.cam_pos.xyz); // probe pass
    }
    let split = look.cascade_split;
    if (vd >= split.z || look.key_dir.w <= 0.0) {
        return 1.0;
    }
    var c = 0;
    if (vd > split.x) {
        c = 1;
    }
    if (vd > split.y) {
        c = 2;
    }
    let b = look.cascade_bias[c];
    let p = world + ng * b.x;
    let clip = look.cascade_vp[c] * vec4<f32>(p, 1.0);
    let uv = vec2<f32>(clip.x * 0.5 + 0.5, 0.5 - clip.y * 0.5);
    let z = clip.z + b.y;
    // the probe pass sees round the camera; outside the cascade box is unshadowed.
    if (frame.copy_step.z > 0.5 && (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || z < 0.0 || z > 1.0)) {
        return 1.0;
    }
    let texel = 1.0 / f32(textureDimensions(lk_shadow_map).x);
    // Four bilinear compares at half-texel offsets: a 3 x 3 texel tent.
    let h = 0.5 * texel;
    let vis = 0.25 * (textureSampleCompareLevel(lk_shadow_map, lk_shadow_cmp, uv + vec2<f32>(-h, -h), c, z)
        + textureSampleCompareLevel(lk_shadow_map, lk_shadow_cmp, uv + vec2<f32>(h, -h), c, z)
        + textureSampleCompareLevel(lk_shadow_map, lk_shadow_cmp, uv + vec2<f32>(-h, h), c, z)
        + textureSampleCompareLevel(lk_shadow_map, lk_shadow_cmp, uv + vec2<f32>(h, h), c, z));
    let fade = lk_smooth(split.w, split.z, vd);
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
// Set by the house vertices that select a local reflection, consumed by enamel and waxed boards.
var<private> room_radiance: vec3<f32>;
var<private> oak_coated: bool;
var<private> oak_coat_radiance: vec3<f32>;
var<private> oak_daylight: vec3<f32>;
var<private> oak_pendant: vec3<f32>;
var<private> room_enclosure: f32;
var<private> room_daylight: f32;
var<private> measured_room: bool;
var<private> room_diffuse: vec3<f32>;
var<private> surface_fixture: vec3<f32>;
var<private> dielectric_scale: f32;

fn look_light(s_in: Surface, m: Material, v: vec3<f32>, world: vec3<f32>, ng: vec3<f32>, object: u32) -> vec4<f32> {
    var s = s_in; // sky
    if ((m.flags & F_GLASS) == 0u && s.env <= 0 && room_enclosure < 0.5) {
        sky_wet_generic(&s, ng); // roofs, ground, walls, bodies in the rain (the street wets itself)
    }
    let n = s.n;
    let rough = s.rough;
    let diffuse = s.albedo * (1.0 - s.metal);
    let f0 = mix(vec3<f32>(0.04 * select(1.,dielectric_scale,dielectric_scale>0.)), s.albedo, s.metal);
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
    } else if (s.env == ENV_WET_FILM) {
        cls = 0.65 / max(look.ambient.w, 1e-3); // env_i 0.65 (at 1 the film reads as standing water)
    } else if (kind == 2u || kind == 6u || kind == 7u) {
        cls = look.env_class.z;
    } else if (kind == 1u) {
        cls = look.env_class.w;
    }
    let env_i = look.ambient.w * cls;

    // Key light with its shadow.
    let l = look.key_dir.xyz;
    let nl = max(dot(n, l), 0.0);
    var key = vec3<f32>(0.0);
    if (nl > 0.0 && dot(ng, l) > -0.05) {
        key = look.key.xyz * nl * lk_shadow(world, ng) * sky_cloud_transmittance(world) * sky_contact_shadow(world, ng); // cloud and contact shadows
    }
    var direct_d = key * diffuse / LK_PI;
    var direct_s = key * lk_ggx(n, v, l, f0, rough);
    // Rim (no shadow).
    let rl = look.rim_dir.xyz;
    let rnl = max(dot(n, rl), 0.0);
    let rim = look.rim.xyz * rnl * (1.0 - room_enclosure);
    direct_d = direct_d + rim * diffuse / LK_PI;
    direct_s = direct_s + rim * lk_ggx(n, v, rl, f0, rough) * select(1.0, 0.0, s.env == ENV_WET_FILM || s.env == ENV_LOCAL_PROBE || s.env == ENV_INTERIOR_FLOOR); // the rig's rim light draws no false sun in a water film

    // Indirect diffuse.
    let hemi = mix(look.hemi_ground.xyz, look.hemi_sky.xyz, n.y * 0.5 + 0.5);
    var env_e = lk_sh(n);
    if (fa_probe_on() && (fa_run() & 16u) != 0u) {
        env_e = fa_probe_irradiance(n); // the bound scene probe's diffuse
    }
    let sky_field = hemi + look.ambient.xyz + env_e * env_i;
    let indoor_field = (0.30 + 0.70 * sqrt(room_daylight)) * vec3<f32>(1.0, 0.92, 0.80);
    let irr = select(sky_field * mix(vec3<f32>(1.0), indoor_field, room_enclosure), room_diffuse*LK_PI, measured_room);
    let nv = max(dot(n, v), 1e-4);
    let dfg = lk_dfg(nv, rough);
    let fss = f0 * dfg.x + vec3<f32>(dfg.y);
    // the scene-scale occlusion and bounce (previous frame).
    var ao4 = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    if (frame.copy_step.z < 0.5) {
        ao4 = ao_fetch(world); // no screen-space AO inside the probe pass
    }
    let ao_vis = ao_multi_bounce(ao4.a);
    // The measured room contains the furniture and walls. Its vertex contact
    // term resolves folds below the probe spacing; the broad screen-space
    // visibility would darken that same lighting a second time.
    let ao = s.ao * select(ao_vis, 1., measured_room);
    var indirect_d = irr * diffuse / LK_PI * (vec3<f32>(1.0) - fss) * ao;
    if ((objects[object].kind & TABLE_LIGHT_BIT) != 0u) {
        indirect_d += surface_fixture * diffuse * (vec3<f32>(1.0) - fss);
    }

    // Indirect specular: the sky along the reflection, blurred with roughness.
    let r = reflect(-v, n);
    var rad = lk_sh(r) / LK_PI;
    let sharp = 1.0 - lk_smooth(0.15, 0.5, rough);
    if (s.env == ENV_LOCAL_PROBE || s.env == ENV_INTERIOR_FLOOR) {
        rad = room_radiance;
    } else if (fa_probe_on()) {
        // the scene probe is the specular environment (glass reads it
        // through the glazing floor and canyon parallax).
        if (glass) {
            rad = fa_glazing(r, world, rough);
        } else {
            rad = fa_probe_sample(r, fa_probe_lod(rough));
        }
    } else if (sharp > 0.0) {
        rad = mix(rad, lk_sky_sample(r), sharp);
    }
    if (room_enclosure > 0.0 && s.env != ENV_LOCAL_PROBE && s.env != ENV_INTERIOR_FLOOR) {
        let local = select(indoor_field * (hemi + look.ambient.xyz + env_e * env_i) / LK_PI, room_diffuse + surface_fixture, measured_room);
        rad = mix(rad, local, room_enclosure * smoothstep(0.18, 0.75, rough));
    }
    let spec_occ = clamp(pow(nv + ao, exp2(-16.0 * rough - 1.0)) - 1.0 + ao, 0.0, 1.0);
    // glossy (wet) ground reflects the previous frame where its ray finds it.
    if (s.env != ENV_LOCAL_PROBE && s.env != ENV_INTERIOR_FLOOR) {
        let ssr = sky_ssr(world, n, v, rough);
        rad = mix(rad * env_i, ssr.rgb, ssr.a);
    }
    let indirect_s = rad * fss * spec_occ;
    // The bounce is outgoing Lambert radiance (bounce x diffuse albedo), added
    // outside the occlusion multiply as the JS adds it on the emissive slot.
    let bounce = select(ao4.rgb * diffuse, vec3<f32>(0.), measured_room);
    if (look.ao_params.z > 0.5) {
        // EO_AO_DEBUG=1: visibility; =2: the bounce irradiance fraction.
        if (look.ao_params.z > 1.5) {
            return vec4<f32>(ao4.rgb, 1.0);
        }
        return vec4<f32>(vec3<f32>(ao4.a), 1.0);
    }

    var lit = direct_d + direct_s + indirect_d + indirect_s + bounce + m.emissive.rgb + rm_emit; // + the room
    lit = lit + sky_local_lights(world, n, v, diffuse, f0, rough); // street lamps, headlamps
    if (oak_coated) {
        let finish = look.oak_finish[1];
        let coat_nv = max(dot(ng,v),.0001);
        let grazing = finish.w + (1.-finish.w)*pow(1.-coat_nv,5.);
        let dfg_coat = lk_dfg(coat_nv,finish.z);
        var wax = oak_coat_radiance*(finish.w*dfg_coat.x+dfg_coat.y);
        if (nl>0.) {
            wax += key*(max(dot(ng,l),0.)/nl)*lk_ggx(ng,v,l,vec3<f32>(finish.w),finish.z);
        }
        if (any(oak_pendant > vec3<f32>(0.))) {
            wax += pendant_gloss(oak_pendant,world,ng,v,finish.z);
        }
        lit = lit*(1.-finish.y*grazing) + finish.y*wax;
    }
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
        }
        let dfg_c = lk_dfg(nvc, ccr);
        let cc_ind = rad_c * env_i * (0.04 * dfg_c.x + dfg_c.y);
        var cc_dir = vec3<f32>(0.0);
        if (nl > 0.0) {
            cc_dir = key * lk_ggx(ncc, v, l, vec3<f32>(0.04), ccr) / max(nl, 1e-4) * max(dot(ncc, l), 0.0);
        }
        lit = lit * (1.0 - cc * fcc) + cc * (cc_ind + cc_dir);
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
    lit = mix(lit, look.fog_color.xyz, fog);
    return vec4<f32>(lit, alpha);
}
