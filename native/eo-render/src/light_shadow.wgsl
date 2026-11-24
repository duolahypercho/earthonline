// lamp and headlight shadows (lighting, `--light-shadows`,
// `--headlight-shadows`; light_shadow.rs): the compute side, appended to
// sky_common.wgsl, sky.rs sea_wgsl() and light_shadow.rs's constants. After
// the sky's light gather (sky_lights.wgsl) and before the depth passes into
// the atlas (light_shadow_draw.wgsl):
//
//   ls_rank        the frame's lights ranked nearest first (ties by position,
//                  kind, intensity, slot): `ls_order`;
//   ls_cluster     every light cluster's list rebuilt in that order (the
//                  gather's appends shuffled it): its 32 are its nearest;
//   ls_vehicles    the vehicles with their low beams in the frame list;
//   ls_select      (one invocation) the lamps that keep or take an atlas
//                  tile, the moving lamps, the headlight vehicles, the
//                  table's rows, the tiles drawn this frame and their planes;
//   ls_pads        each frame light's spare word: its tile + 1, or -1;
//   ls_cull_*      the casters against the tiles drawn this frame;
//   ls_scan        the caster lists' triangle sums and draw arguments.

struct LsLight {
    pos: vec3<f32>,
    intensity: f32,
    color: vec3<f32>,
    range: f32,
    dir: vec3<f32>,
    cos_outer: f32,
    cos_inner: f32,
    kind: u32,
    size: f32,
    pad: f32,
};

// sky.rs LtParams (sky_lights.wgsl): only counts, scale are read here.
struct LsLt {
    counts: vec4<u32>,
    scale: vec4<f32>,
    props: vec4<f32>,
    water: vec4<f32>,
    water_n: vec4<u32>,
    led: vec4<f32>,
    led_key: vec4<f32>,
    shops: vec4<f32>,
};

struct LsParams {
    // x city objects, y prop slots in use, z agents, w props present (1) | agents present (2)
    counts: vec4<u32>,
    // x lamp tiles, y moving tiles, z headlight tiles, w columns
    tiles: vec4<u32>,
    // columns, rows, tile texels, tiles
    grid: vec4<f32>,
    // x tan of the lamp's half angle, y lamp near, z headlight near, w 1: no light reads its map (diagnostic)
    lamp: vec4<f32>,
    // x tan of the headlights' half width, y tan up, z tan down
    head: vec4<f32>,
};

struct LsFrame {
    // x lamp tiles drawn, y headlight tiles drawn, z new lamp maps, w lamps selected
    head: vec4<u32>,
    // x new lamp maps since the start, y frames, z vehicles chosen, w vehicle candidates
    totals: vec4<u32>,
    // the tiles drawn this frame (the lamps' from 0, the headlights' from LS_LAMP_REDRAWS):
    // x tile, y mode (0 a lamp's map, 1 a moving lamp's, 2 a headlight's), z the vehicle (2)
    redraw: array<vec4<u32>, LS_REDRAWS>,
    // their frustums' planes (world, inside >= 0), six a tile
    planes: array<vec4<f32>, LS_PLANES>,
    // the lamps selected this frame: xyz their luminaire's position bits, w the tile shading reads
    sel: array<vec4<u32>, LS_MAX_LAMPS>,
    // the chosen vehicles' headlamps: xyz, w their tile
    heads: array<vec4<f32>, LS_HEAD_LAMPS>,
};

struct LsRec {
    tile: u32,
    first: u32,
    tris: u32,
    base: i32,
    // city: the object; props: the instance; agents: agent | lod << 28
    payload: u32,
    // props: 1 the alpha-tested part
    flags: u32,
    // the triangles of the records before it (ls_scan)
    start: u32,
    pad: u32,
};

// the city's object (scene.rs GpuObject) and LOD group (look_shadow_cull.wgsl's copies).
struct LsObject {
    min: vec3<f32>,
    index_count: u32,
    max: vec3<f32>,
    first_index: u32,
    base_vertex: i32,
    kind: u32,
    copy: u32,
    material: u32,
    lod_group: u32,
    lod_role: u32,
    far_first_index: u32,
    far_index_count: u32,
};

struct LsGroup {
    center: vec3<f32>,
    radius: f32,
    ready: u32,
    detailed: u32,
    kind: u32,
    level: u32,
};

// props.rs GpuPropInstance, props_lib.rs GpuPropModel, GpuPropMesh.
struct LsPrInstance {
    pos: vec3<f32>,
    yaw: f32,
    scale: vec3<f32>,
    model: u32,
    variant: u32,
    lamp: u32,
    slope: f32,
    pad: u32,
};

struct LsPrModel {
    meshes: vec4<u32>,
    lod: vec4<f32>,
    center: vec3<f32>,
    radius: f32,
    casts: vec4<u32>,
};

struct LsPrMesh {
    first: u32,
    count: u32,
    cut_first: u32,
    cut_count: u32,
    base_vertex: i32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
};

