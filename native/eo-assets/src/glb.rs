//! GLB loading: digest check, then parse with the `gltf` crate and flatten the
//! scene into one indexed mesh per building, grouped by material.
//!
//! What is kept: every mesh primitive reachable from the default scene, with
//! its node transform (translation * rotation * scale, parents first), the
//! POSITION and NORMAL attributes and the indices; every material's name and
//! its PBR factors (base colour, metallic, roughness, emissive, alpha mode).
//! The building GLBs carry no textures and no texture references (checked on
//! all 1,958 files: 0 images, 0 textures); their surfaces are the authored
//! kit's procedural maps, chosen by material name (see `kit`).
//!
//! What is dropped: nodes named `shadow-proxy*` (the JS runtime takes them off
//! the main camera's layer: they are shadow-only casters) and TEXCOORD/COLOR
//! attributes (no texture reads them).

use anyhow::{Context, Result, anyhow, bail};
use glam::{Mat3, Mat4, Vec3};
use sha2::{Digest, Sha256};
use std::path::Path;

use crate::manifest::AssetRef;

/// Lower-case hex SHA-256.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let d = Sha256::digest(bytes);
    let mut s = String::with_capacity(64);
    for b in d.iter() {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Read a GLB and refuse it unless its length and SHA-256 are the manifest's,
/// as `loadVerified` in blender/runtime.js does.
pub fn read_verified(path: &Path, asset: &AssetRef) -> Result<Vec<u8>> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    verify_bytes(&bytes, asset)?;
    Ok(bytes)
}

