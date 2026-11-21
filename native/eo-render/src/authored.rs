//! The game's Blender-built buildings in the GPU-driven renderer: materials,
//! LOD groups, reserved GPU space, and the background loader that streams
//! the models in around the camera.
//!
//! Startup (fast, no GLB binary is read): every manifest entry is matched to
//! its record in the merged city, placed (eo_assets::placement), and its GLB's
//! JSON header is read for vertex/index/part counts and materials. Each
//! building gets an LOD group, a reserved range of the shared vertex, index
//! and object buffers, and its materials resolve into the one material table.
//! The footprint of each placed building becomes its far stand-in.
//!
//! Streaming: worker threads take the nearest pending building to the
//! camera, load its model (binary cache, or verified GLB + cook), transform it
//! to world space and hand it to the render thread, which writes it into its
//! reserved range and flips the group's `ready` flag. Until then the cull pass
//! keeps drawing the footprint.

use crate::scene::{BLEND_BIT, GpuObject, LOD_NEAR, NO_GROUP, ObjectKind, Vertex};
use anyhow::Result;
use bytemuck::{Pod, Zeroable};
use eo_assets::glb::{AlphaMode, CookedMaterial, GlbCounts, peek_header};
use eo_assets::kit::{KitLibrary, hex_to_linear};
use eo_assets::placement::{Placement, PlacementConstants, place};
use eo_assets::{AssetRef, BlenderBuilding, LoadSource};
use glam::{DVec2, Mat3, Mat4, Vec3};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Material flags (mirrored in draw.wgsl).
pub const F_VERTEX_COLOUR: u32 = 1;
pub const F_TEXTURED: u32 = 2;
pub const F_BLEND: u32 = 4;
pub const F_GLASS: u32 = 8;
pub const F_THIN_GLASS: u32 = 16;
/// the street surface material (draw.wgsl `street_surface`).
pub const F_STREET: u32 = 32;
pub const F_FRONT_DAYLIGHT: u32 = 64;

/// One material (64 bytes, std430).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
pub struct GpuMaterial {
    /// Linear RGB multiplier (kit tint, or the glTF base colour) and opacity.
    pub base: [f32; 4],
    /// Linear emissive RGB (factor x strength); w unused.
    pub emissive: [f32; 4],
    /// roughness, metalness, normal strength, 1 / metres per texture repeat.
    pub params: [f32; 4],
    /// Texture array layer of the kit maps (-1: none).
    pub layer: i32,
    pub flags: u32,
    /// Mix of sampled roughness toward `params.x` (kit `roughFlatten`).
    pub rough_flatten: f32,
    /// Thin glazing absorption (kit `thinAbsorption`).
    pub thin_absorption: f32,
}

impl GpuMaterial {
    pub fn vertex_colour() -> Self {
        Self {
            base: [1.0, 1.0, 1.0, 1.0],
            emissive: [0.0; 4],
            params: [0.9, 0.0, 0.0, 0.0],
            layer: -1,
            flags: F_VERTEX_COLOUR,
            rough_flatten: 0.0,
            thin_absorption: 0.0,
        }
    }
    pub fn is_blend(&self) -> bool {
        self.flags & F_BLEND != 0
    }
}

/// Every material the static world and the models draw, and the kit map
/// sources (texture array layers) they sample.
pub struct MaterialTable {
    pub gpu: Vec<GpuMaterial>,
    pub names: Vec<String>,
    by_key: HashMap<String, u32>,
    /// Kit material whose bake is layer i of the texture arrays.
    pub layers: Vec<String>,
    layer_of: HashMap<String, i32>,
    pub kit_materials: usize,
    pub factor_materials: usize,
    /// Kit materials the JS draws from a map set this port bakes differently
    /// (a borrowed interior bake, or the scanned facade atlas): name -> why.
    pub deviations: Vec<(String, String)>,
}

impl MaterialTable {
    pub fn new() -> Self {
        Self {
            gpu: vec![GpuMaterial::vertex_colour()],
            names: vec!["vertex-colour".into()],
            by_key: HashMap::new(),
            layers: vec![],
            layer_of: HashMap::new(),
            kit_materials: 0,
            factor_materials: 0,
            deviations: vec![],
        }
    }

