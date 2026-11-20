//! the meshes the street-furniture walk, the ez-tree trees and the
//! street overhead add to the props library (`props_lib.rs` calls
//! [`cook_extras`]). Nothing here runs on the render thread.
//!
//! * Furniture kinds with a JS GLB (`STREET_PROP_ASSETS_V1`, read with the
//!   digest check, never modified): level 0 is the GLB as the JS loads it
//!   (the hydrant's albedo repainted off-white, the pay station stretched and
//!   repainted charcoal, as `street-prop-assets.js` does at load), levels 1
//!   and 2 are this module's own vertex-clustered reductions of it (the JS
//!   draws its procedural catalogue there; most kinds are `nearOnly`, so they
//!   stop at the JS near ring, 120 m, with a dithered fade).
//! * Kinds the JS builds in code (`buildCatalogueGeometry`): planter,
//!   standpipe, wall meter, scaffold, barricade, poster hoarding, pedestrian
//!   signal post, signal cabinet, at the JS dimensions and palette (simplified
//!   where the JS bakes per-vertex weathering; the poster bills are plain tones).
//! * Street trees: one render model per `STREET_TREE_SPECIES` entry, levels
//!   near / window / far from `eo_props::eztree` (`buildStreetTreeGeometry`;
//!   window and far by the species' silhouette, as the JS rings draw them),
//!   bark opaque and leaves alpha-tested on the JS vegetation atlas.
//! * The overhead (`street-overhead.js`): span and support poles (tapered
//!   tubes at `STREET_OVERHEAD`'s radii), the distribution pole and cobra arm
//!   GLBs, the controller cabinet, the corner sign cluster, the lantern GLB
//!   and one model per `SIGN_VARIANTS` entry (`createSignAssemblyGeometry`:
//!   post, plates with band and rules; printed stacks on the printed atlas).

use crate::props_lib::*;
use eo_props::{JsProps, Model};

/// Render model ids past `eo_props::MODEL_COUNT` (instances carry these).
pub const TREE_BASE: u32 = eo_props::MODEL_COUNT as u32;
pub const MAX_TREE_SPECIES: u32 = 32;
pub const OH_BASE: u32 = TREE_BASE + MAX_TREE_SPECIES;
pub const OH_POLE_STEEL: u32 = OH_BASE;
pub const OH_POLE_WOOD: u32 = OH_BASE + 1;
pub const OH_DIST_POLE: u32 = OH_BASE + 2;
pub const OH_COBRA_ARM: u32 = OH_BASE + 3;
pub const OH_CABINET: u32 = OH_BASE + 4;
pub const OH_CORNER_SIGN: u32 = OH_BASE + 5;
pub const OH_LANTERN: u32 = OH_BASE + 6;
pub const OH_SIGN_BASE: u32 = OH_BASE + 7;
pub const MAX_SIGN_VARIANTS: u32 = 32;
pub const RENDER_MODEL_COUNT: usize = (OH_SIGN_BASE + MAX_SIGN_VARIANTS) as usize;
/// Nominal size of the cabinet mesh (width X, height, depth Z), the JS `signalCabinetNear` shell.
pub const CABINET_NOMINAL: [f32; 3] = [0.74, 1.43, 0.5];
/// The fraction of a level's edge distance over which two levels cross-fade (dithered).
pub const FADE_BAND: f32 = 0.12;

/// `DETECTABLE_WARNING_PAD` (street-surface-detail.js): the dome base diameter and height
/// the middle of the standard's range (`detectable-warning-dome-geometry`), the pitch its
/// widest SF spacing (`spacingSF`, the JS counts domes from it), the panel depth
/// (`detectable-warning-size` `depthMin`). Every value through `caMetric`.
pub fn dome_standards(ca: &eo_data::CaStandards) -> anyhow::Result<[f32; 4]> {
    let base = ca.metric_range("detectable-warning-dome-geometry", "baseDia")?;
    let height = ca.metric_range("detectable-warning-dome-geometry", "height")?;
    let pitch = ca.metric_range("detectable-warning-dome-geometry", "spacingSF")?;
    let depth = ca.metric_key("detectable-warning-size", "depthMin")?;
    Ok([((base[0] + base[1]) / 2.0) as f32, ((height[0] + height[1]) / 2.0) as f32, pitch[1] as f32, depth as f32])
}

/// Inputs beyond the JS props export.
#[derive(Clone, Copy, Default)]
pub struct LibExtras<'a> {
    pub ez: Option<&'a eo_props::eztree::EzLibrary>,
    pub signs: Option<&'a eo_props::sign_atlas::SignAtlases>,
    pub overhead: Option<&'a eo_props::overhead::OverheadJs>,
}

/// What `cook_extras` reports back to the library.
#[derive(Default)]
pub struct ExtraInfo {
    pub tree_species: Vec<String>,
    /// Per species: (height m, max half crown m) for the card-tree fallback.
    pub tree_dims: Vec<[f32; 2]>,
}

pub(crate) struct Out<'a> {
    pub b: Builder,
    pub meshes: &'a mut Vec<GpuPropMesh>,
    pub v: &'a mut Vec<PropVertex>,
    pub i: &'a mut Vec<u32>,
    pub models: &'a mut Vec<GpuPropModel>,
    pub materials: &'a mut Vec<PropMaterial>,
    pub layers: &'a mut Vec<[Vec<u8>; 3]>,
    pub sources: &'a mut Vec<String>,
    pub missing: &'a mut Vec<String>,
}

impl Out<'_> {
    /// Close the builder's geometry as one mesh.
    fn finish(&mut self) -> u32 {
        let b = &mut self.b;
        let base_vertex = self.v.len() as i32;
        self.v.append(&mut b.v);
        let first = self.i.len() as u32;
        let count = b.opaque.len() as u32;
        self.i.append(&mut b.opaque);
        let cut_first = self.i.len() as u32;
        let cut_count = b.cut.len() as u32;
        self.i.append(&mut b.cut);
        self.meshes.push(GpuPropMesh { first, count, cut_first, cut_count, base_vertex, pad: [0; 3] });
        self.meshes.len() as u32 - 1
    }

    /// The bounding sphere of meshes (box centre, farthest vertex).
    fn sphere(&self, meshes: &[u32]) -> ([f32; 3], f32) {
        let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
        let verts = |m: u32| {
            let g = self.meshes[m as usize];
            let ids: Vec<u32> = self.i[g.first as usize..(g.first + g.count) as usize].iter().chain(self.i[g.cut_first as usize..(g.cut_first + g.cut_count) as usize].iter()).copied().collect();
            ids.into_iter().map(move |k| (g.base_vertex + k as i32) as usize)
        };
        for &m in meshes {
            for k in verts(m) {
                let p = self.v[k].pos;
                for a in 0..3 {
                    mn[a] = mn[a].min(p[a]);
                    mx[a] = mx[a].max(p[a]);
                }
            }
        }
        if mn[0] > mx[0] {
            return ([0.0; 3], 0.5);
        }
        let c = [(mn[0] + mx[0]) / 2.0, (mn[1] + mx[1]) / 2.0, (mn[2] + mx[2]) / 2.0];
        let mut r = 0.0f32;
        for &m in meshes {
            for k in verts(m) {
                let p = self.v[k].pos;
                r = r.max(((p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2) + (p[2] - c[2]).powi(2)).sqrt());
            }
        }
        (c, r.max(0.05))
    }

    fn set(&mut self, id: u32, meshes: [u32; 3], lod: [f32; 4], cast: u32, pad: f32) {
        let (c, r) = self.sphere(&meshes);
        self.models[id as usize] = GpuPropModel { meshes: [meshes[0], meshes[1], meshes[2], NONE], lod, center: c, radius: r + pad, cast: [cast, 0, 0, 0] };
    }
}

