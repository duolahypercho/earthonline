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
//! * Stand-ins (the section at the end): a GLB kind whose file is absent from
//!   the checkout (furniture, the signal head's housing, the palm, the overhead's
//!   distribution pole, cobra arm and lantern) gets a procedural model of the San
//!   Francisco design at the GLB's dimensions, three levels, base dirt in its
//!   vertex colours. Nothing of it runs when the GLB is there. Only with
//!   `LibExtras::stand_ins` (eo-app `--stand-ins`); without it the grey
//!   proxies as before.

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
/// The moored boats (`eo_props::boats::BODIES` order), after the sign variants.
pub const BOAT_BASE: u32 = OH_SIGN_BASE + MAX_SIGN_VARIANTS;
pub const RENDER_MODEL_COUNT: usize = (BOAT_BASE + eo_props::boats::BODIES.len() as u32) as usize;
/// The boats' level edges (metres): full body to 70 m, the reduced one to 260 m, the
/// silhouette to 3 km (a 10 m hull is a few pixels there).
pub const BOAT_LOD: [f32; 4] = [70.0, 260.0, 0.0, 3000.0];
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
    /// A GLB kind absent from the checkout takes its procedural stand-in (the
    /// section at the end; eo-app `--stand-ins`, on in the high and hyper
    /// tiers). Off (the default): the grey proxy boxes, the box round the
    /// signal lenses, the bare palm trunk and the plain distribution pole as
    /// before the stand-ins, and no cobra arm or lantern without their GLBs.
    /// With every GLB present it changes nothing. `PropLibrary::key_with`
    /// hashes it only when on.
    pub stand_ins: bool,
    /// The moored boats' bodies (`eo_props::boats`; eo-app `--no-boats` leaves them out):
    /// render models `BOAT_BASE + i`, the three levels from the body's three GLBs.
    pub boats: Option<&'a [eo_props::boats::Body]>,
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
            // (a v2 file's colours are as authored: no load-time repaint)
            let v2 = eo_props::props_v2::active().is_some_and(|v| v.is_v2(prop));
            if let Some((_, _, rgb)) = g.maps[0].as_mut().filter(|_| !v2) {
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
    // A v2 file (eo_props::props_v2) is built to the v1 slot's budget and keeps its thin
    // members (lamp arms, crossarms) that clustering erases: level 1 is the file itself, and
    // level 2 as well for a light file (the cobra and banner lamps' arms past 75 m).
    let v2 = js.street_prop(name).ok().is_some_and(|p| eo_props::props_v2::active().is_some_and(|v| v.is_v2(p)));
    let m1 = if v2 {
        m0
    } else {
        let (p1, n1, u1, i1) = cluster(&g, diag / 24.0);
        add_mesh(&mut out.b, m, &p1, &n1, &u1, &i1);
        out.finish()
    };
    let m2 = if v2 && g.indices.len() / 3 <= V2_FAR_FULL_TRIANGLES {
        m0
    } else {
        let (p2, n2, u2, i2) = cluster(&g, diag / 9.0);
        add_mesh(&mut out.b, m, &p2, &n2, &u2, &i2);
        out.finish()
    };
    out.set(id, [m0, m1, m2], lod, cast, 0.0);
    true
}

/// A v2 file up to this many triangles is its own far level (no clustering).
const V2_FAR_FULL_TRIANGLES: usize = 2000;

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
    // A kind whose GLB is absent takes its procedural stand-in with `x.stand_ins` (the
    // materials and the context are made on the first one, so a checkout with every GLB
    // is unchanged); without it, the grey proxy box as before the stand-ins.
    let mut stand: Option<(PMats, StandInCtx)> = None;
    for (name, model, lod, h) in glbs {
        if !glb_model(out, repo, js, name, model as u32, lod, cascades_for(h)) {
            let drawn = x.stand_ins && {
                let (pm, cx) = stand_kit(&mut stand, out, js, x, [m_paint, m_metal, m_matte]);
                stand_in(out, &pm, &cx, name, model as u32, lod, cascades_for(h))
            };
            if !drawn {
                // A proxy at the kind's height (the JS keeps its procedural mesh).
                bx(&mut out.b, m_paint, 0.3, h, 0.3, "#6d6f6d", 0.0, h / 2.0, 0.0, 1.0);
                let k = out.finish();
                out.set(model as u32, [k, k, k], [near, near, 0.0, near], cascades_for(h), 0.0);
            }
        }
    }
    // The vehicle signal head and the palm are cooked by `props_lib`; without their GLBs
    // it draws a plain box round the lenses and a bare trunk, replaced here (`x.stand_ins`).
    if x.stand_ins && glb_absent(out, "signalHead") {
        let (pm, cx) = stand_kit(&mut stand, out, js, x, [m_paint, m_metal, m_matte]);
        signal_head_stand_in(out, &pm, &cx);
    }
    if x.stand_ins && glb_absent(out, "palm") {
        let (pm, cx) = stand_kit(&mut stand, out, js, x, [m_paint, m_metal, m_matte]);
        // The palm's own LOD edges and cascades (`props_lib`, the JS palm and palmFar GLBs).
        stand_in(out, &pm, &cx, "palm", Model::Palm as u32, [120.0, 450.0, 450.0, 450.0], 3);
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
        // Without a GLB each takes its procedural stand-in at the GLB's edges and cascades
        // (`x.stand_ins`); without that, the plain pole as before and no arm or lantern.
        if x.stand_ins {
            for (name, id, lod, cast) in [("utilityPole", OH_DIST_POLE, [40.0, 120.0, 0.0, 360.0], 3), ("cobraArm", OH_COBRA_ARM, [30.0, 90.0, 0.0, 200.0], 2), ("lantern", OH_LANTERN, [20.0, 50.0, 0.0, 120.0], 1)] {
                if !glb_model(out, repo, js, name, id, lod, cast) {
                    let (pm, cx) = stand_kit(&mut stand, out, js, x, [m_paint, m_metal, m_matte]);
                    stand_in(out, &pm, &cx, name, id, lod, cast);
                }
            }
        } else {
            if !glb_model(out, repo, js, "utilityPole", OH_DIST_POLE, [40.0, 120.0, 0.0, 360.0], 3) {
                out.b.cylinder([0.0; 3], 0.0, 11.7, 0.2, 0.14, 6, srgb_hex("#5a4a3a"), m_matte, true);
                let n = out.finish();
                out.set(OH_DIST_POLE, [n, n, n], [360.0, 360.0, 0.0, 360.0], 3, 0.0);
            }
            glb_model(out, repo, js, "cobraArm", OH_COBRA_ARM, [30.0, 90.0, 0.0, 200.0], 2);
            glb_model(out, repo, js, "lantern", OH_LANTERN, [20.0, 50.0, 0.0, 120.0], 1);
        }
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
    if let Some(bodies) = x.boats {
        cook_boats(out, repo, bodies);
    }
    info
}

/// The moored boats: per body, its three GLBs as levels 0..2 (each file digest-checked),
/// on the boats' atlas (one texture layer while the bodies share its bytes, as built).
/// A body with a file missing or refused is left out (its berths stay empty).
fn cook_boats(out: &mut Out, repo: &std::path::Path, bodies: &[eo_props::boats::Body]) {
    let mut atlases: Vec<(Vec<u8>, u32)> = vec![]; // (albedo bytes, material)
    for (i, body) in bodies.iter().enumerate().take(eo_props::boats::BODIES.len()) {
        let read: Result<Vec<Glb>, String> = body.lods.iter().map(|l| read_glb(repo, l).map_err(|e| format!("{} ({e:#})", l.path))).collect();
        let glbs = match read {
            Ok(g) => g,
            Err(e) => {
                out.missing.push(format!("boat {}: {e}", body.name));
                continue;
            }
        };
        let albedo = glbs[0].maps[0].as_ref().map(|m| m.2.clone()).unwrap_or_default();
        let m = match atlases.iter().find(|a| a.0 == albedo) {
            Some(a) => a.1,
            None => {
                let g = &glbs[0];
                let flat_n = flat_layer([128, 128, 255, 255], AUX_RES);
                let flat_orm = flat_layer([255, 255, 0, 255], AUX_RES);
                let white = flat_layer([255; 4], ALBEDO_RES);
                let map = |k: usize, res: u32, d: &Vec<u8>| match &g.maps[k] {
                    Some((w, h, rgb)) => resample(*w, *h, rgb, 3, res),
                    None => d.clone(),
                };
                out.layers.push([map(0, ALBEDO_RES, &white), map(1, AUX_RES, &flat_n), map(2, AUX_RES, &flat_orm)]);
                let m = glb_mat(out.materials, out.layers.len() as i32 - 1);
                atlases.push((albedo, m));
                m
            }
        };
        let mut ids = [0u32; 3];
        for (k, g) in glbs.iter().enumerate() {
            add_mesh(&mut out.b, m, &g.positions, &g.normals, &g.uvs, &g.indices);
            ids[k] = out.finish();
        }
        out.set(BOAT_BASE + i as u32, ids, BOAT_LOD, cascades_for(body.height as f32), 0.0);
        out.sources.push(format!("boat {}: {} (sha256 {})", body.name, body.lods[0].path, &body.lods[0].sha256[..12]));
    }
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

// ================================================================ stand-ins for absent GLBs
//
// The street-prop GLBs cannot be redistributed, so a checkout may lack some of
// them. Each kind whose GLB is absent is built here instead: a procedural model of
// the San Francisco design at the GLB's dimensions (`STREET_PROP_ASSETS_V1`
// `dimensionsM`, quoted per kind), in three levels like the other procedural
// kinds (near: full detail; mid: the silhouette and the main colour breaks, which
// is also what the near sun cascade casts; far: a few prisms). Vertex colours
// carry the paint, and the parts that stand on the footway take a band of road
// dirt near the ground (`dirt`). The GLB, when present, is drawn instead and none
// of this runs (no stand-in material is pushed either).

use glam::{Quat, Vec3};

/// The stand-ins' materials: `cook_extras`' three plain responses and galvanised
/// steel, glass and a satin finish, pushed on the first stand-in built.
#[derive(Clone, Copy)]
struct PMats {
    paint: u32,
    metal: u32,
    matte: u32,
    galv: u32,
    glass: u32,
    satin: u32,
}

impl PMats {
    fn push(v: &mut Vec<PropMaterial>, paint: u32, metal: u32, matte: u32) -> Self {
        let white = [255u8; 3];
        // Hot-dip galvanised steel weathers to a dull, partly metallic grey; glass is smooth;
        // the satin finish is powder coat and enamel signs.
        PMats { paint, metal, matte, galv: push_mat(v, white, 0.55, 0.2, 0, 0), glass: push_mat(v, white, 0.1, 0.0, 0, 0), satin: push_mat(v, white, 0.4, 0.0, 0, 0) }
    }
}

/// What the stand-ins read beyond the kind: the district light heads
/// (`LAMP_HEAD`, furniture-js) so a luminaire sits where its light is, the
/// vehicle signal head's half height (`signalHardware`), the distribution pole's
/// wire seats and hardware floor (overhead-js `utilityPole`).
#[derive(Clone, Debug)]
pub(crate) struct StandInCtx {
    /// District head: height over the foot and reach toward the street (local -z), m.
    pub district: [f32; 2],
    /// Historic column head height, m.
    pub column: f32,
    pub head_half: f32,
    /// Distribution pole: height, lowest hardware, crossarm length, and per crossarm (min x, max x, seat y, seat z).
    pub pole: [f32; 3],
    pub arms: Vec<[f32; 4]>,
}

impl StandInCtx {
    /// The values above from the JS exports. Each falls back to the export's own value
    /// (quoted) when an export does not load, so a stand-in is never left out.
    pub(crate) fn load(js: &JsProps, oh: Option<&eo_props::overhead::OverheadJs>) -> Self {
        let f = eo_props::furniture::data::FurnitureJs::load().ok();
        let head = |k: &str, d: [f64; 2]| f.as_ref().and_then(|f| f.consts.lamp_head.get(k).map(|h| [h.height_m, h.reach_m])).unwrap_or(d);
        let d = head("district", [7.4, 2.9]);
        let c = head("column", [5.12, 0.0]);
        let up = oh.map(|o| o.consts["utilityPole"].clone()).unwrap_or_default();
        let num = |k: &str, dflt: f64| up[k].as_f64().unwrap_or(dflt) as f32;
        let mut arms: Vec<[f32; 4]> = oh.map(|o| o.crossarms().into_iter().map(|a| a.map(|v| v as f32)).collect()).unwrap_or_default();
        if arms.is_empty() {
            arms = vec![[-0.9323, 0.9543, 11.4978, -0.2137], [-0.9178, 0.9543, 10.7867, -0.1485]];
        }
        StandInCtx {
            district: [d[0] as f32, d[1] as f32],
            column: c[0] as f32,
            head_half: js.try_num("signalHardware", "headHalfHeightM").unwrap_or(0.53) as f32,
            pole: [num("heightM", 11.73), num("hardwareFromM", 8.668), num("crossarmM", 2.438)],
            arms,
        }
    }
}

// ---------------------------------------------------------------- the stand-ins' modelling kit

fn hex(h: &str) -> [u8; 3] {
    srgb_hex(h)
}

/// A colour scaled in linear light.
fn shade(c: [u8; 3], k: f32) -> [u8; 3] {
    [to_srgb8(lin(c[0]) * k), to_srgb8(lin(c[1]) * k), to_srgb8(lin(c[2]) * k)]
}

/// Facets of a round part at a level (near count, half at mid, a third far).
fn fa(level: usize, near: u32) -> u32 {
    match level {
        0 => near,
        1 => (near / 2).max(6),
        _ => (near / 3).max(4),
    }
}

fn v3(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, y, z)
}