    /// the street surface material, added once; its index.
    pub fn street(&mut self) -> u32 {
        if let Some(&i) = self.by_key.get("street-surface") {
            return i;
        }
        let i = self.gpu.len() as u32;
        self.gpu.push(crate::street::street_material());
        self.names.push("street-surface".into());
        self.by_key.insert("street-surface".into(), i);
        i
    }

    /// a material under its own key, added once; its index.
    pub fn named(&mut self, key: &str, g: GpuMaterial) -> u32 {
        if let Some(&i) = self.by_key.get(key) {
            return i;
        }
        self.push(key.to_string(), g)
    }

    fn layer(&mut self, source: &str) -> i32 {
        if let Some(&l) = self.layer_of.get(source) {
            return l;
        }
        let l = self.layers.len() as i32;
        self.layers.push(source.to_string());
        self.layer_of.insert(source.to_string(), l);
        l
    }

    /// The material index for a GLB material, adding it on first sight.
    /// Kit-named materials follow `build()` in kit/materials.js; any other
    /// keeps its glTF factors, as `dressScene` in blender/runtime.js does.
    pub fn resolve(&mut self, kit: &KitLibrary, m: &CookedMaterial) -> u32 {
        if let Some(name) = kit.kit_name(&m.name) {
            if let Some(&i) = self.by_key.get(&m.name) {
                return i;
            }
            let spec = &kit.specs[name];
            // `paint` (the @rrggbb a Blender build wrote) times the alias's baseTint.
            let paint = m.name.split('@').nth(1).and_then(hex_to_linear).unwrap_or([1.0; 3]);
            let base_tint = spec.base_tint.as_deref().and_then(hex_to_linear).unwrap_or([1.0; 3]);
            let mut tint = [paint[0] * base_tint[0], paint[1] * base_tint[1], paint[2] * base_tint[2]];
            let source = kit.map_source(name).to_string();
            let layer = self.layer(&source);
            if let Some(why) = spec.borrow.as_ref().map(|b| format!("JS borrows the interior library's '{b}' bake; this draws the kit spec's own bake"))
                .or(spec.atlas_id.as_ref().map(|a| format!("surfaces: draws the scanned facade atlas cell '{a}' (public/assets/facade-atlas-v2-*) as the JS does (facade: two offset taps under its noise mask on the directional face frame; the cell resampled to a 512 px kit layer)")))
            {
                if !self.deviations.iter().any(|d| d.0 == name) {
                    self.deviations.push((name.to_string(), why));
                }
            }
            // an atlas finish is `albedo x tint x paint / atlasMeanLinear`
            // (kit/materials.js build()), so its multiplier carries the division.
            if let Some(mean) = spec.atlas_mean_linear {
                tint = [tint[0] / mean[0].max(1e-3), tint[1] / mean[1].max(1e-3), tint[2] / mean[2].max(1e-3)];
            }
            let tile = spec.world_tile_m.filter(|t| *t > 0.0).unwrap_or(spec.metres_per_tile);
            let physical = spec.physical.as_ref();
            let transparent = physical.map(|p| p.transparent).unwrap_or(false);
            let opacity = if transparent { physical.map(|p| p.opacity).unwrap_or(1.0) } else { 1.0 };
            let mut flags = F_TEXTURED;
            let thin = spec.p.get("thinAbsorption").and_then(|v| v.as_f64());
            if spec.kind == "glass" {
                flags |= F_GLASS;
                let glass = spec.p.get("color").and_then(|c| c.as_str()).and_then(hex_to_linear).unwrap_or([1.0; 3]);
                tint = [glass[0] * tint[0], glass[1] * tint[1], glass[2] * tint[2]];
                if thin.is_some() {
                    flags |= F_THIN_GLASS;
                }
            }
            if transparent {
                flags |= F_BLEND;
            }
            // the finish's ageing class, detail spec and atlas taps.
            flags |= crate::facade::material_flags(name, spec.kind == "glass", spec.atlas_id.is_some());
            if spec.kind == "glass" && thin.is_none() {
                // the room behind kit glazing (NIGHT_INTERIOR_KIT_GLASS; thin glass draws none).
                flags |= crate::rooms::kit_glass_room(name) << crate::rooms::ROOM_SHIFT;
            }
            let g = GpuMaterial {
                base: [tint[0] as f32, tint[1] as f32, tint[2] as f32, opacity as f32],
                // w = the glass's clearcoat (MeshPhysicalMaterial clearcoat layer).
                emissive: [0.0, 0.0, 0.0, if spec.kind == "glass" { physical.map(|p| p.clearcoat).unwrap_or(0.0) as f32 } else { 0.0 }],
                params: [
                    spec.rough as f32,
                    spec.metal as f32,
                    if spec.kind == "glass" { 0.0 } else { spec.normal_strength.unwrap_or(0.9) as f32 },
                    (1.0 / tile) as f32,
                ],
                layer,
                flags,
                rough_flatten: spec.rough_flatten.unwrap_or(0.0).clamp(0.0, 1.0) as f32,
                thin_absorption: thin.unwrap_or(0.0) as f32,
            };
            self.kit_materials += 1;
            return self.push(m.name.clone(), g);
        }
        let key = format!(
            "{}|{:?}|{}|{}|{:?}|{}|{:?}",
            m.name, m.base_color, m.metallic, m.roughness, m.emissive, m.emissive_strength, m.alpha_mode
        );
        if let Some(&i) = self.by_key.get(&key) {
            return i;
        }
        // Non-kit glass is made transparent by dressScene; BLEND is the glTF's own.
        let blend = m.alpha_mode == AlphaMode::Blend || m.name.starts_with("glass");
        // MASK with a constant alpha: drawn or not, decided once.
        if m.alpha_mode == AlphaMode::Mask && m.base_color[3] < m.alpha_cutoff {
            // Fully cut out: keep it in the table but transparent at zero opacity.
        }
        let s = m.emissive_strength;
        let g = GpuMaterial {
            base: [
                m.base_color[0],
                m.base_color[1],
                m.base_color[2],
                if m.alpha_mode == AlphaMode::Mask { if m.base_color[3] < m.alpha_cutoff { 0.0 } else { 1.0 } } else if blend { m.base_color[3] } else { 1.0 },
            ],
            emissive: [m.emissive[0] * s, m.emissive[1] * s, m.emissive[2] * s, 0.0],
            params: [m.roughness, m.metallic, 0.0, 0.0],
            layer: -1,
            flags: if blend || (m.alpha_mode == AlphaMode::Mask && m.base_color[3] < m.alpha_cutoff) { F_BLEND } else { 0 },
            rough_flatten: 0.0,
            thin_absorption: 0.0,
        };
        self.factor_materials += 1;
        self.push(key, g)
    }

