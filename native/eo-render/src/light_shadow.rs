//! Shadows of local lights (lighting): street lamps (`LightingOptions::
//! light_shadows`, sixteens of lamps; `light_shadow_moving` adds moving
//! casters for the nearest few) and headlights (`headlight_shadows`, the
//! nearest N moving vehicles), in one depth atlas at group 1 binding 17 with
//! its table at 18 (`LookGpu::set_light_shadows`), read by the city's local
//! lights (sky_light.wgsl `sky_local_lights_ao`, light_shadow_use.wgsl) and
//! the lit fog (lit_fog.wgsl `lf_light_visibility`).
//!
//! Everything is decided on the GPU, after the sky's light gather (the frame
//! list exists only there), in one compute pass and two depth passes:
//!
//! 1. `ls_rank`: the frame's lights ranked by their distance to the eye
//!    (ties by position, kind, intensity, then slot), so `ls_cluster` builds
//!    every light cluster's list in that order (the 32 a cluster keeps are
//!    its nearest, the same set and order frame after frame and run after
//!    run, where the gather's atomic appends shuffled them).
//! 2. `ls_vehicles`, `ls_select` (one invocation): the `lamps` nearest street
//!    lamps keep the atlas tile their luminaire (its position's bits) held
//!    the frame before; a lamp without one takes a free tile, at most
//!    `NEW_MAPS_PER_FRAME` a frame, nearest first (the map is drawn this
//!    frame, from the static scene and the parked cars, and kept while the
//!    lamp stays selected). With `light_shadow_moving` the `MOVING_LAMPS`
//!    nearest are drawn again every frame into tiles of their own with every
//!    agent. The `heads` nearest vehicles with their low beams lit get a map
//!    each every frame (one frustum over both beams, every agent but the
//!    vehicle itself). The table's rows for every tile in use.
//! 3. `ls_pads`: each frame light's spare word (`SkyLight.pad`, 0 in the
//!    gather) gets its tile + 1, or -1 (no map: only the surface's
//!    occlusion multiplies it).
//! 4. `ls_cull_*`: the city's objects (each at the level it is drawn this
//!    frame, as the sun cascade 0's casters), the props (their middle level)
//!    and the agents (vehicles' first reduced level, people's mid) against
//!    each tile drawn this frame, appended as records (tile, index range,
//!    base vertex, instance) to six lists (lamps / headlights x city / props
//!    / agents); `ls_scan` sums their triangles.
//! 5. Two depth passes over the atlas (lamp tiles, headlight tiles), each a
//!    tile clear and ONE non-indexed indirect draw per list: the vertex
//!    stage finds its record by the triangle's index (a binary search of the
//!    sums) and pulls the index and the vertex from the geometry's own
//!    buffers (bound as storage), so no draw count is read back and no
//!    command list is compacted. Each triangle is clipped to its tile's cell
//!    (clip distances; without the device feature its fragments outside the
//!    cell are discarded); the depth kept is linear along the light's axis.
//!
//! Timed in `lighting::Q_LIGHT_SHADOWS` (the compute pass and the lamp pass)
//! and `lighting::Q_HEADLIGHT_SHADOWS` (the headlight pass). Nothing runs by
//! day with the lamps off (`SkyGpu::local_lights_on`), and with both options
//! off nothing is made: the frame is the one without them.

use crate::lighting::{self, Ts};
use crate::look::LightingOptions;
use bytemuck::{Pod, Zeroable};
use std::sync::Mutex;

/// A tile's side (texels): 2 x tan(72 deg) x 8.84 m / 512 = 0.106 m on the
/// ground under an arterial cobra head (0.062 m under a 5.12 m column).
pub const TILE_PX: u32 = 512;
/// The atlas's format: the caster's depth along the light's axis, linear
/// over the frustum's near to far (36 m: 0.55 mm a step).
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth16Unorm;
/// New lamp maps drawn in one frame at most (nearest first; the rest wait).
pub const NEW_MAPS_PER_FRAME: u32 = 2;
/// With `light_shadow_moving`: the nearest lamps whose maps are drawn again
/// every frame with every agent, into tiles of their own.
pub const MOVING_LAMPS: u32 = 4;
/// Lamp tiles at most (`--light-shadows 48`).
pub const MAX_LAMPS: u32 = 48;
/// Headlight tiles at most (`--headlight-shadows` beyond it is clamped).
pub const MAX_HEADLIGHTS: u32 = 8;
/// Tiles drawn in one frame at most: the lamp group's, then the headlights'.
pub const LAMP_REDRAWS: u32 = NEW_MAPS_PER_FRAME + MOVING_LAMPS;
pub const REDRAWS: u32 = LAMP_REDRAWS + MAX_HEADLIGHTS;
/// Records a caster list holds (lamps / headlights x city / props / agents).
pub const REC_CAP: u32 = 8192;
pub const LISTS: u32 = 6;
/// Words of a list's block in the lists buffer (256 bytes: count, triangles, the draw's arguments).
const LIST_WORDS: u32 = 64;
/// The vehicle candidates' count, in the lists buffer.
const CAND_WORD: u32 = LISTS * LIST_WORDS;
const LISTS_WORDS: u32 = CAND_WORD + 4;
/// The culls' dispatch arguments (city, props, agents: four words each), a buffer of their own.
const ARGS_WORDS: u32 = 12;
/// Vehicles a frame considers for headlight maps (the nearest win).
pub const CAND_CAP: u32 = 256;
/// A street lamp's map: a spot straight down from its luminaire, this half
/// angle from the nadir (the full-cutoff pools reach 60-75 deg; the rim fades
/// to unshadowed), from LAMP_NEAR_M under the lens (its own housing is not a caster).
pub const LAMP_HALF_ANGLE_DEG: f32 = 72.0;
pub const LAMP_NEAR_M: f32 = 0.25;
/// A headlight pair's map: from between the lamps along the vehicle,
/// across the low beam's cone (sky_lights.wgsl 0.42 rad) and the lamps'
/// spread, from just over the cut-off down to the road a few metres ahead.
pub const HEAD_HALF_WIDTH_RAD: f32 = 0.42 + 0.12;
pub const HEAD_UP_DEG: f32 = 6.0;
pub const HEAD_DOWN_DEG: f32 = 32.0;
pub const HEAD_NEAR_M: f32 = 0.3;
/// The receiver's bias: its depth toward the light, and its offset along
/// its normal in texels of the map at its depth.
pub const DEPTH_BIAS_M: f32 = 0.04;
pub const NORMAL_OFFSET_TEXELS: f32 = 1.5;
/// The lookup's four bilinear compares this far from the point (texels).
pub const PCF_TEXELS: f32 = 0.75;
/// Where a map's rim starts fading to unshadowed (share of the half width).
pub const RIM_FADE: f32 = 0.85;
/// The casters' levels: props their middle level, vehicles their first
/// reduced one, people the mid tier (a lamp's map is 0.1 m a texel).
pub const PROP_LOD: u32 = 1;
pub const VEHICLE_LOD: u32 = 1;
pub const PERSON_LOD: u32 = 2;

