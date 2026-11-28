//! The rear-yard fence's model (eo-props `yards.rs`, eo-app `--yard-detail`):
//! render model `props_lib2::YARD_FENCE`, a unit box (x across the fence
//! -0.5..0.5, y up 0..1, z along it -0.5..0.5) that an instance scales to the
//! fence's thickness, measured height and piece length, sheared along its
//! length by the ground's grade (the instance's `slope`, props_types.wgsl
//! `pr_local`). One mesh at every level: the boards are the shader's
//! (props_yards.wgsl `pr_fence`: the CC0 `Planks010` scan stood on end where
//! the scanned sets are on). Its tone and roughness are the kit's unpainted
//! cedar (`cedar-shingle` in eo-assets' kit-material-specs.json), the vertex
//! colour, so the fence reads as the same wood as the shingled houses.
//!
//! Drawn to `FENCE_FAR_M` (a 2 m fence seen at 45 degrees is a few pixels
//! tall there at 1536 px over 40 degrees) and casting into every sun cascade
//! (the shadow is most of what reads from above).

use crate::props_lib::*;
use crate::props_lib2::{Out, YARD_FENCE};

/// Past this the fence is not drawn (m; the props' dithered fade before it).
pub const FENCE_FAR_M: f32 = 1500.0;
/// Sun cascades a fence casts into (all three).
pub const FENCE_CASTS: u32 = 3;
/// The kit finish whose colour and roughness the fence takes.
pub const FENCE_FINISH: &str = "cedar-shingle";

