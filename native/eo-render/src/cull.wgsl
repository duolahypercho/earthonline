// LOD and culling. Two entry points in one compute pass:
//
//  lod:  one invocation per Blender building (LOD group). Distance from the
//        camera to the group's centre minus its bounds radius, against the
//        JS HLOD radii with hysteresis (enter 320 m, leave 380 m:
//        AUTHORED_BUILDING_HLOD_* in src/citygen/renderer.js).
//  cull: one invocation per object. Applies the object's LOD role, then the
//        frustum test, and writes a DrawIndexedIndirect command (5 x u32)
//        into the opaque list [0, n) or the transparent list [n, 2n).
//        With `compact == 1` commands are appended at atomic slots and the
//        draws consume the counts (multi_draw_indexed_indirect_count);
//        otherwise every object keeps slot i in both lists with
//        instance_count 0 or 1 (multi_draw_indexed_indirect fallback).
//        first_instance carries the object index for the vertex shader.

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
    // 0: Blender building (HLOD radii); 1: kit building
    kind: u32,
    // kit: level drawn this frame (0 near rung, 1 mid rung, 2 footprint box)
    level: u32,
};

struct CullParams {
    planes: array<vec4<f32>, 6>,
    // xyz camera, w terrain LOD distance
    cam: vec4<f32>,
    object_count: u32,
    compact: u32,
    group_count: u32,
    blend_base: u32,
    hlod_enter: f32,
    hlod_exit: f32,
    // Fallback only: objects before this index never draw transparent and
    // have no slot in the transparent list.
    blend_first: u32,
    pad1: u32,
    // kit LOD radii to the bounds: near enter, near exit, mid enter, mid exit
    kit: vec4<f32>,
    // street LOD radii to the bounds: near enter, near exit, mid enter, mid exit
    street: vec4<f32>,
    // the camera's clip matrix (the one the city pass draws with)
    view_proj: mat4x4<f32>,
    // x = 1 two-phase occlusion culling on, y = late opaque list base (slots)
    occ: vec4<u32>,
};

// made with the pyramid (occlusion.rs): the depth target's size.
struct HzbInfo {
    screen: vec4<f32>,
};

const LOD_ALWAYS: u32 = 0u;
const LOD_NEAR: u32 = 1u;
const LOD_FAR: u32 = 2u;
const LOD_TERRAIN: u32 = 3u;
const BLEND_BIT: u32 = 256u;
const KIND_BUILDING: u32 = 0u;
const KIND_MODEL: u32 = 4u;
const KIND_KIT: u32 = 5u;
// Kit building roles: drawn while the group's level is 0, 1 or 2.
const LOD_KIT_NEAR: u32 = 4u;
const LOD_KIT_MID: u32 = 5u;
const LOD_KIT_BOX: u32 = 6u;
// Street tile roles: drawn while the group's level is 0 (near pack), 1 (far strips), 2 (coarse).
const LOD_STREET_NEAR: u32 = 7u;
const LOD_STREET_COARSE: u32 = 9u;
const KIND_STREET: u32 = 6u;
const KIND_STREET_PAINT: u32 = 7u;
const KIND_SEA: u32 = 3u; // water
// roles of a landmark's draws (scene.rs); LM_NEAR_M and
// LM_HYSTERESIS_M are appended by lib.rs from eo-landmarks' export.
const LOD_LANDMARK_NEAR: u32 = 10u;
const LOD_LANDMARK_FAR: u32 = 11u;

