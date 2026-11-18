// Depth-only sun shadow pass: the shared vertex buffer, one cascade's
// view-projection (dynamic uniform offset), the object index in
// first_instance as in draw.wgsl. No fragment stage.

struct Object {
    min: vec3<f32>,
    index_count: u32,
    max: vec3<f32>,
    first_index: u32,
    base_vertex: i32,
    kind: u32,
    copy: u32,
    material: u32,
    lod_group: u32,
    lod_role: u32,
    far_first_index: u32,
    far_index_count: u32,
};

struct ShadowView {
    view_proj: mat4x4<f32>,
    // x = metres between stress copies
    copy_step: vec4<f32>,
};

@group(0) @binding(0) var<uniform> sv: ShadowView;
@group(0) @binding(1) var<storage, read> objects: array<Object>;

@vertex
fn shadow_vs(@location(0) pos: vec3<f32>, @builtin(instance_index) inst: u32) -> @builtin(position) vec4<f32> {
    let k = objects[inst].copy;
    let p = pos + vec3<f32>(f32(k % 4u), 0.0, f32(k / 4u)) * sv.copy_step.x;
    return sv.view_proj * vec4<f32>(p, 1.0);
}