    /// Register every kit material (sorted by name) as one contiguous palette
    /// for the kit buildings, which pick their material per vertex (the
    /// vertex colour's alpha byte is the palette index). Returns the first
    /// material index and the names in palette order. Idempotent per name.
    pub fn register_kit_palette(&mut self, kit: &KitLibrary) -> (u32, Vec<String>) {
        let mut names: Vec<String> = kit.specs.keys().cloned().collect();
        names.sort();
        assert!(names.len() <= 255, "kit palette must fit a byte");
        let base = self.gpu.len() as u32;
        for name in &names {
            // A fresh entry per palette slot (even when a Blender model already
            // uses the name) so the palette is contiguous.
            let m = CookedMaterial {
                name: name.clone(),
                base_color: [1.0; 4],
                metallic: 0.0,
                roughness: 1.0,
                emissive: [0.0; 3],
                emissive_strength: 1.0,
                alpha_mode: AlphaMode::Opaque,
                alpha_cutoff: 0.5,
                double_sided: true,
            };
            let before = self.by_key.remove(name);
            let at = self.resolve(kit, &m);
            // a painted wall finish draws on the neutral primer (eo_kit::wall_paint; off: as before).
            if let Some(l) = eo_kit::wall_paint::primer_lift(name) {
                let g = &mut self.gpu[at as usize];
                for k in 0..3 {
                    g.base[k] *= l[k];
                }
            }
            if let Some(prev) = before {
                self.by_key.insert(name.clone(), prev);
            }
        }
        (base, names)
    }

