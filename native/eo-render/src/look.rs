//! The look on the GPU: the JS game's light rig, sky, sun shadows and image
//! pipeline (the numbers and curves come from `eo-look`).
//!
//! Per frame, in order (all recorded by `Renderer::encode_with_ids`):
//!
//! 1. **Shadow cull** (compute, after the camera's LOD + cull pass, which
//!    decides every LOD group's level): one invocation per object against the
//!    three cascade boxes, appending to one indirect list per cascade.
//! 2. **Shadow depth**, one depth-only pass per cascade layer of a
//!    `Depth32Float` 2D array, each ONE `multi_draw_indexed_indirect_count`
//!    over its list: the same GPU-driven path as the city draw, no per-object
//!    CPU work.
//! 3. The **city pass** renders into a `Rgba16Float` HDR target with
//!    `look_light.wgsl` (bind group 1), then the **sky** (a full-screen
//!    triangle at the far plane, only where nothing was drawn), then the
//!    transparent list.
//! 4. **Post**: one full-screen pass from the HDR target to the output
//!    (shoulder, ACES, sRGB, grade, look finish, dither).
//!
//! The CPU writes three small uniforms a frame (the look, the cull boxes, the
//! cascade matrices); the sky model and light rig are rebuilt only when the
//! time of day changes.

use crate::camera::Camera;
use eo_look::shadow::{self, CASCADES, CascadeCamera};
use eo_look::{LookFrame, tone};
/// The look profile and the cinematic look's deviations (eo-look `cinematic`; `--look`, stage L0).
pub use eo_look::cinematic::{CINEMATIC_DEVIATIONS, Deviation, LookProfile};
use std::sync::Mutex;

pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// 16-bit is enough along the light (900 m range: 1.4 cm steps against
/// 5-10 cm texels and their bias) and halves the depth bandwidth.
pub const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth16Unorm;
/// Timestamp slots this module writes after the renderer's own four.
pub const Q_SHADOW_CULL: u32 = 4;
pub const Q_SHADOW_DRAW: u32 = 6;
pub const Q_POST: u32 = 8;
/// the occlusion + bounce compute pass.
pub const Q_AO: u32 = 10;
pub const AO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const AO_Z_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
const VIEW_STRIDE: u64 = 256;
/// Counter words: commands per cascade [0..3), triangles per cascade [3..6), pad.
pub const SHADOW_COUNTER_WORDS: usize = 8;

/// Default hour when nobody asks: noon.
pub const DEFAULT_HOUR: f64 = 12.0;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LookUniform {
    key_dir: [f32; 4],
    key: [f32; 4],
    rim_dir: [f32; 4],
    rim: [f32; 4],
    hemi_sky: [f32; 4],
    hemi_ground: [f32; 4],
    ambient: [f32; 4],
    env_class: [f32; 4],
    sh: [[f32; 4]; 9],
    fog: [f32; 4],
    fog_color: [f32; 4],
    cam_pos: [f32; 4],
    cam_fwd: [f32; 4],
    inv_view_proj: [[f32; 4]; 4],
    cascade_vp: [[[f32; 4]; 4]; 3],
    cascade_split: [f32; 4],
    cascade_bias: [[f32; 4]; 3],
    sun: [f32; 4],
    beta_r: [f32; 4],
    beta_m: [f32; 4],
    beta_sum: [f32; 4],
    mean: [f32; 4],
    tw: [f32; 4],
    tw2: [f32; 4],
    horizon: [f32; 4],
    ground: [f32; 4],
    dg0: [f32; 4],
    dg1: [f32; 4],
    dg2: [f32; 4],
    haze_tint: [f32; 4],
    veil_tint: [f32; 4],
    disc: [f32; 4],
    post0: [f32; 4],
    post1: [f32; 4],
    post2: [f32; 4],
    shadow_tint: [f32; 4],
    highlight_tint: [f32; 4],
    // see look_common.wgsl.
    ao_vp: [[f32; 4]; 4],
    ao_cam: [f32; 4],
    ao_fwd: [f32; 4],
    ao_params: [f32; 4],
    field: [f32; 4],
    // the sky model's zenith radiance (computeSkyModel's zenithRadiance),
    // with `horizon` the two colours of the JS `skyDomeRadiance`.
    dome_zenith: [f32; 4],
    // the room drive (rooms.rs `drive`): (dusk, occupancy, shop occupancy, cool share), (office share, pixel angle).
    rooms_env: [f32; 4],
    rooms_work: [f32; 4],
    // the weather's dome (`upperFromRow`): overcast, isotropy, desaturation; tint.
    wx: [f32; 4],
    wx_tint: [f32; 4],
    cove: [f32; 4],
    // the lighting additions (`LightingOptions`), all zero while every one is off.
    // Rows (`LX_*`; mirrored in look_common.wgsl):
    //   0      flags: x the feature bits (`LightingOptions::flags_word`, read with
    //          bitcast<u32>), y z w the ground bounce's sun exitance (0: off)
    //   1-8    far shadow (far_shadow.rs, far_shadow.wgsl): per ring (rows 1-4,
    //          5-8) the three affine rows of world -> its map's clip space,
    //          then (front layer, normal offset, tap offset, fade-out distance)
    //   9      far shadow params (rings drawn, edge fade, radial fade start share)
    //   10-11  ambient volume: origin, scale, plane mapping
    //   12     probe grid params
    //   13     light shadow params
    //   14     smooth time params
    //   15     spare
    // Rows 1-15 are what the features set (`LookGpu::set_lighting_params`); row 0 x is written from the options.
    lx: [[f32; 4]; LX_LEN],
}

/// Bytes of the look uniform (the WGSL `Look` struct), and where its `lx` rows start.
pub const LOOK_UNIFORM_BYTES: usize = std::mem::size_of::<LookUniform>();
pub const LOOK_LX_OFFSET: usize = std::mem::offset_of!(LookUniform, lx);

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ShadowCullParams {
    planes: [[f32; 4]; 18],
    object_count: u32,
    compact: u32,
    mask: u32,
    pad: u32,
}

/// The JS refresh rotation (src/render/cascade-refresh.js, 'staggered'): the
/// near cascade every other frame, the middle and far ones every fourth, so
/// steady state is ONE shadow pass a frame. A camera jump (or a new clock)
/// refits all three at once (the module's pose convergence).
pub const REFRESH_SCHEDULE: [usize; 4] = [0, 1, 0, 2];
/// A camera move larger than this between two updates refreshes every cascade.
pub const CONVERGE_JUMP_M: f32 = 4.0;

fn v4(a: [f64; 3], w: f64) -> [f32; 4] {
    [a[0] as f32, a[1] as f32, a[2] as f32, w as f32]
}

/// Quality settings (`--shadow-map`, `--no-shadows`).
#[derive(Clone, Copy, Debug)]
pub struct LookOptions {
    pub shadow_map: u32,
    pub shadows: bool,
    /// scene-scale occlusion + one bounce (`--no-ao` turns it off).
    pub ao: bool,
    /// the facade pass's switches (`facade::RUN_*`; `--no-facade-age`, `--no-facade-detail`, `--no-atlas2`, `--no-probe`).
    pub facade: u32,
    /// clouds, weather, night lights and the camera finish (sky.rs).
    pub sky: crate::sky::SkyOptions,
    /// the water pass (`--no-water`: the flat sea quad, as before).
    pub water: bool,
    /// Opt-in wave-driven underwater lighting and camera.
    pub water_optics: bool,
    /// Compensate the camera for daylight lost along the path down to the eye.
    pub dive_exposure: bool,
    /// Local water attenuation in the Aquatic Park Cove preview.
    pub cove_detail: bool,
    pub cove_canopy: bool,
    pub cove_sediment: bool,
    /// the CC0 scanned sets' switches (`cc0::RUN_*`; `--no-cc0` and the per-feature `--no-cc0-*`).
    pub cc0: u32,
    /// The opt-in lighting additions (`--quality`, `--cascade-blend`, ...), all off by default.
    pub lighting: LightingOptions,
    /// The sun cascades' casters drawn with a slope-scaled depth bias
    /// (`--shadow-slope-bias`; `SLOPE_BIAS_SCALE`): surfaces the sun grazes
    /// stop shadowing themselves in stripes. Off: the maps as before.
    pub shadow_slope_bias: bool,
    /// Surface weathering that reads (`--grime 1|2`, stage W1; weathering.rs):
    /// 0 off (the frame as before), 1 high, 2 hyper. Carried to the shaders in
    /// the frame switches (`facade::grime_bits`).
    pub grime: u8,
    /// Street life (`--street-life 1|2`, stage W2b; street_life.rs): litter
    /// and leaves on the walks and in the gutters, graffiti and posters on
    /// ground floors and freeway columns. 0 off (the frame as before), 1 high,
    /// 2 hyper. Carried in the frame switches (`facade::street_life_bits`).
    pub street_life: u8,
    /// The photographic finish (`--photo-finish 1|2`, stage W4; photo.rs,
    /// look_photo.wgsl): a camera's response, sensor noise and lens in the post
    /// pass, San Francisco's measured air in the sky's tables and aerial
    /// perspective, a lens's bloom (sky.rs). 0 off (the post pass and the sky as
    /// before), 1 high, 2 hyper.
    pub photo_finish: u8,
    /// The look profile (`--look default|cinematic`, `--no-look`; eo-look
    /// `cinematic`): `Default` is today's frame; `Cinematic` the graded look the
    /// high and hyper tiers turn on. Its shader code sits behind `SF_CINEMATIC`
    /// (`ShaderFeatures::cinematic`); the sky reads `SkyOptions::cinematic`.
    pub look: eo_look::cinematic::LookProfile,
    /// The sun's first bounce off the sunlit walls round each point
    /// (`--sun-bounce`; surfaces_ao_sun.wgsl, eo-look `sun_bounce`): the
    /// occlusion pass gathers it from the far sun map's ring 1 as a reflective
    /// shadow map, each sunlit wall's radiance less the rig's stand-in for it,
    /// into the bounce channel. Needs `lighting.far_shadows` and `ao`. Off: the
    /// frame as before (its code is compiled out, `ShaderFeatures::sun_bounce`).
    pub sun_bounce: bool,
    /// The occlusion's multi-bounce at each surface's own diffuse albedo, per
    /// channel, on the indirect diffuse (`--ao-albedo`; surfaces_ao_use.wgsl
    /// `ao_multi_bounce_albedo`) instead of the JS's single albedo of 0.7.
    /// Off: the frame as before (compiled out, `ShaderFeatures::ao_albedo`).
    pub ao_albedo: bool,
    /// The sun's penumbra (`--soft-shadows`; look_light.wgsl
    /// `lk_cascade_soft`, eo-look `penumbra`): the cascades' comparison
    /// filtered over the sun's disc (0.5334 degrees) at the distance of the
    /// edge that casts it, found from the casters' depth round the point. Off:
    /// the frame as before (compiled out, `ShaderFeatures::soft_shadows`).
    pub soft_shadows: bool,
    /// The screen-space indirect light (`--screen-gi`; surfaces_ao_gi.wgsl,
    /// surfaces_ao_gi_filter.wgsl, eo-look `screen_gi`): the occlusion pass
    /// as a visibility-bitmask gather out to 12 m (each sample a surface of
    /// some thickness, so a rail or a pole hides only its own sliver), blended
    /// into a reprojected history and filtered over its plane, its bounce
    /// the light of the walls and streets round each point in place of the
    /// rig's. Needs `ao`. Off: the frame as before (the gather, the history
    /// and the filter are compiled out, `ShaderFeatures::screen_gi`).
    pub screen_gi: bool,
    /// The cars' materials and lamps (`--car-materials`; agents_draw.wgsl
    /// `ag_car_*`, eo-agents `CookOptions::car_materials`): 0 off (the frame
    /// as before), 1 on. Its shader code sits behind `SF_CAR_MATERIALS`
    /// (`ShaderFeatures::car_materials`); the agents read the level from
    /// their parameters (agents.rs `AgParams::misc.y`).
    pub car_materials: u8,
    /// The cars' occlusion of the street round them (`--car-contact`;
    /// eo-render agents_contact.rs and .wgsl, a pass of its own after the city
    /// pass, made only when on): 0 off (no pass, the agents' module as
    /// before), 1 on.
    pub car_contact: u8,
    /// The cars' reflections of the street's brightest lights at any distance
    /// (`--car-reflections`; agents_draw.wgsl `ag_car_far_lights`, with
    /// `--car-materials` only): 0 off (the frame as before), 1 on. Its shader
    /// code sits behind `SF_CAR_REFLECTIONS` (`ShaderFeatures::car_reflections`).
    pub car_reflections: u8,
    /// The natural-detail shading (`NATURE_*` bits; eo-app `--foliage-light`,
    /// `--temporal-lod-fade`, `--turf`, `--palm-detail`, `--hill-cover`, `--ground-cover`): 0 (the default) is the frame as before,
    /// and each bit's shader code is compiled out while it is off (`sf_select`).
    pub nature: u32,
}

/// `--foliage-light`: leaves and palm fronds pass light through (props_draw.wgsl
/// `pr_foliage`, hlod_trees.wgsl `ft_foliage`).
pub const NATURE_FOLIAGE: u32 = 1;
/// `--temporal-lod-fade`: the level cross-fades' dither moves every frame, so the
/// temporal anti-aliasing resolves it to a blend (props_draw.wgsl, hlod_trees.wgsl).
pub const NATURE_LOD_FADE: u32 = 2;
/// `--turf`: lawns shaded as a grass canopy (landmarks_surface.wgsl `nt_turf`).
pub const NATURE_TURF: u32 = 4;
/// `--palm-detail`: the palms' trunk relief and crown shading (props_draw.wgsl `pr_palm`).
pub const NATURE_PALM: u32 = 8;
/// `--hill-cover`: the far hills' vegetation cover (farshore.wgsl `fs_cover`).
pub const NATURE_HILLS: u32 = 16;
/// `--ground-cover`: the city's hills and wild parks as a mosaic of grass, brush,
/// groves and chert, their crowns drawn or shared out by the geometric-optical model
/// (landmarks_surface.wgsl `nt_cover`), and their tones without the per-vertex
/// jitter (ground2.rs `hill_tone`).
pub const NATURE_COVER: u32 = 32;
/// `--street-wear` (the street track, round 2; street_pbr.wgsl `sw_*`): the
/// carriageway's distress as laid out by its causes (transverse and lane-joint
/// cracks, fatigue cracking in the wheel paths, sealed with meandering tar
/// snakes) in place of the weathering's crack web, the oil drips as a field of
/// spots down the lane, and utility patches of one pour.
pub const STREET_WEAR: u32 = 1;
/// `--street-glare` (street_pbr.wgsl `sg_*`): the polished binder on the
/// aggregate's tops as a smooth layer over the rough road, its share growing
/// toward grazing as the texture's valleys are masked (the low-sun glare down
/// a road, the wheel paths brightest); the kerb arris and the setts' crowns too.
pub const STREET_GLARE: u32 = 2;
/// Every natural-detail bit.
pub const NATURE_ALL: u32 = NATURE_FOLIAGE | NATURE_LOD_FADE | NATURE_TURF | NATURE_PALM | NATURE_HILLS | NATURE_COVER;

impl Default for LookOptions {
    fn default() -> Self {
        Self { shadow_map: shadow::MAP_SIZE, shadows: true, ao: true, facade: crate::facade::RUN_AGE | crate::facade::RUN_DETAIL | crate::facade::RUN_ATLAS2 | crate::facade::RUN_PROBE | crate::facade::RUN_ROOMS | crate::facade::RUN_AGENT_HZB | crate::facade::RUN_PROBE_DEFER, water: true, water_optics: false, dive_exposure: false, cove_detail: false, cove_canopy: false, cove_sediment: false, sky: Default::default(), cc0: crate::cc0::RUN_DEFAULT, lighting: LightingOptions::OFF, shadow_slope_bias: false, grime: 0, street_life: 0, photo_finish: 0, look: eo_look::cinematic::LookProfile::Default, sun_bounce: false, ao_albedo: false, soft_shadows: false, screen_gi: false, car_materials: 0, car_contact: 0, car_reflections: 0, nature: 0 } // + rooms, agent pyramid test, probe deferral // water // sky
    }
}

/// `--shadow-slope-bias`: the cascades' casters' depth is pushed away from
/// the sun by this many times its slope a texel (wgpu `DepthBiasState`), so
/// the PCF taps (up to a texel and a half from the receiver's own point) of
/// a surface the sun grazes compare it with itself no more. The existing
/// receiver bias (eo-look `recommend_bias`) protects to 75 degrees of
/// incidence at most; past it the I-80 fascia drew 0.4 m stripes (stage A3).
pub const SLOPE_BIAS_SCALE: f32 = 1.5;
/// ... at most this much (depth units: 0.002 of the cascades' 340-560 m
/// light ranges, 0.7-1.1 m), so a wall edge-on to the sun still casts.
pub const SLOPE_BIAS_CLAMP: f32 = 0.002;

/// The opt-in lighting additions, one field per feature (eo-app's
/// `--quality` tiers and per-feature switches fill it). Everything off
/// (`Default`, `OFF`) is the frame as it was before these options existed.
/// Levels are 0 for off; what a level means is the feature's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LightingOptions {
    /// Blend between neighbouring sun cascades instead of switching (`--cascade-blend`).
    pub cascade_blend: bool,
    /// Sun shadows past the cascades: 0 off, 1 one ring, 2 two rings (`--far-shadows`).
    pub far_shadows: u8,
    /// Large-scale sky visibility: 0 off, 1 high (4 m cells, about 320 m), 2 hyper (2 m near, about 512 m) (`--ambient-volume`).
    pub ambient_volume: u8,
    /// Reflection probes: 0 the single cube, 1 cross-fade + 4 cubes, 2 nine cubes at 256 px (`--probe-grid`).
    pub probe_grid: u8,
    /// Shadowed street lamps in sixteens: 0 off, 1 = 16, 2 = 32, 3 = 48 (`--light-shadows N`).
    pub light_shadows: u8,
    /// Lamp shadows include moving casters (`--light-shadow-moving`).
    pub light_shadow_moving: bool,
    /// Headlight shadows for the nearest N vehicles, 0 off (`--headlight-shadows N`).
    pub headlight_shadows: u8,
    /// Fog lit by the local lights: 0 off, 1 coarse, 2 fine (`--lit-fog`).
    pub lit_fog: u8,
    /// The drawn hour follows the simulation clock, with smooth exposure (`--smooth-time`).
    pub smooth_time: bool,
    /// Light bounced up from the sunlit ground onto undersides and walls
    /// facing it, baked into the ambient volume beside the sky's visibility
    /// (`--ground-bounce`; only with `ambient_volume`, whose levels hold it:
    /// eo-app turns it on with the volume unless `--no-ground-bounce`).
    pub ground_bounce: bool,
}