@group(0) @binding(0) var<uniform> params: CullParams;
@group(0) @binding(1) var<storage, read> objects: array<Object>;
@group(0) @binding(2) var<storage, read_write> draws: array<u32>;
// [0] opaque draws, [1] transparent draws, [2] triangles, [3] model parts,
// [4] model triangles, [5] footprint stand-ins of Blender buildings,
// [6] kit rung objects drawn, [7] kit rung triangles drawn,
// [8] street objects drawn, [9] street triangles drawn,
// [10] late opaque draws, [11] objects in the frustum (after LOD),
// [12] triangles in the frustum, [13] objects drawn by the early pass,
// [14] objects the late pass found occluded, [15] pad
@group(0) @binding(3) var<storage, read_write> counters: array<atomic<u32>, 16>;
@group(0) @binding(4) var<storage, read_write> groups: array<Group>;
// 1 if the object passed the occlusion test last frame (the early
// pass draws exactly these); rewritten by the late pass every frame.
@group(0) @binding(5) var<storage, read_write> visible_last: array<u32>;
// the hierarchical Z of this frame's early depth (late pass only;
// its own bind group so the early pass and LOD pass do not need it).
@group(1) @binding(0) var hzb: texture_2d<f32>;
@group(1) @binding(1) var<uniform> hzb_info: HzbInfo;
// the far layer (hlod.rs). Per LOD group its neighbourhood cell (or
// none), written once; per cell whether the cell (bit 0) and its district
// (bit 1) are resident, written by the far layer's streamer; per group what
// the far layer draws this frame, written by `lod` below and read by the far
// layer's draw and by draw.wgsl: bit 0 the cell, bit 1 the district, bit 2 a
// cross-fade with the building's own (near) level; bits 8-15 the farther
// side's share (0..255) of whichever two are drawn. HLOD_ON, HLOD_FAR_A and
// HLOD_FAR_B (the cell -> district band) are appended by lib.rs.
@group(0) @binding(6) var<storage, read> hlod_map: array<u32>;
@group(0) @binding(7) var<storage, read_write> hlod_state: array<u32>;
@group(0) @binding(8) var<storage, read> hlod_flags: array<u32>;
const HLOD_CELL: u32 = 1u;
const HLOD_DISTRICT: u32 = 2u;
const HLOD_NEAR_FADE: u32 = 4u;
// a kit group's level while the far layer alone draws it (no rung, no box).
const HLOD_LEVEL: u32 = 4u;

fn hlod_share(d: f32, a: f32, b: f32) -> u32 {
    return u32(round(clamp((d - a) / max(b - a, 1e-3), 0.0, 1.0) * 255.0)) << 8u;
}

// the far level of group i at distance d, ignoring its own levels: the
// cell, the district, or both in the band between them (with the share);
// 0 when neither is resident.
fn hlod_far(i: u32, d: f32) -> u32 {
    let c = hlod_map[i];
    if (!HLOD_ON || c == 0xffffffffu) {
        return 0u;
    }
    let f = hlod_flags[c];
    let cell = (f & 1u) != 0u;
    let district = (f & 2u) != 0u;
    if (cell && district && d > HLOD_FAR_A && d < HLOD_FAR_B) {
        return HLOD_CELL | HLOD_DISTRICT | hlod_share(d, HLOD_FAR_A, HLOD_FAR_B);
    }
    if (cell && (d <= HLOD_FAR_A || !district)) {
        return HLOD_CELL;
    }
    return select(0u, HLOD_DISTRICT, district);
}

@compute @workgroup_size(64)
fn lod(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= params.group_count) {
        return;
    }
    let g = groups[i];
    let d = max(0.0, distance(params.cam.xyz, g.center) - g.radius);
    if (g.kind == 3u) {
        // near while the camera is within LM_NEAR_M of the
        // landmark's box, with LM_HYSTERESIS_M (landmark-set.js `update`). The
        // box's half extents: radius, then `ready` and `level` bit cast.
        let half = vec3<f32>(g.radius, bitcast<f32>(g.ready), bitcast<f32>(g.level));
        let q = max(abs(params.cam.xyz - g.center) - half, vec3<f32>(0.0));
        let nearest = length(q);
        let near = select(nearest < LM_NEAR_M - LM_HYSTERESIS_M, nearest < LM_NEAR_M + LM_HYSTERESIS_M, g.detailed == 1u);
        groups[i].detailed = select(0u, 1u, near);
        return;
    }
    if (g.kind == 2u) {
        // Street tile: the near pack within the near radius when resident,
        // the far strips to the mid radius, the coarse strips past it.
        let near_r = select(params.street.x, params.street.y, g.detailed == 0u);
        let mid_r = select(params.street.z, params.street.w, g.detailed <= 1u);
        var zone = 2u;
        if (d <= near_r) {
            zone = 0u;
        } else if (d <= mid_r) {
            zone = 1u;
        }
        var level = zone;
        if (zone == 0u && (g.ready & 1u) == 0u) {
            level = 1u;
        }
        groups[i].detailed = zone;
        groups[i].level = level;
        return;
    }
    if (g.kind == 1u) {
        // Kit building: the distance zone with hysteresis, then the level
        // drawn: the zone's rung if resident, else the other rung if
        // resident, else the footprint box. Exactly one level per group.
        let near_r = select(params.kit.x, params.kit.y, g.detailed == 0u);
        let mid_r = select(params.kit.z, params.kit.w, g.detailed <= 1u);
        var zone = 2u;
        if (d <= near_r) {
            zone = 0u;
        } else if (d <= mid_r) {
            zone = 1u;
        }
        let near_ok = (g.ready & 1u) != 0u;
        let mid_ok = (g.ready & 2u) != 0u;
        var level = 2u;
        if (zone == 0u) {
            level = select(select(2u, 1u, mid_ok), 0u, near_ok);
        } else if (zone == 1u) {
            level = select(select(2u, 0u, near_ok), 1u, mid_ok);
        }
        // with a far level resident the building is never a box. Between
        // the mid band's enter and exit radii (the zone hysteresis) the mid rung
        // and the far level are both drawn, the far share growing with the
        // distance, each pixel showing one of them (complementary screen-door
        // dither): continuous in distance, no pop either way. Past the band, or
        // with no rung resident, the far level alone. With none resident the
        // mid rung stays drawn while it is resident, and only then the box.
        if (HLOD_ON) {
            let far = hlod_far(i, d);
            var hl = 0u;
            if (far != 0u) {
                if (mid_ok && zone != 0u && d > params.kit.z && d < params.kit.w) {
                    level = 1u;
                    hl = (far & (HLOD_CELL | HLOD_DISTRICT)) | HLOD_NEAR_FADE | hlod_share(d, params.kit.z, params.kit.w);
                } else if (level == 2u) {
                    level = HLOD_LEVEL;
                    hl = far;
                }
            } else if (level == 2u && mid_ok) {
                level = 1u;
            }
            hlod_state[i] = hl;
        }
        groups[i].detailed = zone;
        groups[i].level = level;
        return;
    }
    let r = select(params.hlod_enter, params.hlod_exit, g.detailed == 1u);
    let detailed = d <= r;
    groups[i].detailed = select(0u, 1u, detailed);
    // a Blender-built (or code-built) building: the model and the far
    // level cross-fade between the JS HLOD radii; the far level stands in for
    // the footprint wherever the model is not drawn.
    if (HLOD_ON) {
        let far = hlod_far(i, d);
        let ready = g.ready == 1u;
        var hl = 0u;
        if (far != 0u) {
            if (ready && d > params.hlod_enter && d < params.hlod_exit) {
                hl = (far & (HLOD_CELL | HLOD_DISTRICT)) | HLOD_NEAR_FADE | hlod_share(d, params.hlod_enter, params.hlod_exit);
            } else if (!(ready && detailed)) {
                hl = far;
            }
        }
        hlod_state[i] = hl;
    }
}

