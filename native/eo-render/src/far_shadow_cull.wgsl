// The far sun shadow map's casters (far_shadow.rs): one invocation per
// object, and one per entry of the far layer's cell table, against a ring's
// box (or the tile of it drawn this frame), appending DrawIndexedIndirect
// commands to the ring's lists (or writing fixed slots with instance count
// 0/1 in the uncompacted fallback).
//
// Each building casts once, at the cheapest level that is whole whatever
// the camera's own LOD choice this frame, so a map drawn over several
// frames, and kept for many, holds every caster:
//
//  * its footprint prism (a kit building's box, a Blender building's
//    stand-in, kinds 0 with roles LOD_KIT_BOX / LOD_FAR), unless the far
//    layer holds it (its neighbourhood cell or district resident): then the
//    cell's mesh, roof line included, casts instead (the cell list, and
//    hlod_far_shadow.wgsl drops a district's buildings whose cell is resident);
//  * the terrain at the level the camera draws it (full within the terrain
//    LOD distance, the coarse index range past it: cull.wgsl's rule, so the
//    hills shadow themselves exactly as they are drawn);
//  * a grade-separated road's concrete at its coarse level (kind 12, role 9);
//  * a landmark's far item (role 11), and the casting objects with no LOD
//    role (kinds 0, 1, 4, 5, 9);
//  * nothing transparent, no street surface, paint, deck surface, sea, skirt
//    or open ground, no near level (a model's parts, kit rungs, near and far
//    street and structure packs, a landmark's near draws).

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

struct FarCull {
    planes: array<vec4<f32>, 6>,
    // xyz the camera, w the terrain LOD distance
    cam: vec4<f32>,
    object_count: u32,
    compact: u32,
    // entries of the far layer's cell table (0: no far layer)
    cell_count: u32,
    ring: u32,
};

const LOD_ALWAYS: u32 = 0u;
const LOD_FAR: u32 = 2u;
const LOD_TERRAIN: u32 = 3u;
const LOD_KIT_BOX: u32 = 6u;
const LOD_STREET_COARSE: u32 = 9u;
const LOD_LANDMARK_FAR: u32 = 11u;
const BLEND_BIT: u32 = 256u;
const KIND_STRUCTURE: u32 = 12u;
const NO_CELL: u32 = 0xffffffffu;

@group(0) @binding(0) var<uniform> fc: FarCull;
@group(0) @binding(1) var<storage, read> objects: array<Object>;
// per ring: object_count command slots
@group(0) @binding(2) var<storage, read_write> draws: array<u32>;
// per ring: object commands, object triangles, cell commands, cell triangles
@group(0) @binding(3) var<storage, read_write> counters: array<atomic<u32>, 8>;
// per LOD group its neighbourhood cell; per cell the residency (bit 0 the cell, bit 1 its district)
@group(0) @binding(4) var<storage, read> hlod_map: array<u32>;
@group(0) @binding(5) var<storage, read> hlod_flags: array<u32>;
@group(0) @binding(6) var<storage, read> cells: array<HlCell>;
// per ring: FAR_MAX_CELLS command slots
@group(0) @binding(7) var<storage, read_write> cell_draws: array<u32>;

fn inside(mn: vec3<f32>, mx: vec3<f32>) -> bool {
    for (var p = 0u; p < 6u; p = p + 1u) {
        let pl = fc.planes[p];
        let v = select(mn, mx, pl.xyz >= vec3<f32>(0.0));
        if (dot(pl.xyz, v) + pl.w < 0.0) {
            return false;
        }
    }
    return true;
}

// Whether the far layer holds group g's building (its cell or its district resident).
fn far_held(g: u32) -> bool {
    if (fc.cell_count == 0u) {
        return false;
    }
    let c = hlod_map[g];
    return c != NO_CELL && (hlod_flags[c] & 3u) != 0u;
}

@compute @workgroup_size(64)
fn far_cull_objects(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    let n = fc.object_count;
    if (i >= n) {
        return;
    }
    let o = objects[i];
    let kind = o.kind & 255u;
    var first = o.first_index;
    var count = o.index_count;
    var casts = (o.kind & BLEND_BIT) == 0u;
    let role = o.lod_role;
    if (role == LOD_TERRAIN) {
        let p = clamp(fc.cam.xyz, o.min, o.max);
        if (distance(p, fc.cam.xyz) > fc.cam.w) {
            first = o.far_first_index;
            count = o.far_index_count;
        }
    } else if (role == LOD_FAR || role == LOD_KIT_BOX) {
        casts = casts && !far_held(o.lod_group);
    } else if (role == LOD_STREET_COARSE) {
        casts = casts && kind == KIND_STRUCTURE;
    } else if (role == LOD_ALWAYS) {
        casts = casts && (kind == 0u || kind == 1u || kind == 4u || kind == 5u || kind == 9u);
    } else if (role != LOD_LANDMARK_FAR) {
        casts = false;
    }
    casts = casts && count > 0u && inside(o.min, o.max);
    let base = fc.ring * n;
    let w = fc.ring * 4u;
    if (fc.compact == 1u) {
        if (casts) {
            let slot = atomicAdd(&counters[w], 1u);
            let b = (base + slot) * 5u;
            draws[b] = count;
            draws[b + 1u] = 1u;
            draws[b + 2u] = first;
            draws[b + 3u] = bitcast<u32>(o.base_vertex);
            draws[b + 4u] = i;
            atomicAdd(&counters[w + 1u], count / 3u);
        }
    } else {
        let b = (base + i) * 5u;
        draws[b] = count;
        draws[b + 1u] = select(0u, 1u, casts);
        draws[b + 2u] = first;
        draws[b + 3u] = bitcast<u32>(o.base_vertex);
        draws[b + 4u] = i;
        if (casts) {
            atomicAdd(&counters[w], 1u);
            atomicAdd(&counters[w + 1u], count / 3u);
        }
    }
}

@compute @workgroup_size(64)
fn far_cull_cells(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= fc.cell_count) {
        return;
    }
    let c = cells[i];
    if ((c.flags & 1u) == 0u || c.index_count == 0u || !inside(c.min, c.max)) {
        return;
    }
    let w = fc.ring * 4u;
    let slot = atomicAdd(&counters[w + 2u], 1u);
    let b = (fc.ring * FAR_MAX_CELLS + slot) * 5u;
    cell_draws[b] = c.index_count;
    cell_draws[b + 1u] = 1u;
    cell_draws[b + 2u] = c.first_index;
    cell_draws[b + 3u] = bitcast<u32>(c.base_vertex);
    cell_draws[b + 4u] = i;
    atomicAdd(&counters[w + 3u], c.index_count / 3u);
}
