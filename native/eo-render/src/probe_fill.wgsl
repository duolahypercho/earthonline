// finishing a scene-probe capture (prepended with look_common.wgsl).
//
//  fill: level 0 of the cube from the six captured faces; where nothing was
//        drawn (alpha 0) the analytic sky the look evaluates, exactly as the
//        JS bake hides the display sky and paints the analytic radiance as the
//        background (environment-ibl.js `bakeSceneCube`).
//  mip:  2 x 2 box levels 1..5 (the roughness lookups read them).
//  lobe: the cosine lobe about +Y over level 3, of the capture and of the
//        analytic sky over the same texels: their ratio is the bake's TRANSFER
//        (`probeBakeTransfer`: an empty scene measures exactly 1).
//
// Faces are a 2D array (0 +X, 1 -X, 2 +Y, 3 -Y, 4 +Z, 5 -Z); face f's
// forward F, up U and right R = F x U are `probe_basis` (probe.rs renders
// each face with the same basis, draw.wgsl samples with it).

@group(0) @binding(0) var<uniform> look: Look;
@group(0) @binding(1) var src: texture_2d_array<f32>;
@group(0) @binding(2) var dst: texture_storage_2d_array<rgba16float, write>;
// [0] capture lobe (cosine about +Y), [1] unused, [2] weight, [3] 1; [4 + 4k ..] SH coefficient k (rgb):
// the capture's irradiance, cosine-convolved SH9 (as eo-look `irradiance_sh`).
@group(0) @binding(3) var<storage, read_write> lobe: array<f32, 40>;

fn probe_basis(f: u32) -> mat3x3<f32> {
    // columns: right, up, forward
    switch f {
        case 0u: { return mat3x3<f32>(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0)); }
        case 1u: { return mat3x3<f32>(vec3<f32>(0.0, 0.0, -1.0), vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(-1.0, 0.0, 0.0)); }
        case 2u: { return mat3x3<f32>(vec3<f32>(-1.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, -1.0), vec3<f32>(0.0, 1.0, 0.0)); }
        case 3u: { return mat3x3<f32>(vec3<f32>(-1.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0, -1.0, 0.0)); }
        case 4u: { return mat3x3<f32>(vec3<f32>(-1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(0.0, 0.0, 1.0)); }
        default: { return mat3x3<f32>(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(0.0, 0.0, -1.0)); }
    }
}

fn texel_dir(f: u32, p: vec2<u32>, size: u32) -> vec3<f32> {
    let uv = (vec2<f32>(p) + 0.5) / f32(size);
    let x = uv.x * 2.0 - 1.0;
    let y = 1.0 - uv.y * 2.0;
    let b = probe_basis(f);
    return normalize(b[2] + b[0] * x + b[1] * y);
}

@compute @workgroup_size(8, 8, 1)
fn fill(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = textureDimensions(src).x;
    if (gid.x >= size || gid.y >= size) {
        return;
    }
    var c = textureLoad(src, vec2<u32>(gid.xy), gid.z, 0);
    if (c.a < 0.5) {
        c = vec4<f32>(lk_sky_sample(texel_dir(gid.z, gid.xy, size)), 1.0);
    }
    textureStore(dst, vec2<u32>(gid.xy), gid.z, vec4<f32>(c.rgb, 1.0));
}

@compute @workgroup_size(8, 8, 1)
fn mip(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = textureDimensions(dst).x;
    if (gid.x >= size || gid.y >= size) {
        return;
    }
    let p = vec2<u32>(gid.xy) * 2u;
    let s = textureLoad(src, p, gid.z, 0) + textureLoad(src, p + vec2<u32>(1u, 0u), gid.z, 0)
        + textureLoad(src, p + vec2<u32>(0u, 1u), gid.z, 0) + textureLoad(src, p + vec2<u32>(1u, 1u), gid.z, 0);
    textureStore(dst, vec2<u32>(gid.xy), gid.z, s * 0.25);
}

var<workgroup> acc: array<array<f32, 30>, 64>;

fn sh_basis(d: vec3<f32>) -> array<f32, 9> {
    return array<f32, 9>(
        0.282095,
        0.488603 * d.y,
        0.488603 * d.z,
        0.488603 * d.x,
        1.092548 * d.x * d.y,
        1.092548 * d.y * d.z,
        0.315392 * (3.0 * d.z * d.z - 1.0),
        1.092548 * d.x * d.z,
        0.546274 * (d.x * d.x - d.y * d.y),
    );
}

@compute @workgroup_size(64, 1, 1)
fn lobe_sum(@builtin(local_invocation_index) li: u32) {
    let size = textureDimensions(src).x;
    let per_face = size * size;
    var a: array<f32, 30>;
    for (var q = 0u; q < 30u; q = q + 1u) {
        a[q] = 0.0;
    }
    let luma = vec3<f32>(0.2126, 0.7152, 0.0722);
    for (var k = li; k < per_face * 6u; k = k + 64u) {
        let f = k / per_face;
        let r = k - f * per_face;
        let p = vec2<u32>(r % size, r / size);
        let d = texel_dir(f, p, size);
        let uv = (vec2<f32>(p) + 0.5) / f32(size) * 2.0 - 1.0;
        // The texel's solid angle.
        let dw = 4.0 / (f32(per_face) * pow(1.0 + dot(uv, uv), 1.5));
        let c = textureLoad(src, p, f, 0).rgb;
        // The cosine lobe about +Y (probeBakeTransfer), of the capture and the sky.
        let w = max(d.y, 0.0) * dw;
        a[0] = a[0] + w * dot(c, luma);
        a[2] = a[2] + w;
        let y = sh_basis(d);
        for (var j = 0u; j < 9u; j = j + 1u) {
            a[3u + j * 3u] = a[3u + j * 3u] + c.r * y[j] * dw;
            a[4u + j * 3u] = a[4u + j * 3u] + c.g * y[j] * dw;
            a[5u + j * 3u] = a[5u + j * 3u] + c.b * y[j] * dw;
        }
    }
    acc[li] = a;
    workgroupBarrier();
    for (var stride = 32u; stride > 0u; stride = stride / 2u) {
        if (li < stride) {
            for (var q = 0u; q < 30u; q = q + 1u) {
                acc[li][q] = acc[li][q] + acc[li + stride][q];
            }
        }
        workgroupBarrier();
    }
    if (li == 0u) {
        lobe[0] = acc[0][0];
        lobe[1] = acc[0][1];
        lobe[2] = acc[0][2];
        lobe[3] = 1.0;
        let band = array<f32, 9>(3.14159265, 2.0943951, 2.0943951, 2.0943951, 0.78539816, 0.78539816, 0.78539816, 0.78539816, 0.78539816);
        for (var j = 0u; j < 9u; j = j + 1u) {
            lobe[4u + j * 4u] = acc[0][3u + j * 3u] * band[j];
            lobe[5u + j * 4u] = acc[0][4u + j * 3u] * band[j];
            lobe[6u + j * 4u] = acc[0][5u + j * 3u] * band[j];
            lobe[7u + j * 4u] = 0.0;
        }
    }
}
