// Photo mode: a thin-lens depth of field and photo mode's finish (exposure
// compensation, white balance, grade, natural vignetting, film grain,
// letterbox), between the temporal resolve and post. See photo_mode.rs.
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
    // largest radius (half px), this frame's pattern turn (rad), accumulation weight (1: none), flags (1 dof, 2 bloom here, 4 peaking)
    lens2: vec4<f32>,
    // exposure multiplier, saturation, contrast (log slope about mid-grey), fade
    grade: vec4<f32>,
    // the white-balance matrix, rows (linear Rec. 709)
    wb0: vec4<f32>,
    wb1: vec4<f32>,
    wb2: vec4<f32>,
    // monochrome: the contrast filter's channel weights, on
    mono: vec4<f32>,
    // natural vignetting share, sensor width / f, sensor height / f, post's exposure / 0.6 (scene to the tone curve's input)
    vig: vec4<f32>,
    // letterbox image aspect (0: off), grain seed, grain colour share, radial jitter (0..1)
    frame: vec4<f32>,
    // iris: blades (0: a circle), roundness (1: round), polygon turn (rad), polygon circumradius of the circle's area
    bokeh: vec4<f32>,
    // cat's eye: barrel circle offset at the corner, its radius (disc radii); disc rim; letterbox offset (-1..1)
    cat: vec4<f32>,
    // grain octaves: sigma (stops) at this pixel pitch, and cell (px)
    grain_a: vec4<f32>,
    grain_c: vec4<f32>,
    // half the diagonal (px), the 0.03 mm circle's radius (half px), the bloom prefilter's ceiling, -
    extra: vec4<f32>,
    // the frame's bloom as post would add it (photo_mode.rs `PostBloom`): its share, the
    // cinematic look's share of the bright part (0: added), the prefilter's threshold and knee
    bloom: vec4<f32>,
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

