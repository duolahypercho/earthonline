// The image pipeline after the scene (src/render/post-chain.js, in its order):
// highlight shoulder on the tone mapper's input, three's ACES filmic with
// toneMappingExposure, sRGB encode, the display-referred grade (CSS-order
// saturate, power S-curve contrast pivoted on 0.5, brightness), the look's
// finish (density shoulder, split tone, vibrance, vignette) and the display
// dither. CPU reference: eo-look `tone::display`.
// Not here: GTAO, the post aerial-path and sun-veil stages, bloom, SMAA.

@group(0) @binding(0) var<uniform> look: Look;
@group(0) @binding(1) var hdr: texture_2d<f32>;
// the bloom pyramid's result (added before the highlight shoulder, as
// post-chain.js adds its BloomNode), and the sky uniform it is switched by.
@group(0) @binding(2) var<uniform> skyu: Sky;
@group(0) @binding(3) var bloom_tex: texture_2d<f32>;
@group(0) @binding(4) var bloom_samp: sampler;

@vertex
fn post_vs(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(vi & 1u) * 4 - 1);
    let y = f32(i32(vi >> 1u) * 4 - 1);
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn post_fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(hdr));
    var c = textureLoad(hdr, vec2<i32>(pos.xy), 0).rgb;
    c = max(c, vec3<f32>(0.0));
    if (skyu.bloom.w > 0.) {
        c = c + skyu.bloom.w * max(textureSampleLevel(bloom_tex, bloom_samp, pos.xy / dims, 0.0).rgb, vec3<f32>(0.0)); // sky
    }
    // the camera look (look_camera.wgsl) in place of the filmic or the ACES-fit chain
    // (the playable house's source camera, `--blender-camera`, in place of both: `finish_color`)
    if (look.tone2.z > 0.5 && !finish_blender_camera()) {
        return finish_camera(c, pos);
    }
    return finish_color(c, pos, dims);
}
