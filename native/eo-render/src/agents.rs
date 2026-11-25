//! Moving things on the GPU: the simulated cars and people and the player.
//!
//! The agent library (`eo_agents::library::Library`: every traffic body's
//! LODs and every civilian tier in one vertex and one index buffer, a mesh
//! table, a model table, materials, atlases and baked joint matrices) is
//! written once. Per frame the CPU writes the instance array (64 bytes an
//! agent, built off the render thread by `eo_agents::feed`) and two small
//! uniforms; everything else is GPU-driven, like the static city:
//!
//! 1. one compute pass (`agents_cull.wgsl`): frustum cull, the distance tier
//!    per agent with the JS tier caps applied nearest first, a coarser caster
//!    tier per sun cascade, and one `DrawIndexedIndirect` per (view, mesh)
//!    whose instances are the agents in that bucket;
//! 2. the cascades the look refreshed this frame get the agents' casters with
//!    ONE `multi_draw_indexed_indirect` each, after the city's own casters
//!    (so moving things cast sun shadows);
//! 3. in the city pass, after the opaque city list: ONE multi-draw for the
//!    agents' opaque parts (people skinned in the vertex shader from the baked
//!    matrices, wheels spun and steered), lit by the city's `look_light`
//!    (sun with the cascaded shadow, so they also receive shadows), and after
//!    the city's transparent list ONE multi-draw for their glass and lenses.

use crate::camera::{Camera, frustum_planes};
use crate::look::LookGpu;
use crate::{DEPTH_FORMAT, DRAW_ARGS_BYTES, GpuContext, IDS_FORMAT};
use anyhow::{Result, anyhow};
use bytemuck::{Pod, Zeroable};
use eo_agents::library::Library;
use eo_agents::{AgentVertex, GpuAgent, LODS};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use wgpu::util::DeviceExt;

/// Agents the buffers hold.
pub const AGENT_CAPACITY: u32 = 32_768;
/// Views: the camera and three cascades.
pub const VIEWS: u64 = 4;
/// Command lists: camera opaque, camera transparent, cascades 0..2.
pub const LISTS: u64 = 5;
const BINS: u64 = 512;
/// Counter words of the stats buffer: per view (instances, triangles), then
/// agents submitted per class (vehicles, people), pad.
pub const STATS_WORDS: usize = 12;
/// Timestamps this module writes per frame: cull begin/end, casters begin/end.
pub const QUERIES_PER_FRAME: u32 = 4;

