// lamp and headlight shadows (lighting; light_shadow.rs): the cars and
// people into the atlas, after agents_types.wgsl, agents_common.wgsl (group 2:
// the agents' own data group, posing as the sun's casters pose them) and
// light_shadow_draw.wgsl.

/// eo-agents AgentVertex (8 words): position, normal (snorm8x4), uv, joints (u8x4), weights (unorm8x4), tag.
@vertex
fn ls_agent_vs(@builtin(vertex_index) v: u32) -> LsOut {
    if (v / 3u >= ls_list[1]) {
        return ls_none();
    }
    let iv = ls_vertex(v);
    let rec = ls_recs[iv.y];
    let w = iv.x * 8u;
    let p = vec3<f32>(bitcast<f32>(ls_vtx[w]), bitcast<f32>(ls_vtx[w + 1u]), bitcast<f32>(ls_vtx[w + 2u]));
    let n = unpack4x8snorm(ls_vtx[w + 3u]).xyz;
    let jw = ls_vtx[w + 5u];
    let joints = vec4<u32>(jw & 255u, (jw >> 8u) & 255u, (jw >> 16u) & 255u, jw >> 24u);
    let weights = unpack4x8unorm(ls_vtx[w + 6u]);
    let pose = ag_pose(rec.payload, p, n, joints, weights, ls_vtx[w + 7u]);
    return ls_project(rec.tile, pose.world);
}
