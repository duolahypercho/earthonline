//! The JS game's authored traffic bodies (`HERO_VEHICLE_MANIFEST`), cooked for
//! the agent renderer.
//!
//! Each LOD GLB is verified (byte length + SHA-256, as `loadHeroSedanSource`
//! does) and flattened: every mesh primitive reachable from the default scene
//! with its node transform, POSITION and NORMAL, one index list for the opaque
//! materials and one for the alpha-blended ones (glass and lenses). Every
//! vertex carries its material and its wheel part, read from the rig the JS
//! animates (`instantiateHeroSedan`): the steer pivots `Wheel_FL/FR/RL/RR` at
//! the wheel centres, their `Wheel_XX_Spin` children spinning about the
//! pivot's x axis, and the calipers that steer but do not spin. The bodies
//! carry no textures (checked: no images in any fleet GLB).
//!
//! Materials keep their glTF factors (as the overlay's materials do), with
//! these roles taken by name from the JS overlay:
//! `VehiclePaint` takes the instance paint (`instanceColor` on the paint batch);
//! the lamp materials (`LAMP_CHANNEL_BY_MATERIAL`: VehicleHeadlamp and
//! VehicleDRL head, VehicleTaillamp tail, VehicleBrakeLamp brake, VehicleReverse
//! reverse, VehicleIndicator indicator) emit their `HERO_LAMP` channel colour at
//! the channel's day/night intensity for the lamp state (the authored emissive
//! is replaced, as `heroLampState` drives the lens); a body without its own
//! brake material lights the tail lamps for the brake (`max(tail, brake)`).

use crate::{AgentMaterial, AgentVertex, F_BLEND, F_GLASS, M_BRAKE, M_DRL, M_HEAD, M_INDICATOR, M_PAINT, M_REVERSE, M_TAIL, hex_linear, tag};
use anyhow::{Result, anyhow, bail};
use glam::{Mat3, Mat4, Vec3};
use std::collections::BTreeMap;

/// One cooked LOD of a body (body frame: +y up, +z nose, ground at y = 0).
#[derive(Clone, Debug, Default)]
pub struct CookedBody {
    pub vertices: Vec<AgentVertex>,
    pub opaque: Vec<u32>,
    pub glass: Vec<u32>,
    /// Material per local index (into `materials`).
    pub materials: Vec<AgentMaterial>,
    /// Wheel pivots FL, FR, RL, RR (None when the rig lacks one).
    pub pivots: [Option<[f32; 3]>; 4],
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub triangles: u32,
    pub material_names: Vec<String>,
}

pub const WHEELS: [&str; 4] = ["Wheel_FL", "Wheel_FR", "Wheel_RL", "Wheel_RR"];

pub fn pack_normal(n: Vec3) -> [i8; 4] {
    let n = n.normalize_or_zero();
    [(n.x * 127.0).round() as i8, (n.y * 127.0).round() as i8, (n.z * 127.0).round() as i8, 0]
}

/// Lamp colours from `HERO_LAMP` (linear).
#[derive(Clone, Debug)]
pub struct LampColours {
    pub head: [f32; 3],
    pub tail: [f32; 3],
    pub brake: [f32; 3],
    pub reverse: [f32; 3],
    pub indicator: [f32; 3],
}

impl LampColours {
    pub fn from_js(js: &crate::catalog::AgentsJs) -> Self {
        let c = |k: &str| js.hero_lamp.get(k).map(|l| hex_linear(&l.hex)).unwrap_or([1.0; 3]);
        Self { head: c("head"), tail: c("tail"), brake: c("brake"), reverse: c("reverse"), indicator: c("indicator") }
    }
}

fn material_of(m: &gltf::Material, lamps: &LampColours) -> AgentMaterial {
    let pbr = m.pbr_metallic_roughness();
    let bc = pbr.base_color_factor();
    let name = m.name().unwrap_or("");
    let es = m.emissive_strength().unwrap_or(1.0);
    let e = m.emissive_factor();
    let blend = m.alpha_mode() == gltf::material::AlphaMode::Blend;
    let mut flags = 0u32;
    let mut emissive = [e[0] * es, e[1] * es, e[2] * es, 0.0];
    let mut lamp = |f: u32, c: [f32; 3]| {
        flags |= f;
        emissive = [c[0], c[1], c[2], 0.0];
    };
    match name {
        "VehicleTaillamp" => lamp(M_TAIL, lamps.tail),
        "VehicleBrakeLamp" => lamp(M_BRAKE, lamps.brake),
        "VehicleIndicator" => lamp(M_INDICATOR, lamps.indicator),
        "VehicleHeadlamp" => lamp(M_HEAD, lamps.head),
        "VehicleDRL" => lamp(M_DRL, lamps.head),
        "VehicleReverse" => lamp(M_REVERSE, lamps.reverse),
        _ => {}
    }
    if name == "VehiclePaint" {
        flags |= M_PAINT;
    }
    if blend {
        flags |= F_BLEND;
    }
    if name == "VehicleGlass" || name == "VehicleGlassTint" {
        flags |= F_GLASS;
    }
    AgentMaterial {
        base: [bc[0], bc[1], bc[2], if blend { bc[3] } else { 1.0 }],
        emissive,
        params: [pbr.roughness_factor(), pbr.metallic_factor(), 0.0, 0.0],
        layer: -1,
        flags,
        rough_flatten: 0.0,
        thin_absorption: 0.0,
    }
}

