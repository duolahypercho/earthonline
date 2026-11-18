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
