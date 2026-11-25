// the cloud noise volumes, baked once at start-up (clouds.rs). All tile
// seamlessly (every lattice wraps at the texture's edge), generated here, no
// stored art.
//  * base (128^3 rgba8): R a Perlin-Worley fBm (gradient noise dilated by a
//    Worley fBm: billowy cauliflower lumps), G / B / A Worley fBm at 4, 8 and 16
//    cells per tile (the low-frequency body the shape is eroded against);
//  * detail (32^3 rgba8): R / G / B Worley fBm at 2, 4 and 8 cells per tile (the
//    edge erosion: wisps at a cloud's base, billows on its top);
//  * curl (128^2 rgba8): RG the curl of a tiling 2D gradient fBm (a divergence-free
//    swirl that distorts the detail lookups), BA unused.
// Worley returns 1 - the distance to the nearest feature point (bright cell
// centres), as the base-shape remap expects.

@group(0) @binding(0) var nz_out3: texture_storage_3d<rgba8unorm, write>;
@group(0) @binding(1) var nz_src3: texture_3d<f32>;
@group(0) @binding(2) var nz_out2: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(3) var nz_src2: texture_2d<f32>;

fn nz_hash(p: vec3<i32>, salt: u32) -> vec3<f32> {
    var q = vec3<u32>(p + vec3<i32>(4096)) * vec3<u32>(1597334673u, 3812015801u, 2798796415u);
    var n = (q.x ^ q.y ^ q.z ^ (salt * 2654435761u)) * 1597334673u;
    n = n ^ (n >> 15u);
    n = n * 2246822519u;
    n = n ^ (n >> 13u);
    let a = vec3<u32>(n, n * 16807u, n * 48271u);
    return vec3<f32>(a) / 4294967296.0;
}

fn nz_wrap(c: vec3<i32>, period: i32) -> vec3<i32> {
    return ((c % period) + period) % period;
}

/// Tiling Worley (1 - nearest feature distance, in cell units) with `cells` cells per tile.
fn nz_worley(p: vec3<f32>, cells: i32, salt: u32) -> f32 {
    let q = p * f32(cells);
    let i = vec3<i32>(floor(q));
    let f = fract(q);
    var best = 1e9;
    for (var z = -1; z <= 1; z = z + 1) {
        for (var y = -1; y <= 1; y = y + 1) {
            for (var x = -1; x <= 1; x = x + 1) {
                let o = vec3<i32>(x, y, z);
                let fp = vec3<f32>(o) + nz_hash(nz_wrap(i + o, cells), salt) - f;
                best = min(best, dot(fp, fp));
            }
        }
    }
    return clamp(1.0 - sqrt(best), 0.0, 1.0);
}

fn nz_worley_fbm(p: vec3<f32>, cells: i32, salt: u32) -> f32 {
    return 0.625 * nz_worley(p, cells, salt) + 0.25 * nz_worley(p, cells * 2, salt + 1u) + 0.125 * nz_worley(p, cells * 4, salt + 2u);
}

/// Tiling 3D gradient (Perlin) noise, about -1..1, `period` cells per tile.
fn nz_perlin(p: vec3<f32>, period: i32, salt: u32) -> f32 {
    let q = p * f32(period);
    let i = vec3<i32>(floor(q));
    let f = fract(q);
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    var acc = 0.0;
    for (var k = 0; k < 8; k = k + 1) {
        let o = vec3<i32>(k & 1, (k >> 1) & 1, (k >> 2) & 1);
        let h = nz_hash(nz_wrap(i + o, period), salt) * 2.0 - 1.0;
        let g = normalize(h + vec3<f32>(1e-4));
        let w = select(1.0 - u.x, u.x, o.x == 1) * select(1.0 - u.y, u.y, o.y == 1) * select(1.0 - u.z, u.z, o.z == 1);
        acc = acc + w * dot(g, f - vec3<f32>(o));
    }
    return acc * 1.6;
}

fn nz_perlin_fbm(p: vec3<f32>, period: i32, octaves: i32, salt: u32) -> f32 {
    var v = 0.0;
    var a = 1.0;
    var norm = 0.0;
    var per = period;
    for (var o = 0; o < octaves; o = o + 1) {
        v = v + a * nz_perlin(p, per, salt + u32(o) * 7u);
        norm = norm + a;
        a = a * 0.5;
        per = per * 2;
    }
    return v / norm;
}

