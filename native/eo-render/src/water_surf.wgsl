// the breaking surf on the beaches, src/render/materials/ocean-surf.js
// (tidewater's ShoreWaves: `shoreBar`, `shoreWobble`, `shoreWaveAmp`,
// `shoreBreakParams`, `shoreFaceFill`, `shoreWhitewater`, `shoreProfile`, the
// closed-form travel time over the 1:55 profile, the trough blend, `surfAt`),
// with the parameters ocean-open-sea.js gives it (variation 0.2, rip depth
// 0.2, rip edge -1.0; the rest `createSurfParams` defaults). Transcribed from
// the TSL; the simulated surf zone (ShoreSim, Breakers, Spray) is not ported.

const SF_TAU: f32 = 6.283185307;
const SF_G: f32 = 9.81;
const SF_TOE: f32 = 0.12;
const SF_SLOPE: f32 = 0.018181818; // 1 / 55
const SF_MAX_DEPTH: f32 = 12.0;
const SF_PERIOD: f32 = 11.0;
const SF_AMPLITUDE: f32 = 0.55;
const SF_VARIATION: f32 = 0.2;
const SF_GAMMA: f32 = 0.78;
const SF_BREAK_SPAN: f32 = 0.13;
const SF_CURL: f32 = 1.0;
const SF_RIP_DEPTH: f32 = 0.2;
const SF_RIP_EDGE: f32 = -1.0;
const SF_TROUGH_BLEND_FROM: f32 = 0.25;

fn sf_sat(x: f32) -> f32 {
    return clamp(x, 0.0, 1.0);
}

fn sf_hash1(x: f32) -> f32 {
    return fract(sin(x * 127.1 + 311.7) * 43758.5453);
}

// Sand bar with rip channels: (k, rip).
fn sf_bar(along: f32) -> vec2<f32> {
    let w = sin(along * 0.021 + 1.9) * 1.4 + sin(along * 0.009 + 0.3) * 0.9;
    let r = sin(along * 0.059 + w);
    let width = sin(along * 0.017 + 4.1) * 0.08 + 0.8;
    let rip = wg_smooth(width + SF_RIP_EDGE, 0.985, r);
    let bar = (sin(along * 0.031 + 7.3) * 0.6 + sin(along * 0.083 + 1.1) * 0.4) * 0.14 + 0.9;
    return vec2<f32>(bar * (1.0 - rip * SF_RIP_DEPTH), rip);
}

fn sf_wobble(along: f32) -> f32 {
    return sin(along * 0.029 + 0.7) * 0.07 + sin(along * 0.083 + 2.1) * 0.035 + sin(along * 0.19 + 0.4) * 0.022 + sin(along * 0.37 + 2.6) * 0.011 + sf_bar(along).y * 0.045;
}

fn sf_wave_amp(m: f32, along: f32) -> f32 {
    let wave_set = abs(sin(m * (3.14159265 / 7.0))) * 0.6 + 0.55;
    let rnd = (sf_hash1(m) - 0.5) * 2.0;
    let warp = sin(along * 0.016 + m * 0.9) * 1.6;
    let a1 = sin(along * 0.062 + m * 1.7 + warp);
    let a2 = sin(along * 0.13 + m * 4.1 + 1.3 - warp * 0.7);
    let a3 = sin(along * 0.29 + m * 2.3 + warp * 0.5) * 0.6 + sin(along * 0.47 + m * 5.9 + 0.8) * 0.4;
    let along_v = a1 * 0.6 + a2 * 0.4 + a3 * 0.28;
    return max(SF_AMPLITUDE * wave_set * (1.0 + rnd * SF_VARIATION * 0.5 + along_v * SF_VARIATION * 0.7) * sf_bar(along).x, 0.02);
}

struct SfBreak {
    b: f32,
    ash: f32,
    p: f32,
    mean_p: f32,
    crest_peak: f32,
    w_bore: f32,
    xi: f32,
    hb: f32,
    yc_b: f32,
    yt_b: f32,
    xc: f32,
    wt: f32,
};

