//! Sun shadows past the cascades (`LightingOptions::far_shadows`: 1 one
//! ring, 2 two rings): a cached depth map a ring, sampled past the cascades
//! through group 1 binding 12 (`LookGpu::set_far_shadow_map`), its fits and
//! params in `Look.lx[LX_FAR_SHADOW_VP..=LX_FAR_SHADOW_PARAMS]` (far_shadow.wgsl).
//!
//! Rings (eo-look `shadow::fit_far_ring`): ring 1 is 2 km across round a
//! street-level eye and grows with its height (`far_ring_radius`: about 6.7
//! times the height, up to the far plane), ring 2 (hyper) reaches the far
//! plane; each a `FAR_MAP_SIZE`
//! square Depth16 map fitted in light space (across the sun the disc's
//! diameter, up the light's frame the disc foreshortened plus the scene's
//! heights), its centre snapped to whole texels.
//!
//! Each ring has two layers: the front, which the frame samples, and the
//! back, which a redraw fills one tile a frame (`TILES` x `TILES` scissored
//! tiles, each culled to its own part of the box) and which becomes the front
//! once its last tile is drawn. A ring is redrawn only when the key light has
//! turned `SUN_TURN_DEG` from its fit or the camera has moved `RECENTRE_SHARE`
//! of the ring's radius from it; otherwise the front is kept frame after
//! frame. At most one tile is drawn a frame (ring 1 first).
//!
//! Casters (far_shadow_cull.wgsl): each building once, as its footprint prism
//! or, while the far layer holds it, its cell's mesh (hlod_far_shadow.wgsl,
//! the far layer's vertex layout); the terrain as the camera draws it; the
//! grade-separated roads' concrete at its coarse level; landmarks' far items.
//! Far trees and the far shore do not cast. The casters' depth gets a
//! slope-scaled bias in the rasterizer (`CASTER_SLOPE_SCALE`), the receivers
//! a normal offset and a pull-back (eo-look `recommend_bias` at the ring's
//! texel).
//!
//! The culls run every frame for every ring in use (the tile being drawn, or
//! the whole box of the fit the ring would take now), so the count
//! emulation's bounds (counted.rs) always know the lists' size; they are in
//! the renderer's recovery cull too. Everything is timed in
//! `lighting::Q_FAR_SHADOW`.

use crate::camera::Camera;
use crate::lighting::{self, Ts};
use crate::look::{LX_FAR_SHADOW_PARAMS, LX_FAR_SHADOW_VP};
use eo_look::shadow::{self as fit, FarRing};
/// The maps' size and the rings' radii (eo-look `shadow`).
pub use eo_look::shadow::{FAR_MAP_SIZE, FAR_RING_RADIUS_M};
use std::sync::{Mutex, OnceLock};

/// The lookup (look_light.wgsl `lk_shadow` and water.wgsl `w_shadow` call it).
pub const WGSL: &str = include_str!("far_shadow.wgsl");
const CULL_WGSL: &str = include_str!("far_shadow_cull.wgsl");
const CELL_WGSL: &str = include_str!("hlod_far_shadow.wgsl");
const OBJECT_WGSL: &str = include_str!("look_shadow_draw.wgsl");

