// the conservative hierarchical-Z test of a world-space box.
//
// Shared by the static cull's late pass (cull.wgsl) and any other cull that
// runs after the pyramid is built (moving agents: bind
// `OcclusionGpu::late_bgl` / the pyramid's bind group, declare its binding 0
// as a `texture_2d<f32>` global named `hzb` and binding 1 as the `HzbInfo`
// uniform below, and call `hzb_occluded(min, max, view_proj, info.screen.xy)`).
//
// struct HzbInfo { screen: vec4<f32> }  // x, y: depth target size in pixels
//
// Reverse Z: near 1, far 0. Pyramid level L holds, per texel, the FARTHEST
// (smallest) depth of the 2^(L+1) x 2^(L+1) block of depth pixels it covers
// (level 0 is half resolution; texels past the screen edge hold 1, "no
// pixel", which a minimum ignores). A box is occluded only when its NEAREST
// point (the largest depth of its eight corners) lies behind the farthest
// depth already drawn over every pixel its screen rectangle can touch, at a
// level where that rectangle spans at most 2 x 2 texels. A box that reaches
// the eye plane or the near plane is never occluded.

// Relative depth margin (reverse Z depth ~ 1 / distance): 1 cm at 1 km.
const HZB_MARGIN: f32 = 1e-5;

fn hzb_occluded(mn: vec3<f32>, mx: vec3<f32>, view_proj: mat4x4<f32>, screen: vec2<f32>) -> bool {
    var lo = vec2<f32>(1e30, 1e30);
    var hi = vec2<f32>(-1e30, -1e30);
    var zmax = 0.0;
    for (var k = 0u; k < 8u; k = k + 1u) {
        let p = vec3<f32>(
            select(mn.x, mx.x, (k & 1u) != 0u),
            select(mn.y, mx.y, (k & 2u) != 0u),
            select(mn.z, mx.z, (k & 4u) != 0u),
        );
        let c = view_proj * vec4<f32>(p, 1.0);
        if (c.w <= 1e-4) {
            return false;
        }
        let ndc = c.xyz / c.w;
        lo = min(lo, ndc.xy);
        hi = max(hi, ndc.xy);
        zmax = max(zmax, ndc.z);
    }
    if (zmax >= 1.0) {
        return false;
    }
    // Pixels (x right, y down), widened by half a pixel each side for
    // rasterisation snapping; clamped to the target.
    let lim = screen - vec2<f32>(1.0, 1.0);
    let p0 = clamp(vec2<f32>((lo.x * 0.5 + 0.5) * screen.x, (0.5 - hi.y * 0.5) * screen.y) - 0.5, vec2<f32>(0.0), lim);
    let p1 = clamp(vec2<f32>((hi.x * 0.5 + 0.5) * screen.x, (0.5 - lo.y * 0.5) * screen.y) + 0.5, vec2<f32>(0.0), lim);
    let a = vec2<u32>(floor(p0));
    let b = vec2<u32>(floor(p1));
    let size = max(b.x - a.x, b.y - a.y) + 1u;
    // A level-L texel covers 2^(L+1) pixels: `size` pixels touch at most two.
    var level = 0u;
    if (size > 2u) {
        level = firstLeadingBit(size - 1u);
    }
    level = min(level, textureNumLevels(hzb) - 1u);
    let dims = textureDimensions(hzb, level) - vec2<u32>(1u, 1u);
    let sh = level + 1u;
    let t0 = min(a >> vec2<u32>(sh, sh), dims);
    let t1 = min(b >> vec2<u32>(sh, sh), dims);
    let d = min(
        min(textureLoad(hzb, t0, level).r, textureLoad(hzb, vec2<u32>(t1.x, t0.y), level).r),
        min(textureLoad(hzb, vec2<u32>(t0.x, t1.y), level).r, textureLoad(hzb, t1, level).r),
    );
    return zmax < d * (1.0 - HZB_MARGIN);
}