fn sf_break(a: f32, d: f32) -> SfBreak {
    var o: SfBreak;
    let db = pow(a * 3.556 / SF_GAMMA, 0.8);
    o.b = (db - d) / (db * SF_BREAK_SPAN);
    let shoal = pow(10.0 / clamp(d, 0.35, 10.0), 0.25);
    o.ash = a * shoal;
    o.p = mix(1.0, 3.0, wg_smooth(-2.5, 0.0, o.b));
    o.mean_p = 1.0 / sqrt((o.p + 0.25) * 3.14159265);
    o.crest_peak = wg_smooth(-1.0, 0.3, o.b) * 0.22 + 1.0;
    let yc = o.ash * 2.0 * (1.0 - o.mean_p) * o.crest_peak;
    let yt = o.ash * -2.0 * o.mean_p * o.crest_peak;
    let h = yc - yt;
    o.w_bore = wg_smooth(0.85, 1.35, o.b);
    o.xi = h * 0.8;
    o.hb = min(h * 0.6, max(d, 0.0) * 0.75) * (wg_smooth(0.0, 0.3, d) * 0.7 + 0.3);
    o.yc_b = mix(yc, yt * 0.6 + o.hb, o.w_bore);
    o.yt_b = mix(yt, yt * 0.6, o.w_bore);
    o.xc = mix(0.0, o.xi - o.hb * 0.55, o.w_bore) * wg_smooth(0.03, 0.3, o.hb);
    o.wt = mix(h * mix(0.25, 0.55, wg_smooth(0.1, 0.9, o.b)), o.hb * 0.6 + 0.08, o.w_bore);
    return o;
}

fn sf_face_fill(b: f32, s: f32) -> f32 {
    let k = (1.0 - s) * 0.45;
    return wg_smooth(k + 0.9, k + 1.08, b);
}

fn sf_whitewater(xi: f32, lam: f32, bk: SfBreak) -> f32 {
    let landed = wg_smooth(0.86, 0.99, bk.b);
    let spread = sf_sat((bk.b - 0.9) / 0.45);
    let reach = bk.xi * 0.25 + spread * (bk.xi * 0.9 + 1.0);
    let impact = landed * wg_smooth(reach, reach * 0.6, abs(xi - bk.xi)) * (1.0 - wg_smooth(1.4, 1.9, bk.b));
    let toe = bk.xc + bk.wt;
    let face_fill = sf_face_fill(bk.b, sf_sat((xi - bk.xc) / max(bk.wt, 0.05)));
    let ahead = exp((xi - toe) * -3.0) * bk.w_bore;
    let behind = exp((bk.xc - xi) / lam * -16.0) * bk.w_bore;
    var roller = max(face_fill, bk.w_bore);
    if (xi > toe) {
        roller = ahead;
    } else if (xi < bk.xc) {
        roller = behind;
    }
    return sf_sat(max(impact, roller));
}

// Cross-section at local phase u: (x, y, foam, face).
fn sf_profile(u: f32, lam: f32, bk: SfBreak) -> vec4<f32> {
    let skew = wg_smooth(-3.0, 0.0, bk.b) * 0.55;
    let phi = u - skew * (1.0 - cos(u * SF_TAU)) / SF_TAU;
    let c = max((cos(phi * SF_TAU) + 1.0) * 0.5, 0.0);
    let y_pre = bk.ash * 2.0 * (pow(c, bk.p) - bk.mean_p) * bk.crest_peak;
    let q = wg_smooth(-3.0, 0.0, bk.b) * 0.25 + 0.1;
    let x_pre = sin(u * SF_TAU) * bk.ash * q;
    let uf = 0.09;
    let in_face = u > -uf;
    let th = clamp(-u / uf, 0.0, 1.0) * 1.5707963;
    let fx = mix(1.0 - cos(th), sin(th), bk.w_bore);
    let fy = mix(1.0 - sin(th), cos(th), bk.w_bore);
    let s_tr = clamp((-u - uf) / (0.5 - uf), 0.0, 1.0);
    let ul = u * lam;
    var x_front = mix(bk.xc + bk.wt, lam * 0.5, s_tr) + ul;
    var y_front = bk.yt_b;
    if (in_face) {
        x_front = bk.xc + bk.wt * fx + ul;
        y_front = bk.yt_b + (bk.yc_b - bk.yt_b) * fy;
    }
    let cb = pow(max((cos(u * SF_TAU * 0.85) + 1.0) * 0.5, 0.0), bk.p);
    let y_back = bk.yt_b + (bk.yc_b - bk.yt_b) * mix(cb, exp(u * -7.0), bk.w_bore);
    let x_back = bk.xc * exp(u * -6.0) * wg_smooth(0.5, 0.35, u) + x_pre * (1.0 - bk.w_bore);
    let front = u < 0.0;
    let w_c = wg_smooth(-0.35, 0.25, bk.b) * SF_CURL;
    let x = mix(x_pre, select(x_back, x_front, front), w_c);
    let y = mix(y_pre, select(y_back, y_front, front), w_c);
    let foam = sf_whitewater(x - u * lam, lam, bk);
    let face = select(0.0, 1.0, front && in_face) * wg_smooth(-0.4, 0.0, bk.b) * (1.0 - sf_face_fill(bk.b, fx));
    return vec4<f32>(x, y, foam, face);
}

