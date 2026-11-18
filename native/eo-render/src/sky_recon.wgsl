// the volume's full-resolution reconstruction (appended to look_common.wgsl).
// The volume pass traces one pixel of each 2 x 2 block a frame (the block's
// pixels in turn: (0,0), (1,1), (1,0), (0,1)), at that pixel's own position, not
// the block's corner. This pass rebuilds every full-resolution pixel from:
//  * its fresh sample, when it is the pixel traced this frame;
//  * its own history (last frame's reconstruction at the same world point:
//    the sky by direction, the ground by depth), clamped to the range of the
//    four nearest traced samples so a moved cloud does not leave a ghost;
//  * a depth-aware blend of the four nearest traced samples, where the
//    history is missing (a cut, a disocclusion, the edge of the screen).
// Every pixel is traced every fourth frame and keeps its own value between, so
// the clouds come out at full resolution (crisp rims, near clouds sharp) for the
// cost of a quarter-resolution trace, and the march's grain averages out.
// Output: radiance (rgb) and transmittance (a), the view distance (-1 - the
// share of the sun's disc that gets through, over the sky), as the volume pass.

@group(0) @binding(0) var<uniform> look: Look;
@group(0) @binding(1) var<uniform> skyu: Sky;
@group(0) @binding(2) var rc_depth: texture_depth_2d;
@group(0) @binding(3) var rc_vol: texture_2d<f32>;
@group(0) @binding(4) var rc_volz: texture_2d<f32>;
@group(0) @binding(5) var rc_prev: texture_2d<f32>;
@group(0) @binding(6) var rc_prevz: texture_2d<f32>;
@group(0) @binding(7) var rc_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(8) var rc_zout: texture_storage_2d<r32float, write>;
@group(0) @binding(9) var rc_lin: sampler;

/// The block pixel the volume pass traces this frame (sky.rs `trace_offset`, the same order).
fn rc_offset(frame: u32) -> vec2<i32> {
    var offs = array<vec2<i32>, 4>(vec2<i32>(0, 0), vec2<i32>(1, 1), vec2<i32>(1, 0), vec2<i32>(0, 1));
    return offs[frame % 4u];
}

