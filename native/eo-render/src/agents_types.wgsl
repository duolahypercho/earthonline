// Moving things (eo-render/src/agents.rs): the records the agent passes share
// (the cull compute, the colour pass, the sun shadow pass). Every name here
// starts with ag_ / Ag so nothing collides with draw.wgsl or the look.
// Layouts match eo-agents (GpuAgent, GpuModel, GpuMesh) and agents.rs (AgParams).

struct AgParams {
    planes: array<vec4<f32>, 6>,
    // xyz eye
    cam: vec4<f32>,
    // x agents, y meshes, z capacity (visible list stride per view), w models
    counts: vec4<u32>,
    // vehicles: LOD0, LOD1, LOD2 radii, box max distance
    veh_radius: vec4<f32>,
    veh_cap: vec4<u32>,
    // hero, near, mid tier radii, far max distance
    ped_radius: vec4<f32>,
    ped_cap: vec4<u32>,
    clip_frames: vec4<u32>,
    clip_base: vec4<u32>,
    // tail day, tail night, brake day, brake night
    lamp_tail: vec4<f32>,
    // head day, head night, indicator day, indicator night
    lamp_head: vec4<f32>,
    // x night factor 0..1, y daytime running floor, z seconds, w reverse
    lamp_misc: vec4<f32>,
    // x the views this cull dispatch serves (bit 0 the camera, bits 1..3
    // the cascades): before the shadows the cascades (and the camera without
    // an occlusion pass), after the pyramid the camera alone.
    misc: vec4<u32>,
    // the camera's view-projection (the pyramid test).
    view_proj: mat4x4<f32>,
};

struct AgAgent {
    pos: vec3<f32>,
    model: u32,
    rot: vec4<f32>,
    color: u32,
    flags: u32,
    anim: f32,
    steer: f32,
    speed: f32,
    scale: f32,
    clip_phase: f32,
    clip: u32,
};

struct AgModel {
    meshes: vec4<u32>,
    wheel: array<vec4<f32>, 4>,
    center: vec3<f32>,
    radius: f32,
    model_class: u32,
    skeleton: u32,
    joints: u32,
    wheel_radius: f32,
    pad: vec4<u32>,
};

struct AgMesh {
    first_index: u32,
    index_count: u32,
    glass_first: u32,
    glass_count: u32,
    base_vertex: i32,
    // first joint matrix of the mesh's skeleton bake, joints
    skeleton: u32,
    joints: u32,
    pad: u32,
};

const AG_PERSON: u32 = 1u;
const AG_FLAG_BRAKING: u32 = 1u;
const AG_FLAG_TURN_LEFT: u32 = 4u;
const AG_FLAG_TURN_RIGHT: u32 = 8u;
const AG_PLAYER: u32 = 65536u;
const AG_BLINK_ON: u32 = 131072u;

fn ag_rotate(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    let t = 2.0 * cross(q.xyz, v);
    return v + q.w * t + cross(q.xyz, t);
}
