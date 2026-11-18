// Moving things: GPU cull and level of detail (appended to agents_types.wgsl).
//
// One compute pass a frame, five dispatches, nothing per agent on the CPU:
//   classify  (one invocation per agent) bounding sphere against the camera
//             frustum; visible agents fill a distance histogram per class;
//   cutoffs   (one invocation) walks the histograms nearest first and finds,
//             per detail tier, the distance bin where the tier's cap (the JS
//             hero overlay's / crowd budget's caps) or its radius is reached:
//             the nearest agents get the finest tier, the rest are demoted,
//             as `chooseHeroTier` and the crowd bands do;
//   assign    (one invocation per agent) picks the camera tier and, per sun
//             shadow cascade, a coarser caster tier (sphere against the
//             cascade's box: casters behind the camera still cast), and takes
//             a slot in that (view, mesh) bucket;
//   offsets   (one invocation) prefix-sums the buckets and writes one
//             DrawIndexedIndirect per (view, mesh): count = agents in it,
//             first_instance = where its visible-list run starts;
//   scatter   (one invocation per agent) writes agent | lod << 28 into the runs.
// Views: 0 the camera (an opaque and a transparent list), 1..3 the cascades.
// A tier's agents are drawn by ONE instanced command, all of them by ONE
// multi-draw per list.

struct AgShadowViewPadded {
    view_proj: mat4x4<f32>,
    copy_step: vec4<f32>,
    pad: array<vec4<f32>, 11>,
};

const AG_VIEWS: u32 = 4u;
const AG_BINS: u32 = 512u;
const AG_BIN_M: f32 = 0.5;
const AG_NONE: u32 = 0xffffffffu;
// Counter words of `ag_stats`: per view instances drawn and triangles; then
// per class instances submitted.
const AG_STATS: u32 = 12u;

@group(0) @binding(0) var<uniform> ag: AgParams;
@group(0) @binding(1) var<storage, read> ag_agents: array<AgAgent>;
@group(0) @binding(2) var<storage, read> ag_models: array<AgModel>;
@group(0) @binding(3) var<storage, read> ag_meshes: array<AgMesh>;
// 4 words an agent: (mesh << 23 | lod << 21 | slot) per view, or AG_NONE (9 mesh bits, up to 511 meshes).
@group(0) @binding(4) var<storage, read_write> ag_records: array<u32>;
@group(0) @binding(5) var<storage, read_write> ag_visible: array<u32>;
// [0, 4M) counts per (view, mesh); [4M, 8M) offsets.
@group(0) @binding(6) var<storage, read_write> ag_counts: array<atomic<u32>>;
// [0, 2 BINS) histograms per class; [2 BINS, 2 BINS + 8) cutoff bins (class * 4 + tier).
@group(0) @binding(7) var<storage, read_write> ag_hist: array<atomic<u32>>;
// 5 lists of M commands: camera opaque, camera transparent, cascades 0..2.
@group(0) @binding(8) var<storage, read_write> ag_cmds: array<u32>;
@group(0) @binding(9) var<storage, read_write> ag_stats: array<atomic<u32>, 12>;
@group(0) @binding(10) var<uniform> ag_cascades: array<AgShadowViewPadded, 3>;

struct AgSphere {
    c: vec3<f32>,
    r: f32,
};

fn ag_sphere(a: AgAgent, m: AgModel) -> AgSphere {
    var s: AgSphere;
    s.c = a.pos + ag_rotate(a.rot, m.center * a.scale);
    s.r = m.radius * max(a.scale, 1.0);
    return s;
}

fn ag_in_camera(s: AgSphere) -> bool {
    for (var p = 0u; p < 6u; p = p + 1u) {
        let pl = ag.planes[p];
        if (dot(pl.xyz, s.c) + pl.w < -s.r) {
            return false;
        }
    }
    return true;
}

fn ag_in_cascade(c: u32, s: AgSphere) -> bool {
    let m = ag_cascades[c].view_proj;
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

fn ag_bin(d: f32) -> u32 {
    return min(u32(d / AG_BIN_M), AG_BINS - 1u);
}

@compute @workgroup_size(64)
fn classify(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= ag.counts.x) {
        return;
    }
    let a = ag_agents[i];
    if (a.model >= ag.counts.w || (a.flags & AG_PLAYER) != 0u) {
        return;
    }
    let m = ag_models[a.model];
    let s = ag_sphere(a, m);
    if (!ag_in_camera(s)) {
        return;
    }
    let cls = min(m.model_class, 1u);
    atomicAdd(&ag_hist[cls * AG_BINS + ag_bin(distance(s.c, ag.cam.xyz))], 1u);
}

