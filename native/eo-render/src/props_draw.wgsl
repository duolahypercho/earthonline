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
// Props material flags (vertex-visible mirror of group 0's materials buffer).
@group(2) @binding(8) var<storage, read> pr_materials: array<Material>;
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
// sf:PROP_FAR+
    // (`--prop-far-shadow`) the far map's sun visibility at the vertex (`pr_far_sun`).
    @location(8) far_sun: f32,
// sf:end
// sf:CROWN|LEAF_SHADE+
    // (`--crown-light`) the wind weight: a tree vertex's height in its crown (props_lib2.rs `tree_mesh`).
    @location(9) sway: f32,
// sf:end
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
    let is_leaf = (pr_materials[material].flags & PR_LEAF) != 0u;
    let world = pr_world_leaf(inst, pos, uv, sway, is_leaf);
    var out: PrOut;
    out.clip = frame.view_proj * vec4<f32>(world, 1.0);
    out.world = world;
    var nw = pr_rot(inst.yaw, pr_local_dir(inst, normal.xyz / max(inst.scale, vec3<f32>(1e-3))));
    // the slope shear's inverse transpose (pr_veg_grad).
    let g = pr_veg_grad(inst);
    nw = vec3<f32>(nw.x - g.x * nw.y, nw.y, nw.z - g.y * nw.y);
    out.normal = nw;
    out.uv = uv;
    out.color = srgb_decode(color.rgb);
    if (SF_LUMINAIRES && color.a < 0.9) {
        // (`--luminaires`) a district light's lit glass: its radiance (`pr_lamp_glass`) in the colour
        out.color = pr_lamp_glass(color, inst, pos, normalize(look.cam_pos.xyz - world));
    }
    if (SF_SIGNAL_SPILL && inst.model == PR_SIGNAL_HEAD && (materials[material].flags & PR_LENS) != 0u) {
        // (`--signal-spill`) a signal lens's radiance (`pr_signal_lens`) in the colour (a lens's albedo is 0)
        out.color = pr_signal_lens(materials[material].flags, inst, normalize(look.cam_pos.xyz - world));
    }
    out.material = material;
    out.inst = idx;
    out.local = pos;
    let view = ii / max(pr.counts.z, 1u);
    out.fade = vec2<f32>(f32((word >> 17u) & 255u) / 255.0, select(0.0, 1.0, view == 5u));
// sf:PROP_FAR+
    out.far_sun = pr_far_sun(world);
// sf:end
// sf:CROWN|LEAF_SHADE+
    out.sway = sway;
// sf:end
    return out;
}
// sf:PROP_FAR+

// (`--prop-far-shadow`) The sun's visibility from the far map (far_shadow.wgsl)
// at a vertex past the cascades' fade start (`cascade_split.w`): the props'
// fragment shaders compile the cascades alone (look.rs `ShadowLevels::Near`,
// the cut-out pipeline at the edge of what Apple's compiler takes), so past the
// cascades a street tree, palm or pole in a building's shadow stood in full sun.
// Looked up here, once a vertex, and read by `lk_shadow` through
// `lk_far_vertex`. The map holds buildings, terrain and structures, not props,
// so a prop never shadows itself in it; the normal offset is the ground's (up).
fn pr_far_sun(world: vec3<f32>) -> f32 {
    let vd = dot(world - look.cam_pos.xyz, look.cam_fwd.xyz);
    if (vd < look.cascade_split.w || look.key_dir.w <= 0.0 || !lk_far_on()) {
        return 1.0;
    }
    return lk_far_shadow(world, vec3<f32>(0.0, 1.0, 0.0));
}
// sf:end

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
// sf:LOD_FADE-
    let n = pr_dither(floor(in.clip.xy));
    let t = in.fade.x;
// sf:end
// sf:LOD_FADE+
    // (`--temporal-lod-fade`) the pattern moves every frame (Jimenez's
    // interleaved gradient noise stepped 5.588238 px a frame over 64 frames),
    // so the temporal anti-aliasing resolves the pair to a blend of the two
    // levels instead of a fixed screen door; and the blend takes the middle of
    // the band only, so an instance shows one level whole over most of it and
    // two levels whose silhouettes differ (the palm's near and far models) are
    // seen together over a short reach.
    let n = pr_dither(floor(in.clip.xy) + vec2<f32>(5.588238 * look.post2.w));
    let t = smoothstep(0.3, 0.7, in.fade.x);
// sf:end
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

// ---- tree basins (eo-props `basins.rs`, `--tree-basins`) ----
// The bed is drawn here in the basin's own metres (x across the walk, y along
// the kerb): the scanned CC0 `park_dirt` soil where the scanned sets are on
// (a fresh patch of it in every basin, its detail round the basin's tone),
// else drawn grain, clods, grit and a few stones; over it damp patches and
// the watered ground round the trunk, the root flare, leaf litter piled
// against the cut edge, a cigarette end now and then, moss where it stays
// damp, and the shade line where the soil meets the walk's cut face. Past the cut edge (the L0
// mesh's fringe, drawn by the cutout pipeline) only the crumbs and leaves
// kicked onto the walk are kept. Every term fades by the pixel's footprint.
// Tones are authored (INFERRED) near neutral: the look's grading lifts the
// warmth of dark tones (a #3a342e bed rendered 5.7:1 red to blue in sun).
const PR_BASIN_TRUNK_STEP_M: f32 = 0.05; // eo-props basins::TRUNK_STEP_M
const PR_BASIN_DRY: vec3<f32> = vec3<f32>(0.092, 0.082, 0.072);
const PR_BASIN_DAMP: vec3<f32> = vec3<f32>(0.045, 0.040, 0.036);
const PR_BASIN_ROOT: vec3<f32> = vec3<f32>(0.062, 0.054, 0.046);
const PR_BASIN_MOSS: vec3<f32> = vec3<f32>(0.040, 0.050, 0.026);

// the trunk's offset across the basin (local X, m): the variant's bits 11-14 and sign bit 15.
fn pr_basin_trunk(inst: PrInstance) -> f32 {
    let k = f32((inst.variant >> 11u) & 15u) * PR_BASIN_TRUNK_STEP_M;
    return select(k, -k, (inst.variant & 0x8000u) != 0u);
}

// A layer of lumps (clods or stones): one in a cell (`p`, cells) with
// probability `density`, an irregular dome inside its own cell, so no network of cell
// edges. xy the surface gradient (per m), z the cavity at its foot, w its share; `k` its
// footprint fade.
fn pr_lumps(p: vec2<f32>, salt: f32, per_m: f32, height_m: f32, density: f32, k: f32) -> vec4<f32> {
    let id = floor(p);
    if (k <= 0.0 || st_hash(id + vec2<f32>(salt, 43.0)) >= density) {
        return vec4<f32>(0.0);
    }
    let centre = vec2<f32>(0.3) + 0.4 * vec2<f32>(st_hash(id + vec2<f32>(salt, 47.0)), st_hash(id + vec2<f32>(53.0, salt)));
    let d = p - id - centre;
    let dist = max(length(d), 1e-4);
    let a = atan2(d.y, d.x);
    let w = st_hash(id + vec2<f32>(salt, 59.0)) * 6.2832;
    let r = mix(0.12, 0.26, st_hash(id + vec2<f32>(61.0, salt))) * (1.0 + 0.2 * sin(a * 3.0 + w) + 0.1 * sin(a * 5.0 + w * 1.7));
    let t = dist / r;
    // h = height (1 - t^2) inside: dh/d(dist) = -2 height t / r.
    let inside = 1.0 - smoothstep(0.92, 1.0, t);
    let grad = (d / dist) * (-2.0 * height_m * min(t, 1.0) / r) * per_m;
    let cavity = smoothstep(1.45, 1.0, t) * step(1.0, t);
    return vec4<f32>(grad * inside * k, cavity * k, inside * k);
}

struct PrLeaf {
    cover: f32,
    tone: vec3<f32>,
    // the leaf's tilt (its surface gradient), how far toward its curled edge the pixel is, its midrib.
    tilt: vec2<f32>,
    curl: f32,
    rib: f32,
};

// One layer of fallen leaves over the basin and the walk round it (`q` m, `off` the basin's
// pattern offset): cells of `cell_m`, each a pointed oval at its own angle, the topmost
// covering one. A leaf lies where its centre's density lets it: `base` on the bed, up to
// `pile` within 0.16 m of the cut edge, `spill` just past it, none further out.
fn pr_leaves(q: vec2<f32>, off: vec2<f32>, half: vec2<f32>, cell_m: f32, salt: f32, base: f32, pile: f32, spill: f32, fw: f32) -> PrLeaf {
    var o: PrLeaf;
    o.cover = 0.0;
    o.tone = vec3<f32>(0.0);
    o.tilt = vec2<f32>(0.0);
    o.curl = 0.0;
    o.rib = 0.0;
    let p = (q + off) / cell_m;
    let i = floor(p);
    let fwc = max(fw / cell_m, 1e-4);
    var best = 2.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let id = i + vec2<f32>(f32(x), f32(y));
            let h = vec4<f32>(st_hash(id + vec2<f32>(salt, 3.3)), st_hash(id + vec2<f32>(7.7, salt)), st_hash(id + vec2<f32>(salt, 5.1)), st_hash(id + vec2<f32>(2.9, salt)));
            let centre = id + vec2<f32>(0.2) + 0.6 * vec2<f32>(st_hash(id + vec2<f32>(salt, 11.0)), st_hash(id + vec2<f32>(13.0, salt)));
            let cq = centre * cell_m - off;
            let ed = min(half.x - abs(cq.x), half.y - abs(cq.y));
            var dens = mix(base, pile, 1.0 - smoothstep(0.0, 0.16, ed));
            if (ed < 0.0) {
                dens = spill * (1.0 - smoothstep(0.0, 0.08, -ed));
            }
            if (h.x >= dens) {
                continue;
            }
            let a = h.y * 6.2832;
            let r = p - centre;
            let u = r.x * cos(a) + r.y * sin(a);
            let v = -r.x * sin(a) + r.y * cos(a);
            let len = mix(0.3, 0.5, h.z);
            let t = u / len;
            if (abs(t) >= 1.0) {
                continue;
            }
            // narrow to broad, pointed or blunt, and some broken (a bite of noise out of them).
            let blunt = st_hash(id + vec2<f32>(salt, 37.0));
            let half_w = len * mix(0.22, 0.6, h.w) * pow(max(1.0 - t * t, 0.0), mix(1.0, 0.55, blunt)) * (1.0 + 0.3 * t);
            var cover = clamp((half_w - abs(v)) / fwc + 0.5, 0.0, 1.0);
            if (st_hash(id + vec2<f32>(41.0, salt)) < 0.45) {
                cover = cover * smoothstep(0.38, 0.46, st_noise(r * 9.0 + id * 3.7));
            }
            // the lowest hash lies on top.
            if (cover <= 0.0 || h.x >= best) {
                continue;
            }
            best = h.x;
            o.cover = cover;
            let k = st_hash(id + vec2<f32>(salt, 17.0));
            // mostly old and dull: dark and decayed, brown, a dry tan, a yellowed one now and then,
            // each dusted with the soil.
            var tone = vec3<f32>(0.042, 0.037, 0.032);
            if (k > 0.4) {
                tone = vec3<f32>(0.08, 0.064, 0.05);
            }
            if (k > 0.76) {
                tone = vec3<f32>(0.125, 0.105, 0.082);
            }
            if (k > 0.95) {
                tone = vec3<f32>(0.14, 0.122, 0.07);
            }
            o.tone = mix(tone * mix(0.8, 1.15, st_hash(id + vec2<f32>(23.0, salt))), PR_BASIN_DRY, mix(0.15, 0.5, st_hash(id + vec2<f32>(salt, 19.0))));
            o.tilt = (vec2<f32>(st_hash(id + vec2<f32>(salt, 29.0)), st_hash(id + vec2<f32>(31.0, salt))) - 0.5) * 0.7;
            o.curl = smoothstep(0.55, 1.0, abs(v) / max(half_w, 1e-4));
            o.rib = (1.0 - smoothstep(0.0, max(0.035 * len, fwc), abs(v))) * (1.0 - smoothstep(0.7, 1.0, abs(t)));
        }
    }
    return o;
}