/// Heights (model metres) where the sides of a part standing on the footway are cut,
/// so the base dirt has vertices to fade over.
const DIRT_CUTS: [f32; 2] = [0.1, 0.32];

/// A surface of revolution's options: `facet` gives flat faces (an octagonal
/// plinth), `flutes` > 0 cuts that many round flutes `flute` deep (a fraction of the
/// radius), `square` > 2 makes the section a superellipse of that exponent (a
/// rounded square), `phase` turns the first facet, `open` leaves the ends uncapped.
#[derive(Clone, Copy)]
struct Turn {
    sides: u32,
    facet: bool,
    flutes: u32,
    flute: f32,
    square: f32,
    phase: f32,
    open: bool,
}

impl Turn {
    fn round(sides: u32) -> Self {
        Turn { sides, facet: false, flutes: 0, flute: 0.0, square: 0.0, phase: 0.0, open: false }
    }
    fn facets(sides: u32) -> Self {
        Turn { sides, facet: true, flutes: 0, flute: 0.0, square: 0.0, phase: std::f32::consts::PI / sides as f32, open: false }
    }
    fn fluted(sides: u32, flutes: u32, flute: f32) -> Self {
        Turn { sides, facet: false, flutes, flute, square: 0.0, phase: 0.0, open: false }
    }
    fn squared(sides: u32, square: f32) -> Self {
        Turn { sides, facet: false, flutes: 0, flute: 0.0, square, phase: std::f32::consts::FRAC_PI_4, open: false }
    }
    /// The radius factor at angle `a`.
    fn factor(&self, a: f32) -> f32 {
        let mut f = 1.0;
        if self.flutes > 0 {
            f *= 1.0 - self.flute * (0.5 - 0.5 * (self.flutes as f32 * a).cos());
        }
        if self.square > 2.0 {
            let (s, c) = a.sin_cos();
            // Superellipse |x|^p + |z|^p = 1: the radius is the half-width across the flats.
            f *= (c.abs().powf(self.square) + s.abs().powf(self.square)).powf(-1.0 / self.square);
        }
        f
    }
}

/// A surface of revolution about +Y through `at`: `prof` is (radius, y, smooth) bottom
/// up, and the normal breaks at a point whose smooth value is 0. Caps close an end
/// whose radius is above zero.
fn turn(b: &mut Builder, m: u32, prof: &[[f32; 3]], t: Turn, col: [u8; 3], at: Vec3) {
    let n = prof.len();
    if n < 2 || t.sides < 3 {
        return;
    }
    let seg: Vec<[f32; 2]> = prof
        .windows(2)
        .map(|w| {
            let (dr, dy) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
            let l = (dr * dr + dy * dy).sqrt().max(1e-6);
            [dy / l, -dr / l]
        })
        .collect();
    let mix = |a: [f32; 2], c: [f32; 2]| {
        let s = [a[0] + c[0], a[1] + c[1]];
        let l = (s[0] * s[0] + s[1] * s[1]).sqrt().max(1e-6);
        [s[0] / l, s[1] / l]
    };
    let ang = |i: u32| t.phase + i as f32 / t.sides as f32 * std::f32::consts::TAU;
    let point = |p: [f32; 3], a: f32| {
        let (s, c) = a.sin_cos();
        let f = t.factor(a);
        at + v3(c * p[0] * f, p[1], s * p[0] * f)
    };
    let normal = |nn: [f32; 2], a: f32| {
        // The section's outward normal at `a` (from the radius factor's slope), tilted by the profile.
        let e = 1e-3;
        let (s, c) = a.sin_cos();
        let f = t.factor(a);
        let df = (t.factor(a + e) - t.factor(a - e)) / (2.0 * e);
        let h = (v3(c, 0.0, s) - v3(-s, 0.0, c) * (df / f.max(1e-3))).normalize_or_zero();
        (h * nn[0] + Vec3::Y * nn[1]).normalize_or(Vec3::Y)
    };
    for k in 0..n - 1 {
        if prof[k][0] <= 1e-5 && prof[k + 1][0] <= 1e-5 {
            continue;
        }
        let na = if prof[k][2] > 0.5 && k > 0 { mix(seg[k - 1], seg[k]) } else { seg[k] };
        let nb = if prof[k + 1][2] > 0.5 && k + 1 < n - 1 { mix(seg[k], seg[k + 1]) } else { seg[k] };
        for i in 0..t.sides {
            let (a0, a1) = (ang(i), ang(i + 1));
            let (n0, n1) = if t.facet { ((a0 + a1) * 0.5, (a0 + a1) * 0.5) } else { (a0, a1) };
            let base = b.v.len() as u32;
            b.vert(point(prof[k], a0).to_array(), normal(na, n0).to_array(), [0.0; 2], col, m);
            b.vert(point(prof[k], a1).to_array(), normal(na, n1).to_array(), [0.0; 2], col, m);
            b.vert(point(prof[k + 1], a1).to_array(), normal(nb, n1).to_array(), [0.0; 2], col, m);
            b.vert(point(prof[k + 1], a0).to_array(), normal(nb, n0).to_array(), [0.0; 2], col, m);
            b.tri(false, base, base + 1, base + 2);
            b.tri(false, base, base + 2, base + 3);
        }
    }
    for (p, ny) in [(prof[0], -1.0f32), (prof[n - 1], 1.0)] {
        if p[0] <= 1e-5 || t.open {
            continue;
        }
        let c = b.vert((at + Vec3::Y * p[1]).to_array(), [0.0, ny, 0.0], [0.0; 2], col, m);
        let ring = b.v.len() as u32;
        for i in 0..=t.sides {
            b.vert(point(p, ang(i)).to_array(), [0.0, ny, 0.0], [0.0; 2], col, m);
        }
        for i in 0..t.sides {
            b.tri(false, c, ring + i, ring + i + 1);
        }
    }
}

/// A profile point that breaks the normal, and one the normal runs smoothly through.
fn kp(r: f32, y: f32) -> [f32; 3] {
    [r, y, 0.0]
}
fn sp(r: f32, y: f32) -> [f32; 3] {
    [r, y, 1.0]
}

fn quad(b: &mut Builder, m: u32, p: [Vec3; 4], n: Vec3, col: [u8; 3]) {
    let base = b.v.len() as u32;
    for (k, q) in p.iter().enumerate() {
        b.vert(q.to_array(), n.to_array(), [(k == 1 || k == 2) as u8 as f32, (k >= 2) as u8 as f32], col, m);
    }
    b.tri(false, base, base + 1, base + 2);
    b.tri(false, base, base + 2, base + 3);
}

/// An axis-aligned box from `mn` to `mx`, its sides cut at `DIRT_CUTS`.
fn bxa(b: &mut Builder, m: u32, mn: Vec3, mx: Vec3, col: [u8; 3]) {
    let mut ys = vec![mn.y];
    ys.extend(DIRT_CUTS.iter().copied().filter(|&c| c > mn.y + 1e-3 && c < mx.y - 1e-3));
    ys.push(mx.y);
    quad(b, m, [v3(mn.x, mx.y, mn.z), v3(mn.x, mx.y, mx.z), v3(mx.x, mx.y, mx.z), v3(mx.x, mx.y, mn.z)], Vec3::Y, col);
    quad(b, m, [v3(mn.x, mn.y, mn.z), v3(mx.x, mn.y, mn.z), v3(mx.x, mn.y, mx.z), v3(mn.x, mn.y, mx.z)], -Vec3::Y, col);
    let sides: [(Vec3, [f32; 2], [f32; 2]); 4] = [
        (Vec3::X, [mx.x, mn.z], [mx.x, mx.z]),
        (-Vec3::X, [mn.x, mx.z], [mn.x, mn.z]),
        (Vec3::Z, [mx.x, mx.z], [mn.x, mx.z]),
        (-Vec3::Z, [mn.x, mn.z], [mx.x, mn.z]),
    ];
    for (n, a, c) in sides {
        for w in ys.windows(2) {
            quad(b, m, [v3(a[0], w[0], a[1]), v3(c[0], w[0], c[1]), v3(c[0], w[1], c[1]), v3(a[0], w[1], a[1])], n, col);
        }
    }
}

/// `bxa` by centre and size.
fn bxc(b: &mut Builder, m: u32, c: Vec3, size: Vec3, col: [u8; 3]) {
    bxa(b, m, c - size * 0.5, c + size * 0.5, col);
}

/// A box of `size` turned by `q` about its centre `c`.
fn cube(b: &mut Builder, m: u32, c: Vec3, size: Vec3, q: Quat, col: [u8; 3]) {
    let h = size * 0.5;
    let p = |x: f32, y: f32, z: f32| c + q * v3(x * h.x, y * h.y, z * h.z);
    let faces: [(Vec3, [Vec3; 4]); 6] = [
        (Vec3::X, [p(1., -1., -1.), p(1., 1., -1.), p(1., 1., 1.), p(1., -1., 1.)]),
        (-Vec3::X, [p(-1., -1., 1.), p(-1., 1., 1.), p(-1., 1., -1.), p(-1., -1., -1.)]),
        (Vec3::Y, [p(-1., 1., -1.), p(-1., 1., 1.), p(1., 1., 1.), p(1., 1., -1.)]),
        (-Vec3::Y, [p(-1., -1., -1.), p(1., -1., -1.), p(1., -1., 1.), p(-1., -1., 1.)]),
        (Vec3::Z, [p(1., -1., 1.), p(1., 1., 1.), p(-1., 1., 1.), p(-1., -1., 1.)]),
        (-Vec3::Z, [p(-1., -1., -1.), p(-1., 1., -1.), p(1., 1., -1.), p(1., -1., -1.)]),
    ];
    for (n, f) in faces {
        quad(b, m, f, q * n, col);
    }
}

/// A round tube along `pts` (radius `r[i]` at point i, the last given radius beyond),
/// smooth normals on parallel-transported frames, flat caps on its ends when `caps`.
fn sweep(b: &mut Builder, m: u32, pts: &[Vec3], r: &[f32], sides: u32, col: [u8; 3], caps: bool) {
    let n = pts.len();
    if n < 2 || r.is_empty() {
        return;
    }
    let rad = |i: usize| r[i.min(r.len() - 1)];
    let tan = |i: usize| (pts[(i + 1).min(n - 1)] - pts[i.saturating_sub(1)]).normalize_or(Vec3::Y);
    let t0 = tan(0);
    let mut nrm = if t0.y.abs() < 0.9 { t0.cross(Vec3::Y).normalize() } else { t0.cross(Vec3::X).normalize() };
    let mut prev = t0;
    let base = b.v.len() as u32;
    let mut frames = Vec::with_capacity(n);
    for i in 0..n {
        let t = tan(i);
        nrm = (Quat::from_rotation_arc(prev, t) * nrm).normalize_or(nrm);
        prev = t;
        let bin = t.cross(nrm);
        frames.push((nrm, bin));
        for j in 0..=sides {
            let a = j as f32 / sides as f32 * std::f32::consts::TAU;
            let d = nrm * a.cos() + bin * a.sin();
            b.vert((pts[i] + d * rad(i)).to_array(), d.to_array(), [0.0; 2], col, m);
        }
    }
    let row = sides + 1;
    for i in 0..n as u32 - 1 {
        for j in 0..sides {
            let a = base + i * row + j;
            let c = a + row;
            b.tri(false, a, c, a + 1);
            b.tri(false, a + 1, c, c + 1);
        }
    }
    if caps {
        for (i, s) in [(0usize, -1.0f32), (n - 1, 1.0)] {
            let nn = (tan(i) * s).to_array();
            let c = b.vert(pts[i].to_array(), nn, [0.0; 2], col, m);
            let ring = b.v.len() as u32;
            let (fu, fv) = frames[i];
            for j in 0..=sides {
                let a = j as f32 / sides as f32 * std::f32::consts::TAU;
                b.vert((pts[i] + (fu * a.cos() + fv * a.sin()) * rad(i)).to_array(), nn, [0.0; 2], col, m);
            }
            for j in 0..sides {
                b.tri(false, c, ring + j, ring + j + 1);
            }
        }
    }
}

/// A cubic Bezier sampled at `k` + 1 points.
fn bez(p: [Vec3; 4], k: usize) -> Vec<Vec3> {
    (0..=k)
        .map(|i| {
            let t = i as f32 / k as f32;
            let u = 1.0 - t;
            p[0] * (u * u * u) + p[1] * (3.0 * u * u * t) + p[2] * (3.0 * u * t * t) + p[3] * (t * t * t)
        })
        .collect()
}