// ---------------------------------------------------------------- colours and primitives (street-furniture.js)

fn lin(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

pub(crate) fn to_srgb8(l: f32) -> u8 {
    let l = l.clamp(0.0, 1.0);
    let s = if l <= 0.0031308 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0).round() as u8
}

/// `tint(geometry, hex, shade)`: the hex in linear, times shade, back to the vertex sRGB.
fn tone(hex: &str, shade: f32) -> [u8; 3] {
    let c = srgb_hex(hex);
    [to_srgb8(lin(c[0]) * shade), to_srgb8(lin(c[1]) * shade), to_srgb8(lin(c[2]) * shade)]
}

/// A linear colour (the overhead's `SIGN_COLOURS`) as vertex sRGB.
fn lin3(c: [f32; 3]) -> [u8; 3] {
    [to_srgb8(c[0]), to_srgb8(c[1]), to_srgb8(c[2])]
}

/// The JS `PALETTE` (street-furniture.js ~768) entries these kinds use.
fn pal(name: &str) -> &'static str {
    match name {
        "planterConcrete" => "#b9b3a4",
        "planterSoil" => "#4a3d30",
        "canopyB" => "#728255",
        "standpipeBrass" => "#9a7c3c",
        "standpipeBody" => "#8e3a30",
        "meterBoxGrey" => "#8d908c",
        "meterPole" => "#63676a",
        "signWhite" => "#eae6dc",
        "poleGrey" => "#7c8083",
        "signalHousing" => "#2f3336",
        "signalVisor" => "#23272a",
        "signFace" => "#efece4",
        "buttonYellow" => "#b8a33d",
        "cabinetShell" => "#9aa09c",
        "cabinetDoor" => "#8a908d",
        "cabinetTrim" => "#4d5254",
        "cabinetPlinth" => "#8f8b80",
        "plateSteel" => "#5e6266",
        "scaffoldTube" => "#9ea3a5",
        "scaffoldPlank" => "#8f7a55",
        "scaffoldTie" => "#c0392b",
        "barricadeOrange" => "#d3641f",
        "barricadeWhite" => "#e8e4dc",
        "barricadeLeg" => "#4c4f52",
        "coneOrange" => "#e0652a",
        "hoardingPly" => "#a98a5c",
        "hoardingPlyB" => "#9c7f54",
        "hoardingPost" => "#7b6446",
        _ => "#808080",
    }
}

/// `box(w, h, d, hex, x, y, z, shade)`: centred at (x, y, z).
fn bx(b: &mut Builder, m: u32, w: f32, h: f32, d: f32, hex: &str, x: f32, y: f32, z: f32, shade: f32) {
    b.cuboid([x - w / 2.0, y - h / 2.0, z - d / 2.0], [x + w / 2.0, y + h / 2.0, z + d / 2.0], tone(hex, shade), m);
}

/// A box rotated about its own centre (Z then X), then placed.
fn bx_rot(b: &mut Builder, m: u32, w: f32, h: f32, d: f32, col: [u8; 3], rz: f32, rx: f32, at: [f32; 3]) {
    let from = b.v.len();
    b.cuboid([-w / 2.0, -h / 2.0, -d / 2.0], [w / 2.0, h / 2.0, d / 2.0], col, m);
    let (sz, cz) = rz.sin_cos();
    let (sx, cx) = rx.sin_cos();
    for v in &mut b.v[from..] {
        let rot = |p: [f32; 3]| {
            let p = [p[0] * cz - p[1] * sz, p[0] * sz + p[1] * cz, p[2]];
            [p[0], p[1] * cx - p[2] * sx, p[1] * sx + p[2] * cx]
        };
        let p = rot(v.pos);
        v.pos = [p[0] + at[0], p[1] + at[1], p[2] + at[2]];
        let n = rot([v.normal[0] as f32, v.normal[1] as f32, v.normal[2] as f32]);
        v.normal = [n[0].round() as i8, n[1].round() as i8, n[2].round() as i8, 0];
    }
}

/// `cyl(rTop, rBottom, h, sides, hex, x, y, z, axis)`: centred at y; `axis` 'x' / 'z' lays it down.
#[allow(clippy::too_many_arguments)]
fn cyl(b: &mut Builder, m: u32, r_top: f32, r_bot: f32, h: f32, sides: u32, col: [u8; 3], at: [f32; 3], axis: Option<char>) {
    let from = b.v.len();
    b.cylinder([0.0; 3], -h / 2.0, h / 2.0, r_bot, r_top, sides, col, m, true);
    // A bottom cap as well (the JS cylinders are closed).
    let ring = b.v.len() as u32;
    let c0 = b.vert([0.0, -h / 2.0, 0.0], [0.0, -1.0, 0.0], [0.5, 0.5], col, m);
    for i in 0..=sides {
        let a = i as f32 / sides as f32 * std::f32::consts::TAU;
        b.vert([a.cos() * r_bot, -h / 2.0, a.sin() * r_bot], [0.0, -1.0, 0.0], [0.5, 0.5], col, m);
    }
    for i in 0..sides {
        b.tri(false, c0, ring + 1 + i, ring + 2 + i);
    }
    for v in &mut b.v[from..] {
        let [x, y, z] = v.pos;
        let [nx, ny, nz, w] = v.normal;
        match axis {
            // rotateX(PI/2): y -> z, z -> -y.
            Some('z') => {
                v.pos = [x, -z, y];
                v.normal = [nx, nz.saturating_neg(), ny, w];
            }
            // rotateZ(PI/2): x -> y, y -> -x.
            Some('x') => {
                v.pos = [-y, x, z];
                v.normal = [ny.saturating_neg(), nx, nz, w];
            }
            _ => {}
        }
        v.pos = [v.pos[0] + at[0], v.pos[1] + at[1], v.pos[2] + at[2]];
    }
}

/// `lathe(profile, sides, hex, x, y, z)`: [radius, height] pairs bottom up, about +Y.
fn lathe(b: &mut Builder, m: u32, profile: &[[f32; 2]], sides: u32, col: [u8; 3], at: [f32; 3]) {
    let base = b.v.len() as u32;
    let rows = profile.len() as u32;
    for i in 0..=sides {
        let a = i as f32 / sides as f32 * std::f32::consts::TAU;
        let (s, c) = a.sin_cos();
        for k in 0..rows as usize {
            let [r, y] = profile[k];
            // Profile normal from the neighbouring points (outward).
            let (p0, p1) = (profile[k.saturating_sub(1)], profile[(k + 1).min(profile.len() - 1)]);
            let (dr, dy) = (p1[0] - p0[0], p1[1] - p0[1]);
            let (nr, ny) = (dy, -dr);
            b.vert([at[0] + c * r.max(0.0005), at[1] + y, at[2] + s * r.max(0.0005)], [c * nr, ny, s * nr], [0.0, 0.0], col, m);
        }
    }
    for i in 0..sides {
        for k in 0..rows - 1 {
            let a = base + i * rows + k;
            let bb = base + (i + 1) * rows + k;
            b.tri(false, a, bb, a + 1);
            b.tri(false, a + 1, bb, bb + 1);
        }
    }
}

