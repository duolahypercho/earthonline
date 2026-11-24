// (`--screen-reflect`, the reflect track; sky_reflect.rs) The reflection
// source pyramid: the resolved history's own mip chain, built after the
// resolve, which the city pass reads next frame as its previous frame
// (sky_light.wgsl `sky_sr_cone`). Each level is the 13-tap mean of the level
// above (Jimenez 2014, as the bloom's downsample: a smooth, alias-free
// footprint about twice the level's texel), so a glossy lobe reads the frame
// prefiltered at its own width instead of scattering jittered taps over it
// (the speckle in tail-lamp and window reflections). The first level holds
// each tap to the bloom's source ceiling (threshold x ceiling, the sun's disc
// and nothing else), so a disc a few pixels wide cannot spread a glow
// through the coarse levels; the mean is otherwise energy-conserving (no
// Karis weights: a tail lamp keeps its power as it spreads).

@group(0) @binding(0) var<uniform> skyu: Sky;
@group(0) @binding(1) var rp_src: texture_2d<f32>;
@group(0) @binding(2) var rp_lin: sampler;
@group(0) @binding(3) var rp_dst: texture_storage_2d<rgba16float, write>;

fn rp_tap(uv: vec2<f32>, ceiling: bool) -> vec3<f32> {
    var c = max(textureSampleLevel(rp_src, rp_lin, uv, 0.0).rgb, vec3<f32>(0.0));
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

fn rp_down13(uv: vec2<f32>, t: vec2<f32>, ceiling: bool) -> vec3<f32> {
    let a = rp_tap(uv + t * vec2<f32>(-2.0, -2.0), ceiling);
    let b = rp_tap(uv + t * vec2<f32>(0.0, -2.0), ceiling);
    let c = rp_tap(uv + t * vec2<f32>(2.0, -2.0), ceiling);
    let d = rp_tap(uv + t * vec2<f32>(-2.0, 0.0), ceiling);
    let e = rp_tap(uv, ceiling);
    let f = rp_tap(uv + t * vec2<f32>(2.0, 0.0), ceiling);
    let g = rp_tap(uv + t * vec2<f32>(-2.0, 2.0), ceiling);
    let h = rp_tap(uv + t * vec2<f32>(0.0, 2.0), ceiling);
    let i = rp_tap(uv + t * vec2<f32>(2.0, 2.0), ceiling);
    let j = rp_tap(uv + t * vec2<f32>(-1.0, -1.0), ceiling);
    let k = rp_tap(uv + t * vec2<f32>(1.0, -1.0), ceiling);
    let l = rp_tap(uv + t * vec2<f32>(-1.0, 1.0), ceiling);
    let m = rp_tap(uv + t * vec2<f32>(1.0, 1.0), ceiling);
    return e * 0.125 + (a + c + g + i) * 0.03125 + (b + d + f + h) * 0.0625 + (j + k + l + m) * 0.125;
}

fn rp_run(gid: vec3<u32>, ceiling: bool) {
    let od = textureDimensions(rp_dst);
    if (gid.x >= od.x || gid.y >= od.y) {
        return;
    }
    let uv = (vec2<f32>(gid.xy) + 0.5) / vec2<f32>(od);
    let t = 1.0 / vec2<f32>(textureDimensions(rp_src));
    textureStore(rp_dst, vec2<i32>(gid.xy), vec4<f32>(rp_down13(uv, t, ceiling), 1.0));
}

/// Level 1 from the full-resolution history, each tap held to the ceiling.
@compute @workgroup_size(8, 8)
fn rp_first(@builtin(global_invocation_id) gid: vec3<u32>) {
    rp_run(gid, true);
}

/// Levels 2.. from the level above.
@compute @workgroup_size(8, 8)
fn rp_down(@builtin(global_invocation_id) gid: vec3<u32>) {
    rp_run(gid, false);
}
