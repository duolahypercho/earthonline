// (`--rt-gi`, `--rt-reflect`; rt_composite.rs) The deferred composite of the
// traced passes: a full-screen pass over the HDR target after the traces, in
// the frame they were traced (no reprojection, no lag), before anything the
// city's opaque pass did not shade is drawn (props, agents, the far layer,
// the sky, the glass). rt_composite.rs prepends look_common_source().
//
// Per pixel the city pass left a G-buffer (rt_gi_use.wgsl). The pass adds,
// where a traced result covers the pixel (its half-resolution texels whose
// depth is this surface's) and before its fade:
//  * the diffuse light (`--rt-gi`): w x (b x E x ao' - a), E the traced
//    irradiance (the traced sky and bounce, the rig's ambient, the
//    environment by the traced open share), ao' the screen-space visibility
//    with params.x of it kept;
//  * the reflection (`--rt-reflect`): w x (d x L - c), L the traced radiance
//    along the surface's reflection lobe (a hit's light, or the sky).

struct RtCompositeView {
    // x how much of the screen-space occlusion stays on the diffuse (0:
    // replaced by the traced visibility), y the diffuse light's weight, z the
    // view depth it starts fading out at, w where it is gone
    gi: vec4<f32>,
    // the reflection's: y its weight, z its fade start, w its fade end
    refl: vec4<f32>,
    // xy the results' size (half resolution), zw the output's
    size: vec4<f32>,
    // x 1 with the diffuse light, y 1 with the reflection, z EO_RT_GI_DEBUG
    // (1 the traced diffuse light alone, 2 the open share, 3 the traced
    // irradiance), w EO_RT_REFLECT_DEBUG (1 the traced reflection alone, 2 its
    // radiance, 3 the rig's reflection it replaces)
    misc: vec4<f32>,
};

@group(0) @binding(0) var<uniform> look: Look;
@group(0) @binding(1) var<uniform> rc_view: RtCompositeView;
@group(0) @binding(2) var rc_gb_a: texture_2d<f32>;
@group(0) @binding(3) var rc_gb_b: texture_2d<f32>;
@group(0) @binding(4) var rc_gb_n: texture_2d<f32>;
@group(0) @binding(5) var rc_depth: texture_depth_2d;
@group(0) @binding(6) var rc_gi: texture_2d<f32>;
@group(0) @binding(7) var rc_gi_z: texture_2d<f32>;
@group(0) @binding(8) var rc_gb_c: texture_2d<f32>;
@group(0) @binding(9) var rc_gb_d: texture_2d<f32>;
@group(0) @binding(10) var rc_refl: texture_2d<f32>;
@group(0) @binding(11) var rc_refl_z: texture_2d<f32>;

@vertex
fn rc_vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

// look_light.wgsl `lk_sh` (the environment's irradiance at n).
fn rc_sh(n: vec3<f32>) -> vec3<f32> {
    let x = n.x;
    let y = n.y;
    let z = n.z;
    var e = look.sh[0].xyz * 0.282095;
    e = e + look.sh[1].xyz * (0.488603 * y) + look.sh[2].xyz * (0.488603 * z) + look.sh[3].xyz * (0.488603 * x);
    e = e + look.sh[4].xyz * (1.092548 * x * y) + look.sh[5].xyz * (1.092548 * y * z) + look.sh[6].xyz * (0.315392 * (3.0 * z * z - 1.0));
    e = e + look.sh[7].xyz * (1.092548 * x * z) + look.sh[8].xyz * (0.546274 * (x * x - y * y));
    return max(e, vec3<f32>(0.0));
}

// surfaces_ao_use.wgsl `ao_multi_bounce`.
fn rc_multi_bounce(v: f32) -> f32 {
    let al = 0.7;
    let a = 2.0404 * al - 0.3324;
    let b = -4.7951 * al + 0.6417;
    let c = 2.7552 * al + 0.6903;
    return max(v, ((v * a + b) * v + c) * v);
}

// A half-resolution result at full-resolution pixel p: its 2 x 2 texels round
// the pixel, bilinear, each refused where its depth is not view depth vd
// (w 0: none).
struct RcSample {
    v: vec4<f32>,
    w: f32,
};

