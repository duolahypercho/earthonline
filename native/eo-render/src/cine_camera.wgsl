// The camera model's meter (`--cine-camera`, camera track; cine_post.rs lists
// the steps). Composed after look_common (`Look`, `Sky`, `lk_luma`), cine_post.rs's
// `Cine` and `CamState` and its camera constants (`CAM_*`). Three passes over
// the resolved frame, before the post pass reads it:
//
// - `cam_build`: one workgroup a grid column (`CAM_CELL_PX` pixels square), a
//   thread a 4 x 4 pixel block: the block's log luminance (exposed by the hour's
//   exposure) into the meter's histogram (weighted toward the lower centre), into
//   the frame's plain histogram (every block alike) and into the column's
//   luminance slices (sum of positions, count);
// - `cam_adapt`: one workgroup: the histogram's log-average over the meter's
//   window, the correction of the hour's exposure (clamped; snapped on a cut,
//   else converging), the frame's range (its dark end where `CAM_RANGE_SHARE.x`
//   of the metered weight lies, its bright end where `CAM_RANGE_SHARE.y` of the
//   whole frame does, its brightest sources at `CAM_TOP_SHARE`) and from it the
//   local exposure's compression under middle grey and over the highlights'
//   knee (only as much as brings that range into the display's room,
//   `cine.cam3.xy`), or the highlights' expansion when the frame's brightest
//   sources fall short of `cine.cam4.y` (at night: the lamps' cores burn), the
//   visible sun's energy over its disc, and the lens's focus (the mean dioptre
//   of the frame's middle, from the distances the resolve leaves in its alpha);
// - `cam_blur`: the grid blurred ([1 2 1] in x, y and the slices) and, beside
//   it, the 5 x 5 column blur of each column's mean, into a 3D texture the
//   post pass samples.
//
// (`--lens-flare`, `SF_CAM_FLARE`) `cam_build` ends with the cell's point
// source: its brightest block over `CAM_FLARE_DETECT`, refined and tested by
// `cam_find_source`, appended to the list the post pass draws spikes from.
// (`--white-balance`,
// `SF_CAM_WB`) `cam_build` sums the chromaticity of the frame's surfaces (no
// sky; within `CAM_WB_EV` of middle grey at the last frame's exposure; under
// `CAM_WB_CHROMA`) after the histograms, and `cam_adapt` turns it into the
// balance's gains (`cam_state.wb_*`): the colour temperature's axis corrected
// by `cam6.x` for a warm light and `cam6.y` for a cool one, the green-magenta
// axis by `cam6.z`.

@group(0) @binding(0) var<uniform> look: Look;
@group(0) @binding(1) var<uniform> skyu: Sky;
@group(0) @binding(2) var cam_src: texture_2d<f32>;
@group(0) @binding(3) var cam_lin: sampler;
// the meter's histogram (weighted, bins 0 .. CAM_HIST_BINS) and the frame's plain one (after it).
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