// the share of a pixel that is blurred: a smooth ramp over 0.5-1.5 full px of blur radius.
fn blur_share(k: f32) -> f32 {
    return smoothstep(0.25, 0.75, abs(k));
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// a full-resolution position's place in the field: |f| = 1 at the corners.
fn field(px: vec2<f32>) -> vec2<f32> {
    return (px - 0.5 * P.size.xy) / P.extra.x;
}

// ---------------------------------------------------------------------------
// 1. Prep: half resolution, two depth-aware downsamples. The far one keeps
//    the farthest of each 2 x 2 (and the colours near its blur), the near one
//    the nearest, so an in-focus edge never smears into either layer. A pixel
//    beside a depth edge (its anti-aliased colour is both sides') counts for little.
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
    let o = vec2<i32>(id.xy) * 2 - 1;
    // the blur of the 4 x 4 block round the 2 x 2.
    var kk: array<f32, 16>;
    for (var j = 0; j < 16; j++) {
        let q = clamp(o + vec2<i32>(j & 3, j >> 2), vec2<i32>(0), full);
        kk[j] = coc(view_depth(textureLoad(depth, q, 0)));
    }
    var cs: array<vec3<f32>, 4>;
    var ks: array<f32, 4>;
    var pure: array<f32, 4>;
    var kf = -1e9;
    var kn = 1e9;
    for (var i = 0; i < 4; i++) {
        let lx = 1 + (i & 1);
        let ly = 1 + (i >> 1);
        let k = kk[ly * 4 + lx];
        let p = min(vec2<i32>(id.xy) * 2 + vec2<i32>(i & 1, i >> 1), full);
        cs[i] = min(max(textureLoad(src, p, 0).rgb, vec3<f32>(0.0)), vec3<f32>(65000.0));
        ks[i] = k;
        let tol = max(1.0, 0.25 * abs(k));
        let dm = max(max(abs(kk[ly * 4 + lx - 1] - k), abs(kk[ly * 4 + lx + 1] - k)), max(abs(kk[ly * 4 + lx - 4] - k), abs(kk[ly * 4 + lx + 4] - k)));
        pure[i] = select(0.02, 1.0, dm <= tol);
        kf = max(kf, k);
        kn = min(kn, k);
    }
    var cf = vec3<f32>(0.0);
    var wf = 0.0;
    var cn = vec3<f32>(0.0);
    var wn = 0.0;
    for (var i = 0; i < 4; i++) {
        let a = (1.0 - saturate(abs(ks[i] - kf) / max(0.5, 0.25 * abs(kf)))) * pure[i];
        let b = (1.0 - saturate(abs(ks[i] - kn) / max(0.5, 0.25 * abs(kn)))) * pure[i];
        cf += cs[i] * a;
        wf += a;
        cn += cs[i] * b;
        wn += b;
    }
    textureStore(prep_far_out, vec2<i32>(id.xy), vec4<f32>(cf / max(wf, 1e-6), kf));
    textureStore(prep_near_out, vec2<i32>(id.xy), vec4<f32>(cn / max(wn, 1e-6), kn));
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
//    tap standing for the same area (pi d^2 / 4 for ring spacing d), each
//    read from one texel (point-sampled: no blur interpolated across an edge).
//    A tap's disc is the iris's image (`shape`): a circle or the blades'
//    polygon, clipped by the barrel toward the corners (the cat's eye).
//    Far layer: a tap adds where its own disc reaches the centre and the
//    centre's disc reaches it (min of the two radii): a sharper surface in
//    front never spreads onto a blurred background (no halo), a blurred
//    background never leaks over a sharper surface in front of it. Each tap
//    spreads its energy over its disc (weight 1 / r^2), a little brighter at
//    the rim behind the focus and a little softer in front.
//    Near layer: in-front taps spread over whatever is behind them, with
//    their coverage as alpha (the share of the centre their discs cover).
//    In front of the focus plane the far layer gathers the background
//    around the centre instead: what the blurred foreground's edge lets through.
//    Both layers are accumulated while the view is still (the pattern turns).
// ---------------------------------------------------------------------------

@group(0) @binding(10) var far_out: texture_storage_2d<rgba32float, write>;
@group(0) @binding(11) var near_out: texture_storage_2d<rgba32float, write>;
@group(0) @binding(18) var far_hist: texture_2d<f32>;
@group(0) @binding(19) var near_hist: texture_2d<f32>;

// the boundary of a unit disc along direction `u` (angle `a`): the iris's
// polygon (rounded toward a circle by `bokeh.y`) and the barrel's circle of
// radius cat.y centred at `c` (both in disc radii).
fn shape(a: f32, u: vec2<f32>, c: vec2<f32>) -> f32 {
    var r = 1.0;
    let n = P.bokeh.x;
    if (n >= 3.0) {
        let seg = 2.0 * PI / n;
        let b = a - P.bokeh.z;
        let m = b - seg * floor(b / seg) - 0.5 * seg;
        r = mix(P.bokeh.w * cos(0.5 * seg) / cos(m), 1.0, P.bokeh.y);
    }
    if (P.cat.x > 0.0) {
        let uc = dot(u, c);
        let t = uc + sqrt(max(uc * uc - dot(c, c) + P.cat.y * P.cat.y, 0.0));
        r = min(r, max(t, 0.0));
    }
    return r;
}

@compute @workgroup_size(8, 8)
fn gather(@builtin(global_invocation_id) id: vec3<u32>) {
    let hs = P.half.xy;
    if (f32(id.x) >= hs.x || f32(id.y) >= hs.y) {
        return;
    }
    let q = vec2<i32>(id.xy);
    let hmax = vec2<i32>(hs) - 1;
    let wgt = P.lens2.z;
    let cf = textureLoad(prep_far, q, 0);
    let tile = textureLoad(tiles_in, q / 8, 0).x;
    let fill = cf.a < -SHARP;
    let r_far = select(max(cf.a, 0.0), min(-cf.a, P.lens2.x), fill);
    let r_near = select(0.0, tile, tile >= SHARP);
    let r_all = max(r_far, r_near);
    var far_rgb = cf.rgb;
    var near_pm = vec4<f32>(0.0);
    if (r_all >= SHARP) {
        let k = clamp(i32(ceil(r_all / SPACING)), 1, K_MAX);
        let d = r_all / f32(k);
        let centre = vec2<f32>(q) + 0.5;
        let shaped = P.bokeh.x >= 3.0 || P.cat.x > 0.0;
        // the barrel's circle for this place in the field (toward the frame centre).
        let cc = -P.cat.x * field(centre * 2.0);
        var far_c = vec3<f32>(0.0);
        var far_w = 0.0;
        var near_c = vec3<f32>(0.0);
        var near_w = 0.0;
        let want_near = r_near >= SHARP;
        // a disc's edge: one half px wide on a single frame (no ring shows
        // while the view moves), a quarter of that once enough jittered frames
        // are accumulated: a crisp-edged disc, as a lens draws it.
        let edge_k = 1.0 / clamp(wgt * 4.0, 0.25, 1.0);
        for (var ring = 0; ring <= k; ring++) {
            let n = max(1, 8 * ring);
            // the rings' radii and turns move from frame to frame while the view is still (accumulated).
            let r = (f32(ring) + select(0.0, (P.frame.w - 0.5) * 0.9, ring > 0)) * d;
            let turn = P.lens2.y + f32(ring) * 0.61803 * PI;
            for (var t = 0; t < n; t++) {
                let a = turn + f32(t) * (2.0 * PI / f32(n));
                let dir = vec2<f32>(cos(a), sin(a));
                let tq = clamp(vec2<i32>(floor(centre + dir * r)), vec2<i32>(0), hmax);
                // behind the focus the tap's disc reaches the centre along -dir; in
                // front the pupil's image is the other way up (+dir).
                var sh_f = 1.0;
                var sh_n = 1.0;
                if (shaped && ring > 0) {
                    sh_f = shape(a + PI, -dir, cc);
                    sh_n = shape(a, dir, cc);
                }
                let sf = textureLoad(prep_far, tq, 0);
                if (sf.a >= -SHARP) {
                    if (fill) {
                        if (r <= r_far * sh_f) {
                            far_c += sf.rgb;
                            far_w += 1.0;
                        }
                    } else {
                        let rs = max(sf.a, 0.0);
                        let cov = saturate((min(rs, r_far) * sh_f - r) * edge_k + 0.5);
                        if (cov > 0.0) {
                            let rho = r / max(rs * sh_f, 0.5);
                            let prof = 1.0 + P.cat.z * (smoothstep(0.6, 1.0, rho) - 0.352);
                            let w = cov * prof / max(rs * rs, 0.25);
                            far_c += sf.rgb * w;
                            far_w += w;
                        }
                    }
                }
                if (want_near) {
                    let sn = textureLoad(prep_near, tq, 0);
                    let rn = -sn.a;
                    if (rn >= SHARP) {
                        let reach = rn * sh_n;
                        let cov = saturate((reach - r) * edge_k + 0.5);
                        if (cov > 0.0) {
                            let rho = r / max(reach, 0.5);
                            let prof = 1.0 - P.cat.z * (smoothstep(0.6, 1.0, rho) - 0.352);
                            let w = cov * prof / (rn * rn);
                            near_c += sn.rgb * w;
                            near_w += w;
                        }
                    }
                }
            }
        }
        far_rgb = select(cf.rgb, far_c / max(far_w, 1e-8), far_w > 0.0);
        // coverage: each tap's area (pi d^2 / 4) over its disc's (pi r^2), summed.
        let alpha = saturate(near_w * d * d * 0.25);
        let near_rgb = select(vec3<f32>(0.0), near_c / max(near_w, 1e-8), near_w > 0.0);
        near_pm = vec4<f32>(near_rgb * alpha, alpha);
    }
    // the running mean while the view is still (weight 1: this frame alone).
    var fo = vec4<f32>(far_rgb, 1.0);
    var no = near_pm;
    if (wgt < 0.999) {
        fo = mix(textureLoad(far_hist, q, 0), fo, wgt);
        no = mix(textureLoad(near_hist, q, 0), no, wgt);
    }
    textureStore(far_out, q, fo);
    textureStore(near_out, q, no);
}

// ---------------------------------------------------------------------------
// 5. Finish (full resolution): the sharp (temporally resolved, so
//    anti-aliased) frame, the far layer (upsampled by blur likeness) by each
//    pixel's blurred share on a smooth ramp, and the near layer over it; then
//    the photo finish. The blurred share is accumulated from the jittered
//    depth while the view is still (one frame: a 3 x 3 tent of it at an
//    edge). At a focus-plane silhouette over a blurred background the
//    resolved pixel is matted: its anti-aliased coverage is taken from its
//    colour between the in-focus and the background neighbours, and only the
//    sharp background's share of it is swapped for the blurred background,
//    so the edge keeps its anti-aliasing and no sharp background or blurred
//    light shows on the in-focus side.
// ---------------------------------------------------------------------------

@group(0) @binding(9) var lin: sampler;
@group(0) @binding(12) var far_layer: texture_2d<f32>;
@group(0) @binding(13) var near_layer: texture_2d<f32>;
@group(0) @binding(14) var cov_in: texture_2d<f32>;
@group(0) @binding(15) var cov_out: texture_storage_2d<r32float, write>;
@group(0) @binding(16) var photo_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(17) var bloom: texture_2d<f32>;

// PCG3D (Jarzynski and Olano, "Hash Functions for GPU Rendering", JCGT 9(3), 2020).
fn pcg3d(v0: vec3<u32>) -> vec3<u32> {
    var v = v0 * 1664525u + 1013904223u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v ^= v >> vec3<u32>(16u);
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    return v;
}

// uniform on [0, 1) for a lattice point and a stream.
fn h1(i: vec2<i32>, s: u32) -> f32 {
    let v = pcg3d(vec3<u32>(bitcast<u32>(i.x), bitcast<u32>(i.y), s));
    return f32(v.x >> 8u) * (1.0 / 16777216.0);
}

// value noise of unit variance: lattice values of unit variance, smoothstep
// weights (whose two-dimensional sum of squares averages 0.7429^2).
fn vnoise(x: vec2<f32>, s: u32) -> f32 {
    let i = vec2<i32>(floor(x));
    let f = fract(x);
    let u = f * f * (3.0 - 2.0 * f);
    let a = h1(i, s);
    let b = h1(i + vec2<i32>(1, 0), s);
    let c = h1(i + vec2<i32>(0, 1), s);
    let d = h1(i + vec2<i32>(1, 1), s);
    return (mix(mix(a, b, u.x), mix(c, d, u.x), u.y) - 0.5) * 3.4641016 / 0.7429;
}

// the film's grain at a pixel (stops of log exposure): octaves of clumps in
// the film plane, those finer than a pixel one value a pixel (their sigma
// already averaged over it on the CPU, photo.rs `grain_octaves`).
fn grain_field(p: vec2<f32>, seed: u32) -> f32 {
    var n = 0.0;
    for (var o = 0; o < 4; o++) {
        let a = P.grain_a[o];
        if (a <= 0.0) {
            continue;
        }
        let cell = P.grain_c[o];
        let s = seed * 4u + u32(o) * 7919u + 17u;
        if (cell < 1.0) {
            n += a * (h1(vec2<i32>(floor(p)), s) - 0.5) * 3.4641016;
        } else {
            n += a * vnoise(p / cell + vec2<f32>(f32(o) * 37.1, f32(o) * 11.7), s);
        }
    }
    return n;
}

fn finish(c0: vec3<f32>, p: vec2<f32>) -> vec3<f32> {
    // scene values to the tone curve's input, where mid-grey is 0.18.
    let norm = P.vig.w;
    var c = c0 * P.grade.x;
    c = vec3<f32>(dot(P.wb0.xyz, c), dot(P.wb1.xyz, c), dot(P.wb2.xyz, c));
    c = max(c, vec3<f32>(0.0));
    // monochrome: the panchromatic film through its contrast filter (photo.rs `MonoFilter`).
    if (P.mono.w > 0.5) {
        c = vec3<f32>(max(dot(c, P.mono.xyz), 0.0));
    }
    let grey = 0.18 / norm;
    let y = luma(c);
    // contrast: a slope in log exposure about mid-grey (a film's gamma), the hue kept.
    if (P.grade.z != 1.0 && y > 0.0) {
        let y2 = grey * pow(y / grey, P.grade.z);
        c = c * (y2 / y);
    }
    if (P.grade.y != 1.0) {
        let y3 = luma(c);
        c = max(mix(vec3<f32>(y3), c, P.grade.y), vec3<f32>(0.0));
    }
    // fade: a faded print's veil (blacks lifted to about 10 % of the display range at full) and a softer top.
    if (P.grade.w > 0.0) {
        let f = P.grade.w;
        c = c * (1.0 - 0.15 * f) + vec3<f32>(0.035 * f / norm);
    }
    let uv = p * P.size.zw;
    // natural vignetting of a rectilinear lens, the cos^4 law (Ray, Applied
    // Photographic Optics, 3rd ed., 10.1): E = E0 / (1 + (r / f)^2)^2.
    if (P.vig.x > 0.0) {
        let s = (uv - 0.5) * P.vig.yz;
        let t = 1.0 + dot(s, s);
        c = c * pow(1.0 / (t * t), P.vig.x);
    }
    // film grain: density noise, so multiplicative in exposure; graininess is
    // most visible in the mid-tones (Kodak publication F-20, "Understanding
    // Graininess and Granularity"): a bell over +-2 stops round mid-grey.
    if (any(P.grain_a > vec4<f32>(0.0))) {
        let st = log2(max(luma(c) * norm, 1e-5) / 0.18) / 2.0;
        let m = exp(-0.5 * st * st);
        let seed = u32(P.frame.y);
        let gl = grain_field(p, seed);
        var n = vec3<f32>(gl);
        if (P.mono.w < 0.5) {
            // a colour film's three dye layers: part shared, part each its own.
            let k = P.frame.z;
            let s = 1.0 / sqrt((1.0 - k) * (1.0 - k) + k * k);
            n = (vec3<f32>(gl) * (1.0 - k) + k * vec3<f32>(grain_field(p, seed + 101u), grain_field(p, seed + 211u), grain_field(p, seed + 307u))) * s;
        }
        c = c * exp2(n * m);
    }
    // letterbox: black bars outside the chosen image aspect, the opening moved by the offset.
    if (P.frame.x > 0.0) {
        let a = P.size.x / P.size.y;
        let off = P.cat.w;
        if (P.frame.x > a) {
            let h = a / P.frame.x;
            let cy = 0.5 + off * 0.5 * (1.0 - h);
            if (abs(uv.y - cy) > 0.5 * h) {
                c = vec3<f32>(0.0);
            }
        } else {
            let w = P.frame.x / a;
            let cx = 0.5 + off * 0.5 * (1.0 - w);
            if (abs(uv.x - cx) > 0.5 * w) {
                c = vec3<f32>(0.0);
            }
        }
    }
    return c;
}

// A pixel's bright part as the bloom's prefilter takes it (look_cine.wgsl
// `cn_bright`: sky_bloom.wgsl's ceiling, cap and threshold).
fn bloom_bright(c: vec3<f32>) -> vec3<f32> {
    let th = P.bloom.z;
    let cap = th * P.extra.z;
    let m = max(c.r, max(c.g, c.b));
    var b = c * (cap / max(m, cap));
    let lw = vec3<f32>(0.2126, 0.7152, 0.0722);
    b = b * min(1.0, 16.0 * th / max(dot(b, lw), 1e-6));
    return b * smoothstep(th, th + P.bloom.w * th, dot(b, lw));
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

// the near layer (premultiplied), bilinear.
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
        c += textureLoad(near_layer, t, 0) * bw;
    }
    return c;
}

