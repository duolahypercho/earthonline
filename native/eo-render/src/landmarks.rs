//! the hand-built landmarks and the city's built edges and open
//! spaces (`eo-landmarks`) in the GPU-driven path.
//!
//! Every main draw, far item and skirt becomes static objects of the city
//! table (split into `CHUNK_M` plan cells, so the frustum, the Hi-Z test and
//! the cascade cull see them in pieces), drawn by the city pipelines with a
//! vertex-colour material of the registry's roughness and metalness and its
//! surface kind (`landmarks_surface.wgsl`: landmark-surface.js; the shore's
//! walls, wet ground and beach sand in `shore_surface.wgsl`), fogged with
//! the set's capped sight distance (`fogCapM`). One LOD group per landmark
//! (`GROUP_KIND`) holds the set's near/far switch on the GPU: the box
//! distance from the camera against `nearM` with `hysteresisM`
//! (landmark-set.js `update`). Near draws cast sun shadows; the far item and
//! the skirt do not (the JS far batch casts none).
//!
//! The wire members (main cables, suspender ropes) are their own small
//! pipeline (`landmarks_wire.wgsl`: landmark-surface.js `applyWireCoverage`):
//! prisms at the envelope radius, each pixel keeping the rope where its view
//! ray passes within max(true radius, `wireMinPixels` / 2 of a pixel) of the
//! axis, faded by the share of that width the rope covers. They are drawn
//! where the glass is drawn (after the water when the water pass is on) and
//! write no depth.

use crate::authored::{F_VERTEX_COLOUR, GpuLodGroup, GpuMaterial, MaterialTable};
use crate::scene::{GpuObject, ObjectKind, Scene, Vertex};
use bytemuck::{Pod, Zeroable};
use eo_landmarks::{LandmarkSet, Role};
use glam::Vec3;
use std::collections::HashMap;

/// Material flag: a landmark material, fogged with the set's cap (read in
/// look_light, which the props' and agents' shaders also call: bit 29 is free
/// in their tables, whose lens index uses bits 24-25); bits 25-27 the surface
/// kind (read only by draw.wgsl's `shade`).
pub const F_LANDMARK: u32 = 1 << 29;
pub const SURFACE_SHIFT: u32 = 25;
/// Material flag on the vertex-colour material: the terrain takes the wet margin at the waterline.
pub const F_WET_GROUND: u32 = 1 << 28;
pub const SURFACE_NONE: u32 = 0;
pub const SURFACE_PAINTED_STEEL: u32 = 1;
pub const SURFACE_CONCRETE: u32 = 2;
/// LOD group kind of a landmark (cull.wgsl `lod`).
pub const GROUP_KIND: u32 = 3;
/// Plan cell the draws are split into for culling, metres.
pub const CHUNK_M: f64 = 128.0;

/// The quay wall's concrete (renderer.js's seawall colour node).
pub const SURFACE_SEAWALL: u32 = 3;

/// the landmark buildings' surfaces (landmark-surface.js `precast`,
/// `finCurtain`, `gildedCladding`).
pub const SURFACE_PRECAST: u32 = 4;
pub const SURFACE_FIN_CURTAIN: u32 = 5;
pub const SURFACE_GILDED_CLADDING: u32 = 6;
/// the Golden Gate's far item: no surface of its own, only the towers'
/// floodlighting at night (`BridgeFeatures::night`).
pub const SURFACE_FLOODLIT_FAR: u32 = 7;

pub fn surface_code(kind: Option<&str>) -> u32 {
    match kind {
        Some("painted-steel") => SURFACE_PAINTED_STEEL,
        Some("concrete") => SURFACE_CONCRETE,
        Some("seawall") => SURFACE_SEAWALL,
        Some("precast") => SURFACE_PRECAST,
        Some("fin-curtain") => SURFACE_FIN_CURTAIN,
        Some("gilded-cladding") => SURFACE_GILDED_CLADDING,
        _ => SURFACE_NONE,
    }
}

/// One vertex of a wire member (52 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct WireVertex {
    pub pos: [f32; 3],
    /// The member's true radius (0: not a wire).
    pub radius: f32,
    pub axis: [f32; 3],
    /// The landmark's LOD group.
    pub group: u32,
    pub dir: [f32; 3],
    /// sRGB bytes of the linear vertex colour.
    pub color: u32,
    /// Index into the city material table.
    pub material: u32,
}

/// What the scene build added, for the renderer.
#[derive(Default)]
pub struct LandmarkStatic {
    pub group_base: u32,
    pub groups: Vec<GpuLodGroup>,
    pub wire_vertices: Vec<WireVertex>,
    pub wire_indices: Vec<u32>,
    pub objects: usize,
    pub near_triangles: usize,
    pub far_triangles: usize,
    pub wire_triangles: usize,
    pub always_triangles: usize,
    pub wall_near_triangles: usize,
    pub wall_far_triangles: usize,
    pub lights: Vec<eo_landmarks::LandmarkLight>,
    /// Landmark id -> (box min, box max), for the bench and captures.
    pub boxes: Vec<(String, [f32; 3], [f32; 3])>,
    /// (cutting the plan cells, growing the scene's arrays, pushing) in ms.
    pub append_ms: [f64; 3],
}

fn srgb_byte(c: f32) -> u32 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0).round() as u32
}

pub fn pack_colour(c: &[f32]) -> u32 {
    srgb_byte(c[0]) | (srgb_byte(c[1]) << 8) | (srgb_byte(c[2]) << 16) | (255 << 24)
}