// The root flare at `d` from the trunk (m): five roots wandering out 0.12-0.5 m, half
// buried and tapering (3.5 to 1 cm half width), under a low mound. x the height (m),
// y the roots' cover (antialiased by `fw`).
fn pr_root_h(d: vec2<f32>, seed: f32, fw: f32) -> vec2<f32> {
    let rr = length(d);
    let phase = atan2(d.y, d.x) * 5.0 + (st_noise(vec2<f32>(rr * 6.0, seed)) - 0.5) * 2.0 + seed * 6.2832;
    let n = round(phase / 6.2832);
    let root = n - 5.0 * floor(n / 5.0);
    let reach = mix(0.12, 0.5, st_hash(vec2<f32>(root, seed)));
    let hw = mix(0.035, 0.01, clamp(rr / reach, 0.0, 1.0));
    let lat = abs(phase - n * 6.2832) / 5.0 * rr;
    let x = clamp(lat / hw, 0.0, 1.0);
    let out = 1.0 - smoothstep(reach * 0.75, reach, rr);
    let h = sqrt(1.0 - x * x) * hw * 0.5 * out + 0.012 * (1.0 - smoothstep(0.05, 0.4, rr));
    return vec2<f32>(h, clamp((hw * 0.9 - lat) / max(fw, 1e-4) + 0.5, 0.0, 1.0) * out);
}

// Past the cut edge: the crumbs and leaves on the walk. cover, then the tone.
fn pr_basin_spill(q: vec2<f32>, off: vec2<f32>, half: vec2<f32>, ed: f32, fw: f32) -> PrLeaf {
    var o = pr_leaves(q, off, half, 0.1, 1.0, 0.0, 0.35, 0.15, fw);
    // crumbs (3-9 mm) within 6 cm of the edge in patches, gone where a pixel spans one.
    let per = 70.0;
    let p = (q + off) * per;
    let id = floor(p);
    let patchy = smoothstep(0.4, 0.75, st_noise((q + off) * 9.0));
    let spill = (1.0 - smoothstep(0.0, 0.06, -ed)) * (1.0 - smoothstep(0.004, 0.01, fw)) * patchy;
    if (st_hash(id + vec2<f32>(3.0, 61.0)) < spill * 0.5) {
        let centre = id + vec2<f32>(0.3) + 0.4 * vec2<f32>(st_hash(id + vec2<f32>(67.0, 1.0)), st_hash(id + vec2<f32>(1.0, 71.0)));
        let r = mix(0.1, 0.32, pow(st_hash(id + vec2<f32>(73.0, 5.0)), 2.0));
        let crumb = clamp((r - length(p - centre)) / max(fw * per, 1e-4) + 0.5, 0.0, 1.0);
        if (crumb > o.cover) {
            o.cover = crumb;
            o.tone = PR_BASIN_DRY * mix(1.0, 1.5, st_hash(id + vec2<f32>(79.0, 2.0)));
            o.tilt = vec2<f32>(0.0);
            o.curl = 0.3;
            o.rib = 0.0;
        }
    }
    return o;
}

struct PrBasinAt {
    q: vec2<f32>,
    dqx: vec2<f32>,
    dqy: vec2<f32>,
    off: vec2<f32>,
    half: vec2<f32>,
    ed: f32,
    fw: f32,
    seed: f32,
};

fn pr_basin_at(inst: PrInstance, local: vec3<f32>, dlx: vec3<f32>, dly: vec3<f32>) -> PrBasinAt {
    var a: PrBasinAt;
    let sc = vec2<f32>(inst.scale.x, inst.scale.z);
    a.q = local.xz * sc;
    a.dqx = dlx.xz * sc;
    a.dqy = dly.xz * sc;
    a.half = sc * 0.5;
    a.fw = max(length(abs(a.dqx) + abs(a.dqy)), 1e-5);
    a.seed = fract(dot(inst.pos.xz, vec2<f32>(0.0173, 0.0219))) * 61.0;
    a.off = vec2<f32>(a.seed, a.seed * 0.7);
    a.ed = min(a.half.x - abs(a.q.x), a.half.y - abs(a.q.y));
    return a;
}

// the cutout test: past the cut edge only the spilled crumbs and leaves.
fn pr_basin_cut(inst: PrInstance, local: vec3<f32>, dlx: vec3<f32>, dly: vec3<f32>) -> bool {
    let a = pr_basin_at(inst, local, dlx, dly);
    return a.ed >= 0.0 || pr_basin_spill(a.q, a.off, a.half, a.ed, a.fw).cover >= 0.5;
}