/// Bits of `LightingOptions::mask` (`Look.lx[LX_FLAGS].x`, look_common.wgsl `LX_BIT_*`).
pub const LX_BIT_CASCADE_BLEND: u32 = 1 << 0;
pub const LX_BIT_FAR_SHADOWS: u32 = 1 << 1;
pub const LX_BIT_AMBIENT_VOLUME: u32 = 1 << 2;
pub const LX_BIT_PROBE_GRID: u32 = 1 << 3;
pub const LX_BIT_LIGHT_SHADOWS: u32 = 1 << 4;
pub const LX_BIT_HEADLIGHT_SHADOWS: u32 = 1 << 5;
pub const LX_BIT_LIT_FOG: u32 = 1 << 6;
pub const LX_BIT_SMOOTH_TIME: u32 = 1 << 7;
/// Rows of `Look.lx` (see `LookUniform::lx` for who owns which).
pub const LX_LEN: usize = 16;
/// row 0: x the feature bits; y z w the ground bounce's sun exitance
/// (`ambient::AmbientGpu::update`, eo-look `bounce::sun_exitance`; zero
/// while the bounce is off or the sun is down).
pub const LX_FLAGS: usize = 0;
/// the far shadow rings (rows 1-4 and 5-8): each the three affine rows of
/// world -> its map's clip space and a row of params (far_shadow.wgsl).
pub const LX_FAR_SHADOW_VP: usize = 1;
pub const LX_FAR_SHADOW_PARAMS: usize = 9;
/// two rows (10, 11): the ambient volume's origin, scale and plane mapping.
pub const LX_AMBIENT: usize = 10;
pub const LX_PROBE_GRID: usize = 12;
pub const LX_LIGHT_SHADOW: usize = 13;
pub const LX_SMOOTH_TIME: usize = 14;
pub const LX_SPARE: usize = 15;

impl LightingOptions {
    /// Every feature off.
    pub const OFF: Self = Self { cascade_blend: false, far_shadows: 0, ambient_volume: 0, probe_grid: 0, light_shadows: 0, light_shadow_moving: false, headlight_shadows: 0, lit_fog: 0, smooth_time: false, ground_bounce: false };

    /// Whether any feature is on.
    pub fn any(&self) -> bool {
        *self != Self::OFF
    }

    /// Whether the ground bounce is drawn: asked for, and the ambient volume that holds it on.
    /// (No bit of its own: the shader takes it from `Look.lx[0].yzw`, zero while it is off.)
    pub fn ground_bounce_on(&self) -> bool {
        self.ground_bounce && self.ambient_volume > 0
    }

    /// The feature bits (`LX_BIT_*`); `light_shadow_moving` has none of its own (it refines `light_shadows`).
    pub fn mask(&self) -> u32 {
        let on = [
            (self.cascade_blend, LX_BIT_CASCADE_BLEND),
            (self.far_shadows > 0, LX_BIT_FAR_SHADOWS),
            (self.ambient_volume > 0, LX_BIT_AMBIENT_VOLUME),
            (self.probe_grid > 0, LX_BIT_PROBE_GRID),
            (self.light_shadows > 0, LX_BIT_LIGHT_SHADOWS),
            (self.headlight_shadows > 0, LX_BIT_HEADLIGHT_SHADOWS),
            (self.lit_fog > 0, LX_BIT_LIT_FOG),
            (self.smooth_time, LX_BIT_SMOOTH_TIME),
        ];
        on.iter().filter(|(b, _)| *b).fold(0, |m, (_, bit)| m | bit)
    }

    /// Street lamps that get a shadow map (`light_shadows` x 16).
    pub fn shadowed_lamps(&self) -> u32 {
        self.light_shadows as u32 * 16
    }

    /// `Look.lx[LX_FLAGS].x`: 0.0 with no bit set, else the bits under the
    /// exponent of 1.0, so the value is a normal float that no flush to zero
    /// can lose. The shader reads `bitcast<u32>(look.lx[0].x) & LX_BIT_*`.
    pub fn flags_word(&self) -> f32 {
        let m = self.mask();
        if m == 0 { 0.0 } else { f32::from_bits(0x3F80_0000 | m) }
    }
}

/// the half-resolution occlusion/bounce result and its depth.
struct AoTarget {
    size: (u32, u32),
    ao_view: wgpu::TextureView,
    z_view: wgpu::TextureView,
    /// The compute bind group for one (depth texture, HDR texture) pair.
    bind: Option<(wgpu::Texture, wgpu::Texture, wgpu::BindGroup)>,
    /// (`--screen-gi`) the gather's scratch target, the history and its depth, and their groups.
    gi: Option<AoGiTarget>,
}

/// (`--screen-gi`) the gather writes `tmp` (raw + history blend) and the
/// view depth (`z_view`); the filter reads them into `ao_view`; then `tmp`
/// and `z_view` are copied into the history the next gather reads.
struct AoGiTarget {
    tmp: wgpu::TextureView,
    hist: wgpu::TextureView,
    hist_z: wgpu::TextureView,
    filter_bind: wgpu::BindGroup,
    /// The gather's group for one (depth texture, HDR texture) pair.
    bind: Option<(wgpu::Texture, wgpu::Texture, wgpu::BindGroup)>,
}

/// (`--screen-gi`) the gather and filter pipelines (only when compiled in).
struct AoGiPipelines {
    gather: wgpu::ComputePipeline,
    gather_bgl: wgpu::BindGroupLayout,
    filter: wgpu::ComputePipeline,
    filter_bgl: wgpu::BindGroupLayout,
}

struct State {
    hour: f64,
    /// the scene probe's measured transfer (None: the JS fallback yield).
    transfer: Option<f64>,
    frame: Option<LookFrame>,
    frame_index: u32,
    /// The cascades as last rendered (a cascade keeps its box until it is refreshed).
    last_cascades: [shadow::Cascade; CASCADES],
    /// Cascades refreshed by the coming encode (bit c).
    mask: u32,
    /// Position in `REFRESH_SCHEDULE`, and the eye of the last update.
    rotation: usize,
    last_eye: Option<glam::Vec3>,
    /// Refit every cascade on the next update.
    force_all: bool,
    /// What the last update fitted (cascades as they will be after the coming encode, eye).
    pending: Option<([shadow::Cascade; CASCADES], glam::Vec3)>,
    /// the camera of the last update (view-projection, eye, forward, vertical fov radians).
    cur_cam: Option<(glam::Mat4, glam::Vec3, glam::Vec3, f32)>,
    /// the camera the occlusion result now in the texture was made with.
    ao_made: Option<(glam::Mat4, glam::Vec3, glam::Vec3)>,
    /// the weather and night terms of `frame`, and the weather they were built for.
    terms: Option<eo_sky::SkyTerms>,
    weather_key: (i64, i64, i64),
    /// the weather frame before its fill (hour, sky profile, frame): `eo_sky::frame::base_frame`.
    sky_base: Option<(f64, String, LookFrame)>,
    /// `--smooth-time`: the hour comes through `set_time_of_day_smooth` (the
    /// keyframes below), not `set_time_of_day`.
    soft: bool,
    keys: crate::smooth_time::Keys,
    /// `--smooth-time`: the key direction each cascade was last fitted with.
    cascade_dirs: [Option<[f64; 3]>; CASCADES],
}

struct PostTarget {
    size: (u32, u32),
    view: wgpu::TextureView,
    /// reading the HDR scene, the resolved history 0, history 1.
    bind: [wgpu::BindGroup; 3],
    /// (`--photo-finish`) the same three sources and the bloom, and the photo
    /// pass's bind groups over them (made once its pipeline exists).
    srcs: [wgpu::TextureView; 3],
    bloom: wgpu::TextureView,
    photo_bind: Option<[wgpu::BindGroup; 3]>,
    /// (`--look cinematic`) the cinematic pass's bind groups over them (once its pipeline exists).
    cine_bind: Option<[wgpu::BindGroup; 3]>,
}

/// Group 1 bindings of the lighting additions (declared in look_ext.wgsl).
pub const B_FAR_SHADOW_MAP: u32 = 12;
pub const B_AMBIENT_VOLUME: u32 = 13;
pub const B_AMBIENT_PLANE: u32 = 14;
pub const B_PROBE_GRID: u32 = 15;
pub const B_PROBE_TABLE: u32 = 16;
pub const B_LIGHT_SHADOW_ATLAS: u32 = 17;
pub const B_LIGHT_SHADOW_TABLE: u32 = 18;
/// (`--shop-light`, stage N2) the shop fronts table (shop_lights.rs `fronts_table`).
pub const B_SHOP_FRONTS: u32 = 19;

/// What group 1 bindings 12-19 hold: a feature's real resources, or the
/// 1 x 1 stand-ins made once with the look (`LookGpu::lighting_stand_ins`).
/// The views must match the layout: depth 2D arrays at 12 and 17, a
/// filterable float 3D texture at 13, 2D at 14, 2D array at 15 (6 layers a
/// probe), read-only storage `array<vec4<f32>>` at 16 and 18, and
/// `array<vec4<u32>>` at 19 (its stand-in is 16's: zeros, no fronts).
#[derive(Clone, Debug)]
pub struct LightingBindings {
    pub far_shadow_map: wgpu::TextureView,
    pub ambient_volume: wgpu::TextureView,
    pub ambient_plane: wgpu::TextureView,
    pub probe_grid: wgpu::TextureView,
    pub probe_table: wgpu::Buffer,
    pub light_shadow_atlas: wgpu::TextureView,
    pub light_shadow_table: wgpu::Buffer,
    pub shop_fronts: wgpu::Buffer,
}

impl LightingBindings {
    /// The stand-ins: 1 x 1 texels (zero), one-matrix tables (zero). Returns them and their bytes.
    fn stand_ins(device: &wgpu::Device) -> (Self, u64) {
        let tex = |label: &str, dimension: wgpu::TextureDimension, layers: u32, format: wgpu::TextureFormat, view: wgpu::TextureViewDimension| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: layers },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor { dimension: Some(view), ..Default::default() })
        };
        let table = |label: &str| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: 64, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false });
        let (d2, d3) = (wgpu::TextureDimension::D2, wgpu::TextureDimension::D3);
        let f16 = wgpu::TextureFormat::Rgba16Float;
        let probe_table = table("probe-table-stand-in");
        let b = Self {
            far_shadow_map: tex("far-shadow-stand-in", d2, 1, SHADOW_FORMAT, wgpu::TextureViewDimension::D2Array),
            ambient_volume: tex("ambient-volume-stand-in", d3, 1, f16, wgpu::TextureViewDimension::D3),
            ambient_plane: tex("ambient-plane-stand-in", d2, 1, f16, wgpu::TextureViewDimension::D2),
            probe_grid: tex("probe-grid-stand-in", d2, 6, f16, wgpu::TextureViewDimension::D2Array),
            // (stage N2) the shop fronts' stand-in is the probe table's (zeros: no fronts; no new allocation).
            shop_fronts: probe_table.clone(),
            probe_table,
            light_shadow_atlas: tex("light-shadow-stand-in", d2, 1, SHADOW_FORMAT, wgpu::TextureViewDimension::D2Array),
            light_shadow_table: table("light-shadow-table-stand-in"),
        };
        (b, 2 * 2 + 8 + 8 + 6 * 8 + 2 * 64)
    }

    fn entries(&self) -> [wgpu::BindGroupEntry<'_>; 8] {
        use wgpu::BindingResource::TextureView as T;
        [
            wgpu::BindGroupEntry { binding: B_FAR_SHADOW_MAP, resource: T(&self.far_shadow_map) },
            wgpu::BindGroupEntry { binding: B_AMBIENT_VOLUME, resource: T(&self.ambient_volume) },
            wgpu::BindGroupEntry { binding: B_AMBIENT_PLANE, resource: T(&self.ambient_plane) },
            wgpu::BindGroupEntry { binding: B_PROBE_GRID, resource: T(&self.probe_grid) },
            wgpu::BindGroupEntry { binding: B_PROBE_TABLE, resource: self.probe_table.as_entire_binding() },
            wgpu::BindGroupEntry { binding: B_LIGHT_SHADOW_ATLAS, resource: T(&self.light_shadow_atlas) },
            wgpu::BindGroupEntry { binding: B_LIGHT_SHADOW_TABLE, resource: self.light_shadow_table.as_entire_binding() },
            wgpu::BindGroupEntry { binding: B_SHOP_FRONTS, resource: self.shop_fronts.as_entire_binding() },
        ]
    }

    /// The layout entries (fragment only).
    pub(crate) fn layout_entries() -> [wgpu::BindGroupLayoutEntry; 8] {
        let fs = wgpu::ShaderStages::FRAGMENT;
        let e = |binding, ty| wgpu::BindGroupLayoutEntry { binding, visibility: fs, ty, count: None };
        let tex = |sample_type, view_dimension| wgpu::BindingType::Texture { sample_type, view_dimension, multisampled: false };
        let float = wgpu::TextureSampleType::Float { filterable: true };
        let table = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None };
        [
            e(B_FAR_SHADOW_MAP, tex(wgpu::TextureSampleType::Depth, wgpu::TextureViewDimension::D2Array)),
            e(B_AMBIENT_VOLUME, tex(float, wgpu::TextureViewDimension::D3)),
            e(B_AMBIENT_PLANE, tex(float, wgpu::TextureViewDimension::D2)),
            e(B_PROBE_GRID, tex(float, wgpu::TextureViewDimension::D2Array)),
            e(B_PROBE_TABLE, table),
            e(B_LIGHT_SHADOW_ATLAS, tex(wgpu::TextureSampleType::Depth, wgpu::TextureViewDimension::D2Array)),
            e(B_LIGHT_SHADOW_TABLE, table),
            e(B_SHOP_FRONTS, table),
        ]
    }
}

pub struct LookGpu {
    pub options: LookOptions,
    pub(crate) output_format: wgpu::TextureFormat,
    object_count: u32,
    compact: bool,
    look_buf: wgpu::Buffer,
    cull_buf: wgpu::Buffer,
    view_buf: wgpu::Buffer,
    shadow_draws: wgpu::Buffer,
    shadow_counters: wgpu::Buffer,
    /// The cascades' lists in the count emulation (counter word c, list c).
    shadow_lists: Option<crate::counted::CountedLists>,
    shadow_layers: Vec<wgpu::TextureView>,
    main_bgl: wgpu::BindGroupLayout,
    /// one per history parity (the previous resolved frame differs).
    main_bg: Mutex<[wgpu::BindGroup; 2]>,
    pub(crate) shadow_array: wgpu::TextureView, // pub(crate)
    // occlusion + bounce.
    ao_pipeline: wgpu::ComputePipeline,
    ao_bgl: wgpu::BindGroupLayout,
    /// (`--sun-bounce`) the occlusion pass has the sun bounce compiled in: its binding 5 is the far sun map.
    ao_sun: bool,
    /// (`--screen-gi`) the gather and its filter, when compiled in.
    ao_gi: Option<AoGiPipelines>,
    ao_sampler: wgpu::Sampler,
    ao_dummy: (wgpu::TextureView, wgpu::TextureView),
    ao: Mutex<Option<AoTarget>>,
    cull_pipeline: wgpu::ComputePipeline,
    cull_bg: wgpu::BindGroup,
    depth_pipeline: wgpu::RenderPipeline,
    /// `--shadow-slope-bias`: the same pass with the slope-scaled caster bias.
    depth_pipeline_sloped: wgpu::RenderPipeline,
    depth_bg: wgpu::BindGroup,
    sky_pipeline: wgpu::RenderPipeline,
    sky_bg: wgpu::BindGroup,
    post_pipeline: wgpu::RenderPipeline,
    post_bgl: wgpu::BindGroupLayout,
    post: Mutex<Option<PostTarget>>,
    /// (`--photo-finish`) the photo pass (photo.rs), made the first time the option is on.
    photo: Option<crate::photo::PhotoGpu>,
    /// (`--look cinematic`) the cinematic post pass (cine_post.rs), made the first time the look is on.
    cine: Option<crate::cine_post::CineGpu>,
    state: Mutex<State>,
    pub(crate) cmp_sampler: wgpu::Sampler, // pub(crate)
    pub(crate) device: wgpu::Device, // pub(crate) (the probe rebinds group 0)
    /// the sky, weather, night lights and camera finish passes.
    pub sky: crate::sky::SkyGpu,
    sky_depth_ok: std::sync::atomic::AtomicBool, // sky
    /// what post reads: 0 the HDR scene, 1 + p the resolved history p.
    post_source: std::sync::atomic::AtomicU32,
    /// group 1 bindings 12-18: the stand-ins (made once) and what is bound now.
    lighting_stand_ins: LightingBindings,
    lighting_bound: Mutex<LightingBindings>,
    /// `Look.lx` rows as the lighting features set them (row 0 x is the feature mask, written by `update`).
    lx: Mutex<[[f32; 4]; LX_LEN]>,
    /// Bytes this module allocated on the GPU (shadow map, lists, uniforms; the
    /// HDR target is counted when it exists).
    pub bytes: u64,
}

/// WGSL of the city shader: draw.wgsl + the look's shared code and lighting.
pub fn city_shader_source() -> String {
    city_shader_source_with(crate::ambient::ShaderLevels::Full)
}

/// The city shader for the far layer's pipelines (hlod cells, far trees,
/// far lights): the ambient volume reads only its citywide plane there (the
/// far cells lie beyond the volume's levels, and their fragment shader stands
/// at the edge of what Apple's compiler takes: `ambient::ShaderLevels`), and
/// the sun's shadow is the far map's alone (`ShadowLevels::Far`).
pub fn city_shader_source_far() -> String {
    city_shader_source_variant(crate::ambient::ShaderLevels::Plane, ShadowLevels::Far)
}

