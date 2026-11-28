//! the far terrain around the city to the horizon (eo-farshore's
//! cooked heightfield), drawn inside the city pass after the opaque city and
//! before the sky. One instanced draw a frame: a CDLOD quadtree chosen on the
//! CPU (frustum, distance, and the city's own ground hiding the inside of its
//! grid within the far plane), each chosen node one instance of a 32 x 32 quad
//! patch whose vertices are morphed onto the next level's lattice before the
//! level changes (no cracks, no pops). WebGPU core only: two sampled textures
//! (R16Float heights, RGBA8 classes, eight mips each), a uniform, a vertex
//! and an instance buffer; no storage, no indirect draws, three bind groups.
//! See farshore.wgsl for the depth, the haze and the shading.

use crate::camera::{Camera, frustum_planes};
use bytemuck::{Pod, Zeroable};
use glam::{Vec3, Vec4};
use std::sync::Mutex;

pub mod outer; // the outer ring (stage L5, the cinematic look)

/// Quads a side of one node's patch (farshore.wgsl divides by the same 32).
pub const PATCH: u32 = 32;
/// Mip levels of both textures; the quadtree's levels are 0..=TOP (TOP + 1 is the last morph target).
pub const MIPS: u32 = 8;
pub const TOP: u32 = MIPS - 2;
/// Instances a frame at most.
pub const MAX_NODES: usize = 4096;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FarUniform {
    view_proj: [[f32; 4]; 4],
    cam: [f32; 4],
    frame: [f32; 4],
    tex: [f32; 4],
    lod: [f32; 4],
    grid: [f32; 4],
    reach: [f32; 4],
    misc: [f32; 4],
    /// the sea's grid this frame: centre x, z, half size, drawn (1/0).
    sea: [f32; 4],
    /// the cinematic look (stage L5; all 0 otherwise): x water everywhere outside the
    /// drawn sea (1/0), y the skirt to the horizon (1/0), z the outer ring under this
    /// layer's rim (1/0), w the skirt's length (game m).
    ring: [f32; 4],
    /// the outer ring's texture: origin x, z (surveyed m), 1 / (texel x width), 1 / (texel x height).
    otex: [f32; 4],
    /// this layer's extent on the plan (game m): min x, min z, max x, max z.
    ext: [f32; 4],
    /// x the inner cook's rim taper (surveyed m), y its sea floor (game y), z log2(outer texel / this texel),
    /// w the sea sheet inside the city grid (1/0: the sea pass stood down for distance).
    rim: [f32; 4],
    /// x: 1 when the grass reads the look uniform's per-frame season (`tone2.w`,
    /// eo-look `season::current`, the season on the date in force), 0 when it
    /// keeps `misc.x` (`--no-photo-season`, or `EO_FARSHORE_DRY` given); y, z, w unused.
    season: [f32; 4],
}

/// The skirt's reach past the outermost layer's edge (game m, stage L5). 400 km:
/// far enough that the volume's aerial perspective has fully hazed it (past its
/// end the sky's pixels under the horizon are hazed as sea at that distance,
/// sky_resolve.wgsl, the same colour), near enough that its end is no edge: a
/// 3,000 km skirt left a one-pixel dark line where it met the sky.
pub const SKIRT_M: f32 = 4.0e5;

/// Stage L5's world edge under the cinematic look (the sea's fog test from the
/// fog in use, the far layer's water everywhere the sea is not, the sea's reach
/// south, the outer ring, the skirt): on unless `EO_FARSHORE_EDGE=0`, which
/// keeps the look's frame without it (its A/B on one binary). Read once.
pub fn world_edge_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("EO_FARSHORE_EDGE").map_or(true, |v| v.trim() != "0"))
}

/// Who draws the water at a plan point (stage L5's rule, farshore.wgsl `fs_vs`):
/// the sea pass inside its reach and its grid square when it is drawn this
/// frame; the far layer everywhere else. `reach` and `sea` as `FarUniform`'s.
pub fn sea_draws_at(p: [f32; 2], reach: [f32; 4], sea: [f32; 4]) -> bool {
    let in_reach = p[0] >= reach[0] && p[1] >= reach[1] && p[0] <= reach[2] && p[1] <= reach[3];
    sea[3] > 0.5 && in_reach && (p[0] - sea[0]).abs() <= sea[2] && (p[1] - sea[1]).abs() <= sea[2]
}

/// Switches: `--no-farshore` / `EO_FARSHORE=0` (eo-app); `EO_FARSHORE_RANGE`
/// (level-0 range, game m), `EO_FARSHORE_TAIL` (haze tail x the fog's far
/// end), `EO_FARSHORE_SHADOW=0`, `EO_FARSHORE_DEBUG=1` (the class guesses),
/// `EO_FARSHORE_DRY` (0 green .. 1 golden; default from the sun model's date,
/// each frame: the look uniform's season, `--no-photo-season` reads it at load).
#[derive(Clone, Copy, Debug)]
pub struct FarOptions {
    pub range0: f32,
    pub morph_start: f32,
    pub tail: f32,
    pub self_shadow: bool,
    pub debug: bool,
    pub dryness: f32,
    /// The grass follows the date in force each frame (the look uniform's
    /// season) instead of `dryness` read at load: on unless `--no-photo-season`
    /// or `EO_FARSHORE_DRY` pins it.
    pub season_per_frame: bool,
}

fn env_f32(name: &str) -> Option<f32> {
    std::env::var(name).ok().and_then(|v| v.trim().parse::<f32>().ok()).filter(|v| v.is_finite())
}

/// How golden the annual grass is on a day of the year (`eo_look::season::dryness`,
/// the one season curve).
pub fn dryness_for_day(doy: f64) -> f32 {
    eo_look::season::dryness(doy)
}

/// The season's dryness both land layers read (`EO_FARSHORE_DRY`, else
/// `eo_look::season::current`, the season on the sun model's date): the far
/// layer's grass and the city terrain's far land (eo-app ground_tone), so they
/// meet at the city grid's edge in one season.
pub fn season_dryness() -> f32 {
    env_f32("EO_FARSHORE_DRY").map(|v| v.clamp(0.0, 1.0)).unwrap_or_else(eo_look::season::current)
}

