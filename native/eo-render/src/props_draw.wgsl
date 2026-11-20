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
const PR_ROCK: u32 = 131072u;
// the meadow patches (props_lib2/vegetation_models.rs P_MEADOW*, P_BLADE_PBR).
const PR_MEADOW: u32 = 262144u;
const PR_MEADOW_LIVE: u32 = 524288u;
const PR_MEADOW_MAT: u32 = 1048576u;
const PR_BLADE_PBR: u32 = 2097152u;
const PR_MEADOW_THICK: u32 = 4194304u;
const PR_MEADOW_SOIL: u32 = 8388608u;

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
    var nw = pr_rot(inst.yaw, normal.xyz / max(inst.scale, vec3<f32>(1e-3)));
    // the slope shear's inverse transpose (pr_veg_grad).
    let g = pr_veg_grad(inst);
    nw = vec3<f32>(nw.x - g.x * nw.y, nw.y, nw.z - g.y * nw.y);
    out.normal = nw;
    out.uv = uv;
    out.color = srgb_decode(color.rgb);
    out.material = material;
    out.inst = idx;
    out.local = pos;
    let view = ii / max(pr.counts.z, 1u);
    out.fade = vec2<f32>(f32((word >> 17u) & 255u) / 255.0, select(0.0, 1.0, view == 5u));
    return out;
}

// value noise in [0, 1] (rock grain).
fn pr_hash3(p: vec3<f32>) -> f32 {
    var q = fract(p * 0.1031);
    q = q + dot(q, q.yzx + 33.33);
    return fract((q.x + q.y) * q.z);
}

