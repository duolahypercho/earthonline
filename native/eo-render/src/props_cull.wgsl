// GPU cull and level of detail of the static street props (appended
// to props_types.wgsl, and to occlusion.wgsl in the Hi-Z variant).
//
// Views: 0 the camera, 1..3 the sun cascades, 4..5 (props2) the camera's
// cross-fade pair: an instance inside the band before a level's edge is
// drawn twice, its leaving level in view 4 and its arriving level in view 5,
// with complementary screen-space dither (no pop); inside the band before its
// draw distance it fades out in view 4 alone. A frame runs this twice:
//   * before the city pass (`run S`): the cascades the look refreshes (and
//     the camera when occlusion is off this frame);
//   * inside the occlusion pass, after the pyramid is built (`run C`, the
//     Hi-Z variant): the camera, tested against this frame's pyramid
//     (`hzb_occluded`, the city's own conservative test), drawn by the late
//     city pass.
// Each run: classify (one invocation per instance slot: distance level,
// frustum / cascade box, Hi-Z, a slot in its (view, mesh) bucket), offsets
// (one workgroup: prefix sums, then (props2) one compacted command list per
// (view, part) with its count, for a count-driven multi-draw), scatter (the
// visible lists).

struct PrShadowViewPadded {
    view_proj: mat4x4<f32>,
    copy_step: vec4<f32>,
    pad: array<vec4<f32>, 11>,
};

@group(0) @binding(0) var<uniform> pr: PrParams;
@group(0) @binding(1) var<storage, read> pr_instances: array<PrInstance>;
@group(0) @binding(2) var<storage, read> pr_models: array<PrModel>;
@group(0) @binding(3) var<storage, read> pr_meshes: array<PrMesh>;
// One word per (instance, view): mesh << 20 | slot, or PR_NONE.
@group(0) @binding(4) var<storage, read_write> pr_records: array<u32>;
@group(0) @binding(5) var<storage, read_write> pr_visible: array<u32>;
// [0, VM) counts per (view, mesh); [VM, 2VM) offsets.
@group(0) @binding(6) var<storage, read_write> pr_counts: array<atomic<u32>>;
// VIEWS x 2 parts (opaque, alpha-tested) lists of M commands, compacted.
@group(0) @binding(7) var<storage, read_write> pr_cmds: array<u32>;
// Per view: instances drawn, triangles.
@group(0) @binding(8) var<storage, read_write> pr_stats: array<atomic<u32>, 16>;
@group(0) @binding(9) var<uniform> pr_cascades: array<PrShadowViewPadded, 3>;
// commands in each list (the multi-draw count).
@group(0) @binding(10) var<storage, read_write> pr_list_counts: array<u32>;

const PR_FADE_BAND: f32 = 0.12;

struct PrSphere {
    c: vec3<f32>,
    r: f32,
};

fn pr_sphere(i: PrInstance, m: PrModel) -> PrSphere {
    var s: PrSphere;
    let v = pr_veg_scale(i);
    s.c = i.pos + pr_rot(i.yaw, m.center * i.scale * v);
    // a tree's lean (`pr_tree_lean`) shears its points by their height: the centre with
    // them, and the radius by the shear's length (zero for everything else)
    let lean = pr_tree_lean(i);
    let cy = (m.center * i.scale * v).y;
    s.c = s.c + vec3<f32>(lean.x * cy, 0.0, lean.y * cy);
    s.r = m.radius * max(max(i.scale.x, i.scale.y), i.scale.z) * v * (1.0 + length(pr_veg_grad(i)) + length(lean));
    if (pr_afloat_on(i)) {
        s.r = s.r * 1.12; // a boat's roll and pitch turn it about its waterline (`pr_afloat`)
    }
    if ((pr.mode.z & PR_F_WEATHER_WIND) != 0u && (i.model == PR_TREE || i.model == PR_PALM || (i.model >= PR_TREE_BASE && i.model < PR_OH_BASE))) {
        s.r = s.r + pr.wind.z * 2.5 * max(i.scale.y, 1.0);
    }
    return s;
}

fn pr_in_camera(s: PrSphere) -> bool {
    for (var p = 0u; p < 6u; p = p + 1u) {
        let pl = pr.planes[p];
        if (dot(pl.xyz, s.c) + pl.w < -s.r) {
            return false;
        }
    }
    return true;
}

fn pr_in_cascade(c: u32, s: PrSphere) -> bool {
    let m = pr_cascades[c].view_proj;
    let r0 = vec4<f32>(m[0].x, m[1].x, m[2].x, m[3].x);
    let r1 = vec4<f32>(m[0].y, m[1].y, m[2].y, m[3].y);
    let r2 = vec4<f32>(m[0].z, m[1].z, m[2].z, m[3].z);
    let r3 = vec4<f32>(m[0].w, m[1].w, m[2].w, m[3].w);
    var planes = array<vec4<f32>, 6>(r3 + r0, r3 - r0, r3 + r1, r3 - r1, r2, r3 - r2);
    for (var p = 0u; p < 6u; p = p + 1u) {
        let pl = planes[p];
        let l = length(pl.xyz);
        if (l > 1e-12 && dot(pl.xyz, s.c) + pl.w < -s.r * l) {
            return false;
        }
    }
    return true;
}

