// (ppr) the frame's geometry projected across the sea plane (water.rs
// FEAT_PPR): pixel-projected reflections (Cichocki 2017, "Optimized
// pixel-projected reflections for planar reflectors", SIGGRAPH Advances in
// Real-Time Rendering). Every pixel of the opaque frame that stands above the
// still sea is mirrored through it (x, 2 sea - y, z) and projected; where its
// mirror image lands, the pixel is written as the source of the reflection
// there, the nearest mirror image winning (one atomic max of a packed key per
// target: the mirrored point's log view depth inverted in the high 11 bits,
// the offset from the target to its source in the low 21). The water's
// fragments read the source a reflected ray lands on (water.wgsl `w_ppr_*`),
// so a thin bridge deck, its cables and a skyline stand in the water wherever
// the frame shows them, with no march to step over them.
//
// Only what the mirror can see is written: a surface whose normal (from the
// depth buffer's neighbours) faces away from the mirrored eye (a roof, a
// deck's top, a flat beach seen from below the sea) is not in the mirror;
// whatever lies at or under the sea (the seabed, the water's own foot) is not
// either.

@group(0) @binding(0) var<uniform> wu: WaterU;
@group(0) @binding(1) var p_depth: texture_depth_2d;
@group(0) @binding(2) var<storage, read_write> p_buf: array<atomic<u32>>;

// the world point at a pixel, with the far layer's depth form (1e-9 / view
// depth past the far plane, eo-farshore DEPTH_K); w 0 for the sky
fn pp_world(px: vec2<i32>, dims: vec2<i32>) -> vec4<f32> {
    let q = clamp(px, vec2<i32>(0), dims - 1);
    let d = textureLoad(p_depth, q, 0);
    if (d <= 0.0) {
        return vec4<f32>(0.0);
    }
    let uv = (vec2<f32>(q) + 0.5) / vec2<f32>(dims);
    let w = wu.inv_view_proj * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    var p = w.xyz / w.w;
    let rel = p - wu.cam.xyz;
    let vd = dot(rel, wu.fwd.xyz);
    let far_vd = 1e-9 / d;
    if (far_vd > vd * 1.001) {
        let dir = normalize(rel);
        p = wu.cam.xyz + dir * (far_vd / max(dot(dir, wu.fwd.xyz), 1e-3));
    }
    return vec4<f32>(p, 1.0);
}

@compute @workgroup_size(8, 8)
fn ppr_project(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = vec2<i32>(textureDimensions(p_depth));
    let px = vec2<i32>(id.xy);
    if (px.x >= dims.x || px.y >= dims.y) {
        return;
    }
    let pw = pp_world(px, dims);
    if (pw.w <= 0.0) {
        return;
    }
    let p = pw.xyz;
    let sea = wu.grid.z;
    let h = p.y - sea;
    if (h < 0.05) {
        return;
    }
    // the surface's normal from its neighbours (the nearer of each pair, so an edge takes its own side)
    let pr = pp_world(px + vec2<i32>(1, 0), dims);
    let pl = pp_world(px - vec2<i32>(1, 0), dims);
    let pd = pp_world(px + vec2<i32>(0, 1), dims);
    let pu = pp_world(px - vec2<i32>(0, 1), dims);
    var dx = pr.xyz - p;
    if (pr.w <= 0.0 || (pl.w > 0.0 && length(p - pl.xyz) < length(dx))) {
        dx = p - pl.xyz;
    }
    var dy = pd.xyz - p;
    if (pd.w <= 0.0 || (pu.w > 0.0 && length(p - pu.xyz) < length(dy))) {
        dy = p - pu.xyz;
    }
    var n = cross(dx, dy);
    let nl = length(n);
    if (nl > 1e-12) {
        n = n / nl;
        // (toward the eye: the frame sees this face)
        if (dot(n, wu.cam.xyz - p) < 0.0) {
            n = -n;
        }
        // the mirrored eye under the sea must see the face too
        let cm = vec3<f32>(wu.cam.x, 2.0 * sea - wu.cam.y, wu.cam.z);
        if (dot(n, normalize(cm - p)) < 0.02) {
            return;
        }
    }
    let pm = vec3<f32>(p.x, sea - h, p.z);
    let c = wu.view_proj * vec4<f32>(pm, 1.0);
    if (c.w <= 0.1) {
        return;
    }
    let uv = vec2<f32>(c.x / c.w * 0.5 + 0.5, 0.5 - c.y / c.w * 0.5);
    if (any(uv < vec2<f32>(0.0)) || any(uv >= vec2<f32>(1.0))) {
        return;
    }
    let q = vec2<i32>(uv * vec2<f32>(dims));
    let ox = px.x - q.x;
    let oy = q.y - px.y; // the source stands above its mirror image
    if (oy < 0 || oy > 2047 || abs(ox) > 511) {
        return;
    }
    // the mirrored point's view depth, log2 over 0.25 m .. 64 km in 2047 steps, nearer larger
    let lk = clamp(log2(max(c.w, 0.25) / 0.25) * (2047.0 / 18.0), 0.0, 2046.0);
    let key = 2047u - u32(lk);
    let v = (key << 21u) | (u32(oy) << 10u) | u32(ox + 512);
    atomicMax(&p_buf[u32(q.y) * u32(dims.x) + u32(q.x)], v);
}
