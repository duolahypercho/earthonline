//! The traced scene (stage R2, split out of the diffuse light's pass): the
//! acceleration structures every ray-traced pass shares (`--rt-gi`,
//! rt_gi.rs, and the reflections), built from the renderer's own vertex and
//! index arena (no copy of the geometry), its geometry table (what a hit
//! reads: material, kind) and the kit layers' mean albedo; one bind group
//! (`RtScene::bind_group`, group 0 of every tracing pipeline) and the WGSL
//! that reads it (rt_scene.wgsl: the hit's albedo, its Lambert light, the
//! sun's visibility).
//!
//! What is traced:
//!
//! * static cells (`RtKey::Cell`, 256 m): the terrain, the street's flat far
//!   strips and the plazas, the grade-separated roads' static level, the
//!   landmarks' near and always-drawn meshes, from the renderer's initial
//!   object table; built when the camera comes within `scene_radius_m`;
//! * kit packs (`RtKey::Kit`): each resident rung of a 128 m kit tile, as
//!   kit.rs reports it resident or evicted, one geometry a building; the near
//!   rung stands in for the mid one where both are built;
//! * hand-built models (`RtKey::Model`), as they are uploaded.
//!
//! Agents, props, trees, water and the street's near packs (kerbs, paint)
//! are not in it. One top-level structure (TLAS) over what lies within
//! `scene_radius_m`, rebuilt when that set changes; BLAS builds are spread
//! over frames (`build_triangles` a frame, nearest first). `update` runs once
//! a frame, before the first pass that traces.
//!
//! The device: `set_wanted` (eo-app, before any `GpuContext`) asks for the
//! experimental ray-query feature, which `GpuContext::new` requests only on
//! Metal (the one backend tested) where the adapter has it; without it no
//! tracing pass exists and eo-app says so in one line.

use crate::kit::RangeAlloc;
use crate::scene::{GpuObject, LOD_ALWAYS, LOD_LANDMARK_NEAR, LOD_STREET_FAR, Vertex};
use glam::Vec3;
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use wgpu::util::DeviceExt;

static WANTED: AtomicBool = AtomicBool::new(false);

/// Ask for the ray-query device feature (eo-app, before any `GpuContext`):
/// `GpuContext::new` requests it where the adapter has it on Metal.
pub fn set_wanted(on: bool) {
    WANTED.store(on, Ordering::Relaxed);
}

pub fn wanted() -> bool {
    WANTED.load(Ordering::Relaxed)
}

/// Static cells' size (m).
pub const CELL_M: f32 = 256.0;
/// The TLAS's capacity (instances).
pub const MAX_INSTANCES: u32 = 4096;
/// Geometry table entries (16 bytes each); a TLAS instance's custom data is
/// its first entry, 24 bits.
pub const TABLE_ENTRIES: u32 = 1 << 20;
const STRIDE: u64 = std::mem::size_of::<Vertex>() as u64;

/// The scene's tunables (`from_env`: `EO_RT_RADIUS`, `EO_RT_DROP`,
/// `EO_RT_BUILD`, `EO_RT_KIT_NEAR`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RtSceneConfig {
    /// Instances within this of the camera (m, to their bounds); BLASes dropped past `drop_radius_m`.
    pub scene_radius_m: f32,
    pub drop_radius_m: f32,
    /// Triangles built into BLASes a frame at most (the nearest first; one larger is built alone).
    pub build_triangles: u64,
    /// The kit's near rung in place of its mid rung where both are resident.
    pub kit_near: bool,
}

impl Default for RtSceneConfig {
    fn default() -> Self {
        Self { scene_radius_m: 420.0, drop_radius_m: 640.0, build_triangles: 400_000, kit_near: true }
    }
}

