// Depth-only pass of the far layer's cells into the far sun shadow map
// (far_shadow.rs): the far layer's own vertex layout (position, and the
// building's LOD group at location 1), the entry index in first_instance as
// hlod_draw.wgsl has it, one ring's view-projection (dynamic uniform offset).
// A neighbourhood cell casts every building it holds; a district casts only
// the buildings whose own cell is not resident (the cell casts those), so
// every building is in the map once (far_shadow_cull.wgsl keeps their
// footprint prisms out while the far layer holds them). No fragment stage.

struct ShadowView {
    view_proj: mat4x4<f32>,
    copy_step: vec4<f32>,
};

struct HlCell {
    min: vec3<f32>,
    layer_base: u32,
    max: vec3<f32>,
    flags: u32,
    first_index: u32,
    index_count: u32,
    base_vertex: i32,
    cell: u32,
};

@group(0) @binding(0) var<uniform> sv: ShadowView;
@group(0) @binding(1) var<storage, read> fs_cells: array<HlCell>;
@group(0) @binding(2) var<storage, read> fs_map: array<u32>;
@group(0) @binding(3) var<storage, read> fs_flags: array<u32>;

@vertex
fn far_cell_vs(@location(0) pos: vec3<f32>, @location(1) grp: u32, @builtin(instance_index) entry: u32) -> @builtin(position) vec4<f32> {
    if ((fs_cells[entry].flags & 2u) != 0u) {
        let c = fs_map[grp];
        if (c != 0xffffffffu && (fs_flags[c] & 1u) != 0u) {
            return vec4<f32>(2.0, 2.0, 2.0, 1.0); // its cell casts it
        }
    }
    return sv.view_proj * vec4<f32>(pos, 1.0);
}
