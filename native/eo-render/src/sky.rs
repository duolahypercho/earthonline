//! the sky, the weather, the night and the camera finish on the GPU.
//!
//! The pure models live in `eo-sky` (weather schedule and ground water,
//! cloud decks and their coverage sheet, the moon, the practical clock,
//! exposure, photometric references, lamp records). This module owns the
//! passes and the resources, all on the GPU-driven path (no per-object or
//! per-light CPU work per frame):
//!
//! 1. **Lights** (compute, before the city pass): the street lamps near the
//!    camera and two low beams per lit vehicle (read straight from the agent
//!    instances) appended into one frame list, then assigned to a 16 x 9 x 24
//!    froxel grid that the city shader loops over (`sky_lights.wgsl`).
//! 2. In the **city pass** (`sky_light.wgsl`, group 1 of the look): cloud
//!    shadows on the key, the clustered lights, wet ground with puddles and
//!    rain rings, the screen-space reflection of the previous frame on glossy
//!    ground.
//! 3. **FX** (render, after the occlusion pass): the luminaires and rain
//!    streaks into the HDR scene; the moving agents' velocity.
//! 4. **Volume** (compute, half resolution): sun shafts in the haze, the
//!    marine layer, the low cloud deck (marched) and the high deck.
//! 5. **Resolve** (render): the volume composited by depth, temporal
//!    anti-aliasing with motion vectors, into the history the post reads.
//! 6. **Bloom** (compute): bright pass and a five-level pyramid.
//!
//! Every stage has a switch (`SkyOptions`), and the default must hold the
//! frame budget.

use crate::camera::Camera;
use bytemuck::{Pod, Zeroable};
use eo_sky::lights::{LampKind, LampLight};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Timestamp slots (after the renderer's 22).
/// (after the water slice's 18-21)
pub const Q_SKY_LIGHTS: u32 = 22;
pub const Q_SKY_FX: u32 = 24;
pub const Q_SKY_VOLUME: u32 = 26;
pub const Q_SKY_RESOLVE: u32 = 28;
pub const Q_SKY_BLOOM: u32 = 30;
/// The cloud-shadow map's pass (and the noise bake on the first frame).
pub const Q_CLOUD_SHADOW: u32 = 32;

pub const LAMP_CAP: u32 = 65_536;
pub const FRAME_LIGHT_CAP: u32 = 1024;
/// Luminaires drawn a frame (near and far; sky_lights.wgsl LT_SPRITE_CAP).
pub const SPRITE_CAP: u32 = 32_768;
pub const CLUSTERS: [u32; 3] = [16, 9, 24];
pub const CLUSTER_CAP: u32 = 32;
/// Lights are shaded within this view depth (m).
pub const CLUSTER_FAR_M: f32 = 300.0;
pub const CLUSTER_NEAR_M: f32 = 0.5;
pub const BLOOM_LEVELS: usize = 5;
pub const NOISE_RES: u32 = 64;
pub const NOISE_MIPS: u32 = 4;
const SHEET_RES: u32 = eo_sky::clouds::SHEET_RES as u32;
pub const VEL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;
/// A velocity texel that says "no object motion here: reproject by the camera".
pub const VEL_NONE: f64 = 65000.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WeatherMode {
    Preset(eo_sky::Kind),
    /// The shared world schedule (eo-sky `weather::state_at`) at the world clock.
    Schedule,
}

/// The sky slice's switches (`--no-clouds`, ...) and quality.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyOptions {
    pub clouds: bool,
    pub cloud_shadows: bool,
    pub fog: bool,
    pub shafts: bool,
    pub moon_stars: bool,
    pub lights: bool,
    pub sprites: bool,
    pub wet: bool,
    pub rain: bool,
    pub ssr: bool,
    pub taa: bool,
    pub bloom: bool,
    pub adaptation: bool,
    /// the camera meter on the exposure (`--no-meter`).
    pub meter: bool,
    /// the sky from the atmosphere tables (`--no-atmosphere`: the JS analytic dome).
    pub atmosphere: bool,
    /// Contact shadows toward the key (`--no-contact-shadows`).
    pub contact: bool,
    /// The fair-weather cumulus field, off by default (a clear sky keeps the high sheet
    /// alone; the overcast, fog and rain decks are drawn either way); `EO_CUMULUS=1` draws it.
    pub cumulus: bool,
    /// The physical daylight (`--no-phys-sky`): the sun, the sky and the ground's light from
    /// the clear coastal atmosphere (eo-look `atmosphere`, `phys`), the sky tables drawn at
    /// the sun's own scale and the city's reflections read from them, in place of the
    /// browser game's rig, aerosol and dome anchor.
    pub phys_sky: bool,
    /// The aerial-perspective volume (`--no-ap-lut`): the in-scattered light and
    /// transmittance of the atmosphere's own medium over the view distance, from a
    /// precomputed table, in place of the haze toward the horizon's colour.
    pub ap_lut: bool,
    /// The filmic display transform (`--no-filmic`): eo-look `tone::display_filmic`
    /// in place of the ACES fit with the post chain's shoulder, grade and finish.
    pub filmic: bool,
    /// The incident meter's exposure in EV100 while the sun is up (`--no-ev100`: the JS curve).
    pub ev100: bool,
    /// The canyon-scale sky visibility and wall bounce of the occlusion pass (`--no-sky-vis`:
    /// the 4 m contact term alone).
    pub sky_vis: bool,
    /// The night lit by its sky and its practicals alone (`--no-night-shaping`: the rig's rim
    /// fill stays on at night; eo-sky `frame::apply_night_shaping`).
    pub night_shaping: bool,
    pub weather: WeatherMode,
    pub seed: u32,
    /// The world clock at the first frame (seconds since the world's epoch).
    pub world_time_s: f64,
    /// 0 low, 1 default, 2 high (cloud steps 12 / 18 / 28).
    pub quality: u32,
    pub moon_day: f64,
}

impl Default for SkyOptions {
    fn default() -> Self {
        Self {
            clouds: true,
            cloud_shadows: true,
            fog: true,
            shafts: true,
            moon_stars: true,
            lights: true,
            sprites: true,
            wet: true,
            rain: true,
            ssr: true,
            taa: true,
            bloom: true,
            adaptation: true,
            meter: true,
            atmosphere: true,
            contact: true,
            cumulus: std::env::var("EO_CUMULUS").is_ok_and(|v| v == "1"),
            phys_sky: true,
            ap_lut: true,
            filmic: true,
            ev100: true,
            sky_vis: true,
            night_shaping: true,
            weather: WeatherMode::Preset(eo_sky::Kind::Clear),
            seed: eo_sky::world_seed(),
            world_time_s: 0.0,
            quality: 1,
            moon_day: eo_sky::moon::DEFAULT_MOON_DAY,
        }
    }
}