fn pr_noise3(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(pr_hash3(i), pr_hash3(i + vec3<f32>(1.0, 0.0, 0.0)), u.x);
    let b = mix(pr_hash3(i + vec3<f32>(0.0, 1.0, 0.0)), pr_hash3(i + vec3<f32>(1.0, 1.0, 0.0)), u.x);
    let c = mix(pr_hash3(i + vec3<f32>(0.0, 0.0, 1.0)), pr_hash3(i + vec3<f32>(1.0, 0.0, 1.0)), u.x);
    let d = mix(pr_hash3(i + vec3<f32>(0.0, 1.0, 1.0)), pr_hash3(i + vec3<f32>(1.0, 1.0, 1.0)), u.x);
    return mix(mix(a, b, u.y), mix(c, d, u.y), u.z);
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
    if (inst.model >= PR_VEG_BASE + PR_VEG_GRASS && inst.model < PR_VEG_BASE + PR_VEG_ROCK && (m.flags & PR_TEX) != 0u) {
        // the ground cover's blades: the atlas cell's own shading near (blade by blade),
        // its mean from 6 m to 25 m. What survives the alpha test far off is the
        // cell's dark base clump (its rows run 0.3 of the tips' value at the root),
        // which drew a dark stroke on the ground tone; the mean keeps a far bunch
        // the ground's tone, its own value and cast (the instance tint) its only mark.
        let t = smoothstep(6.0, 25.0, distance(in.world, frame.cam_pos.xyz));
        s.albedo = in.color * m.base.rgb * mix(albedo_tex.rgb, vec3<f32>(PR_VEG_GRASS_CELL_MEAN), t);
    }
    if (pr_is_veg(inst.model) && inst.lamp != 0u && (m.flags & (PR_LEAF | PR_ROCK)) != 0u) {
        // the vegetation's own colour on its leaves, blades and rock (the bark keeps its own): sRGB x 4 in the lamp word.
        s.albedo = s.albedo * srgb_decode(unpack4x8unorm(inst.lamp).rgb) * 4.0;
    }
    let tip_word = inst.lamp >> 24u;
    if (inst.model >= PR_VEG_BASE + PR_VEG_GRASS && inst.model < PR_VEG_BASE + PR_VEG_ROCK && (m.flags & PR_TEX) != 0u && inst.lamp != 0u && tip_word != 255u) {
        // the meadow's blades (eo_landmarks::vegetation::meadow_tip; 255, every instance
        // before it and with --no-grass-color-variety, skips this): near the eye each blade
        // column of each card its own value (+-14 %) and cast, and the top of the cell (the
        // tallest blades' seed heads and awns) toward the bunch's tip colour: straw
        // lighter than the stem, or one of the flower tints (yellow, purple, white; linear,
        // INFERRED from the species' flower colour words) faded out by 30 m so a far
        // meadow is not speckled.
        let dist = distance(in.world, frame.cam_pos.xyz);
        let near = 1.0 - smoothstep(6.0, 25.0, dist);
        let h = pr_hash3(vec3<f32>(floor(in.uv.x * 48.0), floor(in.local.x * 5.0) + 37.0, floor(in.local.z * 5.0) + 11.0));
        s.albedo = s.albedo * mix(vec3<f32>(1.0), vec3<f32>(0.86 + 0.28 * h) * vec3<f32>(1.0 + 0.1 * (h - 0.5), 1.0, 1.0 - 0.16 * (h - 0.5)), near);
        let up = clamp((0.248 - in.uv.y) / 0.246, 0.0, 1.0);
        let kind = tip_word >> 6u;
        let amount = f32(tip_word & 63u) / 62.0;
        var tip_col = s.albedo * vec3<f32>(1.4, 1.3, 1.1);
        var fade = 1.0;
        if (kind == 1u) {
            tip_col = vec3<f32>(0.62, 0.42, 0.03);
            fade = 1.0 - smoothstep(12.0, 30.0, dist);
        } else if (kind == 2u) {
            tip_col = vec3<f32>(0.20, 0.10, 0.42);
            fade = 1.0 - smoothstep(12.0, 30.0, dist);
        } else if (kind == 3u) {
            tip_col = vec3<f32>(0.62, 0.62, 0.56);
            fade = 1.0 - smoothstep(12.0, 30.0, dist);
        }
        s.albedo = mix(s.albedo, tip_col, smoothstep(0.62, 0.92, up) * amount * 0.7 * fade);
    }
    // a meadow blade's diffuse transmittance (linear; zero for everything else).
    var blade_t = vec3<f32>(0.0);
    var n_light = n;
    if ((m.flags & PR_MEADOW) != 0u) {
        let dist = distance(in.world, frame.cam_pos.xyz);
        var live = in.color;
        if ((m.flags & PR_MEADOW_LIVE) != 0u && inst.lamp != 0u) {
            // the patch's cure (eo_landmarks::vegetation::meadow_patch_tint), sRGB x 4.
            live = live * srgb_decode(unpack4x8unorm(inst.lamp).rgb) * 4.0;
        }
        if ((m.flags & PR_MEADOW_MAT) != 0u) {
            if (PR_MEADOW_BREAKUP) {
                // a lobed rim (`--no-meadow-breakup`): a world-space field cuts the mats
                // back from their discs, the same on every mat overlapping a point, so
                // the cover opens in irregular gaps where the terrain's tone shows
                // instead of meeting its neighbours along disc edges.
                let rl = length(in.local.xz) / 0.95;
                let lobe = pr_noise3(vec3<f32>(in.world.xz * 1.6, 2.0)) * 0.65 + pr_noise3(vec3<f32>(in.world.xz * 5.0, 4.0)) * 0.35;
                if (rl > 0.4 + 0.8 * lobe) {
                    discard;
                }
            }
            // the mat under the blades: matted straw lying in two directions per
            // patch (its yaw), the sward's green through it in tufts (share: uv.y),
            // and dark pores between the straws. Each scale fades to its mean
            // where a pixel spans it, so the far mat is its mean colour, not noise.
            let fw = length(dpx) + length(dpy);
            let q = in.world.xz;
            let d1 = vec2<f32>(cos(inst.yaw), sin(inst.yaw));
            let d2 = vec2<f32>(cos(inst.yaw + 1.1), sin(inst.yaw + 1.1));
            let r1 = vec2<f32>(dot(q, d1), dot(q, vec2<f32>(-d1.y, d1.x)));
            let r2 = vec2<f32>(dot(q, d2), dot(q, vec2<f32>(-d2.y, d2.x)));
            let fine = 1.0 - smoothstep(0.003, 0.012, fw);
            let mid = 1.0 - smoothstep(0.012, 0.05, fw);
            let st = max(pr_noise3(vec3<f32>(r1.x * 5.0, r1.y * 70.0, 0.0)), pr_noise3(vec3<f32>(r2.x * 7.0, r2.y * 95.0, 3.0)));
            let straw = mix(0.62, st, fine);
            let coarse = 1.0 - smoothstep(0.03, 0.12, fw);
            let tuft = mix(0.5, pr_noise3(vec3<f32>(q * 24.0, 7.0)), mid) * 0.6 + mix(0.5, pr_noise3(vec3<f32>(q * 9.0, 11.0)), coarse) * 0.4;
            // far off every patch's mat goes to one share, so the types' mats do not
            // tile the hill in blobs (the patch tint, smooth over the hill, stays).
            let share = mix(in.uv.y, 0.5, smoothstep(15.0, 45.0, dist));
            // a soft mix: the green shoots come up through the straw, not in islands.
            let green = smoothstep(0.5 - share * 0.9 - 0.18, 0.5 - share * 0.9 + 0.62, tuft) * min(share * 1.6, 1.0);
            let blades = mix(1.0, 0.7 + 0.6 * pr_noise3(vec3<f32>(q.x * 80.0, q.y * 80.0, 1.0)), fine);
            let thatch_col = m.base.rgb * (0.55 + 0.7 * straw);
            // the clumps' own shade at a few per metre (what reads as texture from 20 to 100 m).
            let clump = mix(0.5, pr_noise3(vec3<f32>(q * 6.0, 5.0)), 1.0 - smoothstep(0.06, 0.25, fw));
            // near, where the blades stand over it, the mat is the canopy's floor: lit
            // through the blades, so darker (about 0.6; INFERRED from the photographs'
            // dark gaps between blades). Farther the mat alone draws the canopy (the
            // levels keep fewer blades), so it comes up to the canopy's mean.
            let floor = mix(0.6, 1.0, smoothstep(5.0, 26.0, dist));
            s.albedo = mix(thatch_col, live * blades, green) * (0.78 + 0.44 * clump) * floor;
            if ((m.flags & PR_MEADOW_SOIL) != 0u) {
                // bare eroded soil: its colour in clods and grit (two scales, each faded
                // to its mean where a pixel spans it), paler weathered crust in patches,
                // a little green at its own share; no straw, and not the canopy's floor.
                let clod = mix(0.5, pr_noise3(vec3<f32>(q * 3.0, 9.0)), coarse);
                let grit = mix(0.5, pr_noise3(vec3<f32>(q * 40.0, 13.0)), fine);
                let crust = smoothstep(0.55, 0.8, pr_noise3(vec3<f32>(q * 0.7, 15.0)));
                let soil = m.base.rgb * (0.78 + 0.34 * clod + 0.22 * (grit - 0.5)) * mix(vec3<f32>(1.0), vec3<f32>(1.18, 1.12, 1.05), crust);
                let sprout = smoothstep(0.72, 0.9, tuft) * in.uv.y * 3.0;
                s.albedo = mix(soil, live, clamp(sprout, 0.0, 1.0));
            }
            s.ao = mix(1.0, 0.55 + 0.45 * smoothstep(0.25, 0.7, straw), fine) * mix(0.85, 1.0, green);
            s.rough = 0.9;
            // the mat lies on the ground: lit from above even where the eye grazes
            // it from below its plane (a patch's sheared plane past a brow).
            s.n = normalize(in.normal);
            n_light = s.n;
        } else {
            if ((m.flags & PR_MEADOW_LIVE) != 0u) {
                s.albedo = live;
                if (inst.model >= PR_VEG_BASE + PR_VEG_MOUND && inst.model < PR_VEG_BASE + PR_VEG_MOUND_END) {
                    // a scrub mound's leaf clusters as grain where its mesh draws none (the
                    // clump cores between the leaves, the far level's clumps): two scales of
                    // lit clusters and dark gaps on the albedo and as a bump on the normal,
                    // each faded to its mean where a pixel spans it.
                    let fw = length(dpx) + length(dpy);
                    let p = in.world * 9.0;
                    let k1 = 1.0 - smoothstep(0.03, 0.12, fw);
                    let k2 = 1.0 - smoothstep(0.1, 0.4, fw);
                    let g1 = pr_noise3(p * 2.2);
                    let g2 = pr_noise3(p * 0.45 + vec3<f32>(5.0));
                    let gaps = mix(0.5, g1, k1) * 0.5 + mix(0.5, g2, k2) * 0.5;
                    s.albedo = s.albedo * (0.45 + 1.1 * smoothstep(0.2, 0.8, gaps));
                    s.ao = mix(0.6, 1.0, smoothstep(0.25, 0.6, gaps));
                    let e = 0.03;
                    let gx = pr_noise3((in.world + vec3<f32>(e, 0.0, 0.0)) * 4.0) - pr_noise3(in.world * 4.0);
                    let gy = pr_noise3((in.world + vec3<f32>(0.0, e, 0.0)) * 4.0) - pr_noise3(in.world * 4.0);
                    let gz = pr_noise3((in.world + vec3<f32>(0.0, 0.0, e)) * 4.0) - pr_noise3(in.world * 4.0);
                    let grad = vec3<f32>(gx, gy, gz) / e;
                    let bump = normalize(s.n - (grad - s.n * dot(grad, s.n)) * 0.12 * k2);
                    if (all(bump == bump)) {
                        s.n = bump;
                    }
                }
            } else {
                // dead straw: its own colour, each patch a little lighter or darker.
                s.albedo = in.color * (0.9 + 0.2 * f32(inst.variant & 255u) / 255.0);
            }
            if ((m.flags & PR_BLADE_PBR) != 0u) {
                let along = clamp(in.uv.y, 0.0, 1.0);
                let across = in.uv.x * 2.0 - 1.0;
                let thick = (m.flags & PR_MEADOW_THICK) != 0u;
                // clump occlusion: inside a clump the base sees little sun and a
                // narrow cone of sky, so it is darker and cooler (sky-lit); the tips
                // stand in the open. (A blade's position along itself stands in for
                // its depth in the canopy.)
                let open = clamp(0.3 + 0.7 * along, 0.0, 1.0);
                s.albedo = s.albedo * mix(vec3<f32>(0.68, 0.75, 0.85), vec3<f32>(1.0), open);
                s.ao = mix(0.45, 1.0, open);
                // the blade's keel (a V fold at the midrib) and rolled margins as a
                // normal map: the across direction from the screen derivatives of u.
                let near = 1.0 - smoothstep(6.0, 18.0, dist);
                let r1 = cross(dpy, n);
                let r2 = cross(n, dpx);
                let tq = r1 * duvx.x + r2 * duvy.x;
                if (!thick && near > 0.0 && dot(tq, tq) > 1e-24) {
                    let t = normalize(tq);
                    let fold = sign(across) * 0.45 + across * abs(across) * 0.55;
                    let nb = normalize(s.n - t * fold * near);
                    if (all(nb == nb)) {
                        s.n = nb;
                    }
                    // a paler midrib and faint parallel veins.
                    s.albedo = s.albedo * (1.0 + near * (0.2 * exp(-across * across * 60.0) + 0.05 * cos(across * 28.0)));
                }
                // dry, paler tips in the sun.
                s.albedo = s.albedo * (1.0 + 0.12 * smoothstep(0.75, 1.0, along));
                // Leaf optics. A green leaf's visible transmittance is close to its
                // reflectance in the green (about 0.1 at 550 nm) and far below it in
                // the blue and red, where chlorophyll absorbs (PROSPECT: Jacquemoud
                // and Baret 1990, Remote Sens. Environ. 34, 75-91; PROSPECT-D: Feret
                // et al. 2017, Remote Sens. Environ. 193, 204-215; measured leaves:
                // Woolley 1971, Plant Physiol. 47, 656-662), so light through a thin
                // blade is a saturated yellow-green. Dry straw transmits less and
                // warmer (brown pigments); a succulent leaf, thick and water-filled,
                // little. The cuticle's wax gives a weak, broad specular lobe (leaf
                // BRDF: Bousquet et al. 2005, Remote Sens. Environ. 98, 201-211):
                // dielectric F0 0.04, rough 0.5 to 0.6, not a plastic sheen.
                if (thick) {
                    blade_t = s.albedo * vec3<f32>(0.5, 0.6, 0.2) * 0.3;
                    s.rough = 0.38;
                } else if ((m.flags & PR_MEADOW_LIVE) != 0u) {
                    blade_t = s.albedo * vec3<f32>(0.95, 0.9, 0.3) * 0.9;
                    s.rough = 0.55;
                } else {
                    blade_t = s.albedo * vec3<f32>(0.75, 0.6, 0.35) * 0.6;
                    s.rough = 0.72;
                }
            }
        }
    }
    if ((m.flags & PR_ROCK) != 0u) {
        // rock grain: two octaves of value noise in world metres (mottle and
        // pitting), on the albedo and as a bump on the normal, faded out where
        // a pixel spans more than its cell.
        let fw = length(dpx) + length(dpy);
        let k = 1.0 - smoothstep(0.05, 0.4, fw);
        let g = pr_noise3(in.world * 2.3) * 0.65 + pr_noise3(in.world * 9.0) * 0.35;
        let coarse = pr_noise3(in.world * 0.6) * 0.6 + pr_noise3(in.world * 0.17) * 0.4;
        // lichen and soil stain on the upward faces (grey-olive), darker in the coarse lows.
        let up = clamp(s.n.y, 0.0, 1.0);
        let stain = smoothstep(0.55, 0.8, pr_noise3(in.world * 1.1 + vec3<f32>(7.0))) * up * 0.6;
        // (a narrow spread, 0.82-1.18 fine and 0.85-1.15 coarse: a wider one reads as
        // pockmarks on a smooth slab at 1.7 m.)
        s.albedo = s.albedo * mix(1.0, 0.82 + 0.36 * g, k) * (0.85 + 0.3 * coarse);
        s.albedo = mix(s.albedo, vec3<f32>(dot(s.albedo, vec3<f32>(0.33))) * vec3<f32>(1.05, 1.08, 0.9), stain);
        let e = 0.04;
        let gx = pr_noise3((in.world + vec3<f32>(e, 0.0, 0.0)) * 9.0) - pr_noise3(in.world * 9.0);
        let gy = pr_noise3((in.world + vec3<f32>(0.0, e, 0.0)) * 9.0) - pr_noise3(in.world * 9.0);
        let gz = pr_noise3((in.world + vec3<f32>(0.0, 0.0, e)) * 9.0) - pr_noise3(in.world * 9.0);
        let grad = vec3<f32>(gx, gy, gz) / e;
        let bump = normalize(s.n - (grad - s.n * dot(grad, s.n)) * 0.012 * k);
        if (all(bump == bump)) {
            s.n = bump;
        }
    }
    s.rough = clamp(s.rough, 0.03, 1.0);
    var c = look_light(s, m, v, in.world, n_light, 0u);
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
    if (blade_t.g > 0.0) {
        // light through the blade: diffuse transmission when the sun is behind
        // it (as seen from here) and a forward lobe toward the sun, shadowed and
        // fogged as the key light. The geometric normal faces the eye.
        let l = look.key_dir.xyz;
        let through = max(dot(-n, l), 0.0);
        let lobe = pow(max(dot(-v, l), 0.0), 4.0);
        let sh = lk_shadow(in.world, n);
        let vd = dot(in.world - look.cam_pos.xyz, look.cam_fwd.xyz);
        let fog = lk_smooth(look.fog.x, look.fog.y, vd);
        let trans = look.key.xyz * blade_t * (through + lobe * 0.8) * sh / LK_PI;
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
        let vm = pr_instances[in.inst].model;
        if (pr_is_veg(vm)) {
            // the vegetation's cards (leaf sprigs, grass blades): alpha from a
            // sharper level (a quarter of the footprint, two mips finer) so thin
            // blades keep their coverage instead of thinning to their base; the
            // temporal resolve smooths the finer edge. The ground cover (grass,
            // tufts, flowers) goes back to the footprint's own level from 10 m
            // to 40 m, so its far blades soften and fade into the ground tone
            // instead of standing as sharp strokes.
            var k = 0.25;
            if (vm >= PR_VEG_BASE + PR_VEG_GRASS && vm < PR_VEG_BASE + PR_VEG_ROCK) {
                k = mix(0.25, 1.0, smoothstep(10.0, 40.0, distance(in.world, pr.cam.xyz)));
            }
            a = textureSampleGrad(pr_big, kit_sampler, in.uv, m.layer, duvx * k, duvy * k).a;
        } else {
            a = textureSampleGrad(pr_big, kit_sampler, in.uv, m.layer, duvx, duvy).a;
        }
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