/// Parse a verified body GLB.
pub fn cook_body(bytes: &[u8], lamps: &LampColours) -> Result<CookedBody> {
    let gltf = gltf::Gltf::from_slice(bytes).map_err(|e| anyhow!("glTF parse: {e}"))?;
    let blob = gltf.blob.as_deref();
    let doc = &gltf.document;
    let mut out = CookedBody::default();
    let mut mat_index: BTreeMap<usize, u32> = BTreeMap::new();
    let default_mat = AgentMaterial { base: [0.5, 0.5, 0.5, 1.0], params: [0.6, 0.0, 0.0, 0.0], layer: -1, ..Default::default() };
    let scene = doc.default_scene().or_else(|| doc.scenes().next()).ok_or_else(|| anyhow!("no scene"))?;
    // (node, parent world, wheel tag inherited)
    let mut stack: Vec<(gltf::Node, Mat4, u32)> = scene.nodes().map(|n| (n, Mat4::IDENTITY, 0)).collect();
    let (mut mn, mut mx) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    while let Some((node, parent, inherited)) = stack.pop() {
        let world = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
        let name = node.name().unwrap_or("");
        let mut wheel = inherited;
        if let Some(w) = WHEELS.iter().position(|w| *w == name) {
            out.pivots[w] = Some(world.transform_point3(Vec3::ZERO).to_array());
            wheel = 5 + w as u32; // steer-only until a Spin node below
        } else if let Some(w) = WHEELS.iter().position(|w| name == format!("{w}_Spin")) {
            wheel = 1 + w as u32;
        }
        for child in node.children() {
            stack.push((child, world, wheel));
        }
        if name.starts_with("shadow-proxy") {
            continue;
        }
        let Some(mesh) = node.mesh() else { continue };
        let normal_m = Mat3::from_mat4(world).inverse().transpose();
        for p in mesh.primitives() {
            if p.mode() != gltf::mesh::Mode::Triangles {
                bail!("primitive mode {:?} is not supported (triangles only)", p.mode());
            }
            let reader = p.reader(|b| match b.source() {
                gltf::buffer::Source::Bin => blob,
                gltf::buffer::Source::Uri(_) => None,
            });
            let positions: Vec<[f32; 3]> = reader.read_positions().ok_or_else(|| anyhow!("primitive without POSITION"))?.collect();
            let normals: Option<Vec<[f32; 3]>> = reader.read_normals().map(|n| n.collect());
            let local = match p.material().index() {
                Some(i) => *mat_index.entry(i).or_insert_with(|| {
                    out.materials.push(material_of(&p.material(), lamps));
                    out.material_names.push(p.material().name().unwrap_or("").to_string());
                    out.materials.len() as u32 - 1
                }),
                None => *mat_index.entry(usize::MAX).or_insert_with(|| {
                    out.materials.push(default_mat);
                    out.material_names.push(String::new());
                    out.materials.len() as u32 - 1
                }),
            };
            let blend = out.materials[local as usize].flags & F_BLEND != 0;
            let base = out.vertices.len() as u32;
            for (i, p3) in positions.iter().enumerate() {
                let wp = world.transform_point3(Vec3::from(*p3));
                mn = mn.min(wp);
                mx = mx.max(wp);
                let n = normals.as_ref().map(|n| normal_m * Vec3::from(n[i])).unwrap_or(Vec3::Y);
                out.vertices.push(AgentVertex { pos: wp.to_array(), normal: pack_normal(n), uv: [0, 0], joints: [0; 4], weights: [0; 4], tag: tag(local, wheel) });
            }
            let indices: Vec<u32> = match reader.read_indices() {
                Some(ix) => ix.into_u32().map(|i| i + base).collect(),
                None => (0..positions.len() as u32).map(|i| i + base).collect(),
            };
            out.triangles += indices.len() as u32 / 3;
            if blend { out.glass.extend(indices) } else { out.opaque.extend(indices) }
        }
    }
    if out.vertices.is_empty() {
        bail!("body has no geometry");
    }
    out.min = mn.to_array();
    out.max = mx.to_array();
    Ok(out)
}