/// `--people-shading`: the people's skin, cloth and hair shading
/// (agents_draw.wgsl `pp_person`) compiled into the agents' pipelines made
/// after this is set (eo-app sets it before the library attaches); off, the
/// code is compiled out and the frame is as before.
static PEOPLE_SHADING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_people_shading(on: bool) {
    PEOPLE_SHADING.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn people_shading() -> bool {
    PEOPLE_SHADING.load(std::sync::atomic::Ordering::Relaxed)
}

/// `--people-bounce`: the ground round a person's feet lights them from
/// below (agents_draw.wgsl `pb_person`) in place of the rig's ground lobe,
/// compiled into the agents' pipelines made after this is set; off, compiled
/// out and the frame is as before.
static PEOPLE_BOUNCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_people_bounce(on: bool) {
    PEOPLE_BOUNCE.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn people_bounce() -> bool {
    PEOPLE_BOUNCE.load(std::sync::atomic::Ordering::Relaxed)
}

/// `--people-contact`: the ground round each person occluded by their body
/// (people_contact.wgsl, its own pipeline after the agents' opaque list; the
/// library's contact anchors, eo-agents `people::add_contact_anchors`), made
/// with the agents' pipelines after this is set; off, nothing of it is
/// compiled, made or drawn.
static PEOPLE_CONTACT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_people_contact(on: bool) {
    PEOPLE_CONTACT.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn people_contact() -> bool {
    PEOPLE_CONTACT.load(std::sync::atomic::Ordering::Relaxed)
}

/// `--people-capsules`: the ground round each person darkened by their whole
/// body as capsules, for the sky and for each street lamp and headlamp
/// (people_capsules.wgsl, in people_contact.wgsl's place and pipeline; the
/// library's capsules, eo-agents `people::add_capsule_anchors`), made with
/// the agents' pipelines after this is set; off, nothing of it is compiled,
/// made or drawn and `--people-contact` draws as before.
static PEOPLE_CAPSULES: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_people_capsules(on: bool) {
    PEOPLE_CAPSULES.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn people_capsules() -> bool {
    PEOPLE_CAPSULES.load(std::sync::atomic::Ordering::Relaxed)
}

/// The people's ground pass (its own pipeline after the agents' opaque list):
/// with `--people-capsules` or `--people-contact`.
pub fn people_ground_pass() -> bool {
    people_capsules() || people_contact()
}

/// `--people-lamps`: the street lamps', headlamps' and shop windows' skin,
/// cloth and hair terms on the people (agents_draw.wgsl `pl_person`),
/// compiled into the agents' pipelines made after this is set; off, compiled
/// out and the frame is as before.
static PEOPLE_LAMPS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_people_lamps(on: bool) {
    PEOPLE_LAMPS.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn people_lamps() -> bool {
    PEOPLE_LAMPS.load(std::sync::atomic::Ordering::Relaxed)
}

/// The generated constants agents_draw.wgsl reads (`SF_PEOPLE_SHADING`,
/// `SF_PEOPLE_BOUNCE`, `SF_PEOPLE_LAMPS`).
pub fn people_shading_wgsl() -> String {
    format!("const SF_PEOPLE_SHADING: bool = {};\nconst SF_PEOPLE_BOUNCE: bool = {};\nconst SF_PEOPLE_LAMPS: bool = {};", people_shading(), people_bounce(), people_lamps())
}

/// `--car-panels`: the cars' panels cooked taut (eo-agents
/// `vehicles::filter_panel_normals`) and their badges drawn as pressed chrome
/// emblems (agents_draw.wgsl `ag_car_badge`, role `ROLE_BADGE`), compiled into
/// the agents' pipelines made after this is set (eo-app sets it before the
/// library attaches, and cooks the library with it); off, the code is compiled
/// out, the library is cooked as before and the frame is as before.
static CAR_PANELS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_car_panels(on: bool) {
    CAR_PANELS.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn car_panels() -> bool {
    CAR_PANELS.load(std::sync::atomic::Ordering::Relaxed)
}

/// The vehicles' generated constants agents_draw.wgsl reads: `SF_CAR_PANELS`
/// (`--car-panels`), and (`--car-lamps`) a daytime running lamp's luminance in
/// the rig's radiance units (`DRL_CD` over `DRL_LENS_M2`, eo-sky
/// `photometry::cd_m2`'s scale).
pub fn car_wgsl() -> String {
    let drl = DRL_CD / DRL_LENS_M2 / eo_sky::photometry::lux_per_unit();
    format!("const SF_CAR_PANELS: bool = {};\nconst AG_CAR_DRL_DAY: f32 = {:.6};\n", car_panels() && crate::look::shader_features().car_materials, drl)
}

/// The people's ground occlusion's shader, appended to the agents' colour
/// shader only with `--people-contact` (empty without it); with
/// `--people-capsules` the capsules' in its place.
pub fn people_contact_wgsl() -> &'static str {
    if people_capsules() {
        include_str!("people_capsules.wgsl")
    } else if people_contact() {
        include_str!("people_contact.wgsl")
    } else {
        ""
    }
}

/// (`--car-lamps`) A tail lamp's luminous intensity on its axis, cd: UN ECE R7
/// holds a rear position lamp to 4-17 cd, FMVSS 108 a tail lamp to 2-18 cd;
/// an LED tail lamp sits near the middle.
pub const TAIL_LAMP_CD: f64 = 8.0;
/// (`--car-lamps`) A stop lamp's, cd: UN ECE R7 60-185 cd (S1), FMVSS 108
/// 80-300 cd; on the low side of both, as a single LED lamp.
pub const STOP_LAMP_CD: f64 = 100.0;
/// (`--car-lamps`) The rear lamps' photometric field: a lobe about the body's
/// axis, wider across than up and down, as FMVSS 108 and UN ECE R7 shape a
/// tail or stop lamp's test points (a third of the axis value at 20 degrees
/// left or right; up and down the lamp is made for the following driver's
/// eye, a third to a half of the axis value at 10 degrees: two fifths here,
/// an eighth at 15, so the road a few metres behind the car, 10 to 20 degrees
/// below the lamp, takes only the lobe's edge), over the lens's own diffuse
/// emission into the whole rear half-space (an LED light guide's lens glows
/// from every side it faces). The lobe is
/// exp(-(tan h / tan H)^2 - (tan v / tan V)^2) for the angles h across and v
/// up or down; the floor is this share of the axis value times the cosine off
/// the axis. The floor is what lights the bumper and the street right behind
/// the lamp (a faint glow from the bumper, gone within a metre or two), the
/// lobe the car behind and the wet street's streak; neither has an edge.
/// Degrees, and the floor's share.
pub const REAR_LAMP_SPREAD_H_DEG: f64 = 20.0;
pub const REAR_LAMP_SPREAD_V_DEG: f64 = 10.5;
pub const REAR_LAMP_FLOOR: f64 = 0.06;
/// (`--car-lamps`) The cone the light list's record carries for a reader
/// that shapes it as a spot (none does: every one reads `sf_rear_lamp_shape`):
/// its lobe's reach. Degrees.
pub const REAR_LAMP_FULL_DEG: f64 = 5.0;
pub const REAR_LAMP_EDGE_DEG: f64 = 40.0;
/// (`--car-lamps`) A daytime running lamp's intensity on its axis, cd (UN ECE
/// R87: 400-1200 cd), and its lit lens, m^2 (a 30 x 2 cm LED light guide):
/// a moving car's running lamps by day at their luminance (~1.2e5 cd/m^2, a
/// bright strip in sunlight; agents_draw.wgsl `ag_lamp`).
pub const DRL_CD: f64 = 700.0;
pub const DRL_LENS_M2: f64 = 0.006;

/// (`--car-lamps`) A rear lamp's lit lens, m^2 (a 25 x 5 cm LED light guide).
pub const REAR_LAMP_LENS_M2: f64 = 0.0125;
/// (`--car-lamps`) A rear lamp lights the clusters out to where it gives this
/// much, lux (a sixteenth of the unlit night ground's `NIGHT_UNLIT_GROUND_LUX`).
pub const REAR_LAMP_REACH_LUX: f64 = 0.25;
/// (`--car-lamps`) The rear lamps' kind in the frame's light list (after the
/// shop windows' 4, the lamps' bounce 5 and the night track's signal lenses 6
/// and rear lamps 7, sky.rs `SIGNAL_LIGHT_KIND`, `REAR_LIGHT_KIND`: shaded as a
/// spot of its cone, its lobe widened by its lens). (Round 2 integration: 6 was
/// taken by both tracks; the vehicles track's moved to 8.)
pub const REAR_LAMP_KIND: u32 = 8;

/// (`--car-lamps`) A rear lamp's emission toward the unit direction `d` (from
/// the lamp) over its axis's (`REAR_LAMP_SPREAD_*`, `REAR_LAMP_FLOOR`), as
/// WGSL: `{prefix}_rear_lamp_shape(d, axis)`. The look's modules carry it as
/// `sf_` (look.rs `ShaderFeatures::wgsl`, every module that reads the light
/// list: the clusters, the streaks, the lit fog, the cars' far lights, the
/// contact pass), the gather as `lt_` (its streak pick).
pub fn rear_lamp_shape_fn(prefix: &str) -> String {
    let th = REAR_LAMP_SPREAD_H_DEG.to_radians().tan();
    let tv = REAR_LAMP_SPREAD_V_DEG.to_radians().tan();
    format!(
        "\n// generated by agents.rs rear_lamp_shape_fn (`--car-lamps`): a car's rear lamp's field\nfn {prefix}_rear_lamp_shape(d: vec3<f32>, axis: vec3<f32>) -> f32 {{\n    let z = dot(d, axis);\n    if (z <= 0.0) {{\n        return 0.0;\n    }}\n    let up = normalize(vec3<f32>(0.0, 1.0, 0.0) - axis * axis.y + vec3<f32>(0.0, 1e-4, 0.0));\n    let y = dot(d, up);\n    let x2 = max(1.0 - z * z - y * y, 0.0);\n    let lobe = exp(-(x2 * {:.6} + y * y * {:.6}) / max(z * z, 1e-4));\n    return max(lobe, {:?} * z);\n}}\n",
        1.0 / (th * th),
        1.0 / (tv * tv),
        REAR_LAMP_FLOOR as f32,
    )
}

/// (`--car-lamps`) The look's modules' share of it (look.rs `ShaderFeatures::wgsl`
/// appends it to every set, as the penumbra's constants: declared either way,
/// read only behind `SF_CAR_LAMPS`): the rear lamps' kind and their shape.
pub fn rear_lamp_look_wgsl() -> String {
    format!("const SF_REAR_LAMP_KIND: u32 = {REAR_LAMP_KIND}u;\n{}", rear_lamp_shape_fn("sf"))
}

/// The light gather's constants and code for the cars' rear lamps
/// (`--car-lamps`, agents_lamps.wgsl; sky.rs `cine_lamps_wgsl` appends it):
/// `LT_CAR_LAMPS` false (the switch off) compiles it out.
pub fn car_lamps_wgsl() -> String {
    let on = crate::look::shader_features().car_lamps;
    // the tail lamps' LED red (agents_draw.wgsl AG_CAR_LED_RED) at unit luminance, as the frame's lights carry colour
    let red = [1.0f64, 0.012, 0.004];
    let lum = 0.2126 * red[0] + 0.7152 * red[1] + 0.0722 * red[2];
    format!(
        "\n// generated by agents.rs car_lamps_wgsl (`--car-lamps`)\nconst LT_CAR_LAMPS: bool = {on};\nconst LT_FLAG_BRAKING: u32 = 1u;\nconst LT_TAIL_LAMP_CD: f32 = {TAIL_LAMP_CD:?};\nconst LT_STOP_LAMP_CD: f32 = {STOP_LAMP_CD:?};\nconst LT_LOW_BEAM_PEAK_CD: f32 = {:?};\nconst LT_REAR_RED: vec3<f32> = vec3<f32>({:.6}, {:.6}, {:.6});\nconst LT_REAR_REACH_LUX: f32 = {REAR_LAMP_REACH_LUX:?};\nconst LT_REAR_COS_OUTER: f32 = {:.6};\nconst LT_REAR_COS_INNER: f32 = {:.6};\nconst LT_REAR_KIND: u32 = {REAR_LAMP_KIND}u;\nconst LT_REAR_LENS_M2: f32 = {REAR_LAMP_LENS_M2:?};\nconst LT_REAR_PACK: f32 = {:?};\n// a body without its own lamps: above the rear axle by this, behind it by this (m)\nconst LT_REAR_STAND_IN: vec3<f32> = vec3<f32>(0.0, 0.55, 0.95);\n// the light leaves this far behind the body's tail (m)\nconst LT_REAR_OUT_M: f32 = 0.05;\n// a low beam's lens floor toward the street behind it (sky_light.wgsl SKY_CINE_STREAK_HEAD_LENS)\nconst LT_HEAD_LENS_SHARE: f32 = 0.06;\n{}{}",
        eo_sky::photometry::LOW_BEAM_PEAK_CD,
        red[0] / lum,
        red[1] / lum,
        red[2] / lum,
        REAR_LAMP_EDGE_DEG.to_radians().cos(),
        REAR_LAMP_FULL_DEG.to_radians().cos(),
        eo_agents::vehicles::REAR_LAMP_PACK,
        rear_lamp_shape_fn("lt"),
        include_str!("agents_lamps.wgsl")
    )
}

/// The city's shader source the agents' module is made on: the full set but the
/// single probe's lookups. The sun's penumbra (`--soft-shadows`, look.rs
/// `shadow_wgsl` `LK_SHADOW_SOFT`) is looked up once at the top of the agents'
/// fragment (`LK_SUN_PRE`, agents_draw.wgsl `ag_shade`), not inside look_light's
/// key branch: there, with the cars' materials, Apple's compiler aborted the
/// agents' opaque pipeline (XPC_ERROR_CONNECTION_INTERRUPTED: an LLVM fatal
/// error in MTLCompilerService's instruction emission). r2i2 bisect on the
/// round's switch set: not its size (fewer taps, no loads, no filter all
/// failed); the same lookup hoisted out of that divergent branch builds, and a
/// divergent test put back round it (the sub-pixel skip) failed again, so the
/// agents' module keeps the penumbra's branch uniform. The occluder light
/// (`--occluder-light`): the cars and people take what hides the sky as the
/// city does, but the street's own sky from the volume they read at the point
/// itself (`AMB_OL_STREET` off, ambient.wgsl `amb_sample`), not from the
/// separate reads under it (`amb_sample_ground`), with which the opaque
/// pipeline aborted Apple's compiler with the cars' materials (r2i1, r2i2, its
/// reads hoisted too), and with the walls' or a deck's exitance worked out per
/// pixel in any shape tried (r2i3): what hides the sky is lit by a stand-in set
/// first thing in the fragment (`AMB_OL_PRE`, ambient.wgsl `amb_ol_pre`: the
/// ground lobe x eo-look `occluder::AGENTS_OCCLUDER`), the rest of the model as
/// the city's. Until r2i3 they kept the whole stand-in (the occluder light
/// compiled out: the full ground lobe below them, the sunlit street's bounce
/// under a deck), brighter than a deck's shade round them.
fn agents_city_source() -> String {
    let mut src = crate::look::city_shader_source_variant_probes(crate::ambient::ShaderLevels::Full, crate::look::ShadowLevels::Full, false);
    let f = crate::look::shader_features();
    if f.soft_shadows {
        src = src.replacen("const LK_SUN_PRE: bool = false;", "const LK_SUN_PRE: bool = true;", 1);
    }
    if f.occluder_light {
        src = src.replacen("const AMB_OL_STREET: bool = true;", "const AMB_OL_STREET: bool = false;", 1).replacen("const AMB_OL_PRE: bool = false;", "const AMB_OL_PRE: bool = true;", 1);
    }
    src
}

/// Distance tiers and caps (`HERO_OVERLAY` / the crowd's bands and budget), metres.
#[derive(Clone, Copy, Debug)]
pub struct Tiers {
    pub veh_radius: [f32; 4],
    pub veh_cap: [u32; 4],
    pub ped_radius: [f32; 4],
    pub ped_cap: [u32; 4],
}

impl Tiers {
    /// The JS values: vehicles LOD0 within `nearRadius` (22 m, at most
    /// `nearMax` 2), LOD1 within `radius` (60 m, at most `max` 10), LOD2 (no
    /// cap here: the JS draws the bodies past its 40 overlay slots as the
    /// procedural lofted fleet, which is not ported) to 160 m, the box to 400 m;
    /// hero tier within 6 m (`civilianTierForDistance`), near (skinned
    /// band) within 34 m and the hero + near tiers at most `CROWD_BUDGET.skinned`
    /// 24, mid (instanced band) within 145 m at most 104, far within 240 m at most 320.
    pub fn from_js(js: &eo_agents::catalog::AgentsJs) -> Self {
        let h = js.hero_overlay;
        let c = &js.crowd;
        Tiers {
            veh_radius: [h.near_radius as f32, h.radius as f32, 160.0, 400.0],
            veh_cap: [h.near_max, h.max, u32::MAX / 2, u32::MAX / 2],
            ped_radius: [6.0, c.band_distances.skinned as f32, c.band_distances.instanced as f32, c.band_distances.far as f32],
            ped_cap: [c.budget.skinned, c.budget.skinned, c.budget.instanced, c.budget.far],
        }
    }

    /// The vehicle tiers under `--car-detail LEVEL` (0: as `from_js`, the
    /// browser build's overlay budget of 2 full bodies and 10 LOD 1 bodies).
    /// The JS caps were set for a browser's draw budget; the native renderer
    /// draws the agents with one multi-draw per list, so the nearest cars can
    /// keep their full body further out and more of them at once. Level 1:
    /// LOD 0 within 30 m (at most 6), LOD 1 within 90 m (at most 24), LOD 2 to
    /// 220 m, the far body to 400 m; level 2: 40 m (12), 120 m (40), 260 m,
    /// 400 m. The far slot is the simplified body (eo-agents `far_body`) the
    /// library cooks under the same switch. People are as before.
    pub fn with_car_detail(self, level: u8) -> Self {
        let (veh_radius, near) = match level {
            0 => return self,
            1 => ([30.0, 90.0, 220.0, 400.0], [6, 24]),
            _ => ([40.0, 120.0, 260.0, 400.0], [12, 40]),
        };
        Tiers { veh_radius, veh_cap: [near[0], near[1], u32::MAX / 2, u32::MAX / 2], ..self }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct AgParams {
    planes: [[f32; 4]; 6],
    cam: [f32; 4],
    counts: [u32; 4],
    veh_radius: [f32; 4],
    veh_cap: [u32; 4],
    ped_radius: [f32; 4],
    ped_cap: [u32; 4],
    clip_frames: [u32; 4],
    clip_base: [u32; 4],
    lamp_tail: [f32; 4],
    lamp_head: [f32; 4],
    lamp_misc: [f32; 4],
    /// x the views a cull dispatch serves (bit 0 camera, bits 1..3 cascades).
    misc: [u32; 4],
    /// the camera's view-projection (the pyramid test).
    view_proj: [[f32; 4]; 4],
}

/// draw.wgsl's `Frame`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct AgFrame {
    view_proj: [[f32; 4]; 4],
    cam_pos: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    ambient_sky: [f32; 4],
    ambient_ground: [f32; 4],
    copy_step: [f32; 4],
}

pub struct AgentsGpu {
    enabled: AtomicBool,
    active: AtomicBool,
    count: AtomicU32,
    pub mesh_count: u32,
    pub model_count: u32,
    tiers: Tiers,
    clip_frames: [u32; 4],
    clip_base: [u32; 4],
    lamps: ([f32; 4], [f32; 4], f32),
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    agents: wgpu::Buffer,
    params: wgpu::Buffer,
    frame: wgpu::Buffer,
    counts: wgpu::Buffer,
    hist: wgpu::Buffer,
    cmds: wgpu::Buffer,
    stats: wgpu::Buffer,
    cull_bg: wgpu::BindGroup,
    cull: [wgpu::ComputePipeline; 5],
    /// the camera view's cull after the pyramid (assign, offsets, scatter)
    /// with its own parameters (`params_c`: the camera view only).
    params_c: wgpu::Buffer,
    cull_bg_c: wgpu::BindGroup,
    cull_c: [wgpu::ComputePipeline; 3],
    camera_late: AtomicBool,
    draw_bg: wgpu::BindGroup,
    data_bg: wgpu::BindGroup,
    view_bg: wgpu::BindGroup,
    opaque: wgpu::RenderPipeline,
    glass: wgpu::RenderPipeline,
    /// (`--car-finish`) the people's own opaque and glass pipelines and the
    /// first of their meshes (`person_mesh_split`): the cars' (`opaque`,
    /// `glass`, `AG_PIPE` 1) and the people's (`AG_PIPE` 2) each compiled
    /// without the other's code, the two ranges of the one command list drawn
    /// one after the other. None (the switch off, or a library without people):
    /// `opaque` and `glass` draw every mesh, as before.
    people_pipes: Option<(u32, wgpu::RenderPipeline, wgpu::RenderPipeline)>,
    /// (`--tunnels`) the agents in a tunnel's bore drawn by pipelines of their
    /// own (agents_draw.wgsl `agent_bore_fs`: opaque, glass), made by
    /// `enable_bores` when the world has bores; and whether this frame's
    /// agents hold one (eo_agents `AGENT_IN_BORE`, `upload`). None and false
    /// without bores: nothing more is drawn.
    bore: std::sync::OnceLock<[wgpu::RenderPipeline; 2]>,
    bore_live: AtomicBool,
    pipes: AgentPipes,
    /// the velocity pipeline (single-sample renderers) and the model table the light gather reads.
    velocity: Option<wgpu::RenderPipeline>,
    /// (`--car-contact`) the cars' occlusion of the street round them (agents_contact.rs); None without the switch.
    contact: Option<crate::agents_contact::Contact>,
    models_buf: wgpu::Buffer,
    shadow: wgpu::RenderPipeline,
    /// (`--people-contact`) the people's ground occlusion (None without it).
    people_contact: Option<wgpu::RenderPipeline>,
    timing: Mutex<Option<(wgpu::QuerySet, u32)>>,
    /// (lighting) the buffers and group 2 the lamp and headlight shadow maps' agent casters read.
    pub(crate) casters: crate::light_shadow::Casters,
    /// GPU bytes allocated here.
    pub bytes: u64,
}

/// What the agents' draw pipelines are made of (agent-opaque, agent-glass;
/// and, on demand, the bores' pair).
struct AgentPipes {
    device: wgpu::Device,
    module: wgpu::ShaderModule,
    layout: wgpu::PipelineLayout,
    sample_count: u32,
    debug_ids: bool,
}

impl AgentPipes {
    const ATTRS: [wgpu::VertexAttribute; 6] = [
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Snorm8x4, offset: 12, shader_location: 1 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Unorm16x2, offset: 16, shader_location: 2 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Uint8x4, offset: 20, shader_location: 3 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Unorm8x4, offset: 24, shader_location: 4 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Uint32, offset: 28, shader_location: 5 },
    ];

    /// A camera pipeline over the agent vertices with fragment entry `fs` (`blend`: the glass list's).
    fn make(&self, label: &str, fs: &str, blend: bool) -> wgpu::RenderPipeline {
        self.make_for(label, fs, blend, 0)
    }

    /// `make` for the cars alone (`pipe` 1) or the people alone (2), agents_draw.wgsl
    /// `AG_PIPE` (0: every agent, as `make`).
    fn make_for(&self, label: &str, fs: &str, blend: bool, pipe: u32) -> wgpu::RenderPipeline {
        let d = &self.device;
        let constants = [("AG_PIPE", pipe as f64)];
        let fs_options = wgpu::PipelineCompilationOptions { constants: if pipe == 0 { &[] } else { &constants }, ..Default::default() };
        let stride = std::mem::size_of::<AgentVertex>() as u64;
        let color = wgpu::ColorTargetState { format: crate::look::HDR_FORMAT, blend: blend.then_some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL };
        let ids = wgpu::ColorTargetState { format: IDS_FORMAT, blend: None, write_mask: if blend { wgpu::ColorWrites::empty() } else { wgpu::ColorWrites::ALL } };
        let targets_ids = [Some(color.clone()), Some(ids)];
        let targets = [Some(color)];
        d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&self.layout),
            vertex: wgpu::VertexState {
                module: &self.module,
                entry_point: Some("agent_vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout { array_stride: stride, step_mode: wgpu::VertexStepMode::Vertex, attributes: &Self::ATTRS })],
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(!blend),
                depth_compare: Some(if blend { wgpu::CompareFunction::GreaterEqual } else { wgpu::CompareFunction::Greater }),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState { count: self.sample_count, ..Default::default() },
            fragment: Some(wgpu::FragmentState { module: &self.module, entry_point: Some(fs), compilation_options: fs_options, targets: if self.debug_ids { &targets_ids } else { &targets } }),
            multiview_mask: None,
            cache: None,
        })
    }
}

/// (`--car-finish`) The first person mesh, where every vehicle's meshes lie
/// before it and every person's from it (the library cooks the bodies before
/// the people): the people's pipelines draw the command list from there. None
/// when the library has no people or does not hold them apart.
pub fn person_mesh_split(lib: &Library) -> Option<u32> {
    let meshes = |class: u32| lib.models.iter().filter(move |m| m.class == class).flat_map(|m| m.meshes.iter().copied().filter(|&k| k != eo_agents::NO_MESH));
    let first_person = meshes(eo_agents::CLASS_PERSON).min()?;
    let last_body = meshes(eo_agents::CLASS_VEHICLE).max().unwrap_or(0);
    (last_body < first_person && first_person < lib.meshes.len() as u32).then_some(first_person)
}

impl AgentsGpu {
    /// `late_bgl`: the occlusion pyramid's bind group layout (`OcclusionGpu::late_bgl`).
    /// `car_detail`: the vehicle tiers' level (`Tiers::with_car_detail`, `--car-detail`; 0 as before).
    pub fn new(ctx: &GpuContext, look: &LookGpu, lib: &Library, sample_count: u32, debug_ids: bool, late_bgl: &wgpu::BindGroupLayout, car_detail: u8) -> Result<Self> {
        let d = &ctx.device;
        let q = &ctx.queue;
        let js = eo_agents::catalog::AgentsJs::load()?;
        let mesh_count = lib.meshes.len() as u32;
        if mesh_count == 0 || mesh_count > 511 {
            // the cull keeps the mesh in 9 bits of its record (511, was 255).
            return Err(anyhow!("agent library has {mesh_count} meshes (1..=511 supported)"));
        }
        let init = |label: &str, bytes: &[u8], usage: wgpu::BufferUsages| {
            let contents: Vec<u8> = if bytes.is_empty() { vec![0u8; 16] } else { bytes.to_vec() };
            d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents: &contents, usage })
        };
        let empty = |label: &str, size: u64, usage: wgpu::BufferUsages| {
            d.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size.max(16).next_multiple_of(16), usage, mapped_at_creation: false })
        };
        use wgpu::BufferUsages as U;
        // (also read as storage by the lamp and headlight shadow maps' casters, light_shadow.rs)
        let vbuf = init("agent-vertices", bytemuck::cast_slice(&lib.vertices), U::VERTEX | U::STORAGE);
        let ibuf = init("agent-indices", bytemuck::cast_slice(&lib.indices), U::INDEX | U::STORAGE);
        let meshes = init("agent-meshes", bytemuck::cast_slice(&lib.meshes), U::STORAGE);
        // (`--car-lamps`) each body's rear lamps in its spare words, for the light gather (agents_lamps.wgsl)
        let models = if crate::look::shader_features().car_lamps {
            init("agent-models", bytemuck::cast_slice(&lib.models_with_rear_lamps()), U::STORAGE)
        } else {
            init("agent-models", bytemuck::cast_slice(&lib.models), U::STORAGE)
        };
        let bones = init("agent-bones", bytemuck::cast_slice(&lib.bones), U::STORAGE);
        let materials = init("agent-materials", bytemuck::cast_slice(&lib.materials), U::STORAGE);
        // One object entry: the look's material class "painted" (draw.wgsl kind 0).
        let objects = init("agent-object", &[0u8; 64], U::STORAGE);
        let n = AGENT_CAPACITY as u64;
        let m = mesh_count as u64;
        let agents = empty("agents", n * 64, U::STORAGE | U::COPY_DST);
        let params = empty("agent-params", std::mem::size_of::<AgParams>() as u64, U::UNIFORM | U::COPY_DST);
        let params_c = empty("agent-params-camera", std::mem::size_of::<AgParams>() as u64, U::UNIFORM | U::COPY_DST);
        let frame = empty("agent-frame", std::mem::size_of::<AgFrame>() as u64, U::UNIFORM | U::COPY_DST);
        let records = empty("agent-records", n * VIEWS * 4, U::STORAGE);
        let visible = empty("agent-visible", n * VIEWS * 4, U::STORAGE);
        let counts = empty("agent-counts", m * VIEWS * 2 * 4, U::STORAGE | U::COPY_DST);
        let hist = empty("agent-hist", (2 * BINS + 8) * 4, U::STORAGE | U::COPY_DST);
        let cmds = empty("agent-draws", m * LISTS * DRAW_ARGS_BYTES, U::STORAGE | U::INDIRECT | U::COPY_SRC);
        let stats = empty("agent-stats", (STATS_WORDS * 4) as u64, U::STORAGE | U::COPY_DST | U::COPY_SRC);

        // Atlases: albedo (sRGB), normal, ORM; one layer per civilian family.
        // or the cooked BC chains (BC3 sRGB, BC5, BC1), decoded to RGBA8
        // on a device without BC textures.
        let res = lib.meta.atlas_res.max(1);
        let layers = (lib.atlases.len() as u32).max(1);
        let bc_levels = lib.atlases.first().filter(|a| !a.bc[0].is_empty()).map(|a| a.bc_levels);
        let bc_native = bc_levels.is_some() && d.features().contains(wgpu::Features::TEXTURE_COMPRESSION_BC);
        let mips = bc_levels.unwrap_or(if res > 1 { res.ilog2() + 1 } else { 1 });
        let mut tex_bytes = 0u64;
        let formats = if bc_native {
            [wgpu::TextureFormat::Bc3RgbaUnormSrgb, wgpu::TextureFormat::Bc5RgUnorm, wgpu::TextureFormat::Bc1RgbaUnorm]
        } else {
            [wgpu::TextureFormat::Rgba8UnormSrgb, wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureFormat::Rgba8Unorm]
        };
        let textures: Vec<wgpu::Texture> = formats
            .iter()
            .enumerate()
            .map(|(k, fmt)| {
                let t = d.create_texture(&wgpu::TextureDescriptor {
                    label: Some(["agent-atlas-albedo", "agent-atlas-normal", "agent-atlas-orm"][k]),
                    size: wgpu::Extent3d { width: res, height: res, depth_or_array_layers: layers },
                    mip_level_count: mips,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: *fmt,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                if bc_levels.is_some() {
                    // the cooked chains, level by level.
                    for (layer, a) in lib.atlases.iter().enumerate() {
                        let mut at = 0usize;
                        for mip in 0..mips {
                            let r = (res >> mip).max(4);
                            let n = eo_agents::rocketbox::bc_level_bytes(k, r);
                            let Some(level) = a.bc[k].get(at..at + n) else { break };
                            at += n;
                            let origin = wgpu::Origin3d { x: 0, y: 0, z: layer as u32 };
                            let size = wgpu::Extent3d { width: r, height: r, depth_or_array_layers: 1 };
                            if bc_native {
                                let blocks = r.div_ceil(4);
                                q.write_texture(wgpu::TexelCopyTextureInfo { texture: &t, mip_level: mip, origin, aspect: wgpu::TextureAspect::All }, level, wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(blocks * (n as u32 / (blocks * blocks))), rows_per_image: Some(blocks) }, size);
                            } else {
                                let rgba = eo_agents::rocketbox::bc_level_rgba(k, level, r);
                                q.write_texture(wgpu::TexelCopyTextureInfo { texture: &t, mip_level: mip, origin, aspect: wgpu::TextureAspect::All }, &rgba, wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(r * 4), rows_per_image: Some(r) }, size);
                            }
                        }
                    }
                    tex_bytes += (0..mips).map(|mp| if bc_native { eo_agents::rocketbox::bc_level_bytes(k, (res >> mp).max(4)) as u64 } else { ((res >> mp).max(4) as u64).pow(2) * 4 }).sum::<u64>() * layers as u64;
                    return t;
                }
                for (layer, a) in lib.atlases.iter().enumerate() {
                    let chain = crate::authored::mip_chain(&a.maps[k], res as usize, k == 0);
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
                tex_bytes += (0..mips).map(|mp| ((res >> mp).max(1) as u64).pow(2) * 4).sum::<u64>() * layers as u64;
                t
            })
            .collect();
        let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        let sampler = d.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("agent-atlas-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 16,
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

        // Cull.
        let cull_src = format!("{}\n{}\nfn ag_occluded(mn: vec3<f32>, mx: vec3<f32>) -> bool {{ return false; }}\n", include_str!("agents_types.wgsl"), include_str!("agents_cull.wgsl"));
        // the camera view after the pyramid (occlusion.wgsl's `hzb_occluded`).
        let hzb_src = format!(
            "{}\n{}\nstruct HzbInfo {{ screen: vec4<f32> }};\n@group(1) @binding(0) var hzb: texture_2d<f32>;\n@group(1) @binding(1) var<uniform> hzb_info: HzbInfo;\n{}\nfn ag_occluded(mn: vec3<f32>, mx: vec3<f32>) -> bool {{ return hzb_occluded(mn, mx, ag.view_proj, hzb_info.screen.xy); }}\n",
            include_str!("agents_types.wgsl"),
            include_str!("agents_cull.wgsl"),
            crate::occlusion::OCCLUSION_WGSL
        );
        let cull_module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("agents_cull.wgsl"), source: wgpu::ShaderSource::Wgsl(cull_src.into()) });
        let cull_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("agent-cull-bgl"),
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
                entry(9, cs, sbuf(false)),
                entry(10, cs, ubuf),
            ],
        });
        let cull_bg_for = |p: &wgpu::Buffer, label: &str| d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &cull_bgl,
            entries: &[
                bge(0, p),
                bge(1, &agents),
                bge(2, &models),
                bge(3, &meshes),
                bge(4, &records),
                bge(5, &visible),
                bge(6, &counts),
                bge(7, &hist),
                bge(8, &cmds),
                bge(9, &stats),
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: look.shadow_view_buffer(), offset: 0, size: Some(std::num::NonZeroU64::new(768).unwrap()) }),
                },
            ],
        });
        let cull_bg = cull_bg_for(&params, "agent-cull-bg");
        let cull_bg_c = cull_bg_for(&params_c, "agent-cull-bg-camera");
        let cull_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("agent-cull-layout"), bind_group_layouts: &[Some(&cull_bgl)], immediate_size: 0 });
        let compute = |e: &str| {
            d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(e), layout: Some(&cull_layout), module: &cull_module, entry_point: Some(e), compilation_options: Default::default(), cache: None })
        };
        let cull = [compute("classify"), compute("cutoffs"), compute("assign"), compute("offsets"), compute("scatter")];
        // the late camera cull binds the pyramid as group 1.
        let hzb_module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("agents_cull.wgsl (hzb)"), source: wgpu::ShaderSource::Wgsl(hzb_src.into()) });
        let hzb_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("agent-cull-hzb-layout"), bind_group_layouts: &[Some(&cull_bgl), Some(late_bgl)], immediate_size: 0 });
        let compute_c = |e: &str| {
            d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(e), layout: Some(&hzb_layout), module: &hzb_module, entry_point: Some(e), compilation_options: Default::default(), cache: None })
        };
        let cull_c = [compute_c("assign"), compute_c("offsets"), compute_c("scatter")];

        // Agent data (group 2 of the colour and shadow pipelines).
        let data_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("agent-data-bgl"),
            entries: &[entry(0, vf, ubuf), entry(1, vf, sbuf(true)), entry(2, vf, sbuf(true)), entry(3, vf, sbuf(true)), entry(4, vf, sbuf(true)), entry(5, vf, sbuf(true))],
        });
        let data_bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("agent-data-bg"),
            layout: &data_bgl,
            entries: &[bge(0, &params), bge(1, &agents), bge(2, &models), bge(3, &meshes), bge(4, &bones), bge(5, &visible)],
        });
        // Group 0 in draw.wgsl's layout, with the agents' own resources.
        let tex = |binding| {
            entry(binding, fs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false })
        };
        let draw_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("agent-draw-bgl"),
            entries: &[
                entry(0, vf, ubuf),
                entry(1, vf, sbuf(true)),
                entry(2, fs, sbuf(true)),
                tex(3),
                tex(4),
                tex(5),
                entry(6, fs, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                // look_light names the scene probe; agents keep the analytic sky
                // (their frame says no probe), so a 1 x 1 stand-in is bound.
                tex(14),
                entry(15, fs, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                entry(16, fs, sbuf(true)),
            ],
        });
        let probe_stand_in = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("agent-probe-stand-in"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 6 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let probe_view = view(&probe_stand_in); // facade
        let probe_sh = d.create_buffer(&wgpu::BufferDescriptor { label: Some("agent-probe-sh-stand-in"), size: 160, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false }); // facade
        let (av, nv, ov) = (view(&textures[0]), view(&textures[1]), view(&textures[2]));
        let draw_bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("agent-draw-bg"),
            layout: &draw_bgl,
            entries: &[
                bge(0, &frame),
                bge(1, &objects),
                bge(2, &materials),
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&av) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&nv) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&ov) },
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::Sampler(&sampler) },
                wgpu::BindGroupEntry { binding: 14, resource: wgpu::BindingResource::TextureView(&probe_view) }, // facade
                wgpu::BindGroupEntry { binding: 15, resource: wgpu::BindingResource::Sampler(&sampler) },        // facade
                wgpu::BindGroupEntry { binding: 16, resource: probe_sh.as_entire_binding() },                   // facade
            ],
        });
        // (`--car-contact`) the cars' occlusion of the street, appended only with the switch on.
        let contact_on = look.options.car_contact > 0;
        let draw_src = format!(
            "{}\n{}\n{}\n{}\n{}\n{}\n{}{}\n{}",
            // the city's full set but the single probe's lookups (their frame never names one; lighting: room for the probe grid)
            agents_city_source(),
            people_shading_wgsl(), // (`--people-shading`, `--people-bounce`) compiled out without them
            car_wgsl(), // (`--car-panels`, `--car-lamps`) the vehicles' generated constants
            include_str!("agents_types.wgsl"),
            include_str!("agents_common.wgsl"),
            include_str!("agents_draw.wgsl"),
            include_str!("sky_velocity.wgsl"), // sky
            if contact_on { concat!("\n", include_str!("agents_contact.wgsl")) } else { "" }, // (`--car-contact`) only with it
            people_contact_wgsl() // (`--people-contact`) only with it
        );
        let draw_module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("agents_draw.wgsl"), source: wgpu::ShaderSource::Wgsl(draw_src.into()) });
        let draw_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("agent-draw-layout"),
            bind_group_layouts: &[Some(&draw_bgl), Some(look.main_bind_group_layout()), Some(&data_bgl)],
            immediate_size: 0,
        });
        let pipes = AgentPipes { device: d.clone(), module: draw_module.clone(), layout: draw_layout.clone(), sample_count, debug_ids };
        let contact = contact_on.then(|| crate::agents_contact::Contact::new(d, &draw_module, [&draw_bgl, look.main_bind_group_layout(), &data_bgl]));
        let attrs = AgentPipes::ATTRS;
        let stride = std::mem::size_of::<AgentVertex>() as u64;
        // (`--car-finish`) the cars and the people in pipelines of their own (each
        // without the other's code: the two in one were more than Apple's compiler takes)
        let split = if crate::look::shader_features().car_finish { person_mesh_split(lib) } else { None };
        let fs = if debug_ids { "agent_fs_ids" } else { "agent_fs" };
        let cars = if crate::look::shader_features().car_finish && (split.is_some() || lib.meta.people_models.is_empty()) { 1 } else { 0 };
        let opaque = pipes.make_for("agent-opaque", fs, false, cars);
        let glass = pipes.make_for("agent-glass", fs, true, cars);
        let people_pipes = split.map(|k| (k, pipes.make_for("agent-opaque-people", fs, false, 2), pipes.make_for("agent-glass-people", fs, true, 2)));
        // the agents' screen motion for the temporal resolve (single-sample, no ids).
        let velocity = (sample_count == 1 && !debug_ids).then(|| {
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("agent-velocity"),
                layout: Some(&draw_layout),
                vertex: wgpu::VertexState {
                    module: &draw_module,
                    entry_point: Some("agent_vel_vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout { array_stride: stride, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attrs })],
                },
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil: Default::default(),
                    bias: wgpu::DepthBiasState { constant: 4, slope_scale: 1.0, clamp: 0.0 },
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &draw_module,
                    entry_point: Some("agent_vel_fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: crate::sky::VEL_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        });

        // (`--people-contact`) the people's ground occlusion: a quad on the
        // ground round each person within reach, multiplying the lit frame
        // (dst x src), depth-tested and not written, after the opaque agents.
        let people_contact_pipe = people_ground_pass().then(|| {
            let color = wgpu::ColorTargetState {
                format: crate::look::HDR_FORMAT,
                blend: Some(wgpu::BlendState {
                    color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::Src, operation: wgpu::BlendOperation::Add },
                    alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
                }),
                write_mask: wgpu::ColorWrites::ALL,
            };
            let ids = wgpu::ColorTargetState { format: IDS_FORMAT, blend: None, write_mask: wgpu::ColorWrites::empty() };
            let targets_ids = [Some(color.clone()), Some(ids)];
            let targets = [Some(color)];
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("agent-contact"),
                layout: Some(&draw_layout),
                vertex: wgpu::VertexState { module: &draw_module, entry_point: Some("agent_contact_vs"), compilation_options: Default::default(), buffers: &[] },
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState { count: sample_count, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &draw_module,
                    entry_point: Some(if debug_ids { "agent_contact_fs_ids" } else { "agent_contact_fs" }),
                    compilation_options: Default::default(),
                    targets: if debug_ids { &targets_ids } else { &targets },
                }),
                multiview_mask: None,
                cache: None,
            })
        });

        // Shadow casters.
        let view_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("agent-shadow-view-bgl"),
            entries: &[entry(0, wgpu::ShaderStages::VERTEX, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: None })],
        });
        let view_bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("agent-shadow-view-bg"),
            layout: &view_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: look.shadow_view_buffer(), offset: 0, size: Some(std::num::NonZeroU64::new(80).unwrap()) }),
            }],
        });
        let shadow_src = format!("{}\n{}\n{}", include_str!("agents_types.wgsl"), include_str!("agents_common.wgsl"), include_str!("agents_shadow.wgsl"));
        let shadow_module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("agents_shadow.wgsl"), source: wgpu::ShaderSource::Wgsl(shadow_src.into()) });
        let shadow_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("agent-shadow-layout"),
            bind_group_layouts: &[Some(&view_bgl), None, Some(&data_bgl)],
            immediate_size: 0,
        });
        let shadow = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("agent-shadow"),
            layout: Some(&shadow_layout),
            vertex: wgpu::VertexState {
                module: &shadow_module,
                entry_point: Some("agent_shadow_vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout { array_stride: stride, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attrs })],
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
            fragment: None,
            multiview_mask: None,
            cache: None,
        });

        let lamp = |k: &str| js.hero_lamp.get(k).map(|l| (l.day as f32, l.night as f32)).unwrap_or((0.0, 0.0));
        let (tail, brake, head, ind) = (lamp("tail"), lamp("brake"), lamp("head"), lamp("indicator"));
        let bytes = [&vbuf, &ibuf, &meshes, &models, &bones, &materials, &objects, &agents, &params, &params_c, &frame, &records, &visible, &counts, &hist, &cmds, &stats].iter().map(|b| b.size()).sum::<u64>() + tex_bytes;
        // (lighting) what the lamp and headlight shadow maps draw the agents with.
        let casters = crate::light_shadow::Casters { vertices: vbuf.clone(), indices: ibuf.clone(), instances: agents.clone(), models: models.clone(), meshes: meshes.clone(), data_bgl: data_bgl.clone(), data_bg: data_bg.clone() };
        Ok(Self {
            enabled: AtomicBool::new(true),
            active: AtomicBool::new(false),
            count: AtomicU32::new(0),
            mesh_count,
            model_count: lib.models.len() as u32,
            tiers: Tiers::from_js(&js).with_car_detail(car_detail),
            clip_frames: lib.meta.clip_frames,
            clip_base: lib.meta.clip_base,
            lamps: ([tail.0, tail.1, brake.0, brake.1], [head.0, head.1, ind.0, ind.1], js.hero_lamp_day_running_floor as f32),
            vbuf,
            ibuf,
            agents,
            params,
            frame,
            counts,
            hist,
            cmds,
            stats,
            cull_bg,
            cull,
            params_c,
            cull_bg_c,
            cull_c,
            camera_late: AtomicBool::new(false),
            draw_bg,
            data_bg,
            view_bg,
            opaque,
            glass,
            people_pipes,
            bore: std::sync::OnceLock::new(),
            bore_live: AtomicBool::new(false),
            pipes,
            velocity,                // sky
            contact,
            models_buf: models.clone(), // sky
            shadow,
            people_contact: people_contact_pipe,
            timing: Mutex::new(None),
            casters,
            bytes,
        })
    }

    /// Agents on or off (off: no pass runs, nothing is drawn).
    /// The distance tiers and caps in use (`--car-detail` for the vehicles).
    pub fn tiers(&self) -> Tiers {
        self.tiers
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// This frame's agents (at most `AGENT_CAPACITY`; the rest are dropped).
    pub fn upload(&self, queue: &wgpu::Queue, agents: &[GpuAgent]) -> usize {
        let n = agents.len().min(AGENT_CAPACITY as usize);
        // (`--tunnels`) agents in a bore: drawn by the bores' pipelines when they exist; without
        // them (never enabled) the bit is taken off, so agent-opaque draws them as before.
        let bored = agents[..n].iter().any(|a| a.flags & eo_agents::AGENT_IN_BORE != 0);
        if n > 0 {
            if bored && self.bore.get().is_none() {
                let plain: Vec<GpuAgent> = agents[..n].iter().map(|a| GpuAgent { flags: a.flags & !(eo_agents::AGENT_IN_BORE | (eo_agents::BORE_DEPTH_MAX_Q << eo_agents::BORE_DEPTH_SHIFT)), ..*a }).collect();
                queue.write_buffer(&self.agents, 0, bytemuck::cast_slice(&plain));
            } else {
                queue.write_buffer(&self.agents, 0, bytemuck::cast_slice(&agents[..n]));
            }
        }
        self.count.store(n as u32, Ordering::Relaxed);
        self.bore_live.store(bored && self.bore.get().is_some(), Ordering::Relaxed);
        n
    }

    /// (`--tunnels`) The pipelines that draw the agents in a tunnel's bore
    /// (once; the app calls it when its world has bores). Until then, and
    /// without bores, nothing of them is made or drawn.
    pub fn enable_bores(&self) {
        self.bore.get_or_init(|| {
            let fs = if self.pipes.debug_ids { "agent_bore_fs_ids" } else { "agent_bore_fs" };
            [self.pipes.make("agent-bore-opaque", fs, false), self.pipes.make("agent-bore-glass", fs, true)]
        });
    }

    /// Whether the bores' pipelines exist and this frame's agents hold one in a bore.
    pub fn bores_drawn(&self) -> bool {
        self.bore.get().is_some() && self.bore_live.load(Ordering::Relaxed)
    }

    pub fn count(&self) -> u32 {
        self.count.load(Ordering::Relaxed)
    }

    /// Timestamps for the coming encode (bench): a query set and the first of `QUERIES_PER_FRAME` slots.
    pub fn set_timestamps(&self, ts: Option<(wgpu::QuerySet, u32)>) {
        *self.timing.lock().unwrap() = ts;
    }

    pub fn stats_buffer(&self) -> &wgpu::Buffer {
        &self.stats
    }

    /// The camera's uniforms (after the look's update, which fitted this frame's cascades).
    pub fn update(&self, queue: &wgpu::Queue, cam: &Camera, look: &LookGpu, seconds: f64) {
        let vp = cam.view_proj();
        let planes = frustum_planes(vp).map(|p| p.to_array());
        let alt = look.frame().model.sun.altitude_deg as f32;
        // Day / night lamp state from the sun (not the JS's own lamp clock): night below -6 deg, day above 6 deg.
        let night = ((6.0 - alt) / 12.0).clamp(0.0, 1.0);
        let t = self.tiers;
        let p = AgParams {
            planes,
            cam: [cam.eye.x, cam.eye.y, cam.eye.z, 0.0],
            counts: [self.count(), self.mesh_count, AGENT_CAPACITY, self.model_count],
            veh_radius: t.veh_radius,
            veh_cap: t.veh_cap,
            ped_radius: t.ped_radius,
            ped_cap: t.ped_cap,
            clip_frames: self.clip_frames,
            clip_base: self.clip_base,
            lamp_tail: self.lamps.0,
            lamp_head: self.lamps.1,
            lamp_misc: [night, self.lamps.2, seconds.rem_euclid(3600.0) as f32, 0.0],
            // before the shadows the cascades, and the camera only
            // without an occlusion pass this frame.
            // y: (`--car-materials`) the cars' materials' level (agents_draw.wgsl `ag_car_on`; 0 as before).
            // z: (`--car-lamps`) bit 0, (`--car-wet`) bit 1, (`--car-finish`) bit 2 (agents_draw.wgsl `ag_car_lamps_on`, `ag_car_wet_on`, `ag_car_finish_on`; 0 as before).
            misc: [if self.camera_late.load(Ordering::Relaxed) { 0b1110 } else { 0b1111 }, look.options.car_materials as u32, (look.options.car_lamps > 0) as u32 | ((look.options.car_wet > 0) as u32) << 1 | ((look.options.car_finish > 0) as u32) << 2, 0],
            view_proj: vp.to_cols_array_2d(),
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&p));
        let pc = AgParams { misc: [0b0001, p.misc[1], p.misc[2], 0], ..p }; // after the pyramid, the camera
        queue.write_buffer(&self.params_c, 0, bytemuck::bytes_of(&pc));
        let f = AgFrame {
            view_proj: vp.to_cols_array_2d(),
            cam_pos: cam.eye.extend(1.0).to_array(),
            sun_dir: [0.0, 1.0, 0.0, 0.0],
            sun_color: [1.0, 1.0, 1.0, 0.0],
            ambient_sky: [0.0; 4],
            ambient_ground: [0.0; 4],
            // y: the weathering level's switch bits only (`--grime`; 0 by default), read by agents_draw.wgsl's cars.
            copy_step: [0.0, crate::facade::grime_bits(look.options.grime) as f32, 0.0, 0.0],
        };
        queue.write_buffer(&self.frame, 0, bytemuck::bytes_of(&f));
    }

    /// The compute pass and the casters into the cascades the look refreshed
    /// this frame (after `LookGpu::encode_shadows`, before the city pass).
    pub fn encode(&self, enc: &mut wgpu::CommandEncoder, look: &LookGpu) {
        let n = self.count();
        let on = self.enabled() && n > 0;
        self.active.store(on, Ordering::Relaxed);
        if !on {
            return;
        }
        let ts = self.timing.lock().unwrap().clone();
        enc.clear_buffer(&self.counts, 0, None);
        enc.clear_buffer(&self.hist, 0, None);
        enc.clear_buffer(&self.stats, 0, None);
        {
            let tw = ts.as_ref().map(|(q, b)| wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(*b), end_of_pass_write_index: Some(*b + 1) });
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("agents-cull"), timestamp_writes: tw });
            pass.set_bind_group(0, &self.cull_bg, &[]);
            let groups = n.div_ceil(64);
            for (k, p) in self.cull.iter().enumerate() {
                pass.set_pipeline(p);
                pass.dispatch_workgroups(if k == 1 || k == 3 { 1 } else { groups }, 1, 1);
            }
        }
        let mask = look.encoded_shadow_mask();
        let layers = look.shadow_layer_views();
        let list: Vec<usize> = (0..3).filter(|c| mask & (1 << c) != 0).collect();
        let m = self.mesh_count as u64;
        if list.is_empty() {
            if let Some((q, b)) = &ts {
                // Keep the query pair written.
                enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("agents-casters (none this frame)"),
                    timestamp_writes: Some(wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(*b + 2), end_of_pass_write_index: Some(*b + 3) }),
                });
            }
            return;
        }
        for (k, &c) in list.iter().enumerate() {
            let tw = ts.as_ref().and_then(|(q, b)| {
                let begin = (k == 0).then_some(*b + 2);
                let end = (k == list.len() - 1).then_some(*b + 3);
                (begin.is_some() || end.is_some()).then_some(wgpu::RenderPassTimestampWrites { query_set: q, beginning_of_pass_write_index: begin, end_of_pass_write_index: end })
            });
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("agents-casters"),
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
            pass.set_pipeline(&self.shadow);
            pass.set_bind_group(0, &self.view_bg, &[(c as u64 * 256) as u32]);
            pass.set_bind_group(2, &self.data_bg, &[]);
            pass.set_vertex_buffer(0, self.vbuf.slice(..));
            pass.set_index_buffer(self.ibuf.slice(..), wgpu::IndexFormat::Uint32);
            pass.multi_draw_indexed_indirect(&self.cmds, (2 + c as u64) * m * DRAW_ARGS_BYTES, self.mesh_count);
        }
    }

    /// whether the coming frames test the camera view against the
    /// occlusion pyramid (`cull_camera`, inside the pyramid pass) instead of
    /// in `encode`. Set before `update`.
    pub fn set_camera_late(&self, on: bool) {
        self.camera_late.store(on, Ordering::Relaxed);
    }

    /// inside the occlusion compute pass, after the pyramid: the camera
    /// view's tiers and commands, an agent the pyramid hides left out
    /// (`hzb_bg`: `OcclusionGpu::late_bgl`'s group). The casters were assigned
    /// in `encode`, before the shadow passes, as they must be: a car hidden
    /// from the camera still casts a shadow the camera sees.
    pub fn cull_camera(&self, pass: &mut wgpu::ComputePass<'_>, hzb_bg: &wgpu::BindGroup) {
        if !self.active.load(Ordering::Relaxed) || !self.camera_late.load(Ordering::Relaxed) {
            return;
        }
        let groups = self.count().div_ceil(64);
        pass.set_bind_group(0, &self.cull_bg_c, &[]);
        pass.set_bind_group(1, hzb_bg, &[]);
        for (k, p) in self.cull_c.iter().enumerate() {
            pass.set_pipeline(p);
            pass.dispatch_workgroups(if k == 1 { 1 } else { groups }, 1, 1);
        }
    }

    /// Inside the city pass, after the opaque city list. Leaves the agent
    /// vertex and index buffers bound: the caller binds its own again.
    pub fn draw_opaque(&self, pass: &mut wgpu::RenderPass<'_>, look: &LookGpu) {
        if !self.active.load(Ordering::Relaxed) {
            return;
        }
        pass.set_pipeline(&self.opaque);
        pass.set_bind_group(0, &self.draw_bg, &[]);
        look.bind_main(pass);
        pass.set_bind_group(2, &self.data_bg, &[]);
        pass.set_vertex_buffer(0, self.vbuf.slice(..));
        pass.set_index_buffer(self.ibuf.slice(..), wgpu::IndexFormat::Uint32);
        if let Some((k, people, _)) = &self.people_pipes {
            // (`--car-finish`) the cars' meshes, then the people's in their own pipeline
            pass.multi_draw_indexed_indirect(&self.cmds, 0, *k);
            pass.set_pipeline(people);
            pass.multi_draw_indexed_indirect(&self.cmds, *k as u64 * DRAW_ARGS_BYTES, self.mesh_count - k);
        } else {
            pass.multi_draw_indexed_indirect(&self.cmds, 0, self.mesh_count);
        }
        // (`--tunnels`) the same list again for the agents in a bore (the first draw discarded them).
        if let Some(b) = self.bore.get().filter(|_| self.bore_live.load(Ordering::Relaxed)) {
            pass.set_pipeline(&b[0]);
            pass.multi_draw_indexed_indirect(&self.cmds, 0, self.mesh_count);
        }
        // (`--people-contact`) the ground round each person, occluded by them (same groups bound).
        if let Some(p) = &self.people_contact {
            pass.set_pipeline(p);
            pass.draw(0..6, 0..self.count());
        }
    }

    /// (`--car-contact`) Whether the cars' occlusion of the street will read this depth after the city pass.
    pub fn contact_reads(&self, depth: &wgpu::TextureView) -> bool {
        self.contact.is_some() && crate::agents_contact::Contact::reads(depth)
    }

    /// (`--car-contact`) After the city pass: the street round each car near
    /// the eye multiplied by what its body leaves of the light (agents_contact.wgsl).
    /// Nothing without the switch or without agents this frame.
    pub fn encode_contact(&self, enc: &mut wgpu::CommandEncoder, look: &LookGpu, hdr: &wgpu::TextureView, depth: &wgpu::TextureView) {
        let Some(c) = &self.contact else { return };
        if !self.active.load(Ordering::Relaxed) {
            return;
        }
        c.encode(&self.pipes.device, enc, &self.draw_bg, &self.data_bg, look, hdr, depth, self.count());
    }

    /// the agent instances, the model table and how many agents are live (the lights' gather reads them).
    pub fn sky_buffers(&self) -> (&wgpu::Buffer, &wgpu::Buffer, u32) {
        let n = if self.active.load(Ordering::Relaxed) { self.count.load(Ordering::Relaxed) } else { 0 };
        (&self.agents, &self.models_buf, n)
    }

    /// the camera's visible agents again, into the velocity target (after the city pass).
    pub fn draw_velocity(&self, pass: &mut wgpu::RenderPass<'_>, look: &LookGpu) {
        let Some(p) = &self.velocity else { return };
        if !self.active.load(Ordering::Relaxed) {
            return;
        }
        pass.set_pipeline(p);
        pass.set_bind_group(0, &self.draw_bg, &[]);
        look.bind_main(pass);
        pass.set_bind_group(2, &self.data_bg, &[]);
        pass.set_vertex_buffer(0, self.vbuf.slice(..));
        pass.set_index_buffer(self.ibuf.slice(..), wgpu::IndexFormat::Uint32);
        pass.multi_draw_indexed_indirect(&self.cmds, 0, self.mesh_count);
    }

    /// Inside the city pass, after the city's transparent list: glass and lenses.
    pub fn draw_glass(&self, pass: &mut wgpu::RenderPass<'_>, look: &LookGpu) {
        if !self.active.load(Ordering::Relaxed) {
            return;
        }
        pass.set_pipeline(&self.glass);
        pass.set_bind_group(0, &self.draw_bg, &[]);
        look.bind_main(pass);
        pass.set_bind_group(2, &self.data_bg, &[]);
        pass.set_vertex_buffer(0, self.vbuf.slice(..));
        pass.set_index_buffer(self.ibuf.slice(..), wgpu::IndexFormat::Uint32);
        if let Some((k, _, people)) = &self.people_pipes {
            // (`--car-finish`) the cars' glass and lenses, then the people's in their own pipeline
            pass.multi_draw_indexed_indirect(&self.cmds, self.mesh_count as u64 * DRAW_ARGS_BYTES, *k);
            pass.set_pipeline(people);
            pass.multi_draw_indexed_indirect(&self.cmds, (self.mesh_count + k) as u64 * DRAW_ARGS_BYTES, self.mesh_count - k);
        } else {
            pass.multi_draw_indexed_indirect(&self.cmds, self.mesh_count as u64 * DRAW_ARGS_BYTES, self.mesh_count);
        }
        if let Some(b) = self.bore.get().filter(|_| self.bore_live.load(Ordering::Relaxed)) {
            pass.set_pipeline(&b[1]);
            pass.multi_draw_indexed_indirect(&self.cmds, self.mesh_count as u64 * DRAW_ARGS_BYTES, self.mesh_count);
        }
    }

    /// The command lists as the last cull wrote them (tests; blocks): camera
    /// opaque, camera transparent, cascades 0..2, `mesh_count` commands each.
    pub fn commands_readback(&self, ctx: &GpuContext) -> Result<Vec<[u32; 5]>> {
        let size = self.cmds.size();
        let staging = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: Some("agent-cmds-readback"), size, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(&self.cmds, 0, &staging, 0, size);
        ctx.queue.submit([enc.finish()]);
        let words: Vec<u32> = bytemuck::pod_collect_to_vec(&crate::read_mapped(ctx, &staging)?);
        Ok(words.chunks_exact(5).take(self.mesh_count as usize * LISTS as usize).map(|c| [c[0], c[1], c[2], c[3], c[4]]).collect())
    }

    /// The stats words of the last cull (tests; blocks): see `STATS_WORDS`.
    pub fn stats_readback(&self, ctx: &GpuContext) -> Result<Vec<u32>> {
        let size = self.stats.size();
        let staging = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: Some("agent-stats-readback"), size, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(&self.stats, 0, &staging, 0, size);
        ctx.queue.submit([enc.finish()]);
        Ok(bytemuck::pod_collect_to_vec(&crate::read_mapped(ctx, &staging)?))
    }

    /// Slots per model (vehicles: LOD0-2 + box; hero, near, mid, far).
    pub const LOD_SLOTS: usize = LODS;
}

