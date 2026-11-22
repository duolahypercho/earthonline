// The photographic finish's post pass (`--photo-finish 1|2`, stage W4;
// eo-render photo.rs, which lists the steps). Composed after look_common
// (`Look`, `Sky`), look_finish.wgsl (its helpers `to_srgb`, `from_srgb`,
// `hash`; its `finish_color` is not called here) and photo.rs's constants
// (`PH_*`, from eo-look `tone`). CPU reference: eo-look `tone::photo_display`
// (all but the lateral colour, the mesopic fade, the noise and the dither).

@group(0) @binding(0) var<uniform> look: Look;
@group(0) @binding(1) var hdr: texture_2d<f32>;
@group(0) @binding(2) var<uniform> skyu: Sky;
@group(0) @binding(3) var bloom_tex: texture_2d<f32>;
@group(0) @binding(4) var ph_lin: sampler;
@group(0) @binding(5) var<uniform> photo: Photo;

struct Photo {
    // x level (1, 2), y the noise's frame seed, z lateral colour (px red-to-blue at the corner), w vignette residual
    lens: vec4<f32>,
    // x, y tan of the half field across and down
    view: vec4<f32>,
    // x shot variance per unit scene value, y read variance, z chroma share, w ISO
    grain: vec4<f32>,
};

@vertex
fn photo_vs(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(vi & 1u) * 4 - 1);
    let y = f32(i32(vi >> 1u) * 4 - 1);
    return vec4<f32>(x, y, 0.0, 1.0);
}

/// ACES's fit (`RRTAndODTFit`) on one value.
fn ph_fit(s: f32) -> f32 {
    let a = s * (s + 0.0245786) - 0.000090537;
    let b = s * (0.983729 * s + 0.4329510) + 0.238081;
    return a / b;
}

/// The old chain's display value of a grey of scene value `x` (eo-look
/// `tone::neutral_display`): its shoulder (`look.post0.xy`), then the fit.
fn ph_neutral(x: f32) -> f32 {
    var s = min(max(x, 0.0) / 0.6, PH_NEUTRAL_MAX);
    let start = look.post0.x;
    let scale = look.post0.y;
    if (s > start) {
        s = start + scale * log((s - start) / scale + 1.0);
    }
    return clamp(ph_fit(s), 0.0, 1.0);
}

/// The camera response (eo-look `tone::camera_response`): the brightest
/// channel on the neutral curve, the ratios kept, then straight toward white
/// by as much as a per-channel curve lifts the least channel, and a share of
/// a per-channel curve's turn in the shoulder.
fn ph_response(x0: vec3<f32>) -> vec3<f32> {
    let x = clamp(x0, vec3<f32>(0.0), vec3<f32>(0.6 * PH_NEUTRAL_MAX));
    let n = max(max(x.r, x.g), x.b);
    if (n <= 1e-10) {
        return vec3<f32>(0.0);
    }
    let m = min(min(x.r, x.g), x.b);
    let t = ph_neutral(n);
    let k = t / n;
    let qmin = m * k;
    let pmin = ph_neutral(max(m, PH_CROSSTALK * n));
    let w = clamp((pmin - qmin) / max(t - qmin, 1e-9), 0.0, 1.0);
    let q = x * k;
    let h = q + (vec3<f32>(t) - q) * w;
    // the shoulder's turn toward yellow, a share of a per-channel curve's.
    let p = clamp(vec3<f32>(ph_neutral(x.r), ph_neutral(x.g), ph_neutral(x.b)), h, vec3<f32>(t));
    return h + (p - h) * PH_HUE_SKEW;
}