/// The material of one landmark draw (`landmark`: the set's fog cap applies).
pub fn material(m: &eo_landmarks::data::Material, anchor: [f64; 2], landmark: bool) -> GpuMaterial {
    let mut g = GpuMaterial::vertex_colour();
    g.params = [m.roughness as f32, m.metalness as f32, anchor[0] as f32, anchor[1] as f32];
    g.flags = F_VERTEX_COLOUR | if landmark { F_LANDMARK } else { 0 } | (surface_code(m.surface.as_deref()) << SURFACE_SHIFT);
    if surface_code(m.surface.as_deref()) == SURFACE_PRECAST {
        // the precast surface's panel bay and storey (its
        // `surfaceOptions`, else the JS defaults) ride in two words a
        // vertex-colour material does not read (they are the kit's roughness
        // flattening and the thin glazing's absorption).
        let o = m.surface_options.as_ref();
        g.rough_flatten = o.and_then(|o| o["bayM"].as_f64()).unwrap_or(2.0) as f32;
        g.thin_absorption = o.and_then(|o| o["storeyM"].as_f64()).unwrap_or(3.9) as f32;
    }
    let kind = surface_code(m.surface.as_deref());
    if (kind == SURFACE_PAINTED_STEEL || kind == SURFACE_CONCRETE) && m.surface_options.as_ref().is_some_and(|o| o["bridge"] == serde_json::json!(true)) {
        // a bridge's own coat (eo-landmarks suspension.rs `finish_material`): the
        // word a vertex-colour material does not read marks it for lm_surface.
        g.rough_flatten = 1.0;
    }
    g
}