/// A painted box over a body's bounds: the last distance level, and the
/// stand-in for a body whose GLBs are missing from the checkout.
pub fn box_body(min: [f32; 3], max: [f32; 3]) -> CookedBody {
    let paint = AgentMaterial { base: [0.8, 0.8, 0.8, 1.0], params: [0.4, 0.0, 0.0, 0.0], layer: -1, flags: M_PAINT, ..Default::default() };
    let glass = AgentMaterial { base: [0.03, 0.033, 0.035, 1.0], params: [0.1, 0.0, 0.0, 0.0], layer: -1, flags: 0, ..Default::default() };
    let tyre = AgentMaterial { base: [0.014, 0.014, 0.015, 1.0], params: [0.9, 0.0, 0.0, 0.0], layer: -1, flags: 0, ..Default::default() };
    let mut b = CookedBody { materials: vec![paint, glass, tyre], material_names: vec!["box-paint".into(), "box-glass".into(), "box-tyre".into()], min, max, ..Default::default() };
    let h = max[1] - min[1];
    let (x0, x1, z0, z1) = (min[0] * 0.98, max[0] * 0.98, min[2], max[2]);
    // Lower body (tyres band), painted body, glasshouse.
    let y_sill = min[1].max(0.0) + h * 0.18;
    let y_belt = min[1].max(0.0) + h * 0.62;
    let mut cuboid = |lo: [f32; 3], hi: [f32; 3], m: u32| {
        let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
            ([1.0, 0.0, 0.0], [[hi[0], lo[1], lo[2]], [hi[0], hi[1], lo[2]], [hi[0], hi[1], hi[2]], [hi[0], lo[1], hi[2]]]),
            ([-1.0, 0.0, 0.0], [[lo[0], lo[1], hi[2]], [lo[0], hi[1], hi[2]], [lo[0], hi[1], lo[2]], [lo[0], lo[1], lo[2]]]),
            ([0.0, 1.0, 0.0], [[lo[0], hi[1], lo[2]], [lo[0], hi[1], hi[2]], [hi[0], hi[1], hi[2]], [hi[0], hi[1], lo[2]]]),
            ([0.0, -1.0, 0.0], [[lo[0], lo[1], hi[2]], [lo[0], lo[1], lo[2]], [hi[0], lo[1], lo[2]], [hi[0], lo[1], hi[2]]]),
            ([0.0, 0.0, 1.0], [[hi[0], lo[1], hi[2]], [hi[0], hi[1], hi[2]], [lo[0], hi[1], hi[2]], [lo[0], lo[1], hi[2]]]),
            ([0.0, 0.0, -1.0], [[lo[0], lo[1], lo[2]], [lo[0], hi[1], lo[2]], [hi[0], hi[1], lo[2]], [hi[0], lo[1], lo[2]]]),
        ];
        for (n, q) in faces {
            let base = b.vertices.len() as u32;
            for p in q {
                b.vertices.push(AgentVertex { pos: p, normal: pack_normal(Vec3::from(n)), tag: tag(m, 0), ..Default::default() });
            }
            b.opaque.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            b.triangles += 2;
        }
    };
    cuboid([x0 * 0.92, min[1].max(0.0), z0 * 0.96], [x1 * 0.92, y_sill, z1 * 0.96], 2);
    cuboid([x0, y_sill, z0], [x1, y_belt, z1], 0);
    let zl = z1 - z0;
    cuboid([x0 * 0.94, y_belt, z0 + zl * 0.2], [x1 * 0.94, max[1], z1 - zl * 0.25], 1);
    b
}

/// The far body's geometric error (`--car-detail`): the simplifier may move
/// the surface by at most this many metres. The far slot starts at 220 m
/// under `--car-detail 1` (260 m under 2), where a metre is about 4.9 px of
/// a 1080 px frame at the game's 52 degree field: 0.2 m is about a pixel
/// there. Over the shipped fleet this keeps 1,080-1,800 of LOD 2's
/// 5,380-6,000 triangles (eo-agents tests/far_body.rs prints each body).
pub const FAR_BODY_ERROR_M: f32 = 0.2;
/// Bump when `far_body` changes what it makes (it is part of the library's cache key).
pub const FAR_BODY_VERSION: u32 = 1;
/// The far body's triangle budget, shared among its material runs by their
/// share of the source's; the error (`FAR_BODY_ERROR_M`) binds first on the
/// shipped fleet, so this only caps a denser body.
pub const FAR_BODY_TRIANGLES: usize = 600;
/// A material run this small is kept whole in the far body (lamp lenses, badges).
pub const FAR_BODY_KEEP_TRIANGLES: usize = 24;

