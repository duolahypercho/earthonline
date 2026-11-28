//! the static street props (signals, signs, trees, tactile pads, and
//! (props2) the street-furniture kinds, the ez-tree trees and the street
//! overhead) as an INSTANCED class of the GPU-driven path.
//!
//! The library (`props_lib.rs`, `props_lib2.rs`: meshes with up to three
//! levels each, their materials and texture arrays) is written once.
//! Instances (48 bytes: position, yaw, scale, model, variant, lamp channel,
//! slope) live in one arena; the app streams them per 128 m street tile
//! (planned on worker threads by eo-props, written through the budgeted
//! upload queue, `upload::Target::PropInstances`). Nothing is done per
//! instance on the CPU.
//!
//! Per frame (`props_cull.wgsl`):
//! 1. before the city pass, after the look's and the agents' casters: one
//!    compute pass classifies every slot for the sun cascades the look
//!    refreshed (and for the camera when occlusion is off), with the model's
//!    distance level and caster cascades; prefix sums give one compacted
//!    command list per (view, part) with its count; each refreshed cascade
//!    draws its casters with two count-driven multi-draws (opaque,
//!    alpha-tested);
//! 2. inside the occlusion pass, after the city's pyramid: the camera view,
//!    each instance's box tested with the city's own `hzb_occluded`, and
//!    (props2) the wire segments;
//! 3. in the late city pass (the single pass with occlusion off), before the
//!    the camera lists (opaque, alpha-tested) and (props2) the
//!    cross-fade pair (views 4 and 5, dithered), lit by `look_light` with the
//!    cascaded sun shadow; the AO pass reads their depth like any other;
//! 4. (props2) after the late city pass, a pass of its own (so it can be
//!    timed): the wires, one indirect draw, alpha-blended, depth-tested.
//! Signal and pedestrian lamps read a small per-frame lamp-channel array
//! (`write_lamps`) the app fills from the simulation's signal controllers.
//!
//! props2 lamp lights: the district street lights' luminaires
//! (`eo_props::furniture::LampLight`) of the resident tiles are kept in a
//! storage buffer here (`lights_buffer`, `GpuLampLight`, 64 bytes each, the
//! count in `light_count_buffer`), written per tile through the upload queue
//! (`Target::PropLights`). Nothing reads them yet: they are for the night lighting.

use crate::camera::{Camera, frustum_planes};
use crate::look::LookGpu;
use crate::props_lib::PropLibrary;
use crate::{DEPTH_FORMAT, DRAW_ARGS_BYTES, GpuContext, IDS_FORMAT};
use anyhow::{Result, anyhow};
use bytemuck::{Pod, Zeroable};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use wgpu::util::DeviceExt;

/// Instance slots in the arena (sized once at start; props2 measured at most
/// ~40 k resident on the bench paths, see the README).
pub const INSTANCE_CAPACITY: u32 = 131_072;
/// Lamp channels (signal and pedestrian heads resident at once).
pub const LAMP_CAPACITY: u32 = 16_384;
/// 0 camera, 1..3 cascades, 4..5 the camera's cross-fade pair.
pub const VIEWS: u64 = 6;
/// Timestamps this module writes per frame (props2): cascade cull begin/end,
/// casters begin/end, wires begin/end.
pub const QUERIES_PER_FRAME: u32 = 6;
/// wire segment slots.
pub const WIRE_CAPACITY: u32 = 196_608;
/// lamp light slots.
pub const LIGHT_CAPACITY: u32 = 16_384;
/// (`--look cinematic`, stage L2; with `--no-signal-light` only: the lenses as lights
/// are their own luminance, props_draw.wgsl `pr_sl_vertex`) A lit signal lens at full night over its
/// tone-solved radiance (the default's ~0.6 of the display's white, under the
/// look's bloom threshold): 3.5 x puts the lens's core over the threshold
/// (eo-look `CINE_BLOOM_THRESHOLD_X` 0.9), so a signal bleeds into the bloom
/// as the reference's red and green heads do (sheet 7.1, 7.2); the hue-keeping
/// shoulder holds its colour. Ramped in with the lamps (`night::lamps_on`).
pub const CINE_SIGNAL_LENS_GAIN: f32 = 3.5;
/// props2 feature bits (PrParams.mode.z).
pub const F_FADE: u32 = 1;
pub const F_WIND: u32 = 2;
pub const F_LEAF: u32 = 4;
pub const F_DOMES: u32 = 8;
pub const F_SIGNS: u32 = 16;
pub const F_WEATHER_WIND: u32 = 32;
/// Moored boats ride the harbour's chop (props_types.wgsl `pr_afloat`; eo-app `--no-boat-motion`).
pub const F_AFLOAT: u32 = 64;
/// (eo-app `--no-retro-viewer` clears it) An arm Street Name sign's sheeting returns light only
/// to a lamp at the viewer (retroreflection), and the pedestrian and free cameras carry none:
/// its face takes no glow of its own at night, only the street's lights.
pub const F_RETRO_VIEWER: u32 = 128;
/// (eo-app `--no-sign-back-aged` clears it) An arm Street Name sign's back is weathered mill
/// aluminium: dull and rough, not the bright metal it was.
pub const F_SIGN_BACK_AGED: u32 = 256;
pub const F_ALL: u32 = 31 | F_AFLOAT | F_RETRO_VIEWER | F_SIGN_BACK_AGED;
/// A slot no instance uses (its model is out of range).
pub const EMPTY: GpuPropInstance = GpuPropInstance { pos: [0.0; 3], yaw: 0.0, scale: [0.0; 3], model: u32::MAX, variant: 0, lamp: 0, slope: 0.0, pad: 0 };

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable, PartialEq)]
pub struct GpuPropInstance {
    pub pos: [f32; 3],
    pub yaw: f32,
    pub scale: [f32; 3],
    pub model: u32,
    pub variant: u32,
    pub lamp: u32,
    pub slope: f32,
    pub pad: u32,
}
const _: () = assert!(std::mem::size_of::<GpuPropInstance>() == 48);

/// one wire segment (32 bytes): end a, true radius, end b, kind; radius 0 = empty slot.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable, PartialEq)]
pub struct GpuWireSeg {
    pub a: [f32; 3],
    pub radius: f32,
    pub b: [f32; 3],
    pub kind: u32,
}
const _: () = assert!(std::mem::size_of::<GpuWireSeg>() == 32);

/// one luminaire (64 bytes), `eo_props::furniture::LampLight` as the GPU will read it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable, PartialEq)]
pub struct GpuLampLight {
    /// The luminaire (`streetLampHead`); the spot source sits `source_lift` above it.
    pub pos: [f32; 3],
    /// three.js `SpotLight.distance` (m).
    pub range: f32,
    /// `LAMP_PRACTICALS.color`, linear RGB.
    pub colour: [f32; 3],
    /// Candela at full `lampsOn` (0: empty slot).
    pub intensity: f32,
    /// Cone half-angle (rad), penumbra fraction, decay, source lift (m).
    pub angle: f32,
    pub penumbra: f32,
    pub decay: f32,
    pub source_lift: f32,
    /// `eo_props::Model` of the lamp (LampHistoric / LampBanner / LampCobra).
    pub kind: u32,
    pub pad: [u32; 3],
}
const _: () = assert!(std::mem::size_of::<GpuLampLight>() == 64);

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PrParams {
    planes: [[f32; 4]; 6],
    cam: [f32; 4],
    counts: [u32; 4],
    mode: [u32; 4],
    view_proj: [[f32; 4]; 4],
    lens: [f32; 4],
    dome: [f32; 4],
    wind: [f32; 4],
    wire: [f32; 4],
    extra: [u32; 4],
}

/// draw.wgsl's `Frame`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PrFrame {
    view_proj: [[f32; 4]; 4],
    cam_pos: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    ambient_sky: [f32; 4],
    ambient_ground: [f32; 4],
    copy_step: [f32; 4],
}

// Street props use the lean props base shader (ambient plane, near cascades, no unused city/house/room/landmark modules).
fn prop_draw_source() -> String {
    format!("{}\n{}\n{}", crate::look::props_city_base_source(), pr_types_src(),
        signal_light_select(&crate::look::sf_select(&crate::props_yards::draw_wgsl())))
}

#[test]
fn prop_shader_validates_each_combination_using_tree_wind_weight() {
    use crate::look::{ShaderFeatures, AAA_DETAIL, NATURE_CROWN, NATURE_LEAF_SHADE};
    for nature in [0, NATURE_CROWN, NATURE_LEAF_SHADE, NATURE_CROWN | NATURE_LEAF_SHADE] {
        for aaa_buildings in [0, AAA_DETAIL] {
            let features = ShaderFeatures { nature, aaa_buildings, ..ShaderFeatures::NONE };
            let source = crate::look::with_shader_features(features, prop_draw_source);
            wgpu::naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| {
                panic!("nature={nature} detail={aaa_buildings}: {}", e.emit_to_string(&source))
            });
        }
    }
}

/// props_types.wgsl with the vegetation's range and thinning ahead of it
/// (`props_lib2::vegetation_models::thin_wgsl`): every shader that composes
/// props_types.wgsl takes it from here (light_shadow.rs's props variant too).
pub(crate) fn pr_types_src() -> String {
    let boats = crate::props_lib2::BOAT_BASE..crate::props_lib2::SIGN_MODEL_BASE;
    format!(
        "{}\n{}\nconst PR_BOAT_START: u32 = {}u;\nconst PR_BOAT_END: u32 = {}u;\n// the rear-yard fence (props_lib2.rs `YARD_FENCE`, `--yard-detail`): sheared along its length (local +Z) by its grade.\nconst PR_YARD_FENCE: u32 = {}u;\n// the yards' objects and the houses' backs (props_lib2.rs `YARD_ITEM_BASE` .. the end of `YARD_BACK_BASE`'s).\nconst PR_YARD_ITEM_BASE: u32 = {}u;\nconst PR_YARD_ITEM_END: u32 = {}u;\n{}",
        crate::props_lib2::vegetation_models::thin_wgsl(),
        signal_light_wgsl(),
        boats.start,
        boats.end,
        crate::props_lib2::YARD_FENCE,
        crate::props_lib2::YARD_ITEM_BASE,
        crate::props_lib2::YARD_BACK_BASE + crate::props_lib2::yard_backs::COUNT,
        include_str!("props_types.wgsl")
    )
}

// ---- (`--signal-light`, eo-app `--no-signal-light`) the signal lenses as lights ----
//
// Off, a lit lens is the display value it was solved to (`SIGNAL_LENS_TONE_INPUT`): the
// same disc of paint by day and night, from any side. On (props_draw.wgsl `pr_sl_*`):
// - a lit lens or pedestrian symbol is its luminance (eo-sky `SIGNAL_CD` over the lens,
//   `PED_SIGNAL_NIT`), toward the eye by the module's lobe (`SIGNAL_LOBE`: the head's
//   yaw its axis), in the frame's units (`pr_sl_units`), so the tone curve, the camera
//   response and the bloom take it as they take any light: a saturated disc of LEDs by
//   day; at night, under the camera model's look, a white-hot core in its red halation, and
//   under the default chain (whose bloom, thresholded on luminance, passes over a saturated red)
//   a saturated disc rolled off like a camera's highlights (`SL_HOT`);
// - the glass (the lens's polycarbonate dome, the pedestrian face's cover) reflects the
//   sky through its visor's mouth (`SL_VISOR_*`: what the tunnel hides, the visor's dark
//   inside), the sun's glint and its light on the unlit LED module (a phantom) likewise.

static SIGNAL_LIGHT: AtomicBool = AtomicBool::new(true);

/// Whether the props draw module made after this call shades the signal lenses as
/// lights (default true; eo-app `--no-signal-light` sets false: the lenses as before).
pub fn set_signal_light(on: bool) {
    SIGNAL_LIGHT.store(on, Ordering::Relaxed);
}

pub fn signal_light() -> bool {
    SIGNAL_LIGHT.load(Ordering::Relaxed)
}