/// Append the set's static draws to the scene's arrays and table; their
/// materials to `materials`; their LOD groups numbered from `group_base`.
pub fn append(scene: &mut Scene, set: &LandmarkSet, materials: &mut MaterialTable, group_base: u32) -> LandmarkStatic {
    let mut out = LandmarkStatic { group_base, lights: set.lights.clone(), ..Default::default() };
    // The terrain (the vertex-colour material, kind 1) takes the wet margin.
    if let Some(m0) = materials.gpu.first_mut() {
        m0.flags |= F_WET_GROUND;
    }
    // Every static mesh's plan cells, cut over a few threads (most of the
    // work: the wall's relief), then pushed below in this same order.
    let mut meshes: Vec<&eo_landmarks::Frozen> = Vec::new();
    for l in &set.landmarks {
        meshes.extend(l.draws.iter().filter(|d| !d.material.wire).map(|d| &d.mesh));
    }
    if set.wall_material.is_some() {
        for w in &set.walls {
            meshes.push(&w.near);
            meshes.push(&w.far);
        }
    }
    meshes.extend(set.structures.iter().filter(|s| !s.mesh.is_empty()).map(|s| &s.mesh));
    let t = std::time::Instant::now();
    let chunks = chunk_meshes(&meshes);
    out.append_ms[0] = t.elapsed().as_secs_f64() * 1e3;
    // Grow the scene's arrays once (they hold the whole city: each doubling copies it).
    let t = std::time::Instant::now();
    let (nv, ni) = chunks.iter().flatten().fold((0, 0), |(v, i), c| (v + c.verts.len(), i + c.idx.len()));
    scene.vertices.reserve_exact(nv);
    scene.indices.reserve_exact(ni);
    scene.objects.reserve_exact(chunks.iter().map(|c| c.len()).sum());
    out.append_ms[1] = t.elapsed().as_secs_f64() * 1e3;
    let t = std::time::Instant::now();
    let mut chunks = chunks.into_iter();
    let mut next_chunks = move || chunks.next().expect("one chunk list per mesh");
    for (li, l) in set.landmarks.iter().enumerate() {
        let group = group_base + li as u32;
        let (mn, mx) = (Vec3::from(l.box_min), Vec3::from(l.box_max));
        let (c, h) = ((mn + mx) * 0.5, (mx - mn) * 0.5);
        // The box's half extents ride in the group's radius and two of its
        // words, bit cast (cull.wgsl `lod`, kind 3).
        out.groups.push(GpuLodGroup { center: c.to_array(), radius: h.x, ready: h.y.to_bits(), detailed: 0, kind: GROUP_KIND, level: h.z.to_bits() });
        out.boxes.push((l.id.clone(), l.box_min, l.box_max));
        let features = eo_landmarks::suspension::features();
        for d in &l.draws {
            let mut gm = material(&d.material, l.anchor, true);
            if features.night && d.key == "far" && l.id == "golden-gate-bridge" {
                gm.flags |= SURFACE_FLOODLIT_FAR << SURFACE_SHIFT;
            }
            let mat = materials.named(&format!("landmark:{}:{}", l.id, d.key), gm);
            let m = &d.mesh;
            if d.material.wire {
                let (axes, dirs, radii) = (m.axes.as_ref().unwrap(), m.dirs.as_ref().unwrap(), m.radii.as_ref().unwrap());
                let base = out.wire_vertices.len() as u32;
                // the west span's suspender ropes carry its LEDs (alpha 0: lm_wire2_fs's mark).
                let leds = features.night && features.wires && l.id == "bay-bridge-west";
                for v in 0..m.vertices() {
                    let mut color = pack_colour(&m.colors[3 * v..3 * v + 3]);
                    if leds && radii[v] > 0.0 && radii[v] < LED_ROPE_MAX_RADIUS_M {
                        color &= 0x00ff_ffff;
                    }
                    out.wire_vertices.push(WireVertex {
                        pos: [m.positions[3 * v], m.positions[3 * v + 1], m.positions[3 * v + 2]],
                        radius: radii[v],
                        axis: [axes[3 * v], axes[3 * v + 1], axes[3 * v + 2]],
                        group,
                        dir: [dirs[3 * v], dirs[3 * v + 1], dirs[3 * v + 2]],
                        color,
                        material: mat,
                    });
                }
                out.wire_indices.extend(m.indices.iter().map(|i| i + base));
                out.wire_triangles += m.triangles();
                continue;
            }
            let (kind, role) = match d.role {
                Role::Near if d.key != "skirt" => (ObjectKind::Landmark, crate::scene::LOD_LANDMARK_NEAR),
                Role::Near => (ObjectKind::LandmarkNoShadow, crate::scene::LOD_LANDMARK_NEAR),
                Role::Far => (ObjectKind::LandmarkNoShadow, crate::scene::LOD_LANDMARK_FAR),
                Role::Always => (ObjectKind::LandmarkNoShadow, crate::scene::LOD_ALWAYS),
            };
            match d.role {
                Role::Near => out.near_triangles += m.triangles(),
                Role::Far => out.far_triangles += m.triangles(),
                Role::Always => out.always_triangles += m.triangles(),
            }
            out.objects += push_chunks(scene, next_chunks(), kind, role, group, mat);
        }
    }
    // The shore edge: one group per wall chunk (the set's near rule on its
    // box), the chunk's relief tier near and the base tier far; no shadow
    // (the JS wall receives only).
    if let Some(wm) = &set.wall_material {
        let mat = materials.named("seawall", material(wm, [0.0, 0.0], false));
        for w in &set.walls {
            let group = group_base + out.groups.len() as u32;
            let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
            for p in w.near.positions.chunks_exact(3) {
                for k in 0..3 {
                    mn[k] = mn[k].min(p[k]);
                    mx[k] = mx[k].max(p[k]);
                }
            }
            let (c, h) = ((Vec3::from(mn) + Vec3::from(mx)) * 0.5, (Vec3::from(mx) - Vec3::from(mn)) * 0.5);
            out.groups.push(GpuLodGroup { center: c.to_array(), radius: h.x, ready: h.y.to_bits(), detailed: 0, kind: GROUP_KIND, level: h.z.to_bits() });
            out.objects += push_chunks(scene, next_chunks(), ObjectKind::LandmarkNoShadow, crate::scene::LOD_LANDMARK_NEAR, group, mat);
            out.objects += push_chunks(scene, next_chunks(), ObjectKind::LandmarkNoShadow, crate::scene::LOD_LANDMARK_FAR, group, mat);
            out.wall_near_triangles += w.near.triangles();
            out.wall_far_triangles += w.far.triangles();
        }
    }
    for s in set.structures.iter().filter(|s| !s.mesh.is_empty()) {
        if s.footway {
            // The plaza paving: the street surface's material (its concrete
            // scan) in the footway class, the JS palette tone as its tint.
            let mat = materials.street();
            let first = scene.objects.len();
            out.objects += push_chunks(scene, next_chunks(), ObjectKind::Street, crate::scene::LOD_ALWAYS, crate::scene::NO_GROUP, mat);
            let vfirst = scene.objects[first].base_vertex as usize;
            for v in &mut scene.vertices[vfirst..] {
                v.color = (v.color & 0x00ff_ffff) | ((crate::street::class::FOOTWAY as u32) << 24);
                v.normal = (v.normal & 0x00ff_ffff) | (((-127i8) as u8 as u32) << 24);
            }
            out.always_triangles += s.mesh.triangles();
            continue;
        }
        // a structure drawn in a kit finish takes the kit palette's material (textured, aged).
        // (`<finish>*<k>`: the finish at k times its texture repeat, authored::kit_repeat.)
        let kit = s.kit.as_deref().and_then(|k| materials.kit_finish(k));
        let mat = match kit {
            Some(m) => m,
            None => materials.named(&format!("structure:{}", s.name), material(&s.material, [0.0, 0.0], false)),
        };
        let kind = if s.cast { ObjectKind::Landmark } else { ObjectKind::LandmarkNoShadow };
        out.objects += push_chunks(scene, next_chunks(), kind, crate::scene::LOD_ALWAYS, crate::scene::NO_GROUP, mat);
        out.always_triangles += s.mesh.triangles();
    }
    out.append_ms[2] = t.elapsed().as_secs_f64() * 1e3;
    out
}

/// A wire member under this radius is a suspender rope (the ropes are
/// 0.035 m; the main cables 0.37 m and over, the cable bands and the ropes'
/// sockets 0.09 m and over), metres.
const LED_ROPE_MAX_RADIUS_M: f32 = 0.06;

/// One plan cell of a mesh as GPU vertices and indices, with its bounds.
pub struct Chunk {
    pub verts: Vec<Vertex>,
    pub idx: Vec<u32>,
    pub min: [f32; 3],
    pub max: [f32; 3],
}