/// The city shader with `ambient` of the ambient volume compiled in (the
/// full shadow lookup with the full volume, `ShadowLevels::Near` otherwise:
/// the props).
pub fn city_shader_source_with(ambient: crate::ambient::ShaderLevels) -> String {
    let shadow = if ambient == crate::ambient::ShaderLevels::Full { ShadowLevels::Full } else { ShadowLevels::Near };
    city_shader_source_variant(ambient, shadow)
}

/// What a pipeline's `lk_shadow` (look_light.wgsl) compiles, by the
/// constants `LK_SHADOW_CASCADES`, `LK_SHADOW_BLEND`, `LK_SHADOW_FAR`. On
/// Apple's GPU compiler the props' cut-outs and the far cells stand at the
/// edge of what compiles (stage B3), so they take less. Measured (stage B2,
/// every pipeline built): `prop-cutout` takes the cascades' blend but not the
/// far map (not even one compare with its three matrix rows), `hlod-draw`
/// takes the far map in place of the cascades.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadowLevels {
    /// The cascades, their blend (`--cascade-blend`) and the far map past
    /// them (`--far-shadows`): the city and its probe face, agents, landmark
    /// wires, CC0.
    Full,
    /// The cascades and their blend: the props (none stands past 300 m; past
    /// the cascades' 160 m they stay unshadowed, as before).
    Near,
    /// The far map, no cascades: the far layer (cells, far trees, far
    /// lights), every pixel of which lies past the cascades' 160 m of view
    /// depth (cells past 320 m, far trees past 270 m from the eye), where the
    /// cascades' lookup returned 1 before.
    Far,
}

impl ShadowLevels {
    /// (cascades, blend, far map) compiled in.
    pub fn parts(self) -> (bool, bool, bool) {
        match self {
            ShadowLevels::Full => (true, true, true),
            ShadowLevels::Near => (true, true, false),
            ShadowLevels::Far => (false, false, true),
        }
    }
}

/// The shadow lookup's constants for a variant (look_light.wgsl `lk_shadow`).
pub fn shadow_wgsl(levels: ShadowLevels) -> String {
    let (c, b, f) = levels.parts();
    format!(
        "// generated by look.rs shadow_wgsl\nconst LK_SHADOW_CASCADES: bool = {c};\nconst LK_SHADOW_BLEND: bool = {b};\nconst LK_SHADOW_FAR: bool = {f};\nconst LK_CASCADE_BLEND_BAND: f32 = {:.6};\nconst LK_SHADOW_SOFT: bool = {};\n",
        shadow::CASCADE_BLEND_BAND,
        // (`--soft-shadows`) the penumbra's search and filter in the full variant only: the
        // props' cut-out pipeline stands at the edge of what Apple's compiler takes, and with
        // them failed (XPC_ERROR_CONNECTION_INTERRUPTED, measured); props keep the tent.
        levels == ShadowLevels::Full
    )
}

/// The opt-in features whose shader code is compiled into the pipelines
/// (stage P1). A feature that is off at start has its code compiled out of
/// every pipeline (generated `const SF_*` in `look_common_source`, read by
/// each feature's gate: `lk_lx_on`, `wx_level`, `sl_level`, `sx_in_bore`,
/// the structures' concrete, the shop lights, the facade clutter, the lamp
/// and headlight shadow test), so the frame with the feature off costs what
/// it cost before the feature existed: an untaken uniform branch still costs
/// every pixel in registers and occupancy on Apple's GPU (measured: the city's
/// early opaque draw 0.65 ms before the features, 4.0 ms with all of them in
/// and off, street_walk). With the feature on, its gate is the runtime test
/// it was before, so the output is the same.
///
/// The set is process-wide (`set_shader_features`), chosen by the app from
/// its options before the renderer and its pipelines are made; until it is set
/// every feature is compiled in (`ShaderFeatures::ALL`, the shaders as they
/// were before this set existed: what the renderer's own tests compose).
/// Pipelines are made once, so a feature compiled out stays off until the
/// next start (`limit` keeps a later options change from asking for it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShaderFeatures {
    /// The lighting additions' bits compiled in (`LX_BIT_*`; `LightingOptions::mask`).
    pub lx_bits: u32,
    /// Surface weathering (`--grime`; weathering.wgsl `wx_level`).
    pub grime: bool,
    /// Street life (`--street-life`; street_life.wgsl `sl_level`).
    pub street_life: bool,
    /// The grade-separated roads' concrete (`--structures`; draw.wgsl kind 12).
    pub structures: bool,
    /// Tunnel bores (`--tunnels`; structures.wgsl `sx_in_bore`).
    pub tunnels: bool,
    /// The shop windows' lights and rooms (`--shop-light`; draw.wgsl `fc_lit`, rooms.wgsl).
    pub shop_light: bool,
    /// Facade clutter: lit sign faces and their lettering (`--facade-clutter`; draw.wgsl `shade`).
    pub facade_clutter: bool,
    /// The cinematic look (`--look cinematic`; `SF_CINEMATIC`): every shader line of the
    /// cinematic look plan sits behind it, as well as behind its runtime test.
    pub cinematic: bool,
    /// The kit glazing as a two-surface dielectric pane over its room, with the
    /// rooms' office depth, ceilings and window coverings (`--facade-glass`;
    /// draw.wgsl, rooms.wgsl `SF_FACADE_GLASS`; eo-render `facade::glass_from_args`).
    pub facade_glass: bool,
    /// The far layer's facades and roofs (`--far-facades`; hlod_draw.wgsl
    /// `SF_FAR_FACADES`; eo-render `facade::far_from_args`).
    pub far_facades: bool,
    /// The kit walls' run-off under each window and their soiled foot
    /// (`--facade-weather`; draw.wgsl, facade_age.wgsl `SF_FACADE_WEATHER`;
    /// eo-render `facade::weather_from_args`).
    pub facade_weather: bool,
    /// The street canyon in the kit glass: the sky over the opposite roofline,
    /// the opposite facades in sun or shade and the street, cast from the
    /// ambient volume's canyon (`--canyon-glass`; draw.wgsl, look_light.wgsl,
    /// rooms.wgsl `SF_CANYON_GLASS`; eo-render `facade::canyon_from_args`).
    pub canyon_glass: bool,
    /// The street's physically based surface pass (`--street-pbr`; street_pbr.wgsl,
    /// `SF_STREET_PBR`): compiled in only with the switch (eo-app sets it; the look's
    /// options do not carry it, and `ALL` leaves it out), and then always on.
    pub street_pbr: bool,
    /// The street track's round-2 passes (`STREET_*` bits: `--street-wear`,
    /// `--street-glare`; `SF_STREET_WEAR`, `SF_STREET_GLARE`): as `street_pbr`,
    /// compiled in only with their switches (eo-app sets them; `ALL` leaves them out).
    pub street: u32,
    /// The sun bounce (`--sun-bounce`; `SF_SUN_BOUNCE`): the occlusion pass's
    /// gather (surfaces_ao_sun.wgsl, its binding 5; `ao_shader_source`).
    pub sun_bounce: bool,
    /// The occlusion's multi-bounce at the surface's albedo (`--ao-albedo`; `SF_AO_ALBEDO`, look_light.wgsl).
    pub ao_albedo: bool,
    /// The sun's penumbra (`--soft-shadows`; `SF_SOFT_SHADOWS`, look_light.wgsl `lk_cascade_soft`).
    pub soft_shadows: bool,
    /// The screen-space indirect light (`--screen-gi`; `SF_SCREEN_GI`): the
    /// occlusion pass's gather, history and filter (`ao_shader_source`, `ao_gi_filter_source`).
    pub screen_gi: bool,
    /// The wet surfaces (`--wet-surfaces`; sky_light.wgsl `sky_w2_on`, `SF_WET_SURFACES`).
    pub wet_surfaces: bool,
    /// The screen-space reflection's second generation (`--screen-reflect`; sky_light.wgsl `sky_sr_on`, `SF_SCREEN_REFLECT`).
    pub screen_reflect: bool,
    /// The camera model under the cinematic look (`--cine-camera`; `SF_CAMERA`; cine_post.rs):
    /// its passes are made only with it, and its lines in the post and bloom shaders sit behind it.
    pub camera: bool,
    /// The cars' materials and lamps (`--car-materials`; `SF_CAR_MATERIALS`,
    /// agents_draw.wgsl `ag_car_*`): compiled into the agents' pipelines only.
    pub car_materials: bool,
    /// The cars' reflections of the street's brightest lights (`--car-reflections`;
    /// `SF_CAR_REFLECTIONS`, agents_draw.wgsl `ag_car_far_lights`): the agents' pipelines only.
    pub car_reflections: bool,
    /// The night's photometric scale (`--night-photometry`; `SF_NIGHT_PHOTOMETRY`):
    /// the practicals' gain on the emission the shaders carry in the rig's units.
    pub night_photometry: bool,
    /// Each street lamp's pool as a bounce light (`--lamp-bounce`; `SF_LAMP_BOUNCE`,
    /// sky.rs `lamp_bounce_wgsl`'s `LT_LAMP_BOUNCE`).
    pub lamp_bounce: bool,
    /// The city sky's stars by its limiting magnitude (`--urban-sky`; `SF_URBAN_SKY`).
    pub urban_sky: bool,
    /// Office storeys behind the glass (`--office-floors`; `SF_OFFICE_FLOORS`, rooms.wgsl).
    pub office_floors: bool,
    /// Lit homes and shops seen from the night street (`--room-depth`; `SF_ROOM_DEPTH`, rooms.wgsl).
    pub room_depth: bool,
    /// The natural-detail bits compiled in (`NATURE_*`; their `// sf:` blocks, `sf_select`).
    /// New code with no runtime test: `ALL` leaves it out, as the shaders were before it.
    pub nature: u32,
}

impl ShaderFeatures {
    /// Every feature compiled in (the shaders as before stage P1). The street's
    /// physically based pass stays out: it has no runtime gate, so compiling it in
    /// turns it on; it comes in only with its switch (`--street-pbr`). So do the
    /// natural-detail bits (`nature`: new code with no runtime test).
    pub const ALL: Self = Self {
        lx_bits: u32::MAX, grime: true, street_life: true, structures: true, tunnels: true, shop_light: true, facade_clutter: true, cinematic: true,
        facade_glass: true, far_facades: true, facade_weather: true, canyon_glass: true, // the facades track
        street_pbr: false, street: 0, // the street track
        sun_bounce: true, ao_albedo: true, soft_shadows: true, screen_gi: true, // the light track
        wet_surfaces: true, screen_reflect: true, // the reflect track
        camera: true, // the camera track
        car_materials: true, car_reflections: true, // the vehicles track
        night_photometry: true, lamp_bounce: true, urban_sky: true, office_floors: true, room_depth: true, // the night track
        nature: 0, // the nature track
    };
    /// No feature compiled in (the default frame's pipelines).
    pub const NONE: Self = Self {
        lx_bits: 0, grime: false, street_life: false, structures: false, tunnels: false, shop_light: false, facade_clutter: false, cinematic: false,
        facade_glass: false, far_facades: false, facade_weather: false, canyon_glass: false, // the facades track
        street_pbr: false, street: 0, // the street track
        sun_bounce: false, ao_albedo: false, soft_shadows: false, screen_gi: false, // the light track
        wet_surfaces: false, screen_reflect: false, // the reflect track
        camera: false, // the camera track
        car_materials: false, car_reflections: false, // the vehicles track
        night_photometry: false, lamp_bounce: false, urban_sky: false, office_floors: false, room_depth: false, // the night track
        nature: 0, // the nature track
    };

    /// The features the look's options ask for, with the world's (`structures`,
    /// `tunnels`, `facade_clutter`: eo-app's `--structures`, `--tunnels`,
    /// `--facade-clutter`, which the look's options do not carry).
    pub fn of(o: &LookOptions, structures: bool, tunnels: bool, facade_clutter: bool) -> Self {
        Self {
            lx_bits: o.lighting.mask(),
            grime: o.grime > 0,
            street_life: o.street_life > 0,
            structures,
            tunnels: structures && tunnels,
            shop_light: o.sky.shop_light > 0,
            facade_clutter,
            cinematic: o.look.is_cinematic(),
            facade_glass: false,
            far_facades: false,
            facade_weather: false,
            canyon_glass: false,
            street_pbr: false,
            street: 0,
            sun_bounce: o.sun_bounce,
            ao_albedo: o.ao_albedo,
            soft_shadows: o.soft_shadows,
            screen_gi: o.screen_gi,
            wet_surfaces: o.sky.wet_surfaces,
            screen_reflect: o.sky.screen_reflect,
            camera: o.look.is_cinematic() && o.sky.cine_camera,
            car_materials: o.car_materials > 0,
            car_reflections: o.car_materials > 0 && o.car_reflections > 0,
            night_photometry: o.sky.night_photometry,
            lamp_bounce: o.sky.lamp_bounce,
            urban_sky: o.sky.urban_sky,
            office_floors: o.sky.office_floors,
            room_depth: o.sky.room_depth,
            nature: o.nature & NATURE_ALL,
        }
    }

    /// Whether the lamp and headlight shadow test is compiled in (light_shadow.rs `use_wgsl`).
    pub fn light_shadows(&self) -> bool {
        self.lx_bits & (LX_BIT_LIGHT_SHADOWS | LX_BIT_HEADLIGHT_SHADOWS) != 0
    }

    /// `o` with every look feature this set compiled out turned off (a
    /// feature off at start comes on only after a restart; one compiled in
    /// may go off and on again). Levels are kept where the feature is in.
    pub fn limit(&self, o: &LookOptions) -> LookOptions {
        let mut l = *o;
        let lx = |bit: u32| self.lx_bits & bit != 0;
        let g = &mut l.lighting;
        g.cascade_blend &= lx(LX_BIT_CASCADE_BLEND);
        if !lx(LX_BIT_FAR_SHADOWS) {
            g.far_shadows = 0;
        }
        if !lx(LX_BIT_AMBIENT_VOLUME) {
            g.ambient_volume = 0;
            g.ground_bounce = false;
        }
        if !lx(LX_BIT_PROBE_GRID) {
            g.probe_grid = 0;
        }
        if !lx(LX_BIT_LIGHT_SHADOWS) {
            g.light_shadows = 0;
            g.light_shadow_moving = false;
        }
        if !lx(LX_BIT_HEADLIGHT_SHADOWS) {
            g.headlight_shadows = 0;
        }
        if !lx(LX_BIT_LIT_FOG) {
            g.lit_fog = 0;
        }
        g.smooth_time &= lx(LX_BIT_SMOOTH_TIME);
        if !self.grime {
            l.grime = 0;
        }
        if !self.street_life {
            l.street_life = 0;
        }
        if !self.shop_light {
            l.sky.shop_light = 0;
        }
        if !self.cinematic {
            l.look = eo_look::cinematic::LookProfile::Default;
            l.sky.cinematic = false;
        }
        l.sun_bounce &= self.sun_bounce;
        l.ao_albedo &= self.ao_albedo;
        l.soft_shadows &= self.soft_shadows;
        l.screen_gi &= self.screen_gi;
        if !self.wet_surfaces {
            l.sky.wet_surfaces = false;
        }
        if !self.screen_reflect {
            l.sky.screen_reflect = false;
        }
        l.sky.cine_camera &= self.camera;
        if !self.car_materials {
            l.car_materials = 0;
        }
        if !self.car_reflections {
            l.car_reflections = 0;
        }
        l.sky.night_photometry &= self.night_photometry;
        l.sky.lamp_bounce &= self.lamp_bounce;
        l.sky.urban_sky &= self.urban_sky;
        l.sky.office_floors &= self.office_floors;
        l.sky.room_depth &= self.room_depth;
        l.nature &= self.nature;
        l
    }

    /// The generated constants (look_common_source; every shader on the look's common code reads them).
    pub fn wgsl(&self) -> String {
        format!(
            "\n// generated by look.rs ShaderFeatures::wgsl (stage P1: what is off is compiled out)\nconst SF_LX_BITS: u32 = {}u;\n{}{}",
            self.lx_bits,
            [
                ("SF_GRIME", self.grime),
                ("SF_STREET_LIFE", self.street_life),
                ("SF_STRUCTURES", self.structures),
                ("SF_TUNNELS", self.tunnels),
                ("SF_SHOP_LIGHT", self.shop_light),
                ("SF_FACADE_CLUTTER", self.facade_clutter),
                ("SF_CINEMATIC", self.cinematic),
                // the facades track
                ("SF_FACADE_GLASS", self.facade_glass),
                ("SF_FAR_FACADES", self.far_facades),
                ("SF_FACADE_WEATHER", self.facade_weather),
                ("SF_CANYON_GLASS", self.canyon_glass),
                // the street track
                ("SF_STREET_PBR", self.street_pbr),
                ("SF_STREET_WEAR", self.street & STREET_WEAR != 0),
                ("SF_STREET_GLARE", self.street & STREET_GLARE != 0),
                // the light track
                ("SF_SUN_BOUNCE", self.sun_bounce),
                ("SF_AO_ALBEDO", self.ao_albedo),
                ("SF_SOFT_SHADOWS", self.soft_shadows),
                ("SF_SCREEN_GI", self.screen_gi),
                // the reflect track
                ("SF_WET_SURFACES", self.wet_surfaces),
                ("SF_SCREEN_REFLECT", self.screen_reflect),
                // the camera track
                ("SF_CAMERA", self.camera),
                // the vehicles track
                ("SF_CAR_MATERIALS", self.car_materials),
                ("SF_CAR_REFLECTIONS", self.car_reflections),
                // the night track
                ("SF_NIGHT_PHOTOMETRY", self.night_photometry),
                ("SF_LAMP_BOUNCE", self.lamp_bounce),
                ("SF_URBAN_SKY", self.urban_sky),
                ("SF_OFFICE_FLOORS", self.office_floors),
                ("SF_ROOM_DEPTH", self.room_depth),
            ]
            .iter()
            .map(|(name, on)| format!("const {name}: bool = {on};\n"))
            .collect::<String>(),
            penumbra_wgsl()
        )
    }
}