/// What the atlas holds for a set of options.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub lamps: u32,
    pub moving: u32,
    pub heads: u32,
    pub cols: u32,
    pub rows: u32,
}

impl Layout {
    /// None with both options off.
    pub fn of(o: &LightingOptions) -> Option<Layout> {
        let lamps = o.shadowed_lamps().min(MAX_LAMPS);
        let moving = if o.light_shadow_moving { MOVING_LAMPS.min(lamps) } else { 0 };
        let heads = (o.headlight_shadows as u32).min(MAX_HEADLIGHTS);
        let tiles = lamps + moving + heads;
        if tiles == 0 {
            return None;
        }
        let cols = (tiles as f64).sqrt().ceil() as u32;
        Some(Layout { lamps, moving, heads, cols, rows: tiles.div_ceil(cols) })
    }

    pub fn tiles(&self) -> u32 {
        self.lamps + self.moving + self.heads
    }

    /// The atlas's texels (width, height).
    pub fn atlas_px(&self) -> (u32, u32) {
        (self.cols * TILE_PX, self.rows * TILE_PX)
    }

    /// GPU bytes: the atlas and the buffers.
    pub fn bytes(&self) -> u64 {
        let (w, h) = self.atlas_px();
        w as u64 * h as u64 * 2 + buffer_bytes()
    }
}

fn table_rows() -> u64 {
    1 + 5 * (MAX_LAMPS + MOVING_LAMPS + MAX_HEADLIGHTS) as u64
}

/// `LsFrame` (light_shadow.wgsl): two header rows, the redraws, their planes, the selected lamps, the chosen headlamps.
fn frame_rows() -> u64 {
    2 + REDRAWS as u64 + REDRAWS as u64 * 6 + MAX_LAMPS as u64 + MAX_HEADLIGHTS as u64 * 2
}

fn buffer_bytes() -> u64 {
    let params = std::mem::size_of::<LsParams>() as u64;
    let order = crate::sky::FRAME_LIGHT_CAP as u64 * 4;
    params + order + MAX_LAMPS as u64 * 16 + frame_rows() * 16 + table_rows() * 16 + CAND_CAP as u64 * 8 + LISTS_WORDS as u64 * 4 + ARGS_WORDS as u64 * 4 + LISTS as u64 * REC_CAP as u64 * 32
}

/// The props' and agents' geometry and group 2, as their own passes bind
/// them (props.rs, agents.rs keep one; the casters here read the vertex and
/// index buffers as storage).
#[derive(Clone, Debug)]
pub struct Casters {
    pub vertices: wgpu::Buffer,
    pub indices: wgpu::Buffer,
    /// prop instances / agents
    pub instances: wgpu::Buffer,
    pub models: wgpu::Buffer,
    pub meshes: wgpu::Buffer,
    pub data_bgl: wgpu::BindGroupLayout,
    pub data_bg: wgpu::BindGroup,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, Default)]
struct LsParams {
    /// x city objects, y prop slots in use, z agents, w props present (1) | agents present (2)
    counts: [u32; 4],
    /// x lamp tiles, y moving tiles, z headlight tiles, w columns
    tiles: [u32; 4],
    /// columns, rows, tile texels, tiles
    grid: [f32; 4],
    /// x tan of the lamp's half angle, y lamp near, z headlight near, w 1: no light reads its map (diagnostic)
    lamp: [f32; 4],
    /// x tan of the headlights' half width, y tan up, z tan down
    head: [f32; 4],
}

/// What the light shadows did, for the logs, the bench and the tests.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LightShadowStats {
    pub layout: Option<Layout>,
    pub bytes: u64,
    /// Frames the passes ran, and frames they were skipped with an option on (no light lit).
    pub frames: u64,
    pub skipped: u64,
}

/// One frame's selection as the GPU left it (`LightShadowGpu::readback`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    /// Tiles drawn this frame: lamp group, headlights.
    pub lamp_redraws: u32,
    pub head_redraws: u32,
    /// New lamp maps this frame, lamps selected, all new lamp maps since the start, frames.
    pub new_maps: u32,
    pub selected: u32,
    pub new_total: u32,
    pub frames: u32,
    /// The selected lamps: their luminaire's position bits and the tile shading reads (u32::MAX: none yet).
    pub lamps: Vec<([u32; 3], u32)>,
    /// The lamp tiles' keys (position bits) and whether each holds a map.
    pub tiles: Vec<([u32; 3], bool)>,
    /// The six caster lists' records (count, triangles).
    pub lists: Vec<(u32, u32)>,
    /// Vehicle candidates and the vehicles chosen for headlight maps.
    pub candidates: u32,
    pub vehicles: u32,
    /// The frame's lights that read a map: street lamps, headlamps.
    pub lamps_shadowed: u32,
    pub headlamps_shadowed: u32,
}