    /// the material registered under `key` (a kit finish's name once the kit palette is in), if any.
    pub fn by_name(&self, key: &str) -> Option<u32> {
        self.by_key.get(key).copied()
    }

    fn push(&mut self, key: String, g: GpuMaterial) -> u32 {
        let i = self.gpu.len() as u32;
        let name = key.split('|').next().unwrap_or_default().to_string();
        self.gpu.push(g);
        self.names.push(name);
        self.by_key.insert(key, i);
        i
    }
}

impl Default for MaterialTable {
    fn default() -> Self {
        Self::new()
    }
}

/// One Blender building's LOD state on the GPU (32 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
pub struct GpuLodGroup {
    /// The JS HLOD centre (frame centre, baseY + height / 2) and bounds radius.
    pub center: [f32; 3],
    pub radius: f32,
    /// 1 once the model is resident (written by the CPU on upload).
    pub ready: u32,
    /// Hysteresis state, written by the LOD pass each frame (kit groups: the
    /// distance zone, 0 near / 1 mid / 2 far).
    pub detailed: u32,
    /// 0: a Blender building (HLOD radii); 1: a kit building (`kit::GROUP_KIND_KIT`).
    pub kind: u32,
    /// Kit groups: the level drawn this frame (0 near rung, 1 mid rung, 2 box),
    /// written by the LOD pass. `ready` holds bit 0 (near resident) and bit 1 (mid resident).
    pub level: u32,
}

/// Reserved space of one model, relative to the start of the model arena.
#[derive(Clone, Copy, Debug, Default)]
pub struct ModelSlot {
    pub vertex_offset: u32,
    pub vertex_count: u32,
    pub index_offset: u32,
    pub index_count: u32,
    pub object_offset: u32,
    pub object_count: u32,
}

pub struct AuthoredBuilding {
    pub id: String,
    /// Key of the record in the merged city.
    pub record_key: String,
    pub asset: AssetRef,
    pub placement: Placement,
    pub group: u32,
    pub slot: ModelSlot,
    pub counts: GlbCounts,
    /// GLB material index -> material table index.
    pub material_map: Vec<u32>,
    /// the weathering record its kit materials read (blender/runtime.js).
    pub weather: crate::facade::WeatherRecord,
}

pub struct AuthoredSet {
    pub buildings: Vec<AuthoredBuilding>,
    /// Manifest entries not drawn from their model, with the reason.
    pub skipped: Vec<(String, String)>,
    pub materials: MaterialTable,
    pub groups: Vec<GpuLodGroup>,
    /// City building id -> LOD group (feeds `SceneInputs::authored`).
    pub group_of_record: HashMap<String, u32>,
    pub vertex_capacity: u32,
    pub index_capacity: u32,
    pub object_capacity: u32,
    pub manifest_entries: usize,
    pub hlod_enter_m: f32,
    pub hlod_exit_m: f32,
    pub prepare_ms: f64,
    pub triangles_reserved: u64,
}

impl AuthoredSet {
    /// An empty set (models off): footprints everywhere, one material.
    pub fn empty() -> Self {
        Self {
            buildings: vec![],
            skipped: vec![],
            materials: MaterialTable::new(),
            groups: vec![],
            group_of_record: HashMap::new(),
            vertex_capacity: 0,
            index_capacity: 0,
            object_capacity: 0,
            manifest_entries: 0,
            hlod_enter_m: 0.0,
            hlod_exit_m: 0.0,
            prepare_ms: 0.0,
            triangles_reserved: 0,
        }
    }