/// A lofted shell along local -z: per station (z, half width, y of the bottom, y of
/// the top), a superellipse section of exponent `p` (flat bottom when `flat`), closed
/// at both ends. Smooth normals from the grid.
fn loft(b: &mut Builder, m: u32, st: &[[f32; 4]], ring: u32, p: f32, flat: bool, col: [u8; 3], at: Vec3) {
    let n = st.len();
    if n < 2 {
        return;
    }
    let sec = |s: [f32; 4], a: f32| {
        let (sn, cs) = a.sin_cos();
        // The lower half of a flat-bottomed section is a much squarer superellipse.
        let e = if flat && sn < 0.0 { 0.2 } else { 2.0 / p };
        let x = cs.signum() * cs.abs().powf(e) * s[1];
        let mid = (s[2] + s[3]) * 0.5;
        let half = (s[3] - s[2]) * 0.5;
        let y = mid + sn.signum() * sn.abs().powf(e) * half;
        at + v3(x, y, s[0])
    };
    let grid: Vec<Vec<Vec3>> = st.iter().map(|&s| (0..ring).map(|j| sec(s, j as f32 / ring as f32 * std::f32::consts::TAU)).collect()).collect();
    let base = b.v.len() as u32;
    for i in 0..n {
        for j in 0..ring as usize {
            let jn = (j + 1) % ring as usize;
            let jp = (j + ring as usize - 1) % ring as usize;
            let around = grid[i][jn] - grid[i][jp];
            let along = grid[(i + 1).min(n - 1)][j] - grid[i.saturating_sub(1)][j];
            let mut nn = around.cross(along).normalize_or(Vec3::Y);
            let c = (grid[i].iter().copied().sum::<Vec3>()) / ring as f32;
            if nn.dot(grid[i][j] - c) < 0.0 {
                nn = -nn;
            }
            b.vert(grid[i][j].to_array(), nn.to_array(), [0.0; 2], col, m);
        }
    }
    for i in 0..n as u32 - 1 {
        for j in 0..ring {
            let jn = (j + 1) % ring;
            let (a, c, d, e) = (base + i * ring + j, base + i * ring + jn, base + (i + 1) * ring + jn, base + (i + 1) * ring + j);
            b.tri(false, a, c, d);
            b.tri(false, a, d, e);
        }
    }
    for (i, s) in [(0usize, 1.0f32), (n - 1, -1.0)] {
        let c = grid[i].iter().copied().sum::<Vec3>() / ring as f32;
        let nn = v3(0.0, 0.0, s);
        let ci = b.vert(c.to_array(), nn.to_array(), [0.0; 2], col, m);
        let r0 = b.v.len() as u32;
        for j in 0..ring as usize {
            b.vert(grid[i][j].to_array(), nn.to_array(), [0.0; 2], col, m);
        }
        for j in 0..ring {
            b.tri(false, ci, r0 + j, r0 + (j + 1) % ring);
        }
    }
}

/// Weathering: the vertex colours from `from` on varied by up to `amp` (a fraction, in
/// linear light) by a hash of the position, so no painted part is one flat tone.
fn mottle(b: &mut Builder, from: usize, amp: f32) {
    for v in &mut b.v[from..] {
        let q = v.pos.map(|c| (c * 40.0).round() as i32);
        let mut h = (q[0] as u32).wrapping_mul(0x9e37_79b1) ^ (q[1] as u32).wrapping_mul(0x85eb_ca77) ^ (q[2] as u32).wrapping_mul(0xc2b2_ae3d);
        h ^= h >> 15;
        h = h.wrapping_mul(0x2c1b_3c6d);
        h ^= h >> 12;
        let k = 1.0 + amp * ((h & 0xffff) as f32 / 65535.0 - 0.5) * 2.0;
        for c in 0..3 {
            v.color[c] = to_srgb8(lin(v.color[c]) * k);
        }
    }
}

