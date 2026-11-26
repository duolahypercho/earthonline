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
// The house's local reflections (draw.wgsl: the kitchen's captured room, the fitted
// finishes, the entry brass): no screen-space term or environment intensity over them.
const ENV_LOCAL_PROBE: i32 = 5;
// The water pass accounts for air-interface reflection on submerged terrain.
const ENV_SUBMERGED: i32 = 6;
// The house's waxed boards, reflecting their surveyed room (draw.wgsl `floor_reflected_room`).
const ENV_INTERIOR_FLOOR: i32 = 7;
// GLAZING_PROBE_ROUGHNESS_FLOOR (src/render/materials/glazing-environment.js:
// a 4-texel footprint on the 128 px probe).
const GLAZING_ROUGHNESS_FLOOR: f32 = 0.152;

@group(1) @binding(0) var<uniform> look: Look;
@group(1) @binding(1) var lk_shadow_map: texture_depth_2d_array;
@group(1) @binding(2) var lk_shadow_cmp: sampler_comparison;
// the atmosphere's sky-view table (sky_atmo.wgsl), read for the physical daylight's sky
// (group 1 binding 20, look.rs `B_ATMO_VIEW`: after the lighting additions' 12-19, look_ext.wgsl)
@group(1) @binding(20) var lk_atmo_view: texture_2d<f32>;
// (`--far-street-light`) the street lamps' pools on the ground, lux (sky.rs `bake_lamp_map`, `LAMP_MAP_*`).
@group(1) @binding(22) var lk_lamp_map: texture_2d<f32>;
const LK_LAMP_MAP_SIZE: f32 = 3072.0;
const LK_LAMP_MAP_TEXEL_M: f32 = 8.0;
const LK_LAMP_MAP_ORIGIN_M: f32 = -12288.0;

// (`--far-street-light`) a street's light from its lamps' pools past the frame's
// lamp lights: the map's lux at the point, as outgoing Lambert radiance per albedo,
// where the clustered lights stop (sky_light.wgsl `sky_local_lights_ao`: past the
// clusters' far plane in forward depth, `skyu.cluster.y`), over a band of a few metres.
fn lk_far_street_light(world: vec3<f32>, eye: vec3<f32>) -> vec3<f32> {
    if (skyu.spare.w <= 0.0) {
        return vec3<f32>(0.0);
    }
    let vz = dot(world - skyu.eye.xyz, skyu.cam_fwd.xyz);
    var w = smoothstep(skyu.cluster.y - 2.0, skyu.cluster.y + 2.0, vz);
    if (frame.copy_step.z > 0.5) {
        w = 1.0; // the probe pass draws no clustered lights (`sky_local_lights_ao`): the map is the lamps' light in every face
    }
    if (w <= 0.0) {
        return vec3<f32>(0.0);
    }
    let uv = (world.xz - vec2<f32>(LK_LAMP_MAP_ORIGIN_M)) / (LK_LAMP_MAP_TEXEL_M * LK_LAMP_MAP_SIZE);
    let lux = textureSampleLevel(lk_lamp_map, lk_ao_samp, uv, 0.0).r;
    return skyu.spare.rgb * (lux * w / LK_PI);
}

// the physical daylight's weight in this frame (eo-look `phys`; 0 with the rig's light,
// and compiled out with `--no-phys-sky` at start: look.rs `ShaderFeatures::phys_sky`)
fn lk_phys() -> f32 {
    if (!SF_PHYS_SKY) {
        return 0.0;
    }
    return clamp(look.hemi_sky.w, 0.0, 1.0);
}

// the sky the physical daylight draws, along a reflection: the sky-view table at the
// sun's own scale (`skyu.atmo4`) above the horizon, and under it the SH9 of the ground's
// and the sky's light (the ground's radiance), blended over the horizon's few degrees.
fn lk_phys_sky(r: vec3<f32>) -> vec3<f32> {
    let up = normalize(vec3<f32>(r.x, max(r.y, 0.0), r.z) + vec3<f32>(0.0, 1e-4, 0.0));
    let sky = at_spectral(textureSampleLevel(lk_atmo_view, lk_ao_samp, at_view_uv(up, skyu.atmo.xyz), 0.0).rgb * skyu.atmo4.rgb, 1.0);
    let below = lk_sh(r) / LK_PI;
    return mix(below, sky, smoothstep(-0.03, 0.02, r.y));
}