/// The constants the lookup (light_shadow_use.wgsl) is compiled with, and
/// the file; `on` false compiles the city's light shadow test out (the far
/// layer's variant).
pub fn use_wgsl(on: bool) -> String {
    let (c, k) = crate::sky::led_params();
    format!(
        "\n// generated by light_shadow.rs use_wgsl\nconst LS_SHADER: bool = {on};\nconst LS_DEPTH_BIAS_M: f32 = {DEPTH_BIAS_M:.6};\nconst LS_NORMAL_OFFSET_TEXELS: f32 = {NORMAL_OFFSET_TEXELS:.6};\nconst LS_PCF_TEXELS: f32 = {PCF_TEXELS:.6};\nconst LS_RIM_FADE: f32 = {RIM_FADE:.6};\nconst LS_LED_RGB: vec3<f32> = vec3<f32>({:.6}, {:.6}, {:.6});\nconst LS_LED_KEY: vec3<f32> = vec3<f32>({:.6}, {:.6}, {:.6});\nconst LS_LED_KEY_TOLERANCE: f32 = {:.6};\n{}",
        c[0],
        c[1],
        c[2],
        k[0],
        k[1],
        k[2],
        k[3],
        include_str!("light_shadow_use.wgsl")
    )
}

/// The constants both of this module's shaders are compiled with.
fn constants_wgsl() -> String {
    format!(
        "\n// generated by light_shadow.rs\nconst LS_MAX_LAMPS: u32 = {MAX_LAMPS}u;\nconst LS_MAX_HEADS: u32 = {MAX_HEADLIGHTS}u;\nconst LS_NEW_MAPS: u32 = {NEW_MAPS_PER_FRAME}u;\nconst LS_LAMP_REDRAWS: u32 = {LAMP_REDRAWS}u;\nconst LS_REDRAWS: u32 = {REDRAWS}u;\nconst LS_PLANES: u32 = {}u;\nconst LS_HEAD_LAMPS: u32 = {}u;\nconst LS_REC_CAP: u32 = {REC_CAP}u;\nconst LS_LIST_WORDS: u32 = {LIST_WORDS}u;\nconst LS_CAND_WORD: u32 = {CAND_WORD}u;\nconst LS_CAND_CAP: u32 = {CAND_CAP}u;\nconst LS_FRAME_CAP: u32 = {}u;\nconst LS_CLUSTER_CAP: u32 = {}u;\nconst LS_PROP_LOD: u32 = {PROP_LOD}u;\nconst LS_VEHICLE_LOD: u32 = {VEHICLE_LOD}u;\nconst LS_PERSON_LOD: u32 = {PERSON_LOD}u;\nconst LS_TILE_PX: f32 = {TILE_PX}.0;\n",
        REDRAWS * 6,
        MAX_HEADLIGHTS * 2,
        crate::sky::FRAME_LIGHT_CAP,
        crate::sky::CLUSTER_CAP,
    )
}

/// The compute passes' WGSL (sky_common.wgsl, the sea, the constants, light_shadow.wgsl).
pub fn compute_source() -> String {
    format!("{}\n{}\n{}\n{}\n{}", include_str!("sky_common.wgsl"), crate::sky::sea_wgsl(), constants_wgsl(), crate::shop_lights::wgsl(), include_str!("light_shadow.wgsl"))
}

/// The depth passes' WGSL for one kind of caster: "clear" / "city" (the
/// common part), "props" (+ props_types.wgsl), "agents" (+ agents_types.wgsl,
/// agents_common.wgsl). `clip`: the device has clip distances (each
/// triangle clipped to its tile; else the fragments outside it are discarded,
/// which costs the fragments of a triangle that reaches far past it).
pub fn draw_source(kind: &str, clip: bool) -> String {
    let out = format!(
        "struct LsOut {{\n    @builtin(position) clip: vec4<f32>,\n{}    @location(0) lin: f32,\n    @location(1) @interpolate(flat) cell: vec4<f32>,\n    @location(2) uv: vec2<f32>,\n    @location(3) @interpolate(flat) mat: vec2<u32>,\n}};\n// what the fragment stage takes (no clip distances)\nstruct LsIn {{\n    @builtin(position) clip: vec4<f32>,\n    @location(0) lin: f32,\n    @location(1) @interpolate(flat) cell: vec4<f32>,\n    @location(2) uv: vec2<f32>,\n    @location(3) @interpolate(flat) mat: vec2<u32>,\n}};\nfn ls_clip_to_tile(o: ptr<function, LsOut>, d: vec4<f32>) {{\n{}}}\n",
        if clip { "    @builtin(clip_distances) sides: array<f32, 4>,\n" } else { "" },
        if clip { "    (*o).sides = array<f32, 4>(d.x, d.y, d.z, d.w);\n" } else { "" }
    );
    let common = format!("{}\n{out}\n{}", constants_wgsl(), include_str!("light_shadow_draw.wgsl"));
    let body = match kind {
        "props" => format!("{}\n{}\n{}", include_str!("props_types.wgsl"), common, include_str!("light_shadow_props.wgsl")),
        "agents" => format!("{}\n{}\n{}\n{}", include_str!("agents_types.wgsl"), include_str!("agents_common.wgsl"), common, include_str!("light_shadow_agents.wgsl")),
        _ => common,
    };
    if clip { format!("enable clip_distances;\n{body}") } else { body }
}

struct Gpu {
    layout: Layout,
    atlas_draw: wgpu::TextureView,
    atlas_sample: wgpu::TextureView,
    params: wgpu::Buffer,
    frame: wgpu::Buffer,
    table: wgpu::Buffer,
    state: wgpu::Buffer,
    lists: wgpu::Buffer,
    args: wgpu::Buffer,
    args_bg: wgpu::BindGroup,
    order: wgpu::Buffer,
    cand: wgpu::Buffer,
    recs: wgpu::Buffer,
    compute_bgl: wgpu::BindGroupLayout,
    /// rank, cluster, vehicles, select, pads, cull city, cull props, cull agents, scan
    pipes: Vec<wgpu::ComputePipeline>,
    draw_bgl: wgpu::BindGroupLayout,
    clear_bgl: wgpu::BindGroupLayout,
    clear_pipe: wgpu::RenderPipeline,
    clear_bg: wgpu::BindGroup,
    city_pipe: wgpu::RenderPipeline,
    /// per list (lamps city, props, agents, headlights city, props, agents)
    list_bgs: Vec<Option<wgpu::BindGroup>>,
    props_pipe: Option<wgpu::RenderPipeline>,
    agents_pipe: Option<wgpu::RenderPipeline>,
    /// the compute group, and whether it was made with the props' and agents' buffers.
    compute_bg: Option<(wgpu::BindGroup, bool, bool)>,
    /// Whether the city's geometry fits a storage binding (else its casters are left out).
    city_ok: bool,
    /// This frame's: props present, agents present (`update`).
    have: (bool, bool),
    atlas: wgpu::Texture,
}