/// `chunk_mesh` of every mesh, over a few threads; in the meshes' order.
pub fn chunk_meshes(meshes: &[&eo_landmarks::Frozen]) -> Vec<Vec<Chunk>> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 8);
    let next = AtomicUsize::new(0);
    let done: Vec<(usize, Vec<Chunk>)> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut mine = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= meshes.len() {
                            break;
                        }
                        mine.push((i, chunk_mesh(meshes[i])));
                    }
                    mine
                })
            })
            .collect();
        hs.into_iter().flat_map(|h| h.join().expect("landmark chunk thread panicked")).collect()
    });
    let mut out: Vec<Vec<Chunk>> = (0..meshes.len()).map(|_| Vec::new()).collect();
    for (i, c) in done {
        out[i] = c;
    }
    out
}

/// Push chunks as objects of the static table. Returns the objects pushed.
pub fn push_chunks(scene: &mut Scene, chunks: Vec<Chunk>, kind: ObjectKind, role: u32, group: u32, material: u32) -> usize {
    let n = chunks.len();
    for Chunk { verts, idx, min: mn, max: mx } in chunks {
        let base_vertex = scene.vertices.len() as i32;
        let first_index = scene.indices.len() as u32;
        scene.vertices.extend_from_slice(&verts);
        scene.indices.extend_from_slice(&idx);
        let mut o = GpuObject::new(kind, mn, mx, first_index, idx.len() as u32, base_vertex);
        o.material = material;
        o.lod_group = group;
        o.lod_role = role;
        scene.objects.push(o);
        for k in 0..3 {
            scene.world_min[k] = scene.world_min[k].min(mn[k]);
            scene.world_max[k] = scene.world_max[k].max(mx[k]);
        }
        scene.stats.objects += 1;
        scene.stats.triangles += (idx.len() / 3) as u64;
        scene.stats.vertices += verts.len();
        scene.stats.indices += idx.len();
    }
    n
}

/// A mesh's triangles split by the plan cell (`CHUNK_M`) of their centroid,
/// each cell's vertices packed once, in first-use order.
pub fn chunk_mesh(m: &eo_landmarks::Frozen) -> Vec<Chunk> {
    let mut cells: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
    let mut order: Vec<(i64, i64)> = Vec::new();
    for t in m.indices.chunks_exact(3) {
        let (mut x, mut z) = (0.0f64, 0.0f64);
        for &i in t {
            x += m.positions[3 * i as usize] as f64;
            z += m.positions[3 * i as usize + 2] as f64;
        }
        let key = ((x / 3.0 / CHUNK_M).floor() as i64, (z / 3.0 / CHUNK_M).floor() as i64);
        let e = cells.entry(key).or_insert_with(|| {
            order.push(key);
            Vec::new()
        });
        e.extend_from_slice(t);
    }
    let mut out = Vec::with_capacity(order.len());
    let mut remap = vec![u32::MAX; m.vertices()];
    // Most meshes carry a handful of colours: pack each once.
    let mut packed: Vec<([f32; 3], u32)> = Vec::new();
    let mut pack = |c: &[f32]| -> u32 {
        let key = [c[0], c[1], c[2]];
        if let Some((_, p)) = packed.iter().rev().take(8).find(|(k, _)| *k == key) {
            return *p;
        }
        let p = pack_colour(c);
        packed.push((key, p));
        p
    };
    for key in order {
        let tris = &cells[&key];
        let mut verts: Vec<Vertex> = Vec::new();
        let mut used: Vec<u32> = Vec::new();
        let mut idx: Vec<u32> = Vec::with_capacity(tris.len());
        for &i in tris {
            if remap[i as usize] == u32::MAX {
                let v = i as usize;
                verts.push(Vertex {
                    pos: [m.positions[3 * v], m.positions[3 * v + 1], m.positions[3 * v + 2]],
                    normal: eo_assets::glb::pack_normal(Vec3::new(m.normals[3 * v], m.normals[3 * v + 1], m.normals[3 * v + 2])),
                    color: pack(&m.colors[3 * v..3 * v + 3]),
                });
                remap[v] = (verts.len() - 1) as u32;
                used.push(i);
            }
            idx.push(remap[i as usize]);
        }
        for i in used {
            remap[i as usize] = u32::MAX;
        }
        let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
        for v in &verts {
            for k in 0..3 {
                mn[k] = mn[k].min(v.pos[k]);
                mx[k] = mx[k].max(v.pos[k]);
            }
        }
        out.push(Chunk { verts, idx, min: mn, max: mx });
    }
    out
}