/// `ring(radius, tube, sides, hex, x, y, z, axis)`: a torus, flat unless laid along 'z' (facing Z) or 'x'.
fn torus(b: &mut Builder, m: u32, radius: f32, tube: f32, sides: u32, section: u32, col: [u8; 3], at: [f32; 3], axis: Option<char>) {
    let base = b.v.len() as u32;
    for i in 0..=sides {
        let u = i as f32 / sides as f32 * std::f32::consts::TAU;
        for j in 0..=section {
            let w = j as f32 / section as f32 * std::f32::consts::TAU;
            // Three's torus lies in XY, facing Z.
            let (cu, su, cw, sw) = (u.cos(), u.sin(), w.cos(), w.sin());
            let p = [(radius + tube * cw) * cu, (radius + tube * cw) * su, tube * sw];
            let n = [cw * cu, cw * su, sw];
            let (p, n) = match axis {
                Some('z') => (p, n),
                Some('x') => ([p[2], p[1], -p[0]], [n[2], n[1], -n[0]]),
                _ => ([p[0], -p[2], p[1]], [n[0], -n[2], n[1]]),
            };
            b.vert([p[0] + at[0], p[1] + at[1], p[2] + at[2]], n, [0.0, 0.0], col, m);
        }
    }
    let row = section + 1;
    for i in 0..sides {
        for j in 0..section {
            let a = base + i * row + j;
            let c = base + (i + 1) * row + j;
            b.tri(false, a, c, a + 1);
            b.tri(false, a + 1, c, c + 1);
        }
    }
}

/// `tube(points, radius, sides, hex)`: a straight round member per pair of points.
fn tube(b: &mut Builder, m: u32, a: [f32; 3], c: [f32; 3], r: f32, sides: u32, col: [u8; 3]) {
    let d = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-6);
    let dir = glam::Vec3::new(d[0], d[1], d[2]) / len;
    let q = glam::Quat::from_rotation_arc(glam::Vec3::Y, dir);
    let from = b.v.len();
    b.cylinder([0.0; 3], 0.0, len, r, r, sides, col, m, true);
    for v in &mut b.v[from..] {
        let p = q * glam::Vec3::from(v.pos) + glam::Vec3::from(a);
        v.pos = p.to_array();
        let n = q * glam::Vec3::new(v.normal[0] as f32, v.normal[1] as f32, v.normal[2] as f32);
        v.normal = [n.x.round() as i8, n.y.round() as i8, n.z.round() as i8, 0];
    }
}

/// `planterShrub`: three faceted lumps (an icosahedron, detail 1 approximated by a 2-ring sphere).
fn shrub(b: &mut Builder, m: u32, radius: f32, hex: &str) {
    for (i, (x, y, z, r, shade)) in [(-0.16f32, 0.84f32, 0.04f32, radius * 0.95, 1.0f32), (0.17, 0.82, -0.05, radius * 0.85, 0.9), (0.0, 0.97, 0.06, radius * 0.8, 1.12)].into_iter().enumerate() {
        let col = tone(hex, 1.05 * shade);
        let base = b.v.len() as u32;
        let (rings, segs) = (4u32, 7u32);
        let rot = 0.7 + i as f32 * 1.3;
        for k in 0..=rings {
            let phi = k as f32 / rings as f32 * std::f32::consts::PI;
            for j in 0..=segs {
                let th = j as f32 / segs as f32 * std::f32::consts::TAU + rot;
                let n = [phi.sin() * th.cos(), phi.cos(), phi.sin() * th.sin()];
                b.vert([x + n[0] * r * 1.15, y + n[1] * r * 0.85, z + n[2] * r], n, [0.0, 0.0], col, m);
            }
        }
        for k in 0..rings {
            for j in 0..segs {
                let a = base + k * (segs + 1) + j;
                let c = a + segs + 1;
                b.tri(false, a, a + 1, c);
                b.tri(false, a + 1, c + 1, c);
            }
        }
    }
}

// ---------------------------------------------------------------- GLB reductions

/// Vertex clustering on a `cell`-metre grid (also split by a coarse UV cell so
/// seams keep their charts): a cheap, deterministic reduction of a GLB mesh.
fn cluster(g: &Glb, cell: f32) -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<u32>) {
    use std::collections::HashMap;
    let mut key_of: HashMap<(i32, i32, i32, i16, i16), u32> = HashMap::new();
    let mut acc: Vec<([f32; 3], [f32; 3], [f32; 2], f32)> = vec![];
    let mut remap = vec![0u32; g.positions.len()];
    for (k, p) in g.positions.iter().enumerate() {
        let uv = g.uvs[k];
        let key = ((p[0] / cell).floor() as i32, (p[1] / cell).floor() as i32, (p[2] / cell).floor() as i32, (uv[0] * 4.0).floor() as i16, (uv[1] * 4.0).floor() as i16);
        let id = *key_of.entry(key).or_insert_with(|| {
            acc.push(([0.0; 3], [0.0; 3], [0.0; 2], 0.0));
            acc.len() as u32 - 1
        });
        let a = &mut acc[id as usize];
        for c in 0..3 {
            a.0[c] += p[c];
            a.1[c] += g.normals[k][c];
        }
        a.2[0] += uv[0];
        a.2[1] += uv[1];
        a.3 += 1.0;
        remap[k] = id;
    }
    let pos: Vec<[f32; 3]> = acc.iter().map(|a| [a.0[0] / a.3, a.0[1] / a.3, a.0[2] / a.3]).collect();
    let nrm: Vec<[f32; 3]> = acc.iter().map(|a| a.1).collect();
    let uvs: Vec<[f32; 2]> = acc.iter().map(|a| [a.2[0] / a.3, a.2[1] / a.3]).collect();
    let mut seen = std::collections::HashSet::new();
    let mut idx = vec![];
    for t in g.indices.chunks_exact(3) {
        let (a, b, c) = (remap[t[0] as usize], remap[t[1] as usize], remap[t[2] as usize]);
        if a == b || b == c || a == c {
            continue;
        }
        let mut s = [a, b, c];
        s.sort_unstable();
        if seen.insert(s) {
            idx.extend_from_slice(&[a, b, c]);
        }
    }
    (pos, nrm, uvs, idx)
}

fn add_mesh(b: &mut Builder, m: u32, pos: &[[f32; 3]], nrm: &[[f32; 3]], uvs: &[[f32; 2]], idx: &[u32]) {
    let base = b.v.len() as u32;
    for k in 0..pos.len() {
        b.vert(pos[k], nrm[k], uvs[k], [255, 255, 255], m);
    }
    for t in idx.chunks_exact(3) {
        b.tri(false, base + t[0], base + t[1], base + t[2]);
    }
}

/// The JS load-time repaints (`whitenHydrantPaint`, `charcoalPayStationPaint`) on an RGB albedo.
fn repaint(kind: &str, rgb: &mut [u8]) {
    let n = rgb.len() / 3;
    if n == 0 {
        return;
    }
    let mean = rgb.chunks_exact(3).map(|c| (c[0] as f64 + c[1] as f64 + c[2] as f64) / 3.0).sum::<f64>() / n as f64;
    let mean = if mean > 0.0 { mean } else { 1.0 };
    for c in rgb.chunks_exact_mut(3) {
        let (r, g, b) = (c[0] as f64, c[1] as f64, c[2] as f64);
        let l = (r + g + b) / 3.0;
        match kind {
            "hydrant" => {
                let v = (0.86 + (l - mean) / 255.0 * 1.6).clamp(0.35, 1.0);
                c[0] = (236.0 * v).round() as u8;
                c[1] = (235.0 * v).round() as u8;
                c[2] = (228.0 * v).round() as u8;
            }
            "payStation" => {
                if b > r + 18.0 && b > g + 6.0 {
                    continue;
                }
                let t = if l < 70.0 {
                    14.0 + (l / 70.0) * 12.0
                } else if l > mean + 45.0 {
                    88.0 + ((l - mean - 45.0) * 0.6).min(40.0)
                } else {
                    52.0 + ((l - 70.0) / (mean + 45.0 - 70.0).max(1.0)) * 14.0
                };
                c[0] = (t * 0.94).round() as u8;
                c[1] = t.round() as u8;
                c[2] = (t * 1.07).round().min(255.0) as u8;
            }
            _ => return,
        }
    }
}

