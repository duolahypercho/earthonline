//! Earth Online native slice.
//!
//!   eo-app                 windowed fly-through (WASD + mouse)
//!   eo-app --bench         offscreen 1920x1080 timing run, prints and writes JSON
//!
//! See native/README.md for every flag and for what is and is not drawn.

/// the render thread allocates from a heap of its own; the workers
/// from another (`eo_render::alloc`).
#[global_allocator]
static ALLOC: eo_render::alloc::EoAlloc = eo_render::alloc::EoAlloc;

mod agents; // agents
mod props; // props
mod agents_bench;
mod people_capture; // people
mod bench;
mod dive_sites;
mod cove_bed;
mod cc0_runtime; // the CC0 scanned sets at run time
mod farshore; // the far terrain's load and attach
mod models;
mod net; // net
mod quality; // `--quality` tiers and the lighting switches
mod streets;
mod street_life; // litter, graffiti, posters (`--street-life`): the level and its survey
mod structures; // grade-separated roads (`--structures`): build and attach
mod window;
mod audio; // the sound (eo-audio), see audio.rs
mod ui; // the interface (eo-ui): `--ui`, `--capture-ui`, `--ui-bench`; at the crate root so its tests run

use anyhow::Result;
use eo_assets::kit::{self, KitLibrary};
use eo_assets::placement::PlacementConstants;
use eo_data::{CaStandards, City, DataPaths, ElevationGrid, LandCover};
use eo_render::KitUpload;
use eo_render::authored::AuthoredSet;
use eo_render::kit::{KitRadii, KitSet};
use eo_render::scene::{Scene, SceneInputs, build_scene_with_cuts};
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct Args {
    pub bench: bool,
    pub start_underwater: bool,
    pub dive_site: Option<eo_render::seabed::Habitat>,
    pub underwater_habitats: bool,
    pub cove_detail: bool,
    pub black_point_intertidal: bool,
    pub surface_start: bool,
    /// `--quality` (default medium: today's frame); expanded into `look` once
    /// every argument is read (quality.rs), and the bench's frame budget.
    pub quality: quality::Quality,
    pub frames: usize,
    pub warmup: usize,
    pub width: u32,
    pub height: u32,
    pub msaa: u32,
    pub indirect_validation: bool,
    pub no_indirect_count: bool,
    /// Without MULTI_DRAW_INDIRECT_COUNT (Metal): the per-slot fallback instead of the count emulation.
    pub no_count_emulation: bool,
    pub vsync: bool,
    pub out_dir: Option<std::path::PathBuf>,
    pub label: Option<String>,
    pub capture: bool,
    /// Far plane, metres (default: the game camera's 4200 m).
    pub far: f32,
    /// Stress: draw every object this many times (1, 4, 16, ...).
    pub multiply: u32,
    /// Exit non-zero when a standard path's median frame exceeds the budget.
    pub check_budget: bool,
    /// Override of the 10 ms budget, only to prove the check fails (self-test).
    pub budget_ms: Option<f64>,
    /// Draw the Blender-built buildings' models (off: slice 1's footprints).
    pub models: bool,
    /// Coarse terrain past TERRAIN_LOD_DISTANCE_M (off: slice 1's full grid).
    pub terrain_lod: bool,
    /// Model cache directory (default native/cache).
    pub cache_dir: Option<std::path::PathBuf>,
    /// Do not read or write any cache: every model cooked from its GLB.
    pub no_cache: bool,
    pub stream_threads: usize,
    /// Models load only within this distance of the camera.
    pub stream_radius: f32,
    /// Bench only these segments (comma list).
    pub segments: Option<Vec<String>>,
    /// Slice 3: kit (archetype) buildings (off: slice 2's footprints for them).
    pub kit: bool,
    pub kit_threads: usize,
    /// One object per building, rung and material instead of per building and rung.
    pub kit_split_materials: bool,
    /// Bytes written to the GPU per frame at most (all streamed content).
    pub upload_budget: u64,
    /// Kit arena capacities: vertices, indices, objects.
    pub kit_vertices: u32,
    pub kit_indices: u32,
    pub kit_objects: u32,
    pub kit_radii: KitRadii,
    /// Frames of the `drive` path (it streams while it measures; may exceed 1024).
    pub drive_frames: usize,
    /// Slice 4: streets, sidewalks and grounded buildings (off: slice 3's ribbons and bases).
    pub streets: bool,
    /// clock hour (0..24) of the sun, sky and light rig; shadow quality.
    pub time_of_day: f64,
    pub look: eo_render::look::LookOptions,
    /// draw the simulated cars and people (and run the simulation); `--no-agents` turns it off.
    pub agents: bool,
    /// population multiplier (1 = 42 moving cars + People medium).
    pub agent_density: u32,
    /// the Rocketbox people walk the streets when a copy is at EO_PEOPLE_ROOT; `--no-rocketbox` draws no people.
    pub rocketbox: bool,
    /// street props, trees and parked cars (`--no-props` turns them off).
    pub props: bool,
    /// the furniture kinds, ez-tree trees, overhead, wires and their shading (`--no-props2` etc.).
    pub props2: props::Props2Args,
    /// occlusion / hitch: visibility culling and frame-hitch options.
    pub perf: PerfArgs,
    /// the bridges, shore edges, piers and open spaces (`--no-landmarks` turns them off).
    pub landmarks: bool,
    /// the waterfront, the landmark buildings, the park ground, the beach band and the bridges' lights
    /// (`--no-landmarks2` turns them all off, the A/B).
    pub landmarks2: bool,
    /// the grade-separated roads (`--structures`; `--tunnels` for the bores
    /// and portals): None (the default) builds none. A world switch: client
    /// and server must agree. Set by `quality::Switches::apply`.
    pub structures: Option<eo_structures::StructureOptions>,
    /// the facade clutter (`--facade-clutter [1|2]`, stage W3; eo-kit
    /// clutter.rs): fire escapes, awnings, signs, services and roof clutter on
    /// the kit buildings; 0 (the default) none. Set by `quality::Switches::apply`.
    pub facade_clutter: u8,
}

/// occlusion / hitch: one `Args` field, so `play.rs` and `ui.rs` name it once.
#[derive(Clone, Debug)]
pub struct PerfArgs {
    /// GPU occlusion culling (two-phase hierarchical Z); `--no-occlusion` for A/B.
    pub occlusion: bool,
    /// `--occlusion-check`: the ground-truth visibility test on every bench path.
    pub occlusion_check: bool,
    /// How the streaming drive gets real time (`--pace realtime|60hz|all-60hz|none`).
    pub pace: Pace,
    /// Render thread above normal, streaming workers below (`--no-thread-priority` for A/B).
    pub thread_priority: bool,
    /// Worst frames per path kept with their breakdown in the bench JSON.
    pub worst_frames: usize,
    /// `--check-max`: exit 1 when any path's worst frame reaches the budget.
    pub check_max: bool,
    /// streamed bulk writes through the staging ring (`--no-upload-ring` for A/B).
    pub upload_ring: bool,
    /// the bench steers the driver's periodic submission spike into
    /// its wait for the GPU (`eo_render::pace`; `--no-submit-pacing` for A/B).
    pub submit_pacing: bool,
    /// the best-quality layer (kit near packs, full street tiles,
    /// Blender models) only for what is in view (`--no-view-detail` for A/B;
    /// `--view-margin-deg`, `--view-predict-s`, `--view-hold-s`).
    pub view_detail: eo_render::view_detail::ViewDetailConfig,
    /// the bench streams during every path's measured frames
    /// (`--stream-all-paths`; by default only the streaming paths do).
    pub stream_all_paths: bool,
    /// today's arena sizes even with view detail on (`--full-arenas`;
    /// with it on the arenas are sized from its measured peaks, models.rs `VD_*`).
    pub full_arenas: bool,
}

/// the drive's camera moves at 60 km/h of wall time so the streamers
/// get real time. How its frames are paced:
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pace {
    /// (default) The camera is where the wall clock puts it; frames are
    /// rendered back to back, as an uncapped game loop renders. The GPU stays
    /// at its working clocks.
    Realtime,
    /// Each frame waits for its 1/60 s slot (before this change, the default).
    /// The GPU idles about three quarters of each frame and the driver drops
    /// it to a power-saving state (P3/P5: memory 810 MHz instead of 9751),
    /// so every pass measures several times slower.
    Hz60,
    /// Every path at 60 Hz (the experiment that shows the clock effect).
    All60,
    /// No pacing at all: the drive's camera advances one 1/60 s step a frame.
    None,
}

impl Default for PerfArgs {
    fn default() -> Self {
        Self {
            occlusion: true,
            occlusion_check: false,
            pace: Pace::Realtime,
            thread_priority: true,
            worst_frames: 12,
            check_max: false,
            upload_ring: true,
            submit_pacing: true,
            view_detail: eo_render::view_detail::ViewDetailConfig::on(),
            stream_all_paths: false,
            full_arenas: false,
        }
    }
}

fn apply_cove_look(args: &mut Args, raw: &[String]) {
    if args.cove_detail {
        args.look.cove_detail = true;
        args.look.water_optics = !raw.iter().any(|s| s == "--no-water-optics");
        args.look.dive_exposure = !raw.iter().any(|s| s == "--no-dive-exposure");
        args.look.sky.cove_cumulus = args.look.sky.clouds;
    }
}