/// Road dirt on the vertex colours from `from` on: blended toward a dust grey-brown
/// near the ground (model y), strongest at the foot and gone by 0.9 m, so the part
/// sits in the street (`k` 0..1 its strength).
fn dirt(b: &mut Builder, from: usize, k: f32) {
    let ss = |e0: f32, e1: f32, x: f32| {
        let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    // A dry road-dust tone (sRGB about #6b6357), in linear light.
    let dust = [0.147f32, 0.125, 0.096];
    for v in &mut b.v[from..] {
        let y = v.pos[1];
        let d = k * (0.75 * (1.0 - ss(0.0, 0.32, y)) + 0.25 * (1.0 - ss(0.0, 0.9, y)));
        if d <= 0.0 {
            continue;
        }
        for c in 0..3 {
            let l = lin(v.color[c]);
            v.color[c] = to_srgb8(l + (dust[c] - l) * d);
        }
    }
}

/// Turns and moves the vertices from `from` on (positions and normals).
fn xform(b: &mut Builder, from: usize, q: Quat, t: Vec3) {
    for v in &mut b.v[from..] {
        v.pos = (q * Vec3::from(v.pos) + t).to_array();
        let n = q * v3(v.normal[0] as f32, v.normal[1] as f32, v.normal[2] as f32);
        v.normal = [n.x.round().clamp(-127.0, 127.0) as i8, n.y.round().clamp(-127.0, 127.0) as i8, n.z.round().clamp(-127.0, 127.0) as i8, 0];
    }
}

/// A surface of revolution laid along `axis` (its +Y turned onto `axis`), base at `at`.
fn turn_on(b: &mut Builder, m: u32, prof: &[[f32; 3]], t: Turn, col: [u8; 3], at: Vec3, axis: Vec3) {
    let from = b.v.len();
    turn(b, m, prof, t, col, Vec3::ZERO);
    xform(b, from, Quat::from_rotation_arc(Vec3::Y, axis.normalize()), at);
}

// ---------------------------------------------------------------- the kinds (local -z faces the street)

/// SF low-pressure hydrant (`hydrant` 0.50 x 0.78 x 0.44 m): a flanged foot, a short
/// barrel with a raised band, the outlet section with two side outlets and the
/// pumper toward the street, a domed bonnet with a pentagon operating nut. White
/// body as the GLB is repainted (`whitenHydrantPaint`); the bonnet in the blue the
/// city paints on its newer low-pressure hydrants.
fn hydrant(b: &mut Builder, pm: &PMats, l: usize) {
    let body = hex("#ecebe4");
    let bonnet = hex("#4a7db0");
    if l == 2 {
        turn(b, pm.paint, &[kp(0.16, 0.0), kp(0.16, 0.04), kp(0.12, 0.05), kp(0.125, 0.58)], Turn::round(6), body, Vec3::ZERO);
        turn(b, pm.paint, &[kp(0.135, 0.58), sp(0.12, 0.67), kp(0.03, 0.78)], Turn::round(6), bonnet, Vec3::ZERO);
        bxc(b, pm.paint, v3(0.0, 0.5, 0.0), v3(0.48, 0.1, 0.1), body);
        bxc(b, pm.paint, v3(0.0, 0.47, -0.1), v3(0.13, 0.13, 0.22), body);
        return;
    }
    let s = fa(l, 20);
    turn(b, pm.paint, &[kp(0.17, 0.0), kp(0.17, 0.035), kp(0.135, 0.05), kp(0.118, 0.07), sp(0.112, 0.2), kp(0.115, 0.33), kp(0.135, 0.34), kp(0.135, 0.39), kp(0.122, 0.4), sp(0.128, 0.5), kp(0.124, 0.58)], Turn::round(s), body, Vec3::ZERO);
    turn(b, pm.paint, &[kp(0.142, 0.575), kp(0.142, 0.61), sp(0.127, 0.65), sp(0.088, 0.705), kp(0.045, 0.725), kp(0.032, 0.73)], Turn::round(s), bonnet, Vec3::ZERO);
    turn(b, pm.metal, &[kp(0.032, 0.725), kp(0.028, 0.78)], Turn::facets(5), shade(bonnet, 0.75), Vec3::ZERO);
    let o = fa(l, 14);
    for sx in [-1.0f32, 1.0] {
        let ax = v3(sx, 0.0, 0.0);
        turn_on(b, pm.paint, &[kp(0.05, 0.0), kp(0.05, 0.1), kp(0.058, 0.105), kp(0.058, 0.14)], Turn::round(o), body, v3(sx * 0.105, 0.5, 0.0), ax);
        turn_on(b, pm.metal, &[kp(0.024, 0.0), kp(0.02, 0.035)], Turn::facets(5), shade(bonnet, 0.75), v3(sx * 0.245, 0.5, 0.0), ax);
    }
    let ax = v3(0.0, 0.0, -1.0);
    turn_on(b, pm.paint, &[kp(0.064, 0.0), kp(0.064, 0.085), kp(0.074, 0.09), kp(0.074, 0.12)], Turn::round(o), body, v3(0.0, 0.47, -0.1), ax);
    turn_on(b, pm.metal, &[kp(0.026, 0.0), kp(0.022, 0.03)], Turn::facets(5), shade(bonnet, 0.75), v3(0.0, 0.47, -0.22), ax);
    if l == 0 {
        // Flange bolts.
        for k in 0..8 {
            let a = k as f32 / 8.0 * std::f32::consts::TAU + 0.2;
            bxc(b, pm.metal, v3(a.cos() * 0.152, 0.042, a.sin() * 0.152), v3(0.022, 0.016, 0.022), shade(body, 0.6));
        }
        // Cap chains, a link hanging from each side cap toward the barrel.
        for sx in [-1.0f32, 1.0] {
            sweep(b, pm.metal, &bez([v3(sx * 0.215, 0.47, 0.03), v3(sx * 0.2, 0.36, 0.05), v3(sx * 0.15, 0.36, 0.08), v3(sx * 0.12, 0.43, 0.09)], 5), &[0.004], 4, shade(body, 0.5), false);
        }
    }
}

/// SFMTA single-space meter (`parkingMeter` 0.33 x 1.25 x 0.22 m): a grey pipe post
/// and a charcoal smart-meter head with a round top, its display, keypad, card and
/// coin slots facing the footway (+z), a small solar panel on the dome.
fn parking_meter(b: &mut Builder, pm: &PMats, l: usize) {
    let post = hex("#6d7174");
    let body = hex("#4a5054");
    let trim = hex("#a3a8aa");
    let s = fa(l, 12);
    if l == 2 {
        bxc(b, pm.paint, v3(0.0, 0.46, 0.0), v3(0.06, 0.92, 0.06), post);
        bxc(b, pm.paint, v3(0.0, 1.08, 0.0), v3(0.22, 0.32, 0.17), body);
        return;
    }
    bxc(b, pm.metal, v3(0.0, 0.006, 0.0), v3(0.14, 0.012, 0.14), shade(post, 0.8));
    turn(b, pm.paint, &[kp(0.03, 0.0), kp(0.03, 0.86)], Turn::round(s), post, Vec3::ZERO);
    turn(b, pm.paint, &[kp(0.046, 0.86), kp(0.046, 0.935)], Turn::round(s), body, Vec3::ZERO);
    bxa(b, pm.paint, v3(-0.11, 0.93, -0.085), v3(0.11, 1.14, 0.085), body);
    cyl(b, pm.paint, 0.11, 0.11, 0.17, fa(l, 16), body, [0.0, 1.14, 0.0], Some('z'));
    // Front bezel, display, slots.
    bxc(b, pm.metal, v3(0.0, 1.07, 0.088), v3(0.19, 0.24, 0.008), trim);
    bxc(b, pm.glass, v3(0.0, 1.14, 0.093), v3(0.13, 0.07, 0.004), hex("#2b3a35"));
    bxc(b, pm.paint, v3(0.05, 1.0, 0.094), v3(0.06, 0.01, 0.006), hex("#141618"));
    bxc(b, pm.satin, v3(0.0, 0.965, 0.0875), v3(0.1, 0.045, 0.004), hex("#1f55a5"));
    if l == 0 {
        for k in 0..4 {
            bxc(b, pm.satin, v3(-0.045 + k as f32 * 0.03, 1.085, 0.095), v3(0.02, 0.016, 0.008), hex("#2c3134"));
        }
        bxc(b, pm.paint, v3(-0.05, 1.0, 0.094), v3(0.012, 0.03, 0.006), hex("#141618"));
        // Solar panel on the dome, tilted toward the footway.
        cube(b, pm.glass, v3(0.0, 1.252, 0.01), v3(0.15, 0.01, 0.1), Quat::from_rotation_x(0.35), hex("#20262e"));
    }
}

/// SFMTA pay station (`payStation`, a charcoal box about 0.42 x 1.5 x 0.3 m as the
/// JS stands it upright): plinth, cabinet, a hood carrying the solar panel, the
/// screen, keypad, card reader and ticket tray toward the footway, the blue P plate.
fn pay_station(b: &mut Builder, pm: &PMats, l: usize) {
    let body = hex("#464c51");
    let dark = hex("#24292d");
    let steel = hex("#9aa0a2");
    let tilt = Quat::from_rotation_x(0.2);
    bxc(b, pm.paint, v3(0.0, 0.04, 0.0), v3(0.38, 0.08, 0.28), dark);
    bxa(b, pm.paint, v3(-0.2, 0.08, -0.14), v3(0.2, 1.38, 0.14), body);
    cube(b, pm.paint, v3(0.0, 1.415, 0.02), v3(0.46, 0.05, 0.36), tilt, body);
    if l == 2 {
        return;
    }
    cube(b, pm.glass, v3(0.0, 1.444, 0.024), v3(0.4, 0.012, 0.3), tilt, hex("#20262e"));
    bxc(b, pm.paint, v3(0.0, 1.13, 0.142), v3(0.34, 0.42, 0.006), dark);
    bxc(b, pm.glass, v3(0.0, 1.22, 0.147), v3(0.21, 0.15, 0.004), hex("#1d3138"));
    bxc(b, pm.satin, v3(0.0, 1.335, 0.146), v3(0.15, 0.07, 0.006), hex("#1f55a5"));
    bxc(b, pm.satin, v3(-0.045, 1.335, 0.15), v3(0.03, 0.05, 0.003), hex("#eef0f0"));
    bxc(b, pm.paint, v3(0.0, 0.86, 0.15), v3(0.17, 0.05, 0.03), dark);
    if l == 0 {
        for r in 0..4 {
            for c in 0..3 {
                bxc(b, pm.metal, v3(-0.03 + c as f32 * 0.03, 1.12 - r as f32 * 0.024, 0.148), v3(0.022, 0.016, 0.008), steel);
            }
        }
        bxc(b, pm.paint, v3(0.11, 1.08, 0.149), v3(0.06, 0.012, 0.01), hex("#0e1012"));
        bxc(b, pm.metal, v3(0.11, 1.11, 0.147), v3(0.07, 0.03, 0.008), steel);
        for x in [-0.17f32, 0.17] {
            bxc(b, pm.paint, v3(x, 0.5, 0.1405), v3(0.006, 0.8, 0.002), dark);
        }
        bxc(b, pm.paint, v3(0.0, 0.92, 0.1405), v3(0.34, 0.006, 0.002), dark);
        bxc(b, pm.satin, v3(0.06, 0.64, 0.1415), v3(0.08, 0.1, 0.002), hex("#e4e2da"));
    }
}

/// Mid-block sign pole (`signPole` 0.25 x 2.6 x 0.12 m): a galvanised pipe with a
/// cap and a small regulatory parking plate facing the street.
fn sign_pole(b: &mut Builder, pm: &PMats, l: usize) {
    let galv = hex("#b4b8b6");
    let s = fa(l, 10);
    turn(b, pm.galv, &[kp(0.03, 0.0), kp(0.03, 2.59), sp(0.031, 2.6), kp(0.0, 2.625)], Turn::round(s), galv, Vec3::ZERO);
    let (w, h, y, z) = (0.25f32, 0.38f32, 2.33f32, -0.04f32);
    bxc(b, pm.paint, v3(0.0, y, z), v3(w, h, 0.006), hex("#8f9293"));
    if l == 2 {
        return;
    }
    bxc(b, pm.satin, v3(0.0, y, z - 0.004), v3(w - 0.01, h - 0.01, 0.002), hex("#ecebe5"));
    bxc(b, pm.satin, v3(0.0, y + h / 2.0 - 0.05, z - 0.0055), v3(w - 0.04, 0.06, 0.002), hex("#b0262b"));
    if l == 0 {
        for k in 0..3 {
            bxc(b, pm.satin, v3(0.0, y - 0.01 - k as f32 * 0.06, z - 0.0055), v3(w - 0.08 - k as f32 * 0.03, 0.014, 0.002), hex("#202325"));
        }
        for dy in [h / 2.0 - 0.04, -h / 2.0 + 0.04] {
            turn(b, pm.metal, &[kp(0.034, y + dy - 0.01), kp(0.034, y + dy + 0.01)], Turn::round(s), shade(galv, 0.8), Vec3::ZERO);
        }
    }
}

/// Steel sidewalk bollard (`bollard` 0.19 x 0.92 x 0.18 m): black, a white reflective
/// band, a domed cap with a lip.
fn bollard(b: &mut Builder, pm: &PMats, l: usize) {
    let black = hex("#2b2e30");
    let band = hex("#e6e4dc");
    let s = fa(l, 16);
    if l == 2 {
        turn(b, pm.paint, &[kp(0.09, 0.0), kp(0.085, 0.82), kp(0.06, 0.9), kp(0.0, 0.92)], Turn::round(6), black, Vec3::ZERO);
        return;
    }
    turn(b, pm.paint, &[kp(0.094, 0.0), kp(0.094, 0.04), kp(0.085, 0.055), sp(0.083, 0.4), kp(0.081, 0.7)], Turn::round(s), black, Vec3::ZERO);
    turn(b, pm.satin, &[kp(0.0815, 0.7), kp(0.0815, 0.78)], Turn::round(s), band, Vec3::ZERO);
    turn(b, pm.paint, &[kp(0.081, 0.78), kp(0.081, 0.82), kp(0.088, 0.83), kp(0.088, 0.85), sp(0.072, 0.89), sp(0.035, 0.915), kp(0.0, 0.92)], Turn::round(s), black, Vec3::ZERO);
}

/// SF municipal litter receptacle (`wasteBin` 0.59 x 0.94 x 0.58 m): dark green, a
/// rounded-square ribbed body on a plinth, a rim band, a domed hood with its slot
/// toward the footway and the Public Works roundel.
fn waste_bin(b: &mut Builder, pm: &PMats, l: usize) {
    let green = hex("#2f5a3f");
    let t = Turn::squared(fa(l, 28), 3.2);
    turn(b, pm.paint, &[kp(0.25, 0.0), kp(0.25, 0.04)], t, shade(green, 0.65), Vec3::ZERO);
    let body = if l == 0 { Turn { flutes: 40, flute: 0.03, ..t } } else { t };
    turn(b, pm.paint, &[kp(0.27, 0.04), kp(0.27, 0.1), kp(0.276, 0.32), kp(0.284, 0.78)], body, green, Vec3::ZERO);
    turn(b, pm.paint, &[kp(0.295, 0.78), kp(0.295, 0.82)], t, shade(green, 1.1), Vec3::ZERO);
    let hood = [kp(0.29, 0.82), sp(0.28, 0.86), sp(0.23, 0.905), sp(0.13, 0.932), kp(0.0, 0.94)];
    turn(b, pm.paint, &hood, t, green, Vec3::ZERO);
    if l == 2 {
        return;
    }
    // The slot: a dark band laid on the hood's front, following its curve.
    let hood_r = |y: f32| {
        let k = hood.windows(2).find(|w| y <= w[1][1]).unwrap_or(&hood[3..5]);
        let f = ((y - k[0][1]) / (k[1][1] - k[0][1])).clamp(0.0, 1.0);
        k[0][0] + (k[1][0] - k[0][0]) * f
    };
    let n = 8;
    let base = b.v.len() as u32;
    for i in 0..=n {
        let a = std::f32::consts::FRAC_PI_2 + (i as f32 / n as f32 - 0.5) * 0.95;
        let (sn, cs) = a.sin_cos();
        for y in [0.83f32, 0.872] {
            let r = hood_r(y) * t.factor(a) + 0.003;
            b.vert([cs * r, y, sn * r], [cs, 0.3, sn], [0.0; 2], hex("#0c0f0d"), pm.matte);
        }
    }
    for i in 0..n {
        let a = base + i * 2;
        b.tri(false, a, a + 2, a + 3);
        b.tri(false, a, a + 3, a + 1);
    }
    if l == 0 {
        cyl(b, pm.satin, 0.055, 0.055, 0.004, 16, hex("#e5e3da"), [0.0, 0.55, 0.283], Some('z'));
        cyl(b, pm.satin, 0.04, 0.04, 0.006, 16, hex("#2b5f8a"), [0.0, 0.55, 0.284], Some('z'));
    }
}

/// Inverted-U bike racks (`bikeRack` 1.9 x 0.92 x 0.74 m): three galvanised staples
/// across the kerb, 0.9 m apart, each on two foot flanges.
fn bike_rack(b: &mut Builder, pm: &PMats, l: usize) {
    let galv = hex("#b4b8b6");
    let (h, half, rc, r) = (0.9f32, 0.34f32, 0.17f32, 0.03f32);
    let arc = if l == 0 { 5 } else if l == 1 { 2 } else { 1 };
    for x in [-0.9f32, 0.0, 0.9] {
        let mut pts = vec![v3(x, 0.0, -half), v3(x, h - rc, -half)];
        for k in 1..=arc {
            let a = k as f32 / arc as f32 * std::f32::consts::FRAC_PI_2;
            pts.push(v3(x, h - rc + a.sin() * rc, -half + rc - a.cos() * rc));
        }
        for k in 0..=arc {
            let a = k as f32 / arc as f32 * std::f32::consts::FRAC_PI_2;
            pts.push(v3(x, h - rc + a.cos() * rc, half - rc + a.sin() * rc));
        }
        pts.push(v3(x, 0.0, half));
        sweep(b, pm.galv, &pts, &[r], fa(l, 10), galv, false);
        if l == 0 {
            for z in [-half, half] {
                turn(b, pm.galv, &[kp(0.055, 0.0), kp(0.055, 0.008), kp(0.035, 0.014)], Turn::round(10), shade(galv, 0.85), v3(x, 0.0, z));
            }
        }
    }
}

/// A coin-box cluster (`newsBox`, 0.58 x 1.18 x 0.44 m): a blue paid-paper box on its
/// leg frames with a sloped lid and coin mechanism, beside a taller, narrow red free
/// paper box; windows show the paper behind the glass, toward the footway.
fn news_box(b: &mut Builder, pm: &PMats, l: usize) {
    let grey = hex("#4a4e50");
    let paper = hex("#d8d4c6");
    let ink = hex("#2a2b2b");
    let boxes: [(f32, f32, f32, f32, &str, bool); 2] = [(-0.16, 0.4, 0.36, 0.88, "#2c5d9f", true), (0.19, 0.29, 0.3, 1.1, "#a33228", false)];
    for (x, w, legs, top, colour, lid) in boxes {
        let c = hex(colour);
        let d = if lid { 0.4 } else { 0.34 };
        for sx in [-1.0f32, 1.0] {
            bxa(b, pm.paint, v3(x + sx * (w / 2.0 - 0.02) - 0.015, 0.0, -d / 2.0 + 0.02), v3(x + sx * (w / 2.0 - 0.02) + 0.015, legs, d / 2.0 - 0.02), grey);
        }
        bxa(b, pm.paint, v3(x - w / 2.0, legs, -d / 2.0), v3(x + w / 2.0, top, d / 2.0), c);
        if lid {
            cube(b, pm.paint, v3(x, top + 0.02, 0.0), v3(w + 0.02, 0.05, d + 0.03), Quat::from_rotation_x(0.16), c);
        } else {
            bxc(b, pm.paint, v3(x, top + 0.015, 0.0), v3(w + 0.02, 0.03, d + 0.02), shade(c, 0.85));
        }
        if l == 2 {
            continue;
        }
        let (wy0, wy1) = (legs + 0.12, top - 0.06);
        bxa(b, pm.glass, v3(x - w / 2.0 + 0.04, wy0, d / 2.0), v3(x + w / 2.0 - 0.04, wy1, d / 2.0 + 0.004), paper);
        if lid {
            bxc(b, pm.metal, v3(x, top + 0.07, d / 2.0 - 0.09), v3(0.18, 0.06, 0.12), hex("#8d9294"));
        } else {
            bxa(b, pm.satin, v3(x - w / 2.0 + 0.03, top - 0.05, d / 2.0), v3(x + w / 2.0 - 0.03, top - 0.01, d / 2.0 + 0.005), hex("#eeeae0"));
        }
        if l == 0 {
            // The masthead and headline rules seen through the glass, the handle below.
            bxa(b, pm.glass, v3(x - w / 2.0 + 0.06, wy1 - 0.06, d / 2.0 + 0.0045), v3(x + w / 2.0 - 0.06, wy1 - 0.03, d / 2.0 + 0.0055), ink);
            for k in 0..3 {
                let y = wy1 - 0.1 - k as f32 * 0.035;
                bxa(b, pm.glass, v3(x - w / 2.0 + 0.06, y, d / 2.0 + 0.0045), v3(x + w / 2.0 - 0.08 - k as f32 * 0.02, y + 0.012, d / 2.0 + 0.0055), shade(ink, 1.6));
            }
            bxc(b, pm.metal, v3(x, wy0 - 0.05, d / 2.0 + 0.015), v3(0.1, 0.025, 0.03), hex("#8d9294"));
        }
    }
}

/// USPS collection box (`mailbox`, kind height 1.2 m, 0.56 x 0.48 m): blue, on four
/// short legs, an arched top, the pull-down chute and labels toward the footway.
fn mailbox(b: &mut Builder, pm: &PMats, l: usize) {
    let blue = hex("#24508c");
    let dark = shade(blue, 0.6);
    let white = hex("#e9e8e2");
    for (x, z) in [(-0.21f32, -0.19f32), (0.21, -0.19), (-0.21, 0.19), (0.21, 0.19)] {
        bxc(b, pm.paint, v3(x, 0.065, z), v3(0.05, 0.13, 0.05), dark);
    }
    bxa(b, pm.paint, v3(-0.26, 0.12, -0.235), v3(0.26, 0.95, 0.235), blue);
    cyl(b, pm.paint, 0.26, 0.26, 0.47, fa(l, 20), blue, [0.0, 0.95, 0.0], Some('z'));
    if l == 2 {
        return;
    }
    cube(b, pm.paint, v3(0.0, 1.0, 0.246), v3(0.38, 0.14, 0.025), Quat::from_rotation_x(-0.22), shade(blue, 0.85));
    bxc(b, pm.metal, v3(0.0, 0.94, 0.258), v3(0.2, 0.022, 0.022), hex("#9aa0a2"));
    bxc(b, pm.satin, v3(0.0, 0.72, 0.237), v3(0.22, 0.12, 0.004), white);
    if l == 0 {
        bxc(b, pm.satin, v3(0.0, 0.84, 0.237), v3(0.16, 0.05, 0.004), white);
        for sx in [-1.0f32, 1.0] {
            bxc(b, pm.satin, v3(sx * 0.262, 0.62, 0.0), v3(0.004, 0.16, 0.2), white);
            bxc(b, pm.satin, v3(sx * 0.263, 0.62, 0.0), v3(0.004, 0.1, 0.12), shade(blue, 0.7));
        }
        bxa(b, pm.paint, v3(-0.262, 0.12, -0.237), v3(0.262, 0.15, 0.237), dark);
    }
}

/// Park bench (`bench` 1.09 x 0.86 x 0.6 m): two cast end frames with arms, four seat
/// slats and three back slats in wood, the seat facing the street.
fn bench(b: &mut Builder, pm: &PMats, l: usize) {
    let iron = hex("#2d3133");
    let wood = hex("#7f5b3a");
    let lean = Quat::from_rotation_x(-0.2);
    if l == 2 {
        bxa(b, pm.paint, v3(-0.54, 0.0, -0.26), v3(-0.48, 0.65, 0.24), iron);
        bxa(b, pm.paint, v3(0.48, 0.0, -0.26), v3(0.54, 0.65, 0.24), iron);
        bxa(b, pm.matte, v3(-0.54, 0.42, -0.26), v3(0.54, 0.47, 0.22), wood);
        cube(b, pm.matte, v3(0.0, 0.68, 0.24), v3(1.08, 0.3, 0.03), lean, wood);
        return;
    }
    for x in [-0.5f32, 0.5] {
        bxa(b, pm.paint, v3(x - 0.03, 0.0, -0.27), v3(x + 0.03, 0.44, -0.21), iron);
        bxa(b, pm.paint, v3(x - 0.03, 0.0, 0.16), v3(x + 0.03, 0.44, 0.22), iron);
        bxa(b, pm.paint, v3(x - 0.025, 0.4, -0.27), v3(x + 0.025, 0.44, 0.24), iron);
        cube(b, pm.paint, v3(x, 0.66, 0.235), v3(0.05, 0.46, 0.04), lean, iron);
        bxa(b, pm.paint, v3(x - 0.032, 0.62, -0.29), v3(x + 0.032, 0.655, 0.18), iron);
        bxa(b, pm.paint, v3(x - 0.02, 0.44, -0.26), v3(x + 0.02, 0.62, -0.22), iron);
        if l == 0 {
            bxa(b, pm.paint, v3(x - 0.045, 0.0, -0.285), v3(x + 0.045, 0.02, -0.195), iron);
            bxa(b, pm.paint, v3(x - 0.045, 0.0, 0.145), v3(x + 0.045, 0.02, 0.235), iron);
        }
    }
    for i in 0..4 {
        let z = -0.25 + i as f32 * 0.12;
        let tone = if i % 2 == 0 { wood } else { shade(wood, 0.9) };
        bxa(b, pm.matte, v3(-0.545, 0.44, z), v3(0.545, 0.475, z + 0.1), tone);
    }
    for i in 0..3 {
        let y = 0.56 + i as f32 * 0.105;
        let tone = if i % 2 == 0 { shade(wood, 0.95) } else { wood };
        cube(b, pm.matte, v3(0.0, y + 0.04, 0.245 + (y - 0.56) * 0.2), v3(1.09, 0.08, 0.025), lean, tone);
    }
}

/// SF transit shelter (`transitShelter` 3.6 x 2.6 x 1.29 m), open toward the street:
/// three stainless back posts with cantilever beams, a thin roof with a fascia,
/// glazed back and end panels (the glass shown by its rails, edges and the dot-frit
/// band at eye height, since the props pass has no transparency), the double-sided
/// advertising panel across the far end, a bench, the arrival-time sign.
fn transit_shelter(b: &mut Builder, pm: &PMats, l: usize) {
    let steel = hex("#b4b8b9");
    let roof = hex("#d4d8d8");
    let frame = hex("#3a3e41");
    let frit = hex("#e8ebea");
    let edge = hex("#9fb3ad");
    let (zb, yr) = (0.52f32, 2.46f32);
    for x in [-1.72f32, 0.0, 1.72] {
        bxa(b, pm.metal, v3(x - 0.04, 0.0, zb - 0.04), v3(x + 0.04, yr, zb + 0.04), steel);
        bxa(b, pm.metal, v3(x - 0.03, yr - 0.12, -0.6), v3(x + 0.03, yr, zb + 0.04), steel);
    }
    bxa(b, pm.satin, v3(-1.8, yr, -0.66), v3(1.8, yr + 0.06, 0.62), roof);
    bxa(b, pm.metal, v3(-1.8, yr - 0.02, -0.68), v3(1.8, yr + 0.1, -0.64), steel);
    // Advertising panel across the +x end.
    bxa(b, pm.paint, v3(1.72, 0.12, -0.6), v3(1.86, 2.08, 0.58), frame);
    if l == 2 {
        bxa(b, pm.metal, v3(-1.72, 0.44, zb - 0.22), v3(1.0, 0.48, zb), steel);
        return;
    }
    // The poster behind the panel's glass on both faces: a photograph (a pale sky over a
    // darker ground, a warm subject) above a white strip of type.
    for sx in [1.719f32, 1.861] {
        let n = if sx < 1.8 { -0.001 } else { 0.001 };
        let layer = |b: &mut Builder, k: f32, y0: f32, y1: f32, z0: f32, z1: f32, c: &str| {
            bxa(b, pm.glass, v3(sx + k * n - 0.0005, y0, z0), v3(sx + k * n + 0.0005, y1, z1), hex(c));
        };
        layer(b, 1.0, 0.22, 1.98, -0.52, 0.5, "#a9b8bf");
        layer(b, 2.0, 0.22, 1.05, -0.52, 0.5, "#4d5a52");
        layer(b, 3.0, 0.62, 1.55, -0.2, 0.22, "#b8743f");
        layer(b, 3.0, 0.22, 0.52, -0.52, 0.5, "#eeece6");
        if l == 0 {
            for (k, z1) in [(0usize, 0.3f32), (1, 0.1), (2, 0.2)] {
                let y = 0.44 - k as f32 * 0.07;
                layer(b, 4.0, y, y + 0.035, -0.42, z1, "#2a2d30");
            }
        }
    }
    // Back glazing between the posts and the -x end screen: rails, edges, frit band.
    let panes: [(Vec3, Vec3); 3] = [(v3(-1.68, 0.0, zb), v3(-0.04, 0.0, zb)), (v3(0.04, 0.0, zb), v3(1.68, 0.0, zb)), (v3(-1.76, 0.0, zb - 0.04), v3(-1.76, 0.0, -0.45))];
    for (a, c) in panes {
        let along = c - a;
        let mid = (a + c) * 0.5;
        let len = along.length();
        let q = Quat::from_rotation_arc(Vec3::X, along / len);
        for (y, hgt) in [(0.12f32, 0.05f32), (2.28, 0.05)] {
            cube(b, pm.metal, mid + v3(0.0, y, 0.0), v3(len, hgt, 0.04), q, steel);
        }
        if l == 0 {
            for k in 0..6 {
                cube(b, pm.satin, mid + v3(0.0, 1.3 + k as f32 * 0.03, 0.0), v3(len - 0.04, 0.011, 0.012), q, frit);
            }
            for e in [a, c] {
                bxc(b, pm.glass, e + v3(0.0, 1.2, 0.0), v3(0.012, 2.1, 0.012), edge);
            }
        }
    }
    // Bench on two brackets, the arrival-time sign hung under the roof.
    bxa(b, pm.metal, v3(-1.3, 0.44, zb - 0.32), v3(0.9, 0.47, zb - 0.02), steel);
    for x in [-1.1f32, 0.7] {
        bxa(b, pm.metal, v3(x - 0.02, 0.0, zb - 0.3), v3(x + 0.02, 0.44, zb - 0.26), steel);
    }
    bxa(b, pm.paint, v3(-1.15, yr - 0.32, -0.32), v3(-0.45, yr - 0.14, -0.24), frame);
    if l == 0 {
        bxa(b, pm.satin, v3(-1.1, yr - 0.29, -0.325), v3(-0.5, yr - 0.24, -0.32), hex("#d89a3a"));
        bxa(b, pm.metal, v3(-0.82, yr - 0.14, -0.29), v3(-0.78, yr, -0.27), steel);
        // Route map in a frame on the back glass.
        bxa(b, pm.paint, v3(-0.65, 1.0, zb - 0.03), v3(-0.15, 1.75, zb - 0.02), frame);
        bxa(b, pm.satin, v3(-0.62, 1.03, zb - 0.034), v3(-0.18, 1.72, zb - 0.03), hex("#e9e6dc"));
    }
}

/// Muni stop flag (`busStopFlag` 0.35 x 2.9 x 0.12 m): a galvanised pole with the
/// yellow stop band and the flag sign (white, a maroon head, route number tiles).
fn bus_stop_flag(b: &mut Builder, pm: &PMats, l: usize) {
    let galv = hex("#b4b8b6");
    let s = fa(l, 10);
    turn(b, pm.galv, &[kp(0.032, 0.0), kp(0.032, 2.88), kp(0.0, 2.9)], Turn::round(s), galv, Vec3::ZERO);
    let (y, h, z) = (2.52f32, 0.56f32, -0.045f32);
    bxc(b, pm.paint, v3(0.0, y, z), v3(0.34, h, 0.008), hex("#8f9293"));
    if l == 2 {
        return;
    }
    turn(b, pm.satin, &[kp(0.033, 1.62), kp(0.033, 1.92)], Turn::round(s), hex("#e2b52c"), Vec3::ZERO);
    for dz in [-0.0045f32, 0.0045] {
        bxc(b, pm.satin, v3(0.0, y, z + dz), v3(0.33, h - 0.01, 0.002), hex("#eeede6"));
        bxc(b, pm.satin, v3(0.0, y + h / 2.0 - 0.08, z + dz * 1.3), v3(0.33, 0.14, 0.002), hex("#8c1d2a"));
        if l == 0 {
            for k in 0..3 {
                bxc(b, pm.satin, v3(-0.1 + k as f32 * 0.1, y - 0.06, z + dz * 1.3), v3(0.08, 0.1, 0.002), hex("#1d1f21"));
            }
            bxc(b, pm.satin, v3(0.0, y - 0.2, z + dz * 1.3), v3(0.26, 0.04, 0.002), hex("#1d1f21"));
        }
    }
}

/// Café table and two chairs (`cafeSet` 1.72 x 0.86 x 0.74 m), black bistro metal.
fn cafe_set(b: &mut Builder, pm: &PMats, l: usize) {
    let black = hex("#272a2b");
    let s = fa(l, 16);
    turn(b, pm.paint, &[kp(0.22, 0.0), kp(0.2, 0.02), kp(0.03, 0.04), kp(0.025, 0.7), kp(0.31, 0.71), kp(0.31, 0.74), kp(0.0, 0.745)], Turn::round(s), black, Vec3::ZERO);
    for sx in [-1.0f32, 1.0] {
        let cx = sx * 0.62;
        bxa(b, pm.paint, v3(cx - 0.2, 0.44, -0.2), v3(cx + 0.2, 0.465, 0.2), black);
        let back = cx + sx * 0.19;
        if l == 2 {
            bxa(b, pm.paint, v3(cx - 0.18, 0.0, -0.18), v3(cx + 0.18, 0.44, 0.18), black);
            bxa(b, pm.paint, v3(back - 0.02, 0.44, -0.19), v3(back + 0.02, 0.86, 0.19), black);
            continue;
        }
        for (x, z) in [(-0.17f32, -0.17f32), (0.17, -0.17), (-0.17, 0.17), (0.17, 0.17)] {
            sweep(b, pm.paint, &[v3(cx + x * 1.1, 0.0, z * 1.1), v3(cx + x, 0.44, z)], &[0.011], fa(l, 6), black, false);
        }
        for z in [-0.17f32, 0.17] {
            sweep(b, pm.paint, &[v3(back, 0.46, z), v3(back + sx * 0.04, 0.86, z * 0.95)], &[0.011], fa(l, 6), black, false);
        }
        if l == 0 {
            for k in 0..3 {
                let y = 0.6 + k as f32 * 0.09;
                let t = (y - 0.46) / 0.4;
                bxc(b, pm.paint, v3(back + sx * 0.04 * t, y, 0.0), v3(0.015, 0.035, 0.34), black);
            }
        } else {
            bxc(b, pm.paint, v3(back + sx * 0.03, 0.72, 0.0), v3(0.015, 0.24, 0.34), black);
        }
    }
}

/// Sandwich board (`sandwichBoard` 0.5 x 1.0 x 0.61 m): an A-frame of two chalkboards
/// in wooden frames, chalk lines on both faces.
fn sandwich_board(b: &mut Builder, pm: &PMats, l: usize) {
    let wood = hex("#6e5233");
    let board = hex("#232725");
    let chalk = hex("#d9d6cc");
    let a = 0.3f32;
    // Each board hangs from the hinge, turned out by `a`, its feet on the ground.
    let hinge = v3(0.0, 1.02 * a.cos() + 0.004, 0.0);
    for s in [-1.0f32, 1.0] {
        let q = Quat::from_rotation_x(s * a);
        let c = hinge + q * v3(0.0, -0.51, 0.0);
        let out = q * v3(0.0, 0.0, -s * 0.012);
        cube(b, pm.matte, c, v3(0.5, 1.02, 0.022), q, wood);
        if l == 2 {
            continue;
        }
        cube(b, pm.matte, c + out, v3(0.42, 0.86, 0.004), q, board);
        if l == 0 {
            for (k, (w, y)) in [(0.3f32, 0.3f32), (0.24, 0.17), (0.28, 0.06), (0.2, -0.06), (0.26, -0.17)].into_iter().enumerate() {
                let col = if k == 0 { hex("#d9a441") } else { chalk };
                cube(b, pm.matte, c + out * 1.25 + q * v3(0.0, y, 0.0), v3(w, if k == 0 { 0.05 } else { 0.022 }, 0.002), q, col);
            }
        }
    }
    if l < 2 {
        // The hinge bar across the top.
        cyl(b, pm.metal, 0.012, 0.012, 0.5, 6, hex("#55595b"), [0.0, 1.02 * a.cos(), 0.0], Some('x'));
    }
}

/// Utility cabinet (`utilityBox` 0.92 x 1.12 x 0.43 m): grey, on a concrete pad, a
/// lid overhang, two doors with handles toward the street, louvres on both ends.
fn utility_box(b: &mut Builder, pm: &PMats, l: usize) {
    let shell = hex(pal("cabinetShell"));
    let trim = hex(pal("cabinetTrim"));
    bxa(b, pm.matte, v3(-0.49, 0.0, -0.25), v3(0.49, 0.06, 0.25), hex(pal("cabinetPlinth")));
    bxa(b, pm.paint, v3(-0.45, 0.06, -0.21), v3(0.45, 1.08, 0.21), shell);
    bxa(b, pm.paint, v3(-0.47, 1.08, -0.23), v3(0.47, 1.12, 0.23), shade(shell, 0.95));
    if l == 2 {
        return;
    }
    bxa(b, pm.paint, v3(-0.43, 0.1, -0.214), v3(-0.006, 1.04, -0.21), hex(pal("cabinetDoor")));
    bxa(b, pm.paint, v3(0.006, 0.1, -0.214), v3(0.43, 1.04, -0.21), hex(pal("cabinetDoor")));
    for x in [-0.06f32, 0.06] {
        bxc(b, pm.metal, v3(x, 0.62, -0.222), v3(0.025, 0.14, 0.02), trim);
    }
    if l == 0 {
        for sx in [-1.0f32, 1.0] {
            for k in 0..6 {
                bxc(b, pm.paint, v3(sx * 0.452, 0.72 + k as f32 * 0.035, 0.0), v3(0.008, 0.012, 0.26), trim);
            }
        }
        bxc(b, pm.satin, v3(0.22, 0.86, -0.2155), v3(0.12, 0.08, 0.002), hex("#e8e6dd"));
        bxc(b, pm.satin, v3(-0.25, 0.4, -0.2155), v3(0.09, 0.05, 0.002), hex("#d8b23a"));
    }
}

/// A galvanised tapered street-light pole to `top`: a flared anchor-base cover, the
/// shaft, a cap; the handhole and the pole tag near.
fn light_pole(b: &mut Builder, pm: &PMats, l: usize, top: f32) {
    let galv = hex("#b4b8b6");
    let s = fa(l, 16);
    turn(b, pm.galv, &[kp(0.18, 0.0), kp(0.18, 0.03), sp(0.14, 0.08), sp(0.122, 0.2), kp(0.118, 0.42), kp(0.106, 0.45), kp(0.068, top), sp(0.06, top + 0.03), kp(0.0, top + 0.05)], Turn::round(s), galv, Vec3::ZERO);
    if l == 0 {
        bxc(b, pm.galv, v3(0.0, 0.72, 0.112), v3(0.07, 0.16, 0.012), shade(galv, 0.85));
        bxc(b, pm.satin, v3(0.0, 2.6, -0.094), v3(0.05, 0.12, 0.004), hex("#d8d6cf"));
    }
}

/// A flat-bottomed LED cobra head along -z with its lens centre at `at` (the light's
/// position): a teardrop shell, the neck round the arm's slip fitter, the lens.
fn cobra_head(b: &mut Builder, pm: &PMats, l: usize, at: Vec3) {
    let shell = hex("#8e9396");
    let st: Vec<[f32; 4]> = if l == 2 {
        vec![[0.42, 0.14, 0.0, 0.16], [-0.38, 0.12, 0.0, 0.12]]
    } else {
        vec![[0.42, 0.06, 0.05, 0.13], [0.32, 0.13, 0.015, 0.17], [0.12, 0.18, -0.005, 0.18], [-0.12, 0.19, -0.005, 0.17], [-0.3, 0.15, 0.0, 0.125], [-0.38, 0.08, 0.012, 0.07]]
    };
    loft(b, pm.paint, &st, if l == 2 { 6 } else { fa(l, 16) }, 3.0, true, shell, at);
    if l < 2 {
        bxc(b, pm.glass, at + v3(0.0, -0.007, -0.04), v3(0.26, 0.006, 0.46), hex("#d9d7cc"));
    }
}

/// The street-light arm from the pole at `y0` to the head's neck at (`y1`, `-reach`): an
/// upswept tube rising off the pole and levelling out, with a brace near.
fn light_arm(b: &mut Builder, pm: &PMats, l: usize, y0: f32, y1: f32, reach: f32) {
    let galv = hex("#b4b8b6");
    let k = if l == 0 { 10 } else if l == 1 { 5 } else { 2 };
    let pts = bez([v3(0.0, y0, -0.02), v3(0.0, y0 + 0.4, -0.55 * reach / 2.9), v3(0.0, y1, -1.6 * reach / 2.9), v3(0.0, y1, -reach)], k);
    sweep(b, pm.galv, &pts, &[0.045, 0.042, 0.039, 0.036, 0.034, 0.032, 0.031, 0.03, 0.03, 0.03, 0.03], fa(l, 10), galv, true);
    turn(b, pm.galv, &[kp(0.085, y0 - 0.16), kp(0.085, y0 + 0.12)], Turn::round(fa(l, 14)), shade(galv, 0.9), Vec3::ZERO);
    if l == 0 {
        sweep(b, pm.galv, &bez([v3(0.0, y0 - 0.55, -0.07), v3(0.0, y0 - 0.25, -0.3), v3(0.0, y0 + 0.05, -0.55), v3(0.0, y0 + 0.3, -0.8)], 6), &[0.016], 6, galv, false);
    }
}

/// District cobra-head light (`lampCobra` 0.88 x 7.62 x 3.51 m): galvanised pole,
/// upswept arm, LED cobra head whose lens is at the light (`LAMP_HEAD.district`).
fn lamp_cobra(b: &mut Builder, pm: &PMats, cx: &StandInCtx, l: usize) {
    let [h, reach] = cx.district;
    light_pole(b, pm, l, h - 0.3);
    light_arm(b, pm, l, h - 0.55, h + 0.1, reach - 0.38);
    cobra_head(b, pm, l, v3(0.0, h, -reach));
}

/// Banner light (`lampBanner` 1.34 x 7.62 x 3.51 m): the cobra light with two banner
/// arms and a pair of fabric banners either side of the pole (the overhead's banner colours).
fn lamp_banner(b: &mut Builder, pm: &PMats, cx: &StandInCtx, l: usize) {
    lamp_cobra(b, pm, cx, l);
    let field = lin3([0.36, 0.07, 0.19]);
    let ink = lin3([0.58, 0.46, 0.36]);
    let galv = hex("#b4b8b6");
    let (y0, y1) = (4.15f32, 5.75f32);
    for sx in [-1.0f32, 1.0] {
        for y in [y0, y1] {
            sweep(b, pm.galv, &[v3(sx * 0.05, y, 0.0), v3(sx * 0.68, y, 0.0)], &[0.014], fa(l, 6), galv, true);
        }
        // The banner, held flat between its arms.
        let (xa, xb, ya, yb) = (sx * 0.1, sx * 0.65, y0 + 0.04, y1 - 0.04);
        quad(b, pm.matte, [v3(xa, ya, 0.0), v3(xb, ya, 0.0), v3(xb, yb, 0.0), v3(xa, yb, 0.0)], Vec3::Z, field);
        // The pale band across the middle, and (near) a smaller block of ink under it.
        let ym = (y0 + y1) * 0.5;
        bxa(b, pm.matte, v3(xa.min(xb), ym + 0.05, -0.004), v3(xa.max(xb), ym + 0.24, 0.004), ink);
        if l == 0 {
            bxa(b, pm.matte, v3(xa.min(xb) + 0.1, ym - 0.2, -0.005), v3(xa.max(xb) - 0.1, ym - 0.12, 0.005), ink);
        }
    }
}

/// Historic ornamental light (`lampHistoric`, the Market Street family): an
/// octagonal plinth, a fluted bell base with rings, a fluted cast-iron shaft, a
/// collar and a six-sided lantern with its glass round the light (`LAMP_HEAD.column`),
/// a roof and a finial. Painted dark green-black.
fn lamp_historic(b: &mut Builder, pm: &PMats, cx: &StandInCtx, l: usize) {
    let iron = hex("#26332d");
    let glass = hex("#efe4c8");
    let hc = cx.column;
    let s = fa(l, 24);
    turn(b, pm.paint, &[kp(0.25, 0.0), kp(0.25, 0.1), kp(0.21, 0.14)], Turn::facets(8), iron, Vec3::ZERO);
    let bell = if l == 0 { Turn::fluted(s, 16, 0.07) } else { Turn::round(s) };
    turn(b, pm.paint, &[kp(0.195, 0.14), sp(0.188, 0.3), kp(0.16, 0.6), kp(0.178, 0.62), kp(0.178, 0.69), kp(0.152, 0.71), sp(0.122, 0.98), kp(0.096, 1.24)], bell, iron, Vec3::ZERO);
    turn(b, pm.paint, &[kp(0.108, 1.24), sp(0.112, 1.275), kp(0.108, 1.31)], Turn::round(s), iron, Vec3::ZERO);
    let shaft = if l == 0 { Turn::fluted(s, 12, 0.1) } else { Turn::round(fa(l, 12)) };
    turn(b, pm.paint, &[kp(0.082, 1.31), kp(0.066, hc - 0.72)], shaft, iron, Vec3::ZERO);
    turn(b, pm.paint, &[kp(0.082, hc - 0.72), kp(0.082, hc - 0.66), sp(0.105, hc - 0.6), kp(0.13, hc - 0.52), kp(0.13, hc - 0.46), kp(0.09, hc - 0.44)], Turn::round(s), iron, Vec3::ZERO);
    let six = Turn::facets(6);
    turn(b, pm.paint, &[kp(0.09, hc - 0.44), kp(0.17, hc - 0.32), kp(0.17, hc - 0.28)], six, iron, Vec3::ZERO);
    turn(b, pm.glass, &[kp(0.155, hc - 0.28), kp(0.212, hc + 0.26)], six, glass, Vec3::ZERO);
    turn(b, pm.paint, &[kp(0.25, hc + 0.26), kp(0.25, hc + 0.3), kp(0.08, hc + 0.47), kp(0.05, hc + 0.5)], six, iron, Vec3::ZERO);
    turn(b, pm.paint, &[kp(0.05, hc + 0.5), sp(0.048, hc + 0.56), sp(0.032, hc + 0.6), kp(0.012, hc + 0.66), kp(0.0, hc + 0.8)], Turn::round(fa(l, 10)), iron, Vec3::ZERO);
    if l < 2 {
        for k in 0..6 {
            let a = (k as f32 + 0.5) / 6.0 * std::f32::consts::TAU + std::f32::consts::PI / 6.0;
            let (sn, cs) = a.sin_cos();
            sweep(b, pm.paint, &[v3(cs * 0.16, hc - 0.28, sn * 0.16), v3(cs * 0.218, hc + 0.26, sn * 0.218)], &[0.012], 4, iron, false);
        }
    }
}

/// Wood distribution pole (overhead `utilityPole` 2.44 x 11.73 x 1.25 m): the shaft,
/// two crossarms on the -z face with pin insulators at the wire seats and braces,
/// a pole-top transformer can on the +z face above the hardware floor, step bolts.
fn dist_pole(b: &mut Builder, pm: &PMats, cx: &StandInCtx, l: usize) {
    let wood = hex("#5e4c3a");
    let arm = hex("#6a5640");
    let porcelain = hex("#c9c7be");
    let steel = hex("#868b8c");
    let [h, hw, arm_len] = cx.pole;
    let r_at = |y: f32| 0.19 + (0.13 - 0.19) * ((y - 0.3) / (h - 0.3)).clamp(0.0, 1.0);
    turn(b, pm.matte, &[kp(0.2, 0.0), kp(0.19, 0.3), kp(0.13, h - 0.02), kp(0.1, h + 0.02)], Turn::round(fa(l, 14)), wood, Vec3::ZERO);
    for a in &cx.arms {
        let top = a[2] - 0.16;
        let r = r_at(top);
        let (z0, z1) = (-(r + 0.095), -r + 0.005);
        bxa(b, pm.matte, v3(-arm_len / 2.0, top - 0.11, z0), v3(arm_len / 2.0, top, z1), arm);
        let zc = (z0 + z1) * 0.5;
        if l < 2 {
            for x in [-0.6f32, 0.6] {
                let p0 = v3(x, top - 0.11, zc);
                let p1 = v3(0.0, top - 0.72, -r_at(top - 0.72) - 0.01);
                let d = p1 - p0;
                cube(b, pm.metal, (p0 + p1) * 0.5, v3(0.04, d.length(), 0.008), Quat::from_rotation_arc(Vec3::Y, d.normalize()), steel);
            }
        }
        for x in [a[0], a[1]] {
            if l == 0 {
                turn(b, pm.glass, &[kp(0.014, top), kp(0.014, top + 0.03), kp(0.055, top + 0.05), kp(0.048, top + 0.075), kp(0.058, top + 0.1), kp(0.045, top + 0.125), kp(0.04, top + 0.15), kp(0.0, a[2])], Turn::round(10), porcelain, v3(x, 0.0, zc));
            } else if l == 1 {
                turn(b, pm.glass, &[kp(0.05, top), kp(0.045, a[2])], Turn::round(6), porcelain, v3(x, 0.0, zc));
            }
        }
    }
    // Transformer can: body, lid, two bushings, the hanger bands.
    let (y0, y1) = (hw + 0.1, hw + 0.95);
    let rz = r_at(y1) + 0.27;
    let tank = hex("#8f9593");
    turn(b, pm.paint, &[kp(0.24, y0), kp(0.24, y1), sp(0.2, y1 + 0.06), kp(0.0, y1 + 0.08)], Turn::round(fa(l, 16)), tank, v3(0.0, 0.0, rz));
    if l < 2 {
        bxa(b, pm.metal, v3(-0.07, y0 + 0.15, r_at(y0) - 0.02), v3(0.07, y1 - 0.15, rz - 0.22), steel);
        for x in [-0.09f32, 0.09] {
            turn(b, pm.glass, &[kp(0.025, y1 + 0.04), kp(0.025, y1 + 0.2), kp(0.0, y1 + 0.22)], Turn::round(6), shade(porcelain, 0.55), v3(x, 0.0, rz));
        }
    }
    if l == 0 {
        for k in 0..10 {
            let y = 2.4 + k as f32 * 0.6;
            if y > hw - 0.2 {
                break;
            }
            let a = if k % 2 == 0 { 0.5f32 } else { 0.5 + std::f32::consts::PI };
            let d = v3(a.cos(), 0.0, a.sin());
            sweep(b, pm.metal, &[d * (r_at(y) - 0.02) + Vec3::Y * y, d * (r_at(y) + 0.17) + Vec3::Y * y, d * (r_at(y) + 0.17) + Vec3::Y * (y + 0.04)], &[0.009], 5, steel, false);
        }
        bxc(b, pm.satin, v3(0.0, 2.0, -r_at(2.0) - 0.003), v3(0.08, 0.06, 0.006), hex("#c9cccd"));
    }
}

/// The cobra-head luminaire a distribution pole carries (overhead `cobraArm` 1.83 x
/// 0.66 x 0.44 m, along +x from the pole face): a mounting plate, the curved arm, a
/// brace, the head at the arm's end.
fn cobra_arm(b: &mut Builder, pm: &PMats, l: usize) {
    let galv = hex("#b4b8b6");
    bxa(b, pm.galv, v3(-0.1, -0.16, -0.07), v3(0.02, 0.16, 0.07), shade(galv, 0.9));
    let k = if l == 0 { 8 } else if l == 1 { 4 } else { 2 };
    sweep(b, pm.galv, &bez([v3(0.0, 0.02, 0.0), v3(0.5, 0.06, 0.0), v3(0.95, 0.3, 0.0), v3(1.22, 0.33, 0.0)], k), &[0.032], fa(l, 10), galv, true);
    if l < 2 {
        sweep(b, pm.galv, &[v3(0.0, -0.13, 0.0), v3(0.62, 0.08, 0.0)], &[0.014], 6, galv, false);
    }
    let from = b.v.len();
    cobra_head(b, pm, l, Vec3::ZERO);
    // The head along -z with its neck at z 0.42: turned onto +x, the neck at the arm's end.
    xform(b, from, Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2), v3(1.22 + 0.42, 0.25, 0.0));
}