/// The maps' depth format (the cascades', 16-bit: 8 km of light-space depth in
/// steps under 0.2 m against texels of 0.2-2 m and their bias).
pub const FORMAT: wgpu::TextureFormat = crate::look::SHADOW_FORMAT;
/// A redraw's tiles a side: `SLICES` frames a map.
pub const TILES: u32 = 2;
pub const SLICES: u32 = TILES * TILES;
/// A ring is redrawn once the key light has turned this far from its fit
/// (degrees; at the clock's pace about every 12 s). A 50 m building's shadow
/// tip at a 10 degree sun moves 1.4 m when the new map takes over.
pub const SUN_TURN_DEG: f64 = 0.05;
/// ... or the camera has moved this share of the ring's radius on the ground
/// (the texel grid stays put under a still sun, so only the ring's edge moves).
pub const RECENTRE_SHARE: f64 = 0.125;
/// A redraw under way starts again from this frame's fit when its own fit is
/// this far out (a cut, a clock jump): a share of the radius, degrees.
pub const RESTART_SHARE: f64 = 0.5;
pub const RESTART_TURN_DEG: f64 = 2.0;
/// A ring fades out with the ground distance from the eye, from this share of
/// its fade-out distance ((1 - RECENTRE_SHARE) x its radius: 875 m for the
/// 1 km ring) to all of it, to the next ring or to unshadowed: a long, soft
/// end with no line, which moves with the eye (no step when the map is
/// redrawn round a new centre).
pub const FADE_START_SHARE: f32 = 0.6;
/// The edge fade of a ring's box, a share of its map's uv (a safety net past the radial fade).
pub const EDGE_FADE_UV: f32 = 0.02;
/// The lookup's tap offset, texels (four bilinear compares: a 4 x 4 texel tent).
pub const TAP_TEXELS: f32 = 1.0;
/// The casters' slope-scaled depth bias (wgpu `DepthBiasState`, in depth
/// slopes a texel) and its clamp (depth units; 0.003 of the 6-15 km range).
pub const CASTER_SLOPE_SCALE: f32 = 2.0;
pub const CASTER_BIAS_CLAMP: f32 = 0.003;
const SLOT: u64 = 256;
/// Counter words: per ring object commands, object triangles, cell commands, cell triangles.
const COUNTER_WORDS: u64 = 8;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CullParams {
    planes: [[f32; 4]; 6],
    cam: [f32; 4],
    object_count: u32,
    compact: u32,
    cell_count: u32,
    ring: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct View {
    view_proj: [[f32; 4]; 4],
    copy_step: [f32; 4],
}

/// What the far maps did since they were attached.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FarStats {
    /// Redraws started (a ring's fit went stale), finished (its back became the front), tiles drawn.
    pub redraws_started: u64,
    pub maps_completed: u64,
    pub tiles_drawn: u64,
    /// Redraws started because of the sun, the camera, a first fit, an overflow of the emulated lists.
    pub for_sun: u64,
    pub for_camera: u64,
    pub for_first: u64,
    pub for_overflow: u64,
}

#[derive(Clone, Copy, Default)]
struct Ring {
    /// The fit the frame samples and its layer.
    front: Option<(FarRing, u32)>,
    /// A redraw under way: its fit, the next tile, its layer.
    back: Option<(FarRing, u32, u32)>,
}

#[derive(Clone, Copy)]
struct Plan {
    ring: usize,
    tile: u32,
    layer: u32,
    rect: [u32; 4],
}

#[derive(Default)]
struct State {
    rings: [Ring; 2],
    /// What the coming encode draws (set by `update`).
    plan: Option<Plan>,
    /// The rings whose culls the coming encode runs (0: nothing this frame).
    culls: usize,
    frozen: bool,
    overflow: bool,
    stats: FarStats,
}

struct Res {
    rings: usize,
    _map: wgpu::Texture,
    layers: Vec<wgpu::TextureView>,
    cull_buf: wgpu::Buffer,
    view_buf: wgpu::Buffer,
    draws: wgpu::Buffer,
    cell_draws: wgpu::Buffer,
    counts: wgpu::Buffer,
    lists: Option<crate::counted::CountedLists>,
    cull_bgl: wgpu::BindGroupLayout,
    cull_objects: wgpu::ComputePipeline,
    cull_cells: wgpu::ComputePipeline,
    depth_objects: wgpu::RenderPipeline,
    depth_bg: wgpu::BindGroup,
    cell_bgl: wgpu::BindGroupLayout,
    depth_cells: wgpu::RenderPipeline,
    dummy: wgpu::Buffer,
    /// The cull's bind group and the cells' depth bind group, made for the far layer as it is (attached or not).
    binds: Mutex<Option<(bool, wgpu::BindGroup, Option<wgpu::BindGroup>)>>,
    object_count: u32,
    compact: bool,
    bytes: u64,
}