fn parse_args() -> Result<Args> {
    parse_args_from(&std::env::args().skip(1).collect::<Vec<_>>())
}

fn parse_args_from(raw: &[String]) -> Result<Args> {
    let mut a = Args {
        bench: false,
        start_underwater: false,
        dive_site: None,
        underwater_habitats: false,
        cove_detail: false,
        black_point_intertidal: false,
        surface_start: false,
        quality: Default::default(),
        frames: 720,
        warmup: 60,
        width: 1920,
        height: 1080,
        msaa: 1,
        indirect_validation: true,
        no_indirect_count: false,
        no_count_emulation: false,
        vsync: true,
        out_dir: None,
        label: None,
        capture: false,
        far: eo_render::camera::GAME_FAR_M,
        multiply: 1,
        check_budget: false,
        budget_ms: None,
        models: true,
        terrain_lod: true,
        cache_dir: None,
        no_cache: false,
        stream_threads: 6,
        stream_radius: f32::INFINITY,
        segments: None,
        kit: true,
        kit_threads: 6,
        kit_split_materials: false,
        upload_budget: 4 << 20,
        kit_vertices: 16_000_000,
        kit_indices: 26_000_000,
        kit_objects: 32_768,
        kit_radii: KitRadii::default(),
        drive_frames: 3600,
        streets: true,
        time_of_day: eo_render::look::DEFAULT_HOUR,
        look: Default::default(),
        agents: true, // agents
        agent_density: 1,
        rocketbox: true, // people
        props: true, // props
        props2: Default::default(),
        perf: Default::default(), // occlusion / hitch
        landmarks: true,          // landmarks
        landmarks2: true,
        structures: None, // `--structures` (quality::Switches)
        facade_clutter: 0, // `--facade-clutter` (quality::Switches)
    };
    let mut tier = quality::Switches::default(); // `--quality` and the lighting switches, applied after the loop
    let mut it = raw.iter();
    while let Some(s) = it.next() {
        if let Some(n) = tier.take(s, it.as_slice().first().map(String::as_str))? {
            it.by_ref().take(n).for_each(drop); // the value it took, if any
            continue;
        }
        let mut val = || it.next().cloned().ok_or_else(|| anyhow::anyhow!("{s} needs a value"));
        match s.as_str() {
            "--bench" => a.bench = true,
            "--frames" => a.frames = val()?.parse()?,
            "--warmup" => a.warmup = val()?.parse()?,
            "--width" => a.width = val()?.parse()?,
            "--height" => a.height = val()?.parse()?,
            "--msaa" => a.msaa = val()?.parse()?,
            "--no-indirect-validation" => a.indirect_validation = false,
            "--no-indirect-count" => a.no_indirect_count = true,
            "--no-count-emulation" => a.no_count_emulation = true,
            "--no-vsync" => a.vsync = false,
            "--out" => a.out_dir = Some(val()?.into()),
            "--label" => a.label = Some(val()?),
            "--capture" => a.capture = true,
            "--far" => a.far = val()?.parse()?,
            "--multiply" => a.multiply = val()?.parse::<u32>()?.max(1),
            "--check-budget" => a.check_budget = true,
            "--budget-ms" => a.budget_ms = Some(val()?.parse()?),
            "--no-models" => a.models = false,
            "--no-terrain-lod" => a.terrain_lod = false,
            "--no-kit" => a.kit = false,
            "--slice1" => {
                a.models = false;
                a.terrain_lod = false;
                a.kit = false;
            }
            "--slice2" => {
                a.kit = false;
                a.streets = false;
            }
            "--slice3" | "--no-streets" => a.streets = false,
            "--cache" => a.cache_dir = Some(val()?.into()),
            "--no-cache" => a.no_cache = true,
            "--stream-threads" => a.stream_threads = val()?.parse::<usize>()?.max(1),
            "--stream-radius" => a.stream_radius = val()?.parse()?,
            "--kit-threads" => a.kit_threads = val()?.parse::<usize>()?.max(1),
            "--kit-split-materials" => {
                a.kit_split_materials = true;
                a.kit_objects = a.kit_objects.max(524_288);
            }
            "--upload-budget-mb" => a.upload_budget = (val()?.parse::<f64>()? * 1048576.0) as u64,
            "--kit-arena-vertices" => a.kit_vertices = val()?.parse()?,
            "--kit-arena-indices" => a.kit_indices = val()?.parse()?,
            "--kit-arena-objects" => a.kit_objects = val()?.parse()?,
            "--kit-near-load" => a.kit_radii.near_load = val()?.parse()?,
            "--kit-mid-load" => {
                a.kit_radii.mid_load = val()?.parse()?;
                a.kit_radii.mid_drop = a.kit_radii.mid_load + 80.0;
            }
            "--drive-frames" => a.drive_frames = val()?.parse()?,
            // look
            "--time-of-day" => a.time_of_day = val()?.parse()?,
            "--shadow-map" => a.look.shadow_map = val()?.parse::<u32>()?.clamp(256, 8192),
            "--no-shadows" => a.look.shadows = false,
            "--no-ao" => a.look.ao = false, // no occlusion + bounce pass
            "--no-facade-age" => a.look.facade &= !eo_render::facade::RUN_AGE, // facade
            "--no-facade-detail" => a.look.facade &= !eo_render::facade::RUN_DETAIL, // facade
            "--no-atlas2" => a.look.facade &= !eo_render::facade::RUN_ATLAS2, // facade
            "--no-probe" => a.look.facade &= !eo_render::facade::RUN_PROBE, // facade
            "--no-rooms" => a.look.facade &= !eo_render::facade::RUN_ROOMS, // no room behind kit glazing
            "--no-room-frames" | "--no-room-dim" => {} // read by eo_render::rooms::wgsl_constants
            // near kit buildings as their envelope box ("box"), or the box with the shader-only facade ("shader").
            "--facade-test" => match val()?.as_str() {
                "box" => eo_render::kit::FACADE_TEST.store(1, std::sync::atomic::Ordering::Relaxed),
                "shader" => {
                    eo_render::kit::FACADE_TEST.store(2, std::sync::atomic::Ordering::Relaxed);
                    a.look.facade |= eo_render::facade::RUN_FACADE_TEST;
                }
                other => anyhow::bail!("--facade-test takes box or shader, not {other}"),
            },
            "--no-agent-occlusion" => a.look.facade &= !eo_render::facade::RUN_AGENT_HZB, // agents not tested against the pyramid
            "--no-probe-defer" => a.look.facade &= !eo_render::facade::RUN_PROBE_DEFER, // a probe face on any frame
            "--probe-yield" => a.look.facade |= eo_render::facade::RUN_PROBE_YIELD, // opt in
            // weather (a preset or the shared schedule) and the stage switches.
            "--weather" => {
                let v = val()?;
                a.look.sky.weather = match v.as_str() {
                    "schedule" | "auto" => eo_render::sky::WeatherMode::Schedule,
                    k => eo_render::sky::WeatherMode::Preset(eo_sky::Kind::parse(k).ok_or_else(|| anyhow::anyhow!("--weather clear|overcast|fog|drizzle|rain|storm|damp|schedule, not {k}"))?),
                }
            }
            "--weather-seed" => a.look.sky.seed = val()?.parse()?,
            "--world-time" => a.look.sky.world_time_s = val()?.parse()?,
            "--moon-day" => a.look.sky.moon_day = val()?.parse()?,
            "--sky-quality" => {
                a.look.sky.quality = match val()?.as_str() {
                    "low" => 0,
                    "high" => 2,
                    _ => 1,
                }
            }
            "--no-sky" => a.look.sky = eo_render::sky::SkyOptions { weather: a.look.sky.weather, ..eo_render::sky::SkyOptions::off() },
            "--no-clouds" => a.look.sky.clouds = false,
            "--no-cloud-shadows" => a.look.sky.cloud_shadows = false,
            "--cloud-depth-haze" => a.look.sky.cloud_depth_haze = true,
            "--no-cloud-depth-haze" => a.look.sky.cloud_depth_haze = false,
            "--no-fog" => a.look.sky.fog = false,
            "--no-shafts" => a.look.sky.shafts = false,
            "--no-moon-stars" => a.look.sky.moon_stars = false,
            "--no-lights" => a.look.sky.lights = false,
            "--no-lamp-sprites" => a.look.sky.sprites = false,
            "--no-wet" => a.look.sky.wet = false,
            "--no-rain" => a.look.sky.rain = false,
            "--no-ssr" => a.look.sky.ssr = false,
            "--no-taa" => a.look.sky.taa = false,
            "--no-bloom" => a.look.sky.bloom = false,
            "--no-adaptation" => a.look.sky.adaptation = false,
            "--no-meter" => a.look.sky.meter = false, // the camera meter off (the hour's exposure curve alone)
            "--no-atmosphere" => a.look.sky.atmosphere = false, // the JS analytic dome instead of the atmosphere tables
            "--no-contact-shadows" => a.look.sky.contact = false,
            "--sky-shots" => {} // bench/sky_shots.rs reads it
            "--light-shots" => {} // bench/light_shots.rs reads it (the lighting reference cards)
            "--structures-shots" => {} // bench/structures_shots.rs reads it (the grade-separated roads' capture card)
            "--dusk-strip" => {} // bench/light_shots.rs reads it (the drawn hour across sunset and sunrise)
            "--so-shots" => {} // so: bench/so_shots.rs reads it (the sky and ocean review cards)
            "--no-farshore" => {} // farshore.rs reads it (no far terrain)
            "--start-underwater" => a.start_underwater = true,
            "--dive-site" => {
                let s_site = val()?;
                a.surface_start = false;
                a.dive_site = Some(match s_site.as_str() {
                    "bay" | "bay-surface" => {
                        a.surface_start = s_site == "bay-surface";
                        eo_render::seabed::Habitat::Eelgrass
                    },
                    "pacific" => eo_render::seabed::Habitat::Sand,
                    "black-point" => eo_render::seabed::Habitat::Rocky,
                    other => anyhow::bail!("unknown dive site {other}; choose bay, bay-surface, black-point or pacific"),
                });
            }
            "--water-optics" => a.look.water_optics = true,
            "--cove-detail" => a.cove_detail = true,
            "--cove-canopy" => { a.cove_detail = true; a.look.cove_canopy = true; }
            "--no-cove-canopy" => a.look.cove_canopy = false,
            "--cove-sediment" => { a.cove_detail = true; a.look.cove_sediment = true; }
            "--no-cove-sediment" => a.look.cove_sediment = false,
            "--black-point-intertidal" => { a.cove_detail = true; a.black_point_intertidal = true; }
            "--no-black-point-intertidal" => a.black_point_intertidal = false,
            "--no-cove-detail" => a.cove_detail = false,
            "--underwater-habitats" => a.underwater_habitats = true,
            "--no-underwater-habitats" => a.underwater_habitats = false,
            "--no-water-optics" => a.look.water_optics = false,
            "--dive-exposure" => a.look.dive_exposure = true,
            "--no-dive-exposure" => a.look.dive_exposure = false,
            "--no-water" => a.look.water = false, // the flat sea quad, as before
            // agents
            "--no-agents" => a.agents = false,
            "--no-props" => a.props = false, // props
            "--no-landmarks" => a.landmarks = false, // landmarks
            // the CC0 scanned sets (all, or one feature at a time for A/B); `--cc0-shots` is read by bench/cc0_shots.rs.
            "--no-cc0" => a.look.cc0 = 0,
            "--no-cc0-walls" => a.look.cc0 &= !eo_render::cc0::RUN_WALLS,
            "--no-cc0-street" => a.look.cc0 &= !eo_render::cc0::RUN_STREET,
            "--no-cc0-ground" => a.look.cc0 &= !eo_render::cc0::RUN_GROUND,
            "--no-cc0-decals" => a.look.cc0 &= !eo_render::cc0::RUN_DECALS,
            "--no-cc0-pom" => a.look.cc0 &= !eo_render::cc0::RUN_POM,
            "--no-cc0-tiling" => a.look.cc0 &= !eo_render::cc0::RUN_TILING,
            "--no-cc0-grime" => a.look.cc0 &= !eo_render::cc0::RUN_GRIME,
            "--no-cc0-stains" => a.look.cc0 &= !eo_render::cc0::RUN_STAINS,
            "--cc0-shots" => {}
            "--clutter-shots" => {} // the facade clutter's capture card (bench/clutter_shots.rs)
            "--no-landmarks2" => a.landmarks2 = false,
            "--bridge-detail" | "--no-bridge-detail" => {} // read where the landmark set is built (`bridge_detail`)
            "--no-audio" | "--audio-null" => {} // read by audio::AudioRuntime::start
            s if a.props2.parse_flag(s) => {} // --no-props2, --no-furniture, --no-eztree, --no-overhead, --no-wires, --no-props2-shading, --capture-props2
            "--agents-bench" | "--agents-capture" | "--people-capture" | "--people-lineup" => {} // + --people-capture, --people-lineup
            "--no-rocketbox" => a.rocketbox = false, // people
            "--no-rocketbox-motions" => eo_agents::motion::set_enabled(false), // no motion, so no people
            // read by eo_render::hlod::HlodOptions::from_env and bench/hlod_shots.rs
            "--hlod-shots" => {}
            "--far-shots" | "--far-diag" => {} // bench/far_shots.rs reads them
            "--gs-shots" => {} // bench/gs_shots.rs reads it
            s if eo_render::hlod::HlodOptions::FLAGS.iter().any(|(f, _)| *f == s) => {
                if eo_render::hlod::HlodOptions::FLAGS.iter().any(|(f, v)| *f == s && *v) {
                    val()?;
                }
            }
            "--ab-rounds" | "--densities" | "--ab-frames" | "--people-frames" | "--people-strip" => { // + --people-frames; + --people-strip
                val()?;
            }
            "--agent-density" => a.agent_density = val()?.parse::<u32>()?.clamp(1, 64),
            // read by `groundscale_flags` (main) into the environment switches.
            "--ground-scale" | "--height-scale" | "--people-scale" | "--vehicle-scale" => {
                val()?;
            }
            "--low-camera" | "--default-camera" => {}
            // read by `roofs_flags` (main) into the environment switches.
            "--no-roof-colours" | "--no-roof-contrast" | "--no-roof-parapet" | "--no-roof-patches" | "--no-roof-plant" => {}
            "--no-wall-paint" | "--no-ground-green" | "--no-ground-hills" | "--no-ground-contact" => {} // `roofs_flags`
            "--segments" => a.segments = Some(val()?.split(',').map(|s| s.trim().to_string()).collect()),
            // occlusion / hitch
            "--no-occlusion" => a.perf.occlusion = false,
            "--occlusion-check" => a.perf.occlusion_check = true,
            "--no-thread-priority" => a.perf.thread_priority = false,
            "--worst-frames" => a.perf.worst_frames = val()?.parse()?,
            "--check-max" => a.perf.check_max = true,
            "--no-upload-ring" => a.perf.upload_ring = false,
            "--no-submit-pacing" => a.perf.submit_pacing = false,
            "--no-view-detail" => a.perf.view_detail.enabled = false,
            "--view-margin-deg" => a.perf.view_detail.margin_deg = val()?.parse()?,
            "--view-predict-s" => a.perf.view_detail.predict_s = val()?.parse()?,
            "--view-hold-s" => a.perf.view_detail.hold_s = val()?.parse()?,
            "--stream-all-paths" => a.perf.stream_all_paths = true,
            "--full-arenas" => a.perf.full_arenas = true,
            "--pace" => {
                a.perf.pace = match val()?.as_str() {
                    "realtime" => Pace::Realtime,
                    "60hz" => Pace::Hz60,
                    "all-60hz" => Pace::All60,
                    "none" => Pace::None,
                    other => anyhow::bail!("--pace realtime|60hz|all-60hz|none, not {other}"),
                }
            }
            "-h" | "--help" => {
                println!(
                    "eo-app [--dive-site bay|bay-surface|black-point|pacific] [--cove-detail|--no-cove-detail] [--cove-canopy|--no-cove-canopy] [--cove-sediment|--no-cove-sediment] [--black-point-intertidal|--no-black-point-intertidal] [--water-optics] [--dive-exposure|--no-dive-exposure] [--underwater-habitats|--no-underwater-habitats] [--bench] [--frames N=720] [--warmup N=60] [--width 1920] [--height 1080] [--msaa 1|4]\n       [--no-indirect-validation] [--no-indirect-count] [--no-count-emulation] [--no-vsync] [--out DIR] [--label NAME] [--capture] [--far M=4200]\n       [--multiply N=1] [--check-budget] [--budget-ms 10 (self-test only)]\n       [--no-models] [--no-terrain-lod] [--no-kit] [--slice1] [--slice2] [--cache DIR] [--no-cache] [--stream-threads N=6] [--stream-radius M=inf]\n       [--kit-threads N=6] [--kit-split-materials] [--upload-budget-mb 4] [--kit-arena-vertices N] [--kit-arena-indices N] [--kit-arena-objects N]\n       [--kit-near-load M=110] [--kit-mid-load M=520] [--drive-frames N=3600]\n       [--time-of-day H=12] [--shadow-map N=3072] [--no-shadows] [--no-agents] [--agent-density N=1] [--no-rocketbox]\n       [--no-occlusion] [--occlusion-check] [--no-thread-priority] [--pace realtime|60hz|all-60hz|none] [--worst-frames N=12] [--check-max]\n       [--no-view-detail] [--view-margin-deg 35] [--view-predict-s 0.3] [--view-hold-s 2.5] [--stream-all-paths] [--full-arenas]\n       [--segments street_walk,elevated_sweep,city_overview,authored_street,kit_street,drive,turn_around,turn_around_authored]\n       (with --structures and built decks also freeway_drive,under_viaduct) [--light-shots] (capture cards; EO_LIGHT_SHOTS=a,b; EO_LIGHT_SHOTS_HOUR=H every card at another hour; EO_LIGHT_SHOTS=hero the six key views, clock held)\n       [--cc0-shots] (the scanned sets' matched cards; EO_CC0_SHOTS=a,b; EO_CC0_SHOTS_HOUR=H every card at another hour)\n       [--clutter-shots] (the facade clutter's cards: the kit and authored streets, fronts with fire escapes and shop fronts the planner dresses, a roof; the same poses with or without --facade-clutter; EO_CLUTTER_SHOTS=a,b; EO_CLUTTER_SHOTS_HOUR=H, default 15)\n       [--structures-shots] (the grade-separated roads at four places, from the street and from above, 15:00 clear; the same poses with or without --structures; EO_STRUCTURES_SHOTS=a,b; EO_STRUCTURES_SHOTS_HOUR=H another hour; EO_STRUCTURES_SHOTS_HDR=1 the scene luminance too)\n       [--dusk-strip] (40 frames a clock minute apart across sunset and sunrise at one camera, a CSV of mean luminance; with --smooth-time through the soft path)\n       (--light-shots with EO_PROBE_STRIP=path[,frames[,m a frame[,turn]]]: the reflections along a street while probe captures finish, a CSV and the frames round each)\n\nQuality and lighting (opt-in; a switch wins over the tier wherever it stands; each has a --no- form):\n       [--quality low|medium|high|hyper] (default medium, today's frame; bench budget 8 / 10 / 16.6 / 33.3 ms;\n       low: a 2048 sun map, low sky, no SSR, contact shadows, AO, shafts or bloom; high: the lighting additions but the probe grid,\n       with grime, street life, stand-ins, photo finish, facade clutter and shop light at 1 and the cinematic look; hyper: high at level 2, 48 lamps, the probe grid 2)\n       [--cascade-blend] [--far-shadows [1|2]] [--ambient-volume [1|2]] [--probe-grid [1|2]] [--light-shadows [N lamps, 16..48]]\n       [--light-shadow-moving] [--headlight-shadows [N vehicles]] [--lit-fog [1|2]]\n       (--ambient-volume: narrow streets and the ground under decks get less sky light, baked round the camera and over the city)\n       [--ground-bounce] (with --ambient-volume, on unless --no-ground-bounce: light bounced up from the sunlit ground onto deck undersides and walls facing it)\n       [--smooth-time] (the drawn hour follows the simulation's clock, the exposure converges instead of snapping)\n       (--far-shadows: sun shadows past the cascades from cached maps round the camera, 1 a 2 km ring, 2 another to the far plane; --cascade-blend: the cascades hand over in a band instead of a line)\n       (--probe-grid: reflection probes on the streets round the camera, cross-faded, read by every lit surface, 1 four of 128 px, 2 nine of 256 px)\n       (--lit-fog: the fog lit by the street lamps and headlamps, a froxel volume added by the resolve; 1 160 x 90 x 64, 2 240 x 135 x 96; nothing by day)\n       (--light-shadows: the nearest N street lamps keep a shadow map, drawn once, at most 2 new a frame, and draw in SF's 3000 K LED colour; --light-shadow-moving: the 4 nearest redrawn every frame with the traffic; --headlight-shadows: the nearest N vehicles' low beams cast shadows; nothing by day)\n       [--shadow-slope-bias] (the sun cascades' casters with a slope-scaled depth bias: no self-shadow stripes where the sun grazes a surface; high and hyper turn it on)\n       [--grime [1|2]] (surface weathering that reads: soiled walks with gum and stains, chipped kerbs and faded kerb paint, patched and sealed carriageways, washed-down facades, dusty cars; high 1, hyper 2)\n       [--street-life [1|2]] (litter, butts, leaves and grit on the walks and in the gutters; graffiti, buffed patches and posters on ground floors and freeway columns; 2 adds stickers, utility marks, pigeon droppings; high 1, hyper 2)\n       [--stand-ins] (procedural models for street-prop kinds whose GLB is absent from the checkout, instead of grey boxes; high and hyper)\n       [--photo-finish [1|2]] (a camera instead of the JS display transform: highlights pale to white on their hue, ISO noise by the scene's light, lens fall-off and lateral colour, a lens's bloom; San Francisco's measured air in the sky and the haze; 2 smoother fringes and chroma noise; high 1, hyper 2)\n       [--facade-clutter [1|2]] (fire escapes, awnings, colour-field and neon signs, roll-down shutters, bars on older ground-floor windows, downspouts, meters and conduit, chimneys, vents, dishes and aerials on the kit buildings; signs lit by night where the shop is; 2 adds window air conditioners, front dishes, scaffolding, boarded shops; high 1, hyper 2)\n       [--shop-light [1|2]] (lit shop windows at night: a light behind every kit shop window in its trade's colour, at its window's luminance, by its opening hours, spilling onto the sidewalk and lighting the fog, the room behind the glass lit or dark with it; 1 within 160 m, 2 to 300 m with two lights a window; high 1, hyper 2)\n       [--look default|cinematic] (the look profile: default is today's frame on its physical references; cinematic a graded open-world look, its departures from those references listed in CINEMATIC_DEVIATIONS; high and hyper; --no-look is --look default)\n       (a level left out is the high tier's; --probe-grid, which high leaves off, takes 1)\n\nWorld (opt-in; client and server must agree, the handshake refuses a mismatch; each has a --no- form):\n       [--structures] (grade-separated roads: freeways, ramps, road bridges, drawn and cast in three levels of detail) [--tunnels] (bores and portals, with --structures)"
                );
                std::process::exit(0);
            }
            other => anyhow::bail!("unknown argument {other} (try --help)"),
        }
    }
    tier.apply(&mut a); // the tier, then the lighting switches over it
    if a.dive_site.is_some() {
        a.look.water_optics = !raw.iter().any(|flag| flag == "--no-water-optics");
        a.look.dive_exposure = !raw.iter().any(|flag| flag == "--no-dive-exposure");
        a.underwater_habitats = !raw.iter().any(|flag| flag == "--no-underwater-habitats");
    }
    if matches!(a.dive_site, Some(eo_render::seabed::Habitat::Eelgrass | eo_render::seabed::Habitat::Rocky)) {
        a.cove_detail = !raw.iter().any(|s| s == "--no-cove-detail");
        a.look.cove_canopy = a.cove_detail && !raw.iter().any(|s| s == "--no-cove-canopy");
        a.look.cove_sediment = a.cove_detail && !raw.iter().any(|s| s == "--no-cove-sediment");
        a.black_point_intertidal = a.cove_detail && !raw.iter().any(|s| s == "--no-black-point-intertidal");
    }
    apply_cove_look(&mut a, raw);
    if a.frames + a.warmup > 1024 {
        // 4 timestamp queries per frame; one query set holds at most 4096.
        anyhow::bail!("--frames + --warmup must be <= 1024 per segment");
    }
    Ok(a)
}