/// (`--signal-light`) The vehicle head's lens and tunnel visor (street-signals-v1 provenance,
/// `signalHead`: `lensRadiusM`, `lensFaceZ` the dome's apex, `lensDomeSagM`, `visorInsideRadiusM`,
/// `visorDoorZM`, `visorMouthZM`, `visorCutMouthOfRadius`; tests/signal_models.rs holds them to it):
/// a ray from the lens (its dome's mid surface) leaves through the visor's mouth or its cut-away
/// underside, or meets the visor's inside.
pub const SL_LENS_RADIUS_M: f32 = 0.1455;
pub const SL_LENS_APEX_Z_M: f32 = 0.023;
pub const SL_LENS_DOME_SAG_M: f32 = 0.0272;
pub const SL_VISOR_RADIUS_M: f32 = 0.147;
pub const SL_VISOR_DOOR_Z_M: f32 = -0.0338;
pub const SL_VISOR_MOUTH_Z_M: f32 = 0.2613;
pub const SL_VISOR_CUT_MOUTH: f32 = 0.5;
/// The lens's plane for the visor's tests: its dome's mid surface (m).
pub const SL_LENS_PLANE_Z_M: f32 = SL_LENS_APEX_Z_M - 0.5 * SL_LENS_DOME_SAG_M;
/// (`--signal-light`) The pedestrian head's hood in the face's half sizes (pedHead-v1.glb,
/// measured: the face 0.474 x 0.406 m, the hood's mouth 0.219 m in front of it, its inside
/// at the face's edge): depth over the half width and over the half height.
pub const SL_HOOD_DEPTH: [f32; 2] = [0.219 / 0.237, 0.219 / 0.203];
/// (`--signal-light`) The share of the lights' diffuse light (the sun's where the glass does not
/// see it, the lamps') the glass behind a visor or hood still gets (the mouth's view factor from
/// the lens, ~0.3 for a tunnel 1.7 radii deep, more with its underside cut away, the visor's
/// dark inside returning little; the sky's own is SL_VIEW_SAMPLES's). Modelling value.
pub const SL_VISOR_DIFFUSE: f32 = 0.4;
/// (`--signal-light`) The directions the glass's own view of the sky and the ground through
/// its visor or hood is taken over (props_draw.wgsl `pr_sl_view_irradiance`, in the vertex
/// stage): the unlit module's light, the visor's view factor at each point of the lens (a
/// lens's lower half under the cut-away sees the street; its top, under the hood, little),
/// in place of SL_VISOR_DIFFUSE for the sky's light. 16 hold the dome's gradient smooth.
pub const SL_VIEW_SAMPLES: u32 = 16;
/// (`--signal-light`) The glass's roughness: weathered polycarbonate (UV-hazed and dusty,
/// not new; a new cover is ~0.05). Modelling value.
pub const SL_GLASS_ROUGH: f32 = 0.14;
/// (`--signal-light`) The glass's reflection: look_light's own irradiance about the reflected ray
/// over pi (sky and ground, cosine-weighted) in its colour, at the analytic sky's luminance along
/// the ray (`lk_sky_sample`) held to this many times its own: the analytic sky alone made a
/// pedestrian face across the street a bright teal sheet, where in the street it sees facades.
/// Modelling value.
pub const SL_SKY_BOUND: f32 = 2.5;
/// (`--signal-light`) The street's skyline in the glass's reflection: the sines of the elevations
/// between which the analytic sky takes over from the facades (SF's three- to six-storey fronts
/// across 20-30 m streets stand 10-35 deg over a head's height), and the facades' share of the
/// rig's light (their albedo, ~0.35), in the glass's reflection and in the unlit module's own view
/// through its visor (props_draw.wgsl `pr_sl_street`). With the open horizon a far lens reflected
/// the sky's bright horizon and took its light (milky ochre and teal discs at 30-50 m, ~6 x the
/// black backplate beside them, where "Diridon Station by bike" (San Jose, 2025) shows the unlit
/// modules at 0.6-0.75 of it), and a pedestrian face was a grey sheet. Modelling values.
pub const SL_STREET: [f32; 3] = [0.17, 0.57, 0.35];
/// (`--signal-light`) The share of its axis intensity a module still sends wide of its lobe
/// (its emitters' own wide beams and the lens's scatter: photographs of SF heads from below
/// and beside still show the LEDs hot), times the cosine to its axis. Modelling value: the
/// ITE tables are minima and stop at 27.5 deg (~0.07 of the axis).
pub const SL_WIDE_SHARE: f32 = 0.3;
/// (`--signal-light`) A pedestrian module's: its symbols behind a diffusing face read from the
/// whole crosswalk and from under the head (photographs of SF countdown heads). Modelling value.
pub const SL_PED_WIDE_SHARE: f32 = 0.6;
/// (`--signal-light`) A lit source as a camera renders one far over its white (props_draw.wgsl
/// `pr_sl_hot`): its least channels lifted toward `[0]` x f of its brightest (a sensor's
/// crosstalk; f the LED module's local brightness over its mean, at most 4, so its centre and,
/// near, its LEDs clip pale first) from the display's white to `[1]` stops over it, its luminance
/// kept; its brightest channel rolled off from `[2] - 1` to `[2]` stops over the white, its hue
/// kept. Photographs of lit heads show a red lens a saturated pink-red: at night ~(247, 56, 125)
/// sRGB (Commons: "Red traffic light and no-walk crosswalk signal at night", 2024; "Green-over-red
/// stoplight at night (2013)"), by day at street distance ~(231, 42, 72) and (176, 42, 58)
/// ("Stevens Creek Blvd traffic light", "Market St at Fourth St in San Francisco ... (2011)").
/// Through the default display chain the standard red is (255, 51, 74) 1 stop over the white,
/// (255, 104, 107) at 2 and white-pink past that; without the roll-off a red at night (~10 stops
/// over) went white and the frame's resolve (YCoCg clip) fringed it in green. Modelling values
/// fitted to those photographs. Not under the camera model's look (`--cine-camera`, the game's
/// play mode): its response whitens a clipped source by its own crosstalk and its threshold-free
/// bloom spreads it, so it takes the lens's whole radiance (a white-hot core in a red halation,
/// as the night photographs show); rolled off there, the lens lost both.
pub const SL_HOT: [f32; 3] = [0.01, 2.0, 1.0];

// ---- (`--signal-chroma`, eo-app `--no-signal-chroma`) the lit lens's light on its visor's inside, held ----
//
// The visor's inside is the brightest part of a lit head seen from below (the cut-away shows its
// inside lit by its lens), and its light was taken whole while its lens was held (`SL_HOT`):
// - under the default chain the visor (its red 2.3-2.9 stops over the display white at a stop
//   line at 21 h; EO_AO_DEBUG=10's radiance) outshone its own lens (held 1 stop over) and the
//   display's path to white turned it a salmon pink, (254, 144, 151) sRGB against the lens's
//   (255, 95, 108): now the red's is held as its lens is (`pr_sl_hot`; the amber's held so went a
//   saturated orange, purity 0.70, where the photographs show it pale, 0.21-0.39: it is left whole);
// - under the camera model's look (`--cine-camera`) the curve's inset (AgX: 4.2 % of each channel
//   into the others) whitens a colour with nothing in its other channels from ~3.5 stops over
//   middle grey, where a camera's colour matrix cancels its sensor's crosstalk and keeps a narrow-
//   band source's colour until that crosstalk clips: the signal red's visor (~5.3 stops over grey
//   after the meter) read (248, 149, 180) where the photographs show it a saturated magenta-red,
//   (249, 67, 123) ("Red traffic light and no-walk crosswalk signal at night", 2024, from under
//   the head). A narrow-band colour (its second channel under `SL_VISOR_HOLD[2]` of its first:
//   the red's 0.05) is held under `SL_VISOR_HOLD[0]` stops over grey at the hour's exposure
//   (look.key.w; the meter adds ~1 stop at night), rolled off over its last `[1]`; the amber and the
//   green carry their own second channel (0.23 of the amber's red, 0.34 of the green's green) and
//   whiten under the curve as the same series' photographs show them (pale cream, pale cyan), so
//   they are left whole. The lens itself stays whole there (its white core and its bloom);
// - the lenses' colours inside their ITE boxes (eo-sky `SIGNAL_XY`, the standards table's
//   `signal-lens-chromaticity`): the green a blue-green (505 nm) where it was a yellow-green
//   outside its box; the red and yellow move by under 1 %.
//   Modelling values fitted to those photographs.

static SIGNAL_CHROMA: AtomicBool = AtomicBool::new(true);

/// Whether the props draw module made after this call holds the lit lens's light on its visor
/// (default true; eo-app `--no-signal-chroma` sets false: the visor's light whole, as before).
pub fn set_signal_chroma(on: bool) {
    SIGNAL_CHROMA.store(on, Ordering::Relaxed);
}

pub fn signal_chroma() -> bool {
    SIGNAL_CHROMA.load(Ordering::Relaxed)
}

/// (`--signal-chroma`) A lit lens's colour (0 red, 1 yellow, 2 green): inside its ITE box
/// (eo-sky `SIGNAL_XY`), or with the switch off the chromaticities before (`SIGNAL_XY_BEFORE`).
pub fn signal_color(aspect: usize) -> [f64; 3] {
    if signal_chroma() { eo_sky::lights::signal_color(aspect) } else { eo_sky::lights::signal_color_before(aspect) }
}

/// (`--signal-chroma`) Under the camera model's look: the hold's top in stops over middle grey at
/// the hour's exposure, the roll-off's width in stops under it, and the second channel's share of
/// the first under which a colour is held (fully under it, not at all from twice it).
pub const SL_VISOR_HOLD: [f32; 3] = [3.4, 1.0, 0.1];

// ---- (`--signal-hold-all`, eo-app `--no-signal-hold-all`) the amber's and the green's visor held too ----
//
// Round 4 held the red's visor only (`SL_VISOR_HOLD[2]`'s narrow band): under the camera model's look
// the amber and the green, taken whole, whitened past their photographs at night (purity of the
// lit module's pixels 0.11 and 0.13 from 2 m under the head, 0.12 and 0.21 at a stop line, where
// "Yellow traffic light ... at night" and "Green traffic light ... at night" (Ewing NJ 2024, from
// under the head; Kailua-Kona 2021, at street distance) show 0.21-0.39 and 0.35-0.47). Each colour's
// visor is held now at its own top (`SL_VISOR_HOLD_TOP`, fitted to those photographs: amber 0.25 /
// 0.30, green 0.33 at a stop line, the most the hold gives: held harder the visor falls under the
// lens's white core, 0.28 at 0.8 stops; from under the head the green's lens, wide in the cut-away,
// keeps the module paler than its photograph). The default chain keeps the red's only: its amber and green
// matched (0.40, 0.44). Modelling values.

static SIGNAL_HOLD_ALL: AtomicBool = AtomicBool::new(true);

/// Whether the props draw module made after this call holds every colour's visor light under the
/// camera model's look (default true; eo-app `--no-signal-hold-all` sets false: the red's only).
pub fn set_signal_hold_all(on: bool) {
    SIGNAL_HOLD_ALL.store(on, Ordering::Relaxed);
}

pub fn signal_hold_all() -> bool {
    SIGNAL_HOLD_ALL.load(Ordering::Relaxed)
}

/// (`--signal-hold-all`) Under the camera model's look, each colour's hold top in stops over middle
/// grey at the hour's exposure: red (`SL_VISOR_HOLD[0]`), amber, green.
pub const SL_VISOR_HOLD_TOP: [f32; 3] = [SL_VISOR_HOLD[0], 3.0, 1.5];

fn signal_chroma_wgsl() -> String {
    format!(
        "// (`--signal-chroma`) the lit lens's light on its visor, held (props.rs `SL_VISOR_HOLD`)\nconst PR_SL_CHROMA: bool = {};\nconst PR_SL_VISOR_HOLD: vec3<f32> = vec3<f32>({:.2}, {:.2}, {:.3});\n// (`--signal-hold-all`) every colour's visor held under the camera model's look, at its own top\nconst PR_SL_HOLD_ALL: bool = {};\nconst PR_SL_VISOR_TOP: vec3<f32> = vec3<f32>({:.2}, {:.2}, {:.2});\n// (`--signal-unlit-look`) the default chain's unlit module over the camera model's (`SL_UNLIT_SHARE_LOOK`)\nconst PR_SL_UNLIT_LOOK: f32 = {:.4};\n",
        signal_chroma(),
        SL_VISOR_HOLD[0],
        SL_VISOR_HOLD[1],
        SL_VISOR_HOLD[2],
        signal_hold_all(),
        SL_VISOR_HOLD_TOP[0],
        SL_VISOR_HOLD_TOP[1],
        SL_VISOR_HOLD_TOP[2],
        signal_unlit_look_ratio()
    )
}
/// (`--signal-light`) The LED module's emitters: a hex array this many across the lens (the
/// 12 in modules in photographs of SF heads show 12-14), each a hot dot of this radius (in
/// pitches) over a dim fill (the lenslets' scatter), and the module brighter at its centre
/// (1.3 - 0.6 r^2, mean 1 over the disc). Modelling values.
pub const SL_LEDS_ACROSS: f32 = 14.0;
pub const SL_LED_DOT: f32 = 0.22;
pub const SL_LED_FILL: f32 = 0.06;
/// (`--signal-light`) The vehicle head's lens centres (signalHead-v1.glb provenance
/// `lensCentres`: red 0.35 m over the middle lens, green 0.35 m under it).
pub const SL_LENS_PITCH_M: f32 = 0.35;
/// (`--signal-light`) The unlit LED module's albedo, red, amber, green (linear): the tinted
/// polycarbonate over the LED array, so sunlight and skylight come back through the tint (dark
/// ochre, dark teal). Fitted to photographs of unlit modules in daylight (Commons: "Chinatown
/// San Francisco" (2014): an unlit amber (0.111, 0.073, 0.055) linear, ~1.2 x the black housing
/// beside it, a green (0.021, 0.103, 0.075); "LED traffic light on red" (2009, no visors, in sun):
/// the unlit modules ~0.04 of the sky's luminance, olive and teal): a filter passed twice over
/// an LED board of ~0.5 reflectance (amber passes red and green, green blue-green). Modelling values.
pub const SL_UNLIT_ALBEDO: [[f32; 3]; 3] = [[0.30, 0.02, 0.02], [0.32, 0.19, 0.06], [0.02, 0.25, 0.18]];