var<workgroup> wg_hist: array<atomic<u32>, 2u * CAM_HIST_BINS>;
var<workgroup> wg_sum: array<atomic<u32>, CAM_GRID_Z>;
var<workgroup> wg_cnt: array<atomic<u32>, CAM_GRID_Z>;
// (`--white-balance`) the workgroup's chromaticity sums (red, green, blue x weight) and weight.
var<workgroup> wg_wb: array<atomic<u32>, 4>;
/// (`--white-balance`) the estimate's fixed point (a block of full weight adds this much).
const CAM_WB_SCALE: f32 = 4096.0;
// (`--lens-flare`) the cell's brightest block (its exposed luminance's bits) and the
// first thread holding it.
var<workgroup> wg_star: atomic<u32>;
var<workgroup> wg_star_li: atomic<u32>;

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
    for (var i = li; i < 2u * CAM_HIST_BINS; i = i + 64u) {
        atomicStore(&wg_hist[i], 0u);
    }
    if (li < CAM_GRID_Z) {
        atomicStore(&wg_sum[li], 0u);
        atomicStore(&wg_cnt[li], 0u);
    }
    if (SF_CAM_WB && li < 4u) {
        atomicStore(&wg_wb[li], 0u);
    }
    if (SF_CAM_FLARE && li == 0u) {
        atomicStore(&wg_star, 0u);
        atomicStore(&wg_star_li, 0xffffffffu);
    }
    workgroupBarrier();
    let dims = textureDimensions(cam_src);
    // (`--lens-flare`) the block's mean exposed luminance at the last frame's exposure.
    var blk = 0.0;
    let p0 = wid.xy * CAM_CELL_PX + lid.xy * 4u;
    if (all(p0 < dims)) {
        // the block's mean: four bilinear taps, each the mean of 2 x 2 pixels.
        let inv = 1.0 / vec2<f32>(dims);
        let b = vec2<f32>(p0);
        let t0 = textureSampleLevel(cam_src, cam_lin, (b + vec2<f32>(1.0, 1.0)) * inv, 0.0);
        let t1 = textureSampleLevel(cam_src, cam_lin, (b + vec2<f32>(3.0, 1.0)) * inv, 0.0);
        let t2 = textureSampleLevel(cam_src, cam_lin, (b + vec2<f32>(1.0, 3.0)) * inv, 0.0);
        let t3 = textureSampleLevel(cam_src, cam_lin, (b + vec2<f32>(3.0, 3.0)) * inv, 0.0);
        let c = t0.rgb + t1.rgb + t2.rgb + t3.rgb;
        let y = lk_luma(max(c * 0.25, vec3<f32>(0.0))) * look.key.w;
        blk = select(0.0, y * cam_state.gain, y == y);
        let l = log2(max(y, 1e-12));
        if (l == l) {
            let uv = (b + vec2<f32>(2.0)) * inv;
            let hb = u32(clamp((l - CAM_HIST_LO) / (CAM_HIST_HI - CAM_HIST_LO) * f32(CAM_HIST_BINS), 0.0, f32(CAM_HIST_BINS) - 1.0));
            // (`--white-balance`) the block's sky: its mean distance from the resolve's alpha
            // (2 + metres, the sky 65504: a quarter of it is past 16000 m).
            let am = 0.25 * (t0.a + t1.a + t2.a + t3.a);
            let sky = am > 16000.0;
            atomicAdd(&wg_hist[hb], u32(cam_meter_weight(uv) * 64.0 + 0.5));
            if (SF_CAM_WB && !sky && am >= 2.0) {
                // a surface lit near middle grey at the last frame's exposure, not a strong colour.
                let cs = max(c * 0.25, vec3<f32>(0.0));
                let sum = cs.r + cs.g + cs.b;
                let mx = max(cs.r, max(cs.g, cs.b));
                let chroma = (mx - min(cs.r, min(cs.g, cs.b))) / max(mx, 1e-12);
                let u = l + log2(max(cam_state.gain, 1e-6)) - log2(0.18);
                let w = cam_meter_weight(uv) * smoothstep(CAM_WB_EV.x - 1.0, CAM_WB_EV.x, u) * (1.0 - smoothstep(CAM_WB_EV.y, CAM_WB_EV.y + 1.0, u))
                    * (1.0 - smoothstep(CAM_WB_CHROMA.x, CAM_WB_CHROMA.y, chroma));
                if (w > 0.0 && sum > 1e-12) {
                    let q = cs / sum * (w * CAM_WB_SCALE);
                    atomicAdd(&wg_wb[0], u32(q.r + 0.5));
                    atomicAdd(&wg_wb[1], u32(q.g + 0.5));
                    atomicAdd(&wg_wb[2], u32(q.b + 0.5));
                    atomicAdd(&wg_wb[3], u32(w * CAM_WB_SCALE + 0.5));
                }
            }
            atomicAdd(&wg_hist[CAM_HIST_BINS + hb], 64u);
            let gz = clamp((l - CAM_GRID_LO) / (CAM_GRID_HI - CAM_GRID_LO), 0.0, 1.0);
            let zb = min(u32(gz * f32(CAM_GRID_Z)), CAM_GRID_Z - 1u);
            atomicAdd(&wg_sum[zb], u32(gz * CAM_POS_SCALE + 0.5));
            atomicAdd(&wg_cnt[zb], 1u);
        }
    }
    workgroupBarrier();
    for (var i = li; i < 2u * CAM_HIST_BINS; i = i + 64u) {
        let v = atomicLoad(&wg_hist[i]);
        if (v > 0u) {
            atomicAdd(&cam_hist[i], v);
        }
    }
    if (SF_CAM_WB && li < 4u) {
        let v = atomicLoad(&wg_wb[li]);
        if (v > 0u) {
            atomicAdd(&cam_hist[2u * CAM_HIST_BINS + li], v);
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
    // (`--lens-flare`) the cell's point source: its brightest block over the threshold, the
    // first thread holding it refines and tests it.
    if (SF_CAM_FLARE) {
        if (blk > CAM_FLARE_DETECT) {
            atomicMax(&wg_star, bitcast<u32>(blk));
        }
        workgroupBarrier();
        let top = atomicLoad(&wg_star);
        if (top != 0u && bitcast<u32>(blk) == top) {
            atomicMin(&wg_star_li, li);
        }
        workgroupBarrier();
        if (top != 0u && atomicLoad(&wg_star_li) == li) {
            cam_find_source(vec2<i32>(p0), vec2<i32>(dims));
        }
    }
}

/// (`--lens-flare`) a cell's point source from its brightest block `p0`: the block's
/// brightest pixel, kept only if no pixel within 5 px is brighter (an equal one only
/// in a cell further along: one cell keeps a flat core), it stands `CAM_FLARE_POINT`
/// over the mean of a ring `CAM_FLARE_RING_PX` round it (a lamp or its globe, not a
/// lit window) and it is not on the sun's disc (its own glare draws the sun's); its
/// flux (the 11 x 11 pixels' exposed luminance over `CAM_FLARE_FLUX_FLOOR`,
/// logarithmic past `CAM_FLARE_FLUX_CAP`) sets its spikes' intensity and the length
/// they are drawn to; appended with its colour.
fn cam_find_source(p0: vec2<i32>, dims: vec2<i32>) {
    let e = look.key.w * cam_state.gain;
    let di = dims - vec2<i32>(1);
    var best = -1.0;
    var bp = p0;
    for (var k = 0; k < 16; k = k + 1) {
        let q = p0 + vec2<i32>(k % 4, k / 4);
        if (any(q > di)) {
            continue;
        }
        let l = lk_luma(max(textureLoad(cam_src, q, 0).rgb, vec3<f32>(0.0))) * e;
        if (l > best) {
            best = l;
            bp = q;
        }
    }
    if (!(best > 0.0)) {
        return;
    }
    let px = f32(dims.y) / 1080.0;
    // not on the sun's disc (2.5 x its radius).
    let sc = skyu.vp * vec4<f32>(skyu.atmo.xyz, 0.0);
    if (sc.w > 1e-6) {
        let sp = vec2<f32>(sc.x / sc.w * 0.5 + 0.5, 0.5 - sc.y / sc.w * 0.5) * vec2<f32>(dims);
        let r = 2.5 * 0.0046548 / max(skyu.cluster.z, 1e-4) * 0.5 * f32(dims.y);
        if (length(vec2<f32>(bp) + 0.5 - sp) < r) {
            return;
        }
    }
    // a point: its ring's mean well under it.
    var ring = 0.0;
    for (var k = 0; k < 16; k = k + 1) {
        let a = f32(k) * 0.3926991;
        let q = clamp(bp + vec2<i32>(round(vec2<f32>(cos(a), sin(a)) * CAM_FLARE_RING_PX * px)), vec2<i32>(0), di);
        ring = ring + lk_luma(max(textureLoad(cam_src, q, 0).rgb, vec3<f32>(0.0)));
    }
    if (!(best >= CAM_FLARE_POINT * ring * e / 16.0)) {
        return;
    }
    let cell = vec2<i32>(i32(CAM_CELL_PX));
    let bc = bp / cell;
    let bci = bc.y * dims.x + bc.x;
    var flux = 0.0;
    var col = vec3<f32>(0.0);
    for (var k = 0; k < 121; k = k + 1) {
        let q = bp + vec2<i32>(k % 11 - 5, k / 11 - 5);
        if (any(q < vec2<i32>(0)) || any(q > di)) {
            continue;
        }
        let c = max(textureLoad(cam_src, q, 0).rgb, vec3<f32>(0.0)) * e;
        let l = lk_luma(c);
        if (!(l == l)) {
            continue;
        }
        let qc = q / cell;
        if (l > best || (l == best && qc.y * dims.x + qc.x < bci)) {
            return;
        }
        let over = max(l - CAM_FLARE_FLUX_FLOOR, 0.0);
        flux = flux + over;
        col = col + c * (over / max(l, 1e-6));
    }
    if (!(flux > 0.0)) {
        return;
    }
    let fc = select(flux, CAM_FLARE_FLUX_CAP * (1.0 + log(flux / CAM_FLARE_FLUX_CAP)), flux > CAM_FLARE_FLUX_CAP);
    let a = CAM_FLARE_SPIKE.x * fc;
    if (a < 2.0 * CAM_FLARE_SPIKE.w) {
        return;
    }
    // drawn out to where the longest (red) spike falls under `CAM_FLARE_SPIKE.w`.
    let len = min(CAM_FLARE_SPIKE.z * (pow(a / CAM_FLARE_SPIKE.w, 1.0 / CAM_FLARE_SPIKE.y) - 1.0) * CAM_FLARE_DISPERSION.x, CAM_FLARE_SPIKE_MAX_PX) * px;
    cam_star_push(vec4<f32>(vec2<f32>(bp) + 0.5, a, len), vec4<f32>(col / max(lk_luma(col), 1e-6), 0.0));
}

var<workgroup> wa_cum: array<f32, CAM_HIST_BINS>;
// the plain histogram's prefix sum.
var<workgroup> wa_cum2: array<f32, CAM_HIST_BINS>;
var<workgroup> wa_num: array<f32, CAM_HIST_BINS>;
var<workgroup> wa_den: array<f32, CAM_HIST_BINS>;
var<workgroup> wa_sun: array<f32, 64>;
// the focus's taps: (dioptres, 1 if the frame carried a distance there).
var<workgroup> wa_af: array<vec2<f32>, 64>;
// the frame's range: the log luminance (exposed by the hour's exposure) at its dark
// end (`CAM_RANGE_SHARE.x` of the metered weight), its bright end (`CAM_RANGE_SHARE.y`
// of the whole frame) and its brightest sources (`CAM_TOP_SHARE` of the whole frame).
var<workgroup> wa_range: array<f32, 3>;

@compute @workgroup_size(128)
fn cam_adapt(@builtin(local_invocation_index) li: u32) {
    let i = li;
    let v = f32(atomicLoad(&cam_hist[i]));
    let v2 = f32(atomicLoad(&cam_hist[CAM_HIST_BINS + i]));
    wa_cum[i] = v;
    wa_cum2[i] = v2;
    workgroupBarrier();
    // the inclusive prefix sums (Hillis and Steele).
    for (var off = 1u; off < CAM_HIST_BINS; off = off * 2u) {
        var t = 0.0;
        var t2 = 0.0;
        if (i >= off) {
            t = wa_cum[i - off];
            t2 = wa_cum2[i - off];
        }
        workgroupBarrier();
        wa_cum[i] = wa_cum[i] + t;
        wa_cum2[i] = wa_cum2[i] + t2;
        workgroupBarrier();
    }
    let total = wa_cum[CAM_HIST_BINS - 1u];
    let total2 = wa_cum2[CAM_HIST_BINS - 1u];
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
        wa_range[2] = CAM_HIST_HI;
    }
    workgroupBarrier();
    let bw = (CAM_HIST_HI - CAM_HIST_LO) / f32(CAM_HIST_BINS);
    let edge = CAM_HIST_LO + f32(i) * bw;
    let q0 = CAM_RANGE_SHARE.x * total;
    if (v > 0.0 && a < q0 && wa_cum[i] >= q0) {
        wa_range[0] = edge + bw * clamp((q0 - a) / v, 0.0, 1.0);
    }
    let a2 = wa_cum2[i] - v2;
    for (var e = 1u; e < 3u; e = e + 1u) {
        let q = select(CAM_RANGE_SHARE.y, CAM_TOP_SHARE, e == 2u) * total2;
        if (v2 > 0.0 && a2 < q && wa_cum2[i] >= q) {
            wa_range[e] = edge + bw * clamp((q - a2) / v2, 0.0, 1.0);
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
    } else {
        // the focus: an 8 x 8 grid over the frame's middle (a third across, 30 % down), each
        // tap's distance from the resolve's alpha (2 + metres; the sky 65504: 0 dioptres).
        let j = i - 64u;
        let dims = vec2<f32>(textureDimensions(cam_src));
        let uv = vec2<f32>(0.333 + 0.333 * (f32(j % 8u) + 0.5) / 8.0, 0.35 + 0.3 * (f32(j / 8u) + 0.5) / 8.0);
        let a = textureLoad(cam_src, vec2<i32>(uv * dims), 0).a;
        var t = vec2<f32>(0.0);
        if (a >= 2.0) {
            t = vec2<f32>(select(1.0 / max(a - 2.0, 0.3), 0.0, a > 60000.0), 1.0);
        }
        wa_af[j] = t;
    }
    workgroupBarrier();
    for (var s = CAM_HIST_BINS / 2u; s > 0u; s = s / 2u) {
        if (i < s) {
            wa_num[i] = wa_num[i] + wa_num[i + s];
            wa_den[i] = wa_den[i] + wa_den[i + s];
            if (i < 32u && s <= 32u) {
                wa_sun[i] = wa_sun[i] + wa_sun[i + s];
                wa_af[i] = wa_af[i] + wa_af[i + s];
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
        // side only as far as it needs and never past `cam0.xy` (the highlights above
        // their knee `cam3.w`); or, when even the frame's brightest sources fall short of
        // `cam4.y`, the highlights expanded over the knee toward it, never past `cam4.x`;
        // converging as the meter does.
        let mid = log2(0.18);
        let lo = wa_range[0] + rel - mid;
        let hi = wa_range[1] + rel - mid;
        let top = wa_range[2] + rel - mid;
        let knee = cine.cam3.w;
        var ks = 1.0;
        var kh = 1.0;
        if (total > 0.5 && total2 > 0.5) {
            ks = clamp(cine.cam3.x / max(-lo, 1e-3), cine.cam0.x, 1.0);
            if (hi > cine.cam3.y) {
                kh = clamp((cine.cam3.y - knee) / max(hi - knee, 1e-3), cine.cam0.y, 1.0);
            } else if (top > knee + 0.5 && top < cine.cam4.y) {
                kh = clamp((cine.cam4.y - knee) / (top - knee), 1.0, max(cine.cam4.x, 1.0));
            }
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
        // the focus pulled toward the middle's mean dioptre (snapped on a cut).
        if (wa_af[0].y > 32.0) {
            let target_f = wa_af[0].x / wa_af[0].y;
            if (cut || st.focus_ok < 0.5 || !(st.focus == st.focus)) {
                st.focus = target_f;
            } else {
                let dt = clamp(skyu.taa.w, 1.0 / 240.0, 0.25);
                st.focus = st.focus + (target_f - st.focus) * (1.0 - exp(-dt * CAM_FOCUS_RATE));
            }
            st.focus_ok = 1.0;
        } else {
            st.focus_ok = 0.0;
        }
        st.sun_e = wa_sun[0] / 64.0 * st.gain;
        // (`--white-balance`) the balance: the surfaces' mean chromaticity's log ratios, red and
        // blue over green; the colour temperature's axis (blue against red) corrected by
        // `cam6.x` (warm) or `cam6.y` (cool), the green-magenta axis (green against red and
        // blue) by `cam6.z`, each within its limit; the estimate's luminance kept (the balance moves colour, not the
        // exposure); converging as the meter does (snapped on a cut). No estimate (a frame of
        // sky, or of nothing near grey): toward neutral.
        if (SF_CAM_WB) {
            let wb0 = 2u * CAM_HIST_BINS;
            let sw = f32(atomicLoad(&cam_hist[wb0 + 3u]));
            var g = vec3<f32>(1.0);
            if (sw > 32.0 * CAM_WB_SCALE) {
                let e = vec3<f32>(f32(atomicLoad(&cam_hist[wb0])), f32(atomicLoad(&cam_hist[wb0 + 1u])), f32(atomicLoad(&cam_hist[wb0 + 2u]))) / sw;
                let lr = log2(max(e.r, 1e-4) / max(e.g, 1e-4));
                let lb = log2(max(e.b, 1e-4) / max(e.g, 1e-4));
                // (a warm light, blue under red: `cam6.x`; a cool one: `cam6.y`)
                let tl = 0.5 * (lb - lr);
                let tc = clamp(tl * select(cine.cam6.y, cine.cam6.x, tl < 0.0), -cine.cam6.w, cine.cam6.w);
                let mc = clamp(-0.5 * (lr + lb) * cine.cam6.z, -CAM_WB_TINT_LIMIT, CAM_WB_TINT_LIMIT);
                g = exp2(vec3<f32>(tc, -mc, -tc));
                g = g * (lk_luma(e) / max(lk_luma(e * g), 1e-6));
            }
            var wb = vec3<f32>(st.wb_r, st.wb_g, st.wb_b);
            if (cut || st.wb_ok < 0.5 || !all(wb > vec3<f32>(0.0)) || !all(wb < vec3<f32>(16.0))) {
                wb = g;
            } else {
                let dt = clamp(skyu.taa.w, 1.0 / 240.0, 0.25);
                wb = exp2(mix(log2(wb), log2(g), 1.0 - exp(-dt * CAM_ADAPT_RATE)));
            }
            st.wb_r = wb.r;
            st.wb_g = wb.g;
            st.wb_b = wb.b;
            st.wb_ok = 1.0;
        }
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