fn write_cmd(slot: u32, count: u32, instances: u32, first: u32, base_vertex: i32, object: u32) {
    let b = slot * 5u;
    draws[b] = count;
    draws[b + 1u] = instances;
    draws[b + 2u] = first;
    draws[b + 3u] = bitcast<u32>(base_vertex);
    draws[b + 4u] = object;
}

// what the LOD role and the frustum say about object i (shared by
// the early and late passes): x = visible, y = first index, z = index count.
fn lod_and_frustum(i: u32, o: Object) -> vec3<u32> {
    var first = o.first_index;
    var count = o.index_count;
    var visible = true;
    if (o.lod_role == LOD_NEAR || o.lod_role == LOD_FAR) {
        let g = groups[o.lod_group];
        let near = g.ready == 1u && g.detailed == 1u;
        // in a cross-fade the model draws whatever its zone; the footprint
        // stands down wherever the far layer draws the building.
        let hl = select(0u, hlod_state[o.lod_group], HLOD_ON);
        visible = select(!near && hl == 0u, near || (hl & HLOD_NEAR_FADE) != 0u, o.lod_role == LOD_NEAR);
    } else if (o.lod_role >= LOD_KIT_NEAR && o.lod_role <= LOD_KIT_BOX) {
        visible = groups[o.lod_group].level == o.lod_role - LOD_KIT_NEAR;
    } else if (o.lod_role >= LOD_STREET_NEAR && o.lod_role <= LOD_STREET_COARSE) {
        visible = groups[o.lod_group].level == o.lod_role - LOD_STREET_NEAR;
    } else if (o.lod_role == LOD_LANDMARK_NEAR || o.lod_role == LOD_LANDMARK_FAR) {
        // the near draws or the far item, by the group's switch.
        visible = (groups[o.lod_group].detailed == 1u) == (o.lod_role == LOD_LANDMARK_NEAR);
    } else if (o.lod_role == LOD_TERRAIN) {
        // Camera to the tile box.
        let c = clamp(params.cam.xyz, o.min, o.max);
        if (distance(c, params.cam.xyz) > params.cam.w) {
            first = o.far_first_index;
            count = o.far_index_count;
        }
    }
    if ((o.kind & 255u) == 13u) {
        // The dive sites contain centimetre-scale relief, hidden at distance by water.
        visible = distance(params.cam.xyz, clamp(params.cam.xyz, o.min, o.max)) < 58.0;
    }
    if (count == 0u) {
        visible = false;
    }
    // with the water pass on, the flat sea quad stands down (the probe keeps it).
    if ((o.kind & 255u) == KIND_SEA && params.occ.z == 1u) {
        visible = false;
    }
    if (visible) {
        for (var p = 0u; p < 6u; p = p + 1u) {
            let pl = params.planes[p];
            let v = select(o.min, o.max, pl.xyz >= vec3<f32>(0.0));
            if (dot(pl.xyz, v) + pl.w < 0.0) {
                visible = false;
            }
        }
    }
    return vec3<u32>(select(0u, 1u, visible), first, count);
}

