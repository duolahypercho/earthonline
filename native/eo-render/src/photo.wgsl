// Photo mode: a thin-lens depth of field and the photo finish (exposure
// compensation, white balance, grade, natural vignetting, film grain,
// letterbox), between the temporal resolve and post. See photo.rs.
//
// Units: a circle of confusion is a signed RADIUS in half-resolution
// pixels (positive behind the focus plane, negative in front of it).

struct Photo {
    // W, H, 1 / W, 1 / H (the output, full resolution)
    size: vec4<f32>,
    // half-resolution W, H; tiles across, down
    half: vec4<f32>,
    // CoC radius at infinity (half px), focus distance (m), near, far (m)
    lens: vec4<f32>,
    // largest radius (half px), this frame's pattern turn (rad), accumulation weight (1: none), flags (1 dof, 2 bloom here)
    lens2: vec4<f32>,
    // exposure multiplier, saturation, log contrast (gamma about 18 % grey), monochrome
    grade: vec4<f32>,
    // the white-balance matrix, rows (linear Rec. 709)
    wb0: vec4<f32>,
    wb1: vec4<f32>,
    wb2: vec4<f32>,
    // natural vignetting share, sensor width / f, sensor height / f, grain sigma (stops)
    vig: vec4<f32>,
    // letterbox image aspect (0: off), grain seed, grain colour share, radial jitter (0..1)
    frame: vec4<f32>,
}

@group(0) @binding(0) var<uniform> P: Photo;

const PI: f32 = 3.14159265;
// the gather's ring spacing (half px) and its most rings (taps 1 + 4 K (K + 1)).
const SPACING: f32 = 1.0;
const K_MAX: i32 = 8;
// half px: a radius under this is in focus; over twice it, wholly blurred.
const SHARP: f32 = 0.5;

fn view_depth(d: f32) -> f32 {
    // the reversed-Z projection (camera.rs): d = n / (f - n) (f / z - 1).
    let n = P.lens.z;
    let f = P.lens.w;
    return n * f / max(d * (f - n) + n, 1e-6);
}

