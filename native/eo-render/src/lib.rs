//! GPU-driven renderer for the native slice.
//!
//! All static geometry and every streamed building model live in one vertex
//! buffer and one index buffer. One storage buffer holds every object's AABB,
//! draw range, material index and LOD role; one holds the materials; three
//! texture arrays hold the authored kit's albedo, normal and ORM maps. Each
//! frame two compute dispatches run: the LOD pass updates every Blender
//! building's near/far state (the JS HLOD radii with hysteresis), then the
//! cull pass frustum-culls every object, applies its LOD role and writes
//! `DrawIndexedIndirect` commands into an opaque list and a transparent list.
//! The render pass draws each list with one `multi_draw_indexed_indirect_count`
//! (two pipelines: opaque, then alpha-blended glass). The CPU never loops over
//! objects per frame.

pub mod agents; // agents
pub mod alloc;
pub mod authored;
pub mod camera;
pub mod clouds; // the cloud noise, blue noise and cloud-shadow map
pub mod cc0; // the CC0 scanned sets
pub mod cc0_decals; // street wear decals
pub mod facade; // facade
pub mod ground2; // the open ground from the measured ground-green table
pub mod hitch; // hitch
pub mod hlod; // hlod
pub mod hlod_bake; // hlod
pub mod hlod_district; // hlod
pub mod hlod_lights; // hlod
pub mod hlod_trees; // hlod
pub mod kit;
pub mod landmarks; // landmarks
pub mod look;
pub mod occlusion; // occlusion
pub mod pace;
pub mod props; // props
pub mod props_lib; // props
pub mod props_lib2;
pub mod probe; // facade
pub mod rooms;
pub mod sky; // sky
pub mod street;
pub mod scene;
pub mod surfaces;
pub mod upload;
pub mod view_detail;
pub mod seabed;
pub mod water; // water
pub mod surf; // the surf strip along the beaches
pub mod farshore; // the far terrain

use anyhow::{Context, Result, anyhow};
use authored::{GpuLodGroup, GpuMaterial};
use bytemuck::{Pod, Zeroable};
use camera::{Camera, frustum_planes};
use scene::{GpuObject, Scene, Vertex};
use wgpu::util::DeviceExt;

pub use wgpu;

pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
pub const IDS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg32Uint;
/// Bytes of one `DrawIndexedIndirectArgs`.
pub const DRAW_ARGS_BYTES: u64 = 20;
/// Timestamp queries written per frame: cull begin/end, city pass begin/end
/// (the early pass with occlusion on), then (look.rs) shadow cull begin/end,
/// shadow depth begin/end, post begin/end, (surfaces) AO begin/end, then
/// (occlusion) pyramid + late cull begin/end and late city pass begin/end.
/// Post is the last pass executed whatever its slot.
pub const QUERIES_PER_FRAME: u32 = 34; // +2 (sky::Q_CLOUD_SHADOW, slots 32-33) // +10 (sky::Q_SKY_*, slots 22-31) // water: +4 (Q_WATER_FFT, Q_WATER) // facade: +2 for the scene probe (Q_PROBE) // surfaces: +2 for the AO pass (look::Q_AO); +4
/// timestamp slots of the pyramid + late cull pass and the late city pass.
pub const Q_OCCLUSION: u32 = 12;
pub const Q_LATE_DRAW: u32 = 14;
/// the scene probe's face (cull + draw, and on a capture's last face the fill and levels).
pub const Q_PROBE: u32 = 16;
/// the FFT (+ the patch cull without occlusion), and the water pass
/// with the glass drawn after the water in it (the frame copy is the gap between the two).
pub const Q_WATER_FFT: u32 = 18;
pub const Q_WATER: u32 = 20;
/// Counter words (see cull.wgsl): opaque draws, blend draws, triangles, model
/// parts drawn, model triangles, footprint stand-ins drawn, kit objects, kit
/// triangles, street objects, street triangles, (occlusion) late opaque draws,
/// objects in the frustum, triangles in the frustum, objects drawn early,
/// objects found occluded, pad.
pub const COUNTER_WORDS: usize = 16; // was 10
/// Terrain tiles further than this (camera to tile box) draw their coarse level.
pub const TERRAIN_LOD_DISTANCE_M: f32 = 1200.0;

#[derive(Clone, Copy, Debug)]
pub struct GpuOptions {
    /// wgpu's indirect-call validation (on by default in wgpu release builds).
    pub indirect_validation: bool,
    /// Ignore MULTI_DRAW_INDIRECT_COUNT even if the adapter has it.
    pub force_no_indirect_count: bool,
    pub backends: wgpu::Backends,
}

impl Default for GpuOptions {
    fn default() -> Self {
        Self { indirect_validation: true, force_no_indirect_count: false, backends: wgpu::Backends::PRIMARY }
    }
}

pub struct GpuContext {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub info: wgpu::AdapterInfo,
    pub adapter_features: wgpu::Features,
    pub enabled_features: wgpu::Features,
    pub indirect_count: bool,
    pub timestamps: bool,
    pub first_instance: bool,
    pub options: GpuOptions,
    /// BC block compression (the CC0 sets are BC7 / BC5).
    pub bc: bool,
}

impl GpuContext {
    pub fn create_instance(opts: &GpuOptions) -> wgpu::Instance {
        let mut flags = wgpu::InstanceFlags::from_build_config();
        flags.set(wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL, opts.indirect_validation);
        let flags = flags.with_env();
        let desc = wgpu::InstanceDescriptor { backends: opts.backends, flags, ..wgpu::InstanceDescriptor::new_without_display_handle_from_env() };
        wgpu::Instance::new(desc)
    }

    pub fn new(instance: wgpu::Instance, surface: Option<&wgpu::Surface<'_>>, options: GpuOptions) -> Result<Self> {
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: surface,
            apply_limit_buckets: false,
        }))
        .map_err(|e| anyhow!("no GPU adapter: {e}"))?;
        let info = adapter.get_info();
        let adapter_features = adapter.features();
        let down = adapter.get_downlevel_capabilities();
        if !down.flags.contains(wgpu::DownlevelFlags::INDIRECT_EXECUTION) {
            return Err(anyhow!("adapter {} lacks DownlevelFlags::INDIRECT_EXECUTION; indirect draws impossible", info.name));
        }
        if !down.flags.contains(wgpu::DownlevelFlags::VERTEX_STORAGE) {
            return Err(anyhow!("adapter {} lacks DownlevelFlags::VERTEX_STORAGE; the vertex shader reads the object table", info.name));
        }
        let mut wanted = wgpu::Features::empty();
        let indirect_count =
            adapter_features.contains(wgpu::Features::MULTI_DRAW_INDIRECT_COUNT) && !options.force_no_indirect_count;
        if indirect_count {
            wanted |= wgpu::Features::MULTI_DRAW_INDIRECT_COUNT;
        }
        let timestamps = adapter_features.contains(wgpu::Features::TIMESTAMP_QUERY);
        if timestamps {
            wanted |= wgpu::Features::TIMESTAMP_QUERY;
        }
        // Every indirect command carries its object index as first_instance:
        // that is how a vertex finds its object (material, stress copy).
        let first_instance = adapter_features.contains(wgpu::Features::INDIRECT_FIRST_INSTANCE);
        if !first_instance {
            return Err(anyhow!(
                "adapter {} lacks Features::INDIRECT_FIRST_INSTANCE, which this renderer uses to pass the object index to the shaders (no fallback is implemented: it would need a per-vertex object id)",
                info.name
            ));
        }
        wanted |= wgpu::Features::INDIRECT_FIRST_INSTANCE;
        // the people atlases are BC; without the feature they are decoded to RGBA8.
        // the far layer's facade pages are BC3 with it too (RGBA8 otherwise).
        // the CC0 sets are uploaded block-compressed when the adapter samples BC.
        let bc = adapter_features.contains(wgpu::Features::TEXTURE_COMPRESSION_BC);
        if bc {
            wanted |= wgpu::Features::TEXTURE_COMPRESSION_BC;
        }
        let limits = adapter.limits();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("eo-device"),
            required_features: wanted,
            required_limits: limits,
            ..Default::default()
        }))
        .context("request_device")?;
        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            info,
            adapter_features,
            enabled_features: wanted,
            indirect_count,
            timestamps,
            first_instance,
            options,
            bc, // cc0
        })
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CullParams {
    planes: [[f32; 4]; 6],
    /// xyz camera, w terrain LOD distance.
    cam: [f32; 4],
    object_count: u32,
    compact: u32,
    group_count: u32,
    blend_base: u32,
    hlod_enter: f32,
    hlod_exit: f32,
    blend_first: u32,
    _pad: u32,
    /// Kit LOD radii: near enter, near exit, mid enter, mid exit (metres to the bounds).
    kit: [f32; 4],
    /// Street LOD radii: near enter, near exit, mid enter, mid exit (metres to the bounds).
    street: [f32; 4],
    /// the city pass's clip matrix.
    view_proj: [[f32; 4]; 4],
    /// x two-phase occlusion on, y late opaque list base (slots).
    occ: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FrameUniform {
    view_proj: [[f32; 4]; 4],
    cam_pos: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    ambient_sky: [f32; 4],
    /// the front probe cube's capture point (xyz) and whether one is in use (w).
    probe_info: [f32; 4],
    copy_step: [f32; 4],
}

pub struct CullReadback {
    pub counters: [u32; COUNTER_WORDS],
    /// Opaque commands as written (compacted, or one slot per object).
    pub draws: Vec<[u32; 5]>,
    /// Transparent commands.
    pub blend_draws: Vec<[u32; 5]>,
}

impl CullReadback {
    pub fn draw_count(&self) -> u32 {
        self.counters[0] + self.counters[1] + self.counters[10] // + late opaque
    }
    pub fn triangles(&self) -> u32 {
        self.counters[2]
    }
}

/// The kit maps ready for upload: per layer, the mip chains of albedo
/// (sRGB), normal and ORM. Built off the render thread (`KitUpload::build`).
pub struct KitUpload {
    pub res: u32,
    pub layers: Vec<[Vec<Vec<u8>>; 3]>,
}

impl KitUpload {
    pub fn build(maps: &[eo_assets::kit::BakedMaps]) -> Self {
        let res = maps.first().map(|m| m.res as u32).unwrap_or(1);
        let layers = maps
            .iter()
            .map(|m| {
                [
                    authored::mip_chain(&m.albedo, m.res, true),
                    authored::mip_chain(&m.normal, m.res, false),
                    authored::mip_chain(&m.orm, m.res, false),
                ]
            })
            .collect();
        Self { res, layers }
    }
}

pub struct RendererInit<'a> {
    pub vertices: &'a [Vertex],
    pub indices: &'a [u32],
    /// Static objects followed by the model arena's placeholder objects.
    pub objects: &'a [GpuObject],
    /// Vertex and index capacity reserved after the static arrays for models.
    pub extra_vertices: u32,
    pub extra_indices: u32,
    pub materials: &'a [GpuMaterial],
    pub groups: &'a [GpuLodGroup],
    pub hlod_enter_m: f32,
    pub hlod_exit_m: f32,
    /// Texture array layers to allocate for the kit maps, and their size.
    /// Filled later by `upload_kit` (the bake runs off the render thread).
    pub kit_layers: u32,
    pub kit_res: u32,
    pub color_format: wgpu::TextureFormat,
    pub sample_count: u32,
    /// Stress copies of the object table (1 = the real city once).
    pub multiplier: u32,
    /// Also write (object, material) per pixel into an Rg32Uint target (tests).
    pub debug_ids: bool,
    /// First object that may be transparent (the fallback sizes its transparent list from here).
    pub blend_first_object: u32,
}

/// GPU memory this renderer allocated, by kind.
#[derive(Clone, Copy, Debug, Default)]
pub struct GpuBytes {
    pub vertex: u64,
    pub index: u64,
    pub objects: u64,
    pub draws: u64,
    pub materials: u64,
    pub groups: u64,
    pub textures: u64,
}