/// `--bridge-detail`: the Golden Gate's detail pass, a preview that is off by
/// default (`--no-bridge-detail` keeps it off whatever else is passed).
pub fn bridge_detail() -> bool {
    let has = |f: &str| std::env::args().any(|a| a == f);
    has("--bridge-detail") && !has("--no-bridge-detail")
}

pub struct World {
    pub paths: DataPaths,
    pub city: Arc<City>,
    pub elevation: ElevationGrid,
    pub scene: Scene,
    pub authored: Arc<AuthoredSet>,
    /// The kit bake, running on its own thread; None when nothing textured draws.
    pub kit_rx: Option<Receiver<Result<(KitUpload, f64), String>>>,
    /// Slice 3: the kit buildings (groups, tiles, palette) and the planner's
    /// city, being built on its own thread.
    pub kit: Option<Arc<KitSet>>,
    pub kit_city_rx: Option<Receiver<Result<(Arc<eo_kit::city::KitCity>, String), String>>>,
    /// Slice 4: street tiles (groups after the kit's) and the street world.
    pub street: Option<Arc<eo_render::street::StreetSet>>,
    pub load_ms: LoadTimes,
    /// the street's scanned sets (public/assets), and whether they came from the cache.
    pub street_maps: Option<(Arc<eo_assets::street_maps::StreetMaps>, bool)>,
    /// the facade-detail-v1 sheet (public/assets, verified, cached decoded).
    pub facade_detail: Option<Arc<eo_assets::facade_detail::FacadeDetail>>,
    /// the citywide shore field and the FFT spectrum (drawn unless `--no-water`).
    pub water: Option<Arc<eo_water::WaterData>>,
    pub dive_sites: Vec<dive_sites::Site>,
    /// what the scene build added (LOD groups after the street's, the wire members).
    pub landmarks: Option<Arc<eo_render::landmarks::LandmarkStatic>>,
    /// the far layer's cells (every building drawn as a building, in one cell each).
    pub hlod: Option<Arc<eo_render::hlod_bake::HlodSet>>,
    /// the distant trees, gathered on their own thread during the load.
    pub hlod_trees: Option<Receiver<eo_render::hlod_trees::FarTreeList>>,
    /// the grade-separated roads (`--structures` with streets; built beside
    /// the load, structures.rs), shared by the renderer, the player's ground,
    /// the agents' feed, the props world and the in-process server. None without the switch.
    pub structures: Option<Arc<eo_structures::StructureWorld>>,
    /// what the renderer draws of them (groups, materials, the static far
    /// and coarse levels, the arena they need); None without the switch or
    /// for an empty world.
    pub structures_drawn: Option<Arc<eo_render::structures::StructurePrepared>>,
}

