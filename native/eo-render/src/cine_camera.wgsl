// The camera model's meter (`--cine-camera`, camera track; cine_post.rs lists
// the steps). Composed after look_common (`Look`, `Sky`, `lk_luma`), cine_post.rs's
// `Cine` and `CamState` and its camera constants (`CAM_*`). Three passes over
// the resolved frame, before the post pass reads it:
//
// - `cam_build`: one workgroup a grid column (`CAM_CELL_PX` pixels square), a
//   thread a 4 x 4 pixel block: the block's log luminance (exposed by the hour's
//   exposure) into the meter's histogram (weighted toward the lower centre) and
//   into the column's luminance slices (sum of positions, count);
// - `cam_adapt`: one workgroup: the histogram's log-average over the meter's
//   window, the correction of the hour's exposure (clamped; snapped on a cut,
//   else converging), the frame's range (where `CAM_RANGE_SHARE` of the
//   metered weight lies at each end) and from it the local exposure's
//   compression each side of middle grey (only as much as brings that range
//   into the display's room, `cine.cam3.xy`), and the visible sun's energy
//   over its disc;
// - `cam_blur`: the grid blurred ([1 2 1] in x, y and the slices) and, beside
//   it, the 5 x 5 column blur of each column's mean, into a 3D texture the
//   post pass samples.

@group(0) @binding(0) var<uniform> look: Look;
@group(0) @binding(1) var<uniform> skyu: Sky;
@group(0) @binding(2) var cam_src: texture_2d<f32>;
@group(0) @binding(3) var cam_lin: sampler;
@group(0) @binding(4) var<storage, read_write> cam_hist: array<atomic<u32>>;
@group(0) @binding(5) var<storage, read_write> cam_state: CamState;
// per cell (column-major in z: (row * columns + column) * CAM_GRID_Z + slice): the
// slices' sum of grid positions (x 65535) and their count.
@group(0) @binding(6) var<storage, read_write> cam_cells: array<vec2<u32>>;
// per column: the same over all its slices.
@group(0) @binding(7) var<storage, read_write> cam_cols: array<vec2<u32>>;
@group(0) @binding(8) var cam_grid_out: texture_storage_3d<rgba16float, write>;
@group(0) @binding(9) var<uniform> cine: Cine;

const CAM_POS_SCALE: f32 = 65535.0;
/// a cell's blocks (8 x 8): the grid's counts are written as shares of it.
const CAM_CELL_BLOCKS: f32 = 64.0;

var<workgroup> wg_hist: array<atomic<u32>, CAM_HIST_BINS>;
var<workgroup> wg_sum: array<atomic<u32>, CAM_GRID_Z>;
var<workgroup> wg_cnt: array<atomic<u32>, CAM_GRID_Z>;

/// The meter's weight at `uv`: the lower two thirds and the centre count most
/// (a street, not the sky over it; a subject, not the frame's edge).
fn cam_meter_weight(uv: vec2<f32>) -> f32 {
    let v = 0.35 + 0.65 * smoothstep(0.08, 0.55, uv.y);
    let h = 1.0 - 0.5 * smoothstep(0.2, 0.5, abs(uv.x - 0.5));
    return v * h;
}

fn cam_columns(dims: vec2<u32>) -> vec2<u32> {
    return (dims + vec2<u32>(CAM_CELL_PX - 1u)) / CAM_CELL_PX;
}

