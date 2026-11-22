// the records the street prop passes share (cull compute, colour
// pass, sun shadow pass, props2 wires). Names start with pr_ / Pr. Layouts
// match eo-render/src/props_lib.rs (GpuPropMesh, GpuPropModel) and props.rs
// (PrParams, GpuPropInstance).

struct PrParams {
    planes: array<vec4<f32>, 6>,
    // xyz eye
    cam: vec4<f32>,
    // x instance slots in use (high-water mark), y meshes, z visible stride per view, w models
    counts: vec4<u32>,
    // x views this run classifies (bit per view: 0 camera, 1..3 cascades, 4..5 the camera's
    // cross-fade pair), y views whose commands it writes, z props2 features (PR_F_*), w unused
    mode: vec4<u32>,
    view_proj: mat4x4<f32>,
    // x lens scale (ACES scale / exposure), y alpha test, z seconds (wind), w sign face count
    lens: vec4<f32>,
    // detectable-warning domes: base diameter, height, pitch (m), unused
    dome: vec4<f32>,
    // wind direction xz, sway amplitude (m), wire half width floor (rad, pixelHalfWidthRad)
    wind: vec4<f32>,
    // wire fade near, fade far (m), minimum coverage, unused
    wire: vec4<f32>,
    // x wire segment slots in use
    extra: vec4<u32>,
};

struct PrInstance {
    pos: vec3<f32>,
    yaw: f32,
    scale: vec3<f32>,
    model: u32,
    variant: u32,
    lamp: u32,
    slope: f32,
    pad: u32,
};

struct PrModel {
    meshes: vec4<u32>,
    lod: vec4<f32>,
    center: vec3<f32>,
    radius: f32,
    casts: vec4<u32>,
};

struct PrMesh {
    first: u32,
    count: u32,
    cut_first: u32,
    cut_count: u32,
    base_vertex: i32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
};

const PR_NONE: u32 = 0xffffffffu;
const PR_VIEWS: u32 = 6u; // + the camera's cross-fade pair (4: leaving level, 5: arriving level)
const PR_CAMERA_VIEWS: u32 = 49u; // views 0, 4, 5
// Model ids (eo_props::Model).
const PR_SIGNAL_HEAD: u32 = 2u;
const PR_PED_HEAD: u32 = 3u;
const PR_SIGN_PLATE: u32 = 5u;
const PR_TACTILE_PAD: u32 = 8u;
// eo-props `Model::TreeBasin` (`--tree-basins`): tilted along the kerb (its local +Z) as the pad is along its depth.
const PR_TREE_BASIN: u32 = 35u;
// props2 features (PrParams.mode.z).
const PR_F_FADE: u32 = 1u;
const PR_F_WIND: u32 = 2u;
const PR_F_LEAF: u32 = 4u;
const PR_F_DOMES: u32 = 8u;
const PR_F_SIGNS: u32 = 16u;
// a visible word: instance index (17 bits) | fade (8 bits) << 17.
const PR_INDEX_MASK: u32 = 0x1ffffu;

// three.js rotation.y: local +Z goes to (sin yaw, 0, cos yaw).
fn pr_rot(yaw: f32, v: vec3<f32>) -> vec3<f32> {
    let s = sin(yaw);
    let c = cos(yaw);
    return vec3<f32>(v.x * c + v.z * s, v.y, -v.x * s + v.z * c);
}

fn pr_local(i: PrInstance, p: vec3<f32>) -> vec3<f32> {
    var l = p * i.scale;
    if (i.model == PR_TACTILE_PAD || i.model == PR_TREE_BASIN) {
        // The pad takes the ramp's running slope along its depth; a tree basin the street's grade along the kerb.
        l.y = l.y + l.z * i.slope;
    }
    return l;
}

// gentle wind sway, height-weighted per vertex (`w`, baked by the
// library: 0 for everything but trees), a function of the instance position
// and the simulation clock only, so the colour and the shadow passes agree.
fn pr_wind(i: PrInstance, p: vec3<f32>, w: f32) -> vec3<f32> {
    if ((pr.mode.z & PR_F_WIND) == 0u || w <= 0.0) {
        return vec3<f32>(0.0);
    }
    let t = pr.lens.z;
    let phase = dot(i.pos.xz, vec2<f32>(0.071, 0.113)) + f32(i.variant & 255u) * 0.37;
    let gust = sin(t * 0.9 + phase) * 0.6 + sin(t * 2.1 + phase * 1.3) * 0.3;
    let flutter = sin(t * 5.3 + phase * 2.9 + (p.x + p.z) * 3.0) * 0.1 * w;
    let a = (gust + flutter) * w * pr.wind.z * i.scale.y;
    return vec3<f32>(pr.wind.x * a, -abs(a) * 0.05, pr.wind.y * a);
}

fn pr_world(i: PrInstance, p: vec3<f32>, w: f32) -> vec3<f32> {
    return i.pos + pr_rot(i.yaw, pr_local(i, p)) + pr_wind(i, p, w);
}