impl GpuBytes {
    pub fn total(&self) -> u64 {
        self.vertex + self.index + self.objects + self.draws + self.materials + self.groups + self.textures
    }
}

pub struct Renderer {
    pub object_count: u32,
    /// Objects of one copy (the real table) when multiplied.
    pub base_object_count: u32,
    pub group_count: u32,
    pub compact: bool,
    pub color_format: wgpu::TextureFormat,
    pub sample_count: u32,
    pub debug_ids: bool,
    /// First object that may be transparent (the fallback sizes its transparent list from here).
    pub blend_first: u32,
    vertex_buf: wgpu::Buffer,
    index_buf: wgpu::Buffer,
    object_buf: wgpu::Buffer,
    group_buf: wgpu::Buffer,
    _material_buf: wgpu::Buffer,
    kit_textures: [wgpu::Texture; 3],
    draw_buf: wgpu::Buffer,
    counter_buf: wgpu::Buffer,
    cull_params_buf: wgpu::Buffer,
    frame_buf: wgpu::Buffer,
    lod_pipeline: wgpu::ComputePipeline,
    cull_pipeline: wgpu::ComputePipeline,
    cull_bg: wgpu::BindGroup,
    /// the late cull, the pyramid, the per-object flags; on/off
    /// (`set_occlusion`) and whether the last `update` asked for it.
    late_pipeline: wgpu::ComputePipeline,
    pub occlusion: occlusion::OcclusionGpu,
    occlusion_enabled: std::sync::atomic::AtomicBool,
    occlusion_frame: std::sync::atomic::AtomicBool,
    opaque_pipeline: wgpu::RenderPipeline,
    blend_pipeline: wgpu::RenderPipeline,
    /// one per probe front cube (made with the renderer and when the street
    /// sets or the detail sheet arrive; a capture flips the index, nothing is made mid-play).
    draw_bg: std::sync::Mutex<[wgpu::BindGroup; 2]>,
    /// group 0 of the probe face pass (the probe frame uniform). hitch2: per front cube.
    probe_draw_bg: std::sync::Mutex<[wgpu::BindGroup; 2]>,
    /// the scene probe.
    pub probe: probe::ProbeGpu,
    // what group 0 is rebuilt from when the street sets arrive.
    draw_bgl: wgpu::BindGroupLayout,
    kit_sampler: wgpu::Sampler,
    street_sampler: wgpu::Sampler,
    /// the street's scanned sets (albedo sRGB, NRA), 1 x 1 until uploaded.
    street_textures: std::sync::Mutex<[wgpu::Texture; 2]>,
    /// The look: light rig, sky, sun shadows, HDR target and post (look.rs).
    pub look: look::LookGpu,
    /// appearance, weathering and the detail sheet (group 0 bindings 10-13).
    pub facade: facade::FacadeGpu,
    /// runtime switches (facade::RUN_*), read by draw.wgsl from the frame uniform.
    pub facade_flags: std::sync::atomic::AtomicU32,
    pub bytes: GpuBytes,
    pub multiplier: u32,
    pub hlod_enter_m: f32,
    pub hlod_exit_m: f32,
    pub texture_layers: u32,
    pub texture_res: u32,
    pub material_count: u32,
    pub vertex_capacity: u32,
    pub index_capacity: u32,
    /// Kit LOD radii (see `CullParams::kit`).
    pub kit_radii: [f32; 4],
    /// Street LOD radii (see `CullParams::street`).
    pub street_radii: [f32; 4],
    /// moving things (cars, people, the player), attached once their library is loaded.
    pub agents: std::sync::OnceLock<agents::AgentsGpu>,
    /// static street props (signals, signs, trees, pads), attached once their library is cooked.
    pub props: std::sync::OnceLock<props::PropsGpu>,
    /// bytes written through `write` since the last `update` (the
    /// frame's streaming uploads); a frame over `PROBE_DEFER_UPLOAD_BYTES`
    /// draws no probe face.
    frame_upload_bytes: std::sync::atomic::AtomicU64,
    /// the sea (water.rs), attached once its shore field and spectrum are built.
    pub water: std::sync::OnceLock<water::WaterGpu>,
    /// the surf strip (surf.rs), attached after the water.
    pub surf: std::sync::OnceLock<surf::SurfGpu>,
    /// the far terrain (farshore.rs), attached once its cooked heightfield is read.
    pub farshore: std::sync::OnceLock<farshore::FarShoreGpu>,
    /// the bridges' wire members (landmarks.rs); their solids are static objects of the table.
    pub landmarks: std::sync::OnceLock<landmarks::LandmarksGpu>,
    /// the CC0 scanned sets, their tables and the street decal grid (group 0 bindings 17-27).
    pub cc0: cc0::Cc0Gpu,
    /// the camera's widened view each frame, which the streamers
    /// gate their best-quality layer by (off unless `set_view_detail` turns it on).
    pub view: std::sync::Mutex<view_detail::ViewTracker>,
    /// per LOD group its far cell (written once, `write_hlod_map`), per
    /// group the far layer's state (the LOD pass writes it; draw.wgsl and the
    /// far layer read it), per cell its residency (the far layer's streamer).
    pub hlod_map_buf: wgpu::Buffer,
    pub hlod_state_buf: wgpu::Buffer,
    pub hlod_flags_buf: wgpu::Buffer,
    /// the far layer (hlod.rs), attached once (`attach_hlod`).
    pub hlod: std::sync::OnceLock<hlod::HlodGpu>,
    /// the far street lamps by night (stage 3) and the distant trees (stage 4).
    pub hlod_lights: std::sync::OnceLock<hlod_lights::FarLights>,
    pub hlod_trees: std::sync::OnceLock<hlod_trees::FarTrees>,
}

/// a frame that uploads this much (a quarter of the default 4 MiB budget)
/// moves its scene-probe face to a later frame.
pub const PROBE_DEFER_UPLOAD_BYTES: u64 = 1 << 20;

/// the per-target byte slot of a write (`hitch::WRITE_TARGET_NAMES`).
fn hitch_target(t: upload::Target) -> usize {
    use upload::Target;
    match t {
        Target::Vertex => 0,
        Target::Index => 1,
        Target::Object => 2,
        Target::Group => 3,
        Target::Age => 4,
        Target::Texture { .. } => 5,
        Target::PropInstances => 6,
        Target::PropWires => 7,
        Target::PropLights => 8,
        Target::Cc0Texture { .. } => 5, // counted with the textures
        Target::Cc0Grid | Target::Cc0Decals => 10, // the street decal grid and records
    }
}

/// A square 2D texture array with a full mip chain; `layers` layers (at
/// least one), left for `write_array_layer`. Returns it and its byte size.
fn texture_array(ctx: &GpuContext, label: &str, format: wgpu::TextureFormat, res: u32, layers: u32) -> (wgpu::Texture, u64) {
    let mips = if res > 1 { res.ilog2() + 1 } else { 1 };
    let layers = layers.max(1);
    let tex = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: res, height: res, depth_or_array_layers: layers },
        mip_level_count: mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC, // + COPY_SRC (the upload test reads it back)
        view_formats: &[],
    });
    let bytes: u64 = (0..mips).map(|m| ((res >> m).max(1) as u64).pow(2) * 4).sum::<u64>() * layers as u64;
    (tex, bytes)
}

fn array_view(t: &wgpu::Texture) -> wgpu::TextureView {
    t.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() })
}

/// Group 0 of the city pipelines.
#[allow(clippy::too_many_arguments)]
fn make_draw_bg(
    d: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    frame_buf: &wgpu::Buffer,
    object_buf: &wgpu::Buffer,
    material_buf: &wgpu::Buffer,
    kit: [&wgpu::Texture; 3],
    sampler: &wgpu::Sampler,
    street: [&wgpu::Texture; 2],
    street_sampler: &wgpu::Sampler,
    facade: &facade::FacadeGpu, // facade
    probe_view: &wgpu::TextureView, // the front probe cube
    probe_sampler: &wgpu::Sampler,
    probe_sh: &wgpu::Buffer,
    cc0: &cc0::Cc0Gpu, // cc0
) -> wgpu::BindGroup {
    let (av, nv, mv) = (array_view(kit[0]), array_view(kit[1]), array_view(kit[2]));
    let cb = cc0.bind(); // cc0
    let (sa, sn) = (array_view(street[0]), array_view(street[1]));
    let fdv = facade.detail.lock().unwrap().create_view(&Default::default()); // facade
    d.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("draw-bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: frame_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: object_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: material_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&av) },
            wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&nv) },
            wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&mv) },
            wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::Sampler(sampler) },
            wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(&sa) },
            wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&sn) },
            wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::Sampler(street_sampler) },
            // per-building appearance, authored weathering, the detail sheet.
            wgpu::BindGroupEntry { binding: 10, resource: facade.age_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 11, resource: facade.weather_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 12, resource: wgpu::BindingResource::TextureView(&fdv) },
            wgpu::BindGroupEntry { binding: 13, resource: wgpu::BindingResource::Sampler(&facade.detail_sampler) },
            wgpu::BindGroupEntry { binding: 14, resource: wgpu::BindingResource::TextureView(probe_view) },
            wgpu::BindGroupEntry { binding: 15, resource: wgpu::BindingResource::Sampler(probe_sampler) },
            wgpu::BindGroupEntry { binding: 16, resource: probe_sh.as_entire_binding() },
            // the material and decal arrays, their sampler, the tables, the street decal grid and records.
            wgpu::BindGroupEntry { binding: 17, resource: wgpu::BindingResource::TextureView(&cb.mat[0]) },
            wgpu::BindGroupEntry { binding: 18, resource: wgpu::BindingResource::TextureView(&cb.mat[1]) },
            wgpu::BindGroupEntry { binding: 19, resource: wgpu::BindingResource::TextureView(&cb.mat[2]) },
            wgpu::BindGroupEntry { binding: 20, resource: wgpu::BindingResource::Sampler(&cc0.sampler) },
            wgpu::BindGroupEntry { binding: 21, resource: cb.mats.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 22, resource: cb.layers.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 23, resource: wgpu::BindingResource::TextureView(&cb.decal[0]) },
            wgpu::BindGroupEntry { binding: 24, resource: wgpu::BindingResource::TextureView(&cb.decal[1]) },
            wgpu::BindGroupEntry { binding: 25, resource: wgpu::BindingResource::TextureView(&cb.decal[2]) },
            wgpu::BindGroupEntry { binding: 26, resource: cb.grid.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 27, resource: cb.decals.as_entire_binding() },
        ],
    })
}

fn write_array_layer(queue: &wgpu::Queue, tex: &wgpu::Texture, layer: u32, chain: &[Vec<u8>]) {
    let res = tex.width();
    for (mip, level) in chain.iter().enumerate().take(tex.mip_level_count() as usize) {
        let r = (res >> mip).max(1);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: tex,
                mip_level: mip as u32,
                origin: wgpu::Origin3d { x: 0, y: 0, z: layer },
                aspect: wgpu::TextureAspect::All,
            },
            level,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(r * 4), rows_per_image: Some(r) },
            wgpu::Extent3d { width: r, height: r, depth_or_array_layers: 1 },
        );
    }
}

impl Renderer {
    /// The static scene alone: footprints everywhere, the vertex-colour material, no textures.
    pub fn new(ctx: &GpuContext, scene: &Scene, color_format: wgpu::TextureFormat, sample_count: u32) -> Result<Self> {
        Self::new_multiplied(ctx, scene, color_format, sample_count, 1)
    }

    /// Stress variant of `new`: every object `n` times (see `scene::multiply_objects`).
    pub fn new_multiplied(ctx: &GpuContext, scene: &Scene, color_format: wgpu::TextureFormat, sample_count: u32, n: u32) -> Result<Self> {
        let materials = [GpuMaterial::vertex_colour()];
        Self::from_init(
            ctx,
            &RendererInit {
                vertices: &scene.vertices,
                indices: &scene.indices,
                objects: &scene.objects,
                extra_vertices: 0,
                extra_indices: 0,
                materials: &materials,
                groups: &[],
                hlod_enter_m: 0.0,
                hlod_exit_m: 0.0,
                kit_layers: 0,
                kit_res: 1,
                color_format,
                sample_count,
                multiplier: n,
                debug_ids: false,
                blend_first_object: 0,
            },
        )
    }

