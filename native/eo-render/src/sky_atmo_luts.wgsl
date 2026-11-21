// the atmosphere tables' compute passes (appended after
// sky_common.wgsl + sky_atmo.wgsl). Binding 0 the sky uniform, 1 the table
// written, 2-3 the transmittance and multiple-scattering tables read, 4 a
// clamped linear sampler; one bind group per pass.

@group(0) @binding(0) var<uniform> skyu: Sky;
@group(0) @binding(1) var at_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(2) var at_trans: texture_2d<f32>;
@group(0) @binding(3) var at_ms: texture_2d<f32>;
@group(0) @binding(4) var at_lin: sampler;
// the aerial-perspective volume's two tables (its pass only; the others bind 1 x 1 x 1 placeholders)
@group(0) @binding(5) var at_ap_in: texture_storage_3d<rgba16float, write>;
@group(0) @binding(6) var at_ap_tr: texture_storage_3d<rgba16float, write>;

// the medium at a height (km): the aerosol's scattering, absorption, scale height
// and spectral factor from the uniform (`atmo_m`, `atmo_s`: eo-look `atmosphere`, the
// clear coastal air with the physical daylight, else the standard aerosol x the haze).
// The coastal aerosol scatters and absorbs more of the blue than the red (the
// Angstrom factor): grey aerosol over a reddened sun mixed with the Rayleigh blue
// into an olive-khaki low sky at golden hour.
fn at_medium(h: f32) -> AtMedium {
    return at_medium_of(h, skyu.atmo_m, skyu.atmo_s, skyu.atmo_r, skyu.atmo_o);
}


fn at_transmittance(r: f32, mu: f32) -> vec3<f32> {
    if (at_ground_hit(r, mu)) {
        return vec3<f32>(0.0);
    }
    return textureSampleLevel(at_trans, at_lin, at_trans_uv(r, mu), 0.0).rgb;
}

fn at_multi(r: f32, mu_s: f32) -> vec3<f32> {
    let uv = vec2<f32>(mu_s * 0.5 + 0.5, (r - AT_R) / (AT_TOP - AT_R));
    return textureSampleLevel(at_ms, at_lin, uv, 0.0).rgb;
}

@compute @workgroup_size(8, 8)
fn at_trans_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = textureDimensions(at_out);
    if (gid.x >= n.x || gid.y >= n.y) {
        return;
    }
    let uv = (vec2<f32>(gid.xy) + 0.5) / vec2<f32>(n);
    let hh = sqrt(AT_TOP * AT_TOP - AT_R * AT_R);
    let rho = hh * uv.y;
    let r = sqrt(rho * rho + AT_R * AT_R);
    let d_min = AT_TOP - r;
    let d_max = rho + hh;
    let d = d_min + uv.x * (d_max - d_min);
    var mu = 1.0;
    if (d > 0.0) {
        mu = clamp((hh * hh - rho * rho - d * d) / (2.0 * r * d), -1.0, 1.0);
    }
    let len = max(at_sphere(r, mu, AT_TOP), 0.0);
    let steps = 40;
    var tau = vec3<f32>(0.0);
    let dt = len / f32(steps);
    for (var i = 0; i < steps; i = i + 1) {
        let t = (f32(i) + 0.5) * dt;
        let h = sqrt(r * r + t * t + 2.0 * r * mu * t) - AT_R;
        tau = tau + at_medium(h).ext * dt;
    }
    textureStore(at_out, vec2<i32>(gid.xy), vec4<f32>(exp(-tau), 1.0));
}

/// The multiple-scattering transfer (Hillaire 2020, section 5.5): the second
/// order over 64 directions, isotropic, and its geometric series 1 / (1 - f_ms).
@compute @workgroup_size(8, 8)
fn at_ms_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = textureDimensions(at_out);
    if (gid.x >= n.x || gid.y >= n.y) {
        return;
    }
    let uv = (vec2<f32>(gid.xy) + 0.5) / vec2<f32>(n);
    let mu_s = uv.x * 2.0 - 1.0;
    let r = AT_R + max(uv.y, 0.002) * (AT_TOP - AT_R);
    let sun = vec3<f32>(sqrt(max(1.0 - mu_s * mu_s, 0.0)), mu_s, 0.0);
    var l2 = vec3<f32>(0.0);
    var fms = vec3<f32>(0.0);
    let nd = 64;
    for (var k = 0; k < nd; k = k + 1) {
        // A Fibonacci sphere.
        let z = 1.0 - (f32(k) + 0.5) * 2.0 / f32(nd);
        let rr = sqrt(max(1.0 - z * z, 0.0));
        let ph = f32(k) * 2.39996323;
        let dir = vec3<f32>(rr * cos(ph), z, rr * sin(ph));
        let mu = dir.y;
        var len = at_sphere(r, mu, AT_TOP);
        let g = at_sphere(r, mu, AT_R);
        if (g > 0.0) {
            len = g;
        }
        len = max(len, 0.0);
        let steps = 20;
        let dt = len / f32(steps);
        var trans = vec3<f32>(1.0);
        for (var i = 0; i < steps; i = i + 1) {
            let t = (f32(i) + 0.5) * dt;
            let p = vec3<f32>(0.0, r, 0.0) + dir * t;
            let pr = length(p);
            let m = at_medium(pr - AT_R);
            let up = p / pr;
            let ts = at_transmittance(pr, dot(up, sun));
            let st = exp(-m.ext * dt);
            let k_int = (vec3<f32>(1.0) - st) / max(m.ext, vec3<f32>(1e-7));
            l2 = l2 + trans * m.scat * ts * (1.0 / (4.0 * AT_PI)) * k_int;
            fms = fms + trans * m.scat * k_int;
            trans = trans * st;
        }
    }
    l2 = l2 / f32(nd);
    fms = fms / f32(nd);
    let psi = l2 / max(vec3<f32>(1.0) - fms, vec3<f32>(1e-3));
    textureStore(at_out, vec2<i32>(gid.xy), vec4<f32>(psi, 1.0));
}

