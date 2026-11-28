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
const PR_TREE: u32 = 6u;
const PR_PALM: u32 = 7u;
const PR_TACTILE_PAD: u32 = 8u;
// eo-props `Model::TreeBasin` (`--tree-basins`): tilted along the kerb (its local +Z) as the pad is along its depth.
const PR_TREE_BASIN: u32 = 35u;
// ez-tree species range (props_lib2.rs `TREE_BASE..OH_BASE`).
const PR_TREE_BASE: u32 = 36u;
const PR_OH_BASE: u32 = 68u;
// props2 features (PrParams.mode.z).
const PR_F_FADE: u32 = 1u;
const PR_F_WIND: u32 = 2u;
const PR_F_LEAF: u32 = 4u;
const PR_F_DOMES: u32 = 8u;
const PR_F_SIGNS: u32 = 16u;
const PR_F_WEATHER_WIND: u32 = 32u;
// Boats afloat (eo-app `--no-boat-motion` clears it): an instance whose `pad` has
// PR_AFLOAT_BIT carries its hull's roll and pitch on the sea, 1e-4 rad each: roll
// in bits 0..15, pitch in bits 16..30, both signed (eo-props `boats::afloat_word`;
// the pose is the physics', eo-player `harbour`). Its heave is in its height.
const PR_F_AFLOAT: u32 = 64u;
// (eo-app `--no-retro-viewer`) an arm name sign's face takes no glow of its own at night
const PR_F_RETRO_VIEWER: u32 = 128u;
// (eo-app `--no-sign-back-aged`) an arm name sign's back is dull weathered aluminium
const PR_F_SIGN_BACK_AGED: u32 = 256u;
const PR_AFLOAT_BIT: u32 = 0x80000000u;
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

// How much of its true distance an instance's draw distance and thinning
// count (its levels keep the true one): PR_YARD_REACH for a yard plant (its `variant` carries
// PR_YARD_PLANT, vegetation_models.rs `YARD_PLANT_BIT`), 1 for everything else.
fn pr_reach(i: PrInstance) -> f32 {
    return select(1.0, PR_YARD_REACH, pr_is_veg(i.model) && (i.variant & PR_YARD_PLANT) != 0u);
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
    let d = distance(i.pos, pr.cam.xyz) * pr_reach(i);
    let f = clamp((d - t.x) / max(t.y - t.x, 1.0), 0.0, 1.0);
    if (f <= 0.0) {
        return 1.0;
    }
    let keep = mix(1.0, PR_VEG_FLOOR, f);
    let s = 1.0 - smoothstep(keep, keep + 0.06, i.slope);
    return s * min(inverseSqrt(keep), t.z);
}
fn pr_afloat_on(i: PrInstance) -> bool {
    // Vegetation stores a signed terrain gradient in this same word. Its
    // sign bit must never roll a rock or shrub as though it were a hull.
    return i.model >= PR_BOAT_START && i.model < PR_BOAT_END
        && (pr.mode.z & PR_F_AFLOAT) != 0u && (i.pad & PR_AFLOAT_BIT) != 0u;
}

// A moored boat's pose on the sea (x roll about its bow axis, local +Z; y pitch
// about its beam, local +X; radians), as the instance's word carries it.
fn pr_afloat(i: PrInstance) -> vec3<f32> {
    let r = i32(i.pad & 0xffffu);
    let p = i32((i.pad >> 16u) & 0x7fffu);
    let roll = f32(select(r, r - 65536, r >= 32768)) * 1e-4;
    let pitch = f32(select(p, p - 32768, p >= 16384)) * 1e-4;
    return vec3<f32>(roll, pitch, 0.0);
}

// a local direction or point turned by the boat's roll (about +Z), then its pitch (about +X).
fn pr_afloat_turn(a: vec3<f32>, v: vec3<f32>) -> vec3<f32> {
    let cr = cos(a.x);
    let sr = sin(a.x);
    let r = vec3<f32>(v.x * cr - v.y * sr, v.x * sr + v.y * cr, v.z);
    let cp = cos(a.y);
    let sp = sin(a.y);
    return vec3<f32>(r.x, r.y * cp - r.z * sp, r.y * sp + r.z * cp);
}