impl RtSceneConfig {
    pub fn from_env() -> Self {
        let mut c = Self::default();
        let get = |k: &str| std::env::var(format!("EO_RT_{k}")).ok().and_then(|s| s.parse::<f64>().ok());
        if let Some(x) = get("RADIUS") {
            c.scene_radius_m = x as f32;
        }
        if let Some(x) = get("DROP") {
            c.drop_radius_m = x as f32;
        }
        if let Some(x) = get("BUILD") {
            c.build_triangles = x.max(1.0) as u64;
        }
        if let Some(x) = get("KIT_NEAR") {
            c.kit_near = x > 0.5;
        }
        c.drop_radius_m = c.drop_radius_m.max(c.scene_radius_m);
        c
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({ "scene_radius_m": self.scene_radius_m, "drop_radius_m": self.drop_radius_m, "build_triangles": self.build_triangles, "kit_near": self.kit_near })
    }
}

/// One geometry of a BLAS: an indexed range of the shared arena (absolute
/// vertex and index positions) and what its hit reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RtGeom {
    pub base_vertex: u32,
    pub vertex_count: u32,
    pub first_index: u32,
    pub index_count: u32,
    /// The object's material (the kit palette's first entry for a kit building).
    pub material: u32,
    /// `ObjectKind` (bits 0-7 of the object's kind).
    pub kind: u32,
}

/// What a BLAS is built from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RtKey {
    /// A static cell (`CELL_M` grid).
    Cell(i32, i32),
    /// A kit tile's rung (kit.rs `RUNG_NEAR`, `RUNG_MID`).
    Kit(u32, u8),
    /// A hand-built model (its first object).
    Model(u32),
}

struct Built {
    blas: wgpu::Blas,
    table_at: u32,
    table_len: u32,
}

struct Entry {
    geoms: Vec<RtGeom>,
    min: Vec3,
    max: Vec3,
    triangles: u64,
    built: Option<Built>,
}

impl Entry {
    fn distance(&self, p: Vec3) -> f32 {
        (self.min - p).max(p - self.max).max(Vec3::ZERO).length()
    }
}

/// What the pass did, for the bench JSON (`RtGi::stats`).

/// What the scene did, for the bench JSON (`RtScene::stats`).
#[derive(Clone, Copy, Debug, Default)]
pub struct RtSceneStats {
    pub updates: u64,
    pub entries: usize,
    pub blas_built: usize,
    pub blas_triangles: u64,
    pub instances: usize,
    pub instance_triangles: u64,
    pub tlas_builds: u64,
    pub blas_builds: u64,
    pub built_triangles: u64,
    pub most_triangles_a_frame: u64,
    pub blas_dropped: u64,
}

impl RtSceneStats {
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "updates": self.updates, "entries": self.entries,
            "blas_built": self.blas_built, "blas_triangles": self.blas_triangles,
            "instances": self.instances, "instance_triangles": self.instance_triangles,
            "tlas_builds": self.tlas_builds, "blas_builds": self.blas_builds, "built_triangles": self.built_triangles,
            "most_triangles_built_a_frame": self.most_triangles_a_frame, "blas_dropped": self.blas_dropped,
        })
    }
}

struct SceneState {
    entries: HashMap<RtKey, Entry>,
    table: RangeAlloc,
    /// The TLAS's instances as last built (key order).
    instances: Vec<RtKey>,
    tlas_live: bool,
    stats: RtSceneStats,
}

/// The traced scene (made only with a tracing pass on a device with ray queries).
pub struct RtScene {
    pub config: RtSceneConfig,
    device: wgpu::Device,
    queue: wgpu::Queue,
    vertex_buf: wgpu::Buffer,
    index_buf: wgpu::Buffer,
    geo_buf: wgpu::Buffer,
    layer_mean_buf: wgpu::Buffer,
    tlas: Mutex<wgpu::Tlas>,
    bgl: wgpu::BindGroupLayout,
    bg: wgpu::BindGroup,
    state: Mutex<SceneState>,
    /// Bytes allocated on the GPU (the table and the means; the BLASes are counted by triangle).
    pub bytes: u64,
}

/// What the renderer hands the scene at its creation.
pub struct RtSceneInit<'a> {
    pub vertex_buf: &'a wgpu::Buffer,
    pub index_buf: &'a wgpu::Buffer,
    pub material_buf: &'a wgpu::Buffer,
    pub look_buf: &'a wgpu::Buffer,
    /// Kit map layers (the mean albedo table's length).
    pub kit_layers: u32,
}