@compute @workgroup_size(8, 8)
fn rc_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let od = vec2<i32>(textureDimensions(rc_out));
    let p = vec2<i32>(gid.xy);
    if (p.x >= od.x || p.y >= od.y) {
        return;
    }
    let dd = vec2<i32>(textureDimensions(rc_depth));
    let pd = min(p, dd - 1);
    let d = textureLoad(rc_depth, pd, 0);
    let uv = (vec2<f32>(p) + 0.5) / vec2<f32>(od);
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let fp = look.inv_view_proj * vec4<f32>(ndc, 1.0, 1.0);
    let dir = normalize(fp.xyz / fp.w);
    var dist = 1e9;
    if (d > 0.0) {
        let w = look.inv_view_proj * vec4<f32>(ndc, d, 1.0);
        dist = length(w.xyz / w.w);
        dist = max(dist, 1e-9 / d / max(dot(dir, look.cam_fwd.xyz), 1e-3));
    }
    let eye = look.cam_pos.xyz;
    dist = sky_air_end(eye.y, dir.y, dist);
    let geo = dist < 1e8;

    // The four nearest traced samples (the volume texels round this pixel), by depth.
    let vd = vec2<i32>(textureDimensions(rc_vol));
    let hv = (vec2<f32>(p) + 0.5) * vec2<f32>(vd) / vec2<f32>(od) - 0.5;
    let b = vec2<i32>(floor(hv));
    let f = fract(hv);
    var acc = vec4<f32>(0.0);
    var wsum = 0.0;
    var lo = vec4<f32>(1e9);
    var hi = vec4<f32>(-1e9);
    var nbox = 0.0;
    var disc = 0.0;
    var dn = 0.0;
    for (var k = 0; k < 4; k = k + 1) {
        let o = vec2<i32>(k & 1, k >> 1);
        let q = clamp(b + o, vec2<i32>(0), vd - 1);
        let bw = select(1.0 - f.x, f.x, o.x == 1) * select(1.0 - f.y, f.y, o.y == 1);
        let z = textureLoad(rc_volz, q, 0).r;
        var dw = 0.001;
        if (!geo) {
            dw = select(0.001, 1.0, z < 0.0);
        } else if (z >= 0.0) {
            dw = exp(-abs(z - dist) / (0.04 * dist + 0.5)) + 0.001;
            if (dist > 3000.0 && z > 3000.0) {
                dw = max(dw, 0.5);
            }
        }
        let s = textureLoad(rc_vol, q, 0);
        let w = (bw + 0.05) * dw;
        acc = acc + s * w;
        wsum = wsum + w;
        if (dw > 0.05) {
            lo = min(lo, s);
            hi = max(hi, s);
            nbox = nbox + 1.0;
        }
        if (z < 0.0) {
            disc = disc + clamp(-z - 1.0, 0.0, 1.0);
            dn = dn + 1.0;
        }
    }
    var spatial = acc / max(wsum, 1e-6);
    if (any(spatial != spatial)) {
        spatial = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }

    // This pixel's own fresh sample.
    let off = rc_offset(skyu.flags.y);
    let fresh = all((p % 2) == off);
    var cur = spatial;
    if (fresh) {
        // 0.5 its own trace, 0.5 the depth-aware blend of the four round it (the
        // march's per-trace noise at a pixel is uncorrelated with its neighbours': rim grain).
        // up to 0.8 at a thin rim (the blend's cover 0.05-0.6): the speckle sat there
        let own = textureLoad(rc_vol, clamp(p / 2, vec2<i32>(0), vd - 1), 0);
        let edge = smoothstep(0.02, 0.15, 1.0 - spatial.a) * (1.0 - smoothstep(0.5, 0.85, 1.0 - spatial.a));
        cur = mix(own, spatial, mix(0.5, 0.8, edge));
    }

    var res = cur;
    if (skyu.taa.y > 0.5) {
        let wp = select(eye + dir * 1e5, eye + dir * dist, geo);
        let pc = skyu.prev_vp * vec4<f32>(wp, 1.0);
        if (pc.w > 1e-3) {
            let puv = vec2<f32>(pc.x / pc.w * 0.5 + 0.5, 0.5 - pc.y / pc.w * 0.5);
            if (all(puv >= vec2<f32>(0.0)) && all(puv <= vec2<f32>(1.0))) {
                let hd = vec2<i32>(textureDimensions(rc_prevz));
                let hz = textureLoad(rc_prevz, clamp(vec2<i32>(puv * vec2<f32>(hd)), vec2<i32>(0), hd - 1), 0).r;
                let same = select((hz < 0.0), (hz >= 0.0) && (abs(hz - dist) < 0.05 * dist + 1.0), geo);
                if (same) {
                    var h = textureSampleLevel(rc_prev, rc_lin, puv, 0.0);
                    if (all(h == h)) {
                        // how far the history moved on screen (pixels): a still view keeps
                        // averaging its samples instead of being pulled back to this frame's noisy
                        // neighbourhood every frame (that clamp is what left the rims' grain).
                        // (this frame's sub-pixel jitter taken out: the rays are jittered, `prev_vp` is not)
                        let mv = puv - uv + vec2<f32>(skyu.jitter.x * 0.5, -skyu.jitter.y * 0.5);
                        let still = 1.0 - smoothstep(0.15, 0.6, length(mv * vec2<f32>(od)));
                        if (nbox > 0.0) {
                            // The traced samples' range, widened a little (the march's grain); wider
                            // on a still view (only a moved cloud or a change of light needs it).
                            let m = mix(0.15, 0.5, still) * (hi - lo) + mix(0.02, 0.06, still) * abs(hi);
                            h = clamp(h, lo - m, hi + m);
                        }
                        // A pixel is traced every fourth frame: its fresh sample weighs as one of
                        // the samples it has had since the cut (`taa.y` frames), then 0.2 (clouds:
                        // 0.08 on a still view, ~24 samples).
                        let n = max(skyu.taa.y / 4.0, 1.0);
                        let a = select(0.06, max(mix(0.2, 0.08, still), 1.0 / (n + 1.0)), fresh);
                        res = mix(h, cur, a);
                    }
                }
            }
        }
    }
    if (any(res != res) || any(abs(res) > vec4<f32>(1e5))) {
        res = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    textureStore(rc_out, p, max(res, vec4<f32>(0.0)));
    let tdisc = select(0.0, disc / max(dn, 1.0), dn > 0.0);
    textureStore(rc_zout, p, vec4<f32>(select(-1.0 - tdisc, dist, geo)));
}