/// (`--soft-shadows`) the penumbra's constants (eo-look `penumbra`; read only
/// behind `SF_SOFT_SHADOWS`, declared in every set so the code compiles either way).
fn penumbra_wgsl() -> String {
    use eo_look::penumbra as p;
    let f = |v: f64| format!("{v:.8}");
    format!(
        "const PS_TAN_DIAMETER: f32 = {};\nconst PS_MAX_SPREAD_M: f32 = {};\nconst PS_MIN_CASTER_M: f32 = {};\nconst PS_MAX_RADIUS_TEXELS: f32 = {};\nconst PS_TENT_TEXELS: f32 = {};\nconst PS_MAX_PLANE_SLOPE: f32 = {};\nconst PS_SEARCH_TAPS: u32 = {}u;\nconst PS_FILTER_TAPS: u32 = {}u;\n",
        f(p::tan_diameter()),
        f(p::MAX_SPREAD_M),
        f(p::MIN_CASTER_M),
        f(p::MAX_RADIUS_TEXELS),
        f(p::TENT_TEXELS),
        f(p::MAX_PLANE_SLOPE),
        p::SEARCH_TAPS,
        p::FILTER_TAPS
    )
}

/// (`--night-photometry`) the look uniform's `fog.z`: the practicals' gain
/// at the frame's sun, 0 with the switch off (the uniform as before).
fn night_gain(on: bool, f: &LookFrame) -> f32 {
    if on { eo_sky::night_photometry::practical_gain(f.model.sun.altitude_deg) as f32 } else { 0.0 }
}

/// A shader file's natural-detail blocks for the features compiled in now: the
/// lines between `// sf:NAME+` and `// sf:end` are kept only with the bit NAME
/// (`FOLIAGE`, `LOD_FADE`, `TURF`, `PALM`, `HILLS`, `COVER`: `NATURE_*`) in, those between `// sf:NAME-`
/// and `// sf:end` only without it, and the marker lines are dropped (`NAME|OTHER`:
/// either bit; blocks do not nest). With every
/// bit out the text is the file as it was before the bits existed, so a shader
/// at the edge of what Apple's compiler takes (the props' cut-out) compiles as
/// before, and with a bit in it holds its code alone, not the code it replaces.
pub fn sf_select(src: &str) -> String {
    let f = shader_features();
    let on = |name: &str| match name {
        "FOLIAGE" => f.nature & NATURE_FOLIAGE != 0,
        "LOD_FADE" => f.nature & NATURE_LOD_FADE != 0,
        "TURF" => f.nature & NATURE_TURF != 0,
        "PALM" => f.nature & NATURE_PALM != 0,
        "HILLS" => f.nature & NATURE_HILLS != 0,
        "COVER" => f.nature & NATURE_COVER != 0,
        _ => false,
    };
    let mut out = String::with_capacity(src.len());
    let mut keep = true;
    for line in src.split_inclusive('\n') {
        if let Some(rest) = line.trim().strip_prefix("// sf:") {
            if rest == "end" {
                keep = true;
                continue;
            }
            // `A|B`: kept with either bit in (code two features share).
            if let Some(names) = rest.strip_suffix('+') {
                keep = names.split('|').any(on);
                continue;
            }
            if let Some(names) = rest.strip_suffix('-') {
                keep = !names.split('|').any(on);
                continue;
            }
        }
        if keep {
            out.push_str(line);
        }
    }
    out
}

static SHADER_FEATURES: std::sync::RwLock<ShaderFeatures> = std::sync::RwLock::new(ShaderFeatures::ALL);

/// Chooses the features compiled into every pipeline made after this call
/// (eo-app, before it makes the renderer). Not set: `ShaderFeatures::ALL`.
pub fn set_shader_features(f: ShaderFeatures) {
    *SHADER_FEATURES.write().unwrap_or_else(|e| e.into_inner()) = f;
}

thread_local! {
    static SCOPED_SHADER_FEATURES: std::cell::Cell<Option<ShaderFeatures>> = const { std::cell::Cell::new(None) };
}

/// The features compiled into the pipelines made now (on this thread: a
/// `with_shader_features` scope's, else the process's).
pub fn shader_features() -> ShaderFeatures {
    SCOPED_SHADER_FEATURES.with(|s| s.get()).unwrap_or_else(|| *SHADER_FEATURES.read().unwrap_or_else(|e| e.into_inner()))
}

/// Runs `body` with `f` compiled into every pipeline and shader source this
/// thread makes meanwhile (tests: two renderers of different sets in one
/// process, without touching the process's set).
pub fn with_shader_features<R>(f: ShaderFeatures, body: impl FnOnce() -> R) -> R {
    let old = SCOPED_SHADER_FEATURES.with(|s| s.replace(Some(f)));
    struct Restore(Option<ShaderFeatures>);
    impl Drop for Restore {
        fn drop(&mut self) {
            SCOPED_SHADER_FEATURES.with(|s| s.set(self.0));
        }
    }
    let _restore = Restore(old);
    body()
}

/// The city shader with `ambient` of the ambient volume and `shadow` of the sun's shadow lookup compiled in.
pub fn city_shader_source_variant(ambient: crate::ambient::ShaderLevels, shadow: ShadowLevels) -> String {
    city_shader_source_variant_probes(ambient, shadow, true)
}

/// `city_shader_source_variant`, and `single_probe` false compiles the
/// single scene probe's lookups out (facade_age.wgsl `FA_PROBE_SHADER`): for
/// the props and agents, whose frame never names a probe (their group 0
/// binds stand-ins), so their output is the same and their fragment shaders,
/// at the edge of what Apple's compiler takes, have room for the probe grid
/// (probe_grid.wgsl, compiled in everywhere but the far layer).
pub fn city_shader_source_variant_probes(ambient: crate::ambient::ShaderLevels, shadow: ShadowLevels, single_probe: bool) -> String {
    sf_select(&city_shader_parts(ambient, shadow, single_probe)) // the natural-detail blocks (`// sf:`) of the features compiled in
}

fn city_shader_parts(ambient: crate::ambient::ShaderLevels, shadow: ShadowLevels, single_probe: bool) -> String {
    format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}",
        include_str!("draw.wgsl"),
        street_constants_wgsl(),
        look_common_source(),
        include_str!("look_light.wgsl"),
        include_str!("surfaces_ao_use.wgsl"),
        include_str!("facade_age.wgsl"),             // facade
        crate::facade::facade_constants_wgsl()        // facade
    ) + "\n" + &crate::rooms::wgsl_constants() + include_str!("rooms.wgsl")
        + "\n" + include_str!("sky_light_core.wgsl") + "\n" + &crate::sky_reflect::sky_light_source(single_probe || shadow != ShadowLevels::Full) // sky (the reflect track's switches' code out of the agents' variant, sky_reflect.rs)
        + "\n" + LOOK_EXT_WGSL // group 1 bindings 12-18
        + "\n" + crate::far_shadow::WGSL + &shadow_wgsl(shadow) // the far sun shadow map's lookup and the variant's shadow parts (lighting)
        + "\n" + &crate::ambient::wgsl(ambient) // the ambient volume's sampling (lighting)
        + "\n" + &crate::probe_grid::wgsl(shadow != ShadowLevels::Far) // the probe grid's lookups; the far layer lies past its reach (lighting)
        + "\n" + &crate::light_shadow::use_wgsl(shadow != ShadowLevels::Far && shader_features().light_shadows()) // lamp and headlight shadows; the far layer lies past the lights' clusters, and without the option the test is compiled out (lighting, stage P1)
        + &crate::landmarks::city_wgsl() // landmarks
        + "\n" + include_str!("cc0.wgsl") + &crate::cc0::wgsl_constants(crate::cc0::scoring_pitch_m()) // cc0
        + "\n" + include_str!("weathering.wgsl") + &crate::weathering::wgsl() // the weathering level (`--grime`; every term behind it)
        + "\n" + include_str!("street_life.wgsl") + &crate::street_life::wgsl() // street life (`--street-life`; every term behind it)
        + "\n" + include_str!("street_pbr.wgsl") + &crate::street_pbr::wgsl() // the street's physically based surface (`--street-pbr`; every call behind SF_STREET_PBR)
        + &crate::structures::wgsl() // the grade-separated roads' concrete (draw.wgsl kind 12)
        + &format!("\n// generated by look.rs city_shader_source_variant_probes\nconst FA_PROBE_SHADER: bool = {single_probe};\n") // the single probe's lookups (lighting)
}

/// Group 1 bindings 12-18 (the lighting additions' resources). Every shader
/// of a pipeline on `look-bgl` composes it (the city shader above, and the
/// far shore, water, surf and prop wire shaders, which build their own).
pub const LOOK_EXT_WGSL: &str = include_str!("look_ext.wgsl");

/// The occlusion pass's shader (surfaces_ao.wgsl on the look's common code)
/// and, with `ShaderFeatures::sun_bounce`, the sun bounce's gather
/// (surfaces_ao_sun.wgsl, eo-look `sun_bounce`'s constants); without it a
/// stub that gathers nothing, compiled out with `SF_SUN_BOUNCE`.
pub fn ao_shader_source() -> String {
    let mut s = format!("{}\n{}", look_common_source(), include_str!("surfaces_ao.wgsl"));
    if shader_features().sun_bounce {
        use eo_look::sun_bounce as sb;
        let f = |v: f64| format!("{v:.8}");
        let v3 = |v: [f64; 3]| format!("vec3<f32>({}, {}, {})", f(v[0]), f(v[1]), f(v[2]));
        s.push_str("\n// generated by look.rs ao_shader_source (eo-look sun_bounce)\n");
        s.push_str(&format!("const SB_SAMPLES: i32 = {};\n", sb::SAMPLES));
        for (n, v) in [("SB_RADIUS_M", sb::RADIUS_M), ("SB_MIN_DIST2", sb::MIN_DIST2_M2), ("SB_GROUND_COS", sb::GROUND_COS), ("SB_MIN_SUN_COS", sb::MIN_SUN_COS), ("SB_GOLDEN_ANGLE", sb::GOLDEN_ANGLE), ("SB_MAX_SLOPE", sb::MAX_SLOPE)] {
            s.push_str(&format!("const {n}: f32 = {};\n", f(v)));
        }
        s.push_str(&format!("const SB_WALL_ALBEDO: vec3<f32> = {};\n", v3(sb::WALL_ALBEDO)));
        s.push_str(include_str!("surfaces_ao_sun.wgsl"));
    } else {
        s.push_str("\nfn ao_sun_bounce(world: vec3<f32>, n: vec3<f32>, rot: f32, jit: f32, near: bool) -> vec3<f32> {\n    return vec3<f32>(0.0);\n}\n");
        if shader_features().screen_gi {
            s.push_str("var<private> sb_near_m: f32 = AO_RADIUS_M;\n");
        }
    }
    if shader_features().screen_gi {
        // (`--screen-gi`) the gather (eo-look `screen_gi`'s constants).
        s.push_str(&gi_constants());
        s.push_str(include_str!("surfaces_ao_gi.wgsl"));
    }
    s
}

/// (`--screen-gi`) eo-look `screen_gi`'s constants for the gather and the filter.
fn gi_constants() -> String {
    use eo_look::screen_gi as g;
    let f = |v: f64| format!("{v:.8}");
    let mut s = String::from("\n// generated by look.rs gi_constants (eo-look screen_gi)\n");
    s.push_str(&format!("const GI_SLICES: u32 = {}u;\nconst GI_STEPS: u32 = {}u;\nconst GI_SECTORS: u32 = {}u;\n", g::SLICES, g::STEPS, g::SECTORS));
    for (n, v) in [
        ("GI_RADIUS_M", g::RADIUS_M),
        ("GI_MAX_REACH_SHARE", g::MAX_REACH_SHARE),
        ("GI_THICKNESS_M", g::THICKNESS_M),
        ("GI_THICKNESS_PER_M", g::THICKNESS_PER_M),
        ("GI_THICKNESS_SHARE", g::THICKNESS_SHARE),
        ("GI_HISTORY_WEIGHT", g::HISTORY_WEIGHT),
        ("GI_FILTER_PLANE_M", g::FILTER_PLANE_M),
        ("GI_FILTER_PLANE_REL", g::FILTER_PLANE_REL),
        ("GI_MAX_DIFFUSE_ALBEDO", g::MAX_DIFFUSE_ALBEDO),
    ] {
        s.push_str(&format!("const {n}: f32 = {};\n", f(v)));
    }
    s
}

/// (`--screen-gi`) the occlusion pass's spatial filter (its own module).
pub fn ao_gi_filter_source() -> String {
    format!("{}\n{}", gi_constants(), include_str!("surfaces_ao_gi_filter.wgsl"))
}

/// look_common.wgsl with (surfaces) the street field's constants (GF_*), from
/// eo-look's `GroundField` (data/ground-field.json, the JS DEFAULT_GROUND_FIELD).
pub fn look_common_source() -> String {
    let g = eo_look::ground_field::GroundField::get();
    let f = |v: f64| format!("{v:.8}");
    let v3 = |v: [f64; 3]| format!("vec3<f32>({}, {}, {})", f(v[0]), f(v[1]), f(v[2]));
    let consts = [
        ("GF_S_KERB", "f32", f(g.s_kerb)),
        ("GF_S_FAR_KERB", "f32", f(g.s_far_kerb)),
        ("GF_S_FAR_WALK", "f32", f(g.s_far_walk)),
        ("GF_AXIS", "vec2<f32>", format!("vec2<f32>({}, {})", f(g.axis[0]), f(g.axis[1]))),
        ("GF_CROSS_SHARE", "f32", f(g.cross_share)),
        ("GF_ROAD_RATIO", "f32", f(g.road_ratio)),
        ("GF_WALL_RATIO", "f32", f(g.wall_ratio)),
        ("GF_ROAD_TINT", "vec3<f32>", v3(g.road_tint)),
        ("GF_WALL_TINT", "vec3<f32>", v3(g.wall_tint)),
        ("GF_SPEC_MAX", "f32", f(g.spec_max)),
        ("GF_WALLED_SHARE", "f32", f(g.walled_share)),
        ("GF_CLUTTER", "f32", f(g.clutter)),
        ("GF_CLUTTER_SIN", "f32", f(g.clutter_sin)),
        ("GF_SHARPNESS", "f32", f(g.sharpness)),
        ("GF_POOL_COLOR", "vec3<f32>", v3(g.pool_color)),
        ("GF_SPILL_COLOR", "vec3<f32>", v3(g.spill_color)),
    ];
    let mut s = String::from(include_str!("look_common.wgsl"));
    s.push_str("\n// generated by look.rs look_common_source (eo-look GroundField)\n");
    for (n, t, v) in consts {
        s.push_str(&format!("const {n}: {t} = {v};\n"));
    }
    s.push_str(include_str!("sky_common.wgsl")); // sky
    s.push_str(include_str!("sky_atmo.wgsl")); // the atmosphere tables' sampling helpers
    s.push_str(&crate::sky::sea_wgsl()); // sky
    s.push_str(&shader_features().wgsl()); // the features compiled in (stage P1)
    s
}

/// the mean linear albedo (luminance) the walk concrete is drawn at:
/// weathered Portland-cement concrete, the measured 0.20-0.30 band of the
/// Lawrence Berkeley National Laboratory Heat Island Group's pavement albedo
/// survey (new concrete 0.35-0.45, falling to 0.20-0.30 as it weathers and
/// soils), at the band's top for a swept city footway. The JS draws its
/// footway at the sidewalk tint x the concrete tile's 0.456, a mean of 0.389:
/// new concrete, and the sunlit walk delivered near white (sRGB 219, 202, 185
/// at 11 h) with its scored joints in the tone curve's shoulder. A documented
/// departure from the JS (owner direction 2026-11-14: physical reference
/// values where the JS is the weaker reference).
pub const CONCRETE_AGED_ALBEDO: f64 = 0.30;

/// the fold that takes the mean JS footway (sidewalk tint x 0.456) to
/// `CONCRETE_AGED_ALBEDO`; applied to every walk-concrete class (footway,
/// kerb top, ramp, kerb face) in the street shader.
pub fn concrete_aged_fold() -> f64 {
    let m = eo_street::material::StreetMaterial::get();
    let t = m.tint("sidewalk").map(|v| eo_look::math::srgb_to_linear(v as f64 / 255.0) * m.concrete_base_mean_linear);
    let luma = 0.2126 * t[0] + 0.7152 * t[1] + 0.0722 * t[2];
    (CONCRETE_AGED_ALBEDO / luma).min(1.0)
}