/// the JS street ground materials' scanned sets, as
/// `STREET_SURFACE_PBR_SETS` names them (eo-street/data/street-material.json).
fn spawn_street_maps(repo_root: std::path::PathBuf, cache: Option<std::path::PathBuf>) -> std::thread::JoinHandle<Result<(eo_assets::street_maps::StreetMaps, bool)>> {
    std::thread::spawn(move || {
        let m = eo_street::material::StreetMaterial::get();
        let (a, c) = (&m.pbr_sets.carriageway, &m.pbr_sets.concrete);
        let sets = [
            (a.id.as_str(), a.provenance.as_str(), a.diffuse.as_str(), a.nra.as_str()),
            (c.id.as_str(), c.provenance.as_str(), c.diffuse.as_str(), c.nra.as_str()),
        ];
        eo_assets::street_maps::load_cached(&repo_root, &sets, cache.as_deref())
    })
}

#[derive(Clone, Copy, Debug, Default)]
pub struct LoadTimes {
    pub city_ms: f64,
    pub elevation_ms: f64,
    pub land_cover_ms: f64,
    pub authored_prepare_ms: f64,
    pub kit_prepare_ms: f64,
    pub scene_build_ms: f64,
}

pub fn cache_dir(args: &Args) -> Option<std::path::PathBuf> {
    if args.no_cache {
        return None;
    }
    Some(args.cache_dir.clone().unwrap_or_else(eo_assets::default_cache_dir))
}

