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

/// The steps of a refined component inside one 8-bit step (`pack_normal_fine`).
pub const NORMAL_FINE_STEPS: i32 = 14;

/// (`--car-materials`) `pack_normal`, and in the spare fourth byte what the
/// 8-bit components left: an 8-bit normal is off by up to 0.3 degrees,
/// vertex to vertex at random, which a mirror-like lacquer doubles into
/// ripples in its reflections (a dented-can look at a few metres). The two
/// components smaller than the largest (by the 8-bit values, the lowest
/// axis first on a tie) carry their residual in fourteenths of a step
/// (`c = (e1 + 7) + 15 (e2 + 7)`, 0..224, stored as `c - 112` below 112 and
/// `c - 111` from it: never 0, which stays the people's "no refinement",
/// nor -128); the largest follows from unit length with its 8-bit sign
/// (`unpack_normal_fine`; agents_draw.wgsl `ag_car_normal`). At worst about
/// 0.04 degrees (the 8-bit normal's 0.37). xyz are `pack_normal`'s own, so a
/// reader of xyz alone sees the normal it saw before.
pub fn pack_normal_fine(n: Vec3) -> [i8; 4] {
    let mut q = pack_normal(n);
    let n = n.normalize_or_zero();
    if n == Vec3::ZERO {
        return q;
    }
    let (i, j) = normal_fine_axes([q[0], q[1], q[2]]);
    let e = |k: usize| ((n[k] * 127.0 - q[k] as f32) * NORMAL_FINE_STEPS as f32).round().clamp(-7.0, 7.0) as i32;
    let c = (e(i) + 7) + 15 * (e(j) + 7);
    q[3] = (if c < 112 { c - 112 } else { c - 111 }) as i8;
    q
}

/// The two refined axes of `pack_normal_fine`: all but the largest 8-bit magnitude (the lowest axis wins a tie).
fn normal_fine_axes(q: [i8; 3]) -> (usize, usize) {
    let a = q.map(|c| (c as i32).abs());
    let k = if a[0] >= a[1] && a[0] >= a[2] { 0 } else if a[1] >= a[2] { 1 } else { 2 };
    match k {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    }
}