/// the JS street material numbers (eo-street/data/street-material.json)
/// as WGSL constants for `street_surface` in draw.wgsl.
pub fn street_constants_wgsl() -> String {
    let m = eo_street::material::StreetMaterial::get();
    let (a, c) = (&m.pbr_sets.carriageway, &m.pbr_sets.concrete);
    let g = a.aggregate.as_ref().expect("carriageway aggregate tap");
    let sh = &m.shading;
    let f = |v: f64| format!("{:.6}", v);
    let v2 = |v: [f64; 2]| format!("vec2<f32>({}, {})", f(v[0]), f(v[1]));
    let v3 = |v: [f64; 3]| format!("vec3<f32>({}, {}, {})", f(v[0]), f(v[1]), f(v[2]));
    let rough = |k: &str| m.roughness.get(k).copied().unwrap_or(0.9);
    let base = a.base_mean_linear.unwrap_or([1.0; 3]);
    let mut s = String::from("// generated by look.rs street_constants_wgsl from eo-street/data/street-material.json\n");
    let mut c_ = |name: &str, ty: &str, val: String| s.push_str(&format!("const {name}: {ty} = {val};\n"));
    for (p, set) in [("ASPHALT", a), ("CONCRETE", c)] {
        let tile = if p == "CONCRETE" { sh.concrete_scan_period_m } else { set.tile_metres };
        c_(&format!("ST_{p}_TILE"), "f32", f(tile));
        c_(&format!("ST_{p}_MEAN"), "vec3<f32>", v3(set.mean_linear));
        c_(&format!("ST_{p}_CONTRAST"), "f32", f(set.albedo_contrast));
        c_(&format!("ST_{p}_FAR_CONTRAST"), "f32", f(set.far_contrast));
        c_(&format!("ST_{p}_NEAR"), "f32", f(set.detail_near_m));
        c_(&format!("ST_{p}_FAR"), "f32", f(set.detail_far_m));
        c_(&format!("ST_{p}_ALBEDO_CLAMP"), "vec2<f32>", v2(set.albedo_clamp));
        c_(&format!("ST_{p}_ROUGH_GAIN"), "f32", f(set.roughness_gain));
        c_(&format!("ST_{p}_ROUGH_CLAMP"), "vec2<f32>", v2(set.roughness_clamp));
        c_(&format!("ST_{p}_MEAN_ROUGH"), "f32", f(set.mean_roughness));
        c_(&format!("ST_{p}_MEAN_AO"), "f32", f(set.mean_ao));
        c_(&format!("ST_{p}_AO_STRENGTH"), "f32", f(set.ao_strength));
        c_(&format!("ST_{p}_NORMAL"), "f32", f(set.normal_strength));
    }
    c_("ST_ASPHALT_ROUGHNESS", "f32", f(rough("carriageway")));
    c_("ST_CONCRETE_ROUGHNESS", "f32", f(rough("concrete")));
    c_("ST_PAINT_ROUGHNESS", "f32", f(rough("markings")));
    // The asphalt's base fold is a neutral triple in the JS ([0.44, 0.44, 0.44]).
    c_("ST_ASPHALT_BASE", "vec3<f32>", v3(base));
    // x the aged-concrete fold (see `concrete_aged_fold`).
    c_("ST_CONCRETE_BASE", "f32", f(m.concrete_base_mean_linear * concrete_aged_fold()));
    c_("ST_AGG_TILE", "f32", f(g.tile_metres));
    c_("ST_AGG_CONTRAST", "f32", f(g.albedo_contrast));
    c_("ST_AGG_FAR_CONTRAST", "f32", f(g.far_contrast));
    c_("ST_AGG_NEAR", "f32", f(g.near_m));
    c_("ST_AGG_FAR", "f32", f(g.far_m));
    c_("ST_AGG_ALBEDO_CLAMP", "vec2<f32>", v2(g.albedo_clamp));
    c_("ST_AGG_ROUGH_GAIN", "f32", f(g.roughness_gain));
    c_("ST_AGG_ROUGH_CLAMP", "vec2<f32>", v2(g.roughness_clamp));
    c_("ST_AGG_NORMAL", "f32", f(g.normal_strength));
    c_("ST_AGG_RELIEF_NEAR", "f32", f(g.relief_near_m));
    c_("ST_AGG_RELIEF_FAR", "f32", f(g.relief_far_m));
    let pt = a.patch.as_ref().expect("carriageway patch");
    c_("ST_PATCH_FREQ", "f32", f(pt.frequency));
    c_("ST_PATCH_PHASE", "f32", f(pt.phase));
    c_("ST_PATCH_START", "f32", f(pt.start));
    c_("ST_PATCH_FULL", "f32", f(pt.full));
    c_("ST_PATCH_SCALE", "f32", f(pt.scale));
    c_("ST_PATCH_OFFSET", "vec2<f32>", v2(pt.offset));
    c_("ST_PATCH_TONE", "f32", f(pt.tone));
    c_("ST_PATCH_ROUGH", "f32", f(pt.roughness));
    c_("ST_MACRO_COARSE", "f32", f(sh.macro_coarse));
    c_("ST_MACRO_FINE", "f32", f(sh.macro_fine));
    c_("ST_MACRO_AMPLITUDE", "f32", f(sh.macro_amplitude));
    c_("ST_CROSS_FALL_DARKEN", "f32", f(sh.cross_fall_darken));
    c_("ST_CROSS_FALL_REACH", "f32", f(sh.cross_fall_reach_m));
    c_("ST_SILT_REACH", "f32", f(sh.gutter_silt_reach_m));
    c_("ST_SILT_DARKEN", "f32", f(sh.gutter_silt_darken));
    c_("ST_GUTTER_POUR_SILT", "f32", f(sh.gutter_pan_pour_silt));
    c_("ST_GUTTER_WIDTH", "f32", f(m.gutter_width));
    // A rendering cut-off, not a JS number: 400 m is past every range fade
    // (`detailFarM` 110, aggregate `farM` 85), where a 3 m scan tile is
    // smaller than a pixel and its mip chain has averaged it to the mean.
    c_("ST_SCAN_CUTOFF_M", "f32", f(400.0));
    c_("ST_ENV_PAINTED_METAL", "f32", f(m.env_base.get("painted-metal").copied().unwrap_or(0.9)));
    s
}

pub struct LookInit<'a> {
    pub object_buf: &'a wgpu::Buffer,
    pub group_buf: &'a wgpu::Buffer,
    pub object_count: u32,
    pub compact: bool,
    /// The count emulation (no MULTI_DRAW_INDIRECT_COUNT): the cascades' lists are registered in it.
    pub counts: Option<std::sync::Arc<crate::counted::Counts>>,
    pub output_format: wgpu::TextureFormat,
    pub sample_count: u32,
    pub debug_ids: bool,
    /// Bytes per vertex of the shared vertex buffer (position f32x3 first).
    pub vertex_stride: u64,
}

/// a half-resolution occlusion target (storage-written, sampled).
fn ao_texture(device: &wgpu::Device, format: wgpu::TextureFormat, size: (u32, u32)) -> wgpu::TextureView {
    ao_texture_with(device, format, size, wgpu::TextureUsages::empty())
}

/// An occlusion target with `extra` usages besides storage and sampling (`--screen-gi`'s copies).
fn ao_texture_with(device: &wgpu::Device, format: wgpu::TextureFormat, size: (u32, u32), extra: wgpu::TextureUsages) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("ao"),
            size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | extra,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

fn shadow_texture(device: &wgpu::Device, size: u32) -> (Vec<wgpu::TextureView>, wgpu::TextureView, u64) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sun-shadow-cascades"),
        size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: CASCADES as u32 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: SHADOW_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let layers = (0..CASCADES as u32)
        .map(|l| {
            tex.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: l,
                array_layer_count: Some(1),
                ..Default::default()
            })
        })
        .collect();
    let array = tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
    (layers, array, size as u64 * size as u64 * 2 * CASCADES as u64)
}

