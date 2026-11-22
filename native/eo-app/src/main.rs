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
mod test_characters;
mod car_access;
mod house;
mod play;
mod props; // props
mod agents_bench;
mod people_capture; // people
mod bench;
mod dive_sites;
mod cove_bed;
mod cc0_runtime; // the CC0 scanned sets at run time
mod farshore; // the far terrain's load and attach
mod ground_tone; // the terrain's vertex tone and the far land (the scene build's closures)
mod fade; // the landmark lift's fade and prompt
mod models;
mod net; // net
mod photo; // photo mode (--play, --connect; `--no-photo-mode`, `--no-dof`)
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
    pub beaver_house: bool,
    pub interior_lighting: bool,
    pub house_finishes: bool,
    pub house_patina: bool,
    pub house_furniture: bool,
    pub house_photo: bool,
    pub house_photo_lod: bool,
    pub house_blender_bedroom: bool,
    pub house_mission_roof: bool,
    pub house_dining_light: bool,
    pub house_downstairs_light: bool,
    pub house_table_light: bool,
    pub house_oak_finish: bool,
    pub house_oak_joinery: bool,
    pub house_floor_reflections: bool,
    pub house_fitted_dining: bool,
    pub house_fitted_finishes: bool,
    pub house_kitchen_light: bool,
    pub house_window_linen: bool,
    pub house_street_garden: bool,
    pub house_front_daylight: bool,
    pub house_wall_daylight: bool,
    pub house_window_optics: bool,
    pub house_joinery_light: bool,
    pub house_porch_lamp: bool,
    pub house_fired_stoop: bool,
    pub house_site_camera: bool,
    pub house_dining_meter: bool,
    pub house_glazing: bool,
    pub bench: bool,
    pub start_underwater: bool,
    pub dive_site: Option<eo_render::seabed::Habitat>,
    pub underwater_habitats: bool,
    pub cove_detail: bool,
    /// the harbours' floating docks (eo-landmarks floating_docks.rs); `--no-floating-docks` leaves them out.
    pub floating_docks: bool,
    pub black_point_intertidal: bool,
    pub black_point_rockweed: bool,
    pub cove_pile_contact: bool,
    pub surface_start: bool,
    /// `--quality` (default medium: the street look); expanded into `look` once
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
    /// `--bounded-draws` (opt-in; `--no-bounded-draws` wins wherever it stands):
    /// with the count emulation, the lists the culls count guards for drawn to
    /// a bound from those guards (eo-render bounds.rs, `GpuOptions::bounded_draws`).
    pub bounded_draws: bool,
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
    /// the Rocketbox people walk the streets when the people folder (external/people, or EO_PEOPLE_ROOT) holds a copy; `--no-rocketbox` draws no people.
    pub rocketbox: bool,
    pub test_character: Option<eo_agents::test_characters::Selection>,
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
    /// other players' ground contact is drawn (`--connect` without
    /// `--no-player-contact` or `--no-remote-bodies`; net.rs): its shader
    /// code is compiled in (`ShaderFeatures::player_contact`). False
    /// everywhere else.
    pub player_contact: bool,
    /// the landmarks' walkable decks, their lifts and the tower bands they open
    /// (`--no-landmark-access` builds the landmarks as before, with no decks).
    pub landmark_access: bool,
    /// the cars' detail (`--car-detail [1|2]`): the far slot a simplified body
    /// instead of three boxes (eo-agents `far_body`) and the nearest cars kept
    /// at full detail further out (eo-render `Tiers::with_car_detail`); 0 (the
    /// default) as before. Set by `quality::Switches::apply`.
    pub car_detail: u8,
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
    args.cove_pile_contact = args.cove_detail && !raw.iter().any(|s| s == "--no-cove-pile-contact");
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