pub fn verify_bytes(bytes: &[u8], asset: &AssetRef) -> Result<()> {
    if asset.bytes != 0 && bytes.len() as u64 != asset.bytes {
        bail!("{}: byte length mismatch ({} on disk, {} in the manifest); refused", asset.url, bytes.len(), asset.bytes);
    }
    let hash = sha256_hex(bytes);
    if !hash.eq_ignore_ascii_case(&asset.sha256) {
        bail!("{}: SHA-256 mismatch ({hash} on disk, {} in the manifest); refused", asset.url, asset.sha256);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum AlphaMode {
    Opaque = 0,
    Mask = 1,
    Blend = 2,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CookedMaterial {
    pub name: String,
    pub base_color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    pub emissive: [f32; 3],
    pub emissive_strength: f32,
    pub alpha_mode: AlphaMode,
    pub alpha_cutoff: f32,
    pub double_sided: bool,
}

impl CookedMaterial {
    fn default_material() -> Self {
        Self {
            name: String::new(),
            base_color: [1.0; 4],
            metallic: 1.0,
            roughness: 1.0,
            emissive: [0.0; 3],
            emissive_strength: 1.0,
            alpha_mode: AlphaMode::Opaque,
            alpha_cutoff: 0.5,
            double_sided: false,
        }
    }
}

/// A run of indices that uses one material.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CookedPart {
    pub material: u32,
    pub first_index: u32,
    pub index_count: u32,
    pub min: [f32; 3],
    pub max: [f32; 3],
}

/// One building's model in its GLB frame (the footprint frame: +X along the
/// front edge, +Z out of it, y up from the building's base).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct CookedModel {
    pub positions: Vec<[f32; 3]>,
    /// Unit normals packed as snorm8 x 3 (w = 0).
    pub normals: Vec<u32>,
    pub indices: Vec<u32>,
    pub parts: Vec<CookedPart>,
    pub materials: Vec<CookedMaterial>,
    pub min: [f32; 3],
    pub max: [f32; 3],
    /// Mesh primitive instances drawn (after dropping shadow proxies).
    pub primitives: u32,
    /// Nodes dropped as shadow-only proxies, and their triangles.
    pub dropped_shadow_nodes: u32,
    pub dropped_shadow_triangles: u32,
}

impl CookedModel {
    pub fn triangles(&self) -> u64 {
        self.indices.len() as u64 / 3
    }
}

pub fn pack_normal(n: Vec3) -> u32 {
    let n = n.normalize_or(Vec3::Y);
    let q = |v: f32| ((v.clamp(-1.0, 1.0) * 127.0).round() as i32 as i8) as u8 as u32;
    q(n.x) | (q(n.y) << 8) | (q(n.z) << 16)
}

pub fn unpack_normal(p: u32) -> Vec3 {
    let c = |s: u32| (((p >> s) & 255) as u8 as i8) as f32 / 127.0;
    Vec3::new(c(0), c(8), c(16))
}

fn is_shadow_proxy(name: Option<&str>) -> bool {
    name.map(|n| n.starts_with("shadow-proxy")).unwrap_or(false)
}

fn materials_of(doc: &gltf::Document) -> Vec<CookedMaterial> {
    doc.materials()
        .map(|m| {
            let pbr = m.pbr_metallic_roughness();
            let mut name = m.name().unwrap_or_default().to_string();
            let mut base_color = pbr.base_color_factor();
            let mut roughness = pbr.roughness_factor();
            let mut metallic = pbr.metallic_factor();
            let mut alpha_mode = match m.alpha_mode() {
                gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
                gltf::material::AlphaMode::Mask => AlphaMode::Mask,
                gltf::material::AlphaMode::Blend => AlphaMode::Blend,
            };
            // Exterior Blender buildings carry an unshadowed `int-plaster` hollow shell
            // behind window openings (some facades omit `glass-clear`, leaving punched holes
            // open onto raw `int-plaster`, e.g. `sf-building-262689159` +X wall, while its
            // `glazing` node uses opaque dark `painted-white@26221f`/`@1c1e22`). Verified
            // absent from all playable `beaver-terrace` GLBs. Re-map to `glass-shell` so
            // missing-glass punched openings receive glossy Fresnel reflections + 3D parallax
            // `rm_atlas` interiors, while glazed facades still draw `glass-clear` in front.
            if ((name == "int-plaster" || name.starts_with("int-plaster@")) && roughness >= 0.85)
                || name == "painted-white@26221f"
                || name == "painted-white@1c1e22"
            {
                name = "glass-shell".to_string();
                base_color = [0.22, 0.26, 0.30, 1.0];
                roughness = 0.08;
                metallic = 0.02;
                alpha_mode = AlphaMode::Opaque;
            }
            // `sf-building-262716139`'s 4 ground-floor Beaux-Arts arched storefront bays
            // (`painted-white@d8cca8`, unique to this GLB across all 345 models) were
            // exported as opaque beige quads behind iron mullions (`painted-steel@141516`).
            // Re-map to `glass-storefront` so the 4 arched bays read as glazed retail/lobby
            // windows with 3D `rm_atlas` interiors and sky/street reflections.
            if name == "painted-white@d8cca8" {
                name = "glass-storefront".to_string();
                base_color = [0.60, 0.65, 0.68, 0.86];
                roughness = 0.05;
                metallic = 0.0;
                alpha_mode = AlphaMode::Blend;
            }
            // Three exterior Blender GLB plinth retaining walls (`sf-building-262689168`,
            // `1469656012`, `262716134`) inherited garden-hedge green on their rigid 1.4 m
            // sidewalk retaining walls (`painted-stucco@4a6a3c` / `@48683c`), reading as
            // smooth green boxes. Re-skin to weathered hillside ashlar stone masonry.
            if name == "painted-stucco@4a6a3c" || name == "painted-stucco@48683c" {
                name = "limestone-ashlar@8a8478".to_string();
                base_color = [0.25, 0.235, 0.205, 1.0];
                roughness = 0.90;
                metallic = 0.0;
            }
            CookedMaterial {
                name,
                base_color,
                metallic,
                roughness,
                emissive: m.emissive_factor(),
                emissive_strength: m.emissive_strength().unwrap_or(1.0),
                alpha_mode,
                alpha_cutoff: m.alpha_cutoff().unwrap_or(0.5),
                double_sided: m.double_sided(),
            }
        })
        .collect()
}

struct PrimOut {
    material: u32,
    indices: Vec<u32>,
}

/// Parse a (verified) GLB and flatten it.
pub fn cook_glb(bytes: &[u8]) -> Result<CookedModel> {
    cook_mesh(bytes, None, None, None, None, None)
}

/// Return the first texture coordinates in the same vertex order as the cooked mesh.
pub fn cook_glb_with_uv0(bytes: &[u8]) -> Result<(CookedModel, Vec<[f32; 2]>)> {
    let mut uv = Vec::new();
    let model = cook_mesh(bytes, Some(&mut uv), None, None, None, None)?;
    Ok((model, uv))
}

/// Preserve the light coordinates and optional near-field visibility in TEXCOORD_1.x.
/// Older primitives without baked visibility receive a fully exposed ambient term.
pub fn cook_glb_with_visibility(bytes: &[u8], placements: &std::collections::HashMap<String, Mat4>) -> Result<(CookedModel, Vec<[f32; 2]>, Vec<f32>)> {
    let mut uv = Vec::new();
    let mut visibility = Vec::new();
    let model = cook_mesh(bytes, Some(&mut uv), Some(&mut visibility), None, Some(placements), None)?;
    Ok((model, uv, visibility))
}

/// Additional UV data stays sparse. Most of a furnished building has no cut-fabric pattern.
pub type CutPattern = std::collections::HashMap<u32, [u8; 2]>;

pub fn cook_glb_with_cut_pattern(
    bytes: &[u8], placements: &std::collections::HashMap<String, Mat4>,
) -> Result<(CookedModel, Vec<[f32; 2]>, Vec<f32>, CutPattern)> {
    cook_furnished_glb(bytes, placements, true, None)
}

/// Cook selected architectural meshes before material batching. Placement and
/// lighting attributes use the same vertex order as the surviving primitives.
pub fn cook_furnished_glb(
    bytes: &[u8],
    placements: &std::collections::HashMap<String, Mat4>,
    read_pattern: bool,
    omit_mesh: Option<fn(&str) -> bool>,
) -> Result<(CookedModel, Vec<[f32; 2]>, Vec<f32>, CutPattern)> {
    let filter = omit_mesh.as_ref().map(|f| f as &dyn Fn(&str) -> bool);
    cook_furnished_glb_filtered(bytes, placements, read_pattern, filter)
}

/// Apply a selection from the installed asset manifest before any shared material batching.
pub fn cook_furnished_glb_filtered(
    bytes: &[u8],
    placements: &std::collections::HashMap<String, Mat4>,
    read_pattern: bool,
    omit_mesh: Option<&dyn Fn(&str) -> bool>,
) -> Result<(CookedModel, Vec<[f32; 2]>, Vec<f32>, CutPattern)> {
    let mut light = Vec::new();
    let mut contact = Vec::new();
    let mut cut = CutPattern::new();
    let model = cook_mesh(bytes, Some(&mut light), Some(&mut contact), read_pattern.then_some(&mut cut), Some(placements), omit_mesh)?;
    Ok((model, light, contact, cut))
}

fn cook_mesh(bytes: &[u8], mut uv: Option<&mut Vec<[f32; 2]>>, mut visibility: Option<&mut Vec<f32>>, mut cut: Option<&mut CutPattern>, placements: Option<&std::collections::HashMap<String, Mat4>>, omit_mesh: Option<&dyn Fn(&str) -> bool>) -> Result<CookedModel> {
    let gltf = gltf::Gltf::from_slice(bytes).map_err(|e| anyhow!("glTF parse: {e}"))?;
    let blob = gltf.blob.as_deref();
    let doc = &gltf.document;
    let mut model = CookedModel::default();
    let mut materials = materials_of(doc);
    let default_material = materials.len() as u32;
    let mut used_default = false;
    let mut prims: Vec<PrimOut> = Vec::new();
    let scene = match doc.default_scene().or_else(|| doc.scenes().next()) {
        Some(s) => s,
        None => {
            // An empty GLB (no scene, no meshes) is a valid build: nothing to draw.
            model.materials = materials;
            return Ok(model);
        }
    };
    let mut stack: Vec<(gltf::Node, Mat4)> = scene.nodes().map(|n| (n, Mat4::IDENTITY)).collect();
    while let Some((node, parent)) = stack.pop() {
        let mut world = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
        // World-space placement on the assembly root is inherited by every mesh and normal.
        if let Some(m) = placements.and_then(|p| p.get(node.name().unwrap_or_default())) {
            world = *m * world;
        }
        for child in node.children() {
            stack.push((child, world));
        }
        let Some(mesh) = node.mesh() else { continue };
        if omit_mesh.is_some_and(|omit| omit(node.name().unwrap_or_default())) {
            continue;
        }
        if is_shadow_proxy(node.name()) {
            model.dropped_shadow_nodes += 1;
            for p in mesh.primitives() {
                if let Some(acc) = p.indices() {
                    model.dropped_shadow_triangles += acc.count() as u32 / 3;
                }
            }
            continue;
        }
        let normal_m = Mat3::from_mat4(world).inverse().transpose();
        for p in mesh.primitives() {
            if p.mode() != gltf::mesh::Mode::Triangles {
                bail!("primitive mode {:?} is not supported (triangles only)", p.mode());
            }
            let reader = p.reader(|b| match b.source() {
                gltf::buffer::Source::Bin => blob,
                gltf::buffer::Source::Uri(_) => None,
            });
            let positions: Vec<[f32; 3]> =
                reader.read_positions().ok_or_else(|| anyhow!("primitive without POSITION"))?.collect();
            let normals: Option<Vec<[f32; 3]>> = reader.read_normals().map(|n| n.collect());
            if let Some(uv) = uv.as_deref_mut() {
                let coords = reader.read_tex_coords(0).ok_or_else(|| anyhow!("primitive without TEXCOORD_0"))?.into_f32();
                anyhow::ensure!(coords.len() == positions.len(), "texture coordinate count differs from POSITION");
                uv.extend(coords);
            }
            if let Some(values) = visibility.as_deref_mut() {
                if let Some(coords) = reader.read_tex_coords(1) {
                    let coords = coords.into_f32();
                    anyhow::ensure!(coords.len() == positions.len(), "visibility count differs from POSITION");
                    for [value, _] in coords {
                        anyhow::ensure!((0.0..=1.0).contains(&value), "invalid near-field visibility");
                        values.push(value);
                    }
                } else {
                    values.resize(values.len() + positions.len(), 1.0);
                }
            }
            let base = model.positions.len() as u32;
            if let Some(pattern) = cut.as_deref_mut()
                && let Some(coords) = reader.read_tex_coords(2)
            {
                let coords = coords.into_f32();
                anyhow::ensure!(coords.len() == positions.len(), "cut pattern count differs from POSITION");
                for (i, uv) in coords.enumerate() {
                    anyhow::ensure!(uv.iter().all(|c| (0.0..=1.0).contains(c)), "cut pattern outside fabric");
                    pattern.insert(base + i as u32, uv.map(|v| (v * 255.).round() as u8));
                }
            }
            for (i, p3) in positions.iter().enumerate() {
                let wp = world.transform_point3(Vec3::from(*p3));
                model.positions.push(wp.to_array());
                let n = normals.as_ref().map(|n| normal_m * Vec3::from(n[i])).unwrap_or(Vec3::ZERO);
                // A missing normal stays zero: the shader then takes the face normal.
                model.normals.push(if normals.is_some() { pack_normal(n) } else { 0 });
            }
            let indices: Vec<u32> = match reader.read_indices() {
                Some(ix) => ix.into_u32().map(|i| i + base).collect(),
                None => (0..positions.len() as u32).map(|i| i + base).collect(),
            };
            let material = match p.material().index() {
                Some(i) => i as u32,
                None => {
                    used_default = true;
                    default_material
                }
            };
            model.primitives += 1;
            prims.push(PrimOut { material, indices });
        }
    }
    if used_default {
        materials.push(CookedMaterial::default_material());
    }
    // Group by material, in material order: one part per material.
    let mut order: Vec<u32> = prims.iter().map(|p| p.material).collect();
    order.sort_unstable();
    order.dedup();
    for m in order {
        let first = model.indices.len() as u32;
        let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
        for p in prims.iter().filter(|p| p.material == m) {
            for &i in &p.indices {
                let v = model.positions[i as usize];
                for k in 0..3 {
                    mn[k] = mn[k].min(v[k]);
                    mx[k] = mx[k].max(v[k]);
                }
            }
            model.indices.extend_from_slice(&p.indices);
        }
        let count = model.indices.len() as u32 - first;
        if count > 0 {
            model.parts.push(CookedPart { material: m, first_index: first, index_count: count, min: mn, max: mx });
        }
    }
    let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
    for v in &model.positions {
        for k in 0..3 {
            mn[k] = mn[k].min(v[k]);
            mx[k] = mx[k].max(v[k]);
        }
    }
    if model.positions.is_empty() {
        mn = [0.0; 3];
        mx = [0.0; 3];
    }
    model.min = mn;
    model.max = mx;
    model.materials = materials;
    Ok(model)
}

/// Vertex, index and primitive counts a GLB's JSON header declares (no
/// buffer reads, no digest check): enough to reserve GPU space before the
/// model itself is loaded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GlbCounts {
    pub vertices: u32,
    pub indices: u32,
    pub primitives: u32,
    /// Materials the GLB declares (plus the default one if a primitive has none).
    pub materials: u32,
    /// Distinct materials the drawn primitives use = parts after cooking.
    pub parts: u32,
}