/// A far body made from a finer cooked LOD (the far slot under `--car-detail`,
/// in place of `box_body`): each run of triangles that shares a vertex tag
/// (material and wheel part) is simplified on its own with meshopt, toward
/// its share of `target_triangles` but never past `max_error_m` from the
/// source surface, so paint, glass, lamps and tyres keep their materials and
/// the wheels still spin about their pivots. The vertices are the source's
/// own (the simplifier only picks among them), compacted to the ones still
/// used. Bounds, pivots and the material table are the source's.
/// Deterministic: the same source gives the same body.
pub fn far_body(src: &CookedBody, target_triangles: usize, max_error_m: f32) -> CookedBody {
    // Triangles grouped by their first vertex's tag, in the order the tags first appear.
    let groups = |list: &[u32]| {
        let mut order: Vec<u32> = vec![];
        let mut by_tag: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        for t in list.chunks_exact(3) {
            let tag = src.vertices[t[0] as usize].tag;
            by_tag.entry(tag).or_insert_with(|| {
                order.push(tag);
                vec![]
            });
            by_tag.get_mut(&tag).unwrap().extend_from_slice(t);
        }
        order.into_iter().map(|t| by_tag.remove(&t).unwrap()).collect::<Vec<_>>()
    };
    // The bodies are built from panel-cut pieces whose borders an
    // edge-collapse simplifier may not move (it kept ~45 % of LOD 2 even at a
    // 0.3 m error), so each group is clustered instead (meshopt's sloppy
    // simplifier, which ignores topology) down to the error. A group of at
    // most `FAR_BODY_KEEP_TRIANGLES` (a lamp lens, a badge) is kept whole, so
    // the lamps still light at night. The clustering runs over every vertex
    // it is handed, so each group gets a vertex buffer of its own vertices
    // alone (else it would pick another material's vertex), and its error is
    // relative to that buffer's extent.
    let total = ((src.opaque.len() + src.glass.len()) / 3).max(1);
    let simplify = |list: &[u32]| -> Vec<u32> {
        let mut out = vec![];
        for g in groups(list) {
            let n = g.len() / 3;
            if n <= FAR_BODY_KEEP_TRIANGLES {
                out.extend_from_slice(&g);
                continue;
            }
            // the group's own vertices (local index -> source index) and its indices into them.
            let mut local: BTreeMap<u32, u32> = BTreeMap::new();
            let mut back: Vec<u32> = vec![];
            let li: Vec<u32> = g
                .iter()
                .map(|&i| {
                    *local.entry(i).or_insert_with(|| {
                        back.push(i);
                        back.len() as u32 - 1
                    })
                })
                .collect();
            let pos: Vec<[f32; 3]> = back.iter().map(|&i| src.vertices[i as usize].pos).collect();
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for p in &pos {
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
            let extent = (0..3).map(|k| hi[k] - lo[k]).fold(0.0f32, f32::max).max(1e-3);
            let Ok(adapter) = meshopt::VertexDataAdapter::new(bytemuck::cast_slice(&pos), std::mem::size_of::<[f32; 3]>(), 0) else {
                out.extend_from_slice(&g);
                continue;
            };
            let want = (n * target_triangles).div_ceil(total).max(FAR_BODY_KEEP_TRIANGLES);
            let s = meshopt::simplify_sloppy(&li, &adapter, want * 3, max_error_m / extent, None);
            if s.is_empty() {
                out.extend_from_slice(&g);
            } else {
                out.extend(s.iter().map(|&l| back[l as usize]));
            }
        }
        out
    };
    let (opaque, glass) = (simplify(&src.opaque), simplify(&src.glass));
    // Keep the vertices still used, in first-use order.
    let mut remap = vec![u32::MAX; src.vertices.len()];
    let mut b = CookedBody { materials: src.materials.clone(), material_names: src.material_names.clone(), pivots: src.pivots, min: src.min, max: src.max, ..Default::default() };
    let mut take = |list: &[u32], vertices: &mut Vec<AgentVertex>| -> Vec<u32> {
        list.iter()
            .map(|&i| {
                if remap[i as usize] == u32::MAX {
                    remap[i as usize] = vertices.len() as u32;
                    vertices.push(src.vertices[i as usize]);
                }
                remap[i as usize]
            })
            .collect()
    };
    b.opaque = take(&opaque, &mut b.vertices);
    b.glass = take(&glass, &mut b.vertices);
    b.triangles = ((b.opaque.len() + b.glass.len()) / 3) as u32;
    if b.opaque.is_empty() {
        return box_body(src.min, src.max);
    }
    b
}
