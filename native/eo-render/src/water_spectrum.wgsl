// the sea state's spectrum on the GPU (eo-water waves.rs `texel_h0`, the
// same numbers in f32). One invocation per texel of the four cascades: the
// JONSWAP systems the plan puts on this cascade, each spread around its
// travel direction (cos-2s), give the energy travelling along +k (the second
// draw; the first is 0, every direction is a texel of its own), so the sea
// runs downwind. The lead swell texel of cascade 0 is set outright.
// Run when the plan changes (water.rs), not every frame.

struct SpecParams {
    sizes: vec4<f32>,
    k_lo: vec4<f32>,
    k_hi: vec4<f32>,
    // per system: (scale, omega_p, ln gamma, travel direction)
    sa: array<vec4<f32>, 3>,
    // per system: (fixed cos-2s exponent or 0, cascade mask, 0, 0)
    sb: array<vec4<f32>, 3>,
    // x lead texel (cascade 0), y amplitude (m), z phase (rad), w short-wave fade (m)
    lead: vec4<f32>,
    // x depth (m)
    misc: vec4<f32>,
};

@group(0) @binding(0) var<uniform> spp: SpecParams;
@group(0) @binding(1) var<storage, read> gauss: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read_write> h0_out: array<vec4<f32>>;

const SN: u32 = 256u;
const SHALF: f32 = 128.0;
const S_TWO_PI: f32 = 6.283185307;
const S_PI: f32 = 3.14159265;
const S_G: f32 = 9.81;

fn s_norm(s: f32) -> f32 {
    if (s < 5.0) {
        let s2 = s * s;
        return -0.000564 * s2 * s2 + 0.00776 * s2 * s - 0.044 * s2 + 0.192 * s + 0.163;
    }
    return sqrt(s / (4.0 * S_PI)) * (1.0 + 1.0 / (8.0 * s));
}

fn s_spread(rel: f32, s: f32) -> f32 {
    return s_norm(s) * pow(abs(cos(rel * 0.5)), 2.0 * s);
}

fn s_jonswap(w: f32, a: vec4<f32>) -> f32 {
    let wp = a.y;
    let sigma = select(0.09, 0.07, w <= wp);
    let d = (w - wp) / (sigma * wp);
    let r = exp(-0.5 * d * d);
    let q = wp / w;
    let q2 = q * q;
    return a.x * pow(w, -5.0) * exp(-1.25 * q2 * q2) * exp(r * a.z);
}

@compute @workgroup_size(64)
fn spectrum_init(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= SN * SN * 4u) {
        return;
    }
    let c = i / (SN * SN);
    let within = i % (SN * SN);
    if (c == 0u && within == u32(spp.lead.x)) {
        h0_out[i] = vec4<f32>(0.0, 0.0, spp.lead.y * cos(spp.lead.z), spp.lead.y * sin(spp.lead.z));
        return;
    }
    let m = within / SN;
    let n = within % SN;
    let size = spp.sizes[c];
    let kx = (f32(n) - SHALF) * S_TWO_PI / size;
    let kz = (f32(m) - SHALF) * S_TWO_PI / size;
    let k = sqrt(kx * kx + kz * kz);
    if (k < 1e-6 || k < spp.k_lo[c] || k >= spp.k_hi[c]) {
        h0_out[i] = vec4<f32>(0.0);
        return;
    }
    let kd = min(k * spp.misc.x, 20.0);
    let th = tanh(kd);
    let w = sqrt(S_G * k * th);
    let dwdk = S_G * (th + kd * (1.0 - th * th)) / (2.0 * w);
    let theta = atan2(kz, kx);
    let fade = exp(-(k * spp.lead.w) * (k * spp.lead.w));
    var eb = 0.0;
    let bit = 1u << c;
    for (var s = 0u; s < 3u; s = s + 1u) {
        if ((u32(spp.sb[s].y + 0.5) & bit) == 0u) {
            continue;
        }
        let a = spp.sa[s];
        let sw = s_jonswap(w, a) * fade;
        var sp = spp.sb[s].x;
        if (sp <= 0.0) {
            let x = w / a.y;
            sp = max(select(9.77 * pow(x, 5.0), 9.77 * pow(x, -2.5), x > 1.0), 0.3);
        }
        eb = eb + sw * s_spread(theta - a.w, sp);
    }
    let dk = S_TWO_PI / size;
    let jac = dwdk / k * dk * dk;
    let ab = sqrt(2.0 * eb * jac);
    let g = gauss[i];
    h0_out[i] = vec4<f32>(0.0, 0.0, g.z * ab, g.w * ab) * 0.70710678;
}
