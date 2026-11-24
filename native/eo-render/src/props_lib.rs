//! the mesh library every street prop instance draws from, cooked
//! once: the JS game's hash-locked street-prop GLBs (`signalHead-v1`,
//! `palm-v1`, `palmFar-v1`; sizes and digests from the JS manifest
//! `STREET_PROP_ASSETS_V1`, read with eo-assets' digest check, never modified)
//! with their baked JPEG maps, and procedural meshes where the JS builds the
//! part in code (signal and sign poles, mast arms, lenses, pedestrian heads,
//! sign plates, tactile pads) or where its generator is not ported (street
//! trees: a trunk and leaf cards on the JS vegetation atlas, three levels down
//! to two crossed cards). Cached under `cache/props/<key>.eopl` (the payload's
//! SHA-256 checked on read).

use anyhow::{Context, Result, anyhow, bail};
use bytemuck::{Pod, Zeroable};
use eo_props::{JsProps, MODEL_COUNT, Model};
use std::path::Path;

pub const PROPS_LIB_VERSION: &str = "eo-props-lib-v2";
/// Texture size of the albedo array: the GLB maps are 512 px (two are 1024 x 512). // props2
pub const ALBEDO_RES: u32 = 512;
/// Normal / ORM arrays. // props2
pub const AUX_RES: u32 = 256;
/// the big array (layer 0 the vegetation atlas, 1 the sign faces, 2 the printed faces).
pub const BIG_RES: u32 = 2048;
pub const NONE: u32 = u32::MAX;
/// Levels a model can have (the last is its far form).
pub const LODS: usize = 3;

/// One vertex (32 bytes): position, normal, uv, sRGB colour, material.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct PropVertex {
    pub pos: [f32; 3],
    pub normal: [i8; 4],
    pub uv: [u16; 2],
    pub color: [u8; 4],
    pub material: u32,
    pub pad: u32,
}
const _: () = assert!(std::mem::size_of::<PropVertex>() == 32);

/// One drawable mesh: an opaque index range and an alpha-tested one.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct GpuPropMesh {
    pub first: u32,
    pub count: u32,
    pub cut_first: u32,
    pub cut_count: u32,
    pub base_vertex: i32,
    pub pad: [u32; 3],
}

/// One model (64 bytes): its levels, their distances, a bounding sphere in
/// the unit model frame, which cascades it casts into.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct GpuPropModel {
    pub meshes: [u32; 4],
    /// Level 0 below x, 1 below y, 2 below z; nothing from w.
    pub lod: [f32; 4],
    pub center: [f32; 3],
    pub radius: f32,
    /// x: cascades it casts into (0 none, 1 = cascade 0, ...); y: flags.
    pub cast: [u32; 4],
}

/// The same 64-byte layout as draw.wgsl's `Material`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct PropMaterial {
    pub base: [f32; 4],
    pub emissive: [f32; 4],
    /// roughness, metalness, normal strength, unused
    pub params: [f32; 4],
    pub layer: i32,
    pub flags: u32,
    pub rough_flatten: f32,
    pub thin_absorption: f32,
}

/// Material flags (props_types.wgsl).
pub const P_TEX: u32 = 1 << 8;
pub const P_CUT: u32 = 1 << 9;
/// The tree basin's level-0 fringe past its cut edge (unit panel; props_draw.wgsl `pr_basin_spill`).
pub const BASIN_FRINGE: f32 = 0.1;
pub const P_LENS: u32 = 1 << 10;
pub const P_PED_HAND: u32 = 1 << 11;
pub const P_PED_WALK: u32 = 1 << 12;
pub const P_PLATE: u32 = 1 << 13;
pub const P_GLB_MAPS: u32 = 1 << 14;
/// the layer is in the big array (vegetation atlas, sign faces, printed faces).
pub const P_BIG: u32 = 1 << 15;
/// a leaf (back-lit transmission).
pub const P_LEAF: u32 = 1 << 16;
/// (`--luminaires`) a luminaire's lit glass (props_draw.wgsl `pr_lamp_glass`);
/// its material's `emissive` carries `[kind, area, height, share]` (`luminaire_sockets`).
pub const P_LAMP_GLOW: u32 = 1 << 20;
/// Bits 24..25: the lens index (0 red, 1 amber, 2 green); clear of draw.wgsl's and the facade's flag bits.
pub const P_LENS_SHIFT: u32 = 24;

pub struct PropLibrary {
    pub vertices: Vec<PropVertex>,
    pub indices: Vec<u32>,
    pub meshes: Vec<GpuPropMesh>,
    pub models: Vec<GpuPropModel>,
    pub materials: Vec<PropMaterial>,
    /// Per layer: albedo RGBA at ALBEDO_RES, normal and ORM at AUX_RES.
    pub layers: Vec<[Vec<u8>; 3]>,
    /// Plate faces' sRGB colours (`SIGN_FACE_COLOURS`) by face index, then the plate back.
    pub faces: Vec<[f32; 4]>,
    /// Lens tone inputs (`SIGNAL_LENS_TONE_INPUT`): lit r, a, g then unlit r, a, g; w of [0] the ACES scale.
    pub lens: [[f32; 4]; 6],
    pub alpha_test: f32,
    /// Where each mesh came from (report).
    pub sources: Vec<String>,
    pub missing: Vec<String>,
    /// the big array's layers (RGBA at BIG_RES, rows top first).
    pub big_layers: Vec<Vec<u8>>,
    /// `STREET_TREE_SPECIES` in variant order (render model `TREE_BASE + i`), and per species
    /// (height m, max half crown m) for the card-tree fallback.
    pub tree_species: Vec<String>,
    pub tree_dims: Vec<[f32; 2]>,
    /// sign faces in the faces buffer (tiles follow the colours and the back).
    pub face_count: u32,
    /// detectable-warning domes: base diameter, height, pitch (m), pad depth (`caMetric`).
    pub dome: [f32; 4],
}

#[derive(Default)]
pub(crate) struct Builder {
    pub(crate) v: Vec<PropVertex>,
    pub(crate) opaque: Vec<u32>,
    pub(crate) cut: Vec<u32>,
}

fn pack_n(n: [f32; 3]) -> [i8; 4] {
    let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-9);
    [(n[0] / l * 127.0).round() as i8, (n[1] / l * 127.0).round() as i8, (n[2] / l * 127.0).round() as i8, 0]
}

fn uv16(u: f32, v: f32) -> [u16; 2] {
    [(u.clamp(0.0, 1.0) * 65535.0).round() as u16, (v.clamp(0.0, 1.0) * 65535.0).round() as u16]
}