/// WGSL constants of the landmark surfaces (landmark-surface.js `LANDMARK_SURFACES`,
/// the set's fog cap, the wire rules), generated from the JS export.
pub fn landmarks_constants_wgsl() -> String {
    let js = eo_landmarks::LandmarksJs::get();
    let k = &js.surfaces.kinds;
    let f = |v: f64| format!("{v:.6}");
    let n = |kind: &str, key: &str| f(eo_landmarks::data::num(&k[kind], key));
    let ps = &k["painted-steel"];
    let plate = eo_landmarks::data::pair(ps, "plateM");
    let mut s = String::from("\n// generated by landmarks.rs landmarks_constants_wgsl (eo-landmarks data/landmarks-js.json, the JS tables)\n");
    for (name, v) in [
        ("LM_F_LANDMARK", format!("{}u", F_LANDMARK)),
        ("LM_F_WET_GROUND", format!("{}u", F_WET_GROUND)),
        ("LM_SURFACE_SHIFT", format!("{}u", SURFACE_SHIFT)),
        ("LM_SEA_LEVEL_Y", f(js.sea_level_y)),
        ("LM_FOG_CAP_M", f(js.rules.fog_cap_m)),
        ("LM_WIRE_MIN_PIXELS", f(js.surfaces.wire_min_pixels)),
        ("LM_WIRE_ENVELOPE_M", f(js.wire_drawn_radius_m)),
        ("LM_PS_PATCH_M", n("painted-steel", "patchM")),
        ("LM_PS_PATCH_TONE", n("painted-steel", "patchTone")),
        ("LM_PS_FINE_PATCH_M", n("painted-steel", "finePatchM")),
        ("LM_PS_FINE_PATCH_TONE", n("painted-steel", "finePatchTone")),
        ("LM_PS_GRIME_HEIGHT_M", n("painted-steel", "grimeHeightM")),
        ("LM_PS_GRIME_TONE", n("painted-steel", "grimeTone")),
        ("LM_PS_STREAK_TONE", n("painted-steel", "streakTone")),
        ("LM_PS_CHALK_TONE", n("painted-steel", "chalkTone")),
        ("LM_PS_ROUGH_SPREAD", n("painted-steel", "roughnessSpread")),
        ("LM_PS_CHALK_ROUGH", n("painted-steel", "chalkRoughness")),
        ("LM_PS_PLATE_U", f(plate[0])),
        ("LM_PS_PLATE_V", f(plate[1])),
        ("LM_PS_SEAM_DEPTH", n("painted-steel", "seamDepthM")),
        ("LM_PS_RIVET_PITCH", n("painted-steel", "rivetPitchM")),
        ("LM_PS_RIVET_RADIUS", n("painted-steel", "rivetRadiusM")),
        ("LM_PS_RIVET_HEIGHT", n("painted-steel", "rivetHeightM")),
        ("LM_CO_PATCH_M", n("concrete", "patchM")),
        ("LM_CO_PATCH_TONE", n("concrete", "patchTone")),
        ("LM_CO_LIFT_M", n("concrete", "liftM")),
        ("LM_CO_FORM_M", n("concrete", "formM")),
        ("LM_CO_JOINT_DEPTH", n("concrete", "jointDepthM")),
        ("LM_CO_WET_BAND_M", n("concrete", "wetBandM")),
        ("LM_CO_WET_TONE", n("concrete", "wetTone")),
        ("LM_CO_STREAK_TONE", n("concrete", "streakTone")),
        ("LM_CO_ROUGH_SPREAD", n("concrete", "roughnessSpread")),
        // precast, fin-curtain, gilded-cladding.
        ("LM_PC_PATCH_M", n("precast", "patchM")),
        ("LM_PC_PATCH_TONE", n("precast", "patchTone")),
        ("LM_PC_JOINT_DEPTH", n("precast", "jointDepthM")),
        ("LM_PC_STREAK_TONE", n("precast", "streakTone")),
        ("LM_PC_ROUGH_SPREAD", n("precast", "roughnessSpread")),
        ("LM_FC_MODULE_M", n("fin-curtain", "moduleM")),
        ("LM_FC_FIN_SHARE", n("fin-curtain", "finShare")),
        ("LM_FC_FIN_ROUGH", n("fin-curtain", "finRoughness")),
        ("LM_FC_GLASS_ROUGH", n("fin-curtain", "glassRoughness")),
        ("LM_FC_GLASS_METAL", n("fin-curtain", "glassMetalness")),
        ("LM_FC_FIN_R", f(eo_landmarks::geometry::srgb_hex(eo_landmarks::data::hex(&k["fin-curtain"], "finSrgb"))[0])),
        ("LM_FC_FIN_G", f(eo_landmarks::geometry::srgb_hex(eo_landmarks::data::hex(&k["fin-curtain"], "finSrgb"))[1])),
        ("LM_FC_FIN_B", f(eo_landmarks::geometry::srgb_hex(eo_landmarks::data::hex(&k["fin-curtain"], "finSrgb"))[2])),
        ("LM_GC_GILT_ROUGH", n("gilded-cladding", "giltRoughness")),
        ("LM_GC_GILT_METAL", n("gilded-cladding", "giltMetalness")),
        ("LM_GC_WARM_FROM", n("gilded-cladding", "warmFrom")),
        ("LM_GC_WARM_TO", n("gilded-cladding", "warmTo")),
        ("LM_GC_PATCH_M", n("gilded-cladding", "patchM")),
        ("LM_GC_PATCH_TONE", n("gilded-cladding", "patchTone")),
    ] {
        let ty = if v.ends_with('u') { "u32" } else { "f32" };
        s.push_str(&format!("const {name}: {ty} = {v};\n"));
    }
    s.push_str(&bridges_wgsl(js));
    s
}