// ---- (`--signal-unlit`, eo-app `--no-signal-unlit`) the unlit module refitted ----
//
// `SL_UNLIT_ALBEDO` was fitted when the module took a flat share of the sky's light
// (`SL_VISOR_DIFFUSE`); since it takes its own view through its visor (`SL_VIEW_SAMPLES`, the
// street's fronts at low elevations) that albedo times that light was ~83 % of an unlit lens
// (its gloss's reflection ~12 %, the lights' diffuse ~5 %: each term dropped in turn at 30 m,
// 15 h, play look), and an unlit amber read 4.8-5.0 x the black housing beside it at 30 m and
// 0.36 of the sky, where "Diridon Station by bike" (San Jose, 2025) shows unlit modules at
// 0.6-0.75 of their backplate and "LED traffic light on red" (2009, no visors, in sun) at ~0.04
// of the sky. At this share of it: 0.77 x the housing at 30 m, 0.059 of the sky, 0.79 x the
// backplate at 3 m ("Chinatown San Francisco" (2014) shows ~1.2 near, its lens in the sun).
// Modelling value fitted to those photographs.

static SIGNAL_UNLIT: AtomicBool = AtomicBool::new(true);

/// Whether the props draw module made after this call takes the refitted unlit module (default
/// true; eo-app `--no-signal-unlit` sets false: `SL_UNLIT_ALBEDO` whole, as before).
pub fn set_signal_unlit(on: bool) {
    SIGNAL_UNLIT.store(on, Ordering::Relaxed);
}

pub fn signal_unlit() -> bool {
    SIGNAL_UNLIT.load(Ordering::Relaxed)
}

/// (`--signal-unlit`) The share of `SL_UNLIT_ALBEDO` the unlit module takes.
pub const SL_UNLIT_SHARE: f32 = 0.1;

// ---- (`--signal-unlit-look`, eo-app `--no-signal-unlit-look`) the unlit module fitted per look ----
//
// One share (`SL_UNLIT_SHARE`) fitted the camera model's look (0.75 x the housing at 30 m by day) and
// left the default chain's unlit modules at 0.22, near black (its tone puts the dark end lower). Each
// look takes its own now, to "Diridon Station by bike" (San Jose, 2025: 0.6-0.75 of the backplate),
// against the chalked housing (build-street-signals-v1.py `poly`): 0.64 under the default chain, 0.67
// under the camera model (amber, 30 m, 15 h). Modelling values.

static SIGNAL_UNLIT_LOOK: AtomicBool = AtomicBool::new(true);

/// Whether the props draw module made after this call fits the unlit module to each look (default
/// true; eo-app `--no-signal-unlit-look` sets false: `SL_UNLIT_SHARE` in both, as before).
pub fn set_signal_unlit_look(on: bool) {
    SIGNAL_UNLIT_LOOK.store(on, Ordering::Relaxed);
}

pub fn signal_unlit_look() -> bool {
    SIGNAL_UNLIT_LOOK.load(Ordering::Relaxed)
}

/// (`--signal-unlit-look`) The unlit module's share of `SL_UNLIT_ALBEDO` per look: the default
/// chain, the camera model's.
pub const SL_UNLIT_SHARE_LOOK: [f32; 2] = [0.3, 0.12];

/// The unlit module's share in `PR_SL_UNLIT`: the camera model's look's (`--signal-unlit-look`),
/// `SL_UNLIT_SHARE` without it, all of the albedo with `--no-signal-unlit`.
fn signal_unlit_share() -> f32 {
    if !signal_unlit() { 1.0 } else if signal_unlit_look() { SL_UNLIT_SHARE_LOOK[1] } else { SL_UNLIT_SHARE }
}

/// The default chain's share over the camera model's (1 with either switch off).
fn signal_unlit_look_ratio() -> f32 {
    if signal_unlit() && signal_unlit_look() { SL_UNLIT_SHARE_LOOK[0] / SL_UNLIT_SHARE_LOOK[1] } else { 1.0 }
}
/// The mean luminance of the lens image's unlit cells over the module (r < 0.93; red, amber,
/// green; linear): the shader takes the cell's pattern (LEDs, lenslets, rings) over it.
pub const SL_UNLIT_CELL_LUM: [f32; 3] = [0.01872, 0.04267, 0.03213];
/// (`--signal-light`) The alpha a lens's, a pedestrian face's or a lit visor's pixel writes in the
/// scene's HDR target (nothing else reads the city pass's alpha), so the screen-space bounce
/// (surfaces_ao.wgsl) takes its light no brighter than `SL_AO_CAP` of the display's white.
pub const SL_AO_MARK: f32 = 0.125;
/// (`--signal-light`) The marked pixels' light in the screen-space bounce: at most this share of
/// the display's white (the lens drawn as before reached the walls round it at about this; at the
/// white a lit visor still tinted them pink). Modelling value.
pub const SL_AO_CAP: f32 = 0.25;

// ---- (`--signal-bounce`, eo-app `--no-signal-bounce`) the marked pixels' bounce, held ----
//
// The screen-space bounce gathers the lens's and the lit visor's pixels (`SL_AO_MARK`) as lights at
// up to `SL_AO_CAP` of the display's white, onto whatever stands near them in the frame: its own
// head's backplate and housing first, which the visor's tunnel and the housing hide from the lens.
// At night a lit red tinted the backplate beside it (0.134 linear red under the camera model from
// 2 m below at 21 h; without that bounce 0.087; the photographs show the plate black round the
// lens, "Red traffic light and no-walk crosswalk signal at night", 2024). Its light on what stands
// in its beam is the spill's (`--signal-spill`). Modelling value.
pub const SL_AO_CAP_HELD: f32 = 0.03;

static SIGNAL_BOUNCE: AtomicBool = AtomicBool::new(true);

/// Whether the occlusion pass made after this call takes the marked pixels under `SL_AO_CAP_HELD`
/// (default true; eo-app `--no-signal-bounce` sets false: `SL_AO_CAP`, as before).
pub fn set_signal_bounce(on: bool) {
    SIGNAL_BOUNCE.store(on, Ordering::Relaxed);
}

pub fn signal_bounce() -> bool {
    SIGNAL_BOUNCE.load(Ordering::Relaxed)
}

/// (`--signal-light`) props_draw.wgsl with the signal lenses' code for the switch's state: the
/// `// sl:on+` blocks (the lenses and faces as lights) with it on, the `// sl:off+` blocks (the
/// lenses as before) with it off, the markers dropped. Only one of the two reaches the compiler
/// (the props pipelines stand at the edge of what Apple's compiler takes; off, the shader is
/// the one before the feature).
pub(crate) fn signal_light_select(src: &str) -> String {
    let on = signal_light();
    let mut keep = true;
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        match line.trim() {
            "// sl:on+" => keep = on,
            "// sl:off+" => keep = !on,
            "// sl:end" => keep = true,
            _ if keep => {
                out.push_str(line);
                out.push('\n');
            }
            _ => {}
        }
    }
    out
}

/// (`--signal-light`) The occlusion pass's constants (look.rs `ao_shader_source`): the marked
/// pixels' alpha and whether to look for it (off, the pass is as before).
pub fn signal_light_ao_wgsl() -> String {
    format!("\n// generated by props.rs signal_light_ao_wgsl (`--signal-light`)\nconst SL_AO_BOUND: bool = {};\nconst SL_AO_MARK: f32 = {:.4};\nconst SL_AO_CAP: f32 = {:.3};\n", signal_light(), SL_AO_MARK, if signal_bounce() { SL_AO_CAP_HELD } else { SL_AO_CAP })
}

/// (`--signal-light`) The constants props_draw.wgsl's `pr_sl_*` read (`PR_SIGNAL_LIGHT` false:
/// the lenses as before; the constants are declared either way so the module compiles).
pub fn signal_light_wgsl() -> String {
    let v3 = |c: [f64; 3]| format!("vec3<f32>({:.6}, {:.6}, {:.6})", c[0], c[1], c[2]);
    let v3f = |c: [f32; 3]| format!("vec3<f32>({:.5}, {:.5}, {:.5})", c[0], c[1], c[2]);
    let p = eo_sky::lights::PED_SIGNAL_NIT;
    format!(
        "\n// generated by props.rs signal_light_wgsl (`--signal-light`)\nconst PR_SIGNAL_LIGHT: bool = {};\n\
         // radiance units per cd/m2 by day (eo-look phys: one unit of irradiance is lux_per_unit lux)\n\
         const PR_SL_DAY_UNITS: f32 = {:.6e};\n\
         const PR_SL_PED_NIT: vec2<f32> = vec2<f32>({:.1}, {:.1});\n\
         const PR_SL_PED_RGB = array<vec3<f32>, 2>({}, {});\n\
         // the visor (lens radii): inside radius, mouth and door from the lens, the cut's chord at the mouth (share of the radius)\n\
         const PR_SL_VISOR: vec4<f32> = vec4<f32>({:.5}, {:.5}, {:.5}, {:.4});\n\
         const PR_SL_HOOD: vec2<f32> = vec2<f32>({:.5}, {:.5});\n\
         const PR_SL_VISOR_DIFFUSE: f32 = {:.3};\nconst PR_SL_GLASS_ROUGH: f32 = {:.3};\nconst PR_SL_WIDE_SHARE: f32 = {:.4};\nconst PR_SL_VF_N: u32 = {}u;\n\
         const PR_SL_LEDS: vec3<f32> = vec3<f32>({:.2}, {:.3}, {:.3});\n// the head's lens pitch, its lenses' plane and its visor's inside radius (m)\nconst PR_SL_LENS: vec3<f32> = vec3<f32>({:.4}, {:.4}, {:.4});\n\
         const PR_SL_PED_WIDE_SHARE: f32 = {:.3};\nconst PR_SL_HOT: vec3<f32> = vec3<f32>({:.3}, {:.2}, {:.2});\nconst PR_SL_SKY_BOUND: f32 = {:.2};\nconst PR_SL_STREET: vec3<f32> = vec3<f32>({:.3}, {:.3}, {:.3});\n\
         const PR_SL_UNLIT = array<vec3<f32>, 3>({}, {}, {});\nconst PR_SL_UNLIT_LUM: vec3<f32> = vec3<f32>({:.5}, {:.5}, {:.5});\nconst PR_SL_AO_MARK: f32 = {:.4};\n",
        signal_light(),
        1.0 / eo_look::phys::lux_per_unit(),
        p[0],
        p[1],
        v3(eo_sky::lights::ped_signal_color(0)),
        v3(eo_sky::lights::ped_signal_color(1)),
        SL_VISOR_RADIUS_M / SL_LENS_RADIUS_M,
        (SL_VISOR_MOUTH_Z_M - SL_LENS_PLANE_Z_M) / SL_LENS_RADIUS_M,
        (SL_VISOR_DOOR_Z_M - SL_LENS_PLANE_Z_M) / SL_LENS_RADIUS_M,
        SL_VISOR_CUT_MOUTH,
        SL_HOOD_DEPTH[0],
        SL_HOOD_DEPTH[1],
        SL_VISOR_DIFFUSE,
        SL_GLASS_ROUGH,
        SL_WIDE_SHARE,
        SL_VIEW_SAMPLES,
        SL_LEDS_ACROSS,
        SL_LED_DOT,
        SL_LED_FILL,
        SL_LENS_PITCH_M,
        SL_LENS_PLANE_Z_M,
        SL_VISOR_RADIUS_M,
        SL_PED_WIDE_SHARE,
        SL_HOT[0],
        SL_HOT[1],
        SL_HOT[2],
        SL_SKY_BOUND,
        SL_STREET[0],
        SL_STREET[1],
        SL_STREET[2],
        v3f(SL_UNLIT_ALBEDO[0].map(|v| v * signal_unlit_share())),
        v3f(SL_UNLIT_ALBEDO[1].map(|v| v * signal_unlit_share())),
        v3f(SL_UNLIT_ALBEDO[2].map(|v| v * signal_unlit_share())),
        SL_UNLIT_CELL_LUM[0],
        SL_UNLIT_CELL_LUM[1],
        SL_UNLIT_CELL_LUM[2],
        SL_AO_MARK
    ) + &signal_chroma_wgsl()
}