impl Builder {
    pub(crate) fn vert(&mut self, p: [f32; 3], n: [f32; 3], uv: [f32; 2], c: [u8; 3], m: u32) -> u32 {
        self.v.push(PropVertex { pos: p, normal: pack_n(n), uv: uv16(uv[0], uv[1]), color: [c[0], c[1], c[2], 255], material: m, pad: 0 });
        self.v.len() as u32 - 1
    }
    pub(crate) fn tri(&mut self, cut: bool, a: u32, b: u32, c: u32) {
        let l = if cut { &mut self.cut } else { &mut self.opaque };
        l.extend_from_slice(&[a, b, c]);
    }
    /// A capped tapered cylinder along +y from y0 to y1 (radii r0, r1) at (cx, cz).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn cylinder(&mut self, c: [f32; 3], y0: f32, y1: f32, r0: f32, r1: f32, sides: u32, col: [u8; 3], m: u32, cap: bool) {
        let base = self.v.len() as u32;
        for i in 0..=sides {
            let a = i as f32 / sides as f32 * std::f32::consts::TAU;
            let (s, co) = a.sin_cos();
            self.vert([c[0] + co * r0, c[1] + y0, c[2] + s * r0], [co, 0.0, s], [i as f32 / sides as f32, 0.0], col, m);
            self.vert([c[0] + co * r1, c[1] + y1, c[2] + s * r1], [co, 0.0, s], [i as f32 / sides as f32, 1.0], col, m);
        }
        for i in 0..sides {
            let (a, b, c2, d) = (base + 2 * i, base + 2 * i + 1, base + 2 * i + 2, base + 2 * i + 3);
            self.tri(false, a, c2, b);
            self.tri(false, b, c2, d);
        }
        if cap {
            let top = self.vert([c[0], c[1] + y1, c[2]], [0.0, 1.0, 0.0], [0.5, 0.5], col, m);
            let ring = self.v.len() as u32;
            for i in 0..=sides {
                let a = i as f32 / sides as f32 * std::f32::consts::TAU;
                self.vert([c[0] + a.cos() * r1, c[1] + y1, c[2] + a.sin() * r1], [0.0, 1.0, 0.0], [0.5, 0.5], col, m);
            }
            for i in 0..sides {
                self.tri(false, top, ring + i + 1, ring + i);
            }
        }
    }
    /// An axis-aligned box from min to max.
    pub(crate) fn cuboid(&mut self, mn: [f32; 3], mx: [f32; 3], col: [u8; 3], m: u32) {
        let faces: [([f32; 3], [usize; 4]); 6] = [
            ([1.0, 0.0, 0.0], [1, 3, 7, 5]),
            ([-1.0, 0.0, 0.0], [4, 6, 2, 0]),
            ([0.0, 1.0, 0.0], [2, 6, 7, 3]),
            ([0.0, -1.0, 0.0], [0, 1, 5, 4]),
            ([0.0, 0.0, 1.0], [4, 5, 7, 6]),
            ([0.0, 0.0, -1.0], [0, 2, 3, 1]),
        ];
        let corner = |k: usize| [if k & 1 != 0 { mx[0] } else { mn[0] }, if k & 2 != 0 { mx[1] } else { mn[1] }, if k & 4 != 0 { mx[2] } else { mn[2] }];
        for (n, q) in faces {
            let b = self.v.len() as u32;
            for (j, &k) in q.iter().enumerate() {
                self.vert(corner(k), n, [(j == 1 || j == 2) as u8 as f32, (j >= 2) as u8 as f32], col, m);
            }
            self.tri(false, b, b + 1, b + 2);
            self.tri(false, b, b + 2, b + 3);
        }
    }
    /// A quad facing +z (normal +z), centred at c, w x h, with a uv rectangle.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn quad_z(&mut self, c: [f32; 3], w: f32, h: f32, uv: [f32; 4], col: [u8; 3], m: u32, cut: bool) {
        let b = self.v.len() as u32;
        let n = [0.0, 0.0, 1.0];
        self.vert([c[0] - w / 2.0, c[1] - h / 2.0, c[2]], n, [uv[0], uv[1]], col, m);
        self.vert([c[0] + w / 2.0, c[1] - h / 2.0, c[2]], n, [uv[2], uv[1]], col, m);
        self.vert([c[0] + w / 2.0, c[1] + h / 2.0, c[2]], n, [uv[2], uv[3]], col, m);
        self.vert([c[0] - w / 2.0, c[1] + h / 2.0, c[2]], n, [uv[0], uv[3]], col, m);
        self.tri(cut, b, b + 1, b + 2);
        self.tri(cut, b, b + 2, b + 3);
    }
}

