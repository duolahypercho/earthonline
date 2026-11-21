// Display transform shared by the ordinary and submerged camera.
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

// the filmic curve (Lottes 2016): x^a / ((x^a)^d b + c), `look.tone` = (a, d, b, c)
fn pf_curve(x: f32) -> f32 {
    if (x <= 0.0) {
        return 0.0;
    }
    let xa = pow(x, look.tone.x);
    return clamp(xa / (pow(xa, look.tone.y) * look.tone.z + look.tone.w), 0.0, 1.0);
}

// the chroma gain in Oklab (Ottosson 2020), faded out toward black and white
fn pf_chroma(c: vec3<f32>, gain: f32) -> vec3<f32> {
    let l0 = 0.4122214708 * c.r + 0.5363325363 * c.g + 0.0514459929 * c.b;
    let m0 = 0.2119034982 * c.r + 0.6806995451 * c.g + 0.1073969566 * c.b;
    let s0 = 0.0883024619 * c.r + 0.2817188376 * c.g + 0.6299787005 * c.b;
    let l1 = pow(max(l0, 0.0), 1.0 / 3.0);
    let m1 = pow(max(m0, 0.0), 1.0 / 3.0);
    let s1 = pow(max(s0, 0.0), 1.0 / 3.0);
    let L = 0.2104542553 * l1 + 0.7936177850 * m1 - 0.0040720468 * s1;
    let a = 1.9779984951 * l1 - 2.4285922050 * m1 + 0.4505937099 * s1;
    let b = 0.0259040371 * l1 + 0.7827717662 * m1 - 0.8086757660 * s1;
    let k = 1.0 + (gain - 1.0) * smoothstep(0.0, 0.15, L) * (1.0 - smoothstep(0.85, 1.0, L));
    let l2 = L + 0.3963377774 * a * k + 0.2158037573 * b * k;
    let m2 = L - 0.1055613458 * a * k - 0.0638541728 * b * k;
    let s2 = L - 0.0894841775 * a * k - 1.2914855480 * b * k;
    let l3 = l2 * l2 * l2;
    let m3 = m2 * m2 * m2;
    let s3 = s2 * s2 * s2;
    return clamp(vec3<f32>(
        4.0767416621 * l3 - 3.3077115913 * m3 + 0.2309699292 * s3,
        -1.2684380046 * l3 + 2.6097574011 * m3 - 0.3413193965 * s3,
        -0.0041960863 * l3 - 0.7034186147 * m3 + 1.7076147010 * s3,
    ), vec3<f32>(0.0), vec3<f32>(1.0));
}

// mesopic vision (shared by `finish_color` and the camera look's `finish_camera`).
fn finish_mesopic(radiance: vec3<f32>) -> vec3<f32> {
    var c = radiance;
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
    return c;
}

fn finish_color(radiance: vec3<f32>, pos: vec4<f32>, dims: vec2<f32>) -> vec4<f32> {
    var c = finish_mesopic(radiance);
    let exposure = look.key.w;
    if (look.tone2.x > 0.5) {
        // the filmic display transform (eo-look `tone::display_filmic`): the toe and
        // shoulder curve per channel, the Oklab chroma gain, sRGB, the display dither.
        let cw = vec3<f32>(dot(look.wb[0].xyz, c), dot(look.wb[1].xyz, c), dot(look.wb[2].xyz, c));
        let x = max(cw, vec3<f32>(0.0)) * exposure / 0.6;
        var y = vec3<f32>(pf_curve(x.r), pf_curve(x.g), pf_curve(x.b));
        y = pf_chroma(y, look.tone2.y);
        var e = to_srgb(clamp(y, vec3<f32>(0.0), vec3<f32>(1.0)));
        let amp2 = look.post2.y;
        let n2 = hash(pos.xy, look.post2.w) + hash(pos.yx * 1.31, look.post2.w + 3.7) - 1.0;
        e = clamp(e + vec3<f32>(n2 * amp2), vec3<f32>(0.0), vec3<f32>(1.0));
        if (look.post2.z > 0.5) {
            e = from_srgb(e);
        }
        return vec4<f32>(e, 1.0);
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