/// The normal `pack_normal_fine` packed, as agents_draw.wgsl `ag_car_normal`
/// reads it (a spare byte of 0: the 8-bit normal, unrefined).
pub fn unpack_normal_fine(p: [i8; 4]) -> Vec3 {
    let q = [p[0], p[1], p[2]];
    if p[3] == 0 {
        return Vec3::new(q[0] as f32, q[1] as f32, q[2] as f32).normalize_or_zero();
    }
    let c = p[3] as i32 + if p[3] < 0 { 112 } else { 111 };
    let (i, j) = normal_fine_axes(q);
    let k = 3 - i - j;
    let mut n = [0f32; 3];
    n[i] = (q[i] as f32 + ((c % 15) - 7) as f32 / NORMAL_FINE_STEPS as f32) / 127.0;
    n[j] = (q[j] as f32 + ((c / 15) - 7) as f32 / NORMAL_FINE_STEPS as f32) / 127.0;
    n[k] = (1.0 - n[i] * n[i] - n[j] * n[j]).max(0.0).sqrt() * if q[k] < 0 { -1.0 } else { 1.0 };
    Vec3::from(n).normalize_or_zero()
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

// ---- (`--car-materials`) what each material is, for the cars' own shading
// (eo-render agents_draw.wgsl `ag_car_*`), and the plates' own coordinates.

/// The material's role sits in these bits of its flags (above the agent
/// flags `M_*`, below nothing draw.wgsl reads): 0 none (as before).
pub const M_ROLE_SHIFT: u32 = 20;
pub const M_ROLE_MASK: u32 = 0xf << M_ROLE_SHIFT;
/// VehicleGlass: the windscreen, side and rear glass.
pub const ROLE_GLASS: u32 = 1;
/// VehicleGlassTint: privacy glass (darker).
pub const ROLE_GLASS_TINT: u32 = 2;
/// VehicleTyre.
pub const ROLE_TYRE: u32 = 3;
/// VehiclePlate: the licence plates (their texture coordinates are the
/// plate's own, 0..1 across as the plate is read and 0..1 up).
pub const ROLE_PLATE: u32 = 4;
/// VehicleChrome, VehicleMirror.
pub const ROLE_CHROME: u32 = 5;
/// The clear and coloured covers over the lamps (`*Lens*`).
pub const ROLE_LENS: u32 = 6;
/// The cabin: VehicleInterior, VehicleInteriorTrim, the seats, the floor and the screen.
pub const ROLE_INTERIOR: u32 = 7;
/// The wheels: VehicleAlloy, VehicleAlloyDark, VehicleWheelSteel.
pub const ROLE_WHEEL: u32 = 8;
/// The black plastics: VehicleTrimBlack, VehicleTrimGloss, VehicleTrimSatin.
pub const ROLE_TRIM: u32 = 9;
/// VehicleLampReflector: the chrome bowls inside the lamps.
pub const ROLE_REFLECTOR: u32 = 10;
/// VehicleEmergencyBlue, VehicleEmergencyRed: a patrol car's light bar.
pub const ROLE_EMERGENCY: u32 = 11;
/// VehicleBrakeDisc, VehicleCaliper.
pub const ROLE_BRAKE: u32 = 12;
/// The liveries (`VehicleLivery*`): body panels painted in a fixed colour.
pub const ROLE_LIVERY: u32 = 13;
/// Bump when `role_of`, the plates' coordinates, the normals' refinement or
/// the contact shapes change (part of the library's cache key).
pub const CAR_MATERIALS_VERSION: u32 = 3;

/// The role of a fleet material by its name (0: none of the roles).
pub fn role_of(name: &str) -> u32 {
    match name {
        "VehicleGlass" => ROLE_GLASS,
        "VehicleGlassTint" => ROLE_GLASS_TINT,
        "VehicleTyre" => ROLE_TYRE,
        "VehiclePlate" => ROLE_PLATE,
        "VehicleChrome" | "VehicleMirror" => ROLE_CHROME,
        "VehicleInterior" | "VehicleInteriorTrim" | "VehicleSeatInsert" | "VehicleSeatFabric" | "VehicleFloor" | "VehicleScreen" => ROLE_INTERIOR,
        "VehicleAlloy" | "VehicleAlloyDark" | "VehicleWheelSteel" => ROLE_WHEEL,
        "VehicleTrimBlack" | "VehicleTrimGloss" | "VehicleTrimSatin" => ROLE_TRIM,
        "VehicleLampReflector" => ROLE_REFLECTOR,
        "VehicleEmergencyBlue" | "VehicleEmergencyRed" => ROLE_EMERGENCY,
        "VehicleBrakeDisc" | "VehicleCaliper" => ROLE_BRAKE,
        n if n.starts_with("VehicleLivery") => ROLE_LIVERY,
        n if n.contains("Lens") => ROLE_LENS,
        _ => 0,
    }
}

/// The role a material's flags carry (0: none, or a library cooked without `--car-materials`).
pub fn role_of_flags(flags: u32) -> u32 {
    (flags & M_ROLE_MASK) >> M_ROLE_SHIFT
}

/// Parse a verified body GLB.
pub fn cook_body(bytes: &[u8], lamps: &LampColours) -> Result<CookedBody> {
    cook_body_with(bytes, lamps, false)
}

/// `cook_body`, and with `car_materials` (`--car-materials`) each material's
/// role in its flags (`role_of`) and each licence plate's own coordinates in
/// its vertices' texture coordinates: u 0..1 across the plate as it is read
/// (left to right seen from in front of it: the front plate's +x, the rear
/// plate's -x, the body's +x being its left flank), v 0..1 bottom to top,
/// from the plate mesh's own bounds (every fleet plate is a 12 x 6 in box).
pub fn cook_body_with(bytes: &[u8], lamps: &LampColours, car_materials: bool) -> Result<CookedBody> {
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
                    let mut m = material_of(&p.material(), lamps);
                    if car_materials {
                        m.flags |= role_of(p.material().name().unwrap_or("")) << M_ROLE_SHIFT;
                    }
                    out.materials.push(m);
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
            // (`--car-materials`) a plate's own coordinates, from its mesh's bounds (before the node's transform).
            let plate = (car_materials && role_of_flags(out.materials[local as usize].flags) == ROLE_PLATE).then(|| {
                let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
                for q in &positions {
                    lo = lo.min(Vec3::from(*q));
                    hi = hi.max(Vec3::from(*q));
                }
                (lo, (hi - lo).max(Vec3::splat(1e-4)), name.contains("Rear"))
            });
            let base = out.vertices.len() as u32;
            for (i, p3) in positions.iter().enumerate() {
                let wp = world.transform_point3(Vec3::from(*p3));
                mn = mn.min(wp);
                mx = mx.max(wp);
                let n = normals.as_ref().map(|n| normal_m * Vec3::from(n[i])).unwrap_or(Vec3::Y);
                let uv = match plate {
                    Some((lo, size, rear)) => {
                        let u = (p3[0] - lo.x) / size.x;
                        let v = (p3[1] - lo.y) / size.y;
                        let u = if rear { 1.0 - u } else { u };
                        [(u.clamp(0.0, 1.0) * 65535.0).round() as u16, (v.clamp(0.0, 1.0) * 65535.0).round() as u16]
                    }
                    None => [0, 0],
                };
                // (`--car-materials`) the normal's residual in its spare byte.
                let normal = if car_materials { pack_normal_fine(n) } else { pack_normal(n) };
                out.vertices.push(AgentVertex { pos: wp.to_array(), normal, uv, joints: [0; 4], weights: [0; 4], tag: tag(local, wheel) });
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

/// (`--car-contact`) The boxes a body's occlusion of the street round it is
/// reckoned from (eo-render agents_contact.wgsl), in the body frame, metres:
/// the lower body from its underside to the beltline, the upper body (the
/// glasshouse, a van's or a truck's box) from there to the roof, and the
/// tyres' width.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ContactShape {
    /// Half the lower body's width (without the mirrors).
    pub lower_half_w: f32,
    /// The underside's height between the axles.
    pub clearance: f32,
    /// The lower body's ends along z (tail, nose).
    pub z0: f32,
    pub z1: f32,
    /// The beltline: the foot of the side glass.
    pub belt: f32,
    /// Half the upper body's width, its ends along z and its top.
    pub upper_half_w: f32,
    pub uz0: f32,
    pub uz1: f32,
    pub top: f32,
    /// Half a tyre's width.
    pub tyre_half_w: f32,
}

/// The `p` quantile (0..1) of `v` (sorted in place); `d` when empty.
fn quantile(v: &mut [f32], p: f32, d: f32) -> f32 {
    if v.is_empty() {
        return d;
    }
    v.sort_by(f32::total_cmp);
    v[((v.len() - 1) as f32 * p).round() as usize]
}

/// The occluder boxes of a cooked body (`ContactShape`), from its vertices by
/// material: the body is every part that is not a wheel part, glass, a
/// mirror or the cabin. Quantiles, not extremes, so a stray vertex (an
/// aerial, a mirror stalk, an exhaust tip) does not widen a box.
pub fn contact_shape(b: &CookedBody) -> ContactShape {
    let name = |v: &AgentVertex| b.material_names.get((v.tag & 0xffff) as usize).map(String::as_str).unwrap_or("");
    let wheel = |v: &AgentVertex| (v.tag >> 16) & 0xff != 0;
    let glass = |n: &str| n == "VehicleGlass" || n == "VehicleGlassTint";
    let cabin = |n: &str| role_of(n) == ROLE_INTERIOR || n == "VehicleMirror";
    let (lo, hi) = (Vec3::from(b.min), Vec3::from(b.max));
    let (zc, len, height) = ((lo.z + hi.z) * 0.5, (hi.z - lo.z).max(0.5), hi.y.max(0.5));
    let body: Vec<&AgentVertex> = b.vertices.iter().filter(|v| !wheel(v) && !glass(name(v)) && !cabin(name(v))).collect();
    let mut under: Vec<f32> = body.iter().filter(|v| (v.pos[2] - zc).abs() < 0.25 * len).map(|v| v.pos[1]).collect();
    let clearance = quantile(&mut under, 0.01, 0.18 * height.min(1.5)).max(0.05);
    // (a low quantile: a bus's door glass reaches far below its windows)
    let mut glass_y: Vec<f32> = b.vertices.iter().filter(|v| glass(name(v))).map(|v| v.pos[1]).collect();
    let belt = quantile(&mut glass_y, 0.05, 0.55 * height).clamp(clearance + 0.2, height - 0.1);
    let mut lx: Vec<f32> = body.iter().filter(|v| v.pos[1] < belt).map(|v| v.pos[0].abs()).collect();
    let mut lz: Vec<f32> = body.iter().filter(|v| v.pos[1] < belt).map(|v| v.pos[2]).collect();
    let lower_half_w = quantile(&mut lx, 0.99, hi.x.abs().max(lo.x.abs()) * 0.9);
    let (z0, z1) = (quantile(&mut lz, 0.005, lo.z), quantile(&mut lz, 0.995, hi.z));
    let upper: Vec<&AgentVertex> = b.vertices.iter().filter(|v| !wheel(v) && !cabin(name(v)) && v.pos[1] > belt + 0.05).collect();
    let mut ux: Vec<f32> = upper.iter().map(|v| v.pos[0].abs()).collect();
    let mut uz: Vec<f32> = upper.iter().map(|v| v.pos[2]).collect();
    let mut uy: Vec<f32> = upper.iter().map(|v| v.pos[1]).collect();
    let upper_half_w = quantile(&mut ux, 0.95, lower_half_w * 0.85).min(lower_half_w);
    let (uz0, uz1) = (quantile(&mut uz, 0.01, z0 + 0.3 * len), quantile(&mut uz, 0.99, z1 - 0.3 * len));
    let top = quantile(&mut uy, 0.995, height).max(belt + 0.1);
    let mut tx: Vec<f32> = b.vertices.iter().filter(|v| name(v) == "VehicleTyre").map(|v| v.pos[0].abs()).collect();
    let tyre_half_w = if tx.is_empty() { 0.11 } else { ((quantile(&mut tx, 1.0, 0.0) - quantile(&mut tx, 0.0, 0.0)) * 0.5).clamp(0.06, 0.25) };
    ContactShape { lower_half_w, clearance, z0, z1, belt, upper_half_w, uz0, uz1, top, tyre_half_w }
}

/// `ContactShape` as the GPU model carries it (`GpuModel::pad` and the
/// wheels' spare w; eo-render agents_contact.wgsl `ag_ct_shape`): pairs of
/// signed millimetres in the pad words (lower half width and clearance, the
/// lower ends, the beltline and the upper half width, the upper ends), the
/// top in wheel 0's w and the tyres' half width in wheel 1's.
pub fn contact_words(s: &ContactShape) -> ([u32; 4], [f32; 2]) {
    let mm = |a: f32, b: f32| {
        let q = |x: f32| ((x * 1000.0).round().clamp(-32767.0, 32767.0) as i16) as u16 as u32;
        q(a) | (q(b) << 16)
    };
    ([mm(s.lower_half_w, s.clearance), mm(s.z0, s.z1), mm(s.belt, s.upper_half_w), mm(s.uz0, s.uz1)], [s.top, s.tyre_half_w])
}

/// (`--car-lamps`) Where a body's rear lamps are: the mean of the vertices of
/// its tail and brake lamp materials (`M_TAIL`, `M_BRAKE`) behind `mid_z`,
/// x as the distance from the centre line (the left lamp; the right one
/// mirrors it), y up, z along, metres in the body frame. `parts` yields each
/// vertex of the body's full level with its material's flags (a vertex once
/// per triangle it is in, so a lamp's larger faces weigh more). None for a
/// body without rear lamps.
pub fn rear_lamp<'a>(parts: impl Iterator<Item = (&'a AgentVertex, u32)>, mid_z: f32) -> Option<[f32; 3]> {
    let (mut s, mut n) = ([0f64; 3], 0u32);
    for (v, flags) in parts {
        if flags & (M_TAIL | M_BRAKE) != 0 && v.pos[2] < mid_z {
            s[0] += v.pos[0].abs() as f64;
            s[1] += v.pos[1] as f64;
            s[2] += v.pos[2] as f64;
            n += 1;
        }
    }
    (n > 0).then(|| s.map(|c| (c / n as f64) as f32))
}

/// The packing of `rear_lamp` in a model's spare words (eo-render
/// agents_lamps.wgsl `lt_rear_lamp_at`): the lamp's z in wheel 2's w, its x
/// and y in centimetres as `x * REAR_LAMP_PACK + y` in wheel 3's w (an exact
/// integer in an f32; 0 there means no lamp).
pub const REAR_LAMP_PACK: f32 = 512.0;

pub fn rear_lamp_words(p: [f32; 3]) -> [f32; 2] {
    let x = (p[0] * 100.0).round().clamp(0.0, 511.0);
    let y = (p[1] * 100.0).round().clamp(1.0, REAR_LAMP_PACK - 1.0);
    [p[2], x * REAR_LAMP_PACK + y]
}

/// `rear_lamp_words` read back (as the shader reads them; None for 0).
pub fn rear_lamp_from_words(w: [f32; 2]) -> Option<[f32; 3]> {
    if w[1] <= 0.0 {
        return None;
    }
    let xi = (w[1] / REAR_LAMP_PACK).floor();
    Some([xi / 100.0, (w[1] - xi * REAR_LAMP_PACK) / 100.0, w[0]])
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
