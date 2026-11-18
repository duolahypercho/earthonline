// the cascaded FFT ocean (src/render/materials/ocean-fft.js, tidewater's
// Tessendorf transform), on the GPU in two dispatches instead of the JS's 18:
//
//  fft_rows: one workgroup per (cascade, row m). Evolves the spectrum to time
//            t for the row (`evolve`), then a 256-point inverse FFT along x in
//            workgroup memory (radix-2 Stockham, 8 stages).
//  fft_cols: one workgroup per (cascade, column n): the inverse FFT along z,
//            then `finalize` (the (-1)^(x+z) sign, choppiness, the Jacobian
//            foam accumulation) and the two output layers.
//
// The JS transforms eight complex fields and keeps their real parts. The
// real part of the inverse transform of a spectrum F equals the transform of
// its Hermitian part F_h(k) = (F(k) + conj(F(-k))) / 2, so each field is
// symmetrised here (reading the texel at -k) and two real fields share one
// complex transform (a + i b): four transforms instead of eight, the same
// numbers. eo-render test `water` compares the output with eo-water's CPU
// sum of the same spectrum.
//
// the spectrum is the sea state's (eo-water waves.rs, built by
// water_spectrum.wgsl); the choppiness is negative (points gather toward the
// crests, which sharpen, and the Jacobian falls below 1 on them); the fourth
// transform's free half carries dDx/dz (the Jacobian's cross term); the foam
// is the whitecaps of two places (the open Pacific, the bay): where that
// place's Jacobian falls below its threshold (water.rs sets them from the
// plan so the foam covers what the wind gives) foam is injected, decays over
// seconds, drifts downwind and spreads along the wind into streaks
// (ping-pong buffers read upwind). Outputs: displacement (dx, height, dz,
// either place's foam), derivatives (dh/dx, dh/dz, Pacific foam, bay foam).

struct FftParams {
    time: f32,
    dt: f32,
    choppiness: f32,
    depth: f32,
    sizes: vec4<f32>,
    // per cascade, each place's amplitude (the field times these is its sea)
    amp_pac: vec4<f32>,
    amp_bay: vec4<f32>,
    // per cascade and place: the Jacobian's whitecap threshold and its softness
    thr_pac: vec4<f32>,
    thr_bay: vec4<f32>,
    soft_pac: vec4<f32>,
    soft_bay: vec4<f32>,
    // x 1/lifetime Pacific, y 1/lifetime bay (1/s), z injection rate (1/s), w along-wind spread (texel^2/s)
    foam: vec4<f32>,
    // xy cascade 0's wind (unit, toward), zw the shorter cascades'
    wind: vec4<f32>,
    // x cascade 0's foam drift, y the shorter cascades' (m/s)
    drift: vec4<f32>,
};

@group(0) @binding(0) var<uniform> fp: FftParams;
@group(0) @binding(1) var<storage, read> h0: array<vec4<f32>>;
// Two vec4 per texel: (C0, C1), (C2, C3), each a complex pair.
@group(0) @binding(2) var<storage, read_write> spec: array<vec4<f32>>;
// the last step's foam (Pacific, bay) per texel; this step's goes to foam_out.
@group(0) @binding(3) var<storage, read> foam_in: array<vec2<f32>>;
@group(0) @binding(4) var disp_out: texture_storage_2d_array<rgba16float, write>;
@group(0) @binding(5) var deriv_out: texture_storage_2d_array<rgba16float, write>;
@group(0) @binding(6) var<storage, read_write> foam_out: array<vec2<f32>>;

const N: u32 = 256u;
const HALF: u32 = 128u;
const LOG2N: u32 = 8u;
const TWO_PI: f32 = 6.283185307;
const GRAVITY: f32 = 9.81;

var<workgroup> a0: array<vec4<f32>, 256>;
var<workgroup> b0: array<vec4<f32>, 256>;
var<workgroup> a1: array<vec4<f32>, 256>;
var<workgroup> b1: array<vec4<f32>, 256>;

fn cmul(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(a.x * b.x - a.y * b.y, a.x * b.y + a.y * b.x);
}

fn conj(a: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(a.x, -a.y);
}

// The seven fields `evolve` derives from h(k, t) at one texel (k as stored).
struct Fields {
    h: vec2<f32>,
    dx: vec2<f32>,
    dz: vec2<f32>,
    dhdx: vec2<f32>,
    dhdz: vec2<f32>,
    dxdx: vec2<f32>,
    dzdz: vec2<f32>,
    dxdz: vec2<f32>, // the Jacobian's cross term
};