fn sf_model_depth(s: f32) -> f32 {
    return min(max(s, 0.0) * SF_SLOPE + SF_TOE, SF_MAX_DEPTH);
}

fn sf_travel_time(s: f32) -> f32 {
    let a = SF_TOE;
    let k = SF_SLOPE;
    let s_knee = (SF_MAX_DEPTH - a) / k;
    let inner = min(max(s, 0.0), s_knee);
    let t_linear = (sqrt(inner * k + a) - sqrt(a)) * (2.0 / (k * sqrt(SF_G)));
    let t_flat = max(s - s_knee, 0.0) / sqrt(SF_G * SF_MAX_DEPTH);
    return t_linear + t_flat;
}

// MaterialX 2D Perlin noise (mx_noise_float(vec2)), for the whitewater patches.
fn mx_hash2(x: i32, y: i32) -> u32 {
    let seed = 0xdeadbeefu + (2u << 2u) + 13u;
    var a = seed + bitcast<u32>(x);
    var b = seed + bitcast<u32>(y);
    var c = seed;
    c = c ^ b;
    c = c - mx_rotl(b, 14u);
    a = a ^ c;
    a = a - mx_rotl(c, 11u);
    b = b ^ a;
    b = b - mx_rotl(a, 25u);
    c = c ^ b;
    c = c - mx_rotl(b, 16u);
    a = a ^ c;
    a = a - mx_rotl(c, 4u);
    b = b ^ a;
    b = b - mx_rotl(a, 14u);
    c = c ^ b;
    c = c - mx_rotl(b, 24u);
    return c;
}

fn mx_grad2(hash: u32, x: f32, y: f32) -> f32 {
    let h = hash & 7u;
    let u = select(y, x, h < 4u);
    let v = 2.0 * select(x, y, h < 4u);
    return select(u, -u, (h & 1u) != 0u) + select(v, -v, (h & 2u) != 0u);
}

fn mx_noise2(p: vec2<f32>) -> f32 {
    let fl = floor(p);
    let i = vec2<i32>(fl);
    let f = p - fl;
    let u = mx_fade(f.x);
    let v = mx_fade(f.y);
    let g00 = mx_grad2(mx_hash2(i.x, i.y), f.x, f.y);
    let g10 = mx_grad2(mx_hash2(i.x + 1, i.y), f.x - 1.0, f.y);
    let g01 = mx_grad2(mx_hash2(i.x, i.y + 1), f.x, f.y - 1.0);
    let g11 = mx_grad2(mx_hash2(i.x + 1, i.y + 1), f.x - 1.0, f.y - 1.0);
    let res = (1.0 - v) * (g00 * (1.0 - u) + g10 * u) + v * (g01 * (1.0 - u) + g11 * u);
    return 0.6616 * res;
}

struct Surf {
    height: f32,
    shoreward: f32,
    foam: f32,
    whitewater: f32,
    face: f32,
    depth: f32,
    // seconds since the last crest passed here, and how far its wave had broken (0..1, x weight)
    age: f32,
    bored: f32,
};

// The surf's offshore amplitude here: the Ocean Beach groundswell west of the
// Golden Gate; inside the bay the same fraction of it the bay keeps of the
// open-ocean spectrum's swell (water.rs `BAY_SWELL`): the JS breaks its Ocean
// Beach surf on every mapped beach, the bay's included.
fn sf_amp_scale(x: f32) -> f32 {
    // a storm's surf is bigger (1.8 x at the storm preset's wind).
    // (sea r3: the Pacific's surf 1.9 x the JS's 0.55 m: breaking crests about 2 m over the sea at Ocean Beach, on the horizon line from the sand)
    return mix(wu.bay.x, 1.9, wg_ocean(x)) * (1.0 + 0.8 * w_storm());
}

