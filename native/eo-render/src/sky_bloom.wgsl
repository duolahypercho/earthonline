// bloom (appended to look_common.wgsl). The JS post chain's bloom
// (post-chain.js with three's BloomNode): the bright pass
// `texel x smoothstep(T, T + 0.55 T, luma)` with T re-solved each frame
// (2.101 / exposure: the display knee 0.965 on the tone curve), the source
// ceiling (each tap held to 4 T so the sun disc cannot flood it), five
// levels from half resolution down, composited with the JS weights
// (strength 0.26 x mix(f, 1.2 - f, radius 0.22), f = 1.0 .. 0.2). The
// pyramid is a 13-tap downsample and a tent upsample (Jimenez 2014) instead
// of three's separable Gaussians: the same footprint per level for a fraction
// of the taps.

@group(0) @binding(0) var<uniform> skyu: Sky;
@group(0) @binding(1) var bl_src: texture_2d<f32>;
@group(0) @binding(2) var bl_add: texture_2d<f32>;
@group(0) @binding(3) var bl_lin: sampler;
@group(0) @binding(4) var bl_dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(5) var<uniform> bl_level: vec4<f32>;

fn bl_tap(uv: vec2<f32>, ceiling: bool) -> vec3<f32> {
    var c = max(textureSampleLevel(bl_src, bl_lin, uv, 0.0).rgb, vec3<f32>(0.0));
    if (any(c != c)) {
        c = vec3<f32>(0.0);
    }
    if (ceiling) {
        let cap = skyu.bloom.x * skyu.bloom_w2.y;
        let m = max(c.r, max(c.g, c.b));
        c = c * (cap / max(m, cap));
    }
    return c;
}

fn bl_down13(uv: vec2<f32>, t: vec2<f32>, ceiling: bool) -> vec3<f32> {
    let a = bl_tap(uv + t * vec2<f32>(-2.0, -2.0), ceiling);
    let b = bl_tap(uv + t * vec2<f32>(0.0, -2.0), ceiling);
    let c = bl_tap(uv + t * vec2<f32>(2.0, -2.0), ceiling);
    let d = bl_tap(uv + t * vec2<f32>(-2.0, 0.0), ceiling);
    let e = bl_tap(uv, ceiling);
    let f = bl_tap(uv + t * vec2<f32>(2.0, 0.0), ceiling);
    let g = bl_tap(uv + t * vec2<f32>(-2.0, 2.0), ceiling);
    let h = bl_tap(uv + t * vec2<f32>(0.0, 2.0), ceiling);
    let i = bl_tap(uv + t * vec2<f32>(2.0, 2.0), ceiling);
    let j = bl_tap(uv + t * vec2<f32>(-1.0, -1.0), ceiling);
    let k = bl_tap(uv + t * vec2<f32>(1.0, -1.0), ceiling);
    let l = bl_tap(uv + t * vec2<f32>(-1.0, 1.0), ceiling);
    let m = bl_tap(uv + t * vec2<f32>(1.0, 1.0), ceiling);
    return e * 0.125 + (a + c + g + i) * 0.03125 + (b + d + f + h) * 0.0625 + (j + k + l + m) * 0.125;
}

/// (`--cine-camera`) a tap's share the lens spreads (cine_post.rs `cam_bloom_part`).
fn bl_cam_tap(uv: vec2<f32>, e: f32) -> vec3<f32> {
    var c = max(textureSampleLevel(bl_src, bl_lin, uv, 0.0).rgb, vec3<f32>(0.0));
    if (any(c != c)) {
        c = vec3<f32>(0.0);
    }
    return cam_bloom_part(c, e);
}

/// (`--cine-camera`) the 13-tap downsample of the taps' shares (each tap mapped
/// before it is averaged: a glint keeps its own share, it does not lend it to its
/// neighbours).
fn bl_cam_down13(uv: vec2<f32>, t: vec2<f32>, e: f32) -> vec3<f32> {
    let a = bl_cam_tap(uv + t * vec2<f32>(-2.0, -2.0), e);
    let b = bl_cam_tap(uv + t * vec2<f32>(0.0, -2.0), e);
    let c = bl_cam_tap(uv + t * vec2<f32>(2.0, -2.0), e);
    let d = bl_cam_tap(uv + t * vec2<f32>(-2.0, 0.0), e);
    let f = bl_cam_tap(uv + t * vec2<f32>(2.0, 0.0), e);
    let g = bl_cam_tap(uv + t * vec2<f32>(-2.0, 2.0), e);
    let h = bl_cam_tap(uv + t * vec2<f32>(0.0, 2.0), e);
    let i = bl_cam_tap(uv + t * vec2<f32>(2.0, 2.0), e);
    let j = bl_cam_tap(uv + t * vec2<f32>(-1.0, -1.0), e);
    let k = bl_cam_tap(uv + t * vec2<f32>(1.0, -1.0), e);
    let l = bl_cam_tap(uv + t * vec2<f32>(-1.0, 1.0), e);
    let m = bl_cam_tap(uv + t * vec2<f32>(1.0, 1.0), e);
    return bl_cam_tap(uv, e) * 0.125 + (a + c + g + i) * 0.03125 + (b + d + f + h) * 0.0625 + (j + k + l + m) * 0.125;
}