/// rt_scene.wgsl: group 0 of every tracing pipeline and the hit's shading.
pub const SCENE_WGSL: &str = include_str!("rt_scene.wgsl");

/// A tracing pass's shader source: the ray-query extension, the look's
/// common code, the street constants, the scene (rt_scene.wgsl), then `pass`.
pub fn module_source(pass: &str) -> String {
    format!("enable wgpu_ray_query;\n{}\n{}\n{}\n{}", crate::look::look_common_source(), crate::look::street_constants_wgsl(), SCENE_WGSL, pass)
}

/// Whether a static object of the initial table is traced (`RtKey::Cell`):
/// the terrain, the street's far strips and plazas, the structures' static
/// level, the landmarks' near and always-drawn meshes, the plain extrusions.
pub fn static_object_traced(o: &GpuObject) -> bool {
    let role = o.lod_role;
    match o.kind & 255 {
        1 => true,
        6 | 7 | 11 | 12 => role == LOD_STREET_FAR || role == LOD_ALWAYS,
        9 | 10 => role == LOD_LANDMARK_NEAR || role == LOD_ALWAYS,
        0 => role == LOD_ALWAYS,
        _ => false,
    }
}

fn cell_of(min: [f32; 3], max: [f32; 3]) -> (i32, i32) {
    let c = (Vec3::from(min) + Vec3::from(max)) * 0.5;
    ((c.x / CELL_M).floor() as i32, (c.z / CELL_M).floor() as i32)
}

/// The static cells of an initial object table: each traced object
/// (`static_object_traced`) with its vertex count from its indices (where
/// they are in `indices`; an object written later, the structures' static
/// level, gets the rest of the arena from its base vertex, which the build
/// validates and Metal does not read for indexed geometry).
pub fn static_cells(objects: &[GpuObject], indices: &[u32], vertex_capacity: u32) -> Vec<(RtKey, Vec<RtGeom>, Vec3, Vec3)> {
    let mut cells: HashMap<(i32, i32), (Vec<RtGeom>, Vec3, Vec3)> = HashMap::new();
    for o in objects {
        if o.index_count < 3 || o.copy != 0 || !static_object_traced(o) || o.base_vertex < 0 {
            continue;
        }
        let base = o.base_vertex as u32;
        if base >= vertex_capacity {
            continue;
        }
        let (first, count) = (o.first_index as usize, (o.index_count / 3 * 3) as usize);
        let vertex_count = match indices.get(first..first + count) {
            Some(ix) => ix.iter().copied().max().map(|m| m + 1).unwrap_or(0),
            None => vertex_capacity - base,
        };
        if vertex_count == 0 || base + vertex_count > vertex_capacity {
            continue;
        }
        let g = RtGeom { base_vertex: base, vertex_count, first_index: o.first_index, index_count: count as u32, material: o.material, kind: o.kind & 255 };
        let e = cells.entry(cell_of(o.min, o.max)).or_insert_with(|| (vec![], Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)));
        e.0.push(g);
        e.1 = e.1.min(Vec3::from(o.min));
        e.2 = e.2.max(Vec3::from(o.max));
    }
    let mut out: Vec<_> = cells.into_iter().map(|(k, (g, mn, mx))| (RtKey::Cell(k.0, k.1), g, mn, mx)).collect();
    out.sort_by_key(|e| e.0);
    out
}

/// A model's geometries (`Renderer::upload_model`): each object's range, its
/// vertex count from its indices (`indices` start at `index_at`).
pub fn model_geoms(objects: &[GpuObject], index_at: u32, indices: &[u32]) -> (Vec<RtGeom>, Vec3, Vec3) {
    let (mut mn, mut mx) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    let mut out = vec![];
    for o in objects {
        if o.index_count < 3 || o.base_vertex < 0 || o.first_index < index_at {
            continue;
        }
        let first = (o.first_index - index_at) as usize;
        let count = (o.index_count / 3 * 3) as usize;
        let Some(ix) = indices.get(first..first + count) else { continue };
        let vc = ix.iter().copied().max().map(|m| m + 1).unwrap_or(0);
        out.push(RtGeom { base_vertex: o.base_vertex as u32, vertex_count: vc, first_index: o.first_index, index_count: count as u32, material: o.material, kind: o.kind & 255 });
        mn = mn.min(Vec3::from(o.min));
        mx = mx.max(Vec3::from(o.max));
    }
    (out, mn, mx)
}