/// the aerial-perspective volume (Hillaire 2020, section 5.4, here by
/// direction about the sun as the sky-view table, not by screen position, so
/// it holds for every pass that knows a direction and a distance): along each
/// of the table's directions from the eye, the light scattered in (single
/// scattering with the planet's shadow plus the multiple-scattering transfer,
/// per unit solar illuminance) and the transmittance over the distance to the
/// end of each slice, slice k ending at reach x ((k + 1) / n)^2. Four steps a slice.
@compute @workgroup_size(8, 8)
fn at_ap_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = textureDimensions(at_ap_in);
    if (gid.x >= n.x || gid.y >= n.y) {
        return;
    }
    let uv = (vec2<f32>(gid.xy) + 0.5) / vec2<f32>(n.xy);
    let sun = skyu.atmo.xyz;
    let dir = at_view_dir(uv, sun);
    let r0 = AT_R + max(skyu.eye.y, 1.0) * 0.001;
    let cth = dot(dir, sun);
    let p_ray = at_rayleigh_phase(cth);
    let p_mie = at_mie_phase(cth);
    let reach = skyu.atmo_s.w;
    var lum = vec3<f32>(0.0);
    var trans = vec3<f32>(1.0);
    var t_prev = 0.0;
    let sub = 4;
    for (var k = 0u; k < n.z; k = k + 1u) {
        let fk = (f32(k) + 1.0) / f32(n.z);
        let t_end = reach * fk * fk;
        let seg = (t_end - t_prev) / f32(sub);
        for (var j = 0; j < sub; j = j + 1) {
            let t = t_prev + (f32(j) + 0.5) * seg;
            let p = vec3<f32>(0.0, r0, 0.0) + dir * t;
            let pr = length(p);
            let m = at_medium(pr - AT_R);
            let up = p / pr;
            let mus = dot(up, sun);
            let ts = at_transmittance(pr, mus);
            let single = (m.ray * p_ray + m.mie * p_mie) * ts;
            let multi = m.scat * at_multi(pr, mus);
            let st = exp(-m.ext * seg);
            let k_int = (vec3<f32>(1.0) - st) / max(m.ext, vec3<f32>(1e-7));
            lum = lum + trans * (single + multi) * k_int;
            trans = trans * st;
        }
        t_prev = t_end;
        textureStore(at_ap_in, vec3<u32>(gid.xy, k), vec4<f32>(lum, 1.0));
        textureStore(at_ap_tr, vec3<u32>(gid.xy, k), vec4<f32>(trans, 1.0));
    }
}

/// The sky seen from the eye (Hillaire 2020, section 5.3): single scattering
/// with the planet's shadow plus the multiple-scattering transfer, 32 steps.
@compute @workgroup_size(8, 8)
fn at_view_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = textureDimensions(at_out);
    if (gid.x >= n.x || gid.y >= n.y) {
        return;
    }
    let uv = (vec2<f32>(gid.xy) + 0.5) / vec2<f32>(n);
    let sun = skyu.atmo.xyz;
    let dir = at_view_dir(uv, sun);
    let r = AT_R + max(skyu.eye.y, 1.0) * 0.001;
    let mu = dir.y;
    var len = at_sphere(r, mu, AT_TOP);
    let g = at_sphere(r, mu, AT_R);
    if (g > 0.0) {
        len = g;
    }
    len = min(max(len, 0.0), 1200.0);
    let steps = 32;
    var lum = vec3<f32>(0.0);
    var trans = vec3<f32>(1.0);
    let cth = dot(dir, sun);
    let p_ray = at_rayleigh_phase(cth);
    let p_mie = at_mie_phase(cth);
    var t_prev = 0.0;
    for (var i = 0; i < steps; i = i + 1) {
        // Denser near the eye.
        let f = (f32(i) + 0.5) / f32(steps);
        let t = len * f * f;
        let fn1 = (f32(i) + 1.0) / f32(steps);
        let t_next = len * fn1 * fn1;
        let dt = t_next - t_prev;
        t_prev = t_next;
        let p = vec3<f32>(0.0, r, 0.0) + dir * t;
        let pr = length(p);
        let m = at_medium(pr - AT_R);
        let up = p / pr;
        let mus = dot(up, sun);
        let ts = at_transmittance(pr, mus);
        let single = (m.ray * p_ray + m.mie * p_mie) * ts;
        let multi = m.scat * at_multi(pr, mus);
        let st = exp(-m.ext * dt);
        let k_int = (vec3<f32>(1.0) - st) / max(m.ext, vec3<f32>(1e-7));
        lum = lum + trans * (single + multi) * k_int;
        trans = trans * st;
    }
    textureStore(at_out, vec2<i32>(gid.xy), vec4<f32>(lum, 1.0));
}
