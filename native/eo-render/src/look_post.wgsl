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

fn rrt_odt(x: vec3<f32>) -> vec3<f32> {
    let a = x * (x + 0.0245786) - 0.000090537;
    let b = x * (0.983729 * x + 0.4329510) + 0.238081;
    return a / b;
}

fn aces(c: vec3<f32>, exposure: f32) -> vec3<f32> {
    let v = c * exposure / 0.6;
    let i = vec3<f32>(
        dot(vec3<f32>(0.59719, 0.35458, 0.04823), v),
        dot(vec3<f32>(0.07600, 0.90834, 0.01566), v),
        dot(vec3<f32>(0.02840, 0.13383, 0.83777), v),
    );
    let f = rrt_odt(i);
    let o = vec3<f32>(
        dot(vec3<f32>(1.60475, -0.53108, -0.07367), f),
        dot(vec3<f32>(-0.10208, 1.10813, -0.00605), f),
        dot(vec3<f32>(-0.00327, -0.07276, 1.07602), f),
    );
    return clamp(o, vec3<f32>(0.0), vec3<f32>(1.0));
}

fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055, c * 12.92, c <= vec3<f32>(0.0031308));
}

fn from_srgb(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

fn hash(p: vec2<f32>, k: f32) -> f32 {
    return fract(sin(dot(p + k, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

@fragment
fn post_fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(hdr));
    var c = textureLoad(hdr, vec2<i32>(pos.xy), 0).rgb;
    c = max(c, vec3<f32>(0.0));
    if (skyu.bloom.w > 0.5) {
        c = c + max(textureSampleLevel(bloom_tex, bloom_samp, pos.xy / dims, 0.0).rgb, vec3<f32>(0.0)); // sky
    }
    let exposure = look.key.w;
    // mesopic vision: at night colour fades from what is dim (CIE
    // 191:2010; rods carry no colour), so an unlit lawn or facade goes
    // grey-blue-black while a lamp's pool and a lit window keep theirs.
    // only once the sky is dark (nightness 0.85 .. 1: sun under ~-5 deg): through
    // the blue hour it greyed the twilight sky into a uniform lavender wash.
    let nz = smoothstep(0.85, 1.0, skyu.night.z);
    if (nz > 0.0) {
        let y = lk_luma(c);
        let dim = 1.0 - smoothstep(0.03, 0.3, y * exposure);
        // 0.5 (0.75 greyed the city's orange skyglow into neutral charcoal)
        c = mix(c, vec3<f32>(y), 0.5 * nz * dim);
    }
    // Highlight shoulder, one gain off the peak channel.
    let start = look.post0.x;
    let scale = look.post0.y;
    let peak = max(max(c.r, c.g), c.b) * exposure / 0.6;
    let held = max(peak, start);
    let rolled = start + scale * log((held - start) / scale + 1.0);
    c = c * (rolled / held);
    var d = to_srgb(aces(c, exposure));
    // Grade: saturate, contrast (power S-curve), brightness.
    let css = vec3<f32>(0.213, 0.715, 0.072);
    let luma = dot(d, css);
    let sat = clamp(mix(vec3<f32>(luma), d, look.post0.z), vec3<f32>(0.0), vec3<f32>(1.0));
    let k = vec3<f32>(look.post0.w);
    let toe = pow(sat * 2.0, k) * 0.5;
    let shoulder = vec3<f32>(1.0) - pow((vec3<f32>(1.0) - sat) * 2.0, k) * 0.5;
    d = clamp(select(toe, shoulder, sat >= vec3<f32>(0.5)) * look.post1.x, vec3<f32>(0.0), vec3<f32>(1.0));
    // Look finish.
    let dens = look.post2.x;
    let rgb = d * (vec3<f32>(1.0) - dens * smoothstep(vec3<f32>(0.25), vec3<f32>(1.0), d) * (vec3<f32>(1.0) - d));
    let l = dot(rgb, css);
    let bal = look.post1.y;
    let sw = smoothstep(0.0, 0.12, l) * (1.0 - smoothstep(0.12, bal, l));
    let hw = smoothstep(bal, 0.9, l) * (1.0 - smoothstep(0.96, 1.0, l));
    let toned = rgb + look.shadow_tint.xyz * sw + look.highlight_tint.xyz * hw;
    let chroma = max(max(toned.r, toned.g), toned.b) - min(min(toned.r, toned.g), toned.b);
    let tl = dot(toned, css);
    let vivid = mix(vec3<f32>(tl), toned, (1.0 - clamp(chroma, 0.0, 1.0)) * look.post1.z + 1.0);
    let cu = pos.xy / dims - 0.5;
    let vig = 1.0 - smoothstep(0.12, 0.5, dot(cu, cu)) * look.post1.w;
    d = clamp(vivid * vig, vec3<f32>(0.0), vec3<f32>(1.0));
    // Display dither: triangular, +-1 step of 8-bit.
    let amp = look.post2.y;
    let n = hash(pos.xy, look.post2.w) + hash(pos.yx * 1.31, look.post2.w + 3.7) - 1.0;
    d = clamp(d + vec3<f32>(n * amp), vec3<f32>(0.0), vec3<f32>(1.0));
    if (look.post2.z > 0.5) {
        d = from_srgb(d);
    }
    return vec4<f32>(d, 1.0);
}