    pub fn prepare(
        repo_root: &Path,
        city: &eo_data::City,
        elevation: &eo_data::ElevationGrid,
        kit: &KitLibrary,
        constants: &PlacementConstants,
    ) -> Result<Self> {
        let t0 = Instant::now();
        let manifest: Vec<BlenderBuilding> = eo_assets::load_blender_buildings(repo_root)?;
        let records: HashMap<&str, &eo_data::Building> = city.buildings.iter().map(|b| (b.id.as_str(), b)).collect();
        let height_at = |x: f64, z: f64| elevation.sample(x, z);
        let mut set = Self::empty();
        set.manifest_entries = manifest.len();
        set.hlod_enter_m = constants.hlod.enter_radius_m as f32;
        set.hlod_exit_m = constants.hlod.exit_radius_m as f32;
        for b in &manifest {
            let Some(rec) = records.get(b.record_key()) else {
                set.skipped.push((b.id.clone(), "no record with this id in the merged city (the JS draws nothing for it either)".into()));
                continue;
            };
            let ring: Vec<DVec2> = rec.record_polygon().iter().map(|p| DVec2::new(p[0] as f64, p[1] as f64)).collect();
            let placement = match place(b, &ring, rec.height as f64, constants, &height_at) {
                Ok(p) => p,
                Err(e) => {
                    set.skipped.push((b.id.clone(), format!("{e:#}")));
                    continue;
                }
            };
            let asset = b.assets.exterior.clone();
            let header = match peek_header(&asset.path(repo_root)) {
                Ok(h) => h,
                Err(e) => {
                    set.skipped.push((b.id.clone(), format!("exterior GLB header: {e:#}")));
                    continue;
                }
            };
            let c = header.counts;
            if c.indices == 0 {
                set.skipped.push((b.id.clone(), "exterior GLB draws nothing (empty build)".into()));
                continue;
            }
            let material_map: Vec<u32> = header.materials.iter().map(|m| set.materials.resolve(kit, m)).collect();
            let group = set.groups.len() as u32;
            set.groups.push(GpuLodGroup {
                center: placement.lod_center.as_vec3().to_array(),
                radius: placement.lod_radius as f32,
                ready: 0,
                detailed: 1,
                kind: 0,
                level: 0,
            });
            set.group_of_record.insert(rec.id.clone(), group);
            let slot = ModelSlot {
                vertex_offset: set.vertex_capacity,
                vertex_count: c.vertices,
                index_offset: set.index_capacity,
                index_count: c.indices,
                object_offset: set.object_capacity,
                object_count: c.parts,
            };
            set.vertex_capacity += c.vertices;
            set.index_capacity += c.indices;
            set.object_capacity += c.parts;
            set.triangles_reserved += c.indices as u64 / 3;
            set.buildings.push(AuthoredBuilding {
                id: b.id.clone(),
                record_key: rec.id.clone(),
                asset,
                placement,
                group,
                slot,
                counts: c,
                material_map,
                // the runtime's weathering object at this floor and frame (authoring frame, z north).
                weather: crate::facade::weather_record(
                    crate::facade::authored_weathering(&b.levels, &b.weathering, placement.base_y, [placement.frame.center.x, placement.frame.center.y]).as_ref(),
                ),
            });
        }
        set.prepare_ms = t0.elapsed().as_secs_f64() * 1e3;
        Ok(set)
    }

    /// Placeholder objects for every reserved model part: role NEAR, empty,
    /// so nothing draws until a model is uploaded over them.
    pub fn placeholder_objects(&self) -> Vec<GpuObject> {
        let mut out = Vec::with_capacity(self.object_capacity as usize);
        for b in &self.buildings {
            for _ in 0..b.slot.object_count {
                let mut o = GpuObject::new(ObjectKind::Model, [0.0; 3], [0.0; 3], 0, 0, 0);
                o.lod_group = b.group;
                o.lod_role = LOD_NEAR;
                out.push(o);
            }
        }
        out
    }
}

/// Where the model arena starts in the shared buffers.
#[derive(Clone, Copy, Debug, Default)]
pub struct ArenaBase {
    pub vertex: u32,
    pub index: u32,
    pub object: u32,
}

