// (`--look cinematic`, stage L3) the streak lights: of the frame's lights
// (`lt_frame`, gathered by sky_lights.wgsl), the `LT_STREAK_CAP` brightest
// toward the eye (luminance x intensity over the square of the eye's
// distance), whose reflections the wet street draws as long streaks without
// the clusters' reach (sky_light.wgsl `sky_cine_streaks`). Their indices go
// after the clusters' lists in `lt_clusters`: [count, index...]. One
// workgroup, after the clusters: the brightest's level, a histogram of how
// far below it each light lies (half-EV bins), the bin that fills the list,
// then the lights over it (so the set does not hang on the order the lights
// were gathered in; only when the brightest bin alone fills the list does it).
// Appended to the gather's module only with SF_CINEMATIC compiled in (sky.rs
// `cine_lamps_wgsl`), dispatched only under the look.

const LT_STREAK_BINS: u32 = 32u;

var<workgroup> lt_smax: atomic<u32>;
var<workgroup> lt_shist: array<atomic<u32>, 32>;
var<workgroup> lt_scut: u32;
var<workgroup> lt_sn: atomic<u32>;

/// How bright a light's reflection can be toward the eye (0: none: behind the eye).
fn lt_streak_q(l: SkyLight) -> f32 {
    let v = lt_view(l.pos);
    if (v.z < -10.0 || l.intensity <= 0.0) {
        return 0.0;
    }
    if (LT_LAMP_BOUNCE && l.kind == LT_BOUNCE_KIND) {
        return 0.0; // (`--lamp-bounce`) a pool's bounce stands under the street: no streak
    }
    let lum = dot(l.color, vec3<f32>(0.2126, 0.7152, 0.0722));
    let r = l.pos - skyu.eye.xyz;
    var q = lum * l.intensity / max(dot(r, r), 1.0);
    if ((SG_ON || RL_ON) && l.kind >= NT_SIGNAL_KIND) {
        // (`--signal-spill`, `--rear-lamps`) by its lobe toward the eye: a head or a car
        // facing away has no streak to give, and takes no lamp's place in the list
        let sig = l.kind == NT_SIGNAL_KIND;
        q = q * nt_lobe(-r / max(length(r), 1e-3), l.dir, select(NT_REAR_V, NT_SIGNAL_V, sig), select(NT_REAR_H, NT_SIGNAL_H, sig));
    }
    return q;
}

fn lt_streak_bin(q: f32, qmax: f32) -> u32 {
    return min(u32(max(log2(qmax / q), 0.0) * 2.0), LT_STREAK_BINS - 1u);
}

@compute @workgroup_size(256)
fn lights_streaks(@builtin(local_invocation_index) lid: u32) {
    if (lid < LT_STREAK_BINS) {
        atomicStore(&lt_shist[lid], 0u);
    }
    if (lid == 0u) {
        atomicStore(&lt_smax, 0u);
        atomicStore(&lt_sn, 0u);
    }
    workgroupBarrier();
    let n = min(atomicLoad(&lt_count[0]), lt.counts.z);
    for (var i = lid; i < n; i = i + 256u) {
        let q = lt_streak_q(lt_frame[i]);
        if (q > 0.0) {
            atomicMax(&lt_smax, bitcast<u32>(q)); // positive floats order as their bits
        }
    }
    workgroupBarrier();
    let qmax = bitcast<f32>(atomicLoad(&lt_smax));
    for (var i = lid; i < n; i = i + 256u) {
        let q = lt_streak_q(lt_frame[i]);
        if (q > 0.0) {
            atomicAdd(&lt_shist[lt_streak_bin(q, qmax)], 1u);
        }
    }
    workgroupBarrier();
    if (lid == 0u) {
        var cum = 0u;
        var cut = LT_STREAK_BINS - 1u;
        for (var b = 0u; b < LT_STREAK_BINS; b = b + 1u) {
            cum = cum + atomicLoad(&lt_shist[b]);
            if (cum >= LT_STREAK_CAP) {
                cut = b;
                break;
            }
        }
        lt_scut = cut;
    }
    let cut = workgroupUniformLoad(&lt_scut);
    let g = skyu.lights.yzw;
    let base = g.x * g.y * g.z * (1u + LT_CAP);
    for (var i = lid; i < n; i = i + 256u) {
        let q = lt_streak_q(lt_frame[i]);
        let b = lt_streak_bin(q, qmax);
        if (q > 0.0 && (b < cut || (cut == 0u && b == 0u))) {
            let k = atomicAdd(&lt_sn, 1u);
            if (k < LT_STREAK_CAP) {
                lt_clusters[base + 1u + k] = i;
            }
        }
    }
    workgroupBarrier();
    if (lid == 0u) {
        lt_clusters[base] = min(atomicLoad(&lt_sn), LT_STREAK_CAP);
    }
}
