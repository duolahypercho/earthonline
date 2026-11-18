// Moving things: the agent vertex stages' bindings (group 2) and posing,
// shared by the colour pass and the sun shadow pass (appended to
// agents_types.wgsl).
//
// A draw's instance index (first_instance + i, written by agents_cull.wgsl)
// selects an entry of the visible list: the agent index in the low 28 bits,
// the level of detail it was drawn at in the top 4. The agent gives the
// model, pose and animation state; a person is skinned from the baked joint
// matrices (two frames of the walk and two of the standing clip, blended by
// ground speed); a car's wheel parts spin about their pivot's x axis and the
// front ones steer about y.

@group(2) @binding(0) var<uniform> ag: AgParams;
@group(2) @binding(1) var<storage, read> ag_agents: array<AgAgent>;
@group(2) @binding(2) var<storage, read> ag_models: array<AgModel>;
@group(2) @binding(3) var<storage, read> ag_meshes: array<AgMesh>;
@group(2) @binding(4) var<storage, read> ag_bones: array<vec4<f32>>;
@group(2) @binding(5) var<storage, read> ag_visible: array<u32>;

struct AgPose {
    world: vec3<f32>,
    normal: vec3<f32>,
    // body-frame position (the indicator side, the lamp lenses)
    local: vec3<f32>,
    agent: u32,
};

// One joint's skinning matrix, rows r0..r2, blended over two frames of the
// walk and two of the standing clip.
fn ag_joint_rows(base: u32, joints: u32, j: u32, wf0: u32, wf1: u32, wt: f32, cf0: u32, cf1: u32, ct: f32, ww: f32) -> mat3x4<f32> {
    var r: mat3x4<f32>;
    for (var k = 0u; k < 3u; k = k + 1u) {
        let w0 = ag_bones[(base + wf0 * joints + j) * 3u + k];
        let w1 = ag_bones[(base + wf1 * joints + j) * 3u + k];
        let c0 = ag_bones[(base + cf0 * joints + j) * 3u + k];
        let c1 = ag_bones[(base + cf1 * joints + j) * 3u + k];
        r[k] = mix(mix(c0, c1, ct), mix(w0, w1, wt), ww);
    }
    return r;
}

fn ag_pose(entry: u32, pos: vec3<f32>, normal: vec3<f32>, joints: vec4<u32>, weights: vec4<f32>, tag: u32) -> AgPose {
    let index = entry & 0x0fffffffu;
    let lod = entry >> 28u;
    let a = ag_agents[index];
    let m = ag_models[a.model];
    var p = pos;
    var n = normal;
    if (m.model_class == AG_PERSON) {
        let mesh = ag_meshes[m.meshes[lod]];
        let jn = mesh.joints;
        // Walk: the clip's own phase (0 = left heel strike), one stride a cycle.
        let wn = ag.clip_frames.x;
        let wf = fract(a.anim) * f32(wn);
        let wf0 = u32(floor(wf)) % wn;
        let wf1 = (wf0 + 1u) % wn;
        let wt = fract(wf);
        // Standing clip.
        let c = min(a.clip, 3u);
        let cn = ag.clip_frames[c];
        let cb = ag.clip_base[c];
        let cf = fract(a.clip_phase) * f32(cn);
        let cf0 = cb + u32(floor(cf)) % cn;
        let cf1 = cb + (u32(floor(cf)) + 1u) % cn;
        let ct = fract(cf);
        // locomotionBlend's walk ramp (smoothstep 0.15 .. 0.60 m/s).
        let ww = smoothstep(0.15, 0.60, a.speed);
        let base = mesh.skeleton;
        let wb = ag.clip_base.x;
        var sp = vec3<f32>(0.0);
        var sn = vec3<f32>(0.0);
        let p4 = vec4<f32>(pos, 1.0);
        let n4 = vec4<f32>(normal, 0.0);
        for (var i = 0u; i < 4u; i = i + 1u) {
            let w = weights[i];
            if (w > 0.0) {
                let r = ag_joint_rows(base, jn, joints[i], wb + wf0, wb + wf1, wt, cf0, cf1, ct, ww);
                sp = sp + w * vec3<f32>(dot(r[0], p4), dot(r[1], p4), dot(r[2], p4));
                sn = sn + w * vec3<f32>(dot(r[0], n4), dot(r[1], n4), dot(r[2], n4));
            }
        }
        p = sp * a.scale;
        n = sn;
    } else {
        let wheel = (tag >> 16u) & 255u;
        if (wheel > 0u) {
            let wi = (wheel - 1u) % 4u;
            let piv = m.wheel[wi].xyz;
            var q = p - piv;
            var qn = n;
            if (wheel <= 4u) {
                // Spin about the pivot's x axis (rotation.x = spin, three's convention).
                let cs = cos(a.anim);
                let sn = sin(a.anim);
                q = vec3<f32>(q.x, q.y * cs - q.z * sn, q.y * sn + q.z * cs);
                qn = vec3<f32>(qn.x, qn.y * cs - qn.z * sn, qn.y * sn + qn.z * cs);
            }
            if (wi < 2u) {
                // Front wheels steer about y (rotation.y = steer).
                let cs = cos(a.steer);
                let sn = sin(a.steer);
                q = vec3<f32>(q.x * cs + q.z * sn, q.y, -q.x * sn + q.z * cs);
                qn = vec3<f32>(qn.x * cs + qn.z * sn, qn.y, -qn.x * sn + qn.z * cs);
            }
            p = q + piv;
            n = qn;
        }
    }
    var out: AgPose;
    out.local = p;
    // a vehicle drawn at its instance scale (1.0 except in the trial
    // switch; a person's skinned position above already carries it).
    var pw = p;
    if (m.model_class != AG_PERSON) {
        pw = p * a.scale;
    }
    out.world = a.pos + ag_rotate(a.rot, pw);
    out.normal = ag_rotate(a.rot, n);
    out.agent = index;
    return out;
}