/// A model ready to be written into its reserved slot.
pub struct ModelUpload {
    pub building: usize,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub objects: Vec<GpuObject>,
    pub source: LoadSource,
    pub load_ms: f64,
    pub triangles: u64,
}

/// Transform a cooked model into its slot: world-space vertices (white
/// vertex colour), indices local to the model, one object per part with
/// absolute draw ranges, world AABB, material and LOD group.
pub fn model_upload(
    set: &AuthoredSet,
    building: usize,
    model: &eo_assets::CookedModel,
    base: ArenaBase,
    materials: &[GpuMaterial],
) -> Result<ModelUpload> {
    let b = &set.buildings[building];
    if model.positions.len() as u32 != b.slot.vertex_count
        || model.indices.len() as u32 != b.slot.index_count
        || model.parts.len() as u32 != b.slot.object_count
    {
        anyhow::bail!(
            "{}: cooked model ({} v, {} i, {} parts) does not fit its reserved slot ({} v, {} i, {} parts)",
            b.id,
            model.positions.len(),
            model.indices.len(),
            model.parts.len(),
            b.slot.vertex_count,
            b.slot.index_count,
            b.slot.object_count
        );
    }
    let m: Mat4 = b.placement.world_from_model.as_mat4();
    let nm = Mat3::from_mat4(m).inverse().transpose();
    let white = u32::MAX;
    let vertices: Vec<Vertex> = model
        .positions
        .iter()
        .zip(&model.normals)
        .map(|(p, &n)| Vertex {
            pos: m.transform_point3(Vec3::from(*p)).to_array(),
            normal: if n == 0 { 0 } else { eo_assets::glb::pack_normal(nm * eo_assets::glb::unpack_normal(n)) },
            color: white,
        })
        .collect();
    let mut objects = Vec::with_capacity(model.parts.len());
    for part in &model.parts {
        let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
        // World AABB of the part: its 8 local box corners transformed.
        for k in 0..8 {
            let c = Vec3::new(
                if k & 1 == 0 { part.min[0] } else { part.max[0] },
                if k & 2 == 0 { part.min[1] } else { part.max[1] },
                if k & 4 == 0 { part.min[2] } else { part.max[2] },
            );
            let w = m.transform_point3(c);
            for a in 0..3 {
                mn[a] = mn[a].min(w[a]);
                mx[a] = mx[a].max(w[a]);
            }
        }
        let material = b.material_map.get(part.material as usize).copied().unwrap_or(0);
        let mut o = GpuObject::new(
            ObjectKind::Model,
            mn,
            mx,
            base.index + b.slot.index_offset + part.first_index,
            part.index_count,
            (base.vertex + b.slot.vertex_offset) as i32,
        );
        o.material = material;
        if materials.get(material as usize).map(|g| g.is_blend()).unwrap_or(false) {
            o.kind |= BLEND_BIT;
        }
        o.lod_group = b.group;
        o.lod_role = LOD_NEAR;
        objects.push(o);
    }
    debug_assert!(objects.iter().all(|o| o.lod_group != NO_GROUP));
    Ok(ModelUpload {
        building,
        vertices,
        indices: model.indices.clone(),
        objects,
        source: LoadSource::Cache,
        load_ms: 0.0,
        triangles: model.triangles(),
    })
}

/// Background model loader.
pub struct Streamer {
    rx: Receiver<Result<ModelUpload, (usize, String)>>,
    held: Option<ModelUpload>,
    camera: Arc<Mutex<Vec3>>,
    pending: Arc<Mutex<Vec<usize>>>,
    in_flight: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    threads: Vec<std::thread::JoinHandle<()>>,
    pub total: usize,
    pub received: usize,
    pub failed: Vec<(String, String)>,
    /// gated (the caller requests each model; nothing is loaded
    /// unasked), the requests not yet handed to the workers, and the view
    /// the workers order by.
    pub gated: bool,
    outbox: crate::hitch::Outbox<usize>,
    view: Arc<Mutex<Option<crate::view_detail::ViewCone>>>,
}