// a local normal as the instance turns it (a boat afloat: its roll and pitch).
fn pr_local_dir(i: PrInstance, n: vec3<f32>) -> vec3<f32> {
    if (pr_afloat_on(i)) {
        return pr_afloat_turn(pr_afloat(i), n);
    }
    return n;
}

fn pr_local(i: PrInstance, p: vec3<f32>) -> vec3<f32> {
    var l = p * i.scale * pr_veg_scale(i);
    if (pr_afloat_on(i)) {
        return pr_afloat_turn(pr_afloat(i), l);
    }
    if (i.model == PR_TACTILE_PAD || i.model == PR_TREE_BASIN || i.model == PR_YARD_FENCE) {
        // (a rear-yard fence piece: the ground's grade along its length, its boards plumb.)
        // The pad takes the ramp's running slope along its depth; a tree basin the street's grade along the kerb.
        l.y = l.y + l.z * i.slope;
    }
    return l;
}

// wind sway, height-weighted per vertex (`w`, baked by the library: 0 for
// everything but trees), a function of the instance position and the
// simulation clock only, so the colour and the shadow passes agree.
// Under `--weather-fx` (`PR_F_WEATHER_WIND`), trees and palms bend downwind
// with traveling gust fronts, cross-wind vortex oscillation, arc-length
// droop and faster leaf/twig flutter scaled by the live weather wind.
fn pr_wind(i: PrInstance, p: vec3<f32>, w_in: f32) -> vec3<f32> {
    if ((pr.mode.z & PR_F_WIND) == 0u) {
        return vec3<f32>(0.0);
    }
    let weather_wind = (pr.mode.z & PR_F_WEATHER_WIND) != 0u;
    var w = w_in;
    var is_palm = false;
    if (weather_wind && w <= 0.0 && i.model == PR_PALM) {
        let h = clamp(p.y / 10.0, 0.0, 1.0);
        let frond = smoothstep(0.62, 0.98, h) * clamp(length(p.xz) / 2.2, 0.0, 1.0);
        w = h * h * 0.85 + frond * 0.45;
        is_palm = true;
    }
    if (w <= 0.0) {
        return vec3<f32>(0.0);
    }
    let t = pr.lens.z;
    let phase = dot(i.pos.xz, vec2<f32>(0.071, 0.113)) + f32(i.variant & 255u) * 0.37;
    if (!weather_wind) {
        let gust = sin(t * 0.9 + phase) * 0.6 + sin(t * 2.1 + phase * 1.3) * 0.3;
        let flutter = sin(t * 5.3 + phase * 2.9 + (p.x + p.z) * 3.0) * 0.1 * w;
        let a = (gust + flutter) * w * pr.wind.z * i.scale.y;
        return vec3<f32>(pr.wind.x * a, -abs(a) * 0.05, pr.wind.y * a);
    }
    let wd = pr.wind.xy;
    let perp = vec2<f32>(-wd.y, wd.x);
    let ws = pr.wire.w; // 0 calm .. 1 storm
    // Spatially coherent traveling wind gust front sweeping down city blocks.
    let adv = dot(i.pos.xz, wd) * 0.042;
    let cross_p = dot(i.pos.xz, perp) * 0.029;
    let wave_spd = 1.15 + 2.35 * ws;
    let squall = 0.5 + 0.5 * sin(adv * 0.45 - t * (0.42 + 0.65 * ws) + cross_p * 0.7);
    let g1 = sin(adv - t * wave_spd + phase * 0.35);
    let g2 = sin(adv * 1.75 - t * (wave_spd * 1.95) + phase * 1.15);
    let gust_along = (0.30 + 0.58 * squall) * (0.55 * ws) + (g1 * 0.62 + g2 * 0.34) * (0.55 + 0.55 * squall);
    // Cross-wind vortex shedding so crowns trace organic 3D elliptical arcs.
    let sway_cross = (sin(t * (1.05 + 1.45 * ws) + phase * 1.7) * 0.36 + sin(t * (2.6 + 2.1 * ws) + phase * 2.3 + p.y * 0.4) * 0.16) * (0.35 + 0.65 * ws);
    // High-frequency branch & leaf flutter (faster in rain gusts & storms).
    let leaf_freq = 4.6 + 6.5 * ws;
    let f_arg = t * leaf_freq + phase * 2.9 + (p.x * 2.7 + p.z * 3.1 + p.y * 1.9);
    let flutter_a = sin(f_arg) * (0.12 + 0.24 * ws) * w;
    let flutter_c = cos(f_arg * 1.27 + 1.1) * (0.09 + 0.19 * ws) * w;
    let flutter_y = sin(f_arg * 1.63 + 2.4) * (0.04 + 0.10 * ws) * w * w;
    let palm_gain = select(1.0, 1.32, is_palm);
    let scale = w * pr.wind.z * i.scale.y * palm_gain;
    let disp_xz = wd * ((gust_along + flutter_a) * scale) + perp * ((sway_cross + flutter_c) * scale);
    // Arc-length-preserving droop as the trunk/crown bends sideways.
    let approx_h = max(p.y * i.scale.y, 2.5);
    let droop = -dot(disp_xz, disp_xz) / (2.0 * approx_h) + flutter_y * pr.wind.z * i.scale.y;
    return vec3<f32>(disp_xz.x, droop, disp_xz.y);
}