#[cfg(test)]
mod tests {
    /// agents_draw.wgsl's bore constants are eo-agents' (`AGENT_IN_BORE`, the depth's shift and steps).
    #[test]
    fn the_shaders_bore_flags_are_the_feeds() {
        let src = include_str!("agents_draw.wgsl");
        assert!(src.contains(&format!("const AG_IN_BORE: u32 = {}u;", eo_agents::AGENT_IN_BORE)), "AG_IN_BORE");
        assert!(src.contains(&format!("const AG_BORE_DEPTH_SHIFT: u32 = {}u;", eo_agents::BORE_DEPTH_SHIFT)), "AG_BORE_DEPTH_SHIFT");
        assert!(src.contains(&format!("const AG_BORE_DEPTH_QUANT: f32 = {:.1};", eo_agents::BORE_DEPTH_QUANT)), "AG_BORE_DEPTH_QUANT");
        assert_eq!(eo_agents::AGENT_IN_BORE & (eo_agents::AGENT_PLAYER | eo_agents::AGENT_BLINK_ON | 0xffff), 0, "a bit of its own");
    }

    /// (`--car-materials`) agents_draw.wgsl's material roles are eo-agents' (`vehicles::M_ROLE_SHIFT`, `ROLE_*`).
    #[test]
    fn the_shaders_car_roles_are_the_cooks() {
        use eo_agents::vehicles as v;
        let src = include_str!("agents_draw.wgsl");
        assert!(src.contains(&format!("const AG_ROLE_SHIFT: u32 = {}u;", v::M_ROLE_SHIFT)), "AG_ROLE_SHIFT");
        for (name, role) in [
            ("GLASS", v::ROLE_GLASS),
            ("GLASS_TINT", v::ROLE_GLASS_TINT),
            ("TYRE", v::ROLE_TYRE),
            ("PLATE", v::ROLE_PLATE),
            ("CHROME", v::ROLE_CHROME),
            ("LENS", v::ROLE_LENS),
            ("INTERIOR", v::ROLE_INTERIOR),
            ("WHEEL", v::ROLE_WHEEL),
            ("TRIM", v::ROLE_TRIM),
            ("REFLECTOR", v::ROLE_REFLECTOR),
            ("EMERGENCY", v::ROLE_EMERGENCY),
            ("BRAKE", v::ROLE_BRAKE),
            ("LIVERY", v::ROLE_LIVERY),
            ("BADGE", v::ROLE_BADGE),
        ] {
            assert!(src.contains(&format!("const AG_ROLE_{name}: u32 = {role}u;")), "AG_ROLE_{name}");
        }
        // the role field sits above every agent and draw.wgsl flag the shaders read.
        assert_eq!(v::M_ROLE_MASK & (eo_agents::M_RIM | (eo_agents::M_RIM - 1)), 0, "a field of its own");
    }