// eo-agents GpuAgent, GpuModel, GpuMesh.
struct LsAgAgent {
    pos: vec3<f32>,
    model: u32,
    rot: vec4<f32>,
    color: u32,
    flags: u32,
    anim: f32,
    steer: f32,
    speed: f32,
    scale: f32,
    clip_phase: f32,
    clip: u32,
};

struct LsAgModel {
    meshes: vec4<u32>,
    wheel: array<vec4<f32>, 4>,
    center: vec3<f32>,
    radius: f32,
    model_class: u32,
    skeleton: u32,
    joints: u32,
    wheel_radius: f32,
    pad: vec4<u32>,
};

struct LsAgMesh {
    first_index: u32,
    index_count: u32,
    glass_first: u32,
    glass_count: u32,
    base_vertex: i32,
    skeleton: u32,
    joints: u32,
    pad: u32,
};

@group(0) @binding(0) var<uniform> skyu: Sky;
@group(0) @binding(1) var<uniform> lt: LsLt;
@group(0) @binding(2) var<uniform> ls_p: LsParams;
@group(0) @binding(3) var<storage, read_write> ls_lights: array<LsLight>;
@group(0) @binding(4) var<storage, read> ls_counter: array<u32>;
@group(0) @binding(5) var<storage, read_write> ls_clusters: array<u32>;
@group(0) @binding(6) var<storage, read_write> ls_order: array<u32>;
// the lamp tiles' keys (persistent): xyz the luminaire's position bits, w 1 while a map is held
@group(0) @binding(7) var<storage, read_write> ls_state: array<vec4<u32>>;
@group(0) @binding(8) var<storage, read_write> ls_frame: LsFrame;
@group(0) @binding(9) var<storage, read_write> ls_table: array<vec4<f32>>;
@group(0) @binding(10) var<storage, read_write> ls_cand: array<vec2<u32>>;
// per list (LS_LIST_WORDS words): count, triangles, the draw's arguments at 4..8;
// then the candidates' count (at LS_CAND_WORD), the frame lights reading a map (lamps, headlamps).
@group(0) @binding(11) var<storage, read_write> ls_lists: array<atomic<u32>>;
@group(0) @binding(12) var<storage, read_write> ls_recs: array<LsRec>;
@group(0) @binding(13) var<storage, read> ls_objects: array<LsObject>;
@group(0) @binding(14) var<storage, read> ls_groups: array<LsGroup>;
@group(0) @binding(15) var<storage, read> ls_pr_instances: array<LsPrInstance>;
@group(0) @binding(16) var<storage, read> ls_pr_models: array<LsPrModel>;
@group(0) @binding(17) var<storage, read> ls_pr_meshes: array<LsPrMesh>;
@group(0) @binding(18) var<storage, read> ls_agents: array<LsAgAgent>;
@group(0) @binding(19) var<storage, read> ls_ag_models: array<LsAgModel>;
@group(0) @binding(20) var<storage, read> ls_ag_meshes: array<LsAgMesh>;
// the culls' dispatch arguments (city, props, agents: four words each), written by ls_select
// alone (a buffer the dispatches read as their arguments is bound to no other pass of theirs).
@group(1) @binding(0) var<storage, read_write> ls_args: array<u32>;

const LS_NONE: u32 = 0xffffffffu;
const LS_PERSON: u32 = 1u;
const LS_MODE_LAMP: u32 = 0u;
const LS_MODE_MOVING: u32 = 1u;
const LS_MODE_HEAD: u32 = 2u;

// ---- the frame list in a stable order ----

/// A light in the camera's view space (sky_lights.wgsl `lt_view`).
fn ls_view(p: vec3<f32>) -> vec3<f32> {
    let r = p - skyu.eye.xyz;
    return vec3<f32>(dot(r, skyu.cam_right.xyz), dot(r, skyu.cam_up.xyz), dot(r, skyu.cam_fwd.xyz));
}

/// The rank key: the squared distance to the eye, then the position's bits.
/// (`--shop-light`) A shop window's light ranks after every other light
/// (its distance pushed past any in the clusters' reach), nearest first
/// among them: a cluster keeps its street lamps and headlamps first.
/// (`--lamp-bounce`) A pool's bounce ranks after the shop windows.
fn ls_key(l: LsLight) -> vec4<u32> {
    let d = l.pos - skyu.eye.xyz;
    var d2 = select(dot(d, d), dot(d, d) + LS_SHOP_RANK_M2, l.kind == SHOP_KIND);
    if (LT_LAMP_BOUNCE && l.kind == LT_BOUNCE_KIND) {
        d2 = dot(d, d) + 2.0 * LS_SHOP_RANK_M2;
    }
    return vec4<u32>(bitcast<u32>(d2), bitcast<u32>(l.pos.x), bitcast<u32>(l.pos.y), bitcast<u32>(l.pos.z));
}

// past the square of any distance in the clusters' 300 m reach.
const LS_SHOP_RANK_M2: f32 = 1.0e7;