// Counters of a drawn object (kind and triangles).
fn count_drawn(o: Object, count: u32) {
    let tris = count / 3u;
    atomicAdd(&counters[2], tris);
    let kind = o.kind & 255u;
    if (kind == KIND_MODEL) {
        atomicAdd(&counters[3], 1u);
        atomicAdd(&counters[4], tris);
    } else if (kind == KIND_KIT) {
        atomicAdd(&counters[6], 1u);
        atomicAdd(&counters[7], tris);
    } else if (kind == KIND_STREET || kind == KIND_STREET_PAINT) {
        atomicAdd(&counters[8], 1u);
        atomicAdd(&counters[9], tris);
    } else if (kind == KIND_BUILDING && o.lod_role == LOD_FAR) {
        atomicAdd(&counters[5], 1u);
    }
}

// The early pass (and, with occlusion off, the only pass): LOD + frustum,
// then with occlusion on only the objects that passed the occlusion test
// last frame are drawn now (opaque only; glass waits for the late pass).
@compute @workgroup_size(64)
fn cull(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= params.object_count) {
        return;
    }
    let o = objects[i];
    let lf = lod_and_frustum(i, o);
    var visible = lf.x == 1u;
    let first = lf.y;
    let count = lf.z;
    let blend = (o.kind & BLEND_BIT) != 0u;
    let list = select(0u, 1u, blend);
    if (params.compact == 1u) {
        if (!visible) {
            return;
        }
        if (params.occ.x == 1u) {
            // early pass.
            if (blend || visible_last[i] == 0u) {
                return;
            }
            atomicAdd(&counters[13], 1u);
        } else {
            atomicAdd(&counters[11], 1u);
            atomicAdd(&counters[12], count / 3u);
        }
        let slot = atomicAdd(&counters[list], 1u) + select(0u, params.blend_base, blend);
        write_cmd(slot, count, 1u, first, o.base_vertex, i);
    } else {
        // Slot i in the opaque list, slot i - blend_first in the transparent
        // one; only the object's own list may draw it.
        write_cmd(i, count, select(0u, 1u, visible && !blend), first, o.base_vertex, i);
        if (i >= params.blend_first) {
            write_cmd(params.blend_base + i - params.blend_first, count, select(0u, 1u, visible && blend), first, o.base_vertex, i);
        }
        if (!visible) {
            return;
        }
        atomicAdd(&counters[list], 1u);
        atomicAdd(&counters[11], 1u);
        atomicAdd(&counters[12], count / 3u);
    }
    count_drawn(o, count);
}

// the late pass, after the early opaque list was drawn and the
// hierarchical Z built from its depth. Every object in the frustum is tested
// against that depth; the result is next frame's early set. What is visible
// and was not drawn early (newly visible this frame, or glass) is drawn now,
// in the same frame: nothing pops in a frame late.
@compute @workgroup_size(64)
fn cull_late(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= params.object_count) {
        return;
    }
    let o = objects[i];
    let lf = lod_and_frustum(i, o);
    let was = visible_last[i];
    if (lf.x == 0u) {
        if (was != 0u) {
            visible_last[i] = 0u;
        }
        return;
    }
    let first = lf.y;
    let count = lf.z;
    atomicAdd(&counters[11], 1u);
    atomicAdd(&counters[12], count / 3u);
    let occluded = hzb_occluded(o.min, o.max, params.view_proj, hzb_info.screen.xy);
    let now = select(1u, 0u, occluded);
    if (now != was) {
        visible_last[i] = now;
    }
    if (occluded) {
        atomicAdd(&counters[14], 1u);
        return;
    }
    let blend = (o.kind & BLEND_BIT) != 0u;
    if (!blend && was == 1u) {
        // Drawn by the early pass.
        return;
    }
    var slot = 0u;
    if (blend) {
        slot = atomicAdd(&counters[1], 1u) + params.blend_base;
    } else {
        slot = atomicAdd(&counters[10], 1u) + params.occ.y;
    }
    write_cmd(slot, count, 1u, first, o.base_vertex, i);
    count_drawn(o, count);
}
