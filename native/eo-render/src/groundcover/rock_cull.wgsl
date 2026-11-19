// The scanned rock's cull and level of detail (appended to
// rock_types.wgsl; the Hi-Z variant also to occlusion.wgsl). One
// invocation per placed piece: for each view of the run (bit 0 the camera,
// bits 1-3 the sun cascades) the piece's bounding sphere against the view's
// frustum, its level by its projected radius in pixels (and, for the
// camera with occlusion on, its box against this frame's depth pyramid);
// then a slot in its (view, scan, level) bucket, whose indirect draw's
// instance count is the bucket's atomic counter and whose first instance is
// the bucket's base in the visible list. A cascade draws a piece the camera
// would draw (within the cast distance) one level coarser.

struct GcParams {
    planes: array<vec4<f32>, 6>,
    // the eye, and pixels per unit of tan (half the target's height over tan(fov / 2))
    cam: vec4<f32>,
    view_proj: mat4x4<f32>,
    // x pieces, y views to classify (bits), z the stats' on bit
    counts: vec4<u32>,
    // level edges in projected radius (pixels): level 0 over x, 1 over y, 2 over z; nothing under w
    lod: vec4<f32>,
    // x the farthest drawn (m), y the farthest casting (m), z the cascades' margin (m)
    dist: vec4<f32>,
};

struct GcShadowViewPadded {
    view_proj: mat4x4<f32>,
    copy_step: vec4<f32>,
    pad: array<vec4<f32>, 11>,
};

@group(0) @binding(0) var<uniform> gc: GcParams;
@group(0) @binding(1) var<storage, read> gc_rocks: array<GcRock>;
// 5 words per (view, scan, level): index count, instance count, first index, base vertex, first instance
@group(0) @binding(2) var<storage, read_write> gc_args: array<atomic<u32>>;
@group(0) @binding(3) var<storage, read_write> gc_visible: array<u32>;
@group(0) @binding(4) var<uniform> gc_cascades: array<GcShadowViewPadded, 3>;
// per view: pieces drawn, triangles drawn
@group(0) @binding(5) var<storage, read_write> gc_stats: array<atomic<u32>, 8>;
// triangles per (scan, level)
@group(0) @binding(6) var<storage, read> gc_tris: array<u32>;

fn gc_in_planes(planes: array<vec4<f32>, 6>, c: vec3<f32>, r: f32) -> bool {
    for (var p = 0u; p < 6u; p = p + 1u) {
        let pl = planes[p];
        let l = length(pl.xyz);
        if (l > 1e-12 && dot(pl.xyz, c) + pl.w < -r * l) {
            return false;
        }
    }
    return true;
}

fn gc_cascade_planes(m: mat4x4<f32>) -> array<vec4<f32>, 6> {
    let r0 = vec4<f32>(m[0].x, m[1].x, m[2].x, m[3].x);
    let r1 = vec4<f32>(m[0].y, m[1].y, m[2].y, m[3].y);
    let r2 = vec4<f32>(m[0].z, m[1].z, m[2].z, m[3].z);
    let r3 = vec4<f32>(m[0].w, m[1].w, m[2].w, m[3].w);
    return array<vec4<f32>, 6>(r3 + r0, r3 - r0, r3 + r1, r3 - r1, r2, r3 - r2);
}

fn gc_push(view: u32, kind: u32, level: u32, i: u32) {
    let b = (view * GC_KINDS + kind) * GC_LODS + level;
    let slot = atomicAdd(&gc_args[b * 5u + 1u], 1u);
    let base = atomicLoad(&gc_args[b * 5u + 4u]);
    gc_visible[base + slot] = i;
    if (gc.counts.z != 0u) {
        atomicAdd(&gc_stats[view * 2u], 1u);
        atomicAdd(&gc_stats[view * 2u + 1u], gc_tris[kind * GC_LODS + level]);
    }
}

@compute @workgroup_size(64)
fn classify(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i >= gc.counts.x) {
        return;
    }
    let r = gc_rocks[i];
    let d = distance(r.center, gc.cam.xyz);
    if (d > gc.dist.x + r.radius) {
        return;
    }
    let px = r.radius * gc.cam.w / max(d - r.radius * 0.5, 0.5);
    if (px < gc.lod.w) {
        return;
    }
    var level = 3u;
    if (px > gc.lod.x) {
        level = 0u;
    } else if (px > gc.lod.y) {
        level = 1u;
    } else if (px > gc.lod.z) {
        level = 2u;
    }
    let views = gc.counts.y;
    if ((views & 1u) != 0u && gc_in_planes(gc.planes, r.center, r.radius)) {
        let e = vec3<f32>(r.radius);
        if (!gc_occluded(r.center - e, r.center + e)) {
            gc_push(0u, r.kind, level, i);
        }
    }
    if (d > gc.dist.y) {
        return;
    }
    let cast_level = min(level + 1u, 3u);
    for (var c = 0u; c < 3u; c = c + 1u) {
        if ((views & (2u << c)) != 0u && gc_in_planes(gc_cascade_planes(gc_cascades[c].view_proj), r.center, r.radius + gc.dist.z)) {
            gc_push(1u + c, r.kind, cast_level, i);
        }
    }
}
