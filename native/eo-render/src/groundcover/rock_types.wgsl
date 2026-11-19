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
// negative: nothing cut).
struct GcKind {
    rim: vec4<f32>,
};

const GC_KINDS: u32 = 7u;
const GC_LODS: u32 = 4u;
const GC_VIEWS: u32 = 4u;

fn gc_rotate(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    let t = 2.0 * cross(q.xyz, v);
    return v + q.w * t + cross(q.xyz, t);
}