/// The far layer's land albedos (farshore.wgsl, linear), shared with the city
/// terrain's far land (eo-app ground_tone `far_land_season_tone`): the annual
/// grass green and golden, coastal scrub, woodland (open and north face),
/// marsh, bare rock and the built flats.
pub const FAR_GRASS_GREEN: [f32; 3] = [0.065, 0.100, 0.034];
pub const FAR_GRASS_GOLDEN: [f32; 3] = [0.215, 0.165, 0.092];
pub const FAR_SCRUB: [f32; 3] = [0.075, 0.074, 0.046];
pub const FAR_TREE: [f32; 3] = [0.036, 0.046, 0.026];
pub const FAR_MARSH: [f32; 3] = [0.065, 0.075, 0.048];
pub const FAR_ROCK: [f32; 3] = [0.17, 0.155, 0.13];
pub const FAR_TOWN: [f32; 3] = [0.092, 0.094, 0.088];

impl Default for FarOptions {
    fn default() -> Self {
        Self {
            range0: env_f32("EO_FARSHORE_RANGE").unwrap_or(7000.0).clamp(1500.0, 40000.0),
            morph_start: 0.7,
            tail: env_f32("EO_FARSHORE_TAIL").unwrap_or(6.0).clamp(0.05, 50.0),
            self_shadow: std::env::var("EO_FARSHORE_SHADOW").map_or(true, |v| v.trim() != "0"),
            debug: std::env::var("EO_FARSHORE_DEBUG").is_ok_and(|v| v.trim() == "1"),
            dryness: season_dryness(),
            season_per_frame: eo_look::season::features().photo_season && env_f32("EO_FARSHORE_DRY").is_none(),
        }
    }
}

/// What `attach_farshore` takes.
pub struct FarInit<'a> {
    pub terrain: &'a eo_farshore::FarTerrain,
    /// The city's elevation grid on the ground plan (game m): min x, min z, max x, max z.
    pub grid: [f32; 4],
    /// The sea's reach (water_common.wgsl `wg_reach`: the grid + 60 km west, north and east), game m.
    pub reach: [f32; 4],
    pub plan_scale: f64,
    pub height_scale: f64,
    /// The outer ring (`outer::load_or_bake`, the cinematic look only), None without it.
    pub outer: Option<&'a eo_farshore::FarTerrain>,
}

/// The outer ring's own textures, uniform, nodes and quadtree (drawn with the
/// far layer's pipeline and patch, after it, under the cinematic look only).
struct OuterLayer {
    bg: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    inst: wgpu::Buffer,
    count: Mutex<u32>,
    tree: FarTree,
    texel_game: f64,
    tex: [f32; 4],
}

/// A cooked heightfield's three textures (heights R16Float, classes RGBA8,
/// shape RG8, `MIPS` box-filtered mips each) and the bytes they take.
fn upload_terrain(device: &wgpu::Device, queue: &wgpu::Queue, t: &eo_farshore::FarTerrain) -> (wgpu::TextureView, wgpu::TextureView, wgpu::TextureView, u64) {
    let hd = &t.header;
    let (w, h) = (hd.width, hd.height);
    // Height mips (box average), uploaded as R16Float.
    let mut levels: Vec<(usize, usize, Vec<f32>)> = vec![(w, h, t.heights.clone())];
    for _ in 1..MIPS {
        let (pw, ph, prev) = levels.last().unwrap();
        let (nw, nh) = ((pw / 2).max(1), (ph / 2).max(1));
        let mut next = vec![0f32; nw * nh];
        for j in 0..nh {
            for i in 0..nw {
                let at = |a: usize, b: usize| prev[(b.min(ph - 1)) * pw + a.min(pw - 1)];
                next[j * nw + i] = 0.25 * (at(2 * i, 2 * j) + at(2 * i + 1, 2 * j) + at(2 * i, 2 * j + 1) + at(2 * i + 1, 2 * j + 1));
            }
        }
        levels.push((nw, nh, next));
    }
    let height_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("farshore-heights"),
        size: wgpu::Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
        mip_level_count: MIPS,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mut bytes = 0u64;
    for (m, (lw, lh, data)) in levels.iter().enumerate() {
        let px: Vec<u16> = data.iter().map(|&v| half(v)).collect();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &height_tex, mip_level: m as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytemuck::cast_slice(&px),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(*lw as u32 * 2), rows_per_image: Some(*lh as u32) },
            wgpu::Extent3d { width: *lw as u32, height: *lh as u32, depth_or_array_layers: 1 },
        );
        bytes += (lw * lh * 2) as u64;
    }
    // Class mips (box average), RGBA8.
    let (cw, ch) = (t.class_width, t.class_height);
    let class_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("farshore-classes"),
        size: wgpu::Extent3d { width: cw as u32, height: ch as u32, depth_or_array_layers: 1 },
        mip_level_count: MIPS,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mut cl: (usize, usize, Vec<[u8; 4]>) = (cw, ch, t.classes.clone());
    for m in 0..MIPS {
        let (lw, lh, data) = &cl;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &class_tex, mip_level: m, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytemuck::cast_slice(data),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(*lw as u32 * 4), rows_per_image: Some(*lh as u32) },
            wgpu::Extent3d { width: *lw as u32, height: *lh as u32, depth_or_array_layers: 1 },
        );
        bytes += (lw * lh * 4) as u64;
        let (nw, nh) = ((lw / 2).max(1), (lh / 2).max(1));
        let mut next = vec![[0u8; 4]; nw * nh];
        for j in 0..nh {
            for i in 0..nw {
                let at = |a: usize, b: usize| data[(b.min(lh - 1)) * lw + a.min(lw - 1)];
                for k in 0..4 {
                    let s = at(2 * i, 2 * j)[k] as u32 + at(2 * i + 1, 2 * j)[k] as u32 + at(2 * i, 2 * j + 1)[k] as u32 + at(2 * i + 1, 2 * j + 1)[k] as u32;
                    next[j * nw + i][k] = ((s + 2) / 4) as u8;
                }
            }
        }
        cl = (nw, nh, next);
    }
    // the shape plane (sky visibility, valley), RG8 with box mips.
    let shape_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("farshore-shape"),
        size: wgpu::Extent3d { width: cw as u32, height: ch as u32, depth_or_array_layers: 1 },
        mip_level_count: MIPS,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rg8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mut sl: (usize, usize, Vec<[u8; 2]>) = (cw, ch, t.shape.clone());
    for m in 0..MIPS {
        let (lw, lh, data) = &sl;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &shape_tex, mip_level: m, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytemuck::cast_slice(data),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(*lw as u32 * 2), rows_per_image: Some(*lh as u32) },
            wgpu::Extent3d { width: *lw as u32, height: *lh as u32, depth_or_array_layers: 1 },
        );
        bytes += (lw * lh * 2) as u64;
        let (nw, nh) = ((lw / 2).max(1), (lh / 2).max(1));
        let mut next = vec![[0u8; 2]; nw * nh];
        for j in 0..nh {
            for i in 0..nw {
                let at = |a: usize, b: usize| data[(b.min(lh - 1)) * lw + a.min(lw - 1)];
                for k in 0..2 {
                    let s = at(2 * i, 2 * j)[k] as u32 + at(2 * i + 1, 2 * j)[k] as u32 + at(2 * i, 2 * j + 1)[k] as u32 + at(2 * i + 1, 2 * j + 1)[k] as u32;
                    next[j * nw + i][k] = ((s + 2) / 4) as u8;
                }
            }
        }
        sl = (nw, nh, next);
    }
    (height_tex.create_view(&Default::default()), class_tex.create_view(&Default::default()), shape_tex.create_view(&Default::default()), bytes)
}