pub(crate) fn srgb_hex(h: &str) -> [u8; 3] {
    let v = u32::from_str_radix(h.trim_start_matches('#'), 16).unwrap_or(0xffffff);
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

pub(crate) fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

pub(crate) struct Glb {
    pub(crate) positions: Vec<[f32; 3]>,
    pub(crate) normals: Vec<[f32; 3]>,
    pub(crate) uvs: Vec<[f32; 2]>,
    pub(crate) indices: Vec<u32>,
    pub(crate) maps: [Option<(usize, usize, Vec<u8>)>; 3],
    /// (`--luminaires`) the vertex ranges (first, count) of the primitives
    /// whose material emits (a luminaire's `lamp_glass`): the glass its lamp
    /// lights. The loader draws them with the rest unless the cook splits them out.
    pub(crate) glow: Vec<(u32, u32)>,
}

/// (`--luminaires`) A luminaire's lit glass as separate parts (a twin post's
/// two lanterns, a cobra head's one lens): the glow primitives' vertices
/// gathered into boxes that touch (a lantern's faces each have their own
/// vertices), each part's model-space box (min, max). Its centre is where the
/// lamp's light stands.
pub(crate) fn glow_parts(g: &Glb) -> Vec<([f32; 3], [f32; 3])> {
    const TOUCH_M: f32 = 0.05;
    let mut boxes: Vec<([f32; 3], [f32; 3])> = vec![];
    for &(first, n) in &g.glow {
        // each triangle of the range a box; merged below
        for t in g.indices.chunks_exact(3) {
            if t.iter().all(|&i| i >= first && i < first + n) {
                let mut mn = [f32::MAX; 3];
                let mut mx = [f32::MIN; 3];
                for &i in t {
                    let p = g.positions[i as usize];
                    for a in 0..3 {
                        mn[a] = mn[a].min(p[a]);
                        mx[a] = mx[a].max(p[a]);
                    }
                }
                boxes.push((mn, mx));
            }
        }
    }
    let touch = |a: &([f32; 3], [f32; 3]), b: &([f32; 3], [f32; 3])| (0..3).all(|k| a.0[k] <= b.1[k] + TOUCH_M && b.0[k] <= a.1[k] + TOUCH_M);
    loop {
        let mut merged = false;
        let mut i = 0;
        while i < boxes.len() {
            let mut j = i + 1;
            while j < boxes.len() {
                if touch(&boxes[i], &boxes[j]) {
                    let b = boxes.swap_remove(j);
                    for k in 0..3 {
                        boxes[i].0[k] = boxes[i].0[k].min(b.0[k]);
                        boxes[i].1[k] = boxes[i].1[k].max(b.1[k]);
                    }
                    merged = true;
                } else {
                    j += 1;
                }
            }
            i += 1;
        }
        if !merged {
            break;
        }
    }
    boxes.sort_by(|a, b| a.0[0].total_cmp(&b.0[0]));
    boxes
}

/// (`--luminaires`) A luminaire's glass parts as its lamp's sockets: each
/// part's centre (model metres) and the shading the glass takes (props_draw.wgsl
/// `pr_lamp_glass`): `[kind, area, height, share]`, kind 1 a lantern's frosted
/// panes seen from the side (area its side's projected width x height), kind 2
/// a flat lens facing down (area its plan), the part's centre height, and the
/// part's share of the post's light.
pub fn luminaire_sockets(parts: &[([f32; 3], [f32; 3])]) -> Vec<([f32; 3], [f32; 4])> {
    let n = parts.len().max(1) as f32;
    parts
        .iter()
        .map(|(mn, mx)| {
            let c = [(mn[0] + mx[0]) * 0.5, (mn[1] + mx[1]) * 0.5, (mn[2] + mx[2]) * 0.5];
            let (w, h, d) = (mx[0] - mn[0], mx[1] - mn[1], mx[2] - mn[2]);
            let flat = h < 0.25 * w.min(d);
            let params = if flat { [2.0, (w * d).max(1e-3), c[1], 1.0 / n] } else { [1.0, (w.max(d) * h).max(1e-3), c[1], 1.0 / n] };
            (c, params)
        })
        .collect()
}

/// (`--luminaires`) The sockets of the district lights' GLBs (`lampCobra`,
/// `lampBanner`, `lampHistoric`) present in the checkout, by model (eo-app
/// props: their lights stand at the sockets). A kind without its GLB, or a
/// GLB without lit glass, has none (its light stays at the JS head).
pub fn district_light_sockets(repo: &Path, js: &JsProps) -> Vec<(eo_props::Model, Vec<([f32; 3], [f32; 4])>)> {
    district_light_sockets_with(repo, |name| js.street_prop(name).ok().cloned())
}

/// `district_light_sockets` with the street props read by `prop` (kind -> file).
pub fn district_light_sockets_with(repo: &Path, prop: impl Fn(&str) -> Option<eo_props::data::StreetProp>) -> Vec<(eo_props::Model, Vec<([f32; 3], [f32; 4])>)> {
    use eo_props::Model;
    [("lampCobra", Model::LampCobra), ("lampBanner", Model::LampBanner), ("lampHistoric", Model::LampHistoric)]
        .into_iter()
        .filter_map(|(name, model)| {
            let g = read_glb(repo, &prop(name)?).ok()?;
            let s = luminaire_sockets(&glow_parts(&g));
            (!s.is_empty()).then_some((model, s))
        })
        .collect()
}

/// Read a hash-verified GLB: the default scene's meshes in world frame, and
/// the first material's base colour, normal and metallic-roughness JPEGs.
pub(crate) fn read_glb(repo: &Path, prop: &eo_props::data::StreetProp) -> Result<Glb> {
    let asset = eo_assets::manifest::AssetRef { url: format!("/{}", prop.path.trim_start_matches("public/")), sha256: prop.sha256.clone(), bytes: prop.bytes, storey: None };
    let path = repo.join(&prop.path);
    let bytes = eo_assets::glb::read_verified(&path, &asset)?;
    let gltf = gltf::Gltf::from_slice(&bytes).map_err(|e| anyhow!("glTF parse: {e}"))?;
    let blob = gltf.blob.as_deref().ok_or_else(|| anyhow!("GLB without a binary chunk"))?;
    let mut g = Glb { positions: vec![], normals: vec![], uvs: vec![], indices: vec![], maps: [None, None, None], glow: vec![] };
    let scene = gltf.default_scene().or_else(|| gltf.scenes().next()).ok_or_else(|| anyhow!("no scene"))?;
    let mut stack: Vec<(gltf::Node, glam::Mat4)> = scene.nodes().map(|n| (n, glam::Mat4::IDENTITY)).collect();
    // A v2 file's blended primitives (the shelter's glazing) are left out: this loader draws
    // one opaque material, and an opaque pane would stand as a grey wall.
    let skip_blend = eo_props::props_v2::active().is_some_and(|v| v.is_v2(prop));
    while let Some((node, parent)) = stack.pop() {
        let m = parent * glam::Mat4::from_cols_array_2d(&node.transform().matrix());
        if let Some(mesh) = node.mesh() {
            let nm = glam::Mat3::from_mat4(m).inverse().transpose();
            for prim in mesh.primitives() {
                if skip_blend && prim.material().alpha_mode() == gltf::material::AlphaMode::Blend {
                    continue;
                }
                let r = prim.reader(|b| if b.index() == 0 { Some(blob) } else { None });
                let base = g.positions.len() as u32;
                let pos: Vec<[f32; 3]> = r.read_positions().ok_or_else(|| anyhow!("no positions"))?.collect();
                let n = pos.len();
                if prim.material().emissive_factor().iter().any(|e| *e > 0.0) {
                    g.glow.push((base, n as u32)); // (`--luminaires`) a luminaire's lit glass
                }
                g.positions.extend(pos.iter().map(|p| m.transform_point3(glam::Vec3::from(*p)).to_array()));
                match r.read_normals() {
                    Some(it) => g.normals.extend(it.map(|q| (nm * glam::Vec3::from(q)).normalize_or_zero().to_array())),
                    None => g.normals.extend(std::iter::repeat_n([0.0, 1.0, 0.0], n)),
                }
                match r.read_tex_coords(0) {
                    Some(t) => g.uvs.extend(t.into_f32()),
                    None => g.uvs.extend(std::iter::repeat_n([0.0, 0.0], n)),
                }
                match r.read_indices() {
                    Some(ix) => g.indices.extend(ix.into_u32().map(|i| base + i)),
                    None => g.indices.extend(base..base + n as u32),
                }
            }
        }
        for c in node.children() {
            stack.push((c, m));
        }
    }
    if let Some(mat) = gltf.materials().next() {
        let pbr = mat.pbr_metallic_roughness();
        let tex = [pbr.base_color_texture().map(|t| t.texture()), mat.normal_texture().map(|t| t.texture()), pbr.metallic_roughness_texture().map(|t| t.texture())];
        for (k, t) in tex.into_iter().enumerate() {
            let Some(t) = t else { continue };
            if let gltf::image::Source::View { view, .. } = t.source().source() {
                let s = view.offset();
                let data = &blob[s..s + view.length()];
                let (w, h, rgb) = eo_assets::jpeg::decode_rgb(data).with_context(|| format!("{}: image {k}", prop.path))?;
                g.maps[k] = Some((w, h, rgb));
            } else {
                bail!("{}: external image refused", prop.path);
            }
        }
    }
    Ok(g)
}

/// Box-filtered / bilinear resample of RGB(A) to `res` x `res` RGBA.
pub(crate) fn resample(w: usize, h: usize, src: &[u8], ch: usize, res: u32) -> Vec<u8> {
    let r = res as usize;
    let mut out = vec![0u8; r * r * 4];
    let (sx, sy) = (w as f32 / r as f32, h as f32 / r as f32);
    for y in 0..r {
        for x in 0..r {
            let (x0, x1) = ((x as f32 * sx) as usize, (((x + 1) as f32 * sx) as usize).max((x as f32 * sx) as usize + 1).min(w));
            let (y0, y1) = ((y as f32 * sy) as usize, (((y + 1) as f32 * sy) as usize).max((y as f32 * sy) as usize + 1).min(h));
            let mut acc = [0u32; 4];
            let mut n = 0u32;
            for yy in y0..y1 {
                for xx in x0..x1 {
                    let o = (yy * w + xx) * ch;
                    for c in 0..ch.min(4) {
                        acc[c] += src[o + c] as u32;
                    }
                    if ch == 3 {
                        acc[3] += 255;
                    }
                    n += 1;
                }
            }
            let o = (y * r + x) * 4;
            for c in 0..4 {
                out[o + c] = (acc[c] / n.max(1)) as u8;
            }
        }
    }
    out
}

pub(crate) fn push_mat(v: &mut Vec<PropMaterial>, base: [u8; 3], rough: f32, metal: f32, flags: u32, layer: i32) -> u32 {
    let b = base.map(|c| srgb_to_linear(c as f32 / 255.0));
    v.push(PropMaterial { base: [b[0], b[1], b[2], 1.0], emissive: [0.0; 4], params: [rough, metal, 1.0, 0.0], layer, flags, rough_flatten: 0.0, thin_absorption: 0.0 });
    v.len() as u32 - 1
}

pub(crate) fn glb_mat(v: &mut Vec<PropMaterial>, layer: i32) -> u32 {
    v.push(PropMaterial { base: [1.0; 4], emissive: [0.0; 4], params: [1.0, 1.0, 1.0, 0.0], layer, flags: P_TEX | P_GLB_MAPS, rough_flatten: 0.0, thin_absorption: 0.0 });
    v.len() as u32 - 1
}

/// carry the colour of the opaque texels out into the transparent ones
/// (`passes` rings of 4-neighbour averages, the rest the mean opaque colour), alpha
/// untouched, so the mip chain of an alpha-tested atlas does not average its leaves
/// with the transparent background's colour (pale fringes on every card at distance).
pub(crate) fn dilate_colour(rgba: &mut [u8], res: usize, passes: usize) {
    let n = res * res;
    let mut filled: Vec<bool> = (0..n).map(|i| rgba[i * 4 + 3] >= 128).collect();
    let (mut sum, mut cnt) = ([0u64; 3], 0u64);
    for i in 0..n {
        if filled[i] {
            for c in 0..3 {
                sum[c] += rgba[i * 4 + c] as u64;
            }
            cnt += 1;
        }
    }
    for _ in 0..passes {
        let mut next = filled.clone();
        let mut any = false;
        for y in 0..res {
            for x in 0..res {
                let i = y * res + x;
                if filled[i] {
                    continue;
                }
                let (mut acc, mut k) = ([0u32; 3], 0u32);
                for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= res as i32 || ny >= res as i32 {
                        continue;
                    }
                    let j = ny as usize * res + nx as usize;
                    if filled[j] {
                        for c in 0..3 {
                            acc[c] += rgba[j * 4 + c] as u32;
                        }
                        k += 1;
                    }
                }
                if k > 0 {
                    for c in 0..3 {
                        rgba[i * 4 + c] = (acc[c] / k) as u8;
                    }
                    next[i] = true;
                    any = true;
                }
            }
        }
        filled = next;
        if !any {
            break;
        }
    }
    if cnt > 0 {
        for i in 0..n {
            if !filled[i] {
                for c in 0..3 {
                    rgba[i * 4 + c] = (sum[c] / cnt) as u8;
                }
            }
        }
    }
}