/// `parse_args` over the given arguments (the program name left out).
fn parse_args_from(raw: &[String]) -> Result<Args> {
    let mut test_character = test_characters::Options::default();
    let mut a = Args {
        interior_lighting: true,
        house_finishes: false,
        house_patina: false,
        house_furniture: false,
        house_photo: false,
        house_photo_lod: false,
        house_blender_bedroom: false,
        house_mission_roof: false,
        house_dining_light: false,
        house_downstairs_light: false,
        house_table_light: false,
        house_oak_finish: false,
        house_oak_joinery: false,
        house_floor_reflections: false,
        house_fitted_dining: false,
        house_fitted_finishes: false,
        house_kitchen_light: false,
        house_window_linen: false,
        house_street_garden: false,
        house_front_daylight: false,
        house_wall_daylight: false,
        house_window_optics: false,
        house_joinery_light: false,
        house_porch_lamp: false,
        house_fired_stoop: false,
        house_site_camera: false,
        house_dining_meter: false,
        house_glazing: true,
        beaver_house: false,
        bench: false,
        start_underwater: false,
        dive_site: None,
        underwater_habitats: false,
        cove_detail: false,
        floating_docks: true,
        black_point_intertidal: false,
        black_point_rockweed: false,
        cove_pile_contact: false,
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
        bounded_draws: false,
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
        test_character: None,
        props: true, // props
        props2: Default::default(),
        perf: Default::default(), // occlusion / hitch
        landmarks: true,          // landmarks
        landmarks2: true,
        structures: None, // `--structures` (quality::Switches)
        facade_clutter: 0, // `--facade-clutter` (quality::Switches)
        player_contact: false, // set by net.rs (`--connect`)
        landmark_access: true,
        car_detail: 0, // `--car-detail` (quality::Switches)
    };
    let mut tier = quality::Switches::default(); // `--quality` and the lighting switches, applied after the loop
    // `--scene-cards` renders at 2560 x 1440 unless a size is given.
    let (mut sized, mut scene_cards) = (false, false);
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
            "--width" => {
                a.width = val()?.parse()?;
                sized = true;
            }
            "--height" => {
                a.height = val()?.parse()?;
                sized = true;
            }
            // offscreen shots and the visual-gate scene cards (bench/shots.rs reads the values).
            "--shots" | "--clip" => {
                val()?;
                a.bench = true;
            }
            "--scene-cards" => {
                a.bench = true;
                scene_cards = true;
            }
            "--shots-only" | "--card8-fps" | "--card8-seconds" | "--card8-speed" | "--settle-timeout" => {
                val()?;
            }
            "--msaa" => a.msaa = val()?.parse()?,
            "--no-indirect-validation" => a.indirect_validation = false,
            "--no-indirect-count" => a.no_indirect_count = true,
            "--no-count-emulation" => a.no_count_emulation = true,
            "--bounded-draws" => a.bounded_draws = true,
            "--no-bounded-draws" => {} // read after the loop: it wins over `--bounded-draws`
            "--no-vsync" => a.vsync = false,
            "--out" => a.out_dir = Some(val()?.into()),
            "--label" => a.label = Some(val()?),
            "--capture" => a.capture = true,
            "--far" => a.far = val()?.parse()?,
            "--multiply" => a.multiply = val()?.parse::<u32>()?.max(1),
            "--check-budget" => a.check_budget = true,
            "--budget-ms" => a.budget_ms = Some(val()?.parse()?),
            "--beaver-house" => a.beaver_house = true,
            "--no-interior-lighting" => a.interior_lighting = false,
            "--house-finishes" => a.house_finishes = true,
            "--house-patina" => { a.house_finishes = true; a.house_patina = true; }
            "--no-house-patina" => { a.house_patina = false; a.house_furniture = false; a.house_photo = false; }
            "--house-furniture" => { a.beaver_house = true; a.house_finishes = true; a.house_patina = true; a.house_furniture = true; }
            "--no-house-furniture" => { a.house_furniture = false; a.house_photo = false; }
            "--house-photo" => { a.beaver_house = true; a.house_finishes = true; a.house_patina = true; a.house_furniture = true; a.house_photo = true; }
            "--no-house-photo" => a.house_photo = false,
            "--house-blender-bedroom" | "--house-mission-roof" | "--house-dining-light" | "--house-downstairs-light" | "--house-table-light" | "--house-oak-finish" | "--house-oak-joinery" | "--house-floor-reflections" | "--house-dining-meter" | "--house-fitted-dining" | "--house-fitted-finishes" | "--house-kitchen-light" | "--house-window-linen" | "--house-street-garden" | "--house-front-daylight" | "--house-wall-daylight" | "--house-site-camera" | "--house-window-optics" | "--house-joinery-light" | "--house-porch-lamp" | "--house-fired-stoop" => {
                a.beaver_house = true;
                a.house_finishes = true;
                a.house_patina = true;
                a.house_furniture = true;
                a.house_photo = true;
                match s.as_str() {
                    "--house-dining-light" => a.house_dining_light = true,
                    "--house-downstairs-light" => a.house_downstairs_light = true,
                    "--house-table-light" => a.house_table_light = true,
                    "--house-street-garden" => a.house_street_garden = true,
                    "--house-site-camera" => a.house_site_camera = true,
                    "--house-porch-lamp" => a.house_porch_lamp = true,
                    "--house-fired-stoop" => a.house_fired_stoop = true,
                    "--house-joinery-light" => a.house_joinery_light = true,
                    "--house-window-optics" => a.house_window_optics = true,
                    "--house-wall-daylight" => a.house_wall_daylight = true,
                    "--house-front-daylight" => { a.house_street_garden = true; a.house_front_daylight = true; }
                    "--house-window-linen" => { a.house_fitted_dining = true; a.house_window_linen = true; }
                    "--house-kitchen-light" => { a.house_fitted_dining = true; a.house_kitchen_light = true; }
                    "--house-fitted-finishes" => { a.house_fitted_dining = true; a.house_fitted_finishes = true; }
                    "--house-fitted-dining" => a.house_fitted_dining = true,
                    "--house-oak-finish" => a.house_oak_finish = true,
                    "--house-dining-meter" => a.house_dining_meter = true,
                    "--house-floor-reflections" => a.house_floor_reflections = true,
                    "--house-oak-joinery" => a.house_oak_joinery = true,
                    "--house-mission-roof" => a.house_mission_roof = true,
                    _ => a.house_blender_bedroom = true,
                }
            }
            "--no-house-dining-light" => a.house_dining_light = false,
            "--no-house-downstairs-light" => a.house_downstairs_light = false,
            "--no-house-table-light" => a.house_table_light = false,
            "--no-house-fitted-finishes" => a.house_fitted_finishes = false,
            "--no-house-street-garden" => a.house_street_garden = false,
            "--no-house-site-camera" => a.house_site_camera = false,
            "--no-house-porch-lamp" => a.house_porch_lamp = false,
            "--no-house-fired-stoop" => a.house_fired_stoop = false,
            "--no-house-joinery-light" => a.house_joinery_light = false,
            "--no-house-window-optics" => a.house_window_optics = false,
            "--no-house-wall-daylight" => a.house_wall_daylight = false,
            "--no-house-front-daylight" => a.house_front_daylight = false,
            "--no-house-window-linen" => a.house_window_linen = false,
            "--no-house-kitchen-light" => a.house_kitchen_light = false,
            "--no-house-fitted-dining" => a.house_fitted_dining = false,
            "--no-house-oak-finish" => a.house_oak_finish = false,
            "--no-house-dining-meter" => a.house_dining_meter = false,
            "--no-house-floor-reflections" => a.house_floor_reflections = false,
            "--no-house-oak-joinery" => a.house_oak_joinery = false,
            "--no-house-mission-roof" => a.house_mission_roof = false,
            "--no-house-blender-bedroom" => a.house_blender_bedroom = false,
            "--house-photo-lod" => a.house_photo_lod = true,
            "--no-house-photo-lod" => a.house_photo_lod = false,
            "--no-house-glazing" => a.house_glazing = false,
            "--no-house-finishes" => a.house_finishes = false,
            "--no-beaver-house" => a.beaver_house = false,
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
            "--blender-camera" => a.look.blender_camera = true,
            "--no-blender-camera" => a.look.blender_camera = false,
            "--no-shadows" => a.look.shadows = false,
            "--no-ao" => a.look.ao = false, // no occlusion + bounce pass
            "--no-facade-age" => a.look.facade &= !eo_render::facade::RUN_AGE, // facade
            "--no-facade-detail" => a.look.facade &= !eo_render::facade::RUN_DETAIL, // facade
            "--no-atlas2" => a.look.facade &= !eo_render::facade::RUN_ATLAS2, // facade
            "--no-probe" => a.look.facade &= !eo_render::facade::RUN_PROBE, // facade
            "--no-rooms" => a.look.facade &= !eo_render::facade::RUN_ROOMS, // no room behind kit glazing
            "--no-room-frames" | "--no-room-dim" | "--no-room-blind-runs" | "--no-pane-transmittance" => {} // read by eo_render::rooms (wgsl_constants, pane_tv_flags)
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
            // lighting: the physical daylight, the aerial-perspective volume, the filmic display, the EV100 meter
            "--no-phys-sky" => a.look.sky.phys_sky = false,
            "--no-ap-lut" => a.look.sky.ap_lut = false,
            "--no-filmic" => a.look.sky.filmic = false,
            "--no-ev100" => a.look.sky.ev100 = false,
            "--no-sky-vis" => a.look.sky.sky_vis = false,
            "--no-night-shaping" => a.look.sky.night_shaping = false,
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
            "--cove-daylight" => { a.cove_detail = true; a.look.cove_daylight = true; }
            "--no-cove-daylight" => a.look.cove_daylight = false,
            "--cove-pile-contact" => a.cove_detail = true,
            "--no-cove-pile-contact" => {} // applied after the dive-site defaults
            "--black-point-intertidal" => { a.cove_detail = true; a.black_point_intertidal = true; }
            "--no-black-point-intertidal" => a.black_point_intertidal = false,
            "--black-point-rockweed" => { a.cove_detail = true; a.black_point_intertidal = true; a.black_point_rockweed = true; }
            "--no-black-point-rockweed" => a.black_point_rockweed = false,
            "--no-cove-detail" => a.cove_detail = false,
            "--floating-docks" => a.floating_docks = true,
            "--no-floating-docks" => a.floating_docks = false,
            "--underwater-habitats" => a.underwater_habitats = true,
            "--no-underwater-habitats" => a.underwater_habitats = false,
            "--no-water-optics" => a.look.water_optics = false,
            "--dive-exposure" => a.look.dive_exposure = true,
            "--no-dive-exposure" => a.look.dive_exposure = false,
            "--no-water" => a.look.water = false, // the flat sea quad, as before
            "--no-water-instanced" => a.look.water_features &= !eo_render::water::FEAT_INSTANCED, // the water pass needs multi-draw-indirect-count again (without it, the flat sea)
            "--no-water-brdf" => a.look.water_features &= !eo_render::water::FEAT_BRDF, // the earlier Fresnel, roughness and sun lobe
            "--no-water-body" => a.look.water_features &= !eo_render::water::FEAT_BODY, // the earlier body colour
            "--no-water-foam-cover" => a.look.water_features &= !eo_render::water::FEAT_FOAM, // the earlier whitecap cover and foam sampling
            "--no-water-glint" => a.look.water_features &= !eo_render::water::FEAT_GLINT, // the earlier hashed sparkle and glare flashes
            "--no-water-turbid" => a.look.water_features &= !eo_render::water::FEAT_TURBID, // the earlier three-wavelength body optics
            "--no-water-shore" => a.look.water_features &= !eo_render::water::FEAT_SHORE, // the earlier waterline and wall collar
            "--no-water-foam-lit" => a.look.water_features &= !eo_render::water::FEAT_FOAM_LIT, // the earlier whitecap shapes and foam light
            "--no-water-ppr" => a.look.water_features &= !eo_render::water::FEAT_PPR, // the reflection march alone
            "--no-water-grazing" => a.look.water_features &= !eo_render::water::FEAT_GRAZE, // one mean Fresnel times the sky in one raised direction
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
            "--bridge-detail" => {} // read where the landmark set is built (`bridge_detail`: the opt-in detail preview)
            s if eo_landmarks::suspension::SWITCHES.contains(&s) => {} // read by eo_landmarks::suspension::features (the bridges)
            "--no-ferry-building" => eo_landmarks::features::set_ferry_building(false), // its record's box, as before
            "--no-tram-tracks" => eo_landmarks::features::set_tram_tracks(false), // no streetcar rails, as before
            "--no-quay-railing" => eo_landmarks::features::set_quay_railing(false), // no guard on the quay wall, as before
            "--no-ferry-building-detail" => eo_landmarks::features::set_ferry_building_detail(false), // the Ferry Building's first pass, as before
            "--no-rail-finish" => eo_landmarks::features::set_rail_finish(false), // one bare-steel rail head, as before
            "--no-corridor-palms" => eo_landmarks::features::set_corridor_palms(false), // no Embarcadero palms, as before
            "--no-guard-detail" => eo_landmarks::features::set_guard_detail(false), // the quay guard's first pass, as before
            "--no-pier-sheds" => eo_landmarks::features::set_pier_sheds(false), // the Ferry Plaza's JS shed boxes, as before
            "--no-palm-detail" => eo_landmarks::features::set_palm_detail(false), // the corridor palms' first pass, as before
            "--no-facade-relief" => eo_landmarks::features::set_ferry_building_relief(false), // the Ferry Building's second pass, as before
            "--no-rail-polish" => eo_landmarks::features::set_rail_polish(false), // the rails' worn band at 0.4 / 0.65, no sealant joints, as before
            "--no-furniture-models" => eo_render::props_lib2::furniture_models::set_enabled(false), // missing-GLB furniture as boxes, as before
            "--no-landmark-access" => a.landmark_access = false, // the landmark decks (eo-landmarks access.rs)
            "--no-audio" | "--audio-null" => {} // read by audio::AudioRuntime::start
            "--no-photo-mode" | "--no-dof" | "--photo-bench" => {} // read by photo.rs (and the bench, `--photo-bench`)
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
            "--no-wall-paint" | "--no-ground-green" | "--no-ground-hills" | "--no-ground-contact" | "--no-ground-beach-fit" => {} // `roofs_flags`
            "--no-external" => {} // read by `external_flag` (main): the optional sets only from their variables
            "--no-far-facade" => {} // `roofs_flags`: the far boxes as before (eo-render far_box.rs)
            "--no-period-paint" => {} // `roofs_flags`: the timber rows' period paint at its finish's luminance (eo-kit wall_paint.rs)
            // `roofs_flags`: one window module per archetype (eo-kit typology.rs), uncapped wall chroma
            // (wall_paint.rs), the measured roofs' cast kept (roof_measured.rs), the far box's flat cap (far_box.rs).
            "--no-facade-typology" | "--no-wall-chroma" | "--no-roof-neutral" | "--no-far-roofs" => {}
            // the Golden Gate photo-realism pass's switches, each read by its crate from the command line
            // (the bridges' own are in eo_landmarks::suspension::SWITCHES, above).
            "--no-gg2-camera" | "--no-camera-look" | "--no-scene-meter" | "--no-lamp-photocell" => {} // read by its crate (the camera)
            "--no-gg2-daylight" | "--no-ground-albedo" | "--no-photo-date" | "--no-cirrus" | "--no-clear-sky-state" | "--no-terminator-bias" | "--no-spectral-sky" => {} // read by its crate (the daylight)
            "--no-gg2-water" | "--no-water-exposure" | "--no-water-current" | "--no-water-ppr-blur" | "--no-beach-profiles" | "--no-water-streak-caps" | "--no-water-far-calm" | "--no-baker-surf" | "--no-swash-line" => {} // read by its crate (the water)
            "--no-gg2-ground" | "--no-beach-sand-limit" | "--no-sand-albedo" | "--no-rocks" | "--no-riprap" | "--no-pier-lines" | "--no-ground-steps" | "--no-season-seam" => {} // read by its crate (the shore ground)
            "--no-gg2-far" | "--no-far-fallback" | "--no-landmark-far-models" | "--no-far-tree-density" | "--no-presidio-rule" | "--no-far-paint" => {} // read by its crate (the far layer)
            "--no-corridor-traffic" | "--no-deck-walkers" => {} // read by its crate (the corridor traffic)
            "--no-gg2-vegetation" | "--no-coastal-trees" | "--no-ground-cover" => {} // read by its crate (the vegetation)
            // the round-3 tracks' switches: one list per track, each in a file the track owns (read by its crate)
            s if eo_render::pace::GG3_SWITCHES.contains(&s) => {} // read by its crate (the frame budget; --no-gg3 every track's roll-up)
            s if s == eo_render::CityClasses::SWITCH => {} // read by its crate (the city pass's classes, one pipeline each)
            s if s == eo_render::counted::MotionBounds::SWITCH => {} // read by its crate (the count emulation's motion-aware bounds)
            s if eo_look::season::SWITCHES.contains(&s) => {} // read by its crate (the photo dates, season and sky)
            s if eo_look::features::GG3_SWITCHES.contains(&s) => {} // read by its crate (the colour and camera)
            s if eo_landmarks::vegetation::GG3_SWITCHES.contains(&s) => {} // read by its crate (the coastal vegetation)
            s if eo_landmarks::vegetation::GRASS2_SWITCHES.contains(&s) => {} // read by its crate (the second grass pass)
            s if eo_landmarks::vegetation::GRASS3_SWITCHES.contains(&s) => {} // read by its crate (the third grass pass)
            s if eo_landmarks::flora::FLORA_SWITCHES.contains(&s) => {} // read by its crate (the flora pass)
            s if eo_landmarks::flora2::FLORA2_SWITCHES.contains(&s) => {} // read by its crate (the second flora pass)
            s if eo_landmarks::features::GG3_SWITCHES.contains(&s) => {} // read by its crate (the terrain surface)
            s if eo_landmarks::hill_rock::SWITCHES.contains(&s) => {} // read by its crate (the scanned rock; --no-hill-rock)
            s if eo_water::switches::GG3_SWITCHES.contains(&s) => {} // read by its crate (the surf and bathymetry)
            s if eo_landmarks::suspension::GG3_SWITCHES.contains(&s) => {} // read by its crate (the bridge finish)
            s if eo_landmarks::suspension::AGING_SWITCHES.contains(&s) => {} // read by its crate (the Golden Gate's aging)
            "--no-towers2" | "--no-skyline" | "--no-towers3" | "--no-grove" | "--no-towers4" => {} // `roofs_flags` (eo_landmarks::set::Options, eo_landmarks::towers3_on, towers4_on)
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
                    "eo-app [--dive-site bay|bay-surface|black-point|pacific] [--cove-detail|--no-cove-detail] [--cove-canopy|--no-cove-canopy] [--cove-sediment|--no-cove-sediment] [--cove-daylight|--no-cove-daylight] [--black-point-intertidal|--no-black-point-intertidal] [--black-point-rockweed|--no-black-point-rockweed] [--cove-pile-contact|--no-cove-pile-contact] [--water-optics] [--dive-exposure|--no-dive-exposure] [--underwater-habitats|--no-underwater-habitats] [--bench] [--frames N=720] [--warmup N=60] [--width 1920] [--height 1080] [--msaa 1|4]\n       [--no-indirect-validation] [--no-indirect-count] [--no-count-emulation] [--no-motion-bounds] [--bounded-draws|--no-bounded-draws] [--no-city-classes] [--no-vsync] [--out DIR] [--label NAME] [--capture] [--far M=4200]\n       [--multiply N=1] [--check-budget] [--budget-ms 10 (self-test only)]\n       [--no-models] [--no-terrain-lod] [--no-kit] [--slice1] [--slice2] [--cache DIR] [--no-cache] [--stream-threads N=6] [--stream-radius M=inf]\n       [--kit-threads N=6] [--kit-split-materials] [--upload-budget-mb 4] [--kit-arena-vertices N] [--kit-arena-indices N] [--kit-arena-objects N]\n       [--kit-near-load M=110] [--kit-mid-load M=520] [--drive-frames N=3600]\n       [--time-of-day H=12] [--shadow-map N=3072] [--no-shadows] [--no-agents] [--agent-density N=1] [--no-rocketbox] [--no-external]\n       [--no-occlusion] [--occlusion-check] [--no-thread-priority] [--pace realtime|60hz|all-60hz|none] [--worst-frames N=12] [--check-max]\n       [--no-view-detail] [--view-margin-deg 35] [--view-predict-s 0.3] [--view-hold-s 2.5] [--stream-all-paths] [--full-arenas]\n       [--segments street_walk,elevated_sweep,city_overview,authored_street,kit_street,drive,turn_around,turn_around_authored]\n       (with --structures and built decks also freeway_drive,under_viaduct) [--light-shots] (capture cards; EO_LIGHT_SHOTS=a,b; EO_LIGHT_SHOTS_HOUR=H every card at another hour; EO_LIGHT_SHOTS=hero the six key views, clock held)\n       [--cc0-shots] (the scanned sets' matched cards; EO_CC0_SHOTS=a,b; EO_CC0_SHOTS_HOUR=H every card at another hour)\n       [--clutter-shots] (the facade clutter's cards: the kit and authored streets, fronts with fire escapes and shop fronts the planner dresses, a roof; the same poses with or without --facade-clutter; EO_CLUTTER_SHOTS=a,b; EO_CLUTTER_SHOTS_HOUR=H, default 15)\n       [--structures-shots] (the grade-separated roads at four places, from the street and from above, 15:00 clear; the same poses with or without --structures; EO_STRUCTURES_SHOTS=a,b; EO_STRUCTURES_SHOTS_HOUR=H another hour; EO_STRUCTURES_SHOTS_HDR=1 the scene luminance too)\n       [--dusk-strip] (40 frames a clock minute apart across sunset and sunrise at one camera, a CSV of mean luminance; with --smooth-time through the soft path)\n       (--light-shots with EO_PROBE_STRIP=path[,frames[,m a frame[,turn]]]: the reflections along a street while probe captures finish, a CSV and the frames round each)\n\nQuality and lighting (opt-in; a switch wins over the tier wherever it stands; each has a --no- form):\n       [--quality low|medium|high|hyper] (default medium: the street look, grime 1, stand-ins, surveyed furniture and tree basins; bench budget 8 / 10 / 16.6 / 33.3 ms;\n       low: a 2048 sun map, low sky, no SSR, contact shadows, AO, shafts or bloom; high: the lighting additions but the probe grid,\n       with grime, street life, stand-ins, photo finish, facade clutter, shop light and car detail at 1 and the cinematic look; hyper: high at level 2, 48 lamps, the probe grid 2)\n       [--cascade-blend] [--far-shadows [1|2]] [--ambient-volume [1|2]] [--probe-grid [1|2]] [--light-shadows [N lamps, 16..48]]\n       [--light-shadow-moving] [--headlight-shadows [N vehicles]] [--lit-fog [1|2]]\n       (--ambient-volume: narrow streets and the ground under decks get less sky light, baked round the camera and over the city)\n       [--ground-bounce] (with --ambient-volume, on unless --no-ground-bounce: light bounced up from the sunlit ground onto deck undersides and walls facing it)\n       [--smooth-time] (the drawn hour follows the simulation's clock, the exposure converges instead of snapping)
       [--rt-gi] (ray-traced one-bounce diffuse light: one ray a texel at half resolution through the static city (buildings, terrain, streets' strips, structures, landmarks; no agents, props or water), accumulated over frames and filtered, in place of the sky terms and the ground bounce; Metal with ray queries only, else off with one line; no tier turns it on; EO_RT_GI_* tunables in eo-render rt_gi.rs)\n       [--rt-reflect] (ray-traced reflections on the streets, wet or dry, the buildings and their glazing up to roughness 0.6: one ray a texel at half resolution along the surface's lobe through the same static city, the sky where it leaves; in place of the sky, probe and screen-space reflection; not on cars, props or water yet; Metal with ray queries only; no tier turns it on; EO_RT_REFLECT_* tunables in eo-render rt_reflect.rs)\n       (--far-shadows: sun shadows past the cascades from cached maps round the camera, 1 a 2 km ring, 2 another to the far plane; --cascade-blend: the cascades hand over in a band instead of a line)\n       (--probe-grid: reflection probes on the streets round the camera, cross-faded, read by every lit surface, 1 four of 128 px, 2 nine of 256 px)\n       (--lit-fog: the fog lit by the street lamps and headlamps, a froxel volume added by the resolve; 1 160 x 90 x 64, 2 240 x 135 x 96; nothing by day)\n       (--light-shadows: the nearest N street lamps keep a shadow map, drawn once, at most 2 new a frame, and draw in SF's 3000 K LED colour; --light-shadow-moving: the 4 nearest redrawn every frame with the traffic; --headlight-shadows: the nearest N vehicles' low beams cast shadows; nothing by day)\n       [--shadow-slope-bias] (the sun cascades' casters with a slope-scaled depth bias: no self-shadow stripes where the sun grazes a surface; high and hyper turn it on)\n       [--grime [1|2]] (surface weathering that reads: soiled walks with gum and stains, chipped kerbs and faded kerb paint, patched and sealed carriageways, washed-down facades, dusty cars; high 1, hyper 2)\n       [--street-life [1|2]] (litter, butts, leaves and grit on the walks and in the gutters; graffiti, buffed patches and posters on ground floors and freeway columns; 2 adds stickers, utility marks, pigeon droppings; high 1, hyper 2)\n       [--stand-ins] (procedural models for street-prop kinds whose GLB is absent from the checkout, instead of grey boxes; medium, high and hyper)\n       [--surveyed-furniture] (the furniture walk invents no bollards, planters, barricades, scaffolding or newsracks, and benches, bins, post boxes, bike racks and café sets only at shopfronts; surveyed items as before; every tier)\n       [--tree-basins] (a soil and mulch basin with a flush edge under each street tree on a footway, sized by the footway from SF Public Works Director's Order 187,246; every tier)\n       [--photo-finish [1|2]] (a camera instead of the JS display transform: highlights pale to white on their hue, ISO noise by the scene's light, lens fall-off and lateral colour, a lens's bloom; San Francisco's measured air in the sky and the haze; 2 smoother fringes and chroma noise; high 1, hyper 2)\n       [--facade-clutter [1|2]] (fire escapes, awnings, colour-field and neon signs, roll-down shutters, bars on older ground-floor windows, downspouts, meters and conduit, chimneys, vents, dishes and aerials on the kit buildings; signs lit by night where the shop is; 2 adds window air conditioners, front dishes, scaffolding, boarded shops; high 1, hyper 2)\n       [--shop-light [1|2]] (lit shop windows at night: a light behind every kit shop window in its trade's colour, at its window's luminance, by its opening hours, spilling onto the sidewalk and lighting the fog, the room behind the glass lit or dark with it; 1 within 160 m, 2 to 300 m with two lights a window; high 1, hyper 2)\n       [--car-detail [1|2]] (cars past 220 m (2: 260 m) drawn as their own body simplified to ~1 px instead of three boxes, and the nearest 6 (2: 12) at full detail within 30 m (40 m), 24 (40) at LOD 1 within 90 m (120 m); high 1, hyper 2)\n       [--look default|cinematic] (the look profile: default is today's frame on its physical references; cinematic a graded open-world look, its departures from those references listed in CINEMATIC_DEVIATIONS; high and hyper; --no-look is --look default)\n       (a level left out is the high tier's; --probe-grid, which high leaves off, takes 1)\n\nWorld (opt-in; client and server must agree, the handshake refuses a mismatch; each has a --no- form):\n       [--structures] (grade-separated roads: freeways, ramps, road bridges, drawn and cast in three levels of detail) [--tunnels] (bores and portals, with --structures)\n\n\
Golden Gate photo-realism (on by default; each is a kill switch that restores the earlier output):\n       \
[--no-gg2-camera] [--no-camera-look] [--no-scene-meter] [--no-lamp-photocell] (the camera look, the scene meter, lamp photocells)\n       \
[--no-gg2-daylight] [--no-bridges4] [--no-gg2-water] [--no-gg2-ground] [--no-gg2-far] [--no-corridor-traffic] [--no-gg2-vegetation]\n       \
(the Golden Gate pass's roll-up switches; each part's own switch is in README.md, Kill switches)\n\n\
eo-app --shots FILE.json --out DIR [--width W --height H] [--time-of-day H] [--weather KIND] [--shots-only a,b] [--settle-timeout S=600] [any world or kill switch]\n\
       offscreen: one settled, converged PNG per shot of FILE.json, and DIR/<name>.json (resolved pose, build, command line, switches).\n\
       A shot is {{\"name\", \"geo\": {{\"lat\", \"lon\", \"alt\" (m above mean sea level), \"range\" (m), \"heading\" (deg, 0 north, 90 east),\n\
       \"tilt\" (deg from straight down), \"fov\" (vertical deg)}}}} (the target is looked at from range metres), or {{\"eye\": [x, y, z],\n\
       \"yaw\" (deg, 0 north (-z), 90 east (+x)), \"pitch\" (deg, up +)}} or {{\"eye\", \"look_at\": [x, y, z]}} in game metres; optional \"fov\",\n\
       \"far\", \"time_of_day\", \"weather\", \"what\".\n\
eo-app --scene-cards --out DIR [--width 2560 --height 1440] [--shots-only a,b] [--card8-fps 10] [--card8-seconds 30] [--card8-speed 5.6] [any world or kill switch]\n\
       offscreen: the eight scene cards of Docs/VISUAL_QUALITY_GATE.md at the Ferry Building and the Embarcadero, a manifest per card;\n\
       card 8 is a numbered frame sequence across a 128 m tile boundary.\n\
eo-app --clip FILE.json --out DIR [--width W --height H] [--shots-only a,b] [any world or kill switch]\n\
       offscreen: moving clips at 60 Hz of wall time along camera paths, every 60 / fps-th frame written as DIR/<name>/NNNN.png;\n\
       FILE.json holds {{\"name\", \"seconds\", \"fps\", \"time_of_day\", \"fov\", \"keys\": [{{\"t\", \"eye\": [x, y, z], \"look_at\": [x, y, z]}}]}} (bench/clip.rs)."
                );
                println!("       [--play --test-character ID [--test-clip NAME] [--car-door-animation]] [--no-test-character] [--list-test-clips]");
                std::process::exit(0);
            }
            flag if test_character.parse(flag, &mut val)? => {}
            other => anyhow::bail!("unknown argument {other} (try --help)"),
        }
    }
    tier.apply(&mut a); // the tier, then the lighting switches over it
    // `--no-bounded-draws` restores the count emulation's bounds wherever it stands.
    a.bounded_draws = a.bounded_draws && !raw.iter().any(|s| s == "--no-bounded-draws");
    // (`--rt-gi`, `--rt-reflect`) the device asks for ray queries (EO_RT_GI_DEVICE_ONLY=1, a
    // frame-cost diagnostic: the device asks for them with the options off).
    eo_render::rt_scene::set_wanted(a.look.lighting.rt_gi || a.look.lighting.rt_reflect || std::env::var_os("EO_RT_GI_DEVICE_ONLY").is_some_and(|v| v == "1"));
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
    if scene_cards && !sized {
        (a.width, a.height) = (2560, 1440);
    }
    if a.frames + a.warmup > 1024 {
        // 4 timestamp queries per frame; one query set holds at most 4096.
        anyhow::bail!("--frames + --warmup must be <= 1024 per segment");
    }
    a.test_character = test_character.selection(a.agents)?;
    Ok(a)
}