/// What building a `PropsGpu` needs from the renderer (clones: wgpu handles are shared).
#[derive(Clone)]
pub struct PropsInit {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub main_bgl: wgpu::BindGroupLayout,
    pub shadow_views: wgpu::Buffer,
    pub late_bgl: wgpu::BindGroupLayout,
    pub sample_count: u32,
    pub debug_ids: bool,
}

/// the wire constants of `STREET_OVERHEAD` (street-overhead.js).
#[derive(Clone, Copy, Debug)]
pub struct WireLook {
    pub pixel_half_width_rad: f32,
    pub fade_near_m: f32,
    pub fade_far_m: f32,
    pub min_coverage: f32,
}

impl Default for WireLook {
    fn default() -> Self {
        // street-overhead.js STREET_OVERHEAD: pixelHalfWidthRad, fadeNearM, fadeFarM, minCoverage.
        Self { pixel_half_width_rad: 0.0014, fade_near_m: 140.0, fade_far_m: 205.0, min_coverage: 0.7 }
    }
}

pub struct PropsGpu {
    enabled: AtomicBool,
    active: AtomicBool,
    camera_late: AtomicBool,
    high_water: AtomicU32,
    wire_high_water: AtomicU32,
    wires_active: AtomicBool,
    features: AtomicU32,
    /// The model whose instances lie on the ground's plane (`set_grade_model`; u32::MAX: none).
    grade_model: AtomicU32,
    time_bits: AtomicU32,
    pub mesh_count: u32,
    pub model_count: u32,
    indirect_count: bool,
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    instances: wgpu::Buffer,
    lamps: wgpu::Buffer,
    params_s: wgpu::Buffer,
    params_c: wgpu::Buffer,
    frame: wgpu::Buffer,
    counts: wgpu::Buffer,
    cmds: wgpu::Buffer,
    list_counts: wgpu::Buffer,
    /// The 12 compacted lists in the count emulation (no MULTI_DRAW_INDIRECT_COUNT), registered at attach.
    counted: std::sync::OnceLock<crate::counted::CountedLists>,
    stats: wgpu::Buffer,
    cull_bg_s: wgpu::BindGroup,
    cull_bg_c: wgpu::BindGroup,
    cull_s: [wgpu::ComputePipeline; 3],
    cull_c: [wgpu::ComputePipeline; 3],
    draw_bg: wgpu::BindGroup,
    /// group 0 again with the CC0 scanned sets in bindings 17-20 and 22 (once they are on the
    /// GPU, `bind_cc0`): the tree basins' soil (props_draw.wgsl `pr_basin`).
    draw_bg_cc0: std::sync::OnceLock<wgpu::BindGroup>,
    draw_parts: DrawParts,
    /// the CC0 switches the props' frame carries (`set_cc0_run`; 0 until `bind_cc0`).
    cc0_run: AtomicU32,
    data_bg: wgpu::BindGroup,
    view_bg: wgpu::BindGroup,
    opaque: wgpu::RenderPipeline,
    cutout: wgpu::RenderPipeline,
    opaque_fade: wgpu::RenderPipeline,
    cutout_fade: wgpu::RenderPipeline,
    shadow: wgpu::RenderPipeline,
    shadow_cut: wgpu::RenderPipeline,
    // wires.
    wires: wgpu::Buffer,
    wire_args: wgpu::Buffer,
    wire_cull_s: wgpu::ComputePipeline,
    wire_cull_c: wgpu::ComputePipeline,
    wire_cull_bg_s: wgpu::BindGroup,
    wire_cull_bg_c: wgpu::BindGroup,
    wire_draw_bg: wgpu::BindGroup,
    wire_pipe: wgpu::RenderPipeline,
    wire_look: WireLook,
    // lamp lights for the night lighting (kept, not read yet).
    lights: wgpu::Buffer,
    /// (lighting) the buffers and group 2 the lamp and headlight shadow maps' prop casters read.
    pub(crate) casters: crate::light_shadow::Casters,
    light_count: wgpu::Buffer,
    timing: Mutex<Option<(wgpu::QuerySet, u32)>>,
    alpha_test: f32,
    lens_scale: Mutex<f32>,
    face_count: u32,
    dome: [f32; 4],
    /// GPU bytes allocated here.
    pub bytes: u64,
}

fn texture_array(d: &wgpu::Device, q: &wgpu::Queue, label: &str, format: wgpu::TextureFormat, res: u32, layers: &[&[u8]], keep_alpha_coverage: Option<(usize, f32)>) -> (wgpu::Texture, u64) {
    let mips = res.ilog2() + 1;
    let n = layers.len().max(1) as u32;
    let t = d.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: res, height: res, depth_or_array_layers: n },
        mip_level_count: mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let srgb = format == wgpu::TextureFormat::Rgba8UnormSrgb;
    for (layer, data) in layers.iter().enumerate() {
        let mut chain = crate::authored::mip_chain(data, res as usize, srgb);
        // alpha-tested foliage keeps its mip 0 coverage at every mip
        // (else the crowns thin out with distance as the alpha averages down).
        if let Some((l, test)) = keep_alpha_coverage {
            if l == layer {
                preserve_coverage(&mut chain, res as usize, test);
            }
        }
        for (mip, level) in chain.iter().enumerate().take(mips as usize) {
            let r = (res >> mip).max(1);
            q.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &t, mip_level: mip as u32, origin: wgpu::Origin3d { x: 0, y: 0, z: layer as u32 }, aspect: wgpu::TextureAspect::All },
                level,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(r * 4), rows_per_image: Some(r) },
                wgpu::Extent3d { width: r, height: r, depth_or_array_layers: 1 },
            );
        }
    }
    let bytes = (0..mips).map(|m| ((res >> m).max(1) as u64).pow(2) * 4).sum::<u64>() * n as u64;
    (t, bytes)
}

/// scale each mip's alpha so the fraction of texels passing `test` equals mip 0's
/// (the standard coverage-preserving alpha mip fix, bisection on the scale).
fn preserve_coverage(chain: &mut [Vec<u8>], res: usize, test: f32) {
    let cov = |m: &[u8], s: f32| m.chunks_exact(4).filter(|p| (p[3] as f32 / 255.0 * s) >= test).count() as f32 / (m.len() / 4).max(1) as f32;
    let target = cov(&chain[0], 1.0);
    for (k, m) in chain.iter_mut().enumerate().skip(1) {
        if (res >> k) < 4 {
            break;
        }
        let (mut lo, mut hi) = (0.25f32, 4.0f32);
        for _ in 0..12 {
            let mid = (lo + hi) / 2.0;
            if cov(m, mid) < target {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let s = (lo + hi) / 2.0;
        for p in m.chunks_exact_mut(4) {
            p[3] = (p[3] as f32 * s).round().clamp(0.0, 255.0) as u8;
        }
    }
}

/// What the props' group 0 binds, kept so it can be bound again with the CC0 sets.
struct DrawParts {
    layout: wgpu::BindGroupLayout,
    frame: wgpu::Buffer,
    objects: wgpu::Buffer,
    materials: wgpu::Buffer,
    kit: [wgpu::TextureView; 3],
    sampler: wgpu::Sampler,
    probe: wgpu::TextureView,
    probe_sh: wgpu::Buffer,
    /// the CC0 stand-ins: a one-texel array and a small table.
    cc0_tex: wgpu::TextureView,
    cc0_layers: wgpu::Buffer,
}

impl DrawParts {
    fn bind(&self, d: &wgpu::Device, label: &str, cc0: Option<(&[wgpu::TextureView; 3], &wgpu::Sampler, &wgpu::Buffer)>) -> wgpu::BindGroup {
        fn tv(binding: u32, v: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry { binding, resource: wgpu::BindingResource::TextureView(v) }
        }
        let (mat, samp, layers) = match cc0 {
            Some((m, s, l)) => ([&m[0], &m[1], &m[2]], s, l),
            None => ([&self.cc0_tex, &self.cc0_tex, &self.cc0_tex], &self.sampler, &self.cc0_layers),
        };
        d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.frame.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: self.objects.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: self.materials.as_entire_binding() },
                tv(3, &self.kit[0]),
                tv(4, &self.kit[1]),
                tv(5, &self.kit[2]),
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                tv(14, &self.probe),
                wgpu::BindGroupEntry { binding: 15, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 16, resource: self.probe_sh.as_entire_binding() },
                tv(17, mat[0]),
                tv(18, mat[1]),
                tv(19, mat[2]),
                wgpu::BindGroupEntry { binding: 20, resource: wgpu::BindingResource::Sampler(samp) },
                wgpu::BindGroupEntry { binding: 22, resource: layers.as_entire_binding() },
            ],
        })
    }
}

/// The most meshes a props library may hold: props_cull.wgsl `PR_MESH_MAX` (the offsets
/// pass keeps each view's per-mesh counts in one workgroup array of PR_VIEWS x this: 24 KB,
/// over WebGPU's 16 KB floor but under the 32 KB Apple's GPUs give; checked at build).
pub const PROP_MESH_MAX: u32 = 1024;