pub(crate) fn flat_layer(rgba: [u8; 4], res: u32) -> Vec<u8> {
    rgba.iter().copied().cycle().take((res * res * 4) as usize).collect()
}

pub(crate) fn decode_png(bytes: &[u8]) -> Result<(usize, usize, Vec<u8>)> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = dec.read_info()?;
    let mut buf = vec![0u8; r.output_buffer_size().ok_or_else(|| anyhow!("png too large"))?];
    let info = r.next_frame(&mut buf)?;
    let px = (info.width * info.height) as usize;
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..px * 4].to_vec(),
        png::ColorType::Rgb => buf[..px * 3].chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf[..px * 2].chunks_exact(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Grayscale => buf[..px].iter().flat_map(|&g| [g, g, g, 255]).collect(),
        other => bail!("png colour type {other:?}"),
    };
    Ok((info.width as usize, info.height as usize, rgba))
}

impl PropLibrary {
    /// The cache key of the library: version, the JS export, the GLBs' digests.
    pub fn key(js: &JsProps) -> String {
        Self::key_with(js, &crate::props_lib2::LibExtras::default(), [0.0; 4])
    }

    /// the key over every input (the ez-tree bake's street forms, the sign atlases, the overhead export, the domes).
    pub fn key_with(js: &JsProps, x: &crate::props_lib2::LibExtras, dome: [f32; 4]) -> String {
        use sha2::Digest;
        let mut h = sha2::Sha256::new();
        h.update(PROPS_LIB_VERSION.as_bytes());
        h.update(eo_props::data::PROPS_JS_GZ);
        h.update(include_bytes!("props_lib.rs"));
        h.update(include_bytes!("props_lib2.rs"));
        for p in js.street_props.values() {
            h.update(p.sha256.as_bytes());
        }
        h.update(bytemuck::cast_slice(&dome));
        if let Some(ez) = x.ez {
            for ((sp, form), m) in &ez.street {
                h.update(sp.as_bytes());
                h.update(format!("{form:?}").as_bytes());
                h.update(bytemuck::cast_slice(&m.mesh.positions));
                h.update(bytemuck::cast_slice(&m.mesh.colors));
                h.update(bytemuck::cast_slice(&m.mesh.indices));
            }
        }
        if let Some(s) = x.signs {
            h.update(&s.faces.rgba);
            h.update(bytemuck::cast_slice(&s.faces.tiles));
            h.update(&s.printed.rgba);
        }
        if x.overhead.is_some() {
            h.update(eo_props::overhead::OVERHEAD_JS_GZ);
        }
        // The stand-ins (`LibExtras::stand_ins`) only when on: off, the key is over the same
        // inputs as before the switch (its source bytes above changed with it anyway).
        if x.stand_ins {
            h.update(b"stand-ins");
        }
        // (`--luminaires`) the lamps' glass split out only when on (off: the key as before).
        if x.luminaires {
            h.update(b"luminaires");
        }
        // The boats only when drawn (`--no-boats`: the key over the inputs before them).
        if let Some(bodies) = x.boats {
            for l in bodies.iter().flat_map(|b| b.lods.iter()) {
                h.update(l.sha256.as_bytes());
            }
        }
        h.finalize().iter().take(12).map(|b| format!("{b:02x}")).collect()
    }

    pub fn load(repo: &Path, js: &JsProps, lens_dia_m: f64, cache_dir: Option<&Path>) -> Result<Self> {
        Self::load_full(repo, js, lens_dia_m, &crate::props_lib2::LibExtras::default(), [0.0; 4], cache_dir)
    }

    /// the library with the furniture kinds, the ez-tree trees, the sign atlases and the overhead.
    /// `dome`: `DETECTABLE_WARNING_PAD` (base diameter, height, pitch max, depth), from `caMetric`.
    pub fn load_full(repo: &Path, js: &JsProps, lens_dia_m: f64, x: &crate::props_lib2::LibExtras, dome: [f32; 4], cache_dir: Option<&Path>) -> Result<Self> {
        // A checkout without some prop files cooks (and caches) a library of its own.
        let boat_files = x.boats.unwrap_or_default().iter().flat_map(|b| b.lods.iter());
        let absent: String = js.street_props.values().chain(boat_files).map(|p| if repo.join(&p.path).is_file() { '1' } else { '0' }).collect();
        let tag = if absent.contains('0') { format!("-{absent}") } else { String::new() };
        let path = cache_dir.map(|d| d.join("props").join(format!("{}{tag}.eopl", Self::key_with(js, x, dome))));
        if let Some(p) = &path {
            if let Ok(l) = Self::read_cache(p) {
                return Ok(l);
            }
        }
        let lib = Self::cook_full(repo, js, lens_dia_m, x, dome)?;
        if let Some(p) = &path {
            if let Err(e) = lib.write_cache(p) {
                eprintln!("props: library cache not written ({e:#})");
            }
        }
        Ok(lib)
    }

    /// Cook the library. `lens_dia_m`: `caMetric('signal-lens-size', 'circularNewFaces')`.
    pub fn cook(repo: &Path, js: &JsProps, lens_dia_m: f64) -> Result<Self> {
        Self::cook_full(repo, js, lens_dia_m, &crate::props_lib2::LibExtras::default(), [0.0; 4])
    }

