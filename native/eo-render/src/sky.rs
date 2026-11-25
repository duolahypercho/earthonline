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

/// (`--far-street-light`, on unless `--no-far-street-light`) past the frame's lamp
/// lights' reach a street is lit from a map of every placed lamp's pool on the
/// ground (`bake_lamp_map`): lux every `LAMP_MAP_TEXEL_M` over a square of
/// `LAMP_MAP_SIZE` texels from `LAMP_MAP_ORIGIN_M` (x and z), read by draw.wgsl
/// where the clustered lights stop (look_light.wgsl `LK_LAMP_MAP_*`, `lk_far_street_light`).
pub const LAMP_MAP_SIZE: u32 = 3072;
pub const LAMP_MAP_TEXEL_M: f32 = 8.0;
pub const LAMP_MAP_ORIGIN_M: f32 = -12_288.0;

pub fn far_street_light_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| !std::env::args().any(|a| a == "--no-far-street-light"))
}

/// Each lamp's illuminance on the ground (lux) summed into the map: its
/// batwing (sky_light.wgsl's street head, `eo_sky::lights::street_lamp_lux`'s
/// shape) from its head's height (`candela / LAMP_POOL_PEAK_LUX`, the pool's
/// nadir the RP-8 peak), inverse square, the ground's cosine, out to the cobra
/// head's reach (`RANGE_MOUNTING_HEIGHTS`) under the shader's window.
pub fn bake_lamp_map(lamps: &[LampLight]) -> Vec<f32> {
    let n = LAMP_MAP_SIZE as usize;
    let mut map = vec![0.0f32; n * n];
    for l in lamps {
        if l.kind != LampKind::Street || l.candela <= 0.0 {
            continue;
        }
        let h = (l.candela / eo_sky::photometry::LAMP_POOL_PEAK_LUX).sqrt();
        let range = h * eo_sky::cine_lamps::RANGE_MOUNTING_HEIGHTS;
        let (fx, fz) = (((l.pos[0] as f32 - LAMP_MAP_ORIGIN_M) / LAMP_MAP_TEXEL_M) as f64, ((l.pos[2] as f32 - LAMP_MAP_ORIGIN_M) / LAMP_MAP_TEXEL_M) as f64);
        let rt = range / LAMP_MAP_TEXEL_M as f64;
        let (x0, x1) = ((fx - rt).floor().max(0.0) as usize, ((fx + rt).ceil() as usize).min(n - 1));
        let (z0, z1) = ((fz - rt).floor().max(0.0) as usize, ((fz + rt).ceil() as usize).min(n - 1));
        if fx + rt < 0.0 || fz + rt < 0.0 || x0 > n - 1 || z0 > n - 1 {
            continue;
        }
        for z in z0..=z1 {
            for x in x0..=x1 {
                let (dx, dz) = ((x as f64 + 0.5 - fx) * LAMP_MAP_TEXEL_M as f64, (z as f64 + 0.5 - fz) * LAMP_MAP_TEXEL_M as f64);
                let r = (dx * dx + dz * dz).sqrt();
                if r >= range {
                    continue;
                }
                let d2 = h * h + r * r;
                let cos_t = h / d2.sqrt();
                let shape = (1.0 + 1.5 * (1.0 - cos_t * cos_t)) * eo_look::math::smoothstep(l.cos_outer, l.cos_inner, cos_t);
                let w = 1.0 - (r / range).powi(4);
                map[z * n + x] += (l.candela * shape / d2 * cos_t * w * w) as f32;
            }
        }
    }
    map
}
pub const FRAME_LIGHT_CAP: u32 = 1024;
/// Luminaires drawn a frame (near and far; sky_lights.wgsl LT_SPRITE_CAP).
pub const SPRITE_CAP: u32 = 32_768;
pub const CLUSTERS: [u32; 3] = [16, 9, 24];
pub const CLUSTER_CAP: u32 = 32;
/// (`--look cinematic`, stage L3) The streak lights a frame (sky_streaks.wgsl
/// `LT_STREAK_CAP`, sky_light.wgsl `SKY_STREAK_CAP`): their list's words
/// (count + indices) follow the clusters' only with SF_CINEMATIC compiled in.
pub const STREAK_CAP: u32 = 32;
/// (`--weather damp`, stage L3) sky_common.wgsl `SKY_DAMP`.
pub const SKY_DAMP_BIT: u32 = 1 << 18;
/// (`--wet-surfaces`) the wet surfaces' runtime bit (sky_light.wgsl `SKY_WET_SURFACES`).
pub const SKY_WET_SURFACES_BIT: u32 = 1 << 23;
/// (`--screen-reflect`) the reflection's runtime bit (sky_light.wgsl `SKY_SCREEN_REFLECT`).
pub const SKY_SCREEN_REFLECT_BIT: u32 = 1 << 24;
/// (`--wet-relief`) the relief's water's runtime bit (sky_light.wgsl `SKY_WET_RELIEF`).
pub const SKY_WET_RELIEF_BIT: u32 = 1 << 22;
/// (`--night-photometry`) sky_lights.wgsl `LT_NIGHT_PHOTOMETRY_BIT`: the gather
/// marks each street luminaire's intensity toward the eye on its sprite.
pub const SKY_NIGHT_PHOTOMETRY_BIT: u32 = 1 << 25; // (1 << 24 is `--screen-reflect`'s, the reflect track)

/// (`--night-photometry`) the light gather's constants (sky_lights.wgsl): the
/// feature compiled in or out (`look::ShaderFeatures::night_photometry`) and its sky flag.
pub fn night_photometry_wgsl() -> String {
    format!(
        "\n// generated by sky.rs night_photometry_wgsl\nconst LT_NIGHT_PHOTOMETRY: bool = {};\nconst LT_NIGHT_PHOTOMETRY_BIT: u32 = {SKY_NIGHT_PHOTOMETRY_BIT}u;\n",
        crate::look::shader_features().night_photometry
    )
}
/// (`--lamp-bounce`) sky_lights.wgsl `LT_LAMP_BOUNCE_BIT`: the gather adds
/// each street lamp's pool as an upward Lambertian light (eo-sky `lamp_bounce`).
pub const SKY_LAMP_BOUNCE_BIT: u32 = 1 << 26;
/// (`--lamp-bounce`) the bounce light's kind in the frame list (after the
/// shop windows' `shop_lights::SHOP_LIGHT_KIND`; sky_light.wgsl shades kinds
/// from 3 as a spot of their cone, and from 4 widens the lobe by `size`).
pub const BOUNCE_LIGHT_KIND: u32 = 5;
/// (`--signal-spill`) a signal lens's light's kind in the frame list (after the
/// bounce's): its surfaces' and haze's shape the signal's lobe (eo-sky
/// `lights::SIGNAL_LOBE`), ranked with the shop windows (after the lamps).
pub const SIGNAL_LIGHT_KIND: u32 = 6;
/// (`--rear-lamps`) a rear lamp's light's kind (eo-sky `lights::REAR_LOBE`), ranked as the signals'.
pub const REAR_LIGHT_KIND: u32 = 7;
/// (`--signal-spill`, `--rear-lamps`) the two kinds for the modules that rank or shade them.
pub fn night_kinds_wgsl() -> String {
    format!("\n// generated by sky.rs night_kinds_wgsl\nconst NT_SIGNAL_KIND: u32 = {SIGNAL_LIGHT_KIND}u;\nconst NT_REAR_KIND: u32 = {REAR_LIGHT_KIND}u;\nconst NT_SIGNAL_BIT: u32 = {SKY_SIGNAL_SPILL_BIT}u;\n")
}
/// (`--urban-sky`) look_sky.wgsl's stars by the city sky's limiting magnitude.
pub const SKY_URBAN_SKY_BIT: u32 = 1 << 27;
/// (`--lamp-bounce`) the pools' bounce is drawn for lamps within these
/// distances of the eye (m), faded out between them: past them the rig's
/// unlit fill stands for the street's inter-reflection, and no far cluster
/// (which holds many lamps) drops a pool its neighbour keeps.
pub const BOUNCE_NEAR_M: f32 = 40.0;
pub const BOUNCE_FAR_M: f32 = 80.0;

/// (`--lamp-bounce`) the light gather's and the light shadows' constants:
/// the feature compiled in or out (`look::ShaderFeatures::lamp_bounce`), its
/// sky flag, the bounce's kind and eo-sky's model (`lamp_bounce::wgsl`).
pub fn lamp_bounce_wgsl() -> String {
    format!(
        "\n// generated by sky.rs lamp_bounce_wgsl\nconst LT_LAMP_BOUNCE: bool = {};\nconst LT_LAMP_BOUNCE_BIT: u32 = {SKY_LAMP_BOUNCE_BIT}u;\nconst LT_BOUNCE_KIND: u32 = {BOUNCE_LIGHT_KIND}u;\nconst LT_BOUNCE_NEAR_M: f32 = {BOUNCE_NEAR_M:?};\nconst LT_BOUNCE_FAR_M: f32 = {BOUNCE_FAR_M:?};\n{}",
        crate::look::shader_features().lamp_bounce,
        eo_sky::lamp_bounce::wgsl()
    )
}
/// (`--urban-sky`) the star field's constants (look_sky.wgsl `sky_stars_urban`):
/// its sky flag and eo-sky's model (`urban_sky::wgsl`).
pub fn urban_sky_wgsl() -> String {
    format!("\n// generated by sky.rs urban_sky_wgsl\nconst SKY_URBAN_SKY: u32 = {SKY_URBAN_SKY_BIT}u;\n{}", eo_sky::urban_sky::wgsl())
}
/// (`--office-floors`) rooms.wgsl `rm_office_floor`: an office building's
/// glass shows open-plan storeys (floors and tenants, mullions, spandrels).
pub const SKY_OFFICE_FLOORS_BIT: u32 = 1 << 28;
/// (`--room-depth`) rooms.wgsl: a lit home by its lamp as a point source,
/// its coverings lit from behind, the cap on its level; a shop's lit surfaces
/// at the street's exposure.
pub const SKY_ROOM_DEPTH_BIT: u32 = 1 << 29;
/// (`--lit-rooms`) rooms.wgsl: a home's light on the lamps' photometric scale (1 << 21, the night track's).
pub const SKY_LIT_ROOMS_BIT: u32 = 1 << 21;
/// (`--office-floors`, `--room-depth`) the rooms' constants (rooms.rs `wgsl_constants`).
pub fn interiors_wgsl() -> String {
    format!("\n// generated by sky.rs interiors_wgsl\nconst SKY_OFFICE_FLOORS: u32 = {SKY_OFFICE_FLOORS_BIT}u;\nconst SKY_ROOM_DEPTH: u32 = {SKY_ROOM_DEPTH_BIT}u;\nconst SKY_LIT_ROOMS: u32 = {SKY_LIT_ROOMS_BIT}u;\n")
}
/// (`--lamp-units`) the units' runtime bit (eo-sky `lamp_units`; `LU_BIT`).
/// (1 << 19..22 and 30, 31 were free at round 2; 30 and 31 are the night track's.)
pub const SKY_LAMP_UNITS_BIT: u32 = 1 << 30;
/// (`--luminaires`) the luminaires' runtime bit (`LUM_BIT`).
pub const SKY_LUMINAIRES_BIT: u32 = 1 << 31;
/// (`--rear-lamps`) the vehicles' rear lamps' runtime bit (`RL_BIT`; 1 << 19, the night track's).
pub const SKY_REAR_LAMPS_BIT: u32 = 1 << 19;
/// (`--signal-spill`) the signals' lenses' runtime bit (`SG_BIT`; 1 << 20, the night track's).
pub const SKY_SIGNAL_SPILL_BIT: u32 = 1 << 20;
/// (`--signal-spill`) the frame's signal lens lights at most (the lamp list's tail).
pub const SIGNAL_LIGHT_CAP: u32 = 256;
/// (`--signal-spill`) the mark a signal lens's record carries in its spare word.
pub const SIGNAL_MARK: f32 = -3.0;
/// (`--luminaires`) a lantern's lamp seen through its clear panes: the fx pass's
/// core's radius over a lantern drawn lit (m; a corn-cob LED or an A19-sized lamp,
/// INFERRED), in place of a cobra head's lens (sky_fx.wgsl `FX_LENS_M`).
pub const LANTERN_LAMP_RADIUS_M: f32 = 0.06;
/// (`--lamp-units`) the units' constants and functions for a module built
/// with the feature compiled in (`on`) or out, and the luminaires' bit.
pub fn lamp_units_wgsl_with(on: bool) -> String {
    let v3 = |c: [f64; 3]| format!("vec3<f32>({:.6}, {:.6}, {:.6})", c[0], c[1], c[2]);
    format!(
        "\n// generated by sky.rs lamp_units_wgsl\nconst LU_ON: bool = {on};\nconst LU_BIT: u32 = {SKY_LAMP_UNITS_BIT}u;\nconst LUM_BIT: u32 = {SKY_LUMINAIRES_BIT}u;\n\
         // (`--luminaires`) the glass: the pool's nadir (eo-sky photometry), the full-cutoff tail (eo-sky lit_fog), the colours\n\
         const LUM_POOL_PEAK_LUX: f32 = {:.4};\nconst LUM_EYE_FULL_COS: f32 = {:.6};\nconst LUM_EYE_TAIL_POWER: f32 = {:.4};\n\
         const LUM_MODEL_COBRA: u32 = {}u;\nconst LUM_COLOR_LED: vec3<f32> = {};\nconst LUM_COLOR_WARM: vec3<f32> = {};\nconst LUM_GLASS_CUT_M: f32 = {:.1};\nconst LUM_LANTERN_LAMP_M: f32 = {LANTERN_LAMP_RADIUS_M:?};\n{}{}{}const NT_SIGNAL_CD: vec3<f32> = {};\n",
        eo_sky::photometry::LAMP_POOL_PEAK_LUX,
        eo_sky::lit_fog::LUMINAIRE_FULL_DEG.to_radians().cos(),
        eo_sky::lit_fog::LUMINAIRE_TAIL_POWER,
        eo_props::Model::LampCobra as u32,
        v3(eo_sky::lights::led_street_lamp_color()),
        v3(eo_sky::cine_lamps::LampTint::Historic.colour()),
        crate::props_lib2::NEAR_RING_M,
        eo_sky::lamp_units::wgsl(),
        // (`--signal-spill`, `--rear-lamps`) the lights' kinds, lobes and the lenses' intensities
        night_kinds_wgsl(),
        eo_sky::lights::lobe_wgsl(),
        v3(eo_sky::lights::SIGNAL_CD)
    ) + &format!("const NT_SIGNAL_RGB = array<vec3<f32>, 3>({}, {}, {});\n", v3(eo_sky::lights::signal_color(0)), v3(eo_sky::lights::signal_color(1)), v3(eo_sky::lights::signal_color(2)))
}
/// (`--lamp-units`) `lamp_units_wgsl_with` for the light gather (the
/// features of this process), and the luminaires' compile flag there.
pub fn lamp_units_wgsl() -> String {
    let f = crate::look::shader_features();
    let red = eo_sky::lights::rear_lamp_color();
    format!(
        "{}const LUM_ON: bool = {};\nconst SG_ON: bool = {};\nconst SG_BIT: u32 = {SKY_SIGNAL_SPILL_BIT}u;\nconst SG_MARK: f32 = {SIGNAL_MARK:?};\n// (`--rear-lamps`) eo-sky lights `REAR_*`\nconst RL_ON: bool = {};\nconst RL_BIT: u32 = {SKY_REAR_LAMPS_BIT}u;\nconst RL_TAIL_CD: f32 = {:.3};\nconst RL_STOP_CD: f32 = {:.3};\nconst RL_REACH_LUX: f32 = {:.4};\nconst RL_INNER_DEG: f32 = {:.2};\nconst RL_OUTER_DEG: f32 = {:.2};\nconst RL_LENS_AREA_M2: f32 = {:.4};\nconst RL_LOW_BEAM_PEAK_CD: f32 = {:.1};\nconst RL_RED: vec3<f32> = vec3<f32>({:.5}, {:.5}, {:.5});\n",
        lamp_units_wgsl_with(f.lamp_units),
        f.luminaires,
        f.signal_spill,
        f.rear_lamps,
        eo_sky::lights::REAR_TAIL_CD,
        eo_sky::lights::REAR_STOP_CD,
        eo_sky::lights::REAR_REACH_LUX,
        eo_sky::lights::REAR_INNER_DEG,
        eo_sky::lights::REAR_OUTER_DEG,
        eo_sky::lights::REAR_LENS_AREA_M2,
        eo_sky::photometry::LOW_BEAM_PEAK_CD,
        red[0],
        red[1],
        red[2]
    )
}
/// Lights are shaded within this view depth (m).
pub const CLUSTER_FAR_M: f32 = 300.0;
pub const CLUSTER_NEAR_M: f32 = 0.5;
pub const BLOOM_LEVELS: usize = 5;
/// (`--cine-camera`) the bloom's levels under the camera model: two more, down
/// to 1/128 of the frame, for the lens's long tail.
pub const CAM_BLOOM_LEVELS: usize = 7;