fn pr_basin(s: ptr<function, Surface>, inst: PrInstance, local: vec3<f32>, dlx: vec3<f32>, dly: vec3<f32>) {
    let a = pr_basin_at(inst, local, dlx, dly);
    let qs = a.q + a.off;
    var g = vec2<f32>(0.0);
    var tn = vec2<f32>(0.0);
    var albedo = PR_BASIN_DRY;
    var rough = 0.95;
    var ao = 1.0;
    let tc = vec2<f32>(pr_basin_trunk(inst), 0.0);
    let dt = a.q - tc;
    let rr = length(dt);
    // the scanned soil (CC0 `park_dirt`) where the sets are on, else the drawn one.
    let scan = cc0_on(CC0_RUN_GROUND);
    if (a.ed < 0.0) {
        let o = pr_basin_spill(a.q, a.off, a.half, a.ed, a.fw);
        albedo = o.tone * (1.0 - 0.3 * o.curl) * (1.0 - 0.25 * o.rib);
        g = o.tilt;
        rough = 0.8;
    } else {
        // damp patches, the watered ground round the trunk, the edge.
        let m = st_noise(qs * 2.3) * 0.6 + st_noise(qs * 6.1 + vec2<f32>(3.1)) * 0.4;
        let damp = clamp(smoothstep(0.5, 0.78, m) * select(0.6, 0.35, scan) + (1.0 - smoothstep(0.12, 0.5, rr)) * 0.4 + (1.0 - smoothstep(0.0, 0.06, a.ed)) * 0.25, 0.0, 1.0);
        let tone = mix(PR_BASIN_DRY, PR_BASIN_DAMP, damp);
        if (scan) {
            // the scan's detail round the basin's tone (half its own colour variation),
            // a fresh patch of it in every basin; its relief in cc0_ground_apply's frame.
            let l = CC0_L_PARK_DIRT;
            let inv = cc0_linv(l);
            let t = cc0_tap((a.q + a.off * 1.9) * inv, a.dqx * inv, a.dqy * inv, l);
            let rel = t.alb.rgb / max(cc0_lmean(l).rgb, vec3<f32>(1e-3));
            albedo = tone * max(mix(vec3<f32>(dot(rel, CC0_LUMA)), rel, 0.5), vec3<f32>(0.0));
            rough = clamp(t.rough / max(cc0_lmisc(l).x, 0.05) * mix(0.95, 0.82, damp), 0.3, 1.0);
            ao = clamp(t.ao / max(cc0_lmisc(l).y, 0.05), 0.3, 1.0);
            tn = t.n;
        } else {
            albedo = tone * mix(0.85, 1.12, st_noise(qs * 11.0));
            rough = mix(0.97, 0.84, damp);
            // mottled grain (2.5 and 1.2 cm) with its relief, fading as a pixel spans it.
            let gk = 1.0 - smoothstep(0.3, 0.8, a.fw * 80.0);
            if (gk > 0.0) {
                let gp = qs * 80.0;
                let g0 = st_noise(gp);
                let gx = st_noise(gp + vec2<f32>(0.35, 0.0));
                let gy = st_noise(gp + vec2<f32>(0.0, 0.35));
                g = g + vec2<f32>(gx - g0, gy - g0) / 0.35 * 80.0 * 0.0012 * gk;
                albedo = albedo * mix(1.0, mix(0.78, 1.2, g0 * 0.6 + st_noise(qs * 40.0) * 0.4), gk);
            }
            // clods and a few stones of decomposed granite.
            let c1 = pr_lumps(qs * 14.0, 1.0, 14.0, 0.008, 0.35, 1.0 - smoothstep(0.2, 0.6, a.fw * 14.0));
            let c2 = pr_lumps(qs * 40.0 + vec2<f32>(0.5), 2.0, 40.0, 0.003, 0.45, 1.0 - smoothstep(0.25, 0.7, a.fw * 40.0));
            let st = pr_lumps(qs * 24.0 + vec2<f32>(0.25), 3.0, 24.0, 0.004, 0.1, 1.0 - smoothstep(0.25, 0.7, a.fw * 24.0));
            g = g + c1.xy + c2.xy * (1.0 - c1.w) + st.xy * (1.0 - c1.w);
            ao = ao * (1.0 - 0.2 * c1.z) * (1.0 - 0.12 * c2.z) * (1.0 - 0.2 * st.z);
            albedo = albedo * mix(1.0, 1.12, c1.w) * mix(1.0, 1.06, c2.w);
            albedo = mix(albedo, vec3<f32>(0.15, 0.142, 0.128) * mix(0.75, 1.15, st_noise(qs * 90.0)), st.w * (1.0 - c1.w));
            rough = mix(rough, 0.6, st.w * (1.0 - c1.w));
            // grit: grains of sand and perlite, light or dark.
            let sp = qs * 160.0;
            let gid = floor(sp);
            let sk = 1.0 - smoothstep(0.3, 0.8, a.fw * 160.0);
            if (sk > 0.0 && st_hash(gid + vec2<f32>(83.0, 3.0)) > 0.9) {
                let gc = gid + vec2<f32>(0.3) + 0.4 * vec2<f32>(st_hash(gid + vec2<f32>(89.0, 7.0)), st_hash(gid + vec2<f32>(7.0, 97.0)));
                let grain = clamp((0.28 - length(sp - gc)) / max(a.fw * 160.0, 1e-4) + 0.5, 0.0, 1.0) * sk;
                let light = st_hash(gid + vec2<f32>(101.0, 9.0)) > 0.35;
                albedo = mix(albedo, select(vec3<f32>(0.025, 0.023, 0.021), vec3<f32>(0.21, 0.2, 0.185), light), grain);
            }
        }
        // the root flare: a low mound and the roots breaking the surface near the trunk.
        if (rr < 0.65) {
            let e = 0.003;
            let h0 = pr_root_h(dt, a.seed, a.fw);
            let hx = pr_root_h(dt + vec2<f32>(e, 0.0), a.seed, a.fw).x;
            let hy = pr_root_h(dt + vec2<f32>(0.0, e), a.seed, a.fw).x;
            g = g + vec2<f32>(hx - h0.x, hy - h0.x) / e;
            // the bark, streaked along the root and dusted with the soil it breaks through.
            let bark = h0.y;
            let streak = st_noise(vec2<f32>(rr * 14.0, atan2(dt.y, dt.x) * 40.0) + vec2<f32>(a.seed)) * 0.6 + st_noise(vec2<f32>(rr * 60.0, atan2(dt.y, dt.x) * 90.0)) * 0.4;
            let dust = smoothstep(0.35, 0.7, st_noise(dt * 45.0 + vec2<f32>(a.seed * 2.0))) * 0.6;
            albedo = mix(albedo, mix(PR_BASIN_ROOT * mix(0.7, 1.3, streak), albedo, dust), bark);
            rough = mix(rough, 0.9, bark);
            tn = tn * (1.0 - bark * (1.0 - dust));
        }
        // moss where the soil stays damp against the cut face.
        let moss = smoothstep(0.62, 0.8, st_noise(qs * 7.0 + vec2<f32>(5.0))) * (1.0 - smoothstep(0.02, 0.16, a.ed)) * (0.4 + 0.6 * damp);
        albedo = mix(albedo, PR_BASIN_MOSS * mix(0.8, 1.2, st_noise(qs * 60.0)), moss);
        // a cigarette end now and then (one cell in three of 45 cm), gone where a pixel spans its width.
        let bp = qs / 0.45;
        let bid = floor(bp);
        let bk = 1.0 - smoothstep(0.002, 0.005, a.fw);
        if (bk > 0.0 && st_hash(bid + vec2<f32>(107.0, 11.0)) < 0.33) {
            let bc = (bid + vec2<f32>(0.25) + 0.5 * vec2<f32>(st_hash(bid + vec2<f32>(109.0, 1.0)), st_hash(bid + vec2<f32>(3.0, 113.0)))) * 0.45;
            let ang = st_hash(bid + vec2<f32>(127.0, 5.0)) * 6.2832;
            let d = qs - bc;
            let u = d.x * cos(ang) + d.y * sin(ang);
            let v = -d.x * sin(ang) + d.y * cos(ang);
            let len = mix(0.008, 0.013, st_hash(bid + vec2<f32>(131.0, 7.0)));
            let w = length(vec2<f32>(max(abs(u) - len, 0.0), v));
            let butt = clamp((0.0038 - w) / a.fw + 0.5, 0.0, 1.0) * bk;
            let paper = select(vec3<f32>(0.3, 0.19, 0.1), vec3<f32>(0.38, 0.36, 0.33), u > -len * 0.35);
            albedo = mix(albedo, paper, butt);
            rough = mix(rough, 0.7, butt);
            tn = tn * (1.0 - butt);
            g = g + vec2<f32>(-sin(ang), cos(ang)) * clamp(v / 0.0038, -1.0, 1.0) * 0.8 * butt;
        }
        // leaf litter, piled against the cut edge (the scan has its own fine litter: fewer).
        let lk = 1.0 - smoothstep(0.02, 0.06, a.fw);
        let l1 = pr_leaves(a.q, a.off, a.half, 0.1, 1.0, select(0.07, 0.015, scan), select(0.6, 0.35, scan), 0.15, a.fw);
        let l2 = pr_leaves(a.q, a.off * 1.3, a.half, 0.075, 2.0, select(0.05, 0.0, scan), select(0.6, 0.25, scan), 0.0, a.fw);
        var lf = l1;
        if (l2.cover > 0.0) {
            lf = l2;
        }
        let lc = lf.cover * lk;
        if (lc > 0.0) {
            albedo = mix(albedo, lf.tone * (1.0 - 0.35 * lf.curl) * (1.0 - 0.3 * lf.rib), lc);
            g = mix(g, lf.tilt, lc);
            tn = tn * (1.0 - lc);
            rough = mix(rough, 0.78, lc);
            ao = mix(ao, 1.0 - 0.3 * lf.curl, lc);
        }
        // the soil meets the walk's cut face a little low: its shade, and the trunk's foot.
        let edge = smoothstep(0.0, 0.035, a.ed);
        ao = ao * mix(0.45, 1.0, edge) * mix(0.6, 1.0, smoothstep(0.0, 0.12, rr - 0.08));
        albedo = albedo * mix(0.72, 1.0, smoothstep(0.0, 0.022, a.ed));
    }
    let t = pr_rot(inst.yaw, vec3<f32>(1.0, 0.0, 0.0));
    let b = pr_rot(inst.yaw, vec3<f32>(0.0, 0.0, 1.0));
    // the scan's relief (u across, image-up along -y), then the drawn relief's gradient.
    let n0 = (*s).n;
    let ns = normalize(t * tn.x - b * tn.y + n0 * sqrt(max(0.0, 1.0 - dot(tn, tn))));
    let n = normalize(ns - t * g.x - b * g.y);
    if (all(n == n)) {
        (*s).n = n;
    }
    (*s).albedo = albedo;
    (*s).rough = rough;
    (*s).ao = ao;
}

// sf:FOLIAGE+
// ---- foliage light (`--foliage-light`; this block is in the shader only with it) ----
// A leaf is a thin two-sided scatterer: of the light on the face the eye sees it
// reflects R (look_light, with its normal turned to the eye), and of the light on
// its far face it passes T through to the eye, nearly diffusely. T is R's own
// pigment taken deeper: the ratio of transmittance to reflectance of mesophytic
// leaves (LOPEX / PROSPECT spectra, about 0.3 at 450 nm, 0.85 at 550 nm, 0.55 at
// 650 nm), so the light through a leaf is a more saturated yellow-green than the
// light off it. The far face takes the sun (shadowed like the key light and
// thinned by the clouds; a little of it in a forward lobe, Henyey-Greenstein
// g 0.5, the direct light through thin blades and edges) and the sky and the
// ground's light on the side the eye does not see (the ambient volume's sky
// visibility when it is on, the screen-space occlusion of the crown's depth).
// sf:end
// sf:FOLIAGE&!LEAF_SHADE+
const PR_LEAF_T_OVER_R: vec3<f32> = vec3<f32>(0.55, 0.85, 0.3);
// sf:end
// sf:FOLIAGE&LEAF_SHADE+
// (`--leaf-shade`) the ratio over each display primary's band, not at one
// wavelength: LOPEX93's fresh broadleaf leaves pass about 0.08 of 600 nm
// against 0.09 reflected (the red primary's band, 580-640 nm, mostly short of
// chlorophyll a's 665-680 nm trough), 0.12-0.13 of 550 nm against 0.13, and
// 0.01-0.02 of 450 nm against 0.05. At 650 nm alone (0.55) the light through a
// leaf read lime under a warm lamp; over the band it is the yellow-olive of a
// backlit leaf.
const PR_LEAF_T_OVER_R: vec3<f32> = vec3<f32>(0.8, 0.95, 0.35);
// sf:end
// sf:FOLIAGE+
const PR_LEAF_FWD_G: f32 = 0.5;
const PR_LEAF_FWD_SHARE: f32 = 0.3;
// a wet leaf's roughness at the pixel's scale (drops beaded on the waxy cuticle:
// glossier than the dry leaf's 0.7, never the mirror of a continuous film, which
// read as a sky-blue sheen over a whole crown at night).
const PR_LEAF_WET_ROUGH: f32 = 0.5;

// Light through a leaf toward the eye (radiance, before the fog): `n` the face the eye
// sees, `sh` the key light's shadow on the far face.
fn pr_foliage(albedo: vec3<f32>, n: vec3<f32>, v: vec3<f32>, world: vec3<f32>, sh: f32) -> vec3<f32> {
    let t = albedo * PR_LEAF_T_OVER_R;
    let b = -n;
    let l = look.key_dir.xyz;
    var e = vec3<f32>(0.0);
    let bl = dot(b, l);
    if (bl > 0.0) {
        let g = PR_LEAF_FWD_G;
        let cs = dot(-v, l);
        let hg = (1.0 - g * g) / (4.0 * LK_PI * pow(max(1.0 + g * g - 2.0 * g * cs, 1e-4), 1.5));
        let phase = (1.0 - PR_LEAF_FWD_SHARE) / LK_PI + PR_LEAF_FWD_SHARE * hg;
        e = look.key.xyz * bl * sh * sky_cloud_transmittance(world) * phase * LK_PI;
    }
    let env_i = look.ambient.w * look.env_class.x;
    var irr = mix(look.hemi_ground.xyz, look.hemi_sky.xyz, b.y * 0.5 + 0.5) + look.ambient.xyz + lk_sh(b) * env_i;
    if (amb_on()) {
        irr = amb_irradiance(amb_sample(world, b), b, lk_sh(b) * env_i);
    }
    var ao = 1.0;
    if (frame.copy_step.z < 0.5) {
        ao = ao_multi_bounce(ao_fetch(world).a);
    }
// sf:end
// sf:FOLIAGE&CROWN+
    ao = ao * pr_crown_sky; // (`--crown-light`) the sky through the leaves above
// sf:end
// sf:FOLIAGE+
    return t * (e + irr * ao) / LK_PI;
}