/// Chinatown lantern (overhead `lantern` 0.45 x 0.55 x 0.46 m, foot at its tassel): a
/// ribbed red silk body between gold caps, the tassel below, the hook above.
fn lantern(b: &mut Builder, pm: &PMats, l: usize) {
    let red = hex("#b3221c");
    let gold = hex("#c49a3c");
    let s = fa(l, 16);
    turn(b, pm.matte, &[kp(0.0, 0.0), kp(0.032, 0.02), kp(0.024, 0.12), kp(0.012, 0.14)], Turn::round(fa(l, 8)), hex("#8f1a16"), Vec3::ZERO);
    turn(b, pm.metal, &[kp(0.012, 0.14), kp(0.095, 0.15), kp(0.095, 0.172)], Turn::round(s), gold, Vec3::ZERO);
    let rows = if l == 0 { 8 } else { 4 };
    let prof: Vec<[f32; 3]> = (0..=rows)
        .map(|i| {
            let t = i as f32 / rows as f32;
            let a = (t - 0.5) * std::f32::consts::PI * 0.86;
            sp(0.095 + (0.225 - 0.095) * a.cos().powf(1.4) / 1.0, 0.172 + t * 0.3)
        })
        .collect();
    let body = if l == 0 { Turn::fluted(s, 16, 0.06) } else { Turn::round(s) };
    turn(b, pm.matte, &prof, body, red, Vec3::ZERO);
    turn(b, pm.metal, &[kp(0.095, 0.472), kp(0.095, 0.494), kp(0.025, 0.505)], Turn::round(s), gold, Vec3::ZERO);
    if l < 2 {
        turn(b, pm.metal, &[kp(0.008, 0.5), kp(0.008, 0.55)], Turn::round(4), shade(gold, 0.6), Vec3::ZERO);
    }
}