pub struct FarShadowGpu {
    device: wgpu::Device,
    /// The scene's lowest and highest point (the rings' receivers and the casters' reach).
    scene_y: (f32, f32),
    res: OnceLock<Res>,
    state: Mutex<State>,
}

/// The lowest and highest point of a scene's objects (every one drawn: the
/// rings' receivers reach from the one to the other, and a caster's reach
/// toward the sun is the difference over the sun's altitude).
pub fn scene_heights(objects: &[crate::scene::GpuObject]) -> (f32, f32) {
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for o in objects.iter().filter(|o| o.index_count > 0) {
        lo = lo.min(o.min[1]);
        hi = hi.max(o.max[1]);
    }
    if lo > hi { (0.0, 1.0) } else { (lo, hi) }
}

impl FarShadowGpu {
    pub fn new(device: &wgpu::Device, scene_y: (f32, f32)) -> Self {
        Self { device: device.clone(), scene_y, res: OnceLock::new(), state: Mutex::new(State::default()) }
    }

    /// Whether the maps exist (`Renderer::attach_far_shadows`).
    pub fn attached(&self) -> bool {
        self.res.get().is_some()
    }

    /// GPU bytes allocated (the maps, lists and uniforms).
    pub fn bytes(&self) -> u64 {
        self.res.get().map(|r| r.bytes).unwrap_or(0)
    }

    pub fn stats(&self) -> FarStats {
        self.state.lock().unwrap().stats
    }

    /// Rings attached (0 without the maps).
    pub fn rings(&self) -> usize {
        self.res.get().map(|r| r.rings).unwrap_or(0)
    }

    /// Hold the maps as they are: no redraw, no swap (the occlusion check's
    /// two renders of one camera must see the same maps).
    pub fn set_frozen(&self, on: bool) {
        self.state.lock().unwrap().frozen = on;
    }

    /// Every ring in use has a front and no redraw is under way.
    pub fn settled(&self, level: u8) -> bool {
        let Some(res) = self.res.get() else { return true };
        let st = self.state.lock().unwrap();
        st.rings.iter().take((level as usize).min(res.rings)).all(|r| r.front.is_some() && r.back.is_none())
    }

    /// Whether the coming encode draws a tile (the probe waits for the next frame).
    pub fn draws_this_frame(&self) -> bool {
        self.state.lock().unwrap().plan.is_some()
    }

    /// The fronts' fits (ring order), for tests and reports.
    pub fn fronts(&self) -> Vec<Option<FarRing>> {
        self.state.lock().unwrap().rings.iter().map(|r| r.front.map(|f| f.0)).collect()
    }