pub fn load_world(args: &Args) -> Result<World> {
    let street_plan = args.streets.then(streets::spawn_plan);
    let paths = DataPaths::discover()?;
    let street_maps_job = args.streets.then(|| spawn_street_maps(paths.repo_root.clone(), cache_dir(args)));
    // the detail sheet, decoded (or read from the cache) beside the load.
    let facade_job = {
        let (root, cache) = (paths.repo_root.clone(), cache_dir(args));
        std::thread::spawn(move || eo_assets::facade_detail::load_cached(&root, cache.as_deref()))
    };
    // the shore field (baked from sf-city.json's boundary, or read from the cache) and the spectrum, beside the load
    // (also with `--no-water`: the bench's water paths are placed on the field; nothing is drawn from it then).
    farshore::spawn(&paths.repo_root, cache_dir(args)); // the far terrain, beside the load
    let water_job = Some({
        let (root, cache) = (paths.repo_root.clone(), cache_dir(args));
        std::thread::spawn(move || eo_water::load(&root, cache.as_deref()))
    });
    let t = Instant::now();
    let city = Arc::new(City::load(&paths)?);
    let city_ms = t.elapsed().as_secs_f64() * 1e3;
    let t = Instant::now();
    let elevation = ElevationGrid::load(&paths)?;
    let elevation_ms = t.elapsed().as_secs_f64() * 1e3;
    // the bridges and the citywide shore edge, built beside the load on the drawn ground.
    let landmark_job = args.landmarks.then(|| {
        let (g, p) = (elevation.clone(), paths.clone());
        // `--no-landmarks2` builds the set as the first landmarks slice did.
        let opts = if args.landmarks2 { eo_landmarks::set::Options::default() } else { eo_landmarks::set::Options::off() };
        let opts = eo_landmarks::set::Options { bridge_detail: bridge_detail(), ..opts };
        let opts = eo_landmarks::set::Options { cove_detail: args.cove_detail, ..opts };
        std::thread::spawn(move || -> Result<eo_landmarks::LandmarkSet> {
            let ca = CaStandards::load(&p)?;
            eo_landmarks::LandmarkSet::build_with(&p.repo_root, &|x: f64, z: f64| g.sample(x, z), &ca, opts)
        })
    });
    // the open spaces' ground (the greens' tones, beach sand) beside the load.
    let cover_job = (args.landmarks && args.landmarks2).then(|| {
        let root = paths.repo_root.clone();
        std::thread::spawn(move || eo_landmarks::ground_cover::GroundCover::load(&root))
    });
    let t = Instant::now();
    let land = LandCover::load(&paths).ok();
    let land_cover_ms = t.elapsed().as_secs_f64() * 1e3;
    let street_job = street_plan.map(|p| streets::spawn(p, &city, &elevation));
    let standards = CaStandards::load(&paths)?;
    let mut kit_rx = None;
    let kit_lib = if args.models || args.kit { Some(KitLibrary::load()?) } else { None };
    let mut authored = if args.models {
        let constants = PlacementConstants::load()?;
        AuthoredSet::prepare(&paths.repo_root, &city, &elevation, kit_lib.as_ref().unwrap(), &constants)?
    } else {
        AuthoredSet::empty()
    };
    let street_world = match street_job {
        Some(j) => Some(j.join().map_err(|_| anyhow::anyhow!("street world thread panicked"))??),
        None => None,
    };
    // the grade-separated roads over the street world (`--structures`), built beside the rest of the load.
    let mut structures_job = street_world.as_ref().and_then(|sw| structures::spawn(args.structures, sw, &city, &standards));
    // with them the street is built again with their joins (stage A6:
    // junction legs, capped, raised and lowered stubs): that street world is
    // the one everything below stands on (street tiles, kit floors, terrain
    // carpet, plazas, shore, props, player, agents), as eo-net's server
    // builds it. Without `--structures` the first one stands, untouched.
    let street_world = match (structures_job.as_mut(), street_world) {
        (Some(j), Some(sw)) => Some(j.streets(sw)?),
        (_, sw) => sw,
    };
    // the paved plazas, planned and laid at the street world's
    // footway datum (plaza-paving.js), and the Ocean Beach and Sunset Dunes
    // seawalls on the drawn land and beach (the carpet), beside the rest of the load.
    let plaza_job = match (&street_world, args.landmarks) {
        (Some(sw), true) => {
            let (sw, city, root) = (sw.clone(), city.clone(), paths.repo_root.clone());
            let bay_tones = args.landmarks2;
            Some(std::thread::spawn(move || -> Result<(eo_landmarks::LandmarkSet, eo_landmarks::set::PlazaReport, [(usize, usize); 2])> {
                let fps: Vec<Vec<[f64; 2]>> = city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
                // The footway datum (`curbTopY(datum) + curbTopFall` = ground + footwayLift), which
                // stands well over the carpet (ground + openGroundLift) the JS also keeps it above.
                let o = &sw.o;
                let mut s = eo_landmarks::LandmarkSet::default();
                let r = s.add_plazas_with(&root, &|x, z| sw.buildings.contains(x, z), &fps, &|x, z| sw.ground(x, z) + o.footway_lift, &|x, z| sw.paved_at(x, z), o.verge_max_drop, bay_tones)?; // bay tones
                let carpet = |x: f64, z: f64| sw.carpet_y(x, z);
                let pr = s.add_promenades(&carpet, &carpet);
                Ok((s, r, pr))
            }))
        }
        _ => None,
    };
    // the building records the landmark buildings draw instead.
    let mut landmark_claims: HashSet<String> = if args.landmarks && args.landmarks2 { eo_landmarks::set::claimed_records() } else { HashSet::new() };
    // stage A9 (`--tunnels`): a building over a tunnel's approach cut or portal canopy is
    // left out as a claimed one (none without tunnels: the claims and every key as before).
    if let Some(w) = structures_job.as_ref().and_then(|j| j.world()).filter(|w| !w.terrain_cuts().is_empty()) {
        let rings: Vec<Vec<[f64; 2]>> = city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
        landmark_claims.extend(w.buildings_over_cuts(&rings).into_iter().map(|i| city.buildings[i].id.trim_start_matches("sf-building-").to_string()));
    }
    // Kit buildings: every building the authored registry does not draw.
    let t = Instant::now();
    let mut kit_set = None;
    let mut kit_city_rx = None;
    if args.kit {
        let mut excluded: HashSet<String> = authored.group_of_record.keys().cloned().collect();
        for id in eo_kit::city::load_exclusions() {
            excluded.insert(id.trim_start_matches("sf-building-").to_string());
        }
        let group_base = authored.groups.len() as u32;
        let mut set = KitSet::prepare(&city, &elevation, &excluded, group_base, &mut authored.materials, kit_lib.as_ref().unwrap());
        // the records a landmark building draws (its `claims`)
        // keep their kit group (the cached tile packs carry them), which never
        // draws a level. The kit's own exclusion set is left as it was.
        for (i, b) in city.buildings.iter().enumerate() {
            if landmark_claims.contains(b.id.trim_start_matches("sf-building-")) {
                let g = set.building_group[i];
                if g != eo_render::scene::NO_GROUP {
                    let k = (g - set.group_base) as usize;
                    set.groups[k].kind = eo_render::kit::GROUP_KIND_CLAIMED;
                    set.groups[k].level = eo_render::kit::LEVEL_NONE;
                }
            }
        }
        // (the joined street's tag keys its own tiles; none without `--structures`: the key is unchanged.)
        let street_tag = street_world.as_ref().map(|w| w.o.structures_tag()).unwrap_or_default();
        // the facade clutter the kit tiles are cooked with (`--facade-clutter`; 0: none, the tiles and their key as before).
        eo_kit::clutter::set_level(args.facade_clutter);
        kit_city_rx = Some(eo_render::kit::spawn_kit_city(city.clone(), paths.repo_root.clone(), set.palette.clone(), args.streets, street_tag));
        set.streets = street_world.clone();
        kit_set = Some(Arc::new(set));
    }
    let kit_prepare_ms = t.elapsed().as_secs_f64() * 1e3;
    let mut street_parts = None;
    if let Some(sw) = &street_world {
        let group_base = authored.groups.len() as u32 + kit_set.as_ref().map(|k| k.groups.len() as u32).unwrap_or(0);
        // the street draws its own material (the JS ground materials over the scanned sets).
        let pal = Some(eo_render::street::StreetPalette::new(authored.materials.street()));
        street_parts = Some(streets::prepare(sw.clone(), group_base, &mut authored, &city, pal));
    }
    let mut rock_material = eo_render::scene::MATERIAL_VERTEX_COLOUR;
    if let Some(kit) = kit_lib {
        // The kit bake runs beside the first frames; models and kit packs wait for it.
        let layers = authored.materials.layers.clone();
        let rock_maps = args.black_point_intertidal
            .then(|| eo_render::seabed::rock_maps::load(&paths.repo_root)).transpose()?;
        if let Some(maps) = &rock_maps {
            rock_material = eo_render::seabed::rock_maps::register(&mut authored.materials, layers.len(), maps);
        }
        let cache = cache_dir(args);
        let (tx, rx) = std::sync::mpsc::channel();
        let repo_root = paths.repo_root.clone();
        std::thread::spawn(move || {
            let t = Instant::now();
            let r = kit::bake_all(&kit, &layers, kit::HIGH_RES, cache.as_deref())
                .map(|mut maps| {
                    // a finish the JS draws from the scanned facade
                    // atlas (`atlasId`) takes that atlas cell, not a bake.
                    for (i, name) in layers.iter().enumerate() {
                        let Some(cell) = kit.specs.get(name).and_then(|s| s.atlas_id.clone()) else { continue };
                        match eo_assets::street_maps::facade_atlas_cell_cached(&repo_root, &cell, kit::HIGH_RES, cache.as_deref()) {
                            Ok((a, n, o)) => {
                                maps[i].albedo = a;
                                maps[i].normal = n;
                                maps[i].orm = o;
                            }
                            Err(e) => eprintln!("warning: facade atlas cell {cell} not loaded ({e:#}); {name} keeps its bake"),
                        }
                    }
                    maps.extend(rock_maps);
                    (KitUpload::build(&maps), t.elapsed().as_secs_f64() * 1e3)
                })
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(r);
        });
        kit_rx = Some(rx);
    }
    let kit_groups: Vec<u32> = kit_set.as_ref().map(|k| k.building_group.clone()).unwrap_or_default();
    // the far boxes' floors near the kit rungs' street-front floor.
    let t_floor = Instant::now();
    let box_floors: Vec<Option<f32>> = match &street_world {
        Some(sw) if !kit_groups.is_empty() => {
            let n = city.buildings.len();
            let threads = std::thread::available_parallelism().map(|p| p.get()).unwrap_or(4).min(16);
            let chunk = n.div_ceil(threads);
            std::thread::scope(|s| {
                let hs: Vec<_> = (0..threads)
                    .map(|t| {
                        let (sw, city, kg) = (sw.clone(), city.clone(), &kit_groups);
                        s.spawn(move || {
                            (t * chunk..((t + 1) * chunk).min(n))
                                .map(|i| {
                                    if kg.get(i).is_none_or(|g| *g == eo_render::scene::NO_GROUP) {
                                        return None;
                                    }
                                    let ring: Vec<[f64; 2]> = city.buildings[i].ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect();
                                    Some(eo_render::kit::far_floor_estimate(&sw, &ring) as f32)
                                })
                                .collect::<Vec<_>>()
                        })
                    })
                    .collect();
                hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
            })
        }
        _ => vec![],
    };
    let box_floor_ms = t_floor.elapsed().as_secs_f64() * 1e3;
    if std::env::var("EO_TRACE_LOAD").is_ok() {
        eprintln!("surfaces: far box floors {box_floor_ms:.0} ms");
    }
    let box_floor_fn = |i: usize| box_floors.get(i).copied().flatten();
    let skip_claimed = |i: usize| landmark_claims.contains(city.buildings[i].id.trim_start_matches("sf-building-"));
    // the shore field (joined here: the ground under the rings' sea is
    // pressed to the seabed, the JS ground rule, while the water pass is on).
    let water = match water_job.map(|j| j.join()) {
        Some(Ok(Ok(d))) => {
            if std::env::var("EO_TRACE_LOAD").is_ok() {
                eprintln!("water: shore field {} x {} ({}), spectrum, {:.0} ms", d.field.width, d.field.height, if d.field_from_cache { "cache" } else { "baked" }, d.ms);
            }
            Some(Arc::new(d))
        }
        Some(Ok(Err(e))) => {
            eprintln!("warning: water not loaded ({e:#}); the flat sea draws");
            None
        }
        Some(Err(_)) => anyhow::bail!("water thread panicked"),
        None => None,
    };
    let press = water.clone().filter(|_| args.look.water);
    let profile = press.as_ref().filter(|_| args.cove_detail || args.capture || args.bench)
        .map(|p| Arc::new(cove_bed::CoveBed::new(&elevation, &p.field)));
    let cove = profile.clone().filter(|_| args.cove_detail);
    // the beach foreshore goes into the street world's carpet, which the
    // drawn ground, the collision ground (eo-player StreetGround) and the
    // agents all read, so they agree on the lowered beach.
    if let (Some(sw), Some(p)) = (&street_world, &press) {
        let p = p.clone();
        let cove = cove.clone();
        sw.set_shore_press(Arc::new(move |x, z, y| {
            if let Some((h, w)) = cove.as_ref().and_then(|c| c.sample(x, z)) {
                let old = p.field.press_ground(x, z, y);
                return old + w * (h - old);
            }
            p.field.press_beach(x, z, y)
        }));
    }
    let elev = &elevation;
    let carpet_fn: Option<Box<dyn Fn(f32, f32) -> f32 + Sync + '_>> = match (street_world.clone(), press) {
        (Some(w), Some(p)) => Some(Box::new(move |x: f32, z: f32| {
            let y = w.carpet_vertex(x, z);
            if cove.as_ref().is_some_and(|c| c.sample(x as f64, z as f64).is_some()) { y }
            else { p.field.press_ground_rings(x as f64, z as f64, y as f64) as f32 }
        })), // the beach is in the carpet already
        (Some(w), None) => Some(Box::new(move |x: f32, z: f32| w.carpet_vertex(x, z))),
        (None, Some(p)) => Some(Box::new(move |x: f32, z: f32| {
            let (x, z) = (x as f64, z as f64);
            let old = p.field.press_ground(x, z, elev.ground(x, z));
            cove.as_ref().and_then(|c| c.sample(x, z)).map_or(old, |(h, w)| old + w * (h - old)) as f32
        })),
        (None, None) => None,
    };
    // the landmark set joined before the scene build (it was done
    // long before: the wall and the piers take about half a second beside the
    // street world); its waterfront press shapes the terrain under the pier
    // decks and behind the quay walls.
    let landmark_wait = Instant::now();
    let landmark_set = match landmark_job {
        Some(job) => Some(job.join().map_err(|_| anyhow::anyhow!("landmark thread panicked"))??),
        None => None,
    };
    let landmark_wait_ms = landmark_wait.elapsed().as_secs_f64() * 1e3;
    let shore_press = landmark_set.as_ref().and_then(|s| s.press.clone());
    // a terrain vertex's colour from the open-space ground: beach
    // sand (its shore distance, metres inland, in the alpha byte: 1.5 a metre
    // from 64 m offshore; 255 is no beach) or a green's tone; sRGB bytes as
    // the terrain's vertex colours are (draw.wgsl decodes them with a 2.2 power).
    let cover = match cover_job.map(|j| j.join()) {
        Some(Ok(Ok(c))) => {
            // The props tiles plant the park trees from it (props.rs `cook`).
            let c = Arc::new(c);
            eo_landmarks::ground_cover::install(c.clone());
            Some(c)
        }
        Some(Ok(Err(e))) => {
            eprintln!("warning: landmarks2 ground cover not loaded ({e:#}); the terrain keeps its land-cover tint");
            None
        }
        Some(Err(_)) => anyhow::bail!("ground cover thread panicked"),
        None => None,
    };
    // stage 4, the distant trees: the street tree survey on the footway and
    // the ground cover's park trees over the whole city, gathered beside the rest of the load.
    let hlod_trees = match (&street_world, &cover, eo_render::hlod::HlodOptions::from_env().on && eo_render::hlod::HlodOptions::from_env().trees && args.kit) {
        (Some(sw), cover, true) => {
            let (sw, cover, g) = (sw.clone(), cover.clone(), (elevation.meta.origin_x, elevation.meta.origin_z, elevation.max_x(), elevation.max_z()));
            let (tx, rx) = std::sync::mpsc::channel();
            let cove_detail = args.cove_detail;
            let canopy_root = (cove_detail && args.look.cove_canopy).then(|| paths.repo_root.clone());
            std::thread::spawn(move || {
                let t = Instant::now();
                let Ok(js) = eo_props::JsProps::load() else { return };
                let lift = sw.o.footway_lift;
                let mut trees = eo_render::hlod_trees::street_trees(&js, &|x, z| sw.ground(x, z) + lift);
                let street_n = trees.len();
                if let Some(cover) = &cover {
                    let sea = eo_water::SEA_LEVEL_Y;
                    let blocked = |x: f64, z: f64| sw.buildings.contains(x, z) || sw.paved_at(x, z).is_some() || sw.ground(x, z) < sea + 0.3;
                    let ground = |x, z| sw.ground(x, z);
                    trees.extend(eo_render::hlod_trees::park_trees(cover, g.0, g.1, g.2, g.3, &ground, &blocked));
                    if cove_detail {
                        eo_render::hlod_trees::fill_black_point(&mut trees, cover, &ground, &blocked);
                    }
                }
                if std::env::var("EO_TRACE_LOAD").is_ok() {
                    eprintln!("hlod: {} distant trees ({} street, {} park) in {:.0} ms (load thread)", trees.len(), street_n, trees.len() - street_n, t.elapsed().as_secs_f64() * 1e3);
                }
                // Ordered by block here, off the render thread (the GPU culls the blocks).
                let mut list = eo_render::hlod_trees::into_blocks(trees);
                if let Some(root) = canopy_root {
                    list.bake_black_point_canopy(&root, &js);
                }
                let _ = tx.send(list);
            });
            Some(rx)
        }
        _ => None,
    };
    let pack = |c: [f64; 3], a: u8| -> u32 {
        let b = |v: f64| (v.max(0.0).powf(1.0 / 2.2).min(1.0) * 255.0).round() as u32;
        b(c[0]) | (b(c[1]) << 8) | (b(c[2]) << 16) | ((a as u32) << 24)
    };
    let water_field = water.clone();
    const FAR_LAND_ABOVE_SEA_M: f64 = 2.0;
    const FAR_LAND_SHORE_M: f64 = 60.0;
    let municipal = shore_press.clone().filter(|_| args.cove_detail);
    let far_land_fn = |x: f64, z: f64| -> Option<f64> {
        if municipal.as_ref().is_some_and(|p| p.under_municipal_pier(x, z)) { return None; }
        cover.as_ref()?;
        let w = water_field.as_deref()?;
        if w.field.signed_at(x, z) <= FAR_LAND_SHORE_M {
            return None;
        }
        elev.sample(x, z).filter(|h| *h > eo_water::SEA_LEVEL_Y + FAR_LAND_ABOVE_SEA_M)
    };
    let ground_tone_fn = |x: f32, z: f32| -> Option<u32> {
        let cover = cover.as_ref()?;
        let (x, z) = (x as f64, z as f64);
        // the far land's ground by its land-cover class (tile-ground-cover.js
        // `DERIVED_TONES`: unmapped open ground, wild grass or coastal scrub; under trees understorey).
        if far_land_fn(x, z).is_some() {
            let class = land.as_ref().map(|l| l.class_at(x, z)).unwrap_or(0);
            return Some(pack(eo_landmarks::ground_cover::far_land_tone(class, x, z), 255));
        }
        if let Some(w) = water_field.as_deref() {
            let s = w.field.signed_at(x, z);
            if cover.beach_sand(x, z, s.abs()) {
                let a = ((64.0 - s) * 1.5).round().clamp(0.0, 253.0) as u8;
                return Some(pack(cover.sand_at(x, z), a));
            }
        }
        // the open ground's measured green (yards, lots, hills), inland of the shore.
        let park = cover.tone_name_at(x, z).and_then(|n| cover.tone_at(x, z).map(|c| (n, c)));
        let class = land.as_ref().map(|l| l.class_at(x, z)).unwrap_or(0);
        let shore = water_field.as_deref().map(|w| w.field.signed_at(x, z).abs());
        let h = |dx: f64, dz: f64| elev.sample(x + dx, z + dz);
        let slope = match (h(-6.0, 0.0), h(6.0, 0.0), h(0.0, -6.0), h(0.0, 6.0)) {
            (Some(a), Some(b), Some(c), Some(d)) => ((b - a).hypot(d - c)) / 12.0,
            _ => 0.0,
        };
        if let Some(c) = eo_render::ground2::open_ground(x, z, class, park, shore, h(0.0, 0.0).map(|h| h - eo_water::SEA_LEVEL_Y), slope) {
            return Some(pack(c, 255));
        }
        cover.tone_at(x, z).map(|c| pack(c, 255))
    };
    // land beyond the city's shoreline rings. The rings call the
    // Marin headlands across the Golden Gate sea (they are not San
    // Francisco's), so the water press drew the elevation grid's own ground
    // there (it reaches z -7,512) as the seabed and the bridge's north end
    // stood in open water. Ground more than `FAR_LAND_ABOVE_SEA_M` over the
    // sea and more than `FAR_LAND_SHORE_M` out from every ring edge (never a
    // quay's or a pier's own LiDAR cells, which the sampler reads a few metres
    // out over the water) keeps its height; the sea is drawn under it.
    let far_land = &far_land_fn;
    let carpet_fn: Option<Box<dyn Fn(f32, f32) -> f32 + Sync + '_>> = match (carpet_fn, shore_press) {
        (Some(f), Some(sp)) => Some(Box::new(move |x: f32, z: f32| match far_land(x as f64, z as f64) {
            Some(h) => h as f32,
            None => sp.press(x as f64, z as f64, f(x, z) as f64) as f32,
        })),
        (None, Some(sp)) => Some(Box::new(move |x: f32, z: f32| sp.press(x as f64, z as f64, elev.ground(x as f64, z as f64)) as f32)),
        (f, None) => f,
    };
    // the tunnels' open cuts and portal canopies (`--structures --tunnels`; none otherwise) cut the terrain.
    let terrain_cuts = structures_job.as_ref().and_then(|j| j.world()).map(|w| w.as_ref());
    let mut scene = build_scene_with_cuts(&SceneInputs { // mut (the landmarks are appended)
        city: &city,
        elevation: &elevation,
        land: land.as_ref(),
        standards: &standards,
        authored: &authored.group_of_record,
        terrain_lod: args.terrain_lod,
        kit_groups: &kit_groups,
        carpet: carpet_fn.as_deref(), // the street carpet, pressed under the sea
        street: street_parts.as_ref().map(|p| &p.1),
        skirts: street_parts.as_ref().map(|p| p.2.as_slice()).unwrap_or(&[]),
        box_floor: if box_floors.is_empty() { None } else { Some(&box_floor_fn) },
        skip_building: if landmark_claims.is_empty() { None } else { Some(&skip_claimed) },
        ground_tone: if cover.is_some() { Some(&ground_tone_fn) } else { None },
        smooth_terrain: args.landmarks && args.landmarks2,
    }, terrain_cuts)?;
    let mut dive_sites = vec![];
    if args.look.water && (args.underwater_habitats || args.cove_detail || args.dive_site.is_some() || args.capture || args.bench)
        && let (Some(data), Some(height)) = (&water, &carpet_fn)
    {
        let rocky_site = (args.black_point_intertidal || args.dive_site == Some(eo_render::seabed::Habitat::Rocky)
            || args.segments.as_ref().is_some_and(|p| p.iter().any(|n| n == "black_point_rocks"))).then_some(args.black_point_intertidal);
        dive_sites = dive_sites::prepare(&mut scene, &data.field, elev, height.as_ref(), args.underwater_habitats, args.cove_detail, rocky_site.map(|draw| (draw, rock_material)), profile.as_deref());
    }
    drop(carpet_fn); // water
    // the bridges (eo-landmarks, the JS generators ported) on the
    // drawn ground, appended to the static table; their LOD groups follow the street's.
    let landmarks = if let Some(mut set) = landmark_set {
        let t = Instant::now();
        if std::env::var("EO_TRACE_LOAD").is_ok() {
            eprintln!("landmarks: waited {landmark_wait_ms:.0} ms for the bridge and wall thread (before the scene build)");
            if let Some(c) = &set.census {
                eprintln!(
                    "landmarks2: shore edges {} visited: {} wall ({:.0} m), {} pier ({:.0} m), {} beach, {} county line; {} kept that the 4 m probe alone dropped; {} pier decks, {} piles",
                    c.visited, c.wall, c.wall_m, c.pier, c.pier_m, c.beach, c.county_line, c.kept_by_far_probe, set.piers_built.0, set.piers_built.1
                );
            }
        }
        // The paved plazas and the promenade seawalls (built on their own thread
        // since the street world arrived; on the bare ground without one).
        let pr = if let Some(job) = plaza_job {
            let waited = std::time::Instant::now();
            let (plazas, r, pr) = job.join().map_err(|_| anyhow::anyhow!("plaza thread panicked"))??;
            set.structures.extend(plazas.structures);
            if std::env::var("EO_TRACE_LOAD").is_ok() {
                eprintln!("landmarks: waited {:.0} ms for the plaza and promenade thread", waited.elapsed().as_secs_f64() * 1e3);
                eprintln!("landmarks: plazas {} ({} skipped: {:?}), {} children, {} pieces laid, {} under street paving, {} triangles, {:.0} ms (greens / plan / footprints / pieces / lay {:.0?})", r.plazas, r.skipped.len(), r.skipped, r.children, r.pieces, r.covered_by_street, r.triangles, r.ms, r.stage_ms);
            }
            pr
        } else {
            let ground = |x: f64, z: f64| elevation.ground(x, z);
            set.add_promenades(&ground, &ground)
        };
        if std::env::var("EO_TRACE_LOAD").is_ok() {
            eprintln!("landmarks: Ocean Beach seawall {} stations, {} stairs; Sunset Dunes seawall {} stations, {} flights", pr[0].0, pr[0].1, pr[1].0, pr[1].1);
        }
        let base = authored.groups.len() as u32 + kit_set.as_ref().map(|k| k.groups.len() as u32).unwrap_or(0) + street_parts.as_ref().map(|p| p.0.groups.len() as u32).unwrap_or(0);
        let lm = eo_render::landmarks::append(&mut scene, &set, &mut authored.materials, base);
        if std::env::var("EO_TRACE_LOAD").is_ok() {
            eprintln!(
                "landmarks: {} landmarks, {} wall chunks, {} objects, {} groups, near {} / far {} / wire {} / always {} triangles, {:.0} ms (bridges {:.0} ms, shore {:.0} ms; cells / grow / push {:.0?} ms)",
                set.landmarks.len(),
                set.walls.len(),
                lm.objects,
                lm.groups.len(),
                lm.near_triangles,
                lm.far_triangles,
                lm.wire_triangles,
                lm.always_triangles,
                t.elapsed().as_secs_f64() * 1e3,
                set.build_ms,
                set.walls_ms,
                lm.append_ms
            );
        }
        Some(Arc::new(lm))
    } else {
        None
    };
    // the far layer's cells; a code-built authored building's box gets a group of its own (after every other group).
    let hlod = match (&kit_set, eo_render::hlod::HlodOptions::from_env().on && args.models && args.kit) {
        (Some(k), true) => {
            let t = Instant::now();
            let before = authored.groups.len() + k.groups.len() + street_parts.as_ref().map(|p| p.0.groups.len()).unwrap_or(0) + landmarks.as_ref().map(|l| l.groups.len()).unwrap_or(0);
            let set = eo_render::hlod_bake::HlodSet::prepare(&city, k, &authored, &landmark_claims, &mut scene, before as u32);
            if std::env::var("EO_TRACE_LOAD").is_ok() {
                eprintln!(
                    "hlod: {} cells of {} m, {} buildings ({} kit, {} Blender, {} code-built), {} claimed by landmarks, {} unmatched, {:.0} ms",
                    set.cells.len(),
                    eo_render::hlod_bake::CELL_M,
                    set.covered,
                    set.by_source[0],
                    set.by_source[1],
                    set.by_source[2],
                    set.skipped_claimed,
                    set.unmatched_plain,
                    t.elapsed().as_secs_f64() * 1e3
                );
            }
            Some(Arc::new(set))
        }
        _ => None,
    };
    let load_ms = LoadTimes {
        city_ms,
        elevation_ms,
        land_cover_ms,
        authored_prepare_ms: authored.prepare_ms,
        kit_prepare_ms,
        scene_build_ms: scene.stats.build_ms,
    };
    let street = street_parts.map(|p| Arc::new(p.0));
    let street_maps = match street_maps_job.map(|j| j.join()) {
        Some(Ok(Ok((m, cached)))) => Some((Arc::new(m), cached)),
        Some(Ok(Err(e))) => {
            eprintln!("warning: street surface sets not loaded ({e:#}); the street draws its flat palette tints");
            None
        }
        Some(Err(_)) => anyhow::bail!("street surface set thread panicked"),
        None => None,
    };
    // facade
    let facade_detail = match facade_job.join() {
        Ok(Ok(d)) => Some(Arc::new(d)),
        Ok(Err(e)) => {
            eprintln!("warning: facade-detail-v1 not loaded ({e:#}); kit walls draw without its relief and cavity");
            None
        }
        Err(_) => anyhow::bail!("facade detail thread panicked"),
    };
    // the grade-separated roads (None without `--structures`); a failed build stops the load.
    let (structures, structures_drawn) = match structures_job {
        Some(j) => {
            let b = j.join()?;
            (Some(b.world), b.drawn)
        }
        None => (None, None),
    };
    Ok(World { paths, city, elevation, scene, authored: Arc::new(authored), kit_rx, kit: kit_set, kit_city_rx, street, load_ms, street_maps, facade_detail, water, dive_sites, landmarks, hlod, hlod_trees, structures, structures_drawn }) // hlod
}