/// The grade's brightness with the soft white (eo-look `tone::soft_white`).
fn ph_soft_white(y: vec3<f32>, top: f32) -> vec3<f32> {
    if (top <= 1.0) {
        return clamp(y, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    let k = PH_SOFT_KNEE;
    let d = max(y - vec3<f32>(k), vec3<f32>(0.0));
    let soft = vec3<f32>(k) + d - d * d * ((top - 1.0) / ((top - k) * (top - k)));
    let v = select(y, soft, y > vec3<f32>(k));
    return clamp(select(v, vec3<f32>(1.0), y >= vec3<f32>(top)), vec3<f32>(0.0), vec3<f32>(1.0));
}

/// A 32-bit integer hash (PCG's output permutation).
fn ph_pcg(v: u32) -> u32 {
    let state = v * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

/// Uniform in [-1, 1) for a pixel, a frame seed and a stream.
fn ph_rand(p: vec2<i32>, seed: u32, k: u32) -> f32 {
    let h = ph_pcg(bitcast<u32>(p.x) ^ ph_pcg(bitcast<u32>(p.y) ^ ph_pcg(seed * 4u + k)));
    return f32(h >> 8u) * (2.0 / 16777216.0) - 1.0;
}

/// About a unit Gaussian, a little over a pixel across: the pixel's uniform
/// twice and its right, lower and diagonal neighbours' once (variance 7/3,
/// scaled to 1; neighbours correlate by about 0.3, as a demosaiced and
/// denoised sensor's grain does).
fn ph_grain(p: vec2<i32>, seed: u32, k: u32) -> f32 {
    let a = ph_rand(p, seed, k);
    let b = ph_rand(p + vec2<i32>(1, 0), seed, k);
    let c = ph_rand(p + vec2<i32>(0, 1), seed, k);
    let d = ph_rand(p + vec2<i32>(1, 1), seed, k);
    return (2.0 * a + b + c + d) * 0.65465367;
}

/// The sensor's noise on scene values `x` (radiance x exposure): per channel
/// sigma^2 = shot x + read, one luminance pattern (level 2: and a chroma share).
fn ph_noise(x: vec3<f32>, p: vec2<i32>) -> vec3<f32> {
    let shot = photo.grain.x;
    let read = photo.grain.y;
    if (shot <= 0.0 && read <= 0.0) {
        return x;
    }
    let seed = u32(photo.lens.y);
    let sig = sqrt(shot * max(x, vec3<f32>(0.0)) + vec3<f32>(read));
    var n = vec3<f32>(ph_grain(p, seed, 0u));
    let ch = photo.grain.z;
    if (ch > 0.0) {
        let c = vec3<f32>(ph_grain(p, seed, 1u), ph_grain(p, seed, 2u), ph_grain(p, seed, 3u));
        n = n * sqrt(1.0 - ch * ch) + c * ch;
    }
    return max(x + sig * n, vec3<f32>(0.0));
}

/// The frame through the lens: the lateral colour (red outward, blue inward,
/// half the corner's separation each, as the square of the radius).
fn ph_lens(pos: vec2<f32>, dims: vec2<f32>) -> vec3<f32> {
    var c = textureLoad(hdr, vec2<i32>(pos), 0).rgb;
    let ca = photo.lens.z;
    if (ca > 0.0) {
        let half = 0.5 * dims;
        let pc = pos - half;
        let k = 0.5 * ca * length(pc) / dot(half, half);
        if (photo.lens.x >= 2.0) {
            // three taps a side across each colour's band (weights 1/4, 1/2, 1/4).
            var r = 0.0;
            var b = 0.0;
            for (var i = 0; i < 3; i = i + 1) {
                let f = 0.5 + 0.5 * f32(i);
                let wt = select(0.25, 0.5, i == 1);
                r = r + wt * textureSampleLevel(hdr, ph_lin, (pos + pc * k * f) / dims, 0.0).r;
                b = b + wt * textureSampleLevel(hdr, ph_lin, (pos - pc * k * f) / dims, 0.0).b;
            }
            c = vec3<f32>(r, c.g, b);
        } else {
            let r = textureSampleLevel(hdr, ph_lin, (pos + pc * k) / dims, 0.0).r;
            let b = textureSampleLevel(hdr, ph_lin, (pos - pc * k) / dims, 0.0).b;
            c = vec3<f32>(r, c.g, b);
        }
    }
    return c;
}

@fragment
fn photo_fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(hdr));
    var c = ph_lens(pos.xy, dims);
    if (any(c != c)) {
        c = vec3<f32>(0.0);
    }
    c = clamp(c, vec3<f32>(0.0), vec3<f32>(65504.0));
    if (skyu.bloom.w > 0.0) {
        c = c + skyu.bloom.w * max(textureSampleLevel(bloom_tex, ph_lin, pos.xy / dims, 0.0).rgb, vec3<f32>(0.0)); // sky (`bloom.w` its share)
    }
    // the residual fall-off (eo-look `tone::vignette_gain`).
    let a = (pos.xy / dims * 2.0 - 1.0) * photo.view.xy;
    let c2 = 1.0 / (1.0 + dot(a, a));
    c = c * (1.0 - photo.lens.w * (1.0 - c2 * c2));
    let exposure = look.key.w;
    // the eye's mesopic fade at night (as look_finish.wgsl `finish_color`).
    let nz = smoothstep(0.85, 1.0, skyu.night.z);
    if (nz > 0.0) {
        let y = lk_luma(c);
        let dim = 1.0 - smoothstep(0.03, 0.3, y * exposure);
        c = mix(c, vec3<f32>(y), 0.5 * nz * dim);
    }
    // the sensor, then the camera's response.
    let x = ph_noise(c * exposure, vec2<i32>(pos.xy));
    var d = to_srgb(ph_response(x));
    // Grade: saturate, contrast (power S-curve), brightness with the soft white.
    let css = vec3<f32>(0.213, 0.715, 0.072);
    let luma = dot(d, css);
    let sat = clamp(mix(vec3<f32>(luma), d, PH_SATURATION), vec3<f32>(0.0), vec3<f32>(1.0));
    let k = vec3<f32>(look.post0.w);
    let toe = pow(sat * 2.0, k) * 0.5;
    let shoulder = vec3<f32>(1.0) - pow((vec3<f32>(1.0) - sat) * 2.0, k) * 0.5;
    d = ph_soft_white(select(toe, shoulder, sat >= vec3<f32>(0.5)) * look.post1.x, look.post1.x);
    // Look finish (as `finish_color`, with the photo finish's vibrance).
    let dens = look.post2.x;
    let rgb = d * (vec3<f32>(1.0) - dens * smoothstep(vec3<f32>(0.25), vec3<f32>(1.0), d) * (vec3<f32>(1.0) - d));
    let l = dot(rgb, css);
    let bal = look.post1.y;
    let sw = smoothstep(0.0, 0.12, l) * (1.0 - smoothstep(0.12, bal, l));
    let hw = smoothstep(bal, 0.9, l) * (1.0 - smoothstep(0.96, 1.0, l));
    let toned = rgb + look.shadow_tint.xyz * sw + look.highlight_tint.xyz * hw;
    let chroma = max(max(toned.r, toned.g), toned.b) - min(min(toned.r, toned.g), toned.b);
    let tl = dot(toned, css);
    let vivid = mix(vec3<f32>(tl), toned, (1.0 - clamp(chroma, 0.0, 1.0)) * PH_VIBRANCE + 1.0);
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