impl PropsGpu {
    /// Build on any thread (pipelines, buffers, texture mips): `init` from `Renderer::props_init`.
    pub fn new(pi: &PropsInit, lib: &PropLibrary) -> Result<Self> {
        let d = &pi.device;
        let q = &pi.queue;
        let (sample_count, debug_ids, late_bgl) = (pi.sample_count, pi.debug_ids, &pi.late_bgl);
        let mesh_count = lib.meshes.len() as u32;
        if mesh_count == 0 || mesh_count > PROP_MESH_MAX {
            return Err(anyhow!("props library has {mesh_count} meshes (1..={PROP_MESH_MAX} supported)"));
        }
        let wg_bytes = (6 * PROP_MESH_MAX + 6 + 12) * 4; // props_cull.wgsl `pr_wg`, `pr_wg_total` and `pr_wg_tail`
        if d.limits().max_compute_workgroup_storage_size < wg_bytes {
            return Err(anyhow!("the props cull needs {wg_bytes} bytes of compute workgroup storage (props_cull.wgsl `offsets`); the device gives {}", d.limits().max_compute_workgroup_storage_size));
        }
        let indirect_count = d.features().contains(wgpu::Features::MULTI_DRAW_INDIRECT_COUNT);
        use wgpu::BufferUsages as U;
        let init = |label: &str, bytes: &[u8], usage: wgpu::BufferUsages| {
            let contents: Vec<u8> = if bytes.is_empty() { vec![0u8; 16] } else { bytes.to_vec() };
            d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents: &contents, usage })
        };
        let empty = |label: &str, size: u64, usage: wgpu::BufferUsages| d.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size.max(16).next_multiple_of(16), usage, mapped_at_creation: false });
        // (also read as storage by the lamp and headlight shadow maps' casters, light_shadow.rs)
        let vbuf = init("prop-vertices", bytemuck::cast_slice(&lib.vertices), U::VERTEX | U::STORAGE);
        let ibuf = init("prop-indices", bytemuck::cast_slice(&lib.indices), U::INDEX | U::STORAGE);
        let meshes = init("prop-meshes", bytemuck::cast_slice(&lib.meshes), U::STORAGE);
        let models = init("prop-models", bytemuck::cast_slice(&lib.models), U::STORAGE);
        let materials = init("prop-materials", bytemuck::cast_slice(&lib.materials), U::STORAGE);
        let faces = init("prop-faces", bytemuck::cast_slice(&lib.faces), U::STORAGE);
        let lens = init("prop-lens", bytemuck::cast_slice(&lib.lens), U::UNIFORM);
        let objects = init("prop-object", &[0u8; 64], U::STORAGE);
        let n = INSTANCE_CAPACITY as u64;
        let empty_slots = vec![EMPTY; INSTANCE_CAPACITY as usize];
        let instances = init("prop-instances", bytemuck::cast_slice(&empty_slots), U::STORAGE | U::COPY_DST);
        let lamps = empty("prop-lamps", LAMP_CAPACITY as u64 * 4, U::STORAGE | U::COPY_DST);
        let params_s = empty("prop-params-pre", std::mem::size_of::<PrParams>() as u64, U::UNIFORM | U::COPY_DST);
        let params_c = empty("prop-params-camera", std::mem::size_of::<PrParams>() as u64, U::UNIFORM | U::COPY_DST);
        let frame = empty("prop-frame", std::mem::size_of::<PrFrame>() as u64, U::UNIFORM | U::COPY_DST);
        let records = empty("prop-records", n * VIEWS * 4, U::STORAGE);
        let visible = empty("prop-visible", n * VIEWS * 4, U::STORAGE);
        let m = mesh_count as u64;
        let counts = empty("prop-counts", m * VIEWS * 2 * 4, U::STORAGE | U::COPY_DST);
        let cmds = empty("prop-draws", m * VIEWS * 2 * DRAW_ARGS_BYTES, U::STORAGE | U::INDIRECT | U::COPY_SRC);
        let list_counts = empty("prop-list-counts", VIEWS * 2 * 4, U::STORAGE | U::INDIRECT | U::COPY_DST | U::COPY_SRC);
        let stats = empty("prop-stats", 16 * 4, U::STORAGE | U::COPY_DST | U::COPY_SRC);
        // wires and lamp lights.
        let wires = init("prop-wires", bytemuck::cast_slice(&vec![GpuWireSeg::default(); WIRE_CAPACITY as usize]), U::STORAGE | U::COPY_DST);
        let wire_vis = empty("prop-wire-visible", WIRE_CAPACITY as u64 * 4, U::STORAGE);
        let wire_args = empty("prop-wire-args", 32, U::STORAGE | U::INDIRECT | U::COPY_DST | U::COPY_SRC);
        let lights = init("prop-lamp-lights", bytemuck::cast_slice(&vec![GpuLampLight::default(); LIGHT_CAPACITY as usize]), U::STORAGE | U::COPY_DST | U::COPY_SRC);
        let light_count = empty("prop-lamp-light-count", 16, U::STORAGE | U::UNIFORM | U::COPY_DST | U::COPY_SRC);

        let alb: Vec<&[u8]> = lib.layers.iter().map(|l| l[0].as_slice()).collect();
        let nor: Vec<&[u8]> = lib.layers.iter().map(|l| l[1].as_slice()).collect();
        let orm: Vec<&[u8]> = lib.layers.iter().map(|l| l[2].as_slice()).collect();
        let (t_alb, b0) = texture_array(d, q, "prop-albedo", wgpu::TextureFormat::Rgba8UnormSrgb, crate::props_lib::ALBEDO_RES, &alb, None);
        let (t_nor, b1) = texture_array(d, q, "prop-normal", wgpu::TextureFormat::Rgba8Unorm, crate::props_lib::AUX_RES, &nor, None);
        let (t_orm, b2) = texture_array(d, q, "prop-orm", wgpu::TextureFormat::Rgba8Unorm, crate::props_lib::AUX_RES, &orm, None);
        // the big array (vegetation atlas with coverage-preserving mips, sign faces, printed faces).
        let big: Vec<&[u8]> = lib.big_layers.iter().map(|l| l.as_slice()).collect();
        let fallback = crate::props_lib::flat_layer([255; 4], 4);
        let big_ok = !big.is_empty() && big.iter().all(|l| l.len() == (crate::props_lib::BIG_RES as usize).pow(2) * 4);
        let (t_big, b3) = if big_ok {
            texture_array(d, q, "prop-big", wgpu::TextureFormat::Rgba8UnormSrgb, crate::props_lib::BIG_RES, &big, Some((0, lib.alpha_test)))
        } else {
            texture_array(d, q, "prop-big", wgpu::TextureFormat::Rgba8UnormSrgb, 4, &[&fallback, &fallback, &fallback], None)
        };
        let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        let sampler = d.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("prop-sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });

        let entry = |binding, visibility, ty| wgpu::BindGroupLayoutEntry { binding, visibility, ty, count: None };
        let ubuf = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None };
        let sbuf = |read_only| wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None };
        let cs = wgpu::ShaderStages::COMPUTE;
        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let fs = wgpu::ShaderStages::FRAGMENT;
        fn bge(binding: u32, b: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry { binding, resource: b.as_entire_binding() }
        }
        let tex = |binding, vis| {
            entry(binding, vis, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false })
        };

        // Cull (group 0); the Hi-Z variant also binds the city pyramid (group 1).
        let cull_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("prop-cull-bgl"),
            entries: &[
                entry(0, cs, ubuf),
                entry(1, cs, sbuf(true)),
                entry(2, cs, sbuf(true)),
                entry(3, cs, sbuf(true)),
                entry(4, cs, sbuf(false)),
                entry(5, cs, sbuf(false)),
                entry(6, cs, sbuf(false)),
                entry(7, cs, sbuf(false)),
                entry(8, cs, sbuf(false)),
                entry(9, cs, ubuf),
                entry(10, cs, sbuf(false)),
            ],
        });
        let cascades = wgpu::BindGroupEntry {
            binding: 9,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &pi.shadow_views, offset: 0, size: Some(std::num::NonZeroU64::new(768).unwrap()) }),
        };
        let cull_bg = |params: &wgpu::Buffer, label: &str| {
            d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &cull_bgl,
                entries: &[
                    bge(0, params),
                    bge(1, &instances),
                    bge(2, &models),
                    bge(3, &meshes),
                    bge(4, &records),
                    bge(5, &visible),
                    bge(6, &counts),
                    bge(7, &cmds),
                    bge(8, &stats),
                    cascades.clone(),
                    bge(10, &list_counts),
                ],
            })
        };
        let cull_bg_s = cull_bg(&params_s, "prop-cull-bg-pre");
        let cull_bg_c = cull_bg(&params_c, "prop-cull-bg-camera");
        let plain_src = format!("{}\n{}\nfn pr_occluded(mn: vec3<f32>, mx: vec3<f32>) -> bool {{ return false; }}\n", pr_types_src(), include_str!("props_cull.wgsl"));
        let hzb_tail = format!(
            "struct HzbInfo {{ screen: vec4<f32> }};\n@group(1) @binding(0) var hzb: texture_2d<f32>;\n@group(1) @binding(1) var<uniform> hzb_info: HzbInfo;\n{}\nfn pr_occluded(mn: vec3<f32>, mx: vec3<f32>) -> bool {{ return hzb_occluded(mn, mx, pr.view_proj, hzb_info.screen.xy); }}\n",
            crate::occlusion::OCCLUSION_WGSL
        );
        let hzb_src = format!("{}\n{}\n{hzb_tail}", pr_types_src(), include_str!("props_cull.wgsl"));
        let plain_mod = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("props_cull.wgsl"), source: wgpu::ShaderSource::Wgsl(plain_src.into()) });
        let hzb_mod = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("props_cull.wgsl (hzb)"), source: wgpu::ShaderSource::Wgsl(hzb_src.into()) });
        let lay_s = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("prop-cull-layout"), bind_group_layouts: &[Some(&cull_bgl)], immediate_size: 0 });
        let lay_c = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("prop-cull-hzb-layout"), bind_group_layouts: &[Some(&cull_bgl), Some(late_bgl)], immediate_size: 0 });
        let compute = |module: &wgpu::ShaderModule, layout: &wgpu::PipelineLayout, e: &str| {
            d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(e), layout: Some(layout), module, entry_point: Some(e), compilation_options: Default::default(), cache: None })
        };
        let cull_s = [compute(&plain_mod, &lay_s, "classify"), compute(&plain_mod, &lay_s, "offsets"), compute(&plain_mod, &lay_s, "scatter")];
        let cull_c = [compute(&hzb_mod, &lay_c, "classify"), compute(&hzb_mod, &lay_c, "offsets"), compute(&hzb_mod, &lay_c, "scatter")];

        // Props data (group 2 of the colour and shadow pipelines).
        let data_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("prop-data-bgl"),
            entries: &[
                entry(0, vf, ubuf),
                entry(1, vf, sbuf(true)),
                entry(2, vf, sbuf(true)),
                entry(3, vf, sbuf(true)),
                entry(4, vf, sbuf(true)),
                entry(5, vf, ubuf),
                tex(6, vf),
                entry(7, vf, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                entry(8, vf, sbuf(true)),
                tex(9, vf), // the big array
            ],
        });
        let (av, nv, ov, bv) = (view(&t_alb), view(&t_nor), view(&t_orm), view(&t_big));
        let data_bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("prop-data-bg"),
            layout: &data_bgl,
            entries: &[
                bge(0, &params_s),
                bge(1, &instances),
                bge(2, &visible),
                bge(3, &lamps),
                bge(4, &faces),
                bge(5, &lens),
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&av) },
                wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::Sampler(&sampler) },
                bge(8, &materials),
                wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(&bv) },
            ],
        });
        let draw_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("prop-draw-bgl"),
            entries: &[
                entry(0, vf, ubuf),
                entry(1, vf, sbuf(true)),
                // (the materials: the vertex stage reads a signal lens's flags, `--signal-spill`)
                entry(2, vf, sbuf(true)),
                tex(3, fs),
                tex(4, fs),
                tex(5, fs),
                entry(6, fs, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                // A scene-probe slot for a look_light that names one (bindings
                // 14-16 of draw.wgsl's group 0 in trees that have it): a 1 x 1
                // stand-in; the props' frame says no probe.
                tex(14, fs),
                entry(15, fs, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                entry(16, fs, sbuf(true)),
                // the CC0 material sets and their layer table (cc0.wgsl bindings 17-20, 22): the
                // tree basins' soil scan; stand-ins until `bind_cc0`.
                tex(17, fs),
                tex(18, fs),
                tex(19, fs),
                entry(20, fs, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                entry(22, fs, sbuf(true)),
            ],
        });
        let probe_stand_in = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("prop-probe-stand-in"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 6 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let probe_view = view(&probe_stand_in);
        let probe_sh = empty("prop-probe-sh-stand-in", 160, U::STORAGE);
        let cc0_stand_in = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("prop-cc0-stand-in"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let draw_parts = DrawParts {
            layout: draw_bgl.clone(),
            frame: frame.clone(),
            objects: objects.clone(),
            materials: materials.clone(),
            kit: [av.clone(), nv.clone(), ov.clone()],
            sampler: sampler.clone(),
            probe: probe_view,
            probe_sh,
            cc0_tex: view(&cc0_stand_in),
            cc0_layers: empty("prop-cc0-layers-stand-in", 64, U::STORAGE),
        };
        let draw_bg = draw_parts.bind(d, "prop-draw-bg", None);
        let draw_src = prop_draw_source(); // (`--signal-light`: its blocks or the old ones, `signal_light_select`)
        let draw_mod = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("props_draw.wgsl"), source: wgpu::ShaderSource::Wgsl(draw_src.into()) });
        let draw_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("prop-draw-layout"),
            bind_group_layouts: &[Some(&draw_bgl), Some(&pi.main_bgl), Some(&data_bgl)],
            immediate_size: 0,
        });
        let attrs = [
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Snorm8x4, offset: 12, shader_location: 1 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Unorm16x2, offset: 16, shader_location: 2 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Unorm8x4, offset: 20, shader_location: 3 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Uint32, offset: 24, shader_location: 4 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32, offset: 28, shader_location: 5 }, // wind weight
        ];
        let stride = std::mem::size_of::<crate::props_lib::PropVertex>() as u64;
        let make = |label: &str, fs_entry: &str| {
            let color = wgpu::ColorTargetState { format: crate::look::HDR_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL };
            let ids = wgpu::ColorTargetState { format: IDS_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL };
            let targets_ids = [Some(color.clone()), Some(ids)];
            let targets = [Some(color)];
            let entry_name = if debug_ids { format!("{fs_entry}_ids") } else { fs_entry.to_string() };
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&draw_layout),
                vertex: wgpu::VertexState {
                    module: &draw_mod,
                    entry_point: Some("prop_vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout { array_stride: stride, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attrs })],
                },
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Greater),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState { count: sample_count, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &draw_mod,
                    entry_point: Some(&entry_name),
                    compilation_options: Default::default(),
                    targets: if debug_ids { &targets_ids } else { &targets },
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let opaque = make("prop-opaque", "prop_fs");
        let cutout = make("prop-cutout", "prop_fs_cut");
        let opaque_fade = make("prop-opaque-fade", "prop_fs_fade");
        let cutout_fade = make("prop-cutout-fade", "prop_fs_cut_fade");

        // Sun casters.
        let view_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("prop-shadow-view-bgl"),
            entries: &[entry(0, wgpu::ShaderStages::VERTEX, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: None })],
        });
        let view_bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("prop-shadow-view-bg"),
            layout: &view_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &pi.shadow_views, offset: 0, size: Some(std::num::NonZeroU64::new(80).unwrap()) }),
            }],
        });
        let shadow_src = format!("{}\n{}", pr_types_src(), include_str!("props_shadow.wgsl"));
        let shadow_mod = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("props_shadow.wgsl"), source: wgpu::ShaderSource::Wgsl(shadow_src.into()) });
        let shadow_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("prop-shadow-layout"), bind_group_layouts: &[Some(&view_bgl), None, Some(&data_bgl)], immediate_size: 0 });
        let shadow_attrs = [attrs[0], attrs[2], attrs[4], attrs[5]];
        let shadow_pipe = |label: &str, cut: bool| {
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&shadow_layout),
                vertex: wgpu::VertexState {
                    module: &shadow_mod,
                    entry_point: Some("prop_shadow_vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout { array_stride: stride, step_mode: wgpu::VertexStepMode::Vertex, attributes: &shadow_attrs })],
                },
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: crate::look::SHADOW_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: cut.then(|| wgpu::FragmentState { module: &shadow_mod, entry_point: Some("prop_shadow_cut_fs"), compilation_options: Default::default(), targets: &[] }),
                multiview_mask: None,
                cache: None,
            })
        };
        let shadow = shadow_pipe("prop-shadow", false);
        let shadow_cut = shadow_pipe("prop-shadow-cut", true);

        // wires (cull in the props' compute passes, draw in a pass of its own).
        let wsrc = include_str!("props_wires.wgsl");
        let section = |tag: &str| -> String {
            let (open, close) = (format!("#ifdef_{tag}"), format!("#endif_{tag}"));
            let a = wsrc.find(&open).map(|i| i + open.len()).unwrap_or(0);
            let b = wsrc.find(&close).unwrap_or(wsrc.len());
            wsrc[a..b].to_string()
        };
        let head = wsrc.split("#ifdef_cull").next().unwrap_or("");
        let wire_cull_plain = format!("{}\n{head}\n{}\nfn pr_occluded(mn: vec3<f32>, mx: vec3<f32>) -> bool {{ return false; }}\n", pr_types_src(), section("cull"));
        let wire_cull_hzb = format!("{}\n{head}\n{}\n{hzb_tail}", pr_types_src(), section("cull"));
        let wire_draw_src = format!("{}\n{}\n{head}\n{}\n{}", crate::look::look_common_source(), pr_types_src(), section("draw"), crate::look::LOOK_EXT_WGSL); // + group 1 bindings 12-18
        let wire_cull_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("prop-wire-cull-bgl"),
            entries: &[entry(0, cs, ubuf), entry(1, cs, sbuf(true)), entry(2, cs, sbuf(false)), entry(3, cs, sbuf(false))],
        });
        let wire_cull_bg = |p: &wgpu::Buffer, label: &str| {
            d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some(label), layout: &wire_cull_bgl, entries: &[bge(0, p), bge(1, &wires), bge(2, &wire_vis), bge(3, &wire_args)] })
        };
        let wire_cull_bg_s = wire_cull_bg(&params_s, "prop-wire-cull-bg-pre");
        let wire_cull_bg_c = wire_cull_bg(&params_c, "prop-wire-cull-bg-camera");
        let wmod_p = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("props_wires.wgsl (cull)"), source: wgpu::ShaderSource::Wgsl(wire_cull_plain.into()) });
        let wmod_h = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("props_wires.wgsl (cull, hzb)"), source: wgpu::ShaderSource::Wgsl(wire_cull_hzb.into()) });
        let wlay_s = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("prop-wire-cull-layout"), bind_group_layouts: &[Some(&wire_cull_bgl)], immediate_size: 0 });
        let wlay_c = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("prop-wire-cull-hzb-layout"), bind_group_layouts: &[Some(&wire_cull_bgl), Some(late_bgl)], immediate_size: 0 });
        let wire_cull_s = compute(&wmod_p, &wlay_s, "wire_cull");
        let wire_cull_c = compute(&wmod_h, &wlay_c, "wire_cull");
        let wire_draw_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("prop-wire-draw-bgl"),
            entries: &[entry(0, vf, ubuf), entry(1, vf, sbuf(true)), entry(2, vf, sbuf(true))],
        });
        let wire_draw_bg = d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("prop-wire-draw-bg"), layout: &wire_draw_bgl, entries: &[bge(0, &params_c), bge(1, &wires), bge(2, &wire_vis)] });
        let wmod_d = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("props_wires.wgsl (draw)"), source: wgpu::ShaderSource::Wgsl(wire_draw_src.into()) });
        let wlay_d = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("prop-wire-draw-layout"), bind_group_layouts: &[Some(&wire_draw_bgl), Some(&pi.main_bgl)], immediate_size: 0 });
        let wire_pipe = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("prop-wires"),
            layout: Some(&wlay_d),
            vertex: wgpu::VertexState { module: &wmod_d, entry_point: Some("wire_vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState { count: sample_count, ..Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &wmod_d,
                entry_point: Some("wire_fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: crate::look::HDR_FORMAT, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::COLOR })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let bytes = [
            &vbuf, &ibuf, &meshes, &models, &materials, &faces, &lens, &objects, &instances, &lamps, &params_s, &params_c, &frame, &records, &visible, &counts, &cmds, &list_counts, &stats, &wires, &wire_vis, &wire_args, &lights, &light_count,
        ]
        .iter()
        .map(|b| b.size())
        .sum::<u64>()
            + b0
            + b1
            + b2
            + b3;
        // (lighting) what the lamp and headlight shadow maps draw the props with.
        let casters = crate::light_shadow::Casters { vertices: vbuf.clone(), indices: ibuf.clone(), instances: instances.clone(), models: models.clone(), meshes: meshes.clone(), data_bgl: data_bgl.clone(), data_bg: data_bg.clone() };
        Ok(Self {
            enabled: AtomicBool::new(true),
            active: AtomicBool::new(false),
            camera_late: AtomicBool::new(false),
            high_water: AtomicU32::new(0),
            wire_high_water: AtomicU32::new(0),
            wires_active: AtomicBool::new(false),
            features: AtomicU32::new(F_ALL),
            grade_model: AtomicU32::new(u32::MAX),
            time_bits: AtomicU32::new(0),
            mesh_count,
            model_count: lib.models.len() as u32,
            indirect_count,
            vbuf,
            ibuf,
            instances,
            lamps,
            params_s,
            params_c,
            frame,
            counts,
            cmds,
            list_counts,
            counted: std::sync::OnceLock::new(),
            stats,
            cull_bg_s,
            cull_bg_c,
            cull_s,
            cull_c,
            draw_bg,
            draw_bg_cc0: std::sync::OnceLock::new(),
            draw_parts,
            cc0_run: AtomicU32::new(0),
            data_bg,
            view_bg,
            opaque,
            cutout,
            opaque_fade,
            cutout_fade,
            shadow,
            shadow_cut,
            wires,
            wire_args,
            wire_cull_s,
            wire_cull_c,
            wire_cull_bg_s,
            wire_cull_bg_c,
            wire_draw_bg,
            wire_pipe,
            wire_look: WireLook::default(),
            lights,
            casters,
            light_count,
            timing: Mutex::new(None),
            alpha_test: lib.alpha_test,
            lens_scale: Mutex::new(lib.lens[0][3]),
            face_count: lib.face_count,
            dome: lib.dome,
            bytes,
        })
    }

    /// Put the 12 compacted prop draw lists in the count emulation (no MULTI_DRAW_INDIRECT_COUNT).
    /// `props_cull.wgsl` zeroes unused command slots itself and `mesh_count` is small (<= 512), so
    /// no per-frame clear or synchronous recovery stall is needed.
    pub fn register_counts(&self, counts: &std::sync::Arc<crate::counted::Counts>) {
        let m = self.mesh_count as u64;
        let lists: Vec<crate::counted::List> = (0..VIEWS * 2)
            .map(|k| crate::counted::List {
                word: k as u32,
                draws: self.cmds.clone(),
                offset: k * m * DRAW_ARGS_BYTES,
                stride: DRAW_ARGS_BYTES,
                max: self.mesh_count,
                floor: None,
            })
            .collect();
        let opts = crate::counted::EntryOptions { margin: 32, zero_cmds: false, recover_sync: false };
        let _ = self.counted.set(counts.register_with("props", &self.list_counts, lists, opts));
    }

    /// Bind the CC0 material sets (once they are all on the GPU; later calls are ignored).
    pub fn bind_cc0(&self, d: &wgpu::Device, mat: &[wgpu::TextureView; 3], sampler: &wgpu::Sampler, layers: &wgpu::Buffer) {
        if self.draw_bg_cc0.get().is_none() {
            let _ = self.draw_bg_cc0.set(self.draw_parts.bind(d, "prop-draw-bg-cc0", Some((mat, sampler, layers))));
        }
    }

    /// The CC0 switches (`cc0::Cc0Gpu::run_bits`) for the next `update`: none until `bind_cc0`.
    pub fn set_cc0_run(&self, bits: u32) {
        self.cc0_run.store(if self.draw_bg_cc0.get().is_some() { bits } else { 0 }, Ordering::Relaxed);
    }

    /// Props on or off (off: no pass runs, nothing is drawn).
    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// the shader features (`F_*` bits; `--no-props2` clears them all).
    pub fn set_features(&self, f: u32) {
        self.features.store(f, Ordering::Relaxed);
    }

    /// The model whose instances are sheared onto the ground's plane by the gradient in their
    /// `pad` word, as the low vegetation is (props_types.wgsl `pr_veg_grad`; eo-app: the pull
    /// box, `--no-pull-boxes` off); u32::MAX: none.
    pub fn set_grade_model(&self, model: u32) {
        self.grade_model.store(model, Ordering::Relaxed);
    }

    /// the clock the wind runs on (the simulation's, seconds).
    pub fn set_time(&self, seconds: f64) {
        self.time_bits.store((seconds.rem_euclid(3600.0) as f32).to_bits(), Ordering::Relaxed);
    }

    /// the wire constants (`STREET_OVERHEAD`).
    pub fn set_wire_look(&mut self, w: WireLook) {
        self.wire_look = w;
    }

    /// Instance slots in use (the cull runs over `0..n`).
    pub fn set_high_water(&self, n: u32) {
        self.high_water.store(n.min(INSTANCE_CAPACITY), Ordering::Relaxed);
    }

    pub fn high_water(&self) -> u32 {
        self.high_water.load(Ordering::Relaxed)
    }

    /// wire segment slots in use (the wire cull runs over `0..n`).
    pub fn set_wire_high_water(&self, n: u32) {
        self.wire_high_water.store(n.min(WIRE_CAPACITY), Ordering::Relaxed);
    }

    pub fn wire_high_water(&self) -> u32 {
        self.wire_high_water.load(Ordering::Relaxed)
    }

    pub fn instance_buffer(&self) -> &wgpu::Buffer {
        &self.instances
    }

    /// the wire segment arena (`Target::PropWires`).
    pub fn wire_buffer(&self) -> &wgpu::Buffer {
        &self.wires
    }

    /// the lamp lights of the resident tiles (`GpuLampLight` slots; `Target::PropLights`).
    pub fn lights_buffer(&self) -> &wgpu::Buffer {
        &self.lights
    }

    /// `[slots in use (high-water), capacity, 0, 0]` of `lights_buffer`.
    pub fn light_count_buffer(&self) -> &wgpu::Buffer {
        &self.light_count
    }

    /// the lamp-light slots in use (written when it changes).
    pub fn set_light_count(&self, queue: &wgpu::Queue, n: u32) {
        queue.write_buffer(&self.light_count, 0, bytemuck::cast_slice(&[n.min(LIGHT_CAPACITY), LIGHT_CAPACITY, 0, 0]));
    }

    /// This frame's lamp channel states (a few hundred words).
    pub fn write_lamps(&self, queue: &wgpu::Queue, lamps: &[u32]) {
        let n = lamps.len().min(LAMP_CAPACITY as usize);
        if n > 0 {
            queue.write_buffer(&self.lamps, 0, bytemuck::cast_slice(&lamps[..n]));
        }
    }

    pub fn set_timestamps(&self, ts: Option<(wgpu::QuerySet, u32)>) {
        *self.timing.lock().unwrap() = ts;
    }

    pub fn stats_buffer(&self) -> &wgpu::Buffer {
        &self.stats
    }

    /// The camera's uniforms (after the look's update). `occlusion`: whether
    /// this frame's camera view is culled in the occlusion pass (Hi-Z).
    pub fn update(&self, queue: &wgpu::Queue, cam: &Camera, look: &LookGpu, occlusion: bool) {
        let vp = cam.view_proj();
        let planes = frustum_planes(vp).map(|p| p.to_array());
        let lf = look.frame();
        let exposure = lf.exposure.max(1e-6) as f32;
        // (`--look cinematic`, stage L2) at night a lit signal's lens bright enough to bloom (`CINE_SIGNAL_LENS_GAIN`).
        let lens_gain = if look.options.look.is_cinematic() { 1.0 + (CINE_SIGNAL_LENS_GAIN - 1.0) * eo_sky::night::lamps_on(lf.model.sun.altitude_deg) as f32 } else { 1.0 };
        let aces = *self.lens_scale.lock().unwrap() * lens_gain;
        let counts = [self.high_water(), self.mesh_count, INSTANCE_CAPACITY, self.model_count];
        let mut feat = self.features.load(Ordering::Relaxed);
        let mut t = f32::from_bits(self.time_bits.load(Ordering::Relaxed));
        let w = self.wire_look;
        // Default: a light westerly (the bay breeze), amplitude 6 cm at the crown of a 1x tree.
        // Under `--weather-fx`, wind direction, gusting sway amplitude and normalized speed follow
        // the live weather state (gentle bay breeze in clear air, brisk sway in rain, gale in storm).
        let (wind_dir, wind_amp, wind_speed_norm, rain_milli) = if (feat & F_WIND) != 0 && look.sky.options().weather_fx {
            feat |= F_WEATHER_WIND;
            if t == 0.0 {
                t = look.sky.world_time().rem_euclid(3600.0) as f32;
            }
            let wx = look.sky.weather();
            let (d, a, sn) = weather_wind_params(&wx);
            let rm = ((wx.rain_mm_h / 22.0).clamp(0.0, 1.0) * 1000.0).round() as u32;
            (d, a, sn, rm)
        } else {
            ([0.94f32, 0.34], 0.06, 0.0, 0)
        };
        let base = PrParams {
            planes,
            cam: [cam.eye.x, cam.eye.y, cam.eye.z, 0.0],
            counts,
            mode: [0, 0, feat, 0],
            view_proj: vp.to_cols_array_2d(),
            lens: [aces / exposure, self.alpha_test, t, self.face_count as f32],
            dome: self.dome,
            wind: [wind_dir[0], wind_dir[1], wind_amp, w.pixel_half_width_rad],
            wire: [w.fade_near_m, w.fade_far_m, w.min_coverage, wind_speed_norm],
            extra: [self.wire_high_water(), rain_milli, self.grade_model.load(Ordering::Relaxed), 0],
        };
        // Run S: only the cascades refreshed this frame (+ the camera when there is no occlusion pass).
        let cascade_bits = (look.encoded_shadow_mask() & 0b111) << 1;
        let cam_bits = if occlusion { 0 } else { 0b110001 };
        let run_s = cascade_bits | cam_bits;
        let s = PrParams { mode: [run_s, run_s, feat, 0], ..base };
        let c = PrParams { mode: [0b110001, 0b110001, feat, 0], ..base };
        queue.write_buffer(&self.params_s, 0, bytemuck::bytes_of(&s));
        queue.write_buffer(&self.params_c, 0, bytemuck::bytes_of(&c));
        // the wire draw: 6 vertices a slot over every slot (culled slots degenerate,
        // fixed blend order); word 4 counts the segments the cull kept.
        queue.write_buffer(&self.wire_args, 0, bytemuck::cast_slice(&[6u32, self.wire_high_water(), 0, 0, 0, 0, 0, 0]));
        self.camera_late.store(occlusion, Ordering::Relaxed);
        let f = PrFrame {
            view_proj: vp.to_cols_array_2d(),
            cam_pos: cam.eye.extend(1.0).to_array(),
            sun_dir: [0.0, 1.0, 0.0, 0.0],
            sun_color: [1.0, 1.0, 1.0, 0.0],
            ambient_sky: [0.0; 4],
            ambient_ground: [0.0; 4],
            copy_step: [0.0, 0.0, 0.0, self.cc0_run.load(Ordering::Relaxed) as f32],
        };
        queue.write_buffer(&self.frame, 0, bytemuck::bytes_of(&f));
    }

    /// Empty timestamp passes (a query a frame resolves is always written).
    fn empty_ts(enc: &mut wgpu::CommandEncoder, ts: &Option<(wgpu::QuerySet, u32)>, slot: u32) {
        if let Some((q, b)) = ts {
            enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("props idle"),
                timestamp_writes: Some(wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(b + slot), end_of_pass_write_index: Some(b + slot + 1) }),
            });
        }
    }

    /// Before the city pass, after `LookGpu::encode_shadows` (and the agents'
    /// casters): the cascades' cull and their casters.
    pub fn encode_pre(&self, enc: &mut wgpu::CommandEncoder, look: &LookGpu) {
        let n = self.high_water();
        let nw = self.wire_high_water();
        let on = self.enabled() && n > 0;
        let wires_on = self.enabled() && nw > 0;
        self.active.store(on, Ordering::Relaxed);
        self.wires_active.store(wires_on, Ordering::Relaxed);
        let ts = self.timing.lock().unwrap().clone();
        if !on {
            Self::empty_ts(enc, &ts, 0);
            Self::empty_ts(enc, &ts, 2);
            if wires_on && !self.camera_late.load(Ordering::Relaxed) {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("props-wire-cull"), timestamp_writes: None });
                pass.set_pipeline(&self.wire_cull_s);
                pass.set_bind_group(0, &self.wire_cull_bg_s, &[]);
                pass.dispatch_workgroups(nw.div_ceil(64), 1, 1);
            }
            return;
        }
        enc.clear_buffer(&self.counts, 0, None);
        enc.clear_buffer(&self.stats, 0, None);
        let mask = look.encoded_shadow_mask();
        let list: Vec<usize> = (0..3).filter(|c| mask & (1 << c) != 0).collect();
        let cam_late = self.camera_late.load(Ordering::Relaxed);
        if list.is_empty() && cam_late {
            Self::empty_ts(enc, &ts, 0);
        } else {
            let tw = ts.as_ref().map(|(q, b)| wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(*b), end_of_pass_write_index: Some(*b + 1) });
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("props-cull"), timestamp_writes: tw });
            pass.set_bind_group(0, &self.cull_bg_s, &[]);
            let groups = n.div_ceil(64);
            for (k, p) in self.cull_s.iter().enumerate() {
                pass.set_pipeline(p);
                pass.dispatch_workgroups(if k == 1 { 1 } else { groups }, 1, 1);
            }
            // the wires, when the camera is culled here (occlusion off).
            if wires_on && !cam_late {
                pass.set_pipeline(&self.wire_cull_s);
                pass.set_bind_group(0, &self.wire_cull_bg_s, &[]);
                pass.dispatch_workgroups(nw.div_ceil(64), 1, 1);
            }
        }
        if list.is_empty() {
            Self::empty_ts(enc, &ts, 2);
        }
        let layers = look.shadow_layer_views();
        for (k, &c) in list.iter().enumerate() {
            let tw = ts.as_ref().map(|(q, b)| wgpu::RenderPassTimestampWrites {
                query_set: q,
                beginning_of_pass_write_index: (k == 0).then_some(*b + 2),
                end_of_pass_write_index: (k == list.len() - 1).then_some(*b + 3),
            });
            let tw = tw.filter(|t| t.beginning_of_pass_write_index.is_some() || t.end_of_pass_write_index.is_some());
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("props-casters"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &layers[c],
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: tw,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.view_bg, &[(c as u64 * 256) as u32]);
            pass.set_bind_group(2, &self.data_bg, &[]);
            pass.set_vertex_buffer(0, self.vbuf.slice(..));
            pass.set_index_buffer(self.ibuf.slice(..), wgpu::IndexFormat::Uint32);
            let v = 1 + c as u64;
            pass.set_pipeline(&self.shadow);
            self.draw_list(&mut pass, v * 2);
            pass.set_pipeline(&self.shadow_cut);
            self.draw_list(&mut pass, v * 2 + 1);
        }
    }

    /// One compacted command list (props2): count-driven where the device can,
    /// else bounded by the count emulation (Metal).
    fn draw_list(&self, pass: &mut wgpu::RenderPass<'_>, list: u64) {
        let m = self.mesh_count as u64;
        if let Some(c) = self.counted.get() {
            crate::counted::draw_indexed(pass, Some(c), list as usize, &self.cmds, list * m * DRAW_ARGS_BYTES, &self.list_counts, list * 4, self.mesh_count);
        } else if self.indirect_count {
            pass.multi_draw_indexed_indirect_count(&self.cmds, list * m * DRAW_ARGS_BYTES, &self.list_counts, list * 4, self.mesh_count);
        } else {
            pass.multi_draw_indexed_indirect(&self.cmds, list * m * DRAW_ARGS_BYTES, self.mesh_count);
        }
    }

    /// Inside the occlusion compute pass, after the pyramid: the camera view
    /// tested against it (`hzb_bg`: `OcclusionGpu::late_bgl`'s group).
    pub fn cull_camera(&self, pass: &mut wgpu::ComputePass<'_>, hzb_bg: &wgpu::BindGroup) {
        if !self.camera_late.load(Ordering::Relaxed) {
            return;
        }
        if self.active.load(Ordering::Relaxed) {
            let groups = self.high_water().div_ceil(64);
            pass.set_bind_group(0, &self.cull_bg_c, &[]);
            pass.set_bind_group(1, hzb_bg, &[]);
            for (k, p) in self.cull_c.iter().enumerate() {
                pass.set_pipeline(p);
                pass.dispatch_workgroups(if k == 1 { 1 } else { groups }, 1, 1);
            }
        }
        // the wire segments against the same pyramid.
        if self.wires_active.load(Ordering::Relaxed) {
            pass.set_pipeline(&self.wire_cull_c);
            pass.set_bind_group(0, &self.wire_cull_bg_c, &[]);
            pass.set_bind_group(1, hzb_bg, &[]);
            pass.dispatch_workgroups(self.wire_high_water().div_ceil(64), 1, 1);
        }
    }

    /// Inside the (late) city pass, after the opaque city list. Leaves the
    /// props vertex and index buffers bound: the caller binds its own again.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, look: &LookGpu) {
        if !self.active.load(Ordering::Relaxed) {
            return;
        }
        pass.set_bind_group(0, self.draw_bg_cc0.get().unwrap_or(&self.draw_bg), &[]);
        look.bind_main(pass);
        pass.set_bind_group(2, &self.data_bg, &[]);
        pass.set_vertex_buffer(0, self.vbuf.slice(..));
        pass.set_index_buffer(self.ibuf.slice(..), wgpu::IndexFormat::Uint32);
        pass.set_pipeline(&self.opaque);
        self.draw_list(pass, 0);
        pass.set_pipeline(&self.cutout);
        self.draw_list(pass, 1);
        // the cross-fade pair (leaving level 8/9, arriving level 10/11).
        pass.set_pipeline(&self.opaque_fade);
        self.draw_list(pass, 8);
        self.draw_list(pass, 10);
        pass.set_pipeline(&self.cutout_fade);
        self.draw_list(pass, 9);
        self.draw_list(pass, 11);
    }

    /// whether the wire pass will draw this frame (the city pass keeps its depth for it).
    pub fn wires_pending(&self) -> bool {
        self.enabled() && self.wire_high_water() > 0
    }

    /// Draw the wires inside an existing HDR + depth render pass (depth test Greater, no depth
    /// write) to avoid a full-screen TBDR load/store pass on Metal.
    pub fn draw_wires_in_pass(&self, pass: &mut wgpu::RenderPass<'_>, look: &LookGpu) {
        if !self.wires_active.load(Ordering::Relaxed) {
            return;
        }
        pass.set_pipeline(&self.wire_pipe);
        pass.set_bind_group(0, &self.wire_draw_bg, &[]);
        look.bind_main(pass);
        pass.draw_indirect(&self.wire_args, 0);
    }

    /// Write an empty timestamp span for the wire slot when wires were folded into another pass.
    pub fn empty_wires_ts(&self, enc: &mut wgpu::CommandEncoder) {
        let ts = self.timing.lock().unwrap().clone();
        Self::empty_ts(enc, &ts, 4);
    }

    /// the wires, a pass of its own after the late city pass (timed).
    /// `color` / `resolve` / `depth` as the city pass had them.
    pub fn draw_wires(&self, enc: &mut wgpu::CommandEncoder, color: &wgpu::TextureView, resolve: Option<&wgpu::TextureView>, depth: &wgpu::TextureView, look: &LookGpu) {
        let ts = self.timing.lock().unwrap().clone();
        if !self.wires_active.load(Ordering::Relaxed) {
            Self::empty_ts(enc, &ts, 4);
            return;
        }
        let tw = ts.as_ref().map(|(q, b)| wgpu::RenderPassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(*b + 4), end_of_pass_write_index: Some(*b + 5) });
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("props-wires"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: color, depth_slice: None, resolve_target: resolve, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: depth, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }), stencil_ops: None }),
            timestamp_writes: tw,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.wire_pipe);
        pass.set_bind_group(0, &self.wire_draw_bg, &[]);
        look.bind_main(&mut pass);
        pass.draw_indirect(&self.wire_args, 0);
    }

    /// The stats words of the last frame (tests; blocks): per view instances and triangles.
    pub fn stats_readback(&self, ctx: &GpuContext) -> Result<Vec<u32>> {
        let size = self.stats.size();
        let staging = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: Some("prop-stats-readback"), size, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(&self.stats, 0, &staging, 0, size);
        ctx.queue.submit([enc.finish()]);
        Ok(bytemuck::pod_collect_to_vec(&crate::read_mapped(ctx, &staging)?))
    }

    /// the wire draw's instance count of the last frame (tests; blocks).
    pub fn wire_drawn_readback(&self, ctx: &GpuContext) -> Result<u32> {
        let staging = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: Some("prop-wire-readback"), size: 32, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(&self.wire_args, 0, &staging, 0, 32);
        ctx.queue.submit([enc.finish()]);
        let w: Vec<u32> = bytemuck::pod_collect_to_vec(&crate::read_mapped(ctx, &staging)?);
        Ok(w[4])
    }
}