fn fields_at(c: u32, m: u32, n: u32) -> Fields {
    let size = fp.sizes[c];
    let kx = (f32(n) - f32(HALF)) * TWO_PI / size;
    let kz = (f32(m) - f32(HALF)) * TWO_PI / size;
    let k = sqrt(kx * kx + kz * kz);
    let omega = sqrt(GRAVITY * k * tanh(min(k * fp.depth, 20.0)));
    let g = h0[c * N * N + m * N + n];
    let ph = omega * fp.time;
    let cp = cos(ph);
    let sp = sin(ph);
    // h(k,t) = h0(k) e^{iwt} + conj(h0'(k)) e^{-iwt}
    let h = vec2<f32>(g.x * cp - g.y * sp + g.z * cp + g.w * sp, g.x * sp + g.y * cp - g.z * sp + g.w * cp);
    let inv_k = 1.0 / max(k, 1e-6);
    var f: Fields;
    f.h = h;
    f.dx = vec2<f32>(h.y * kx * inv_k, -h.x * kx * inv_k);
    f.dz = vec2<f32>(h.y * kz * inv_k, -h.x * kz * inv_k);
    f.dhdx = vec2<f32>(-h.y * kx, h.x * kx);
    f.dhdz = vec2<f32>(-h.y * kz, h.x * kz);
    f.dxdx = vec2<f32>(-f.dx.y * kx, f.dx.x * kx);
    f.dzdz = vec2<f32>(-f.dz.y * kz, f.dz.x * kz);
    f.dxdz = vec2<f32>(-f.dx.y * kz, f.dx.x * kz); // waves
    return f;
}

// The four packed transforms at one texel: Hermitian parts of the fields,
// two real fields per complex transform.
fn packed(c: u32, m: u32, n: u32) -> array<vec4<f32>, 2> {
    let p = fields_at(c, m, n);
    let q = fields_at(c, (N - m) % N, (N - n) % N);
    let h = (p.h + conj(q.h)) * 0.5;
    let dx = (p.dx + conj(q.dx)) * 0.5;
    let dz = (p.dz + conj(q.dz)) * 0.5;
    let dhdx = (p.dhdx + conj(q.dhdx)) * 0.5;
    let dhdz = (p.dhdz + conj(q.dhdz)) * 0.5;
    let dxdx = (p.dxdx + conj(q.dxdx)) * 0.5;
    let dzdz = (p.dzdz + conj(q.dzdz)) * 0.5;
    let dxdz = (p.dxdz + conj(q.dxdz)) * 0.5; // waves
    // a + i b: (a.re - b.im, a.im + b.re)
    let c0 = vec2<f32>(h.x - dx.y, h.y + dx.x);
    let c1 = vec2<f32>(dz.x - dhdx.y, dz.y + dhdx.x);
    let c2 = vec2<f32>(dhdz.x - dxdx.y, dhdz.y + dxdx.x);
    let c3 = vec2<f32>(dzdz.x - dxdz.y, dzdz.y + dxdz.x); // + i dDx/dz
    return array<vec4<f32>, 2>(vec4<f32>(c0, c1), vec4<f32>(c2, c3));
}

fn butterfly(a: vec4<f32>, b: vec4<f32>, w: vec2<f32>) -> array<vec4<f32>, 2> {
    let r0 = cmul(b.xy, w);
    let r1 = cmul(b.zw, w);
    let t = vec4<f32>(r0, r1);
    return array<vec4<f32>, 2>(a + t, a - t);
}

// 8 radix-2 Stockham stages over the workgroup arrays; thread j owns
// butterfly j each stage. The result ends in (a0, b0) (an even stage count).
fn stockham(j: u32) {
    var ns = 1u;
    for (var s = 0u; s < LOG2N; s = s + 1u) {
        let k = j % ns;
        let ang = TWO_PI * f32(k) / f32(2u * ns);
        let w = vec2<f32>(cos(ang), sin(ang));
        let d = (j / ns) * ns * 2u + k;
        if ((s & 1u) == 0u) {
            let ra = butterfly(a0[j], a0[j + HALF], w);
            let rb = butterfly(b0[j], b0[j + HALF], w);
            a1[d] = ra[0];
            a1[d + ns] = ra[1];
            b1[d] = rb[0];
            b1[d + ns] = rb[1];
        } else {
            let ra = butterfly(a1[j], a1[j + HALF], w);
            let rb = butterfly(b1[j], b1[j + HALF], w);
            a0[d] = ra[0];
            a0[d + ns] = ra[1];
            b0[d] = rb[0];
            b0[d + ns] = rb[1];
        }
        workgroupBarrier();
        ns = ns * 2u;
    }
}

@compute @workgroup_size(128)
fn fft_rows(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>) {
    let c = wg.x / N;
    let m = wg.x % N;
    let j = lid.x;
    for (var e = 0u; e < 2u; e = e + 1u) {
        let n = j + e * HALF;
        let p = packed(c, m, n);
        a0[n] = p[0];
        b0[n] = p[1];
    }
    workgroupBarrier();
    stockham(j);
    for (var e = 0u; e < 2u; e = e + 1u) {
        let n = j + e * HALF;
        let at = (c * N * N + m * N + n) * 2u;
        spec[at] = a0[n];
        spec[at + 1u] = b0[n];
    }
}