// sf:end
// sf:CROWN+
// ---- (`--crown-light`) a street tree's leaves in their crown ----
// A leaf card takes a measured mature leaf's chroma (hlod_trees.wgsl
// `FT_LEAF_CHROMA`, as the far crowns do, so the two levels agree; the
// cooked tints ran to a saturation of 0.7 on screen where a leaf's is about
// 0.35-0.45), its luminance kept; and the sky light reaching it falls with the
// leaf area above it (Monsi and Saeki's vertical Beer-Lambert profile, k 0.5
// over a crown's leaf area index of 2: e^-(1 - f), f its height in the crown
// from the wind weight), so a crown in shade is lit from its top, not
// evenly through. The sun's shade inside the crown is the cascades' (the
// leaves cast) and the cooked crown tone's.
const PR_LEAF_CHROMA: vec3<f32> = vec3<f32>(0.64, 1.17, 0.50);
const PR_CROWN_KL: f32 = 1.0; // k 0.5 x leaf area index 2
var<private> pr_crown_sky: f32 = 1.0;
// sf:end
// sf:LEAF_SHADE+
// ---- (`--leaf-shade`) the street's lights through a crown's leaves ----
// A street lamp with its own shadow map (`--light-shadows`) is already cut
// off by the fronds and leaves between it and a leaf (the props cast into it);
// every other light (a shop window's, a lamp's ground bounce, a headlamp) lit
// every leaf of a crown as if it stood alone, so a palm over a lit shop front
// glowed through to its top, the fronds facing away from the light as bright
// as those facing it. Such a light now reaches a leaf through the crown's leaf
// area on its path (sky_light.wgsl `sky_leaf_t`). A Canary Island date palm's
// crown: some 40 live fronds of about 3 m2 of leaflets over 5 m each, all from
// the heart, a leaf area per volume of about 2.9 / r^2 m2/m3 over the upper
// two thirds of a sphere (G 0.5 for leaflets facing every way: sigma 1.43 / r^2
// per metre, C 1.43 model m), out to 5.2 model m from a point 0.5 m under the
// heart (the drooping fronds' centroid), plus the rachises' and old leaf
// bases' little uniform part. A street tree's: an open-grown broadleaf crown,
// leaf area density about 0.8 m2/m3 (sigma 0.4 per metre), a sphere of 0.38
// of the tree's height round 0.64 of it.
const PR_LS_PALM: u32 = 7u; // eo_props::Model::Palm
const PR_LS_TREE: u32 = 6u; // eo_props::Model::Tree
const PR_LS_PALM_HEART: f32 = 6.9; // model m: PR_PALM_CROWN_Y less the fronds' droop
const PR_LS_PALM_R: f32 = 5.2; // model m
const PR_LS_PALM_K: vec2<f32> = vec2<f32>(0.03, 1.43); // per m, model m
const PR_LS_TREE_SIGMA: f32 = 0.4; // per m
// sf:end
// sf:FOLIAGE&WET_FOLIAGE+
// ---- (`--wet-foliage`) rain on leaves and bark ----
// A leaf's waxy cuticle does not soak: the rain stands on it as drops and,
// where they join, a film, over about half to most of a leaflet in steady
// rain (each its own share). Where it stands, the water's surface is the
// specular one, curved over the drops (GGX alpha about 0.05: the street lamps
// glint in it), and the leaf under it scatters a little less at its surface
// (Lekner and Dorf 1988 for a non-porous surface: some 15 % darker). A dead
// frond's straw and a trunk's bark and fibre are porous: they soak, about
// half as bright (Lekner and Dorf: the water's index fills the pores), their
// raised parts glossy with a film (roughness 0.3), darker still where the
// rain runs down the trunk. This replaces the leaf's earlier single wet tone
// (its 0.85 and 0.5), and gives the leaves both faces' water whichever way
// they face (the city's wetting takes a face seen from below as a soffit the
// rain does not reach).
const PR_WET_PALM: u32 = 7u; // eo_props::Model::Palm (its trunk: the atlas's u < 0.5)
const PR_WET_TREE: u32 = 6u; // eo_props::Model::Tree (its bark: the material without PR_LEAF)
const PR_WET_FILM_ROUGH: f32 = 0.22;
const PR_WET_LEAF_DARK: f32 = 0.15;
const PR_WET_SOAK_DARK: f32 = 0.5;
const PR_WET_BARK_ROUGH: f32 = 0.3;

// a leaf in the rain: `w` the wetness, `cell` 0..1 this leaf's own, `porous` 0
// (live, waxy) .. 1 (dead straw), `fw` the pixel's footprint (m).
fn pr_wet_leaf(s: ptr<function, Surface>, w: f32, cell: f32, porous: f32, fw: f32) {
    let cover = w * (0.45 + 0.55 * cell);
    (*s).albedo = (*s).albedo * (1.0 - cover * mix(PR_WET_LEAF_DARK, PR_WET_SOAK_DARK, porous));
    var r = mix((*s).rough, min((*s).rough, mix(PR_WET_FILM_ROUGH, PR_WET_BARK_ROUGH, porous)), cover);
    // where a pixel spans several leaves, wet and turned their own ways, the
    // spread of their normals widens the lobe (Toksvig, as the palm's leaflets)
    let a2 = r * r;
    r = sqrt(min(sqrt(a2 * a2 + 2.0 * 0.07 * smoothstep(0.03, 0.15, fw)), 1.0));
    (*s).rough = r;
}

// bark or fibre in the rain: soaked, the film on its raised parts, darker in the runs (`run` 0..1).
fn pr_wet_bark(s: ptr<function, Surface>, w: f32, run: f32) {
    let soak = w * mix(0.7, 1.0, run);
    (*s).albedo = (*s).albedo * (1.0 - PR_WET_SOAK_DARK * soak);
    (*s).rough = mix((*s).rough, min((*s).rough, PR_WET_BARK_ROUGH), soak);
}
// sf:end
// sf:FOLIAGE|PALM+
// the least n.v a leaf's shading normal keeps (it is seen, so it faces the eye)
const PR_LEAF_NV_MIN: f32 = 0.1;
// sf:end
// sf:PALM+
// ---- palm detail (`--palm-detail`; this block is in the shader only with it) ----
// The palm GLBs (public/assets/street-props/palm-v2.glb and palmFar-v2.glb, 10 m
// tall in their own metres, scaled by the tree's height over 10) carry one material
// on a shared atlas: the trunk's bark at u < 0.5 (u once round the trunk, v up it,
// 0.148 of v a metre), the crown shaft's fibre at u < 0.75 and v < 0.5, the
// rachises at v < 0.5 and the leaflets at v >= 0.5, each in four bands of age
// (dead, yellowing, mature, young); every leaflet is a few triangles at one v.
//
// Trunk: a Canary Island date palm's trunk is the stubs of its old leaf bases,
// cut back, in a diamond lattice on crossing spirals (about 10 round a trunk
// 0.85 m across, a base about 26 cm wide and 17 cm tall, each its own size), each a raised boss with
// the cut face looking up and out, and dark fibre packed in the grooves; the
// lower trunk weathered to a grey-brown, the fresh cuts under the crown orange
// brown, the rows wavering round the trunk. Drawn as a height field on the trunk's own (u, height): its slope bends
// the normal (the surface gradient of Mikkelsen 2010, from the screen
// derivatives of u and the height), its grooves take the sky's occlusion, and
// where a pixel spans a base it fades to the lattice's mean tone.
//
// Crown: the leaflets' tones are the atlas bands' gradients carried onto
// measured leaf reflectances (a mature leaflet's adaxial face about 0.06 / 0.10 /
// 0.05 linear sRGB, luminance ~0.09: the atlas's young band was twice that, the
// rachis three times), varied per leaflet (its v) and per frond (its bearing round the
// crown), and per palm (some yellowing: potassium deficiency is common in the
// city's date palms). The crown is a porous volume: the sky light reaching a
// frond falls with the leaf area between it and the open sky (Beer-Lambert,
// k 0.5: deep at the heart of the crown and under the upper fronds, open at the
// tips), and the leaflets, inserted in several planes along the rachis, face
// every way, so their shading normal leans toward the crown's own outward normal
// (top fronds lit from above, the underside dark) and each leaflet turns a little
// its own way.
const PR_PALM_CROWN_Y: f32 = 7.4; // the crown's heart (model m above the base)
const PR_PALM_TRUNK_R: f32 = 0.42; // the trunk's radius (model m)
const PR_PALM_AROUND: f32 = 10.0; // leaf bases round the trunk (whole: the lattice closes on itself)
const PR_PALM_BASE_H: f32 = 0.17; // a leaf base's height up the trunk (model m)
const PR_PALM_RELIEF: f32 = 0.03; // a base's rise out of its grooves (model m)
const PR_PALM_V_PER_M: f32 = 0.148; // the atlas's v per model metre up the trunk
const PR_PALM_LEAF_K: f32 = 0.5; // Beer-Lambert extinction of the sky light per unit leaf area
const PR_PALM_LEAN: f32 = 0.3; // how far a leaflet's shading normal leans to the crown's
// the variance of the leaflets' normals about their mean (the 0.22 jitter on three
// axes and the 30 % lean toward the crown's outward normal, ~0.07 a component)
const PR_PALM_NORMAL_VAR: f32 = 0.07;

