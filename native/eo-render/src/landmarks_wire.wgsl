// the wire members (main cables, suspender ropes), appended to the
// city shader (draw.wgsl + look + landmarks_surface). Group 0 is the city's
// draw group, group 1 the look's, group 2 the LOD groups (the near/far
// switch the cull pass wrote this frame).
//
// landmark-surface.js `applyWireCoverage`: the member is a prism at the
// envelope radius; a pixel keeps the rope where its view ray passes within
// max(true radius, LM_WIRE_MIN_PIXELS / 2 of a pixel) of the axis, with the
// share of that width the rope truly covers as its opacity, lit with the
// member's own normal (perpendicular to the axis, toward the eye).

struct LmGroup {
    center: vec3<f32>,
    radius: f32,
    ready: u32,
    detailed: u32,
    kind: u32,
    level: u32,
};

@group(2) @binding(0) var<storage, read> lm_groups: array<LmGroup>;

struct LmWireOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) axis: vec3<f32>,
    @location(2) dir: vec3<f32>,
    @location(3) @interpolate(flat) radius: f32,
    @location(4) color: vec3<f32>,
    @location(5) @interpolate(flat) material: u32,
};

@vertex
fn lm_wire_vs(
    @location(0) pos: vec3<f32>,
    @location(1) radius: f32,
    @location(2) axis: vec3<f32>,
    @location(3) group: u32,
    @location(4) dir: vec3<f32>,
    @location(5) color: vec4<f32>,
    @location(6) material: u32,
) -> LmWireOut {
    var out: LmWireOut;
    out.clip = frame.view_proj * vec4<f32>(pos, 1.0);
    if (lm_groups[group].detailed == 0u) {
        // The landmark is far: its far item draws the cables.
        out.clip = vec4<f32>(0.0);
    }
    out.world = pos;
    out.axis = axis;
    out.dir = dir;
    out.radius = radius;
    out.color = srgb_decode(color.rgb);
    out.material = material;
    return out;
}

struct LmWireFrag {
    @location(0) color: vec4<f32>,
    // LM_WIRE_ROUND (landmarks.rs `ROUND_WIRES`): the depth of the point on
    // the view ray nearest the member's axis, not the envelope prism's, so a
    // railing in front hides a rope behind it; else the prism's own depth.
    @builtin(frag_depth) depth: f32,
};

@fragment
fn lm_wire_fs(in: LmWireOut) -> LmWireFrag {
    let cam = frame.cam_pos.xyz;
    // Metres per pixel at the fragment's depth: depth x |d ndc_y / d pixel| / P11
    // (the NDC derivative is 2 / the target height; P11 is the length of the
    // clip matrix's second row). Derivatives first, in uniform control flow.
    let clip = frame.view_proj * vec4<f32>(in.world, 1.0);
    let ndc_y = clip.y / clip.w;
    let dndc = abs(dpdy(ndc_y)) + abs(dpdx(ndc_y));
    let vp = frame.view_proj;
    let p11 = length(vec3<f32>(vp[0][1], vp[1][1], vp[2][1]));
    let depth = max(clip.w, 0.05);
    let mpp = depth * dndc / max(p11, 1e-6);
    let dir = normalize(in.dir);
    let to_eye = cam - in.axis;
    let ray = normalize(in.world - cam);
    let across = cross(ray, dir);
    let miss = abs(dot(to_eye, across)) / max(length(across), 1e-6);
    let drawn = min(max(in.radius, mpp * LM_WIRE_MIN_PIXELS * 0.5), LM_WIRE_ENVELOPE_M);
    if (!(in.radius > 0.0 && miss <= drawn)) {
        discard;
    }
    let opacity = clamp(in.radius / max(drawn, 1e-5), 0.0, 1.0);
    let facing = normalize(to_eye - dir * dot(to_eye, dir));
    var out: LmWireFrag;
    out.depth = in.clip.z;
    var nrm = facing;
    if (LM_WIRE_ROUND) {
        // the nearest points of the view ray and the axis (two lines).
        let b = dot(ray, dir);
        let w0 = cam - in.axis;
        let den = max(1.0 - b * b, 1e-6);
        let t = (b * dot(dir, w0) - dot(ray, w0)) / den;
        let p = cam + ray * max(t, 0.0);
        let q = in.axis + dir * dot(p - in.axis, dir);
        let pc = frame.view_proj * vec4<f32>(p, 1.0);
        out.depth = clamp(pc.z / max(pc.w, 1e-6), 0.0, 1.0);
        // a cylinder's normal where the ray crosses it, `u` of the drawn radius off the axis.
        let u = clamp(miss / max(drawn, 1e-5), 0.0, 1.0);
        let lat = p - q;
        let ll = length(lat);
        if (ll > 1e-6) {
            nrm = normalize(facing * sqrt(max(1.0 - u * u, 0.0)) + (lat / ll) * u);
        }
    }
    let m = materials[in.material];
    var s: Surface;
    s.albedo = in.color;
    s.rough = clamp(m.params.x, 0.03, 1.0);
    s.metal = m.params.y;
    s.ao = 1.0;
    s.n = nrm;
    s.alpha = opacity;
    s.env = ENV_PAINTED_METAL;
    let v = normalize(cam - in.world);
    let c = look_light(s, m, v, in.world, nrm, 0u);
    out.color = vec4<f32>(c.rgb, opacity);
    return out;
}