/// (a, its kind and intensity bits, its slot) before b, lexicographically.
fn ls_less(a: vec4<u32>, am: vec2<u32>, ai: u32, b: vec4<u32>, bm: vec2<u32>, bi: u32) -> bool {
    for (var k = 0u; k < 4u; k = k + 1u) {
        if (a[k] != b[k]) {
            return a[k] < b[k];
        }
    }
    if (am.x != bm.x) {
        return am.x < bm.x;
    }
    if (am.y != bm.y) {
        return am.y < bm.y;
    }
    return ai < bi;
}

var<workgroup> lsw_n: u32;
var<workgroup> lsr_keys: array<vec4<u32>, 64>;
var<workgroup> lsr_more: array<vec2<u32>, 64>;

/// The frame's light count (the gather's, at most the frame list's cap), the same for the whole workgroup.
fn ls_frame_count(lid: u32) -> u32 {
    if (lid == 0u) {
        lsw_n = min(ls_counter[0], LS_FRAME_CAP);
    }
    return workgroupUniformLoad(&lsw_n);
}

@compute @workgroup_size(64)
fn ls_rank(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(local_invocation_index) lid: u32) {
    let n = ls_frame_count(lid);
    let i = gid.x;
    var ka = vec4<u32>(0u);
    var ma = vec2<u32>(0u);
    if (i < n) {
        let a = ls_lights[i];
        ka = ls_key(a);
        ma = vec2<u32>(a.kind, bitcast<u32>(a.intensity));
    }
    var rank = 0u;
    for (var base = 0u; base < n; base = base + 64u) {
        let j = base + lid;
        if (j < n) {
            let b = ls_lights[j];
            lsr_keys[lid] = ls_key(b);
            lsr_more[lid] = vec2<u32>(b.kind, bitcast<u32>(b.intensity));
        }
        workgroupBarrier();
        if (i < n) {
            let m = min(64u, n - base);
            for (var k = 0u; k < m; k = k + 1u) {
                if (ls_less(lsr_keys[k], lsr_more[k], base + k, ka, ma, i)) {
                    rank = rank + 1u;
                }
            }
        }
        workgroupBarrier();
    }
    if (i < n) {
        ls_order[rank] = i;
    }
}

var<workgroup> lsc_hits: array<u32, 64>;

/// sky_lights.wgsl `lights_cluster` with the lights taken in rank order and
/// each cluster's list written in that order (a prefix sum a 64-light chunk).
@compute @workgroup_size(64)
fn ls_cluster(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) lid: u32) {
    let n = ls_frame_count(lid);
    let g = skyu.lights.yzw;
    let c = wid.x;
    let tx = c % g.x;
    let ty = (c / g.x) % g.y;
    let tz = c / (g.x * g.y);
    let near = skyu.cluster.x;
    let far = skyu.cluster.y;
    let z0 = near * pow(far / near, f32(tz) / f32(g.z));
    let z1 = near * pow(far / near, f32(tz + 1u) / f32(g.z));
    let x0 = -1.0 + 2.0 * f32(tx) / f32(g.x);
    let x1 = -1.0 + 2.0 * f32(tx + 1u) / f32(g.x);
    let y1 = 1.0 - 2.0 * f32(ty) / f32(g.y);
    let y0 = 1.0 - 2.0 * f32(ty + 1u) / f32(g.y);
    let sx = skyu.cluster.z * skyu.cluster.w;
    let sy = skyu.cluster.z;
    let lo = vec3<f32>(min(x0 * z0, x0 * z1) * sx, min(y0 * z0, y0 * z1) * sy, z0);
    let hi = vec3<f32>(max(x1 * z0, x1 * z1) * sx, max(y1 * z0, y1 * z1) * sy, z1);
    let total = g.x * g.y * g.z;
    var count = 0u;
    for (var base = 0u; base < n; base = base + 64u) {
        let k = base + lid;
        var hit = 0u;
        var idx = 0u;
        if (k < n) {
            idx = ls_order[k];
            let l = ls_lights[idx];
            let v = ls_view(l.pos);
            let dd = v - clamp(v, lo, hi);
            hit = select(0u, 1u, dot(dd, dd) <= l.range * l.range);
        }
        lsc_hits[lid] = hit;
        workgroupBarrier();
        for (var off = 1u; off < 64u; off = off * 2u) {
            var s = lsc_hits[lid];
            if (lid >= off) {
                s = s + lsc_hits[lid - off];
            }
            workgroupBarrier();
            lsc_hits[lid] = s;
            workgroupBarrier();
        }
        let slot = count + lsc_hits[lid] - 1u;
        if (hit == 1u && slot < LS_CLUSTER_CAP) {
            ls_clusters[total + c * LS_CLUSTER_CAP + slot] = idx;
        }
        count = count + lsc_hits[63];
        workgroupBarrier();
    }
    if (lid == 0u) {
        ls_clusters[c] = min(count, LS_CLUSTER_CAP);
    }
}

// ---- the lamps and vehicles that get a map ----

/// A street lamp: the fallback's (kind 0) or a props district light's (kind 3), lit.
fn ls_is_lamp(l: LsLight) -> bool {
    return (l.kind == 0u || l.kind == 3u) && l.intensity > 0.0;
}