fn src_at(p: vec2<i32>) -> vec3<f32> {
    return max(textureLoad(src, clamp(p, vec2<i32>(0), vec2<i32>(P.size.xy) - 1), 0).rgb, vec3<f32>(0.0));
}

@compute @workgroup_size(8, 8)
fn final_pass(@builtin(global_invocation_id) id: vec3<u32>) {
    if (f32(id.x) >= P.size.x || f32(id.y) >= P.size.y) {
        return;
    }
    let p = vec2<i32>(id.xy);
    let pf = vec2<f32>(p) + 0.5;
    let full = vec2<i32>(P.size.xy) - 1;
    var c = src_at(p);
    let flags = u32(P.lens2.w);
    let k = coc(view_depth(textureLoad(depth, p, 0)));
    if ((flags & 1u) != 0u) {
        let wgt = P.lens2.z;
        let first = wgt >= 0.999;
        let t = blur_share(k);
        // the 3 x 3 round the pixel: its blurred shares now and as held, the
        // fully in-focus neighbours' colour (F) and the fully blurred ones'
        // sharp colour (Bs, the background as the resolved frame has it).
        var tmin = t;
        var tmax = t;
        var asp = 0.0;
        var fsum = vec3<f32>(0.0);
        var fw = 0.0;
        var bsum = vec3<f32>(0.0);
        var bw = 0.0;
        var kb = 0.0;
        var nsum = vec3<f32>(0.0);
        var nw = 0.0;
        for (var j = 0; j < 9; j++) {
            let o = vec2<i32>(j % 3 - 1, j / 3 - 1);
            let qq = clamp(p + o, vec2<i32>(0), full);
            let kq = select(coc(view_depth(textureLoad(depth, qq, 0))), k, j == 4);
            let tq = blur_share(kq);
            // a 3 x 3 tent (the centre 4, the sides 2, the corners 1).
            let tw = select(select(1.0, 2.0, o.x == 0 || o.y == 0), 4.0, j == 4);
            tmin = min(tmin, tq);
            tmax = max(tmax, tq);
            asp += tw * tq;
            if (tq < 0.01) {
                fsum += src_at(qq) * tw;
                fw += tw;
            }
            if (tq > 0.99 && kq > 0.0) {
                bsum += src_at(qq) * tw;
                bw += tw;
                kb = max(kb, kq);
            }
            if (tq > 0.99 && kq < 0.0) {
                nsum += src_at(qq) * tw;
                nw += tw;
            }
        }
        // a blurred surface in front reaches two pixels further: the resolve's
        // anti-aliasing spreads a sharp edge over about two pixels.
        if (t < 0.01) {
            for (var j = 0; j < 4; j++) {
                let o = select(vec2<i32>(0, 2 * (j & 1) * 2 - 2), vec2<i32>(2 * (j & 1) * 2 - 2, 0), j < 2);
                let qq = clamp(p + o, vec2<i32>(0), full);
                let kq = coc(view_depth(textureLoad(depth, qq, 0)));
                if (blur_share(kq) > 0.99 && kq < 0.0) {
                    nsum += src_at(qq);
                    nw += 1.0;
                }
            }
        }
        asp = asp / 16.0;
        let edge = tmax - tmin > 0.5;
        // the blurred share, accumulated from the jittered depth while the view is still (one frame: a 3 x 3 tent at an edge).
        let prev = textureLoad(cov_in, p, 0).r;
        let a = select(mix(prev, t, wgt), select(t, asp, edge), first);
        textureStore(cov_out, p, vec4<f32>(a, 0.0, 0.0, 0.0));
        var base = c;
        if (fw > 0.0 && bw > 0.0) {
            // a silhouette in the focus plane over a blurred background. The
            // resolved pixel is c = A F + (1 - A) Bs (A: the in-focus surface's
            // anti-aliased coverage, F its colour, Bs the sharp background's);
            // the lens sees A F + (1 - A) Bb, with Bb the blurred background
            // there (the far layer at the background's blur). A is taken from
            // the colour where F and Bs differ enough (the projection of c on
            // the line from Bs to F), else from the depth (its accumulated
            // share, a tent of it on one frame); the in-focus colour is the
            // pixel's own while it is mostly the surface's, its in-focus
            // neighbours' at a thin rim, and a pixel wholly background is Bb.
            let f = fsum / fw;
            let bs = bsum / bw;
            let bb = far_up(p, kb);
            let d = f - bs;
            let den = dot(d, d);
            let lum = max(max(luma(f), luma(bs)), 1e-6);
            let conf = smoothstep(0.01, 0.09, den / (lum * lum));
            let ac = saturate(dot(c - bs, d) / max(den, 1e-12));
            let cover = mix(1.0 - a, ac, conf);
            let own = max(c + (1.0 - cover) * (bb - bs), vec3<f32>(0.0));
            let rim = f * cover + bb * (1.0 - cover);
            base = mix(rim, own, smoothstep(0.3, 0.7, cover));
        } else if (t < 0.01 && fw > 4.5 && nw > 0.0) {
            // an in-focus pixel at the sharp edge of a blurred surface in
            // front of it: the resolved pixel holds that edge (its colour and
            // the resolve's dark or light rim), which the near layer draws
            // blurred over it. Under the near layer's soft edge it is its
            // in-focus neighbours' colour (the centre left out), so no outline
            // of the sharp edge shows inside the blurred one.
            base = (fsum - 4.0 * c) / (fw - 4.0);
        } else if (t > 0.0) {
            // the smooth ramp out of focus (the blurred share over 0.5-1.5 px of blur radius).
            base = mix(c, far_up(p, k), t);
        }
        let nl = near_up(p);
        let na = saturate(nl.a);
        c = base * (1.0 - na) + nl.rgb * select(0.0, na / nl.a, nl.a > 1e-6);
    }
    // the bloom of the frame (post adds none to a photo-mode frame): before
    // the exposure, so the two scale together and the bars stay black. At
    // the frame's share; under the cinematic look energy kept, as its pass
    // keeps it (look_cine.wgsl: the pixel gives up the share of its bright part).
    if ((flags & 2u) != 0u) {
        let b = max(textureSampleLevel(bloom, lin, pf * P.size.zw, 0.0).rgb, vec3<f32>(0.0));
        if (P.bloom.y > 0.0) {
            c = max(c - P.bloom.x * P.bloom.y * bloom_bright(c), vec3<f32>(0.0));
        }
        c += P.bloom.x * b;
    }
    var o = finish(c, pf);
    // focus peaking (the window only): an edge of the sharp frame inside the
    // 0.03 mm circle, marked in a red the tone curve keeps.
    if ((flags & 4u) != 0u && abs(k) < P.extra.y) {
        let l = luma(src_at(p));
        let gx = luma(src_at(p + vec2<i32>(1, 0))) - luma(src_at(p - vec2<i32>(1, 0)));
        let gy = luma(src_at(p + vec2<i32>(0, 1))) - luma(src_at(p - vec2<i32>(0, 1)));
        let g = (abs(gx) + abs(gy)) / max(l, 0.01 / P.vig.w);
        if (g > 0.45) {
            o = mix(o, vec3<f32>(1.0, 0.14, 0.04) * (1.2 / P.vig.w), 0.85);
        }
    }
    textureStore(photo_out, p, vec4<f32>(o, 1.0));
}