/// `--bridge-detail`: the Golden Gate's opt-in detail preview, off by default
/// (left out, the bridge draws as before). It draws only the parts the
/// bridges' own passes leave as exported (eo-landmarks golden_gate.rs).
/// `--no-bridge-detail` is not its off form: it is those passes' kill switch
/// (`eo_landmarks::suspension::BridgeFeatures::detail`), so with both given
/// the preview's legs and portal struts stand in for the passes' detail.
pub fn bridge_detail() -> bool {
    std::env::args().any(|a| a == "--bridge-detail")
}

pub struct World {
    pub house_layer_count: u32,
    pub house_vertex_base: usize,
    pub house: Option<Arc<house::House>>,
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
    /// the berths along the harbours' floating docks (eo-landmarks), where the
    /// props draw the moored boats; empty without the docks.
    pub berths: Arc<Vec<eo_landmarks::floating_docks::Berth>>,
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
    // street props v2 (`--no-street-props-v2`): before anything reads the props manifest.
    eo_props::props_v2::set_street_props_v2(args.props && args.props2.street_v2);
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
        let opts = if args.landmarks2 { eo_landmarks::set::Options { access: args.landmark_access, ..Default::default() } } else { eo_landmarks::set::Options::off() };
        let opts = eo_landmarks::set::Options { bridge_detail: bridge_detail(), ..opts };
        let opts = eo_landmarks::set::Options { cove_detail: args.cove_detail, ..opts };
        let opts = eo_landmarks::set::Options { floating_docks: opts.floating_docks && args.floating_docks, ..opts };
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
    let land = LandCover::load(&paths).ok().map(Arc::new);
    // the land-cover classes the daylight's ground albedo reads (eo-look `phys`).
    if let Some(l) = &land {
        let l = l.clone();
        eo_look::phys::set_ground_classes(Box::new(move |x: f64, z: f64| l.class_at(x, z)));
    }
    let land_cover_ms = t.elapsed().as_secs_f64() * 1e3;
    let street_job = street_plan.map(|p| streets::spawn(p, &city, &elevation));
    let standards = CaStandards::load(&paths)?;
    let mut kit_rx = None;
    let kit_lib = if args.models || args.kit || args.beaver_house { Some(KitLibrary::load()?) } else { None };
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
                // the Ferry Building on the footway datum (eo-landmarks ferry_building.rs) and the
                // corridor's streetcar rails in the drawn paving, a paint step over the junction paint.
                if bay_tones {
                    let trace = std::env::var("EO_TRACE_LOAD").is_ok();
                    if let Some(fb) = s.add_ferry_building(&|x, z| sw.ground(x, z) + o.footway_lift, &|x, z| sw.paved_at(x, z), &|x, z| sw.carpet_y(x, z)) {
                        if trace {
                            eprintln!("landmarks: Ferry Building floor {:.2} m, {:.1} x {:.1} m, bays {:?}, {} triangles, {:.0} ms", fb.base_y, fb.length_m, fb.depth_m, fb.bays, fb.triangles, fb.ms);
                        }
                    }
                    let lift = o.junction_paint_lift + (o.junction_paint_lift - o.marking_lift).abs();
                    match s.add_tram_tracks(&root, &|x, z| sw.surface_at(x, z, None), lift) {
                        Ok(Some((n, m, tris))) if trace => eprintln!("landmarks: streetcar rails {n} tracks, {m:.0} m, {tris} triangles"),
                        Err(e) => eprintln!("warning: streetcar rails not built ({e:#})"),
                        _ => {}
                    }
                    // the corridor's surveyed palms (eo-landmarks palms.rs) from the street tree list.
                    if eo_landmarks::features::corridor_palms() {
                        match eo_props::JsProps::load() {
                            Ok(js) => {
                                let t = &js.trees;
                                let st = t.stride.max(3);
                                let palm = t.forms.iter().position(|f| f == "palm");
                                let list: Vec<(f64, f64, Option<f64>, Option<f64>, u32)> = (0..t.data.len() / st)
                                    .filter(|&i| palm.is_some_and(|p| t.data[i * st + 2] as usize == p))
                                    .map(|i| {
                                        let d = &t.data[i * st..i * st + st];
                                        let dm = |k: usize| (st > k && d[k] > 0).then(|| d[k] as f64 / 10.0);
                                        (d[0] as f64 / 10.0, d[1] as f64 / 10.0, dm(3), dm(4), i as u32)
                                    })
                                    .collect();
                                if let Some((n, tris)) = s.add_corridor_palms(&list, &|x, z| sw.surface_at(x, z, None), &|x, z| sw.buildings.contains(x, z)) {
                                    if trace {
                                        eprintln!("landmarks: corridor palms {n}, {tris} triangles");
                                    }
                                }
                            }
                            Err(e) => eprintln!("warning: corridor palms not built ({e:#})"),
                        }
                    }
                }
                Ok((s, r, pr))
            }))
        }
        _ => None,
    };
    // the building records the landmark buildings draw instead.
    let mut landmark_claims: HashSet<String> = if args.landmarks && args.landmarks2 { eo_landmarks::set::claimed_records_with(eo_landmarks::set::Options::default()) } else { HashSet::new() };
    // stage A9 (`--tunnels`): a building over a tunnel's approach cut or portal canopy is
    // left out as a claimed one (none without tunnels: the claims and every key as before).
    if let Some(w) = structures_job.as_ref().and_then(|j| j.world()).filter(|w| !w.terrain_cuts().is_empty()) {
        let rings: Vec<Vec<[f64; 2]>> = city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
        landmark_claims.extend(w.buildings_over_cuts(&rings).into_iter().map(|i| city.buildings[i].id.trim_start_matches("sf-building-").to_string()));
    }
    let finishes = (args.beaver_house && args.house_finishes)
        .then(|| house::Finishes::load(&paths.repo_root, args))
        .transpose()?;
    let house = if args.beaver_house {
        let record = city.buildings.iter().find(|b| house::owns(&b.id)).ok_or_else(|| anyhow::anyhow!("missing Beaver Street house footprint"))?;
        landmark_claims.insert(house::ID.to_string());
        let ground = |x, z| street_world.as_ref().map_or_else(|| elevation.ground(x, z), |w| w.surface_at(x, z, None));
        Some(Arc::new(house::House::build(args.interior_lighting, finishes.as_ref(), record, &ground, street_world.as_deref(), &paths.repo_root, kit_lib.as_ref().unwrap(), &mut authored.materials)?))
    } else { None };
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
    let house_has_scans = finishes.as_ref().is_some_and(|f| !f.scans.is_empty());
    let finish_maps = finishes.map(house::Finishes::into_maps).unwrap_or_default();
    let house_layer_count = finish_maps.len() as u32;
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
        // the shore rock takes the layer right after the kit's bake; the house's maps follow it.
        let rock_maps = args.black_point_intertidal
            .then(|| eo_render::seabed::rock_maps::load(&paths.repo_root)).transpose()?;
        if house_has_scans {
            house::Scan::bind(&mut authored.materials, usize::from(rock_maps.is_some()));
        }
        let layers = authored.materials.layers.clone();
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
                    // the layers after the kit's bake: the shore rock (`rock_maps::register`
                    // above), then the house's maps (`house::Finishes::into_maps`), which
                    // its materials address from their first layer and draw.wgsl from the array's end.
                    maps.extend(rock_maps);
                    maps.extend(finish_maps);
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
    // the far tier's bluff cypress reads the citywide shore field's distance, as the near tier
    // does (eo-landmarks `vegetation::install_shore_distance`; the first install wins, before the
    // distant-tree thread below starts), not the drawn sea's estimate (hlod_trees `shore_estimate`)
    if let Some(w) = water.clone() {
        eo_landmarks::vegetation::install_shore_distance(Box::new(move |x, z| w.field.signed_at(x, z).abs()));
    }
    let press = water.clone().filter(|_| args.look.water);
    let profile = press.as_ref().filter(|_| args.cove_detail || args.capture || args.bench)
        .map(|p| Arc::new(cove_bed::CoveBed::new(&elevation, &p.field)));
    let cove = profile.clone().filter(|_| args.cove_detail);
    // (water r2) piers on piles over the water, unless --no-water-shore
    eo_water::field::PIERS_OVER_WATER.store(args.look.water_features & eo_render::water::FEAT_SHORE != 0, std::sync::atomic::Ordering::Relaxed);
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
    let mut landmark_set = match landmark_job {
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
    let water_field = water.clone();
    // the far land's height and a terrain vertex's colour from the open-space
    // ground (ground_tone.rs): beach sand (its shore distance, metres inland,
    // in the alpha byte) or a green's tone. With `--cove-detail` the cells
    // under Municipal Pier (its deck's own LiDAR returns) are never far land.
    let municipal = shore_press.clone().filter(|_| args.cove_detail);
    let far_land_fn = ground_tone::far_land(cover.as_deref(), water_field.as_deref(), elev, municipal.as_deref());
    let ground_tone_fn = ground_tone::ground_tone(cover.as_deref(), water_field.as_deref(), elev, land.as_deref(), municipal.as_deref());
    // land beyond the city's shoreline rings. The rings call the
    // Marin headlands across the Golden Gate sea (they are not San
    // Francisco's), so the water press drew the elevation grid's own ground
    // there (it reaches z -7,512) as the seabed and the bridge's north end
    // stood in open water. Ground more than `ground_tone::FAR_LAND_ABOVE_SEA_M` over the
    // sea and more than `ground_tone::FAR_LAND_SHORE_M` out from every ring edge (never a
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
        land: land.as_deref(),
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
    let house_vertex_base = scene.vertices.len();
    if let Some(house) = &house { house.append(&mut scene, &authored.materials, args.house_photo_lod); }
    let mut dive_sites = vec![];
    if args.look.water && (args.underwater_habitats || args.cove_detail || args.dive_site.is_some() || args.capture || args.bench)
        && let (Some(data), Some(height)) = (&water, &carpet_fn)
    {
        let rocky_site = (args.black_point_intertidal || args.dive_site == Some(eo_render::seabed::Habitat::Rocky)
            || args.segments.as_ref().is_some_and(|p| p.iter().any(|n| n == "black_point_rocks"))).then_some(args.black_point_intertidal);
        let mut bed = dive_sites::terrain_bed(elev, height.as_ref());
        if args.cove_pile_contact && let Some(set) = &mut landmark_set {
            set.seat_municipal_piles(|x, z| bed(glam::Vec2::new(x, z)));
        }
        dive_sites = dive_sites::prepare(&mut scene, &data.field, elev, height.as_ref(), &mut bed, args.underwater_habitats, args.cove_detail, rocky_site.map(|draw| eo_render::seabed::IntertidalOptions { draw, material: rock_material, rockweed: args.black_point_rockweed }), profile.as_deref());
    }
    drop(carpet_fn); // water
    // (the ground-tone closures borrow the elevation, which the world takes below)
    drop(ground_tone_fn);
    drop(far_land_fn);
    // the bridges (eo-landmarks, the JS generators ported) on the
    // drawn ground, appended to the static table; their LOD groups follow the street's.
    let mut berths = Vec::new();
    let landmarks = if let Some(mut set) = landmark_set {
        let t = Instant::now();
        berths = std::mem::take(&mut set.berths);
        if std::env::var("EO_TRACE_LOAD").is_ok() {
            eprintln!("landmarks: waited {landmark_wait_ms:.0} ms for the bridge and wall thread (before the scene build)");
            if let Some(c) = &set.census {
                eprintln!(
                    "landmarks2: shore edges {} visited: {} wall ({:.0} m), {} pier ({:.0} m), {} beach, {} county line; {} kept that the 4 m probe alone dropped; {} pier decks, {} piles",
                    c.visited, c.wall, c.wall_m, c.pier, c.pier_m, c.beach, c.county_line, c.kept_by_far_probe, set.piers_built.0, set.piers_built.1
                );
            }
            if set.docks_built.0 > 0 {
                eprintln!("landmarks2: {} floating docks ({:.0} m), {} guide piles", set.docks_built.0, set.docks_built.2, set.docks_built.1);
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
            if args.landmarks2 {
                set.add_ferry_building(&ground, &|_, _| None, &|_, _| f64::NEG_INFINITY);
            }
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
    Ok(World { house_layer_count, house_vertex_base, house, paths, city, elevation, scene, authored: Arc::new(authored), kit_rx, kit: kit_set, kit_city_rx, street, load_ms, street_maps, facade_detail, water, dive_sites, landmarks, hlod, hlod_trees, structures, structures_drawn, berths: Arc::new(berths) }) // hlod
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
        ("--no-far-facade", "EO_FAR_FACADE"),
        ("--no-period-paint", "EO_PERIOD_PAINT"),
        ("--no-facade-typology", "EO_FACADE_TYPOLOGY"),
        ("--no-wall-chroma", "EO_WALL_CHROMA"),
        ("--no-roof-neutral", "EO_ROOF_NEUTRAL"),
        ("--no-far-roofs", "EO_FAR_ROOFS"),
        ("--no-ground-green", "EO_GROUND_GREEN"),
        ("--no-ground-hills", "EO_GROUND_HILLS"),
        ("--no-ground-contact", "EO_GROUND_CONTACT"),
        ("--no-ground-beach-fit", "EO_GROUND_BEACH_FIT"),
        // the landmark buildings' second generators and the named skyline towers (eo_landmarks::set::Options).
        ("--no-towers2", "EO_TOWERS2"),
        ("--no-skyline", "EO_SKYLINE"),
        // the landmark buildings' and skyline towers' third pass (eo_landmarks::towers3_on).
        ("--no-towers3", "EO_TOWERS3"),
        // the groves round Coit and Sutro towers (eo_landmarks::grove::grove_on).
        ("--no-grove", "EO_GROVE"),
        // the landmark buildings', skyline towers' and groves' fourth pass (eo_landmarks::towers4_on).
        ("--no-towers4", "EO_TOWERS4"),
    ] {
        if a.iter().any(|x| x == flag) {
            // SAFETY: first thing in main, before any other thread exists.
            unsafe { std::env::set_var(var, "off") };
        }
    }
}

/// `--no-external`: the optional sets are read only from the folders their
/// variables name, not from `<repo>/external/<set>` (eo_data::external).
fn external_flag() {
    if std::env::args().any(|x| x == "--no-external") {
        eo_data::external::set_default_enabled(false);
    }
}

fn main() -> Result<()> {
    if std::env::args().nth(1).as_deref() == Some("--list-test-clips") {
        return test_characters::list();
    }
    groundscale_flags()?;
    roofs_flags();
    external_flag();
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

#[cfg(test)]
mod tests {
    /// Every round-3 track's lists, as main.rs accepts them.
    const GG3_LISTS: [&[&str]; 9] = [
        eo_render::pace::GG3_SWITCHES,
        eo_look::season::SWITCHES,
        eo_look::features::GG3_SWITCHES,
        eo_landmarks::vegetation::GG3_SWITCHES,
        eo_landmarks::features::GG3_SWITCHES,
        eo_landmarks::hill_rock::SWITCHES,
        eo_water::switches::GG3_SWITCHES,
        eo_landmarks::suspension::GG3_SWITCHES,
        eo_landmarks::suspension::AGING_SWITCHES,
    ];

    #[test]
    fn every_listed_switch_parses() {
        let mut seen = std::collections::HashSet::new();
        for list in GG3_LISTS.iter().chain([&eo_landmarks::suspension::SWITCHES]) {
            for name in list.iter() {
                assert!(name.starts_with("--no-"), "{name}");
                assert!(seen.insert(*name), "{name} is in two lists");
                if let Err(e) = super::parse_args_from(&[name.to_string()]) {
                    panic!("{name} does not parse: {e}");
                }
            }
        }
        for name in ["--no-gg3", "--no-gg3-perf", "--no-gg3-sky", "--no-gg3-colour", "--no-gg3-vegetation", "--no-gg3-ground", "--no-gg3-rock", "--no-gg3-water", "--no-gg3-bridge", "--no-bridges5", "--no-hill-rock"] {
            assert!(seen.contains(name), "{name} is reserved");
        }
        assert!(super::parse_args_from(&["--no-such-switch".to_string()]).is_err());
    }

    /// `--no-city-classes` (eo-render `CityClasses`, read by that crate) parses.
    #[test]
    fn the_city_classes_switch_parses() {
        assert_eq!(eo_render::CityClasses::SWITCH, "--no-city-classes");
        assert!(super::parse_args_from(&[eo_render::CityClasses::SWITCH.to_string()]).is_ok());
    }

    /// `--no-motion-bounds` (eo-render `counted::MotionBounds`, read by that crate) parses.
    #[test]
    fn the_motion_bounds_switch_parses() {
        assert_eq!(eo_render::counted::MotionBounds::SWITCH, "--no-motion-bounds");
        assert!(super::parse_args_from(&[eo_render::counted::MotionBounds::SWITCH.to_string()]).is_ok());
    }

    /// The bounded draws are opt-in, and `--no-bounded-draws` wins wherever it stands.
    #[test]
    fn bounded_draws_are_opt_in_and_the_off_switch_wins() {
        let args = |line: &[&str]| super::parse_args_from(&line.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap();
        assert!(!args(&[]).bounded_draws);
        assert!(args(&["--bounded-draws"]).bounded_draws);
        assert!(!args(&["--no-bounded-draws", "--bounded-draws"]).bounded_draws);
        assert!(!args(&["--bounded-draws", "--no-bounded-draws"]).bounded_draws);
    }
}
