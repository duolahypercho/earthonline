// (`--lamp-streaks`, the reflect track; sky_reflect.rs `lamp_streaks_gather_wgsl`)
// The cars' rear lamps as streak lights on the wet street. A tail lamp on a
// wet road at night throws a long red streak toward the eye like any lamp
// (Fresnel at grazing incidence and the film's long lobe): its lens is
// small but bright (ECE R7: a rear position lamp 4-17 cd, a stop lamp
// 60-185 cd, on a lens of a few hundred square centimetres). The frame's
// emissive lens alone cannot make it: read through the film's wide lobe its
// few pixels average away with the dark body round them. So, like the
// street lamps and headlamps (sky_streaks.wgsl), the rear lamps are
// reflected analytically, as a lens of their own size and intensity
// (sky_light.wgsl `sky_cine_streaks`, after the streak list).
//
// Of the cars whose rear faces the eye and stands in view, the `LS_CARS`
// brightest at the eye (intensity over the square of the distance; quarter-EV
// bins, the bin that fills the list, then the cars over it, as the streak
// list does) write both rear lamps' records whole after the streak list in
// `lt_clusters`: [lamps, 16 words a lamp (a SkyLight)...]. They are not in
// the frame list (`lt_frame`), so no cluster, sprite, fog or shadow sees them,
// and the frame's other lights are unchanged. A stop lamp where the car
// brakes (the agent's braking flag, as agents_draw.wgsl `ag_lamp` lights its
// lens), a tail lamp elsewhere; parked cars' tails as the draw lights them.
// None while `lt.scale.z` (units per candela, 0 with the switch off) is 0.
// One workgroup, after the streak pick.

const LS_BINS: u32 = 32u;

var<workgroup> ls_qmax: atomic<u32>;
var<workgroup> ls_hist: array<atomic<u32>, 32>;
var<workgroup> ls_cut: u32;
var<workgroup> ls_n: atomic<u32>;

/// The lamp's candela: the stop lamp's while the car brakes, the tail's else.
fn ls_cd(a: LtAgent) -> f32 {
    return select(LS_TAIL_CD, LS_STOP_CD, (a.flags & LS_FLAG_BRAKING) != 0u);
}

/// A rear lamp's place (`side` -1 right, 1 left, 0 the rear's middle): over
/// the rear wheels' pivots (wheels 2 and 3), inboard of them, at the tail
/// lamps' height over the axle and the rear overhang behind it; a model with
/// no wheels from its bounding sphere (as the headlamps, sky_lights.wgsl
/// `lights_agents`).
fn ls_lamp(a: LtAgent, m: LtModel, side: f32) -> vec3<f32> {
    let rw = m.wheel[2].xyz;
    var lat = abs(rw.x) * LS_LAMP.x;
    var up = rw.y + LS_LAMP.y;
    var back = rw.z - LS_LAMP.z;
    if (abs(rw.x) < 0.2) {
        lat = LS_LAMP_NONE.x;
        up = LS_LAMP_NONE.y;
        back = m.center.z - m.radius * LS_LAMP_NONE.z;
    }
    return a.pos + lt_rotate(a.rot, vec3<f32>(side * lat, up, back) * a.scale);
}

/// How bright the car's rear lamps are at the eye (0: not a car, its rear
/// turned from the eye, or out of view).
fn ls_q(i: u32) -> f32 {
    let a = lt_agents[i];
    let m = lt_models[a.model];
    if (m.model_class == LT_PERSON) {
        return 0.0;
    }
    let c = ls_lamp(a, m, 0.0);
    let back = lt_rotate(a.rot, vec3<f32>(0.0, 0.0, -1.0));
    let e = skyu.eye.xyz - c;
    if (dot(e, back) <= 0.0 || !lt_visible(c, LS_VIEW_M)) {
        return 0.0;
    }
    return ls_cd(a) / max(dot(e, e), 1.0);
}