// sf:end
// sf:PALM&PALM_VARIETY+
// ---- palm variety (`--palm-variety`, with `--palm-detail`; in the shader only with both) ----
// The city's Canary Island date palms are not clones: the crowns differ in their
// green (a deficient palm yellower, a vigorous one bluer and darker), the
// unpruned ones keep a skirt of dead fronds hanging under the live crown (grey-
// brown, drier with each year), about a third carry the orange date clusters
// on their stalks among the lower leaf bases, the bark weathers to its own tone,
// and many have the lower trunk skinned smooth by the city's pruning crews (the
// old leaf bases cut off flush, a pale fibrous column). Each palm draws its own
// from hashes of where it stands.
struct PrPalmVariety {
    hue: vec3<f32>,
    skirt: f32, // 0 pruned .. 1 a full skirt
    skirt_y: f32, // how far under the crown's heart the skirt starts (model m)
    fruit: f32, // 0 or 1: dates
    bark: vec3<f32>,
    skin: f32, // 0 or 1: the lower trunk skinned
    skin_h: f32, // up to here (model m)
    around: f32, // leaf bases round the trunk (9 to 13, whole)
    base_h: f32, // a leaf base's height over PR_PALM_BASE_H
};

fn pr_palm_variety(inst: PrInstance) -> PrPalmVariety {
    let h1 = st_hash(inst.pos.xz * 0.71 + vec2<f32>(9.1, 2.3));
    let h2 = st_hash(inst.pos.xz * 1.13 + vec2<f32>(4.4, 8.8));
    let h3 = st_hash(inst.pos.xz * 0.29 + vec2<f32>(1.7, 6.2));
    let h4 = st_hash(inst.pos.xz * 0.53 + vec2<f32>(7.3, 3.9));
    var o: PrPalmVariety;
    // the crown's green: yellower and paler to bluer and darker (about +-8 % in r and b)
    o.hue = mix(vec3<f32>(1.14, 1.04, 0.8), vec3<f32>(0.88, 0.96, 1.14), h1) * (0.82 + 0.36 * h2);
    o.skirt = select(0.0, mix(0.7, 1.0, h2), h3 < 0.5);
    o.skirt_y = mix(1.4, 0.4, h2);
    o.fruit = select(0.0, 1.0, h4 < 0.35);
    o.bark = mix(vec3<f32>(0.88, 0.93, 1.04), vec3<f32>(1.1, 0.99, 0.88), h3) * (0.85 + 0.3 * h4);
    o.skin = select(0.0, 1.0, h1 > 0.8);
    o.skin_h = mix(0.7, 1.6, h4);
    // (r2i2) the lattice's own count and pitch: the spirals a palm grew (8 to 13
    // contact parastichies on Canary Island date palms), the bases' heights with its vigour
    o.around = 9.0 + floor(fract(h2 * 7.13 + h4 * 3.7) * 4.999);
    o.base_h = mix(0.82, 1.3, fract(h1 * 5.31 + h3 * 2.9));
    return o;
}
// sf:end
// sf:PALM+
// the lattice's height (0 groove .. 1 boss top) at (a round, b up) in bases: each
// base its own (`j`, three hashes 0..1): its size, how far its cut was rounded
// (a sharp diamond to a worn oval), how high it stands out of the fibre, and a
// chipped corner on some; its row wavering round the trunk.
fn pr_palm_h(a: f32, b: f32, j: vec3<f32>) -> f32 {
    let p = vec2<f32>(a + b, a - b);
    let f = fract(p) - vec2<f32>(0.5);
    let diamond = 2.0 * max(abs(f.x), abs(f.y));
    let oval = 2.0 * length(f); // >= the diamond's: a rounder base, never past its cell
    let e = mix(diamond, oval, 0.75 * j.y) * (0.86 + 0.28 * j.x);
    let up = f.x - f.y; // -1 the base's foot .. 1 its top (b within the cell)
    let chip = 1.0 - 0.6 * step(0.7, j.z) * smoothstep(0.1, 0.45, f.x * select(1.0, -1.0, j.z > 0.85));
    return (1.0 - smoothstep(0.66, 1.0, e)) * (0.7 + 0.3 * up) * (0.65 + 0.5 * j.z) * chip;
}