pub struct LightShadowGpu {
    device: wgpu::Device,
    gpu: Mutex<Option<Gpu>>,
    stats: Mutex<LightShadowStats>,
    /// Diagnostic (`set_maps_read`): the maps drawn but read by no light.
    maps_unread: std::sync::atomic::AtomicBool,
}

impl LightShadowGpu {
    pub fn new(device: &wgpu::Device) -> Self {
        Self { device: device.clone(), gpu: Mutex::new(None), stats: Mutex::new(LightShadowStats::default()), maps_unread: std::sync::atomic::AtomicBool::new(false) }
    }

    /// Diagnostic (the capture cards' `EO_LIGHT_SHADOW_MAPS_OFF=1`): with
    /// `false` every light is drawn as with the options on (the selection,
    /// the stable clusters, the LED colour, the occlusion) but none reads its
    /// map, so a pair shows the shadows alone.
    pub fn set_maps_read(&self, on: bool) {
        self.maps_unread.store(!on, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn stats(&self) -> LightShadowStats {
        *self.stats.lock().unwrap()
    }

    /// GPU bytes held (0 until an option is first used).
    pub fn bytes(&self) -> u64 {
        self.stats.lock().unwrap().bytes
    }

    /// Before the look's update, every frame: the atlas made (or let go)
    /// for the options, bound at group 1, and this frame's parameters.
    pub(crate) fn update(&self, r: &crate::Renderer, queue: &wgpu::Queue) {
        let layout = Layout::of(&r.look.options.lighting);
        let mut g = self.gpu.lock().unwrap();
        if g.as_ref().map(|g| g.layout) != layout {
            *g = layout.map(|l| Gpu::new(&self.device, r, l));
            match g.as_ref() {
                Some(gpu) => {
                    r.look.set_light_shadows(Some(&gpu.atlas_sample), Some(&gpu.table));
                    let (w, h) = gpu.layout.atlas_px();
                    eprintln!(
                        "light shadows: {} lamp tiles (+{} moving), {} headlight tiles, atlas {} x {} ({} x {} of {} px), {:.1} MB",
                        gpu.layout.lamps,
                        gpu.layout.moving,
                        gpu.layout.heads,
                        w,
                        h,
                        gpu.layout.cols,
                        gpu.layout.rows,
                        TILE_PX,
                        gpu.layout.bytes() as f64 / 1e6
                    );
                }
                None => r.look.set_light_shadows(None, None),
            }
            *self.stats.lock().unwrap() = LightShadowStats { layout, bytes: layout.map(|l| l.bytes()).unwrap_or(0), ..Default::default() };
        }
        let Some(gpu) = g.as_mut() else { return };
        let props = r.props.get().filter(|p| p.enabled() && p.high_water() > 0);
        let agents = r.agents.get().filter(|a| a.enabled() && a.count() > 0);
        gpu.have = (props.is_some(), agents.is_some());
        gpu.ensure_casters(&self.device, r);
        let l = gpu.layout;
        let p = LsParams {
            counts: [if gpu.city_ok { r.object_count } else { 0 }, props.map(|p| p.high_water()).unwrap_or(0), agents.map(|a| a.count()).unwrap_or(0), props.is_some() as u32 | (agents.is_some() as u32) << 1],
            tiles: [l.lamps, l.moving, l.heads, l.cols],
            grid: [l.cols as f32, l.rows as f32, TILE_PX as f32, l.tiles() as f32],
            lamp: [LAMP_HALF_ANGLE_DEG.to_radians().tan(), LAMP_NEAR_M, HEAD_NEAR_M, if self.maps_unread.load(std::sync::atomic::Ordering::Relaxed) { 1.0 } else { 0.0 }],
            head: [HEAD_HALF_WIDTH_RAD.tan(), HEAD_UP_DEG.to_radians().tan(), HEAD_DOWN_DEG.to_radians().tan(), 0.0],
        };
        queue.write_buffer(&gpu.params, 0, bytemuck::bytes_of(&p));
    }

    /// The lamp and headlight maps for this frame: after the sky-lights pass
    /// (this frame's light list and clusters) and before the early city
    /// pass. Lamps (and the selection) are timed in `lighting::Q_LIGHT_SHADOWS`,
    /// headlights in `lighting::Q_HEADLIGHT_SHADOWS`.
    pub(crate) fn encode(&self, enc: &mut wgpu::CommandEncoder, r: &crate::Renderer, ts: Ts<'_>) {
        let o = r.look.options.lighting;
        let lamps_on = o.light_shadows > 0;
        let heads_on = o.headlight_shadows > 0;
        let off = |enc: &mut wgpu::CommandEncoder, why: &str| {
            lighting::empty_span(enc, ts, lighting::Q_LIGHT_SHADOWS, &format!("light shadows ({why})"));
            lighting::empty_span(enc, ts, lighting::Q_HEADLIGHT_SHADOWS, &format!("headlight shadows ({why})"));
        };
        let mut g = self.gpu.lock().unwrap();
        let Some(gpu) = g.as_mut() else {
            off(enc, "off");
            return;
        };
        if !r.look.sky.local_lights_on() || !r.look.sky.options().lights {
            // by day with the lamps off: nothing to shadow (the gather's records keep their 0).
            self.stats.lock().unwrap().skipped += 1;
            off(enc, "no lights");
            return;
        }
        let Some((cbg, _, _)) = gpu.compute_bg.as_ref() else {
            off(enc, "not ready");
            return;
        };
        let l = gpu.layout;
        enc.clear_buffer(&gpu.lists, 0, None);
        // the selection and the culls: the lamps' slot (the headlights' when only they are on).
        let first = if lamps_on { lighting::Q_LIGHT_SHADOWS } else { lighting::Q_HEADLIGHT_SHADOWS };
        if !lamps_on {
            lighting::empty_span(enc, ts, lighting::Q_LIGHT_SHADOWS, "light shadows (off)");
        }
        {
            let tw = ts.map(|(q, b)| wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(b + first), end_of_pass_write_index: None });
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("light shadows (select, cull)"), timestamp_writes: tw });
            pass.set_bind_group(0, cbg, &[]);
            let lights = crate::sky::FRAME_LIGHT_CAP.div_ceil(64);
            let clusters = crate::sky::CLUSTERS.iter().product::<u32>();
            pass.set_pipeline(&gpu.pipes[0]);
            pass.dispatch_workgroups(lights, 1, 1);
            pass.set_pipeline(&gpu.pipes[1]);
            pass.dispatch_workgroups(clusters, 1, 1);
            let agents = r.agents.get().filter(|_| gpu.have.1).map(|a| a.count()).unwrap_or(0);
            if l.heads > 0 && agents > 0 {
                pass.set_pipeline(&gpu.pipes[2]);
                pass.dispatch_workgroups(agents.div_ceil(64), 1, 1);
            }
            pass.set_pipeline(&gpu.pipes[3]);
            pass.set_bind_group(1, &gpu.args_bg, &[]);
            pass.dispatch_workgroups(1, 1, 1);
            pass.set_pipeline(&gpu.pipes[4]);
            pass.dispatch_workgroups(lights, 1, 1);
            for (k, p) in gpu.pipes[5..8].iter().enumerate() {
                pass.set_pipeline(p);
                pass.dispatch_workgroups_indirect(&gpu.args, 16 * k as u64);
            }
            pass.set_pipeline(&gpu.pipes[8]);
            pass.dispatch_workgroups(LISTS, 1, 1);
        }
        for (group, on, slot) in [(0u32, lamps_on, lighting::Q_LIGHT_SHADOWS), (1, heads_on, lighting::Q_HEADLIGHT_SHADOWS)] {
            if !on {
                if group == 1 {
                    lighting::empty_span(enc, ts, slot, "headlight shadows (off)");
                }
                continue;
            }
            let tw = ts.map(|(q, b)| wgpu::RenderPassTimestampWrites {
                query_set: q,
                beginning_of_pass_write_index: (group == 1 && lamps_on).then_some(b + slot),
                end_of_pass_write_index: Some(b + slot + 1),
            });
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(if group == 0 { "light shadows (lamps)" } else { "light shadows (headlights)" }),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &gpu.atlas_draw, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }), stencil_ops: None }),
                timestamp_writes: tw,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // the tiles drawn this frame cleared to the far plane
            pass.set_pipeline(&gpu.clear_pipe);
            pass.set_bind_group(0, &gpu.clear_bg, &[]);
            let redraws = if group == 0 { 0..LAMP_REDRAWS } else { LAMP_REDRAWS..REDRAWS };
            pass.draw(0..6, redraws);
            for (k, pipe) in [Some(&gpu.city_pipe), gpu.props_pipe.as_ref(), gpu.agents_pipe.as_ref()].into_iter().enumerate() {
                let list = group * 3 + k as u32;
                let (Some(pipe), Some(bg)) = (pipe, gpu.list_bgs[list as usize].as_ref()) else { continue };
                if k == 1 && !gpu.have.0 || k == 2 && !gpu.have.1 {
                    continue;
                }
                pass.set_pipeline(pipe);
                pass.set_bind_group(0, bg, &[]);
                if k == 1 {
                    pass.set_bind_group(2, &r.props.get().unwrap().casters.data_bg, &[]);
                } else if k == 2 {
                    pass.set_bind_group(2, &r.agents.get().unwrap().casters.data_bg, &[]);
                }
                pass.draw_indirect(&gpu.lists, (list * LIST_WORDS) as u64 * 4 + 16);
            }
        }
        self.stats.lock().unwrap().frames += 1;
    }

    /// The selection as the GPU last left it (tests and diagnostics; waits for the GPU).
    pub fn readback(&self, ctx: &crate::GpuContext) -> Option<Selection> {
        let g = self.gpu.lock().unwrap();
        let gpu = g.as_ref()?;
        let read = |b: &wgpu::Buffer, size: u64| -> Option<Vec<u32>> {
            let s = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: Some("light-shadow-read"), size, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            let mut enc = ctx.device.create_command_encoder(&Default::default());
            enc.copy_buffer_to_buffer(b, 0, &s, 0, size);
            ctx.queue.submit([enc.finish()]);
            crate::read_mapped(ctx, &s).ok().map(|v| bytemuck::pod_collect_to_vec(&v))
        };
        let f = read(&gpu.frame, frame_rows() * 16)?;
        let st = read(&gpu.state, MAX_LAMPS as u64 * 16)?;
        let li = read(&gpu.lists, LISTS_WORDS as u64 * 4)?;
        let sel_at = (2 + REDRAWS + REDRAWS * 6) as usize * 4;
        let selected = f[3];
        Some(Selection {
            lamp_redraws: f[0],
            head_redraws: f[1],
            new_maps: f[2],
            selected,
            new_total: f[4],
            frames: f[5],
            vehicles: f[6],
            candidates: f[7],
            lamps: (0..selected as usize).map(|s| ([f[sel_at + s * 4], f[sel_at + s * 4 + 1], f[sel_at + s * 4 + 2]], f[sel_at + s * 4 + 3])).collect(),
            tiles: (0..gpu.layout.lamps as usize).map(|t| ([st[t * 4], st[t * 4 + 1], st[t * 4 + 2]], st[t * 4 + 3] == 1)).collect(),
            lists: (0..LISTS as usize).map(|k| (li[k * LIST_WORDS as usize], li[k * LIST_WORDS as usize + 1])).collect(),
            lamps_shadowed: li[CAND_WORD as usize + 1],
            headlamps_shadowed: li[CAND_WORD as usize + 2],
        })
    }

    /// The atlas as last drawn: width, height and its depths (16 bits, row
    /// by row; 1 the far plane). Waits for the GPU (diagnostics: the capture
    /// cards' `EO_LIGHT_SHADOW_ATLAS`).
    pub fn atlas_readback(&self, ctx: &crate::GpuContext) -> Option<(u32, u32, Vec<u16>)> {
        let g = self.gpu.lock().unwrap();
        let gpu = g.as_ref()?;
        let (w, h) = gpu.layout.atlas_px();
        let row = (w * 2).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let b = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: Some("light-shadow-atlas-read"), size: row as u64 * h as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &gpu.atlas, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::DepthOnly },
            wgpu::TexelCopyBufferInfo { buffer: &b, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        ctx.queue.submit([enc.finish()]);
        let raw = crate::read_mapped(ctx, &b).ok()?;
        Some((w, h, raw.chunks(row as usize).flat_map(|r| bytemuck::pod_collect_to_vec::<u8, u16>(&r[..(w * 2) as usize])).collect()))
    }

    /// The frame's lights in the order `ls_rank` left them: each one's
    /// position, kind and spare word (tests: the order of the lights
    /// themselves, whatever slots the gather's appends gave them).
    pub fn order_readback(&self, ctx: &crate::GpuContext, sky: &crate::sky::SkyGpu) -> Option<Vec<([f32; 3], u32, f32)>> {
        let g = self.gpu.lock().unwrap();
        let gpu = g.as_ref()?;
        let cap = crate::sky::FRAME_LIGHT_CAP as u64;
        let s = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: Some("light-shadow-order-read"), size: cap * 4 + 16 + cap * 64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(&gpu.order, 0, &s, 0, cap * 4);
        enc.copy_buffer_to_buffer(&sky.counter, 0, &s, cap * 4, 16);
        enc.copy_buffer_to_buffer(&sky.frame_lights, 0, &s, cap * 4 + 16, cap * 64);
        ctx.queue.submit([enc.finish()]);
        let w: Vec<u32> = bytemuck::pod_collect_to_vec(&crate::read_mapped(ctx, &s).ok()?);
        let n = (w[cap as usize] as usize).min(cap as usize);
        let lights = &w[cap as usize + 4..];
        Some(w[..n].iter().map(|&i| {
            let l = &lights[i as usize * 16..i as usize * 16 + 16];
            ([f32::from_bits(l[0]), f32::from_bits(l[1]), f32::from_bits(l[2])], l[13], f32::from_bits(l[15]))
        }).collect())
    }
}

