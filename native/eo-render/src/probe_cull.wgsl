// the scene probe's cull (one cube face per dispatch). One invocation
// per object: the same LOD choice the camera's cull pass made this frame
// (cull.wgsl runs first and writes every group's level), terrain at its
// coarse level, nothing transparent, then the face's frustum and the probe's
// far distance (SCENE_PROBE_DEFAULTS.far). Appends DrawIndexedIndirect
// commands for one multi_draw_indexed_indirect_count.

struct Object {
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

struct Group {
    center: vec3<f32>,
    radius: f32,
    ready: u32,
    detailed: u32,
    kind: u32,
    level: u32,
};

struct ProbeCull {
    planes: array<vec4<f32>, 6>,
    // xyz capture point, w far distance
    cam: vec4<f32>,
    object_count: u32,
    // a kit building farther than this (to its bounds) is drawn as its footprint box
    kit_box_m: f32,
    // the bounded list's guard (bounds.rs): objects selected within this of the
    // capture point (0: not counted, without the bounded draws)
    guard_reach: f32,
    pad2: u32,
};

const LOD_NEAR: u32 = 1u;
const LOD_FAR: u32 = 2u;
const LOD_TERRAIN: u32 = 3u;
const LOD_KIT_NEAR: u32 = 4u;
const LOD_KIT_BOX: u32 = 6u;
const LOD_STREET_NEAR: u32 = 7u;
const LOD_STREET_COARSE: u32 = 9u;
const BLEND_BIT: u32 = 256u;

@group(0) @binding(0) var<uniform> pc: ProbeCull;
@group(0) @binding(1) var<storage, read> objects: array<Object>;
@group(0) @binding(2) var<storage, read> groups: array<Group>;
@group(0) @binding(3) var<storage, read_write> draws: array<u32>;
// [0] commands, [1] triangles, [2] the bounded list's guard (bounds.rs): every
// object this cull selects within `guard_reach`, whatever the face, [3] pad,
// [4..10) per face what it draws (`probe_count`), [10..12) pad
@group(0) @binding(4) var<storage, read_write> counters: array<atomic<u32>, 12>;
// the six faces' planes of the capture (`probe_count`).
struct ProbeFaces {
    planes: array<vec4<f32>, 36>,
};
@group(0) @binding(5) var<uniform> pf: ProbeFaces;

// what the probe draws of object i before the face's frustum: x selected
// (LOD, not transparent, within the far distance), y first index, z index count.
fn probe_select(i: u32) -> vec3<u32> {
    let o = objects[i];
    // The probe draws the closed sea plane over the submerged sites.
    if ((o.kind & BLEND_BIT) != 0u || (o.kind & 255u) == 13u) {
        return vec3<u32>(0u);
    }
    var first = o.first_index;
    var count = o.index_count;
    var visible = true;
    if (o.lod_role == LOD_NEAR || o.lod_role == LOD_FAR) {
        let g = groups[o.lod_group];
        let near = g.ready == 1u && g.detailed == 1u;
        visible = select(!near, near, o.lod_role == LOD_NEAR);
    } else if (o.lod_role >= LOD_KIT_NEAR && o.lod_role <= LOD_KIT_BOX) {
        // The reduced rung near the probe when it is resident (else the level the
        // camera chose), the footprint box past kit_box_m: a 128 px face resolves
        // no joinery, and nothing here is occlusion-culled.
        let g = groups[o.lod_group];
        let far = max(0.0, distance(pc.cam.xyz, g.center) - g.radius) > pc.kit_box_m;
        let near_level = select(g.level, 1u, (g.ready & 2u) != 0u);
        visible = select(near_level == o.lod_role - LOD_KIT_NEAR, o.lod_role == LOD_KIT_BOX, far);
    } else if (o.lod_role >= LOD_STREET_NEAR && o.lod_role <= LOD_STREET_COARSE) {
        visible = groups[o.lod_group].level == o.lod_role - LOD_STREET_NEAR;
    } else if (o.lod_role == 10u || o.lod_role == 11u) {
        // the camera's near/far switch (scene.rs LOD_LANDMARK_*).
        visible = (groups[o.lod_group].detailed == 1u) == (o.lod_role == 10u);
    } else if (o.lod_role == 12u) {
        if (distance(pc.cam.xyz, clamp(pc.cam.xyz, o.min, o.max)) > 120.) {
            first = o.far_first_index;
            count = o.far_index_count;
        }
    } else if (o.lod_role == LOD_TERRAIN) {
        first = o.far_first_index;
        count = o.far_index_count;
    }
    if (count == 0u || !visible) {
        return vec3<u32>(0u);
    }
    let c = clamp(pc.cam.xyz, o.min, o.max);
    let d = distance(c, pc.cam.xyz);
    if (pc.guard_reach > 0.0 && d <= pc.guard_reach) {
        atomicAdd(&counters[2], 1u);
    }
    if (d > pc.cam.w) {
        return vec3<u32>(0u);
    }
    return vec3<u32>(1u, first, count);
}

fn in_planes(o: Object, base: u32) -> bool {
    for (var p = 0u; p < 6u; p = p + 1u) {
        let pl = pf.planes[base + p];
        let v = select(o.min, o.max, pl.xyz >= vec3<f32>(0.0));
        if (dot(pl.xyz, v) + pl.w < 0.0) {
            return false;
        }
    }
    return true;
}

@compute @workgroup_size(64)
fn probe_cull(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= pc.object_count) {
        return;
    }
    let s = probe_select(i);
    if (s.x == 0u) {
        return;
    }
    let o = objects[i];
    for (var p = 0u; p < 6u; p = p + 1u) {
        let pl = pc.planes[p];
        let v = select(o.min, o.max, pl.xyz >= vec3<f32>(0.0));
        if (dot(pl.xyz, v) + pl.w < 0.0) {
            return;
        }
    }
    let slot = atomicAdd(&counters[0], 1u);
    let b = slot * 5u;
    draws[b] = s.z;
    draws[b + 1u] = 1u;
    draws[b + 2u] = s.y;
    draws[b + 3u] = bitcast<u32>(o.base_vertex);
    draws[b + 4u] = i;
    atomicAdd(&counters[1], s.z / 3u);
}

// the bounded list's exact count (bounds.rs): once a capture starts, what
// each of its six faces will draw ([4 + face]), read back before the faces.
@compute @workgroup_size(64)
fn probe_count(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= pc.object_count) {
        return;
    }
    if (probe_select(i).x == 0u) {
        return;
    }
    let o = objects[i];
    for (var f = 0u; f < 6u; f = f + 1u) {
        if (in_planes(o, f * 6u)) {
            atomicAdd(&counters[4u + f], 1u);
        }
    }
}