/// Where a lamp's light leaves its luminaire: a props light's source stands `size` over its head.
fn ls_head_of(l: LsLight) -> vec3<f32> {
    return select(l.pos, l.pos - vec3<f32>(0.0, l.size, 0.0), l.kind == 3u && l.size > 0.0);
}

/// How far a lamp's light reaches below its luminaire.
fn ls_reach_of(l: LsLight) -> f32 {
    return select(l.range, l.range + l.size, l.kind == 3u);
}

fn ls_rotate(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    let t = 2.0 * cross(q.xyz, v);
    return v + q.w * t + cross(q.xyz, t);
}

/// A vehicle's two low beams as the gather places them (sky_lights.wgsl
/// `lights_agents`), and their axis: xyz, and the beam axis in [2].
fn ls_headlamps(a: LsAgAgent, m: LsAgModel) -> array<vec3<f32>, 3> {
    let fl = m.wheel[0].xyz;
    var lat = abs(fl.x) * 0.78;
    var up = fl.y + 0.28;
    var fwd = fl.z + 0.8;
    if (lat < 0.2) {
        lat = 0.62;
        up = 0.7;
        fwd = m.center.z + m.radius * 0.85;
    }
    var out: array<vec3<f32>, 3>;
    out[0] = a.pos + ls_rotate(a.rot, vec3<f32>(lat, up, fwd) * a.scale);
    out[1] = a.pos + ls_rotate(a.rot, vec3<f32>(-lat, up, fwd) * a.scale);
    out[2] = normalize(ls_rotate(a.rot, vec3<f32>(0.0, -0.0819, 1.0)));
    return out;
}

/// sky_lights.wgsl `lt_visible`.
fn ls_in_clusters(p: vec3<f32>, r: f32) -> bool {
    let v = ls_view(p);
    if (v.z < -r || v.z - r > skyu.cluster.y) {
        return false;
    }
    let z = max(v.z, 0.1);
    let hx = z * skyu.cluster.z * skyu.cluster.w + r * 1.5;
    let hy = z * skyu.cluster.z + r * 1.5;
    return abs(v.x) <= hx && abs(v.y) <= hy;
}

/// The vehicles whose low beams the gather put in the frame list (lights_agents' tests).
@compute @workgroup_size(64)
fn ls_vehicles(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (ls_p.tiles.z == 0u || (ls_p.counts.w & 2u) == 0u || lt.counts.w == 0u || i >= min(lt.counts.y, ls_p.counts.z) || lt.scale.y <= 0.0) {
        return;
    }
    let a = ls_agents[i];
    if (a.model >= arrayLength(&ls_ag_models)) {
        return;
    }
    let m = ls_ag_models[a.model];
    if (m.model_class == LS_PERSON || (a.flags == 2u && a.speed == 0.0 && a.anim == 0.0)) {
        return;
    }
    let d = distance(a.pos, skyu.eye.xyz);
    if (d > lt.scale.w + 20.0) {
        return;
    }
    let h = ls_headlamps(a, m);
    let r = lt.scale.w;
    if (!ls_in_clusters(h[0] + h[2] * r * 0.5, r * 0.6) && !ls_in_clusters(h[1] + h[2] * r * 0.5, r * 0.6)) {
        return;
    }
    let k = atomicAdd(&ls_lists[LS_CAND_WORD], 1u);
    if (k < LS_CAND_CAP) {
        ls_cand[k] = vec2<u32>(bitcast<u32>(d * d), i);
    }
}

/// Five table rows for tile `t`: world -> view (three rows), the projection, (near, far, valid, texel per metre).
fn ls_write_tile(t: u32, r0: vec4<f32>, r1: vec4<f32>, r2: vec4<f32>, pp: vec4<f32>, dd: vec4<f32>) {
    let b = 1u + t * 5u;
    ls_table[b] = r0;
    ls_table[b + 1u] = r1;
    ls_table[b + 2u] = r2;
    ls_table[b + 3u] = pp;
    ls_table[b + 4u] = dd;
}

fn ls_invalidate(t: u32) {
    ls_table[1u + t * 5u + 4u] = vec4<f32>(0.0);
}

/// A lamp's map: straight down from its luminaire (view x = world x, y = world z, along = down).
fn ls_lamp_tile(t: u32, head: vec3<f32>, reach: f32) {
    let tn = ls_p.lamp.x;
    ls_write_tile(t, vec4<f32>(1.0, 0.0, 0.0, -head.x), vec4<f32>(0.0, 0.0, 1.0, -head.z), vec4<f32>(0.0, -1.0, 0.0, head.y), vec4<f32>(1.0 / tn, 1.0 / tn, 0.0, 0.0), vec4<f32>(ls_p.lamp.y, max(reach, ls_p.lamp.y + 1.0), 1.0, 2.0 * tn / LS_TILE_PX));
}

