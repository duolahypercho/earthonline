// the far layer's cull (hlod.rs). One invocation per entry of the cell
// table (neighbourhood cells, then districts): a resident entry inside the
// frustum, and (the variant run after the pyramid, between the early and the
// late city pass) not hidden behind this frame's early depth, gets one
// DrawIndexedIndirect command, its entry index as first_instance. The
// buildings inside a drawn entry are switched one by one in the vertex
// shader (hlod_draw.wgsl) from the state the LOD pass wrote.
//
// Appended: `fn hl_occluded(mn, mx) -> bool` (false, or the pyramid test).

struct HlCell {
    min: vec3<f32>,
    layer_base: u32,
    max: vec3<f32>,
    flags: u32,
    first_index: u32,
    index_count: u32,
    base_vertex: i32,
    // a district chunk: its neighbourhood cell + 1 (0: none)
    cell: u32,
};

struct HlCull {
    planes: array<vec4<f32>, 6>,
    view_proj: mat4x4<f32>,
    // x = entries
    counts: vec4<u32>,
    // the camera; w the cell -> district band's start (HLOD_FAR_A)
    cam: vec4<f32>,
    // the bounded list's guards (bounds.rs): the widened frustum; x the ring's
    // reach, y 1 to count them, z the near sphere
    wide: array<vec4<f32>, 6>,
    guard: vec4<f32>,
};

@group(0) @binding(0) var<uniform> hc: HlCull;
@group(0) @binding(1) var<storage, read> hl_cells: array<HlCell>;
@group(0) @binding(2) var<storage, read_write> hl_cmds: array<u32>;
// [0] commands, [1] triangles, [2] entries in the frustum, [3] entries occluded,
// [4] entries in the widened frustum or the near sphere, [5] within the ring (bounds.rs)
@group(0) @binding(3) var<storage, read_write> hl_count: array<atomic<u32>, 8>;
// per cell its residency (hlod.rs FLAG_*: 1 the cell, 2 its district)
@group(0) @binding(4) var<storage, read> hl_flags: array<u32>;

// Margin, metres, between a chunk's box and its buildings' bounding-sphere centres.
const HL_CHUNK_MARGIN_M: f32 = 50.0;

@compute @workgroup_size(64)
fn hlod_cull(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= hc.counts.x) {
        return;
    }
    let c = hl_cells[i];
    if ((c.flags & 1u) == 0u || c.index_count == 0u) {
        return;
    }
    // A district chunk whose cell is resident and whose buildings are all nearer
    // than the band: the LOD pass gives every one of them to the cell (or a
    // nearer level), so the chunk would draw nothing (cull.wgsl `hlod_far`).
    if ((c.flags & 2u) != 0u && c.cell != 0u && (hl_flags[c.cell - 1u] & 1u) != 0u) {
        let far = max(abs(hc.cam.xyz - c.min), abs(hc.cam.xyz - c.max)) + vec3<f32>(HL_CHUNK_MARGIN_M);
        if (length(far) < hc.cam.w) {
            return;
        }
    }
    // the bounded draws' guards (bounds.rs; `hc.guard.y` 1 with them).
    let gd = select(0.0, distance(clamp(hc.cam.xyz, c.min, c.max), hc.cam.xyz), hc.guard.y == 1.0);
    if (hc.guard.y == 1.0 && gd <= hc.guard.x) {
        atomicAdd(&hl_count[5], 1u);
        var wide = true;
        if (gd > hc.guard.z) {
            for (var p = 0u; p < 6u; p = p + 1u) {
                let pl = hc.wide[p];
                let v = select(c.min, c.max, pl.xyz >= vec3<f32>(0.0));
                if (dot(pl.xyz, v) + pl.w < 0.0) {
                    wide = false;
                }
            }
        }
        if (wide) {
            atomicAdd(&hl_count[4], 1u);
        }
    }
    for (var p = 0u; p < 6u; p = p + 1u) {
        let pl = hc.planes[p];
        let v = select(c.min, c.max, pl.xyz >= vec3<f32>(0.0));
        if (dot(pl.xyz, v) + pl.w < 0.0) {
            return;
        }
    }
    atomicAdd(&hl_count[2], 1u);
    if (hl_occluded(c.min, c.max)) {
        atomicAdd(&hl_count[3], 1u);
        return;
    }
    let slot = atomicAdd(&hl_count[0], 1u);
    let b = slot * 5u;
    hl_cmds[b] = c.index_count;
    hl_cmds[b + 1u] = 1u;
    hl_cmds[b + 2u] = c.first_index;
    hl_cmds[b + 3u] = bitcast<u32>(c.base_vertex);
    hl_cmds[b + 4u] = i;
    atomicAdd(&hl_count[1], c.index_count / 3u);
}
