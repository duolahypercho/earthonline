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
    var nw = pr_rot(inst.yaw, pr_local_dir(inst, normal.xyz / max(inst.scale, vec3<f32>(1e-3))));
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
