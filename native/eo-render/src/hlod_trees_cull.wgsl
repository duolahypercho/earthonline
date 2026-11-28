// stage 4, the distant trees' cull (hlod_trees.rs). One invocation per
// block of trees (`TreeBlock`, 128 m): a block wholly nearer than the near
// distance (the street props' trees draw there), outside the frustum, or (the
// variant run after the pyramid) hidden behind this frame's early depth draws
// nothing; any other gets one DrawIndirect command, its trees as instances.
//
// Appended: `fn tc_occluded(mn, mx) -> bool` (false, or the pyramid test).

struct TreeBlock {
    min: vec3<f32>,
    first: u32,
    max: vec3<f32>,
    count: u32,
};

struct TcParams {
    planes: array<vec4<f32>, 6>,
    view_proj: mat4x4<f32>,
    // the camera; w the near distance (ground distance, as the vertex shader's)
    cam: vec4<f32>,
    // x = blocks, y = 1 under `--tree-crowns`: the near distance is 3D (the props' measure)
    counts: vec4<u32>,
    // the bounded list's guards (bounds.rs): the camera's frustum widened by
    // WIDE_DEG a side; x the ring's reach, y 1 to count them, z the near sphere
    wide: array<vec4<f32>, 6>,
    guard: vec4<f32>,
};

@group(0) @binding(0) var<uniform> tc: TcParams;
@group(0) @binding(1) var<storage, read> tc_blocks: array<TreeBlock>;
@group(0) @binding(2) var<storage, read_write> tc_cmds: array<u32>;
// [0] commands, [1] trees, [2] blocks in the frustum, [3] blocks occluded,
// [4] blocks past the near distance in the widened frustum, [5] past it within the ring (bounds.rs)
@group(0) @binding(3) var<storage, read_write> tc_count: array<atomic<u32>, 8>;

// Whether block `i` draws (and the frustum and occlusion counters).
fn tc_keep(i: u32) -> bool {
    let b = tc_blocks[i];
    if (b.count == 0u) {
        return false;
    }
    let fx = max(abs(tc.cam.x - b.min.x), abs(tc.cam.x - b.max.x));
    let fz = max(abs(tc.cam.z - b.min.z), abs(tc.cam.z - b.max.z));
    if (tc.counts.y != 0u) {
        // (`--tree-crowns`) a tree's 3D distance to its sphere is below its base's, which is below the far corner's.
        let fy = max(abs(tc.cam.y - b.min.y), abs(tc.cam.y - b.max.y));
        if (length(vec3<f32>(fx, fy, fz)) < tc.cam.w) {
            return false;
        }
    } else if (length(vec2<f32>(fx, fz)) < tc.cam.w) {
        return false;
    }
    for (var p = 0u; p < 6u; p = p + 1u) {
        let pl = tc.planes[p];
        let v = select(b.min, b.max, pl.xyz >= vec3<f32>(0.0));
        if (dot(pl.xyz, v) + pl.w < 0.0) {
            return false;
        }
    }
    atomicAdd(&tc_count[2], 1u);
    if (tc_occluded(b.min, b.max)) {
        atomicAdd(&tc_count[3], 1u);
        return false;
    }
    return true;
}

// The bounded list's guards (bounds.rs, `tc.guard.y` 1) of block i past the
// near distance: within the ring's reach ([5]) and, of those, in the widened
// frustum or the near sphere ([4]).
fn tc_count_guards(i: u32) {
    let b = tc_blocks[i];
    if (tc.guard.y != 1.0 || b.count == 0u) {
        return;
    }
    let fx = max(abs(tc.cam.x - b.min.x), abs(tc.cam.x - b.max.x));
    let fz = max(abs(tc.cam.z - b.min.z), abs(tc.cam.z - b.max.z));
    if (tc.counts.y != 0u) {
        // a tree's 3D distance to its sphere is below its base's, which is below the far corner's.
        let fy = max(abs(tc.cam.y - b.min.y), abs(tc.cam.y - b.max.y));
        if (length(vec3<f32>(fx, fy, fz)) < tc.cam.w) {
            return;
        }
    } else if (length(vec2<f32>(fx, fz)) < tc.cam.w) {
        return;
    }
    let gd = distance(clamp(tc.cam.xyz, b.min, b.max), tc.cam.xyz);
    if (gd > tc.guard.x) {
        return;
    }
    atomicAdd(&tc_count[5], 1u);
    var wide = true;
    if (gd > tc.guard.z) {
        for (var p = 0u; p < 6u; p = p + 1u) {
            let pl = tc.wide[p];
            let v = select(b.min, b.max, pl.xyz >= vec3<f32>(0.0));
            if (dot(pl.xyz, v) + pl.w < 0.0) {
                wide = false;
            }
        }
    }
    if (wide) {
        atomicAdd(&tc_count[4], 1u);
    }
}

@compute @workgroup_size(64)
fn tree_cull(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= tc.counts.x) {
        return;
    }
    tc_count_guards(i);
    if (!tc_keep(i)) {
        return;
    }
    let b = tc_blocks[i];
    let slot = atomicAdd(&tc_count[0], 1u);
    let c = slot * 4u;
    tc_cmds[c] = 6u;
    tc_cmds[c + 1u] = b.count;
    tc_cmds[c + 2u] = 0u;
    tc_cmds[c + 3u] = b.first;
    atomicAdd(&tc_count[1], b.count);
}
