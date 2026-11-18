// the street overhead's wires (street-overhead.js `createWireMaterial`),
// appended to props_types.wgsl (cull: and occlusion.wgsl in the Hi-Z variant;
// draw: and look_common.wgsl). Every wire is a polyline of the JS's own
// catenary samples, uploaded per street tile as segments (a, b, true radius).
//
// Cull (one invocation per segment slot): distance (the JS fade far), the
// camera frustum and, in the occlusion pass, the city's Hi-Z test; each slot
// gets a visibility flag (and a drawn count for the stats). ONE non-indexed
// indirect draw of 6 vertices an instance runs over every slot in slot order
// (a culled slot is a degenerate quad): the blend order of overlapping
// wires is then fixed, so two frames that see the same wires are bit-equal
// (an appended list's order depends on the atomics' scheduling).
//
// Draw: a camera-facing ribbon per segment, its half width widened in the
// vertex stage to the JS floor (`pixelHalfWidthRad` x distance, about 1.6 px)
// and its opacity reduced by exactly the widening ratio (analytic coverage,
// floored at the JS `minCoverage`), faded out between the JS fade distances.
// Unlit `WIRE_COLOUR` as the JS draws it, fogged; no shadows (as the JS).

struct PrWire {
    a: vec3<f32>,
    radius: f32,
    b: vec3<f32>,
    kind: u32,
};

#ifdef_cull
@group(0) @binding(0) var<uniform> pr: PrParams;
@group(0) @binding(1) var<storage, read> pr_wires: array<PrWire>;
@group(0) @binding(2) var<storage, read_write> pr_wire_vis: array<u32>;
// [0..4) the DrawIndirect (6, slots, 0, 0); [4] segments drawn (stats).
@group(0) @binding(3) var<storage, read_write> pr_wire_args: array<atomic<u32>, 8>;

@compute @workgroup_size(64)
fn wire_cull(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= pr.extra.x) {
        return;
    }
    pr_wire_vis[i] = 0u;
    let w = pr_wires[i];
    if (w.radius <= 0.0) {
        return;
    }
    let c = (w.a + w.b) * 0.5;
    let r = length(w.b - w.a) * 0.5 + 0.3;
    if (distance(c, pr.cam.xyz) - r > pr.wire.y) {
        return;
    }
    for (var p = 0u; p < 6u; p = p + 1u) {
        let pl = pr.planes[p];
        if (dot(pl.xyz, c) + pl.w < -r) {
            return;
        }
    }
    let pad = vec3<f32>(0.3);
    if (pr_occluded(min(w.a, w.b) - pad, max(w.a, w.b) + pad)) {
        return;
    }
    atomicAdd(&pr_wire_args[4], 1u);
    pr_wire_vis[i] = 1u;
}
#endif_cull

#ifdef_draw
@group(0) @binding(0) var<uniform> pr: PrParams;
@group(0) @binding(1) var<storage, read> pr_wires: array<PrWire>;
@group(0) @binding(2) var<storage, read> pr_wire_vis: array<u32>;
@group(1) @binding(0) var<uniform> look: Look;

// street-overhead.js `WIRE_COLOUR`.
const PR_WIRE_COLOUR: vec3<f32> = vec3<f32>(0.05, 0.054, 0.06);

struct PrWireOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) side: f32,
    @location(1) @interpolate(flat) alpha: f32,
    @location(2) world: vec3<f32>,
};

@vertex
fn wire_vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> PrWireOut {
    let w = pr_wires[ii];
    if (pr_wire_vis[ii] == 0u) {
        var none: PrWireOut;
        none.clip = vec4<f32>(0.0, 0.0, -2.0, 1.0);
        return none;
    }
    // Two triangles: (t, side) per corner.
    var ts = array<vec2<f32>, 6>(vec2<f32>(0.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0));
    let q = ts[vi % 6u];
    let p = mix(w.a, w.b, q.x);
    let to_eye = pr.cam.xyz - p;
    let dist = max(length(to_eye), 0.05);
    let view = to_eye / dist;
    let axis = cross(normalize(w.b - w.a), view);
    let unit = axis / max(length(axis), 1e-4);
    let half_w = max(w.radius, dist * pr.wind.w);
    let world = p + unit * q.y * half_w;
    var out: PrWireOut;
    out.clip = pr.view_proj * vec4<f32>(world, 1.0);
    out.side = q.y;
    let coverage = clamp(w.radius / half_w, pr.wire.z, 1.0);
    let mid = distance((w.a + w.b) * 0.5, pr.cam.xyz);
    out.alpha = coverage * (1.0 - smoothstep(pr.wire.x, pr.wire.y, mid));
    out.world = world;
    return out;
}

@fragment
fn wire_fs(in: PrWireOut) -> @location(0) vec4<f32> {
    let edge = 1.0 - smoothstep(0.5, 1.0, abs(in.side));
    let vd = dot(in.world - look.cam_pos.xyz, look.cam_fwd.xyz);
    let fog = lk_smooth(look.fog.x, look.fog.y, vd);
    let c = mix(PR_WIRE_COLOUR, look.fog_color.xyz, fog);
    return vec4<f32>(c, in.alpha * edge);
}
#endif_draw