/// The first level: the source frame's bright pass at half resolution.
@compute @workgroup_size(8, 8)
fn bloom_prefilter(@builtin(global_invocation_id) gid: vec3<u32>) {
    let od = textureDimensions(bl_dst);
    if (gid.x >= od.x || gid.y >= od.y) {
        return;
    }
    let uv = (vec2<f32>(gid.xy) + 0.5) / vec2<f32>(od);
    let t = 1.0 / vec2<f32>(textureDimensions(bl_src));
    // (`--cine-camera`, compiled out without it) threshold-free: every pixel's share the
    // lens spreads at the hour's exposure (`bl_level.z`, 0 with the model off); the levels'
    // weights sum to 1 (cine_post.rs `CAM_BLOOM_WEIGHTS`).
    if (SF_CAMERA && bl_level.z > 0.0) {
        textureStore(bl_dst, vec2<i32>(gid.xy), vec4<f32>(bl_cam_down13(uv, t, bl_level.z), 1.0));
        return;
    }
    var c = bl_down13(uv, t, true);
    let th = skyu.bloom.x;
    // the glare from a source is capped at 16 x the threshold: the sun's disc
    // (thousands of times the sky) spread a ~300 px halo that buried its own edge and
    // lit what stood in front of it; the disc itself still clips in the frame.
    let cl = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    c = c * min(1.0, 16.0 * th / max(cl, 1e-6));
    let w = smoothstep(th, th + skyu.bloom.y * th, dot(c, vec3<f32>(0.2126, 0.7152, 0.0722)));
    textureStore(bl_dst, vec2<i32>(gid.xy), vec4<f32>(c * w, 1.0));
}

@compute @workgroup_size(8, 8)
fn bloom_down(@builtin(global_invocation_id) gid: vec3<u32>) {
    let od = textureDimensions(bl_dst);
    if (gid.x >= od.x || gid.y >= od.y) {
        return;
    }
    let uv = (vec2<f32>(gid.xy) + 0.5) / vec2<f32>(od);
    let t = 1.0 / vec2<f32>(textureDimensions(bl_src));
    textureStore(bl_dst, vec2<i32>(gid.xy), vec4<f32>(bl_down13(uv, t, false), 1.0));
}

/// up_i = level_i x weight_i + tent(up_(i+1)); `bl_level.x` the weight.
@compute @workgroup_size(8, 8)
fn bloom_up(@builtin(global_invocation_id) gid: vec3<u32>) {
    let od = textureDimensions(bl_dst);
    if (gid.x >= od.x || gid.y >= od.y) {
        return;
    }
    let uv = (vec2<f32>(gid.xy) + 0.5) / vec2<f32>(od);
    let t = 1.0 / vec2<f32>(textureDimensions(bl_src));
    var s = bl_tap(uv, false) * 4.0;
    s = s + (bl_tap(uv + vec2<f32>(t.x, 0.0), false) + bl_tap(uv - vec2<f32>(t.x, 0.0), false) + bl_tap(uv + vec2<f32>(0.0, t.y), false) + bl_tap(uv - vec2<f32>(0.0, t.y), false)) * 2.0;
    s = s + bl_tap(uv + t, false) + bl_tap(uv - t, false) + bl_tap(uv + vec2<f32>(t.x, -t.y), false) + bl_tap(uv + vec2<f32>(-t.x, t.y), false);
    s = s / 16.0 * bl_level.y;
    let lv = textureSampleLevel(bl_add, bl_lin, uv, 0.0).rgb * bl_level.x;
    textureStore(bl_dst, vec2<i32>(gid.xy), vec4<f32>(lv + s, 1.0));
}