    /// Make the maps (`rings` 1 or 2), their lists, pipelines and bind group 12. Returns the bytes allocated.
    pub(crate) fn attach(&self, r: &crate::Renderer, rings: u8) -> u64 {
        if self.res.get().is_some() {
            return 0;
        }
        let d = &self.device;
        let rings = rings.clamp(1, 2) as usize;
        let size = fit::FAR_MAP_SIZE;
        let map = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("far-sun-shadow"),
            size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 2 * rings as u32 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let layers: Vec<wgpu::TextureView> = (0..2 * rings as u32)
            .map(|l| map.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2), base_array_layer: l, array_layer_count: Some(1), ..Default::default() }))
            .collect();
        let array = map.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        let n = r.object_count.max(1) as u64;
        let cells = crate::hlod::MAX_CELLS as u64;
        let buf = |label: &str, size: u64, usage: wgpu::BufferUsages| d.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size.max(16).next_multiple_of(16), usage, mapped_at_creation: false });
        use wgpu::BufferUsages as U;
        let cull_buf = buf("far-shadow-cull", SLOT * rings as u64, U::UNIFORM | U::COPY_DST);
        let view_buf = buf("far-shadow-views", SLOT * rings as u64, U::UNIFORM | U::COPY_DST);
        let draws = buf("far-shadow-draws", rings as u64 * n * crate::DRAW_ARGS_BYTES, U::STORAGE | U::INDIRECT | U::COPY_DST);
        let cell_draws = buf("far-shadow-cell-draws", rings as u64 * cells * crate::DRAW_ARGS_BYTES, U::STORAGE | U::INDIRECT | U::COPY_DST);
        let counts = buf("far-shadow-counts", COUNTER_WORDS * 4, U::STORAGE | U::INDIRECT | U::COPY_SRC | U::COPY_DST);
        let dummy = buf("far-shadow-no-cells", 64, U::STORAGE);
        let lists = r.counts.as_ref().filter(|_| r.compact).map(|c| {
            let mut l = vec![];
            for k in 0..rings as u64 {
                l.push(crate::counted::List { word: 4 * k as u32, draws: draws.clone(), offset: k * n * crate::DRAW_ARGS_BYTES, stride: crate::DRAW_ARGS_BYTES, max: n as u32, floor: None });
                l.push(crate::counted::List { word: 4 * k as u32 + 2, draws: cell_draws.clone(), offset: k * cells * crate::DRAW_ARGS_BYTES, stride: crate::DRAW_ARGS_BYTES, max: cells as u32, floor: None });
            }
            c.register("far-shadow", &counts, l)
        });
        // the culls.
        let cs = wgpu::ShaderStages::COMPUTE;
        let vs = wgpu::ShaderStages::VERTEX;
        let e = |binding, visibility, ty| wgpu::BindGroupLayoutEntry { binding, visibility, ty, count: None };
        let ubuf_dyn = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: None };
        let sbuf = |read_only| wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None };
        let cull_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("far-shadow-cull-bgl"),
            entries: &[e(0, cs, ubuf_dyn), e(1, cs, sbuf(true)), e(2, cs, sbuf(false)), e(3, cs, sbuf(false)), e(4, cs, sbuf(true)), e(5, cs, sbuf(true)), e(6, cs, sbuf(true)), e(7, cs, sbuf(false))],
        });
        let cull_src = format!("{CULL_WGSL}\nconst FAR_MAX_CELLS: u32 = {}u;\n", crate::hlod::MAX_CELLS);
        let cull_module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("far_shadow_cull.wgsl"), source: wgpu::ShaderSource::Wgsl(cull_src.into()) });
        let cull_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("far-shadow-cull-layout"), bind_group_layouts: &[Some(&cull_bgl)], immediate_size: 0 });
        let compute = |entry: &str| {
            d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(entry), layout: Some(&cull_layout), module: &cull_module, entry_point: Some(entry), compilation_options: Default::default(), cache: None })
        };
        let (cull_objects, cull_cells) = (compute("far_cull_objects"), compute("far_cull_cells"));
        // the depth passes: the city's objects (look_shadow_draw.wgsl) and the far layer's cells.
        let bias = wgpu::DepthBiasState { constant: 0, slope_scale: CASTER_SLOPE_SCALE, clamp: CASTER_BIAS_CLAMP };
        let depth_state = wgpu::DepthStencilState { format: FORMAT, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::Less), stencil: Default::default(), bias };
        let obj_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("far-shadow-depth-bgl"), entries: &[e(0, vs, ubuf_dyn), e(1, vs, sbuf(true))] });
        let view_entry = |b: u32| wgpu::BindGroupEntry { binding: b, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &view_buf, offset: 0, size: Some(std::num::NonZeroU64::new(std::mem::size_of::<View>() as u64).unwrap()) }) };
        let depth_bg = d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("far-shadow-depth-bg"), layout: &obj_bgl, entries: &[view_entry(0), wgpu::BindGroupEntry { binding: 1, resource: r.object_buf.as_entire_binding() }] });
        let obj_module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("look_shadow_draw.wgsl (far)"), source: wgpu::ShaderSource::Wgsl(OBJECT_WGSL.into()) });
        let obj_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("far-shadow-depth-layout"), bind_group_layouts: &[Some(&obj_bgl)], immediate_size: 0 });
        let pos_only = [wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 }];
        let depth_pipeline = |label: &str, layout: &wgpu::PipelineLayout, module: &wgpu::ShaderModule, entry: &str, stride: u64, attrs: &[wgpu::VertexAttribute]| {
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState { module, entry_point: Some(entry), compilation_options: Default::default(), buffers: &[Some(wgpu::VertexBufferLayout { array_stride: stride, step_mode: wgpu::VertexStepMode::Vertex, attributes: attrs })] },
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
                depth_stencil: Some(depth_state.clone()),
                multisample: Default::default(),
                fragment: None,
                multiview_mask: None,
                cache: None,
            })
        };
        let depth_objects = depth_pipeline("far-shadow-depth", &obj_layout, &obj_module, "shadow_vs", std::mem::size_of::<crate::scene::Vertex>() as u64, &pos_only);
        let cell_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("far-shadow-cells-bgl"), entries: &[e(0, vs, ubuf_dyn), e(1, vs, sbuf(true)), e(2, vs, sbuf(true)), e(3, vs, sbuf(true))] });
        let cell_module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("hlod_far_shadow.wgsl"), source: wgpu::ShaderSource::Wgsl(CELL_WGSL.into()) });
        let cell_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("far-shadow-cells-layout"), bind_group_layouts: &[Some(&cell_bgl)], immediate_size: 0 });
        let cell_attrs = [
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Uint32, offset: std::mem::offset_of!(crate::hlod_bake::HlodVertex, group) as u64, shader_location: 1 },
        ];
        let depth_cells = depth_pipeline("far-shadow-cells", &cell_layout, &cell_module, "far_cell_vs", std::mem::size_of::<crate::hlod_bake::HlodVertex>() as u64, &cell_attrs);
        let bytes = size as u64 * size as u64 * 2 * 2 * rings as u64 + cull_buf.size() + view_buf.size() + draws.size() + cell_draws.size() + counts.size() + dummy.size();
        let res = Res {
            rings,
            _map: map,
            layers,
            cull_buf,
            view_buf,
            draws,
            cell_draws,
            counts,
            lists,
            cull_bgl,
            cull_objects,
            cull_cells,
            depth_objects,
            depth_bg,
            cell_bgl,
            depth_cells,
            dummy,
            binds: Mutex::new(None),
            object_count: r.object_count,
            compact: r.compact,
            bytes,
        };
        if self.res.set(res).is_err() {
            return 0;
        }
        r.look.set_far_shadow_map(Some(&array));
        bytes
    }

    /// The fit ring `k` would take now.
    /// The fit ring `k` would take now, its radius by the eye's height (eo-look `far_ring_radius`, from the map's own).
    fn fit_now(&self, k: usize, cam: &Camera, to_sun: [f64; 3], current: Option<f64>) -> FarRing {
        let eye = [cam.eye.x as f64, cam.eye.y as f64, cam.eye.z as f64];
        let radius = fit::far_ring_radius(k, eye[1] - self.scene_y.0 as f64, cam.far as f64, current);
        fit::fit_far_ring(eye, radius, to_sun, fit::FAR_MAP_SIZE, (self.scene_y.0 as f64, self.scene_y.1 as f64))
    }

    /// The CPU side, before the look's uniform takes its rows: which ring is
    /// stale, which tile the coming encode draws, the culls' boxes, the rows.
    pub(crate) fn update(&self, r: &crate::Renderer, queue: &wgpu::Queue, cam: &Camera, copy_step: f32) {
        let Some(res) = self.res.get() else { return };
        let level = r.look.options.lighting.far_shadows as usize;
        let mut guard = self.state.lock().unwrap();
        let st = &mut *guard;
        st.plan = None;
        st.culls = 0;
        let rings = level.min(res.rings);
        if rings == 0 || !r.look.shadows_on() {
            return;
        }
        if let Some(l) = &res.lists {
            if l.take_overflow() {
                st.overflow = true; // a tile drew with a bound short of its count: draw every ring again
            }
        }
        let to_sun = r.look.frame().key_dir;
        let mut culls = [[[0f32; 4]; 6]; 2];
        let frozen = st.frozen;
        for k in 0..rings {
            let current = st.rings[k].back.map(|b| b.0.radius).or(st.rings[k].front.map(|f| f.0.radius));
            let now = self.fit_now(k, cam, to_sun, current);
            let radius = now.radius;
            let overflow = st.overflow;
            let stats = &mut st.stats;
            let ring = &mut st.rings[k];
            if !frozen {
                // a redraw under way whose fit is far out starts again from this frame's.
                if let Some((b, _, layer)) = ring.back {
                    if b.moved_m(now.eye) > RESTART_SHARE * radius || b.sun_turn_deg(to_sun) > RESTART_TURN_DEG || b.radius != radius {
                        ring.back = Some((now, 0, layer));
                        stats.redraws_started += 1;
                    }
                }
                if ring.back.is_none() {
                    let why = match ring.front {
                        None => Some(0),
                        Some(_) if overflow => Some(3),
                        Some((f, _)) if f.sun_turn_deg(to_sun) > SUN_TURN_DEG => Some(1),
                        Some((f, _)) if f.moved_m(now.eye) > RECENTRE_SHARE * radius || f.radius != radius => Some(2),
                        _ => None,
                    };
                    if let Some(why) = why {
                        let layer = match ring.front {
                            Some((_, l)) => l ^ 1,
                            None => 2 * k as u32,
                        };
                        ring.back = Some((now, 0, layer));
                        stats.redraws_started += 1;
                        match why {
                            0 => stats.for_first += 1,
                            1 => stats.for_sun += 1,
                            2 => stats.for_camera += 1,
                            _ => stats.for_overflow += 1,
                        }
                    }
                }
            }
            culls[k] = now.planes;
        }
        st.overflow = false;
        // at most one tile a frame, ring 1 first.
        if !frozen {
            if let Some(k) = (0..rings).find(|&k| st.rings[k].back.is_some()) {
                let (f, tile, layer) = st.rings[k].back.unwrap();
                let (rect, planes) = f.tile(tile, TILES);
                culls[k] = planes;
                st.plan = Some(Plan { ring: k, tile, layer, rect });
                let v = View { view_proj: f.view_proj, copy_step: [copy_step, 0.0, 0.0, 0.0] };
                queue.write_buffer(&res.view_buf, k as u64 * SLOT, bytemuck::bytes_of(&v));
            }
        }
        st.culls = rings;
        let cell_count = if r.hlod.get().is_some() { crate::hlod::MAX_CELLS } else { 0 };
        for (k, planes) in culls.iter().enumerate().take(rings) {
            let p = CullParams {
                planes: *planes,
                cam: [cam.eye.x, cam.eye.y, cam.eye.z, crate::TERRAIN_LOD_DISTANCE_M],
                object_count: res.object_count,
                compact: res.compact as u32,
                cell_count,
                ring: k as u32,
            };
            queue.write_buffer(&res.cull_buf, k as u64 * SLOT, bytemuck::bytes_of(&p));
        }
        // the rows the frame samples: the fronts.
        let mut rows = [[0f32; 4]; LX_FAR_SHADOW_PARAMS - LX_FAR_SHADOW_VP + 1];
        let mut drawn = 0;
        for k in 0..rings {
            if let Some((f, layer)) = st.rings[k].front {
                for (i, row) in f.rows.iter().enumerate() {
                    rows[4 * k + i] = row.map(|v| v as f32);
                }
                // w: where the ring has faded out, metres on the ground from the eye (the eye stays within RECENTRE_SHARE of its centre).
                rows[4 * k + 3] = [layer as f32, f.normal_bias as f32, TAP_TEXELS / f.map_size as f32, ((1.0 - RECENTRE_SHARE) * f.radius) as f32];
                drawn += 1;
            }
        }
        let n = rows.len();
        rows[n - 1] = [if drawn > 0 { rings as f32 } else { 0.0 }, EDGE_FADE_UV, FADE_START_SHARE, 0.0];
        drop(guard);
        r.look.set_lighting_params(LX_FAR_SHADOW_VP, &rows);
    }

    /// The bind groups for the far layer as it is now.
    fn binds(&self, res: &Res, r: &crate::Renderer) -> (wgpu::BindGroup, Option<wgpu::BindGroup>) {
        let hlod = r.hlod.get();
        let mut b = res.binds.lock().unwrap();
        if b.as_ref().is_none_or(|(h, _, _)| *h != hlod.is_some()) {
            let d = &self.device;
            let cells = hlod.map(|h| &h.cell_buf).unwrap_or(&res.dummy);
            let ent = |binding, resource| wgpu::BindGroupEntry { binding, resource };
            let cull = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("far-shadow-cull-bg"),
                layout: &res.cull_bgl,
                entries: &[
                    ent(0, wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &res.cull_buf, offset: 0, size: Some(std::num::NonZeroU64::new(std::mem::size_of::<CullParams>() as u64).unwrap()) })),
                    ent(1, r.object_buf.as_entire_binding()),
                    ent(2, res.draws.as_entire_binding()),
                    ent(3, res.counts.as_entire_binding()),
                    ent(4, r.hlod_map_buf.as_entire_binding()),
                    ent(5, r.hlod_flags_buf.as_entire_binding()),
                    ent(6, cells.as_entire_binding()),
                    ent(7, res.cell_draws.as_entire_binding()),
                ],
            });
            let cell_depth = hlod.map(|h| {
                d.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("far-shadow-cells-bg"),
                    layout: &res.cell_bgl,
                    entries: &[
                        ent(0, wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &res.view_buf, offset: 0, size: Some(std::num::NonZeroU64::new(std::mem::size_of::<View>() as u64).unwrap()) })),
                        ent(1, h.cell_buf.as_entire_binding()),
                        ent(2, r.hlod_map_buf.as_entire_binding()),
                        ent(3, r.hlod_flags_buf.as_entire_binding()),
                    ],
                })
            });
            crate::hitch::mark(crate::hitch::Mark::BindGroupCreated, 1 + cell_depth.is_some() as u64);
            *b = Some((hlod.is_some(), cull, cell_depth));
        }
        let (_, c, cd) = b.as_ref().unwrap();
        (c.clone(), cd.clone())
    }

    /// The culls of every ring in use (also the count emulation's recovery cull).
    fn encode_culls(&self, res: &Res, r: &crate::Renderer, enc: &mut wgpu::CommandEncoder, rings: usize, tw: Option<wgpu::ComputePassTimestampWrites<'_>>) {
        enc.clear_buffer(&res.counts, 0, None);
        let (cull_bg, _) = self.binds(res, r);
        let cells = r.hlod.get().is_some();
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("far-shadow-cull"), timestamp_writes: tw });
        for k in 0..rings {
            pass.set_bind_group(0, &cull_bg, &[(k as u64 * SLOT) as u32]);
            pass.set_pipeline(&res.cull_objects);
            pass.dispatch_workgroups(res.object_count.div_ceil(64).max(1), 1, 1);
            if cells {
                pass.set_pipeline(&res.cull_cells);
                pass.dispatch_workgroups(crate::hlod::MAX_CELLS.div_ceil(64), 1, 1);
            }
        }
    }

    /// The count emulation's recovery: this frame's culls, untimed.
    pub(crate) fn encode_recovery_cull(&self, r: &crate::Renderer, enc: &mut wgpu::CommandEncoder) {
        let Some(res) = self.res.get() else { return };
        let rings = self.state.lock().unwrap().culls;
        if rings > 0 {
            self.encode_culls(res, r, enc, rings, None);
        }
    }

    /// The far maps' passes for this frame: after `LookGpu::encode_shadows`
    /// (the cascades) and before anything that samples group 1: the culls,
    /// then the tile `update` planned (if any). Timed in `lighting::Q_FAR_SHADOW`.
    pub(crate) fn encode(&self, enc: &mut wgpu::CommandEncoder, r: &crate::Renderer, ts: Ts<'_>) {
        if r.look.options.lighting.far_shadows == 0 {
            lighting::empty_span(enc, ts, lighting::Q_FAR_SHADOW, "far shadow (off)");
            return;
        }
        let Some(res) = self.res.get() else {
            lighting::empty_span(enc, ts, lighting::Q_FAR_SHADOW, "far shadow (not attached)");
            return;
        };
        let (rings, plan) = {
            let st = self.state.lock().unwrap();
            (st.culls, st.plan)
        };
        if rings == 0 {
            lighting::empty_span(enc, ts, lighting::Q_FAR_SHADOW, "far shadow (idle)");
            return;
        }
        let begin = ts.map(|(q, b)| (q, b + lighting::Q_FAR_SHADOW));
        let ctw = begin.map(|(q, b)| wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(b), end_of_pass_write_index: plan.is_none().then_some(b + 1) });
        self.encode_culls(res, r, enc, rings, ctw);
        let Some(p) = plan else { return };
        let rtw = begin.map(|(q, b)| wgpu::RenderPassTimestampWrites { query_set: q, beginning_of_pass_write_index: None, end_of_pass_write_index: Some(b + 1) });
        let (_, cell_bg) = self.binds(res, r);
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("far-shadow-depth"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &res.layers[p.layer as usize],
                    // the first tile clears the whole back layer (nothing samples it until it is whole).
                    depth_ops: Some(wgpu::Operations { load: if p.tile == 0 { wgpu::LoadOp::Clear(1.0) } else { wgpu::LoadOp::Load }, store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: rtw,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_scissor_rect(p.rect[0], p.rect[1], p.rect[2], p.rect[3]);
            let k = p.ring as u64;
            let n = res.object_count.max(1);
            pass.set_pipeline(&res.depth_objects);
            pass.set_bind_group(0, &res.depth_bg, &[(k * SLOT) as u32]);
            pass.set_vertex_buffer(0, r.vertex_buf.slice(..));
            pass.set_index_buffer(r.index_buf.slice(..), wgpu::IndexFormat::Uint32);
            let offset = k * n as u64 * crate::DRAW_ARGS_BYTES;
            if res.compact {
                crate::counted::draw_indexed(&mut pass, res.lists.as_ref(), 2 * p.ring, &res.draws, offset, &res.counts, k * 16, n);
            } else {
                pass.multi_draw_indexed_indirect(&res.draws, offset, n);
            }
            if let (Some(h), Some(bg)) = (r.hlod.get(), cell_bg.as_ref()) {
                pass.set_pipeline(&res.depth_cells);
                pass.set_bind_group(0, bg, &[(k * SLOT) as u32]);
                pass.set_vertex_buffer(0, h.vertex_buf.slice(..));
                pass.set_index_buffer(h.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                let cells = crate::hlod::MAX_CELLS;
                crate::counted::draw_indexed(&mut pass, res.lists.as_ref(), 2 * p.ring + 1, &res.cell_draws, k * cells as u64 * crate::DRAW_ARGS_BYTES, &res.counts, k * 16 + 8, cells);
            }
        }
        let mut st = self.state.lock().unwrap();
        st.stats.tiles_drawn += 1;
        st.plan = None;
        let ring = &mut st.rings[p.ring];
        if let Some((f, tile, layer)) = ring.back {
            if tile + 1 >= SLICES {
                ring.front = Some((f, layer));
                ring.back = None;
                st.stats.maps_completed += 1;
            } else {
                ring.back = Some((f, tile + 1, layer));
            }
        }
    }
}