fn rc_upsample(t: texture_2d<f32>, tz: texture_2d<f32>, p: vec2<i32>, vd: f32) -> RcSample {
    let hd = vec2<i32>(rc_view.size.xy);
    let f = (vec2<f32>(p) + 0.5) * 0.5 - 0.5;
    let base = vec2<i32>(floor(f));
    let fr = f - floor(f);
    var s = vec4<f32>(0.0);
    var wsum = 0.0;
    for (var k = 0; k < 4; k++) {
        let o = vec2<i32>(k & 1, k >> 1);
        let q = clamp(base + o, vec2<i32>(0), hd - 1);
        let z = textureLoad(tz, q, 0).r;
        if (z < 0.0 || abs(z - vd) > 0.04 * vd + 0.15) {
            continue;
        }
        let bw = select(1.0 - fr.x, fr.x, o.x == 1) * select(1.0 - fr.y, fr.y, o.y == 1) + 1e-4;
        s = s + textureLoad(t, q, 0) * bw;
        wsum = wsum + bw;
    }
    if (wsum <= 1e-3) {
        return RcSample(vec4<f32>(0.0), 0.0);
    }
    return RcSample(s / wsum, 1.0);
}

struct RcOut {
    @location(0) color: vec4<f32>,
};

@fragment
fn rc_fs(@builtin(position) frag: vec4<f32>) -> RcOut {
    let p = vec2<i32>(frag.xy);
    let gn = textureLoad(rc_gb_n, p, 0);
    if (gn.a < 0.5) {
        discard;
    }
    let dims = vec2<f32>(rc_view.size.zw);
    let d = textureLoad(rc_depth, p, 0);
    let uv = (vec2<f32>(p) + 0.5) / dims;
    let w4 = look.inv_view_proj * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    let vd = dot(w4.xyz / w4.w, look.cam_fwd.xyz);
    var add = vec3<f32>(0.0);
    var dbg = vec3<f32>(0.0);
    var any_term = false;
    if (rc_view.misc.x > 0.5) {
        let fade = (1.0 - lk_smooth(rc_view.gi.z, rc_view.gi.w, vd)) * rc_view.gi.y;
        let s = rc_upsample(rc_gi, rc_gi_z, p, vd);
        if (fade > 0.0 && s.w > 0.0) {
            let ga = textureLoad(rc_gb_a, p, 0);
            let gb = textureLoad(rc_gb_b, p, 0);
            let n = normalize(gn.xyz * 2.0 - 1.0);
            let e = s.v.rgb + look.ambient.xyz + rc_sh(n) * ga.a * s.v.a;
            let ao = rc_multi_bounce(mix(gb.a, 1.0, 1.0 - rc_view.gi.x));
            let traced = gb.rgb * e * ao;
            add = add + fade * (traced - ga.rgb);
            dbg = select(select(s.v.rgb, vec3<f32>(s.v.a), rc_view.misc.z < 2.5), traced, rc_view.misc.z < 1.5);
            any_term = true;
        }
    }
    if (rc_view.misc.y > 0.5) {
        let fade = (1.0 - lk_smooth(rc_view.refl.z, rc_view.refl.w, vd)) * rc_view.refl.y;
        let s = rc_upsample(rc_refl, rc_refl_z, p, vd);
        if (fade > 0.0 && s.w > 0.0) {
            let gc = textureLoad(rc_gb_c, p, 0);
            let gd = textureLoad(rc_gb_d, p, 0);
            let traced = gd.rgb * s.v.rgb;
            add = add + fade * (traced - gc.rgb);
            if (rc_view.misc.w > 0.5) {
                dbg = select(select(gc.rgb, s.v.rgb, rc_view.misc.w < 2.5), traced, rc_view.misc.w < 1.5);
            }
            any_term = true;
        }
    }
    if (!any_term) {
        discard;
    }
    var out: RcOut;
    if (rc_view.misc.z > 0.5 || rc_view.misc.w > 0.5) {
        // the debug views (the replacing pipeline).
        out.color = vec4<f32>(dbg, 1.0);
        return out;
    }
    // added to the colour (one + one): the traced light in place of the rig's.
    out.color = vec4<f32>(add, 0.0);
    return out;
}
