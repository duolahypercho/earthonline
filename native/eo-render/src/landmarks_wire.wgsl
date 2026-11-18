// the wire members (main cables, suspender ropes), appended to the
// city shader (draw.wgsl + look + landmarks_surface). Group 0 is the city's
// draw group, group 1 the look's, group 2 the LOD groups (the near/far
// switch the cull pass wrote this frame).
//
// landmark-surface.js `applyWireCoverage`: the member is a prism at the
// envelope radius; a pixel keeps the rope where its view ray passes within
// max(true radius, LM_WIRE_MIN_PIXELS / 2 of a pixel) of the axis, with the
// share of that width the rope truly covers as its opacity, lit with the
// member's own normal (perpendicular to the axis, toward the eye).

struct LmGroup {
    center: vec3<f32>,
    radius: f32,
    ready: u32,
    detailed: u32,
    kind: u32,
    level: u32,
};

@group(2) @binding(0) var<storage, read> lm_groups: array<LmGroup>;

struct LmWireOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) axis: vec3<f32>,
    @location(2) dir: vec3<f32>,
    @location(3) @interpolate(flat) radius: f32,
    @location(4) color: vec3<f32>,
    @location(5) @interpolate(flat) material: u32,
};

@vertex
fn lm_wire_vs(
    @location(0) pos: vec3<f32>,
    @location(1) radius: f32,
    @location(2) axis: vec3<f32>,
    @location(3) group: u32,
    @location(4) dir: vec3<f32>,
    @location(5) color: vec4<f32>,
    @location(6) material: u32,
) -> LmWireOut {
    var out: LmWireOut;
    out.clip = frame.view_proj * vec4<f32>(pos, 1.0);
    if (lm_groups[group].detailed == 0u) {
        // The landmark is far: its far item draws the cables.
        out.clip = vec4<f32>(0.0);
    }
    out.world = pos;
    out.axis = axis;
    out.dir = dir;
    out.radius = radius;
    out.color = srgb_decode(color.rgb);
    out.material = material;
    return out;
}

@fragment
fn lm_wire_fs(in: LmWireOut) -> @location(0) vec4<f32> {
    let cam = frame.cam_pos.xyz;
    // Metres per pixel at the fragment's depth: depth x |d ndc_y / d pixel| / P11
    // (the NDC derivative is 2 / the target height; P11 is the length of the
    // clip matrix's second row). Derivatives first, in uniform control flow.
    let clip = frame.view_proj * vec4<f32>(in.world, 1.0);
    let ndc_y = clip.y / clip.w;
    let dndc = abs(dpdy(ndc_y)) + abs(dpdx(ndc_y));
    let vp = frame.view_proj;
    let p11 = length(vec3<f32>(vp[0][1], vp[1][1], vp[2][1]));
    let depth = max(clip.w, 0.05);
    let mpp = depth * dndc / max(p11, 1e-6);
    let dir = normalize(in.dir);
    let to_eye = cam - in.axis;
    let ray = normalize(in.world - cam);
    let across = cross(ray, dir);
    let miss = abs(dot(to_eye, across)) / max(length(across), 1e-6);
    let drawn = min(max(in.radius, mpp * LM_WIRE_MIN_PIXELS * 0.5), LM_WIRE_ENVELOPE_M);
    if (!(in.radius > 0.0 && miss <= drawn)) {
        discard;
    }
    let opacity = clamp(in.radius / max(drawn, 1e-5), 0.0, 1.0);
    let facing = normalize(to_eye - dir * dot(to_eye, dir));
    let m = materials[in.material];
    var s: Surface;
    s.albedo = in.color;
    s.rough = clamp(m.params.x, 0.03, 1.0);
    s.metal = m.params.y;
    s.ao = 1.0;
    s.n = facing;
    s.alpha = opacity;
    s.env = ENV_PAINTED_METAL;
    let v = normalize(cam - in.world);
    let c = look_light(s, m, v, in.world, facing, 0u);
    return vec4<f32>(c.rgb, opacity);
}