fn pr_mesh_at(m: PrModel, lod: u32) -> u32 {
    var l = min(lod, 2u);
    loop {
        if (m.meshes[l] != PR_NONE || l == 0u) {
            break;
        }
        l = l - 1u;
    }
    return m.meshes[l];
}

fn pr_take(view: u32, mesh: u32) -> u32 {
    let slot = atomicAdd(&pr_counts[view * pr.counts.y + mesh], 1u);
    return (mesh << 20u) | slot;
}

fn pr_lod_of(m: PrModel, d: f32) -> u32 {
    if (d < m.lod.x) {
        return 0u;
    }
    if (d < m.lod.y) {
        return 1u;
    }
    return 2u;
}

// the cross-fade of an instance at distance d: x the fade (0 none,
// 0..1 inside the band), y the arriving level's mesh (PR_NONE: fading out).
fn pr_fade(m: PrModel, lod: u32, d: f32) -> vec2<u32> {
    if ((pr.mode.z & PR_F_FADE) == 0u) {
        return vec2<u32>(0u, PR_NONE);
    }
    var e = m.lod.w;
    if (lod == 0u) {
        e = min(m.lod.x, m.lod.w);
    } else if (lod == 1u) {
        e = min(m.lod.y, m.lod.w);
    }
    let band = e * PR_FADE_BAND;
    if (d <= e - band) {
        return vec2<u32>(0u, PR_NONE);
    }
    let t = clamp((d - (e - band)) / band, 0.0, 1.0);
    var next = PR_NONE;
    if (e < m.lod.w) {
        next = pr_mesh_at(m, lod + 1u);
        if (next == pr_mesh_at(m, lod)) {
            return vec2<u32>(0u, PR_NONE);
        }
    }
    return vec2<u32>(max(u32(t * 255.0), 1u), next);
}

@compute @workgroup_size(64)
fn classify(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= pr.counts.x) {
        return;
    }
    for (var v = 0u; v < PR_VIEWS; v = v + 1u) {
        if ((pr.mode.x & (1u << v)) != 0u) {
            pr_records[i * PR_VIEWS + v] = PR_NONE;
        }
    }
    let inst = pr_instances[i];
    if (inst.model >= pr.counts.w) {
        return;
    }
    let m = pr_models[inst.model];
    if (pr_veg_scale(inst) <= 0.0) {
        return; // thinned out at this distance (vegetation)
    }
    let s = pr_sphere(inst, m);
    let d = max(distance(s.c, pr.cam.xyz) - s.r, 0.0);
    if (d >= m.lod.w) {
        return;
    }
    let dl = pr_lod_dist(inst, d);
    let lod = pr_lod_of(m, dl);
    if ((pr.mode.x & 1u) != 0u && pr_in_camera(s)) {
        let r = vec3<f32>(s.r);
        if (!pr_occluded(s.c - r, s.c + r)) {
            let mesh = pr_mesh_at(m, lod);
            if (mesh != PR_NONE) {
                let f = pr_fade(m, lod, dl);
                if (f.x == 0u) {
                    pr_records[i * PR_VIEWS] = pr_take(0u, mesh);
                } else {
                    pr_records[i * PR_VIEWS + 4u] = pr_take(4u, mesh);
                    if (f.y != PR_NONE) {
                        pr_records[i * PR_VIEWS + 5u] = pr_take(5u, f.y);
                    }
                }
            }
        }
    }
    // Sun casters: the model's cascades, a coarser level further out (the
    // role of the JS caster policy: small kit casts into the near cascade only).
    for (var c = 0u; c < 3u; c = c + 1u) {
        if ((pr.mode.x & (2u << c)) == 0u || c >= m.casts.x || !pr_in_cascade(c, s)) {
            continue;
        }
        let mesh = pr_mesh_at(m, max(lod, min(c + 1u, 2u)));
        if (mesh != PR_NONE) {
            pr_records[i * PR_VIEWS + 1u + c] = pr_take(1u + c, mesh);
        }
    }
}

fn pr_write_cmd(slot: u32, count: u32, instances: u32, first: u32, base_vertex: i32, first_instance: u32) {
    let b = slot * 5u;
    pr_cmds[b] = count;
    pr_cmds[b + 1u] = instances;
    pr_cmds[b + 2u] = first;
    pr_cmds[b + 3u] = bitcast<u32>(base_vertex);
    pr_cmds[b + 4u] = first_instance;
}