impl LookGpu {
    pub fn new(device: &wgpu::Device, init: LookInit) -> Self {
        // a bare renderer (tests) starts with the sky slice's stages off;
        // the app turns them on through `set_options` (LookOptions::default()).
        let options = LookOptions { sky: crate::sky::SkyOptions::off(), ..LookOptions::default() };
        let buffer = |label: &str, size: u64, usage: wgpu::BufferUsages| {
            device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size.max(16).next_multiple_of(16), usage, mapped_at_creation: false })
        };
        let uni = wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST;
        let look_buf = buffer("look", std::mem::size_of::<LookUniform>() as u64, uni);
        let cull_buf = buffer("shadow-cull", std::mem::size_of::<ShadowCullParams>() as u64, uni);
        let view_buf = buffer("shadow-views", VIEW_STRIDE * CASCADES as u64, uni);
        let n = init.object_count.max(1) as u64;
        let draws_bytes = n * CASCADES as u64 * crate::DRAW_ARGS_BYTES;
        let shadow_draws = buffer("shadow-draws", draws_bytes, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST); // COPY_DST: the count emulation zeroes it
        let shadow_counters = buffer(
            "shadow-counters",
            (SHADOW_COUNTER_WORDS * 4) as u64,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
        );
        let shadow_lists = init.counts.as_ref().filter(|_| init.compact).map(|c| {
            let lists = (0..CASCADES as u32)
                .map(|k| crate::counted::List { word: k, draws: shadow_draws.clone(), offset: k as u64 * n * crate::DRAW_ARGS_BYTES, stride: crate::DRAW_ARGS_BYTES, max: n as u32, floor: None })
                .collect();
            c.register("shadow", &shadow_counters, lists)
        });
        let (shadow_layers, shadow_array, shadow_bytes) = shadow_texture(device, options.shadow_map);
        let cmp_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow-compare"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });

        let entry = |binding, visibility, ty| wgpu::BindGroupLayoutEntry { binding, visibility, ty, count: None };
        let ubuf = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None };
        let sbuf = |read_only| wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None };
        let fs = wgpu::ShaderStages::FRAGMENT;
        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let cs = wgpu::ShaderStages::COMPUTE;

        // Group 1 of the city pipelines.
        let [l12, l13, l14, l15, l16, l17, l18, l19] = LightingBindings::layout_entries();
        let main_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("look-bgl"),
            entries: &[
                entry(0, vf, ubuf),
                entry(1, fs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false }),
                entry(2, fs, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison)),
                // occlusion + bounce result, its depth, a clamped linear sampler.
                entry(3, fs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                entry(4, fs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                entry(5, fs, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                // the sky uniform (the agents' velocity vertex shader reads it too), the
                // cloud sheet, the frame's lights and clusters, the previous resolved frame, a repeat sampler.
                entry(6, vf, ubuf),
                entry(7, fs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                entry(8, fs, sbuf(true)),
                entry(9, fs, sbuf(true)),
                entry(10, fs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                entry(11, fs, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                // the lighting additions (look_ext.wgsl), fragment only: far shadow
                // map, ambient volume and plane, probe grid and its table, light
                // shadow atlas and its matrices. Stand-ins until a feature binds its own.
                l12,
                l13,
                l14,
                l15,
                l16,
                l17,
                l18,
                l19, // (stage N2) the shop fronts
            ],
        });
        let (lighting_stand_ins, stand_in_bytes) = LightingBindings::stand_ins(device);
        let ao_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ao-linear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let ao_dummy = (ao_texture(device, AO_FORMAT, (1, 1)), ao_texture(device, AO_Z_FORMAT, (1, 1)));
        // the sky passes and their resources.
        let sky = crate::sky::SkyGpu::new(
            device,
            crate::sky::SkyInit { look_buf: &look_buf, hdr_format: HDR_FORMAT, depth_format: crate::DEPTH_FORMAT, shadow_array: &shadow_array, cmp_sampler: &cmp_sampler },
        );
        let main_bg = [0, 1].map(|_| Self::make_main_bg(device, &main_bgl, &look_buf, &shadow_array, &cmp_sampler, &ao_dummy.0, &ao_dummy.1, &ao_sampler, &sky, &sky.dummy_hist, &lighting_stand_ins));
        // the occlusion + bounce compute.
        let ao_src = ao_shader_source();
        let ao_sun = shader_features().sun_bounce;
        let ao_module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("surfaces_ao.wgsl"), source: wgpu::ShaderSource::Wgsl(ao_src.into()) });
        let mut ao_entries = vec![
            entry(0, cs, ubuf),
            entry(1, cs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
            entry(2, cs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
            entry(3, cs, wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: AO_FORMAT, view_dimension: wgpu::TextureViewDimension::D2 }),
            entry(4, cs, wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: AO_Z_FORMAT, view_dimension: wgpu::TextureViewDimension::D2 }),
        ];
        if ao_sun {
            // (`--sun-bounce`) the far sun map (ring 1's layer read with textureLoad).
            ao_entries.push(entry(5, cs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false }));
        }
        let ao_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("ao-bgl"), entries: &ao_entries });
        let ao_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("ao-layout"), bind_group_layouts: &[Some(&ao_bgl)], immediate_size: 0 });
        let ao_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ao_main"),
            layout: Some(&ao_layout),
            module: &ao_module,
            entry_point: Some("ao_main"),
            compilation_options: Default::default(),
            cache: None,
        });
        // (`--screen-gi`) the gather (the same module, its history bound at 6-8) and the filter.
        let ao_gi = shader_features().screen_gi.then(|| {
            let mut entries = ao_entries.clone();
            entries.push(entry(6, cs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }));
            entries.push(entry(7, cs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }));
            entries.push(entry(8, cs, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)));
            let gather_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("ao-gi-bgl"), entries: &entries });
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("ao-gi-layout"), bind_group_layouts: &[Some(&gather_bgl)], immediate_size: 0 });
            let gather = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("ao_gi_main"),
                layout: Some(&layout),
                module: &ao_module,
                entry_point: Some("ao_gi_main"),
                compilation_options: Default::default(),
                cache: None,
            });
            let filter_module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("surfaces_ao_gi_filter.wgsl"), source: wgpu::ShaderSource::Wgsl(ao_gi_filter_source().into()) });
            let filter_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("ao-gi-filter-bgl"),
                entries: &[
                    entry(0, cs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                    entry(1, cs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                    entry(2, cs, wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: AO_FORMAT, view_dimension: wgpu::TextureViewDimension::D2 }),
                ],
            });
            let filter_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("ao-gi-filter-layout"), bind_group_layouts: &[Some(&filter_bgl)], immediate_size: 0 });
            let filter = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("ao_gi_filter"),
                layout: Some(&filter_layout),
                module: &filter_module,
                entry_point: Some("ao_gi_filter"),
                compilation_options: Default::default(),
                cache: None,
            });
            AoGiPipelines { gather, gather_bgl, filter, filter_bgl }
        });

        // Shadow cull.
        let cull_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("look_shadow_cull.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("look_shadow_cull.wgsl").into()),
        });
        let cull_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow-cull-bgl"),
            entries: &[entry(0, cs, ubuf), entry(1, cs, sbuf(true)), entry(2, cs, sbuf(true)), entry(3, cs, sbuf(false)), entry(4, cs, sbuf(false))],
        });
        let cull_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow-cull-bg"),
            layout: &cull_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: cull_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: init.object_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: init.group_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: shadow_draws.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: shadow_counters.as_entire_binding() },
            ],
        });
        let cull_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("shadow-cull-layout"), bind_group_layouts: &[Some(&cull_bgl)], immediate_size: 0 });
        let cull_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("shadow_cull"),
            layout: Some(&cull_layout),
            module: &cull_module,
            entry_point: Some("shadow_cull"),
            compilation_options: Default::default(),
            cache: None,
        });

        // Shadow depth.
        let depth_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("look_shadow_draw.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("look_shadow_draw.wgsl").into()),
        });
        let depth_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow-depth-bgl"),
            entries: &[
                entry(0, wgpu::ShaderStages::VERTEX, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: None }),
                entry(1, wgpu::ShaderStages::VERTEX, sbuf(true)),
            ],
        });
        let depth_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow-depth-bg"),
            layout: &depth_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &view_buf, offset: 0, size: Some(std::num::NonZeroU64::new(80).unwrap()) }),
                },
                wgpu::BindGroupEntry { binding: 1, resource: init.object_buf.as_entire_binding() },
            ],
        });
        let depth_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("shadow-depth-layout"), bind_group_layouts: &[Some(&depth_bgl)], immediate_size: 0 });
        let pos_only = [wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 }];
        let depth_with = |label: &str, bias: wgpu::DepthBiasState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&depth_layout),
                vertex: wgpu::VertexState {
                    module: &depth_module,
                    entry_point: Some("shadow_vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout { array_stride: init.vertex_stride, step_mode: wgpu::VertexStepMode::Vertex, attributes: &pos_only })],
                },
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: SHADOW_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias,
                }),
                multisample: Default::default(),
                fragment: None,
                multiview_mask: None,
                cache: None,
            })
        };
        let depth_pipeline = depth_with("shadow-depth", Default::default());
        let depth_pipeline_sloped = depth_with("shadow-depth (slope bias)", wgpu::DepthBiasState { constant: 0, slope_scale: SLOPE_BIAS_SCALE, clamp: SLOPE_BIAS_CLAMP });

        // Sky (inside the city pass: same targets and sample count).
        let sky_src = format!("{}\n{}\n{}", look_common_source(), crate::sky::urban_sky_wgsl(), include_str!("look_sky.wgsl")); // (`--urban-sky`) the stars' constants
        let sky_module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("look_sky.wgsl"), source: wgpu::ShaderSource::Wgsl(sky_src.into()) });
        // + the sky uniform (moon, stars).
        let look_only_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("look-only-bgl"), entries: &[entry(0, vf, ubuf), entry(1, vf, ubuf)] });
        let sky_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky-bg"),
            layout: &look_only_bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: look_buf.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: sky.uniform.as_entire_binding() }],
        });
        let sky_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("sky-layout"), bind_group_layouts: &[Some(&look_only_bgl)], immediate_size: 0 });
        let hdr_target = wgpu::ColorTargetState { format: HDR_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL };
        let ids_target = wgpu::ColorTargetState { format: crate::IDS_FORMAT, blend: None, write_mask: wgpu::ColorWrites::empty() };
        let sky_targets_ids = [Some(hdr_target.clone()), Some(ids_target)];
        let sky_targets = [Some(hdr_target)];
        let sky_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky"),
            layout: Some(&sky_layout),
            vertex: wgpu::VertexState { module: &sky_module, entry_point: Some("sky_vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: crate::DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState { count: init.sample_count, ..Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &sky_module,
                entry_point: Some(if init.debug_ids { "sky_fs_ids" } else { "sky_fs" }),
                compilation_options: Default::default(),
                targets: if init.debug_ids { &sky_targets_ids } else { &sky_targets },
            }),
            multiview_mask: None,
            cache: None,
        });

        // Post.
        let post_src = format!("{}\n{}\n{}", look_common_source(), include_str!("look_finish.wgsl"), include_str!("look_post.wgsl"));
        let post_module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("look_post.wgsl"), source: wgpu::ShaderSource::Wgsl(post_src.into()) });
        let post_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post-bgl"),
            entries: &[
                entry(0, vf, ubuf),
                entry(1, fs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                // the sky uniform, the bloom, its sampler.
                entry(2, fs, ubuf),
                entry(3, fs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                entry(4, fs, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
            ],
        });
        let post_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("post-layout"), bind_group_layouts: &[Some(&post_bgl)], immediate_size: 0 });
        let post_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("post"),
            layout: Some(&post_layout),
            vertex: wgpu::VertexState { module: &post_module, entry_point: Some("post_vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &post_module,
                entry_point: Some("post_fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: init.output_format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let bytes = shadow_bytes + draws_bytes + (SHADOW_COUNTER_WORDS * 4) as u64 + look_buf.size() + cull_buf.size() + view_buf.size() + sky.bytes + stand_in_bytes; // sky; + the lighting stand-ins
        Self {
            sky, // sky
            sky_depth_ok: std::sync::atomic::AtomicBool::new(false), // sky
            post_source: std::sync::atomic::AtomicU32::new(0),       // sky
            lighting_bound: Mutex::new(lighting_stand_ins.clone()),
            lighting_stand_ins,
            lx: Mutex::new([[0.0; 4]; LX_LEN]),
            options,
            output_format: init.output_format,
            object_count: init.object_count,
            compact: init.compact,
            look_buf,
            cull_buf,
            view_buf,
            shadow_draws,
            shadow_counters,
            shadow_lists,
            shadow_layers,
            main_bgl,
            main_bg: Mutex::new(main_bg),
            shadow_array,
            ao_pipeline,
            ao_bgl,
            ao_sun,
            ao_gi,
            ao_sampler,
            ao_dummy,
            ao: Mutex::new(None),
            cull_pipeline,
            cull_bg,
            depth_pipeline,
            depth_pipeline_sloped,
            depth_bg,
            sky_pipeline,
            sky_bg,
            post_pipeline,
            post_bgl,
            post: Mutex::new(None),
            photo: None, // (`--photo-finish`) made by `set_options`
            cine: None,  // (`--look cinematic`) made by `set_options`
            state: Mutex::new(State {
                hour: DEFAULT_HOUR,
                transfer: None,
                frame: None,
                frame_index: 0,
                last_cascades: Default::default(),
                mask: 0,
                rotation: 0,
                last_eye: None,
                force_all: true,
                pending: None,
                cur_cam: None,
                ao_made: None,
                terms: None,                // sky
                weather_key: (i64::MIN, 0, 0), // sky
                sky_base: None,
                soft: false, // smooth time
                keys: Default::default(),
                cascade_dirs: [None; CASCADES],
            }),
            cmp_sampler,
            device: device.clone(),
            bytes,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn make_main_bg(
        device: &wgpu::Device,
        bgl: &wgpu::BindGroupLayout,
        look: &wgpu::Buffer,
        shadow: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        ao: &wgpu::TextureView,
        ao_z: &wgpu::TextureView,
        ao_sampler: &wgpu::Sampler,
        sky: &crate::sky::SkyGpu,     // sky
        prev: &wgpu::TextureView,     // the previous resolved frame
        lighting: &LightingBindings,  // bindings 12-18
    ) -> wgpu::BindGroup {
        let [l12, l13, l14, l15, l16, l17, l18, l19] = lighting.entries();
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("look-bg"),
            layout: bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: look.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(shadow) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(ao) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(ao_z) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(ao_sampler) },
                // sky
                wgpu::BindGroupEntry { binding: 6, resource: sky.uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(&sky.clouds.shadow_view) }, // the cloud-shadow map (was the coverage sheet)
                wgpu::BindGroupEntry { binding: 8, resource: sky.frame_lights.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 9, resource: sky.clusters.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(prev) },
                wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::Sampler(&sky.rep) },
                // the lighting additions (stand-ins until a feature binds its own)
                l12,
                l13,
                l14,
                l15,
                l16,
                l17,
                l18,
                l19, // (stage N2) the shop fronts
            ],
        })
    }

    fn rebuild_main_bg(&self) {
        let ao = self.ao.lock().unwrap();
        let (a, z) = match ao.as_ref() {
            Some(t) => (&t.ao_view, &t.z_view),
            None => (&self.ao_dummy.0, &self.ao_dummy.1),
        };
        let lighting = self.lighting_bound.lock().unwrap().clone();
        // main_bg[p] is bound while the resolve writes history p: it reads history 1 - p.
        let bg = |p: usize| {
            let prev = self.sky.reflect_view(1 - p).unwrap_or_else(|| self.sky.dummy_hist.clone()); // (`--screen-reflect`: with its mip chain)
            Self::make_main_bg(&self.device, &self.main_bgl, &self.look_buf, &self.shadow_array, &self.cmp_sampler, a, z, &self.ao_sampler, &self.sky, &prev, &lighting)
        };
        *self.main_bg.lock().unwrap() = [bg(0), bg(1)];
    }

    /// The stand-ins group 1 bindings 12-18 hold while no feature binds its own.
    pub fn lighting_stand_ins(&self) -> &LightingBindings {
        &self.lighting_stand_ins
    }

    /// What group 1 bindings 12-18 hold now.
    pub fn lighting_bindings(&self) -> LightingBindings {
        self.lighting_bound.lock().unwrap().clone()
    }

    /// Bind all of group 1 bindings 12-18 at once and rebuild both main bind
    /// groups (made on the calling thread; call it when a resource is
    /// created or replaced, not every frame).
    pub fn set_lighting_bindings(&self, b: LightingBindings) {
        *self.lighting_bound.lock().unwrap() = b;
        crate::hitch::mark(crate::hitch::Mark::BindGroupCreated, 2);
        self.rebuild_main_bg();
    }

    /// Change some of bindings 12-18 (`f` edits the bound set) and rebuild once.
    fn edit_lighting(&self, f: impl FnOnce(&mut LightingBindings, &LightingBindings)) {
        let mut b = self.lighting_bindings();
        f(&mut b, &self.lighting_stand_ins);
        self.set_lighting_bindings(b);
    }

    /// Binding 12: the far sun shadow map (a depth 2D array, a layer per ring); None the stand-in.
    pub fn set_far_shadow_map(&self, map: Option<&wgpu::TextureView>) {
        self.edit_lighting(|b, s| b.far_shadow_map = map.unwrap_or(&s.far_shadow_map).clone());
        if self.ao_sun {
            if let Some(t) = self.ao.lock().unwrap().as_mut() {
                t.bind = None; // (`--sun-bounce`) the occlusion pass's group binds the map
                if let Some(g) = t.gi.as_mut() {
                    g.bind = None; // (`--screen-gi`) and so does the gather's
                }
            }
        }
        self.sky.set_far_shadow_map(map); // (stage L4) the golden haze's shade past the cascades
    }

    /// Bindings 13 and 14: the ambient volume (3D) and the citywide plane (2D); None the stand-ins.
    pub fn set_ambient_volume(&self, volume: Option<&wgpu::TextureView>, plane: Option<&wgpu::TextureView>) {
        self.edit_lighting(|b, s| {
            b.ambient_volume = volume.unwrap_or(&s.ambient_volume).clone();
            b.ambient_plane = plane.unwrap_or(&s.ambient_plane).clone();
        });
    }

    /// Bindings 15 and 16: the probe grid's cubes (a 2D array, 6 layers a probe) and its table; None the stand-ins.
    pub fn set_probe_grid(&self, cubes: Option<&wgpu::TextureView>, table: Option<&wgpu::Buffer>) {
        self.edit_lighting(|b, s| {
            b.probe_grid = cubes.unwrap_or(&s.probe_grid).clone();
            b.probe_table = table.unwrap_or(&s.probe_table).clone();
        });
    }

    /// Bindings 17 and 18: the light shadow atlas (a depth 2D array) and its matrices; None the stand-ins.
    pub fn set_light_shadows(&self, atlas: Option<&wgpu::TextureView>, matrices: Option<&wgpu::Buffer>) {
        self.edit_lighting(|b, s| {
            b.light_shadow_atlas = atlas.unwrap_or(&s.light_shadow_atlas).clone();
            b.light_shadow_table = matrices.unwrap_or(&s.light_shadow_table).clone();
        });
    }

    /// (`--shop-light`, stage N2) Binding 19: the shop fronts table
    /// (shop_lights.rs `fronts_table`, `fronts_buffer`); None the stand-in (no fronts).
    pub fn set_shop_fronts(&self, table: Option<&wgpu::Buffer>) {
        self.edit_lighting(|b, s| b.shop_fronts = table.unwrap_or(&s.shop_fronts).clone());
    }

    /// Set rows of `Look.lx` from `first` on (rows 1-15, and row 0's y z w,
    /// see `LX_*`); the next `update` writes them. Row 0 x is always the feature mask.
    pub fn set_lighting_params(&self, first: usize, rows: &[[f32; 4]]) {
        let mut lx = self.lx.lock().unwrap();
        let end = (first + rows.len()).min(LX_LEN);
        if first < end {
            lx[first..end].copy_from_slice(&rows[..end - first]);
        }
    }

    /// The `Look.lx` rows the next `update` writes (row 0 x: the feature mask from the options).
    pub fn lighting_params(&self) -> [[f32; 4]; LX_LEN] {
        let mut lx = *self.lx.lock().unwrap();
        lx[LX_FLAGS][0] = self.options.lighting.flags_word();
        lx
    }

    /// occlusion + bounce at half resolution of the output, after the
    /// city pass (needs the depth it drew, stored, and the HDR it lit).
    /// Records an empty pass (for the timestamps) when it cannot run.
    pub fn encode_ao(&self, enc: &mut wgpu::CommandEncoder, depth: &wgpu::TextureView, hdr: &wgpu::TextureView, ts: Option<(&wgpu::QuerySet, u32)>) {
        let tw = ts.map(|(q, b)| wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(b + Q_AO), end_of_pass_write_index: Some(b + Q_AO + 1) });
        let dt = depth.texture();
        let usable = self.options.ao && dt.sample_count() == 1 && dt.usage().contains(wgpu::TextureUsages::TEXTURE_BINDING) && dt.format() == crate::DEPTH_FORMAT;
        if !usable {
            self.state.lock().unwrap().ao_made = None;
            enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("ao (off)"), timestamp_writes: tw });
            return;
        }
        let size = ((dt.width() / 2).max(1), (dt.height() / 2).max(1));
        let mut created = false;
        let mut ao = self.ao.lock().unwrap();
        if ao.as_ref().map(|t| t.size) != Some(size) {
            // (`--screen-gi`) the view depth is copied into the history: its target can be a copy's source.
            let z_extra = if self.ao_gi.is_some() { wgpu::TextureUsages::COPY_SRC } else { wgpu::TextureUsages::empty() };
            let ao_view = ao_texture(&self.device, AO_FORMAT, size);
            let z_view = ao_texture_with(&self.device, AO_Z_FORMAT, size, z_extra);
            let gi = self.ao_gi.as_ref().map(|p| {
                let tmp = ao_texture_with(&self.device, AO_FORMAT, size, wgpu::TextureUsages::COPY_SRC);
                let hist = ao_texture_with(&self.device, AO_FORMAT, size, wgpu::TextureUsages::COPY_DST);
                let hist_z = ao_texture_with(&self.device, AO_Z_FORMAT, size, wgpu::TextureUsages::COPY_DST);
                let filter_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("ao-gi-filter-bg"),
                    layout: &p.filter_bgl,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&tmp) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&z_view) },
                        wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&ao_view) },
                    ],
                });
                AoGiTarget { tmp, hist, hist_z, filter_bind, bind: None }
            });
            *ao = Some(AoTarget { size, ao_view, z_view, bind: None, gi });
            created = true;
        }
        let t = ao.as_mut().unwrap();
        // (`--screen-gi`) the gather, its filter and the history's copies, in place of the 4 m search.
        if let (Some(p), Some(g), true) = (self.ao_gi.as_ref(), t.gi.as_mut(), self.options.screen_gi) {
            let ht = hdr.texture();
            if g.bind.as_ref().is_none_or(|(d, h, _)| d != dt || h != ht) {
                crate::hitch::mark(crate::hitch::Mark::BindGroupCreated, 1);
                let far = self.ao_sun.then(|| self.lighting_bound.lock().unwrap().far_shadow_map.clone());
                let mut entries = vec![
                    wgpu::BindGroupEntry { binding: 0, resource: self.look_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(depth) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(hdr) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&g.tmp) },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&t.z_view) },
                ];
                if let Some(f) = far.as_ref() {
                    entries.push(wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(f) });
                }
                entries.push(wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&g.hist) });
                entries.push(wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(&g.hist_z) });
                entries.push(wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::Sampler(&self.ao_sampler) });
                let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("ao-gi-bg"), layout: &p.gather_bgl, entries: &entries });
                g.bind = Some((dt.clone(), ht.clone(), bg));
            }
            let tw0 = ts.map(|(q, b)| wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(b + Q_AO), end_of_pass_write_index: None });
            let tw1 = ts.map(|(q, b)| wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: None, end_of_pass_write_index: Some(b + Q_AO + 1) });
            {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("ao gi"), timestamp_writes: tw0 });
                pass.set_pipeline(&p.gather);
                pass.set_bind_group(0, &g.bind.as_ref().unwrap().2, &[]);
                pass.dispatch_workgroups(size.0.div_ceil(8), size.1.div_ceil(8), 1);
            }
            {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("ao gi filter"), timestamp_writes: tw1 });
                pass.set_pipeline(&p.filter);
                pass.set_bind_group(0, &g.filter_bind, &[]);
                pass.dispatch_workgroups(size.0.div_ceil(8), size.1.div_ceil(8), 1);
            }
            let extent = wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 };
            enc.copy_texture_to_texture(g.tmp.texture().as_image_copy(), g.hist.texture().as_image_copy(), extent);
            enc.copy_texture_to_texture(t.z_view.texture().as_image_copy(), g.hist_z.texture().as_image_copy(), extent);
            drop(ao);
            if created {
                self.rebuild_main_bg();
            }
            let mut st = self.state.lock().unwrap();
            st.ao_made = st.cur_cam.map(|(vp, eye, fwd, _)| (vp, eye, fwd));
            return;
        }
        let ht = hdr.texture();
        if t.bind.as_ref().is_none_or(|(d, h, _)| d != dt || h != ht) {
            crate::hitch::mark(crate::hitch::Mark::BindGroupCreated, 1);
            // (`--sun-bounce`) the far sun map bound now (or its stand-in); `set_far_shadow_map` drops this group.
            let far = self.ao_sun.then(|| self.lighting_bound.lock().unwrap().far_shadow_map.clone());
            let mut entries = vec![
                wgpu::BindGroupEntry { binding: 0, resource: self.look_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(depth) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(hdr) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&t.ao_view) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&t.z_view) },
            ];
            if let Some(f) = far.as_ref() {
                entries.push(wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(f) });
            }
            let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("ao-bg"), layout: &self.ao_bgl, entries: &entries });
            t.bind = Some((dt.clone(), ht.clone(), bg));
        }
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("ao"), timestamp_writes: tw });
            pass.set_pipeline(&self.ao_pipeline);
            pass.set_bind_group(0, &t.bind.as_ref().unwrap().2, &[]);
            pass.dispatch_workgroups(size.0.div_ceil(8), size.1.div_ceil(8), 1);
        }
        drop(ao);
        if created {
            self.rebuild_main_bg();
        }
        let mut st = self.state.lock().unwrap();
        st.ao_made = st.cur_cam.map(|(vp, eye, fwd, _)| (vp, eye, fwd));
    }

    /// Bytes of the occlusion targets, once they exist.
    pub fn ao_bytes(&self) -> u64 {
        // (`--screen-gi`) + the scratch target and the history (8 + 8 + 4 bytes a texel).
        self.ao.lock().unwrap().as_ref().map(|t| t.size.0 as u64 * t.size.1 as u64 * (8 + 4 + if t.gi.is_some() { 20 } else { 0 })).unwrap_or(0)
    }

    /// Group 1 of the city pipelines.
    pub fn main_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.main_bgl
    }

    /// Re-allocate the shadow map at another size (quality setting).
    pub fn set_options(&mut self, device: &wgpu::Device, options: LookOptions) {
        if options.shadow_map != self.options.shadow_map {
            let (layers, array, bytes) = shadow_texture(device, options.shadow_map.max(256));
            let old = self.options.shadow_map as u64 * self.options.shadow_map as u64 * 2 * CASCADES as u64;
            self.bytes = self.bytes - old + bytes;
            self.shadow_layers = layers;
            self.shadow_array = array;
            let _ = device;
            self.rebuild_main_bg();
        }
        if !options.ao {
            self.state.lock().unwrap().ao_made = None;
        }
        if options.shadow_slope_bias != self.options.shadow_slope_bias {
            self.state.lock().unwrap().force_all = true; // the maps were drawn with the other bias
        }
        // the stages and the weather; a new weather rebuilds the frame.
        self.set_sky_options(options.sky);
        // `--smooth-time`: the exposure's snaps and the histories' recuts (sky.rs);
        // turned off, the hour goes back to the hard path.
        let smooth = options.lighting.smooth_time;
        self.sky.set_smooth_time(smooth);
        // `--light-shadows`: the street lamps in SF's 3000 K LED colour (sky.rs `led_params`).
        self.sky.set_led_lamps(options.lighting.light_shadows > 0);
        if smooth != self.options.lighting.smooth_time {
            let mut s = self.state.lock().unwrap();
            s.frame = None;
            s.sky_base = None;
            if !smooth {
                s.soft = false;
                s.keys.clear();
            }
        }
        // `--photo-finish`: the photo pass, made the first time it is on (its bind
        // groups follow at the next `hdr_target`); the sky's air and bloom (sky.rs).
        if options.photo_finish > 0 && self.photo.is_none() {
            self.photo = Some(crate::photo::PhotoGpu::new(device, self.output_format));
        }
        self.sky.set_photo_finish(options.photo_finish);
        // `--look cinematic`: the cinematic post pass, made the first time the look is on.
        if options.look.is_cinematic() && self.cine.is_none() {
            self.cine = Some(crate::cine_post::CineGpu::new(device, self.output_format));
        }
        if options.look != self.options.look {
            self.state.lock().unwrap().frame = None; // the look's night exposure and fill (`cinematic_night`)
        }
        self.options = options;
    }

    /// the output size the post targets were made for (1920 x 1080 before the first frame).
    pub fn out_size(&self) -> (u32, u32) {
        self.post.lock().unwrap().as_ref().map(|t| t.size).unwrap_or((1920, 1080))
    }

    /// whether the passes after the city pass need the scene's depth.
    pub fn sky_wants_depth(&self) -> bool {
        let o = self.sky.options();
        o.any_post() || o.ssr
    }

    /// change the sky stages or the weather at run time (A/B, captures).
    pub fn set_sky_options(&self, o: crate::sky::SkyOptions) {
        if self.sky.options() != o {
            self.sky.set_options(o);
            let mut s = self.state.lock().unwrap();
            s.frame = None;
            s.weather_key = (i64::MIN, 0, 0);
            s.force_all = true;
        }
    }

    /// record whether this frame's depth is usable by the sky passes
    /// (single-sample, bindable); the next update writes it into the uniform.
    pub fn set_sky_depth_ok(&self, ok: bool) {
        self.sky_depth_ok.store(ok, std::sync::atomic::Ordering::Relaxed);
    }

    /// the lights' gather and clusters (before the city pass).
    pub fn encode_sky_lights(&self, enc: &mut wgpu::CommandEncoder, agents: Option<&crate::agents::AgentsGpu>, props: Option<&crate::props::PropsGpu>, ts: Option<(&wgpu::QuerySet, u32)>) {
        self.sky.bind_props(props.map(|p| (p.lights_buffer(), p.light_count_buffer())));
        self.sky.bind_agents(agents.map(|a| a.sky_buffers()));
        self.sky.encode_lights(enc, ts);
    }

    /// after the occlusion pass: luminaires and rain, the agents'
    /// velocity, the volume, the resolve and the bloom; post then reads the
    /// resolved frame.
    pub fn encode_sky_after(&self, enc: &mut wgpu::CommandEncoder, depth: &wgpu::TextureView, hdr: &wgpu::TextureView, agents: Option<&crate::agents::AgentsGpu>, ts: Option<(&wgpu::QuerySet, u32)>) {
        self.encode_sky_after_with(enc, depth, hdr, agents, ts, &mut |_, _| {});
    }

    /// `encode_sky_after` with a hook between the volume and the resolve (the
    /// renderer's lit fog, see `SkyGpu::encode_after` for what `mid` may do).
    pub fn encode_sky_after_with(
        &self,
        enc: &mut wgpu::CommandEncoder,
        depth: &wgpu::TextureView,
        hdr: &wgpu::TextureView,
        agents: Option<&crate::agents::AgentsGpu>,
        ts: Option<(&wgpu::QuerySet, u32)>,
        mid: &mut dyn FnMut(&mut wgpu::CommandEncoder, bool),
    ) {
        let dt = depth.texture();
        let depth_now = dt.sample_count() == 1 && dt.usage().contains(wgpu::TextureUsages::TEXTURE_BINDING) && dt.format() == crate::DEPTH_FORMAT;
        self.set_sky_depth_ok(depth_now);
        let r = self.sky.encode_after(enc, &self.look_buf, &self.shadow_array, &self.cmp_sampler, depth, hdr, agents, self, ts, depth_now, mid);
        self.post_source.store(r.map(|p| p as u32 + 1).unwrap_or(0), std::sync::atomic::Ordering::Relaxed);
    }

    /// The clock (hours, 0..24); the sky model and light rig follow on the next update.
    pub fn set_time_of_day(&self, hour: f64) {
        let mut s = self.state.lock().unwrap();
        // (a soft path in force hands over: the frame is built at this hour, not blended)
        if s.hour != hour || s.soft {
            s.hour = hour;
            s.frame = None;
            s.force_all = true;
        }
        s.soft = false;
    }

    /// `--smooth-time`'s soft path: the clock has moved on to `hour` (wrapped
    /// to 0..24). The look is the blend of the keyframes either side of it
    /// (`smooth_time::Keys`, at most one built on this thread a frame); no
    /// cascade refit is forced (they refit once the key light has turned
    /// `smooth_time::REFIT_DEG` from a cascade's fit). Without the option it
    /// is `set_time_of_day`.
    pub fn set_time_of_day_smooth(&self, hour: f64) {
        if !self.options.lighting.smooth_time {
            return self.set_time_of_day(hour);
        }
        let hour = hour.rem_euclid(24.0);
        let mut s = self.state.lock().unwrap();
        if s.hour != hour || !s.soft {
            s.hour = hour;
            s.frame = None;
        }
        s.soft = true;
    }

    /// Keyframes the soft path built on the render thread, and took from its worker.
    pub fn smooth_time_keys(&self) -> (u64, u64) {
        let s = self.state.lock().unwrap();
        (s.keys.built, s.keys.prefetched)
    }

    /// the look uniform buffer (the probe's sky fill reads it).
    pub fn look_buffer(&self) -> &wgpu::Buffer {
        &self.look_buf
    }

    /// the rig follows the scene probe's measured transfer (None:
    /// the JS fallback yield). The frame is rebuilt only when it moves by more
    /// than 0.005 (the readback lands once per capture).
    pub fn set_transfer(&self, transfer: Option<f64>) {
        let mut s = self.state.lock().unwrap();
        let changed = match (s.transfer, transfer) {
            (Some(a), Some(b)) => (a - b).abs() > 0.005,
            (None, None) => false,
            _ => true,
        };
        if changed {
            s.transfer = transfer;
            // Re-solve the fill only (no sky integration on the render thread).
            // a weather frame carries its own fill terms: rebuilt instead.
            // (`--smooth-time`: the cubic rig; the soft path's keyframes follow the transfer.)
            s.frame = if s.terms.is_some() || s.soft {
                None
            } else if self.options.lighting.smooth_time {
                s.frame.map(|f| f.retarget_interp(transfer, eo_look::rig::Interp::MonotoneCubic))
            } else {
                s.frame.map(|f| f.retarget(transfer))
            };
        }
    }

    pub fn transfer(&self) -> Option<f64> {
        self.state.lock().unwrap().transfer
    }

    pub fn time_of_day(&self) -> f64 {
        self.state.lock().unwrap().hour
    }

    /// The range fog's far end the passes apply (the look uniform's `fog.y`):
    /// pushed out of reach (2e7 m) when the volume carries the aerial
    /// perspective (the atmosphere on and the sky's terms in), else the
    /// frame's aerial far end. Stage L5 (the cinematic look): the sea's "all
    /// fogged" test reads this, not the aerial far end the volume replaced.
    pub fn fog_far_in_use(&self) -> f32 {
        let atmo_ap = self.sky.options().atmosphere && self.state.lock().unwrap().terms.is_some();
        if atmo_ap { 2.0e7 } else { self.frame().aerial.far as f32 }
    }

    /// The look frame in force (built on demand). sky: with any sky stage on,
    /// the frame of the weather in force (its sky model, the weather's fill,
    /// the moon key, the night exposure; eo-sky `frame::look_frame`), rebuilt
    /// when the weather moves; with every stage off, the JS clear frame as before.
    pub fn frame(&self) -> LookFrame {
        let opts = self.sky.options(); // sky
        let legacy = opts == crate::sky::SkyOptions::off();
        let w = if legacy { None } else { Some(self.sky.weather()) };
        // the exposure the frame was drawn with (metered, adapted), for the frame's readers.
        let drawn = if legacy { None } else { self.sky.drawn_exposure() };
        let mut s = self.state.lock().unwrap();
        let (h, t) = (s.hour, s.transfer);
        // `--smooth-time`: the cubic rig, the blended moon key, the ramped vehicle lamps.
        let smooth = self.options.lighting.smooth_time;
        // (`--look cinematic`, stage L4) the golden hour applied to the frame (eo-sky `apply_with`).
        let fo = eo_sky::frame::FrameOptions { smooth, cinematic: opts.cinematic };
        if let Some(w) = w {
            let key = crate::sky::weather_key(&w);
            if s.weather_key != key {
                s.weather_key = key;
                s.frame = None;
            }
            if s.frame.is_none() && s.soft {
                // the soft path: the blend of the keyframes either side of the hour.
                let inputs = crate::smooth_time::KeyInputs { weather: Some(w), weather_key: key, moon_day: opts.moon_day, transfer: t, cinematic: opts.cinematic };
                let fi = s.frame_index;
                let (mut f, mut terms) = s.keys.frame(h, inputs, fi, true);
                self.cinematic_night(&mut f, terms.as_mut());
                self.night_photometry(opts.night_photometry, &mut f, terms.as_mut()); // (`--night-photometry`)
                s.frame = Some(f);
                s.terms = terms;
            }
            if s.frame.is_none() {
                let _span = crate::hitch::span(crate::hitch::Phase::LookFrame);
                crate::hitch::mark(crate::hitch::Mark::LookFrameRebuilt, 1);
                let c = eo_sky::Conditions { hour: h, weather: w, moon_day: opts.moon_day };
                // the sky is integrated once per hour and weather; a new
                // probe transfer (each capture) re-solves only the fill from it.
                let mut profile = format!("{:?}", eo_sky::weather::sky_profile(&w.profile));
                if smooth {
                    profile.push_str(" smooth"); // a base of the cubic rig
                }
                if s.sky_base.as_ref().is_none_or(|(bh, bp, _)| *bh != h || *bp != profile) {
                    s.sky_base = Some((h, profile, eo_sky::frame::base_frame_with(&c, t, fo)));
                }
                let (mut f, mut terms) = eo_sky::frame::look_frame_from_with(&s.sky_base.as_ref().unwrap().2, &c, t, fo);
                self.cinematic_night(&mut f, Some(&mut terms));
                self.night_photometry(opts.night_photometry, &mut f, Some(&mut terms)); // (`--night-photometry`)
                s.frame = Some(f);
                s.terms = Some(terms);
            }
            let mut fr = s.frame.unwrap();
            if let Some(e) = drawn {
                fr.exposure = e; // sky
            }
            return fr;
        }
        s.terms = None;
        if s.frame.is_none() && s.soft {
            let inputs = crate::smooth_time::KeyInputs { weather: None, weather_key: (0, 0, 0), moon_day: opts.moon_day, transfer: t, cinematic: opts.cinematic };
            let fi = s.frame_index;
            let (f, _) = s.keys.frame(h, inputs, fi, true);
            s.frame = Some(f);
        }
        *s.frame.get_or_insert_with(|| {
            let _span = crate::hitch::span(crate::hitch::Phase::LookFrame);
            crate::hitch::mark(crate::hitch::Mark::LookFrameRebuilt, 1);
            if smooth {
                LookFrame::with_sky_profile_interp(h, t, eo_look::sky::Profile::clear_with_look(&eo_look::look::LOOK), eo_look::rig::Interp::MonotoneCubic)
            } else {
                LookFrame::with_transfer(h, t)
            }
        })
    }

    /// (`--look cinematic`) the cinematic post pass, once the look has been on.
    pub fn cine(&self) -> Option<&crate::cine_post::CineGpu> {
        self.cine.as_ref()
    }

    /// (`--look cinematic`, stage L1) the night of the look on a frame `frame` has
    /// built: the night fill keeps more of its colour (eo-sky
    /// `night::cinematic_night_fill`) and the exposure's night share takes the
    /// cinematic night solve (`night::cinematic_night_shift`: unlit ground at
    /// 3.5/255 through the cinematic display). Nothing without the look.
    fn cinematic_night(&self, f: &mut LookFrame, terms: Option<&mut eo_sky::SkyTerms>) {
        if !self.options.look.is_cinematic() {
            return;
        }
        let Some(t) = terms else { return };
        eo_sky::night::cinematic_night_fill(f, t.nightness);
        // only the night solve's share moves (the day part is the frame's, stage L4's included).
        t.exposure_target += eo_sky::night::cinematic_night_shift(f);
        f.exposure = t.exposure_target;
    }

    /// (`--night-photometry`) the night on one photometric scale on a frame
    /// `frame` has built (after `cinematic_night`): the lamps' units per lux
    /// with the practicals' gain and the exposure's night share metered on the
    /// lit street (eo-sky `night_photometry::apply`). Nothing without the switch.
    fn night_photometry(&self, on: bool, f: &mut LookFrame, terms: Option<&mut eo_sky::SkyTerms>) {
        if !on {
            return;
        }
        let Some(t) = terms else { return };
        eo_sky::night_photometry::apply(f, t, self.options.look.is_cinematic());
    }

    /// the weather and night terms of the frame in force (None with every sky stage off).
    pub fn sky_terms(&self) -> Option<eo_sky::SkyTerms> {
        let _ = self.frame();
        self.state.lock().unwrap().terms
    }

    /// The shadow counter buffer (see `SHADOW_COUNTER_WORDS`).
    pub fn shadow_counter_buffer(&self) -> &wgpu::Buffer {
        &self.shadow_counters
    }

    /// Write this frame's uniforms (the look, the cascade boxes and matrices).
    pub fn update(&self, queue: &wgpu::Queue, cam: &Camera, copy_step: f32) {
        let f = self.frame();
        let fwd = cam.forward();
        let right = cam.right();
        let up = right.cross(fwd);
        let to = |v: glam::Vec3| [v.x as f64, v.y as f64, v.z as f64];
        let cc = CascadeCamera {
            eye: to(cam.eye),
            forward: to(fwd),
            right: to(right),
            up: to(up),
            fov_y_deg: cam.fov_y_deg as f64,
            aspect: cam.aspect as f64,
            near: cam.near as f64,
            far: cam.far as f64,
        };
        let fitted = shadow::fit_cascades(&cc, f.key_dir, self.options.shadow_map);
        let shadows_on = self.options.shadows && f.shadow_strength > 0.0;
        // Which cascades the coming encode refreshes; the others keep the box
        // their depth map was rendered with (committed in `encode_shadows`).
        let mut st = self.state.lock().unwrap();
        st.frame_index = st.frame_index.wrapping_add(1);
        let jumped = st.last_eye.is_none_or(|e| e.distance(cam.eye) > CONVERGE_JUMP_M);
        // `--smooth-time`: with the hour moving, every cascade at once only when the
        // key light has turned `REFIT_DEG` from one cascade's fit (the staggered
        // refresh below fits one a frame with this frame's light).
        let smooth = self.options.lighting.smooth_time;
        let turned = smooth && st.cascade_dirs.iter().any(|d| d.is_none_or(|d| crate::smooth_time::angle_deg(d, f.key_dir) > crate::smooth_time::REFIT_DEG));
        let mask = if st.force_all || jumped || turned {
            (1u32 << CASCADES) - 1
        } else {
            1u32 << REFRESH_SCHEDULE[st.rotation % REFRESH_SCHEDULE.len()]
        };
        if smooth {
            for c in 0..CASCADES {
                if mask & (1 << c) != 0 {
                    st.cascade_dirs[c] = Some(f.key_dir);
                }
            }
        }
        let mut cascades = st.last_cascades;
        for (c, fit) in fitted.iter().enumerate() {
            if mask & (1 << c) != 0 {
                cascades[c] = *fit;
            }
        }
        st.mask = mask;
        st.pending = Some((cascades, cam.eye));
        let frame_index = st.frame_index;
        // the occlusion result in the texture was made with `ao_made`;
        // this frame's camera is what the coming AO pass will run with.
        let ao_made = if self.options.ao { st.ao_made } else { None };
        st.cur_cam = Some((cam.view_proj(), cam.eye, fwd, cam.fov_y_deg.to_radians()));
        let terms = st.terms; // sky
        drop(st);
        // the sky passes' uniform (unjittered camera) and the exposure, adapted.
        let out_size = self.post.lock().unwrap().as_ref().map(|t| t.size).unwrap_or((1920, 1080));
        let exposure = match terms {
            Some(t) => {
                let _span = crate::hitch::span(crate::hitch::Phase::SkyUpdate);
                self.sky.update(queue, &Camera { jitter: [0.0; 2], ..*cam }, &f, &t, out_size, self.sky_depth_ok.load(std::sync::atomic::Ordering::Relaxed))
            }
            None => f.exposure,
        };
        let out_h = self.post.lock().unwrap().as_ref().map(|t| t.size.1).unwrap_or(1080) as f32;
        let focal_px = 0.5 * out_h / (0.5 * cam.fov_y_deg.to_radians()).tan();
        let m = &f.model;
        let fld = &m.field;
        let fog = f.fog_color(fwd.x as f64, fwd.z as f64);
        let rot = glam::camera::rh::view::look_to_mat4(glam::Vec3::ZERO, fwd, glam::Vec3::Y);
        let inv = (cam.proj() * rot).inverse();
        let g = m.grade;
        let look = eo_look::look::LOOK;
        let mut sh = [[0f32; 4]; 9];
        for (k, c) in f.env_sh.iter().enumerate() {
            sh[k] = v4(*c, 0.0);
        }
        let split_far = [fitted[0].split_far, fitted[1].split_far, fitted[2].split_far];
        let fade_start = split_far[2] * 0.75;
        // over the marine layer (fog weather, the eye 50-250 m above its
        // top: Twin Peaks, the towers, the air) the sky is the clear sky the fog
        // lies under: the dome's weather terms fade out and its gain makes the
        // zenith the clear hour's (eo-sky `above_fog_gain`); below, the fog's grey dome.
        let above = match terms {
            Some(t) if t.weather.kind == eo_sky::Kind::Fog => {
                let top = t.decks.low_top_m.max(t.decks.marine_top_m + t.decks.marine_swing_m);
                eo_look::math::smoothstep(top + 50.0, top + 250.0, cam.eye.y as f64)
            }
            _ => 0.0,
        };
        let wx_keep = 1.0 - above;
        let above_gain = 1.0 + (terms.map(|t| t.above_fog_gain).unwrap_or(1.0) - 1.0) * above;
        // clear air's aerial perspective is the volume's; a weather that veils (fog,
        // drizzle, rain, storm: visibility under clear air's) keeps the range fog in every
        // pass (the far land and other late passes take it too).
        // every weather: the veiling weathers' fog is the volume's too (sky.rs `atmo3.x`).
        let atmo_ap = self.sky.options().atmosphere && terms.is_some();
        let u = LookUniform {
            key_dir: v4(f.key_dir, if shadows_on { f.shadow_strength } else { 0.0 }),
            key: v4(f.key, exposure), // adapted (f.exposure with every sky stage off)
            rim_dir: v4(f.rim_dir, 0.0),
            rim: v4(f.rim, 0.0),
            hemi_sky: v4(f.hemi_sky, 0.0),
            hemi_ground: v4(f.hemi_ground, 0.0),
            ambient: v4(f.ambient, f.rig.environment_intensity),
            env_class: [f.rig.env_painted as f32, f.rig.env_glass as f32, f.rig.env_asphalt as f32, f.rig.env_ground as f32],
            sh,
            // with the atmosphere tables the volume pass carries the aerial
            // perspective (sky_volume `vol_aerial`) for the city, the far land and
            // the sea: the range fog every pass applies is pushed out of reach.
            // z: (`--night-photometry`) the practicals' gain (eo-sky `night_photometry::practical_gain`; 0 without it).
            fog: { let g = night_gain(self.sky.options().night_photometry && terms.is_some(), &f); if atmo_ap { [1.0e7, 2.0e7, g, 0.0] } else { [f.aerial.near as f32, f.aerial.far as f32, g, 0.0] } },
            fog_color: v4(fog, 0.0),
            cam_pos: [cam.eye.x, cam.eye.y, cam.eye.z, 1.0],
            cam_fwd: [fwd.x, fwd.y, fwd.z, 0.0],
            inv_view_proj: inv.to_cols_array_2d(),
            cascade_vp: [cascades[0].view_proj, cascades[1].view_proj, cascades[2].view_proj],
            cascade_split: [split_far[0] as f32, split_far[1] as f32, split_far[2] as f32, fade_start as f32],
            // w (`--soft-shadows`, while it runs) the cascade's light-space depth range, metres.
            cascade_bias: cascades.map(|c| [c.normal_bias as f32, (-c.depth_bias_m / c.depth_range) as f32, c.texel_world as f32, if self.options.soft_shadows { c.depth_range as f32 } else { 0.0 }]),
            sun: v4(m.sun.dir(), fld.state.sun_e),
            beta_r: v4(fld.state.beta_r, fld.state.horizon_up),
            beta_m: v4(fld.state.beta_m, fld.state.g),
            beta_sum: v4(fld.state.beta_sum, fld.grade),
            mean: v4(fld.mean, fld.twilight),
            tw: [fld.night as f32, fld.ozone as f32, fld.belt as f32, fld.shadow_top_sin as f32],
            tw2: [fld.deep_twilight as f32, fld.airglow as f32, m.profile.urban_glow as f32, m.profile.overcast as f32],
            horizon: v4(m.horizon_radiance, m.profile.turbidity),
            // the street field's unlit ground x `NIGHT_GROUND_FILL` by nightness, as the
            // surfaces' night fill (eo-sky frame.rs); its lamp pools and shop spill (`field`) keep theirs.
            ground: v4(m.ground_radiance.map(|v| v * (1.0 + (eo_sky::frame::NIGHT_GROUND_FILL - 1.0) * eo_sky::night::nightness(m.sun.altitude_deg))), if g.is_some() { 1.0 } else { 0.0 }),
            dg0: g.map(|g| [g.weight, g.veil_luminance, g.zenith_desaturation, g.horizon_desaturation].map(|x| x as f32)).unwrap_or_default(),
            dg1: g.map(|g| [g.horizon_lift, g.dome_gain * above_gain, g.skyline_share, g.low_sun_share].map(|x| x as f32)).unwrap_or([0.0, 1.0, 0.0, 0.0]),
            dg2: [
                g.map(|g| g.sunward_horizon_desaturation).unwrap_or(0.0) as f32,
                look.dome_gradient_power as f32,
                look.skyline_band_width as f32,
                look.skyline_band_peak as f32,
            ],
            haze_tint: v4(look.haze_tint, 0.0),
            veil_tint: v4(look.veil_tint, 0.0),
            disc: v4(f.disc, f.disc_radius.cos()),
            post0: [tone::aces_neutral_inverse(tone::HIGHLIGHT_START_DISPLAY) as f32, tone::HIGHLIGHT_SCALE as f32, look.grade_saturation as f32, look.grade_contrast as f32],
            post1: [look.grade_brightness as f32, look.finish_balance as f32, look.finish_vibrance as f32, look.finish_vignette as f32],
            post2: [look.finish_density as f32, 1.0 / 255.0, if self.output_format.is_srgb() { 1.0 } else { 0.0 }, (frame_index % 64) as f32],
            shadow_tint: v4(look.finish_shadow_tint, 0.0),
            highlight_tint: v4(look.finish_highlight_tint, 0.0),
            ao_vp: ao_made.map(|m| m.0.to_cols_array_2d()).unwrap_or([[0.0; 4]; 4]),
            ao_cam: ao_made.map(|m| m.1.extend(1.0).to_array()).unwrap_or([0.0; 4]),
            ao_fwd: ao_made.map(|m| m.2.extend(0.0).to_array()).unwrap_or([0.0; 4]),
            ao_params: [
                if ao_made.is_some() { 1.0 } else { 0.0 },
                1.0 / focal_px,
                std::env::var("EO_AO_DEBUG").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0),
                // the occlusion's additions that run: bit 0 (`--sun-bounce`) the gather (it reads
                // ring 1 of the far sun map, in the occlusion pass), bit 1 (`--ao-albedo`) the albedo's multi-bounce.
                (if self.ao_sun && self.options.sun_bounce && self.options.ao && self.options.lighting.far_shadows > 0 { 1.0 } else { 0.0 }) + if self.options.ao_albedo { 2.0 } else { 0.0 },
            ],
            field: {
                // makeGroundField's practicals ramps on the sun's altitude (dry, model exposure 1).
                let gf = eo_look::ground_field::GroundField::get();
                let alt = m.sun.altitude_deg;
                let ss = |a: f64, b: f64, x: f64| {
                    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
                    t * t * (3.0 - 2.0 * t)
                };
                [(gf.lamp_pool * (1.0 - ss(-3.0, 8.0, alt))) as f32, (gf.shop_spill * (1.0 - ss(2.0, 16.0, alt))) as f32, 0.0, 0.0]
            },
            dome_zenith: v4(m.zenith_radiance, 0.0),
            // (`--look cinematic`, stage L2: more lit homes, the screens' share and flicker; rooms.rs `drive_look`)
            rooms_env: crate::rooms::drive_look(m.sun.altitude_deg, self.time_of_day(), smooth, self.options.look.is_cinematic()).0,
            rooms_work: crate::rooms::drive_look(m.sun.altitude_deg, self.time_of_day(), smooth, self.options.look.is_cinematic()).1,
            wx: [(m.profile.overcast * wx_keep) as f32, (m.profile.isotropy * wx_keep) as f32, (m.profile.desaturation * wx_keep) as f32, if m.ext.on { 1.0 } else { 0.0 }], // eo-look `SkyExt`
            cove: [if self.options.cove_sediment && self.options.cove_detail { 1.0 / eo_data::groundscale::scale() as f32 } else { 0.0 }, 0.0, 0.0, 0.0],
            wx_tint: v4(m.profile.tint.map(|v| 1.0 + (v - 1.0) * wx_keep), m.ext.afterglow), // w the afterglow arch
            lx: self.lighting_params(), // zeros while every lighting addition is off
        };
        queue.write_buffer(&self.look_buf, 0, bytemuck::bytes_of(&u));
        // `--photo-finish`: the photo pass's frame (the camera's ISO from the scene's light, the noise's seed, the lens).
        if let (Some(p), true) = (&self.photo, self.options.photo_finish > 0) {
            let (lamps, dome) = terms.map(|t| (t.lamps_on, t.weather.profile.brightness)).unwrap_or((0.0, 1.0));
            p.write(queue, &crate::photo::params(self.options.photo_finish, frame_index, out_size, cam, m.sun.altitude_deg, lamps, dome));
        }
        // `--look cinematic`: the cinematic pass's frame (the photo finish's lens when it is on;
        // the hour's grade, its table baked again only when the keys' weights move).
        if let (Some(c), true) = (&self.cine, self.options.look.is_cinematic()) {
            let (lamps, dome) = terms.map(|t| (t.lamps_on, t.weather.profile.brightness)).unwrap_or((0.0, 1.0));
            let photo = (self.options.photo_finish > 0).then(|| crate::photo::params(self.options.photo_finish, frame_index, out_size, cam, m.sun.altitude_deg, lamps, dome));
            let overcast = terms.map(|t| t.weather.profile.overcast).unwrap_or(0.0);
            c.write(queue, photo.as_ref(), m.sun.altitude_deg, f.hour, overcast, crate::sky::camera_on(&self.sky.options())); // (`--cine-camera`)
        }
        let mut planes = [[0f32; 4]; 18];
        for (c, cas) in cascades.iter().enumerate() {
            planes[c * 6..c * 6 + 6].copy_from_slice(&cas.planes);
        }
        let p = ShadowCullParams { planes, object_count: self.object_count, compact: self.compact as u32, mask, pad: 0 };
        queue.write_buffer(&self.cull_buf, 0, bytemuck::bytes_of(&p));
        for (c, cas) in cascades.iter().enumerate() {
            let mut bytes = [0f32; 20];
            for col in 0..4 {
                bytes[col * 4..col * 4 + 4].copy_from_slice(&cas.view_proj[col]);
            }
            bytes[16] = copy_step;
            queue.write_buffer(&self.view_buf, c as u64 * VIEW_STRIDE, bytemuck::cast_slice(&bytes));
        }
    }

    fn shadows_active(&self) -> bool {
        self.options.shadows && self.frame().shadow_strength > 0.0
    }

    /// Whether the sun (or moon) casts this frame (`--no-shadows`, a key with no shadow strength).
    pub(crate) fn shadows_on(&self) -> bool {
        self.shadows_active()
    }

    pub(crate) fn encode_shadow_cull(&self, enc: &mut wgpu::CommandEncoder, active: bool, ts: Option<(&wgpu::QuerySet, u32)>) {
        enc.clear_buffer(&self.shadow_counters, 0, None);
        {
            let tw = ts.map(|(q, b)| wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(b + Q_SHADOW_CULL), end_of_pass_write_index: Some(b + Q_SHADOW_CULL + 1) });
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("shadow-cull"), timestamp_writes: tw });
            if active {
                pass.set_pipeline(&self.cull_pipeline);
                pass.set_bind_group(0, &self.cull_bg, &[]);
                pass.dispatch_workgroups(self.object_count.div_ceil(64), 1, 1);
            }
        }
    }

    /// Shadow cull + one depth pass per cascade (after the camera's cull pass).
    pub fn encode_shadows(&self, enc: &mut wgpu::CommandEncoder, vertex: &wgpu::Buffer, index: &wgpu::Buffer, ts: Option<(&wgpu::QuerySet, u32)>) {
        let active = self.shadows_active();
        // Commit what the last update fitted: these boxes are what the maps
        // will hold after this encode.
        let mask = {
            let mut st = self.state.lock().unwrap();
            let mask = st.mask;
            if let Some((cascades, eye)) = st.pending.take() {
                if active {
                    st.last_cascades = cascades;
                    st.last_eye = Some(eye);
                    if mask == (1u32 << CASCADES) - 1 {
                        st.force_all = false;
                    } else {
                        st.rotation += 1;
                    }
                } else {
                    st.force_all = true;
                }
            }
            if active { mask } else { 0 }
        };
        crate::hitch::mark(crate::hitch::Mark::CascadesRefreshed, mask.count_ones() as u64);
        self.encode_shadow_cull(enc, active, ts);
        let n = self.object_count;
        // Only the refreshed cascades get a pass; the others keep their maps.
        // With nothing to refresh, one empty pass still carries the timestamps.
        let list: Vec<usize> = (0..CASCADES).filter(|c| mask & (1 << c) != 0).collect();
        let passes: Vec<Option<usize>> = if list.is_empty() { vec![None] } else { list.iter().map(|c| Some(*c)).collect() };
        for (k, pc) in passes.iter().enumerate() {
            let tw = ts.and_then(|(q, b)| {
                let begin = (k == 0).then_some(b + Q_SHADOW_DRAW);
                let end = (k == passes.len() - 1).then_some(b + Q_SHADOW_DRAW + 1);
                (begin.is_some() || end.is_some()).then_some(wgpu::RenderPassTimestampWrites { query_set: q, beginning_of_pass_write_index: begin, end_of_pass_write_index: end })
            });
            let Some(c) = *pc else {
                enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("shadow-depth (none this frame)"),
                    timestamp_writes: tw.map(|t| wgpu::ComputePassTimestampWrites {
                        query_set: t.query_set,
                        beginning_of_pass_write_index: t.beginning_of_pass_write_index,
                        end_of_pass_write_index: t.end_of_pass_write_index,
                    }),
                });
                continue;
            };
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow-depth"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_layers[c],
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: tw,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(if self.options.shadow_slope_bias { &self.depth_pipeline_sloped } else { &self.depth_pipeline });
            pass.set_bind_group(0, &self.depth_bg, &[(c as u64 * VIEW_STRIDE) as u32]);
            pass.set_vertex_buffer(0, vertex.slice(..));
            pass.set_index_buffer(index.slice(..), wgpu::IndexFormat::Uint32);
            let offset = c as u64 * n as u64 * crate::DRAW_ARGS_BYTES;
            if self.compact {
                crate::counted::draw_indexed(&mut pass, self.shadow_lists.as_ref(), c, &self.shadow_draws, offset, &self.shadow_counters, c as u64 * 4, n);
            } else {
                pass.multi_draw_indexed_indirect(&self.shadow_draws, offset, n);
            }
        }
    }

    pub fn bind_main(&self, pass: &mut wgpu::RenderPass<'_>) {
        let bg = self.main_bg.lock().unwrap()[self.sky.parity()].clone(); // the previous frame for this parity
        pass.set_bind_group(1, &bg, &[]);
    }

    /// The sky, inside the city pass, after the opaque list.
    pub fn draw_sky(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.sky_pipeline);
        pass.set_bind_group(0, &self.sky_bg, &[]);
        pass.draw(0..3, 0..1);
    }

    /// The HDR scene target (single-sample; the MSAA resolve target when MSAA is on) for an output size.
    pub fn hdr_target(&self, w: u32, h: u32) -> wgpu::TextureView {
        let device = &self.device;
        let mut p = self.post.lock().unwrap();
        if p.as_ref().map(|t| t.size) != Some((w, h)) {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("hdr-scene"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: HDR_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = tex.create_view(&Default::default());
            // the resolved histories and the bloom, of the same size.
            self.sky.ensure_targets((w, h), HDR_FORMAT);
            let bloom = self.sky.bloom_view().unwrap_or_else(|| self.sky.dummy_hist.clone());
            let make = |src: &wgpu::TextureView| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("post-bg"),
                    layout: &self.post_bgl,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: self.look_buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(src) },
                        wgpu::BindGroupEntry { binding: 2, resource: self.sky.uniform.as_entire_binding() }, // sky
                        wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&bloom) },
                        wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(self.sky.linear_sampler()) },
                    ],
                })
            };
            let h0 = self.sky.history_view(0).unwrap_or_else(|| self.sky.dummy_hist.clone());
            let h1 = self.sky.history_view(1).unwrap_or_else(|| self.sky.dummy_hist.clone());
            let bind = [make(&view), make(&h0), make(&h1)];
            *p = Some(PostTarget { size: (w, h), view: view.clone(), bind, srcs: [view, h0, h1], bloom, photo_bind: None, cine_bind: None });
            self.ensure_photo_bind(p.as_mut().unwrap());
            drop(p);
            self.rebuild_main_bg(); // the city pass reads the new histories
            return self.post.lock().unwrap().as_ref().unwrap().view.clone();
        }
        self.ensure_photo_bind(p.as_mut().unwrap());
        p.as_ref().unwrap().view.clone()
    }

    /// (`--photo-finish`) the photo pass's bind groups over the post's sources, once its pipeline exists.
    fn ensure_photo_bind(&self, t: &mut PostTarget) {
        if let (Some(photo), None) = (&self.photo, &t.photo_bind) {
            let [a, b, c] = &t.srcs;
            t.photo_bind = Some(photo.bind_groups(&self.device, &self.look_buf, [a, b, c], &self.sky.uniform, &t.bloom, self.sky.linear_sampler()));
        }
        // (`--look cinematic`) and the cinematic pass's.
        if let (Some(cine), None) = (&self.cine, &t.cine_bind) {
            let [a, b, c] = &t.srcs;
            t.cine_bind = Some(cine.bind_groups(&self.device, &self.look_buf, [a, b, c], &self.sky.uniform, &t.bloom, self.sky.linear_sampler()));
        }
    }

    /// Bytes of the HDR target, once it exists.
    pub fn hdr_bytes(&self) -> u64 {
        self.post.lock().unwrap().as_ref().map(|t| t.size.0 as u64 * t.size.1 as u64 * 8).unwrap_or(0)
    }

    /// HDR target -> output.
    pub fn encode_post(&self, enc: &mut wgpu::CommandEncoder, out: &wgpu::TextureView, ts: Option<(&wgpu::QuerySet, u32)>) {
        self.encode_post_with_water(enc, out, ts, None);
    }

    pub(crate) fn encode_post_with_water(
        &self, enc: &mut wgpu::CommandEncoder, out: &wgpu::TextureView,
        ts: Option<(&wgpu::QuerySet, u32)>, water: Option<(&crate::water::WaterGpu, usize)>,
    ) {
        let p = self.post.lock().unwrap();
        let t = p.as_ref().expect("hdr_target before encode_post");
        // (`--cine-camera`) the camera model's meter over the post's source, before the pass reads it.
        if let (Some(c), true) = (&self.cine, self.options.look.is_cinematic()) {
            c.encode_camera(enc, self.post_source.load(std::sync::atomic::Ordering::Relaxed) as usize);
        }
        let tw = ts.map(|(q, b)| wgpu::RenderPassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(b + Q_POST), end_of_pass_write_index: Some(b + Q_POST + 1) });
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("post"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: out,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: tw,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let src = self.post_source.load(std::sync::atomic::Ordering::Relaxed) as usize; // sky
        // `--look cinematic`: the cinematic pass (cine_post.rs; it takes the photo finish's lens itself).
        let cine = match (&self.cine, &t.cine_bind, self.options.look.is_cinematic()) {
            (Some(c), Some(b), true) => Some((c, b)),
            _ => None,
        };
        match (&self.photo, &t.photo_bind, self.options.photo_finish > 0 && cine.is_none()) {
            // `--photo-finish`: the photo pass over the same source (photo.rs).
            (Some(photo), Some(bind), true) => {
                pass.set_pipeline(&photo.pipeline);
                pass.set_bind_group(0, &bind[src.min(2)], &[]);
            }
            _ if cine.is_some() => {}
            _ => {
                pass.set_pipeline(&self.post_pipeline);
                pass.set_bind_group(0, &t.bind[src.min(2)], &[]);
            }
        }
        if let Some((c, b)) = cine {
            pass.set_pipeline(&c.pipeline);
            pass.set_bind_group(0, &b[src.min(2)], &[]);
        }
        pass.draw(0..3, 0..1);
        if let Some((water, front)) = water {
            water.draw_submerged(&mut pass, self, front);
        }
    }

    // what the agent renderer (agents.rs) needs to cast into the cascades.
    /// The cascades the last `encode_shadows` refreshed (bit c), 0 when shadows are off.
    pub fn encoded_shadow_mask(&self) -> u32 {
        if self.shadows_active() { self.state.lock().unwrap().mask } else { 0 }
    }
    /// One depth view per cascade layer.
    pub fn shadow_layer_views(&self) -> &[wgpu::TextureView] {
        &self.shadow_layers
    }
    /// The cascade view-projections, 256 bytes a cascade (mat4 + copy step).
    pub fn shadow_view_buffer(&self) -> &wgpu::Buffer {
        &self.view_buf
    }
    // end agents

    /// Cascade far edges (view depth, metres) and texel sizes of the last update.
    pub fn cascade_summary(&self) -> Vec<(f64, f64)> {
        self.state.lock().unwrap().last_cascades.iter().map(|c| (c.split_far, c.texel_world)).collect()
    }
}