@compute @workgroup_size(8, 8)
fn cam_build(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    for (var i = li; i < CAM_HIST_BINS; i = i + 64u) {
        atomicStore(&wg_hist[i], 0u);
    }
    if (li < CAM_GRID_Z) {
        atomicStore(&wg_sum[li], 0u);
        atomicStore(&wg_cnt[li], 0u);
    }
    workgroupBarrier();
    let dims = textureDimensions(cam_src);
    let p0 = wid.xy * CAM_CELL_PX + lid.xy * 4u;
    if (all(p0 < dims)) {
        // the block's mean: four bilinear taps, each the mean of 2 x 2 pixels.
        let inv = 1.0 / vec2<f32>(dims);
        let b = vec2<f32>(p0);
        var c = textureSampleLevel(cam_src, cam_lin, (b + vec2<f32>(1.0, 1.0)) * inv, 0.0).rgb;
        c = c + textureSampleLevel(cam_src, cam_lin, (b + vec2<f32>(3.0, 1.0)) * inv, 0.0).rgb;
        c = c + textureSampleLevel(cam_src, cam_lin, (b + vec2<f32>(1.0, 3.0)) * inv, 0.0).rgb;
        c = c + textureSampleLevel(cam_src, cam_lin, (b + vec2<f32>(3.0, 3.0)) * inv, 0.0).rgb;
        let y = lk_luma(max(c * 0.25, vec3<f32>(0.0))) * look.key.w;
        let l = log2(max(y, 1e-12));
        if (l == l) {
            let uv = (b + vec2<f32>(2.0)) * inv;
            let hb = u32(clamp((l - CAM_HIST_LO) / (CAM_HIST_HI - CAM_HIST_LO) * f32(CAM_HIST_BINS), 0.0, f32(CAM_HIST_BINS) - 1.0));
            atomicAdd(&wg_hist[hb], u32(cam_meter_weight(uv) * 64.0 + 0.5));
            let gz = clamp((l - CAM_GRID_LO) / (CAM_GRID_HI - CAM_GRID_LO), 0.0, 1.0);
            let zb = min(u32(gz * f32(CAM_GRID_Z)), CAM_GRID_Z - 1u);
            atomicAdd(&wg_sum[zb], u32(gz * CAM_POS_SCALE + 0.5));
            atomicAdd(&wg_cnt[zb], 1u);
        }
    }
    workgroupBarrier();
    for (var i = li; i < CAM_HIST_BINS; i = i + 64u) {
        let v = atomicLoad(&wg_hist[i]);
        if (v > 0u) {
            atomicAdd(&cam_hist[i], v);
        }
    }
    let gd = cam_columns(dims);
    let col = wid.y * gd.x + wid.x;
    if (li < CAM_GRID_Z) {
        cam_cells[col * CAM_GRID_Z + li] = vec2<u32>(atomicLoad(&wg_sum[li]), atomicLoad(&wg_cnt[li]));
    }
    if (li == 0u) {
        var t = vec2<u32>(0u);
        for (var z = 0u; z < CAM_GRID_Z; z = z + 1u) {
            t = t + vec2<u32>(atomicLoad(&wg_sum[z]), atomicLoad(&wg_cnt[z]));
        }
        cam_cols[col] = t;
    }
}

var<workgroup> wa_w: array<f32, CAM_HIST_BINS>;
var<workgroup> wa_cum: array<f32, CAM_HIST_BINS>;
var<workgroup> wa_num: array<f32, CAM_HIST_BINS>;
var<workgroup> wa_den: array<f32, CAM_HIST_BINS>;
var<workgroup> wa_sun: array<f32, 64>;
// the frame's range: the log luminance (exposed by the hour's exposure) at the
// `CAM_RANGE_SHARE` ends of the metered weight.
var<workgroup> wa_range: array<f32, 2>;

