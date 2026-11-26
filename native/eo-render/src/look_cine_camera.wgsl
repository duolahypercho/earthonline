// The camera model's post path (`--cine-camera`, camera track; cine_post.rs
// lists the steps). Composed after look_cine.wgsl (its bindings, the grade's
// table), the photo finish's noise source (`ph_rand`), and
// cine_post.rs's camera constants (`CAM_*`), `cam_bloom_part` and the
// meter's bindings (`cam_state`, `cam_grid`). Called from `cine_fs` only when
// the model is compiled in (`SF_CAMERA`) and on.

const CAM_LW: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

/// The camera's response in log2 about middle grey (cine_post.rs `tone_log`):
/// the contrast `CAM_CONTRAST` from the mid-tones up, easing to `CAM_TOE.x` in
/// the deep shadows (a softplus about `CAM_TOE.y` EV, `CAM_TOE.z` EV wide), so
/// the shadows roll into black over several stops as a cine camera's do.
/// (`--camera-print`) the deep shadows' slope and the curve's constant by the hour
/// (`cam8`: a print's black by day, the long toe at night).
fn cam_tone_log(u: vec3<f32>) -> vec3<f32> {
    let x = (u - vec3<f32>(CAM_TOE.y)) / CAM_TOE.z;
    let sp = max(x, vec3<f32>(0.0)) + log(vec3<f32>(1.0) + exp(-abs(x)));
    var lo = CAM_TOE.x;
    var k = CAM_TOE_K;
    if (SF_CAM_PRINT) {
        lo = cine.cam8.x;
        k = cine.cam8.y;
    }
    return lo * u + (CAM_CONTRAST - lo) * CAM_TOE.z * sp - vec3<f32>(k);
}

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
    // the camera's response about middle grey (`cam_tone_log`: a film's gamma over the
    // sigmoid's own in the mid-tones, a long toe under them).
    let g = log2(0.18);
    v = vec3<f32>(g) + cam_tone_log(log2(max(v, vec3<f32>(1e-10))) - vec3<f32>(g));
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

/// (`--camera-tone`) the display gamut's soft edge (cine_post.rs `gamut`), display
/// linear in and out: a colour whose brightest channel passes the knee
/// (`CAM_TONE_GAMUT.x`) is brought under the rational curve that reaches 1 at
/// `CAM_TONE_GAMUT.y`, first by darkening it (`CAM_TONE_GAMUT.z` of the excess in
/// log, x its saturation: a grey is never darkened), then by desaturating it toward
/// its own luminance; never a channel clipped alone (the hour's warm balance on a
/// golden sky keeps its hue and gradient instead of an R 255 plateau).
fn cam_gamut(c0: vec3<f32>) -> vec3<f32> {
    var c = max(c0, vec3<f32>(0.0));
    let mx0 = max(c.r, max(c.g, c.b));
    let knee = CAM_TONE_GAMUT.x;
    if (!(mx0 > knee)) {
        return c;
    }
    let mn0 = min(c.r, min(c.g, c.b));
    let lim = CAM_TONE_GAMUT.y;
    let sk = (1.0 - knee) * (lim - knee) / ((lim - knee) - (1.0 - knee));
    let t = knee + (mx0 - knee) / (1.0 + (mx0 - knee) / sk);
    c = c * pow(t / mx0, CAM_TONE_GAMUT.z * (mx0 - mn0) / mx0);
    let l = dot(c, CAM_LW);
    let mx = max(c.r, max(c.g, c.b));
    if (l >= t) {
        return vec3<f32>(min(l, 1.0));
    }
    let k = select(1.0, min((t - l) / (mx - l), 1.0), mx > l + 1e-7);
    return clamp(vec3<f32>(l) + (c - vec3<f32>(l)) * k, vec3<f32>(0.0), vec3<f32>(1.0));
}