    /// Bare parts (tests): one material, no groups, no textures.
    pub fn from_parts(
        ctx: &GpuContext,
        vertices: &[Vertex],
        indices: &[u32],
        objects: &[GpuObject],
        color_format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> Result<Self> {
        let materials = [GpuMaterial::vertex_colour()];
        Self::from_init(
            ctx,
            &RendererInit {
                vertices,
                indices,
                objects,
                extra_vertices: 0,
                extra_indices: 0,
                materials: &materials,
                groups: &[],
                hlod_enter_m: 0.0,
                hlod_exit_m: 0.0,
                kit_layers: 0,
                kit_res: 1,
                color_format,
                sample_count,
                multiplier: 1,
                debug_ids: false,
                blend_first_object: 0,
            },
        )
    }

    pub fn from_init(ctx: &GpuContext, init: &RendererInit) -> Result<Self> {
        let d = &ctx.device;
        let limits = d.limits();
        let n = init.multiplier.max(1);
        let objects: Vec<GpuObject> =
            if n > 1 { scene::multiply_objects(init.objects, n) } else { init.objects.to_vec() };
        let vertex_capacity = init.vertices.len() as u32 + init.extra_vertices;
        let index_capacity = init.indices.len() as u32 + init.extra_indices;
        let vb_bytes = vertex_capacity as u64 * std::mem::size_of::<Vertex>() as u64;
        let ib_bytes = index_capacity as u64 * 4;
        let ob_bytes = (objects.len().max(1) * std::mem::size_of::<GpuObject>()) as u64;
        for (what, b) in [("vertex", vb_bytes), ("index", ib_bytes), ("object", ob_bytes)] {
            if b > limits.max_buffer_size {
                return Err(anyhow!("{what} buffer {b} B exceeds max_buffer_size {}", limits.max_buffer_size));
            }
        }
        if ob_bytes > limits.max_storage_buffer_binding_size as u64 {
            return Err(anyhow!("object table {ob_bytes} B exceeds max_storage_buffer_binding_size"));
        }
        let object_count = objects.len() as u32;
        if object_count.div_ceil(64) > limits.max_compute_workgroups_per_dimension {
            return Err(anyhow!(
                "{object_count} objects need {} cull workgroups, over max_compute_workgroups_per_dimension {} (1-D dispatch)",
                object_count.div_ceil(64),
                limits.max_compute_workgroups_per_dimension
            ));
        }
        let buffer = |label: &str, size: u64, usage: wgpu::BufferUsages| {
            d.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size.max(4).next_multiple_of(4), usage, mapped_at_creation: false })
        };
        let vertex_buf = buffer("vertices", vb_bytes, wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST);
        ctx.queue.write_buffer(&vertex_buf, 0, bytemuck::cast_slice(init.vertices));
        let index_buf = buffer("indices", ib_bytes, wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST);
        ctx.queue.write_buffer(&index_buf, 0, bytemuck::cast_slice(init.indices));
        let object_buf = buffer("objects", ob_bytes, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC);
        ctx.queue.write_buffer(&object_buf, 0, bytemuck::cast_slice(&objects));
        let groups: Vec<GpuLodGroup> = if init.groups.is_empty() { vec![GpuLodGroup::zeroed()] } else { init.groups.to_vec() };
        let group_buf = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("lod-groups"),
            contents: bytemuck::cast_slice(&groups),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
        });
        let material_buf = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("materials"),
            contents: bytemuck::cast_slice(init.materials),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        // no group has a far cell until `write_hlod_map`; the state starts empty.
        let hlod_map_buf = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("hlod-map"),
            contents: bytemuck::cast_slice(&vec![u32::MAX; groups.len()]),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        let hlod_state_buf = buffer("hlod-state", groups.len() as u64 * 4, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC);
        let hlod_flags_buf = buffer("hlod-flags", hlod::MAX_CELLS as u64 * 4, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
        // Opaque list at [0, n), transparent list at [n, 2n); the
        // late opaque list at [2n, 3n) (compacted lists only).
        let lists = if ctx.indirect_count { 3 } else { 2 };
        let draw_bytes = (object_count.max(1) as u64) * lists * DRAW_ARGS_BYTES;
        let draw_buf = buffer("indirect-draws", draw_bytes, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_SRC);
        let counter_buf = buffer(
            "draw-counters",
            (COUNTER_WORDS * 4) as u64,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
        );
        let cull_params_buf = buffer("cull-params", std::mem::size_of::<CullParams>() as u64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
        let frame_buf = buffer("frame", std::mem::size_of::<FrameUniform>() as u64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);

        // Kit maps: three 2D arrays, one layer per map source, filled by upload_kit.
        let (res, layers) = if init.kit_layers > 0 { (init.kit_res.max(1), init.kit_layers) } else { (1, 1) };
        let (albedo_tex, ab) = texture_array(ctx, "kit-albedo", wgpu::TextureFormat::Rgba8UnormSrgb, res, layers);
        let (normal_tex, nb) = texture_array(ctx, "kit-normal", wgpu::TextureFormat::Rgba8Unorm, res, layers);
        let (orm_tex, mb) = texture_array(ctx, "kit-orm", wgpu::TextureFormat::Rgba8Unorm, res, layers);
        let sampler = d.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("kit-sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 16,
            ..Default::default()
        });

        let cull_module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cull.wgsl"),
            // + the pyramid test
            source: wgpu::ShaderSource::Wgsl(format!("{}\n{}{}{}", include_str!("cull.wgsl"), occlusion::OCCLUSION_WGSL, landmarks::cull_constants_wgsl(), hlod::cull_constants_wgsl()).into()), // + the set's near rule // hlod: + the far layer's switch and band
        });
        let occlusion_gpu = occlusion::OcclusionGpu::new(d, object_count, init.sample_count); // occlusion
        let draw_module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("draw.wgsl"),
            source: wgpu::ShaderSource::Wgsl(look::city_shader_source().into()), // draw.wgsl + look_*.wgsl
        });

        let buf_entry = |binding, visibility, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let cs = wgpu::ShaderStages::COMPUTE;
        let storage = |read_only| wgpu::BufferBindingType::Storage { read_only };
        let cull_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cull-bgl"),
            entries: &[
                buf_entry(0, cs, wgpu::BufferBindingType::Uniform),
                buf_entry(1, cs, storage(true)),
                buf_entry(2, cs, storage(false)),
                buf_entry(3, cs, storage(false)),
                buf_entry(4, cs, storage(false)),
                buf_entry(5, cs, storage(false)), // visible last frame
                buf_entry(6, cs, storage(true)),  // per group its far cell
                buf_entry(7, cs, storage(false)), // per group the far layer's state
                buf_entry(8, cs, storage(true)),  // per cell its residency
            ],
        });
        let cull_bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cull-bg"),
            layout: &cull_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: cull_params_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: object_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: draw_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: counter_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: group_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: occlusion_gpu.vis_buf.as_entire_binding() }, // occlusion
                wgpu::BindGroupEntry { binding: 6, resource: hlod_map_buf.as_entire_binding() },          // hlod
                wgpu::BindGroupEntry { binding: 7, resource: hlod_state_buf.as_entire_binding() },        // hlod
                wgpu::BindGroupEntry { binding: 8, resource: hlod_flags_buf.as_entire_binding() },        // hlod
            ],
        });
        let cull_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cull-layout"),
            bind_group_layouts: &[Some(&cull_bgl)],
            immediate_size: 0,
        });
        let compute = |entry: &str| {
            d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&cull_layout),
                module: &cull_module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let lod_pipeline = compute("lod");
        let cull_pipeline = compute("cull");
        // the late cull also binds the pyramid (group 1).
        let late_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cull-late-layout"),
            bind_group_layouts: &[Some(&cull_bgl), Some(&occlusion_gpu.late_bgl)],
            immediate_size: 0,
        });
        let late_pipeline = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("cull_late"),
            layout: Some(&late_layout),
            module: &cull_module,
            entry_point: Some("cull_late"),
            compilation_options: Default::default(),
            cache: None,
        });

        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let fs = wgpu::ShaderStages::FRAGMENT;
        let tex_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: fs,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2Array,
                multisampled: false,
            },
            count: None,
        };
        let draw_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("draw-bgl"),
            entries: &[
                buf_entry(0, vf, wgpu::BufferBindingType::Uniform),
                buf_entry(1, vf, storage(true)),
                buf_entry(2, fs, storage(true)),
                tex_entry(3),
                tex_entry(4),
                tex_entry(5),
                wgpu::BindGroupLayoutEntry { binding: 6, visibility: fs, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                // the street's scanned sets and their sampler.
                tex_entry(7),
                tex_entry(8),
                wgpu::BindGroupLayoutEntry { binding: 9, visibility: fs, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                // appearance per LOD group, weathering per Blender building, facade-detail-v1.
                buf_entry(10, fs, storage(true)),
                buf_entry(11, fs, storage(true)),
                wgpu::BindGroupLayoutEntry {
                    binding: 12,
                    visibility: fs,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 13, visibility: fs, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                // the scene probe (6 faces, 2D array) and its sampler.
                tex_entry(14),
                wgpu::BindGroupLayoutEntry { binding: 15, visibility: fs, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                buf_entry(16, fs, storage(true)), // the probe's irradiance SH9
                // material arrays, sampler, tables, decal arrays, decal grid and records.
                tex_entry(17),
                tex_entry(18),
                tex_entry(19),
                wgpu::BindGroupLayoutEntry { binding: 20, visibility: fs, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                buf_entry(21, fs, storage(true)),
                buf_entry(22, fs, storage(true)),
                tex_entry(23),
                tex_entry(24),
                tex_entry(25),
                buf_entry(26, fs, storage(true)),
                buf_entry(27, fs, storage(true)),
            ],
        });
        // the JS street scans' sampler (`configureSurfaceTexture`: repeat, trilinear, anisotropy 8).
        let street_sampler = d.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("street-sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        let (street_albedo, _) = texture_array(ctx, "street-albedo", wgpu::TextureFormat::Rgba8UnormSrgb, 1, 2);
        let (street_nra, _) = texture_array(ctx, "street-nra", wgpu::TextureFormat::Rgba8Unorm, 1, 2);
        // appearance per LOD group, weathering per Blender building (the leading kind-0 groups).
        let authored_groups = init.groups.iter().take_while(|g| g.kind == 0).count() as u32;
        let facade_gpu = facade::FacadeGpu::new(d, &ctx.queue, groups.len() as u32, authored_groups);
        // the light rig, sky, shadows and post (bind group 1 of the city pipelines).
        let look_gpu = look::LookGpu::new(
            d,
            look::LookInit {
                object_buf: &object_buf,
                group_buf: &group_buf,
                object_count,
                compact: ctx.indirect_count,
                output_format: init.color_format,
                sample_count: init.sample_count,
                debug_ids: init.debug_ids,
                vertex_stride: std::mem::size_of::<Vertex>() as u64,
            },
        );
        let draw_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("draw-layout"),
            bind_group_layouts: &[Some(&draw_bgl), Some(look_gpu.main_bind_group_layout())],
            immediate_size: 0,
        });
        let attrs = [
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Snorm8x4, offset: 12, shader_location: 1 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Unorm8x4, offset: 16, shader_location: 2 },
        ];
        let make = |label: &str, blend: bool| {
            let color = wgpu::ColorTargetState {
                format: look::HDR_FORMAT, // the city renders HDR; post writes init.color_format
                blend: blend.then_some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            };
            let ids = wgpu::ColorTargetState {
                format: IDS_FORMAT,
                blend: None,
                write_mask: if blend { wgpu::ColorWrites::empty() } else { wgpu::ColorWrites::ALL },
            };
            let targets_ids = [Some(color.clone()), Some(ids)];
            let targets = [Some(color)];
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&draw_layout),
                vertex: wgpu::VertexState {
                    module: &draw_module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &attrs,
                    })],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    // Footprint rings come in both windings, the GLBs are double
                    // sided and drawn under a mirror: draw both faces.
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(!blend),
                    depth_compare: Some(if blend { wgpu::CompareFunction::GreaterEqual } else { wgpu::CompareFunction::Greater }),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState { count: init.sample_count, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &draw_module,
                    entry_point: Some(if init.debug_ids { "fs_ids" } else { "fs" }),
                    compilation_options: Default::default(),
                    targets: if init.debug_ids { &targets_ids } else { &targets },
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let opaque_pipeline = make("draw-opaque", false);
        let blend_pipeline = make("draw-blend", true);
        // the scene probe (its face pass draws with the city pipeline layout).
        let probe_gpu = probe::ProbeGpu::new(
            d,
            &probe::ProbeInit {
                object_buf: &object_buf,
                group_buf: &group_buf,
                object_count,
                look_buf: look_gpu.look_buffer(),
                draw_module: &draw_module,
                draw_layout: &draw_layout,
                vertex_stride: std::mem::size_of::<Vertex>() as u64,
                attrs: &attrs,
                hdr_format: look::HDR_FORMAT,
                depth_format: DEPTH_FORMAT,
            },
        );
        // one bind group per probe front cube, both made now.
        let cc0_gpu = cc0::Cc0Gpu::new(d); // placeholders until attached
        let draw_bg = [0, 1].map(|f| make_draw_bg(d, &draw_bgl, &frame_buf, &object_buf, &material_buf, [&albedo_tex, &normal_tex, &orm_tex], &sampler, [&street_albedo, &street_nra], &street_sampler, &facade_gpu, &probe_gpu.cube_views[f], &probe_gpu.sampler, &probe_gpu.lobe_buf, &cc0_gpu));
        let probe_bg = [0, 1].map(|f| make_draw_bg(d, &draw_bgl, &probe_gpu.frame_buf, &object_buf, &material_buf, [&albedo_tex, &normal_tex, &orm_tex], &sampler, [&street_albedo, &street_nra], &street_sampler, &facade_gpu, &probe_gpu.cube_views[f], &probe_gpu.sampler, &probe_gpu.lobe_buf, &cc0_gpu));

        let bytes = GpuBytes {
            vertex: vb_bytes,
            index: ib_bytes,
            objects: ob_bytes,
            draws: draw_bytes + (COUNTER_WORDS * 4) as u64 + occlusion_gpu.vis_buf.size(), // + per-object flags
            materials: std::mem::size_of_val(init.materials) as u64,
            groups: std::mem::size_of_val(groups.as_slice()) as u64 + hlod_map_buf.size() + hlod_state_buf.size() + hlod_flags_buf.size(), // + the far layer's per-group words
            textures: ab + nb + mb + look_gpu.bytes + facade_gpu.bytes + probe_gpu.bytes, // shadow map, shadow lists, uniforms; ages, weather
        };
        Ok(Self {
            object_count,
            base_object_count: init.objects.len() as u32,
            group_count: init.groups.len() as u32,
            compact: ctx.indirect_count,
            color_format: init.color_format,
            sample_count: init.sample_count,
            debug_ids: init.debug_ids,
            blend_first: if n > 1 { 0 } else { init.blend_first_object.min(object_count) },
            vertex_buf,
            index_buf,
            object_buf,
            group_buf,
            _material_buf: material_buf,
            kit_textures: [albedo_tex, normal_tex, orm_tex],
            draw_buf,
            counter_buf,
            cull_params_buf,
            frame_buf,
            lod_pipeline,
            cull_pipeline,
            cull_bg,
            late_pipeline,
            occlusion: occlusion_gpu,
            occlusion_enabled: std::sync::atomic::AtomicBool::new(ctx.indirect_count),
            occlusion_frame: std::sync::atomic::AtomicBool::new(false),
            opaque_pipeline,
            blend_pipeline,
            draw_bg: std::sync::Mutex::new(draw_bg),
            probe_draw_bg: std::sync::Mutex::new(probe_bg),
            probe: probe_gpu,
            draw_bgl,
            kit_sampler: sampler,
            street_sampler,
            street_textures: std::sync::Mutex::new([street_albedo, street_nra]),
            look: look_gpu,
            facade: facade_gpu,
            facade_flags: std::sync::atomic::AtomicU32::new(facade::RUN_AGE | facade::RUN_DETAIL | facade::RUN_ATLAS2 | facade::RUN_PROBE | facade::RUN_ROOMS | facade::RUN_AGENT_HZB | facade::RUN_PROBE_DEFER), // + rooms, agent pyramid test, probe deferral
            bytes,
            multiplier: n,
            hlod_enter_m: init.hlod_enter_m,
            hlod_exit_m: init.hlod_exit_m,
            texture_layers: init.kit_layers,
            texture_res: res,
            material_count: init.materials.len() as u32,
            vertex_capacity,
            index_capacity,
            kit_radii: kit::DEFAULT_KIT_RADII,
            street_radii: street::DEFAULT_STREET_RADII,
            agents: std::sync::OnceLock::new(), // agents
            props: std::sync::OnceLock::new(),  // props
            frame_upload_bytes: std::sync::atomic::AtomicU64::new(0),
            water: std::sync::OnceLock::new(),  // water
            surf: std::sync::OnceLock::new(), // surf
            farshore: std::sync::OnceLock::new(), // farshore
            landmarks: std::sync::OnceLock::new(), // landmarks
            cc0: cc0_gpu, // cc0
            view: std::sync::Mutex::new(view_detail::ViewTracker::new(Default::default())),
            hlod_map_buf,                           // hlod
            hlod_state_buf,                         // hlod
            hlod_flags_buf,                         // hlod
            hlod: std::sync::OnceLock::new(),       // hlod
            hlod_lights: std::sync::OnceLock::new(), // hlod
            hlod_trees: std::sync::OnceLock::new(),  // hlod
        })
    }

    /// turn the view-gated best-quality layer on or off (and its numbers).
    pub fn set_view_detail(&self, config: view_detail::ViewDetailConfig) {
        if let Ok(mut v) = self.view.lock() {
            v.config = config;
        }
    }

    /// take this frame's camera at view-clock `t` seconds (None:
    /// the tracker's own clock: the last time given, else the wall clock).
    /// `update` calls it with None, so a caller that gives no time gets the
    /// view one frame late, which the hysteresis and the turn prediction cover.
    pub fn observe_view(&self, cam: &Camera, t: Option<f64>) {
        if let Ok(mut v) = self.view.lock() {
            match t {
                Some(t) => v.observe_at(cam, t),
                None => {
                    let t = v.clock();
                    v.observe(cam, t);
                }
            }
        }
    }

    /// the last observed widened view (None when off).
    pub fn view_cone(&self) -> Option<view_detail::ViewCone> {
        self.view.lock().ok().and_then(|v| v.cone())
    }

    /// stage 3, one point sprite per street lamp past the near lights (once). Returns the bytes allocated.
    pub fn attach_far_lights(&self, ctx: &GpuContext, lamps: &[eo_sky::lights::LampLight]) -> u64 {
        if self.hlod_lights.get().is_some() || !hlod::HlodOptions::from_env().on || !hlod::HlodOptions::from_env().lamps {
            return 0;
        }
        let l = hlod_lights::FarLights::new(&hlod_lights::FarLightsInit {
            device: &ctx.device,
            draw_bgl: &self.draw_bgl,
            main_bgl: self.look.main_bind_group_layout(),
            sample_count: self.sample_count,
            debug_ids: self.debug_ids,
            lamps,
        });
        let b = l.bytes;
        let _ = self.hlod_lights.set(l);
        b
    }

    /// stage 4, the distant trees' pipeline and buffer (once; the list
    /// comes later: `hlod_trees.get()?.supply`). Returns the bytes allocated.
    pub fn attach_far_trees(&self, ctx: &GpuContext) -> u64 {
        if self.hlod_trees.get().is_some() || (!self.compact && !self.look.options.cove_detail) || !hlod::HlodOptions::from_env().on || !hlod::HlodOptions::from_env().trees {
            return 0;
        }
        let t = hlod_trees::FarTrees::new(&hlod_trees::FarTreesInit {
            cove_only: !self.compact,
            device: &ctx.device,
            draw_bgl: &self.draw_bgl,
            main_bgl: self.look.main_bind_group_layout(),
            late_bgl: &self.occlusion.late_bgl,
            sample_count: self.sample_count,
            debug_ids: self.debug_ids,
        });
        let b = t.bytes;
        let _ = self.hlod_trees.set(t);
        b
    }

    /// build the far layer's GPU side (once; later calls are ignored).
    /// Needs the compacted lists (multi-draw-indirect-count); without them the
    /// far layer stays off and the boxes draw as before. Returns the bytes allocated.
    pub fn attach_hlod(&self, ctx: &GpuContext, opts: &hlod::HlodOptions) -> u64 {
        if self.hlod.get().is_some() || !self.compact || !opts.on {
            return 0;
        }
        let h = hlod::HlodGpu::new(&hlod::HlodInit {
            device: &ctx.device,
            draw_bgl: &self.draw_bgl,
            main_bgl: self.look.main_bind_group_layout(),
            late_bgl: &self.occlusion.late_bgl,
            state_buf: &self.hlod_state_buf,
            flags_buf: &self.hlod_flags_buf,
            sample_count: self.sample_count,
            debug_ids: self.debug_ids,
            opts: *opts,
        });
        let b = h.total_bytes();
        let _ = self.hlod.set(h);
        b
    }

    /// each LOD group's far cell (`hlod_bake::NO_CELL`: none), written once.
    pub fn write_hlod_map(&self, queue: &wgpu::Queue, group_cell: &[u32]) {
        let n = (self.hlod_map_buf.size() / 4) as usize;
        let words: Vec<u32> = (0..n).map(|g| group_cell.get(g).copied().unwrap_or(u32::MAX)).collect();
        queue.write_buffer(&self.hlod_map_buf, 0, bytemuck::cast_slice(&words));
    }

    /// the wire members' pipeline over the city's own groups (once;
    /// nothing on a debug-ids renderer, whose ground truth leaves blended members out).
    pub fn attach_landmark_wires(&self, ctx: &GpuContext, vertices: &[landmarks::WireVertex], indices: &[u32]) -> u64 {
        if self.landmarks.get().is_some() || self.debug_ids {
            return 0;
        }
        let w = landmarks::LandmarksGpu::new(&landmarks::WireInit {
            device: &ctx.device,
            draw_bgl: &self.draw_bgl,
            main_bgl: self.look.main_bind_group_layout(),
            group_buf: &self.group_buf,
            sample_count: self.sample_count,
            vertices,
            indices,
        });
        let bytes = w.as_ref().map(|w| w.bytes).unwrap_or(0);
        if let Some(w) = w {
            let _ = self.landmarks.set(w);
        }
        bytes
    }

    /// One budgeted write (see `upload::UploadQueue`). Object writes are
    /// repeated for every stress copy.
    pub fn write(&self, queue: &wgpu::Queue, target: upload::Target, offset: u64, data: &[u8]) {
        let t = std::time::Instant::now(); // hitch
        self.frame_upload_bytes.fetch_add(data.len() as u64, std::sync::atomic::Ordering::Relaxed);
        self.write_untimed(queue, target, offset, data);
        hitch::note_write_to(hitch_target(target), data.len() as u64, t.elapsed().as_secs_f64() * 1e3); // hitch // hitch2: per target
    }

    /// the buffer a streamed write lands in, for the upload stager's GPU
    /// copies (None: a texture, or the stress copies of the object table, which
    /// `write` repeats).
    pub fn upload_buffer(&self, target: upload::Target) -> Option<&wgpu::Buffer> {
        use upload::Target;
        match target {
            Target::Vertex => Some(&self.vertex_buf),
            Target::Index => Some(&self.index_buf),
            Target::Group => Some(&self.group_buf),
            Target::Age => Some(&self.facade.age_buf),
            Target::Object => (self.multiplier <= 1).then_some(&self.object_buf),
            Target::PropInstances => self.props.get().map(|p| p.instance_buffer()),
            Target::PropWires => self.props.get().map(|p| p.wire_buffer()),
            Target::PropLights => self.props.get().map(|p| p.lights_buffer()),
            Target::Texture { .. } | Target::Cc0Texture { .. } => None,
            // the street decal grid and records (once the CC0 buffers are attached).
            Target::Cc0Grid => self.cc0.grid_buffer(),
            Target::Cc0Decals => self.cc0.decals_buffer(),
        }
    }

    /// the texture subresource a streamed write lands in, for the
    /// upload stager's row-band GPU copies (None: not a texture). cc0: the
    /// CC0 arrays' BC mips too, banded in whole block rows.
    pub fn upload_texture(&self, target: upload::Target) -> Option<UploadTexture<'_>> {
        let (tex, layer, mip) = match target {
            upload::Target::Texture { map, layer, mip } => (self.kit_textures.get(map as usize)?, layer, mip),
            upload::Target::Cc0Texture { array, layer, mip } => (self.cc0.array(array)?, layer as u32, mip as u32),
            _ => return None,
        };
        UploadTexture::of(tex, mip, layer)
    }

    /// streamed bytes copied to the GPU this frame outside `write` (the
    /// upload stager's copies), counted like a write for the probe deferral.
    pub fn note_streamed(&self, bytes: u64) {
        self.frame_upload_bytes.fetch_add(bytes, std::sync::atomic::Ordering::Relaxed);
    }

    /// the device (the upload stager makes its chunks with it).
    pub fn gpu_device(&self) -> &wgpu::Device {
        &self.look.device
    }

    fn write_untimed(&self, queue: &wgpu::Queue, target: upload::Target, offset: u64, data: &[u8]) {
        use upload::Target;
        match target {
            Target::Vertex => queue.write_buffer(&self.vertex_buf, offset, data),
            Target::Index => queue.write_buffer(&self.index_buf, offset, data),
            Target::Group => queue.write_buffer(&self.group_buf, offset, data),
            Target::Age => queue.write_buffer(&self.facade.age_buf, offset, data), // facade
            Target::Object => {
                if self.multiplier <= 1 {
                    queue.write_buffer(&self.object_buf, offset, data);
                } else {
                    let objs: Vec<GpuObject> = bytemuck::pod_collect_to_vec(data);
                    for k in 0..self.multiplier {
                        let d = scene::copy_offset(k);
                        let copies: Vec<GpuObject> = objs
                            .iter()
                            .map(|o| GpuObject {
                                min: [o.min[0] + d[0], o.min[1] + d[1], o.min[2] + d[2]],
                                max: [o.max[0] + d[0], o.max[1] + d[1], o.max[2] + d[2]],
                                copy: k,
                                ..*o
                            })
                            .collect();
                        let at = offset + (k * self.base_object_count) as u64 * std::mem::size_of::<GpuObject>() as u64;
                        queue.write_buffer(&self.object_buf, at, bytemuck::cast_slice(&copies));
                    }
                }
            }
            // the street prop instance arena.
            Target::PropInstances => {
                if let Some(p) = self.props.get() {
                    queue.write_buffer(p.instance_buffer(), offset, data);
                }
            }
            // wire segments and lamp lights.
            Target::PropWires => {
                if let Some(p) = self.props.get() {
                    queue.write_buffer(p.wire_buffer(), offset, data);
                }
            }
            Target::PropLights => {
                if let Some(p) = self.props.get() {
                    queue.write_buffer(p.lights_buffer(), offset, data);
                }
            }
            // the CC0 arrays, the street decal grid and records.
            Target::Cc0Texture { .. } | Target::Cc0Grid | Target::Cc0Decals => self.cc0.write(queue, target, offset, data),
            Target::Texture { map, layer, mip } => {
                let tex = &self.kit_textures[map as usize];
                let r = (tex.width() >> mip).max(1);
                // `offset` is a byte offset into the mip and `data` whole
                // rows, so a mip can go in row bands (offset 0 and the whole mip: as before).
                let row = r as u64 * 4;
                let (y0, rows) = ((offset / row) as u32, (data.len() as u64 / row) as u32);
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo { texture: tex, mip_level: mip, origin: wgpu::Origin3d { x: 0, y: y0, z: layer }, aspect: wgpu::TextureAspect::All },
                    data,
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(r * 4), rows_per_image: Some(rows) },
                    wgpu::Extent3d { width: r, height: rows, depth_or_array_layers: 1 },
                );
            }
        }
    }

    /// Write a streamed model into its reserved range (vertices at
    /// `vertex_at`, indices at `index_at`, objects at `object_at`, all
    /// absolute) and mark its LOD group resident. Runs once per model arrival,
    /// never per frame.
    pub fn upload_model(
        &self,
        queue: &wgpu::Queue,
        vertex_at: u32,
        vertices: &[Vertex],
        index_at: u32,
        indices: &[u32],
        object_at: u32,
        objects: &[GpuObject],
        group: u32,
    ) {
        queue.write_buffer(&self.vertex_buf, vertex_at as u64 * std::mem::size_of::<Vertex>() as u64, bytemuck::cast_slice(vertices));
        queue.write_buffer(&self.index_buf, index_at as u64 * 4, bytemuck::cast_slice(indices));
        for k in 0..self.multiplier {
            let copies: Vec<GpuObject>;
            let data: &[GpuObject] = if k == 0 {
                objects
            } else {
                let d = scene::copy_offset(k);
                copies = objects
                    .iter()
                    .map(|o| GpuObject {
                        min: [o.min[0] + d[0], o.min[1] + d[1], o.min[2] + d[2]],
                        max: [o.max[0] + d[0], o.max[1] + d[1], o.max[2] + d[2]],
                        copy: k,
                        ..*o
                    })
                    .collect();
                &copies
            };
            let at = (k * self.base_object_count + object_at) as u64 * std::mem::size_of::<GpuObject>() as u64;
            queue.write_buffer(&self.object_buf, at, bytemuck::cast_slice(data));
        }
        self.set_group_ready(queue, group, true);
    }

    pub fn set_group_ready(&self, queue: &wgpu::Queue, group: u32, ready: bool) {
        let at = group as u64 * std::mem::size_of::<GpuLodGroup>() as u64 + 16;
        queue.write_buffer(&self.group_buf, at, bytemuck::bytes_of(&(ready as u32)));
    }

    /// Write the kit maps into the texture arrays (once, when the bake is done).
    pub fn upload_kit(&self, queue: &wgpu::Queue, kit: &KitUpload) -> Result<()> {
        self.upload_kit_layers(queue, kit, 0..kit.layers.len())
    }

    /// Write some layers of the kit maps (spreads the upload over frames).
    pub fn upload_kit_layers(&self, queue: &wgpu::Queue, kit: &KitUpload, layers: std::ops::Range<usize>) -> Result<()> {
        if kit.layers.len() as u32 != self.texture_layers || (kit.res != self.texture_res && !kit.layers.is_empty()) {
            return Err(anyhow!(
                "kit upload of {} layers at {} px does not match the {} x {} px arrays",
                kit.layers.len(),
                kit.res,
                self.texture_layers,
                self.texture_res
            ));
        }
        for layer in layers {
            for (tex, chain) in self.kit_textures.iter().zip(kit.layers[layer].iter()) {
                write_array_layer(queue, tex, layer as u32, chain);
            }
        }
        Ok(())
    }

    /// build the agent renderer over a loaded library (once; later calls are ignored).
    pub fn attach_agents(&self, ctx: &GpuContext, lib: &eo_agents::library::Library) -> Result<()> {
        if self.agents.get().is_none() {
            let a = agents::AgentsGpu::new(ctx, &self.look, lib, self.sample_count, self.debug_ids, &self.occlusion.late_bgl)?; // + the pyramid's layout
            let _ = self.agents.set(a);
        }
        Ok(())
    }

    /// build the street prop renderer over a cooked library (once; later calls are ignored).
    pub fn attach_props(&self, ctx: &GpuContext, lib: &props_lib::PropLibrary) -> Result<()> {
        if self.props.get().is_none() {
            let p = props::PropsGpu::new(&self.props_init(ctx), lib)?;
            let _ = self.props.set(p);
        }
        Ok(())
    }

    /// what building the props renderer off this thread needs.
    pub fn props_init(&self, ctx: &GpuContext) -> props::PropsInit {
        props::PropsInit {
            device: ctx.device.clone(),
            queue: ctx.queue.clone(),
            main_bgl: self.look.main_bind_group_layout().clone(),
            shadow_views: self.look.shadow_view_buffer().clone(),
            late_bgl: self.occlusion.late_bgl.clone(),
            sample_count: self.sample_count,
            debug_ids: self.debug_ids,
        }
    }

    /// attach a props renderer built elsewhere (once; later calls are ignored).
    pub fn attach_props_gpu(&self, p: props::PropsGpu) {
        let _ = self.props.set(p);
    }

    /// build the sea over its shore field and spectrum (once; later calls are ignored).
    pub fn attach_water(&self, ctx: &GpuContext, init: &water::WaterInit) {
        if self.water.get().is_none() {
            let w = water::WaterGpu::new(&ctx.device, &ctx.queue, &self.look, &self.occlusion.late_bgl, init);
            let _ = self.water.set(w);
        }
    }

    /// build the surf strip over the beaches' strips (once, after the water; drawn with `EO_SURF=1`).
    pub fn attach_surf(&self, ctx: &GpuContext, strips: &[eo_water::surf_strip::Strip]) -> u64 {
        match self.water.get() {
            Some(w) if self.surf.get().is_none() && surf::enabled() && !strips.is_empty() => {
                let s = surf::SurfGpu::new(&ctx.device, &ctx.queue, &self.look, w, strips);
                let b = s.bytes;
                let _ = self.surf.set(s);
                b
            }
            _ => 0,
        }
    }

    /// build the far terrain over its cooked heightfield (once; later calls are ignored).
    pub fn attach_farshore(&self, ctx: &GpuContext, init: &farshore::FarInit) {
        if self.farshore.get().is_none() {
            let _ = self.farshore.set(farshore::FarShoreGpu::new(&ctx.device, &ctx.queue, &self.look, init));
        }
    }

    /// the far terrain is drawn this frame (attached, single sample, no debug ids).
    pub fn farshore_active(&self) -> bool {
        self.sample_count == 1 && !self.debug_ids && self.farshore.get().is_some()
    }

    /// Draw the sea into single-sample colour when attached and enabled.
    /// With fixed draw slots the dive preview activates over the wet shore mask.
    pub fn water_active(&self) -> bool {
        self.sample_count == 1 && !self.debug_ids && self.water.get().is_some_and(|w| {
            w.on() && (self.compact || (self.look.options.water_optics && w.fallback_dive()))
        })
    }

    /// allocate the street's scanned-set arrays at their size, write
    /// both sets (every mip) and rebind group 0. Returns the bytes allocated.
    pub fn upload_street_maps(&self, ctx: &GpuContext, maps: &eo_assets::street_maps::StreetMaps) -> u64 {
        let layers = maps.albedo.len() as u32;
        let (a, ab) = texture_array(ctx, "street-albedo", wgpu::TextureFormat::Rgba8UnormSrgb, maps.res, layers);
        let (n, nb) = texture_array(ctx, "street-nra", wgpu::TextureFormat::Rgba8Unorm, maps.res, layers);
        for l in 0..layers as usize {
            write_array_layer(&ctx.queue, &a, l as u32, &maps.albedo[l]);
            write_array_layer(&ctx.queue, &n, l as u32, &maps.nra[l]);
        }
        *self.street_textures.lock().unwrap() = [a, n];
        self.rebuild_draw_bgs(&ctx.device);
        ab + nb
    }

    /// write the facade-detail-v1 sheet (every kept level) and rebind
    /// group 0. Returns the bytes allocated.
    pub fn upload_facade_detail(&self, ctx: &GpuContext, detail: &eo_assets::facade_detail::FacadeDetail) -> u64 {
        let tex = facade::FacadeGpu::detail_texture(&ctx.device, &ctx.queue, detail.width, detail.height, &detail.levels);
        *self.facade.detail.lock().unwrap() = tex;
        self.rebuild_draw_bgs(&ctx.device);
        detail.levels.iter().map(|l| l.len() as u64).sum()
    }

    /// rebuild group 0 of the city pass and of the probe face pass
    /// (street sets or the detail sheet arrived). hitch2: both probe front cubes'
    /// variants, so a finished capture only switches the index (no bind group mid-play).
    pub fn rebuild_draw_bgs(&self, device: &wgpu::Device) {
        hitch::mark(hitch::Mark::DrawBindGroups, 1);
        hitch::mark(hitch::Mark::BindGroupCreated, 4);
        let st = self.street_textures.lock().unwrap();
        let kit = [&self.kit_textures[0], &self.kit_textures[1], &self.kit_textures[2]];
        let bg = [0, 1].map(|f| make_draw_bg(device, &self.draw_bgl, &self.frame_buf, &self.object_buf, &self._material_buf, kit, &self.kit_sampler, [&st[0], &st[1]], &self.street_sampler, &self.facade, &self.probe.cube_views[f], &self.probe.sampler, &self.probe.lobe_buf, &self.cc0));
        let pbg = [0, 1].map(|f| make_draw_bg(device, &self.draw_bgl, &self.probe.frame_buf, &self.object_buf, &self._material_buf, kit, &self.kit_sampler, [&st[0], &st[1]], &self.street_sampler, &self.facade, &self.probe.cube_views[f], &self.probe.sampler, &self.probe.lobe_buf, &self.cc0));
        *self.draw_bg.lock().unwrap() = bg;
        *self.probe_draw_bg.lock().unwrap() = pbg;
    }

    /// allocate the CC0 arrays, tables and decal buffers (when the adapter
    /// has BC compression) and rebind group 0. Returns the bytes allocated.
    pub fn attach_cc0(&self, ctx: &GpuContext) -> u64 {
        if !ctx.bc || self.cc0.attached() {
            return 0;
        }
        let b = self.cc0.attach(&ctx.device, self.material_count as usize);
        self.cc0.clear_headers(&ctx.queue);
        self.rebuild_draw_bgs(&ctx.device);
        b
    }

    /// every layer is written: the tables, the switch, and the arrays bound in group 0.
    pub fn cc0_finish(&self, queue: &wgpu::Queue, mats: &[[f32; 4]], layers: &[[f32; 4]]) {
        self.cc0.finish(queue, mats, layers);
        self.rebuild_draw_bgs(&self.look.device);
    }

    /// the runtime switches (`facade::RUN_*`).
    pub fn set_facade_flags(&self, flags: u32) {
        self.facade_flags.store(flags, std::sync::atomic::Ordering::Relaxed);
    }

    /// Overwrite one object's material on the GPU only (tests: a planted swap).
    pub fn debug_set_object_material(&self, queue: &wgpu::Queue, object: u32, material: u32) {
        let at = object as u64 * std::mem::size_of::<GpuObject>() as u64 + 44;
        queue.write_buffer(&self.object_buf, at, bytemuck::bytes_of(&material));
    }

    /// Upload this frame's camera (two small uniform writes; the only
    /// per-frame CPU data).
    pub fn update(&self, queue: &wgpu::Queue, cam: &Camera) {
        self.update_with(queue, cam, self.occlusion_on()); // occlusion
    }

    /// turn the two-phase occlusion cull on or off (from the next
    /// `update`). Off, or without the compacted lists, frames draw as before.
    pub fn set_occlusion(&self, on: bool) {
        self.occlusion_enabled.store(on && self.compact, std::sync::atomic::Ordering::Relaxed);
    }

    /// whether the next `update` turns occlusion on.
    pub fn occlusion_on(&self) -> bool {
        self.occlusion_enabled.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn update_with(&self, queue: &wgpu::Queue, cam: &Camera, occlusion: bool) {
        if !self.compact && self.look.options.water_optics && let Some(w) = self.water.get() {
            w.locate_fallback_dive(cam);
        }
        self.observe_view(cam, None); // the streamers' view (unjittered)
        // the coming frame's sub-pixel jitter (TAA); every view of the frame goes through it.
        self.look.sky.begin_frame(self.look.out_size());
        let jittered = Camera { jitter: self.look.sky.jitter(), ..*cam };
        let cam = &jittered;
        if let Some(f) = self.farshore.get().filter(|_| self.farshore_active()) {
            f.update(queue, cam, self.water_active()); // the far terrain's nodes and camera (jittered, as the city), and where the sea is drawn
        }
        self.occlusion_frame.store(occlusion, std::sync::atomic::Ordering::Relaxed); // occlusion
        let vp = cam.view_proj();
        let planes = frustum_planes(vp).map(|p| p.to_array());
        let params = CullParams {
            planes,
            cam: [cam.eye.x, cam.eye.y, cam.eye.z, TERRAIN_LOD_DISTANCE_M],
            object_count: self.object_count,
            compact: self.compact as u32,
            group_count: self.group_count,
            blend_base: self.object_count,
            blend_first: self.blend_first,
            hlod_enter: self.hlod_enter_m,
            hlod_exit: self.hlod_exit_m,
            _pad: 0,
            kit: self.kit_radii,
            street: self.street_radii,
            view_proj: vp.to_cols_array_2d(),                           // occlusion
            occ: [occlusion as u32, 2 * self.object_count, self.water_active() as u32, 0], // occlusion; z = the flat sea quad stands down
        };
        queue.write_buffer(&self.cull_params_buf, 0, bytemuck::bytes_of(&params));
        if let Some(h) = self.hlod.get() {
            h.update(queue, vp, planes, cam.eye); // the far layer's cull, the same camera
        }
        if let Some(l) = self.hlod_lights.get() {
            l.update(queue, cam.fov_y_deg, self.look.out_size().1, true); // stage 3
        }
        if let Some(t) = self.hlod_trees.get() {
            t.update(queue, true, vp, planes, cam.eye); // stage 4, its blocks culled with the same camera
        }
        // A fixed afternoon sun from the south-west, for readability only.
        let sun = glam::Vec3::new(-0.45, 0.75, 0.48).normalize();
        let probe_on = self.compact && self.facade_flags.load(std::sync::atomic::Ordering::Relaxed) & facade::RUN_PROBE != 0; // facade
        let frame = FrameUniform {
            view_proj: vp.to_cols_array_2d(),
            cam_pos: cam.eye.extend(1.0).to_array(),
            sun_dir: sun.extend(0.0).to_array(),
            sun_color: [1.0, 0.96, 0.9, 0.0],
            ambient_sky: [0.32, 0.36, 0.42, 0.0],
            // where the front probe cube was captured (w 1 in use).
            probe_info: match (self.probe.front().1, probe_on) {
                (Some(p), true) => [p.x, p.y, p.z, 1.0],
                _ => [0.0; 4],
            },
            copy_step: [if self.multiplier > 1 { scene::COPY_STEP_M } else { 0.0 }, self.facade_flags.load(std::sync::atomic::Ordering::Relaxed) as f32, 0.0, self.cc0.run_bits() as f32], // y = switches; w = its switches
        };
        queue.write_buffer(&self.frame_buf, 0, bytemuck::bytes_of(&frame));
        // the probe's face for this frame (compacted lists only).
        {
            let sky_up = self.look.frame().sky_irradiance_up;
            let probe_eye = if self.look.options.cove_detail && self.water_active() {
                self.water.get().map_or(cam.eye, |w| w.cove_reflection_eye(cam.eye))
            } else { cam.eye };
            let _span = hitch::span(hitch::Phase::ProbeUpdate);
            self.probe.update(queue, probe_eye, self.look.time_of_day(), sky_up, probe_on, bytemuck::cast_ref(&frame));
        }
        // the rig is solved for the probe's measured transfer (keyFillBalance
        // with `bakedProbeDiffuseYield(model, transfer)`), else the JS fallback yield.
        let yield_on = self.facade_flags.load(std::sync::atomic::Ordering::Relaxed) & facade::RUN_PROBE_YIELD != 0;
        self.look.set_transfer(if frame.probe_info[3] > 0.5 && yield_on { self.probe.stats().transfer } else { None });
        self.look.update(queue, cam, frame.copy_step[0]); // look
        // no probe face on a frame that refreshes the middle or far
        // cascade or streams a large upload; the face waits for the next frame.
        let uploaded = self.frame_upload_bytes.swap(0, std::sync::atomic::Ordering::Relaxed);
        let fx = self.facade_flags.load(std::sync::atomic::Ordering::Relaxed);
        if fx & facade::RUN_PROBE_DEFER != 0 && (self.look.encoded_shadow_mask() & 0b110 != 0 || uploaded >= PROBE_DEFER_UPLOAD_BYTES) {
            self.probe.defer_this_frame();
        }
        if let Some(a) = self.agents.get() {
            a.set_camera_late(occlusion && self.compact && fx & facade::RUN_AGENT_HZB != 0); // the camera view against this frame's pyramid
            a.update(queue, cam, &self.look, 0.0); // agents
        }
        if let Some(p) = self.props.get() {
            p.update(queue, cam, &self.look, occlusion && self.compact); // props
        }
        if let Some(w) = self.water.get() {
            // the grid, the clock, the gate, the uniforms (the probe as the mirror's fallback).
            let probe = (frame.probe_info[3] > 0.5).then(|| glam::Vec3::new(frame.probe_info[0], frame.probe_info[1], frame.probe_info[2]));
            let wx = self.look.sky.weather(); // the sea state follows the hour and the weather
            w.set_sea(Some(water::SeaInput::Weather { hour: self.look.time_of_day(), wind: wx.wind, severity: wx.severity }));
            let clock = if self.look.options.water_optics { self.look.sky.world_time() } else { 0.0 };
            w.update_at_time(queue, cam, probe, self.water_active(), self.look.frame().aerial.far as f32, clock);
        }
        if let Some(s) = self.surf.get() {
            let swell_hs = self.water.get().and_then(|w| w.plan()).map(|p| p.state.swell.hs as f32).unwrap_or(1.8); // the swell's Hs scales the incoming surf
            s.update(queue, cam, self.water_active(), 0.0, swell_hs); // the strips in view, their columns
        }
    }

    fn encode_cull(&self, enc: &mut wgpu::CommandEncoder, ts: Option<(&wgpu::QuerySet, u32)>) {
        enc.clear_buffer(&self.counter_buf, 0, None);
        let hlod = self.hlod.get(); // hlod
        if let Some(h) = hlod {
            h.clear(enc);
        }
        if let Some(t) = self.hlod_trees.get() {
            t.clear(enc); // stage 4
        }
        let tw = ts.map(|(q, base)| wgpu::ComputePassTimestampWrites {
            query_set: q,
            beginning_of_pass_write_index: Some(base),
            end_of_pass_write_index: Some(base + 1),
        });
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("lod+cull"), timestamp_writes: tw });
        pass.set_bind_group(0, &self.cull_bg, &[]);
        if self.group_count > 0 {
            pass.set_pipeline(&self.lod_pipeline);
            pass.dispatch_workgroups(self.group_count.div_ceil(64), 1, 1);
        }
        pass.set_pipeline(&self.cull_pipeline);
        pass.dispatch_workgroups(self.object_count.div_ceil(64), 1, 1);
        // without occlusion the far layer is culled here (after the LOD
        // pass wrote the state); with it, after the pyramid (encode_with_ids).
        if let Some(h) = hlod {
            if !(self.compact && self.occlusion_frame.load(std::sync::atomic::Ordering::Relaxed)) {
                h.encode_cull(&mut pass, None);
            }
        }
        if let Some(t) = self.hlod_trees.get() {
            if !(self.compact && self.occlusion_frame.load(std::sync::atomic::Ordering::Relaxed)) {
                t.encode_cull(&mut pass, None); // stage 4, the tree blocks
            }
        }
    }

    /// Record one frame: LOD + cull pass, then one render pass with one
    /// indirect multi-draw per pipeline (opaque, then transparent).
    /// `resolve` is the MSAA resolve target when sample_count > 1; `ids` the
    /// Rg32Uint target when the renderer was made with `debug_ids`.
    pub fn encode(
        &self,
        enc: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        resolve: Option<&wgpu::TextureView>,
        depth: &wgpu::TextureView,
        ts: Option<(&wgpu::QuerySet, u32)>,
    ) {
        self.encode_with_ids(enc, color, resolve, depth, None, ts)
    }

    pub fn encode_with_ids(
        &self,
        enc: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        resolve: Option<&wgpu::TextureView>,
        depth: &wgpu::TextureView,
        ids: Option<&wgpu::TextureView>,
        ts: Option<(&wgpu::QuerySet, u32)>,
    ) {
        self.encode_cull(enc, ts);
        // sun shadow cull + depth passes; the city renders into the HDR
        // target (resolving MSAA into it) and post writes the output.
        self.look.encode_shadows(enc, &self.vertex_buf, &self.index_buf, ts);
        if let Some(a) = self.agents.get() {
            a.encode(enc, &self.look); // cull + LOD compute, casters into the refreshed cascades
        }
        if let Some(p) = self.props.get() {
            p.encode_pre(enc, &self.look); // cascade cull + casters (and the camera cull with occlusion off)
        }
        // the scene probe's face for this frame (after the shadows it is lit with).
        {
            let pbg = self.probe_draw_bg.lock().unwrap()[self.probe.front().0].clone();
            // a finished capture flips the front cube; the city pass below
            // takes that cube's bind group (made in advance), nothing is rebuilt.
            let _flipped = self.probe.encode(enc, &self.vertex_buf, &self.index_buf, &pbg, &self.look, ts.map(|(q, b)| (q, b + Q_PROBE)));
        }
        // the frame's lights (lamps, headlamps) gathered and clustered before the city reads them.
        self.look.encode_sky_lights(enc, self.agents.get(), self.props.get(), ts);
        let output = resolve.unwrap_or(color);
        let hdr = self.look.hdr_target(output.texture().width(), output.texture().height());
        let (color, resolve) = if resolve.is_some() { (color, Some(&hdr)) } else { (&hdr, None) };
        // the occlusion pass reads the depth after the city pass.
        // only a single-sample depth (the pyramid also reads MSAA depth, so usage alone no longer says it).
        let keep_depth = (self.look.options.ao || self.look.sky_wants_depth()) && depth.texture().usage().contains(wgpu::TextureUsages::TEXTURE_BINDING) && depth.texture().sample_count() == 1; // + the sky passes
        // the wire pass after the late city pass depth-tests against it.
        let keep_depth = keep_depth || self.props.get().is_some_and(|p| p.wires_pending());
        let draw_bg = self.draw_bg.lock().unwrap()[self.probe.front().0].clone(); // the front cube's
        // with occlusion on the city is two passes, the early
        // (objects visible last frame, depth kept for the pyramid) and the
        // late (newly visible objects, agents, the sky, the glass); off, one pass.
        let occlusion = self.compact && self.occlusion_frame.load(std::sync::atomic::Ordering::Relaxed);
        // with the water pass on, the glass is drawn after the water (its own pass below).
        let water = self.water.get().filter(|_| self.water_active());
        let water_on = water.is_some();
        let city_pass = |enc: &mut wgpu::CommandEncoder, late: bool, last: bool, ts_base: u32| {
            let tw = ts.map(|(q, base)| wgpu::RenderPassTimestampWrites {
                query_set: q,
                beginning_of_pass_write_index: Some(base + ts_base),
                end_of_pass_write_index: Some(base + ts_base + 1),
            });
            let mut attachments = vec![Some(wgpu::RenderPassColorAttachment {
                view: color,
                depth_slice: None,
                resolve_target: if last { resolve } else { None },
                ops: wgpu::Operations {
                    load: if late { wgpu::LoadOp::Load } else { wgpu::LoadOp::Clear(wgpu::Color { r: 0.55, g: 0.66, b: 0.78, a: 1.0 }) },
                    store: wgpu::StoreOp::Store,
                },
            })];
            if self.debug_ids {
                let v = ids.expect("renderer made with debug_ids needs an ids target");
                attachments.push(Some(wgpu::RenderPassColorAttachment {
                    view: v,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: if late { wgpu::LoadOp::Load } else { wgpu::LoadOp::Clear(wgpu::Color { r: u32::MAX as f64, g: u32::MAX as f64, b: 0.0, a: 0.0 }) },
                        store: wgpu::StoreOp::Store,
                    },
                }));
            }
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(if late { "city-late" } else { "city" }),
                color_attachments: &attachments,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
                    depth_ops: Some(wgpu::Operations {
                        load: if late { wgpu::LoadOp::Load } else { wgpu::LoadOp::Clear(0.0) },
                        // (kept for the pyramid, the AO pass, or a debug-ids renderer's tests)
                        store: if last && !keep_depth && !self.debug_ids && !water_on { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store }, // the water pass reads it
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: tw,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &draw_bg, &[]);
            self.look.bind_main(&mut pass); // look
            pass.set_vertex_buffer(0, self.vertex_buf.slice(..));
            pass.set_index_buffer(self.index_buf.slice(..), wgpu::IndexFormat::Uint32);
            let n = self.object_count as u64;
            let blend_offset = n * DRAW_ARGS_BYTES;
            // In the fallback the transparent list has a slot only for the objects
            // from `blend_first` on (the first possible glass, static or streamed).
            let slots = [self.object_count, self.object_count - self.blend_first];
            // (pipeline, list offset, count word): the early pass draws the
            // opaque list; the late pass the late opaque list, then the glass.
            let lists: &[(&wgpu::RenderPipeline, u64, u64)] = match (occlusion, late) {
                (false, _) => &[(&self.opaque_pipeline, 0, 0), (&self.blend_pipeline, blend_offset, 4)],
                (true, false) => &[(&self.opaque_pipeline, 0, 0)],
                (true, true) => &[(&self.opaque_pipeline, 2 * n * DRAW_ARGS_BYTES, 40), (&self.blend_pipeline, blend_offset, 4)],
            };
            for &(pipeline, offset, counter) in lists {
                if std::ptr::eq(pipeline, &self.blend_pipeline) {
                    if let Some(h) = self.hlod.get() {
                        // the far layer after the opaque city (the late pass with occlusion), before the sky.
                        h.draw(&mut pass);
                        pass.set_vertex_buffer(0, self.vertex_buf.slice(..));
                        pass.set_index_buffer(self.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                    }
                    if let Some(t) = self.hlod_trees.get() {
                        t.draw(&mut pass);
                    }
                    if let Some(p) = self.props.get() {
                        // the street props after the opaque city (late pass with occlusion).
                        p.draw(&mut pass, &self.look);
                        pass.set_vertex_buffer(0, self.vertex_buf.slice(..));
                        pass.set_index_buffer(self.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                    }
                    if let Some(a) = self.agents.get() {
                        // moving things after the opaque city, then the city buffers again.
                        a.draw_opaque(&mut pass, &self.look);
                        pass.set_vertex_buffer(0, self.vertex_buf.slice(..));
                        pass.set_index_buffer(self.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                    }
                    if let Some(f) = self.farshore.get().filter(|_| self.farshore_active()) {
                        // the far terrain behind the city (its depth past the far plane), before the sky and the sea.
                        f.draw(&mut pass, &self.look);
                        pass.set_vertex_buffer(0, self.vertex_buf.slice(..));
                        pass.set_index_buffer(self.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                    }
                    // the sky where nothing opaque was drawn, before the glass.
                    self.look.draw_sky(&mut pass);
                    pass.set_bind_group(0, &draw_bg, &[]);
                    if water_on {
                        continue; // the glass after the water pass
                    }
                    if let Some(l) = self.hlod_lights.get() {
                        // stage 3, the far lamps over the sky and the city (with water: after the water pass).
                        self.look.bind_main(&mut pass);
                        l.draw(&mut pass);
                    }
                }
                pass.set_pipeline(pipeline);
                if self.compact {
                    pass.multi_draw_indexed_indirect_count(&self.draw_buf, offset, &self.counter_buf, counter, self.object_count);
                } else {
                    let k = (offset != 0) as usize;
                    if slots[k] > 0 {
                        pass.multi_draw_indexed_indirect(&self.draw_buf, offset, slots[k]);
                    }
                }
            }
            if last && !water_on {
                if let Some(a) = self.agents.get() {
                    a.draw_glass(&mut pass, &self.look); // glass and lenses after the city glass
                }
                if let Some(l) = self.landmarks.get() {
                    // the wire members where the glass is drawn.
                    pass.set_bind_group(0, &draw_bg, &[]);
                    self.look.bind_main(&mut pass);
                    l.draw_wires(&mut pass);
                }
            }
        };
        if occlusion {
            city_pass(enc, false, false, 2);
            {
                let tw = ts.map(|(q, b)| wgpu::ComputePassTimestampWrites {
                    query_set: q,
                    beginning_of_pass_write_index: Some(b + Q_OCCLUSION),
                    end_of_pass_write_index: Some(b + Q_OCCLUSION + 1),
                });
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("hzb+cull-late"), timestamp_writes: tw });
                self.occlusion.build(&mut pass, depth, |pass, hzb_bg| {
                    pass.set_pipeline(&self.late_pipeline);
                    pass.set_bind_group(0, &self.cull_bg, &[]);
                    pass.set_bind_group(1, hzb_bg, &[]);
                    pass.dispatch_workgroups(self.object_count.div_ceil(64), 1, 1);
                    if let Some(p) = self.props.get() {
                        p.cull_camera(pass, hzb_bg); // the camera view against this frame's pyramid
                    }
                    if let Some(a) = self.agents.get() {
                        a.cull_camera(pass, hzb_bg); // moving agents against this frame's pyramid
                    }
                    if let Some(w) = water {
                        w.cull_hzb(pass, hzb_bg); // the grid patches against this frame's pyramid
                    }
                    if let Some(h) = self.hlod.get() {
                        h.encode_cull(pass, Some(hzb_bg)); // the far cells against this frame's pyramid
                    }
                    if let Some(t) = self.hlod_trees.get() {
                        t.encode_cull(pass, Some(hzb_bg)); // stage 4, the tree blocks against it too
                    }
                });
            }
            city_pass(enc, true, true, Q_LATE_DRAW);
        } else {
            city_pass(enc, false, true, 2);
            // The occlusion slots are still written (empty passes), so every
            // query a frame resolves is defined.
            if let Some((q, b)) = ts {
                for slot in [Q_OCCLUSION, Q_LATE_DRAW] {
                    enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("occlusion off"),
                        timestamp_writes: Some(wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(b + slot), end_of_pass_write_index: Some(b + slot + 1) }),
                    });
                }
            }
        }
        // the FFT (+ the plain patch cull), the frame copy and the water
        // pass, the city's and the agents' glass drawn over it in the same pass.
        if let Some(w) = water {
            w.encode_compute(enc, occlusion, ts.map(|(q, b)| (q, b + Q_WATER_FFT)));
            let (front, _) = self.probe.front();
            // The glass in the same pass, after the water (it tests the read-only depth, never writes it).
            w.encode_draw(enc, &self.look, &hdr, depth, [&self.probe.cube_views[0], &self.probe.cube_views[1]], front, /* hitch2: both cubes */ ts.map(|(q, b)| (q, b + Q_WATER)), |pass| {
                pass.set_bind_group(0, &draw_bg, &[]);
                self.look.bind_main(pass);
                pass.set_vertex_buffer(0, self.vertex_buf.slice(..));
                pass.set_index_buffer(self.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                pass.set_pipeline(&self.blend_pipeline);
                if self.compact {
                    pass.multi_draw_indexed_indirect_count(&self.draw_buf, self.object_count as u64 * DRAW_ARGS_BYTES, &self.counter_buf, 4, self.object_count);
                } else if self.object_count > self.blend_first {
                    pass.multi_draw_indexed_indirect(&self.draw_buf, self.object_count as u64 * DRAW_ARGS_BYTES, self.object_count - self.blend_first);
                }
                if let Some(a) = self.agents.get() {
                    a.draw_glass(pass, &self.look);
                }
                if let Some(l) = self.landmarks.get() {
                    // the wire members over the water, with the glass.
                    pass.set_bind_group(0, &draw_bg, &[]);
                    self.look.bind_main(pass);
                    l.draw_wires(pass);
                }
                if let Some(l) = self.hlod_lights.get() {
                    // stage 3, the far lamps after the water.
                    pass.set_bind_group(0, &draw_bg, &[]);
                    self.look.bind_main(pass);
                    l.draw(pass);
                }
            });
            if let Some(s) = self.surf.get() {
                s.encode(enc, w, &self.look, &hdr, front); // the surf strip over the sea, its own depth
            }
        } else if let Some((q, b)) = ts {
            // Every slot a frame resolves is written (empty passes).
            for slot in [Q_WATER_FFT, Q_WATER] {
                enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("water off"),
                    timestamp_writes: Some(wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(b + slot), end_of_pass_write_index: Some(b + slot + 1) }),
                });
            }
        }
        if let Some(p) = self.props.get() {
            p.draw_wires(enc, color, resolve, depth, &self.look); // the overhead wires, a timed pass of their own
        }
        self.look.encode_ao(enc, depth, &hdr, ts); // surfaces
        self.look.encode_sky_after(enc, depth, &hdr, self.agents.get(), ts); // rain, luminaires, velocity, volume, resolve, bloom
        self.look.encode_post_with_water(enc, output, ts, water.filter(|_| self.look.options.water_optics).map(|w| (w, self.probe.front().0)));
    }

    /// The counter buffer: see `COUNTER_WORDS`.
    pub fn counter_buffer(&self) -> &wgpu::Buffer {
        &self.counter_buf
    }

    /// Run only the LOD + cull passes for `cam` and read back what they wrote.
    /// For tests and diagnostics; blocks.
    pub fn cull_readback(&self, ctx: &GpuContext, cam: &Camera) -> Result<CullReadback> {
        self.update_with(&ctx.queue, cam, false); // the frustum + LOD reference
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        self.encode_cull(&mut enc, None);
        let draw_bytes = self.object_count as u64 * 2 * DRAW_ARGS_BYTES;
        let cb = (COUNTER_WORDS * 4) as u64;
        let staging = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cull-readback"),
            size: draw_bytes + cb,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        enc.copy_buffer_to_buffer(&self.counter_buf, 0, &staging, 0, cb);
        enc.copy_buffer_to_buffer(&self.draw_buf, 0, &staging, cb, draw_bytes);
        pace::submit(&ctx.queue, [enc.finish()]); // counted
        let bytes = read_mapped(ctx, &staging)?;
        let words: Vec<u32> = bytemuck::pod_collect_to_vec(&bytes);
        let mut counters = [0u32; COUNTER_WORDS];
        counters.copy_from_slice(&words[..COUNTER_WORDS]);
        let n = self.object_count as usize;
        let (opaque_slots, blend_slots) =
            if self.compact { (counters[0] as usize, counters[1] as usize) } else { (n, n - self.blend_first as usize) };
        let cmds = &words[COUNTER_WORDS..];
        let parse = |from: usize, count: usize| -> Vec<[u32; 5]> {
            cmds[from * 5..(from + count) * 5].chunks_exact(5).map(|c| [c[0], c[1], c[2], c[3], c[4]]).collect()
        };
        Ok(CullReadback { counters, draws: parse(0, opaque_slots), blend_draws: parse(n, blend_slots) })
    }

    /// after a submitted frame (`encode`), the counters and every
    /// object index that frame's lists drew (early opaque, late opaque,
    /// transparent). Compacted lists only. Tests and diagnostics; blocks.
    pub fn drawn_readback(&self, ctx: &GpuContext) -> Result<([u32; COUNTER_WORDS], Vec<u32>)> {
        if !self.compact {
            return Err(anyhow!("drawn_readback needs the compacted lists"));
        }
        let n = self.object_count as u64;
        let cb = (COUNTER_WORDS * 4) as u64;
        let lists = n * 3 * DRAW_ARGS_BYTES;
        let staging = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("drawn-readback"),
            size: cb + lists,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(&self.counter_buf, 0, &staging, 0, cb);
        enc.copy_buffer_to_buffer(&self.draw_buf, 0, &staging, cb, lists);
        pace::submit(&ctx.queue, [enc.finish()]); // counted
        let words: Vec<u32> = bytemuck::pod_collect_to_vec(&read_mapped(ctx, &staging)?);
        let mut counters = [0u32; COUNTER_WORDS];
        counters.copy_from_slice(&words[..COUNTER_WORDS]);
        let cmds = &words[COUNTER_WORDS..];
        let n = n as usize;
        let mut drawn = vec![];
        let late = if self.occlusion_frame.load(std::sync::atomic::Ordering::Relaxed) { counters[10] as usize } else { 0 };
        for (from, count) in [(0, counters[0] as usize), (n, counters[1] as usize), (2 * n, late)] {
            drawn.extend(cmds[from * 5..(from + count) * 5].chunks_exact(5).filter(|c| c[1] == 1).map(|c| c[4]));
        }
        Ok((counters, drawn))
    }

    /// Read the LOD groups back (tests).
    pub fn groups_readback(&self, ctx: &GpuContext) -> Result<Vec<GpuLodGroup>> {
        let size = self.group_buf.size();
        let staging = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("groups-readback"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(&self.group_buf, 0, &staging, 0, size);
        pace::submit(&ctx.queue, [enc.finish()]); // counted
        Ok(bytemuck::pod_collect_to_vec(&read_mapped(ctx, &staging)?))
    }

    /// the far layer's per-group state as the last LOD pass wrote it (tests).
    pub fn hlod_state_readback(&self, ctx: &GpuContext) -> Result<Vec<u32>> {
        let size = self.hlod_state_buf.size();
        let staging = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: Some("hlod-state-readback"), size, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(&self.hlod_state_buf, 0, &staging, 0, size);
        pace::submit(&ctx.queue, [enc.finish()]);
        Ok(bytemuck::pod_collect_to_vec(&read_mapped(ctx, &staging)?))
    }

    /// set the per-cell residency words directly (tests; the far layer's streamer writes them otherwise).
    pub fn write_hlod_flags(&self, queue: &wgpu::Queue, flags: &[u32]) {
        queue.write_buffer(&self.hlod_flags_buf, 0, bytemuck::cast_slice(flags));
    }

    /// Read the object table back (tests).
    pub fn objects_readback(&self, ctx: &GpuContext) -> Result<Vec<GpuObject>> {
        let size = self.object_buf.size();
        let staging = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("objects-readback"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(&self.object_buf, 0, &staging, 0, size);
        pace::submit(&ctx.queue, [enc.finish()]); // counted
        Ok(bytemuck::pod_collect_to_vec(&read_mapped(ctx, &staging)?))
    }

    /// bytes of a streamed target's buffer (tests: what the upload queue wrote).
    pub fn buffer_readback(&self, ctx: &GpuContext, target: upload::Target, offset: u64, len: u64) -> Result<Vec<u8>> {
        let buf = self.upload_buffer(target).ok_or_else(|| anyhow!("{target:?} has no buffer"))?;
        let staging = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: Some("buffer-readback"), size: len, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(buf, offset, &staging, 0, len);
        pace::submit(&ctx.queue, [enc.finish()]); // counted
        read_mapped(ctx, &staging)
    }

    /// read back a whole texture subresource a streamed write lands
    /// in, rows packed tightly (for the upload test).
    pub fn texture_readback(&self, ctx: &GpuContext, target: upload::Target) -> Result<Vec<u8>> {
        let t = self.upload_texture(target).ok_or_else(|| anyhow!("{target:?} is not an upload texture"))?;
        let row = t.row_bytes();
        let stride = row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as u64);
        let staging = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: Some("texture-readback"), size: stride * t.rows() as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: t.texture, mip_level: t.mip, origin: wgpu::Origin3d { x: 0, y: 0, z: t.layer }, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &staging, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(stride as u32), rows_per_image: Some(t.rows()) } },
            wgpu::Extent3d { width: t.width.next_multiple_of(t.block), height: t.rows() * t.block, depth_or_array_layers: 1 },
        );
        pace::submit(&ctx.queue, [enc.finish()]); // counted
        let padded = read_mapped(ctx, &staging)?;
        Ok(padded.chunks(stride as usize).flat_map(|r| &r[..row as usize]).copied().collect())
    }
}