/// the street size. `--ground-scale S` (an explicit ground-plan
/// scale for A/B; 1.0 is the surveyed, standard world; default: the approved
/// ground-plan deviation, 1.25), `--default-camera` (the player camera as it
/// was; the lower camera is the default), and the trial's visual-only
/// `--people-scale P` / `--vehicle-scale V` and (hills) `--height-scale H`
/// (the ground heights about sea level; default: they follow the plan, so
/// every grade is the surveyed one; `--height-scale 1.0` is the 1.25 plan with
/// real heights) set the environment switches
/// every crate reads (`eo_data::groundscale`), before any thread starts; the
/// mode parsers skip them. Then the scale is resolved once (an unapproved
/// deviation or a bad value stops here) and named on one line.
fn groundscale_flags() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    for (flag, var) in [("--ground-scale", "EO_GROUND_SCALE"), ("--height-scale", "EO_HEIGHT_SCALE"), ("--people-scale", "EO_PEOPLE_SCALE"), ("--vehicle-scale", "EO_VEHICLE_SCALE")] {
        if let Some(i) = a.iter().position(|x| x == flag) {
            let v = a.get(i + 1).ok_or_else(|| anyhow::anyhow!("{flag} needs a value"))?;
            if flag == "--ground-scale" || flag == "--height-scale" {
                eo_data::groundscale::parse_scale(v)?;
            }
            // SAFETY: first thing in main, before any other thread exists.
            unsafe { std::env::set_var(var, v) };
        }
    }
    if a.iter().any(|x| x == "--default-camera") {
        // SAFETY: as above.
        unsafe { std::env::set_var("EO_LOW_CAMERA", "0") };
    } else if a.iter().any(|x| x == "--low-camera") {
        // SAFETY: as above.
        unsafe { std::env::set_var("EO_LOW_CAMERA", "1") };
    }
    let s = eo_data::groundscale::resolve()?;
    let h = eo_data::groundscale::resolve_height()?; // hills
    eprintln!(
        "street size: ground plan x{s} ({}), heights x{h} ({}), {} camera",
        eo_data::groundscale::source(),
        eo_data::groundscale::height_source(),
        if eo_data::groundscale::low_camera() { "the lower" } else { "the default" }
    );
    Ok(())
}