/// The mean of a kit layer's albedo (its last mip: one sRGB texel) in linear rgb.
pub fn layer_mean(albedo_mips: &[Vec<u8>]) -> [f32; 4] {
    let lin = |c: u8| {
        let c = c as f32 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    match albedo_mips.last() {
        Some(t) if t.len() >= 4 => [lin(t[0]), lin(t[1]), lin(t[2]), 1.0],
        _ => [0.5, 0.5, 0.5, 1.0],
    }
}

impl RtScene {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, init: RtSceneInit) -> Self {
        let config = RtSceneConfig::from_env();
        let geo_buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("rt-geometries"), size: TABLE_ENTRIES as u64 * 16, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let layers = init.kit_layers.max(1);
        let grey: Vec<[f32; 4]> = vec![[0.5, 0.5, 0.5, 1.0]; layers as usize];
        let layer_mean_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("rt-layer-means"), contents: bytemuck::cast_slice(&grey), usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST });
        let tlas = device.create_tlas(&wgpu::CreateTlasDescriptor { label: Some("rt-tlas"), max_instances: MAX_INSTANCES, flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE, update_mode: wgpu::AccelerationStructureUpdateMode::Build });
        let cs = wgpu::ShaderStages::COMPUTE;
        let e = |binding, ty| wgpu::BindGroupLayoutEntry { binding, visibility: cs, ty, count: None };
        let ubuf = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None };
        let sbuf = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rt-scene-bgl"),
            entries: &[e(0, ubuf), e(1, wgpu::BindingType::AccelerationStructure { vertex_return: false }), e(2, sbuf), e(3, sbuf), e(4, sbuf), e(5, sbuf), e(6, sbuf)],
        });
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rt-scene-bg"),
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: init.look_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: tlas.as_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: init.vertex_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: init.index_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: geo_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: init.material_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: layer_mean_buf.as_entire_binding() },
            ],
        });
        Self {
            config,
            device: device.clone(),
            queue: queue.clone(),
            vertex_buf: init.vertex_buf.clone(),
            index_buf: init.index_buf.clone(),
            geo_buf,
            layer_mean_buf,
            tlas: Mutex::new(tlas),
            bgl,
            bg,
            state: Mutex::new(SceneState { entries: HashMap::new(), table: RangeAlloc::new(TABLE_ENTRIES), instances: vec![], tlas_live: false, stats: RtSceneStats::default() }),
            bytes: TABLE_ENTRIES as u64 * 16 + layers as u64 * 16,
        }
    }

    /// Group 0 of every tracing pipeline (rt_scene.wgsl).
    pub fn bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.bgl
    }

    pub fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bg
    }

    /// Register a BLAS source (replacing one of the same key).
    pub fn add(&self, key: RtKey, geoms: Vec<RtGeom>, min: Vec3, max: Vec3) {
        let geoms: Vec<RtGeom> = geoms.into_iter().filter(|g| g.index_count >= 3 && g.vertex_count > 0).collect();
        if geoms.is_empty() {
            self.remove(key);
            return;
        }
        let triangles = geoms.iter().map(|g| g.index_count as u64 / 3).sum();
        let mut st = self.state.lock().unwrap();
        if let Some(old) = st.entries.insert(key, Entry { geoms, min, max, triangles, built: None }) {
            if let Some(b) = old.built {
                st.table.free(b.table_at, b.table_len);
            }
        }
    }

    /// Forget a source (its BLAS leaves the TLAS at the next encode).
    pub fn remove(&self, key: RtKey) {
        let mut st = self.state.lock().unwrap();
        if let Some(old) = st.entries.remove(&key) {
            if let Some(b) = old.built {
                st.table.free(b.table_at, b.table_len);
            }
        }
    }

    /// The kit maps' layers landed: their mean albedo (the last mip, linear).
    pub fn set_layer_means(&self, first: usize, means: &[[f32; 4]]) {
        if !means.is_empty() {
            self.queue.write_buffer(&self.layer_mean_buf, first as u64 * 16, bytemuck::cast_slice(means));
        }
    }

    pub fn stats(&self) -> RtSceneStats {
        let st = self.state.lock().unwrap();
        let mut s = st.stats;
        s.entries = st.entries.len();
        s.blas_built = st.entries.values().filter(|e| e.built.is_some()).count();
        s.blas_triangles = st.entries.values().filter(|e| e.built.is_some()).map(|e| e.triangles).sum();
        s
    }

    /// Whether a TLAS with instances is built (no pass traces before).
    pub fn live(&self) -> bool {
        let st = self.state.lock().unwrap();
        st.tlas_live && !st.instances.is_empty()
    }

    /// The frame's scene round `eye` (once a frame, before the first pass that
    /// traces): BLAS builds within the budget, the TLAS when the set changed.
    pub fn update(&self, enc: &mut wgpu::CommandEncoder, eye: Vec3) {
        let mut guard = self.state.lock().unwrap();
        guard.stats.updates += 1;
        self.update_scene(&mut guard, enc, eye, self.config.build_triangles);
    }

    /// (bench settling, no frame drawn) Build what the camera of the last
    /// look `update` wants, several frames' budget at a time; true when nothing
    /// wanted is left unbuilt, so a capture starts on the whole traced scene.
    pub fn prepare(&self, look: &crate::look::LookGpu) -> bool {
        let Some((_, eye, _, _)) = look.current_camera() else { return true };
        let mut guard = self.state.lock().unwrap();
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("rt-scene-prepare") });
        let left = self.update_scene(&mut guard, &mut enc, eye, self.config.build_triangles * 4);
        drop(guard);
        self.queue.submit([enc.finish()]);
        left == 0
    }

    /// Builds and drops BLASes round `eye` (at most `budget` triangles, one
    /// larger alone) and rebuilds the TLAS when the traced set changed. Returns
    /// how many wanted sources are still unbuilt.
    fn update_scene(&self, st: &mut SceneState, enc: &mut wgpu::CommandEncoder, eye: Vec3, budget: u64) -> usize {
        let cfg = self.config;
        // 1. what is traced: every source within the radius; for a kit tile,
        //    the near rung where it is built (and wanted), else the mid one.
        let mut want: Vec<(f32, RtKey)> = st.entries.iter().map(|(k, e)| (e.distance(eye), *k)).filter(|(d, _)| *d <= cfg.scene_radius_m).collect();
        want.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        // 2. builds: the nearest unbuilt first, within the budget (one larger alone).
        let mut budget = budget;
        let mut new_blas: Vec<(RtKey, wgpu::Blas, Vec<wgpu::BlasTriangleGeometrySizeDescriptor>)> = vec![];
        let mut built_now = 0u64;
        for (_, k) in &want {
            let e = &st.entries[k];
            if e.built.is_some() {
                continue;
            }
            if let RtKey::Kit(t, r) = *k {
                // a near rung not wanted, or a mid rung whose near rung stands in for it.
                if r == crate::kit::RUNG_NEAR && !cfg.kit_near {
                    continue;
                }
                if r == crate::kit::RUNG_MID && cfg.kit_near && st.entries.get(&RtKey::Kit(t, crate::kit::RUNG_NEAR)).is_some_and(|n| n.built.is_some()) {
                    continue;
                }
            }
            if e.triangles > budget && built_now > 0 {
                break;
            }
            budget = budget.saturating_sub(e.triangles);
            built_now += e.triangles;
            let sizes: Vec<wgpu::BlasTriangleGeometrySizeDescriptor> = e
                .geoms
                .iter()
                .map(|g| wgpu::BlasTriangleGeometrySizeDescriptor { vertex_format: wgpu::VertexFormat::Float32x3, vertex_count: g.vertex_count, index_format: Some(wgpu::IndexFormat::Uint32), index_count: Some(g.index_count), flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE })
                .collect();
            let blas = self.device.create_blas(
                &wgpu::CreateBlasDescriptor { label: Some("rt-blas"), flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE, update_mode: wgpu::AccelerationStructureUpdateMode::Build },
                wgpu::BlasGeometrySizeDescriptors::Triangles { descriptors: sizes.clone() },
            );
            new_blas.push((*k, blas, sizes));
            if budget == 0 {
                break;
            }
        }
        // the new BLASes' geometry table entries.
        let mut tables = vec![];
        for (k, _, _) in &new_blas {
            let geos: Vec<[u32; 4]> = st.entries[k].geoms.iter().map(|g| [g.base_vertex, g.first_index, g.material, g.kind]).collect();
            let n = geos.len() as u32;
            let Some(at) = st.table.alloc(n) else {
                tables.push(None);
                continue;
            };
            self.queue.write_buffer(&self.geo_buf, at as u64 * 16, bytemuck::cast_slice(&geos));
            tables.push(Some((at, n)));
        }
        // 3. drops: built BLASes now far away.
        let far: Vec<RtKey> = st.entries.iter().filter(|(_, e)| e.built.is_some() && e.distance(eye) > cfg.drop_radius_m).map(|(k, _)| *k).collect();
        for k in far {
            if let Some(b) = st.entries.get_mut(&k).and_then(|e| e.built.take()) {
                st.table.free(b.table_at, b.table_len);
                st.stats.blas_dropped += 1;
            }
        }
        // the builds this frame.
        let entries: Vec<wgpu::BlasBuildEntry> = new_blas
            .iter()
            .zip(&tables)
            .filter(|(_, t)| t.is_some())
            .map(|((k, blas, sizes), _)| wgpu::BlasBuildEntry {
                blas,
                geometry: wgpu::BlasGeometries::TriangleGeometries(
                    st.entries[k]
                        .geoms
                        .iter()
                        .zip(sizes)
                        .map(|(g, s)| wgpu::BlasTriangleGeometry { size: s, vertex_buffer: &self.vertex_buf, first_vertex: g.base_vertex, vertex_stride: STRIDE, index_buffer: Some(&self.index_buf), first_index: Some(g.first_index), transform_buffer: None, transform_buffer_offset: None })
                        .collect(),
                ),
            })
            .collect();
        let n_builds = entries.len();
        // 4. the instances: what is wanted and built (or built this frame).
        let built_keys: std::collections::HashSet<RtKey> = new_blas.iter().zip(&tables).filter(|(_, t)| t.is_some()).map(|((k, _, _), _)| *k).collect();
        let is_built = |st: &SceneState, k: &RtKey| st.entries.get(k).is_some_and(|e| e.built.is_some()) || built_keys.contains(k);
        let mut inst: Vec<RtKey> = want
            .iter()
            .map(|(_, k)| *k)
            .filter(|k| is_built(&st, k))
            .filter(|k| match *k {
                RtKey::Kit(t, r) if r == crate::kit::RUNG_MID => !(cfg.kit_near && is_built(&st, &RtKey::Kit(t, crate::kit::RUNG_NEAR))),
                RtKey::Kit(_, r) if r == crate::kit::RUNG_NEAR => cfg.kit_near,
                _ => true,
            })
            .take(MAX_INSTANCES as usize)
            .collect();
        inst.sort();
        let rebuild = inst != st.instances || n_builds > 0;
        let mut tlas = self.tlas.lock().unwrap();
        if rebuild && !inst.is_empty() {
            // the table entries of this frame's builds, looked up before they are stored.
            let table_of: HashMap<RtKey, u32> = new_blas.iter().zip(&tables).filter_map(|((k, _, _), t)| t.map(|t| (*k, t.0))).collect();
            let blas_of = |k: &RtKey| -> (&wgpu::Blas, u32) {
                match new_blas.iter().find(|(nk, _, _)| nk == k) {
                    Some((_, b, _)) if table_of.contains_key(k) => (b, table_of[k]),
                    _ => {
                        let b = st.entries[k].built.as_ref().unwrap();
                        (&b.blas, b.table_at)
                    }
                }
            };
            let old = st.instances.len();
            for (i, k) in inst.iter().enumerate() {
                let (b, at) = blas_of(k);
                tlas[i] = Some(wgpu::TlasInstance::new(b, [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0], at, 0xff));
            }
            for i in inst.len()..old.max(inst.len()) {
                tlas[i] = None;
            }
            enc.build_acceleration_structures(entries.iter(), std::iter::once(&*tlas));
            st.stats.tlas_builds += 1;
        } else if n_builds > 0 {
            enc.build_acceleration_structures(entries.iter(), std::iter::empty());
        }
        drop(entries);
        // the built BLASes join their entries.
        for ((k, blas, _), t) in new_blas.into_iter().zip(tables) {
            if let Some((at, n)) = t {
                if let Some(e) = st.entries.get_mut(&k) {
                    st.stats.blas_builds += 1;
                    st.stats.built_triangles += e.triangles;
                    e.built = Some(Built { blas, table_at: at, table_len: n });
                }
            }
        }
        st.stats.most_triangles_a_frame = st.stats.most_triangles_a_frame.max(built_now);
        if rebuild && !inst.is_empty() {
            st.tlas_live = true;
        }
        st.stats.instances = inst.len();
        st.stats.instance_triangles = inst.iter().map(|k| st.entries[k].triangles).sum();
        st.instances = inst;
        drop(tlas);
        // still to build: wanted, needed (a mid rung whose near rung is built is not) and unbuilt.
        want.iter()
            .filter(|(_, k)| st.entries.get(k).is_some_and(|e| e.built.is_none()))
            .filter(|(_, k)| match *k {
                RtKey::Kit(_, r) if r == crate::kit::RUNG_NEAR => cfg.kit_near,
                RtKey::Kit(t, r) if r == crate::kit::RUNG_MID => !(cfg.kit_near && st.entries.get(&RtKey::Kit(t, crate::kit::RUNG_NEAR)).is_some_and(|n| n.built.is_some())),
                _ => true,
            })
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::ObjectKind;

    #[test]
    fn static_cells_take_the_traced_kinds_with_their_vertex_counts() {
        let mut objs = vec![];
        let mut t = GpuObject::new(ObjectKind::Terrain, [0.0; 3], [10.0, 1.0, 10.0], 0, 6, 0);
        t.lod_role = crate::scene::LOD_TERRAIN;
        objs.push(t);
        let mut s = GpuObject::new(ObjectKind::Street, [300.0, 0.0, 0.0], [310.0, 1.0, 10.0], 6, 3, 4);
        s.lod_role = crate::scene::LOD_STREET_NEAR; // streamed: not static
        objs.push(s);
        let mut f = GpuObject::new(ObjectKind::Street, [300.0, 0.0, 0.0], [310.0, 1.0, 10.0], 6, 3, 4);
        f.lod_role = LOD_STREET_FAR;
        objs.push(f);
        let k = GpuObject::new(ObjectKind::Kit, [0.0; 3], [1.0; 3], 0, 3, 0);
        objs.push(k);
        let late = GpuObject::new(ObjectKind::Terrain, [0.0; 3], [1.0; 3], 100, 3, 50); // indices not in the table yet
        objs.push(late);
        let indices = [0, 1, 2, 2, 3, 0, 0, 1, 2];
        let cells = static_cells(&objs, &indices, 64);
        assert_eq!(cells.len(), 2, "{cells:?}");
        let a = cells.iter().find(|c| c.0 == RtKey::Cell(0, 0)).unwrap();
        assert_eq!(a.1.len(), 2, "the terrain and the late terrain");
        assert_eq!(a.1[0].vertex_count, 4);
        assert_eq!(a.1[1].vertex_count, 64 - 50, "a range not yet written: the rest of the arena");
        let b = cells.iter().find(|c| c.0 == RtKey::Cell(1, 0)).unwrap();
        assert_eq!(b.1.len(), 1, "the far strip only");
        assert_eq!((b.1[0].base_vertex, b.1[0].vertex_count, b.1[0].kind), (4, 3, ObjectKind::Street as u32));
    }

    #[test]
    fn the_config_reads_its_defaults() {
        let c = RtSceneConfig::default();
        assert!(c.drop_radius_m >= c.scene_radius_m);
    }
}