@compute @workgroup_size(128)
fn fft_cols(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>) {
    let c = wg.x / N;
    let n = wg.x % N;
    let j = lid.x;
    for (var e = 0u; e < 2u; e = e + 1u) {
        let m = j + e * HALF;
        let at = (c * N * N + m * N + n) * 2u;
        a0[m] = spec[at];
        b0[m] = spec[at + 1u];
    }
    workgroupBarrier();
    stockham(j);
    for (var e = 0u; e < 2u; e = e + 1u) {
        let m = j + e * HALF;
        let sgn = select(-1.0, 1.0, ((m + n) & 1u) == 0u);
        let u = a0[m] * sgn;
        let v = b0[m] * sgn;
        let height = u.x;
        let dx = u.y * fp.choppiness;
        let dz = u.z * fp.choppiness;
        let dydx = u.w;
        let dydz = v.x;
        let dxdx = v.y * fp.choppiness;
        let dzdz = v.z * fp.choppiness;
        let dxdz = v.w * fp.choppiness; // waves
        // the Jacobian of the horizontal map, per place (its amplitude
        // scales the strains): below 1 the surface converges, below 0 it folds,
        // which is where a real sea breaks and aerates.
        let ap = fp.amp_pac[c];
        let ab = fp.amp_bay[c];
        let j_pac = (1.0 + ap * dxdx) * (1.0 + ap * dzdz) - ap * ap * dxdz * dxdz;
        let j_bay = (1.0 + ab * dxdx) * (1.0 + ab * dzdz) - ab * ab * dxdz * dxdz;
        let fo = w_foam_step(c, m, n, vec2<f32>(j_pac, j_bay));
        foam_out[c * N * N + m * N + n] = fo;
        textureStore(disp_out, vec2<u32>(n, m), c, vec4<f32>(dx, height, dz, max(fo.x, fo.y)));
        textureStore(deriv_out, vec2<u32>(n, m), c, vec4<f32>(dydx, dydz, fo.x, fo.y));
    }
}

// last step's foam at a texel position (bilinear, the tile wraps).
fn w_foam_at(c: u32, x: f32, y: f32) -> vec2<f32> {
    let fx = floor(x);
    let fy = floor(y);
    let t = vec2<f32>(x - fx, y - fy);
    let x0 = u32(i32(fx) & 255);
    let y0 = u32(i32(fy) & 255);
    let x1 = (x0 + 1u) & 255u;
    let y1 = (y0 + 1u) & 255u;
    let base = c * N * N;
    let a = mix(foam_in[base + y0 * N + x0], foam_in[base + y0 * N + x1], t.x);
    let b = mix(foam_in[base + y1 * N + x0], foam_in[base + y1 * N + x1], t.x);
    return mix(a, b, t.y);
}

// one step of the whitecap foam at texel (n, m) of cascade c: last
// step's foam carried downwind (read upwind) and spread along the wind, faded
// by its lifetime, plus what the surface injects where each place's Jacobian
// is below its threshold.
fn w_foam_step(c: u32, m: u32, n: u32, j: vec2<f32>) -> vec2<f32> {
    let dt = fp.dt;
    let wind = select(fp.wind.zw, fp.wind.xy, c == 0u);
    let texel = fp.sizes[c] / f32(N);
    let carry = select(fp.drift.y, fp.drift.x, c == 0u) * dt / texel;
    let x = f32(n) - wind.x * carry;
    let y = f32(m) - wind.y * carry;
    var old = w_foam_at(c, x, y);
    let kappa = clamp(fp.foam.w * dt, 0.0, 0.45);
    if (kappa > 0.0) {
        let side = 0.5 * (w_foam_at(c, x + wind.x, y + wind.y) + w_foam_at(c, x - wind.x, y - wind.y));
        old = mix(old, side, kappa);
    }
    // windrows: old foam lasts on lines along the wind (the
    // convergence lines of the wind's roll vortices, 3-40 m apart by cascade),
    // fades fast between them, so a whitecap's trail draws out into streaks.
    // Whole cycles across the tile, so the lines wrap with it.
    let cyc = array<f32, 4>(64.0, 40.0, 20.0, 8.0)[c];
    let across = round(vec2<f32>(-wind.y, wind.x) * cyc);
    let along = round(wind * cyc * 0.21);
    let q = (f32(n) * across.x + f32(m) * across.y) / f32(N);
    let r = (f32(n) * along.x + f32(m) * along.y) / f32(N);
    let row = 0.5 + 0.5 * sin(TWO_PI * q + 1.3 * sin(TWO_PI * r));
    let kept = old * exp(-dt * fp.foam.xy * mix(1.8, 0.45, row * row));
    let thr = vec2<f32>(fp.thr_pac[c], fp.thr_bay[c]);
    let soft = vec2<f32>(fp.soft_pac[c], fp.soft_bay[c]);
    let inject = clamp((thr - j) / soft, vec2<f32>(0.0), vec2<f32>(1.0));
    let take = min(fp.foam.z * dt, 1.0);
    return clamp(kept + (vec2<f32>(1.0) - kept) * inject * take, vec2<f32>(0.0), vec2<f32>(1.0));
}
