// lamp and headlight shadows (lighting; light_shadow.rs): the street props
// into the atlas, after props_types.wgsl and light_shadow_draw.wgsl. Group 2
// is the props' own data group (props.rs, as props_shadow.wgsl binds it):
// the instance's placement and wind (`pr_world`), and for the alpha-tested
// parts the material's coverage, tested as the sun's casters test it.

struct LsPrMaterial {
    base: vec4<f32>,
    emissive: vec4<f32>,
    params: vec4<f32>,
    layer: i32,
    flags: u32,
    rough_flatten: f32,
    thin_absorption: f32,
};

const LS_PR_S_BIG: u32 = 32768u;

@group(2) @binding(0) var<uniform> pr: PrParams;
@group(2) @binding(1) var<storage, read> pr_instances: array<PrInstance>;
@group(2) @binding(6) var pr_albedo: texture_2d_array<f32>;
@group(2) @binding(7) var pr_sampler: sampler;
@group(2) @binding(8) var<storage, read> pr_materials: array<LsPrMaterial>;
@group(2) @binding(9) var pr_big: texture_2d_array<f32>;

/// props_lib.rs PropVertex (8 words): position, normal, uv, colour, material, wind weight.
@vertex
fn ls_prop_vs(@builtin(vertex_index) v: u32) -> LsOut {
    if (v / 3u >= ls_list[1]) {
        return ls_none();
    }
    let iv = ls_vertex(v);
    let rec = ls_recs[iv.y];
    let w = iv.x * 8u;
    let p = vec3<f32>(bitcast<f32>(ls_vtx[w]), bitcast<f32>(ls_vtx[w + 1u]), bitcast<f32>(ls_vtx[w + 2u]));
    let inst = pr_instances[rec.payload];
    var o = ls_project(rec.tile, pr_world(inst, p, bitcast<f32>(ls_vtx[w + 7u])));
    o.uv = unpack2x16unorm(ls_vtx[w + 4u]);
    o.mat = vec2<u32>(ls_vtx[w + 6u], rec.flags);
    return o;
}

@fragment
fn ls_prop_fs(in: LsIn) -> @builtin(frag_depth) f32 {
    if (ls_outside(in)) {
        discard;
    }
    if (in.mat.y == 1u) {
        // props_shadow.wgsl `prop_shadow_cut_fs`
        let m = pr_materials[in.mat.x];
        var a = 1.0;
        if ((m.flags & LS_PR_S_BIG) != 0u) {
            a = textureSampleLevel(pr_big, pr_sampler, in.uv, m.layer, 1.0).a;
        } else if ((m.flags & 8192u) == 0u) {
            a = textureSampleLevel(pr_albedo, pr_sampler, in.uv, m.layer, 1.0).a;
        }
        if (a < pr.lens.y) {
            discard;
        }
    }
    return clamp(in.lin, 0.0, 1.0);
}