    /// (`--people-shading`) each class roughness the Rocketbox cook writes,
    /// as BC1 keeps it (6 bits of green; a block of one value keeps it
    /// exactly), lands in its class's band of agents_draw.wgsl `pp_class`,
    /// and no two classes share a band.
    #[test]
    fn the_people_classes_read_back_from_the_cooks_roughness() {
        use eo_agents::rocketbox::{ROUGH_CLOTH, ROUGH_GEAR, ROUGH_GLASS, ROUGH_HAIR, ROUGH_HEAD, ROUGH_SKIN};
        let src = include_str!("agents_draw.wgsl");
        let band = |name: &str| -> (f32, f32) {
            let line = src.lines().find(|l| l.starts_with(&format!("const {name}: vec2<f32> = vec2<f32>("))).unwrap_or_else(|| panic!("{name}"));
            let inner = &line[line.find("vec2<f32>(").unwrap() + 10..line.rfind(')').unwrap()];
            let v: Vec<f32> = inner.split(',').map(|x| x.trim().parse().unwrap()).collect();
            (v[0], v[1])
        };
        let bc1 = |r: f32| (r * 63.0).round() / 63.0;
        let inside = |r: f32, b: (f32, f32)| r >= b.0 && r < b.1;
        let (hair, skin, cloth) = (band("PP_HAIR_BAND"), band("PP_SKIN_BAND"), band("PP_CLOTH_BAND"));
        for r in [ROUGH_HEAD, ROUGH_SKIN] {
            assert!(inside(bc1(r), skin) && !inside(bc1(r), hair) && !inside(bc1(r), cloth), "skin {r}");
        }
        assert!(inside(bc1(ROUGH_HAIR), hair) && !inside(bc1(ROUGH_HAIR), skin), "hair");
        assert!(inside(bc1(ROUGH_CLOTH), cloth), "cloth");
        for r in [ROUGH_GEAR, ROUGH_GLASS] {
            assert!(!inside(bc1(r), skin) && !inside(bc1(r), hair) && !inside(bc1(r), cloth), "neither {r}");
        }
        // a block mixing skin and cloth (BC1's thirds between the two) reads as neither or cloth, never skin.
        let (s, c) = (bc1(ROUGH_SKIN), bc1(ROUGH_CLOTH));
        assert!(!inside(s + (c - s) / 3.0, skin));
    }