// The palm's surface; returns how much of it is thin leaf (for `--foliage-light`).
fn pr_palm(s: ptr<function, Surface>, inst: PrInstance, local: vec3<f32>, uv: vec2<f32>, ng: vec3<f32>, view: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>, duvx: vec2<f32>, duvy: vec2<f32>, dlx: vec3<f32>, dly: vec3<f32>) -> f32 {
    let seed = st_hash(inst.pos.xz * 0.37 + vec2<f32>(3.1, 7.7)); // this palm
    let rel = local - vec3<f32>(0.0, PR_PALM_CROWN_Y, 0.0);
    let rh = length(rel.xz);
    let fw = max(length(dpx), length(dpy)); // the pixel's footprint (world m)
    let sc = max(inst.scale.y, 1e-3);
    if (uv.x < 0.5) {
// sf:end
// sf:PALM&PALM_VARIETY+
        // (`--palm-variety`) this palm's lattice: its count round the trunk and its
        // bases' height (these function-scope lets stand in for the module's
        // constants of the same names in the lines below), and the weathering
        // that breaks it up: patches where the bases are worn down to the fibre
        // (their relief nearly gone), on the lower and middle trunk.
        let pvt = pr_palm_variety(inst);
        let relief0 = PR_PALM_RELIEF;
        let wear = smoothstep(0.52, 0.74, st_noise(vec2<f32>(uv.x * 6.0 + seed * 5.0, local.y * 0.55 + seed * 3.0))) * (1.0 - smoothstep(3.5, 5.5, local.y));
        let PR_PALM_AROUND = pvt.around;
        let PR_PALM_BASE_H = 0.17 * pvt.base_h;
        let PR_PALM_RELIEF = relief0 * (1.0 - 0.8 * wear);
// sf:end
// sf:PALM+
        // the trunk: the leaf-base lattice in bases round (a) and up (b).
        let b = local.y / PR_PALM_BASE_H;
        let a = uv.x * 2.0 * PR_PALM_AROUND + 0.6 * (st_noise(vec2<f32>(b * 0.45, seed * 9.0)) - 0.5) + 0.25 * (st_noise(vec2<f32>(b * 1.7, seed * 5.0)) - 0.5); // the spirals waver (in b only: the seam still closes)
        // its cell, for the base's own tone and size (round the trunk modulo the count, so the seam closes)
        let c1 = floor(a + b);
        let c2 = floor(a - b);
        let ca = (c1 + c2) * 0.5;
        let cb = (c1 - c2) * 0.5;
        let cid = vec2<f32>(ca - PR_PALM_AROUND * floor(ca / PR_PALM_AROUND), cb);
        let cell = st_hash(cid + vec2<f32>(seed * 17.0, 1.7));
        let cj = vec3<f32>(cell, st_hash(cid + vec2<f32>(5.3, seed * 11.0)), st_hash(cid + vec2<f32>(seed * 7.0, 9.9)));
        let h = pr_palm_h(a, b, cj);
        let eps = 0.03;
        let ha = (pr_palm_h(a + eps, b, cj) - h) / eps;
        let hb = (pr_palm_h(a, b + eps, cj) - h) / eps;
        let k = 1.0 - smoothstep(0.02 * sc, 0.07 * sc, fw); // the lattice resolved
        // the surface gradient of the height (world m): H_u grad u + H_y grad y.
        let r1 = cross(dpy, ng);
        let r2 = cross(ng, dpx);
        let det = dot(dpx, r1);
        if (abs(det) > 1e-12 && k > 0.0) {
            let gu = (r1 * duvx.x + r2 * duvy.x) / det;
            let gy = (r1 * dlx.y + r2 * dly.y) / det;
            let amp = PR_PALM_RELIEF * sc;
            let grad = amp * (ha * 2.0 * PR_PALM_AROUND * gu + hb / PR_PALM_BASE_H * gy);
            let nb = normalize((*s).n - grad * k);
            if (all(nb == nb)) {
                (*s).n = nb;
            }
        }
        // tones: weathered grey-brown below, fresh orange-brown cuts under the crown,
        // dark fibre in the grooves, each base its own; soil splash at the foot.
        let top = smoothstep(4.6, 6.4, local.y);
        var bark = mix(vec3<f32>(0.185, 0.165, 0.140), vec3<f32>(0.230, 0.150, 0.085), top);
        bark = bark * (0.86 + 0.28 * cell) * mix(vec3<f32>(1.0), vec3<f32>(0.92, 0.97, 1.06), step(0.82, cell) * (1.0 - top)); // a greyer, lichened base now and then
        let fibre = vec3<f32>(0.085, 0.068, 0.050);
        let groove = 1.0 - smoothstep(0.0, 0.25, h);
        let streak = 0.9 + 0.2 * st_noise(vec2<f32>(a * 9.0, local.y * 1.3));
        let near_tone = mix(bark * streak, fibre, groove);
        let mean_tone = mix(bark, fibre, 0.15);
        var alb = mix(mean_tone, near_tone, k);
        alb = alb * mix(0.7, 1.0, smoothstep(0.0, 0.7, local.y)); // splash and grime at the foot
// sf:end
// sf:PALM&PALM_VARIETY+
        // (`--palm-variety`) this palm's bark, and on some the lower trunk skinned smooth
        let pv = pr_palm_variety(inst);
        alb = alb * pv.bark;
        // the worn patches' fibre, weathered grey; rain's dark streaks run down from
        // under the crown; grey-green crustose lichen on the trunk's damper side
        alb = mix(alb, vec3<f32>(0.20, 0.188, 0.165) * (0.9 + 0.2 * st_noise(vec2<f32>(uv.x * 30.0, local.y * 4.0))), 0.65 * wear * k);
        let drip = smoothstep(0.6, 0.85, st_noise(vec2<f32>(uv.x * 36.0 + seed * 11.0, local.y * 0.22))) * smoothstep(0.8, 2.5, local.y);
        alb = alb * (1.0 - 0.3 * drip * k);
        let side = 0.5 + 0.5 * cos((uv.x * 2.0 + seed) * 6.2831853);
        let lichen = smoothstep(0.66, 0.82, st_noise(vec2<f32>(uv.x * 24.0 + 3.0, local.y * 2.2 + seed * 9.0))) * side * (1.0 - smoothstep(4.0, 6.0, local.y));
        alb = mix(alb, vec3<f32>(0.235, 0.245, 0.200), 0.7 * lichen * k);
        let skin = pv.skin * (1.0 - smoothstep(pv.skin_h - 0.25, pv.skin_h + 0.25, local.y + 0.3 * (st_noise(vec2<f32>(uv.x * 20.0, seed * 3.0)) - 0.5)));
        if (skin > 0.0) {
            // the cut leaf bases' scars: shallow rings a base's height apart, fibre between
            let ring = 0.9 + 0.1 * st_noise(vec2<f32>(local.y * 9.0, uv.x * 3.0 + seed * 7.0));
            let scar = sin(local.y / PR_PALM_BASE_H * 6.2831853 + 2.0 * st_noise(vec2<f32>(uv.x * 10.0, seed)));
            alb = mix(alb, vec3<f32>(0.205, 0.182, 0.152) * ring * (0.92 + 0.08 * scar) * mix(0.75, 1.0, smoothstep(0.0, 0.6, local.y)), skin);
            let nsk = normalize(ng + vec3<f32>(0.0, 0.22 * scar * k, 0.0));
            (*s).n = normalize(mix((*s).n, nsk, skin * 0.85));
        }
// sf:end
// sf:PALM+
        (*s).albedo = alb;
        (*s).rough = 0.92;
        (*s).metal = 0.0;
        // the grooves' occlusion, and the crown's shade over the top of the trunk
        let cav = mix(0.85, mix(1.0, 0.55, groove), k);
        (*s).ao = (*s).ao * cav * exp(-PR_PALM_LEAF_K * 1.0 * smoothstep(4.8, 6.9, local.y));
        return 0.0;
    }
    // the crown: the band (age) and part.
    let fibre_part = uv.x < 0.75 && uv.y < 0.5;
    let leaflet = uv.y >= 0.5;
    let band = i32(clamp(floor(fract(uv.y * 2.0) * 4.0), 0.0, 3.0)); // 0 dead, 1 yellowing, 2 mature, 3 young
    // the sky light through the crown: leaf area over the point
    let lai = 1.3 * (1.0 - smoothstep(0.3, 3.8, rh)) + 0.5 * smoothstep(0.5, -1.8, rel.y) * (1.0 - smoothstep(2.4, 4.2, rh));
    (*s).ao = (*s).ao * exp(-PR_PALM_LEAF_K * lai);
    if (fibre_part) {
        (*s).albedo = (*s).albedo * 0.8;
        (*s).rough = 0.9;
        return 0.0;
    }
    let lum = dot((*s).albedo, vec3<f32>(0.2126, 0.7152, 0.0722));
    // each frond (its bearing round the crown) and each leaflet (its v) its own
    let bearing = atan2(rel.z, rel.x) * 4.8 + seed * 31.0;
    let frond = st_noise(vec2<f32>(bearing, seed * 13.0));
    let lv = sin(uv.y * 31415.0);
    let lw = sin(uv.y * 21377.0 + 1.3);
    var target_c: vec3<f32>;
    var band_lum: f32;
    var thin = 1.0;
    if (leaflet) {
        if (band == 0) {
            target_c = vec3<f32>(0.160, 0.120, 0.082); // dead: dry, pale brown-grey
            band_lum = 0.123;
            thin = 0.35;
        } else if (band == 1) {
            target_c = vec3<f32>(0.160, 0.132, 0.050); // yellowing
            band_lum = 0.188;
        } else if (band == 2) {
            target_c = vec3<f32>(0.062, 0.098, 0.046); // mature
            band_lum = 0.099;
        } else {
            target_c = vec3<f32>(0.082, 0.124, 0.052); // young
            band_lum = 0.186;
        }
        (*s).rough = 0.52 + 0.1 * lw;
    } else {
        // the rachis: thick, yellow-green to orange at the base
        if (band == 0) {
            target_c = vec3<f32>(0.150, 0.110, 0.070);
            band_lum = 0.133;
        } else if (band == 1) {
            target_c = vec3<f32>(0.180, 0.150, 0.065);
            band_lum = 0.273;
        } else if (band == 2) {
            target_c = vec3<f32>(0.130, 0.130, 0.056);
            band_lum = 0.265;
        } else {
            target_c = vec3<f32>(0.140, 0.150, 0.060);
            band_lum = 0.307;
        }
        (*s).rough = 0.62;
        thin = 0.15;
    }
    var alb = target_c * clamp(lum / band_lum, 0.5, 1.6);
    // a palm's health, a frond's age within its band, a leaflet's own tone
    let yellow = smoothstep(0.55, 0.95, seed * 0.6 + frond * 0.55) * select(0.0, 1.0, band == 2 && leaflet);
    alb = mix(alb, vec3<f32>(0.105, 0.098, 0.040), yellow * 0.45);
    alb = alb * (0.86 + 0.28 * frond) * (1.0 + 0.12 * lv);
// sf:end
// sf:PALM&PALM_VARIETY+
    // (`--palm-variety`) this palm's crown: its own green, a skirt of dead fronds on
    // some, and on others the orange date clusters hanging under the crown's heart
    let pvc = pr_palm_variety(inst);
    alb = alb * pvc.hue;
    let low = 1.0 - smoothstep(-pvc.skirt_y - 0.35, -pvc.skirt_y + 0.35, rel.y + 0.5 * (frond - 0.5));
    let dead = pvc.skirt * low * smoothstep(0.15, 0.4, frond + 0.3 * (st_noise(vec2<f32>(bearing * 2.3, seed * 7.0)) - 0.5));
    let dry = vec3<f32>(0.150, 0.122, 0.090) * (0.85 + 0.3 * frond) * (1.0 + 0.1 * lv);
    alb = mix(alb, dry, dead);
    thin = mix(thin, 0.3, dead);
    let heart = (1.0 - smoothstep(0.6, 1.5, rh)) * smoothstep(-1.9, -1.3, rel.y) * (1.0 - smoothstep(-0.5, 0.0, rel.y));
    let bunch = smoothstep(0.55, 0.75, st_noise(vec2<f32>(bearing * 1.1 + 3.0, rel.y * 4.0 + seed * 11.0)));
    let fruit = pvc.fruit * heart * bunch;
    alb = mix(alb, vec3<f32>(0.42, 0.17, 0.025) * (0.8 + 0.4 * frond), fruit);
    thin = thin * (1.0 - fruit);
// sf:end
// sf:PALM+
    (*s).albedo = alb;
    (*s).metal = 0.0;
    // the leaflets' normals: toward the crown's outward normal, each turned its own way
    let nv = pr_rot(inst.yaw, normalize(rel / vec3<f32>(4.0, 2.4, 4.0)));
    let jitter = vec3<f32>(lv, 0.5 * lw, sin(uv.y * 17713.0 + 2.1)) * 0.22;
    var nl = normalize(mix((*s).n, nv, PR_PALM_LEAN) + jitter);
    // A shading normal turned from the eye (a leaflet on the crown's far side
    // leaning out, away from the eye) makes the microfacet terms blow up at
    // grazing (n.v near 0: the Fresnel near 1, the visibility term unbounded):
    // the white slivers on night and wet fronds. The leaflet is seen, so its
    // normal is kept in the eye's hemisphere (n.v at least PR_LEAF_NV_MIN).
    let nvd = dot(nl, view);
    if (nvd < PR_LEAF_NV_MIN) {
        nl = normalize(nl + view * (PR_LEAF_NV_MIN - nvd));
    }
    if (all(nl == nl)) {
        (*s).n = nl;
    }
    // Leaflets narrower than the pixel each turn their own way: the pixel sees
    // the spread of their normals as a wider lobe (Toksvig's normal-variance
    // roughening: alpha^2 + 2 sigma^2), not one leaflet's glint.
    let sub = smoothstep(0.008, 0.04, fw / sc);
    let a2 = (*s).rough * (*s).rough;
    (*s).rough = sqrt(min(sqrt(a2 * a2 + 2.0 * PR_PALM_NORMAL_VAR * sub), 1.0));
    return thin;
}
// sf:end
// ---- (`--luminaires`) a district light's lit glass ----
// eo-render props_lib `luminaire_sockets`, props_lib2 `add_mesh_glow`: a glass
// vertex's colour carries its socket's shading (r its area / 0.5 m^2, g the
// socket's height / 10 m (model), b its share of the post's light, a 0.25 a
// lantern or 0.5 a lens; every other vertex's alpha is 1). Shaded in the
// vertex stage (the lit fragment stands at Apple's compiler's limit), its
// radiance handed to the fragment in the colour. The lamp's intensity is the gather's for that socket
// (sky_lights.wgsl `lights_props` under the look: the RP-8 pool's nadir from
// the socket's height, its share, its unit's factor); the glass shows it as:
// kind 1, a lantern's clear panes, seen from the side: the lamp inside is the fx
// pass's core (sky_fx.wgsl), the panes the light their seeds, dirt and inner
// faces scatter, `LUM_PANE_SHARE` of the intensity over their projected area,
// brightest level with the lamp (the housing's frame stays dark between them);
// kind 2, a cobra head's flat lens facing down: I(theta) / (A cos theta), its
// intensity toward the eye by the full-cutoff tail (eo-sky lit_fog
// `luminaire_shape`), dark from above. Linear radiance in the lamps' units
// (their units per lux carry the practicals' gain: the caller takes look_light's off).
// INFERRED (`LUM_PANE_SHARE`): a lantern's clear or seeded glass returns a tenth
// of its lamp's luminance over the pane (frosted glass would be the whole).
const LUM_PANE_SHARE: f32 = 0.1;
const PR_LAMP_GLOW: u32 = 1048576u; // props_lib P_LAMP_GLOW (1 << 20)
fn pr_lamp_glass(c: vec4<f32>, inst: PrInstance, local: vec3<f32>, v: vec3<f32>) -> vec3<f32> {
    if ((skyu.flags.x & LUM_BIT) == 0u) {
        return vec3<f32>(0.0);
    }
    let g = vec4<f32>(select(1.0, 2.0, c.a > 0.375), c.r * 0.5, c.g * 10.0, c.b);
    let units = SF_LAMP_UNITS && (skyu.flags.x & LU_BIT) != 0u;
    let uf = lu_factor(inst.pad, skyu.night.x, units);
    let h = g.z * inst.scale.y;
    let i = LUM_POOL_PEAK_LUX * skyu.bloom_w2.z * skyu.night.x * h * h * g.w * uf;
    var l = 0.0;
    if (g.x < 1.5) {
        let dy = (local.y - g.z) / 0.2;
        l = i / max(g.y, 1e-3) * (0.55 + 0.9 * exp(-dy * dy)) * LUM_PANE_SHARE;
    } else {
        let c = -v.y;
        if (c > 0.0) {
            let tail = pow(min(c / LUM_EYE_FULL_COS, 1.0), LUM_EYE_TAIL_POWER);
            let share = clamp((1.0 + 1.5 * (1.0 - c * c)) * tail, 0.03, 1.0);
            l = i * share / (max(g.y, 1e-3) * max(c, 0.05));
        }
    }
    let col = select(LUM_COLOR_WARM, LUM_COLOR_LED, inst.model == LUM_MODEL_COBRA);
    return col * lu_tint(inst.pad, units) * l;
}

