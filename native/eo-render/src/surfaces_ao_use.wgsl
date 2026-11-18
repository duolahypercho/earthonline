// the city shader's side of the occlusion + bounce pass
// (surfaces_ao.wgsl): the previous frame's half-resolution result,
// reprojected with the camera it was made with and refused where the depth it
// recorded is not this surface (disocclusion), so a new view starts open.

@group(1) @binding(3) var lk_ao: texture_2d<f32>;
@group(1) @binding(4) var lk_ao_z: texture_2d<f32>;
@group(1) @binding(5) var lk_ao_samp: sampler;

/// rgb: the bounce (cosine-weighted fraction x radiance of what hides it),
/// a: sky visibility. (0, 0, 0, 1) where nothing usable was recorded.
fn ao_fetch(world: vec3<f32>) -> vec4<f32> {
    let none = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    if (look.ao_params.x < 0.5) {
        return none;
    }
    let clip = look.ao_vp * vec4<f32>(world, 1.0);
    if (clip.w <= 1e-4) {
        return none;
    }
    let uv = vec2<f32>(clip.x / clip.w * 0.5 + 0.5, 0.5 - clip.y / clip.w * 0.5);
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) {
        return none;
    }
    let dims = vec2<i32>(textureDimensions(lk_ao_z));
    let tp = clamp(vec2<i32>(uv * vec2<f32>(dims)), vec2<i32>(0), dims - 1);
    let z = textureLoad(lk_ao_z, tp, 0).r;
    let expect = dot(world - look.ao_cam.xyz, look.ao_fwd.xyz);
    if (z < 0.0 || abs(z - expect) > 0.04 * expect + 0.12) {
        return none;
    }
    return textureSampleLevel(lk_ao, lk_ao_samp, uv, 0.0);
}

/// Jimenez et al.'s multi-bounce fit (the JS's `multiBounceAlbedo` 0.7).
fn ao_multi_bounce(v: f32) -> f32 {
    let al = 0.7;
    let a = 2.0404 * al - 0.3324;
    let b = -4.7951 * al + 0.6417;
    let c = 2.7552 * al + 0.6903;
    return max(v, ((v * a + b) * v + c) * v);
}
