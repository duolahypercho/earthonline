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
//
// The alpha is the trace's hierarchical depth (sky_reflect.rs): level 1's the
// occlusion pass's half-resolution view depth (`rp_z`, the trace's own), each
// later level's the nearest of the texels under it (with the last row or
// column of an odd level above folded into its neighbour's), rounded toward
// the eye before it is stored at half precision, so a cell never claims to
// be further than anything in it; sky and nothing are `RP_FAR_M`.

@group(0) @binding(0) var<uniform> skyu: Sky;
@group(0) @binding(1) var rp_src: texture_2d<f32>;
@group(0) @binding(2) var rp_lin: sampler;
@group(0) @binding(3) var rp_dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(4) var rp_z: texture_2d<f32>;

/// Sky and nothing in the Hi-Z (m; half precision's largest is 65504).
const RP_FAR_M: f32 = 65000.0;
/// The share a depth is pulled toward the eye before half precision rounds it
/// (its step is 2^-11 of the value).
const RP_NEAR_ROUND: f32 = 0.998;

/// Level 1's Hi-Z: the view depth at its own texel.
fn rp_near_first(p: vec2<u32>) -> f32 {
    let zd = textureDimensions(rp_z);
    let z = textureLoad(rp_z, min(p, zd - 1u), 0).r;
    return select(RP_FAR_M, min(z * RP_NEAR_ROUND, RP_FAR_M), z > 0.0);
}

/// A later level's Hi-Z: the nearest of the level above's texels under `p`
/// (an odd level's last row or column goes to the texel beside it).
fn rp_near_down(p: vec2<u32>) -> f32 {
    let sd = textureDimensions(rp_src);
    let od = textureDimensions(rp_dst);
    let ex = select(2u, 3u, p.x + 1u == od.x && sd.x > 2u * od.x);
    let ey = select(2u, 3u, p.y + 1u == od.y && sd.y > 2u * od.y);
    var z = RP_FAR_M;
    for (var j = 0u; j < ey; j = j + 1u) {
        for (var i = 0u; i < ex; i = i + 1u) {
            z = min(z, textureLoad(rp_src, min(2u * p + vec2<u32>(i, j), sd - 1u), 0).a);
        }
    }
    return z;
}

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
    let z = select(rp_near_down(gid.xy), rp_near_first(gid.xy), ceiling);
    textureStore(rp_dst, vec2<i32>(gid.xy), vec4<f32>(rp_down13(uv, t, ceiling), z));
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
