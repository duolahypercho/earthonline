// the lamp and headlight shadow maps' lookup (lighting, `--light-shadows`,
// `--headlight-shadows`; light_shadow.rs), composed into the city shader
// (after look_ext.wgsl, whose bindings 17 and 18 it reads with the look's
// comparison sampler at binding 2) and into the lit fog's passes (which
// declare the same three names on their own group). Constants from
// light_shadow.rs `use_wgsl` (LS_SHADER false compiles every caller's test
// out: the far layer's variant).
//
// The table (binding 18): row 0 the atlas grid (columns, rows, tile texels,
// tiles), then five rows a tile: three rows of world -> the light's view
// (x, y across, z along its axis), the projection (ndc = view.xy / view.z *
// p.xy + p.zw) and (near, far, valid, a texel's width per metre of depth).
// A tile's map holds the nearest caster's depth along the axis, linear
// between near and far (0..1).

/// A light's visibility from a surface point `p` (normal `n`; zero for a
/// point of the fog): 0 shadowed .. 1 lit, by the map of atlas tile `tile`.
/// Outside the map's frustum, nearer than its near plane or past its far
/// plane: lit (the frustum covers the light's reach; its rim fades to lit).
fn ls_light_visibility(tile: u32, p: vec3<f32>, n: vec3<f32>) -> f32 {
    let hdr = lx_light_shadow_table[0];
    if (f32(tile) >= hdr.w) {
        return 1.0;
    }
    let b = 1u + tile * 5u;
    let r0 = lx_light_shadow_table[b];
    let r1 = lx_light_shadow_table[b + 1u];
    let r2 = lx_light_shadow_table[b + 2u];
    let pp = lx_light_shadow_table[b + 3u];
    let dd = lx_light_shadow_table[b + 4u];
    if (dd.z < 0.5) {
        return 1.0;
    }
    // the receiver moved off its surface by a share of a texel's width at its depth (no acne
    // where the light grazes it), and its depth pulled toward the light by a constant.
    let z0 = dot(r2.xyz, p) + r2.w;
    let q = p + n * (LS_NORMAL_OFFSET_TEXELS * dd.w * max(z0, dd.x));
    let v = vec3<f32>(dot(r0.xyz, q) + r0.w, dot(r1.xyz, q) + r1.w, dot(r2.xyz, q) + r2.w);
    if (v.z <= dd.x) {
        return 1.0;
    }
    let ndc = v.xy / v.z * pp.xy + pp.zw;
    let edge = max(abs(ndc.x), abs(ndc.y));
    let lin = (v.z - dd.x) / (dd.y - dd.x);
    if (edge >= 1.0 || lin >= 1.0) {
        return 1.0;
    }
    let grid = hdr.xy;
    let cell = vec2<f32>(f32(tile % u32(hdr.x)), f32(tile / u32(hdr.x)));
    let inset = 1.5 / hdr.z;
    let t = clamp(vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5), vec2<f32>(inset), vec2<f32>(1.0 - inset));
    let uv = (cell + t) / grid;
    let reference = lin - LS_DEPTH_BIAS_M / (dd.y - dd.x);
    // four bilinear compares half a texel and a quarter apart (a 3 x 3 texel tent).
    let h = vec2<f32>(LS_PCF_TEXELS) / (grid * hdr.z);
    let s = textureSampleCompareLevel(lx_light_shadow_atlas, lk_shadow_cmp, uv + vec2<f32>(-h.x, -h.y), 0, reference)
        + textureSampleCompareLevel(lx_light_shadow_atlas, lk_shadow_cmp, uv + vec2<f32>(h.x, -h.y), 0, reference)
        + textureSampleCompareLevel(lx_light_shadow_atlas, lk_shadow_cmp, uv + vec2<f32>(-h.x, h.y), 0, reference)
        + textureSampleCompareLevel(lx_light_shadow_atlas, lk_shadow_cmp, uv + vec2<f32>(h.x, h.y), 0, reference);
    return mix(s * 0.25, 1.0, smoothstep(LS_RIM_FADE, 1.0, edge));
}

/// (lighting, `--light-shadows`) A street lamp of the JS role (its colour's
/// chromaticity is the role's 0xffc46a) in SF's 3000 K LED colour at the
/// same luminance, while the option is on; anything else as given (sky.rs
/// `led_params`; the far lights, hlod_lights.wgsl).
fn ls_led(c: vec3<f32>) -> vec3<f32> {
    if (!lk_lx_on(LX_BIT_LIGHT_SHADOWS)) {
        return c;
    }
    let k = c / max(max(c.r, max(c.g, c.b)), 1e-6);
    if (any(abs(k - LS_LED_KEY) > vec3<f32>(LS_LED_KEY_TOLERANCE))) {
        return c;
    }
    return LS_LED_RGB * dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}