/// The six planes (world, inside >= 0) of tile `t`'s frustum, into redraw slot `r`.
fn ls_planes(r: u32, t: u32) {
    let b = 1u + t * 5u;
    let r0 = ls_table[b];
    let r1 = ls_table[b + 1u];
    let r2 = ls_table[b + 2u];
    let pp = ls_table[b + 3u];
    let dd = ls_table[b + 4u];
    // in view space: near, far, ndc x >= -1, x <= 1, y >= -1, y <= 1
    var v = array<vec4<f32>, 6>(
        vec4<f32>(0.0, 0.0, 1.0, -dd.x),
        vec4<f32>(0.0, 0.0, -1.0, dd.y),
        vec4<f32>(pp.x, 0.0, pp.z + 1.0, 0.0),
        vec4<f32>(-pp.x, 0.0, 1.0 - pp.z, 0.0),
        vec4<f32>(0.0, pp.y, pp.w + 1.0, 0.0),
        vec4<f32>(0.0, -pp.y, 1.0 - pp.w, 0.0),
    );
    for (var k = 0u; k < 6u; k = k + 1u) {
        let q = v[k];
        let n = q.x * r0.xyz + q.y * r1.xyz + q.z * r2.xyz;
        ls_frame.planes[r * 6u + k] = vec4<f32>(n, q.x * r0.w + q.y * r1.w + q.z * r2.w + q.w);
    }
}

fn ls_pair_less(a: vec2<u32>, b: vec2<u32>) -> bool {
    return a.x < b.x || (a.x == b.x && a.y < b.y);
}

