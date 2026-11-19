// stage 4, the distant trees' cull without multi-draw-indirect-count
// (hlod_trees.rs, appended to hlod_trees_cull.wgsl). The same block test;
// a kept block appends its trees' indices to one list, and the trees draw
// as one instanced draw whose instance count the cull adds up (the vertex
// shader reads its tree through the list: `ft_index`).

// The kept trees' indices, in the order the blocks were kept.
@group(0) @binding(4) var<storage, read_write> tc_list: array<u32>;
// The draw: 6 vertices (written once), the kept trees, 0, 0.
@group(0) @binding(5) var<storage, read_write> tc_draw: array<atomic<u32>, 4>;

@compute @workgroup_size(64)
fn tree_cull_list(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= tc.counts.x) {
        return;
    }
    if (!tc_keep(i)) {
        return;
    }
    let b = tc_blocks[i];
    let at = atomicAdd(&tc_draw[1], b.count);
    atomicAdd(&tc_count[0], 1u);
    atomicAdd(&tc_count[1], b.count);
    for (var k = 0u; k < b.count; k = k + 1u) {
        tc_list[at + k] = b.first + k;
    }
}