/// Whether the camera model draws with these options (its code compiled in, the look on).
pub(crate) fn camera_on(o: &SkyOptions) -> bool {
    o.cine_camera && o.cinematic && crate::look::shader_features().camera
}

/// The bloom's level count: the camera model's when its code is compiled in
/// (chosen at start, as the pipelines are), else `BLOOM_LEVELS`.
fn bloom_level_count() -> usize {
    if crate::look::shader_features().camera { CAM_BLOOM_LEVELS } else { BLOOM_LEVELS }
}
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
    /// Enable fair-weather clouds when the camera is near Aquatic Park.
    pub cove_cumulus: bool,
    /// A low-deck cloud seen over the ground takes the aerial perspective of its own depth
    /// and hides the haze behind it. Off by default until reviewed under the visual gate
    /// (`--cloud-depth-haze` turns it on); off, the ground's whole aerial perspective is in
    /// front of the cloud, a blue veil under the horizon, as before.
    pub cloud_depth_haze: bool,
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
    /// (`--wet-surfaces`, the reflect track) every exposed surface takes the
    /// rain: a porosity-weighted darkening and film on walks, plazas, roofs and
    /// walls, puddles standing in the ground's hollows and the gutters with
    /// damp halos, a water surface's whole environment on the film, and the
    /// screen-space reflection traced and filtered for it (sky_light.wgsl
    /// `sky_w2_*`). Off by default (its code compiled out: `SF_WET_SURFACES`);
    /// off, the frame is as before.
    pub wet_surfaces: bool,
    /// (`--screen-reflect`, the reflect track) the screen-space reflection's
    /// second generation: a jittered trace the history accumulates, filtered
    /// by the surface's lobe at the hit (sharp at contact, long for far
    /// lights), the frame's own sky where a ray leaves the scene, and glass
    /// traced as well (sky_light.wgsl `sky_sr_*`). Off by default (its code
    /// compiled out: `SF_SCREEN_REFLECT`); off, the frame is as before.
    pub screen_reflect: bool,
    /// (`--probe-sky`, the reflect track) the probe grid's sky is the one the
    /// frame draws (the atmosphere tables over the drawn dome, the city's
    /// glow, greyed by the cloud cover) instead of the analytic dome alone, and
    /// its levels are GGX-prefiltered instead of box-filtered (probe_sky.wgsl,
    /// probe_grid.rs `SkyFill`). Off by default (made only with
    /// `SF_PROBE_SKY` compiled in); off, the probes are as before.
    pub probe_sky: bool,
    /// (`--wet-relief`, the reflect track; over `--wet-surfaces`) the water
    /// stands where the street's relief holds it (the street pass's ruts,
    /// birdbaths and flow line, not the switch's noise hollows on the road),
    /// the film as smooth as the surface under it (the streaks' lobes with
    /// it), and Lekner & Dorf's darkening (sky_light.wgsl `sky_wr_*`). Off by
    /// default (its code compiled out: `SF_WET_RELIEF`); off, the frame is as before.
    pub wet_relief: bool,
    /// (`--lamp-streaks`, the reflect track; under the cinematic look) the
    /// cars' rear lamps streak on the wet street as lights of their own
    /// (sky_lamp_streaks.wgsl picks the cars brightest at the eye, sky_light.wgsl
    /// `sky_cine_streaks` reflects them). Off by default (its code and pass made
    /// only with `SF_LAMP_STREAKS`); off, the gather writes no rear lamp.
    pub lamp_streaks: bool,
    /// (`--no-volumetric-clouds` to disable) deep multi-layer volumetric cloudscape
    /// (64^3 curl-advected detail, 512^2 meteorological weather map, altocumulus/cirrus
    /// and hemispherical water reflection dome). On by default.
    pub volumetric_clouds: bool,
    /// (`--weather-fx`) physical rain/wind weather details (puddle pooling, capillary ripples,
    /// raindrop impact splash crowns/sparkles, wind-coupled tree/leaf motion). Off by default
    /// (`--no-weather-fx` restores).
    pub weather_fx: bool,
    pub weather: WeatherMode,
    pub seed: u32,
    /// The world clock at the first frame (seconds since the world's epoch).
    pub world_time_s: f64,
    /// 0 low, 1 default, 2 high (cloud steps 12 / 18 / 28).
    pub quality: u32,
    pub moon_day: f64,
    /// (`--shop-light 1|2`, stage N1; shop_lights.rs) the shop windows' lights:
    /// 0 none (the frame as before), 1 within 160 m of the eye, 2 the clusters'
    /// whole 300 m. Lit only once their placement is handed over (`set_shop_lights`).
    pub shop_light: u8,
    /// (`--look cinematic`, the cinematic look plan) the sky, weather and night
    /// lights draw the cinematic look; false (the default) draws today's frame.
    /// Set with `LookOptions::look` (eo-app quality.rs); its shader code sits
    /// behind `SF_CINEMATIC`.
    pub cinematic: bool,
    /// (`--cine-camera`, camera track; cine_post.rs, cine_camera.wgsl) under the
    /// cinematic look, the camera model: the frame metered from its own
    /// histogram with local exposure, a hue-preserving filmic curve, a
    /// threshold-free long-tailed bloom (`CAM_BLOOM_LEVELS` levels) with no lamp
    /// starbursts, the sun's glare and ghosts, film grain and the optical fall-off.
    /// False (the default) is the frame as before; its shader code sits behind
    /// `SF_CAMERA`.
    pub cine_camera: bool,
    /// (camera track, round 2) the camera model's later features (cine_post.rs
    /// `CAM_BIT_*`: `--camera-tone`, `--white-balance`, `--lens-halation`,
    /// `--camera-sharpen`, `--lens-flare`, `--camera-resolve`, `--camera-print`), each
    /// compiled in only with its switch and `cine_camera`; 0 (the default) is the
    /// model as before.
    pub cam_bits: u32,
    /// (`--night-photometry`; eo-sky `night_photometry`) the night on one
    /// photometric scale: the practicals at their lux against the night's
    /// natural light and the exposure metered on the lit street. Off (the
    /// default) draws today's frame; its shader code sits behind `SF_NIGHT_PHOTOMETRY`.
    pub night_photometry: bool,
    /// (`--lamp-bounce`; eo-sky `lamp_bounce`) each street lamp's pool lights
    /// the walls, kerbs, cars and undersides round it (one bounce). Off (the
    /// default) draws today's frame; its shader code sits behind `SF_LAMP_BOUNCE`.
    pub lamp_bounce: bool,
    /// (`--urban-sky`) the sky over a lit city: the city's skyglow on the
    /// practicals' scale (it is their light, scattered) and only the stars a
    /// city sky's brightness leaves visible (its limiting magnitude). Off (the
    /// default) draws today's frame; its shader code sits behind `SF_URBAN_SKY`.
    pub urban_sky: bool,
    /// (`--office-floors`) an office building's glass shows open-plan storeys:
    /// lit and dark floors and tenants' zones, their fixtures, workstation
    /// rows and core, blinds, a curtain wall's mullions and spandrels. Off (the
    /// default) draws today's frame; its shader code sits behind `SF_OFFICE_FLOORS`.
    pub office_floors: bool,
    /// (`--room-depth`) a lit home by its lamp as a point source (a hot spot,
    /// dark corners, a low lamp's shade), its coverings lit from behind, the
    /// pane cap on the room's level; a lit shop's surfaces at the street's
    /// exposure. Off (the default) draws today's frame; its shader code sits
    /// behind `SF_ROOM_DEPTH`.
    pub room_depth: bool,
    /// (`--lamp-units`) the street lamps as the city's units (eo-sky
    /// `lamp_units`): each on by its own control as the sky darkens past its
    /// level (none in sun, all by the blue hour), at its own output and colour
    /// bin, a few failed. Off (the default) draws today's frame; its shader
    /// code sits behind `SF_LAMP_UNITS`.
    pub lamp_units: bool,
    /// (`--luminaires`) the luminaires' own optics: a lamp's light from inside
    /// its fixture's glass (each lantern of a twin post), the glass lit by it
    /// (a lantern's frosted panes, a cobra head's flat lens dimming off its
    /// axis), and no sprite orb where the fixture is drawn. Off (the default)
    /// draws today's frame; its shader code sits behind `SF_LUMINAIRES`.
    pub luminaires: bool,
    /// (`--rear-lamps`) a lit vehicle's tail lamps as lights of the frame (eo-sky
    /// `lights::REAR_*`), its stop lamps while it is held at a junction: red on
    /// its bumper, the road and the car behind, and in the wet street's streaks.
    /// Off (the default) draws today's frame; its shader code sits behind `SF_REAR_LAMPS`.
    pub rear_lamps: bool,
    /// (`--signal-spill`) each traffic signal's lit lens near the eye as a light
    /// of the frame (eo-sky `lights::SIGNAL_*`): its red, yellow or green on the
    /// mast, the road and the wet street's streaks. Off (the default) draws
    /// today's frame; its shader code sits behind `SF_SIGNAL_SPILL`.
    pub signal_spill: bool,
    /// (`--lit-rooms`, with `--room-depth`) a lit home's light on the street lamps'
    /// photometric scale (rooms.wgsl `RD_HOME_LUX`: 60 lux 1.5 m from its lamp), its
    /// cap a soft shoulder, so a lit window stands ~2.5-3.5 EV over the metered walk.
    /// Off (the default) draws today's frame; its shader code sits behind `SF_LIT_ROOMS`.
    pub lit_rooms: bool,
    /// (`--wire-light`) the street overhead's wires lit as the scene's surfaces are
    /// (a matte dark cable under the sky, the rig's fill and the key, by the
    /// cylinder's own shading) in place of the JS's unlit colour, which the
    /// night's exposure lifted into glowing strokes over the sky. Off (the
    /// default) draws today's frame; its shader code sits behind `SF_WIRE_LIGHT`.
    pub wire_light: bool,
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
            cove_cumulus: false,
            cumulus: std::env::var("EO_CUMULUS").is_ok_and(|v| v == "1"),
            cloud_depth_haze: false,
            phys_sky: true,
            ap_lut: true,
            filmic: true,
            ev100: true,
            sky_vis: true,
            night_shaping: true,
            wet_surfaces: false,
            screen_reflect: false,
            probe_sky: false,
            wet_relief: false,
            lamp_streaks: false,
            volumetric_clouds: true,
            weather_fx: true,
            weather: WeatherMode::Preset(eo_sky::Kind::Clear),
            seed: eo_sky::world_seed(),
            world_time_s: 0.0,
            quality: 1,
            moon_day: eo_sky::moon::DEFAULT_MOON_DAY,
            shop_light: 0,
            cinematic: false,
            cine_camera: false,
            cam_bits: 0,
            night_photometry: false,
            lamp_bounce: false,
            urban_sky: false,
            office_floors: false,
            room_depth: false,
            lamp_units: false,
            luminaires: false,
            rear_lamps: false,
            signal_spill: false,
            lit_rooms: false,
            wire_light: false,
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
            cove_cumulus: false,
            cumulus: false,
            cloud_depth_haze: false,
            phys_sky: false,
            ap_lut: false,
            filmic: false,
            ev100: false,
            sky_vis: false,
            night_shaping: false,
            wet_surfaces: false,
            screen_reflect: false,
            probe_sky: false,
            wet_relief: false,
            lamp_streaks: false,
            volumetric_clouds: false,
            weather_fx: false,
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
    fn cumulus_weight(&self, eye: glam::Vec3) -> f32 {
        if self.cumulus { return 1.0; }
        if !self.cove_cumulus { return 0.0; }
        let scale = eo_data::groundscale::scale() as f32;
        let distance = (eye.x / scale + 400.0).hypot(eye.z / scale + 3340.0);
        let t = ((1250.0 - distance) / 550.0).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
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
            (self.cumulus || self.cove_cumulus, 16384), // SKY_CUMULUS
            (self.cloud_depth_haze, 2048), // SKY_CLOUD_DEPTH_HAZE
            (self.cinematic, 1 << 17), // SKY_CINEMATIC (`--look cinematic`, stage L4: the golden haze and shafts)
            (self.wet_surfaces, SKY_WET_SURFACES_BIT), // SKY_WET_SURFACES (`--wet-surfaces`, sky_light.wgsl)
            (self.screen_reflect, SKY_SCREEN_REFLECT_BIT), // SKY_SCREEN_REFLECT (`--screen-reflect`, sky_light.wgsl)
            (self.wet_relief, SKY_WET_RELIEF_BIT), // SKY_WET_RELIEF (`--wet-relief`, sky_light.wgsl)
            (self.night_photometry, SKY_NIGHT_PHOTOMETRY_BIT), // (`--night-photometry`) sky_lights.wgsl LT_NIGHT_PHOTOMETRY_BIT
            (self.lamp_bounce, SKY_LAMP_BOUNCE_BIT), // (`--lamp-bounce`) sky_lights.wgsl LT_LAMP_BOUNCE_BIT
            (self.urban_sky, SKY_URBAN_SKY_BIT), // (`--urban-sky`) look_sky.wgsl's stars
            (self.office_floors, SKY_OFFICE_FLOORS_BIT), // (`--office-floors`) rooms.wgsl SKY_OFFICE_FLOORS
            (self.room_depth, SKY_ROOM_DEPTH_BIT), // (`--room-depth`) rooms.wgsl SKY_ROOM_DEPTH
            (self.lamp_units, SKY_LAMP_UNITS_BIT), // (`--lamp-units`) LU_BIT
            (self.luminaires, SKY_LUMINAIRES_BIT), // (`--luminaires`) LUM_BIT
            (self.rear_lamps, SKY_REAR_LAMPS_BIT), // (`--rear-lamps`) RL_BIT
            (self.signal_spill, SKY_SIGNAL_SPILL_BIT), // (`--signal-spill`) SG_BIT
            (self.lit_rooms, SKY_LIT_ROOMS_BIT), // (`--lit-rooms`) rooms.wgsl SKY_LIT_ROOMS
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
    // the twilight light on the clouds from the sky toward the set sun (rgb); w a
    // tunnel bore's interior roadway irradiance (stage A8, `--tunnels`; 0 without bores)
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
    // the second high deck (eo_sky::clouds::Decks::cirrus): x its height (m), y its opacity, z, w unused; read by nothing yet
    cirrus: [f32; 4],
    // spare (the sky tracks' next field); zero
    spare: [f32; 4],
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
    /// (lighting, `--light-shadows`) SF's 3000 K LED lamps: rgb the colour a
    /// street lamp of the JS role takes (unit luminance), w 1 on (0: as before);
    /// `led_key` that role's chromaticity key (largest channel 1), w the tolerance.
    led: [f32; 4],
    led_key: [f32; 4],
    /// (`--shop-light`) the shop windows' lights: how many, units per candela
    /// (the lamps' scale times the rooms' dusk; 0 without them), the drawn
    /// hour, the eye distance they are gathered within (sky_lights.wgsl `lights_shops`).
    shops: [f32; 4],
}