impl SkyOptions {
    /// Every stage off (the renderer before this slice, but the weather's
    /// look terms still apply when a weather is asked for).
    pub fn off() -> Self {
        Self {
            clouds: false,
            cloud_shadows: false,
            fog: false,
            shafts: false,
            moon_stars: false,
            lights: false,
            sprites: false,
            wet: false,
            rain: false,
            ssr: false,
            taa: false,
            bloom: false,
            adaptation: false,
            meter: false,
            atmosphere: false,
            contact: false,
            cumulus: false,
            phys_sky: false,
            ap_lut: false,
            filmic: false,
            ev100: false,
            sky_vis: false,
            night_shaping: false,
            ..Self::default()
        }
    }
    /// Whether any stage after the city pass runs (else post reads the HDR scene).
    pub fn any_post(&self) -> bool {
        self.clouds || self.fog || self.shafts || self.taa || self.bloom || self.rain || self.sprites
    }
    pub fn needs_volume(&self) -> bool {
        self.clouds || self.fog || self.shafts
    }
    fn bits(&self) -> u32 {
        let mut b = 0;
        for (on, bit) in [
            (self.clouds, 1),
            (self.cloud_shadows, 2),
            (self.fog, 4),
            (self.shafts, 8),
            (self.moon_stars, 16),
            (self.lights, 32),
            (self.wet, 64),
            (self.rain, 128),
            (self.ssr, 256),
            (self.taa, 512),
            (self.bloom, 1024),
            (self.sprites, 4096),
            (self.contact, 8192),
            (self.cumulus, 16384), // SKY_CUMULUS
        ] {
            if on {
                b |= bit;
            }
        }
        b
    }
    fn cloud_steps(&self) -> f32 {
        match self.quality {
            0 => 12.0,
            2 => 28.0,
            _ => 18.0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct SkyUniform {
    vp: [[f32; 4]; 4],
    prev_vp: [[f32; 4]; 4],
    eye: [f32; 4],
    prev_eye: [f32; 4],
    jitter: [f32; 4],
    taa: [f32; 4],
    res: [f32; 4],
    cloud0: [f32; 4],
    cloud1: [f32; 4],
    cloud2: [f32; 4],
    cloud3: [f32; 4],
    cloud4: [f32; 4],
    sun_rad: [f32; 4],
    amb_top: [f32; 4],
    amb_base: [f32; 4],
    wet: [f32; 4],
    rain: [f32; 4],
    wind: [f32; 4],
    lights: [u32; 4],
    cluster: [f32; 4],
    cam_fwd: [f32; 4],
    cam_right: [f32; 4],
    cam_up: [f32; 4],
    night: [f32; 4],
    moon: [f32; 4],
    stars: [f32; 4],
    bloom: [f32; 4],
    bloom_w: [f32; 4],
    bloom_w2: [f32; 4],
    flags: [u32; 4],
    // x the weather's mean direct-beam transmission (the sun every pass
    // takes under a continuous deck), y the volume key's scale, z,w unused
    beam: [f32; 4],
    // the twilight light on the clouds from the sky toward the set sun (rgb), w unused
    twi: [f32; 4],
    // the atmosphere tables (sky_atmo.wgsl): toward the sun, w the sky scale
    atmo: [f32; 4],
    // on, share of the sky, aerosol density x, the clouds' sun scale
    atmo2: [f32; 4],
    // aerial perspective: x the weather veils (1/0: its visibility under clear air's), y clear air's aerosol extinction at the ground (1/m), z, w the weather's range-fog near / far (m)
    atmo3: [f32; 4],
    sun_glint: [f32; 4],
    // the physical daylight: rgb a white sun at the top of the atmosphere x the Sun's colour
    // (look units; the sky tables are per unit solar illuminance), w its weight (0: the dome anchor)
    atmo4: [f32; 4],
    // the medium the tables integrate: x aerosol scattering, y aerosol absorption (1/km at the
    // ground, 550 nm, x the haze density), z the aerosol's scale height (km), w the
    // aerial-perspective volume on (1/0)
    atmo_m: [f32; 4],
    // rgb the aerosol's spectral factor per channel, w the aerial-perspective volume's reach (km)
    atmo_s: [f32; 4],
    // rgb Rayleigh scattering at sea level (1/km), w its scale height (km)
    atmo_r: [f32; 4],
    // rgb ozone's peak absorption (1/km), w unused
    atmo_o: [f32; 4],
}

/// The aerial-perspective volume: texels (azimuth about the sun, elevation, distance) and reach.
pub const AP_SIZE: [u32; 3] = [32, 32, 32];
pub const AP_REACH_KM: f32 = 40.0;

/// One light as the GPU reads it (sky_light.wgsl `SkyLight`), 64 bytes.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, Default)]
pub struct GpuLight {
    pub pos: [f32; 3],
    /// Candela for the static lamps (scaled to irradiance units per frame), units on the frame list.
    pub intensity: f32,
    pub color: [f32; 3],
    pub range: f32,
    pub dir: [f32; 3],
    pub cos_outer: f32,
    pub cos_inner: f32,
    pub kind: u32,
    pub size: f32,
    pub pad: f32,
}
const _: () = assert!(std::mem::size_of::<GpuLight>() == 64);

impl GpuLight {
    pub fn from_lamp(l: &LampLight) -> Self {
        let f = |v: [f64; 3]| v.map(|x| x as f32);
        Self {
            pos: f(l.pos),
            intensity: l.candela as f32,
            color: f(l.color),
            range: l.range as f32,
            dir: f(l.dir),
            cos_outer: l.cos_outer as f32,
            cos_inner: l.cos_inner as f32,
            kind: match l.kind {
                LampKind::Street => 0,
                LampKind::Headlight => 1,
                LampKind::Point => 2,
            },
            size: 0.0,
            pad: 0.0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LtParams {
    counts: [u32; 4],
    scale: [f32; 4],
    /// props lamps: units per JS candela (lamps on), any props tile resident (1/0).
    props: [f32; 4],
    /// The waterside cells: the grid corner x, z, the cell (m); columns, rows.
    water: [f32; 4],
    water_n: [u32; 4],
}

/// The mask words: the props tiles resident (STREET_TILE_M keys -128..127 on
/// each axis), then the waterside cells (up to `WATER_CELLS_MAX` squared).
const TILE_WORDS: usize = 2048;
pub const WATER_CELLS_MAX: usize = 512;
const MASK_WORDS: usize = TILE_WORDS + WATER_CELLS_MAX * WATER_CELLS_MAX / 32;

struct Targets {
    size: (u32, u32),
    /// The volume and its view distance, one pair per history parity (the
    /// pass reprojects the other pair: temporal accumulation).
    vol: [wgpu::TextureView; 2],
    volz: [wgpu::TextureView; 2],
    /// the reconstructed full-resolution volume and its distance, per parity.
    full: [wgpu::TextureView; 2],
    fullz: [wgpu::TextureView; 2],
    hist: [wgpu::TextureView; 2],
    vel: wgpu::TextureView,
    _mips: Vec<wgpu::TextureView>,
    ups: Vec<wgpu::TextureView>,
    /// Bind groups that depend on the scene's depth and HDR textures.
    scene: Option<(wgpu::Texture, wgpu::Texture, SceneBinds)>,
    bloom_bgs: [Vec<wgpu::BindGroup>; 2],
    bytes: u64,
}

struct SceneBinds {
    /// volume[p] writes volume p and reprojects volume 1 - p.
    volume: [wgpu::BindGroup; 2],
    /// recon[p] writes full p from volume p and full 1 - p.
    recon: [wgpu::BindGroup; 2],
    resolve: [wgpu::BindGroup; 2],
}

struct State {
    opts: SkyOptions,
    frame: u32,
    world_s: f64,
    dt: f64,
    prev_vp: Option<glam::Mat4>,
    prev_eye: glam::Vec3,
    jitter: [f32; 2],
    prev_jitter: [f32; 2],
    history_valid: bool,
    /// Which history texture the coming resolve writes.
    parity: usize,
    adapt: eo_sky::night::Adaptation,
    /// the forward the meter last metered (a turn of more than ~30 deg snaps the adaptation, as a cut does).
    meter_fwd: Option<glam::Vec3>,
    /// the aerosol density the transmittance and multiple-scattering
    /// tables were built for (they depend on the medium only; rebuilt on a change).
    atmo_built: Option<[[f32; 4]; 4]>,
    atmo_haze: f32,
    /// the medium the tables are built for (`atmo_m`, `atmo_s`, `atmo_r`, `atmo_o`), the rebuild key.
    atmo_medium: [[f32; 4]; 4],
    /// the (hour, weather) the histories were made under; frames left to discard after a change; frames of history since the last cut.
    hist_look: Option<(f64, (i64, i64, i64))>,
    recut: u32,
    /// a cut seen by `update` holds until a frame is encoded: a loop that
    /// updates many times before it draws (a capture's settle) used up the cut and
    /// the recut frames without drawing them, and the next card kept the last
    /// card's sky (its sun, its pink horizon, its clouds) in the histories.
    cut_hold: bool,
    hist_n: u32,
    terms: Option<eo_sky::SkyTerms>,
    exposure: f64,
    agents_bg: Option<(u32, wgpu::BindGroup)>,
    /// The depth texture usable by the passes after the city (single-sample, bindable).
    depth_ok: bool,
    weather_cache: Option<(f64, eo_sky::WeatherState)>,
    /// The props slice's lamp lights and their count (bound with the lights), and the light bind group without agents.
    props: Option<(wgpu::Buffer, wgpu::Buffer)>,
    lights_bg0: Option<wgpu::BindGroup>,
    /// The props tiles resident (their mask words as last written) and the waterside grid.
    tiles: Vec<u32>,
    water: ([f32; 4], [u32; 4]),
}

pub struct SkyGpu {
    device: wgpu::Device,
    pub(crate) uniform: wgpu::Buffer,
    sheet: wgpu::Texture,
    pub(crate) sheet_view: wgpu::TextureView,
    sheet_ready: AtomicBool,
    _noise: wgpu::Texture,
    noise_view: wgpu::TextureView,
    // the atmosphere tables (transmittance, multiple scattering, sky view) and their passes
    atmo_views: [wgpu::TextureView; 3],
    atmo_pipes: [wgpu::ComputePipeline; 3],
    atmo_bgs: [wgpu::BindGroup; 3],
    atmo_sizes: [(u32, u32); 3],
    /// the aerial-perspective volume (in-scattered light, transmittance) and its pass
    ap_views: [wgpu::TextureView; 2],
    ap_pipe: wgpu::ComputePipeline,
    ap_bg: wgpu::BindGroup,
    noise_ready: AtomicBool,
    /// the cloud noise, the blue-noise tile and the cloud-shadow map (clouds.rs).
    pub(crate) clouds: crate::clouds::CloudGpu,
    pub(crate) rep: wgpu::Sampler,
    lin: wgpu::Sampler,
    lamps: wgpu::Buffer,
    lamp_count: AtomicU32,
    /// Of them, by the water.
    waterside: AtomicU32,
    pub(crate) frame_lights: wgpu::Buffer,
    sprites: wgpu::Buffer,
    counter: wgpu::Buffer,
    pub(crate) clusters: wgpu::Buffer,
    lt_params: wgpu::Buffer,
    /// sky_lights.wgsl `lt_mask` (`MASK_WORDS`).
    masks: wgpu::Buffer,
    dummy_storage: wgpu::Buffer,
    lights_bgl: wgpu::BindGroupLayout,
    lights_pipes: [wgpu::ComputePipeline; 4],
    noise_pipe: wgpu::ComputePipeline,
    noise_bg: wgpu::BindGroup,
    noise_down: wgpu::ComputePipeline,
    noise_down_bgs: Vec<wgpu::BindGroup>,
    volume_bgl: wgpu::BindGroupLayout,
    volume_pipe: wgpu::ComputePipeline,
    /// the volume's full-resolution reconstruction (sky_recon.wgsl).
    recon_bgl: wgpu::BindGroupLayout,
    recon_pipe: wgpu::ComputePipeline,
    resolve_bgl: wgpu::BindGroupLayout,
    resolve_pipe: wgpu::RenderPipeline,
    bloom_bgl: wgpu::BindGroupLayout,
    bloom_pipes: [wgpu::ComputePipeline; 3],
    bloom_levels: Vec<wgpu::Buffer>,
    fx_bg: wgpu::BindGroup,
    sprite_pipe: wgpu::RenderPipeline,
    /// The waterside luminaires' glitter in the sea (sky_fx.wgsl `glitter_*`).
    glitter_pipe: wgpu::RenderPipeline,
    rain_pipe: wgpu::RenderPipeline,
    /// A 1 x 1 stand-in for the previous frame before there is one.
    pub(crate) dummy_hist: wgpu::TextureView,
    targets: Mutex<Option<Targets>>,
    state: Mutex<State>,
    pub bytes: u64,
}

fn tex(device: &wgpu::Device, label: &str, size: (u32, u32), format: wgpu::TextureFormat, usage: wgpu::TextureUsages) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: size.0.max(1), height: size.1.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

/// The sea the sky passes meet (eo-water): its level, its facets' mean square
/// slope at the water slice's reference wind (Cox & Munk 1954 as eo-water's
/// wind field has it), and where a view ray's air ends over it.
pub fn sea_wgsl() -> String {
    let mss = eo_water::surface::wind_field(eo_water::surface::REFERENCE_WIND).mss;
    format!(
        "\n// generated by sky.rs sea_wgsl (eo-water)\nconst SKY_SEA_Y: f32 = {:.6};\nconst SKY_SEA_MSS: f32 = {:.6};\n\
         fn sky_air_end(eye_y: f32, dir_y: f32, dist: f32) -> f32 {{\n    if (dir_y < -1e-4 && eye_y > SKY_SEA_Y) {{\n        return min(dist, (SKY_SEA_Y - eye_y) / dir_y);\n    }}\n    return dist;\n}}\n",
        eo_water::SEA_LEVEL_Y,
        mss
    )
}

/// Halton (2, 3), 8 phases, centred: the sub-pixel jitter sequence.
fn halton(i: u32, b: u32) -> f32 {
    let mut f = 1.0f32;
    let mut r = 0.0f32;
    let mut i = i;
    while i > 0 {
        f /= b as f32;
        r += f * (i % b) as f32;
        i /= b;
    }
    r
}

pub struct SkyInit<'a> {
    pub look_buf: &'a wgpu::Buffer,
    pub hdr_format: wgpu::TextureFormat,
    pub depth_format: wgpu::TextureFormat,
    pub shadow_array: &'a wgpu::TextureView,
    pub cmp_sampler: &'a wgpu::Sampler,
}

impl SkyGpu {
    pub fn new(device: &wgpu::Device, init: SkyInit) -> Self {
        let d = device;
        let common = crate::look::look_common_source();
        let buffer = |label: &str, size: u64, usage: wgpu::BufferUsages| d.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size.max(16).next_multiple_of(16), usage, mapped_at_creation: false });
        let uni = wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST;
        let st = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        let uniform = buffer("sky-uniform", std::mem::size_of::<SkyUniform>() as u64, uni);
        let lamps = buffer("sky-lamps", LAMP_CAP as u64 * 64, st);
        let frame_lights = buffer("sky-frame-lights", FRAME_LIGHT_CAP as u64 * 64, wgpu::BufferUsages::STORAGE);
        let sprites = buffer("sky-sprites", SPRITE_CAP as u64 * 64, wgpu::BufferUsages::STORAGE);
        let counter = buffer("sky-light-counter", 32, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST);
        let ncl = (CLUSTERS[0] * CLUSTERS[1] * CLUSTERS[2]) as u64;
        let clusters = buffer("sky-clusters", ncl * (1 + CLUSTER_CAP as u64) * 4, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
        let lt_params = buffer("sky-light-params", std::mem::size_of::<LtParams>() as u64, uni);
        let dummy_storage = buffer("sky-dummy-storage", 256, wgpu::BufferUsages::STORAGE);
        let masks = buffer("sky-light-masks", MASK_WORDS as u64 * 4, st);
        let sheet = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("sky-cloud-sheet"),
            size: wgpu::Extent3d { width: SHEET_RES, height: SHEET_RES, depth_or_array_layers: 1 },
            mip_level_count: SHEET_RES.ilog2() + 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let sheet_view = sheet.create_view(&Default::default());
        let noise = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("sky-cloud-noise"),
            size: wgpu::Extent3d { width: NOISE_RES, height: NOISE_RES, depth_or_array_layers: NOISE_RES },
            // Mips 64..8 (box filtered once): far samples read the level of their footprint.
            mip_level_count: NOISE_MIPS,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        });
        let noise_view = noise.create_view(&Default::default());
        let noise_mip = |m: u32| noise.create_view(&wgpu::TextureViewDescriptor { base_mip_level: m, mip_level_count: Some(1), ..Default::default() });
        let rep = d.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sky-repeat"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let lin = d.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sky-linear-clamp"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let entry = |binding, visibility, ty| wgpu::BindGroupLayoutEntry { binding, visibility, ty, count: None };
        let cs = wgpu::ShaderStages::COMPUTE;
        let fs = wgpu::ShaderStages::FRAGMENT;
        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let ubuf = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None };
        let sbuf = |read_only| wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None };
        let t2 = |filterable| wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false };
        let depth2 = wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false };
        let store = |format| wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format, view_dimension: wgpu::TextureViewDimension::D2 };
        let samp = wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering);
        let module = |label: &str, src: String| d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(src.into()) });
        let layout = |label: &str, bgl: &wgpu::BindGroupLayout| d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(label), bind_group_layouts: &[Some(bgl)], immediate_size: 0 });
        let compute = |label: &str, l: &wgpu::PipelineLayout, m: &wgpu::ShaderModule, e: &str| {
            d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(label), layout: Some(l), module: m, entry_point: Some(e), compilation_options: Default::default(), cache: None })
        };

        // Lights.
        let lights_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky-lights-bgl"),
            entries: &[entry(0, cs, ubuf), entry(1, cs, sbuf(true)), entry(2, cs, sbuf(false)), entry(3, cs, sbuf(false)), entry(4, cs, sbuf(false)), entry(5, cs, sbuf(true)), entry(6, cs, sbuf(true)), entry(7, cs, ubuf), entry(8, cs, sbuf(false)), entry(9, cs, sbuf(true)), entry(10, cs, sbuf(true)), entry(11, cs, sbuf(true))],
        });
        let sky_only = include_str!("sky_common.wgsl");
        let lm = module("sky_lights.wgsl", format!("{sky_only}\n{}\nconst LT_TILE_M: f32 = {:.1};\n{}", sea_wgsl(), eo_street::STREET_TILE_M, include_str!("sky_lights.wgsl")));
        let ll = layout("sky-lights-layout", &lights_bgl);
        let lights_pipes = [
            compute("lights_lamps", &ll, &lm, "lights_lamps"),
            compute("lights_agents", &ll, &lm, "lights_agents"),
            compute("lights_cluster", &ll, &lm, "lights_cluster"),
            compute("lights_props", &ll, &lm, "lights_props"),
        ];

        // Cloud noise (once).
        let (vol_src, noise_src) = include_str!("sky_volume.wgsl").split_once("// ---- noise module").expect("noise marker");
        let noise_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky-noise-bgl"),
            entries: &[entry(0, cs, wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: wgpu::TextureFormat::Rgba8Unorm, view_dimension: wgpu::TextureViewDimension::D3 })],
        });
        let nm = module("sky_noise", format!("// ---- noise module{noise_src}"));
        let noise_pipe = compute("sky_noise", &layout("sky-noise-layout", &noise_bgl), &nm, "noise_main");
        let noise_bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky-noise-bg"),
            layout: &noise_bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&noise_mip(0)) }],
        });
        // Its mips: each level the 2 x 2 x 2 mean of the one above.
        let down_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky-noise-down-bgl"),
            entries: &[
                entry(0, cs, wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: wgpu::TextureFormat::Rgba8Unorm, view_dimension: wgpu::TextureViewDimension::D3 }),
                entry(1, cs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false }),
            ],
        });
        let noise_down = compute("sky_noise_down", &layout("sky-noise-down-layout", &down_bgl), &nm, "noise_down");
        let noise_down_bgs: Vec<wgpu::BindGroup> = (1..NOISE_MIPS)
            .map(|m| {
                d.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("sky-noise-down-bg"),
                    layout: &down_bgl,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&noise_mip(m)) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&noise_mip(m - 1)) },
                    ],
                })
            })
            .collect();

        // Volume.
        let volume_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky-volume-bgl"),
            entries: &[
                entry(0, cs, ubuf),
                entry(1, cs, ubuf),
                entry(2, cs, depth2),
                entry(3, cs, t2(true)),
                entry(4, cs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false }),
                entry(5, cs, samp),
                entry(6, cs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false }),
                entry(7, cs, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison)),
                entry(8, cs, store(wgpu::TextureFormat::Rgba16Float)),
                entry(9, cs, store(wgpu::TextureFormat::R32Float)),
                // The other parity's volume and distance (temporal reprojection), a clamped sampler.
                entry(10, cs, t2(true)),
                entry(11, cs, t2(false)),
                entry(12, cs, samp),
                // the atmosphere's transmittance and sky-view tables.
                entry(13, cs, t2(true)),
                entry(14, cs, t2(true)),
                // the base and detail noise, the curl field, the cloud-shadow map, the blue noise
                entry(15, cs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false }),
                entry(16, cs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false }),
                entry(17, cs, t2(true)),
                entry(18, cs, t2(true)),
                entry(19, cs, t2(true)),
                // the aerial-perspective volume: in-scattered light, transmittance
                entry(20, cs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false }),
                entry(21, cs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false }),
            ],
        });
        let atmo_src = include_str!("sky_atmo.wgsl");
        let vm = module("sky_volume.wgsl", format!("{common}\n{vol_src}\n{}", include_str!("sky_clouds.wgsl"))); // + the cloud density and light
        // the atmosphere tables (Hillaire 2020), rebuilt each frame (the sun moves, the eye climbs).
        let atmo_sizes = [(256u32, 64u32), (32, 32), (192, 108)];
        let atmo_tex: Vec<wgpu::Texture> = ["sky-atmo-transmittance", "sky-atmo-multi", "sky-atmo-view"]
            .iter()
            .zip(atmo_sizes.iter())
            .map(|(l, s)| {
                d.create_texture(&wgpu::TextureDescriptor {
                    label: Some(l),
                    size: wgpu::Extent3d { width: s.0, height: s.1, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba16Float,
                    usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
            })
            .collect();
        let atmo_views: [wgpu::TextureView; 3] = std::array::from_fn(|i| atmo_tex[i].create_view(&Default::default()));
        let atmo_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky-atmo-bgl"),
            entries: &[entry(0, cs, ubuf), entry(1, cs, store(wgpu::TextureFormat::Rgba16Float)), entry(2, cs, t2(true)), entry(3, cs, t2(true)), entry(4, cs, samp)],
        });
        let am = module("sky_atmo_luts.wgsl", format!("{sky_only}\n{atmo_src}\n{}", include_str!("sky_atmo_luts.wgsl")));
        let al = layout("sky-atmo-layout", &atmo_bgl);
        let atmo_pipes = [compute("sky_atmo_trans", &al, &am, "at_trans_main"), compute("sky_atmo_ms", &al, &am, "at_ms_main"), compute("sky_atmo_view", &al, &am, "at_view_main")];
        // Each pass reads the tables the others wrote, never the one it writes (a 1 x 1 placeholder there).
        let dummy = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("sky-atmo-dummy"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let dummy_view = dummy.create_view(&Default::default());
        let atmo_bg = |out: usize, tr: &wgpu::TextureView, ms: &wgpu::TextureView| {
            d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("sky-atmo-bg"),
                layout: &atmo_bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&atmo_views[out]) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(tr) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(ms) },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&lin) },
                ],
            })
        };
        let atmo_bgs = [atmo_bg(0, &dummy_view, &dummy_view), atmo_bg(1, &atmo_views[0], &dummy_view), atmo_bg(2, &atmo_views[0], &atmo_views[1])];
        // the aerial-perspective volume (sky_atmo_luts.wgsl `at_ap_main`): two 3D tables, written
        // after the sky view from the same transmittance and multiple-scattering tables.
        let ap_tex: Vec<wgpu::Texture> = ["sky-ap-inscatter", "sky-ap-transmittance"]
            .iter()
            .map(|l| {
                d.create_texture(&wgpu::TextureDescriptor {
                    label: Some(l),
                    size: wgpu::Extent3d { width: AP_SIZE[0], height: AP_SIZE[1], depth_or_array_layers: AP_SIZE[2] },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D3,
                    format: wgpu::TextureFormat::Rgba16Float,
                    usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
            })
            .collect();
        let ap_views: [wgpu::TextureView; 2] = std::array::from_fn(|i| ap_tex[i].create_view(&Default::default()));
        let store3 = wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: wgpu::TextureFormat::Rgba16Float, view_dimension: wgpu::TextureViewDimension::D3 };
        let ap_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky-ap-bgl"),
            entries: &[entry(0, cs, ubuf), entry(2, cs, t2(true)), entry(3, cs, t2(true)), entry(4, cs, samp), entry(5, cs, store3), entry(6, cs, store3)],
        });
        let ap_bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky-ap-bg"),
            layout: &ap_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&atmo_views[0]) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&atmo_views[1]) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&lin) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&ap_views[0]) },
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&ap_views[1]) },
            ],
        });
        let ap_pipe = compute("sky_atmo_ap", &layout("sky-ap-layout", &ap_bgl), &am, "at_ap_main");
        let volume_pipe = compute("sky_volume", &layout("sky-volume-layout", &volume_bgl), &vm, "vol_main");
        // the cloud noise, blue noise and cloud-shadow map (the shadow pass is the volume module's `cl_shadow_main`)
        let clouds = crate::clouds::CloudGpu::new(d, crate::clouds::ShadowInputs { module: &vm, look_buf: init.look_buf, sky_uniform: &uniform, sheet: &sheet_view, repeat: &rep, sky_view: &atmo_views[2], linear: &lin });
        // the reconstruction: this frame's traced quarter, the last frame's full-resolution volume, out.
        let recon_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky-recon-bgl"),
            entries: &[
                entry(0, cs, ubuf),
                entry(1, cs, ubuf),
                entry(2, cs, depth2),
                entry(3, cs, t2(false)),
                entry(4, cs, t2(false)),
                entry(5, cs, t2(true)),
                entry(6, cs, t2(false)),
                entry(7, cs, store(wgpu::TextureFormat::Rgba16Float)),
                entry(8, cs, store(wgpu::TextureFormat::R32Float)),
                entry(9, cs, samp),
            ],
        });
        let rcm = module("sky_recon.wgsl", format!("{common}\n{}", include_str!("sky_recon.wgsl")));
        let recon_pipe = compute("sky_recon", &layout("sky-recon-layout", &recon_bgl), &rcm, "rc_main");

        // Resolve.
        let resolve_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky-resolve-bgl"),
            entries: &[
                entry(0, fs, ubuf),
                entry(1, fs, ubuf),
                entry(2, fs, t2(false)),
                entry(3, fs, depth2),
                entry(4, fs, t2(false)),
                entry(5, fs, t2(false)),
                entry(6, fs, t2(true)),
                entry(7, fs, t2(false)),
                entry(8, fs, samp),
                // the atmosphere's transmittance table (the sun's disc colour)
                entry(9, fs, t2(true)),
            ],
        });
        let rm = module("sky_resolve.wgsl", format!("{common}\n{}", include_str!("sky_resolve.wgsl")));
        let resolve_pipe = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky-resolve"),
            layout: Some(&layout("sky-resolve-layout", &resolve_bgl)),
            vertex: wgpu::VertexState { module: &rm, entry_point: Some("resolve_vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &rm,
                entry_point: Some("resolve_fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: init.hdr_format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });

        // Bloom.
        let bloom_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky-bloom-bgl"),
            entries: &[entry(0, cs, ubuf), entry(1, cs, t2(true)), entry(2, cs, t2(true)), entry(3, cs, samp), entry(4, cs, store(wgpu::TextureFormat::Rgba16Float)), entry(5, cs, ubuf)],
        });
        let bm = module("sky_bloom.wgsl", format!("{sky_only}\n{}", include_str!("sky_bloom.wgsl")));
        let bl = layout("sky-bloom-layout", &bloom_bgl);
        let bloom_pipes = [compute("bloom_prefilter", &bl, &bm, "bloom_prefilter"), compute("bloom_down", &bl, &bm, "bloom_down"), compute("bloom_up", &bl, &bm, "bloom_up")];
        let bloom_levels = (0..BLOOM_LEVELS).map(|_| buffer("sky-bloom-level", 16, uni)).collect();

        // FX: luminaires and rain.
        let fx_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky-fx-bgl"),
            entries: &[entry(0, vf, ubuf), entry(1, vf, ubuf), entry(2, wgpu::ShaderStages::VERTEX, sbuf(true))],
        });
        let fx_bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky-fx-bg"),
            layout: &fx_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: init.look_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: sprites.as_entire_binding() },
            ],
        });
        let fm = module("sky_fx.wgsl", format!("{common}\n{}", include_str!("sky_fx.wgsl")));
        let fl = layout("sky-fx-layout", &fx_bgl);
        let fx_pipe = |label: &str, vs: &str, fsn: &str, blend: wgpu::BlendState| {
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&fl),
                vertex: wgpu::VertexState { module: &fm, entry_point: Some(vs), compilation_options: Default::default(), buffers: &[] },
                primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: init.depth_format,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &fm,
                    entry_point: Some(fsn),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: init.hdr_format, blend: Some(blend), write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let add = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
        let sprite_pipe = fx_pipe("sky-sprites", "sprite_vs", "sprite_fs", wgpu::BlendState { color: add, alpha: add });
        let glitter_pipe = fx_pipe("sky-glitter", "glitter_vs", "glitter_fs", wgpu::BlendState { color: add, alpha: add });
        let pre = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add };
        let rain_pipe = fx_pipe("sky-rain", "rain_vs", "rain_fs", wgpu::BlendState { color: pre, alpha: pre });
        let dummy_hist = tex(d, "sky-hist-stand-in", (1, 1), init.hdr_format, wgpu::TextureUsages::TEXTURE_BINDING);
        let bytes = uniform.size() + lamps.size() + frame_lights.size() + sprites.size() + counter.size() + clusters.size() + (SHEET_RES as u64).pow(2) * 4 * 4 / 3 + (NOISE_RES as u64).pow(3) * 4 * 8 / 7;
        let _ = (init.shadow_array, init.cmp_sampler);
        let bytes = bytes + masks.size() + clouds.bytes; // clouds
        Self {
            device: d.clone(),
            uniform,
            sheet,
            sheet_view,
            sheet_ready: AtomicBool::new(false),
            _noise: noise,
            noise_view,
            noise_ready: AtomicBool::new(false),
            clouds,
            rep,
            lin,
            lamps,
            lamp_count: AtomicU32::new(0),
            waterside: AtomicU32::new(0),
            frame_lights,
            sprites,
            counter,
            clusters,
            lt_params,
            masks,
            dummy_storage,
            lights_bgl,
            lights_pipes,
            noise_pipe,
            noise_bg,
            noise_down,
            noise_down_bgs,
            volume_bgl,
            volume_pipe,
            recon_bgl,
            recon_pipe,
            atmo_views,
            atmo_pipes,
            atmo_bgs,
            atmo_sizes,
            ap_views,
            ap_pipe,
            ap_bg,
            resolve_bgl,
            resolve_pipe,
            bloom_bgl,
            bloom_pipes,
            bloom_levels,
            fx_bg,
            sprite_pipe,
            glitter_pipe,
            rain_pipe,
            dummy_hist,
            targets: Mutex::new(None),
            state: Mutex::new(State {
                opts: SkyOptions::off(),
                frame: 0,
                world_s: 0.0,
                dt: 1.0 / 60.0,
                prev_vp: None,
                prev_eye: glam::Vec3::ZERO,
                jitter: [0.0; 2],
                prev_jitter: [0.0; 2],
                history_valid: false,
                parity: 0,
                adapt: Default::default(),
                meter_fwd: None,
                atmo_built: None,
                atmo_haze: 0.0,
                atmo_medium: [[0.0; 4]; 4],
                hist_look: None,
                recut: 0,
                cut_hold: false,
                hist_n: 0,
                terms: None,
                exposure: 1.0,
                agents_bg: None,
                depth_ok: false,
                weather_cache: None,
                props: None,
                lights_bg0: None,
                tiles: vec![0; TILE_WORDS],
                water: ([0.0; 4], [0; 4]),
            }),
            bytes,
        }
    }

    /// the atmosphere tables for other passes: (sky view, transmittance),
    /// both Rgba16Float 2D, sampled with a clamped linear sampler through the
    /// look_common helpers `at_sky(view, sampler, dir, look.sun.xyz,
    /// lk_luma(lk_sky_dome(vec3(0, 1, 0))))` (the sky as the volume draws it) and
    /// `at_sun(transmittance, sampler, height_m, look.sun.y)` (the sun through the
    /// atmosphere to a height). Rebuilt each frame before the volume pass.
    pub fn atmo_tables(&self) -> (&wgpu::TextureView, &wgpu::TextureView) {
        (&self.atmo_views[2], &self.atmo_views[0])
    }
    /// the exposure the last frame was drawn with (the target, metered and adapted), None before the first.
    pub fn drawn_exposure(&self) -> Option<f64> {
        let s = self.state.lock().unwrap();
        if s.frame > 0 { Some(s.exposure) } else { None }
    }

    pub fn options(&self) -> SkyOptions {
        self.state.lock().unwrap().opts
    }

    pub(crate) fn set_options(&self, o: SkyOptions) {
        let mut s = self.state.lock().unwrap();
        if s.opts.weather != o.weather || s.opts.seed != o.seed {
            s.terms = None;
        }
        if s.opts.world_time_s != o.world_time_s {
            s.world_s = o.world_time_s;
        }
        if !o.taa {
            s.history_valid = false;
        }
        s.opts = o;
    }

    /// The world clock (seconds since the world's epoch) and the frame time
    /// the next update advances it by. Default: 1/60 s a frame from
    /// `SkyOptions::world_time_s`.
    pub fn set_clock(&self, world_s: f64, dt: f64) {
        let mut s = self.state.lock().unwrap();
        s.world_s = world_s;
        s.dt = dt.max(0.0);
    }

    pub fn world_time(&self) -> f64 {
        self.state.lock().unwrap().world_s
    }

    /// The weather in force (preset, or the schedule at the world clock,
    /// re-derived once a world second: the ground water's integration is a
    /// few thousand steps).
    pub fn weather(&self) -> eo_sky::WeatherState {
        let mut s = self.state.lock().unwrap();
        if let WeatherMode::Schedule = s.opts.weather {
            let sec = s.world_s.floor();
            if let Some((t, w)) = s.weather_cache {
                if t == sec {
                    return w;
                }
            }
            let w = weather_of(&s.opts, sec);
            s.weather_cache = Some((sec, w));
            return w;
        }
        weather_of(&s.opts, s.world_s)
    }

    pub fn terms(&self) -> Option<eo_sky::SkyTerms> {
        self.state.lock().unwrap().terms
    }

    /// The street lamps (replaces the set). Returns how many were kept.
    /// The fallback placement's street lamps (they stand down where the props
    /// slice's real lamps are resident, `set_props_tiles`).
    pub fn set_lamps(&self, queue: &wgpu::Queue, lamps: &[LampLight]) -> usize {
        let n = lamps.len().min(LAMP_CAP as usize);
        // `size`: the head's height over the street (the pool decal stands under it).
        let gpu: Vec<GpuLight> = lamps[..n].iter().map(|l| GpuLight { size: (l.candela / eo_sky::photometry::LAMP_POOL_PEAK_LUX).sqrt() as f32, ..GpuLight::from_lamp(l) }).collect();
        if !gpu.is_empty() {
            queue.write_buffer(&self.lamps, 0, bytemuck::cast_slice(&gpu));
        }
        self.lamp_count.store(n as u32, Ordering::Relaxed);
        n
    }

    pub fn lamp_count(&self) -> u32 {
        self.lamp_count.load(Ordering::Relaxed)
    }

    /// The cells by the water (`set_waterside`): a lamp in one is drawn in the sea too.
    pub fn waterside_count(&self) -> u32 {
        self.waterside.load(Ordering::Relaxed)
    }

    /// The waterside cells: a grid of `cols` x `rows` cells of `cell` metres
    /// from `corner` (x, z), `cells` row-major (true: within the waterside
    /// reach of the shore). At most `WATER_CELLS_MAX` on a side.
    pub fn set_waterside(&self, queue: &wgpu::Queue, corner: [f64; 2], cell: f64, cols: usize, rows: usize, cells: &[bool]) {
        let (cols, rows) = (cols.min(WATER_CELLS_MAX), rows.min(WATER_CELLS_MAX));
        let mut words = vec![0u32; (cols * rows).div_ceil(32).max(1)];
        let mut n = 0;
        for r in 0..rows {
            for c in 0..cols {
                if cells.get(r * cols + c).copied().unwrap_or(false) {
                    let b = r * cols + c;
                    words[b / 32] |= 1 << (b % 32);
                    n += 1;
                }
            }
        }
        queue.write_buffer(&self.masks, TILE_WORDS as u64 * 4, bytemuck::cast_slice(&words));
        self.waterside.store(n, Ordering::Relaxed);
        self.state.lock().unwrap().water = ([corner[0] as f32, corner[1] as f32, cell as f32, 0.0], [cols as u32, rows as u32, 0, 0]);
    }

    /// The props slice's resident tiles (`eo_street` tile keys): the fallback
    /// lamps stand down in them, the props' own lamps light them. Written
    /// when the set changes.
    pub fn set_props_tiles(&self, queue: &wgpu::Queue, keys: impl Iterator<Item = (i32, i32)>) {
        let mut words = vec![0u32; TILE_WORDS];
        for (x, z) in keys {
            let (kx, kz) = (x + 128, z + 128);
            if (0..256).contains(&kx) && (0..256).contains(&kz) {
                let b = (kz * 256 + kx) as usize;
                words[b / 32] |= 1 << (b % 32);
            }
        }
        let mut s = self.state.lock().unwrap();
        if s.tiles != words {
            queue.write_buffer(&self.masks, 0, bytemuck::cast_slice(&words));
            s.tiles = words;
        }
    }

    /// The props slice's lamp lights (`PropsGpu::lights_buffer`, `light_count_buffer`), once they exist.
    pub(crate) fn bind_props(&self, props: Option<(&wgpu::Buffer, &wgpu::Buffer)>) {
        let mut s = self.state.lock().unwrap();
        let same = match (&s.props, props) {
            (None, None) => true,
            (Some((a, b)), Some((c, d))) => a == c && b == d,
            _ => false,
        };
        if !same {
            s.props = props.map(|(a, b)| (a.clone(), b.clone()));
            // The light bind groups carry them: built again.
            s.lights_bg0 = None;
            s.agents_bg = None;
        }
    }

    /// This frame's sub-pixel jitter (NDC), zero without TAA.
    pub fn jitter(&self) -> [f32; 2] {
        let s = self.state.lock().unwrap();
        if s.opts.taa && s.depth_ok { s.jitter } else { [0.0; 2] }
    }

    /// Advance the frame: the jitter for the coming frame (call before the camera is used).
    pub(crate) fn begin_frame(&self, out_size: (u32, u32)) {
        let mut s = self.state.lock().unwrap();
        s.prev_jitter = s.jitter;
        let i = s.frame % 8 + 1;
        let (w, h) = (out_size.0.max(1) as f32, out_size.1.max(1) as f32);
        s.jitter = [(halton(i, 2) - 0.5) * 2.0 / w, (halton(i, 3) - 0.5) * 2.0 / h];
    }

    /// The weather, the decks and the night for this frame, and the uniform.
    /// `f` is the look frame (already built for this weather), `cam` the
    /// unjittered camera.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update(&self, queue: &wgpu::Queue, cam: &Camera, f: &eo_look::LookFrame, terms: &eo_sky::SkyTerms, out_size: (u32, u32), depth_ok: bool, daylight: Option<&eo_look::phys::Daylight>) -> f64 {
        if !self.sheet_ready.swap(true, Ordering::Relaxed) {
            let sheet = eo_sky::clouds::bake_sheet(0x5eed_c10d);
            for (mip, level) in eo_sky::clouds::sheet_mips(&sheet).iter().enumerate() {
                let r = (SHEET_RES >> mip).max(1);
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo { texture: &self.sheet, mip_level: mip as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                    bytemuck::cast_slice(level),
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(r * 4), rows_per_image: Some(r) },
                    wgpu::Extent3d { width: r, height: r, depth_or_array_layers: 1 },
                );
            }
        }
        self.clouds.upload_once(queue); // the blue-noise tile, once
        let mut s = self.state.lock().unwrap();
        let o = s.opts;
        s.frame = s.frame.wrapping_add(1);
        let dt = s.dt;
        s.world_s += dt;
        s.depth_ok = depth_ok;
        let vp = cam.view_proj();
        let prev_vp = s.prev_vp.unwrap_or(vp);
        let prev_eye = if s.prev_vp.is_some() { s.prev_eye } else { cam.eye };
        // A camera cut (or a first frame) discards the history.
        // a clock jump of a quarter hour or more is a cut too (the clouds and
        // the frame of another hour must not linger in the histories: a blue-hour
        // card taken after a golden one kept the golden clouds for its settle).
        let jumped = s.adapt.last_hour.is_some_and(|h| (h - f.hour).abs() >= 0.25);
        // A change of hour or weather discards the histories for two frames (the first
        // frame after a change may still carry the last look's terms).
        let look_now = (f.hour, weather_key(&terms.weather));
        if s.hist_look.is_some_and(|l| (l.0 - look_now.0).abs() >= 0.25 || l.1 != look_now.1) {
            s.recut = 2;
        }
        s.hist_look = Some(look_now);
        // the recut frames count down per encoded frame (in `encode`), not per update.
        let recut = s.recut > 0;
        let cut = s.prev_vp.is_none() || cam.eye.distance(prev_eye) > 25.0 || jumped || recut || s.cut_hold;
        s.cut_hold = cut;
        // Frames of history since the cut: the volume averages them evenly until its
        // own weight takes over (one frame no longer stands for a dozen).
        s.hist_n = if cut || !s.history_valid { 0 } else { s.hist_n.saturating_add(1) };
        // the camera meter (eo-sky `night::meter`) over a 5 x 3 grid of
        // the view, centre-weighted; a cut or a turn of more than ~30 deg
        // snaps the adaptation (a new shot, not a pan).
        let meter = if o.meter {
            let (fw, rt) = (cam.forward(), cam.right());
            let up = rt.cross(fw);
            let ty = (0.5 * cam.fov_y_deg.to_radians()).tan();
            let tx = ty * cam.aspect;
            let mut dirs = Vec::with_capacity(15);
            for j in 0..3 {
                for i in 0..5 {
                    let (x, y) = (i as f32 / 2.0 - 1.0, j as f32 - 1.0);
                    let d = (fw + rt * (x * tx) + up * (y * ty)).normalize();
                    dirs.push(([d.x as f64, d.y as f64, d.z as f64], 1.5 - 0.5 * (x * x + y * y) as f64));
                }
            }
            match daylight.filter(|d| d.weight > 0.0) {
                // the physical daylight's sky and card where it lights the frame (by its weight)
                Some(d) => {
                    let wt = d.weight;
                    let card_js = 0.18 * (eo_sky::night::key_on_ground(f) * eo_sky::night::mean_beam(&terms.weather) + eo_sky::night::ground_fill(f, eo_sky::night::SIDEWALK_ENV)) / std::f64::consts::PI;
                    let card_ph = 0.18 * eo_sky::night::horizontal_illuminance(f, &terms.weather, wt) / std::f64::consts::PI;
                    let card = card_js + (card_ph - card_js) * wt;
                    let eye_m = cam.eye.y as f64;
                    eo_sky::night::meter_with(f, &dirs, card, &|dd| {
                        let js = eo_look::math::luminance(f.model.dome(dd));
                        js + (eo_look::math::luminance(d.sky(dd, eye_m)) - js) * wt
                    })
                }
                None => eo_sky::night::meter(f, &terms.weather, &dirs),
            }
        } else {
            1.0
        };
        let turned = s.meter_fwd.is_some_and(|p| p.dot(cam.forward()) < 0.87);
        s.meter_fwd = Some(cam.forward());
        if cut || turned {
            s.adapt.current = None;
        }
        // Exposure: the target (weather, night solve, the meter), filtered.
        let target = terms.exposure_target * meter;
        let exposure = if o.adaptation {
            s.adapt.step(target, f.hour, dt)
        } else {
            s.adapt.last_hour = Some(f.hour);
            target
        };
        s.exposure = exposure;
        if cut || !s.history_valid {
            crate::hitch::mark(crate::hitch::Mark::SkyHistoryReset, 1);
        }
        // The weather of this instant (the frame's terms carry the one it was built for).
        let w_now = match o.weather {
            WeatherMode::Schedule => s.weather_cache.map(|c| c.1).unwrap_or(terms.weather),
            WeatherMode::Preset(_) => terms.weather,
        };
        let w = &w_now;
        let decks = &terms.decks;
        let world_s = s.world_s;
        // the decks drift with the hour of the day too (the clock the
        // look is set to), so each hour shows another field (every card of one
        // camera showed the same clouds at 12, 18.5, 19.4 and 21 h).
        let drift_s = world_s + f.hour * 3600.0;
        let low = eo_sky::clouds::drift_offset(decks.drift_low, drift_s, eo_sky::clouds::LOW_TILE_M);
        let high = eo_sky::clouds::drift_offset(decks.drift_high, drift_s, eo_sky::clouds::HIGH_TILE_M);
        let marine = eo_sky::clouds::drift_offset(decks.marine_drift, drift_s, eo_sky::clouds::MARINE_TILE_M);
        let v4 = |a: [f64; 3], w: f64| [a[0] as f32, a[1] as f32, a[2] as f32, w as f32];
        // The sky's irradiance on the cloud tops, and under the deck the ground's
        // bounce plus, at night, the city's glow (urban glow x the lamp pool colour).
        let m = &f.model;
        let sky_irr = m.sky_irradiance();
        let amb_top = sky_irr.map(|v| v / std::f64::consts::PI);
        // At night a deck's base is lit by the city below it (the sodium-orange
        // underside of San Francisco's low cloud): the night sky's own
        // radiance x 2.5 in the street-lamp colour, x the weather's urban glow
        // over clear's (environment-ibl.js `urbanGlow`).
        // (the full night's sky radiance, not this hour's: the city's light does
        // not grow with the twilight; it shows once the sky is darker than it,
        // from nautical twilight: sun -6 .. -12 deg)
        let glow = terms.lamps_on * (1.0 - eo_look::math::smoothstep(-12.0, -6.0, f.model.sun.altitude_deg)) * w.profile.urban_glow / eo_sky::weather::CLEAR.urban_glow;
        let pool = eo_sky::lights::street_lamp_color();
        let night_sky = {
            static N: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
            *N.get_or_init(|| eo_look::math::luminance(eo_look::sky::SkyModel::new(1.0).sky_irradiance()) / std::f64::consts::PI)
        };
        let gnd = m.ground_radiance;
        // the volume's light (clouds, the marine layer, the shafts; the
        // city keeps the rig's own key). The rig's key carries the shipped
        // look's warm grade (R/B 1.75 at noon); a cloud or a fog bank is lit
        // by the sun itself, near white at a high sun (~5500 K, R/B ~1.1), and
        // reddened only as it sinks: the key is taken 60 % of the way to its
        // luminance at a high sun, less as the sun drops under 30 deg, and all
        // the way under a full deck, where every point is lit by light
        // scattered many times (the cream overcast and the sepia storm).
        let alt = m.sun.altitude_deg;
        let ovc = w.profile.overcast.clamp(0.0, 1.0);
        let neutral = (0.6 * eo_look::math::smoothstep(4.0, 30.0, alt)).max(ovc);
        // The moon's key (the JS's day-for-night units, up to 0.02) is a third of
        // the twilight sky's irradiance at the blue hour, where the real moon is
        // ~1/1000 of it: through the clouds' forward lobe it lit a twilight
        // cumulus field bright peach. It lights the volume only as the
        // twilight fades (sun -4 .. -12 deg).
        let moon_keys = terms.moon_key > 0.0 && f.key_dir == terms.moon.dir;
        let moon_fade = if moon_keys { eo_look::math::smoothstep(-4.0, -12.0, alt) } else { 1.0 };
        let vkey = {
            let l = eo_look::math::luminance(f.key);
            f.key.map(|v| (v + (l - v) * neutral) * moon_fade)
        };
        // a cloud 1-2 km up still sees the sun for ~1-1.5 deg after it has
        // set for the ground, through the longest, reddest path: the rig's key
        // (the ground's) is gone by then, and the clouds over a setting sun went
        // black-brown. The volume takes at least a deep orange key from the sun
        // at +4 deg down to -2 deg.
        let vkey = {
            let k = eo_look::math::smoothstep(-2.0, 0.5, alt) * (1.0 - eo_look::math::smoothstep(1.5, 4.0, alt)) * (1.0 - ovc);
            [0, 1, 2].map(|i| vkey[i].max(SUNSET_CLOUD_KEY[i] * k))
        };
        // The sky's light on the clouds: Preetham's irradiance is a deep blue
        // (B/R 5.8 at noon); skylight on a cloud is B/R ~1.5-2: half-way to grey.
        let amb_top = {
            let l = eo_look::math::luminance(amb_top);
            amb_top.map(|v| v + (l - v) * 0.5)
        };
        // Under the deck: mostly the sky (the base of a cumulus is blue-grey,
        // not the brown of the street multiplied in), a little ground bounce,
        // and at night the city's glow, about twice the skyglow at the horizon
        // (a low deck over a lit city is brighter than the clear sky between).
        let glow_rgb = eo_look::math::luminance(pool).max(1e-6);
        // (the city's glow on the bases is the shader's, by position: sky_volume `vol_extra_amb`)
        let _ = glow_rgb;
        let amb_base = [0, 1, 2].map(|i| gnd[i] * 0.3 + amb_top[i] * 0.55 + glow * night_sky * 2.5 * pool[i]);
        // the twilight sky's light toward the set sun on the clouds (the
        // afterglow arch at the horizon in the sun's azimuth, eo-look `SkyExt`).
        let twi = {
            let s = [m.sun.x, m.sun.y, m.sun.z];
            let h = (s[0] * s[0] + s[2] * s[2]).sqrt().max(1e-6);
            eo_look::sky::afterglow_arch([s[0] / h, 0.02, s[2] / h], s, m.ext.afterglow)
        };
        let atmo_haze = (ATMO_HAZE * (1.0 + 2.0 * w.profile.haze)) as f32;
        s.atmo_haze = atmo_haze;
        // the medium the tables integrate: the clear coastal air (eo-look `atmosphere`) with the
        // physical daylight, else the standard aerosol x ATMO_HAZE as before; both x the weather's haze.
        // With the physical daylight the medium moves from the old one to the coastal air by
        // the daylight's weight (the sun up: all of it; the twilight and the night keep the
        // tables they were tuned on). The haze density folds into the aerosol's coefficients.
        let legacy = eo_look::atmosphere::Atmosphere::legacy();
        let legacy = eo_look::atmosphere::Atmosphere { aerosol_scat: legacy.aerosol_scat * atmo_haze as f64, aerosol_abs: legacy.aerosol_abs * atmo_haze as f64, ..legacy };
        let atm = match daylight.filter(|_| o.phys_sky) {
            Some(d) => {
                let c = eo_look::atmosphere::Atmosphere::coastal_clear();
                let dens = eo_sky::frame::daylight_density(w);
                let c = eo_look::atmosphere::Atmosphere { aerosol_scat: c.aerosol_scat * dens, aerosol_abs: c.aerosol_abs * dens, ..c };
                eo_look::atmosphere::Atmosphere::lerp(&legacy, &c, d.weight.clamp(0.0, 1.0))
            }
            None => legacy,
        };
        let atmo_m = [atm.aerosol_scat as f32, atm.aerosol_abs as f32, atm.aerosol_h_km as f32, if o.ap_lut { 1.0 } else { 0.0 }];
        let atmo_s = [atm.aerosol_spectral[0] as f32, atm.aerosol_spectral[1] as f32, atm.aerosol_spectral[2] as f32, AP_REACH_KM];
        let atmo_r = [atm.rayleigh[0] as f32, atm.rayleigh[1] as f32, atm.rayleigh[2] as f32, atm.rayleigh_h_km as f32];
        let atmo_o = [atm.ozone[0] as f32, atm.ozone[1] as f32, atm.ozone[2] as f32, 0.0];
        // the transmittance and multiple-scattering tables are rebuilt when any of the medium moves
        s.atmo_medium = [[atmo_m[0], atmo_m[1], atmo_m[2], 0.0], atmo_s, atmo_r, atmo_o];
        let atmo4 = match daylight.filter(|_| o.phys_sky) {
            Some(d) => [d.sun_toa[0] as f32, d.sun_toa[1] as f32, d.sun_toa[2] as f32, d.weight as f32],
            None => [0.0; 4],
        };
        let fwd = cam.forward();
        let right = cam.right();
        let up = right.cross(fwd);
        let haze_sigma = if o.shafts { (3.912 / 30000.0) * (1.0 + 8.0 * w.profile.haze) } else { 0.0 };
        let rate = w.rain_mm_h;
        let fall = (3.5 + 1.9 * (1.0 + rate).ln()).clamp(3.5, 9.5);
        let live_near = 0.1 + 0.9 * (rate / 25.0).sqrt();
        let live_far = ((rate - 1.5) / 18.0).clamp(0.0, 1.0);
        let threshold = 2.101 / exposure.max(1e-3);
        let (strength, radius) = (0.26, 0.22);
        let mw = [1.0, 0.8, 0.6, 0.4, 0.2].map(|fv: f64| ((fv + (1.2 - fv - fv) * radius) * strength) as f32);
        let lamp_k = terms.lamp_units_per_lux;
        let lamps_on = if o.lights { terms.lamps_on } else { 0.0 };
        let veh = if o.lights { terms.vehicle_lamps } else { 0.0 };
        let moon = terms.moon;
        let volume_ok = depth_ok && o.needs_volume();
        let u = SkyUniform {
            vp: vp.to_cols_array_2d(),
            prev_vp: prev_vp.to_cols_array_2d(),
            eye: [cam.eye.x, cam.eye.y, cam.eye.z, (world_s % 86400.0) as f32],
            prev_eye: prev_eye.extend(1.0).to_array(),
            jitter: [s.jitter[0], s.jitter[1], s.prev_jitter[0], s.prev_jitter[1]],
            taa: [if o.taa && depth_ok { 1.0 } else { 0.0 }, if s.history_valid && !cut { s.hist_n.max(1) as f32 } else { 0.0 }, 0.1, dt.max(1e-4) as f32],
            res: [out_size.0 as f32, out_size.1 as f32, 1.0 / out_size.0.max(1) as f32, 1.0 / out_size.1.max(1) as f32],
            cloud0: [decks.low_base_m as f32, decks.low_top_m as f32, if o.clouds { decks.coverage as f32 } else { 0.0 }, decks.low_opacity as f32],
            cloud1: [decks.extinction as f32, decks.high_alt_m as f32, decks.high_opacity as f32, decks.beam as f32],
            cloud2: [low[0] as f32, low[1] as f32, high[0] as f32, high[1] as f32],
            cloud3: [
                if o.fog { decks.marine_sigma as f32 } else { 0.0 },
                decks.marine_top_m as f32,
                decks.marine_swing_m as f32,
                // Offshore, a bank stands over the Pacific in clear weather too.
                if o.fog { (eo_sky::weather::MARINE_FOG_SIGMA * 0.8) as f32 } else { 0.0 },
            ],
            cloud4: [marine[0] as f32, marine[1] as f32, haze_sigma as f32, 400.0],
            sun_rad: v4(vkey, if terms.moon_key > 0.0 && f.key_dir == moon.dir { 1.0 } else { 0.0 }),
            amb_top: v4(amb_top, glow),
            amb_base: v4(amb_base, o.cloud_steps() as f64),
            wet: [if o.wet { w.wetness as f32 } else { 0.0 }, w.puddle_fill as f32, if o.rain || o.wet { rate as f32 } else { 0.0 }, (world_s % 3600.0) as f32],
            rain: [live_near as f32, live_far as f32, fall as f32, 0.3], // streak strength 0.62 -> 0.3 (opaque white rods)
            wind: [w.wind[0] as f32, w.wind[1] as f32, w.gust as f32, w.flash as f32],
            lights: [if o.lights { FRAME_LIGHT_CAP } else { 0 }, CLUSTERS[0], CLUSTERS[1], CLUSTERS[2]],
            cluster: [CLUSTER_NEAR_M, CLUSTER_FAR_M, (0.5 * cam.fov_y_deg.to_radians()).tan(), cam.aspect],
            cam_fwd: fwd.extend(0.0).to_array(),
            cam_right: right.extend(0.0).to_array(),
            cam_up: up.extend(0.0).to_array(),
            night: [lamps_on as f32, terms.dusk as f32, terms.nightness as f32, veh as f32],
            moon: v4(moon.dir, if o.moon_stars { terms.moon_disc } else { 0.0 }),
            stars: [if o.moon_stars { terms.star_level as f32 } else { 0.0 }, (f.hour * 15.041 * std::f64::consts::PI / 180.0) as f32, moon.illuminated as f32, if moon.waxing { 1.0 } else { 0.0 }],
            bloom: [threshold as f32, 0.55, 1.0, if o.bloom { 1.0 } else { 0.0 }],
            bloom_w: [mw[0], mw[1], mw[2], mw[3]],
            bloom_w2: [mw[4], 4.0, lamp_k as f32, w.profile.lamp_haze as f32],
            flags: [o.bits(), s.frame, (o.ssr && s.history_valid && !cut) as u32, volume_ok as u32],
            beam: [eo_sky::night::mean_beam(w) as f32, 0.0, 0.0, 0.0],
            twi: v4(twi, 0.0),
            atmo: [m.sun.x as f32, m.sun.y as f32, m.sun.z as f32, ATMO_SKY_SCALE as f32],
            atmo2: [
                if o.atmosphere { 1.0 } else { 0.0 },
                // under a deck the weather's own (CIE overcast) dome
                (eo_look::math::smoothstep(-16.0, -9.0, m.sun.altitude_deg) * (1.0 - 0.9 * w.profile.overcast.clamp(0.0, 1.0))) as f32,
                atmo_haze,
                ATMO_CLOUD_SUN as f32,
            ],
            sun_glint: sun_glint(f.hour, eo_sky::night::mean_beam(&w)),
            atmo3: {
                // the weather's visibility over clear air's: the range fog reached 95 % at `aerial.far`
                let far = f.aerial.far.max(100.0);
                let far_clear = far * eo_sky::weather::CLEAR.vis_far / w.profile.vis_far.max(1e-3);
                // the weather's own veil is drawn here with the atmosphere on (look.rs
                // pushes the range fog out): surfaces fade to the sky the volume draws behind
                // them, not to `look.fog_color` (lighter and bluer than the grey fog bank and
                // rain veil, so the far hills stood out of the fog in front of them).
                [if o.atmosphere && far < far_clear * 0.97 { 1.0 } else { 0.0 }, (AP_HAZE_PER_KM * 0.001 * (1.0 + 2.0 * w.profile.haze)) as f32, f.aerial.near as f32, far as f32]
            },
            atmo4,
            atmo_m,
            atmo_s,
            atmo_r,
            atmo_o,
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
        // The lights' scales: candela -> irradiance units, on with the lamps.
        let agents = s.agents_bg.as_ref().map(|a| a.0).unwrap_or(0);
        // The props lamps carry the JS practical's intensity in the JS's light
        // units, which are this rig's (only the lamps' clock and the twilight
        // scale, `lamp_k` over the night's units per lux, apply); their pools
        // are set to the RP-8 peak (sky_lights.wgsl `lights_props`).
        let props_k = lamp_k / eo_sky::night::lamp_units_per_lux();
        let tiles_any = s.tiles.iter().any(|w| *w != 0);
        let p = LtParams {
            counts: [self.lamp_count.load(Ordering::Relaxed), agents, FRAME_LIGHT_CAP, (s.agents_bg.is_some()) as u32],
            scale: [(lamp_k * lamps_on) as f32, (lamp_k * eo_sky::photometry::LOW_BEAM_PEAK_CD * veh) as f32, 0.0, 45.0],
            props: [(props_k * lamps_on) as f32, if tiles_any { 1.0 } else { 0.0 }, (eo_sky::photometry::LAMP_POOL_PEAK_LUX * lamp_k * lamps_on) as f32, 0.0],
            water: s.water.0,
            water_n: s.water.1,
        };
        queue.write_buffer(&self.lt_params, 0, bytemuck::bytes_of(&p));
        let bw: [[f32; 4]; BLOOM_LEVELS] = [[mw[0], 1.0, 0.0, 0.0], [mw[1], 1.0, 0.0, 0.0], [mw[2], 1.0, 0.0, 0.0], [mw[3], mw[4], 0.0, 0.0], [0.0, 0.0, 0.0, 0.0]];
        for (b, w) in self.bloom_levels.iter().zip(bw.iter()) {
            queue.write_buffer(b, 0, bytemuck::cast_slice(w));
        }
        s.prev_vp = Some(vp);
        s.prev_eye = cam.eye;
        exposure
    }

    /// The agent buffers the light gather reads (once the agents exist; every
    /// frame, since their count moves).
    pub(crate) fn bind_agents(&self, agents: Option<(&wgpu::Buffer, &wgpu::Buffer, u32)>) {
        let mut s = self.state.lock().unwrap();
        match agents {
            Some((ab, mb, n)) => match s.agents_bg.as_mut() {
                Some(a) => a.0 = n,
                None => {
                    let bg = self.lights_bg(Some((ab, mb)), s.props.as_ref());
                    s.agents_bg = Some((n, bg));
                }
            },
            None => s.agents_bg = None,
        }
    }

    fn lights_bg(&self, agents: Option<(&wgpu::Buffer, &wgpu::Buffer)>, props: Option<&(wgpu::Buffer, wgpu::Buffer)>) -> wgpu::BindGroup {
        let d = &self.dummy_storage;
        let (ab, mb) = agents.unwrap_or((d, d));
        let (pl, pn) = props.map(|(a, b)| (a, b)).unwrap_or((d, d));
        crate::hitch::mark(crate::hitch::Mark::LightsBindGroup, 1);
        crate::hitch::mark(crate::hitch::Mark::BindGroupCreated, 1);
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky-lights-bg"),
            layout: &self.lights_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: self.lamps.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: self.frame_lights.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: self.counter.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: self.clusters.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: ab.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: mb.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: self.lt_params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 8, resource: self.sprites.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 9, resource: pl.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 10, resource: pn.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 11, resource: self.masks.as_entire_binding() },
            ],
        })
    }

    /// Gather + cluster the lights (before the city pass).
    pub(crate) fn encode_lights(&self, enc: &mut wgpu::CommandEncoder, ts: Option<(&wgpu::QuerySet, u32)>) {
        let tw = ts.map(|(q, b)| wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(b + Q_SKY_LIGHTS), end_of_pass_write_index: Some(b + Q_SKY_LIGHTS + 1) });
        if !self.noise_ready.swap(true, Ordering::Relaxed) {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("sky-noise (once)"), timestamp_writes: None });
            pass.set_pipeline(&self.noise_pipe);
            pass.set_bind_group(0, &self.noise_bg, &[]);
            pass.dispatch_workgroups(NOISE_RES / 4, NOISE_RES / 4, NOISE_RES / 4);
            pass.set_pipeline(&self.noise_down);
            for (k, bg) in self.noise_down_bgs.iter().enumerate() {
                let r = (NOISE_RES >> (k + 1)).max(1);
                pass.set_bind_group(0, bg, &[]);
                pass.dispatch_workgroups(r.div_ceil(4), r.div_ceil(4), r.div_ceil(4));
            }
        }
        // the cloud noise (once) and this frame's cloud-shadow map, before the city pass reads it
        self.clouds.encode(enc, ts.map(|(q, b)| (q, b + Q_CLOUD_SHADOW)));
        let mut s = self.state.lock().unwrap();
        let on = s.opts.lights;
        enc.clear_buffer(&self.counter, 0, None);
        let ncl = CLUSTERS[0] * CLUSTERS[1] * CLUSTERS[2];
        if !on {
            enc.clear_buffer(&self.clusters, 0, Some(ncl as u64 * 4));
            enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("sky-lights (off)"), timestamp_writes: tw });
            return;
        }
        if s.agents_bg.is_none() && s.lights_bg0.is_none() {
            s.lights_bg0 = Some(self.lights_bg(None, s.props.as_ref()));
        }
        let bg = match &s.agents_bg {
            Some((_, bg)) => bg,
            None => s.lights_bg0.as_ref().unwrap(),
        };
        let agents = s.agents_bg.as_ref().map(|a| a.0).unwrap_or(0);
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("sky-lights"), timestamp_writes: tw });
        pass.set_bind_group(0, bg, &[]);
        let lamps = self.lamp_count.load(Ordering::Relaxed);
        if lamps > 0 {
            pass.set_pipeline(&self.lights_pipes[0]);
            pass.dispatch_workgroups(lamps.div_ceil(64), 1, 1);
        }
        if s.props.is_some() {
            // The props slice's lamps (the slots past its count return at once).
            pass.set_pipeline(&self.lights_pipes[3]);
            pass.dispatch_workgroups(crate::props::LIGHT_CAPACITY.div_ceil(64), 1, 1);
        }
        if agents > 0 {
            pass.set_pipeline(&self.lights_pipes[1]);
            pass.dispatch_workgroups(agents.div_ceil(64), 1, 1);
        }
        pass.set_pipeline(&self.lights_pipes[2]);
        pass.dispatch_workgroups(ncl, 1, 1);
    }

    /// The parity of the history the coming resolve writes (the city pass
    /// reads the other one as the previous frame).
    pub(crate) fn parity(&self) -> usize {
        self.state.lock().unwrap().parity
    }

    /// Size-dependent targets (created on the first frame of a size). Returns
    /// whether they were (re)created.
    pub(crate) fn ensure_targets(&self, size: (u32, u32), hdr_format: wgpu::TextureFormat) -> bool {
        let mut t = self.targets.lock().unwrap();
        if t.as_ref().map(|t| t.size) == Some(size) {
            return false;
        }
        let d = &self.device;
        // the volume's resolution divisor (`VOL_DIV`). At 1 (full resolution) the
        // cloud rims are sharper but the volume pass costs 0.5-1.15 ms more
        // (0.69-1.55 ms against 0.18-0.40 ms), so it stays at 2.
        // the trace stays a quarter of the pixels (one of each 2 x 2 block a
        // frame, in turn); sky_recon.wgsl rebuilds the full resolution from it.
        let half = ((size.0 / VOL_DIV).max(1), (size.1 / VOL_DIV).max(1));
        let su = wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING;
        let ru = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let vol = [0, 1].map(|_| tex(d, "sky-volume", half, wgpu::TextureFormat::Rgba16Float, su));
        let volz = [0, 1].map(|_| tex(d, "sky-volume-z", half, wgpu::TextureFormat::R32Float, su));
        let full = [0, 1].map(|_| tex(d, "sky-volume-full", size, wgpu::TextureFormat::Rgba16Float, su));
        let fullz = [0, 1].map(|_| tex(d, "sky-volume-full-z", size, wgpu::TextureFormat::R32Float, su));
        let hist = [tex(d, "sky-history-0", size, hdr_format, ru), tex(d, "sky-history-1", size, hdr_format, ru)];
        let vel = tex(d, "sky-velocity", size, VEL_FORMAT, ru);
        let mut mips = vec![];
        let mut ups = vec![];
        let mut bytes = half.0 as u64 * half.1 as u64 * (8 + 4) * 2 + size.0 as u64 * size.1 as u64 * (8 * 2 + 4) + size.0 as u64 * size.1 as u64 * (8 + 4) * 2;
        for i in 0..BLOOM_LEVELS {
            let s = ((size.0 >> (i + 1)).max(1), (size.1 >> (i + 1)).max(1));
            mips.push(tex(d, "sky-bloom-down", s, wgpu::TextureFormat::Rgba16Float, su));
            bytes += s.0 as u64 * s.1 as u64 * 8;
            if i + 1 < BLOOM_LEVELS {
                ups.push(tex(d, "sky-bloom-up", s, wgpu::TextureFormat::Rgba16Float, su));
                bytes += s.0 as u64 * s.1 as u64 * 8;
            }
        }
        let bg = |src: &wgpu::TextureView, add: &wgpu::TextureView, dst: &wgpu::TextureView, level: usize| {
            d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("sky-bloom-bg"),
                layout: &self.bloom_bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: self.uniform.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(src) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(add) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.lin) },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(dst) },
                    wgpu::BindGroupEntry { binding: 5, resource: self.bloom_levels[level].as_entire_binding() },
                ],
            })
        };
        // Per history parity: prefilter, 4 downs, 4 ups.
        let mut bloom_bgs: [Vec<wgpu::BindGroup>; 2] = [vec![], vec![]];
        for (p, list) in bloom_bgs.iter_mut().enumerate() {
            list.push(bg(&hist[p], &hist[p], &mips[0], 0));
            for i in 1..BLOOM_LEVELS {
                list.push(bg(&mips[i - 1], &mips[i - 1], &mips[i], i));
            }
            // up_3 = m3 w3 + tent(m4) w4; up_i = m_i w_i + tent(up_(i+1)).
            for i in (0..BLOOM_LEVELS - 1).rev() {
                let src = if i == BLOOM_LEVELS - 2 { &mips[i + 1] } else { &ups[i + 1] };
                list.push(bg(src, &mips[i], &ups[i], i));
            }
        }
        *t = Some(Targets { size, vol, volz, full, fullz, hist, vel, _mips: mips, ups, scene: None, bloom_bgs, bytes });
        let mut s = self.state.lock().unwrap();
        s.history_valid = false;
        true
    }

    /// Bytes of the size-dependent targets, once they exist.
    pub fn target_bytes(&self) -> u64 {
        self.targets.lock().unwrap().as_ref().map(|t| t.bytes).unwrap_or(0)
    }

    /// The previous resolved frame (the city pass's reflection source) for a parity.
    pub(crate) fn history_view(&self, p: usize) -> Option<wgpu::TextureView> {
        self.targets.lock().unwrap().as_ref().map(|t| t.hist[p].clone())
    }

    /// The bloom result (up level 0).
    pub(crate) fn bloom_view(&self) -> Option<wgpu::TextureView> {
        self.targets.lock().unwrap().as_ref().map(|t| t.ups[0].clone())
    }

    pub(crate) fn linear_sampler(&self) -> &wgpu::Sampler {
        &self.lin
    }

    /// After the city and occlusion passes: luminaires and rain into the HDR
    /// scene, the agents' velocity, the volume, the resolve, the bloom.
    /// Returns the parity of the history it wrote (None: nothing ran, post
    /// reads the HDR scene).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode_after(
        &self,
        enc: &mut wgpu::CommandEncoder,
        look_buf: &wgpu::Buffer,
        shadow_array: &wgpu::TextureView,
        cmp: &wgpu::Sampler,
        depth: &wgpu::TextureView,
        hdr: &wgpu::TextureView,
        agents: Option<&crate::agents::AgentsGpu>,
        look: &crate::look::LookGpu,
        ts: Option<(&wgpu::QuerySet, u32)>,
        depth_now: bool,
    ) -> Option<usize> {
        let tw = |slot: u32| ts.map(|(q, b)| (q, b + slot));
        let empty = |enc: &mut wgpu::CommandEncoder, slot: u32| {
            if let Some((q, b)) = tw(slot) {
                enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("sky (off)"),
                    timestamp_writes: Some(wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(b), end_of_pass_write_index: Some(b + 1) }),
                });
            }
        };
        let (o, parity, history_valid) = {
            let s = self.state.lock().unwrap();
            (s.opts, s.parity, s.history_valid)
        };
        let depth_ok = depth_now;
        if !o.any_post() || !depth_ok {
            for slot in [Q_SKY_FX, Q_SKY_VOLUME, Q_SKY_RESOLVE, Q_SKY_BLOOM] {
                empty(enc, slot);
            }
            self.state.lock().unwrap().history_valid = false;
            return None;
        }
        let mut tg = self.targets.lock().unwrap();
        let t = tg.as_mut().expect("ensure_targets before encode_after");
        // FX: luminaires (additive) and rain (premultiplied) over the scene.
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sky-fx"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: hdr, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: depth, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }), stencil_ops: None }),
                timestamp_writes: tw(Q_SKY_FX).map(|(q, b)| wgpu::RenderPassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(b), end_of_pass_write_index: None }),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.fx_bg, &[]);
            if o.sprites && o.lights {
                // Luminaires and far pools (12 vertices an instance, from the gather).
                pass.set_pipeline(&self.sprite_pipe);
                pass.draw_indirect(&self.counter, 16);
                // The waterside ones' glitter in the sea (the same list; 6 of the 12 vertices).
                pass.set_pipeline(&self.glitter_pipe);
                pass.draw_indirect(&self.counter, 16);
            }
            if o.rain {
                pass.set_pipeline(&self.rain_pipe);
                pass.draw(0..6, 0..11_000);
            }
        }
        // The agents' velocity (cleared to "none": the camera's motion).
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sky-velocity"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &t.vel,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: VEL_NONE, g: VEL_NONE, b: 0.0, a: 0.0 }), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: depth, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }), stencil_ops: None }),
                timestamp_writes: tw(Q_SKY_FX).map(|(q, b)| wgpu::RenderPassTimestampWrites { query_set: q, beginning_of_pass_write_index: None, end_of_pass_write_index: Some(b + 1) }),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if o.taa {
                if let Some(a) = agents {
                    a.draw_velocity(&mut pass, look);
                }
            }
        }
        // Scene-dependent bind groups.
        let dt = depth.texture();
        let ht = hdr.texture();
        if t.scene.as_ref().is_none_or(|(d, h, _)| d != dt || h != ht) {
            let d = &self.device;
            let volume = |p: usize| {
                d.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("sky-volume-bg"),
                    layout: &self.volume_bgl,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: look_buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: self.uniform.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(depth) },
                        wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&self.sheet_view) },
                        wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&self.noise_view) },
                        wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(&self.rep) },
                        wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(shadow_array) },
                        wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::Sampler(cmp) },
                        wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&t.vol[p]) },
                        wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(&t.volz[p]) },
                        wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(&t.vol[1 - p]) },
                        wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::TextureView(&t.volz[1 - p]) },
                        wgpu::BindGroupEntry { binding: 12, resource: wgpu::BindingResource::Sampler(&self.lin) },
                        wgpu::BindGroupEntry { binding: 13, resource: wgpu::BindingResource::TextureView(&self.atmo_views[0]) },
                        wgpu::BindGroupEntry { binding: 14, resource: wgpu::BindingResource::TextureView(&self.atmo_views[2]) },
                        // clouds
                        wgpu::BindGroupEntry { binding: 15, resource: wgpu::BindingResource::TextureView(&self.clouds.base_view) },
                        wgpu::BindGroupEntry { binding: 16, resource: wgpu::BindingResource::TextureView(&self.clouds.detail_view) },
                        wgpu::BindGroupEntry { binding: 17, resource: wgpu::BindingResource::TextureView(&self.clouds.curl_view) },
                        wgpu::BindGroupEntry { binding: 18, resource: wgpu::BindingResource::TextureView(&self.clouds.shadow_view) },
                        wgpu::BindGroupEntry { binding: 19, resource: wgpu::BindingResource::TextureView(&self.clouds.blue_view) },
                        wgpu::BindGroupEntry { binding: 20, resource: wgpu::BindingResource::TextureView(&self.ap_views[0]) },
                        wgpu::BindGroupEntry { binding: 21, resource: wgpu::BindingResource::TextureView(&self.ap_views[1]) },
                    ],
                })
            };
            let recon = |p: usize| {
                d.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("sky-recon-bg"),
                    layout: &self.recon_bgl,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: look_buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: self.uniform.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(depth) },
                        wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&t.vol[p]) },
                        wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&t.volz[p]) },
                        wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&t.full[1 - p]) },
                        wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&t.fullz[1 - p]) },
                        wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(&t.full[p]) },
                        wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&t.fullz[p]) },
                        wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::Sampler(&self.lin) },
                    ],
                })
            };
            let resolve = |p: usize| {
                let prev = 1 - p;
                d.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("sky-resolve-bg"),
                    layout: &self.resolve_bgl,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: look_buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: self.uniform.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(hdr) },
                        wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(depth) },
                        // the reconstructed full-resolution volume
                        wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&t.full[p]) },
                        wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&t.fullz[p]) },
                        wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&t.hist[prev]) },
                        wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(&t.vel) },
                        wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::Sampler(&self.lin) },
                        wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(&self.atmo_views[0]) },
                    ],
                })
            };
            // resolve[p] writes history p and reads history 1 - p and volume p.
            crate::hitch::mark(crate::hitch::Mark::BindGroupCreated, 4);
            t.scene = Some((dt.clone(), ht.clone(), SceneBinds { volume: [volume(0), volume(1)], recon: [recon(0), recon(1)], resolve: [resolve(0), resolve(1)] }));
        }
        let sb = &t.scene.as_ref().unwrap().2;
        // the atmosphere tables, before the volume reads them.
        if o.atmosphere && o.needs_volume() {
            // The transmittance and multiple-scattering tables only when the medium changed (0.3-0.4 ms each frame otherwise).
            let first = {
                let mut s = self.state.lock().unwrap();
                let rebuild = s.atmo_built != Some(s.atmo_medium);
                s.atmo_built = Some(s.atmo_medium);
                if rebuild { 0 } else { 2 }
            };
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("sky-atmosphere"), timestamp_writes: None });
            for k in first..3 {
                pass.set_pipeline(&self.atmo_pipes[k]);
                pass.set_bind_group(0, &self.atmo_bgs[k], &[]);
                pass.dispatch_workgroups(self.atmo_sizes[k].0.div_ceil(8), self.atmo_sizes[k].1.div_ceil(8), 1);
            }
            if o.ap_lut {
                pass.set_pipeline(&self.ap_pipe);
                pass.set_bind_group(0, &self.ap_bg, &[]);
                pass.dispatch_workgroups(AP_SIZE[0].div_ceil(8), AP_SIZE[1].div_ceil(8), 1);
            }
        }
        // Volume.
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("sky-volume"),
                timestamp_writes: tw(Q_SKY_VOLUME).map(|(q, b)| wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(b), end_of_pass_write_index: Some(b + 1) }),
            });
            if o.needs_volume() {
                let half = ((t.size.0 / VOL_DIV).max(1), (t.size.1 / VOL_DIV).max(1));
                pass.set_pipeline(&self.volume_pipe);
                pass.set_bind_group(0, &sb.volume[parity], &[]);
                pass.dispatch_workgroups(half.0.div_ceil(8), half.1.div_ceil(8), 1);
                // every full-resolution pixel from its quarter's trace and its history.
                pass.set_pipeline(&self.recon_pipe);
                pass.set_bind_group(0, &sb.recon[parity], &[]);
                pass.dispatch_workgroups(t.size.0.div_ceil(8), t.size.1.div_ceil(8), 1);
            }
        }
        // Resolve into history[parity].
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sky-resolve"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &t.hist[parity], depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: tw(Q_SKY_RESOLVE).map(|(q, b)| wgpu::RenderPassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(b), end_of_pass_write_index: Some(b + 1) }),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.resolve_pipe);
            pass.set_bind_group(0, &sb.resolve[parity], &[]);
            pass.draw(0..3, 0..1);
        }
        // Bloom.
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("sky-bloom"),
                timestamp_writes: tw(Q_SKY_BLOOM).map(|(q, b)| wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(b), end_of_pass_write_index: Some(b + 1) }),
            });
            if o.bloom {
                let bgs = &t.bloom_bgs[parity];
                let sizes: Vec<(u32, u32)> = (0..BLOOM_LEVELS).map(|i| ((t.size.0 >> (i + 1)).max(1), (t.size.1 >> (i + 1)).max(1))).collect();
                pass.set_pipeline(&self.bloom_pipes[0]);
                pass.set_bind_group(0, &bgs[0], &[]);
                pass.dispatch_workgroups(sizes[0].0.div_ceil(8), sizes[0].1.div_ceil(8), 1);
                pass.set_pipeline(&self.bloom_pipes[1]);
                for i in 1..BLOOM_LEVELS {
                    pass.set_bind_group(0, &bgs[i], &[]);
                    pass.dispatch_workgroups(sizes[i].0.div_ceil(8), sizes[i].1.div_ceil(8), 1);
                }
                pass.set_pipeline(&self.bloom_pipes[2]);
                for (k, i) in (0..BLOOM_LEVELS - 1).rev().enumerate() {
                    pass.set_bind_group(0, &bgs[BLOOM_LEVELS + k], &[]);
                    pass.dispatch_workgroups(sizes[i].0.div_ceil(8), sizes[i].1.div_ceil(8), 1);
                }
            }
        }
        drop(tg);
        let mut s = self.state.lock().unwrap();
        let _ = history_valid;
        s.history_valid = true;
        // this frame drew the cut (see `cut_hold`).
        s.cut_hold = false;
        s.recut = s.recut.saturating_sub(1);
        s.parity = 1 - parity;
        Some(parity)
    }
}