impl Streamer {
    /// Start `threads` workers over every building of `set`, nearest to
    /// `camera` first, inside `radius_m` of the camera (all of them when infinite).
    pub fn start(
        set: Arc<AuthoredSet>,
        repo_root: PathBuf,
        cache_dir: Option<PathBuf>,
        base: ArenaBase,
        camera: Vec3,
        radius_m: f32,
        threads: usize,
    ) -> Self {
        Self::start_with(set, repo_root, cache_dir, base, camera, radius_m, threads, false)
    }

    /// `start`, or with `gated` workers that load only what
    /// `request` asks for (in view first, by screen importance) and wait for
    /// more instead of finishing when nothing is pending.
    #[allow(clippy::too_many_arguments)]
    pub fn start_with(
        set: Arc<AuthoredSet>,
        repo_root: PathBuf,
        cache_dir: Option<PathBuf>,
        base: ArenaBase,
        camera: Vec3,
        radius_m: f32,
        threads: usize,
        gated: bool,
    ) -> Self {
        let (tx, rx): (Sender<_>, Receiver<_>) = std::sync::mpsc::channel();
        let pending = Arc::new(Mutex::new(if gated { vec![] } else { (0..set.buildings.len()).collect::<Vec<_>>() }));
        let cam = Arc::new(Mutex::new(camera));
        let in_flight = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let materials: Arc<Vec<GpuMaterial>> = Arc::new(set.materials.gpu.clone());
        let view: Arc<Mutex<Option<crate::view_detail::ViewCone>>> = Arc::new(Mutex::new(None));
        let mut handles = vec![];
        for _ in 0..threads.max(1) {
            let view = view.clone();
            let (set, repo, cache, tx, pending, cam, in_flight, stop, materials) = (
                set.clone(),
                repo_root.clone(),
                cache_dir.clone(),
                tx.clone(),
                pending.clone(),
                cam.clone(),
                in_flight.clone(),
                stop.clone(),
                materials.clone(),
            );
            handles.push(std::thread::spawn(move || {
                crate::hitch::worker_started(); // below the render thread
                loop {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let next = {
                        let c = *cam.lock().unwrap();
                        let v = *view.lock().unwrap();
                        let mut p = pending.lock().unwrap();
                        let mut best: Option<(usize, f32)> = None;
                        for (k, &i) in p.iter().enumerate() {
                            let g = &set.groups[set.buildings[i].group as usize];
                            let d = (Vec3::from(g.center) - c).length() - g.radius;
                            // in view first by screen importance (no view: nearest, as before).
                            let r = crate::view_detail::rank(v.as_ref(), Vec3::from(g.center) - g.radius, Vec3::from(g.center) + g.radius, d);
                            if d <= radius_m && best.map(|b| r < b.1).unwrap_or(true) {
                                best = Some((k, r));
                            }
                        }
                        best.map(|(k, _)| {
                            in_flight.fetch_add(1, Ordering::SeqCst);
                            p.swap_remove(k)
                        })
                    };
                    let Some(i) = next else {
                        if !gated && pending.lock().unwrap().is_empty() {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(15));
                        continue;
                    };
                    let t = Instant::now();
                    let r = eo_assets::load_model(&repo, cache.as_deref(), &set.buildings[i].asset)
                        .and_then(|(model, src)| {
                            let mut up = model_upload(&set, i, &model, base, &materials)?;
                            up.source = src;
                            Ok(up)
                        })
                        .map(|mut up| {
                            up.load_ms = t.elapsed().as_secs_f64() * 1e3;
                            up
                        })
                        .map_err(|e| (i, format!("{e:#}")));
                    in_flight.fetch_sub(1, Ordering::SeqCst);
                    if tx.send(r).is_err() {
                        break;
                    }
                }
            }));
        }
        Self {
            rx,
            held: None,
            camera: cam,
            pending,
            in_flight,
            stop,
            threads: handles,
            total: set.buildings.len(),
            received: 0,
            failed: vec![],
            gated,
            outbox: Default::default(),
            view,
        }
    }

    pub fn set_camera(&self, eye: Vec3) {
        crate::hitch::try_set(&self.camera, eye); // never wait for a worker's lock
    }

