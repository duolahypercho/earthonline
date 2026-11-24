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
const PR_LEAF_T_OVER_R: vec3<f32> = vec3<f32>(0.55, 0.85, 0.3);
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
    return t * (e + irr * ao) / LK_PI;
}

// sf:end
// sf:FOLIAGE|PALM+
const PR_PALM: u32 = 7u; // eo_props::Model::Palm
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
    // (`--night-photometry`) a lens's radiance is solved in display terms (x 1 / exposure):
    // look_light's practicals' gain does not apply to it (it lit the dark aspects).
    m.emissive = vec4<f32>(m.emissive.rgb / lk_practical_gain(), 0.0);
// sf:FOLIAGE|PALM+
    var palm_leaf = -1.0;
// sf:end
// sf:PALM+
    if (inst.model == PR_PALM) {
        palm_leaf = pr_palm(&s, inst, in.local, in.uv, n, v, dpx, dpy, duvx, duvy, dlx, dly); // (`--palm-detail`)
    }
// sf:end
    s.rough = clamp(s.rough, 0.03, 1.0);
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
        let wet = skyu.wet.x * leaf;
        if (sky_on(skyu.flags.x, SKY_WET) && wet > 0.02) {
            s.rough = mix(s.rough, min(s.rough, PR_LEAF_WET_ROUGH), wet);
            s.albedo = s.albedo * mix(1.0, 0.85, wet);
        }
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
    var c = look_light(s, m, v, in.world, n, 0u);
// sf:FOLIAGE-
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
// sf:end
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