// the ground's gradient under a vegetation instance (dy/dx, dy/dz; its `pad`
// word, two snorm16 of gradient / 4), zero for everything else and for the
// trees (a trunk stands plumb): grass, scrub and rock follow the slope they
// stand on by a shear (y += gradient . horizontal offset).
fn pr_veg_grad(i: PrInstance) -> vec2<f32> {
    // (eo-app `--no-pull-boxes` off) the pull box's lid lies on the sidewalk the same way (its model in `extra.z`)
    if (i.model == pr.extra.z) {
        return unpack2x16snorm(i.pad) * 4.0;
    }
    if (!pr_is_veg(i.model) || i.model < PR_VEG_BASE + PR_VEG_TREES) {
        return vec2<f32>(0.0);
    }
    return unpack2x16snorm(i.pad) * 4.0;
}

// A tree's lean (the coastal trees; eo-app props/lidar_trees.rs `lean_of`): the
// horizontal shear per metre of its height, in the slope's slot (a tree takes no slope).
fn pr_tree_lean(i: PrInstance) -> vec2<f32> {
    if (!pr_is_veg(i.model) || i.model >= PR_VEG_BASE + PR_VEG_TREES) {
        return vec2<f32>(0.0);
    }
    return unpack2x16snorm(i.pad) * 4.0;
}

fn pr_world(i: PrInstance, p: vec3<f32>, w: f32) -> vec3<f32> {
    let r = pr_rot(i.yaw, pr_local(i, p));
    var o = i.pos + r + pr_wind(i, p, w);
    o.y = o.y + dot(pr_veg_grad(i), r.xz);
    let lean = pr_tree_lean(i);
    o.x = o.x + lean.x * r.y;
    o.z = o.z + lean.y * r.y;
    return o;
}