@compute @workgroup_size(128)
fn cam_adapt(@builtin(local_invocation_index) li: u32) {
    let i = li;
    let v = f32(atomicLoad(&cam_hist[i]));
    wa_w[i] = v;
    wa_cum[i] = v;
    workgroupBarrier();
    // the inclusive prefix sum (Hillis and Steele).
    for (var off = 1u; off < CAM_HIST_BINS; off = off * 2u) {
        var t = 0.0;
        if (i >= off) {
            t = wa_cum[i - off];
        }
        workgroupBarrier();
        wa_cum[i] = wa_cum[i] + t;
        workgroupBarrier();
    }
    let total = wa_cum[CAM_HIST_BINS - 1u];
    // this bin's part of the window [lo, hi] of the metered weight.
    let lo = cine.cam1.x * total;
    let hi = cine.cam1.y * total;
    let a = wa_cum[i] - v;
    let inw = max(min(wa_cum[i], hi) - max(a, lo), 0.0);
    let centre = CAM_HIST_LO + (f32(i) + 0.5) / f32(CAM_HIST_BINS) * (CAM_HIST_HI - CAM_HIST_LO);
    wa_num[i] = inw * centre;
    wa_den[i] = inw;
    // the range's ends: the bin the cumulative weight crosses each share in (interpolated in it).
    if (i == 0u) {
        wa_range[0] = CAM_HIST_LO;
        wa_range[1] = CAM_HIST_HI;
    }
    workgroupBarrier();
    let bw = (CAM_HIST_HI - CAM_HIST_LO) / f32(CAM_HIST_BINS);
    let edge = CAM_HIST_LO + f32(i) * bw;
    for (var e = 0u; e < 2u; e = e + 1u) {
        let q = select(CAM_RANGE_SHARE.x, CAM_RANGE_SHARE.y, e == 1u) * total;
        if (v > 0.0 && a < q && wa_cum[i] >= q) {
            wa_range[e] = edge + bw * clamp((q - a) / v, 0.0, 1.0);
        }
    }
    // the sun's disc: 64 taps over 1.2 x its radius (the first 64 threads).
    if (i < 64u) {
        var e = 0.0;
        let sc = skyu.vp * vec4<f32>(skyu.atmo.xyz, 0.0);
        if (sc.w > 1e-6) {
            let suv = vec2<f32>(sc.x / sc.w * 0.5 + 0.5, 0.5 - sc.y / sc.w * 0.5);
            let dims = vec2<f32>(textureDimensions(cam_src));
            // the disc's radius (0.2667 deg) in pixels of the frame's height.
            let r = 1.2 * 0.0046548 / max(skyu.cluster.z, 1e-4) * 0.5 * dims.y;
            let ga = f32(i) * 2.3999632;
            let o = vec2<f32>(cos(ga), sin(ga)) * r * sqrt((f32(i) + 0.5) / 64.0);
            let q = suv * dims + o;
            if (all(q >= vec2<f32>(0.0)) && all(q < dims)) {
                e = lk_luma(max(textureSampleLevel(cam_src, cam_lin, q / dims, 0.0).rgb, vec3<f32>(0.0))) * look.key.w;
            }
        }
        wa_sun[i] = select(0.0, e, e == e);
    }
    workgroupBarrier();
    for (var s = CAM_HIST_BINS / 2u; s > 0u; s = s / 2u) {
        if (i < s) {
            wa_num[i] = wa_num[i] + wa_num[i + s];
            wa_den[i] = wa_den[i] + wa_den[i + s];
            if (i < 32u && s <= 32u) {
                wa_sun[i] = wa_sun[i] + wa_sun[i + s];
            }
        }
        workgroupBarrier();
    }
    if (i == 0u) {
        let ev_hour = log2(max(look.key.w, 1e-12));
        var st = cam_state;
        var corr = 0.0;
        if (wa_den[0] > 0.5) {
            let mean = wa_num[0] / wa_den[0];
            st.mean = mean;
            corr = clamp(log2(0.18) + cine.cam1.z - mean, -cine.cam1.w, cine.cam0.z);
        }
        let target_ev = ev_hour + corr;
        // a cut (no history), the first frame, or a far jump snaps; else converge.
        let cut = skyu.taa.y < 0.5 || st.init < 0.5 || !(abs(st.ev - target_ev) < 12.0);
        var ev = target_ev;
        if (!cut) {
            let dt = clamp(skyu.taa.w, 1.0 / 240.0, 0.25);
            ev = st.ev + (target_ev - st.ev) * (1.0 - exp(-dt * CAM_ADAPT_RATE));
        }
        let rel = clamp(ev - ev_hour, -cine.cam1.w, cine.cam0.z);
        st.ev = ev_hour + rel;
        st.gain = exp2(rel);
        // the local exposure's compression: the range's ends about middle grey once the
        // meter has exposed the frame, brought inside the display's room (`cam3.xy`), each
        // side only as far as it needs and never past `cam0.xy`; converging as the meter does.
        let mid = log2(0.18);
        let lo = wa_range[0] + rel - mid;
        let hi = wa_range[1] + rel - mid;
        var ks = 1.0;
        var kh = 1.0;
        if (total > 0.5) {
            ks = clamp(cine.cam3.x / max(-lo, 1e-3), cine.cam0.x, 1.0);
            kh = clamp(cine.cam3.y / max(hi, 1e-3), cine.cam0.y, 1.0);
        }
        if (cut || !(st.k_shadow > 0.0) || !(st.k_high > 0.0)) {
            st.k_shadow = ks;
            st.k_high = kh;
        } else {
            let dt = clamp(skyu.taa.w, 1.0 / 240.0, 0.25);
            let f = 1.0 - exp(-dt * CAM_ADAPT_RATE);
            st.k_shadow = st.k_shadow + (ks - st.k_shadow) * f;
            st.k_high = st.k_high + (kh - st.k_high) * f;
        }
        st.lo = lo;
        st.sun_e = wa_sun[0] / 64.0 * st.gain;
        st.init = 1.0;
        cam_state = st;
    }
}