    /// (`--people-shading`, `--people-bounce`, `--people-contact`,
    /// `--people-capsules`, `--people-lamps`) the agents' colour shader validates with each of
    /// the people's switches compiled in and out, the constants say which,
    /// and the ground pass's shader is there only with one of its switches
    /// (the capsules' in the contact's place).
    #[test]
    fn the_people_shading_compiles_in_and_out() {
        for mask in 0..32u32 {
            let (shading, bounce, contact, capsules, lamps) = (mask & 1 != 0, mask & 2 != 0, mask & 4 != 0, mask & 8 != 0, mask & 16 != 0);
            super::set_people_shading(shading);
            super::set_people_bounce(bounce);
            super::set_people_contact(contact);
            super::set_people_capsules(capsules);
            super::set_people_lamps(lamps);
            let konst = super::people_shading_wgsl();
            assert_eq!(konst, format!("const SF_PEOPLE_SHADING: bool = {shading};\nconst SF_PEOPLE_BOUNCE: bool = {bounce};\nconst SF_PEOPLE_LAMPS: bool = {lamps};"));
            assert_eq!(super::people_contact_wgsl().contains("fn agent_contact_vs"), contact || capsules);
            assert_eq!(super::people_contact_wgsl().contains("fn pk_sky"), capsules);
            assert_eq!(super::people_ground_pass(), contact || capsules);
            let src = format!(
                "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
                crate::look::city_shader_source_variant_probes(crate::ambient::ShaderLevels::Full, crate::look::ShadowLevels::Full, false),
                konst,
                super::car_wgsl(),
                include_str!("agents_types.wgsl"),
                include_str!("agents_common.wgsl"),
                include_str!("agents_draw.wgsl"),
                include_str!("sky_velocity.wgsl"),
                super::people_contact_wgsl()
            );
            let module = wgpu::naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("people {mask}: {}", e.emit_to_string(&src)));
            wgpu::naga::valid::Validator::new(wgpu::naga::valid::ValidationFlags::all(), wgpu::naga::valid::Capabilities::all()).validate(&module).unwrap_or_else(|e| panic!("people {mask}: {e:?}"));
        }
        super::set_people_shading(false);
        super::set_people_bounce(false);
        super::set_people_contact(false);
        super::set_people_capsules(false);
        super::set_people_lamps(false);
    }

    /// (`--car-panels`, `--car-lamps`) the agents' colour shader validates with the panels compiled in
    /// and out, the constant says which, and a running lamp by day is a bright strip in sunlight
    /// (several times the white of a sunlit card, under a headlamp's core).
    #[test]
    fn the_car_panels_compile_in_and_out() {
        for on in [false, true] {
            super::set_car_panels(on);
            let konst = super::car_wgsl();
            let want = on && crate::look::shader_features().car_materials;
            assert!(konst.contains(&format!("const SF_CAR_PANELS: bool = {want};")), "{konst}");
            let src = format!(
                "{}\n{}\n{}\n{}\n{}\n{}\n{}",
                crate::look::city_shader_source_variant_probes(crate::ambient::ShaderLevels::Full, crate::look::ShadowLevels::Full, false),
                super::people_shading_wgsl(),
                konst,
                include_str!("agents_types.wgsl"),
                include_str!("agents_common.wgsl"),
                include_str!("agents_draw.wgsl"),
                include_str!("sky_velocity.wgsl"),
            );
            let module = wgpu::naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("panels {on}: {}", e.emit_to_string(&src)));
            wgpu::naga::valid::Validator::new(wgpu::naga::valid::ValidationFlags::all(), wgpu::naga::valid::Capabilities::all()).validate(&module).unwrap_or_else(|e| panic!("panels {on}: {e:?}"));
        }
        super::set_car_panels(false);
        // a white card in the noon sun (90 klx) is ~2.9e4 cd/m^2; a running lamp's strip four times that
        let drl = super::DRL_CD / super::DRL_LENS_M2;
        let card = 90_000.0 / std::f64::consts::PI;
        assert!(drl > 3.0 * card && drl < 1e6, "{drl:.0} cd/m^2");
    }

    /// (`--soft-shadows`, `--occluder-light`) the agents' module keeps the penumbra,
    /// looked up at the top of the fragment (`LK_SUN_PRE`), and takes the
    /// occluder light with the street's sky from its own sample (`AMB_OL_STREET`
    /// off), with or without the cars' materials; the stand-in as before without it.
    #[test]
    fn the_agents_look_the_sun_up_first_and_read_the_street_sky_where_they_stand() {
        let none = crate::look::ShaderFeatures::NONE;
        let src = |f: crate::look::ShaderFeatures| crate::look::with_shader_features(f, super::agents_city_source);
        let soft = |s: &str| s.contains("const LK_SHADOW_SOFT: bool = true;");
        let pre = |s: &str| s.contains("const LK_SUN_PRE: bool = true;");
        let ol = |s: &str| s.contains("const SF_OCCLUDER_LIGHT: bool = true;");
        let street = |s: &str| s.contains("const AMB_OL_STREET: bool = true;");
        let first = |s: &str| s.contains("const AMB_OL_PRE: bool = true;");
        let occ = |s: &str| s.contains(&format!("const AMB_OCCLUDER: f32 = {:.6};", crate::ambient::OCCLUDER_OVER_GROUND));
        for cars in [false, true] {
            let s = src(crate::look::ShaderFeatures { soft_shadows: true, occluder_light: true, car_materials: cars, ..none });
            assert!(soft(&s) && pre(&s) && ol(&s) && !street(&s) && first(&s) && occ(&s), "cars' materials {cars}: the penumbra first, the occluder light from the point's own sample, worked out first");
        }
        let s = src(crate::look::ShaderFeatures { soft_shadows: true, car_materials: true, ..none });
        assert!(occ(&s) && !ol(&s), "without the occluder light, the stand-in as before");
        let s = src(crate::look::ShaderFeatures { car_materials: true, ..none });
        assert!(soft(&s) && !pre(&s), "(the constants are declared either way; the lookup in place without the penumbra)");
        // the city's own source looks the sun up in place and reads the street's sky apart.
        let city = crate::look::with_shader_features(crate::look::ShaderFeatures { soft_shadows: true, occluder_light: true, ..none }, || crate::look::city_shader_source_variant_probes(crate::ambient::ShaderLevels::Full, crate::look::ShadowLevels::Full, false));
        assert!(city.contains("const LK_SUN_PRE: bool = false;") && street(&city) && !first(&city));
        let plane = crate::look::with_shader_features(crate::look::ShaderFeatures { occluder_light: true, ..none }, || crate::look::city_shader_source_variant_probes(crate::ambient::ShaderLevels::Plane, crate::look::ShadowLevels::Near, false));
        let far = crate::look::with_shader_features(crate::look::ShaderFeatures { occluder_light: true, ..none }, crate::look::city_shader_source_far);
        assert!(!ol(&plane) && !ol(&far), "the props and the far layer keep the stand-in (their pipelines do not build with it)");
    }

    /// (`--people-capsules`) the shader's joints are eo-agents' capsule joints, in order.
    #[test]
    fn the_capsule_joints_are_the_libraries() {
        let src = include_str!("people_capsules.wgsl");
        let j = eo_agents::people::CAPSULE_JOINTS;
        let list = j.iter().map(|j| format!("{j}u")).collect::<Vec<_>>().join(", ");
        assert!(src.contains(&format!("const PK_JOINTS: array<u32, {}> = array<u32, {}>({list});", j.len(), j.len())), "PK_JOINTS");
        assert!(src.contains(&format!("const PK_PARTS: u32 = {}u;", j.len())), "PK_PARTS");
        assert!(src.contains(&format!("var<private> pk_a: array<vec4<f32>, {}>;", j.len())), "pk_a");
        // two packed ends a capsule and a radius byte each fit the eight flat words' 32
        assert!(2 * j.len() + j.len().div_ceil(4) <= 32);
    }

    /// (`--people-capsules`) the closed form the shader's `pk_sky` evaluates
    /// (the share of the cosine-weighted sky a capsule hides from a ground
    /// point) against a brute-force integral over the hemisphere: a standing
    /// body, a thigh, a shoe, at distances from beside it to 1.2 m.
    #[test]
    fn the_capsules_sky_share_is_the_hemisphere_integral() {
        use glam::Vec3;
        // the shader's closed form, transcribed
        let prim = |t: f32, c: f32| t / (c * (t * t + c * c)) + (t / c).atan() / (c * c);
        let closed = |g: Vec3, a: Vec3, b: Vec3, r: f32| {
            let ab = b - a;
            let len = ab.length();
            let u = if len > 1e-4 { ab / len } else { Vec3::Y };
            let ext = std::f32::consts::FRAC_PI_4 * r;
            let p0 = a - u * ext - g;
            let t0 = p0.dot(u);
            let t1 = t0 + len + 2.0 * ext;
            let perp = p0 - u * t0;
            let c = perp.length().max(r);
            let h = perp.y;
            let f = |t: f32| h * prim(t, c) - u.y * c / (t * t + c * c);
            (r / std::f32::consts::PI * (f(t1) - f(t0))).clamp(0.0, 1.0)
        };
        // the capsule's hit along a ray from g
        let hit = |g: Vec3, d: Vec3, a: Vec3, b: Vec3, r: f32| {
            let n = 400;
            (0..n).any(|i| {
                let s = (i as f32 + 0.5) / n as f32 * 3.0;
                let p = g + d * s;
                let ab = b - a;
                let t = ((p - a).dot(ab) / ab.length_squared().max(1e-9)).clamp(0.0, 1.0);
                (p - (a + ab * t)).length() < r
            })
        };
        let brute = |g: Vec3, a: Vec3, b: Vec3, r: f32| {
            // cosine-weighted directions on a stratified grid (Malley: a disc lifted onto the hemisphere)
            let k = 160;
            let mut hidden = 0usize;
            let mut all = 0usize;
            for i in 0..k {
                for j in 0..k {
                    let (x, z) = ((i as f32 + 0.5) / k as f32 * 2.0 - 1.0, (j as f32 + 0.5) / k as f32 * 2.0 - 1.0);
                    if x * x + z * z >= 1.0 {
                        continue;
                    }
                    all += 1;
                    let d = Vec3::new(x, (1.0 - x * x - z * z).sqrt(), z);
                    if hit(g, d, a, b, r) {
                        hidden += 1;
                    }
                }
            }
            hidden as f32 / all as f32
        };
        let body = (Vec3::new(0.0, 0.2, 0.0), Vec3::new(0.0, 1.5, 0.0), 0.16);
        let thigh = (Vec3::new(0.0, 0.5, 0.0), Vec3::new(0.05, 0.9, 0.08), 0.07);
        let shoe = (Vec3::new(0.0, 0.045, -0.08), Vec3::new(0.0, 0.045, 0.12), 0.045);
        for (name, (a, b, r)) in [("body", body), ("thigh", thigh), ("shoe", shoe)] {
            for d in [0.25f32, 0.4, 0.6, 0.9, 1.2] {
                let g = Vec3::new(d, 0.0, 0.03);
                let (c, f) = (closed(g, a, b, r), brute(g, a, b, r));
                eprintln!("{name} at {d} m: closed {c:.4}, integral {f:.4}");
                assert!((c - f).abs() < 0.005 + 0.1 * f, "{name} at {d} m: closed {c:.4}, integral {f:.4}");
            }
        }
        // a standing body hides a soft halo: about a tenth of the sky half a metre out, a few hundredths at a metre
        let at = |d: f32| closed(Vec3::new(d, 0.0, 0.0), body.0, body.1, body.2);
        assert!((0.05..0.2).contains(&at(0.5)) && at(1.0) < 0.06 && at(0.3) > at(0.5), "{} {} {}", at(0.3), at(0.5), at(1.0));
    }

    /// (`--people-contact`) the shader's joints are eo-agents' contact joints, in order.
    #[test]
    fn the_contact_joints_are_the_libraries() {
        let src = include_str!("people_contact.wgsl");
        let list = eo_agents::people::CONTACT_JOINTS.iter().map(|j| format!("{j}u")).collect::<Vec<_>>().join(", ");
        assert!(src.contains(&format!("const PC_JOINTS: array<u32, {}> = array<u32, {}>({list});", eo_agents::people::CONTACT_JOINTS.len(), eo_agents::people::CONTACT_JOINTS.len())), "PC_JOINTS");
        assert!(src.contains(&format!("const PC_PARTS: u32 = {}u;", eo_agents::people::CONTACT_JOINTS.len())), "PC_PARTS");
    }

    /// (`--car-lamps`) the light gather composes with the cars' rear lamps compiled in and out; its
    /// lamps' kind is a kind of its own and the streak pick's low-beam floor is the street's.
    #[test]
    fn the_rear_lamps_compile_in_and_out_of_the_gather() {
        let none = crate::look::ShaderFeatures::NONE;
        // (round 2 integration) with the night track's rear lamps and signals and the reflect
        // track's lamp streaks beside them: the cars' rear lamps lit once, streaked once.
        let all_rear = crate::look::ShaderFeatures { car_materials: true, car_lamps: true, cinematic: true, rear_lamps: true, signal_spill: true, lamp_units: true, lamp_streaks: true, ..none };
        for (on, f) in [(false, none), (true, crate::look::ShaderFeatures { car_materials: true, car_lamps: true, cinematic: true, ..none }), (false, crate::look::ShaderFeatures { cinematic: true, ..none }), (true, all_rear), (false, crate::look::ShaderFeatures { car_lamps: false, ..all_rear })] {
            crate::look::with_shader_features(f, || {
                // as sky.rs composes the gather's module (the night track's units, lenses and rear lamps with it)
                let src = format!("{}\n{}\nconst LT_TILE_M: f32 = 1.0;\n{}{}", include_str!("sky_common.wgsl"), crate::sky::sea_wgsl(), crate::shop_lights::wgsl() + &crate::sky::cine_lamps_wgsl(f.cinematic) + &crate::sky::night_photometry_wgsl() + &crate::sky::lamp_bounce_wgsl() + &crate::sky::lamp_units_wgsl(), include_str!("sky_lights.wgsl"));
                assert!(src.contains(&format!("const LT_CAR_LAMPS: bool = {on};")), "the constant follows the feature");
                use wgpu::naga;
                let m = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("the gather ({on}): {}", e.emit_to_string(&src)));
                naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).unwrap_or_else(|e| panic!("the gather ({on}): {}", e.emit_to_string(&src)));
            });
        }
        assert!(![0, 1, 3, crate::shop_lights::SHOP_LIGHT_KIND, crate::sky::BOUNCE_LIGHT_KIND, crate::sky::SIGNAL_LIGHT_KIND, crate::sky::REAR_LIGHT_KIND, crate::sky_reflect::LAMP_KIND].contains(&super::REAR_LAMP_KIND), "a kind of its own");
        assert!(include_str!("sky_light.wgsl").contains("const SKY_CINE_STREAK_HEAD_LENS: f32 = 0.06;"), "LT_HEAD_LENS_SHARE");
        assert!(super::car_lamps_wgsl().contains("const LT_HEAD_LENS_SHARE: f32 = 0.06;"));
        assert!(super::REAR_LAMP_FULL_DEG < super::REAR_LAMP_EDGE_DEG && super::TAIL_LAMP_CD < super::STOP_LAMP_CD);
        assert!(super::REAR_LAMP_SPREAD_V_DEG < super::REAR_LAMP_SPREAD_H_DEG && super::REAR_LAMP_FLOOR > 0.0 && super::REAR_LAMP_FLOOR < 0.2);
    }
}