    /// `cook` with the extras (see `load_full`).
    pub fn cook_full(repo: &Path, js: &JsProps, lens_dia_m: f64, extras: &crate::props_lib2::LibExtras, dome: [f32; 4]) -> Result<Self> {
        let num = |g: &str, k: &str| js.num(g, k) as f32;
        let mut layers: Vec<[Vec<u8>; 3]> = vec![];
        let mut big_layers: Vec<Vec<u8>> = vec![];
        let flat_n = flat_layer([128, 128, 255, 255], AUX_RES);
        let flat_orm = flat_layer([255, 255, 0, 255], AUX_RES);
        layers.push([flat_layer([255, 255, 255, 255], ALBEDO_RES), flat_n.clone(), flat_orm.clone()]);
        let mut materials: Vec<PropMaterial> = vec![];
        let mt = &mut materials;
        let mut mat = |base: [u8; 3], rough: f32, metal: f32, flags: u32, layer: i32| push_mat(mt, base, rough, metal, flags, layer);
        // Base colours are vertex colours (sRGB); the material carries response.
        let white = [255u8, 255, 255];
        // The JS `hardwareMaterial` (traffic-signal-heads.js): roughness 0.62, metalness 0.3.
        let m_metal = mat(white, 0.62, 0.3, 0, 0); // signal poles, arms, sign pipe
        let m_paint = mat(white, 0.6, 0.1, 0, 0); // painted housings
        // Sheeting on the face; the bare sheet behind is coloured in the shader.
        let m_plate = mat(white, 0.5, 0.0, P_PLATE, 0);
        let m_pad = mat(white, 0.8, 0.0, 0, 0);
        let m_bark = mat(white, 0.9, 0.0, P_TEX | P_BIG, -1); // layer set below // props2: big array
        let m_leaf = mat(white, 0.7, 0.0, P_TEX | P_CUT | P_BIG | P_LEAF, -1);
        let m_lens: Vec<u32> = (0..3).map(|k| mat([10, 10, 10], 0.2, 0.0, P_LENS | ((k as u32) << P_LENS_SHIFT), 0)).collect();
        let m_hand = mat([20, 20, 20], 0.3, 0.0, P_PED_HAND, 0);
        let m_walk = mat([20, 20, 20], 0.3, 0.0, P_PED_WALK, 0);
        let mut missing = vec![];
        let mut sources = vec![];
        let glb_layer = |name: &str, layers: &mut Vec<[Vec<u8>; 3]>, sources: &mut Vec<String>, missing: &mut Vec<String>| -> Option<(Glb, i32)> {
            let prop = js.street_prop(name).ok()?;
            match read_glb(repo, prop) {
                Ok(g) => {
                    let map = |k: usize, res: u32, default: &Vec<u8>| match &g.maps[k] {
                        Some((w, h, rgb)) => resample(*w, *h, rgb, 3, res),
                        None => default.clone(),
                    };
                    let white_a = flat_layer([255; 4], ALBEDO_RES);
                    layers.push([map(0, ALBEDO_RES, &white_a), map(1, AUX_RES, &flat_n), map(2, AUX_RES, &flat_orm)]);
                    sources.push(format!("{name}: {} (sha256 {})", prop.path, &prop.sha256[..12]));
                    Some((g, layers.len() as i32 - 1))
                }
                Err(e) => {
                    missing.push(format!("{name}: {} ({e:#})", prop.path));
                    None
                }
            }
        };
        // Vegetation atlas (albedo + alpha) at its own 2048 px: big layer 0. // props2
        let atlas = &js.consts["vegetationAtlas"];
        let atlas_path = repo.join("public").join(atlas["url"].as_str().unwrap_or("assets/vegetation/ez-vegetation-atlas-v1.png"));
        let atlas_ok = match std::fs::read(&atlas_path).map_err(anyhow::Error::from).and_then(|b| decode_png(&b)) {
            Ok((w, h, rgba)) => {
                let mut l = if w == BIG_RES as usize && h == BIG_RES as usize { rgba } else { resample(w, h, &rgba, 4, BIG_RES) };
                dilate_colour(&mut l, BIG_RES as usize, 16); // no pale fringes in the mips
                big_layers.push(l);
                sources.push(format!("vegetation atlas: {}", atlas_path.display()));
                true
            }
            Err(e) => {
                missing.push(format!("vegetation atlas {} ({e:#}): trees draw untextured", atlas_path.display()));
                big_layers.push(flat_layer([255; 4], BIG_RES));
                false
            }
        };
        materials[m_bark as usize].layer = 0;
        materials[m_leaf as usize].layer = 0;
        if !atlas_ok {
            materials[m_leaf as usize].flags = 0;
            materials[m_bark as usize].flags = 0;
        }
        // the sign faces (big layer 1) and the printed picket faces (big layer 2, at x 0..256).
        match extras.signs {
            Some(s) => {
                let mut f = if s.faces.width == BIG_RES && s.faces.height == BIG_RES { s.faces.rgba.clone() } else { resample(s.faces.width as usize, s.faces.height as usize, &s.faces.rgba, 4, BIG_RES) };
                dilate_colour(&mut f, BIG_RES as usize, 8);
                big_layers.push(f);
                let mut p = flat_layer([255; 4], BIG_RES);
                let (pw, ph) = (s.printed.width.min(BIG_RES) as usize, s.printed.height.min(BIG_RES) as usize);
                for y in 0..ph {
                    let src = &s.printed.rgba[y * s.printed.width as usize * 4..][..pw * 4];
                    p[y * BIG_RES as usize * 4..][..pw * 4].copy_from_slice(src);
                }
                big_layers.push(p);
                sources.push("sign faces and printed faces: eo_props::sign_atlas (system fonts at run time)".into());
            }
            None => {
                big_layers.push(flat_layer([255; 4], BIG_RES));
                big_layers.push(flat_layer([255; 4], BIG_RES));
            }
        }
        let head = glb_layer("signalHead", &mut layers, &mut sources, &mut missing);
        let palm = glb_layer("palm", &mut layers, &mut sources, &mut missing);
        let palm_far = glb_layer("palmFar", &mut layers, &mut sources, &mut missing);

        let mut b = Builder::default();
        let mut meshes: Vec<GpuPropMesh> = vec![];
        let finish = |b: &mut Builder, meshes: &mut Vec<GpuPropMesh>, all_v: &mut Vec<PropVertex>, all_i: &mut Vec<u32>| -> u32 {
            let base_vertex = all_v.len() as i32;
            all_v.append(&mut b.v);
            let first = all_i.len() as u32;
            let count = b.opaque.len() as u32;
            all_i.append(&mut b.opaque);
            let cut_first = all_i.len() as u32;
            let cut_count = b.cut.len() as u32;
            all_i.append(&mut b.cut);
            meshes.push(GpuPropMesh { first, count, cut_first, cut_count, base_vertex, pad: [0; 3] });
            meshes.len() as u32 - 1
        };
        let mut all_v: Vec<PropVertex> = vec![];
        let mut all_i: Vec<u32> = vec![];
        let hw = js.consts["signalHardwareColour"].as_u64().unwrap_or(0x2f3431) as u32;
        let grey = [(hw >> 16) as u8, (hw >> 8) as u8, hw as u8];
        let back = js.consts["plateBack"].as_u64().unwrap_or(0x9a9d9f) as u32;
        let zinc = [(back >> 16) as u8, (back >> 8) as u8, back as u8];
        let dark = srgb_hex("#2b2d2f");
        let r_top = num("signalHardware", "poleRadiusTopM");
        let r_base = num("signalHardware", "poleRadiusBaseM");
        let mut models = vec![GpuPropModel { meshes: [NONE; 4], ..Default::default() }; crate::props_lib2::RENDER_MODEL_COUNT.max(MODEL_COUNT)];
        let set = |models: &mut Vec<GpuPropModel>, m: Model, meshes: [u32; 3], lod: [f32; 4], center: [f32; 3], radius: f32, cast: u32| {
            models[m as usize] = GpuPropModel { meshes: [meshes[0], meshes[1], meshes[2], NONE], lod, center, radius, cast: [cast, 0, 0, 0] };
        };
        // Signal pole: unit height, scale.y = height (radii stay).
        for sides in [12u32, 6] {
            b.cylinder([0.0; 3], 0.0, 1.0, r_base, r_top, sides, grey, m_metal, true);
            if sides == 12 {
                b.cylinder([0.0; 3], 0.0, 0.42 / 6.9, 0.26, 0.2, sides, grey, m_metal, true);
            }
            finish(&mut b, &mut meshes, &mut all_v, &mut all_i);
        }
        let n = meshes.len() as u32;
        set(&mut models, Model::SignalPole, [n - 2, n - 1, n - 1], [40.0, 160.0, 400.0, 400.0], [0.0, 0.5, 0.0], 0.6, 2);
        // Mast arm: unit length along +x.
        for sides in [8u32, 4] {
            let bb = b.v.len();
            b.cylinder([0.0; 3], 0.0, 1.0, 0.09, 0.06, sides, grey, m_metal, false);
            for v in &mut b.v[bb..] {
                // y-up cylinder -> +x arm.
                let [x, y, z] = v.pos;
                v.pos = [y, x, z];
                let [nx, ny, nz, w] = v.normal;
                v.normal = [ny, nx, nz, w];
            }
            finish(&mut b, &mut meshes, &mut all_v, &mut all_i);
        }
        let n = meshes.len() as u32;
        set(&mut models, Model::MastArm, [n - 2, n - 1, n - 1], [40.0, 160.0, 400.0, 400.0], [0.5, 0.0, 0.0], 0.6, 2);
        // Vehicle head: the JS GLB body (or a box), three lenses on its face.
        let lens_r = (lens_dia_m / 2.0) as f32;
        // A v2 head (eo_props::props_v2) gives its lens face: its visors reach past the lenses.
        let v2_face = js.street_prop("signalHead").ok().and_then(|p| eo_props::props_v2::active().filter(|v| v.is_v2(p)).and_then(|v| v.lens_face_z()));
        let (face_z, mid_y) = match &head {
            Some((g, _)) if v2_face.is_some() => {
                let (mut y0, mut y1) = (f32::MAX, f32::MIN);
                for p in &g.positions {
                    y0 = y0.min(p[1]);
                    y1 = y1.max(p[1]);
                }
                (v2_face.unwrap_or_default() as f32 + 0.004, (y0 + y1) / 2.0)
            }
            Some((g, _)) => {
                let (mut zmax, mut y0, mut y1) = (f32::MIN, f32::MAX, f32::MIN);
                for p in &g.positions {
                    zmax = zmax.max(p[2]);
                    y0 = y0.min(p[1]);
                    y1 = y1.max(p[1]);
                }
                (zmax + 0.004, (y0 + y1) / 2.0)
            }
            None => (0.158, 0.0),
        };
        for lod in 0..2 {
            match (&head, lod) {
                (Some((g, layer)), 0) => {
                    let m = glb_mat(&mut materials, *layer);
                    let base = b.v.len() as u32;
                    for i in 0..g.positions.len() {
                        b.vert(g.positions[i], g.normals[i], g.uvs[i], white, m);
                    }
                    for t in g.indices.chunks_exact(3) {
                        b.tri(false, base + t[0], base + t[1], base + t[2]);
                    }
                }
                _ => b.cuboid([-0.21, -0.56, -0.14], [0.21, 0.56, face_z - 0.004], dark, m_paint),
            }
            for (k, dy) in [0.35f32, 0.0, -0.35].into_iter().enumerate() {
                let sides = if lod == 0 { 16 } else { 8 };
                let c = b.vert([0.0, mid_y + dy, face_z], [0.0, 0.0, 1.0], [0.5, 0.5], white, m_lens[k]);
                let ring = b.v.len() as u32;
                for i in 0..=sides {
                    let a = i as f32 / sides as f32 * std::f32::consts::TAU;
                    b.vert([a.cos() * lens_r, mid_y + dy + a.sin() * lens_r, face_z], [0.0, 0.0, 1.0], [0.5, 0.5], white, m_lens[k]);
                }
                for i in 0..sides {
                    b.tri(false, c, ring + i, ring + i + 1);
                }
            }
            finish(&mut b, &mut meshes, &mut all_v, &mut all_i);
        }
        let n = meshes.len() as u32;
        set(&mut models, Model::SignalHead, [n - 2, n - 1, n - 1], [45.0, 220.0, 400.0, 400.0], [0.0, 0.0, 0.0], 0.8, 1);
        // Pedestrian head: housing + hand and person faces side by side (JS PED_HEAD size).
        {
            let (w, h, d) = (num("pedHead", "widthM"), num("pedHead", "heightM"), num("pedHead", "depthM"));
            b.cuboid([-w / 2.0, -h / 2.0, -d], [w / 2.0, h / 2.0, 0.0], dark, m_paint);
            b.quad_z([-w / 4.0, 0.0, 0.003], w / 2.0 - 0.04, h - 0.06, [0.0, 0.0, 1.0, 1.0], white, m_hand, false);
            b.quad_z([w / 4.0, 0.0, 0.003], w / 2.0 - 0.04, h - 0.06, [0.0, 0.0, 1.0, 1.0], white, m_walk, false);
            finish(&mut b, &mut meshes, &mut all_v, &mut all_i);
        }
        let n = meshes.len() as u32;
        set(&mut models, Model::PedHead, [n - 1, n - 1, n - 1], [45.0, 120.0, 220.0, 220.0], [0.0, 0.0, -0.1], 0.4, 1);
        // Sign post: the JS pipe (manifest shaft radius), unit height.
        let pipe = js.consts["signPostRadiusM"].as_f64().unwrap_or(0.03) as f32;
        for sides in [10u32, 5] {
            b.cylinder([0.0; 3], 0.0, 1.0, pipe, pipe, sides, zinc, m_metal, true);
            finish(&mut b, &mut meshes, &mut all_v, &mut all_i);
        }
        let n = meshes.len() as u32;
        set(&mut models, Model::SignPost, [n - 2, n - 1, n - 1], [35.0, 120.0, 180.0, 180.0], [0.0, 0.5, 0.0], 0.6, 1);
        // Sign plate: a unit quad (front +z, coloured by face in the shader; the back bare sheet), 3 mm thick.
        // alpha-tested (the face atlas's alpha is the outline).
        {
            let bb = b.opaque.len();
            b.cuboid([-0.5, -0.5, -0.003], [0.5, 0.5, 0.0], white, m_plate);
            let moved: Vec<u32> = b.opaque.drain(bb..).collect();
            b.cut.extend(moved);
            finish(&mut b, &mut meshes, &mut all_v, &mut all_i);
        }
        let n = meshes.len() as u32;
        set(&mut models, Model::SignPlate, [n - 1, n - 1, n - 1], [60.0, 120.0, 180.0, 180.0], [0.0, 0.0, 0.0], 0.75, 1);
        // Street tree (unit crown diameter 1, height 1): trunk + leaf cards, three levels.
        {
            let cells = &atlas["cells"];
            let cell = |name: &str| -> [f32; 4] {
                let c = &cells[name];
                // Atlas UVs are y-up (three's flipY); the PNG rows are y-down.
                [c["u0"].as_f64().unwrap_or(0.0) as f32, 1.0 - c["v1"].as_f64().unwrap_or(1.0) as f32, c["u1"].as_f64().unwrap_or(1.0) as f32, 1.0 - c["v0"].as_f64().unwrap_or(0.0) as f32]
            };
            let bark = &atlas["bark"];
            let bark_uv = [0.0f32, 1.0 - bark["v1"].as_f64().unwrap_or(0.246) as f32, 0.25, 1.0 - bark["v0"].as_f64().unwrap_or(0.004) as f32];
            let leaf = cell("oak");
            let trunk_col = [255u8, 255, 255];
            let leaf_col = [255u8, 255, 255];
            for (lod, cards) in [(0usize, 36usize), (1, 10), (2, 0)] {
                // Trunk to the crown base.
                if lod < 2 {
                    let bb = b.v.len();
                    b.cylinder([0.0; 3], 0.0, 0.55, 0.035, 0.022, if lod == 0 { 7 } else { 4 }, trunk_col, m_bark, false);
                    for v in &mut b.v[bb..] {
                        let u = v.uv[0] as f32 / 65535.0;
                        let t = v.uv[1] as f32 / 65535.0;
                        v.uv = uv16(bark_uv[0] + (bark_uv[2] - bark_uv[0]) * u, bark_uv[1] + (bark_uv[3] - bark_uv[1]) * t);
                    }
                }
                // Leaf cards: a deterministic Fibonacci shell inside the crown ellipsoid (centre 0.66, half height 0.34, radius 0.5).
                let add_card =|b: &mut Builder, c: [f32; 3], size: f32, yaw: f32, tilt: f32| {
                    let (s, co) = yaw.sin_cos();
                    let (ts, tc) = tilt.sin_cos();
                    let ax = [co * size / 2.0, 0.0, -s * size / 2.0];
                    let ay = [s * ts * size / 2.0, tc * size / 2.0, co * ts * size / 2.0];
                    let nrm = [c[0], (c[1] - 0.66) * 1.5, c[2]];
                    let base = b.v.len() as u32;
                    for (j, (fx, fy)) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].iter().enumerate() {
                        let p = [c[0] + ax[0] * fx + ay[0] * fy, c[1] + ax[1] * fx + ay[1] * fy, c[2] + ax[2] * fx + ay[2] * fy];
                        let uv = [if j == 0 || j == 3 { leaf[0] } else { leaf[2] }, if j < 2 { leaf[3] } else { leaf[1] }];
                        b.vert(p, nrm, uv, leaf_col, m_leaf);
                    }
                    b.tri(true, base, base + 1, base + 2);
                    b.tri(true, base, base + 2, base + 3);
                };
                if lod < 2 {
                    let golden = std::f32::consts::PI * (3.0 - 5.0f32.sqrt());
                    for i in 0..cards {
                        let t = (i as f32 + 0.5) / cards as f32;
                        let yv = 1.0 - 2.0 * t;
                        let r = (1.0 - yv * yv).sqrt();
                        let a = golden * i as f32;
                        let shell = 0.62 + 0.3 * ((i * 7 % 5) as f32 / 4.0);
                        let c = [a.cos() * r * 0.5 * shell, 0.66 + yv * 0.34 * shell, a.sin() * r * 0.5 * shell];
                        let size = if lod == 0 { 0.42 } else { 0.62 };
                        add_card(&mut b, c, size, a + 1.3, (i as f32 * 0.9).sin() * 0.6);
                    }
                } else {
                    // Far form: two crossed cards over the whole crown, trunk included.
                    for yaw in [0.0f32, std::f32::consts::FRAC_PI_2] {
                        let (s, co) = yaw.sin_cos();
                        let base = b.v.len() as u32;
                        for (j, (fx, fy)) in [(-1.0f32, 0.0f32), (1.0, 0.0), (1.0, 1.0), (-1.0, 1.0)].iter().enumerate() {
                            let p = [co * 0.5 * fx, 0.32 + 0.68 * fy, -s * 0.5 * fx];
                            let uv = [if j == 0 || j == 3 { leaf[0] } else { leaf[2] }, if j < 2 { leaf[3] } else { leaf[1] }];
                            b.vert(p, [s, 0.3, co], uv, leaf_col, m_leaf);
                        }
                        b.tri(true, base, base + 1, base + 2);
                        b.tri(true, base, base + 2, base + 3);
                    }
                }
                finish(&mut b, &mut meshes, &mut all_v, &mut all_i);
            }
            let n = meshes.len() as u32;
            // JS street trees: near ring 120 m, window 260 m, far silhouettes beyond.
            set(&mut models, Model::Tree, [n - 3, n - 2, n - 1], [60.0, 160.0, 450.0, 450.0], [0.0, 0.6, 0.0], 0.75, 3);
        }
        // Palm: the JS GLBs (palm near, palmFar beyond), unit = 10 m tall.
        {
            let add_glb = |g: &Glb, layer: i32, b: &mut Builder, materials: &mut Vec<PropMaterial>| {
                let m = glb_mat(materials, layer);
                let base = b.v.len() as u32;
                for i in 0..g.positions.len() {
                    b.vert(g.positions[i], g.normals[i], g.uvs[i], white, m);
                }
                for t in g.indices.chunks_exact(3) {
                    b.tri(false, base + t[0], base + t[1], base + t[2]);
                }
            };
            let near = palm.as_ref().map(|(g, l)| (g, *l));
            let far = palm_far.as_ref().map(|(g, l)| (g, *l)).or(near);
            match (near, far) {
                (Some(nr), Some(fr)) => {
                    add_glb(nr.0, nr.1, &mut b, &mut materials);
                    finish(&mut b, &mut meshes, &mut all_v, &mut all_i);
                    add_glb(fr.0, fr.1, &mut b, &mut materials);
                    finish(&mut b, &mut meshes, &mut all_v, &mut all_i);
                    let n = meshes.len() as u32;
                    set(&mut models, Model::Palm, [n - 2, n - 1, n - 1], [120.0, 450.0, 450.0, 450.0], [0.0, 5.0, 0.0], 6.5, 3);
                }
                _ => {
                    // Proxy: a trunk; the JS draws palms only from the GLB (`installedOnly`).
                    b.cylinder([0.0; 3], 0.0, 1.0, 0.025, 0.02, 6, srgb_hex("#7a6a55"), m_paint, true);
                    finish(&mut b, &mut meshes, &mut all_v, &mut all_i);
                    let n = meshes.len() as u32;
                    set(&mut models, Model::Palm, [n - 1, n - 1, n - 1], [450.0, 450.0, 450.0, 450.0], [0.0, 0.5, 0.0], 0.6, 2);
                }
            }
        }
        // Tactile pad: unit quad (x -0.5..0.5, z -0.5..0.5) facing up, Federal Yellow (`domePadYellow`).
        {
            let yellow = srgb_hex(js.consts["domePadYellow"].as_str().unwrap_or("#cda336"));
            let base = b.v.len() as u32;
            for (x, z) in [(-0.5f32, -0.5f32), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)] {
                b.vert([x, 0.0, z], [0.0, 1.0, 0.0], [x + 0.5, z + 0.5], yellow, m_pad);
            }
            b.tri(false, base, base + 2, base + 1);
            b.tri(false, base, base + 3, base + 2);
            finish(&mut b, &mut meshes, &mut all_v, &mut all_i);
        }
        let n = meshes.len() as u32;
        set(&mut models, Model::TactilePad, [n - 1, n - 1, n - 1], [60.0, 90.0, 120.0, 120.0], [0.0, 0.0, 0.0], 0.75, 0);
        // Tree basin (eo-props `basins.rs`, `--tree-basins`): a unit panel (x across the walk,
        // z along the kerb, -0.5..0.5) facing up, flush with the walk, drawn by the cutout
        // pipeline: the bed is the shader's (props_draw.wgsl `pr_basin`: soil, clods, grit,
        // roots, leaf litter, the cut edge's shade), and at level 0 a fringe of
        // `BASIN_FRINGE` past the cut edge keeps only the crumbs and leaves spilled onto the
        // walk. Levels 1 and 2 are the bed alone. The vertex colour is the bed's mean.
        {
            let soil = srgb_hex("#4f4a44");
            let up = [0.0f32, 1.0, 0.0];
            let quad = |b: &mut Builder, r: f32| {
                let base = b.v.len() as u32;
                for (x, z) in [(-r, -r), (r, -r), (r, r), (-r, r)] {
                    b.vert([x, 0.0, z], up, [x + 0.5, z + 0.5], soil, m_pad);
                }
                b.tri(true, base, base + 2, base + 1);
                b.tri(true, base, base + 3, base + 2);
            };
            quad(&mut b, 0.5 + BASIN_FRINGE);
            finish(&mut b, &mut meshes, &mut all_v, &mut all_i);
            quad(&mut b, 0.5);
            finish(&mut b, &mut meshes, &mut all_v, &mut all_i);
            quad(&mut b, 0.5);
            finish(&mut b, &mut meshes, &mut all_v, &mut all_i);
        }
        let n = meshes.len() as u32;
        set(&mut models, Model::TreeBasin, [n - 3, n - 2, n - 1], [35.0, 70.0, 110.0, 110.0], [0.0, 0.0, 0.0], (0.5 + BASIN_FRINGE) * std::f32::consts::SQRT_2, 0);

        // the furniture kinds, the ez-tree trees and the overhead.
        let alpha_test = atlas["alphaTest"].as_f64().unwrap_or(0.42) as f32;
        let info = {
            let mut out = crate::props_lib2::Out { b: Builder::default(), meshes: &mut meshes, v: &mut all_v, i: &mut all_i, models: &mut models, materials: &mut materials, layers: &mut layers, sources: &mut sources, missing: &mut missing };
            crate::props_lib2::cook_extras(&mut out, repo, js, extras, m_bark, m_leaf, alpha_test)
        };

        // Plate faces (sRGB linearised in the shader) and the plate back.
        let faces_js = &js.consts["signFaceColours"];
        let mut faces: Vec<[f32; 4]> = js
            .signs
            .faces
            .iter()
            .map(|f| {
                let c = srgb_hex(faces_js[f.as_str()].as_str().unwrap_or("#f4f4f0"));
                [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0]
            })
            .collect();
        let back = js.consts["plateBack"].as_u64().unwrap_or(0x9a9d9f) as u32;
        faces.push([((back >> 16) & 255) as f32 / 255.0, ((back >> 8) & 255) as f32 / 255.0, (back & 255) as f32 / 255.0, 1.0]);
        // then each face's atlas tile ([u0, v0 up, du, dv]; zero where there is no atlas).
        let face_count = js.signs.faces.len() as u32;
        for i in 0..face_count as usize {
            let t = extras.signs.and_then(|s| s.faces.tiles.get(i).copied()).unwrap_or([0.0; 4]);
            faces.push(t);
        }
        let tone = &js.consts["lensTone"];
        let mut lens = [[0.0f32; 4]; 6];
        for k in 0..3 {
            for c in 0..3 {
                lens[k][c] = tone["lit"][k][c].as_f64().unwrap_or(0.0) as f32;
                lens[3 + k][c] = tone["unlit"][k][c].as_f64().unwrap_or(0.0) as f32;
            }
        }
        lens[0][3] = tone["acesScale"].as_f64().unwrap_or(0.6) as f32;
        Ok(PropLibrary {
            vertices: all_v,
            indices: all_i,
            meshes,
            models,
            materials,
            layers,
            faces,
            lens,
            alpha_test,
            sources,
            missing,
            big_layers,
            tree_species: info.tree_species,
            tree_dims: info.tree_dims,
            face_count,
            dome,
        })
    }

    pub fn triangles(&self, mesh: u32) -> u32 {
        let m = self.meshes[mesh as usize];
        (m.count + m.cut_count) / 3
    }

    fn write_cache(&self, path: &Path) -> Result<()> {
        use sha2::Digest;
        let mut out: Vec<u8> = vec![];
        let put = |out: &mut Vec<u8>, b: &[u8]| {
            out.extend_from_slice(&(b.len() as u64).to_le_bytes());
            out.extend_from_slice(b);
        };
        put(&mut out, bytemuck::cast_slice(&self.vertices));
        put(&mut out, bytemuck::cast_slice(&self.indices));
        put(&mut out, bytemuck::cast_slice(&self.meshes));
        put(&mut out, bytemuck::cast_slice(&self.models));
        put(&mut out, bytemuck::cast_slice(&self.materials));
        put(&mut out, bytemuck::cast_slice(&self.faces));
        put(&mut out, bytemuck::cast_slice(&self.lens));
        put(&mut out, &self.alpha_test.to_le_bytes());
        put(&mut out, &(self.layers.len() as u64).to_le_bytes());
        for l in &self.layers {
            for m in l {
                put(&mut out, m);
            }
        }
        put(&mut out, self.sources.join("\n").as_bytes());
        put(&mut out, self.missing.join("\n").as_bytes());
        put(&mut out, &(self.big_layers.len() as u64).to_le_bytes());
        for l in &self.big_layers {
            put(&mut out, l);
        }
        put(&mut out, self.tree_species.join("\n").as_bytes());
        put(&mut out, bytemuck::cast_slice(&self.tree_dims));
        put(&mut out, &self.face_count.to_le_bytes());
        put(&mut out, bytemuck::cast_slice(&self.dome));
        let digest = sha2::Sha256::digest(&out);
        out.extend_from_slice(&digest);
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &out)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    fn read_cache(path: &Path) -> Result<Self> {
        use sha2::Digest;
        let bytes = std::fs::read(path)?;
        if bytes.len() < 32 {
            bail!("short");
        }
        let (body, digest) = bytes.split_at(bytes.len() - 32);
        if sha2::Sha256::digest(body).as_slice() != digest {
            bail!("props library cache digest mismatch");
        }
        let mut at = 0usize;
        let mut take = || -> Result<&[u8]> {
            let n = u64::from_le_bytes(body.get(at..at + 8).ok_or_else(|| anyhow!("truncated"))?.try_into()?) as usize;
            at += 8;
            let s = body.get(at..at + n).ok_or_else(|| anyhow!("truncated"))?;
            at += n;
            Ok(s)
        };
        let vertices = bytemuck::pod_collect_to_vec(take()?);
        let indices = bytemuck::pod_collect_to_vec(take()?);
        let meshes = bytemuck::pod_collect_to_vec(take()?);
        let models = bytemuck::pod_collect_to_vec(take()?);
        let materials = bytemuck::pod_collect_to_vec(take()?);
        let faces = bytemuck::pod_collect_to_vec(take()?);
        let lens_v: Vec<[f32; 4]> = bytemuck::pod_collect_to_vec(take()?);
        let alpha_test = f32::from_le_bytes(take()?.try_into()?);
        let nl = u64::from_le_bytes(take()?.try_into()?) as usize;
        let mut layers = vec![];
        for _ in 0..nl {
            layers.push([take()?.to_vec(), take()?.to_vec(), take()?.to_vec()]);
        }
        let s = String::from_utf8(take()?.to_vec())?;
        let m = String::from_utf8(take()?.to_vec())?;
        let nb = u64::from_le_bytes(take()?.try_into()?) as usize;
        let mut big_layers = vec![];
        for _ in 0..nb {
            big_layers.push(take()?.to_vec());
        }
        let species = String::from_utf8(take()?.to_vec())?;
        let tree_dims: Vec<[f32; 2]> = bytemuck::pod_collect_to_vec(take()?);
        let face_count = u32::from_le_bytes(take()?.try_into()?);
        let dome_v: Vec<f32> = bytemuck::pod_collect_to_vec(take()?);
        let mut dome = [0.0f32; 4];
        dome.copy_from_slice(&dome_v[..4]);
        let mut lens = [[0.0f32; 4]; 6];
        lens.copy_from_slice(&lens_v[..6]);
        Ok(Self {
            vertices,
            indices,
            meshes,
            models,
            materials,
            layers,
            faces,
            lens,
            alpha_test,
            sources: s.lines().map(String::from).collect(),
            missing: m.lines().filter(|l| !l.is_empty()).map(String::from).collect(),
            big_layers,
            tree_species: species.lines().filter(|l| !l.is_empty()).map(String::from).collect(),
            tree_dims,
            face_count,
            dome,
        })
    }
}
