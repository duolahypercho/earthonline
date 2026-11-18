// street props into the sun shadow cascades (appended to
// props_types.wgsl). Group 0: the look's cascade view (dynamic offset);
// group 2: the props data. Opaque parts depth only; alpha-tested parts
// (leaf cards, props2: sign outlines) discard as the colour pass does. The
// wind sway is the colour pass's own (`pr_wind`), so the shadow follows the tree.

struct PrShadowView {
    view_proj: mat4x4<f32>,
    copy_step: vec4<f32>,
};

struct PrMaterial {
    base: vec4<f32>,
    emissive: vec4<f32>,
    params: vec4<f32>,
    layer: i32,
    flags: u32,
    rough_flatten: f32,
    thin_absorption: f32,
};

const PR_S_BIG: u32 = 32768u;

@group(0) @binding(0) var<uniform> pr_sv: PrShadowView;
@group(2) @binding(0) var<uniform> pr: PrParams;
@group(2) @binding(1) var<storage, read> pr_instances: array<PrInstance>;
@group(2) @binding(2) var<storage, read> pr_visible: array<u32>;
@group(2) @binding(6) var pr_albedo: texture_2d_array<f32>;
@group(2) @binding(7) var pr_sampler: sampler;
@group(2) @binding(8) var<storage, read> pr_materials: array<PrMaterial>;
@group(2) @binding(9) var pr_big: texture_2d_array<f32>;

struct PrShadowOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) material: u32,
};

@vertex
fn prop_shadow_vs(
    @location(0) pos: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(4) material: u32,
    @location(5) sway: f32,
    @builtin(instance_index) ii: u32,
) -> PrShadowOut {
    let inst = pr_instances[pr_visible[ii] & PR_INDEX_MASK];
    let world = pr_world(inst, pos, sway);
    var out: PrShadowOut;
    out.clip = pr_sv.view_proj * vec4<f32>(world, 1.0);
    out.uv = uv;
    out.material = material;
    return out;
}

@fragment
fn prop_shadow_cut_fs(in: PrShadowOut) {
    let m = pr_materials[in.material];
    var a = 1.0;
    if ((m.flags & PR_S_BIG) != 0u) {
        a = textureSampleLevel(pr_big, pr_sampler, in.uv, m.layer, 1.0).a;
    } else if ((m.flags & 8192u) == 0u) {
        a = textureSampleLevel(pr_albedo, pr_sampler, in.uv, m.layer, 1.0).a;
    }
    if (a < pr.lens.y) {
        discard;
    }
}