fn nz_remap(v: f32, lo: f32, hi: f32, a: f32, b: f32) -> f32 {
    return a + (v - lo) / max(hi - lo, 1e-4) * (b - a);
}

@compute @workgroup_size(4, 4, 4)
fn nz_base_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = textureDimensions(nz_out3);
    if (any(gid >= n)) {
        return;
    }
    let p = (vec3<f32>(gid) + 0.5) / vec3<f32>(n);
    // billowy gradient fBm (|2x - 1| folds it into rounded lumps), 0..1
    let pf = clamp(nz_perlin_fbm(p, 4, 5, 11u) * 0.5 + 0.5, 0.0, 1.0);
    let billow = 1.0 - abs(pf * 2.0 - 1.0);
    let w4 = nz_worley_fbm(p, 4, 101u);
    let w8 = nz_worley_fbm(p, 8, 211u);
    let w16 = nz_worley_fbm(p, 16, 307u);
    // Perlin-Worley: the gradient fBm with the Worley fBm's cells (lumps grown round
    // the cells' centres), stretched to fill 0..1 about a mean of ~0.5.
    let pw = clamp((mix(pf, billow, 0.3) - 0.5) * 1.6 + (w4 - 0.5) * 1.1 + 0.5, 0.0, 1.0);
    textureStore(nz_out3, vec3<i32>(gid), vec4<f32>(pw, w4, w8, w16));
}

@compute @workgroup_size(4, 4, 4)
fn nz_detail_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = textureDimensions(nz_out3);
    if (any(gid >= n)) {
        return;
    }
    let p = (vec3<f32>(gid) + 0.5) / vec3<f32>(n);
    textureStore(nz_out3, vec3<i32>(gid), vec4<f32>(nz_worley_fbm(p, 2, 401u), nz_worley_fbm(p, 4, 503u), nz_worley_fbm(p, 8, 601u), 1.0));
}

/// One mip of a 3D noise: the mean of the 2 x 2 x 2 texels above it.
@compute @workgroup_size(4, 4, 4)
fn nz_down_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = textureDimensions(nz_out3);
    if (any(gid >= n)) {
        return;
    }
    var acc = vec4<f32>(0.0);
    for (var k = 0u; k < 8u; k = k + 1u) {
        let o = vec3<u32>(k & 1u, (k >> 1u) & 1u, (k >> 2u) & 1u);
        acc = acc + textureLoad(nz_src3, vec3<i32>(gid * 2u + o), 0);
    }
    textureStore(nz_out3, vec3<i32>(gid), acc / 8.0);
}

fn nz_psi(p: vec2<f32>) -> f32 {
    return nz_perlin_fbm(vec3<f32>(p, 0.37), 4, 3, 907u);
}

@compute @workgroup_size(8, 8)
fn nz_curl_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = textureDimensions(nz_out2);
    if (any(gid.xy >= n)) {
        return;
    }
    let p = (vec2<f32>(gid.xy) + 0.5) / vec2<f32>(n);
    let e = 0.5 / f32(n.x);
    let dx = (nz_psi(p + vec2<f32>(e, 0.0)) - nz_psi(p - vec2<f32>(e, 0.0))) / (2.0 * e);
    let dy = (nz_psi(p + vec2<f32>(0.0, e)) - nz_psi(p - vec2<f32>(0.0, e))) / (2.0 * e);
    // the curl (d psi / dy, -d psi / dx), scaled into 0..1 (about +-1 after / 12)
    let c = clamp(vec2<f32>(dy, -dx) / 12.0, vec2<f32>(-1.0), vec2<f32>(1.0));
    textureStore(nz_out2, vec2<i32>(gid.xy), vec4<f32>(c * 0.5 + 0.5, 0.5, 1.0));
}

// ---- `--volumetric-clouds` noise volumes (baked lazily on first use) ----

fn nz_wrap2(c: vec2<i32>, period: i32) -> vec2<i32> {
    return ((c % period) + period) % period;
}