// ---- the wire members after the atmosphere (`BridgeFeatures::wires`) ----
//
// The members write no depth, so the sky resolve (sky_resolve.wgsl), which
// composites every pixel that holds no depth as the sky (the scene there
// times the volume's transmittance to infinity, about 0, plus its in-scatter),
// wiped every rope drawn over the sky: the cables and suspenders vanished
// against the sky at every distance. This pass draws them over the resolved
// frame instead (the frame the post reads), at every distance (the far item
// carries no cable beams then), with the unjittered camera, depth-tested
// against the scene:
//
// * the vertex stage widens each member's prism to its true radius plus two
//   pixels at the vertex's distance (a rope never falls between pixels);
// * the fragment's coverage is the share of a one-pixel tent filter centred
//   on the pixel's ray that the rope's true cross-section covers (the
//   filter's CDF at miss + r less at miss - r): a thick cable is solid with a
//   one-pixel soft edge, a thin rope a hairline whose summed coverage is its
//   true width (2 r / pixel), so it neither shimmers nor vanishes;
// * a rope wider than a pixel is shaded with its round section's normal.

fn lm_tent_cdf(u: f32, f: f32) -> f32 {
    let x = clamp(u / f, -1.0, 1.0);
    if (x < 0.0) {
        return 0.5 * (x + 1.0) * (x + 1.0);
    }
    return 1.0 - 0.5 * (1.0 - x) * (1.0 - x);
}

/// Metres per pixel at a clip w (the unjittered camera's).
fn lm_wire_mpp(w: f32) -> f32 {
    let vp = skyu.vp;
    let p11 = length(vec3<f32>(vp[0][1], vp[1][1], vp[2][1]));
    return max(w, 0.05) * 2.0 / (max(skyu.res.y, 1.0) * max(p11, 1e-6));
}

struct LmWire2Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) axis: vec3<f32>,
    @location(2) dir: vec3<f32>,
    @location(3) @interpolate(flat) radius: f32,
    @location(4) color: vec4<f32>,
    @location(5) @interpolate(flat) material: u32,
};

@vertex
fn lm_wire2_vs(
    @location(0) pos: vec3<f32>,
    @location(1) radius: f32,
    @location(2) axis: vec3<f32>,
    @location(3) group: u32,
    @location(4) dir: vec3<f32>,
    @location(5) color: vec4<f32>,
    @location(6) material: u32,
) -> LmWire2Out {
    var out: LmWire2Out;
    let vp = skyu.vp;
    let drawn0 = max(radius, LM_WIRE_ENVELOPE_M);
    let need = max(drawn0, radius + 2.0 * lm_wire_mpp((vp * vec4<f32>(pos, 1.0)).w));
    let p = axis + (pos - axis) * (need / drawn0);
    out.clip = vp * vec4<f32>(p, 1.0);
    if (!(radius > 0.0)) {
        out.clip = vec4<f32>(0.0);
    }
    out.world = p;
    out.axis = axis;
    out.dir = dir;
    out.radius = radius;
    out.color = vec4<f32>(srgb_decode(color.rgb), color.a);
    out.material = material;
    return out;
}