// Workgroup copies for the two one-workgroup passes (histograms; counts and offsets).
// 4 views x 512 meshes (16 KiB together, the default workgroup limit).
var<workgroup> ag_wg_a: array<u32, 2048>;
var<workgroup> ag_wg_b: array<u32, 2048>;

@compute @workgroup_size(256)
fn cutoffs(@builtin(local_invocation_index) l: u32) {
    for (var k = l; k < 2u * AG_BINS; k = k + 256u) {
        ag_wg_a[k] = atomicLoad(&ag_hist[k]);
    }
    workgroupBarrier();
    // One invocation per class walks its histogram in workgroup memory.
    if (l < 2u) {
        let cls = l;
        var b = 0u;
        for (var t = 0u; t < 3u; t = t + 1u) {
            var cap = ag.veh_cap[t];
            var radius = ag.veh_radius[t];
            if (cls == 1u) {
                cap = ag.ped_cap[t];
                radius = ag.ped_radius[t];
            }
            var n = 0u;
            loop {
                if (b >= AG_BINS || f32(b) * AG_BIN_M >= radius) {
                    break;
                }
                let h = ag_wg_a[cls * AG_BINS + b];
                if (n + h > cap) {
                    break;
                }
                n = n + h;
                b = b + 1u;
            }
            atomicStore(&ag_hist[2u * AG_BINS + cls * 4u + t], b);
        }
        // The last tier keeps its own cap (the crowd's far band); vehicles: none.
        var cap3 = ag.veh_cap[3];
        if (cls == 1u) {
            cap3 = ag.ped_cap[3];
        }
        var n3 = 0u;
        var b3 = b;
        loop {
            if (b3 >= AG_BINS) {
                break;
            }
            let h = ag_wg_a[cls * AG_BINS + b3];
            if (n3 + h > cap3) {
                break;
            }
            n3 = n3 + h;
            b3 = b3 + 1u;
        }
        atomicStore(&ag_hist[2u * AG_BINS + cls * 4u + 3u], b3);
    }
}

// Tier by distance alone (players, and the shadow casters).
fn ag_tier_by_distance(cls: u32, d: f32) -> u32 {
    var r = ag.veh_radius;
    if (cls == 1u) {
        r = ag.ped_radius;
    }
    if (d < r.x) {
        return 0u;
    }
    if (d < r.y) {
        return 1u;
    }
    if (d < r.z) {
        return 2u;
    }
    if (d < r.w) {
        return 3u;
    }
    return 4u;
}

fn ag_take(view: u32, mesh: u32, lod: u32) -> u32 {
    let slot = atomicAdd(&ag_counts[view * ag.counts.y + mesh], 1u);
    return (mesh << 23u) | (lod << 21u) | slot; // 9 mesh bits
}

@compute @workgroup_size(64)
fn assign(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= ag.counts.x) {
        return;
    }
    // only the views this dispatch serves (`ag.misc.x`).
    let views = ag.misc.x;
    for (var v = 0u; v < AG_VIEWS; v = v + 1u) {
        if ((views & (1u << v)) != 0u) {
            ag_records[i * AG_VIEWS + v] = AG_NONE;
        }
    }
    let a = ag_agents[i];
    if (a.model >= ag.counts.w) {
        return;
    }
    let m = ag_models[a.model];
    let s = ag_sphere(a, m);
    let d = distance(s.c, ag.cam.xyz);
    let cls = min(m.model_class, 1u);
    let by_distance = ag_tier_by_distance(cls, d);
    if ((views & 14u) != 0u) {
        atomicAdd(&ag_stats[8u + cls], 1u);
    }
    // the camera view drops an agent the pyramid hides (`ag_occluded`:
    // false before the pyramid exists; after it, `hzb_occluded` on the box
    // around the agent's bounding sphere).
    let r3 = vec3<f32>(s.r);
    if ((views & 1u) != 0u && ag_in_camera(s) && !ag_occluded(s.c - r3, s.c + r3)) {
        var lod = by_distance;
        if ((a.flags & AG_PLAYER) == 0u) {
            // The tiers the cutoffs leave this distance bin.
            let b = ag_bin(d);
            lod = 4u;
            for (var t = 0u; t < 4u; t = t + 1u) {
                if (b < atomicLoad(&ag_hist[2u * AG_BINS + cls * 4u + t])) {
                    lod = t;
                    break;
                }
            }
            lod = max(lod, by_distance);
            // a corridor vehicle past the box tier keeps its box (the frustum's far plane ends it).
            if (lod >= 4u && cls == 0u && (a.flags & AG_CORRIDOR) != 0u && ag.misc.y != 0u) {
                lod = 3u;
            }
        }
        if (lod < 4u) {
            ag_records[i * AG_VIEWS] = ag_take(0u, m.meshes[lod], lod);
        }
    }
    // Sun shadow casters: a coarser tier per cascade (the role of the JS
    // caster policy: cascade 0 casts a car at LOD1 or coarser and a person at
    // the mid tier, cascade 1 cars at LOD2 and people at the far tier,
    // cascade 2 boxes and far tiers).
    for (var c = 0u; c < 3u; c = c + 1u) {
        if ((views & (2u << c)) == 0u || by_distance >= 4u || !ag_in_cascade(c, s)) {
            continue;
        }
        var floor_lod = 1u + c;
        if (cls == 1u) {
            floor_lod = min(2u + c, 3u);
        }
        let lod = min(max(by_distance, floor_lod), 3u);
        ag_records[i * AG_VIEWS + 1u + c] = ag_take(1u + c, m.meshes[lod], lod);
    }
}