// Individual leaf-card wiggle, corner twist and raindrop-impact twitch (`--weather-fx`):
// anchored toward the leaf stem (`uv.y -> 1`) so leaf tips wiggle and flutter out of phase
// with adjacent clusters and briefly dip & rebound when heavy raindrops strike them.
fn pr_leaf_wiggle(i: PrInstance, p: vec3<f32>, uv: vec2<f32>, w: f32) -> vec3<f32> {
    if ((pr.mode.z & (PR_F_WIND | PR_F_WEATHER_WIND)) != (PR_F_WIND | PR_F_WEATHER_WIND) || w <= 0.0) {
        return vec3<f32>(0.0);
    }
    let t = pr.lens.z;
    let ws = pr.wire.w; // 0 calm .. 1 storm
    let rain = f32(pr.extra.y) * 0.001; // 0 dry .. 1 storm downpour
    let wd = pr.wind.xy;
    let perp = vec2<f32>(-wd.y, wd.x);
    let cluster = floor(p * 4.5);
    let seed = dot(cluster, vec3<f32>(12.9898, 78.233, 37.719)) + dot(i.pos.xz, vec2<f32>(0.19, 0.31));
    let h = fract(sin(seed) * 43758.5453);
    let h2 = fract(h * 17.31);
    let tip = clamp(1.0 - uv.y, 0.0, 1.0);
    let side = uv.x - 0.5;
    let edge = 0.25 + 0.75 * clamp(tip * 0.8 + abs(side) * 0.9, 0.0, 1.0);
    let freq = 6.2 + 9.5 * ws + 2.0 * h;
    let a1 = t * freq + h * 6.2831853 + dot(p, vec3<f32>(4.1, 3.3, 5.2));
    let a2 = t * (freq * 1.67) + h2 * 6.2831853 + side * 3.8;
    let amp = (0.018 + 0.068 * ws + 0.018 * rain) * edge * clamp(w, 0.2, 1.0) * max(i.scale.y, 0.7);
    let w_along = (sin(a1) * 0.65 + sin(a2) * 0.35) * amp;
    let w_cross = (cos(a1 * 1.19 + 0.8) * 0.60 + sin(a2 * 0.91 - side * 2.5) * 0.40) * amp;
    let w_twist_y = (sin(a2 + tip * 2.1) * 0.55 + cos(a1 * 0.85) * 0.25) * amp * 0.75;
    var drop_dip = 0.0;
    if (rain > 0.02) {
        let hit_rate = 4.2 + 5.8 * rain + 1.5 * h2;
        let pulse = pow(max(0.0, sin(t * hit_rate + h * 31.4159)), 14.0);
        drop_dip = -pulse * (0.014 + 0.036 * rain) * edge * clamp(w, 0.2, 1.0);
    }
    let d_xz = wd * w_along + perp * w_cross;
    return vec3<f32>(d_xz.x, w_twist_y + drop_dip, d_xz.y);
}

fn pr_world_leaf(i: PrInstance, p: vec3<f32>, uv: vec2<f32>, w: f32, is_leaf: bool) -> vec3<f32> {
    var world = pr_world(i, p, w);
    if (is_leaf) {
        world = world + pr_leaf_wiggle(i, p, uv, w);
    }
    return world;
}

// A crown core's leaf field (`--tree-crowns`; props_draw.wgsl `pr_crown_leaves`,
// `pr_crown_edge`, props_shadow.wgsl): value noise, 0..1, in the tree's own metres.
fn pr_crown_hash(p: vec3<f32>) -> f32 {
    return fract(sin(dot(p, vec3<f32>(127.1, 311.7, 74.7))) * 43758.5453);
}

fn pr_crown_noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(pr_crown_hash(i), pr_crown_hash(i + vec3<f32>(1.0, 0.0, 0.0)), u.x);
    let b = mix(pr_crown_hash(i + vec3<f32>(0.0, 1.0, 0.0)), pr_crown_hash(i + vec3<f32>(1.0, 1.0, 0.0)), u.x);
    let c = mix(pr_crown_hash(i + vec3<f32>(0.0, 0.0, 1.0)), pr_crown_hash(i + vec3<f32>(1.0, 0.0, 1.0)), u.x);
    let d = mix(pr_crown_hash(i + vec3<f32>(0.0, 1.0, 1.0)), pr_crown_hash(i + vec3<f32>(1.0, 1.0, 1.0)), u.x);
    return mix(mix(a, b, u.y), mix(c, d, u.y), u.z);
}
