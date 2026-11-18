// the moving agents' screen motion for the temporal resolve (appended to
// the agent draw shader: draw.wgsl + the look + agents_*.wgsl). The agents
// are drawn again after the city pass into a velocity target, depth-tested
// against the depth they wrote, each vertex at its pose now and where its
// body was a frame ago (the agent's position less its velocity along its
// heading x the frame time). Elsewhere the target keeps its clear value and
// the resolve reprojects by the camera's motion and the depth.

struct AgVelOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) cur: vec4<f32>,
    @location(1) prev: vec4<f32>,
};

@vertex
fn agent_vel_vs(
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) joints: vec4<u32>,
    @location(4) weights: vec4<f32>,
    @location(5) tag: u32,
    @builtin(instance_index) inst: u32,
) -> AgVelOut {
    let pose = ag_pose(ag_visible[inst], pos, normal.xyz, joints, weights, tag);
    let a = ag_agents[pose.agent];
    let fwd = ag_rotate(a.rot, vec3<f32>(0.0, 0.0, 1.0));
    let prev_world = pose.world - fwd * a.speed * skyu.taa.w;
    var out: AgVelOut;
    out.clip = frame.view_proj * vec4<f32>(pose.world, 1.0);
    out.cur = skyu.vp * vec4<f32>(pose.world, 1.0);
    out.prev = skyu.prev_vp * vec4<f32>(prev_world, 1.0);
    return out;
}

@fragment
fn agent_vel_fs(in: AgVelOut) -> @location(0) vec4<f32> {
    let c = in.cur.xy / in.cur.w;
    let p = in.prev.xy / in.prev.w;
    return vec4<f32>((c - p) * vec2<f32>(0.5, -0.5), 0.0, 1.0);
}