/// The mask words: the props tiles resident (STREET_TILE_M keys -128..127 on
/// each axis), then the waterside cells (up to `WATER_CELLS_MAX` squared).
const TILE_WORDS: usize = 2048;
pub const WATER_CELLS_MAX: usize = 512;
const MASK_WORDS: usize = TILE_WORDS + WATER_CELLS_MAX * WATER_CELLS_MAX / 32;
/// (`--look cinematic`, stage L2) the street lamp palette's grid after the
/// waterside cells (eo-sky `cine_lamps::PaletteGrid`, a byte a cell, four a
/// word), up to `PALETTE_CELLS_MAX` on a side; allocated only with
/// SF_CINEMATIC compiled in.
pub const PALETTE_CELLS_MAX: usize = 1024;
const PALETTE_WORDS: usize = PALETTE_CELLS_MAX * PALETTE_CELLS_MAX / 4;

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
    /// (lighting) the lit fog binding's generation the scene binds were made with.
    lit_fog_gen: u64,
    bloom_bgs: [Vec<wgpu::BindGroup>; 2],
    bytes: u64,
    /// (`--screen-reflect`) the histories' mip chain (sky_reflect.rs).
    pyr: Option<crate::sky_reflect::ReflectTargets>,
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
    /// the scene meter (`--no-scene-meter`): after a cut or a turn, the frame
    /// from which a reading counts as the new view's (its first snaps the adaptation).
    meter_snap_after: Option<u32>,
    interior_meter: Option<crate::interior_meter::InteriorMeter>,
    exterior_camera: Option<crate::interior_meter::ExteriorCamera>,
    frame_bloom: bool,
    /// the uniform's bloom terms this frame (`bloom_terms`).
    bloom_terms: [f32; 4],
    /// the frame of the last cut or turn the scene meter snapped for: no reading
    /// older (of the view before it) sets the exposure after it.
    meter_floor: u32,
    /// the aerosol density the transmittance and multiple-scattering
    /// tables were built for (they depend on the medium only; rebuilt on a change).
    atmo_built: Option<[[f32; 4]; 4]>,
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
    /// (`--look cinematic`, stage L2) the lamp palette's grid: its cell (m; 0
    /// none), columns, rows (`set_lamp_palette`; its corner is the waterside grid's).
    palette: (f32, u32, u32),
    /// `--smooth-time` (`LightingOptions::smooth_time`): no exposure snap but a
    /// camera cut, histories kept through small weather steps (see `update`).
    smooth: bool,
    /// Stage A8 (`--tunnels`, `set_bore`): how far into a tunnel's bore the
    /// eye is (None outside), and whether bores with lamps are drawn (their
    /// lamps are lit by day too). None and false without tunnels.
    bore_depth: Option<f64>,
    tunnel_lamps: bool,
    /// Whether any local light is lit this frame (street lamps, vehicle lamps or
    /// a bore's lamps, with the lights on): the lit fog has nothing to scatter otherwise.
    lights_live: bool,
    /// (lighting, `--light-shadows`) the street lamps in SF's 3000 K LED colour
    /// (`eo_sky::lights::led_street_lamp_color`): the gather's lamps, their
    /// luminaires and the city's glow. Off: the JS role's 0xffc46a, as before.
    led: bool,
    /// (`--photo-finish`, stage W4: `LookOptions::photo_finish`) San Francisco's
    /// measured air in the tables and the aerial perspective (eo-sky `photo`),
    /// and a lens's bloom (eo-look `tone::PHOTO_BLOOM_*`). 0: as before.
    photo: u8,
    /// (`--shop-light`) the shop windows' lights (shop_lights.rs `GpuShopLight`)
    /// and how many: bound with the lights at binding 12 (None: a stand-in).
    shops: Option<(wgpu::Buffer, u32)>,
}

pub struct SkyGpu {
    device: wgpu::Device,
    pub(crate) uniform: wgpu::Buffer,
    sheet: wgpu::Texture,
    pub(crate) sheet_view: wgpu::TextureView,
    sheet_ready: AtomicBool,
    _noise: wgpu::Texture,
    pub(crate) noise_view: wgpu::TextureView,
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
    /// (`--far-street-light`) the lamps' pools on the ground, lux (`bake_lamp_map`; 1 x 1 while off).
    lamp_map: wgpu::Texture,
    pub(crate) lamp_map_view: wgpu::TextureView,
    lamp_count: AtomicU32,
    /// (`--signal-spill`) the signal lens lights after the static lamps (`set_signal_lights`).
    signal_count: AtomicU32,
    /// Of them, by the water.
    waterside: AtomicU32,
    pub(crate) frame_lights: wgpu::Buffer,
    sprites: wgpu::Buffer,
    pub(crate) counter: wgpu::Buffer,
    pub(crate) clusters: wgpu::Buffer,
    pub(crate) lt_params: wgpu::Buffer,
    /// sky_lights.wgsl `lt_mask` (`MASK_WORDS`).
    masks: wgpu::Buffer,
    dummy_storage: wgpu::Buffer,
    lights_bgl: wgpu::BindGroupLayout,
    lights_pipes: [wgpu::ComputePipeline; 5],
    /// (`--look cinematic`, stage L3) the streak lights' pick (sky_streaks.wgsl), made only with SF_CINEMATIC.
    streaks_pipe: Option<wgpu::ComputePipeline>,
    /// (`--lamp-streaks`) the cars' rear lamps' pick (sky_lamp_streaks.wgsl), made only with SF_LAMP_STREAKS and SF_CINEMATIC.
    lamp_streaks_pipe: Option<wgpu::ComputePipeline>,
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
    /// (`--look cinematic`, stage L1) the lamp glare (sky_fx.wgsl `glare_vs`), with SF_CINEMATIC compiled in.
    glare_pipe: Option<wgpu::RenderPipeline>,
    rain_pipe: wgpu::RenderPipeline,
    /// A 1 x 1 stand-in for the previous frame before there is one.
    pub(crate) dummy_hist: wgpu::TextureView,
    /// (lighting, `--lit-fog`) the resolve's lit fog bindings 10 and 11 while
    /// the lit fog has no volume: a 1 x 1 x 1 3D texture and a zero buffer
    /// (the shader reads neither without the feature's bit).
    lit_fog_stand_in: (wgpu::TextureView, wgpu::Buffer),
    /// The lit fog's integrated volume and its state word (`set_lit_fog_volume`),
    /// with a generation that rebuilds the resolve's bind groups when it changes.
    lit_fog_bind: Mutex<(u64, Option<(wgpu::TextureView, wgpu::Buffer)>)>,
    /// (`--look cinematic`, stage L4) the far sun shadow map (`--far-shadows`) at the volume's
    /// binding 22, for the shade in the golden haze past the cascades (`set_far_shadow_map`),
    /// with a generation that rebuilds the volume's bind groups; a 1 x 1 stand-in without it
    /// (the shader reads it only with SF_CINEMATIC, SKY_CINEMATIC and the far rings drawn).
    far_shadow_bind: Mutex<(u64, Option<wgpu::TextureView>)>,
    far_shadow_stand_in: wgpu::TextureView,
    targets: Mutex<Option<Targets>>,
    state: Mutex<State>,
    pub bytes: u64,
    /// (`--screen-reflect`) the histories' mip chain's pipelines (sky_reflect.rs), with SF_SCREEN_REFLECT compiled in.
    reflect: Option<crate::sky_reflect::ReflectPyramid>,
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

/// (lighting, `--light-shadows`) The gather's LED lamps (`LtParams.led`,
/// `led_key`): SF's 3000 K LED colour at unit luminance with w 1, and the JS
/// street lamp role's chromaticity key with the tolerance a channel may differ
/// by (sky_lights.wgsl `lt_led`).
pub fn led_params() -> ([f32; 4], [f32; 4]) {
    let c = eo_sky::lights::led_street_lamp_color();
    let k = eo_sky::lights::street_lamp_chroma();
    ([c[0] as f32, c[1] as f32, c[2] as f32, 1.0], [k[0] as f32, k[1] as f32, k[2] as f32, LED_KEY_TOLERANCE])
}

/// (`--look cinematic`, stage L2) The lights gather's constants for the
/// look's street lamps (sky_lights.wgsl `lt_cine_*`; eo-sky `cine_lamps`):
/// `LT_CINE` whether the code is compiled in (SF_CINEMATIC; the gather's
/// shader does not take the look's common constants), where the palette's
/// words start in `lt_mask`, each tint's colour (unit luminance, an eye
/// adapted to 3000 K), the street lamp role's key, the reach in mounting
/// heights, the axis's tilt into a light's `dir`, the fallback's cut-off and
/// peak (kind 0's cone, read by the lit fog and the pool decals), and the
/// props' lamp models (historic, banner) and the landmarks' mark.
/// `--lamp-spread` (on unless `--no-lamp-spread`, read once): the default
/// look's street lamps throw the cobra head's batwing over the street, as the
/// cinematic look's do (sky_lights.wgsl `LT_LAMP_SPREAD`): a props lamp is the
/// fallback's type III head at its own head (cut off at `STREET_CUTOFF_DEG`, its
/// peak at `STREET_PEAK_DEG`) instead of the JS practical's spot (a disc 1.75
/// mounting heights across, the road between the poles dark), and every such
/// head reaches `RANGE_MOUNTING_HEIGHTS` (the default's 2.6 windowed out the
/// batwing's own peak). The nadir stays the RP-8 peak (`LAMP_POOL_PEAK_LUX`).
pub fn lamp_spread_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| !std::env::args().any(|a| a == "--no-lamp-spread"))
}