fn ag_write_cmd(slot: u32, count: u32, instances: u32, first: u32, base_vertex: i32, first_instance: u32) {
    let b = slot * 5u;
    ag_cmds[b] = count;
    ag_cmds[b + 1u] = instances;
    ag_cmds[b + 2u] = first;
    ag_cmds[b + 3u] = bitcast<u32>(base_vertex);
    ag_cmds[b + 4u] = first_instance;
}

// One invocation per mesh (two per invocation, meshes <= 511): counts into workgroup memory, one
// invocation per view prefix-sums them there, then every mesh writes its commands.
@compute @workgroup_size(256)
fn offsets(@builtin(local_invocation_index) l0: u32) {
    let meshes = ag.counts.y;
    let cap = ag.counts.z;
    for (var k = l0; k < meshes; k = k + 256u) {
        for (var v = 0u; v < AG_VIEWS; v = v + 1u) {
            ag_wg_a[v * 512u + k] = atomicLoad(&ag_counts[v * meshes + k]);
        }
    }
    workgroupBarrier();
    // the serial prefix sum reads workgroup memory only (it loaded each
    // mesh record from storage, one at a time, which grew with the mesh count);
    // the triangle counts are added per mesh below.
    if (l0 < AG_VIEWS) {
        let v = l0;
        var run = 0u;
        for (var j = 0u; j < meshes; j = j + 1u) {
            let n = ag_wg_a[v * 512u + j];
            ag_wg_b[v * 512u + j] = run;
            run = run + n;
        }
        atomicStore(&ag_stats[v * 2u], run);
    }
    workgroupBarrier();
    for (var k = l0; k < meshes; k = k + 256u) {
        let mesh = ag_meshes[k];
        for (var v = 0u; v < AG_VIEWS; v = v + 1u) {
            let n = ag_wg_a[v * 512u + k];
            let off = ag_wg_b[v * 512u + k];
            // triangles drawn, for the views this dispatch serves (once each).
            if (n > 0u && (ag.misc.x & (1u << v)) != 0u) {
                var t = mesh.index_count;
                if (v == 0u) {
                    t = t + mesh.glass_count;
                }
                atomicAdd(&ag_stats[v * 2u + 1u], n * t / 3u);
            }
            atomicStore(&ag_counts[(AG_VIEWS + v) * meshes + k], off);
            let first_instance = v * cap + off;
            if (v == 0u) {
                ag_write_cmd(k, mesh.index_count, n, mesh.first_index, mesh.base_vertex, first_instance);
                ag_write_cmd(meshes + k, mesh.glass_count, select(0u, n, mesh.glass_count > 0u), mesh.glass_first, mesh.base_vertex, first_instance);
            } else {
                ag_write_cmd((1u + v) * meshes + k, mesh.index_count, n, mesh.first_index, mesh.base_vertex, first_instance);
            }
        }
    }
}

@compute @workgroup_size(64)
fn scatter(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= ag.counts.x) {
        return;
    }
    let meshes = ag.counts.y;
    for (var v = 0u; v < AG_VIEWS; v = v + 1u) {
        let r = ag_records[i * AG_VIEWS + v];
        if (r == AG_NONE || (ag.misc.x & (1u << v)) == 0u) { // this dispatch's views
            continue;
        }
        let mesh = r >> 23u; // 9 mesh bits
        let lod = (r >> 21u) & 3u;
        let slot = r & 0x1fffffu;
        let at = v * ag.counts.z + atomicLoad(&ag_counts[(AG_VIEWS + v) * meshes + mesh]) + slot;
        ag_visible[at] = i | (lod << 28u);
    }
}