/// The far layer's shader as composed for its pipeline.
fn shader_source() -> String {
    crate::look::sf_select(&shader_parts()) // the natural-detail blocks (`// sf:`, `--hill-cover`) compiled in now
}

fn shader_parts() -> String {
    let survey = crate::scene::terrain_detail::marin_ground::overview();
    let (x,z,w,h) = survey.map_or((0.,0.,1.,1.), |im| (im.origin[0],im.origin[1],im.extent[0],im.extent[1]));
    format!(
        "const FS_MARIN: bool = {};\nconst FS_MARIN_IMAGE: vec4<f32> = vec4<f32>({x:.3}, {z:.3}, {w:.3}, {h:.3});\nconst FS_DEPTH_K: f32 = {:e};\n{}\n{}\n{}\n{}",
        survey.is_some(),
        eo_farshore::DEPTH_K,
        crate::look::look_common_source(),
        include_str!("sky_light_core.wgsl"),
        crate::look::LOOK_EXT_WGSL, // group 1 bindings 12-18
        include_str!("farshore.wgsl")
    )
}

/// Whether the far layer draws the water at a plan point (a mirror of
/// farshore.wgsl `fs_vs`'s `own_water`; the sea's reach and grid as
/// `FarUniform`'s, `grid` the city grid). By default it draws south of the grid
/// within the reach's width and inside the reach where the sea's grid is not;
/// past that nothing (the void the cinematic look closes). Under the cinematic
/// look (stage L5) everywhere `sea_draws_at` is false.
pub fn far_draws_water_at(p: [f32; 2], reach: [f32; 4], grid: [f32; 4], sea: [f32; 4], cinematic: bool) -> bool {
    let in_sea = sea_draws_at(p, reach, sea);
    if cinematic {
        return !in_sea;
    }
    let in_reach = p[0] >= reach[0] && p[1] >= reach[1] && p[0] <= reach[2] && p[1] <= reach[3];
    !in_sea && ((p[0] >= reach[0] && p[0] <= reach[2] && p[1] < grid[3] + 60000.0) || in_reach)
}

/// A heightfield's texture mapping (`FarUniform::tex`).
fn tex_of(t: &eo_farshore::FarTerrain) -> [f32; 4] {
    let hd = &t.header;
    [hd.origin_x as f32, hd.origin_z as f32, (1.0 / (hd.texel_m * hd.width as f64)) as f32, (1.0 / (hd.texel_m * hd.height as f64)) as f32]
}

pub struct FarShoreGpu {
    pipeline: wgpu::RenderPipeline,
    bg: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    patch_vb: wgpu::Buffer,
    patch_ib: wgpu::Buffer,
    index_count: u32,
    inst: wgpu::Buffer,
    count: Mutex<u32>,
    /// The quadtree on the CPU.
    pub tree: FarTree,
    texel_game: f64,
    grid: [f32; 4],
    reach: [f32; 4],
    plan_scale: f64,
    height_scale: f64,
    tex: [f32; 4],
    pub options: FarOptions,
    pub bytes: u64,
    /// Nodes drawn last frame (for the bench's line).
    pub last_nodes: std::sync::atomic::AtomicU32,
    /// The outer ring (stage L5), drawn only under the cinematic look.
    outer: Option<OuterLayer>,
}

/// f32 -> IEEE half (round to nearest), for the R16Float heights.
fn half(v: f32) -> u16 {
    eo_farshore::half::f16::from_f32(v).to_bits()
}

