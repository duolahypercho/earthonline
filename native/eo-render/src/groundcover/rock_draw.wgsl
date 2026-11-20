// The scanned rock in the city pass (appended to draw.wgsl + the look's
// lighting via `look::city_shader_source`, and rock_types.wgsl). Group 0
// has draw.wgsl's layout with the scans' three texture arrays in the kit's
// slots (layer = scan); group 1 is the look's; group 2 the pieces. Each
// piece is its scan turned, scaled and placed; its albedo the scan's
// diffuse map times its tint, darkened and glossed where it is wet; its
// occlusion and roughness the scan's ARM map; its normal the scan's
// OpenGL-convention normal map (two channels, z rebuilt) on a tangent
// frame from the derivatives (as the props' glTF maps). Open shells (faces,
// patches) are seen from both sides, their inner side shaded as the hollow
// it is; their cut rim sinks and frays (rock_types.wgsl). Lit by the city's
// own `look_light` (sun with the cascaded shadow, sky, AO, fog).

@group(2) @binding(0) var<storage, read> gc_rocks: array<GcRock>;
@group(2) @binding(1) var<storage, read> gc_visible: array<u32>;
@group(2) @binding(2) var<uniform> gc_kinds: array<GcKind, 8>;

// Wet rock: the albedo over the dry (Lekner and Dorf 1988, the mineral
// surface's measured ratio, ground_cover `WET_SAND_RATIO`) and the wet
// rock's roughness (INFERRED: a film over rough rock, glossier than dry but
// not a mirror; a glossier film glitters on the scans' fine normal detail).
const GC_WET_RATIO: f32 = 0.5;
const GC_WET_ROUGH: f32 = 0.5;
// The swash's film on a boulder wet through (INFERRED: the photograph's sheen on the
// boulders in the surf; glossier than the damp film above it).
const GC_SHEEN_ROUGH: f32 = 0.28;
// An open shell's inner side: the hollow under a scanned face, which in
// the ground it came from is never seen; drawn at this share of its albedo
// so a piece turned to show it does not read as a bleached lit face
// (INFERRED).
const GC_INNER_SHADE: f32 = 0.35;

struct GcOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) local: vec3<f32>,
    @location(4) @interpolate(flat) inst: u32,
    @location(5) rim: f32,
};

@vertex
fn rock_vs(
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) uv: vec2<f32>,
    @builtin(instance_index) ii: u32,
) -> GcOut {
    let idx = gc_visible[ii];
    let r = gc_rocks[idx];
    let k = gc_kinds[r.kind];
    let rim = gc_rim_dist(k, normal.w);
    let world = r.pos + gc_rotate(r.rot, gc_rim_sink(k, pos, rim) * r.scale);
    var out: GcOut;
    out.clip = frame.view_proj * vec4<f32>(world, 1.0);
    out.world = world;
    out.normal = gc_rotate(r.rot, normal.xyz / max(r.scale, vec3<f32>(1e-3)));
    out.uv = uv;
    out.local = pos;
    out.inst = idx;
    out.rim = rim;
    return out;
}

fn gc_shade(in: GcOut, front: bool) -> vec4<f32> {
    let r = gc_rocks[in.inst];
    let layer = i32(r.kind);
    let dpx = dpdx(in.world);
    let dpy = dpdy(in.world);
    let duvx = dpdx(in.uv);
    let duvy = dpdy(in.uv);
    let v = normalize(frame.cam_pos.xyz - in.world);
    var n = normalize(in.normal);
    if (!front) {
        n = -n;
    }
    let ng = n;
    let albedo = textureSampleGrad(kit_albedo, kit_sampler, in.uv, layer, duvx, duvy).rgb;
    let arm = textureSampleGrad(kit_orm, kit_sampler, in.uv, layer, duvx, duvy).rgb;
    let nxy = textureSampleGrad(kit_normal, kit_sampler, in.uv, layer, duvx, duvy).xy * 2.0 - 1.0;
    let nt = vec3<f32>(nxy, sqrt(max(1.0 - dot(nxy, nxy), 0.0)));
    let r1 = cross(dpy, n);
    let r2 = cross(n, dpx);
    let t = r1 * duvx.x + r2 * duvy.x;
    let b = r1 * duvx.y + r2 * duvy.y;
    let inv = inverseSqrt(max(max(dot(t, t), dot(b, b)), 1e-20));
    var nm = normalize(t * inv * nt.x - b * inv * nt.y + n * max(nt.z, 0.05));
    if (!all(nm == nm)) {
        nm = n;
    }
    var s: Surface;
    let wet = clamp(r.wet, 0.0, 1.0);
    // past 1: wet through in the swash with a glossier film (the second flora pass's
    // boulders, eo_landmarks::flora2 `boulder_wet`; no other piece passes 1).
    let sheen = clamp(r.wet - 1.0, 0.0, 1.0);
    s.albedo = albedo * r.tint * mix(1.0, GC_WET_RATIO, wet) * select(GC_INNER_SHADE, 1.0, front);
    s.rough = clamp(mix(mix(arm.g, GC_WET_ROUGH, wet), GC_SHEEN_ROUGH, sheen), 0.05, 1.0);
    s.metal = 0.0;
    s.ao = arm.r;
    s.n = nm;
    s.alpha = 1.0;
    s.env = 0;
    var m: Material;
    m.base = vec4<f32>(1.0);
    m.emissive = vec4<f32>(0.0);
    m.params = vec4<f32>(s.rough, 0.0, 1.0, 0.0);
    m.layer = 0;
    m.flags = 0u;
    m.rough_flatten = 0.0;
    m.thin_absorption = 0.0;
    return look_light(s, m, v, in.world, ng, 0u);
}

// a scanned patch's own ground: cut away under its clip height over its
// rim plane; and any open scan's cut rim frayed.
fn gc_clip(in: GcOut) {
    let kk = gc_kinds[gc_rocks[in.inst].kind];
    let k = kk.rim;
    let h = in.local.y - (k.x * in.local.x + k.y * in.local.z + k.z);
    if (h < k.w || gc_frayed(kk, in.local, in.rim)) {
        discard;
    }
}

@fragment
fn rock_fs(in: GcOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    return gc_shade(in, front);
}

@fragment
fn rock_fs_clip(in: GcOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    gc_clip(in);
    return gc_shade(in, front);
}

struct GcIdsOut {
    @location(0) color: vec4<f32>,
    @location(1) ids: vec2<u32>,
};

@fragment
fn rock_fs_ids(in: GcOut, @builtin(front_facing) front: bool) -> GcIdsOut {
    var out: GcIdsOut;
    out.color = gc_shade(in, front);
    out.ids = vec2<u32>(0xfffffffcu, gc_rocks[in.inst].kind);
    return out;
}

@fragment
fn rock_fs_clip_ids(in: GcOut, @builtin(front_facing) front: bool) -> GcIdsOut {
    gc_clip(in);
    var out: GcIdsOut;
    out.color = gc_shade(in, front);
    out.ids = vec2<u32>(0xfffffffcu, gc_rocks[in.inst].kind);
    return out;
}