/// Weather-coupled tree wind parameters `(dir_xz, sway_amp_m, speed_norm)` for `--weather-fx`:
/// a gentle bay breeze on clear days, lively gusting sway in rain (`~4.5 m/s`), and strong
/// downwind bend + whip in storms (`~12.2 m/s`).
pub(crate) fn weather_wind_params(wx: &eo_sky::WeatherState) -> ([f32; 2], f32, f32) {
    let wx_speed = wx.wind[0].hypot(wx.wind[1]);
    let (dir, eff_speed) = if wx_speed > 0.25 {
        ([(wx.wind[0] / wx_speed) as f32, (wx.wind[1] / wx_speed) as f32], wx_speed)
    } else {
        ([0.9403761f32, 0.34013605], 1.8)
    };
    let gust = wx.gust.clamp(0.5, 1.6);
    let speed = eff_speed * gust;
    let speed_norm = ((speed - 1.2) / 11.8).clamp(0.0, 1.0) as f32;
    let rain_boost = ((wx.rain_mm_h / 22.0).clamp(0.0, 1.0) as f32) * 0.08;
    let amp = 0.07 + 0.50 * speed_norm.powf(1.15) + rain_boost;
    (dir, amp, speed_norm)
}

#[cfg(test)]
mod wind_tests {
    use super::*;