/// Tiling 2D Worley (1 - nearest feature distance, in cell units).
fn nz_worley2(p: vec2<f32>, cells: i32, salt: u32) -> f32 {
    let q = p * f32(cells);
    let i = vec2<i32>(floor(q));
    let f = fract(q);
    var best = 1e9;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let o = vec2<i32>(x, y);
            let w = nz_wrap2(i + o, cells);
            let fp = vec2<f32>(o) + nz_hash(vec3<i32>(w, 0), salt).xy - f;
            best = min(best, dot(fp, fp));
        }
    }
    return clamp(1.0 - sqrt(best), 0.0, 1.0);
}

/// Tiling 2D gradient (Perlin) noise, about -1..1, `period` cells per tile.
fn nz_perlin2(p: vec2<f32>, period: i32, salt: u32) -> f32 {
    let q = p * f32(period);
    let i = vec2<i32>(floor(q));
    let f = fract(q);
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    var acc = 0.0;
    for (var k = 0; k < 4; k = k + 1) {
        let o = vec2<i32>(k & 1, (k >> 1) & 1);
        let w = nz_wrap2(i + o, period);
        let ang = nz_hash(vec3<i32>(w, 1), salt).x * 6.2831853;
        let g = vec2<f32>(cos(ang), sin(ang));
        let wt = select(1.0 - u.x, u.x, o.x == 1) * select(1.0 - u.y, u.y, o.y == 1);
        acc = acc + wt * dot(g, f - vec2<f32>(o));
    }
    return acc * 1.45;
}

fn nz_perlin2_fbm(p: vec2<f32>, period: i32, octaves: i32, salt: u32) -> f32 {
    var v = 0.0;
    var a = 1.0;
    var norm = 0.0;
    var per = period;
    for (var o = 0; o < octaves; o = o + 1) {
        v = v + a * nz_perlin2(p, per, salt + u32(o) * 13u);
        norm = norm + a;
        a = a * 0.52;
        per = per * 2;
    }
    return v / norm;
}

/// Tiling 3D plump-dome Worley for `--volumetric-clouds` cauliflower detail: blends hemispherical
/// spherical caps (`1 - 1.32 d^2`, convex rounded tops with crisp V-crevices) with linear Worley.
fn nz_worley_dome(p: vec3<f32>, cells: i32, salt: u32) -> f32 {
    let q = p * f32(cells);
    let i = vec3<i32>(floor(q));
    let f = fract(q);
    var best = 1e9;
    for (var z = -1; z <= 1; z = z + 1) {
        for (var y = -1; y <= 1; y = y + 1) {
            for (var x = -1; x <= 1; x = x + 1) {
                let o = vec3<i32>(x, y, z);
                let fp = vec3<f32>(o) + nz_hash(nz_wrap(i + o, cells), salt) - f;
                best = min(best, dot(fp, fp));
            }
        }
    }
    let lin = clamp(1.0 - sqrt(best), 0.0, 1.0);
    let sph = clamp(1.0 - best * 1.32, 0.0, 1.0);
    return 0.52 * lin + 0.48 * sph;
}

/// True 3D divergence-free fluid curl (`nabla x psi`) of a seamless periodic 3D vector potential:
/// 100% isotropic across X/Y/Z and exact-periodic at `period`, turning spherical Worley blobs into
/// organic, curled, folded atmospheric fluid plumes at zero runtime cost.
fn nz_curl3(p: vec3<f32>, period: i32, salt: u32) -> vec3<f32> {
    let e = 0.015625; // 1/64 exact texel spacing
    let ox = vec3<f32>(0.31, 0.57, 0.19);
    let oy = vec3<f32>(0.67, 0.13, 0.43);
    let dpy_dz = (nz_perlin_fbm(p + ox + vec3<f32>(0.0, 0.0, e), period, 2, salt + 11u)
        - nz_perlin_fbm(p + ox - vec3<f32>(0.0, 0.0, e), period, 2, salt + 11u));
    let dpz_dy = (nz_perlin_fbm(p + oy + vec3<f32>(0.0, e, 0.0), period, 2, salt + 23u)
        - nz_perlin_fbm(p + oy - vec3<f32>(0.0, e, 0.0), period, 2, salt + 23u));
    let dpz_dx = (nz_perlin_fbm(p + oy + vec3<f32>(e, 0.0, 0.0), period, 2, salt + 23u)
        - nz_perlin_fbm(p + oy - vec3<f32>(e, 0.0, 0.0), period, 2, salt + 23u));
    let dpx_dz = (nz_perlin_fbm(p + vec3<f32>(0.0, 0.0, e), period, 2, salt)
        - nz_perlin_fbm(p - vec3<f32>(0.0, 0.0, e), period, 2, salt));
    let dpx_dy = (nz_perlin_fbm(p + vec3<f32>(0.0, e, 0.0), period, 2, salt)
        - nz_perlin_fbm(p - vec3<f32>(0.0, e, 0.0), period, 2, salt));
    let dpy_dx = (nz_perlin_fbm(p + ox + vec3<f32>(e, 0.0, 0.0), period, 2, salt + 11u)
        - nz_perlin_fbm(p + ox - vec3<f32>(e, 0.0, 0.0), period, 2, salt + 11u));
    return vec3<f32>(dpz_dy - dpy_dz, dpx_dz - dpz_dx, dpy_dx - dpx_dy) / (2.0 * e * f32(period) * 6.0);
}