/// the bridges' night constants (eo-landmarks lights.rs): the Golden Gate
/// towers' floodlighting and the west span's suspender LEDs.
fn bridges_wgsl(js: &eo_landmarks::LandmarksJs) -> String {
    let feat = eo_landmarks::suspension::features();
    let fl = eo_landmarks::lights::gg_floodlit(js);
    let (bo, ba) = eo_landmarks::lights::bay_axis(js);
    let v2 = |p: [f64; 2]| format!("vec2<f32>({:.6}, {:.6})", p[0], p[1]);
    let mut s = String::new();
    s.push_str(&format!("const LM_FLOOD_ON: bool = {};\nconst LM_LED_ON: bool = {};\n", feat.night, feat.night && feat.wires));
    s.push_str(&format!("const LM_AIR_ON: bool = {};\nconst LM_FLOODS2_ON: bool = {};\n", feat.air && feat.wires, feat.floods && feat.night));
    s.push_str(&format!("const LM_FINISH_ON: bool = {};\nconst LM_CURTAIN_ON: bool = {};\n", feat.finish, feat.curtain && feat.wires));
    let pitch = |d: &serde_json::Value| d["suspenderSpacingM"].as_f64().unwrap_or(15.24);
    s.push_str(&format!(
        "const LM_GG_SUSP_PITCH: f32 = {:.4};\nconst LM_BAY_SUSP_PITCH: f32 = {:.4};\nconst LM_SEA_ALBEDO: f32 = {:.4};\n",
        pitch(&eo_landmarks::golden_gate::data(js).dimensions),
        pitch(&eo_landmarks::bay_bridge::data(js).dimensions),
        eo_landmarks::suspension::SEA_ALBEDO
    ));
    s.push_str(&format!("const LM_GG_ORIGIN: vec2<f32> = {};\nconst LM_GG_AXIS: vec2<f32> = {};\n", v2(fl.origin), v2(fl.axis)));
    for (name, v) in [
        ("LM_GG_SPAN", fl.span),
        ("LM_GG_HP", fl.half_plane),
        ("LM_GG_DECK_Y", fl.deck_y),
        ("LM_GG_TOP_Y", fl.top_y),
        ("LM_GG_HALF_ALONG", fl.half_along),
        ("LM_GG_HALF_ACROSS", fl.half_across),
        ("LM_FLOOD_LUX", fl.lux),
        ("LM_LED_CD", eo_landmarks::lights::LED_CD),
        ("LM_LED_PITCH_M", eo_landmarks::lights::LED_PITCH_M),
    ] {
        s.push_str(&format!("const {name}: f32 = {v:.6};\n"));
    }
    s.push_str(&format!("const LM_FLOOD_RGB: vec3<f32> = vec3<f32>({:.6}, {:.6}, {:.6});\n", fl.colour[0], fl.colour[1], fl.colour[2]));
    s.push_str(&format!("const LM_BAY_ORIGIN: vec2<f32> = {};\nconst LM_BAY_AXIS: vec2<f32> = {};\n", v2(bo), v2(ba)));
    s.push_str(&gg_floods_wgsl(js));
    // the decks' lines for the analytic deck shadow (`finish`; eo-landmarks suspension.rs `deck_line`):
    // off when the bridges were not built before the shaders.
    let (gg, bay) = (eo_landmarks::suspension::deck_line("golden-gate-bridge"), eo_landmarks::suspension::deck_line("bay-bridge-west"));
    let arr = |v: Option<[f64; 8]>| format!("array<f32, 8>({})", v.unwrap_or([0.0; 8]).iter().map(|x| format!("{x:.4}")).collect::<Vec<_>>().join(", "));
    s.push_str(&format!(
        "const LM_DECKSHADE_ON: bool = {};\nconst LM_GG_DECK: array<f32, 8> = {};\nconst LM_BAY_DECK: array<f32, 8> = {};\nconst LM_BAY_ROUND_M: f32 = {:.3};\n",
        feat.finish && gg.is_some() && bay.is_some(),
        arr(gg),
        arr(bay),
        eo_landmarks::bay_bridge::CORNER_ROUNDING_M
    ));
    // the fourth pass's shading (`shade2`, `dielectric`): the Golden Gate's own sky
    // occlusion (its deck line and its legs' stages) and its sea light counted once.
    let g = eo_landmarks::golden_gate::data(js);
    let w = js.sea_level_y;
    let stages = g.leg_stages.iter().map(|st| format!("vec4<f32>({:.4}, {:.4}, {:.4}, {:.4})", w + st[0], w + st[1], st[2] / 2.0, st[3] / 2.0)).collect::<Vec<_>>();
    // the chainages between which the Golden Gate stands over the strait (the
    // shore-end pylon S1 to the Marin anchorage): the sea's light replaces the
    // ground's there, the land's stays outside.
    let site = eo_landmarks::golden_gate::Site::new(&g);
    let ch = |k: &str| site.chainage(g.supports[k].centre[0], g.supports[k].centre[1]);
    s.push_str(&format!("const LM_GG_WATER_C: vec2<f32> = vec2<f32>({:.3}, {:.3});\n", ch("archNorth"), ch("northAnchor")));
    s.push_str(&format!(
        "const LM_SHADE2_ON: bool = {};\nconst LM_DIELECTRIC_ON: bool = {};\nconst LM_GG_STAGE_N: u32 = {}u;\nconst LM_GG_STAGES: array<vec4<f32>, {}> = array<vec4<f32>, {}>({});\n",
        feat.shade2 && feat.finish && gg.is_some(),
        feat.dielectric && feat.finish,
        stages.len(),
        stages.len(),
        stages.len(),
        stages.join(", ")
    ));
    s
}