/// Strip `// <tag>:on+` .. `// <tag>:end` blocks from WGSL when `keep` is false so
/// disabled yard/crown branches add zero IR to `pr_shade` (which stands near Apple
/// Metal's compiler limit alongside `--signal-light`).
fn select_tag(src: &str, tag: &str, keep: bool) -> String {
    let start = format!("// {tag}:on+");
    let end = format!("// {tag}:end");
    let mut out = String::with_capacity(src.len());
    let mut skip = false;
    for line in src.lines() {
        let t = line.trim();
        if t == start {
            skip = !keep;
            continue;
        }
        if t == end {
            skip = false;
            continue;
        }
        if !skip {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// props_draw.wgsl with the fence's boards (props_yards.wgsl `pr_fence`) when
/// the yards are drawn (`--yard-detail`), and the crown-core leaf field when
/// `--tree-crowns` is on; without each switch its branches and helpers are
/// stripped out completely, so `pr_shade` keeps its IR budget under Apple Metal's
/// compiler limit.
pub fn draw_wgsl() -> String {
    draw_wgsl_for(eo_props::yards::enabled(), eo_props::yards::plants_enabled(), eo_props::yards::wood_enabled(), eo_props::eztree::tree_crowns_enabled())
}

pub(crate) fn draw_wgsl_for(yd: bool, plants: bool, wood: bool, cr: bool) -> String {
    let base = select_tag(&select_tag(include_str!("props_draw.wgsl"), "yd", yd), "cr", cr);
    // (the weathering and the framing's timber come with the yards' plants, `--no-yard-plants` off.)
    let fence = if yd {
        format!("const PR_FENCE_AGED: bool = {plants};\nconst PR_YARD_WOOD: bool = {wood};\n{}", include_str!("props_yards.wgsl"))
    } else {
        String::new()
    };
    let crowns = if cr { CROWN_WGSL } else { "" };
    format!("{base}\n{fence}\n{crowns}")
}

/// `pr_fence` without `--yard-detail`: kept for `props_yards::tests` signature check.
#[allow(dead_code)]
const FENCE_STUB_WGSL: &str = "fn pr_fence(s: ptr<function, Surface>, inst: PrInstance, local: vec3<f32>, dlx: vec3<f32>, dly: vec3<f32>) {}\nfn pr_yard_item(s: ptr<function, Surface>, inst: PrInstance, kind: f32, local: vec3<f32>, uv: vec2<f32>, dlx: vec3<f32>, dly: vec3<f32>) {}\n";

/// Crown cores (`--tree-crowns`; eo-props `eztree::crown_core`, props_lib2.rs `CrownCores`):
/// compiled in only under `--tree-crowns`, using analytic value-noise gradients so `pr_shade`
/// stays within Apple Metal's LLVM IR limit alongside `--signal-light`.
const CROWN_WGSL: &str = r#"
const PR_CROWN_CLUMP_M: f32 = 0.6;
const PR_CROWN_LEAF_M: f32 = 0.2;

fn pr_crown_noise_grad(p: vec3<f32>) -> vec4<f32> {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let du = 6.0 * f * (1.0 - f);
    let n000 = pr_crown_hash(i);
    let n100 = pr_crown_hash(i + vec3<f32>(1.0, 0.0, 0.0));
    let n010 = pr_crown_hash(i + vec3<f32>(0.0, 1.0, 0.0));
    let n110 = pr_crown_hash(i + vec3<f32>(1.0, 1.0, 0.0));
    let n001 = pr_crown_hash(i + vec3<f32>(0.0, 0.0, 1.0));
    let n101 = pr_crown_hash(i + vec3<f32>(1.0, 0.0, 1.0));
    let n011 = pr_crown_hash(i + vec3<f32>(0.0, 1.0, 1.0));
    let n111 = pr_crown_hash(i + vec3<f32>(1.0, 1.0, 1.0));
    let k1 = n100 - n000;
    let k2 = n010 - n000;
    let k3 = n001 - n000;
    let k4 = n000 - n100 - n010 + n110;
    let k5 = n000 - n010 - n001 + n011;
    let k6 = n000 - n100 - n001 + n101;
    let k7 = -n000 + n100 + n010 - n110 + n001 - n101 - n011 + n111;
    let val = n000 + k1 * u.x + k2 * u.y + k3 * u.z + k4 * u.x * u.y + k5 * u.y * u.z + k6 * u.z * u.x + k7 * u.x * u.y * u.z;
    let gx = du.x * (k1 + k4 * u.y + k6 * u.z + k7 * u.y * u.z);
    let gy = du.y * (k2 + k5 * u.z + k4 * u.x + k7 * u.z * u.x);
    let gz = du.z * (k3 + k6 * u.x + k5 * u.y + k7 * u.x * u.y);
    return vec4<f32>(val, gx, gy, gz);
}

fn pr_crown_edge(in: PrOut, inst: PrInstance, dlx: vec3<f32>, dly: vec3<f32>) -> bool {
    let v = normalize(frame.cam_pos.xyz - in.world);
    let rim = 1.0 - abs(dot(normalize(in.normal), v));
    let edge = smoothstep(0.45, 0.9, rim);
    if (edge <= 0.0) {
        return true;
    }
    let p = in.local * inst.scale;
    let fw = max(length(dlx * inst.scale), length(dly * inst.scale));
    let k = max(PR_CROWN_LEAF_M * 1.75, fw * 1.5);
    let q = pr_crown_noise(p / k) * 0.65 + pr_crown_noise(p / (k * 2.3) + vec3<f32>(7.1)) * 0.35;
    return q > edge * 0.62;
}

fn pr_crown_leaves(s: ptr<function, Surface>, p: vec3<f32>, px: vec3<f32>, py: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>) {
    let fw = max(length(px), length(py));
    let k1 = 1.0 - smoothstep(PR_CROWN_CLUMP_M * 0.4, PR_CROWN_CLUMP_M * 1.2, fw);
    if (k1 <= 0.0) {
        return;
    }
    let k2 = 1.0 - smoothstep(PR_CROWN_LEAF_M * 0.4, PR_CROWN_LEAF_M * 1.2, fw);
    let cg = pr_crown_noise_grad(p / PR_CROWN_CLUMP_M);
    let c0 = cg.x;
    let leaf = 0.5 + (c0 - 0.5) * k1 + (pr_crown_noise(p / PR_CROWN_LEAF_M + vec3<f32>(31.7)) - 0.5) * 0.6 * k2;
    let hx = dot(cg.yzw, px / PR_CROWN_CLUMP_M) * 0.5 * k1;
    let hy = dot(cg.yzw, py / PR_CROWN_CLUMP_M) * 0.5 * k1;
    let n = (*s).n;
    let r1 = cross(dpy, n);
    let r2 = cross(n, dpx);
    let det = dot(dpx, r1);
    let nb = normalize(abs(det) * n - sign(det) * (hx * r1 + hy * r2));
    if (all(nb == nb) && abs(det) > 1e-12) {
        (*s).n = nb;
    }
    let gap = smoothstep(0.3, 0.15, leaf);
    (*s).albedo = (*s).albedo * (0.85 + 0.3 * leaf) * (1.0 - 0.4 * gap);
    (*s).ao = (*s).ao * (1.0 - 0.2 * gap);
}
"#;

/// The finish's sRGB colour and roughness (`kit-material-specs.json`).
pub fn fence_finish() -> anyhow::Result<([u8; 3], f32)> {
    let lib = eo_assets::kit::KitLibrary::load()?;
    let spec = lib.specs.get(FENCE_FINISH).ok_or_else(|| anyhow::anyhow!("kit-material-specs.json has no {FENCE_FINISH}"))?;
    let hex = spec.p["color"].as_str().ok_or_else(|| anyhow::anyhow!("{FENCE_FINISH} has no colour"))?;
    Ok((srgb_hex(hex), spec.rough as f32))
}

/// Add the fence's material, mesh and model (`YARD_FENCE`) to the library.
pub(crate) fn cook_fence(out: &mut Out) {
    let (colour, rough) = match fence_finish() {
        Ok(f) => f,
        Err(e) => {
            out.missing.push(format!("yard fence: {e:#}"));
            return;
        }
    };
    out.materials.push(PropMaterial { base: [1.0; 4], params: [rough, 0.0, 1.0, 0.0], ..Default::default() });
    let m = out.materials.len() as u32 - 1;
    out.b.cuboid([-0.5, 0.0, -0.5], [0.5, 1.0, 0.5], colour, m);
    // close the builder's geometry as one mesh (props_lib2 `Out::finish`).
    let base_vertex = out.v.len() as i32;
    out.v.append(&mut out.b.v);
    let first = out.i.len() as u32;
    let count = out.b.opaque.len() as u32;
    out.i.append(&mut out.b.opaque);
    let cut_first = out.i.len() as u32;
    let cut_count = out.b.cut.len() as u32;
    out.i.append(&mut out.b.cut);
    out.meshes.push(GpuPropMesh { first, count, cut_first, cut_count, base_vertex, pad: [0; 3] });
    let mesh = out.meshes.len() as u32 - 1;
    out.models[YARD_FENCE as usize] = GpuPropModel {
        meshes: [mesh, mesh, mesh, NONE],
        lod: [FENCE_FAR_M, FENCE_FAR_M, FENCE_FAR_M, FENCE_FAR_M],
        center: [0.0, 0.5, 0.0],
        radius: 0.75f32.sqrt(),
        cast: [FENCE_CASTS, 0, 0, 0],
    };
    out.sources.push(format!("yard fence: a unit box in the kit's {FENCE_FINISH} finish"));
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_shaders_know_the_fence() {
        let types = crate::props::pr_types_src();
        assert!(types.contains(&format!("const PR_YARD_FENCE: u32 = {}u;", crate::props_lib2::YARD_FENCE)), "pr_types_src's PR_YARD_FENCE is not props_lib2::YARD_FENCE");
        assert!(types.contains("i.model == PR_YARD_FENCE"), "pr_local shears the fence by its grade");
        assert!(include_str!("props_draw.wgsl").contains("pr_fence(&s"), "props_draw.wgsl draws the boards");
    }

    #[test]
    fn the_shaders_know_the_yard_models() {
        // the paint and wood of the yards' objects and the houses' backs (props_yards.wgsl `pr_yard_item`)
        // over every yard model and no other, and a stub of it without the yards.
        let types = crate::props::pr_types_src();
        assert!(types.contains(&format!("const PR_YARD_ITEM_BASE: u32 = {}u;", crate::props_lib2::YARD_ITEM_BASE)));
        let end = crate::props_lib2::YARD_BACK_BASE + crate::props_lib2::yard_backs::COUNT;
        assert_eq!(end as usize, crate::props_lib2::RENDER_MODEL_COUNT, "the yard models are the library's last");
        assert!(types.contains(&format!("const PR_YARD_ITEM_END: u32 = {end}u;")));
        assert!(include_str!("props_draw.wgsl").contains("pr_yard_item(&s, inst, m.params.w, in.local, in.uv, dlx, dly)"));
        assert!(super::FENCE_STUB_WGSL.contains("fn pr_yard_item(s: ptr<function, Surface>, inst: PrInstance, kind: f32, local: vec3<f32>, uv: vec2<f32>, dlx: vec3<f32>, dly: vec3<f32>) {}"));
        assert!(include_str!("props_yards.wgsl").contains("fn pr_yard_item(s: ptr<function, Surface>, inst: PrInstance, kind: f32, local: vec3<f32>, uv: vec2<f32>, dlx: vec3<f32>, dly: vec3<f32>) {"));
    }

    #[test]
    fn the_finish_is_the_kits_cedar() {
        let (c, r) = super::fence_finish().unwrap();
        assert_eq!(c, [0x6a, 0x56, 0x46]);
        assert!((r - 0.82).abs() < 1e-6);
    }

    #[test]
    fn yard_and_crown_shader_paths_validate() {
        use wgpu::naga;
        let city = crate::look::props_city_source(crate::look::city_shader_source_variant_probes(
            crate::ambient::ShaderLevels::Plane,
            crate::look::ShadowLevels::Near,
            false,
        ));
        let types = crate::props::pr_types_src();
        for yd in [false, true] {
            for cr in [false, true] {
                let draw = crate::look::sf_select(&super::draw_wgsl_for(yd, true, true, cr));
                assert_eq!(draw.contains("pr_fence(&s"), yd, "yd={yd} cr={cr}: pr_fence presence");
                assert_eq!(draw.contains("pr_yard_item(&s"), yd, "yd={yd} cr={cr}: pr_yard_item presence");
                assert_eq!(draw.contains("pr_crown_leaves(&s"), cr, "yd={yd} cr={cr}: pr_crown_leaves presence");
                assert_eq!(draw.contains("pr_crown_edge("), cr, "yd={yd} cr={cr}: pr_crown_edge presence");
                let src = format!("{city}\n{types}\n{draw}");
                let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("yd={yd} cr={cr}: {}", e.emit_to_string(&src)));
                naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
                    .validate(&module)
                    .unwrap_or_else(|e| panic!("yd={yd} cr={cr}: {}", e.emit_to_string(&src)));
            }
        }
    }
}