impl Gpu {
    fn new(d: &wgpu::Device, r: &crate::Renderer, layout: Layout) -> Gpu {
        use wgpu::BufferUsages as U;
        let (w, h) = layout.atlas_px();
        let atlas = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("light-shadow-atlas"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let atlas_draw = atlas.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2), ..Default::default() });
        let atlas_sample = atlas.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        let buf = |label: &str, size: u64, usage: wgpu::BufferUsages| d.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false });
        let params = buf("light-shadow-params", std::mem::size_of::<LsParams>() as u64, U::UNIFORM | U::COPY_DST);
        let frame = buf("light-shadow-frame", frame_rows() * 16, U::STORAGE | U::COPY_SRC);
        let table = buf("light-shadow-table", table_rows() * 16, U::STORAGE | U::COPY_SRC);
        // the lamp tiles' keys: zero (none held) at the start.
        let state = buf("light-shadow-state", MAX_LAMPS as u64 * 16, U::STORAGE | U::COPY_SRC);
        let lists = buf("light-shadow-lists", LISTS_WORDS as u64 * 4, U::STORAGE | U::INDIRECT | U::COPY_DST | U::COPY_SRC);
        let args = buf("light-shadow-dispatch", ARGS_WORDS as u64 * 4, U::STORAGE | U::INDIRECT);
        let order = buf("light-shadow-order", crate::sky::FRAME_LIGHT_CAP as u64 * 4, U::STORAGE | U::COPY_SRC);
        let cand = buf("light-shadow-candidates", CAND_CAP as u64 * 8, U::STORAGE);
        let recs = buf("light-shadow-records", LISTS as u64 * REC_CAP as u64 * 32, U::STORAGE);

        let entry = |binding, visibility, ty| wgpu::BindGroupLayoutEntry { binding, visibility, ty, count: None };
        let ubuf = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None };
        let sbuf = |read_only| wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None };
        let cs = wgpu::ShaderStages::COMPUTE;
        let compute_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("light-shadow-compute-bgl"),
            entries: &[
                entry(0, cs, ubuf),
                entry(1, cs, ubuf),
                entry(2, cs, ubuf),
                entry(3, cs, sbuf(false)),
                entry(4, cs, sbuf(true)),
                entry(5, cs, sbuf(false)),
                entry(6, cs, sbuf(false)),
                entry(7, cs, sbuf(false)),
                entry(8, cs, sbuf(false)),
                entry(9, cs, sbuf(false)),
                entry(10, cs, sbuf(false)),
                entry(11, cs, sbuf(false)),
                entry(12, cs, sbuf(false)),
                entry(13, cs, sbuf(true)),
                entry(14, cs, sbuf(true)),
                entry(15, cs, sbuf(true)),
                entry(16, cs, sbuf(true)),
                entry(17, cs, sbuf(true)),
                entry(18, cs, sbuf(true)),
                entry(19, cs, sbuf(true)),
                entry(20, cs, sbuf(true)),
            ],
        });
        let args_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("light-shadow-dispatch-bgl"), entries: &[entry(0, cs, sbuf(false))] });
        let args_bg = d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("light-shadow-dispatch-bg"), layout: &args_bgl, entries: &[wgpu::BindGroupEntry { binding: 0, resource: args.as_entire_binding() }] });
        let cm = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("light_shadow.wgsl"), source: wgpu::ShaderSource::Wgsl(compute_source().into()) });
        let cl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("light-shadow-compute"), bind_group_layouts: &[Some(&compute_bgl)], immediate_size: 0 });
        // ls_select alone writes the culls' dispatch arguments (group 1).
        let sl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("light-shadow-select"), bind_group_layouts: &[Some(&compute_bgl), Some(&args_bgl)], immediate_size: 0 });
        let pipes = ["ls_rank", "ls_cluster", "ls_vehicles", "ls_select", "ls_pads", "ls_cull_city", "ls_cull_props", "ls_cull_agents", "ls_scan"]
            .iter()
            .map(|e| d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(e), layout: Some(if *e == "ls_select" { &sl } else { &cl }), module: &cm, entry_point: Some(e), compilation_options: Default::default(), cache: None }))
            .collect();

        let vs = wgpu::ShaderStages::VERTEX;
        let clip = d.features().contains(wgpu::Features::CLIP_DISTANCES);
        let draw_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("light-shadow-draw-bgl"),
            entries: &[entry(0, vs, sbuf(true)), entry(1, vs, sbuf(true)), entry(2, vs, sbuf(true)), entry(3, vs, sbuf(true)), entry(4, vs, sbuf(true))],
        });
        let clear_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("light-shadow-clear-bgl"), entries: &[entry(0, vs, sbuf(true)), entry(5, vs, sbuf(true))] });
        let clear_bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("light-shadow-clear-bg"),
            layout: &clear_bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: table.as_entire_binding() }, wgpu::BindGroupEntry { binding: 5, resource: frame.as_entire_binding() }],
        });
        let clear_pipe = depth_pipeline(d, "light-shadow-clear", &draw_source("clear", clip), "ls_clear_vs", "ls_clear_fs", &[Some(&clear_bgl)], wgpu::CompareFunction::Always);
        let city_pipe = depth_pipeline(d, "light-shadow-city", &draw_source("city", clip), "ls_city_vs", "ls_cast_fs", &[Some(&draw_bgl)], wgpu::CompareFunction::Less);
        let limit = d.limits().max_storage_buffer_binding_size as u64;
        let city_ok = r.vertex_buf.size() <= limit && r.index_buf.size() <= limit;
        if !city_ok {
            eprintln!("light shadows: the city's geometry ({} + {} B) is over a storage binding ({limit} B): its casters are left out", r.vertex_buf.size(), r.index_buf.size());
        }
        let mut gpu = Gpu {
            layout,
            atlas_draw,
            atlas_sample,
            params,
            frame,
            table,
            state,
            lists,
            args,
            args_bg,
            order,
            cand,
            recs,
            compute_bgl,
            pipes,
            draw_bgl,
            clear_bgl,
            clear_pipe,
            clear_bg,
            city_pipe,
            list_bgs: vec![None; LISTS as usize],
            props_pipe: None,
            agents_pipe: None,
            compute_bg: None,
            city_ok,
            have: (false, false),
            atlas,
        };
        if city_ok {
            for list in [0, 3] {
                gpu.list_bgs[list] = Some(gpu.list_bg(d, list as u32, &r.vertex_buf, &r.index_buf));
            }
        }
        let _ = &gpu.clear_bgl;
        gpu
    }

    /// The draw group of list `list`: the table, its block, its records, the geometry.
    fn list_bg(&self, d: &wgpu::Device, list: u32, vertices: &wgpu::Buffer, indices: &wgpu::Buffer) -> wgpu::BindGroup {
        fn range(b: &wgpu::Buffer, offset: u64, size: u64) -> wgpu::BindingResource<'_> {
            wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: b, offset, size: std::num::NonZeroU64::new(size) })
        }
        d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("light-shadow-list-bg"),
            layout: &self.draw_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.table.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: range(&self.lists, (list * LIST_WORDS) as u64 * 4, LIST_WORDS as u64 * 4) },
                wgpu::BindGroupEntry { binding: 2, resource: range(&self.recs, list as u64 * REC_CAP as u64 * 32, REC_CAP as u64 * 32) },
                wgpu::BindGroupEntry { binding: 3, resource: vertices.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: indices.as_entire_binding() },
            ],
        })
    }

    /// The props' and agents' pipelines and groups once they exist, and the
    /// compute group with whichever of them are present (dummies otherwise).
    fn ensure_casters(&mut self, d: &wgpu::Device, r: &crate::Renderer) {
        let props = r.props.get().map(|p| &p.casters);
        let agents = r.agents.get().map(|a| &a.casters);
        let clip = d.features().contains(wgpu::Features::CLIP_DISTANCES);
        if let (Some(c), None) = (props, &self.props_pipe) {
            self.props_pipe = Some(depth_pipeline(d, "light-shadow-props", &draw_source("props", clip), "ls_prop_vs", "ls_prop_fs", &[Some(&self.draw_bgl), None, Some(&c.data_bgl)], wgpu::CompareFunction::Less));
            for list in [1, 4] {
                self.list_bgs[list] = Some(self.list_bg(d, list as u32, &c.vertices, &c.indices));
            }
        }
        if let (Some(c), None) = (agents, &self.agents_pipe) {
            self.agents_pipe = Some(depth_pipeline(d, "light-shadow-agents", &draw_source("agents", clip), "ls_agent_vs", "ls_cast_fs", &[Some(&self.draw_bgl), None, Some(&c.data_bgl)], wgpu::CompareFunction::Less));
            for list in [2, 5] {
                self.list_bgs[list] = Some(self.list_bg(d, list as u32, &c.vertices, &c.indices));
            }
        }
        let want = (props.is_some(), agents.is_some());
        if self.compute_bg.as_ref().is_some_and(|(_, p, a)| (*p, *a) == want) {
            return;
        }
        let dummy = d.create_buffer(&wgpu::BufferDescriptor { label: Some("light-shadow-dummy"), size: 256, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false });
        let sky = &r.look.sky;
        fn b(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry { binding, resource: buffer.as_entire_binding() }
        }
        let (pi, pm, pme) = props.map(|c| (&c.instances, &c.models, &c.meshes)).unwrap_or((&dummy, &dummy, &dummy));
        let (ai, am, ame) = agents.map(|c| (&c.instances, &c.models, &c.meshes)).unwrap_or((&dummy, &dummy, &dummy));
        let bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("light-shadow-compute-bg"),
            layout: &self.compute_bgl,
            entries: &[
                b(0, &sky.uniform),
                b(1, &sky.lt_params),
                b(2, &self.params),
                b(3, &sky.frame_lights),
                b(4, &sky.counter),
                b(5, &sky.clusters),
                b(6, &self.order),
                b(7, &self.state),
                b(8, &self.frame),
                b(9, &self.table),
                b(10, &self.cand),
                b(11, &self.lists),
                b(12, &self.recs),
                b(13, &r.object_buf),
                b(14, &r.group_buf),
                b(15, pi),
                b(16, pm),
                b(17, pme),
                b(18, ai),
                b(19, am),
                b(20, ame),
            ],
        });
        crate::hitch::mark(crate::hitch::Mark::BindGroupCreated, 1);
        self.compute_bg = Some((bg, want.0, want.1));
    }
}

