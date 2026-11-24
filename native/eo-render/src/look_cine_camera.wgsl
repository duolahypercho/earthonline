// The camera model's post path (`--cine-camera`, camera track; cine_post.rs
// lists the steps). Composed after look_cine.wgsl (its bindings, the grade's
// table), the photo finish's noise source (`ph_rand`), and
// cine_post.rs's camera constants (`CAM_*`), `cam_bloom_part` and the
// meter's bindings (`cam_state`, `cam_grid`). Called from `cine_fs` only when
// the model is compiled in (`SF_CAMERA`) and on.

const CAM_LW: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

/// The filmic curve: an AgX-style log sigmoid applied per channel in an inset
/// gamut (the primaries pulled toward white), so a bright saturated colour
/// runs smoothly to white along its own hue instead of one channel clipping
/// into a flat plateau; then the outset back to Rec. 709. Scene value (middle
/// grey 0.18) -> display linear 0..1.
fn cam_agx(x0: vec3<f32>) -> vec3<f32> {
    // inset and outset (column-major), as published with Blender's AgX view.
    let m_in = mat3x3<f32>(
        vec3<f32>(0.842479062253094, 0.0423282422610123, 0.0423756549057051),
        vec3<f32>(0.0784335999999992, 0.878468636469772, 0.0784336),
        vec3<f32>(0.0792237451477643, 0.0791661274605434, 0.879142973793104),
    );
    let m_out = mat3x3<f32>(
        vec3<f32>(1.19687900512017, -0.0528968517574562, -0.0529716355144438),
        vec3<f32>(-0.0980208811401368, 1.15190312990417, -0.0980434501171241),
        vec3<f32>(-0.0990297440797205, -0.0989611768448433, 1.15107367264116),
    );
    var v = m_in * max(x0, vec3<f32>(0.0));
    // the scene's contrast about middle grey (`CAM_CONTRAST`: a film's gamma over the sigmoid's own).
    let g = log2(0.18);
    v = vec3<f32>(g) + (log2(max(v, vec3<f32>(1e-10))) - vec3<f32>(g)) * CAM_CONTRAST;
    v = clamp(v, vec3<f32>(CAM_AGX_LO), vec3<f32>(CAM_AGX_HI));
    v = (v - vec3<f32>(CAM_AGX_LO)) / (CAM_AGX_HI - CAM_AGX_LO);
    // the sigmoid (its sixth-order fit): display-encoded (about a 2.2 power).
    let v2 = v * v;
    let v4 = v2 * v2;
    v = 15.5 * v4 * v2 - 40.14 * v4 * v + 31.96 * v4 - 6.868 * v2 * v + 0.4298 * v2 + 0.1191 * v - vec3<f32>(0.00232);
    // the look's saturation on the encoded values (`CAM_SATURATION`).
    v = clamp(v, vec3<f32>(0.0), vec3<f32>(1.0));
    let l = dot(v, CAM_LW);
    v = vec3<f32>(l) + (v - vec3<f32>(l)) * CAM_SATURATION;
    v = pow(clamp(v, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(2.2));
    return clamp(m_out * v, vec3<f32>(0.0), vec3<f32>(1.0));
}

/// The local exposure's gain at a pixel of log2 luminance `lx` (exposed by the
/// hour's exposure, the grid's units). The base luminance round it (the
/// bilateral grid sliced at its own luminance, blended with the wider blur of
/// the grid's columns) is compressed about middle grey by the meter's
/// `k_shadow` or `k_high` (1 when the frame's range fits the display: then
/// the base keeps its contrast); the pixel's detail against the edge-aware
/// base alone (the grid's slice: only neighbours within about 1.5 EV of it,
/// so an edge's other side takes no part) is raised by `cam3.z`, rolled off
/// past `CAM_DETAIL_SPAN` so a step is not doubled. A print's local contrast:
/// texture, mortar lines and a crevice's dark read, the blacks are not lifted.
fn cam_local(pos: vec2<f32>, lx: f32) -> f32 {
    let gd = vec3<f32>(textureDimensions(cam_grid));
    let span = CAM_GRID_HI - CAM_GRID_LO;
    let gz = clamp((lx - CAM_GRID_LO) / span, 0.0, 1.0);
    let g = textureSampleLevel(cam_grid, ph_lin, vec3<f32>(pos / (f32(CAM_CELL_PX) * gd.xy), gz), 0.0);
    let blur = select(gz, g.z / max(g.w, 1e-6), g.w > 1e-4);
    let bil = select(blur, g.x / max(g.y, 1e-6), g.y > 1e-5);
    let support = smoothstep(0.004, 0.06, g.y);
    let bn = mix(blur, bil, CAM_BILATERAL_SHARE * support);
    let base = CAM_GRID_LO + bn * span + log2(max(cam_state.gain, 1e-6));
    let mid = log2(0.18);
    let k = select(cam_state.k_shadow, cam_state.k_high, base > mid);
    let d = (gz - bil) * span * support;
    let ds = d / sqrt(1.0 + (d / CAM_DETAIL_SPAN) * (d / CAM_DETAIL_SPAN));
    return exp2((base - mid) * (clamp(k, 0.0, 1.0) - 1.0) + (cine.cam3.z - 1.0) * ds);
}

/// A robust contrast-adaptive sharpen of the resolved frame's luminance (after
/// AMD's RCAS): on the luminance range-compressed at the exposure (`l / (1 +
/// l)`), a negative lobe over the four neighbours as strong as it can be
/// without taking the pixel past its neighbourhood's range (no clipping, no
/// ringing on a silhouette, no halo on a lamp), `CAM_SHARPEN` of RCAS's limit,
/// halved where the pixel stands out of its neighbours as noise does, and off
/// in a flat gradient (`CAM_SHARPEN_GATE`: a clear sky's or a wall's faint
/// noise is not raised); the colour keeps its chromaticity, only its luminance
/// moves. Restores the detail the temporal resolve's history blend softens.
fn cam_sharpen(p: vec2<i32>, c: vec3<f32>, e: f32) -> vec3<f32> {
    let dims = vec2<i32>(textureDimensions(hdr));
    let lc0 = dot(c, CAM_LW) * e;
    let ec = lc0 / (1.0 + lc0);
    var n = array<f32, 4>(0.0, 0.0, 0.0, 0.0);
    let o = array<vec2<i32>, 4>(vec2<i32>(0, -1), vec2<i32>(-1, 0), vec2<i32>(1, 0), vec2<i32>(0, 1));
    var mn4 = 1.0;
    var mx4 = 0.0;
    var sum = 0.0;
    for (var k = 0; k < 4; k = k + 1) {
        let v = max(textureLoad(hdr, clamp(p + o[k], vec2<i32>(0), dims - 1), 0).rgb, vec3<f32>(0.0));
        let l0 = dot(v, CAM_LW) * e;
        let l = select(l0 / (1.0 + l0), ec, l0 != l0);
        mn4 = min(mn4, l);
        mx4 = max(mx4, l);
        sum = sum + l;
    }
    let mn = min(mn4, ec);
    let mx = max(mx4, ec);
    // RCAS's lobe: the strongest negative weight that keeps the result inside [mn, mx].
    let hit_min = mn / max(4.0 * mx4, 1e-6);
    let hit_max = (1.0 - mx) / min(4.0 * mn4 - 4.0, -1e-6);
    // its noise term: a pixel unlike the mean of its neighbours by its range's whole width.
    let nz = 1.0 - 0.5 * clamp(abs(0.25 * sum - ec) / max(mx - mn, 1e-6), 0.0, 1.0);
    let gate = smoothstep(CAM_SHARPEN_GATE.x, CAM_SHARPEN_GATE.y, (mx - mn) / max(mx, 1e-5));
    let lobe = max(-0.1875, min(max(-hit_min, hit_max), 0.0)) * CAM_SHARPEN * nz * gate;
    let ls = clamp((lobe * sum + ec) / (4.0 * lobe + 1.0), 0.0, 0.999);
    let ratio = (ls / (1.0 - ls)) / max(lc0, 1e-9);
    return c * select(1.0, clamp(ratio, 0.5, 2.0), lc0 > 1e-9);
}

/// The frame through the camera's lens: its lateral colour (`cam2.w` px of a
/// 1080-line frame, red against blue at the corner, red outward and blue
/// inward as the square of the radius; three taps a side across each colour's
/// band, so the fringe is a soft spectrum, not a doubled edge).
fn cam_lens(pos: vec2<f32>, dims: vec2<f32>) -> vec3<f32> {
    var c = textureLoad(hdr, vec2<i32>(pos), 0).rgb;
    let ca = cine.cam2.w * dims.y / 1080.0;
    if (!(ca > 0.0)) {
        return c;
    }
    let half = 0.5 * dims;
    let pc = pos - half;
    let k = 0.5 * ca * length(pc) / dot(half, half);
    var r = 0.0;
    var b = 0.0;
    for (var i = 0; i < 3; i = i + 1) {
        let f = 0.5 + 0.5 * f32(i);
        let wt = select(0.25, 0.5, i == 1);
        r = r + wt * textureSampleLevel(hdr, ph_lin, (pos + pc * k * f) / dims, 0.0).r;
        b = b + wt * textureSampleLevel(hdr, ph_lin, (pos - pc * k * f) / dims, 0.0).b;
    }
    return vec3<f32>(r, c.g, b);
}

/// A hexagon's signed distance (apothem `r`), turned by `rot` (cos, sin).
fn cam_hex(p0: vec2<f32>, r: f32, rot: vec2<f32>) -> f32 {
    let p = abs(vec2<f32>(rot.x * p0.x + rot.y * p0.y, -rot.y * p0.x + rot.x * p0.y));
    return max(dot(p, vec2<f32>(0.8660254, 0.5)), p.y) - r;
}

/// The sun's glare, exposed units, added before the curve: a six-ray
/// starburst (three aperture blades' diffraction) and a line of ghosts (the
/// lens's inter-reflections) through the frame's centre, both scaled by the
/// sun's measured energy over its disc (`cam_state.sun_e`: nothing when it is
/// hidden by a building or a deck, a little when it shows through haze).
fn cam_flare(pos: vec2<f32>, dims: vec2<f32>) -> vec3<f32> {
    let s = smoothstep(CAM_FLARE_EV.x, CAM_FLARE_EV.y, log2(max(cam_state.sun_e, 1e-6))) * cine.cam2.y;
    if (s <= 0.0) {
        return vec3<f32>(0.0);
    }
    let sc = skyu.vp * vec4<f32>(skyu.atmo.xyz, 0.0);
    if (sc.w <= 1e-6) {
        return vec3<f32>(0.0);
    }
    let suv = vec2<f32>(sc.x / sc.w * 0.5 + 0.5, 0.5 - sc.y / sc.w * 0.5);
    let sp = suv * dims;
    let px = dims.y / 1080.0;
    // the sun's colour (the key's chromaticity), unit luminance.
    let key = max(look.key.rgb, vec3<f32>(0.0));
    let col = mix(vec3<f32>(1.0), key / max(dot(key, CAM_LW), 1e-6), 0.6);
    // the starburst: three blades' rays, a few pixels wide, r^-1.35 out to a third of the frame.
    let d = pos - sp;
    let r = length(d);
    var star = 0.0;
    for (var k = 0; k < 3; k = k + 1) {
        let ang = 0.2618 + f32(k) * 1.0471976;
        let across = abs(dot(d, vec2<f32>(-sin(ang), cos(ang))));
        let w = (0.8 + 0.01 * r) * px;
        star = star + exp(-(across * across) / (w * w));
    }
    let len = 360.0 * px * (0.5 + 0.5 * s);
    let fall = pow(1.0 + r / (6.0 * px), -1.35) * (1.0 - smoothstep(0.25 * len, len, r));
    var out = col * (star * fall * 5.0);
    // the ghosts: discs of the aperture's hexagon along the line from the sun through
    // the centre (t: 0 the sun, 1 the centre's mirror), tinted by the coatings, soft-edged.
    let axis = vec2<f32>(0.5) - suv;
    let rot = vec2<f32>(cos(0.2618), sin(0.2618));
    for (var i = 0; i < 6; i = i + 1) {
        var g = vec4<f32>(0.0);
        var tint = vec3<f32>(0.0);
        switch i {
            case 0: { g = vec4<f32>(0.62, 26.0, 0.09, 0.0); tint = vec3<f32>(1.0, 0.62, 0.30); }
            case 1: { g = vec4<f32>(1.22, 64.0, 0.040, 0.0); tint = vec3<f32>(0.45, 1.0, 0.72); }
            case 2: { g = vec4<f32>(1.48, 16.0, 0.11, 0.0); tint = vec3<f32>(0.85, 0.50, 1.0); }
            case 3: { g = vec4<f32>(1.80, 105.0, 0.022, 0.0); tint = vec3<f32>(1.0, 0.70, 0.40); }
            case 4: { g = vec4<f32>(2.20, 38.0, 0.06, 0.0); tint = vec3<f32>(0.50, 0.85, 1.0); }
            default: { g = vec4<f32>(-0.32, 12.0, 0.07, 0.0); tint = vec3<f32>(0.9, 0.55, 1.0); }
        }
        let gc = (suv + axis * g.x) * dims;
        let rad = g.y * px;
        let h = cam_hex(pos - gc, rad, rot);
        let soft = 0.2 * rad;
        let body = 1.0 - smoothstep(-soft, soft, h);
        let rim = exp(-(h * h) / (0.012 * rad * rad));
        out = out + tint * (g.z * (0.55 * body + 0.45 * rim));
    }
    return out * s;
}

/// The camera model's display value of a pixel.
fn cine_camera(pos: vec2<f32>, dims: vec2<f32>) -> vec4<f32> {
    var c = cam_lens(pos, dims);
    if (any(c != c)) {
        c = vec3<f32>(0.0);
    }
    c = clamp(c, vec3<f32>(0.0), vec3<f32>(65504.0));
    let e0 = look.key.w;
    // the sharpen, before the lens spreads anything (the bloom is smooth already).
    c = cam_sharpen(vec2<i32>(pos), c, e0 * cam_state.gain);
    // the bloom, energy kept: the pixel gives up the share the lens spreads
    // (`cam_bloom_part`, as the prefilter took it) and the levels give it back.
    if (skyu.bloom.w > 0.5) {
        let b = max(textureSampleLevel(bloom_tex, ph_lin, pos / dims, 0.0).rgb, vec3<f32>(0.0));
        c = max(c - cam_bloom_part(c, e0), vec3<f32>(0.0)) + b;
    }
    // the optical fall-off: a share of cos^4 of the field angle.
    let tf = vec2<f32>(skyu.cluster.z * skyu.cluster.w, skyu.cluster.z);
    let a = (pos / dims * 2.0 - 1.0) * tf;
    let c2 = 1.0 / (1.0 + dot(a, a));
    c = c * (1.0 - CAM_VIGNETTE * (1.0 - c2 * c2));
    // the exposure (the hour's x the meter's), the local exposure, the sun's glare.
    var x = c * (e0 * cam_state.gain);
    x = x * cam_local(pos, log2(max(dot(c * e0, CAM_LW), 1e-12)));
    x = x + cam_flare(pos, dims);
    var d = to_srgb(cam_agx(x));
    // the hour's grade (cine_post.rs bakes the camera's keys when the model is on).
    d = textureSampleLevel(cine_lut, cine_samp, d * cine.grade.y + vec3<f32>(cine.grade.z), 0.0).rgb;
    // film grain: one luminance pattern a pixel across (about Gaussian: two uniforms), in the
    // shadows' upper half and the low mid-tones only: nothing over display 0.55 (a bright sky
    // stays clean) and little under 0.1 (a night sky stays a clean dark).
    let y = clamp(dot(d, CAM_LW), 0.0, 1.0);
    let seed = u32(max(look.post2.w, 0.0)) % 4096u;
    let gw = smoothstep(0.06, 0.2, y) * (1.0 - smoothstep(0.32, 0.55, y));
    let gn = (ph_rand(vec2<i32>(pos), seed, 5u) + ph_rand(vec2<i32>(pos), seed, 6u)) * 1.2247449 * cine.cam2.x * gw;
    d = clamp(d + vec3<f32>(gn), vec3<f32>(0.0), vec3<f32>(1.0));
    // Display dither: triangular, +-1 step of 8-bit (as `finish_color`).
    let amp = look.post2.y;
    let n = hash(pos, look.post2.w) + hash(pos.yx * 1.31, look.post2.w + 3.7) - 1.0;
    d = clamp(d + vec3<f32>(n * amp), vec3<f32>(0.0), vec3<f32>(1.0));
    // (`EO_CAM_DEBUG=1`) the meter's state as bars along the top: the sun's energy (log2,
    // 0 .. 10), the meter's gain (log2, -4 .. +4 about the middle), its metered mean (-12 .. +2),
    // the local exposure's shadow and highlight compression (0 .. 1), the range's dark end
    // (EV about grey, -10 .. 0).
    if (cine.cam2.z > 0.5 && pos.y < 60.0) {
        let row = u32(pos.y / 10.0);
        let w = dims.x * 0.5;
        var v = 0.0;
        if (row == 0u) {
            v = clamp(log2(max(cam_state.sun_e, 1e-6)) / 10.0, 0.0, 1.0);
        } else if (row == 1u) {
            v = clamp(log2(max(cam_state.gain, 1e-6)) / 8.0 + 0.5, 0.0, 1.0);
        } else if (row == 2u) {
            v = clamp((cam_state.mean + 12.0) / 14.0, 0.0, 1.0);
        } else if (row == 3u) {
            v = clamp(cam_state.k_shadow, 0.0, 1.0);
        } else if (row == 4u) {
            v = clamp(cam_state.k_high, 0.0, 1.0);
        } else {
            v = clamp(cam_state.lo / 10.0 + 1.0, 0.0, 1.0);
        }
        let tick = fract(pos.x / (w / select(10.0, 8.0, row == 1u) + 1e-6) + 1e-4) < 0.01;
        d = select(vec3<f32>(0.05), select(vec3<f32>(1.0, 0.3, 0.2), vec3<f32>(0.3, 1.0, 0.4), row == 1u), pos.x < v * w);
        if (row == 2u && pos.x < v * w) {
            d = vec3<f32>(0.3, 0.5, 1.0);
        }
        if (tick) {
            d = vec3<f32>(1.0);
        }
    }
    if (look.post2.z > 0.5) {
        d = from_srgb(d);
    }
    return vec4<f32>(d, 1.0);
}
