// building the hierarchical Z from the early pass's depth.
//
// Level 0 is half the depth target, padded to a power of two so every level
// is exactly half the one below it; each texel is the minimum (reverse Z: the
// farthest) depth of the 2 x 2 texels (or depth pixels, every MSAA sample)
// under it. Texels past the depth target's edge hold 1.0 ("no pixel"), which
// the minimum ignores, so the padding never makes a texel nearer than the
// pixels it covers.

@group(0) @binding(1) var hzb_out: texture_storage_2d<r32float, write>;

#ifdef_single
@group(0) @binding(0) var hzb_depth: texture_depth_2d;

@compute @workgroup_size(8, 8)
fn hzb_first(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(hzb_out);
    if (gid.x >= dims.x || gid.y >= dims.y) {
        return;
    }
    let sd = textureDimensions(hzb_depth);
    var m = 1.0;
    for (var k = 0u; k < 4u; k = k + 1u) {
        let q = gid.xy * 2u + vec2<u32>(k & 1u, k >> 1u);
        if (q.x < sd.x && q.y < sd.y) {
            m = min(m, textureLoad(hzb_depth, q, 0));
        }
    }
    textureStore(hzb_out, gid.xy, vec4<f32>(m, 0.0, 0.0, 0.0));
}
#endif_single

#ifdef_msaa
@group(0) @binding(2) var hzb_depth_ms: texture_depth_multisampled_2d;

@compute @workgroup_size(8, 8)
fn hzb_first(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(hzb_out);
    if (gid.x >= dims.x || gid.y >= dims.y) {
        return;
    }
    let sd = textureDimensions(hzb_depth_ms);
    let n = textureNumSamples(hzb_depth_ms);
    var m = 1.0;
    for (var k = 0u; k < 4u; k = k + 1u) {
        let q = gid.xy * 2u + vec2<u32>(k & 1u, k >> 1u);
        if (q.x < sd.x && q.y < sd.y) {
            for (var s = 0u; s < n; s = s + 1u) {
                m = min(m, textureLoad(hzb_depth_ms, q, i32(s)));
            }
        }
    }
    textureStore(hzb_out, gid.xy, vec4<f32>(m, 0.0, 0.0, 0.0));
}
#endif_msaa

@group(0) @binding(3) var hzb_src: texture_2d<f32>;

@compute @workgroup_size(8, 8)
fn hzb_down(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(hzb_out);
    if (gid.x >= dims.x || gid.y >= dims.y) {
        return;
    }
    let p = gid.xy * 2u;
    let m = min(
        min(textureLoad(hzb_src, p, 0).r, textureLoad(hzb_src, p + vec2<u32>(1u, 0u), 0).r),
        min(textureLoad(hzb_src, p + vec2<u32>(0u, 1u), 0).r, textureLoad(hzb_src, p + vec2<u32>(1u, 1u), 0).r),
    );
    textureStore(hzb_out, gid.xy, vec4<f32>(m, 0.0, 0.0, 0.0));
}