// a specular environment read toward the physical sky by the daylight's weight
fn lk_sky_spec(r: vec3<f32>, rig: vec3<f32>) -> vec3<f32> {
    let w = lk_phys();
    if (w <= 0.0) {
        return rig;
    }
    return mix(rig, lk_phys_sky(r), w);
}

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
    let nl = dot(ng, look.key_dir.xyz);
    // On grazing faces the clamped receiver-plane slope cannot extrapolate accurately
    // across a wide PCF disc and causes diagonal shadow-map texel diamonds; fall back
    // to the tight 3x3 hardware tent where direct sun is already grazing out.
    if (nl <= 0.14) {
        return -1.0;
    }
    let g_fade = smoothstep(0.14, 0.48, nl);
    let grazing_pad = (1.0 - clamp(nl, 0.0, 1.0)) * 0.75 * b.y;
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
    // Coherent blocker search capped to a dense texel footprint so adjacent pixels
    // agree smoothly on caster distance instead of producing salt-and-pepper shadow dots.
    let base = uv * dims;
    let rs = clamp(PS_MAX_SPREAD_M / b.z, 1.0, min(PS_MAX_RADIUS_TEXELS, 9.0)) * g_fade;
    if (rs <= PS_TENT_TEXELS) {
        return -1.0;
    }
    var nb = 0.0;
    var dz_min = 1e9;
    var dz_sum = 0.0;
    var dz_cnt = 0.0;
    for (var i = 0u; i < PS_SEARCH_TAPS; i = i + 1u) {
        let t = clamp(floor(base + PS_SEARCH_DISC[i] * rs), vec2<f32>(0.0), dims - 1.0);
        let d = textureLoad(lk_shadow_map, vec2<i32>(t), c, 0);
        let zp = z + grazing_pad + dot(g, (t + 0.5 - base) * texel);
        if (d < zp) {
            nb = nb + 1.0;
            let h = (zp - d) * b.w;
            if (h > PS_MIN_CASTER_M) {
                dz_min = min(dz_min, h);
                dz_sum = dz_sum + h;
                dz_cnt = dz_cnt + 1.0;
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
    if (dz_cnt < 0.5) {
        return -1.0;
    }
    // Blend nearest caster with mean caster distance for spatial stability, and keep the
    // filter radius within the dense coverage of 16 overlapping 2x2 bilinear hardware PCF taps
    // so the penumbra reads as a continuous soft shadow rather than isolated stipple dots.
    let dz = mix(dz_min, dz_sum / dz_cnt, 0.35);
    let rf = min(0.5 * dz * PS_TAN_DIAMETER / b.z, min(rs, 6.0)) * g_fade;
    if (rf <= PS_TENT_TEXELS) {
        return -1.0;
    }
    let mf = mat2x2<f32>(rot.x, rot.y, -rot.y, rot.x) * (rf * texel);
    var s = 0.0;
    var wsum = 0.0;
    for (var i = 0u; i < PS_FILTER_TAPS; i = i + 1u) {
        let tap = PS_FILTER_DISC[i];
        let o = mf * tap;
        let w = exp(-1.25 * dot(tap, tap));
        s = s + w * textureSampleCompareLevel(lk_shadow_map, lk_shadow_cmp, uv + o, c, z + grazing_pad + dot(g, o));
        wsum = wsum + w;
    }
    return s / max(wsum, 1e-6);
}

// (`--soft-shadows`) the cascades' visibility through the sun's disc while the
// penumbra runs (look.cascade_bias[c].w the cascade's depth range; never in
// the probe pass). Uses a tiny spatially-ordered 2x2 phase rotation instead of
// per-pixel random noise and blends cascade splits smoothly rather than
// stochastically dithering them, eliminating crawling dot patterns in live play.
fn lk_cascades_soft(c0: i32, world: vec3<f32>, ng: vec3<f32>, vd: f32) -> f32 {
    let cp = frame.view_proj * vec4<f32>(world, 1.0);
    let sp = (cp.xy / max(cp.w, 1e-6) * vec2<f32>(0.5, -0.5) + 0.5) * vec2<f32>(textureDimensions(lk_ao)) * 2.0;
    let q2 = vec2<u32>(abs(floor(sp))) & vec2<u32>(1u);
    let bayer = f32((q2.x ^ (q2.y << 1u)) + (q2.y << 1u)) * 0.25;
    // Small coherent sub-tap turn (~0..22 deg) keeps overlapping bilinear PCF smooth on every frame.
    let a = bayer * 0.3926991;
    let b = look.cascade_bias[c0];
    let clip = look.cascade_vp[c0] * vec4<f32>(world + ng * b.x, 1.0);
    let uv = vec2<f32>(clip.x * 0.5 + 0.5, 0.5 - clip.y * 0.5);
    let z = clip.z + b.y;
    var vis = lk_cascade_soft(c0, uv, z, b, ng, vec2<f32>(cos(a), sin(a)));
    if (vis < 0.0) {
        vis = lk_cascade(c0, world, ng, false);
    }
    if (LK_SHADOW_BLEND && c0 < 2 && lk_lx_on(LX_BIT_CASCADE_BLEND)) {
        let split = look.cascade_split;
        let end = select(split.x, split.y, c0 == 1);
        let start = select(0.0, split.x, c0 == 1);
        let t = lk_smooth(end - (end - start) * LK_CASCADE_BLEND_BAND, end, vd);
        if (t > 0.0) {
            let next = lk_cascade(c0 + 1, world, ng, true);
            if (next >= 0.0) {
                vis = mix(vis, next, t);
            }
        }
    }
    return vis;
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
// sf:PROP_FAR+
// (`--prop-far-shadow`) the far map's sun visibility where a pipeline compiles
// no far map of its own: the props set it from their vertex shader's lookup
// (props_draw.wgsl `pr_far_sun`); 1 everywhere else, as before.
var<private> lk_far_vertex: f32 = 1.0;
// sf:end
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
// sf:PROP_FAR-
            return 1.0;
// sf:end
// sf:PROP_FAR+
            return mix(1.0, lk_far_vertex, look.key_dir.w);
// sf:end
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
// sf:PROP_FAR-
    return mix(1.0, mix(vis, 1.0, fade), look.key_dir.w);
// sf:end
// sf:PROP_FAR+
    return mix(1.0, mix(vis, lk_far_vertex, fade), look.key_dir.w);
// sf:end
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

// Site irradiance is set by the fitted garden and exterior plaster receivers.
var<private> front_ambient: vec3<f32>;
var<private> measured_front: f32;
var<private> measured_wall: f32;
var<private> joinery_diffuse: vec4<f32>;
var<private> porch_field: vec3<f32>;
var<private> porch_measured: f32;
// the share of a landmark wall that is glazing (landmarks_surface.wgsl
// `lm_authored3`): its environment reflection takes the glass class.
var<private> lm_glaze: f32;
// the share of a landmark wall's pixel its own relief shades from the key
// light (landmarks_surface.wgsl `lm_authored4`: fins, sunshades and reveals).
var<private> lm_unlit: f32;

// (`--people-shading`, agents_draw.wgsl `pp_person`) what `look_light` found:
// the key's visibility (shadow x clouds x contact; 0 where it did not look)
// and the fog it laid over the point; and how far past the terminator (n.l
// down to minus this) the caller wants the visibility looked up (skin: its
// wrap's tail; 0, everywhere else, as before). Read and set only by the people.
var<private> lk_key_vis: f32;
var<private> lk_fog_out: f32;
var<private> lk_vis_wrap: f32;
// sf:CROWN+
// (`--crown-light`) the share of the key `look_light` lays on the surface's own
// normal: 0 for a far crown, whose sun the far trees add as a volume's
// (hlod_trees.wgsl `ft_crown`, reading `lk_key_vis` and `lk_fog_out` as the
// people do, with `lk_vis_wrap` set so the visibility is always looked up); 1
// everywhere else, as before.
var<private> lk_leaf_sun: f32 = 1.0;
// sf:end

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
    if ((m.flags & F_GLASS) == 0u && s.env <= 0 && (!SF_HOUSE || room_enclosure < 0.5)) { // (not inside the house's rooms)
        sky_wet_generic_at(&s, ng, world, object != 0u); // roofs, ground, walls, bodies in the rain (the street wets itself)
    }
    let n = s.n;
    let rough = s.rough;
    let diffuse = s.albedo * (1.0 - s.metal);
    var f0 = mix(vec3<f32>(0.04), s.albedo, s.metal);
    if (SF_HOUSE && dielectric_scale > 0.0) {
        f0 = mix(vec3<f32>(0.04 * dielectric_scale), s.albedo, s.metal); // (the house's fitted cloth and oiled boards)
    }
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
    // the rig's envMapIntensity classes stand in for the light a probe delivered; the
    // physical daylight's environment is the light itself, at its own intensity
    // (landmark glazing, landmarks_surface.wgsl `lm_glaze`: the glass class; `--wet-surfaces`: the water's environment whole)
    let env_i = sky_w2_env(mix(look.ambient.w * mix(cls, look.env_class.y, clamp(lm_glaze, 0.0, 1.0)), 1.0, lk_phys()));
    // (`--car-materials`) a car's paint: its reflections (the base's and the
    // lacquer's) at their own class (agents_draw.wgsl sets `sky_coat_env`; 0
    // elsewhere), the diffuse at the surface's; wet (`--wet-surfaces`), the
    // water's environment whole as on any other surface.
    var env_s = env_i;
    if (SF_CAR_MATERIALS && sky_coat_env > 0.0) {
        env_s = sky_w2_env(mix(look.ambient.w * sky_coat_env, 1.0, lk_phys()));
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
        key = look.key.xyz * max(nl, 0.0) * kv.x * kv.y * kv.z * lm_key_vis * (1.0 - lm_unlit); // cloud and contact shadows (and a bridge deck's, a landmark's relief: landmarks_surface.wgsl)
        lk_key_vis = kv.x * kv.y * kv.z; // (`--people-shading`)
// sf:CROWN+
        key = key * lk_leaf_sun; // (`--crown-light`)
// sf:end
        if (SF_STREET_PBR) {
            key = key * sp_sun_vis; // (`--street-pbr`, street_pbr.wgsl) the paving's own relief over the key
        }
        if (SF_FAR_RELIEF || SF_WINDOW_DEPTH) {
            key = key * fr_sun_vis; // (`--far-relief`, hlod_draw.wgsl) the far layer's relief over the key; (`--window-depth`, rooms.wgsl) a kit pane's reveal
        }
    }
    var direct_d = key * diffuse / LK_PI;
    var direct_s = key * lk_ggx(n, v, l, f0, rough);
    // Rim (no shadow).
    let rl = look.rim_dir.xyz;
    let rnl = max(dot(n, rl), 0.0);
    var rim = look.rim.xyz * rnl * lm_rim_vis; // (a bridge's night: landmarks_surface.wgsl)
    if (SF_HOUSE && (room_enclosure > 0.0 || measured_wall > 0.0)) {
        rim = rim * (1.0 - room_enclosure) * (1.0 - measured_wall); // (the house's enclosed and measured surfaces)
    }
    direct_d = direct_d + rim * diffuse / LK_PI;
// sf:WEATHER_FX_FLOOR+
    var rim_spec_w = select(1.0, 0.0, s.env == ENV_WET_FILM || s.env == ENV_LOCAL_PROBE || s.env == ENV_INTERIOR_FLOOR); // the rig's rim light draws no false sun in a water film
    if (sky_weather_fx_on(skyu.cirrus) && skyu.wet.x > 0.05 && ng.y > 0.92) {
        // Suppress the rig's synthetic rim-light glare on glossy rain puddles so storm puddles reflect the sky instead of a false white sun.
        rim_spec_w = rim_spec_w * (1.0 - smoothstep(0.32, 0.06, rough) * skyu.wet.x);
    }
    direct_s = direct_s + rim * lk_ggx(n, v, rl, f0, rough) * rim_spec_w;
// sf:end
// sf:WEATHER_FX_FLOOR-
    direct_s = direct_s + rim * lk_ggx(n, v, rl, f0, rough) * select(1.0, 0.0, s.env == ENV_WET_FILM || s.env == ENV_LOCAL_PROBE || s.env == ENV_INTERIOR_FLOOR); // the rig's rim light draws no false sun in a water film
// sf:end

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
        if (amb_ol_on() && AMB_OL_STREET) {
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
    // the playable house (`--house-*`, SF_HOUSE): inside, the room's daylight transfer
    // or its measured irradiance; outside, the measured site irradiance.
    let indoor_field = (0.30 + 0.70 * sqrt(room_daylight)) * vec3<f32>(1.0, 0.92, 0.80);
    if (SF_HOUSE && (measured_room || room_enclosure > 0.0 || measured_front > 0.0)) {
        irr = select(irr * mix(vec3<f32>(1.0), indoor_field, room_enclosure), room_diffuse * LK_PI, measured_room);
        irr = mix(irr, front_ambient, measured_front);
    }
    let nv = max(dot(n, v), 1e-4);
    let dfg = lk_dfg(nv, rough);
    let fss = f0 * dfg.x + vec3<f32>(dfg.y);
    // the scene-scale occlusion and bounce (previous frame).
    var ao4 = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    if (frame.copy_step.z < 0.5 && !(SF_HOUSE && measured_room)) {
        ao4 = ao_fetch(world); // no screen-space AO inside the probe pass (nor in the house's measured rooms: their bake holds it)
        if (SF_HOUSE && measured_front > 0.0) {
            ao4 = vec4<f32>(ao4.rgb * (1.0 - measured_front), ao4.a); // (the house's measured site carries its share of the bounce)
        }
    }
    let ao_vis = ao_multi_bounce(ao4.a);
    // The measured room contains the furniture and walls. Its vertex contact
    // term resolves folds below the probe spacing; the broad screen-space
    // visibility would darken that same lighting a second time.
    var ao = s.ao * ao_vis;
    if (SF_HOUSE && (measured_room || measured_front > 0.0 || measured_wall > 0.0)) {
        ao = mix(s.ao * select(mix(ao_vis, 1., measured_front), 1., measured_room), 1., measured_wall);
    }
    // (`--ao-albedo`) the multi-bounce at the surface's own albedo on the indirect diffuse.
    var ao_d = vec3<f32>(ao);
    if (SF_AO_ALBEDO && ao_run(2u) && !(SF_HOUSE && (measured_room || measured_front > 0.0 || measured_wall > 0.0))) {
        ao_d = s.ao * ao_multi_bounce_albedo(ao4.a, diffuse);
    }
    var indirect_d = irr * diffuse / LK_PI * (vec3<f32>(1.0) - fss) * ao_d;
    if (SF_HOUSE && (objects[object].kind & TABLE_LIGHT_BIT) != 0u) {
        indirect_d += surface_fixture * diffuse * (vec3<f32>(1.0) - fss);
    }

    // Indirect specular: the sky along the reflection, blurred with roughness.
    let r = reflect(-v, n);
    var rad = lk_sh(r) / LK_PI;
    let sharp = 1.0 - lk_smooth(0.15, 0.5, rough);
    if (SF_HOUSE && (s.env == ENV_LOCAL_PROBE || s.env == ENV_INTERIOR_FLOOR)) { // (only the house's surfaces take these classes)
        rad = room_radiance;
    } else if (fa_probe_on()) {
        // the scene probe is the specular environment (glass reads it
        // through the glazing floor and canyon parallax).
        if (glass) {
            rad = fa_glazing(r, world, rough);
        } else {
            rad = fa_probe_sample(r, fa_probe_lod(rough));
        }
    } else {
        if (sharp > 0.0) {
            rad = mix(rad, lk_sky_spec(r, lk_sky_sample(r)), sharp);
        }
        if (pg && pgs.cover > 0.0) {
            // the probe grid is the specular environment where its probes reach
            // (glass through the glazing floor and the canyon lift).
            rad = mix(rad, pg_reflect(pgs, r, world, rough, glass), pgs.cover);
        }
    }
    if (SF_HOUSE && room_enclosure > 0.0 && s.env != ENV_LOCAL_PROBE && s.env != ENV_INTERIOR_FLOOR) {
        let local = select(indoor_field * (hemi + look.ambient.xyz + env_e * env_i) / LK_PI, room_diffuse + surface_fixture, measured_room);
        rad = mix(rad, local, room_enclosure * smoothstep(0.18, 0.75, rough));
    }
    let spec_occ = clamp(pow(nv + ao, exp2(-16.0 * rough - 1.0)) - 1.0 + ao, 0.0, 1.0);
    // glossy (wet) ground reflects the previous frame where its ray finds it.
    // (`--look cinematic`, stage L3) on car paint the clearcoat's sharper reflection (below)
    let coat_on = SKY_CINE_FULL && sky_coat > 0.0;
    // the house's kitchen and its other local probes reflect their own captured room
    // (draw.wgsl `kitchen_reflection`, `floor_reflected_room`): no screen-space term,
    // ambient volume or environment intensity over it.
    let kitchen = SF_HOUSE && (s.env == ENV_LOCAL_PROBE || s.env == ENV_INTERIOR_FLOOR);
    var ssr = vec4<f32>(0.0);
    if (!kitchen) {
        sky_sr_glass = glass; // (`--screen-reflect`) panes trace too
        ssr = sky_ssr(world, select(n, ng, coat_on), v, select(rough, sky_coat_rough, coat_on));
    }
    var rad_e = rad;
    if (!kitchen) {
        rad_e = rad * env_s;
        if (amb) {
            // the ambient volume: the environment by the sky seen along the
            // reflection, what blocks it by its own radiance (ambient.wgsl).
            let aw = amb_reflection(av, r);
            rad_e = rad_e * aw.x + amb_occluder_light() * (env_s * aw.y / LK_PI); // (`--occluder-light`: the shaded walls)
        }
        if (SF_CANYON_GLASS && glass && cg_sky.w > 0.0) {
            rad_e = cg_apply(rad) * env_i; // (`--canyon-glass`, rooms.wgsl) the street canyon in kit glass, the volume's occlusion in it
        }
    }
    var indirect_s = sky_sr_spec(rad_e, ssr, fss, spec_occ); // (`--screen-reflect`) the occlusion on what the trace did not find
    if (SF_HOUSE && measured_wall > 0.0) {
        // Rough plaster sees the enclosing passage in its broad reflection (the house's measured walls).
        let spec_e = mix(mix(rad_e, ssr.rgb, ssr.a), front_ambient / LK_PI, measured_wall * smoothstep(0.30, 0.75, rough));
        indirect_s = spec_e * fss * spec_occ;
    }
    // (the playable house, SF_HOUSE) its source-lit joinery takes its measured diffuse
    // light in place of the rig's. The house's terms replace the rig's inputs (here and
    // at the occlusion fetch above), so the default frame's sums stay the same
    // expressions, bit for bit, with the house compiled in or out.
    if (SF_HOUSE && joinery_diffuse.a > 0.) {
        direct_d = joinery_diffuse.rgb;
        indirect_d = vec3<f32>(0.0);
        ao4 = vec4<f32>(vec3<f32>(0.0), ao4.a);
    }
    // The bounce is outgoing Lambert radiance (bounce x diffuse albedo), added
    // outside the occlusion multiply as the JS adds it on the emissive slot.
    let bounce = ao4.rgb * diffuse;
    if (look.ao_params.z > 0.5 && look.ao_params.z < 9.5) {
        // EO_AO_DEBUG=1: visibility; =2: the bounce irradiance fraction.
        if (look.ao_params.z > 1.5) {
            return vec4<f32>(ao4.rgb, 1.0);
        }
        return vec4<f32>(vec3<f32>(ao4.a), 1.0);
    }

    var lit = direct_d + direct_s + indirect_d + indirect_s + bounce + m.emissive.rgb + rm_emit; // + the room
    if (SF_NIGHT_PHOTOMETRY && look.fog.z > 0.0) {
        lit = lit + m.emissive.rgb * (look.fog.z - 1.0); // (`--night-photometry`: the lenses' gain, applied after the sum so the default frame stays bit-identical)
    }
    // the playable house (SF_HOUSE): the porch lamp's field
    if SF_HOUSE && porch_field.z > 0. {
        lit = porch_lit(lit,world,n,v,diffuse,f0,rough);
    }
// sf:WEATHER_FX_FLOOR+
    let local_lit = sky_local_lights_ao(world, n, v, diffuse, f0, rough, ao); // street lamps, headlamps (`ao` and their shadows only with the light shadows on)
    lit = lit + local_lit;
    if (sky_weather_fx_on(skyu.cirrus) && sky_rain_spark > 0.001) {
        // (`--weather-fx`) subtle raindrop specular glints on standing puddles,
        // kept understated so impacts read as delicate water droplets rather than painted dots.
        let sp_light = (irr / LK_PI) * 0.42 + look.hemi_sky.xyz * 0.08 + key * 0.35 + local_lit * 0.85;
        lit = lit + sp_light * min(sky_rain_spark, 0.45);
    }
    sky_rain_spark = 0.0;
// sf:end
// sf:WEATHER_FX_FLOOR-
    lit = lit + sky_local_lights_ao(world, n, v, diffuse, f0, rough, ao); // street lamps, headlamps (`ao` and their shadows only with the light shadows on)
// sf:end
    // the house's fitted finishes and waxed oak: their clear coats over the lit base
    if (SF_HOUSE && (objects[object].kind & FITTED_ROOM_BIT) != 0u && m.thin_absorption>0. && measured_room) {
        if (any(fitted_pendant_horizontal>vec3<f32>(0.))) {
            lit += pendant_gloss(fitted_pendant_horizontal,world,n,v,rough);
        }
        if (m.emissive.w > 0.) {
            let coat_rough=max(.035,m.rough_flatten);
            let coat_nv=max(dot(ng,v),.0001);
            let fresnel=.04+.96*pow(1.-coat_nv,5.);
            let lut=lk_dfg(coat_nv,coat_rough);
            var coat=fitted_coat_light*(.04*lut.x+lut.y);
            if (nl>0.) { coat+=key*(max(dot(ng,l),0.)/nl)*lk_ggx(ng,v,l,vec3<f32>(.04),coat_rough); }
            if (any(fitted_pendant_horizontal>vec3<f32>(0.))) {
                coat+=pendant_gloss(fitted_pendant_horizontal,world,ng,v,coat_rough);
            }
            lit=lit*(1.-m.emissive.w*fresnel)+m.emissive.w*coat;
        }
    }
    if (SF_HOUSE && oak_coated) {
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
        var rad_c = mix(lk_sh(rc) / LK_PI, lk_sky_spec(rc, lk_sky_sample(rc)), sharp_c);
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
    // the radiance probe (`lk_probe`): one term of the light in place of the whole
    if (look.ao_params.z > 9.5) {
        lit = lk_probe(i32(round(look.ao_params.z)), lit, direct_d, direct_s, indirect_d, indirect_s, bounce + m.emissive.rgb + rm_emit);
    }
    // RT_GI_HOOK
    return vec4<f32>(lit, alpha);
}

// The radiance probe (EO_AO_DEBUG = 10 + term; eo-look `probe`): a surface draws
// one term of its scene-linear radiance in place of the whole, before the
// aerial perspective and the camera, and the camera writes the radiance
// log-encoded (look_camera.wgsl `cam_probe_encode`) so a capture reads back
// in the look's units. Terms: 10 all (as drawn), 11 the sun's diffuse, 12 the
// sun's specular, 13 the sky's and the ground's diffuse (the environment's
// irradiance), 14 the sky's specular, 15 the bounce (the scene's and the
// sea's reflected light, emission), 16 none (what is left is the aerial
// in-scatter over the surface), 17 a unit white (the aerial transmittance
// plus the in-scatter). Not split: the street lamps and headlamps
// (`sky_local_lights`), the glass clearcoat and the set's capped-sight fog are
// in term 10 only, so the single terms do not add up to it at night, on glass,
// or under thick set fog.
fn lk_probe(term: i32, all: vec3<f32>, sun_d: vec3<f32>, sun_s: vec3<f32>, sky_d: vec3<f32>, sky_s: vec3<f32>, bounce: vec3<f32>) -> vec3<f32> {
    switch (term) {
        case 11: { return sun_d; }
        case 12: { return sun_s; }
        case 13: { return sky_d; }
        case 14: { return sky_s; }
        case 15: { return bounce; }
        case 16: { return vec3<f32>(0.0); }
        case 17: { return vec3<f32>(1.0); }
        default: { return all; }
    }
}
