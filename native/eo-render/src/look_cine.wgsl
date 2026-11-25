// The cinematic look's post pass (`--look cinematic`, stage L1; eo-render
// cine_post.rs lists the steps; CPU reference eo-look
// `cinematic::grade::cinematic_display_lut`). Composed after photo.rs's
// shader (look_common's `Look` and `Sky`, look_finish.wgsl's helpers, the
// photo finish's constants and functions: its lens, fall-off and noise are
// used when the photo finish is on, `photo.lens.x` >= 1) and cine_post.rs's
// constants (`CN_*`). Its own pipeline, made only under the look: the default
// post pass and the photo pass are untouched.

@group(0) @binding(6) var cine_lut: texture_3d<f32>;
@group(0) @binding(7) var cine_samp: sampler;
@group(0) @binding(8) var<uniform> cine: Cine;
// (`Cine` is cine_post.rs `CINE_STRUCT_WGSL`.)

/// The tone curve (eo-look `grade::curve`): scene value -> display linear.
fn cn_curve(x: f32) -> f32 {
    if (!(x > 0.0)) {
        return 0.0;
    }
    if (x < CN_X0) {
        return CN_Y0 * pow(x / CN_X0, CN_TOE_POWER);
    }
    if (x < CN_X1) {
        return CN_Y0 + CN_SLOPE * (x - CN_X0);
    }
    return 1.0 - (1.0 - CN_Y1) * exp(-CN_RATE * (x - CN_X1));
}

/// The tone on a colour (eo-look `grade::tone`): the brightest channel on the
/// curve, the ratios kept, toward white once near or past the white point.
fn cn_tone(x0: vec3<f32>) -> vec3<f32> {
    let x = clamp(x0, vec3<f32>(0.0), vec3<f32>(CN_MAX_X));
    let n = max(max(x.r, x.g), x.b);
    if (n <= 1e-10) {
        return vec3<f32>(0.0);
    }
    let t = cn_curve(n);
    let a = smoothstep(CN_WHITEN_EV0, CN_WHITEN_EV1, log2(n / CN_WHITE_X));
    let q = x * (t / n);
    return q + (vec3<f32>(t) - q) * a;
}

/// A pixel's bright part as the bloom's prefilter takes it (sky_bloom.wgsl
/// `bl_tap`'s ceiling, `bloom_prefilter`'s cap and threshold), so the share
/// the pyramid spreads is taken off the pixel it came from.
fn cn_bright(c: vec3<f32>) -> vec3<f32> {
    let th = skyu.bloom.x;
    let cap = th * skyu.bloom_w2.y;
    let m = max(c.r, max(c.g, c.b));
    var b = c * (cap / max(m, cap));
    let lw = vec3<f32>(0.2126, 0.7152, 0.0722);
    b = b * min(1.0, 16.0 * th / max(dot(b, lw), 1e-6));
    return b * smoothstep(th, th + skyu.bloom.y * th, dot(b, lw));
}

@fragment
fn cine_fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(hdr));
    // (`--cine-camera`) the camera model (look_cine_camera.wgsl; compiled out without it).
    if (SF_CAMERA && cine.cam0.w > 0.5) {
        return cine_camera(pos.xy, dims);
    }
    let lens = photo.lens.x >= 1.0;
    var c: vec3<f32>;
    if (lens) {
        c = ph_lens(pos.xy, dims);
    } else {
        c = textureLoad(hdr, vec2<i32>(pos.xy), 0).rgb;
    }
    if (any(c != c)) {
        c = vec3<f32>(0.0);
    }
    c = clamp(c, vec3<f32>(0.0), vec3<f32>(65504.0));
    // the bloom, energy kept: the pixel gives up the share of its bright part
    // the pyramid spreads (the levels' weights sum to that share, sky.rs).
    // (`bloom.w` its share: under 1 where the playable house's source lens takes the view;
    // photo mode's output carries the bloom already, added the same way: photo_mode.wgsl)
    if (skyu.bloom.w > 0.0 && !(SF_PHOTO_MODE && cine.bloom.y > 0.5)) {
        let b = max(textureSampleLevel(bloom_tex, ph_lin, pos.xy / dims, 0.0).rgb, vec3<f32>(0.0));
        c = max(c - skyu.bloom.w * cine.bloom.x * cn_bright(c), vec3<f32>(0.0)) + skyu.bloom.w * b;
    }
    if (lens) {
        // the photo finish's residual fall-off (eo-look `tone::vignette_gain`).
        let a = (pos.xy / dims * 2.0 - 1.0) * photo.view.xy;
        let c2 = 1.0 / (1.0 + dot(a, a));
        c = c * (1.0 - photo.lens.w * (1.0 - c2 * c2));
    }
    var x = c * look.key.w;
    if (lens) {
        x = ph_noise(x, vec2<i32>(pos.xy));
    }
    var d = to_srgb(cn_tone(x));
    // the hour's grade (cine_post.rs bakes the table when the hour or the weather moves it).
    d = textureSampleLevel(cine_lut, cine_samp, d * cine.grade.y + vec3<f32>(cine.grade.z), 0.0).rgb;
    let cu = pos.xy / dims - 0.5;
    d = clamp(d * (1.0 - smoothstep(0.12, 0.5, dot(cu, cu)) * cine.grade.x), vec3<f32>(0.0), vec3<f32>(1.0));
    // Display dither: triangular, +-1 step of 8-bit (as `finish_color`).
    let amp = look.post2.y;
    let n = hash(pos.xy, look.post2.w) + hash(pos.yx * 1.31, look.post2.w + 3.7) - 1.0;
    d = clamp(d + vec3<f32>(n * amp), vec3<f32>(0.0), vec3<f32>(1.0));
    if (look.post2.z > 0.5) {
        d = from_srgb(d);
    }
    return vec4<f32>(d, 1.0);
}
