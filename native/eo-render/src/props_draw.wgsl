// the street props in the city pass (appended to draw.wgsl +
// look_common.wgsl + look_light.wgsl + props_types.wgsl). Group 0 has
// draw.wgsl's layout with the props library's own buffers and texture
// arrays; group 1 is the look's; group 2 the props data. Lighting, the
// cascaded sun shadow and fog are the city's own `look_light`; the AO pass
// reads the depth these write.
//
// the big texture array (vegetation atlas, sign faces, printed
// faces), sign legends on the plates (the face atlas's alpha is the plate's
// outline, alpha-tested), the detectable-warning domes as a normal detail on
// the pad (the JS dome grid's pitch, base and height), back-lit leaves (a
// transmission term under the sun), wind sway (props_types.wgsl `pr_wind`),
// and the dithered cross-fade pipelines (`*_fade`) for views 4 and 5.

const PR_TEX: u32 = 256u;
const PR_CUT: u32 = 512u;
const PR_LENS: u32 = 1024u;
const PR_PED_HAND: u32 = 2048u;
const PR_PED_WALK: u32 = 4096u;
const PR_PLATE: u32 = 8192u;
const PR_GLB_MAPS: u32 = 16384u;
const PR_BIG: u32 = 32768u;
const PR_LEAF: u32 = 65536u;

@group(2) @binding(0) var<uniform> pr: PrParams;
@group(2) @binding(1) var<storage, read> pr_instances: array<PrInstance>;
@group(2) @binding(2) var<storage, read> pr_visible: array<u32>;
// Per lamp channel: a vehicle head's lit lens (0 red, 1 amber, 2 green, 3 dark);
// a pedestrian head's bits (1 hand lit, 2 walking person lit).
@group(2) @binding(3) var<storage, read> pr_lamps: array<u32>;
// Plate face colours (sRGB) by face index, then the plate back, then (props2) each face's atlas tile.
@group(2) @binding(4) var<storage, read> pr_faces: array<vec4<f32>>;
// Lens tone inputs: lit red, amber, green, then unlit; w of [0] the ACES scale.
@group(2) @binding(5) var<uniform> pr_lens: array<vec4<f32>, 6>;
// the big array.
@group(2) @binding(9) var pr_big: texture_2d_array<f32>;

struct PrOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec3<f32>,
    @location(4) @interpolate(flat) material: u32,
    @location(5) @interpolate(flat) inst: u32,
    // the model-space position (plates, pads) and the cross-fade (t, arriving 1 / leaving 0).
    @location(6) local: vec3<f32>,
    @location(7) @interpolate(flat) fade: vec2<f32>,
};

@vertex
fn prop_vs(
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) material: u32,
    @location(5) sway: f32,
    @builtin(instance_index) ii: u32,
) -> PrOut {
    let word = pr_visible[ii];
    let idx = word & PR_INDEX_MASK;
    let inst = pr_instances[idx];
    let world = pr_world(inst, pos, sway);
    var out: PrOut;
    out.clip = frame.view_proj * vec4<f32>(world, 1.0);
    out.world = world;
    out.normal = pr_rot(inst.yaw, normal.xyz / max(inst.scale, vec3<f32>(1e-3)));
    out.uv = uv;
    out.color = srgb_decode(color.rgb);
    out.material = material;
    out.inst = idx;
    out.local = pos;
    let view = ii / max(pr.counts.z, 1u);
    out.fade = vec2<f32>(f32((word >> 17u) & 255u) / 255.0, select(0.0, 1.0, view == 5u));
    return out;
}