// `surfAt(P, time, offshore, along, weight)` (no bed depth: the 1:55 model),
// the offshore amplitude scaled by `amp` (1 = the JS's).
fn surf_at(time: f32, offshore: f32, along: f32, weight: f32, amp: f32) -> Surf {
    var o: Surf;
    // where the surf strip draws (surf.wgsl) this analytic surf stands down; the
    // open Pacific's share is read back from `amp` (sf_amp_scale), so no caller changes.
    let ocean_back = clamp((amp / (1.0 + 0.8 * w_storm()) - wu.bay.x) / (1.9 - wu.bay.x), 0.0, 1.0);
    let w = weight * wg_smooth(-1.0, 2.0, offshore) * wg_smooth(420.0, 220.0, offshore) * (1.0 - surf_cover_at(ocean_back, offshore, weight));
    let d = sf_model_depth(offshore);
    o.depth = d;
    if (w <= 0.0) {
        o.height = 0.0;
        o.shoreward = 0.0;
        o.foam = 0.0;
        o.whitewater = 0.0;
        o.face = 0.0;
        o.age = 0.0;
        o.bored = 0.0;
        return o;
    }
    let lam = SF_PERIOD * sqrt(d * SF_G);
    // the crest lines break in sections (peaks and shoulders 50-300 m long)
    let sections = 0.22 * sin(along * 0.011 + 0.4) + 0.14 * sin(along * 0.037 + 1.3) + 0.08 * sin(along * 0.083 + 2.2);
    let phase = sf_travel_time(offshore) / SF_PERIOD + time / SF_PERIOD + sf_wobble(along) + sections;
    let m = floor(phase + 0.5);
    let u = phase - m;
    let a_own = sf_wave_amp(m, along) * amp;
    let a_other = sf_wave_amp(m + select(-1.0, 1.0, u > 0.0), along) * amp;
    let a = mix(a_own, (a_own + a_other) * 0.5, wg_smooth(SF_TROUGH_BLEND_FROM, 0.5, abs(u)));
    let bk = sf_break(a, d);
    let prof = sf_profile(u, lam, bk);
    let since = max(u, 0.0) * SF_PERIOD;
    let bored = wg_smooth(0.9, 1.3, bk.b);
    let persist = bored * exp(since / -4.5) * 0.8;
    let patch_w = wg_smooth(-0.5, 0.45, mx_noise2(vec2<f32>(along * 0.06 + time * 0.05, m * 2.9 + 0.4)));
    let ww = prof.z * mix(0.3, 1.0, patch_w);
    // a trough never sinks below the sand under it (the bigger surf bared the bar)
    o.height = max(prof.y, -0.7 * d) * w;
    // no shoreward throw: on the sea's grid the folded triangles drew a dotted row of dark dashes in the outer surf
    o.shoreward = 0.0 * prof.x * w;
    o.foam = max(ww, persist) * w;
    o.whitewater = ww * w;
    o.face = prof.w * w;
    o.age = fract(phase) * SF_PERIOD;
    o.bored = bored * w;
    return o;
}

// ---- surf: the hand-over between the sea and the surf strip (surf.rs, surf.wgsl) ----
// eo-water surf_strip.rs `surf_cover`, the same numbers: 1 inside the strip
// (the sea stands down there: its fragments are discarded and the analytic
// surf above is faded out), 0 past the strip's outer ring, where the sea
// draws alone; across the ring the strip is blended over the sea, and both
// are the same surface (the sea's FFT, swell and faded analytic surf).
// `wu.screen.y` is 1 while the strip is attached and on.
const SFS_RING_PACIFIC: vec2<f32> = vec2<f32>(395.0, 330.0);
const SFS_RING_BAY: vec2<f32> = vec2<f32>(105.0, 80.0);
const SFS_RING_BEACH: vec2<f32> = vec2<f32>(0.85, 0.98);
const SFS_LAND_FADE: vec2<f32> = vec2<f32>(-42.0, -32.0);
// the strip is built where the Pacific swell reaches (the bay's beaches keep the analytic surf)
const SFS_PACIFIC: vec2<f32> = vec2<f32>(0.45, 0.75);