/// `standPayStationUpright`: the JS stretch about the base.
const PAY_STATION_STRETCH: [f32; 3] = [0.45, 1.15, 0.7];

fn load_glb(out: &mut Out, repo: &std::path::Path, js: &JsProps, name: &str) -> Option<(Glb, i32)> {
    let prop = js.street_prop(name).ok()?;
    match read_glb(repo, prop) {
        Ok(mut g) => {
            if let Some((_, _, rgb)) = g.maps[0].as_mut() {
                repaint(name, rgb);
            }
            if name == "payStation" {
                let base = g.positions.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
                for p in &mut g.positions {
                    *p = [p[0] * PAY_STATION_STRETCH[0], (p[1] - base) * PAY_STATION_STRETCH[1] + base, p[2] * PAY_STATION_STRETCH[2]];
                }
                for n in &mut g.normals {
                    let v = glam::Vec3::new(n[0] / PAY_STATION_STRETCH[0], n[1] / PAY_STATION_STRETCH[1], n[2] / PAY_STATION_STRETCH[2]).normalize_or_zero();
                    *n = v.to_array();
                }
            }
            let flat_n = flat_layer([128, 128, 255, 255], AUX_RES);
            let flat_orm = flat_layer([255, 255, 0, 255], AUX_RES);
            let white = flat_layer([255; 4], ALBEDO_RES);
            let map = |k: usize, res: u32, d: &Vec<u8>| match &g.maps[k] {
                Some((w, h, rgb)) => resample(*w, *h, rgb, 3, res),
                None => d.clone(),
            };
            let layer = [map(0, ALBEDO_RES, &white), map(1, AUX_RES, &flat_n), map(2, AUX_RES, &flat_orm)];
            out.layers.push(layer);
            out.sources.push(format!("{name}: {} (sha256 {})", prop.path, &prop.sha256[..12]));
            Some((g, out.layers.len() as i32 - 1))
        }
        Err(e) => {
            out.missing.push(format!("{name}: {} ({e:#})", prop.path));
            None
        }
    }
}

/// A GLB kind: level 0 the GLB, levels 1 and 2 clustered at 1/24 and 1/9 of its diagonal.
fn glb_model(out: &mut Out, repo: &std::path::Path, js: &JsProps, name: &str, id: u32, lod: [f32; 4], cast: u32) -> bool {
    let Some((g, layer)) = load_glb(out, repo, js, name) else { return false };
    let m = glb_mat(out.materials, layer);
    let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
    for p in &g.positions {
        for a in 0..3 {
            mn[a] = mn[a].min(p[a]);
            mx[a] = mx[a].max(p[a]);
        }
    }
    let diag = ((mx[0] - mn[0]).powi(2) + (mx[1] - mn[1]).powi(2) + (mx[2] - mn[2]).powi(2)).sqrt();
    add_mesh(&mut out.b, m, &g.positions, &g.normals, &g.uvs, &g.indices);
    let m0 = out.finish();
    let (p1, n1, u1, i1) = cluster(&g, diag / 24.0);
    add_mesh(&mut out.b, m, &p1, &n1, &u1, &i1);
    let m1 = out.finish();
    let (p2, n2, u2, i2) = cluster(&g, diag / 9.0);
    add_mesh(&mut out.b, m, &p2, &n2, &u2, &i2);
    let m2 = out.finish();
    out.set(id, [m0, m1, m2], lod, cast, 0.0);
    true
}

/// `streamedFurnitureCascades`: under 1.2 m the near cascade only, under 4 m the near two, else all.
fn cascades_for(height: f32) -> u32 {
    if height < 1.2 {
        1
    } else if height < 4.0 {
        2
    } else {
        3
    }
}

// ---------------------------------------------------------------- the cook