    #[test]
    fn the_cull_shader_holds_max_meshes() {
        // props_cull.wgsl's workgroup array is sized for PROP_MESH_MAX meshes in each of PR_VIEWS views
        let cull = include_str!("props_cull.wgsl");
        assert!(cull.contains(&format!("const PR_MESH_MAX: u32 = {PROP_MESH_MAX}u;")));
        assert!(include_str!("props_types.wgsl").contains("const PR_VIEWS: u32 = 6u;"));
        assert!(cull.contains(&format!("var<workgroup> pr_wg: array<u32, {}>;", 6 * PROP_MESH_MAX)));
    }

    #[test]
    fn weather_wind_scales_monotonically_from_clear_through_rain_to_storm() {
        let clear = weather_wind_params(&eo_sky::weather::preset(eo_sky::Kind::Clear));
        let drizzle = weather_wind_params(&eo_sky::weather::preset(eo_sky::Kind::Drizzle));
        let light = weather_wind_params(&eo_sky::weather::preset(eo_sky::Kind::LightRain));
        let sun = weather_wind_params(&eo_sky::weather::preset(eo_sky::Kind::SunRain));
        let rain = weather_wind_params(&eo_sky::weather::preset(eo_sky::Kind::Rain));
        let heavy = weather_wind_params(&eo_sky::weather::preset(eo_sky::Kind::HeavyRain));
        let storm = weather_wind_params(&eo_sky::weather::preset(eo_sky::Kind::Storm));
        for (dir, amp, norm) in [clear, drizzle, light, sun, rain, heavy, storm] {
            let len = (dir[0] * dir[0] + dir[1] * dir[1]).sqrt();
            assert!((len - 1.0).abs() < 1e-4, "unit wind dir: {dir:?}");
            assert!(amp.is_finite() && amp > 0.0);
            assert!((0.0..=1.0).contains(&norm));
        }
        assert!(clear.1 < drizzle.1 && drizzle.1 < light.1 && light.1 < sun.1 && sun.1 < rain.1 && rain.1 < heavy.1 && heavy.1 < storm.1);
        assert!(rain.1 >= 0.18 && heavy.1 >= 0.35 && storm.1 >= 0.50, "rain={} heavy={} storm={}", rain.1, heavy.1, storm.1);
    }
}