/// The local exposure's gain at a pixel of log2 luminance `lx` (exposed by the
/// hour's exposure, the grid's units). The base luminance round it (the
/// bilateral grid sliced at its own luminance, blended with the wider blur of
/// the grid's columns) is compressed under middle grey by the meter's
/// `k_shadow`, and over the highlights' knee (`cam3.w`) by its `k_high` (1 when
/// the frame's range fits the display: then the base keeps its contrast; over
/// 1 at night when the brightest sources fall short); the pixel's detail against the edge-aware
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
    let rb = base - log2(0.18);
    let knee = cine.cam3.w;
    var shift = 0.0;
    if (rb < 0.0) {
        shift = rb * (clamp(cam_state.k_shadow, 0.0, 1.0) - 1.0);
    } else if (rb > knee) {
        // (an expansion, over 1, is the pixel's own: `cam_push`)
        shift = (rb - knee) * (clamp(cam_state.k_high, 0.0, 1.0) - 1.0);
    }
    let d = (gz - bil) * span * support;
    let ds = d / sqrt(1.0 + (d / CAM_DETAIL_SPAN) * (d / CAM_DETAIL_SPAN));
    return exp2(shift + (cine.cam3.z - 1.0) * ds);
}

/// (night) the highlights pushed: when even the frame's brightest sources fall
/// short of `cam4.y` the meter's `k_high` is over 1 and a pixel's own exposed
/// luminance over the knee (`cam3.w`) is expanded by it, as a pushed stock's
/// steeper curve: a lamp's core, its glare and a lit window's glint burn toward
/// white while everything under the knee keeps its level. On the pixel, not the
/// local base: a lamp a few pixels across is detail to the bilateral grid.
fn cam_push(x: vec3<f32>) -> vec3<f32> {
    let k = clamp(cam_state.k_high, 1.0, 2.0);
    let u = log2(max(dot(x, CAM_LW), 1e-12) / 0.18) - cine.cam3.w;
    if (k <= 1.0 || u <= 0.0) {
        return x;
    }
    return x * exp2(u * (k - 1.0));
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
/// (`--camera-sharpen`) and on top an unsharp mask of the 3 x 3 neighbourhood
/// (`cam5.y` of the pixel's difference from its binomial blur, the four diagonal
/// neighbours read as well), held inside the 3 x 3 range widened by
/// `CAM_SHARPEN_OVERSHOOT` of it, gated and halved on noise as the lobe is.
fn cam_sharpen(p: vec2<i32>, c: vec3<f32>, e: f32, strength: f32) -> vec3<f32> {
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
    let lobe = max(-0.1875, min(max(-hit_min, hit_max), 0.0)) * strength * nz * gate;
    var ls = clamp((lobe * sum + ec) / (4.0 * lobe + 1.0), 0.0, 0.999);
    if (SF_CAM_SHARPEN && cine.cam5.y > 0.0) {
        let od = array<vec2<i32>, 4>(vec2<i32>(-1, -1), vec2<i32>(1, -1), vec2<i32>(-1, 1), vec2<i32>(1, 1));
        var dsum = 0.0;
        var mn9 = mn;
        var mx9 = mx;
        for (var k = 0; k < 4; k = k + 1) {
            let v = max(textureLoad(hdr, clamp(p + od[k], vec2<i32>(0), dims - 1), 0).rgb, vec3<f32>(0.0));
            let l0 = dot(v, CAM_LW) * e;
            let l = select(l0 / (1.0 + l0), ec, l0 != l0);
            mn9 = min(mn9, l);
            mx9 = max(mx9, l);
            dsum = dsum + l;
        }
        let blur = (4.0 * ec + 2.0 * sum + dsum) / 16.0;
        let ov = CAM_SHARPEN_OVERSHOOT * (mx9 - mn9);
        ls = clamp(ls + cine.cam5.y * nz * gate * (ec - blur), max(mn9 - ov, 0.0), min(mx9 + ov, 0.999));
    }
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

/// The blur circle's diameter (px of this frame) of a pixel whose resolved
/// alpha is `a` (2 + its distance in metres, the sky 65504; under 2: no
/// distance, no blur): a lens on a `CAM_SENSOR_MM` sensor whose focal length the
/// frame's vertical field gives, at f-number `cam4.w`, focused at the meter's
/// `focus` (dioptres): (f^2 / N) |1/S - 1/D| on the sensor.
fn cam_coc(a: f32, dims: vec2<f32>) -> f32 {
    if (!(a >= 2.0) || !(cine.cam4.w > 0.0) || cam_state.focus_ok < 0.5) {
        return 0.0;
    }
    let dd = select(1.0 / max(a - 2.0, 0.05), 0.0, a > 60000.0);
    let f_mm = 0.5 * CAM_SENSOR_MM / max(skyu.cluster.z, 1e-3);
    let px_per_dioptre = f_mm * f_mm / cine.cam4.w * 0.001 / CAM_SENSOR_MM * dims.y;
    return min(px_per_dioptre * abs(dd - cam_state.focus), CAM_DOF_MAX_PX * dims.y / 1080.0);
}

/// The depth of field at a pixel whose blur circle is `coc0` px across: a
/// gather over `CAM_DOF_TAPS` taps on a Vogel disc of its radius, each tap
/// counted where its own circle reaches this pixel (scatter as gather), one
/// farther than the pixel only as far as the pixel's own circle (a sharp
/// background does not bleed into a blurred foreground's edge), in the scene's
/// linear light (a bright point keeps its energy).
fn cam_dof(pos: vec2<f32>, dims: vec2<f32>, c0: vec3<f32>, a0: f32, coc0: f32) -> vec3<f32> {
    let r0 = 0.5 * coc0;
    let dmax = vec2<i32>(dims) - vec2<i32>(1);
    var acc = c0;
    var wsum = 1.0;
    for (var i = 0; i < CAM_DOF_TAPS; i = i + 1) {
        let rr = r0 * sqrt((f32(i) + 0.5) / f32(CAM_DOF_TAPS));
        let th = f32(i) * 2.3999632;
        let q = pos + vec2<f32>(cos(th), sin(th)) * rr;
        let a = textureLoad(hdr, clamp(vec2<i32>(q), vec2<i32>(0), dmax), 0).a;
        var rt = 0.5 * cam_coc(a, dims);
        if (a > a0) {
            rt = min(rt, r0);
        }
        let w = smoothstep(rr - 0.5, rr + 0.5, rt);
        acc = acc + max(textureSampleLevel(hdr, ph_lin, q / dims, 0.0).rgb, vec3<f32>(0.0)) * w;
        wsum = wsum + w;
    }
    return acc / wsum;
}

/// A hexagon's signed distance (apothem `r`), turned by `rot` (cos, sin).
fn cam_hex(p0: vec2<f32>, r: f32, rot: vec2<f32>) -> f32 {
    let p = abs(vec2<f32>(rot.x * p0.x + rot.y * p0.y, -rot.y * p0.x + rot.x * p0.y));
    return max(dot(p, vec2<f32>(0.8660254, 0.5)), p.y) - r;
}

/// The sun's glare, exposed units, added before the curve, all scaled by the
/// sun's measured energy over its disc (`cam_state.sun_e`: nothing when it is
/// hidden by a building or a deck, a little when it shows through haze):
/// - the veiling glare (`cam4.z` at its core): the lens's scatter round the
///   source, a hot core over the disc and a long skirt falling as about 1/r^2
///   over the frame, so a wall beside the sun loses contrast toward it;
/// - a six-ray starburst (three aperture blades' diffraction), soft and short,
///   only once the disc is many stops over white;
/// - a line of ghosts (the lens's inter-reflections) through the frame's centre:
///   the aperture's hexagon, a faint body and a rim split by the coatings'
///   dispersion (blue inside, red outside), tracking the sun across the axis.
fn cam_flare(pos: vec2<f32>, dims: vec2<f32>) -> vec3<f32> {
    let le = log2(max(cam_state.sun_e, 1e-6));
    let s = smoothstep(CAM_FLARE_EV.x, CAM_FLARE_EV.y, le) * cine.cam2.y;
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
    let d = pos - sp;
    let r = length(d) / px;
    // the veiling glare: compact core and tighter skirt so nearby buildings stay crisp.
    let core = pow(1.0 + (r / 6.0) * (r / 6.0), -1.6);
    let skirt = 1.0 / (1.0 + (r / 18.0) * (r / 18.0));
    var out = (mix(col, vec3<f32>(1.0), 0.3 * core) * (1.2 * core) + col * (0.55 * skirt)) * cine.cam4.z;
    // the starburst: subtle three-blade diffraction rays, kept compact.
    let ss = smoothstep(CAM_FLARE_EV.x + 1.5, CAM_FLARE_EV.y + 1.0, le);
    var star = 0.0;
    for (var k = 0; k < 3; k = k + 1) {
        let ang = 0.2618 + f32(k) * 1.0471976;
        let across = abs(dot(d, vec2<f32>(-sin(ang), cos(ang)))) / px;
        let w = 1.1 + 0.008 * r;
        star = star + exp(-(across * across) / (w * w));
    }
    let len = 140.0 * (0.5 + 0.5 * ss);
    let fall = pow(1.0 + r / 6.0, -1.8) * (1.0 - smoothstep(0.3 * len, len, r));
    out = out + col * (star * fall * 0.65 * ss);
    // coated-prime ghosts kept very faint so artificial colored hexagon rings do not clutter the view.
    let axis = vec2<f32>(0.5) - suv;
    let rot = vec2<f32>(cos(0.2618), sin(0.2618));
    for (var i = 0; i < 6; i = i + 1) {
        var g = vec4<f32>(0.0);
        var tint = vec3<f32>(0.0);
        switch i {
            case 0: { g = vec4<f32>(0.62, 26.0, 0.07, 0.0); tint = vec3<f32>(1.0, 0.62, 0.30); }
            case 1: { g = vec4<f32>(1.22, 64.0, 0.030, 0.0); tint = vec3<f32>(0.45, 1.0, 0.72); }
            case 2: { g = vec4<f32>(1.48, 16.0, 0.08, 0.0); tint = vec3<f32>(0.85, 0.50, 1.0); }
            case 3: { g = vec4<f32>(1.80, 105.0, 0.016, 0.0); tint = vec3<f32>(1.0, 0.70, 0.40); }
            case 4: { g = vec4<f32>(2.20, 38.0, 0.045, 0.0); tint = vec3<f32>(0.50, 0.85, 1.0); }
            default: { g = vec4<f32>(-0.32, 12.0, 0.05, 0.0); tint = vec3<f32>(0.9, 0.55, 1.0); }
        }
        let gc = (suv + axis * g.x) * dims;
        let rad = g.y * px;
        let h = cam_hex(pos - gc, rad, rot);
        let soft = 0.28 * rad;
        let body = 1.0 - smoothstep(-soft, soft, h);
        let rim = exp(-(h * h) / (0.04 * rad * rad));
        let disp = mix(vec3<f32>(0.55, 0.75, 1.0), vec3<f32>(1.0, 0.6, 0.35), smoothstep(-0.12 * rad, 0.12 * rad, h));
        out = out + tint * (0.18 * g.z * select(1.0, CAM_FLARE_GHOST_GAIN, SF_CAM_FLARE) * (0.45 * body + 0.55 * rim * disp));
    }
    return out * s;
}

/// The camera model's display value of a pixel.
fn cine_camera(pos: vec2<f32>, dims: vec2<f32>) -> vec4<f32> {
    var c = cam_lens(pos, dims);
    // keep gameplay crisp: only apply subtle shallow-focus soften past a 1.25 px circle of confusion.
    let a0 = textureLoad(hdr, vec2<i32>(pos), 0).a;
    let coc0 = cam_coc(a0, dims);
    if (coc0 > 1.25 && cine.cam2.z < 1.5) {
        c = cam_dof(pos, dims, c, a0, coc0);
    }
    if (any(c != c)) {
        c = vec3<f32>(0.0);
    }
    c = clamp(c, vec3<f32>(0.0), vec3<f32>(65504.0));
    let e0 = look.key.w;
    // (`EO_CAM_DEBUG=2`) the scene's log2 luminance exposed by the hour's exposure (before the
    // lens spreads anything and before the meter), over -16 .. +10 in two 8-bit channels (red
    // the coarse, green the fine part): the frame's own luminance, read back for tuning.
    if (cine.cam2.z > 1.5) {
        let t = clamp((log2(max(dot(c, CAM_LW) * e0, 1e-12)) + 16.0) / 26.0, 0.0, 1.0) * 255.0;
        var q = vec3<f32>(floor(t) / 255.0, fract(t), 0.0);
        if (look.post2.z > 0.5) {
            q = from_srgb(q);
        }
        return vec4<f32>(q, 1.0);
    }
    // the sharpen, before the lens spreads anything (the bloom is smooth already); not where
    // the focus blurs (it reads the unblurred neighbours).
    // (`--camera-sharpen`: stronger, `cam5.w`, with its unsharp band)
    c = mix(cam_sharpen(vec2<i32>(pos), c, e0 * cam_state.gain, select(CAM_SHARPEN, cine.cam5.w, SF_CAM_SHARPEN)), c, smoothstep(0.6, 1.5, coc0));
    // the bloom, energy kept: the pixel gives up the share the lens spreads
    // (`cam_bloom_part`, as the prefilter took it) and the levels give it back.
    if (skyu.bloom.w > 0.5) {
        let b = max(textureSampleLevel(bloom_tex, ph_lin, pos / dims, 0.0).rgb, vec3<f32>(0.0));
        c = max(c - cam_bloom_part(c, e0), vec3<f32>(0.0)) + b;
    }
    // (`--white-balance`) the camera's balance on the scene's light (cine_camera.wgsl `cam_adapt`).
    // (and the hour's preset, `cam7`: a warm golden hour, a cool blue hour, in the scene before the curve)
    var wbg = vec3<f32>(1.0);
    if (SF_CAM_WB) {
        if (cam_state.wb_ok > 0.5) {
            wbg = clamp(vec3<f32>(cam_state.wb_r, cam_state.wb_g, cam_state.wb_b), vec3<f32>(0.25), vec3<f32>(4.0));
        }
        if (cine.cam7.w > 0.0) {
            wbg = wbg * clamp(cine.cam7.xyz, vec3<f32>(0.5), vec3<f32>(2.0));
        }
        c = c * wbg;
        // the hour's saturation as the camera's colour rendering (about the colour's luminance;
        // the display transform's gamut edge takes what passes it)
        if (cine.cam7.w > 0.0) {
            let yl = dot(c, CAM_LW);
            c = vec3<f32>(yl) + (c - vec3<f32>(yl)) * clamp(cine.cam7.w, 0.5, 1.5);
        }
    }
    // the optical fall-off: a share of cos^4 of the field angle.
    let tf = vec2<f32>(skyu.cluster.z * skyu.cluster.w, skyu.cluster.z);
    let a = (pos / dims * 2.0 - 1.0) * tf;
    let c2 = 1.0 / (1.0 + dot(a, a));
    c = c * (1.0 - CAM_VIGNETTE * (1.0 - c2 * c2));
    // the exposure (the hour's x the meter's), the local exposure, the sun's glare.
    var x = c * (e0 * cam_state.gain);
    x = x * cam_local(pos, log2(max(dot(c * e0, CAM_LW), 1e-12)));
    x = cam_push(x);
    if (SF_CAM_TONE) {
        // (`--camera-tone`) the shade's colour toward neutral (`cam5.z` of its chroma kept from
        // `CAM_TONE_SHADE_EV.x` EV about grey down, all of it from `.y` up): an open shade's sky
        // blue and a deck's orange bounce read near grey against the warm light.
        let yx = dot(x, CAM_LW);
        let ks = mix(cine.cam5.z, 1.0, smoothstep(CAM_TONE_SHADE_EV.x, CAM_TONE_SHADE_EV.y, log2(max(yx, 1e-12) / 0.18)));
        x = max(vec3<f32>(yx) + (x - vec3<f32>(yx)) * ks, vec3<f32>(0.0));
        // the lens's veiling glare as a floor: `cam5.x` of the metered mean (exposed).
        x = x + vec3<f32>(cine.cam5.x * exp2(clamp(cam_state.mean, -20.0, 8.0)) * cam_state.gain);
    }
    x = x + cam_flare(pos, dims) * wbg;
    // (`--lens-flare`) the point sources' diffraction spikes (look_cine_flare.wgsl).
    if (SF_CAM_FLARE) {
        x = x + cam_spikes(pos, dims) * wbg;
    }
    var d = to_srgb(cam_agx(x));
    // the hour's grade (cine_post.rs bakes the camera's keys when the model is on).
    d = textureSampleLevel(cine_lut, cine_samp, d * cine.grade.y + vec3<f32>(cine.grade.z), 0.0).rgb;
    // (`--camera-tone`) the table is open at the top: its colour brought into the display's
    // gamut by the soft edge (`cam_gamut`), not clipped a channel at a time.
    if (SF_CAM_TONE) {
        d = to_srgb(cam_gamut(from_srgb(max(d, vec3<f32>(0.0)))));
    }
    // film grain: one luminance pattern a little over a pixel across (`ph_grain`, about
    // Gaussian), in the shadows and the low mid-tones (`cam2.x` at its peak: more at night):
    // nothing over display 0.65 (a bright sky stays clean), fading under 0.12 (black stays black).
    let y = clamp(dot(d, CAM_LW), 0.0, 1.0);
    let seed = u32(max(look.post2.w, 0.0)) % 4096u;
    let gw = smoothstep(0.02, 0.12, y) * (1.0 - smoothstep(0.35, 0.65, y));
    if (SF_CAM_TONE) {
        // (`--camera-tone`) the sensor's noise follows its gain (the meter's lift over the
        // hour's exposure, whose own grain is the night's or the day's: `cam2.x`); luminance only.
        let amp = cine.cam2.x * sqrt(clamp(cam_state.gain, 0.5, 3.0)) * gw;
        d = clamp(d + vec3<f32>(ph_grain(vec2<i32>(pos), seed, 5u) * amp), vec3<f32>(0.0), vec3<f32>(1.0));
    } else {
        let gn = ph_grain(vec2<i32>(pos), seed, 5u) * cine.cam2.x * gw;
        d = clamp(d + vec3<f32>(gn), vec3<f32>(0.0), vec3<f32>(1.0));
    }
    // Display dither: triangular, +-1 step of 8-bit (as `finish_color`).
    let amp = look.post2.y;
    let n = hash(pos, look.post2.w) + hash(pos.yx * 1.31, look.post2.w + 3.7) - 1.0;
    d = clamp(d + vec3<f32>(n * amp), vec3<f32>(0.0), vec3<f32>(1.0));
    // (`EO_CAM_DEBUG=1`) the meter's state as bars along the top: the sun's energy (log2,
    // 0 .. 10), the meter's gain (log2, -4 .. +4 about the middle), its metered mean (-12 .. +2),
    // the local exposure's shadow compression (0 .. 1) and highlight compression (0 .. 2), the range's dark end
    // (EV about grey, -10 .. 0), the focus (dioptres, 0 .. 1).
    if (cine.cam2.z > 0.5 && pos.y < 70.0) {
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
            v = clamp(0.5 * cam_state.k_high, 0.0, 1.0);
        } else if (row == 5u) {
            v = clamp(cam_state.lo / 10.0 + 1.0, 0.0, 1.0);
        } else {
            v = clamp(cam_state.focus, 0.0, 1.0);
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