/// Add every props2 model to the library. `big`: the big texture array's
/// layers (0 the vegetation atlas, 1 the sign faces, 2 the printed faces).
pub(crate) fn cook_extras(out: &mut Out, repo: &std::path::Path, js: &JsProps, x: &LibExtras, m_bark: u32, m_leaf: u32, alpha_test: f32) -> ExtraInfo {
    let mut info = ExtraInfo::default();
    let _ = alpha_test;
    // Plain response materials (base colour is the vertex colour).
    let white = [255u8; 3];
    let m_paint = push_mat(out.materials, white, 0.62, 0.1, 0, 0);
    let m_metal = push_mat(out.materials, white, 0.45, 0.6, 0, 0);
    let m_matte = push_mat(out.materials, white, 0.85, 0.0, 0, 0);

    // --- furniture kinds with a GLB (STREET_FURNITURE_KINDS heights decide the cascades).
    // (kind, model, LOD edges [0->1, 1->2, unused, cut-off], kind height m)
    let near = 120.0; // the JS near ring (`STREET_FURNITURE_RINGS[0].radius`)
    let glbs: [(&str, Model, [f32; 4], f32); 18] = [
        ("hydrant", Model::Hydrant, [16.0, 45.0, 0.0, near], 0.8),
        ("parkingMeter", Model::ParkingMeter, [16.0, 45.0, 0.0, near], 1.3),
        ("payStation", Model::PayStation, [16.0, 45.0, 0.0, near], 1.5),
        ("signPole", Model::SignPole, [22.0, 60.0, 0.0, near], 2.6),
        ("bollard", Model::Bollard, [14.0, 40.0, 0.0, near], 0.9),
        ("wasteBin", Model::WasteBin, [16.0, 45.0, 0.0, near], 1.0),
        ("bikeRack", Model::BikeRack, [16.0, 45.0, 0.0, near], 0.9),
        ("newsBox", Model::NewsBox, [16.0, 45.0, 0.0, near], 1.2),
        ("mailbox", Model::Mailbox, [16.0, 45.0, 0.0, near], 1.2),
        ("lampHistoric", Model::LampHistoric, [30.0, 75.0, 0.0, near], 7.62),
        ("lampBanner", Model::LampBanner, [30.0, 75.0, 0.0, near], 7.62),
        ("lampCobra", Model::LampCobra, [30.0, 75.0, 0.0, near], 7.62),
        ("bench", Model::Bench, [18.0, 50.0, 0.0, near], 0.9),
        ("transitShelter", Model::TransitShelter, [30.0, 90.0, 0.0, 260.0], 2.6),
        ("busStopFlag", Model::BusStopFlag, [22.0, 70.0, 0.0, 260.0], 2.9),
        ("cafeSet", Model::CafeSet, [16.0, 45.0, 0.0, near], 0.9),
        ("sandwichBoard", Model::SandwichBoard, [14.0, 40.0, 0.0, near], 1.0),
        ("utilityBox", Model::UtilityBox, [16.0, 45.0, 0.0, near], 1.12),
    ];
    for (name, model, lod, h) in glbs {
        if !glb_model(out, repo, js, name, model as u32, lod, cascades_for(h)) {
            // A proxy at the kind's height (the JS keeps its procedural mesh).
            bx(&mut out.b, m_paint, 0.3, h, 0.3, "#6d6f6d", 0.0, h / 2.0, 0.0, 1.0);
            let k = out.finish();
            out.set(model as u32, [k, k, k], [near, near, 0.0, near], cascades_for(h), 0.0);
        }
    }

    // --- procedural kinds (buildCatalogueGeometry), near and coarse.
    let b = &mut out.b;
    // planter
    {
        bx(b, m_matte, 0.94, 0.14, 0.58, pal("planterConcrete"), 0.0, 0.07, 0.0, 1.0);
        bx(b, m_matte, 1.01, 0.44, 0.63, pal("planterConcrete"), 0.0, 0.36, 0.0, 1.0);
        bx(b, m_matte, 1.06, 0.10, 0.68, pal("planterConcrete"), 0.0, 0.605, 0.0, 1.0);
        bx(b, m_matte, 0.9, 0.06, 0.52, pal("planterSoil"), 0.0, 0.63, 0.0, 1.0);
        shrub(b, m_matte, 0.3, pal("canopyB"));
        let n = out.finish();
        let b = &mut out.b;
        bx(b, m_matte, 1.06, 0.62, 0.68, pal("planterConcrete"), 0.0, 0.31, 0.0, 1.0);
        bx(b, m_matte, 0.9, 0.06, 0.52, pal("planterSoil"), 0.0, 0.63, 0.0, 1.0);
        let c = out.finish();
        out.set(Model::Planter as u32, [n, c, c], [25.0, near, 0.0, near], 1, 0.0);
    }
    // standpipe
    {
        let b = &mut out.b;
        let body = tone(pal("standpipeBody"), 1.0);
        let brass = tone(pal("standpipeBrass"), 1.0);
        let prof: [[f32; 2]; 14] = [[0.1, 0.0], [0.1, 0.026], [0.078, 0.04], [0.058, 0.058], [0.055, 0.35], [0.055, 0.62], [0.066, 0.64], [0.066, 0.72], [0.055, 0.74], [0.055, 0.8], [0.066, 0.82], [0.066, 0.9], [0.055, 0.92], [0.052, 0.95]];
        lathe(b, m_paint, &prof, 10, body, [0.0; 3]);
        cyl(b, m_metal, 0.05, 0.056, 0.16, 8, brass, [0.0, 0.86, -0.09], Some('z'));
        cyl(b, m_metal, 0.05, 0.056, 0.16, 8, brass, [0.0, 0.68, -0.09], Some('z'));
        torus(b, m_metal, 0.05, 0.01, 8, 4, brass, [0.0, 0.86, -0.155], Some('z'));
        torus(b, m_metal, 0.05, 0.01, 8, 4, brass, [0.0, 0.68, -0.155], Some('z'));
        let n = out.finish();
        let b = &mut out.b;
        cyl(b, m_paint, 0.055, 0.055, 0.95, 4, body, [0.0, 0.48, 0.0], None);
        cyl(b, m_metal, 0.05, 0.05, 0.16, 4, brass, [0.0, 0.86, -0.09], Some('z'));
        bx(b, m_paint, 0.2, 0.05, 0.2, pal("standpipeBody"), 0.0, 0.025, 0.0, 1.0);
        let c = out.finish();
        out.set(Model::Standpipe as u32, [n, c, c], [20.0, near, 0.0, near], 1, 0.0);
    }
    // wall meter
    {
        let b = &mut out.b;
        bx(b, m_paint, 0.3, 0.42, 0.16, pal("meterBoxGrey"), 0.0, 1.32, 0.0, 1.0);
        cyl(b, m_metal, 0.03, 0.03, 0.42, 6, tone(pal("meterPole"), 1.0), [0.0, 0.9, 0.0], None);
        cyl(b, m_paint, 0.09, 0.09, 0.04, 8, tone(pal("signWhite"), 1.0), [0.0, 1.38, -0.09], Some('z'));
        let n = out.finish();
        let b = &mut out.b;
        bx(b, m_paint, 0.3, 0.42, 0.16, pal("meterBoxGrey"), 0.0, 1.32, 0.0, 1.0);
        cyl(b, m_metal, 0.03, 0.03, 0.42, 4, tone(pal("meterPole"), 1.0), [0.0, 0.9, 0.0], None);
        let c = out.finish();
        out.set(Model::WallMeter as u32, [n, c, c], [20.0, near, 0.0, near], 2, 0.0);
    }
    // pedestrian signal post
    {
        let b = &mut out.b;
        let pole = tone(pal("poleGrey"), 1.0);
        let prof: [[f32; 2]; 10] = [[0.108, 0.0], [0.108, 0.03], [0.09, 0.052], [0.062, 0.078], [0.057, 0.12], [0.055, 0.35], [0.05, 2.88], [0.054, 2.935], [0.046, 2.98], [0.016, 3.0]];
        lathe(b, m_metal, &prof, 12, pole, [0.0; 3]);
        bx(b, m_paint, 0.34, 0.4, 0.2, pal("signalHousing"), 0.0, 2.52, -0.06, 1.0);
        bx(b, m_paint, 0.28, 0.32, 0.014, pal("signFace"), 0.0, 2.52, -0.168, 0.25);
        bx(b, m_paint, 0.36, 0.03, 0.15, pal("signalVisor"), 0.0, 2.73, -0.13, 1.0);
        bx(b, m_paint, 0.03, 0.42, 0.15, pal("signalVisor"), -0.185, 2.52, -0.13, 1.0);
        bx(b, m_paint, 0.03, 0.42, 0.15, pal("signalVisor"), 0.185, 2.52, -0.13, 1.0);
        bx(b, m_paint, 0.15, 0.22, 0.11, pal("signalHousing"), 0.0, 1.06, -0.09, 1.0);
        cyl(b, m_paint, 0.035, 0.035, 0.03, 8, tone(pal("buttonYellow"), 1.0), [0.0, 1.12, -0.15], Some('z'));
        bx(b, m_paint, 0.13, 0.16, 0.01, pal("signFace"), 0.0, 0.86, -0.15, 1.0);
        let n = out.finish();
        let b = &mut out.b;
        cyl(b, m_metal, 0.048, 0.058, 3.0, 4, pole, [0.0, 1.5, 0.0], None);
        bx(b, m_paint, 0.34, 0.4, 0.2, pal("signalHousing"), 0.0, 2.52, -0.06, 1.0);
        bx(b, m_paint, 0.15, 0.22, 0.11, pal("signalHousing"), 0.0, 1.06, -0.09, 1.0);
        let c = out.finish();
        out.set(Model::PedSignalPost as u32, [n, c, c], [25.0, near, 0.0, near], 2, 0.0);
    }
    // signal cabinet (near: plinth, shell, cap, crown, door, louvres; coarse: the JS's four boxes)
    let cabinet = {
        let b = &mut out.b;
        let t = pal("cabinetTrim");
        bx(b, m_matte, 0.82, 0.11, 0.58, pal("cabinetPlinth"), 0.0, 0.055, 0.0, 1.0);
        bx(b, m_paint, 0.74, 1.26, 0.5, pal("cabinetShell"), 0.0, 0.74, 0.0, 1.0);
        bx(b, m_paint, 0.8, 0.04, 0.56, t, 0.0, 1.39, 0.0, 1.0);
        bx(b, m_paint, 0.668, 0.016, 0.428, pal("cabinetShell"), 0.0, 1.418, 0.0, 0.92);
        bx(b, m_paint, 0.62, 1.1, 0.02, t, 0.0, 0.75, -0.252, 1.0);
        bx(b, m_paint, 0.56, 1.04, 0.015, pal("cabinetDoor"), 0.0, 0.75, -0.262, 1.0);
        bx(b, m_metal, 0.03, 0.16, 0.03, t, 0.235, 0.78, -0.284, 1.0);
        bx(b, m_paint, 0.18, 0.22, 0.006, t, 0.13, 1.0, -0.272, 1.0);
        bx(b, m_paint, 0.14, 0.1, 0.005, pal("signFace"), -0.14, 1.04, -0.272, 1.0);
        for k in 0..5 {
            bx(b, m_paint, 0.4, 0.012, 0.02, t, 0.0, 1.15 + k as f32 * 0.03, -0.274, 1.0);
        }
        for k in 0..4 {
            bx(b, m_paint, 0.4, 0.012, 0.02, t, 0.0, 0.3 + k as f32 * 0.03, -0.274, 1.0);
        }
        for side in [-1.0f32, 1.0] {
            bx(b, m_paint, 0.006, 0.2, 0.34, t, side * 0.372, 0.6, 0.02, 1.0);
            bx(b, m_paint, 0.006, 0.16, 0.24, t, side * 0.372, 1.12, 0.08, 1.0);
        }
        bx(b, m_paint, 0.62, 1.1, 0.006, t, 0.0, 0.75, 0.252, 1.0);
        let n = out.finish();
        let b = &mut out.b;
        bx(b, m_matte, 0.82, 0.11, 0.58, pal("cabinetPlinth"), 0.0, 0.055, 0.0, 1.0);
        bx(b, m_paint, 0.74, 1.28, 0.5, pal("cabinetShell"), 0.0, 0.75, 0.0, 1.0);
        bx(b, m_paint, 0.8, 0.06, 0.56, pal("cabinetTrim"), 0.0, 1.42, 0.0, 1.0);
        let c = out.finish();
        out.set(Model::SignalCabinet as u32, [n, c, c], [30.0, near, 0.0, near], 2, 0.0);
        [n, c]
    };
    // scaffold (nearOnly)
    {
        let b = &mut out.b;
        let tcol = tone(pal("scaffoldTube"), 1.0);
        let r = 0.024;
        let xs = [-2.4f32, 0.0, 2.4];
        let zs = [-0.55f32, 0.55];
        for &x in &xs {
            for &z in &zs {
                cyl(b, m_metal, r, r, 6.2, 6, tcol, [x, 3.1, z], None);
                bx(b, m_metal, 0.15, 0.012, 0.15, pal("plateSteel"), x, 0.006, z, 1.0);
                bx(b, m_matte, 0.22, 0.04, 0.22, pal("scaffoldPlank"), x, 0.032, z, 1.0);
            }
        }
        for y in [0.2f32, 2.25, 4.3, 6.15] {
            for &z in &zs {
                cyl(b, m_metal, r, r, 4.9, 6, tcol, [0.0, y, z], Some('x'));
            }
            for &x in &xs {
                cyl(b, m_metal, r, r, 1.2, 6, tcol, [x, y + 0.05, 0.0], Some('z'));
            }
        }
        for y in [3.3f32, 5.35] {
            cyl(b, m_metal, r, r, 4.9, 6, tcol, [0.0, y, -0.55], Some('x'));
        }
        tube(b, m_metal, [-2.4, 2.25, 0.55], [0.0, 6.15, 0.55], r, 6, tcol);
        tube(b, m_metal, [2.4, 2.25, 0.55], [0.0, 6.15, 0.55], r, 6, tcol);
        for y in [2.3f32, 4.35] {
            for i in 0..4 {
                bx(b, m_matte, 4.85, 0.038, 0.225, pal("scaffoldPlank"), 0.0, y + 0.04, -0.36 + i as f32 * 0.24, 0.9 + (i % 2) as f32 * 0.12);
            }
            bx(b, m_matte, 4.85, 0.15, 0.025, pal("scaffoldPlank"), 0.0, y + 0.13, -0.585, 0.8);
        }
        for x in [-2.4f32, 2.4] {
            bx(b, m_paint, 0.06, 0.06, 0.06, pal("scaffoldTie"), x, 2.0, -0.58, 1.0);
        }
        bx(b, m_paint, 0.5, 0.5, 0.012, "#e8b41c", -2.4, 1.45, -0.6, 1.0);
        let n = out.finish();
        out.set(Model::Scaffold as u32, [n, n, n], [near, near, 0.0, near], 3, 0.0);
    }
    // barricade (nearOnly)
    {
        let b = &mut out.b;
        for y in [0.92f32, 0.58] {
            for i in 0..6 {
                let hex = if i % 2 == 1 { pal("barricadeWhite") } else { pal("barricadeOrange") };
                bx(b, m_paint, 1.1 / 6.0, 0.15, 0.022, hex, -0.55 + (i as f32 + 0.5) * (1.1 / 6.0), y, -0.02, 1.0);
            }
        }
        let leg = tone(pal("barricadeLeg"), 1.0);
        for (x, dz) in [(-0.5f32, 1.0f32), (-0.5, -1.0), (0.5, 1.0), (0.5, -1.0)] {
            bx_rot(b, m_paint, 0.04, 1.02, 0.035, leg, 0.0, dz * 0.2, [x, 0.5, dz * 0.1]);
        }
        bx(b, m_paint, 0.06, 0.035, 0.42, pal("barricadeLeg"), -0.5, 0.018, 0.0, 1.0);
        bx(b, m_paint, 0.06, 0.035, 0.42, pal("barricadeLeg"), 0.5, 0.018, 0.0, 1.0);
        bx(b, m_paint, 0.36, 0.03, 0.36, pal("barricadeLeg"), 0.68, 0.015, 0.0, 1.0);
        lathe(b, m_paint, &[[0.15, 0.03], [0.13, 0.12], [0.085, 0.36], [0.06, 0.5], [0.03, 0.7], [0.0, 0.71]], 10, tone(pal("coneOrange"), 1.0), [0.68, 0.0, 0.0]);
        lathe(b, m_paint, &[[0.106, 0.265], [0.096, 0.335]], 10, tone(pal("barricadeWhite"), 1.0), [0.68, 0.0, 0.0]);
        lathe(b, m_paint, &[[0.069, 0.455], [0.061, 0.505]], 10, tone(pal("barricadeWhite"), 1.0), [0.68, 0.0, 0.0]);
        let n = out.finish();
        out.set(Model::Barricade as u32, [n, n, n], [near, near, 0.0, near], 1, 0.0);
    }
    // poster hoarding (nearOnly; the JS bills sample the poster sheet, here plain paper tones)
    {
        let b = &mut out.b;
        for (k, x) in [-1.83f32, -0.61, 0.61, 1.83].into_iter().enumerate() {
            bx(b, m_matte, 1.215, 2.38, 0.018, if k % 2 == 1 { pal("hoardingPly") } else { pal("hoardingPlyB") }, x, 1.25, 0.0, 1.0);
        }
        for x in [-2.44f32, -1.22, 0.0, 1.22, 2.44] {
            bx(b, m_matte, 0.09, 2.5, 0.09, pal("hoardingPost"), x, 1.25, 0.055, 1.0);
        }
        bx(b, m_matte, 4.98, 0.045, 0.09, pal("hoardingPost"), 0.0, 2.47, 0.03, 1.0);
        bx(b, m_matte, 4.98, 0.09, 0.045, pal("hoardingPost"), 0.0, 0.62, 0.035, 1.0);
        let post = tone(pal("hoardingPost"), 1.0);
        for x in [-1.83f32, -0.61, 0.61, 1.83] {
            tube(b, m_matte, [x, 1.7, 0.06], [x, 0.02, 0.28], 0.024, 4, post);
            bx(b, m_matte, 0.16, 0.04, 0.12, pal("hoardingPost"), x, 0.02, 0.28, 1.0);
        }
        // Bills: the JS layout (rows of sheets), each a flat tone of a printed sheet.
        let bills: [(f32, f32, f32, f32, &str); 10] = [
            (0.46, 0.66, -1.89, 1.86, "#d9d3c2"),
            (0.46, 0.66, -1.415, 1.86, "#c9a43c"),
            (0.46, 0.66, -0.94, 1.86, "#d9d3c2"),
            (0.61, 0.9, -0.1, 1.72, "#3f5e8c"),
            (0.46, 0.66, 0.73, 1.84, "#b8483a"),
            (0.46, 0.66, 1.205, 1.84, "#e6e1d4"),
            (0.46, 0.66, -1.77, 1.1, "#8a9a5b"),
            (0.46, 0.66, -1.295, 1.1, "#d9d3c2"),
            (0.61, 0.9, 1.2, 1.02, "#2d2d2b"),
            (0.24, 0.34, 2.06, 1.96, "#c96f2c"),
        ];
        for (k, (w, h, x, y, hex)) in bills.into_iter().enumerate() {
            bx(b, m_matte, w, h, 0.002, hex, x, y, -0.011 - (k % 3) as f32 * 0.002, 0.9);
        }
        let n = out.finish();
        out.set(Model::PosterHoarding as u32, [n, n, n], [near, near, 0.0, near], 2, 0.0);
    }

    // --- street trees (ez-tree), one model per species.
    if let Some(ez) = x.ez {
        let mode = eo_props::eztree::StreetTreeMode::default();
        let species = eo_props::eztree::street_tree_species().map(|s| s.to_vec()).unwrap_or_default();
        let mut cache: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        for (si, sp) in species.iter().enumerate().take(MAX_TREE_SPECIES as usize) {
            let sil = eo_props::eztree::street_tree_silhouette_for(si, mode).unwrap_or(0);
            let forms = [ez.street_tree(si, false, false, mode), ez.street_tree(sil, true, false, mode), ez.street_tree(sil, true, true, mode)];
            let mut ids = [NONE; 3];
            let mut ok = true;
            for (k, f) in forms.into_iter().enumerate() {
                let Ok(f) = f else {
                    ok = false;
                    break;
                };
                let key = format!("{}:{k}", f.species);
                if let Some(&id) = cache.get(&key) {
                    ids[k] = id;
                    continue;
                }
                tree_mesh(&mut out.b, &f.mesh, m_bark, m_leaf);
                let id = out.finish();
                cache.insert(key, id);
                ids[k] = id;
            }
            if !ok {
                out.missing.push(format!("ez-tree street species {sp}: a form is not cooked"));
                continue;
            }
            // JS streamed rings: near 120 m, window 260 m, far silhouettes beyond (tiles drop at 360 m).
            out.set(TREE_BASE + si as u32, ids, [70.0, 200.0, 0.0, 360.0], 3, 0.15);
            let s = eo_props::eztree::ez_species(sp).ok();
            info.tree_dims.push([s.map(|s| s.height_m as f32).unwrap_or(9.0), s.map(|s| s.max_half_crown_m as f32).unwrap_or(3.0)]);
        }
        info.tree_species = species;
        out.sources.push(format!("ez-tree street trees: {} species (eztree cook {})", info.tree_species.len(), if ez.from_cache { "from cache" } else { "baked" }));
    }

    // --- the overhead.
    if let Some(oh) = x.overhead {
        let h = |k: &str| oh.consts["streetOverhead"][k].as_f64().unwrap_or(0.1) as f32;
        let col = |k: &str, d: [f32; 3]| -> [u8; 3] {
            let a = &oh.consts["signColours"][k];
            if let Some(v) = a.as_array() {
                lin3([v[0].as_f64().unwrap_or(d[0] as f64) as f32, v[1].as_f64().unwrap_or(d[1] as f64) as f32, v[2].as_f64().unwrap_or(d[2] as f64) as f32])
            } else {
                lin3(d)
            }
        };
        let post = col("post", [0.5, 0.52, 0.53]);
        let plate = col("plate", [0.8, 0.8, 0.78]);
        let plate_back = col("plateBack", [0.33, 0.34, 0.335]);
        let rule = col("rule", [0.15, 0.16, 0.17]);
        let band_col = |name: &str| match name {
            "red" => col("red", [0.5, 0.075, 0.095]),
            "green" => col("green", [0.05, 0.33, 0.185]),
            "blue" => col("blue", [0.04, 0.16, 0.42]),
            "bannerPale" => col("bannerPale", [0.36, 0.07, 0.19]),
            "bannerInk" => col("bannerInk", [0.58, 0.46, 0.36]),
            _ => plate,
        };
        // Poles: `createOverheadPoleGeometry` (unit height, scale.y = height).
        for (id, top, base, sides, hex) in [(OH_POLE_STEEL, h("poleRadiusTopM"), h("poleRadiusBaseM"), 8u32, "#5c6167"), (OH_POLE_WOOD, h("woodPoleRadiusTopM"), h("woodPoleRadiusBaseM"), 10, "#5a4a3a")] {
            let c = srgb_hex(hex);
            out.b.cylinder([0.0; 3], 0.0, 1.0, base, top, sides, c, if id == OH_POLE_WOOD { m_matte } else { m_metal }, true);
            let n = out.finish();
            out.b.cylinder([0.0; 3], 0.0, 1.0, base, top, 4, c, if id == OH_POLE_WOOD { m_matte } else { m_metal }, false);
            let f = out.finish();
            out.set(id, [n, f, f], [50.0, 360.0, 0.0, 360.0], 3, 0.0);
        }
        // The distribution pole and its luminaire arm, and the lantern: the JS GLBs.
        if !glb_model(out, repo, js, "utilityPole", OH_DIST_POLE, [40.0, 120.0, 0.0, 360.0], 3) {
            out.b.cylinder([0.0; 3], 0.0, 11.7, 0.2, 0.14, 6, srgb_hex("#5a4a3a"), m_matte, true);
            let n = out.finish();
            out.set(OH_DIST_POLE, [n, n, n], [360.0, 360.0, 0.0, 360.0], 3, 0.0);
        }
        glb_model(out, repo, js, "cobraArm", OH_COBRA_ARM, [30.0, 90.0, 0.0, 200.0], 2);
        glb_model(out, repo, js, "lantern", OH_LANTERN, [20.0, 50.0, 0.0, 120.0], 1);
        // The overhead's controller cabinet: the furniture's cabinet mesh, scaled by its size.
        out.models[OH_CABINET as usize] = out.models[Model::SignalCabinet as usize];
        let _ = cabinet;
        // The corner cluster (`createCornerSignGeometry`): round plate, ring, slash, lower plate, strap, bracket.
        {
            let b = &mut out.b;
            cyl(b, m_paint, 0.18, 0.18, 0.014, 16, plate, [0.0; 3], Some('z'));
            torus(b, m_paint, 0.138, 0.023, 16, 4, band_col("red"), [0.0, 0.0, 0.01], Some('z'));
            bx_rot(b, m_paint, 0.3, 0.036, 0.004, band_col("red"), std::f32::consts::FRAC_PI_4, 0.0, [0.0, 0.0, 0.014]);
            sign_plate(b, m_paint, &PlateSpec { w: 0.26, h: 0.36, y: -0.52, x: 0.0, band: Some("green".into()), fill: None, rules: 2 }, -0.006, plate, plate_back, rule, &band_col);
            bx_rot(b, m_paint, 0.032, 0.58, 0.012, rule, 0.0, 0.0, [0.0, -0.27, -0.014]);
            bx_rot(b, m_paint, 0.05, 0.05, 0.05, rule, 0.0, 0.0, [0.0, 0.0, -0.03]);
            let n = out.finish();
            out.set(OH_CORNER_SIGN, [n, n, n], [80.0, 80.0, 0.0, 150.0], 1, 0.0);
        }
        // Sign variants (`createSignAssemblyGeometry`, `createPrintedStackGeometry`).
        let section = h("signPostSectionM");
        let printed_mat = {
            let mut m = PropMaterial { base: [1.0; 4], emissive: [0.0; 4], params: [0.55, 0.0, 1.0, 0.0], layer: 2, flags: P_TEX | P_BIG, rough_flatten: 0.0, thin_absorption: 0.0 };
            if x.signs.is_none() {
                m.flags = 0;
            }
            out.materials.push(m);
            out.materials.len() as u32 - 1
        };
        for (vi, spec) in oh.sign_variants.iter().enumerate().take(MAX_SIGN_VARIANTS as usize) {
            for lvl in 0..2 {
                let b = &mut out.b;
                if spec.post_height_m > 0.0 {
                    if spec.printed {
                        cyl(b, m_metal, 0.03, 0.03, spec.post_height_m as f32, if lvl == 0 { 10 } else { 4 }, post, [0.0, spec.post_height_m as f32 / 2.0, 0.0], None);
                    } else {
                        bx_rot(b, m_metal, section, spec.post_height_m as f32, section, post, 0.0, 0.0, [0.0, spec.post_height_m as f32 / 2.0, 0.0]);
                    }
                } else if let (Some(a), Some(z)) = (spec.plates.first(), spec.plates.last()) {
                    for p in [a, z] {
                        bx_rot(b, m_metal, 0.06, 0.03, section, post, 0.0, 0.0, [0.0, p.y as f32, 0.0]);
                    }
                }
                if spec.fence.is_some() {
                    // Frontage fences (the block overlays, not ported): a plain panel.
                    bx_rot(b, m_paint, 1.0, 1.9, 0.03, srgb_hex("#202224"), 0.0, 0.0, [0.0, 0.95, 0.0]);
                }
                for p in &spec.plates {
                    let ps = PlateSpec { w: p.w as f32, h: p.h as f32, y: p.y as f32, x: p.x.unwrap_or(0.0) as f32, band: if lvl == 0 { p.band.clone() } else { None }, fill: p.fill.clone(), rules: if lvl == 0 { p.rules.unwrap_or(0) } else { 0 } };
                    if spec.printed {
                        // Body (plate back colour), then the printed front on the atlas.
                        let t = 0.012;
                        bx_rot(b, m_paint, ps.w, ps.h, t, plate_back, 0.0, 0.0, [ps.x, ps.y, section / 2.0 + t / 2.0]);
                        let tile = p.face.as_deref().map(eo_props::sign_atlas::printed_face_tile).unwrap_or([0.0, 0.0, 1.0, 0.125]);
                        // Printed atlas (256 x 2048) sits at x 0..256 of the big layer; texture v is 1 - v_up.
                        let s = eo_props::sign_atlas::PRINTED_ATLAS_WIDTH as f32 / BIG_RES as f32;
                        let uv = [tile[0] * s, 1.0 - tile[1], (tile[0] + tile[2]) * s, 1.0 - (tile[1] + tile[3])];
                        b.quad_z([ps.x, ps.y, section / 2.0 + t + 0.001], ps.w, ps.h, uv, [255, 255, 255], printed_mat, false);
                    } else {
                        sign_plate(b, m_paint, &ps, section / 2.0, plate, plate_back, rule, &band_col);
                    }
                }
                let _ = out.finish();
            }
            let n = out.meshes.len() as u32;
            out.set(OH_SIGN_BASE + vi as u32, [n - 2, n - 1, n - 1], [35.0, 150.0, 0.0, 150.0], 1, 0.0);
        }
        out.sources.push(format!("street overhead: {} sign variants, poles, cabinets, corner signs (procedural, street-overhead.js)", oh.sign_variants.len().min(MAX_SIGN_VARIANTS as usize)));
    }
    info
}

