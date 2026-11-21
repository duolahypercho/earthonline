// Sun shadow casters: one invocation per object tests it against the three
// cascade boxes and appends a DrawIndexedIndirect command to each cascade's
// list it falls in (or writes its fixed slot with instance_count 0/1 in the
// fallback). The same object table, LOD groups and level choice as the
// camera's cull pass (cull.wgsl, which runs first in the frame and decides
// every group's level): a building casts at exactly the level it is drawn.
// Casters: buildings, Blender model parts and kit buildings (and any kind
// added later); not the ground (terrain, roads, sea, the street surface and
// its paint: kinds 1, 2, 3, 6, 7 receive only), and nothing transparent.

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

struct Group {
    center: vec3<f32>,
    radius: f32,
    ready: u32,
    detailed: u32,
    kind: u32,
    level: u32,
};

struct ShadowCull {
    planes: array<vec4<f32>, 18>,
    camera: vec4<f32>,
    object_count: u32,
    compact: u32,
    // bit c: cascade c is refreshed this frame
    mask: u32,
    pad1: u32,
};

const LOD_NEAR: u32 = 1u;
const LOD_FAR: u32 = 2u;
const LOD_TERRAIN: u32 = 3u;
const LOD_KIT_NEAR: u32 = 4u;
const LOD_KIT_BOX: u32 = 6u;
const BLEND_BIT: u32 = 256u;

@group(0) @binding(0) var<uniform> sp: ShadowCull;
@group(0) @binding(1) var<storage, read> objects: array<Object>;
@group(0) @binding(2) var<storage, read> groups: array<Group>;
@group(0) @binding(3) var<storage, read_write> draws: array<u32>;
// [0..3) commands per cascade, [3..6) triangles per cascade
@group(0) @binding(4) var<storage, read_write> counters: array<atomic<u32>, 8>;

fn write_cmd(slot: u32, count: u32, instances: u32, first: u32, base_vertex: i32, object: u32) {
    let b = slot * 5u;
    draws[b] = count;
    draws[b + 1u] = instances;
    draws[b + 2u] = first;
    draws[b + 3u] = bitcast<u32>(base_vertex);
    draws[b + 4u] = object;
}

@compute @workgroup_size(64)
fn shadow_cull(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    let n = sp.object_count;
    if (i >= n) {
        return;
    }
    let o = objects[i];
    let kind = o.kind & 255u;
    var first = o.first_index;
    var count = o.index_count;
    if (o.lod_role == 12u && distance(sp.camera.xyz, clamp(sp.camera.xyz, o.min, o.max)) > 120.) {
        first = o.far_first_index;
        count = o.far_index_count;
    }
    let base_caster = (o.kind & BLEND_BIT) == 0u && kind != 1u && kind != 2u && kind != 3u && kind != 6u && kind != 7u && count > 0u;
    var caster = base_caster;
    // Kit buildings in the outer cascades (5-10 cm texels) cast with their
    // reduced rung when it is resident, else their footprint box: the shadow
    // LOD the JS gets from its caster policy (sub-texel parts refused).
    // Cascade 1: kit buildings with the reduced rung (else the box); cascade 2
    // (15 cm texels): every kit and Blender building as its footprint box.
    var mid_caster = caster;
    var far_caster = caster;
    if (o.lod_role == LOD_NEAR || o.lod_role == LOD_FAR) {
        let g = groups[o.lod_group];
        let near = g.ready == 1u && g.detailed == 1u;
        caster = caster && select(!near, near, o.lod_role == LOD_NEAR);
        mid_caster = caster;
        far_caster = base_caster && o.lod_role == LOD_FAR;
    } else if (o.lod_role >= LOD_KIT_NEAR && o.lod_role <= LOD_KIT_BOX) {
        let g = groups[o.lod_group];
        let role_level = o.lod_role - LOD_KIT_NEAR;
        caster = caster && g.level == role_level;
        mid_caster = base_caster && select(2u, 1u, (g.ready & 2u) != 0u) == role_level;
        far_caster = base_caster && role_level == 2u;
    } else if (o.lod_role == LOD_TERRAIN) {
        caster = false;
        mid_caster = false;
        far_caster = false;
    } else if (o.lod_role == 10u || o.lod_role == 11u) {
        // the near draws cast while the landmark is near (the JS's one
        // shadow draw); the far item, the skirt and open ground (kind 10) never.
        caster = base_caster && kind == 9u && o.lod_role == 10u && groups[o.lod_group].detailed == 1u;
        mid_caster = caster;
        far_caster = caster;
    } else if (kind == 10u) {
        caster = false; // open ground receives only
        mid_caster = false;
        far_caster = false;
    }
    for (var c = 0u; c < 3u; c = c + 1u) {
        if ((sp.mask & (1u << c)) == 0u) {
            continue;
        }
        var inside = select(select(caster, mid_caster, c == 1u), far_caster, c == 2u);
        if (inside) {
            for (var p = 0u; p < 6u; p = p + 1u) {
                let pl = sp.planes[c * 6u + p];
                let v = select(o.min, o.max, pl.xyz >= vec3<f32>(0.0));
                if (dot(pl.xyz, v) + pl.w < 0.0) {
                    inside = false;
                }
            }
        }
        if (sp.compact == 1u) {
            if (inside) {
                let slot = atomicAdd(&counters[c], 1u);
                write_cmd(c * n + slot, count, 1u, first, o.base_vertex, i);
                atomicAdd(&counters[3u + c], count / 3u);
            }
        } else {
            write_cmd(c * n + i, count, select(0u, 1u, inside), first, o.base_vertex, i);
            if (inside) {
                atomicAdd(&counters[c], 1u);
                atomicAdd(&counters[3u + c], count / 3u);
            }
        }
    }
}