impl FarShoreGpu {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, look: &crate::look::LookGpu, init: &FarInit) -> Self {
        let t = init.terrain;
        let hd = &t.header;
        let (hv, cv, sv, mut bytes) = upload_terrain(device, queue, t);
        // the outer ring's textures (stage L5), when it was cooked
        let outer_views = init.outer.map(|o| upload_terrain(device, queue, o));
        let overview = crate::scene::terrain_detail::marin_ground::overview().map(|im| {
            use wgpu::util::DeviceExt;
            bytes += im.rgba.len() as u64;
            device.create_texture_with_data(queue, &wgpu::TextureDescriptor {
                label: Some("Marin distant albedo"), size: wgpu::Extent3d { width: im.width as u32, height: im.height as u32, depth_or_array_layers: 1 },
                mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[],
            }, wgpu::util::TextureDataOrder::LayerMajor, &im.rgba).create_view(&Default::default())
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("farshore-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("farshore-uniform"),
            size: std::mem::size_of::<FarUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let tex_entry = |b: u32| wgpu::BindGroupLayoutEntry {
            binding: b,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("farshore-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                tex_entry(1),
                tex_entry(2),
                tex_entry(4), // the shape plane
                tex_entry(6), // opt-in Marin aerial albedo
                tex_entry(5), // the outer ring's heights under the rim (stage L5; this layer's own heights when there is no ring)
                wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let make_bg = |label: &str, u: &wgpu::Buffer, h: &wgpu::TextureView, c: &wgpu::TextureView, sh: &wgpu::TextureView, o: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: u.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(h) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(c) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&sampler) },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(sh) },
                    wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(o) },
                    wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(overview.as_ref().unwrap_or(c)) },
                ],
            })
        };
        let bg = make_bg("farshore-bg", &uniform, &hv, &cv, &sv, outer_views.as_ref().map_or(&hv, |o| &o.0));
        let src = shader_source();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("farshore.wgsl"), source: wgpu::ShaderSource::Wgsl(src.into()) });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("farshore-layout"), bind_group_layouts: &[Some(&bgl), Some(look.main_bind_group_layout())], immediate_size: 0 });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("farshore"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("fs_vs"),
                compilation_options: Default::default(),
                buffers: &[
                    Some(wgpu::VertexBufferLayout { array_stride: 4, step_mode: wgpu::VertexStepMode::Vertex, attributes: &[wgpu::VertexAttribute { format: wgpu::VertexFormat::Uint16x2, offset: 0, shader_location: 0 }] }),
                    Some(wgpu::VertexBufferLayout { array_stride: 16, step_mode: wgpu::VertexStepMode::Instance, attributes: &[wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 0, shader_location: 1 }] }),
                ],
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: crate::DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState { count: 1, ..Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: crate::look::HDR_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        // The patch: (PATCH + 1)^2 lattice points, two triangles a quad, wound
        // counter-clockwise seen from above (+y) in x east, z south.
        let side = PATCH + 1;
        let mut verts: Vec<[u16; 2]> = Vec::with_capacity((side * side) as usize);
        for j in 0..side {
            for i in 0..side {
                verts.push([i as u16, j as u16]);
            }
        }
        let mut idx: Vec<u16> = Vec::with_capacity((PATCH * PATCH * 6) as usize);
        for j in 0..PATCH {
            for i in 0..PATCH {
                let a = (j * side + i) as u16;
                let b = a + 1;
                let c = a + side as u16;
                let d = c + 1;
                // seen from +y with +z toward the viewer's bottom: (a, c, b), (b, c, d) are counter-clockwise
                idx.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
        use wgpu::util::DeviceExt;
        let patch_vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("farshore-patch"), contents: bytemuck::cast_slice(&verts), usage: wgpu::BufferUsages::VERTEX });
        let patch_ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("farshore-patch-idx"), contents: bytemuck::cast_slice(&idx), usage: wgpu::BufferUsages::INDEX });
        let inst = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("farshore-nodes"),
            size: (MAX_NODES * 16) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        bytes += (verts.len() * 4 + idx.len() * 2 + MAX_NODES * 16) as u64 + std::mem::size_of::<FarUniform>() as u64;
        let s = init.plan_scale;
        let hs = init.height_scale;
        let options = FarOptions::default();
        let tree = FarTree::new(t, init.grid, s, hs, options.range0 as f64);
        let texel_game = hd.texel_m * s;
        let tex = tex_of(t);
        let outer = init.outer.zip(outer_views).map(|(o, (ohv, ocv, osv, ob))| {
            bytes += ob;
            let ou = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("farshore-outer-uniform"),
                size: std::mem::size_of::<FarUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let oi = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("farshore-outer-nodes"),
                size: (MAX_NODES * 16) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            bytes += (MAX_NODES * 16) as u64 + std::mem::size_of::<FarUniform>() as u64;
            // its level 0 is the inner cook's level log2(texel ratio): the same ranges for the same node size
            let ratio = o.header.texel_m / hd.texel_m;
            let mut otree = FarTree::new(o, init.grid, s, hs, options.range0 as f64 * ratio);
            otree.hole = Some(tree.extent());
            OuterLayer { bg: make_bg("farshore-outer-bg", &ou, &ohv, &ocv, &osv, &ohv), uniform: ou, inst: oi, count: Mutex::new(0), tree: otree, texel_game: o.header.texel_m * s, tex: tex_of(o) }
        });
        Self {
            pipeline,
            bg,
            uniform,
            patch_vb,
            patch_ib,
            index_count: idx.len() as u32,
            inst,
            count: Mutex::new(0),
            tree,
            texel_game,
            grid: init.grid,
            reach: init.reach,
            plan_scale: s,
            height_scale: hs,
            tex,
            options,
            bytes,
            last_nodes: std::sync::atomic::AtomicU32::new(0),
            outer,
        }
    }

    /// Whether the outer ring was attached (stage L5).
    pub fn has_outer(&self) -> bool {
        self.outer.is_some()
    }

    /// The camera (jittered, as the city is drawn) and the node list for this
    /// frame. `sea_on`: the sea pass draws this frame. `cinematic` (stage L5):
    /// the far layer draws the water everywhere the sea does not (the sea's
    /// reach widened south by `water::CINEMATIC_SOUTH_REACH_M`), the outer
    /// ring under its rim and past it, and the outermost layer's edge skirted
    /// to the horizon. `sheet` (the cinematic look, the sea pass stood down for
    /// distance): the water inside the city grid too, a sheet at sea level over
    /// the city's seabed. Off: exactly the frame before stage L5.
    pub fn update(&self, queue: &wgpu::Queue, cam: &Camera, sea_on: bool, cinematic: bool, sheet: bool) {
        let outer = self.outer.as_ref().filter(|_| cinematic);
        let skirt = if cinematic && outer.is_none() { SKIRT_M as f64 } else { 0.0 };
        let sheet = cinematic && sheet;
        let nodes = self.tree.select_full(cam, skirt, sheet);
        let n = nodes.len().min(MAX_NODES);
        if n > 0 {
            queue.write_buffer(&self.inst, 0, bytemuck::cast_slice(&nodes[..n]));
        }
        *self.count.lock().unwrap() = n as u32;
        let o = &self.options;
        let px = (cam.fov_y_deg.to_radians() / 1080.0).max(1e-5);
        let mut reach = self.reach;
        if cinematic {
            reach[3] += crate::water::CINEMATIC_SOUTH_REACH_M;
        }
        let ext = self.tree.extent().map(|v| v as f32);
        let u = FarUniform {
            view_proj: cam.view_proj().to_cols_array_2d(),
            cam: [cam.eye.x, cam.eye.y, cam.eye.z, cam.far],
            frame: [self.plan_scale as f32, self.height_scale as f32, eo_data::SEA_LEVEL_Y as f32, o.tail],
            tex: self.tex,
            lod: [self.texel_game as f32, TOP as f32, o.morph_start, o.range0],
            grid: self.grid,
            reach,
            misc: [o.dryness, px, o.self_shadow as u32 as f32, o.debug as u32 as f32],
            sea: {
                // the water's own grid rule (water.rs update), without its hysteresis
                let r = eo_water::grid::radius_for((cam.eye.y as f64 - eo_data::SEA_LEVEL_Y).max(0.0), cam.far as f64, 0.0);
                let c = eo_water::grid::offset_for(cam.eye.x as f64, cam.eye.z as f64, eo_water::grid::snap_for(r));
                [c[0] as f32, c[1] as f32, r as f32, sea_on as u32 as f32]
            },
            ring: if cinematic { [1.0, (skirt > 0.0) as u32 as f32, outer.is_some() as u32 as f32, SKIRT_M] } else { [0.0; 4] },
            otex: outer.map_or([0.0; 4], |l| l.tex),
            ext: if cinematic { ext } else { [0.0; 4] },
            rim: if cinematic { [eo_farshore::OUTER_TAPER_M as f32, eo_data::SEA_FLOOR_Y as f32, outer.map_or(0.0, |l| (l.texel_game / self.texel_game).log2() as f32), sheet as u32 as f32] } else { [0.0; 4] },
            season: [o.season_per_frame as u32 as f32, 0.0, 0.0, 0.0],
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
        let mut total = n;
        if let Some(l) = outer {
            let nodes = l.tree.select_with(cam, SKIRT_M as f64);
            let m = nodes.len().min(MAX_NODES);
            if m > 0 {
                queue.write_buffer(&l.inst, 0, bytemuck::cast_slice(&nodes[..m]));
            }
            *l.count.lock().unwrap() = m as u32;
            total += m;
            let ou = FarUniform {
                tex: l.tex,
                lod: [l.texel_game as f32, TOP as f32, o.morph_start, l.tree.range0 as f32],
                ring: [1.0, 1.0, 0.0, SKIRT_M],
                otex: [0.0; 4],
                ext: l.tree.extent().map(|v| v as f32),
                rim: [0.0; 4],
                ..u
            };
            queue.write_buffer(&l.uniform, 0, bytemuck::bytes_of(&ou));
        } else if let Some(l) = &self.outer {
            *l.count.lock().unwrap() = 0;
        }
        self.last_nodes.store(total as u32, std::sync::atomic::Ordering::Relaxed);
    }

    /// Inside the city pass (single sample, no debug ids): group 0 is this
    /// layer's; group 1 the look's (bound here again). The caller restores
    /// its own vertex and index buffers and group 0 afterwards.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, look: &crate::look::LookGpu) {
        let n = *self.count.lock().unwrap();
        let m = self.outer.as_ref().map_or(0, |l| *l.count.lock().unwrap());
        if n == 0 && m == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bg, &[]);
        look.bind_main(pass);
        pass.set_vertex_buffer(0, self.patch_vb.slice(..));
        pass.set_vertex_buffer(1, self.inst.slice(..));
        pass.set_index_buffer(self.patch_ib.slice(..), wgpu::IndexFormat::Uint16);
        if n > 0 {
            pass.draw_indexed(0..self.index_count, 0, 0..n);
        }
        if let Some(l) = self.outer.as_ref().filter(|_| m > 0) {
            pass.set_bind_group(0, &l.bg, &[]);
            pass.set_vertex_buffer(1, l.inst.slice(..));
            pass.draw_indexed(0..self.index_count, 0, 0..m);
        }
    }
}