/// The weather in force for options and a world clock.
pub fn weather_of(o: &SkyOptions, world_s: f64) -> eo_sky::WeatherState {
    match o.weather {
        WeatherMode::Preset(k) => eo_sky::weather::preset(k),
        WeatherMode::Schedule => eo_sky::weather::state_at(o.seed, world_s),
    }
}

/// A key that changes when the weather changes enough to rebuild the look
/// frame (the JS regrades when wetness moves by 0.02).
/// the key on the clouds at a setting sun (deep orange, ~1.3 x the rig's key at +3 deg).
/// the atmosphere tables' scale in the look's units: the sky's
/// radiance per unit solar illuminance x this puts a clear noon zenith at the
/// JS dome's luminance (~0.15). The aerosol density over the standard
/// atmosphere's (coastal marine air, ~25-40 km visibility in clear weather),
/// and the sun's scale on the clouds (the rig's noon key, ~7).
pub const ATMO_SKY_SCALE: f64 = 11.0;
/// the volume's resolution divisor (1: full resolution; the half-resolution volume was 2).
pub const VOL_DIV: u32 = 2;
/// the sun's key light reaching the ground through the atmosphere, past the rig's
/// horizon envelope (the pinned `key_envelope` is 0 at 0 deg, so at 19.1 h the sea had no
/// glint under a sun the sky still draws): the rig table's unenveloped key x the direct
/// beam at the refracted elevation (Bennett), reddened by Rayleigh over the air mass, x the
/// weather's mean beam. rgb in key units, w the refracted elevation's sine.
pub fn sun_glint(hour: f64, beam: f64) -> [f32; 4] {
    use eo_look::rig;
    let alt = eo_look::sun::sun_direction(hour).altitude_deg;
    let a = alt.max(-1.5);
    let app = alt + 1.0 / ((a + 7.31 / (a + 4.4)).to_radians()).tan() / 60.0;
    if app <= -0.27 {
        return [0.0; 4];
    }
    // the unenveloped key of the nearest rig row with the sun up
    let mut best = (f64::MAX, 0.0, [1.0, 1.0, 1.0]);
    for r in rig::table() {
        let s = eo_look::sun::sun_direction(r.hour).altitude_deg;
        let e = rig::key_envelope(s) * rig::direct_beam_transmittance(s);
        let dh = (r.hour - hour).abs();
        if e > 0.02 && dh < best.0 {
            best = (dh, r.sun_intensity / e, r.sun_color);
        }
    }
    let e0 = app.max(0.0);
    let am = 1.0 / (e0.to_radians().sin() + 0.50572 * (e0 + 6.07995).powf(-1.6364));
    let t = 0.7f64.powf(am.powf(0.678)) * smoothstep_f(-0.27, 0.27, app);
    let ray = [5.802e-3, 13.558e-3, 33.1e-3];
    let g = (-ray[1] * 8.0 * am).exp();
    let c = [0, 1, 2].map(|i| best.2[i] * (-ray[i] * 8.0 * am).exp() / g.max(1e-9));
    let k = best.1 * t * beam;
    [(c[0] * k) as f32, (c[1] * k) as f32, (c[2] * k) as f32, app.to_radians().sin() as f32]
}

fn smoothstep_f(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub const ATMO_HAZE: f64 = 0.55; // aerosol density scale; at 0.8 the long horizontal path drew a clipped white band ~1.5 deg tall along the horizon ~30 deg either side of a low sun
pub const ATMO_CLOUD_SUN: f64 = 12.0; // 12 (8): a sunlit cumulus face was 1-1.9 x the sky beside it
/// the aerosol extinction of clear coastal air at the ground for the
/// aerial perspective, per km (visibility ~ 3.912 / (0.06 + Rayleigh 0.013) ~ 55
/// km: San Francisco's clear-day 30-60 km); thinning with height (400 m).
pub const AP_HAZE_PER_KM: f64 = 0.06;

pub const SUNSET_CLOUD_KEY: [f64; 3] = [2.6, 1.0, 0.32];

pub fn weather_key(w: &eo_sky::WeatherState) -> (i64, i64, i64) {
    ((w.severity * 50.0).round() as i64, (w.wetness * 50.0).round() as i64, w.kind as i64)
}