fn cam_cell(c: vec3<i32>, gd: vec3<i32>) -> vec2<f32> {
    if (any(c < vec3<i32>(0)) || any(c >= gd)) {
        return vec2<f32>(0.0);
    }
    let v = cam_cells[u32((c.y * gd.x + c.x) * gd.z + c.z)];
    return vec2<f32>(f32(v.x) / CAM_POS_SCALE, f32(v.y)) / CAM_CELL_BLOCKS;
}

fn cam_col(c: vec2<i32>, gd: vec2<i32>) -> vec2<f32> {
    let q = clamp(c, vec2<i32>(0), gd - 1);
    let v = cam_cols[u32(q.y * gd.x + q.x)];
    return vec2<f32>(f32(v.x) / CAM_POS_SCALE, f32(v.y)) / CAM_CELL_BLOCKS;
}

@compute @workgroup_size(4, 4, 4)
fn cam_blur(@builtin(global_invocation_id) gid: vec3<u32>) {
    let gd = vec3<i32>(textureDimensions(cam_grid_out));
    let p = vec3<i32>(gid);
    if (any(p >= gd)) {
        return;
    }
    // the grid: [1 2 1] in x, y and the slices (homogeneous: sums and counts together).
    var b = vec2<f32>(0.0);
    for (var k = 0; k < 27; k = k + 1) {
        let o = vec3<i32>(k % 3 - 1, (k / 3) % 3 - 1, k / 9 - 1);
        let w = (2.0 - f32(abs(o.x))) * (2.0 - f32(abs(o.y))) * (2.0 - f32(abs(o.z))) / 64.0;
        b = b + cam_cell(p + o, gd) * w;
    }
    // the columns: [1 4 6 4 1] in x and y (about 1.3 cells), clamped at the frame's edge.
    var c = vec2<f32>(0.0);
    for (var k = 0; k < 25; k = k + 1) {
        let o = vec2<i32>(k % 5 - 2, k / 5 - 2);
        let wx = select(select(1.0, 4.0, abs(o.x) == 1), 6.0, o.x == 0);
        let wy = select(select(1.0, 4.0, abs(o.y) == 1), 6.0, o.y == 0);
        c = c + cam_col(p.xy + o.xy, gd.xy) * (wx * wy / 256.0);
    }
    textureStore(cam_grid_out, p, vec4<f32>(b, c));
}