/// Canary Island date palm (`palm` 8.73 x 10 x 8.31 m, the Dolores Street and
/// Embarcadero palm): a thick ringed trunk of leaf bases, the "pineapple" boss under
/// the crown, a dense crown of long arching pinnate fronds (geometry, no alpha:
/// each a folded ribbon whose edge is cut into leaflet teeth near), the oldest
/// fronds yellowing and the dead ones hanging. Fronds sway with the wind.
fn palm(b: &mut Builder, pm: &PMats, l: usize) {
    let trunk = hex("#7a6a55");
    let boss = hex("#8b7650");
    let greens = [hex("#4b6a31"), hex("#57763a"), hex("#43602c")];
    let old = hex("#7f7a3c");
    let dead = hex("#776045");
    let top = 7.7f32;
    // Trunk: bands of leaf-base ridges (near), a plain taper further out.
    if l == 0 {
        let bands = 14;
        for k in 0..bands {
            let y0 = 0.25 + k as f32 * (top - 0.6) / bands as f32;
            let y1 = y0 + (top - 0.6) / bands as f32;
            let r = 0.47 - 0.05 * (k as f32 / bands as f32) + if k > bands - 4 { 0.035 * (k - (bands - 4)) as f32 } else { 0.0 };
            let c = shade(trunk, if k % 2 == 0 { 1.0 } else { 0.86 });
            turn(b, pm.matte, &[kp(r * 0.97, y0), kp(r * 1.05, y0 + (y1 - y0) * 0.75), kp(r * 0.97, y1)], Turn { open: true, ..Turn::round(10) }, c, Vec3::ZERO);
        }
        turn(b, pm.matte, &[kp(0.62, 0.0), sp(0.52, 0.12), kp(0.46, 0.25)], Turn::round(12), shade(trunk, 0.8), Vec3::ZERO);
    } else {
        turn(b, pm.matte, &[kp(0.58, 0.0), kp(0.47, 0.3), kp(0.43, 4.0), kp(0.5, top - 0.35)], Turn::round(fa(l, 12)), trunk, Vec3::ZERO);
    }
    let bt = if l == 0 { Turn::fluted(14, 9, 0.16) } else { Turn::round(fa(l, 12)) };
    turn(b, pm.matte, &[kp(0.5, top - 0.4), sp(0.68, top - 0.05), sp(0.62, top + 0.35), kp(0.34, top + 0.6)], bt, boss, Vec3::ZERO);
    // Fronds: (count, segments, teeth).
    let (n, segs, teeth) = match l {
        0 => (36u32, 9u32, true),
        1 => (20, 5, false),
        _ => (12, 3, false),
    };
    let golden = std::f32::consts::PI * (3.0 - 5.0f32.sqrt());
    for i in 0..n {
        let f = i as f32 / n as f32;
        let phi = i as f32 * golden;
        // Elevation from steep (young, top) to drooping (old) to hanging (dead).
        let (elev, len, droop, col) = if f < 0.22 {
            (1.15 - f * 1.6, 3.7, 0.26, greens[(i % 3) as usize])
        } else if f < 0.84 {
            (0.75 - (f - 0.22) * 1.15, 4.6, 0.55, greens[(i % 3) as usize])
        } else if f < 0.94 {
            (-0.2, 4.2, 0.45, old)
        } else {
            (-1.2, 2.8, 0.12, dead)
        };
        let len = len * (0.92 + 0.16 * ((i * 7 % 5) as f32 / 4.0));
        let d0 = v3(elev.cos() * phi.cos(), elev.sin(), elev.cos() * phi.sin());
        let root = v3(phi.cos() * 0.32, top + 0.4 - f * 0.8, phi.sin() * 0.32);
        let fold = 0.38f32;
        // The far levels' fewer fronds are wider, so the crown keeps its mass.
        let wide = if l == 0 { 1.0 } else { 1.0 + 0.35 * l as f32 };
        let base = b.v.len() as u32;
        for k in 0..=segs {
            let t = k as f32 / segs as f32;
            let p = root + d0 * (len * t) - Vec3::Y * (droop * len * t * t);
            let tan = (d0 * len - Vec3::Y * (2.0 * droop * len * t)).normalize();
            let side = tan.cross(Vec3::Y).normalize_or(Vec3::X);
            let up = side.cross(tan);
            let mut w = 0.52 * wide * (std::f32::consts::PI * (t * 1.08).min(1.0)).sin().max(0.0).powf(0.6) * (1.0 - 0.3 * t);
            if t < 0.1 {
                w *= t / 0.1;
            }
            if teeth && k % 2 == 1 {
                w *= 0.68;
            }
            let shade_k = 0.85 + 0.25 * t;
            let c = shade(col, shade_k);
            let sway = (0.35 * t).to_bits();
            let nl = (up * fold.cos() - side * fold.sin()).normalize();
            let nr = (up * fold.cos() + side * fold.sin()).normalize();
            for (pos, nrm) in [(p, up), (p + side * (w * fold.cos()) + up * (w * fold.sin()), nl), (p - side * (w * fold.cos()) + up * (w * fold.sin()), nr)] {
                let id = b.vert(pos.to_array(), nrm.to_array(), [t, 0.0], c, pm.matte);
                b.v[id as usize].pad = sway;
            }
        }
        for k in 0..segs {
            let (a, c) = (base + k * 3, base + (k + 1) * 3);
            b.tri(false, a, a + 1, c + 1);
            b.tri(false, a, c + 1, c);
            b.tri(false, a, c + 2, a + 2);
            b.tri(false, a, c, c + 2);
        }
    }
}