@compute @workgroup_size(1)
fn ls_select() {
    let L = ls_p.tiles.x;
    let M = ls_p.tiles.y;
    let N = ls_p.tiles.z;
    let n = min(ls_counter[0], LS_FRAME_CAP);
    ls_table[0] = ls_p.grid;
    // 1. the nearest lamps (rank order), one a luminaire.
    var head: array<vec3<f32>, LS_MAX_LAMPS>;
    var reach: array<f32, LS_MAX_LAMPS>;
    var tile: array<u32, LS_MAX_LAMPS>;
    var sel = 0u;
    for (var k = 0u; k < n && sel < L; k = k + 1u) {
        let l = ls_lights[ls_order[k]];
        if (!ls_is_lamp(l)) {
            continue;
        }
        let h = ls_head_of(l);
        let key = bitcast<vec3<u32>>(h);
        var dup = false;
        for (var s = 0u; s < sel; s = s + 1u) {
            if (all(ls_frame.sel[s].xyz == key)) {
                dup = true;
                break;
            }
        }
        if (dup) {
            continue;
        }
        head[sel] = h;
        reach[sel] = ls_reach_of(l);
        tile[sel] = LS_NONE;
        ls_frame.sel[sel] = vec4<u32>(key, LS_NONE);
        sel = sel + 1u;
    }
    // 2. a lamp keeps the tile it held.
    var kept: array<u32, LS_MAX_LAMPS>;
    for (var t = 0u; t < L; t = t + 1u) {
        kept[t] = 0u;
    }
    for (var s = 0u; s < sel; s = s + 1u) {
        for (var t = 0u; t < L; t = t + 1u) {
            let st = ls_state[t];
            if (st.w == 1u && kept[t] == 0u && all(st.xyz == ls_frame.sel[s].xyz)) {
                tile[s] = t;
                kept[t] = 1u;
                break;
            }
        }
    }
    // 3. new maps, nearest first, at most LS_NEW_MAPS: a tile never used, else one no
    // selected lamp holds (the nearest lamps without a map wait for the next frames).
    var nr = 0u;
    var fresh = 0u;
    for (var s = 0u; s < sel && fresh < LS_NEW_MAPS; s = s + 1u) {
        if (tile[s] != LS_NONE) {
            continue;
        }
        var t = LS_NONE;
        for (var u = 0u; u < L; u = u + 1u) {
            if (kept[u] == 0u && ls_state[u].w == 0u) {
                t = u;
                break;
            }
        }
        if (t == LS_NONE) {
            for (var u = 0u; u < L; u = u + 1u) {
                if (kept[u] == 0u) {
                    t = u;
                    break;
                }
            }
        }
        if (t == LS_NONE) {
            break;
        }
        ls_state[t] = vec4<u32>(ls_frame.sel[s].xyz, 1u);
        kept[t] = 1u;
        tile[s] = t;
        ls_lamp_tile(t, head[s], reach[s]);
        ls_frame.redraw[nr] = vec4<u32>(t, LS_MODE_LAMP, 0u, 0u);
        ls_planes(nr, t);
        nr = nr + 1u;
        fresh = fresh + 1u;
    }
    // the lamp tiles no selected lamp holds are not read this frame (they keep their map and key).
    for (var t = 0u; t < L; t = t + 1u) {
        let b = 1u + t * 5u + 4u;
        ls_table[b].z = select(0.0, 1.0, kept[t] == 1u);
    }
    // 4. the moving lamps (`--light-shadow-moving`): the nearest with a map, drawn again
    // every frame with every agent into a tile of their own, which shading reads.
    var mv = 0u;
    for (var s = 0u; s < sel; s = s + 1u) {
        var shade = tile[s];
        if (tile[s] != LS_NONE && mv < M) {
            let t = L + mv;
            ls_lamp_tile(t, head[s], reach[s]);
            ls_frame.redraw[nr] = vec4<u32>(t, LS_MODE_MOVING, 0u, 0u);
            ls_planes(nr, t);
            nr = nr + 1u;
            mv = mv + 1u;
            shade = t;
        }
        ls_frame.sel[s].w = shade;
    }
    for (var m = mv; m < M; m = m + 1u) {
        ls_invalidate(L + m);
    }
    // 5. the headlights: the nearest vehicles with their beams in the frame list (by
    // distance, then agent), one map a vehicle over both beams.
    let nc = min(atomicLoad(&ls_lists[LS_CAND_WORD]), LS_CAND_CAP);
    var nh = 0u;
    var last = vec2<u32>(0u);
    for (var h = 0u; h < N; h = h + 1u) {
        var best = vec2<u32>(LS_NONE);
        var found = false;
        for (var c = 0u; c < nc; c = c + 1u) {
            let e = ls_cand[c];
            if ((h == 0u || ls_pair_less(last, e)) && (!found || ls_pair_less(e, best))) {
                best = e;
                found = true;
            }
        }
        if (!found) {
            break;
        }
        last = best;
        let i = best.y;
        let a = ls_agents[i];
        let m = ls_ag_models[a.model];
        let hl = ls_headlamps(a, m);
        let o = 0.5 * (hl[0] + hl[1]);
        let f = normalize(ls_rotate(a.rot, vec3<f32>(0.0, 0.0, 1.0)));
        let rt = normalize(ls_rotate(a.rot, vec3<f32>(1.0, 0.0, 0.0)));
        let up = normalize(ls_rotate(a.rot, vec3<f32>(0.0, 1.0, 0.0)));
        let t = L + M + h;
        let tw = ls_p.head.x;
        let yhi = ls_p.head.y;
        let ylo = -ls_p.head.z;
        ls_write_tile(
            t,
            vec4<f32>(rt, -dot(rt, o)),
            vec4<f32>(up, -dot(up, o)),
            vec4<f32>(f, -dot(f, o)),
            vec4<f32>(1.0 / tw, 2.0 / (yhi - ylo), 0.0, -(yhi + ylo) / (yhi - ylo)),
            vec4<f32>(ls_p.lamp.z, max(lt.scale.w, ls_p.lamp.z + 1.0), 1.0, 2.0 * tw / LS_TILE_PX),
        );
        let r = LS_LAMP_REDRAWS + h;
        ls_frame.redraw[r] = vec4<u32>(t, LS_MODE_HEAD, i, 0u);
        ls_planes(r, t);
        ls_frame.heads[2u * h] = vec4<f32>(hl[0], f32(t));
        ls_frame.heads[2u * h + 1u] = vec4<f32>(hl[1], f32(t));
        nh = nh + 1u;
    }
    for (var h = nh; h < LS_MAX_HEADS; h = h + 1u) {
        if (h < N) {
            ls_invalidate(L + M + h);
        }
        ls_frame.heads[2u * h] = vec4<f32>(0.0, 0.0, 0.0, -1.0);
        ls_frame.heads[2u * h + 1u] = vec4<f32>(0.0, 0.0, 0.0, -1.0);
    }
    // 6. what was done, and the culls' dispatches (none without a tile to draw).
    ls_frame.head = vec4<u32>(nr, nh, fresh, sel);
    let tot = ls_frame.totals;
    ls_frame.totals = vec4<u32>(tot.x + fresh, tot.y + 1u, nh, nc);
    let draw = nr + nh > 0u;
    let groups = vec3<u32>((ls_p.counts.x + 63u) / 64u, select(0u, (ls_p.counts.y + 63u) / 64u, (ls_p.counts.w & 1u) != 0u), select(0u, (ls_p.counts.z + 63u) / 64u, (ls_p.counts.w & 2u) != 0u));
    for (var k = 0u; k < 3u; k = k + 1u) {
        ls_args[4u * k] = select(0u, groups[k], draw);
        ls_args[4u * k + 1u] = 1u;
        ls_args[4u * k + 2u] = 1u;
    }
}

/// Each frame light's spare word: a selected lamp's tile + 1, a chosen vehicle's beams' tile + 1,
/// else -1 (no map; the surfaces' occlusion still multiplies it).
@compute @workgroup_size(64)
fn ls_pads(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = min(ls_counter[0], LS_FRAME_CAP);
    let i = gid.x;
    if (i >= n) {
        return;
    }
    let l = ls_lights[i];
    var pad = -1.0;
    if (ls_is_lamp(l)) {
        let key = bitcast<vec3<u32>>(ls_head_of(l));
        let sel = ls_frame.head.w;
        for (var s = 0u; s < sel; s = s + 1u) {
            let e = ls_frame.sel[s];
            if (all(e.xyz == key)) {
                if (e.w != LS_NONE) {
                    pad = f32(e.w + 1u);
                }
                break;
            }
        }
    } else if (l.kind == 1u) {
        for (var h = 0u; h < 2u * ls_frame.head.y; h = h + 1u) {
            let e = ls_frame.heads[h];
            if (e.w >= 0.0 && distance(e.xyz, l.pos) < 1e-3) {
                pad = e.w + 1.0;
                break;
            }
        }
    }
    if (ls_p.lamp.w > 0.5 && pad >= 1.0) {
        pad = -1.0;
    }
    if (pad >= 1.0) {
        // the frame's lights that read a map: lamps, headlamps (stats)
        atomicAdd(&ls_lists[LS_CAND_WORD + select(1u, 2u, l.kind == 1u)], 1u);
    }
    ls_lights[i].pad = pad;
}