/// A depth-only pipeline into the atlas (no colour target; the fragment
/// stage writes the linear depth, or the far plane for a clear).
fn depth_pipeline(d: &wgpu::Device, label: &str, src: &str, vs: &str, fs: &str, groups: &[Option<&wgpu::BindGroupLayout>], compare: wgpu::CompareFunction) -> wgpu::RenderPipeline {
    let module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(src.into()) });
    let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(label), bind_group_layouts: groups, immediate_size: 0 });
    d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        vertex: wgpu::VertexState { module: &module, entry_point: Some(vs), compilation_options: Default::default(), buffers: &[] },
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
        depth_stencil: Some(wgpu::DepthStencilState { format: FORMAT, depth_write_enabled: Some(true), depth_compare: Some(compare), stencil: Default::default(), bias: Default::default() }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some(fs), compilation_options: Default::default(), targets: &[] }),
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::naga;

    fn validate(label: &str, src: &str) -> naga::Module {
        let module = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).unwrap_or_else(|e| panic!("{label}: {e:?}"));
        module
    }

    #[test]
    fn the_shaders_parse_and_validate() {
        let m = validate("compute", &compute_source());
        for e in ["ls_rank", "ls_cluster", "ls_vehicles", "ls_select", "ls_pads", "ls_cull_city", "ls_cull_props", "ls_cull_agents", "ls_scan"] {
            assert!(m.entry_points.iter().any(|p| p.name == e), "{e}");
        }
        for k in ["clear", "city", "props", "agents"] {
            validate(k, &draw_source(k, false));
            validate(k, &draw_source(k, true));
        }
        // the city shader with the lookup compiled in and out
        validate("city", &crate::look::city_shader_source());
        validate("far", &crate::look::city_shader_source_far());
    }

    #[test]
    fn the_layout_follows_the_options() {
        let o = |l: u8, m: bool, h: u8| LightingOptions { light_shadows: l, light_shadow_moving: m, headlight_shadows: h, ..LightingOptions::OFF };
        assert_eq!(Layout::of(&o(0, false, 0)), None);
        assert_eq!(Layout::of(&o(0, true, 0)), None, "moving casters need lamps");
        // the high tier: 16 lamps, 2 headlights: 18 tiles in 5 x 4 (2560 x 2048, 10.5 MB of atlas)
        let hi = Layout::of(&o(1, false, 2)).unwrap();
        assert_eq!((hi.lamps, hi.moving, hi.heads, hi.cols, hi.rows), (16, 0, 2, 5, 4));
        assert_eq!(hi.atlas_px(), (2560, 2048));
        // hyper: 48 lamps, 4 moving, 8 headlights: 60 tiles in 8 x 8 (32 MB of atlas)
        let hy = Layout::of(&o(3, true, 8)).unwrap();
        assert_eq!((hy.lamps, hy.moving, hy.heads, hy.cols, hy.rows), (48, 4, 8, 8, 8));
        assert_eq!(hy.atlas_px(), (4096, 4096));
        assert_eq!(Layout::of(&o(0, false, 200)).unwrap().heads, MAX_HEADLIGHTS);
        for l in [hi, hy] {
            assert!(l.cols * l.rows >= l.tiles() && (l.cols - 1) * l.rows < l.tiles());
        }
        // the redraws a frame: the budget of new maps plus the moving ones and the headlights
        assert_eq!(REDRAWS, NEW_MAPS_PER_FRAME + MOVING_LAMPS + MAX_HEADLIGHTS);
    }

    #[test]
    fn the_records_layouts_match() {
        // the structs the shaders copy (sizes as their Rust sources have them).
        assert_eq!(std::mem::size_of::<crate::props::GpuPropInstance>(), 48);
        assert_eq!(std::mem::size_of::<crate::props_lib::GpuPropModel>(), 64);
        assert_eq!(std::mem::size_of::<crate::props_lib::GpuPropMesh>(), 32);
        assert_eq!(std::mem::size_of::<eo_agents::GpuAgent>(), 64);
        assert_eq!(std::mem::size_of::<crate::scene::GpuObject>(), 64);
        assert_eq!(std::mem::size_of::<crate::sky::GpuLight>(), 64);
        let m = validate("compute", &compute_source());
        let size = |name: &str| m.types.iter().find(|(_, t)| t.name.as_deref() == Some(name)).map(|(_, t)| t.inner.size(m.to_ctx())).unwrap_or_else(|| panic!("{name}"));
        assert_eq!(size("LsPrInstance"), 48);
        assert_eq!(size("LsPrModel"), 64);
        assert_eq!(size("LsPrMesh"), 32);
        assert_eq!(size("LsAgAgent"), 64);
        assert_eq!(size("LsAgModel"), std::mem::size_of::<eo_agents::GpuModel>() as u32);
        assert_eq!(size("LsAgMesh"), std::mem::size_of::<eo_agents::GpuMesh>() as u32);
        assert_eq!(size("LsObject"), 64);
        assert_eq!(size("LsLight"), 64);
        assert_eq!(size("LsParams"), std::mem::size_of::<LsParams>() as u32);
        assert_eq!(size("LsFrame"), (frame_rows() * 16) as u32);
        assert_eq!(size("LsRec"), 32);
        // the sky's light parameters as the gather declares them
        let sky = naga::front::wgsl::parse_str(&format!("{}\n{}\nconst LT_TILE_M: f32 = 1.0;\n{}{}", include_str!("sky_common.wgsl"), crate::sky::sea_wgsl(), crate::shop_lights::wgsl() + &crate::sky::cine_lamps_wgsl(true), include_str!("sky_lights.wgsl"))).unwrap();
        let lt = sky.types.iter().find(|(_, t)| t.name.as_deref() == Some("LtParams")).map(|(_, t)| t.inner.size(sky.to_ctx())).unwrap();
        assert_eq!(size("LsLt"), lt);
    }
}