struct PlateSpec {
    w: f32,
    h: f32,
    y: f32,
    x: f32,
    band: Option<String>,
    fill: Option<String>,
    rules: u32,
}

/// `signPlateParts`: a white field (or `fill`), a coloured band across the head, dark rules.
#[allow(clippy::too_many_arguments)]
fn sign_plate(b: &mut Builder, m: u32, p: &PlateSpec, face_z: f32, plate: [u8; 3], back: [u8; 3], rule: [u8; 3], band_col: &dyn Fn(&str) -> [u8; 3]) {
    let t = 0.012;
    let field = p.fill.as_deref().map(band_col).unwrap_or(plate);
    // The front face takes the field colour, the rest the bare back.
    let from = b.v.len();
    b.cuboid([p.x - p.w / 2.0, p.y - p.h / 2.0, face_z], [p.x + p.w / 2.0, p.y + p.h / 2.0, face_z + t], back, m);
    for v in &mut b.v[from..] {
        if v.normal[2] > 100 {
            v.color = [field[0], field[1], field[2], 255];
        }
    }
    let front = face_z + t + 0.002;
    if let Some(band) = &p.band {
        let bh = p.h * 0.26;
        b.cuboid([-(p.w - 0.024) / 2.0, p.y + p.h / 2.0 - bh - 0.016, front - 0.002], [(p.w - 0.024) / 2.0, p.y + p.h / 2.0 - 0.016, front + 0.002], band_col(band), m);
    }
    let top = p.y + p.h / 2.0 - if p.band.is_some() { p.h * 0.34 } else { 0.05 };
    for k in 0..p.rules {
        let w = p.w - 0.08 - k as f32 * 0.05;
        let y = top - 0.055 - k as f32 * 0.075;
        b.cuboid([-w / 2.0, y - 0.0085, front - 0.0015], [w / 2.0, y + 0.0085, front + 0.0015], rule, m);
    }
}

