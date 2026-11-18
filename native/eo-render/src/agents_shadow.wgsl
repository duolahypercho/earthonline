// Moving things into the sun shadow cascades: appended to agents_common.wgsl.
// Group 0: the look's cascade view (dynamic offset, as look_shadow_draw.wgsl);
// group 2: the agent data. Depth only.

struct AgShadowView {
    view_proj: mat4x4<f32>,
    copy_step: vec4<f32>,
};

@group(0) @binding(0) var<uniform> ag_sv: AgShadowView;

@vertex
fn agent_shadow_vs(
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec4<f32>,
    @location(3) joints: vec4<u32>,
    @location(4) weights: vec4<f32>,
    @location(5) tag: u32,
    @builtin(instance_index) inst: u32,
) -> @builtin(position) vec4<f32> {
    let pose = ag_pose(ag_visible[inst], pos, normal.xyz, joints, weights, tag);
    return ag_sv.view_proj * vec4<f32>(pose.world, 1.0);
}
