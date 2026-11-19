// The scanned rock into a sun cascade (appended to rock_types.wgsl).
// Group 0: the look's cascade view (dynamic offset); group 2 the pieces.
// Depth only; a scanned patch's own ground is cut away, and an open scan's
// rim sunk and frayed, as in the colour pass.

struct GcShadowView {
    view_proj: mat4x4<f32>,
    copy_step: vec4<f32>,
};

@group(0) @binding(0) var<uniform> gc_sv: GcShadowView;
@group(2) @binding(0) var<storage, read> gc_rocks: array<GcRock>;
@group(2) @binding(1) var<storage, read> gc_visible: array<u32>;
@group(2) @binding(2) var<uniform> gc_kinds: array<GcKind, 8>;

struct GcShadowOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) local: vec3<f32>,
    @location(1) @interpolate(flat) kind: u32,
    @location(2) rim: f32,
};

@vertex
fn rock_shadow_vs(@location(0) pos: vec3<f32>, @location(1) normal: vec4<f32>, @builtin(instance_index) ii: u32) -> GcShadowOut {
    let r = gc_rocks[gc_visible[ii]];
    let k = gc_kinds[r.kind];
    let rim = gc_rim_dist(k, normal.w);
    var out: GcShadowOut;
    out.clip = gc_sv.view_proj * vec4<f32>(r.pos + gc_rotate(r.rot, gc_rim_sink(k, pos, rim) * r.scale), 1.0);
    out.local = pos;
    out.kind = r.kind;
    out.rim = rim;
    return out;
}

@fragment
fn rock_shadow_clip_fs(in: GcShadowOut) {
    let kk = gc_kinds[in.kind];
    let k = kk.rim;
    if (in.local.y - (k.x * in.local.x + k.y * in.local.z + k.z) < k.w || gc_frayed(kk, in.local, in.rim)) {
        discard;
    }
}
