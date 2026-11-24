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

/// (`--ao-albedo`, SF_AO_ALBEDO) the same fit at the surface's own diffuse
/// albedo, per channel (Jimenez, Wu, Pesce and Jarabo, "Practical Real-Time
/// Strategies for Accurate Indirect Occlusion", 2016, eq. 10: the light the
/// occluded hemisphere's surfaces pass on, taken at the albedo of the surface
/// itself, its surroundings assumed alike). The city's surfaces are 0.1-0.4,
/// not the JS's 0.7: a corner keeps less of its sky light (a visibility of
/// 0.3 keeps 0.37 at 0.25, 0.57 at 0.7) and a coloured one takes its colour.
fn ao_multi_bounce_albedo(v: f32, al: vec3<f32>) -> vec3<f32> {
    let a = 2.0404 * al - 0.3324;
    let b = -4.7951 * al + 0.6417;
    let c = 2.7552 * al + 0.6903;
    return max(vec3<f32>(v), ((v * a + b) * v + c) * v);
}

// Which of the occlusion pass's additions run (look.ao_params.w: bit 0 the
// sun bounce, bit 1 the albedo's multi-bounce).
fn ao_run(bit: u32) -> bool {
    return (u32(look.ao_params.w + 0.5) & bit) != 0u;
}

/// Jimenez et al.'s multi-bounce fit (the JS's `multiBounceAlbedo` 0.7).
fn ao_multi_bounce(v: f32) -> f32 {
    let al = 0.7;
    let a = 2.0404 * al - 0.3324;
    let b = -4.7951 * al + 0.6417;
    let c = 2.7552 * al + 0.6903;
    return max(v, ((v * a + b) * v + c) * v);
}