/// What a GLB's JSON chunk says, without reading or checking its binary chunk.
pub struct GlbHeader {
    pub counts: GlbCounts,
    /// Same order and values `cook_glb` gives (default material appended last if used).
    pub materials: Vec<CookedMaterial>,
}

pub fn peek_counts(path: &Path) -> Result<GlbCounts> {
    Ok(peek_header(path)?.counts)
}

/// Read only the JSON chunk of a GLB and total what its default scene draws
/// (shadow-proxy nodes excluded, as `cook_glb` excludes them). Enough to
/// reserve GPU space and resolve materials before the model is loaded.
pub fn peek_header(path: &Path) -> Result<GlbHeader> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut head = [0u8; 20];
    f.read_exact(&mut head)?;
    if &head[0..4] != b"glTF" || &head[16..20] != b"JSON" {
        bail!("{}: not a GLB", path.display());
    }
    let len = u32::from_le_bytes(head[12..16].try_into().unwrap()) as usize;
    let mut json = vec![0u8; len];
    f.read_exact(&mut json)?;
    let root: gltf::json::Root = gltf::json::deserialize::from_slice(&json).map_err(|e| anyhow!("{}: {e}", path.display()))?;
    let mut c = GlbCounts { materials: root.materials.len() as u32, ..Default::default() };
    let mut used: Vec<u32> = Vec::new();
    let scene = root.scene.map(|s| s.value()).unwrap_or(0);
    let mut any_default = false;
    if let Some(scene) = root.scenes.get(scene) {
        let mut stack: Vec<usize> = scene.nodes.iter().map(|n| n.value()).collect();
        while let Some(ni) = stack.pop() {
            let node = &root.nodes[ni];
            if let Some(ch) = &node.children {
                stack.extend(ch.iter().map(|n| n.value()));
            }
            let Some(mi) = node.mesh else { continue };
            if is_shadow_proxy(node.name.as_deref()) {
                continue;
            }
            for p in &root.meshes[mi.value()].primitives {
                let pos = p.attributes.get(&gltf::json::validation::Checked::Valid(gltf::Semantic::Positions));
                let nv = pos.map(|a| root.accessors[a.value()].count.0 as u32).unwrap_or(0);
                c.vertices += nv;
                c.indices += p.indices.map(|a| root.accessors[a.value()].count.0 as u32).unwrap_or(nv);
                c.primitives += 1;
                match p.material {
                    Some(m) => used.push(m.value() as u32),
                    None => {
                        any_default = true;
                        used.push(u32::MAX);
                    }
                }
            }
        }
    }
    used.sort_unstable();
    used.dedup();
    c.parts = used.len() as u32;
    let doc = gltf::Document::from_json_without_validation(root);
    let mut materials = materials_of(&doc);
    if any_default {
        c.materials += 1;
        materials.push(CookedMaterial::default_material());
    }
    Ok(GlbHeader { counts: c, materials })
}