/// a texture subresource a streamed write lands in: `width` x
/// `height` texels, rows packed tightly in the payload. cc0: a row is a row
/// of `block` x `block` blocks of `texel_bytes` (block 1 for the kit's RGBA8,
/// 4 for BC7 / BC5).
pub struct UploadTexture<'a> {
    pub texture: &'a wgpu::Texture,
    pub mip: u32,
    pub layer: u32,
    pub width: u32,
    pub height: u32,
    /// Bytes of one block (one texel for an uncompressed format).
    pub texel_bytes: u32,
    /// block side in texels.
    pub block: u32,
}

impl<'a> UploadTexture<'a> {
    /// One mip of one layer of `tex` (None: out of range, or a non-square block).
    pub fn of(tex: &'a wgpu::Texture, mip: u32, layer: u32) -> Option<Self> {
        if mip >= tex.mip_level_count() || layer >= tex.depth_or_array_layers() {
            return None;
        }
        let (bw, bh) = tex.format().block_dimensions();
        if bw != bh {
            return None;
        }
        let texel_bytes = tex.format().block_copy_size(None)?;
        Some(Self { texture: tex, mip, layer, width: (tex.width() >> mip).max(1), height: (tex.height() >> mip).max(1), texel_bytes, block: bw })
    }

    /// Bytes of one row of blocks.
    pub fn row_bytes(&self) -> u64 {
        self.width.div_ceil(self.block) as u64 * self.texel_bytes as u64
    }