/// Builds level `l` of the stand-in for `name` into `b`; false for a name with none.
fn stand_in_level(name: &str, b: &mut Builder, pm: &PMats, cx: &StandInCtx, l: usize) -> bool {
    match name {
        "hydrant" => hydrant(b, pm, l),
        "parkingMeter" => parking_meter(b, pm, l),
        "payStation" => pay_station(b, pm, l),
        "signPole" => sign_pole(b, pm, l),
        "bollard" => bollard(b, pm, l),
        "wasteBin" => waste_bin(b, pm, l),
        "bikeRack" => bike_rack(b, pm, l),
        "newsBox" => news_box(b, pm, l),
        "mailbox" => mailbox(b, pm, l),
        "lampHistoric" => lamp_historic(b, pm, cx, l),
        "lampBanner" => lamp_banner(b, pm, cx, l),
        "lampCobra" => lamp_cobra(b, pm, cx, l),
        "bench" => bench(b, pm, l),
        "transitShelter" => transit_shelter(b, pm, l),
        "busStopFlag" => bus_stop_flag(b, pm, l),
        "cafeSet" => cafe_set(b, pm, l),
        "sandwichBoard" => sandwich_board(b, pm, l),
        "utilityBox" => utility_box(b, pm, l),
        "utilityPole" => dist_pole(b, pm, cx, l),
        "cobraArm" => cobra_arm(b, pm, l),
        "lantern" => lantern(b, pm, l),
        "palm" => palm(b, pm, l),
        _ => return false,
    }
    true
}

/// How strongly a kind takes the road dirt (0: it hangs overhead).
fn dirt_of(name: &str) -> f32 {
    match name {
        "cobraArm" | "lantern" => 0.0,
        "palm" | "utilityPole" => 0.35,
        _ => 0.55,
    }
}