#[cfg(test)]
mod signal_light_tests {
    use super::*;
    /// The tests that flip the props shader's switches (process-wide atomics) run one at a time.
    fn switch_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// (`--signal-light`) the props shader's constants: the switch's state, the pedestrian
    /// symbols' luminance and colours from eo-sky, the visor measured from the modelled head
    /// (signalHead-v1 provenance: lens radius 0.1455 m), the switch restoring the shader as before.
    #[test]
    fn signal_light_constants_follow_the_sources_and_the_switch() {
        let _switches = switch_lock();
        let on = signal_light_wgsl();
        assert!(on.contains("const PR_SIGNAL_LIGHT: bool = true;"), "on by default");
        assert!(on.contains(&format!("vec2<f32>({:.1}, {:.1})", eo_sky::lights::PED_SIGNAL_NIT[0], eo_sky::lights::PED_SIGNAL_NIT[1])));
        assert!(SL_VISOR_RADIUS_M > SL_LENS_RADIUS_M && SL_VISOR_MOUTH_Z_M > SL_LENS_APEX_Z_M && SL_VISOR_DOOR_Z_M < SL_LENS_PLANE_Z_M);
        assert!(signal_light_ao_wgsl().contains("const SL_AO_BOUND: bool = true;"));
        assert!(SL_HOT[0] > 0.0 && SL_HOT[0] < eo_look::tone::SENSOR_CROSSTALK as f32 && SL_HOT[2] > 0.0);
        set_signal_light(false);
        let off = signal_light_wgsl();
        set_signal_light(true);
        assert!(off.contains("const PR_SIGNAL_LIGHT: bool = false;"));
        // the draw shader holds the lenses' new code with the switch on, the old without
        let draw = crate::look::sf_select(include_str!("props_draw.wgsl"));
        let with = signal_light_select(&draw);
        set_signal_light(false);
        let without = signal_light_select(&draw);
        set_signal_light(true);
        assert!(with.contains("fn pr_sl_vertex(") && with.contains("pr_sl_glass(") && !with.contains("SIGNAL_LENS_TONE_INPUT"));
        assert!(!without.contains("pr_sl_") && without.contains("SIGNAL_LENS_TONE_INPUT") && without.contains("pr_signal_lens(materials[material].flags"));
        assert!(!with.contains("// sl:") && !without.contains("// sl:"), "no marker is left");
    }

    /// (`--signal-chroma`) eo-sky's lens chromaticities inside their boxes in the California street standards table
    /// (`signal-lens-chromaticity`: half-planes a x + b y <= c).
    #[test]
    fn signal_chromaticities_lie_in_their_ite_boxes() {
        let _switches = switch_lock();
        let table = eo_data::standards::CaStandards::bundled().expect("the standards table");
        for (k, key) in ["red", "yellow", "green"].into_iter().enumerate() {
            let planes = table.metric_list(eo_sky::lights::SIGNAL_CHROMATICITY_ID, key).expect("the box");
            assert!(!planes.is_empty() && planes.len() % 3 == 0, "{key}: {planes:?}");
            let [x, y] = eo_sky::lights::SIGNAL_XY[k];
            for p in planes.chunks(3) {
                assert!(p[0] * x + p[1] * y <= p[2] + 1e-9, "{key} ({x}, {y}) outside {p:?}");
            }
        }
        // the green before was outside its box (a yellow-green)
        let g = table.metric_list(eo_sky::lights::SIGNAL_CHROMATICITY_ID, "green").unwrap();
        let [x, y] = eo_sky::lights::SIGNAL_XY_BEFORE[2];
        assert!(g.chunks(3).any(|p| p[0] * x + p[1] * y > p[2]));
    }

    /// (`--signal-chroma`) the visor's hold: on by default, its switch in the props shader's
    /// constants, the props shader parsing either way, and the narrow-band share between the
    /// signal red's second channel and the amber's and the green's (eo-sky's lens colours).
    #[test]
    fn signal_chroma_holds_the_red_only_and_switches() {
        let _switches = switch_lock();
        assert!(signal_light_wgsl().contains("const PR_SL_CHROMA: bool = true;"), "on by default");
        let second = |c: [f64; 3]| {
            let mut v = c;
            v.sort_by(|a, b| b.total_cmp(a));
            (v[1] / v[0]) as f32
        };
        let [red, amber, green] = [0, 1, 2].map(|k| second(eo_sky::lights::signal_color(k)));
        assert!(red < SL_VISOR_HOLD[2], "the red is held: {red}");
        assert!(amber >= 2.0 * SL_VISOR_HOLD[2] && green >= 2.0 * SL_VISOR_HOLD[2], "the amber and green are not: {amber} {green}");
        for on in [true, false] {
            set_signal_chroma(on);
            let src = prop_draw_source();
            set_signal_chroma(true);
            assert!(src.contains(&format!("const PR_SL_CHROMA: bool = {on};")));
            wgpu::naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("signal chroma {on}: {}", e.emit_to_string(&src)));
        }
    }

    /// (`--signal-unlit`) the unlit module's albedo: refitted by default, the switch restoring it whole.
    #[test]
    fn signal_unlit_scales_the_unlit_module_and_switches() {
        let _switches = switch_lock();
        let amber = |w: &str| w.split("const PR_SL_UNLIT = array<vec3<f32>, 3>(").nth(1).and_then(|r| r.split("), vec3").nth(1)).map(str::to_owned).unwrap();
        let on = signal_light_wgsl();
        set_signal_unlit(false);
        let off = signal_light_wgsl();
        set_signal_unlit(true);
        let a = SL_UNLIT_ALBEDO[1];
        let k = SL_UNLIT_SHARE_LOOK[1];
        assert!(amber(&on).starts_with(&format!("<f32>({:.5}, {:.5}, {:.5}", a[0] * k, a[1] * k, a[2] * k)), "{}", amber(&on));
        assert!(amber(&off).starts_with(&format!("<f32>({:.5}, {:.5}, {:.5}", a[0], a[1], a[2])), "{}", amber(&off));
        assert!(SL_UNLIT_SHARE > 0.0 && SL_UNLIT_SHARE < 1.0);
    }

    /// (`--signal-unlit-look`, `--signal-hold-all`, `--signal-bounce`) round 5's fits: each on by
    /// default, each switch restoring round 4's constants, the props shader parsing either way.
    #[test]
    fn round_five_fits_follow_their_switches() {
        let _switches = switch_lock();
        let w = signal_light_wgsl();
        assert!(w.contains(&format!("const PR_SL_UNLIT_LOOK: f32 = {:.4};", SL_UNLIT_SHARE_LOOK[0] / SL_UNLIT_SHARE_LOOK[1])));
        assert!(w.contains("const PR_SL_HOLD_ALL: bool = true;"));
        assert!(w.contains(&format!("const PR_SL_VISOR_TOP: vec3<f32> = vec3<f32>({:.2}, {:.2}, {:.2});", SL_VISOR_HOLD_TOP[0], SL_VISOR_HOLD_TOP[1], SL_VISOR_HOLD_TOP[2])));
        assert!(signal_light_ao_wgsl().contains(&format!("const SL_AO_CAP: f32 = {:.3};", SL_AO_CAP_HELD)));
        assert!(SL_VISOR_HOLD_TOP[0] == SL_VISOR_HOLD[0] && SL_AO_CAP_HELD < SL_AO_CAP);
        set_signal_unlit_look(false);
        set_signal_hold_all(false);
        set_signal_bounce(false);
        let (w4, ao4, src4) = (signal_light_wgsl(), signal_light_ao_wgsl(), prop_draw_source());
        set_signal_unlit_look(true);
        set_signal_hold_all(true);
        set_signal_bounce(true);
        assert!(w4.contains("const PR_SL_UNLIT_LOOK: f32 = 1.0000;") && w4.contains("const PR_SL_HOLD_ALL: bool = false;"));
        let a = SL_UNLIT_ALBEDO[1];
        let amber = w4.split("const PR_SL_UNLIT = array<vec3<f32>, 3>(").nth(1).and_then(|r| r.split("), vec3").nth(1)).unwrap().to_owned();
        assert!(amber.starts_with(&format!("<f32>({:.5}, {:.5}, {:.5}", a[0] * SL_UNLIT_SHARE, a[1] * SL_UNLIT_SHARE, a[2] * SL_UNLIT_SHARE)), "round 4's share: {amber}");
        assert!(ao4.contains(&format!("const SL_AO_CAP: f32 = {:.3};", SL_AO_CAP)));
        wgpu::naga::front::wgsl::parse_str(&src4).unwrap_or_else(|e| panic!("round 4's switches: {}", e.emit_to_string(&src4)));
    }
}