pub fn cine_lamps_wgsl(on: bool) -> String {
    use eo_sky::cine_lamps::{LampTint, RANGE_MOUNTING_HEIGHTS};
    let v3 = |c: [f64; 3]| format!("vec3<f32>({:.6}, {:.6}, {:.6})", c[0], c[1], c[2]);
    let tints: Vec<String> = LampTint::ALL.iter().map(|t| v3(t.colour())).collect();
    let k = eo_sky::lights::street_lamp_chroma();
    let mut src = format!(
        "\n// generated by sky.rs cine_lamps_wgsl (`--look cinematic`, stage L2)\nconst LT_CINE: bool = {on};\nconst LT_PALETTE_WORD: u32 = {MASK_WORDS}u;\nconst LT_CINE_TINTS = array<vec3<f32>, 4>({});\nconst LT_CINE_KEY: vec3<f32> = {};\nconst LT_CINE_KEY_TOL: f32 = {LED_KEY_TOLERANCE:?};\nconst LT_CINE_RANGE_MH: f32 = {RANGE_MOUNTING_HEIGHTS:?};\nconst LT_CINE_AXIS_TILT: f32 = {CINE_AXIS_TILT:?};\nconst LT_CINE_COS_CUT: f32 = {:.6};\nconst LT_CINE_COS_PEAK: f32 = {:.6};\nconst LT_MODEL_HISTORIC: u32 = {}u;\nconst LT_MODEL_BANNER: u32 = {}u;\nconst LT_LANDMARK: u32 = {}u;\nconst LT_LAMP_SPREAD: bool = {};\n",
        tints.join(", "),
        v3(k),
        eo_sky::lights::STREET_CUTOFF_DEG.to_radians().cos(),
        eo_sky::lights::STREET_PEAK_DEG.to_radians().cos(),
        eo_props::Model::LampHistoric as u32,
        eo_props::Model::LampBanner as u32,
        crate::landmarks::LIGHT_KIND_LANDMARK,
        lamp_spread_enabled(),
    );
    src += &crate::agents::car_lamps_wgsl(); // (`--car-lamps`) the cars' rear lamps, compiled out without it
    if on {
        // (stage L3) the streak lights' pick (sky.rs `streaks_pipe`)
        src += &format!("const LT_STREAK_CAP: u32 = {STREAK_CAP}u;\n");
        src += include_str!("sky_streaks.wgsl");
        if crate::look::shader_features().lamp_streaks {
            src += &crate::sky_reflect::lamp_streaks_gather_wgsl(); // (`--lamp-streaks`, the reflect track)
        }
    }
    src
}

/// (`--look cinematic`, stage L2) How far a street luminaire's `dir` leans
/// from straight down toward its street's axis (the horizontal part's
/// length): the axis its pool is long along (sky_light.wgsl
/// `sky_cine_cobra`), 2.9 deg, which nothing reading `dir` as the beam
/// (the lit fog, the lamp maps' straight-down spot) sees.
pub const CINE_AXIS_TILT: f32 = 0.05;