/// `--no-roof-colours` (no measured roof colour table: the finishes as
/// before), `--no-roof-contrast`, `--no-roof-parapet`, `--no-roof-patches`,
/// `--no-roof-plant` set the switches `eo_data::roof_colours` reads, before
/// any thread starts.
fn roofs_flags() {
    let a: Vec<String> = std::env::args().collect();
    for (flag, var) in [
        ("--no-roof-colours", "EO_ROOF_COLOURS"),
        ("--no-roof-contrast", "EO_ROOF_CONTRAST"),
        ("--no-roof-parapet", "EO_ROOF_PARAPET"),
        ("--no-roof-patches", "EO_ROOF_PATCHES"),
        ("--no-roof-plant", "EO_ROOF_PLANT"),
        ("--no-wall-paint", "EO_WALL_PAINT"),
        ("--no-ground-green", "EO_GROUND_GREEN"),
        ("--no-ground-hills", "EO_GROUND_HILLS"),
        ("--no-ground-contact", "EO_GROUND_CONTACT"),
    ] {
        if a.iter().any(|x| x == flag) {
            // SAFETY: first thing in main, before any other thread exists.
            unsafe { std::env::set_var(var, "off") };
        }
    }
}

fn main() -> Result<()> {
    groundscale_flags()?;
    roofs_flags();
    let process_start = Instant::now();
    // `--connect ADDR|local` (online: window, --net-bench, --net-capture), see net.rs.
    if let Some(r) = net::intercept(process_start) {
        return r;
    }
    // `--audio-render` (the offline mixes, no GPU), see audio.rs.
    if let Some(r) = audio::intercept(process_start) {
        return r;
    }
    // ==== eo-player: `--play` (window) and `--bench --play` (offscreen), see play.rs ====
    #[path = "play.rs"]
    mod play;
    if let Some(r) = play::intercept(process_start) {
        return r;
    }
    // ==== end eo-player hook ====
    // ---- eo-ui (interface): --ui, --capture-ui SCREEN, --ui-bench; see ui.rs. Default paths untouched. ----
    if let Some(flags) = ui::Flags::from_env() {
        return ui::run(flags, process_start);
    }
    // ---- end eo-ui ----
    let args = parse_args()?;
    // `--agents-bench` (A/B cost) and `--agents-capture` (PNGs), see agents_bench.rs.
    if agents_bench::requested() {
        return agents_bench::run(&args, process_start);
    }
    if args.bench { bench::run(&args, process_start) } else { window::run(&args, process_start) }
}