// ---- the casters ----

fn ls_box_in(r: u32, mn: vec3<f32>, mx: vec3<f32>) -> bool {
    for (var k = 0u; k < 6u; k = k + 1u) {
        let pl = ls_frame.planes[r * 6u + k];
        if (dot(pl.xyz, select(mn, mx, pl.xyz >= vec3<f32>(0.0))) + pl.w < 0.0) {
            return false;
        }
    }
    return true;
}

fn ls_sphere_in(r: u32, c: vec3<f32>, rad: f32) -> bool {
    for (var k = 0u; k < 6u; k = k + 1u) {
        let pl = ls_frame.planes[r * 6u + k];
        if (dot(pl.xyz, c) + pl.w < -rad * length(pl.xyz)) {
            return false;
        }
    }
    return true;
}

/// Whether redraw slot `r` is drawn this frame, and its list group (0 lamps, 1 headlights).
fn ls_redraw_used(r: u32) -> bool {
    if (r < LS_LAMP_REDRAWS) {
        return r < ls_frame.head.x;
    }
    return r - LS_LAMP_REDRAWS < ls_frame.head.y;
}

fn ls_append(list: u32, rec: LsRec) {
    let slot = atomicAdd(&ls_lists[list * LS_LIST_WORDS], 1u);
    if (slot < LS_REC_CAP) {
        ls_recs[list * LS_REC_CAP + slot] = rec;
    }
}

const LS_LOD_NEAR: u32 = 1u;
const LS_LOD_FAR: u32 = 2u;
const LS_LOD_TERRAIN: u32 = 3u;
const LS_LOD_KIT_NEAR: u32 = 4u;
const LS_LOD_KIT_BOX: u32 = 6u;
const LS_LOD_STREET_NEAR: u32 = 7u;
const LS_BLEND_BIT: u32 = 256u;

/// Whether a city object casts, at the level the camera's cull chose this
/// frame (look_shadow_cull.wgsl's rule for the nearest cascade): buildings,
/// models, kit buildings at their drawn rung, structures' concrete, the
/// landmarks' near draws; not the ground, the street, paint, glass.
fn ls_city_casts(o: LsObject) -> bool {
    let kind = o.kind & 255u;
    let base = (o.kind & LS_BLEND_BIT) == 0u && kind != 1u && kind != 2u && kind != 3u && kind != 6u && kind != 7u && kind != 11u && o.index_count >= 3u;
    let role = o.lod_role;
    if (role == LS_LOD_NEAR || role == LS_LOD_FAR) {
        let g = ls_groups[o.lod_group];
        let near = g.ready == 1u && g.detailed == 1u;
        return base && select(!near, near, role == LS_LOD_NEAR);
    }
    if (role >= LS_LOD_KIT_NEAR && role <= LS_LOD_KIT_BOX) {
        return base && ls_groups[o.lod_group].level == role - LS_LOD_KIT_NEAR;
    }
    if (role == LS_LOD_TERRAIN) {
        return false;
    }
    if (role >= LS_LOD_STREET_NEAR && role <= LS_LOD_STREET_NEAR + 2u) {
        return base && ls_groups[o.lod_group].level == role - LS_LOD_STREET_NEAR;
    }
    if (role == 10u || role == 11u) {
        return base && kind == 9u && role == 10u && ls_groups[o.lod_group].detailed == 1u;
    }
    return base && kind != 10u;
}

@compute @workgroup_size(64)
fn ls_cull_city(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= ls_p.counts.x) {
        return;
    }
    let o = ls_objects[i];
    if (!ls_city_casts(o)) {
        return;
    }
    for (var r = 0u; r < LS_REDRAWS; r = r + 1u) {
        if (!ls_redraw_used(r) || !ls_box_in(r, o.min, o.max)) {
            continue;
        }
        let list = select(3u, 0u, r < LS_LAMP_REDRAWS);
        ls_append(list, LsRec(ls_frame.redraw[r].x, o.first_index, o.index_count / 3u, o.base_vertex, i, 0u, 0u, 0u));
    }
}

fn ls_pr_rot(yaw: f32, v: vec3<f32>) -> vec3<f32> {
    let s = sin(yaw);
    let c = cos(yaw);
    return vec3<f32>(v.x * c + v.z * s, v.y, -v.x * s + v.z * c);
}

