// The scanned rock's shared types (groundcover/rock.rs), ahead of its
// cull, draw and shadow shaders.

struct GcRock {
    // where the scan's origin lands, and which scan
    pos: vec3<f32>,
    kind: u32,
    // the turn (x, y, z, w), scan frame to world
    rot: vec4<f32>,
    scale: vec3<f32>,
    // 0 dry, 1 wet through
    wet: f32,
    // a factor on the scan's albedo (linear)
    tint: vec3<f32>,
    // the bounding sphere's radius (world metres)
    radius: f32,
    // and its centre (world)
    center: vec3<f32>,
    flags: u32,
};

// Per scan: its rim plane in its own frame (y = a x + b z + c) and the
// height over it under which a scanned patch's own ground is cut away (w;
// negative: nothing cut); and its cut rim's treatment: x the band (scan
// metres) over which the rim frays, y how far the rim sinks under the
// rim plane (scan metres), z the fray's noise scale (cycles a metre), w
// the rim distance a vertex's 127 stands for (scan metres).
struct GcKind {
    rim: vec4<f32>,
    fray: vec4<f32>,
};

const GC_KINDS: u32 = 7u;
const GC_LODS: u32 = 4u;
const GC_VIEWS: u32 = 4u;

fn gc_rotate(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    let t = 2.0 * cross(q.xyz, v);
    return v + q.w * t + cross(q.xyz, t);
}

// A vertex's distance to its scan's cut rim (scan metres), from the
// vertex's fourth normal byte (snorm: 1 at `fray.w`).
fn gc_rim_dist(k: GcKind, n_w: f32) -> f32 {
    return clamp(n_w, 0.0, 1.0) * k.fray.w;
}

// Where a vertex lands in its scan's frame: near the cut rim it sinks
// under the rim plane (along the plane's normal), most at the rim itself,
// so the cut edge goes into the ground instead of standing as a straight
// lip over it.
fn gc_rim_sink(k: GcKind, pos: vec3<f32>, rim_d: f32) -> vec3<f32> {
    if (k.fray.y <= 0.0) {
        return pos;
    }
    let t = 1.0 - smoothstep(0.0, k.fray.x, rim_d);
    let n = normalize(vec3<f32>(-k.rim.x, 1.0, -k.rim.y));
    return pos - n * (k.fray.y * t * t);
}

fn gc_hash3(p: vec3<f32>) -> f32 {
    let q = fract(p * vec3<f32>(0.1031, 0.1030, 0.0973));
    let r = q + dot(q, q.yxz + 33.33);
    return fract((r.x + r.y) * r.z);
}

fn gc_noise3(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(mix(gc_hash3(i), gc_hash3(i + vec3<f32>(1.0, 0.0, 0.0)), u.x), mix(gc_hash3(i + vec3<f32>(0.0, 1.0, 0.0)), gc_hash3(i + vec3<f32>(1.0, 1.0, 0.0)), u.x), u.y);
    let b = mix(mix(gc_hash3(i + vec3<f32>(0.0, 0.0, 1.0)), gc_hash3(i + vec3<f32>(1.0, 0.0, 1.0)), u.x), mix(gc_hash3(i + vec3<f32>(0.0, 1.0, 1.0)), gc_hash3(i + vec3<f32>(1.0, 1.0, 1.0)), u.x), u.y);
    return mix(a, b, u.z);
}

// Whether a fragment near the cut rim is frayed away: a ragged edge (two
// octaves of value noise on the scan's own frame, so it holds still and
// is the same in the colour and shadow passes) instead of the straight
// line where the scan stopped.
fn gc_frayed(k: GcKind, local: vec3<f32>, rim_d: f32) -> bool {
    if (k.fray.x <= 0.0) {
        return false;
    }
    let p = local * k.fray.z;
    let n = 0.65 * gc_noise3(p) + 0.35 * gc_noise3(p * 2.7 + vec3<f32>(17.0, 5.0, 11.0));
    return rim_d < k.fray.x * n;
}
