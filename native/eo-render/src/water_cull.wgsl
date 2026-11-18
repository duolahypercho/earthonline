// one invocation per grid patch (32 x 32 patches of 16 x 16 quads).
// A patch is drawn when its box (the displaced surface's reach) is inside the
// frustum and the terrain grid, not wholly over land (the max-reduction
// pyramid of the shore field below the land mask), and, with occlusion on,
// not hidden behind this frame's early depth (`hzb_occluded`, the city's own
// test: the water is a receiver, never an occluder, so it is tested against
// the pyramid and never drawn into it). Survivors are listed (`wdraws[slot]` =
// the patch) for ONE instanced indirect draw whose instance count the cull
// counts (`wcount[9]`): a single draw call on every adapter, with or without
// multi-draw-indirect-count (the vertex shader reads its patch from the list).

@group(0) @binding(0) var<uniform> wu: WaterU;
@group(0) @binding(1) var<storage, read_write> wdraws: array<u32>;
// [0] patches drawn, [1] in the frustum, [2] culled as land, [3] culled as
// occluded, [4] the nearest drawn patch's distance (m, the gate), [5..8) pad,
// [8..13) the draw's DrawIndexedIndirect (index count, instance count = the
// patches drawn, first index, base vertex, first instance), [13..16) pad
@group(0) @binding(2) var<storage, read_write> wcount: array<atomic<u32>, 16>;
@group(0) @binding(3) var land_max: texture_2d<f32>;

// The largest signed shore distance over a world rectangle (field texels).
fn land_max_over(lo: vec2<f32>, hi: vec2<f32>) -> f32 {
    let dims0 = vec2<f32>(textureDimensions(land_max, 0));
    let t0 = (lo - wu.field.xy) / wu.field.zw * (dims0 - 1.0);
    let t1 = (hi - wu.field.xy) / wu.field.zw * (dims0 - 1.0);
    let a = clamp(floor(t0), vec2<f32>(0.0), dims0 - 1.0);
    let b = clamp(ceil(t1), vec2<f32>(0.0), dims0 - 1.0);
    let span = max(b.x - a.x, b.y - a.y);
    var level = 0u;
    if (span > 1.0) {
        level = u32(ceil(log2(span)));
    }
    level = min(level, textureNumLevels(land_max) - 1u);
    let dims = textureDimensions(land_max, level) - vec2<u32>(1u, 1u);
    let p0 = min(vec2<u32>(a) >> vec2<u32>(level, level), dims);
    let p1 = min(vec2<u32>(b) >> vec2<u32>(level, level), dims);
    return max(
        max(textureLoad(land_max, p0, level).r, textureLoad(land_max, vec2<u32>(p1.x, p0.y), level).r),
        max(textureLoad(land_max, vec2<u32>(p0.x, p1.y), level).r, textureLoad(land_max, p1, level).r),
    );
}

fn cull_patch(id: u32) {
    if (id >= WG_PATCHES * WG_PATCHES) {
        return;
    }
    let pi = id % WG_PATCHES;
    let pj = id / WG_PATCHES;
    let r = wu.fwd.w;
    let q = f32(WG_PATCH_QUADS);
    // the rim's outer line is thrown out as the vertex shader throws it (the skirt)
    let sk = mix(12.0, 1.0, wg_smooth(60.0, 200.0, wu.cam.y - wu.grid.z));
    let ex0 = select(1.0, sk, pi == 0u);
    let ex1 = select(1.0, sk, pi == WG_PATCHES - 1u);
    let ez0 = select(1.0, sk, pj == 0u);
    let ez1 = select(1.0, sk, pj == WG_PATCHES - 1u);
    let x0 = wu.grid.x + wg_axis(f32(pi) * q, r) * ex0;
    let x1 = wu.grid.x + wg_axis(f32(pi + 1u) * q, r) * ex1;
    let z0 = wu.grid.y + wg_axis(f32(pj) * q, r) * ez0;
    let z1 = wu.grid.y + wg_axis(f32(pj + 1u) * q, r) * ez1;
    // Inside the water's reach (the terrain grid widened over open water, `wg_reach`).
    let reach = wg_reach();
    let lo = vec2<f32>(max(x0, reach.x), max(z0, reach.y));
    let hi = vec2<f32>(min(x1, reach.z), min(z1, reach.w));
    if (lo.x >= hi.x || lo.y >= hi.y) {
        return;
    }
    // a patch on the grid's rim carries the skirt to the horizon; its
    // land and occlusion tests (of the rim's own box) stand down for it; its
    // frustum test takes the thrown-out box (a culled rim patch left a wedge).
    let rim = pi == 0u || pj == 0u || pi == WG_PATCHES - 1u || pj == WG_PATCHES - 1u;
    // The displaced surface moves horizontally too (choppiness): widen.
    let pad = wu.grid.w;
    let mn = vec3<f32>(lo.x - pad, wu.grid.z - wu.grid.w, lo.y - pad);
    let mx = vec3<f32>(hi.x + pad, wu.grid.z + wu.grid.w, hi.y + pad);
    for (var p = 0u; p < 6u; p = p + 1u) {
        let pl = wu.planes[p];
        let v = select(mn, mx, pl.xyz >= vec3<f32>(0.0));
        if (dot(pl.xyz, v) + pl.w < 0.0) {
            return;
        }
    }
    atomicAdd(&wcount[1], 1u);
    if (!rim && land_max_over(lo - pad, hi + pad) < min(wu.flags.z, wu.probe.w)) {
        atomicAdd(&wcount[2], 1u);
        return;
    }
    if (!rim && patch_occluded(mn, mx)) {
        atomicAdd(&wcount[3], 1u);
        return;
    }
    let near = distance(clamp(wu.cam.xyz, mn, mx), wu.cam.xyz);
    atomicMin(&wcount[4], u32(min(near, 1e9)));
    atomicAdd(&wcount[0], 1u);
    let slot = atomicAdd(&wcount[9], 1u);
    wdraws[slot] = id;
}

@compute @workgroup_size(64)
fn water_cull(@builtin(global_invocation_id) gid: vec3<u32>) {
    cull_patch(gid.x);
}