@compute @workgroup_size(64)
fn ls_cull_props(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= ls_p.counts.y || i >= arrayLength(&ls_pr_instances)) {
        return;
    }
    let inst = ls_pr_instances[i];
    if (inst.model >= arrayLength(&ls_pr_models)) {
        return;
    }
    let m = ls_pr_models[inst.model];
    if (m.casts.x == 0u) {
        return;
    }
    // the middle level (or the nearest one the model has).
    var lod = min(LS_PROP_LOD, 2u);
    loop {
        if (m.meshes[lod] != LS_NONE || lod == 0u) {
            break;
        }
        lod = lod - 1u;
    }
    let mesh = m.meshes[lod];
    if (mesh >= arrayLength(&ls_pr_meshes)) {
        return;
    }
    let pm = ls_pr_meshes[mesh];
    let c = inst.pos + ls_pr_rot(inst.yaw, m.center * inst.scale);
    let rad = m.radius * max(max(inst.scale.x, inst.scale.y), inst.scale.z);
    for (var r = 0u; r < LS_REDRAWS; r = r + 1u) {
        if (!ls_redraw_used(r) || !ls_sphere_in(r, c, rad)) {
            continue;
        }
        let list = select(4u, 1u, r < LS_LAMP_REDRAWS);
        let t = ls_frame.redraw[r].x;
        if (pm.count >= 3u) {
            ls_append(list, LsRec(t, pm.first, pm.count / 3u, pm.base_vertex, i, 0u, 0u, 0u));
        }
        if (pm.cut_count >= 3u) {
            ls_append(list, LsRec(t, pm.cut_first, pm.cut_count / 3u, pm.base_vertex, i, 1u, 0u, 0u));
        }
    }
}

@compute @workgroup_size(64)
fn ls_cull_agents(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= ls_p.counts.z || i >= arrayLength(&ls_agents)) {
        return;
    }
    let a = ls_agents[i];
    if (a.model >= arrayLength(&ls_ag_models)) {
        return;
    }
    let m = ls_ag_models[a.model];
    let lod = select(LS_VEHICLE_LOD, LS_PERSON_LOD, m.model_class == LS_PERSON);
    let mesh = m.meshes[lod];
    if (mesh >= arrayLength(&ls_ag_meshes)) {
        return;
    }
    let g = ls_ag_meshes[mesh];
    if (g.index_count < 3u) {
        return;
    }
    // a parked car (eo-app props: stopped, speed 0, wheels never turned) is part of the
    // street a lamp's map keeps; moving things only in the maps drawn every frame.
    let parked = a.flags == 2u && a.speed == 0.0 && a.anim == 0.0;
    let c = a.pos + ls_rotate(a.rot, m.center * a.scale);
    let rad = m.radius * a.scale;
    for (var r = 0u; r < LS_REDRAWS; r = r + 1u) {
        if (!ls_redraw_used(r)) {
            continue;
        }
        let rd = ls_frame.redraw[r];
        if ((rd.y == LS_MODE_LAMP && !parked) || (rd.y == LS_MODE_HEAD && rd.z == i) || !ls_sphere_in(r, c, rad)) {
            continue;
        }
        let list = select(5u, 2u, r < LS_LAMP_REDRAWS);
        ls_append(list, LsRec(rd.x, g.first_index, g.index_count / 3u, g.base_vertex, i | (lod << 28u), 0u, 0u, 0u));
    }
}

var<workgroup> lss_sums: array<u32, 256>;

/// Per caster list: each record's first triangle among the list's, the triangles, the draw.
@compute @workgroup_size(256)
fn ls_scan(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) lid: u32) {
    let l = wid.x;
    let n = min(atomicLoad(&ls_lists[l * LS_LIST_WORDS]), LS_REC_CAP);
    let per = (n + 255u) / 256u;
    let b0 = min(lid * per, n);
    let b1 = min(b0 + per, n);
    var s = 0u;
    for (var k = b0; k < b1; k = k + 1u) {
        s = s + ls_recs[l * LS_REC_CAP + k].tris;
    }
    lss_sums[lid] = s;
    workgroupBarrier();
    for (var off = 1u; off < 256u; off = off * 2u) {
        var v = lss_sums[lid];
        if (lid >= off) {
            v = v + lss_sums[lid - off];
        }
        workgroupBarrier();
        lss_sums[lid] = v;
        workgroupBarrier();
    }
    var run = lss_sums[lid] - s;
    for (var k = b0; k < b1; k = k + 1u) {
        let at = l * LS_REC_CAP + k;
        ls_recs[at].start = run;
        run = run + ls_recs[at].tris;
    }
    if (lid == 255u) {
        let total = lss_sums[255];
        let w = l * LS_LIST_WORDS;
        atomicStore(&ls_lists[w + 1u], total);
        atomicStore(&ls_lists[w + 4u], total * 3u);
        atomicStore(&ls_lists[w + 5u], 1u);
        atomicStore(&ls_lists[w + 6u], 0u);
        atomicStore(&ls_lists[w + 7u], 0u);
    }
}
