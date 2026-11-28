// stage 4, the distant trees' cull without multi-draw-indirect-count
// (hlod_trees.rs, appended to hlod_trees_cull.wgsl). The same block test;
// a kept block appends its trees' indices to one list, and the trees draw
// as one instanced draw whose instance count the cull adds up (the vertex
// shader reads its tree through the list: `ft_index`).

// The kept trees' indices, in the order the blocks were kept.
@group(0) @binding(4) var<storage, read_write> tc_list: array<u32>;
// The draw: 6 vertices (written once), the kept trees, 0, 0.
@group(0) @binding(5) var<storage, read_write> tc_draw: array<atomic<u32>, 4>;

var<workgroup> tc_wg_first: array<u32, 64>;
var<workgroup> tc_wg_count: array<u32, 64>;
var<workgroup> tc_wg_at: array<u32, 64>;
var<workgroup> tc_wg_kept: atomic<u32>;

@compute @workgroup_size(64)
fn tree_cull_list(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(local_invocation_index) lid: u32) {
    if (lid == 0u) {
        atomicStore(&tc_wg_kept, 0u);
    }
    workgroupBarrier();
    let i = gid.x;
    if (i < tc.counts.x && tc_keep(i)) {
        let b = tc_blocks[i];
        let at = atomicAdd(&tc_draw[1], b.count);
        atomicAdd(&tc_count[0], 1u);
        atomicAdd(&tc_count[1], b.count);
        let q = atomicAdd(&tc_wg_kept, 1u);
        tc_wg_first[q] = b.first;
        tc_wg_count[q] = b.count;
        tc_wg_at[q] = at;
    }
    workgroupBarrier();
    // Workgroup-cooperative coalesced burst writes into `tc_list`.
    let kept = atomicLoad(&tc_wg_kept);
    for (var q = 0u; q < kept; q = q + 1u) {
        let first = tc_wg_first[q];
        let count = tc_wg_count[q];
        let at = tc_wg_at[q];
        for (var k = lid; k < count; k = k + 64u) {
            tc_list[at + k] = first + k;
        }
    }
}
