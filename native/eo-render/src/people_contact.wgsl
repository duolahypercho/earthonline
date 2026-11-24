// Appended to the agents' colour shader (agents.rs) only with `--people-contact`:
// its own pipeline, drawn after the agents' opaque list in the city pass.

// ---- the ground a person stands on, occluded by them (`--people-contact`;
// agents.rs `SF_PEOPLE_CONTACT`, its own pipeline drawn after the agents) ----
// A person's body hides part of the sky (and the rest of the ambient light)
// from the ground round them: most at the shoes' edges, a faint pool under
// the legs and hips. Each person is a few spheres riding their skeleton (eo-agents
// `people::add_contact_anchors`: the centroid and RMS radius of the vertices
// each part's joints carry, in the bind pose, posed by the same joint matrices the
// body is skinned with: heels and toes, shins, thighs, the pelvis), and a
// ground point's sky visibility is the product over them of 1 - the sphere's
// cosine-weighted solid angle, (d.n / |d|) (r / |d|)^2 (exact for a sphere
// above the horizon; 1 inside it). That occlusion takes away the ambient
// share of the ground's light only (the sky and environment over the sun's
// shadowed key and the street lights' own, which carry their own shadows):
// a quad on the ground round each person, within `PC_REACH_M` of the eye,
// multiplies the lit frame by 1 - (1 - visibility) x that share x (1 - fog).
const PC_PARTS: u32 = 9u;
// the joint posing each anchor (eo-agents people `CONTACT_JOINTS`): the left
// shoe's heel and toe halves on Bip01 L Foot, the right's on R Foot, the
// calves, the thighs, the pelvis.
const PC_JOINTS: array<u32, 9> = array<u32, 9>(49u, 49u, 53u, 53u, 48u, 52u, 47u, 51u, 0u);
const PC_EXTENT_M: f32 = 1.2;
const PC_REACH_M: f32 = 90.0;
const PC_LIFT_M: f32 = 0.03;

struct PcOut {
    @builtin(position) clip: vec4<f32>,
    // xyz the quad's point, w the ground's height under the person
    @location(0) world: vec4<f32>,
    @location(1) @interpolate(flat) s0: vec4<f32>,
    @location(2) @interpolate(flat) s1: vec4<f32>,
    @location(3) @interpolate(flat) s2: vec4<f32>,
    @location(4) @interpolate(flat) s3: vec4<f32>,
    @location(5) @interpolate(flat) s4: vec4<f32>,
    @location(6) @interpolate(flat) s5: vec4<f32>,
    @location(7) @interpolate(flat) s6: vec4<f32>,
    @location(8) @interpolate(flat) s7: vec4<f32>,
    @location(9) @interpolate(flat) s8: vec4<f32>,
};

// A bind-pose point carried by joint `j`, posed as ag_pose poses the body
// (the walk and standing frames blended by ground speed), in world space.
fn pc_posed(a: AgAgent, m: AgModel, j: u32, p: vec3<f32>) -> vec3<f32> {
    let wn = ag.clip_frames.x;
    let wf = fract(a.anim) * f32(wn);
    let wf0 = u32(floor(wf)) % wn;
    let wf1 = (wf0 + 1u) % wn;
    let c = min(a.clip, 3u);
    let cn = ag.clip_frames[c];
    let cb = ag.clip_base[c];
    let cf = fract(a.clip_phase) * f32(cn);
    let cf0 = cb + u32(floor(cf)) % cn;
    let cf1 = cb + (u32(floor(cf)) + 1u) % cn;
    let ww = smoothstep(0.15, 0.60, a.speed);
    let wb = ag.clip_base.x;
    let r = ag_joint_rows(m.skeleton, m.joints, j, wb + wf0, wb + wf1, fract(wf), cf0, cf1, fract(cf), ww);
    let p4 = vec4<f32>(p, 1.0);
    let q = vec3<f32>(dot(r[0], p4), dot(r[1], p4), dot(r[2], p4)) * a.scale;
    return a.pos + ag_rotate(a.rot, q);
}