    /// Rows of blocks.
    pub fn rows(&self) -> u32 {
        self.height.div_ceil(self.block)
    }

    pub fn bytes(&self) -> u64 {
        self.row_bytes() * self.rows() as u64
    }
}

/// Map a MAP_READ buffer, wait, copy out, unmap.
pub fn read_mapped(ctx: &GpuContext, buf: &wgpu::Buffer) -> Result<Vec<u8>> {
    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    ctx.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| anyhow!("poll: {e}"))?;
    rx.recv()?.map_err(|e| anyhow!("map: {e}"))?;
    let out = slice.get_mapped_range()?.to_vec();
    buf.unmap();
    Ok(out)
}

/// Depth (and optional MSAA colour) targets for a size.
pub struct Targets {
    pub depth: wgpu::TextureView,
    pub msaa: Option<wgpu::TextureView>,
}

impl Targets {
    pub fn new(device: &wgpu::Device, w: u32, h: u32, color_format: wgpu::TextureFormat, sample_count: u32) -> Self {
        let mk = |format, label| {
            // the depth target is read by the pyramid build.
            let usage = if format == DEPTH_FORMAT { wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING } else { wgpu::TextureUsages::RENDER_ATTACHMENT };
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    // single-sample depth is read by the AO pass;
                    // any depth (MSAA too) is read by the pyramid build.
                    usage,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        Self {
            depth: mk(DEPTH_FORMAT, "depth"),
            // the city renders HDR, so its MSAA target is HDR (color_format is the post's output).
            msaa: (sample_count > 1).then(|| mk(if color_format == look::HDR_FORMAT { color_format } else { look::HDR_FORMAT }, "msaa-color")),
        }
    }
}
