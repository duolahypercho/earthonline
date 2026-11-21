// (`--rt-gi`, rt_gi.rs) The city shader's side of the ray-traced diffuse
// light: the previous frame's half-resolution result (rt_gi.wgsl),
// reprojected with the camera it was made with and refused where the depth it
// recorded is another surface's (disocclusion), so a new view starts with the
// rig's light and the traced light fades in. Composed into the city shader
// only with the option (look.rs `city_shader_source_variant_probes`); without
// it these bindings, this code and the hook in look_light.wgsl are absent and
// every pipeline compiles as before.
//
// Group 1 bindings 20-22 (look.rs `RtGiBindings`): the result (rgb the
// irradiance without the rig's ambient and environment terms, a the
// environment's open share; filterable), its view depth (negative where
// none), and the camera it was made with.

struct RtGiView {
    vp: mat4x4<f32>,
    // xyz the eye the result was made from, w 1 when there is a result
    cam: vec4<f32>,
    fwd: vec4<f32>,
    // x how much of the screen-space occlusion stays on the indirect diffuse
    // (0: replaced by the traced visibility), y the traced light's weight,
    // z the distance it starts fading out at, w where it is gone
    params: vec4<f32>,
    // xy the result's size in texels
    size: vec4<f32>,
    // x 1: show the traced irradiance alone (EO_RT_GI_DEBUG)
    misc: vec4<f32>,
};

@group(1) @binding(20) var lx_rt_gi: texture_2d<f32>;
@group(1) @binding(21) var lx_rt_gi_z: texture_2d<f32>;
@group(1) @binding(22) var<uniform> lx_rt_view: RtGiView;

struct RtGiSample {
    // the irradiance without the ambient and environment terms
    e: vec3<f32>,
    // the environment's open share
    open: f32,
    // how much of it to take (0: none here)
    w: f32,
};

// As `ao_fetch` reads the AO: the texel under the point refused where its
// depth is not this surface's, then the result sampled bilinearly with the
// AO's clamping sampler (one depth load and one sample: every lit pipeline
// carries this, and on Apple's GPU a heavier read costs the city's opaque
// draw registers and occupancy even where it is not taken).
fn rt_gi_fetch(world: vec3<f32>) -> RtGiSample {
    var r = RtGiSample(vec3<f32>(0.0), 1.0, 0.0);
    if (lx_rt_view.cam.w < 0.5) {
        return r;
    }
    let clip = lx_rt_view.vp * vec4<f32>(world, 1.0);
    if (clip.w <= 1e-4) {
        return r;
    }
    let uv = vec2<f32>(clip.x / clip.w * 0.5 + 0.5, 0.5 - clip.y / clip.w * 0.5);
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) {
        return r;
    }
    let dims = vec2<i32>(lx_rt_view.size.xy);
    let tp = clamp(vec2<i32>(uv * lx_rt_view.size.xy), vec2<i32>(0), dims - 1);
    let z = textureLoad(lx_rt_gi_z, tp, 0).r;
    let expect = dot(world - lx_rt_view.cam.xyz, lx_rt_view.fwd.xyz);
    if (z < 0.0 || abs(z - expect) > 0.04 * expect + 0.15) {
        return r;
    }
    let s = textureSampleLevel(lx_rt_gi, lk_ao_samp, uv, 0.0);
    r.e = s.rgb;
    r.open = s.a;
    r.w = 1.0 - lk_smooth(lx_rt_view.params.z, lx_rt_view.params.w, expect);
    return r;
}