fn ls_bin(q: f32, qmax: f32) -> u32 {
    return min(u32(max(log2(qmax / q), 0.0) * 4.0), LS_BINS - 1u);
}

fn ls_word(o: u32, x: f32) {
    lt_clusters[o] = bitcast<u32>(x);
}

/// Car `i`'s two rear lamps' records from word `o`.
fn ls_write(o: u32, i: u32) {
    let a = lt_agents[i];
    let m = lt_models[a.model];
    let back = normalize(lt_rotate(a.rot, vec3<f32>(0.0, 0.0, -1.0)));
    let intensity = ls_cd(a) * lt.scale.z;
    for (var s = 0u; s < 2u; s = s + 1u) {
        let p = ls_lamp(a, m, select(-1.0, 1.0, s == 0u));
        let w = o + 16u * s;
        ls_word(w, p.x);
        ls_word(w + 1u, p.y);
        ls_word(w + 2u, p.z);
        ls_word(w + 3u, intensity);
        ls_word(w + 4u, LS_RED.x);
        ls_word(w + 5u, LS_RED.y);
        ls_word(w + 6u, LS_RED.z);
        ls_word(w + 7u, 0.0);
        ls_word(w + 8u, back.x);
        ls_word(w + 9u, back.y);
        ls_word(w + 10u, back.z);
        ls_word(w + 11u, LS_COS.x);
        ls_word(w + 12u, LS_COS.y);
        lt_clusters[w + 13u] = LS_KIND;
        ls_word(w + 14u, LS_AREA_M2);
        ls_word(w + 15u, 0.0);
    }
}

@compute @workgroup_size(256)
fn lights_lamp_streaks(@builtin(local_invocation_index) lid: u32) {
    if (lid < LS_BINS) {
        atomicStore(&ls_hist[lid], 0u);
    }
    if (lid == 0u) {
        atomicStore(&ls_qmax, 0u);
        atomicStore(&ls_n, 0u);
    }
    workgroupBarrier();
    let n = select(0u, lt.counts.y, lt.counts.w != 0u && lt.scale.z > 0.0);
    for (var i = lid; i < n; i = i + 256u) {
        let q = ls_q(i);
        if (q > 0.0) {
            atomicMax(&ls_qmax, bitcast<u32>(q)); // positive floats order as their bits
        }
    }
    workgroupBarrier();
    let qmax = bitcast<f32>(atomicLoad(&ls_qmax));
    for (var i = lid; i < n; i = i + 256u) {
        let q = ls_q(i);
        if (q > 0.0) {
            atomicAdd(&ls_hist[ls_bin(q, qmax)], 1u);
        }
    }
    workgroupBarrier();
    if (lid == 0u) {
        var cum = 0u;
        var cut = LS_BINS - 1u;
        for (var b = 0u; b < LS_BINS; b = b + 1u) {
            cum = cum + atomicLoad(&ls_hist[b]);
            if (cum >= LS_CARS) {
                cut = b;
                break;
            }
        }
        // every car fits: all of them
        ls_cut = select(cut, LS_BINS, cum < LS_CARS);
    }
    let cut = workgroupUniformLoad(&ls_cut);
    let g = skyu.lights.yzw;
    let at = g.x * g.y * g.z * (1u + LT_CAP) + 1u + LT_STREAK_CAP;
    for (var i = lid; i < n; i = i + 256u) {
        let q = ls_q(i);
        let b = ls_bin(q, qmax);
        if (q > 0.0 && (b < cut || (cut == 0u && b == 0u))) {
            let k = atomicAdd(&ls_n, 1u);
            if (k < LS_CARS) {
                ls_write(at + 1u + 32u * k, i);
            }
        }
    }
    workgroupBarrier();
    if (lid == 0u) {
        lt_clusters[at] = 2u * min(atomicLoad(&ls_n), LS_CARS);
    }
}