// ---- (`--signal-spill`) a signal's lens at night ----
// The lit lens the light the gather spills from it (sky.rs `set_signal_lights`):
// its intensity toward the eye by the signal's lobe (eo-sky lights `SIGNAL_LOBE`:
// bright facing the eye, dim from the side, none from behind) over its 300 mm
// area, in the lamps' units; the dark aspects dark glass. Shaded in the vertex
// stage (the lit fragment stands at Apple's compiler's limit); the fragment
// blends it in by the night (by day the display-referenced lens as before).
fn pr_signal_lens(flags: u32, inst: PrInstance, v: vec3<f32>) -> vec3<f32> {
    let k = (flags >> 24u) & 3u;
    if (pr_lamps[inst.lamp] != k || k > 2u) {
        return vec3<f32>(0.0);
    }
    let i = NT_SIGNAL_CD[k] * nt_lobe(v, pr_rot(inst.yaw, vec3<f32>(0.0, 0.0, 1.0)), NT_SIGNAL_V, NT_SIGNAL_H);
    return NT_SIGNAL_RGB[k] * i / NT_SIGNAL_LENS_M2 * skyu.bloom_w2.z;
}

fn pr_shade(in: PrOut, front: bool) -> vec4<f32> {
// sf:PROP_FAR+
    lk_far_vertex = in.far_sun; // (`--prop-far-shadow`)
// sf:end
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
    if ((m.flags & PR_LEAF) != 0u && m.params.w > 0.0) {
        // the flora pass's leaf sprigs (vegetation_models.rs `SPRIG_LUMINANCE`): the atlas
        // sprig drawn grey, its luminance over the cells' mean, so the vertex colour is
        // the species' leaf albedo.
        albedo_tex = vec4<f32>(vec3<f32>(dot(albedo_tex.rgb, vec3<f32>(0.2126, 0.7152, 0.0722)) * m.params.w), albedo_tex.a);
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
    if (inst.model == PR_TREE_BASIN) {
        pr_basin(&s, inst, in.local, dlx, dly);
    }
    m.emissive = vec4<f32>(0.0);
    if ((m.flags & PR_LENS) != 0u) {
        // Lens radiance solved in the tone mapper's units (SIGNAL_LENS_TONE_INPUT):
        // v x ACES scale / exposure, lit or dark glass.
        let k = (m.flags >> 24u) & 3u;
        let lit = pr_lamps[inst.lamp] == k;
        let tone = select(pr_lens[3u + k].rgb, pr_lens[k].rgb, lit);
        m.emissive = vec4<f32>(tone * pr.lens.x, 0.0);
        if (SF_SIGNAL_SPILL && (skyu.flags.x & NT_SIGNAL_BIT) != 0u) {
            // (`--signal-spill`) as the night comes, the lens its own light's (`pr_signal_lens`,
            // in the vertex stage: its radiance rides the colour).
            m.emissive = vec4<f32>(mix(m.emissive.rgb, in.color, skyu.night.z), 0.0);
        }
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
    if (SF_LUMINAIRES && (m.flags & PR_LAMP_GLOW) != 0u) {
        // (`--luminaires`) a district light's glass lit by its own lamp (`pr_lamp_glass`, in the
        // vertex stage: its radiance rides the colour), over the glass's small diffuse return.
        m.emissive = vec4<f32>(in.color, 0.0); // (the division below takes look_light's gain back off)
        s.albedo = m.base.rgb * albedo_tex.rgb * 0.3;
    }
    // (`--night-photometry`) a lens's radiance is solved in display terms (x 1 / exposure):
    // look_light's practicals' gain does not apply to it (it lit the dark aspects).
    m.emissive = vec4<f32>(m.emissive.rgb / lk_practical_gain(), 0.0);
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
                var soil = m.base.rgb * (0.78 + 0.34 * clod + 0.22 * (grit - 0.5)) * mix(vec3<f32>(1.0), vec3<f32>(1.18, 1.12, 1.05), crust);
                if (PR_CHERT_GROUND) {
                    // the flora pass's red chert soil (eo_landmarks::flora CHERT_TOP_SRGB,
                    // CHERT_FACE_SRGB: the knoll's measured top and cut face) by the ground's
                    // slope, with angular tabular chips of chert rubble (5-30 cm talus, the
                    // guide's estimate; here the finer gravel, 4-8 cm), lighter or darker
                    // than the soil, each faded to its mean where a pixel spans it.
                    let up = clamp(normalize(in.normal).y, 0.0, 1.0);
                    let base = mix(PR_CHERT_FACE, PR_CHERT_TOP, smoothstep(0.62, 0.85, up));
                    soil = base * (0.86 + 0.24 * clod + 0.16 * (grit - 0.5)) * mix(vec3<f32>(1.0), vec3<f32>(1.06, 1.04, 1.02), crust);
                    let cq = q * 16.0;
                    let cell = cq - fract(cq); // (`floor` is the mat's local above)
                    let ch = pr_hash3(vec3<f32>(cell, 21.0));
                    let fq = fract(cq) - vec2<f32>(0.5);
                    let turn = ch * 6.283;
                    let rq = vec2<f32>(fq.x * cos(turn) - fq.y * sin(turn), fq.x * sin(turn) + fq.y * cos(turn));
                    let chip = step(0.55, ch) * (1.0 - step(0.3, abs(rq.x) + 1.6 * abs(rq.y)));
                    let tone = mix(0.7, 1.25, pr_hash3(vec3<f32>(cell, 22.0)));
                    soil = soil * mix(1.0, tone, chip * mid);
                }
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
                if ((inst.model >= PR_VEG_BASE + PR_VEG_MOUND && inst.model < PR_VEG_BASE + PR_VEG_MOUND_END) || (inst.model >= PR_VEG_BASE + PR_VEG_SPECIES && inst.model < PR_VEG_BASE + PR_VEG_SPECIES_END)) {
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
                // (`--no-city-grass`) the city's annual canopy carries its depth in its
                // vertex colours (vegetation_models.rs `canopy_light`): not again per blade;
                // its sky light falls down the canopy by height (`pr_annual_sky`).
                let annual = inst.model == PR_VEG_BASE + PR_VEG_ANNUAL;
                let open = select(clamp(0.3 + 0.7 * along, 0.0, 1.0), 1.0, annual);
                s.albedo = s.albedo * mix(vec3<f32>(0.68, 0.75, 0.85), vec3<f32>(1.0), open);
                s.ao = select(mix(0.45, 1.0, open), pr_annual_sky(in.local.y), annual);
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
                    // (the annual canopy's cured straw passes its own hue: `ANNUAL_STRAW_T`.)
                    blade_t = select(s.albedo * vec3<f32>(0.95, 0.9, 0.3) * 0.9, s.albedo * PR_ANNUAL_T, annual);
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
        if ((inst.lamp >> 24u) == PR_TIP_RIBBON) {
            // the flora pass's ribbon chert (eo_landmarks::flora TIP_RIBBON): hard red
            // chert beds 2-10 cm thick parted by thin dark shale, folded (field guide
            // 4.1: "highly folded bedded chert", chevron folds; beds 2.5-5 cm on
            // average, secondary source). The beds dip by the block's yaw; folds are
            // a sine of a few metres along the strike; each bed its own value; the
            // partings fade to the beds' mean where a pixel spans a bed.
            let fw = length(dpx) + length(dpy);
            let dip = vec3<f32>(sin(inst.yaw) * 0.7, 1.0, cos(inst.yaw) * 0.7);
            let strike = normalize(vec3<f32>(cos(inst.yaw), 0.0, -sin(inst.yaw)));
            let along = dot(in.world, strike);
            let fold = 0.35 * sin(along * 1.7 + inst.yaw * 3.0) + 0.12 * sin(along * 4.3);
            let beds = (dot(in.world, normalize(dip)) + fold) / 0.06;
            let bed = floor(beds);
            let f = fract(beds);
            let kb = 1.0 - smoothstep(0.02, 0.09, fw);
            let value = 0.82 + 0.34 * pr_hash3(vec3<f32>(bed, 5.0, 9.0));
            let parting = 1.0 - 0.55 * (1.0 - smoothstep(0.0, 0.12, f)) ;
            s.albedo = s.albedo * mix(0.92, value * parting, kb);
            // the beds stand proud of the partings: a groove in the normal.
            let gdir = normalize(dip);
            let groove = (1.0 - smoothstep(0.0, 0.12, f)) * kb;
            let nb = normalize(s.n + gdir * groove * 0.35);
            if (all(nb == nb)) {
                s.n = nb;
            }
        }
        if ((inst.lamp >> 24u) == PR_TIP_SCALY) {
            // the flora pass's serpentinite (eo_landmarks::flora TIP_SCALY): a scaly,
            // sheared fabric, slick polished lens-shaped scales (field guide 4.2), as
            // cells of noise stretched along a foliation turned by the block's yaw;
            // each lens its own value and polish, darker seams between, and the
            // weathered skin buff to rusty brown on the upward faces.
            let fw = length(dpx) + length(dpy);
            let ks = 1.0 - smoothstep(0.03, 0.15, fw);
            let fol = vec3<f32>(cos(inst.yaw), 0.35, sin(inst.yaw));
            let p = in.world * 3.0;
            let along = dot(p, normalize(fol));
            let q = p - normalize(fol) * along * 0.75;
            let lens = pr_noise3(q * 1.6 + vec3<f32>(along * 0.4));
            let seam = smoothstep(0.42, 0.5, lens) * (1.0 - smoothstep(0.5, 0.58, lens));
            let scale = pr_noise3(q * 0.8 + vec3<f32>(13.0));
            s.albedo = s.albedo * mix(1.0, (0.85 + 0.3 * scale) * (1.0 - 0.45 * seam), ks);
            let rust = smoothstep(0.55, 0.8, pr_noise3(in.world * 0.9 + vec3<f32>(3.0))) * clamp(s.n.y, 0.0, 1.0) * 0.55;
            s.albedo = mix(s.albedo, vec3<f32>(dot(s.albedo, vec3<f32>(0.33))) * vec3<f32>(1.35, 1.05, 0.7), rust);
            // polished scales: smoother where the lens is whole, rough at the seams.
            s.rough = mix(s.rough, mix(0.38, 0.8, seam), ks * (1.0 - rust));
        }
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
// sf:FOLIAGE|PALM+
    var palm_leaf = -1.0;
// sf:end
// sf:PALM+
    if (inst.model == PR_PALM) {
        palm_leaf = pr_palm(&s, inst, in.local, in.uv, n, v, dpx, dpy, duvx, duvy, dlx, dly); // (`--palm-detail`)
    }
// sf:end
// sf:WEATHER_FX+
    if ((m.flags & PR_LEAF) != 0u && sky_weather_fx_on(skyu.cirrus)) {
        // Subtle normal shimmer in sync with leaf flutter so wet/sunlit leaves catch light as they wiggle.
        let ws = pr.wire.w;
        let lt = pr.lens.z * (6.5 + 8.5 * ws);
        let lp = dot(in.local, vec3<f32>(5.3, 4.1, 6.2)) + in.uv.x * 3.1;
        let tilt = (0.02 + 0.06 * ws) * sin(lt + lp);
        s.n = normalize(s.n + vec3<f32>(pr.wind.x * tilt, tilt * 0.35, pr.wind.y * tilt));
        if (skyu.wet.x > 0.05) {
            // Rain-wetted foliage darkens slightly with a soft waxy cuticle sheen
            // (kept well above the 0.36 SSR cutoff so leaf cards never mirror the sky).
            let lw = skyu.wet.x;
            s.albedo = s.albedo * mix(1.0, 0.86, lw);
            s.rough = mix(s.rough, min(s.rough, 0.54), lw * 0.5);
        }
    }
// sf:end
    s.rough = clamp(s.rough, 0.03, 1.0);
// sf:CROWN+
    if ((m.flags & PR_LEAF) != 0u) {
        // (`--crown-light`) a measured leaf's chroma, the sky through the leaves above
        let lum = dot(s.albedo, vec3<f32>(0.2126, 0.7152, 0.0722));
        s.albedo = mix(s.albedo, PR_LEAF_CHROMA * lum, 0.5);
        let f = pow(clamp((in.sway - 0.15) / 0.85, 0.0, 1.0), 0.6667);
        pr_crown_sky = exp(-PR_CROWN_KL * (1.0 - f));
        s.ao = s.ao * pr_crown_sky;
    }
// sf:end
// sf:LEAF_SHADE+
    // (`--leaf-shade`) the crown this pixel stands in, for the lights without a map (`sky_leaf_t`)
    if (inst.model == PR_LS_PALM && (in.uv.x >= 0.5 || in.local.y > PR_LS_PALM_HEART - 2.0)) {
        // (the crown, and the trunk's top under it: the atlas's u < 0.5 is the bark)
        let sc = max(inst.scale.y, 1e-3);
        sky_leaf = vec4<f32>(pr_world(inst, vec3<f32>(0.0, PR_LS_PALM_HEART, 0.0), 0.0), PR_LS_PALM_R * sc);
        sky_leaf_k = vec2<f32>(PR_LS_PALM_K.x, PR_LS_PALM_K.y * sc);
    } else if (inst.model == PR_LS_TREE && (m.flags & PR_LEAF) != 0u) {
        // the tree's height in its own metres from this leaf's (its wind weight 0.15 + 0.85 f^1.5)
        let f = max(pow(clamp((in.sway - 0.15) / 0.85, 0.0, 1.0), 0.6667), 0.3);
        let h = max(in.local.y, 0.5) / f;
        sky_leaf = vec4<f32>(pr_world(inst, vec3<f32>(0.0, 0.64 * h, 0.0), 0.0), 0.38 * h * max(inst.scale.y, 1e-3));
        sky_leaf_k = vec2<f32>(PR_LS_TREE_SIGMA, 0.0);
    }
// sf:end
// sf:FOLIAGE&WET_FOLIAGE+
    // (`--wet-foliage`) a palm's trunk and a street tree's bark in the rain
    if (sky_on(skyu.flags.x, SKY_WET) && skyu.wet.x > 0.02) {
        if (inst.model == PR_WET_PALM && in.uv.x < 0.5) {
            let run = smoothstep(0.45, 0.75, st_noise(vec2<f32>(in.uv.x * 44.0, in.local.y * 0.35)));
            pr_wet_bark(&s, skyu.wet.x, run);
        } else if (inst.model == PR_WET_TREE && (m.flags & PR_LEAF) == 0u) {
            let run = smoothstep(0.45, 0.75, st_noise(vec2<f32>((in.local.x + in.local.z) * 9.0, in.local.y * 0.8)));
            pr_wet_bark(&s, skyu.wet.x, run);
        }
    }
// sf:end
// sf:FOLIAGE+
    // (`--foliage-light`) a leaf card, or a palm's frond (the palm's one
    // material: its green parts; with `--palm-detail` its leaflets by the atlas).
    var leaf = select(0.0, 1.0, (m.flags & PR_LEAF) != 0u);
    if (inst.model == PR_PALM) {
        leaf = select(smoothstep(1.0, 1.3, s.albedo.g / max(s.albedo.r, 1e-4)), palm_leaf, palm_leaf >= 0.0);
    }
    if (leaf > 0.0) {
        // In the rain a leaf's waxy cuticle carries water on both faces,
        // whichever way it faces (the ground's wetting takes only what faces
        // up): beaded drops, not a mirror film (a little glossier at the
        // pixel's scale), and the leaf darker and deeper in tone.
// sf:end
// sf:FOLIAGE&!WET_FOLIAGE+
        let wet = skyu.wet.x * leaf;
        if (sky_on(skyu.flags.x, SKY_WET) && wet > 0.02) {
            s.rough = mix(s.rough, min(s.rough, PR_LEAF_WET_ROUGH), wet);
            s.albedo = s.albedo * mix(1.0, 0.85, wet);
        }
// sf:end
// sf:FOLIAGE&WET_FOLIAGE+
        if (sky_on(skyu.flags.x, SKY_WET) && skyu.wet.x > 0.02) {
            // (`--wet-foliage`) each leaflet (a palm's: its atlas v) or each 25 cm of a leaf card its own water
            let cell = select(fract(sin(dot(floor(in.world * 4.0), vec3<f32>(12.9898, 78.233, 37.719))) * 43758.5453), fract(sin(in.uv.y * 31415.0) * 43758.5453), inst.model == PR_WET_PALM);
            pr_wet_leaf(&s, skyu.wet.x, cell, 1.0 - smoothstep(0.35, 0.9, leaf), max(length(dpx), length(dpy)));
        }
// sf:end
// sf:FOLIAGE+
        // a seen leaf faces the eye (see PR_LEAF_NV_MIN)
        let lnv = dot(s.n, v);
        if (lnv < PR_LEAF_NV_MIN) {
            s.n = normalize(s.n + v * (PR_LEAF_NV_MIN - lnv));
        }
        // it passes the light on its far face (in the emissive slot: look_light
        // adds it before its fog); the sun's shadow there only when the sun is behind it.
        var leaf_sh = 0.0;
        if (dot(-n, look.key_dir.xyz) > 0.0) {
            leaf_sh = lk_shadow(in.world, -n);
        }
        // (the light through the leaf rides on the emission, which look_light takes at the
        // practicals' gain with `--night-photometry`: it is light, not a practical, so it
        // is carried over that gain, as the lenses' radiance is above)
        let leaf_g = leaf / lk_practical_gain();
        m.emissive = vec4<f32>(m.emissive.rgb + pr_foliage(s.albedo, n, v, in.world, leaf_sh) * leaf_g, m.emissive.w);
        // The street's lamps light the leaves from every side: what falls on
        // the far face passes through as the sun's does (T = R x T/R), so a
        // crown over a lamp glows a warm green from below. The same lights'
        // loop on the far face (-n) with the leaf's transmittance as its
        // diffuse; no specular there (f0 0, the eye on that face's normal).
        if (skyu.lights.x != 0u) {
            let tl = s.albedo * PR_LEAF_T_OVER_R;
            m.emissive = vec4<f32>(m.emissive.rgb + sky_local_lights_ao(in.world, -n, -n, tl, vec3<f32>(0.0), 1.0, 1.0) * leaf_g, m.emissive.w);
        }
    }
// sf:end
    var c = look_light(s, m, v, in.world, n_light, 0u);
// sf:FOLIAGE-
    if ((m.flags & PR_LEAF) != 0u && (pr.mode.z & PR_F_LEAF) != 0u) {
        // back-lit leaves. A thin leaf passes part of the sun it does not
        // reflect: a diffuse transmission through the card (the sun behind the
        // leaf as seen from here) plus a forward-scattered lobe toward the sun,
        // both shadowed like the key light and fogged like the rest.
        let l = look.key_dir.xyz;
        let through = max(dot(-n, l), 0.0);
        let lobe = pow(max(dot(-v, l), 0.0), 6.0);
        let sh = lk_shadow(in.world, n) * select(1.0, sky_cloud_transmittance(in.world), SF_VOL_CLOUDS && sky_vol_clouds_on(skyu.cirrus));
        let vd = dot(in.world - look.cam_pos.xyz, look.cam_fwd.xyz);
        let fog = lk_smooth(look.fog.x, look.fog.y, vd);
        let trans = look.key.xyz * s.albedo * (through * 0.35 + lobe * 0.6) * sh / LK_PI;
        c = vec4<f32>(c.rgb + trans * (1.0 - fog), c.a);
    }
// sf:end
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
    let dlx = dpdx(in.local);
    let dly = dpdy(in.local);
    let inst = pr_instances[in.inst];
    var a = 1.0;
    if (inst.model == PR_TREE_BASIN) {
        // a tree basin: the bed, and past its cut edge the spilled crumbs and leaves only.
        if (!pr_basin_cut(inst, in.local, dlx, dly)) {
            discard;
        }
        return;
    }
    if ((m.flags & PR_PLATE) != 0u) {
        let tile = pr_face_tile(inst.variant);
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
