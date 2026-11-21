// The far sun shadow map (far_shadow.rs, `LightingOptions::far_shadows`):
// a cached orthographic depth map a ring round the camera (ring 1 about 2 km
// across, ring 2 out to the far plane), sampled past the cascades by
// look_light.wgsl `lk_shadow` and water.wgsl `w_shadow` through group 1
// binding 12 (`lx_far_shadow_map`, look_ext.wgsl) with the comparison sampler
// at binding 2 (`lk_shadow_cmp`, declared by the shader this is composed
// into). Rows of `look.lx` (look.rs `LX_FAR_SHADOW_VP`, `LX_FAR_SHADOW_PARAMS`):
//
//   1-3  ring 1: the affine rows of world -> the map's clip space (x, y in
//        -1..1 over the ring's box; z 0 at its sunward plane, 1 at the far
//        one, the receivers' depth pull-back folded in)
//   4    ring 1: front layer, normal offset (m), tap offset (uv), the ground
//        distance from the eye where it has faded out (m; 0: not drawn)
//   5-8  ring 2, the same
//   9    x rings drawn (0: none yet), y the box's edge fade (uv), z where the
//        radial fade starts (a share of row 4/8's w), w spare
//
// The inner ring first; each fades with the ground distance from the eye
// (and, a safety net, at its box's edge) to the next, the last to
// unshadowed: a long soft end that moves with the eye, no visible edge.

fn lk_far_on() -> bool {
    return lk_lx_on(LX_BIT_FAR_SHADOWS) && look.lx[9].x > 0.0;
}

// Sun visibility 0..1 from the far rings at a world point (normal offset,
// four bilinear compares a tap offset apart: a 4 x 4 texel tent).
fn lk_far_shadow(world: vec3<f32>, ng: vec3<f32>) -> f32 {
    var vis = 0.0;
    var rest = 1.0;
    let rings = u32(look.lx[9].x + 0.5);
    for (var k = 0u; k < rings; k = k + 1u) {
        let r = 1u + 4u * k;
        let prm = look.lx[r + 3u];
        if (prm.w <= 0.0) {
            continue;
        }
        let p = vec4<f32>(world + ng * prm.y, 1.0);
        let c = vec3<f32>(dot(look.lx[r], p), dot(look.lx[r + 1u], p), dot(look.lx[r + 2u], p));
        let uv = vec2<f32>(c.x * 0.5 + 0.5, 0.5 - c.y * 0.5);
        let e = min(min(uv.x, 1.0 - uv.x), min(uv.y, 1.0 - uv.y));
        let d = length(world.xz - look.cam_pos.xz);
        let radial = 1.0 - lk_smooth(prm.w * look.lx[9].z, prm.w, d);
        let w = lk_smooth(0.0, look.lx[9].y, e) * radial * select(0.0, 1.0, c.z >= 0.0 && c.z <= 1.0);
        if (w > 0.0) {
            let layer = i32(prm.x + 0.5);
            let h = prm.z;
            let v = 0.25 * (textureSampleCompareLevel(lx_far_shadow_map, lk_shadow_cmp, uv + vec2<f32>(-h, -h), layer, c.z)
                + textureSampleCompareLevel(lx_far_shadow_map, lk_shadow_cmp, uv + vec2<f32>(h, -h), layer, c.z)
                + textureSampleCompareLevel(lx_far_shadow_map, lk_shadow_cmp, uv + vec2<f32>(-h, h), layer, c.z)
                + textureSampleCompareLevel(lx_far_shadow_map, lk_shadow_cmp, uv + vec2<f32>(h, h), layer, c.z));
            vis = vis + rest * w * v;
            rest = rest * (1.0 - w);
            if (rest <= 0.0) {
                break;
            }
        }
    }
    return vis + rest;
}