    /// the view the workers order requests by.
    pub fn set_view(&self, cone: Option<crate::view_detail::ViewCone>) {
        crate::hitch::try_set(&self.view, cone);
    }

    /// viewdetail (gated): load building `i`; `cancel` takes back a request
    /// no worker has taken yet. Both reach the workers at `flush`.
    pub fn request(&mut self, i: usize) {
        self.outbox.request(i);
    }

    pub fn cancel(&mut self, i: usize) {
        self.outbox.cancel(i);
    }

    /// hand the requests to the workers (never waits for their lock).
    pub fn flush(&mut self) {
        self.outbox.flush(&self.pending);
    }

    /// requests queued here or with the workers, and loads in flight.
    pub fn busy(&self) -> bool {
        !self.outbox.is_empty() || self.in_flight.load(Ordering::SeqCst) > 0 || self.held.is_some() || self.pending.try_lock().map(|p| !p.is_empty()).unwrap_or(true)
    }

    /// Models finished since the last call (non-blocking).
    pub fn poll(&mut self, max: usize) -> Vec<ModelUpload> {
        self.poll_bytes(max, usize::MAX)
    }

    /// Like `poll`, stopping once `max_bytes` of vertices and indices are
    /// taken (the first model is always taken; the one that would overflow
    /// waits for the next call).
    pub fn poll_bytes(&mut self, max: usize, max_bytes: usize) -> Vec<ModelUpload> {
        let mut out = vec![];
        let mut bytes = 0usize;
        while out.len() < max {
            let next = match self.held.take() {
                Some(up) => Ok(Ok(up)),
                None => self.rx.try_recv(),
            };
            match next {
                Ok(Ok(up)) => {
                    let b = up.vertices.len() * std::mem::size_of::<Vertex>() + up.indices.len() * 4;
                    if !out.is_empty() && bytes + b > max_bytes {
                        self.held = Some(up);
                        break;
                    }
                    bytes += b;
                    self.received += 1;
                    out.push(up);
                }
                Ok(Err((i, e))) => {
                    self.received += 1;
                    self.failed.push((i.to_string(), e));
                }
                Err(_) => break,
            }
        }
        out
    }

    /// Nothing pending, nothing in flight, nothing unreceived.
    pub fn done(&self) -> bool {
        self.received >= self.total
            || (self.pending.lock().unwrap().is_empty() && self.in_flight.load(Ordering::SeqCst) == 0 && self.received >= self.total)
    }

    /// Buildings still waiting (outside the radius, or not yet reached).
    pub fn pending(&self) -> usize {
        self.pending.lock().unwrap().len()
    }
}

impl Drop for Streamer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for h in self.threads.drain(..) {
            let _ = h.join();
        }
    }
}

/// Box-filtered mip chain of a square RGBA8 image. `srgb` filters in linear.
pub fn mip_chain(rgba: &[u8], res: usize, srgb: bool) -> Vec<Vec<u8>> {
    let to_lin = |c: u8| {
        let c = c as f32 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let to_srgb = |c: f32| {
        let v = c.clamp(0.0, 1.0);
        let s = if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
        (s * 255.0).round() as u8
    };
    let mut levels = vec![rgba.to_vec()];
    let mut r = res;
    while r > 1 {
        let prev = levels.last().unwrap();
        let n = r / 2;
        let mut next = vec![0u8; n * n * 4];
        for y in 0..n {
            for x in 0..n {
                for c in 0..4 {
                    let at = |xx: usize, yy: usize| prev[((yy * r) + xx) * 4 + c];
                    let px = [at(2 * x, 2 * y), at(2 * x + 1, 2 * y), at(2 * x, 2 * y + 1), at(2 * x + 1, 2 * y + 1)];
                    next[(y * n + x) * 4 + c] = if srgb && c < 3 {
                        to_srgb(px.iter().map(|&p| to_lin(p)).sum::<f32>() / 4.0)
                    } else {
                        ((px.iter().map(|&p| p as u32).sum::<u32>() + 2) / 4) as u8
                    };
                }
            }
        }
        levels.push(next);
        r = n;
    }
    levels
}