/// An ez-tree street form into the builder: bark opaque, leaves alpha-tested,
/// vertex colours (linear tint x tone) to sRGB, UV v flipped (rows top first),
/// and a wind weight per vertex (height fraction; leaves more than wood).
fn tree_mesh(b: &mut Builder, m: &eo_props::eztree::EzMesh, m_bark: u32, m_leaf: u32) {
    let base = b.v.len() as u32;
    let height = m.positions.iter().map(|p| p[1]).fold(1.0f32, f32::max);
    for k in 0..m.positions.len() {
        let leaf = m.part.get(k).copied().unwrap_or(0) != 0;
        let c = m.colors.get(k).copied().unwrap_or([1.0; 3]);
        let uv = m.uvs.get(k).copied().unwrap_or([0.0; 2]);
        let id = b.vert(m.positions[k], m.normals[k], [uv[0], 1.0 - uv[1]], [to_srgb8(c[0]), to_srgb8(c[1]), to_srgb8(c[2])], if leaf { m_leaf } else { m_bark });
        let f = (m.positions[k][1] / height).clamp(0.0, 1.0);
        let w = if leaf { 0.15 + 0.85 * f.powf(1.5) } else { 0.6 * f * f };
        b.v[id as usize].pad = w.to_bits();
    }
    for t in m.indices.chunks_exact(3) {
        let leaf = t.iter().any(|&v| m.part.get(v as usize).copied().unwrap_or(0) != 0);
        b.tri(leaf, base + t[0], base + t[1], base + t[2]);
    }
}