/// A Golden Gate tower's 12 placed floods and the members that shadow them
/// (`BridgeFeatures::floods`, eo-landmarks lights.rs `gg_tower_floods`), in
/// the tower's frame (c along the span, s across it, y up).
fn gg_floods_wgsl(js: &eo_landmarks::LandmarksJs) -> String {
    let (floods, fl) = eo_landmarks::lights::gg_tower_floods(js);
    let below = eo_landmarks::lights::gg_floodlit_below(js);
    let v3 = |p: [f64; 3]| format!("vec3<f32>({:.5}, {:.5}, {:.5})", p[0], p[1], p[2]);
    let list = |f: &dyn Fn(&eo_landmarks::lights::Flood) -> [f64; 3]| floods.iter().map(|x| v3(f(x))).collect::<Vec<_>>().join(", ");
    let n = floods.len();
    let n_above = floods.iter().filter(|x| !x.below).count();
    let g = eo_landmarks::golden_gate::data(js);
    let w = js.sea_level_y;
    // the portal struts (with `floods`, the ones under the deck too: the below-roadway lights'):
    // (y0, y1, half along, half span between the legs' centre planes, less half a leg).
    let all = eo_landmarks::suspension::features().floods;
    let struts: Vec<String> = g.struts.iter().filter(|st| all || w + st[0] > fl.deck_y).map(|st| format!("vec4<f32>({:.4}, {:.4}, {:.4}, {:.4})", w + st[0], w + st[1], st[2] / 2.0, fl.half_plane - 0.5 * fl.half_across)).collect();
    let ns = struts.len().max(1);
    let struts = if struts.is_empty() { "vec4<f32>(0.0, 0.0, 0.0, 0.0)".to_string() } else { struts.join(", ") };
    let mut s = String::new();
    s.push_str(&format!("const LM_GG_FLOOD_NA: u32 = {n_above}u;\nconst LM_FLOOD_LUX_B: f32 = {:.5};\nconst LM_GG_UNDER_Y: f32 = {:.4};\nconst LM_GG_BASE_Y: f32 = {:.4};\n", below.lux, below.under_y, below.base_y));
    s.push_str(&format!("const LM_GG_FLOOD_N: u32 = {n}u;\nconst LM_GG_FLOOD_POS: array<vec3<f32>, {n}> = array<vec3<f32>, {n}>({});\n", list(&|x| x.pos)));
    s.push_str(&format!("const LM_GG_FLOOD_AIM: array<vec3<f32>, {n}> = array<vec3<f32>, {n}>({});\n", list(&|x| x.aim)));
    s.push_str(&format!("const LM_GG_STRUT_N: u32 = {ns}u;\nconst LM_GG_STRUTS: array<vec4<f32>, {ns}> = array<vec4<f32>, {ns}>({struts});\n"));
    let (inner, outer) = (eo_landmarks::lights::FLOOD_INNER_DEG.to_radians().cos(), eo_landmarks::lights::FLOOD_OUTER_DEG.to_radians().cos());
    let [p0, p1] = eo_landmarks::lights::FLOOD_PROFILE;
    s.push_str(&format!("const LM_FLOOD_COS_IN: f32 = {inner:.6};\nconst LM_FLOOD_COS_OUT: f32 = {outer:.6};\nconst LM_FLOOD_PROFILE: vec2<f32> = vec2<f32>({p0:.4}, {p1:.4});\n"));
    s
}

/// Marks a landmark light in `GpuLampLight::kind` (the props' lamps carry their `eo_props::Model`).
pub const LIGHT_KIND_LANDMARK: u32 = 0xffff_ff00;

/// The landmarks' lights as the street lamps' GPU records (props2
/// `GpuLampLight`, the layout the night lighting reads from `PropsGpu::lights_buffer`).
pub fn gpu_lights(lights: &[eo_landmarks::LandmarkLight]) -> Vec<crate::props::GpuLampLight> {
    lights
        .iter()
        .map(|l| crate::props::GpuLampLight {
            pos: [l.pos[0] as f32, l.pos[1] as f32, l.pos[2] as f32],
            range: l.range_m,
            colour: l.colour_linear,
            intensity: l.intensity,
            angle: l.angle,
            penumbra: l.penumbra,
            decay: l.decay,
            source_lift: l.source_lift_m,
            kind: LIGHT_KIND_LANDMARK,
            pad: [0; 3],
        })
        .collect()
}

/// The landmarks' lights as lamps of the sky slice's clustered gather (the
/// static list beside the fallback street lamps, `SkyGpu::set_lamps`: the
/// clusters, the luminaire sprites, the glitter in the bay). Each is the JS
/// lamp practical as the props lamps are read (sky_lights.wgsl
/// `lights_props`): the source `source_lift_m` over the light, aimed down, its
/// cone and penumbra; the intensity from the JS's light units to candela
/// (`props_k = lamp_k / lamp_units_per_lux` there, `lamp_k` here). An omni
/// light (cone >= 90 deg) is a point light; a spot is shaded through the
/// street luminaire's cone. The JS bridges carry none, so this is empty.
pub fn sky_lamps(lights: &[eo_landmarks::LandmarkLight]) -> Vec<eo_sky::lights::LampLight> {
    use eo_sky::lights::{LampKind, LampLight};
    lights
        .iter()
        .map(|l| {
            let angle = l.angle as f64;
            LampLight {
                pos: [l.pos[0], l.pos[1] + l.source_lift_m as f64, l.pos[2]],
                candela: l.intensity as f64 / eo_sky::night::lamp_units_per_lux(),
                color: l.colour_linear.map(|c| c as f64),
                range: l.range_m as f64,
                dir: [0.0, -1.0, 0.0],
                cos_outer: angle.cos(),
                cos_inner: (angle * (1.0 - l.penumbra as f64)).cos(),
                kind: if angle >= std::f64::consts::FRAC_PI_2 - 1e-6 { LampKind::Point } else { LampKind::Street },
            }
        })
        .collect()
}