/// 64^3 seamlessly-tiling, L2-cache-resident 3D divergence-free curl-advected fluid detail volume (`vc_detail`):
///  * R: 2-stage curl-advected asymmetric billowing cauliflower plumes (cells 4 + 8 + 12 + sharp ridged
///       gradient billows — kills uniform spherical foam/clay cadence at zero runtime cost)
///  * G: curl-advected crisp high-frequency boiling micro-florets (cells 6 + 12 + 20, alias-free at 64^3)
///  * B: 2-stage divergence-free curl-sheared silky torn vapor filaments for frayed rims, undersides & wisps
///  * A: curl-advected isotropic 4-octave 3D turbulent eddy field for runtime cascade swirl & altocumulus
@compute @workgroup_size(4, 4, 4)
fn nz_vol_detail_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = textureDimensions(nz_out3);
    if (any(gid >= n)) {
        return;
    }
    let p = (vec3<f32>(gid) + 0.5) / vec3<f32>(n);
    // Two-stage RK2-style divergence-free 3D fluid curl advection (isotropic & exact-periodic).
    let c1 = nz_curl3(p, 3, 1697u);
    let p1 = fract(p + c1 * 0.056);
    let c2 = nz_curl3(p1, 5, 1753u);
    let pa = fract(p1 + c2 * 0.022);

    // R: curl-advected billowing convective domes + sharp folded ridges (breaks spherical Worley monotony).
    let r_w = 0.54 * nz_worley_dome(pa, 4, 1103u) + 0.30 * nz_worley_dome(pa, 8, 1217u) + 0.16 * nz_worley_dome(pa, 12, 1301u);
    let r_p = 1.0 - abs(nz_perlin_fbm(pa, 4, 3, 1361u));
    let r_raw = 0.72 * r_w + 0.28 * r_p;
    let r_lin = clamp((r_raw - 0.23) / 0.59, 0.0, 1.0);
    let r = smoothstep(0.02, 0.98, pow(max(r_lin, 1e-5), 0.94));

    // G: crisp curl-advected boiling micro-florets (cells 6 + 12 + 20, alias-free at 64^3).
    let pb = fract(p + c1 * 0.042 + c2 * 0.018 + vec3<f32>(0.17, 0.43, 0.71));
    let g_w = 0.50 * nz_worley_dome(pb, 6, 1423u) + 0.32 * nz_worley_dome(pb, 12, 1531u) + 0.18 * nz_worley_dome(pb, 20, 1613u);
    let g_p = 1.0 - abs(nz_perlin_fbm(pb, 6, 2, 1637u));
    let g_raw = 0.80 * g_w + 0.20 * g_p;
    let g_lin = clamp((g_raw - 0.22) / 0.60, 0.0, 1.0);
    let g = smoothstep(0.02, 0.98, pow(max(g_lin, 1e-5), 0.92));

    // B: strongly curl-advected silky torn fluid vapor filaments for crisp wispy undersides & rims.
    let wp1 = fract(p + c1 * 0.095);
    let c3 = nz_curl3(wp1, 6, 1657u);
    let wp = fract(wp1 + c3 * 0.036);
    let pf = clamp(nz_perlin_fbm(wp, 4, 4, 1733u) * 0.68 + 0.5, 0.0, 1.0);
    let ridge = pow(clamp(1.0 - abs(nz_perlin_fbm(wp, 5, 4, 1789u)), 0.0, 1.0), 1.25);
    let wf = 0.52 * nz_worley(wp, 5, 1847u) + 0.30 * nz_worley(wp, 10, 1951u) + 0.18 * nz_worley(wp, 16, 1997u);
    let b = clamp((0.34 * pf + 0.40 * ridge + 0.26 * wf - 0.31) / 0.54, 0.0, 1.0);

    // A: curl-advected isotropic 4-octave 3D turbulent eddy field.
    let pa_e = fract(p + c1 * 0.034);
    let a = clamp(nz_perlin_fbm(pa_e, 3, 4, 2053u) * 0.78 + 0.5, 0.0, 1.0);
    textureStore(nz_out3, vec3<i32>(gid), vec4<f32>(r, g, b, a));
}