// thin lens (Hecht, Optics, 5th ed., 5.2): blur-circle diameter
// c = A f |z - s| / (z (s - f)) with A = f / N, i.e. c(z) = c_inf (1 - s / z).
fn coc(z: f32) -> f32 {
    let r = P.lens.x * (1.0 - P.lens.y / max(z, 1e-3));
    return clamp(r, -P.lens2.x, P.lens2.x);
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// ---------------------------------------------------------------------------
// 1. Prep: half resolution, two depth-aware downsamples. The far one keeps
//    the farthest of each 2 x 2 (and the colours near its blur), the near one
//    the nearest, so an in-focus edge never smears into either layer.
// ---------------------------------------------------------------------------

@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var depth: texture_depth_2d;
@group(0) @binding(3) var prep_far_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(4) var prep_near_out: texture_storage_2d<rgba16float, write>;

@compute @workgroup_size(8, 8)
fn prep(@builtin(global_invocation_id) id: vec3<u32>) {
    let hs = vec2<u32>(P.half.xy);
    if (id.x >= hs.x || id.y >= hs.y) {
        return;
    }
    let full = vec2<i32>(P.size.xy) - 1;
    var cs: array<vec3<f32>, 4>;
    var ks: array<f32, 4>;
    var kf = -1e9;
    var kn = 1e9;
    for (var i = 0; i < 4; i++) {
        let p = min(vec2<i32>(id.xy) * 2 + vec2<i32>(i & 1, i >> 1), full);
        cs[i] = min(max(textureLoad(src, p, 0).rgb, vec3<f32>(0.0)), vec3<f32>(65000.0));
        ks[i] = coc(view_depth(textureLoad(depth, p, 0)));
        kf = max(kf, ks[i]);
        kn = min(kn, ks[i]);
    }
    var cf = vec3<f32>(0.0);
    var wf = 0.0;
    var cn = vec3<f32>(0.0);
    var wn = 0.0;
    for (var i = 0; i < 4; i++) {
        let a = 1.0 - saturate(abs(ks[i] - kf) / max(0.5, 0.25 * abs(kf)));
        let b = 1.0 - saturate(abs(ks[i] - kn) / max(0.5, 0.25 * abs(kn)));
        cf += cs[i] * a;
        wf += a;
        cn += cs[i] * b;
        wn += b;
    }
    textureStore(prep_far_out, vec2<i32>(id.xy), vec4<f32>(cf / max(wf, 1e-4), kf));
    textureStore(prep_near_out, vec2<i32>(id.xy), vec4<f32>(cn / max(wn, 1e-4), kn));
}

// ---------------------------------------------------------------------------
// 2. Tiles (8 x 8 half px): the largest near and far radius in each.
// 3. Dilate: the near radius of any tile whose blur can reach this one.
// ---------------------------------------------------------------------------

@group(0) @binding(5) var prep_far: texture_2d<f32>;
@group(0) @binding(6) var prep_near: texture_2d<f32>;
@group(0) @binding(7) var tiles_out: texture_storage_2d<rg32float, write>;

@compute @workgroup_size(8, 8)
fn tiles(@builtin(global_invocation_id) id: vec3<u32>) {
    let nt = vec2<u32>(P.half.zw);
    if (id.x >= nt.x || id.y >= nt.y) {
        return;
    }
    let hs = vec2<i32>(P.half.xy) - 1;
    var near = 0.0;
    var far = 0.0;
    for (var y = 0; y < 8; y++) {
        for (var x = 0; x < 8; x++) {
            let q = min(vec2<i32>(id.xy) * 8 + vec2<i32>(x, y), hs);
            near = max(near, -textureLoad(prep_near, q, 0).a);
            far = max(far, textureLoad(prep_far, q, 0).a);
        }
    }
    textureStore(tiles_out, vec2<i32>(id.xy), vec4<f32>(near, far, 0.0, 0.0));
}

@group(0) @binding(8) var tiles_in: texture_2d<f32>;

@compute @workgroup_size(8, 8)
fn dilate(@builtin(global_invocation_id) id: vec3<u32>) {
    let nt = vec2<i32>(P.half.zw);
    if (i32(id.x) >= nt.x || i32(id.y) >= nt.y) {
        return;
    }
    let reach = i32(ceil(P.lens2.x / 8.0));
    var near = 0.0;
    for (var y = -reach; y <= reach; y++) {
        for (var x = -reach; x <= reach; x++) {
            let t = vec2<i32>(id.xy) + vec2<i32>(x, y);
            if (any(t < vec2<i32>(0)) || any(t >= nt)) {
                continue;
            }
            let r = textureLoad(tiles_in, t, 0).x;
            // the nearest pixel of that tile is (chebyshev - 1) tiles away at least.
            let gap = f32(max(max(abs(x), abs(y)) - 1, 0)) * 8.0;
            if (r >= gap) {
                near = max(near, r);
            }
        }
    }
    let own = textureLoad(tiles_in, vec2<i32>(id.xy), 0);
    textureStore(tiles_out, vec2<i32>(id.xy), vec4<f32>(near, own.y, 0.0, 0.0));
}

// ---------------------------------------------------------------------------
// 4. Gather (half resolution): scatter-as-gather over rings of taps, each
//    tap standing for the same area (pi d^2 / 4 for ring spacing d).
//    Far layer: a tap adds where its own blur disc reaches the centre and
//    the centre's disc reaches it (min of the two radii): a sharper surface
//    in front never spreads onto a blurred background (no halo), a blurred
//    background never leaks over a sharper surface in front of it. Each tap
//    spreads its energy over its disc (weight 1 / r^2).
//    Near layer: in-front taps spread over whatever is behind them, with
//    their coverage as alpha (the share of the centre their discs cover).
//    In front of the focus plane the far layer gathers the background
//    around the centre instead: what the blurred foreground's edge lets through.
// ---------------------------------------------------------------------------

@group(0) @binding(9) var lin: sampler;
@group(0) @binding(10) var far_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(11) var near_out: texture_storage_2d<rgba16float, write>;

@compute @workgroup_size(8, 8)
fn gather(@builtin(global_invocation_id) id: vec3<u32>) {
    let hs = P.half.xy;
    if (f32(id.x) >= hs.x || f32(id.y) >= hs.y) {
        return;
    }
    let q = vec2<i32>(id.xy);
    let cf = textureLoad(prep_far, q, 0);
    let tile = textureLoad(tiles_in, q / 8, 0).x;
    let fill = cf.a < -SHARP;
    let r_far = select(max(cf.a, 0.0), min(-cf.a, P.lens2.x), fill);
    let r_near = select(0.0, tile, tile >= SHARP);
    let r_all = max(r_far, r_near);
    if (r_all < SHARP) {
        textureStore(far_out, q, vec4<f32>(cf.rgb, 1.0));
        textureStore(near_out, q, vec4<f32>(0.0));
        return;
    }
    let k = clamp(i32(ceil(r_all / SPACING)), 1, K_MAX);
    let d = r_all / f32(k);
    let inv = 1.0 / hs;
    let centre = (vec2<f32>(q) + 0.5);
    var far_c = vec3<f32>(0.0);
    var far_w = 0.0;
    var near_c = vec3<f32>(0.0);
    var near_w = 0.0;
    let want_near = r_near >= SHARP;
    for (var ring = 0; ring <= k; ring++) {
        let n = max(1, 8 * ring);
        // the rings' radii and turns move from frame to frame while the view is still (accumulated).
        let r = (f32(ring) + select(0.0, (P.frame.w - 0.5) * 0.9, ring > 0)) * d;
        let turn = P.lens2.y + f32(ring) * 0.61803 * PI;
        for (var t = 0; t < n; t++) {
            let a = turn + f32(t) * (2.0 * PI / f32(n));
            let o = vec2<f32>(cos(a), sin(a)) * r;
            let uv = (centre + o) * inv;
            let sf = textureSampleLevel(prep_far, lin, uv, 0.0);
            if (sf.a >= -SHARP) {
                if (fill) {
                    if (r <= r_far) {
                        far_c += sf.rgb;
                        far_w += 1.0;
                    }
                } else {
                    let rs = max(sf.a, 0.0);
                    let cov = saturate(min(rs, r_far) - r + 0.5);
                    let w = cov / max(rs * rs, 0.25);
                    far_c += sf.rgb * w;
                    far_w += w;
                }
            }
            if (want_near) {
                let sn = textureSampleLevel(prep_near, lin, uv, 0.0);
                let rn = -sn.a;
                if (rn >= SHARP) {
                    let w = saturate(rn - r + 0.5) / (rn * rn);
                    near_c += sn.rgb * w;
                    near_w += w;
                }
            }
        }
    }
    let far_rgb = select(cf.rgb, far_c / max(far_w, 1e-8), far_w > 0.0);
    // coverage: each tap's area (pi d^2 / 4) over its disc's (pi r^2), summed.
    let alpha = saturate(near_w * d * d * 0.25);
    let near_rgb = select(vec3<f32>(0.0), near_c / max(near_w, 1e-8), near_w > 0.0);
    textureStore(far_out, q, vec4<f32>(far_rgb, 1.0));
    textureStore(near_out, q, vec4<f32>(near_rgb, alpha));
}

// ---------------------------------------------------------------------------
// 5. Finish (full resolution): the sharp frame, the far layer (upsampled by
//    blur likeness) and the near layer over it; accumulated while the view
//    is still; then the photo finish.
// ---------------------------------------------------------------------------

@group(0) @binding(12) var far_layer: texture_2d<f32>;
@group(0) @binding(13) var near_layer: texture_2d<f32>;
@group(0) @binding(14) var history: texture_2d<f32>;
@group(0) @binding(15) var history_out: texture_storage_2d<rgba32float, write>;
@group(0) @binding(16) var photo_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(17) var bloom: texture_2d<f32>;

fn hash(p: vec2<f32>, k: f32) -> f32 {
    let q = fract(p * vec2<f32>(0.1031, 0.1030) + k * 0.0973);
    let r = q + dot(q, q.yx + 33.33);
    return fract((r.x + r.y) * r.x);
}

// a standard normal from four uniforms (Irwin-Hall, n = 4).
fn gauss(p: vec2<f32>, k: f32) -> f32 {
    let s = hash(p, k) + hash(p, k + 17.0) + hash(p, k + 41.0) + hash(p, k + 73.0);
    return (s - 2.0) * 1.7320508;
}

fn finish(c0: vec3<f32>, p: vec2<f32>) -> vec3<f32> {
    var c = c0 * P.grade.x;
    c = vec3<f32>(dot(P.wb0.xyz, c), dot(P.wb1.xyz, c), dot(P.wb2.xyz, c));
    c = max(c, vec3<f32>(0.0));
    let y = luma(c);
    // log contrast about 18 % grey (a film's gamma), the hue kept.
    if (P.grade.z != 1.0 && y > 0.0) {
        let y2 = 0.18 * pow(y / 0.18, P.grade.z);
        c = c * (y2 / y);
    }
    let y2 = luma(c);
    c = max(mix(vec3<f32>(y2), c, P.grade.y), vec3<f32>(0.0));
    let uv = p * P.size.zw;
    // natural vignetting of a rectilinear lens, the cos^4 law (Ray, Applied
    // Photographic Optics, 3rd ed., 10.1): E = E0 / (1 + (r / f)^2)^2.
    if (P.vig.x > 0.0) {
        let s = (uv - 0.5) * P.vig.yz;
        let t = 1.0 + dot(s, s);
        c = c * pow(1.0 / (t * t), P.vig.x);
    }
    // film grain: density noise, so multiplicative in exposure (see photo.rs `grain_sigma_stops`).
    if (P.vig.w > 0.0) {
        let g = gauss(p, P.frame.y);
        let mono = P.grade.w > 0.5;
        var n = vec3<f32>(g);
        if (!mono) {
            let k = P.frame.z;
            n = g * (1.0 - k) + k * vec3<f32>(gauss(p, P.frame.y + 101.0), gauss(p, P.frame.y + 211.0), gauss(p, P.frame.y + 307.0));
        }
        c = c * exp2(n * P.vig.w);
    }
    // letterbox (black bars outside the chosen image aspect)
    if (P.frame.x > 0.0) {
        let a = P.size.x / P.size.y;
        if (P.frame.x > a) {
            let h = a / P.frame.x;
            if (abs(uv.y - 0.5) > 0.5 * h) {
                c = vec3<f32>(0.0);
            }
        } else {
            let w = P.frame.x / a;
            if (abs(uv.x - 0.5) > 0.5 * w) {
                c = vec3<f32>(0.0);
            }
        }
    }
    return c;
}

fn far_up(p: vec2<i32>, k: f32) -> vec3<f32> {
    // the four half texels round the pixel, weighted bilinearly and by how
    // alike their blur is to this pixel's (no in-focus colour across an edge).
    let h = (vec2<f32>(p) + 0.5) * 0.5 - 0.5;
    let b = floor(h);
    let f = h - b;
    let hs = vec2<i32>(P.half.xy) - 1;
    var c = vec3<f32>(0.0);
    var w = 0.0;
    let tol = 0.5 + 0.25 * abs(k);
    for (var i = 0; i < 4; i++) {
        let o = vec2<i32>(i & 1, i >> 1);
        let t = clamp(vec2<i32>(b) + o, vec2<i32>(0), hs);
        let bw = select(1.0 - f.x, f.x, o.x == 1) * select(1.0 - f.y, f.y, o.y == 1);
        let kt = textureLoad(prep_far, t, 0).a;
        let lw = bw * exp(-abs(kt - k) / tol) + 1e-5 * bw;
        c += textureLoad(far_layer, t, 0).rgb * lw;
        w += lw;
    }
    return c / max(w, 1e-8);
}

fn near_up(p: vec2<i32>) -> vec4<f32> {
    let h = (vec2<f32>(p) + 0.5) * 0.5 - 0.5;
    let b = floor(h);
    let f = h - b;
    let hs = vec2<i32>(P.half.xy) - 1;
    var c = vec4<f32>(0.0);
    for (var i = 0; i < 4; i++) {
        let o = vec2<i32>(i & 1, i >> 1);
        let t = clamp(vec2<i32>(b) + o, vec2<i32>(0), hs);
        let bw = select(1.0 - f.x, f.x, o.x == 1) * select(1.0 - f.y, f.y, o.y == 1);
        let s = textureLoad(near_layer, t, 0);
        c += vec4<f32>(s.rgb * s.a, s.a) * bw;
    }
    return c;
}

@compute @workgroup_size(8, 8)
fn final_pass(@builtin(global_invocation_id) id: vec3<u32>) {
    if (f32(id.x) >= P.size.x || f32(id.y) >= P.size.y) {
        return;
    }
    let p = vec2<i32>(id.xy);
    let pf = vec2<f32>(p) + 0.5;
    var c = max(textureLoad(src, p, 0).rgb, vec3<f32>(0.0));
    let flags = u32(P.lens2.w);
    if ((flags & 1u) != 0u) {
        let k = coc(view_depth(textureLoad(depth, p, 0)));
        let t = saturate((abs(k) - SHARP) / SHARP);
        var base = c;
        if (t > 0.0) {
            base = mix(c, far_up(p, k), t);
        }
        let nl = near_up(p);
        let a = saturate(nl.a);
        let blurred = base * (1.0 - a) + nl.rgb * select(0.0, a / max(nl.a, 1e-6), nl.a > 0.0);
        // accumulated where blurred while the view is still (the gather's
        // pattern turns each frame); in focus the frame passes through.
        let amount = max(t, a);
        let wa = mix(1.0, P.lens2.z, amount);
        let prev = textureLoad(history, p, 0).rgb;
        c = mix(prev, blurred, wa);
        textureStore(history_out, p, vec4<f32>(c, 1.0));
    }
    // the bloom of the frame (post adds none to a photo-mode frame): before
    // the exposure, so the two scale together and the bars stay black.
    if ((flags & 2u) != 0u) {
        c += max(textureSampleLevel(bloom, lin, pf * P.size.zw, 0.0).rgb, vec3<f32>(0.0));
    }
    textureStore(photo_out, p, vec4<f32>(finish(c, pf), 1.0));
}