// interleaved gradient noise, the dither of the cross-fade.
fn pr_dither(p: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

fn pr_fade_test(in: PrOut) {
    let n = pr_dither(floor(in.clip.xy));
    let t = in.fade.x;
    if (in.fade.y > 0.5) {
        if (n >= t) {
            discard;
        }
    } else if (n < t) {
        discard;
    }
}

// a sign plate's face tile (u0, v0 up, du, dv), zero without an atlas.
fn pr_face_tile(variant: u32) -> vec4<f32> {
    let f = u32(pr.lens.w);
    let n = arrayLength(&pr_faces);
    if ((pr.mode.z & PR_F_SIGNS) == 0u || f == 0u || n < 2u * f + 1u) {
        return vec4<f32>(0.0);
    }
    return pr_faces[f + 1u + min(variant, f - 1u)];
}

fn pr_face_uv(tile: vec4<f32>, local: vec3<f32>) -> vec2<f32> {
    let s = clamp(local.xy + vec2<f32>(0.5), vec2<f32>(0.0), vec2<f32>(1.0));
    return vec2<f32>(tile.x + s.x * tile.z, 1.0 - (tile.y + s.y * tile.w));
}

fn pr_shade(in: PrOut, front: bool) -> vec4<f32> {
    var m = materials[in.material];
    let inst = pr_instances[in.inst];
    let dpx = dpdx(in.world);
    let dpy = dpdy(in.world);
    let duvx = dpdx(in.uv);
    let duvy = dpdy(in.uv);
    let dlx = dpdx(in.local);
    let dly = dpdy(in.local);
    var albedo_tex = vec4<f32>(1.0);
    if ((m.flags & PR_TEX) != 0u) {
        if ((m.flags & PR_BIG) != 0u) {
            albedo_tex = textureSampleGrad(pr_big, kit_sampler, in.uv, m.layer, duvx, duvy);
        } else {
            albedo_tex = textureSampleGrad(kit_albedo, kit_sampler, in.uv, m.layer, duvx, duvy);
        }
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
    s.albedo = in.color * m.base.rgb * albedo_tex.rgb;
    s.rough = m.params.x;
    s.metal = m.params.y;
    s.ao = 1.0;
    s.n = n;
    s.alpha = 1.0;
    if ((m.flags & PR_GLB_MAPS) != 0u) {
        // glTF metallic-roughness: G roughness, B metalness.
        let orm = textureSampleGrad(kit_orm, kit_sampler, in.uv, m.layer, duvx, duvy);
        s.rough = orm.g;
        s.metal = orm.b;
        let nt = textureSampleGrad(kit_normal, kit_sampler, in.uv, m.layer, duvx, duvy).xyz * 2.0 - 1.0;
        let r1 = cross(dpy, n);
        let r2 = cross(n, dpx);
        let t = r1 * duvx.x + r2 * duvy.x;
        let b = r1 * duvx.y + r2 * duvy.y;
        let inv = inverseSqrt(max(max(dot(t, t), dot(b, b)), 1e-20));
        let nm = normalize(t * inv * nt.x - b * inv * nt.y + n * max(nt.z, 0.05));
        if (all(nm == nm)) {
            s.n = nm;
        }
    }
    if ((m.flags & PR_PLATE) != 0u) {
        // The plate's front face takes its face (the legend from the face
        // atlas where there is one); the back is the bare sheet.
        let f = u32(pr.lens.w);
        let k = min(inst.variant, max(f, 1u) - 1u);
        let facing = dot(pr_rot(inst.yaw, vec3<f32>(0.0, 0.0, 1.0)), v) > 0.0;
        if (facing) {
            s.albedo = srgb_decode(pr_faces[k].rgb);
            let tile = pr_face_tile(inst.variant);
            if (tile.z > 0.0 && in.local.z > -0.001) {
                let uv = pr_face_uv(tile, in.local);
                let du = dlx.xy * tile.zw;
                let dv = dly.xy * tile.zw;
                s.albedo = textureSampleGrad(pr_big, kit_sampler, uv, 1, vec2<f32>(du.x, -du.y), vec2<f32>(dv.x, -dv.y)).rgb;
                s.rough = 0.45;
            }
        } else {
            s.albedo = srgb_decode(pr_faces[f].rgb);
        }
    }
    if (inst.model == PR_TACTILE_PAD && (pr.mode.z & PR_F_DOMES) != 0u && pr.dome.z > 0.0) {
        // the truncated dome grid (DETECTABLE_WARNING_PAD: pitch, base, height),
        // as a normal detail in the pad's own metres, faded out where a pixel spans a pitch.
        let lm = vec2<f32>(in.local.x * inst.scale.x, in.local.z * inst.scale.z);
        let g = lm / pr.dome.z;
        let sc = vec2<f32>(inst.scale.x, inst.scale.z) / pr.dome.z;
        let fw = max(length(abs(dlx.xz * sc) + abs(dly.xz * sc)), 1e-4);
        let detail = 1.0 - smoothstep(0.25, 0.6, fw);
        if (detail > 0.0) {
            let off = (fract(g) - vec2<f32>(0.5)) * pr.dome.z;
            let r = length(off);
            let base = pr.dome.x * 0.5;
            if (r < base) {
                let slope = pr.dome.y / max(base, 1e-4);
                let radial = off / max(r, 1e-5);
                let side = pr_rot(inst.yaw, vec3<f32>(radial.x, 0.0, radial.y));
                let k = smoothstep(base * 0.25, base * 0.45, r) * detail;
                s.n = normalize(s.n + side * slope * k * 2.0);
                // `domeCap` (#dcb44a) over `domePadYellow` (#cda336).
                s.albedo = mix(s.albedo, s.albedo * vec3<f32>(1.12, 1.1, 1.08), detail);
            } else {
                s.ao = mix(1.0, 0.8, detail * (1.0 - smoothstep(base, base * 1.35, r)));
            }
        }
    }
    m.emissive = vec4<f32>(0.0);
    if ((m.flags & PR_LENS) != 0u) {
        // Lens radiance solved in the tone mapper's units (SIGNAL_LENS_TONE_INPUT):
        // v x ACES scale / exposure, lit or dark glass.
        let k = (m.flags >> 24u) & 3u;
        let lit = pr_lamps[inst.lamp] == k;
        let tone = select(pr_lens[3u + k].rgb, pr_lens[k].rgb, lit);
        m.emissive = vec4<f32>(tone * pr.lens.x, 0.0);
        s.albedo = vec3<f32>(0.0);
    }
    if ((m.flags & (PR_PED_HAND | PR_PED_WALK)) != 0u) {
        let state = pr_lamps[inst.lamp];
        let hand = (m.flags & PR_PED_HAND) != 0u;
        let lit = select((state & 2u) != 0u, (state & 1u) != 0u, hand);
        // Portland orange hand, lunar white person: the lit amber / a white at the same scale.
        let hue = select(vec3<f32>(0.85, 0.85, 0.8), pr_lens[1].rgb * 0.6, hand);
        m.emissive = vec4<f32>(select(hue * 0.04, hue, lit) * pr.lens.x, 0.0);
        s.albedo = vec3<f32>(0.02);
    }
    s.rough = clamp(s.rough, 0.03, 1.0);
    var c = look_light(s, m, v, in.world, n, 0u);
    if ((m.flags & PR_LEAF) != 0u && (pr.mode.z & PR_F_LEAF) != 0u) {
        // back-lit leaves. A thin leaf passes part of the sun it does not
        // reflect: a diffuse transmission through the card (the sun behind the
        // leaf as seen from here) plus a forward-scattered lobe toward the sun,
        // both shadowed like the key light and fogged like the rest.
        let l = look.key_dir.xyz;
        let through = max(dot(-n, l), 0.0);
        let lobe = pow(max(dot(-v, l), 0.0), 6.0);
        let sh = lk_shadow(in.world, n);
        let vd = dot(in.world - look.cam_pos.xyz, look.cam_fwd.xyz);
        let fog = lk_smooth(look.fog.x, look.fog.y, vd);
        let trans = look.key.xyz * s.albedo * (through * 0.35 + lobe * 0.6) * sh / LK_PI;
        c = vec4<f32>(c.rgb + trans * (1.0 - fog), c.a);
    }
    return c;
}

@fragment
fn prop_fs(in: PrOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    return pr_shade(in, front);
}

// Alpha-tested parts: the JS foliage material's alpha test (leaf cards), and
// (props2) the sign face's outline on a plate.
fn pr_cut(in: PrOut) {
    let m = materials[in.material];
    let duvx = dpdx(in.uv);
    let duvy = dpdy(in.uv);
    var a = 1.0;
    if ((m.flags & PR_PLATE) != 0u) {
        let tile = pr_face_tile(pr_instances[in.inst].variant);
        if (tile.z > 0.0) {
            a = textureSampleLevel(pr_big, kit_sampler, pr_face_uv(tile, in.local), 1, 0.0).a;
        }
    } else if ((m.flags & PR_BIG) != 0u) {
        a = textureSampleGrad(pr_big, kit_sampler, in.uv, m.layer, duvx, duvy).a;
    } else {
        a = textureSampleGrad(kit_albedo, kit_sampler, in.uv, m.layer, duvx, duvy).a;
    }
    if (a < pr.lens.y) {
        discard;
    }
}

@fragment
fn prop_fs_cut(in: PrOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    pr_cut(in);
    return pr_shade(in, front);
}

@fragment
fn prop_fs_fade(in: PrOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    pr_fade_test(in);
    return pr_shade(in, front);
}

@fragment
fn prop_fs_cut_fade(in: PrOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    pr_fade_test(in);
    pr_cut(in);
    return pr_shade(in, front);
}

struct PrIdsOut {
    @location(0) color: vec4<f32>,
    @location(1) ids: vec2<u32>,
};

@fragment
fn prop_fs_cut_ids(in: PrOut, @builtin(front_facing) front: bool) -> PrIdsOut {
    pr_cut(in);
    var out: PrIdsOut;
    out.color = pr_shade(in, front);
    out.ids = vec2<u32>(0xfffffffdu, in.material);
    return out;
}

@fragment
fn prop_fs_ids(in: PrOut, @builtin(front_facing) front: bool) -> PrIdsOut {
    var out: PrIdsOut;
    out.color = pr_shade(in, front);
    out.ids = vec2<u32>(0xfffffffdu, in.material);
    return out;
}

@fragment
fn prop_fs_fade_ids(in: PrOut, @builtin(front_facing) front: bool) -> PrIdsOut {
    pr_fade_test(in);
    var out: PrIdsOut;
    out.color = pr_shade(in, front);
    out.ids = vec2<u32>(0xfffffffdu, in.material);
    return out;
}

@fragment
fn prop_fs_cut_fade_ids(in: PrOut, @builtin(front_facing) front: bool) -> PrIdsOut {
    pr_fade_test(in);
    pr_cut(in);
    var out: PrIdsOut;
    out.color = pr_shade(in, front);
    out.ids = vec2<u32>(0xfffffffdu, in.material);
    return out;
}