/// How far a light's chromaticity key may lie from the street lamp role's
/// (per channel, its largest channel 1) for the gather to take it for a
/// street lamp: the role's lamps carry 0xffc46a exactly (the tunnel lamps'
/// 4000 K key differs by 0.1 and more).
pub const LED_KEY_TOLERANCE: f32 = 0.02;

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
        let map_n = if far_street_light_enabled() { LAMP_MAP_SIZE } else { 1 };
        let lamp_map = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("sky-lamp-map"),
            size: wgpu::Extent3d { width: map_n, height: map_n, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let lamp_map_view = lamp_map.create_view(&wgpu::TextureViewDescriptor::default());
        // (COPY_SRC: the light shadows' tests read the frame's lights, light_shadow.rs)
        // (`--lamp-streaks`, the reflect track) the cars' rear lamps' records past the cap, only with it compiled in.
        let lamp_streaks = crate::look::shader_features().cinematic && crate::look::shader_features().lamp_streaks;
        let rear = if lamp_streaks { crate::sky_reflect::lamp_streak_lamps() } else { 0 };
        let frame_lights = buffer("sky-frame-lights", (FRAME_LIGHT_CAP as u64 + rear) * 64, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC);
        let sprites = buffer("sky-sprites", SPRITE_CAP as u64 * 64, wgpu::BufferUsages::STORAGE);
        // (COPY_SRC: the light shadows' tests read the frame's light count, light_shadow.rs)
        let counter = buffer("sky-light-counter", 32, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC);
        let ncl = (CLUSTERS[0] * CLUSTERS[1] * CLUSTERS[2]) as u64;
        // (COPY_SRC: the shop lights' tests and cards read the clusters, `lights_readback`)
        // (`--look cinematic`, stage L3) the streak lights' count and indices after the clusters, only with the look compiled in.
        let streak_words = if crate::look::shader_features().cinematic { 1 + STREAK_CAP as u64 } else { 0 };
        // (`--lamp-streaks`) the cars' rear lamps' indices after them, only with both compiled in.
        let streak_words = streak_words + rear;
        let clusters = buffer("sky-clusters", (ncl * (1 + CLUSTER_CAP as u64) + streak_words) * 4, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC);
        let lt_params = buffer("sky-light-params", std::mem::size_of::<LtParams>() as u64, uni);
        let dummy_storage = buffer("sky-dummy-storage", 256, wgpu::BufferUsages::STORAGE);
        // (`--look cinematic`, stage L2) the lamp palette's words after the masks, only with the look compiled in.
        let mask_words = MASK_WORDS + if crate::look::shader_features().cinematic { PALETTE_WORDS } else { 0 };
        let masks = buffer("sky-light-masks", mask_words as u64 * 4, st);
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
            entries: &[entry(0, cs, ubuf), entry(1, cs, sbuf(true)), entry(2, cs, sbuf(false)), entry(3, cs, sbuf(false)), entry(4, cs, sbuf(false)), entry(5, cs, sbuf(true)), entry(6, cs, sbuf(true)), entry(7, cs, ubuf), entry(8, cs, sbuf(false)), entry(9, cs, sbuf(true)), entry(10, cs, sbuf(true)), entry(11, cs, sbuf(true)), entry(12, cs, sbuf(true))],
        });
        let sky_only = include_str!("sky_common.wgsl");
        let lm = module("sky_lights.wgsl", format!("{sky_only}\n{}\nconst LT_TILE_M: f32 = {:.1};\n{}{}{}", sea_wgsl(), eo_street::STREET_TILE_M, crate::shop_lights::wgsl(), cine_lamps_wgsl(crate::look::shader_features().cinematic) + &night_photometry_wgsl() + &lamp_bounce_wgsl() + &lamp_units_wgsl(), include_str!("sky_lights.wgsl")));
        let ll = layout("sky-lights-layout", &lights_bgl);
        let lights_pipes = [
            compute("lights_lamps", &ll, &lm, "lights_lamps"),
            compute("lights_agents", &ll, &lm, "lights_agents"),
            compute("lights_cluster", &ll, &lm, "lights_cluster"),
            compute("lights_props", &ll, &lm, "lights_props"),
            compute("lights_shops", &ll, &lm, "lights_shops"), // (`--shop-light`)
        ];
        let streaks_pipe = crate::look::shader_features().cinematic.then(|| compute("lights_streaks", &ll, &lm, "lights_streaks"));
        // (`--lamp-streaks`) the cars' rear lamps' pick, after the streaks' (sky_lamp_streaks.wgsl)
        let lamp_streaks_pipe = (crate::look::shader_features().cinematic && crate::look::shader_features().lamp_streaks).then(|| compute("lights_lamp_streaks", &ll, &lm, "lights_lamp_streaks"));

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
                // (`--look cinematic`, stage L4) the far sun shadow map (or its stand-in)
                entry(22, cs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false }),
                // (`--volumetric-clouds`) high-definition detail 3D noise and meteorological weather map
                entry(23, cs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false }),
                entry(24, cs, t2(true)),
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
        let clouds = crate::clouds::CloudGpu::new(d, crate::clouds::ShadowInputs { module: &vm, look_buf: init.look_buf, sky_uniform: &uniform, sheet: &sheet_view, repeat: &rep, trans: &atmo_views[0], sky_view: &atmo_views[2], linear: &lin });
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
                // (lighting, `--lit-fog`) the lit fog's integrated volume and its state word
                entry(10, fs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false }),
                entry(11, fs, sbuf(true)),
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
        // (`--cine-camera`) the camera model's prefilter compiled in or out (`SF_CAMERA`) and its constants.
        let bm = module("sky_bloom.wgsl", format!("{sky_only}\n{}\n{}", crate::cine_post::bloom_wgsl(crate::look::shader_features().camera), include_str!("sky_bloom.wgsl")));
        let bl = layout("sky-bloom-layout", &bloom_bgl);
        let bloom_pipes = [compute("bloom_prefilter", &bl, &bm, "bloom_prefilter"), compute("bloom_down", &bl, &bm, "bloom_down"), compute("bloom_up", &bl, &bm, "bloom_up")];
        let bloom_levels = (0..bloom_level_count()).map(|_| buffer("sky-bloom-level", 16, uni)).collect();

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
        // (`--look cinematic`, stage L1) the lamp glare, only with the look compiled in.
        let glare_pipe = crate::look::shader_features().cinematic.then(|| fx_pipe("sky-glare", "glare_vs", "glare_fs", wgpu::BlendState { color: add, alpha: add }));
        let dummy_hist = tex(d, "sky-hist-stand-in", (1, 1), init.hdr_format, wgpu::TextureUsages::TEXTURE_BINDING);
        let lit_fog_stand_in = (
            d.create_texture(&wgpu::TextureDescriptor {
                label: Some("sky-lit-fog-stand-in"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D3,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default()),
            buffer("sky-lit-fog-state-stand-in", 16, wgpu::BufferUsages::STORAGE),
        );
        let far_shadow_stand_in = d
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("sky-far-shadow-stand-in"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: crate::look::SHADOW_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
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
            lamp_map,
            lamp_map_view,
            lamp_count: AtomicU32::new(0),
            signal_count: AtomicU32::new(0),
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
            streaks_pipe,
            lamp_streaks_pipe,
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
            glare_pipe,
            rain_pipe,
            dummy_hist,
            lit_fog_stand_in,
            lit_fog_bind: Mutex::new((0, None)),
            far_shadow_bind: Mutex::new((0, None)),
            far_shadow_stand_in,
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
                meter_snap_after: None,
                interior_meter: None,
                exterior_camera: None,
                frame_bloom: false,
                bloom_terms: [0.0; 4],
                meter_floor: 0,
                atmo_built: None,
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
                palette: (0.0, 0, 0),
                smooth: false,
                bore_depth: None,
                tunnel_lamps: false,
                lights_live: false,
                led: false,
                photo: 0,
                shops: None,
            }),
            bytes,
            reflect: crate::look::shader_features().screen_reflect.then(|| crate::sky_reflect::ReflectPyramid::new(d)),
        }
    }

    /// (lighting, `--lit-fog`; called by `LitFogGpu` when its volume is made or
    /// dropped) the integrated volume the resolve composites and the state word
    /// it checks (`[0]`: the frame it was last integrated in), or None for the
    /// stand-ins. The resolve's bind groups are rebuilt at the next frame. Locks
    /// only its own slot (safe inside `encode_after`'s `mid` hook).
    /// (`--look cinematic`, stage L4) the far sun shadow map for the volume's binding 22
    /// (None: the stand-in); the volume's bind groups are rebuilt at the next frame.
    pub(crate) fn set_far_shadow_map(&self, v: Option<&wgpu::TextureView>) {
        let mut b = self.far_shadow_bind.lock().unwrap();
        b.0 += 1;
        b.1 = v.cloned();
    }

    pub(crate) fn set_lit_fog_volume(&self, v: Option<(&wgpu::TextureView, &wgpu::Buffer)>) {
        let mut b = self.lit_fog_bind.lock().unwrap();
        b.0 += 1;
        b.1 = v.map(|(t, s)| (t.clone(), s.clone()));
    }

    /// Whether a local light was lit at the last update (street lamps, vehicle
    /// lamps or a bore's lamps, with the lights on): the lit fog skips its
    /// passes otherwise (by day nothing is scattered).
    pub(crate) fn local_lights_on(&self) -> bool {
        self.state.lock().unwrap().lights_live
    }

    /// Stage A8 (`--tunnels`; Renderer::update, before the look's update):
    /// how far into a tunnel's bore the eye is (None outside: the exposure
    /// as before; inside, it adapts to the bore's lighting,
    /// `structures::bore_exposure_gain`), and whether bores with lamps are
    /// drawn (their lamps are lit round the clock, `LtParams.props.w`).
    pub fn set_bore(&self, depth: Option<f64>, tunnel_lamps: bool) {
        let mut s = self.state.lock().unwrap();
        s.bore_depth = depth;
        s.tunnel_lamps = tunnel_lamps;
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
    /// Hemispherical off-screen cloud reflection dome (`64 x 64` RGBA16F, hemi-octahedral),
    /// refreshed each frame in the volume pass when `--volumetric-clouds` is on.
    pub fn cloud_refl_view(&self) -> &wgpu::TextureView {
        &self.clouds.refl_view
    }
    /// the frame whose scene-meter reading the exposure waits on (after a cut or a turn), if any.
    pub fn meter_snap_pending(&self) -> Option<u32> {
        self.state.lock().unwrap().meter_snap_after
    }

    /// the frame whose scene-meter reading the coming frame's exposure uses: the
    /// one `scene_meter::LAG` frames back, never one from before the last cut or
    /// turn (the frame after it uses that frame's own).
    pub fn meter_reading_frame(&self) -> u32 {
        let s = self.state.lock().unwrap();
        s.frame.wrapping_add(1).saturating_sub(crate::look::scene_meter::LAG).max(s.meter_floor)
    }

    /// the frame counter `update` advances (the scene meter tags its readings with it).
    pub fn frame_index(&self) -> u32 {
        self.state.lock().unwrap().frame
    }

    /// A new shot (eo-app `--shots`, its clock stopped): the exposure
    /// meters afresh instead of adapting from the last shot's (a camera's
    /// first frame after a cut), so a view does not inherit the exposure
    /// of a room shot taken a few metres away. With the scene meter, as
    /// after a cut: only a reading of the coming frames (the new shot's)
    /// counts, and its first snaps the adaptation.
    pub fn snap_adaptation(&self) {
        let mut s = self.state.lock().unwrap();
        s.adapt.current = None;
        s.meter_fwd = None;
        let next = s.frame.wrapping_add(1);
        s.meter_snap_after = Some(next);
        s.meter_floor = next;
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

    /// Register the daylight camera around the photographed site.
    pub fn set_exterior_camera(&self, camera: crate::interior_meter::ExteriorCamera) {
        self.state.lock().unwrap().exterior_camera = Some(camera);
    }

    /// Register the glazed enclosure used by the camera meter in the playable interior.
    pub fn set_interior_meter(&self, meter: crate::interior_meter::InteriorMeter) {
        self.state.lock().unwrap().interior_meter = Some(meter);
    }

    /// The world clock (seconds since the world's epoch) and the frame time
    /// the next update advances it by. Default: 1/60 s a frame from
    /// `SkyOptions::world_time_s`.
    pub fn set_clock(&self, world_s: f64, dt: f64) {
        let mut s = self.state.lock().unwrap();
        s.world_s = world_s;
        s.dt = dt.max(0.0);
    }

    /// (bench captures) The frame counter moves on to the next multiple of
    /// `period`: the sub-pixel jitter's phase and every pass seeded by the frame
    /// (`flags.y`: the reflections' and the lit fog's jitter, the volume's
    /// reconstruction offset, the clouds' blue-noise step; all periodic in a
    /// divisor of `period`) start from one phase, so the frames a capture
    /// renders do not depend on how many frames its settle took (a wall-clock
    /// bound); the counter never goes back, so no history (the lit fog's frame
    /// stamp) can match a frame of the capture before it.
    pub fn align_frame_phase(&self, period: u32) {
        let mut s = self.state.lock().unwrap();
        s.frame = (s.frame / period + 1).wrapping_mul(period);
    }

    /// `--smooth-time`: the exposure converges instead of snapping on a turn or
    /// a clock jump (only a camera cut snaps it), the histories survive weather
    /// steps under `WEATHER_RECUT_STEPS`, midnight is no clock jump, and the moon's
    /// key weight blends the clouds' light (`SkyTerms::moon_weight`).
    /// (lighting, `--light-shadows`) the street lamps in SF's 3000 K LED colour.
    pub(crate) fn set_led_lamps(&self, on: bool) {
        self.state.lock().unwrap().led = on;
    }

    pub(crate) fn set_smooth_time(&self, on: bool) {
        self.state.lock().unwrap().smooth = on;
    }

    /// (`--photo-finish`) the level (0 off): the measured air and the lens's bloom.
    /// The measured air reaches the tables through the medium `update` writes
    /// (`atmo_m`: the aerosol's absorption and scale height, eo-sky `photo`), so
    /// the tables' passes are the default ones and the medium's change rebuilds them.
    pub(crate) fn set_photo_finish(&self, level: u8) {
        self.state.lock().unwrap().photo = level;
    }

    pub fn world_time(&self) -> f64 {
        self.state.lock().unwrap().world_s
    }

    /// The frame time each update advances the world clock by (`set_clock`;
    /// 1/60 s by default). The probe grid's fades run on it (lighting).
    pub fn frame_dt(&self) -> f64 {
        self.state.lock().unwrap().dt
    }

    /// The weather in force (preset, or the schedule at the world clock,
    /// re-derived once a world second: the ground water's integration is a
    /// few thousand steps).
    pub fn weather(&self) -> eo_sky::WeatherState {
        let mut s = self.state.lock().unwrap();
        // the live calendar's date follows the world clock this sky draws (eo-look
        // `sun::set_calendar_clock`; eo-sky's weather functions stay pure)
        eo_look::sun::set_calendar_clock(s.world_s);
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
        if far_street_light_enabled() {
            // (`--far-street-light`) the pools on the ground, for the streets past the frame's lights.
            let map: Vec<u16> = bake_lamp_map(&lamps[..n]).into_iter().map(crate::ambient_bake::f16_bits).collect();
            let side = LAMP_MAP_SIZE;
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &self.lamp_map, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                bytemuck::cast_slice(&map),
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(side * 2), rows_per_image: Some(side) },
                wgpu::Extent3d { width: side, height: side, depth_or_array_layers: 1 },
            );
        }
        n
    }

    pub fn lamp_count(&self) -> u32 {
        self.lamp_count.load(Ordering::Relaxed)
    }

    /// (`--signal-spill`) This frame's lit signal lenses near the eye, after the
    /// static lamps in their buffer (at most `SIGNAL_LIGHT_CAP`): (lens centre,
    /// its axis, aspect 0 red / 1 yellow / 2 green). The gather takes each as a
    /// spot of its beam (eo-sky `lights::SIGNAL_*`), not as a street lamp.
    pub fn set_signal_lights(&self, queue: &wgpu::Queue, lenses: &[([f32; 3], [f32; 3], u32)]) {
        let base = self.lamp_count.load(Ordering::Relaxed);
        let n = (lenses.len() as u32).min(SIGNAL_LIGHT_CAP).min(LAMP_CAP.saturating_sub(base));
        let (co, ci) = (eo_sky::lights::SIGNAL_OUTER_DEG.to_radians().cos() as f32, eo_sky::lights::SIGNAL_INNER_DEG.to_radians().cos() as f32);
        let gpu: Vec<GpuLight> = lenses[..n as usize]
            .iter()
            .map(|(p, d, a)| {
                let k = (*a as usize).min(2);
                let cd = eo_sky::lights::SIGNAL_CD[k];
                GpuLight {
                    pos: *p,
                    intensity: cd as f32,
                    color: eo_sky::lights::signal_color(k).map(|v| v as f32),
                    range: (cd / eo_sky::lights::SIGNAL_REACH_LUX).sqrt() as f32,
                    dir: *d,
                    cos_outer: co,
                    cos_inner: ci,
                    kind: SIGNAL_LIGHT_KIND,
                    // its lens's area: its reflections' source size (the lobe and the beam's
                    // clear near reach are the kind's, sky_light.wgsl)
                    size: eo_sky::lights::SIGNAL_LENS_AREA_M2 as f32,
                    pad: SIGNAL_MARK,
                }
            })
            .collect();
        if !gpu.is_empty() {
            queue.write_buffer(&self.lamps, base as u64 * 64, bytemuck::cast_slice(&gpu));
        }
        self.signal_count.store(n, Ordering::Relaxed);
    }

    /// The cells by the water (`set_waterside`): a lamp in one is drawn in the sea too.
    pub fn waterside_count(&self) -> u32 {
        self.waterside.load(Ordering::Relaxed)
    }

    /// The waterside cells: a grid of `cols` x `rows` cells of `cell` metres
    /// from `corner` (x, z), `cells` row-major (true: within the waterside
    /// reach of the shore). At most `WATER_CELLS_MAX` on a side.
    /// (`--look cinematic`, stage L2) The street lamp palette (eo-sky
    /// `cine_lamps::PaletteGrid`): each street's lamp colour and axis, read by
    /// the gather where a street luminaire stands (sky_lights.wgsl
    /// `lt_cine_street`). Its corner becomes the waterside grid's (eo-app
    /// builds both on the elevation grid's corner). Nothing is written without
    /// the look compiled in (the masks then have no room for it) or for a grid
    /// over `PALETTE_CELLS_MAX` on a side. Returns whether it was kept.
    pub fn set_lamp_palette(&self, queue: &wgpu::Queue, grid: &eo_sky::cine_lamps::PaletteGrid) -> bool {
        if (self.masks.size() as usize) < (MASK_WORDS + PALETTE_WORDS) * 4 || grid.cols > PALETTE_CELLS_MAX || grid.rows > PALETTE_CELLS_MAX || grid.cell <= 0.0 {
            return false;
        }
        let words = grid.words();
        queue.write_buffer(&self.masks, MASK_WORDS as u64 * 4, bytemuck::cast_slice(&words[..words.len().min(PALETTE_WORDS)]));
        let mut s = self.state.lock().unwrap();
        s.water.0[0] = grid.corner[0] as f32;
        s.water.0[1] = grid.corner[1] as f32;
        s.palette = (grid.cell as f32, grid.cols as u32, grid.rows as u32);
        true
    }

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

    /// (`--shop-light`, stage N1) The shop windows' lights (shop_lights.rs
    /// `build`; replaces the set, once their placement is done): gathered each
    /// night frame near the eye (`SkyOptions::shop_light`'s reach), lit by
    /// their hours. An empty set binds the stand-in and gathers nothing.
    pub fn set_shop_lights(&self, queue: &wgpu::Queue, lights: &[crate::shop_lights::GpuShopLight]) {
        let mut s = self.state.lock().unwrap();
        s.shops = (!lights.is_empty()).then(|| {
            let b = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("sky-shop-lights"),
                size: (lights.len() * std::mem::size_of::<crate::shop_lights::GpuShopLight>()) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&b, 0, bytemuck::cast_slice(lights));
            (b, lights.len() as u32)
        });
        // The light bind groups carry them: built again.
        s.lights_bg0 = None;
        s.agents_bg = None;
    }

    /// The shop windows' lights handed over (0 before or without them).
    pub fn shop_light_count(&self) -> u32 {
        self.state.lock().unwrap().shops.as_ref().map(|(_, n)| *n).unwrap_or(0)
    }

    /// The last gather's frame list (as many as it kept) and every light
    /// cluster's list (its count, then its slots): tests and the capture
    /// cards' census (blocks; diagnostics only).
    pub fn lights_readback(&self, ctx: &crate::GpuContext) -> anyhow::Result<(Vec<GpuLight>, Vec<Vec<u32>>)> {
        let copy = |b: &wgpu::Buffer| -> anyhow::Result<Vec<u8>> {
            let staging = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: Some("sky-lights-readback"), size: b.size(), usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            let mut enc = ctx.device.create_command_encoder(&Default::default());
            enc.copy_buffer_to_buffer(b, 0, &staging, 0, b.size());
            ctx.queue.submit([enc.finish()]);
            crate::read_mapped(ctx, &staging)
        };
        let count: Vec<u32> = bytemuck::pod_collect_to_vec(&copy(&self.counter)?);
        let n = (count[0] as usize).min(FRAME_LIGHT_CAP as usize);
        let lights: Vec<GpuLight> = bytemuck::pod_collect_to_vec(&copy(&self.frame_lights)?);
        let words: Vec<u32> = bytemuck::pod_collect_to_vec(&copy(&self.clusters)?);
        let ncl = (CLUSTERS[0] * CLUSTERS[1] * CLUSTERS[2]) as usize;
        let cap = CLUSTER_CAP as usize;
        let clusters = (0..ncl).map(|c| words[ncl + c * cap..ncl + c * cap + (words[c] as usize).min(cap)].to_vec()).collect();
        Ok((lights[..n].to_vec(), clusters))
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
    /// `scene`: the scene meter's latest reading and the display's shoulder
    /// (`--no-scene-meter`: None, the dome estimate below meters the frame).
    /// `source_lens`: the playable house's calibrated source camera (`--blender-camera`) may take the ground floor's lens.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update(&self, queue: &wgpu::Queue, cam: &Camera, f: &eo_look::LookFrame, terms: &eo_sky::SkyTerms, out_size: (u32, u32), depth_ok: bool, daylight: Option<&eo_look::phys::Daylight>, scene: Option<(Option<eo_sky::night::SceneMeterReading>, f64)>, source_lens: bool) -> f64 {
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
        // `--smooth-time`: the clock's distance wraps at midnight (a moving clock
        // passes it), and only a weather change of `WEATHER_RECUT_STEPS` key steps or of
        // kind between two updates recuts the histories (a schedule's slow drift does not).
        let smooth = s.smooth;
        let hour_gap = |a: f64, b: f64| {
            let d = (a - b).abs();
            if smooth { d.rem_euclid(24.0).min(24.0 - d.rem_euclid(24.0)) } else { d }
        };
        let weather_moved = |a: (i64, i64, i64), b: (i64, i64, i64)| {
            if smooth { a.2 != b.2 || (a.0 - b.0).abs() >= WEATHER_RECUT_STEPS || (a.1 - b.1).abs() >= WEATHER_RECUT_STEPS } else { a != b }
        };
        let jumped = s.adapt.last_hour.is_some_and(|h| hour_gap(h, f.hour) >= 0.25);
        // A change of hour or weather discards the histories for two frames (the first
        // frame after a change may still carry the last look's terms).
        let look_now = (f.hour, weather_key(&terms.weather));
        if s.hist_look.is_some_and(|l| hour_gap(l.0, look_now.0) >= 0.25 || weather_moved(l.1, look_now.1)) {
            s.recut = 2;
        }
        s.hist_look = Some(look_now);
        // the recut frames count down per encoded frame (in `encode`), not per update.
        let recut = s.recut > 0;
        // a camera cut: the first frame, or a move of more than 25 m (a teleport).
        let camera_cut = s.prev_vp.is_none() || cam.eye.distance(prev_eye) > 25.0;
        let cut = camera_cut || jumped || recut || s.cut_hold;
        s.cut_hold = cut;
        // Frames of history since the cut: the volume averages them evenly until its
        // own weight takes over (one frame no longer stands for a dozen).
        s.hist_n = if cut || !s.history_valid { 0 } else { s.hist_n.saturating_add(1) };
        // the camera meter (eo-sky `night::meter`) over a 5 x 3 grid of
        // the view, centre-weighted; a cut or a turn of more than ~30 deg
        // snaps the adaptation (a new shot, not a pan).
        let turned = s.meter_fwd.is_some_and(|p| p.dot(cam.forward()) < 0.87);
        let meter = if o.meter && let Some((reading, shoulder)) = &scene {
            // the scene meter: a reading of this view (taken after the last cut or
            // turn) sets the exposure; its first snaps the adaptation (a new shot).
            // (`--smooth-time`: only a camera cut, as below; `step_smooth` converges.)
            if camera_cut || ((cut || turned) && !smooth) {
                s.meter_snap_after = Some(s.frame);
                s.meter_floor = s.frame;
            }
            match reading {
                Some(r) if s.meter_snap_after.is_none_or(|a| r.frame >= a) => {
                    if s.meter_snap_after.take().is_some() {
                        s.adapt.current = None;
                    }
                    let k = eo_sky::night::scene_meter(f.model.sun.altitude_deg, &r.hist, *shoulder);
                    let range = eo_sky::night::SCENE_METER_RANGE_EV.exp2();
                    let m = (r.prior * k / terms.exposure_target.max(1e-12)).clamp(1.0 / range, range);
                    if std::env::var_os("EO_SCENE_METER_LOG").is_some() {
                        eprintln!("scene meter: frame {} reading {} factor {:+.2} EV (prior {:.4})", s.frame, r.frame, m.log2(), terms.exposure_target);
                    }
                    m
                }
                _ => 1.0,
            }
        } else if o.meter {
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
            // inside a landmark deck's room (deck_rooms.rs; none with `--no-landmark-access`),
            // a direction that meets the room's own surfaces meters them, not the sky or the open ground
            // (the dome estimate's correction only: the scene meter above reads the room in the frame,
            // as it does the house's rooms; the house's interior meter below is 1 away from the house).
            let e = cam.eye;
            let room = crate::deck_rooms::classify([e.x as f64, e.y as f64, e.z as f64], &dirs.iter().map(|d| d.0).collect::<Vec<_>>());
            let room_dirs = room.map(|k| dirs.iter().zip(k).map(|(d, k)| (d.0, d.1, k)).collect::<Vec<_>>());
            let sky_meter = match daylight.filter(|d| d.weight > 0.0) {
                // the physical daylight's sky and card where it lights the frame (by its weight)
                Some(d) => {
                    let wt = d.weight;
                    let beam = eo_sky::night::key_on_ground(f) * eo_sky::night::mean_beam(&terms.weather);
                    let card_js = 0.18 * (beam + eo_sky::night::ground_fill(f, eo_sky::night::SIDEWALK_ENV)) / std::f64::consts::PI;
                    let card_ph = 0.18 * eo_sky::night::horizontal_illuminance(f, &terms.weather, wt) / std::f64::consts::PI;
                    let card = card_js + (card_ph - card_js) * wt;
                    let eye_m = cam.eye.y as f64;
                    let sky = |dd: [f64; 3]| {
                        let js = eo_look::math::luminance(f.model.dome(dd));
                        js + (eo_look::math::luminance(d.sky(dd, eye_m)) - js) * wt
                    };
                    match &room_dirs {
                        // a room's card: the card without the sun's beam (the room's sky visibility scales it)
                        Some(rd) => eo_sky::night::meter_room_with(f, rd, card, card - 0.18 * beam / std::f64::consts::PI, &sky),
                        None => eo_sky::night::meter_with(f, &dirs, card, &sky),
                    }
                }
                None => match &room_dirs {
                    Some(rd) => eo_sky::night::meter_room(f, &terms.weather, rd),
                    None => eo_sky::night::meter(f, &terms.weather, &dirs),
                },
            };
            // the playable house's interior meter (eo-app `--house-photo`): the light the
            // view takes in through the house's glazing, while the eye is inside it.
            let gain = s.interior_meter.as_ref().map_or(1.0, |m| m.scene_gain(cam, f, &terms.weather, &dirs, sky_meter, terms.exposure_target));
            sky_meter * (1. + (gain - 1.) * (1. - terms.nightness))
        } else {
            1.0
        };
        s.meter_fwd = Some(cam.forward());
        // (`--smooth-time`: neither a turn nor a clock jump resets it; `step_smooth` converges.)
        if (cut || turned) && !smooth {
            s.adapt.current = None;
        }
        // Exposure: the target (weather, night solve, the meter), filtered.
        let mut ordinary = terms.exposure_target * meter;
        if let Some(d) = s.bore_depth {
            // in a tunnel's bore (`--tunnels`): adapted to its lighting, the meter giving way.
            ordinary = terms.exposure_target * crate::structures::bore_exposure_gain(f, terms, meter, d);
        }
        // the playable house's site camera (`--house-site-camera`): its registered exposure near the house.
        let target = s.exterior_camera.as_ref().filter(|_| o.meter).map_or(ordinary, |camera| {
            let daylight = (1.-terms.nightness)*(1.-terms.weather.profile.overcast);
            camera.target(cam.eye, ordinary, daylight.clamp(0.,1.))
        });
        let exposure = if o.adaptation {
            if smooth { s.adapt.step_smooth(target, f.hour, dt, camera_cut) } else { s.adapt.step(target, f.hour, dt) }
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
        let vol_decks;
        let decks = if o.volumetric_clouds {
            vol_decks = eo_sky::clouds::volumetric_decks(w);
            &vol_decks
        } else {
            &terms.decks
        };
        let world_s = s.world_s;
        // the decks drift with the hour of the day too (the clock the
        // look is set to), so each hour shows another field (every card of one
        // camera showed the same clouds at 12, 18.5, 19.4 and 21 h).
        let drift_s = world_s + f.hour * 3600.0;
        let low_wrap = if o.volumetric_clouds { eo_sky::clouds::VOL_WRAP_TILE_M } else { eo_sky::clouds::LOW_TILE_M };
        let high_wrap = if o.volumetric_clouds { eo_sky::clouds::VOL_WRAP_TILE_M } else { eo_sky::clouds::HIGH_TILE_M };
        let low = eo_sky::clouds::drift_offset(decks.drift_low, drift_s, low_wrap);
        let high = eo_sky::clouds::drift_offset(decks.drift_high, drift_s, high_wrap);
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
        // (`--urban-sky`) the skyglow is the street lamps' light scattered back by the air over the
        // city: on their photometric scale (`--night-photometry`'s practical gain; 1 without it).
        let glow = if o.urban_sky && o.night_photometry { glow * eo_sky::night_photometry::practical_gain(f.model.sun.altitude_deg) } else { glow };
        // (lighting, `--light-shadows`) the glow in the LED lamps' colour.
        let pool = if s.led { eo_sky::lights::led_street_lamp_color() } else { eo_sky::lights::street_lamp_color() };
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
        // (`--smooth-time`: by the moon's weight in the key, which blends instead of switching.)
        let moon_weight = if smooth { terms.moon_weight } else if moon_keys { 1.0 } else { 0.0 };
        let moon_fade = if smooth {
            1.0 + (eo_look::math::smoothstep(-4.0, -12.0, alt) - 1.0) * moon_weight
        } else if moon_keys {
            eo_look::math::smoothstep(-4.0, -12.0, alt)
        } else {
            1.0
        };
        // (`--smooth-time`: the clouds hand over to the moon only once the Earth's
        // shadow has risen over them, `CLOUD_MOON_HANDOVER_DEG`; sky_clouds.wgsl `cl_sun`.)
        let cloud_moon = moon_weight * (1.0 - eo_look::math::smoothstep(CLOUD_MOON_HANDOVER_DEG[1], CLOUD_MOON_HANDOVER_DEG[0], alt));
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
        // `--smooth-time`: the volume's light as the sun's part (its key, at least
        // the orange above) from the sun's key direction, plus the moon's part
        // (as above, from -4 deg) from the moon, the direction turning by their
        // shares (`beam.yzw`, sky_volume.wgsl). Without it the volume took the
        // surfaces' key direction, which swings to the moon at the ground's
        // takeover while the light is still the sunset's: a step in the haze.
        let (vkey, vol_dir) = if smooth {
            let neutralise = |c: [f64; 3]| {
                let l = eo_look::math::luminance(c);
                c.map(|v| v + (l - v) * neutral)
            };
            let moon_rgb = eo_sky::moon::COLOR.map(|v| v * terms.moon_intensity);
            let sun_rgb = [0, 1, 2].map(|i| (f.key[i] - moon_rgb[i]).max(0.0)); // (apply_with's key: the sun's share plus the moon)
            let k = eo_look::math::smoothstep(-2.0, 0.5, alt) * (1.0 - eo_look::math::smoothstep(1.5, 4.0, alt)) * (1.0 - ovc);
            let vsun = { let n = neutralise(sun_rgb); [0, 1, 2].map(|i| n[i].max(SUNSET_CLOUD_KEY[i] * k)) };
            let vmoon = neutralise(moon_rgb).map(|v| v * eo_look::math::smoothstep(-4.0, -12.0, alt));
            let (ls, lm) = (eo_look::math::luminance(vsun), eo_look::math::luminance(vmoon));
            let share = if ls + lm > 0.0 { lm / (ls + lm) } else { 0.0 };
            ([0, 1, 2].map(|i| vsun[i] + vmoon[i]), eo_look::blend::slerp_dir(eo_look::rig::key_direction(f.hour), terms.moon.dir, share))
        } else {
            (vkey, f.key_dir)
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
        // (`--photo-finish`) San Francisco's measured air (eo-sky `photo`): the tables'
        // aerosol (its density here, its absorption and scale height in the medium below),
        // the aerial perspective's extinction (`atmo3.y` below); the lens's bloom (eo-look `tone`).
        let photo = s.photo > 0;
        let atmo_haze = if photo { eo_sky::photo::lut_density(w.profile.haze) as f32 } else { (ATMO_HAZE * (1.0 + 2.0 * w.profile.haze)) as f32 };
        // the medium the tables integrate: the clear coastal air (eo-look `atmosphere`) with the
        // physical daylight, else the standard aerosol x ATMO_HAZE as before; both x the weather's haze.
        // With the physical daylight the medium moves from the old one to the coastal air by
        // the daylight's weight (the sun up: all of it; the twilight and the night keep the
        // tables they were tuned on). The haze density folds into the aerosol's coefficients.
        let legacy = eo_look::atmosphere::Atmosphere::legacy();
        // (`--photo-finish`) the measured air's aerosol: its absorption per unit density and
        // its scale height (eo-sky `photo::LUT_MIE_ABSORB_PHOTO_PER_KM`, `LUT_AEROSOL_SCALE_KM`).
        let legacy = if photo { eo_look::atmosphere::Atmosphere { aerosol_abs: eo_sky::photo::LUT_MIE_ABSORB_PHOTO_PER_KM, aerosol_h_km: eo_sky::photo::LUT_AEROSOL_SCALE_KM, ..legacy } } else { legacy };
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
        // (`--look cinematic`, stage L1) the look's bloom (eo-look `cinematic::grade`): a lower,
        // softer threshold, a higher ceiling, the levels' weights summing to the share the
        // cinematic post pass takes off each pixel (energy kept); over the photo finish's too
        // (one bloom).
        let cine = o.cinematic;
        let threshold = if cine { eo_look::cinematic::grade::CINE_BLOOM_THRESHOLD_X / exposure.max(1e-3) } else if photo { eo_look::tone::PHOTO_BLOOM_THRESHOLD_X / exposure.max(1e-3) } else { 2.101 / exposure.max(1e-3) };
        // The fitted source camera has no glare compositor. Its response is
        // calibrated independently of the city's exterior lens.
        let lens_weight = if source_lens {
            s.interior_meter.as_ref().map_or(0., |m| m.ground_floor_lens(cam.eye))
        } else { 0. };
        let porch_lens = s.exterior_camera.as_ref().map_or(0., |c| c.porch_lens(cam.eye));
        let glare = if o.bloom { 1. - lens_weight.max(porch_lens) } else { 0. };
        s.frame_bloom = glare > 0.;
        let (strength, radius) = (0.26, 0.22);
        let mw = if cine { eo_look::cinematic::grade::bloom_weights().map(|v| v as f32) } else if photo { eo_look::tone::PHOTO_BLOOM_WEIGHTS.map(|v| v as f32) } else { [1.0, 0.8, 0.6, 0.4, 0.2].map(|fv: f64| ((fv + (1.2 - fv - fv) * radius) * strength) as f32) };
        let lamp_k = terms.lamp_units_per_lux;
        let lamps_on = if o.lights { terms.lamps_on } else { 0.0 };
        // (`--shop-light`) the shop windows' lights, once placed: lit with the rooms
        // behind kit glass (rooms.rs `drive`'s dusk), at the lamps' units per candela.
        let shop_n = s.shops.as_ref().filter(|_| o.shop_light > 0 && o.lights).map(|(_, n)| *n).unwrap_or(0);
        let shop_k = if shop_n > 0 { lamp_k * crate::rooms::drive_ramp(f.model.sun.altitude_deg, f.hour, crate::rooms::dusk_ramp()).0[0] as f64 } else { 0.0 };
        let shop_bits = if shop_n > 0 { crate::shop_lights::SKY_SHOPS | if o.shop_light >= 2 { crate::shop_lights::SKY_SHOPS_FAR } else { 0 } } else { 0 };
        let veh = if o.lights { terms.vehicle_lamps } else { 0.0 };
        let moon = terms.moon;
        let volume_ok = depth_ok && o.needs_volume();
        // stage A8 (`--tunnels`): the bores' interior irradiance (0 without bores or lights).
        let bore_units = if s.tunnel_lamps && o.lights { crate::structures::bore_interior_units(f, terms) } else { 0.0 };
        // (lighting, `--lit-fog`) any local light lit this frame.
        s.lights_live = lamps_on > 0.0 || veh > 0.0 || bore_units > 0.0;
        let u = SkyUniform {
            vp: vp.to_cols_array_2d(),
            prev_vp: prev_vp.to_cols_array_2d(),
            eye: [cam.eye.x, cam.eye.y, cam.eye.z, (world_s % 86400.0) as f32],
            prev_eye: prev_eye.extend(o.cumulus_weight(cam.eye)).to_array(),
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
            sun_rad: v4(vkey, if smooth { cloud_moon } else if terms.moon_key > 0.0 && f.key_dir == moon.dir { 1.0 } else { 0.0 }),
            amb_top: v4(amb_top, glow),
            amb_base: v4(amb_base, o.cloud_steps() as f64),
            wet: [if o.wet { w.wetness as f32 } else { 0.0 }, w.puddle_fill as f32, if o.rain || o.wet { rate as f32 } else { 0.0 }, (world_s % 3600.0) as f32],
            rain: [live_near as f32, live_far as f32, fall as f32, 0.3], // streak strength 0.62 -> 0.3 (opaque white rods)
            wind: [w.wind[0] as f32, w.wind[1] as f32, w.gust as f32, w.flash as f32],
            lights: [if o.lights { FRAME_LIGHT_CAP } else { 0 }, CLUSTERS[0], CLUSTERS[1], CLUSTERS[2]],
            cluster: [CLUSTER_NEAR_M, CLUSTER_FAR_M, (0.5 * cam.fov_y_deg.to_radians()).tan(), cam.aspect],
            cam_fwd: fwd.extend(0.0).to_array(),
            cam_right: right.extend(0.0).to_array(),
            cam_up: up.extend(porch_lens).to_array(),
            night: [lamps_on as f32, terms.dusk as f32, terms.nightness as f32, veh as f32],
            moon: v4(moon.dir, if o.moon_stars { terms.moon_disc } else { 0.0 }),
            stars: [if o.moon_stars { terms.star_level as f32 } else { 0.0 }, (f.hour * 15.041 * std::f64::consts::PI / 180.0) as f32, moon.illuminated as f32, if moon.waxing { 1.0 } else { 0.0 }],
            // (`--look cinematic`: z the lamp glare's angle, rad; the default's z 1 is read by nothing)
            // w: the bloom's share (0 off; under 1 where the house's source lens takes the view)
            bloom: [threshold as f32, if cine { eo_look::cinematic::grade::CINE_BLOOM_KNEE as f32 } else if photo { eo_look::tone::PHOTO_BLOOM_KNEE as f32 } else { 0.55 }, if cine { eo_look::cinematic::grade::CINE_GLARE_ANGLE_DEG.to_radians() as f32 } else { 1.0 }, glare],
            // (the levels' weights reach the pyramid through `bloom_levels`; no shader reads these.
            // `--look cinematic`: the lamp glare's share, longest ray (px of this frame), the EV it
            // starts at and, negative (the look's mark), its EV range: sky_fx.wgsl `fx_glare`.)
            bloom_w: if cine { glare_params(out_size.1) } else { [mw[0], mw[1], mw[2], mw[3]] },
            bloom_w2: [mw[4], if cine { eo_look::cinematic::grade::CINE_BLOOM_CEILING as f32 } else if photo { eo_look::tone::PHOTO_BLOOM_CEILING as f32 } else { 4.0 }, lamp_k as f32, w.profile.lamp_haze as f32],
            flags: [o.bits() | shop_bits | if w.kind == eo_sky::Kind::Damp { SKY_DAMP_BIT } else { 0 }, s.frame, (o.ssr && s.history_valid && !cut) as u32, volume_ok as u32],
            // yzw: `--smooth-time`'s light direction for the volume (0 without it).
            beam: if smooth { [eo_sky::night::mean_beam(w) as f32, vol_dir[0] as f32, vol_dir[1] as f32, vol_dir[2] as f32] } else { [eo_sky::night::mean_beam(w) as f32, 0.0, 0.0, 0.0] },
            twi: v4(twi, bore_units),
            atmo: [m.sun.x as f32, m.sun.y as f32, m.sun.z as f32, ATMO_SKY_SCALE as f32],
            atmo2: [
                if o.atmosphere { 1.0 } else { 0.0 },
                // under a deck the weather's own (CIE overcast) dome
                (eo_look::math::smoothstep(-16.0, -9.0, m.sun.altitude_deg) * (1.0 - 0.9 * w.profile.overcast.clamp(0.0, 1.0))) as f32,
                atmo_haze,
                ATMO_CLOUD_SUN as f32,
            ],
            sun_glint: if smooth { sun_glint_smooth(f.hour, eo_sky::night::mean_beam(&w)) } else { sun_glint(f.hour, eo_sky::night::mean_beam(&w)) },
            atmo3: {
                // the weather's visibility over clear air's: the range fog reached 95 % at `aerial.far`
                let far = f.aerial.far.max(100.0);
                let far_clear = far * eo_sky::weather::CLEAR.vis_far / w.profile.vis_far.max(1e-3);
                // the weather's own veil is drawn here with the atmosphere on (look.rs
                // pushes the range fog out): surfaces fade to the sky the volume draws behind
                // them, not to `look.fog_color` (lighter and bluer than the grey fog bank and
                // rain veil, so the far hills stood out of the fog in front of them).
                let ap = if photo { eo_sky::photo::ap_extinction_per_m(w.profile.haze) as f32 } else { (AP_HAZE_PER_KM * 0.001 * (1.0 + 2.0 * w.profile.haze)) as f32 };
                [if o.atmosphere && far < far_clear * 0.97 { 1.0 } else { 0.0 }, ap, f.aerial.near as f32, far as f32]
            },
            atmo4,
            atmo_m,
            atmo_s,
            atmo_r,
            atmo_o,
            cirrus: {
                let c = decks.cirrus();
                [c.alt_m as f32, c.opacity as f32, if o.volumetric_clouds { 1.0 } else { 0.0 }, if o.weather_fx { 1.0 } else { 0.0 }]
            },
            // (`--far-street-light`) the far streets' lamp light: rgb the lamps' colour (unit
            // luminance; SF's LED under the lighting's `led`) x units per lux x lamps on, w on.
            spare: {
                let c = if s.led { let l = led_params().0; [l[0] as f64, l[1] as f64, l[2] as f64] } else { eo_sky::lights::street_lamp_color() };
                let k = lamp_k * lamps_on;
                let on = far_street_light_enabled() && k > 0.0;
                [(c[0] * k) as f32, (c[1] * k) as f32, (c[2] * k) as f32, if on { 1.0 } else { 0.0 }]
            },
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
        s.bloom_terms = [u.bloom[0], u.bloom[1], u.bloom_w2[1], u.bloom[3]];
        // The lights' scales: candela -> irradiance units, on with the lamps.
        let agents = s.agents_bg.as_ref().map(|a| a.0).unwrap_or(0);
        // The props lamps carry the JS practical's intensity in the JS's light
        // units, which are this rig's (only the lamps' clock and the twilight
        // scale, `lamp_k` over the night's units per lux, apply); their pools
        // are set to the RP-8 peak (sky_lights.wgsl `lights_props`).
        let props_k = lamp_k / eo_sky::night::lamp_units_per_lux();
        let tiles_any = s.tiles.iter().any(|w| *w != 0);
        // stage A8 (`--tunnels`): the bores' lamps' pool at the nadir (0 without bores or lights).
        let tunnel_pool = bore_units / crate::structures::TUNNEL_POOL_OVERLAP;
        let p = LtParams {
            counts: [self.lamp_count.load(Ordering::Relaxed) + self.signal_count.load(Ordering::Relaxed), agents, FRAME_LIGHT_CAP, (s.agents_bg.is_some()) as u32],
            // (z: `--lamp-streaks`, the cars' rear lamps' units per candela, with the street
            // lamps' clock: by day a rear lamp's few hundred cd/m2 are lost in the daylit
            // film, which the lamps' constant night scale would not show; 0 off)
            scale: [(lamp_k * lamps_on) as f32, (lamp_k * eo_sky::photometry::LOW_BEAM_PEAK_CD * veh) as f32, if o.lamp_streaks { (lamp_k * veh * lamps_on) as f32 } else { 0.0 }, 45.0],
            props: [(props_k * lamps_on) as f32, if tiles_any { 1.0 } else { 0.0 }, (eo_sky::photometry::LAMP_POOL_PEAK_LUX * lamp_k * lamps_on) as f32, tunnel_pool as f32],
            // (`--look cinematic`, stage L2) w the lamp palette's cell (0: none or the look off),
            // the waterside's z, w its columns and rows (both unread by the gather without the look).
            water: [s.water.0[0], s.water.0[1], s.water.0[2], if o.cinematic { s.palette.0 } else { 0.0 }],
            water_n: [s.water.1[0], s.water.1[1], s.palette.1, s.palette.2],
            led: if s.led { led_params().0 } else { [0.0; 4] },
            led_key: if s.led { led_params().1 } else { [0.0; 4] },
            shops: [shop_n as f32, shop_k as f32, f.hour.rem_euclid(24.0) as f32, crate::shop_lights::reach_m(o.shop_light) as f32],
        };
        queue.write_buffer(&self.lt_params, 0, bytemuck::bytes_of(&p));
        let bw: Vec<[f32; 4]> = if bloom_level_count() == CAM_BLOOM_LEVELS && camera_on(&o) {
            // (`--cine-camera`) the camera's levels (cine_post.rs `bloom_level_weights`, summing to 1:
            // the prefilter takes the shares), and the exposure the prefilter meters by (level 0's z).
            let w = crate::cine_post::bloom_level_weights();
            let n = CAM_BLOOM_LEVELS;
            (0..n).map(|i| if i + 1 == n { [0.0; 4] } else { [w[i], if i + 2 == n { w[i + 1] } else { 1.0 }, if i == 0 { exposure as f32 } else { 0.0 }, 0.0] }).collect()
        } else if bloom_level_count() == CAM_BLOOM_LEVELS {
            // the camera's levels made but the model off: the five levels' weights, the two
            // widest levels empty (the old prefilter: level 0's z is 0).
            vec![[mw[0], 1.0, 0.0, 0.0], [mw[1], 1.0, 0.0, 0.0], [mw[2], 1.0, 0.0, 0.0], [mw[3], 1.0, 0.0, 0.0], [mw[4], 1.0, 0.0, 0.0], [0.0; 4], [0.0; 4]]
        } else {
            vec![[mw[0], 1.0, 0.0, 0.0], [mw[1], 1.0, 0.0, 0.0], [mw[2], 1.0, 0.0, 0.0], [mw[3], mw[4], 0.0, 0.0], [0.0, 0.0, 0.0, 0.0]]
        };
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
                    let bg = self.lights_bg(Some((ab, mb)), s.props.as_ref(), s.shops.as_ref().map(|x| &x.0));
                    s.agents_bg = Some((n, bg));
                }
            },
            None => s.agents_bg = None,
        }
    }

    fn lights_bg(&self, agents: Option<(&wgpu::Buffer, &wgpu::Buffer)>, props: Option<&(wgpu::Buffer, wgpu::Buffer)>, shops: Option<&wgpu::Buffer>) -> wgpu::BindGroup {
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
                wgpu::BindGroupEntry { binding: 12, resource: shops.unwrap_or(d).as_entire_binding() }, // (`--shop-light`)
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
        let vol_clouds = self.state.lock().unwrap().opts.volumetric_clouds;
        // the cloud noise (once) and this frame's cloud-shadow map, before the city pass reads it
        self.clouds.encode(enc, ts.map(|(q, b)| (q, b + Q_CLOUD_SHADOW)), vol_clouds);
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
            s.lights_bg0 = Some(self.lights_bg(None, s.props.as_ref(), s.shops.as_ref().map(|x| &x.0)));
        }
        let bg = match &s.agents_bg {
            Some((_, bg)) => bg,
            None => s.lights_bg0.as_ref().unwrap(),
        };
        let agents = s.agents_bg.as_ref().map(|a| a.0).unwrap_or(0);
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("sky-lights"), timestamp_writes: tw });
        pass.set_bind_group(0, bg, &[]);
        let lamps = self.lamp_count.load(Ordering::Relaxed) + self.signal_count.load(Ordering::Relaxed); // (`--signal-spill`: its lenses after the lamps)
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
        // (`--shop-light`) the shop windows' lights, after every other (the frame list's cap drops them first).
        if let Some((_, n)) = s.shops.as_ref().filter(|(_, n)| *n > 0 && s.opts.shop_light > 0) {
            pass.set_pipeline(&self.lights_pipes[4]);
            pass.dispatch_workgroups(n.div_ceil(64), 1, 1);
        }
        pass.set_pipeline(&self.lights_pipes[2]);
        pass.dispatch_workgroups(ncl, 1, 1);
        // (`--look cinematic`, stage L3) the streak lights, after the clusters
        if let (Some(p), true) = (&self.streaks_pipe, s.opts.cinematic) {
            pass.set_pipeline(p);
            pass.dispatch_workgroups(1, 1, 1);
        }
        // (`--lamp-streaks`) the cars' rear lamps after them (none written while the switch is off)
        if let (Some(p), true) = (&self.lamp_streaks_pipe, s.opts.cinematic) {
            pass.set_pipeline(p);
            pass.dispatch_workgroups(1, 1, 1);
        }
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
        // (`--screen-reflect`) with their mip chain (level-0 views as without it)
        let (hist, pyr) = match self.reflect.as_ref().filter(|_| hdr_format == wgpu::TextureFormat::Rgba16Float) {
            Some(r) => {
                let (h, p) = r.histories(d, &self.uniform, size, hdr_format, ru);
                (h, Some(p))
            }
            None => ([tex(d, "sky-history-0", size, hdr_format, ru), tex(d, "sky-history-1", size, hdr_format, ru)], None),
        };
        let vel = tex(d, "sky-velocity", size, VEL_FORMAT, ru);
        let mut mips = vec![];
        let mut ups = vec![];
        let mut bytes = half.0 as u64 * half.1 as u64 * (8 + 4) * 2 + size.0 as u64 * size.1 as u64 * (8 * 2 + 4) + size.0 as u64 * size.1 as u64 * (8 + 4) * 2;
        // (`--cine-camera`: `CAM_BLOOM_LEVELS`, else `BLOOM_LEVELS`)
        let levels = bloom_level_count();
        for i in 0..levels {
            let s = ((size.0 >> (i + 1)).max(1), (size.1 >> (i + 1)).max(1));
            mips.push(tex(d, "sky-bloom-down", s, wgpu::TextureFormat::Rgba16Float, su));
            bytes += s.0 as u64 * s.1 as u64 * 8;
            if i + 1 < levels {
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
            for i in 1..levels {
                list.push(bg(&mips[i - 1], &mips[i - 1], &mips[i], i));
            }
            // up_3 = m3 w3 + tent(m4) w4; up_i = m_i w_i + tent(up_(i+1)).
            for i in (0..levels - 1).rev() {
                let src = if i == levels - 2 { &mips[i + 1] } else { &ups[i + 1] };
                list.push(bg(src, &mips[i], &ups[i], i));
            }
        }
        bytes += pyr.as_ref().map(|p| p.bytes).unwrap_or(0);
        *t = Some(Targets { size, vol, volz, full, fullz, hist, vel, _mips: mips, ups, scene: None, lit_fog_gen: 0, bloom_bgs, bytes, pyr });
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

    /// The bloom's prefilter threshold, knee and ceiling and its share as the
    /// last `update` wrote them (the uniform's `bloom.x`, `bloom.y`,
    /// `bloom_w2.y`, `bloom.w`; zeros before the first): photo mode adds the
    /// bloom as post would (photo_mode.rs `PostBloom`).
    pub(crate) fn bloom_terms(&self) -> [f32; 4] {
        self.state.lock().unwrap().bloom_terms
    }

    /// (`--screen-reflect`) The previous resolved frame for the city pass: every
    /// level of its mip chain where the histories carry one, else `history_view`.
    pub(crate) fn reflect_view(&self, p: usize) -> Option<wgpu::TextureView> {
        self.targets.lock().unwrap().as_ref().map(|t| t.pyr.as_ref().map(|r| r.all[p].clone()).unwrap_or_else(|| t.hist[p].clone()))
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
    /// reads the HDR scene). `mid` is called once a frame between the volume
    /// and the resolve (the lit fog's hook), with whether the volume and the
    /// resolve run this frame (false: they are skipped, it is called before
    /// returning). It runs while the sky's targets are locked, so it must not
    /// call the `SkyGpu` methods that lock them (`history_view`, `bloom_view`,
    /// `ensure_targets`, `target_bytes`), nor anything that rebuilds the look's
    /// main bind groups (`LookGpu::set_lighting_bindings` and the per-feature setters).
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
        mid: &mut dyn FnMut(&mut wgpu::CommandEncoder, bool),
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
        let (o, parity, history_valid, frame_bloom) = {
            let s = self.state.lock().unwrap();
            (s.opts, s.parity, s.history_valid, s.frame_bloom)
        };
        let depth_ok = depth_now;
        if !o.any_post() || !depth_ok {
            for slot in [Q_SKY_FX, Q_SKY_VOLUME, Q_SKY_RESOLVE, Q_SKY_BLOOM] {
                empty(enc, slot);
            }
            mid(enc, false); // the lit fog hook (its timestamps), with nothing to composite into
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
                // (`--look cinematic`) the brightest lights' glare (the same list; 6 of the 12 vertices).
                // (`--cine-camera`: none; the camera's bloom and the sun's glare stand in for it)
                if let (Some(p), true) = (&self.glare_pipe, o.cinematic && frame_bloom && !camera_on(&o)) {
                    pass.set_pipeline(p);
                    pass.draw_indirect(&self.counter, 16);
                }
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
        // (lighting, `--lit-fog`) the lit fog's volume (or its stand-ins) at resolve bindings 10 and 11.
        let (fog_gen, fog_bind) = {
            let b = self.lit_fog_bind.lock().unwrap();
            (b.0, b.1.clone())
        };
        // (stage L4) the far sun shadow map's generation rides in the high half of the key.
        let (far_gen, far_bind) = {
            let b = self.far_shadow_bind.lock().unwrap();
            (b.0, b.1.clone())
        };
        let fog_gen = fog_gen.wrapping_add(far_gen << 32);
        let far_view = far_bind.as_ref().unwrap_or(&self.far_shadow_stand_in);
        if t.scene.as_ref().is_none_or(|(d, h, _)| d != dt || h != ht) || t.lit_fog_gen != fog_gen {
            t.lit_fog_gen = fog_gen;
            let (fog_view, fog_state) = match &fog_bind {
                Some((v, b)) => (v, b),
                None => (&self.lit_fog_stand_in.0, &self.lit_fog_stand_in.1),
            };
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
                        wgpu::BindGroupEntry { binding: 22, resource: wgpu::BindingResource::TextureView(far_view) }, // (stage L4)
                        wgpu::BindGroupEntry { binding: 23, resource: wgpu::BindingResource::TextureView(&self.clouds.vol_detail_view) },
                        wgpu::BindGroupEntry { binding: 24, resource: wgpu::BindingResource::TextureView(&self.clouds.vol_weather_view) },
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
                        wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(fog_view) },
                        wgpu::BindGroupEntry { binding: 11, resource: fog_state.as_entire_binding() },
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
            // (`--photo-finish`'s measured air reaches these passes through the medium: `update`'s `atmo_m`.)
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
                if o.volumetric_clouds && o.clouds {
                    self.clouds.encode_refl(&mut pass);
                }
            }
        }
        // the lit fog hook, between the volume and the resolve.
        mid(enc, true);
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
        // (`--screen-reflect`) the history's mip chain, read next frame (sky_reflect.rs)
        if let (Some(r), Some(p), true) = (&self.reflect, &t.pyr, o.screen_reflect) {
            r.encode(&self.device, &self.uniform, enc, p, parity, look.ao_z_view().as_ref()); // + the trace's Hi-Z from this frame's occlusion depth
        }
        // Bloom.
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("sky-bloom"),
                timestamp_writes: tw(Q_SKY_BLOOM).map(|(q, b)| wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(b), end_of_pass_write_index: Some(b + 1) }),
            });
            if frame_bloom {
                let bgs = &t.bloom_bgs[parity];
                let levels = t._mips.len(); // (`--cine-camera`: `CAM_BLOOM_LEVELS`)
                let sizes: Vec<(u32, u32)> = (0..levels).map(|i| ((t.size.0 >> (i + 1)).max(1), (t.size.1 >> (i + 1)).max(1))).collect();
                pass.set_pipeline(&self.bloom_pipes[0]);
                pass.set_bind_group(0, &bgs[0], &[]);
                pass.dispatch_workgroups(sizes[0].0.div_ceil(8), sizes[0].1.div_ceil(8), 1);
                pass.set_pipeline(&self.bloom_pipes[1]);
                for i in 1..levels {
                    pass.set_bind_group(0, &bgs[i], &[]);
                    pass.dispatch_workgroups(sizes[i].0.div_ceil(8), sizes[i].1.div_ceil(8), 1);
                }
                pass.set_pipeline(&self.bloom_pipes[2]);
                for (k, i) in (0..levels - 1).rev().enumerate() {
                    pass.set_bind_group(0, &bgs[levels + k], &[]);
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
        // the preset carries the world clock's game day, so a clear day's sky
        // (eo-sky `clouds::sky_state`) is drawn afresh each game day in live play
        WeatherMode::Preset(k) => eo_sky::weather::preset_at(k, world_s),
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

/// `--smooth-time`: `sun_glint` with the rig's unenveloped key taken between
/// the two rows with the sun up either side of the hour, linear in the hour,
/// instead of from the nearest one (which stepped at each row's midpoint).
pub fn sun_glint_smooth(hour: f64, beam: f64) -> [f32; 4] {
    use eo_look::rig;
    let alt = eo_look::sun::sun_direction(hour).altitude_deg;
    let a = alt.max(-1.5);
    let app = alt + 1.0 / ((a + 7.31 / (a + 4.4)).to_radians()).tan() / 60.0;
    if app <= -0.27 {
        return [0.0; 4];
    }
    // the rows with the sun up (as `sun_glint`'s), the nearest before and after the hour.
    let (mut before, mut after): (Option<(f64, f64, [f64; 3])>, Option<(f64, f64, [f64; 3])>) = (None, None);
    for r in rig::table() {
        let s = eo_look::sun::sun_direction(r.hour).altitude_deg;
        let e = rig::key_envelope(s) * rig::direct_beam_transmittance(s);
        if e <= 0.02 {
            continue;
        }
        let row = (r.hour, r.sun_intensity / e, r.sun_color);
        if r.hour <= hour && before.is_none_or(|b| r.hour > b.0) {
            before = Some(row);
        }
        if r.hour >= hour && after.is_none_or(|b| r.hour < b.0) {
            after = Some(row);
        }
    }
    let best = match (before, after) {
        (Some(b), Some(n)) if n.0 > b.0 => {
            let t = (hour - b.0) / (n.0 - b.0);
            (b.1 + (n.1 - b.1) * t, [0, 1, 2].map(|i| b.2[i] + (n.2[i] - b.2[i]) * t))
        }
        (Some(b), _) => (b.1, b.2),
        (None, Some(n)) => (n.1, n.2),
        (None, None) => (0.0, [1.0, 1.0, 1.0]),
    };
    let e0 = app.max(0.0);
    let am = 1.0 / (e0.to_radians().sin() + 0.50572 * (e0 + 6.07995).powf(-1.6364));
    let t = 0.7f64.powf(am.powf(0.678)) * smoothstep_f(-0.27, 0.27, app);
    let ray = [5.802e-3, 13.558e-3, 33.1e-3];
    let g = (-ray[1] * 8.0 * am).exp();
    let c = [0, 1, 2].map(|i| best.1[i] * (-ray[i] * 8.0 * am).exp() / g.max(1e-9));
    let k = best.0 * t * beam;
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

/// `--smooth-time`: the sun altitudes (degrees) between which the clouds hand
/// their light over from the sun to the moon's key (`SkyUniform::sun_rad.w`).
/// Without the option they switch when the moon takes the ground's key, about
/// 0.2 deg before the sun sets, while every deck is still sunlit through the
/// atmosphere (a step on every cloud). The Earth's shadow stands about
/// R (sec(depression) - 1) over the ground: 1 km at -1 deg, 9 km at -3 deg,
/// 16 km at -4 deg, so by -4 deg no deck sees the sun.
pub const CLOUD_MOON_HANDOVER_DEG: [f64; 2] = [-1.0, -4.0];

/// `--smooth-time`: a weather change recuts the histories only when its severity
/// or wetness moved this many `weather_key` steps (1/50 each, so 0.1) between two
/// updates, or its kind changed: a preset switched, not a schedule drifting.
pub const WEATHER_RECUT_STEPS: i64 = 5;

/// (`--look cinematic`, stage L1) the lamp glare's parameters in the sky
/// uniform's `bloom_w` (sky_fx.wgsl `fx_glare`): its energy share, its longest
/// ray in pixels of a frame `out_h` lines high, the EV (threshold-bright pixels'
/// worth of light, log2) it starts at, and minus its EV range (negative: the
/// look's mark; the default's weights there are positive).
pub fn glare_params(out_h: u32) -> [f32; 4] {
    use eo_look::cinematic::grade as g;
    [g::CINE_GLARE_SHARE as f32, (g::CINE_GLARE_LENGTH_PX * out_h.max(1) as f64 / 1080.0) as f32, g::CINE_GLARE_EV[0] as f32, -((g::CINE_GLARE_EV[1] - g::CINE_GLARE_EV[0]) as f32)]
}

pub fn weather_key(w: &eo_sky::WeatherState) -> (i64, i64, i64) {
    ((w.severity * 50.0).round() as i64, (w.wetness * 50.0).round() as i64, w.kind as i64)
}

#[cfg(test)]
mod lamp_map_tests {
    use super::*;

    #[test]
    fn the_shaders_lamp_map_constants_are_the_bakes() {
        let src = include_str!("look_light.wgsl");
        for (k, v) in [("LK_LAMP_MAP_SIZE", LAMP_MAP_SIZE as f32), ("LK_LAMP_MAP_TEXEL_M", LAMP_MAP_TEXEL_M), ("LK_LAMP_MAP_ORIGIN_M", LAMP_MAP_ORIGIN_M)] {
            assert!(src.contains(&format!("const {k}: f32 = {v:?};")), "{k} is not {v}");
        }
    }

    #[test]
    fn a_lamps_pool_lands_in_the_map_at_its_foot() {
        let h = 8.84;
        let l = LampLight { pos: [100.0, h, -40.0], candela: eo_sky::photometry::nadir_candela_for_pool(h), color: [1.0; 3], range: h * 2.6, dir: [0.0, -1.0, 0.0], cos_outer: 75f64.to_radians().cos(), cos_inner: 60f64.to_radians().cos(), kind: LampKind::Street };
        let map = bake_lamp_map(&[l]);
        let n = LAMP_MAP_SIZE as usize;
        let at = |x: f32, z: f32| map[((z - LAMP_MAP_ORIGIN_M) / LAMP_MAP_TEXEL_M) as usize * n + ((x - LAMP_MAP_ORIGIN_M) / LAMP_MAP_TEXEL_M) as usize];
        let foot = at(100.0, -40.0);
        assert!(foot > 20.0 && foot < 40.0, "the pool's nadir near the RP-8 peak (30 lx): {foot}");
        assert!(at(116.0, -40.0) > 2.0 && at(116.0, -40.0) < foot, "16 m out lit, less: {}", at(116.0, -40.0));
        assert_eq!(at(160.0, -40.0), 0.0, "past the reach, dark");
    }
}