/// The set's near rule for cull.wgsl's `lod` (landmark-registry.js `LANDMARK_SET_RULES`).
pub fn cull_constants_wgsl() -> String {
    let r = &eo_landmarks::LandmarksJs::get().rules;
    format!("\n// generated by landmarks.rs (LANDMARK_SET_RULES)\nconst LM_NEAR_M: f32 = {:.3};\nconst LM_HYSTERESIS_M: f32 = {:.3};\n", r.near_m, r.hysteresis_m)
}

/// The landmarks' WGSL appended to the city shader (look.rs `city_shader_source`):
/// the landmark surfaces, then the shore's ground and wall surfaces
/// (`shore_surface.wgsl`, which uses the former's helpers).
pub fn city_wgsl() -> String {
    landmarks_constants_wgsl() + include_str!("landmarks_surface.wgsl") + include_str!("shore_surface.wgsl")
}

/// The wire members on the GPU.
pub struct LandmarksGpu {
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    index_count: u32,
    pipeline: wgpu::RenderPipeline,
    /// `BridgeFeatures::wires`: the members over the resolved frame (single
    /// sample, depth read only), instead of `pipeline` in the glass pass.
    overlay: Option<wgpu::RenderPipeline>,
    bg: wgpu::BindGroup,
    pub bytes: u64,
}

pub struct WireInit<'a> {
    pub device: &'a wgpu::Device,
    pub draw_bgl: &'a wgpu::BindGroupLayout,
    pub main_bgl: &'a wgpu::BindGroupLayout,
    pub group_buf: &'a wgpu::Buffer,
    pub sample_count: u32,
    pub vertices: &'a [WireVertex],
    pub indices: &'a [u32],
    /// The atmosphere's sky-view table (`SkyGpu::atmo_tables`), for the members' aerial perspective.
    pub sky_view: &'a wgpu::TextureView,
}

impl LandmarksGpu {
    pub fn new(w: &WireInit) -> Option<Self> {
        use wgpu::util::DeviceExt;
        if w.indices.is_empty() {
            return None;
        }
        let d = w.device;
        let vbuf = d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("landmark-wires-v"), contents: bytemuck::cast_slice(w.vertices), usage: wgpu::BufferUsages::VERTEX });
        let ibuf = d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("landmark-wires-i"), contents: bytemuck::cast_slice(w.indices), usage: wgpu::BufferUsages::INDEX });
        let bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("landmark-wires-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                // the sky-view table and its sampler (the overlay's aerial perspective).
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let lin = d.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("landmark-wires-sky"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("landmark-wires-bg"),
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: w.group_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(w.sky_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&lin) },
            ],
        });
        let src = format!("{}\n{}", crate::look::city_shader_source(), include_str!("landmarks_wire.wgsl"));
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("landmarks_wire.wgsl"), source: wgpu::ShaderSource::Wgsl(src.into()) });
        let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("landmark-wires-layout"), bind_group_layouts: &[Some(w.draw_bgl), Some(w.main_bgl), Some(&bgl)], immediate_size: 0 });
        let attrs = [
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32, offset: 12, shader_location: 1 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 16, shader_location: 2 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Uint32, offset: 28, shader_location: 3 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 32, shader_location: 4 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Unorm8x4, offset: 44, shader_location: 5 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Uint32, offset: 48, shader_location: 6 },
        ];
        let pipeline = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("landmark-wires"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("lm_wire_vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<WireVertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attrs })],
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: crate::DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState { count: w.sample_count, ..Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("lm_wire_fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: crate::look::HDR_FORMAT, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        // the overlay: one sample (the resolved frame), the same groups and vertices.
        let overlay = eo_landmarks::suspension::features().wires.then(|| {
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("landmark-wires-overlay"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("lm_wire2_vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<WireVertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attrs })],
                },
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: crate::DEPTH_FORMAT,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::Greater),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState { count: 1, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("lm_wire2_fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: crate::look::HDR_FORMAT, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::COLOR })],
                }),
                multiview_mask: None,
                cache: None,
            })
        });
        let bytes = vbuf.size() + ibuf.size();
        Some(Self { vbuf, ibuf, index_count: w.indices.len() as u32, pipeline, overlay, bg, bytes })
    }

    /// The members are drawn after the atmosphere (`draw_overlay`) rather than in the glass pass.
    pub fn overlay(&self) -> bool {
        self.overlay.is_some()
    }

    /// In a single-sample pass over the resolved frame whose groups 0 and 1
    /// are the city's: the members at every distance (`lm_wire2_fs`).
    pub fn draw_overlay(&self, pass: &mut wgpu::RenderPass<'_>) {
        let Some(p) = &self.overlay else { return };
        pass.set_pipeline(p);
        pass.set_bind_group(2, &self.bg, &[]);
        pass.set_vertex_buffer(0, self.vbuf.slice(..));
        pass.set_index_buffer(self.ibuf.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.index_count, 0, 0..1);
    }

    /// Inside a city or water pass whose groups 0 and 1 are the city's: the
    /// wires (the caller re-binds its own vertex and index buffers after).
    pub fn draw_wires(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(2, &self.bg, &[]);
        pass.set_vertex_buffer(0, self.vbuf.slice(..));
        pass.set_index_buffer(self.ibuf.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.index_count, 0, 0..1);
    }
}