@vertex
fn agent_contact_vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> PcOut {
    var out: PcOut;
    out.clip = vec4<f32>(2.0, 2.0, 2.0, 1.0); // nothing drawn
    if (ii >= ag.counts.x) {
        return out;
    }
    let a = ag_agents[ii];
    let m = ag_models[a.model];
    let anchors = m.pad.x;
    if (m.model_class != AG_PERSON || m.pad.y < PC_PARTS || distance(a.pos, frame.cam_pos.xyz) > PC_REACH_M) {
        return out;
    }
    var s: array<vec4<f32>, 9>;
    var c = vec2<f32>(0.0);
    var joints = PC_JOINTS;
    for (var k = 0u; k < PC_PARTS; k = k + 1u) {
        let b = ag_bones[anchors + k];
        s[k] = vec4<f32>(pc_posed(a, m, joints[k], b.xyz), b.w * a.scale);
        if (k < 4u) {
            c = c + s[k].xz * 0.25;
        }
    }
    var corner = array<vec2<f32>, 6>(vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0));
    let xz = c + corner[vi % 6u] * PC_EXTENT_M;
    let w = vec3<f32>(xz.x, a.pos.y + PC_LIFT_M, xz.y);
    out.clip = frame.view_proj * vec4<f32>(w, 1.0);
    out.world = vec4<f32>(w, a.pos.y);
    out.s0 = s[0];
    out.s1 = s[1];
    out.s2 = s[2];
    out.s3 = s[3];
    out.s4 = s[4];
    out.s5 = s[5];
    out.s6 = s[6];
    out.s7 = s[7];
    out.s8 = s[8];
    return out;
}

// 1 - a sphere's cosine-weighted solid angle seen from ground point g (normal up).
fn pc_sphere(g: vec3<f32>, s: vec4<f32>) -> f32 {
    let d = s.xyz - g;
    let l2 = dot(d, d);
    let r2 = s.w * s.w;
    if (l2 <= r2) {
        return 0.0;
    }
    let l = sqrt(l2);
    return 1.0 - clamp(d.y / l, 0.0, 1.0) * r2 / l2;
}

fn pc_factor(in: PcOut) -> f32 {
    let g = vec3<f32>(in.world.x, in.world.w, in.world.z);
    var vis = pc_sphere(g, in.s0) * pc_sphere(g, in.s1) * pc_sphere(g, in.s2) * pc_sphere(g, in.s3) * pc_sphere(g, in.s4);
    vis = vis * pc_sphere(g, in.s5) * pc_sphere(g, in.s6) * pc_sphere(g, in.s7) * pc_sphere(g, in.s8);
    if (vis >= 0.999) {
        return 1.0;
    }
    // the ground's light: the ambient (sky, environment) and what carries its own shadow
    let up = vec3<f32>(0.0, 1.0, 0.0);
    let env_i = look.ambient.w * look.env_class.w;
    let e_amb = lk_luma(look.hemi_sky.xyz + look.ambient.xyz + lk_sh(up) * env_i);
    var e_key = 0.0;
    if (look.key_dir.y > 0.0) {
        e_key = lk_luma(look.key.xyz) * look.key_dir.y * lk_shadow(g, up) * sky_cloud_transmittance(g);
    }
    let v = normalize(frame.cam_pos.xyz - g);
    let e_lamps = LK_PI * lk_luma(sky_local_lights(g, up, v, vec3<f32>(1.0), vec3<f32>(0.0), 1.0));
    let share = e_amb / max(e_amb + e_key + e_lamps, 1e-6);
    let vd = dot(g - look.cam_pos.xyz, look.cam_fwd.xyz);
    let fog = lk_smooth(look.fog.x, look.fog.y, vd);
    return 1.0 - (1.0 - vis) * share * (1.0 - fog);
}

@fragment
fn agent_contact_fs(in: PcOut) -> @location(0) vec4<f32> {
    return vec4<f32>(vec3<f32>(pc_factor(in)), 1.0);
}

@fragment
fn agent_contact_fs_ids(in: PcOut) -> AgIdsOut {
    var out: AgIdsOut;
    out.color = vec4<f32>(vec3<f32>(pc_factor(in)), 1.0);
    out.ids = vec2<u32>(0u, 0u);
    return out;
}