/// The CDLOD quadtree on the CPU: per level the height bounds (game y on this
/// plan) of every node, level-0 nodes `PATCH` cooked texels a side. Pure (no
/// GPU), so its choice can be tested and timed alone.
pub struct FarTree {
    minmax: Vec<Vec<[f32; 2]>>,
    dims: Vec<(usize, usize)>,
    /// Game metres: the cooked origin and the level-0 node size.
    origin: [f64; 2],
    node0: f64,
    texel_game: f64,
    grid: [f32; 4],
    range0: f64,
    /// The cooked extent on the plan (game m): min x, min z, max x, max z.
    ext: [f64; 4],
    /// A rectangle (game m) left to another layer: nodes wholly inside it are
    /// skipped, nodes across its edge split (the outer ring round the inner cook).
    pub hole: Option<[f64; 4]>,
}

impl FarTree {
    pub fn new(t: &eo_farshore::FarTerrain, grid: [f32; 4], plan_scale: f64, height_scale: f64, range0: f64) -> Self {
        let hd = &t.header;
        let (w, h) = (hd.width, hd.height);
        let lift = |y: f32| eo_data::groundscale::lift_by(y as f64, height_scale) as f32;
        let n = PATCH as usize;
        let (nw0, nh0) = (w.div_ceil(n), h.div_ceil(n));
        let mut lv0 = vec![[f32::MAX, f32::MIN]; nw0 * nh0];
        for j in 0..h {
            let nj = j / n;
            for i in 0..w {
                let y = lift(t.heights[j * w + i]);
                let k = nj * nw0 + i / n;
                lv0[k][0] = lv0[k][0].min(y);
                lv0[k][1] = lv0[k][1].max(y);
            }
        }
        // A node's patch reaches one texel past its edge (the bilinear taps): widen by the neighbours.
        let mut wide = lv0.clone();
        for j in 0..nh0 {
            for i in 0..nw0 {
                for (di, dj) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                    let (a, b) = (i as i64 + di, j as i64 + dj);
                    if a >= 0 && b >= 0 && (a as usize) < nw0 && (b as usize) < nh0 {
                        let o = lv0[b as usize * nw0 + a as usize];
                        let k = j * nw0 + i;
                        wide[k][0] = wide[k][0].min(o[0]);
                        wide[k][1] = wide[k][1].max(o[1]);
                    }
                }
            }
        }
        let sea = eo_data::SEA_LEVEL_Y as f32;
        let mut minmax = vec![wide.iter().map(|m| [m[0].min(sea) - 2.5, m[1].max(sea)]).collect::<Vec<_>>()];
        let mut dims = vec![(nw0, nh0)];
        for _ in 1..=TOP {
            let (pw, ph) = *dims.last().unwrap();
            let (nw, nh) = (pw.div_ceil(2), ph.div_ceil(2));
            let prev = minmax.last().unwrap();
            let mut next = vec![[f32::MAX, f32::MIN]; nw * nh];
            for j in 0..ph {
                for i in 0..pw {
                    let k = (j / 2) * nw + i / 2;
                    next[k][0] = next[k][0].min(prev[j * pw + i][0]);
                    next[k][1] = next[k][1].max(prev[j * pw + i][1]);
                }
            }
            minmax.push(next);
            dims.push((nw, nh));
        }
        let texel_game = hd.texel_m * plan_scale;
        let origin = [hd.origin_x * plan_scale, hd.origin_z * plan_scale];
        let ext = [origin[0], origin[1], origin[0] + w as f64 * texel_game, origin[1] + h as f64 * texel_game];
        Self { minmax, dims, origin, node0: texel_game * PATCH as f64, texel_game, grid, range0, ext, hole: None }
    }

    /// The cooked extent on the plan (game m): min x, min z, max x, max z.
    pub fn extent(&self) -> [f64; 4] {
        self.ext
    }

    fn range(&self, level: u32) -> f64 {
        self.range0 * (1u64 << level) as f64
    }

    /// This frame's nodes, (x, z, size, level) in game metres (CDLOD): a node
    /// is split while any of it is nearer than the next finer level's range;
    /// nodes wholly inside the city grid and wholly within the far plane are
    /// left to the city's ground.
    pub fn select(&self, cam: &Camera) -> Vec<[f32; 4]> {
        self.select_with(cam, 0.0)
    }

    /// `select`, with the nodes along the cooked extent's edge tested in the
    /// frustum as reaching `skirt` game m further out (the skirt, stage L5).
    pub fn select_with(&self, cam: &Camera, skirt: f64) -> Vec<[f32; 4]> {
        self.select_full(cam, skirt, false)
    }

    /// `select_with`; `keep_deep`: the nodes inside the city grid within the
    /// far plane are kept too (the sea sheet, stage L5).
    pub fn select_full(&self, cam: &Camera, skirt: f64, keep_deep: bool) -> Vec<[f32; 4]> {
        let planes = frustum_planes(cam.view_proj());
        // The side and near planes only (reverse Z: index 4 is the far plane; the far layer has none).
        let planes: [Vec4; 5] = [planes[0], planes[1], planes[2], planes[3], planes[5]];
        let eye = cam.eye.as_dvec3();
        let mut nodes = Vec::with_capacity(512);
        let (tw, th) = self.dims[TOP as usize];
        for j in 0..th {
            for i in 0..tw {
                self.visit(TOP, i, j, &planes, eye, if keep_deep { 0.0 } else { cam.far as f64 }, skirt, &mut nodes);
            }
        }
        nodes
    }

    #[allow(clippy::too_many_arguments)]
    fn visit(&self, level: u32, i: usize, j: usize, planes: &[Vec4; 5], eye: glam::DVec3, far: f64, skirt: f64, out: &mut Vec<[f32; 4]>) {
        let (w, h) = self.dims[level as usize];
        if i >= w || j >= h || out.len() >= MAX_NODES {
            return;
        }
        let [y0, y1] = self.minmax[level as usize][j * w + i];
        if y0 > y1 {
            return;
        }
        let size = self.node0 * (1u64 << level) as f64;
        let x0 = self.origin[0] + i as f64 * size;
        let z0 = self.origin[1] + j as f64 * size;
        let (x1, z1) = (x0 + size, z0 + size);
        // the hole another layer draws: skipped when wholly inside, split across its edge
        let mut split = false;
        if let Some(hl) = self.hole {
            if x0 >= hl[0] && z0 >= hl[1] && x1 <= hl[2] && z1 <= hl[3] {
                return;
            }
            split = x0 < hl[2] && x1 > hl[0] && z0 < hl[3] && z1 > hl[1];
        }
        let (mut mn, mut mx) = (Vec3::new(x0 as f32, y0, z0 as f32), Vec3::new(x1 as f32, y1, z1 as f32));
        if skirt > 0.0 {
            // a node on the extent's edge carries the skirt beyond it (farshore.wgsl `fs_skirt`)
            let e = self.ext;
            let tol = 0.5 * self.texel_game;
            if x0 <= e[0] + tol {
                mn.x -= skirt as f32;
            }
            if x1 >= e[2] - tol {
                mx.x += skirt as f32;
            }
            if z0 <= e[1] + tol {
                mn.z -= skirt as f32;
            }
            if z1 >= e[3] - tol {
                mx.z += skirt as f32;
            }
        }
        if !planes.iter().all(|pl| {
            let n = pl.truncate();
            let p = Vec3::new(if n.x >= 0.0 { mx.x } else { mn.x }, if n.y >= 0.0 { mx.y } else { mn.y }, if n.z >= 0.0 { mx.z } else { mn.z });
            n.dot(p) + pl.w >= 0.0
        }) {
            return;
        }
        // Nearest and farthest points of the box from the eye.
        let near = glam::DVec3::new(eye.x.clamp(x0, x1), eye.y.clamp(y0 as f64, y1 as f64), eye.z.clamp(z0, z1));
        let dmin = (near - eye).length();
        let fx = if (eye.x - x0).abs() > (eye.x - x1).abs() { x0 } else { x1 };
        let fz = if (eye.z - z0).abs() > (eye.z - z1).abs() { z0 } else { z1 };
        let fy = if (eye.y - y0 as f64).abs() > (eye.y - y1 as f64).abs() { y0 as f64 } else { y1 as f64 };
        let dmax = (glam::DVec3::new(fx, fy, fz) - eye).length();
        let g = self.grid;
        let m = 2.0 * self.texel_game;
        let deep = x0 > g[0] as f64 + m && z0 > g[1] as f64 + m && x1 < g[2] as f64 - m && z1 < g[3] as f64 - m;
        if deep && dmax < far * 0.995 {
            return; // the city's own ground covers it
        }
        if level == 0 || (dmin > self.range(level - 1) && !split) {
            out.push([x0 as f32, z0 as f32, size as f32, level as f32]);
            return;
        }
        for (a, b) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            self.visit(level - 1, 2 * i + a, 2 * j + b, planes, eye, far, skirt, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The quadtree's choice for the cameras the review cards use: how many
    /// nodes (instances of the 2,048-triangle patch) and how long it takes on
    /// the CPU. Skipped (and says so) without the DEM or a cooked file.
    #[test]
    fn chooses_few_nodes_quickly() {
        let paths = eo_data::DataPaths::discover().unwrap();
        let cache = std::env::var("EO_FARSHORE_TEST_CACHE").ok().map(std::path::PathBuf::from);
        let Some(t) = eo_farshore::load_or_bake(&paths.repo_root, cache.as_deref()).unwrap() else {
            eprintln!("skipped: no DEM and no cooked file");
            return;
        };
        let g = eo_data::ElevationGrid::load(&paths).unwrap();
        let grid = [g.meta.origin_x as f32, g.meta.origin_z as f32, g.max_x() as f32, g.max_z() as f32];
        let (s, hs) = (eo_data::groundscale::scale(), eo_data::groundscale::height_scale());
        let tree = FarTree::new(&t, grid, s, hs, FarOptions::default().range0 as f64);
        let aspect = 16.0 / 9.0;
        let east = std::f32::consts::FRAC_PI_2;
        for (name, eye, yaw, pitch, far) in [
            ("Embarcadero, eye height, east", Vec3::new(3080.0, 3.0, -2085.0), east - 0.3, 0.02, 4200.0),
            ("Embarcadero, eye height, west", Vec3::new(3080.0, 3.0, -2085.0), -east, 0.02, 4200.0),
            ("over the bay, 1.6 km up", Vec3::new(6100.0, 1600.0, -1300.0), -2.2, -0.45, 9000.0),
            ("650 m over the western hills", Vec3::new(-4500.0, 650.0, 4125.0), 0.9, -0.12, 9000.0),
            ("18 km up, looking down", Vec3::new(0.0, 18000.0, 6000.0), 0.0, -1.2, 40000.0),
        ] {
            let cam = Camera { far, ..Camera::new(eye, yaw, pitch, aspect) };
            let t0 = std::time::Instant::now();
            let mut n = 0;
            for _ in 0..200 {
                n = tree.select(&cam).len();
            }
            let us = t0.elapsed().as_secs_f64() * 1e6 / 200.0;
            println!("{name}: {n} nodes ({:.0} k triangles submitted), {us:.0} us to choose", n as f64 * 2048.0 / 1e3);
            assert!(n > 0 && n < MAX_NODES / 2, "{name}: {n} nodes");
            assert!(us < 1000.0, "{name}: {us:.0} us");
        }
    }

    /// The far layer's shader composes and validates with the look compiled in
    /// and out, and its uniform is the Rust struct's size.
    #[test]
    fn the_shader_validates_with_and_without_the_look() {
        use wgpu::naga;
        for (cinematic, nature) in [(false, 0), (true, 0), (false, crate::look::NATURE_HILLS), (false, crate::look::NATURE_HILL_RELIEF), (true, crate::look::NATURE_HILLS | crate::look::NATURE_HILL_RELIEF), (true, crate::look::NATURE_ALL)] {
            let f = crate::look::ShaderFeatures { cinematic, nature, ..crate::look::ShaderFeatures::NONE };
            crate::look::with_shader_features(f, || {
                let src = shader_source();
                // (`--hill-cover`) its block in with its bit only, the lighting it replaces out
                assert_eq!(src.contains("fn fs_cover("), nature & crate::look::NATURE_HILLS != 0);
                assert_eq!(src.contains("lit = alb * (key + rim + irr * ao) / FS_PI;"), nature & crate::look::NATURE_HILLS == 0);
                // (`--hill-relief`) the relief and its hook with its bit only
                assert_eq!(src.contains("fn fs_relief("), nature & crate::look::NATURE_HILL_RELIEF != 0);
                assert_eq!(src.contains("let n = fs_relief(p, "), nature & crate::look::NATURE_HILL_RELIEF != 0);
                assert!(!src.contains("// sf:"));
                let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("farshore.wgsl: {}", e.emit_to_string(&src)));
                naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).expect("farshore.wgsl validates");
                let ty = module.types.iter().find(|(_, t)| t.name.as_deref() == Some("FarU")).expect("FarU").1;
                let naga::TypeInner::Struct { span, .. } = &ty.inner else { panic!("FarU is a struct") };
                assert_eq!(*span as usize, std::mem::size_of::<FarUniform>());
            });
        }
        // the shader's water rule is the one `far_draws_water_at` mirrors
        let src = include_str!("farshore.wgsl");
        assert!(src.contains("own_water = !in_sea;") && src.contains("SF_CINEMATIC && fu.ring.x > 0.5"));
    }

    /// Stage L5: under the cinematic look the sea and the far layer agree on who
    /// draws the water at every point (exactly one of them), whether the sea pass
    /// draws this frame or not; by default the void past the reach stays.
    #[test]
    fn the_sea_and_the_far_layer_agree_on_the_water() {
        let grid = [-8000.0f32, -9000.0, 9000.0, 8000.0];
        let reach = [grid[0] - 60000.0, grid[1] - 60000.0, grid[2] + 60000.0, grid[3]];
        let south = [reach[0], reach[1], reach[2], reach[3] + crate::water::CINEMATIC_SOUTH_REACH_M];
        let mut void_default = 0;
        for sea in [[0.0f32, 0.0, 4400.0, 1.0], [3000.0, -2000.0, 35200.0, 1.0], [0.0, 0.0, 70400.0, 0.0], [-50000.0, 20000.0, 140800.0, 1.0]] {
            for j in -60..=60 {
                for i in -60..=60 {
                    let p = [i as f32 * 2500.0 + 7.0, j as f32 * 2500.0 - 3.0];
                    let s = sea_draws_at(p, south, sea);
                    let f = far_draws_water_at(p, south, grid, sea, true);
                    assert!(s != f, "{p:?} sea {sea:?}: sea {s}, far {f}");
                    let (sd, fd) = (sea_draws_at(p, reach, sea), far_draws_water_at(p, reach, grid, sea, false));
                    assert!(!(sd && fd), "{p:?}: both draw by default");
                    void_default += (!sd && !fd) as u32;
                }
            }
        }
        assert!(void_default > 0, "the default keeps its void (the owner decides)");
    }

    /// Stage L5's sea test: decided by the fog given, so the volume's pushed-out
    /// fog (2e7 m) never skips the sea from the air, the old 3 km range fog does.
    #[test]
    fn the_sea_is_fogged_only_past_the_fog_in_use() {
        let cam = Camera { far: 40000.0, ..Camera::new(Vec3::new(0.0, 8000.0, 0.0), 0.0, -1.2, 16.0 / 9.0) };
        assert!(crate::water::sea_fogged(&cam, 3000.0));
        assert!(!crate::water::sea_fogged(&cam, 2.0e7));
        let low = Camera { far: 4000.0, ..Camera::new(Vec3::new(0.0, 30.0, 0.0), 0.0, -0.1, 16.0 / 9.0) };
        assert!(!crate::water::sea_fogged(&low, 3000.0));
    }

    /// The outer ring's quadtree leaves the inner cook's extent alone (its
    /// nodes meet it edge to edge), covers the rest of the ring, and chooses
    /// the same nodes each time.
    #[test]
    fn the_outer_ring_tiles_round_the_inner_cook() {
        use crate::farshore::outer::*;
        let (w, h) = (OUTER_WIDTH, OUTER_HEIGHT);
        let header = eo_farshore::CookHeader {
            version: eo_farshore::COOK_VERSION.into(),
            frame: String::new(),
            origin_x: OUTER_ORIGIN_X,
            origin_z: OUTER_ORIGIN_Z,
            texel_m: OUTER_TEXEL_M,
            width: w,
            height: h,
            sea_level_y: eo_data::SEA_LEVEL_Y,
            heights: String::new(),
            classes: String::new(),
            shape: String::new(),
            source: String::new(),
            tiles: vec![],
            city_grid: String::new(),
        };
        let heights: Vec<f32> = (0..w * h).map(|k| ((k % w) as f32 * 0.37).sin() * 200.0 + 100.0).collect();
        let t = eo_farshore::FarTerrain { header, heights, classes: vec![], shape: vec![], class_width: w / 2, class_height: h / 2, from: "test" };
        let inner = [eo_farshore::ORIGIN_X, eo_farshore::ORIGIN_Z, eo_farshore::ORIGIN_X + eo_farshore::WIDTH as f64 * eo_farshore::TEXEL_M, eo_farshore::ORIGIN_Z + eo_farshore::HEIGHT as f64 * eo_farshore::TEXEL_M];
        let grid = [-8000.0f32, -9000.0, 9000.0, 8000.0];
        let mut tree = FarTree::new(&t, grid, 1.0, 1.0, 7000.0 * 4.0);
        tree.hole = Some(inner);
        // from 600 km up looking straight down: the whole ring in view
        let cam = Camera { far: 2.0e6, ..Camera::new(Vec3::new(-6000.0, 600_000.0, -24000.0), 0.0, -1.5707, 1.0) };
        let nodes = tree.select_with(&cam, SKIRT_M as f64);
        assert_eq!(nodes, tree.select_with(&cam, SKIRT_M as f64));
        let mut area = 0.0f64;
        for n in &nodes {
            let (x0, z0, x1, z1) = (n[0] as f64, n[1] as f64, (n[0] + n[2]) as f64, (n[1] + n[2]) as f64);
            let overlap = (x1.min(inner[2]) - x0.max(inner[0])).max(0.0) * (z1.min(inner[3]) - z0.max(inner[1])).max(0.0);
            assert_eq!(overlap, 0.0, "node {n:?} overlaps the inner cook");
            let e = extent();
            area += (x1.min(e[2]) - x0.max(e[0])).max(0.0) * (z1.min(e[3]) - z0.max(e[1])).max(0.0);
        }
        let e = extent();
        let want = (e[2] - e[0]) * (e[3] - e[1]) - (inner[2] - inner[0]) * (inner[3] - inner[1]);
        assert!((area - want).abs() < 1.0, "covered {area} of {want}");
        assert!(nodes.len() < MAX_NODES / 2, "{} nodes", nodes.len());
    }

    #[test]
    fn grass_is_golden_in_late_summer_and_green_in_winter() {
        assert!(dryness_for_day(265.0) > 0.99); // the sun model's date (September 22)
        assert!(dryness_for_day(45.0) < 0.01); // mid-February
        assert!(dryness_for_day(130.0) > 0.2 && dryness_for_day(130.0) < 0.8); // May: browning
    }
}

#[cfg(test)]
mod palette_tests {
    #[test]
    fn the_shared_palette_is_the_far_layers() {
        // every vec3<f32>(a, b, c) literal of the shader, parsed
        let w = include_str!("farshore.wgsl");
        let lits: Vec<[f32; 3]> = w
            .split("vec3<f32>(")
            .skip(1)
            .filter_map(|t| {
                let v: Vec<f32> = t.split(')').next()?.split(',').map(|x| x.trim().parse::<f32>()).collect::<Result<_, _>>().ok()?;
                (v.len() == 3).then(|| [v[0], v[1], v[2]])
            })
            .collect();
        for c in [super::FAR_GRASS_GREEN, super::FAR_GRASS_GOLDEN, super::FAR_SCRUB, super::FAR_TREE, super::FAR_MARSH, super::FAR_ROCK, super::FAR_TOWN] {
            assert!(lits.contains(&c), "{c:?} not in farshore.wgsl");
        }
    }
}