/// The stand-in for `name` as model `id` (three levels, `lod` edges, `cast` cascades),
/// as the GLB would have been set. False (nothing added) for a kind with no stand-in.
fn stand_in(out: &mut Out, pm: &PMats, cx: &StandInCtx, name: &str, id: u32, lod: [f32; 4], cast: u32) -> bool {
    let mut ids = [NONE; 3];
    for (l, slot) in ids.iter_mut().enumerate() {
        if !stand_in_level(name, &mut out.b, pm, cx, l) {
            out.b = Builder::default();
            return false;
        }
        let k = dirt_of(name);
        if k > 0.0 {
            dirt(&mut out.b, 0, k);
        }
        mottle(&mut out.b, 0, 0.08);
        *slot = out.finish();
    }
    out.set(id, ids, lod, cast, 0.0);
    out.sources.push(format!("{name}: procedural stand-in (its GLB is not in this checkout)"));
    true
}

/// The stand-ins' materials and context, made on the first stand-in a cook needs.
fn stand_kit(stand: &mut Option<(PMats, StandInCtx)>, out: &mut Out, js: &JsProps, x: &LibExtras, plain: [u32; 3]) -> (PMats, StandInCtx) {
    stand.get_or_insert_with(|| (PMats::push(out.materials, plain[0], plain[1], plain[2]), StandInCtx::load(js, x.overhead))).clone()
}

/// Whether `name`'s GLB failed to load (`props_lib` records it in `missing`).
fn glb_absent(out: &Out, name: &str) -> bool {
    let tag = format!("{name}: ");
    out.missing.iter().any(|m| m.starts_with(&tag))
}

/// The vehicle signal head without its GLB: the props library's own lens discs (lit by the
/// lamp channel, sized from `signal-lens-size`) kept as they are, the box round them
/// replaced by a black three-section housing with doors, lens rims, tunnel visors open
/// underneath and the top hanger (`headHalfHeightM` the housing's half height).
fn signal_head_stand_in(out: &mut Out, pm: &PMats, cx: &StandInCtx) {
    let old = out.models[Model::SignalHead as usize];
    let black = hex("#1d2022");
    let mut ids = [NONE; 3];
    for (l, slot) in ids.iter_mut().enumerate().take(2) {
        let src = old.meshes[l.min(1)];
        if src == NONE {
            return;
        }
        let g = out.meshes[src as usize];
        // Copy the lens triangles and find each lens's centre and radius.
        let mut lens: Vec<(u32, Vec3, f32)> = vec![];
        let tris: Vec<u32> = out.i[g.first as usize..(g.first + g.count) as usize].to_vec();
        for t in tris.chunks_exact(3) {
            let vs: Vec<PropVertex> = t.iter().map(|&k| out.v[(g.base_vertex + k as i32) as usize]).collect();
            let mat = vs[0].material;
            if out.materials.get(mat as usize).is_none_or(|m| m.flags & P_LENS == 0) {
                continue;
            }
            let base = out.b.v.len() as u32;
            for v in &vs {
                out.b.v.push(*v);
            }
            out.b.tri(false, base, base + 1, base + 2);
            if !lens.iter().any(|e| e.0 == mat) {
                lens.push((mat, Vec3::from(vs[0].pos), 0.0));
            }
        }
        // The fan centre is each disc's first vertex; its radius the farthest vertex.
        for e in &mut lens {
            for v in &out.b.v {
                if v.material == e.0 {
                    e.2 = e.2.max(Vec3::from(v.pos).distance(e.1));
                }
            }
        }
        if lens.is_empty() {
            return;
        }
        let face = lens[0].1.z;
        let hh = cx.head_half;
        let b = &mut out.b;
        let (w, back) = (0.21f32, -0.14f32);
        if l == 0 {
            for e in &lens {
                let y = e.1.y;
                let sec = (2.0 * hh) / 3.0;
                bxa(b, pm.paint, v3(-w, y - sec / 2.0 + 0.004, back), v3(w, y + sec / 2.0 - 0.004, face - 0.022), black);
                bxa(b, pm.paint, v3(-w + 0.012, y - sec / 2.0 + 0.016, face - 0.022), v3(w - 0.012, y + sec / 2.0 - 0.016, face - 0.004), shade(black, 1.15));
                torus(b, pm.paint, e.2 + 0.008, 0.01, 20, 4, black, [0.0, y, face + 0.002], Some('z'));
                // Tunnel visor: a three-quarter tube open underneath.
                let r = e.2 + 0.03;
                let (n, deep) = (14u32, 0.24f32);
                let vb = b.v.len() as u32;
                for k in 0..=n {
                    let a = -0.35 + (std::f32::consts::PI + 0.7) * k as f32 / n as f32;
                    let (sn, cs) = a.sin_cos();
                    for z in [face - 0.004, face + deep] {
                        b.vert([cs * r, y + sn * r, z], [cs, sn, 0.0], [0.0; 2], black, pm.paint);
                    }
                }
                for k in 0..n {
                    let a = vb + k * 2;
                    b.tri(false, a, a + 2, a + 3);
                    b.tri(false, a, a + 3, a + 1);
                }
            }
            turn(b, pm.paint, &[kp(0.032, hh - 0.01), kp(0.032, hh + 0.13)], Turn::round(10), black, Vec3::ZERO);
            bxa(b, pm.paint, v3(-0.08, hh - 0.01, -0.06), v3(0.08, hh + 0.02, 0.06), black);
            bxa(b, pm.paint, v3(-0.05, -hh - 0.02, -0.05), v3(0.05, -hh, 0.05), black);
        } else {
            bxa(b, pm.paint, v3(-w, -hh, back), v3(w, hh, face - 0.004), black);
            for e in &lens {
                bxa(b, pm.paint, v3(-e.2 - 0.03, e.1.y + e.2 + 0.01, face), v3(e.2 + 0.03, e.1.y + e.2 + 0.03, face + 0.22), black);
            }
            bxa(b, pm.paint, v3(-0.03, hh, -0.03), v3(0.03, hh + 0.13, 0.03), black);
        }
        *slot = out.finish();
    }
    ids[2] = ids[1];
    out.set(Model::SignalHead as u32, ids, old.lod, old.cast[0], 0.0);
    out.sources.push("signalHead: procedural housing round the lens discs (its GLB is not in this checkout)".into());
}

#[cfg(test)]
mod stand_in_tests {
    use super::*;

    const KINDS: [&str; 22] = [
        "hydrant", "parkingMeter", "payStation", "signPole", "bollard", "wasteBin", "bikeRack", "newsBox", "mailbox", "lampHistoric", "lampBanner", "lampCobra", "bench", "transitShelter", "busStopFlag", "cafeSet", "sandwichBoard", "utilityBox", "utilityPole", "cobraArm", "lantern", "palm",
    ];

    fn ctx() -> (JsProps, StandInCtx) {
        let js = JsProps::load().unwrap();
        let oh = eo_props::overhead::OverheadJs::load().unwrap();
        let cx = StandInCtx::load(&js, Some(&oh));
        (js, cx)
    }

    fn pm() -> PMats {
        PMats { paint: 0, metal: 1, matte: 2, galv: 3, glass: 4, satin: 5 }
    }

    fn level(name: &str, cx: &StandInCtx, l: usize) -> Builder {
        let mut b = Builder::default();
        assert!(stand_in_level(name, &mut b, &pm(), cx, l), "{name} has a stand-in");
        dirt(&mut b, 0, dirt_of(name));
        b
    }

    fn bounds(b: &Builder) -> (Vec3, Vec3) {
        b.v.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(mn, mx), v| (mn.min(Vec3::from(v.pos)), mx.max(Vec3::from(v.pos))))
    }

    /// Every stand-in has three levels, each lighter than the one before, a near level in
    /// the brief's budget, finite geometry, unit normals and only indices it owns.
    #[test]
    fn stand_ins_have_three_lighter_levels() {
        let (_, cx) = ctx();
        let mut table = String::new();
        for name in KINDS {
            let tris: Vec<usize> = (0..3)
                .map(|l| {
                    let b = level(name, &cx, l);
                    assert!(!b.opaque.is_empty(), "{name} level {l} is not empty");
                    assert!(b.opaque.iter().chain(b.cut.iter()).all(|&k| (k as usize) < b.v.len()), "{name} level {l}: indices in range");
                    for v in &b.v {
                        assert!(v.pos.iter().all(|c| c.is_finite()), "{name} level {l}: finite positions");
                        let n = (v.normal[0] as f32).hypot(v.normal[1] as f32).hypot(v.normal[2] as f32);
                        assert!(n > 100.0, "{name} level {l}: a normal of length {n}");
                    }
                    (b.opaque.len() + b.cut.len()) / 3
                })
                .collect();
            assert!(tris[0] <= 2600, "{name}: near level {} triangles", tris[0]);
            assert!(tris[1] < tris[0] && tris[2] < tris[1], "{name}: levels get lighter ({tris:?})");
            table.push_str(&format!("{name:15} {:5} {:5} {:5}\n", tris[0], tris[1], tris[2]));
        }
        eprintln!("stand-in triangles (near, mid, far):\n{table}");
    }

    /// Each stand-in stands on its foot (nothing below the footway) and is about the size of
    /// the GLB it replaces (`dimensionsM`): the height within 12 % (the historic light, a
    /// pedestrian-scale lantern under the 7.62 m district height, excepted), and no wider
    /// or deeper than the GLB's larger footprint side by more than 30 %.
    #[test]
    fn stand_ins_match_the_glb_dimensions() {
        let (js, cx) = ctx();
        let oh = eo_props::overhead::OverheadJs::load().unwrap();
        for name in KINDS {
            let dims: Vec<f64> = js.street_prop(name).ok().and_then(|p| p.dimensions_m.clone()).or_else(|| oh.consts[name]["dimensionsM"].as_array().map(|a| a.iter().filter_map(|v| v.as_f64()).collect())).unwrap_or_default();
            assert_eq!(dims.len(), 3, "{name}: the manifest gives its dimensions");
            let (mn, mx) = bounds(&level(name, &cx, 0));
            let size = mx - mn;
            if !matches!(name, "cobraArm" | "lantern" | "utilityPole") {
                assert!(mn.y > -0.002, "{name}: nothing below the foot ({})", mn.y);
            }
            let h = dims[1] as f32;
            if name == "lampHistoric" {
                assert!(mx.y > cx.column + 0.3 && mx.y < h, "{name}: the lantern's top {} over its light at {}", mx.y, cx.column);
            } else if name == "mailbox" || name == "payStation" {
                // The GLB's heights are below the kind's (1.2 and 1.5 m, which these follow).
                assert!(size.y < 1.6, "{name}: height {}", size.y);
            } else {
                assert!((size.y - h).abs() < 0.12 * h, "{name}: height {} against the GLB's {h}", size.y);
            }
            let foot = dims[0].max(dims[2]) as f32 * 1.3;
            assert!(size.x <= foot && size.z <= foot.max(1.3 * dims[2] as f32), "{name}: footprint {size:?} against the GLB's {dims:?}");
        }
    }

    /// The district lights' heads hold their light: the lens under the cobra and banner
    /// heads at `LAMP_HEAD.district` (height, reach toward local -z), the historic
    /// lantern's glass round `LAMP_HEAD.column`.
    #[test]
    fn luminaires_sit_at_their_lights() {
        let (_, cx) = ctx();
        let [h, reach] = cx.district;
        for name in ["lampCobra", "lampBanner"] {
            let b = level(name, &cx, 0);
            let lens = b.v.iter().filter(|v| v.material == pm().glass).map(|v| Vec3::from(v.pos)).collect::<Vec<_>>();
            assert!(!lens.is_empty(), "{name}: a lens");
            let c = lens.iter().copied().sum::<Vec3>() / lens.len() as f32;
            assert!((c - v3(0.0, h, -reach)).length() < 0.12, "{name}: lens centre {c} against the light at (0, {h}, {})", -reach);
        }
        let b = level("lampHistoric", &cx, 0);
        let glass: Vec<f32> = b.v.iter().filter(|v| v.material == pm().glass).map(|v| v.pos[1]).collect();
        let (lo, hi) = glass.iter().fold((f32::MAX, f32::MIN), |(a, c), &y| (a.min(y), c.max(y)));
        assert!(lo < cx.column && hi > cx.column, "lampHistoric: glass {lo}..{hi} round the light at {}", cx.column);
    }

    /// The distribution pole's insulators top out at the overhead's wire seats.
    #[test]
    fn distribution_pole_insulators_meet_the_wire_seats() {
        let (_, cx) = ctx();
        let b = level("utilityPole", &cx, 0);
        for a in &cx.arms {
            for x in [a[0], a[1]] {
                let near = b.v.iter().filter(|v| v.material == pm().glass).map(|v| Vec3::from(v.pos)).filter(|p| (p.x - x).abs() < 0.08 && (p.y - a[2]).abs() < 0.3).fold(f32::MIN, |m, p| m.max(p.y));
                assert!((near - a[2]).abs() < 0.01, "insulator at x {x}: top {near} against the seat at {}", a[2]);
            }
        }
    }

    /// The base dirt darkens a white part at its foot and leaves it clean higher up.
    #[test]
    fn base_dirt_fades_with_height() {
        let mut b = Builder::default();
        bxa(&mut b, 0, v3(-0.1, 0.0, -0.1), v3(0.1, 1.2, 0.1), [236, 235, 228]);
        dirt(&mut b, 0, 0.42);
        let at = |y: f32| b.v.iter().find(|v| (v.pos[1] - y).abs() < 1e-4).map(|v| v.color[0]).unwrap();
        assert!(at(0.0) < at(0.32) && at(0.32) < at(1.2) && at(1.2) == 236, "dirt {} / {} / {}", at(0.0), at(0.32), at(1.2));
    }
}