fn surf_cover_at(ocean: f32, signed: f32, beach: f32) -> f32 {
    if (wu.screen.y < 0.5) {
        return 0.0;
    }
    let ring = mix(SFS_RING_BAY, SFS_RING_PACIFIC, ocean);
    return wg_smooth(SFS_RING_BEACH.x, SFS_RING_BEACH.y, beach) * wg_smooth(ring.x, ring.y, signed) * wg_smooth(SFS_LAND_FADE.x, SFS_LAND_FADE.y, signed) * wg_smooth(SFS_PACIFIC.x, SFS_PACIFIC.y, ocean);
}

// At an undisplaced point `p` with its shore field `f` (signed distance, beach).
fn surf_cover(p: vec2<f32>, f: vec2<f32>) -> f32 {
    return surf_cover_at(wg_ocean(p.x), f.x, f.y);
}

// What the strip hands the sea's shading for its own fragments (zero for the
// sea's: a private variable is zero until the strip's fragment sets it).
struct SfInject {
    // 1 inside the strip's fragment
    on: f32,
    // the strip surface's slope (d height / d x, d z), added to the sea's
    slope: vec2<f32>,
    // foam cover, breaking face, fresh whitewater, bore, swash foam
    foam: f32,
    face: f32,
    ww: f32,
    bored: f32,
    swash: f32,
    // how much of the water's mirror glint the strip's foam covers
    spec_cut: f32,
    // 1 in the strip's churned water: its chopped, aerated surface mirrors no scene (no march)
    rough: f32,
};

var<private> sf_inj: SfInject;

// The sea's `w_shade` calls this after its own surf (one marked line): in the
// strip's fragments the strip's slope, foam and face join the sea's (whose
// analytic surf is faded out there); nothing changes for the sea's fragments.
fn sf_inject(sx: ptr<function, f32>, sz: ptr<function, f32>, whitecap: ptr<function, f32>, surf_cov: ptr<function, f32>, surf_face: ptr<function, f32>, surf_ww: ptr<function, f32>, surf_bored: ptr<function, f32>, swash_foam: ptr<function, f32>, p: vec2<f32>, f: vec2<f32>, column: f32) {
    if (sf_inj.on < 0.5) {
        // the sea's own fragments at a bay beach (Crissy Field, Aquatic Park): the swash
        // edge a continuous, wavering bubble line along the thinnest water, not a string of dots
        if (f.y > 0.01 && f.x < 30.0 && wg_ocean(p.x) < 0.5) {
            // its width and its gaps wander along the shore (a bubbly lace, not a ruled tube)
            let wob = mx_noise(vec3<f32>(p * 0.35, wu.cam.w * 0.2)) * 0.6 + mx_noise(vec3<f32>(p * 1.7, wu.cam.w * 0.5 + 3.0)) * 0.4;
            let edge = wg_smooth(0.012 + 0.03 * (0.5 + 0.5 * wob), 0.003, column);
            *swash_foam = edge * wg_smooth(-0.25, 0.3, wob) * 0.7 * f.y;
        }
        return;
    }
    *sx = *sx + sf_inj.slope.x;
    *sz = *sz + sf_inj.slope.y;
    *surf_cov = max(*surf_cov, sf_inj.foam);
    *whitecap = max(*whitecap, sf_inj.foam);
    *surf_face = max(*surf_face, sf_inj.face);
    *surf_ww = max(*surf_ww, sf_inj.ww);
    *surf_bored = max(*surf_bored, sf_inj.bored);
    *swash_foam = sf_inj.swash;
}

// the surf zone's sediment (water.wgsl's silt): the bay's beaches carry a
// fifth of Ocean Beach's (their small waves stir little sand), and water under about
// 30 cm is clear, the sand showing through it (the shallows read as brown glass).
fn surf_silt(x: f32, column: f32) -> f32 {
    return mix(0.2, 1.0, wg_ocean(x)) * wg_smooth(0.05, 0.35, column);
}

// the old surf haze's veil ends before the shore field's 400 m cap (past it
// the field's distance is constant and the veil stopped on a hard band near the horizon).
fn surf_veil(signed: f32) -> f32 {
    return wg_smooth(395.0, 300.0, signed);
}