/// 512^2 seamlessly-tiling meteorological weather + cirrus map (`vc_weather`, 25.6 km tile):
///  * R: true scale-hierarchy cumulus ensembles (broad 12.8 km coastal fronts, 6.4 km onshore cloud
///       streets, organic 3.2 km clusters, and delicate 1.4 km scattered fair-weather scout puffs)
///  * G: high-contrast convective development field (calm low humilis/mediocris vs. rare towering
///       multi-headed congestus cathedrals — prevents uniform-height horizon pearl necklaces)
///  * B: broad synoptic air-mass humidity & LCL moisture driver
///  * A: razor-fine gossamer jet-stream fibrous mare's-tail cirrus silk (exact 5:1 integer wrap)
@compute @workgroup_size(8, 8)
fn nz_vol_weather_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = textureDimensions(nz_out2);
    if (any(gid.xy >= n)) {
        return;
    }
    let p = (vec2<f32>(gid.xy) + 0.5) / vec2<f32>(n);
    // Smooth two-stage periodic flow advection (12.8 km synoptic & 8.5 km coastal eddies).
    let w0 = vec2<f32>(nz_perlin2_fbm(p, 2, 2, 3001u), nz_perlin2_fbm(p + vec2<f32>(0.37, 0.61), 2, 2, 3049u));
    let p1 = p + w0 * 0.130;
    let w1 = vec2<f32>(nz_perlin2_fbm(p1, 3, 2, 3067u), nz_perlin2_fbm(p1 + vec2<f32>(0.53, 0.29), 3, 2, 3083u));
    let pw = p1 + w1 * 0.058;

    // Multi-scale smooth C1 cumulus ensembles & broken cloud streets on exact integer-periodic bases:
    //  * `front_iso`: broad 3.2..12.8 km isotropic synoptic cloud fronts
    //  * `st_uv = (x, -x + 2y)`: ~2.3:1 elongation along WNW -> ESE onshore breeze (exact integer wrap)
    //  * `rot_uv = (x + y, -x + y)`: 45-deg rotated organic cluster lobes
    //  * `cluster_w`: smooth 2.8..5.1 km rounded cellular cumulus hubs
    //  * `scouts`: subtle 1.6 km scattered fair-weather puff variations breaking uniform spacing
    let front_iso = clamp(nz_perlin2_fbm(pw, 2, 3, 3109u) * 0.96 + 0.5, 0.0, 1.0);
    let st_uv = vec2<f32>(pw.x, -pw.x + 2.0 * pw.y);
    let street_p = clamp(nz_perlin2_fbm(st_uv + vec2<f32>(0.29, 0.17), 2, 3, 3251u) * 0.90 + 0.5, 0.0, 1.0);
    let rot_uv = vec2<f32>(pw.x + pw.y, -pw.x + pw.y);
    let cluster_p = clamp(nz_perlin2_fbm(rot_uv + vec2<f32>(0.13, 0.47), 2, 3, 3361u) * 0.88 + 0.5, 0.0, 1.0);
    let w_cell = 0.58 * nz_worley2(pw, 5, 3413u) + 0.42 * nz_worley2(pw + vec2<f32>(0.31, 0.19), 9, 3469u);
    let cluster_w = smoothstep(0.14, 0.86, w_cell);
    let scouts = clamp(nz_perlin2_fbm(pw + vec2<f32>(0.71, 0.23), 8, 2, 3527u) * 0.72 + 0.5, 0.0, 1.0);
    let cov_mix = 0.34 * front_iso + 0.26 * street_p + 0.18 * cluster_p + 0.14 * cluster_w + 0.08 * scouts;
    // Clean open-sky valleys (0.00) with a soft upper knee (peak ~0.98, zero flat clipped plateaus).
    let cov_lin = clamp((cov_mix - 0.308) / 0.442, 0.0, 1.0);
    let cov = (cov_lin * (1.0 - 0.16 * cov_lin * cov_lin)) / 0.84 * 0.98;

    // High-contrast convective instability field: separates delicate low humilis and mid mediocris
    // from rare colossal congestus cores so skylines vary dramatically instead of flat uniform fences.
    let instab = clamp(nz_perlin2_fbm(p1 + vec2<f32>(0.19, 0.43), 2, 3, 3607u) * 0.92 + 0.5, 0.0, 1.0);
    let cores = clamp(nz_perlin2_fbm(pw + vec2<f32>(0.53, 0.17), 3, 3, 3709u) * 0.88 + 0.5, 0.0, 1.0);
    let ctype_lin = smoothstep(0.14, 0.86, 0.46 * instab + 0.34 * cores + 0.20 * pow(cov, 1.25));
    let ctype = clamp(pow(max(ctype_lin, 1e-5), 1.18), 0.0, 1.0);

    // Broad air-mass humidity & LCL field, predominantly decoupled from `cov`.
    let air_hum = clamp(nz_perlin2_fbm(p1 + vec2<f32>(0.27, 0.59), 2, 4, 3917u) * 0.92 + 0.5, 0.0, 1.0);
    let air_alt = clamp(nz_perlin2_fbm(rot_uv + vec2<f32>(0.41, 0.73), 2, 3, 3967u) * 0.84 + 0.5, 0.0, 1.0);
    let moist_raw = 0.62 * air_hum + 0.28 * air_alt + 0.10 * ctype;
    let moist = clamp(smoothstep(0.14, 0.86, moist_raw), 0.0, 1.0);

    // Crisp gossamer jet-stream fibrous mare's-tail cirrus silk: exact integer-periodic wind shear
    // `(x + 2y, -4x + 2y)` curled into fine sweeping ice-crystal wisps with crisp sky between bands.
    let cw = vec2<f32>(nz_perlin2_fbm(p, 4, 3, 4001u), nz_perlin2_fbm(p + vec2<f32>(0.41, 0.73), 4, 3, 4099u));
    let pc = p + cw * 0.058;
    let c_uv = vec2<f32>(pc.x + 2.0 * pc.y, -4.0 * pc.x + 2.0 * pc.y);
    let c_env = clamp(nz_perlin2_fbm(c_uv, 2, 3, 4201u) * 0.76 + 0.5, 0.0, 1.0);
    let c_fil1 = 1.0 - abs(nz_perlin2_fbm(c_uv, 6, 4, 4327u));
    let c_fil2 = 1.0 - abs(nz_perlin2_fbm(c_uv + vec2<f32>(0.23, 0.67), 14, 3, 4441u));
    let wisps = pow(clamp(0.50 * c_fil1 + 0.50 * c_fil2, 0.0, 1.0), 2.9);
    let cirrus = clamp(smoothstep(0.40, 0.88, c_env) * smoothstep(0.14, 0.86, wisps), 0.0, 1.0);

    textureStore(nz_out2, vec2<i32>(gid.xy), vec4<f32>(cov, ctype, moist, cirrus));
}

/// One mip of a 2D noise texture: the mean of the 2 x 2 texels above it.
@compute @workgroup_size(8, 8)
fn nz_down2_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = textureDimensions(nz_out2);
    if (any(gid.xy >= n)) {
        return;
    }
    var acc = vec4<f32>(0.0);
    for (var k = 0u; k < 4u; k = k + 1u) {
        let o = vec2<u32>(k & 1u, (k >> 1u) & 1u);
        acc = acc + textureLoad(nz_src2, vec2<i32>(gid.xy * 2u + o), 0);
    }
    textureStore(nz_out2, vec2<i32>(gid.xy), acc * 0.25);
}