/// The west span's suspender LEDs at night (`BridgeFeatures::night`, wire
/// vertices of alpha 0): luminance of the rope's strip, cd/m2, before the
/// lamps' scale. LM_LED_CD candela every LM_LED_PITCH_M along the rope is
/// LM_LED_CD / pitch candela a metre, over the rope's width 2 r; resolved
/// into dots where a pixel is under a third of the pitch. The sequence (the
/// installation plays changing patterns) is drawn as two slow waves along the
/// span and one down the ropes.
fn lm_led(p: vec3<f32>, r: f32, mpp: f32) -> f32 {
    let along = dot(p.xz - LM_BAY_ORIGIN, LM_BAY_AXIS);
    let t = skyu.eye.w;
    let w1 = 0.5 + 0.5 * sin(along * 0.021 - t * 0.35 + 1.3 * sin(along * 0.0047 + t * 0.07));
    let w2 = 0.5 + 0.5 * sin(along * 0.083 + p.y * 0.11 + t * 0.9);
    let fall = 0.5 + 0.5 * sin(p.y * 0.19 + t * 1.6 + along * 0.013);
    let pattern = 0.04 + 0.96 * pow(clamp(w1 * (0.45 + 0.55 * w2) * (0.5 + 0.5 * fall), 0.0, 1.0), 1.6);
    let ph = fract(p.y / LM_LED_PITCH_M) - 0.5;
    let dot_w = 0.02;
    let g = exp(-(ph * LM_LED_PITCH_M) * (ph * LM_LED_PITCH_M) / (2.0 * dot_w * dot_w)) / (dot_w * 2.5066 / LM_LED_PITCH_M);
    let resolve = lm_ss(LM_LED_PITCH_M * 0.33, LM_LED_PITCH_M * 0.1, mpp);
    return LM_LED_CD / (LM_LED_PITCH_M * 2.0 * max(r, 1e-3)) * pattern * mix(1.0, g, resolve);
}

@fragment
fn lm_wire2_fs(in: LmWire2Out) -> @location(0) vec4<f32> {
    let cam = skyu.eye.xyz;
    let dir = normalize(in.dir);
    let a = cam - in.axis;
    // Only the prism's half toward the eye (it covers the whole silhouette
    // once; the far half drew every pixel a second time).
    let off = in.world - in.axis;
    if (dot(off - dir * dot(off, dir), a - dir * dot(a, dir)) < 0.0) {
        discard;
    }
    let ray = normalize(in.world - cam);
    let q0 = cross(dir, ray);
    let q = q0 / max(length(q0), 1e-6);
    let miss = dot(a, q);
    let mpp = lm_wire_mpp((skyu.vp * vec4<f32>(in.world, 1.0)).w);
    let r = in.radius;
    let cov = lm_tent_cdf(miss + r, mpp) - lm_tent_cdf(miss - r, mpp);
    if (!(r > 0.0) || cov < 1.0 / 1024.0) {
        discard;
    }
    let facing = normalize(a - dir * dot(a, dir));
    let on_axis = in.axis + dir * dot(in.world - in.axis, dir);
    let m = materials[in.material];
    var s: Surface;
    s.albedo = in.color.rgb;
    s.rough = clamp(m.params.x, 0.03, 1.0);
    s.metal = m.params.y;
    s.ao = 1.0;
    s.alpha = 1.0;
    s.env = ENV_PAINTED_METAL;
    // The section the pixel sees: the round normal at its offset where the
    // rope is wider than the pixel's tent, else the mean of the visible half
    // section (two offsets across it, +-0.6 of the radius: the sunlit flank
    // of a hairline shows).
    var c = vec3<f32>(0.0);
    var offsets = array<f32, 2>(-0.6, 0.6);
    let wide = r > mpp;
    let taps = select(2, 1, wide);
    for (var k = 0; k < taps; k = k + 1) {
        var t = clamp(miss / r, -1.0, 1.0);
        if (!wide) {
            t = offsets[k];
        }
        let n = normalize(q * t + facing * sqrt(max(1.0 - t * t, 0.0)));
        let p = on_axis + n * r;
        s.n = n;
        c = c + look_light(s, m, normalize(cam - p), p, n, 0u).rgb;
    }
    c = c / f32(taps);
    let p = on_axis + facing * r;
    if (in.color.a < 0.5 && LM_LED_ON) {
        c = c + vec3<f32>(lm_led(p, r, mpp) * skyu.bloom_w2.z * skyu.night.x);
    }
    return vec4<f32>(c, clamp(cov, 0.0, 1.0));
}