// Up to PR_MESH_MAX meshes (eo-render props.rs `PROP_MESH_MAX`): each view's per-mesh counts,
// then (in place) their exclusive prefix sum; a mesh's count is the gap to the next offset
// (the view's total past the last). One array, so 1024 meshes need ~24 KB of workgroup memory (props.rs checks the device gives it).
const PR_MESH_MAX: u32 = 1024u;
var<workgroup> pr_wg: array<u32, 6144>; // PR_VIEWS * PR_MESH_MAX
var<workgroup> pr_wg_total: array<u32, 6>; // PR_VIEWS
var<workgroup> pr_wg_tail: array<u32, 12>; // PR_VIEWS * 2

fn pr_wg_count(v: u32, j: u32, meshes: u32) -> u32 {
    let next = select(pr_wg_total[v], pr_wg[v * PR_MESH_MAX + j + 1u], j + 1u < meshes);
    return next - pr_wg[v * PR_MESH_MAX + j];
}

@compute @workgroup_size(256)
fn offsets(@builtin(local_invocation_index) t: u32) {
    let meshes = pr.counts.y;
    let cap = pr.counts.z;
    let mode_y = pr.mode.y;
    for (var k = t; k < meshes; k = k + 256u) {
        for (var v = 0u; v < PR_VIEWS; v = v + 1u) {
            if ((mode_y & (1u << v)) != 0u) {
                pr_wg[v * PR_MESH_MAX + k] = atomicLoad(&pr_counts[v * meshes + k]);
            }
        }
    }
    workgroupBarrier();
    if (t < PR_VIEWS && (mode_y & (1u << t)) != 0u) {
        let v = t;
        var run = 0u;
        var tris = 0u;
        for (var j = 0u; j < meshes; j = j + 1u) {
            let n = pr_wg[v * PR_MESH_MAX + j];
            pr_wg[v * PR_MESH_MAX + j] = run;
            if (n > 0u) {
                let mesh = pr_meshes[j];
                tris = tris + n * (mesh.count + mesh.cut_count) / 3u;
                run = run + n;
            }
        }
        pr_wg_total[v] = run;
        atomicStore(&pr_stats[v * 2u], run);
        atomicStore(&pr_stats[v * 2u + 1u], tris);
    }
    workgroupBarrier();
    for (var k = t; k < meshes; k = k + 256u) {
        for (var v = 0u; v < PR_VIEWS; v = v + 1u) {
            if ((mode_y & (1u << v)) != 0u) {
                atomicStore(&pr_counts[(PR_VIEWS + v) * meshes + k], pr_wg[v * PR_MESH_MAX + k]);
            }
        }
    }
    // one compacted list per (view, part): only meshes with instances,
    // the rest of the list zero-instance (for a renderer without the count draw).
    if (t < PR_VIEWS * 2u && (mode_y & (1u << (t / 2u))) != 0u) {
        let k = t;
        let v = k / 2u;
        let cut = (k & 1u) == 1u;
        var cursor = 0u;
        for (var j = 0u; j < meshes; j = j + 1u) {
            let n = pr_wg_count(v, j, meshes);
            if (n > 0u) {
                let mesh = pr_meshes[j];
                let count = select(mesh.count, mesh.cut_count, cut);
                if (count > 0u) {
                    let first = select(mesh.first, mesh.cut_first, cut);
                    pr_write_cmd(k * meshes + cursor, count, n, first, mesh.base_vertex, v * cap + pr_wg[v * PR_MESH_MAX + j]);
                    cursor = cursor + 1u;
                }
            }
        }
        pr_wg_tail[k] = cursor;
        pr_list_counts[k] = cursor;
    }
    workgroupBarrier();
    for (var list = 0u; list < PR_VIEWS * 2u; list = list + 1u) {
        if ((mode_y & (1u << (list / 2u))) == 0u) {
            continue;
        }
        let start = pr_wg_tail[list];
        for (var j = start + t; j < meshes; j = j + 256u) {
            pr_write_cmd(list * meshes + j, 0u, 0u, 0u, 0, 0u);
        }
    }
}

@compute @workgroup_size(64)
fn scatter(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= pr.counts.x) {
        return;
    }
    let meshes = pr.counts.y;
    var fade_word = 0u;
    if ((pr.mode.x & 48u) != 0u && (pr_records[i * PR_VIEWS + 4u] != PR_NONE)) {
        // the fade (0..255) rides the visible word.
        let inst = pr_instances[i];
        let m = pr_models[inst.model];
        let s = pr_sphere(inst, m);
        let d = max(distance(s.c, pr.cam.xyz) - s.r, 0.0);
        let dl = pr_lod_dist(inst, d);
        fade_word = pr_fade(m, pr_lod_of(m, dl), dl).x << 17u;
    }
    for (var v = 0u; v < PR_VIEWS; v = v + 1u) {
        if ((pr.mode.x & (1u << v)) == 0u) {
            continue;
        }
        let r = pr_records[i * PR_VIEWS + v];
        if (r == PR_NONE) {
            continue;
        }
        let mesh = r >> 20u;
        let slot = r & 0xfffffu;
        let at = v * pr.counts.z + atomicLoad(&pr_counts[(PR_VIEWS + v) * meshes + mesh]) + slot;
        pr_visible[at] = select(i, i | fade_word, v >= 4u);
    }
}
