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

fn pr_is_veg(model: u32) -> bool {
    return model >= PR_VEG_BASE && model < PR_VEG_BASE + PR_VEG_COUNT;
}

// the vegetation's distance thinning (props_lib2/vegetation_models.rs, whose
// `thin_wgsl` defines PR_VEG_* and `pr_veg_thin` ahead of this file): the
// share of a thinned model's instances kept falls linearly from 1 at the thin
// distance to PR_VEG_FLOOR at the draw distance; an instance whose rank (its
// `slope` word, uniform in [0, 1)) is over the share shrinks away over a short
// band, and the survivors grow by 1/sqrt(share), at most the model's own
// limit (`pr_veg_thin(..).z`), to keep the cover. 1 for everything else. A function of the instance and the eye only, so the cull,
// colour and shadow passes agree.
// the distance a meadow patch picks its level by (`--no-meadow-breakup` off:
// PR_MEADOW_BREAKUP, from `thin_wgsl`): its own distance times 0.65-1.35 by a
// hash of its place, so the levels' edges scatter over a band instead of
// drawing a line of changed texture across the view. The draw distance and the
// thinning keep the true distance. A function of the instance and the eye only.
fn pr_lod_dist(i: PrInstance, d: f32) -> f32 {
    if (!PR_MEADOW_BREAKUP || i.model < PR_VEG_BASE + PR_VEG_MEADOW || i.model >= PR_VEG_BASE + PR_VEG_COUNT) {
        return d;
    }
    let h = fract(sin(dot(floor(i.pos.xz * 4.0), vec2<f32>(12.9898, 78.233))) * 43758.5453);
    return d * (0.65 + 0.7 * h);
}

fn pr_veg_scale(i: PrInstance) -> f32 {
    if (!pr_is_veg(i.model)) {
        return 1.0;
    }
    let t = pr_veg_thin(i.model);
    if (t.x <= 0.0) {
        return 1.0;
    }
    let d = distance(i.pos, pr.cam.xyz);
    let f = clamp((d - t.x) / max(t.y - t.x, 1.0), 0.0, 1.0);
    if (f <= 0.0) {
        return 1.0;
    }
    let keep = mix(1.0, PR_VEG_FLOOR, f);
    let s = 1.0 - smoothstep(keep, keep + 0.06, i.slope);
    return s * min(inverseSqrt(keep), t.z);
}

fn pr_local(i: PrInstance, p: vec3<f32>) -> vec3<f32> {
    var l = p * i.scale * pr_veg_scale(i);
    if (i.model == PR_TACTILE_PAD) {
        // The pad takes the ramp's running slope along its depth.
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

// the ground's gradient under a vegetation instance (dy/dx, dy/dz; its `pad`
// word, two snorm16 of gradient / 4), zero for everything else and for the
// trees (a trunk stands plumb): grass, scrub and rock follow the slope they
// stand on by a shear (y += gradient . horizontal offset).
fn pr_veg_grad(i: PrInstance) -> vec2<f32> {
    if (!pr_is_veg(i.model) || i.model < PR_VEG_BASE + PR_VEG_TREES) {
        return vec2<f32>(0.0);
    }
    return unpack2x16snorm(i.pad) * 4.0;
}

fn pr_world(i: PrInstance, p: vec3<f32>, w: f32) -> vec3<f32> {
    let r = pr_rot(i.yaw, pr_local(i, p));
    var o = i.pos + r + pr_wind(i, p, w);
    o.y = o.y + dot(pr_veg_grad(i), r.xz);
    return o;
}
